use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::groups::GroupState;
use crate::{Envelope, Hlc, Message};

/// Encrypted local store. The key must come from the platform keychain.
pub struct Store {
    connection: Connection,
}

impl Store {
    pub fn open(path: &str, key: &[u8; 32]) -> Result<Self, String> {
        let connection = Connection::open(path).map_err(|e| e.to_string())?;
        let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        connection
            .execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(
                "PRAGMA foreign_keys = ON;
                 CREATE TABLE IF NOT EXISTS peers (id TEXT PRIMARY KEY, public_key BLOB NOT NULL, trusted INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, name TEXT NOT NULL, is_group INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sender TEXT NOT NULL, body TEXT NOT NULL, physical_ms INTEGER NOT NULL, logical INTEGER NOT NULL, tombstone INTEGER NOT NULL DEFAULT 0, FOREIGN KEY(chat_id) REFERENCES chats(id));
                 CREATE INDEX IF NOT EXISTS messages_by_chat_clock ON messages(chat_id, physical_ms, logical); 
                 CREATE TABLE IF NOT EXISTS outbox (id TEXT PRIMARY KEY, peer_id TEXT NOT NULL, envelope BLOB NOT NULL, attempt INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS groups (id TEXT PRIMARY KEY, name TEXT NOT NULL, epoch INTEGER NOT NULL, state BLOB NOT NULL);
                 CREATE TABLE IF NOT EXISTS group_members (group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE, peer_id TEXT NOT NULL, role TEXT NOT NULL CHECK(role IN ('admin','member')), PRIMARY KEY(group_id, peer_id));
                 CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(message_id UNINDEXED, body);
                 PRAGMA user_version = 1;",
            )
            .map_err(|e| e.to_string())?;
        Ok(Self { connection })
    }

    pub fn save_message(&mut self, message: &Message) -> Result<(), String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO chats(id, name) VALUES (?1, ?1) ON CONFLICT(id) DO NOTHING",
                params![message.chat_id],
            )
            .map_err(|e| e.to_string())?;
        let inserted = transaction
            .execute(
                "INSERT INTO messages(id, chat_id, sender, body, physical_ms, logical, tombstone) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO NOTHING",
                params![message.id.to_string(), message.chat_id, message.sender, message.body, message.clock.physical_ms, message.clock.logical, message.tombstone],
            )
            .map_err(|e| e.to_string())?;
        if inserted == 1 && !message.tombstone {
            transaction
                .execute(
                    "INSERT INTO message_search(message_id, body) VALUES (?1, ?2)",
                    params![message.id.to_string(), message.body],
                )
                .map_err(|e| e.to_string())?;
        }
        transaction.commit().map_err(|e| e.to_string())
    }

    pub fn enqueue(
        &mut self,
        peer_id: &str,
        envelope: &Envelope<serde_json::Value>,
    ) -> Result<(), String> {
        let bytes = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO outbox(id, peer_id, envelope) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
                params![envelope.id.to_string(), peer_id, bytes],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    pub fn pending_outbox(
        &self,
        peer_id: &str,
    ) -> Result<Vec<Envelope<serde_json::Value>>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT envelope FROM outbox WHERE peer_id = ?1 ORDER BY rowid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![peer_id], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let bytes = row.map_err(|error| error.to_string())?;
            serde_json::from_slice(&bytes).map_err(|error| error.to_string())
        })
        .collect()
    }

    pub fn acknowledge_outbox(&mut self, id: Uuid) -> Result<bool, String> {
        self.connection
            .execute("DELETE FROM outbox WHERE id = ?1", params![id.to_string()])
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// Save a new signed group or its next valid membership epoch atomically.
    pub fn save_group_state(&mut self, state: &GroupState) -> Result<(), String> {
        if !state.verify() {
            return Err("group signature or roster is invalid".into());
        }
        let previous = self
            .connection
            .query_row(
                "SELECT state FROM groups WHERE id = ?1",
                params![state.id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|bytes| serde_json::from_slice::<GroupState>(&bytes))
            .transpose()
            .map_err(|error| error.to_string())?;
        match previous {
            Some(previous) if !state.verify_successor_of(&previous) => {
                return Err("group update is stale or skips an epoch".into());
            }
            None if state.epoch != 1 => return Err("new group must start at epoch 1".into()),
            _ => {}
        }

        let state_bytes = serde_json::to_vec(state).map_err(|error| error.to_string())?;
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO groups(id, name, epoch, state) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(id) DO UPDATE SET name = excluded.name, epoch = excluded.epoch, state = excluded.state",
                params![state.id, state.name, state.epoch, state_bytes],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM group_members WHERE group_id = ?1",
                params![state.id],
            )
            .map_err(|error| error.to_string())?;
        for member in &state.members {
            let role = if member == &state.admin {
                "admin"
            } else {
                "member"
            };
            transaction
                .execute(
                    "INSERT INTO group_members(group_id, peer_id, role) VALUES (?1, ?2, ?3)",
                    params![state.id, member, role],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    pub fn get_group_state(&self, id: &str) -> Result<Option<GroupState>, String> {
        self.connection
            .query_row(
                "SELECT state FROM groups WHERE id = ?1",
                params![id],
                |row| row.get::<_, Vec<u8>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
            .transpose()
    }

    pub fn get_message(&self, id: Uuid) -> Result<Option<Message>, String> {
        self.connection
            .query_row(
                "SELECT id, chat_id, sender, body, physical_ms, logical, tombstone FROM messages WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok(Message {
                        id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|e| rusqlite::Error::FromSqlConversionFailure(0, rusqlite::types::Type::Text, Box::new(e)))?,
                        chat_id: row.get(1)?, sender: row.get(2)?, body: row.get(3)?,
                        clock: Hlc { physical_ms: row.get::<_, i64>(4)? as u64, logical: row.get::<_, i64>(5)? as u32 },
                        tombstone: row.get::<_, i64>(6)? != 0,
                    })
                },
            )
            .optional()
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn migrations_and_message_round_trip() {
        let mut store = Store::open(":memory:", &[9; 32]).unwrap();
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: "peer-1".into(),
            sender: "self".into(),
            body: "hello".into(),
            clock: Hlc {
                physical_ms: 22,
                logical: 3,
            },
            tombstone: false,
        };
        store.save_message(&message).unwrap();
        assert_eq!(store.get_message(message.id).unwrap(), Some(message));
    }

    #[test]
    fn persistent_outbox_is_ordered_and_acknowledged() {
        let store_key = [2; 32];
        let mut store = Store::open(":memory:", &store_key).unwrap();
        let envelope = Envelope {
            v: crate::PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: "message".into(),
            from: "self".into(),
            to: "chat".into(),
            hlc: Hlc::default(),
            payload: serde_json::json!({ "body": "queued" }),
        };
        store.enqueue("peer", &envelope).unwrap();
        store.enqueue("peer", &envelope).unwrap();
        assert_eq!(
            store.pending_outbox("peer").unwrap(),
            vec![envelope.clone()]
        );
        assert!(store.acknowledge_outbox(envelope.id).unwrap());
        assert!(store.pending_outbox("peer").unwrap().is_empty());
    }

    #[test]
    fn signed_group_states_persist_atomically_and_reject_stale_epochs() {
        let admin = crate::identity::Identity::from_secret([11; 32]);
        let mut state = GroupState::create(
            &admin,
            "group-persist".into(),
            "Project Alpha".into(),
            ["peer-a".into()],
        )
        .unwrap();
        let first_epoch = state.clone();
        let mut store = Store::open(":memory:", &[5; 32]).unwrap();
        store.save_group_state(&state).unwrap();
        assert_eq!(
            store.get_group_state(&state.id).unwrap(),
            Some(state.clone())
        );

        state
            .update_members(&admin, [admin.peer_id(), "peer-b".into()])
            .unwrap();
        store.save_group_state(&state).unwrap();
        assert_eq!(store.get_group_state(&state.id).unwrap(), Some(state));
        assert!(store.save_group_state(&first_epoch).is_err());
    }
}
