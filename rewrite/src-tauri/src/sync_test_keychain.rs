//! macOS Sync Test only. The adjacent password file makes this weaker than the login keychain.
use super::{mac_error, reference, Store};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use security_framework::os::macos::keychain::{CreateOptions, SecKeychain};
use std::{
    fs::{self, DirBuilder, OpenOptions},
    io::{Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt},
    path::Path,
    sync::Mutex,
};
use zeroize::Zeroizing;

static ACCESS: Mutex<()> = Mutex::new(());
const FILE_ERROR: &str = "Cannot open the private Sync Test keychain files. The credential directory must belong to this user with mode 0700, and its files must have mode 0600. Keep the old test data before starting a fresh test folder.";

fn private_metadata(path: &Path, directory: bool) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|_| FILE_ERROR)?;
    let expected = if directory { 0o700 } else { 0o600 };
    if m.file_type().is_symlink()
        || m.uid() != unsafe { libc::geteuid() }
        || (m.mode() & 0o777) != expected
        || (directory && !m.is_dir())
        || (!directory && (!m.is_file() || m.nlink() != 1))
    {
        return Err(FILE_ERROR.into());
    }
    Ok(())
}

fn password(path: &Path, keychain_exists: bool) -> Result<Zeroizing<String>, String> {
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
    {
        Ok(mut file) => {
            // Never replace a missing password for an existing keychain.
            if keychain_exists {
                drop(file);
                fs::remove_file(path).map_err(|_| FILE_ERROR)?;
                return Err(FILE_ERROR.into());
            }
            let random = Zeroizing::new(crate::sync_crypto::random::<32>());
            let value = Zeroizing::new(URL_SAFE_NO_PAD.encode(&*random));
            file.write_all(value.as_bytes()).map_err(|_| FILE_ERROR)?;
            file.sync_all().map_err(|_| FILE_ERROR)?;
            private_metadata(path, false)?;
            Ok(value)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            private_metadata(path, false)?;
            let file = OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NOFOLLOW)
                .open(path)
                .map_err(|_| FILE_ERROR)?;
            let m = file.metadata().map_err(|_| FILE_ERROR)?;
            if m.len() != 43
                || m.uid() != unsafe { libc::geteuid() }
                || m.mode() & 0o777 != 0o600
                || m.nlink() != 1
            {
                return Err(FILE_ERROR.into());
            }
            let mut value = Zeroizing::new(String::new());
            file.take(44)
                .read_to_string(&mut value)
                .map_err(|_| FILE_ERROR)?;
            if value.len() != 43
                || !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
            {
                return Err(FILE_ERROR.into());
            }
            Ok(value)
        }
        Err(_) => Err(FILE_ERROR.into()),
    }
}

fn open(store: &Store) -> Result<SecKeychain, String> {
    let directory = store.root().join("sync-test-keychain");
    store.check_owned(&directory)?;
    match DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => (),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => (),
        Err(_) => return Err(FILE_ERROR.into()),
    }
    private_metadata(&directory, true)?;
    let path = directory.join("credentials.keychain-db");
    let exists = match fs::symlink_metadata(&path) {
        Ok(_) => {
            private_metadata(&path, false)?;
            true
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err(FILE_ERROR.into()),
    };
    let password = password(&directory.join("password"), exists)?;
    let mut keychain = if exists {
        SecKeychain::open(&path).map_err(|e| mac_error("test-open", e.code()))?
    } else {
        let keychain = CreateOptions::new()
            .password(&password)
            .prompt_user(false)
            .create(&path)
            .map_err(|e| mac_error("test-create", e.code()))?;
        // The directory is private before Security.framework writes the keychain.
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&path)
            .map_err(|_| FILE_ERROR)?;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| FILE_ERROR)?;
        private_metadata(&path, false)?;
        keychain
    };
    keychain
        .unlock(Some(&password))
        .map_err(|e| mac_error("test-unlock", e.code()))?;
    Ok(keychain)
}

