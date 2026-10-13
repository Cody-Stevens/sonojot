#[path = "meeting_export.rs"]
mod export;
pub use export::{render_export, render_pdf};
mod outages {
    //! Meeting-only interpretation of capture and recognition diagnostics.
    use crate::{
        asr::RecognitionResult,
        domain::{AudioOutage, RecognitionCompleteness},
        recording::{IssueKind, IssueSource, SourceIssue},
    };

    pub(super) fn capture(issues: &[SourceIssue], duration: f64) -> (Vec<AudioOutage>, f64) {
        let mut missing = 0.0;
        let mut outages = Vec::new();
        for issue in issues {
            if !issue.started_at_seconds.is_finite() || !issue.lost_duration_seconds.is_finite() {
                continue;
            }
            let start = issue.started_at_seconds.clamp(0.0, duration);
            let end = issue.ended_at_seconds.filter(|end| end.is_finite())
                .unwrap_or(duration).clamp(start, duration);
            // Coalesced diagnostics retain exact lost duration separately from
            // their bounding range; never count healthy audio inside that range.
            missing += if issue.ended_at_seconds.is_none() { duration - start }
                else { issue.lost_duration_seconds.max(0.0).min(end - start) };
            if !issue.is_outage() { continue; }
            let source = match issue.source {
                IssueSource::Microphone => "microphone",
                IssueSource::System => "system",
            };
            let label = match issue.source {
                IssueSource::Microphone => "Microphone",
                IssueSource::System => "System audio",
            };
            let cause = match issue.kind {
                IssueKind::Disconnected => "disconnected",
                IssueKind::Overflow => "capture queue overflowed",
                IssueKind::InvalidBuffer => "returned invalid audio",
                IssueKind::Discontinuity => "had an audio gap",
            };
            outages.push(AudioOutage {
                source: source.into(), start_seconds: start, end_seconds: end,
                reason: format!("{label} {cause}{}", if issue.ended_at_seconds.is_none() {
                    " and did not recover before recording ended"
                } else { "" }),
            });
        }
        outages.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
        (outages, missing.min(duration))
    }

    pub(super) fn clock(seconds: f64) -> String {
        let seconds = seconds.max(0.0).floor() as u64;
        if seconds >= 3600 {
            format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
        } else {
            format!("{}:{:02}", seconds / 60, seconds % 60)
        }
    }

    pub(super) fn seconds(clock: &str) -> Option<f64> {
        let parts: Vec<_> = clock.split(':').collect();
        if !(2..=3).contains(&parts.len()) { return None; }
        let mut total = 0u64;
        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) { return None; }
            let value = part.parse::<u64>().ok()?;
            if index > 0 && (part.len() != 2 || value >= 60) { return None; }
            total = total.checked_mul(60)?.checked_add(value)?;
        }
        Some(total as f64)
    }

    /// Old strings cannot prove short-gap loss or its duration. Surface only an
    /// explicit stop/failure, or a disconnect with no recovery clause.
    pub(super) fn legacy(warnings: &[String], duration: f64) -> Vec<AudioOutage> {
        warnings.iter().filter_map(|warning| {
            let lower = warning.to_ascii_lowercase();
            let recovered = lower.contains("reconnected") || lower.contains("resumed");
            let unrecovered = lower.contains("never recovered") || lower.contains("did not recover")
                || lower.contains("never reconnected") || lower.contains("did not reconnect")
                // Older coalesced open issues use Stop as their missing end time.
                || lower.contains(" to stop;");
            let open_disconnect = lower.contains("disconnected at ") && !recovered;
            if recovered && !unrecovered { return None; }
            if !unrecovered && !open_disconnect && !lower.contains("stopped") && !lower.contains("failed")
                && !lower.starts_with("recording was interrupted") {
                return None;
            }
            let labeled = lower.strip_prefix("earlier ").unwrap_or(&lower);
            let source = if labeled.starts_with("microphone") { "microphone" }
                else if labeled.starts_with("system") { "system" } else { "recording" };
            let time = lower.split_once(" at ").and_then(|(_, rest)| rest.split_whitespace().next())
                .and_then(|value| seconds(value.trim_end_matches(['.', ';', ','])));
            // A fatal stop with no saved time is anchored to the last saved frame;
            // the unknowable tail must not become invented lost duration.
            let start = time.unwrap_or(duration).clamp(0.0, duration);
            Some(AudioOutage {
                source: source.into(), start_seconds: start, end_seconds: duration,
                reason: if source == "recording" { "Recording ended early".into() }
                    else { format!("{} did not recover before recording ended",
                        if source == "microphone" { "Microphone" } else { "System audio" }) },
            })
        }).collect()
    }

    fn collapse(warning: &str) -> bool {
        warning.contains("Repeated text was reduced to its first occurrence")
    }

    fn omission(warning: &str) -> bool {
        warning.contains("This section was omitted") || warning.contains("output token limit")
            || warning.contains("output-token limit")
    }

    fn labeling_warning(warning: &str) -> bool {
        // Diarization runs after recognition and does not affect its completeness.
        warning.starts_with("Speaker labeling failed:")
    }

    pub(super) fn collapse_only(result: &RecognitionResult) -> bool {
        result.completeness == RecognitionCompleteness::Incomplete
            && collapse_warnings_only(&result.warnings)
    }

    pub(super) fn collapse_warnings_only(warnings: &[String]) -> bool {
        warnings.iter().any(|warning| collapse(warning))
            && warnings.iter().all(|warning| collapse(warning) || labeling_warning(warning))
    }

    pub(super) fn genuine_stop(completeness: RecognitionCompleteness, warnings: &[String]) -> bool {
        let mut warnings = warnings.iter().filter(|warning| !labeling_warning(warning)).peekable();
        completeness == RecognitionCompleteness::Incomplete
            && (warnings.peek().is_none() || warnings.any(|warning| {
                !collapse(warning) && !omission(warning)
                    && !warning.contains("Available text was retained for review")
            }))
    }

    pub(super) fn stopped_at(result: &RecognitionResult, duration: f64) -> f64 {
        result.segments.iter().map(|segment| segment.end_seconds.unwrap_or(segment.start_seconds) as f64)
            .filter(|seconds| seconds.is_finite()).fold(0.0, f64::max).clamp(0.0, duration)
    }

    pub(super) fn recognition(result: &RecognitionResult, duration: f64) -> Vec<AudioOutage> {
        if !result.warnings.iter().any(|warning| omission(warning)) { return Vec::new(); }
        let dropped = result.warnings.iter().any(|warning| warning.contains("This section was omitted"));
        let collapsed = result.warnings.iter().any(|warning| collapse(warning));
        let reason = if dropped { "Transcription omitted speech after repeated output" }
            else { "Transcription reached its output token limit; speech may be missing" };
        // Some recognizers omit empty dropped segments from the public result.
        // Do not label silence between retained segments as a proven omission.
        let mut gaps: Vec<_> = result.segments.iter().filter(|segment| {
            !segment.complete && segment.start_seconds.is_finite()
                && (!dropped || segment.text.trim().is_empty()) && !collapsed
        }).map(|segment| {
            let start = (segment.start_seconds as f64).clamp(0.0, duration);
            AudioOutage {
                source: "recognition".into(), start_seconds: start,
                end_seconds: segment.end_seconds.filter(|end| end.is_finite())
                    .map(f64::from).unwrap_or(start).clamp(start, duration),
                reason: reason.into(),
            }
        }).collect();
        if gaps.is_empty() {
            let approximate = result.segments.iter().find(|segment| !segment.complete)
                .map(|segment| f64::from(segment.start_seconds)).filter(|time| time.is_finite())
                .unwrap_or_else(|| stopped_at(result, duration)).clamp(0.0, duration);
            gaps.push(AudioOutage {
                source: "recognition".into(), start_seconds: approximate, end_seconds: approximate,
                reason: format!("{reason}; time is approximate"),
            });
        }
        gaps
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        fn issue(start: f64, lost: f64) -> SourceIssue {
            SourceIssue { source: IssueSource::Microphone, kind: IssueKind::Disconnected,
                started_at_seconds: start, ended_at_seconds: Some(start + lost),
                occurrences: 1, lost_duration_seconds: lost, message: String::new() }
        }
        #[test]
        fn strict_threshold_and_small_losses_keep_exact_totals() {
            let (gaps, missing) = capture(&[issue(0.0, 3.0), issue(10.0, 3.01)], 100.0);
            assert_eq!(gaps.len(), 1);
            assert_eq!(gaps[0].start_seconds, 10.0);
            assert!((missing - 6.01).abs() < 1e-9);
            let (_, exactly_one_percent) = capture(&[issue(0.0, 1.0)], 100.0);
            assert!(!(exactly_one_percent > 100.0 * 0.01));
            let (gaps, more_than_one_percent) = capture(&[issue(0.0, 1.01)], 100.0);
            assert!(gaps.is_empty());
            assert!(more_than_one_percent > 100.0 * 0.01);
        }
        #[test]
        fn aggregated_range_does_not_become_an_outage() {
            let mut summary = issue(10.0, 7.2);
            summary.ended_at_seconds = Some(7000.0);
            summary.occurrences = 191;
            let (gaps, missing) = capture(&[summary], 7680.0);
            assert!(gaps.is_empty());
            assert_eq!(missing, 7.2);
            assert!(missing < 7680.0 * 0.01);
        }
        #[test]
        fn unrecovered_source_runs_to_end_even_below_threshold() {
            let mut loss = issue(99.0, 0.0);
            loss.ended_at_seconds = None;
            let (gaps, missing) = capture(&[loss], 100.0);
            assert_eq!(missing, 1.0);
            assert_eq!(gaps[0].end_seconds, 100.0);
        }
        #[test]
        fn collapse_mixed_with_a_failure_is_a_stop_and_omissions_keep_known_bounds() {
            let mut result = RecognitionResult {
                original_text: None, text: "Retained words.".into(),
                segments: vec![crate::asr::Segment {
                    id: 1, start_seconds: 40.0, end_seconds: Some(50.0), speaker: None,
                    text: String::new(), complete: false,
                }],
                completeness: RecognitionCompleteness::Incomplete,
                warnings: vec!["Repeated text was reduced to its first occurrence".into(), "Worker stopped".into()],
            };
            assert!(!collapse_only(&result));
            assert!(genuine_stop(result.completeness, &result.warnings));
            result.warnings[1] = "Speaker labeling failed: worker unavailable".into();
            assert!(collapse_only(&result));
            assert!(!genuine_stop(result.completeness, &result.warnings));
            result.warnings = vec!["This section was omitted; review the recording for missing speech.".into()];
            result.warnings.push("Speaker labeling failed: worker unavailable".into());
            assert!(!genuine_stop(result.completeness, &result.warnings));
            let gaps = recognition(&result, 100.0);
            assert_eq!(gaps.len(), 1);
            assert_eq!(gaps[0].start_seconds, 40.0);
            assert_eq!(gaps[0].end_seconds, 50.0);
            assert!(!gaps[0].reason.contains("approximate"));
        }

        #[test]
        fn legacy_strings_only_surface_unrecovered_or_early_stops() {
            let warnings = [
                "System audio had an audio gap at 1:00; recording continued.",
                "Microphone disconnected at 1:00; recording continued. Reconnected to Microphone at 1:14.",
                "Earlier System audio gaps: 191 occurrences, 7.200 seconds of audio lost from 0:10 to 127:59; recording continued.",
                "Microphone disconnected at 1:00; recording continued.",
                "Recording stopped early: writer failed.",
                "Recording was interrupted; saved audio was recovered.",
                "Earlier System audio disconnects: 2 occurrences, 4.000 seconds of audio lost from 0:10 to Stop; recording continued.",
            ].map(str::to_owned);
            let gaps = legacy(&warnings, 90.0);
            assert_eq!(gaps.len(), 4);
            assert_eq!(gaps[0].start_seconds, 60.0);
            assert_eq!(gaps[0].end_seconds, 90.0);
            assert_eq!(gaps[1].start_seconds, 90.0);
            assert_eq!(gaps[3].source, "system");
            assert_eq!(gaps[3].start_seconds, 90.0);
        }
    }
}
mod clean {
    //! A separately saved local Clean copy. No clipboard or dictation ownership.
    use super::*;

    impl Meetings {
        pub fn clean_local(&self, id: &str, started: impl FnOnce()) -> Result<MeetingDetail, String> {
            let settings = self.store.load_settings()?;
            let mut vocabulary = Vec::new();
            let mut confirmed = Vec::new();
            for entry in self.store.get_vocabulary()?.into_iter().filter(|entry| entry.enabled) {
                if let Some(scope) = entry.scope.as_deref().map(str::trim).filter(|scope| !scope.is_empty()) {
                    confirmed.push(crate::structured_clean::VocabularyEntry {
                        id: Some(entry.id.clone()), canonical: entry.written.clone(),
                        aliases: vec![entry.heard.clone()], scope: scope.into(),
                        status: crate::structured_clean::VocabularyStatus::Confirmed,
                        provenance: crate::structured_clean::VocabularyProvenance::ExplicitUserConfirmation,
                    });
                }
                vocabulary.push(crate::cleanup::Vocabulary {
                    heard: entry.heard, written: entry.written, enabled: true, context: Vec::new(),
                });
            }
            self.clean_with(id, |transcript, cancel| {
                started();
                crate::resource_manager::configure(&settings);
                let stop = AtomicBool::new(false);
                export::map_passages(transcript, |passage| {
                    crate::ai::check_cancel(cancel)?;
                    // Public entry point owns punctuation's resource admission and
                    // worker. local-rules avoids acquiring an optional LLM lease.
                    let result = crate::cleanup::run_dictation_interruptible_with_options(
                        &self.store.models_dir(), passage, crate::cleanup::Mode::Clean,
                        &vocabulary, &confirmed, "local-rules", crate::cleanup::CleanupPolicy::conservative(),
                        cancel, &stop, settings.written_form,
                    );
                    crate::ai::check_cancel(cancel)?;
                    Ok(result.text)
                })
            })
        }

        fn clean_with(&self, id: &str, clean: impl FnOnce(&str, &AtomicBool) -> Result<String, String>)
            -> Result<MeetingDetail, String>
        {
            let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Ai)?;
            if self.get(id)?.transcript.trim().is_empty() {
                return Err("Transcribe this meeting before cleaning it.".into());
            }
            let (record, token) = self.begin(id, "processingNotes")?;
            let result = clean(&record.detail.transcript, &token).and_then(|text| {
                validate_text(&text, "Cleaned transcript")?;
                if text.trim().is_empty() { return Err("Cleanup returned no text. The original was preserved.".into()); }
                Ok(text)
            });
            let _guard = self.store.lock()?;
            let mut operations = self.operations.lock().map_err(|_| "Meeting operation lock failed.")?;
            if !operations.get(id).is_some_and(|current| Arc::ptr_eq(current, &token)) {
                return Err("The meeting operation is no longer current.".into());
            }
            operations.remove(id);
            let mut current = self.read(id)?;
            if token.load(Ordering::Acquire) || current.detail.status == "cancelled" {
                return Err("Meeting cleanup cancelled.".into());
            }
            current.detail.summary.status = completed_status(&current.detail).into();
            match result {
                Ok(text) => {
                    current.detail.cleaned_transcript = Some(text);
                    current.detail.error = None;
                    self.write(&current)?;
                    Ok(current.detail)
                }
                Err(error) => {
                    current.detail.error = Some(error.chars().take(2048).collect());
                    self.write(&current)?;
                    Err(error)
                }
            }
        }

        #[cfg(test)]
        pub(super) fn clean_for_test(&self, id: &str, clean: impl FnOnce(&str, &AtomicBool) -> Result<String, String>)
            -> Result<MeetingDetail, String>
        {
            self.clean_with(id, clean)
        }
    }
}

