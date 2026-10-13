//! Original meeting audio export (Round 6, meeting-audio-export worker).
//!
//! Copies the saved meeting WAV bytes verbatim: no re-encode, no
//! transcription, no text changes. The primary saved meeting audio
//! (`{id}.wav`, i.e. `audio_for("mixed")`) is the original for both native
//! recordings (mixed/mic/system stems written together) and imports
//! (ffmpeg-normalised `.import.wav` renamed to mixed on finish).
//!
//! Ownership: this module owns only the streaming atomic copy + audio
//! export constants. `Meetings` (meetings.rs) owns record/audio resolution
//! and destination guards. `commands.rs` owns the Tauri command thin
//! wrapper. No edits to app/ai/meeting_ai/ai_commands/resource/speech.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use crate::store::new_id;

/// Canonical format key accepted by `Meetings::export` for audio.
pub const AUDIO_FORMAT: &str = "audio";
/// Alias accepted for UI-fix convenience; identical bytes.
pub const AUDIO_FORMAT_ALIAS: &str = "wav";
/// Honest on-disk extension: saved originals are 16 kHz mono PCM16 WAV.
pub const AUDIO_EXTENSION: &str = "wav";
/// User-facing label; component and Save dialog use this verbatim. The WAV
/// label applies only when the stored source is a WAV file (native 16 kHz
/// mono PCM16 recordings and ffmpeg-normalised imports; see
/// `Meetings::export_audio`, which refuses non-WAV stored sources rather
/// than mislabelling their bytes).
pub const AUDIO_LABEL: &str = "Original recording";
/// Save-dialog filter label; states the honest container.
pub const AUDIO_FILTER_LABEL: &str = "Original recording (WAV)";

pub fn is_audio_format(format: &str) -> bool {
    matches!(format, "audio" | "wav")
}

/// True when two existing paths name the same file data: same path,
/// symlink/junction alias (resolved), or hardlink alias (same identity).
/// Missing files are never the same. Used to refuse exporting a meeting
/// onto its own original media even through an alias outside app data.
pub fn is_same_file(a: &Path, b: &Path) -> bool {
    if let (Ok(ca), Ok(cb)) = (fs::canonicalize(a), fs::canonicalize(b)) {
        if ca == cb {
            return true;
        }
    }
    same_identity(a, b)
}

#[cfg(unix)]
fn same_identity(a: &Path, b: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    match (fs::metadata(a), fs::metadata(b)) {
        (Ok(ma), Ok(mb)) => ma.dev() == mb.dev() && ma.ino() == mb.ino(),
        _ => false,
    }
}

