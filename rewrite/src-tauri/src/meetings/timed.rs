//! Timed exports use saved recognition, never timestamps inferred from edits.

use super::*;
use std::io::Write;

/// Matches the labels accepted by the meeting speaker editor.
pub fn validate_speaker(speaker: Option<&str>) -> Result<Option<String>, String> {
    speaker.map(|speaker| {
        let speaker = speaker.trim();
        if speaker.is_empty() || speaker.len() > 48
            || speaker.chars().any(|c| c.is_control() || matches!(c, ':' | '[' | ']')) {
            return Err("Speaker names must be 1–48 bytes without colons, brackets or control characters.".into());
        }
        Ok(speaker.to_owned())
    }).transpose()
}

impl Meetings {
    /// Saves an import's single-speaker preference without changing an existing
    /// transcript or any global setting. Passing None clears the preference.
    pub fn set_import_speaker(&self, id: &str, speaker: Option<&str>) -> Result<MeetingDetail, String> {
        let speaker = validate_speaker(speaker)?;
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if record.detail.source != "import" {
            return Err("A saved speaker preference is only available for imported media.".into());
        }
        if self.operations.lock().map_err(|_| "Meeting operation lock failed.")?.contains_key(id)
            || (busy(&record.detail.status) && record.detail.status != "importing") {
            return Err("Finish or cancel the current meeting operation first.".into());
        }
        record.import_speaker = speaker;
        self.write(&record)?;
        Ok(record.detail)
    }

    /// A stable, read-only view of the history that produced the current
    /// transcript. A later incomplete retry may be recorded separately; its
    /// diagnostics must not be attached to these older segments as provenance.
    pub fn timed_transcript(&self, id: &str) -> Result<Value, String> {
        self.timed_transcript_attempt(id, false)
    }