use crate::{
    domain::{MeetingDetail, MeetingSummary},
    meeting_audio_export,
    store::{
        atomic_write, field, new_id, now, read_json, stable_id, string_field, validate_id,
        validate_text, Store,
    },
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

/// Returned when switching labels would discard transcript edits; the UI
/// confirms and retries.
pub const SPEAKER_LABELS_REPLACE_EDITS: &str =
    "Changing speaker labels rebuilds the transcript and replaces your edits.";

#[derive(Clone)]
pub struct Meetings {
    store: Store,
    operations: Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>,
    revision: Arc<AtomicU64>,
    pub(crate) automatic_titles: Arc<crate::meeting_ai::AutomaticTitles>,
}
#[derive(Clone, Debug)]
pub struct RecordingPaths {
    pub mixed: PathBuf,
    pub microphone: PathBuf,
    pub system: PathBuf,
}

/// Owns transcription before a background worker starts, so Stop can return a
/// persisted busy status and a second request cannot decode the same meeting.
///
/// Single-owner rule (Round 6 final): this struct owns the ASR lease for the
/// whole transcription. The `recognize` closure receives the snapshotted model
/// ID and must NOT re-read settings or acquire another ASR lease. It may retain
/// one recognizer across stems; that recognizer must be owned by the closure
/// so it drops before the lease is released. Remote jobs bypass this lease.
/// Unknown IDs error via Recognizer::load; never fall back to another model.
/// The lease is atomic (`acquire_asr_with_admission`: fit + active + pending
/// in one critical section), and `run` requests + awaits the speech worker's
/// confirmed cached-recognizer drop before the closure loads, so no two
/// native Engines coexist in one process.
pub struct MeetingTranscription {
    meetings: Meetings,
    id: String,
    audio: PathBuf,
    stems: Option<(PathBuf, PathBuf)>,
    cancel: Arc<AtomicBool>,
    asr_model: String,
    remote: Option<crate::meeting_speech::Snapshot>,
    /// Setting, availability and models directory captured with the speech model.
    diarization_root: Option<PathBuf>,
    _update_activity: crate::resource_manager::UpdateActivityLease,
}

impl MeetingTranscription {
    /// Snapshotted model accessor for callers that need it before `run`.
    pub fn asr_model(&self) -> &str {
        &self.asr_model
    }

    pub fn run<F>(self, mut recognize: F) -> Result<MeetingDetail, String>
    where
        F: FnMut(&Path, &str, &AtomicBool) -> Result<String, String>,
    {
        self.run_recognition(move |path, model, cancel| {
            recognize(path, model, cancel).map(|text| crate::asr::RecognitionResult {
                original_text: None,
                segments: Vec::new(),
                text,
                completeness: crate::domain::RecognitionCompleteness::Complete,
                warnings: Vec::new(),
            })
        })
    }

    pub fn run_recognition<F>(mut self, recognize: F) -> Result<MeetingDetail, String>
    where
        F: FnMut(&Path, &str, &AtomicBool) -> Result<crate::asr::RecognitionResult, String>,
    {
        if let Some(snapshot) = self.remote.take() {
            return self.run_remote(|path, _, cancel| snapshot.transcribe(path, cancel));
        }
        // Single snapshot from begin_transcription; a Settings edit mid-job
        // cannot switch the model. Lease the shared ASR stage once; the
        // closure uses the held model without re-acquiring.
        // Cancellation preserves audio; errors become failed status, never a
        // silent loss. Raw transcript guards stay in finish_operation.
        if self.cancel.load(Ordering::SeqCst) {
            return self.meetings.finish_operation(
                &self.id,
                &self.cancel,
                Err("Meeting operation cancelled.".into()),
                false,
                None,
            );
        }
        let asr_model = self.asr_model.clone();
        // Keep the coordinator on current lifecycle choices (best effort).
        if let Ok(settings) = self.meetings.store.load_settings() {
            crate::resource_manager::configure(&settings);
        }
        crate::resource_manager::touch_activity(crate::resource_manager::Stage::Asr);
        // Atomic reservation + fit tied to lease ownership (same contract as
        // dictation): fit + active + pending reservation in ONE coordinator
        // critical section, so a file job racing dictation (or another file
        // job) cannot slip a joint OOM past a pre-acquire check. Proven OOM
        // fails fast here with a failed status, never retried as busy.
        let lease = match crate::resource_manager::acquire_asr_with_admission(
            &asr_model,
            "meeting transcription",
        ) {
            Ok(()) => Some(MeetingAsrLease { finished: false }),
            Err(error) => {
                return self
                    .meetings
                    .finish_operation(&self.id, &self.cancel, Err(error), false, None);
            }
        };
        let mut lease = lease;
        // Cached-recognizer discipline: the speech worker may retain an idle
        // native Recognizer in this process while the file path constructs
        // another Engine via `asr::transcribe_file`. Same-GPU selections would
        // duplicate weights; CPU/GPU variants collide by DLL basename. Under
        // this reserved ownership, request the cached drop and await the
        // worker's CONFIRMED release before the closure loads: the pending
        // flag is never counted as freed, `loaded` is cleared only by the
        // owning worker after the physical drop (never faked here), and
        // cancellation or timeout releases this lease instead of loading
        // concurrently.
        if self.cancel.load(Ordering::SeqCst) {
            if let Some(ref mut guard) = lease {
                guard.finish();
            }
            return self.meetings.finish_operation(
                &self.id,
                &self.cancel,
                Err("Meeting operation cancelled.".into()),
                false,
                None,
            );
        }
        crate::resource_manager::request_asr_release();
        if let Err(error) =
            crate::resource_manager::await_asr_release(&self.cancel, Duration::from_secs(10))
        {
            if let Some(ref mut guard) = lease {
                guard.finish();
            }
            return self
                .meetings
                .finish_operation(&self.id, &self.cancel, Err(error), false, None);
        }
        let result = if self.cancel.load(Ordering::SeqCst) {
            Err("Meeting operation cancelled.".into())
        } else {
            self.recognize_audio(recognize)
        };
        // Residency truthfulness: the file Engine is ephemeral (owned by the
        // closure, dropped by recognize_audio), so this job deliberately never
        // marks `note_asr_loaded`: `finish` clears active + pending while
        // `loaded` stays None, which is the truth after the drop. Dictation
        // marks readiness on its cached worker instead; both paths agree that
        // only a live native handle counts as resident.
        if let Some(ref mut guard) = lease {
            guard.finish();
        }
        self.meetings
            .finish_recognition_operation(&self.id, &self.cancel, result, &asr_model)
    }

    fn run_remote<F>(mut self, recognize: F) -> Result<MeetingDetail, String>
    where F: FnMut(&Path, &str, &AtomicBool) -> Result<crate::asr::RecognitionResult, String> {
        // Remote jobs own only their meeting token, never the local ASR stage.
        self.diarization_root = None;
        let result = self.recognize_audio(recognize);
        self.meetings.finish_recognition_operation(&self.id, &self.cancel, result, &self.asr_model)
    }

    fn recognize_audio<F>(&self, recognize: F) -> Result<MeetingRecognition, String>
    where F: FnMut(&Path, &str, &AtomicBool) -> Result<crate::asr::RecognitionResult, String> {
        self.recognize_audio_with(recognize, |root, audio, cancel| {
            crate::diarization_process::run(root, audio, &self.meetings.store, cancel)
        })
    }

    fn recognize_audio_with<F, D>(&self, mut recognize: F, mut diarize: D) -> Result<MeetingRecognition, String>
    where
        F: FnMut(&Path, &str, &AtomicBool) -> Result<crate::asr::RecognitionResult, String>,
        D: FnMut(&Path, &Path, &AtomicBool) -> Result<Vec<crate::diarization::Turn>, String>,
    {
        crate::ai::check_cancel(&self.cancel)?;
        if let Some((microphone, system)) = &self.stems {
            // Missing or silent stems retain the mixed-file behavior. Recognition
            // is still checked for empty speech because low room noise is not speech.
            let usable = microphone.is_file() && system.is_file()
                && crate::meeting_speech::wav_has_signal(microphone, &self.cancel)?
                && crate::meeting_speech::wav_has_signal(system, &self.cancel)?;
            if usable {
                let mic = recognize(microphone, &self.asr_model, &self.cancel)?;
                crate::ai::check_cancel(&self.cancel)?;
                let other = recognize(system, &self.asr_model, &self.cancel);
                crate::ai::check_cancel(&self.cancel)?;
                let mut other = match other {
                    Ok(value) => value,
                    Err(error) => {
                        // A later stem failure must not discard earlier raw words.
                        let mut partial = mic;
                        partial.completeness = crate::domain::RecognitionCompleteness::Incomplete;
                        partial.warnings.push(error);
                        return Ok(merge_stems(partial, empty_recognition()));
                    }
                };
                if mic.completeness == crate::domain::RecognitionCompleteness::Incomplete
                    || other.completeness == crate::domain::RecognitionCompleteness::Incomplete
                    || (!mic.text.trim().is_empty() && !other.text.trim().is_empty())
                {
                    // Release the native ASR process before loading the diarizer.
                    // The meeting retains its lease and transcribing status.
                    drop(recognize);
                    self.label_source(system, &mut other, false, &mut diarize)?;
                    return Ok(merge_stems(mic, other));
                }
            }
        }
        crate::ai::check_cancel(&self.cancel)?;
        let mut raw = recognize(&self.audio, &self.asr_model, &self.cancel)?;
        crate::ai::check_cancel(&self.cancel)?;
        drop(recognize);
        self.label_source(&self.audio, &mut raw, true, &mut diarize)?;
        Ok(MeetingRecognition { published: publish(&raw, true), raw })
    }

    fn label_source<D>(
        &self, audio: &Path, raw: &mut crate::asr::RecognitionResult, mixed: bool, diarize: &mut D,
    ) -> Result<(), String>
    where D: FnMut(&Path, &Path, &AtomicBool) -> Result<Vec<crate::diarization::Turn>, String> {
        let Some(root) = &self.diarization_root else { return Ok(()); };
        if raw.text.trim().is_empty() && raw.segments.iter().all(|segment| segment.text.trim().is_empty()) {
            return Ok(());
        }
        crate::ai::check_cancel(&self.cancel)?;
        let result = diarize(root, audio, &self.cancel);
        // Cancellation is never demoted to a warning or allowed to publish late text.
        crate::ai::check_cancel(&self.cancel)?;
        match result {
            Ok(turns) => {
                assign_speakers(raw, &turns, mixed);
                Ok(())
            }
            Err(error) => {
                raw.warnings.push(format!("Speaker labeling failed: {error}"));
                Ok(())
            }
        }
    }
}

struct MeetingRecognition {
    raw: crate::asr::RecognitionResult,
    published: String,
}

fn empty_recognition() -> crate::asr::RecognitionResult {
    crate::asr::RecognitionResult {
        original_text: None,
        text: String::new(), segments: Vec::new(),
        completeness: crate::domain::RecognitionCompleteness::Complete, warnings: Vec::new(),
    }
}

fn ensure_segments(result: &mut crate::asr::RecognitionResult) {
    if result.segments.is_empty() && !result.text.trim().is_empty() {
        result.segments.push(crate::asr::Segment {
                id: 0, start_seconds: 0.0, end_seconds: None, speaker: None,
                text: result.text.clone(),
                complete: result.completeness == crate::domain::RecognitionCompleteness::Complete,
        });
    }
}

/// Assign whole ASR segments; token timestamps are intentionally not inferred.
fn assign_speakers(
    raw: &mut crate::asr::RecognitionResult, turns: &[crate::diarization::Turn], mixed: bool,
) -> bool {
    let Some(first) = turns.first() else { return false; };
    ensure_segments(raw);
    raw.segments.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    let mut previous = None;
    let mut names = Vec::new();
    let mut labeled = false;
    for segment in &mut raw.segments {
        if segment.text.trim().is_empty() { continue; }
        let (start, end) = segment_bounds(segment);
        let (start, end) = (f64::from(start), f64::from(end));
        let mut overlap = [0.0f64; 8];
        for turn in turns {
            overlap[turn.speaker] += (end.min(turn.end) - start.max(turn.start)).max(0.0);
        }
        let mut selected = None;
        let mut best = 0.0;
        for (speaker, seconds) in overlap.into_iter().enumerate() {
            if seconds > best { selected = Some(speaker); best = seconds; }
        }
        if selected.is_none() {
            let mut nearest = f64::INFINITY;
            for turn in turns {
                let distance = (start - turn.end).max(turn.start - end).max(0.0);
                if distance <= 1.0 && (distance < nearest
                    || (distance == nearest && selected.is_none_or(|speaker| turn.speaker < speaker))) {
                    selected = Some(turn.speaker);
                    nearest = distance;
                }
            }
        }
        selected = selected.or(previous).or_else(|| mixed.then_some(first.speaker));
        if let Some(speaker) = selected {
            previous = Some(speaker);
            let number = match names.iter().position(|known| *known == speaker) {
                Some(index) => index + 1,
                None => { names.push(speaker); names.len() },
            };
            segment.speaker = Some(format!("Speaker {number}"));
            labeled = true;
        } else {
            segment.speaker = Some("Other speakers".into());
        }
    }
    // One voice on mixed audio adds a label to every passage and nothing else;
    // keep the plain transcript it had before diarization.
    if mixed && names.len() < 2 {
        for segment in &mut raw.segments { segment.speaker = None; }
        return false;
    }
    labeled
}

/// The microphone is You; system segments keep numbered voices from speaker
/// labeling and are otherwise Other speakers.
fn merge_stems(mut mic: crate::asr::RecognitionResult, mut system: crate::asr::RecognitionResult) -> MeetingRecognition {
    // Preserve each original stem in source order. Only guarded segments are
    // interleaved or speaker-labeled; a dropped section may have no segment.
    let original_text = if mic.original_text.is_some() || system.original_text.is_some() {
        Some([mic.original_text(), system.original_text()].into_iter()
            .filter(|text| !text.is_empty()).collect::<Vec<_>>().join(" "))
    } else { None };
    ensure_segments(&mut mic);
    ensure_segments(&mut system);
    for segment in &mut mic.segments { segment.speaker = Some("You".into()); }
    for segment in &mut system.segments {
        segment.speaker.get_or_insert_with(|| "Other speakers".into());
    }
    if system.completeness == crate::domain::RecognitionCompleteness::Incomplete {
        mic.completeness = crate::domain::RecognitionCompleteness::Incomplete;
    }
    mic.warnings.extend(system.warnings);
    mic.segments.extend(system.segments);
    mic.segments.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    mic.text = mic.segments.iter().filter(|s| !s.text.is_empty())
        .map(|s| s.text.as_str()).collect::<Vec<_>>().join(" ");
    mic.original_text = original_text;
    MeetingRecognition { published: publish(&mic, true), raw: mic }
}

fn numbered_voice(label: &str) -> bool {
    label.strip_prefix("Speaker ").is_some_and(|n| n.parse::<u8>().is_ok())
}

/// Distinct numbered voices in a recognition. One voice is not worth labeling.
fn detected_voices(segments: &[crate::asr::Segment]) -> usize {
    let mut seen: Vec<&str> = Vec::new();
    for label in segments.iter().filter(|s| !s.text.trim().is_empty()).filter_map(|s| s.speaker.as_deref()) {
        if numbered_voice(label) && !seen.contains(&label) { seen.push(label); }
    }
    seen.len()
}

/// The published form of a recognition: numbered voices when labels are on
/// and at least two were found; otherwise You / Other speakers for two-source
/// audio and plain text for one source.
fn publish(raw: &crate::asr::RecognitionResult, labels: bool) -> String {
    if labels && detected_voices(&raw.segments) >= 2 {
        return speaker_passages(&raw.segments);
    }
    if raw.segments.iter().any(|s| s.speaker.as_deref() == Some("You")) {
        let sides: Vec<crate::asr::Segment> = raw.segments.iter().cloned().map(|mut s| {
            if s.speaker.as_deref() != Some("You") { s.speaker = Some("Other speakers".into()); }
            s
        }).collect();
        return speaker_passages(&sides);
    }
    tidy_spaces(&raw.text)
}

/// Recognizers pad segments with leading spaces, so joined text can carry
/// doubled ones. Published text collapses runs of spaces and tabs within each
/// line and trims the ends; the saved raw recognition keeps them.
fn tidy_spaces(text: &str) -> String {
    text.lines()
        .map(|line| line.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn segment_bounds(segment: &crate::asr::Segment) -> (f32, f32) {
    let start = if segment.start_seconds.is_finite() { segment.start_seconds.max(0.0) } else { 0.0 };
    let end = segment.end_seconds.filter(|end| end.is_finite() && *end >= start).unwrap_or(start);
    (start, end)
}

fn speaker_passages(segments: &[crate::asr::Segment]) -> String {
    let mut blocks: Vec<(f32, f32, String, String)> = Vec::new();
    for segment in segments {
        if segment.text.trim().is_empty() { continue; }
        let (start, end) = segment_bounds(segment);
        let speaker = segment.speaker.as_deref().unwrap_or("Other speakers");
        if let Some(last) = blocks.last_mut() {
            if last.2 == speaker && start - last.1 < 2.0 {
                last.1 = last.1.max(end);
                last.3.push(' ');
                last.3.push_str(&tidy_spaces(&segment.text));
                continue;
            }
        }
        blocks.push((start, end, speaker.into(), tidy_spaces(&segment.text)));
    }
    blocks.into_iter().map(|(start, _, speaker, text)| {
        let seconds = start.floor() as u64;
        let clock = if seconds >= 3600 {
            format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
        } else { format!("{}:{:02}", seconds / 60, seconds % 60) };
        format!("[{clock}] {speaker}: {text}")
    }).collect::<Vec<_>>().join("\n\n")
}

/// RAII release for one meeting transcription. Drop always clears the active
/// flag so cancellation and errors cannot strand the coordinator busy.
struct MeetingAsrLease {
    finished: bool,
}

impl MeetingAsrLease {
    fn finish(&mut self) {
        if !self.finished {
            crate::resource_manager::notify_asr_finished();
            self.finished = true;
        }
    }
}

impl Drop for MeetingAsrLease {
    fn drop(&mut self) {
        if !self.finished {
            crate::resource_manager::notify_asr_finished();
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct MeetingRecord {
    schema_version: u32,
    detail: MeetingDetail,
    /// The first completed ASR result is never replaced by user edits or subsequent decodes.
    raw_transcript: Option<String>,
    /// Old records default to protected: their title may have been deliberately edited.
    #[serde(default = "protected_title")]
    title_user_edited: bool,
    /// Survives a restart or a disabled preference until idle naming succeeds.
    #[serde(default)]
    automatic_title_pending: bool,
    #[serde(default)]
    legacy_audio: Option<PathBuf>,
    #[serde(default)]
    legacy_microphone_audio: Option<PathBuf>,
    #[serde(default)]
    legacy_system_audio: Option<PathBuf>,
    #[serde(default)]
    source_media: Option<PathBuf>,
    /// History file (in `<id>.asr/`) that produced the current transcript, so
    /// speaker labels can be switched without recognizing again.
    #[serde(default)]
    recognition_file: Option<String>,
    #[serde(default)]
    outages_recorded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct TitleCandidate {
    pub id: String,
    pub version: u64,
}

impl MeetingRecord {
    fn needs_automatic_title(&self) -> bool {
        !self.title_user_edited
            && !busy(&self.detail.status)
            && self.detail.recognition_completeness == crate::domain::RecognitionCompleteness::Complete
            && !self.detail.transcript.trim().is_empty()
            && (self.automatic_title_pending
                || self.detail.title.trim().eq_ignore_ascii_case("Untitled meeting")
                || self.detail.title.trim().eq_ignore_ascii_case("Imported meeting"))
    }

    fn title_candidate(&self) -> TitleCandidate {
        use std::hash::{Hash, Hasher};
        let mut version = std::collections::hash_map::DefaultHasher::new();
        self.detail.title.hash(&mut version);
        self.detail.transcript.hash(&mut version);
        self.recognition_file.hash(&mut version);
        TitleCandidate { id: self.detail.id.clone(), version: version.finish() }
    }
}

impl Meetings {
    pub fn new(store: Store) -> Self {
        Self {
            store,
            operations: Arc::new(Mutex::new(HashMap::new())),
            revision: Arc::new(AtomicU64::new(0)),
            automatic_titles: Arc::default(),
        }
    }
    pub fn store(&self) -> &Store {
        &self.store
    }

    pub(crate) fn automatic_title_candidates(&self) -> Result<Vec<TitleCandidate>, String> {
        // Run on the blocking worker. Release the store between records so a
        // large backfill never holds the library lock across model inference.
        let mut candidates = Vec::new();
        for entry in fs::read_dir(self.store.meetings_dir()).map_err(|e| format!("Cannot list meetings: {e}"))? {
            let path = entry.map_err(|e| format!("Cannot inspect meeting: {e}"))?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") { continue; }
            let Some(id) = path.file_stem().and_then(|s| s.to_str()) else { continue };
            let _guard = self.store.lock()?;
            // A deletion between list and read is harmless.
            if let Ok(record) = self.read(id) {
                if record.needs_automatic_title() {
                    candidates.push(record.title_candidate());
                }
            }
            if candidates.len() >= 20_000 { break; }
        }
        Ok(candidates)
    }

    pub(crate) fn generate_automatic_title<F>(
        &self,
        candidate: &TitleCandidate,
        cancel: &AtomicBool,
        infer: F,
    ) -> Result<bool, String>
    where
        F: FnOnce(&str, &AtomicBool) -> Result<String, String>,
    {
        let record = {
            let _guard = self.store.lock()?;
            let record = self.read(&candidate.id)?;
            if cancel.load(Ordering::Acquire)
                || !self.store.settings_unlocked()?.auto_meeting_titles
                || !record.needs_automatic_title()
                || record.title_candidate() != *candidate
            {
                return Ok(false);
            }
            record
        };
        // No busy status or meeting-operation claim: users can edit, delete or
        // transcribe while this optional job runs. Publication compares the
        // saved source again under the store lock and never replaces their work.
        let title = infer(&record.detail.transcript, cancel)?;
        let _guard = self.store.lock()?;
        if cancel.load(Ordering::Acquire) || !self.store.settings_unlocked()?.auto_meeting_titles {
            return Ok(false);
        }
        let Ok(mut current) = self.read(&candidate.id) else { return Ok(false) };
        if !current.needs_automatic_title() || current.title_candidate() != *candidate {
            return Ok(false);
        }
        validate_title(&title)?;
        current.detail.summary.title = title.trim().into();
        current.automatic_title_pending = false;
        // Clear only the old inline title failure, never a recording/ASR notice.
        if current.detail.error.as_deref().is_some_and(|e| e.starts_with("Transcript saved. Automatic title unavailable:")) {
            current.detail.error = None;
        }
        self.write(&current)?;
        Ok(true)
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub(crate) fn cancel_all(&self) {
        self.automatic_titles.cancel();
        if let Ok(operations) = self.operations.lock() {
            for cancel in operations.values() {
                cancel.store(true, Ordering::Release);
            }
        }
    }
    fn path(&self, id: &str) -> Result<PathBuf, String> {
        validate_id(id)?;
        Ok(self.store.meetings_dir().join(format!("{id}.json")))
    }
    fn read(&self, id: &str) -> Result<MeetingRecord, String> {
        let mut record: MeetingRecord = self.store.read(&self.path(id)?)?;
        if record.schema_version != 1 || record.detail.id != id {
            return Err(
                "The meeting record has an unsupported schema or mismatched identifier.".into(),
            );
        }
        validate_record(&record)?;
        let audio = self.audio_for(&record, "mixed")?;
        record.detail.audio_path = audio.as_ref().map(|p| p.to_string_lossy().into_owned());
        record.detail.summary.has_audio = audio.as_ref().is_some_and(|p| p.is_file());
        // Older failed stops saved audio but left duration at zero. Derive it
        // from the existing header for reads/lists; never rewrite the audio or
        // record just to display a recovered duration.
        if record.detail.status == "failed" && record.detail.duration_seconds == 0.0 {
            if let Some(duration) = audio.as_ref()
                .and_then(|path| wav_duration(path).ok())
                .filter(|duration| duration.is_finite() && *duration > 0.0)
            {
                record.detail.summary.duration_seconds = duration;
            }
        }
        record.detail.summary.has_transcript = !record.detail.transcript.trim().is_empty();
        if !record.outages_recorded {
            record.detail.audio_outages.retain(|gap| gap.source == "recognition");
            let gaps = outages::legacy(&record.detail.recording_warnings, record.detail.duration_seconds);
            record.detail.missing_seconds = gaps.iter().map(|gap| gap.end_seconds - gap.start_seconds)
                .sum::<f64>().min(record.detail.duration_seconds);
            record.detail.audio_outages.extend(gaps);
        }
        // Interpret old saved diagnostics with the same meeting-only policy.
        // Do not rewrite recognition history or mutate the record on a read.
        if matches!(record.detail.status.as_str(), "interrupted" | "transcribed" | "processed")
            && record.detail.recognition_completeness == crate::domain::RecognitionCompleteness::Incomplete
            && record.detail.transcription_stopped_at_seconds.is_none()
        {
            let segments = record.recognition_file.as_ref().and_then(|file| {
                let path = self.store.meetings_dir().join(format!("{id}.asr")).join(file);
                let history: Value = self.store.read(&path).ok()?;
                serde_json::from_value(history["segments"].clone()).ok()
            }).unwrap_or_default();
            let recognition = crate::asr::RecognitionResult {
                original_text: None, text: record.detail.transcript.clone(), segments,
                completeness: record.detail.recognition_completeness,
                warnings: record.detail.recognition_warnings.clone(),
            };
            if outages::genuine_stop(recognition.completeness, &recognition.warnings) {
                let at = outages::stopped_at(&recognition, record.detail.duration_seconds);
                record.detail.transcription_stopped_at_seconds = Some(at);
                record.detail.error = Some(format!("Transcription stopped at {}. Saved audio is available to transcribe again.", outages::clock(at)));
            } else {
                if outages::collapse_only(&recognition) {
                    record.detail.recognition_completeness = crate::domain::RecognitionCompleteness::Complete;
                }
                if !record.detail.audio_outages.iter().any(|gap| gap.source == "recognition") {
                    record.detail.audio_outages.extend(outages::recognition(&recognition, record.detail.duration_seconds));
                }
                record.detail.summary.status = completed_status(&record.detail).into();
                record.detail.error = None;
            }
        }
        Ok(record)
    }
    fn write(&self, record: &MeetingRecord) -> Result<(), String> {
        validate_record(record)?;
        self.store.write(&self.path(&record.detail.id)?, record)?;
        self.revision.fetch_add(1, Ordering::Release);
        self.automatic_titles.wake();
        Ok(())
    }
    pub fn get(&self, id: &str) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        Ok(self.read(id)?.detail)
    }
    pub fn list(&self) -> Result<Vec<MeetingSummary>, String> {
        let _guard = self.store.lock()?;
        let mut rows = Vec::new();
        for entry in fs::read_dir(self.store.meetings_dir())
            .map_err(|e| format!("Cannot list meetings: {e}"))?
        {
            let path = entry
                .map_err(|e| format!("Cannot inspect meeting: {e}"))?
                .path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("A meeting filename is invalid.")?;
            rows.push(self.read(id)?.detail.summary);
            if rows.len() > 20_000 {
                return Err("The meeting library exceeds the 20,000-record limit.".into());
            }
        }
        rows.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.id.cmp(&b.id))
        });
        Ok(rows)
    }
    pub fn recording_paths(&self, id: &str) -> Result<RecordingPaths, String> {
        validate_id(id)?;
        let directory = self.store.meetings_dir();
        let paths = RecordingPaths {
            mixed: directory.join(format!("{id}.wav")),
            microphone: directory.join(format!("{id}.mic.wav")),
            system: directory.join(format!("{id}.system.wav")),
        };
        for path in [&paths.mixed, &paths.microphone, &paths.system] {
            self.store.check_owned(path)?;
        }
        Ok(paths)
    }
    fn create(
        &self,
        title: &str,
        source: &str,
        status: &str,
        media: Option<PathBuf>,
        title_user_edited: bool,
    ) -> Result<MeetingDetail, String> {
        validate_title(title)?;
        let _guard = self.store.lock()?;
        let id = new_id();
        let created_at = now();
        let audio_path = self
            .recording_paths(&id)?
            .mixed
            .to_string_lossy()
            .into_owned();
        let record = MeetingRecord {
            schema_version: 1,
            detail: MeetingDetail {
                recognition_completeness: crate::domain::RecognitionCompleteness::Complete,
                recognition_warnings: Vec::new(),
                recording_warnings: Vec::new(),
                audio_outages: Vec::new(), missing_seconds: 0.0,
                cleaned_transcript: None, transcription_stopped_at_seconds: None,
                summary: MeetingSummary {
                    id,
                    title: title.trim().into(),
                    created_at,
                    duration_seconds: 0.0,
                    status: status.into(),
                    has_audio: false,
                    has_transcript: false,
                },
                transcript: String::new(),
                notes: String::new(),
                source: source.into(),
                audio_path: Some(audio_path),
                error: None,
                transcription_model: None, speaker_count: 0, speaker_labels: true,
            },
            raw_transcript: None,
            title_user_edited,
            automatic_title_pending: false,
            legacy_audio: None,
            legacy_microphone_audio: None,
            legacy_system_audio: None,
            source_media: media,
            recognition_file: None,
            outages_recorded: false,
        };
        self.write(&record)?;
        Ok(record.detail)
    }
    pub fn create_recording(&self, title: &str, source: &str) -> Result<MeetingDetail, String> {
        self.automatic_titles.cancel();
        if !matches!(source, "microphone" | "system" | "both") {
            return Err("Choose microphone, system audio, or both.".into());
        }
        self.create(
            if title.trim().is_empty() {
                "Untitled meeting"
            } else {
                title
            },
            source,
            "recording",
            None,
            !title.trim().is_empty(),
        )
    }
    pub fn finish_recording(
        &self,
        id: &str,
        duration_seconds: f64,
    ) -> Result<MeetingDetail, String> {
        self.finish_recording_with_warnings(id, duration_seconds, Vec::new())
    }
    pub fn begin_recording_finalization(&self, id: &str) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if record.detail.status != "recording" {
            return Err("This meeting is no longer recording.".into());
        }
        record.detail.summary.status = "finalizing".into();
        self.write(&record)?;
        Ok(record.detail)
    }

    /// Call only after the recorder returns its result or its worker exits.
    pub(crate) fn complete_recording(
        &self,
        id: &str,
        result: Result<crate::recording::RecordingOutcome, String>,
        warnings: Vec<String>,
    ) -> Result<MeetingDetail, String> {
        match result {
            Ok(outcome) => self.finish_recording_details(id, outcome.duration, outcome.warnings(), Some(&outcome.issues))
                .or_else(|error| self.recover_recording_with_warnings(id, &error, warnings)),
            Err(error) => self.recover_recording_with_warnings(id, &error, warnings),
        }
    }

    pub(crate) fn complete_recording_with_issues(
        &self, id: &str, result: Result<crate::recording::RecordingOutcome, String>,
        mut warnings: Vec<String>, issues: Vec<crate::recording::SourceIssue>,
    ) -> Result<MeetingDetail, String> {
        if let Err(error) = &result {
            let duration = wav_duration(&self.recording_paths(id)?.mixed).unwrap_or(0.0);
            if duration.is_finite() && duration > 0.0 && !issues.is_empty() {
                warnings.push(format!("Recording stopped early: {}", error.chars().take(2048).collect::<String>()));
                return self.finish_recording_details(id, duration, warnings, Some(&issues));
            }
        }
        self.complete_recording(id, result, warnings)
    }

    pub fn finish_recording_with_warnings(
        &self,
        id: &str,
        duration_seconds: f64,
        warnings: Vec<String>,
    ) -> Result<MeetingDetail, String> {
        self.finish_recording_details(id, duration_seconds, warnings, None)
    }
    fn finish_recording_details(
        &self, id: &str, duration_seconds: f64, warnings: Vec<String>,
        issues: Option<&[crate::recording::SourceIssue]>,
    ) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if !matches!(record.detail.status.as_str(), "recording" | "finalizing") {
            return Err("This meeting is no longer recording.".into());
        }
        if !duration_seconds.is_finite() || duration_seconds <= 0.0 {
            return Err("The recording contains no audio.".into());
        }
        let duration = wav_duration(&self.recording_paths(id)?.mixed)?;
        if duration <= 0.0 {
            return Err("The recording contains no audio.".into());
        }
        record.detail.summary.duration_seconds = duration;
        record.detail.summary.status = "ready".into();
        record.detail.summary.has_audio = true;
        record.detail.recording_warnings = warnings;
        let (gaps, missing) = match issues {
            Some(issues) => {
                let (mut gaps, missing) = outages::capture(issues, duration);
                gaps.extend(outages::legacy(&record.detail.recording_warnings, duration).into_iter()
                    .filter(|gap| gap.source == "recording"));
                (gaps, missing)
            },
            None => {
                let gaps = outages::legacy(&record.detail.recording_warnings, duration);
                let missing = gaps.iter().map(|gap| gap.end_seconds - gap.start_seconds).sum::<f64>().min(duration);
                (gaps, missing)
            }
        };
        record.detail.audio_outages = gaps;
        record.detail.missing_seconds = missing;
        record.outages_recorded = true;
        record.detail.error = None;
        self.write(&record)?;
        Ok(record.detail)
    }
    /// A fatal stop may still have finalized useful audio. Keep that recording
    /// ready for transcription, with its actual header duration and diagnostic.
    pub fn recover_recording(&self, id: &str, error: &str) -> Result<MeetingDetail, String> {
        self.recover_recording_with_warnings(id, error, Vec::new())
    }
    pub fn recover_recording_with_warnings(&self, id: &str, error: &str, mut warnings: Vec<String>) -> Result<MeetingDetail, String> {
        let duration = wav_duration(&self.recording_paths(id)?.mixed).unwrap_or(0.0);
        if duration.is_finite() && duration > 0.0 {
            warnings.push(format!("Recording stopped early: {}", error.chars().take(2048).collect::<String>()));
            return self.finish_recording_with_warnings(
                id,
                duration,
                warnings,
            );
        }
        self.fail(id, error)?;
        Err(error.into())
    }
    pub fn save(
        &self,
        id: &str,
        title: &str,
        transcript: &str,
        notes: &str,
    ) -> Result<MeetingDetail, String> {
        self.save_with_cleaned(id, title, transcript, notes, None)
    }
    pub fn save_with_cleaned(
        &self, id: &str, title: &str, transcript: &str, notes: &str,
        cleaned_transcript: Option<&str>,
    ) -> Result<MeetingDetail, String> {
        validate_title(title)?;
        validate_text(transcript, "Transcript")?;
        validate_text(notes, "Notes")?;
        if let Some(cleaned) = cleaned_transcript { validate_text(cleaned, "Cleaned transcript")?; }
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if busy(&record.detail.status) {
            return Err("Finish or cancel the current meeting operation before editing.".into());
        }
        if record.detail.title != title.trim() {
            record.title_user_edited = true;
            record.automatic_title_pending = false;
            record.detail.summary.title = title.trim().into();
        }
        record.detail.transcript = transcript.into();
        record.detail.notes = notes.into();
        if let Some(cleaned) = cleaned_transcript {
            record.detail.cleaned_transcript = Some(cleaned.into());
        }
        record.detail.summary.has_transcript = !transcript.trim().is_empty();
        record.detail.summary.status = completed_status(&record.detail).into();
        self.write(&record)?;
        Ok(record.detail)
    }
    /// Rebuilds the transcript from the recognition that produced it, with or
    /// without numbered voices. Edits since then are replaced only when confirmed.
    pub fn set_speaker_labels(
        &self,
        id: &str,
        enabled: bool,
        confirmed: bool,
    ) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if busy(&record.detail.status) {
            return Err("Finish or cancel the current meeting operation first.".into());
        }
        if record.detail.speaker_labels == enabled {
            return Ok(record.detail);
        }
        let file = record.recognition_file.clone()
            .ok_or("This meeting has no saved speaker labels. Transcribe it again first.")?;
        let path = self.store.meetings_dir().join(format!("{id}.asr")).join(file);
        let saved: Value = self.store.read(&path)?;
        let raw = crate::asr::RecognitionResult {
            original_text: None,
            // New history stores raw speech separately. An explicitly empty
            // guarded string is authoritative; never fall back to raw for it.
            text: saved.get("guarded").unwrap_or(&saved["raw"])
                .as_str().ok_or("Cannot read saved recognition text.")?.into(),
            segments: serde_json::from_value(saved["segments"].clone())
                .map_err(|e| format!("Cannot read saved speaker labels: {e}"))?,
            completeness: crate::domain::RecognitionCompleteness::Complete,
            warnings: Vec::new(),
        };
        if detected_voices(&raw.segments) < 2 {
            return Err("Speaker labels need at least two detected voices.".into());
        }
        // Transcripts published before spaces were tidied still count as unedited.
        let current = publish(&raw, record.detail.speaker_labels);
        if !confirmed && (record.detail.cleaned_transcript.is_some()
            || tidy_spaces(&record.detail.transcript) != tidy_spaces(&current)) {
            return Err(SPEAKER_LABELS_REPLACE_EDITS.into());
        }
        record.detail.transcript = publish(&raw, enabled);
        record.detail.cleaned_transcript = None;
        record.detail.speaker_labels = enabled;
        record.detail.summary.has_transcript = !record.detail.transcript.trim().is_empty();
        self.write(&record)?;
        Ok(record.detail)
    }
    /// Only fixed files derived from this record's validated ID are deleted. Legacy/source files are never deleted.
    pub fn delete(&self, id: &str) -> Result<(), String> {
        self.cancel(id)?;
        let _guard = self.store.lock()?;
        let _record = self.read(id)?;
        if self
            .operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?
            .contains_key(id)
        {
            return Err(
                "The meeting is still stopping. Delete it again after the operation finishes."
                    .into(),
            );
        }
        let paths = self.recording_paths(id)?;
        for path in [
            &paths.mixed,
            &paths.microphone,
            &paths.system,
            &self.store.meetings_dir().join(format!("{id}.import.wav")),
        ] {
            self.store.check_owned(path)?;
            if path.is_file() {
                fs::remove_file(path)
                    .map_err(|e| format!("Cannot remove owned meeting audio: {e}"))?;
            }
        }
        let history = self.store.meetings_dir().join(format!("{id}.asr"));
        if history.is_dir() {
            self.store.check_owned(&history)?;
            for entry in fs::read_dir(&history)
                .map_err(|e| format!("Cannot inspect transcript history: {e}"))?
            {
                let path = entry
                    .map_err(|e| format!("Cannot inspect transcript history: {e}"))?
                    .path();
                self.store.check_owned(&path)?;
                if path.extension().and_then(|e| e.to_str()) == Some("json") && path.is_file() {
                    fs::remove_file(path)
                        .map_err(|e| format!("Cannot remove transcript history: {e}"))?;
                }
            }
            fs::remove_dir(history)
                .map_err(|e| format!("Cannot remove transcript history directory: {e}"))?;
        }
        fs::remove_file(self.path(id)?)
            .map_err(|e| format!("Cannot remove the meeting record: {e}"))?;
        self.revision.fetch_add(1, Ordering::Release);
        Ok(())
    }
    pub fn select_audio(&self, id: &str, source: &str) -> Result<PathBuf, String> {
        let _guard = self.store.lock()?;
        let record = self.read(id)?;
        if matches!(record.detail.status.as_str(), "recording" | "finalizing") {
            return Err("Wait for the recording to finish saving before using its audio.".into());
        }
        self.audio_for(&record, source)?
            .filter(|p| p.is_file())
            .ok_or_else(|| "This meeting has no saved audio for the selected source.".into())
    }
    fn audio_for(&self, record: &MeetingRecord, source: &str) -> Result<Option<PathBuf>, String> {
        let owned = self.recording_paths(&record.detail.id)?;
        let (path, legacy) = match source {
            "mixed" | "both" => (owned.mixed, &record.legacy_audio),
            "microphone" => (owned.microphone, &record.legacy_microphone_audio),
            "system" => (owned.system, &record.legacy_system_audio),
            _ => return Err("Unknown meeting audio source.".into()),
        };
        if path.is_file() {
            Ok(Some(path))
        } else {
            Ok(legacy.clone().or(Some(path)))
        }
    }
    pub fn raw_transcript(&self, id: &str) -> Result<Option<String>, String> {
        let _guard = self.store.lock()?;
        Ok(self.read(id)?.raw_transcript)
    }
    fn begin(&self, id: &str, status: &str) -> Result<(MeetingRecord, Arc<AtomicBool>), String> {
        // Foreground meeting work takes priority over optional idle naming.
        self.automatic_titles.cancel();
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if matches!(status, "processingNotes" | "generatingTitle")
            && outages::genuine_stop(record.detail.recognition_completeness, &record.detail.recognition_warnings)
        {
            return Err("Recognition is incomplete. Retry transcription before generating notes or a title.".into());
        }
        let pending_import = status == "importing" && record.detail.status == "importing";
        if busy(&record.detail.status) && !pending_import {
            return Err("This meeting already has an active operation.".into());
        }
        if status == "importing" && record.detail.status == "cancelled" {
            return Err("Media import cancelled.".into());
        }
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?;
        if operations.contains_key(id) {
            return Err("The previous meeting operation is still stopping.".into());
        }
        let cancel = Arc::new(AtomicBool::new(false));
        record.detail.summary.status = status.into();
        record.detail.error = None;
        self.write(&record)?;
        operations.insert(id.into(), cancel.clone());
        Ok((record, cancel))
    }
    pub fn cancel(&self, id: &str) -> Result<(), String> {
        validate_id(id)?;
        let _guard = self.store.lock()?;
        if let Some(cancel) = self
            .operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?
            .get(id)
        {
            cancel.store(true, Ordering::SeqCst);
        }
        let mut record = self.read(id)?;
        if matches!(record.detail.status.as_str(), "recording" | "finalizing") {
            return Err("Wait for the recording to finish saving before cancelling or deleting it.".into());
        }
        if busy(&record.detail.status) {
            record.detail.summary.status = "cancelled".into();
            record.detail.error =
                Some("Operation cancelled. Saved audio and completed text were preserved.".into());
            self.write(&record)?;
        }
        Ok(())
    }
    pub fn fail(&self, id: &str, error: &str) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        record.detail.summary.status = "failed".into();
        record.detail.error = Some(error.chars().take(2048).collect());
        self.write(&record)?;
        Ok(record.detail)
    }
    /// `model` overrides the Speech setting for this one job (Transcribe again).
    pub fn begin_transcription(
        &self,
        id: &str,
        model: Option<&str>,
    ) -> Result<MeetingTranscription, String> {
        let activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Meeting)?;
        if model.is_some_and(|m| m.is_empty() || m.len() > 128 || m.contains('\0')) {
            return Err("Choose an installed speech model.".into());
        }
        let audio = self.select_audio(id, "mixed")?;
        let remote = if model == Some("openrouter") {
            Some(crate::meeting_speech::Snapshot::load(&self.store)?)
        } else { None };
        // Snapshot the selected ASR model once at begin; run() reuses it for
        // both the lease and the recognizer so a Settings edit cannot tear
        // them. Unknown IDs are preserved verbatim and fail in Recognizer.
        let settings = self.store.load_settings().unwrap_or_default();
        let asr_model = match (remote.as_ref(), model) {
            (Some(snapshot), _) => format!("openrouter:{}", snapshot.model),
            (_, Some(model)) => model.into(),
            (_, None) if !settings.meeting_asr_model.is_empty() => settings.meeting_asr_model,
            (_, None) => settings.asr_model,
        };
        let models = self.store.models_dir();
        let diarization_root = (remote.is_none() && settings.meeting_speaker_labels
            && crate::diarization_model::installed(&models)).then_some(models);
        let stems = {
            let _guard = self.store.lock()?;
            let record = self.read(id)?;
            if matches!(record.detail.source.as_str(), "both" | "legacy") && record.source_media.is_none() {
                self.audio_for(&record, "microphone")?.zip(self.audio_for(&record, "system")?)
            } else { None }
        };
        let (_, cancel) = self.begin(id, "transcribing")?;
        Ok(MeetingTranscription {
            _update_activity: activity,
            meetings: self.clone(),
            id: id.into(),
            audio,
            stems,
            cancel,
            asr_model,
            remote,
            diarization_root,
        })
    }
    pub fn generate_notes<F>(
        &self,
        id: &str,
        instructions: &str,
        infer: F,
    ) -> Result<MeetingDetail, String>
    where
        F: FnOnce(&str, &str, &AtomicBool) -> Result<String, String>,
    {
        let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Ai)?;
        if instructions.len() > 16_384 || instructions.contains('\0') {
            return Err("Note instructions are limited to 16 KiB.".into());
        }
        if self.get(id)?.transcript.trim().is_empty() {
            return Err("Transcribe this meeting before generating notes.".into());
        }
        let (record, token) = self.begin(id, "processingNotes")?;
        // Single-owner rule (Round 6 final): this method owns operation state
        // only (begin/finish). The text lease is owned by the snapshot worker
        // in meeting_ai (generate_notes_with_snapshot / held variant), which
        // snapshots backend/model once and reuses held calls for every chunk.
        // Callers must NOT call ai::generate (per-request acquire) inside;
        // use the snapshot APIs so a whole job never acquires twice and fails
        // busy against itself. Busy/missing fails via finish_operation with
        // existing notes preserved; cancellation discards late output.
        if let Ok(settings) = self.store.load_settings() {
            crate::resource_manager::configure(&settings);
        }
        let result = infer(&record.detail.transcript, instructions, &token);
        self.finish_operation(id, &token, result, true, None)
    }
    /// Title generation never replaces a user-supplied title or changes saved text.
    pub fn generate_title<F>(&self, id: &str, infer: F) -> Result<MeetingDetail, String>
    where
        F: FnOnce(&str, &AtomicBool) -> Result<String, String>,
    {
        // The desktop scheduler owns automatic naming. Legacy command callers
        // still reach here after transcription; never run a second inline job
        // (or use their notes/cloud selection) alongside the idle worker.
        if self.automatic_titles.is_managed() {
            self.automatic_titles.wake();
            return self.get(id);
        }
        let _activity = crate::resource_manager::begin_update_activity(crate::resource_manager::UpdateActivity::Ai)?;
        {
            let _guard = self.store.lock()?;
            let record = self.read(id)?;
            if record.title_user_edited || record.detail.transcript.trim().is_empty() {
                return Ok(record.detail);
            }
        }
        let (record, token) = self.begin(id, "generatingTitle")?;
        // Single-owner rule: operation state only. Text lease ownership lives
        // in meeting_ai snapshot title APIs. Title never replaces a user title;
        // missing/busy fails with transcript preserved via finish_title_result.
        if let Ok(settings) = self.store.load_settings() {
            crate::resource_manager::configure(&settings);
        }
        let result = if record.title_user_edited {
            Ok(record.detail.title.clone())
        } else {
            infer(&record.detail.transcript, &token)
        };
        self.finish_title_result(id, &token, &record, result)
    }

    fn finish_title_result(
        &self,
        id: &str,
        token: &Arc<AtomicBool>,
        record: &MeetingRecord,
        result: Result<String, String>,
    ) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?;
        if !operations
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, &token))
        {
            return Err("The meeting operation is no longer current.".into());
        }
        operations.remove(id);
        let mut current = self.read(id)?;
        if token.load(Ordering::SeqCst) || current.detail.status == "cancelled" {
            return Err("Meeting title generation cancelled.".into());
        }
        current.detail.summary.status = completed_status(&current.detail).into();
        match result.and_then(|title| {
            validate_title(&title)?;
            Ok(title)
        }) {
            Ok(title) => {
                if !current.title_user_edited && current.detail.title == record.detail.title {
                    current.detail.summary.title = title.trim().into();
                    current.automatic_title_pending = false;
                }
                current.detail.error = None;
                self.write(&current)?;
                Ok(current.detail)
            }
            Err(error) => {
                current.detail.error = Some(format!(
                    "Transcript saved. Automatic title unavailable: {}",
                    error.chars().take(1700).collect::<String>()
                ));
                self.write(&current)?;
                Err(error)
            }
        }
    }
    fn finish_recognition_operation(
        &self,
        id: &str,
        token: &Arc<AtomicBool>,
        result: Result<MeetingRecognition, String>,
        model: &str,
    ) -> Result<MeetingDetail, String> {
        let result = result.and_then(|result| {
            validate_text(&result.raw.text, "Guarded transcript")?;
            validate_text(result.raw.original_text(), "Raw transcript")?;
            validate_text(&result.published, "Transcript")?;
            if result.raw.completeness == crate::domain::RecognitionCompleteness::Complete
                && result.published.trim().is_empty()
            {
                return Err("The model returned no text.".into());
            }
            Ok(result)
        });
        let result = match result {
            Ok(value) => value,
            Err(error) => return self.finish_operation(id, token, Err(error), false, Some(model)),
        };
        let _guard = self.store.lock()?;
        let mut operations = self.operations.lock().map_err(|_| "Meeting operation lock failed.")?;
        if !operations.get(id).is_some_and(|current| Arc::ptr_eq(current, token)) {
            return Err("The meeting operation is no longer current.".into());
        }
        operations.remove(id);
        let mut record = self.read(id)?;
        if token.load(Ordering::Acquire) || record.detail.status == "cancelled" {
            return Err("Meeting operation cancelled.".into());
        }
        let recognition = result.raw;
        let incomplete = recognition.completeness == crate::domain::RecognitionCompleteness::Incomplete
            && !outages::collapse_only(&recognition);
        let stopped = outages::genuine_stop(recognition.completeness, &recognition.warnings);
        let directory = self.store.meetings_dir().join(format!("{id}.asr"));
        self.store.check_owned(&directory)?;
        fs::create_dir_all(&directory).map_err(|e| format!("Cannot preserve raw recognition: {e}"))?;
        let history = format!("{}.json", new_id());
        let mut saved = serde_json::json!({
            "createdAt":now(), "raw":recognition.original_text(), "segments":recognition.segments,
            "completeness":recognition.completeness, "warnings":recognition.warnings, "model":model,
        });
        if recognition.original_text.is_some() {
            saved["guarded"] = serde_json::json!(&recognition.text);
        }
        self.store.write(&directory.join(&history), &saved)?;
        // Incomplete retries retain the existing completed/edited document.
        let keep_previous = incomplete && !record.detail.transcript.trim().is_empty();
        if !keep_previous {
            // The meeting's own speaker-label choice survives transcribing again.
            record.detail.transcript = if record.detail.speaker_labels {
                result.published
            } else {
                publish(&recognition, false)
            };
            record.detail.transcription_model = Some(model.into());
            record.detail.speaker_count = detected_voices(&recognition.segments) as u32;
            record.recognition_file = Some(history);
            // A completed replacement is a new source for Clean. Incomplete
            // retries keep the existing edited document and its cleaned copy.
            if !incomplete { record.detail.cleaned_transcript = None; }
            record.detail.audio_outages.retain(|gap| gap.source != "recognition");
            record.detail.audio_outages.extend(outages::recognition(&recognition, record.detail.duration_seconds));
        }
        if !incomplete && record.raw_transcript.is_none() {
            record.raw_transcript = Some(recognition.original_text().to_owned());
        }
        if keep_previous && !stopped {
            // A limited retry keeps the earlier editable document, but its
            // missing sections still need an honest, timed notice.
            let mut gaps = outages::recognition(&recognition, record.detail.duration_seconds);
            for gap in &mut gaps { gap.reason.push_str("; previous transcript kept"); }
            record.detail.audio_outages.retain(|gap| gap.source != "recognition");
            record.detail.audio_outages.extend(gaps);
        }
        record.detail.recognition_completeness = if incomplete { recognition.completeness }
            else { crate::domain::RecognitionCompleteness::Complete };
        record.detail.transcription_stopped_at_seconds = stopped.then(|| outages::stopped_at(&recognition, record.detail.duration_seconds));
        record.detail.recognition_warnings = recognition.warnings;
        record.detail.summary.has_transcript = !record.detail.transcript.trim().is_empty();
        record.detail.summary.status = if stopped { "interrupted" } else { completed_status(&record.detail) }.into();
        if !incomplete && !record.title_user_edited {
            record.automatic_title_pending = true;
        }
        record.detail.error = if let Some(at) = record.detail.transcription_stopped_at_seconds {
            Some(format!("Transcription stopped at {}. Saved audio is available to transcribe again.", outages::clock(at)))
        } else { None };
        self.write(&record)?;
        Ok(record.detail)
    }

    fn finish_operation(
        &self,
        id: &str,
        token: &Arc<AtomicBool>,
        result: Result<String, String>,
        notes: bool,
        // The speech model that produced `result`; recorded with the transcript it wrote.
        model: Option<&str>,
    ) -> Result<MeetingDetail, String> {
        let _guard = self.store.lock()?;
        let mut operations = self
            .operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?;
        if !operations
            .get(id)
            .is_some_and(|current| Arc::ptr_eq(current, token))
        {
            return Err("The meeting operation is no longer current.".into());
        }
        operations.remove(id);
        let mut record = self.read(id)?;
        if token.load(Ordering::SeqCst) || record.detail.status == "cancelled" {
            return Err("Meeting operation cancelled.".into());
        }
        let text = match result.and_then(|text| {
            validate_text(&text, if notes { "Notes" } else { "Transcript" })?;
            if text.trim().is_empty() {
                return Err("The model returned no text.".into());
            }
            Ok(text)
        }) {
            Ok(text) => text,
            Err(error) => {
                record.detail.summary.status =
                    if !notes && record.detail.transcript.trim().is_empty() {
                        "failed"
                    } else {
                        completed_status(&record.detail)
                    }
                    .into();
                record.detail.error = Some(error.chars().take(2048).collect());
                self.write(&record)?;
                return Err(error);
            }
        };
        if notes {
            record.detail.notes = text;
        } else {
            let directory = self.store.meetings_dir().join(format!("{id}.asr"));
            record.detail.recognition_completeness =
                crate::domain::RecognitionCompleteness::Complete;
            record.detail.recognition_warnings.clear();
            record.detail.transcription_stopped_at_seconds = None;
            record.detail.cleaned_transcript = None;
            record.detail.audio_outages.retain(|gap| gap.source != "recognition");
            fs::create_dir_all(&directory)
                .map_err(|e| format!("Cannot create raw transcript history: {e}"))?;
            self.store.check_owned(&directory)?;
            self.store.write(
                &directory.join(format!("{}.json", new_id())),
                &serde_json::json!({"createdAt":now(),"raw":text}),
            )?;
            if record.raw_transcript.is_none() {
                record.raw_transcript = Some(text.clone());
            }
            record.detail.transcript = text;
            record.detail.transcription_model = model.map(Into::into);
            record.detail.summary.has_transcript = true;
            record.automatic_title_pending = !record.title_user_edited;
        }
        record.detail.summary.status = completed_status(&record.detail).into();
        record.detail.error = None;
        self.write(&record)?;
        Ok(record.detail)
    }
    pub fn begin_import(&self, source: &Path) -> Result<MeetingDetail, String> {
        let source = source
            .canonicalize()
            .map_err(|e| format!("Cannot open the selected media file: {e}"))?;
        let metadata = source
            .metadata()
            .map_err(|e| format!("Cannot inspect the selected media file: {e}"))?;
        if !metadata.is_file() || metadata.len() == 0 || metadata.len() > 32 * 1024 * 1024 * 1024 {
            return Err("Select a nonempty media file smaller than 32 GiB.".into());
        }
        let title = source
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .chars()
            .take(200)
            .collect::<String>();
        self.create(
            if title.trim().is_empty() {
                "Imported meeting"
            } else {
                &title
            },
            "import",
            "importing",
            Some(source),
            false,
        )
    }
    pub fn finish_import(&self, id: &str, ffmpeg: Option<&Path>) -> Result<MeetingDetail, String> {
        let (record, token) = self.begin(id, "importing")?;
        let result: Result<f64, String> = (|| {
            let source = record
                .source_media
                .as_ref()
                .ok_or("This meeting has no media source.")?;
            let destination = self.store.meetings_dir().join(format!("{id}.import.wav"));
            self.store.check_owned(&destination)?;
            let executable = resolve_ffmpeg(&self.store, ffmpeg)?;
            run_ffmpeg(&executable, source, &destination, &token)?;
            let duration = wav_duration(&destination)?;
            if duration <= 0.0 {
                return Err("The selected file contains no audio.".into());
            }
            let final_path = self.recording_paths(id)?.mixed;
            if final_path.exists() {
                return Err("This meeting already has saved audio.".into());
            }
            if token.load(Ordering::SeqCst) {
                return Err("Media import cancelled.".into());
            }
            fs::rename(&destination, &final_path)
                .map_err(|e| format!("Cannot save imported audio: {e}"))?;
            Ok(duration)
        })();
        let _guard = self.store.lock()?;
        self.operations
            .lock()
            .map_err(|_| "Meeting operation lock failed.")?
            .remove(id);
        let mut current = self.read(id)?;
        if token.load(Ordering::SeqCst) {
            return Err("Media import cancelled.".into());
        }
        match result {
            Ok(duration) => {
                current.detail.summary.duration_seconds = duration;
                current.detail.summary.has_audio = true;
                current.detail.summary.status = "ready".into();
                current.detail.error = None;
                self.write(&current)?;
                Ok(current.detail)
            }
            Err(error) => {
                current.detail.summary.status = "failed".into();
                current.detail.error = Some(error.chars().take(2048).collect());
                self.write(&current)?;
                Err(error)
            }
        }
    }
    pub fn import_media(
        &self,
        path: &Path,
        ffmpeg: Option<&Path>,
    ) -> Result<MeetingDetail, String> {
        let meeting = self.begin_import(path)?;
        self.finish_import(&meeting.id, ffmpeg)
    }
    pub fn recover_interrupted(&self) -> Result<usize, String> {
        let rows = self.list()?;
        let mut recovered = 0;
        for row in rows {
            if busy(&row.status) {
                let _guard = self.store.lock()?;
                let mut record = self.read(&row.id)?;
                let recording = matches!(record.detail.status.as_str(), "recording" | "finalizing");
                let duration = if recording {
                    wav_duration(&self.recording_paths(&row.id)?.mixed).unwrap_or(0.0)
                } else { 0.0 };
                if recording && duration > 0.0 {
                    record.detail.summary.status = "ready".into();
                    record.detail.summary.duration_seconds = duration;
                    record.detail.summary.has_audio = true;
                    record.detail.error = None;
                    record.detail.recording_warnings.push(
                        "Recording was interrupted; saved audio was recovered.".into(),
                    );
                } else {
                    record.detail.summary.status = "interrupted".into();
                    record.detail.error = Some("The previous operation was interrupted. Saved audio and completed text were preserved; retry to continue.".into());
                }
                self.write(&record)?;
                recovered += 1;
            }
        }
        Ok(recovered)
    }
    /// Copy the saved original recording bytes verbatim (no re-encode).
    ///
    /// Original is the primary saved meeting audio (`audio_for("mixed")`):
    /// native `{id}.wav` mixed stem (mic/system stems are not the original)
    /// or the finished import mix. Works while transcription/notes are
    /// processing as long as the saved recording is final; refuses only an
    /// active unfinished recording. Never modifies the record or the
    /// original file; streams so hour-long meetings stay bounded.
    pub fn export_audio(&self, id: &str, path: &Path) -> Result<u64, String> {
        let detail = self.get(id)?;
        if matches!(detail.status.as_str(), "recording" | "finalizing") {
            return Err("Stop the recording before exporting its audio.".into());
        }
        self.store.check_export_destination(path)?;
        let (source, protected): (PathBuf, Vec<PathBuf>) = {
            let _guard = self.store.lock()?;
            let record = self.read(id)?;
            if matches!(record.detail.status.as_str(), "recording" | "finalizing") {
                return Err("Stop the recording before exporting its audio.".into());
            }
            let source = self
                .audio_for(&record, "mixed")?
                .filter(|p| p.is_file())
                .ok_or_else(|| {
                    "This meeting has no saved audio for the selected source.".to_string()
                })?;
            let mut protected = vec![source.clone()];
            if let Ok(paths) = self.recording_paths(&record.detail.id) {
                protected.push(paths.mixed);
                protected.push(paths.microphone);
                protected.push(paths.system);
            }
            protected.push(
                self.store
                    .meetings_dir()
                    .join(format!("{}.import.wav", record.detail.id)),
            );
            for legacy in [
                &record.legacy_audio,
                &record.legacy_microphone_audio,
                &record.legacy_system_audio,
                &record.source_media,
            ]
            .into_iter()
            .flatten()
            {
                protected.push(legacy.clone());
            }
            (source, protected)
        };
        // The WAV label/dialog applies only when the stored source is a WAV
        // file (native 16 kHz mono PCM16 recordings and ffmpeg-normalised
        // imports). Legacy fallbacks carry unverified containers: refuse to
        // mislabel their bytes rather than writing them under a .wav name.
        if !source
            .extension()
            .is_some_and(|ext| ext.eq_ignore_ascii_case("wav"))
        {
            return Err(
                "The saved audio for this meeting is not in WAV format, so it cannot be exported as an original WAV recording.".into(),
            );
        }
        let source_len = source
            .metadata()
            .map_err(|e| format!("Cannot inspect the saved meeting audio: {e}"))?
            .len();
        if source_len == 0 {
            return Err("This meeting has no saved audio for the selected source.".into());
        }
        if path.exists() {
            path.canonicalize()
                .map_err(|e| format!("Cannot resolve export destination: {e}"))?;
            for original in &protected {
                if meeting_audio_export::is_same_file(original, path) {
                    return Err("The export destination is original meeting media. Choose a different filename.".into());
                }
            }
        }
        // Length is verified inside the copy from the opened source handle
        // against the temp file BEFORE the atomic replace; a failed
        // validation leaves any pre-existing destination untouched (never
        // deleted as cleanup), so the stale size above is only a fast
        // refusal, never the gate.
        meeting_audio_export::stream_copy_atomic(&source, path)
    }

    pub fn export(&self, id: &str, path: &Path, format: &str) -> Result<(), String> {
        self.export_version(id, path, format, None)
    }
    pub fn export_version(&self, id: &str, path: &Path, format: &str, transcript_version: Option<&str>) -> Result<(), String> {
        if meeting_audio_export::is_audio_format(format) {
            self.export_audio(id, path).map(|_| ())?;
            return Ok(());
        }
        let mut detail = self.get(id)?;
        match transcript_version.unwrap_or("recognized") {
            "recognized" => {},
            "cleaned" => detail.transcript = detail.cleaned_transcript.clone()
                .ok_or("Clean this meeting before exporting its cleaned transcript.")?,
            _ => return Err("Choose the cleaned or recognized transcript.".into()),
        }
        self.store.check_export_destination(path)?;
        let record = {
            let _guard = self.store.lock()?;
            self.read(id)?
        };
        if path.exists() {
            let target = path
                .canonicalize()
                .map_err(|e| format!("Cannot resolve export destination: {e}"))?;
            for protected in [
                &record.legacy_audio,
                &record.legacy_microphone_audio,
                &record.legacy_system_audio,
                &record.source_media,
            ]
            .into_iter()
            .flatten()
            {
                if protected.canonicalize().ok().as_ref() == Some(&target) {
                    return Err("The export destination is original meeting media. Choose a different filename.".into());
                }
            }
        }
        if format == "pdf" {
            atomic_write(path, &render_pdf(&detail)?)
        } else {
            let contents = render_export(&detail, format)?;
            atomic_write(path, contents.as_bytes())
        }
    }
}

