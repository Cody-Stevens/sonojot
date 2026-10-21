//! Joining split recordings and naming speakers.
//!
//! Joining never touches the source meetings: it writes a new meeting whose
//! stems are the parts end to end. Each part is padded to its longest stem so
//! microphone and system audio stay aligned after the join. Speaker names
//! edit the published transcript only; raw recognition keeps its labels.

use super::*;
use crate::domain::AudioOutage;
use std::io::{BufWriter, Seek, SeekFrom, Write};

const MAX_JOIN_PARTS: usize = 32;
const MAX_SPEAKER_NAME_BYTES: usize = 48;

/// A speaker label in the published transcript and how many passages it has.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpeakerLabel {
    pub label: String,
    pub passages: usize,
}

struct Stem {
    path: PathBuf,
    info: crate::recording::WavInfo,
}

impl Stem {
    fn open(path: PathBuf) -> Result<Self, String> {
        let info = crate::recording::read_wav_info(&path)?;
        if info.format != 1 || info.channels != 1 || info.bits_per_sample != 16 || info.sample_rate == 0 {
            return Err("Only SonoJot recordings (16-bit mono PCM) can be joined.".into());
        }
        Ok(Self { path, info })
    }
    fn frames(&self) -> u64 {
        self.info.data_bytes / 2
    }
    fn seconds(&self) -> f64 {
        self.frames() as f64 / f64::from(self.info.sample_rate)
    }
}

struct Part {
    record: MeetingRecord,
    mixed: Stem,
    stems: Option<(Stem, Stem)>,
}

impl Part {
    /// The longest stem sets the part length so every stem pads to the same end.
    fn seconds(&self) -> f64 {
        let mut seconds = self.mixed.seconds();
        if let Some((microphone, system)) = &self.stems {
            seconds = seconds.max(microphone.seconds()).max(system.seconds());
        }
        seconds
    }
}

