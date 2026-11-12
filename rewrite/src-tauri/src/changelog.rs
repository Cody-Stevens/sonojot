//! User-facing release notes: "What's new" after an update and the preview
//! of an offered update.
//!
//! Sources, most trusted last so they win on a version clash:
//! 1. `changelog.json` at the repository's `rewrite/` root, bundled into the
//!    binary. Every build carries the notes for its own version and earlier,
//!    so "What's new" after a restart needs no network.
//! 2. The channel's `changelog.json` on the releases host, fetched only when
//!    an update is offered and cached under the data folder.
//! 3. Notes staged just before an update installs, so the restarted app can
//!    show them even when the new build's bundled file lacks a nightly entry.
//!
//! Notes are plain text. The webview renders them as text, never as HTML.
use crate::domain::UpdateChannel;
use crate::updater::{newer, on_channel};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};

const BUNDLED: &str = include_str!("../../changelog.json");
const MAX_FILE: usize = 512 * 1024;
const MAX_RELEASES: usize = 400;
const MAX_NOTES: usize = 24;
const MAX_NOTE_CHARS: usize = 600;
/// More sections than anyone reads in one sitting.
const MAX_SHOWN: usize = 30;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Release {
    pub version: String,
    /// `YYYY-MM-DD`, or empty when only the update manifest's notes are known.
    #[serde(default)]
    pub date: String,
    pub notes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNew {
    /// The version this one replaced, when known.
    pub from: Option<String>,
    pub to: String,
    /// Newest first.
    pub releases: Vec<Release>,
}

#[derive(Deserialize)]
struct FileFormat {
    schema: u32,
    #[serde(default)]
    releases: Vec<serde_json::Value>,
}

fn valid_date(date: &str) -> bool {
    let b = date.as_bytes();
    date.is_empty()
        || (b.len() == 10
            && b[4] == b'-'
            && b[7] == b'-'
            && b.iter()
                .enumerate()
                .all(|(i, c)| i == 4 || i == 7 || c.is_ascii_digit()))
}

/// Plain text only: control characters become spaces and runs of whitespace
/// collapse. Anything that looks like markup still renders as literal text.
fn clean_note(note: &str) -> Option<String> {
    let text = note
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    (!text.is_empty()).then(|| text.chars().take(MAX_NOTE_CHARS).collect())
}

fn clean_release(value: serde_json::Value) -> Option<Release> {
    let release: Release = serde_json::from_value(value).ok()?;
    semver::Version::parse(&release.version).ok()?;
    if !valid_date(&release.date) {
        return None;
    }
    let notes = release
        .notes
        .iter()
        .filter_map(|n| clean_note(n))
        .take(MAX_NOTES)
        .collect::<Vec<_>>();
    (!notes.is_empty()).then_some(Release { notes, ..release })
}

/// Parses a changelog file. A malformed entry is skipped; a malformed file,
/// an unknown schema or an oversized file is an error.
pub fn parse(text: &str) -> Result<Vec<Release>, String> {
    if text.len() > MAX_FILE {
        return Err("The changelog is too large.".into());
    }
    let file: FileFormat =
        serde_json::from_str(text).map_err(|_| "The changelog is not valid JSON.")?;
    if file.schema != 1 {
        return Err("The changelog format is not supported.".into());
    }
    Ok(sort(
        file.releases
            .into_iter()
            .take(MAX_RELEASES)
            .filter_map(clean_release)
            .collect(),
    ))
}

fn sort(mut releases: Vec<Release>) -> Vec<Release> {
    releases.sort_by(|a, b| {
        let a = semver::Version::parse(&a.version).ok();
        let b = semver::Version::parse(&b.version).ok();
        b.cmp(&a)
    });
    releases.dedup_by(|a, b| a.version == b.version);
    releases
}

/// Union by version. Earlier lists win, so pass the most trusted first.
pub fn merge(lists: &[&[Release]]) -> Vec<Release> {
    let mut all = Vec::new();
    for list in lists {
        for release in *list {
            if !all.iter().any(|r: &Release| r.version == release.version) {
                all.push(release.clone());
            }
        }
    }
    sort(all)
}

/// The releases someone moving from `installed` to `target` on `channel`
/// gets, newest first: every version on the channel above `installed` up to
/// and including `target`. The target itself always counts. Moving to an
/// older or the same build (a channel switch or repair) shows only the target.
pub fn select(
    releases: &[Release],
    installed: Option<&str>,
    target: &str,
    channel: UpdateChannel,
) -> Vec<Release> {
    let forward = installed.is_some_and(|installed| newer(target, installed));
    let mut chosen = releases
        .iter()
        .filter(|r| {
            r.version == target
                || (forward
                    && on_channel(&r.version, channel)
                    && installed.is_some_and(|installed| newer(&r.version, installed))
                    && !newer(&r.version, target))
        })
        .cloned()
        .collect::<Vec<_>>();
    chosen = sort(chosen);
    chosen.truncate(MAX_SHOWN);
    chosen
}

/// Release notes from an update manifest, one note per paragraph or line.
pub fn from_manifest_notes(version: &str, notes: Option<&str>) -> Option<Release> {
    let notes = notes?
        .lines()
        .filter_map(clean_note)
        .take(MAX_NOTES)
        .collect::<Vec<_>>();
    (!notes.is_empty()).then(|| Release {
        version: version.into(),
        date: String::new(),
        notes,
    })
}

pub fn bundled() -> Vec<Release> {
    parse(BUNDLED).unwrap_or_default()
}

pub fn feed_url(channel: UpdateChannel) -> &'static str {
    match channel {
        UpdateChannel::Stable => "https://releases.sonojot.com/changelog.json",
        UpdateChannel::Nightly => "https://releases.sonojot.com/nightly/changelog.json",
    }
}

