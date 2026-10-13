//! SonoJot overrides take precedence; existing developer setups keep working.
pub(crate) fn var_os(suffix: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(format!("SONOJOT_{suffix}"))
        .or_else(|| std::env::var_os(format!("EAVESDROPPER_{suffix}")))
}