impl Meetings {
    /// Creates a new meeting from saved recordings in the given order.
    pub fn join(&self, ids: &[String], title: Option<&str>) -> Result<MeetingDetail, String> {
        if ids.len() < 2 || ids.len() > MAX_JOIN_PARTS {
            return Err(format!("Choose 2–{MAX_JOIN_PARTS} meetings to join."));
        }
        for (index, id) in ids.iter().enumerate() {
            validate_id(id)?;
            if ids[..index].contains(id) {
                return Err("Each meeting can appear only once in a join.".into());
            }
        }
        if let Some(title) = title { validate_title(title)?; }
        let parts = {
            let _guard = self.store.lock()?;
            let mut parts = Vec::with_capacity(ids.len());
            for id in ids {
                let record = self.read(id)?;
                if busy(&record.detail.status) {
                    return Err(format!("“{}” is busy. Wait for it to finish before joining.", record.detail.title));
                }
                let mixed = self.audio_for(&record, "mixed")?.filter(|p| p.is_file())
                    .ok_or_else(|| format!("“{}” has no saved audio.", record.detail.title))?;
                let stems = if matches!(record.detail.source.as_str(), "both" | "legacy") && record.source_media.is_none() {
                    match (self.audio_for(&record, "microphone")?, self.audio_for(&record, "system")?) {
                        (Some(m), Some(s)) if m.is_file() && s.is_file() => Some((Stem::open(m)?, Stem::open(s)?)),
                        _ => None,
                    }
                } else { None };
                parts.push(Part { mixed: Stem::open(mixed)?, stems, record });
            }
            parts
        };
        let with_stems = parts.iter().all(|part| part.stems.is_some());
        let same_rate = |rates: Vec<u32>| rates.windows(2).all(|pair| pair[0] == pair[1]);
        let stem_rates = |pick: fn(&(Stem, Stem)) -> &Stem| {
            parts.iter().filter_map(|p| p.stems.as_ref()).map(|s| pick(s).info.sample_rate).collect()
        };
        if !same_rate(parts.iter().map(|p| p.mixed.info.sample_rate).collect())
            || (with_stems && (!same_rate(stem_rates(|s| &s.0)) || !same_rate(stem_rates(|s| &s.1)))) {
            return Err("These recordings use different sample rates and cannot be joined.".into());
        }
        let first = &parts[0].record.detail;
        let source = if with_stems {
            "both"
        } else if parts.iter().all(|p| p.record.detail.source == first.source && first.source != "both") {
            first.source.as_str()
        } else {
            "import"
        };
        let title = match title {
            Some(title) => title.trim().to_owned(),
            None => format!("{} (joined)", first.title.chars().take(1000).collect::<String>()),
        };
        let created = self.create(&title, source, "importing", None, true)?;
        let id = created.summary.id.clone();
        let result = self.write_joined(&id, &parts, with_stems);
        let _guard = self.store.lock()?;
        let mut record = self.read(&id)?;
        match result {
            Ok(duration) => {
                let mut warnings = Vec::new();
                let mut outages = Vec::new();
                let mut offset = 0.0;
                let mut starts = Vec::new();
                for (index, part) in parts.iter().enumerate() {
                    starts.push(format!("“{}” at {}", part.record.detail.title, clock(offset)));
                    warnings.extend(part.record.detail.recording_warnings.iter()
                        .map(|warning| format!("Part {}: {warning}", index + 1)));
                    outages.extend(part.record.detail.audio_outages.iter()
                        .filter(|gap| gap.source != "recognition")
                        .map(|gap| AudioOutage {
                            start_seconds: gap.start_seconds + offset,
                            end_seconds: gap.end_seconds + offset,
                            ..gap.clone()
                        }));
                    offset += part.seconds();
                }
                warnings.insert(0, format!("Joined from {} recordings: {}.", parts.len(), starts.join(", ")));
                record.detail.summary.created_at = parts[0].record.detail.created_at.clone();
                record.detail.summary.duration_seconds = duration;
                record.detail.summary.has_audio = true;
                record.detail.summary.status = "ready".into();
                record.detail.recording_warnings = warnings;
                record.detail.missing_seconds = parts.iter().map(|p| p.record.detail.missing_seconds).sum::<f64>().min(duration);
                record.detail.audio_outages = outages;
                record.outages_recorded = true;
                record.detail.error = None;
                self.write(&record)?;
                Ok(record.detail)
            }
            Err(error) => {
                for path in self.joined_paths(&id)?.into_iter().flat_map(|(temp, _)| [temp]) {
                    let _ = fs::remove_file(path);
                }
                record.detail.summary.status = "failed".into();
                record.detail.error = Some(error.chars().take(2048).collect());
                self.write(&record)?;
                Err(error)
            }
        }
    }

    fn joined_paths(&self, id: &str) -> Result<[(PathBuf, PathBuf); 3], String> {
        let paths = self.recording_paths(id)?;
        let directory = self.store.meetings_dir();
        let temp = |name: &str| -> Result<PathBuf, String> {
            let path = directory.join(format!("{id}.{name}.join.wav"));
            self.store.check_owned(&path)?;
            Ok(path)
        };
        Ok([
            (temp("mixed")?, paths.mixed),
            (temp("mic")?, paths.microphone),
            (temp("system")?, paths.system),
        ])
    }

    /// Writes each stem to a temporary file, then renames all of them into place.
    fn write_joined(&self, id: &str, parts: &[Part], with_stems: bool) -> Result<f64, String> {
        let [mixed, microphone, system] = self.joined_paths(id)?;
        let lengths: Vec<f64> = parts.iter().map(Part::seconds).collect();
        concatenate(&mixed.0, parts.iter().map(|p| &p.mixed), &lengths)?;
        let mut outputs = vec![mixed];
        if with_stems {
            concatenate(&microphone.0, parts.iter().map(|p| &p.stems.as_ref().unwrap().0), &lengths)?;
            concatenate(&system.0, parts.iter().map(|p| &p.stems.as_ref().unwrap().1), &lengths)?;
            outputs.extend([microphone, system]);
        }
        for (temp, destination) in &outputs {
            if destination.exists() {
                return Err("The joined meeting already has saved audio.".into());
            }
            fs::rename(temp, destination).map_err(|e| format!("Cannot save joined audio: {e}"))?;
        }
        wav_duration(&outputs[0].1)
    }

