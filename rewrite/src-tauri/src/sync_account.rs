use crate::{
    app::Runtime,
    secret_store,
    store::Store,
    sync_client::{SyncClient, Wrap, STAGING, AUTH_ORIGIN},
    sync_crypto::{self as crypto, DeviceKey, Keyring, Secret},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD as B64, Engine};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, sync::Arc};
use tauri::{Emitter, Manager};
use zeroize::Zeroize;

#[derive(Serialize, Deserialize, Clone)]
pub struct Pending {
    request: String,
    verifier: String,
    expires: u64,
    #[serde(default)]
    device: DeviceKey,
    #[serde(default)]
    device_id: String,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct PairState {
    pub id: String,
    pub nonce: [u8; 32],
    pub public: [u8; 32],
    pub commitment: [u8; 32],
    pub approver: bool,
    #[serde(default)]
    pub peer_nonce: Option<[u8; 32]>,
    #[serde(default)]
    pub transcript: Option<crypto::PairTranscript>,
    #[serde(default)]
    pub expires_at: u64,
    /// Last server state this computer saw, and when it changed. A request
    /// that stops moving is replaced (receiver) or forgotten (approver).
    #[serde(default)]
    pub state: String,
    #[serde(default)]
    pub progress_at: u64,
}
/// How long a request waits for one of the person's other computers.
/// Matches the server; an older server answers with its own shorter expiry.
const PAIR_LIFETIME: u64 = 86_400;
/// A claimed request that has not moved for this long is abandoned, for
/// example when an older app is waiting for someone to press Approve.
const PAIR_STALL: u64 = 600;
/// An automatic approver seals the key within seconds of reaching "ready".
/// A request that sits there was claimed by an older app waiting for a person.
const PAIR_READY_STALL: u64 = 90;
/// Background pairing poll: idle and while an exchange is moving.
const PAIR_IDLE: u64 = 20;
const PAIR_FAST: u64 = 3;
const JOINS: &str = "sync-joins.json";
#[derive(Serialize, Deserialize, Clone)]
pub struct Credentials {
    pub server: String,
    pub device_id: String,
    pub device: DeviceKey,
    pub account: String,
    pub email: String,
    pub session: String,
    pub ak: Option<Secret>,
    pub recovery: Option<String>,
    pending: Option<Pending>,
    pub pairs: BTreeMap<String, PairState>,
    #[serde(default)]
    pub trusted: BTreeMap<String, [u8; 32]>,
    #[serde(default)]
    pub keyset: Option<[u8; 32]>,
    #[serde(default)]
    pub initialized: Option<bool>,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.session.zeroize();
        self.recovery.zeroize();
        if let Some(p) = &mut self.pending {
            p.verifier.zeroize()
        }
    }
}
impl Credentials {
    pub fn new(server: String) -> Self {
        Self {
            server,
            device_id: uuid::Uuid::new_v4().to_string(),
            device: DeviceKey::new(),
            account: String::new(),
            email: String::new(),
            session: String::new(),
            ak: None,
            recovery: None,
            pending: None,
            pairs: BTreeMap::new(),
            trusted: BTreeMap::new(),
            keyset: None,
            initialized: None,
        }
    }
    pub fn client(&self) -> Result<SyncClient, String> {
        let client = SyncClient::new(&self.server, &self.session)?;
        Ok(match &self.ak {
            Some(ak) => client.with_proof(ak, &self.account),
            None => client,
        })
    }
}
#[derive(Default)]
pub struct SyncService {
    pub gate: tokio::sync::Mutex<()>,
    pub failure: std::sync::Mutex<Option<String>>,
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn text<'a>(v: &'a Value, key: &str) -> Result<&'a str, String> {
    v[key].as_str().ok_or_else(|| format!("Missing {key}."))
}
fn confirmed(args: &Value) -> Result<(), String> {
    if args["confirmed"] == true {
        Ok(())
    } else {
        Err("Confirm this action first.".into())
    }
}
fn removal_rekeys(args: &Value, target: &str, current: &str) -> Result<bool, String> {
    match args.get("rekey") {
        None | Some(Value::Null) => Ok(target != current),
        Some(Value::Bool(value)) => Ok(*value),
        _ => Err("Re-key must be true or false.".into()),
    }
}
const RESET_NOTICE: &str = "sync-reset-notice.json";
const RESET_NOTICE_TITLE: &str = "SonoJot cancelled a sync reset";
const RESET_NOTICE_BODY: &str = "This unlocked device protected your synced words. If you did not request the reset, secure your sign-in account and re-key from Account and sync.";

