use serde::{Deserialize, Serialize};

macro_rules! string_enum {
    ($name:ident, $default:ident, {$($variant:ident => $value:literal),+ $(,)?}) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
        pub enum $name { $(#[serde(rename = $value)] $variant),+ }
        impl Default for $name { fn default() -> Self { Self::$default } }
        impl $name { pub fn as_str(self) -> &'static str { match self { $(Self::$variant => $value),+ } } }
        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { f.write_str(self.as_str()) }
        }
    };
}
string_enum!(UpdateChannel, Stable, {Stable => "stable", Nightly => "nightly"});
string_enum!(Theme, Fog, {Fog => "fog", Slate => "slate"});
string_enum!(CleanupMode, Clean, {Clean => "clean", Original => "original", Format => "format"});
string_enum!(SetupProfile, Balanced, {Accurate => "accurate", Balanced => "balanced", Light => "light", Custom => "custom"});
string_enum!(HotkeyMode, Toggle, {Toggle => "toggle", Hold => "hold"});
string_enum!(AppState, Idle, {Idle => "idle", Loading => "loading", Listening => "listening", Finishing => "finishing", Cleaning => "cleaning", Error => "error"});
string_enum!(RecognitionCompleteness, Complete, {Complete => "complete", Incomplete => "incomplete"});
string_enum!(ModelKind, Asr, {Asr => "asr", Cleanup => "cleanup"});
string_enum!(TextAcceleration, Auto, {Auto => "auto", Cpu => "cpu"});
string_enum!(MemoryMode, Overlap, {Overlap => "overlap", Sequential => "sequential"});
string_enum!(ModelRetention, Minutes10, {Immediately => "immediately", Minutes10 => "10m", Minutes30 => "30m", OnExit => "exit"});

// Pending owner confirmation: new installs get Summary; upgrades remain opt-in.
pub const NEW_INSTALL_MEETING_NOTES_PRESET: Option<crate::meeting_ai::NotesPreset> = Some(crate::meeting_ai::NotesPreset::Summary);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct Settings {
    pub terms_accepted_version: u32,
    pub terms_accepted_at: Option<String>,
    pub meeting_consent_acknowledged: bool,
    pub component_acknowledgments: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub setup_profile: Option<SetupProfile>,
    /// One-time punctuation download recommendation, never an enable/disable switch.
    pub punctuation_recommendation_acknowledged: bool,
    /// Separate v2 recommendation; acknowledging v1 does not dismiss the upgrade.
    pub punctuation_v2_recommendation_acknowledged: bool,
    /// Separate v3 recommendation; an earlier v1 or v2 answer does not dismiss it.
    pub punctuation_v3_recommendation_acknowledged: bool,
    pub update_channel: UpdateChannel,
    pub theme: Theme,
    pub material: bool,
    pub cleanup_mode: CleanupMode,
    /// Opt-in structured cleanup trial; applies to Clean only and leaves
    /// As heard and Format only unchanged.
    pub cleanup_trial: bool,
    /// Opt-in layout using a local generation model in Format only and Clean.
    pub smart_layout: bool,
    /// Use a fixed category hint from the capture target when layout runs.
    pub smart_layout_app_hint: bool,
    /// Dictation's one cleanup model is derived from these three keys and
    /// `cleanup_model`; see [`Settings::cleanup_choice`]. `proofreader` picks
    /// the online model in `proofreader_model`.
    pub proofreader: bool,
    /// Use the SonoJot cleanup model whenever one is installed. On by default,
    /// so ours runs as soon as it is present unless another choice was saved.
    pub local_proofreader: bool,
    pub proofreader_model: String,
    /// Training notices accepted for these exact model identifiers only.
    pub proofreader_training_models: Vec<String>,
    /// Conservative written numbers and model names in Clean dictation.
    pub written_form: bool,
    /// Run the newest installed SonoJot punctuation/cleanup package. Off keeps
    /// the deterministic punctuation rules only.
    pub sonojot_punctuation: bool,
    pub launch_at_login: bool,
    pub hotkey: String,
    pub hotkey_mode: HotkeyMode,
    pub auto_paste: bool,
    pub audio_cues: bool,
    /// Show a tiny model's draft beside the primary while dictating. Display
    /// only; it does nothing until the live-preview model is installed.
    pub live_preview: bool,
    pub auto_stop_seconds: u32,
    pub microphone: Option<String>,
    pub asr_model: String,
    pub cleanup_model: String,
    pub notes_model: String,
    pub text_acceleration: TextAcceleration,
    pub start_tab_hidden: bool,
    pub cursor_indicator: bool,
    pub meeting_source: String,
    /// Reset the old one-sided UI default once; subsequent choices are deliberate.
    #[serde(default)]
    pub meeting_source_migrated: bool,
    /// Once system audio first works, a Mac's old forced "microphone" becomes
    /// "both". Later choices are deliberate and stay.
    #[serde(default)]
    pub system_audio_default_applied: bool,
    /// Empty follows Speech (or the separately configured online meeting route).
    /// This override never changes dictation or profile selections.
    pub meeting_asr_model: String,
    pub auto_meeting_titles: bool,
    #[serde(default)]
    pub meeting_notes_preset: Option<crate::meeting_ai::NotesPreset>,
    #[serde(default)]
    pub meeting_notes_instructions: String,
    pub meeting_speaker_labels: bool,
    /// Two-source meetings: remove the other participants' words that the
    /// microphone picked up from loudspeakers. `None` (no choice yet) is
    /// automatic: on when both stems carry word times (SonoJot Parakeet),
    /// off otherwise (Qwen and others: the looser rule can remove words the
    /// user repeats). A saved choice always wins. See `meeting_leak`.
    pub meeting_remove_echo: Option<bool>,
    pub do_not_save_dictations: bool,
    pub keep_dictation_audio: bool,
    pub learn_from_corrections: bool,
    pub memory_mode: MemoryMode,
    pub model_retention: ModelRetention,
    /// Marks for doubtful words after a paste. See `trust`.
    pub trust: TrustSettings,
}

string_enum!(MarkStyle, Band, {Band => "band", Dots => "dots"});
string_enum!(PopoverStyle, Card, {Card => "card", Strip => "strip"});
string_enum!(PlayOn, Hover, {Hover => "hover", Click => "click"});

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TrustSettings {
    pub enabled: bool,
    pub sensitivity: crate::trust::decision::Sensitivity,
    pub mark_style: MarkStyle,
    pub popover_style: PopoverStyle,
    pub play_on: PlayOn,
    /// App identities (as `paste_target_app_identity` reports them) where marks stay off.
    pub off_apps: Vec<String>,
    /// Optional global shortcut for "next doubtful word". Empty is unbound.
    pub next_shortcut: String,
}

impl Default for TrustSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sensitivity: Default::default(),
            mark_style: MarkStyle::Band,
            popover_style: PopoverStyle::Card,
            play_on: PlayOn::Hover,
            off_apps: Vec::new(),
            next_shortcut: String::new(),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            terms_accepted_version: 0,
            terms_accepted_at: None,
            meeting_consent_acknowledged: false,
            component_acknowledgments: Default::default(),
            setup_profile: None,
            punctuation_recommendation_acknowledged: false,
            punctuation_v2_recommendation_acknowledged: false,
            punctuation_v3_recommendation_acknowledged: false,
            update_channel: UpdateChannel::Stable,
            theme: Theme::Fog,
            material: true,
            cleanup_mode: CleanupMode::Clean,
            cleanup_trial: false,
            smart_layout: false,
            smart_layout_app_hint: false,
            proofreader: false,
            local_proofreader: true,
            proofreader_model: "anthropic/claude-opus-5.5".into(),
            proofreader_training_models: Vec::new(),
            written_form: true,
            sonojot_punctuation: true,
            launch_at_login: false,
            hotkey: "Ctrl+Space".into(),
            hotkey_mode: HotkeyMode::Toggle,
            auto_paste: true,
            audio_cues: true,
            live_preview: true,
            auto_stop_seconds: 0,
            microphone: None,
            asr_model: "moonshine-medium".into(),
            // Rules, vocabulary and the punctuation model; generative cleanup is opt-in.
            cleanup_model: "local-rules".into(),
            notes_model: "qwen3-4b-instruct-2507".into(),
            text_acceleration: TextAcceleration::Auto,
            start_tab_hidden: crate::flavor::SYNC_TEST,
            cursor_indicator: true,
            // The legacy app always kept both sides; a one-sided meeting loses the other speakers.
            meeting_source: "both".into(),
            meeting_source_migrated: true,
            system_audio_default_applied: false,
            meeting_asr_model: String::new(),
            auto_meeting_titles: true,
            meeting_notes_preset: NEW_INSTALL_MEETING_NOTES_PRESET,
            meeting_notes_instructions: String::new(),
            meeting_speaker_labels: true,
            meeting_remove_echo: None,
            do_not_save_dictations: false,
            keep_dictation_audio: false,
            learn_from_corrections: false,
            memory_mode: MemoryMode::Overlap,
            model_retention: ModelRetention::Minutes10,
            trust: TrustSettings::default(),
        }
    }
}