    /// The published document may deliberately retain an older attempt after
    /// an incomplete retry. Keep both histories accessible without editing it.
    pub fn timed_transcript_attempt(&self, id: &str, latest: bool) -> Result<Value, String> {
        let _guard = self.store.lock()?;
        let record = self.read(id)?;
        let history = record.recognition_file.as_deref().map(|file| {
            if file.contains(['/', '\\']) || !file.ends_with(".json")
                || Path::new(file).file_name().and_then(|name| name.to_str()) != Some(file) {
                return Err("The saved recognition filename is invalid.".into());
            }
            self.store.read::<Value>(&self.store.meetings_dir().join(format!("{id}.asr")).join(file))
        }).transpose()?;
        let mut value = structured(&record, history.as_ref())?;
        let directory = self.store.meetings_dir().join(format!("{id}.asr"));
        let mut newest = history.clone().map(|h| (record.recognition_file.clone().unwrap_or_default(), h));
        if directory.is_dir() {
            self.store.check_owned(&directory)?;
            for entry in fs::read_dir(&directory).map_err(|e| format!("Cannot list recognition history: {e}"))? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_file()
                    || entry.path().extension().is_none_or(|ext| ext != "json") { continue; }
                let candidate: Value = self.store.read(&entry.path())?;
                let name = entry.file_name().to_string_lossy().into_owned();
                if newest.as_ref().is_none_or(|(previous_name, previous)|
                    (history_time(&candidate), name.as_str()) > (history_time(previous), previous_name.as_str())) {
                    newest = Some((name, candidate));
                }
            }
        }
        let differs = newest.as_ref().is_some_and(|(name, _)| Some(name) != record.recognition_file.as_ref());
        value["publishedTranscriptUsesLatestAttempt"] = serde_json::json!(!differs);
        if let Some((name, attempt)) = newest {
            let mut snapshot = structured(&record, Some(&attempt))?;
            snapshot["recognitionFile"] = serde_json::json!(name);
            snapshot["recognizedTranscript"] = Value::Null;
            snapshot["cleanedTranscript"] = Value::Null;
            snapshot["textVersion"] = serde_json::json!("latestAttempt");
            snapshot["publishedTranscriptUsesLatestAttempt"] = serde_json::json!(!differs);
            if latest { return Ok(snapshot); }
            if differs {
                value["latestAttempt"] = snapshot;
                value["attemptNotice"] = serde_json::json!("The published transcript was preserved from an older attempt. Latest attempt segments and coverage are separate; use --latest-attempt to export them without replacing the document.");
            }
        } else if latest { return Err("This meeting has no saved recognition attempt.".into()); }
        Ok(value)
    }

    pub(super) fn export_timed(&self, id: &str, path: &Path, format: &str, version: Option<&str>) -> Result<(), String> {
        match version.unwrap_or("recognized") {
            "recognized" => {},
            "cleaned" if format == "json" => {},
            "latestAttempt" => {},
            "cleaned" => return Err("Cleaned text has no word alignment. Export JSON to keep cleaned text beside the original timed segments.".into()),
            _ => return Err("Choose the cleaned or recognized transcript.".into()),
        }
        self.store.check_export_destination(path)?;
        let transcript = self.timed_transcript_attempt(id, version == Some("latestAttempt"))?;
        if version == Some("cleaned") && transcript["cleanedTranscript"].is_null() {
            return Err("Clean this meeting before exporting its cleaned transcript.".into());
        }
        let contents = if format == "json" {
            serde_json::to_string_pretty(&transcript).map_err(|e| format!("Cannot serialize timed transcript: {e}"))? + "\n"
        } else { subtitles(&transcript, format)? };
        // create_new protects existing files, including hardlink aliases of
        // original media, and closes the check-then-overwrite race.
        let mut file = fs::OpenOptions::new().write(true).create_new(true).open(path)
            .map_err(|e| if e.kind() == std::io::ErrorKind::AlreadyExists {
                "The export destination already exists. Choose a different filename.".into()
            } else { format!("Cannot create transcript export: {e}") })?;
        let result = file.write_all(contents.as_bytes()).and_then(|_| file.sync_all());
        drop(file);
        if let Err(error) = result {
            let _ = fs::remove_file(path);
            return Err(format!("Cannot save transcript export: {error}"));
        }
        Ok(())
    }
}

fn history_time(history: &Value) -> Option<i128> {
    time::OffsetDateTime::parse(history["createdAt"].as_str()?, &time::format_description::well_known::Rfc3339)
        .ok().map(|stamp| stamp.unix_timestamp_nanos())
}

fn union_ranges(mut ranges: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    ranges.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut merged: Vec<(f64, f64)> = Vec::new();
    for (start, end) in ranges {
        if let Some(last) = merged.last_mut().filter(|last| start <= last.1) {
            last.1 = last.1.max(end);
        } else { merged.push((start, end)); }
    }
    merged
}

