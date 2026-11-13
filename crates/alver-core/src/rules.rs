//! Pure authorization and convergence rules.
//!
//! These are the spec's ordering, authority and permanence invariants expressed over
//! plain values, with no database, clock source, or OS access, so they can be
//! property-tested directly and reused by the storage layer, the replay planner and the
//! desktop commands. Every function here is deterministic and side-effect free.

use std::cmp::Ordering;
use std::collections::HashSet;

use uuid::Uuid;

use crate::Hlc;

/// Default edit window: how long after sending a message its author may still edit it.
pub const DEFAULT_EDIT_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

/// A rule rejected the request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RuleViolation {
    /// Only the original sender may edit a message.
    NotSender,
    /// The message is a permanent tombstone.
    Tombstoned,
    /// The edit arrived after the configured edit window.
    EditWindowExpired {
        sent_ms: u64,
        now_ms: u64,
        window_ms: u64,
    },
    /// Group epochs must advance by exactly one.
    EpochGap {
        expected: u64,
        found: u64,
    },
    /// The state was not signed by the group's recorded admin.
    SignerNotAdmin,
    /// A roster tried to change the group's admin.
    AdminChanged,
    /// A roster must always contain its admin.
    AdminMissingFromRoster,
    /// The group admin may not remove or leave their own group.
    AdminCannotLeave,
    /// A plain member may not remove another peer.
    MemberCannotRemoveAnother,
    /// Only an admin may add, remove or mute members.
    NotAdmin,
    /// The built-in Everyone roster is presence-derived and is never signed.
    EveryoneRosterImmutable,
    /// The target is not a member of the group.
    NotMember,
    /// The target is already a member of the group.
    AlreadyMember,
    /// A delivered receipt may not downgrade an existing read receipt.
    StaleReceipt,
    /// One acknowledgement per peer per announcement.
    AlreadyAcknowledged,
}

/// Receipt levels, ordered so that `read` always dominates `delivered`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum ReceiptKind {
    Delivered,
    Read,
}

impl ReceiptKind {
    /// The wire spelling of this receipt level.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Delivered => "delivered",
            Self::Read => "read",
        }
    }

    /// Parse a wire receipt level; anything else is not a receipt.
    #[must_use]
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "delivered" => Some(Self::Delivered),
            "read" => Some(Self::Read),
            _ => None,
        }
    }
}

/// What a roster mutation asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RosterAction {
    Add,
    Remove,
    Mute,
    Unmute,
    Leave,
}

/// Which side of a duplicate connection keeps its session.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionRole {
    /// This side wins and keeps its connection.
    Keep,
    /// The remote side wins; this side must drop.
    Drop,
}

/// Inputs for one roster decision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterRequest<'a> {
    /// Peer requesting the change.
    pub actor: &'a str,
    /// Peer the change is about.
    pub target: &'a str,
    pub change: RosterAction,
    /// Recorded admin of the group.
    pub admin: &'a str,
    /// True for the built-in `everyone` group.
    pub is_everyone: bool,
    /// Current members, in canonical order.
    pub members: &'a [String],
}

/// A signed roster reduced to the fields the epoch rules need.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RosterFacts<'a> {
    pub epoch: u64,
    pub admin: &'a str,
    pub members: &'a [String],
}

/// Edit window configuration, read from settings by the storage layer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EditWindow {
    pub ms: u64,
}

impl Default for EditWindow {
    fn default() -> Self {
        Self::default_window()
    }
}

impl EditWindow {
    /// `0` disables editing; `u64::MAX` keeps the window open forever.
    #[must_use]
    pub const fn new(ms: u64) -> Self {
        Self { ms }
    }

    /// The configured default window.
    #[must_use]
    pub const fn default_window() -> Self {
        Self {
            ms: DEFAULT_EDIT_WINDOW_MS,
        }
    }

