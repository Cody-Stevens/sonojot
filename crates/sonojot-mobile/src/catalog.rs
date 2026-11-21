//! The model packs phones download: one per language group, every file pinned by Hugging Face
//! revision, size and SHA-256. English uses SonoJot Parakeet, as the SonoJot apps do; every other
//! language uses Whisper Small, because Parakeet is English only.
//!
//! The pins are the desktop's own (`rewrite/src-tauri/src/optional_asr.rs` for SonoJot Parakeet,
//! `rewrite/src-tauri/src/whisper_catalog.rs` for Whisper); a test fails if they disagree, so
//! re-pin both together. Apps download with their own HTTP stack (only from
//! [`DOWNLOAD_HOSTS`], over HTTPS), hash while downloading, then call [`finalize`] on the staging
//! directory, which writes `manifest.json` and verifies every file before the pack is moved into
//! place.

use crate::{
    language_of, manifest::VerificationReport, EngineError, ModelFamily, ModelOptions, ModelPackManifest, PackFile,
    RuntimeRequirement, VerifyMode, MANIFEST_FILE, MANIFEST_SCHEMA, SHERPA_ONNX_ABI,
};
use serde::Serialize;
use std::{
    fs::OpenOptions,
    io::Write,
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};

/// Hosts a download may start at or be redirected to (the desktop allows the same). Entries
/// starting with a dot match any subdomain; the others match exactly. Downloads use HTTPS only and
/// follow at most [`MAX_REDIRECTS`] redirects, each of which must land on an allowed host.
pub const DOWNLOAD_HOSTS: [&str; 5] = ["huggingface.co", ".huggingface.co", ".hf.co", "cas-bridge.xethub.hf.co", ".xethub.hf.co"];

/// The most redirects one download may follow.
pub const MAX_REDIRECTS: u32 = 8;

const WHISPER_LICENSE: &str = "MIT (OpenAI Whisper weights)";

/// One downloadable file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogFile {
    pub role: &'static str,
    /// Name inside the pack directory.
    pub path: &'static str,
    /// Name in the Hugging Face repository.
    pub remote: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
}

/// One downloadable pack.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct CatalogPack {
    pub id: &'static str,
    pub display_name: &'static str,
    /// Languages this pack is the choice for, or `*` for every language without its own pack.
    pub languages: &'static [&'static str],
    pub family: ModelFamily,
    pub repo: &'static str,
    pub revision: &'static str,
    pub files: &'static [CatalogFile],
    pub license: &'static str,
    /// The manifest's `options.model_type` (`nemo_transducer` for Parakeet).
    pub model_type: Option<&'static str>,
}

/// Every pack phones may download. Changing an entry's files means a new `id`, so installed
/// copies of the old pack are recognised as outdated rather than silently reinterpreted.
pub const CATALOG: &[CatalogPack] = &[
    // SonoJot Parakeet v1: SonoJot's fine-tune of NVIDIA Parakeet TDT 0.6B v2, the English model
    // of the SonoJot apps. Same sherpa-onnx int8 export and revision as the desktop.
    CatalogPack {
        id: "sonojot-parakeet-v1-int8",
        display_name: "SonoJot Parakeet (English)",
        languages: &["en"],
        family: ModelFamily::Transducer,
        repo: "SonoJot/sonojot-parakeet-v1",
        revision: "f6c3de766e2667c59ccad189c1d7469a07738340",
        files: &[
            CatalogFile { role: "encoder", path: "encoder.int8.onnx", remote: "encoder.int8.onnx", bytes: 652_808_550, sha256: "6ee0399477f54dcd2ef84e32a66e62d43b5ed9c3e326847ce59d29a8e0063ed7" },
            CatalogFile { role: "decoder", path: "decoder.int8.onnx", remote: "decoder.int8.onnx", bytes: 7_257_753, sha256: "eb10992246617a09f8752a1b4b0a951fd99499740ca8ef9eeffe83adb0fc3940" },
            CatalogFile { role: "joiner", path: "joiner.int8.onnx", remote: "joiner.int8.onnx", bytes: 1_739_080, sha256: "fcd749d846047b8ac98b41eaf0044b678e41503523e3ea9df039f863c1a1014f" },
            CatalogFile { role: "tokens", path: "tokens.txt", remote: "tokens.txt", bytes: 9_384, sha256: "ec182b70dd42113aff6c5372c75cac58c952443eb22322f57bbd7f53977d497d" },
        ],
        license: "CC BY 4.0 (SonoJot Parakeet v1, a fine-tune of NVIDIA Parakeet TDT 0.6B v2)",
        model_type: Some("nemo_transducer"),
    },
    // Whisper Base is too weak for Arabic; Small is the smallest checkpoint usable there.
    CatalogPack {
        id: "whisper-small-int8",
        display_name: "Whisper Small (multilingual)",
        languages: &["*"],
        family: ModelFamily::Whisper,
        repo: "csukuangfj/sherpa-onnx-whisper-small",
        revision: "8f3c18b358db4d1f2fc1eae49d75cd20989e4309",
        files: &[
            CatalogFile { role: "encoder", path: "encoder.int8.onnx", remote: "small-encoder.int8.onnx", bytes: 112_442_483, sha256: "4cbe7b22fa9026b843b60a68640c747de05bafb1a11b57edc0e66c232d9f33a9" },
            CatalogFile { role: "decoder", path: "decoder.int8.onnx", remote: "small-decoder.int8.onnx", bytes: 262_226_114, sha256: "acad50b5c782696e91b55914cc5ab4f756f1532f76e22aa6fc615f39fb69a8ee" },
            CatalogFile { role: "tokens", path: "tokens.txt", remote: "small-tokens.txt", bytes: 816_730, sha256: "b34b360dbb493e781e479794586d661700670d65564001f23024971d1f2fa126" },
        ],
        license: WHISPER_LICENSE,
        model_type: None,
    },
];

