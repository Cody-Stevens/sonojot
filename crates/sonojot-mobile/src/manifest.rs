//! Model packs: one directory per model, described by `manifest.json`, which pins every file's
//! size and SHA-256. Packs are data only; the sherpa-onnx runtime ships inside the app.
//!
//! ```json
//! {
//!   "schema": 1,
//!   "id": "whisper-base-multilingual-int8",
//!   "version": "1",
//!   "family": "whisper",
//!   "languages": ["*"],
//!   "runtime": { "kind": "sherpa-onnx", "min_version": "1.13.8" },
//!   "files": [
//!     { "role": "encoder", "path": "base-encoder.int8.onnx", "bytes": 28734567, "sha256": "…" },
//!     { "role": "decoder", "path": "base-decoder.int8.onnx", "bytes": 129834567, "sha256": "…" },
//!     { "role": "tokens",  "path": "base-tokens.txt",        "bytes": 816730,    "sha256": "…" }
//!   ]
//! }
//! ```
//!
//! Like SonoJot's desktop verifier, status checks only compare sizes ([`VerifyMode::Sizes`]);
//! loading hashes every file ([`VerifyMode::Full`]) before native code reads it.

use crate::{sherpa::SHERPA_ONNX_ABI, EngineError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const MANIFEST_FILE: &str = "manifest.json";
/// Manifest format version. Additive optional fields keep it; anything an older engine would
/// misread bumps it.
pub const MANIFEST_SCHEMA: u32 = 1;
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// The sherpa-onnx offline model families the mobile runner supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    /// Whisper encoder/decoder (`.en` checkpoints or multilingual).
    Whisper,
    /// SenseVoice CTC (zh, en, ja, ko, yue).
    SenseVoice,
    /// Transducers: Zipformer (`transducer`) or NeMo Parakeet (`nemo_transducer`).
    Transducer,
    /// Moonshine v1 (preprocessor, encoder, uncached and cached decoders) or v2 (merged decoder).
    Moonshine,
}

impl ModelFamily {
    fn required_roles(self, roles: &HashSet<&str>) -> Vec<&'static str> {
        match self {
            Self::Whisper => vec!["encoder", "decoder", "tokens"],
            Self::SenseVoice => vec!["model", "tokens"],
            Self::Transducer => vec!["encoder", "decoder", "joiner", "tokens"],
            Self::Moonshine if roles.contains("merged_decoder") => vec!["encoder", "merged_decoder", "tokens"],
            Self::Moonshine => vec!["preprocessor", "encoder", "uncached_decoder", "cached_decoder", "tokens"],
        }
    }

    fn known_roles(self) -> &'static [&'static str] {
        match self {
            Self::Whisper => &["encoder", "decoder", "tokens"],
            Self::SenseVoice => &["model", "tokens"],
            Self::Transducer => &["encoder", "decoder", "joiner", "tokens"],
            Self::Moonshine => {
                &["preprocessor", "encoder", "uncached_decoder", "cached_decoder", "merged_decoder", "tokens"]
            }
        }
    }
}

/// Which runtime the pack needs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeRequirement {
    /// Always `sherpa-onnx` for schema 1.
    pub kind: String,
    /// Oldest sherpa-onnx release that reads these files.
    pub min_version: String,
}

/// Model settings that cannot be inferred from the files.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModelOptions {
    /// Transducers: `transducer` (Zipformer, the default) or `nemo_transducer` (Parakeet).
    pub model_type: Option<String>,
    /// Whisper or SenseVoice decoder language, overriding the language of the request.
    pub language: Option<String>,
    /// Feature dimension (80 by default; 128 for Parakeet).
    pub feature_dim: Option<i32>,
    /// SenseVoice inverse text normalization (on by default).
    pub use_itn: Option<bool>,
    /// The model's longest input window; caps the gate's region length.
    pub max_segment_ms: Option<u32>,
}

/// One file of a pack. `role` says what the runner does with it; `other` marks files kept for
/// verification only (licenses, notices).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackFile {
    pub role: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