async fn cancel_reset(store: &Store, c: &Credentials) -> Result<(), String> {
    let result: Value = c.client()?.json(Method::DELETE, "/v1/reset", None).await?;
    if result["cancelled"] == true {
        store.write(
            &store.root().join(RESET_NOTICE),
            &json!({"account":c.account,"server":c.server,"delivered":false}),
        )?;
    }
    store.write(&store.root().join("sync-remote.json"), &Value::Null)
}
async fn cancel_observed_reset(
    store: &Store,
    c: &Credentials,
    poll: &mut Value,
) -> Result<(), String> {
    if c.ak.is_some() && poll["ready_at"].is_number() {
        cancel_reset(store, c).await?;
        poll["ready_at"] = Value::Null;
        poll["requested_at"] = Value::Null;
    }
    Ok(())
}
fn deliver_reset_notice(
    store: &Store,
    send: impl FnOnce(&str, &str) -> Result<(), String>,
) -> Result<(), String> {
    let path = store.root().join(RESET_NOTICE);
    let mut notice: Value = store.read(&path).unwrap_or(Value::Null);
    let Some(c) = secret_store::load::<Credentials>(store)? else {
        return Ok(());
    };
    if notice["account"] == c.account
        && notice["server"] == c.server
        && notice["delivered"] == false
    {
        send(RESET_NOTICE_TITLE, RESET_NOTICE_BODY)?;
        notice["delivered"] = json!(true);
        store.write(&path, &notice)?;
    }
    Ok(())
}
fn decode32(s: &str) -> Result<[u8; 32], String> {
    B64.decode(s)
        .map_err(|_| "Invalid pairing data.")?
        .try_into()
        .map_err(|_| "Invalid pairing data.".into())
}
pub fn status(store: &Store) -> Result<Value, String> {
    let saved = secret_store::load::<Credentials>(store)?;
    Ok(match saved {
        None => json!({"signedIn":false,"unlocked":false,"server":STAGING}),
        Some(c) => {
            let meta = crate::sync_engine::metadata(store, &c)?;
            let remote: Value = store
                .read(&store.root().join("sync-remote.json"))
                .unwrap_or(Value::Null);
            let pending_reset = if remote["account"] == c.account && remote["server"] == c.server {
                remote["ready_at"].clone()
            } else {
                Value::Null
            };
            let notice: Option<String> = store.read(&store.root().join("sync-notice.json")).ok();
            let reset_notice: Value = store
                .read(&store.root().join("sync-reset-notice.json"))
                .unwrap_or(Value::Null);
            json!({"signedIn":!c.session.is_empty(),"unlocked":c.ak.is_some(),"email":c.email,"account":c.account,"device":c.device_id,"server":c.server,"recoveryPending":c.recovery.is_some(),"pendingSignin":c.pending.is_some(),"initialized":remote["ring_rev"].as_u64().filter(|_| remote["account"] == c.account && remote["server"] == c.server).map(|r| r > 0).or(c.initialized),"lastSync":meta.last_sync,"conflicts":meta.conflicts.len(),"pendingReset":pending_reset,"notice":notice,"meetings":crate::meeting_sync::outcome(&crate::meeting_sync::state(store, &c)?),"resetCancelled":reset_notice["account"] == c.account && reset_notice["server"] == c.server,"joins":if c.ak.is_some() { pending_joins(store, &c) } else { Vec::new() }})
        }
    })
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct KeyTransfer {
    ak: Secret,
    ring_rev: u64,
    vocabulary_rev: u64,
    epoch: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    trust: Option<TrustedRoster>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct TrustedRoster {
    keyset: [u8; 32],
    devices: BTreeMap<String, [u8; 32]>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RosterProof {
    sender: String,
    ciphertext: String,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationMessage {
    previous: [u8; 32],
    transfer: KeyTransfer,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationWrap {
    sender: String,
    epoch: u64,
    ciphertext: String,
    #[serde(default)]
    introduction: Option<String>,
}
const REPAIR_TRUST: &str = "This computer cannot verify the key change. Sign out and add it again, or use the newest recovery code.";

fn accept_roster(c: &Credentials, encoded: &str) -> Result<TrustedRoster, String> {
    let proof: RosterProof = serde_json::from_slice(&B64.decode(encoded).map_err(|_| REPAIR_TRUST)?).map_err(|_| REPAIR_TRUST)?;
    let public = c.trusted.get(&proof.sender).ok_or(REPAIR_TRUST)?;
    let plain = crypto::unwrap_from(&c.device, public, &c.account, &format!("roster/{}", c.device_id), &B64.decode(&proof.ciphertext).map_err(|_| REPAIR_TRUST)?)?;
    let roster: TrustedRoster = serde_json::from_slice(&plain).map_err(|_| REPAIR_TRUST)?;
    if Some(roster.keyset) != c.keyset || roster.devices.len() > 64
        || roster.devices.get(&c.device_id) != Some(&c.device.public)
        || roster.devices.get(&proof.sender) != Some(public) {
        return Err(REPAIR_TRUST.into());
    }
    Ok(roster)
}
async fn publish_roster(c: &Credentials) -> Result<(), String> {
    let roster = TrustedRoster { keyset: c.keyset.ok_or(REPAIR_TRUST)?, devices: c.trusted.clone() };
    let plain = zeroize::Zeroizing::new(serde_json::to_vec(&roster).map_err(|_| REPAIR_TRUST)?);
    let client = c.client()?;
    let wraps = client.wraps().await?;
    for (id, public) in &c.trusted {
        if id == &c.device_id { continue; }
        let proof = RosterProof { sender: c.device_id.clone(), ciphertext: B64.encode(crypto::wrap_from(&c.device, public, &c.account, &format!("roster/{id}"), &plain)?) };
        let previous = wraps.iter().find(|w| w.kind == "roster" && &w.reference == id).map_or(0, |w| w.rev);
        client.put_wrap("roster", id, B64.encode(serde_json::to_vec(&proof).map_err(|_| REPAIR_TRUST)?), previous).await?;
    }
    Ok(())
}
fn rotation_keys(c: &Credentials, wraps: &[Wrap], keyring: &Wrap) -> Result<(KeyTransfer, Keyring), String> {
    let w = wraps.iter().find(|w| w.kind == "device" && w.reference == c.device_id).ok_or(REPAIR_TRUST)?;
    let envelope: RotationWrap = serde_json::from_slice(&B64.decode(&w.ciphertext).map_err(|_| REPAIR_TRUST)?).map_err(|_| REPAIR_TRUST)?;
    let mut trusted = c.trusted.clone();
    // An offline receiver may not have seen the pairing that introduced this
    // sender. Carry its existing, authenticated roster update across the rotation.
    if let Some(proof) = &envelope.introduction {
        if let Ok(roster) = accept_roster(c, proof) { trusted = roster.devices; }
    }
    let sender = trusted.get(&envelope.sender).ok_or(REPAIR_TRUST)?;
    let plain = crypto::unwrap_from(&c.device, sender, &c.account,
        &format!("rekey/{}/{}", c.device_id, envelope.epoch),
        &B64.decode(&envelope.ciphertext).map_err(|_| REPAIR_TRUST)?)?;
    let message: RotationMessage = serde_json::from_slice(&plain).map_err(|_| REPAIR_TRUST)?;
    let t = message.transfer;
    let roster = t.trust.as_ref().ok_or(REPAIR_TRUST)?;
    if c.keyset != Some(message.previous) || t.epoch != envelope.epoch || keyring.rev < t.ring_rev
        || roster.devices.len() > 64 || roster.devices.get(&c.device_id) != Some(&c.device.public)
        || roster.devices.get(&envelope.sender) != Some(sender) {
        return Err(REPAIR_TRUST.into());
    }
    let ring = Keyring::open(&t.ak, &c.account, keyring.rev, &B64.decode(&keyring.ciphertext).map_err(|_| REPAIR_TRUST)?)?;
    if ring.epoch != t.epoch || crypto::keyset_pin(&ring) != roster.keyset {
        return Err(REPAIR_TRUST.into());
    }
    Ok((t, ring))
}
pub async fn keyring(store: &Store, c: &mut Credentials) -> Result<(Keyring, u64), String> {
    let wraps = c.client()?.wraps().await?;
    let w = wraps
        .iter()
        .find(|w| w.kind == "keyring" && w.reference == "main")
        .ok_or("Sync keys are not initialized.")?;
    let meta = crate::sync_engine::metadata(store, c)?;
    if w.rev < meta.ring_rev {
        return Err("The service returned older encryption keys.".into());
    }
    let bytes = B64.decode(&w.ciphertext).map_err(|_| "Invalid keyring.")?;
    let ak =
        c.ak.as_ref()
            .ok_or("This computer is still waiting for its key. Use your recovery code if no other computer is available.")?;
    let mut ring = match Keyring::open(ak, &c.account, w.rev, &bytes) {
        Ok(ring) => { check_keyset(c, &ring)?; ring }
        Err(_) => {
            let (transfer, ring) = rotation_keys(c, &wraps, w).map_err(|_| REPAIR_TRUST)?;
            if transfer.epoch <= meta.epoch { return Err(REPAIR_TRUST.into()); }
            let roster = transfer.trust.as_ref().ok_or(REPAIR_TRUST)?;
            c.ak = Some(transfer.ak.clone());
            c.keyset = Some(roster.keyset);
            c.trusted = roster.devices.clone();
            // Persist the new pin and AK together before advancing rollback floors.
            secret_store::save(store, c)?;
            apply_checkpoint(store, c, &transfer)?;
            ring
        }
    };
    if let Some(w) = wraps.iter().find(|w| w.kind == "roster" && w.reference == c.device_id) {
        if let Ok(roster) = accept_roster(c, &w.ciphertext) {
            // Within one keyset, roster proofs only add computers; a re-key
            // changes the keyset and replaces trust. Merge, so an older proof
            // (for example the one that introduced this computer) never drops
            // a computer it approved since, and never replaces a pinned key.
            let before = c.trusted.len();
            for (id, public) in roster.devices {
                c.trusted.entry(id).or_insert(public);
            }
            if c.trusted.len() != before { secret_store::save(store, c)?; }
        }
    }
    crate::sync_cutovers::hydrate(store, c, &mut ring).await?;
    store.write(
        &store.root().join("sync-keyring.json"),
        &json!({"account":c.account,"server":c.server,"wrap":w}),
    )?;
    Ok((ring, w.rev))
}
/// Opens the key package only if it was sealed in authenticated mode by the
/// approver named in the transcript, bound to this exact exchange. Nobody
/// compares digits any more, so the transcript itself is the binding.
fn authenticated_transfer(
    device: &DeviceKey,
    transcript: &crypto::PairTranscript,
    ciphertext: &[u8],
) -> Result<KeyTransfer, String> {
    let plain = crypto::unwrap_from(
        device,
        &transcript.sender,
        &transcript.account,
        &transcript.binding(),
        ciphertext,
    )?;
    serde_json::from_slice(&plain).map_err(|_| "Invalid pairing keys.".into())
}
fn check_keyset(c: &Credentials, ring: &Keyring) -> Result<(), String> {
    if c.keyset != Some(crypto::keyset_pin(ring)) {
        return Err("Encryption keys changed. Sign out and add this computer again, or use the new recovery code.".into());
    }
    Ok(())
}
fn apply_checkpoint(store: &Store, c: &Credentials, t: &KeyTransfer) -> Result<(), String> {
    let mut meta = crate::sync_engine::metadata(store, c)?;
    meta.rev = meta.rev.max(t.vocabulary_rev);
    meta.ring_rev = meta.ring_rev.max(t.ring_rev);
    meta.epoch = meta.epoch.max(t.epoch);
    store.write(&store.root().join("sync-state.json"), &meta)
}
async fn rekey(
    store: &Store,
    c: &mut Credentials,
    target: &str,
    reset: bool,
) -> Result<(), String> {
    let client = c.client()?;
    let account: Value = client.json(Method::GET, "/v1/account", None).await?;
    let epoch = account["epoch"].as_u64().ok_or("Invalid account epoch.")?;
    let (mut ring, rev) = if reset {
        let wraps = client.wraps().await?;
        let rev = wraps
            .iter()
            .find(|w| w.kind == "keyring")
            .ok_or("No sync data to reset.")?
            .rev;
        (Keyring::new(), rev)
    } else {
        keyring(store, c).await?
    };
    let client = c.client()?;
    // Inspect the remote revision without uploading pending local edits under old keys.
    let (vocabulary_revision, prune_cursor) = if reset {
        (0, None)
    } else {
        let usage = crate::sync_engine::rotation_usage(store, c, &ring).await?;
        ring.rotate_retaining(&usage.keys)?;
        ring.vocabulary_cutover = usage.vocabulary_rev;
        ring.object_cutovers = usage.objects;
        (usage.vocabulary_rev, Some(usage.cursor))
    };
    // Only OS-pinned peers or an authenticated introduction can receive a new
    // AK. The old shared-key roster and the server's device list cannot add trust.
    let old_wraps = client.wraps().await?;
    let previous_pin = c.keyset.unwrap_or([0; 32]);
    let active: Vec<Value> = client.json(Method::GET, "/v1/devices", None).await?;
    if reset { c.trusted.clear(); }
    c.trusted.insert(c.device_id.clone(), c.device.public);
    ring.devices = c.trusted.iter().filter(|(id, public)| *id != target && active.iter().any(|d|
        d["id"] == **id && d["revoked_at"].is_null() && d["public_key"].as_str().and_then(|p| decode32(p).ok()).as_ref() == Some(*public)))
        .map(|(id, key)| (id.clone(), *key)).collect();
    ring.devices.insert(c.device_id.clone(), c.device.public);
    if ring.devices.len() > 64 { return Err("Remove unused devices before re-keying.".into()); }
    ring.epoch = epoch + 1;
    if !reset {
        let id_key = crypto::derive(
            &c.ak.as_ref().ok_or("Unlock sync first.")?.0,
            &c.account,
            "meeting-ids",
        );
        if !ring.meeting_id_keys.iter().any(|key| key.0 == id_key.0) {
            if ring.meeting_id_keys.len() >= 64 {
                return Err("Meeting ID history has reached its rotation limit.".into());
            }
            ring.meeting_id_keys.push(id_key);
        }
    }
    let ak = Secret::random();
    crate::sync_cutovers::publish(c, &ak, &mut ring).await?;
    let recovery = crypto::recovery_code().to_string();
    let transfer = KeyTransfer {
        ak: ak.clone(),
        ring_rev: rev + 1,
        vocabulary_rev: vocabulary_revision,
        epoch: epoch + 1,
        trust: Some(TrustedRoster { keyset: crypto::keyset_pin(&ring), devices: ring.devices.clone() }),
    };
    let plain = zeroize::Zeroizing::new(
        serde_json::to_vec(&RotationMessage { previous: previous_pin, transfer: KeyTransfer { ak: transfer.ak.clone(), ring_rev: transfer.ring_rev, vocabulary_rev: transfer.vocabulary_rev, epoch: transfer.epoch, trust: transfer.trust.clone() } }).map_err(|_| "Cannot wrap new keys.")?,
    );
    let devices=ring.devices.iter().map(|(id,public)|{
  let sealed=crypto::wrap_from(&c.device,public,&c.account,&format!("rekey/{id}/{}",epoch+1),&plain)?;
  let wrapped=RotationWrap{sender:c.device_id.clone(),epoch:epoch+1,ciphertext:B64.encode(sealed),introduction:old_wraps.iter().find(|w| w.kind == "roster" && &w.reference == id).map(|w| w.ciphertext.clone())};
  Ok(json!({"id":id,"ciphertext":B64.encode(serde_json::to_vec(&wrapped).map_err(|_|"Cannot encode re-key message.")?)}))
 }).collect::<Result<Vec<Value>,String>>()?;
    // Save the replacement recovery code first; retry/recovery survives an uncertain network result.
    c.recovery = Some(recovery.clone());
    c.trusted.insert(c.device_id.clone(), c.device.public);
    secret_store::save(store, c)?;
    let mut request = json!({"confirmed":true,"reset":reset,"epoch":epoch,"previous":rev,"device":target,"keyring":B64.encode(ring.seal(&ak,&c.account,rev+1)?),"recovery":B64.encode(crypto::wrap_recovery(&recovery,&c.account,&ak)?),"devices":devices,"vocabularyRevision":vocabulary_revision,"verifier":B64.encode(crypto::server_auth_verifier(&crypto::server_auth(&ak,&c.account)))});
    if let Some(cursor) = prune_cursor {
        request["pruneCursor"] = json!(cursor);
    }
    if let Some(checkpoint) = &ring.cutover_checkpoint {
        request["checkpoint"] = json!(checkpoint);
    }
    let _: Value = client
        .json(Method::POST, "/v1/rekey", Some(&request))
        .await?;
    c.keyset = Some(crypto::keyset_pin(&ring));
    c.ak = Some(ak);
    c.trusted = ring.devices;
    secret_store::save(store, c)?;
    if reset {
        crate::meeting_sync::reset(store, c)?;
        store.write(
            &store.root().join("sync-state.json"),
            &crate::sync_engine::Metadata {
                account: c.account.clone(),
                server: c.server.clone(),
                ..Default::default()
            },
        )?;
    }
    apply_checkpoint(store, c, &transfer)
}
async fn device_wrap(c: &Credentials) -> Result<(), String> {
    let ak = c.ak.as_ref().ok_or("Unlock this device first.")?;
    let client = c.client()?;
    let wraps = client.wraps().await?;
    let previous = wraps
        .iter()
        .find(|w| w.kind == "device" && w.reference == c.device_id)
        .map_or(0, |w| w.rev);
    client
        .put_wrap(
            "device",
            &c.device_id,
            B64.encode(crypto::wrap_device(
                &c.device.public,
                &c.account,
                &format!("device/{}", c.device_id),
                &ak.0,
            )?),
            previous,
        )
        .await?;
    Ok(())
}
fn accept_signin(c: &mut Credentials, reply: &Value) -> Result<String, String> {
    let old_device = c.device_id.clone();
    let pending = c.pending.as_ref().ok_or("Start sign-in again.")?;
    if text(reply, "device")? != pending.device_id {
        return Err("Sign-in device changed. Start again.".into());
    }
    let account = text(&reply["account"], "id")?;
    if c.account != account {
        c.ak = None;
        c.recovery = None;
        c.pairs.clear();
        c.trusted.clear();
        c.keyset = None;
    }
    c.device = pending.device.clone();
    c.device_id = pending.device_id.clone();
    c.account = account.into();
    c.email = text(&reply["account"], "email")?.into();
    c.session = text(reply, "token")?.into();
    c.initialized = reply["account"]["encrypted"].as_u64().map(|v| v != 0);
    c.pending = None;
    Ok(old_device)
}
pub async fn action(store: &Store, operation: &str, args: Value) -> Result<Value, String> {
    if operation == "status" {
        return status(store);
    }
    let mut c = secret_store::load::<Credentials>(store)?
        .unwrap_or_else(|| Credentials::new(STAGING.into()));
    c.pairs.retain(|_, pair| pair.expires_at > now());
    if operation.starts_with("meeting_") {
        return crate::meeting_sync::action(store, &mut c, operation, &args).await;
    }
    match operation {
        #[cfg(feature = "synctest")]
        "test_signin" => {
            if !c.session.is_empty() {
                return Err("Sign out of the test account first.".into());
            }
            let mut token = zeroize::Zeroizing::new(text(&args, "token")?.to_owned());
            if token.len() != 64 || !token.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("Invalid test token.".into());
            }
            let mut fresh = Credentials::new(STAGING.into());
            let reply: Value = fresh.client()?.with_test_token(std::mem::take(&mut *token))
                .json(Method::POST, "/v1/auth/test-session", Some(&json!({
                    "subject": text(&args, "subject")?, "device": fresh.device_id,
                    "publicKey": B64.encode(fresh.device.public), "platform": std::env::consts::OS
                }))).await?;
            if text(&reply, "device")? != fresh.device_id {
                return Err("Test device identity changed.".into());
            }
            fresh.account = text(&reply["account"], "id")?.into();
            fresh.email = text(&reply["account"], "email")?.into();
            fresh.session = text(&reply, "token")?.into();
            secret_store::save(store, &fresh)?;
            return status(store);
        }
        "now" => {
            let mut poll: Value = c.client()?.json(Method::GET, "/v1/poll", None).await?;
            cancel_observed_reset(store, &c, &mut poll).await?;
            if store.root().join("sync-notice.json").exists() {
                return Err("Review your local words and acknowledge the quarantined sync journal before syncing.".into());
            }
            if poll["refreshAfter"].as_u64().is_some_and(|t| t <= now()) {
                let _: Value = c
                    .client()?
                    .json(Method::POST, "/v1/heartbeat", None)
                    .await?;
            }
            poll["account"] = json!(c.account);
            poll["server"] = json!(c.server);
            let old: Value = store
                .read(&store.root().join("sync-remote.json"))
                .unwrap_or(Value::Null);
            if old != poll {
                store.write(&store.root().join("sync-remote.json"), &poll)?;
            }
            let cached: Value = store
                .read(&store.root().join("sync-keyring.json"))
                .unwrap_or(Value::Null);
            let (ring, rev) = if cached["account"] == c.account
                && cached["server"] == c.server
                && cached["wrap"]["rev"] == poll["ring_rev"]
            {
                let w: Wrap = serde_json::from_value(cached["wrap"].clone())
                    .map_err(|_| "Invalid cached keys.")?;
                let mut ring = Keyring::open(
                    c.ak.as_ref().ok_or("Unlock this device first.")?,
                    &c.account,
                    w.rev,
                    &B64.decode(&w.ciphertext)
                        .map_err(|_| "Invalid cached keys.")?,
                )?;
                check_keyset(&c, &ring)?;
                crate::sync_cutovers::hydrate(store, &c, &mut ring).await?;
                (ring, w.rev)
            } else {
                keyring(store, &mut c).await?
            };
            let meta = crate::sync_engine::synchronize(store, &c, &ring, rev).await?;
            let meetings = if args["automatic"] == true {
                crate::meeting_sync::scheduled(store, &c, &ring, rev).await
            } else {
                crate::meeting_sync::run(store, &c, &ring, rev, true).await
            };
            let meeting_result = match meetings {
                Ok(m) => crate::meeting_sync::outcome(&m),
                Err(_) => json!({"errors":1,"retryAt":0,"rateLimited":false,"missing":0}),
            };
            return Ok(
                json!({"synced":true,"lastSync":meta.last_sync,"conflicts":meta.conflicts.len(),"meetings":meeting_result}),
            );
        }
        "journal_ack" => {
            confirmed(&args)?;
            let path = store.root().join("sync-notice.json");
            if path.exists() {
                std::fs::remove_file(path).map_err(|_| "Cannot acknowledge sync notice.")?;
            }
        }
        "conflicts" => return Ok(json!(crate::sync_engine::metadata(store, &c)?.conflicts)),
        "use_local" => {
            confirmed(&args)?;
            crate::sync_engine::resolve(store, &c, text(&args, "id")?)?;
        }

        "signin" => {
            if let Some(server) = args["server"].as_str() {
                if !c.session.is_empty() && server != c.server {
                    return Err("Sign out before changing the sync server.".into());
                }
                SyncClient::new(server, "")?;
                c.server = server.into()
            }
            let provider = authkit_provider(&args)?;
            let signin_device = if c.ak.is_some() { c.device.clone() } else { DeviceKey::new() };
            let signin_id = if c.ak.is_some() { c.device_id.clone() } else { uuid::Uuid::new_v4().to_string() };
            let verifier = B64.encode(crypto::random::<32>());
            let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
            let reply:Value=c.client()?.json(Method::POST,"/v1/auth/start",Some(&json!({"provider":provider,"browserOrigin":AUTH_ORIGIN,"challenge":challenge,"device":signin_id,"publicKey":B64.encode(signin_device.public),"platform":std::env::consts::OS,"redirect":args["redirect"],"link":args["link"],"intent":if args["intent"] == "sign-up" {"sign-up"} else {"sign-in"}}))).await?;
            c.pending = Some(Pending {
                request: text(&reply, "id")?.into(),
                verifier,
                expires: now() + 300,
                device: signin_device,
                device_id: signin_id,
            });
            secret_store::save(store, &c)?;
            let url = text(&reply, "url")?;
            if args["noBrowser"] != true {
                open_browser(url, &c.server)?
            }
            return Ok(json!({"url":url}));
        }
        "callback" => {
            let u =
                reqwest::Url::parse(text(&args, "url")?).map_err(|_| "Invalid sign-in return.")?;
            if u.scheme() != "sonojot" || u.host_str() != Some("sync") || u.fragment().is_some() {
                return Err("Invalid sign-in return.".into());
            }
            let params = u.query_pairs().collect::<Vec<_>>();
            if params.len() != 2 {
                return Err("Invalid sign-in return.".into());
            }
            let get = |name: &str| {
                params
                    .iter()
                    .find(|(k, _)| k == name)
                    .map(|(_, v)| v.as_ref())
                    .ok_or("Invalid sign-in return.")
            };
            let pending = c
                .pending
                .as_ref()
                .ok_or("Start sign-in on this device first.")?;
            if pending.expires < now() || pending.request != get("request")? {
                return Err("Sign-in expired. Start again.".into());
            }
            let reply:Value=c.client()?.json(Method::POST,"/v1/auth/token",Some(&json!({"request":pending.request,"verifier":pending.verifier,"code":get("code")?}))).await?;
            let old_device = accept_signin(&mut c, &reply)?;

            secret_store::save(store, &c)?;
            if c.ak.is_some() && old_device != c.device_id {
                let (mut ring, rev) = keyring(store, &mut c).await?;
                ring.devices.remove(&old_device);
                ring.devices.insert(c.device_id.clone(), c.device.public);
                c.client()?
                    .put_wrap(
                        "keyring",
                        "main",
                        B64.encode(ring.seal(
                            c.ak.as_ref().ok_or("Missing key.")?,
                            &c.account,
                            rev + 1,
                        )?),
                        rev,
                    )
                    .await?;
                device_wrap(&c).await?;
                let _: Value = c
                    .client()?
                    .json(
                        Method::DELETE,
                        &format!("/v1/devices/{}", safe_id(&old_device)?),
                        Some(&json!({"confirmed":true})),
                    )
                    .await?;
            }
        }
        "setup" => {
            confirmed(&args)?;
            if c.session.is_empty() {
                return Err("Sign in first.".into());
            }
            let wraps = c.client()?.wraps().await?;
            if wraps.iter().any(|w| w.kind == "keyring") {
                if c.ak.is_some() && c.recovery.is_some() {
                    keyring(store, &mut c).await?;
                    save_recovery(&c, &wraps).await?;
                    device_wrap(&c).await?;
                    return status(store);
                }
                return Err("This account already has an encryption key. This computer gets it from your other computer, or use your recovery code.".into());
            }
            if c.ak.is_none() {
                c.ak = Some(Secret::random())
            }
            if c.recovery.is_none() {
                c.recovery = Some(crypto::recovery_code().to_string())
            }
            secret_store::save(store, &c)?;
            let mut ring = Keyring::new();
            ring.devices.insert(c.device_id.clone(), c.device.public);
            c.keyset = Some(crypto::keyset_pin(&ring));
            secret_store::save(store, &c)?;
            let ak = c.ak.as_ref().ok_or("Missing encryption key.")?;
            let _: Value = c.client()?.json(Method::POST,"/v1/bootstrap",Some(&json!({
                "keyring":B64.encode(ring.seal(ak,&c.account,1)?),
                "recovery":B64.encode(crypto::wrap_recovery(c.recovery.as_ref().ok_or("Missing recovery code.")?,&c.account,ak)?),
                "device":B64.encode(crypto::wrap_device(&c.device.public,&c.account,&format!("device/{}",c.device_id),&ak.0)?)
            }))).await?;
            c.keyset = Some(crypto::keyset_pin(&ring));
            c.trusted = ring.devices;
            secret_store::save(store, &c)?;
        }
        "recovery_show" => {
            confirmed(&args)?;
            return Ok(
                json!({"code":c.recovery.as_ref().ok_or("Create a new recovery code to replace the previous one.")?}),
            );
        }
        "recovery_ack" => {
            confirmed(&args)?;
            c.recovery = None;
            secret_store::save(store, &c)?;
        }
        "recovery_new" => {
            confirmed(&args)?;
            if c.ak.is_none() {
                return Err("Unlock this device first.".into());
            }
            let wraps = c.client()?.wraps().await?;
            c.recovery = Some(crypto::recovery_code().to_string());
            secret_store::save(store, &c)?;
            save_recovery(&c, &wraps).await?;
            return Ok(json!({"code":c.recovery}));
        }
        "recovery_use" => {
            let wraps = c.client()?.wraps().await?;
            let w = wraps
                .iter()
                .find(|w| w.kind == "recovery")
                .ok_or("No recovery wrap is available.")?;
            c.ak = Some(crypto::unwrap_recovery(
                text(&args, "code")?,
                &c.account,
                &B64.decode(&w.ciphertext)
                    .map_err(|_| "Invalid recovery wrap.")?,
            )?);
            let w = wraps
                .iter()
                .find(|w| w.kind == "keyring")
                .ok_or("Missing keyring.")?;
            let mut ring = Keyring::open(
                c.ak.as_ref().ok_or("Missing recovery key.")?,
                &c.account,
                w.rev,
                &B64.decode(&w.ciphertext).map_err(|_| "Invalid keyring.")?,
            )?;
            let rev = w.rev;
            c.keyset = Some(crypto::keyset_pin(&ring));
            c.trusted = ring.devices.clone();
            ring.devices.insert(c.device_id.clone(), c.device.public);
            c.client()?
                .put_wrap(
                    "keyring",
                    "main",
                    B64.encode(ring.seal(c.ak.as_ref().unwrap(), &c.account, rev + 1)?),
                    rev,
                )
                .await?;
            secret_store::save(store, &c)?;
            cancel_reset(store, &c).await?;
            device_wrap(&c).await?;
            // The waiting request is no longer needed; withdraw it so no other
            // computer relays a key this one already has.
            withdraw_requests(store, &mut c).await;
            let _ = baseline_joins(store, &c).await;
        }
        "remote_status" => {
            let mut poll: Value = c.client()?.json(Method::GET, "/v1/poll", None).await?;
            cancel_observed_reset(store, &c, &mut poll).await?;
            poll["account"] = json!(c.account);
            poll["server"] = json!(c.server);
            let old: Value = store
                .read(&store.root().join("sync-remote.json"))
                .unwrap_or(Value::Null);
            if old != poll {
                store.write(&store.root().join("sync-remote.json"), &poll)?;
            }
        }
        "reset_request" => {
            confirmed(&args)?;
            let mut pending: Value = c
                .client()?
                .json(Method::POST, "/v1/reset", Some(&json!({"confirmed":true})))
                .await?;
            pending["account"] = json!(c.account);
            pending["server"] = json!(c.server);
            store.write(&store.root().join("sync-remote.json"), &pending)?;
        }
        "reset_cancel" => {
            cancel_reset(store, &c).await?;
        }
        "reset" => {
            confirmed(&args)?;
            let id = c.device_id.clone();
            rekey(store, &mut c, &id, true).await?;
        }
        "devices" => return c.client()?.json(Method::GET, "/v1/devices", None).await,
        "remove" => {
            confirmed(&args)?;
            let target = text(&args, "device")?;
            if removal_rekeys(&args, target, &c.device_id)? {
                rekey(store, &mut c, target, false).await?;
                ack_join(store, &c, target)?;
                return status(store);
            }
            let _: Value = c
                .client()?
                .json(
                    Method::DELETE,
                    &format!("/v1/devices/{}", safe_id(target)?),
                    Some(&json!({"confirmed":true})),
                )
                .await?;
            if target == c.device_id {
                secret_store::delete(store)?;
                forget_joins(store);
                return status(store);
            }
            ack_join(store, &c, target)?;
        }
        "pair_start" => {
            if c.ak.is_some() {
                return Err("This computer already has the key.".into());
            }
            let pair = pair_start(store, &mut c).await?;
            return Ok(json!({"id":pair.id,"expiresAt":pair.expires_at}));
        }
        "pair_wait" => {
            // The new computer drives its side on its own: start a request,
            // reveal when an approver commits, and import the sealed key.
            if c.ak.is_some() {
                return Ok(json!({"unlocked":true}));
            }
            if c.session.is_empty() {
                return Err("Sign in first.".into());
            }
            let Some(id) = c.pairs.values().find(|p| !p.approver).map(|p| p.id.clone()) else {
                pair_start(store, &mut c).await?;
                return Ok(json!({"waiting":true,"advanced":true}));
            };
            let result = pair_step(store, &mut c, &id).await?;
            if result["expired"] == true || result["denied"] == true {
                // The next call starts a fresh request.
                return Ok(json!({"waiting":true,"advanced":true}));
            }
            if c.pairs.get(&id).is_some_and(|p| {
                let limit = if p.state == "ready" { PAIR_READY_STALL } else { PAIR_STALL };
                p.state != "committed" && p.progress_at.saturating_add(limit) < now()
            }) {
                // Another computer claimed the request and went quiet, or an
                // older app is waiting for a person. Replace it so any other
                // computer can relay a fresh one.
                withdraw_requests(store, &mut c).await;
                return Ok(json!({"waiting":true,"advanced":false}));
            }
            if result["unlocked"] == true {
                let _ = baseline_joins(store, &c).await;
            }
            return Ok(result);
        }
        "pair_relay" => {
            // An unlocked computer answers requests from the person's other
            // computers. The server lets exactly one approver claim each
            // request; a computer that loses that race moves on quietly.
            if c.ak.is_none() || c.session.is_empty() {
                return Ok(json!({"advanced":false,"approved":0}));
            }
            let requests: Vec<Value> = c.client()?.json(Method::GET, "/v1/pairings", None).await?;
            let before = c.pairs.len();
            c.pairs.retain(|_, p| !p.approver || p.progress_at.saturating_add(PAIR_STALL) > now());
            if c.pairs.len() != before {
                secret_store::save(store, &c)?;
            }
            let (mut advanced, mut approved, mut failure) = (false, 0, None);
            for request in &requests {
                let Some(id) = request["id"].as_str().and_then(|id| safe_id(id).ok()) else {
                    continue;
                };
                if request["device"] == c.device_id.as_str() {
                    continue;
                }
                let claimed = request["approver"] == c.device_id.as_str()
                    && c.pairs.get(id).is_some_and(|p| p.approver);
                if request["state"] == "committed" && !c.pairs.contains_key(id) {
                    if pair_accept(store, &mut c, id, request).await.is_err() {
                        continue;
                    }
                    advanced = true;
                } else if !claimed {
                    continue;
                }
                match pair_step(store, &mut c, id).await {
                    Ok(step) => {
                        if step["approved"] == true {
                            approved += 1;
                        }
                        advanced |= step["advanced"] == true || step["approved"] == true;
                    }
                    Err(error) => failure = failure.or(Some(error)),
                }
            }
            if let (Some(error), false) = (failure, advanced) {
                return Err(error);
            }
            return Ok(json!({"advanced":advanced,"approved":approved}));
        }
        "join_check" => {
            if c.ak.is_none() || c.session.is_empty() {
                return Ok(json!({"joined":[]}));
            }
            let devices: Vec<Value> = c.client()?.json(Method::GET, "/v1/devices", None).await?;
            return Ok(json!({"joined":record_joins(store, &c, &devices)?}));
        }
        "join_ack" => {
            ack_join(store, &c, text(&args, "device")?)?;
        }
        "pair_list" => return c.client()?.json(Method::GET, "/v1/pairings", None).await,
        "pair_accept" => {
            if c.ak.is_none() {
                return Err("This computer needs the key before it can add another.".into());
            }
            let id = safe_id(text(&args, "id")?)?;
            if let Some(pair) = c.pairs.get(id).filter(|p| p.approver) {
                return Ok(json!({"waiting":true,"expiresAt":pair.expires_at}));
            }
            let p: Value = c
                .client()?
                .json(Method::GET, &format!("/v1/pairings/{id}"), None)
                .await?;
            pair_accept(store, &mut c, id, &p).await?;
            return Ok(json!({"waiting":true}));
        }
        "pair_poll" => {
            let id = safe_id(text(&args, "id")?)?.to_owned();
            return pair_step(store, &mut c, &id).await;
        }
        "pair_deny" => {
            let id = safe_id(text(&args, "id")?)?;
            let _: Value = c
                .client()?
                .json(Method::POST, &format!("/v1/pairings/{id}"), Some(&json!({"action":"deny"})))
                .await?;
            c.pairs.remove(id);
            secret_store::save(store, &c)?;
            return Ok(json!({"denied":true}));
        }
        "signout" => {
            withdraw_requests(store, &mut c).await;
            let result = c
                .client()?
                .json::<Value>(Method::POST, "/v1/auth/signout", Some(&json!({})))
                .await;
            secret_store::delete(store)?;
            forget_joins(store);
            if result.is_err() {
                return Ok(
                    json!({"signedOut":true,"notice":"Local keys removed. Remove this device from your account to revoke its server session while offline."}),
                );
            }
            let reply = result.map_err(|_| "Sign-out failed.".to_string())?;
            if args["noBrowser"] != true {
                if let Some(url) = reply["url"].as_str() {
                    open_browser(url, &c.server)?;
                }
            }
            return Ok(reply);
        }
        "recovery_download" => {
            confirmed(&args)?;
            let code = c
                .recovery
                .as_ref()
                .ok_or("Create or show a recovery code first.")?;
            export_file(store,text(&args,"path")?,format!("SonoJot recovery code\n{code}\nKeep this with your password manager. Anyone with this code and your sign-in can unlock your sync.\n").as_bytes())?;
            return Ok(json!({"saved":true}));
        }
        "account_export_file" => {
            let value: Value = c
                .client()?
                .json(Method::GET, "/v1/account/export", None)
                .await?;
            export_file(
                store,
                text(&args, "path")?,
                &serde_json::to_vec_pretty(&value).map_err(|_| "Cannot export account data.")?,
            )?;
            return Ok(json!({"saved":true}));
        }
        "account_export" => {
            return c
                .client()?
                .json(Method::GET, "/v1/account/export", None)
                .await
        }
        "account_delete" => {
            confirmed(&args)?;
            let _: Value = c
                .client()?
                .json(
                    Method::DELETE,
                    "/v1/account",
                    Some(&json!({"confirmed":true})),
                )
                .await?;
            secret_store::delete(store)?;
            forget_joins(store);
        }
        _ => return Err("Unknown sync action.".into()),
    }
    status(store)
}
/// Starts this computer's request, or resumes the one it already has.
/// Reopening Settings or restarting keeps the same commitment and nonce.
async fn pair_start(store: &Store, c: &mut Credentials) -> Result<PairState, String> {
    if let Some(pair) = c.pairs.values().find(|p| !p.approver) {
        return Ok(pair.clone());
    }
    let id = uuid::Uuid::new_v4().to_string();
    let transcript = crypto::PairTranscript {
        account: c.account.clone(),
        request: id.clone(),
        receiver_id: c.device_id.clone(),
        receiver: c.device.public,
        sender_id: String::new(),
        sender: [0; 32],
        nonce_a: [0; 32],
        nonce_b: crypto::random(),
    };
    let commit = transcript.receiver_commit();
    let reply: Value = c
        .client()?
        .json(
            Method::POST,
            "/v1/pairings",
            Some(&json!({"id":id,"commitment":B64.encode(commit)})),
        )
        .await?;
    let pair = PairState {
        id: id.clone(),
        nonce: transcript.nonce_b,
        public: c.device.public,
        commitment: commit,
        approver: false,
        peer_nonce: None,
        transcript: Some(transcript),
        expires_at: reply["expiresAt"]
            .as_u64()
            .unwrap_or(now() + PAIR_LIFETIME)
            .min(now() + PAIR_LIFETIME),
        state: "committed".into(),
        progress_at: now(),
    };
    c.pairs.clear();
    c.pairs.insert(id, pair.clone());
    secret_store::save(store, c)?;
    Ok(pair)
}
/// Claims a committed request from another computer of this account. The
/// server accepts only the first claim; the caller treats an error as "another
/// computer has it".
async fn pair_accept(store: &Store, c: &mut Credentials, id: &str, p: &Value) -> Result<(), String> {
    if p["state"] != "committed" || p["device"] == c.device_id.as_str() {
        return Err("Another computer is already handling this request.".into());
    }
    let transcript = crypto::PairTranscript {
        account: c.account.clone(),
        request: id.into(),
        receiver_id: text(p, "device")?.into(),
        receiver: decode32(text(p, "public_key")?)?,
        sender_id: c.device_id.clone(),
        sender: c.device.public,
        nonce_a: crypto::random(),
        nonce_b: [0; 32],
    };
    let commitment = decode32(text(p, "commitment")?)?;
    let expires_at = p["expires_at"].as_u64().ok_or("Missing pairing expiry.")?.min(now() + PAIR_LIFETIME);
    let _: Value = c
        .client()?
        .json(
            Method::POST,
            &format!("/v1/pairings/{id}"),
            Some(&json!({"action":"accept","commitment":B64.encode(transcript.sender_commit())})),
        )
        .await?;
    c.pairs.insert(
        id.into(),
        PairState {
            id: id.into(),
            nonce: transcript.nonce_a,
            public: transcript.receiver,
            commitment,
            approver: true,
            peer_nonce: Some(transcript.sender_commit()),
            transcript: Some(transcript),
            expires_at,
            state: "accepted".into(),
            progress_at: now(),
        },
    );
    secret_store::save(store, c)
}
/// Advances one request as far as the server state allows. Both computers
/// call it on their own. Every commit-then-reveal, identity and nonce check
/// of the manual flow still runs; only the person comparing digits is gone.
async fn pair_step(store: &Store, c: &mut Credentials, id: &str) -> Result<Value, String> {
    let path = format!("/v1/pairings/{id}");
    let Some(mut local) = c.pairs.get(id).cloned() else {
        return Ok(json!({"expired":true}));
    };
    let p: Value = match c.client()?.json_status(Method::GET, &path, None).await {
        Ok(p) => p,
        Err(e) if e.status == 404 => {
            c.pairs.remove(id);
            secret_store::save(store, c)?;
            return Ok(json!({"expired":true}));
        }
        Err(e) => return Err(e.message),
    };
    let state = text(&p, "state")?.to_owned();
    if state == "denied" || state == "consumed" {
        c.pairs.remove(id);
        secret_store::save(store, c)?;
        return Ok(json!({"denied":true}));
    }
    let mut t = local
        .transcript
        .clone()
        .ok_or("Start pairing again with the updated app.")?;
    if text(&p, "device")? != t.receiver_id
        || decode32(text(&p, "public_key")?)? != t.receiver
        || decode32(text(&p, "commitment")?)? != local.commitment
        || (local.approver && p["approver"] != c.device_id.as_str())
    {
        return Err("Pairing identity changed. Start again.".into());
    }
    let advanced = state != local.state;
    if advanced {
        local.state = state.clone();
        local.progress_at = now();
        c.pairs.insert(id.into(), local.clone());
        secret_store::save(store, c)?;
    }
    let waiting = |advanced: bool| -> Result<Value, String> {
        Ok(json!({"waiting":true,"advanced":advanced}))
    };
    if state == "committed" {
        return waiting(advanced);
    }
    if !local.approver && local.peer_nonce.is_none() {
        // Pin the approver's commitment BEFORE revealing our nonce.
        if state != "accepted" {
            return Err("Pairing skipped its commitment. Start again.".into());
        }
        local.peer_nonce = Some(decode32(text(&p, "commitment_a")?)?);
        c.pairs.insert(id.into(), local.clone());
        secret_store::save(store, c)?;
    }
    if local.peer_nonce != Some(decode32(text(&p, "commitment_a")?)?) {
        return Err("Approver commitment changed.".into());
    }
    if state == "accepted" {
        if local.approver {
            return waiting(advanced);
        }
        let _: Value = c
            .client()?
            .json(
                Method::POST,
                &path,
                Some(&json!({"action":"reveal","nonce":B64.encode(t.nonce_b)})),
            )
            .await?;
        return waiting(true);
    }
    let b = decode32(text(&p, "nonce_b")?)?;
    if !local.approver && b != t.nonce_b {
        return Err("Pairing nonce changed.".into());
    }
    t.nonce_b = b;
    if t.receiver_commit() != local.commitment {
        return Err("Recipient commitment changed.".into());
    }
    if state == "revealed" {
        if !local.approver {
            return waiting(advanced);
        }
        let _: Value = c
            .client()?
            .json(
                Method::POST,
                &path,
                Some(&json!({"action":"reveal_approver","nonce":B64.encode(t.nonce_a)})),
            )
            .await?;
        return waiting(true);
    }
    if !local.approver {
        t.sender = decode32(text(&p, "public_key_a")?)?;
        t.sender_id = text(&p, "approver")?.into();
        t.nonce_a = decode32(text(&p, "nonce_a")?)?;
    }
    if t.sender_commit() != local.peer_nonce.ok_or("Missing approver commitment.")? {
        return Err("Approver identity changed.".into());
    }
    if state != "ready" && state != "approved" {
        return Err("Pairing is no longer available.".into());
    }
    if local.approver {
        if state != "ready" {
            return waiting(advanced);
        }
        approve_pair(store, c, id, &t).await?;
        return Ok(json!({"approved":true,"advanced":true}));
    }
    if state != "approved" {
        return waiting(advanced);
    }
    let ciphertext = B64
        .decode(text(&p, "ciphertext")?)
        .map_err(|_| "Invalid wrapped keys.")?;
    import_pair(store, c, id, &t, &ciphertext).await?;
    Ok(json!({"unlocked":true}))
}
/// Seals the account key to the new computer and adds it to the roster,
/// exactly as a manual approval did.
async fn approve_pair(
    store: &Store,
    c: &mut Credentials,
    id: &str,
    t: &crypto::PairTranscript,
) -> Result<(), String> {
    let (mut ring, rev) = keyring(store, c).await?;
    let mut trusted = c.trusted.clone();
    trusted.insert(c.device_id.clone(), c.device.public);
    trusted.insert(t.receiver_id.clone(), t.receiver);
    ring.devices = trusted.clone();
    let ak = c.ak.clone().ok_or("Unlock this device first.")?;
    c.client()?
        .put_wrap(
            "keyring",
            "main",
            B64.encode(ring.seal(&ak, &c.account, rev + 1)?),
            rev,
        )
        .await?;
    let meta = crate::sync_engine::metadata(store, c)?;
    let transfer = KeyTransfer {
        ak: ak.clone(),
        ring_rev: rev + 1,
        vocabulary_rev: meta.rev,
        epoch: ring.epoch,
        trust: Some(TrustedRoster { keyset: crypto::keyset_pin(&ring), devices: trusted.clone() }),
    };
    let plain = zeroize::Zeroizing::new(
        serde_json::to_vec(&transfer).map_err(|_| "Cannot encode pairing keys.")?,
    );
    let wrap = crypto::wrap_from(&c.device, &t.receiver, &c.account, &t.binding(), &plain)?;
    // Older servers still require the explicit confirmation flag.
    let _: Value = c
        .client()?
        .json(
            Method::POST,
            &format!("/v1/pairings/{id}"),
            Some(&json!({"action":"approve","confirmed":true,"ciphertext":B64.encode(wrap)})),
        )
        .await?;
    c.trusted = trusted;
    c.pairs.remove(id);
    secret_store::save(store, c)?;
    publish_roster(c).await
}
/// Imports the sealed key on the new computer after checking that it came
/// from the committed approver and that the keyring names both computers.
async fn import_pair(
    store: &Store,
    c: &mut Credentials,
    id: &str,
    t: &crypto::PairTranscript,
    ciphertext: &[u8],
) -> Result<(), String> {
    let transfer = authenticated_transfer(&c.device, t, ciphertext)?;
    let wraps = c.client()?.wraps().await?;
    let w = wraps
        .iter()
        .find(|w| w.kind == "keyring")
        .ok_or("Missing paired keyring.")?;
    if w.rev < transfer.ring_rev {
        return Err("The service returned older keys.".into());
    }
    let ring = Keyring::open(
        &transfer.ak,
        &c.account,
        w.rev,
        &B64.decode(&w.ciphertext).map_err(|_| "Invalid keyring.")?,
    )?;
    if ring.epoch != transfer.epoch
        || ring.devices.get(&t.sender_id) != Some(&t.sender)
        || ring.devices.get(&t.receiver_id) != Some(&t.receiver)
    {
        return Err("Paired keyring identity changed.".into());
    }
    if let Some(roster) = &transfer.trust {
        if roster.keyset != crypto::keyset_pin(&ring)
            || roster.devices.len() > 64
            || roster.devices.get(&t.sender_id) != Some(&t.sender)
            || roster.devices.get(&t.receiver_id) != Some(&t.receiver)
        {
            return Err(REPAIR_TRUST.into());
        }
    }
    c.ak = Some(transfer.ak.clone());
    c.keyset = Some(crypto::keyset_pin(&ring));
    c.trusted = [
        (t.sender_id.clone(), t.sender),
        (t.receiver_id.clone(), t.receiver),
    ]
    .into();
    if let Some(roster) = &transfer.trust {
        c.trusted = roster.devices.clone();
    }
    apply_checkpoint(store, c, &transfer)?;
    secret_store::save(store, c)?;
    cancel_reset(store, c).await?;
    device_wrap(c).await?;
    let _: Value = c
        .client()?
        .json(
            Method::POST,
            &format!("/v1/pairings/{id}"),
            Some(&json!({"action":"consume"})),
        )
        .await?;
    c.pairs.remove(id);
    secret_store::save(store, c)
}
/// Withdraws this computer's own waiting requests, best effort.
async fn withdraw_requests(store: &Store, c: &mut Credentials) {
    let ids: Vec<String> = c.pairs.values().filter(|p| !p.approver).map(|p| p.id.clone()).collect();
    if ids.is_empty() {
        return;
    }
    if let Ok(client) = c.client() {
        for id in &ids {
            let _ = client
                .json::<Value>(Method::POST, &format!("/v1/pairings/{id}"), Some(&json!({"action":"deny"})))
                .await;
        }
    }
    for id in &ids {
        c.pairs.remove(id);
    }
    let _ = secret_store::save(store, c);
}

/// A computer of this account that other computers have not acknowledged yet.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Joined {
    id: String,
    platform: String,
    joined_at: u64,
}
#[derive(Default, Serialize, Deserialize)]
struct Joins {
    account: String,
    server: String,
    #[serde(default)]
    seen: std::collections::BTreeSet<String>,
    #[serde(default)]
    pending: Vec<Joined>,
}
fn load_joins(store: &Store, c: &Credentials) -> Option<Joins> {
    store
        .read::<Joins>(&store.root().join(JOINS))
        .ok()
        .filter(|j| j.account == c.account && j.server == c.server)
}
/// Signed-in, unrevoked computers other than this one. A computer counts as
/// joined from sign-in: with automatic relay it receives the key moments later.
fn other_devices(c: &Credentials, devices: &[Value]) -> Vec<Joined> {
    devices
        .iter()
        .filter(|d| {
            d["revoked_at"].is_null()
                && d["public_key"].as_str().is_some_and(|k| !k.is_empty())
                && d["id"].as_str().is_some_and(|id| id != c.device_id)
        })
        .map(|d| Joined {
            id: d["id"].as_str().unwrap_or_default().into(),
            platform: d["platform"].as_str().unwrap_or_default().chars().take(40).collect(),
            joined_at: d["created_at"].as_u64().unwrap_or(0),
        })
        .collect()
}
/// Adds computers this one has not seen before to the pending alerts and
/// returns them. The first look on a computer only records what is already
/// there, so upgrading or joining never alerts about older computers.
fn record_joins(store: &Store, c: &Credentials, devices: &[Value]) -> Result<Vec<Joined>, String> {
    if !devices.iter().any(|d| d["id"] == c.device_id.as_str()) {
        return Ok(Vec::new());
    }
    let active = other_devices(c, devices);
    let path = store.root().join(JOINS);
    let Some(mut joins) = load_joins(store, c) else {
        store.write(
            &path,
            &Joins {
                account: c.account.clone(),
                server: c.server.clone(),
                seen: active.iter().map(|j| j.id.clone()).collect(),
                pending: Vec::new(),
            },
        )?;
        return Ok(Vec::new());
    };
    let before = (joins.seen.len(), joins.pending.len());
    joins.seen.retain(|id| devices.iter().any(|d| d["id"] == id.as_str()));
    joins.pending.retain(|j| active.iter().any(|a| a.id == j.id));
    let fresh: Vec<Joined> = active.into_iter().filter(|j| !joins.seen.contains(&j.id)).collect();
    for j in &fresh {
        joins.seen.insert(j.id.clone());
        joins.pending.push(j.clone());
    }
    if !fresh.is_empty() || before != (joins.seen.len(), joins.pending.len()) {
        store.write(&path, &joins)?;
    }
    Ok(fresh)
}
/// After this computer gets the key, everything already in the account
/// predates it; only later computers raise an alert here.
async fn baseline_joins(store: &Store, c: &Credentials) -> Result<(), String> {
    let devices: Vec<Value> = c.client()?.json(Method::GET, "/v1/devices", None).await?;
    forget_joins(store);
    record_joins(store, c, &devices).map(|_| ())
}
fn ack_join(store: &Store, c: &Credentials, device: &str) -> Result<(), String> {
    let Some(mut joins) = load_joins(store, c) else {
        return Ok(());
    };
    joins.pending.retain(|j| j.id != device);
    joins.seen.insert(device.into());
    store.write(&store.root().join(JOINS), &joins)
}
fn pending_joins(store: &Store, c: &Credentials) -> Vec<Joined> {
    load_joins(store, c).map(|j| j.pending).unwrap_or_default()
}
fn forget_joins(store: &Store) {
    let _ = std::fs::remove_file(store.root().join(JOINS));
}
fn platform_name(platform: &str) -> &'static str {
    match platform {
        "macos" => "Mac",
        "windows" => "Windows PC",
        "linux" => "Linux computer",
        "ios" => "iPhone",
        _ => "A new computer",
    }
}
async fn save_recovery(c: &Credentials, wraps: &[Wrap]) -> Result<(), String> {
    let code = c.recovery.as_ref().ok_or("Create a recovery code first.")?;
    let ak = c.ak.as_ref().ok_or("Unlock this device first.")?;
    let prev = wraps
        .iter()
        .find(|w| w.kind == "recovery")
        .map_or(0, |w| w.rev);
    c.client()?
        .put_wrap(
            "recovery",
            "main",
            B64.encode(crypto::wrap_recovery(code, &c.account, ak)?),
            prev,
        )
        .await?;
    Ok(())
}
fn safe_id(id: &str) -> Result<&str, String> {
    if id.len() > 128
        || id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err("Invalid device or pairing id.".into());
    }
    Ok(id)
}
fn authkit_provider(args: &Value) -> Result<&str, String> {
    let provider = args["provider"].as_str().unwrap_or("authkit");
    if provider != "authkit" && provider != "email" {
        return Err("Use email sign-in.".into());
    }
    Ok("authkit")
}
fn browser_url(value: &str, server: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid sign-in URL.")?;
    let origin = reqwest::Url::parse(server).map_err(|_| "Invalid sync server.")?;
    let auth_origin = !AUTH_ORIGIN.is_empty() && server == STAGING
        && reqwest::Url::parse(AUTH_ORIGIN).is_ok_and(|auth| auth.scheme() == "https" && auth.origin() == url.origin());
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !(url.origin() == origin.origin() || auth_origin)
    {
        return Err("Unexpected sign-in address.".into());
    }
    Ok(url)
}
fn open_browser(value: &str, server: &str) -> Result<(), String> {
    let url = browser_url(value, server)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let exe = std::path::PathBuf::from(
            std::env::var_os("SystemRoot").ok_or("Cannot find the browser opener.")?,
        )
        .join("System32/rundll32.exe");
        std::process::Command::new(exe)
            .arg("url.dll,FileProtocolHandler")
            .arg(url.as_str())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "Cannot open the browser.")?;
    }
    #[cfg(not(windows))]
    {
        std::process::Command::new(if cfg!(target_os = "macos") {
            "/usr/bin/open"
        } else {
            "xdg-open"
        })
        .arg(url.as_str())
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|_| "Cannot open the browser.")?;
    }
    Ok(())
}
#[tauri::command]
pub async fn sync_command(
    app: tauri::AppHandle,
    operation: String,
    args: Option<Value>,
) -> Result<Value, String> {
    crate::flavor::require_sync()?;
    let runtime = app.state::<Arc<Runtime>>();
    let service = app.state::<SyncService>();
    let _guard = service.gate.lock().await;
    #[allow(unused_mut)]
    let mut args = args.unwrap_or(json!({}));
    #[cfg(all(target_os = "macos", feature = "synctest"))]
    if operation == "test_credentials" {
        let phase = args["phase"].as_str().unwrap_or("roundtrip");
        secret_store::probe(&runtime.store, phase)?;
        return Ok(
            json!({"credentialStore":"dedicated-test-keychain","phase":phase,"passed":true,"probeDeleted":phase != "prepare"}),
        );
    }
    #[cfg(feature = "synctest")]
    if operation == "test_offline" {
        let offline = args["offline"]
            .as_bool()
            .ok_or("Choose offline on or off.")?;
        crate::sync_client::TEST_OFFLINE.store(offline, std::sync::atomic::Ordering::Relaxed);
        if !offline {
            wake();
        }
        return Ok(json!({"offline":offline}));
    }
    #[cfg(any(target_os = "linux", feature = "synctest"))]
    if operation == "signin" {
        args["redirect"] = json!(loopback(&app).await?);
    }

    let mut result = if operation == "recovery_copy" {
        confirmed(&args)?;
        let c = secret_store::load::<Credentials>(&runtime.store)?.ok_or("Sign in first.")?;
        let code = c
            .recovery
            .as_ref()
            .ok_or("Create or show a recovery code first.")?;
        crate::platform::copy_secret_after_claim(code, || runtime.claim_manual_copy(None))
            .await
            .map(|_| json!({"copied":true}))
    } else if operation == "recovery_print" {
        confirmed(&args)?;
        app.get_webview_window("main")
            .ok_or("Open the recovery screen before printing.")?
            .print()
            .map(|_| json!({"printing":true}))
            .map_err(|_| "Cannot open the print dialog.".to_string())
    } else {
        action(&runtime.store, &operation, args).await
    };
    // Delivery is independent of upload success. Keep the in-app notice if the OS suppresses it.
    let _ = deliver_reset_notice(&runtime.store, |title, body| {
        use tauri_plugin_notification::NotificationExt;
        app.notification()
            .builder()
            .title(title)
            .body(body)
            .show()
            .map_err(|_| "System notification is unavailable.".to_string())
    });
    if operation == "status" {
        if let Ok(value) = &mut result {
            if let Ok(failure) = service.failure.lock() {
                value["error"] = json!(*failure);
            }
        }
    }
    if !matches!(
        operation.as_str(),
        "status"
            | "devices"
            | "pair_list"
            | "conflicts"
            | "recovery_show"
            | "meeting_status"
            | "meeting_storage"
            | "meeting_preferences"
            | "pair_wait"
            | "pair_relay"
            | "join_check"
    ) {
        if let Ok(mut failure) = service.failure.lock() {
            *failure = result.as_ref().err().cloned()
        }
        let _ = app.emit("sync-changed", ());
    } else if let (Ok(value), true) = (
        &result,
        matches!(operation.as_str(), "pair_wait" | "pair_relay" | "join_check"),
    ) {
        // Pairing polls stay quiet unless they changed something worth showing.
        if pairing_changed(value) {
            let _ = app.emit("sync-changed", ());
            if value["unlocked"] == true {
                wake();
            }
        }
    }
    if result.is_ok()
        && (operation == "now"
            || matches!(
                operation.as_str(),
                "meeting_resolve"
                    | "meeting_field_resolve"
                    | "meeting_delete_resolve"
                    | "meeting_missing_resolve"
                    | "meeting_download"
            ))
    {
        runtime.meetings.sync_changed();
        runtime.emit(&app);
    }
    if result.is_ok()
        && matches!(
            operation.as_str(),
            "meeting_resolve"
                | "meeting_field_resolve"
                | "meeting_audio"
                | "meeting_preferences"
                | "meeting_delete_resolve"
                | "meeting_missing_resolve"
        )
    {
        wake();
    }
    result
}
pub fn callback(app: &tauri::AppHandle, value: String) {
    if !crate::flavor::sync_enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let _ = sync_command(app, "callback".into(), Some(json!({"url":value}))).await;
    });
}