fn validate_title(title: &str) -> Result<(), String> {
    if title.trim().is_empty() || title.len() > 1024 || title.chars().any(char::is_control) {
        return Err("Meeting titles must contain 1–1,024 bytes without control characters.".into());
    }
    Ok(())
}
fn protected_title() -> bool {
    true
}

#[cfg(test)]
mod meeting_tests {
    use super::*;

    fn recognition(text: &str, start: f32, end: f32) -> crate::asr::RecognitionResult {
        crate::asr::RecognitionResult {
            original_text: None,
            text: text.into(), completeness: crate::domain::RecognitionCompleteness::Complete,
            warnings: Vec::new(), segments: vec![crate::asr::Segment {
                id: 1, start_seconds: start, end_seconds: Some(end),
                speaker: None, text: text.into(), complete: true,
            }],
        }
    }

    fn turn(start: f64, end: f64, speaker: usize) -> crate::diarization::Turn {
        crate::diarization::Turn { start, end, speaker }
    }

    #[test]
    fn guarded_meeting_history_and_speaker_toggles_never_republish_original_loops() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = recorded(&meetings);
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        let mut raw = recognition("First useful words.", 0.0, 1.0);
        raw.segments[0].speaker = Some("Speaker 1".into());
        let mut second = recognition("Second useful words.", 2.0, 3.0).segments.remove(0);
        second.speaker = Some("Speaker 2".into());
        raw.segments.push(second);
        raw.text = "First useful words. Second useful words.".into();
        raw.original_text = Some("First useful words. go go go go Second useful words.".into());
        raw.completeness = crate::domain::RecognitionCompleteness::Incomplete;
        raw.warnings.push("Loop omitted.".into());
        let labeled = publish(&raw, true);
        let plain = raw.text.clone();
        let result = MeetingRecognition { published: labeled.clone(), raw };
        let saved = meetings.finish_recognition_operation(&job.id, &job.cancel, Ok(result), &job.asr_model).unwrap();
        assert_eq!(saved.transcript, labeled);
        let record = meetings.read(&detail.id).unwrap();
        let path = meetings.store.meetings_dir().join(format!("{}.asr", detail.id))
            .join(record.recognition_file.unwrap());
        let history: Value = meetings.store.read(&path).unwrap();
        assert_eq!(history["raw"], "First useful words. go go go go Second useful words.");
        assert_eq!(history["guarded"], plain);
        assert!(!history["segments"].to_string().contains("go go"));
        assert_eq!(meetings.set_speaker_labels(&detail.id, false, false).unwrap().transcript, plain);
        assert_eq!(meetings.set_speaker_labels(&detail.id, true, false).unwrap().transcript, labeled);
        // Explicit edits still require confirmation even with retained original text.
        meetings.save(&detail.id, &saved.title, "Edited.", "").unwrap();
        assert_eq!(meetings.set_speaker_labels(&detail.id, false, false).unwrap_err(), SPEAKER_LABELS_REPLACE_EDITS);
        assert_eq!(meetings.set_speaker_labels(&detail.id, false, true).unwrap().transcript, plain);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn wholly_dropped_meeting_stems_keep_raw_history_and_protect_prior_transcript() {
        let _gate = crate::resource_manager::tests::test_gate();
        for prior in ["", "Existing edited transcript."] {
            let meetings = library();
            let detail = recorded(&meetings);
            if !prior.is_empty() { meetings.save(&detail.id, &detail.title, prior, "").unwrap(); }
            let job = meetings.begin_transcription(&detail.id, None).unwrap();
            let dropped = || crate::sherpa::llama_test_recognition(("go go go go".into(), Some(8), "stop".into()));
            let result = merge_stems(dropped(), dropped());
            assert!(result.published.is_empty());
            assert!(result.raw.segments.is_empty());
            assert_eq!(result.raw.original_text(), "go go go go go go go go");
            let saved = meetings.finish_recognition_operation(&job.id, &job.cancel, Ok(result), &job.asr_model).unwrap();
            assert_eq!(saved.transcript, prior);
            assert_eq!(saved.status, if prior.is_empty() { "ready" } else { "transcribed" });
            assert!(saved.error.is_none());
            assert!(saved.transcription_stopped_at_seconds.is_none());
            let path = fs::read_dir(meetings.store.meetings_dir().join(format!("{}.asr", detail.id))).unwrap()
                .next().unwrap().unwrap().path();
            let history: Value = meetings.store.read(&path).unwrap();
            assert_eq!(history["raw"], "go go go go go go go go");
            assert_eq!(history["guarded"], "");
            assert!(meetings.set_speaker_labels(&detail.id, false, true).is_err());
            assert_eq!(meetings.get(&detail.id).unwrap().transcript, prior);
            fs::remove_dir_all(meetings.store.root()).unwrap();
        }
    }

