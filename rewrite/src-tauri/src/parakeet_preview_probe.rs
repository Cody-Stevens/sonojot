//! Opt-in timing probe using the installed runtime and weights, without Tauri,
//! capture, user recordings or a second model. See docs/research/ParakeetLivePreview.md.
use super::*;
use std::time::{Duration, Instant};

fn input() -> (PathBuf, Vec<f32>) {
    let root = PathBuf::from(std::env::var_os("SONOJOT_PREVIEW_MODELS").expect("model root"));
    let wav = std::env::var_os("SONOJOT_PREVIEW_WAV").expect("public 16 kHz mono PCM16 WAV");
    let mut reader = hound::WavReader::open(wav).unwrap();
    assert_eq!(reader.spec().sample_rate, 16_000);
    assert_eq!(reader.spec().channels, 1);
    let source: Vec<f32> = reader
        .samples::<i16>()
        .map(|s| f32::from(s.unwrap()) / 32768.0)
        .collect();
    assert!(!source.is_empty());
    (root, source)
}

#[test]
#[ignore = "requires SONOJOT_PREVIEW_MODELS and SONOJOT_PREVIEW_WAV"]
fn parakeet_decode_timings() {
    let (root, source) = input();
    let started = Instant::now();
    let engine = Engine::load(&root, optional_asr::PARAKEET_FT2_ID).unwrap();
    println!("load_ms={:.1}", started.elapsed().as_secs_f64() * 1000.0);
    for seconds in [1, 5, 10, 20, 30] {
        let audio: Vec<_> = source
            .iter()
            .copied()
            .cycle()
            .take(seconds * 16_000)
            .collect();
        let mut times = Vec::new();
        for run in 0..4 {
            let started = Instant::now();
            let result = engine.decode_once(&audio).unwrap();
            let ms = started.elapsed().as_secs_f64() * 1000.0;
            println!(
                "audio_s={seconds} run={run} decode_ms={ms:.1} words={}",
                result.text.split_whitespace().count()
            );
            if run > 0 {
                times.push(ms);
            }
        }
        times.sort_by(f64::total_cmp);
        println!("audio_s={seconds} warm_median_ms={:.1}", times[1]);
    }
}

#[test]
#[ignore = "requires SONOJOT_PREVIEW_MODELS and SONOJOT_PREVIEW_WAV"]
fn parakeet_preview_overlap() {
    let (root, source) = input();
    let engine = Engine::load(&root, optional_asr::PARAKEET_FT2_ID).unwrap();
    let Decoder::Native(native) = &engine.decoder else {
        unreachable!()
    };
    let tail: Vec<_> = source.iter().copied().cycle().take(5 * 16_000).collect();
    engine.decode_once(&tail).unwrap();
    for seconds in [1, 5, 30] {
        let audio: Vec<_> = source
            .iter()
            .copied()
            .cycle()
            .take(seconds * 16_000)
            .collect();
        for run in 0..3 {
            let start = Instant::now();
            let expected = engine.decode_once(&audio).unwrap().text;
            let alone = start.elapsed();
            let model = native.clone();
            let mut preview =
                crate::parakeet_preview::Preview::start(0, move |audio| model.preview(audio))
                    .unwrap();
            preview.allow(true);
            preview.push(&tail);
            preview.poll(0, true);
            let deadline = Instant::now() + Duration::from_secs(2);
            while !native.preview_busy.load(Ordering::Acquire) {
                assert!(Instant::now() < deadline, "preview must start");
                std::thread::yield_now();
            }
            let start = Instant::now();
            let actual = engine.decode_once(&audio).unwrap().text;
            let overlap = start.elapsed();
            assert_eq!(actual, expected, "parallel drafts changed final words");
            println!(
                "audio_s={seconds} run={run} alone_ms={:.1} overlap_ms={:.1}",
                alone.as_secs_f64() * 1000.0,
                overlap.as_secs_f64() * 1000.0
            );
            drop(preview);
            while native.preview_busy.load(Ordering::Acquire) {
                std::thread::yield_now();
            }
        }
    }
}

#[test]
#[ignore = "requires installed model; replays eight seconds of public audio in real time"]
fn parakeet_preview_live_preserves_final_result() {
    let (root, source) = input();
    let mut engine = Engine::load(&root, optional_asr::PARAKEET_FT2_ID).unwrap();
    let audio: Vec<_> = source.iter().copied().cycle().take(8 * 16_000).collect();
    let mut baseline = engine.stream().unwrap();
    for chunk in audio.chunks(320) {
        baseline.add(chunk, 16_000).unwrap();
    }
    let expected = baseline.finish_recognition().unwrap();
    let mut stream = engine.stream().unwrap();
    stream.allow_preview(true);
    let mut drafts = 0;
    let started = Instant::now();
    for (index, chunk) in audio.chunks(320).enumerate() {
        stream.add(chunk, 16_000).unwrap();
        if !stream.preview_text().is_empty() {
            drafts += 1;
        }
        std::thread::sleep(
            (Duration::from_millis((index as u64 + 1) * 20)).saturating_sub(started.elapsed()),
        );
    }
    stream.allow_preview(false);
    assert!(stream.preview_text().is_empty());
    let actual = stream.finish_recognition().unwrap();
    assert!(drafts > 0, "the live stream must display some drafts");
    assert_eq!(actual.text, expected.text);
    assert_eq!(actual.completeness, expected.completeness);
    assert_eq!(actual.warnings, expected.warnings);
    println!(
        "draft_updates={drafts} final_words={}",
        actual.text.split_whitespace().count()
    );
}
