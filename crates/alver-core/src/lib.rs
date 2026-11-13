//! Platform-independent protocol and domain primitives for AlverChat.

pub mod crypto;
pub mod discovery;
pub mod groups;
pub mod identity;
pub mod protocol;
pub mod storage;

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};
use uuid::Uuid;

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
    outbox: Vec<Envelope<serde_json::Value>>,
    applied: HashSet<Uuid>,
}
impl MessageLedger {
    pub fn apply(&mut self, message: Message) -> bool {
        if !self.applied.insert(message.id) {
            return false;
        }
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
}
