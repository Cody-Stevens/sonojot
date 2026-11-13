use crate::{
    domain::VocabularyEntry,
    store::Store,
    sync_account::Credentials,
    sync_crypto::{self as crypto, Keyring},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeSet;
use zeroize::Zeroizing;
const STATE: &str = "sync-state.json";
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Metadata {
    pub account: String,
    pub server: String,
    pub rev: u64,
    pub ring_rev: u64,
    pub epoch: u64,
    pub last_sync: u64,
    #[serde(default)]
    pub cursor: u64,
    pub base: Vec<VocabularyEntry>,
    pub conflicts: Vec<VocabularyEntry>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Document {
    schema: u32,
    entries: Vec<VocabularyEntry>,
}
#[derive(Default)]
pub struct Merge {
    pub entries: Vec<VocabularyEntry>,
    pub conflicts: Vec<VocabularyEntry>,
}
pub fn merge(
    base: &[VocabularyEntry],
    local: &[VocabularyEntry],
    remote: &[VocabularyEntry],
) -> Merge {
    let ids: BTreeSet<&str> = base
        .iter()
        .chain(local)
        .chain(remote)
        .map(|e| e.id.as_str())
        .collect();
    let mut result = Merge::default();
    for id in ids {
        let b = base.iter().find(|e| e.id == id);
        let l = local.iter().find(|e| e.id == id);
        let r = remote.iter().find(|e| e.id == id);
        let chosen = if l == r {
            l
        } else if l == b {
            r
        } else if r == b {
            l
        } else {
            match (l, r) {
                (Some(l), Some(r)) => {
                    result.conflicts.push(l.clone());
                    Some(r)
                }
                (Some(l), None) => Some(l),
                (None, Some(r)) => Some(r),
                (None, None) => None,
            }
        };
        if let Some(e) = chosen {
            result.entries.push(e.clone())
        }
    }
    // Stable ids make identical additions converge regardless of pull order.
    let mut unique: Vec<VocabularyEntry> = Vec::new();
    for entry in result.entries {
        if !unique.iter().any(|e| same_content(e, &entry)) {
            unique.push(entry)
        }
    }
    result.entries = unique;
    result
}
fn same_content(a: &VocabularyEntry, b: &VocabularyEntry) -> bool {
    let mut b = b.clone();
    b.id = a.id.clone();
    *a == b
}
pub fn metadata(store: &Store, c: &Credentials) -> Result<Metadata, String> {
    let path = store.root().join(STATE);
    if !path.exists() {
        return Ok(Metadata {
            account: c.account.clone(),
            server: c.server.clone(),
            ..Default::default()
        });
    }
    let m: Metadata = store.read(&path)?;
    if m.account != c.account || m.server != c.server {
        return Ok(Metadata {
            account: c.account.clone(),
            server: c.server.clone(),
            ..Default::default()
        });
    }
    Ok(m)
}
fn parse(data: &[u8]) -> Result<Vec<VocabularyEntry>, String> {
    let v: Value = serde_json::from_slice(data).map_err(|_| "Synced vocabulary is invalid.")?;
    if let Some(entries) = v["entries"].as_array() {
        for entry in entries {
            let fields = entry.as_object().ok_or("Invalid vocabulary entry.")?;
            if fields.keys().any(|k| {
                ![
                    "id", "kind", "heard", "written", "enabled", "aliases", "context", "scope",
                ]
                .contains(&k.as_str())
            }) {
                return Err("Update SonoJot before syncing this vocabulary schema.".into());
            }
        }
    }
    let d: Document = serde_json::from_value(v)
        .map_err(|_| "Update SonoJot before syncing this vocabulary schema.")?;
    if d.schema != 1 {
        return Err("Update SonoJot before syncing this vocabulary schema.".into());
    }
    crate::store::validate_vocabulary(&d.entries)?;
    Ok(d.entries)
}
fn vocabulary_key(ring: &Keyring, kid: u32, rev: u64) -> Result<&crypto::Secret, String> {
    if rev > ring.vocabulary_cutover && kid != ring.active {
        return Err(
            "An old encryption key cannot authenticate vocabulary written after re-keying.".into(),
        );
    }
    ring.keys
        .get(&kid)
        .ok_or_else(|| "Unlock the latest sync keys before syncing.".into())
}
pub(crate) struct RotationUsage {
    pub vocabulary_rev: u64,
    pub cursor: u64,
    pub keys: BTreeSet<u32>,
    pub objects: std::collections::BTreeMap<String, u64>,
}
pub(crate) async fn rotation_usage(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
) -> Result<RotationUsage, String> {
    let client = c.client()?;
    let _: Value = client
        .json(
            Method::POST,
            "/v1/rekey/prepare",
            Some(&serde_json::json!({"epoch":ring.epoch})),
        )
        .await?;
    let result = rotation_usage_inner(store, c, ring).await;
    // The server also expires the lease after ten minutes or an epoch change.
    let _ = client
        .json::<Value>(Method::DELETE, "/v1/rekey/prepare", None)
        .await;
    result
}
async fn rotation_usage_inner(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
) -> Result<RotationUsage, String> {
    let meta = metadata(store, c)?;
    let client = c.client()?.for_rotation();
    let (_, bytes) = client.read_with_retry("/v1/inventory").await?;
    let inventory: Value =
        serde_json::from_slice(&bytes).map_err(|_| "Invalid object inventory.")?;
    let cursor = inventory["account"]["seq"]
        .as_u64()
        .ok_or("Invalid object inventory.")?;
    if cursor < meta.cursor || inventory["account"]["epoch"].as_u64() != Some(ring.epoch) {
        return Err("The object inventory changed. Unlock and retry re-keying.".into());
    }
    let mut objects = inventory["objects"]
        .as_array()
        .ok_or("Invalid object inventory.")?
        .clone();
    let mut page = inventory.clone();
    while page["hasMore"] == true {
        let after = objects
            .last()
            .and_then(|v| v["id"].as_str())
            .ok_or("Invalid inventory page.")?
            .to_owned();
        if after.len() > 256
            || !after
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_/".contains(&b))
        {
            return Err("Invalid inventory cursor.".into());
        }
        let (_, bytes) = client
            .read_with_retry(&format!("/v1/inventory?after={after}"))
            .await?;
        page = serde_json::from_slice(&bytes).map_err(|_| "Invalid object inventory.")?;
        if page["account"]["seq"].as_u64() != Some(cursor)
            || page["account"]["epoch"].as_u64() != Some(ring.epoch)
        {
            return Err("Inventory changed during re-keying. Retry.".into());
        }
        let more = page["objects"]
            .as_array()
            .ok_or("Invalid inventory page.")?;
        if (more.is_empty() && page["hasMore"] == true)
            || more
                .first()
                .and_then(|v| v["id"].as_str())
                .is_some_and(|id| id <= after.as_str())
            || objects.len() + more.len() > 250000
        {
            return Err("Invalid or oversized inventory.".into());
        }
        objects.extend(more.iter().cloned());
    }
    let mut usage = RotationUsage {
        vocabulary_rev: 0,
        cursor,
        keys: BTreeSet::new(),
        objects: Default::default(),
    };
    let mut ids = BTreeSet::new();
    for object in &objects {
        let id = object["id"].as_str().ok_or("Invalid object id.")?;
        let rev = object["rev"]
            .as_u64()
            .filter(|rev| *rev > 0)
            .ok_or("Invalid object revision.")?;
        if id.is_empty()
            || id.len() > 256
            || !id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_/".contains(&b))
            || !ids.insert(id)
        {
            return Err("Invalid object inventory.".into());
        }
        if id == "vocabulary" {
            usage.vocabulary_rev = rev;
        }
        if matches!(object["deleted"].as_u64(), Some(1 | 2 | 3 | 4)) {
            continue;
        }
        if object["deleted"] != 0 {
            return Err("Invalid object state.".into());
        }
        if id.starts_with("blob/") {
            // The authenticated head binds audio bytes. Keep its possible decryption
            // keys without fetching gigabytes of audio during device removal.
            usage.keys.extend(ring.keys.keys().copied());
            continue;
        }
        let path = match id.strip_prefix("blob/") {
            Some(blob) => format!("/v1/blobs/{blob}"),
            None => format!("/v1/objects/{id}"),
        };
        let (actual_rev, bytes) = client.read_with_retry(&path).await?;
        if actual_rev != rev {
            return Err("An object changed. Retry re-keying.".into());
        }
        let kid = crypto::envelope_kid(&bytes)?;
        let key = if id == "vocabulary" {
            vocabulary_key(ring, kid, rev)?
        } else {
            ring.object_key(id, kid, rev)?
        };
        let plain = crypto::open(key, &c.account, id, rev, &bytes)?;
        if id == "vocabulary" {
            parse(&plain)?;
        }
        usage.keys.insert(kid);
        if id != "vocabulary" {
            usage.objects.insert(id.into(), rev);
        }
    }
    if usage.vocabulary_rev < meta.rev {
        return Err("The service returned older vocabulary.".into());
    }
    Ok(usage)
}
pub async fn synchronize(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    ring_rev: u64,
) -> Result<Metadata, String> {
    let client = c.client()?;
    let mut meta = metadata(store, c)?;
    if ring_rev < meta.ring_rev {
        return Err(
            "The server returned older encryption keys. Your local words are unchanged.".into(),
        );
    }
    let account: Value = client.json(Method::GET, "/v1/poll", None).await?;
    let cursor = account["seq"].as_u64().ok_or("Invalid sync cursor.")?;
    if cursor < meta.cursor {
        return Err("The server returned an older change cursor.".into());
    }
    let epoch = account["epoch"].as_u64().ok_or("Invalid account epoch.")?;
    if epoch != ring.epoch {
        return Err(
            "Encryption keys changed. Refresh or recover this device before syncing.".into(),
        );
    }
    if epoch < meta.epoch {
        return Err("The server returned an older account epoch.".into());
    }
    let unchanged = meta.last_sync > 0 && cursor == meta.cursor;
    if unchanged && store.get_vocabulary()? == meta.base {
        return Ok(meta);
    }
    for attempt in 0..4 {
        let local = store.get_vocabulary()?;
        let (remote_rev, remote) = if unchanged && attempt == 0 {
            (meta.rev, meta.base.clone())
        } else {
            match client
                .request(Method::GET, "/v1/objects/vocabulary", None, None, None, 0)
                .await
            {
                Ok((rev, bytes)) => {
                    if rev < meta.rev {
                        return Err(
                            "The server returned older vocabulary. Your local words are unchanged."
                                .into(),
                        );
                    }
                    let kid = crypto::envelope_kid(&bytes)?;
                    let key = vocabulary_key(ring, kid, rev)?;
                    let plain = crypto::open(key, &c.account, "vocabulary", rev, &bytes)?;
                    (rev, parse(&plain)?)
                }
                Err(e) if e.status == 404 && meta.rev == 0 => (0, Vec::new()),
                Err(e) => return Err(e.into()),
            }
        };
        let merged = merge(&meta.base, &local, &remote);
        let mut final_rev = remote_rev;
        if merged.entries != remote || remote_rev == 0 {
            let rev = remote_rev
                .checked_add(1)
                .ok_or("Vocabulary revision is exhausted.")?;
            let plain = Zeroizing::new(
                serde_json::to_vec(&Document {
                    schema: 1,
                    entries: merged.entries.clone(),
                })
                .map_err(|_| "Cannot encode vocabulary.")?,
            );
            let key = ring.keys.get(&ring.active).ok_or("No active sync key.")?;
            let encrypted = crypto::seal(key, ring.active, &c.account, "vocabulary", rev, &plain)?;
            match client
                .request(
                    Method::PUT,
                    "/v1/objects/vocabulary",
                    None,
                    Some(encrypted),
                    Some(remote_rev),
                    epoch,
                )
                .await
            {
                Ok((_, body)) => {
                    let ack: Value =
                        serde_json::from_slice(&body).map_err(|_| "Invalid sync receipt.")?;
                    if ack["rev"].as_u64() != Some(rev) {
                        return Err("Unexpected sync revision.".into());
                    }
                    final_rev = rev;
                }
                Err(e) if e.status == 412 => continue,
                Err(e) => return Err(e.into()),
            }
        }
        meta.cursor = cursor;
        meta.rev = final_rev;
        meta.ring_rev = ring_rev;
        meta.epoch = epoch;
        meta.base = merged.entries.clone();
        meta.last_sync = time::OffsetDateTime::now_utc().unix_timestamp().max(0) as u64;
        for conflict in merged.conflicts {
            if !meta.conflicts.contains(&conflict) {
                meta.conflicts.push(conflict)
            }
        }
        // Re-read and merge while holding the existing vocabulary owner lock. A GUI edit
        // made during HTTP stays local and uploads on the next pass.
        store.commit_synced_vocabulary(&local, &merged.entries, &mut meta)?;
        return Ok(meta);
    }
    Err("Words changed repeatedly on another device. Your local words are safe; retry sync.".into())
}
pub fn resolve(store: &Store, c: &Credentials, id: &str) -> Result<(), String> {
    let mut meta = metadata(store, c)?;
    let chosen = meta
        .conflicts
        .iter()
        .find(|e| e.id == id)
        .cloned()
        .ok_or("This conflict was already resolved.")?;
    store.resolve_sync_conflict(&chosen, &mut meta)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::VocabularyKind;
    #[test]
    fn removed_key_cannot_authenticate_a_revision_after_the_rotation_boundary() {
        let mut ring = Keyring::new();
        ring.rotate().unwrap();
        ring.vocabulary_cutover = 7;
        assert!(vocabulary_key(&ring, 1, 7).is_ok());
        assert!(vocabulary_key(&ring, 1, 8).is_err());
        assert!(vocabulary_key(&ring, 2, 8).is_ok());
    }
    #[test]
    fn corrupt_pending_journal_is_quarantined_without_blocking_startup() {
        let root = std::env::temp_dir().join(format!("sj-corrupt-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let words = vec![entry("local", "keep these words")];
        store.save_vocabulary(&words).unwrap();
        std::fs::write(store.root().join("sync-pending.json"), b"{broken JSON").unwrap();
        drop(store);
        let reopened = Store::open(Some(root.clone())).unwrap();
        assert_eq!(reopened.get_vocabulary().unwrap(), words);
        assert!(!reopened.root().join("sync-pending.json").exists());
        assert!(reopened.root().join("sync-notice.json").exists());
        assert!(std::fs::read_dir(reopened.root()).unwrap().any(|p| p
            .unwrap()
            .path()
            .extension()
            .is_some_and(|e| e == "corrupt")));
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn interrupted_commit_replays_without_losing_the_local_conflict() {
        let root = std::env::temp_dir().join(format!("sj-journal-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let local = entry("id", "local");
        let remote = entry("id", "remote");
        store.save_vocabulary(&[remote.clone()]).unwrap();
        let meta = Metadata {
            account: "test".into(),
            server: "https://example.invalid".into(),
            rev: 2,
            base: vec![remote.clone()],
            conflicts: vec![local.clone()],
            ..Default::default()
        };
        store.write(&store.root().join("sync-pending.json"),&serde_json::json!({"before":[local.clone()],"remote":[remote.clone()],"metadata":meta})).unwrap();
        drop(store);
        let reopened = Store::open(Some(root.clone())).unwrap();
        let saved: Metadata = reopened
            .read(&reopened.root().join("sync-state.json"))
            .unwrap();
        assert_eq!(saved.conflicts, vec![local]);
        assert_eq!(reopened.get_vocabulary().unwrap(), vec![remote]);
        assert!(!root.join("sync-pending.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn entry(id: &str, text: &str) -> VocabularyEntry {
        VocabularyEntry {
            id: id.into(),
            kind: VocabularyKind::Hint,
            heard: text.into(),
            written: text.into(),
            enabled: true,
            aliases: vec![],
            context: vec![],
            scope: None,
        }
    }
    #[test]
    fn edits_to_different_entries_merge() {
        let b = vec![entry("a", "one"), entry("b", "two")];
        let l = vec![entry("a", "local"), b[1].clone()];
        let r = vec![b[0].clone(), entry("b", "remote")];
        let m = merge(&b, &l, &r);
        assert_eq!(m.entries, vec![l[0].clone(), r[1].clone()]);
        assert!(m.conflicts.is_empty())
    }
    #[test]
    fn simultaneous_edit_keeps_remote_and_local_conflict() {
        let b = vec![entry("a", "base")];
        let l = vec![entry("a", "local")];
        let r = vec![entry("a", "remote")];
        let m = merge(&b, &l, &r);
        assert_eq!(m.entries, r);
        assert_eq!(m.conflicts, l)
    }
    #[test]
    fn deletion_against_edit_keeps_the_edit() {
        let b = vec![entry("a", "base")];
        let edit = vec![entry("a", "new")];
        assert_eq!(merge(&b, &[], &edit).entries, edit);
        assert_eq!(merge(&b, &edit, &[]).entries, edit);
        assert!(merge(&b, &[], &b).entries.is_empty());
        assert!(merge(&b, &b, &[]).entries.is_empty())
    }
    #[test]
    fn identical_additions_collapse_deterministically() {
        let a = entry("a", "same");
        let b = entry("b", "same");
        assert_eq!(
            merge(&[], &[b.clone()], &[a.clone()]).entries,
            vec![a.clone()]
        );
        assert_eq!(merge(&[], &[a.clone()], &[b]).entries, vec![a])
    }
    #[test]
    fn schema_gate_refuses_unknown_data() {
        assert!(parse(br#"{"schema":2,"entries":[]}"#).is_err());
        assert!(parse(br#"{"schema":1,"future":1,"entries":[]}"#).is_err());
        assert!(parse(br#"{"schema":1,"entries":[{"future":true}]}"#).is_err())
    }
    #[test]
    fn local_edit_during_pull_is_preserved() {
        let root = std::env::temp_dir().join(format!("sj-merge-{}", uuid::Uuid::new_v4()));
        let s = Store::open(Some(root.clone())).unwrap();
        let before = vec![entry("a", "one")];
        s.save_vocabulary(&[entry("a", "edited-during-network")])
            .unwrap();
        let mut meta = Metadata::default();
        s.commit_synced_vocabulary(&before, &[entry("a", "remote")], &mut meta)
            .unwrap();
        assert_eq!(s.get_vocabulary().unwrap()[0].heard, "remote");
        assert_eq!(meta.conflicts[0].heard, "edited-during-network");
        std::fs::remove_dir_all(root).unwrap()
    }
    #[test]
    fn repeated_merge_is_idempotent() {
        let base = vec![entry("a", "base")];
        let m = merge(&base, &[entry("a", "left")], &[entry("a", "right")]);
        assert_eq!(merge(&m.entries, &m.entries, &m.entries).entries, m.entries)
    }
    #[test]
    #[ignore = "Run sync-server npm run test:e2e against isolated Miniflare"]
    fn two_device_miniflare() {
        let url = std::env::var("SONOJOT_SYNC_E2E_URL").unwrap();
        let account = std::env::var("SONOJOT_SYNC_E2E_ACCOUNT").unwrap();
        let root = std::env::temp_dir().join(format!("sj-e2e-{}", uuid::Uuid::new_v4()));
        let a = Store::open(Some(root.join("a"))).unwrap();
        let b = Store::open(Some(root.join("b"))).unwrap();
        let mut ca = Credentials::new(url.clone());
        ca.account = account.clone();
        ca.device_id = "a".into();
        ca.session = std::env::var("SONOJOT_SYNC_E2E_A").unwrap();
        let mut cb = Credentials::new(url.clone());
        cb.account = account.clone();
        cb.device_id = "b".into();
        cb.session = std::env::var("SONOJOT_SYNC_E2E_B").unwrap();
        let lost = Store::open(Some(root.join("lost"))).unwrap();
        let mut clost = Credentials::new(url);
        clost.account = account;
        clost.device_id = "lost-device".into();
        clost.session = std::env::var("SONOJOT_SYNC_E2E_LOST").unwrap();
        use base64::Engine;
        let identities: Value =
            serde_json::from_str(&std::env::var("SONOJOT_SYNC_E2E_KEYS").unwrap()).unwrap();
        for c in [&mut ca, &mut cb, &mut clost] {
            let d = &identities[&c.device_id];
            c.device = crypto::DeviceKey {
                secret: base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(d["secret"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
                public: base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(d["publicKey"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
            };
        }
        crate::secret_store::save(&a, &ca).unwrap();
        crate::secret_store::save(&b, &cb).unwrap();
        crate::secret_store::save(&lost, &clost).unwrap();
        async fn pair(a: &Store, b: &Store) {
            use crate::sync_account::action;
            let id = action(b, "pair_start", serde_json::json!({}))
                .await
                .unwrap()["id"]
                .as_str()
                .unwrap()
                .to_owned();
            let arg = serde_json::json!({"id":id});
            action(a, "pair_accept", arg.clone()).await.unwrap();
            // Opening either screen again must keep its committed nonce.
            assert_eq!(action(b, "pair_start", serde_json::json!({})).await.unwrap()["id"], id);
            action(a, "pair_accept", arg.clone()).await.unwrap();
            action(b, "pair_poll", arg.clone()).await.unwrap();
            action(a, "pair_poll", arg.clone()).await.unwrap();
            assert!(action(a, "pair_approve", serde_json::json!({"id":id,"confirmed":true})).await.is_err());
            let on_b = action(b, "pair_poll", arg.clone()).await.unwrap();
            let on_a = action(a, "pair_poll", arg.clone()).await.unwrap();
            assert_eq!(on_a["code"], on_b["code"]);
            assert!(action(b, "pair_approve", serde_json::json!({"id":id,"confirmed":true})).await.is_err());
            assert!(action(a, "pair_approve", arg.clone()).await.is_err());
            assert_ne!(action(b, "pair_poll", arg.clone()).await.unwrap()["unlocked"], true);
            action(
                a,
                "pair_approve",
                serde_json::json!({"id":id,"confirmed":true}),
            )
            .await
            .unwrap();
            let poll = action(b, "pair_poll", arg.clone()).await.unwrap();
            assert_eq!(poll["unlocked"], true);
        }
        let runtime = tokio::runtime::Runtime::new().unwrap();
        runtime.block_on(async {
            crate::sync_account::action(&a, "setup", serde_json::json!({"confirmed":true}))
                .await
                .unwrap();
            // Pair confirmation must cancel a provider-requested reset immediately.
            crate::sync_account::action(&b, "reset_request", serde_json::json!({"confirmed":true}))
                .await
                .unwrap();
            pair(&a, &b).await;
            pair(&a, &lost).await;
            crate::sync_account::action(&lost, "now", serde_json::json!({})).await.unwrap();
            let poll: Value = cb
                .client()
                .unwrap()
                .json(Method::GET, "/v1/poll", None)
                .await
                .unwrap();
            assert!(poll["ready_at"].is_null());
            assert_eq!(
                crate::sync_account::status(&b).unwrap()["resetCancelled"],
                true
            );
            let mut ca = crate::secret_store::load::<Credentials>(&a)
                .unwrap()
                .unwrap();
            let mut cb = crate::secret_store::load::<Credentials>(&b)
                .unwrap()
                .unwrap();
            // Recovery cancels a pending reset, even without a subsequent sync.
            crate::sync_account::action(&b, "reset_request", serde_json::json!({"confirmed":true}))
                .await
                .unwrap();
            cb.ak = None;
            crate::secret_store::save(&b, &cb).unwrap();
            crate::sync_account::action(
                &b,
                "recovery_use",
                serde_json::json!({"code":ca.recovery.as_ref().unwrap()}),
            )
            .await
            .unwrap();
            let poll: Value = ca
                .client()
                .unwrap()
                .json(Method::GET, "/v1/poll", None)
                .await
                .unwrap();
            assert!(poll["ready_at"].is_null());
            cb = crate::secret_store::load::<Credentials>(&b)
                .unwrap()
                .unwrap();
            // Every unlocked sync cancels before uploads, independent of the requester's intent.
            crate::sync_account::action(&b, "reset_request", serde_json::json!({"confirmed":true}))
                .await
                .unwrap();
            crate::sync_account::action(&a, "now", serde_json::json!({}))
                .await
                .unwrap();
            let poll: Value = ca
                .client()
                .unwrap()
                .json(Method::GET, "/v1/poll", None)
                .await
                .unwrap();
            assert!(poll["ready_at"].is_null());
            assert_eq!(
                crate::sync_account::status(&a).unwrap()["resetCancelled"],
                true
            );
            let (ring, ring_rev) = crate::sync_account::keyring(&a, &mut ca).await.unwrap();
            a.save_vocabulary(&[entry("shared", "Asterlume")]).unwrap();
            synchronize(&a, &ca, &ring, ring_rev).await.unwrap();
            synchronize(&b, &cb, &ring, ring_rev).await.unwrap();
            assert_eq!(b.get_vocabulary().unwrap(), a.get_vocabulary().unwrap());
            let before: Value = cb
                .client()
                .unwrap()
                .json(Method::GET, "/__test/requests", None)
                .await
                .unwrap();
            for _ in 0..3 {
                synchronize(&b, &cb, &ring, ring_rev).await.unwrap();
            }
            let after: Value = cb
                .client()
                .unwrap()
                .json(Method::GET, "/__test/requests", None)
                .await
                .unwrap();
            assert_eq!(
                before["objectDownloads"], after["objectDownloads"],
                "unchanged cursors must not download envelopes"
            );
            a.save_vocabulary(&[entry("shared", "Asterlume edited on A")])
                .unwrap();
            b.save_vocabulary(&[entry("shared", "Asterlume edited on B")])
                .unwrap();
            synchronize(&a, &ca, &ring, ring_rev).await.unwrap();
            let meta = synchronize(&b, &cb, &ring, ring_rev).await.unwrap();
            assert_eq!(meta.conflicts.len(), 1);
            assert_eq!(b.get_vocabulary().unwrap(), a.get_vocabulary().unwrap());
            // A removed key holder and the service could forge the OLD roster.
            // The rotating device must not wrap its new AK to that roster.
            let mut poisoned = ring.clone();
            // Fill the cap with unused keys. The current vocabulary still needs key 1.
            for kid in 2..=64 {
                poisoned.keys.insert(kid, crypto::Secret::random());
            }
            // Authenticate the fixture's local checkpoint before presenting this keyring.
            ca.keyset = Some(crypto::keyset_pin(&poisoned));
            crate::secret_store::save(&a, &ca).unwrap();
            cb.keyset = ca.keyset;
            crate::secret_store::save(&b, &cb).unwrap();
            poisoned
                .devices
                .insert("colluding-device".into(), crypto::DeviceKey::new().public);
            ca.client()
                .unwrap()
                .put_wrap(
                    "keyring",
                    "main",
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
                        poisoned
                            .seal(ca.ak.as_ref().unwrap(), &ca.account, ring_rev + 1)
                            .unwrap(),
                    ),
                    ring_rev,
                )
                .await
                .unwrap();
            a.save_vocabulary(&[entry("shared", "Asterlume private pending removal")])
                .unwrap();
            crate::sync_account::action(
                &a,
                "remove",
                serde_json::json!({"device":"lost-device","confirmed":true}),
            )
            .await
            .unwrap();
            let (old_rev, old_bytes) = ca
                .client()
                .unwrap()
                .request(Method::GET, "/v1/objects/vocabulary", None, None, None, 0)
                .await
                .unwrap();
            let old_plain = crypto::open(
                ring.keys.get(&ring.active).unwrap(),
                &ca.account,
                "vocabulary",
                old_rev,
                &old_bytes,
            )
            .unwrap();
            assert!(
                !String::from_utf8_lossy(&old_plain).contains("private pending removal"),
                "re-key preparation must not upload pending edits under the old key"
            );
            let current = crate::secret_store::load::<Credentials>(&a)
                .unwrap()
                .unwrap();
            let wraps = current.client().unwrap().wraps().await.unwrap();
            let recipients = wraps
                .iter()
                .filter(|w| w.kind == "device")
                .map(|w| w.reference.as_str())
                .collect::<Vec<_>>();
            assert_eq!(
                recipients,
                vec!["a", "b"],
                "only pinned remaining peers receive a new account key"
            );
            assert!(crate::sync_account::action(&lost, "now", serde_json::json!({})).await.is_err());
            // No pairing, recovery or confirmation: the next ordinary sync
            // authenticates the transition from the already pinned computer.
            crate::sync_account::action(&b, "now", serde_json::json!({})).await.unwrap();
            cb = crate::secret_store::load::<Credentials>(&b)
                .unwrap()
                .unwrap();
            let (next, rev) = crate::sync_account::keyring(&b, &mut cb).await.unwrap();
            assert_eq!(next.epoch, 2);
            assert_eq!(next.active, 65);
            assert_eq!(next.keys.len(), 2, "unused keys must be pruned at the cap");
            assert!(next.keys.contains_key(&1));
            assert!(!ring.keys.contains_key(&65));
            b.save_vocabulary(&[entry("shared", "Asterlume after re-key")])
                .unwrap();
            synchronize(&b, &cb, &next, rev).await.unwrap();
            let mut ca = crate::secret_store::load::<Credentials>(&a)
                .unwrap()
                .unwrap();
            let (next, rev) = crate::sync_account::keyring(&a, &mut ca).await.unwrap();
            synchronize(&a, &ca, &next, rev).await.unwrap();
            assert_eq!(a.get_vocabulary().unwrap(), b.get_vocabulary().unwrap());
            // A rollback to the earlier data-key/epoch must fail before writing.
            assert!(synchronize(&a, &ca, &ring, ring_rev).await.is_err());
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
