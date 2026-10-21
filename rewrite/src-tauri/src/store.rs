use crate::domain::{
    CleanupMode, DictationRecord, HotkeyMode, LearnedEntry, LearnedKind, MigrationReport,
    RecognitionCompleteness, SavedCorrection, Settings, VocabularyEntry,
};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BinaryHeap, HashSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[path = "learning.rs"]
pub mod learning;

pub const MAX_TEXT_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_JSON_BYTES: u64 = 32 * 1024 * 1024;
pub const MAX_VOCABULARY_ENTRIES: usize = 2_000;

#[derive(Clone, Debug)]
pub struct Store {
    root: Arc<PathBuf>,
    gate: Arc<Mutex<()>>,
    privacy_generation: Arc<AtomicU64>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SettingsFile {
    schema_version: u32,
    settings: Settings,
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VocabularyFile {
    schema_version: u32,
    entries: Vec<VocabularyEntry>,
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LearningFile {
    schema_version: u32,
    entries: Vec<LearnedEntry>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct DictationHeader {
    id: String,
    created_at: String,
}

impl Store {
    /// Opening a custom directory never reads legacy data implicitly.
    pub fn open(root: Option<PathBuf>) -> Result<Self, String> {
        let root = root
            .or_else(|| crate::environment::var_os("DATA_DIR").map(PathBuf::from))
            .map(Ok)
            .unwrap_or_else(default_root)?;
        if root.file_name().is_some_and(|name| {
            name.to_string_lossy()
                .eq_ignore_ascii_case("EavesdropperData")
        }) {
            return Err(
                "SonoJot must use a separate data directory from the legacy C# app.".into(),
            );
        }
        fs::create_dir_all(&root)
            .map_err(|e| format!("Cannot create the app data directory: {e}"))?;
        let root = fs::canonicalize(root)
            .map_err(|e| format!("Cannot open the app data directory: {e}"))?;
        let store = Self {
            root: Arc::new(root),
            gate: Arc::new(Mutex::new(())),
            privacy_generation: Arc::new(AtomicU64::new(0)),
        };
        for folder in ["meetings", "dictations", "models", "logs", "migrations"] {
            let path = store.root.join(folder);
            fs::create_dir_all(&path)
                .map_err(|e| format!("Cannot create {folder} directory: {e}"))?;
            store.check_owned(&path)?;
        }
        // Retire the unassociated content cache; Copy-last is session-only.
        store.clear_dictation_caches_unlocked()?;
        store.clear_abandoned_dictation_writes()?;
        Ok(store)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn logs_dir(&self) -> PathBuf {
        self.root.join("logs")
    }
    pub fn models_dir(&self) -> PathBuf {
        if let Some(path) = crate::environment::var_os("MODELS") {
            return PathBuf::from(path);
        }
        let mut candidates = vec![self.root.join("models/rewrite")];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(parent) = exe.parent() {
                candidates.push(parent.join("models/rewrite"));
                candidates.push(parent.join("resources/models/rewrite"));
            }
        }
        if let Ok(cwd) = std::env::current_dir() {
            for ancestor in cwd.ancestors().take(5) {
                candidates.push(ancestor.join("models/rewrite"));
            }
        }
        candidates
            .into_iter()
            .find(|p| p.is_dir())
            .unwrap_or_else(|| self.root.join("models/rewrite"))
    }
    pub(crate) fn lock(&self) -> Result<MutexGuard<'_, ()>, String> {
        self.gate.lock().map_err(|_| {
            "The data store lock was interrupted. Restart the app before saving.".into()
        })
    }
    pub(crate) fn meetings_dir(&self) -> PathBuf {
        self.root.join("meetings")
    }
    pub(crate) fn check_owned(&self, path: &Path) -> Result<(), String> {
        if !path.starts_with(self.root()) {
            return Err("The requested path is outside the app data directory.".into());
        }
        let ancestor = if path.exists() {
            path
        } else {
            path.parent().ok_or("The data path has no parent.")?
        };
        let canonical = fs::canonicalize(ancestor)
            .map_err(|e| format!("Cannot resolve the app data path: {e}"))?;
        if !canonical.starts_with(self.root()) {
            return Err("The app data path points outside its owned directory.".into());
        }
        if let Ok(metadata) = fs::symlink_metadata(path) {
            if metadata.file_type().is_symlink() {
                return Err("Linked files cannot be modified through the app data store.".into());
            }
        }
        Ok(())
    }
    pub(crate) fn read<T: DeserializeOwned>(&self, path: &Path) -> Result<T, String> {
        self.check_owned(path)?;
        read_json(path)
    }
    pub(crate) fn write<T: Serialize>(&self, path: &Path, value: &T) -> Result<(), String> {
        self.check_owned(path)?;
        let data = serde_json::to_vec_pretty(value)
            .map_err(|_| "Could not serialize app data.".to_string())?;
        if data.len() as u64 > MAX_JSON_BYTES {
            return Err("The data file exceeds the 32 MiB limit.".into());
        }
        atomic_write(path, &data)
    }
    pub fn load_settings(&self) -> Result<Settings, String> {
        let _guard = self.lock()?;
        self.settings_unlocked()
    }
    pub(crate) fn settings_unlocked(&self) -> Result<Settings, String> {
        // Single-critical-section helper for per-job snapshots
        // (`ai::snapshot_notes_config`): callers hold `self.lock()` across
        // this plus the AI-settings read so one job never tears backend vs
        // model vs the text-acceleration preference across two reads.
        let path = self.root.join("settings.json");
        if !path.exists() {
            let mut settings = Settings::default();
            if self.root.join("setup-seen.json").is_file() {
                settings.setup_profile = Some(crate::domain::SetupProfile::Custom);
            }
            return Ok(settings);
        }
        // An explicit null belongs to an unfinished new install. Only an absent
        // key is a legacy settings file; migration must never choose its models.
        let mut raw: Value = self.read(&path)?;
        let has_profile = raw.get("settings").and_then(|s| s.get("setupProfile")).is_some();
        let mut value: SettingsFile = serde_json::from_value(raw.clone())
            .map_err(|e| format!("Cannot read settings: {e}"))?;
        if value.schema_version != 1 {
            return Err("This settings file uses an unsupported schema version.".into());
        }
        if !has_profile {
            value.settings.setup_profile = Some(crate::domain::SetupProfile::Custom);
        }
        value.settings.validate()?;
        if !value.settings.meeting_source_migrated {
            if matches!(value.settings.meeting_source.as_str(), "microphone" | "system") {
                value.settings.meeting_source = "both".into();
            }
            value.settings.meeting_source_migrated = true;
            // Preserve unrelated and unknown fields, including an absent setupProfile.
            raw["settings"]["meetingSource"] = value.settings.meeting_source.clone().into();
            raw["settings"]["meetingSourceMigrated"] = true.into();
            self.write(&path, &raw)?;
        }
        Ok(value.settings)
    }
    pub fn save_settings(&self, settings: &Settings) -> Result<(), String> {
        settings.validate()?;
        let _guard = self.lock()?;
        let old = self.settings_unlocked()?;
        if old.do_not_save_dictations != settings.do_not_save_dictations
            || old.learn_from_corrections != settings.learn_from_corrections
        {
            // Invalidate before I/O: a failed save must never authorize stale work.
            self.invalidate_privacy_unlocked();
        }
        self.clear_dictation_caches_unlocked()?;
        let mut settings = settings.clone();
        // A save is an explicit choice, including from clients predating the marker.
        settings.meeting_source_migrated = true;
        self.write(
            &self.root.join("settings.json"),
            &SettingsFile {
                schema_version: 1,
                settings,
            },
        )
    }
    pub fn get_vocabulary(&self) -> Result<Vec<VocabularyEntry>, String> {
        let _guard = self.lock()?;
        self.vocabulary_unlocked()
    }
    fn vocabulary_unlocked(&self) -> Result<Vec<VocabularyEntry>, String> {
        let path = self.root.join("vocabulary.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let value: VocabularyFile = self.read(&path)?;
        if value.schema_version != 1 {
            return Err("This vocabulary file uses an unsupported schema version.".into());
        }
        validate_vocabulary(&value.entries)?;
        Ok(value.entries)
    }
    pub fn save_vocabulary(
        &self,
        entries: &[VocabularyEntry],
    ) -> Result<Vec<VocabularyEntry>, String> {
        let entries = entries
            .iter()
            .cloned()
            .map(|mut e| {
                if e.id.is_empty() {
                    e.id = new_id();
                }
                e.heard = e.heard.trim().into();
                e.written = e.written.trim().into();
                e.scope = e
                    .scope
                    .map(|scope| scope.trim().into())
                    .filter(|scope: &String| !scope.is_empty());
                e
            })
            .collect::<Vec<_>>();
        validate_vocabulary(&entries)?;
        let _guard = self.lock()?;
        self.write(
            &self.root.join("vocabulary.json"),
            &VocabularyFile {
                schema_version: 1,
                entries: entries.clone(),
            },
        )?;
        Ok(entries)
    }
    /// A dictation is append-only; raw ASR and the finished result occupy distinct fields.
    pub fn record_dictation(
        &self,
        raw: &str,
        published: &str,
        recognition_completeness: RecognitionCompleteness,
        recognition_warnings: &[String],
    ) -> Result<DictationRecord, String> {
        self.commit_dictation(
            self.privacy_generation(),
            raw,
            published,
            recognition_completeness,
            recognition_warnings,
        )?
        .ok_or_else(|| "This dictation is temporary and was not saved.".into())
    }

    pub fn privacy_generation(&self) -> u64 {
        self.privacy_generation.load(Ordering::Acquire)
    }

    fn invalidate_privacy_unlocked(&self) {
        self.privacy_generation.fetch_add(1, Ordering::AcqRel);
    }

    /// Capture `generation` before starting work. Recheck it and retention under
    /// the same lock as settings changes/deletion, immediately before the write.
    pub fn commit_dictation(
        &self,
        generation: u64,
        raw: &str,
        published: &str,
        recognition_completeness: RecognitionCompleteness,
        recognition_warnings: &[String],
    ) -> Result<Option<DictationRecord>, String> {
        self.commit_dictation_with_metadata(
            generation, raw, published, recognition_completeness, recognition_warnings,
            None, None, None, None,
        )
    }

    pub fn commit_dictation_with_metadata(
        &self,
        generation: u64,
        raw: &str,
        published: &str,
        recognition_completeness: RecognitionCompleteness,
        recognition_warnings: &[String],
        pre_model_text: Option<&str>,
        publication_stage: Option<&str>,
        guard_reason: Option<&str>,
        target_app: Option<&str>,
    ) -> Result<Option<DictationRecord>, String> {
        self.commit_dictation_with_status(
            generation, raw, published, recognition_completeness, recognition_warnings,
            pre_model_text, publication_stage, guard_reason, target_app, None,
        )
    }

    pub fn commit_dictation_with_status(
        &self,
        generation: u64,
        raw: &str,
        published: &str,
        recognition_completeness: RecognitionCompleteness,
        recognition_warnings: &[String],
        pre_model_text: Option<&str>,
        publication_stage: Option<&str>,
        guard_reason: Option<&str>,
        target_app: Option<&str>,
        cleanup_status: Option<&str>,
    ) -> Result<Option<DictationRecord>, String> {
        validate_text(raw, "Raw dictation")?;
        validate_text(published, "Published dictation")?;
        if let Some(text) = pre_model_text {
            validate_text(text, "Original cleanup text")?;
        }
        let _guard = self.lock()?;
        if generation != self.privacy_generation()
            || self.settings_unlocked()?.do_not_save_dictations
        {
            return Ok(None);
        }
        let value = DictationRecord {
            id: new_id(),
            created_at: now(),
            raw: raw.into(),
            published: published.into(),
            target_app: target_app.map(str::to_owned),
            pre_model_text: pre_model_text.map(str::to_owned),
            publication_stage: publication_stage.map(str::to_owned),
            guard_reason: guard_reason.map(str::to_owned),
            cleanup_status: cleanup_status.map(str::to_owned),
            recognition_completeness,
            recognition_warnings: recognition_warnings.to_vec(),
        };
        self.write(
            &self
                .root
                .join("dictations")
                .join(format!("{}.json", value.id)),
            &value,
        )?;
        Ok(Some(value))
    }
    pub fn get_dictation(&self, id: &str) -> Result<DictationRecord, String> {
        validate_id(id)?;
        let _guard = self.lock()?;
        self.dictation_unlocked(id)
    }

    fn dictation_unlocked(&self, id: &str) -> Result<DictationRecord, String> {
        let record: DictationRecord =
            self.read(&self.root.join("dictations").join(format!("{id}.json")))?;
        if record.id != id {
            return Err("The dictation record has a mismatched identifier.".into());
        }
        validate_text(&record.raw, "Raw dictation")?;
        validate_text(&record.published, "Published dictation")?;
        Ok(record)
    }

    /// Returns at most 100 immutable records, newest first. Text is never truncated.
    pub fn list_dictations(&self, limit: usize) -> Result<Vec<DictationRecord>, String> {
        let limit = limit.min(100);
        if limit == 0 {
            return Ok(Vec::new());
        }
        let _guard = self.lock()?;
        // Keep only the requested headers in memory while scanning the append-only archive.
        let mut newest = BinaryHeap::new();
        let mut count = 0;
        for entry in fs::read_dir(self.root.join("dictations"))
            .map_err(|e| format!("Cannot list dictation history: {e}"))?
        {
            let path = entry
                .map_err(|e| format!("Cannot inspect dictation history: {e}"))?
                .path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            count += 1;
            if count > 20_000 {
                return Err("Dictation history exceeds the 20,000-record scan limit.".into());
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("A dictation filename is invalid.")?;
            validate_id(id)?;
            let header: DictationHeader = self.read(&path)?;
            if header.id != id {
                return Err("A dictation record has a mismatched identifier.".into());
            }
            let timestamp = OffsetDateTime::parse(&header.created_at, &Rfc3339)
                .map_err(|_| "A dictation record has an invalid creation date.")?
                .unix_timestamp_nanos();
            newest.push(std::cmp::Reverse((timestamp, header.id)));
            if newest.len() > limit {
                newest.pop();
            }
        }
        let mut headers = newest.into_vec();
        headers.sort();
        headers
            .into_iter()
            .map(|std::cmp::Reverse((_, id))| self.dictation_unlocked(&id))
            .collect()
    }
    fn dictation_paths_unlocked(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let directory = self.root.join("dictations");
        self.check_owned(&directory)?;
        let mut paths = Vec::new();
        for entry in
            fs::read_dir(directory).map_err(|e| format!("Cannot list dictation history: {e}"))?
        {
            let path = entry
                .map_err(|e| format!("Cannot inspect dictation history: {e}"))?
                .path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("A dictation filename is invalid.")?;
            validate_id(id)?;
            self.check_owned(&path)?;
            let header: DictationHeader = self.read(&path)?;
            if header.id != id {
                return Err("A dictation record has a mismatched identifier.".into());
            }
            paths.push((id.to_owned(), path));
        }
        Ok(paths)
    }

    pub fn count_dictations(&self) -> Result<usize, String> {
        let _guard = self.lock()?;
        Ok(self.dictation_paths_unlocked()?.len())
    }

    fn remove_owned_unlocked(&self, path: &Path) -> Result<(), String> {
        self.check_owned(path)?;
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("Cannot remove app-owned dictation data: {e}")),
        }
    }

    fn clear_dictation_caches_unlocked(&self) -> Result<(), String> {
        self.remove_owned_unlocked(&self.root.join("last-published.json"))?;
        // Previous delivery logs could contain recognizer errors or window titles.
        // They have no reliable source association, so discard the entire log.
        self.remove_owned_unlocked(&self.logs_dir().join("delivery.jsonl"))
    }

    // Only called at open, before this Store is shared with workers. Atomic
    // writes interrupted by process exit are never a second copy of history.
    fn clear_abandoned_dictation_writes(&self) -> Result<(), String> {
        for directory in [self.root().to_path_buf(), self.root.join("dictations")] {
            for entry in fs::read_dir(&directory)
                .map_err(|e| format!("Cannot inspect abandoned writes: {e}"))?
            {
                let path = entry
                    .map_err(|e| format!("Cannot inspect abandoned writes: {e}"))?
                    .path();
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                let Some(id) = name
                    .strip_prefix(".sonojot-")
                    .or_else(|| name.strip_prefix(".eavesdropper-"))
                    .and_then(|n| n.strip_suffix(".tmp"))
                else {
                    continue;
                };
                if id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()) {
                    self.remove_owned_unlocked(&path)?;
                }
            }
        }
        Ok(())
    }

    pub fn delete_dictation(&self, id: &str, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Confirm deletion of this dictation first.".into());
        }
        validate_id(id)?;
        let _guard = self.lock()?;
        self.dictation_unlocked(id)?;
        self.invalidate_privacy_unlocked();
        let mut entries = self.learning_unlocked()?;
        remove_learning_sources(&mut entries, &HashSet::from([id.to_owned()]));
        // Remove derivatives first: an I/O failure cannot orphan private excerpts.
        self.write_learning_unlocked(&entries)?;
        self.clear_dictation_caches_unlocked()?;
        self.remove_owned_unlocked(&self.root.join("dictations").join(format!("{id}.json")))
    }

    /// Scans the complete archive, independent of the 100-row display limit.
    /// Any partial I/O failure is an error, never a successful deleted-all count.
    pub fn delete_all_dictations(&self, confirmed: bool) -> Result<usize, String> {
        if !confirmed {
            return Err("Confirm deletion of all saved dictations first.".into());
        }
        let _guard = self.lock()?;
        self.invalidate_privacy_unlocked();
        let paths = self.dictation_paths_unlocked()?;
        let mut entries = self.learning_unlocked()?;
        // Includes dangling source references left by externally removed records.
        entries.retain(|e| e.source_ids.is_empty() && e.source_type == "manual");
        self.write_learning_unlocked(&entries)?;
        self.clear_dictation_caches_unlocked()?;
        for (index, (_, path)) in paths.iter().enumerate() {
            self.remove_owned_unlocked(path).map_err(|error| {
                format!("Deleted {index} of {} dictations. {error}", paths.len())
            })?;
        }
        Ok(paths.len())
    }

    fn learning_unlocked(&self) -> Result<Vec<LearnedEntry>, String> {
        let path = self.root.join("learning.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        let value: LearningFile = self.read(&path)?;
        if value.schema_version != 1 {
            return Err("This learning file uses an unsupported schema version.".into());
        }
        learning::validate(&value.entries)?;
        Ok(value.entries)
    }

    fn write_learning_unlocked(&self, entries: &[LearnedEntry]) -> Result<(), String> {
        learning::validate(entries)?;
        if entries.is_empty() {
            return self.remove_owned_unlocked(&self.root.join("learning.json"));
        }
        self.write(
            &self.root.join("learning.json"),
            &LearningFile {
                schema_version: 1,
                entries: entries.to_vec(),
            },
        )
    }

    pub fn get_learning(&self) -> Result<Vec<LearnedEntry>, String> {
        let _guard = self.lock()?;
        self.supported_learning_unlocked()
    }

    fn supported_learning_unlocked(&self) -> Result<Vec<LearnedEntry>, String> {
        let mut entries = self.learning_unlocked()?;
        // Missing externally removed sources also cannot support retrieval.
        for entry in &mut entries {
            entry
                .source_ids
                .retain(|id| self.dictation_unlocked(id).is_ok());
        }
        entries.retain(|e| e.source_type == "manual" || !e.source_ids.is_empty());
        self.write_learning_unlocked(&entries)?;
        Ok(entries)
    }

    pub fn save_learning(&self, requested: &[LearnedEntry]) -> Result<Vec<LearnedEntry>, String> {
        let _guard = self.lock()?;
        let existing = self.learning_unlocked()?;
        let mut entries = requested.to_vec();
        for entry in &mut entries {
            entry.text = entry.text.trim().into();
            entry.heard = entry
                .heard
                .take()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            entry.scope = entry
                .scope
                .take()
                .map(|s| s.trim().to_owned())
                .filter(|s| !s.is_empty());
            if entry.id.is_empty() {
                // A deliberate authored entry is independent. A caller cannot
                // forge a deleted derived entry or strip its source metadata.
                if !entry.source_ids.is_empty()
                    || entry.source_type != "manual"
                    || !entry.user_pinned
                {
                    return Err(
                        "New manual learning must be explicitly saved without source references."
                            .into(),
                    );
                }
                entry.id = new_id();
            } else {
                let old = existing
                    .iter()
                    .find(|e| e.id == entry.id)
                    .ok_or("This learned entry was deleted. Reload Vocabulary before saving.")?;
                if entry.source_ids != old.source_ids
                    || entry.source_type != old.source_type
                    || entry.user_pinned != old.user_pinned
                {
                    return Err("Learning provenance cannot be changed. Save an independent vocabulary entry explicitly.".into());
                }
            }
            for id in &entry.source_ids {
                self.dictation_unlocked(id)?;
            }
        }
        learning::validate(&entries)?;
        self.invalidate_privacy_unlocked();
        self.write_learning_unlocked(&entries)?;
        Ok(entries)
    }

    pub fn reset_learning(&self, confirmed: bool) -> Result<(), String> {
        if !confirmed {
            return Err("Confirm removal of learned entries first.".into());
        }
        let _guard = self.lock()?;
        self.invalidate_privacy_unlocked();
        self.remove_owned_unlocked(&self.root.join("learning.json"))
    }

    /// Save one explicit edit, preserving immutable raw recognition. Automatic
    /// learning is synchronous under this gate; there is no queued extractor.
    pub fn save_dictation_correction(
        &self,
        id: &str,
        text: &str,
        scope: Option<&str>,
    ) -> Result<SavedCorrection, String> {
        validate_id(id)?;
        validate_text(text, "Corrected dictation")?;
        let generation = self.privacy_generation();
        let _guard = self.lock()?;
        let mut record = self.dictation_unlocked(id)?;
        let settings = self.settings_unlocked()?;
        let scope = scope.map(str::trim).filter(|s| !s.is_empty());
        if scope.is_some_and(|s| s.len() > 160 || s.chars().any(char::is_control)) {
            return Err(
                "Learning scopes must be at most 160 bytes without control characters.".into(),
            );
        }
        let mut entries = self.learning_unlocked()?;
        let mut learned_count = 0;
        if generation == self.privacy_generation()
            && settings.learn_from_corrections
            && !settings.do_not_save_dictations
            && scope.is_some()
            && record.recognition_completeness == RecognitionCompleteness::Complete
        {
            if let Some((heard, written)) = learning::narrow_correction(&record.published, text) {
                // A previous generated spelling is not independent ASR evidence.
                if contains_phrase(&record.raw, &heard) && entries.len() < learning::MAX_ENTRIES {
                    let same = entries.iter_mut().find(|e| {
                        e.kind == LearnedKind::Correction
                            && e.heard
                                .as_deref()
                                .is_some_and(|h| h.eq_ignore_ascii_case(&heard))
                            && e.scope
                                .as_deref()
                                .is_some_and(|s| s.eq_ignore_ascii_case(scope.unwrap()))
                    });
                    if let Some(entry) = same {
                        // Disabled/conflicting evidence stays disabled/unchanged.
                        if entry.enabled
                            && entry.text == written
                            && entry.source_type == "dictationCorrection"
                            && !entry.source_ids.iter().any(|s| s == id)
                            && entry.source_ids.len() < 32
                        {
                            entry.source_ids.push(id.to_owned());
                        }
                    } else {
                        entries.push(LearnedEntry {
                            id: new_id(),
                            kind: LearnedKind::Correction,
                            text: written,
                            heard: Some(heard),
                            scope: scope.map(str::to_owned),
                            source_ids: vec![id.to_owned()],
                            source_type: "dictationCorrection".into(),
                            enabled: true,
                            user_pinned: false,
                        });
                        learned_count = 1;
                    }
                }
            }
        }
        record.published = text.to_owned();
        if record.publication_stage.is_some() {
            record.publication_stage = Some("edited".into());
        }
        self.write(
            &self.root.join("dictations").join(format!("{id}.json")),
            &record,
        )?;
        self.write_learning_unlocked(&entries)?;
        Ok(SavedCorrection {
            record,
            learned_count,
        })
    }

    /// Call after recognition, never for a cloud notes request. The returned
    /// entries are small, relevant, explicitly scoped, and supported by sources.
    pub fn learned_vocabulary_for(
        &self,
        raw: &str,
        mode: CleanupMode,
        byte_budget: usize,
    ) -> Result<Vec<VocabularyEntry>, String> {
        let _guard = self.lock()?;
        let settings = self.settings_unlocked()?;
        if !settings.learn_from_corrections {
            return Ok(Vec::new());
        }
        Ok(learning::relevant(
            &self.supported_learning_unlocked()?,
            raw,
            mode,
            byte_budget,
        ))
    }

    /// Speech integration hook, called after ASR on the full raw utterance.
    /// Do not retrieve per chunk: an unscoped occurrence elsewhere must abstain.
    pub fn learned_cleanup_vocabulary_for(
        &self,
        raw: &str,
        mode: CleanupMode,
        byte_budget: usize,
    ) -> Result<Vec<crate::cleanup::Vocabulary>, String> {
        Ok(self
            .learned_vocabulary_for(raw, mode, byte_budget)?
            .into_iter()
            .map(|entry| crate::cleanup::Vocabulary {
                heard: entry.heard,
                written: entry.written,
                enabled: true,
                context: entry
                    .scope
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_lowercase)
                    .collect(),
            })
            .collect())
    }

    /// Explicitly saved Vocabulary is independent of its source transcript and
    /// retained on history deletion. Automatic learning uses learning.json instead.
    pub fn learn_correction(
        &self,
        dictation_id: &str,
        heard: &str,
        written: &str,
    ) -> Result<Vec<VocabularyEntry>, String> {
        validate_id(dictation_id)?;
        let _guard = self.lock()?;
        let record = self.dictation_unlocked(dictation_id)?;
        if heard.trim().is_empty() || !contains_phrase(&record.raw, heard.trim()) {
            return Err("The heard phrase must occur in the original dictation.".into());
        }
        let mut entries = self.vocabulary_unlocked()?;
        if let Some(entry) = entries
            .iter_mut()
            .find(|e| e.heard.eq_ignore_ascii_case(heard.trim()))
        {
            if entry.written != written.trim() {
                return Err("This heard phrase already has a different spelling. Edit it in Vocabulary to change it.".into());
            }
            entry.written = written.trim().into();
            entry.enabled = true;
        } else {
            entries.push(VocabularyEntry {
                id: new_id(),
                heard: heard.trim().into(),
                written: written.trim().into(),
                enabled: true,
                scope: None,
            });
        }
        validate_vocabulary(&entries)?;
        self.write(
            &self.root.join("vocabulary.json"),
            &VocabularyFile {
                schema_version: 1,
                entries: entries.clone(),
            },
        )?;
        Ok(entries)
    }

    pub fn import_legacy_default(&self) -> Result<MigrationReport, String> {
        let mut report = MigrationReport::default();
        let mut roots = Vec::new();
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            roots.push(PathBuf::from(local).join("EavesdropperData"));
        }
        // MeetingSessionStore historically used roaming AppData/Eavesdropper.
        if let Some(roaming) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(roaming).join("Eavesdropper"));
        }
        for root in roots.into_iter().filter(|p| p.is_dir()) {
            let imported = self.import_legacy(&root)?;
            report.settings_imported |= imported.settings_imported;
            report.vocabulary_imported += imported.vocabulary_imported;
            report.meetings_imported += imported.meetings_imported;
            report.training_sessions_imported += imported.training_sessions_imported;
            report.warnings.extend(imported.warnings);
        }
        Ok(report)
    }

    /// Reads selected legacy fields; credentials and `.env` contents are never copied.
    pub fn import_legacy(&self, legacy_root: &Path) -> Result<MigrationReport, String> {
        let legacy_root = fs::canonicalize(legacy_root)
            .map_err(|e| format!("Cannot read the legacy data directory: {e}"))?;
        if legacy_root == *self.root
            || legacy_root.starts_with(self.root())
            || self.root.starts_with(&legacy_root)
        {
            return Err("Legacy and new app data directories must be separate.".into());
        }
        let source_id = stable_id(&legacy_root.to_string_lossy());
        let marker = self
            .root
            .join("migrations")
            .join(format!("legacy-v1-{source_id}.json"));
        if marker.exists() {
            return Ok(MigrationReport::default());
        }
        let mut report = MigrationReport::default();
        let config_path = legacy_root.join("config.json");
        let config: Value = if config_path.is_file() {
            read_json(&config_path)?
        } else {
            Value::Null
        };
        if config.is_object() && !self.root.join("settings.json").exists() {
            let mut settings = Settings::default();
            if let Some(v) = field(&config, "Hotkey").and_then(Value::as_str) {
                settings.hotkey = v.replace(' ', "");
            }
            if field(&config, "HotkeyMode")
                .is_some_and(|v| v.as_i64() == Some(1) || v.as_str() == Some("HoldToTalk"))
            {
                settings.hotkey_mode = HotkeyMode::Hold;
            }
            if let Some(v) = field(&config, "LaunchAtLogin").and_then(Value::as_bool) {
                settings.launch_at_login = v;
            }
            if let Some(v) = field(&config, "PlayAudioCues").and_then(Value::as_bool) {
                settings.audio_cues = v;
            }
            if field(&config, "EnableTextCleanup").and_then(Value::as_bool) == Some(false) {
                settings.cleanup_mode = CleanupMode::Original;
            }
            if field(&config, "AutoStopAfterSilence").and_then(Value::as_bool) == Some(true) {
                settings.auto_stop_seconds = field(&config, "AutoStopSilenceSeconds")
                    .and_then(Value::as_u64)
                    .unwrap_or(2)
                    .clamp(1, 30) as u32;
            }
            self.save_settings(&settings)?;
            report.settings_imported = true;
        }
        if !self.root.join("vocabulary.json").exists() {
            let mut entries = Vec::new();
            if let Some(v) = field(&config, "CustomVocabulary").and_then(Value::as_str) {
                for pair in v.split(';') {
                    if let Some((heard, written)) = pair.split_once('=') {
                        if !heard.trim().is_empty() && !written.trim().is_empty() {
                            entries.push(VocabularyEntry {
                                id: new_id(),
                                heard: heard.trim().into(),
                                written: written.trim().into(),
                                enabled: true,
                                scope: None,
                            });
                        }
                    }
                }
            }
            let glossary_path = legacy_root.join("glossary.json");
            if glossary_path.is_file() {
                let glossary: Value = read_json(&glossary_path)?;
                if let Some(items) = field(&glossary, "entries").and_then(Value::as_array) {
                    for item in items {
                        let heard = string_field(item, "heard");
                        let written = string_field(item, "meant");
                        if heard.trim().is_empty() || written.trim().is_empty() {
                            continue;
                        }
                        if entries.iter().any(|e| e.heard.eq_ignore_ascii_case(&heard)) {
                            continue;
                        }
                        let status = string_field(item, "status");
                        let scope = string_field(item, "scope");
                        // Scope cannot be represented by the new global vocabulary; preserve it disabled.
                        let enabled = scope.is_empty()
                            && matches!(
                                status.to_ascii_lowercase().as_str(),
                                "confirmed" | "active" | "approved"
                            );
                        entries.push(VocabularyEntry {
                            id: new_id(),
                            heard,
                            written,
                            enabled,
                            scope: None,
                        });
                    }
                }
            }
            if !entries.is_empty() {
                report.vocabulary_imported = self.save_vocabulary(&entries)?.len();
            }
        }
        report.meetings_imported = crate::meetings::import_legacy_meetings(self, &legacy_root)?;
        let sessions = legacy_root.join("training/sessions");
        if sessions.is_dir() {
            let paths = fs::read_dir(sessions)
                .map_err(|e| format!("Cannot read legacy training sessions: {e}"))?;
            for entry in paths.take(2_000) {
                let path = entry
                    .map_err(|e| format!("Cannot read a legacy training session: {e}"))?
                    .path();
                if path.extension().and_then(|s| s.to_str()) != Some("json") || !path.is_file() {
                    continue;
                }
                let legacy: Value = read_json(&path)?;
                let raw = string_field(&legacy, "Raw");
                let published = string_field(&legacy, "Refined");
                validate_text(&raw, "Legacy raw dictation")?;
                validate_text(&published, "Legacy published dictation")?;
                let id = format!("legacy-{source_id}-{}", stable_id(&path.to_string_lossy()));
                let dest = self.root.join("dictations").join(format!("{id}.json"));
                if !dest.exists() {
                    self.write(
                        &dest,
                        &DictationRecord {
                            id,
                            created_at: string_field(&legacy, "TimestampUtc"),
                            raw,
                            published,
                            target_app: None,
                            pre_model_text: None,
                            publication_stage: None,
                            guard_reason: None,
                            cleanup_status: None,
                            recognition_completeness: RecognitionCompleteness::Complete,
                            recognition_warnings: Vec::new(),
                        },
                    )?;
                    report.training_sessions_imported += 1;
                }
            }
        }
        self.write(
            &marker,
            &serde_json::json!({"schemaVersion":1,"sourceRoot":legacy_root,"report":report}),
        )?;
        Ok(report)
    }

    pub(crate) fn check_export_destination(&self, path: &Path) -> Result<(), String> {
        let parent = path
            .parent()
            .ok_or("Choose an export destination.")?
            .canonicalize()
            .map_err(|e| format!("Cannot open export directory: {e}"))?;
        if parent.starts_with(self.root()) {
            return Err("Choose an export destination outside the app data directory.".into());
        }
        let markers = fs::read_dir(self.root.join("migrations"))
            .map_err(|e| format!("Cannot inspect legacy data protection: {e}"))?;
        for marker in markers {
            let marker = marker
                .map_err(|e| format!("Cannot inspect legacy data protection: {e}"))?
                .path();
            if marker.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let value: Value = self.read(&marker)?;
            if let Some(root) = value.get("sourceRoot").and_then(Value::as_str) {
                if parent.starts_with(Path::new(root)) {
                    return Err(
                        "Choose an export destination outside the original legacy data directory."
                            .into(),
                    );
                }
            }
        }
        Ok(())
    }
}

