//! The system's preferred display language, reduced to its primary subtag
//! ("en-GB" -> "en"). Used only to pick a speech-model recommendation for a
//! new install; it never changes saved settings.

/// The primary subtag of the first preferred language, lowercase, or `None`
/// when it cannot be read.
pub fn primary() -> Option<String> {
    preferred().as_deref().and_then(primary_subtag)
}

/// "en-US", "en_GB.UTF-8", "pt-BR" or "zh-Hans-CN" -> "en", "en", "pt", "zh".
/// The POSIX "C" and "POSIX" locales say nothing about the language.
pub(crate) fn primary_subtag(tag: &str) -> Option<String> {
    let tag = tag.trim();
    let first = tag.split([':']).next().unwrap_or("");
    let subtag = first.split(['-', '_', '.', '@']).next().unwrap_or("").trim();
    if subtag.is_empty() || subtag.eq_ignore_ascii_case("c") || subtag.eq_ignore_ascii_case("posix") {
        return None;
    }
    if !(2..=3).contains(&subtag.len()) || !subtag.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    Some(subtag.to_ascii_lowercase())
}

#[cfg(windows)]
fn preferred() -> Option<String> {
    const MUI_LANGUAGE_NAME: u32 = 0x8;
    #[link(name = "kernel32")]
    extern "system" {
        fn GetUserPreferredUILanguages(flags: u32, count: *mut u32, buffer: *mut u16, length: *mut u32) -> i32;
    }
    let mut count = 0u32;
    let mut buffer = [0u16; 512];
    let mut length = buffer.len() as u32;
    // SAFETY: the buffer and its length in UTF-16 units are valid for the call.
    let ok = unsafe { GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buffer.as_mut_ptr(), &mut length) };
    if ok == 0 || count == 0 {
        return None;
    }
    let end = buffer.iter().position(|&unit| unit == 0).unwrap_or(0);
    (end > 0).then(|| String::from_utf16_lossy(&buffer[..end]))
}

#[cfg(target_os = "macos")]
fn preferred() -> Option<String> {
    use std::ffi::{c_char, c_void, CStr};
    const UTF8: u32 = 0x0800_0100;
    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFLocaleCopyPreferredLanguages() -> *const c_void;
        fn CFArrayGetCount(array: *const c_void) -> isize;
        fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
        fn CFStringGetCString(string: *const c_void, buffer: *mut c_char, size: isize, encoding: u32) -> bool;
        fn CFRelease(value: *const c_void);
    }
    // SAFETY: the copied array is released once; its first element is
    // borrowed only while the array is alive.
    unsafe {
        let languages = CFLocaleCopyPreferredLanguages();
        if languages.is_null() {
            return None;
        }
        let mut result = None;
        if CFArrayGetCount(languages) > 0 {
            let first = CFArrayGetValueAtIndex(languages, 0);
            let mut buffer = [0 as c_char; 64];
            if !first.is_null() && CFStringGetCString(first, buffer.as_mut_ptr(), buffer.len() as isize, UTF8) {
                result = Some(CStr::from_ptr(buffer.as_ptr()).to_string_lossy().into_owned());
            }
        }
        CFRelease(languages);
        result
    }
}

#[cfg(not(any(windows, target_os = "macos")))]
fn preferred() -> Option<String> {
    ["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
        .into_iter()
        .filter_map(|name| std::env::var(name).ok())
        .find(|value| primary_subtag(value).is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primary_subtags_come_from_common_tag_shapes() {
        for (tag, expected) in [
            ("en-US", Some("en")),
            ("en", Some("en")),
            ("EN-gb", Some("en")),
            ("en_GB.UTF-8", Some("en")),
            ("de_DE@euro", Some("de")),
            ("pt-BR", Some("pt")),
            ("zh-Hans-CN", Some("zh")),
            ("fil-PH", Some("fil")),
            ("en_US:fr_FR", Some("en")),
            ("C", None),
            ("C.UTF-8", None),
            ("POSIX", None),
            ("", None),
            ("x1-y", None),
        ] {
            assert_eq!(primary_subtag(tag).as_deref(), expected, "{tag}");
        }
    }

    #[test]
    fn reading_the_system_language_does_not_panic() {
        let _ = primary();
    }
}
