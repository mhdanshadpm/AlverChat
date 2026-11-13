use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};

pub fn derive_lan_key(passphrase: &str, salt: &[u8; 16]) -> Result<[u8; 32], String> {
    let mut key = [0_u8; 32];
    Argon2::default()
        .hash_password_into(passphrase.as_bytes(), salt, &mut key)
        .map_err(|error| error.to_string())?;
    Ok(key)
}

pub fn encrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 24],
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    XChaCha20Poly1305::new(key.into())
        .encrypt(XNonce::from_slice(nonce), plaintext)
        .map_err(|_| "payload encryption failed".to_owned())
}

pub fn decrypt_payload(
    key: &[u8; 32],
    nonce: &[u8; 24],
    ciphertext: &[u8],
) -> Result<Vec<u8>, String> {
    XChaCha20Poly1305::new(key.into())
        .decrypt(XNonce::from_slice(nonce), ciphertext)
        .map_err(|_| "payload authentication failed".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_authentication_rejects_wrong_key() {
        let nonce = [4; 24];
        let encrypted = encrypt_payload(&[1; 32], &nonce, b"lan message").unwrap();
        assert_eq!(
            decrypt_payload(&[1; 32], &nonce, &encrypted).unwrap(),
            b"lan message"
        );
        assert!(decrypt_payload(&[2; 32], &nonce, &encrypted).is_err());
    }

    #[test]
    fn passphrase_derivation_is_stable_and_salt_specific() {
        assert_eq!(
            derive_lan_key("secret", &[1; 16]),
            derive_lan_key("secret", &[1; 16])
        );
        assert_ne!(
            derive_lan_key("secret", &[1; 16]),
            derive_lan_key("secret", &[2; 16])
        );
    }
}