fn export_file(store: &Store, path: &str, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let path = std::path::Path::new(path);
    store.check_export_destination(path)?;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| "Choose a new writable export file.")?;
    file.write_all(bytes)
        .and_then(|_| file.sync_all())
        .map_err(|_| "Could not finish the export.".into())
}

#[cfg(any(target_os = "linux", feature = "synctest"))]
async fn loopback(app: &tauri::AppHandle) -> Result<String, String> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .await
        .map_err(|_| "Cannot open the local sign-in return.")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot read the local sign-in port.")?
        .port();
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Ok(Ok((mut socket, _))) =
            tokio::time::timeout(std::time::Duration::from_secs(600), listener.accept()).await
        {
            let mut buffer = [0u8; 4096];
            if let Ok(Ok(n)) =
                tokio::time::timeout(std::time::Duration::from_secs(10), socket.read(&mut buffer))
                    .await
            {
                let first = std::str::from_utf8(&buffer[..n])
                    .unwrap_or("")
                    .lines()
                    .next()
                    .unwrap_or("");
                if let Some(target) = first
                    .strip_prefix("GET ")
                    .and_then(|s| s.strip_suffix(" HTTP/1.1"))
                    .filter(|s| s.starts_with("/sync?"))
                {
                    callback(
                        &app,
                        format!("sonojot://sync?{}", target.trim_start_matches("/sync?")),
                    );
                    let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: no-store\r\nConnection: close\r\n\r\nReturn to SonoJot to finish signing in.").await;
                }
            }
        }
    });
    Ok(format!("http://127.0.0.1:{port}/sync"))
}

