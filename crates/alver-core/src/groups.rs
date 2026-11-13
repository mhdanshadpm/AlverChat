//! Signed group roster state with epoch and role checks.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{identity::Identity, peer_id};

pub const EVERYONE_GROUP_ID: &str = "everyone";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GroupState {
    pub id: String,
    pub name: String,
    pub epoch: u64,
    pub admin: String,
    pub members: Vec<String>,
    pub admin_public_key: [u8; 32],
    pub signature: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GroupError {
    InvalidIdentity,
    InvalidState,
    NotAdmin,
    CannotChangeEveryone,
    CannotRemoveAdmin,
}

impl GroupState {
    pub fn create(
        identity: &Identity,
        id: String,
        name: String,
        members: impl IntoIterator<Item = String>,
    ) -> Result<Self, GroupError> {
        let admin_public_key = identity.verifying_key().to_bytes();
        let admin = peer_id(&admin_public_key);
        let mut members: Vec<_> = members.into_iter().collect();
        members.push(admin.clone());
        members.sort();
        members.dedup();
        let mut state = Self {
            id,
            name,
            epoch: 1,
            admin,
            members,
            admin_public_key,
            signature: Vec::new(),
        };
        state.sign(identity)?;
        Ok(state)
    }

    pub fn verify(&self) -> bool {
        if self.id.is_empty()
            || self.name.trim().is_empty()
            || self.epoch == 0
            || self.members.is_empty()
            || self.members.windows(2).any(|pair| pair[0] >= pair[1])
            || !self.members.binary_search(&self.admin).is_ok()
            || self.admin != peer_id(&self.admin_public_key)
        {
            return false;
        }
        let Ok(key) = VerifyingKey::from_bytes(&self.admin_public_key) else {
            return false;
        };
        let Ok(signature_bytes) = <[u8; 64]>::try_from(self.signature.as_slice()) else {
            return false;
        };
        Identity::verify(
            &key,
            &self.signing_bytes(),
            &Signature::from_bytes(&signature_bytes),
        )
    }

    /// Check an incoming signed roster against the currently accepted epoch.
    pub fn verify_successor_of(&self, previous: &Self) -> bool {
        self.verify()
            && previous.verify()
            && self.id == previous.id
            && self.admin == previous.admin
            && self.admin_public_key == previous.admin_public_key
            && previous.epoch.checked_add(1) == Some(self.epoch)
    }

    pub fn update_members(
        &mut self,
        actor: &Identity,
        members: impl IntoIterator<Item = String>,
    ) -> Result<(), GroupError> {
        if self.id == EVERYONE_GROUP_ID {
            return Err(GroupError::CannotChangeEveryone);
        }
        if actor.peer_id() != self.admin {
            return Err(GroupError::NotAdmin);
        }
        if !self.verify() {
            return Err(GroupError::InvalidState);
        }
        let mut members: Vec<_> = members.into_iter().collect();
        if !members.iter().any(|member| member == &self.admin) {
            return Err(GroupError::CannotRemoveAdmin);
        }
        members.sort();
        members.dedup();
        self.members = members;
        self.epoch = self.epoch.saturating_add(1);
        self.sign(actor)
    }

    fn sign(&mut self, identity: &Identity) -> Result<(), GroupError> {
        if identity.peer_id() != self.admin {
            return Err(GroupError::InvalidIdentity);
        }
        self.signature = identity.sign(&self.signing_bytes()).to_bytes().to_vec();
        Ok(())
    }

    fn signing_bytes(&self) -> Vec<u8> {
        // Fields are encoded in a fixed order and members are canonicalized at creation/update.
        serde_json::to_vec(&(
            &self.id,
            &self.name,
            self.epoch,
            &self.admin,
            &self.members,
            self.admin_public_key,
        ))
        .expect("serializing a group state tuple cannot fail")
    }
}

/// One recipient of a group fan-out, ready to become an outbox row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeliveryTarget {
    /// Peer that must receive the envelope.
    pub recipient: String,
    /// Chat the envelope belongs to, normally the group id.
    pub chat_id: String,
    /// Roster epoch the envelope was fanned out under.
    pub epoch: u64,
    /// Deterministic per-recipient outbox row id.
    pub outbox_id: Uuid,
}

impl DeliveryTarget {
    #[must_use]
    pub fn new(recipient: &str, chat_id: &str, epoch: u64, envelope_id: Uuid) -> Self {
        Self {
            recipient: recipient.to_owned(),
            chat_id: chat_id.to_owned(),
            epoch,
            outbox_id: derive_outbox_id(envelope_id, recipient),
        }
    }
}

