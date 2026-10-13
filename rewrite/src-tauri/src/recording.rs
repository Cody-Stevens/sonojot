//! Synchronized, bounded-memory meeting recording. Capture timestamps share one
//! monotonic epoch; all three PCM files use that timeline, including silence.
use crate::{
    audio::{capture_event, AudioMessage, Capture, OverflowPolicy},
    system_audio::{SystemAudioCapture, SystemAudioChunk},
};
use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const RATE: u32 = 16_000;
const BLOCK: usize = 800;
const REORDER_DELAY: Duration = Duration::from_millis(750);
const MAX_PENDING: usize = RATE as usize * 6;
const MAX_CHUNK_SECONDS: usize = 2;
const FILTER_RADIUS: i64 = 24;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueSource {
    Microphone,
    System,
}
impl IssueSource {
    fn label(self) -> &'static str {
        match self { Self::Microphone => "Microphone", Self::System => "System audio" }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IssueKind { Disconnected, Overflow, InvalidBuffer, Discontinuity }

#[derive(Clone, Debug)]
pub struct SourceIssue {
    pub source: IssueSource,
    pub started_at_seconds: f64,
    pub ended_at_seconds: Option<f64>,
    pub kind: IssueKind,
    pub occurrences: u64,
    pub lost_duration_seconds: f64,
    pub message: String,
}

impl SourceIssue {
    /// Aggregated endpoints cover healthy audio too. Only an individual gap,
    /// a continuous loss, or an unrecovered source may become a marker.
    pub(crate) fn is_outage(&self) -> bool {
        self.ended_at_seconds.is_none() || (self.lost_duration_seconds > 3.0
            && (self.occurrences == 1 || self.ended_at_seconds.is_some_and(|end| {
                (end - self.started_at_seconds - self.lost_duration_seconds).abs() < 0.000_001
            })))
    }
}

#[derive(Clone, Debug)]
pub struct RecordingOutcome {
    pub duration: f64,
    pub issues: Vec<SourceIssue>,
}
impl RecordingOutcome {
    pub fn warnings(&self) -> Vec<String> {
        self.issues.iter().map(|issue| issue.message.clone()).collect()
    }
}

#[derive(Default)]
struct SourceNotices {
    // A recovery notice expires; an open gap stays visible until audio returns.
    notices: [Option<(String, Option<Instant>)>; 2],
    warnings: Arc<Mutex<Vec<String>>>,
    issues: Arc<Mutex<Vec<SourceIssue>>>,
}
type SharedNotices = Arc<Mutex<SourceNotices>>;

fn source_index(source: IssueSource) -> usize {
    usize::from(source == IssueSource::System)
}
fn clock_text(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

#[derive(Clone, Debug)]
pub struct RecordingPaths {
    pub audio: PathBuf,
    pub microphone: PathBuf,
    pub system: PathBuf,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Source {
    Microphone,
    System,
    Both,
}
impl Source {
    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "microphone" => Ok(Self::Microphone),
            "system" => Ok(Self::System),
            "both" => Ok(Self::Both),
            _ => Err("Choose microphone, system, or both for the meeting source.".into()),
        }
    }
    fn microphone(self) -> bool {
        self != Self::System
    }
    fn system(self) -> bool {
        self != Self::Microphone
    }
}

type SharedError = Arc<Mutex<Option<String>>>;
fn report(error: &SharedError, message: impl Into<String>) {
    let mut guard = error.lock().unwrap_or_else(|poison| poison.into_inner());
    if guard.is_none() {
        *guard = Some(message.into());
    }
}
fn current_error(error: &SharedError) -> Option<String> {
    error
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .clone()
}

/// WAV writes and source supervision remain on one owner thread; native streams
/// keep their existing dedicated owners. Drop still requests a draining stop.
pub struct MeetingRecorder {
    stop: Arc<AtomicBool>,
    error: SharedError,
    level: Arc<AtomicU32>,
    /// Microphone and system levels; reported only when both are captured.
    stems: Option<StemLevels>,
    notices: SharedNotices,
    done: Receiver<Result<RecordingOutcome, String>>,
    worker: Option<JoinHandle<()>>,
}

/// A timeout transfers ownership of the completion receiver; it is not a
/// recording failure and does not make the checkpoint safe to consume.
pub enum RecordingStop {
    Complete(Result<RecordingOutcome, String>),
    Finalizing(MeetingRecorder),
}

impl RecordingStop {
    /// Complete now or transfer the still-owned receiver to a background task.
    /// The same callback reconciles metadata and starts any requested processing.
    pub(crate) fn finish_with<T: Send + 'static>(
        self,
        finish: impl FnOnce(Result<RecordingOutcome, String>) -> Result<T, String> + Send + 'static,
        on_error: impl FnOnce(String) + Send + 'static,
    ) -> Result<Option<T>, String> {
        match self {
            Self::Complete(result) => finish(result).map(Some),
            Self::Finalizing(recorder) => {
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) = finish(recorder.wait()) {
                        on_error(error);
                    }
                });
                Ok(None)
            }
        }
    }
}

impl MeetingRecorder {
    pub fn start(
        paths: RecordingPaths,
        source: &str,
        microphone: Option<String>,
    ) -> Result<Self, String> {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Meeting)?;
        let source = Source::parse(source)?;
        let writers = Writers::create(&paths)?;
        let stop = Arc::new(AtomicBool::new(false));
        let error: SharedError = Arc::new(Mutex::new(None));
        let level = Arc::new(AtomicU32::new(0.0f32.to_bits()));
        let thread_stop = stop.clone();
        let thread_error = error.clone();
        let thread_level = level.clone();
        let stem_levels = StemLevels::default();
        let thread_stems = stem_levels.clone();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let thread_notices = notices.clone();
        let stems = matches!(source, Source::Both).then_some(stem_levels);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let (done_tx, done) = mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("meeting-recording".into())
            .spawn(move || {
                let _activity = activity;
                let result = record(
                    writers,
                    source,
                    microphone,
                    &thread_stop,
                    &thread_error,
                    thread_level,
                    thread_stems,
                    &thread_notices,
                    ready_tx,
                );
                if let Err(message) = &result {
                    report(&thread_error, message.clone());
                }
                let _ = done_tx.send(result);
            })
            .map_err(|e| format!("Cannot start meeting recorder: {e}"))?;
        match ready_rx.recv_timeout(Duration::from_secs(20)) {
            Ok(Ok(())) => Ok(Self {
                stop,
                error,
                level,
                stems,
                notices,
                done,
                worker: Some(worker),
            }),
            Ok(Err(message)) => {
                stop.store(true, Ordering::Release);
                // Failure is announced after capture shutdown and WAV finalization.
                let _ = worker.join();
                Err(message)
            }
            Err(_) => {
                stop.store(true, Ordering::Release);
                Err("Meeting capture did not become ready within 20 seconds. Any captured audio will be preserved.".into())
            }
        }
    }

    pub fn error(&self) -> Option<String> {
        current_error(&self.error)
    }

    /// RMS amplitude of the latest mixed output block, bounded to 0..=1.
    /// The value follows the recorder's alignment delay and returns to zero
    /// during silence or when the capture worker finishes.
    pub fn level(&self) -> f32 {
        f32::from_bits(self.level.load(Ordering::Relaxed))
    }

    /// (microphone, system) RMS of the latest block, for a two-source meeting.
    pub fn stem_levels(&self) -> Option<(f32, f32)> {
        self.stems.as_ref().map(|stems| {
            (
                f32::from_bits(stems.0[0].load(Ordering::Relaxed)),
                f32::from_bits(stems.0[1].load(Ordering::Relaxed)),
            )
        })
    }

    pub fn source_status(&self) -> Vec<String> {
        self.notices.lock().unwrap_or_else(|p| p.into_inner()).notices.iter()
            .flatten().filter(|(_, until)| until.is_none_or(|until| Instant::now() < until))
            .map(|(message, _)| message.clone()).collect()
    }

    pub(crate) fn recording_warnings_handle(&self) -> Arc<Mutex<Vec<String>>> {
        self.notices.lock().unwrap_or_else(|p| p.into_inner()).warnings.clone()
    }
    pub(crate) fn recording_issues_handle(&self) -> Arc<Mutex<Vec<SourceIssue>>> {
        self.notices.lock().unwrap_or_else(|p| p.into_inner()).issues.clone()
    }

    pub fn stop(self) -> RecordingStop {
        self.stop_timeout(Duration::from_secs(15))
    }

    pub(crate) fn request_stop(&self) {
        self.stop.store(true, Ordering::Release);
    }

    fn stop_timeout(mut self, timeout: Duration) -> RecordingStop {
        self.request_stop();
        match self.done.recv_timeout(timeout) {
            Ok(result) => RecordingStop::Complete(self.reap(result)),
            Err(mpsc::RecvTimeoutError::Timeout) => RecordingStop::Finalizing(self),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                RecordingStop::Complete(self.reap(Err(
                    "The meeting recorder stopped unexpectedly; partial WAV files are preserved.".into(),
                )))
            }
        }
    }

    /// Only a background task may wait here. The result is sent after the WAV
    /// writers finish; a disconnected channel confirms the worker has exited.
    pub fn wait(mut self) -> Result<RecordingOutcome, String> {
        let result = self.done.recv().unwrap_or_else(|_| Err(
            "The meeting recorder stopped unexpectedly; partial WAV files are preserved.".into(),
        ));
        self.reap(result)
    }

    fn reap(&mut self, result: Result<RecordingOutcome, String>) -> Result<RecordingOutcome, String> {
        if let Some(worker) = self.worker.take() {
            if worker.join().is_err() {
                return Err(
                    "The meeting recorder stopped unexpectedly; partial audio is preserved.".into(),
                );
            }
        }
        result
    }
}
impl Drop for MeetingRecorder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}