    /// Distinct speaker labels in the published transcript, in order of first use.
    pub fn speakers(&self, id: &str) -> Result<Vec<SpeakerLabel>, String> {
        let detail = self.get(id)?;
        let mut labels: Vec<SpeakerLabel> = Vec::new();
        for label in detail.transcript.lines().filter_map(passage_label) {
            match labels.iter_mut().find(|known| known.label == label) {
                Some(known) => known.passages += 1,
                None => labels.push(SpeakerLabel { label: label.into(), passages: 1 }),
            }
        }
        Ok(labels)
    }

    /// Renames one speaker in the transcript and cleaned transcript. This is an
    /// edit: switching speaker labels later asks before replacing it.
    pub fn rename_speaker(&self, id: &str, from: &str, to: &str) -> Result<MeetingDetail, String> {
        let to = to.trim();
        if to.is_empty() || to.len() > MAX_SPEAKER_NAME_BYTES
            || to.chars().any(|c| c.is_control() || matches!(c, ':' | '[' | ']')) {
            return Err(format!("Speaker names must be 1–{MAX_SPEAKER_NAME_BYTES} bytes without colons, brackets or control characters."));
        }
        let _guard = self.store.lock()?;
        let mut record = self.read(id)?;
        if busy(&record.detail.status) {
            return Err("Finish or cancel the current meeting operation before editing.".into());
        }
        let (transcript, count) = rename_label(&record.detail.transcript, from, to);
        if count == 0 {
            let known: Vec<&str> = record.detail.transcript.lines().filter_map(passage_label).collect();
            let mut unique: Vec<&str> = Vec::new();
            for label in known { if !unique.contains(&label) { unique.push(label); } }
            return Err(if unique.is_empty() {
                "This transcript has no speaker labels.".into()
            } else {
                format!("No passages are labeled “{from}”. Labels: {}.", unique.join(", "))
            });
        }
        validate_text(&transcript, "Transcript")?;
        record.detail.transcript = transcript;
        if let Some(cleaned) = &record.detail.cleaned_transcript {
            record.detail.cleaned_transcript = Some(rename_label(cleaned, from, to).0);
        }
        self.write(&record)?;
        Ok(record.detail)
    }
}

/// `[0:12] Label: text` → `Label`.
fn passage_label(line: &str) -> Option<&str> {
    let rest = line.strip_prefix('[')?;
    let (clock, rest) = rest.split_once(']')?;
    if clock.is_empty() || !clock.chars().all(|c| c.is_ascii_digit() || c == ':') { return None; }
    let (label, _) = rest.trim_start().split_once(':')?;
    let label = label.trim();
    (!label.is_empty() && label.len() <= 64).then_some(label)
}

fn rename_label(transcript: &str, from: &str, to: &str) -> (String, usize) {
    let mut count = 0;
    let lines: Vec<String> = transcript.split('\n').map(|line| {
        match passage_label(line.trim_end_matches('\r')) {
            Some(label) if label == from => {
                let (head, rest) = line.split_once(']').unwrap();
                let (_, text) = rest.split_once(':').unwrap();
                count += 1;
                format!("{head}] {to}:{text}")
            }
            _ => line.to_owned(),
        }
    }).collect();
    (lines.join("\n"), count)
}

fn clock(seconds: f64) -> String {
    let seconds = seconds.max(0.0).floor() as u64;
    if seconds >= 3600 {
        format!("{}:{:02}:{:02}", seconds / 3600, seconds / 60 % 60, seconds % 60)
    } else {
        format!("{}:{:02}", seconds / 60, seconds % 60)
    }
}

