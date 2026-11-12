//! Local-only dictation derivatives. Never shared with meeting storage or network code.
use super::{atomic_write, validate_id, Store};
use flacenc::{component::BitRepr, error::Verify};
use serde::Serialize;
use std::{fs, path::PathBuf};
use time::{format_description::well_known::Rfc3339, OffsetDateTime};

#[derive(Debug, Serialize)]
pub struct DictationAudioUsage {
    pub count: usize,
    pub bytes: u64,
}

#[derive(Debug, Serialize)]
pub struct DeletedDictationAudio {
    pub deleted: usize,
}

/// Quantize once before recognition so the kept PCM24 represents exactly the
/// normalized samples sent to ASR, rather than a later approximation of them.
pub(crate) fn prepare_input(samples: &[f32], rate: u32) -> Vec<f32> {
    let mut input = crate::sherpa::resample(samples, rate, 16_000);
    for sample in &mut input {
        *sample = pcm24(*sample) as f32 / 8_388_608.0;
    }
    input
}

fn pcm24(sample: f32) -> i32 {
    (sample.clamp(-1.0, 1.0) * 8_388_608.0)
        .round()
        .clamp(-8_388_608.0, 8_388_607.0) as i32
}

impl Store {
    // Association is the validated record ID, never a path read from JSON.
    pub(super) fn dictation_audio_path(&self, id: &str) -> PathBuf {
        self.root.join("dictations").join(format!("{id}.flac"))
    }

    pub(super) fn save_dictation_audio_unlocked(
        &self,
        id: &str,
        samples: &[f32],
    ) -> Result<(), String> {
        let path = self.dictation_audio_path(id);
        self.check_owned(&path)?;
        // Preserve sample order and timing; quantize normalized floats to PCM24.
        let pcm: Vec<i32> = samples.iter().copied().map(pcm24).collect();
        let config = flacenc::config::Encoder::default()
            .into_verified()
            .map_err(|_| "Invalid dictation FLAC configuration.")?;
        let source = flacenc::source::MemSource::from_samples(&pcm, 1, 24, 16_000);
        let stream = flacenc::encode_with_fixed_block_size(&config, source, config.block_size)
            .map_err(|e| format!("Cannot encode dictation audio: {e}"))?;
        let mut sink = flacenc::bitsink::ByteSink::new();
        stream
            .write(&mut sink)
            .map_err(|e| format!("Cannot encode dictation audio: {e}"))?;
        atomic_write(&path, sink.as_slice())
    }

    fn dictation_audio_paths_unlocked(&self) -> Result<Vec<(String, PathBuf)>, String> {
        let mut paths = Vec::new();
        for entry in fs::read_dir(self.root.join("dictations")).map_err(|e| e.to_string())? {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.extension().and_then(|s| s.to_str()) != Some("flac") {
                continue;
            }
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or("Invalid audio filename.")?;
            validate_id(id)?;
            self.check_owned(&path)?;
            paths.push((id.to_owned(), path));
        }
        Ok(paths)
    }

    pub fn dictation_audio_usage(&self) -> Result<DictationAudioUsage, String> {
        let _guard = self.lock()?;
        self.purge_dictation_audio_unlocked(OffsetDateTime::now_utc())?;
        let mut usage = DictationAudioUsage { count: 0, bytes: 0 };
        for (_, path) in self.dictation_audio_paths_unlocked()? {
            usage.count += 1;
            usage.bytes += fs::metadata(path).map_err(|e| e.to_string())?.len();
        }
        Ok(usage)
    }

    pub fn delete_dictation_audio(&self) -> Result<DeletedDictationAudio, String> {
        let _guard = self.lock()?;
        // In-flight publications cannot recreate audio after Delete all.
        self.invalidate_privacy_unlocked();
        Ok(DeletedDictationAudio {
            deleted: self.delete_dictation_audio_unlocked()?,
        })
    }

    pub(super) fn delete_dictation_audio_unlocked(&self) -> Result<usize, String> {
        let paths = self.dictation_audio_paths_unlocked()?;
        for (_, path) in &paths {
            self.remove_owned_unlocked(path)?;
        }
        Ok(paths.len())
    }

    pub fn purge_dictation_audio(&self) -> Result<usize, String> {
        let _guard = self.lock()?;
        self.purge_dictation_audio_unlocked(OffsetDateTime::now_utc())
    }

