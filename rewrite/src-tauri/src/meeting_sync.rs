//! Incremental, account-scoped meeting synchronization. HTTP never holds Store's lock.
use crate::{
    meeting_layers::{self as layers, Head, Layers, SCHEMA},
    meeting_sync_audio as audio,
    meetings::{layers as local, Meetings},
    store::Store,
    sync_account::Credentials,
    sync_crypto::{self as crypto, Keyring},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const STATE: &str = "meeting-sync-state.json";
fn encode_document(value: &Value) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    use std::io::Write;
    let plain =
        zeroize::Zeroizing::new(serde_json::to_vec(value).map_err(|_| "Cannot encode meeting.")?);
    if plain.len() as u64 > crate::store::MAX_JSON_BYTES {
        return Err("Meeting layer exceeds the local size limit.".into());
    }
    if plain.len() < 8192 {
        return Ok(plain);
    }
    let mut encoder =
        flate2::write::ZlibEncoder::new(b"SJM1".to_vec(), flate2::Compression::fast());
    encoder
        .write_all(&plain)
        .map_err(|_| "Cannot compress meeting layer.")?;
    Ok(zeroize::Zeroizing::new(
        encoder
            .finish()
            .map_err(|_| "Cannot finish meeting layer compression.")?,
    ))
}
fn decode_document(plain: &[u8]) -> Result<Value, String> {
    use std::io::Read;
    let mut unpacked = zeroize::Zeroizing::new(Vec::new());
    let bytes = if let Some(compressed) = plain.strip_prefix(b"SJM1") {
        flate2::read::ZlibDecoder::new(compressed)
            .take(crate::store::MAX_JSON_BYTES + 1)
            .read_to_end(&mut unpacked)
            .map_err(|_| "Invalid compressed meeting layer.")?;
        if unpacked.len() as u64 > crate::store::MAX_JSON_BYTES {
            return Err("Meeting layer exceeds the local size limit.".into());
        }
        unpacked.as_slice()
    } else {
        if plain.len() as u64 > crate::store::MAX_JSON_BYTES {
            return Err("Meeting layer exceeds the local size limit.".into());
        }
        plain
    };
    serde_json::from_slice(bytes).map_err(|_| "Invalid encrypted meeting document.".into())
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Track {
    pub key: String,
    pub head: Option<Head>,
    pub notes: BTreeMap<String, Value>,
    pub revisions: BTreeMap<String, u64>,
    pub blobs: BTreeSet<String>,
    #[serde(default)]
    pub parts: BTreeMap<String, Vec<String>>,
    pub status: String,
    pub conflicts: BTreeMap<String, Value>,
    pub delete_conflict: bool,
    #[serde(default)]
    pub server_version: Option<u64>,
    #[serde(default)]
    pub references: Option<Receipt>,
    #[serde(default)]
    pub held_at: u64,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Receipt {
    version: u64,
    head: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MeetingRetry {
    at: u64,
    delay: u64,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct State {
    pub account: String,
    pub server: String,
    pub cursor: u64,
    pub epoch: u64,
    pub ring_rev: u64,
    pub meetings: BTreeMap<String, Track>,
    pub used: u64,
    pub quota: u64,
    #[serde(default)]
    pub pending_pulls: BTreeSet<String>,
    #[serde(default)]
    pub push_after: String,
    #[serde(default)]
    pub errors: BTreeMap<String, String>,
    #[serde(default)]
    pub retry_at: u64,
    #[serde(default)]
    pub retry_delay: u64,
    #[serde(default)]
    pub rate_limited: bool,
    #[serde(default)]
    pub missing: BTreeSet<String>,
    #[serde(default)]
    pub retries: BTreeMap<String, MeetingRetry>,
}
#[derive(Clone, Deserialize)]
struct Object {
    id: String,
    rev: u64,
    #[serde(default)]
    deleted: u8,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Inventory {
    id: String,
    state: String,
    version: u64,
    objects: Vec<Object>,
    has_more: bool,
}
pub fn state(store: &Store, c: &Credentials) -> Result<State, String> {
    let path = store.root().join(STATE);
    if path.exists() {
        let m: State = store.read(&path)?;
        if m.account == c.account && m.server == c.server {
            return Ok(m);
        }
    }
    Ok(State {
        account: c.account.clone(),
        server: c.server.clone(),
        ..Default::default()
    })
}
pub(crate) fn reset(store: &Store, c: &Credentials) -> Result<(), String> {
    save(
        store,
        &State {
            account: c.account.clone(),
            server: c.server.clone(),
            ..Default::default()
        },
    )
}
fn save(store: &Store, m: &State) -> Result<(), String> {
    store.write(&store.root().join(STATE), m)
}
fn opaque_with(key: &crypto::Secret, id: &str) -> Result<String, String> {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    let mut h =
        <Hmac<Sha256> as Mac>::new_from_slice(&key.0).map_err(|_| "Cannot derive meeting id.")?;
    h.update(id.as_bytes());
    Ok(base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(h.finalize().into_bytes()))
}
fn opaque(c: &Credentials, id: &str) -> Result<String, String> {
    opaque_with(
        &crypto::derive(
            &c.ak.as_ref().ok_or("Unlock sync first.")?.0,
            &c.account,
            "meeting-ids",
        ),
        id,
    )
}
fn identity_matches(c: &Credentials, ring: &Keyring, key: &str, id: &str) -> Result<bool, String> {
    crate::store::validate_id(id)?;
    Ok(opaque(c, id)? == key
        || ring
            .meeting_id_keys
            .iter()
            .any(|old| opaque_with(old, id).is_ok_and(|v| v == key)))
}
fn tombstone(value: &Value, id: &str, revision: u64, tracked: u64) -> bool {
    value["schema"] == SCHEMA && value["deleted"] == true && value["id"] == id && revision > tracked
}
fn member(object: &str) -> Option<&str> {
    let mut parts = object.split('/');
    match parts.next()? {
        "m" | "blob" => parts.next(),
        _ => None,
    }
}
fn valid_key(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_".contains(&b))
}
fn path(object: &str) -> String {
    match object.strip_prefix("blob/") {
        Some(id) => format!("/v1/blobs/{id}"),
        None => format!("/v1/objects/{object}"),
    }
}
async fn get_plain(
    c: &Credentials,
    ring: &Keyring,
    object: &Object,
) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
    if !object
        .id
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"-_/".contains(&b))
        || object.id.len() > 256
    {
        return Err("Invalid meeting object id.".into());
    }
    let (rev, body) = c
        .client()?
        .request(Method::GET, &path(&object.id), None, None, None, 0)
        .await?;
    if rev < object.rev {
        return Err("The service returned an older meeting revision.".into());
    }
    if rev != object.rev {
        return Err("Meeting changed during download. Retry sync.".into());
    }
    let kid = crypto::envelope_kid(&body)?;
    let plain = crypto::open(
        ring.object_key(&object.id, kid, rev)?,
        &c.account,
        &object.id,
        rev,
        &body,
    )?;
    Ok(plain)
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextPart {
    id: String,
    bytes: usize,
    sha256: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TextManifest {
    bytes: usize,
    sha256: String,
    parts: Vec<TextPart>,
}
struct Published {
    rev: u64,
    parts: Vec<String>,
}
fn segmented(
    ring: &Keyring,
    object: &str,
    rev: u64,
    plain: &[u8],
) -> Result<(Vec<u8>, Vec<(String, Vec<u8>)>), String> {
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    if plain.len() <= 480 * 1024 {
        return Ok((plain.to_vec(), vec![]));
    }
    let meeting = member(object).ok_or("Invalid segmented meeting.")?;
    let key = ring.keys.get(&ring.active).ok_or("Missing meeting key.")?;
    let mut parts = Vec::new();
    let mut bodies = Vec::new();
    for (i, bytes) in plain.chunks(128 * 1024).enumerate() {
        let mut h = <Hmac<Sha256> as Mac>::new_from_slice(&key.0)
            .map_err(|_| "Cannot derive text part.")?;
        h.update(b"meeting-text-part/v1");
        h.update(object.as_bytes());
        h.update(&rev.to_be_bytes());
        h.update(&(i as u64).to_be_bytes());
        h.update(bytes);
        let id = format!("{:x}", h.finalize().into_bytes());
        parts.push(TextPart {
            id: id.clone(),
            bytes: bytes.len(),
            sha256: audio::hash(bytes),
        });
        bodies.push((format!("m/{meeting}/part/{id}"), bytes.to_vec()));
    }
    let manifest = TextManifest {
        bytes: plain.len(),
        sha256: audio::hash(plain),
        parts,
    };
    let mut root = b"SJM2".to_vec();
    root.extend(serde_json::to_vec(&manifest).map_err(|_| "Cannot encode text manifest.")?);
    Ok((root, bodies))
}
async fn get_parts(
    c: &Credentials,
    ring: &Keyring,
    object: &Object,
) -> Result<(Value, Vec<String>), String> {
    let plain = get_plain(c, ring, object).await?;
    let Some(encoded) = plain.strip_prefix(b"SJM2") else {
        return Ok((decode_document(&plain)?, vec![]));
    };
    let manifest: TextManifest =
        serde_json::from_slice(encoded).map_err(|_| "Invalid text manifest.")?;
    if manifest.bytes > crate::store::MAX_JSON_BYTES as usize + 65536
        || manifest.parts.is_empty()
        || manifest.parts.len() > 258
    {
        return Err("Segmented meeting exceeds its size limit.".into());
    }
    let meeting = member(&object.id).ok_or("Invalid segmented meeting.")?;
    let mut combined = zeroize::Zeroizing::new(Vec::new());
    let mut ids = Vec::new();
    for part in manifest.parts {
        crate::store::validate_id(&part.id)?;
        if part.bytes == 0
            || part.bytes > 128 * 1024
            || combined.len() + part.bytes > manifest.bytes
        {
            return Err("Invalid text part size.".into());
        }
        let id = format!("m/{meeting}/part/{}", part.id);
        if ids.contains(&id) {
            return Err("Duplicate text part.".into());
        }
        let bytes = get_plain(
            c,
            ring,
            &Object {
                id: id.clone(),
                rev: 1,
                deleted: 0,
            },
        )
        .await?;
        if bytes.len() != part.bytes || audio::hash(&bytes) != part.sha256 {
            return Err("Text part checksum differs.".into());
        }
        combined.extend_from_slice(&bytes);
        ids.push(id);
    }
    if combined.len() != manifest.bytes || audio::hash(&combined) != manifest.sha256 {
        return Err("Text manifest checksum differs.".into());
    }
    Ok((decode_document(&combined)?, ids))
}
async fn get(c: &Credentials, ring: &Keyring, object: &Object) -> Result<Value, String> {
    Ok(get_parts(c, ring, object).await?.0)
}
async fn put(
    c: &Credentials,
    ring: &Keyring,
    object: &str,
    previous: u64,
    value: &Value,
) -> Result<Published, crate::sync_client::Failure> {
    let convert = |message: String| crate::sync_client::Failure { status: 0, message };
    let rev = previous
        .checked_add(1)
        .ok_or_else(|| convert("Meeting revision exhausted.".into()))?;
    let plain = encode_document(value).map_err(convert)?;
    let (mut plain, mut parts) = segmented(ring, object, rev, &plain).map_err(convert)?;
    let client = c.client().map_err(convert)?;
    for (id, bytes) in &mut parts {
        let sealed = crypto::seal(
            ring.keys
                .get(&ring.active)
                .ok_or_else(|| convert("No meeting key.".into()))?,
            ring.active,
            &c.account,
            id,
            1,
            bytes,
        )
        .map_err(convert)?;
        match client
            .request(
                Method::PUT,
                &path(id),
                None,
                Some(sealed),
                Some(0),
                ring.epoch,
            )
            .await
        {
            Ok(_) => {}
            Err(e) if e.status == 412 => {
                match client
                    .request(Method::GET, &path(id), None, None, None, 0)
                    .await
                {
                    Err(missing) if missing.status == 404 => {
                        // An expired, unpublished part has a tombstone. Allocate a new
                        // immutable identity and bind it into the parent before commit.
                        *id = format!(
                            "m/{}/part/{}",
                            member(object).ok_or_else(|| convert("Invalid meeting.".into()))?,
                            uuid::Uuid::new_v4()
                        );
                        let sealed = crypto::seal(
                            &ring.keys[&ring.active],
                            ring.active,
                            &c.account,
                            id,
                            1,
                            bytes,
                        )
                        .map_err(convert)?;
                        client
                            .request(
                                Method::PUT,
                                &path(id),
                                None,
                                Some(sealed),
                                Some(0),
                                ring.epoch,
                            )
                            .await?;
                    }
                    Err(error) => return Err(error),
                    Ok((revision, body)) => {
                        let kid = crypto::envelope_kid(&body).map_err(convert)?;
                        let remote = crypto::open(
                            ring.object_key(id, kid, revision).map_err(convert)?,
                            &c.account,
                            id,
                            revision,
                            &body,
                        )
                        .map_err(convert)?;
                        if revision != 1 || remote.as_slice() != bytes {
                            return Err(convert("Immutable text part changed.".into()));
                        }
                    }
                }
            }
            Err(e) => return Err(e),
        }
    }
    if !parts.is_empty() {
        let mut manifest: TextManifest = serde_json::from_slice(&plain[4..])
            .map_err(|_| convert("Invalid text manifest.".into()))?;
        for (part, (id, _)) in manifest.parts.iter_mut().zip(&parts) {
            part.id = id.rsplit('/').next().unwrap().into();
        }
        plain = b"SJM2".to_vec();
        plain.extend(
            serde_json::to_vec(&manifest)
                .map_err(|_| convert("Cannot encode text manifest.".into()))?,
        );
    }

    let bytes = crypto::seal(
        ring.keys
            .get(&ring.active)
            .ok_or_else(|| convert("No meeting key.".into()))?,
        ring.active,
        &c.account,
        object,
        rev,
        &plain,
    )
    .map_err(convert)?;
    let (_, receipt) = c
        .client()
        .map_err(convert)?
        .request(
            Method::PUT,
            &path(object),
            None,
            Some(bytes),
            Some(previous),
            ring.epoch,
        )
        .await?;
    let receipt: Value = serde_json::from_slice(&receipt)
        .map_err(|_| convert("Invalid meeting sync receipt.".into()))?;
    if receipt["rev"].as_u64() != Some(rev) {
        return Err(convert("Unexpected meeting revision.".into()));
    }
    Ok(Published {
        rev,
        parts: parts.into_iter().map(|p| p.0).collect(),
    })
}
async fn inventory(c: &Credentials, key: &str) -> Result<Inventory, String> {
    let mut after = String::new();
    let mut combined = None;
    for _ in 0..1000 {
        let page: Inventory = c
            .client()?
            .json(
                Method::GET,
                &format!("/v1/meetings/{key}?after={after}"),
                None,
            )
            .await?;
        if page.id != key {
            return Err("Meeting inventory has a different id.".into());
        }
        if let Some(ref mut all) = combined {
            let all: &mut Inventory = all;
            if all.version != page.version || all.state != page.state {
                return Err("Meeting changed during inventory. Retry sync.".into());
            }
            all.objects.extend(page.objects.clone());
        } else {
            combined = Some(Inventory {
                id: page.id.clone(),
                state: page.state.clone(),
                version: page.version,
                objects: page.objects.clone(),
                has_more: false,
            });
        }
        if !page.has_more {
            return Ok(combined.unwrap());
        }
        let next = page
            .objects
            .last()
            .ok_or("Invalid meeting inventory page.")?
            .id
            .clone();
        if next <= after {
            return Err("Meeting inventory did not advance.".into());
        }
        after = next;
    }
    Err("Meeting inventory exceeds the supported limit.".into())
}
fn documents(l: &Layers, key: &str) -> Result<BTreeMap<String, Value>, String> {
    let mut docs = BTreeMap::new();
    let prefix = format!("m/{key}");
    macro_rules! encode {
        ($map:expr,$name:expr) => {
            for (id, value) in $map {
                docs.insert(
                    format!("{prefix}/{}/{}", $name, id),
                    serde_json::to_value(value).map_err(|_| "Cannot encode meeting layer.")?,
                );
            }
        };
    }
    encode!(&l.raw, "raw");
    encode!(&l.clean, "clean");
    for (id, b) in &l.batches {
        docs.insert(format!("{prefix}/ops/{}/{}", b.base, id), json!(b));
    }
    for (id, s) in &l.snapshots {
        docs.insert(format!("{prefix}/snap/{}/{}", s.base, id), json!(s));
    }
    Ok(docs)
}
fn ingest(l: &mut Layers, key: &str, object: &str, value: Value) -> Result<(), String> {
    let suffix = object
        .strip_prefix(&format!("m/{key}/"))
        .ok_or("Invalid meeting namespace.")?;
    let parts: Vec<_> = suffix.split('/').collect();
    macro_rules! insert {
        ($map:ident,$id:expr) => {{
            let incoming =
                serde_json::from_value(value).map_err(|_| "Unsupported meeting layer schema.")?;
            if l.$map.get($id).is_some_and(|old| old != &incoming) {
                return Err("Immutable meeting layer changed.".into());
            }
            l.$map.insert($id.into(), incoming);
        }};
    }
    match parts.as_slice() {
        ["raw", id] => insert!(raw, *id),
        ["clean", id] => insert!(clean, *id),
        ["ops", base, id] => {
            insert!(batches, *id);
            if l.batches[*id].base != *base {
                return Err("Meeting batch base differs.".into());
            }
        }
        ["snap", base, id] => {
            insert!(snapshots, *id);
            if l.snapshots[*id].base != *base {
                return Err("Meeting snapshot base differs.".into());
            }
        }
        ["notes", id] => {
            if value["schema"] != SCHEMA {
                return Err("Unsupported notes schema.".into());
            }
            l.notes.insert((*id).into(), value);
        }
        ["head"] => {
            l.head = serde_json::from_value(value).map_err(|_| "Unsupported meeting head.")?
        }
        _ => return Err("Update SonoJot before syncing these meeting layers.".into()),
    }
    Ok(())
}
fn dirty(l: &Layers, t: &Track) -> Result<bool, String> {
    Ok(t.head.as_ref() != Some(&l.head)
        || t.notes != l.notes
        || documents(l, &t.key)?
            .keys()
            .any(|id| !t.revisions.contains_key(id))
        || l.head.audio.as_ref().is_some_and(|a| {
            a.stems
                .iter()
                .flat_map(|s| &s.chunks)
                .any(|chunk| !t.blobs.contains(&format!("blob/{}/{}", t.key, chunk.id)))
        }))
}
fn unshared_audio(meetings: &Meetings, id: &str, track: &Track) -> Result<bool, String> {
    let paths = meetings.recording_paths(id)?;
    let microphone = paths.microphone.is_file();
    let system = paths.system.is_file();
    let mixed = paths.mixed.is_file()
        || meetings
            .store()
            .meetings_dir()
            .join(format!("{id}.import.wav"))
            .is_file();
    if !microphone && !system && !mixed {
        return Ok(false);
    }
    let Some(audio) = track.head.as_ref().and_then(|head| head.audio.as_ref()) else {
        return Ok(true);
    };
    let complete = |source: &str| {
        audio.stems.iter().any(|stem| {
            stem.source == source
                && stem.chunks.iter().all(|chunk| {
                    track
                        .blobs
                        .contains(&format!("blob/{}/{}", track.key, chunk.id))
                })
        })
    };
    Ok((microphone && !complete("microphone"))
        || (system && !complete("system"))
        || (mixed && !microphone && !system && !complete("mixed")))
}
async fn pull(
    meetings: &Meetings,
    c: &Credentials,
    ring: &Keyring,
    m: &mut State,
    key: &str,
) -> Result<(), String> {
    let inv = inventory(c, key).await?;
    let known_id = m
        .meetings
        .iter()
        .find(|(_, t)| t.key == key)
        .map(|(id, _)| id.clone());
    if let Some(id) = &known_id {
        if m.meetings[id].status == "localOnly" || m.missing.contains(id) {
            return Ok(());
        }
        if deletions(meetings.store())?.contains(id) {
            return Ok(());
        }
        if meetings
            .sync_optional(id)?
            .is_some_and(|local| local.local_only)
        {
            m.meetings.get_mut(id).unwrap().status = "localOnly".into();
            return Ok(());
        }
    }
    let pending_deletions = deletions(meetings.store())?;
    if known_id
        .as_ref()
        .is_some_and(|id| pending_deletions.contains(id))
    {
        return Ok(());
    }
    let head_id = format!("m/{key}/head");
    let head_object = inv
        .objects
        .iter()
        .find(|o| o.id == head_id && o.deleted == 0);
    let (head_value, head_parts) = match head_object {
        Some(object) => match get_parts(c, ring, object).await {
            Ok((value, parts)) => (Ok(value), parts),
            Err(e) => (Err(e), vec![]),
        },
        None => (Err("Meeting head is unavailable.".into()), vec![]),
    };
    if inv.state != "active" || head_value.as_ref().is_ok_and(|v| v["deleted"] == true) {
        if let Some(id) = known_id {
            let track = m.meetings.get_mut(&id).unwrap();
            if ["localOnly", "deleted"].contains(&track.status.as_str()) {
                return Ok(());
            }
            let verified = head_object.is_some_and(|object| {
                head_value.as_ref().is_ok_and(|value| {
                    tombstone(
                        value,
                        &id,
                        object.rev,
                        track.revisions.get(&head_id).copied().unwrap_or(0),
                    )
                })
            }) && identity_matches(c, ring, key, &id)?;
            if verified
                && !unshared_audio(meetings, &id, track)?
                && !meetings
                    .sync_optional(&id)?
                    .is_some_and(|local| dirty(&local.layers, track).unwrap_or(true))
            {
                meetings.sync_delete_remote(&id)?;
                track.revisions.insert(head_id, head_object.unwrap().rev);
                track.status = "deleted".into();
            } else {
                // Server metadata alone must never reach the local deletion sink.
                track.delete_conflict = true;
                track.status = "deletedElsewhere".into();
            }
        }
        return Ok(());
    }
    let Some(head_object) = head_object else {
        return Ok(());
    };
    let head: Head = serde_json::from_value(head_value?).map_err(|_| "Invalid meeting head.")?;
    if !identity_matches(c, ring, key, &head.id)? {
        return Err("Meeting identity does not match its encrypted namespace.".into());
    }
    crate::store::validate_id(&head.id)?;
    if known_id.as_ref().is_some_and(|id| id != &head.id) {
        return Err("Meeting identity changed.".into());
    }
    let id = head.id.clone();
    if let Some(existing) = m.meetings.get(&id) {
        if existing.key != key {
            return Err("Two synced meetings claim the same local id.".into());
        }
    }
    let track = m.meetings.entry(id.clone()).or_insert_with(|| Track {
        key: key.into(),
        ..Default::default()
    });
    if track.status == "deleted" || track.status == "localOnly" {
        return Ok(());
    }
    if head_object.rev < track.revisions.get(&head_id).copied().unwrap_or(0) {
        return Err("The service returned an older meeting head.".into());
    }
    track.parts.insert(head_id.clone(), head_parts);
    let existing = meetings.sync_optional(&id)?;
    let before = existing
        .as_ref()
        .map(|l| l.layers.clone())
        .unwrap_or_else(|| Layers::empty(&id));
    let mut remote = before.clone();
    remote.head = head;
    remote.notes = track.notes.clone();
    let cached = documents(&before, key)?;
    let mut revisions = track.revisions.clone();
    for object in &inv.objects {
        if object.deleted != 0 {
            if object.deleted == 3 {
                revisions.insert(object.id.clone(), object.rev);
            }
            continue;
        }
        if object.id.starts_with("blob/") || object.id.contains("/part/") {
            continue;
        }
        if object.id == head_id {
            revisions.insert(object.id.clone(), object.rev);
            continue;
        }
        let old = track.revisions.get(&object.id).copied().unwrap_or(0);
        if object.rev < old {
            return Err("The service returned an older meeting layer.".into());
        }
        if object.rev == old && cached.contains_key(&object.id) {
            continue;
        }
        if object.rev == old && object.id.contains("/notes/") {
            continue;
        }
        let (value, parts) = get_parts(c, ring, object).await?;
        track.parts.insert(object.id.clone(), parts);
        ingest(&mut remote, key, &object.id, value)?;
        revisions.insert(object.id.clone(), object.rev);
    }
    // A new device need not reconstruct retired, unreferenced views. A device
    // which already has them keeps its local recognition history unchanged.
    remote.clean.retain(|_, v| remote.raw.contains_key(&v.base));
    remote
        .batches
        .retain(|_, v| remote.raw.contains_key(&v.base));
    remote
        .snapshots
        .retain(|_, v| remote.raw.contains_key(&v.base));
    remote.validate()?;
    let authenticated_head = remote.head.clone();
    let authenticated_notes = remote.notes.clone();
    let mut base = before.clone();
    if let Some(head) = &track.head {
        base.head = head.clone();
        base.notes = track.notes.clone();
    } else if existing.is_none() {
        base = remote.clone();
    }
    // sync_commit re-reads under the meeting owner lock, preserving edits made during HTTP.
    meetings.sync_commit(&id, &remote, &base, &mut track.conflicts)?;
    track.head = Some(authenticated_head);
    track.notes = authenticated_notes;
    track.revisions = revisions;
    track.blobs = inv
        .objects
        .iter()
        .filter(|o| o.deleted == 0 && o.id.starts_with("blob/"))
        .map(|o| o.id.clone())
        .collect();
    track.status = "synced".into();
    track.server_version = Some(inv.version);
    if audio::preferences(meetings.store())?.always_download {
        if let Some(manifest) = &remote.head.audio {
            audio::download(meetings.store(), c, ring, key, &id, manifest).await?;
        }
    }
    Ok(())
}
async fn push(
    meetings: &Meetings,
    c: &Credentials,
    ring: &Keyring,
    m: &mut State,
    id: &str,
) -> Result<(), String> {
    let store = meetings.store();
    let mut current = meetings.sync_local(id)?;
    if current.local_only {
        return Ok(());
    }
    if !m.meetings.contains_key(id) {
        m.meetings.insert(
            id.into(),
            Track {
                key: opaque(c, id)?,
                status: "pending".into(),
                ..Default::default()
            },
        );
        save(store, m)?;
    }
    let track = m.meetings.get(id).unwrap();
    if ["storageFull", "deleted", "deletedElsewhere", "localOnly"].contains(&track.status.as_str())
    {
        return Ok(());
    }
    let key = track.key.clone();
    let enabled = current
        .audio_override
        .unwrap_or(audio::preferences(store)?.audio_default);
    if !enabled
        && track.head.as_ref().is_none_or(|head| head.audio.is_none())
        && current.layers.head.audio.is_some()
    {
        let _guard = store.lock()?;
        let mut latest = local::load(store, id)?.ok_or("Meeting was deleted.")?;
        latest.layers.head.audio = None;
        local::save(store, &latest)?;
        current = latest;
    }
    audio::cleanup_cache(store, id, current.layers.head.audio.as_ref(), !enabled)?;
    let needs_audio = enabled && current.layers.head.audio.is_none() && meetings.get(id)?.has_audio;
    if !dirty(&current.layers, track)? && !needs_audio {
        return hold(meetings, c, m, id).await;
    }
    m.meetings.get_mut(id).unwrap().server_version = None;
    let created = current
        .layers
        .head
        .fields
        .get("createdAt")
        .and_then(Value::as_str)
        .and_then(|v| {
            time::OffsetDateTime::parse(v, &time::format_description::well_known::Rfc3339).ok()
        })
        .map(|v| (v.unix_timestamp().max(0) / 86400) * 86400)
        .unwrap_or(0);
    let _: Value = c
        .client()?
        .json(
            Method::POST,
            &format!("/v1/meetings/{key}"),
            Some(&json!({"createdAt":created})),
        )
        .await?;
    if enabled && current.layers.head.audio.is_none() && meetings.get(id)?.has_audio {
        let owned = meetings.clone();
        let owned_id = id.to_owned();
        let manifest = tokio::task::spawn_blocking(move || audio::prepare(&owned, &owned_id))
            .await
            .map_err(|_| "Audio encoding stopped.")??;
        let _guard = store.lock()?;
        let mut latest = local::load(store, id)?.ok_or("Meeting was deleted during encoding.")?;
        latest.layers.head.audio = Some(manifest);
        local::save(store, &latest)?;
        current = latest;
    }
    // Turning uploads off does not erase an already synced recording. Whole-meeting
    // deletion and storage eviction own server erasure and its retained-copy rules.
    current.layers.compact("original")?;
    if let Some(view) = current.layers.head.clean.clone() {
        current.layers.compact(&view)?;
    }
    {
        let _guard = store.lock()?;
        let mut latest = local::load(store, id)?.ok_or("Meeting was deleted.")?;
        latest.layers.merge_immutable(&current.layers)?;
        // Only publish the snapshot references if the underlying head did not change.
        if latest.layers.head.raw == current.layers.head.raw {
            latest.layers.head.snapshots = current.layers.head.snapshots.clone();
        }
        local::save(store, &latest)?;
        current = latest;
    }
    if let Some(mut manifest) = current.layers.head.audio.clone() {
        let inv = inventory(c, &key).await?;
        let expired = inv
            .objects
            .iter()
            .filter(|o| o.deleted == 4 && o.id.starts_with("blob/"))
            .filter_map(|o| o.id.rsplit('/').next().map(str::to_string))
            .collect();
        m.meetings.get_mut(id).unwrap().blobs.retain(|object| {
            inv.objects
                .iter()
                .any(|o| &o.id == object && o.deleted == 0)
        });
        if audio::renew_expired(store, id, &mut manifest, &expired)? {
            let _guard = store.lock()?;
            let mut latest = local::load(store, id)?.ok_or("Meeting was deleted.")?;
            if latest.layers.head.audio != current.layers.head.audio {
                return Err("Audio changed during upload. Retry sync.".into());
            }
            latest.layers.head.audio = Some(manifest.clone());
            local::save(store, &latest)?;
            current = latest;
        }
        audio::upload(
            store,
            c,
            ring,
            &key,
            id,
            &manifest,
            &mut m.meetings.get_mut(id).unwrap().blobs,
        )
        .await?;
        save(store, m)?;
    }
    for (object, value) in documents(&current.layers, &key)? {
        if m.meetings[id].revisions.contains_key(&object) {
            continue;
        }
        match put(c, ring, &object, 0, &value).await {
            Ok(Published { rev, parts }) => {
                m.meetings
                    .get_mut(id)
                    .unwrap()
                    .parts
                    .insert(object.clone(), parts);
                m.meetings
                    .get_mut(id)
                    .unwrap()
                    .revisions
                    .insert(object, rev);
                save(store, m)?;
            }
            Err(e) if e.status == 412 => {
                let (remote, parts) = get_parts(
                    c,
                    ring,
                    &Object {
                        id: object.clone(),
                        rev: 1,
                        deleted: 0,
                    },
                )
                .await?;
                m.meetings
                    .get_mut(id)
                    .unwrap()
                    .parts
                    .insert(object.clone(), parts);
                if value != remote {
                    return Err("An immutable meeting layer changed.".into());
                }
                m.meetings.get_mut(id).unwrap().revisions.insert(object, 1);
                save(store, m)?;
            }
            Err(e) => return Err(e.into()),
        }
    }
    for (note, value) in &current.layers.notes {
        if m.meetings[id].notes.get(note) == Some(value) {
            continue;
        }
        let object = format!("m/{key}/notes/{note}");
        let rev = m.meetings[id].revisions.get(&object).copied().unwrap_or(0);
        match put(c, ring, &object, rev, value).await {
            Ok(Published { rev, parts }) => {
                m.meetings
                    .get_mut(id)
                    .unwrap()
                    .parts
                    .insert(object.clone(), parts);
                let t = m.meetings.get_mut(id).unwrap();
                t.revisions.insert(object, rev);
                t.notes.insert(note.clone(), value.clone());
                save(store, m)?;
            }
            Err(e) if e.status == 412 => {
                pull(meetings, c, ring, m, &key).await?;
                save(store, m)?;
                return Err("Meeting notes changed on another device. Sync again to publish the merged copy.".into());
            }
            Err(e) => return Err(e.into()),
        }
    }
    let object = format!("m/{key}/head");
    let rev = m.meetings[id].revisions.get(&object).copied().unwrap_or(0);
    match put(c, ring, &object, rev, &json!(&current.layers.head)).await {
        Ok(Published { rev, parts }) => {
            m.meetings
                .get_mut(id)
                .unwrap()
                .parts
                .insert(object.clone(), parts);
            let t = m.meetings.get_mut(id).unwrap();
            t.revisions.insert(object, rev);
            t.head = Some(current.layers.head.clone());
            t.status = "synced".into();
            save(store, m)?;
        }
        Err(e) if e.status == 412 => {
            pull(meetings, c, ring, m, &key).await?;
            save(store, m)?;
            return Err(
                "Meeting changed on another device. Sync again to publish the merged copy.".into(),
            );
        }
        Err(e) => return Err(e.into()),
    }
    for snapshot_id in current.layers.head.snapshots.values() {
        let snapshot = &current.layers.snapshots[snapshot_id];
        let objects: Vec<_> = snapshot
            .folded
            .iter()
            .map(|id| format!("m/{key}/ops/{}/{id}", snapshot.base))
            .collect();
        for folded in objects.chunks(250) {
            let _:Value=c.client()?.json(Method::POST,&format!("/v1/meetings/{key}/compact"),Some(&json!({"snapshot":format!("m/{key}/snap/{}/{snapshot_id}",snapshot.base),"folded":folded}))).await?;
        }
    }
    hold(meetings, c, m, id).await
}
async fn hold(meetings: &Meetings, c: &Credentials, m: &mut State, id: &str) -> Result<(), String> {
    let track = &m.meetings[id];
    if track.status != "synced" {
        return Ok(());
    }
    let local = meetings.sync_local(id)?;
    if dirty(&local.layers, track)? {
        return Ok(());
    }
    let head_revision = track
        .revisions
        .get(&format!("m/{}/head", track.key))
        .copied()
        .ok_or("Meeting head has not synced.")?;
    let complete_audio =
        local.layers.head.audio.as_ref().is_none_or(|a| {
            audio::directory(meetings.store(), id).is_ok_and(|dir| a.complete(&dir))
        });
    if track
        .references
        .as_ref()
        .is_some_and(|r| Some(r.version) == track.server_version && r.head == head_revision)
        && (!complete_audio
            || (track.held_at > 0 && now().saturating_sub(track.held_at) < 7 * 86400))
    {
        return Ok(());
    }
    let inv = inventory(c, &track.key).await?;
    if inv.state != "active" {
        return Ok(());
    }
    let live: BTreeSet<_> = inv
        .objects
        .iter()
        .filter(|o| o.deleted == 0)
        .map(|o| o.id.clone())
        .collect();
    let retained = reachable(&local.layers, track);
    let retained: Vec<_> = retained.intersection(&live).cloned().collect();
    let token = uuid::Uuid::new_v4().to_string();
    let head = format!("m/{}/head", track.key);
    let head_revision = track
        .revisions
        .get(&head)
        .copied()
        .ok_or("Meeting head has not synced.")?;
    let receipt = Receipt {
        version: inv.version,
        head: head_revision,
    };
    if track.references.as_ref() != Some(&receipt) {
        for (index, page) in retained.chunks(250).enumerate() {
            let _: Value = c.client()?.json(Method::POST, &format!("/v1/meetings/{}/references", track.key),
                Some(&json!({"token":token,"headRevision":head_revision,"version":inv.version,"objects":page,"complete":(index+1)*250>=retained.len()}))).await?;
        }
        let track = m.meetings.get_mut(id).unwrap();
        track.references = Some(receipt);
        track.server_version = Some(inv.version);
        track.held_at = 0;
        save(meetings.store(), m)?;
    }
    let track = &m.meetings[id];
    if let Some(audio) = &local.layers.head.audio {
        if !audio.complete(&audio::directory(meetings.store(), id)?) {
            return Ok(());
        }
    }
    let known = documents(&local.layers, &track.key)?;
    for o in &inv.objects {
        if o.deleted != 0 {
            continue;
        }
        // Unpublished uploads do not belong to the held head and expire separately.
        if (o.id.starts_with("blob/") || o.id.contains("/part/")) && !retained.contains(&o.id) {
            continue;
        }
        if o.id.contains("/part/") {
            continue;
        } // Authenticated while reading/writing its parent.
        if o.id.starts_with("blob/") {
            let chunk = o.id.rsplit('/').next().unwrap_or("");
            if !local.layers.head.audio.as_ref().is_some_and(|a| {
                a.stems
                    .iter()
                    .any(|s| s.chunks.iter().any(|c| c.id == chunk))
            }) {
                return Ok(());
            }
        } else if !o.id.ends_with("/head")
            && !o.id.contains("/notes/")
            && !known.contains_key(&o.id)
        {
            return Ok(());
        }
        if !o.id.starts_with("blob/") && track.revisions.get(&o.id) != Some(&o.rev) {
            return Ok(());
        }
    }
    let object = format!("m/{}/head", track.key);
    let Some(rev) = track.revisions.get(&object) else {
        return Ok(());
    };
    let _: Value = c
        .client()?
        .json(
            Method::POST,
            "/v1/holds",
            Some(&json!({"object":object,"rev":rev,"version":inv.version,"complete":true})),
        )
        .await?;
    m.meetings.get_mut(id).unwrap().held_at = now();
    Ok(())
}
fn reachable(l: &Layers, t: &Track) -> BTreeSet<String> {
    let key = &t.key;
    let mut refs = BTreeSet::from([
        format!("m/{key}/head"),
        format!("m/{key}/raw/{}", l.head.raw),
    ]);
    if let Some(first) = l
        .head
        .fields
        .get("record.rawTranscriptRef")
        .and_then(Value::as_str)
    {
        refs.insert(format!("m/{key}/raw/{first}"));
    }
    if let Some(clean) = &l.head.clean {
        refs.insert(format!("m/{key}/clean/{clean}"));
    }
    for note in &l.head.notes {
        refs.insert(format!("m/{key}/notes/{note}"));
    }
    let mut folded = BTreeSet::new();
    for (id, snapshot) in &l.snapshots {
        if snapshot.base == l.head.raw {
            refs.insert(format!("m/{key}/snap/{}/{id}", snapshot.base));
            folded.extend(snapshot.folded.iter().cloned());
        }
    }
    for (id, batch) in &l.batches {
        if batch.base == l.head.raw && !folded.contains(id) {
            refs.insert(format!("m/{key}/ops/{}/{id}", batch.base));
        }
    }
    if let Some(audio) = &l.head.audio {
        refs.extend(
            audio
                .stems
                .iter()
                .flat_map(|s| &s.chunks)
                .map(|c| format!("blob/{key}/{}", c.id)),
        );
    }
    for root in refs.clone() {
        if let Some(parts) = t.parts.get(&root) {
            refs.extend(parts.iter().cloned());
        }
    }
    refs
}
pub async fn synchronize(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    ring_rev: u64,
) -> Result<State, String> {
    match synchronize_inner(store, c, ring, ring_rev).await {
        Err(error) if crate::sync_client::rate_limit_delay(&error).is_some() => {
            let mut state = state(store, c)?;
            rate_limited(store, &mut state, &error)?;
            Ok(state)
        }
        result => result,
    }
}
async fn synchronize_inner(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    ring_rev: u64,
) -> Result<State, String> {
    #[cfg(not(test))]
    crate::flavor::require_sync()?;
    let meetings = Meetings::new(store.clone());
    let mut ids = meetings.sync_collect()?;
    ids.sort();
    let mut m = state(store, c)?;
    m.rate_limited = false;
    m.errors.remove("pass");
    if ring.epoch < m.epoch || ring_rev < m.ring_rev {
        return Err("The service returned older meeting keys.".into());
    }
    let mut touched = BTreeSet::new();
    let mut cursor = m.cursor;
    let mut full = m.cursor == 0;
    for _ in 0..1000 {
        let page: Value = c
            .client()?
            .json(Method::GET, &format!("/v1/changes?since={cursor}"), None)
            .await?;
        if page["epoch"].as_u64() != Some(ring.epoch) {
            return Err("Meeting encryption keys changed. Unlock again.".into());
        }
        let next = page["cursor"]
            .as_u64()
            .ok_or("Invalid meeting change cursor.")?;
        if next < cursor {
            return Err("The service returned older meeting changes.".into());
        }
        full |= page["fullResync"] == true;
        for item in page["changes"]
            .as_array()
            .ok_or("Invalid meeting change feed.")?
        {
            let object = item["id"].as_str().ok_or("Invalid changed object.")?;
            if let Some(key) = member(object) {
                if !valid_key(key) {
                    return Err("Invalid meeting id.".into());
                }
                touched.insert(key.to_string());
            }
        }
        if page["hasMore"] != true {
            cursor = next;
            break;
        }
        if next == cursor {
            return Err("Meeting change cursor did not advance.".into());
        }
        cursor = next;
    }
    // Index pages also contain tombstones after a stale change cursor.
    let mut after = String::new();
    let mut listed = BTreeSet::new();
    for _ in 0..1000 {
        let page: Value = c
            .client()?
            .json(Method::GET, &format!("/v1/meetings?after={after}"), None)
            .await?;
        m.used = page["storage"]["used"]
            .as_u64()
            .ok_or("Invalid meeting storage usage.")?;
        m.quota = page["storage"]["quota"]
            .as_u64()
            .ok_or("Invalid meeting storage quota.")?;
        for row in page["meetings"]
            .as_array()
            .ok_or("Invalid meeting index.")?
        {
            let key = row["id"]
                .as_str()
                .filter(|v| valid_key(v))
                .ok_or("Invalid meeting index id.")?;
            listed.insert(key.to_owned());
            let known = m.meetings.values().find(|t| t.key == key);
            if full
                || row["state"] != "active"
                || known.is_none_or(|t| t.server_version != row["version"].as_u64())
            {
                touched.insert(key.to_owned());
            } else {
                touched.remove(key);
            }
            after = key.into();
        }
        if page["hasMore"] != true {
            break;
        }
    }
    if full {
        for t in m.meetings.values_mut() {
            if t.head.is_some() && !listed.contains(&t.key) {
                t.status = "deletedElsewhere".into();
                t.delete_conflict = true;
            }
        }
    }
    m.pending_pulls.extend(touched);
    m.cursor = cursor;
    m.epoch = ring.epoch;
    m.ring_rev = ring_rev;
    save(store, &m)?;
    for key in m.pending_pulls.clone() {
        if !meeting_ready(&m, &key) {
            continue;
        }
        match pull(&meetings, c, ring, &mut m, &key).await {
            Ok(()) => {
                m.pending_pulls.remove(&key);
                m.errors.remove(&key);
                m.retries.remove(&key);
            }
            Err(e) => {
                if rate_limited(store, &mut m, &e)? {
                    return Ok(m);
                }
                meeting_failed(&mut m, &key, e);
            }
        }
        save(store, &m)?;
    }
    let deletions = deletions(store)?;
    for id in deletions {
        if !meeting_ready(&m, &id) {
            continue;
        }
        let result: Result<(), String> = async {
            if let Some(t) = m.meetings.get_mut(&id) {
                if !["deleted", "localOnly"].contains(&t.status.as_str()) {
                    let object = format!("m/{}/head", t.key);
                    let inv = inventory(c, &t.key).await?;
                    let current = inv
                        .objects
                        .iter()
                        .find(|o| o.id == object && o.deleted == 0);
                    let mut rev = t.revisions.get(&object).copied().unwrap_or(0);
                    if let Some(current) = current {
                        let value = get(c, ring, current).await?;
                        if value["id"] != id
                            || !identity_matches(c, ring, &t.key, &id)?
                            || current.rev < rev
                        {
                            return Err("Cannot authenticate the meeting being deleted.".into());
                        }
                        rev = current.rev;
                        if value["deleted"] != true {
                            if !meetings.sync_delete_ready(&id)? {
                                return Ok(());
                            }
                            rev = put(
                                c,
                                ring,
                                &object,
                                rev,
                                &json!({"schema":SCHEMA,"deleted":true,"id":id}),
                            )
                            .await?
                            .rev;
                        }
                    } else {
                        if !identity_matches(c, ring, &t.key, &id)? {
                            return Err("Cannot authenticate the meeting being deleted.".into());
                        }
                        // Eviction zeroes the head but retains its CAS revision. A
                        // local user deletion can publish a tombstone at the next one.
                        if let Some(retired) = inv.objects.iter().find(|o| o.id == object) {
                            if retired.rev < rev {
                                return Err(
                                    "The service returned an older meeting revision.".into()
                                );
                            }
                            rev = retired.rev;
                        }
                        if !meetings.sync_delete_ready(&id)? {
                            return Ok(());
                        }
                        rev = put(
                            c,
                            ring,
                            &object,
                            rev,
                            &json!({"schema":SCHEMA,"deleted":true,"id":id}),
                        )
                        .await?
                        .rev;
                    }
                    if !meetings.sync_delete_published(&id)? {
                        t.status = "localOnly".into();
                        save(store, &m)?;
                        return Ok(());
                    }
                    let _: Value = c
                        .client()?
                        .json(
                            Method::DELETE,
                            &format!("/v1/meetings/{}", t.key),
                            Some(&json!({"confirmed":true,"headRevision":rev})),
                        )
                        .await?;
                    t.revisions.insert(object, rev);
                }
                t.status = "deleted".into();
                t.delete_conflict = false;
            }
            m.errors.remove(&id);
            m.retries.remove(&id);
            save(store, &m)?;
            let _guard = store.lock()?;
            cancel_delete(store, &id)?;
            Ok(())
        }
        .await;
        if let Err(e) = result {
            if rate_limited(store, &mut m, &e)? {
                return Ok(m);
            }
            meeting_failed(&mut m, &id, e);
            save(store, &m)?;
        }
    }
    // Resume at the interrupted meeting. Successful earlier work must not consume
    // the next window before a large library reaches its remaining meetings.
    let start = ids.partition_point(|id| id <= &m.push_after);
    ids.rotate_left(start);
    for id in ids {
        if !meeting_ready(&m, &id) {
            continue;
        }
        if meetings.get(&id).is_ok() {
            let key = m.meetings.get(&id).map(|t| t.key.clone());
            if key
                .as_ref()
                .is_some_and(|key| m.pending_pulls.contains(key))
            {
                continue;
            }
            match push(&meetings, c, ring, &mut m, &id).await {
                Ok(()) => {
                    m.errors.remove(&id);
                    m.retries.remove(&id);
                }
                Err(e) => {
                    if rate_limited(store, &mut m, &e)? {
                        return Ok(m);
                    }
                    meeting_failed(&mut m, &id, e);
                    // CAS/busy/corrupt meetings retry independently next pass.
                    if let Some(key) = key {
                        m.pending_pulls.insert(key);
                    }
                }
            }
            m.push_after = id;
            // Upload receipts save themselves. Persist traversal at a pause/end,
            // without rewriting the entire account state for every unchanged row.
        }
    }
    m.push_after.clear();
    // Pull evictions caused by this pass before returning storage status.
    let summary: Value = c.client()?.json(Method::GET, "/v1/meetings", None).await?;
    m.used = summary["storage"]["used"].as_u64().unwrap_or(m.used);
    for row in summary["meetings"].as_array().into_iter().flatten() {
        if row["state"] == "dropped" {
            if let Some(t) = m.meetings.values_mut().find(|t| row["id"] == t.key) {
                if !t.delete_conflict {
                    t.status = "storageFull".into();
                }
            }
        }
    }
    save(store, &m)?;
    Ok(m)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn rate_limited(store: &Store, m: &mut State, error: &str) -> Result<bool, String> {
    let Some(delay) = crate::sync_client::rate_limit_delay(error) else {
        return Ok(false);
    };
    m.rate_limited = true;
    m.retry_at = now() + delay;
    save(store, m)?;
    Ok(true)
}
fn meeting_ready(m: &State, key: &str) -> bool {
    m.retries.get(key).is_none_or(|retry| retry.at <= now())
}
fn meeting_failed(m: &mut State, key: &str, error: String) {
    let retry = m.retries.entry(key.into()).or_default();
    retry.delay = (retry.delay.max(30) * 2).min(3600);
    retry.at = now() + retry.delay;
    m.errors.insert(key.into(), error);
}
fn complete_pass(previous: &State, mut next: State, failed: bool) -> State {
    if next.rate_limited {
        return next;
    }
    if failed {
        next.retry_delay = (previous.retry_delay.max(30) * 2).min(3600);
        next.retry_at = now() + next.retry_delay;
    } else {
        next.retry_delay = 0;
        next.retry_at = 0;
    }
    next
}
pub fn outcome(m: &State) -> Value {
    json!({"errors":m.errors.len(), "retryAt":m.retry_at,
        "rateLimited":m.rate_limited, "missing":m.missing.len()})
}
/// Only a whole-pass failure delays the library. Meeting errors keep independent
/// retries; manual sync bypasses them, but never a server rate-limit window.
pub async fn scheduled(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    rev: u64,
) -> Result<State, String> {
    run(store, c, ring, rev, false).await
}
pub async fn run(
    store: &Store,
    c: &Credentials,
    ring: &Keyring,
    rev: u64,
    manual: bool,
) -> Result<State, String> {
    let mut previous = state(store, c)?;
    if manual && !(previous.rate_limited && previous.retry_at > now()) {
        previous.retry_at = 0;
        previous.retries.clear();
        save(store, &previous)?;
    }
    if previous.retry_at > now() {
        return Ok(previous);
    }
    let (next, failed) = match synchronize(store, c, ring, rev).await {
        Ok(next) => (next, false),
        Err(error) => {
            let mut next = state(store, c)?;
            next.rate_limited = false;
            next.errors.insert("pass".into(), error);
            (next, true)
        }
    };
    let next = complete_pass(&previous, next, failed);
    save(store, &next)?;
    Ok(next)
}
pub(crate) fn deletions_unlocked(store: &Store) -> Result<BTreeSet<String>, String> {
    let path = store.root().join("meeting-sync-deletions.json");
    if path.exists() {
        store.read(&path)
    } else {
        Ok(BTreeSet::new())
    }
}
pub(crate) fn tracked_ids(store: &Store) -> Result<BTreeSet<String>, String> {
    let path = store.root().join(STATE);
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let state: State = store.read(&path)?;
    Ok(state
        .meetings
        .into_iter()
        .filter(|(_, t)| t.head.is_some() && !["deleted", "localOnly"].contains(&t.status.as_str()))
        .map(|(id, _)| id)
        .collect())
}
pub(crate) fn keep_restored(store: &Store, id: &str) -> Result<(), String> {
    let path = store.root().join(STATE);
    let mut state: State = if path.exists() {
        store.read(&path)?
    } else {
        State::default()
    };
    let track = state.meetings.entry(id.into()).or_default();
    track.status = "localOnly".into();
    track.delete_conflict = false;
    save(store, &state)?;
    cancel_delete(store, id)
}
fn deletions(store: &Store) -> Result<BTreeSet<String>, String> {
    let _guard = store.lock()?;
    deletions_unlocked(store)
}
fn approvals(store: &Store) -> Result<BTreeSet<String>, String> {
    let path = store.root().join("meeting-sync-delete-approvals.json");
    if local::absent(&path)? {
        Ok(BTreeSet::new())
    } else {
        store.read(&path)
    }
}
pub(crate) fn cancel_delete(store: &Store, id: &str) -> Result<(), String> {
    let mut pending = deletions_unlocked(store)?;
    pending.remove(id);
    store.write(&store.root().join("meeting-sync-deletions.json"), &pending)?;
    let mut confirmed = approvals(store)?;
    confirmed.remove(id);
    store.write(
        &store.root().join("meeting-sync-delete-approvals.json"),
        &confirmed,
    )
}
pub(crate) fn queue_delete(store: &Store, id: &str) -> Result<(), String> {
    // Explicit UI deletions are distinct from disappearance inferred on disk.
    let mut confirmed = approvals(store)?;
    confirmed.insert(id.into());
    store.write(
        &store.root().join("meeting-sync-delete-approvals.json"),
        &confirmed,
    )?;
    let mut pending = deletions_unlocked(store)?;
    pending.insert(id.into());
    store.write(&store.root().join("meeting-sync-deletions.json"), &pending)
}
fn disk_state(store: &Store) -> Result<State, String> {
    let path = store.root().join(STATE);
    if local::absent(&path)? {
        Ok(State::default())
    } else {
        store.read(&path)
    }
}
pub(crate) fn missing_notice(store: &Store, ids: &BTreeSet<String>) -> Result<(), String> {
    let mut m = disk_state(store)?;
    m.missing = ids.clone();
    let confirmed = approvals(store)?;
    let mut pending = deletions_unlocked(store)?;
    pending.retain(|id| confirmed.contains(id));
    store.write(&store.root().join("meeting-sync-deletions.json"), &pending)?;
    save(store, &m)
}
pub(crate) fn reconcile_missing(
    store: &Store,
    tracked: &BTreeSet<String>,
    missing: &BTreeSet<String>,
) -> Result<(), String> {
    let confirmed = approvals(store)?;
    let inferred: BTreeSet<_> = missing.difference(&confirmed).cloned().collect();
    if missing.len() > 3 || missing.len() * 20 > tracked.len() {
        return missing_notice(store, &inferred);
    }
    let mut m = disk_state(store)?;
    let old_notice = m.missing.clone();
    m.missing.retain(|id| missing.contains(id));
    if m.missing != old_notice {
        save(store, &m)?;
    }
    let mut pending = deletions_unlocked(store)?;
    // A displayed notice continues to require a choice even after a restart.
    pending.extend(inferred.difference(&m.missing).cloned());
    if pending.is_empty() && local::absent(&store.root().join("meeting-sync-deletions.json"))? {
        return Ok(());
    }
    store.write(&store.root().join("meeting-sync-deletions.json"), &pending)
}
fn resolve_missing(store: &Store, c: &Credentials, delete: bool) -> Result<(), String> {
    let _guard = store.lock()?;
    let mut m = state(store, c)?;
    if delete {
        local::readable_meetings(store)?;
    }
    for id in &m.missing {
        if delete && local::absent(&store.meetings_dir().join(format!("{id}.json")))? {
            queue_delete(store, id)?;
        } else {
            cancel_delete(store, id)?;
            if let Some(track) = m.meetings.get_mut(id) {
                track.status = "localOnly".into();
                track.delete_conflict = false;
            }
            if let Ok(Some(mut layer)) = local::load(store, id) {
                layer.local_only = true;
                local::save(store, &layer)?;
            }
        }
    }
    m.missing.clear();
    save(store, &m)
}

pub(crate) async fn ensure_audio(
    app: &tauri::AppHandle,
    meetings: &Meetings,
    id: &str,
) -> Result<(), String> {
    if !crate::flavor::sync_enabled() || meetings.get(id)?.has_audio {
        return Ok(());
    }
    if meetings
        .sync_optional(id)?
        .is_some_and(|local| local.layers.head.audio.is_some())
    {
        crate::sync_account::sync_command(
            app.clone(),
            "meeting_download".into(),
            Some(json!({"id":id})),
        )
        .await?;
    }
    Ok(())
}

pub async fn action(
    store: &Store,
    c: &mut Credentials,
    operation: &str,
    args: &Value,
) -> Result<Value, String> {
    #[cfg(not(test))]
    crate::flavor::require_sync()?;
    let meetings = Meetings::new(store.clone());
    if operation == "meeting_missing_resolve" {
        resolve_missing(
            store,
            c,
            args["delete"] == true && args["confirmed"] == true,
        )?;
        return Ok(json!({"resolved":true}));
    }
    if operation == "meeting_preferences" {
        if args.is_object() && args.get("audioDefault").is_some() {
            let p: audio::Preferences = serde_json::from_value(args.clone())
                .map_err(|_| "Invalid audio sync preferences.")?;
            store.write(&store.root().join("meeting-sync-preferences.json"), &p)?;
        }
        return Ok(json!(audio::preferences(store)?));
    }
    if operation == "meeting_storage" {
        let m = state(store, c)?;
        return Ok(json!({"used":m.used,"quota":m.quota,"audio":audio::preferences(store)?}));
    }
    let id = args["id"].as_str().ok_or("Choose a meeting.")?;
    crate::store::validate_id(id)?;
    meetings.sync_capture(id)?;
    let mut local = meetings.sync_local(id)?;
    let mut m = state(store, c)?;
    match operation {
        "meeting_status" => {
            let t = m.meetings.get(id);
            let mut conflicts = local.layers.rendered("original")?.1;
            if let Some(view) = &local.layers.head.clean {
                conflicts.extend(local.layers.rendered(view)?.1);
            }
            let status = if local.local_only {
                "localOnly"
            } else if let Some(t) = t {
                if t.status == "synced"
                    && (dirty(&local.layers, t)?
                        || m.pending_pulls.contains(&t.key)
                        || m.errors.contains_key(id)
                        || m.errors.contains_key(&t.key))
                {
                    "pending"
                } else {
                    t.status.as_str()
                }
            } else {
                "pending"
            };
            Ok(
                json!({"status":status,"audioOverride":local.audio_override,"audioEnabled":local.audio_override.unwrap_or(audio::preferences(store)?.audio_default),"audioAvailable":local.layers.head.audio.is_some(),"conflicts":conflicts,"fieldConflicts":t.map(|t|&t.conflicts),"deleteConflict":t.is_some_and(|t|t.delete_conflict)}),
            )
        }
        "meeting_audio" => {
            let enabled = match args.get("enabled") {
                None | Some(Value::Null) => None,
                Some(Value::Bool(v)) => Some(*v),
                _ => return Err("Audio override must be on, off or default.".into()),
            };
            let _guard = store.lock()?;
            local = local::load(store, id)?.ok_or("Meeting is unavailable.")?;
            local.audio_override = enabled;
            local::save(store, &local)?;
            Ok(json!({"saved":true}))
        }
        "meeting_download" => {
            let manifest = local
                .layers
                .head
                .audio
                .as_ref()
                .ok_or("This meeting has no synced audio.")?;
            let key = &m.meetings.get(id).ok_or("Sync the meeting first.")?.key;
            let (ring, _) = crate::sync_account::keyring(store, c).await?;
            audio::download(store, c, &ring, key, id, manifest).await?;
            let owned = store.clone();
            let id = id.to_string();
            let manifest = manifest.clone();
            tokio::task::spawn_blocking(move || audio::materialize(&owned, &id, &manifest))
                .await
                .map_err(|_| "Audio download stopped.")??;
            Ok(json!({"downloaded":true}))
        }
        "meeting_resolve" => {
            let sid = args["sid"]
                .as_str()
                .ok_or("Choose a conflicting passage.")?;
            let view = args["view"].as_str().unwrap_or("original");
            let version = args["version"]
                .as_str()
                .ok_or("Choose a conflicting version.")?;
            let _guard = store.lock()?;
            let mut local = local::load(store, id)?.ok_or("Meeting is unavailable.")?;
            let conflict = local
                .layers
                .rendered(view)?
                .1
                .into_iter()
                .find(|v| v.sid == sid && v.version == version)
                .ok_or("This conflict was already resolved.")?;
            local.layers.add_ops(
                &c.device_id,
                view,
                vec![layers::Op::Set {
                    sid: sid.into(),
                    text: conflict.text,
                }],
            )?;
            local::save_projected(store, &mut local)?;
            Ok(json!({"resolved":true}))
        }
        "meeting_field_resolve" => {
            let field = args["field"]
                .as_str()
                .ok_or("Choose a conflicting detail.")?;
            let track = m.meetings.get_mut(id).ok_or("Sync this meeting first.")?;
            let value = track
                .conflicts
                .get(field)
                .ok_or("This conflict was already resolved.")?
                .clone();
            if args["useOther"] == true {
                let mut document = json!({"head": local.layers.head, "notes": local.layers.notes});
                let target = document
                    .pointer_mut(&format!("/{field}"))
                    .ok_or("The conflicting detail no longer exists.")?;
                *target = value;
                let mut resolved = local.layers.clone();
                resolved.head = serde_json::from_value(document["head"].clone())
                    .map_err(|_| "Invalid meeting detail.")?;
                resolved.notes = serde_json::from_value(document["notes"].clone())
                    .map_err(|_| "Invalid meeting notes.")?;
                resolved.validate()?;
                meetings.sync_commit(id, &resolved, &local.layers, &mut track.conflicts)?;
            } else if args["useOther"] != false {
                return Err("Choose which detail to keep.".into());
            }
            track.conflicts.remove(field);
            save(store, &m)?;
            Ok(json!({"resolved":true}))
        }
        "meeting_delete_resolve" => {
            let t = m
                .meetings
                .get_mut(id)
                .filter(|t| t.delete_conflict)
                .ok_or("No pending deletion conflict.")?;
            if args["keep"] == true {
                let _guard = store.lock()?;
                local.local_only = true;
                local::save(store, &local)?;
                t.status = "localOnly".into();
            } else if args["keep"] == false {
                meetings.sync_delete_remote(id)?;
                t.status = "deleted".into();
            } else {
                return Err("Choose Keep on this computer or Delete.".into());
            }
            t.delete_conflict = false;
            save(store, &m)?;
            Ok(json!({"resolved":true}))
        }
        _ => Err("Unknown meeting sync action.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn meeting_tombstone_requires_matching_identity_and_a_newer_revision() {
        let value = json!({"schema":SCHEMA,"id":"synthetic","deleted":true});
        assert!(tombstone(&value, "synthetic", 3, 2));
        assert!(!tombstone(&value, "synthetic", 2, 2));
        assert!(!tombstone(&value, "other", 3, 2));
        assert!(!tombstone(&json!({"state":"deleted"}), "synthetic", 3, 2));
        assert!(!tombstone(
            &json!({"schema":SCHEMA,"id":"synthetic","deleted":false}),
            "synthetic",
            3,
            2
        ));
    }
    #[test]
    fn meeting_identity_is_bound_to_current_or_retained_derivation_keys() {
        let mut c = Credentials::new("http://127.0.0.1:1".into());
        c.account = "synthetic-account".into();
        c.ak = Some(crypto::Secret::random());
        let mut ring = Keyring::new();
        let old = opaque(&c, "synthetic").unwrap();
        assert!(identity_matches(&c, &ring, &old, "synthetic").unwrap());
        assert!(!identity_matches(&c, &ring, &old, "different").unwrap());
        ring.meeting_id_keys.push(crypto::derive(
            &c.ak.as_ref().unwrap().0,
            &c.account,
            "meeting-ids",
        ));
        c.ak = Some(crypto::Secret::random());
        assert!(identity_matches(&c, &ring, &old, "synthetic").unwrap());
        ring.meeting_id_keys.clear();
        assert!(!identity_matches(&c, &ring, &old, "synthetic").unwrap());
    }
    #[test]
    fn meeting_incompressible_text_splits_into_bounded_authenticated_parts() {
        use rand::{RngCore, SeedableRng};
        let mut data = vec![0u8; 1_200_000];
        rand::rngs::StdRng::seed_from_u64(7).fill_bytes(&mut data);
        let ring = Keyring::new();
        let (root, parts) = segmented(&ring, "m/synthetic/raw/base", 1, &data).unwrap();
        assert!(root.len() < 512 * 1024);
        let manifest: TextManifest = serde_json::from_slice(&root[4..]).unwrap();
        assert_eq!(manifest.bytes, data.len());
        let joined: Vec<u8> = parts
            .iter()
            .flat_map(|(_, bytes)| bytes.iter().copied())
            .collect();
        assert_eq!(joined, data);
        assert_eq!(manifest.sha256, audio::hash(&joined));
        for (id, bytes) in &parts {
            assert!(bytes.len() <= 128 * 1024);
            let sealed = crypto::seal(
                &ring.keys[&ring.active],
                ring.active,
                "synthetic",
                id,
                1,
                bytes,
            )
            .unwrap();
            assert_eq!(
                &*crypto::open(&ring.keys[&ring.active], "synthetic", id, 1, &sealed).unwrap(),
                bytes
            );
            assert!(crypto::open(
                &ring.keys[&ring.active],
                "synthetic",
                "m/synthetic/part/wrong",
                1,
                &sealed
            )
            .is_err());
        }
        assert_eq!(
            segmented(&ring, "m/synthetic/raw/base", 1, &data)
                .unwrap()
                .1,
            parts
        );
        assert_ne!(
            segmented(&ring, "m/synthetic/raw/base", 2, &data)
                .unwrap()
                .1[0]
                .0,
            parts[0].0
        );
    }
    #[test]
    fn meeting_stable_saves_do_not_create_or_project_sync_layers() {
        if crate::flavor::sync_enabled() {
            return;
        }
        let root = std::env::temp_dir().join(format!("sj-stable-format-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        fixture(&store, "fixture");
        let meetings = Meetings::new(store.clone());
        meetings
            .save(
                "fixture",
                "Synthetic stable title",
                "Synthetic stable text.",
                "Synthetic notes.",
            )
            .unwrap();
        assert!(!root.join("meeting-layers").exists());
        std::fs::create_dir(root.join("meeting-layers")).unwrap();
        std::fs::write(
            root.join("meeting-layers/fixture.json"),
            b"unreadable synthetic layer",
        )
        .unwrap();
        assert_eq!(
            meetings.get("fixture").unwrap().transcript,
            "Synthetic stable text."
        );
        meetings.delete("fixture").unwrap();
        assert!(!root.join("meeting-sync-deletions.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    fn fixture(store: &Store, id: &str) {
        store.write(&store.meetings_dir().join(format!("{id}.json")),&json!({"schemaVersion":1,
            "detail":{"id":id,"title":"Synthetic meeting","createdAt":"2026-01-01T12:00:00Z","durationSeconds":30.0,"status":"transcribed","hasAudio":false,"hasTranscript":true,
                "transcript":"First synthetic line.\nSecond synthetic line.","notes":"Synthetic notes.","source":"microphone","audioPath":null,"error":null,"cleanedTranscript":"First clean line.\nSecond clean line."},
            "rawTranscript":"First raw line.\nSecond raw line.","futureLegacyField":{"keep":true},"speakerNames":{"Speaker 1":"Synthetic speaker"}})).unwrap();
    }
    #[test]
    fn migration_preserves_original_json_and_recognition() {
        let root = std::env::temp_dir().join(format!("sj-layers-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        fixture(&store, "fixture");
        let original = std::fs::read(store.meetings_dir().join("fixture.json")).unwrap();
        let meetings = Meetings::new(store.clone());
        meetings.sync_collect().unwrap();
        assert_eq!(
            std::fs::read(root.join("meeting-layers/fixture.legacy")).unwrap(),
            original
        );
        let raw = meetings.sync_local("fixture").unwrap().layers.raw;
        meetings
            .save("fixture", "Edited title", "Edited line.", "Edited notes.")
            .unwrap();
        assert_eq!(meetings.sync_local("fixture").unwrap().layers.raw, raw);
        assert_eq!(meetings.get("fixture").unwrap().transcript, "Edited line.");
        assert_eq!(
            meetings
                .get("fixture")
                .unwrap()
                .cleaned_transcript
                .as_deref(),
            Some("First clean line.\nSecond clean line.")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn compression_round_trip_and_invalid_data() {
        let value = json!({"text": "Synthetic repeated meeting line. ".repeat(40000)});
        let encoded = encode_document(&value).unwrap();
        assert!(encoded.starts_with(b"SJM1"));
        assert!(encoded.len() < 512 * 1024);
        assert_eq!(decode_document(&encoded).unwrap(), value);
        assert!(decode_document(b"SJM1invalid").is_err());
    }
    #[test]
    fn receive_preserves_edits_made_while_http_is_in_flight() {
        if !crate::flavor::sync_enabled() {
            return;
        }
        let root = std::env::temp_dir().join(format!("sj-concurrent-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        fixture(&store, "fixture");
        let meetings = Meetings::new(store.clone());
        meetings.sync_collect().unwrap();
        let before = meetings.sync_local("fixture").unwrap().layers;
        let mut remote = before.clone();
        remote
            .head
            .fields
            .insert("title".into(), json!("Remote title"));
        meetings
            .save(
                "fixture",
                "Synthetic meeting",
                "Local edit during HTTP.",
                "Synthetic notes.",
            )
            .unwrap();
        meetings
            .sync_commit("fixture", &remote, &before, &mut BTreeMap::new())
            .unwrap();
        let detail = meetings.get("fixture").unwrap();
        assert_eq!(detail.title, "Remote title");
        assert_eq!(detail.transcript, "Local edit during HTTP.");
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn first_recognition_after_initial_migration_survives_remote_restore() {
        let root = std::env::temp_dir().join(format!("sj-first-{}", uuid::Uuid::new_v4()));
        let a = Store::open(Some(root.join("a"))).unwrap();
        let b = Store::open(Some(root.join("b"))).unwrap();
        fixture(&a, "fixture");
        let mut record: Value = a.read(&a.meetings_dir().join("fixture.json")).unwrap();
        record["rawTranscript"] = Value::Null;
        record["detail"]["transcript"] = json!("");
        a.write(&a.meetings_dir().join("fixture.json"), &record)
            .unwrap();
        Meetings::new(a.clone()).sync_collect().unwrap();
        record = local::load(&a, "fixture").unwrap().unwrap().record;
        record["rawTranscript"] = json!("First recognition.");
        record["detail"]["transcript"] = json!("Published recognition.");
        local::capture(&a, &record).unwrap();
        let layers = local::load(&a, "fixture").unwrap().unwrap().layers;
        let mb = Meetings::new(b.clone());
        mb.sync_commit("fixture", &layers, &layers, &mut BTreeMap::new())
            .unwrap();
        assert_eq!(
            mb.raw_transcript("fixture").unwrap().as_deref(),
            Some("First recognition.")
        );
        assert_eq!(
            mb.get("fixture").unwrap().transcript,
            "Published recognition."
        );
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn field_conflicts_can_choose_the_retained_value() {
        let root = std::env::temp_dir().join(format!("sj-field-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        fixture(&store, "fixture");
        let meetings = Meetings::new(store.clone());
        meetings.sync_collect().unwrap();
        let mut c = Credentials::new("http://127.0.0.1:1".into());
        let mut m = state(&store, &c).unwrap();
        let mut track = Track::default();
        track
            .conflicts
            .insert("head/fields/title".into(), json!("Other title"));
        m.meetings.insert("fixture".into(), track);
        save(&store, &m).unwrap();
        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(action(
                &store,
                &mut c,
                "meeting_field_resolve",
                &json!({"id":"fixture","field":"head/fields/title","useOther":true}),
            ))
            .unwrap();
        assert_eq!(meetings.get("fixture").unwrap().title, "Other title");
        assert!(state(&store, &c).unwrap().meetings["fixture"]
            .conflicts
            .is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn disappearance_guard_is_bounded_and_keep_detaches_without_deleting() {
        let root = std::env::temp_dir().join(format!("sj-missing-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let c = Credentials::new("https://sync.example.invalid".into());
        let mut m = state(&store, &c).unwrap();
        for i in 0..80 {
            m.meetings.insert(
                format!("fixture-{i}"),
                Track {
                    head: Some(Layers::empty(&format!("fixture-{i}")).head),
                    ..Default::default()
                },
            );
        }
        save(&store, &m).unwrap();
        let tracked = tracked_ids(&store).unwrap();
        let three: BTreeSet<_> = tracked.iter().take(3).cloned().collect();
        reconcile_missing(&store, &tracked, &three).unwrap();
        assert_eq!(deletions(&store).unwrap(), three);
        let four: BTreeSet<_> = tracked.iter().take(4).cloned().collect();
        reconcile_missing(&store, &tracked, &four).unwrap();
        assert!(deletions(&store).unwrap().is_empty());
        assert_eq!(state(&store, &c).unwrap().missing, four);
        // Confirmed deletions still count in the observed disappearance total,
        // but the guard preserves that explicit authorization in the queue.
        let confirmed = four.iter().next().unwrap();
        queue_delete(&store, confirmed).unwrap();
        reconcile_missing(&store, &tracked, &four).unwrap();
        assert_eq!(deletions(&store).unwrap(), BTreeSet::from([confirmed.clone()]));
        assert_eq!(state(&store, &c).unwrap().missing.len(), 3);
        cancel_delete(&store, confirmed).unwrap();
        reconcile_missing(&store, &tracked, &four).unwrap();
        resolve_missing(&store, &c, false).unwrap();
        let detached = state(&store, &c).unwrap();
        assert!(detached.missing.is_empty());
        assert!(four
            .iter()
            .all(|id| detached.meetings[id].status == "localOnly"));
        assert!(!root.join("meeting-trash").exists());
        let twenty: BTreeSet<_> = tracked.iter().take(20).cloned().collect();
        let one: BTreeSet<_> = twenty.iter().take(1).cloned().collect();
        reconcile_missing(&store, &twenty, &one).unwrap();
        assert_eq!(deletions(&store).unwrap(), one); // exactly 5% is allowed
        let nineteen: BTreeSet<_> = tracked.iter().take(19).cloned().collect();
        reconcile_missing(&store, &nineteen, &one).unwrap();
        assert!(deletions(&store).unwrap().is_empty());
        resolve_missing(&store, &c, true).unwrap();
        assert_eq!(deletions(&store).unwrap(), one);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn manual_sync_respects_rate_limit_but_clears_other_waits_and_returns_result() {
        let root = std::env::temp_dir().join(format!("sj-manual-retry-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let c = Credentials::new("https://sync.example.invalid".into());
        let mut m = state(&store, &c).unwrap();
        let error = format!("{} Retry in 120 seconds.", crate::sync_client::RATE_LIMITED);
        rate_limited(&store, &mut m, &error).unwrap();
        assert!(m.retry_at >= now() + 119);
        let rt = tokio::runtime::Runtime::new().unwrap();
        let result = rt
            .block_on(run(&store, &c, &Keyring::new(), 1, true))
            .unwrap();
        assert_eq!(result.retry_at, m.retry_at);
        assert_eq!(outcome(&result)["rateLimited"], true);
        m.rate_limited = false;
        m.retries.insert(
            "fixture".into(),
            MeetingRetry {
                at: now() + 3600,
                delay: 3600,
            },
        );
        save(&store, &m).unwrap();
        // Make the new attempt fail locally, proving the old wait was bypassed.
        std::fs::rename(store.meetings_dir(), root.join("missing-folder")).unwrap();
        let result = rt
            .block_on(run(&store, &c, &Keyring::new(), 1, true))
            .unwrap();
        assert!(result.errors.contains_key("pass"));
        assert!(result.retries.is_empty());
        assert_eq!(outcome(&result)["errors"], 1);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn corrupt_meeting_never_sets_library_backoff_or_overrides_rate_retry() {
        let previous = State {
            retry_delay: 3600,
            ..Default::default()
        };
        let mut m = State::default();
        meeting_failed(&mut m, "corrupt-fixture", "Invalid layers.".into());
        assert!(!meeting_ready(&m, "corrupt-fixture"));
        assert!(meeting_ready(&m, "healthy-fixture"));
        let mut m = complete_pass(&previous, m, false);
        assert_eq!(m.retry_at, 0);
        assert_eq!(m.retry_delay, 0);
        m.rate_limited = true;
        m.retry_at = now() + 61;
        let expected = m.retry_at;
        assert_eq!(complete_pass(&previous, m, false).retry_at, expected);
        assert!(complete_pass(&previous, State::default(), true).retry_at >= now() + 3599);
    }
    #[test]
    #[ignore = "Run sync-server npm run test:e2e -- --meeting-library against isolated Miniflare"]
    fn large_meeting_library_miniflare() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
        let root =
            std::env::temp_dir().join(format!("sj-meeting-library-{}", uuid::Uuid::new_v4()));
        let a = Store::open(Some(root.join("a"))).unwrap();
        let b = Store::open(Some(root.join("b"))).unwrap();
        let keys: Value =
            serde_json::from_str(&std::env::var("SONOJOT_SYNC_E2E_KEYS").unwrap()).unwrap();
        let mut ca = Credentials::new(std::env::var("SONOJOT_SYNC_E2E_URL").unwrap());
        ca.account = std::env::var("SONOJOT_SYNC_E2E_ACCOUNT").unwrap();
        ca.device_id = "a".into();
        ca.session = std::env::var("SONOJOT_SYNC_E2E_A").unwrap();
        let mut cb = Credentials::new(ca.server.clone());
        cb.account = ca.account.clone();
        cb.device_id = "b".into();
        cb.session = std::env::var("SONOJOT_SYNC_E2E_B").unwrap();
        for c in [&mut ca, &mut cb] {
            let k = &keys[&c.device_id];
            c.device = crypto::DeviceKey {
                secret: B64
                    .decode(k["secret"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
                public: B64
                    .decode(k["publicKey"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
            };
        }
        crate::secret_store::save(&a, &ca).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            crate::sync_account::action(&a, "setup", json!({"confirmed":true}))
                .await
                .unwrap();
            ca = crate::secret_store::load(&a).unwrap().unwrap();
            cb.ak = ca.ak.clone();
            cb.keyset = ca.keyset;
            let (ring, rev) = crate::sync_account::keyring(&a, &mut ca).await.unwrap();
            let ma = Meetings::new(a.clone());
            let mb = Meetings::new(b.clone());
            for i in 0..200 {
                fixture(&a, &format!("meeting-{i:03}"));
            }
            fixture(&a, "bad");
            ma.sync_collect().unwrap();
            std::fs::write(
                local::path(&a, "bad").unwrap(),
                b"synthetic future layer format",
            )
            .unwrap();
            let mut limited = 0;
            let mut passes = 0;
            for pass in 0..80 {
                let _: Value = ca
                    .client()
                    .unwrap()
                    .json(Method::POST, "/__test/clear-limits", None)
                    .await
                    .unwrap();
                // Advancing the synthetic rate window also releases its persisted wait.
                let mut prior = state(&a, &ca).unwrap();
                prior.retry_at = 0;
                save(&a, &prior).unwrap();
                let state = scheduled(&a, &ca, &ring, rev).await.unwrap();
                if state.rate_limited {
                    assert!(state.retry_at <= now() + 61);
                } else {
                    assert_eq!(
                        state.retry_at, 0,
                        "a corrupt meeting must not delay the library"
                    );
                }
                limited += usize::from(state.rate_limited);
                assert!(state.errors.contains_key("bad"));
                println!(
                    "upload pass {}: {} held, {} pending pulls",
                    pass + 1,
                    state.meetings.values().filter(|t| t.held_at > 0).count(),
                    state.pending_pulls.len()
                );
                if state
                    .meetings
                    .values()
                    .filter(|t| t.status == "synced" && t.held_at > 0)
                    .count()
                    == 200
                {
                    passes = pass + 1;
                    break;
                }
            }
            assert!(
                passes > 1 && passes <= 80,
                "library did not finish: {passes}"
            );
            assert!(limited > 0);
            for pass in 0..80 {
                let _: Value = cb
                    .client()
                    .unwrap()
                    .json(Method::POST, "/__test/clear-limits", None)
                    .await
                    .unwrap();
                let state = synchronize(&b, &cb, &ring, rev).await.unwrap();
                println!(
                    "download pass {}: {} held, {} pending pulls",
                    pass + 1,
                    state.meetings.values().filter(|t| t.held_at > 0).count(),
                    state.pending_pulls.len()
                );
                if state
                    .meetings
                    .values()
                    .filter(|t| t.status == "synced" && t.held_at > 0)
                    .count()
                    == 200
                {
                    passes += pass + 1;
                    break;
                }
            }
            assert_eq!(mb.list().unwrap().len(), 200);
            assert_eq!(
                state(&b, &cb)
                    .unwrap()
                    .meetings
                    .values()
                    .filter(|t| t.held_at > 0)
                    .count(),
                200
            );
            let _: Value = ca
                .client()
                .unwrap()
                .json(Method::POST, "/__test/clear-limits", None)
                .await
                .unwrap();
            let before: Value = ca
                .client()
                .unwrap()
                .json(Method::GET, "/__test/requests", None)
                .await
                .unwrap();
            synchronize(&a, &ca, &ring, rev).await.unwrap();
            synchronize(&b, &cb, &ring, rev).await.unwrap();
            let after: Value = ca
                .client()
                .unwrap()
                .json(Method::GET, "/__test/requests", None)
                .await
                .unwrap();
            assert_eq!(before["referencePosts"], after["referencePosts"]);
            assert_eq!(before["holdPosts"], after["holdPosts"]);
            let mut expired = state(&a, &ca).unwrap();
            expired.meetings.get_mut("meeting-050").unwrap().held_at = now() - 8 * 86400;
            save(&a, &expired).unwrap();
            synchronize(&a, &ca, &ring, rev).await.unwrap();
            let renewed: Value = ca
                .client()
                .unwrap()
                .json(Method::GET, "/__test/requests", None)
                .await
                .unwrap();
            assert_eq!(after["referencePosts"], renewed["referencePosts"]);
            assert_eq!(
                after["holdPosts"].as_u64().unwrap() + 1,
                renewed["holdPosts"].as_u64().unwrap()
            );
            // A receiving meeting that becomes busy cannot block a later meeting.
            let mut record: Value = b.read(&b.meetings_dir().join("meeting-000.json")).unwrap();
            record["detail"]["status"] = json!("recording");
            b.write(&b.meetings_dir().join("meeting-000.json"), &record)
                .unwrap();
            for id in ["meeting-000", "meeting-199"] {
                ma.save(
                    id,
                    "Synthetic changed title",
                    "Synthetic changed line.",
                    "Synthetic notes.",
                )
                .unwrap();
            }
            let _: Value = ca
                .client()
                .unwrap()
                .json(Method::POST, "/__test/clear-limits", None)
                .await
                .unwrap();
            let key = state(&a, &ca).unwrap().meetings["meeting-000"].key.clone();
            let _: Value = ca
                .client()
                .unwrap()
                .json(
                    Method::POST,
                    "/__test/reject-write",
                    Some(&json!({"object":format!("m/{key}/head")})),
                )
                .await
                .unwrap();
            let collision = synchronize(&a, &ca, &ring, rev).await.unwrap();
            assert!(collision.errors.contains_key("meeting-000"));
            assert_eq!(
                collision.meetings["meeting-199"]
                    .head
                    .as_ref()
                    .unwrap()
                    .fields["title"],
                "Synthetic changed title"
            );
            run(&a, &ca, &ring, rev, true).await.unwrap();
            let result = synchronize(&b, &cb, &ring, rev).await.unwrap();
            assert!(!result.errors.is_empty());
            assert_eq!(
                mb.get("meeting-199").unwrap().title,
                "Synthetic changed title"
            );
            record["detail"]["status"] = json!("transcribed");
            b.write(&b.meetings_dir().join("meeting-000.json"), &record)
                .unwrap();
            run(&b, &cb, &ring, rev, true).await.unwrap();
            assert_eq!(
                mb.get("meeting-000").unwrap().title,
                "Synthetic changed title"
            );
            // A remote text-only deletion prompts on the device with unsynced audio.
            let wave = mb.recording_paths("meeting-199").unwrap().microphone;
            std::fs::write(&wave, b"synthetic wav").unwrap();
            ma.delete("meeting-199").unwrap();
            synchronize(&a, &ca, &ring, rev).await.unwrap();
            synchronize(&b, &cb, &ring, rev).await.unwrap();
            assert!(wave.exists());
            assert!(state(&b, &cb).unwrap().meetings["meeting-199"].delete_conflict);
            // Meeting backoff must not make a successful vocabulary pass fail.
            let mut retry = state(&a, &ca).unwrap();
            retry.retry_at = now() + 600;
            save(&a, &retry).unwrap();
            assert!(crate::sync_account::action(&a, "now", json!({}))
                .await
                .is_ok());
            eprintln!("200-meeting convergence: {passes} rate-limit windows");
        });
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    #[ignore = "Run sync-server npm run test:e2e -- --meetings against isolated Miniflare"]
    fn two_device_meeting_miniflare() {
        use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
        use sha2::Digest;
        let root = std::env::temp_dir().join(format!("sj-meeting-e2e-{}", uuid::Uuid::new_v4()));
        let a = Store::open(Some(root.join("a"))).unwrap();
        let b = Store::open(Some(root.join("b"))).unwrap();
        let keys: Value =
            serde_json::from_str(&std::env::var("SONOJOT_SYNC_E2E_KEYS").unwrap()).unwrap();
        let mut ca = Credentials::new(std::env::var("SONOJOT_SYNC_E2E_URL").unwrap());
        ca.account = std::env::var("SONOJOT_SYNC_E2E_ACCOUNT").unwrap();
        ca.device_id = "a".into();
        ca.session = std::env::var("SONOJOT_SYNC_E2E_A").unwrap();
        let mut cb = Credentials::new(ca.server.clone());
        cb.account = ca.account.clone();
        cb.device_id = "b".into();
        cb.session = std::env::var("SONOJOT_SYNC_E2E_B").unwrap();
        for c in [&mut ca, &mut cb] {
            let k = &keys[&c.device_id];
            c.device = crypto::DeviceKey {
                secret: B64
                    .decode(k["secret"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
                public: B64
                    .decode(k["publicKey"].as_str().unwrap())
                    .unwrap()
                    .try_into()
                    .unwrap(),
            };
        }
        crate::secret_store::save(&a, &ca).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async{
            crate::sync_account::action(&a,"setup",json!({"confirmed":true})).await.unwrap();
            ca=crate::secret_store::load(&a).unwrap().unwrap();cb.ak=ca.ak.clone();cb.keyset=ca.keyset;
            crate::secret_store::save(&b,&cb).unwrap();
            let(ring,rev)=crate::sync_account::keyring(&a,&mut ca).await.unwrap();
            fixture(&a,"fixture");let ma=Meetings::new(a.clone());let mb=Meetings::new(b.clone());
            synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            assert_eq!(ma.get("fixture").unwrap().transcript,mb.get("fixture").unwrap().transcript);
            assert_eq!(ma.raw_transcript("fixture").unwrap(),mb.raw_transcript("fixture").unwrap());
            assert_eq!(ma.get("fixture").unwrap().cleaned_transcript,mb.get("fixture").unwrap().cleaned_transcript);
            // A malicious service can lie about state, but cannot erase local files.
            let identity = state(&a,&ca).unwrap().meetings["fixture"].key.clone();
            let wave = mb.recording_paths("fixture").unwrap().microphone;
            std::fs::write(&wave,b"synthetic wav content").unwrap();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/meeting-state",Some(&json!({"id":identity,"state":"deleted"}))).await.unwrap();
            synchronize(&b,&cb,&ring,rev).await.unwrap();
            assert!(wave.exists()); assert!(b.meetings_dir().join("fixture.json").exists());
            assert!(state(&b,&cb).unwrap().meetings["fixture"].delete_conflict);
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/meeting-state",Some(&json!({"id":identity,"state":"active"}))).await.unwrap();
            let mut tracked=state(&b,&cb).unwrap(); tracked.meetings.get_mut("fixture").unwrap().delete_conflict=false; tracked.meetings.get_mut("fixture").unwrap().status="synced".into(); save(&b,&tracked).unwrap();
            ma.save("fixture","Synthetic meeting","Edited on A.\nSecond synthetic line.","Synthetic notes.").unwrap();
            mb.save("fixture","Synthetic meeting","Edited on B.\nSecond synthetic line.","Synthetic notes.").unwrap();
            synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();synchronize(&a,&ca,&ring,rev).await.unwrap();
            assert_eq!(ma.get("fixture").unwrap().transcript,mb.get("fixture").unwrap().transcript);
            let conflicts=ma.sync_local("fixture").unwrap().layers.rendered("original").unwrap().1;
            assert_eq!(conflicts.len(),1);
            action(&a,&mut ca,"meeting_resolve",&json!({"id":"fixture","sid":conflicts[0].sid,"view":"original","version":conflicts[0].version})).await.unwrap();
            let json:Value=a.read(&a.meetings_dir().join("fixture.json")).unwrap();
            assert_eq!(json["detail"]["transcript"],ma.sync_local("fixture").unwrap().layers.text("original").unwrap());
            synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            assert!(mb.sync_local("fixture").unwrap().layers.rendered("original").unwrap().1.is_empty());
            // An interrupted chunk upload resumes from immutable, verified blobs.
            let key=state(&a,&ca).unwrap().meetings["fixture"].key.clone();let dir=audio::directory(&a,"fixture").unwrap();
            let synthetic=vec![7u8;audio::CHUNK];let chunk="synthetic-audio";
            std::fs::write(dir.join(chunk),&synthetic).unwrap();
            let manifest=audio::Manifest{schema:1,codec:"ogg-opus-mono-24000".into(),stems:vec![audio::Stem{source:"mixed".into(),chunks:vec![audio::Chunk{id:chunk.into(),sha256:audio::hash(&synthetic),bytes:synthetic.len()}]}]};
            audio::upload(&a,&ca,&ring,&key,"fixture",&manifest,&mut BTreeSet::new()).await.unwrap();
            audio::upload(&a,&ca,&ring,&key,"fixture",&manifest,&mut BTreeSet::new()).await.unwrap();
            audio::download(&b,&cb,&ring,&key,"fixture",&manifest).await.unwrap();assert!(manifest.complete(&audio::directory(&b,"fixture").unwrap()));
            // A head-bound reference snapshot distinguishes this abandoned chunk
            // from committed audio. Resuming must allocate a fresh immutable ID.
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/expire-upload",Some(&json!({"id":format!("blob/{key}/{chunk}")}))).await.unwrap();
            assert!(inventory(&ca,&key).await.unwrap().objects.iter().any(|o|o.id==format!("blob/{key}/{chunk}")&&o.deleted==4));
            let mut pending=ma.sync_local("fixture").unwrap();pending.audio_override=Some(true);pending.layers.head.audio=Some(manifest.clone());local::save(&a,&pending).unwrap();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            let resumed=mb.sync_local("fixture").unwrap().layers.head.audio.unwrap();
            assert_ne!(resumed.stems[0].chunks[0].id,chunk);
            audio::download(&b,&cb,&ring,&key,"fixture",&resumed).await.unwrap();assert!(resumed.complete(&audio::directory(&b,"fixture").unwrap()));
            // Large incompressible notes traverse real encrypted parts; a validly
            // encrypted manifest with the wrong part hash is still rejected.
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            let random:Vec<u8>=(0..40000u64).flat_map(|i|sha2::Sha256::digest(i.to_be_bytes())).collect();
            let large=json!({"schema":SCHEMA,"body":B64.encode(random)});
            let object=format!("m/{key}/notes/large");
            let published=put(&ca,&ring,&object,0,&large).await.unwrap();assert!(published.parts.len()>3);
            assert_eq!(get(&cb,&ring,&Object{id:object.clone(),rev:1,deleted:0}).await.unwrap(),large);
            let (plain,_)=segmented(&ring,&object,2,&encode_document(&large).unwrap()).unwrap();
            let mut bad:TextManifest=serde_json::from_slice(&plain[4..]).unwrap();
            for (part,id) in bad.parts.iter_mut().zip(&published.parts) {part.id=id.rsplit('/').next().unwrap().into();}
            bad.parts[0].sha256="0".repeat(64);
            let mut bytes=b"SJM2".to_vec();bytes.extend(serde_json::to_vec(&bad).unwrap());
            let sealed=crypto::seal(&ring.keys[&ring.active],ring.active,&ca.account,&object,2,&bytes).unwrap();
            ca.client().unwrap().request(Method::PUT,&path(&object),None,Some(sealed),Some(1),ring.epoch).await.unwrap();
            assert!(get(&cb,&ring,&Object{id:object.clone(),rev:2,deleted:0}).await.unwrap_err().contains("checksum"));
            put(&ca,&ring,&object,2,&large).await.unwrap();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            let before:Value=ca.client().unwrap().json(Method::GET,"/__test/requests",None).await.unwrap();
            let usage=crate::sync_engine::rotation_usage(&a,&ca,&ring).await.unwrap();
            assert!(usage.objects.keys().all(|id|!id.starts_with("blob/")));
            let after:Value=ca.client().unwrap().json(Method::GET,"/__test/requests",None).await.unwrap();
            assert_eq!(before["blobDownloads"],after["blobDownloads"]);
            // Transfer and authenticate a real paged checkpoint, then reject a changed pin.
            let mut rotated=ring.clone();rotated.epoch+=1;
            rotated.object_cutovers=(0..5000).map(|i|(format!("m/synthetic-{i:05}/raw/base"),1)).collect();
            let next=crypto::Secret::random();crate::sync_cutovers::publish(&ca,&next,&mut rotated).await.unwrap();
            let mut receiving=ca.clone();receiving.ak=Some(next);
            let expected=rotated.object_cutovers.clone();rotated.object_cutovers.clear();
            crate::sync_cutovers::hydrate(&b, &receiving,&mut rotated).await.unwrap();assert_eq!(rotated.object_cutovers,expected);
            let count:Value=ca.client().unwrap().json(Method::GET,"/__test/requests",None).await.unwrap();
            rotated.object_cutovers.clear();
            crate::sync_cutovers::hydrate(&b,&receiving,&mut rotated).await.unwrap();
            let cached:Value=ca.client().unwrap().json(Method::GET,"/__test/requests",None).await.unwrap();
            assert_eq!(count["checkpointDownloads"],cached["checkpointDownloads"]);
            assert_eq!(rotated.object_cutovers,expected);
            rotated.cutover_checkpoint.as_mut().unwrap().sha256[0]^=1;
            assert!(crate::sync_cutovers::hydrate(&b, &receiving,&mut rotated).await.is_err());
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            mb.save("fixture","Offline draft","An unsynced local edit.","Synthetic notes.").unwrap();
            ma.delete("fixture").unwrap();synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            assert!(state(&b,&cb).unwrap().meetings["fixture"].delete_conflict);
            action(&b,&mut cb,"meeting_delete_resolve",&json!({"id":"fixture","keep":true})).await.unwrap();
            synchronize(&b,&cb,&ring,rev).await.unwrap();assert_eq!(mb.get("fixture").unwrap().transcript,"An unsynced local edit.");
            assert!(mb.sync_local("fixture").unwrap().local_only);
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            fixture(&a,"clean-delete");
            std::fs::create_dir(a.meetings_dir().join("clean-delete.asr")).unwrap();
            a.write(&a.meetings_dir().join("clean-delete.asr/synthetic.json"),&json!({"raw":"Synthetic retained history."})).unwrap();
            synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            let owned=mb.recording_paths("clean-delete").unwrap().microphone;
            std::fs::write(&owned,b"synthetic retained audio").unwrap();
            let history=b.meetings_dir().join("clean-delete.asr").join("synthetic.json");
            assert!(history.exists());
            std::fs::write(b.root().join("meeting-layers/clean-delete.legacy"),b"synthetic retained legacy").unwrap();
            ma.delete("clean-delete").unwrap();synchronize(&a,&ca,&ring,rev).await.unwrap();synchronize(&b,&cb,&ring,rev).await.unwrap();
            assert!(owned.exists());assert!(state(&b,&cb).unwrap().meetings["clean-delete"].delete_conflict);
            action(&b,&mut cb,"meeting_delete_resolve",&json!({"id":"clean-delete","keep":false})).await.unwrap();
            assert_eq!(state(&b,&cb).unwrap().meetings["clean-delete"].status,"deleted");
            assert!(!owned.exists());assert!(!history.exists());assert!(!b.meetings_dir().join("clean-delete.json").exists());
            let trash=std::fs::read_dir(b.root().join("meeting-trash")).unwrap().next().unwrap().unwrap().path();
            assert_eq!(std::fs::read(trash.join(owned.strip_prefix(b.root()).unwrap())).unwrap(),b"synthetic retained audio");
            assert!(trash.join("meeting-layers/clean-delete.json").exists());
            assert!(trash.join("meeting-layers/clean-delete.legacy").exists());
            assert!(trash.join(history.strip_prefix(b.root()).unwrap()).exists());
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            fixture(&a,"evicted-delete");synchronize(&a,&ca,&ring,rev).await.unwrap();
            let evicted=state(&a,&ca).unwrap().meetings["evicted-delete"].key.clone();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/drop-meeting",Some(&json!({"id":evicted}))).await.unwrap();
            ma.delete("evicted-delete").unwrap();synchronize(&a,&ca,&ring,rev).await.unwrap();
            assert!(deletions(&a).unwrap().is_empty());
            fixture(&a,"rotate-retained");synchronize(&a,&ca,&ring,rev).await.unwrap();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/clear-limits",None).await.unwrap();
            let _:Value=ca.client().unwrap().json(Method::POST,"/__test/pad-inventory",Some(&json!({"account":ca.account}))).await.unwrap();
            // Exactly 500 inventory rows includes an empty terminal page.
            crate::sync_engine::rotation_usage(&a,&ca,&ring).await.unwrap();
            // Model an already-confirmed pairing; the vocabulary e2e covers the
            // interactive handshake. Keep B while removing the third session.
            ca.trusted.insert(cb.device_id.clone(),cb.device.public);
            cb.trusted.insert(ca.device_id.clone(),ca.device.public);
            crate::secret_store::save(&a,&ca).unwrap();
            crate::secret_store::save(&b,&cb).unwrap();
            crate::sync_account::action(&a,"remove",json!({"device":"lost-device","confirmed":true})).await.unwrap();
            ca=crate::secret_store::load(&a).unwrap().unwrap();
            let (new_ring,_)=crate::sync_account::keyring(&a,&mut ca).await.unwrap();
            assert!(new_ring.cutover_checkpoint.is_some());assert!(!new_ring.object_cutovers.is_empty());
            // Automatic key updates must hydrate the pinned checkpoint before
            // a retained device opens pre-rotation meeting heads.
            let (received,received_rev)=crate::sync_account::keyring(&b,&mut cb).await.unwrap();
            assert_eq!(received.object_cutovers,new_ring.object_cutovers);
            synchronize(&b,&cb,&received,received_rev).await.unwrap();
            assert_eq!(mb.get("rotate-retained").unwrap().title,"Synthetic meeting");
            // The cached wrap serializes only the checkpoint pin. A forced pull
            // through `now` must hydrate it before opening retained old-key heads.
            let mut tracked=state(&a,&ca).unwrap();tracked.cursor=0;tracked.ring_rev=0;save(&a,&tracked).unwrap();
            crate::sync_account::action(&a,"now",json!({})).await.unwrap();
            assert_eq!(ma.get("rotate-retained").unwrap().title,"Synthetic meeting");
        });
        std::fs::remove_dir_all(root).unwrap();
    }
}
