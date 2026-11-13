use argon2::Argon2;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    XChaCha20Poly1305, XNonce,
};
use rand_core::{OsRng, RngCore};

const BACKUP_MAGIC: &[u8; 8] = b"ALVBKUP1";
const BACKUP_HEADER_BYTES: usize = 8 + 16 + 24;
pub const MAX_BACKUP_BYTES: usize = 1024 * 1024 * 1024;

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

/// Encrypt an opaque backup payload with a fresh salt and nonce.
/// The returned bytes are: magic, 16-byte Argon2 salt, 24-byte XChaCha nonce,
/// followed by authenticated ciphertext.
pub fn encrypt_backup(passphrase: &str, plaintext: &[u8]) -> Result<Vec<u8>, String> {
    if plaintext.len() > MAX_BACKUP_BYTES {
        return Err("backup exceeds the size limit".into());
    }
    let mut salt = [0_u8; 16];
    let mut nonce = [0_u8; 24];
    OsRng.fill_bytes(&mut salt);
    OsRng.fill_bytes(&mut nonce);
    let key = derive_lan_key(passphrase, &salt)?;
    let ciphertext = encrypt_payload(&key, &nonce, plaintext)?;
    let mut bundle = Vec::with_capacity(BACKUP_HEADER_BYTES + ciphertext.len());
    bundle.extend_from_slice(BACKUP_MAGIC);
    bundle.extend_from_slice(&salt);
    bundle.extend_from_slice(&nonce);
    bundle.extend_from_slice(&ciphertext);
    Ok(bundle)
}

pub fn decrypt_backup(passphrase: &str, bundle: &[u8]) -> Result<Vec<u8>, String> {
    if bundle.len() < BACKUP_HEADER_BYTES + 16
        || bundle.len() > BACKUP_HEADER_BYTES + MAX_BACKUP_BYTES + 16
    {
        return Err("backup container size is invalid".into());
    }
    if bundle.get(..BACKUP_MAGIC.len()) != Some(BACKUP_MAGIC) {
        return Err("backup format is not recognized".into());
    }
    let salt: [u8; 16] = bundle[8..24].try_into().expect("fixed-size backup salt");
    let nonce: [u8; 24] = bundle[24..48].try_into().expect("fixed-size backup nonce");
    let key = derive_lan_key(passphrase, &salt)?;
    decrypt_payload(&key, &nonce, &bundle[BACKUP_HEADER_BYTES..])
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

    #[test]
    fn backup_container_encrypts_authenticates_and_uses_fresh_nonces() {
        let first = encrypt_backup("backup phrase", b"database + settings").unwrap();
        let second = encrypt_backup("backup phrase", b"database + settings").unwrap();
        assert_ne!(first, second);
        assert_eq!(
            decrypt_backup("backup phrase", &first).unwrap(),
            b"database + settings"
        );
        assert!(decrypt_backup("wrong phrase", &first).is_err());
        let mut tampered = first;
        *tampered.last_mut().unwrap() ^= 0x80;
        assert!(decrypt_backup("backup phrase", &tampered).is_err());
    }
}