    fn purge_dictation_audio_unlocked(&self, now: OffsetDateTime) -> Result<usize, String> {
        let cutoff = now - time::Duration::days(30);
        let mut deleted = 0;
        for (id, path) in self.dictation_audio_paths_unlocked()? {
            let record_path = self.root.join("dictations").join(format!("{id}.json"));
            let expired = if !record_path.exists() {
                true
            } else {
                let record: super::DictationHeader = self.read(&record_path)?;
                if record.id != id {
                    return Err("Mismatched audio history identifier.".into());
                }
                OffsetDateTime::parse(&record.created_at, &Rfc3339)
                    .map_err(|e| format!("Invalid dictation date: {e}"))?
                    <= cutoff
            };
            if expired {
                self.remove_owned_unlocked(&path)?;
                deleted += 1;
            }
        }
        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{RecognitionCompleteness, Settings};

    fn store() -> Store {
        Store::open(Some(
            std::env::temp_dir().join(format!("sonojot-audio-{}", uuid::Uuid::new_v4())),
        ))
        .unwrap()
    }
    fn save(store: &Store, generation: u64) -> Option<crate::domain::DictationRecord> {
        store
            .commit_dictation_with_audio(
                generation,
                "synthetic input",
                "Synthetic input.",
                RecognitionCompleteness::Complete,
                &[],
                None,
                None,
                None,
                None,
                None,
                &[],
                Some(&[0.0, 0.25, -0.25, 0.5, -0.5].repeat(3200)),
            )
            .unwrap()
    }
    fn enabled(store: &Store) {
        store
            .save_settings(&Settings {
                keep_dictation_audio: true,
                ..Settings::default()
            })
            .unwrap();
    }
    #[test]
    fn kept_pcm24_is_exactly_the_input_sent_to_recognition() {
        let input = prepare_input(&[0.0, 0.12345678, -0.9876543, 1.0, -1.0], 16_000);
        assert_eq!(input.len(), 5);
        for sample in input {
            assert_eq!(pcm24(sample) as f32 / 8_388_608.0, sample);
        }
    }

    #[test]
    fn save_links_flac_to_record_and_delete_record_removes_audio() {
        let s = store();
        enabled(&s);
        let record = save(&s, s.privacy_generation()).unwrap();
        let path = s.dictation_audio_path(&record.id);
        let data = fs::read(&path).unwrap();
        assert_eq!(&data[..4], b"fLaC");
        let info = u64::from_be_bytes(data[18..26].try_into().unwrap());
        assert_eq!(info >> 44, 16_000);
        assert_eq!((info >> 41) & 7, 0); // mono
        assert_eq!((info >> 36) & 31, 23); // PCM24
        assert_eq!(info & 0xF_FFFFFFFF, 16_000);
        assert_eq!(s.get_dictation(&record.id).unwrap().raw, "synthetic input");
        assert_eq!(s.dictation_audio_usage().unwrap().count, 1);
        assert!(s.dictation_audio_usage().unwrap().bytes > 0);
        s.delete_dictation(&record.id, true).unwrap();
        assert!(!path.exists());
        fs::remove_dir_all(s.root()).unwrap();
    }
    #[test]
    fn off_and_no_history_and_stale_generation_write_no_audio() {
        let s = store();
        save(&s, s.privacy_generation()).unwrap();
        assert_eq!(s.dictation_audio_usage().unwrap().count, 0);
        enabled(&s);
        let generation = s.privacy_generation();
        s.save_settings(&Settings {
            keep_dictation_audio: true,
            do_not_save_dictations: true,
            ..Settings::default()
        })
        .unwrap();
        assert!(save(&s, generation).is_none());
        assert!(save(&s, s.privacy_generation()).is_none());
        enabled(&s);
        assert!(save(&s, generation).is_none());
        assert_eq!(s.dictation_audio_usage().unwrap().count, 0);
        fs::remove_dir_all(s.root()).unwrap();
    }
    #[test]
    fn purge_at_thirty_days_and_startup_remove_orphans_without_touching_meetings() {
        let s = store();
        enabled(&s);
        let record = save(&s, s.privacy_generation()).unwrap();
        let created = OffsetDateTime::parse(&record.created_at, &Rfc3339).unwrap();
        let meeting = s.root.join("meetings/synthetic.flac");
        fs::write(&meeting, b"synthetic").unwrap();
        assert_eq!(
            s.purge_dictation_audio_unlocked(created + time::Duration::days(29))
                .unwrap(),
            0
        );
        assert_eq!(
            s.purge_dictation_audio_unlocked(created + time::Duration::days(30))
                .unwrap(),
            1
        );
        assert!(s.get_dictation(&record.id).is_ok());
        let record = save(&s, s.privacy_generation()).unwrap();
        fs::remove_file(
            s.root
                .join("dictations")
                .join(format!("{}.json", record.id)),
        )
        .unwrap();
        let reopened = Store::open(Some(s.root().to_path_buf())).unwrap();
        assert_eq!(reopened.dictation_audio_usage().unwrap().count, 0);
        assert!(meeting.exists());
        fs::remove_dir_all(s.root()).unwrap();
    }
    #[test]
    fn delete_all_audio_and_clear_history_prevent_recreation() {
        let s = store();
        enabled(&s);
        save(&s, s.privacy_generation()).unwrap();
        let generation = s.privacy_generation();
        assert_eq!(s.delete_dictation_audio().unwrap().deleted, 1);
        assert_eq!(s.count_dictations().unwrap(), 1);
        assert!(save(&s, generation).is_none());
        let record = save(&s, s.privacy_generation()).unwrap();
        s.delete_all_dictations(true).unwrap();
        assert!(!s.dictation_audio_path(&record.id).exists());
        assert_eq!(s.dictation_audio_usage().unwrap().count, 0);
        assert_eq!(s.count_dictations().unwrap(), 0);
        fs::remove_dir_all(s.root()).unwrap();
    }
}
