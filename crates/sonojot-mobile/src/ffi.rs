//! C ABI for Android (through JNA or JNI) and iOS. See `include/sonojot_mobile.h`.
//!
//! - Inputs and outputs are UTF-8 JSON strings, so the ABI stays small and versionable.
//! - Every returned string is owned by the caller and must be released with
//!   [`sonojot_string_free`].
//! - On failure a function returns null (or 0) and, when `out_error` is not null, stores a JSON
//!   object `{"code": "...", "message": "..."}` there. Codes are [`EngineError::code`].
//! - Panics never cross the boundary; they become the `panic` error code.
//! - An engine handle may be used from any thread; calls on one handle are serialized.

use crate::{EngineError, ModelPackManifest, SonojotConfig, SonojotEngine, VerifyMode, MANIFEST_SCHEMA, SHERPA_ONNX_ABI};
use std::{
    ffi::{c_char, CStr, CString},
    panic::{catch_unwind, AssertUnwindSafe},
    path::Path,
    ptr,
    sync::{Mutex, OnceLock},
};

/// Bumped whenever a function is added, removed, or changes signature or meaning. Apps refuse a
/// library with another version. 2: catalog, pack finalize and ABI info.
pub const ABI_VERSION: u32 = 2;

/// Opaque engine handle.
pub struct SonojotEngineHandle {
    engine: Mutex<SonojotEngine>,
}

fn to_c_string(text: String) -> *mut c_char {
    // JSON escapes NUL as \u0000, so serialized output never contains a raw NUL.
    CString::new(text).map(CString::into_raw).unwrap_or(ptr::null_mut())
}

fn error_json(error: &EngineError) -> String {
    serde_json::json!({ "code": error.code(), "message": error.to_string() }).to_string()
}

/// Runs `body`, turning errors and panics into `out_error` and `fallback`.
///
/// # Safety
/// `out_error` must be null or valid for one pointer write.
unsafe fn guard<T>(out_error: *mut *mut c_char, fallback: T, body: impl FnOnce() -> Result<T, EngineError>) -> T {
    if !out_error.is_null() {
        *out_error = ptr::null_mut();
    }
    let outcome = catch_unwind(AssertUnwindSafe(body)).unwrap_or_else(|panic| {
        let detail = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        Err(EngineError::Panic(detail))
    });
    match outcome {
        Ok(value) => value,
        Err(error) => {
            if !out_error.is_null() {
                *out_error = to_c_string(error_json(&error));
            }
            fallback
        }
    }
}

/// # Safety
/// `text` must be null or a NUL-terminated string.
unsafe fn read_str<'a>(text: *const c_char, what: &str) -> Result<&'a str, EngineError> {
    if text.is_null() {
        return Err(EngineError::InvalidArgument(format!("{what} is null")));
    }
    CStr::from_ptr(text).to_str().map_err(|_| EngineError::InvalidArgument(format!("{what} is not UTF-8")))
}

/// Borrows `len` elements at `data`, refusing what `slice::from_raw_parts` forbids: a null or
/// misaligned pointer for a non-empty slice, or a length whose byte size exceeds `isize::MAX`
/// (a negative Java length widened to `usize`, for example).
///
/// # Safety
/// When the checks pass, `data` must be valid for reads of `len` elements for `'a`.
unsafe fn slice_arg<'a, T>(data: *const T, len: usize, what: &str) -> Result<&'a [T], EngineError> {
    if len == 0 {
        return Ok(&[]);
    }
    if data.is_null() {
        return Err(EngineError::InvalidArgument(format!("{what} is null")));
    }
    if !data.is_aligned() {
        return Err(EngineError::InvalidArgument(format!("{what} is not aligned to {} bytes", std::mem::align_of::<T>())));
    }
    if len > isize::MAX as usize / std::mem::size_of::<T>().max(1) {
        return Err(EngineError::InvalidArgument(format!("{what} length {len} is out of range")));
    }
    Ok(std::slice::from_raw_parts(data, len))
}