static WAKE: std::sync::OnceLock<tokio::sync::Notify> = std::sync::OnceLock::new();
pub fn wake() {
    if !crate::flavor::sync_enabled() {
        return;
    }
    WAKE.get_or_init(tokio::sync::Notify::new).notify_one();
}
pub fn start_scheduler(app: &tauri::AppHandle) {
    if !crate::flavor::sync_enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let mut delay = 300u64;
        loop {
            if tokio::time::timeout(
                std::time::Duration::from_secs(delay),
                WAKE.get_or_init(tokio::sync::Notify::new).notified(),
            )
            .await
            .is_ok()
            {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            let runtime = app.state::<Arc<Runtime>>();
            if secret_store::load::<Credentials>(&runtime.store)
                .ok()
                .flatten()
                .is_some_and(|c| !c.session.is_empty() && c.ak.is_some())
            {
                let result =
                    sync_command(app.clone(), "now".into(), Some(json!({"automatic":true}))).await;
                delay = match result {
                    Ok(value) => value["meetings"]["retryAt"]
                        .as_u64()
                        .filter(|at| *at > now())
                        .map(|at| at.saturating_sub(now()).clamp(1, 300))
                        .unwrap_or(300),
                    Err(_) => (delay * 2).min(3600),
                };
            }
        }
    });
}