    /// True when an edit at `now_ms` is inside the window for a message sent at
    /// `sent_ms`. The boundary is inclusive, and clock skew that puts `now_ms` behind
    /// `sent_ms` never expires a message.
    #[must_use]
    pub fn is_open(&self, sent_ms: u64, now_ms: u64) -> bool {
        if self.ms == u64::MAX {
            return true;
        }
        now_ms.saturating_sub(sent_ms) <= self.ms
    }

    /// Full decision for an edit request: authority, tombstone permanence, window.
    pub fn allows_edit(
        &self,
        message_sender: &str,
        actor: &str,
        tombstoned: bool,
        sent_ms: u64,
        now_ms: u64,
    ) -> Result<(), RuleViolation> {
        edit_authority(message_sender, actor)?;
        if tombstoned {
            return Err(RuleViolation::Tombstoned);
        }
        if !self.is_open(sent_ms, now_ms) {
            return Err(RuleViolation::EditWindowExpired {
                sent_ms,
                now_ms,
                window_ms: self.ms,
            });
        }
        Ok(())
    }
}

/// Only the original sender may edit.
pub fn edit_authority(message_sender: &str, actor: &str) -> Result<(), RuleViolation> {
    if message_sender == actor {
        Ok(())
    } else {
        Err(RuleViolation::NotSender)
    }
}

/// Delete-for-everyone belongs to the sender, or to the admin of the group holding it.
///
/// `group_admin` is `Some` only when the message lives in that admin's group.
pub fn tombstone_authority(
    message_sender: &str,
    actor: &str,
    group_admin: Option<&str>,
) -> Result<(), RuleViolation> {
    if message_sender == actor {
        return Ok(());
    }
    if group_admin == Some(actor) {
        return Ok(());
    }
    Err(RuleViolation::NotSender)
}

/// A tombstone is permanent: no late edit, reaction, receipt or pin may act on it.
#[must_use]
pub fn accepts_updates(tombstoned: bool) -> bool {
    !tombstoned
}

/// Late-arrival filters, spelled per event family for the engine's call sites.
#[must_use]
pub fn late_edit_allowed(tombstoned: bool) -> bool {
    accepts_updates(tombstoned)
}

#[must_use]
pub fn late_reaction_allowed(tombstoned: bool) -> bool {
    accepts_updates(tombstoned)
}

#[must_use]
pub fn late_receipt_allowed(tombstoned: bool) -> bool {
    accepts_updates(tombstoned)
}

/// Edits append: the next revision number is one past the highest stored revision.
#[must_use]
pub fn next_revision(highest_stored: u64) -> u64 {
    highest_stored.saturating_add(1)
}

/// Receipts only move forward: `read` may replace `delivered`, never the reverse, and a
/// repeated level is a no-op rather than an error.
pub fn receipt_advances(
    existing: Option<ReceiptKind>,
    incoming: ReceiptKind,
) -> Result<bool, RuleViolation> {
    let Some(current) = existing else {
        return Ok(true);
    };
    match incoming.cmp(&current) {
        Ordering::Greater => Ok(true),
        Ordering::Equal => Ok(false),
        Ordering::Less => Err(RuleViolation::StaleReceipt),
    }
}

/// Total order over clocks: physical time first, then the logical counter.
#[must_use]
pub fn compare_hlc(a: &Hlc, b: &Hlc) -> Ordering {
    a.physical_ms
        .cmp(&b.physical_ms)
        .then_with(|| a.logical.cmp(&b.logical))
}

/// Causal order for messages: HLC first, then the id, so ties are still deterministic.
#[must_use]
pub fn compare_messages(a: &Hlc, a_id: &Uuid, b: &Hlc, b_id: &Uuid) -> Ordering {
    compare_hlc(a, b).then_with(|| a_id.cmp(b_id))
}

