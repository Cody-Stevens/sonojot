//! Adapter owned by Meetings and protected by its existing Store lock.
use super::*;
use crate::meeting_layers::{self as layers, Clean, Layers, Raw, SCHEMA};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct Local {
    pub layers: Layers,
    /// Crash-recovery projection, including device-local media paths. Never uploaded.
    pub record: Value,
    pub audio_override: Option<bool>,
    pub local_only: bool,
    #[serde(default)]
    pub json_hash: String,
}
fn json_hash(record: &Value) -> Result<String, String> {
    use sha2::{Digest, Sha256};
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(record).map_err(|_| "Cannot fingerprint meeting JSON.")?)
    ))
}
fn receipt(store: &Store, id: &str) -> Result<PathBuf, String> {
    Ok(path(store, id)?.with_extension("stamp"))
}
pub(crate) fn diagnostic(code: &str) {
    crate::recognition_diagnostics::event("meeting-store", code, std::process::id(), 0, 0, 0);
}
pub(crate) fn preserve_legacy(store: &Store, id: &str) -> Result<(), String> {
    let backup = path(store, id)?.with_extension("legacy");
    let original = store.meetings_dir().join(format!("{id}.json"));
    if original.exists() && !backup.exists() {
        store.check_owned(&original)?;
        store.check_owned(&backup)?;
        atomic_write(
            &backup,
            &fs::read(original).map_err(|_| "Cannot preserve the original meeting.")?,
        )?;
    }
    Ok(())
}
pub(crate) fn path(store: &Store, id: &str) -> Result<PathBuf, String> {
    validate_id(id)?;
    let dir = store.root().join("meeting-layers");
    store.check_owned(&dir)?;
    fs::create_dir_all(&dir).map_err(|_| "Cannot create meeting layers.")?;
    Ok(dir.join(format!("{id}.json")))
}
pub(crate) fn load(store: &Store, id: &str) -> Result<Option<Local>, String> {
    let path = path(store, id)?;
    if path.is_file() {
        let local: Local = store.read(&path)?;
        local.layers.validate()?;
        Ok(Some(local))
    } else {
        Ok(None)
    }
}
pub(crate) fn save(store: &Store, local: &Local) -> Result<(), String> {
    local.layers.validate()?;
    store.write(&path(store, &local.layers.head.id)?, local)?;
    store.write(&receipt(store, &local.layers.head.id)?, &local.json_hash)
}
/// Publish the JSON first. If the layer write fails, the next capture sees the
/// changed JSON and preserves it instead of recovering an older projection.
pub(crate) fn save_projected(store: &Store, local: &mut Local) -> Result<(), String> {
    project(&local.layers, &mut local.record)?;
    let record: MeetingRecord =
        serde_json::from_value(local.record.clone()).map_err(|_| "Invalid meeting projection.")?;
    validate_record(&record)?;
    local.record = serde_json::to_value(&record).map_err(|e| e.to_string())?;
    store.write(
        &store
            .meetings_dir()
            .join(format!("{}.json", record.detail.id)),
        &record,
    )?;
    local.json_hash = json_hash(&local.record)?;
    save(store, local)
}
fn device(store: &Store) -> Result<String, String> {
    let path = store.root().join("meeting-device.json");
    if path.exists() {
        return store.read(&path);
    }
    let id = new_id();
    store.write(&path, &id)?;
    Ok(id)
}
fn fields(record: &Value) -> BTreeMap<String, Value> {
    let mut fields = BTreeMap::new();
    if let Some(detail) = record["detail"].as_object() {
        for (key, value) in detail {
            if ![
                "id",
                "transcript",
                "notes",
                "cleanedTranscript",
                "audioPath",
                "hasAudio",
                "hasTranscript",
            ]
            .contains(&key.as_str())
            {
                fields.insert(key.clone(), value.clone());
            }
        }
    }
    for key in [
        "speakerNames",
        "titleUserEdited",
        "automaticTitlePending",
        "recognitionFile",
        "importSpeaker",
        "outagesRecorded",
    ] {
        fields.insert(format!("record.{key}"), record[key].clone());
    }
    fields
}
fn recognition(store: &Store, id: &str, local: &mut Local, record: &Value) -> Result<(), String> {
    let directory = store.meetings_dir().join(format!("{id}.asr"));
    if directory.is_dir() {
        store.check_owned(&directory)?;
        for (count, entry) in fs::read_dir(&directory)
            .map_err(|_| "Cannot read recognition history.")?
            .enumerate()
        {
            if count > 20_000 {
                return Err("Recognition history exceeds the supported limit.".into());
            }
            let path = entry
                .map_err(|_| "Cannot read recognition history.")?
                .path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            store.check_owned(&path)?;
            let name = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("Invalid recognition filename.")?;
            validate_id(name)?;
            if local.layers.raw.contains_key(name) {
                continue;
            }
            let value: Value = store.read(&path)?;
            let text = value["raw"].as_str().unwrap_or("");
            local.layers.raw.insert(
                name.into(),
                Raw {
                    schema: SCHEMA,
                    segments: layers::lines(text, name),
                    recognition: value,
                },
            );
        }
    }
    if !local.layers.raw.contains_key("legacy") {
        // Absence of original recognition stays explicit. Never relabel edited text as raw.
        let raw = record["rawTranscript"].as_str().unwrap_or("");
        local.layers.raw.insert("legacy".into(), Raw { schema: SCHEMA, segments: layers::lines(raw, "legacy"),
            recognition: serde_json::json!({"raw": record["rawTranscript"], "legacy": true, "model":record["detail"]["transcriptionModel"],
                "warnings":record["detail"]["recognitionWarnings"], "outages":record["detail"]["audioOutages"]}) });
    }
    if let Some(text) = record["rawTranscript"].as_str() {
        let first = local
            .layers
            .raw
            .iter()
            .find(|(_, raw)| {
                raw.recognition["firstRecognition"] == true && raw.recognition["raw"] == text
            })
            .map(|(id, _)| id.clone())
            .unwrap_or_else(|| format!("first-{}", new_id()));
        local
            .layers
            .raw
            .entry(first.clone())
            .or_insert_with(|| Raw {
                schema: SCHEMA,
                segments: layers::lines(text, &first),
                recognition: serde_json::json!({"raw":text,"firstRecognition":true}),
            });
    }
    let active = record["recognitionFile"]
        .as_str()
        .and_then(|s| s.strip_suffix(".json"))
        .filter(|id| local.layers.raw.contains_key(*id))
        .unwrap_or("legacy");
    if local.layers.head.raw != active {
        local.layers.head.raw = active.into();
        local.layers.head.clean = None;
        local.layers.head.snapshots.clear();
    }
    Ok(())
}
pub(crate) fn capture(store: &Store, record: &Value) -> Result<(), String> {
    let id = record["detail"]["id"]
        .as_str()
        .ok_or("Missing meeting id.")?;
    if busy(record["detail"]["status"].as_str().unwrap_or("")) {
        return Ok(());
    }
    let hash = json_hash(record)?;
    // This small receipt is a cache of the hash also stored in the layer file.
    // An unchanged library never parses or rewrites its full layer histories.
    if path(store, id)?.is_file()
        && store
            .read::<String>(&receipt(store, id)?)
            .is_ok_and(|saved| saved == hash)
    {
        return Ok(());
    }
    // Older JSON can omit serde-defaulted fields. Publish their typed defaults,
    // while the receipt remains a hash of the actual JSON read from disk.
    let normalized = serde_json::to_value(
        serde_json::from_value::<MeetingRecord>(record.clone())
            .map_err(|_| "Invalid meeting JSON.")?,
    )
    .map_err(|_| "Cannot normalize meeting JSON.")?;
    let record = &normalized;
    let existing = load(store, id)?;
    preserve_legacy(store, id)?;
    let mut local = existing.unwrap_or_else(|| Local {
        layers: Layers::empty(id),
        record: record.clone(),
        audio_override: None,
        local_only: false,
        json_hash: String::new(),
    });
    recognition(store, id, &mut local, record)?;
    let device = device(store)?;
    local.layers.edit(
        &device,
        "original",
        record["detail"]["transcript"].as_str().unwrap_or(""),
    )?;
    match record["detail"]["cleanedTranscript"].as_str() {
        Some(text) => {
            if let Some(view) = local.layers.head.clean.clone() {
                local.layers.edit(&device, &view, text)?;
            } else {
                let id = new_id();
                local.layers.clean.insert(
                    id.clone(),
                    Clean {
                        schema: SCHEMA,
                        base: local.layers.head.raw.clone(),
                        segments: layers::lines(text, &id),
                    },
                );
                local.layers.head.clean = Some(id);
            }
        }
        None => local.layers.head.clean = None,
    }
    local.layers.head.fields = fields(record);
    local.layers.head.fields.insert(
        "record.rawTranscriptRef".into(),
        serde_json::json!(local
            .layers
            .raw
            .iter()
            .find(|(_, raw)| raw.recognition["firstRecognition"] == true
                && raw.recognition["raw"] == record["rawTranscript"])
            .map(|(id, _)| id.as_str())
            .unwrap_or("legacy")),
    );
    let notes = record["detail"]["notes"].as_str().unwrap_or("");
    let entry = local.layers.notes.entry("main".into()).or_insert_with(|| serde_json::json!({"schema":SCHEMA,"preset":null,"instructions":null,"model":null,"createdAt":now(),"body":""}));
    if entry["body"].as_str() != Some(notes) {
        entry["body"] = serde_json::json!(notes);
        if local.record["detail"]["status"] == "processingNotes" {
            let settings = store.settings_unlocked()?;
            entry["preset"] =
                serde_json::to_value(settings.meeting_notes_preset).map_err(|e| e.to_string())?;
            entry["instructions"] = serde_json::json!(settings.meeting_notes_instructions);
        }
    }
    local.layers.head.notes = vec!["main".into()];
    local.record = record.clone();
    local.json_hash = hash;
    save(store, &local)
}

