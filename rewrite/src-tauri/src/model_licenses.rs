//! Reviewed license destinations are shared with the model library and notices.
//! The frontend sends a catalog ID, never an arbitrary browser URL.

use serde::Deserialize;
use std::sync::OnceLock;

/// Retain attribution beside exported model folders without changing recognition.
pub(crate) fn write_punctuation_notices(root: &std::path::Path, id: &str) -> Result<(), String> {
    let package = crate::punctuation_model::Package::from_id(id).ok_or("Unknown punctuation model.")?;
    let directory = package.path(root);
    crate::optional_asr::ensure_within(root, &directory)?;
    let notice = match package {
        crate::punctuation_model::Package::V1 => include_str!("../../licenses/punctuation/NOTICE-v1.txt"),
        crate::punctuation_model::Package::V2 => include_str!("../../licenses/punctuation/NOTICE-v2.txt"),
    };
    for (name, text) in [("LICENSE-CC-BY-SA-4.0.txt", include_str!("../../licenses/downloads/standard/cc-by-sa-4.0.txt")), ("NOTICE", notice)] {
        let path = directory.join(name);
        crate::optional_asr::ensure_within(root, &path)?;
        if std::fs::symlink_metadata(&path).is_ok_and(|meta| !meta.file_type().is_file()) {
            return Err("Cannot write model attribution over a linked or non-file path.".into());
        }
        std::fs::write(path, text).map_err(|e| format!("Cannot retain punctuation model {name}: {e}"))?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct Catalog {
    components: Vec<Component>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Component {
    kind: String,
    license: String,
    license_url: String,
    catalog_ids: Vec<String>,
}

fn catalog() -> Result<&'static Catalog, String> {
    static CATALOG: OnceLock<Result<Catalog, String>> = OnceLock::new();
    CATALOG
        .get_or_init(|| {
            serde_json::from_str(include_str!("../../licenses/model-links.json"))
                .map_err(|error| format!("Cannot read the model license catalog: {error}"))
        })
        .as_ref()
        .map_err(Clone::clone)
}

fn license_url(id: &str) -> Result<&'static str, String> {
    if matches!(id, "sonojot-punct-v1" | "sonojot-punct-v2") {
        return Ok("https://creativecommons.org/licenses/by-sa/4.0/legalcode.en");
    }
    let component = catalog()?
        .components
        .iter()
        .find(|component| {
            component.kind == "model" && component.catalog_ids.iter().any(|entry| entry == id)
        })
        .ok_or("Unknown model license.")?;
    // Catalog-owned data is still checked before passing it to an OS program.
    let url =
        reqwest::Url::parse(&component.license_url).map_err(|_| "Invalid model license URL.")?;
    if component.license.trim().is_empty()
        || url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Invalid model license URL.".into());
    }
    Ok(&component.license_url)
}

#[tauri::command]
pub async fn open_model_license(id: String) -> Result<(), String> {
    crate::legal_links::open_https(license_url(&id)?.into(), "model license").await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn both_punctuation_packages_retain_license_and_lineage_beside_weights() {
        let root = std::env::temp_dir().join(format!("sonojot-model-notices-{}", crate::store::new_id()));
        for id in ["sonojot-punct-v1", "sonojot-punct-v2"] {
            let path = crate::punctuation_model::Package::from_id(id).unwrap().path(&root);
            std::fs::create_dir_all(&path).unwrap();
            write_punctuation_notices(&root, id).unwrap();
            let notice = std::fs::read_to_string(path.join("NOTICE")).unwrap();
            for text in ["SonoJot", "Rev.com", "Horizon-Labs", "mmBERT", "CC BY-SA"] { assert!(notice.contains(text), "{id}: {text}"); }
            assert!(std::fs::read_to_string(path.join("LICENSE-CC-BY-SA-4.0.txt")).unwrap().contains("Attribution-ShareAlike"));
        }
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn license_links_are_unique_https_destinations() {
        let mut ids = HashSet::new();
        for component in &catalog().unwrap().components {
            if component.kind != "model" {
                continue;
            }
            for id in &component.catalog_ids {
                assert!(ids.insert(id), "Duplicate model license: {id}");
                assert!(license_url(id).is_ok(), "Invalid license for {id}");
            }
        }
        assert!(!ids.is_empty());
        for id in [
            "",
            "--help",
            "../sonojot-punct-v2",
            "https://example.com",
            "file:///C:/Windows",
        ] {
            assert!(license_url(id).is_err());
        }
    }

    #[test]
    fn every_library_download_has_license_evidence() {
        let _gate = crate::resource_manager::tests::test_gate();
        let root = std::env::temp_dir().join("sonojot-license-coverage-no-models");
        let library =
            crate::model_manager::build_library(&root, &crate::domain::Settings::default());
        for model in library.models {
            if model.id != "local-rules" {
                assert!(
                    license_url(&model.id).is_ok(),
                    "Missing license for {}",
                    model.id
                );
            }
        }
        // These are also shipped in the macOS catalog, even when testing Windows.
        for id in [
            "whisper-small-metal",
            "whisper-turbo-metal",
            "qwen3-asr-1.7b-q8-metal",
            "qwen3-asr-1.7b-q4-metal",
        ] {
            assert!(license_url(id).is_ok(), "Missing Mac license for {id}");
        }
    }
}
