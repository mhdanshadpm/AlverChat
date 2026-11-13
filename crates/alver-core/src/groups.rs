//! Signed group roster state with epoch and role checks.

use ed25519_dalek::{Signature, VerifyingKey};
use serde::{Deserialize, Serialize};

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
}