impl PackFile {
    /// Describes an existing file, for pack builders. `path` is relative to `dir`.
    pub fn from_disk(dir: &Path, role: &str, path: &str) -> std::io::Result<Self> {
        let (bytes, sha256) = hash_file(&dir.join(path))?;
        Ok(Self { role: role.into(), path: path.into(), bytes, sha256 })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPackManifest {
    pub schema: u32,
    pub id: String,
    pub version: String,
    pub family: ModelFamily,
    /// ISO 639 language codes in lower case, or `*` for a multilingual model.
    pub languages: Vec<String>,
    pub runtime: RuntimeRequirement,
    #[serde(default)]
    pub options: ModelOptions,
    pub files: Vec<PackFile>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// How thoroughly [`ModelPackManifest::verify`] checks files.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerifyMode {
    /// Presence and exact size: cheap enough for every status check.
    Sizes,
    /// Size and SHA-256 of every file.
    #[default]
    Full,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct VerificationReport {
    pub pack_id: String,
    pub files: usize,
    pub bytes: u64,
    pub hashed: bool,
}

/// The language part of a locale tag: `ar-JO` → `ar`, `tr_TR` → `tr`, `EN` → `en`.
/// Returns an empty string when the tag has no usable language subtag.
///
/// The deprecated codes Android's `Locale.getLanguage()` still returns for compatibility become
/// their current ISO 639-1 codes: `iw` → `he`, `in` → `id`, `ji` → `yi`.
pub fn language_of(locale: &str) -> String {
    let first = locale.split(['-', '_']).next().unwrap_or("").trim();
    if !(2..=3).contains(&first.len()) || !first.chars().all(|c| c.is_ascii_alphabetic()) {
        return String::new();
    }
    let first = first.to_ascii_lowercase();
    match first.as_str() {
        "iw" => "he".into(),
        "in" => "id".into(),
        "ji" => "yi".into(),
        _ => first,
    }
}

/// The language tokens of the multilingual Whisper checkpoints tiny through large-v2, in the
/// tokenizer's order. sherpa-onnx 1.13.8 ends the whole process (`_Exit`) when it is given a
/// language outside this list, so nothing else may reach a Whisper decoder.
pub const WHISPER_LANGUAGES: [&str; 99] = [
    "en", "zh", "de", "es", "ru", "ko", "fr", "ja", "pt", "tr", "pl", "ca", "nl", "ar", "sv", "it", "id",
    "hi", "fi", "vi", "he", "uk", "el", "ms", "cs", "ro", "da", "hu", "ta", "no", "th", "ur", "hr", "bg",
    "lt", "la", "mi", "ml", "cy", "sk", "te", "fa", "lv", "bn", "sr", "az", "sl", "kn", "et", "mk", "br",
    "eu", "is", "hy", "ne", "mn", "bs", "kk", "sq", "sw", "gl", "mr", "pa", "si", "km", "sn", "yo", "so",
    "af", "oc", "ka", "be", "tg", "sd", "gu", "am", "yi", "lo", "uz", "fo", "ht", "ps", "tk", "nn", "mt",
    "sa", "lb", "my", "bo", "tl", "mg", "as", "tt", "haw", "ln", "ha", "ba", "jw", "su",
];

/// The Whisper language token for a language (as returned by [`language_of`]), or `None` when
/// Whisper has none. Whisper spells three languages its own way: Norwegian Bokmål is `no`,
/// Filipino is `tl` and Javanese is `jw`.
pub fn whisper_language(language: &str) -> Option<&'static str> {
    let language = match language {
        "nb" => "no",
        "fil" => "tl",
        "jv" => "jw",
        other => other,
    };
    WHISPER_LANGUAGES.iter().copied().find(|token| *token == language)
}

impl ModelPackManifest {
    /// Reads and validates `<dir>/manifest.json`. Does not touch the model files.
    pub fn load(dir: &Path) -> Result<Self, EngineError> {
        let path = dir.join(MANIFEST_FILE);
        let too_big = || EngineError::PackInvalid(format!("{MANIFEST_FILE} is not a file under 1 MiB"));
        // Checked before opening so a FIFO or device never blocks the open.
        let metadata = std::fs::metadata(&path)
            .map_err(|_| EngineError::PackMissing(format!("{} not found", path.display())))?;
        if !metadata.is_file() {
            return Err(too_big());
        }
        let file = File::open(&path).map_err(|e| EngineError::PackMissing(format!("cannot read {}: {e}", path.display())))?;
        // Bounded by the read itself, not the size seen above: the file may grow in between.
        let mut bytes = Vec::new();
        file.take(MAX_MANIFEST_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| EngineError::PackMissing(format!("cannot read {}: {e}", path.display())))?;
        if bytes.len() as u64 > MAX_MANIFEST_BYTES {
            return Err(too_big());
        }
        let text = String::from_utf8(bytes).map_err(|_| EngineError::PackInvalid(format!("{MANIFEST_FILE} is not UTF-8")))?;
        Self::parse(&text)
    }

    pub fn parse(json: &str) -> Result<Self, EngineError> {
        let manifest: Self = serde_json::from_str(json).map_err(|e| EngineError::PackInvalid(e.to_string()))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).expect("manifest serializes")
    }

    pub fn validate(&self) -> Result<(), EngineError> {
        let invalid = |detail: String| Err(EngineError::PackInvalid(detail));
        if self.schema != MANIFEST_SCHEMA {
            return invalid(format!("schema {} is not supported (expected {MANIFEST_SCHEMA})", self.schema));
        }
        if self.id.is_empty()
            || self.id.len() > 128
            || !self.id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        {
            return invalid(format!("id {:?} must be 1–128 characters of a-z, 0-9, '.', '_' or '-'", self.id));
        }
        if self.version.is_empty() || self.version.len() > 64 {
            return invalid("version must be 1–64 characters".into());
        }
        if self.languages.is_empty() {
            return invalid("languages must not be empty".into());
        }
        for language in &self.languages {
            if language != "*" && language_of(language) != *language {
                return invalid(format!("language {language:?} must be a lower-case ISO 639 code or '*'"));
            }
        }
        if self.runtime.kind != "sherpa-onnx" {
            return invalid(format!("runtime {:?} is not supported", self.runtime.kind));
        }
        match (parse_version(&self.runtime.min_version), parse_version(SHERPA_ONNX_ABI)) {
            (Some(needed), Some(have)) if needed <= have => {}
            (Some(_), _) => {
                return invalid(format!(
                    "pack needs sherpa-onnx {} or newer; this build uses {SHERPA_ONNX_ABI}",
                    self.runtime.min_version
                ))
            }
            (None, _) => return invalid(format!("runtime min_version {:?} is not a version", self.runtime.min_version)),
        }
        self.validate_options()?;
        self.validate_files()
    }

    fn validate_options(&self) -> Result<(), EngineError> {
        let invalid = |detail: &str| Err(EngineError::PackInvalid(format!("options: {detail}")));
        let options = &self.options;
        match (&options.model_type, self.family) {
            (None, _) => {}
            (Some(kind), ModelFamily::Transducer) if kind == "transducer" || kind == "nemo_transducer" => {}
            (Some(_), _) => return invalid("model_type must be 'transducer' or 'nemo_transducer' on a transducer"),
        }
        if let Some(language) = &options.language {
            if language != "auto" && language_of(language) != *language {
                return invalid("language must be a lower-case ISO 639 code or 'auto'");
            }
            if self.family == ModelFamily::Whisper && language != "auto" && whisper_language(language).is_none() {
                return invalid("language is not a Whisper language token");
            }
        }
        if options.feature_dim.is_some_and(|dim| !(1..=512).contains(&dim)) {
            return invalid("feature_dim must be 1–512");
        }
        if options.max_segment_ms.is_some_and(|ms| !(1_000..=120_000).contains(&ms)) {
            return invalid("max_segment_ms must be 1000–120000");
        }
        Ok(())
    }

    fn validate_files(&self) -> Result<(), EngineError> {
        let invalid = |detail: String| Err(EngineError::PackInvalid(detail));
        if self.files.is_empty() {
            return invalid("files must not be empty".into());
        }
        let known = self.family.known_roles();
        let mut roles = HashSet::new();
        let mut paths = HashSet::new();
        for file in &self.files {
            if !safe_relative_path(&file.path) {
                return invalid(format!("path {:?} must be a plain relative path inside the pack", file.path));
            }
            if file.path == MANIFEST_FILE || !paths.insert(file.path.as_str()) {
                return invalid(format!("path {:?} is listed twice or is the manifest", file.path));
            }
            if file.bytes == 0 {
                return invalid(format!("{} has no size", file.path));
            }
            if file.sha256.len() != 64 || !file.sha256.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b)) {
                return invalid(format!("{} needs a lower-case hex SHA-256", file.path));
            }
            if file.role != "other" {
                if !known.contains(&file.role.as_str()) {
                    return invalid(format!("role {:?} is not used by {:?} models", file.role, self.family));
                }
                if !roles.insert(file.role.as_str()) {
                    return invalid(format!("role {:?} is listed twice", file.role));
                }
            }
        }
        let missing: Vec<_> = self.family.required_roles(&roles).into_iter().filter(|r| !roles.contains(r)).collect();
        if !missing.is_empty() {
            return invalid(format!("{:?} pack is missing {}", self.family, missing.join(", ")));
        }
        Ok(())
    }

    /// Whether the pack recognizes `language` (as returned by [`language_of`]).
    pub fn supports(&self, language: &str) -> bool {
        !language.is_empty() && self.languages.iter().any(|l| l == "*" || l == language)
    }

    pub fn is_multilingual(&self) -> bool {
        self.languages.iter().any(|l| l == "*") || self.languages.len() > 1
    }

    pub fn file(&self, role: &str) -> Option<&PackFile> {
        self.files.iter().find(|f| f.role == role)
    }

    pub fn path_of(&self, dir: &Path, role: &str) -> Option<PathBuf> {
        self.file(role).map(|f| dir.join(&f.path))
    }

    /// Checks every listed file under `dir`. Symbolic links are refused in every component of a
    /// file's path, not only the file itself, so a pack cannot point native code outside its own
    /// directory through a linked subdirectory.
    pub fn verify(&self, dir: &Path, mode: VerifyMode) -> Result<VerificationReport, EngineError> {
        let mut total = 0;
        for file in &self.files {
            let fail = |reason: String| EngineError::Verification { path: file.path.clone(), reason };
            let mut path = dir.to_path_buf();
            let mut parts = file.path.split('/').peekable();
            while let Some(part) = parts.next() {
                path.push(part);
                if parts.peek().is_none() {
                    break;
                }
                let metadata = std::fs::symlink_metadata(&path).map_err(|_| fail("missing".into()))?;
                if !metadata.file_type().is_dir() {
                    return Err(fail(format!("{} is not a plain directory", path.strip_prefix(dir).unwrap_or(&path).display())));
                }
            }
            let metadata = std::fs::symlink_metadata(&path).map_err(|_| fail("missing".into()))?;
            if !metadata.file_type().is_file() {
                return Err(fail("not a regular file".into()));
            }
            if metadata.len() != file.bytes {
                return Err(fail(format!("{} bytes, expected {}", metadata.len(), file.bytes)));
            }
            if mode == VerifyMode::Full {
                let (bytes, sha256) = hash_file(&path).map_err(|e| fail(format!("cannot read: {e}")))?;
                if bytes != file.bytes {
                    return Err(fail(format!("changed while reading ({bytes} bytes)")));
                }
                if sha256 != file.sha256 {
                    return Err(fail("SHA-256 does not match".into()));
                }
            }
            total += file.bytes;
        }
        Ok(VerificationReport { pack_id: self.id.clone(), files: self.files.len(), bytes: total, hashed: mode == VerifyMode::Full })
    }
}