/// A gap in saved speech windows can be silence. Report its bounds for review
/// without claiming a measured amount of lost speech or guessing word times.
fn coverage(segments: &[Value], duration: f64) -> Value {
    let mut covered = Vec::new();
    let mut omitted = Vec::new();
    let mut unknown_ends = false;
    for segment in segments {
        let Some(start) = segment["start"].as_f64() else { continue; };
        let Some(end) = segment["end"].as_f64() else { unknown_ends = true; continue; };
        let (start, end) = (start.clamp(0.0, duration), end.clamp(0.0, duration));
        if end <= start { continue; }
        if segment["complete"] == true && segment["text"].as_str().is_some_and(|text| !text.trim().is_empty()) {
            covered.push((start, end));
        } else if segment["complete"] == false && segment["text"].as_str().is_some_and(|text| text.trim().is_empty()) {
            omitted.push((start, end));
        }
    }
    let covered = union_ranges(covered);
    let omitted = union_ranges(omitted);
    let mut uncovered = Vec::new();
    let mut cursor = 0.0;
    for &(start, end) in &covered {
        if start > cursor { uncovered.push((cursor, start)); }
        cursor = cursor.max(end);
    }
    if cursor < duration { uncovered.push((cursor, duration)); }
    let ranges = |ranges: &[(f64, f64)]| ranges.iter().map(|&(start, end)| serde_json::json!({"start":start,"end":end})).collect::<Vec<_>>();
    let seconds = |ranges: &[(f64, f64)]| ranges.iter().map(|(start, end)| end-start).sum::<f64>().max(0.0);
    serde_json::json!({
        "coveredRanges": ranges(&covered), "uncoveredRanges": ranges(&uncovered),
        "knownOmittedRanges": ranges(&omitted), "knownOmittedSeconds": seconds(&omitted),
        "uncoveredSeconds": seconds(&uncovered), "hasUnknownSegmentEnds": unknown_ends,
        "uncoveredMayIncludeSilence": true,
        "note": "Uncovered ranges have no complete saved transcription window. They may include silence, omitted speech, partial decoding or audio never processed. Known omitted ranges are empty incomplete decode windows. No speech duration is inferred from silence or untimed text."
    })
}

fn structured(record: &MeetingRecord, history: Option<&Value>) -> Result<Value, String> {
    let mut segments = Vec::new();
    let mut complete_bounds = true;
    if let Some(history) = history {
        let saved = history["segments"].as_array().ok_or("The saved recognition has no segment list.")?;
        for segment in saved {
            let start = segment["start_seconds"].as_f64().filter(|n| n.is_finite() && *n >= 0.0)
                .ok_or("A saved recognition segment has an invalid start time.")?;
            if !segment["end_seconds"].is_null() {
                segment["end_seconds"].as_f64().filter(|n| n.is_finite() && *n >= start)
                    .ok_or("A saved recognition segment has an invalid end time.")?;
            } else { complete_bounds = false; }
            if !segment["text"].is_string()
                || (!segment["speaker"].is_null() && !segment["speaker"].is_string()) {
                return Err("A saved recognition segment has invalid text or speaker data.".into());
            }
            // Clone saved JSON numbers directly: no rounding, alignment,
            // sorting, merging, or whitespace normalization in structured data.
            segments.push(serde_json::json!({
                "id": segment["id"], "start": segment["start_seconds"],
                "end": segment["end_seconds"], "speaker": segment["speaker"],
                "text": segment["text"], "complete": segment["complete"],
            }));
        }
    }
    let model = history.and_then(|h| h["model"].as_str());
    let window_bounds = model.is_some_and(|model| model.contains("qwen") || model.starts_with("openrouter:"));
    let available = !segments.is_empty();
    let timing_reason = if history.is_none() {
        Some("No structured recognition history was saved for this transcript. Transcribe the saved audio again to obtain timing.")
    } else if !available {
        Some("The saved recognition contains no timed segments. No timestamps were inferred from transcript text.")
    } else { None };
    let word_reason = "Word timestamps are unavailable: the saved recognition contains no word timing. Segment and audio-window boundaries are not word alignment.";
    let coverage = coverage(&segments, record.detail.duration_seconds);
    Ok(serde_json::json!({
        "schemaVersion": 1,
        "meetingId": record.detail.id,
        "title": record.detail.title,
        "durationSeconds": record.detail.duration_seconds,
        "model": model,
        "recognitionFile": record.recognition_file,
        "recognitionCreatedAt": history.and_then(|h| h.get("createdAt")),
        "knownSpeaker": history.and_then(|h| h.get("knownSpeaker")),
        "rawTranscript": history.and_then(|h| h.get("raw")).cloned()
            .unwrap_or_else(|| serde_json::json!(record.raw_transcript)),
        "guardedTranscript": history.and_then(|h| h.get("guarded").or_else(|| h.get("raw"))),
        "recognizedTranscript": record.detail.transcript,
        "cleanedTranscript": record.detail.cleaned_transcript,
        "cleanedTranscriptAlignment": null,
        "coverage": coverage,
        "captureMissingSeconds": record.detail.missing_seconds.max(0.0),
        "segments": segments,
        "segmentTextSource": "savedRecognitionSegments",
        "segmentTextNote": "Segment text is preserved as saved by recognition, after any recognition safety guards. Original pre-guard text is retained separately in rawTranscript; edited and cleaned text is not aligned to these segments.",
        "words": null,
        "wordTimestamps": {"available": false, "reason": word_reason},
        "timing": {
            "available": available,
            "precision": if !available { "unavailable" } else if window_bounds { "audioWindow" } else { "segment" },
            "unit": "seconds",
            "provenance": if available { Some("savedRecognitionSegments") } else { None },
            "completeBounds": available && complete_bounds,
            "accuracySeconds": null,
            "reason": timing_reason,
            "note": if window_bounds {
                "Times are the audio windows supplied to the recognizer, not measured word or sentence boundaries."
            } else {
                "Times are saved recognition segment boundaries. Boundary accuracy has not been measured."
            },
        },
        "completeness": history.and_then(|h| h.get("completeness")),
        "warnings": history.and_then(|h| h.get("warnings")).cloned().unwrap_or_else(|| serde_json::json!([])),
        "meetingCompleteness": record.detail.recognition_completeness,
        "meetingWarnings": record.detail.recognition_warnings,
        "recordingWarnings": record.detail.recording_warnings,
        "audioOutages": record.detail.audio_outages,
    }))
}