/// One plain GET to the releases host; no cookies, no identifiers.
fn fetch(channel: UpdateChannel) -> Result<Vec<Release>, ()> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(15))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|_| ())?;
    let response = client
        .get(feed_url(channel))
        .send()
        .and_then(|r| r.error_for_status())
        .map_err(|_| ())?;
    let mut bytes = Vec::new();
    response
        .take((MAX_FILE + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    parse(std::str::from_utf8(&bytes).map_err(|_| ())?).map_err(|_| ())
}

fn read_limited(path: &Path) -> Option<String> {
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take((MAX_FILE + 1) as u64)
        .read_to_string(&mut text)
        .ok()?;
    Some(text)
}

#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct State {
    /// The version that last ran; tells the next start what it replaced.
    last_version: Option<String>,
    /// Not yet seen by the user.
    pending: Option<WhatsNew>,
    /// The most recent update's notes, kept for About after dismissal.
    latest: Option<WhatsNew>,
    /// Notes saved just before an update installs.
    #[serde(default)]
    staged: Vec<Release>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WhatsNewState {
    pub pending: Option<WhatsNew>,
    pub latest: Option<WhatsNew>,
}

pub struct Changelog {
    directory: PathBuf,
    state: Mutex<State>,
}

impl Changelog {
    /// Records this run's version. When it differs from the last one that
    /// ran, the notes between them become pending. `previous` comes from the
    /// crash marker (an update restart, or a session that ended while the
    /// installer replaced the app) and covers installs that predate this file.
    pub fn start(
        root: &Path,
        current: &str,
        previous: Option<String>,
        channel: UpdateChannel,
    ) -> Result<Self, String> {
        let directory = root.join("updates");
        fs::create_dir_all(&directory).map_err(|_| "Could not create the updates folder.")?;
        let mut state: State = read_limited(&directory.join("whats-new.json"))
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        let from = state.last_version.clone().or(previous);
        if let Some(from) = from.filter(|from| from != current) {
            let cached = Self::read_cache(&directory);
            let staged = std::mem::take(&mut state.staged);
            let releases = select(
                &merge(&[&staged, &cached, &bundled()]),
                Some(&from),
                current,
                channel,
            );
            let record = (!releases.is_empty()).then(|| WhatsNew {
                from: Some(from),
                to: current.into(),
                releases,
            });
            state.pending = record.clone();
            if record.is_some() {
                state.latest = record;
            }
        }
        state.staged.clear();
        state.last_version = Some(current.into());
        let changelog = Self {
            directory,
            state: Mutex::new(state),
        };
        changelog.save()?;
        Ok(changelog)
    }

    fn read_cache(directory: &Path) -> Vec<Release> {
        read_limited(&directory.join("changelog.json"))
            .and_then(|text| parse(&text).ok())
            .unwrap_or_default()
    }

    fn save(&self) -> Result<(), String> {
        let state = self.state.lock().map_err(|_| "What's new is unavailable.")?;
        crate::store::atomic_write(
            &self.directory.join("whats-new.json"),
            &serde_json::to_vec(&*state).map_err(|e| e.to_string())?,
        )
    }

    pub fn get(&self) -> Result<WhatsNewState, String> {
        let state = self.state.lock().map_err(|_| "What's new is unavailable.")?;
        // Before any update, Updates still shows this build's own notes.
        let current = state.last_version.clone().unwrap_or_default();
        let own = || {
            let releases = select(&bundled(), None, &current, UpdateChannel::Nightly);
            (!releases.is_empty()).then(|| WhatsNew {
                from: None,
                to: current.clone(),
                releases,
            })
        };
        Ok(WhatsNewState {
            pending: state.pending.clone(),
            latest: state.latest.clone().or_else(own),
        })
    }

    pub fn dismiss(&self) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "What's new is unavailable.")?
            .pending = None;
        self.save()
    }

    /// Fetches the channel's notes and keeps them for later, offline use.
    /// Failure leaves the previous cache in place.
    pub fn refresh(&self, channel: UpdateChannel) -> Vec<Release> {
        let cached = Self::read_cache(&self.directory);
        let Ok(fetched) = fetch(channel) else {
            return cached;
        };
        let merged = merge(&[&fetched, &cached]);
        let text = serde_json::json!({ "schema": 1, "releases": merged });
        if let Ok(bytes) = serde_json::to_vec(&text) {
            let _ = crate::store::atomic_write(&self.directory.join("changelog.json"), &bytes);
        }
        merged
    }

    /// The notes for an offered update, from the cache, the bundled file and
    /// the manifest. `fetched` is a fresh fetch when one was made.
    pub fn for_offer(
        &self,
        fetched: Option<Vec<Release>>,
        current: &str,
        offered: &str,
        manifest_notes: Option<&str>,
        channel: UpdateChannel,
    ) -> WhatsNew {
        let known = fetched.unwrap_or_else(|| Self::read_cache(&self.directory));
        let manifest = from_manifest_notes(offered, manifest_notes)
            .into_iter()
            .collect::<Vec<_>>();
        WhatsNew {
            from: Some(current.into()),
            to: offered.into(),
            releases: select(
                &merge(&[&known, &bundled(), &manifest]),
                Some(current),
                offered,
                channel,
            ),
        }
    }

    /// Saved just before installing, so the restarted app has these notes
    /// even offline. Cleared by the next start whatever happens.
    pub fn stage(&self, releases: Vec<Release>) -> Result<(), String> {
        self.state
            .lock()
            .map_err(|_| "What's new is unavailable.")?
            .staged = releases;
        self.save()
    }

    pub fn unstage(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.staged.clear();
        }
        let _ = self.save();
    }
}