mod issues;
mod supervisor;
use supervisor::record;

fn frames(time: Duration) -> u64 {
    (time.as_nanos() * RATE as u128 / 1_000_000_000).min(u64::MAX as u128) as u64
}
fn pcm16(sample: f32) -> i16 {
    if !sample.is_finite() {
        return 0;
    }
    (sample.clamp(-1.0, 1.0) * i16::MAX as f32).round() as i16
}
fn mix_sample(microphone: f32, system: f32, source: Source) -> f32 {
    match source {
        Source::Microphone => microphone,
        Source::System => system,
        Source::Both => {
            let mixed = microphone * 1.65 + system * 0.55;
            mixed / (1.0 + mixed.abs())
        }
    }
}

fn rms_level(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let energy: f32 = samples
        .iter()
        .map(|&sample| {
            let sample = if sample.is_finite() {
                sample.clamp(-1.0, 1.0)
            } else {
                0.0
            };
            sample * sample
        })
        .sum();
    (energy / samples.len() as f32).sqrt().clamp(0.0, 1.0)
}

#[derive(Clone, Default)]
struct StemLevels(Arc<[AtomicU32; 2]>);

struct Mixer {
    stems: StemLevels,
    writers: Writers,
    source: Source,
    cursor: u64,
    microphone: Track,
    system: Track,
    level: Arc<AtomicU32>,
}
impl Mixer {
    fn new(writers: Writers, source: Source, level: Arc<AtomicU32>) -> Self {
        Self {
            stems: StemLevels::default(),
            writers,
            source,
            cursor: 0,
            microphone: Track::default(),
            system: Track::default(),
            level,
        }
    }
    fn push(
        &mut self,
        microphone: bool,
        samples: &[f32],
        sample_rate: u32,
        at: Duration,
        discontinuity: bool,
    ) -> Result<(), String> {
        let track = if microphone {
            &mut self.microphone
        } else {
            &mut self.system
        };
        track.push(samples, sample_rate, at, discontinuity, self.cursor)
    }
    fn flush_resamplers(&mut self) -> Result<(), String> {
        // Flush each source even when one fails, to preserve the other source.
        let mic = self.microphone.flush(self.cursor);
        let system = self.system.flush(self.cursor);
        mic.and(system)
    }
    fn last_frame(&self) -> u64 {
        self.microphone.end.max(self.system.end).max(self.cursor)
    }
    fn write_until(&mut self, end: u64, finalizing: bool) -> Result<(), String> {
        // WASAPI may stop delivering packets while the desktop is silent. Seal
        // the old filter tail before writing that silence, rather than leaving
        // a few held samples to arrive late when playback eventually resumes.
        let microphone = self.microphone.seal_before(end, self.cursor);
        let system = self.system.seal_before(end, self.cursor);
        let mut failure = microphone.and(system).err();
        if let Err(message) = self.repair_late_samples() { failure.get_or_insert(message); }
        if !finalizing {
            if let Some(message) = failure.take() { return Err(message); }
        }
        while self.cursor < end {
            let count = (end - self.cursor).min(BLOCK as u64) as usize;
            if !finalizing && count < BLOCK {
                break;
            }
            let mic = self.microphone.take(count);
            let system = self.system.take(count);
            let mix: Vec<f32> = mic
                .iter()
                .zip(&system)
                .map(|(&m, &s)| mix_sample(m, s, self.source))
                .collect();
            // Attempt every file even after one fails, so available isolated
            // tracks are preserved when the mix path encounters an I/O error.
            self.stems.0[0].store(rms_level(&mic).to_bits(), Ordering::Relaxed);
            self.stems.0[1].store(rms_level(&system).to_bits(), Ordering::Relaxed);
            let mixed = self.writers.audio.write_timeline_samples(self.cursor, &mix);
            let microphone = self.writers.microphone.write_timeline_samples(self.cursor, &mic);
            let system = self.writers.system.write_timeline_samples(self.cursor, &system);
            self.cursor += count as u64;
            if let Err(message) = mixed.and(microphone).and(system) {
                if !finalizing { return Err(message); }
                failure.get_or_insert(message);
            }
            self.level
                .store(rms_level(&mix).to_bits(), Ordering::Relaxed);
        }
        failure.map_or(Ok(()), Err)
    }

    fn repair_late_samples(&mut self) -> Result<(), String> {
        let mut failure = None;
        for microphone in [true, false] {
            let repairs = if microphone { &mut self.microphone.repairs } else { &mut self.system.repairs };
            for (start, samples) in std::mem::take(repairs) {
                let (stem, other) = if microphone { (&mut self.writers.microphone, &mut self.writers.system) }
                    else { (&mut self.writers.system, &mut self.writers.microphone) };
                let other_samples = other.read_samples_at(start, samples.len());
                let saved_stem = stem.write_samples_at(start, &samples);
                let saved_mix = other_samples.and_then(|other_samples| {
                    let mixed: Vec<_> = samples.iter().zip(other_samples).map(|(&sample, other)| {
                        if microphone { mix_sample(sample, other, self.source) } else { mix_sample(other, sample, self.source) }
                    }).collect();
                    self.writers.audio.write_samples_at(start, &mixed)
                });
                if let Err(message) = saved_stem.and(saved_mix) { failure.get_or_insert(message); }
            }
        }
        failure.map_or(Ok(()), Err)
    }
}

impl Drop for Mixer {
    fn drop(&mut self) {
        for stem in self.stems.0.iter() {
            stem.store(0.0f32.to_bits(), Ordering::Relaxed);
        }
        self.level.store(0.0f32.to_bits(), Ordering::Relaxed);
    }
}