/// Deterministic outbox row id for one logical envelope and one recipient.
///
/// A group message is one logical message but many transports, so each recipient needs a
/// unique row id while the payload keeps carrying the original message id. blake3 over
/// `(envelope id, recipient)` gives the same id on every device and on every retry.
#[must_use]
pub fn derive_outbox_id(envelope_id: Uuid, recipient: &str) -> Uuid {
    let mut hasher = blake3::Hasher::new();
    hasher.update(envelope_id.as_bytes());
    hasher.update(recipient.as_bytes());
    let digest = hasher.finalize();
    let mut bytes = [0_u8; 16];
    bytes.copy_from_slice(&digest.as_bytes()[..16]);
    Uuid::from_bytes(bytes)
}

/// Pure fan-out: sorted, deduplicated recipients, never the sender.
///
/// The sender already holds their own copy, so echoing a group message back to them would
/// duplicate it in the UI and re-encrypt it for nothing. Empty identifiers are dropped so a
/// half-opened peer row cannot produce a delivery that no one can acknowledge.
#[must_use]
pub fn fan_out<I, S>(member_ids: I, sender_id: &str) -> Vec<String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut recipients: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for member in member_ids {
        let member = member.as_ref();
        if member.is_empty() || member == sender_id {
            continue;
        }
        recipients.insert(member.to_owned());
    }
    recipients.into_iter().collect()
}

impl GroupState {
    /// True when `peer_id` is on the signed roster.
    #[must_use]
    pub fn is_member(&self, peer_id: &str) -> bool {
        self.members.iter().any(|member| member == peer_id)
    }

    /// True when `peer_id` holds the signing admin role of this group.
    #[must_use]
    pub fn is_admin(&self, peer_id: &str) -> bool {
        self.admin == peer_id
    }

    /// Recipients for a group message sent by `sender_id`.
    #[must_use]
    pub fn fan_out(&self, sender_id: &str) -> Vec<String> {
        fan_out(&self.members, sender_id)
    }

    /// Per-member delivery plan for the outbox, excluding the sender.
    #[must_use]
    pub fn delivery_plan(&self, sender_id: &str, envelope_id: Uuid) -> Vec<DeliveryTarget> {
        delivery_plan_for(&self.members, sender_id, &self.id, self.epoch, envelope_id)
    }
}