/// Sorts in place with the same comparator the ledger and history views use.
pub fn sort_messages<T, F>(items: &mut [T], clock_and_id: F)
where
    F: Fn(&T) -> (Hlc, Uuid),
{
    items.sort_by(|left, right| {
        let (a_clock, a_id) = clock_and_id(left);
        let (b_clock, b_id) = clock_and_id(right);
        compare_messages(&a_clock, &a_id, &b_clock, &b_id)
    });
}

/// Duplicate connections are resolved by the lower peer id, which both sides can
/// compute from the same two identities, so exactly one session survives.
#[must_use]
pub fn duplicate_connection_winner(local_id: &str, remote_id: &str) -> ConnectionRole {
    if local_id <= remote_id {
        ConnectionRole::Keep
    } else {
        ConnectionRole::Drop
    }
}

/// The peer id that wins a tie between two candidates.
#[must_use]
pub fn tie_break_by_peer<'a>(a: &'a str, b: &'a str) -> &'a str {
    if a <= b {
        a
    } else {
        b
    }
}

/// Rosters are canonical: sorted, de-duplicated, no blank ids.
#[must_use]
pub fn canonical_roster(members: &[String]) -> Vec<String> {
    let mut roster: Vec<String> = members
        .iter()
        .filter(|member| !member.trim().is_empty())
        .cloned()
        .collect();
    roster.sort();
    roster.dedup();
    roster
}

/// Group epoch rules for an incoming signed state.
///
/// Rejects a state whose epoch is not exactly `predecessor + 1`, one that changes the
/// admin, one whose signer is no longer the admin, and a roster that dropped its admin.
/// A brand-new group must start at epoch 1.
pub fn check_group_state(
    previous: Option<&RosterFacts<'_>>,
    signer: &str,
    incoming: &RosterFacts<'_>,
) -> Result<(), RuleViolation> {
    let expected = match previous {
        None => 1,
        Some(last) => last.epoch.saturating_add(1),
    };
    if incoming.epoch != expected {
        return Err(RuleViolation::EpochGap {
            expected,
            found: incoming.epoch,
        });
    }
    if let Some(last) = previous {
        if last.admin != incoming.admin {
            return Err(RuleViolation::AdminChanged);
        }
    }
    if signer != incoming.admin {
        return Err(RuleViolation::SignerNotAdmin);
    }
    if incoming
        .members
        .iter()
        .any(|member| member == incoming.admin)
    {
        Ok(())
    } else {
        Err(RuleViolation::AdminMissingFromRoster)
    }
}

/// Roster mutation authority, identical for ordinary and Everyone groups:
/// admins add, remove and mute; members may only leave themselves; nobody may remove or
/// exile the admin.
pub fn check_roster_change(request: &RosterRequest<'_>) -> Result<(), RuleViolation> {
    let actor_is_admin = request.actor == request.admin;
    let target_is_member = request
        .members
        .iter()
        .any(|member| member == request.target);
    let self_targeted = request.actor == request.target;
    let action = if self_targeted && request.change == RosterAction::Remove {
        RosterAction::Leave
    } else {
        request.change
    };
    match action {
        RosterAction::Leave => {
            if actor_is_admin {
                return Err(RuleViolation::AdminCannotLeave);
            }
            if target_is_member {
                Ok(())
            } else {
                Err(RuleViolation::NotMember)
            }
        }
        RosterAction::Add => {
            if !actor_is_admin {
                return Err(RuleViolation::NotAdmin);
            }
            if target_is_member {
                return Err(RuleViolation::AlreadyMember);
            }
            Ok(())
        }
        RosterAction::Remove | RosterAction::Mute | RosterAction::Unmute => {
            if !actor_is_admin {
                return Err(if request.change == RosterAction::Remove {
                    RuleViolation::MemberCannotRemoveAnother
                } else {
                    RuleViolation::NotAdmin
                });
            }
            if request.target == request.admin {
                return Err(RuleViolation::AdminCannotLeave);
            }
            if !target_is_member {
                return Err(RuleViolation::NotMember);
            }
            Ok(())
        }
    }
}