/// The one model that cleans dictation after punctuation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CleanupChoice {
    Off,
    /// The SonoJot cleanup model (today the sideloaded proofreader).
    SonoJot,
    /// Another local text model, run through the local cleanup path.
    Local(String),
    /// An OpenRouter model, run through the online proofreader.
    Online(String),
}

impl CleanupChoice {
    /// Settings that save this choice and switch every other one off.
    pub fn apply(&self, settings: &mut Settings) {
        settings.local_proofreader = matches!(self, Self::SonoJot);
        settings.proofreader = matches!(self, Self::Online(_));
        settings.cleanup_model = match self {
            Self::Local(id) => id.clone(),
            _ => "local-rules".into(),
        };
        if let Self::Online(id) = self {
            settings.proofreader_model = id.clone();
        }
    }
}

impl Settings {
    /// Derive the single cleanup choice from the stored keys. Earlier
    /// versions could run a local cleanup model and a proofreader together;
    /// one now wins, in the order the picker recommends them: SonoJot (only
    /// while installed), then online, then another local model.
    pub fn cleanup_choice(&self, sonojot_available: bool) -> CleanupChoice {
        if self.local_proofreader && sonojot_available {
            CleanupChoice::SonoJot
        } else if self.proofreader {
            CleanupChoice::Online(self.proofreader_model.clone())
        } else if !matches!(self.cleanup_model.as_str(), "" | "local-rules" | "off") {
            CleanupChoice::Local(self.cleanup_model.clone())
        } else {
            CleanupChoice::Off
        }
    }

