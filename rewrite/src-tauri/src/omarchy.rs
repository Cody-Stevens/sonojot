//! Omarchy's bar keeps tray icons in a collapsed drawer unless they are
//! pinned. SonoJot pins its own icon once, the first time it runs under
//! Omarchy, so dictation status stays visible. Unpinning it later sticks.
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The StatusNotifierItem id libayatana derives from `TrayIconBuilder::with_id("sonojot")`.
const TRAY_ID: &str = "tray-icon tray app sonojot";
const MARKER: &str = "omarchy-tray-pinned";

fn shell_config() -> Option<PathBuf> {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("omarchy/shell.json"))
        .filter(|path| path.is_file())
}

/// Add SonoJot to the tray's pinned list in `shell.json`. Returns whether the
/// file changed. Entries may be bare ids or objects, as Omarchy writes both.
fn pin(config: &mut Value) -> bool {
    let Some(layout) = config.pointer_mut("/bar/layout").and_then(Value::as_object_mut) else {
        return false;
    };
    for section in ["left", "center", "right"] {
        let Some(entries) = layout.get_mut(section).and_then(Value::as_array_mut) else { continue };
        for entry in entries.iter_mut() {
            let is_tray = entry.as_str() == Some("omarchy.tray")
                || entry.get("id").and_then(Value::as_str) == Some("omarchy.tray");
            if !is_tray {
                continue;
            }
            if entry.is_string() {
                *entry = serde_json::json!({ "id": "omarchy.tray" });
            }
            let Some(tray) = entry.as_object_mut() else { return false };
            let listed = |key: &str| {
                tray.get(key)
                    .and_then(Value::as_array)
                    .is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(TRAY_ID)))
            };
            if listed("pinned") || listed("hidden") {
                return false;
            }
            let pinned = tray.entry("pinned").or_insert_with(|| Value::Array(Vec::new()));
            if !pinned.is_array() {
                *pinned = Value::Array(Vec::new());
            }
            pinned.as_array_mut().unwrap().push(Value::from(TRAY_ID));
            return true;
        }
    }
    false
}

/// Pin the tray icon the first time SonoJot runs under Omarchy.
pub fn pin_tray_once(data_dir: &Path) {
    let marker = data_dir.join(MARKER);
    let Some(path) = shell_config() else { return };
    if marker.exists() {
        return;
    }
    let Ok(mut config) = std::fs::read_to_string(&path)
        .map_err(|_| ())
        .and_then(|text| serde_json::from_str::<Value>(&text).map_err(|_| ()))
    else {
        return;
    };
    if pin(&mut config) {
        // Omarchy's own helpers write sorted keys (jq -S) with two-space indents.
        let Ok(text) = serde_json::to_string_pretty(&config) else { return };
        let temporary = path.with_extension("json.sonojot-tmp");
        if std::fs::write(&temporary, text + "\n")
            .and_then(|_| std::fs::rename(&temporary, &path))
            .is_err()
        {
            let _ = std::fs::remove_file(&temporary);
            return;
        }
        let _ = std::process::Command::new("omarchy-shell")
            .args(["-q", "shell", "reloadConfig"])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    let _ = std::fs::write(marker, "");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pins_once_and_respects_hidden_or_existing_choices() {
        let mut config = serde_json::json!({ "bar": { "layout": { "right": ["omarchy.clock", "omarchy.tray"] } } });
        assert!(pin(&mut config));
        assert_eq!(config["bar"]["layout"]["right"][1]["pinned"][0], TRAY_ID);
        assert!(!pin(&mut config));
        let mut hidden = serde_json::json!({ "bar": { "layout": { "right": [{ "id": "omarchy.tray", "hidden": [TRAY_ID] }] } } });
        assert!(!pin(&mut hidden));
        let mut none = serde_json::json!({ "bar": { "layout": { "left": ["omarchy.workspaces"] } } });
        assert!(!pin(&mut none));
    }
}