fn remove_learning_sources(entries: &mut Vec<LearnedEntry>, removed: &HashSet<String>) {
    for entry in entries.iter_mut() {
        entry.source_ids.retain(|id| !removed.contains(id));
    }
    entries.retain(|e| e.source_type == "manual" || !e.source_ids.is_empty());
}

pub(crate) const DATA_DIR_NAME: &str = "SonoJot";
/// Builds up to 0.28.3 kept their data here. The C# app's `EavesdropperData`
/// and roaming `Eavesdropper` folders are separate and only ever read by the
/// legacy import.
pub(crate) const PREVIOUS_DATA_DIR_NAME: &str = "EavesdropperNext";

/// Where the per-user data folder lives: `%LOCALAPPDATA%`, macOS Application
/// Support, or the XDG data directory.
fn data_parent() -> Result<PathBuf, String> {
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local));
    }
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join("Library/Application Support"));
    }
    if let Some(data) = std::env::var_os("XDG_DATA_HOME") {
        return Ok(PathBuf::from(data));
    }
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join(".local/share"));
    }
    Err("Set SONOJOT_DATA_DIR to a writable data directory.".into())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum DataFolder {
    /// The SonoJot folder was already in place, or nothing existed yet.
    Current,
    /// The pre-SonoJot folder was renamed to the SonoJot name this launch.
    Moved,
    /// The rename failed; this run keeps using the pre-SonoJot folder and the
    /// next launch tries again. The value is the OS error code, when known.
    MoveFailed(Option<i32>),
}

