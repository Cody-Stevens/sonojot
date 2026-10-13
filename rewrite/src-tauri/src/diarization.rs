// SPDX-License-Identifier: LicenseRef-OpenMDW-1.1
// Copyright 2026 Joost Helfers
// Adapted from artifacts/diarization/diarize.py, the NumPy reference for
// joosthel/Nemotron-3-Diarization-ONNX (4a911fc3ca821b76a99fffd5ce5135bd1efca540).
// Its speaker cache and offline loop mirror NVIDIA's Nemotron 3 Diarization.
// The model distribution includes the OpenMDW-1.1 LICENSE.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Turn {
    pub start: f64,
    pub end: f64,
    pub speaker: usize,
}

/// Worker-only entry point: the desktop process must never initialize ONNX Runtime.
#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
pub(crate) fn diarize_file(models_root: &Path, audio: &Path) -> Result<Vec<Turn>, String> {
    let model_dir = crate::diarization_model::path(models_root);
    let runtime = crate::diarization_model::runtime_library(models_root);
    let constants = algorithm::Constants::load(&model_dir.join("constants.npz"))?;
    if !runtime.is_file() {
        return Err("Speaker labeling requires the installed ONNX Runtime.".into());
    }
    // This is the only ORT entry point in the diarization child; process lifetime
    // owns its DLL. Punctuation initializes ORT in a separate contained child.
    if !ort::init_from(&runtime)
        .map_err(|e| format!("Cannot load speaker-labeling runtime: {e}"))?
        .with_execution_providers([ort::ep::CPU::default().build()])
        .commit()
    {
        return Err("Speaker-labeling runtime was already initialized in this process.".into());
    }
    let mut waveform = Vec::new();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    crate::meeting_speech::for_mono_seconds(audio, &cancel, |samples| {
        waveform.extend_from_slice(samples);
        Ok(true)
    })?;
    algorithm::run(&model_dir, &waveform, &constants)
}

#[cfg(not(any(windows, all(target_os = "macos", target_arch = "aarch64"))))]
pub(crate) fn diarize_file(_models_root: &Path, _audio: &Path) -> Result<Vec<Turn>, String> {
    Err("Speaker labeling is not available on this platform.".into())
}

#[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64"), test))]
mod algorithm {
    use super::Turn;
    use std::collections::BTreeMap;

    const HOP_LENGTH: usize = 160;
    const N_FFT: usize = 512;
    const PREEMPHASIS: f32 = 0.97;
    const MEL_BINS: usize = 128;
    const HIDDEN_SIZE: usize = 512;
    const NUM_SPEAKERS: usize = 8;
    type ProbFrame = [f32; NUM_SPEAKERS];

    const INTEGER_CONSTANTS: [(&str, i64); 9] = [
        ("hidden_size", 512),
        ("subsampling_factor", 8),
        ("num_speakers", 8),
        ("chunk_length", 340),
        ("chunk_right_context", 40),
        ("fifo_length", 40),
        ("speaker_cache_update_period", 300),
        ("speaker_cache_length", 264),
        ("speaker_cache_silence_frames_per_speaker", 1),
    ];
    const FLOAT_CONSTANTS: [(&str, f64); 5] = [
        ("prediction_score_threshold", 0.25),
        ("latest_frames_score_boost", 0.05),
        ("strong_boost_rate", 0.75),
        ("weak_boost_rate", 1.5),
        ("min_positive_scores_rate", 0.5),
    ];

    pub(super) struct Constants {
        subsampling_factor: usize,
        chunk_length: usize,
        chunk_right_context: usize,
        fifo_length: usize,
        speaker_cache_update_period: usize,
        speaker_cache_length: usize,
        num_silence_frames: usize,
        prediction_score_threshold: f32,
        latest_frames_score_boost: f32,
        min_positive_scores: usize,
        num_strong_boosted_frames: usize,
        num_weak_boosted_frames: usize,
        silence_embeds: Vec<f32>,
    }

    impl Constants {
        fn from_entries(mut entries: BTreeMap<String, Vec<u8>>) -> Result<Self, String> {
            let mut integers = Vec::new();
            for (name, expected) in INTEGER_CONSTANTS {
                let bytes = take_constant(&mut entries, name)?;
                let data = npy_data(&bytes, "<i8", &[])?;
                let value = i64::from_le_bytes(data.try_into().unwrap());
                if value != expected {
                    return Err(format!("Unexpected diarization constant {name}: {value}."));
                }
                integers.push(value as usize);
            }
            let mut floats = Vec::new();
            for (name, expected) in FLOAT_CONSTANTS {
                let bytes = take_constant(&mut entries, name)?;
                let data = npy_data(&bytes, "<f8", &[])?;
                let value = f64::from_le_bytes(data.try_into().unwrap());
                if value != expected {
                    return Err(format!("Unexpected diarization constant {name}: {value}."));
                }
                floats.push(value);
            }
            let bytes = take_constant(&mut entries, "silence_embeds")?;
            let silence_embeds: Vec<f32> = npy_data(&bytes, "<f4", &[HIDDEN_SIZE])?
                .chunks_exact(4)
                .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
                .collect();
            if !entries.is_empty() || silence_embeds.iter().any(|x| !x.is_finite()) {
                return Err(
                    "Unexpected diarization constants or non-finite silence embeddings.".into(),
                );
            }
            let budget = integers[7] / integers[2] - integers[8];
            Ok(Self {
                subsampling_factor: integers[1],
                chunk_length: integers[3],
                chunk_right_context: integers[4],
                fifo_length: integers[5],
                speaker_cache_update_period: integers[6],
                speaker_cache_length: integers[7],
                num_silence_frames: integers[8],
                prediction_score_threshold: floats[0] as f32,
                latest_frames_score_boost: floats[1] as f32,
                min_positive_scores: (budget as f64 * floats[4]).floor() as usize,
                num_strong_boosted_frames: (budget as f64 * floats[2]).floor() as usize,
                num_weak_boosted_frames: (budget as f64 * floats[3]).floor() as usize,
                silence_embeds,
            })
        }