fn timestamp(seconds: f64, separator: char) -> Result<String, String> {
    if !seconds.is_finite() || !(0.0..=359_999.999).contains(&seconds) {
        return Err("Subtitle times must be between zero and 99:59:59.999.".into());
    }
    let millis = (seconds * 1000.0).round() as u64;
    Ok(format!("{:02}:{:02}:{:02}{separator}{:03}", millis / 3_600_000,
        millis / 60_000 % 60, millis / 1000 % 60, millis % 1000))
}

fn subtitles(transcript: &Value, format: &str) -> Result<String, String> {
    let webvtt = match format {
        "srt" => false,
        "vtt" => true,
        _ => return Err("Choose JSON, SRT or VTT for a timed transcript export.".into()),
    };
    let segments = transcript["segments"].as_array().ok_or("The timed transcript has no segments.")?;
    let mut cues = Vec::new();
    for (index, segment) in segments.iter().enumerate() {
        let text = segment["text"].as_str().ok_or("A timed segment has no text.")?;
        if text.trim().is_empty() { continue; }
        let start = segment["start"].as_f64().ok_or("A timed segment has no start time.")?;
        let end = segment["end"].as_f64().ok_or("A saved segment has no end time. Export JSON to preserve its unknown boundary.")?;
        // Avoid inventing a display duration for zero-length or sub-ms cues.
        if end <= start || (end * 1000.0).round() <= (start * 1000.0).round() {
            return Err("A saved segment has no positive subtitle duration. Export JSON to preserve its original timing.".into());
        }
        cues.push((start, end, index, text, segment["speaker"].as_str()));
    }
    if cues.is_empty() {
        return Err("This meeting has no timed speech segments. Transcribe its saved audio before exporting subtitles.".into());
    }
    // Subtitle readers expect chronological cues. Keep overlaps, gaps and
    // equal-start order; none of these prove a boundary should be changed.
    cues.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.2.cmp(&b.2)));
    let mut output = if webvtt { "WEBVTT\n\n".to_owned() } else { String::new() };
    for (index, (start, end, _, text, speaker)) in cues.into_iter().enumerate() {
        let separator = if webvtt { '.' } else { ',' };
        let start = timestamp(start, separator)?;
        let end = timestamp(end, separator)?;
        // Subtitle markup must never consume recognized '<...>' text. Flatten
        // blank lines so recognized text cannot start another cue or VTT block.
        let escape = |text: &str| text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
            .replace('\r', "").lines().map(str::trim).filter(|line| !line.is_empty()).collect::<Vec<_>>().join("\n");
        let text = escape(text);
        let text = speaker.filter(|name| !name.is_empty()).map(|name| format!("{}: {text}", escape(name)))
            .unwrap_or(text);
        output.push_str(&format!("{}\n{start} --> {end}\n{text}\n\n", index + 1));
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timed_coverage_reports_omission_and_uncovered_tail_without_claiming_silence_is_speech() {
        let segments = serde_json::json!([
            {"start":0.0,"end":10.0,"text":"Retained.","complete":true},
            {"start":10.0,"end":12.3,"text":"","complete":false},
            {"start":15.0,"end":20.0,"text":"Resumed.","complete":true}
        ]);
        let result = coverage(segments.as_array().unwrap(), 30.0);
        assert_eq!(result["uncoveredRanges"], serde_json::json!([{"start":10.0,"end":15.0},{"start":20.0,"end":30.0}]));
        assert_eq!(result["knownOmittedRanges"], serde_json::json!([{"start":10.0,"end":12.3}]));
        assert_eq!(result["uncoveredSeconds"], 15.0);
        assert_eq!(result["uncoveredMayIncludeSilence"], true);
        let untimed = serde_json::json!([{"start":0.0,"end":null,"text":"Unknown coverage.","complete":false}]);
        let result = coverage(untimed.as_array().unwrap(), 30.0);
        assert_eq!(result["uncoveredRanges"], serde_json::json!([{"start":0.0,"end":30.0}]));
        assert_eq!(result["hasUnknownSegmentEnds"], true);
    }

    #[test]
    fn timed_subtitles_preserve_overlaps_speakers_and_escape_markup() {
        let data = serde_json::json!({"segments":[
            {"start":3600.125,"end":3602.5,"speaker":"Cody", "text":"  later <tag> & words\n\nnext line  "},
            {"start":3599.0,"end":3601.0,"speaker":null,"text":"Earlier."}
        ]});
        let srt = subtitles(&data, "srt").unwrap();
        assert!(srt.starts_with("1\n00:59:59,000 --> 01:00:01,000\nEarlier.\n\n"));
        assert!(srt.contains("2\n01:00:00,125 --> 01:00:02,500\nCody: later &lt;tag&gt; &amp; words\nnext line\n\n"));
        let vtt = subtitles(&data, "vtt").unwrap();
        assert!(vtt.starts_with("WEBVTT\n\n1\n00:59:59.000 --> 01:00:01.000"));
        assert!(vtt.contains("01:00:00.125 --> 01:00:02.500"));
        assert_eq!(data["segments"][0]["text"], "  later <tag> & words\n\nnext line  ");
    }

    #[test]
    fn timed_subtitles_never_invent_missing_or_zero_length_ends() {
        for end in [Value::Null, serde_json::json!(2.0), serde_json::json!(1.0), serde_json::json!(2.0001)] {
            let data = serde_json::json!({"segments":[{"start":2.0,"end":end,"text":"Speech."}]});
            assert!(subtitles(&data, "srt").is_err());
            assert!(subtitles(&data, "vtt").is_err());
        }
        assert!(subtitles(&serde_json::json!({"segments":[]}), "srt").is_err());
    }

    #[test]
    fn timed_speaker_validation_matches_published_label_syntax() {
        assert_eq!(validate_speaker(Some("  Cody Stevens  ")).unwrap().as_deref(), Some("Cody Stevens"));
        assert_eq!(validate_speaker(None).unwrap(), None);
        for invalid in ["", "  ", "A:B", "[Cody]", "Cody\nStevens", "Cody\0", "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXY"] {
            assert!(validate_speaker(Some(invalid)).is_err(), "{invalid:?}");
        }
    }
}
