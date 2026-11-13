//! Deterministic replay ordering for the persistent outbox.
//!
//! A peer that was offline keeps its queued envelopes. When it reconnects, every queued
//! message, edit, delete and reaction must be replayed in the order the sender produced
//! it, delivery must be attempted at least once, and the receiver's apply must stay
//! idempotent. All of that is pure bookkeeping over rows the storage layer hands us, so
//! it can be reasoned about (and tested) without sockets or a database.

use std::collections::{BTreeMap, HashSet, VecDeque};

use serde_json::Value;
use uuid::Uuid;

use crate::rules::{self, compare_messages};
use crate::{Envelope, Hlc};

/// One persisted outbox row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutboxEntry {
    /// The outbox row id; equal to the envelope id for point-to-point traffic and to a
    /// per-recipient id for group fan-out.
    pub id: Uuid,
    /// The recipient this row belongs to.
    pub peer_id: String,
    /// The envelope bytes as stored.
    pub envelope: Envelope<Value>,
    /// Delivery attempts already spent on this row.
    pub attempt: u32,
}

impl OutboxEntry {
    #[must_use]
    pub fn new(peer_id: &str, envelope: &Envelope<Value>) -> Self {
        Self {
            id: envelope.id,
            peer_id: peer_id.to_owned(),
            envelope: envelope.clone(),
            attempt: 0,
        }
    }

    /// The replay position of this row: sender clock, then the stable id.
    #[must_use]
    pub fn sort_key(&self) -> (Hlc, Uuid) {
        (self.envelope.hlc, self.envelope.id)
    }

    /// The id the receiver deduplicates on: never the row id, which is per-recipient.
    #[must_use]
    pub fn apply_key(&self) -> Uuid {
        self.envelope
            .payload
            .get("message_id")
            .and_then(Value::as_str)
            .and_then(|raw| Uuid::parse_str(raw).ok())
            .unwrap_or(self.envelope.id)
    }

    /// The envelope type name, for subsystem routing.
    #[must_use]
    pub fn kind(&self) -> &str {
        &self.envelope.kind
    }
}

/// Deterministic total order for the whole queue: HLC first, then id.
#[must_use]
pub fn replay_order(entries: &[OutboxEntry]) -> Vec<&OutboxEntry> {
    let mut ordered: Vec<&OutboxEntry> = entries.iter().collect();
    ordered.sort_by(|left, right| {
        compare_messages(
            &left.envelope.hlc,
            &left.envelope.id,
            &right.envelope.hlc,
            &right.envelope.id,
        )
    });
    ordered
}

/// The per-recipient replay queue produced from the persisted rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PeerReplay {
    pub peer_id: String,
    pub entries: Vec<OutboxEntry>,
}