/// Everyone-group rule: the roster is derived from presence, so no signed state change
/// is ever accepted for it. Mirrors `GroupError::CannotChangeEveryone`.
pub fn check_everyone_state_change() -> Result<(), RuleViolation> {
    Err(RuleViolation::EveryoneRosterImmutable)
}

/// Every discovered peer belongs to the Everyone group, always.
#[must_use]
pub fn everyone_roster(known_peers: &[String]) -> Vec<String> {
    canonical_roster(known_peers)
}

/// Membership in the Everyone group is implicit.
#[must_use]
pub fn is_everyone_member(peer_id: &str, known_peers: &[String]) -> bool {
    known_peers.iter().any(|peer| peer == peer_id)
}

/// Announcement authority: any member, or admins only when the setting is on.
pub fn check_announcement_authority(
    actor: &str,
    admin: &str,
    members: &[String],
    admins_only: bool,
) -> Result<(), RuleViolation> {
    if actor == admin {
        return Ok(());
    }
    if admins_only {
        return Err(RuleViolation::NotAdmin);
    }
    if members.iter().any(|member| member == actor) {
        Ok(())
    } else {
        Err(RuleViolation::NotMember)
    }
}

/// One acknowledgement per peer per announcement.
pub fn check_announcement_ack(already_acked: &[String], peer_id: &str) -> Result<(), RuleViolation> {
    if already_acked.iter().any(|acked| acked == peer_id) {
        Err(RuleViolation::AlreadyAcknowledged)
    } else {
        Ok(())
    }
}

/// Idempotent apply tracker: the first sight of an id is new, every later one is not.
#[derive(Clone, Debug, Default)]
pub struct ApplyTracker {
    applied: HashSet<Uuid>,
}

impl ApplyTracker {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// True exactly once per id.
    pub fn apply(&mut self, id: Uuid) -> bool {
        self.applied.insert(id)
    }

