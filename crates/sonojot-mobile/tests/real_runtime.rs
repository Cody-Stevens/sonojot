//! End-to-end check against a real sherpa-onnx 1.13.8 runtime and model. Ignored by default:
//! it needs files no repository ships.
//!
//! ```sh
//! SONOJOT_TEST_SHERPA_LIB=/path/libsherpa-onnx-c-api.so \
//! SONOJOT_TEST_TRANSDUCER_DIR=/path/parakeet-tdt-0.6b-v2 \
//! SONOJOT_TEST_WAV=/path/english-speech.wav \
//! cargo test --test real_runtime -- --ignored --nocapture
//! ```
//!
//! The transducer directory holds `encoder.int8.onnx`, `decoder.int8.onnx`, `joiner.int8.onnx`
//! and `tokens.txt` (NeMo Parakeet). The WAV may be any rate; it is resampled to 16 kHz mono.
//!
//! The catalog check runs the download catalog's packs through the C ABI exactly as an app does:
//! `SONOJOT_TEST_CATALOG_DIR` holds one staging directory per catalog pack `id` with that pack's
//! downloaded files under their catalog `path` names (packs not downloaded are skipped).
//!
//! ```sh
//! SONOJOT_TEST_SHERPA_LIB=/path/libsherpa-onnx-c-api.so \
//! SONOJOT_TEST_CATALOG_DIR=/path/staging \
//! SONOJOT_TEST_WAV=/path/english-speech.wav \
//! cargo test --test real_runtime catalog -- --ignored --nocapture
//! ```

use sonojot_mobile::{
    catalog, ffi, GateConfig, ModelFamily, ModelOptions, ModelPackManifest, PackFile, Recognizer, RuntimeRequirement,
    SherpaRecognizer, SherpaRuntime, SonojotConfig, SonojotEngine, VerifyMode, MANIFEST_FILE, MANIFEST_SCHEMA,
    SHERPA_ONNX_ABI,
};
use std::{
    ffi::{c_char, CStr, CString},
    path::PathBuf,
    ptr,
};

fn env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from)
}

fn read_wav_16k_mono(path: &std::path::Path) -> Vec<f32> {
    let mut reader = hound::WavReader::open(path).expect("wav");
    let spec = reader.spec();
    let interleaved: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => reader.samples::<f32>().map(Result::unwrap).collect(),
        hound::SampleFormat::Int => {
            let scale = (1u64 << (spec.bits_per_sample - 1)) as f32;
            reader.samples::<i32>().map(|s| s.unwrap() as f32 / scale).collect()
        }
    };
    let channels = spec.channels as usize;
    let mono: Vec<f32> = interleaved.chunks(channels).map(|f| f.iter().sum::<f32>() / channels as f32).collect();
    if spec.sample_rate == 16_000 {
        return mono;
    }
    let ratio = spec.sample_rate as f64 / 16_000.0;
    (0..(mono.len() as f64 / ratio) as usize)
        .map(|i| {
            let source = i as f64 * ratio;
            let index = source as usize;
            let frac = (source - index as f64) as f32;
            let a = mono[index];
            let b = mono.get(index + 1).copied().unwrap_or(a);
            a + (b - a) * frac
        })
        .collect()
}

#[test]
#[ignore = "needs SONOJOT_TEST_SHERPA_LIB, SONOJOT_TEST_TRANSDUCER_DIR and SONOJOT_TEST_WAV"]
fn parakeet_transcribes_real_speech_through_the_c_api() {
    let (Some(library), Some(dir), Some(wav)) =
        (env_path("SONOJOT_TEST_SHERPA_LIB"), env_path("SONOJOT_TEST_TRANSDUCER_DIR"), env_path("SONOJOT_TEST_WAV"))
    else {
        panic!("set SONOJOT_TEST_SHERPA_LIB, SONOJOT_TEST_TRANSDUCER_DIR and SONOJOT_TEST_WAV");
    };
    let runtime = SherpaRuntime::open(library.to_str().unwrap()).expect("runtime");
    assert_eq!(runtime.version(), SHERPA_ONNX_ABI);

    // The manifest lives in memory: the model directory is read-only test input.
    let files = [("encoder", "encoder.int8.onnx"), ("decoder", "decoder.int8.onnx"), ("joiner", "joiner.int8.onnx"), ("tokens", "tokens.txt")]
        .iter()
        .map(|(role, path)| PackFile::from_disk(&dir, role, path).expect("model file"))
        .collect();
    let manifest = ModelPackManifest {
        schema: MANIFEST_SCHEMA,
        id: "parakeet-test".into(),
        version: "1".into(),
        family: ModelFamily::Transducer,
        languages: vec!["en".into()],
        runtime: RuntimeRequirement { kind: "sherpa-onnx".into(), min_version: SHERPA_ONNX_ABI.into() },
        options: ModelOptions { model_type: Some("nemo_transducer".into()), ..ModelOptions::default() },
        files,
        display_name: None,
        license: None,
        source: None,
    };
    let report = manifest.verify(&dir, VerifyMode::Full).expect("verify");
    assert!(report.hashed);

    let mut recognizer = SherpaRecognizer::new(runtime, &manifest, &dir, "en", 2).expect("recognizer");
    let audio = read_wav_16k_mono(&wav);
    let direct = recognizer.decode(&audio[..audio.len().min(20 * 16_000)]).expect("decode");
    eprintln!("direct decode: {direct}");
    assert!(direct.split_whitespace().count() >= 3, "{direct:?}");

    let config = SonojotConfig { locale: "en-US".into(), gate: GateConfig::default(), ..SonojotConfig::default() };
    let mut engine = SonojotEngine::with_recognizer(&config, manifest, Box::new(recognizer)).expect("engine");
    let started = std::time::Instant::now();
    let result = engine.transcribe(&audio).expect("transcribe");
    eprintln!(
        "engine: {} ms audio, {} ms speech in {} segments, {:?} wall\nraw:  {}\ntext: {}",
        result.audio_ms,
        result.speech_ms,
        result.segments.len(),
        started.elapsed(),
        result.raw_text,
        result.text
    );
    assert!(!result.text.is_empty());
    assert!(result.speech_ms <= result.audio_ms);
}