static DEFAULT_ROOT: std::sync::OnceLock<Result<(PathBuf, DataFolder), String>> =
    std::sync::OnceLock::new();

/// The data folder for this run. The first call moves the pre-SonoJot folder
/// into place, so it must run before anything opens files in either folder.
/// Tests never move a real folder; they only resolve which one exists.
pub(crate) fn default_root() -> Result<PathBuf, String> {
    resolve_default_root().map(|(root, _)| root)
}

/// What the first `default_root` call did to the data folder.
pub(crate) fn default_root_outcome() -> Option<DataFolder> {
    DEFAULT_ROOT
        .get()
        .and_then(|result| result.as_ref().ok())
        .map(|(_, outcome)| outcome.clone())
}

fn resolve_default_root() -> Result<(PathBuf, DataFolder), String> {
    DEFAULT_ROOT
        .get_or_init(|| {
            let parent = data_parent()?;
            if cfg!(test) {
                return Ok((existing_data_folder(&parent), DataFolder::Current));
            }
            Ok(adopt_previous_data_folder(&parent, |from, to| fs::rename(from, to)))
        })
        .clone()
}

/// Read-only resolution: the SonoJot folder unless only the previous one exists.
fn existing_data_folder(parent: &Path) -> PathBuf {
    let current = parent.join(DATA_DIR_NAME);
    let previous = parent.join(PREVIOUS_DATA_DIR_NAME);
    if fs::symlink_metadata(&current).is_err() && previous.is_dir() {
        previous
    } else {
        current
    }
}