    #[test]
    fn merging_guarded_and_clean_stems_retains_both_originals() {
        let dropped = crate::sherpa::llama_test_recognition(("go go go go".into(), Some(8), "stop".into()));
        for (mic, other, expected) in [
            (dropped.clone(), recognition("Other words.", 2.0, 3.0), "go go go go Other words."),
            (recognition("My words.", 1.0, 2.0), dropped, "My words. go go go go"),
        ] {
            let result = merge_stems(mic, other);
            assert_eq!(result.raw.original_text(), expected);
            assert!(!result.published.contains("go go"));
            assert!(result.raw.segments.iter().all(|s| !s.text.contains("go go")));
        }
    }

    #[test]
    fn published_text_collapses_padded_spaces_and_keeps_the_raw_text() {
        let mut raw = recognition(" Thanks for joining.  I wanted to go over\tthe plan. ", 0.0, 3.0);
        raw.segments.push(recognition(" Sure,  I looked.", 5.0, 7.0).segments.remove(0));
        assert_eq!(publish(&raw, false), "Thanks for joining. I wanted to go over the plan.");
        assert_eq!(raw.text, " Thanks for joining.  I wanted to go over\tthe plan. ");
        raw.segments[0].speaker = Some("Speaker 1".into());
        raw.segments[1].speaker = Some("Speaker 2".into());
        assert_eq!(publish(&raw, true),
            "[0:00] Speaker 1: Thanks for joining. I wanted to go over the plan.\n\n[0:05] Speaker 2: Sure, I looked.");
        assert_eq!(tidy_spaces("One.  Two.\r\n\r\n Three. "), "One. Two.\n\nThree.");
    }