/// A string returned by the library, freed after reading: `Ok` for a result, `Err` for an error.
unsafe fn take(result: *mut c_char, error: *mut c_char) -> Result<serde_json::Value, serde_json::Value> {
    let read = |text: *mut c_char| {
        let value = serde_json::from_str(CStr::from_ptr(text).to_str().unwrap()).unwrap();
        ffi::sonojot_string_free(text);
        value
    };
    match (result.is_null(), error.is_null()) {
        (false, true) => Ok(read(result)),
        (true, false) => Err(read(error)),
        _ => panic!("a call must return exactly one of a result and an error"),
    }
}

#[test]
#[ignore = "needs SONOJOT_TEST_SHERPA_LIB, SONOJOT_TEST_CATALOG_DIR and SONOJOT_TEST_WAV"]
fn catalog_packs_finalize_open_and_transcribe_through_the_c_api() {
    let (Some(library), Some(staging), Some(wav)) =
        (env_path("SONOJOT_TEST_SHERPA_LIB"), env_path("SONOJOT_TEST_CATALOG_DIR"), env_path("SONOJOT_TEST_WAV"))
    else {
        panic!("set SONOJOT_TEST_SHERPA_LIB, SONOJOT_TEST_CATALOG_DIR and SONOJOT_TEST_WAV");
    };
    let audio = read_wav_16k_mono(&wav);
    let mut checked = 0;
    for pack in catalog::CATALOG {
        let dir = staging.join(pack.id);
        if !dir.is_dir() {
            eprintln!("{}: not downloaded, skipped", pack.id);
            continue;
        }
        let dir_c = CString::new(dir.to_str().unwrap()).unwrap();
        let id_c = CString::new(pack.id).unwrap();
        unsafe {
            let mut error = ptr::null_mut();
            let report = take(ffi::sonojot_pack_finalize(dir_c.as_ptr(), id_c.as_ptr(), &mut error), error)
                .unwrap_or_else(|e| panic!("{}: finalize: {e}", pack.id));
            assert_eq!(report["hashed"], true, "{report}");
            assert!(dir.join(MANIFEST_FILE).is_file());
        }

        // English for the English pack. For the multilingual pack: locales whose language
        // sherpa-onnx 1.13.8 would end the process on if passed through unchanged (nb, fil, iw, in,
        // ckb, yue), and ones Whisper names directly.
        let locales: &[&str] = if pack.languages.contains(&"*") {
            &["en-GB", "nb-NO", "fil-PH", "iw-IL", "in-ID", "ckb-IQ", "yue-HK", "ar-JO", "tr-TR"]
        } else {
            &["en-US"]
        };
        for locale in locales {
            let config = serde_json::json!({
                "pack_dir": dir, "locale": locale, "runtime_library": library, "verify": "full",
            });
            let config = CString::new(config.to_string()).unwrap();
            unsafe {
                let mut error = ptr::null_mut();
                let handle = ffi::sonojot_engine_open(config.as_ptr(), &mut error);
                if handle.is_null() {
                    panic!("{} {locale}: open: {}", pack.id, take(ptr::null_mut(), error).unwrap_err());
                }
                let started = std::time::Instant::now();
                let result = take(ffi::sonojot_engine_transcribe_f32(handle, audio.as_ptr(), audio.len(), &mut error), error)
                    .unwrap_or_else(|e| panic!("{} {locale}: transcribe: {e}", pack.id));
                ffi::sonojot_engine_close(handle);
                eprintln!("{} {locale} ({:?}): {}", pack.id, started.elapsed(), result["text"]);
                if locale.starts_with("en") {
                    let words = result["text"].as_str().unwrap().split_whitespace().count();
                    assert!(words >= 3, "{} {locale}: {result}", pack.id);
                }
            }
        }

        // An engine never opens on sizes alone.
        let sizes = serde_json::json!({ "pack_dir": dir, "locale": locales[0], "runtime_library": library, "verify": "sizes" });
        let sizes = CString::new(sizes.to_string()).unwrap();
        unsafe {
            let mut error = ptr::null_mut();
            let handle = ffi::sonojot_engine_open(sizes.as_ptr(), &mut error);
            if !handle.is_null() {
                ffi::sonojot_engine_close(handle);
                panic!("{}: an engine opened with verify \"sizes\"", pack.id);
            }
            let refused = take(ptr::null_mut(), error).unwrap_err();
            assert_eq!(refused["code"], "invalid_argument", "{refused}");
        }
        checked += 1;
    }
    assert!(checked > 0, "no catalog pack was downloaded into {}", staging.display());
}