impl PeerReplay {
    /// Ordered, stable plan for one peer.
    #[must_use]
    pub fn new(peer_id: &str, mut entries: Vec<OutboxEntry>) -> Self {
        entries.sort_by(|left, right| {
            compare_messages(
                &left.envelope.hlc,
                &left.envelope.id,
                &right.envelope.hlc,
                &right.envelope.id,
            )
        });
        Self {
            peer_id: peer_id.to_owned(),
            entries,
        }
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// Groups the persisted rows by recipient and orders each queue deterministically.
///
/// Peers are visited in sorted peer-id order so two processes holding the same rows
/// produce the same plan byte for byte.
#[must_use]
pub fn replay_plan(entries: &[OutboxEntry]) -> Vec<PeerReplay> {
    let mut by_peer: BTreeMap<&str, Vec<OutboxEntry>> = BTreeMap::new();
    for entry in entries {
        by_peer
            .entry(entry.peer_id.as_str())
            .or_default()
            .push(entry.clone());
    }
    by_peer
        .into_iter()
        .map(|(peer_id, rows)| PeerReplay::new(peer_id, rows))
        .collect()
}

/// Exponential retry delay for a row, capped so a long outage cannot overflow.
#[must_use]
pub fn retry_backoff_ms(attempt: u32, base_ms: u64, max_ms: u64) -> u64 {
    let factor = 1_u64 << attempt.min(16);
    base_ms.saturating_mul(factor).min(max_ms)
}

/// At-least-once sender, idempotent receiver session over one replay plan.
///
/// A row stays in the queue until it is acknowledged, so a dropped connection replays
/// it; the receiver-side guard collapses repeated envelopes onto one apply.
#[derive(Clone, Debug)]
pub struct ReplaySession {
    queues: BTreeMap<String, VecDeque<OutboxEntry>>,
    inflight: HashSet<Uuid>,
    acknowledged: HashSet<Uuid>,
    receiver_applied: rules::ApplyTracker,
    attempts: BTreeMap<Uuid, u32>,
}

impl ReplaySession {
    #[must_use]
    pub fn new(entries: &[OutboxEntry]) -> Self {
        let queues = replay_plan(entries)
            .into_iter()
            .map(|plan| (plan.peer_id, plan.entries.into()))
            .collect();
        Self {
            queues,
            inflight: HashSet::new(),
            acknowledged: HashSet::new(),
            receiver_applied: rules::ApplyTracker::new(),
            attempts: BTreeMap::new(),
        }
    }

    /// The next row owed to a peer, or `None` when it is fully caught up.
    ///
    /// The row stays queued until [`Self::acknowledge`], which is what makes delivery
    /// at-least-once rather than at-most-once.
    pub fn next_for(&mut self, peer_id: &str) -> Option<OutboxEntry> {
        let queue = self.queues.get_mut(peer_id)?;
        queue.front().cloned()
    }

    /// Records that a row was handed to the transport.
    pub fn mark_inflight(&mut self, id: Uuid) {
        self.inflight.insert(id);
        *self.attempts.entry(id).or_insert(0) += 1;
    }

    /// Removes a delivered row and stops it ever being replayed.
    pub fn acknowledge(&mut self, id: Uuid) -> bool {
        self.inflight.remove(&id);
        for queue in self.queues.values_mut() {
            queue.retain(|entry| entry.id != id);
        }
        self.acknowledged.insert(id)
    }

    /// Puts a failed row back at the head of its peer queue, keeping order.
    pub fn requeue(&mut self, entry: OutboxEntry) {
        self.inflight.remove(&entry.id);
        if self.acknowledged.contains(&entry.id) {
            return;
        }
        let queue = self.queues.entry(entry.peer_id.clone()).or_default();
        if queue.iter().any(|queued| queued.id == entry.id) {
            return;
        }
        queue.push_front(entry);
    }

    /// Peers this session knows about, in plan order.
    #[must_use]
    pub fn peers(&self) -> Vec<String> {
        self.queues.keys().cloned().collect()
    }

    #[must_use]
    pub fn pending_for(&self, peer_id: &str) -> usize {
        self.queues.get(peer_id).map_or(0, VecDeque::len)
    }

    #[must_use]
    pub fn pending_total(&self) -> usize {
        self.queues.values().map(VecDeque::len).sum()
    }

    #[must_use]
    pub fn attempt_for(&self, id: &Uuid) -> u32 {
        self.attempts.get(id).copied().unwrap_or(0)
    }

    /// Receiver side: true exactly once per logical message id.
    pub fn apply_once(&mut self, id: Uuid) -> bool {
        self.receiver_applied.apply(id)
    }

    #[must_use]
    pub fn has_applied(&self, id: &Uuid) -> bool {
        self.receiver_applied.has_applied(id)
    }

    /// Admits a peer whose rows were queued while it was offline.
    pub fn register_offline_peer(&mut self, peer_id: &str, entries: Vec<OutboxEntry>) {
        let mut queue: VecDeque<OutboxEntry> = PeerReplay::new(peer_id, entries).entries.into();
        queue.retain(|entry| !self.acknowledged.contains(&entry.id));
        self.queues.insert(peer_id.to_owned(), queue);
    }
}

/// Applies the receiver-side idempotence rule to a received envelope.
#[must_use]
pub fn apply_envelope_is_new(
    applied: &mut rules::ApplyTracker,
    envelope: &Envelope<Value>,
) -> bool {
    let id = envelope
        .payload
        .get("message_id")
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .unwrap_or(envelope.id);
    applied.apply(id)
}

/// Ordering helper for callers that only hold envelopes.
#[must_use]
pub fn envelope_order(a: &Envelope<Value>, b: &Envelope<Value>) -> std::cmp::Ordering {
    compare_messages(&a.hlc, &a.id, &b.hlc, &b.id)
}

/// True when the plan keeps a causal pair in order: `earlier` before `later`.
#[must_use]
pub fn ordered_before(earlier: &OutboxEntry, later: &OutboxEntry) -> bool {
    matches!(
        rules::compare_messages(
            &earlier.envelope.hlc,
            &earlier.envelope.id,
            &later.envelope.hlc,
            &later.envelope.id,
        ),
        std::cmp::Ordering::Less
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payloads::kind;
    use crate::PROTOCOL_VERSION;

    fn clock(physical_ms: u64, logical: u32) -> Hlc {
        Hlc {
            physical_ms,
            logical,
        }
    }

    fn envelope(kind_name: &str, id: Uuid, at: Hlc, payload: Value) -> Envelope<Value> {
        Envelope {
            v: PROTOCOL_VERSION,
            id,
            kind: kind_name.into(),
            from: "local-peer".into(),
            to: "peer-a".into(),
            hlc: at,
            payload,
        }
    }

    fn entry(peer_id: &str, kind_name: &str, tail: u8, at: Hlc, payload: Value) -> OutboxEntry {
        OutboxEntry::new(
            peer_id,
            &envelope(kind_name, Uuid::from_bytes([tail; 16]), at, payload),
        )
    }

    #[test]
    fn replay_order_is_hlc_then_id_and_ignores_insertion_order() {
        let entries = vec![
            entry(
                "peer-a",
                kind::REACTION,
                4,
                clock(1, 1),
                serde_json::json!({}),
            ),
            entry(
                "peer-a",
                kind::MESSAGE,
                1,
                clock(0, 3),
                serde_json::json!({}),
            ),
            entry(
                "peer-a",
                kind::MESSAGE_EDIT,
                3,
                clock(1, 0),
                serde_json::json!({}),
            ),
            entry(
                "peer-a",
                kind::MESSAGE_DELETE,
                2,
                clock(0, 4),
                serde_json::json!({}),
            ),
        ];
        let ordered = replay_order(&entries);
        let ids: Vec<u8> = ordered
            .iter()
            .map(|entry| entry.envelope.id.as_bytes()[0])
            .collect();
        assert_eq!(ids, vec![1, 2, 3, 4]);
        // The same rows in a different arrival order produce the same plan.
        let mut rotated = entries.clone();
        rotated.rotate_left(2);
        assert_eq!(
            replay_order(&rotated)
                .iter()
                .map(|entry| entry.id)
                .collect::<Vec<_>>(),
            ids.iter()
                .map(|tail| Uuid::from_bytes([*tail; 16]))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn plans_are_grouped_per_peer_in_peer_order() {
        let entries = vec![
            entry(
                "peer-b",
                kind::MESSAGE,
                9,
                clock(5, 0),
                serde_json::json!({}),
            ),
            entry(
                "peer-a",
                kind::MESSAGE,
                8,
                clock(7, 0),
                serde_json::json!({}),
            ),
            entry(
                "peer-a",
                kind::REACTION,
                7,
                clock(2, 0),
                serde_json::json!({}),
            ),
        ];
        let plan = replay_plan(&entries);
        assert_eq!(plan.len(), 2);
        assert_eq!(plan[0].peer_id, "peer-a");
        assert_eq!(
            plan[0]
                .entries
                .iter()
                .map(|e| e.envelope.id)
                .collect::<Vec<_>>(),
            vec![entries[2].envelope.id, entries[1].envelope.id,]
        );
        assert_eq!(plan[1].peer_id, "peer-b");
        assert_eq!(plan[1].len(), 1);
        assert!(ordered_before(&plan[0].entries[0], &plan[0].entries[1]));
    }

    #[test]
    fn delivery_is_at_least_once_until_acknowledged() {
        let message = entry(
            "peer-a",
            kind::MESSAGE,
            1,
            clock(1, 0),
            serde_json::json!({}),
        );
        let mut session = ReplaySession::new(std::slice::from_ref(&message));
        assert_eq!(session.pending_for("peer-a"), 1);
        let first = session.next_for("peer-a").expect("queued");
        session.mark_inflight(first.id);
        // A dropped connection leaves the row owed, so it is offered again.
        assert_eq!(session.next_for("peer-a"), Some(first.clone()));
        assert_eq!(session.attempt_for(&first.id), 1);
        session.mark_inflight(first.id);
        assert_eq!(session.attempt_for(&first.id), 2);
        assert!(session.acknowledge(first.id));
        assert_eq!(session.next_for("peer-a"), None);
        assert_eq!(session.pending_total(), 0);
        assert!(!session.acknowledge(first.id), "second ack is a no-op");
    }

    #[test]
    fn receiver_apply_is_idempotent_across_replays() {
        let message = entry(
            "peer-a",
            kind::MESSAGE,
            2,
            clock(9, 0),
            serde_json::json!({ "message_id": "01900000-0000-7000-8000-0000000000aa" }),
        );
        let mut tracker = rules::ApplyTracker::new();
        assert!(apply_envelope_is_new(&mut tracker, &message.envelope));
        assert!(!apply_envelope_is_new(&mut tracker, &message.envelope));
        assert_eq!(tracker.len(), 1);
        // The row id never leaks into the apply key.
        assert_eq!(message.apply_key().as_bytes()[15], 0xaa);
    }

    #[test]
    fn offline_peer_replays_message_edit_delete_and_reaction_in_order() {
        let message = entry(
            "peer-offline",
            kind::MESSAGE,
            1,
            clock(10, 0),
            serde_json::json!({ "chat_id": "peer-offline", "body": "hello" }),
        );
        let edit = entry(
            "peer-offline",
            kind::MESSAGE_EDIT,
            2,
            clock(11, 0),
            serde_json::json!({ "message_id": message.envelope.id, "body": "hello there" }),
        );
        let reaction = entry(
            "peer-offline",
            kind::REACTION,
            3,
            clock(12, 0),
            serde_json::json!({ "message_id": message.envelope.id, "emoji": "ok" }),
        );
        let delete = entry(
            "peer-offline",
            kind::MESSAGE_DELETE,
            4,
            clock(13, 0),
            serde_json::json!({ "message_id": message.envelope.id }),
        );
        let mut session = ReplaySession::new(&[]);
        assert_eq!(session.next_for("peer-offline"), None);
        // Everything produced while the peer was away.
        session.register_offline_peer(
            "peer-offline",
            vec![
                delete.clone(),
                message.clone(),
                reaction.clone(),
                edit.clone(),
            ],
        );
        let mut sequence = Vec::new();
        while let Some(next) = session.next_for("peer-offline") {
            session.mark_inflight(next.id);
            session.acknowledge(next.id);
            sequence.push(next.kind().to_owned());
        }
        assert_eq!(
            sequence,
            vec![
                kind::MESSAGE,
                kind::MESSAGE_EDIT,
                kind::REACTION,
                kind::MESSAGE_DELETE
            ]
        );
        assert_eq!(session.pending_total(), 0);
        // Re-registering the same rows after a reconnect replays nothing already acked.
        session.register_offline_peer("peer-offline", vec![message, edit]);
        assert_eq!(session.next_for("peer-offline"), None);
    }

    #[test]
    fn failure_requeues_at_the_head_and_keeps_causal_order() {
        let message = entry(
            "peer-a",
            kind::MESSAGE,
            1,
            clock(1, 0),
            serde_json::json!({}),
        );
        let edit = entry(
            "peer-a",
            kind::MESSAGE_EDIT,
            2,
            clock(2, 0),
            serde_json::json!({}),
        );
        let mut session = ReplaySession::new(&[message.clone(), edit.clone()]);
        let first = session.next_for("peer-a").unwrap();
        session.mark_inflight(first.id);
        session.acknowledge(first.id);
        let second = session.next_for("peer-a").unwrap();
        session.mark_inflight(second.id);
        session.requeue(second.clone());
        assert_eq!(session.next_for("peer-a").unwrap(), second);
        session.acknowledge(second.id);
        assert_eq!(session.pending_for("peer-a"), 0);
        assert_eq!(session.peers(), vec!["peer-a".to_owned()]);
        // Requeueing an acknowledged row must never resurrect it.
        session.requeue(first);
        assert_eq!(session.next_for("peer-a"), None);
    }

    #[test]
    fn retry_backoff_grows_and_saturates() {
        assert_eq!(retry_backoff_ms(0, 500, 30_000), 500);
        assert_eq!(retry_backoff_ms(1, 500, 30_000), 1_000);
        assert_eq!(retry_backoff_ms(3, 500, 30_000), 4_000);
        assert_eq!(retry_backoff_ms(40, 500, 30_000), 30_000);
        assert_eq!(retry_backoff_ms(40, u64::MAX, u64::MAX), u64::MAX);
    }

    #[test]
    fn envelope_order_matches_the_plan() {
        let first = envelope(
            kind::MESSAGE,
            Uuid::from_bytes([1; 16]),
            clock(1, 0),
            serde_json::json!({}),
        );
        let second = envelope(
            kind::MESSAGE,
            Uuid::from_bytes([2; 16]),
            clock(1, 1),
            serde_json::json!({}),
        );
        assert_eq!(envelope_order(&first, &second), std::cmp::Ordering::Less);
        assert_eq!(envelope_order(&second, &first), std::cmp::Ordering::Greater);
        assert_eq!(envelope_order(&first, &first), std::cmp::Ordering::Equal);
    }
}
