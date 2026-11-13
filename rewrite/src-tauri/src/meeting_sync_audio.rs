//! Opt-in compressed audio; no encoder runs against the original recording in place.
use crate::{
    store::Store,
    sync_account::Credentials,
    sync_crypto::{self as crypto, Keyring},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub const CHUNK: usize = 8 * 1024 * 1024;
const OPUS_BITRATE_KBPS: u32 = 64;
const OPUS_VBR: bool = false;
const OPUS_APPLICATION: &str = "audio";
fn encoder_condition() -> String {
    format!(
        "{}k-{}-{}",
        OPUS_BITRATE_KBPS,
        if OPUS_VBR { "vbr" } else { "cbr" },
        OPUS_APPLICATION
    )
}
fn encoding_codec() -> String {
    format!("ogg-opus-mono-{}", OPUS_BITRATE_KBPS * 1000)
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    pub id: String,
    pub sha256: String,
    pub bytes: usize,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Stem {
    pub source: String,
    pub chunks: Vec<Chunk>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct Manifest {
    pub schema: u32,
    pub codec: String,
    pub stems: Vec<Stem>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub audio_default: bool,
    pub always_download: bool,
}
pub fn preferences(store: &Store) -> Result<Preferences, String> {
    let path = store.root().join("meeting-sync-preferences.json");
    if path.exists() {
        store.read(&path)
    } else {
        Ok(Preferences::default())
    }
}
pub fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub fn directory(store: &Store, id: &str) -> Result<PathBuf, String> {
    crate::store::validate_id(id)?;
    let parent = store.root().join("meeting-sync-audio");
    store.check_owned(&parent)?;
    fs::create_dir_all(&parent).map_err(|_| "Cannot create the audio cache.")?;
    let dir = parent.join(id);
    store.check_owned(&dir)?;
    fs::create_dir_all(&dir).map_err(|_| "Cannot create the synced audio cache.")?;
    Ok(dir)
}
pub(crate) fn remove_local(store: &Store, id: &str) -> Result<(), String> {
    crate::store::validate_id(id)?;
    let dir = store.root().join("meeting-sync-audio").join(id);
    if !dir.exists() {
        return Ok(());
    }
    store.check_owned(&dir)?;
    for entry in fs::read_dir(&dir).map_err(|_| "Cannot inspect the meeting audio cache.")? {
        let path = entry
            .map_err(|_| "Cannot inspect the meeting audio cache.")?
            .path();
        store.check_owned(&path)?;
        if !path.is_file() {
            return Err("Unexpected folder in the meeting audio cache.".into());
        }
        fs::remove_file(path).map_err(|_| "Cannot remove cached meeting audio.")?;
    }
    fs::remove_dir(dir).map_err(|_| "Cannot remove the meeting audio cache.".into())
}
fn run(ffmpeg: &Path, args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg);
    cmd.args([
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-protocol_whitelist",
        "file",
    ])
    .args(args)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child =
        crate::child_lifetime::spawn(&mut cmd).map_err(|_| "Cannot start the audio converter.")?;
    let start = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| "Cannot check audio conversion.")?
        {
            return if status.success() {
                Ok(())
            } else {
                Err("Audio conversion failed. Choose FFmpeg with libopus support in Settings; the original recording is safe.".into())
            };
        }
        if start.elapsed() > Duration::from_secs(1800) {
            #[cfg(windows)]
            {
                let mut kill = Command::new("taskkill");
                use std::os::windows::process::CommandExt;
                kill.args(["/T", "/F", "/PID", &child.id().to_string()])
                    .creation_flags(0x08000000)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null());
                let _ = kill.status();
            }
            #[cfg(not(windows))]
            let _ = crate::child_lifetime::kill(&mut child);
            let _ = child.wait();
            return Err("Audio conversion timed out. The original recording is safe.".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
pub fn encode(ffmpeg: &Path, input: &Path, output: &Path) -> Result<(), String> {
    let bitrate = format!("{OPUS_BITRATE_KBPS}k");
    run(
        ffmpeg,
        &[
            "-i".as_ref(),
            input.as_os_str(),
            "-map_metadata".as_ref(),
            "-1".as_ref(),
            "-vn".as_ref(),
            "-ac".as_ref(),
            "1".as_ref(),
            "-ar".as_ref(),
            "48000".as_ref(),
            "-c:a".as_ref(),
            "libopus".as_ref(),
            "-b:a".as_ref(),
            bitrate.as_ref(),
            "-vbr".as_ref(),
            (if OPUS_VBR { "on" } else { "off" }).as_ref(),
            "-application".as_ref(),
            OPUS_APPLICATION.as_ref(),
            "-f".as_ref(),
            "ogg".as_ref(),
            output.as_os_str(),
        ],
    )
}
pub fn decode(ffmpeg: &Path, input: &Path, output: &Path) -> Result<(), String> {
    run(
        ffmpeg,
        &[
            "-f".as_ref(),
            "ogg".as_ref(),
            "-i".as_ref(),
            input.as_os_str(),
            "-vn".as_ref(),
            "-ac".as_ref(),
            "1".as_ref(),
            "-ar".as_ref(),
            "16000".as_ref(),
            "-c:a".as_ref(),
            "pcm_s16le".as_ref(),
            "-f".as_ref(),
            "wav".as_ref(),
            output.as_os_str(),
        ],
    )
}
pub fn split(input: &Path, dir: &Path) -> Result<Vec<Chunk>, String> {
    let mut file = fs::File::open(input).map_err(|_| "Cannot read compressed meeting audio.")?;
    let mut chunks = Vec::new();
    loop {
        let mut bytes = Vec::with_capacity(CHUNK);
        Read::by_ref(&mut file)
            .take(CHUNK as u64)
            .read_to_end(&mut bytes)
            .map_err(|_| "Cannot read audio chunk.")?;
        if bytes.is_empty() {
            break;
        }
        let id = uuid::Uuid::new_v4().to_string();
        crate::store::atomic_write(&dir.join(&id), &bytes)?;
        chunks.push(Chunk {
            id,
            sha256: hash(&bytes),
            bytes: bytes.len(),
        });
    }
    Ok(chunks)
}
impl Manifest {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema != 1
            || (self.codec != encoding_codec() && self.codec != "ogg-opus-mono-24000")
            || self.stems.is_empty()
            || self.stems.len() > 2
        {
            return Err("Unsupported meeting audio format.".into());
        }
        let mut sources = std::collections::BTreeSet::new();
        let mut ids = std::collections::BTreeSet::new();
        for stem in &self.stems {
            if !["mixed", "microphone", "system"].contains(&stem.source.as_str())
                || !sources.insert(&stem.source)
                || stem.chunks.is_empty()
                || stem.chunks.len() > 12000
            {
                return Err("Invalid synced audio stem.".into());
            }
            for chunk in &stem.chunks {
                crate::store::validate_id(&chunk.id)?;
                if !ids.insert(&chunk.id)
                    || chunk.bytes == 0
                    || chunk.bytes > CHUNK
                    || chunk.sha256.len() != 64
                    || !chunk.sha256.bytes().all(|b| b.is_ascii_hexdigit())
                {
                    return Err("Invalid synced audio chunk.".into());
                }
            }
        }
        Ok(())
    }
    pub fn complete(&self, dir: &Path) -> bool {
        self.validate().is_ok()
            && self
                .stems
                .iter()
                .all(|stem| stem.chunks.iter().all(|chunk| verified(dir, chunk).is_ok()))
    }
}
fn verified(dir: &Path, chunk: &Chunk) -> Result<Vec<u8>, String> {
    let path = dir.join(&chunk.id);
    if fs::metadata(&path)
        .map_err(|_| "Audio chunk is not on this computer.")?
        .len()
        != chunk.bytes as u64
    {
        return Err("Audio chunk length differs.".into());
    }
    let bytes = fs::read(path).map_err(|_| "Cannot read synced audio.")?;
    if !matches_chunk(chunk, &bytes) {
        return Err("Audio chunk checksum differs.".into());
    }
    Ok(bytes)
}
fn matches_chunk(chunk: &Chunk, bytes: &[u8]) -> bool {
    bytes.len() == chunk.bytes && hash(bytes) == chunk.sha256
}
pub fn prepare(meetings: &crate::meetings::Meetings, id: &str) -> Result<Manifest, String> {
    let dir = directory(meetings.store(), id)?;
    let ffmpeg = crate::meetings::resolve_ffmpeg(meetings.store(), None)?;
    let sources = if meetings.select_audio(id, "microphone").is_ok()
        && meetings.select_audio(id, "system").is_ok()
    {
        vec!["microphone", "system"]
    } else {
        vec!["mixed"]
    };
    let mut signature = Sha256::new();
    // Resume only encodes made with the same format, resampler and settings.
    signature.update(b"ogg-opus/mono/48000/");
    signature.update(encoder_condition().as_bytes());
    for source in &sources {
        let input = meetings.select_audio(id, source)?;
        let metadata = fs::metadata(input).map_err(|_| "Cannot inspect source audio.")?;
        signature.update(source.as_bytes());
        signature.update(metadata.len().to_be_bytes());
        signature.update(
            metadata
                .modified()
                .map_err(|_| "Cannot inspect audio timestamp.")?
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
                .to_be_bytes(),
        );
    }
    let signature = format!("{:x}", signature.finalize());
    let pending = dir.join("prepared.json");
    let mut stems = if pending.exists() {
        let prior: Prepared = meetings.store().read(&pending)?;
        if prior.signature == signature && prior.manifest.complete(&dir) {
            prior.manifest.stems
        } else {
            vec![]
        }
    } else {
        vec![]
    };
    for source in sources {
        if stems.iter().any(|s| s.source == source) {
            continue;
        }
        let input = meetings.select_audio(id, source)?;
        let output = dir.join(format!("{source}.encoding.ogg"));
        let _temporary = Temporary(output.clone());
        encode(&ffmpeg, &input, &output)?;
        let chunks = split(&output, &dir)?;
        fs::remove_file(output).map_err(|_| "Cannot remove temporary encoded audio.")?;
        stems.push(Stem {
            source: source.into(),
            chunks,
        });
        meetings.store().write(
            &pending,
            &Prepared {
                signature: signature.clone(),
                manifest: Manifest {
                    schema: 1,
                    codec: encoding_codec(),
                    stems: stems.clone(),
                },
            },
        )?;
    }
    Ok(Manifest {
        schema: 1,
        codec: encoding_codec(),
        stems,
    })
}
struct Temporary(PathBuf);
impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.0);
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Prepared {
    signature: String,
    manifest: Manifest,
}

pub fn renew_expired(
    store: &Store,
    id: &str,
    manifest: &mut Manifest,
    expired: &std::collections::BTreeSet<String>,
) -> Result<bool, String> {
    let dir = directory(store, id)?;
    let mut changed = false;
    for chunk in manifest.stems.iter_mut().flat_map(|s| &mut s.chunks) {
        if !expired.contains(&chunk.id) {
            continue;
        }
        let bytes = verified(&dir, chunk)?;
        let replacement = uuid::Uuid::new_v4().to_string();
        crate::store::atomic_write(&dir.join(&replacement), &bytes)?;
        chunk.id = replacement;
        changed = true;
    }
    Ok(changed)
}
pub fn cleanup_cache(
    store: &Store,
    id: &str,
    manifest: Option<&Manifest>,
    cancel_pending: bool,
) -> Result<(), String> {
    let dir = directory(store, id)?;
    let mut keep = std::collections::BTreeSet::new();
    if let Some(manifest) = manifest {
        keep.extend(
            manifest
                .stems
                .iter()
                .flat_map(|s| &s.chunks)
                .map(|c| c.id.clone()),
        );
    }
    let pending = dir.join("prepared.json");
    if pending.exists() {
        store.check_owned(&pending)?;
        if cancel_pending {
            fs::remove_file(&pending).map_err(|_| "Cannot clear pending audio.")?;
        } else {
            let prepared: Prepared = store.read(&pending)?;
            keep.extend(
                prepared
                    .manifest
                    .stems
                    .iter()
                    .flat_map(|s| &s.chunks)
                    .map(|c| c.id.clone()),
            );
        }
    }
    for entry in fs::read_dir(&dir).map_err(|_| "Cannot inspect cached audio.")? {
        let path = entry.map_err(|_| "Cannot inspect cached audio.")?.path();
        store.check_owned(&path)?;
        let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
            continue;
        };
        let temporary = name.ends_with(".encoding.ogg")
            || name.ends_with(".download.ogg")
            || name.ends_with(".download.wav");
        if !keep.contains(name)
            && (uuid::Uuid::parse_str(name).is_ok() || temporary)
            && path.is_file()
        {
            let old = fs::metadata(&path)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age.as_secs() >= 86400);
            if cancel_pending || old {
                fs::remove_file(path).map_err(|_| "Cannot clear unused audio chunks.")?;
            }
        }
    }
    Ok(())
}
pub async fn upload(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    meeting: &str,
    id: &str,
    manifest: &Manifest,
    known: &mut std::collections::BTreeSet<String>,
) -> Result<(), String> {
    manifest.validate()?;
    let dir = directory(store, id)?;
    let client = c.client()?;
    for stem in &manifest.stems {
        for chunk in &stem.chunks {
            let object = format!("blob/{meeting}/{}", chunk.id);
            if known.contains(&object) {
                continue;
            }
            let bytes = verified(&dir, chunk)?;
            let sealed = crypto::seal(
                ring.keys.get(&ring.active).ok_or("Missing audio key.")?,
                ring.active,
                &c.account,
                &object,
                1,
                &bytes,
            )?;
            let path = format!("/v1/blobs/{meeting}/{}", chunk.id);
            // A lost receipt must not resend an 8 MiB body just to get an early 412.
            // Authenticate the existing immutable chunk before accepting it as resumed.
            match client
                .request(Method::GET, &path, None, None, None, 0)
                .await
            {
                Ok((rev, body)) => {
                    let kid = crypto::envelope_kid(&body)?;
                    let plain = crypto::open(
                        ring.object_key(&object, kid, rev)?,
                        &c.account,
                        &object,
                        rev,
                        &body,
                    )?;
                    if rev != 1 || plain.as_slice() != bytes {
                        return Err("An immutable audio chunk changed.".into());
                    }
                    known.insert(object);
                    continue;
                }
                Err(e) if e.status == 404 => {}
                Err(e) => return Err(e.into()),
            }
            match client
                .request(Method::PUT, &path, None, Some(sealed), Some(0), ring.epoch)
                .await
            {
                Ok(_) => {}
                Err(e) if e.status == 412 => {
                    let (rev, body) = client
                        .request(Method::GET, &path, None, None, None, 0)
                        .await?;
                    let kid = crypto::envelope_kid(&body)?;
                    let plain = crypto::open(
                        ring.object_key(&object, kid, rev)?,
                        &c.account,
                        &object,
                        rev,
                        &body,
                    )?;
                    if plain.as_slice() != bytes {
                        return Err("An immutable audio chunk changed.".into());
                    }
                }
                Err(e) => return Err(e.into()),
            }
            known.insert(object);
        }
    }
    Ok(())
}
pub async fn download(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    meeting: &str,
    id: &str,
    manifest: &Manifest,
) -> Result<(), String> {
    manifest.validate()?;
    let dir = directory(store, id)?;
    let client = c.client()?;
    for stem in &manifest.stems {
        for chunk in &stem.chunks {
            if verified(&dir, chunk).is_ok() {
                continue;
            }
            let object = format!("blob/{meeting}/{}", chunk.id);
            let (rev, body) = client
                .request(
                    Method::GET,
                    &format!("/v1/blobs/{meeting}/{}", chunk.id),
                    None,
                    None,
                    None,
                    0,
                )
                .await?;
            let kid = crypto::envelope_kid(&body)?;
            let plain = crypto::open(
                ring.object_key(&object, kid, rev)?,
                &c.account,
                &object,
                rev,
                &body,
            )?;
            if rev != 1 || !matches_chunk(chunk, &plain) {
                return Err("Synced audio could not be verified.".into());
            }
            crate::store::atomic_write(&dir.join(&chunk.id), &plain)?;
        }
    }
    Ok(())
}
pub fn materialize(store: &Store, id: &str, manifest: &Manifest) -> Result<(), String> {
    manifest.validate()?;
    let dir = directory(store, id)?;
    let ffmpeg = crate::meetings::resolve_ffmpeg(store, None)?;
    let meetings = crate::meetings::Meetings::new(store.clone());
    let paths = meetings.recording_paths(id)?;
    for stem in &manifest.stems {
        let dest = match stem.source.as_str() {
            "microphone" => &paths.microphone,
            "system" => &paths.system,
            _ => &paths.mixed,
        };
        if dest.is_file() {
            continue;
        }
        let ogg = dir.join(format!("{}.download.ogg", stem.source));
        let mut output = fs::File::create(&ogg).map_err(|_| "Cannot assemble synced audio.")?;
        for chunk in &stem.chunks {
            output
                .write_all(&verified(&dir, chunk)?)
                .map_err(|_| "Cannot assemble synced audio.")?;
        }
        output.sync_all().map_err(|_| "Cannot save synced audio.")?;
        drop(output);
        let wav = dir.join(format!("{}.download.wav", stem.source));
        decode(&ffmpeg, &ogg, &wav)?;
        fs::rename(&wav, dest).map_err(|_| "Cannot publish downloaded audio.")?;
        fs::remove_file(ogg).map_err(|_| "Cannot remove temporary audio.")?;
    }
    if !paths.mixed.exists() && paths.microphone.exists() && paths.system.exists() {
        let wav = dir.join("mixed.download.wav");
        run(
            &ffmpeg,
            &[
                "-f".as_ref(),
                "wav".as_ref(),
                "-i".as_ref(),
                paths.microphone.as_os_str(),
                "-f".as_ref(),
                "wav".as_ref(),
                "-i".as_ref(),
                paths.system.as_os_str(),
                "-filter_complex".as_ref(),
                "amix=inputs=2:duration=longest:normalize=0".as_ref(),
                "-ac".as_ref(),
                "1".as_ref(),
                "-ar".as_ref(),
                "16000".as_ref(),
                "-c:a".as_ref(),
                "pcm_s16le".as_ref(),
                "-f".as_ref(),
                "wav".as_ref(),
                wav.as_os_str(),
            ],
        )?;
        fs::rename(wav, paths.mixed).map_err(|_| "Cannot publish downloaded mixed audio.")?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "Set SONOJOT_FFMPEG to a standalone FFmpeg executable; synthetic audio only"]
    fn meeting_opus_decoder_rejects_format_spoofing() {
        let ffmpeg = PathBuf::from(std::env::var_os("SONOJOT_FFMPEG").expect("SONOJOT_FFMPEG"));
        let root = std::env::temp_dir().join(format!("sj-opus-format-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let input = root.join("synthetic.wav");
        let mut writer = hound::WavWriter::create(
            &input,
            hound::WavSpec {
                channels: 1,
                sample_rate: 16000,
                bits_per_sample: 16,
                sample_format: hound::SampleFormat::Int,
            },
        )
        .unwrap();
        for i in 0..16000 {
            writer
                .write_sample(((i as f32 * 0.09).sin() * 1000.0) as i16)
                .unwrap();
        }
        writer.finalize().unwrap();
        let ogg = root.join("synthetic.ogg");
        let output = root.join("decoded.wav");
        encode(&ffmpeg, &input, &ogg).unwrap();
        decode(&ffmpeg, &ogg, &output).unwrap();
        assert_eq!(
            hound::WavReader::open(&output).unwrap().spec().sample_rate,
            16000
        );
        // Autodetection would accept both a WAV with an Ogg extension and a
        // concat playlist. Explicit demuxing must reject both at the boundary.
        fs::copy(&input, &ogg).unwrap();
        assert!(decode(&ffmpeg, &ogg, &output).is_err());
        fs::write(&ogg, b"ffconcat version 1.0\nfile 'synthetic.wav'\n").unwrap();
        assert!(decode(&ffmpeg, &ogg, &output).is_err());
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn meeting_retained_key_cannot_forge_audio_bound_by_an_authenticated_head() {
        let mut ring = Keyring::new();
        let old = ring.active;
        ring.rotate().unwrap();
        let chunk = Chunk {
            id: "synthetic".into(),
            bytes: 5,
            sha256: hash(b"voice"),
        };
        let bytes = crypto::seal(
            &ring.keys[&old],
            old,
            "synthetic",
            "blob/meeting/synthetic",
            1,
            b"false",
        )
        .unwrap();
        let plain = crypto::open(
            ring.object_key("blob/meeting/synthetic", old, 1).unwrap(),
            "synthetic",
            "blob/meeting/synthetic",
            1,
            &bytes,
        )
        .unwrap();
        assert!(!matches_chunk(&chunk, &plain));
        assert!(!matches_chunk(&chunk, b"voice-extra"));
        assert!(matches_chunk(&chunk, b"voice"));
    }
    #[test]
    fn meeting_expired_audio_renews_ids_and_cancel_cleans_only_unreferenced_cache() {
        let root = std::env::temp_dir().join(format!("sj-audio-renew-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let dir = directory(&store, "synthetic").unwrap();
        let old = uuid::Uuid::new_v4().to_string();
        fs::write(dir.join(&old), b"synthetic encoded audio").unwrap();
        let mut manifest = Manifest {
            schema: 1,
            codec: "ogg-opus-mono-24000".into(),
            stems: vec![Stem {
                source: "mixed".into(),
                chunks: vec![Chunk {
                    id: old.clone(),
                    sha256: hash(b"synthetic encoded audio"),
                    bytes: 23,
                }],
            }],
        };
        manifest.stems[0].chunks[0].bytes = b"synthetic encoded audio".len();
        assert!(renew_expired(&store, "synthetic", &mut manifest, &[old.clone()].into()).unwrap());
        assert_ne!(manifest.stems[0].chunks[0].id, old);
        assert!(manifest.complete(&dir));
        cleanup_cache(&store, "synthetic", Some(&manifest), true).unwrap();
        assert!(!dir.join(old).exists());
        assert!(manifest.complete(&dir));
        fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "Set SONOJOT_OPUS_REPORT to the aggregate report from evaluation/meeting-audio-opus-2026-11/evaluate.py"]
    fn real_audio_opus_wer() {
        let path = std::env::var_os("SONOJOT_OPUS_REPORT")
            .expect("SONOJOT_OPUS_REPORT is required; recognition-as-reference is not accepted");
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(value["model"], "SonoJot Parakeet ft2 int8");
        assert_eq!(value["normalizer"], "Whisper EnglishTextNormalizer");
        let condition = encoder_condition();
        let public = &value["datasets"]["public"];
        let owner = &value["datasets"]["owner"];
        assert_eq!(public["reference_type"], "human");
        assert!(owner["reference_type"]
            .as_str()
            .unwrap()
            .starts_with("pseudo-reference"));
        for dataset in [public, owner] {
            assert!(dataset["utterances"].as_u64().unwrap() >= 2);
            assert!(dataset["reference_words"].as_u64().unwrap() > 0);
            assert!(dataset["bootstrap_iterations"].as_u64().unwrap() >= 1000);
        }
        let arm = |dataset: &serde_json::Value| {
            dataset["conditions"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["condition"] == condition.as_str())
                .unwrap()
                .clone()
        };
        let primary = arm(public);
        let personal = arm(owner);
        assert!(
            primary["delta_pp"].as_f64().unwrap() <= 0.3,
            "Public human-reference WER exceeds the +0.3 pp bound"
        );
        assert!(
            personal["delta_ci"][0].as_f64().unwrap() <= 0.0,
            "Owner's paired CI establishes a degradation"
        );
    }
    #[test]
    fn chunk_boundaries_resume_and_authenticated_envelopes() {
        let root = std::env::temp_dir().join(format!("sj-audio-chunks-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        let input = root.join("synthetic");
        fs::write(&input, vec![42; CHUNK + 123]).unwrap();
        let chunks = split(&input, &root).unwrap();
        assert_eq!(
            chunks.iter().map(|c| c.bytes).collect::<Vec<_>>(),
            vec![CHUNK, 123]
        );
        let manifest = Manifest {
            schema: 1,
            codec: "ogg-opus-mono-24000".into(),
            stems: vec![Stem {
                source: "mixed".into(),
                chunks: chunks.clone(),
            }],
        };
        assert!(manifest.complete(&root));
        let key = crypto::Secret::random();
        let plain = verified(&root, &chunks[0]).unwrap();
        let sealed = crypto::seal(&key, 1, "synthetic", "blob/meeting/chunk", 1, &plain).unwrap();
        assert_eq!(sealed.len(), crypto::MAX_BLOB_ENVELOPE);
        assert_eq!(
            *crypto::open(&key, "synthetic", "blob/meeting/chunk", 1, &sealed).unwrap(),
            plain
        );
        assert!(crypto::open(&key, "synthetic", "blob/meeting/other", 1, &sealed).is_err());
        fs::write(root.join(&chunks[1].id), b"corrupt").unwrap();
        assert!(!manifest.complete(&root));
        assert!(verified(&root, &chunks[0]).is_ok());
        fs::remove_dir_all(root).unwrap();
    }
}