fn with_keychain<T>(
    store: &Store,
    f: impl FnOnce(SecKeychain) -> Result<T, String>,
) -> Result<T, String> {
    let _lock = ACCESS.lock().map_err(|_| FILE_ERROR)?;
    // Never wait on an ACL prompt in unattended tests. Preserve the prior interaction setting.
    let _interaction = if SecKeychain::user_interaction_allowed()
        .map_err(|e| mac_error("test-ui-status", e.code()))?
    {
        Some(
            SecKeychain::disable_user_interaction()
                .map_err(|e| mac_error("test-disable-ui", e.code()))?,
        )
    } else {
        None
    };
    f(open(store)?)
}

pub(super) fn protect(store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    with_keychain(store, |mut keychain| {
        let id = uuid::Uuid::new_v4().to_string();
        keychain
            .add_generic_password(crate::flavor::SYNC_KEYRING, &id, bytes)
            .map_err(|e| mac_error("test-save", e.code()))?;
        Ok(id.into_bytes())
    })
}

pub(super) fn unprotect(store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    with_keychain(store, |keychain| {
        let (password, _) = keychain
            .find_generic_password(crate::flavor::SYNC_KEYRING, reference(bytes)?)
            .map_err(|e| mac_error("test-load", e.code()))?;
        Ok(password.to_vec())
    })
}

pub(super) fn forget(store: &Store, bytes: &[u8]) -> Result<(), String> {
    with_keychain(store, |keychain| {
        let id = reference(bytes)?;
        match keychain.find_generic_password(crate::flavor::SYNC_KEYRING, id) {
            Ok((_, item)) => item.delete(),
            Err(e) if e.code() == -25300 => return Ok(()),
            Err(e) => return Err(mac_error("test-delete", e.code())),
        }
        match keychain.find_generic_password(crate::flavor::SYNC_KEYRING, id) {
            Err(e) if e.code() == -25300 => Ok(()),
            Err(e) => Err(mac_error("test-delete-check", e.code())),
            Ok(_) => Err("macOS did not delete the Sync Test credential.".into()),
        }
    })
}

pub(super) fn restart_probe(store: &Store, phase: &str) -> Result<(), String> {
    // Public fixture, never an account credential. Verify reads it after an app restart, then deletes it.
    const ID: &str = "6ccceea4-5abb-4fa3-bc74-ec7a04111111";
    const VALUE: &[u8] = b"SonoJot Sync Test restart probe";
    match phase {
        "prepare" => {
            forget(store, ID.as_bytes())?;
            with_keychain(store, |mut keychain| {
                keychain
                    .add_generic_password(crate::flavor::SYNC_KEYRING, ID, VALUE)
                    .map_err(|e| mac_error("test-probe-save", e.code()))?;
                Ok(())
            })
        }
        "verify" => {
            let actual = Zeroizing::new(unprotect(store, ID.as_bytes())?);
            forget(store, ID.as_bytes())?;
            if actual.as_slice() != VALUE {
                return Err("The saved keychain probe did not match.".into());
            }
            Ok(())
        }
        _ => Err("Choose roundtrip, prepare or verify for the keychain probe.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn password_is_private_persistent_and_rejects_links_or_open_permissions() {
        let root = std::env::temp_dir().join(format!("synctest-keychain-{}", uuid::Uuid::new_v4()));
        DirBuilder::new().mode(0o700).create(&root).unwrap();
        private_metadata(&root, true).unwrap();
        let path = root.join("password");
        let first = password(&path, false).unwrap();
        assert_eq!(*first, *password(&path, true).unwrap());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(password(&path, true).is_err());
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let link = root.join("linked");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(password(&link, true).is_err());
        fs::remove_file(link).unwrap();
        fs::remove_file(path).unwrap();
        assert!(password(&root.join("missing"), true).is_err());
        fs::remove_dir(root).unwrap();
    }
}
