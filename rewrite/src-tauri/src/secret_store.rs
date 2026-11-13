//! Sync credentials use OS protection. Only macOS Sync Test uses its own test keychain.
use crate::store::Store;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;
const FILE: &str = "sync-secrets.json";
const VERSION: u32 = if cfg!(all(target_os = "macos", feature = "synctest")) {
    2
} else {
    1
};
#[cfg(all(target_os = "macos", feature = "synctest"))]
#[path = "sync_test_keychain.rs"]
mod test_keychain;
const ERROR: &str = "Cannot unlock sync credentials. Unlock your system keyring and try again.";
const STORE_CHANGED: &str = "These sync credentials use a different credential store. Keep the old data folder and start a fresh Sync Test folder, then sign in and pair or use recovery.";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Saved {
    version: u32,
    protected: String,
}
pub fn load<T: DeserializeOwned>(store: &Store) -> Result<Option<T>, String> {
    let path = store.root().join(FILE);
    if !path.exists() {
        return Ok(None);
    }
    store.check_owned(&path)?;
    if std::fs::metadata(&path).map_err(|_| ERROR)?.len() > 65536 {
        return Err(ERROR.into());
    }
    let saved: Saved = store.read(&path).map_err(|_| ERROR)?;
    if saved.version != VERSION {
        return Err(STORE_CHANGED.into());
    }
    let reference = URL_SAFE_NO_PAD.decode(saved.protected).map_err(|_| ERROR)?;
    let bytes = Zeroizing::new(unprotect(store, &reference)?);
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| ERROR.into())
}
pub fn save<T: Serialize>(store: &Store, value: &T) -> Result<(), String> {
    let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| ERROR)?);
    if bytes.len() > 16384 {
        return Err("Sync credentials exceed the secure store limit.".into());
    }
    let path = store.root().join(FILE);
    let previous: Option<Saved> = if path.exists() {
        Some(store.read(&path)?)
    } else {
        None
    };
    if previous.as_ref().is_some_and(|old| old.version != VERSION) {
        return Err(STORE_CHANGED.into());
    }
    let protected = protect(store, &bytes)?;
    if let Err(e) = store.write(
        &path,
        &Saved {
            version: VERSION,
            protected: URL_SAFE_NO_PAD.encode(&protected),
        },
    ) {
        forget(store, &protected)?;
        return Err(e);
    }
    if let Some(old) = previous {
        forget(
            store,
            &URL_SAFE_NO_PAD.decode(old.protected).map_err(|_| ERROR)?,
        )?
    }
    Ok(())
}
pub fn delete(store: &Store) -> Result<(), String> {
    let path = store.root().join(FILE);
    if !path.exists() {
        return Ok(());
    }
    let old: Saved = store.read(&path)?;
    if old.version != VERSION {
        return Err(STORE_CHANGED.into());
    }
    forget(
        store,
        &URL_SAFE_NO_PAD.decode(old.protected).map_err(|_| ERROR)?,
    )?;
    store.check_owned(&path)?;
    std::fs::remove_file(path).map_err(|_| ERROR.into())
}
#[cfg(windows)]
fn protect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    crate::ai::protect_bytes(bytes, true).map_err(|_| ERROR.into())
}
#[cfg(windows)]
fn unprotect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    crate::ai::protect_bytes(bytes, false).map_err(|_| ERROR.into())
}
#[cfg(windows)]
fn forget(_store: &Store, _bytes: &[u8]) -> Result<(), String> {
    Ok(())
}
#[cfg(all(target_os = "macos", not(feature = "synctest")))]
fn protect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    security_framework::passwords::set_generic_password(crate::flavor::SYNC_KEYRING, &id, bytes)
        .map_err(|e| mac_error("save", e.code()))?;
    Ok(id.into_bytes())
}
#[cfg(all(target_os = "macos", not(feature = "synctest")))]
fn unprotect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    security_framework::passwords::get_generic_password(
        crate::flavor::SYNC_KEYRING,
        reference(bytes)?,
    )
    .map_err(|e| mac_error("load", e.code()))
}
#[cfg(all(target_os = "macos", not(feature = "synctest")))]
fn forget(_store: &Store, bytes: &[u8]) -> Result<(), String> {
    match security_framework::passwords::delete_generic_password(
        crate::flavor::SYNC_KEYRING,
        reference(bytes)?,
    ) {
        Ok(()) => Ok(()),
        Err(e) if e.code() == -25300 => Ok(()),
        Err(e) => Err(mac_error("delete", e.code())),
    }
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn reference(bytes: &[u8]) -> Result<&str, String> {
    let id = std::str::from_utf8(bytes).map_err(|_| ERROR)?;
    uuid::Uuid::parse_str(id).map_err(|_| ERROR)?;
    Ok(id)
}
#[cfg(target_os = "linux")]
fn keyring(args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};
    let mut child = Command::new("secret-tool")
        .args(args)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| ERROR)?;
    if let Some(input) = input {
        child
            .stdin
            .take()
            .ok_or(ERROR)?
            .write_all(input)
            .map_err(|_| ERROR)?
    }
    // Drain concurrently: a large credential must not fill the child's stdout pipe.
    let pipe = child.stdout.take().ok_or(ERROR)?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        pipe.take(32768).read_to_end(&mut out).map(|_| out)
    });
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|_| ERROR)? {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            let _ = reader.join();
            return Err(ERROR.into());
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    };
    let output = reader.join().map_err(|_| ERROR)?.map_err(|_| ERROR)?;
    if !status.success() {
        return Err(ERROR.into());
    }
    Ok(output)
}
#[cfg(target_os = "linux")]
fn protect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let id = uuid::Uuid::new_v4().to_string();
    keyring(
        &[
            "store",
            "--label=SonoJot sync",
            "application",
            crate::flavor::SYNC_KEYRING_APP,
            "item",
            &id,
        ],
        Some(bytes),
    )?;
    Ok(id.into_bytes())
}
#[cfg(target_os = "linux")]
fn unprotect(_store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    keyring(
        &[
            "lookup",
            "application",
            crate::flavor::SYNC_KEYRING_APP,
            "item",
            reference(bytes)?,
        ],
        None,
    )
}
#[cfg(target_os = "linux")]
fn forget(_store: &Store, bytes: &[u8]) -> Result<(), String> {
    keyring(
        &[
            "clear",
            "application",
            crate::flavor::SYNC_KEYRING_APP,
            "item",
            reference(bytes)?,
        ],
        None,
    )
    .map(|_| ())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn protect(_: &Store, _: &[u8]) -> Result<Vec<u8>, String> {
    Err(ERROR.into())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn unprotect(_: &Store, _: &[u8]) -> Result<Vec<u8>, String> {
    Err(ERROR.into())
}
#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn forget(_: &Store, _: &[u8]) -> Result<(), String> {
    Err(ERROR.into())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn dpapi_roundtrip_never_writes_plaintext() {
        let root =
            std::env::temp_dir().join(format!("sonojot-secret-test-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let secret = "synthetic-test-value".to_owned();
        save(&store, &secret).unwrap();
        assert_eq!(load::<String>(&store).unwrap(), Some(secret.clone()));
        assert!(!std::fs::read_to_string(root.join(FILE))
            .unwrap()
            .contains(&secret));
        delete(&store).unwrap();
        assert!(load::<String>(&store).unwrap().is_none());
        std::fs::remove_dir_all(root).unwrap()
    }
}

#[cfg(any(target_os = "macos", test))]
fn mac_error(operation: &'static str, status: i32) -> String {
    crate::recognition_diagnostics::sync_keychain_error(operation, status);
    let message = match status {
        -25308 | -25315 | -25337 => "The macOS keychain is locked or cannot show an unlock prompt in this session. Open SonoJot in your Mac desktop session, unlock the login keychain in Keychain Access, then try again.",
        -25300 => "Saved sync credentials are missing from the macOS keychain. Sign in again, then pair this device or use your recovery code.",
        -25293 | -25243 | -128 => "macOS denied access to the saved sync credentials. Review SonoJot's access in Keychain Access, then try again.",
        -25291 | -25294 | -25307 => "The macOS login keychain is unavailable. Open Keychain Access in your Mac desktop session and check that the login keychain is available.",
        _ => "macOS could not access sync credentials. Check the sync keychain error in the app diagnostic log before trying again.",
    };
    format!("{message} (OSStatus {status})")
}

#[cfg(all(target_os = "macos", feature = "synctest"))]
fn protect(store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    test_keychain::protect(store, bytes)
}
#[cfg(all(target_os = "macos", feature = "synctest"))]
fn unprotect(store: &Store, bytes: &[u8]) -> Result<Vec<u8>, String> {
    test_keychain::unprotect(store, bytes)
}
#[cfg(all(target_os = "macos", feature = "synctest"))]
fn forget(store: &Store, bytes: &[u8]) -> Result<(), String> {
    test_keychain::forget(store, bytes)
}

#[cfg(all(target_os = "macos", feature = "synctest"))]
pub fn probe(store: &Store, phase: &str) -> Result<(), String> {
    if phase != "roundtrip" {
        return test_keychain::restart_probe(store, phase);
    }
    let bytes = Zeroizing::new(crate::sync_crypto::random::<32>());
    let protected = protect(store, &*bytes)?;
    let read = unprotect(store, &protected);
    let removed = forget(store, &protected);
    let read = Zeroizing::new(read?);
    removed?;
    if read.as_slice() != bytes.as_slice() {
        return Err("The test keychain did not preserve the probe.".into());
    }
    Ok(())
}

#[cfg(test)]
mod mac_errors {
    use super::*;
    #[test]
    fn keychain_errors_distinguish_unlock_missing_and_denied() {
        for status in [-25308, -25315, -25337] {
            let message = mac_error("save", status);
            assert!(message.contains("locked") && message.contains("prompt"));
            assert!(message.contains(&status.to_string()));
        }
        assert!(mac_error("load", -25300).contains("missing"));
        assert!(mac_error("load", -25293).contains("denied"));
        assert!(mac_error("load", -25291).contains("unavailable"));
        assert!(mac_error("save", -1).contains("diagnostic log"));
    }
    #[test]
    fn test_storage_version_is_only_selected_on_mac_sync_test() {
        assert_eq!(
            VERSION == 2,
            cfg!(all(target_os = "macos", feature = "synctest"))
        );
    }
}