    pub fn validate(&self) -> Result<(), String> {
        crate::meeting_ai::validate_notes_instructions(self.meeting_notes_preset, &self.meeting_notes_instructions)?;
        if !self.meeting_asr_model.is_empty()
            && self.meeting_asr_model != crate::granite_model::GRANITE_ID
        {
            return Err("Choose a meeting transcription model.".into());
        }
        if [&self.asr_model, &self.cleanup_model, &self.notes_model]
            .iter().any(|id| id.as_str() == crate::granite_model::GRANITE_ID)
        {
            return Err("Granite Speech is available for meetings only.".into());
        }
        if !matches!(
            self.meeting_source.as_str(),
            "microphone" | "system" | "both"
        ) {
            return Err("Choose a meeting audio source.".into());
        }
        validate_hotkey(&self.hotkey)?;
        if !self.trust.next_shortcut.is_empty() {
            validate_hotkey(&self.trust.next_shortcut)?;
            if self.trust.next_shortcut.replace(' ', "").eq_ignore_ascii_case(&self.hotkey.replace(' ', "")) {
                return Err("Pick a different shortcut from the dictation shortcut.".into());
            }
        }
        if self.trust.off_apps.len() > 200 || self.trust.off_apps.iter().any(|app| app.len() > 260) {
            return Err("Too many apps in the doubtful words list.".into());
        }
        if self.auto_stop_seconds > 300 {
            return Err("Auto-stop must be between 0 and 300 seconds.".into());
        }
        if !crate::ai::valid_model(&self.proofreader_model)
            || self.proofreader_training_models.len() > 4000
            || self.proofreader_training_models.iter().any(|id| !crate::ai::valid_model(id))
        {
            return Err("Choose a proofreader model from the OpenRouter catalog.".into());
        }
        for value in [&self.asr_model, &self.cleanup_model, &self.notes_model] {
            if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
                return Err("Choose a valid model identifier.".into());
            }
        }
        if self.cleanup_model == "speakoflow-mini-q4_k_m"
            && self.setup_profile != Some(SetupProfile::Custom)
        {
            return Err("Choose Custom before selecting Smart cleanup.".into());
        }
        if self.notes_model == "speakoflow-mini-q4_k_m" {
            return Err("Smart cleanup is available for dictation cleanup only.".into());
        }
        if self.memory_mode == MemoryMode::Overlap
            && self.model_retention == ModelRetention::Immediately
        {
            return Err(
                "Keep speech and cleanup ready requires a non-immediate release setting.".into(),
            );
        }
        if self
            .microphone
            .as_ref()
            .is_some_and(|v| v.len() > 1024 || v.contains('\0'))
        {
            return Err("The microphone identifier is invalid.".into());
        }
        Ok(())
    }
}

/// Check both saved settings and startup registration, including older settings files.
pub fn validate_hotkey(value: &str) -> Result<(), String> {
    let modifier = |part: &str| matches!(part,
        "ctrl" | "control" | "alt" | "option" | "super" | "meta" | "cmd" | "command"
        | "cmdorctrl" | "cmdorcontrol" | "commandorctrl" | "commandorcontrol" | "shift");
    let parts: Vec<String> = value.split('+').map(|part| part.trim().to_ascii_lowercase()).collect();
    let key = parts.last().map(String::as_str).unwrap_or("");
    let modifiers = &parts[..parts.len().saturating_sub(1)];
    let function_key = key.strip_prefix('f').and_then(|n| n.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n) && key == format!("f{n}"));
    if value.len() > 128 || value.chars().any(char::is_control) || key.is_empty()
        || modifier(key) || !modifiers.iter().all(|part| modifier(part))
        || !(function_key || modifiers.iter().any(|part| part != "shift"))
    {
        return Err("Use Ctrl, Alt or Super with a key, or F1–F24.".into());
    }
    Ok(())
}

#[cfg(test)]
mod input_validation_tests {
    use super::*;

    #[test]
    fn shortcuts_reject_bare_typing_and_navigation_keys_at_both_boundaries() {
        for key in ["a", "A", "Tab", "Enter", "Space", "ArrowLeft", "Shift+A", "Shift+Tab", "Ctrl", "Ctrl+", "+A", "F0", "F25", "Ctrl+\nA"] {
            assert!(validate_hotkey(key).is_err(), "{key}");
            assert!(Settings { hotkey: key.into(), ..Settings::default() }.validate().is_err(), "{key}");
        }
        for key in ["Ctrl+Space", "Alt+Shift+D", "Super+A", "CommandOrControl+K", "F1", "F24", "Shift+F10"] {
            assert!(validate_hotkey(key).is_ok(), "{key}");
            assert!(Settings { hotkey: key.into(), ..Settings::default() }.validate().is_ok(), "{key}");
        }
    }