        #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
        pub(super) fn load(path: &std::path::Path) -> Result<Self, String> {
            use std::io::Read;
            let file = std::fs::File::open(path)
                .map_err(|e| format!("Cannot open diarization constants: {e}"))?;
            if file.metadata().map_err(|e| e.to_string())?.len() > 64 * 1024 {
                return Err("Diarization constants archive is too large.".into());
            }
            let mut archive = zip::ZipArchive::new(file)
                .map_err(|e| format!("Cannot read diarization constants: {e}"))?;
            if archive.len() != INTEGER_CONSTANTS.len() + FLOAT_CONSTANTS.len() + 1 {
                return Err("Unexpected number of diarization constants.".into());
            }
            let mut entries = BTreeMap::new();
            for index in 0..archive.len() {
                let mut entry = archive.by_index(index).map_err(|e| e.to_string())?;
                if entry.compression() != zip::CompressionMethod::Stored || entry.size() > 8192 {
                    return Err("Unexpected diarization constants ZIP entry.".into());
                }
                let name = entry.name().to_owned();
                let mut bytes = Vec::new();
                entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
                if entries.insert(name, bytes).is_some() {
                    return Err("Duplicate diarization constant.".into());
                }
            }
            Self::from_entries(entries)
        }
    }

    fn take_constant(
        entries: &mut BTreeMap<String, Vec<u8>>,
        name: &str,
    ) -> Result<Vec<u8>, String> {
        entries
            .remove(&format!("{name}.npy"))
            .ok_or_else(|| format!("Missing diarization constant {name}."))
    }

