use std::fmt;

/// Why the engine could not load or transcribe. [`EngineError::code`] is stable across releases
/// and crosses the C ABI, so apps can choose a fallback without parsing messages.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EngineError {
    /// The sherpa-onnx runtime library could not be found or loaded.
    RuntimeMissing(String),
    /// The runtime loaded but lacks a required C function.
    RuntimeSymbol(String),
    /// The runtime reports a version whose C structs differ from the ones compiled here.
    RuntimeVersion { expected: String, found: String },
    /// No model pack, or no `manifest.json`, at the given directory.
    PackMissing(String),
    /// The manifest is malformed or names files the engine cannot use safely.
    PackInvalid(String),
    /// A pack file is missing or its size or SHA-256 differs from the manifest.
    Verification { path: String, reason: String },
    /// The pack does not cover the requested language.
    UnsupportedLocale { locale: String, pack: String },
    /// The native recognizer could not be created from the pack's files.
    Init(String),
    /// The native recognizer failed while decoding.
    Decode(String),
    /// The audio buffer cannot be read (for example, a byte count that is not a multiple of 4).
    InvalidAudio(String),
    /// A caller passed an argument the API cannot accept.
    InvalidArgument(String),
    /// Rust code panicked behind the C ABI. The engine handle should be closed.
    Panic(String),
}

impl EngineError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::RuntimeMissing(_) => "runtime_missing",
            Self::RuntimeSymbol(_) => "runtime_symbol",
            Self::RuntimeVersion { .. } => "runtime_version",
            Self::PackMissing(_) => "pack_missing",
            Self::PackInvalid(_) => "pack_invalid",
            Self::Verification { .. } => "pack_verification",
            Self::UnsupportedLocale { .. } => "unsupported_locale",
            Self::Init(_) => "init",
            Self::Decode(_) => "decode",
            Self::InvalidAudio(_) => "invalid_audio",
            Self::InvalidArgument(_) => "invalid_argument",
            Self::Panic(_) => "panic",
        }
    }
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RuntimeMissing(detail) => write!(f, "speech runtime not available: {detail}"),
            Self::RuntimeSymbol(name) => write!(f, "speech runtime is missing {name}"),
            Self::RuntimeVersion { expected, found } => {
                write!(f, "speech runtime is sherpa-onnx {found}; this build needs {expected}")
            }
            Self::PackMissing(detail) => write!(f, "model pack not installed: {detail}"),
            Self::PackInvalid(detail) => write!(f, "model pack is invalid: {detail}"),
            Self::Verification { path, reason } => write!(f, "model file {path} failed verification: {reason}"),
            Self::UnsupportedLocale { locale, pack } => write!(f, "model pack {pack} does not cover {locale}"),
            Self::Init(detail) => write!(f, "cannot start speech recognition: {detail}"),
            Self::Decode(detail) => write!(f, "speech recognition failed: {detail}"),
            Self::InvalidAudio(detail) => write!(f, "audio cannot be read: {detail}"),
            Self::InvalidArgument(detail) => write!(f, "invalid argument: {detail}"),
            Self::Panic(detail) => write!(f, "internal error: {detail}"),
        }
    }
}

impl std::error::Error for EngineError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_are_stable_and_distinct() {
        let all = [
            EngineError::RuntimeMissing(String::new()),
            EngineError::RuntimeSymbol(String::new()),
            EngineError::RuntimeVersion { expected: "1".into(), found: "2".into() },
            EngineError::PackMissing(String::new()),
            EngineError::PackInvalid(String::new()),
            EngineError::Verification { path: String::new(), reason: String::new() },
            EngineError::UnsupportedLocale { locale: String::new(), pack: String::new() },
            EngineError::Init(String::new()),
            EngineError::Decode(String::new()),
            EngineError::InvalidAudio(String::new()),
            EngineError::InvalidArgument(String::new()),
            EngineError::Panic(String::new()),
        ];
        let codes: std::collections::BTreeSet<_> = all.iter().map(EngineError::code).collect();
        assert_eq!(codes.len(), all.len());
        assert_eq!(
            EngineError::RuntimeVersion { expected: "1.13.8".into(), found: "1.12.0".into() }.to_string(),
            "speech runtime is sherpa-onnx 1.12.0; this build needs 1.13.8"
        );
    }
}
