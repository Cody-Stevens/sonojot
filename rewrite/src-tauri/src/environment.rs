//! SonoJot overrides take precedence. The EAVESDROPPER_* names stay as
//! fallbacks because evaluation scripts in rewrite/evaluation still set them.
pub(crate) fn var_os(suffix: &str) -> Option<std::ffi::OsString> {
    if crate::flavor::SYNC_TEST {
        return std::env::var_os(format!("SONOJOT_SYNCTEST_{suffix}"));
    }
    std::env::var_os(format!("SONOJOT_{suffix}"))
        .or_else(|| std::env::var_os(format!("EAVESDROPPER_{suffix}")))
}