fn pairing_changed(value: &Value) -> bool {
    value["unlocked"] == true
        || value["approved"].as_u64().is_some_and(|n| n > 0)
        || value["joined"].as_array().is_some_and(|j| !j.is_empty())
}

/// Relays the account key between the person's computers while SonoJot runs.
/// An unlocked computer looks for requests three times a minute and checks
/// for newly joined computers once a minute; a computer without the key keeps
/// asking until one answers. While an exchange moves, polls run every few
/// seconds so it finishes quickly.
pub fn start_pairing(app: &tauri::AppHandle) {
    if !crate::flavor::sync_enabled() {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(10)).await;
        let mut tick = 0u64;
        loop {
            let delay = pairing_tick(&app, tick).await;
            tick = tick.wrapping_add(1);
            tokio::time::sleep(std::time::Duration::from_secs(delay)).await;
        }
    });
}

async fn pairing_tick(app: &tauri::AppHandle, tick: u64) -> u64 {
    let saved = {
        let runtime = app.state::<Arc<Runtime>>();
        secret_store::load::<Credentials>(&runtime.store).ok().flatten()
    };
    let Some(c) = saved.filter(|c| !c.session.is_empty()) else {
        return PAIR_IDLE;
    };
    let result = if c.ak.is_some() {
        let relay = sync_command(app.clone(), "pair_relay".into(), None).await;
        let approved = relay.as_ref().is_ok_and(|r| r["approved"].as_u64().is_some_and(|n| n > 0));
        if tick % 3 == 0 || approved {
            if let Ok(found) = sync_command(app.clone(), "join_check".into(), None).await {
                notify_joins(app, &found["joined"]);
            }
        }
        relay
    } else if c.initialized != Some(false) {
        sync_command(app.clone(), "pair_wait".into(), None).await
    } else {
        return PAIR_IDLE;
    };
    match result {
        Ok(value) if value["advanced"] == true => PAIR_FAST,
        Ok(_) => PAIR_IDLE,
        Err(error) => crate::sync_client::rate_limit_delay(&error).unwrap_or(60).max(PAIR_IDLE),
    }
}