impl CatalogFile {
    /// The pinned download URL.
    pub fn url(&self, pack: &CatalogPack) -> String {
        format!("https://huggingface.co/{}/resolve/{}/{}", pack.repo, pack.revision, self.remote)
    }
}

impl CatalogPack {
    pub fn bytes(&self) -> u64 {
        self.files.iter().map(|f| f.bytes).sum()
    }

    /// The manifest an installed copy of this pack carries.
    pub fn manifest(&self) -> ModelPackManifest {
        ModelPackManifest {
            schema: MANIFEST_SCHEMA,
            id: self.id.into(),
            version: self.revision.into(),
            family: self.family,
            languages: self.languages.iter().map(|l| l.to_string()).collect(),
            runtime: RuntimeRequirement { kind: "sherpa-onnx".into(), min_version: SHERPA_ONNX_ABI.into() },
            options: ModelOptions { model_type: self.model_type.map(Into::into), ..ModelOptions::default() },
            files: self
                .files
                .iter()
                .map(|f| PackFile { role: f.role.into(), path: f.path.into(), bytes: f.bytes, sha256: f.sha256.into() })
                .collect(),
            display_name: Some(self.display_name.into()),
            license: Some(self.license.into()),
            source: Some(format!("https://huggingface.co/{}/tree/{}", self.repo, self.revision)),
        }
    }
}

pub fn find(id: &str) -> Option<&'static CatalogPack> {
    CATALOG.iter().find(|p| p.id == id)
}

/// The pack to download for `language` (a language or locale tag such as `nb-NO`): one naming
/// it, else the multilingual pack.
pub fn pack_for(language: &str) -> Option<&'static CatalogPack> {
    let language = language_of(language);
    let language = language.as_str();
    if language.is_empty() {
        return None;
    }
    CATALOG
        .iter()
        .find(|p| p.languages.contains(&language))
        .or_else(|| CATALOG.iter().find(|p| p.languages.contains(&"*")))
}

/// Whether `host` may serve a download (exact names, or suffixes starting with a dot).
pub fn download_host_allowed(host: &str) -> bool {
    DOWNLOAD_HOSTS.iter().any(|allowed| if allowed.starts_with('.') { host.ends_with(allowed) } else { host == *allowed })
}

/// Writes `manifest.json` for catalog pack `id` into `dir` (which holds the downloaded files)
/// and verifies every file's size and SHA-256. Only then may the app move `dir` into place.
///
/// `dir` must be in the app's private storage: the files are checked, then read by native code.
pub fn finalize(dir: &Path, id: &str) -> Result<VerificationReport, EngineError> {
    let pack = find(id).ok_or_else(|| EngineError::InvalidArgument(format!("{id} is not in the catalog")))?;
    install_manifest(dir, &pack.manifest())
}

/// Verifies `manifest`'s files in `dir`, then writes it as `dir/manifest.json`.
fn install_manifest(dir: &Path, manifest: &ModelPackManifest) -> Result<VerificationReport, EngineError> {
    manifest.validate()?;
    let report = manifest.verify(dir, VerifyMode::Full)?;
    write_manifest(dir, manifest)?;
    Ok(report)
}

