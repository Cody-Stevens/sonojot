//! Reversible remote deletions. Only owned paths are moved; source imports stay put.
use crate::store::Store;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
const RETENTION: u64 = 30 * 86400;
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub key: String,
    pub id: String,
    pub title: String,
    pub deleted_at: u64,
}
fn entry_path(store: &Store, key: &str) -> Result<PathBuf, String> {
    let (stamp, id) = key.split_once('-').ok_or("Invalid trash entry.")?;
    stamp.parse::<u64>().map_err(|_| "Invalid trash entry.")?;
    uuid::Uuid::parse_str(id).map_err(|_| "Invalid trash entry.")?;
    let path = store.root().join("meeting-trash").join(key);
    store.check_owned(&path)?;
    Ok(path)
}
fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn inspect(store: &Store, path: &Path, depth: usize) -> Result<(), String> {
    store.check_owned(path)?;
    if depth > 5 {
        return Err("Unexpected folder in meeting trash.".into());
    }
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| "Cannot inspect meeting trash.")? {
            inspect(
                store,
                &entry.map_err(|_| "Cannot inspect meeting trash.")?.path(),
                depth + 1,
            )?;
        }
    }
    Ok(())
}
pub fn retain(store: &Store, paths: &[PathBuf]) -> Result<(), String> {
    if !paths.iter().any(|p| p.exists()) {
        return Ok(());
    }
    let root = store.root().join("meeting-trash");
    store.check_owned(&root)?;
    fs::create_dir_all(&root).map_err(|_| "Cannot create meeting trash.")?;
    let target = root.join(format!("{}-{}", now(), uuid::Uuid::new_v4()));
    fs::create_dir(&target).map_err(|_| "Cannot create meeting trash entry.")?;
    // Small metadata makes Recently deleted independent of full layer histories.
    for path in paths
        .iter()
        .rev()
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
    {
        if !path.is_file() {
            continue;
        }
        if let Ok(value) = store.read::<Value>(path) {
            let record = if value.get("detail").is_some() {
                &value
            } else {
                &value["record"]
            };
            if let (Some(id), Some(title)) = (
                record["detail"]["id"].as_str(),
                record["detail"]["title"].as_str(),
            ) {
                crate::store::validate_id(id)?;
                store.write(
                    &target.join("entry.json"),
                    &Entry {
                        key: target.file_name().unwrap().to_string_lossy().into_owned(),
                        id: id.into(),
                        title: title.into(),
                        deleted_at: now(),
                    },
                )?;
                // An older build may already have removed the visible JSON.
                store.write(&target.join("record.json"), record)?;
                break;
            }
        }
    }
    for path in paths {
        if !path.exists() {
            continue;
        }
        inspect(store, path, 0)?;
        let relative = path
            .strip_prefix(store.root())
            .map_err(|_| "Meeting file is outside this store.")?;
        let dest = target.join(relative);
        fs::create_dir_all(dest.parent().ok_or("Invalid trash path.")?)
            .map_err(|_| "Cannot create meeting trash.")?;
        store.check_owned(&dest)?;
        fs::rename(path, dest).map_err(|_| "Cannot move the meeting to local trash.")?;
    }
    Ok(())
}
fn remove(store: &Store, path: &Path) -> Result<(), String> {
    store.check_owned(path)?;
    if path.is_dir() {
        for entry in fs::read_dir(path).map_err(|_| "Cannot read meeting trash.")? {
            remove(
                store,
                &entry.map_err(|_| "Cannot read meeting trash.")?.path(),
            )?;
        }
        fs::remove_dir(path).map_err(|_| "Cannot purge meeting trash.".into())
    } else {
        fs::remove_file(path).map_err(|_| "Cannot purge meeting trash.".into())
    }
}
pub fn purge(store: &Store) -> Result<(), String> {
    purge_at(store, now())
}
fn purge_at(store: &Store, time: u64) -> Result<(), String> {
    let root = store.root().join("meeting-trash");
    store.check_owned(&root)?;
    if !root.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(root).map_err(|_| "Cannot inspect meeting trash.")? {
        let path = entry.map_err(|_| "Cannot inspect meeting trash.")?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .ok_or("Invalid trash folder.")?;
        let Some((stamp, id)) = name.split_once('-') else {
            continue;
        };
        let (Ok(stamp), Ok(_)) = (stamp.parse::<u64>(), uuid::Uuid::parse_str(id)) else {
            continue;
        };
        if time.saturating_sub(stamp) >= RETENTION {
            let _guard = store.lock()?;
            inspect(store, &path, 0)?;
            remove(store, &path)?;
        }
    }
    Ok(())
}
fn metadata(store: &Store, path: &Path) -> Result<(Entry, Value), String> {
    if let (Ok(entry), Ok(record)) = (
        store.read::<Entry>(&path.join("entry.json")),
        store.read::<Value>(&path.join("record.json")),
    ) {
        return Ok((entry, record));
    }
    // Earlier nightlies retained owned files without an entry index.
    let key = path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("Invalid trash entry.")?;
    let deleted_at = key
        .split_once('-')
        .and_then(|(stamp, _)| stamp.parse().ok())
        .ok_or("Invalid trash date.")?;
    let relative = store
        .meetings_dir()
        .strip_prefix(store.root())
        .map_err(|_| "Invalid meeting directory.")?
        .to_path_buf();
    for dir in [path.join(relative), path.join("meeting-layers")] {
        if !dir.is_dir() {
            continue;
        }
        for candidate in fs::read_dir(dir)
            .map_err(|_| "Cannot inspect saved meetings.")?
            .take(100)
        {
            let candidate = candidate
                .map_err(|_| "Cannot inspect saved meeting.")?
                .path();
            if candidate.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let Ok(value) = store.read::<Value>(&candidate) else {
                continue;
            };
            let record = if value.get("detail").is_some() {
                value
            } else {
                value["record"].clone()
            };
            if let (Some(id), Some(title)) = (
                record["detail"]["id"].as_str(),
                record["detail"]["title"].as_str(),
            ) {
                crate::store::validate_id(id)?;
                return Ok((
                    Entry {
                        key: key.into(),
                        id: id.into(),
                        title: title.into(),
                        deleted_at,
                    },
                    record,
                ));
            }
        }
    }
    Err("Saved meeting details are unavailable.".into())
}
pub fn list(store: &Store) -> Result<Vec<Entry>, String> {
    let root = store.root().join("meeting-trash");
    store.check_owned(&root)?;
    if !root.exists() {
        return Ok(Vec::new());
    }
    let mut rows = Vec::new();
    for (count, entry) in fs::read_dir(root)
        .map_err(|_| "Cannot list recently deleted meetings.")?
        .enumerate()
    {
        if count > 20_000 {
            return Err("Recently deleted exceeds its supported limit.".into());
        }
        let path = entry
            .map_err(|_| "Cannot inspect recently deleted meetings.")?
            .path();
        let Some(key) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        if entry_path(store, key).is_err() {
            continue;
        }
        let row = store
            .read::<Entry>(&path.join("entry.json"))
            .or_else(|_| metadata(store, &path).map(|(entry, _)| entry));
        if let Ok(mut row) = row {
            if crate::store::validate_id(&row.id).is_err() {
                continue;
            }
            row.key = key.into();
            rows.push(row);
        }
    }
    rows.sort_by_key(|entry| std::cmp::Reverse(entry.deleted_at));
    Ok(rows)
}
pub fn delete_now(store: &Store, key: &str) -> Result<(), String> {
    let _guard = store.lock()?;
    let path = entry_path(store, key)?;
    inspect(store, &path, 0)?;
    remove(store, &path)
}
pub fn restore(store: &Store, key: &str) -> Result<String, String> {
    restore_with(store, key, move_missing_file)
}
// Source and destination share the same store/filesystem. Creating a hard link
// atomically refuses an existing destination on Windows and Unix alike.
fn move_missing_file(source: &Path, dest: &Path) -> std::io::Result<()> {
    if let Err(error) = fs::hard_link(source, dest) {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            return Err(error);
        }
        // Custom stores may use a filesystem without hard links. create_new
        // preserves the same no-overwrite guarantee, with a copy on that path.
        let mut input = fs::File::open(source)?;
        let mut output = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(dest)?;
        if let Err(error) = std::io::copy(&mut input, &mut output).and_then(|_| output.sync_all()) {
            drop(output);
            let _ = fs::remove_file(dest);
            return Err(error);
        }
    }
    if let Err(error) = fs::remove_file(source) {
        let _ = fs::remove_file(dest);
        return Err(error);
    }
    Ok(())
}
fn restore_with(
    store: &Store,
    key: &str,
    mut move_file: impl FnMut(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> Result<String, String> {
    let _guard = store.lock()?;
    let path = entry_path(store, key)?;
    inspect(store, &path, 0)?;
    let (entry, record) = metadata(store, &path)?;
    crate::store::validate_id(&entry.id)?;
    let id = &entry.id;
    let record_path = store.meetings_dir().join(format!("{id}.json"));
    let mut moves = Vec::new();
    let mut collisions = false;
    let relative = store
        .meetings_dir()
        .strip_prefix(store.root())
        .map_err(|_| "Invalid meeting directory.")?
        .to_path_buf();
    let mut names: Vec<PathBuf> = ["wav", "mic.wav", "system.wav", "import.wav", "asr", "json"]
        .iter()
        .map(|suffix| relative.join(format!("{id}.{suffix}")))
        .collect();
    names.extend(
        ["json", "legacy", "stamp"]
            .iter()
            .map(|suffix| PathBuf::from("meeting-layers").join(format!("{id}.{suffix}"))),
    );
    names.push(PathBuf::from("meeting-sync-audio").join(id));
    fn plan(
        store: &Store,
        source: PathBuf,
        dest: PathBuf,
        moves: &mut Vec<(PathBuf, PathBuf)>,
        collisions: &mut bool,
        depth: usize,
    ) -> Result<(), String> {
        use crate::meetings::layers::absent;
        if depth > 4 || moves.len() > 80_000 {
            return Err("Restore exceeds its limit.".into());
        }
        if absent(&source)? {
            return Ok(());
        }
        store.check_owned(&source)?;
        store.check_owned(&dest)?;
        let dest_missing = absent(&dest)?;
        if source.is_dir() && (dest_missing || dest.is_dir()) {
            fs::create_dir_all(&dest).map_err(|_| "Cannot prepare meeting restoration.")?;
            for entry in fs::read_dir(&source).map_err(|_| "Cannot list saved meeting files.")? {
                let entry = entry.map_err(|_| "Cannot list saved meeting files.")?;
                plan(
                    store,
                    entry.path(),
                    dest.join(entry.file_name()),
                    moves,
                    collisions,
                    depth + 1,
                )?;
            }
            return Ok(());
        }
        if !dest_missing {
            *collisions = true;
            return Ok(());
        }
        fs::create_dir_all(dest.parent().ok_or("Invalid restore path.")?)
            .map_err(|_| "Cannot prepare meeting restoration.")?;
        moves.push((source, dest));
        Ok(())
    }
    for name in names {
        plan(
            store,
            path.join(&name),
            store.root().join(&name),
            &mut moves,
            &mut collisions,
            0,
        )?;
    }
    if record["detail"]["id"] != *id {
        return Err("Trash record identity differs.".into());
    }
    for (index, (source, dest)) in moves.iter().enumerate() {
        if move_file(source, dest).is_err() {
            for (source, dest) in moves[..index].iter().rev() {
                let _ = move_missing_file(dest, source);
            }
            return Err(
                "Cannot restore all meeting files. The saved copy remains in Recently deleted."
                    .into(),
            );
        }
    }
    if crate::meetings::layers::absent(&record_path)? {
        let recovery = path.join(format!("restore-{}.json", uuid::Uuid::new_v4()));
        let result = store.write(&recovery, &record).and_then(|_| {
            move_missing_file(&recovery, &record_path)
                .map_err(|_| "Cannot restore meeting JSON without overwriting a file.".to_string())
        });
        if let Err(error) = result {
            let _ = fs::remove_file(&recovery);
            for (source, dest) in moves.iter().rev() {
                let _ = move_missing_file(dest, source);
            }
            return Err(error);
        }
    }
    // The lock prevents sync seeing an intermediate restore. Commit sync intent
    // only after every move succeeded; failures leave the queue and track intact.
    let saved_layer = store
        .root()
        .join("meeting-layers")
        .join(format!("{id}.json"));
    if let Ok(mut value) = store.read::<Value>(&saved_layer) {
        value["localOnly"] = Value::Bool(true);
        store.write(&saved_layer, &value)?;
    }
    crate::meeting_sync::keep_restored(store, id)?;
    // Colliding originals remain available in trash; never overwrite or discard
    // them just because a newer JSON or one sidecar already exists.
    if !collisions && remove(store, &path).is_err() {
        crate::meetings::layers::diagnostic("restored-trash-cleanup-failed");
    }
    Ok(id.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn restore_move_never_replaces_an_existing_destination() {
        let dir =
            std::env::temp_dir().join(format!("sj-trash-no-clobber-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source");
        let dest = dir.join("dest");
        fs::write(&source, b"synthetic saved").unwrap();
        fs::write(&dest, b"synthetic current").unwrap();
        assert!(move_missing_file(&source, &dest).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"synthetic saved");
        assert_eq!(fs::read(&dest).unwrap(), b"synthetic current");
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn failed_restore_rolls_back_files_without_changing_sync_intent() {
        let dir = std::env::temp_dir().join(format!("sj-trash-failure-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(dir.clone())).unwrap();
        let record = store.meetings_dir().join("fixture.json");
        store
            .write(
                &record,
                &serde_json::json!({"detail":{"id":"fixture","title":"Synthetic"}}),
            )
            .unwrap();
        let wave = store.meetings_dir().join("fixture.wav");
        fs::write(&wave, b"synthetic audio").unwrap();
        let asr = store.meetings_dir().join("fixture.asr");
        fs::create_dir_all(&asr).unwrap();
        fs::write(asr.join("missing.json"), b"synthetic missing sidecar").unwrap();
        fs::write(asr.join("existing.json"), b"synthetic original sidecar").unwrap();
        retain(&store, &[wave.clone(), asr.clone(), record.clone()]).unwrap();
        fs::create_dir_all(&asr).unwrap();
        fs::write(asr.join("existing.json"), b"synthetic current sidecar").unwrap();
        crate::meeting_sync::queue_delete(&store, "fixture").unwrap();
        let state = crate::meeting_sync::State::default();
        store
            .write(&store.root().join("meeting-sync-state.json"), &state)
            .unwrap();
        let before = fs::read(dir.join("meeting-sync-state.json")).unwrap();
        let key = list(&store).unwrap().remove(0).key;
        let mut count = 0;
        assert!(restore_with(&store, &key, |src, dest| {
            count += 1;
            if count == 2 {
                Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
            } else {
                fs::rename(src, dest)
            }
        })
        .is_err());
        assert_eq!(count, 2);
        assert_eq!(
            fs::read(dir.join("meeting-sync-state.json")).unwrap(),
            before
        );
        assert!(crate::meeting_sync::deletions_unlocked(&store)
            .unwrap()
            .contains("fixture"));
        assert!(!wave.exists());
        assert!(!record.exists());
        restore(&store, &key).unwrap();
        assert!(wave.exists());
        assert!(record.exists());
        assert_eq!(
            fs::read(asr.join("missing.json")).unwrap(),
            b"synthetic missing sidecar"
        );
        assert_eq!(
            fs::read(asr.join("existing.json")).unwrap(),
            b"synthetic current sidecar"
        );
        assert_eq!(
            list(&store).unwrap().len(),
            1,
            "colliding saved files are retained"
        );
        assert!(crate::meeting_sync::deletions_unlocked(&store)
            .unwrap()
            .is_empty());
        fs::remove_dir_all(dir).unwrap();
    }
    #[test]
    fn meeting_remote_trash_keeps_owned_files_for_thirty_days() {
        let dir = std::env::temp_dir().join(format!("sj-trash-test-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(dir.clone())).unwrap();
        let path = store.meetings_dir().join("synthetic.wav");
        fs::write(&path, b"synthetic audio bytes").unwrap();
        retain(&store, &[path.clone()]).unwrap();
        assert!(!path.exists());
        let root = store.root().join("meeting-trash");
        let saved = fs::read_dir(&root).unwrap().next().unwrap().unwrap().path();
        assert_eq!(
            fs::read(saved.join(path.strip_prefix(store.root()).unwrap())).unwrap(),
            b"synthetic audio bytes"
        );
        purge_at(&store, now() + RETENTION - 1).unwrap();
        assert!(saved.exists());
        purge_at(&store, now() + RETENTION + 1).unwrap();
        assert!(!saved.exists());
        let expired = root.join(format!("1-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&expired).unwrap();
        fs::write(expired.join("synthetic.wav"), b"synthetic audio bytes").unwrap();
        crate::meetings::Meetings::new(store.clone())
            .list()
            .unwrap();
        assert!(
            !expired.exists(),
            "local trash expires even while signed out"
        );
        fs::remove_dir_all(dir).unwrap();
    }
}