const UNAVAILABLE: &str = "What's new is unavailable.";

#[tauri::command]
pub fn get_whats_new(app: tauri::AppHandle) -> Result<WhatsNewState, String> {
    use tauri::Manager;
    app.try_state::<Changelog>().ok_or(UNAVAILABLE)?.get()
}

#[tauri::command]
pub fn dismiss_whats_new(app: tauri::AppHandle) -> Result<WhatsNewState, String> {
    use tauri::Manager;
    let state = app.try_state::<Changelog>().ok_or(UNAVAILABLE)?;
    state.dismiss()?;
    state.get()
}

/// The changelog for the update on offer (available or downloaded).
#[tauri::command]
pub async fn get_update_changelog(
    app: tauri::AppHandle,
    runtime: tauri::State<'_, std::sync::Arc<crate::app::Runtime>>,
) -> Result<Option<WhatsNew>, String> {
    use tauri::Manager;
    let Some((version, notes)) = runtime.updater.offer()? else {
        return Ok(None);
    };
    let channel = runtime
        .snapshot()
        .map(|s| s.settings.update_channel)
        .unwrap_or_default();
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let changelog = handle.try_state::<Changelog>().ok_or(UNAVAILABLE)?;
        let fetched = changelog.refresh(channel);
        Ok(Some(changelog.for_offer(
            Some(fetched),
            env!("CARGO_PKG_VERSION"),
            &version,
            notes.as_deref(),
            channel,
        )))
    })
    .await
    .map_err(|_| "Could not read the update's notes.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn release(version: &str) -> Release {
        Release {
            version: version.into(),
            date: "2026-11-12".into(),
            notes: vec![format!("Notes for {version}.")],
        }
    }

    fn versions(releases: &[Release]) -> Vec<&str> {
        releases.iter().map(|r| r.version.as_str()).collect()
    }

    fn history() -> Vec<Release> {
        sort(
            [
                "0.28.7",
                "0.28.8",
                "0.28.9-nightly.202610310654",
                "0.28.9-nightly.202611090800",
                "0.28.9",
                "0.28.10-nightly.202611120800",
                "0.28.10-nightly.202611121621",
                "0.28.10",
                "0.29.0-beta.1",
            ]
            .iter()
            .map(|v| release(v))
            .collect(),
        )
    }

    #[test]
    fn bundled_changelog_parses_and_covers_recent_versions() {
        let releases = bundled();
        let found = versions(&releases);
        for version in ["0.28.8", "0.28.9", "0.28.10-nightly.202611121621"] {
            assert!(found.contains(&version), "{version} missing");
        }
        for release in &releases {
            for note in &release.notes {
                assert!(!note.contains('\u{2014}'), "em dash in {}", release.version);
            }
        }
    }

    #[test]
    fn stable_range_skips_nightlies_and_is_newest_first() {
        let chosen = select(&history(), Some("0.28.7"), "0.28.10", UpdateChannel::Stable);
        assert_eq!(versions(&chosen), ["0.28.10", "0.28.9", "0.28.8"]);
    }

    #[test]
    fn nightly_range_includes_nightlies_and_stables_up_to_offer() {
        let chosen = select(
            &history(),
            Some("0.28.9-nightly.202610310654"),
            "0.28.10-nightly.202611121621",
            UpdateChannel::Nightly,
        );
        assert_eq!(
            versions(&chosen),
            [
                "0.28.10-nightly.202611121621",
                "0.28.10-nightly.202611120800",
                "0.28.9",
                "0.28.9-nightly.202611090800",
            ]
        );
    }

    #[test]
    fn nightly_to_stable_promotion_lists_nightlies_in_between() {
        let chosen = select(
            &history(),
            Some("0.28.10-nightly.202611120800"),
            "0.28.10",
            UpdateChannel::Nightly,
        );
        assert_eq!(versions(&chosen), ["0.28.10", "0.28.10-nightly.202611121621"]);
    }

    #[test]
    fn older_or_same_target_shows_only_the_target() {
        let older = select(&history(), Some("0.28.10-nightly.202611121621"), "0.28.9", UpdateChannel::Stable);
        assert_eq!(versions(&older), ["0.28.9"]);
        let same = select(&history(), Some("0.28.9"), "0.28.9", UpdateChannel::Stable);
        assert_eq!(versions(&same), ["0.28.9"]);
        let unknown_start = select(&history(), None, "0.28.10", UpdateChannel::Stable);
        assert_eq!(versions(&unknown_start), ["0.28.10"]);
    }

    #[test]
    fn other_prereleases_never_appear_on_either_channel() {
        let chosen = select(&history(), Some("0.28.0"), "0.29.0", UpdateChannel::Nightly);
        assert!(!versions(&chosen).contains(&"0.29.0-beta.1"));
    }

    #[test]
    fn missing_target_entry_yields_only_known_versions() {
        let chosen = select(&history(), Some("0.28.8"), "0.28.11", UpdateChannel::Stable);
        assert_eq!(versions(&chosen), ["0.28.10", "0.28.9"]);
    }

    #[test]
    fn parse_skips_bad_entries_and_cleans_text() {
        let text = r#"{"schema":1,"future":true,"releases":[
            {"version":"0.28.9","date":"2026-11-12","notes":["  Two\nlines\u0007 here. ","",3],"extra":1},
            {"version":"not a version","date":"2026-11-12","notes":["x"]},
            {"version":"0.28.8","date":"21 October","notes":["x"]},
            {"version":"0.28.7","date":"2026-10-01","notes":[]},
            {"version":"0.28.6","date":"2026-09-01","notes":["<b>Bold</b> stays literal"]}
        ]}"#;
        // A non-string note makes its whole entry invalid.
        let releases = parse(text).unwrap();
        assert_eq!(versions(&releases), ["0.28.6"]);
        assert_eq!(releases[0].notes, ["<b>Bold</b> stays literal"]);
        let text = r#"{"schema":1,"releases":[{"version":"0.28.9","date":"2026-11-12","notes":["  Two\nlines\u0007 here. ",""]}]}"#;
        assert_eq!(parse(text).unwrap()[0].notes, ["Two lines here."]);
    }

    #[test]
    fn parse_rejects_unknown_schema_and_garbage() {
        assert!(parse(r#"{"schema":2,"releases":[]}"#).is_err());
        assert!(parse("<html>").is_err());
        assert!(parse(&" ".repeat(MAX_FILE + 1)).is_err());
    }

    #[test]
    fn merge_prefers_earlier_lists_and_sorts() {
        let mut fresh = release("0.28.9");
        fresh.notes = vec!["Fresh.".into()];
        let merged = merge(&[&[fresh.clone()], &history()]);
        assert_eq!(merged[0].version, "0.29.0-beta.1");
        assert_eq!(merged.iter().find(|r| r.version == "0.28.9"), Some(&fresh));
        assert_eq!(merged.len(), history().len());
    }

    #[test]
    fn manifest_notes_become_one_note_per_line() {
        let release = from_manifest_notes("0.28.10", Some("First.\n\nSecond.\n")).unwrap();
        assert_eq!(release.notes, ["First.", "Second."]);
        assert!(release.date.is_empty());
        assert!(from_manifest_notes("0.28.10", Some("  \n")).is_none());
    }

    fn temp_root() -> PathBuf {
        let root = std::env::temp_dir().join(format!("sonojot-changelog-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&root).unwrap();
        root
    }

    #[test]
    fn first_start_records_version_without_pending_notes() {
        let root = temp_root();
        let changelog = Changelog::start(&root, "0.28.9", None, UpdateChannel::Stable).unwrap();
        let state = changelog.get().unwrap();
        assert!(state.pending.is_none());
        // Updates can still show this build's own notes.
        let own = state.latest.unwrap();
        assert_eq!((own.from, versions(&own.releases)), (None, vec!["0.28.9"]));
        let other = temp_root();
        let unknown = Changelog::start(&other, "0.0.1", None, UpdateChannel::Stable).unwrap();
        assert!(unknown.get().unwrap().latest.is_none());
        fs::remove_dir_all(other).unwrap();
        let again = Changelog::start(&root, "0.28.9", None, UpdateChannel::Stable).unwrap();
        assert!(again.get().unwrap().pending.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn update_start_shows_staged_and_bundled_notes_once() {
        let root = temp_root();
        let before = Changelog::start(&root, "0.28.8", None, UpdateChannel::Nightly).unwrap();
        let staged = Release {
            version: "0.28.11-nightly.202611130800".into(),
            date: "2026-11-13".into(),
            notes: vec!["Staged before installing.".into()],
        };
        before.stage(vec![staged.clone()]).unwrap();
        drop(before);
        let after = Changelog::start(
            &root,
            "0.28.11-nightly.202611130800",
            None,
            UpdateChannel::Nightly,
        )
        .unwrap();
        let pending = after.get().unwrap().pending.unwrap();
        assert_eq!(pending.from.as_deref(), Some("0.28.8"));
        assert_eq!(pending.releases[0], staged);
        assert!(versions(&pending.releases).contains(&"0.28.9"));
        assert!(!versions(&pending.releases).contains(&"0.28.8"));
        after.dismiss().unwrap();
        let state = after.get().unwrap();
        assert!(state.pending.is_none());
        assert_eq!(state.latest.unwrap().to, "0.28.11-nightly.202611130800");
        drop(after);
        let next_launch = Changelog::start(&root, "0.28.11-nightly.202611130800", None, UpdateChannel::Nightly).unwrap();
        assert!(next_launch.get().unwrap().pending.is_none());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn previous_version_from_crash_marker_covers_older_installs() {
        let root = temp_root();
        let changelog = Changelog::start(&root, "0.28.9", Some("0.28.8".into()), UpdateChannel::Stable).unwrap();
        let pending = changelog.get().unwrap().pending.unwrap();
        assert_eq!(versions(&pending.releases), ["0.28.9"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn offer_falls_back_to_manifest_notes() {
        let root = temp_root();
        let changelog = Changelog::start(&root, "0.28.9", None, UpdateChannel::Stable).unwrap();
        let offer = changelog.for_offer(Some(Vec::new()), "0.28.8", "0.28.12", Some("Fixed a thing."), UpdateChannel::Stable);
        assert_eq!(versions(&offer.releases), ["0.28.12", "0.28.9"]);
        assert_eq!(offer.releases[0].notes, ["Fixed a thing."]);
        fs::remove_dir_all(root).unwrap();
    }
}