// Do not use Path::exists for destructive decisions: it hides permission and IO errors.
fn metadata_absent<T>(result: std::io::Result<T>) -> Result<bool, String> {
    match result {
        Ok(_) => Ok(false),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(true),
        Err(_) => Err("Cannot inspect meeting files. No deletion was inferred.".into()),
    }
}
pub(crate) fn absent(path: &std::path::Path) -> Result<bool, String> {
    metadata_absent(fs::symlink_metadata(path))
}
pub(crate) fn readable_meetings(store: &Store) -> Result<(), String> {
    let dir = store.meetings_dir();
    store.check_owned(&dir)?;
    for (index, entry) in fs::read_dir(dir)
        .map_err(|_| "Cannot list meetings. No deletion was inferred.")?
        .enumerate()
    {
        if index > 100_000 {
            return Err("Meeting directory exceeds its limit.".into());
        }
        entry.map_err(|_| "Cannot list meetings. No deletion was inferred.")?;
    }
    Ok(())
}

impl Meetings {
    // Absence alone never authorizes deletion or moves audio. The library-wide
    // reconciliation owns that decision; reads only prevent resurrection.
    pub(super) fn reconcile_absence(&self, id: &str) -> Result<bool, String> {
        if !absent(&self.path(id)?)? {
            return Ok(false);
        }
        Ok(crate::meeting_sync::tracked_ids(&self.store)?.contains(id)
            || !absent(&path(&self.store, id)?)?
            || crate::meeting_sync::deletions_unlocked(&self.store)?.contains(id))
    }
    pub(crate) fn sync_delete_ready(&self, id: &str) -> Result<bool, String> {
        let _guard = self.store.lock()?;
        readable_meetings(&self.store)?;
        if absent(&self.path(id)?)? {
            return Ok(true);
        }
        crate::meeting_sync::cancel_delete(&self.store, id)?;
        self.read_record(id, true)?;
        Ok(false)
    }
    pub(crate) fn sync_delete_published(&self, id: &str) -> Result<bool, String> {
        let _guard = self.store.lock()?;
        readable_meetings(&self.store)?;
        if absent(&self.path(id)?)? {
            self.trash_owned(id)?;
            Ok(true)
        } else {
            self.read_record(id, true)?;
            if let Ok(Some(mut layer)) = load(&self.store, id) {
                layer.local_only = true;
                save(&self.store, &layer)?;
            }
            crate::meeting_sync::keep_restored(&self.store, id)?;
            Ok(false)
        }
    }
    pub(crate) fn sync_capture(&self, id: &str) -> Result<(), String> {
        let _guard = self.store.lock()?;
        self.read_record(id, true)?;
        Ok(())
    }
    pub(crate) fn sync_delete_remote(&self, id: &str) -> Result<(), String> {
        if self.path(id)?.exists() {
            self.delete_inner(id, true)?;
        }
        Ok(())
    }
    pub(crate) fn sync_optional(&self, id: &str) -> Result<Option<Local>, String> {
        let _guard = self.store.lock()?;
        if self.reconcile_absence(id)? {
            return Err("Meeting was deleted locally. Sync will publish its deletion.".into());
        }
        if self.path(id)?.exists() {
            self.read(id)?;
        }
        load(&self.store, id)
    }
    pub(crate) fn sync_changed(&self) {
        self.revision.fetch_add(1, Ordering::Release);
    }
    pub(crate) fn sync_collect(&self) -> Result<Vec<String>, String> {
        self.reconcile_missing_json()?;
        let ids: Vec<_> = self
            .list()?
            .into_iter()
            .filter(|row| !busy(&row.status))
            .map(|r| r.id.clone())
            .collect();
        for id in &ids {
            let _guard = self.store.lock()?;
            // read reconciles changed JSON only. One bad meeting cannot stop the library.
            if self.read_record(id, true).is_err() {
                diagnostic("collect-record-unavailable");
            }
        }
        Ok(ids)
    }
    fn reconcile_missing_json(&self) -> Result<(), String> {
        let _guard = self.store.lock()?;
        let tracked = crate::meeting_sync::tracked_ids(&self.store)?;
        // Fully enumerate before changing any queue. A missing/unreadable parent
        // must never look like the deliberate removal of every child.
        if let Err(error) = readable_meetings(&self.store) {
            crate::meeting_sync::missing_notice(&self.store, &tracked)?;
            return Err(error);
        }
        let mut missing = BTreeSet::new();
        for id in &tracked {
            if absent(&self.path(id)?)? {
                missing.insert(id.clone());
            }
        }
        crate::meeting_sync::reconcile_missing(&self.store, &tracked, &missing)
    }
    pub(crate) fn sync_local(&self, id: &str) -> Result<Local, String> {
        let _guard = self.store.lock()?;
        self.read(id)?;
        load(&self.store, id)?.ok_or_else(|| "Meeting layers are not initialized.".into())
    }
    pub(crate) fn sync_commit(
        &self,
        id: &str,
        received: &Layers,
        before: &Layers,
        conflicts: &mut BTreeMap<String, Value>,
    ) -> Result<(), String> {
        let _guard = self.store.lock()?;
        if self.reconcile_absence(id)? {
            return Err("Meeting was deleted locally. Sync will publish its deletion.".into());
        }
        if self.path(id)?.exists() {
            self.read(id)?;
        }
        let mut local = load(&self.store, id)?.unwrap_or_else(|| Local {
            layers: received.clone(),
            record: Value::Null,
            audio_override: None,
            local_only: false,
            json_hash: String::new(),
        });
        if local.local_only {
            return Err("This meeting is kept on this computer.".into());
        }
        let record_path = self.path(id)?;
        if record_path.exists() && busy(&self.read(id)?.detail.status) {
            return Err("A meeting is busy. Sync will retry after it finishes.".into());
        }
        local.layers.merge_immutable(received)?;
        local.layers.head = serde_json::from_value(layers::merge_fields(
            &serde_json::to_value(&before.head).unwrap(),
            &serde_json::to_value(&local.layers.head).unwrap(),
            &serde_json::to_value(&received.head).unwrap(),
            "head",
            conflicts,
        ))
        .map_err(|_| "Invalid merged meeting head.")?;
        let notes = layers::merge_fields(
            &serde_json::to_value(&before.notes).unwrap(),
            &serde_json::to_value(&local.layers.notes).unwrap(),
            &serde_json::to_value(&received.notes).unwrap(),
            "notes",
            conflicts,
        );
        local.layers.notes =
            serde_json::from_value(notes).map_err(|_| "Invalid merged meeting notes.")?;
        local.layers.validate()?;
        let mut value = local.record.clone();
        if !value.is_object() {
            value = serde_json::json!({"schemaVersion":1,"detail":{},"rawTranscript":null,"titleUserEdited":true,"automaticTitlePending":false});
        }
        for (key, val) in &local.layers.head.fields {
            if let Some(key) = key.strip_prefix("record.") {
                value[key] = val.clone();
            } else {
                value["detail"][key] = val.clone();
            }
        }
        value["detail"]["id"] = serde_json::json!(id);
        value["detail"]["audioPath"] = Value::Null;
        value["detail"]["hasAudio"] = serde_json::json!(false);
        let first = local
            .layers
            .head
            .fields
            .get("record.rawTranscriptRef")
            .and_then(Value::as_str)
            .unwrap_or("legacy");
        value["rawTranscript"] = local
            .layers
            .raw
            .get(first)
            .map(|r| r.recognition["raw"].clone())
            .unwrap_or(Value::Null);
        project(&local.layers, &mut value)?;
        let record: MeetingRecord = serde_json::from_value(value.clone())
            .map_err(|_| "Synced meeting has an unsupported record format.")?;
        validate_record(&record)?;
        local.record = value;
        save_projected(&self.store, &mut local)?;
        // Restore recognition sidecars for timed transcript and speaker controls.
        let dir = self.store.meetings_dir().join(format!("{id}.asr"));
        self.store.check_owned(&dir)?;
        fs::create_dir_all(&dir).map_err(|_| "Cannot restore recognition history.")?;
        for (raw_id, raw) in &local.layers.raw {
            validate_id(raw_id)?;
            if raw_id == "legacy" || raw.recognition["firstRecognition"] == true {
                continue;
            }
            let dest = dir.join(format!("{raw_id}.json"));
            if !dest.exists() {
                self.store.write(&dest, &raw.recognition)?;
            }
        }
        self.revision.fetch_add(1, Ordering::Release);
        Ok(())
    }
}
pub(crate) fn project(layers: &Layers, record: &mut Value) -> Result<(), String> {
    record["detail"]["transcript"] = serde_json::json!(layers.text("original")?);
    record["detail"]["hasTranscript"] =
        serde_json::json!(!layers.text("original")?.trim().is_empty());
    record["detail"]["cleanedTranscript"] = match &layers.head.clean {
        Some(id) => serde_json::json!(layers.text(id)?),
        None => Value::Null,
    };
    record["detail"]["notes"] = layers
        .notes
        .get("main")
        .map(|n| n["body"].clone())
        .unwrap_or_else(|| serde_json::json!(""));
    Ok(())
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    fn setup() -> (PathBuf, Store, Meetings, MeetingRecord) {
        let root = std::env::temp_dir().join(format!("sj-meeting-compat-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let meetings = Meetings::new(store.clone());
        let record = serde_json::from_value(serde_json::json!({"schemaVersion":1,
            "detail":{"id":"fixture","title":"Synthetic meeting","createdAt":"2026-01-01T12:00:00Z",
                "durationSeconds":30.0,"status":"transcribed","hasAudio":false,"hasTranscript":true,
                "transcript":"Synthetic original.","notes":"Synthetic notes.","source":"microphone","audioPath":null,"error":null},
            "rawTranscript":"Synthetic recognition."})).unwrap();
        (root, store, meetings, record)
    }
    #[test]
    fn cross_version_json_edits_survive_read_and_remote_commit() {
        let (root, store, meetings, mut record) = setup();
        meetings.write_record(&record, true).unwrap();
        let before = load(&store, "fixture").unwrap().unwrap().layers;
        record.detail.transcript = "Synthetic edit made with sync off.".into();
        record.detail.notes = "Synthetic older-build notes.".into();
        meetings.write_record(&record, false).unwrap();
        assert_eq!(
            meetings
                .read_record("fixture", true)
                .unwrap()
                .detail
                .transcript,
            record.detail.transcript
        );
        assert_eq!(
            load(&store, "fixture")
                .unwrap()
                .unwrap()
                .layers
                .text("original")
                .unwrap(),
            record.detail.transcript
        );
        let mut remote = before.clone();
        remote
            .head
            .fields
            .insert("title".into(), serde_json::json!("Synthetic remote title"));
        meetings
            .sync_commit("fixture", &remote, &before, &mut BTreeMap::new())
            .unwrap();
        let saved: MeetingRecord = store.read(&meetings.path("fixture").unwrap()).unwrap();
        assert_eq!(saved.detail.transcript, record.detail.transcript);
        assert_eq!(saved.detail.notes, record.detail.notes);
        assert_eq!(saved.detail.title, "Synthetic remote title");
        let layer = path(&store, "fixture").unwrap();
        let modified = fs::metadata(&layer).unwrap().modified().unwrap();
        meetings.sync_collect().unwrap();
        meetings.sync_collect().unwrap();
        assert_eq!(
            fs::metadata(layer).unwrap().modified().unwrap(),
            modified,
            "unchanged captures do not rewrite history"
        );
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn missing_json_requires_confirmation_without_trashing_audio() {
        let (root, store, meetings, record) = setup();
        meetings.write_record(&record, true).unwrap();
        // Ensure an original backup exists, as after upgrading an existing library.
        preserve_legacy(&store, "fixture").unwrap();
        let state = crate::meeting_sync::State {
            meetings: [(
                "fixture".into(),
                crate::meeting_sync::Track {
                    key: "synthetic-key".into(),
                    status: "synced".into(),
                    head: Some(load(&store, "fixture").unwrap().unwrap().layers.head),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        store
            .write(&store.root().join("meeting-sync-state.json"), &state)
            .unwrap();
        let before = load(&store, "fixture").unwrap().unwrap().layers;
        fs::remove_file(meetings.path("fixture").unwrap()).unwrap();
        assert!(
            meetings
                .sync_commit("fixture", &before, &before, &mut BTreeMap::new())
                .is_err(),
            "in-flight download must not resurrect a local deletion"
        );
        assert!(meetings.sync_collect().unwrap().is_empty());
        assert!(path(&store, "fixture").unwrap().exists());
        assert!(path(&store, "fixture")
            .unwrap()
            .with_extension("legacy")
            .exists());
        let pending: Vec<String> = store
            .read(&store.root().join("meeting-sync-deletions.json"))
            .unwrap();
        assert!(pending.is_empty());
        let saved: crate::meeting_sync::State = store
            .read(&store.root().join("meeting-sync-state.json"))
            .unwrap();
        assert!(saved.missing.contains("fixture"));
        assert!(!root.join("meeting-trash").exists());
        assert!(meetings.list().unwrap().is_empty());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn restored_trash_is_local_only_and_never_overwrites_a_live_copy() {
        let (root, store, meetings, record) = setup();
        meetings.write_record(&record, true).unwrap();
        let wave = meetings.recording_paths("fixture").unwrap().microphone;
        fs::write(&wave, b"synthetic wav bytes").unwrap();
        meetings.trash_owned("fixture").unwrap();
        let entries = crate::meeting_trash::list(&store).unwrap();
        assert_eq!(entries.len(), 1);
        let mut current = record.clone();
        current.detail.transcript = "Synthetic current JSON kept.".into();
        meetings.write_record(&current, false).unwrap();
        let live_json = fs::read(meetings.path("fixture").unwrap()).unwrap();
        crate::meeting_trash::restore(&store, &entries[0].key).unwrap();
        assert_eq!(
            fs::read(meetings.path("fixture").unwrap()).unwrap(),
            live_json
        );
        assert_eq!(crate::meeting_trash::list(&store).unwrap().len(), 1);
        crate::meeting_trash::delete_now(&store, &entries[0].key).unwrap();
        assert_eq!(fs::read(wave).unwrap(), b"synthetic wav bytes");
        assert!(load(&store, "fixture").unwrap().unwrap().local_only);
        assert!(meetings.sync_delete_remote("fixture").is_err());
        let restored = load(&store, "fixture").unwrap().unwrap().layers;
        assert!(meetings
            .sync_commit("fixture", &restored, &restored, &mut BTreeMap::new())
            .is_err());
        assert!(crate::meeting_trash::list(&store).unwrap().is_empty());
        assert_eq!(
            meetings
                .read_record("fixture", true)
                .unwrap()
                .detail
                .transcript,
            current.detail.transcript
        );
        meetings.trash_owned("fixture").unwrap();
        let entry = crate::meeting_trash::list(&store).unwrap().remove(0);
        // Existing pre-index trash also appears and can be restored.
        let dir = store.root().join("meeting-trash").join(&entry.key);
        fs::remove_file(dir.join("entry.json")).unwrap();
        fs::remove_file(dir.join("record.json")).unwrap();
        assert_eq!(crate::meeting_trash::list(&store).unwrap().len(), 1);
        crate::meeting_trash::restore(&store, &entry.key).unwrap();
        meetings.trash_owned("fixture").unwrap();
        let entry = crate::meeting_trash::list(&store).unwrap().remove(0);
        crate::meeting_trash::delete_now(&store, &entry.key).unwrap();
        assert!(crate::meeting_trash::list(&store).unwrap().is_empty());
        assert!(crate::meeting_trash::delete_now(&store, "../fixture").is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn metadata_permission_error_is_not_absence() {
        assert!(metadata_absent::<()>(Err(std::io::Error::from(
            std::io::ErrorKind::PermissionDenied
        )))
        .is_err());
        assert!(
            metadata_absent::<()>(Err(std::io::Error::from(std::io::ErrorKind::NotFound))).unwrap()
        );
        assert!(!metadata_absent(Ok(())).unwrap());
    }
    #[test]
    fn renamed_folder_stops_reconciliation_and_records_notice() {
        let (root, store, meetings, record) = setup();
        meetings.write_record(&record, true).unwrap();
        let mut m = crate::meeting_sync::State::default();
        m.meetings.insert(
            "fixture".into(),
            crate::meeting_sync::Track {
                head: Some(load(&store, "fixture").unwrap().unwrap().layers.head),
                ..Default::default()
            },
        );
        store
            .write(&store.root().join("meeting-sync-state.json"), &m)
            .unwrap();
        fs::rename(store.meetings_dir(), root.join("renamed-meetings")).unwrap();
        assert!(meetings.sync_collect().is_err());
        assert!(crate::meeting_sync::deletions_unlocked(&store)
            .unwrap()
            .is_empty());
        let m: crate::meeting_sync::State = store
            .read(&store.root().join("meeting-sync-state.json"))
            .unwrap();
        assert!(m.missing.contains("fixture"));
        assert!(path(&store, "fixture").unwrap().exists());
        assert!(!root.join("meeting-trash").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn reappeared_json_cancels_tombstone_and_recaptures_before_publish() {
        let (root, store, meetings, mut record) = setup();
        meetings.write_record(&record, true).unwrap();
        let wave = meetings.recording_paths("fixture").unwrap().microphone;
        fs::write(&wave, b"synthetic audio").unwrap();
        fs::remove_file(meetings.path("fixture").unwrap()).unwrap();
        crate::meeting_sync::queue_delete(&store, "fixture").unwrap();
        assert!(meetings.sync_delete_ready("fixture").unwrap());
        assert!(wave.exists());
        record.detail.transcript = "Synthetic reappeared edits.".into();
        meetings.write_record(&record, false).unwrap();
        assert!(!meetings.sync_delete_ready("fixture").unwrap());
        assert!(crate::meeting_sync::deletions_unlocked(&store)
            .unwrap()
            .is_empty());
        assert_eq!(
            load(&store, "fixture")
                .unwrap()
                .unwrap()
                .layers
                .text("original")
                .unwrap(),
            record.detail.transcript
        );
        assert!(wave.exists());
        fs::remove_file(meetings.path("fixture").unwrap()).unwrap();
        assert!(meetings.sync_delete_published("fixture").unwrap());
        assert!(!wave.exists());
        assert_eq!(crate::meeting_trash::list(&store).unwrap().len(), 1);
        meetings.write_record(&record, false).unwrap();
        fs::write(&wave, b"synthetic reappeared audio").unwrap();
        assert!(!meetings.sync_delete_published("fixture").unwrap());
        assert!(load(&store, "fixture").unwrap().unwrap().local_only);
        assert_eq!(fs::read(&wave).unwrap(), b"synthetic reappeared audio");
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn failed_first_pull_is_not_an_older_build_deletion() {
        let (root, store, meetings, _) = setup();
        let state = crate::meeting_sync::State {
            meetings: [(
                "fixture".into(),
                crate::meeting_sync::Track {
                    key: "synthetic".into(),
                    ..Default::default()
                },
            )]
            .into(),
            ..Default::default()
        };
        store
            .write(&store.root().join("meeting-sync-state.json"), &state)
            .unwrap();
        meetings.sync_collect().unwrap();
        assert!(!store.root().join("meeting-sync-deletions.json").exists());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn newer_layers_and_failed_trash_purge_do_not_hide_json() {
        let (root, store, meetings, record) = setup();
        meetings.write_record(&record, true).unwrap();
        let layer = path(&store, "fixture").unwrap();
        let mut value: Value = store.read(&layer).unwrap();
        value["futureLayerField"] = serde_json::json!(true);
        store.write(&layer, &value).unwrap();
        fs::remove_file(receipt(&store, "fixture").unwrap()).unwrap();
        let bytes = fs::read(&layer).unwrap();
        fs::write(root.join("meeting-trash"), b"synthetic blocked directory").unwrap();
        assert_eq!(
            meetings
                .read_record("fixture", true)
                .unwrap()
                .detail
                .transcript,
            record.detail.transcript
        );
        assert_eq!(meetings.list().unwrap().len(), 1);
        assert_eq!(
            fs::read(layer).unwrap(),
            bytes,
            "fallback preserves unknown layer fields"
        );
        fs::remove_dir_all(root).unwrap();
    }
}