#[derive(Default)]
struct Track {
    pending: VecDeque<f32>,
    repairs: Vec<(u64, Vec<f32>)>,
    resampler: Option<Resampler>,
    end: u64,
}
impl Track {
    fn seal_before(&mut self, end: u64, cursor: u64) -> Result<(), String> {
        let seal = self.resampler.as_ref().is_some_and(|resampler| {
            let input_end = (resampler.origin + resampler.received as f64 / resampler.rate as f64)
                * RATE as f64;
            input_end.ceil() as u64 <= end
        });
        if seal {
            self.flush(cursor)?;
            self.resampler = None;
        }
        Ok(())
    }

    fn push(
        &mut self,
        samples: &[f32],
        rate: u32,
        at: Duration,
        discontinuity: bool,
        cursor: u64,
    ) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        if !(8_000..=384_000).contains(&rate) || samples.len() > rate as usize * MAX_CHUNK_SECONDS {
            return Err("Capture returned an invalid sample rate or oversized audio buffer; partial audio is preserved.".into());
        }
        if discontinuity || self.resampler.as_ref().is_some_and(|previous| previous.rate != rate) {
            self.flush(cursor)?;
            self.resampler = None;
        }
        let timestamp = at.as_secs_f64();
        if let Some(previous) = self.resampler.as_mut() {
            let expected = previous.origin + previous.received as f64 / rate as f64;
            // Follow ordinary device-clock drift over roughly one second. This
            // keeps independent capture clocks aligned without repeatedly
            // rounding callback lengths or treating slow drift as packet loss.
            // A real pause starts a new segment and leaves silence in the gap.
            if (timestamp - expected).abs() > 0.020 {
                if timestamp < expected {
                    return Err(
                        "An audio device timestamp moved backwards; this meeting is incomplete."
                            .into(),
                    );
                }
                self.flush(cursor)?;
                self.resampler = None;
            } else {
                let fraction = (samples.len() as f64 / rate as f64).min(0.1);
                let correction = ((timestamp - expected) * fraction).clamp(-0.000_1, 0.000_1);
                previous.origin += correction;
            }
        }
        let resampler = self
            .resampler
            .get_or_insert_with(|| Resampler::new(rate, timestamp));
        let (start, output) = resampler.push(samples, false);
        self.append(start, &output, cursor)
    }
    fn flush(&mut self, cursor: u64) -> Result<(), String> {
        if let Some(resampler) = self.resampler.as_mut() {
            let (start, output) = resampler.push(&[], true);
            self.append(start, &output, cursor)?;
        }
        Ok(())
    }
    fn append(&mut self, start: u64, samples: &[f32], cursor: u64) -> Result<(), String> {
        if samples.is_empty() {
            return Ok(());
        }
        if start < self.end {
            return Err("Audio device buffers overlapped on the meeting timeline; partial files are preserved.".into());
        }
        // Repair late samples at their original positions instead of moving them
        // forward in time or discarding audio after the reorder delay.
        let late = (cursor.saturating_sub(start) as usize).min(samples.len());
        if late > 0 {
            if self.repairs.iter().map(|(_, data)| data.len()).sum::<usize>() + late > MAX_PENDING {
                return Err("Late audio exceeded the six-second timing buffer.".into());
            }
            self.repairs.push((start, samples[..late].to_vec()));
        }
        let original_end = start + samples.len() as u64;
        let start = start + late as u64;
        let samples = &samples[late..];
        if samples.is_empty() { self.end = original_end; return Ok(()); }
        let offset =
            usize::try_from(start - cursor).map_err(|_| "Invalid audio timestamp.".to_string())?;
        let needed = offset
            .checked_add(samples.len())
            .ok_or("Invalid audio buffer size.")?;
        if needed > MAX_PENDING {
            return Err(
                "Meeting audio exceeded the six-second timing buffer; partial files are preserved."
                    .into(),
            );
        }
        self.pending.resize(self.pending.len().max(needed), 0.0);
        for (index, sample) in samples.iter().enumerate() {
            self.pending[offset + index] = *sample;
        }
        self.end = start + samples.len() as u64;
        Ok(())
    }
    fn take(&mut self, count: usize) -> Vec<f32> {
        (0..count)
            .map(|_| self.pending.pop_front().unwrap_or(0.0))
            .collect()
    }
}

/// Streaming windowed-sinc resampling retains only the current device buffer and
/// filter history. Absolute output sample positions prevent per-buffer rounding
/// drift (notably 44.1 kHz input); the symmetric filter does not shift timestamps.
struct Resampler {
    rate: u32,
    origin: f64,
    received: u64,
    first: i64,
    data: VecDeque<f32>,
    next: u64,
}
impl Resampler {
    fn new(rate: u32, origin: f64) -> Self {
        Self {
            rate,
            origin,
            received: 0,
            first: 0,
            data: VecDeque::new(),
            next: (origin * RATE as f64 - 1e-6).ceil().max(0.0) as u64,
        }
    }
    fn push(&mut self, samples: &[f32], finish: bool) -> (u64, Vec<f32>) {
        self.data.extend(samples.iter().map(|&s| {
            if s.is_finite() {
                s.clamp(-1.0, 1.0)
            } else {
                0.0
            }
        }));
        self.received += samples.len() as u64;
        let start = self.next;
        let mut output = Vec::new();
        let cutoff = (RATE as f64 / self.rate as f64).min(1.0) * 0.94;
        let input_end = self.received as f64;
        loop {
            let position = (self.next as f64 / RATE as f64 - self.origin) * self.rate as f64;
            if position >= input_end - 1e-7 || self.data.is_empty() {
                break;
            }
            if !finish && position.floor() as i64 + FILTER_RADIUS >= self.received as i64 {
                break;
            }
            let center = position.floor() as i64;
            let mut total = 0.0;
            let mut weight_sum = 0.0;
            for index in center - FILTER_RADIUS + 1..=center + FILTER_RADIUS {
                let distance = position - index as f64;
                if distance.abs() >= FILTER_RADIUS as f64 {
                    continue;
                }
                let phase = std::f64::consts::PI * distance;
                let sinc = if phase.abs() < 1e-12 {
                    cutoff
                } else {
                    (phase * cutoff).sin() / phase
                };
                let window_phase = phase / FILTER_RADIUS as f64;
                let window = 0.42 + 0.5 * window_phase.cos() + 0.08 * (2.0 * window_phase).cos();
                let weight = sinc * window;
                let actual = index.clamp(0, self.received as i64 - 1);
                let offset = (actual - self.first).max(0) as usize;
                let value = self
                    .data
                    .get(offset)
                    .copied()
                    .unwrap_or_else(|| *self.data.back().unwrap_or(&0.0));
                total += value as f64 * weight;
                weight_sum += weight;
            }
            output.push(if weight_sum.abs() > 1e-12 {
                (total / weight_sum) as f32
            } else {
                0.0
            });
            self.next += 1;
        }
        let next_position = (self.next as f64 / RATE as f64 - self.origin) * self.rate as f64;
        let keep_from = (next_position.floor() as i64 - FILTER_RADIUS).max(0);
        while self.first < keep_from && self.data.len() > 1 {
            self.data.pop_front();
            self.first += 1;
        }
        (start, output)
    }
}

struct Writers {
    audio: PcmWriter,
    microphone: PcmWriter,
    system: PcmWriter,
}
impl Writers {
    fn create(paths: &RecordingPaths) -> Result<Self, String> {
        if paths.audio == paths.microphone
            || paths.audio == paths.system
            || paths.microphone == paths.system
        {
            return Err("Meeting mix and source recordings need three different paths.".into());
        }
        for path in [&paths.audio, &paths.microphone, &paths.system] {
            if path.exists() {
                return Err(format!(
                    "Refusing to overwrite an existing meeting recording: {}",
                    path.display()
                ));
            }
        }
        Ok(Self {
            audio: PcmWriter::create(&paths.audio)?,
            microphone: PcmWriter::create(&paths.microphone)?,
            system: PcmWriter::create(&paths.system)?,
        })
    }
    fn checkpoint(&mut self) -> Result<(), String> {
        let audio = self.audio.checkpoint();
        let microphone = self.microphone.checkpoint();
        let system = self.system.checkpoint();
        audio.and(microphone).and(system)
    }
}

