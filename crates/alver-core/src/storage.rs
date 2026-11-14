use rusqlite::{params, Connection, OptionalExtension};
use uuid::Uuid;

use crate::groups::GroupState;
use crate::rules;
use crate::{Envelope, Hlc, Message};

/// Ordered, additive schema history.
///
/// Each entry is `(version, DDL)` and runs exactly once, inside its own transaction,
/// guarded by `PRAGMA user_version`. Deviation from the spec: `refinery` is not used,
/// because it would add a new build dependency to a crate that currently vendors
/// OpenSSL for SQLCipher; this hand-rolled runner keeps refinery's guarantees (ordered
/// versions, one transaction per migration, never re-running or dropping anything).
pub const MIGRATIONS: &[MIGRATION] = &[
    MIGRATION {
        version: 1,
        // v1 is frozen: an existing database must open with these tables untouched.
        sql: "
                 CREATE TABLE IF NOT EXISTS peers (id TEXT PRIMARY KEY, public_key BLOB NOT NULL, trusted INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS peer_directory (id TEXT PRIMARY KEY REFERENCES peers(id) ON DELETE CASCADE, display_name TEXT NOT NULL, last_seen_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS chats (id TEXT PRIMARY KEY, name TEXT NOT NULL, is_group INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS messages (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, sender TEXT NOT NULL, body TEXT NOT NULL, physical_ms INTEGER NOT NULL, logical INTEGER NOT NULL, tombstone INTEGER NOT NULL DEFAULT 0, FOREIGN KEY(chat_id) REFERENCES chats(id));
                 CREATE INDEX IF NOT EXISTS messages_by_chat_clock ON messages(chat_id, physical_ms, logical);
                 CREATE TABLE IF NOT EXISTS message_revisions (message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, revision INTEGER NOT NULL, body TEXT NOT NULL, physical_ms INTEGER NOT NULL, logical INTEGER NOT NULL, PRIMARY KEY(message_id, revision));
                 CREATE TABLE IF NOT EXISTS outbox (id TEXT PRIMARY KEY, peer_id TEXT NOT NULL, envelope BLOB NOT NULL, attempt INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS groups (id TEXT PRIMARY KEY, name TEXT NOT NULL, epoch INTEGER NOT NULL, state BLOB NOT NULL);
                 CREATE TABLE IF NOT EXISTS group_members (group_id TEXT NOT NULL REFERENCES groups(id) ON DELETE CASCADE, peer_id TEXT NOT NULL, role TEXT NOT NULL CHECK(role IN ('admin','member')), PRIMARY KEY(group_id, peer_id));
                 CREATE VIRTUAL TABLE IF NOT EXISTS message_search USING fts5(message_id UNINDEXED, body);
             ",
    },
    MIGRATION {
        version: 2,
        sql: "
                 CREATE TABLE IF NOT EXISTS reactions (message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, peer_id TEXT NOT NULL, emoji TEXT NOT NULL, created_ms INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(message_id, peer_id, emoji));
                 CREATE INDEX IF NOT EXISTS reactions_by_message ON reactions(message_id);
                 CREATE TABLE IF NOT EXISTS receipts (message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE, peer_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('delivered','read')), occurred_ms INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(message_id, peer_id));
                 CREATE INDEX IF NOT EXISTS receipts_by_message ON receipts(message_id);
                 CREATE TABLE IF NOT EXISTS pins (message_id TEXT NOT NULL PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, chat_id TEXT NOT NULL, position INTEGER NOT NULL DEFAULT 0, pinned_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE INDEX IF NOT EXISTS pins_by_chat ON pins(chat_id, position, pinned_ms);
                 CREATE TABLE IF NOT EXISTS stars (message_id TEXT NOT NULL PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE, chat_id TEXT NOT NULL, starred_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE INDEX IF NOT EXISTS stars_by_chat ON stars(chat_id, starred_ms);
                 CREATE TABLE IF NOT EXISTS attachments (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, message_id TEXT, file_name TEXT NOT NULL, size_bytes INTEGER NOT NULL DEFAULT 0, blake3_hash BLOB, status TEXT NOT NULL CHECK(status IN ('idle','offered','accepted','transferring','paused','completed','failed','cancelled')), updated_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE INDEX IF NOT EXISTS attachments_by_chat ON attachments(chat_id, updated_ms);
                 CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value TEXT NOT NULL, updated_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE TABLE IF NOT EXISTS avatar_cache (peer_id TEXT PRIMARY KEY, media_type TEXT NOT NULL, width INTEGER NOT NULL, height INTEGER NOT NULL, hash TEXT NOT NULL, bytes BLOB NOT NULL, updated_ms INTEGER NOT NULL DEFAULT 0);
             ",
    },
    MIGRATION {
        version: 3,
        sql: "
                 CREATE TABLE IF NOT EXISTS announcements (id TEXT PRIMARY KEY, group_id TEXT NOT NULL, sender TEXT NOT NULL, body TEXT NOT NULL, physical_ms INTEGER NOT NULL DEFAULT 0, logical INTEGER NOT NULL DEFAULT 0, admins_only INTEGER NOT NULL DEFAULT 0, created_ms INTEGER NOT NULL DEFAULT 0);
                 CREATE INDEX IF NOT EXISTS announcements_by_group ON announcements(group_id, physical_ms, logical);
                 CREATE TABLE IF NOT EXISTS announcement_acks (announcement_id TEXT NOT NULL REFERENCES announcements(id) ON DELETE CASCADE, peer_id TEXT NOT NULL, acked_ms INTEGER NOT NULL DEFAULT 0, PRIMARY KEY(announcement_id, peer_id));
             ",
    },
    MIGRATION {
        version: 4,
        sql: "
                 CREATE TABLE IF NOT EXISTS call_events (id TEXT PRIMARY KEY, chat_id TEXT NOT NULL, call_id TEXT NOT NULL, peer_id TEXT NOT NULL, kind TEXT NOT NULL CHECK(kind IN ('offer','answer','end','reject','busy')), accepted INTEGER, started_ms INTEGER NOT NULL DEFAULT 0, ended_ms INTEGER, note TEXT NOT NULL DEFAULT '');
                 CREATE INDEX IF NOT EXISTS call_events_by_chat ON call_events(chat_id, started_ms, id);
             ",
    },
    MIGRATION {
        version: 5,
        // Rows written before v5 keep `reply_to` NULL, which reads as "not a reply".
        sql: "
                 ALTER TABLE messages ADD COLUMN reply_to TEXT;
             ",
    },
];

/// One entry of [`MIGRATIONS`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MIGRATION {
    pub version: u32,
    pub sql: &'static str,
}

/// The highest schema version this build knows how to reach.
pub const LATEST_SCHEMA_VERSION: u32 = MIGRATIONS[MIGRATIONS.len() - 1].version;

/// Connection-level pragmas that must run outside a migration transaction.
const BOOTSTRAP_PRAGMAS: &str = "PRAGMA foreign_keys = ON;";

/// Encrypted local store. The key must come from the platform keychain.
pub struct Store {
    pub(crate) connection: Connection,
}

impl Store {
    pub fn open(path: &str, key: &[u8; 32]) -> Result<Self, String> {
        Self::open_to_version(path, key, LATEST_SCHEMA_VERSION)
    }

    /// Opens a database and migrates it only as far as `target_version`.
    ///
    /// Used by the upgrade tests to build a v1 database and then raise it.
    pub fn open_to_version(
        path: &str,
        key: &[u8; 32],
        target_version: u32,
    ) -> Result<Self, String> {
        if target_version > LATEST_SCHEMA_VERSION {
            return Err(format!(
                "schema version {target_version} is newer than this build supports"
            ));
        }
        let mut connection = Connection::open(path).map_err(|e| e.to_string())?;
        let key_hex = key.iter().map(|b| format!("{b:02x}")).collect::<String>();
        connection
            .execute_batch(&format!("PRAGMA key = \"x'{key_hex}'\";"))
            .map_err(|e| e.to_string())?;
        connection
            .execute_batch(BOOTSTRAP_PRAGMAS)
            .map_err(|e| e.to_string())?;
        migrate(&mut connection, target_version)?;
        Ok(Self { connection })
    }

    /// The schema version currently recorded in the database header.
    pub fn schema_version(&self) -> Result<u32, String> {
        current_version(&self.connection)
    }

    /// How many stored revisions a message has, including after a tombstone.
    pub fn revision_count(&self, message_id: uuid::Uuid) -> Result<i64, String> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM message_revisions WHERE message_id = ?1",
                rusqlite::params![message_id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map_err(|e| e.to_string())
    }
}

/// Reads `PRAGMA user_version`.
pub(crate) fn current_version(connection: &Connection) -> Result<u32, String> {
    connection
        .query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
        .map(|value| value.max(0) as u32)
        .map_err(|error| error.to_string())
}

/// Applies every unapplied migration up to `target_version`, one transaction each.
///
/// Nothing here drops a table or rewrites an applied migration: already-recorded
/// versions are skipped, and the version is bumped inside the same transaction as the
/// DDL so a half-applied migration can never be recorded.
pub(crate) fn migrate(connection: &mut Connection, target_version: u32) -> Result<(), String> {
    let mut version = current_version(connection)?;
    if version > target_version {
        return Err(format!(
            "database is at schema version {version}, this run only migrates to {target_version}"
        ));
    }
    for migration in MIGRATIONS {
        if migration.version <= version {
            continue;
        }
        if migration.version > target_version {
            break;
        }
        if migration.version != version + 1 {
            return Err(format!(
                "migration history has a gap: expected version {}, found {}",
                version + 1,
                migration.version
            ));
        }
        let transaction = connection
            .transaction()
            .map_err(|error| error.to_string())?;
        transaction
            .execute_batch(migration.sql)
            .map_err(|error| error.to_string())?;
        transaction
            .pragma_update(None, "user_version", migration.version)
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        version = migration.version;
    }
    if version != target_version {
        return Err(format!(
            "migration history stops at version {version}, cannot reach {target_version}"
        ));
    }
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredPeer {
    pub id: String,
    pub name: String,
    pub public_key: [u8; 32],
}

impl Store {
    pub fn save_message(&mut self, message: &Message) -> Result<bool, String> {
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
        transaction.commit().map_err(|e| e.to_string())?;
        Ok(inserted == 1)
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

    /// Atomically persist an outgoing message and its delivery outbox envelope.
    pub fn enqueue_message(
        &mut self,
        message: &Message,
        peer_id: &str,
        envelope: &Envelope<serde_json::Value>,
    ) -> Result<(), String> {
        self.enqueue_quoted_message(message, peer_id, envelope, None)
    }

    /// As [`Store::enqueue_message`], also recording which message this one quotes.
    ///
    /// A quote must point at an existing message in the same chat; a dangling or foreign
    /// `reply_to` is refused instead of being stored, so the UI never renders a ghost.
    pub fn enqueue_quoted_message(
        &mut self,
        message: &Message,
        peer_id: &str,
        envelope: &Envelope<serde_json::Value>,
        reply_to: Option<Uuid>,
    ) -> Result<(), String> {
        if message.id != envelope.id
            || message.chat_id != peer_id
            || envelope.to != peer_id
            || envelope.kind != "message"
            || message.body.is_empty()
            || message.body.len() > 16_384
        {
            return Err("outgoing message and transport envelope do not match".into());
        }
        let envelope_bytes = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        Self::write_outgoing_message(&transaction, message, reply_to)?;
        transaction
            .execute(
                "INSERT INTO outbox(id, peer_id, envelope) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
                params![envelope.id.to_string(), peer_id, envelope_bytes],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())
    }

    /// Atomically persists one group message and one outbox row per recipient.
    ///
    /// The message keeps a single id, while every recipient gets its own transport
    /// envelope (with `to` naming that peer) and a deterministic outbox row id. Rows are
    /// only skipped when the same recipient was already queued for the same message.
    pub fn enqueue_group_message(
        &mut self,
        message: &Message,
        deliveries: &[(String, Envelope<serde_json::Value>)],
        reply_to: Option<Uuid>,
    ) -> Result<(), String> {
        if message.body.is_empty() || message.body.len() > 16_384 {
            return Err("outgoing group message is empty or too large".into());
        }
        for (recipient, envelope) in deliveries {
            if recipient.is_empty()
                || envelope.id != message.id
                || &envelope.to != recipient
                || envelope.kind != "message"
            {
                return Err("group delivery does not match its message".into());
            }
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        Self::write_outgoing_message(&transaction, message, reply_to)?;
        for (recipient, envelope) in deliveries {
            let row_id = crate::groups::derive_outbox_id(envelope.id, recipient);
            let envelope_bytes = serde_json::to_vec(envelope).map_err(|error| error.to_string())?;
            transaction
                .execute(
                    "INSERT INTO outbox(id, peer_id, envelope) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO NOTHING",
                    params![row_id.to_string(), recipient, envelope_bytes],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())
    }

    fn write_outgoing_message(
        transaction: &rusqlite::Transaction,
        message: &Message,
        reply_to: Option<Uuid>,
    ) -> Result<(), String> {
        if let Some(quoted) = reply_to {
            if quoted == message.id {
                return Err("a message cannot quote itself".into());
            }
            let quoted_chat: Option<String> = transaction
                .query_row(
                    "SELECT chat_id FROM messages WHERE id = ?1",
                    params![quoted.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if quoted_chat.as_deref() != Some(message.chat_id.as_str()) {
                return Err("a reply must quote a message from the same chat".into());
            }
        }
        transaction
            .execute(
                "INSERT INTO chats(id, name) VALUES (?1, ?1) ON CONFLICT(id) DO NOTHING",
                params![message.chat_id],
            )
            .map_err(|error| error.to_string())?;
        // Only a real quote touches the v5 column, so an unwritten `reply_to` keeps this
        // statement valid against the v1 message shape.
        let inserted = match reply_to {
            None => transaction
                .execute(
                    "INSERT INTO messages(id, chat_id, sender, body, physical_ms, logical, tombstone) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0) ON CONFLICT(id) DO NOTHING",
                    params![message.id.to_string(), message.chat_id, message.sender, message.body, message.clock.physical_ms, message.clock.logical],
                )
                .map_err(|error| error.to_string())?,
            Some(quoted) => transaction
                .execute(
                    "INSERT INTO messages(id, chat_id, sender, body, physical_ms, logical, tombstone, reply_to) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7) ON CONFLICT(id) DO NOTHING",
                    params![message.id.to_string(), message.chat_id, message.sender, message.body, message.clock.physical_ms, message.clock.logical, quoted.to_string()],
                )
                .map_err(|error| error.to_string())?,
        };
        if inserted == 1 {
            transaction
                .execute(
                    "INSERT INTO message_search(message_id, body) VALUES (?1, ?2)",
                    params![message.id.to_string(), message.body],
                )
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }

    /// Records the message a stored message quotes; `None` clears the quote.
    ///
    /// Used for the receive path, where the quote arrives with the envelope and the row
    /// is already written. Same-chat and no-self-quote rules match
    /// [`Store::enqueue_quoted_message`]; the tombstone of a quoted message is not
    /// checked here, because the delete keeps the id and the UI renders a placeholder.
    pub fn set_reply_to(
        &mut self,
        message_id: Uuid,
        reply_to: Option<Uuid>,
    ) -> Result<bool, String> {
        if reply_to == Some(message_id) {
            return Err("a message cannot quote itself".into());
        }
        let chat_id: Option<String> = self
            .connection
            .query_row(
                "SELECT chat_id FROM messages WHERE id = ?1",
                params![message_id.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(chat_id) = chat_id else {
            return Ok(false);
        };
        if let Some(quoted) = reply_to {
            let quoted_chat: Option<String> = self
                .connection
                .query_row(
                    "SELECT chat_id FROM messages WHERE id = ?1",
                    params![quoted.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| error.to_string())?;
            if quoted_chat.as_deref() != Some(chat_id.as_str()) {
                return Err("a reply must quote a message from the same chat".into());
            }
        }
        self.connection
            .execute(
                "UPDATE messages SET reply_to = ?2 WHERE id = ?1",
                params![message_id.to_string(), reply_to.map(|id| id.to_string())],
            )
            .map(|_| true)
            .map_err(|error| error.to_string())
    }

    /// The message id this message quotes, when it quotes one.
    pub fn reply_to_for(&self, message_id: Uuid) -> Result<Option<Uuid>, String> {
        let raw: Option<Option<String>> = self
            .connection
            .query_row(
                "SELECT reply_to FROM messages WHERE id = ?1",
                params![message_id.to_string()],
                |row| row.get::<_, Option<String>>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        raw.flatten()
            .map(|value| {
                Uuid::parse_str(&value)
                    .map(Some)
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(None))
    }

    /// Quotes of every message in a chat, as `message_id -> quoted_id` pairs.
    ///
    /// One query for the whole thread, so `list_messages` does not pay per row.
    pub fn replies_for_chat(&self, chat_id: &str) -> Result<Vec<(Uuid, Uuid)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, reply_to FROM messages WHERE chat_id = ?1 AND reply_to IS NOT NULL",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            let (id, quoted) = row.map_err(|error| error.to_string())?;
            Ok((
                Uuid::parse_str(&id).map_err(|error| error.to_string())?,
                Uuid::parse_str(&quoted).map_err(|error| error.to_string())?,
            ))
        })
        .collect()
    }

    /// Every group roster this device has verified, ordered by id.
    pub fn all_group_states(&self) -> Result<Vec<GroupState>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT state FROM groups ORDER BY id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| row.get::<_, Vec<u8>>(0))
            .map_err(|error| error.to_string())?;
        rows.map(|row| {
            row.map_err(|error| error.to_string())
                .and_then(|bytes| serde_json::from_slice(&bytes).map_err(|error| error.to_string()))
        })
        .collect()
    }

    pub fn is_message_pending(&self, id: Uuid) -> Result<bool, String> {
        self.connection
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM outbox WHERE id = ?1)",
                params![id.to_string()],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())
    }

    /// Pin an authenticated identity to its stable peer ID on first contact.
    pub fn trust_peer(&mut self, id: &str, public_key: &[u8; 32]) -> Result<(), String> {
        if crate::peer_id(public_key) != id {
            return Err("peer ID does not match its public key".into());
        }
        self.connection
            .execute(
                "INSERT INTO peers(id, public_key, trusted) VALUES (?1, ?2, 1) ON CONFLICT(id) DO NOTHING",
                params![id, public_key.as_slice()],
            )
            .map_err(|error| error.to_string())?;
        let stored: Vec<u8> = self
            .connection
            .query_row(
                "SELECT public_key FROM peers WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if stored.as_slice() != public_key {
            return Err("a different public key is already pinned to this peer ID".into());
        }
        Ok(())
    }

    pub fn save_peer(
        &mut self,
        id: &str,
        name: &str,
        public_key: &[u8; 32],
        last_seen_ms: u64,
    ) -> Result<(), String> {
        if name.trim().is_empty() || name.len() > 128 || name.chars().any(char::is_control) {
            return Err("peer display name is invalid".into());
        }
        self.trust_peer(id, public_key)?;
        self.connection
            .execute(
                "INSERT INTO peer_directory(id, display_name, last_seen_ms) VALUES (?1, ?2, ?3) ON CONFLICT(id) DO UPDATE SET display_name = excluded.display_name, last_seen_ms = excluded.last_seen_ms",
                params![id, name, last_seen_ms.min(i64::MAX as u64) as i64],
            )
            .map_err(|error| error.to_string())?;
        Ok(())
    }

    pub fn known_peers(&self) -> Result<Vec<StoredPeer>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT peers.id, peer_directory.display_name, peers.public_key FROM peer_directory JOIN peers ON peers.id = peer_directory.id ORDER BY peer_directory.display_name COLLATE NOCASE")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let key = row.get::<_, Vec<u8>>(2)?;
                let public_key = <[u8; 32]>::try_from(key.as_slice()).map_err(|_| {
                    rusqlite::Error::FromSqlConversionFailure(
                        2,
                        rusqlite::types::Type::Blob,
                        Box::new(std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "peer public key must contain 32 bytes",
                        )),
                    )
                })?;
                Ok(StoredPeer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    public_key,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
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
        // Authority and permanence come from `rules`, so the identical decisions are
        // property-testable without a database.
        if rules::edit_authority(&owner, sender).is_err() || !rules::late_edit_allowed(tombstone) {
            return Ok(false);
        }
        let highest: u64 = transaction
            .query_row(
                "SELECT COALESCE(MAX(revision), 0) FROM message_revisions WHERE message_id = ?1",
                params![id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as u64)
            .map_err(|error| error.to_string())?;
        let revision = rules::next_revision(highest);
        transaction
            .execute(
                "INSERT INTO message_revisions(message_id, revision, body, physical_ms, logical) VALUES (?1, ?2, ?3, ?4, ?5)",
                params![id.to_string(), revision as i64, old_body, clock.physical_ms, clock.logical],
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
    ///
    /// Point-to-point chats allow the original sender only; a group admin gets
    /// additional authority through [`Store::admin_tombstone_in_group`].
    pub fn tombstone_message(&mut self, id: Uuid, sender: &str) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let owner: Option<String> = transaction
            .query_row(
                "SELECT sender FROM messages WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(owner) = owner else {
            return Ok(false);
        };
        // The authority rule and the permanence rule both come from `rules`.
        if rules::tombstone_authority(&owner, sender, None).is_err() {
            return Ok(false);
        }
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
    fn outgoing_message_and_outbox_are_persisted_together_until_acknowledged() {
        let identity = crate::identity::Identity::from_secret([31; 32]);
        let peer_key = crate::identity::Identity::from_secret([32; 32])
            .verifying_key()
            .to_bytes();
        let peer = crate::peer_id(&peer_key);
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: peer.clone(),
            sender: "local".into(),
            body: "queued securely".into(),
            clock: Hlc {
                physical_ms: 12,
                logical: 0,
            },
            tombstone: false,
        };
        let envelope = Envelope {
            v: crate::PROTOCOL_VERSION,
            id: message.id,
            kind: "message".into(),
            from: identity.peer_id(),
            to: peer.clone(),
            hlc: message.clock,
            payload: serde_json::json!({"text": message.body}),
        };
        let mut store = Store::open(":memory:", &[33; 32]).unwrap();
        store.enqueue_message(&message, &peer, &envelope).unwrap();
        assert!(store.is_message_pending(message.id).unwrap());
        assert_eq!(
            store.messages_for_chat(&peer).unwrap(),
            vec![message.clone()]
        );
        assert_eq!(store.pending_outbox(&peer).unwrap(), vec![envelope.clone()]);
        assert!(store.acknowledge_outbox(message.id).unwrap());
        assert!(!store.is_message_pending(message.id).unwrap());
    }

    #[test]
    fn group_fan_out_stores_one_message_and_one_outbox_row_per_member() {
        let group_id = "group-fanout";
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: group_id.into(),
            sender: "local".into(),
            body: "hello group".into(),
            clock: Hlc {
                physical_ms: 19,
                logical: 0,
            },
            tombstone: false,
        };
        let deliveries: Vec<_> = ["peer-a", "peer-b"]
            .into_iter()
            .map(|recipient| {
                (
                    recipient.to_owned(),
                    Envelope {
                        v: crate::PROTOCOL_VERSION,
                        id: message.id,
                        kind: "message".into(),
                        from: "me".into(),
                        to: recipient.to_owned(),
                        hlc: message.clock,
                        payload: serde_json::json!({"text": "hello group", "chat_id": group_id}),
                    },
                )
            })
            .collect();
        let mut store = Store::open(":memory:", &[40; 32]).unwrap();
        store
            .enqueue_group_message(&message, &deliveries, None)
            .unwrap();
        assert_eq!(
            store.messages_for_chat(group_id).unwrap(),
            vec![message.clone()]
        );
        assert_eq!(
            store.pending_outbox("peer-a").unwrap(),
            vec![deliveries[0].1.clone()]
        );
        assert_eq!(
            store.pending_outbox("peer-b").unwrap(),
            vec![deliveries[1].1.clone()]
        );
        // Every recipient row has its own id so one receipt cannot clear another's row.
        let first = crate::groups::derive_outbox_id(message.id, "peer-a");
        let second = crate::groups::derive_outbox_id(message.id, "peer-b");
        assert_ne!(first, second);
        assert!(store.acknowledge_outbox(first).unwrap());
        assert!(store.pending_outbox("peer-a").unwrap().is_empty());
        assert_eq!(store.pending_outbox("peer-b").unwrap().len(), 1);
    }

    #[test]
    fn peer_identity_is_pinned_and_id_mismatch_is_rejected() {
        let peer = crate::identity::Identity::from_secret([34; 32]);
        let key = peer.verifying_key().to_bytes();
        let mut store = Store::open(":memory:", &[35; 32]).unwrap();
        store.trust_peer(&peer.peer_id(), &key).unwrap();
        store.trust_peer(&peer.peer_id(), &key).unwrap();
        assert!(store.trust_peer("wrong-id", &key).is_err());
        let other_key = crate::identity::Identity::from_secret([36; 32])
            .verifying_key()
            .to_bytes();
        assert!(store.trust_peer(&peer.peer_id(), &other_key).is_err());
    }

    #[test]
    fn known_peers_survive_database_reopen() {
        let path = std::env::temp_dir().join(format!("alverchat-peers-{}.db", Uuid::now_v7()));
        let path = path.to_str().unwrap().to_owned();
        let peer = crate::identity::Identity::from_secret([37; 32]);
        let expected = StoredPeer {
            id: peer.peer_id(),
            name: "LAN colleague".into(),
            public_key: peer.verifying_key().to_bytes(),
        };
        {
            let mut store = Store::open(&path, &[38; 32]).unwrap();
            store
                .save_peer(&expected.id, &expected.name, &expected.public_key, 123)
                .unwrap();
        }
        let store = Store::open(&path, &[38; 32]).unwrap();
        assert_eq!(store.known_peers().unwrap(), vec![expected]);
        drop(store);
        let _ = std::fs::remove_file(path);
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
