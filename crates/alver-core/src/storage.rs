use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::{Hlc, Message};

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
        transaction
            .execute(
                "INSERT INTO messages(id, chat_id, sender, body, physical_ms, logical, tombstone) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(id) DO NOTHING",
                params![message.id.to_string(), message.chat_id, message.sender, message.body, message.clock.physical_ms, message.clock.logical, message.tombstone],
            )
            .map_err(|e| e.to_string())?;
        transaction
            .execute(
                "INSERT INTO message_search(message_id, body) VALUES (?1, ?2)",
                params![message.id.to_string(), message.body],
            )
            .map_err(|e| e.to_string())?;
        transaction.commit().map_err(|e| e.to_string())
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
}