const WAV_HEADER_BYTES: u64 = 80;

/// Reserve ds64 space as JUNK in ordinary RIFF files. A checkpoint promotes the
/// header to RF64 in place when 32-bit sizes no longer fit (EBU Tech 3306).
/// Neither checkpoints nor Stop scan, buffer or rewrite the captured audio.
pub(crate) struct PcmWriter {
    writer: File,
    frames: u64,
    path: PathBuf,
    sample_rate: u32,
    #[cfg(test)]
    fail_writes: usize,
}
impl PcmWriter {
    fn create(path: &Path) -> Result<Self, String> {
        Self::create_at_rate(path, RATE)
    }
    pub(crate) fn create_at_rate(path: &Path, sample_rate: u32) -> Result<Self, String> {
        if !(1..=384_000).contains(&sample_rate) {
            return Err("Invalid recording sample rate.".into());
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("Cannot create meeting folder: {e}"))?;
        }
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(path)
            .map_err(|e| format!("Cannot create meeting audio {}: {e}", path.display()))?;
        let mut result = Self {
            writer: file,
            frames: 0,
            path: path.to_owned(),
            sample_rate,
            #[cfg(test)]
            fail_writes: 0,
        };
        result.checkpoint()?;
        Ok(result)
    }
    pub(crate) fn write_samples(&mut self, samples: &[f32]) -> Result<(), String> {
        self.write_timeline_samples(self.frames, samples)
    }

    fn write_timeline_samples(&mut self, start: u64, samples: &[f32]) -> Result<(), String> {
        let next_frames = start
            .checked_add(samples.len() as u64)
            .filter(|frames| {
                frames
                    .checked_mul(2)
                    .and_then(|bytes| bytes.checked_add(WAV_HEADER_BYTES))
                    .is_some()
            })
            .ok_or("Recording size exceeds the supported 64-bit file range.")?;
        self.write_samples_at(start, samples)?;
        self.frames = self.frames.max(next_frames);
        Ok(())
    }
    fn write_samples_at(&mut self, start: u64, samples: &[f32]) -> Result<(), String> {
        let mut bytes = Vec::with_capacity(samples.len() * 2);
        for &sample in samples {
            bytes.extend_from_slice(&pcm16(sample).to_le_bytes());
        }
        retry_io(|| {
            self.writer.seek(SeekFrom::Start(WAV_HEADER_BYTES + start * 2))?;
            #[cfg(test)]
            if self.fail_writes > 0 {
                self.fail_writes -= 1;
                self.writer.write_all(&bytes[..bytes.len() / 2])?;
                return Err(std::io::Error::other("injected persistent writer failure"));
            }
            // Retry from the block's absolute offset, even after a partial write.
            // File is deliberately unbuffered: BufWriter can retain a partially
            // flushed block and duplicate it when a failed write_all is retried.
            self.writer.write_all(&bytes)
        }).map_err(|e| format!("Cannot write meeting audio {}: {e}", self.path.display()))
    }
    fn read_samples_at(&mut self, start: u64, count: usize) -> Result<Vec<f32>, String> {
        let mut bytes = vec![0; count * 2];
        retry_io(|| {
            self.writer.seek(SeekFrom::Start(WAV_HEADER_BYTES + start * 2))?;
            self.writer.read_exact(&mut bytes)
        }).map_err(|e| format!("Cannot read saved meeting audio: {e}"))?;
        Ok(bytes.chunks_exact(2).map(|pair| i16::from_le_bytes([pair[0], pair[1]]) as f32 / i16::MAX as f32).collect())
    }
    pub(crate) fn checkpoint(&mut self) -> Result<(), String> {
        retry_io(|| self.checkpoint_io())
            .map_err(|e| format!("Cannot finalize meeting audio {}: {e}", self.path.display()))
    }
    fn checkpoint_io(&mut self) -> std::io::Result<()> {
        self.writer.flush()?;
        let header = pcm_header(self.frames, self.sample_rate)?;
        self.writer.seek(SeekFrom::Start(0))?;
        self.writer.write_all(&header)?;
        self.writer.flush()?;
        self.writer
            .seek(SeekFrom::Start(WAV_HEADER_BYTES + self.frames * 2))?;
        Ok(())
    }
}

fn retry_io<T>(mut operation: impl FnMut() -> std::io::Result<T>) -> std::io::Result<T> {
    operation().or_else(|_| operation())
}

fn pcm_header(frames: u64, rate: u32) -> std::io::Result<[u8; WAV_HEADER_BYTES as usize]> {
    let length = frames
        .checked_mul(2)
        .ok_or_else(|| std::io::Error::other("PCM length overflow"))?;
    let file_size = length
        .checked_add(WAV_HEADER_BYTES)
        .ok_or_else(|| std::io::Error::other("WAV length overflow"))?;
    let riff_size = file_size - 8;
    let rf64 = riff_size >= u32::MAX as u64;
    let mut header = [0u8; WAV_HEADER_BYTES as usize];
    header[0..4].copy_from_slice(if rf64 { b"RF64" } else { b"RIFF" });
    header[4..8].copy_from_slice(&(if rf64 { u32::MAX } else { riff_size as u32 }).to_le_bytes());
    header[8..12].copy_from_slice(b"WAVE");
    header[12..16].copy_from_slice(if rf64 { b"ds64" } else { b"JUNK" });
    header[16..20].copy_from_slice(&28u32.to_le_bytes());
    if rf64 {
        header[20..28].copy_from_slice(&riff_size.to_le_bytes());
        header[28..36].copy_from_slice(&length.to_le_bytes());
        header[36..44].copy_from_slice(&frames.to_le_bytes());
    }
    header[48..52].copy_from_slice(b"fmt ");
    header[52..56].copy_from_slice(&16u32.to_le_bytes());
    header[56..58].copy_from_slice(&1u16.to_le_bytes());
    header[58..60].copy_from_slice(&1u16.to_le_bytes());
    header[60..64].copy_from_slice(&rate.to_le_bytes());
    header[64..68].copy_from_slice(&(rate * 2).to_le_bytes());
    header[68..70].copy_from_slice(&2u16.to_le_bytes());
    header[70..72].copy_from_slice(&16u16.to_le_bytes());
    header[72..76].copy_from_slice(b"data");
    header[76..80].copy_from_slice(&(if rf64 { u32::MAX } else { length as u32 }).to_le_bytes());
    Ok(header)
}

#[derive(Debug)]
pub(crate) struct WavInfo {
    pub rf64: bool,
    pub format: u16,
    pub channels: u16,
    pub sample_rate: u32,
    pub byte_rate: u32,
    pub block_align: u16,
    pub bits_per_sample: u16,
    pub data_offset: u64,
    pub data_bytes: u64,
}

pub(crate) fn read_wav_info(path: &Path) -> Result<WavInfo, String> {
    let mut file = File::open(path).map_err(|e| format!("Cannot open saved meeting audio: {e}"))?;
    let length = file.metadata().map_err(|e| e.to_string())?.len();
    inspect_wav(&mut file, length)
}

/// Inspect chunk headers only, including RF64's 64-bit sizes. Keeping this
/// shared with ASR prevents a long recording from becoming unreadable at Stop.
pub fn wav_duration(path: &Path) -> Result<f64, String> {
    let info = read_wav_info(path)?;
    Ok(info.data_bytes as f64 / f64::from(info.byte_rate))
}