#[cfg(windows)]
fn same_identity(a: &Path, b: &Path) -> bool {
    // Stable std has no file-identity API on Windows (MetadataExt volume /
    // file index is still unstable `windows_by_handle`), so query the
    // volume + file index directly, mirroring the raw kernel32 style
    // already used for atomic replacement in `store.rs`. No new
    // dependencies; links are followed so symlink aliases match too.
    use std::os::windows::ffi::OsStrExt;
    #[repr(C)]
    struct FileIdentity {
        _attributes: u32,
        _creation: [u32; 2],
        _access: [u32; 2],
        _write: [u32; 2],
        volume: u32,
        _size_high: u32,
        _size_low: u32,
        _links: u32,
        index_high: u32,
        index_low: u32,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn CreateFileW(
            name: *const u16,
            access: u32,
            share: u32,
            security: *const u8,
            disposition: u32,
            flags: u32,
            template: *const u8,
        ) -> *mut std::ffi::c_void;
        fn GetFileInformationByHandle(
            handle: *mut std::ffi::c_void,
            info: *mut FileIdentity,
        ) -> i32;
        fn CloseHandle(handle: *mut std::ffi::c_void) -> i32;
    }
    fn identity(path: &Path) -> Option<(u32, u32, u32)> {
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // GENERIC_READ, share read/write/delete, OPEN_EXISTING.
        unsafe {
            let handle = CreateFileW(
                wide.as_ptr(),
                0x8000_0000,
                0x1 | 0x2 | 0x4,
                std::ptr::null(),
                3,
                0,
                std::ptr::null(),
            );
            if handle.is_null() || handle as isize == -1 {
                return None;
            }
            let mut out = std::mem::zeroed::<FileIdentity>();
            let ok = GetFileInformationByHandle(handle, &mut out);
            CloseHandle(handle);
            if ok == 0 {
                return None;
            }
            Some((out.volume, out.index_high, out.index_low))
        }
    }
    match (identity(a), identity(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[cfg(not(any(unix, windows)))]
fn same_identity(_a: &Path, _b: &Path) -> bool {
    false
}

/// Stream-copy `source` to `destination` atomically without loading the
/// whole meeting into RAM (hour-long meetings stay bounded).
///
/// - `destination` parent must already exist (Tauri save dialog guarantees).
/// - Refuses to overwrite a symlink destination (same rule as
///   `store::atomic_write`).
/// - The source is opened ONCE; its handle length is the truth. Bytes stream
///   from that handle into a unique temp file in the destination directory,
///   fsync, then the temp length is verified against the handle length BEFORE
///   the atomic replace. A short write or a source that changed mid-copy
///   fails with only the temp removed: the final user destination is never
///   created, replaced, or deleted on a failed validation, so a pre-existing
///   export survives. The original is never modified or deleted.
/// - Returns bytes copied.
pub fn stream_copy_atomic(source: &Path, destination: &Path) -> Result<u64, String> {
    stream_copy_inner(source, destination, None)
}

/// Length-gated variant for callers (and narrow tests) that assert an exact
/// expected size: the temp copy must equal `expected_len` before replace,
/// otherwise only the temp is removed and the destination is untouched.
pub fn stream_copy_atomic_with_expected(
    source: &Path,
    destination: &Path,
    expected_len: u64,
) -> Result<u64, String> {
    stream_copy_inner(source, destination, Some(expected_len))
}

fn stream_copy_inner(
    source: &Path,
    destination: &Path,
    expected_override: Option<u64>,
) -> Result<u64, String> {
    let parent = destination
        .parent()
        .ok_or("The destination has no parent directory.")?;
    if !parent.is_dir() {
        return Err("The destination directory does not exist.".into());
    }
    if fs::symlink_metadata(destination).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("Cannot overwrite a linked destination file.".into());
    }
    // Open first: the handle (not a pre-copy path stat) is the length truth,
    // so a source replaced between lookup and open cannot slip a stale size
    // past verification.
    let mut reader = fs::File::open(source)
        .map_err(|e| format!("Cannot read the saved meeting audio: {e}"))?;
    let handle_len = reader
        .metadata()
        .map_err(|e| format!("Cannot inspect the saved meeting audio: {e}"))?
        .len();
    let expected = expected_override.unwrap_or(handle_len);
    let temporary = parent.join(format!(".sonojot-{}.tmp", new_id()));
    let result: Result<u64, String> = (|| {
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
            .map_err(|e| format!("Cannot create a temporary data file: {e}"))?;
        let mut writer = std::io::BufWriter::with_capacity(64 * 1024, file);
        let bytes = std::io::copy(&mut reader, &mut writer)
            .map_err(|e| format!("Cannot copy the meeting audio: {e}"))?;
        writer
            .flush()
            .and_then(|_| writer.get_ref().sync_all())
            .map_err(|e| format!("Cannot save data: {e}"))?;
        drop(writer);
        // Verify BEFORE replace: the temp copy must equal the opened handle
        // (or the asserted expected length). Anything else removes only the
        // temp; the existing destination — user data — is never touched.
        let temp_len = fs::metadata(&temporary)
            .map_err(|e| format!("Cannot verify the exported audio: {e}"))?
            .len();
        if bytes != expected || temp_len != expected {
            return Err(
                "The exported audio is incomplete. The original recording was preserved.".into(),
            );
        }
        replace_file(&temporary, destination)
            .map_err(|e| format!("Cannot replace the saved file: {e}"))?;
        #[cfg(unix)]
        std::fs::File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|e| format!("Cannot flush the saved directory: {e}"))?;
        Ok(bytes)
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

    #[test]
    fn audio_constants_stay_honest() {
        assert_eq!(AUDIO_EXTENSION, "wav");
        assert!(is_audio_format("audio"));
        assert!(is_audio_format("wav"));
        assert!(!is_audio_format("mp3"));
        assert!(AUDIO_LABEL.contains("Original"));
    }

    #[test]
    fn same_file_detects_hardlink_alias_and_rejects_missing() {
        let root =
            std::env::temp_dir().join(format!("sonojot-audio-same-{}", new_id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.wav");
        fs::write(&source, b"original").unwrap();
        assert!(is_same_file(&source, &source));
        assert!(!is_same_file(&source, &root.join("absent.wav")));
        assert!(!is_same_file(&source, &root.join("other.wav")));
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.join("link.wav");
            symlink(&source, &link).unwrap();
            assert!(is_same_file(&source, &link));
        }
        if fs::hard_link(&source, root.join("hard.wav")).is_ok() {
            assert!(is_same_file(&source, &root.join("hard.wav")));
        }
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn length_mismatch_never_touches_the_existing_destination() {
        // A failed length validation must leave any pre-existing user export
        // untouched: only the temp copy is removed, never the final path.
        let root =
            std::env::temp_dir().join(format!("sonojot-audio-mismatch-{}", new_id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.wav");
        let bytes: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        fs::write(&source, &bytes).unwrap();
        let destination = root.join("prior-export.wav");
        fs::write(&destination, b"the user's earlier export").unwrap();
        // Assert a wrong expected length: copy is correct, validation fails.
        let err = stream_copy_atomic_with_expected(&source, &destination, 1)
            .unwrap_err();
        assert!(err.contains("incomplete"), "{err}");
        // Pre-existing destination byte-identical; source untouched; no temp
        // files left behind in the destination directory.
        assert_eq!(fs::read(&destination).unwrap(), b"the user's earlier export");
        assert_eq!(fs::read(&source).unwrap(), bytes);
        let leftovers: Vec<_> = fs::read_dir(&root)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                e.file_name()
                    .to_string_lossy()
                    .starts_with(".sonojot-")
            })
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
        // The exact-length gate still copies fine over the same destination.
        let copied =
            stream_copy_atomic_with_expected(&source, &destination, bytes.len() as u64)
                .unwrap();
        assert_eq!(copied, bytes.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stream_copy_preserves_bytes_and_refuses_symlink_overwrite() {
        let root =
            std::env::temp_dir().join(format!("sonojot-audio-copy-{}", new_id()));
        fs::create_dir_all(&root).unwrap();
        let source = root.join("source.wav");
        let bytes: Vec<u8> = (0..262_144u32).map(|i| (i % 251) as u8).collect();
        fs::write(&source, &bytes).unwrap();
        let destination = root.join("copy.wav");
        let copied = stream_copy_atomic(&source, &destination).unwrap();
        assert_eq!(copied, bytes.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), bytes);
        // Original preserved.
        assert_eq!(fs::read(&source).unwrap(), bytes);
        #[cfg(unix)]
        {
            use std::os::unix::fs::symlink;
            let link = root.join("link.wav");
            let _ = fs::remove_file(&link);
            symlink(&destination, &link).unwrap();
            assert!(stream_copy_atomic(&source, &link).is_err());
            // Linked target untouched.
            assert_eq!(fs::read(&destination).unwrap(), bytes);
        }
        fs::remove_dir_all(&root).unwrap();
    }
}