    // A deliberately small parser for the pinned export's numeric, C-order .npy files.
    // Never evaluate Python headers or accept object arrays, byte swaps, or guessed shapes.
    fn npy_data<'a>(bytes: &'a [u8], dtype: &str, shape: &[usize]) -> Result<&'a [u8], String> {
        let invalid = || "Invalid or unsupported diarization .npy constant.".to_owned();
        if bytes.get(..6) != Some(b"\x93NUMPY") {
            return Err(invalid());
        }
        let (prefix, header_len) = match bytes.get(6..8) {
            Some([1, 0]) if bytes.len() >= 10 => (
                10,
                u16::from_le_bytes(bytes[8..10].try_into().unwrap()) as usize,
            ),
            Some([2, 0] | [3, 0]) if bytes.len() >= 12 => (
                12,
                u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize,
            ),
            _ => return Err(invalid()),
        };
        if header_len > 4096 {
            return Err(invalid());
        }
        let header_end = prefix + header_len;
        let header = std::str::from_utf8(bytes.get(prefix..header_end).ok_or_else(invalid)?)
            .map_err(|_| invalid())?;
        if !header.ends_with('\n') {
            return Err(invalid());
        }
        let mut rest = header.trim();
        rest = rest.strip_prefix('{').ok_or_else(invalid)?.trim_start();
        let mut descr = None;
        let mut fortran = None;
        let mut dimensions = None;
        loop {
            if let Some(tail) = rest.strip_prefix('}') {
                if !tail.trim().is_empty() {
                    return Err(invalid());
                }
                break;
            }
            let key = npy_string(&mut rest).ok_or_else(invalid)?;
            rest = rest
                .trim_start()
                .strip_prefix(':')
                .ok_or_else(invalid)?
                .trim_start();
            match key {
                "descr" if descr.is_none() => {
                    descr = Some(npy_string(&mut rest).ok_or_else(invalid)?)
                }
                "fortran_order" if fortran.is_none() => {
                    rest = rest.strip_prefix("False").ok_or_else(invalid)?;
                    fortran = Some(false);
                }
                "shape" if dimensions.is_none() => {
                    rest = rest.strip_prefix('(').ok_or_else(invalid)?;
                    let end = rest.find(')').ok_or_else(invalid)?;
                    let raw = &rest[..end];
                    let mut dims = Vec::new();
                    if !raw.trim().is_empty() {
                        let parts: Vec<_> = raw.split(',').collect();
                        // A one-element Python tuple requires its trailing comma.
                        if parts.len() == 1 {
                            return Err(invalid());
                        }
                        for (i, part) in parts.iter().enumerate() {
                            let part = part.trim();
                            if part.is_empty() && i == parts.len() - 1 {
                                continue;
                            }
                            if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
                                return Err(invalid());
                            }
                            dims.push(part.parse::<usize>().map_err(|_| invalid())?);
                        }
                    }
                    dimensions = Some(dims);
                    rest = &rest[end + 1..];
                }
                _ => return Err(invalid()),
            }
            rest = rest.trim_start();
            if let Some(tail) = rest.strip_prefix(',') {
                rest = tail.trim_start();
            } else if !rest.starts_with('}') {
                return Err(invalid());
            }
        }
        if descr != Some(dtype) || fortran != Some(false) || dimensions.as_deref() != Some(shape) {
            return Err(invalid());
        }
        let item_size = match dtype {
            "<f4" => 4,
            "<f8" | "<i8" => 8,
            _ => return Err(invalid()),
        };
        let size = shape
            .iter()
            .try_fold(item_size, |n: usize, d| n.checked_mul(*d))
            .ok_or_else(invalid)?;
        let data = bytes.get(header_end..).ok_or_else(invalid)?;
        if data.len() != size {
            return Err(invalid());
        }
        Ok(data)
    }

    fn npy_string<'a>(rest: &mut &'a str) -> Option<&'a str> {
        let quote = rest.as_bytes().first().copied()?;
        if quote != b'\'' && quote != b'"' {
            return None;
        }
        let body = &rest[1..];
        let end = body.find(quote as char)?;
        let value = &body[..end];
        if value.contains('\\') {
            return None;
        }
        *rest = &body[end + 1..];
        Some(value)
    }

    fn sigmoid(x: f32) -> f32 {
        let z = (-x.abs()).exp();
        if x >= 0.0 {
            1.0 / (1.0 + z)
        } else {
            z / (1.0 + z)
        }
    }

    fn stable_topk_indices(scores: &[f32], k: usize) -> Vec<usize> {
        let mut indices: Vec<_> = (0..scores.len()).collect();
        // Stable descending sort keeps the lower index for ties (including signed zero).
        // NaNs sort last, as in NumPy argsort(-scores); model outputs are checked separately.
        indices.sort_by(|&a, &b| match (scores[a].is_nan(), scores[b].is_nan()) {
            (true, true) => std::cmp::Ordering::Equal,
            (true, false) => std::cmp::Ordering::Greater,
            (false, true) => std::cmp::Ordering::Less,
            (false, false) => scores[b].partial_cmp(&scores[a]).unwrap(),
        });
        indices.truncate(k);
        indices
    }

    struct SpeakerCache<'a> {
        constants: &'a Constants,
        embeds: Vec<f32>,
        probs: Vec<ProbFrame>,
        fifo: Vec<f32>,
        is_compressed: bool,
    }

    impl<'a> SpeakerCache<'a> {
        fn new(constants: &'a Constants) -> Self {
            Self {
                constants,
                embeds: Vec::new(),
                probs: Vec::new(),
                fifo: Vec::new(),
                is_compressed: false,
            }
        }

        fn get_embeds(&self) -> Vec<f32> {
            [self.embeds.as_slice(), self.fifo.as_slice()].concat()
        }

        fn pool_probs(&self, logits: &[f32]) -> Vec<ProbFrame> {
            let factor = self.constants.subsampling_factor;
            logits
                .chunks_exact(factor * NUM_SPEAKERS)
                .map(|block| {
                    let mut pooled = [0.0; NUM_SPEAKERS];
                    for frame in block.chunks_exact(NUM_SPEAKERS) {
                        for speaker in 0..NUM_SPEAKERS {
                            pooled[speaker] += sigmoid(frame[speaker]);
                        }
                    }
                    for value in &mut pooled {
                        *value /= factor as f32;
                    }
                    pooled
                })
                .collect()
        }

        fn num_popped_frames(&self, num_fifo_frames: usize) -> usize {
            if num_fifo_frames <= self.constants.fifo_length {
                return 0;
            }
            self.constants
                .speaker_cache_update_period
                .max(num_fifo_frames - self.constants.fifo_length)
                .min(num_fifo_frames)
        }

        fn update(
            &mut self,
            chunk_input_embeds: &[f32],
            chunk_logits: &[f32],
            num_chunk_frames: usize,
        ) {
            let num_cache_frames = self.probs.len();
            let num_fifo_frames = self.fifo.len() / HIDDEN_SIZE;
            let probs = self.pool_probs(chunk_logits);
            let chunk_start = num_cache_frames + num_fifo_frames;
            let chunk_embeds = &chunk_input_embeds
                [chunk_start * HIDDEN_SIZE..(chunk_start + num_chunk_frames) * HIDDEN_SIZE];
            let mut fifo_embeds = [self.fifo.as_slice(), chunk_embeds].concat();
            let num_popped = self.num_popped_frames(fifo_embeds.len() / HIDDEN_SIZE);
            if num_popped > 0 {
                let stored_probs = if self.is_compressed {
                    &self.probs[..]
                } else {
                    &probs[..num_cache_frames]
                };
                let cache_embeds = [
                    self.embeds.as_slice(),
                    &fifo_embeds[..num_popped * HIDDEN_SIZE],
                ]
                .concat();
                let cache_probs = [
                    stored_probs,
                    &probs[num_cache_frames..num_cache_frames + num_popped],
                ]
                .concat();
                fifo_embeds.drain(..num_popped * HIDDEN_SIZE);
                if cache_probs.len() > self.constants.speaker_cache_length {
                    (self.embeds, self.probs) = self.compress(&cache_embeds, &cache_probs);
                    self.is_compressed = true;
                } else {
                    self.embeds = cache_embeds;
                    self.probs = cache_probs;
                }
            }
            self.fifo = fifo_embeds;
        }

        fn get_frame_scores(&self, probs: &[ProbFrame]) -> Vec<ProbFrame> {
            let threshold = self.constants.prediction_score_threshold;
            let mut scores: Vec<ProbFrame> = probs
                .iter()
                .map(|frame| {
                    let log_probs = frame.map(|p| p.max(threshold).ln());
                    let lc = frame.map(|p| (1.0 - p).max(threshold).ln());
                    // NumPy uses a pairwise sum on this contiguous eight-element axis.
                    let sum =
                        ((lc[0] + lc[1]) + (lc[2] + lc[3])) + ((lc[4] + lc[5]) + (lc[6] + lc[7]));
                    std::array::from_fn(|s| {
                        if frame[s] > 0.5 {
                            log_probs[s] - lc[s] + sum - (-std::f32::consts::LN_2)
                        } else {
                            f32::NEG_INFINITY
                        }
                    })
                })
                .collect();
            for speaker in 0..NUM_SPEAKERS {
                if scores.iter().filter(|row| row[speaker] > 0.0).count()
                    >= self.constants.min_positive_scores
                {
                    for (score, prob) in scores.iter_mut().zip(probs) {
                        if score[speaker] <= 0.0 && prob[speaker] > 0.5 {
                            score[speaker] = f32::NEG_INFINITY;
                        }
                    }
                }
            }
            scores
        }

        fn boost_scores(scores: &mut [ProbFrame], num_boosted: usize, boost: f32) {
            if num_boosted == 0 {
                return;
            }
            for speaker in 0..NUM_SPEAKERS {
                let column: Vec<_> = scores.iter().map(|row| row[speaker]).collect();
                for index in stable_topk_indices(&column, num_boosted) {
                    scores[index][speaker] += boost;
                }
            }
        }

        fn compress(&self, embeds: &[f32], probs: &[ProbFrame]) -> (Vec<f32>, Vec<ProbFrame>) {
            let c = self.constants;
            let num_frames = probs.len();
            let mut scores = self.get_frame_scores(probs);
            for frame in &mut scores[c.speaker_cache_length..] {
                for score in frame {
                    *score += c.latest_frames_score_boost;
                }
            }
            Self::boost_scores(
                &mut scores,
                c.num_strong_boosted_frames,
                2.0 * std::f32::consts::LN_2,
            );
            Self::boost_scores(
                &mut scores,
                c.num_weak_boosted_frames,
                std::f32::consts::LN_2,
            );
            scores.extend(std::iter::repeat_n(
                [f32::INFINITY; NUM_SPEAKERS],
                c.num_silence_frames,
            ));
            let num_scored_frames = num_frames + c.num_silence_frames;
            let sentinel = num_scored_frames * NUM_SPEAKERS;
            // transpose(0, 2, 1).reshape: speaker-major, not frame-major.
            let flat_scores: Vec<_> = (0..NUM_SPEAKERS)
                .flat_map(|speaker| scores.iter().map(move |row| row[speaker]))
                .collect();
            let mut topk_idx = stable_topk_indices(&flat_scores, c.speaker_cache_length);
            for index in &mut topk_idx {
                if flat_scores[*index] == f32::NEG_INFINITY {
                    *index = sentinel;
                }
            }
            topk_idx.sort_unstable();
            let mut selected_embeds = Vec::with_capacity(c.speaker_cache_length * HIDDEN_SIZE);
            let mut selected_probs = Vec::with_capacity(c.speaker_cache_length);
            for index in topk_idx {
                let frame = if index == sentinel {
                    num_frames
                } else {
                    (index % num_scored_frames).min(num_frames)
                };
                if frame == num_frames {
                    selected_embeds.extend_from_slice(&c.silence_embeds);
                    selected_probs.push([0.0; NUM_SPEAKERS]);
                } else {
                    selected_embeds
                        .extend_from_slice(&embeds[frame * HIDDEN_SIZE..(frame + 1) * HIDDEN_SIZE]);
                    selected_probs.push(probs[frame]);
                }
            }
            (selected_embeds, selected_probs)
        }
    }

    #[derive(Debug)]
    struct Chunk {
        mel_start: usize,
        mel_end: usize,
        valid_mel_length: usize,
        num_chunk_frames: usize,
    }

    fn chunk_at(start_idx: usize, num_mel_frames: usize, c: &Constants) -> Chunk {
        let num_embeds = num_mel_frames.div_ceil(c.subsampling_factor);
        let end_idx = (start_idx + c.chunk_length).min(num_embeds);
        let mel_start = start_idx * c.subsampling_factor;
        let mel_end =
            ((end_idx + c.chunk_right_context) * c.subsampling_factor).min(num_mel_frames);
        Chunk {
            mel_start,
            mel_end,
            valid_mel_length: mel_end
                .min(num_mel_frames.saturating_sub(1))
                .saturating_sub(mel_start),
            num_chunk_frames: end_idx - start_idx,
        }
    }

    fn preemphasized_window(waveform: &[f32], start_frame: usize, end_frame: usize) -> Vec<f32> {
        let raw_start = (start_frame * HOP_LENGTH) as i64 - (N_FFT / 2) as i64;
        let raw_end = ((end_frame - 1) * HOP_LENGTH) as i64 + (N_FFT / 2) as i64;
        let mut result = vec![0.0; (raw_end - raw_start) as usize];
        let real_start = raw_start.max(0) as usize;
        let real_end = waveform.len().min(raw_end.max(0) as usize);
        // Subtraction sees only real samples, with one real sample of lookback.
        // Leaving the rest zero adds padding after preemphasis, never before it.
        for index in real_start..real_end {
            result[(index as i64 - raw_start) as usize] = if index == 0 {
                waveform[0]
            } else {
                waveform[index] - PREEMPHASIS * waveform[index - 1]
            };
        }
        result
    }

    fn zero_trailing_mel(mel: &mut [f32], end_frame: usize, total_frames: usize) {
        if end_frame == total_frames {
            let last = mel.len() - MEL_BINS;
            mel[last..].fill(0.0);
        }
    }

    fn extract_turns(active: &[u8]) -> Vec<Turn> {
        let mut turns = Vec::new();
        for speaker in 0..NUM_SPEAKERS {
            let mut start = None;
            // A zero at either boundary mirrors diff([zero, active, zero]).
            for (frame, mask) in active.iter().copied().chain(std::iter::once(0)).enumerate() {
                if mask & (1 << speaker) != 0 {
                    if start.is_none() {
                        start = Some(frame);
                    }
                } else if let Some(first) = start.take() {
                    turns.push(Turn {
                        start: first as f64 / 100.0,
                        end: frame as f64 / 100.0,
                        speaker,
                    });
                }
            }
        }
        turns.sort_by(|a, b| a.start.total_cmp(&b.start).then(a.speaker.cmp(&b.speaker)));
        turns
    }

    #[cfg(any(windows, all(target_os = "macos", target_arch = "aarch64")))]
    pub(super) fn run(
        model_dir: &std::path::Path,
        waveform: &[f32],
        c: &Constants,
    ) -> Result<Vec<Turn>, String> {
        use ort::{session::Session, value::Tensor};
        let threads = (std::thread::available_parallelism().map_or(1, usize::from) / 2).clamp(1, 8);
        let session = |filename: &str| -> Result<Session, String> {
            Session::builder()
                .map_err(|e| e.to_string())?
                .with_intra_threads(threads)
                .map_err(|e| e.to_string())?
                .commit_from_file(model_dir.join(filename))
                .map_err(|e| format!("Cannot load diarization graph {filename}: {e}"))
        };
        let mut preprocessor = session("preprocessor_core.onnx")?;
        let mut model = session("model.int8.onnx")?;
        let num_mel_frames = 1 + waveform.len() / HOP_LENGTH;
        let num_embeds = num_mel_frames.div_ceil(c.subsampling_factor);
        let mut cache = SpeakerCache::new(c);
        // One byte per 10 ms frame; no whole-file mel, embeddings, or logits.
        let mut active = Vec::with_capacity(num_mel_frames);
        for start_idx in (0..num_embeds).step_by(c.chunk_length) {
            let chunk = chunk_at(start_idx, num_mel_frames, c);
            let preemphasized = preemphasized_window(waveform, chunk.mel_start, chunk.mel_end);
            let input = Tensor::from_array(([1, preemphasized.len()], preemphasized))
                .map_err(|e| e.to_string())?;
            let output = preprocessor
                .run(ort::inputs!["preemphasized" => input])
                .map_err(|e| format!("Diarization preprocessing failed: {e}"))?;
            let mel_value = output
                .get("log_mel")
                .ok_or("Diarization preprocessor did not return log_mel.")?;
            let (shape, data) = mel_value
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let frames = chunk.mel_end - chunk.mel_start;
            if shape.as_ref() != [1, frames as i64, MEL_BINS as i64]
                || data.iter().any(|x| !x.is_finite())
            {
                return Err("Unexpected diarization log-mel output.".into());
            }
            let mut mel = data.to_vec();
            drop(output);
            zero_trailing_mel(&mut mel, chunk.mel_end, num_mel_frames);
            let context_embeds = cache.get_embeds();
            let context_length = context_embeds.len() / HIDDEN_SIZE;
            let mel =
                Tensor::from_array(([1, frames, MEL_BINS], mel)).map_err(|e| e.to_string())?;
            let mel_length =
                Tensor::from_array((Vec::<i64>::new(), vec![chunk.valid_mel_length as i64]))
                    .map_err(|e| e.to_string())?;
            let context = Tensor::from_array(([1, context_length, HIDDEN_SIZE], context_embeds))
                .map_err(|e| e.to_string())?;
            let context_len = Tensor::from_array((Vec::<i64>::new(), vec![context_length as i64]))
                .map_err(|e| e.to_string())?;
            let outputs = model
                .run(ort::inputs![
                    "chunk_mel" => mel, "chunk_mel_length" => mel_length,
                    "context_embeds" => context, "context_length" => context_len,
                ])
                .map_err(|e| format!("Speaker labeling failed: {e}"))?;
            let logits_value = outputs
                .get("logits")
                .ok_or("Diarization graph did not return logits.")?;
            let embeds_value = outputs
                .get("embeds")
                .ok_or("Diarization graph did not return embeds.")?;
            let (logit_shape, logits) = logits_value
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let (embed_shape, embeds) = embeds_value
                .try_extract_tensor::<f32>()
                .map_err(|e| e.to_string())?;
            let output_frames = context_length + frames.div_ceil(c.subsampling_factor);
            if logit_shape.as_ref()
                != [
                    1,
                    (output_frames * c.subsampling_factor) as i64,
                    NUM_SPEAKERS as i64,
                ]
                || embed_shape.as_ref() != [1, output_frames as i64, HIDDEN_SIZE as i64]
                || logits.iter().chain(embeds).any(|x| !x.is_finite())
            {
                return Err("Unexpected diarization graph output.".into());
            }
            cache.update(embeds, logits, chunk.num_chunk_frames);
            let start = context_length * c.subsampling_factor * NUM_SPEAKERS;
            let count =
                (chunk.num_chunk_frames * c.subsampling_factor).min(num_mel_frames - active.len());
            for frame in logits[start..start + count * NUM_SPEAKERS].chunks_exact(NUM_SPEAKERS) {
                let mask = frame
                    .iter()
                    .enumerate()
                    .fold(0u8, |mask, (speaker, &logit)| {
                        mask | (u8::from(sigmoid(logit) > 0.5) << speaker)
                    });
                active.push(mask);
            }
        }
        Ok(extract_turns(&active))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn npy(header: &str, data: &[u8]) -> Vec<u8> {
            let header = format!("{header}\n");
            let mut bytes = b"\x93NUMPY\x01\x00".to_vec();
            bytes.extend_from_slice(&(header.len() as u16).to_le_bytes());
            bytes.extend_from_slice(header.as_bytes());
            bytes.extend_from_slice(data);
            bytes
        }

        fn entries() -> BTreeMap<String, Vec<u8>> {
            let mut entries = BTreeMap::new();
            for (name, value) in INTEGER_CONSTANTS {
                entries.insert(
                    format!("{name}.npy"),
                    npy(
                        "{'descr': '<i8', 'fortran_order': False, 'shape': (), }",
                        &value.to_le_bytes(),
                    ),
                );
            }
            for (name, value) in FLOAT_CONSTANTS {
                entries.insert(
                    format!("{name}.npy"),
                    npy(
                        "{'descr': '<f8', 'fortran_order': False, 'shape': (), }",
                        &value.to_le_bytes(),
                    ),
                );
            }
            let silence: Vec<_> = (0..HIDDEN_SIZE)
                .flat_map(|_| (-1.0f32).to_le_bytes())
                .collect();
            entries.insert(
                "silence_embeds.npy".into(),
                npy(
                    "{'descr': '<f4', 'fortran_order': False, 'shape': (512,), }",
                    &silence,
                ),
            );
            entries
        }

        fn constants() -> Constants {
            Constants::from_entries(entries()).unwrap()
        }

        #[test]
        fn constants_require_pinned_values_and_complete_numeric_arrays() {
            let c = constants();
            assert_eq!(
                (
                    c.min_positive_scores,
                    c.num_strong_boosted_frames,
                    c.num_weak_boosted_frames
                ),
                (16, 24, 48)
            );
            assert_eq!(c.silence_embeds, vec![-1.0; HIDDEN_SIZE]);
            let mut changed = entries();
            *changed
                .get_mut("chunk_length.npy")
                .unwrap()
                .last_mut()
                .unwrap() = 1;
            assert!(Constants::from_entries(changed).is_err());
            let mut missing = entries();
            missing.remove("fifo_length.npy");
            assert!(Constants::from_entries(missing).is_err());
            let mut extra = entries();
            extra.insert("unknown.npy".into(), Vec::new());
            assert!(Constants::from_entries(extra).is_err());
            let mut truncated = entries();
            truncated.get_mut("silence_embeds.npy").unwrap().pop();
            assert!(Constants::from_entries(truncated).is_err());
            let mut nonfinite = entries();
            let silence = nonfinite.get_mut("silence_embeds.npy").unwrap();
            let end = silence.len();
            silence[end - 4..].copy_from_slice(&f32::NAN.to_le_bytes());
            assert!(Constants::from_entries(nonfinite).is_err());
        }

        #[test]
        fn npy_rejects_wrong_dtype_shape_order_and_malformed_headers() {
            for header in [
                "{'descr': '>i8', 'fortran_order': False, 'shape': ()}",
                "{'descr': '<i8', 'fortran_order': True, 'shape': ()}",
                "{'descr': '<i8', 'fortran_order': False, 'shape': (1,)}",
                "{'descr': '<i8', 'fortran_order': False, 'shape': (), 'shape': ()}",
                "{'descr': '<i8', 'fortran_order': False, 'shape': ()} garbage",
                "{'descr': '<i8', 'fortran_order': False, 'shape': (,) }",
                "{'descr': '<i8', 'fortran_order': False, 'shape': (1) }",
            ] {
                assert!(
                    npy_data(&npy(header, &8i64.to_le_bytes()), "<i8", &[]).is_err(),
                    "{header}"
                );
            }
            let valid = npy(
                "{\"shape\": (), \"fortran_order\": False, \"descr\": \"<i8\"}",
                &8i64.to_le_bytes(),
            );
            assert_eq!(npy_data(&valid, "<i8", &[]).unwrap(), 8i64.to_le_bytes());
            for end in 0..valid.len() {
                assert!(npy_data(&valid[..end], "<i8", &[]).is_err());
            }
        }

        #[test]
        fn window_preemphasis_matches_full_signal_at_file_and_chunk_boundaries() {
            for length in [0, 1, 159, 160, 161, 511, 512, 513, 8001] {
                let waveform: Vec<_> = (0..length)
                    .map(|i| ((i * 13 % 101) as f32 - 50.0) / 50.0)
                    .collect();
                let mut full = waveform.clone();
                for index in 1..length {
                    full[index] -= PREEMPHASIS * waveform[index - 1];
                }
                let frames = 1 + length / HOP_LENGTH;
                for start in 0..frames {
                    let end = (start + 3).min(frames);
                    let result = preemphasized_window(&waveform, start, end);
                    assert_eq!(result.len(), (end - start - 1) * HOP_LENGTH + N_FFT);
                    for (offset, sample) in result.iter().enumerate() {
                        let raw = (start * HOP_LENGTH + offset) as i64 - (N_FFT / 2) as i64;
                        let expected = if raw < 0 {
                            0.0
                        } else {
                            full.get(raw as usize).copied().unwrap_or(0.0)
                        };
                        assert_eq!(
                            *sample, expected,
                            "length={length}, start={start}, raw={raw}"
                        );
                    }
                }
            }
        }

        #[test]
        fn mel_geometry_keeps_lookahead_and_excludes_only_file_last_frame() {
            let c = constants();
            let first = chunk_at(0, 4001, &c);
            assert_eq!(
                (
                    first.mel_start,
                    first.mel_end,
                    first.valid_mel_length,
                    first.num_chunk_frames
                ),
                (0, 3040, 3040, 340)
            );
            let last = chunk_at(340, 4001, &c);
            assert_eq!(
                (
                    last.mel_start,
                    last.mel_end,
                    last.valid_mel_length,
                    last.num_chunk_frames
                ),
                (2720, 4001, 1280, 161)
            );
            let empty_audio = chunk_at(0, 1, &c);
            assert_eq!(
                (
                    empty_audio.mel_end,
                    empty_audio.valid_mel_length,
                    empty_audio.num_chunk_frames
                ),
                (1, 0, 1)
            );
            let mut mel = vec![3.0; 2 * MEL_BINS];
            zero_trailing_mel(&mut mel, 2, 3);
            assert!(mel.iter().all(|v| *v == 3.0));
            zero_trailing_mel(&mut mel, 3, 3);
            assert!(mel[..MEL_BINS].iter().all(|v| *v == 3.0));
            assert!(mel[MEL_BINS..].iter().all(|v| *v == 0.0));
        }

        #[test]
        fn stable_topk_and_boost_keep_infinities_and_lower_index_ties() {
            assert_eq!(
                stable_topk_indices(&[f32::NEG_INFINITY, 2.0, 2.0, f32::INFINITY, -0.0, 0.0], 6),
                vec![3, 1, 2, 4, 5, 0]
            );
            let mut scores = vec![[2.0; NUM_SPEAKERS]; 3];
            scores[2][0] = f32::NEG_INFINITY;
            SpeakerCache::boost_scores(&mut scores, 1, 1.0);
            assert_eq!(scores[0], [3.0; NUM_SPEAKERS]);
            assert_eq!(scores[1], [2.0; NUM_SPEAKERS]);
            SpeakerCache::boost_scores(&mut scores, 3, 1.0);
            assert_eq!(scores[2][0], f32::NEG_INFINITY);
        }

        #[test]
        fn pooling_applies_sigmoid_before_mean_and_is_stable_at_extremes() {
            assert_eq!(sigmoid(f32::INFINITY), 1.0);
            assert_eq!(sigmoid(f32::NEG_INFINITY), 0.0);
            assert_eq!(sigmoid(0.0), 0.5);
            let c = constants();
            let cache = SpeakerCache::new(&c);
            let mut logits = vec![0.0; 8 * NUM_SPEAKERS];
            logits[..4 * NUM_SPEAKERS].fill(1000.0);
            assert_eq!(cache.pool_probs(&logits), vec![[0.75; NUM_SPEAKERS]]);
            assert_eq!(
                [0, 40, 41, 299, 340, 380, 400].map(|n| cache.num_popped_frames(n)),
                [0, 0, 41, 299, 300, 340, 360]
            );
        }

        #[test]
        fn cache_update_preserves_fifo_embeddings_and_switches_probability_source() {
            for compressed in [false, true] {
                let mut c = constants();
                c.fifo_length = 1;
                c.speaker_cache_update_period = 1;
                let mut cache = SpeakerCache::new(&c);
                cache.embeds = vec![10.0; HIDDEN_SIZE];
                cache.probs = vec![[0.25; NUM_SPEAKERS]];
                cache.fifo = vec![20.0; HIDDEN_SIZE];
                cache.is_compressed = compressed;
                // Context outputs are deliberately changed: old embeddings must survive.
                let mut embeds = vec![99.0; 3 * HIDDEN_SIZE];
                embeds[2 * HIDDEN_SIZE..].fill(30.0);
                cache.update(&embeds, &vec![0.0; 3 * 8 * NUM_SPEAKERS], 1);
                assert_eq!(cache.embeds[..HIDDEN_SIZE], vec![10.0; HIDDEN_SIZE]);
                assert_eq!(cache.embeds[HIDDEN_SIZE..], vec![20.0; HIDDEN_SIZE]);
                assert_eq!(cache.fifo, vec![30.0; HIDDEN_SIZE]);
                assert_eq!(
                    cache.probs,
                    vec![
                        [if compressed { 0.25 } else { 0.5 }; NUM_SPEAKERS],
                        [0.5; NUM_SPEAKERS]
                    ]
                );
                assert_eq!(cache.get_embeds().len(), 3 * HIDDEN_SIZE);
            }
        }

        #[test]
        fn compression_sentinels_replace_negative_infinity_with_silence() {
            let c = constants();
            let cache = SpeakerCache::new(&c);
            let (embeds, probs) = cache.compress(
                &vec![7.0; 300 * HIDDEN_SIZE],
                &vec![[0.0; NUM_SPEAKERS]; 300],
            );
            assert_eq!(embeds, vec![-1.0; c.speaker_cache_length * HIDDEN_SIZE]);
            assert_eq!(probs, vec![[0.0; NUM_SPEAKERS]; c.speaker_cache_length]);
        }

        #[test]
        fn compression_matches_numpy_reference_frame_selection() {
            // Generated from NumpySpeakerCache._compress in artifacts/diarization/diarize.py.
            // Includes overlap, positive-score filtering, both boosts, recent frames and ties.
            let c = constants();
            let cache = SpeakerCache::new(&c);
            let embeds: Vec<_> = (0..600)
                .flat_map(|i| std::iter::repeat_n(i as f32, HIDDEN_SIZE))
                .collect();
            let probs: Vec<_> = (0..600)
                .map(|i| {
                    let mut row = [0.03; NUM_SPEAKERS];
                    row[i % NUM_SPEAKERS] = if i % 5 == 0 {
                        0.6
                    } else {
                        0.85 + (i % 7) as f32 / 100.0
                    };
                    if i % 11 == 0 {
                        row.fill(0.62);
                    }
                    row
                })
                .collect();
            let (selected, selected_probs) = cache.compress(&embeds, &probs);
            let ids: Vec<_> = selected
                .chunks_exact(HIDDEN_SIZE)
                .map(|row| row[0] as i32)
                .collect();
            assert_eq!(ids.len(), 264);
            assert_eq!(ids.iter().filter(|&&id| id == -1).count(), 8);
            assert_eq!(ids.iter().sum::<i32>(), 96484);
            let fingerprint = ids
                .iter()
                .flat_map(|id| id.to_le_bytes())
                .fold(14695981039346656037u64, |hash, byte| {
                    (hash ^ u64::from(byte)).wrapping_mul(1099511628211)
                });
            assert_eq!(fingerprint, 0x008bebaf10e8541c);
            for (id, selected_prob) in ids.iter().zip(selected_probs) {
                assert_eq!(
                    selected_prob,
                    if *id == -1 {
                        [0.0; NUM_SPEAKERS]
                    } else {
                        probs[*id as usize]
                    }
                );
            }
        }

        #[test]
        fn turn_edges_include_overlap_file_boundaries_and_start_speaker_order() {
            assert!(extract_turns(&[]).is_empty());
            assert!(extract_turns(&[0, 0]).is_empty());
            assert_eq!(
                extract_turns(&[3, 1, 0, 2, 2]),
                vec![
                    Turn {
                        start: 0.0,
                        end: 0.02,
                        speaker: 0
                    },
                    Turn {
                        start: 0.0,
                        end: 0.01,
                        speaker: 1
                    },
                    Turn {
                        start: 0.03,
                        end: 0.05,
                        speaker: 1
                    },
                ]
            );
        }
    }
}
