//! Where this device's long-lived secrets live.
//!
//! Normally the OS credential store. With `ALVERCHAT_DEV_SECRETS` set in a debug build they
//! live in a private file under the instance data directory instead: every `tauri dev`
//! rebuild is a new ad-hoc-signed binary, and macOS will not let it read keychain items an
//! earlier build created, so the keychain turns each rebuild into a password prompt. The
//! directory path is compiled out of release builds, so a shipped app can never be talked
//! into keeping keys in a file.

use std::{
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Once,
};
use tauri::AppHandle;

const DEV_VAR: &str = "ALVERCHAT_DEV_SECRETS";

/// Where a file-backed profile keeps its secrets, inside the instance data directory.
const DEV_DIRECTORY: &str = "dev-secrets";

/// Account names inside the store: one per long-lived secret this device keeps.
pub(crate) const DATABASE_KEY: &str = "database-encryption-key";
pub(crate) const IDENTITY_KEY: &str = "peer-identity-signing-key";

static ANNOUNCED: Once = Once::new();

/// The store this process reads and writes secrets through.
pub(crate) enum Secrets {
    Keychain,
    #[cfg_attr(not(debug_assertions), allow(dead_code))]
    Directory(PathBuf),
}

impl Secrets {
    /// Resolves the store for this instance: a directory only when a developer asked.
    ///
    /// The database is encrypted with whichever key it was created with, so a profile cannot
    /// change stores: both directions are refused with a message that says what to do,
    /// instead of letting the database open fail as "file is not a database".
    pub(crate) fn open(app: &AppHandle) -> Result<Self, String> {
        let data = crate::instance_data_dir(app)?;
        let directory = data.join(DEV_DIRECTORY);
        let file_backed = directory.join(DATABASE_KEY).is_file();
        let wanted = cfg!(debug_assertions)
            && std::env::var_os(DEV_VAR).is_some_and(|value| !value.is_empty());
        if !wanted {
            if file_backed {
                return Err(format!(
                    "This profile keeps its keys in {} (a developer flag). Remove that folder to open it with the keychain again.",
                    directory.display()
                ));
            }
            return Ok(Self::Keychain);
        }
        if !file_backed && data.join(crate::MESSAGE_DB_FILE).is_file() {
            return Err(format!(
                "{DEV_VAR} keeps keys in {}, but this profile's messages were encrypted with the keychain key. Point ALVERCHAT_DATA_DIR at a scratch folder for a file-backed dev profile.",
                directory.display()
            ));
        }
        ANNOUNCED.call_once(|| {
            eprintln!(
                "Secrets: {DEV_VAR} is set, keeping keys in {} instead of the keychain.",
                directory.display()
            );
        });
        Ok(Self::Directory(directory))
    }

    /// The secret stored for `account`, or `None` when this device has never made one.
    pub(crate) fn get(&self, account: &str) -> Result<Option<Vec<u8>>, String> {
        match self {
            Self::Keychain => match keychain(account)?.get_secret() {
                Ok(secret) => Ok(Some(secret)),
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(error) => Err(format!("Cannot read {account} securely: {error}")),
            },
            Self::Directory(directory) => match fs::read(directory.join(account)) {
                Ok(secret) => Ok(Some(secret)),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                Err(error) => Err(format!("Cannot read {account}: {error}")),
            },
        }
    }

    pub(crate) fn set(&self, account: &str, secret: &[u8]) -> Result<(), String> {
        match self {
            Self::Keychain => keychain(account)?
                .set_secret(secret)
                .map_err(|error| format!("Cannot save {account} securely: {error}")),
            Self::Directory(directory) => write_private(directory, account, secret),
        }
    }

    /// Forgets one secret. Removing something that is not there is not an error.
    pub(crate) fn delete(&self, account: &str) -> Result<(), String> {
        match self {
            Self::Keychain => match keychain(account)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(format!("Cannot clear {account}: {error}")),
            },
            Self::Directory(directory) => match fs::remove_file(directory.join(account)) {
                Ok(()) => Ok(()),
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                Err(error) => Err(format!("Cannot clear {account}: {error}")),
            },
        }
    }
}

fn keychain(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(&crate::keychain_service(), account)
        .map_err(|error| format!("Cannot access the system credential store: {error}"))
}

/// Creates the file readable by this user alone, so the escape hatch is not also an official
/// leak to everyone with the same machine.
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