fn safe_relative_path(path: &str) -> bool {
    !path.is_empty()
        && path.len() <= 255
        && !path.starts_with('/')
        && !path.contains(['\\', ':', '\0'])
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..")
}

fn parse_version(text: &str) -> Option<Vec<u32>> {
    let parts: Option<Vec<u32>> = text.split('.').map(|p| p.parse().ok()).collect();
    parts.filter(|p| !p.is_empty() && p.len() <= 4)
}

fn hash_file(path: &Path) -> std::io::Result<(u64, String)> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 256 * 1024];
    let mut total = 0u64;
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        total += read as u64;
    }
    let digest = hasher.finalize();
    Ok((total, digest.iter().map(|b| format!("{b:02x}")).collect()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::fs;

    pub(crate) struct TempDir(pub PathBuf);
    impl TempDir {
        pub fn new(name: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
            let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let dir = std::env::temp_dir().join(format!("sonojot-mobile-{name}-{}-{n}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    /// A pack of small fake files with a manifest whose sizes and hashes match them.
    pub(crate) fn write_pack(dir: &Path, family: ModelFamily, languages: &[&str], roles: &[&str]) -> ModelPackManifest {
        let files = roles
            .iter()
            .map(|role| {
                let path = format!("{role}.bin");
                fs::write(dir.join(&path), format!("fake {role} weights")).unwrap();
                PackFile::from_disk(dir, role, &path).unwrap()
            })
            .collect();
        let manifest = ModelPackManifest {
            schema: MANIFEST_SCHEMA,
            id: "test-pack".into(),
            version: "1".into(),
            family,
            languages: languages.iter().map(|l| l.to_string()).collect(),
            runtime: RuntimeRequirement { kind: "sherpa-onnx".into(), min_version: SHERPA_ONNX_ABI.into() },
            options: ModelOptions::default(),
            files,
            display_name: None,
            license: None,
            source: None,
        };
        fs::write(dir.join(MANIFEST_FILE), manifest.to_json()).unwrap();
        manifest
    }

    fn whisper(dir: &Path) -> ModelPackManifest {
        write_pack(dir, ModelFamily::Whisper, &["*"], &["encoder", "decoder", "tokens"])
    }

    #[test]
    fn locale_tags_reduce_to_languages() {
        assert_eq!(language_of("ar-JO"), "ar");
        assert_eq!(language_of("tr_TR"), "tr");
        assert_eq!(language_of("EN"), "en");
        assert_eq!(language_of("zh-Hant-TW"), "zh");
        assert_eq!(language_of("fil-PH"), "fil");
        assert_eq!(language_of("iw-IL"), "he");
        assert_eq!(language_of("in_ID"), "id");
        assert_eq!(language_of("JI"), "yi");
        assert_eq!(language_of(""), "");
        assert_eq!(language_of("1234"), "");
        assert_eq!(language_of("english"), "");
    }

    #[test]
    fn a_written_pack_loads_and_verifies() {
        let dir = TempDir::new("verify");
        let written = whisper(&dir.0);
        let loaded = ModelPackManifest::load(&dir.0).unwrap();
        assert_eq!(loaded, written);
        assert!(loaded.supports("ar") && loaded.supports("tr") && loaded.supports("en"));
        assert!(loaded.is_multilingual());
        let report = loaded.verify(&dir.0, VerifyMode::Full).unwrap();
        assert_eq!(report.files, 3);
        assert!(report.hashed);
        assert_eq!(report.bytes, loaded.files.iter().map(|f| f.bytes).sum::<u64>());
    }

    #[test]
    fn tampered_missing_and_resized_files_fail() {
        let dir = TempDir::new("tamper");
        let manifest = whisper(&dir.0);
        // Same size, different content: only a full check notices.
        let decoder = dir.0.join("decoder.bin");
        let mut bytes = fs::read(&decoder).unwrap();
        bytes[0] ^= 1;
        fs::write(&decoder, &bytes).unwrap();
        assert!(manifest.verify(&dir.0, VerifyMode::Sizes).is_ok());
        assert_eq!(
            manifest.verify(&dir.0, VerifyMode::Full),
            Err(EngineError::Verification { path: "decoder.bin".into(), reason: "SHA-256 does not match".into() })
        );
        fs::write(&decoder, b"x").unwrap();
        assert!(matches!(manifest.verify(&dir.0, VerifyMode::Sizes), Err(EngineError::Verification { reason, .. }) if reason.contains("expected")));
        fs::remove_file(&decoder).unwrap();
        assert!(matches!(manifest.verify(&dir.0, VerifyMode::Sizes), Err(EngineError::Verification { reason, .. }) if reason == "missing"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_files_are_refused() {
        let dir = TempDir::new("symlink");
        let manifest = whisper(&dir.0);
        let outside = TempDir::new("symlink-target");
        let tokens = dir.0.join("tokens.bin");
        fs::rename(&tokens, outside.0.join("tokens.bin")).unwrap();
        std::os::unix::fs::symlink(outside.0.join("tokens.bin"), &tokens).unwrap();
        assert!(matches!(manifest.verify(&dir.0, VerifyMode::Full), Err(EngineError::Verification { reason, .. }) if reason == "not a regular file"));
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_parent_directories_are_refused() {
        let dir = TempDir::new("symlink-parent");
        let outside = TempDir::new("symlink-parent-target");
        fs::write(outside.0.join("LICENSE"), b"MIT").unwrap();
        std::os::unix::fs::symlink(&outside.0, dir.0.join("licenses")).unwrap();
        let mut manifest = whisper(&dir.0);
        // The pins match the file the link reaches; only the link itself is wrong.
        manifest.files.push(PackFile::from_disk(&dir.0, "other", "licenses/LICENSE").unwrap());
        manifest.validate().unwrap();
        let error = manifest.verify(&dir.0, VerifyMode::Sizes).unwrap_err();
        assert!(matches!(&error, EngineError::Verification { path, reason } if path == "licenses/LICENSE" && reason.contains("not a plain directory")), "{error:?}");
        // The same file in a real subdirectory is fine.
        fs::remove_file(dir.0.join("licenses")).unwrap();
        fs::create_dir(dir.0.join("licenses")).unwrap();
        fs::write(dir.0.join("licenses/LICENSE"), b"MIT").unwrap();
        assert_eq!(manifest.verify(&dir.0, VerifyMode::Full).unwrap().files, 4);
    }

    #[test]
    fn oversized_manifests_are_refused_without_reading_them_whole() {
        let dir = TempDir::new("big-manifest");
        whisper(&dir.0);
        let mut text = fs::read_to_string(dir.0.join(MANIFEST_FILE)).unwrap();
        text.push_str(&" ".repeat(MAX_MANIFEST_BYTES as usize));
        fs::write(dir.0.join(MANIFEST_FILE), text).unwrap();
        assert!(matches!(ModelPackManifest::load(&dir.0), Err(EngineError::PackInvalid(d)) if d.contains("1 MiB")));
        fs::write(dir.0.join(MANIFEST_FILE), [0xff, 0xfe]).unwrap();
        assert!(matches!(ModelPackManifest::load(&dir.0), Err(EngineError::PackInvalid(d)) if d.contains("UTF-8")));
        fs::remove_file(dir.0.join(MANIFEST_FILE)).unwrap();
        fs::create_dir(dir.0.join(MANIFEST_FILE)).unwrap();
        assert!(matches!(ModelPackManifest::load(&dir.0), Err(EngineError::PackInvalid(_))));
    }

    #[test]
    fn whisper_language_tokens_cover_phone_locales() {
        assert_eq!(WHISPER_LANGUAGES.len(), 99);
        assert_eq!(WHISPER_LANGUAGES.iter().collect::<HashSet<_>>().len(), 99);
        for (locale, token) in [("nb-NO", Some("no")), ("fil-PH", Some("tl")), ("jv-ID", Some("jw")), ("iw-IL", Some("he")),
            ("in-ID", Some("id")), ("ji", Some("yi")), ("haw-US", Some("haw")), ("ckb-IQ", None), ("yue-HK", None), ("", None)]
        {
            assert_eq!(whisper_language(&language_of(locale)), token, "{locale}");
        }
    }

    #[test]
    fn whisper_packs_only_fix_languages_whisper_knows() {
        let dir = TempDir::new("whisper-option");
        let mut pack = whisper(&dir.0);
        for (language, ok) in [("auto", true), ("ar", true), ("nb", true), ("ckb", false), ("yue", false)] {
            pack.options.language = Some(language.into());
            assert_eq!(pack.validate().is_ok(), ok, "{language}");
        }
        // SenseVoice has its own list; the Whisper table does not apply to it.
        let mut sense = write_pack(&dir.0, ModelFamily::SenseVoice, &["yue"], &["model", "tokens"]);
        sense.options.language = Some("yue".into());
        assert!(sense.validate().is_ok());
    }

    #[test]
    fn missing_pack_is_reported_as_missing() {
        let dir = TempDir::new("missing");
        assert!(matches!(ModelPackManifest::load(&dir.0), Err(EngineError::PackMissing(_))));
        assert!(matches!(ModelPackManifest::load(&dir.0.join("nope")), Err(EngineError::PackMissing(_))));
    }

    fn mutated(edit: impl FnOnce(&mut serde_json::Value)) -> Result<ModelPackManifest, EngineError> {
        let dir = TempDir::new("mutate");
        let mut value: serde_json::Value = serde_json::from_str(&whisper(&dir.0).to_json()).unwrap();
        edit(&mut value);
        ModelPackManifest::parse(&value.to_string())
    }

    #[test]
    fn manifests_that_could_escape_or_mislead_are_refused() {
        use serde_json::json;
        type Edit = Box<dyn FnOnce(&mut serde_json::Value)>;
        let cases: Vec<(&str, Edit)> = vec![
            ("schema", Box::new(|v| v["schema"] = json!(2))),
            ("id", Box::new(|v| v["id"] = json!("Bad Id"))),
            ("languages", Box::new(|v| v["languages"] = json!([]))),
            ("language code", Box::new(|v| v["languages"] = json!(["English"]))),
            ("runtime kind", Box::new(|v| v["runtime"]["kind"] = json!("onnxruntime"))),
            ("newer runtime", Box::new(|v| v["runtime"]["min_version"] = json!("9.0.0"))),
            ("runtime version", Box::new(|v| v["runtime"]["min_version"] = json!("latest"))),
            ("parent path", Box::new(|v| v["files"][0]["path"] = json!("../encoder.bin"))),
            ("absolute path", Box::new(|v| v["files"][0]["path"] = json!("/data/encoder.bin"))),
            ("windows path", Box::new(|v| v["files"][0]["path"] = json!("C:\\encoder.bin"))),
            ("dot path", Box::new(|v| v["files"][0]["path"] = json!("./encoder.bin"))),
            ("manifest path", Box::new(|v| v["files"][0]["path"] = json!("manifest.json"))),
            ("duplicate path", Box::new(|v| v["files"][1]["path"] = v["files"][0]["path"].clone())),
            ("duplicate role", Box::new(|v| v["files"][1]["role"] = json!("encoder"))),
            ("unknown role", Box::new(|v| v["files"][0]["role"] = json!("joiner"))),
            ("missing role", Box::new(|v| { v["files"].as_array_mut().unwrap().pop(); })),
            ("empty size", Box::new(|v| v["files"][0]["bytes"] = json!(0))),
            ("upper-case hash", Box::new(|v| v["files"][0]["sha256"] = json!("A".repeat(64)))),
            ("short hash", Box::new(|v| v["files"][0]["sha256"] = json!("abc"))),
            ("model type", Box::new(|v| v["options"]["model_type"] = json!("nemo_transducer"))),
            ("feature dim", Box::new(|v| v["options"]["feature_dim"] = json!(0))),
            ("segment", Box::new(|v| v["options"]["max_segment_ms"] = json!(10))),
            ("option language", Box::new(|v| v["options"]["language"] = json!("Arabic"))),
        ];
        for (name, edit) in cases {
            assert!(matches!(mutated(edit), Err(EngineError::PackInvalid(_))), "{name} was accepted");
        }
        // Extra license files and fields added by newer pack builders are fine.
        let accepted = mutated(|v| {
            v["files"].as_array_mut().unwrap().push(json!({"role": "other", "path": "licenses/LICENSE", "bytes": 3, "sha256": "a".repeat(64)}));
            v["built_by"] = json!("pack tool 2");
        });
        assert!(accepted.is_ok(), "{accepted:?}");
    }

    #[test]
    fn family_roles_are_enforced() {
        let dir = TempDir::new("roles");
        let sense = write_pack(&dir.0, ModelFamily::SenseVoice, &["en"], &["model", "tokens"]);
        assert!(sense.validate().is_ok() && sense.supports("en") && !sense.supports("ar") && !sense.is_multilingual());
        let v1 = write_pack(&dir.0, ModelFamily::Moonshine, &["en"], &["preprocessor", "encoder", "uncached_decoder", "cached_decoder", "tokens"]);
        assert!(v1.validate().is_ok());
        let v2 = write_pack(&dir.0, ModelFamily::Moonshine, &["ar"], &["encoder", "merged_decoder", "tokens"]);
        assert!(v2.validate().is_ok());
        let mut broken = v2.clone();
        broken.files.retain(|f| f.role != "merged_decoder");
        assert!(matches!(broken.validate(), Err(EngineError::PackInvalid(d)) if d.contains("preprocessor")));
        let mut transducer = write_pack(&dir.0, ModelFamily::Transducer, &["en"], &["encoder", "decoder", "joiner", "tokens"]);
        transducer.options.model_type = Some("nemo_transducer".into());
        assert!(transducer.validate().is_ok());
    }

    #[test]
    fn versions_compare_numerically() {
        assert!(parse_version("1.13.8") > parse_version("1.9.30"));
        assert!(parse_version("1.13") < parse_version("1.13.8"));
        assert_eq!(parse_version("1.x"), None);
        assert_eq!(parse_version(""), None);
    }
}