fn engine_of(handle: *mut SonojotEngineHandle) -> Result<&'static SonojotEngineHandle, EngineError> {
    // SAFETY: callers pass a handle from `sonojot_engine_open` that they have not closed.
    unsafe { handle.as_ref() }.ok_or_else(|| EngineError::InvalidArgument("engine handle is null".into()))
}

fn transcribe_with(
    handle: *mut SonojotEngineHandle,
    run: impl FnOnce(&mut SonojotEngine) -> Result<crate::TranscriptResult, EngineError>,
) -> Result<*mut c_char, EngineError> {
    let handle = engine_of(handle)?;
    let mut engine = handle
        .engine
        .lock()
        .map_err(|_| EngineError::Panic("engine was poisoned by an earlier panic; close it".into()))?;
    let result = run(&mut engine)?;
    Ok(to_c_string(serde_json::to_string(&result).expect("result serializes")))
}

#[no_mangle]
pub extern "C" fn sonojot_mobile_abi_version() -> u32 {
    ABI_VERSION
}

/// The crate version as a static NUL-terminated string. Do not free it.
#[no_mangle]
pub extern "C" fn sonojot_mobile_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// What this build of the library speaks, so apps read it instead of copying the constants:
/// `{"abi_version": 2, "manifest_schema": 1, "required_sherpa_version": "1.13.8"}`.
pub fn abi_info() -> serde_json::Value {
    serde_json::json!({
        "abi_version": ABI_VERSION,
        "manifest_schema": MANIFEST_SCHEMA,
        "required_sherpa_version": SHERPA_ONNX_ABI,
    })
}

/// [`abi_info`] as a static NUL-terminated JSON string. Do not free it.
#[no_mangle]
pub extern "C" fn sonojot_mobile_abi_info() -> *const c_char {
    static INFO: OnceLock<CString> = OnceLock::new();
    INFO.get_or_init(|| CString::new(abi_info().to_string()).expect("JSON has no NUL")).as_ptr()
}

/// Opens an engine from a [`SonojotConfig`] JSON object. Returns null on failure.
///
/// # Safety
/// `config_json` must be a NUL-terminated string; `out_error` null or writable.
#[no_mangle]
pub unsafe extern "C" fn sonojot_engine_open(config_json: *const c_char, out_error: *mut *mut c_char) -> *mut SonojotEngineHandle {
    guard(out_error, ptr::null_mut(), || {
        let json = read_str(config_json, "config_json")?;
        let config: SonojotConfig =
            serde_json::from_str(json).map_err(|e| EngineError::InvalidArgument(format!("config: {e}")))?;
        let engine = SonojotEngine::open(config)?;
        Ok(Box::into_raw(Box::new(SonojotEngineHandle { engine: Mutex::new(engine) })))
    })
}

/// Transcribes `len` bytes of little-endian `f32` 16 kHz mono PCM. Returns a
/// [`crate::TranscriptResult`] JSON object, or null on failure.
///
/// # Safety
/// `handle` must be open; `pcm` must be valid for `len` bytes (it may be null when `len` is 0).
#[no_mangle]
pub unsafe extern "C" fn sonojot_engine_transcribe_f32le(
    handle: *mut SonojotEngineHandle,
    pcm: *const u8,
    len: usize,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    guard(out_error, ptr::null_mut(), || {
        let bytes = slice_arg(pcm, len, "pcm")?;
        transcribe_with(handle, |engine| engine.transcribe_f32le(bytes))
    })
}

/// Transcribes `count` native `f32` samples (16 kHz mono). Same result as
/// [`sonojot_engine_transcribe_f32le`].
///
/// # Safety
/// `handle` must be open; `samples` must be 4-byte aligned and valid for `count` floats (it may
/// be null when `count` is 0). Misaligned pointers are refused with `invalid_argument`.
#[no_mangle]
pub unsafe extern "C" fn sonojot_engine_transcribe_f32(
    handle: *mut SonojotEngineHandle,
    samples: *const f32,
    count: usize,
    out_error: *mut *mut c_char,
) -> *mut c_char {
    guard(out_error, ptr::null_mut(), || {
        let samples = slice_arg(samples, count, "samples")?;
        transcribe_with(handle, |engine| engine.transcribe(samples))
    })
}

