//! Typed store APIs for the v2+ schema, layered on [`crate::storage::Store`].
//!
//! `storage.rs` keeps owning the envelope, message, peer, group and outbox primitives;
//! this file only adds `impl Store` blocks for reactions, receipts, pins, stars,
//! attachments, announcements, calls, settings and the avatar cache. Every mutation
//! consults the pure rules in [`crate::rules`] and keeps the FTS index consistent,
//! including on the paths that delete messages.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::BTreeMap;
use uuid::Uuid;

use crate::files::TransferState;
use crate::groups::EVERYONE_GROUP_ID;
use crate::outbox::OutboxEntry;
use crate::payloads::{MAX_BODY_BYTES, MAX_REACTION_BYTES};
use crate::rules::{self, EditWindow, ReceiptKind};
use crate::security::{self, AvatarLimits};
use crate::storage::Store;
use crate::{Envelope, Hlc, Message};

/// Well-known setting keys. Values are JSON-encoded text in the `settings` table.
pub mod setting {
    /// `u64` milliseconds; see [`crate::rules::EditWindow`].
    pub const EDIT_WINDOW_MS: &str = "message.edit_window_ms";
    /// `u64` bytes; offers above this wait for an explicit accept.
    pub const AUTO_DOWNLOAD_LIMIT_BYTES: &str = "file.auto_download_limit_bytes";
    /// `u64` bytes; offers above this are refused outright.
    pub const MAX_FILE_BYTES: &str = "file.max_bytes";
    /// `bool`; when true only group admins may post announcements.
    pub const ANNOUNCEMENTS_ADMINS_ONLY: &str = "announcement.admins_only";
    /// `u64` days of history kept by [`Store::apply_retention`].
    pub const RETENTION_DAYS: &str = "history.retention_days";
    /// `u64`; sustained events per second per peer.
    pub const RATE_LIMIT_PER_SECOND: &str = "network.rate_limit_per_second";
    /// `u64`; burst allowance per peer.
    pub const RATE_LIMIT_BURST: &str = "network.rate_limit_burst";
    /// `u64` bytes; avatar payload budget.
    pub const AVATAR_MAX_BYTES: &str = "avatar.max_bytes";
    /// `u64` pixels; avatar edge budget.
    pub const AVATAR_MAX_DIMENSION: &str = "avatar.max_dimension";
    /// `u64` pixels; avatar pixel-count budget against decompression bombs.
    pub const AVATAR_MAX_PIXELS: &str = "avatar.max_pixels";
    /// `bool`; refuse reads of tombstoned messages in history views.
    pub const HIDE_DELETED_MESSAGES: &str = "history.hide_deleted_messages";
}

const MESSAGE_COLUMNS: &str = "id, chat_id, sender, body, physical_ms, logical, tombstone";
/// Same columns, always qualified, for the join queries.
const JOINED_MESSAGE_COLUMNS: &str = "messages.id, messages.chat_id, messages.sender, \
    messages.body, messages.physical_ms, messages.logical, messages.tombstone";

/// A stored reaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Reaction {
    pub message_id: Uuid,
    pub peer_id: String,
    pub emoji: String,
    pub created_ms: u64,
}

/// A stored delivery or read receipt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    pub message_id: Uuid,
    pub peer_id: String,
    pub kind: ReceiptKind,
    pub occurred_ms: u64,
}

/// A pinned message with its chat position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedMessage {
    pub message: Message,
    pub position: u32,
    pub pinned_ms: u64,
}

/// A starred message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StarredMessage {
    pub message: Message,
    pub starred_ms: u64,
}

/// A file attachment belonging to a chat.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    pub id: Uuid,
    pub chat_id: String,
    pub message_id: Option<Uuid>,
    pub file_name: String,
    pub size_bytes: u64,
    pub blake3_hash: Option<[u8; 32]>,
    pub state: TransferState,
    pub updated_ms: u64,
}

/// A group announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Announcement {
    pub id: Uuid,
    pub group_id: String,
    pub sender: String,
    pub body: String,
    pub clock: Hlc,
    pub admins_only: bool,
    pub created_ms: u64,
}

/// One peer's acknowledgement of an announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementAck {
    pub announcement_id: Uuid,
    pub peer_id: String,
    pub acked_ms: u64,
}

/// Sender-visible acknowledgement state for one announcement.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnnouncementSummary {
    pub announcement: Announcement,
    pub acks: Vec<AnnouncementAck>,
    /// Peers that have acknowledged.
    pub acked: usize,
    /// Peers expected to acknowledge: the group roster, or known peers for Everyone.
    pub expected: usize,
}

/// Call signalling event kinds recorded per chat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CallEventKind {
    Offer,
    Answer,
    End,
    Reject,
    Busy,
}

impl CallEventKind {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Offer => "offer",
            Self::Answer => "answer",
            Self::End => "end",
            Self::Reject => "reject",
            Self::Busy => "busy",
        }
    }

    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "offer" => Some(Self::Offer),
            "answer" => Some(Self::Answer),
            "end" => Some(Self::End),
            "reject" => Some(Self::Reject),
            "busy" => Some(Self::Busy),
            _ => None,
        }
    }
}

/// A persisted call log row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CallEvent {
    pub id: Uuid,
    pub chat_id: String,
    pub call_id: Uuid,
    pub peer_id: String,
    pub kind: CallEventKind,
    pub accepted: Option<bool>,
    pub started_ms: u64,
    pub ended_ms: Option<u64>,
    pub note: String,
}

/// A cached peer avatar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Avatar {
    pub peer_id: String,
    pub media_type: String,
    pub width: u32,
    pub height: u32,
    pub hash: String,
    pub bytes: Vec<u8>,
    pub updated_ms: u64,
}

fn to_ms(value: u64) -> i64 {
    value.min(i64::MAX as u64) as i64
}

fn from_ms(value: i64) -> u64 {
    value.max(0) as u64
}

fn uuid_from(row: &rusqlite::Row<'_>, column: usize) -> rusqlite::Result<Uuid> {
    let raw: String = row.get(column)?;
    Uuid::parse_str(&raw).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            column,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn message_from_row(row: &rusqlite::Row<'_>, offset: usize) -> rusqlite::Result<Message> {
    Ok(Message {
        id: uuid_from(row, offset)?,
        chat_id: row.get(offset + 1)?,
        sender: row.get(offset + 2)?,
        body: row.get(offset + 3)?,
        clock: Hlc {
            physical_ms: from_ms(row.get::<_, i64>(offset + 4)?),
            logical: row.get::<_, i64>(offset + 5)?.max(0) as u32,
        },
        tombstone: row.get::<_, i64>(offset + 6)? != 0,
    })
}

/// Helpers that must run either on a connection or inside an open transaction; a
/// `Transaction` derefs to `Connection`, so `&transaction` coerces into `&Connection`.
fn message_exists(connection: &Connection, id: Uuid) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1)",
            params![id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(|error| error.to_string())
}

fn message_is_tombstoned(connection: &Connection, id: Uuid) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT tombstone FROM messages WHERE id = ?1",
            params![id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|value| value.is_some_and(|flag| flag != 0))
        .map_err(|error| error.to_string())
}

fn announcement_exists(connection: &Connection, id: Uuid) -> Result<bool, String> {
    connection
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM announcements WHERE id = ?1)",
            params![id.to_string()],
            |row| row.get::<_, i64>(0),
        )
        .map(|value| value != 0)
        .map_err(|error| error.to_string())
}

fn announcement_ack_peers(connection: &Connection, id: Uuid) -> Result<Vec<String>, String> {
    let mut statement = connection
        .prepare(
            "SELECT peer_id FROM announcement_acks WHERE announcement_id = ?1 ORDER BY peer_id",
        )
        .map_err(|error| error.to_string())?;
    let rows = statement
        .query_map(params![id.to_string()], |row| row.get::<_, String>(0))
        .map_err(|error| error.to_string())?;
    rows.map(|row| row.map_err(|error| error.to_string()))
        .collect()
}

/// Removes a chat's messages, their FTS rows, and any attachment rows left dangling.
fn purge_messages_of_chat(connection: &Connection, chat_id: &str) -> Result<u64, String> {
    connection
        .execute(
            "DELETE FROM message_search WHERE message_id IN (SELECT id FROM messages WHERE chat_id = ?1)",
            params![chat_id],
        )
        .map_err(|error| error.to_string())?;
    let removed = connection
        .execute("DELETE FROM messages WHERE chat_id = ?1", params![chat_id])
        .map(|count| count as u64)
        .map_err(|error| error.to_string())?;
    connection
        .execute(
            "DELETE FROM attachments WHERE chat_id = ?1",
            params![chat_id],
        )
        .map_err(|error| error.to_string())?;
    Ok(removed)
}