    #[test]
    fn labels_use_overlap_nearest_previous_and_arrival_order() {
        let mut raw = recognition("First.", 2.0, 4.0);
        raw.segments.extend([
            recognition("Most overlap.", 5.0, 8.0).segments.remove(0),
            recognition("Tie.", 10.0, 12.0).segments.remove(0),
            recognition("Nearby.", 15.0, 15.0).segments.remove(0),
            recognition("Carry forward.", 20.0, 21.0).segments.remove(0),
        ]);
        raw.segments[3].end_seconds = None;
        let turns = [turn(2.0, 4.0, 6), turn(5.0, 6.0, 6), turn(5.0, 8.0, 4),
            turn(10.0, 12.0, 4), turn(10.0, 12.0, 2), turn(15.5, 16.0, 6)];
        assert!(assign_speakers(&mut raw, &turns, false));
        assert_eq!(raw.segments.iter().map(|s| s.speaker.as_deref().unwrap()).collect::<Vec<_>>(),
            ["Speaker 1", "Speaker 2", "Speaker 3", "Speaker 1", "Speaker 1"]);
        assert_eq!(raw.text, "First.", "Labeling must preserve the raw recognition text");

        let mut far = recognition("No nearby turn.", 0.0, 1.0);
        assert!(!assign_speakers(&mut far, &[turn(10.0, 12.0, 6)], false));
        assert_eq!(far.segments[0].speaker.as_deref(), Some("Other speakers"));
        // Mixed audio with one voice stays unlabeled; two voices are numbered.
        assert!(!assign_speakers(&mut far, &[turn(10.0, 12.0, 6)], true));
        assert_eq!(far.segments[0].speaker, None);
        far.segments.push(recognition("Second voice.", 20.0, 21.0).segments.remove(0));
        assert!(assign_speakers(&mut far, &[turn(10.0, 12.0, 6), turn(20.0, 21.0, 2)], true));
        assert_eq!(far.segments.iter().map(|s| s.speaker.as_deref().unwrap()).collect::<Vec<_>>(),
            ["Speaker 1", "Speaker 2"]);

        let mut point = recognition("Point.", 5.0, 5.0);
        assert!(assign_speakers(&mut point, &[turn(4.0, 4.5, 5), turn(5.5, 6.0, 1)], false));
        // A following segment reveals which equal-distance speaker won.
        point.segments.push(recognition("Lower index.", 5.5, 6.0).segments.remove(0));
        assign_speakers(&mut point, &[turn(4.0, 4.5, 5), turn(5.5, 6.0, 1)], false);
        assert_eq!(point.segments[0].speaker, point.segments[1].speaker);
    }

    #[test]
    fn diarization_labels_only_the_system_stem_and_persists_raw_speakers() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = stems(&meetings, "both", Some(8192));
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
        job.diarization_root = Some(PathBuf::from("snapshotted-models"));
        let mut calls = 0;
        let result = job.recognize_audio_with(|path, _, _| {
            Ok(if path == paths.microphone { recognition("Mine.", 2.0, 3.0) }
                else { theirs() })
        }, |root, audio, _| {
            assert_eq!(root, Path::new("snapshotted-models"));
            assert_eq!(audio, paths.system);
            calls += 1;
            Ok(vec![turn(1.0, 2.0, 5), turn(4.0, 5.0, 2)])
        }).unwrap();
        assert_eq!(calls, 1);
        assert_eq!(result.published,
            "[0:01] Speaker 1: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Speaker 2: Reply.");
        let saved = meetings.finish_recognition_operation(&job.id, &job.cancel, Ok(result), &job.asr_model).unwrap();
        assert_eq!(saved.recognition_warnings, Vec::<String>::new());
        assert_eq!((saved.speaker_count, saved.speaker_labels), (2, true));
        let history = fs::read_dir(meetings.store.meetings_dir().join(format!("{}.asr", detail.id))).unwrap()
            .next().unwrap().unwrap().path();
        let raw: Value = meetings.store.read(&history).unwrap();
        assert_eq!(raw["raw"], "Theirs. Mine. Reply.");
        assert_eq!(raw["segments"][0]["speaker"], "Speaker 1");
        assert_eq!(raw["segments"][1]["speaker"], "You");
        assert_eq!(raw["segments"][2]["speaker"], "Speaker 2");
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    fn theirs() -> crate::asr::RecognitionResult {
        let mut raw = recognition("Theirs.", 1.0, 2.0);
        raw.segments.push(recognition("Reply.", 4.0, 5.0).segments.remove(0));
        raw.text = "Theirs. Reply.".into();
        raw
    }

    #[test]
    fn one_system_voice_stays_other_speakers() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = stems(&meetings, "both", Some(8192));
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
        job.diarization_root = Some(PathBuf::from("models"));
        let result = job.recognize_audio_with(|path, _, _| {
            Ok(if path == paths.microphone { recognition("Mine.", 2.0, 3.0) } else { theirs() })
        }, |_, _, _| Ok(vec![turn(1.0, 5.0, 0)])).unwrap();
        assert_eq!(result.published,
            "[0:01] Other speakers: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Other speakers: Reply.");
        let saved = meetings.finish_recognition_operation(&job.id, &job.cancel, Ok(result), &job.asr_model).unwrap();
        assert_eq!(saved.speaker_count, 1);
        assert!(meetings.set_speaker_labels(&detail.id, false, false).is_err());
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn meeting_speaker_labels_switch_protect_edits_and_survive_transcribing_again() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = stems(&meetings, "both", Some(8192));
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let labeled = "[0:01] Speaker 1: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Speaker 2: Reply.";
        let sides = "[0:01] Other speakers: Theirs.\n\n[0:02] You: Mine.\n\n[0:04] Other speakers: Reply.";
        let transcribe = || {
            let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
            job.diarization_root = Some(PathBuf::from("models"));
            let result = job.recognize_audio_with(|path, _, _| {
                Ok(if path == paths.microphone { recognition("Mine.", 2.0, 3.0) } else { theirs() })
            }, |_, _, _| Ok(vec![turn(1.0, 2.0, 5), turn(4.0, 5.0, 2)])).unwrap();
            meetings.finish_recognition_operation(&job.id, &job.cancel, Ok(result), &job.asr_model).unwrap()
        };
        assert_eq!(transcribe().transcript, labeled);
        let off = meetings.set_speaker_labels(&detail.id, false, false).unwrap();
        assert_eq!((off.transcript.as_str(), off.speaker_labels), (sides, false));
        // Transcribing again keeps this meeting's choice.
        assert_eq!(transcribe().transcript, sides);
        meetings.save(&detail.id, &off.title, &format!("{sides}\n\nEdited."), "").unwrap();
        assert_eq!(meetings.set_speaker_labels(&detail.id, true, false).unwrap_err(),
            SPEAKER_LABELS_REPLACE_EDITS);
        assert!(meetings.get(&detail.id).unwrap().transcript.ends_with("Edited."));
        let on = meetings.set_speaker_labels(&detail.id, true, true).unwrap();
        assert_eq!((on.transcript.as_str(), on.speaker_labels), (labeled, true));
        meetings.save_with_cleaned(&detail.id, &on.title, labeled, "", Some("Cleaned edits.")).unwrap();
        assert_eq!(meetings.set_speaker_labels(&detail.id, false, false).unwrap_err(),
            SPEAKER_LABELS_REPLACE_EDITS);
        assert_eq!(meetings.get(&detail.id).unwrap().cleaned_transcript.as_deref(), Some("Cleaned edits."));
        let rebuilt = meetings.set_speaker_labels(&detail.id, false, true).unwrap();
        assert_eq!(rebuilt.transcript, sides);
        assert!(rebuilt.cleaned_transcript.is_none());
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn mixed_and_fallback_paths_share_passages_and_keep_plain_text_on_empty_turns() {
        let _gate = crate::resource_manager::tests::test_gate();
        for (source, system) in [("microphone", Some(8192)), ("import", Some(8192)),
            ("both", None), ("both", Some(0))] {
            let meetings = library();
            let detail = stems(&meetings, source, system);
            let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
            job.diarization_root = Some(PathBuf::from("models"));
            for turns in [vec![turn(0.0, 1.0, 3), turn(3.0, 4.0, 1)], vec![turn(0.0, 4.0, 3)], vec![]] {
                let expect_labels = turns.len() == 2;
                let result = job.recognize_audio_with(|audio, _, _| {
                    assert_eq!(audio, job.audio);
                    let mut raw = recognition("Raw words.", 0.0, 1.0);
                    raw.segments.push(recognition("Reply.", 3.0, 4.0).segments.remove(0));
                    raw.text = "Raw words. Reply.".into();
                    Ok(raw)
                }, |_, audio, _| { assert_eq!(audio, job.audio); Ok(turns.clone()) }).unwrap();
                assert_eq!(result.raw.text, "Raw words. Reply.");
                assert_eq!(result.published, if expect_labels {
                    "[0:00] Speaker 1: Raw words.\n\n[0:03] Speaker 2: Reply."
                } else { "Raw words. Reply." });
                assert!(expect_labels || result.raw.segments.iter().all(|s| s.speaker.is_none()));
            }
            fs::remove_dir_all(meetings.store.root()).unwrap();
        }
    }

    #[test]
    fn diarization_failures_preserve_existing_output_and_cancellation_propagates() {
        let _gate = crate::resource_manager::tests::test_gate();
        for both in [false, true] {
            let meetings = library();
            let detail = stems(&meetings, if both { "both" } else { "import" }, Some(8192));
            let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
            job.diarization_root = Some(PathBuf::from("models"));
            let result = job.recognize_audio_with(|_, _, _| Ok(recognition("Keep words.", 0.0, 1.0)),
                |_, _, _| Err("Worker failed.".into())).unwrap();
            assert_eq!(result.published, if both { "[0:00] You: Keep words.\n\n[0:00] Other speakers: Keep words." } else { "Keep words." });
            assert_eq!(result.raw.warnings, ["Speaker labeling failed: Worker failed."]);
            let canceled = job.recognize_audio_with(|_, _, _| Ok(recognition("Do not publish.", 0.0, 1.0)),
                |_, _, cancel| { cancel.store(true, Ordering::Release); Ok(vec![turn(0.0, 1.0, 0)]) });
            assert!(canceled.is_err());
            fs::remove_dir_all(meetings.store.root()).unwrap();
        }
    }