/// Build a [`DeliveryTarget`] per recipient for any roster shape.
///
/// Used by the Everyone group, whose roster comes from discovered peers instead of a
/// signed [`GroupState`].
#[must_use]
pub fn delivery_plan_for(
    member_ids: &[String],
    sender_id: &str,
    chat_id: &str,
    epoch: u64,
    envelope_id: Uuid,
) -> Vec<DeliveryTarget> {
    fan_out(member_ids, sender_id)
        .into_iter()
        .map(|recipient| DeliveryTarget::new(&recipient, chat_id, epoch, envelope_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn admin_updates_members_with_monotonic_signed_epoch() {
        let admin = Identity::from_secret([3; 32]);
        let other = Identity::from_secret([4; 32]);
        let mut group = GroupState::create(
            &admin,
            "group-1".into(),
            "Design Team".into(),
            [other.peer_id()],
        )
        .unwrap();
        assert!(group.verify());
        assert_eq!(group.epoch, 1);
        group
            .update_members(&admin, [admin.peer_id(), "peer-c".into()])
            .unwrap();
        assert_eq!(group.epoch, 2);
        assert!(group.verify());
        let previous = GroupState::create(
            &admin,
            "group-1".into(),
            "Design Team".into(),
            [other.peer_id()],
        )
        .unwrap();
        assert!(group.verify_successor_of(&previous));
        assert!(!previous.verify_successor_of(&group));
    }

    #[test]
    fn non_admin_and_admin_removal_are_rejected() {
        let admin = Identity::from_secret([5; 32]);
        let other = Identity::from_secret([6; 32]);
        let mut group = GroupState::create(
            &admin,
            "group-2".into(),
            "Billing".into(),
            [other.peer_id()],
        )
        .unwrap();
        assert_eq!(
            group.update_members(&other, [admin.peer_id()]),
            Err(GroupError::NotAdmin)
        );
        assert_eq!(
            group.update_members(&admin, [other.peer_id()]),
            Err(GroupError::CannotRemoveAdmin)
        );
    }

    #[test]
    fn everyone_group_is_immutable_and_tampering_is_detected() {
        let admin = Identity::from_secret([7; 32]);
        let mut group = GroupState::create(
            &admin,
            EVERYONE_GROUP_ID.into(),
            "Everyone".into(),
            std::iter::empty(),
        )
        .unwrap();
        assert_eq!(
            group.update_members(&admin, [admin.peer_id()]),
            Err(GroupError::CannotChangeEveryone)
        );
        group.name = "Renamed".into();
        assert!(!group.verify());
    }

    #[test]
    fn fan_out_excludes_the_sender_and_is_deterministic() {
        let admin = Identity::from_secret([11; 32]);
        let member = Identity::from_secret([12; 32]);
        let group = GroupState::create(
            &admin,
            "group-fan".into(),
            "Design".into(),
            [member.peer_id(), "peer-c".into()],
        )
        .unwrap();
        let recipients = group.fan_out(&admin.peer_id());
        assert!(!recipients.contains(&admin.peer_id()), "no self delivery");
        assert!(recipients.contains(&member.peer_id()));
        assert!(recipients.contains(&"peer-c".into()));
        assert_eq!(recipients, group.fan_out(&admin.peer_id()));
        // A member sending is fanned out to everybody else, including the admin.
        let from_member = group.fan_out(&member.peer_id());
        assert_eq!(from_member.len(), recipients.len());
        assert!(from_member.contains(&admin.peer_id()));
        assert!(!from_member.contains(&member.peer_id()));
        // Duplicates, blanks and the sender are all dropped, order is sorted.
        let messy = fan_out(
            ["zed".to_owned(), "abc".to_owned(), "zed".to_owned(), String::new(), "abc".to_owned()],
            "zed",
        );
        assert_eq!(messy, vec!["abc".to_owned()]);
        assert!(fan_out(std::iter::empty::<String>(), "anyone").is_empty());
        assert!(group.fan_out("not-even-a-member").len() == group.members.len());
    }

    #[test]
    fn delivery_plan_gives_every_recipient_its_own_outbox_row() {
        let admin = Identity::from_secret([13; 32]);
        let member = Identity::from_secret([14; 32]);
        let group = GroupState::create(
            &admin,
            "group-plan".into(),
            "Standup".into(),
            [member.peer_id(), "peer-d".into()],
        )
        .unwrap();
        let envelope_id = Uuid::now_v7();
        let plan = group.delivery_plan(&admin.peer_id(), envelope_id);
        let expected = vec![member.peer_id(), "peer-d".to_owned()];
        let listed: Vec<_> = plan.iter().map(|target| target.recipient.clone()).collect();
        assert_eq!(
            listed,
            {
                let mut sorted = expected.clone();
                sorted.sort();
                sorted
            }
        );
        assert_eq!(plan.len(), 2);
        assert!(plan.iter().all(|target| target.chat_id == group.id));
        assert!(plan.iter().all(|target| target.epoch == group.epoch));
        // Row ids are unique per recipient but reproducible for the same envelope.
        let ids: std::collections::BTreeSet<_> =
            plan.iter().map(|target| target.outbox_id).collect();
        assert_eq!(ids.len(), plan.len());
        assert_eq!(plan, group.delivery_plan(&admin.peer_id(), envelope_id));
        assert_ne!(
            plan,
            group.delivery_plan(&admin.peer_id(), Uuid::now_v7()),
            "a different envelope gets different rows"
        );
        assert_eq!(
            derive_outbox_id(envelope_id, "peer-d"),
            plan[1].outbox_id,
            "row ids are derived, not random"
        );
        assert!(group
            .delivery_plan(&member.peer_id(), envelope_id)
            .iter()
            .all(|target| target.recipient != member.peer_id()));
    }

    #[test]
    fn everyone_plan_uses_the_discovered_peer_roster() {
        let local = Identity::from_secret([15; 32]);
        let peer = Identity::from_secret([16; 32]);
        let roster = crate::rules::everyone_roster(&[local.peer_id(), peer.peer_id()]);
        assert_eq!(roster.len(), 2);
        let envelope_id = Uuid::now_v7();
        let plan = delivery_plan_for(&roster, &local.peer_id(), EVERYONE_GROUP_ID, 1, envelope_id);
        assert_eq!(plan.len(), 1);
        assert_eq!(plan[0].recipient, peer.peer_id());
        assert_eq!(plan[0].chat_id, EVERYONE_GROUP_ID);
        assert_eq!(plan[0].epoch, 1);
        assert!(GroupState::create(
            &local,
            EVERYONE_GROUP_ID.into(),
            "Everyone".into(),
            std::iter::empty(),
        )
        .unwrap()
        .is_admin(&local.peer_id()));
    }
}
