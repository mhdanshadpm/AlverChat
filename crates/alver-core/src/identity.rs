use ed25519_dalek::{Signature, Signer, SigningKey, Verifier, VerifyingKey};
use rand_core::OsRng;

/// A fresh local identity. Persist `secret` only through the OS keychain layer.
pub struct Identity {
    secret: SigningKey,
}

impl Identity {
    pub fn generate() -> Self {
        Self {
            secret: SigningKey::generate(&mut OsRng),
        }
    }

    pub fn from_secret(bytes: [u8; 32]) -> Self {
        Self {
            secret: SigningKey::from_bytes(&bytes),
        }
    }

    pub fn secret_bytes(&self) -> [u8; 32] {
        self.secret.to_bytes()
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        self.secret.verifying_key()
    }

    pub fn peer_id(&self) -> String {
        crate::peer_id(self.verifying_key().as_bytes())
    }

    pub fn fingerprint(&self) -> String {
        let bytes = blake3::hash(self.verifying_key().as_bytes());
        bytes
            .as_bytes()
            .chunks(2)
            .map(|pair| format!("{:02X}{:02X}", pair[0], pair[1]))
            .collect::<Vec<_>>()
            .join(" ")
    }

    pub fn sign(&self, value: &[u8]) -> Signature {
        self.secret.sign(value)
    }

    pub fn verify(key: &VerifyingKey, value: &[u8], signature: &Signature) -> bool {
        key.verify(value, signature).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_id_and_fingerprint_from_secret() {
        let identity = Identity::from_secret([7; 32]);
        let restored = Identity::from_secret(identity.secret_bytes());
        assert_eq!(identity.peer_id(), restored.peer_id());
        assert_eq!(identity.fingerprint(), restored.fingerprint());
        assert_eq!(identity.fingerprint().split(' ').count(), 16);
    }

    #[test]
    fn generated_identities_are_distinct() {
        assert_ne!(
            Identity::generate().peer_id(),
            Identity::generate().peer_id()
        );
    }
}
