//! Read-only verification for Locate models. The caller owns the idle check and
//! atomic replacement of paths.json; verification never starts a model or writes
//! into either the selected folder or the current model cache.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::Read,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SetupState {
    Missing,
    Checking,
    Installed,
    Loading,
    Ready,
    Failed,
    Unsupported,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupCapability {
    pub state: SetupState,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupStatus {
    pub microphone: SetupCapability,
    pub speech: SetupCapability,
    pub shortcut: SetupCapability,
    pub cleanup: SetupCapability,
    pub media_import: SetupCapability,
    pub models_path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Identities {
    schema_version: u32,
    platform: String,
    groups: Vec<Group>,
}
#[derive(Deserialize)]
struct Group {
    id: String,
    directory: String,
    files: Vec<Identity>,
}
#[derive(Deserialize)]
struct Identity {
    name: String,
    bytes: u64,
    sha256: String,
}

/// Constructible only after verification; no selected manifest or paths.json is
/// consulted. Callers must not accept a deserialized value from IPC.
#[derive(Debug)]
pub struct VerifiedModels {
    paths: BTreeMap<String, String>,
    models: Vec<String>,
}
impl VerifiedModels {
    /// Allowlisted keys understood by asr::bundle_path and cleanup. Replace the
    /// current pointer file atomically only after this operation has succeeded.
    pub fn paths_json(&self) -> serde_json::Value {
        serde_json::json!(self.paths)
    }
    pub fn model_ids(&self) -> Vec<String> {
        self.models.clone()
    }
}

pub fn verify_models_folder(folder: &Path) -> Result<VerifiedModels, String> {
    if crate::model_manager::linux_runtime::AVAILABLE {
        let mut identities: Identities = serde_json::from_str(include_str!("../assets/model-identities.json"))
            .map_err(|_| "The app's model identity list could not be read. Reinstall the app.")?;
        identities.platform = "linux-x86_64".into();
        identities.groups.retain(|group| group.id.starts_with("moonshine-"));
        for group in &mut identities.groups {
            if group.id == "moonshine-runtime" {
                group.directory = "moonshine/runtime/linux-x64/lib".into();
                group.files = crate::model_manager::linux_runtime::MOONSHINE_FILES.iter().map(|pin| Identity {
                    name: pin.relative.into(), bytes: pin.bytes, sha256: pin.sha256.into(),
                }).collect();
            } else if folder.join("optional").join(&group.id).is_dir() {
                group.directory = format!("optional/{}", group.id);
            }
        }
        return verify(folder, &identities);
    }
    if !cfg!(all(target_os = "windows", target_arch = "x86_64")) {
        return Err("This model-folder verifier supports the Windows x64 bundle. Choose models on a supported Windows computer.".into());
    }
    let identities: Identities =
        serde_json::from_str(include_str!("../assets/model-identities.json"))
            .map_err(|_| "The app's model identity list could not be read. Reinstall the app.")?;
    verify(folder, &identities)
}

fn safe_relative(value: &str) -> bool {
    !value.is_empty()
        && Path::new(value)
            .components()
            .all(|part| matches!(part, Component::Normal(_)))
}

fn verify(folder: &Path, identities: &Identities) -> Result<VerifiedModels, String> {
    if identities.schema_version != 1 || !matches!(identities.platform.as_str(), "windows-x86_64" | "linux-x86_64") {
        return Err("The app's model identity list is unsupported. Reinstall the app.".into());
    }
    let root = fs::canonicalize(folder)
        .map_err(|_| "The models folder could not be opened. Choose an existing folder.")?;
    if !root.is_dir() {
        return Err("Choose a folder containing the speech models and their runtime.".into());
    }
    let mut groups = BTreeMap::<String, PathBuf>::new();
    for group in &identities.groups {
        if !matches!(
            group.id.as_str(),
            "moonshine-runtime"
                | "moonshine-medium"
                | "moonshine-small"
                | "cleanup-model"
                | "cleanup-cpu"
                | "cleanup-cuda"
        ) || !safe_relative(&group.directory)
            || group.files.is_empty()
        {
            return Err("The app's model identity list is invalid. Reinstall the app.".into());
        }
        let directory = root.join(&group.directory);
        let present = if group.id == "cleanup-model" {
            group
                .files
                .iter()
                .any(|file| directory.join(&file.name).exists())
        } else {
            directory.exists()
        };
        if !present {
            continue;
        }
        let canonical = fs::canonicalize(&directory).map_err(|_| {
            format!(
                "The {} files could not be opened. Choose another models folder.",
                group.id
            )
        })?;
        if !canonical.is_dir() || !canonical.starts_with(&root) {
            return Err(
                "The models folder links to files outside itself. Choose a complete local bundle."
                    .into(),
            );
        }
        for identity in &group.files {
            if !safe_relative(&identity.name) || Path::new(&identity.name).components().count() != 1
            {
                return Err("The app's model identity list is invalid. Reinstall the app.".into());
            }
            verify_file(&canonical, identity)?;
        }
        // These directories can load sibling DLLs. Reject added DLLs even when
        // all required files match; an unchecked dependency is not a trusted runtime.
        if group.id.ends_with("runtime") || group.id.starts_with("cleanup-c") {
            let expected: BTreeSet<_> = group
                .files
                .iter()
                .map(|f| f.name.to_ascii_lowercase())
                .collect();
            for entry in fs::read_dir(&canonical)
                .map_err(|_| "The model runtime folder could not be read.")?
            {
                let entry = entry.map_err(|_| "The model runtime folder could not be read.")?;
                let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
                if (name.ends_with(".dll") || name.contains(".so")) && !expected.contains(&name) {
                    return Err("The model runtime contains an unrecognized library. Choose the original model bundle.".into());
                }
            }
        }
        groups.insert(group.id.clone(), canonical);
    }
    if !groups.contains_key("moonshine-runtime") {
        return Err("The speech runtime is missing. Choose the complete models folder, including its runtime.".into());
    }
    let models: Vec<String> = ["moonshine-medium", "moonshine-small"]
        .into_iter()
        .filter(|id| groups.contains_key(*id))
        .map(str::to_owned)
        .collect();
    if models.is_empty() {
        return Err(
            "No supported speech model was found. Choose a folder with Moonshine Medium or Small."
                .into(),
        );
    }
    let mut paths = BTreeMap::new();
    let runtime = &groups["moonshine-runtime"];
    let library = if identities.platform == "linux-x86_64" { "libmoonshine.so" } else { "moonshine.dll" };
    insert_path(&mut paths, "moonshine_dll", &runtime.join(library))?;
    if let Some(parent) = runtime.parent() {
        insert_path(&mut paths, "moonshine_runtime_dir", parent)?;
    }
    for (id, alias) in [
        ("moonshine-medium", "moonshine_medium_dir"),
        ("moonshine-small", "moonshine_small_dir"),
    ] {
        if let Some(directory) = groups.get(id) {
            insert_path(&mut paths, alias, directory)?;
        }
    }
    let mut result = VerifiedModels { paths, models };
    if let Some(weights) = groups.get("cleanup-model") {
        let cpu = groups.get("cleanup-cpu");
        let cuda = groups.get("cleanup-cuda");
        if cpu.is_some() || cuda.is_some() {
            insert_path(
                &mut result.paths,
                "cleanup_gguf",
                &weights.join("LFM2.5-1.2B-Instruct-Q4_K_M.gguf"),
            )?;
            if let Some(directory) = cpu {
                insert_path(
                    &mut result.paths,
                    "llama_server_exe",
                    &directory.join("llama-server.exe"),
                )?;
            }
            if let Some(directory) = cuda {
                insert_path(
                    &mut result.paths,
                    "llama_server_cuda_exe",
                    &directory.join("llama-server.exe"),
                )?;
            }
            result.models.push("lfm2.5-1.2b".into());
        }
    }
    Ok(result)
}

fn insert_path(
    paths: &mut BTreeMap<String, String>,
    key: &str,
    value: &Path,
) -> Result<(), String> {
    let value = value
        .to_str()
        .ok_or("The models folder name could not be read. Choose another local folder.")?;
    paths.insert(key.into(), value.into());
    Ok(())
}

fn verify_file(directory: &Path, identity: &Identity) -> Result<(), String> {
    let path = fs::canonicalize(directory.join(&identity.name)).map_err(|_| {
        format!(
            "The model bundle is missing {}. Choose the complete original folder.",
            identity.name
        )
    })?;
    if !path.starts_with(directory) {
        return Err(
            "The models folder links to files outside itself. Choose a complete local bundle."
                .into(),
        );
    }
    let mut file =
        File::open(path).map_err(|_| "A model file could not be read. Choose another folder.")?;
    let metadata = file
        .metadata()
        .map_err(|_| "A model file could not be inspected.")?;
    if !metadata.is_file() || metadata.len() != identity.bytes {
        return Err(format!(
            "The model file {} is incomplete or changed. Choose the original model bundle.",
            identity.name
        ));
    }
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    let mut buffer = [0u8; 64 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|_| "A model file could not be read completely.")?;
        if read == 0 {
            break;
        }
        bytes += read as u64;
        if bytes > identity.bytes {
            return Err(
                "A model file changed during verification. Close other model tools and try again."
                    .into(),
            );
        }
        hasher.update(&buffer[..read]);
    }
    if bytes != identity.bytes || format!("{:x}", hasher.finalize()) != identity.sha256 {
        return Err(format!("The model file {} does not match this app's supported bundle. Choose the original models folder.", identity.name));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let root = std::env::temp_dir().join(format!(
                "sonojot-setup-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&root).unwrap();
            Self(root)
        }
        fn group(&self, id: &str, directory: &str, names: &[&str]) -> Group {
            let path = self.0.join(directory);
            fs::create_dir_all(&path).unwrap();
            Group {
                id: id.into(),
                directory: directory.into(),
                files: names
                    .iter()
                    .map(|name| {
                        fs::write(path.join(name), name.as_bytes()).unwrap();
                        Identity {
                            name: (*name).into(),
                            bytes: name.len() as u64,
                            sha256: format!("{:x}", Sha256::digest(name.as_bytes())),
                        }
                    })
                    .collect(),
            }
        }
        fn identities(&self, model: &str) -> Identities {
            Identities {
                schema_version: 1,
                platform: "windows-x86_64".into(),
                groups: vec![
                    self.group(
                        "moonshine-runtime",
                        "moonshine/runtime/win-x64/lib",
                        &["moonshine.dll", "onnxruntime.dll"],
                    ),
                    self.group(model, "speech", &["encoder.ort"]),
                ],
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn either_known_speech_model_works_without_cleanup_or_gpu() {
        for model in ["moonshine-medium", "moonshine-small"] {
            let fixture = Fixture::new();
            let verified = verify(&fixture.0, &fixture.identities(model)).unwrap();
            assert_eq!(verified.model_ids(), vec![model]);
            assert!(verified.paths_json().get("cleanup_gguf").is_none());
        }
    }
    #[test]
    fn missing_and_same_length_corrupt_files_fail_without_changing_pointer_or_settings() {
        let fixture = Fixture::new();
        let identities = fixture.identities("moonshine-medium");
        fs::write(fixture.0.join("paths.json"), b"original pointers").unwrap();
        fs::write(fixture.0.join("settings.json"), b"original settings").unwrap();
        fs::write(fixture.0.join("speech/encoder.ort"), b"XXXXXXXXXXX").unwrap();
        assert!(verify(&fixture.0, &identities)
            .unwrap_err()
            .contains("does not match"));
        fs::remove_file(fixture.0.join("speech/encoder.ort")).unwrap();
        assert!(verify(&fixture.0, &identities)
            .unwrap_err()
            .contains("missing"));
        assert_eq!(
            fs::read(fixture.0.join("paths.json")).unwrap(),
            b"original pointers"
        );
        assert_eq!(
            fs::read(fixture.0.join("settings.json")).unwrap(),
            b"original settings"
        );
    }
    #[test]
    fn selected_manifests_and_absolute_aliases_cannot_change_trusted_identities() {
        let fixture = Fixture::new();
        let identities = fixture.identities("moonshine-small");
        fs::write(
            fixture.0.join("manifest.json"),
            b"{\"sha256\":\"trust me\"}",
        )
        .unwrap();
        fs::write(
            fixture.0.join("paths.json"),
            b"{\"moonshine_dll\":\"C:/elsewhere/unverified.dll\"}",
        )
        .unwrap();
        let paths = verify(&fixture.0, &identities).unwrap().paths_json();
        assert!(!paths.to_string().contains("elsewhere"));
        assert_eq!(paths.as_object().unwrap().len(), 3);
    }
    #[test]
    fn added_runtime_library_is_rejected() {
        let fixture = Fixture::new();
        let identities = fixture.identities("moonshine-small");
        fs::write(
            fixture
                .0
                .join("moonshine/runtime/win-x64/lib/onnxruntime_providers_shared.dll"),
            b"unverified",
        )
        .unwrap();
        assert!(verify(&fixture.0, &identities)
            .unwrap_err()
            .contains("unrecognized"));
    }
    #[test]
    fn empty_folder_and_runtime_only_folder_do_not_replace_models() {
        let fixture = Fixture::new();
        let identities = fixture.identities("moonshine-medium");
        fs::remove_file(fixture.0.join("speech/encoder.ort")).unwrap();
        fs::remove_dir(fixture.0.join("speech")).unwrap();
        assert!(verify(&fixture.0, &identities)
            .unwrap_err()
            .contains("No supported speech"));
        let empty = Fixture::new();
        assert!(verify(&empty.0, &identities)
            .unwrap_err()
            .contains("runtime is missing"));
    }
    #[test]
    fn embedded_manifest_has_the_six_expected_groups() {
        let identities: Identities =
            serde_json::from_str(include_str!("../assets/model-identities.json")).unwrap();
        assert_eq!(identities.groups.len(), 6);
        assert!(identities
            .groups
            .iter()
            .all(|g| !g.files.is_empty()
                && g.files.iter().all(|f| f.bytes > 0 && f.sha256.len() == 64)));
    }
}