fn inspect_wav(reader: &mut (impl Read + Seek), file_len: u64) -> Result<WavInfo, String> {
    let mut header = [0u8; 12];
    reader
        .read_exact(&mut header)
        .map_err(|_| "Meeting audio has an incomplete WAV header.")?;
    let rf64 = &header[..4] == b"RF64";
    if (!rf64 && &header[..4] != b"RIFF") || &header[8..] != b"WAVE" {
        return Err("Meeting audio must be a valid RIFF or RF64 WAV file.".into());
    }
    let size32 = u32::from_le_bytes(header[4..8].try_into().unwrap());
    if rf64 && size32 != u32::MAX {
        return Err("RF64 audio has an invalid size marker.".into());
    }
    let mut container_end = if rf64 {
        file_len
    } else {
        u64::from(size32) + 8
    };
    if container_end > file_len || container_end < 12 {
        return Err("Meeting audio is incomplete. Its WAV length exceeds the saved file.".into());
    }
    let mut extended_data = None;
    let mut extended_frames = None;
    let mut format = None;
    let mut data = None;
    loop {
        let position = reader.stream_position().map_err(|e| e.to_string())?;
        if position == container_end {
            break;
        }
        if position > container_end || container_end - position < 8 {
            return Err("Meeting audio has an incomplete chunk header.".into());
        }
        let mut chunk = [0u8; 8];
        reader
            .read_exact(&mut chunk)
            .map_err(|_| "Cannot read WAV chunk header.")?;
        let size32 = u32::from_le_bytes(chunk[4..8].try_into().unwrap());
        let start = position + 8;
        if rf64 && position == 12 && &chunk[..4] != b"ds64" {
            return Err("RF64 audio is missing its ds64 size chunk.".into());
        }
        let size = if rf64 && size32 == u32::MAX {
            if &chunk[..4] != b"data" {
                return Err(
                    "Unsupported RF64 extended chunk; convert this media before import.".into(),
                );
            }
            extended_data.ok_or("RF64 audio is missing its data size.")?
        } else {
            u64::from(size32)
        };
        let end = start
            .checked_add(size)
            .filter(|end| *end <= container_end)
            .ok_or("Meeting audio is incomplete. Its WAV data length exceeds the saved file.")?;
        if &chunk[..4] == b"ds64" && rf64 {
            if position != 12 || size < 28 {
                return Err("RF64 audio has an invalid ds64 chunk.".into());
            }
            let mut sizes = [0u8; 28];
            reader
                .read_exact(&mut sizes)
                .map_err(|_| "Cannot read RF64 sizes.")?;
            let riff_size = u64::from_le_bytes(sizes[..8].try_into().unwrap());
            container_end = riff_size
                .checked_add(8)
                .filter(|limit| *limit <= file_len && *limit >= end)
                .ok_or("RF64 audio is incomplete. Its length exceeds the saved file.")?;
            extended_data = Some(u64::from_le_bytes(sizes[8..16].try_into().unwrap()));
            extended_frames = Some(u64::from_le_bytes(sizes[16..24].try_into().unwrap()));
            let entries = u32::from_le_bytes(sizes[24..28].try_into().unwrap());
            if 28 + u64::from(entries) * 12 > size {
                return Err("RF64 audio has an incomplete size table.".into());
            }
        } else if &chunk[..4] == b"fmt " {
            if size < 16 || format.is_some() {
                return Err("Meeting audio has an invalid format chunk.".into());
            }
            let mut bytes = [0u8; 16];
            reader
                .read_exact(&mut bytes)
                .map_err(|_| "Cannot read WAV format.")?;
            format = Some(bytes);
        } else if &chunk[..4] == b"data" {
            if data.is_some() {
                return Err("Meeting audio has multiple data chunks.".into());
            }
            if rf64 && size32 != u32::MAX {
                return Err("RF64 audio has an invalid data size marker.".into());
            }
            data = Some((start, size));
        }
        let next = end
            .checked_add(size % 2)
            .filter(|end| *end <= container_end)
            .ok_or("Meeting audio is missing WAV chunk padding.")?;
        reader
            .seek(SeekFrom::Start(next))
            .map_err(|e| format!("Cannot read WAV chunks: {e}"))?;
    }
    let bytes = format.ok_or("Meeting audio is missing its format chunk.")?;
    let (data_offset, data_bytes) = data.ok_or("Meeting audio is missing its data chunk.")?;
    let info = WavInfo {
        rf64,
        format: u16::from_le_bytes(bytes[..2].try_into().unwrap()),
        channels: u16::from_le_bytes(bytes[2..4].try_into().unwrap()),
        sample_rate: u32::from_le_bytes(bytes[4..8].try_into().unwrap()),
        byte_rate: u32::from_le_bytes(bytes[8..12].try_into().unwrap()),
        block_align: u16::from_le_bytes(bytes[12..14].try_into().unwrap()),
        bits_per_sample: u16::from_le_bytes(bytes[14..16].try_into().unwrap()),
        data_offset,
        data_bytes,
    };
    if info.channels == 0 || info.sample_rate == 0 || info.byte_rate == 0 || info.block_align == 0 {
        return Err("Meeting audio has an invalid WAV format.".into());
    }
    if data_bytes % u64::from(info.block_align) != 0 {
        return Err("WAV ends with an incomplete interleaved audio frame.".into());
    }
    if rf64
        && extended_frames
            .is_some_and(|frames| frames != 0 && frames != data_bytes / u64::from(info.block_align))
    {
        return Err("RF64 audio has an inconsistent sample count.".into());
    }
    Ok(info)
}

