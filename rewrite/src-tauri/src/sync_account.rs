use crate::{
    app::Runtime,
    secret_store,
    store::Store,
    sync_client::{SyncClient, Wrap, STAGING},
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
    pub displayed: Option<[u8; 32]>,
    #[serde(default)]
    pub expires_at: u64,
}
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
            json!({"signedIn":!c.session.is_empty(),"unlocked":c.ak.is_some(),"email":c.email,"account":c.account,"device":c.device_id,"server":c.server,"recoveryPending":c.recovery.is_some(),"pendingSignin":c.pending.is_some(),"lastSync":meta.last_sync,"conflicts":meta.conflicts.len(),"pendingReset":pending_reset,"notice":notice,"resetCancelled":reset_notice["account"] == c.account && reset_notice["server"] == c.server})
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
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RotationWrap {
    sender: String,
    epoch: u64,
    ciphertext: String,
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
            .ok_or("Approve this device or use your recovery code first.")?;
    let ring = Keyring::open(ak, &c.account, w.rev, &bytes)
        .map_err(|_| "Encryption keys changed. Compare a pairing code on both devices or use the new recovery code.")?;
    check_keyset(c, &ring)?;
    store.write(
        &store.root().join("sync-keyring.json"),
        &json!({"account":c.account,"server":c.server,"wrap":w}),
    )?;
    Ok((ring, w.rev))
}
fn confirmed_transfer(
    device: &DeviceKey,
    transcript: &crypto::PairTranscript,
    displayed: Option<[u8; 32]>,
    ciphertext: &[u8],
) -> Result<KeyTransfer, String> {
    if displayed != Some(transcript.hash()) {
        return Err("Compare and confirm the code on this device first.".into());
    }
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
        return Err("Encryption keys changed. Compare a pairing code on both devices or use the new recovery code.".into());
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
    // Inspect the remote revision without uploading pending local edits under old keys.
    let (vocabulary_revision, prune_cursor) = if reset {
        (0, None)
    } else {
        let usage = crate::sync_engine::rotation_usage(store, c, &ring).await?;
        ring.rotate_retaining(&usage.keys)?;
        ring.vocabulary_cutover = usage.vocabulary_rev;
        (usage.vocabulary_rev, Some(usage.cursor))
    };
    // The old AK may be held by the removed device. Never distribute a fresh AK
    // to a roster that it could forge with a colluding server.
    ring.devices.clear();
    ring.devices.insert(c.device_id.clone(), c.device.public);
    if ring.devices.len() > 64 {
        return Err("Remove unused devices before re-keying.".into());
    }
    ring.epoch = epoch + 1;
    let ak = Secret::random();
    let recovery = crypto::recovery_code().to_string();
    let transfer = KeyTransfer {
        ak: ak.clone(),
        ring_rev: rev + 1,
        vocabulary_rev: vocabulary_revision,
        epoch: epoch + 1,
    };
    let plain = zeroize::Zeroizing::new(
        serde_json::to_vec(&transfer).map_err(|_| "Cannot wrap new keys.")?,
    );
    let devices=ring.devices.iter().map(|(id,public)|{
  let sealed=crypto::wrap_from(&c.device,public,&c.account,&format!("rekey/{id}/{}",epoch+1),&plain)?;
  let wrapped=RotationWrap{sender:c.device_id.clone(),epoch:epoch+1,ciphertext:B64.encode(sealed)};
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
    let _: Value = client
        .json(Method::POST, "/v1/rekey", Some(&request))
        .await?;
    c.keyset = Some(crypto::keyset_pin(&ring));
    c.ak = Some(ak);
    c.trusted = ring.devices;
    secret_store::save(store, c)?;
    if reset {
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
            if text(&reply, "device")? != fresh.device_id { return Err("Test device identity changed.".into()); }
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
                let ring = Keyring::open(
                    c.ak.as_ref().ok_or("Unlock this device first.")?,
                    &c.account,
                    w.rev,
                    &B64.decode(&w.ciphertext)
                        .map_err(|_| "Invalid cached keys.")?,
                )?;
                check_keyset(&c, &ring)?;
                (ring, w.rev)
            } else {
                keyring(store, &mut c).await?
            };
            let meta = crate::sync_engine::synchronize(store, &c, &ring, rev).await?;
            return Ok(
                json!({"synced":true,"lastSync":meta.last_sync,"conflicts":meta.conflicts.len()}),
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
            let signin_device = DeviceKey::new();
            let signin_id = uuid::Uuid::new_v4().to_string();
            let verifier = B64.encode(crypto::random::<32>());
            let challenge = B64.encode(Sha256::digest(verifier.as_bytes()));
            let reply:Value=c.client()?.json(Method::POST,"/v1/auth/start",Some(&json!({"provider":provider,"challenge":challenge,"device":signin_id,"publicKey":B64.encode(signin_device.public),"platform":std::env::consts::OS,"redirect":args["redirect"],"link":args["link"]}))).await?;
            c.pending = Some(Pending {
                request: text(&reply, "id")?.into(),
                verifier,
                expires: now() + 600,
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
            if c.ak.is_some() {
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
                return Err("This account already has encrypted words. Approve this device or use recovery.".into());
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
                return status(store);
            }
        }
        "pair_start" => {
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
            let _: Value = c
                .client()?
                .json(
                    Method::POST,
                    "/v1/pairings",
                    Some(&json!({"id":id,"commitment":B64.encode(commit)})),
                )
                .await?;
            c.pairs.clear();
            c.pairs.insert(
                id.clone(),
                PairState {
                    id: id.clone(),
                    nonce: transcript.nonce_b,
                    public: c.device.public,
                    commitment: commit,
                    approver: false,
                    peer_nonce: None,
                    transcript: Some(transcript),
                    displayed: None,
                    expires_at: now() + 300,
                },
            );
            secret_store::save(store, &c)?;
            return Ok(json!({"id":id}));
        }
        "pair_list" => return c.client()?.json(Method::GET, "/v1/pairings", None).await,
        "pair_accept" => {
            let id = safe_id(text(&args, "id")?)?;
            let p: Value = c
                .client()?
                .json(Method::GET, &format!("/v1/pairings/{id}"), None)
                .await?;
            let transcript = crypto::PairTranscript {
                account: c.account.clone(),
                request: id.into(),
                receiver_id: text(&p, "device")?.into(),
                receiver: decode32(text(&p, "public_key")?)?,
                sender_id: c.device_id.clone(),
                sender: c.device.public,
                nonce_a: crypto::random(),
                nonce_b: [0; 32],
            };
            let _:Value = c.client()?.json(Method::POST,&format!("/v1/pairings/{id}"),Some(&json!({"action":"accept","commitment":B64.encode(transcript.sender_commit())}))).await?;
            c.pairs.insert(
                id.into(),
                PairState {
                    id: id.into(),
                    nonce: transcript.nonce_a,
                    public: transcript.receiver,
                    commitment: decode32(text(&p, "commitment")?)?,
                    approver: true,
                    peer_nonce: Some(transcript.sender_commit()),
                    transcript: Some(transcript),
                    displayed: None,
                    expires_at: now() + 300,
                },
            );
            secret_store::save(store, &c)?;
            return Ok(json!({"waiting":true}));
        }
        "pair_poll" | "pair_approve" | "pair_confirm" | "pair_deny" => {
            let id = safe_id(text(&args, "id")?)?;
            let path = format!("/v1/pairings/{id}");
            if operation == "pair_deny" {
                let _: Value = c
                    .client()?
                    .json(Method::POST, &path, Some(&json!({"action":"deny"})))
                    .await?;
                c.pairs.remove(id);
                secret_store::save(store, &c)?;
                return Ok(json!({"denied":true}));
            }
            let p: Value = c.client()?.json(Method::GET, &path, None).await?;
            let mut local = c
                .pairs
                .get(id)
                .ok_or("Start pairing on this device again.")?
                .clone();
            let mut t = local
                .transcript
                .clone()
                .ok_or("Start pairing again with the updated app.")?;
            if text(&p, "device")? != t.receiver_id
                || decode32(text(&p, "public_key")?)? != t.receiver
                || decode32(text(&p, "commitment")?)? != local.commitment
            {
                return Err("Pairing identity changed. Deny and start again.".into());
            }
            if p["state"] == "committed" {
                return Ok(json!({"waiting":true}));
            }
            if !local.approver && local.peer_nonce.is_none() {
                // Pin the approver's commitment BEFORE revealing our nonce.
                if p["state"] != "accepted" {
                    return Err("Pairing skipped its commitment. Start again.".into());
                }
                local.peer_nonce = Some(decode32(text(&p, "commitment_a")?)?);
                local.transcript = Some(t.clone());
                c.pairs.insert(id.into(), local.clone());
                secret_store::save(store, &c)?;
            }
            if local.peer_nonce != Some(decode32(text(&p, "commitment_a")?)?) {
                return Err("Approver commitment changed.".into());
            }
            if !local.approver && p["state"] == "accepted" {
                let _: Value = c
                    .client()?
                    .json(
                        Method::POST,
                        &path,
                        Some(&json!({"action":"reveal","nonce":B64.encode(t.nonce_b)})),
                    )
                    .await?;
                return Ok(json!({"waiting":true}));
            }
            if local.approver && p["state"] == "accepted" {
                return Ok(json!({"waiting":true}));
            }
            let b = decode32(text(&p, "nonce_b")?)?;
            if !local.approver && b != t.nonce_b {
                return Err("Pairing nonce changed.".into());
            }
            t.nonce_b = b;
            if t.receiver_commit() != local.commitment {
                return Err("Recipient commitment changed.".into());
            }
            if local.approver && p["state"] == "revealed" {
                let _: Value = c
                    .client()?
                    .json(
                        Method::POST,
                        &path,
                        Some(&json!({"action":"reveal_approver","nonce":B64.encode(t.nonce_a)})),
                    )
                    .await?;
                return Ok(json!({"waiting":true}));
            }
            if p["state"] == "revealed" {
                return Ok(json!({"waiting":true}));
            }
            if !local.approver {
                t.sender = decode32(text(&p, "public_key_a")?)?;
                t.sender_id = text(&p, "approver")?.into();
                t.nonce_a = decode32(text(&p, "nonce_a")?)?;
            }
            if t.sender_commit() != local.peer_nonce.ok_or("Missing approver commitment.")? {
                return Err("Approver identity changed.".into());
            }
            if !["ready", "approved"].contains(&text(&p, "state")?) {
                return Err("Pairing is no longer available.".into());
            }
            let code = t.code();
            let binding = t.binding();
            if operation == "pair_approve" || operation == "pair_confirm" {
                confirmed(&args)?;
                if local.displayed != Some(t.hash()) {
                    return Err("Display and compare the code on both devices first.".into());
                }
            }
            if operation == "pair_approve" {
                if !local.approver || p["state"] != "ready" {
                    return Err("Pairing is not ready for approval.".into());
                }
                let (mut ring, rev) = keyring(store, &mut c).await?;
                ring.devices.insert(t.receiver_id.clone(), t.receiver);
                let ak = c.ak.as_ref().ok_or("Unlock this device first.")?;
                c.client()?
                    .put_wrap(
                        "keyring",
                        "main",
                        B64.encode(ring.seal(ak, &c.account, rev + 1)?),
                        rev,
                    )
                    .await?;
                let meta = crate::sync_engine::metadata(store, &c)?;
                let transfer = KeyTransfer {
                    ak: ak.clone(),
                    ring_rev: rev + 1,
                    vocabulary_rev: meta.rev,
                    epoch: ring.epoch,
                };
                let plain = zeroize::Zeroizing::new(
                    serde_json::to_vec(&transfer).map_err(|_| "Cannot encode pairing keys.")?,
                );
                let wrap = crypto::wrap_from(&c.device, &t.receiver, &c.account, &binding, &plain)?;
                let _:Value = c.client()?.json(Method::POST,&path,Some(&json!({"action":"approve","confirmed":true,"ciphertext":B64.encode(wrap)}))).await?;
                c.pairs.remove(id);
                secret_store::save(store, &c)?;
                return Ok(json!({"approved":true}));
            }
            if operation == "pair_confirm" {
                if local.approver || p["state"] != "approved" {
                    return Err("Approve on the other device first, then confirm here.".into());
                }
                let transfer = confirmed_transfer(
                    &c.device,
                    &t,
                    local.displayed,
                    &B64.decode(text(&p, "ciphertext")?)
                        .map_err(|_| "Invalid wrapped keys.")?,
                )?;
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
                c.ak = Some(transfer.ak.clone());
                c.keyset = Some(crypto::keyset_pin(&ring));
                c.trusted = [
                    (t.sender_id.clone(), t.sender),
                    (t.receiver_id.clone(), t.receiver),
                ]
                .into();
                apply_checkpoint(store, &c, &transfer)?;
                secret_store::save(store, &c)?;
                cancel_reset(store, &c).await?;
                device_wrap(&c).await?;
                let _: Value = c
                    .client()?
                    .json(Method::POST, &path, Some(&json!({"action":"consume"})))
                    .await?;
                c.pairs.remove(id);
                secret_store::save(store, &c)?;
                return Ok(json!({"unlocked":true}));
            }
            local.transcript = Some(t.clone());
            local.displayed = Some(t.hash());
            c.pairs.insert(id.into(), local.clone());
            secret_store::save(store, &c)?;
            return Ok(
                json!({"code":code,"approver":local.approver,"ready":p["state"] == "approved"}),
            );
        }
        "signout" => {
            let result = c
                .client()?
                .json::<Value>(Method::POST, "/v1/auth/signout", Some(&json!({})))
                .await;
            secret_store::delete(store)?;
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
        }
        _ => return Err("Unknown sync action.".into()),
    }
    status(store)
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
    if provider != "authkit" {
        return Err("Use WorkOS AuthKit to sign in.".into());
    }
    Ok(provider)
}
fn browser_url(value: &str, server: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "Invalid sign-in URL.")?;
    let origin = reqwest::Url::parse(server).map_err(|_| "Invalid sync server.")?;
    let workos_logout = url.host_str() == Some("api.workos.com")
        && url.path() == "/user_management/sessions/logout"
        && url.port_or_known_default() == Some(443)
        && url.query_pairs().any(|(key, value)| {
            key == "return_to" && value == format!("{}/account", server.trim_end_matches('/'))
        });
    if url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || !(url.origin() == origin.origin() || workos_logout)
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
        return Ok(json!({"credentialStore":"dedicated-test-keychain","phase":phase,"passed":true,"probeDeleted":phase != "prepare"}));
    }
    #[cfg(feature = "synctest")]
    if operation == "test_offline" {
        let offline = args["offline"].as_bool().ok_or("Choose offline on or off.")?;
        crate::sync_client::TEST_OFFLINE.store(offline, std::sync::atomic::Ordering::Relaxed);
        if !offline { wake(); }
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
        "status" | "devices" | "pair_list" | "conflicts" | "recovery_show"
    ) {
        if let Ok(mut failure) = service.failure.lock() {
            *failure = result.as_ref().err().cloned()
        }
        let _ = app.emit("sync-changed", ());
    }
    result
}
pub fn callback(app: &tauri::AppHandle, value: String) {
    if !crate::flavor::sync_enabled() { return; }
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
    if !crate::flavor::sync_enabled() { return; }
    WAKE.get_or_init(tokio::sync::Notify::new).notify_one();
}
pub fn start_scheduler(app: &tauri::AppHandle) {
    if !crate::flavor::sync_enabled() { return; }
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
                let result = sync_command(app.clone(), "now".into(), None).await;
                delay = if result.is_ok() {
                    300
                } else {
                    (delay * 2).min(3600)
                };
            }
        }
    });
}

#[cfg(test)]
mod security_tests {
    use super::*;
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
        assert!(confirmed_transfer(&receiver, &t, None, &good).is_err());
        assert!(confirmed_transfer(&receiver, &t, Some(t.hash()), &base).is_err());
        assert!(confirmed_transfer(&receiver, &t, Some(t.hash()), &forged).is_err());
        assert!(confirmed_transfer(&receiver, &t, Some(t.hash()), &good).is_ok());
        let mut swapped = t.clone();
        swapped.sender = attacker.public;
        assert!(confirmed_transfer(&receiver, &swapped, Some(t.hash()), &forged).is_err());
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
        for legacy in ["google", "apple", "email"] {
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
        assert!(browser_url("https://api.workos.com/user_management/sessions/logout?session_id=session_test&return_to=https%3A%2F%2Fsync.example.invalid%2Faccount",server).is_ok());
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