impl Store {
    /// Adds a reaction, returning whether it was new.
    ///
    /// Unknown messages and tombstoned messages ignore the reaction: a late reaction can
    /// never resurrect a deleted message.
    pub fn add_reaction(
        &mut self,
        message_id: Uuid,
        peer_id: &str,
        emoji: &str,
        created_ms: u64,
    ) -> Result<bool, String> {
        if peer_id.trim().is_empty() || emoji.trim().is_empty() {
            return Err("reaction needs a peer and an emoji".into());
        }
        if emoji.len() > MAX_REACTION_BYTES {
            return Err("reaction is too long".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        if !message_exists(&transaction, message_id)?
            || !rules::late_reaction_allowed(message_is_tombstoned(&transaction, message_id)?)
        {
            return Ok(false);
        }
        let changed = transaction
            .execute(
                "INSERT INTO reactions(message_id, peer_id, emoji, created_ms) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(message_id, peer_id, emoji) DO NOTHING",
                params![message_id.to_string(), peer_id, emoji, to_ms(created_ms)],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    /// Removes one peer's reaction of one emoji; false when nothing matched.
    pub fn remove_reaction(
        &mut self,
        message_id: Uuid,
        peer_id: &str,
        emoji: &str,
    ) -> Result<bool, String> {
        self.connection
            .execute(
                "DELETE FROM reactions WHERE message_id = ?1 AND peer_id = ?2 AND emoji = ?3",
                params![message_id.to_string(), peer_id, emoji],
            )
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// Reactions on a message, oldest first; empty once the message is a tombstone.
    ///
    /// Tombstone permanence means a delete-for-everyone also retires the message's social
    /// state: the rows stay for convergence but are never shown again.
    pub fn reactions_for_message(&self, message_id: Uuid) -> Result<Vec<Reaction>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT reactions.peer_id, reactions.emoji, reactions.created_ms \
                 FROM reactions JOIN messages ON messages.id = reactions.message_id \
                 WHERE reactions.message_id = ?1 AND messages.tombstone = 0 \
                 ORDER BY reactions.created_ms, reactions.peer_id, reactions.emoji",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![message_id.to_string()], |row| {
                Ok(Reaction {
                    message_id,
                    peer_id: row.get(0)?,
                    emoji: row.get(1)?,
                    created_ms: from_ms(row.get::<_, i64>(2)?),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Records a receipt; returns whether the stored state changed.
    ///
    /// Idempotent and monotonic: repeating a level changes nothing, and a `delivered`
    /// arriving after `read` is refused quietly — a read receipt is never downgraded.
    /// Receipts for unknown or tombstoned messages are ignored.
    pub fn record_receipt(
        &mut self,
        message_id: Uuid,
        peer_id: &str,
        kind: ReceiptKind,
        occurred_ms: u64,
    ) -> Result<bool, String> {
        if peer_id.trim().is_empty() {
            return Err("receipt needs a peer".into());
        }
        let existing: Option<String> = self
            .connection
            .query_row(
                "SELECT kind FROM receipts WHERE message_id = ?1 AND peer_id = ?2",
                params![message_id.to_string(), peer_id],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let current = existing.as_deref().and_then(ReceiptKind::parse);
        let advances = rules::receipt_advances(current, kind).unwrap_or(false);
        if !advances {
            return Ok(false);
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        if !message_exists(&transaction, message_id)?
            || !rules::late_receipt_allowed(message_is_tombstoned(&transaction, message_id)?)
        {
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT INTO receipts(message_id, peer_id, kind, occurred_ms) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(message_id, peer_id) DO UPDATE SET kind = excluded.kind, occurred_ms = excluded.occurred_ms",
                params![message_id.to_string(), peer_id, kind.as_str(), to_ms(occurred_ms)],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// Receipts for a message, one row per peer, ordered by peer id; empty for a tombstone.
    pub fn receipts_for_message(&self, message_id: Uuid) -> Result<Vec<Receipt>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT receipts.peer_id, receipts.kind, receipts.occurred_ms \
                 FROM receipts JOIN messages ON messages.id = receipts.message_id \
                 WHERE receipts.message_id = ?1 AND messages.tombstone = 0 \
                 ORDER BY receipts.peer_id",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![message_id.to_string()], |row| {
                let kind: String = row.get(1)?;
                Ok(Receipt {
                    message_id,
                    peer_id: row.get(0)?,
                    kind: ReceiptKind::parse(&kind).unwrap_or(ReceiptKind::Delivered),
                    occurred_ms: from_ms(row.get::<_, i64>(2)?),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Received messages of a chat that `reader_id` has not read yet.
    ///
    /// Tombstones and the reader's own messages never count, so "Mark unread" on a thread
    /// of only my own messages stays a no-op.
    pub fn unread_messages_for_chat(
        &self,
        chat_id: &str,
        reader_id: &str,
    ) -> Result<usize, String> {
        self.connection
            .query_row(
                "SELECT COUNT(*) FROM messages \
                 WHERE chat_id = ?1 AND tombstone = 0 AND sender <> ?2 \
                 AND NOT EXISTS (SELECT 1 FROM receipts WHERE receipts.message_id = messages.id \
                                 AND receipts.peer_id = ?2 AND receipts.kind = 'read')",
                params![chat_id, reader_id],
                |row| row.get::<_, i64>(0),
            )
            .map(|count| count.max(0) as usize)
            .map_err(|error| error.to_string())
    }

    /// Records a read receipt for every unread message of a chat; returns how many cleared.
    ///
    /// Written in one statement instead of through [`Store::record_receipt`] per row, and
    /// it only ever raises the level, so a `read` can never be downgraded to `delivered`.
    pub fn mark_chat_read(
        &mut self,
        chat_id: &str,
        reader_id: &str,
        occurred_ms: u64,
    ) -> Result<usize, String> {
        if chat_id.trim().is_empty() || reader_id.trim().is_empty() {
            return Err("read state needs a chat and a reader".into());
        }
        let cleared = self.unread_messages_for_chat(chat_id, reader_id)?;
        if cleared == 0 {
            return Ok(0);
        }
        self.connection
            .execute(
                "INSERT INTO receipts(message_id, peer_id, kind, occurred_ms) \
                 SELECT id, ?2, 'read', ?3 FROM messages \
                 WHERE chat_id = ?1 AND tombstone = 0 AND sender <> ?2 \
                 ON CONFLICT(message_id, peer_id) DO UPDATE SET kind = 'read', occurred_ms = excluded.occurred_ms",
                params![chat_id, reader_id, to_ms(occurred_ms)],
            )
            .map_err(|error| error.to_string())?;
        Ok(cleared)
    }

    /// Removes my read receipts in a chat so the unread count returns; returns how many.
    pub fn mark_chat_unread(&mut self, chat_id: &str, reader_id: &str) -> Result<usize, String> {
        if chat_id.trim().is_empty() || reader_id.trim().is_empty() {
            return Err("read state needs a chat and a reader".into());
        }
        self.connection
            .execute(
                "DELETE FROM receipts WHERE peer_id = ?2 AND kind = 'read' \
                 AND message_id IN (SELECT id FROM messages WHERE chat_id = ?1 AND sender <> ?2)",
                params![chat_id, reader_id],
            )
            .map_err(|error| error.to_string())?;
        self.unread_messages_for_chat(chat_id, reader_id)
    }

    /// Pins a message inside its chat; false when the message is missing or tombstoned.
    pub fn pin(&mut self, message_id: Uuid, position: u32, pinned_ms: u64) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let Some(message) = Self::read_message(&transaction, message_id)? else {
            return Ok(false);
        };
        if !rules::accepts_updates(message.tombstone) {
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT INTO pins(message_id, chat_id, position, pinned_ms) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(message_id) DO UPDATE SET position = excluded.position, pinned_ms = excluded.pinned_ms",
                params![message_id.to_string(), message.chat_id, position, to_ms(pinned_ms)],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub fn unpin(&mut self, message_id: Uuid) -> Result<bool, String> {
        self.connection
            .execute(
                "DELETE FROM pins WHERE message_id = ?1",
                params![message_id.to_string()],
            )
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// Pinned messages of a chat, lowest position first.
    pub fn pinned_for_chat(&self, chat_id: &str) -> Result<Vec<PinnedMessage>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT pins.position, pins.pinned_ms, {JOINED_MESSAGE_COLUMNS} \
                 FROM pins JOIN messages ON messages.id = pins.message_id \
                 WHERE pins.chat_id = ?1 \
                 ORDER BY pins.position, pins.pinned_ms, pins.message_id"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                Ok(PinnedMessage {
                    position: row.get::<_, i64>(0)?.max(0) as u32,
                    pinned_ms: from_ms(row.get::<_, i64>(1)?),
                    message: message_from_row(row, 2)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Stars a message; false when the message is missing or tombstoned.
    pub fn star(&mut self, message_id: Uuid, starred_ms: u64) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let Some(message) = Self::read_message(&transaction, message_id)? else {
            return Ok(false);
        };
        if !rules::accepts_updates(message.tombstone) {
            return Ok(false);
        }
        transaction
            .execute(
                "INSERT INTO stars(message_id, chat_id, starred_ms) VALUES (?1, ?2, ?3) ON CONFLICT(message_id) DO UPDATE SET starred_ms = excluded.starred_ms",
                params![message_id.to_string(), message.chat_id, to_ms(starred_ms)],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    pub fn unstar(&mut self, message_id: Uuid) -> Result<bool, String> {
        self.connection
            .execute(
                "DELETE FROM stars WHERE message_id = ?1",
                params![message_id.to_string()],
            )
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// Starred messages of a chat, oldest star first.
    pub fn starred_for_chat(&self, chat_id: &str) -> Result<Vec<StarredMessage>, String> {
        let mut statement = self
            .connection
            .prepare(&format!(
                "SELECT stars.starred_ms, {JOINED_MESSAGE_COLUMNS} \
                 FROM stars JOIN messages ON messages.id = stars.message_id \
                 WHERE stars.chat_id = ?1 ORDER BY stars.starred_ms, stars.message_id"
            ))
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                Ok(StarredMessage {
                    starred_ms: from_ms(row.get::<_, i64>(0)?),
                    message: message_from_row(row, 1)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Inserts or updates an attachment row; the received file name goes through the
    /// path-traversal rules.
    pub fn upsert_attachment(&mut self, attachment: &Attachment) -> Result<(), String> {
        security::validate_received_file_name(&attachment.file_name)
            .map_err(|error| format!("{error:?}"))?;
        if attachment.chat_id.trim().is_empty() {
            return Err("attachment needs a chat".into());
        }
        let hash = attachment
            .blake3_hash
            .map(|bytes| bytes.to_vec())
            .unwrap_or_default();
        self.connection
            .execute(
                "INSERT INTO attachments(id, chat_id, message_id, file_name, size_bytes, blake3_hash, status, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) \
                 ON CONFLICT(id) DO UPDATE SET chat_id = excluded.chat_id, message_id = excluded.message_id, file_name = excluded.file_name, \
                 size_bytes = excluded.size_bytes, blake3_hash = excluded.blake3_hash, status = excluded.status, updated_ms = excluded.updated_ms",
                params![
                    attachment.id.to_string(),
                    attachment.chat_id,
                    attachment.message_id.map(|id| id.to_string()),
                    attachment.file_name,
                    to_ms(attachment.size_bytes),
                    hash,
                    attachment.state.as_str(),
                    to_ms(attachment.updated_ms)
                ],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Attachments of a chat, oldest first.
    pub fn attachments_for_chat(&self, chat_id: &str) -> Result<Vec<Attachment>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, chat_id, message_id, file_name, size_bytes, blake3_hash, status, updated_ms FROM attachments WHERE chat_id = ?1 ORDER BY updated_ms, id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                let message_id: Option<String> = row.get(2)?;
                let status: String = row.get(6)?;
                let hash: Vec<u8> = row.get(5)?;
                Ok(Attachment {
                    id: uuid_from(row, 0)?,
                    chat_id: row.get(1)?,
                    message_id: message_id.and_then(|raw| Uuid::parse_str(&raw).ok()),
                    file_name: row.get(3)?,
                    size_bytes: from_ms(row.get::<_, i64>(4)?),
                    blake3_hash: <[u8; 32]>::try_from(hash.as_slice()).ok(),
                    state: TransferState::parse(&status).unwrap_or(TransferState::Failed),
                    updated_ms: from_ms(row.get::<_, i64>(7)?),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Stores an announcement; false when it is already recorded.
    pub fn save_announcement(&mut self, announcement: &Announcement) -> Result<bool, String> {
        if announcement.group_id.trim().is_empty() || announcement.sender.trim().is_empty() {
            return Err("announcement needs a group and a sender".into());
        }
        if announcement.body.trim().is_empty() || announcement.body.len() > MAX_BODY_BYTES {
            return Err("announcement body is empty or too large".into());
        }
        let changed = self
            .connection
            .execute(
                "INSERT INTO announcements(id, group_id, sender, body, physical_ms, logical, admins_only, created_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(id) DO NOTHING",
                params![
                    announcement.id.to_string(),
                    announcement.group_id,
                    announcement.sender,
                    announcement.body,
                    to_ms(announcement.clock.physical_ms),
                    announcement.clock.logical,
                    announcement.admins_only,
                    to_ms(announcement.created_ms)
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    /// Records one acknowledgement per peer per announcement.
    ///
    /// `Err` reports the rule violation when a peer tries to acknowledge twice, so the
    /// sender-visible list stays stable; the stored row is never overwritten.
    pub fn acknowledge_announcement(
        &mut self,
        announcement_id: Uuid,
        peer_id: &str,
        acked_ms: u64,
    ) -> Result<bool, String> {
        if peer_id.trim().is_empty() {
            return Err("acknowledgement needs a peer".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        if !announcement_exists(&transaction, announcement_id)? {
            return Ok(false);
        }
        let acked = announcement_ack_peers(&transaction, announcement_id)?;
        if !acked.iter().any(|acked| acked == peer_id) {
            transaction
                .execute(
                    "INSERT INTO announcement_acks(announcement_id, peer_id, acked_ms) VALUES (?1, ?2, ?3)",
                    params![announcement_id.to_string(), peer_id, to_ms(acked_ms)],
                )
                .map_err(|error| error.to_string())?;
        }
        transaction.commit().map_err(|error| error.to_string())?;
        rules::check_announcement_ack(&acked, peer_id)
            .map_err(|violation| format!("{violation:?}"))?;
        Ok(true)
    }

    /// The announcement with its acknowledgement list and counts.
    pub fn acknowledgements_for_announcement(
        &self,
        announcement_id: Uuid,
    ) -> Result<Option<AnnouncementSummary>, String> {
        let announcement = self
            .connection
            .query_row(
                "SELECT id, group_id, sender, body, physical_ms, logical, admins_only, created_ms FROM announcements WHERE id = ?1",
                params![announcement_id.to_string()],
                |row| {
                    Ok(Announcement {
                        id: uuid_from(row, 0)?,
                        group_id: row.get(1)?,
                        sender: row.get(2)?,
                        body: row.get(3)?,
                        clock: Hlc {
                            physical_ms: from_ms(row.get::<_, i64>(4)?),
                            logical: row.get::<_, i64>(5)?.max(0) as u32,
                        },
                        admins_only: row.get::<_, i64>(6)? != 0,
                        created_ms: from_ms(row.get::<_, i64>(7)?),
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())?;
        let Some(announcement) = announcement else {
            return Ok(None);
        };
        let acks: Vec<AnnouncementAck> = announcement_ack_peers(&self.connection, announcement.id)?
            .into_iter()
            .map(|peer_id| AnnouncementAck {
                announcement_id: announcement.id,
                acked_ms: self.ack_ms(announcement.id, &peer_id).unwrap_or(0),
                peer_id,
            })
            .collect();
        let expected = self.announcement_expected(&announcement.group_id)?;
        Ok(Some(AnnouncementSummary {
            acked: acks.len(),
            expected,
            announcement,
            acks,
        }))
    }

    fn ack_ms(&self, announcement_id: Uuid, peer_id: &str) -> Result<u64, String> {
        self.connection
            .query_row(
                "SELECT acked_ms FROM announcement_acks WHERE announcement_id = ?1 AND peer_id = ?2",
                params![announcement_id.to_string(), peer_id],
                |row| row.get::<_, i64>(0),
            )
            .map(from_ms)
            .optional()
            .map(|value| value.unwrap_or(0))
            .map_err(|error| error.to_string())
    }

    fn announcement_expected(&self, group_id: &str) -> Result<usize, String> {
        let members: i64 = self
            .connection
            .query_row(
                "SELECT COUNT(*) FROM group_members WHERE group_id = ?1",
                params![group_id],
                |row| row.get(0),
            )
            .map_err(|error| error.to_string())?;
        if members > 0 {
            return Ok(members.max(0) as usize);
        }
        if group_id == EVERYONE_GROUP_ID {
            let peers: i64 = self
                .connection
                .query_row("SELECT COUNT(*) FROM peers", [], |row| row.get(0))
                .map_err(|error| error.to_string())?;
            return Ok(peers.max(0) as usize);
        }
        Ok(0)
    }

    /// Announcements of a group, newest first.
    pub fn announcements_for_group(&self, group_id: &str) -> Result<Vec<Announcement>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, group_id, sender, body, physical_ms, logical, admins_only, created_ms FROM announcements WHERE group_id = ?1 ORDER BY physical_ms DESC, logical DESC, id DESC")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![group_id], |row| {
                Ok(Announcement {
                    id: uuid_from(row, 0)?,
                    group_id: row.get(1)?,
                    sender: row.get(2)?,
                    body: row.get(3)?,
                    clock: Hlc {
                        physical_ms: from_ms(row.get::<_, i64>(4)?),
                        logical: row.get::<_, i64>(5)?.max(0) as u32,
                    },
                    admins_only: row.get::<_, i64>(6)? != 0,
                    created_ms: from_ms(row.get::<_, i64>(7)?),
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Persists a call signalling event; false when the row already exists.
    pub fn save_call_event(&mut self, event: &CallEvent) -> Result<bool, String> {
        if event.chat_id.trim().is_empty() || event.peer_id.trim().is_empty() {
            return Err("call events need a chat and a peer".into());
        }
        if event.note.len() > MAX_BODY_BYTES {
            return Err("call note is too large".into());
        }
        let changed = self
            .connection
            .execute(
                "INSERT INTO call_events(id, chat_id, call_id, peer_id, kind, accepted, started_ms, ended_ms, note) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(id) DO NOTHING",
                params![
                    event.id.to_string(),
                    event.chat_id,
                    event.call_id.to_string(),
                    event.peer_id,
                    event.kind.as_str(),
                    event.accepted,
                    to_ms(event.started_ms),
                    event.ended_ms.map(to_ms),
                    event.note
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(changed == 1)
    }

    /// Call history of a chat, oldest first.
    pub fn calls_for_chat(&self, chat_id: &str) -> Result<Vec<CallEvent>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT id, chat_id, call_id, peer_id, kind, accepted, started_ms, ended_ms, note FROM call_events WHERE chat_id = ?1 ORDER BY started_ms, id")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![chat_id], |row| {
                let kind: String = row.get(4)?;
                let Some(kind) = CallEventKind::parse(&kind) else {
                    return Err(rusqlite::Error::InvalidColumnName(format!(
                        "unknown call event kind {kind:?}"
                    )));
                };
                Ok(CallEvent {
                    id: uuid_from(row, 0)?,
                    chat_id: row.get(1)?,
                    call_id: uuid_from(row, 2)?,
                    peer_id: row.get(3)?,
                    kind,
                    accepted: row.get::<_, Option<i64>>(5)?.map(|value| value != 0),
                    started_ms: from_ms(row.get::<_, i64>(6)?),
                    ended_ms: row.get::<_, Option<i64>>(7)?.map(from_ms),
                    note: row.get(8)?,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Stores a typed setting as JSON text.
    pub fn set_setting<T: Serialize>(
        &mut self,
        key: &str,
        value: &T,
        updated_ms: u64,
    ) -> Result<(), String> {
        if key.trim().is_empty() || key.len() > 128 {
            return Err("setting key is empty or too long".into());
        }
        let encoded = serde_json::to_string(value).map_err(|error| error.to_string())?;
        self.connection
            .execute(
                "INSERT INTO settings(key, value, updated_ms) VALUES (?1, ?2, ?3) ON CONFLICT(key) DO UPDATE SET value = excluded.value, updated_ms = excluded.updated_ms",
                params![key, encoded, to_ms(updated_ms)],
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    /// Reads a typed setting; `None` when unset or not decodable as `T`.
    pub fn get_setting<T: DeserializeOwned>(&self, key: &str) -> Result<Option<T>, String> {
        Ok(self
            .get_setting_raw(key)?
            .and_then(|text| serde_json::from_str(&text).ok()))
    }

    pub fn get_setting_raw(&self, key: &str) -> Result<Option<String>, String> {
        self.connection
            .query_row(
                "SELECT value FROM settings WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Typed setting with a fallback used when the key is unset or malformed.
    pub fn get_setting_or<T>(&self, key: &str, fallback: T) -> Result<T, String>
    where
        T: DeserializeOwned,
    {
        Ok(self.get_setting::<T>(key)?.unwrap_or(fallback))
    }

    pub fn all_settings(&self) -> Result<BTreeMap<String, String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT key, value FROM settings ORDER BY key")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    pub fn delete_setting(&mut self, key: &str) -> Result<bool, String> {
        self.connection
            .execute("DELETE FROM settings WHERE key = ?1", params![key])
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// The configured edit window, defaulting to
    /// [`crate::rules::DEFAULT_EDIT_WINDOW_MS`].
    pub fn edit_window(&self) -> Result<EditWindow, String> {
        Ok(EditWindow::new(self.get_setting_or::<u64>(
            setting::EDIT_WINDOW_MS,
            rules::DEFAULT_EDIT_WINDOW_MS,
        )?))
    }

    /// The configured maximum accepted file size in bytes.
    pub fn max_file_bytes(&self) -> Result<u64, String> {
        self.get_setting_or::<u64>(
            setting::MAX_FILE_BYTES,
            crate::files::DEFAULT_MAX_FILE_BYTES,
        )
    }

    /// The configured auto-download limit in bytes; 0 means always ask first.
    pub fn auto_download_limit_bytes(&self) -> Result<u64, String> {
        self.get_setting_or::<u64>(setting::AUTO_DOWNLOAD_LIMIT_BYTES, 0)
    }

    /// The size policy a [`crate::files::Transfer`] must be built with, straight from
    /// the two file settings.
    pub fn transfer_policy(&self) -> Result<crate::files::TransferPolicy, String> {
        Ok(crate::files::TransferPolicy::new(
            self.max_file_bytes()?,
            self.auto_download_limit_bytes()?,
        ))
    }

    /// Whether announcements require admin authority in this install.
    pub fn announcements_are_admins_only(&self) -> Result<bool, String> {
        self.get_setting_or::<bool>(setting::ANNOUNCEMENTS_ADMINS_ONLY, false)
    }

    /// Avatar budget taken from settings, falling back to the compiled defaults.
    pub fn avatar_limits(&self) -> Result<AvatarLimits, String> {
        Ok(AvatarLimits {
            max_bytes: self
                .get_setting_or::<usize>(setting::AVATAR_MAX_BYTES, security::MAX_AVATAR_BYTES)?,
            max_dimension: self.get_setting_or::<u32>(
                setting::AVATAR_MAX_DIMENSION,
                security::MAX_AVATAR_DIMENSION,
            )?,
            max_pixels: self
                .get_setting_or::<u64>(setting::AVATAR_MAX_PIXELS, security::MAX_AVATAR_PIXELS)?,
            ..AvatarLimits::default()
        })
    }

    /// Applies the configured retention window and reports the purge cutoff.
    pub fn retention_cutoff_ms(&self, now_ms: u64) -> Result<u64, String> {
        let days = self.get_setting_or::<u64>(setting::RETENTION_DAYS, 0)?;
        let window = days.saturating_mul(24 * 60 * 60 * 1_000);
        Ok(now_ms.saturating_sub(window))
    }

    /// Caches a peer avatar, after the pure decode limits have accepted it.
    pub fn cache_avatar(
        &mut self,
        peer_id: &str,
        bytes: &[u8],
        width: u32,
        height: u32,
        media_type: &str,
        updated_ms: u64,
    ) -> Result<Avatar, String> {
        if peer_id.trim().is_empty() {
            return Err("avatar cache needs a peer".into());
        }
        let limits = self.avatar_limits()?;
        let info = security::validate_avatar_image(bytes, width, height, media_type, &limits)
            .map_err(|error| format!("{error:?}"))?;
        let hash = blake3::hash(bytes).to_hex().to_string();
        let stored = Avatar {
            peer_id: peer_id.to_owned(),
            media_type: info.media_type.to_owned(),
            width: info.width,
            height: info.height,
            hash: hash.clone(),
            bytes: bytes.to_vec(),
            updated_ms,
        };
        self.connection
            .execute(
                "INSERT INTO avatar_cache(peer_id, media_type, width, height, hash, bytes, updated_ms) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(peer_id) DO UPDATE SET media_type = excluded.media_type, width = excluded.width, height = excluded.height, hash = excluded.hash, bytes = excluded.bytes, updated_ms = excluded.updated_ms",
                params![
                    stored.peer_id,
                    stored.media_type,
                    stored.width,
                    stored.height,
                    hash,
                    stored.bytes.as_slice(),
                    to_ms(updated_ms)
                ],
            )
            .map_err(|error| error.to_string())?;
        Ok(stored)
    }

    pub fn avatar_for_peer(&self, peer_id: &str) -> Result<Option<Avatar>, String> {
        self.connection
            .query_row(
                "SELECT media_type, width, height, hash, bytes, updated_ms FROM avatar_cache WHERE peer_id = ?1",
                params![peer_id],
                |row| {
                    Ok(Avatar {
                        peer_id: peer_id.to_owned(),
                        media_type: row.get(0)?,
                        width: row.get::<_, i64>(1)?.max(0) as u32,
                        height: row.get::<_, i64>(2)?.max(0) as u32,
                        hash: row.get(3)?,
                        bytes: row.get(4)?,
                        updated_ms: from_ms(row.get::<_, i64>(5)?),
                    })
                },
            )
            .optional()
            .map_err(|error| error.to_string())
    }

    /// Forgets one peer's cached avatar; false when nothing was stored.
    pub fn forget_avatar(&mut self, peer_id: &str) -> Result<bool, String> {
        self.connection
            .execute(
                "DELETE FROM avatar_cache WHERE peer_id = ?1",
                params![peer_id],
            )
            .map(|count| count == 1)
            .map_err(|error| error.to_string())
    }

    /// The oldest received message `reader_id` has not read, if any.
    pub fn first_unread_message_for_chat(
        &self,
        chat_id: &str,
        reader_id: &str,
    ) -> Result<Option<Uuid>, String> {
        self.connection
            .query_row(
                "SELECT id FROM messages \
                 WHERE chat_id = ?1 AND tombstone = 0 AND sender <> ?2 \
                 AND NOT EXISTS (SELECT 1 FROM receipts WHERE receipts.message_id = messages.id \
                                 AND receipts.peer_id = ?2 AND receipts.kind = 'read') \
                 ORDER BY physical_ms, logical, id LIMIT 1",
                params![chat_id, reader_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?
            .map(|raw| {
                Uuid::parse_str(&raw)
                    .map(Some)
                    .map_err(|error| error.to_string())
            })
            .unwrap_or(Ok(None))
    }

    /// Drops cached avatars older than `older_than_ms`; returns how many went.
    pub fn prune_avatars(&mut self, older_than_ms: u64) -> Result<usize, String> {
        self.connection
            .execute(
                "DELETE FROM avatar_cache WHERE updated_ms < ?1",
                params![to_ms(older_than_ms)],
            )
            .map_err(|error| error.to_string())
    }

    /// Deletes every message of a chat, keeping the chat and peer rows.
    ///
    /// Reactions, receipts, pins and stars follow through `ON DELETE CASCADE`, and the
    /// FTS index is emptied for exactly the same messages in the same transaction.
    pub fn clear_chat_history(&mut self, chat_id: &str) -> Result<u64, String> {
        if chat_id.trim().is_empty() {
            return Err("chat id is empty".into());
        }
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let removed = purge_messages_of_chat(&transaction, chat_id)?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(removed)
    }

    /// Hard-deletes messages older than `older_than_ms` across every chat.
    ///
    /// Returns the number of purged messages and keeps FTS, pins, stars, reactions,
    /// receipts and attachments consistent.
    pub fn apply_retention(&mut self, older_than_ms: u64) -> Result<u64, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        transaction
            .execute(
                "DELETE FROM message_search WHERE message_id IN (SELECT id FROM messages WHERE physical_ms < ?1)",
                params![to_ms(older_than_ms)],
            )
            .map_err(|error| error.to_string())?;
        let removed = transaction
            .execute(
                "DELETE FROM messages WHERE physical_ms < ?1",
                params![to_ms(older_than_ms)],
            )
            .map(|count| count as u64)
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM attachments WHERE message_id IS NOT NULL AND message_id NOT IN (SELECT id FROM messages)",
                [],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(removed)
    }

    /// Edit with the configured window enforced; `Err` names the refused rule.
    ///
    /// `now_ms` is supplied by the caller, never read from a clock here, so the decision
    /// is reproducible in unit and property tests.
    pub fn edit_message_in_window(
        &mut self,
        id: Uuid,
        actor: &str,
        body: &str,
        clock: Hlc,
        now_ms: u64,
    ) -> Result<bool, String> {
        let window = self.edit_window()?;
        let Some(message) = self.get_message(id)? else {
            return Ok(false);
        };
        window
            .allows_edit(
                &message.sender,
                actor,
                message.tombstone,
                message.clock.physical_ms,
                now_ms,
            )
            .map_err(|violation| format!("{violation:?}"))?;
        self.edit_message(id, actor, body, clock)
    }

    /// Delete-for-everyone driven by group-admin authority.
    ///
    /// The sender may always delete their own message; the admin of `group_id` may
    /// delete anyone's message inside that group; nobody else can, and a tombstone is
    /// never applied twice.
    pub fn admin_tombstone_in_group(
        &mut self,
        id: Uuid,
        actor: &str,
        group_id: &str,
    ) -> Result<bool, String> {
        let transaction = self.connection.transaction().map_err(|e| e.to_string())?;
        let Some(message) = Self::read_message(&transaction, id)? else {
            return Ok(false);
        };
        if message.chat_id != group_id {
            return Ok(false);
        }
        let admin: Option<String> = transaction
            .query_row(
                "SELECT peer_id FROM group_members WHERE group_id = ?1 AND role = 'admin'",
                params![group_id],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| error.to_string())?;
        rules::tombstone_authority(&message.sender, actor, admin.as_deref())
            .map_err(|violation| format!("{violation:?}"))?;
        if !rules::accepts_updates(message.tombstone) {
            return Ok(false);
        }
        transaction
            .execute(
                "UPDATE messages SET body = '', tombstone = 1 WHERE id = ?1 AND tombstone = 0",
                params![id.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction
            .execute(
                "DELETE FROM message_search WHERE message_id = ?1",
                params![id.to_string()],
            )
            .map_err(|error| error.to_string())?;
        transaction.commit().map_err(|error| error.to_string())?;
        Ok(true)
    }

    /// Every persisted outbox row, ready for [`crate::outbox::replay_plan`].
    pub fn outbox_rows(&self) -> Result<Vec<OutboxEntry>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT peer_id, envelope, attempt FROM outbox ORDER BY rowid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map([], |row| {
                let peer_id: String = row.get(0)?;
                let bytes: Vec<u8> = row.get(1)?;
                let attempt: i64 = row.get(2)?;
                let envelope: Envelope<serde_json::Value> = serde_json::from_slice(&bytes)
                    .map_err(|error| {
                        rusqlite::Error::FromSqlConversionFailure(
                            1,
                            rusqlite::types::Type::Blob,
                            Box::new(error),
                        )
                    })?;
                Ok(OutboxEntry {
                    id: envelope.id,
                    peer_id,
                    envelope,
                    attempt: attempt.max(0) as u32,
                })
            })
            .map_err(|error| error.to_string())?;
        rows.map(|row| row.map_err(|error| error.to_string()))
            .collect()
    }

    /// Records a failed delivery attempt and returns the new count.
    pub fn bump_outbox_attempt(&mut self, id: Uuid) -> Result<u32, String> {
        let updated = self
            .connection
            .execute(
                "UPDATE outbox SET attempt = attempt + 1 WHERE id = ?1",
                params![id.to_string()],
            )
            .map_err(|error| error.to_string())?;
        if updated == 0 {
            return Ok(0);
        }
        self.connection
            .query_row(
                "SELECT attempt FROM outbox WHERE id = ?1",
                params![id.to_string()],
                |row| row.get::<_, i64>(0),
            )
            .map(|value| value.max(0) as u32)
            .map_err(|error| error.to_string())
    }

    /// Messages still present in the FTS index; the consistency check for deletions.
    pub fn indexed_message_count(&self) -> Result<u64, String> {
        self.connection
            .query_row("SELECT COUNT(*) FROM message_search", [], |row| {
                row.get::<_, i64>(0)
            })
            .map(|value| value.max(0) as u64)
            .map_err(|error| error.to_string())
    }

    /// Append-only edit history for one message, oldest revision first. A tombstone is
    /// a state, not a deletion, so deleted-for-everyone messages keep their revisions.
    pub fn revision_history(&self, message_id: Uuid) -> Result<Vec<(u64, String)>, String> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT revision, body FROM message_revisions WHERE message_id = ?1 \
                 ORDER BY revision",
            )
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![message_id.to_string()], |row| {
                Ok((row.get::<_, i64>(0)? as u64, row.get::<_, String>(1)?))
            })
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    /// Column names of a table in declaration order. The migration tests use it to prove
    /// the v1 `messages` shape never changes under the desktop layer's feet.
    pub fn table_columns(&self, table: &str) -> Result<Vec<String>, String> {
        let mut statement = self
            .connection
            .prepare("SELECT name FROM pragma_table_info(?1) ORDER BY cid")
            .map_err(|error| error.to_string())?;
        let rows = statement
            .query_map(params![table], |row| row.get::<_, String>(0))
            .map_err(|error| error.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())
    }

    fn read_message(connection: &Connection, id: Uuid) -> Result<Option<Message>, String> {
        connection
            .query_row(
                &format!("SELECT {MESSAGE_COLUMNS} FROM messages WHERE id = ?1"),
                params![id.to_string()],
                |row| message_from_row(row, 0),
            )
            .optional()
            .map_err(|error| error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::groups::GroupState;
    use crate::identity::Identity;

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut bytes = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        bytes.extend_from_slice(&13_u32.to_be_bytes());
        bytes.extend_from_slice(b"IHDR");
        bytes.extend_from_slice(&width.to_be_bytes());
        bytes.extend_from_slice(&height.to_be_bytes());
        bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
        bytes
    }

    fn store() -> Store {
        Store::open(":memory:", &[7; 32]).expect("encrypted store opens")
    }

    fn message(chat_id: &str, sender: &str, body: &str, at: u64) -> Message {
        Message {
            id: Uuid::now_v7(),
            chat_id: chat_id.into(),
            sender: sender.into(),
            body: body.into(),
            clock: Hlc {
                physical_ms: at,
                logical: 0,
            },
            tombstone: false,
        }
    }

    #[test]
    fn reactions_round_trip_and_a_tombstone_blocks_late_reactions() {
        let mut store = store();
        let message = message("chat-1", "peer-a", "react to me", 10);
        store.save_message(&message).unwrap();
        assert!(store
            .add_reaction(message.id, "peer-b", "thumbs", 11)
            .unwrap());
        assert!(
            !store
                .add_reaction(message.id, "peer-b", "thumbs", 12)
                .unwrap(),
            "the same reaction is idempotent"
        );
        assert!(store
            .add_reaction(message.id, "peer-c", "heart", 13)
            .unwrap());
        assert_eq!(
            store.reactions_for_message(message.id).unwrap(),
            vec![
                Reaction {
                    message_id: message.id,
                    peer_id: "peer-b".into(),
                    emoji: "thumbs".into(),
                    created_ms: 11
                },
                Reaction {
                    message_id: message.id,
                    peer_id: "peer-c".into(),
                    emoji: "heart".into(),
                    created_ms: 13
                }
            ]
        );
        assert!(store
            .remove_reaction(message.id, "peer-b", "thumbs")
            .unwrap());
        assert!(!store
            .remove_reaction(message.id, "peer-b", "thumbs")
            .unwrap());
        // Late reactions for unknown or tombstoned messages are ignored, never stored.
        assert!(!store
            .add_reaction(Uuid::now_v7(), "peer-b", "late", 1)
            .unwrap());
        assert!(store.tombstone_message(message.id, "peer-a").unwrap());
        assert!(!store
            .add_reaction(message.id, "peer-b", "late", 20)
            .unwrap());
        assert!(store.reactions_for_message(message.id).unwrap().is_empty());
        assert!(store.add_reaction(message.id, " ", "emoji", 20).is_err());
    }

    #[test]
    fn receipts_are_idempotent_monotonic_and_never_downgraded() {
        let mut store = store();
        let message = message("chat-1", "peer-a", "delivered then read", 10);
        store.save_message(&message).unwrap();
        assert!(store
            .record_receipt(message.id, "peer-b", ReceiptKind::Delivered, 11)
            .unwrap());
        assert!(
            !store
                .record_receipt(message.id, "peer-b", ReceiptKind::Delivered, 12)
                .unwrap(),
            "repeating a level changes nothing"
        );
        assert!(store
            .record_receipt(message.id, "peer-c", ReceiptKind::Read, 13)
            .unwrap());
        assert_eq!(
            store.receipts_for_message(message.id).unwrap(),
            vec![
                Receipt {
                    message_id: message.id,
                    peer_id: "peer-b".into(),
                    kind: ReceiptKind::Delivered,
                    occurred_ms: 11
                },
                Receipt {
                    message_id: message.id,
                    peer_id: "peer-c".into(),
                    kind: ReceiptKind::Read,
                    occurred_ms: 13
                }
            ]
        );
        // A read receipt may upgrade delivered, and delivered may never downgrade read.
        assert!(store
            .record_receipt(message.id, "peer-b", ReceiptKind::Read, 20)
            .unwrap());
        assert!(!store
            .record_receipt(message.id, "peer-b", ReceiptKind::Delivered, 21)
            .unwrap());
        assert_eq!(
            store.receipts_for_message(message.id).unwrap()[0].kind,
            ReceiptKind::Read
        );
        assert_eq!(
            store.receipts_for_message(message.id).unwrap()[0].occurred_ms,
            20,
            "the refused downgrade did not move the clock"
        );
        assert!(!store
            .record_receipt(Uuid::now_v7(), "peer-b", ReceiptKind::Read, 1)
            .unwrap());
        assert!(store
            .record_receipt(message.id, " ", ReceiptKind::Read, 1)
            .is_err());
        assert!(store.tombstone_message(message.id, "peer-a").unwrap());
        assert!(
            !store
                .record_receipt(message.id, "peer-z", ReceiptKind::Delivered, 30)
                .unwrap(),
            "receipts cannot resurrect a tombstone"
        );
    }

    #[test]
    fn pins_and_stars_are_scoped_to_their_chat() {
        let mut store = store();
        let first = message("chat-1", "peer-a", "first", 10);
        let second = message("chat-1", "peer-a", "second", 20);
        let elsewhere = message("chat-2", "peer-a", "elsewhere", 30);
        for saved in [&first, &second, &elsewhere] {
            store.save_message(saved).unwrap();
        }
        assert!(store.pin(second.id, 1, 40).unwrap());
        assert!(store.pin(first.id, 0, 41).unwrap());
        assert!(store.pin(elsewhere.id, 5, 42).unwrap());
        let pinned = store.pinned_for_chat("chat-1").unwrap();
        assert_eq!(
            pinned.iter().map(|pin| pin.message.id).collect::<Vec<_>>(),
            vec![first.id, second.id]
        );
        assert_eq!(pinned[0].position, 0);
        // Repositioning is an update, not a second row.
        assert!(store.pin(second.id, 0, 50).unwrap());
        assert_eq!(store.pinned_for_chat("chat-1").unwrap().len(), 2);
        assert!(store.unpin(first.id).unwrap());
        assert!(!store.unpin(first.id).unwrap());
        assert_eq!(
            store.pinned_for_chat("chat-1").unwrap()[0].message.id,
            second.id
        );

        assert!(store.star(first.id, 60).unwrap());
        assert!(store.star(elsewhere.id, 61).unwrap());
        assert_eq!(
            store.starred_for_chat("chat-1").unwrap()[0].message.id,
            first.id
        );
        assert_eq!(store.starred_for_chat("chat-2").unwrap().len(), 1);
        assert!(store.unstar(elsewhere.id).unwrap());
        assert!(store.starred_for_chat("chat-2").unwrap().is_empty());

        // Pins and stars never attach to a tombstone or an unknown message.
        assert!(store.tombstone_message(second.id, "peer-a").unwrap());
        assert!(!store.pin(second.id, 9, 70).unwrap());
        assert!(!store.star(second.id, 70).unwrap());
        assert!(!store.pin(Uuid::now_v7(), 0, 70).unwrap());
        // Deleting the message takes its pin and star with it.
        assert!(store.clear_chat_history("chat-1").unwrap() >= 2);
        assert!(store.pinned_for_chat("chat-1").unwrap().is_empty());
        assert!(store.starred_for_chat("chat-1").unwrap().is_empty());
    }

    #[test]
    fn attachments_upsert_and_reject_traversal_names() {
        let mut store = store();
        let chat = "chat-1";
        let message = message(chat, "peer-a", "with a file", 10);
        store.save_message(&message).unwrap();
        let hash = *blake3::hash(b"bytes").as_bytes();
        let attachment = Attachment {
            id: Uuid::now_v7(),
            chat_id: chat.into(),
            message_id: Some(message.id),
            file_name: "spec.pdf".into(),
            size_bytes: 4_096,
            blake3_hash: Some(hash),
            state: TransferState::Offered,
            updated_ms: 20,
        };
        store.upsert_attachment(&attachment).unwrap();
        store.upsert_attachment(&attachment).unwrap();
        let mut advanced = attachment.clone();
        advanced.state = TransferState::Completed;
        advanced.updated_ms = 30;
        store.upsert_attachment(&advanced).unwrap();
        let listed = store.attachments_for_chat(chat).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0], advanced);
        assert_eq!(listed[0].blake3_hash, Some(hash));
        assert!(store.attachments_for_chat("missing").unwrap().is_empty());
        for hostile in ["../escape.pdf", "/etc/passwd", "nul", "a\x00b"] {
            let mut bad = attachment.clone();
            bad.file_name = hostile.into();
            assert!(
                store.upsert_attachment(&bad).is_err(),
                "{hostile:?} must be refused"
            );
        }
        let mut no_chat = attachment.clone();
        no_chat.chat_id = String::new();
        assert!(store.upsert_attachment(&no_chat).is_err());
        // Retention drops attachment rows whose message is gone.
        assert!(store.clear_chat_history(chat).unwrap() >= 1);
        assert!(store.attachments_for_chat(chat).unwrap().is_empty());
    }

    #[test]
    fn announcements_expose_sender_visible_acks_and_counts() {
        let admin = Identity::from_secret([21; 32]);
        let mut store = store();
        let state = GroupState::create(
            &admin,
            "group-announce".into(),
            "Standup".into(),
            ["peer-b".into(), "peer-c".into()],
        )
        .unwrap();
        store.save_group_state(&state).unwrap();
        let announcement = Announcement {
            id: Uuid::now_v7(),
            group_id: state.id.clone(),
            sender: admin.peer_id(),
            body: "freeze on monday".into(),
            clock: Hlc {
                physical_ms: 5,
                logical: 0,
            },
            admins_only: true,
            created_ms: 5,
        };
        assert!(store.save_announcement(&announcement).unwrap());
        assert!(!store.save_announcement(&announcement).unwrap());
        assert_eq!(
            store.announcements_for_group(&state.id).unwrap(),
            vec![announcement.clone()]
        );
        let summary = store
            .acknowledgements_for_announcement(announcement.id)
            .unwrap()
            .expect("stored");
        assert_eq!(summary.announcement, announcement);
        assert_eq!(summary.acked, 0);
        assert_eq!(summary.expected, 3, "admin plus two members");
        assert!(store
            .acknowledge_announcement(announcement.id, "peer-b", 10)
            .unwrap());
        assert!(store
            .acknowledge_announcement(announcement.id, "peer-c", 11)
            .unwrap());
        assert!(
            store
                .acknowledge_announcement(announcement.id, "peer-b", 12)
                .is_err(),
            "one acknowledgement per peer per announcement"
        );
        let summary = store
            .acknowledgements_for_announcement(announcement.id)
            .unwrap()
            .expect("stored");
        assert_eq!(summary.acked, 2);
        assert_eq!(
            summary.acks,
            vec![
                AnnouncementAck {
                    announcement_id: announcement.id,
                    peer_id: "peer-b".into(),
                    acked_ms: 10
                },
                AnnouncementAck {
                    announcement_id: announcement.id,
                    peer_id: "peer-c".into(),
                    acked_ms: 11
                }
            ]
        );
        assert!(!store
            .acknowledge_announcement(Uuid::now_v7(), "peer-b", 1)
            .unwrap());
        let empty = Announcement {
            body: "  ".into(),
            ..announcement.clone()
        };
        assert!(store.save_announcement(&empty).is_err());
    }

    #[test]
    fn everyone_announcements_count_known_peers() {
        let mut store = store();
        let peer = Identity::from_secret([22; 32]);
        store
            .save_peer(&peer.peer_id(), "Ada", &peer.verifying_key().to_bytes(), 1)
            .unwrap();
        let announcement = Announcement {
            id: Uuid::now_v7(),
            group_id: EVERYONE_GROUP_ID.into(),
            sender: "local".into(),
            body: "all hands".into(),
            clock: Hlc::default(),
            admins_only: false,
            created_ms: 0,
        };
        store.save_announcement(&announcement).unwrap();
        let summary = store
            .acknowledgements_for_announcement(announcement.id)
            .unwrap()
            .expect("stored");
        assert_eq!(summary.expected, 1);
        assert_eq!(summary.acked, 0);
    }

    #[test]
    fn call_events_round_trip_in_order() {
        let mut store = store();
        let call_id = Uuid::now_v7();
        let chat = "chat-1";
        store
            .save_message(&message(chat, "peer-a", "before the call", 1))
            .unwrap();
        let events = [
            CallEvent {
                id: Uuid::now_v7(),
                chat_id: chat.into(),
                call_id,
                peer_id: "peer-a".into(),
                kind: CallEventKind::Offer,
                accepted: None,
                started_ms: 100,
                ended_ms: None,
                note: "audio".into(),
            },
            CallEvent {
                id: Uuid::now_v7(),
                chat_id: chat.into(),
                call_id,
                peer_id: "peer-a".into(),
                kind: CallEventKind::Answer,
                accepted: Some(true),
                started_ms: 110,
                ended_ms: None,
                note: String::new(),
            },
            CallEvent {
                id: Uuid::now_v7(),
                chat_id: chat.into(),
                call_id,
                peer_id: "peer-a".into(),
                kind: CallEventKind::End,
                accepted: Some(false),
                started_ms: 120,
                ended_ms: Some(180),
                note: "hangup".into(),
            },
        ];
        for event in &events {
            assert!(store.save_call_event(event).unwrap());
            assert!(!store.save_call_event(event).unwrap());
        }
        assert_eq!(store.calls_for_chat(chat).unwrap(), events.to_vec());
        assert!(store.calls_for_chat("missing").unwrap().is_empty());
        let mut bad = events[0].clone();
        bad.chat_id = String::new();
        assert!(store.save_call_event(&bad).is_err());
    }

    #[test]
    fn settings_are_typed_string_backed() {
        let mut store = store();
        assert_eq!(store.edit_window().unwrap(), EditWindow::default());
        store
            .set_setting(setting::EDIT_WINDOW_MS, &60_000_u64, 1)
            .unwrap();
        assert_eq!(store.edit_window().unwrap(), EditWindow::new(60_000));
        assert_eq!(
            store
                .get_setting_raw(setting::EDIT_WINDOW_MS)
                .unwrap()
                .as_deref(),
            Some("60000"),
            "values are stored as JSON text"
        );
        store
            .set_setting(setting::ANNOUNCEMENTS_ADMINS_ONLY, &true, 2)
            .unwrap();
        assert!(store.announcements_are_admins_only().unwrap());
        store
            .set_setting(setting::AUTO_DOWNLOAD_LIMIT_BYTES, &1_024_u64, 3)
            .unwrap();
        store
            .set_setting(setting::MAX_FILE_BYTES, &4_096_u64, 3)
            .unwrap();
        assert_eq!(store.auto_download_limit_bytes().unwrap(), 1_024);
        assert_eq!(store.max_file_bytes().unwrap(), 4_096);
        assert_eq!(
            store.get_setting::<u64>(setting::RATE_LIMIT_BURST).unwrap(),
            None
        );
        assert_eq!(
            store
                .get_setting_or::<u64>(setting::RATE_LIMIT_BURST, 42)
                .unwrap(),
            42
        );
        // Malformed JSON falls back instead of failing the caller.
        store
            .connection
            .execute(
                "INSERT INTO settings(key, value) VALUES (?1, 'not json') ON CONFLICT(key) DO UPDATE SET value = 'not json'",
                params![setting::RETENTION_DAYS],
            )
            .unwrap();
        assert_eq!(
            store
                .get_setting_or::<u64>(setting::RETENTION_DAYS, 7)
                .unwrap(),
            7
        );
        assert_eq!(store.retention_cutoff_ms(1_000), Ok(1_000));
        assert!(store.delete_setting(setting::MAX_FILE_BYTES).unwrap());
        assert_eq!(
            store.max_file_bytes().unwrap(),
            crate::files::DEFAULT_MAX_FILE_BYTES
        );
        let keys: Vec<String> = store.all_settings().unwrap().into_keys().collect();
        assert!(keys.contains(&setting::EDIT_WINDOW_MS.to_owned()));
        assert!(store.set_setting("", &1_u64, 0).is_err());
    }

    #[test]
    fn the_edit_window_setting_gates_edits() {
        let mut store = store();
        let original = message("chat-1", "peer-a", "original", 1_000);
        store.save_message(&original).unwrap();
        store
            .set_setting(setting::EDIT_WINDOW_MS, &500_u64, 0)
            .unwrap();
        assert_eq!(store.edit_window().unwrap(), EditWindow::new(500));
        let clock = Hlc {
            physical_ms: 1_400,
            logical: 0,
        };
        assert!(store
            .edit_message_in_window(original.id, "peer-a", "in time", clock, 1_400)
            .unwrap());
        let late_clock = Hlc {
            physical_ms: 2_000,
            logical: 0,
        };
        assert!(store
            .edit_message_in_window(original.id, "peer-a", "too late", late_clock, 2_000)
            .unwrap_err()
            .contains("EditWindowExpired"));
        assert!(store
            .edit_message_in_window(original.id, "peer-b", "not mine", clock, 1_400)
            .unwrap_err()
            .contains("NotSender"));
        assert!(!store
            .edit_message_in_window(Uuid::now_v7(), "peer-a", "ghost", clock, 1_400)
            .unwrap());
        // Boundary: exactly at the window edge is allowed, one ms past is not. Each case
        // needs an untouched message because an edit moves the message clock forward.
        let edge = message("chat-1", "peer-a", "at the edge", 1_000);
        store.save_message(&edge).unwrap();
        assert!(store
            .edit_message_in_window(edge.id, "peer-a", "edge", clock, 1_500)
            .is_ok());
        let past = message("chat-1", "peer-a", "one ms late", 1_000);
        store.save_message(&past).unwrap();
        assert!(store
            .edit_message_in_window(past.id, "peer-a", "past", late_clock, 1_501)
            .unwrap_err()
            .contains("EditWindowExpired"));
        // A window of zero closes as soon as a millisecond has passed.
        store
            .set_setting(setting::EDIT_WINDOW_MS, &0_u64, 1)
            .unwrap();
        assert!(store
            .edit_message_in_window(past.id, "peer-a", "closed", late_clock, 1_001)
            .unwrap_err()
            .contains("EditWindowExpired"));
        // A tombstoned message can never be edited, however early the request arrives.
        assert!(store.tombstone_message(original.id, "peer-a").unwrap());
        assert!(store
            .edit_message_in_window(original.id, "peer-a", "resurrect", clock, 1_400)
            .unwrap_err()
            .contains("Tombstoned"));
    }

    #[test]
    fn avatars_are_validated_before_caching_and_survive_reopen() {
        let mut store = store();
        let bytes = png(128, 128);
        let avatar = store
            .cache_avatar("peer-a", &bytes, 128, 128, "image/png", 10)
            .unwrap();
        assert_eq!(avatar.width, 128);
        assert_eq!(avatar.height, 128);
        assert_eq!(avatar.media_type, "image/png");
        assert_eq!(avatar.hash, blake3::hash(&bytes).to_hex().to_string());
        assert_eq!(avatar.bytes, bytes);
        assert_eq!(store.avatar_for_peer("peer-a").unwrap().unwrap(), avatar);
        assert!(store.avatar_for_peer("peer-z").unwrap().is_none());
        // Oversized dimensions and mismatched metadata are refused.
        assert!(store
            .cache_avatar("peer-b", &png(4_096, 4_096), 4_096, 4_096, "image/png", 11)
            .unwrap_err()
            .contains("ImageDimensions"));
        assert!(store
            .cache_avatar("peer-b", &bytes, 512, 512, "image/png", 11)
            .unwrap_err()
            .contains("DimensionMismatch"));
        assert!(store
            .cache_avatar("peer-b", b"junk", 8, 8, "image/png", 11)
            .unwrap_err()
            .contains("UnrecognizedImage"));
        assert!(store
            .cache_avatar("", &bytes, 128, 128, "image/png", 11)
            .is_err());
        // Settings lower the budget for the whole install.
        store
            .set_setting(setting::AVATAR_MAX_BYTES, &8_usize, 0)
            .unwrap();
        assert_eq!(store.avatar_limits().unwrap().max_bytes, 8);
        assert!(store
            .cache_avatar("peer-c", &bytes, 128, 128, "image/png", 12)
            .unwrap_err()
            .contains("TooManyBytes"));
        assert_eq!(store.prune_avatars(u64::from(u32::MAX)).unwrap(), 1);
        assert!(store.avatar_for_peer("peer-a").unwrap().is_none());
    }

    #[test]
    fn clearing_history_and_retention_keep_fts_consistent() {
        let mut store = store();
        let old = message("chat-1", "peer-a", "ancient message", 10);
        let recent = message("chat-1", "peer-a", "recent message", 1_000);
        let other = message("chat-2", "peer-b", "ancient in another chat", 20);
        for saved in [&old, &recent, &other] {
            store.save_message(saved).unwrap();
        }
        assert_eq!(store.indexed_message_count().unwrap(), 3);
        assert_eq!(store.search_messages("ancient", 10).unwrap().len(), 2);
        // Retention cuts by time, not by chat: both pre-cutoff messages go.
        assert_eq!(store.apply_retention(500).unwrap(), 2);
        assert_eq!(store.indexed_message_count().unwrap(), 1);
        assert!(store.search_messages("ancient", 10).unwrap().is_empty());
        assert!(store.messages_for_chat("chat-2").unwrap().is_empty());
        assert_eq!(
            store.messages_for_chat("chat-1").unwrap(),
            vec![recent.clone()]
        );
        assert_eq!(store.clear_chat_history("chat-1").unwrap(), 1);
        assert_eq!(store.indexed_message_count().unwrap(), 0);
        assert!(store.search_messages("recent", 10).unwrap().is_empty());
        assert!(store.messages_for_chat("chat-1").unwrap().is_empty());
        assert!(store.clear_chat_history(" ").is_err());
        assert_eq!(store.clear_chat_history("never-seen").unwrap(), 0);
    }

    #[test]
    fn a_group_admin_may_delete_for_everyone_and_nobody_else_can() {
        let admin = Identity::from_secret([23; 32]);
        let member = Identity::from_secret([24; 32]);
        let outsider = Identity::from_secret([25; 32]);
        let mut store = store();
        let state = GroupState::create(
            &admin,
            "group-delete".into(),
            "Standup".into(),
            [member.peer_id()],
        )
        .unwrap();
        store.save_group_state(&state).unwrap();
        let first = message(&state.id, &member.peer_id(), "spam", 10);
        store.save_message(&first).unwrap();
        // A plain member may delete only their own message.
        assert!(store
            .admin_tombstone_in_group(first.id, &member.peer_id(), &state.id)
            .unwrap());
        let second = message(&state.id, &member.peer_id(), "still here", 20);
        store.save_message(&second).unwrap();
        assert_eq!(
            store
                .admin_tombstone_in_group(second.id, &outsider.peer_id(), &state.id)
                .unwrap_err(),
            "NotSender"
        );
        assert!(
            store
                .admin_tombstone_in_group(second.id, &admin.peer_id(), &state.id)
                .unwrap(),
            "the admin may delete anyone's message in their group"
        );
        assert!(
            !store
                .admin_tombstone_in_group(second.id, &admin.peer_id(), &state.id)
                .unwrap(),
            "a tombstone is applied once"
        );
        assert!(store.search_messages("still here", 10).unwrap().is_empty());
        // Messages outside the group are out of reach.
        let elsewhere = message("chat-9", &member.peer_id(), "elsewhere", 30);
        store.save_message(&elsewhere).unwrap();
        assert!(!store
            .admin_tombstone_in_group(elsewhere.id, &admin.peer_id(), &state.id)
            .unwrap());
    }

    #[test]
    fn outbox_rows_feed_the_replay_planner() {
        let mut store = store();
        let envelope = |tail: u8, at: u64| Envelope {
            v: crate::PROTOCOL_VERSION,
            id: Uuid::from_bytes([tail; 16]),
            kind: "message".into(),
            from: "local".into(),
            to: "peer-a".into(),
            hlc: Hlc {
                physical_ms: at,
                logical: 0,
            },
            payload: serde_json::json!({ "body": format!("body {tail}") }),
        };
        store.enqueue("peer-a", &envelope(3, 30)).unwrap();
        store.enqueue("peer-a", &envelope(1, 10)).unwrap();
        store.enqueue("peer-b", &envelope(2, 20)).unwrap();
        let rows = store.outbox_rows().unwrap();
        assert_eq!(rows.len(), 3);
        let plan = crate::outbox::replay_plan(&rows);
        assert_eq!(
            plan[0]
                .entries
                .iter()
                .map(|e| e.attempt)
                .collect::<Vec<_>>(),
            vec![0, 0]
        );
        assert_eq!(store.bump_outbox_attempt(rows[0].id).unwrap(), 1);
        assert_eq!(store.outbox_rows().unwrap()[0].attempt, 1);
        assert!(store.acknowledge_outbox(rows[0].id).unwrap());
        assert_eq!(store.outbox_rows().unwrap().len(), 2);
        assert_eq!(store.bump_outbox_attempt(Uuid::now_v7()).unwrap(), 0);
    }

    #[test]
    fn read_receipts_drive_the_unread_count_of_a_chat() {
        let mut store = store();
        let reader = "peer-me";
        let mine = message("chat-read", reader, "i spoke", 10);
        let first = message("chat-read", "peer-a", "hello", 11);
        let second = message("chat-read", "peer-b", "hello again", 12);
        let gone = message("chat-read", "peer-a", "unsent", 13);
        for item in [&mine, &first, &second, &gone] {
            store.save_message(item).unwrap();
        }
        store.tombstone_message(gone.id, "peer-a").unwrap();
        // My own words and tombstones never count as unread.
        assert_eq!(
            store.unread_messages_for_chat("chat-read", reader).unwrap(),
            2
        );
        assert_eq!(
            store
                .first_unread_message_for_chat("chat-read", reader)
                .unwrap(),
            Some(first.id)
        );
        assert_eq!(store.mark_chat_read("chat-read", reader, 20).unwrap(), 2);
        assert_eq!(
            store
                .first_unread_message_for_chat("chat-read", reader)
                .unwrap(),
            None
        );
        assert_eq!(
            store.unread_messages_for_chat("chat-read", reader).unwrap(),
            0
        );
        assert!(
            store.receipts_for_message(mine.id).unwrap().is_empty(),
            "marking a chat read does not invent receipts for my own messages"
        );
        assert_eq!(
            store
                .receipts_for_message(first.id)
                .unwrap()
                .iter()
                .filter(|receipt| receipt.kind == ReceiptKind::Read)
                .count(),
            1
        );
        // Marking again is a no-op, not a second row.
        assert_eq!(store.mark_chat_read("chat-read", reader, 21).unwrap(), 0);
        // "Mark unread" takes the receipts back and the count returns.
        assert_eq!(store.mark_chat_unread("chat-read", reader).unwrap(), 2);
        assert_eq!(
            store.unread_messages_for_chat("chat-read", reader).unwrap(),
            2
        );
        assert!(store
            .receipts_for_message(first.id)
            .unwrap()
            .iter()
            .all(|receipt| receipt.kind != ReceiptKind::Read));
        store.mark_chat_read("not-a-chat", reader, 22).unwrap();
        assert!(store.mark_chat_read("", reader, 22).is_err());
        assert!(store.mark_chat_unread("chat-read", "").is_err());
    }
}
