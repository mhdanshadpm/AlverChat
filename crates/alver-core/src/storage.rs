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
                 CREATE TABLE IF NOT EXISTS message_revisions (message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, revision INTEGER NOT NULL, body TEXT NOT NULL, physical_ms INTEGER NOT NULL, logical INTEGER NOT NULL, PRIMARY KEY(message_id, revision));
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

    pub fn messages_for_chat(&self, chat_id: &str) -> Result<Vec<Message>, String> {
        let mut statement = self.connection.prepare(
            "SELECT id, chat_id, sender, body, physical_ms, logical, tombstone FROM messages WHERE chat_id = ?1 ORDER BY physical_ms, logical, id",
        ).map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                let id = row.get::<_, String>(0)?;
                Ok(Message {
                    id: Uuid::parse_str(&id).map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            0,
                            rusqlite::types::Type::Text,
                            Box::new(error),
                        )
                    })?,
                    chat_id: row.get(1)?,
                    sender: row.get(2)?,
                    body: row.get(3)?,
                    clock: Hlc {
                        physical_ms: row.get::<_, i64>(4)? as u64,
                        logical: row.get::<_, i64>(5)? as u32,
                    },
                    tombstone: row.get::<_, i64>(6)? != 0,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
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

    /// Apply a sender-owned edit unless a permanent tombstone already exists.
    pub fn edit_message(
        &mut self,
        id: Uuid,
        sender: &str,
        body: &str,
        clock: Hlc,
    ) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let current = transaction
            .query_row(
                "SELECT sender, body, tombstone FROM messages WHERE id = ?1",
                params![id.to_string()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)? != 0,
                    ))
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some((owner, old_body, tombstone)) = current else {
            return Ok(false);
        };
        if owner != sender || tombstone {
            return Ok(false);
        }
        let revision: i64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) + 1 FROM message_revisions WHERE message_id = ?1",
                params![id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO message_revisions(message_id, revision, body, physical_ms, logical) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.to_string(), revision, old_body, clock.physical_ms, clock.logical],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "UPDATE messages SET body = ?2, physical_ms = ?3, logical = ?4 WHERE id = ?1 AND sender = ?5 AND tombstone = 0",
                params![id.to_string(), body, clock.physical_ms, clock.logical, sender],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM message_search WHERE message_id = ?1",
                params![id.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "INSERT INTO message_search(message_id, body) VALUES (?1, ?2)",
                params![id.to_string(), body],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// Delete-for-everyone keeps a durable tombstone and removes searchable text.
    pub fn tombstone_message(&mut self, id: Uuid, sender: &str) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let changed = transaction
            .execute(
                "UPDATE messages SET body = '', tombstone = 1 WHERE id = ?1 AND sender = ?2 AND tombstone = 0",
                params![id.to_string(), sender],
            )
            .map_err(|error| error.to_string())?;
        if changed == 1 {
            transaction
                .execute(
                    "DELETE FROM message_search WHERE message_id = ?1",
                    params![id.to_string()],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    /// Search local message text without allowing callers to inject FTS syntax.
    pub fn search_messages(
        &self,
        query: &str,
        limit: usize,
    ) -> Result<Vec<(Uuid, String)>, String> {
        let terms: Vec<_> = query
            .split_whitespace()
            .take(16)
            .map(|term| format!("\"{}\"", term.replace('"', "\"\"")))
            .collect();
        if terms.is_empty() || query.len() > 256 || limit == 0 {
            return Ok(Vec::new());
        }
        let expression = terms.join(" AND ");
        let mut statement = self
            .connection
            .prepare("SELECT message_id, body FROM message_search WHERE message_search MATCH ?1 LIMIT ?2")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![expression, limit.min(100) as i64], |row| {
                let id = row.get::<_, String>(0)?;
                let parsed = Uuid::parse_str(&id).map_err(|error| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Text,
                        Box::new(error),
                    )
                })?;
                Ok((parsed, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
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
    fn chat_history_is_ordered_and_scoped_to_the_requested_chat() {
        let mut store = Store::open(":memory:", &[10; 32]).unwrap();
        let later = Message {
            id: Uuid::now_v7(),
            chat_id: "peer-a".into(),
            sender: "local".into(),
            body: "later".into(),
            clock: Hlc {
                physical_ms: 20,
                logical: 0,
            },
            tombstone: false,
        };
        let earlier = Message {
            id: Uuid::now_v7(),
            chat_id: "peer-a".into(),
            sender: "peer-a".into(),
            body: "earlier".into(),
            clock: Hlc {
                physical_ms: 10,
                logical: 0,
            },
            tombstone: false,
        };
        let other_chat = Message {
            id: Uuid::now_v7(),
            chat_id: "peer-b".into(),
            sender: "peer-b".into(),
            body: "other".into(),
            clock: Hlc {
                physical_ms: 15,
                logical: 0,
            },
            tombstone: false,
        };
        store.save_message(&later).unwrap();
        store.save_message(&other_chat).unwrap();
        store.save_message(&earlier).unwrap();
        assert_eq!(
            store.messages_for_chat("peer-a").unwrap(),
            vec![earlier, later]
        );
        assert!(store.messages_for_chat("missing").unwrap().is_empty());
    }

    #[test]
    fn encrypted_messages_survive_reopening_the_database() {
        let path = std::env::temp_dir().join(format!("alverchat-store-{}.db", Uuid::now_v7()));
        let path = path.to_str().unwrap().to_owned();
        let key = [12; 32];
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: "peer-persisted".into(),
            sender: "local".into(),
            body: "durable message".into(),
            clock: Hlc {
                physical_ms: 99,
                logical: 2,
            },
            tombstone: false,
        };
        {
            let mut store = Store::open(&path, &key).unwrap();
            store.save_message(&message).unwrap();
        }
        let store = Store::open(&path, &key).unwrap();
        assert_eq!(
            store.messages_for_chat("peer-persisted").unwrap(),
            vec![message]
        );
        drop(store);
        let _ = std::fs::remove_file(path);
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

    #[test]
    fn message_edits_are_sender_only_and_tombstones_never_resurrect() {
        let mut store = Store::open(":memory:", &[8; 32]).unwrap();
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: "chat-1".into(),
            sender: "peer-a".into(),
            body: "original".into(),
            clock: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            tombstone: false,
        };
        store.save_message(&message).unwrap();
        assert_eq!(store.search_messages("original", 10).unwrap().len(), 1);
        let edited_clock = Hlc {
            physical_ms: 2,
            logical: 0,
        };
        assert!(!store
            .edit_message(message.id, "peer-b", "hijack", edited_clock)
            .unwrap());
        assert!(store
            .edit_message(message.id, "peer-a", "edited", edited_clock)
            .unwrap());
        assert_eq!(
            store.get_message(message.id).unwrap().unwrap().body,
            "edited"
        );
        assert!(store.search_messages("original", 10).unwrap().is_empty());
        assert_eq!(store.search_messages("edited", 10).unwrap().len(), 1);
        assert!(store.tombstone_message(message.id, "peer-a").unwrap());
        assert!(!store
            .edit_message(message.id, "peer-a", "resurrected", edited_clock)
            .unwrap());
        let deleted = store.get_message(message.id).unwrap().unwrap();
        assert!(deleted.tombstone);
        assert!(deleted.body.is_empty());
        assert!(store.search_messages("edited", 10).unwrap().is_empty());
    }
}