    #[test]
    fn absent_diarization_snapshot_never_runs_worker_and_asr_drops_before_labeling() {
        let _gate = crate::resource_manager::tests::test_gate();
        struct Probe(Arc<AtomicBool>);
        impl Drop for Probe { fn drop(&mut self) { self.0.store(true, Ordering::Release); } }
        let meetings = library();
        let detail = recorded(&meetings);
        let mut settings = meetings.store.load_settings().unwrap();
        settings.meeting_speaker_labels = false;
        meetings.store.save_settings(&settings).unwrap();
        let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
        settings.meeting_speaker_labels = true;
        meetings.store.save_settings(&settings).unwrap();
        assert!(job.diarization_root.is_none());
        job.recognize_audio_with(|_, _, _| Ok(recognition("Words.", 0.0, 1.0)),
            |_, _, _| panic!("A settings edit must not enable an existing job")).unwrap();
        job.diarization_root = Some(PathBuf::from("models"));
        let dropped = Arc::new(AtomicBool::new(false));
        let probe = Probe(dropped.clone());
        job.recognize_audio_with(move |_, _, _| {
            let _retain = &probe;
            Ok(recognition("Words.", 0.0, 1.0))
        }, |_, _, _| { assert!(dropped.load(Ordering::Acquire)); Ok(vec![]) }).unwrap();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    fn stems(meetings: &Meetings, source: &str, system_value: Option<i16>) -> MeetingDetail {
        let detail = recorded(meetings);
        let mut record = meetings.read(&detail.id).unwrap();
        record.detail.source = source.into();
        meetings.write(&record).unwrap();
        let paths = meetings.recording_paths(&detail.id).unwrap();
        for (path, value) in [(paths.microphone, Some(8192i16)), (paths.system, system_value)] {
            if let Some(value) = value {
                let mut wav = hound::WavWriter::create(path, hound::WavSpec {
                    channels: 1, sample_rate: 16000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int,
                }).unwrap();
                for _ in 0..160 { wav.write_sample(value).unwrap(); }
                wav.finalize().unwrap();
            }
        }
        meetings.get(&detail.id).unwrap()
    }

    #[test]
    fn stem_paragraphs_sort_join_and_split_on_speaker_or_two_second_pause() {
        let mut mic = recognition("First", 12.0, 14.0);
        mic.segments.extend([
            recognition("second.", 15.0, 17.0).segments.remove(0),
            recognition("After pause.", 19.0, 20.0).segments.remove(0),
            recognition("Hour.", 3661.0, 3662.0).segments.remove(0),
        ]);
        let merged = merge_stems(mic, recognition("Reply.", 18.0, 19.0));
        assert_eq!(merged.published, "[0:12] You: First second.\n\n[0:18] Other speakers: Reply.\n\n[0:19] You: After pause.\n\n[1:01:01] You: Hour.");
        assert_eq!(merged.raw.text, "First second. Reply. After pause. Hour.");
        let mut same = recognition("Before.", 0.0, 5.0);
        same.segments.push(recognition("After.", 7.0, 8.0).segments.remove(0));
        assert_eq!(merge_stems(same, empty_recognition()).published, "[0:00] You: Before.\n\n[0:07] You: After.");
        assert_eq!(merge_stems(recognition("Mic", 1.0, 2.0), recognition("System", 1.0, 2.0)).published,
            "[0:01] You: Mic\n\n[0:01] Other speakers: System");
    }

    #[test]
    fn separate_stems_share_one_lease_and_preserve_raw_segments() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = stems(&meetings, "both", Some(8192));
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let mut calls = Vec::new();
        let result = meetings.begin_transcription(&detail.id, Some("moonshine-small")).unwrap()
            .run_recognition(|path, model, _| {
                assert_eq!(model, "moonshine-small");
                assert!(crate::resource_manager::acquire_asr("moonshine-medium").is_err());
                calls.push(path.to_owned());
                Ok(if path == paths.microphone { recognition("Mine.", 2.0, 4.0) }
                    else { recognition("Theirs.", 1.0, 2.0) })
            }).unwrap();
        assert_eq!(calls, vec![paths.microphone, paths.system]);
        assert_eq!(result.transcript, "[0:01] Other speakers: Theirs.\n\n[0:02] You: Mine.");
        assert_eq!(meetings.raw_transcript(&detail.id).unwrap().as_deref(), Some("Theirs. Mine."));
        let history = fs::read_dir(meetings.store.meetings_dir().join(format!("{}.asr", detail.id))).unwrap()
            .next().unwrap().unwrap().path();
        let raw: Value = meetings.store.read(&history).unwrap();
        assert_eq!(raw["segments"][0]["start_seconds"], 1.0);
        assert_eq!(raw["segments"][0]["speaker"], "Other speakers");
        assert_eq!(raw["raw"], "Theirs. Mine.");
        crate::resource_manager::acquire_asr("moonshine-small").unwrap();
        crate::resource_manager::notify_asr_finished();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn recognizer_callback_drops_before_the_asr_lease_is_released() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        struct Probe(Arc<AtomicBool>);
        impl Drop for Probe {
            fn drop(&mut self) {
                let held = crate::resource_manager::acquire_asr("moonshine-small").is_err();
                self.0.store(held, Ordering::Release);
                if !held { crate::resource_manager::notify_asr_finished(); }
            }
        }
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recorded(&meetings);
        let held_on_drop = Arc::new(AtomicBool::new(false));
        let probe = Probe(held_on_drop.clone());
        meetings.begin_transcription(&detail.id, None).unwrap().run(move |_, _, _| {
            let _keep_alive = &probe;
            Ok("Complete words.".into())
        }).unwrap();
        assert!(held_on_drop.load(Ordering::Acquire));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn single_source_import_missing_and_silent_stems_use_plain_mixed_audio() {
        let _gate = crate::resource_manager::tests::test_gate();
        let _memory = HermeticMemory::acquire();
        for (source, system) in [("microphone", Some(8192)), ("system", Some(8192)),
            ("import", Some(8192)), ("both", None), ("both", Some(0))] {
            let meetings = library();
            let detail = stems(&meetings, source, system);
            let mixed = meetings.recording_paths(&detail.id).unwrap().mixed;
            let mut calls = 0;
            let result = meetings.begin_transcription(&detail.id, None).unwrap().run_recognition(|path, _, _| {
                assert_eq!(path, mixed);
                calls += 1;
                Ok(recognition("Plain words.", 0.0, 1.0))
            }).unwrap();
            assert_eq!(calls, 1);
            assert_eq!(result.transcript, "Plain words.");
            fs::remove_dir_all(meetings.store.root()).unwrap();
        }
    }

    #[test]
    fn empty_stem_falls_back_but_incomplete_stem_preserves_the_edited_transcript() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = stems(&meetings, "legacy", Some(8192));
        let paths = meetings.recording_paths(&detail.id).unwrap();
        let mut calls = 0;
        let result = meetings.begin_transcription(&detail.id, None).unwrap().run_recognition(|path, _, _| {
            calls += 1;
            Ok(if path == paths.system { empty_recognition() } else { recognition("Mixed words.", 0.0, 1.0) })
        }).unwrap();
        assert_eq!(calls, 3);
        assert_eq!(result.transcript, "Mixed words.");
        meetings.save(&detail.id, "Title", "Edited words.", "").unwrap();
        for fail_first in [true, false] {
            let mut calls = 0;
            let result = meetings.begin_transcription(&detail.id, None).unwrap().run_recognition(|path, _, _| {
                assert_ne!(path, paths.mixed);
                calls += 1;
                let mut result = recognition("Available words.", 0.0, 1.0);
                if (path == paths.microphone) == fail_first {
                    result.completeness = crate::domain::RecognitionCompleteness::Incomplete;
                    result.warnings.push("Needs review.".into());
                }
                Ok(result)
            }).unwrap();
            assert_eq!(calls, 2);
            assert_eq!(result.transcript, "Edited words.");
            assert_eq!(result.status, "interrupted");
            assert_eq!(result.recognition_warnings, vec!["Needs review."]);
            assert!(meetings.generate_notes(&detail.id, "", |_, _, _| panic!("incomplete")).is_err());
        }
        assert_eq!(meetings.raw_transcript(&detail.id).unwrap().as_deref(), Some("Mixed words."));
        let partial = meetings.begin_transcription(&detail.id, None).unwrap().run_recognition(|path, _, _| {
            if path == paths.system { Err("Second stem failed.".into()) }
            else { Ok(recognition("Keep this partial attempt.", 3.0, 4.0)) }
        }).unwrap();
        assert_eq!(partial.status, "interrupted");
        assert_eq!(partial.transcript, "Edited words.");
        assert_eq!(partial.recognition_warnings, vec!["Second stem failed."]);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn stem_cancellation_cleans_token_and_remote_run_does_not_touch_local_lease() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = stems(&meetings, "both", Some(8192));
        let mut calls = 0;
        let result = meetings.begin_transcription(&detail.id, None).unwrap().run_recognition(|_, _, _| {
            calls += 1;
            meetings.cancel(&detail.id).unwrap();
            Ok(recognition("Do not save.", 0.0, 1.0))
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
        assert!(meetings.get(&detail.id).unwrap().transcript.is_empty());
        assert!(!meetings.operations.lock().unwrap().contains_key(&detail.id));
        crate::resource_manager::acquire_asr("moonshine-small").unwrap();
        crate::resource_manager::note_asr_loaded("moonshine-small");
        let mut job = meetings.begin_transcription(&detail.id, None).unwrap();
        job.asr_model = "openrouter:provider/audio".into();
        // Even an accidental local-label snapshot must not run on the remote route.
        job.diarization_root = Some(PathBuf::from("must-not-load"));
        let remote = job.run_remote(|_, model, _| {
            assert_eq!(model, "openrouter:provider/audio");
            assert!(!crate::resource_manager::asr_release_pending());
            Ok(recognition("Remote words.", 0.0, 1.0))
        }).unwrap();
        assert_eq!(remote.transcription_model.as_deref(), Some("openrouter:provider/audio"));
        assert!(crate::resource_manager::acquire_asr("moonshine-small").is_err());
        crate::resource_manager::notify_asr_finished();
        crate::resource_manager::confirm_asr_released();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn unconfigured_remote_fails_before_claiming_the_meeting() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = recorded(&meetings);
        assert!(meetings.begin_transcription(&detail.id, Some("openrouter")).is_err());
        assert_ne!(meetings.get(&detail.id).unwrap().status, "transcribing");
        assert!(!meetings.operations.lock().unwrap().contains_key(&detail.id));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn partial_recognition_is_preserved_and_blocks_generation_until_retry() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recorded(&meetings);
        let partial = meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run_recognition(|_, _, _| {
                Ok(crate::asr::RecognitionResult {
                    original_text: None,
                    segments: Vec::new(),
                    text: "Acknowledged words.".into(),
                    completeness: crate::domain::RecognitionCompleteness::Incomplete,
                    warnings: vec!["Worker stopped.".into()],
                })
            })
            .unwrap();
        assert_eq!(partial.transcript, "Acknowledged words.");
        assert_eq!(
            partial.recognition_completeness,
            crate::domain::RecognitionCompleteness::Incomplete
        );
        assert!(partial.has_audio);
        assert!(meetings
            .generate_notes(&detail.id, "", |_, _, _| panic!("must not run"))
            .is_err());
        let complete = meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, _, _| Ok("Acknowledged words and the recovered tail.".into()))
            .unwrap();
        assert_eq!(
            complete.recognition_completeness,
            crate::domain::RecognitionCompleteness::Complete
        );
        assert_eq!(
            meetings.raw_transcript(&detail.id).unwrap().as_deref(),
            Some(complete.transcript.as_str())
        );
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn collapse_only_completes_and_keeps_original_warning_history() {
        let meetings = library();
        let detail = recorded(&meetings);
        let (_, token) = meetings.begin(&detail.id, "transcribing").unwrap();
        let mut raw = recognition("Useful reduced text.", 0.0, 0.01);
        raw.completeness = crate::domain::RecognitionCompleteness::Incomplete;
        raw.segments[0].complete = false;
        raw.original_text = Some("Useful useful useful useful reduced text.".into());
        raw.warnings = vec!["Qwen3-ASR returned a repeating loop. Repeated text was reduced to its first occurrence; review the recording.".into()];
        raw.warnings.push("Speaker labeling failed: worker unavailable".into());
        let saved = meetings.finish_recognition_operation(&detail.id, &token,
            Ok(MeetingRecognition { published: raw.text.clone(), raw }), "qwen").unwrap();
        assert_eq!(saved.status, "transcribed");
        assert_eq!(saved.recognition_completeness, crate::domain::RecognitionCompleteness::Complete);
        assert!(saved.error.is_none());
        assert!(saved.audio_outages.is_empty());
        let record = meetings.read(&detail.id).unwrap();
        let path = meetings.store.meetings_dir().join(format!("{}.asr", detail.id)).join(record.recognition_file.unwrap());
        let history: Value = meetings.store.read(&path).unwrap();
        assert_eq!(history["completeness"], "incomplete");
        assert_eq!(history["warnings"][0], saved.recognition_warnings[0]);
        assert_eq!(history["warnings"][1], saved.recognition_warnings[1]);
        assert_eq!(meetings.raw_transcript(&detail.id).unwrap().as_deref(), Some("Useful useful useful useful reduced text."));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn omission_markers_and_true_stops_have_different_states() {
        for (warning, stopped) in [
            ("Qwen3-ASR reached its output token limit. Review the recording for missing speech.", false),
            ("Qwen3-ASR returned a repeating loop. This section was omitted; review the recording for missing speech.", false),
            ("The speech worker stopped unexpectedly.", true),
        ] {
            let meetings = library();
            let detail = meetings.create("Meeting", "import", "ready", None, true).unwrap();
            let mut record = meetings.read(&detail.id).unwrap();
            record.detail.summary.duration_seconds = 120.0;
            meetings.write(&record).unwrap();
            let (_, token) = meetings.begin(&detail.id, "transcribing").unwrap();
            let mut raw = recognition("Available text.", 20.0, 30.0);
            raw.completeness = crate::domain::RecognitionCompleteness::Incomplete;
            raw.segments[0].complete = false;
            raw.warnings.push(warning.into());
            let saved = meetings.finish_recognition_operation(&detail.id, &token,
                Ok(MeetingRecognition { published: raw.text.clone(), raw }), "qwen").unwrap();
            assert_eq!(saved.status, if stopped { "interrupted" } else { "transcribed" });
            assert_eq!(saved.transcription_stopped_at_seconds, stopped.then_some(30.0));
            assert_eq!(saved.missing_seconds, 0.0);
            if stopped {
                assert!(saved.error.unwrap().contains("Transcription stopped at 0:30"));
                assert!(saved.audio_outages.is_empty());
            } else {
                assert!(saved.error.is_none());
                assert_eq!(saved.audio_outages.len(), 1);
                assert_eq!(saved.audio_outages[0].source, "recognition");
                assert_eq!(saved.audio_outages[0].start_seconds, 20.0);
            }
            fs::remove_dir_all(meetings.store.root()).unwrap();
        }
    }

    #[test]
    fn limited_retry_reports_its_time_and_preserves_both_edited_versions() {
        let meetings = library();
        let detail = meetings.create("Meeting", "import", "ready", None, true).unwrap();
        let mut record = meetings.read(&detail.id).unwrap();
        record.detail.summary.duration_seconds = 120.0;
        meetings.write(&record).unwrap();
        meetings.save_with_cleaned(&detail.id, "Meeting", "Original edits.", "Notes.", Some("Cleaned edits.")).unwrap();
        let (_, token) = meetings.begin(&detail.id, "transcribing").unwrap();
        let mut raw = recognition("Partial retry.", 20.0, 30.0);
        raw.completeness = crate::domain::RecognitionCompleteness::Incomplete;
        raw.segments[0].complete = false;
        raw.warnings.push("Qwen3-ASR reached its output token limit.".into());
        let saved = meetings.finish_recognition_operation(&detail.id, &token,
            Ok(MeetingRecognition { published: raw.text.clone(), raw }), "qwen").unwrap();
        assert_eq!(saved.transcript, "Original edits.");
        assert_eq!(saved.cleaned_transcript.as_deref(), Some("Cleaned edits."));
        assert_eq!(saved.notes, "Notes.");
        assert!(saved.error.is_none());
        assert_eq!(saved.audio_outages[0].start_seconds, 20.0);
        assert!(saved.audio_outages[0].reason.contains("previous transcript kept"));
        assert_eq!(meetings.get(&detail.id).unwrap().audio_outages[0].start_seconds, 20.0);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn old_collapse_only_records_read_as_completed_without_rewriting_history() {
        let meetings = library();
        let detail = recorded(&meetings);
        let mut record = meetings.read(&detail.id).unwrap();
        record.detail.transcript = "Saved reduced words.".into();
        record.detail.recognition_completeness = crate::domain::RecognitionCompleteness::Incomplete;
        record.detail.recognition_warnings = vec!["Repeated text was reduced to its first occurrence".into()];
        record.detail.summary.status = "interrupted".into();
        record.detail.error = Some("Recognition was interrupted.".into());
        meetings.write(&record).unwrap();
        let before = fs::read(meetings.path(&detail.id).unwrap()).unwrap();
        let saved = meetings.get(&detail.id).unwrap();
        assert_eq!(saved.status, "transcribed");
        assert!(saved.error.is_none());
        assert_eq!(fs::read(meetings.path(&detail.id).unwrap()).unwrap(), before);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn cleaned_copy_edits_cancellation_and_failures_preserve_originals() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = recorded(&meetings);
        let mut record = meetings.read(&detail.id).unwrap();
        record.raw_transcript = Some("Raw history.".into());
        record.detail.transcript = "[0:00] You: um useful words".into();
        record.detail.notes = "Existing notes.".into();
        meetings.write(&record).unwrap();
        let cleaned = meetings.clean_for_test(&detail.id, |text, _| {
            assert_eq!(meetings.get(&detail.id).unwrap().status, "processingNotes");
            assert!(meetings.save(&detail.id, "No", "No", "No").is_err());
            export::map_passages(text, |passage| {
                assert_eq!(passage, "um useful words");
                Ok("Useful words.".into())
            })
        }).unwrap();
        assert_eq!(cleaned.cleaned_transcript.as_deref(), Some("[0:00] You: Useful words."));
        assert_eq!(cleaned.transcript, record.detail.transcript);
        assert_eq!(cleaned.notes, "Existing notes.");
        let saved = meetings.save_with_cleaned(&detail.id, &cleaned.title, &cleaned.transcript,
            &cleaned.notes, Some("Edited cleaned copy.")).unwrap();
        let saved = meetings.save(&detail.id, &saved.title, "Edited recognized copy.", &saved.notes).unwrap();
        assert_eq!(saved.cleaned_transcript.as_deref(), Some("Edited cleaned copy."));
        assert!(meetings.clean_for_test(&detail.id, |_, _| Err("Unavailable".into())).is_err());
        assert!(meetings.clean_for_test(&detail.id, |_, cancel| {
            meetings.cancel(&detail.id).unwrap();
            assert!(cancel.load(Ordering::Acquire));
            Ok("Discard late output.".into())
        }).is_err());
        assert_eq!(meetings.get(&detail.id).unwrap().cleaned_transcript, saved.cleaned_transcript);
        assert_eq!(meetings.get(&detail.id).unwrap().transcript, "Edited recognized copy.");
        assert_eq!(meetings.raw_transcript(&detail.id).unwrap().as_deref(), Some("Raw history."));
        assert!(!meetings.operations.lock().unwrap().contains_key(&detail.id));
        let reopened = Meetings::new(Store::open(Some(meetings.store.root().to_path_buf())).unwrap());
        assert_eq!(reopened.get(&detail.id).unwrap().cleaned_transcript, saved.cleaned_transcript);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn selected_export_keeps_gaps_outside_both_editable_versions() {
        let meetings = library();
        let detail = recorded(&meetings);
        let mut record = meetings.read(&detail.id).unwrap();
        record.detail.transcript = "[0:00] You: Recognized wording.\n\n[0:30] You: Tail.".into();
        record.detail.cleaned_transcript = Some("[0:00] You: Cleaned wording.\n\n[0:30] You: Tail.".into());
        record.detail.audio_outages = vec![crate::domain::AudioOutage {
            source: "microphone".into(), start_seconds: 10.0, end_seconds: 24.0,
            reason: "Microphone disconnected".into(),
        }];
        record.detail.summary.duration_seconds = 40.0;
        record.outages_recorded = true;
        meetings.write(&record).unwrap();
        let directory = export_dir();
        let path = directory.join("meeting.txt");
        meetings.export_version(&detail.id, &path, "text", Some("cleaned")).unwrap();
        let exported = fs::read_to_string(&path).unwrap();
        assert!(exported.contains("Cleaned wording."));
        assert!(!exported.contains("Recognized wording."));
        assert!(exported.find("[Audio missing 0:10–0:24]").unwrap() < exported.find("Tail.").unwrap());
        meetings.export(&detail.id, &path, "text").unwrap();
        assert!(fs::read_to_string(&path).unwrap().contains("Recognized wording."));
        assert!(meetings.export_version(&detail.id, &path, "text", Some("invalid")).is_err());
        let saved = meetings.get(&detail.id).unwrap();
        assert!(!saved.transcript.contains("Audio missing"));
        assert!(!saved.cleaned_transcript.unwrap().contains("Audio missing"));
        fs::remove_dir_all(directory).unwrap();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn meeting_revision_changes_on_writes_not_reads() {
        let meetings = library();
        assert_eq!(meetings.revision(), 0);
        let detail = recorded(&meetings);
        let revision = meetings.revision();
        assert!(revision > 0);
        meetings.get(&detail.id).unwrap();
        meetings.list().unwrap();
        assert_eq!(meetings.revision(), revision);
        let clone = meetings.clone();
        clone.save(&detail.id, "Renamed", "", "").unwrap();
        assert!(meetings.revision() > revision);
        let revision = meetings.revision();
        clone.delete(&detail.id).unwrap();
        assert!(meetings.revision() > revision);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    fn recording(meetings: &Meetings) -> MeetingDetail {
        let detail = meetings
            .create_recording("Test meeting", "microphone")
            .unwrap();
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut wav =
            hound::WavWriter::create(meetings.recording_paths(&detail.id).unwrap().mixed, spec)
                .unwrap();
        for _ in 0..160 {
            wav.write_sample(0i16).unwrap();
        }
        wav.finalize().unwrap();
        detail
    }

    fn recorded(meetings: &Meetings) -> MeetingDetail {
        let detail = recording(meetings);
        meetings.finish_recording(&detail.id, 0.01).unwrap()
    }

    #[test]
    fn startup_recovers_recording_and_finalizing_checkpoints_as_ready() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        for finalizing in [false, true] {
            let detail = recording(&meetings);
            if finalizing {
                meetings.begin_recording_finalization(&detail.id).unwrap();
            }
            let path = meetings.recording_paths(&detail.id).unwrap().mixed;
            let before = fs::read(&path).unwrap();
            // A fresh library models a restart: no old writer or waiter survives.
            let reopened = Meetings::new(Store::open(Some(meetings.store.root().to_path_buf())).unwrap());
            assert_eq!(reopened.recover_interrupted().unwrap(), 1);
            let saved = reopened.get(&detail.id).unwrap();
            assert_eq!(saved.status, "ready");
            assert_eq!(saved.duration_seconds, 0.01);
            assert!(saved.has_audio);
            assert!(saved.error.is_none());
            assert_eq!(saved.recording_warnings.len(), 1);
            assert_eq!(fs::read(path).unwrap(), before);
            assert_eq!(reopened.recover_interrupted().unwrap(), 0);
            let job = reopened.begin_transcription(&detail.id, None).unwrap();
            drop(job);
            reopened.cancel(&detail.id).unwrap();
        }
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn startup_recording_recovery_skips_missing_or_invalid_audio() {
        let meetings = library();
        for invalid in [false, true] {
            let detail = meetings.create_recording("Missing audio", "microphone").unwrap();
            meetings.begin_recording_finalization(&detail.id).unwrap();
            if invalid {
                fs::write(meetings.recording_paths(&detail.id).unwrap().mixed, b"RIFF").unwrap();
            }
            assert_eq!(meetings.recover_interrupted().unwrap(), 1);
            assert_eq!(meetings.get(&detail.id).unwrap().status, "interrupted");
        }
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn finalizing_recording_worker_failure_preserves_late_warnings() {
        let meetings = library();
        let detail = recording(&meetings);
        meetings.begin_recording_finalization(&detail.id).unwrap();
        let result = meetings.complete_recording(&detail.id, Err("Worker exited.".into()), vec!["Late source gap.".into()]).unwrap();
        assert_eq!(result.status, "ready");
        assert_eq!(result.duration_seconds, 0.01);
        assert_eq!(result.recording_warnings, ["Late source gap.", "Recording stopped early: Worker exited."]);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn recording_warnings_survive_transcription_edits_and_retry() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recording(&meetings);
        let warnings = vec!["[0:01] Microphone lost; reconnected at 0:03.".to_string()];
        let saved = meetings.finish_recording_with_warnings(&detail.id, 90.0, warnings.clone()).unwrap();
        assert_eq!(saved.status, "ready");
        assert_eq!(saved.duration_seconds, 0.01, "The WAV header is authoritative");
        assert_eq!(saved.recording_warnings, warnings);
        assert!(saved.error.is_none());
        let transcribed = meetings.begin_transcription(&detail.id, None).unwrap()
            .run(|_, _, _| Ok("First raw words.".into())).unwrap();
        assert_eq!(transcribed.recording_warnings, warnings);
        assert!(transcribed.recognition_warnings.is_empty());
        let edited = meetings.save(&detail.id, &detail.title, "Edited words.", "Saved notes.").unwrap();
        assert_eq!(edited.recording_warnings, warnings);
        let retried = meetings.begin_transcription(&detail.id, None).unwrap()
            .run(|_, _, _| Ok("Retry words.".into())).unwrap();
        assert_eq!(retried.recording_warnings, warnings);
        assert_eq!(meetings.get(&detail.id).unwrap().recording_warnings, warnings);
        assert_eq!(meetings.raw_transcript(&detail.id).unwrap().as_deref(), Some("First raw words."));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn failed_recording_retains_structured_unrecovered_source_and_early_stop() {
        let meetings = library();
        let detail = recording(&meetings);
        let issue = crate::recording::SourceIssue {
            source: crate::recording::IssueSource::Microphone,
            kind: crate::recording::IssueKind::Disconnected,
            started_at_seconds: 0.005, ended_at_seconds: None,
            occurrences: 1, lost_duration_seconds: 0.005,
            message: "Microphone disconnected at 0:00; recording continued.".into(),
        };
        let saved = meetings.complete_recording_with_issues(&detail.id,
            Err("Writer stopped".into()), vec![issue.message.clone()], vec![issue]).unwrap();
        assert_eq!(saved.status, "ready");
        assert_eq!(saved.audio_outages.len(), 2);
        assert_eq!(saved.audio_outages[0].source, "microphone");
        assert_eq!(saved.audio_outages[0].start_seconds, 0.005);
        assert_eq!(saved.audio_outages[0].end_seconds, 0.01);
        assert_eq!(saved.audio_outages[1].source, "recording");
        assert_eq!(saved.audio_outages[1].start_seconds, 0.01);
        assert_eq!(saved.missing_seconds, 0.005);
        let reopened = meetings.get(&detail.id).unwrap();
        assert_eq!(reopened.audio_outages[0].start_seconds, 0.005);
        assert_eq!(reopened.recording_warnings.len(), 2);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn fatal_recording_stop_keeps_usable_audio_ready_with_header_duration() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = recording(&meetings);
        let audio = meetings.recording_paths(&detail.id).unwrap().mixed;
        let before = fs::read(&audio).unwrap();
        let gap = "Microphone disconnected at 0:02; recording continued.".to_string();
        let recovered = meetings.recover_recording_with_warnings(&detail.id, "Meeting writer stopped.", vec![gap.clone()]).unwrap();
        assert_eq!(recovered.status, "ready");
        assert!(recovered.has_audio);
        assert_eq!(recovered.duration_seconds, 0.01);
        assert!(recovered.error.is_none());
        assert_eq!(recovered.recording_warnings, [gap, "Recording stopped early: Meeting writer stopped.".into()]);
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        assert_eq!(job.audio, audio);
        drop(job);
        assert_eq!(fs::read(audio).unwrap(), before);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn fatal_recording_stop_without_usable_audio_stays_failed() {
        let meetings = library();
        for malformed in [false, true] {
            let detail = meetings.create_recording("No usable audio", "microphone").unwrap();
            if malformed {
                fs::write(meetings.recording_paths(&detail.id).unwrap().mixed, b"RIFF").unwrap();
            }
            assert_eq!(meetings.recover_recording(&detail.id, "Capture failed.").unwrap_err(), "Capture failed.");
            let failed = meetings.get(&detail.id).unwrap();
            assert_eq!(failed.status, "failed");
            assert_eq!(failed.duration_seconds, 0.0);
            assert_eq!(failed.error.as_deref(), Some("Capture failed."));
        }
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn legacy_failed_recording_derives_duration_without_writes_and_remains_transcribable() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        // Cover both existing rewrite audio and the original legacy audio link.
        for linked_legacy in [false, true] {
            let detail = recording(&meetings);
            let mut record = meetings.read(&detail.id).unwrap();
            record.detail.summary.status = "failed".into();
            record.detail.summary.duration_seconds = 0.0;
            record.detail.error = Some("System capture reported lost audio; this meeting is incomplete.".into());
            let mut audio = meetings.recording_paths(&detail.id).unwrap().mixed;
            if linked_legacy {
                let legacy = meetings.store.root().join(format!("{}.legacy.wav", detail.id));
                fs::rename(&audio, &legacy).unwrap();
                audio = legacy;
                record.legacy_audio = Some(audio.clone());
            }
            meetings.write(&record).unwrap();
            let path = meetings.path(&detail.id).unwrap();
            let before_record = fs::read(&path).unwrap();
            let before_audio = fs::read(&audio).unwrap();
            let revision = meetings.revision();
            let read = meetings.get(&detail.id).unwrap();
            assert_eq!(read.status, "failed", "Reading does not change legacy status");
            assert!(read.has_audio);
            assert_eq!(read.duration_seconds, 0.01);
            let listed = meetings.list().unwrap().into_iter().find(|row| row.id == detail.id).unwrap();
            assert_eq!(listed.duration_seconds, 0.01);
            assert_eq!(meetings.revision(), revision);
            assert_eq!(fs::read(&path).unwrap(), before_record);
            assert_eq!(fs::read(&audio).unwrap(), before_audio);
            let transcribed = meetings.begin_transcription(&detail.id, None).unwrap()
                .run(|path, _, _| {
                    assert_eq!(path, audio.as_path());
                    Ok("Preserved audio words.".into())
                }).unwrap();
            assert_eq!(transcribed.status, "transcribed");
            assert_eq!(transcribed.duration_seconds, 0.01);
            assert_eq!(transcribed.transcript, "Preserved audio words.");
            assert_eq!(fs::read(&audio).unwrap(), before_audio);
        }
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    /// Hands-on check against real two-source audio, in a temporary store so no
    /// saved meeting changes. Opt in with
    /// `SONOJOT_REAL_STEMS=<mixed.wav>;<mic.wav>;<system.wav>`,
    /// `SONOJOT_REAL_MODEL`, `SONOJOT_MODELS` and `SONOJOT_REAL_OUT`.
    #[test]
    #[ignore]
    fn real_two_source_audio_transcribes_into_speaker_passages() {
        let _gate = crate::resource_manager::tests::test_gate();
        let stems = std::env::var("SONOJOT_REAL_STEMS").expect("SONOJOT_REAL_STEMS");
        let stems: Vec<PathBuf> = stems.split(';').map(PathBuf::from).collect();
        let model = std::env::var("SONOJOT_REAL_MODEL").expect("SONOJOT_REAL_MODEL");
        let meetings = library();
        let detail = meetings.create("Real audio check", "legacy", "ready", None, true).unwrap();
        {
            let _guard = meetings.store.lock().unwrap();
            let mut record = meetings.read(&detail.id).unwrap();
            record.legacy_audio = Some(stems[0].clone());
            record.legacy_microphone_audio = Some(stems[1].clone());
            record.legacy_system_audio = Some(stems[2].clone());
            record.detail.summary.has_audio = true;
            meetings.write(&record).unwrap();
        }
        let models = meetings.store.models_dir();
        let started = Instant::now();
        let mut recognizer = None;
        let saved = meetings
            .begin_transcription(&detail.id, Some(&model))
            .unwrap()
            .run_recognition(|audio, model, cancel| {
                if recognizer.is_none() {
                    recognizer = Some(crate::asr::Recognizer::load_interruptible(&models, model, cancel)?);
                }
                crate::asr::transcribe_file_recognition_with(recognizer.as_mut().unwrap(), audio, cancel)
            })
            .unwrap();
        eprintln!("transcribed in {:.0}s, status {}", started.elapsed().as_secs_f32(), saved.status);
        fs::write(std::env::var("SONOJOT_REAL_OUT").expect("SONOJOT_REAL_OUT"), &saved.transcript).unwrap();
        if let Some(raw_out) = std::env::var_os("SONOJOT_REAL_RAW") {
            let history = meetings.store.meetings_dir().join(format!("{}.asr", detail.id));
            let latest = fs::read_dir(history).unwrap().next().unwrap().unwrap().path();
            fs::copy(latest, raw_out).unwrap();
        }
        // Installed speaker labels replace "Other speakers" with "Speaker N".
        assert!(saved.transcript.contains("You: ")
            && (saved.transcript.contains("Other speakers: ") || saved.transcript.contains("Speaker 1: ")));
    }

    #[test]
    fn a_chosen_speech_model_runs_the_job_and_is_recorded_with_its_transcript() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recorded(&meetings);
        assert_eq!(detail.transcription_model, None);
        assert!(meetings.begin_transcription(&detail.id, Some("")).is_err());
        let saved = meetings
            .begin_transcription(&detail.id, Some("moonshine-medium"))
            .unwrap()
            .run(|_, model, _| {
                assert_eq!(model, "moonshine-medium");
                Ok("Chosen model words.".into())
            })
            .unwrap();
        assert_eq!(saved.transcription_model.as_deref(), Some("moonshine-medium"));
        // A failed retry keeps the transcript and the model that wrote it.
        let _ = meetings
            .begin_transcription(&detail.id, Some("moonshine-medium"))
            .unwrap()
            .run(|_, _, _| Err("Speech model unavailable".into()));
        let kept = meetings.get(&detail.id).unwrap();
        assert_eq!(kept.transcript, "Chosen model words.");
        assert_eq!(kept.transcription_model.as_deref(), Some("moonshine-medium"));
    }

    #[test]
    fn granite_default_is_snapshotted_without_changing_dictation_and_can_be_overridden() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        crate::resource_manager::set_test_gpu_table(Some("0, 12288, 10000\n".into()));
        let detail = recorded(&meetings);
        let mut settings = meetings.store.load_settings().unwrap();
        let dictation = settings.asr_model.clone();
        settings.meeting_asr_model = crate::granite_model::GRANITE_ID.into();
        meetings.store.save_settings(&settings).unwrap();
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        settings.meeting_asr_model.clear();
        meetings.store.save_settings(&settings).unwrap();
        let saved = job.run(|_, model, _| {
            assert_eq!(model, crate::granite_model::GRANITE_ID);
            assert!(crate::resource_manager::is_model_active(model));
            assert!(crate::resource_manager::acquire_asr(&dictation).is_err());
            Ok("raw granite words".into())
        }).unwrap();
        assert_eq!(saved.transcription_model.as_deref(), Some(crate::granite_model::GRANITE_ID));
        assert_eq!(saved.transcript, "raw granite words");
        assert_eq!(saved.cleaned_transcript, None);
        assert_eq!(meetings.store.load_settings().unwrap().asr_model, dictation);
        assert!(!crate::resource_manager::is_model_active(crate::granite_model::GRANITE_ID));
        settings.meeting_asr_model = crate::granite_model::GRANITE_ID.into();
        meetings.store.save_settings(&settings).unwrap();
        meetings.begin_transcription(&detail.id, Some("moonshine-medium")).unwrap()
            .run(|_, model, _| {
                assert_eq!(model, "moonshine-medium");
                Ok("explicit override".into())
            }).unwrap();
        crate::resource_manager::set_test_gpu_table(None);
    }

    #[test]
    fn background_transcription_claims_status_and_preserves_first_recognition() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recorded(&meetings);
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        assert_eq!(meetings.get(&detail.id).unwrap().status, "transcribing");
        assert!(meetings.begin_transcription(&detail.id, None).is_err());
        let saved = std::thread::spawn(move || {
            job.run(|_, model, _| {
                assert!(!model.is_empty());
                Ok("Original words.".into())
            })
        })
        .join()
        .unwrap()
        .unwrap();
        assert_eq!(saved.status, "transcribed");
        assert_eq!(
            meetings.get(&detail.id).unwrap().transcript,
            "Original words."
        );
        meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, _, _| Ok("Second recognition.".into()))
            .unwrap();
        assert_eq!(
            meetings.raw_transcript(&detail.id).unwrap().as_deref(),
            Some("Original words.")
        );
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn file_job_awaits_confirmed_cached_release_before_loading() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        // An idle-but-resident cached speech worker holds native weights/DLLs.
        // The file job must observe the worker's CONFIRMED drop before its
        // own Engine loads — never two Engines in one process. Different IDs
        // here (resident tiny, job moonshine) so the job's own pending
        // reservation is exercised across the confirmation.
        let meetings = library();
        let detail = recorded(&meetings);
        let _memory = HermeticMemory::acquire();
        crate::resource_manager::acquire_asr("whisper-tiny").unwrap();
        crate::resource_manager::note_asr_loaded("whisper-tiny");
        crate::resource_manager::notify_asr_finished();
        assert!(crate::resource_manager::asr_resident_blocking());
        // The worker confirms its physical drop shortly after the request.
        let confirmer = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            crate::resource_manager::confirm_asr_released();
        });
        let ran = Arc::new(AtomicBool::new(false));
        let ran_clone = ran.clone();
        let saved = meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, model, _| {
                assert!(
                    !crate::resource_manager::asr_resident_blocking(),
                    "confirmed release must precede the new load"
                );
                assert!(!model.is_empty());
                ran_clone.store(true, Ordering::SeqCst);
                Ok("Confirmed words.".into())
            })
            .unwrap();
        assert!(ran.load(Ordering::SeqCst));
        assert_eq!(saved.transcript, "Confirmed words.");
        assert!(!crate::resource_manager::asr_resident_blocking());
        // Join the helper so no stray thread can touch later tests' state.
        confirmer.join().unwrap();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn file_job_same_id_replacement_keeps_ownership_with_advisory_host_memory() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        // Same-ID cached replacement end to end: the job reuses the resident
        // model (zero pending at acquire), the worker confirms the deliberate
        // drop, and the armed replacement reservation denies a competing text
        // load for the whole ephemeral job — then releases it on finish.
        let meetings = library();
        let detail = recorded(&meetings);
        let _memory = HermeticMemory::tight();
        // Resident moonshine-medium matches the default snapshot model, so
        // begin_transcription + run below is the same-ID replacement path.
        crate::resource_manager::acquire_asr("moonshine-medium").unwrap();
        crate::resource_manager::note_asr_loaded("moonshine-medium");
        crate::resource_manager::notify_asr_finished();
        assert!(crate::resource_manager::asr_resident_blocking());
        let confirmer = std::thread::spawn(|| {
            std::thread::sleep(Duration::from_millis(150));
            crate::resource_manager::confirm_asr_released();
        });
        let saved = meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, model, _| {
                assert_eq!(model, "moonshine-medium");
                assert!(
                    !crate::resource_manager::asr_resident_blocking(),
                    "confirmed release must precede the new load"
                );
                crate::resource_manager::acquire_text_with_admission(
                    "cleanup",
                    "lfm2.5-1.2b",
                    "competing",
                )
                .unwrap();
                // Physical-RAM estimates remain advisory. Stage ownership must
                // still be truthful while a file worker replaces cached ASR.
                assert!(crate::resource_manager::is_model_active(model));
                assert!(crate::resource_manager::is_model_active("lfm2.5-1.2b"));
                crate::resource_manager::notify_text_finished();
                Ok("Same-id words.".into())
            })
            .unwrap();
        assert_eq!(saved.transcript, "Same-id words.");
        // Finished: the reservation released with the lease, text fits again.
        crate::resource_manager::acquire_text_with_admission("cleanup", "lfm2.5-1.2b", "after")
            .unwrap();
        crate::resource_manager::notify_text_finished();
        confirmer.join().unwrap();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn file_job_cancel_while_awaiting_release_never_loads() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        // Cancellation during the release wait must abort without loading a
        // model and must release the lease so the next job can proceed.
        let meetings = library();
        let detail = recorded(&meetings);
        let _memory = HermeticMemory::acquire();
        crate::resource_manager::acquire_asr("moonshine-medium").unwrap();
        crate::resource_manager::note_asr_loaded("moonshine-medium");
        crate::resource_manager::notify_asr_finished();
        // No worker ever confirms; cancel mid-wait instead.
        let canceller = meetings.clone();
        let id = detail.id.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(150));
            canceller.cancel(&id).unwrap();
        });
        let result = meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, _, _| panic!("a cancelled wait must never load"));
        assert!(result.is_err());
        // The lease was released: after the simulated worker finally drops,
        // a fresh job acquires and loads immediately.
        crate::resource_manager::confirm_asr_released();
        meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, _, _| Ok("Recovered.".into()))
            .unwrap();
        assert_eq!(meetings.get(&detail.id).unwrap().transcript, "Recovered.");
        // Join the helper so no stray thread can touch later tests' state.
        canceller.join().unwrap();
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn queued_transcription_cancel_and_failure_release_ownership_and_keep_audio() {
        let _lease_gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let _memory = HermeticMemory::acquire();
        let detail = recorded(&meetings);
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        meetings.cancel(&detail.id).unwrap();
        assert!(job
            .run(|_, _, _| panic!("Cancelled queued work must not load a model"))
            .is_err());
        assert_eq!(meetings.get(&detail.id).unwrap().status, "cancelled");
        assert!(meetings
            .begin_transcription(&detail.id, None)
            .unwrap()
            .run(|_, _, _| Err("Speech model unavailable".into()))
            .is_err());
        let failed = meetings.get(&detail.id).unwrap();
        assert_eq!(failed.status, "failed");
        assert!(failed.has_audio);
        assert_eq!(failed.error.as_deref(), Some("Speech model unavailable"));
        let job = meetings.begin_transcription(&detail.id, None).unwrap();
        assert!(job
            .run(|_, _, _| {
                meetings.cancel(&detail.id).unwrap();
                Ok("Late words must not be published".into())
            })
            .is_err());
        assert!(meetings.get(&detail.id).unwrap().transcript.is_empty());
        assert!(!meetings.operations.lock().unwrap().contains_key(&detail.id));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    fn library() -> Meetings {
        let root = std::env::temp_dir().join(format!("sonojot-meeting-tests-{}", new_id()));
        Meetings::new(Store::open(Some(root)).unwrap())
    }

    /// Hermetic admission memory for transcription tests. The fake recognize
    /// closures never load weights, so admission must not depend on live host
    /// pressure: a browser, dev server, or preceding native suite beside the
    /// run can push free RAM under the honest admission bar and fail the test
    /// for environmental reasons. Restores live probing on drop.
    struct HermeticMemory {
        _gate: std::sync::MutexGuard<'static, ()>,
    }
    impl HermeticMemory {
        fn acquire() -> Self {
            let guard = Self::lock();
            crate::resource_manager::set_test_memory(32 << 30, 24 << 30);
            guard
        }
        /// Tight variant for reservation-gap tests: an lfm2.5-1.2b text load
        /// fits ONLY with zero ASR pending (see the resource-level gap test
        /// for the exact byte math).
        fn tight() -> Self {
            let guard = Self::lock();
            crate::resource_manager::set_test_memory(8 << 30, 4096 << 20);
            guard
        }
        fn lock() -> Self {
            // The fake jobs still share the process-wide speech lease and
            // memory overrides. They cannot run concurrently with one another.
            static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
            Self { _gate: GATE.lock().unwrap_or_else(|poison| poison.into_inner()) }
        }
    }
    impl Drop for HermeticMemory {
        fn drop(&mut self) {
            crate::resource_manager::clear_test_memory();
        }
    }

    fn sample() -> MeetingDetail {
        MeetingDetail {
            recognition_completeness: crate::domain::RecognitionCompleteness::Complete,
            recognition_warnings: Vec::new(),
            recording_warnings: Vec::new(),
            audio_outages: Vec::new(), missing_seconds: 0.0,
            cleaned_transcript: None, transcription_stopped_at_seconds: None,
            summary: MeetingSummary { id: "export-sample".into(), title: "Release planning".into(), created_at: "2026-09-11T14:30:00Z".into(), duration_seconds: 1800.0, status: "processed".into(), has_audio: false, has_transcript: true },
            transcript: "Alex: We agreed to ship on Friday.\nPriya: I will confirm the checklist by Thursday.\nAlex: The budget stays at $12,000.".into(),
            notes: "### Decisions\n\n- **Release:** Friday, subject to the checklist.\n- Budget remains $12,000.\n\n### Follow-up\n\n1. Priya confirms the checklist by Thursday.\n2. Alex shares the release notice.\n\n| Owner | Work | Due |\n| --- | --- | --- |\n| Priya | Confirm checklist | Thursday |\n| Alex | Share notice | Not specified |".into(),
            source: "import".into(), audio_path: None, error: None, transcription_model: None, speaker_count: 0, speaker_labels: true,
        }
    }

    #[test]
    fn automatic_title_triggers_local_and_remote_transcription() {
        let _gate = crate::resource_manager::tests::test_gate();
        let _memory = HermeticMemory::acquire();
        let meetings = library();
        for remote in [false, true] {
            let detail = recorded(&meetings);
            let mut record = meetings.read(&detail.id).unwrap();
            record.title_user_edited = false;
            meetings.write(&record).unwrap();
            let job = meetings.begin_transcription(&detail.id, None).unwrap();
            if remote {
                job.run_remote(|_, _, _| Ok(recognition("Online transcript.", 0.0, 1.0))).unwrap();
            } else {
                job.run_recognition(|_, _, _| Ok(recognition("Local transcript.", 0.0, 1.0))).unwrap();
            }
            assert!(meetings.read(&detail.id).unwrap().automatic_title_pending);
            assert!(meetings.read(&detail.id).unwrap().needs_automatic_title());
        }
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn automatic_title_triggers_cover_imports_retries_and_legacy_completion() {
        let meetings = library();
        for source in ["both", "microphone", "system", "import"] {
            let detail = meetings.create("file name", source, "ready", None, false).unwrap();
            for transcript in ["First complete transcript.", "Transcribed again."] {
                let (_, token) = meetings.begin(&detail.id, "transcribing").unwrap();
                meetings.finish_recognition_operation(&detail.id, &token, Ok(MeetingRecognition {
                    raw: recognition(transcript, 0.0, 1.0), published: transcript.into(),
                }), "test-model").unwrap();
                let record = meetings.read(&detail.id).unwrap();
                assert!(record.needs_automatic_title(), "{source}");
                assert!(record.automatic_title_pending);
                let candidate = record.title_candidate();
                let raw = record.raw_transcript.clone();
                assert!(meetings.generate_automatic_title(&candidate, &AtomicBool::new(false), |_, _| {
                    Ok("Release planning discussion".into())
                }).unwrap());
                assert!(!meetings.read(&detail.id).unwrap().needs_automatic_title());
                assert_eq!(meetings.raw_transcript(&detail.id).unwrap(), raw);
                assert_eq!(meetings.get(&detail.id).unwrap().transcript, transcript);
            }
            let (_, token) = meetings.begin(&detail.id, "transcribing").unwrap();
            meetings.finish_operation(&detail.id, &token, Ok("Legacy completion.".into()), false, None).unwrap();
            assert!(meetings.read(&detail.id).unwrap().automatic_title_pending);
        }
        // Pending work survives app restart, including an imported filename.
        let reopened = Meetings::new(meetings.store.clone());
        assert_eq!(reopened.automatic_title_candidates().unwrap().len(), 4);
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn automatic_title_backfill_eligibility_preserves_provenance_and_incomplete_work() {
        let meetings = library();
        let detail = meetings.create("Untitled meeting", "both", "ready", None, false).unwrap();
        let mut record = meetings.read(&detail.id).unwrap();
        assert!(!record.needs_automatic_title());
        record.detail.transcript = "A saved transcript.".into();
        assert!(record.needs_automatic_title());
        record.title_user_edited = true;
        assert!(!record.needs_automatic_title());
        record.title_user_edited = false;
        record.detail.recognition_completeness = crate::domain::RecognitionCompleteness::Incomplete;
        assert!(!record.needs_automatic_title());
        record.detail.recognition_completeness = crate::domain::RecognitionCompleteness::Complete;
        for status in ["recording", "finalizing", "transcribing", "processingNotes", "generatingTitle", "importing"] {
            record.detail.summary.status = status.into();
            assert!(!record.needs_automatic_title(), "{status}");
        }
        record.detail.summary.status = "transcribed".into();
        record.detail.summary.title = "Already named".into();
        assert!(!record.needs_automatic_title());
        record.automatic_title_pending = true;
        assert!(record.needs_automatic_title());
        let mut legacy = serde_json::to_value(&record).unwrap();
        legacy.as_object_mut().unwrap().remove("titleUserEdited");
        assert!(!serde_json::from_value::<MeetingRecord>(legacy).unwrap().needs_automatic_title());
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    fn untitled_candidate(meetings: &Meetings) -> TitleCandidate {
        let detail = meetings.create("Untitled meeting", "both", "ready", None, false).unwrap();
        meetings.save(&detail.id, &detail.title, "Saved transcript.", "Saved notes.").unwrap();
        meetings.read(&detail.id).unwrap().title_candidate()
    }

    #[test]
    fn automatic_titles_respect_disabled_preference_before_and_during_generation() {
        let meetings = library();
        let candidate = untitled_candidate(&meetings);
        let mut settings = meetings.store.load_settings().unwrap();
        settings.auto_meeting_titles = false;
        meetings.store.save_settings(&settings).unwrap();
        assert!(!meetings.generate_automatic_title(&candidate, &AtomicBool::new(false), |_, _| {
            panic!("Disabled automatic naming must not invoke the model")
        }).unwrap());
        settings.auto_meeting_titles = true;
        meetings.store.save_settings(&settings).unwrap();
        assert!(!meetings.generate_automatic_title(&candidate, &AtomicBool::new(false), |_, _| {
            settings.auto_meeting_titles = false;
            meetings.store.save_settings(&settings).unwrap();
            Ok("Late automatic title".into())
        }).unwrap());
        assert_eq!(meetings.get(&candidate.id).unwrap().title, "Untitled meeting");
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn automatic_titles_discard_results_after_user_edits_or_capture_cancellation() {
        let meetings = library();
        for edit_title in [true, false] {
            let candidate = untitled_candidate(&meetings);
            assert!(!meetings.generate_automatic_title(&candidate, &AtomicBool::new(false), |_, _| {
                meetings.save(&candidate.id,
                    if edit_title { "Director title" } else { "Untitled meeting" },
                    if edit_title { "Saved transcript." } else { "Edited transcript." },
                    "Saved notes.").unwrap();
                Ok("Late automatic title".into())
            }).unwrap());
            assert_ne!(meetings.get(&candidate.id).unwrap().title, "Late automatic title");
        }
        let candidate = untitled_candidate(&meetings);
        let cancel = AtomicBool::new(false);
        assert!(!meetings.generate_automatic_title(&candidate, &cancel, |_, token| {
            token.store(true, Ordering::Release);
            Ok("Cancelled automatic title".into())
        }).unwrap());
        let saved = meetings.get(&candidate.id).unwrap();
        assert_eq!(saved.title, "Untitled meeting");
        assert_eq!(saved.status, "processed");
        assert_eq!(saved.transcript, "Saved transcript.");
        assert_eq!(saved.notes, "Saved notes.");
        assert!(saved.error.is_none());
        assert!(meetings.generate_automatic_title(&candidate, &AtomicBool::new(false), |_, _| {
            Err("No local model".into())
        }).is_err());
        assert!(meetings.get(&candidate.id).unwrap().error.is_none());
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn generated_titles_preserve_user_titles_and_saved_text() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = meetings
            .create("Untitled meeting", "import", "ready", None, false)
            .unwrap();
        meetings
            .save(
                &detail.id,
                &detail.title,
                "We discussed release planning.",
                "User notes.",
            )
            .unwrap();
        let mut record = meetings.read(&detail.id).unwrap();
        record.raw_transcript = Some("raw recognition".into());
        meetings.write(&record).unwrap();
        let named = meetings
            .generate_title(&detail.id, |_, _| Ok("Release planning".into()))
            .unwrap();
        assert_eq!(named.title, "Release planning");
        assert_eq!(named.notes, "User notes.");
        meetings
            .save(
                &detail.id,
                "Director title",
                &named.transcript,
                &named.notes,
            )
            .unwrap();
        let protected = meetings
            .generate_title(&detail.id, |_, _| {
                panic!("A user title must skip the model")
            })
            .unwrap();
        assert_eq!(protected.title, "Director title");
        assert_eq!(
            meetings.raw_transcript(&detail.id).unwrap().as_deref(),
            Some("raw recognition")
        );
        // Files written before title provenance existed remain protected.
        let mut legacy = serde_json::to_value(meetings.read(&detail.id).unwrap()).unwrap();
        legacy.as_object_mut().unwrap().remove("titleUserEdited");
        assert!(
            serde_json::from_value::<MeetingRecord>(legacy)
                .unwrap()
                .title_user_edited
        );
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn title_and_note_failure_or_cancel_keep_completed_text() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = meetings
            .create("Untitled meeting", "import", "ready", None, false)
            .unwrap();
        meetings
            .save(
                &detail.id,
                &detail.title,
                "Saved transcript.",
                "Saved notes.",
            )
            .unwrap();
        assert!(meetings
            .generate_title(&detail.id, |_, _| Err("Model unavailable".into()))
            .is_err());
        let failed = meetings.get(&detail.id).unwrap();
        assert_eq!(failed.transcript, "Saved transcript.");
        assert!(failed.error.unwrap().contains("Transcript saved"));
        assert!(meetings
            .generate_notes(&detail.id, "", |_, _, _| Err("Inference failed".into()))
            .is_err());
        assert_eq!(meetings.get(&detail.id).unwrap().notes, "Saved notes.");
        assert!(meetings
            .generate_notes(&detail.id, "", |_, _, _| {
                meetings.cancel(&detail.id).unwrap();
                Ok("Late generated notes".into())
            })
            .is_err());
        assert_eq!(meetings.get(&detail.id).unwrap().notes, "Saved notes.");
        assert!(!meetings.operations.lock().unwrap().contains_key(&detail.id));
        fs::remove_dir_all(meetings.store.root()).unwrap();
    }

    #[test]
    fn exports_format_notes_and_escape_untrusted_content() {
        let mut detail = sample();
        detail.notes.push_str("\n\n<script>alert('x')</script>\n\n[unsafe](javascript:alert)\n\n![remote](https://example.com/tracker)");
        detail
            .transcript
            .push_str("\n# Verbatim <script> & raw text");
        let html = render_export(&detail, "html").unwrap();
        assert!(html.contains("<h3>Decisions</h3>"));
        assert!(html.contains("<strong>Release:</strong>"));
        assert!(html.contains("<table>"));
        assert!(!html.contains("<script>"));
        assert!(!html.contains("<img "));
        assert!(!html.contains("href=\"javascript:"));
        assert!(html.contains("Content-Security-Policy"));
        assert!(html.contains("data:font/woff2;base64,"));
        assert!(html.contains("data:image/png;base64,"));
        let markdown = render_export(&detail, "markdown").unwrap();
        assert!(markdown.contains("### Decisions"));
        assert!(markdown.contains("**Release:**"));
        assert!(!markdown.contains("<script>"));
        let text = render_export(&detail, "text").unwrap();
        assert!(!text.contains("### Decisions"));
        assert!(!text.contains("**Release:**"));
        assert!(text.contains("# Verbatim <script> & raw text"));
    }

    #[test]
    fn sample_pdf_and_html_have_embedded_material_and_selectable_text() {
        let detail = sample();
        let pdf = render_pdf(&detail).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        let source = String::from_utf8_lossy(&pdf);
        assert!(source.contains("/ToUnicode"));
        assert!(source.contains("/Subtype /Image") || source.contains("/Subtype/Image"));
        if let Some(output) = std::env::var_os("SONOJOT_MEETING_EXPORT_DIR") {
            let directory = PathBuf::from(output);
            fs::create_dir_all(&directory).unwrap();
            fs::write(directory.join("meeting-sample.pdf"), pdf).unwrap();
            fs::write(
                directory.join("meeting-sample.html"),
                render_export(&detail, "html").unwrap(),
            )
            .unwrap();
        }
    }

    fn export_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("sonojot-audio-export-{}", new_id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn audio_export_copies_saved_bytes_verbatim() {
        let meetings = library();
        let detail = recorded(&meetings);
        let source = meetings.select_audio(&detail.id, "mixed").unwrap();
        let source_bytes = fs::read(&source).unwrap();
        assert!(!source_bytes.is_empty());
        let out_dir = export_dir();
        let destination = out_dir.join("exported.wav");
        let bytes = meetings.export_audio(&detail.id, &destination).unwrap();
        assert_eq!(bytes, source_bytes.len() as u64);
        assert_eq!(fs::read(&destination).unwrap(), source_bytes);
        // Format alias shares the same verbatim path.
        let alias = out_dir.join("alias.wav");
        meetings.export(&detail.id, &alias, "audio").unwrap();
        assert_eq!(fs::read(&alias).unwrap(), source_bytes);
        let wav_alias = out_dir.join("alias2.wav");
        meetings.export(&detail.id, &wav_alias, "wav").unwrap();
        assert_eq!(fs::read(&wav_alias).unwrap(), source_bytes);
        // Original and meeting data preserved.
        assert_eq!(fs::read(&source).unwrap(), source_bytes);
        assert_eq!(meetings.get(&detail.id).unwrap().id, detail.id);
        fs::remove_dir_all(meetings.store.root()).unwrap();
        fs::remove_dir_all(&out_dir).unwrap();
    }

    #[test]
    fn audio_export_refuses_active_recording_and_missing_audio() {
        let meetings = library();
        let active = meetings.create_recording("Active", "microphone").unwrap();
        let out_dir = export_dir();
        let refused = meetings
            .export_audio(&active.id, &out_dir.join("active.wav"))
            .unwrap_err();
        assert!(
            refused.contains("Stop the recording"),
            "unexpected: {refused}"
        );
        assert!(!out_dir.join("active.wav").exists());
        // Record with a final non-recording status but no saved WAV.
        let empty = meetings
            .create("Empty", "import", "ready", None, false)
            .unwrap();
        let missing = meetings
            .export_audio(&empty.id, &out_dir.join("missing.wav"))
            .unwrap_err();
        assert!(missing.contains("no saved audio"), "unexpected: {missing}");
        assert!(!out_dir.join("missing.wav").exists());
        fs::remove_dir_all(meetings.store.root()).unwrap();
        fs::remove_dir_all(&out_dir).unwrap();
    }

    #[test]
    fn audio_export_refuses_non_wav_stored_source_without_touching_dest() {
        // Legacy fallbacks carry unverified containers. The WAV label stays
        // honest by refusing non-WAV stored sources instead of mislabelling
        // their bytes under a .wav name.
        let meetings = library();
        let detail = recorded(&meetings);
        // Point the legacy fallback at a non-WAV container and remove the
        // owned mixed file so audio_for falls back to it.
        let legacy = meetings.store.root().join("old-recording.mp3");
        fs::write(&legacy, b"legacy bytes").unwrap();
        let mut record = meetings.read(&detail.id).unwrap();
        record.legacy_audio = Some(legacy.clone());
        meetings.write(&record).unwrap();
        fs::remove_file(meetings.recording_paths(&detail.id).unwrap().mixed).unwrap();
        let out_dir = export_dir();
        let destination = out_dir.join("exported.wav");
        let err = meetings.export_audio(&detail.id, &destination).unwrap_err();
        assert!(err.contains("not in WAV format"), "unexpected: {err}");
        assert!(!destination.exists());
        assert_eq!(fs::read(&legacy).unwrap(), b"legacy bytes");
        fs::remove_dir_all(meetings.store.root()).unwrap();
        fs::remove_dir_all(&out_dir).unwrap();
    }

    #[test]
    fn audio_export_rejects_source_and_internal_targets() {
        let meetings = library();
        let detail = recorded(&meetings);
        let source = meetings.select_audio(&detail.id, "mixed").unwrap();
        let before = fs::read(&source).unwrap();
        let out_dir = export_dir();
        // Same-file destination inside app data is rejected by the existing
        // destination guard before the canonical check; either message is
        // correct, and the original must be untouched.
        let same = meetings.export_audio(&detail.id, &source).unwrap_err();
        assert!(
            same.contains("original meeting media") || same.contains("outside the app data"),
            "unexpected: {same}"
        );
        assert_eq!(fs::read(&source).unwrap(), before);
        // Canonical same-file alias outside app data (hardlink) must hit the
        // original-media guard even though the parent directory is allowed.
        let alias = out_dir.join("alias-source.wav");
        if fs::hard_link(&source, &alias).is_ok() {
            let linked = meetings.export_audio(&detail.id, &alias).unwrap_err();
            assert!(
                linked.contains("original meeting media"),
                "unexpected: {linked}"
            );
            assert_eq!(fs::read(&source).unwrap(), before);
        }
        // Internal app-data destination rejected via existing guard.
        let internal = meetings.store.root().join("outside.wav");
        let rejected = meetings.export_audio(&detail.id, &internal).unwrap_err();
        assert!(
            rejected.contains("outside the app data"),
            "unexpected: {rejected}"
        );
        assert!(!internal.exists());
        fs::remove_dir_all(meetings.store.root()).unwrap();
        fs::remove_dir_all(&out_dir).unwrap();
    }

    #[test]
    fn audio_export_works_while_transcribing() {
        let _gate = crate::resource_manager::tests::test_gate();
        let meetings = library();
        let detail = recorded(&meetings);
        let _job = meetings.begin_transcription(&detail.id, None).unwrap();
        assert_eq!(meetings.get(&detail.id).unwrap().status, "transcribing");
        let out_dir = export_dir();
        let destination = out_dir.join("during-transcribe.wav");
        let source = meetings.select_audio(&detail.id, "mixed").unwrap();
        // select_audio takes the store lock; drop the job guard ordering by
        // exporting through the public path which re-resolves under lock.
        drop(source);
        meetings.export_audio(&detail.id, &destination).unwrap();
        assert!(destination.is_file());
        assert!(
            !meetings.get(&detail.id).unwrap().transcript.is_empty()
                || meetings.get(&detail.id).unwrap().status == "transcribing"
        );
        fs::remove_dir_all(meetings.store.root()).unwrap();
        fs::remove_dir_all(&out_dir).unwrap();
    }
}
fn validate_record(record: &MeetingRecord) -> Result<(), String> {
    validate_id(&record.detail.id)?;
    validate_title(&record.detail.title)?;
    validate_text(&record.detail.transcript, "Transcript")?;
    validate_text(&record.detail.notes, "Notes")?;
    if let Some(cleaned) = &record.detail.cleaned_transcript {
        validate_text(cleaned, "Cleaned transcript")?;
    }
    if !record.detail.missing_seconds.is_finite() || record.detail.missing_seconds < 0.0
        || record.detail.transcription_stopped_at_seconds.is_some_and(|at| !at.is_finite() || at < 0.0)
        || record.detail.audio_outages.iter().any(|gap| {
            !gap.start_seconds.is_finite() || !gap.end_seconds.is_finite()
                || gap.start_seconds < 0.0 || gap.end_seconds < gap.start_seconds
                || !matches!(gap.source.as_str(), "microphone" | "system" | "recording" | "recognition")
                || gap.reason.len() > 2048 || gap.reason.contains('\0')
        }) {
        return Err("The meeting outage data is invalid.".into());
    }
    if let Some(raw) = &record.raw_transcript {
        validate_text(raw, "Raw transcript")?;
    }
    if !record.detail.duration_seconds.is_finite() || record.detail.duration_seconds < 0.0 {
        return Err("The meeting duration is invalid.".into());
    }
    Ok(())
}
fn busy(status: &str) -> bool {
    matches!(
        status,
        "recording" | "finalizing" | "transcribing" | "processingNotes" | "generatingTitle" | "importing"
    )
}
fn completed_status(detail: &MeetingDetail) -> &'static str {
    if detail.transcription_stopped_at_seconds.is_some() {
        "interrupted"
    } else if !detail.notes.trim().is_empty() {
        "processed"
    } else if !detail.transcript.trim().is_empty() {
        "transcribed"
    } else if detail.has_audio {
        "ready"
    } else {
        "failed"
    }
}
pub(crate) fn import_legacy_meetings(store: &Store, legacy_root: &Path) -> Result<usize, String> {
    let index = legacy_root.join("meetings/sessions.json");
    if !index.is_file() {
        return Ok(0);
    }
    let values: Vec<Value> = read_json(&index)?;
    if values.len() > 20_000 {
        return Err("The legacy meeting index exceeds 20,000 entries.".into());
    }
    let meetings = Meetings::new(store.clone());
    let mut count = 0;
    for value in values {
        let legacy_id = string_field(&value, "Id");
        if legacy_id.is_empty() {
            return Err(
                "A legacy meeting is missing its identifier. Original data was preserved.".into(),
            );
        }
        let id = format!("legacy-{}", stable_id(&legacy_id));
        if meetings.path(&id)?.exists() {
            continue;
        }
        let mut title = string_field(&value, "Title");
        if title.trim().is_empty() {
            title = "Imported meeting".into();
        }
        let created_at = string_field(&value, "StartedAtUtc");
        let end = string_field(&value, "EndedAtUtc");
        let duration = OffsetDateTime::parse(&created_at, &Rfc3339)
            .ok()
            .zip(OffsetDateTime::parse(&end, &Rfc3339).ok())
            .map(|(a, b)| (b - a).as_seconds_f64().max(0.0))
            .unwrap_or(0.0);
        let transcript = string_field(&value, "RawTranscript");
        let notes = string_field(&value, "ProcessedNotes");
        let audio = legacy_audio_path(&value, "AudioPath", legacy_root);
        let microphone = legacy_audio_path(&value, "MicrophoneAudioPath", legacy_root);
        let system = legacy_audio_path(&value, "DesktopAudioPath", legacy_root);
        let status = field(&value, "Status");
        let interrupted = status.is_some_and(|v| {
            matches!(v.as_i64(), Some(0 | 1 | 3))
                || matches!(
                    v.as_str(),
                    Some("Recording" | "Transcribing" | "ProcessingNotes")
                )
        });
        let summary = MeetingSummary {
            id,
            title,
            created_at,
            duration_seconds: duration,
            status: if interrupted {
                "interrupted"
            } else if !notes.is_empty() {
                "processed"
            } else if !transcript.is_empty() {
                "transcribed"
            } else {
                "ready"
            }
            .into(),
            has_audio: audio.as_ref().is_some_and(|p| p.is_file()),
            has_transcript: !transcript.is_empty(),
        };
        let record = MeetingRecord {
            schema_version: 1,
            detail: MeetingDetail {
                recognition_completeness: crate::domain::RecognitionCompleteness::Complete,
                recognition_warnings: Vec::new(),
                recording_warnings: Vec::new(),
                audio_outages: Vec::new(), missing_seconds: 0.0,
                cleaned_transcript: None, transcription_stopped_at_seconds: None,
                summary,
                transcript: transcript.clone(),
                notes,
                source: "legacy".into(),
                audio_path: audio.as_ref().map(|p| p.to_string_lossy().into_owned()),
                error: interrupted.then(|| {
                    "Legacy operation was interrupted. Retry using the original audio.".into()
                }),
                transcription_model: None, speaker_count: 0, speaker_labels: true,
            },
            raw_transcript: (!transcript.is_empty()).then_some(transcript),
            title_user_edited: true,
            automatic_title_pending: false,
            legacy_audio: audio,
            legacy_microphone_audio: microphone,
            legacy_system_audio: system,
            source_media: legacy_audio_path(&value, "SourceMediaPath", legacy_root),
            recognition_file: None,
            outages_recorded: false,
        };
        meetings.write(&record)?;
        count += 1;
    }
    Ok(count)
}
fn legacy_audio_path(value: &Value, key: &str, root: &Path) -> Option<PathBuf> {
    let text = string_field(value, key);
    if text.is_empty() {
        return None;
    }
    let path = PathBuf::from(text);
    Some(if path.is_absolute() {
        path
    } else {
        root.join("meetings").join(path)
    })
}

pub(crate) fn resolve_ffmpeg(store: &Store, override_path: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(path) = override_path {
        if path.is_file() {
            return Ok(path.into());
        }
        return Err("The configured FFmpeg executable does not exist.".into());
    }
    if let Some(path) = crate::environment::var_os("FFMPEG").map(PathBuf::from) {
        if path.is_file() {
            return Ok(path);
        }
        return Err("SONOJOT_FFMPEG does not point to an executable file.".into());
    }
    let executable = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let bundled = store.root().join("tools/ffmpeg/bin").join(executable);
    if bundled.is_file() {
        return Ok(bundled);
    }
    if let Some(paths) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&paths) {
            let candidate = directory.join(executable);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("FFmpeg is required for media import. Install FFmpeg or set SONOJOT_FFMPEG to its executable.".into())
}
fn run_ffmpeg(
    executable: &Path,
    source: &Path,
    destination: &Path,
    cancel: &AtomicBool,
) -> Result<(), String> {
    if cancel.load(Ordering::SeqCst) {
        return Err("Media import cancelled.".into());
    }
    let mut command = Command::new(executable);
    command
        .args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-protocol_whitelist",
            "file,pipe",
            "-i",
        ])
        .arg(source)
        .args([
            "-map",
            "0:a:0",
            "-vn",
            "-ac",
            "1",
            "-ar",
            "16000",
            "-c:a",
            "pcm_s16le",
            "-f",
            "wav",
        ])
        .arg(destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = command
        .spawn()
        .map_err(|e| format!("Cannot start FFmpeg: {e}"))?;
    // Drain the pipe continuously without retaining unbounded decoder output.
    let stderr = child.stderr.take();
    let drain = thread::spawn(move || {
        if let Some(mut pipe) = stderr {
            let mut buffer = [0u8; 8192];
            while matches!(pipe.read(&mut buffer), Ok(n) if n > 0) {}
        }
    });
    let started = Instant::now();
    let result = loop {
        let error = if cancel.load(Ordering::SeqCst) {
            Some("Media import cancelled.")
        } else if started.elapsed() > Duration::from_secs(900) {
            Some("Media import exceeded the 15-minute conversion limit.")
        } else if fs::metadata(destination).is_ok_and(|m| m.len() > 2 * 1024 * 1024 * 1024) {
            Some("Converted media exceeded the 2 GiB audio limit.")
        } else {
            None
        };
        if let Some(error) = error {
            let _ = child.kill();
            let _ = child.wait();
            break Err(error.into());
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => break Err("FFmpeg could not decode the selected file. Check that it contains a supported audio stream.".into()),
            Ok(None) => thread::sleep(Duration::from_millis(40)),
            Err(error) => { let _ = child.kill(); let _ = child.wait(); break Err(format!("Cannot monitor FFmpeg: {error}")); }
        }
    };
    let _ = drain.join();
    if result.is_err() {
        let _ = fs::remove_file(destination);
    }
    result
}

/// Read standard and long RF64 meeting headers through the shared parser.
pub fn wav_duration(path: &Path) -> Result<f64, String> {
    crate::recording::wav_duration(path)
}