/// The open engine's manifest and language: `{"language": "ar", "manifest": {...}}`.
///
/// # Safety
/// `handle` must be open; `out_error` null or writable.
#[no_mangle]
pub unsafe extern "C" fn sonojot_engine_info(handle: *mut SonojotEngineHandle, out_error: *mut *mut c_char) -> *mut c_char {
    guard(out_error, ptr::null_mut(), || {
        let handle = engine_of(handle)?;
        let engine = handle.engine.lock().map_err(|_| EngineError::Panic("engine was poisoned".into()))?;
        let info = serde_json::json!({ "language": engine.language(), "manifest": engine.manifest() });
        Ok(to_c_string(info.to_string()))
    })
}

/// Releases an engine. Null is ignored. The handle must not be used afterwards.
///
/// # Safety
/// `handle` must be null or an open handle not used by another thread.
#[no_mangle]
pub unsafe extern "C" fn sonojot_engine_close(handle: *mut SonojotEngineHandle) {
    if !handle.is_null() {
        let _ = catch_unwind(AssertUnwindSafe(|| drop(Box::from_raw(handle))));
    }
}

/// Validates the pack at `pack_dir` and checks its files: sizes when `full` is 0, sizes and
/// SHA-256 otherwise. Returns `{"manifest": {...}, "report": {...}}` or null on failure.
///
/// # Safety
/// `pack_dir` must be a NUL-terminated string; `out_error` null or writable.
#[no_mangle]
pub unsafe extern "C" fn sonojot_pack_inspect(pack_dir: *const c_char, full: i32, out_error: *mut *mut c_char) -> *mut c_char {
    guard(out_error, ptr::null_mut(), || {
        let dir = Path::new(read_str(pack_dir, "pack_dir")?);
        let manifest = ModelPackManifest::load(dir)?;
        let report = manifest.verify(dir, if full == 0 { VerifyMode::Sizes } else { VerifyMode::Full })?;
        Ok(to_c_string(serde_json::json!({ "manifest": manifest, "report": report }).to_string()))
    })
}

/// The download catalog: `{"download_hosts": [...], "packs": [{"id", "display_name", "languages",
/// "family", "bytes", "license", "files": [{"role", "path", "url", "bytes", "sha256"}]}]}`.
/// Never fails; free the result.
#[no_mangle]
pub extern "C" fn sonojot_catalog() -> *mut c_char {
    to_c_string(crate::catalog::catalog_json())
}

/// After downloading every file of catalog pack `pack_id` into `staging_dir`, writes its
/// `manifest.json` and verifies each file's size and SHA-256. Returns the verification report
/// JSON, or null (and nothing written) if any file is wrong. Move the directory into place only
/// after this succeeds.
///
/// # Safety
/// Both strings must be NUL-terminated; `out_error` null or writable.
#[no_mangle]
pub unsafe extern "C" fn sonojot_pack_finalize(staging_dir: *const c_char, pack_id: *const c_char, out_error: *mut *mut c_char) -> *mut c_char {
    guard(out_error, ptr::null_mut(), || {
        let dir = Path::new(read_str(staging_dir, "staging_dir")?);
        let id = read_str(pack_id, "pack_id")?;
        let report = crate::catalog::finalize(dir, id)?;
        Ok(to_c_string(serde_json::to_string(&report).expect("report serializes")))
    })
}

