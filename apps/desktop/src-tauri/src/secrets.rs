//! Where this device's long-lived secrets live: a `secrets/` directory under the instance data
//! directory, holding one `0600` file per secret.
//!
//! The OS credential store is the opt-in (`ALVERCHAT_KEYCHAIN`). A macOS keychain item belongs
//! to the binary that created it, so a rebuilt or relocated app asks for the login password on
//! every launch, and an ad-hoc-signed build usually cannot get in at all. A profile whose keys
//! are already in the credential store keeps working: the first time a secret is missing from
//! its files it is copied across, and the keychain is never touched again.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
};
use tauri::AppHandle;

/// Non-empty value keeps secrets in the OS credential store instead of files.
const KEYCHAIN_VAR: &str = "ALVERCHAT_KEYCHAIN";

/// Where a profile keeps its secrets, inside the instance data directory.
const STORE_DIRECTORY: &str = "secrets";

/// The name that directory had while file-backed secrets were a developer-only escape hatch.
const LEGACY_DIRECTORY: &str = "dev-secrets";

/// Account names inside the store: one per long-lived secret this device keeps.
pub(crate) const DATABASE_KEY: &str = "database-encryption-key";
pub(crate) const IDENTITY_KEY: &str = "peer-identity-signing-key";

/// The store this process reads and writes secrets through.
pub(crate) enum Secrets {
    Keychain,
    Files { data: PathBuf, directory: PathBuf },
}

impl Secrets {
    /// Resolves the store for this instance: files unless a caller asked for the keychain.
    pub(crate) fn open(app: &AppHandle) -> Result<Self, String> {
        let data = crate::instance_data_dir(app)?;
        if std::env::var_os(KEYCHAIN_VAR).is_some_and(|value| !value.is_empty()) {
            return Ok(Self::Keychain);
        }
        Ok(Self::Files {
            directory: file_store(&data),
            data,
        })
    }

    /// The file store of a profile directory, with no app handle behind it. Tests build
    /// throwaway two-profile runs this way so nothing reaches the credential store.
    #[cfg(test)]
    pub(crate) fn files(data: PathBuf) -> Self {
        Self::Files {
            directory: file_store(&data),
            data,
        }
    }

    /// The secret stored for `account`, or `None` when this device has never made one.
    pub(crate) fn get(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Keychain => keychain_get(account),
            Self::Files { data, directory } => match fs::read(directory.join(account)) {
                Ok(secret) => Ok(Some(secret)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => {
                    copy_from_keychain(data, directory, account)
                }
                Err(error) => Err(format!("Cannot read {account}: {error}")),
            },
        }
    }

    pub(crate) fn set(&self, account: &str, secret: &[u8]) -> Result<(), String> {
        match self {
            Self::Keychain => keychain(account)?
                .set_secret(secret)
                .map_err(|error| format!("Cannot save {account} securely: {error}")),
            Self::Files { directory, .. } => write_private(directory, account, secret),
        }
    }

    /// Forgets one secret. Removing something that is not there is not an error.
    pub(crate) fn delete(&self, account: &str) -> Result<(), String> {
        match self {
            Self::Keychain => match keychain(account)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(format!("Cannot clear {account}: {error}")),
            },
            Self::Files { directory, .. } => match fs::remove_file(directory.join(account)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("Cannot clear {account}: {error}")),
            },
        }
    }
}

/// The file store for a profile, under whichever name it was created with.
fn file_store(data: &Path) -> PathBuf {
    let current = data.join(STORE_DIRECTORY);
    let legacy = data.join(LEGACY_DIRECTORY);
    if !current.is_dir() && legacy.is_dir() {
        return legacy;
    }
    current
}

/// Moves over a secret created before file-backed storage was the default. Only a profile that
/// already has an encrypted database can have one, so a new profile never reaches the store.
///
/// A key that cannot be read is an error rather than a reason to make a new one: the database is
/// bound to the key that created it, and a fresh key would leave `messages.db` unreadable for
/// good.
fn copy_from_keychain(
    data: &Path,
    directory: &Path,
    account: &str,
) -> Result<Option<Vec<u8>>, String> {
    if !data.join(crate::MESSAGE_DB_FILE).is_file() {
        return Ok(None);
    }
    let secret = match keychain_get(account) {
        Ok(Some(secret)) => secret,
        Ok(None) => return Ok(None),
        Err(error) => {
            return Err(format!(
                "{error} This profile's keys were created in the system credential store. Approve \
                 that prompt to move them into {}, or start AlverChat with {KEYCHAIN_VAR}=1 to keep \
                 using it.",
                directory.display()
            ))
        }
    };
    write_private(directory, account, &secret)?;
    Ok(Some(secret))
}

fn keychain_get(account: &str) -> Result<Option<Vec<u8>>, String> {
    match keychain(account)?.get_secret() {
        Ok(secret) => Ok(Some(secret)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(error) => Err(format!("Cannot read {account} securely: {error}")),
    }
}

fn keychain(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&crate::keychain_service(), account)
        .map_err(|error| format!("Cannot access the system credential store: {error}"))
}

/// Creates the file readable by this user alone, so keeping keys out of the credential store is
/// not also a leak to everyone else sharing the machine.
fn write_private(directory: &Path, account: &str, secret: &[u8]) -> Result<(), String> {
    fs::create_dir_all(directory).map_err(|error| error.to_string())?;
    let path = directory.join(account);
    let mut options = OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&path)
        .map_err(|error| format!("Cannot create {}: {error}", path.display()))?;
    file.write_all(secret)
        .map_err(|error| format!("Cannot save {account}: {error}"))?;
    file.sync_all()
        .map_err(|error| format!("Cannot save {account}: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_profile_keeps_the_store_it_was_created_with() {
        let root =
            std::env::temp_dir().join(format!("alverchat-secrets-store-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join(LEGACY_DIRECTORY)).unwrap();
        assert_eq!(file_store(&root), root.join(LEGACY_DIRECTORY));
        fs::create_dir_all(root.join(STORE_DIRECTORY)).unwrap();
        assert_eq!(file_store(&root), root.join(STORE_DIRECTORY));
        fs::remove_dir_all(&root).unwrap();
    }
}