    #[test]
    fn saved_notes_enforce_character_limit_and_custom_instructions() {
        for unit in ["a", "界", "😀"] {
            let mut settings = Settings { meeting_notes_preset: Some(crate::meeting_ai::NotesPreset::Custom),
                meeting_notes_instructions: unit.repeat(2000), ..Settings::default() };
            assert!(settings.validate().is_ok());
            settings.meeting_notes_instructions.push_str(unit);
            assert!(settings.validate().is_err());
        }
        let mut settings = Settings { meeting_notes_preset: Some(crate::meeting_ai::NotesPreset::Custom),
            meeting_notes_instructions: " \n ".into(), ..Settings::default() };
        assert!(settings.validate().is_err());
        settings.meeting_notes_preset = None;
        assert!(settings.validate().is_ok());
        settings.meeting_notes_instructions = "a\0b".into();
        assert!(settings.validate().is_err());
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    pub id: String,
    pub name: String,
    pub kind: ModelKind,
    pub ready: bool,
    /// Verification state; `ready` remains false while checking.
    #[serde(default)]
    pub state: String,
    pub description: String,
    pub size_bytes: u64,
    #[serde(default)]
    pub installable: bool,
    #[serde(default)]
    pub removable: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Capabilities {
    #[serde(default)]
    pub sync: bool,
    pub auto_paste: bool,
    /// True only when a meeting can record system audio right now.
    pub system_audio: bool,
    #[serde(default)]
    pub system_audio_status: crate::system_audio::SystemAudioStatus,
    pub global_shortcut: bool,
    pub platform: String,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    #[serde(skip)]
    pub capture_generation: Option<u64>,
    #[serde(default)]
    pub local_proofreader_available: bool,
    pub revision: u64,
    #[serde(default)]
    pub meetings_revision: u64,
    pub session_id: Option<u64>,
    pub tab_visible: bool,
    pub tab_expanded: bool,
    pub tab_temporary: bool,
    pub can_copy_last: bool,
    pub delivery_notice: Option<String>,
    pub result_session_id: Option<u64>,
    pub recognition_completeness: Option<RecognitionCompleteness>,
    pub recognition_warnings: Vec<String>,
    pub settings: Settings,
    #[serde(default)]
    pub recognition_progress: Option<RecognitionProgress>,
    pub state: AppState,
    pub original_text: String,
    pub output_text: String,
    #[serde(default)]
    pub pre_model_text: Option<String>,
    #[serde(default)]
    pub publication_stage: Option<String>,
    #[serde(default)]
    pub guard_reason: Option<String>,
    #[serde(default)]
    pub cleanup_status: Option<String>,
    pub interim_text: String,
    /// Live-preview draft for audio the primary has not committed yet.
    /// Display only: never saved, published, copied or used as a fallback.
    #[serde(default)]
    pub preview_text: String,
    pub audio_level: f32,
    /// Microphone and system levels while a two-source meeting records.
    #[serde(default)]
    pub meeting_levels: Option<MeetingLevels>,
    /// Recoverable source notices, separate from fatal capture errors.
    #[serde(default)]
    pub meeting_source_status: Vec<String>,
    /// "permission", "lost" or "unsupported" while the other side is missing.
    #[serde(default)]
    pub meeting_system_audio: Option<String>,
    pub error: Option<String>,
    pub models: Vec<Model>,
    pub devices: Vec<Device>,
    pub meeting_recording: bool,
    pub active_meeting_id: Option<String>,
    pub capabilities: Capabilities,
    pub version: String,
    #[serde(default)]
    pub update: crate::updater::UpdateSnapshot,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingSummary {
    pub id: String,
    pub title: String,
    pub created_at: String,
    pub duration_seconds: f64,
    pub status: String,
    pub has_audio: bool,
    pub has_transcript: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioOutage {
    pub source: String,
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeetingDetail {
    #[serde(default)]
    pub recognition_completeness: RecognitionCompleteness,
    #[serde(default)]
    pub recognition_warnings: Vec<String>,
    #[serde(default)]
    pub recording_warnings: Vec<String>,
    #[serde(default)]
    pub audio_outages: Vec<AudioOutage>,
    #[serde(default)]
    pub missing_seconds: f64,
    #[serde(default)]
    pub cleaned_transcript: Option<String>,
    #[serde(default)]
    pub transcription_stopped_at_seconds: Option<f64>,
    #[serde(flatten)]
    pub summary: MeetingSummary,
    pub transcript: String,
    pub notes: String,
    pub source: String,
    pub audio_path: Option<String>,
    pub error: Option<String>,
    /// Speech model that produced the current transcript; absent on older records.
    #[serde(default)]
    pub transcription_model: Option<String>,
    /// Distinct voices numbered by speaker labeling in the current recognition.
    #[serde(default)]
    pub speaker_count: u32,
    /// Whether the transcript shows those numbered voices (per meeting).
    #[serde(default = "speaker_labels_default")]
    pub speaker_labels: bool,
}

fn speaker_labels_default() -> bool {
    true
}

impl std::ops::Deref for MeetingDetail {
    type Target = MeetingSummary;
    fn deref(&self) -> &Self::Target {
        &self.summary
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct MeetingLevels {
    pub microphone: f32,
    pub system: f32,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyEntry {
    #[serde(default)]
    pub kind: VocabularyKind,
    pub id: String,
    pub heard: String,
    pub written: String,
    pub enabled: bool,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub context: Vec<String>,
    /// Optional scope for the structured cleanup trial, e.g. "AI model
    /// discussion". An entry with a non-empty scope is an explicit user
    /// confirmation applied only where surrounding words support it.
    #[serde(default)]
    pub scope: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum VocabularyKind {
    #[default]
    Replace,
    Hint,
}

fn default_enabled_true() -> bool {
    true
}

fn default_source_type() -> String {
    "dictationCorrection".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DictationRecord {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub word_confidence: Vec<crate::confidence::WordConfidence>,
    pub id: String,
    pub created_at: String,
    pub raw: String,
    pub published: String,
    /// Local app ID captured at start: the intended paste target, even if
    /// delivery is disabled, fails or is withheld for incomplete recognition.
    #[serde(default)]
    pub target_app: Option<String>,
    #[serde(default)]
    pub pre_model_text: Option<String>,
    #[serde(default)]
    pub publication_stage: Option<String>,
    #[serde(default)]
    pub guard_reason: Option<String>,
    #[serde(default)]
    pub cleanup_status: Option<String>,
    #[serde(default)]
    pub recognition_completeness: RecognitionCompleteness,
    #[serde(default)]
    pub recognition_warnings: Vec<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecognitionProgress {
    pub stage: String,
    pub elapsed_ms: u64,
}

string_enum!(LearnedKind, Correction, {Correction => "correction", Canonical => "canonical", Context => "context", Style => "style"});

impl LearnedKind {
    pub fn is_lexical(self) -> bool {
        matches!(self, Self::Correction | Self::Canonical)
    }
}

/// Back-compat alias for earlier privacy draft naming.
pub type LearnKind = LearnedKind;

/// Small source-traceable learned entry. Concise facts/corrections only;
/// never a whole transcript copy. `text` is the canonical/written form,
/// `heard` the observed alias when applicable. `source_ids` lists supporting
/// dictation record IDs; entries with emptied support are pruned. Only
/// independently authored manual entries may be pinned. Explicit vocabulary is
/// never pruned by history deletion.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct LearnedEntry {
    pub id: String,
    #[serde(default)]
    pub kind: LearnedKind,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub heard: Option<String>,
    #[serde(default)]
    pub scope: Option<String>,
    #[serde(default)]
    pub source_ids: Vec<String>,
    #[serde(default = "default_source_type")]
    pub source_type: String,
    #[serde(default = "default_enabled_true")]
    pub enabled: bool,
    #[serde(default)]
    pub user_pinned: bool,
}

impl Default for LearnedEntry {
    fn default() -> Self {
        Self {
            id: String::new(),
            kind: LearnedKind::Correction,
            text: String::new(),
            heard: None,
            scope: None,
            source_ids: Vec::new(),
            source_type: default_source_type(),
            enabled: true,
            user_pinned: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedCorrection {
    pub record: DictationRecord,
    pub learned_count: usize,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeletedDictations {
    pub deleted_count: usize,
    pub snapshot: Snapshot,
}

/// Alias names used by the Round 6 contract drafts; identical wire shape.
pub type DeleteAllResult = DeletedDictations;
pub type CorrectionSaveResult = SavedCorrection;

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MigrationReport {
    pub settings_imported: bool,
    pub vocabulary_imported: usize,
    pub meetings_imported: usize,
    pub training_sessions_imported: usize,
    pub warnings: Vec<String>,
}

#[cfg(test)]
mod meeting_warning_tests {
    use super::{MeetingDetail, Snapshot};

    #[test]
    fn legacy_meetings_default_recording_warnings_and_round_trip_camel_case() {
        let legacy = serde_json::json!({
            "id": "legacy-meeting", "title": "Saved meeting",
            "createdAt": "2026-09-28T12:00:00Z", "durationSeconds": 0.0,
            "status": "failed", "hasAudio": true, "hasTranscript": false,
            "transcript": "", "notes": "", "source": "both",
            "audioPath": null, "error": "System capture reported lost audio."
        });
        let mut detail: MeetingDetail = serde_json::from_value(legacy).unwrap();
        assert!(detail.recording_warnings.is_empty());
        detail.recording_warnings = vec!["[0:12] Microphone lost; reconnected at 0:15.".into()];
        let saved = serde_json::to_value(&detail).unwrap();
        assert_eq!(saved["recordingWarnings"][0], detail.recording_warnings[0]);
        assert!(saved.get("recording_warnings").is_none());
        let restored: MeetingDetail = serde_json::from_value(saved).unwrap();
        assert_eq!(restored.recording_warnings, detail.recording_warnings);
        assert!(restored.recognition_warnings.is_empty());
    }

    #[test]
    fn legacy_snapshots_default_source_status_and_keep_notices_separate_from_errors() {
        let mut legacy = serde_json::to_value(Snapshot::default()).unwrap();
        legacy.as_object_mut().unwrap().remove("meetingSourceStatus");
        let mut snapshot: Snapshot = serde_json::from_value(legacy).unwrap();
        assert!(snapshot.meeting_source_status.is_empty());
        snapshot.meeting_source_status = vec!["Microphone lost — reconnecting".into()];
        let saved = serde_json::to_value(&snapshot).unwrap();
        assert_eq!(saved["meetingSourceStatus"][0], snapshot.meeting_source_status[0]);
        assert!(snapshot.error.is_none());
    }
}

#[cfg(test)]
mod setup_profile_tests {
    use super::{Settings, SetupProfile};

    #[test]
    fn punctuation_recommendation_migrates_unacknowledged_and_round_trips() {
        for profile in [None, Some(SetupProfile::Custom), Some(SetupProfile::Accurate),
            Some(SetupProfile::Balanced), Some(SetupProfile::Light)] {
            let legacy = serde_json::json!({ "setupProfile": profile });
            let mut settings: Settings = serde_json::from_value(legacy).unwrap();
            assert!(!settings.punctuation_recommendation_acknowledged);
            for acknowledged in [false, true] {
                settings.punctuation_recommendation_acknowledged = acknowledged;
                let value = serde_json::to_value(&settings).unwrap();
                assert_eq!(value["punctuationRecommendationAcknowledged"], acknowledged);
                assert_eq!(serde_json::from_value::<Settings>(value).unwrap(), settings);
            }
        }
    }

    #[test]
    fn punctuation_v2_recommendation_migrates_independently_and_round_trips() {
        assert!(!Settings::default().punctuation_v2_recommendation_acknowledged);
        for profile in [None, Some(SetupProfile::Custom), Some(SetupProfile::Accurate),
            Some(SetupProfile::Balanced), Some(SetupProfile::Light)] {
            for legacy_acknowledged in [None, Some(false), Some(true)] {
                let mut legacy = serde_json::json!({ "setupProfile": profile });
                if let Some(acknowledged) = legacy_acknowledged {
                    legacy["punctuationRecommendationAcknowledged"] = acknowledged.into();
                }
                let mut settings: Settings = serde_json::from_value(legacy).unwrap();
                assert_eq!(settings.punctuation_recommendation_acknowledged, legacy_acknowledged.unwrap_or(false));
                assert!(!settings.punctuation_v2_recommendation_acknowledged);
                for acknowledged in [false, true] {
                    settings.punctuation_v2_recommendation_acknowledged = acknowledged;
                    let value = serde_json::to_value(&settings).unwrap();
                    assert_eq!(value["punctuationV2RecommendationAcknowledged"], acknowledged);
                    assert_eq!(value["punctuationRecommendationAcknowledged"], legacy_acknowledged.unwrap_or(false));
                    assert!(value.get("punctuation_v2_recommendation_acknowledged").is_none());
                    assert_eq!(serde_json::from_value::<Settings>(value).unwrap(), settings);
                }
            }
        }
    }

    #[test]
    fn punctuation_v3_recommendation_migrates_independently_and_round_trips() {
        assert!(!Settings::default().punctuation_v3_recommendation_acknowledged);
        for v1 in [None, Some(false), Some(true)] {
            for v2 in [None, Some(false), Some(true)] {
                let mut legacy = serde_json::json!({ "setupProfile": SetupProfile::Balanced });
                if let Some(value) = v1 { legacy["punctuationRecommendationAcknowledged"] = value.into(); }
                if let Some(value) = v2 { legacy["punctuationV2RecommendationAcknowledged"] = value.into(); }
                let mut settings: Settings = serde_json::from_value(legacy).unwrap();
                assert_eq!(settings.punctuation_recommendation_acknowledged, v1.unwrap_or(false));
                assert_eq!(settings.punctuation_v2_recommendation_acknowledged, v2.unwrap_or(false));
                assert!(!settings.punctuation_v3_recommendation_acknowledged, "earlier answers never dismiss v3");
                for acknowledged in [false, true] {
                    settings.punctuation_v3_recommendation_acknowledged = acknowledged;
                    let value = serde_json::to_value(&settings).unwrap();
                    assert_eq!(value["punctuationV3RecommendationAcknowledged"], acknowledged);
                    assert_eq!(value["punctuationV2RecommendationAcknowledged"], v2.unwrap_or(false));
                    assert!(value.get("punctuation_v3_recommendation_acknowledged").is_none());
                    assert_eq!(serde_json::from_value::<Settings>(value).unwrap(), settings);
                }
            }
        }
    }

    #[test]
    fn smart_cleanup_requires_custom_and_explicit_opt_in() {
        assert_ne!(serde_json::from_str::<Settings>("{}").unwrap().cleanup_model, "speakoflow-mini-q4_k_m");
        for profile in [None, Some(SetupProfile::Accurate), Some(SetupProfile::Balanced),
            Some(SetupProfile::Light), Some(SetupProfile::Custom)] {
                let settings = Settings { setup_profile: profile,
                    cleanup_model: "speakoflow-mini-q4_k_m".into(), ..Settings::default() };
                assert_eq!(settings.validate().is_ok(), profile == Some(SetupProfile::Custom));
        }
        let notes = Settings { notes_model: "speakoflow-mini-q4_k_m".into(), ..Settings::default() };
        assert!(notes.validate().is_err());
    }

    #[test]
    fn profile_serde_preserves_null_and_all_saved_choices() {
        for profile in [None, Some(SetupProfile::Accurate), Some(SetupProfile::Balanced),
            Some(SetupProfile::Light), Some(SetupProfile::Custom)] {
            let settings = Settings { setup_profile: profile, ..Settings::default() };
            let value = serde_json::to_value(&settings).unwrap();
            assert_eq!(value["setupProfile"], serde_json::to_value(profile).unwrap());
            assert!(value.get("setup_profile").is_none());
            assert_eq!(serde_json::from_value::<Settings>(value).unwrap(), settings);
        }
        assert_eq!(serde_json::from_str::<Settings>("{}").unwrap().setup_profile, None);
    }
}

#[cfg(test)]
mod smart_layout_settings_tests {
    use super::{CleanupMode, Settings, SetupProfile};

    #[test]
    fn written_form_defaults_on_and_round_trips() {
        assert!(Settings::default().written_form);
        assert!(serde_json::from_str::<Settings>("{}").unwrap().written_form);
        for enabled in [false, true] {
            let settings = Settings { written_form: enabled, ..Settings::default() };
            let saved = serde_json::to_value(&settings).unwrap();
            assert_eq!(saved["writtenForm"], enabled);
            assert_eq!(serde_json::from_value::<Settings>(saved).unwrap(), settings);
        }
        assert!(serde_json::from_str::<Settings>(r#"{"writtenForm":"true"}"#).is_err());
    }

    #[test]
    fn smart_layout_defaults_off_without_changing_legacy_choices() {
        assert!(!Settings::default().smart_layout);
        assert!(!Settings::default().smart_layout_app_hint);
        assert!(!serde_json::from_str::<Settings>("{}").unwrap().smart_layout);
        assert!(!serde_json::from_str::<Settings>("{}").unwrap().smart_layout_app_hint);
        let legacy = serde_json::json!({
            "setupProfile": "custom",
            "cleanupMode": "format",
            "cleanupModel": "local-rules",
            "notesModel": "openrouter"
        });
        let settings: Settings = serde_json::from_value(legacy).unwrap();
        assert_eq!(
            settings,
            Settings {
                setup_profile: Some(SetupProfile::Custom),
                cleanup_mode: CleanupMode::Format,
                cleanup_model: "local-rules".into(),
                notes_model: "openrouter".into(),
                meeting_source_migrated: false,
                meeting_notes_preset: None,
                ..Settings::default()
            }
        );
        assert!(!settings.smart_layout);
        assert!(!settings.smart_layout_app_hint);
    }

    #[test]
    fn app_hint_round_trips_even_when_layout_is_inactive() {
        for smart_layout_app_hint in [false, true] {
            for smart_layout in [false, true] {
                for cleanup_mode in [CleanupMode::Original, CleanupMode::Format, CleanupMode::Clean] {
                    let settings = Settings {
                        smart_layout_app_hint,
                        smart_layout,
                        cleanup_mode,
                        cleanup_model: "local-rules".into(),
                        notes_model: "off".into(),
                        ..Settings::default()
                    };
                    assert!(settings.validate().is_ok());
                    let saved = serde_json::to_value(&settings).unwrap();
                    assert_eq!(saved["smartLayoutAppHint"], smart_layout_app_hint);
                    assert!(saved.get("smart_layout_app_hint").is_none());
                    assert_eq!(serde_json::from_value::<Settings>(saved).unwrap(), settings);
                }
            }
        }
        for value in [serde_json::json!("true"), serde_json::json!(1), serde_json::Value::Null] {
            assert!(serde_json::from_value::<Settings>(serde_json::json!({
                "smartLayoutAppHint": value
            })).is_err());
        }
    }

    #[test]
    fn smart_layout_round_trips_both_choices_even_when_inactive() {
        for smart_layout in [false, true] {
            for cleanup_mode in [
                CleanupMode::Original,
                CleanupMode::Format,
                CleanupMode::Clean,
            ] {
                let settings = Settings {
                    smart_layout,
                    cleanup_mode,
                    cleanup_model: "local-rules".into(),
                    notes_model: "openrouter".into(),
                    ..Settings::default()
                };
                assert!(settings.validate().is_ok());
                let saved = serde_json::to_value(&settings).unwrap();
                assert_eq!(saved["smartLayout"], smart_layout);
                assert!(saved.get("smart_layout").is_none());
                assert_eq!(serde_json::from_value::<Settings>(saved).unwrap(), settings);
            }
        }
    }

    #[test]
    fn smart_layout_rejects_non_boolean_values() {
        for value in [
            serde_json::json!("true"),
            serde_json::json!(1),
            serde_json::Value::Null,
        ] {
            assert!(
                serde_json::from_value::<Settings>(serde_json::json!({ "smartLayout": value }))
                    .is_err()
            );
        }
    }
}

#[cfg(test)]
mod proofreader_settings_tests {
    use super::Settings;

    #[test]
    fn legacy_settings_default_to_disabled_and_round_trip_exact_consent() {
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert!(!legacy.proofreader);
        assert_eq!(legacy.proofreader_model, "anthropic/claude-opus-5.5");
        assert!(legacy.proofreader_training_models.is_empty());
        for model in ["openai/gpt-6-luna", "anthropic/claude-haiku-5.5"] {
            let saved: Settings = serde_json::from_value(serde_json::json!({"proofreaderModel": model})).unwrap();
            assert_eq!(saved.proofreader_model, model);
        }
        for enabled in [false, true] {
            let settings = Settings {
                proofreader: enabled,
                proofreader_model: "other/contributor".into(),
                proofreader_training_models: vec!["other/contributor".into()],
                ..Settings::default()
            };
            assert!(settings.validate().is_ok());
            let saved = serde_json::to_value(&settings).unwrap();
            assert_eq!(saved["proofreader"], enabled);
            assert_eq!(saved["proofreaderModel"], "other/contributor");
            assert_eq!(saved["proofreaderTrainingModels"], serde_json::json!(["other/contributor"]));
            assert_eq!(serde_json::from_value::<Settings>(saved).unwrap(), settings);
        }
    }

    #[test]
    fn proofreader_settings_reject_invalid_types_and_identifiers() {
        for invalid in [serde_json::json!({"proofreader": "true"}),
            serde_json::json!({"proofreaderModel": null}),
            serde_json::json!({"proofreaderTrainingModels": true})]
        {
            assert!(serde_json::from_value::<Settings>(invalid).is_err());
        }
        let mut settings = Settings::default();
        settings.proofreader_model = "bad-model".into();
        assert!(settings.validate().is_err());
        settings.proofreader_model = "provider/model".into();
        settings.proofreader_training_models.push("https://host/model".into());
        assert!(settings.validate().is_err());
    }
}

#[cfg(test)]
mod cleanup_choice_tests {
    use super::{CleanupChoice, Settings};

    fn legacy(cleanup: &str, proofreader: bool, local: bool) -> Settings {
        Settings {
            cleanup_model: cleanup.into(),
            proofreader,
            local_proofreader: local,
            proofreader_model: "anthropic/claude-haiku-5.5".into(),
            ..Settings::default()
        }
    }

    /// Every combination the earlier Cleanup, Smart cleanup and Proofreading
    /// controls could save, with and without the SonoJot model installed.
    #[test]
    fn every_legacy_combination_maps_to_one_choice() {
        let online = || CleanupChoice::Online("anthropic/claude-haiku-5.5".into());
        let local = |id: &str| CleanupChoice::Local(id.into());
        let smart = crate::optional_llm::SMART_CLEANUP_ID;
        let cases: [(&str, bool, bool, bool, CleanupChoice); 16] = [
            ("local-rules", false, false, false, CleanupChoice::Off),
            ("local-rules", false, false, true, CleanupChoice::Off),
            ("local-rules", false, true, false, CleanupChoice::Off),
            ("local-rules", false, true, true, CleanupChoice::SonoJot),
            ("local-rules", true, false, false, online()),
            ("local-rules", true, false, true, online()),
            ("local-rules", true, true, false, online()),
            ("local-rules", true, true, true, CleanupChoice::SonoJot),
            (smart, false, false, false, local(smart)),
            (smart, false, true, true, CleanupChoice::SonoJot),
            (smart, true, false, false, online()),
            ("qwen3-4b-instruct-2507", false, false, true, local("qwen3-4b-instruct-2507")),
            ("qwen3-4b-instruct-2507", true, false, true, online()),
            ("qwen3-4b-instruct-2507", false, true, true, CleanupChoice::SonoJot),
            ("lfm2.5-1.2b", false, true, false, local("lfm2.5-1.2b")),
            ("off", false, false, false, CleanupChoice::Off),
        ];
        for (cleanup, proofreader, local_flag, available, expected) in cases {
            assert_eq!(
                legacy(cleanup, proofreader, local_flag).cleanup_choice(available),
                expected,
                "{cleanup} proofreader={proofreader} local={local_flag} available={available}"
            );
        }
    }

    #[test]
    fn fresh_settings_use_sonojot_once_it_is_installed() {
        let fresh: Settings = serde_json::from_str("{}").unwrap();
        assert!(fresh.local_proofreader);
        assert_eq!(fresh.cleanup_choice(false), CleanupChoice::Off);
        assert_eq!(fresh.cleanup_choice(true), CleanupChoice::SonoJot);
        assert_eq!(Settings::default().cleanup_choice(true), CleanupChoice::SonoJot);
    }

    #[test]
    fn saved_choices_round_trip_and_switch_the_others_off() {
        let choices = [
            CleanupChoice::Off,
            CleanupChoice::SonoJot,
            CleanupChoice::Local("lfm2.5-1.2b".into()),
            CleanupChoice::Online("anthropic/claude-opus-5.5".into()),
        ];
        for start in [legacy("qwen3-4b-instruct-2507", true, true), Settings::default()] {
            for choice in &choices {
                let mut settings = start.clone();
                choice.apply(&mut settings);
                assert_eq!(&settings.cleanup_choice(true), choice);
                // An explicit Off or another choice never turns into SonoJot
                // when the model is installed later.
                if *choice != CleanupChoice::SonoJot {
                    assert_eq!(&settings.cleanup_choice(false), choice);
                } else {
                    assert_eq!(settings.cleanup_choice(false), CleanupChoice::Off);
                }
                let saved: Settings =
                    serde_json::from_value(serde_json::to_value(&settings).unwrap()).unwrap();
                assert_eq!(saved, settings);
            }
        }
    }

    #[test]
    fn punctuation_switch_defaults_on_and_round_trips() {
        assert!(Settings::default().sonojot_punctuation);
        assert!(serde_json::from_str::<Settings>("{}").unwrap().sonojot_punctuation);
        for on in [false, true] {
            let settings = Settings { sonojot_punctuation: on, ..Settings::default() };
            let saved = serde_json::to_value(&settings).unwrap();
            assert_eq!(saved["sonojotPunctuation"], on);
            assert_eq!(serde_json::from_value::<Settings>(saved).unwrap(), settings);
        }
        assert!(serde_json::from_str::<Settings>(r#"{"sonojotPunctuation":"yes"}"#).is_err());
    }
}

#[cfg(test)]
mod speaker_label_settings_tests {
    use super::Settings;

    #[test]
    fn granite_selection_defaults_to_speech_and_is_meeting_only() {
        let mut settings: Settings = serde_json::from_str("{}").unwrap();
        assert!(settings.meeting_asr_model.is_empty());
        settings.meeting_asr_model = crate::granite_model::GRANITE_ID.into();
        assert!(settings.validate().is_ok());
        let json = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&json).unwrap(), settings);
        settings.asr_model = crate::granite_model::GRANITE_ID.into();
        assert!(settings.validate().unwrap_err().contains("meetings only"));
        settings.asr_model = Settings::default().asr_model;
        settings.meeting_asr_model = "unknown-meeting-model".into();
        assert!(settings.validate().is_err());
    }

    #[test]
    fn speaker_echo_removal_is_automatic_until_chosen() {
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert_eq!(legacy.meeting_remove_echo, None);
        assert_eq!(serde_json::to_value(&legacy).unwrap()["meetingRemoveEcho"], serde_json::Value::Null);
        for choice in [true, false] {
            let saved: Settings = serde_json::from_str(&format!(r#"{{"meetingRemoveEcho":{choice}}}"#)).unwrap();
            assert_eq!(saved.meeting_remove_echo, Some(choice));
            assert_eq!(serde_json::to_value(saved).unwrap()["meetingRemoveEcho"], choice);
        }
    }

    #[test]
    fn speaker_labels_default_on_and_preserve_saved_opt_out() {
        let legacy: Settings = serde_json::from_str("{}").unwrap();
        assert!(legacy.meeting_speaker_labels);
        assert!(Settings::default().meeting_speaker_labels);
        let saved: Settings = serde_json::from_str(r#"{"meetingSpeakerLabels":false}"#).unwrap();
        assert!(!saved.meeting_speaker_labels);
        assert_eq!(
            serde_json::to_value(saved).unwrap()["meetingSpeakerLabels"],
            false
        );
    }
}