/// Copies each part's PCM data and pads it with silence to the part length.
fn concatenate<'a>(destination: &Path, stems: impl Iterator<Item = &'a Stem>, lengths: &[f64]) -> Result<(), String> {
    let stems: Vec<&Stem> = stems.collect();
    let rate = stems[0].info.sample_rate;
    let targets: Vec<u64> = lengths.iter().zip(&stems)
        .map(|(seconds, stem)| ((seconds * f64::from(rate)).round() as u64).max(stem.frames()))
        .collect();
    let frames: u64 = targets.iter().sum();
    let file = fs::File::create(destination).map_err(|e| format!("Cannot create joined audio: {e}"))?;
    let mut writer = BufWriter::with_capacity(1 << 20, file);
    writer.write_all(&crate::recording::pcm_header(frames, rate).map_err(|e| e.to_string())?)
        .map_err(|e| format!("Cannot write joined audio: {e}"))?;
    for (stem, target) in stems.iter().zip(targets) {
        let mut source = fs::File::open(&stem.path).map_err(|e| format!("Cannot open {}: {e}", stem.path.display()))?;
        source.seek(SeekFrom::Start(stem.info.data_offset)).map_err(|e| e.to_string())?;
        let copied = std::io::copy(&mut source.take(stem.frames() * 2), &mut writer)
            .map_err(|e| format!("Cannot copy {}: {e}", stem.path.display()))?;
        if copied != stem.frames() * 2 {
            return Err(format!("{} ended early.", stem.path.display()));
        }
        let mut padding = (target - stem.frames()) * 2;
        let silence = [0u8; 8192];
        while padding > 0 {
            let chunk = padding.min(silence.len() as u64) as usize;
            writer.write_all(&silence[..chunk]).map_err(|e| format!("Cannot write joined audio: {e}"))?;
            padding -= chunk as u64;
        }
    }
    let file = writer.into_inner().map_err(|e| format!("Cannot write joined audio: {e}"))?;
    file.sync_all().map_err(|e| format!("Cannot save joined audio: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_changes_only_matching_passage_labels() {
        let text = "[0:01] You: Hi.\n\n[0:03] Other speakers: You: is not a label here.\n\n[1:00:02] You: Bye.";
        let (renamed, count) = rename_label(text, "You", "Cody Stevens");
        assert_eq!(count, 2);
        assert_eq!(renamed, "[0:01] Cody Stevens: Hi.\n\n[0:03] Other speakers: You: is not a label here.\n\n[1:00:02] Cody Stevens: Bye.");
        assert_eq!(passage_label("[0:03] Other speakers: x"), Some("Other speakers"));
        assert_eq!(passage_label("Plain: prose"), None);
        assert_eq!(passage_label("[laughter] yes: no"), None);
    }

    #[test]
    fn join_pads_parts_to_their_longest_stem() {
        let directory = std::env::temp_dir().join(format!("sonojot-join-{}", new_id()));
        fs::create_dir_all(&directory).unwrap();
        let write = |name: &str, samples: &[i16]| {
            let path = directory.join(name);
            let mut bytes = crate::recording::pcm_header(samples.len() as u64, 4).unwrap().to_vec();
            for sample in samples { bytes.extend_from_slice(&sample.to_le_bytes()); }
            fs::write(&path, bytes).unwrap();
            Stem::open(path).unwrap()
        };
        let a = write("a.wav", &[1, 2]);
        let b = write("b.wav", &[3]);
        let out = directory.join("out.wav");
        // Part one lasts 1 s (4 frames at 4 Hz), part two 0.5 s.
        concatenate(&out, [&a, &b].into_iter(), &[1.0, 0.5]).unwrap();
        let joined = Stem::open(out.clone()).unwrap();
        assert_eq!(joined.frames(), 6);
        let bytes = fs::read(&out).unwrap();
        let samples: Vec<i16> = bytes[joined.info.data_offset as usize..].chunks_exact(2)
            .map(|pair| i16::from_le_bytes([pair[0], pair[1]])).collect();
        assert_eq!(samples, vec![1, 2, 0, 0, 3, 0]);
        let _ = fs::remove_dir_all(&directory);
    }
}