fn notify_joins(app: &tauri::AppHandle, joined: &Value) {
    use tauri_plugin_notification::NotificationExt;
    for device in joined.as_array().into_iter().flatten() {
        let name = platform_name(device["platform"].as_str().unwrap_or_default());
        // The in-app alert in Account and sync stays until OK or Not me.
        let _ = app
            .notification()
            .builder()
            .title(format!("{name} joined sync"))
            .body("If this wasn't you, open Account and sync and choose Not me.")
            .show();
    }
}

#[cfg(test)]
mod security_tests {
    use super::*;
    #[test]
    fn automatic_rotation_requires_the_pinned_predecessor_and_authenticated_roster() {
        let a = DeviceKey::new();
        let b = DeviceKey::new();
        let c = DeviceKey::new();
        let attacker = DeviceKey::new();
        let mut recipient = Credentials::new(STAGING.into());
        recipient.account = "fixture-account".into();
        recipient.device_id = "b".into();
        recipient.device = b.clone();
        let old = Keyring::new();
        recipient.keyset = Some(crypto::keyset_pin(&old));
        recipient.trusted = [("a".into(), a.public), ("b".into(), b.public)].into();
        let roster = TrustedRoster { keyset: recipient.keyset.unwrap(), devices: [("a".into(), a.public), ("b".into(), b.public), ("c".into(), c.public)].into() };
        let introduction = |sender: &DeviceKey| B64.encode(serde_json::to_vec(&RosterProof {
            sender: "a".into(), ciphertext: B64.encode(crypto::wrap_from(sender, &b.public, &recipient.account, "roster/b", &serde_json::to_vec(&roster).unwrap()).unwrap())
        }).unwrap());
        let mut next = old.clone();
        next.rotate().unwrap(); next.epoch = 2; next.devices = roster.devices.clone();
        let ak = Secret::random();
        let transfer = KeyTransfer { ak: ak.clone(), ring_rev: 2, vocabulary_rev: 4, epoch: 2, trust: Some(TrustedRoster { keyset: crypto::keyset_pin(&next), devices: next.devices.clone() }) };
        let message = RotationMessage { previous: recipient.keyset.unwrap(), transfer };
        let ciphertext = B64.encode(crypto::wrap_from(&c, &b.public, &recipient.account, "rekey/b/2", &serde_json::to_vec(&message).unwrap()).unwrap());
        let wrap = |intro: Option<String>, cipher: String, epoch| Wrap { kind: "device".into(), reference: "b".into(), rev: 2, ciphertext: B64.encode(serde_json::to_vec(&RotationWrap { sender: "c".into(), epoch, ciphertext: cipher, introduction: intro }).unwrap()) };
        let ring = Wrap { kind: "keyring".into(), reference: "main".into(), rev: 2, ciphertext: B64.encode(next.seal(&ak, &recipient.account, 2).unwrap()) };
        assert!(rotation_keys(&recipient, &[wrap(None, ciphertext.clone(), 2)], &ring).is_err());
        assert!(rotation_keys(&recipient, &[wrap(Some(introduction(&attacker)), ciphertext.clone(), 2)], &ring).is_err());
        let good = wrap(Some(introduction(&a)), ciphertext.clone(), 2);
        assert!(rotation_keys(&recipient, &[good.clone()], &ring).is_ok());
        assert!(rotation_keys(&recipient, &[wrap(Some(introduction(&a)), ciphertext.clone(), 3)], &ring).is_err());
        let mut forged = next.clone(); forged.rotate().unwrap();
        let forged_ring = Wrap { ciphertext: B64.encode(forged.seal(&ak, &recipient.account, 2).unwrap()), ..ring.clone() };
        assert!(rotation_keys(&recipient, &[good.clone()], &forged_ring).is_err());
        // After accepting an intervening rotation, an old device cannot replay
        // its signed/authenticated fork even if it still holds the old AK.
        recipient.keyset = Some(crypto::keyset_pin(&next));
        assert!(rotation_keys(&recipient, &[good], &ring).is_err());
    }
    #[test]
    fn removing_another_device_rekeys_unless_explicitly_opted_out() {
        assert!(removal_rekeys(&json!({}), "other", "self").unwrap());
        assert!(removal_rekeys(&json!({"rekey":null}), "other", "self").unwrap());
        assert!(!removal_rekeys(&json!({}), "self", "self").unwrap());
        assert!(!removal_rekeys(&json!({"rekey":false}), "other", "self").unwrap());
        assert!(removal_rekeys(&json!({"rekey":"false"}), "other", "self").is_err());
    }
    #[test]
    fn reset_notification_retries_delivery_without_repeating_or_crossing_accounts() {
        let root = std::env::temp_dir().join(format!("sj-reset-notice-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let mut c = Credentials::new(STAGING.into());
        c.account = "synthetic-account".into();
        secret_store::save(&store, &c).unwrap();
        let path = store.root().join(RESET_NOTICE);
        store
            .write(
                &path,
                &json!({"account":c.account,"server":c.server,"delivered":false}),
            )
            .unwrap();
        assert!(deliver_reset_notice(&store, |_, _| Err("OS unavailable".into())).is_err());
        assert_eq!(store.read::<Value>(&path).unwrap()["delivered"], false);
        deliver_reset_notice(&store, |title, body| {
            assert_eq!(title, RESET_NOTICE_TITLE);
            assert!(body.contains("re-key"));
            Ok(())
        })
        .unwrap();
        deliver_reset_notice(&store, |_, _| panic!("must not repeat")).unwrap();
        store
            .write(
                &path,
                &json!({"account":"other","server":c.server,"delivered":false}),
            )
            .unwrap();
        deliver_reset_notice(&store, |_, _| panic!("must not notify another account")).unwrap();
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn sync_pairing_resumes_commitments_and_expires_locally() {
        let root = std::env::temp_dir().join(format!("sj-pair-resume-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let mut c = Credentials::new("https://example.invalid".into());
        let pair = PairState {
            id: "synthetic-pair".into(), nonce: crypto::random(), public: c.device.public,
            commitment: crypto::random(), approver: false, peer_nonce: None,
            transcript: None, expires_at: now() + 300,
            state: "committed".into(), progress_at: now(),
        };
        c.pairs.insert(pair.id.clone(), pair.clone());
        secret_store::save(&store, &c).unwrap();
        tokio::runtime::Runtime::new().unwrap().block_on(async {
            let resumed = action(&store, "pair_start", json!({})).await.unwrap();
            assert_eq!(resumed["id"], pair.id);
            assert_eq!(resumed["expiresAt"], pair.expires_at);
            let saved = secret_store::load::<Credentials>(&store).unwrap().unwrap();
            assert_eq!(saved.pairs[&pair.id].nonce, pair.nonce);
            assert!(action(&store, "pair_accept", json!({"id":pair.id})).await.is_err());
            c.pairs.get_mut(&pair.id).unwrap().expires_at = now() - 1;
            secret_store::save(&store, &c).unwrap();
            assert_eq!(action(&store, "pair_poll", json!({"id":pair.id})).await.unwrap()["expired"], true);
        });
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn malicious_server_cannot_approve_its_own_account_key() {
        let sender = DeviceKey::new();
        let receiver = DeviceKey::new();
        let attacker = DeviceKey::new();
        let t = crypto::PairTranscript {
            account: "account".into(),
            request: "request".into(),
            receiver_id: "b".into(),
            receiver: receiver.public,
            sender_id: "a".into(),
            sender: sender.public,
            nonce_a: crypto::random(),
            nonce_b: crypto::random(),
        };
        let transfer = KeyTransfer {
            ak: Secret::random(),
            ring_rev: 1,
            vocabulary_rev: 1,
            epoch: 1,
            trust: None,
        };
        let plain = serde_json::to_vec(&transfer).unwrap();
        let base = crypto::wrap_device(&receiver.public, &t.account, &t.binding(), &plain).unwrap();
        let forged = crypto::wrap_from(
            &attacker,
            &receiver.public,
            &t.account,
            &t.binding(),
            &plain,
        )
        .unwrap();
        let good =
            crypto::wrap_from(&sender, &receiver.public, &t.account, &t.binding(), &plain).unwrap();
        assert!(authenticated_transfer(&receiver, &t, &base).is_err());
        assert!(authenticated_transfer(&receiver, &t, &forged).is_err());
        assert!(authenticated_transfer(&receiver, &t, &good).is_ok());
        // The package is bound to this exchange's nonces and request.
        let mut other = t.clone();
        other.nonce_a = crypto::random();
        assert!(authenticated_transfer(&receiver, &other, &good).is_err());
        let mut swapped = t.clone();
        swapped.sender = attacker.public;
        assert!(authenticated_transfer(&receiver, &swapped, &forged).is_err());
    }
    fn joins_fixture() -> (std::path::PathBuf, Store, Credentials) {
        let root = std::env::temp_dir().join(format!("sj-joins-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(root.clone())).unwrap();
        let mut c = Credentials::new("https://example.invalid".into());
        c.account = "synthetic-account".into();
        c.device_id = "self".into();
        (root, store, c)
    }
    fn device(id: &str, platform: &str, created: u64) -> Value {
        json!({"id":id,"platform":platform,"public_key":"cGs","created_at":created,"last_seen_at":created,"revoked_at":null})
    }
    #[test]
    fn join_alerts_start_after_this_computer_and_include_every_later_computer() {
        let (root, store, c) = joins_fixture();
        let mut devices = vec![device("self", "windows", 1), device("old", "linux", 2)];
        // The first look records what was already there without alerting.
        assert!(record_joins(&store, &c, &devices).unwrap().is_empty());
        assert!(pending_joins(&store, &c).is_empty());
        // A computer that joined through any route (relay by another computer
        // or recovery code) is simply a new signed-in computer here.
        devices.push(device("mac", "macos", 3));
        let fresh = record_joins(&store, &c, &devices).unwrap();
        assert_eq!(fresh, vec![Joined { id: "mac".into(), platform: "macos".into(), joined_at: 3 }]);
        assert_eq!(pending_joins(&store, &c), fresh);
        // Repeated checks never alert twice.
        assert!(record_joins(&store, &c, &devices).unwrap().is_empty());
        assert_eq!(pending_joins(&store, &c).len(), 1);
        // A computer that was removed drops out of the alerts.
        devices.push(device("phone", "ios", 4));
        record_joins(&store, &c, &devices).unwrap();
        devices.last_mut().unwrap()["revoked_at"] = json!(5);
        record_joins(&store, &c, &devices).unwrap();
        assert_eq!(pending_joins(&store, &c).iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), vec!["mac"]);
        ack_join(&store, &c, "mac").unwrap();
        assert!(pending_joins(&store, &c).is_empty());
        assert!(record_joins(&store, &c, &devices).unwrap().is_empty());
        // Placeholder rows without a key, and a response missing this
        // computer, never raise an alert.
        devices.push(json!({"id":"half","platform":"linux","public_key":"","created_at":6,"revoked_at":null}));
        assert!(record_joins(&store, &c, &devices).unwrap().is_empty());
        assert!(record_joins(&store, &c, &devices[1..]).unwrap().is_empty());
        // Another account on this computer starts over from its own baseline.
        let mut other = Credentials::new(c.server.clone());
        other.account = "other-account".into();
        other.device_id = "self".into();
        assert!(pending_joins(&store, &other).is_empty());
        assert!(record_joins(&store, &other, &devices).unwrap().is_empty());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn this_computer_never_alerts_about_itself() {
        let (root, store, c) = joins_fixture();
        record_joins(&store, &c, &[device("self", "windows", 1)]).unwrap();
        forget_joins(&store);
        store.write(&store.root().join(JOINS), &Joins { account: c.account.clone(), server: c.server.clone(), ..Default::default() }).unwrap();
        let fresh = record_joins(&store, &c, &[device("self", "windows", 1), device("b", "macos", 2)]).unwrap();
        assert_eq!(fresh.iter().map(|j| j.id.as_str()).collect::<Vec<_>>(), vec!["b"]);
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn platform_names_read_naturally() {
        assert_eq!(platform_name("macos"), "Mac");
        assert_eq!(platform_name("windows"), "Windows PC");
        assert_eq!(platform_name("Test"), "A new computer");
    }
    #[test]
    fn removed_device_cannot_replace_the_pinned_keyset_even_with_old_ak() {
        let mut c = Credentials::new("https://example.invalid".into());
        let mut ring = Keyring::new();
        c.keyset = Some(crypto::keyset_pin(&ring));
        c.trusted.insert("removed".into(), DeviceKey::new().public);
        assert!(check_keyset(&c, &ring).is_ok());
        ring.devices.remove("removed");
        assert!(check_keyset(&c, &ring).is_ok());
        ring.rotate().unwrap();
        assert!(check_keyset(&c, &ring).is_err());
        ring.epoch += 1;
        assert!(check_keyset(&c, &ring).is_err());
        c.keyset = None;
        assert!(check_keyset(&c, &ring).is_err());
    }
    #[test]
    fn sign_in_rotates_identity_and_clears_keys_across_accounts() {
        let mut c = Credentials::new("https://example.invalid".into());
        c.account = "account-a".into();
        c.ak = Some(Secret::random());
        c.keyset = Some([7; 32]);
        c.trusted.insert(c.device_id.clone(), c.device.public);
        let old_public = c.device.public;
        let next = DeviceKey::new();
        c.pending = Some(Pending {
            request: "request".into(),
            verifier: "synthetic".into(),
            expires: u64::MAX,
            device: next.clone(),
            device_id: "new-device".into(),
        });
        let reply = json!({"device":"new-device","account":{"id":"account-b","email":"b@example.invalid"},"token":"synthetic"});
        accept_signin(&mut c, &reply).unwrap();
        assert_ne!(c.device.public, old_public);
        assert_eq!(c.device.public, next.public);
        assert!(c.ak.is_none());
        assert!(c.keyset.is_none());
        assert!(c.trusted.is_empty());
        assert_eq!(c.account, "account-b");
        assert_eq!(c.device_id, "new-device");
    }
}

#[cfg(test)]
mod authkit_tests {
    use super::*;
    #[test]
    fn hosted_signin_is_the_only_provider() {
        assert_eq!(authkit_provider(&json!({})).unwrap(), "authkit");
        assert_eq!(
            authkit_provider(&json!({"provider":"authkit"})).unwrap(),
            "authkit"
        );
        for legacy in ["google", "apple"] {
            assert!(authkit_provider(&json!({"provider":legacy})).is_err());
        }
    }
    #[test]
    fn browser_urls_allow_only_the_worker_or_bound_workos_logout() {
        let server = "https://sync.example.invalid";
        assert!(browser_url(
            "https://sync.example.invalid/v1/auth/browser?request=test",
            server
        )
        .is_ok());
        assert!(browser_url("https://api.workos.com/user_management/sessions/logout?session_id=session_test&return_to=https%3A%2F%2Fsync.example.invalid%2Faccount",server).is_err());
        for url in [
            "https://accounts.google.com/",
            "https://evil.invalid/",
            "https://api.workos.com/user_management/sessions/logout?return_to=https://evil.invalid",
            "https://api.workos.com/other?return_to=https://sync.example.invalid/account",
        ] {
            assert!(browser_url(url, server).is_err());
        }
    }
}