/// Move `EavesdropperNext` to `SonoJot` with one same-volume rename, which is
/// atomic: the data is either entirely under the old name or entirely under
/// the new one, never half-copied. If the rename fails (a file held open by
/// another program, permissions), this run keeps using the old folder. When
/// both exist the SonoJot folder wins and the old one is left untouched,
/// unless the SonoJot folder is empty, which only an interrupted or accidental
/// creation leaves behind.
fn adopt_previous_data_folder(
    parent: &Path,
    rename: impl FnOnce(&Path, &Path) -> std::io::Result<()>,
) -> (PathBuf, DataFolder) {
    let current = parent.join(DATA_DIR_NAME);
    let previous = parent.join(PREVIOUS_DATA_DIR_NAME);
    if !previous.is_dir() {
        return (current, DataFolder::Current);
    }
    if fs::symlink_metadata(&current).is_ok() {
        let empty = fs::read_dir(&current).is_ok_and(|mut entries| entries.next().is_none());
        // remove_dir only removes an empty directory, so this cannot lose data.
        if !empty || fs::remove_dir(&current).is_err() {
            return (current, DataFolder::Current);
        }
    }
    let previous_canonical = fs::canonicalize(&previous).ok();
    match rename(&previous, &current) {
        Ok(()) => {
            let mut prefixes = vec![(previous.clone(), current.clone())];
            if let (Some(old), Ok(new)) = (previous_canonical, fs::canonicalize(&current)) {
                prefixes.insert(0, (old, new));
            }
            rebase_stored_paths(&current, &prefixes);
            eprintln!(
                "SonoJot moved its data folder from {} to {}",
                previous.display(),
                current.display()
            );
            (current, DataFolder::Moved)
        }
        Err(error) => {
            eprintln!(
                "SonoJot could not move its data folder from {} to {} ({error}); using the old folder this time",
                previous.display(),
                current.display()
            );
            (previous, DataFolder::MoveFailed(error.raw_os_error()))
        }
    }
}

/// Files that record absolute paths into the data folder: the managed FFmpeg
/// pointer, "Locate models" pointers and model verification receipts. Meeting
/// audio paths are rebuilt from the current root on every read and need no
/// rewrite. Other stored files hold no paths into the data folder.
const PATH_FILES: [&str; 3] = [
    "converter.json",
    "paths.json",
    crate::model_verification::CACHE_FILE,
];
/// Folders that hold user content rather than path records; never scanned.
const CONTENT_FOLDERS: [&str; 6] = ["meetings", "dictations", "logs", "diagnostics", "temp", "migrations"];

/// After the rename, point stored absolute paths at the new folder. A file that
/// cannot be read or rewritten is left as it was: each reader already treats
/// a missing path as absent and falls back (re-locate, re-install, re-verify).
fn rebase_stored_paths(root: &Path, prefixes: &[(PathBuf, PathBuf)]) {
    fn visit(directory: &Path, depth: usize, root: &Path, prefixes: &[(PathBuf, PathBuf)]) {
        let Ok(entries) = fs::read_dir(directory) else { return };
        for entry in entries.flatten() {
            let Ok(kind) = entry.file_type() else { continue };
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if kind.is_dir() {
                let content = directory == root && CONTENT_FOLDERS.contains(&name.as_ref());
                if depth < 4 && !content {
                    visit(&entry.path(), depth + 1, root, prefixes);
                }
            } else if kind.is_file() && PATH_FILES.contains(&name.as_ref()) {
                rebase_file(&entry.path(), prefixes);
            }
        }
    }
    visit(root, 0, root, prefixes);
}

fn rebase_file(path: &Path, prefixes: &[(PathBuf, PathBuf)]) {
    if fs::metadata(path).map_or(true, |m| m.len() > MAX_JSON_BYTES) {
        return;
    }
    let Ok(bytes) = fs::read(path) else { return };
    let Ok(mut value) = serde_json::from_slice::<Value>(&bytes) else { return };
    if rebase_value(&mut value, prefixes) {
        if let Ok(bytes) = serde_json::to_vec_pretty(&value) {
            if let Err(error) = atomic_write(path, &bytes) {
                eprintln!("SonoJot could not update {}: {error}", path.display());
            }
        }
    }
}

fn rebase_value(value: &mut Value, prefixes: &[(PathBuf, PathBuf)]) -> bool {
    match value {
        Value::String(text) => {
            for (old, new) in prefixes {
                if let Ok(rest) = Path::new(text.as_str()).strip_prefix(old) {
                    let moved = if rest.as_os_str().is_empty() {
                        new.clone()
                    } else {
                        new.join(rest)
                    };
                    *text = moved.to_string_lossy().into_owned();
                    return true;
                }
            }
            false
        }
        Value::Array(items) => items
            .iter_mut()
            .fold(false, |changed, item| rebase_value(item, prefixes) | changed),
        Value::Object(map) => map
            .values_mut()
            .fold(false, |changed, item| rebase_value(item, prefixes) | changed),
        _ => false,
    }
}
pub(crate) fn new_id() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
pub(crate) fn now() -> String {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .expect("UTC date formatting is infallible")
}
pub(crate) fn stable_id(value: &str) -> String {
    let hash = value.bytes().fold(0xcbf29ce484222325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x100000001b3)
    });
    format!("{hash:016x}")
}
pub(crate) fn validate_id(id: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid record identifier.".into());
    }
    Ok(())
}
pub(crate) fn validate_text(text: &str, label: &str) -> Result<(), String> {
    if text.len() > MAX_TEXT_BYTES || text.contains('\0') {
        return Err(format!(
            "{label} exceeds the 2 MiB text limit or contains a null character."
        ));
    }
    Ok(())
}
fn validate_vocabulary(entries: &[VocabularyEntry]) -> Result<(), String> {
    if entries.len() > MAX_VOCABULARY_ENTRIES {
        return Err("Vocabulary is limited to 2,000 entries.".into());
    }
    let mut ids = HashSet::new();
    let mut active = HashSet::new();
    for entry in entries {
        validate_id(&entry.id)?;
        if !ids.insert(&entry.id) {
            return Err("Vocabulary identifiers must be unique.".into());
        }
        for value in [&entry.heard, &entry.written] {
            if value.trim().is_empty() || value.len() > 4096 || value.chars().any(char::is_control)
            {
                return Err("Vocabulary phrases must contain between 1 and 4,096 bytes without control characters.".into());
            }
        }
        if entry
            .scope
            .as_deref()
            .is_some_and(|scope| scope.len() > 1024 || scope.chars().any(char::is_control))
        {
            return Err(
                "Vocabulary scopes must be 1,024 bytes or fewer without control characters.".into(),
            );
        }
        if entry.enabled && !active.insert(entry.heard.to_lowercase()) {
            return Err("Only one enabled correction is allowed for each heard phrase.".into());
        }
    }
    Ok(())
}
fn contains_phrase(text: &str, phrase: &str) -> bool {
    let text = text.to_lowercase();
    let phrase = phrase.to_lowercase();
    text.match_indices(&phrase).any(|(start, matched)| {
        !text[..start]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
            && !text[start + matched.len()..]
                .chars()
                .next()
                .is_some_and(char::is_alphanumeric)
    })
}
pub(crate) fn field<'a>(value: &'a Value, key: &str) -> Option<&'a Value> {
    value
        .as_object()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case(key))
        .map(|(_, v)| v)
}
pub(crate) fn string_field(value: &Value, key: &str) -> String {
    field(value, key)
        .and_then(Value::as_str)
        .unwrap_or("")
        .into()
}
pub(crate) fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T, String> {
    let file = File::open(path).map_err(|e| format!("Cannot read {}: {e}", path.display()))?;
    if file
        .metadata()
        .map_err(|e| format!("Cannot inspect data file: {e}"))?
        .len()
        > MAX_JSON_BYTES
    {
        return Err("A data file exceeds the 32 MiB limit.".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_JSON_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| format!("Cannot read data file: {e}"))?;
    if bytes.len() as u64 > MAX_JSON_BYTES {
        return Err("A data file exceeds the 32 MiB limit.".into());
    }
    let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&bytes);
    serde_json::from_slice(bytes).map_err(|e| {
        format!(
            "Invalid data in {} at line {}, column {}. The original file was preserved.",
            path.file_name().unwrap_or_default().to_string_lossy(),
            e.line(),
            e.column()
        )
    })
}