    #[must_use]
    pub fn has_applied(&self, id: &Uuid) -> bool {
        self.applied.contains(id)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.applied.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.applied.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADMIN: &str = "admin-peer";
    const PEER_B: &str = "peer-b";
    const PEER_C: &str = "peer-c";

    fn roster(names: &[&str]) -> Vec<String> {
        names.iter().map(|name| (*name).to_owned()).collect()
    }

    fn request<'a>(
        actor: &'a str,
        target: &'a str,
        change: RosterAction,
        admin: &'a str,
        is_everyone: bool,
        members: &'a [String],
    ) -> RosterRequest<'a> {
        RosterRequest {
            actor,
            target,
            change,
            admin,
            is_everyone,
            members,
        }
    }

    #[test]
    fn only_the_sender_may_edit_and_nobody_may_edit_a_tombstone() {
        assert_eq!(edit_authority(PEER_B, PEER_B), Ok(()));
        assert_eq!(edit_authority(PEER_B, PEER_C), Err(RuleViolation::NotSender));
        let window = EditWindow::new(1_000);
        assert_eq!(
            window.allows_edit(PEER_B, PEER_C, false, 10, 20),
            Err(RuleViolation::NotSender)
        );
        assert_eq!(
            window.allows_edit(PEER_B, PEER_B, true, 10, 20),
            Err(RuleViolation::Tombstoned)
        );
        assert_eq!(window.allows_edit(PEER_B, PEER_B, false, 10, 20), Ok(()));
    }

    #[test]
    fn edit_window_boundary_is_inclusive_and_configurable() {
        let window = EditWindow::new(1_000);
        assert!(window.is_open(10, 10));
        assert!(window.is_open(10, 1_010));
        assert!(!window.is_open(10, 1_011));
        // Skewed clocks never expire a message.
        assert!(window.is_open(5_000, 10));
        assert_eq!(
            window.allows_edit("a", "a", false, 0, 1_011),
            Err(RuleViolation::EditWindowExpired {
                sent_ms: 0,
                now_ms: 1_011,
                window_ms: 1_000
            })
        );
        assert!(!EditWindow::new(0).is_open(0, 1));
        assert!(EditWindow::new(0).is_open(0, 0));
        assert!(EditWindow::new(u64::MAX).is_open(0, u64::MAX));
        assert!(EditWindow::default_window().is_open(0, DEFAULT_EDIT_WINDOW_MS));
        assert_eq!(EditWindow::default(), EditWindow::default_window());
    }

    #[test]
    fn tombstones_are_permmanent_for_late_edits_reactions_and_receipts() {
        assert!(!late_edit_allowed(true));
        assert!(!late_reaction_allowed(true));
        assert!(!late_receipt_allowed(true));
        assert!(late_edit_allowed(false));
        assert!(!accepts_updates(true));
    }

    #[test]
    fn admins_may_delete_anything_in_their_group_but_others_may_not() {
        assert_eq!(tombstone_authority(PEER_B, PEER_B, None), Ok(()));
        assert_eq!(
            tombstone_authority(PEER_B, ADMIN, Some(ADMIN)),
            Ok(())
        );
        assert_eq!(
            tombstone_authority(PEER_B, PEER_C, Some(ADMIN)),
            Err(RuleViolation::NotSender)
        );
        assert_eq!(
            tombstone_authority(PEER_B, PEER_C, None),
            Err(RuleViolation::NotSender)
        );
    }

    #[test]
    fn edits_append_and_never_rewrite_history() {
        assert_eq!(next_revision(0), 1);
        assert_eq!(next_revision(7), 8);
        assert_eq!(next_revision(u64::MAX), u64::MAX);
    }

    #[test]
    fn apply_is_idempotent_per_id() {
        let mut tracker = ApplyTracker::new();
        let id = Uuid::now_v7();
        assert!(tracker.is_empty());
        assert!(tracker.apply(id));
        assert!(!tracker.apply(id));
        assert!(tracker.has_applied(&id));
        assert_eq!(tracker.len(), 1);
    }

    #[test]
    fn hlc_order_is_total_and_messages_tie_break_on_id() {
        let older = Hlc {
            physical_ms: 1,
            logical: 9,
        };
        let newer = Hlc {
            physical_ms: 2,
            logical: 0,
        };
        assert_eq!(compare_hlc(&older, &newer), Ordering::Less);
        assert_eq!(compare_hlc(&newer, &older), Ordering::Greater);
        assert_eq!(compare_hlc(&older, &older), Ordering::Equal);
        let same_tick = Hlc {
            physical_ms: 3,
            logical: 1,
        };
        let same_tick_later = Hlc {
            physical_ms: 3,
            logical: 2,
        };
        assert_eq!(compare_hlc(&same_tick, &same_tick_later), Ordering::Less);
        let low = Uuid::parse_str("01900000-0000-7000-8000-000000000001").unwrap();
        let high = Uuid::parse_str("01900000-0000-7000-8000-000000000002").unwrap();
        assert_eq!(
            compare_messages(&same_tick, &high, &same_tick, &low),
            Ordering::Greater
        );
        let mut ids = vec![high, low];
        sort_messages(&mut ids, |id| (same_tick, *id));
        assert_eq!(ids, vec![low, high]);
    }

    #[test]
    fn duplicate_connections_are_broken_by_the_lower_peer_id() {
        let low = "0aaa";
        let high = "ffff";
        assert_eq!(duplicate_connection_winner(low, high), ConnectionRole::Keep);
        assert_eq!(duplicate_connection_winner(high, low), ConnectionRole::Drop);
        // Both sides of the same pair never both keep, and never both drop.
        assert_ne!(
            duplicate_connection_winner(low, high),
            duplicate_connection_winner(high, low)
        );
        assert_eq!(tie_break_by_peer(high, low), low);
        assert_eq!(tie_break_by_peer(low, low), low);
    }

    #[test]
    fn receipts_only_move_forward() {
        assert_eq!(receipt_advances(None, ReceiptKind::Delivered), Ok(true));
        assert_eq!(
            receipt_advances(Some(ReceiptKind::Delivered), ReceiptKind::Delivered),
            Ok(false)
        );
        assert_eq!(
            receipt_advances(Some(ReceiptKind::Delivered), ReceiptKind::Read),
            Ok(true)
        );
        assert_eq!(
            receipt_advances(Some(ReceiptKind::Read), ReceiptKind::Delivered),
            Err(RuleViolation::StaleReceipt)
        );
        assert_eq!(
            receipt_advances(Some(ReceiptKind::Read), ReceiptKind::Read),
            Ok(false)
        );
        assert_eq!(ReceiptKind::parse("read"), Some(ReceiptKind::Read));
        assert_eq!(ReceiptKind::parse("seen"), None);
        assert_eq!(ReceiptKind::Read.as_str(), "read");
        assert_eq!(ReceiptKind::Delivered.as_str(), "delivered");
    }

    #[test]
    fn group_states_must_advance_exactly_one_epoch_and_keep_their_admin() {
        let members = roster(&[ADMIN, PEER_B]);
        let previous = RosterFacts {
            epoch: 4,
            admin: ADMIN,
            members: &members,
        };
        let grown = roster(&[ADMIN, PEER_B, PEER_C]);
        let successor = RosterFacts {
            epoch: 5,
            admin: ADMIN,
            members: &grown,
        };
        assert_eq!(check_group_state(Some(&previous), ADMIN, &successor), Ok(()));
        let skipped = RosterFacts {
            epoch: 7,
            admin: ADMIN,
            members: &members,
        };
        assert_eq!(
            check_group_state(Some(&previous), ADMIN, &skipped),
            Err(RuleViolation::EpochGap {
                expected: 5,
                found: 7
            })
        );
        let stale = RosterFacts {
            epoch: 4,
            admin: ADMIN,
            members: &members,
        };
        assert_eq!(
            check_group_state(Some(&previous), ADMIN, &stale),
            Err(RuleViolation::EpochGap {
                expected: 5,
                found: 4
            })
        );
        let handed_over = RosterFacts {
            epoch: 5,
            admin: PEER_B,
            members: &roster(&[PEER_B]),
        };
        assert_eq!(
            check_group_state(Some(&previous), PEER_B, &handed_over),
            Err(RuleViolation::AdminChanged)
        );
        let exiled_admin = RosterFacts {
            epoch: 5,
            admin: ADMIN,
            members: &roster(&[PEER_B]),
        };
        assert_eq!(
            check_group_state(Some(&previous), ADMIN, &exiled_admin),
            Err(RuleViolation::AdminMissingFromRoster)
        );
        // A state signed by a peer who is no longer an admin is rejected.
        assert_eq!(
            check_group_state(Some(&previous), PEER_B, &successor),
            Err(RuleViolation::SignerNotAdmin)
        );
        let genesis = RosterFacts {
            epoch: 1,
            admin: ADMIN,
            members: &roster(&[ADMIN]),
        };
        assert_eq!(check_group_state(None, ADMIN, &genesis), Ok(()));
        assert_eq!(
            check_group_state(None, ADMIN, &successor),
            Err(RuleViolation::EpochGap {
                expected: 1,
                found: 5
            })
        );
    }

    #[test]
    fn roster_changes_respect_admin_and_member_limits() {
        let members = roster(&[ADMIN, PEER_B, PEER_C]);
        assert_eq!(
            check_roster_change(&request(ADMIN, "peer-d", RosterAction::Add, ADMIN, false, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, ADMIN, RosterAction::Add, ADMIN, false, &members)),
            Err(RuleViolation::AlreadyMember)
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, PEER_B, RosterAction::Remove, ADMIN, false, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, PEER_B, RosterAction::Mute, ADMIN, false, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, "peer-z", RosterAction::Mute, ADMIN, false, &members)),
            Err(RuleViolation::NotMember)
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, ADMIN, RosterAction::Remove, ADMIN, false, &members)),
            Err(RuleViolation::AdminCannotLeave)
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, ADMIN, RosterAction::Leave, ADMIN, false, &members)),
            Err(RuleViolation::AdminCannotLeave)
        );
        assert_eq!(
            check_roster_change(&request(PEER_B, PEER_B, RosterAction::Leave, ADMIN, false, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(PEER_B, PEER_C, RosterAction::Remove, ADMIN, false, &members)),
            Err(RuleViolation::MemberCannotRemoveAnother)
        );
        assert_eq!(
            check_roster_change(&request(PEER_B, "peer-d", RosterAction::Add, ADMIN, false, &members)),
            Err(RuleViolation::NotAdmin)
        );
        assert_eq!(
            check_roster_change(&request(PEER_B, PEER_C, RosterAction::Mute, ADMIN, false, &members)),
            Err(RuleViolation::NotAdmin)
        );
    }

    #[test]
    fn everyone_group_rules() {
        let peers = roster(&[PEER_B, ADMIN, PEER_C]);
        let members = everyone_roster(&peers);
        assert_eq!(members, roster(&[ADMIN, PEER_B, PEER_C]));
        assert!(is_everyone_member(PEER_C, &peers));
        assert!(!is_everyone_member("peer-z", &peers));
        // Admins can add, remove and mute; every peer is already a member.
        assert_eq!(
            check_roster_change(&request(ADMIN, PEER_B, RosterAction::Mute, ADMIN, true, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, PEER_B, RosterAction::Remove, ADMIN, true, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, "peer-z", RosterAction::Add, ADMIN, true, &members)),
            Ok(())
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, PEER_B, RosterAction::Add, ADMIN, true, &members)),
            Err(RuleViolation::AlreadyMember),
            "every peer is already an Everyone member"
        );
        // No member may remove another, and the admin may not leave.
        assert_eq!(
            check_roster_change(&request(PEER_B, ADMIN, RosterAction::Remove, ADMIN, true, &members)),
            Err(RuleViolation::MemberCannotRemoveAnother)
        );
        assert_eq!(
            check_roster_change(&request(ADMIN, ADMIN, RosterAction::Leave, ADMIN, true, &members)),
            Err(RuleViolation::AdminCannotLeave)
        );
        // A signed roster state for Everyone is always refused.
        assert_eq!(
            check_everyone_state_change(),
            Err(RuleViolation::EveryoneRosterImmutable)
        );
    }

    #[test]
    fn announcements_respect_the_admins_only_setting_and_ack_once() {
        let members = roster(&[ADMIN, PEER_B]);
        assert_eq!(
            check_announcement_authority(PEER_B, ADMIN, &members, false),
            Ok(())
        );
        assert_eq!(
            check_announcement_authority(PEER_B, ADMIN, &members, true),
            Err(RuleViolation::NotAdmin)
        );
        assert_eq!(
            check_announcement_authority(ADMIN, ADMIN, &members, true),
            Ok(())
        );
        assert_eq!(
            check_announcement_authority("peer-z", ADMIN, &members, false),
            Err(RuleViolation::NotMember)
        );
        let acked = roster(&[PEER_B]);
        assert_eq!(check_announcement_ack(&acked, "peer-c"), Ok(()));
        assert_eq!(
            check_announcement_ack(&acked, PEER_B),
            Err(RuleViolation::AlreadyAcknowledged)
        );
    }

    #[test]
    fn rosters_are_canonicalised() {
        let messy = roster(&[PEER_B, PEER_C, PEER_B, " ", "peer-a"]);
        assert_eq!(
            canonical_roster(&messy),
            roster(&["peer-a", PEER_B, PEER_C])
        );
    }
}