impl Drop for PcmWriter {
    fn drop(&mut self) {
        let _ = self.checkpoint_io();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    pub(super) struct TestFiles {
        directory: PathBuf,
        pub(super) paths: RecordingPaths,
    }
    impl TestFiles {
        pub(super) fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let directory = std::env::temp_dir().join(format!(
                "sonojot-recorder-test-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir(&directory).unwrap();
            let paths = RecordingPaths {
                audio: directory.join("mixed.wav"),
                microphone: directory.join("microphone.wav"),
                system: directory.join("system.wav"),
            };
            Self { directory, paths }
        }
    }
    impl Drop for TestFiles {
        fn drop(&mut self) {
            for path in [
                &self.paths.audio,
                &self.paths.microphone,
                &self.paths.system,
            ] {
                let _ = std::fs::remove_file(path);
            }
            let _ = std::fs::remove_dir(&self.directory);
        }
    }
    pub(super) fn read_wav(path: &Path) -> Vec<i16> {
        let info = read_wav_info(path).unwrap();
        assert!(!info.rf64);
        assert_eq!(info.sample_rate, RATE);
        assert_eq!(info.channels, 1);
        assert_eq!(info.bits_per_sample, 16);
        assert_eq!(
            info.data_offset + info.data_bytes,
            std::fs::metadata(path).unwrap().len()
        );
        // An independent ordinary-WAV reader must accept the reserved JUNK.
        hound::WavReader::open(path)
            .unwrap()
            .into_samples::<i16>()
            .map(Result::unwrap)
            .collect()
    }

    #[test]
    fn wav_header_promotes_to_rf64_without_moving_audio() {
        let last_riff_frames = (u32::MAX as u64 - (WAV_HEADER_BYTES - 8) - 1) / 2;
        for count in [
            0,
            16_000,
            last_riff_frames,
            last_riff_frames + 1,
            u32::MAX as u64 * 3,
        ] {
            let header = pcm_header(count, RATE).unwrap();
            let length = WAV_HEADER_BYTES + count * 2;
            let info = inspect_wav(&mut std::io::Cursor::new(header), length).unwrap();
            assert_eq!(info.rf64, count > last_riff_frames);
            assert_eq!(info.data_offset, WAV_HEADER_BYTES);
            assert_eq!(info.data_bytes, count * 2);
            assert_eq!(info.byte_rate, RATE * 2);
            if info.rf64 {
                assert_eq!(&header[..4], b"RF64");
                assert_eq!(&header[12..16], b"ds64");
                assert_eq!(
                    u64::from_le_bytes(header[20..28].try_into().unwrap()),
                    length - 8
                );
                assert_eq!(
                    u64::from_le_bytes(header[36..44].try_into().unwrap()),
                    count
                );
                assert_eq!(&header[76..80], &u32::MAX.to_le_bytes());
            } else {
                assert_eq!(&header[12..16], b"JUNK");
            }
        }
        assert!(pcm_header(u64::MAX, RATE).is_err());
    }

    #[test]
    fn truncated_or_inconsistent_rf64_is_never_treated_as_complete() {
        let count = u32::MAX as u64;
        let header = pcm_header(count, RATE).unwrap();
        let length = WAV_HEADER_BYTES + count * 2;
        assert!(inspect_wav(&mut std::io::Cursor::new(header), length - 2).is_err());
        let mut invalid = header;
        invalid[36..44].copy_from_slice(&(count + 1).to_le_bytes());
        assert!(inspect_wav(&mut std::io::Cursor::new(invalid), length).is_err());
        invalid = header;
        invalid[12..16].copy_from_slice(b"JUNK");
        assert!(inspect_wav(&mut std::io::Cursor::new(invalid), length).is_err());
        invalid = header;
        invalid[28..36].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(inspect_wav(&mut std::io::Cursor::new(invalid), length).is_err());
    }

    #[test]
    fn saved_riff_duration_and_samples_remain_compatible() {
        let files = TestFiles::new();
        {
            let mut writer = PcmWriter::create(&files.paths.audio).unwrap();
            writer.write_samples(&[0.25; 32]).unwrap();
            writer.checkpoint().unwrap();
            assert_eq!(
                wav_duration(&files.paths.audio).unwrap(),
                32.0 / RATE as f64
            );
            assert_eq!(read_wav(&files.paths.audio), vec![pcm16(0.25); 32]);
        }
        // Existing 44-byte RIFF files keep working too.
        let mut writer = hound::WavWriter::create(
            &files.paths.microphone,
            hound::WavSpec {
                channels: 1,
                sample_rate: RATE,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        writer.write_sample(123i16).unwrap();
        writer.finalize().unwrap();
        assert_eq!(
            wav_duration(&files.paths.microphone).unwrap(),
            1.0 / RATE as f64
        );
    }

    #[test]
    fn recorder_handle_is_send() {
        fn is_send<T: Send>() {}
        is_send::<MeetingRecorder>();
    }

    #[test]
    fn stop_timeout_waits_for_late_wav_tail_and_warnings_before_publishing() {
        let _gate = crate::resource_manager::tests::test_gate();
        let files = TestFiles::new();
        let root = files.directory.join("library");
        let meetings = crate::meetings::Meetings::new(crate::store::Store::open(Some(root.clone())).unwrap());
        let detail = meetings.create_recording("Late writer", "microphone").unwrap();
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let mut writers = Writers::create(&RecordingPaths {
            audio: paths.mixed.clone(), microphone: paths.microphone, system: paths.system,
        }).unwrap();
        writers.audio.write_timeline_samples(0, &vec![0.25; RATE as usize]).unwrap();
        writers.checkpoint().unwrap();
        let notices = Arc::new(Mutex::new(SourceNotices::default()));
        let late_warnings = notices.lock().unwrap().warnings.clone();
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            writers.audio.write_timeline_samples(RATE as u64, &vec![0.75; RATE as usize]).unwrap();
            writers.checkpoint().unwrap();
            drop(writers);
            let issue = SourceIssue {
                source: IssueSource::Microphone,
                started_at_seconds: 0.1,
                ended_at_seconds: Some(0.2),
                kind: IssueKind::Disconnected,
                occurrences: 1,
                lost_duration_seconds: 0.1,
                message: "Late source warning".into(),
            };
            let outcome = RecordingOutcome { duration: 2.0, issues: vec![issue] };
            *late_warnings.lock().unwrap() = outcome.warnings();
            done_tx.send(Ok(outcome)).unwrap();
        });
        let stop = Arc::new(AtomicBool::new(false));
        let recorder = MeetingRecorder {
            stop: stop.clone(), error: Arc::new(Mutex::new(None)),
            level: Arc::new(AtomicU32::new(0)), stems: None, notices, done,
            worker: Some(worker),
        };
        let warnings = recorder.recording_warnings_handle();
        let mut active = Some((detail.id.clone(), recorder));
        let (_, recorder, finalizing) = crate::commands::take_finalizing_recording(&meetings, &mut active).unwrap();
        assert!(active.is_none());
        assert_eq!(finalizing.status, "finalizing");
        let pending = recorder.stop_timeout(Duration::ZERO);
        assert!(matches!(pending, RecordingStop::Finalizing(_)));
        let (saved_tx, saved_rx) = mpsc::channel();
        let transcriptions = Arc::new(AtomicU64::new(0));
        let starts = transcriptions.clone();
        let library = meetings.clone();
        let id = detail.id.clone();
        let late_warnings = warnings.clone();
        assert!(pending.finish_with(move |result| {
            let saved = library.complete_recording(&id, result, late_warnings.lock().unwrap().clone())?;
            let job = library.begin_transcription(&id, None)?;
            starts.fetch_add(1, Ordering::SeqCst);
            drop(job);
            saved_tx.send(saved).unwrap();
            Ok(())
        }, |error| panic!("Unexpected finalization error: {error}")).unwrap().is_none());
        assert!(stop.load(Ordering::Acquire));
        assert_eq!(meetings.get(&detail.id).unwrap().status, "finalizing");
        assert_eq!(wav_duration(&paths.mixed).unwrap(), 1.0);
        assert!(warnings.lock().unwrap().is_empty());
        assert_eq!(transcriptions.load(Ordering::SeqCst), 0);
        assert!(meetings.begin_transcription(&detail.id, None).is_err());
        assert!(meetings.cancel(&detail.id).is_err());
        assert!(meetings.delete(&detail.id).is_err());
        assert!(meetings.export_audio(&detail.id, &files.directory.join("export.wav")).is_err());
        release_tx.send(()).unwrap();
        let saved = saved_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(saved.status, "ready");
        assert_eq!(saved.duration_seconds, 2.0);
        assert_eq!(saved.recording_warnings, ["Late source warning"]);
        let samples = read_wav(&paths.mixed);
        assert_eq!(samples.len(), RATE as usize * 2);
        assert_eq!(samples.last(), Some(&pcm16(0.75)));
        assert_eq!(transcriptions.load(Ordering::SeqCst), 1);
        assert!(matches!(saved_rx.recv_timeout(Duration::from_secs(5)),
            Err(mpsc::RecvTimeoutError::Disconnected)));
        assert_eq!(meetings.get(&detail.id).unwrap().status, "transcribing");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    #[cfg(windows)]
    fn stop_signals_capture_even_when_finalizing_metadata_cannot_be_written() {
        let _gate = crate::resource_manager::tests::test_gate();
        let files = TestFiles::new();
        let root = files.directory.join("library");
        let meetings = crate::meetings::Meetings::new(crate::store::Store::open(Some(root.clone())).unwrap());
        let detail = meetings.create_recording("Metadata failure", "microphone").unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, done) = mpsc::sync_channel(1);
        let recorder = MeetingRecorder {
            stop: stop.clone(), error: Arc::new(Mutex::new(None)),
            level: Arc::new(AtomicU32::new(0)), stems: None,
            notices: Arc::new(Mutex::new(SourceNotices::default())), done, worker: None,
        };
        let mut active = Some((detail.id.clone(), recorder));
        let path = meetings.store().meetings_dir().join(format!("{}.json", detail.id));
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        let original = permissions.clone();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();
        let failed = crate::commands::take_finalizing_recording(&meetings, &mut active);
        std::fs::set_permissions(&path, original).unwrap();
        assert!(failed.is_err());
        assert!(stop.load(Ordering::Acquire));
        assert!(active.is_some(), "Keep the receiver and recording guards on metadata failure");
        assert_eq!(meetings.get(&detail.id).unwrap().status, "recording");
        assert!(meetings.begin_transcription(&detail.id, None).is_err());
        done_tx.send(Err("Stopped test worker".into())).unwrap();
        let (_, recorder, _) = crate::commands::take_finalizing_recording(&meetings, &mut active).unwrap();
        let RecordingStop::Complete(result) = recorder.stop_timeout(Duration::ZERO) else {
            panic!("The completed result must survive the metadata retry");
        };
        assert_eq!(result.unwrap_err(), "Stopped test worker");
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn stop_timeout_detects_worker_exit_without_a_result() {
        let (release_tx, release_rx) = mpsc::channel();
        let (done_tx, done) = mpsc::sync_channel(1);
        let worker = thread::spawn(move || {
            release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
            drop(done_tx);
        });
        let recorder = MeetingRecorder {
            stop: Arc::new(AtomicBool::new(false)), error: Arc::new(Mutex::new(None)),
            level: Arc::new(AtomicU32::new(0)), stems: None,
            notices: Arc::new(Mutex::new(SourceNotices::default())), done,
            worker: Some(worker),
        };
        let RecordingStop::Finalizing(recorder) = recorder.stop_timeout(Duration::ZERO) else {
            panic!("The test worker has not exited yet");
        };
        release_tx.send(()).unwrap();
        assert!(recorder.wait().unwrap_err().contains("stopped unexpectedly"));
    }

    #[test]
    fn bounded_source_warnings_remain_small_and_persist_after_finish_or_recovery() {
        let files = TestFiles::new();
        let root = files.directory.join("library");
        let meetings = crate::meetings::Meetings::new(crate::store::Store::open(Some(root.clone())).unwrap());
        let mut issues = issues::IssueCollector::default();
        for index in 0..80_000 {
            issues.push(SourceIssue {
                source: if index % 2 == 0 { IssueSource::Microphone } else { IssueSource::System },
                kind: IssueKind::Discontinuity,
                started_at_seconds: index as f64 * 3.0,
                ended_at_seconds: Some(index as f64 * 3.0 + 0.125),
                occurrences: 1,
                lost_duration_seconds: 0.125,
                message: "A short source gap.".into(),
            });
        }
        let outcome = RecordingOutcome { duration: 1.0, issues: issues.finish() };
        assert!(outcome.issues.len() <= 58);
        assert_eq!(outcome.issues.iter().map(|issue| issue.occurrences).sum::<u64>(), 80_000);
        assert!(serde_json::to_vec(&outcome.warnings()).unwrap().len() < 16_384);
        for failed in [false, true] {
            let detail = meetings.create_recording("Many gaps", "microphone").unwrap();
            let path = meetings.recording_paths(&detail.id).unwrap().mixed;
            let mut writer = PcmWriter::create(&path).unwrap();
            writer.write_timeline_samples(0, &vec![0.25; RATE as usize]).unwrap();
            drop(writer);
            meetings.begin_recording_finalization(&detail.id).unwrap();
            let result = if failed { Err("Writer exited".into()) } else { Ok(outcome.clone()) };
            let saved = meetings.complete_recording(&detail.id, result, outcome.warnings()).unwrap();
            assert_eq!(saved.status, "ready");
            assert_eq!(saved.duration_seconds, 1.0);
            assert!(saved.recording_warnings.iter().any(|warning| warning.starts_with("Earlier ")));
            assert_eq!(meetings.get(&detail.id).unwrap().recording_warnings, saved.recording_warnings);
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn mixed_meter_reports_bounded_rms_and_clears_on_silence_and_drop() {
        assert_eq!(rms_level(&[]), 0.0);
        assert_eq!(rms_level(&[f32::NAN, f32::INFINITY]), 0.0);
        assert_eq!(rms_level(&[-4.0, 4.0]), 1.0);
        assert_eq!(rms_level(&[-0.5, 0.5]), 0.5);

        let files = TestFiles::new();
        let level = Arc::new(AtomicU32::new(0));
        let (_done_tx, done) = mpsc::channel();
        let recorder = MeetingRecorder {
            stop: Arc::new(AtomicBool::new(false)),
            error: Arc::new(Mutex::new(None)),
            level: level.clone(),
            stems: Some(StemLevels::default()),
            notices: Arc::new(Mutex::new(SourceNotices::default())),
            done,
            worker: None,
        };
        let mut mixer = Mixer::new(Writers::create(&files.paths).unwrap(), Source::Both, level);
        mixer.stems = recorder.stems.clone().unwrap();
        assert_eq!(recorder.level(), 0.0);
        mixer
            .push(true, &[0.8; BLOCK], RATE, Duration::ZERO, false)
            .unwrap();
        mixer
            .push(false, &[0.2; BLOCK], RATE, Duration::ZERO, false)
            .unwrap();
        mixer.write_until(BLOCK as u64, true).unwrap();
        assert!((recorder.level() - mix_sample(0.8, 0.2, Source::Both)).abs() < 0.00001);
        let (microphone, system) = recorder.stem_levels().unwrap();
        assert!((microphone - 0.8).abs() < 0.00001 && (system - 0.2).abs() < 0.00001);
        mixer.write_until(BLOCK as u64 * 2, true).unwrap();
        assert_eq!(recorder.level(), 0.0);
        mixer
            .push(true, &[0.8; BLOCK], RATE, Duration::from_millis(100), false)
            .unwrap();
        mixer.write_until(BLOCK as u64 * 3, true).unwrap();
        assert!(recorder.level() > 0.0);
        drop(mixer);
        assert_eq!(recorder.level(), 0.0);
        assert_eq!(recorder.stem_levels(), Some((0.0, 0.0)));
    }

    #[test]
    fn source_validation_and_pcm_clipping_are_explicit() {
        assert!(Source::parse("desktop").is_err());
        assert_eq!(pcm16(2.0), i16::MAX);
        assert_eq!(pcm16(-2.0), -i16::MAX);
        assert_eq!(pcm16(f32::NAN), 0);
        assert_eq!(pcm16(f32::INFINITY), 0);
        assert_eq!(mix_sample(0.5, 1.0, Source::Microphone), 0.5);
        assert_eq!(mix_sample(1.0, 0.25, Source::System), 0.25);
        assert!((mix_sample(1.0, 1.0, Source::Both) - 0.6875).abs() < 1e-6);
    }

    #[test]
    fn distinct_start_times_write_aligned_source_and_mix_files() {
        let files = TestFiles::new();
        let mut mixer = Mixer::new(
            Writers::create(&files.paths).unwrap(),
            Source::Both,
            Arc::new(AtomicU32::new(0)),
        );
        mixer
            .push(
                true,
                &vec![0.8; 1_600],
                RATE,
                Duration::from_millis(100),
                false,
            )
            .unwrap();
        mixer
            .push(false, &vec![0.2; 3_200], RATE, Duration::ZERO, false)
            .unwrap();
        mixer.flush_resamplers().unwrap();
        mixer.write_until(3_200, true).unwrap();
        drop(mixer);
        let mic = read_wav(&files.paths.microphone);
        let system = read_wav(&files.paths.system);
        let mixed = read_wav(&files.paths.audio);
        assert_eq!(mic.len(), 3_200);
        assert_eq!(system.len(), mic.len());
        assert_eq!(mixed.len(), mic.len());
        assert!(mic[..1_600].iter().all(|&sample| sample == 0));
        assert!(mic[1_600..].iter().all(|&sample| sample == pcm16(0.8)));
        assert!(system.iter().all(|&sample| sample == pcm16(0.2)));
        assert!(mixed[..1_600]
            .iter()
            .all(|&sample| sample == pcm16(mix_sample(0.0, 0.2, Source::Both))));
        assert!(mixed[1_600..]
            .iter()
            .all(|&sample| sample == pcm16(mix_sample(0.8, 0.2, Source::Both))));
    }

    #[test]
    fn fractional_resampling_is_independent_of_callback_boundaries() {
        let input: Vec<f32> = (0..44_100 * 2)
            .map(|i| (i as f32 * 0.04).sin() * 0.7)
            .collect();
        let mut whole = Resampler::new(44_100, 0.0);
        let (_, reference) = whole.push(&input, true);
        let mut streaming = Resampler::new(44_100, 0.0);
        let mut output = Vec::new();
        for chunk in input.chunks(257) {
            let (start, values) = streaming.push(chunk, false);
            assert_eq!(start as usize, output.len());
            output.extend(values);
            assert!(streaming.data.len() <= 2 * FILTER_RADIUS as usize + 3);
        }
        let (start, tail) = streaming.push(&[], true);
        assert_eq!(start as usize, output.len());
        output.extend(tail);
        assert_eq!(output.len(), RATE as usize * 2);
        assert_eq!(output, reference);
    }

    #[test]
    fn independent_device_clock_drift_does_not_create_timeline_errors() {
        let mut track = Track::default();
        let mut cursor = 0;
        // A 0.1% slow device is deliberately larger than ordinary clock drift.
        // Its 30 ms accumulated difference must be followed, not rejected.
        for block in 0..3_000 {
            let at = Duration::from_secs_f64(block as f64 * 0.010_01);
            track.push(&[0.4; 160], RATE, at, false, cursor).unwrap();
            let safe = frames(at.saturating_sub(REORDER_DELAY));
            if safe > cursor {
                track.take((safe - cursor) as usize);
                cursor = safe;
            }
        }
        track.flush(cursor).unwrap();
        assert!(track.end > RATE as u64 * 30 + 400);
        assert!(track.pending.len() <= RATE as usize);
    }

    #[test]
    fn downsampling_filters_frequencies_above_the_output_nyquist_limit() {
        let tone = |frequency: f32| -> f32 {
            let samples: Vec<f32> = (0..48_000)
                .map(|i| (i as f32 * std::f32::consts::TAU * frequency / 48_000.0).sin())
                .collect();
            let (_, samples) = Resampler::new(48_000, 0.0).push(&samples, true);
            let interior = &samples[100..samples.len() - 100];
            (interior.iter().map(|s| s * s).sum::<f32>() / interior.len() as f32).sqrt()
        };
        assert!(tone(1_000.0) > 0.65);
        assert!(tone(12_000.0) < 0.025);
    }

    #[test]
    fn timestamp_gaps_and_discontinuities_preserve_samples() {
        let mut track = Track::default();
        track
            .push(&vec![0.5; 1_600], RATE, Duration::ZERO, false, 0)
            .unwrap();
        track
            .push(
                &vec![0.7; 1_600],
                RATE,
                Duration::from_millis(200),
                false,
                0,
            )
            .unwrap();
        track.flush(0).unwrap();
        let samples = track.take(4_800);
        assert!(samples[1_600..3_200].iter().all(|&s| s == 0.0));
        track.push(&[0.1; 160], RATE, Duration::from_millis(300), true, 0).unwrap();
    }

    #[test]
    fn silent_desktop_can_resume_after_the_old_filter_tail_was_written() {
        let files = TestFiles::new();
        let mut mixer = Mixer::new(
            Writers::create(&files.paths).unwrap(),
            Source::System,
            Arc::new(AtomicU32::new(0)),
        );
        mixer
            .push(false, &[0.4; 1_600], RATE, Duration::ZERO, false)
            .unwrap();
        mixer.write_until(RATE as u64 * 2, false).unwrap();
        assert!(mixer.system.resampler.is_none());
        mixer
            .push(false, &[0.6; 1_600], RATE, Duration::from_secs(2), false)
            .unwrap();
        mixer.flush_resamplers().unwrap();
        mixer.write_until(RATE as u64 * 2 + 1_600, true).unwrap();
        drop(mixer);
        let samples = read_wav(&files.paths.audio);
        assert_eq!(samples.len(), 33_600);
        assert!(samples[..1_600].iter().all(|&s| s == pcm16(0.4)));
        assert!(samples[1_600..32_000].iter().all(|&s| s == 0));
        assert!(samples[32_000..].iter().all(|&s| s == pcm16(0.6)));
    }

    #[test]
    fn invalid_late_overlapping_and_excessive_buffers_fail_without_unbounded_growth() {
        let mut track = Track::default();
        assert!(track
            .push(&[0.1; 160], 0, Duration::ZERO, false, 0)
            .is_err());
        assert!(track
            .push(&vec![0.1; 32_001], RATE, Duration::ZERO, false, 0)
            .is_err());
        assert!(track
            .push(&[0.1; 160], RATE, Duration::from_secs(7), false, 0)
            .unwrap_err()
            .contains("six-second"));
        assert!(track.pending.is_empty());
        let mut late = Track::default();
        late.push(&[0.1; 160], RATE, Duration::ZERO, false, 800).unwrap();
        assert_eq!(late.repairs[0].0, 0);
        assert!(!late.repairs[0].1.is_empty());
        let mut overlap = Track::default();
        overlap
            .push(&[0.1; 1_600], RATE, Duration::from_millis(100), false, 0)
            .unwrap();
        assert!(overlap
            .push(&[0.1; 1_600], RATE, Duration::from_millis(50), false, 0)
            .unwrap_err()
            .contains("backwards"));
    }

    #[test]
    fn streaming_meeting_keeps_a_fixed_memory_window_and_flushes_exact_tail() {
        let files = TestFiles::new();
        let mut mixer = Mixer::new(
            Writers::create(&files.paths).unwrap(),
            Source::Microphone,
            Arc::new(AtomicU32::new(0)),
        );
        for block in 0..400 {
            let at = Duration::from_millis(block * 50);
            mixer.push(true, &[0.3; BLOCK], RATE, at, false).unwrap();
            mixer
                .write_until(frames(at.saturating_sub(REORDER_DELAY)), false)
                .unwrap();
            assert!(mixer.microphone.pending.len() < RATE as usize);
        }
        mixer.flush_resamplers().unwrap();
        mixer.write_until(RATE as u64 * 20, true).unwrap();
        mixer.writers.checkpoint().unwrap();
        assert_eq!(read_wav(&files.paths.audio).len(), RATE as usize * 20);
        assert!(read_wav(&files.paths.system)
            .iter()
            .all(|&sample| sample == 0));
        assert_eq!(
            read_wav(&files.paths.audio),
            read_wav(&files.paths.microphone)
        );
    }

    #[test]
    fn headers_survive_checkpoints_drop_and_capture_failure() {
        let files = TestFiles::new();
        {
            let mut mixer = Mixer::new(
                Writers::create(&files.paths).unwrap(),
                Source::Both,
                Arc::new(AtomicU32::new(0)),
            );
            mixer
                .push(true, &[0.4; 1_603], RATE, Duration::ZERO, false)
                .unwrap();
            mixer.flush_resamplers().unwrap();
            mixer.write_until(1_603, true).unwrap();
            mixer.writers.checkpoint().unwrap();
            assert_eq!(read_wav(&files.paths.audio).len(), 1_603);
            mixer.push(false, &[0.4; 160], RATE, Duration::ZERO, false).unwrap();
            mixer.write_until(1_603, true).unwrap();
        }
        assert_eq!(read_wav(&files.paths.microphone).len(), 1_603);
        assert_eq!(read_wav(&files.paths.system).len(), 1_603);
    }

    #[test]
    fn existing_recordings_are_never_overwritten() {
        let files = TestFiles::new();
        std::fs::write(&files.paths.system, b"existing private audio").unwrap();
        assert!(Writers::create(&files.paths).is_err());
        assert_eq!(
            std::fs::read(&files.paths.system).unwrap(),
            b"existing private audio"
        );
        assert!(!files.paths.audio.exists());
        let duplicated = RecordingPaths {
            system: files.paths.audio.clone(),
            ..files.paths.clone()
        };
        assert!(Writers::create(&duplicated).is_err());
    }
}