pub(crate) fn atomic_write(path: &Path, data: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("The destination has no parent directory.")?;
    if !parent.is_dir() {
        return Err("The destination directory does not exist.".into());
    }
    if fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Cannot overwrite a linked destination file.".into());
    }
    let temporary = parent.join(format!(".sonojot-{}.tmp", new_id()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("Cannot create a temporary data file: {e}"))?;
        file.write_all(data)
            .and_then(|_| file.sync_all())
            .map_err(|e| format!("Cannot save data: {e}"))?;
        drop(file);
        replace_file(&temporary, path)
            .map_err(|e| format!("Cannot replace the saved file: {e}"))?;
        #[cfg(unix)]
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("Cannot flush the saved directory: {e}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}
#[cfg(not(windows))]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    fs::rename(from, to)
}
#[cfg(windows)]
fn replace_file(from: &Path, to: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn MoveFileExW(existing: *const u16, replacement: *const u16, flags: u32) -> i32;
    }
    let from: Vec<u16> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<u16> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // Same-directory replacement is atomic; WRITE_THROUGH requests durable completion.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), 0x1 | 0x8) } == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(store: &Store, text: &str) -> DictationRecord {
        store
            .record_dictation(text, text, RecognitionCompleteness::Complete, &[])
            .unwrap()
    }

    fn learning_settings(store: &Store) {
        store
            .save_settings(&Settings {
                learn_from_corrections: true,
                ..Settings::default()
            })
            .unwrap();
    }

    #[test]
    fn privacy_learning_keeps_only_surviving_support_and_manual_entries() {
        let (directory, store) = temp_store("privacy-sources");
        learning_settings(&store);
        let first = sample(&store, "Open the Velmora project");
        let second = sample(&store, "Open the Velmora project");
        for record in [&first, &second] {
            store
                .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
                .unwrap();
        }
        let mut entries = store.get_learning().unwrap();
        assert_eq!(entries[0].source_ids.len(), 2);
        entries.push(LearnedEntry {
            id: String::new(),
            kind: LearnedKind::Canonical,
            text: "SonoJot".into(),
            heard: None,
            scope: Some("app".into()),
            source_ids: vec![],
            source_type: "manual".into(),
            enabled: true,
            user_pinned: true,
        });
        store.save_learning(&entries).unwrap();
        store.delete_dictation(&first.id, true).unwrap();
        let entries = store.get_learning().unwrap();
        assert_eq!(entries[0].source_ids, vec![second.id.clone()]);
        assert_eq!(
            store
                .learned_vocabulary_for("Open the Velmora project", CleanupMode::Clean, 200)
                .unwrap()
                .len(),
            1
        );
        store.delete_all_dictations(true).unwrap();
        let entries = store.get_learning().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].text, "SonoJot");
        assert!(store
            .learned_vocabulary_for("Open the Velmora project", CleanupMode::Clean, 200)
            .unwrap()
            .is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[cfg(windows)]
    #[test]
    fn privacy_delete_failure_is_not_reported_as_success() {
        use std::os::windows::fs::OpenOptionsExt;
        let (directory, store) = temp_store("privacy-delete-failure");
        let record = sample(&store, "A retained record");
        let path = store
            .root()
            .join("dictations")
            .join(format!("{}.json", record.id));
        // FILE_SHARE_READ permits validation but denies delete sharing.
        let blocker = OpenOptions::new()
            .read(true)
            .share_mode(1)
            .open(&path)
            .unwrap();
        let result = store.delete_all_dictations(true);
        drop(blocker);
        assert!(result.is_err());
        assert_eq!(store.count_dictations().unwrap(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_commit_rechecks_opt_out_restart_and_generation() {
        let (directory, store) = temp_store("privacy-commit");
        let saved = sample(&store, "existing saved text");
        let generation = store.privacy_generation();
        let mut settings = store.load_settings().unwrap();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        assert!(store
            .commit_dictation(
                generation,
                "private canary raw",
                "private canary output",
                RecognitionCompleteness::Complete,
                &[]
            )
            .unwrap()
            .is_none());
        assert!(store
            .commit_dictation(
                store.privacy_generation(),
                "private canary raw",
                "private canary output",
                RecognitionCompleteness::Incomplete,
                &["private diagnostic".into()]
            )
            .unwrap()
            .is_none());
        assert_eq!(store.count_dictations().unwrap(), 1);
        assert_eq!(store.get_dictation(&saved.id).unwrap().raw, saved.raw);
        let reopened = Store::open(Some(directory.clone())).unwrap();
        assert!(reopened.load_settings().unwrap().do_not_save_dictations);
        assert!(reopened
            .record_dictation(
                "private after restart",
                "private",
                RecognitionCompleteness::Complete,
                &[]
            )
            .is_err());
        settings.do_not_save_dictations = false;
        store.save_settings(&settings).unwrap();
        assert!(store
            .commit_dictation(
                generation,
                "stale",
                "stale",
                RecognitionCompleteness::Complete,
                &[]
            )
            .unwrap()
            .is_none());
        assert_eq!(sample(&store, "new retained text").raw, "new retained text");
        assert!(!directory.join("last-published.json").exists());
        assert!(!directory.join("logs/delivery.jsonl").exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_delete_all_is_confirmed_full_archive_and_invalidates_late_saves() {
        let (directory, store) = temp_store("privacy-delete-all");
        for i in 0..105 {
            sample(&store, &format!("dictation {i}"));
        }
        assert_eq!(store.list_dictations(100).unwrap().len(), 100);
        assert_eq!(store.count_dictations().unwrap(), 105);
        let generation = store.privacy_generation();
        assert!(store.delete_all_dictations(false).is_err());
        assert_eq!(store.count_dictations().unwrap(), 105);
        assert_eq!(store.delete_all_dictations(true).unwrap(), 105);
        assert!(store
            .commit_dictation(
                generation,
                "late",
                "late",
                RecognitionCompleteness::Complete,
                &[]
            )
            .unwrap()
            .is_none());
        assert_eq!(store.count_dictations().unwrap(), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_deletion_validates_exact_id_and_reports_corrupt_archive() {
        let (directory, store) = temp_store("privacy-exact-id");
        let first = sample(&store, "same published text");
        let second = sample(&store, "same published text");
        assert!(store.delete_dictation(&first.id, false).is_err());
        for id in ["../settings", "..\\settings", "C:\\outside", "CON:", ""] {
            assert!(store.delete_dictation(id, true).is_err());
        }
        store.delete_dictation(&first.id, true).unwrap();
        assert!(store.get_dictation(&first.id).is_err());
        assert!(store.get_dictation(&second.id).is_ok());
        let mut wrong = second.clone();
        wrong.id = "mismatched".into();
        store
            .write(
                &store
                    .root()
                    .join("dictations")
                    .join(format!("{}.json", second.id)),
                &wrong,
            )
            .unwrap();
        assert!(store.delete_all_dictations(true).is_err());
        assert!(directory
            .join("dictations")
            .join(format!("{}.json", second.id))
            .exists());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_learning_is_opt_in_source_bound_and_reset_rejects_stale_edits() {
        let (directory, store) = temp_store("privacy-learning");
        let record = sample(&store, "Open the Velmora project");
        let unchanged_raw = record.raw.clone();
        assert_eq!(
            store
                .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
                .unwrap()
                .learned_count,
            0
        );
        learning_settings(&store);
        let record = sample(&store, "Open the Velmora project");
        let saved = store
            .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
            .unwrap();
        assert_eq!(saved.learned_count, 1);
        assert_eq!(saved.record.raw, unchanged_raw);
        let entries = store.get_learning().unwrap();
        assert_eq!(entries[0].source_ids, vec![record.id.clone()]);
        let mut forged = entries.clone();
        forged[0].source_ids.clear();
        forged[0].user_pinned = true;
        forged[0].source_type = "manual".into();
        assert!(store.save_learning(&forged).is_err());
        store.delete_dictation(&record.id, true).unwrap();
        assert!(store.get_learning().unwrap().is_empty());
        assert!(store.save_learning(&entries).is_err());
        assert!(store
            .save_dictation_correction(&record.id, "Recreate deleted source", Some("project"))
            .is_err());
        let record = sample(&store, "Open the Velmora project");
        store
            .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
            .unwrap();
        let entries = store.get_learning().unwrap();
        assert!(store.reset_learning(false).is_err());
        store.reset_learning(true).unwrap();
        assert!(store.save_learning(&entries).is_err());
        assert!(store.get_dictation(&record.id).is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_disabling_learning_or_no_history_stops_new_learning_keeps_manual_vocabulary() {
        let (directory, store) = temp_store("privacy-opt-ins");
        learning_settings(&store);
        let record = sample(&store, "Open the Velmora project");
        let explicit = store
            .learn_correction(&record.id, "Velmora", "Velmorae")
            .unwrap();
        let mut settings = store.load_settings().unwrap();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        assert_eq!(
            store
                .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
                .unwrap()
                .learned_count,
            0
        );
        store.delete_all_dictations(true).unwrap();
        assert_eq!(store.get_vocabulary().unwrap(), explicit);
        assert!(store.get_learning().unwrap().is_empty());
        settings.do_not_save_dictations = false;
        settings.learn_from_corrections = false;
        store.save_settings(&settings).unwrap();
        let record = sample(&store, "Open the Velmora project");
        assert_eq!(
            store
                .save_dictation_correction(&record.id, "Open the Velmorae project", Some("project"))
                .unwrap()
                .learned_count,
            0
        );
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn privacy_isolated_schema_migration_preserves_profile_and_clears_legacy_cache() {
        let (directory, store) = temp_store("privacy-migration");
        let mut settings = Settings::default();
        settings.asr_model = "existing-asr".into();
        settings.cleanup_model = "existing-cleanup".into();
        settings.notes_model = "existing-notes".into();
        settings.hotkey = "Alt+Space".into();
        settings.launch_at_login = true;
        let mut file = serde_json::to_value(SettingsFile {
            schema_version: 1,
            settings: settings.clone(),
        })
        .unwrap();
        for key in [
            "doNotSaveDictations",
            "learnFromCorrections",
            "memoryMode",
            "modelRetention",
        ] {
            file["settings"].as_object_mut().unwrap().remove(key);
        }
        store
            .write(&store.root().join("settings.json"), &file)
            .unwrap();
        let record = sample(&store, "Keep this original history");
        store
            .write(
                &store.root().join("last-published.json"),
                &"old private cache",
            )
            .unwrap();
        fs::write(directory.join("logs/delivery.jsonl"), "old warning content").unwrap();
        let abandoned = directory
            .join("dictations")
            .join(format!(".sonojot-{}.tmp", new_id()));
        fs::write(&abandoned, "abandoned private record").unwrap();
        let legacy_abandoned = directory.join(format!(".eavesdropper-{}.tmp", new_id()));
        fs::write(&legacy_abandoned, "abandoned legacy private record").unwrap();
        for _ in 0..2 {
            let reopened = Store::open(Some(directory.clone())).unwrap();
            let migrated = reopened.load_settings().unwrap();
            assert_eq!(migrated, settings);
            assert!(!migrated.do_not_save_dictations);
            assert!(!migrated.learn_from_corrections);
            assert_eq!(migrated.memory_mode.as_str(), "overlap");
            assert_eq!(migrated.model_retention.as_str(), "10m");
            assert_eq!(reopened.count_dictations().unwrap(), 1);
            assert_eq!(reopened.get_dictation(&record.id).unwrap().raw, record.raw);
            assert!(!directory.join("last-published.json").exists());
            assert!(!directory.join("logs/delivery.jsonl").exists());
            assert!(!abandoned.exists());
            assert!(!legacy_abandoned.exists());
        }
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn temp_store(label: &str) -> (PathBuf, Store) {
        let directory =
            std::env::temp_dir().join(format!("sonojot-store-{label}-{}", new_id()));
        let store = Store::open(Some(directory.clone())).unwrap();
        (directory, store)
    }

    #[test]
    fn meeting_source_migration_runs_once_and_preserves_later_choices() {
        for old_source in [None, Some("microphone"), Some("system"), Some("both")] {
            let (directory, store) = temp_store("meeting-source");
            let mut original = serde_json::json!({ "schemaVersion": 1, "settings": {
                "setupProfile": null, "futureSetting": "keep me"
            }});
            if let Some(source) = old_source {
                original["settings"]["meetingSource"] = source.into();
            }
            store.write(&store.root().join("settings.json"), &original).unwrap();
            let mut migrated = store.load_settings().unwrap();
            assert_eq!(migrated.meeting_source, "both");
            assert!(migrated.meeting_source_migrated);
            assert_eq!(migrated.setup_profile, None);
            let saved: Value = store.read(&store.root().join("settings.json")).unwrap();
            assert_eq!(saved["settings"]["meetingSourceMigrated"], true);
            assert_eq!(saved["settings"]["futureSetting"], "keep me");
            let bytes = fs::read(directory.join("settings.json")).unwrap();
            assert_eq!(store.load_settings().unwrap(), migrated);
            assert_eq!(fs::read(directory.join("settings.json")).unwrap(), bytes);
            for deliberate in ["microphone", "system", "both"] {
                migrated.meeting_source = deliberate.into();
                // Saving from an older client must not re-arm the migration.
                migrated.meeting_source_migrated = false;
                store.save_settings(&migrated).unwrap();
                let reopened = Store::open(Some(directory.clone())).unwrap();
                let loaded = reopened.load_settings().unwrap();
                assert_eq!(loaded.meeting_source, deliberate);
                assert!(loaded.meeting_source_migrated);
            }
            fs::remove_dir_all(directory).unwrap();
        }
    }

    #[test]
    fn setup_migration_existing_settings_preserves_models_without_writing() {
        let (directory, store) = temp_store("setup-legacy");
        let original = serde_json::json!({ "schemaVersion": 1, "settings": {
            "asrModel": "whisper-large-v3-turbo-fp32-cuda",
            "cleanupModel": "lfm2.5-1.2b", "notesModel": "openrouter",
            "cleanupMode": "original", "meetingSourceMigrated": true
        }});
        let bytes = serde_json::to_vec_pretty(&original).unwrap();
        fs::write(directory.join("settings.json"), &bytes).unwrap();
        let loaded = store.load_settings().unwrap();
        let mut expected: Settings = serde_json::from_value(original["settings"].clone()).unwrap();
        expected.setup_profile = Some(crate::domain::SetupProfile::Custom);
        assert_eq!(loaded, expected);
        assert_eq!(fs::read(directory.join("settings.json")).unwrap(), bytes);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn setup_migration_fresh_install_and_explicit_null_stay_unfinished() {
        let (directory, store) = temp_store("setup-fresh");
        assert_eq!(store.load_settings().unwrap().setup_profile, None);
        store.save_settings(&Settings::default()).unwrap();
        fs::write(directory.join("setup-seen.json"), b"{\"seen\":true}").unwrap();
        assert_eq!(store.load_settings().unwrap().setup_profile, None);
        let reopened = Store::open(Some(directory.clone())).unwrap();
        assert_eq!(reopened.load_settings().unwrap().setup_profile, None);
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn setup_migration_seen_install_without_settings_is_custom() {
        let (directory, store) = temp_store("setup-seen");
        fs::write(directory.join("setup-seen.json"), b"{\"seen\":true}").unwrap();
        let mut expected = Settings::default();
        expected.setup_profile = Some(crate::domain::SetupProfile::Custom);
        assert_eq!(store.load_settings().unwrap(), expected);
        assert!(!directory.join("settings.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn cleanup_trial_and_vocabulary_scope_round_trip() {
        let (directory, store) = temp_store("trial-round-trip");
        assert!(!store.load_settings().unwrap().cleanup_trial);
        let settings = Settings {
            cleanup_trial: true,
            ..Default::default()
        };
        store.save_settings(&settings).unwrap();
        assert!(store.load_settings().unwrap().cleanup_trial);
        let saved = store
            .save_vocabulary(&[VocabularyEntry {
                id: String::new(),
                heard: "quinn".into(),
                written: "Qwen".into(),
                enabled: true,
                scope: Some(" AI model discussion ".into()),
            }])
            .unwrap();
        // Scopes are trimmed on save, like heard and written.
        assert_eq!(saved[0].scope.as_deref(), Some("AI model discussion"));
        assert_eq!(
            store.get_vocabulary().unwrap()[0].scope.as_deref(),
            Some("AI model discussion")
        );
        let blank = store
            .save_vocabulary(&[VocabularyEntry {
                scope: Some("   ".into()),
                ..saved[0].clone()
            }])
            .unwrap();
        assert_eq!(blank[0].scope, None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn old_files_without_trial_or_scope_fields_default_cleanly() {
        let (directory, store) = temp_store("old-fields");
        let mut settings = serde_json::to_value(&SettingsFile {
            schema_version: 1,
            settings: Settings::default(),
        })
        .unwrap();
        settings["settings"]
            .as_object_mut()
            .unwrap()
            .remove("cleanupTrial");
        std::fs::write(
            directory.join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        assert!(!store.load_settings().unwrap().cleanup_trial);
        let mut vocabulary = serde_json::to_value(&VocabularyFile {
            schema_version: 1,
            entries: vec![VocabularyEntry {
                id: "one".into(),
                heard: "grock".into(),
                written: "Grok".into(),
                enabled: true,
                scope: None,
            }],
        })
        .unwrap();
        vocabulary["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("scope");
        std::fs::write(
            directory.join("vocabulary.json"),
            serde_json::to_vec(&vocabulary).unwrap(),
        )
        .unwrap();
        let entries = store.get_vocabulary().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].scope, None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn incomplete_dictation_history_preserves_available_text_and_diagnostics() {
        let (directory, store) = temp_store("incomplete-history");
        let warning = "segment two remained ambiguous after one retry";
        let record = store
            .record_dictation(
                "Keep the first instruction. <thö Do not remove the backup.",
                "Keep the first instruction. <thö Do not remove the backup.",
                RecognitionCompleteness::Incomplete,
                &[warning.into()],
            )
            .unwrap();
        let loaded = store.get_dictation(&record.id).unwrap();
        assert_eq!(
            loaded.recognition_completeness,
            RecognitionCompleteness::Incomplete
        );
        assert_eq!(loaded.recognition_warnings, vec![warning.to_owned()]);
        assert!(loaded.raw.ends_with("Do not remove the backup."));
        let empty = store
            .record_dictation(
                "",
                "",
                RecognitionCompleteness::Incomplete,
                &["no reviewable text after retry".into()],
            )
            .unwrap();
        assert!(store.get_dictation(&empty.id).unwrap().published.is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn learned_corrections_stay_unscoped() {
        let (directory, store) = temp_store("learned-scope");
        let record = store
            .record_dictation(
                "ask grock about it",
                "ask Grok about it",
                RecognitionCompleteness::Complete,
                &[],
            )
            .unwrap();
        let entries = store.learn_correction(&record.id, "grock", "Grok").unwrap();
        assert_eq!(entries[0].scope, None);
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn enable_learning(store: &Store) {
        let mut settings = store.load_settings().unwrap();
        settings.learn_from_corrections = true;
        settings.do_not_save_dictations = false;
        store.save_settings(&settings).unwrap();
    }

    fn commit(store: &Store, raw: &str, published: &str) -> DictationRecord {
        let generation = store.privacy_generation();
        store
            .commit_dictation(
                generation,
                raw,
                published,
                RecognitionCompleteness::Complete,
                &[],
            )
            .unwrap()
            .expect("retention is enabled in these tests")
    }

    #[test]
    fn no_history_commit_boundary_is_transient_and_preserves_existing() {
        let (directory, store) = temp_store("no-history-boundary");
        let kept = commit(&store, "keep grock", "keep Grok");
        let mut settings = store.load_settings().unwrap();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        // Commit-time enforcement under the same gate: no new file, no error surfacing.
        let generation = store.privacy_generation();
        let skipped = store
            .commit_dictation(
                generation,
                "private grock note",
                "private Grok note",
                RecognitionCompleteness::Complete,
                &[],
            )
            .unwrap();
        assert!(skipped.is_none());
        // Existing history remains accessible; only one record persists.
        assert_eq!(store.count_dictations().unwrap(), 1);
        assert_eq!(
            store.get_dictation(&kept.id).unwrap().published,
            "keep Grok"
        );
        assert_eq!(store.list_dictations(100).unwrap().len(), 1);
        // Stale generation commits (e.g. queued work across the toggle) also stay transient.
        let stale = store
            .commit_dictation(
                generation.wrapping_sub(1),
                "late grock",
                "late Grok",
                RecognitionCompleteness::Complete,
                &[],
            )
            .unwrap();
        assert!(stale.is_none());
        assert_eq!(store.count_dictations().unwrap(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn smart_publication_metadata_round_trip_and_legacy_defaults() {
        let (directory, store) = temp_store("smart-publication");
        for (stage, guard) in [("smart_cleanup", None), ("rules", Some("protected_number"))] {
            let record = store.commit_dictation_with_metadata(store.privacy_generation(),
                "raw repeated repeated", "Repeated.", RecognitionCompleteness::Complete, &[],
                Some("Repeated repeated."), Some(stage), guard, None).unwrap().unwrap();
            let loaded = store.get_dictation(&record.id).unwrap();
            assert_eq!(loaded.raw, "raw repeated repeated");
            assert_eq!(loaded.pre_model_text.as_deref(), Some("Repeated repeated."));
            assert_eq!(loaded.publication_stage.as_deref(), Some(stage));
            assert_eq!(loaded.guard_reason.as_deref(), guard);
            let mut legacy = serde_json::to_value(&record).unwrap();
            for field in ["preModelText", "publicationStage", "guardReason"] {
                legacy.as_object_mut().unwrap().remove(field);
            }
            let legacy: DictationRecord = serde_json::from_value(legacy).unwrap();
            assert!(legacy.pre_model_text.is_none());
            assert!(legacy.publication_stage.is_none());
            assert!(legacy.guard_reason.is_none());
        }
        let mut settings = store.load_settings().unwrap();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        assert!(store.commit_dictation_with_metadata(store.privacy_generation(),
            "private raw", "Private.", RecognitionCompleteness::Complete, &[],
            Some("Private private."), Some("smart_cleanup"), None, None).unwrap().is_none());
        assert_eq!(store.count_dictations().unwrap(), 2);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn proofread_history_preserves_recovery_and_content_free_status() {
        let (directory, store) = temp_store("proofreader-history");
        let status = "punctuation_model;proofreader_model=openai/gpt-6-luna;proofreader_outcome=accepted;proofreader_reason=accepted";
        let record = store.commit_dictation_with_status(
            store.privacy_generation(), "send the the link", "Send the link.",
            RecognitionCompleteness::Complete, &[], Some("Send the the link."),
            Some("proofread"), None, None, Some(status),
        ).unwrap().unwrap();
        let loaded = store.get_dictation(&record.id).unwrap();
        assert_eq!(loaded.raw, "send the the link");
        assert_eq!(loaded.pre_model_text.as_deref(), Some("Send the the link."));
        assert_eq!(loaded.publication_stage.as_deref(), Some("proofread"));
        assert_eq!(loaded.cleanup_status.as_deref(), Some(status));
        let mut legacy = serde_json::to_value(&loaded).unwrap();
        legacy.as_object_mut().unwrap().remove("cleanupStatus");
        assert!(serde_json::from_value::<DictationRecord>(legacy).unwrap().cleanup_status.is_none());
        let restored = store.save_dictation_correction(&record.id, "Send the the link.", None).unwrap();
        assert_eq!(restored.record.raw, loaded.raw);
        assert_eq!(restored.record.pre_model_text, loaded.pre_model_text);
        assert_eq!(restored.record.cleanup_status, loaded.cleanup_status);

        let mut settings = store.load_settings().unwrap();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        assert!(store.commit_dictation_with_status(
            store.privacy_generation(), "private raw", "Private text.",
            RecognitionCompleteness::Complete, &[], Some("Private private text."),
            Some("proofread"), None, None, Some(status),
        ).unwrap().is_none());
        assert_eq!(store.count_dictations().unwrap(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn target_app_round_trips_and_is_removed_with_its_record() {
        let (directory, store) = temp_store("target-app-history");
        let record = store.commit_dictation_with_metadata(
            store.privacy_generation(), "raw words", "Published words.",
            RecognitionCompleteness::Complete, &[], None, None, None, Some("outlook.exe"),
        ).unwrap().unwrap();
        let loaded = store.get_dictation(&record.id).unwrap();
        assert_eq!(loaded.target_app.as_deref(), Some("outlook.exe"));
        assert_eq!(loaded.raw, "raw words");
        assert_eq!(loaded.published, "Published words.");
        let mut legacy = serde_json::to_value(&loaded).unwrap();
        assert_eq!(legacy["targetApp"], "outlook.exe");
        assert!(legacy.get("target_app").is_none());
        legacy.as_object_mut().unwrap().remove("targetApp");
        assert!(serde_json::from_value::<DictationRecord>(legacy).unwrap().target_app.is_none());

        // A correction preserves the captured destination and immutable raw.
        let corrected = store.save_dictation_correction(&record.id, "Corrected words.", None).unwrap();
        assert_eq!(corrected.record.target_app, loaded.target_app);
        assert_eq!(corrected.record.raw, loaded.raw);
        let path = directory.join("dictations").join(format!("{}.json", record.id));
        assert!(fs::read_to_string(&path).unwrap().contains("outlook.exe"));
        store.delete_dictation(&record.id, true).unwrap();
        assert!(!path.exists());

        let record = store.commit_dictation_with_metadata(
            store.privacy_generation(), "another raw", "Another result.",
            RecognitionCompleteness::Complete, &[], None, None, None, Some("com.apple.mail"),
        ).unwrap().unwrap();
        assert_eq!(store.delete_all_dictations(true).unwrap(), 1);
        assert!(store.get_dictation(&record.id).is_err());
        assert_eq!(fs::read_dir(directory.join("dictations")).unwrap().count(), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn target_app_uses_the_current_privacy_and_stale_job_save_gate() {
        let (directory, store) = temp_store("target-app-privacy");
        let mut settings = store.load_settings().unwrap();
        settings.smart_layout_app_hint = true;
        store.save_settings(&settings).unwrap();
        assert!(store.load_settings().unwrap().smart_layout_app_hint);
        let retained_generation = store.privacy_generation();
        settings.do_not_save_dictations = true;
        store.save_settings(&settings).unwrap();
        let private_generation = store.privacy_generation();
        for generation in [retained_generation, private_generation] {
            assert!(store.commit_dictation_with_metadata(
                generation, "private raw", "Private result.", RecognitionCompleteness::Complete,
                &[], None, None, None, Some("outlook.exe"),
            ).unwrap().is_none());
        }
        settings.do_not_save_dictations = false;
        store.save_settings(&settings).unwrap();
        // Turning saving back on cannot persist a job begun in private mode.
        assert!(store.commit_dictation_with_metadata(
            private_generation, "private raw", "Private result.", RecognitionCompleteness::Complete,
            &[], None, None, None, Some("outlook.exe"),
        ).unwrap().is_none());
        assert_eq!(store.count_dictations().unwrap(), 0);
        assert_eq!(fs::read_dir(directory.join("dictations")).unwrap().count(), 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn strict_deletion_requires_confirmation_and_valid_id() {
        let (directory, store) = temp_store("strict-delete");
        let record = commit(&store, "ask grock", "ask Grok");
        assert!(store.delete_dictation(&record.id, false).is_err());
        assert!(store.delete_dictation("!!!", true).is_err());
        assert!(store.delete_dictation("missing01", true).is_err());
        assert_eq!(store.count_dictations().unwrap(), 1);
        store.delete_dictation(&record.id, true).unwrap();
        assert_eq!(store.count_dictations().unwrap(), 0);
        assert!(store.get_dictation(&record.id).is_err());
        // Double delete is a stale-commit failure, not silent success.
        assert!(store.delete_dictation(&record.id, true).is_err());
        assert!(store.delete_all_dictations(false).is_err());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn delete_all_covers_beyond_visible_page_and_prunes_learning() {
        let (directory, store) = temp_store("delete-all-full");
        enable_learning(&store);
        for i in 0..105 {
            commit(
                &store,
                &format!("note grock number {i}"),
                &format!("note grock number {i}"),
            );
        }
        assert_eq!(store.count_dictations().unwrap(), 105);
        assert_eq!(store.list_dictations(100).unwrap().len(), 100);
        // One scoped correction creates source-derived learning.
        let first = store.list_dictations(1).unwrap().pop().unwrap();
        let saved = store
            .save_dictation_correction(
                &first.id,
                &format!("{} Grok tail", "note"),
                Some("AI model discussion"),
            )
            .unwrap();
        let _ = saved;
        // Explicit vocabulary is independent and must survive history deletion.
        store.learn_correction(&first.id, "grock", "Grok").unwrap();
        let deleted = store.delete_all_dictations(true).unwrap();
        assert_eq!(deleted, 105);
        assert_eq!(store.count_dictations().unwrap(), 0);
        assert!(store.list_dictations(100).unwrap().is_empty());
        // Derived learning lost its support; explicit vocabulary remains.
        assert!(store.get_learning().unwrap().is_empty());
        assert_eq!(store.get_vocabulary().unwrap().len(), 1);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn correction_narrow_learns_but_numbers_directions_paragraphs_do_not() {
        let (directory, store) = temp_store("correction-gates");
        enable_learning(&store);
        let scope = Some("AI model discussion");
        let narrow = commit(
            &store,
            "ask grock about the model",
            "ask grock about the model",
        );
        let saved = store
            .save_dictation_correction(&narrow.id, "ask Grok about the model", scope)
            .unwrap();
        assert_eq!(saved.learned_count, 1);
        assert_eq!(saved.record.published, "ask Grok about the model");

        let numbers = commit(&store, "set limit to 5", "set limit to 5");
        let saved = store
            .save_dictation_correction(&numbers.id, "set limit to 6", scope)
            .unwrap();
        assert_eq!(saved.learned_count, 0);

        let direction = commit(&store, "go left now", "go left now");
        let saved = store
            .save_dictation_correction(&direction.id, "go right now", scope)
            .unwrap();
        assert_eq!(saved.learned_count, 0);

        let paragraph = commit(&store, "short raw here", "short raw here");
        let rewrite = "Completely rewritten long paragraph with many new words and altered meaning throughout";
        let saved = store
            .save_dictation_correction(&paragraph.id, rewrite, scope)
            .unwrap();
        assert_eq!(saved.learned_count, 0);

        // Unscoped explicit saves update the record but create no reusable alias.
        let unscoped = commit(&store, "ask quinn about it", "ask quinn about it");
        let saved = store
            .save_dictation_correction(&unscoped.id, "ask Qwen about it", None)
            .unwrap();
        assert_eq!(saved.learned_count, 0);

        // Incomplete recognition never becomes a trusted alias.
        let generation = store.privacy_generation();
        let incomplete = store
            .commit_dictation(
                generation,
                "ask grock maybe",
                "ask grock maybe",
                RecognitionCompleteness::Incomplete,
                &["ambiguous".into()],
            )
            .unwrap()
            .unwrap();
        let saved = store
            .save_dictation_correction(&incomplete.id, "ask Grok maybe", scope)
            .unwrap();
        assert_eq!(saved.learned_count, 0);
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn correction_stale_after_source_delete_fails() {
        let (directory, store) = temp_store("correction-stale");
        enable_learning(&store);
        let record = commit(&store, "ask grock now", "ask grock now");
        store.delete_dictation(&record.id, true).unwrap();
        assert!(store
            .save_dictation_correction(&record.id, "ask Grok now", Some("AI model discussion"))
            .is_err());
        // Late learning cannot recreate support: no entries appear.
        assert!(store.get_learning().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn learning_provenance_cannot_be_forged_or_rewritten() {
        let (directory, store) = temp_store("learning-provenance");
        // New entries with source references are rejected; deliberate manual
        // entries must be sourceless, manual, and pinned.
        let forged = LearnedEntry {
            id: String::new(),
            kind: LearnedKind::Correction,
            text: "Grok".into(),
            heard: Some("grock".into()),
            scope: None,
            source_ids: vec!["rec1".into()],
            source_type: "correction".into(),
            enabled: true,
            user_pinned: false,
        };
        assert!(store.save_learning(&[forged]).is_err());
        let manual = LearnedEntry {
            id: String::new(),
            kind: LearnedKind::Canonical,
            text: "SonoJot".into(),
            heard: Some("sonojot".into()),
            scope: Some("AI model discussion".into()),
            source_ids: Vec::new(),
            source_type: "manual".into(),
            enabled: true,
            user_pinned: true,
        };
        let saved = store.save_learning(&[manual]).unwrap();
        assert_eq!(saved.len(), 1);
        // Provenance fields are immutable once stored.
        let mut tampered = saved[0].clone();
        tampered.source_type = "correction".into();
        assert!(store.save_learning(&[tampered]).is_err());
        let mut tampered = saved[0].clone();
        tampered.user_pinned = false;
        assert!(store.save_learning(&[tampered]).is_err());
        // Text remains editable; reset requires confirmation.
        let mut edited = saved[0].clone();
        edited.text = "SonoJot".into();
        assert!(store.save_learning(&[edited]).is_ok());
        assert!(store.reset_learning(false).is_err());
        store.reset_learning(true).unwrap();
        assert!(store.get_learning().unwrap().is_empty());
        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn new_privacy_settings_default_cleanly_and_migrate_isolated() {
        let (directory, store) = temp_store("privacy-defaults");
        let mut settings = serde_json::to_value(&SettingsFile {
            schema_version: 1,
            settings: Settings::default(),
        })
        .unwrap();
        for field in [
            "doNotSaveDictations",
            "learnFromCorrections",
            "memoryMode",
            "modelRetention",
        ] {
            settings["settings"].as_object_mut().unwrap().remove(field);
        }
        std::fs::write(
            directory.join("settings.json"),
            serde_json::to_vec(&settings).unwrap(),
        )
        .unwrap();
        let loaded = store.load_settings().unwrap();
        assert!(!loaded.do_not_save_dictations);
        assert!(!loaded.learn_from_corrections);
        assert_eq!(loaded.memory_mode, crate::domain::MemoryMode::Overlap);
        assert_eq!(
            loaded.model_retention,
            crate::domain::ModelRetention::Minutes10
        );
        // Isolated legacy import never touches the real profile: separate temp roots.
        let legacy = std::env::temp_dir().join(format!("sonojot-legacy-{}", new_id()));
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("config.json"), b"{}").unwrap();
        let report = store.import_legacy(&legacy).unwrap();
        let repeat = store.import_legacy(&legacy).unwrap();
        assert_eq!(repeat.training_sessions_imported, 0);
        let _ = report;
        std::fs::remove_dir_all(legacy).unwrap();
        std::fs::remove_dir_all(directory).unwrap();
    }

    fn data_parent_fixture(label: &str) -> PathBuf {
        let parent = std::env::temp_dir().join(format!("sonojot-folder-{label}-{}", new_id()));
        fs::create_dir_all(&parent).unwrap();
        parent
    }

    #[test]
    fn previous_data_folder_moves_to_sonojot_and_stored_paths_follow() {
        let parent = data_parent_fixture("old-only");
        let previous = parent.join(PREVIOUS_DATA_DIR_NAME);
        let models = previous.join("models/rewrite");
        fs::create_dir_all(previous.join("dictations")).unwrap();
        fs::create_dir_all(&models).unwrap();
        fs::create_dir_all(previous.join("meetings")).unwrap();
        fs::write(previous.join("dictations/a.json"), b"{\"id\":\"a\"}").unwrap();
        let old = fs::canonicalize(&previous).unwrap();
        let ffmpeg = old.join("tools/ffmpeg-n9/bin/ffmpeg.exe");
        let outside = std::env::temp_dir().join("elsewhere/ffmpeg.exe");
        fs::write(previous.join("converter.json"), serde_json::to_vec(&ffmpeg).unwrap()).unwrap();
        fs::write(
            models.join("paths.json"),
            serde_json::to_vec(&serde_json::json!({
                "cleanup_gguf": old.join("models/rewrite/cleanup/model.gguf"),
                "llama_server_exe": outside,
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(
            models.join(crate::model_verification::CACHE_FILE),
            serde_json::to_vec(&serde_json::json!({
                "version": 1,
                "root": old.join("models/rewrite"),
                "receipts": [{"key": {"root": old.join("models/rewrite"),
                    "path": old.join("models/rewrite/a.onnx"), "sha256": "ab"}}],
            }))
            .unwrap(),
        )
        .unwrap();
        // A meeting file that happens to be named like a path record is content, not touched.
        let meeting_bytes = serde_json::to_vec(&serde_json::json!([old.join("x")])).unwrap();
        fs::write(previous.join("meetings/paths.json"), &meeting_bytes).unwrap();

        let (root, outcome) = adopt_previous_data_folder(&parent, |from, to| fs::rename(from, to));
        assert_eq!(outcome, DataFolder::Moved);
        assert_eq!(root, parent.join(DATA_DIR_NAME));
        assert!(!previous.exists());
        assert_eq!(fs::read(root.join("dictations/a.json")).unwrap(), b"{\"id\":\"a\"}");

        let new = fs::canonicalize(&root).unwrap();
        let converter: PathBuf =
            serde_json::from_slice(&fs::read(root.join("converter.json")).unwrap()).unwrap();
        assert_eq!(converter, new.join("tools/ffmpeg-n9/bin/ffmpeg.exe"));
        let located: Value =
            serde_json::from_slice(&fs::read(root.join("models/rewrite/paths.json")).unwrap()).unwrap();
        assert_eq!(
            Path::new(located["cleanup_gguf"].as_str().unwrap()),
            new.join("models/rewrite/cleanup/model.gguf")
        );
        assert_eq!(Path::new(located["llama_server_exe"].as_str().unwrap()), outside);
        let cache: Value = serde_json::from_slice(
            &fs::read(root.join("models/rewrite").join(crate::model_verification::CACHE_FILE)).unwrap(),
        )
        .unwrap();
        assert_eq!(Path::new(cache["root"].as_str().unwrap()), new.join("models/rewrite"));
        assert_eq!(
            Path::new(cache["receipts"][0]["key"]["path"].as_str().unwrap()),
            new.join("models/rewrite/a.onnx")
        );
        assert_eq!(fs::read(root.join("meetings/paths.json")).unwrap(), meeting_bytes);
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn sonojot_data_folder_alone_is_used_as_is() {
        let parent = data_parent_fixture("new-only");
        let current = parent.join(DATA_DIR_NAME);
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("settings.json"), b"{}").unwrap();
        let (root, outcome) =
            adopt_previous_data_folder(&parent, |_, _| panic!("nothing to move"));
        assert_eq!((root, outcome), (current.clone(), DataFolder::Current));
        assert_eq!(fs::read(current.join("settings.json")).unwrap(), b"{}");
        assert!(!parent.join(PREVIOUS_DATA_DIR_NAME).exists());

        // A first launch with neither folder creates nothing here; Store::open does.
        let empty = data_parent_fixture("neither");
        let (root, outcome) = adopt_previous_data_folder(&empty, |_, _| panic!("nothing to move"));
        assert_eq!((root, outcome), (empty.join(DATA_DIR_NAME), DataFolder::Current));
        assert_eq!(fs::read_dir(&empty).unwrap().count(), 0);
        fs::remove_dir_all(parent).unwrap();
        fs::remove_dir_all(empty).unwrap();
    }

    #[test]
    fn both_data_folders_prefer_sonojot_and_leave_the_old_one_alone() {
        let parent = data_parent_fixture("both");
        let current = parent.join(DATA_DIR_NAME);
        let previous = parent.join(PREVIOUS_DATA_DIR_NAME);
        fs::create_dir_all(&current).unwrap();
        fs::create_dir_all(&previous).unwrap();
        fs::write(current.join("settings.json"), b"new").unwrap();
        fs::write(previous.join("settings.json"), b"old").unwrap();
        let (root, outcome) =
            adopt_previous_data_folder(&parent, |_, _| panic!("must not move over data"));
        assert_eq!((root, outcome), (current.clone(), DataFolder::Current));
        assert_eq!(fs::read(current.join("settings.json")).unwrap(), b"new");
        assert_eq!(fs::read(previous.join("settings.json")).unwrap(), b"old");
        assert_eq!(existing_data_folder(&parent), current);

        // An empty SonoJot folder holds nothing to prefer; the old data moves in.
        fs::remove_file(current.join("settings.json")).unwrap();
        let (root, outcome) = adopt_previous_data_folder(&parent, |from, to| fs::rename(from, to));
        assert_eq!((root, outcome), (current.clone(), DataFolder::Moved));
        assert_eq!(fs::read(current.join("settings.json")).unwrap(), b"old");
        assert!(!previous.exists());
        fs::remove_dir_all(parent).unwrap();
    }

    #[test]
    fn failed_data_folder_move_keeps_using_the_old_folder() {
        let parent = data_parent_fixture("locked");
        let previous = parent.join(PREVIOUS_DATA_DIR_NAME);
        fs::create_dir_all(previous.join("models")).unwrap();
        fs::write(previous.join("settings.json"), b"old").unwrap();
        let (root, outcome) = adopt_previous_data_folder(&parent, |_, _| {
            Err(std::io::Error::from_raw_os_error(32))
        });
        assert_eq!(root, previous);
        assert_eq!(outcome, DataFolder::MoveFailed(Some(32)));
        assert_eq!(fs::read(previous.join("settings.json")).unwrap(), b"old");
        assert!(previous.join("models").is_dir());
        assert!(!parent.join(DATA_DIR_NAME).exists());
        assert_eq!(existing_data_folder(&parent), previous);
        // The store opens the old folder normally for this run.
        let store = Store::open(Some(root)).unwrap();
        assert!(store.root().ends_with(PREVIOUS_DATA_DIR_NAME));
        drop(store);
        fs::remove_dir_all(parent).unwrap();
    }
}
