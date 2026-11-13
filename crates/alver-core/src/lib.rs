//! Platform-independent protocol and domain primitives for AlverChat.

pub mod crypto;
pub mod discovery;
pub mod files;
pub mod groups;
pub mod identity;
pub mod outbox;
pub mod payloads;
pub mod protocol;
pub mod rules;
pub mod security;
pub mod storage;
pub mod store_ext;
pub mod transport;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

pub use files::{TransferState, TransferPolicy};
pub use payloads::Dispatch as IncomingDispatch;
pub use rules::{EditWindow, ReceiptKind, RuleViolation};

pub const APP_NAME: &str = "AlverChat";
pub const PROTOCOL_VERSION: u16 = 1;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Envelope<T> {
    pub v: u16,
    pub id: Uuid,
    #[serde(rename = "type")]
    pub kind: String,
    pub from: String,
    pub to: String,
    pub hlc: Hlc,
    pub payload: T,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
pub struct Hlc {
    pub physical_ms: u64,
    pub logical: u32,
}

impl Hlc {
    pub fn tick(self, now_ms: u64) -> Self {
        if now_ms > self.physical_ms {
            Self {
                physical_ms: now_ms,
                logical: 0,
            }
        } else {
            Self {
                physical_ms: self.physical_ms,
                logical: self.logical.saturating_add(1),
            }
        }
    }
    pub fn merge(self, remote: Self, now_ms: u64) -> Self {
        let physical_ms = now_ms.max(self.physical_ms).max(remote.physical_ms);
        let logical = match (
            physical_ms == self.physical_ms,
            physical_ms == remote.physical_ms,
        ) {
            (true, true) => self.logical.max(remote.logical).saturating_add(1),
            (true, false) => self.logical.saturating_add(1),
            (false, true) => remote.logical.saturating_add(1),
            (false, false) => 0,
        };
        Self {
            physical_ms,
            logical,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Message {
    pub id: Uuid,
    pub chat_id: String,
    pub sender: String,
    pub body: String,
    pub clock: Hlc,
    pub tombstone: bool,
}

#[derive(Default)]
pub struct MessageLedger {
    entries: BTreeMap<Uuid, Message>,
    revisions: BTreeMap<Uuid, Vec<(u64, String)>>,
    outbox: Vec<Envelope<serde_json::Value>>,
    applied: HashSet<Uuid>,
    edit_window: EditWindow,
}
impl MessageLedger {
    pub fn apply(&mut self, message: Message) -> bool {
        if !self.applied.insert(message.id) {
            return false;
        }
        self.revisions
            .entry(message.id)
            .or_default()
            .push((0, message.body.clone()));
        self.entries.insert(message.id, message);
        true
    }
    pub fn tombstone(&mut self, id: Uuid, sender: &str) -> bool {
        if let Some(m) = self.entries.get_mut(&id) {
            if m.sender == sender {
                m.tombstone = true;
                m.body.clear();
                return true;
            }
        }
        false
    }
    /// Edit a message inside the configured window; revisions are append-only.
    ///
    /// `Ok(None)` means the message is unknown or the edit was already applied (its clock
    /// is not newer than the stored one), so replaying an edit is harmless. Every other
    /// refusal is a typed [`RuleViolation`] from [`crate::rules`].
    pub fn edit(
        &mut self,
        id: Uuid,
        actor: &str,
        body: &str,
        clock: Hlc,
        now_ms: u64,
    ) -> Result<Option<u64>, RuleViolation> {
        let Some(message) = self.entries.get(&id) else {
            return Ok(None);
        };
        self.edit_window.allows_edit(
            &message.sender,
            actor,
            message.tombstone,
            message.clock.physical_ms,
            now_ms,
        )?;
        if rules::compare_hlc(&clock, &message.clock) != std::cmp::Ordering::Greater {
            return Ok(None);
        }
        let highest = self
            .revisions
            .get(&id)
            .and_then(|history| history.iter().map(|(revision, _)| *revision).max())
            .unwrap_or(0);
        let revision = rules::next_revision(highest);
        let history = self.revisions.entry(id).or_default();
        history.push((revision, body.to_owned()));
        let message = self.entries.get_mut(&id).expect("entry stays present");
        message.body = body.to_owned();
        message.clock = clock;
        Ok(Some(revision))
    }
    /// Append-only revision history of one message, oldest first.
    #[must_use]
    pub fn revisions(&self, id: &Uuid) -> &[(u64, String)] {
        self.revisions.get(id).map_or(&[], Vec::as_slice)
    }
    /// Configure the edit window enforced by [`Self::edit`].
    pub fn set_edit_window(&mut self, window: EditWindow) {
        self.edit_window = window;
    }
    #[must_use]
    pub fn edit_window(&self) -> EditWindow {
        self.edit_window
    }
    pub fn get(&self, id: &Uuid) -> Option<&Message> {
        self.entries.get(id)
    }
    pub fn queue(&mut self, envelope: Envelope<serde_json::Value>) {
        self.outbox.push(envelope);
    }
    pub fn drain_outbox(&mut self) -> Vec<Envelope<serde_json::Value>> {
        std::mem::take(&mut self.outbox)
    }
}

pub fn peer_id(public_key: &[u8]) -> String {
    blake3::hash(public_key).to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clock_orders_same_millisecond() {
        let a = Hlc {
            physical_ms: 7,
            logical: 1,
        };
        assert!(a.tick(7) > a);
    }
    #[test]
    fn clock_merge_observes_remote() {
        let a = Hlc {
            physical_ms: 4,
            logical: 1,
        };
        let b = Hlc {
            physical_ms: 9,
            logical: 2,
        };
        assert!(a.merge(b, 3) > b);
    }
    #[test]
    fn message_apply_is_idempotent_and_tombstone_wins() {
        let id = Uuid::now_v7();
        let m = Message {
            id,
            chat_id: "c".into(),
            sender: "p".into(),
            body: "hello".into(),
            clock: Hlc::default(),
            tombstone: false,
        };
        let mut l = MessageLedger::default();
        assert!(l.apply(m.clone()));
        assert!(!l.apply(m));
        assert!(l.tombstone(id, "p"));
        assert!(l.get(&id).unwrap().tombstone);
        assert!(!l.tombstone(id, "other"));
    }

    #[test]
    fn ledger_edits_follow_the_pure_rules() {
        let id = Uuid::now_v7();
        let mut ledger = MessageLedger::default();
        assert!(ledger.apply(Message {
            id,
            chat_id: "c".into(),
            sender: "p".into(),
            body: "first".into(),
            clock: Hlc {
                physical_ms: 1_000,
                logical: 0,
            },
            tombstone: false,
        }));
        assert_eq!(ledger.revisions(&id), &[(0, "first".to_owned())]);
        assert_eq!(
            ledger
                .edit(id, "other", "hijacked", Hlc::default(), 1_001)
                .unwrap_err(),
            RuleViolation::NotSender
        );
        // Replaying the same or an older clock changes nothing.
        assert_eq!(
            ledger
                .edit(
                    id,
                    "p",
                    "first",
                    Hlc {
                        physical_ms: 1_000,
                        logical: 0
                    },
                    1_001
                )
                .unwrap(),
            None
        );
        assert_eq!(
            ledger
                .edit(
                    id,
                    "p",
                    "second",
                    Hlc {
                        physical_ms: 1_000,
                        logical: 1
                    },
                    1_001
                )
                .unwrap(),
            Some(1)
        );
        assert_eq!(ledger.get(&id).unwrap().body, "second");
        assert_eq!(
            ledger.revisions(&id),
            &[(0, "first".to_owned()), (1, "second".to_owned())]
        );
        // The window is configurable and an expired one refuses.
        ledger.set_edit_window(EditWindow::new(50));
        assert_eq!(
            ledger
                .edit(
                    id,
                    "p",
                    "too late",
                    Hlc {
                        physical_ms: 9_000,
                        logical: 0
                    },
                    9_000
                ),
            Err(RuleViolation::EditWindowExpired {
                sent_ms: 1_000,
                now_ms: 9_000,
                window_ms: 50
            })
        );
        assert!(ledger.tombstone(id, "p"));
        assert_eq!(
            ledger.edit(id, "p", "ghost edit", Hlc::default(), 1_002),
            Err(RuleViolation::Tombstoned)
        );
        assert_eq!(
            ledger
                .edit(Uuid::now_v7(), "p", "nowhere", Hlc::default(), 1)
                .unwrap(),
            None
        );
        assert_eq!(ledger.edit_window(), EditWindow::new(50));
    }
}