/// Writes a new file beside `manifest.json` and renames it into place. The rename replaces
/// whatever is there, so a planted symbolic link is replaced rather than written through, and a
/// failure part-way never leaves a truncated manifest.
fn write_manifest(dir: &Path, manifest: &ModelPackManifest) -> Result<(), EngineError> {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let temp = dir.join(format!(".{MANIFEST_FILE}.{}.{}.tmp", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
    let mut created = false;
    let result = (|| {
        // create_new refuses an existing path, including a symbolic link.
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
        created = true;
        file.write_all((manifest.to_json() + "\n").as_bytes())?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, dir.join(MANIFEST_FILE))
    })();
    if result.is_err() && created {
        let _ = std::fs::remove_file(&temp);
    }
    result.map_err(|e| EngineError::PackInvalid(format!("cannot write {MANIFEST_FILE} in {}: {e}", dir.display())))
}

/// The catalog as the C ABI returns it: every pack with its download URLs and total size.
pub fn catalog_json() -> String {
    let packs: Vec<_> = CATALOG
        .iter()
        .map(|pack| {
            serde_json::json!({
                "id": pack.id,
                "display_name": pack.display_name,
                "languages": pack.languages,
                "family": pack.family,
                "bytes": pack.bytes(),
                "license": pack.license,
                "files": pack.files.iter().map(|f| serde_json::json!({
                    "role": f.role, "path": f.path, "url": f.url(pack), "bytes": f.bytes, "sha256": f.sha256,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    serde_json::json!({ "download_hosts": DOWNLOAD_HOSTS, "max_redirects": MAX_REDIRECTS, "packs": packs }).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::tests::TempDir;

    #[test]
    fn every_pack_is_a_valid_manifest() {
        for pack in CATALOG {
            pack.manifest().validate().unwrap_or_else(|e| panic!("{}: {e}", pack.id));
            assert!(pack.files.iter().all(|f| f.url(pack).starts_with("https://huggingface.co/")));
        }
        let ids: std::collections::HashSet<_> = CATALOG.iter().map(|p| p.id).collect();
        assert_eq!(ids.len(), CATALOG.len());
    }

    /// One desktop file pin: (local name, bytes, sha256).
    type DesktopFile = (String, u64, String);
    /// One desktop model: (repo, revision, url_name of each file).
    type DesktopModel = (String, String, Vec<String>);

    fn hex_consts(source: &str) -> std::collections::HashMap<String, String> {
        regex::Regex::new(r#"const ([A-Z0-9_]+): &str = "([0-9a-f]{40}|[0-9a-f]{64})";"#)
            .unwrap()
            .captures_iter(source)
            .map(|c| (c[1].to_string(), c[2].to_string()))
            .collect()
    }

    /// Every entry must parse; a format change fails here rather than skipping pins.
    fn assert_all_parsed(source: &str, kind: &str, parsed: usize) {
        assert_eq!(parsed, source.matches(&format!("\n    {kind} {{")).count(), "unparsed desktop {kind} entries");
    }

    /// The desktop's Whisper checkpoints and their files by url_name, from `whisper_catalog.rs`.
    fn desktop_whisper() -> (Vec<DesktopModel>, std::collections::HashMap<String, DesktopFile>) {
        use regex::Regex;
        let source = include_str!("../../../rewrite/src-tauri/src/whisper_catalog.rs");
        let consts = hex_consts(source);
        let file = Regex::new(
            r#"WhisperFile \{\s*url_name: "([^"]+)",\s*local_name: "([^"]+)",\s*bytes: ([0-9_]+),\s*sha256: (?:"([0-9a-f]{64})"|([A-Z0-9_]+)),\s*\}"#,
        )
        .unwrap();
        let mut files = std::collections::HashMap::new();
        let mut lists = std::collections::HashMap::new();
        for list in Regex::new(r"(?s)const ([A-Z0-9_]+): &\[WhisperFile\] = &\[(.*?)\n\];").unwrap().captures_iter(source) {
            let mut names = Vec::new();
            for c in file.captures_iter(&list[2]) {
                let sha = c.get(4).map(|m| m.as_str().to_string()).unwrap_or_else(|| consts[&c[5]].clone());
                let entry = (c[2].to_string(), c[3].replace('_', "").parse::<u64>().unwrap(), sha);
                // A url_name listed twice must carry the same pins both times.
                if let Some(previous) = files.insert(c[1].to_string(), entry.clone()) {
                    assert_eq!(previous, entry, "{} pinned twice", &c[1]);
                }
                names.push(c[1].to_string());
            }
            lists.insert(list[1].to_string(), names);
        }
        let checkpoints: Vec<_> = Regex::new(r#"repo: "([^"]+)",\s*revision: "([0-9a-f]{40})",\s*files: ([A-Z0-9_]+),"#)
            .unwrap()
            .captures_iter(source)
            .map(|c| (c[1].to_string(), c[2].to_string(), lists[&c[3]].clone()))
            .collect();
        assert_all_parsed(source, "WhisperFile", lists.values().map(Vec::len).sum());
        assert_all_parsed(source, "WhisperCheckpoint", checkpoints.len());
        (checkpoints, files)
    }

    /// The desktop's current SonoJot Parakeet release (`PARAKEET_FT2` in `optional_asr.rs`). Its
    /// files keep their names, so url_name and local name are the same.
    fn desktop_parakeet() -> (DesktopModel, std::collections::HashMap<String, DesktopFile>) {
        use regex::Regex;
        let source = include_str!("../../../rewrite/src-tauri/src/optional_asr.rs");
        let consts = hex_consts(source);
        let pinned = Regex::new(r#"PinnedFile \{\s*relative: "([^"]+)",\s*bytes: ([0-9_]+),\s*sha256: "([0-9a-f]{64})",\s*\}"#).unwrap();
        let parse = |text: &str| -> Option<DesktopFile> {
            let f = pinned.captures(text)?;
            Some((f[1].to_string(), f[2].replace('_', "").parse().unwrap(), f[3].to_string()))
        };
        // Named pins; only the ones the release lists must parse (others, such as the VAD's, are
        // built from constants and are not Parakeet's).
        let named: std::collections::HashMap<String, Option<DesktopFile>> = Regex::new(r"(?s)\nconst ([A-Z0-9_]+): PinnedFile = (PinnedFile \{.*?\});")
            .unwrap()
            .captures_iter(source)
            .map(|c| (c[1].to_string(), parse(&c[2])))
            .collect();
        let block = Regex::new(r"(?s)\nconst PARAKEET_FT2: OptionalModel = OptionalModel \{(.*?)\n\};").unwrap().captures(source).expect("PARAKEET_FT2")[1].to_string();
        let repo = Regex::new(r#"\n    repo: "([^"]+)","#).unwrap().captures(&block).expect("repo")[1].to_string();
        let revision = Regex::new(r#"\n    revision: (?:"([0-9a-f]{40})"|([A-Z0-9_]+)),"#).unwrap().captures(&block).expect("revision");
        let revision = revision.get(1).map(|m| m.as_str().to_string()).unwrap_or_else(|| consts[&revision[2]].clone());
        let list = Regex::new(r"(?s)\n    files: &\[(.*?)\n    \],").unwrap().captures(&block).expect("files")[1].to_string();
        let mut files = std::collections::HashMap::new();
        let mut names = Vec::new();
        for item in Regex::new(r"(?s)PinnedFile \{.*?\}|\b[A-Z][A-Z0-9_]+\b").unwrap().find_iter(&list) {
            let pin = parse(item.as_str())
                .or_else(|| named.get(item.as_str()).cloned().flatten())
                .unwrap_or_else(|| panic!("unknown pin {}", item.as_str()));
            names.push(pin.0.clone());
            files.insert(pin.0.clone(), pin);
        }
        assert_eq!(names.len(), 4, "parsed {names:?}");
        ((repo, revision, names), files)
    }

    /// Every way `pack` disagrees with the desktop catalog; empty when it matches exactly.
    fn pin_mismatches(pack: &CatalogPack) -> Vec<String> {
        let (mut models, mut files) = desktop_whisper();
        let (parakeet, parakeet_files) = desktop_parakeet();
        models.push(parakeet);
        files.extend(parakeet_files);
        let Some((_, revision, names)) = models.iter().find(|(repo, ..)| repo == pack.repo) else {
            return vec![format!("{}: repo {} is not in the desktop catalog", pack.id, pack.repo)];
        };
        let mut problems = Vec::new();
        if revision != pack.revision {
            problems.push(format!("{}: revision {} != desktop {revision}", pack.id, pack.revision));
        }
        let mut remotes: Vec<_> = pack.files.iter().map(|f| f.remote.to_string()).collect();
        let mut desktop_names = names.clone();
        remotes.sort();
        desktop_names.sort();
        if remotes != desktop_names {
            problems.push(format!("{}: files {remotes:?} != desktop {desktop_names:?}", pack.id));
        }
        for file in pack.files {
            let pinned = (file.path.to_string(), file.bytes, file.sha256.to_string());
            match files.get(file.remote) {
                Some(desktop) if *desktop == pinned => {}
                desktop => problems.push(format!("{} {}: {pinned:?} != desktop {desktop:?}", pack.id, file.remote)),
            }
        }
        problems
    }

    #[test]
    fn pins_match_the_desktop_catalog() {
        // One set of pins: the desktop's whisper catalog is the source. Each file is compared by
        // its url_name, so two files with swapped hashes or sizes fail.
        for pack in CATALOG {
            assert_eq!(pin_mismatches(pack), Vec::<String>::new());
        }
    }

    #[test]
    fn the_pin_comparison_notices_swapped_hashes_and_sizes() {
        // CatalogPack holds 'static slices; leaking a few test entries is fine.
        let with = |files: Vec<CatalogFile>| -> &'static [CatalogFile] { Box::leak(files.into_boxed_slice()) };
        let real = find("whisper-small-int8").unwrap();
        let (encoder, decoder, tokens) = (real.files[0], real.files[1], real.files[2]);
        let swapped_hashes = with(vec![
            CatalogFile { sha256: decoder.sha256, ..encoder },
            CatalogFile { sha256: encoder.sha256, ..decoder },
            tokens,
        ]);
        let problems = pin_mismatches(&CatalogPack { files: swapped_hashes, ..*real });
        assert_eq!(problems.len(), 2, "{problems:?}");
        let swapped_sizes = with(vec![CatalogFile { bytes: decoder.bytes, ..encoder }, CatalogFile { bytes: encoder.bytes, ..decoder }, tokens]);
        assert_eq!(pin_mismatches(&CatalogPack { files: swapped_sizes, ..*real }).len(), 2);
        let renamed = with(vec![CatalogFile { path: "decoder.int8.onnx", ..encoder }, decoder, tokens]);
        assert_eq!(pin_mismatches(&CatalogPack { files: renamed, ..*real }).len(), 1);
        assert_eq!(pin_mismatches(&CatalogPack { revision: "0", ..*real }).len(), 1);
        // The same holds for the Parakeet pack, whose pins come from optional_asr.rs.
        let parakeet = find("sonojot-parakeet-v1-int8").unwrap();
        let (decoder, joiner) = (parakeet.files[1], parakeet.files[2]);
        let swapped = with(vec![parakeet.files[0], CatalogFile { sha256: joiner.sha256, ..decoder }, CatalogFile { sha256: decoder.sha256, ..joiner }, parakeet.files[3]]);
        assert_eq!(pin_mismatches(&CatalogPack { files: swapped, ..*parakeet }).len(), 2);
        // The desktop's earlier encoder (still listed under `previous`) is not the current pin.
        let earlier = with(vec![
            CatalogFile { bytes: 652_184_374, sha256: "623cb6020e8151afc86b1d0c02e17266e77c74de9bfcafaf99eb4cdbe988fbf5", ..parakeet.files[0] },
            decoder,
            joiner,
            parakeet.files[3],
        ]);
        assert_eq!(pin_mismatches(&CatalogPack { files: earlier, ..*parakeet }).len(), 1);
    }

    #[test]
    fn english_uses_sonojot_parakeet_as_a_nemo_transducer() {
        let pack = pack_for("en-US").unwrap();
        assert_eq!(pack.id, "sonojot-parakeet-v1-int8");
        let manifest = pack.manifest();
        assert_eq!(manifest.family, ModelFamily::Transducer);
        assert_eq!(manifest.options.model_type.as_deref(), Some("nemo_transducer"));
        assert_eq!(manifest.languages, ["en"]);
        assert_eq!(pack.bytes(), 661_814_767);
        // Whisper Small stays for every other language.
        for language in ["ar", "tr", "de", "nb-NO"] {
            let other = pack_for(language).unwrap();
            assert_eq!((other.id, other.manifest().options.model_type), ("whisper-small-int8", None), "{language}");
        }
    }

    #[test]
    fn languages_map_to_packs() {
        assert_eq!(pack_for("en").unwrap().id, "sonojot-parakeet-v1-int8");
        assert_eq!(pack_for("ar").unwrap().id, "whisper-small-int8");
        assert_eq!(pack_for("tr").unwrap().id, "whisper-small-int8");
        assert_eq!(pack_for("de").unwrap().id, "whisper-small-int8");
        assert_eq!(pack_for("en-US").unwrap().id, "sonojot-parakeet-v1-int8");
        assert_eq!(pack_for("nb-NO").unwrap().id, "whisper-small-int8");
        assert!(pack_for("").is_none());
        assert!(pack_for("1234").is_none());
        assert!(find("nope").is_none());
    }

    #[test]
    fn download_hosts_are_huggingface_only() {
        for host in ["huggingface.co", "cdn-lfs.huggingface.co", "cdn-lfs-us-1.hf.co", "cas-bridge.xethub.hf.co"] {
            assert!(download_host_allowed(host), "{host}");
        }
        for host in ["evilhuggingface.co", "huggingface.co.evil.com", "hf.co.example", "github.com", ""] {
            assert!(!download_host_allowed(host), "{host}");
        }
    }

    #[test]
    fn finalize_writes_the_manifest_only_for_verified_files() {
        let dir = TempDir::new("finalize");
        let pack = find("sonojot-parakeet-v1-int8").unwrap();
        // Real sizes would be 662 MB; a wrong file must be refused without writing a manifest.
        for file in pack.files {
            std::fs::write(dir.0.join(file.path), b"not the model").unwrap();
        }
        assert_eq!(finalize(&dir.0, pack.id).unwrap_err().code(), "pack_verification");
        assert!(!dir.0.join(MANIFEST_FILE).exists());
        assert_eq!(finalize(&dir.0, "nope").unwrap_err().code(), "invalid_argument");
        assert_eq!(leftovers(&dir.0), Vec::<String>::new());
    }

    fn leftovers(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|n| n.ends_with(".tmp"))
            .collect()
    }

    #[cfg(unix)]
    #[test]
    fn a_planted_manifest_link_is_replaced_not_followed() {
        use crate::manifest::tests::write_pack;
        let dir = TempDir::new("finalize-link");
        let outside = TempDir::new("finalize-link-target");
        let manifest = write_pack(&dir.0, ModelFamily::Whisper, &["en"], &["encoder", "decoder", "tokens"]);
        let target = outside.0.join("victim.json");
        std::fs::write(&target, b"keep me").unwrap();
        std::fs::remove_file(dir.0.join(MANIFEST_FILE)).unwrap();
        std::os::unix::fs::symlink(&target, dir.0.join(MANIFEST_FILE)).unwrap();
        install_manifest(&dir.0, &manifest).unwrap();
        assert_eq!(std::fs::read(&target).unwrap(), b"keep me");
        assert!(std::fs::symlink_metadata(dir.0.join(MANIFEST_FILE)).unwrap().file_type().is_file());
        assert_eq!(ModelPackManifest::load(&dir.0).unwrap(), manifest);
        assert_eq!(leftovers(&dir.0), Vec::<String>::new());
    }

    #[test]
    fn a_failed_manifest_write_leaves_nothing_behind() {
        use crate::manifest::tests::write_pack;
        let dir = TempDir::new("finalize-fail");
        let manifest = write_pack(&dir.0, ModelFamily::Whisper, &["en"], &["encoder", "decoder", "tokens"]);
        // A directory where manifest.json belongs: the rename fails and the temporary file goes.
        std::fs::remove_file(dir.0.join(MANIFEST_FILE)).unwrap();
        std::fs::create_dir(dir.0.join(MANIFEST_FILE)).unwrap();
        std::fs::write(dir.0.join(MANIFEST_FILE).join("keep"), b"x").unwrap();
        assert_eq!(install_manifest(&dir.0, &manifest).unwrap_err().code(), "pack_invalid");
        assert_eq!(leftovers(&dir.0), Vec::<String>::new());
        assert!(dir.0.join(MANIFEST_FILE).join("keep").exists());
    }

    #[test]
    fn catalog_json_lists_urls_and_sizes() {
        let value: serde_json::Value = serde_json::from_str(&catalog_json()).unwrap();
        let packs = value["packs"].as_array().unwrap();
        assert_eq!(packs.len(), CATALOG.len());
        assert_eq!(packs[0]["bytes"], 661_814_767u64);
        assert!(packs[1]["files"][0]["url"].as_str().unwrap().ends_with("/small-encoder.int8.onnx"));
        assert_eq!(value["download_hosts"][0], "huggingface.co");
        assert_eq!(value["max_redirects"], 8);
    }
}