/// Frees a string returned by this library. Null is ignored.
///
/// # Safety
/// `text` must be null or a string from this library, freed once.
#[no_mangle]
pub unsafe extern "C" fn sonojot_string_free(text: *mut c_char) {
    if !text.is_null() {
        drop(CString::from_raw(text));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::{write_pack, TempDir};
    use crate::ModelFamily;

    unsafe fn take(text: *mut c_char) -> Option<serde_json::Value> {
        if text.is_null() {
            return None;
        }
        let value = serde_json::from_str(CStr::from_ptr(text).to_str().unwrap()).unwrap();
        sonojot_string_free(text);
        Some(value)
    }

    fn c(text: &str) -> CString {
        CString::new(text).unwrap()
    }

    #[test]
    fn version_functions_are_stable() {
        assert_eq!(sonojot_mobile_abi_version(), 2);
        let version = unsafe { CStr::from_ptr(sonojot_mobile_version()) }.to_str().unwrap();
        assert_eq!(version, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn abi_info_reports_the_build_constants() {
        let first = sonojot_mobile_abi_info();
        assert_eq!(first, sonojot_mobile_abi_info(), "the string is static and reused");
        let text = unsafe { CStr::from_ptr(first) }.to_str().unwrap();
        let info: serde_json::Value = serde_json::from_str(text).unwrap();
        assert_eq!(info, serde_json::json!({ "abi_version": 2, "manifest_schema": 1, "required_sherpa_version": "1.13.8" }));
        assert_eq!(info["abi_version"], sonojot_mobile_abi_version());
        assert_eq!(info["manifest_schema"], MANIFEST_SCHEMA);
        assert_eq!(info["required_sherpa_version"], SHERPA_ONNX_ABI);
    }

    #[test]
    fn open_failures_come_back_as_error_json() {
        let dir = TempDir::new("ffi-open");
        unsafe {
            let mut error = ptr::null_mut();
            let config = c(&serde_json::json!({ "pack_dir": dir.0, "locale": "en" }).to_string());
            assert!(sonojot_engine_open(config.as_ptr(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "pack_missing");

            assert!(sonojot_engine_open(c("{not json").as_ptr(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");

            assert!(sonojot_engine_open(ptr::null(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");

            // A null out_error is allowed.
            assert!(sonojot_engine_open(ptr::null(), ptr::null_mut()).is_null());
        }
    }

    #[test]
    fn handle_functions_reject_null_handles() {
        unsafe {
            let mut error = ptr::null_mut();
            assert!(sonojot_engine_transcribe_f32le(ptr::null_mut(), ptr::null(), 0, &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");
            assert!(sonojot_engine_transcribe_f32(ptr::null_mut(), ptr::null(), 0, &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");
            assert!(sonojot_engine_info(ptr::null_mut(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");
            sonojot_engine_close(ptr::null_mut());
            sonojot_string_free(ptr::null_mut());
        }
    }

    #[test]
    fn a_handle_transcribes_bytes_and_floats() {
        let dir = TempDir::new("ffi-handle");
        let manifest = write_pack(&dir.0, ModelFamily::Whisper, &["en"], &["encoder", "decoder", "tokens"]);
        let (recognizer, heard) = crate::engine::tests::scripted(&["HELLO THERE", "again"]);
        let config = SonojotConfig { locale: "en".into(), gate: crate::GateConfig { mode: crate::GateMode::Off, ..Default::default() }, ..Default::default() };
        let engine = SonojotEngine::with_recognizer(&config, manifest, recognizer).unwrap();
        let handle = Box::into_raw(Box::new(SonojotEngineHandle { engine: Mutex::new(engine) }));
        unsafe {
            let mut error = ptr::null_mut();
            let samples = vec![0.25f32; 1_600];
            let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
            let result = take(sonojot_engine_transcribe_f32le(handle, bytes.as_ptr(), bytes.len(), &mut error)).unwrap();
            assert!(error.is_null());
            assert_eq!(result["text"], "Hello there");
            assert_eq!(result["audio_ms"], 100);
            let result = take(sonojot_engine_transcribe_f32(handle, samples.as_ptr(), samples.len(), &mut error)).unwrap();
            assert_eq!(result["text"], "again");
            assert_eq!(heard.lock().unwrap().as_slice(), &[1_600, 1_600]);

            assert!(sonojot_engine_transcribe_f32le(handle, bytes.as_ptr(), 3, &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_audio");
            assert!(sonojot_engine_transcribe_f32le(handle, ptr::null(), 8, &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");

            let info = take(sonojot_engine_info(handle, &mut error)).unwrap();
            assert_eq!(info["language"], "en");
            assert_eq!(info["manifest"]["id"], "test-pack");
            sonojot_engine_close(handle);
        }
    }

    #[test]
    fn packs_can_be_inspected_without_a_runtime() {
        let dir = TempDir::new("ffi-inspect");
        write_pack(&dir.0, ModelFamily::SenseVoice, &["en"], &["model", "tokens"]);
        let path = c(dir.0.to_str().unwrap());
        unsafe {
            let mut error = ptr::null_mut();
            let value = take(sonojot_pack_inspect(path.as_ptr(), 1, &mut error)).unwrap();
            assert_eq!(value["manifest"]["family"], "sense_voice");
            assert_eq!(value["report"]["hashed"], true);
            std::fs::write(dir.0.join("model.bin"), b"changed weights!!").unwrap();
            assert!(sonojot_pack_inspect(path.as_ptr(), 0, &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "pack_verification");
        }
    }

    #[test]
    fn catalog_and_finalize_cross_the_boundary() {
        unsafe {
            let catalog = take(sonojot_catalog()).unwrap();
            assert!(catalog["packs"].as_array().unwrap().len() >= 2);
            let dir = TempDir::new("ffi-finalize");
            let path = c(dir.0.to_str().unwrap());
            let mut error = ptr::null_mut();
            assert!(sonojot_pack_finalize(path.as_ptr(), c("whisper-small-int8").as_ptr(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "pack_verification");
            assert!(sonojot_pack_finalize(path.as_ptr(), ptr::null(), &mut error).is_null());
            assert_eq!(take(error).unwrap()["code"], "invalid_argument");
        }
    }

    #[test]
    fn slices_that_from_raw_parts_forbids_are_refused() {
        unsafe {
            let mut error = ptr::null_mut();
            // A ByteBuffer slice at an odd offset: misaligned for f32.
            let backing = vec![0u32; 8];
            let misaligned = backing.as_ptr().cast::<u8>().add(1).cast::<f32>();
            assert!(sonojot_engine_transcribe_f32(ptr::null_mut(), misaligned, 4, &mut error).is_null());
            let error_json = take(error).unwrap();
            assert_eq!(error_json["code"], "invalid_argument");
            assert!(error_json["message"].as_str().unwrap().contains("aligned"));
            // A negative length widened to usize. The pointer is never read.
            let samples = [0.0f32; 4];
            assert!(sonojot_engine_transcribe_f32(ptr::null_mut(), samples.as_ptr(), usize::MAX, &mut error).is_null());
            assert!(take(error).unwrap()["message"].as_str().unwrap().contains("out of range"));
            assert!(sonojot_engine_transcribe_f32(ptr::null_mut(), samples.as_ptr(), isize::MAX as usize / 4 + 1, &mut error).is_null());
            assert!(take(error).unwrap()["message"].as_str().unwrap().contains("out of range"));
            let bytes = [0u8; 4];
            assert!(sonojot_engine_transcribe_f32le(ptr::null_mut(), bytes.as_ptr(), isize::MAX as usize + 1, &mut error).is_null());
            assert!(take(error).unwrap()["message"].as_str().unwrap().contains("out of range"));
            // Bytes have no alignment requirement: an odd offset reaches the handle check.
            assert!(sonojot_engine_transcribe_f32le(ptr::null_mut(), bytes.as_ptr().add(1), 3, &mut error).is_null());
            assert!(take(error).unwrap()["message"].as_str().unwrap().contains("handle"));
            assert_eq!(slice_arg(ptr::null::<f32>(), 0, "x").unwrap().len(), 0);
            assert_eq!(slice_arg(misaligned, 0, "x").unwrap().len(), 0);
        }
    }

    #[test]
    fn panics_do_not_cross_the_boundary() {
        unsafe {
            let mut error = ptr::null_mut();
            let value = guard(&mut error, 7, || -> Result<i32, EngineError> { panic!("boom") });
            assert_eq!(value, 7);
            let error = take(error).unwrap();
            assert_eq!(error["code"], "panic");
            assert!(error["message"].as_str().unwrap().contains("boom"));
        }
    }
}
