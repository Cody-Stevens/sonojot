//! Optional OpenRouter settings and generation. Recording and status never
//! contact a provider; proofreading and meeting speech require explicit choices.
use crate::store::Store;
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    future::{poll_fn, Future},
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    pin::pin,
    sync::atomic::{AtomicBool, Ordering},
    task::Poll,
    time::{Duration, Instant},
};

const SETTINGS_FILE: &str = "ai-settings.json";
#[cfg(not(target_os = "macos"))]
const KEY_FILE: &str = "openrouter-key.dpapi.json";
#[cfg(target_os = "macos")]
const KEY_FILE: &str = "openrouter-key.keychain.json";
const POLICY_FILE: &str = "model-policy.json";
const AUTH_URL: &str = "https://openrouter.ai/auth";
const KEYS_URL: &str = "https://openrouter.ai/api/v1/auth/keys";
pub(crate) const MODELS_URL: &str = "https://openrouter.ai/api/v1/models";
pub(crate) const COMPLETIONS_URL: &str = "https://openrouter.ai/api/v1/chat/completions";
// OpenRouter requires a public callback for a custom app identity. This page
// only relays the single-use code to the loopback listener; it never sees the
// PKCE verifier. Its port/state contract is documented in
// web/connect-openrouter.md and must change on both sides together.
const CALLBACK_PAGE_URL: &str = "https://sonojot.com/connect/openrouter";
const RECOMMENDATIONS_URL: &str = "https://sonojot.com/recommendations.json";
const AUTH_TIMEOUT: Duration = Duration::from_secs(180);
const RECOMMENDATIONS_TIMEOUT: Duration = Duration::from_secs(8);
const MAX_CALLBACK_BYTES: usize = 8192;
const MAX_KEY_BYTES: usize = 4096;
const MAX_OUTPUT_BYTES: usize = 65_536;
const MAX_RECOMMENDATIONS_BYTES: usize = 65_536;
const MAX_RECOMMENDED_MODELS: usize = 32;
const MAX_QUICK_MODELS: usize = 32;
const MAX_DESCRIPTION_BYTES: usize = 240;
/// Reasoning tokens share the model's output budget. When the catalog says a
/// model thinks by default, requests must cover both thinking and text, capped
/// at the existing hard bound.
const REASONING_SHARED_BUDGET: u32 = 8192;
pub(crate) const CANCELLED: &str = "AI operation cancelled.";
const HELP: &str = "Local processing stays on this computer. OpenRouter sends the requested audio or text to your selected model and bills your OpenRouter account. SonoJot excludes providers that collect data, so some models may be unavailable. ChatGPT subscriptions do not include OpenRouter usage.";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AiBackend {
    #[default]
    Local,
    OpenRouter,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum AiQuickModelRole {
    Text,
    Speech,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiQuickModel {
    pub id: String,
    pub role: AiQuickModelRole,
}

#[derive(Clone, Debug, Default, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AiSettings {
    pub backend: AiBackend,
    pub model: String,
    pub speech_model: String,
    pub quick_models: Vec<AiQuickModel>,
}

impl<'de> Deserialize<'de> for AiSettings {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(rename_all = "camelCase", deny_unknown_fields)]
        struct Fields {
            backend: AiBackend,
            model: String,
            #[serde(default)]
            speech_model: String,
            #[serde(default, deserialize_with = "present_quick_models")]
            quick_models: Option<Vec<AiQuickModel>>,
        }

        // Only an absent field migrates. An explicit array (even empty) is the
        // user's list; null and other non-array values must still be rejected.
        fn present_quick_models<'de, D: serde::Deserializer<'de>>(
            deserializer: D,
        ) -> Result<Option<Vec<AiQuickModel>>, D::Error> {
            Vec::deserialize(deserializer).map(Some)
        }

        let fields = Fields::deserialize(deserializer)?;
        let quick_models = fields.quick_models.unwrap_or_else(|| {
            [
                (&fields.model, AiQuickModelRole::Text),
                (&fields.speech_model, AiQuickModelRole::Speech),
            ]
            .into_iter()
            .filter(|(id, _)| !id.is_empty())
            .map(|(id, role)| AiQuickModel {
                id: id.clone(),
                role,
            })
            .collect()
        });
        Ok(Self {
            backend: fields.backend,
            model: fields.model,
            speech_model: fields.speech_model,
            quick_models,
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiSettingsStatus {
    pub backend: AiBackend,
    pub model: String,
    pub speech_model: String,
    pub quick_models: Vec<AiQuickModel>,
    pub connected: bool,
    pub supported: bool,
    pub help: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AiModel {
    pub id: String,
    pub name: String,
    pub context_length: u64,
    pub input_modalities: Vec<String>,
    /// OpenRouter's USD-per-token price, preserved as a decimal string.
    pub prompt_price: Option<String>,
    pub completion_price: Option<String>,
    /// Short curator note from the recommendations document, if any.
    pub description: Option<String>,
    /// Curated OpenRouter reasoning effort for a recommended model.
    pub reasoning_effort: Option<String>,
    pub supported_parameters: Vec<String>,
    pub training_required: bool,
}

#[cfg(not(target_os = "macos"))]
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ProtectedKey {
    version: u32,
    ciphertext: String,
}

pub(crate) fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && model.len() <= 200
        && model.contains('/')
        && !model.contains("://")
        && !model.ends_with(":online")
        && model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._/:@~+".contains(&b))
}

fn validate_settings(settings: &AiSettings) -> Result<(), String> {
    if settings.quick_models.len() > MAX_QUICK_MODELS {
        return Err(format!(
            "Keep up to {MAX_QUICK_MODELS} models in your OpenRouter list."
        ));
    }
    let mut seen = HashSet::new();
    for model in &settings.quick_models {
        if !valid_model(&model.id) {
            return Err("Choose models from the OpenRouter model list.".into());
        }
        if !seen.insert((model.id.as_str(), model.role)) {
            return Err("Your OpenRouter list contains a duplicate model and role.".into());
        }
    }
    if !settings.speech_model.is_empty() && !valid_model(&settings.speech_model) {
        return Err("Choose a speech model from the OpenRouter model list.".into());
    }
    if !settings.model.is_empty() && !valid_model(&settings.model) {
        return Err("Choose a text model from the OpenRouter model list.".into());
    }
    if settings.backend == AiBackend::OpenRouter {
        supported()?;
        if settings.model.is_empty() {
            return Err("Choose an OpenRouter model before using it.".into());
        }
    }
    Ok(())
}

pub(crate) fn supported() -> Result<(), String> {
    if cfg!(any(windows, target_os = "macos", target_os = "linux")) {
        Ok(())
    } else {
        Err("OpenRouter connection is not yet available on this system. Local processing is available.".into())
    }
}

fn load_unlocked(store: &Store) -> Result<AiSettings, String> {
    let path = store.root().join(SETTINGS_FILE);
    if !path.exists() {
        return Ok(AiSettings::default());
    }
    let settings = store.read(&path)?;
    validate_settings(&settings)?;
    Ok(settings)
}

pub fn load_settings(store: &Store) -> Result<AiSettings, String> {
    let _guard = store.lock()?;
    load_unlocked(store)
}

pub fn settings_status(store: &Store) -> Result<AiSettingsStatus, String> {
    let _guard = store.lock()?;
    let settings = load_unlocked(store)?;
    let supported = supported().is_ok();
    let connected = supported && key_saved_unlocked(store);
    Ok(AiSettingsStatus {
        backend: settings.backend,
        model: settings.model,
        speech_model: settings.speech_model,
        quick_models: settings.quick_models,
        connected,
        supported,
        help: if supported {
            HELP.into()
        } else {
            "Local processing stays on this computer. OpenRouter connection is not yet available on this system.".into()
        },
    })
}

pub fn save_settings(store: &Store, settings: &AiSettings) -> Result<AiSettingsStatus, String> {
    validate_settings(settings)?;
    {
        let _guard = store.lock()?;
        if settings.backend == AiBackend::OpenRouter {
            load_key_unlocked(store)?;
        }
        store.write(&store.root().join(SETTINGS_FILE), settings)?;
    }
    settings_status(store)
}

/// Disconnect removes the app's protected key, not the user's provider account.
pub fn disconnect(store: &Store) -> Result<AiSettingsStatus, String> {
    {
        let _guard = store.lock()?;
        let mut settings = load_unlocked(store)?;
        settings.backend = AiBackend::Local;
        store.write(&store.root().join(SETTINGS_FILE), &settings)?;
        remove_key_unlocked(store)?;
    }
    settings_status(store)
}

fn remove_key_unlocked(store: &Store) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    keychain::remove(store)?;
    #[cfg(target_os = "linux")]
    if let Some(reference) = saved_reference(store) {
        secret_service::forget(&reference);
    }
    let path = store.root().join(KEY_FILE);
    store.check_owned(&path)?;
    if path.exists() {
        std::fs::remove_file(&path)
            .map_err(|_| "Cannot remove the saved OpenRouter connection.".to_string())?;
    }
    Ok(())
}

fn valid_secret(value: &str) -> bool {
    !value.is_empty() && value.len() <= MAX_KEY_BYTES && value.bytes().all(|b| b.is_ascii_graphic())
}

/// Whether a connection is saved. On macOS this reads keychain attributes only,
/// so showing status never asks for keychain access.
fn key_saved_unlocked(store: &Store) -> bool {
    #[cfg(target_os = "macos")]
    return keychain::saved(store);
    #[cfg(not(target_os = "macos"))]
    load_key_unlocked(store).is_ok()
}

pub(crate) fn load_key_unlocked(store: &Store) -> Result<String, String> {
    #[cfg(target_os = "macos")]
    let key = keychain::load(store)?;
    #[cfg(not(target_os = "macos"))]
    let key = load_protected_key(store)?;
    if !valid_secret(&key) {
        return Err("The saved OpenRouter connection is invalid.".into());
    }
    Ok(key)
}

#[cfg(not(target_os = "macos"))]
fn load_protected_key(store: &Store) -> Result<String, String> {
    let path = store.root().join(KEY_FILE);
    if !path.exists() {
        return Err("Connect to OpenRouter before using a cloud model.".into());
    }
    // DPAPI blobs are small; refuse a corrupted file before JSON allocation.
    store.check_owned(&path)?;
    if std::fs::metadata(&path)
        .map_err(|_| "Cannot read the saved OpenRouter connection.")?
        .len()
        > 32_768
    {
        return Err(
            "The saved OpenRouter connection is invalid. Disconnect and connect again.".into(),
        );
    }
    let protected: ProtectedKey = store.read(&path)?;
    if protected.version != 1 {
        return Err("The saved OpenRouter connection needs to be replaced.".into());
    }
    let encrypted = URL_SAFE_NO_PAD
        .decode(protected.ciphertext)
        .map_err(|_| "The saved OpenRouter connection is invalid.")?;
    let bytes = protect_bytes(&encrypted, false)?;
    String::from_utf8(bytes).map_err(|_| "The saved OpenRouter connection is invalid.".into())
}

fn save_key(store: &Store, key: &str, cancel: &AtomicBool) -> Result<(), String> {
    if !valid_secret(key) {
        return Err("OpenRouter returned an invalid connection key.".into());
    }
    #[cfg(target_os = "macos")]
    return keychain::save(store, key, cancel);
    #[cfg(not(target_os = "macos"))]
    save_protected_key(store, key, cancel)
}

#[cfg(not(target_os = "macos"))]
fn save_protected_key(store: &Store, key: &str, cancel: &AtomicBool) -> Result<(), String> {
    let encrypted = protect_bytes(key.as_bytes(), true)?;
    let _guard = store.lock()?;
    #[cfg(target_os = "linux")]
    let previous = saved_reference(store);
    let written = check_cancel(cancel).and_then(|_| {
        store.write(
            &store.root().join(KEY_FILE),
            &ProtectedKey {
                version: 1,
                ciphertext: URL_SAFE_NO_PAD.encode(&encrypted),
            },
        )
    });
    #[cfg(target_os = "linux")]
    match &written {
        Ok(()) => {
            if let Some(reference) = previous {
                secret_service::forget(&reference);
            }
        }
        Err(_) => secret_service::forget(&encrypted),
    }
    written
}

#[cfg(windows)]
fn protect_bytes(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
        },
    };
    if input.is_empty() || input.len() > 16_384 {
        return Err("The saved OpenRouter connection is invalid.".into());
    }
    let blob = CRYPT_INTEGER_BLOB {
        cbData: input.len() as u32,
        pbData: input.as_ptr() as *mut u8,
    };
    let mut output = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: std::ptr::null_mut(),
    };
    // DPAPI scopes this blob to the current Windows account. UI is forbidden in
    // background commands. Its allocation is copied then cleared and freed once.
    let ok = unsafe {
        if encrypt {
            CryptProtectData(
                &blob,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &blob,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(
            "Windows could not unlock the OpenRouter connection. Disconnect and connect again."
                .into(),
        );
    }
    if output.pbData.is_null() || output.cbData == 0 || output.cbData > 16_384 {
        if !output.pbData.is_null() {
            unsafe {
                LocalFree(output.pbData.cast());
            }
        }
        return Err("Windows returned an invalid protected connection.".into());
    }
    let result =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        for offset in 0..output.cbData as usize {
            output.pbData.add(offset).write_volatile(0);
        }
        LocalFree(output.pbData.cast());
    }
    Ok(result)
}

/// Linux keeps the key in the Secret Service (GNOME Keyring or KWallet)
/// through libsecret's `secret-tool`. The data folder records only the
/// random item id, so every store owns a separate item.
#[cfg(target_os = "linux")]
mod secret_service {
    use std::io::{Read, Write};
    use std::process::{Command, Stdio};

    const UNAVAILABLE: &str = "Saving the OpenRouter connection needs a Secret Service keyring (gnome-keyring) and secret-tool (libsecret).";

    fn run(args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, String> {
        let mut child = Command::new("secret-tool")
            .args(args)
            .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|_| UNAVAILABLE.to_string())?;
        if let Some(input) = input {
            let mut stdin = child.stdin.take().ok_or(UNAVAILABLE)?;
            stdin.write_all(input).map_err(|_| UNAVAILABLE.to_string())?;
        }
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or(UNAVAILABLE)?
            .take(16_384)
            .read_to_end(&mut output)
            .map_err(|_| UNAVAILABLE.to_string())?;
        let status = child.wait().map_err(|_| UNAVAILABLE.to_string())?;
        if status.success() { Ok(output) } else { Err(UNAVAILABLE.into()) }
    }

    fn valid(id: &str) -> bool {
        uuid::Uuid::parse_str(id).is_ok()
    }

    pub(super) fn store(secret: &[u8]) -> Result<Vec<u8>, String> {
        let id = uuid::Uuid::new_v4().to_string();
        run(
            &["store", "--label=SonoJot OpenRouter connection", "application", "sonojot", "item", &id],
            Some(secret),
        )?;
        Ok(id.into_bytes())
    }

    pub(super) fn lookup(reference: &[u8]) -> Result<Vec<u8>, String> {
        let id = std::str::from_utf8(reference).ok().filter(|id| valid(id))
            .ok_or("The saved OpenRouter connection is invalid. Disconnect and connect again.")?;
        let secret = run(&["lookup", "application", "sonojot", "item", id], None)
            .map_err(|_| "The keyring did not return the OpenRouter connection. Unlock the keyring, or disconnect and connect again.".to_string())?;
        if secret.is_empty() {
            return Err("The OpenRouter connection is missing from the keyring. Disconnect and connect again.".into());
        }
        Ok(secret)
    }

    pub(super) fn forget(reference: &[u8]) {
        if let Some(id) = std::str::from_utf8(reference).ok().filter(|id| valid(id)) {
            let _ = run(&["clear", "application", "sonojot", "item", id], None);
        }
    }
}

#[cfg(target_os = "linux")]
fn protect_bytes(input: &[u8], encrypt: bool) -> Result<Vec<u8>, String> {
    if input.is_empty() || input.len() > 16_384 {
        return Err("The saved OpenRouter connection is invalid.".into());
    }
    if encrypt { secret_service::store(input) } else { secret_service::lookup(input) }
}

/// The keyring item named by the saved connection file, if any.
#[cfg(target_os = "linux")]
fn saved_reference(store: &Store) -> Option<Vec<u8>> {
    let path = store.root().join(KEY_FILE);
    let protected: ProtectedKey = store.read(&path).ok()?;
    URL_SAFE_NO_PAD.decode(protected.ciphertext).ok()
}

#[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
fn protect_bytes(_input: &[u8], _encrypt: bool) -> Result<Vec<u8>, String> {
    Err("Secure OpenRouter storage is not yet available on this system.".into())
}

/// macOS keeps the key in the login keychain, where only this app's signature
/// can read it without asking. The data folder records only which item it is,
/// so every store (including test stores) owns a separate item.
#[cfg(target_os = "macos")]
mod keychain {
    use super::{check_cancel, KEY_FILE};
    use crate::store::Store;
    use security_framework::{
        item::{ItemClass, ItemSearchOptions},
        passwords,
    };
    use serde::{Deserialize, Serialize};
    use std::sync::atomic::AtomicBool;

    const SERVICE: &str = "SonoJot OpenRouter connection";
    const NOT_FOUND: i32 = -25300; // errSecItemNotFound

    #[derive(Deserialize, Serialize)]
    #[serde(deny_unknown_fields)]
    struct KeychainKey {
        version: u32,
        account: String,
    }

    fn account(store: &Store) -> Result<Option<String>, String> {
        let path = store.root().join(KEY_FILE);
        if !path.exists() {
            return Ok(None);
        }
        store.check_owned(&path)?;
        let invalid = "The saved OpenRouter connection is invalid. Disconnect and connect again.";
        if std::fs::metadata(&path).map_err(|_| invalid)?.len() > 4096 {
            return Err(invalid.into());
        }
        let saved: KeychainKey = store.read(&path).map_err(|_| invalid)?;
        if saved.version != 1 || uuid::Uuid::parse_str(&saved.account).is_err() {
            return Err("The saved OpenRouter connection needs to be replaced.".into());
        }
        Ok(Some(saved.account))
    }

    pub(super) fn saved(store: &Store) -> bool {
        let Ok(Some(account)) = account(store) else {
            return false;
        };
        ItemSearchOptions::new()
            .class(ItemClass::generic_password())
            .service(SERVICE)
            .account(&account)
            .load_attributes(true)
            .search()
            .is_ok_and(|items| !items.is_empty())
    }

    pub(super) fn load(store: &Store) -> Result<String, String> {
        let account =
            account(store)?.ok_or("Connect to OpenRouter before using a cloud model.")?;
        let bytes = passwords::get_generic_password(SERVICE, &account).map_err(|error| {
            if error.code() == NOT_FOUND {
                "The OpenRouter connection is missing from your keychain. Disconnect and connect again."
            } else {
                "macOS could not unlock the OpenRouter connection. Allow SonoJot to use it, or disconnect and connect again."
            }
        })?;
        String::from_utf8(bytes).map_err(|_| "The saved OpenRouter connection is invalid.".into())
    }

    pub(super) fn save(store: &Store, key: &str, cancel: &AtomicBool) -> Result<(), String> {
        let account = uuid::Uuid::new_v4().to_string();
        passwords::set_generic_password(SERVICE, &account, key.as_bytes())
            .map_err(|_| "macOS could not save the OpenRouter connection in your keychain.")?;
        let _guard = store.lock()?;
        let previous = self::account(store).ok().flatten();
        let written = check_cancel(cancel).and_then(|()| {
            store.write(
                &store.root().join(KEY_FILE),
                &KeychainKey {
                    version: 1,
                    account: account.clone(),
                },
            )
        });
        if written.is_err() {
            let _ = delete(&account);
            return written;
        }
        if let Some(previous) = previous {
            let _ = delete(&previous);
        }
        Ok(())
    }

    /// Removes the keychain item; the caller removes the record file.
    pub(super) fn remove(store: &Store) -> Result<(), String> {
        match account(store) {
            Ok(Some(account)) => delete(&account),
            _ => Ok(()),
        }
    }

    fn delete(account: &str) -> Result<(), String> {
        match passwords::delete_generic_password(SERVICE, account) {
            Err(error) if error.code() != NOT_FOUND => {
                Err("macOS could not remove the OpenRouter connection from your keychain.".into())
            }
            _ => Ok(()),
        }
    }
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err(CANCELLED.into())
    } else {
        Ok(())
    }
}

pub(crate) fn client(timeout: Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(timeout)
        .user_agent("SonoJot")
        .build()
        .map_err(|_| "Cannot prepare the OpenRouter connection.".into())
}

/// OpenRouter's documented app attribution headers. Applied to OpenRouter
/// requests only; the recommendations endpoint is our own static file.
pub(crate) fn attribution(request: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
    request
        .header("HTTP-Referer", "https://sonojot.com")
        .header("X-OpenRouter-Title", "SonoJot")
}

/// Drop the network future when cancelled, including while waiting for headers.
async fn cancellable<T>(
    future: impl Future<Output = Result<T, String>>,
    cancel: &AtomicBool,
) -> Result<T, String> {
    let mut future = pin!(future);
    let mut ticks = tokio::time::interval(Duration::from_millis(100));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    poll_fn(|cx| {
        if cancel.load(Ordering::Acquire) {
            return Poll::Ready(Err(CANCELLED.into()));
        }
        // The first tick is immediately ready. Poll again until Pending so a
        // wakeup is registered even when the network future has no activity.
        while ticks.poll_tick(cx).is_ready() {}
        future.as_mut().poll(cx)
    })
    .await
}

/// Only a provider error payload that clearly names data policy may surface
/// the privacy-specific message; the status alone cannot diagnose it, and raw
/// provider text never becomes the error shown to the user.
fn http_error(status: reqwest::StatusCode, body: &[u8]) -> String {
    // Only routing failures (400/404) can become a privacy diagnosis, and only
    // when the provider payload itself names a data policy. Other statuses
    // keep their fixed meaning even when a body happens to mention privacy.
    if matches!(status.as_u16(), 400 | 404) {
        if serde_json::from_slice::<serde_json::Value>(body).ok().is_some_and(|value| {
            value.to_string().contains("paid-model-training-violation-by-account")
        }) {
            return PROOFREADER_ACCOUNT_PRIVACY.into();
        }
        let data_policy_named = serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|value| {
                value
                    .pointer("/error/message")
                    .or_else(|| value.get("message"))
                    .and_then(serde_json::Value::as_str)
                    .map(|message| message.to_lowercase())
            })
            .is_some_and(|message| {
                ["data collection", "data_collection", "data policy"]
                    .iter()
                    .any(|phrase| message.contains(phrase))
            });
        if data_policy_named {
            return "This model has no provider route under SonoJot's no-data-collection request. Choose another model or check your OpenRouter provider settings.".into();
        }
        return if status == reqwest::StatusCode::NOT_FOUND {
            "No provider route matched this model with the current request settings. Choose another model or check your OpenRouter provider preferences."
        } else {
            "The request was rejected. Check the selected model and your OpenRouter provider preferences."
        }.into();
    }
    match status.as_u16() {
        401 | 403 => "OpenRouter did not accept the connection. Connect again or check your account permissions.",
        402 => "OpenRouter needs more account credit before this model can run.",
        429 => "OpenRouter is rate limiting this account. Try again later.",
        503 => "The model's providers are temporarily unavailable. Try again later.",
        _ => "OpenRouter could not complete the request. Check the selected model and try again.",
    }.into()
}

pub(crate) const PROOFREADER_TIMEOUT: &str = "deadline_exceeded";
pub(crate) const PROOFREADER_STOPPED: &str = "stopped";
pub(crate) const PROOFREADER_ACCOUNT_PRIVACY: &str = "OpenRouter account privacy settings block this training-tier model. Choose another model or review https://openrouter.ai/settings/privacy.";

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
struct ProofreaderModelPolicy {
    supported_parameters: Vec<String>,
    training_required: bool,
}

fn supported_parameters(item: &serde_json::Value) -> Vec<String> {
    item.get("supported_parameters")
        .and_then(serde_json::Value::as_array)
        .into_iter().flatten()
        .filter_map(serde_json::Value::as_str)
        .filter(|parameter| valid_effort(parameter))
        .take(64).map(str::to_owned).collect()
}

/// Positive evidence requires a separate model notice. Missing metadata never
/// grants training permission: requests still default to data_collection=deny.
pub(crate) fn training_required(id: &str, item: &serde_json::Value) -> bool {
    if id.to_ascii_lowercase().split(['/', '-', ':', '_', '.'])
        .any(|part| part == "contributor" || part == "training")
    {
        return true;
    }
    for path in ["/training_required", "/trainingRequired", "/data_policy/training",
        "/data_policy/training_required", "/privacy/training", "/top_provider/training"]
    {
        if item.pointer(path).is_some_and(|value| value.as_bool() == Some(true)
            || matches!(value.as_str(), Some("allow" | "required" | "enabled")))
        {
            return true;
        }
    }
    for path in ["/data_collection", "/data_policy/data_collection", "/top_provider/data_collection"] {
        if item.pointer(path).and_then(serde_json::Value::as_str) == Some("allow") {
            return true;
        }
    }
    ["name", "description"].iter().any(|field| {
        let text = item.get(*field).and_then(serde_json::Value::as_str)
            .unwrap_or("").to_lowercase();
        ["contributor", "used for training", "used to train", "may train",
            "training on your", "training purposes", "train on prompts"]
            .iter().any(|phrase| text.contains(phrase))
    })
}

fn catalog_proofreader_policy(catalog: &serde_json::Value) -> HashMap<String, ProofreaderModelPolicy> {
    catalog.get("data").and_then(serde_json::Value::as_array)
        .into_iter().flatten().take(4000).filter_map(|item| {
            let id = item.get("id")?.as_str().filter(|id| valid_model(id))?;
            Some((id.to_owned(), ProofreaderModelPolicy {
                supported_parameters: supported_parameters(item),
                training_required: training_required(id, item),
            }))
        }).collect()
}

pub(crate) fn proofreader_cloud_accepted(settings: &crate::domain::Settings) -> bool {
    settings.component_acknowledgments.get("openrouter-cloud").map(String::as_str) == Some("1")
}

fn proofreader_training_accepted(settings: &crate::domain::Settings, model: &str, accepted: bool) -> bool {
    accepted && settings.proofreader_training_models.iter().any(|id| id == model)
}

pub(crate) fn proofreader_checkpoint(
    cancel: &AtomicBool,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    check_cancel(cancel)?;
    if stop.load(Ordering::Acquire) {
        return Err(PROOFREADER_STOPPED.into());
    }
    if Instant::now() >= deadline {
        return Err(PROOFREADER_TIMEOUT.into());
    }
    Ok(())
}

/// One deadline covers preparation, catalog refresh, headers and body. The
/// caller also bounds blocking store/key access outside this network future.
pub(crate) fn proofreader_network<T>(
    cancel: &AtomicBool,
    stop: &AtomicBool,
    deadline: Instant,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    proofreader_checkpoint(cancel, stop, deadline)?;
    let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
        .map_err(|_| "connection_unavailable".to_string())?;
    runtime.block_on(async {
        let mut future = pin!(future);
        let mut ticks = tokio::time::interval(Duration::from_millis(20));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let expiry = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline));
        let mut expiry = pin!(expiry);
        poll_fn(|cx| {
            if let Err(reason) = proofreader_checkpoint(cancel, stop, deadline) {
                return Poll::Ready(Err(reason));
            }
            if expiry.as_mut().poll(cx).is_ready() {
                return Poll::Ready(Err(PROOFREADER_TIMEOUT.into()));
            }
            while ticks.poll_tick(cx).is_ready() {}
            match future.as_mut().poll(cx) {
                Poll::Ready(result) => Poll::Ready(
                    proofreader_checkpoint(cancel, stop, deadline).and(result)),
                Poll::Pending => Poll::Pending,
            }
        }).await
    })
}

fn proofreader_request_body(
    model: &str,
    system: &str,
    text: &str,
    policy: &ProofreaderModelPolicy,
    exact_training_consent: bool,
) -> Result<serde_json::Value, String> {
    if policy.training_required && !exact_training_consent {
        return Err("training_consent_required".into());
    }
    let effort = policy.supported_parameters.iter()
        .any(|parameter| parameter == "reasoning")
        .then(|| "low".to_owned());
    let mut body = text_request_body(model, system, text, 8192, effort);
    body["temperature"] = serde_json::json!(0);
    if policy.training_required && exact_training_consent {
        body["provider"]["data_collection"] = serde_json::json!("allow");
    }
    Ok(body)
}

/// Dictation-only caller supplies the prompt and exact model snapshot. This
/// shares the connection, public catalog cache and bounded response reader.
pub(crate) fn proofread(
    store: &Store,
    model: &str,
    training_accepted: bool,
    system: &str,
    text: &str,
    cancel: &AtomicBool,
    stop: &AtomicBool,
    deadline: Instant,
) -> Result<serde_json::Value, String> {
    proofreader_checkpoint(cancel, stop, deadline)?;
    if !valid_model(model) {
        return Err("invalid_model".into());
    }
    let (key, exact_consent) = {
        let _guard = store.lock().map_err(|_| "settings_unavailable")?;
        proofreader_checkpoint(cancel, stop, deadline)?;
        let settings = store.settings_unlocked().map_err(|_| "settings_unavailable")?;
        if !proofreader_cloud_accepted(&settings) {
            return Err("cloud_consent_required".into());
        }
        let exact_consent = proofreader_training_accepted(&settings, model, training_accepted);
        let key = load_key_unlocked(store).map_err(|_| "connection_required")?;
        (key, exact_consent)
    };
    proofreader_checkpoint(cancel, stop, deadline)?;
    let cached = load_policy(store);
    proofreader_network(cancel, stop, deadline, async {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let client = client(remaining)?;
        let stale = cached.as_ref().is_none_or(|cache|
            now_unix().saturating_sub(cache.fetched_at) > POLICY_REFRESH_SECS
                || !cache.proofreader_models.contains_key(model));
        let mut policy = cached.as_ref().and_then(|cache| cache.proofreader_models.get(model)).cloned();
        if stale {
            // A failed lookup retains cached policy, or defaults to deny
            // routing without reasoning. Its latency consumes this deadline.
            if let Ok(catalog) = request_json(attribution(client.get(MODELS_URL)), 8 * 1024 * 1024).await {
                policy = catalog_proofreader_policy(&catalog).remove(model);
                proofreader_checkpoint(cancel, stop, deadline)?;
                cache_model_policy(store, &catalog);
            }
        }
        proofreader_checkpoint(cancel, stop, deadline)?;
        let mut policy = policy.unwrap_or_default();
        policy.training_required |= training_required(model, &serde_json::Value::Null);
        let body = proofreader_request_body(model, system, text, &policy, exact_consent)?;
        proofreader_checkpoint(cancel, stop, deadline)?;
        request_json(attribution(client.post(COMPLETIONS_URL)).bearer_auth(key).json(&body), 512 * 1024).await
    })
}

/// A failed public request keeps its HTTP status so callers can distinguish
/// "no such route" from "could not check". Messages stay fixed host text; raw
/// provider bodies never become error text.
pub(crate) struct FetchError {
    pub status: Option<u16>,
    pub message: String,
}

impl From<FetchError> for String {
    fn from(error: FetchError) -> String {
        error.message
    }
}

pub(crate) async fn fetch_json(
    request: reqwest::RequestBuilder,
    limit: usize,
) -> Result<serde_json::Value, FetchError> {
    let mut response = request.send().await.map_err(|error| FetchError {
        status: None,
        message: if error.is_timeout() {
            "OpenRouter timed out. Try again."
        } else {
            "Cannot reach OpenRouter. Check your connection."
        }
        .to_string(),
    })?;
    if !response.status().is_success() {
        let status = response.status();
        // Read a bounded slice of the error body only to classify the cause;
        // it is never shown or logged.
        let mut body = Vec::new();
        while let Ok(Some(chunk)) = response.chunk().await {
            let room = 8_192usize.saturating_sub(body.len());
            if room == 0 {
                break;
            }
            body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        }
        return Err(FetchError {
            status: Some(status.as_u16()),
            message: http_error(status, &body),
        });
    }
    let too_large = || FetchError {
        status: None,
        message: "OpenRouter returned more data than this operation can accept.".into(),
    };
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError {
        status: None,
        message: "The OpenRouter response was interrupted.".into(),
    })? {
        if chunk.len() > limit.saturating_sub(bytes.len()) {
            return Err(too_large());
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| FetchError {
        status: None,
        message: "OpenRouter returned an invalid response.".into(),
    })
}

pub(crate) async fn request_json(
    request: reqwest::RequestBuilder,
    limit: usize,
) -> Result<serde_json::Value, String> {
    fetch_json(request, limit).await.map_err(String::from)
}

pub(crate) fn network<T>(
    cancel: &AtomicBool,
    future: impl Future<Output = Result<T, String>>,
) -> Result<T, String> {
    check_cancel(cancel)?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "Cannot start the OpenRouter connection.".to_string())?;
    runtime.block_on(cancellable(future, cancel))
}

struct Pkce {
    verifier: String,
    /// Random listener path segment, sent to the public relay page so it can
    /// forward the code to the matching loopback listener.
    state: String,
}

impl Pkce {
    fn new(port: u16) -> Self {
        // UUID v4 uses the platform CSPRNG. Two independent UUIDs supply 244
        // random bits each; the verifier has 64 RFC 7636 unreserved characters.
        // The state is the listener port as 4 hex digits plus 64 random hex
        // characters: OpenRouter returns `state` but has dropped other relay
        // query parameters (seen 2026-10-10), so the relay reads the port here.
        Self {
            verifier: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
            state: format!(
                "{port:04x}{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        }
    }

    fn listener_path(&self) -> String {
        format!("/openrouter/{}", self.state)
    }

    fn url(&self, port: u16) -> Result<reqwest::Url, String> {
        // The relay page learns only the listener port and the single-use
        // state. The verifier stays in this process until the key exchange.
        let mut callback = reqwest::Url::parse(CALLBACK_PAGE_URL)
            .map_err(|_| "Cannot prepare OpenRouter sign-in.")?;
        callback
            .query_pairs_mut()
            .append_pair("port", &port.to_string())
            .append_pair("state", &self.state);
        let mut url =
            reqwest::Url::parse(AUTH_URL).map_err(|_| "Cannot prepare OpenRouter sign-in.")?;
        url.query_pairs_mut()
            .append_pair("callback_url", callback.as_str())
            .append_pair(
                "code_challenge",
                &URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes())),
            )
            .append_pair("code_challenge_method", "S256");
        Ok(url)
    }
}

fn open_browser(url: &reqwest::Url) -> Result<(), String> {
    if url.scheme() != "https" || url.host_str() != Some("openrouter.ai") || url.path() != "/auth" {
        return Err("The OpenRouter sign-in address is invalid.".into());
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let opener = std::env::var_os("SystemRoot")
            .map(std::path::PathBuf::from)
            .ok_or("Cannot locate the Windows browser opener.")?
            .join("System32/rundll32.exe");
        std::process::Command::new(opener)
            .arg("url.dll,FileProtocolHandler")
            .arg(url.as_str())
            .creation_flags(0x08000000)
            .spawn()
            .map_err(|_| "Cannot open your browser for OpenRouter sign-in.".to_string())?;
        Ok(())
    }
    #[cfg(target_os = "macos")]
    {
        let opened = std::process::Command::new("/usr/bin/open")
            .arg(url.as_str())
            .status()
            .map_err(|_| "Cannot open your browser for OpenRouter sign-in.".to_string())?;
        if !opened.success() {
            return Err("Cannot open your browser for OpenRouter sign-in.".into());
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let opened = std::process::Command::new("xdg-open")
            .arg(url.as_str())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .map_err(|_| "Cannot open your browser for OpenRouter sign-in.".to_string())?;
        if !opened.success() {
            return Err("Cannot open your browser for OpenRouter sign-in.".into());
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        supported()
    }
}

fn callback_code(request: &[u8], expected_path: &str, port: u16) -> Result<String, String> {
    if request.len() > MAX_CALLBACK_BYTES {
        return Err("Invalid sign-in callback.".into());
    }
    let text = std::str::from_utf8(request).map_err(|_| "Invalid sign-in callback.")?;
    if !text.ends_with("\r\n\r\n") {
        return Err("Incomplete sign-in callback.".into());
    }
    let mut lines = text.split("\r\n");
    let first = lines.next().ok_or("Invalid sign-in callback.")?;
    let parts: Vec<_> = first.split(' ').collect();
    if parts.len() != 3
        || parts[0] != "GET"
        || !matches!(parts[2], "HTTP/1.1" | "HTTP/1.0")
        || !parts[1].starts_with('/')
        || parts[1].starts_with("//")
    {
        return Err("Invalid sign-in callback.".into());
    }
    let target = reqwest::Url::parse(&format!("http://localhost:{port}{}", parts[1]))
        .map_err(|_| "Invalid sign-in callback.")?;
    if target.path() != expected_path || target.fragment().is_some() {
        return Err("This sign-in callback does not match the pending connection.".into());
    }
    let hosts: Vec<_> = lines
        .filter_map(|line| line.split_once(':'))
        .filter(|(name, _)| name.eq_ignore_ascii_case("host"))
        .map(|(_, value)| value.trim())
        .collect();
    if hosts != [format!("localhost:{port}")] {
        return Err("Invalid sign-in callback host.".into());
    }
    let codes: Vec<_> = target
        .query_pairs()
        .filter(|(name, _)| name == "code")
        .map(|(_, value)| value.into_owned())
        .collect();
    if codes.len() != 1 || !valid_secret(&codes[0]) {
        return Err("OpenRouter did not return a valid sign-in code.".into());
    }
    Ok(codes[0].clone())
}

fn callback_reply(stream: &mut TcpStream, success: bool) {
    let (status, body) = if success {
        ("200 OK", "<!doctype html><title>SonoJot</title><p>Sign-in received. Return to SonoJot to finish connecting.</p>")
    } else {
        ("400 Bad Request", "<!doctype html><title>SonoJot</title><p>This sign-in callback is invalid. Return to SonoJot and try again.</p>")
    };
    let response = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; frame-ancestors 'none'\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = stream.set_write_timeout(Some(Duration::from_millis(200)));
    let _ = stream.write_all(response.as_bytes());
}

fn receive_callback(
    listener: &TcpListener,
    pkce: &Pkce,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<String, String> {
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot inspect the sign-in listener.")?
        .port();
    listener
        .set_nonblocking(true)
        .map_err(|_| "Cannot prepare the sign-in listener.")?;
    while Instant::now() < deadline {
        check_cancel(cancel)?;
        match listener.accept() {
            Ok((mut stream, address)) => {
                if !address.ip().is_loopback() {
                    continue;
                }
                stream
                    .set_nonblocking(false)
                    .map_err(|_| "Cannot read the sign-in callback.")?;
                stream
                    .set_read_timeout(Some(Duration::from_millis(100)))
                    .map_err(|_| "Cannot read the sign-in callback.")?;
                let request_deadline = deadline.min(Instant::now() + Duration::from_secs(2));
                let mut request = Vec::new();
                while request.len() <= MAX_CALLBACK_BYTES && Instant::now() < request_deadline {
                    check_cancel(cancel)?;
                    let mut buffer = [0u8; 1024];
                    match stream.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(size) => {
                            request.extend_from_slice(&buffer[..size]);
                            if request.windows(4).any(|w| w == b"\r\n\r\n") {
                                break;
                            }
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                            ) =>
                        {
                            continue
                        }
                        Err(_) => break,
                    }
                }
                let code = callback_code(&request, &pkce.listener_path(), port);
                callback_reply(&mut stream, code.is_ok());
                // Stray browser favicon requests or unrelated loopback traffic
                // cannot consume the pending callback or end the login attempt.
                if let Ok(code) = code {
                    return Ok(code);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50))
            }
            Err(_) => return Err("The OpenRouter sign-in listener stopped.".into()),
        }
    }
    Err("OpenRouter sign-in timed out. Connect again when you are ready.".into())
}

pub fn connect(store: &Store, cancel: &AtomicBool) -> Result<AiSettingsStatus, String> {
    supported()?;
    check_cancel(cancel)?;
    let deadline = Instant::now() + AUTH_TIMEOUT;
    let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| "Cannot open the local OpenRouter sign-in callback.")?;
    let port = listener
        .local_addr()
        .map_err(|_| "Cannot inspect the sign-in callback.")?
        .port();
    let pkce = Pkce::new(port);
    open_browser(&pkce.url(port)?)?;
    let code = receive_callback(&listener, &pkce, cancel, deadline)?;
    drop(listener);
    let remaining = deadline.saturating_duration_since(Instant::now());
    if remaining.is_zero() {
        return Err("OpenRouter sign-in timed out. Connect again.".into());
    }
    let response = network(cancel, async {
        request_json(
            attribution(client(remaining.min(Duration::from_secs(30)))?.post(KEYS_URL))
                .json(&serde_json::json!({
                    "code": code, "code_verifier": pkce.verifier, "code_challenge_method": "S256"
                })),
            16_384,
        )
        .await
    })?;
    let key = response
        .get("key")
        .and_then(serde_json::Value::as_str)
        .filter(|key| valid_secret(key))
        .ok_or("OpenRouter returned an invalid connection key.")?;
    save_key(store, key, cancel)?;
    settings_status(store)
}

pub(crate) fn parse_models(value: serde_json::Value) -> Result<Vec<AiModel>, String> {
    let data = value
        .get("data")
        .and_then(serde_json::Value::as_array)
        .filter(|data| data.len() <= 4000)
        .ok_or("OpenRouter returned an invalid model list.")?;
    let mut models = Vec::new();
    for item in data {
        let Some(id) = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| valid_model(id))
        else {
            continue;
        };
        let has_text = |field: &str| {
            item.pointer(field)
                .and_then(serde_json::Value::as_array)
                .is_some_and(|items| items.iter().any(|v| v.as_str() == Some("text")))
        };
        let input_modalities: Vec<String> = item.pointer("/architecture/input_modalities")
            .and_then(serde_json::Value::as_array)
            .into_iter().flatten().filter_map(serde_json::Value::as_str)
            .filter(|m| m.len() <= 40).map(str::to_owned).collect();
        if !input_modalities.iter().any(|m| m == "text" || m == "audio")
            || !has_text("/architecture/output_modalities")
        {
            continue;
        }
        let price = |field: &str| {
            item.pointer(field)
                .and_then(serde_json::Value::as_str)
                .filter(|p| {
                    p.len() <= 40 && p.parse::<f64>().is_ok_and(|n| n.is_finite() && n >= 0.0)
                })
                .map(str::to_owned)
        };
        models.push(AiModel {
            supported_parameters: supported_parameters(item),
            training_required: training_required(id, item),
            input_modalities,
            id: id.into(),
            name: item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .filter(|n| !n.is_empty() && n.len() <= 240 && !n.chars().any(char::is_control))
                .unwrap_or(id)
                .into(),
            context_length: item
                .get("context_length")
                .and_then(serde_json::Value::as_u64)
                .unwrap_or_default(),
            prompt_price: price("/pricing/prompt"),
            completion_price: price("/pricing/completion"),
            description: None,
            reasoning_effort: None,
        });
    }
    models.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    models.dedup_by(|a, b| a.id == b.id);
    Ok(models)
}

struct RecommendedModel {
    id: String,
    description: Option<String>,
    /// OpenRouter `reasoning.effort` chosen for this model, if any.
    effort: Option<String>,
}

/// Effort names OpenRouter's unified `reasoning` parameter accepts, highest
/// first. Anything else in the recommendations document is ignored.
const REASONING_EFFORTS: [&str; 7] = ["max", "xhigh", "high", "medium", "low", "minimal", "none"];

/// The optional `reasoning: {"effort": …}` object on a recommendation. It is
/// an addition to version 1: 0.27/0.28 read only `id` and `description` and
/// ignore it. An unknown effort drops the hint, never the model.
fn recommended_effort(item: &serde_json::Value) -> Option<String> {
    item.pointer("/reasoning/effort")
        .and_then(serde_json::Value::as_str)
        .filter(|effort| REASONING_EFFORTS.contains(effort))
        .map(str::to_owned)
}

/// The website document is data only: model identifiers, optional short
/// curator notes and an optional reasoning effort from a fixed list. Entries
/// that look like URLs, markup or instructions are rejected, so a malformed
/// document cannot influence requests or the UI beyond those known values.
fn parse_recommendations(value: &serde_json::Value) -> Result<Vec<RecommendedModel>, String> {
    if value.get("version").and_then(serde_json::Value::as_u64) != Some(1) {
        return Err("unsupported recommendations version".into());
    }
    let models = value
        .get("models")
        .and_then(serde_json::Value::as_array)
        .filter(|models| !models.is_empty() && models.len() <= MAX_RECOMMENDED_MODELS)
        .ok_or("invalid recommendations list")?;
    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for item in models {
        let Some(id) = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| valid_model(id) && !id.ends_with(":free"))
        else {
            continue;
        };
        if !seen.insert(id.to_owned()) {
            continue;
        }
        let description = item
            .get("description")
            .and_then(serde_json::Value::as_str)
            .filter(|text| {
                !text.is_empty()
                    && text.len() <= MAX_DESCRIPTION_BYTES
                    && !text.contains("://")
                    && !text.contains('<')
                    && !text.contains('>')
                    && !text.chars().any(char::is_control)
            })
            .map(str::to_owned);
        result.push(RecommendedModel {
            id: id.into(),
            description,
            effort: recommended_effort(item),
        });
    }
    if result.is_empty() {
        return Err("recommendations list is empty".into());
    }
    Ok(result)
}

/// Offline copy of web/public/recommendations.json; update both together. The
/// bundled document goes through the same validation as the fetched one.
fn bundled_recommendations() -> Vec<RecommendedModel> {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../../src/brand/recommendations.json"
    ))
    .ok()
    .and_then(|value| parse_recommendations(&value).ok())
    .unwrap_or_default()
}

/// The offerable list is the curated document intersected with the live
/// catalog, in the document's order (the curator's ranking). Curated models
/// missing from the live catalog are never offered, and catalog models outside
/// the list are not shown.
fn curated_models(catalog: Vec<AiModel>, curated: Vec<RecommendedModel>) -> Vec<AiModel> {
    let mut by_id: HashMap<String, AiModel> =
        catalog.into_iter().map(|model| (model.id.clone(), model)).collect();
    curated
        .iter()
        .filter_map(|entry| {
            by_id.remove(&entry.id).map(|mut model| {
                model.description = entry.description.clone();
                model.reasoning_effort = entry.effort.clone();
                model
            })
        })
        .collect()
}

/// Reasoning metadata advertised by the public model catalog. A model that
/// thinks by default shares its output budget between reasoning and text.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct ModelReasoning {
    pub mandatory: bool,
    pub default_enabled: bool,
    pub default_effort: Option<String>,
    pub supported_efforts: Vec<String>,
    /// The top provider's output limit, which bounds the shared budget.
    pub max_completion_tokens: Option<u32>,
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
struct PolicyCache {
    version: u32,
    /// Unix seconds of the last catalog lookup attempt, success or failure.
    /// Marks attempts so a missing cache does not refetch on every call.
    #[serde(default)]
    fetched_at: i64,
    #[serde(default)]
    models: HashMap<String, ModelReasoning>,
    #[serde(default)]
    proofreader_models: HashMap<String, ProofreaderModelPolicy>,
    /// Curated efforts from the last recommendations read, by model id. None
    /// until a list has been read; the bundled list stands in until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recommended_efforts: Option<HashMap<String, String>>,
}

const POLICY_REFRESH_SECS: i64 = 6 * 3600;

fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|age| age.as_secs() as i64)
        .unwrap_or(0)
}

fn valid_effort(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 24
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn parse_reasoning(item: &serde_json::Value) -> Option<ModelReasoning> {
    let reasoning = item.get("reasoning")?;
    let efforts: Vec<String> = reasoning
        .get("supported_efforts")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .filter(|effort| valid_effort(effort))
                .take(8)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    let default_effort = reasoning
        .get("default_effort")
        .and_then(serde_json::Value::as_str)
        .filter(|effort| valid_effort(effort))
        .map(str::to_owned);
    Some(ModelReasoning {
        mandatory: reasoning
            .get("mandatory")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        default_enabled: reasoning
            .get("default_enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false),
        default_effort,
        supported_efforts: efforts,
        max_completion_tokens: item
            .pointer("/top_provider/max_completion_tokens")
            .and_then(serde_json::Value::as_u64)
            .filter(|tokens| *tokens > 0)
            .map(|tokens| tokens.min(u64::from(u32::MAX)) as u32),
    })
}

fn catalog_reasoning(catalog: &serde_json::Value) -> HashMap<String, ModelReasoning> {
    let mut models = HashMap::new();
    let Some(data) = catalog.get("data").and_then(serde_json::Value::as_array) else {
        return models;
    };
    for item in data {
        let Some(id) = item
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| valid_model(id))
        else {
            continue;
        };
        if let Some(reasoning) = parse_reasoning(item) {
            models.insert(id.to_owned(), reasoning);
        }
    }
    models
}

/// Cache only catalog reasoning policy, keyed by model id. This is public
/// metadata, never user text or credentials, so a missing or corrupt cache
/// just means no negotiation.
pub(crate) fn cache_model_policy(store: &Store, catalog: &serde_json::Value) {
    let recommended_efforts = load_policy(store).and_then(|cache| cache.recommended_efforts);
    write_policy(store, catalog, recommended_efforts);
}

fn write_policy(
    store: &Store,
    catalog: &serde_json::Value,
    recommended_efforts: Option<HashMap<String, String>>,
) {
    let _ = store.write(
        &store.root().join(POLICY_FILE),
        &PolicyCache {
            version: 1,
            fetched_at: now_unix(),
            models: catalog_reasoning(catalog),
            proofreader_models: catalog_proofreader_policy(catalog),
            recommended_efforts,
        },
    );
}

fn effort_map(curated: &[RecommendedModel]) -> HashMap<String, String> {
    curated
        .iter()
        .filter_map(|entry| Some((entry.id.clone(), entry.effort.clone()?)))
        .collect()
}

/// The curated effort for a model: the last recommendations read, else the
/// bundled copy. Models outside the list keep catalog negotiation.
fn curated_effort(store: &Store, model: &str) -> Option<String> {
    match load_policy(store).and_then(|cache| cache.recommended_efforts) {
        Some(efforts) => efforts.get(model).cloned(),
        None => effort_map(&bundled_recommendations()).remove(model),
    }
}

/// Approximate share of `max_tokens` OpenRouter gives reasoning per effort
/// (documented for Anthropic budgets; other providers think within the same
/// total). Unknown names count as the largest share.
fn effort_share(effort: &str) -> f64 {
    match effort {
        "none" => 0.0,
        "minimal" => 0.1,
        "low" => 0.2,
        "medium" => 0.5,
        "high" => 0.8,
        _ => 0.95,
    }
}

/// Shared output limit when no provider limit is published.
const REASONING_BUDGET_FALLBACK_CAP: u32 = 32_768;
/// OpenRouter caps reasoning budgets at 128k tokens.
const REASONING_BUDGET_CAP: u32 = 128_000;

/// A curated effort the model accepts: the same effort when the catalog lists
/// it (or lists nothing), otherwise the nearest listed one, preferring the
/// lower of two equally near. `none` is never sent to a model that requires
/// reasoning.
fn accepted_effort(effort: &str, reasoning: Option<&ModelReasoning>) -> Option<String> {
    let rank = |name: &str| REASONING_EFFORTS.iter().position(|e| *e == name);
    let wanted = rank(effort)?;
    let Some(reasoning) = reasoning else {
        // The catalog does not describe reasoning for this model; sending an
        // effort could be rejected, so leave the request unchanged.
        return None;
    };
    let listed: Vec<usize> = reasoning
        .supported_efforts
        .iter()
        .filter_map(|e| rank(e))
        .filter(|r| !(reasoning.mandatory && REASONING_EFFORTS[*r] == "none"))
        .collect();
    if reasoning.supported_efforts.is_empty() {
        return (!(reasoning.mandatory && effort == "none")).then(|| effort.to_owned());
    }
    listed
        .iter()
        .copied()
        .min_by_key(|r| (r.abs_diff(wanted), std::cmp::Reverse(*r)))
        .map(|r| REASONING_EFFORTS[r].to_owned())
}

/// The `max_tokens` and `reasoning.effort` a text request sends. A curated
/// effort keeps the caller's visible-output allowance after the documented
/// reasoning share; other models keep the low catalog-negotiated effort.
fn request_reasoning(
    curated: Option<&str>,
    reasoning: Option<&ModelReasoning>,
    max_tokens: u32,
) -> (u32, Option<String>) {
    if let Some(effort) = curated.and_then(|effort| accepted_effort(effort, reasoning)) {
        let share = effort_share(&effort);
        let cap = reasoning
            .and_then(|r| r.max_completion_tokens)
            .unwrap_or(REASONING_BUDGET_FALLBACK_CAP)
            .min(REASONING_BUDGET_CAP)
            .max(max_tokens);
        let wanted = (f64::from(max_tokens) / (1.0 - share).max(0.05)).ceil() as u32;
        let budget = if effort == "none" {
            max_tokens
        } else {
            wanted.max(REASONING_SHARED_BUDGET).min(cap)
        };
        return (budget, Some(effort));
    }
    let thinking = reasoning.filter(|r| r.mandatory || r.default_enabled);
    let budget = if thinking.is_some() {
        max_tokens.max(REASONING_SHARED_BUDGET)
    } else {
        max_tokens
    };
    (budget, thinking.and_then(negotiate_effort))
}

/// Chat completion body for a text request. Every call asks OpenRouter to
/// use only providers that do not collect data.
fn text_request_body(
    model: &str,
    system: &str,
    prompt: &str,
    budget: u32,
    effort: Option<String>,
) -> serde_json::Value {
    let mut body = serde_json::json!({
        "model": model,
        "messages": [{"role":"system","content":system},{"role":"user","content":prompt}],
        "max_tokens": budget,
        "stream": false,
        "provider": {"data_collection":"deny"}
    });
    if let Some(effort) = effort {
        body["reasoning"] = serde_json::json!({"effort": effort});
    }
    body
}

fn text_request(
    store: &Store,
    model: &str,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> serde_json::Value {
    let reasoning = model_reasoning_fresh(store, model, cancel);
    let curated = curated_effort(store, model);
    let (budget, effort) = request_reasoning(curated.as_deref(), reasoning.as_ref(), max_tokens);
    text_request_body(model, system, prompt, budget, effort)
}

/// Lowest advertised effort first keeps shared-budget thinking small. Efforts
/// are only sent when the catalog lists them; nothing is assumed about models
/// the catalog does not describe.
fn negotiate_effort(reasoning: &ModelReasoning) -> Option<String> {
    const PREFERENCE: [&str; 4] = ["low", "minimal", "medium", "high"];
    let listed = |effort: &String| reasoning.supported_efforts.iter().any(|e| e == effort);
    PREFERENCE
        .iter()
        .find(|effort| reasoning.supported_efforts.iter().any(|e| e.as_str() == **effort))
        .map(|effort| (*effort).to_owned())
        .or_else(|| reasoning.default_effort.clone().filter(|e| listed(e)))
        .or_else(|| reasoning.supported_efforts.last().cloned())
        .or_else(|| reasoning.default_effort.clone())
        .filter(|effort| {
            reasoning.supported_efforts.is_empty() || listed(effort)
        })
}

fn load_policy(store: &Store) -> Option<PolicyCache> {
    let path = store.root().join(POLICY_FILE);
    if !path.exists() {
        return None;
    }
    let cache: PolicyCache = store.read(&path).ok()?;
    (cache.version == 1).then_some(cache)
}

/// The request-level reasoning directive for a model, or None when the cached
/// catalog does not describe reasoning for it.
fn model_reasoning(store: &Store, model: &str) -> Option<ModelReasoning> {
    load_policy(store)?.models.get(model).cloned()
}

/// One bounded, cancellable public catalog GET refreshes the reasoning policy.
/// Failures still stamp the attempt so an unreachable catalog is retried at
/// most once per refresh window instead of on every generation call, and they
/// never fail the user's actual request.
fn refresh_model_policy(store: &Store, cancel: &AtomicBool) {
    let fetched = network(cancel, async {
        request_json(
            attribution(client(Duration::from_secs(15))?.get(MODELS_URL)),
            8 * 1024 * 1024,
        )
        .await
    });
    // A cancelled attempt does not stamp the cache; the next call retries.
    if matches!(&fetched, Err(message) if message.as_str() == CANCELLED) {
        return;
    }
    let mut cache = load_policy(store).unwrap_or(PolicyCache {
        version: 1,
        fetched_at: 0,
        models: HashMap::new(),
        proofreader_models: HashMap::new(),
        recommended_efforts: None,
    });
    if let Ok(catalog) = fetched {
        cache.models = catalog_reasoning(&catalog);
        cache.proofreader_models = catalog_proofreader_policy(&catalog);
    }
    cache.fetched_at = now_unix();
    let _ = store.write(&store.root().join(POLICY_FILE), &cache);
}

/// Reasoning metadata for the saved model. A missing or stale cache triggers
/// one bounded refresh; the lookup is advisory, so any failure leaves the
/// request exactly as it was before reasoning negotiation existed.
fn model_reasoning_fresh(store: &Store, model: &str, cancel: &AtomicBool) -> Option<ModelReasoning> {
    let stale = load_policy(store)
        .map(|cache| now_unix().saturating_sub(cache.fetched_at) > POLICY_REFRESH_SECS)
        .unwrap_or(true);
    if stale {
        refresh_model_policy(store, cancel);
    }
    model_reasoning(store, model)
}

/// Explicit model-list requests contain no dictation data or credentials; the
/// recommendations endpoint is a static file and receives nothing but a GET.
pub fn list_models(store: &Store, cancel: &AtomicBool) -> Result<Vec<AiModel>, String> {
    supported()?;
    let curated = network(cancel, async {
        let value = request_json(
            client(RECOMMENDATIONS_TIMEOUT)?.get(RECOMMENDATIONS_URL),
            MAX_RECOMMENDATIONS_BYTES,
        )
        .await?;
        parse_recommendations(&value)
    })
    .unwrap_or_else(|_| bundled_recommendations());
    let value = network(cancel, async {
        request_json(
            attribution(client(Duration::from_secs(20))?.get(MODELS_URL)),
            8 * 1024 * 1024,
        )
        .await
    })?;
    write_policy(store, &value, Some(effort_map(&curated)));
    Ok(curated_models(parse_models(value)?, curated))
}

/// Only a complete, single text choice is usable. Truncated, filtered and
/// tool-call answers stay rejected; the messages just name the category so a
/// budget or policy failure is not mistaken for malformed output.
fn completion_text(value: serde_json::Value) -> Result<String, String> {
    let choices = value
        .get("choices")
        .and_then(serde_json::Value::as_array)
        .filter(|choices| choices.len() == 1)
        .ok_or("OpenRouter returned an invalid completion.")?;
    let choice = &choices[0];
    if value.get("error").is_some() || choice.get("error").is_some() {
        return Err(
            "The model did not return a complete text response. Try again or choose another model."
                .into(),
        );
    }
    match choice
        .get("finish_reason")
        .and_then(serde_json::Value::as_str)
    {
        Some("stop") => {}
        Some("length") => {
            return Err(
                "The model used its output budget before finishing, possibly on reasoning. Try again or choose another model."
                    .into(),
            )
        }
        Some("content_filter") => {
            return Err(
                "The model declined to produce text for this request. Try different text or another model."
                    .into(),
            )
        }
        _ => {
            return Err(
                "The model did not return a complete text response. Try again or choose another model."
                    .into(),
            )
        }
    }
    if choice.pointer("/message/tool_calls").is_some_and(|calls| {
        !calls.is_null() && calls.as_array().is_none_or(|items| !items.is_empty())
    }) {
        return Err(
            "The model did not return a complete text response. Try again or choose another model."
                .into(),
        );
    }
    let text = choice
        .pointer("/message/content")
        .and_then(serde_json::Value::as_str)
        .filter(|text| !text.trim().is_empty() && text.len() <= MAX_OUTPUT_BYTES)
        .ok_or("The model returned an empty or oversized text response.")?;
    Ok(text.trim().to_owned())
}

/// Called from the existing blocking operation worker. No tools or recordings
/// are sent. Local work uses the same loaded runtime as conservative cleanup.
///
/// Single-shot legacy path only. Multi-chunk jobs (meeting notes/title,
/// vocabulary review) must snapshot once per job and use held calls, never
/// this per-request acquire, or the whole-job lease fails busy against itself.
#[allow(dead_code)]
pub fn generate(
    root: &Path,
    store: &Store,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    // Per-call snapshot so a Settings edit between the two file reads cannot
    // tear backend vs model within one request. Multi-chunk jobs must snapshot
    // once per job (see snapshot_notes_config) and reuse it for every chunk;
    // this wrapper only guarantees per-request atomicity for legacy callers.
    let snapshot = snapshot_notes_config(store)?;
    generate_with_snapshot(root, store, &snapshot, system, prompt, max_tokens, cancel)
}

/// Notes configuration captured once per job. Reusing one snapshot for every
/// chunk guarantees a Settings edit mid-job cannot switch the model/provider
/// between chunks. Cloud consent stays explicit: backend/model come only from
/// the saved AI settings, never from memory or auto-upload.
///
/// `text_acceleration` is snapshotted for the whole job for the same reason:
/// the cleanup server launch matches this preference (`server_matches`), so a
/// later chunk re-reading the live preference could switch CPU/GPU mid-job.
/// (The settings guard additionally rejects preference changes while inference
/// is active; the snapshot is the per-job half of that guarantee.)
#[derive(Clone, Debug)]
pub struct NotesSnapshot {
    pub backend: AiBackend,
    pub cloud_model: String,
    pub notes_model: String,
    pub text_acceleration: crate::domain::TextAcceleration,
}

pub fn snapshot_notes_config(store: &Store) -> Result<NotesSnapshot, String> {
    // Single store critical section: one locked read of both the AI settings
    // file and settings.json, so a concurrent commit cannot tear backend vs
    // model (or the acceleration preference) within one job. Every chunk of
    // the job reuses this snapshot; later chunks never re-read live settings.
    let _guard = store.lock()?;
    let ai = load_unlocked(store)?;
    let main = store.settings_unlocked()?;
    Ok(NotesSnapshot {
        backend: ai.backend,
        cloud_model: ai.model,
        notes_model: main.notes_model,
        text_acceleration: main.text_acceleration,
    })
}

/// Snapshot-aware generation. Local path requires the snapshotted notes model
/// to be installed and never falls back to an unrelated model; callers must
/// surface the error and preserve existing notes.
///
/// Single-request path. Whole-job callers must use held variants under one
/// outer lease instead.
#[allow(dead_code)]
pub fn generate_with_snapshot(
    root: &Path,
    store: &Store,
    snapshot: &NotesSnapshot,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    check_cancel(cancel)?;
    if system.len().saturating_add(prompt.len()) > 200_000 || !(1..=8192).contains(&max_tokens) {
        return Err("The requested AI operation exceeds its text or output limit.".into());
    }
    if snapshot.backend == AiBackend::Local {
        // No silent missing-model fallback: the snapshotted selection must be
        // ready. cleanup::resolve_generation_model (cleanup-owned) still falls
        // back, so check first and never call it with a missing preference.
        if !crate::cleanup::model_ready(root, &snapshot.notes_model) {
            return Err("The selected notes model is not installed. Choose an installed model in Setup before generating notes.".into());
        }
        // Atomic reservation + fit tied to the lease (Bug 1). No separate
        // pre-acquire admission: the waiter fails fast on proven OOM and
        // retries only on busy. Whole-job callers must use held variants
        // under one outer lease instead of this per-request path.
        static NEVER_STOP: AtomicBool = AtomicBool::new(false);
        crate::resource_manager::touch_activity(crate::resource_manager::Stage::Notes);
        crate::resource_manager::acquire_text_waiting_with_admission(
            "notes",
            &snapshot.notes_model,
            "notes",
            std::time::Duration::from_secs(30),
            cancel,
            &NEVER_STOP,
        )?;
        let result =
            generate_local_with_snapshot(root, snapshot, system, prompt, max_tokens, cancel);
        if result.is_ok() {
            crate::resource_manager::note_text_loaded(&snapshot.notes_model);
        } else if let Err(e) = &result {
            if e.contains("not installed") || e.contains("unavailable") {
                crate::resource_manager::note_text_load_failed();
            }
        }
        if crate::resource_manager::no_history_enabled() {
            // Owned clear under the still-held lease; never shutdown after.
            let _ = crate::cleanup_resource::end_job_report(true);
        }
        crate::resource_manager::notify_text_finished();
        return result;
    }
    // Cloud path below uses only the snapshotted model/consent.
    // Explicit cloud consent stays separate: OpenRouter requires a saved key
    // and a chosen model; local memory mode never auto-uploads.
    if snapshot.cloud_model.is_empty() {
        return Err("Choose an OpenRouter model before using it.".into());
    }
    if !valid_model(&snapshot.cloud_model) {
        return Err("Choose a text model from the OpenRouter model list.".into());
    }
    let key = {
        let _guard = store.lock()?;
        load_key_unlocked(store)?
    };
    // Catalog-described reasoning shares the output budget. Recommended models
    // use their curated effort with room for it; other models request the
    // lowest advertised effort under a bounded shared total instead of letting
    // default thinking silently consume the text budget. The catalog lookup is
    // advisory and must never fail an otherwise working request.
    let body = text_request(store, &snapshot.cloud_model, system, prompt, max_tokens, cancel);
    let value = network(cancel, async {
        request_json(
            attribution(client(Duration::from_secs(120))?.post(COMPLETIONS_URL))
                .bearer_auth(key)
                .json(&body),
            512 * 1024,
        )
        .await
    })?;
    check_cancel(cancel)?;
    completion_text(value)
}

/// Local generation when the caller already holds the whole-job text lease.
/// Same no-fallback rule as generate_with_snapshot, but skips acquire/notify
/// so multi-part jobs (vocabulary review, meeting notes) can hold one lease
/// across all chunks instead of acquiring per part.
pub fn generate_local_held(
    root: &Path,
    snapshot: &NotesSnapshot,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    check_cancel(cancel)?;
    if system.len().saturating_add(prompt.len()) > 200_000 || !(1..=8192).contains(&max_tokens) {
        return Err("The requested AI operation exceeds its text or output limit.".into());
    }
    if !crate::cleanup::model_ready(root, &snapshot.notes_model) {
        return Err("The selected notes model is not installed. Choose an installed model in Setup before generating notes.".into());
    }
    generate_local_with_snapshot(root, snapshot, system, prompt, max_tokens, cancel)
}

/// Cloud generation when the caller already holds no lease (cloud needs none).
/// Uses only the snapshotted model/consent; identical validation to the
/// per-request path.
pub fn generate_cloud_held(
    store: &Store,
    snapshot: &NotesSnapshot,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    check_cancel(cancel)?;
    if system.len().saturating_add(prompt.len()) > 200_000 || !(1..=8192).contains(&max_tokens) {
        return Err("The requested AI operation exceeds its text or output limit.".into());
    }
    if snapshot.cloud_model.is_empty() {
        return Err("Choose an OpenRouter model before using it.".into());
    }
    if !valid_model(&snapshot.cloud_model) {
        return Err("Choose a text model from the OpenRouter model list.".into());
    }
    let key = {
        let _guard = store.lock()?;
        load_key_unlocked(store)?
    };
    let body = text_request(store, &snapshot.cloud_model, system, prompt, max_tokens, cancel);
    let value = network(cancel, async {
        request_json(
            attribution(client(Duration::from_secs(120))?.post(COMPLETIONS_URL))
                .bearer_auth(key)
                .json(&body),
            512 * 1024,
        )
        .await
    })?;
    check_cancel(cancel)?;
    completion_text(value)
}

fn generate_local_with_snapshot(
    root: &Path,
    snapshot: &NotesSnapshot,
    system: &str,
    prompt: &str,
    max_tokens: u32,
    cancel: &AtomicBool,
) -> Result<String, String> {
    crate::cleanup::generate_local(
        root,
        &snapshot.notes_model,
        system,
        prompt,
        max_tokens,
        cancel,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proofreader_request_requires_exact_training_consent_and_catalog_reasoning() {
        let mut policy = ProofreaderModelPolicy::default();
        let body = proofreader_request_body("openai/gpt-6-luna", "system", "dictation", &policy, false).unwrap();
        assert_eq!(body["temperature"], 0);
        assert_eq!(body["provider"]["data_collection"], "deny");
        assert!(body.get("reasoning").is_none());
        assert_eq!(body["messages"][1], serde_json::json!({"role":"user", "content":"dictation"}));
        policy.supported_parameters.push("reasoning".into());
        let body = proofreader_request_body("openai/gpt-6-luna", "system", "dictation", &policy, false).unwrap();
        assert_eq!(body["reasoning"], serde_json::json!({"effort":"low"}));
        policy.training_required = true;
        assert_eq!(proofreader_request_body("other/contributor", "system", "dictation", &policy, false), Err("training_consent_required".into()));
        let body = proofreader_request_body("other/contributor", "system", "dictation", &policy, true).unwrap();
        assert_eq!(body["provider"]["data_collection"], "allow");
        let settings = crate::domain::Settings {
            proofreader_training_models: vec!["other/contributor".into()],
            ..Default::default()
        };
        assert!(proofreader_training_accepted(&settings, "other/contributor", true));
        assert!(!proofreader_training_accepted(&settings, "other/contributor", false));
        assert!(!proofreader_training_accepted(&settings, "other/contributor:free", true));
        assert!(!proofreader_training_accepted(&settings, "another/contributor", true));
        policy.training_required = false;
        assert_eq!(proofreader_request_body("other/model", "system", "dictation", &policy, true).unwrap()["provider"]["data_collection"], "deny");
    }

    #[test]
    fn training_detection_uses_catalog_metadata_and_defaults_to_deny() {
        assert!(training_required("meta/muse-spark-1.3-contributor", &serde_json::Value::Null));
        assert!(training_required("other/model", &serde_json::json!({"training_required":true})));
        assert!(training_required("other/model", &serde_json::json!({"data_policy":{"training":true}})));
        assert!(training_required("other/model", &serde_json::json!({"description":"Your prompts may be used to train the model."})));
        assert!(!training_required("other/model", &serde_json::Value::Null));
        assert!(!training_required("other/model", &serde_json::json!({"description":"Pretrained language model", "training_required":false})));
        let catalog = serde_json::json!({"data":[{
            "id":"other/model", "name":"Other", "supported_parameters":["reasoning","temperature"],
            "architecture":{"input_modalities":["text"],"output_modalities":["text"]},
            "training_required":true
        },{
            "id":"other/plain", "architecture":{"input_modalities":["text"],"output_modalities":["text"]}
        }]});
        let models = parse_models(catalog.clone()).unwrap();
        let model = models.iter().find(|model| model.id == "other/model").unwrap();
        let json = serde_json::to_value(model).unwrap();
        assert_eq!(json["trainingRequired"], true);
        assert_eq!(json["supportedParameters"], serde_json::json!(["reasoning","temperature"]));
        let plain = models.iter().find(|model| model.id == "other/plain").unwrap();
        assert!(!plain.training_required);
        assert!(plain.supported_parameters.is_empty());
        let path = std::env::temp_dir().join(format!("sonojot-proofreader-policy-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        cache_model_policy(&store, &catalog);
        let cache = load_policy(&store).unwrap();
        assert!(cache.proofreader_models["other/model"].training_required);
        assert_eq!(cache.proofreader_models["other/model"].supported_parameters, vec!["reasoning", "temperature"]);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn proofreader_cloud_notice_is_versioned_and_checked_before_key_or_network() {
        let path = std::env::temp_dir().join(format!("sonojot-proofreader-consent-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        let cancel = AtomicBool::new(false);
        let stop = AtomicBool::new(false);
        assert_eq!(proofread(&store, "openai/gpt-6-luna", false, "system", "text", &cancel, &stop,
            Instant::now() + Duration::from_secs(1)), Err("cloud_consent_required".into()));
        let mut settings = crate::domain::Settings::default();
        settings.component_acknowledgments.insert("openrouter-cloud".into(), "0".into());
        assert!(!proofreader_cloud_accepted(&settings));
        settings.component_acknowledgments.insert("openrouter-cloud".into(), "1".into());
        assert!(proofreader_cloud_accepted(&settings));
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn training_account_error_is_actionable_and_never_echoes_provider_text() {
        let error = http_error(reqwest::StatusCode::NOT_FOUND,
            br#"{"error":{"message":"private transcript and secret key","metadata":{"reasons":["paid-model-training-violation-by-account"]}}}"#);
        assert_eq!(error, PROOFREADER_ACCOUNT_PRIVACY);
        assert!(error.contains("https://openrouter.ai/settings/privacy"));
        assert!(!error.contains("private transcript"));
        assert!(!error.contains("secret key"));
        assert!(!http_error(reqwest::StatusCode::UNAUTHORIZED,
            br#"{"error":{"message":"paid-model-training-violation-by-account"}}"#).contains("settings/privacy"));
    }

    #[test]
    fn pkce_is_random_and_matches_rfc7636_challenge() {
        let first = Pkce::new(54321);
        let second = Pkce::new(54321);
        assert_eq!(first.verifier.len(), 64);
        assert_eq!(first.state.len(), 68);
        assert_eq!(&first.state[..4], "d431");
        assert!(first.state.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(first.verifier, second.verifier);
        assert_ne!(first.state, second.state);
        let known = Pkce {
            verifier: "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk".into(),
            state: first.state.clone(),
        };
        let url = known.url(54321).unwrap();
        let params: std::collections::HashMap<_, _> = url.query_pairs().collect();
        assert_eq!(
            params["code_challenge"],
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_eq!(params["code_challenge_method"], "S256");
        let callback = reqwest::Url::parse(&params["callback_url"]).unwrap();
        assert_eq!(callback.scheme(), "https");
        assert_eq!(callback.host_str(), Some("sonojot.com"));
        assert_eq!(callback.path(), "/connect/openrouter");
        let relay: std::collections::HashMap<_, _> = callback.query_pairs().collect();
        assert_eq!(relay["port"], "54321");
        assert_eq!(relay["state"], known.state.as_str());
        assert_eq!(
            known.listener_path(),
            format!("/openrouter/{}", known.state)
        );
        assert!(!url.as_str().contains(&known.verifier));
    }

    #[test]
    fn callback_requires_matching_path_host_and_one_bounded_code() {
        let request = |target: &str, host: &str| {
            format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n").into_bytes()
        };
        assert_eq!(
            callback_code(
                &request("/openrouter/state?code=valid-code", "localhost:50123"),
                "/openrouter/state",
                50123
            )
            .unwrap(),
            "valid-code"
        );
        for (target, host) in [
            ("/openrouter/wrong?code=valid", "localhost:50123"),
            ("/openrouter/state?code=valid", "example.com"),
            ("/openrouter/state?code=one&code=two", "localhost:50123"),
            ("/openrouter/state?code=%0a", "localhost:50123"),
            ("/openrouter/state?error=denied", "localhost:50123"),
            (
                "//example.com/openrouter/state?code=valid",
                "localhost:50123",
            ),
        ] {
            assert!(callback_code(&request(target, host), "/openrouter/state", 50123).is_err());
        }
        assert!(callback_code(
            &vec![b'a'; MAX_CALLBACK_BYTES + 1],
            "/openrouter/state",
            50123
        )
        .is_err());
    }

    #[test]
    fn incomplete_or_nontext_completions_are_rejected() {
        assert_eq!(completion_text(serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":" Text "}}]})).unwrap(), "Text");
        for value in [
            serde_json::json!({"choices":[]}),
            serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":"","tool_calls":[{}]}}]}),
            serde_json::json!({"choices":[{"finish_reason":"stop","message":{"content":null}}]}),
        ] {
            assert!(completion_text(value).is_err());
        }
    }

    #[test]
    fn http_errors_classify_privacy_only_from_the_payload() {
        let named = serde_json::to_string(&serde_json::json!({
            "error": {"message": "No endpoints found matching your data policy"}
        }))
        .unwrap();
        assert!(http_error(reqwest::StatusCode::NOT_FOUND, named.as_bytes())
            .contains("no-data-collection"));
        for body in [
            &b""[..],
            b"not json",
            br#"{"error":{"message":"model is not available"}}"#,
            br#"{"error":{"message":"privacy settings may apply"}}"#,
        ] {
            assert!(http_error(reqwest::StatusCode::NOT_FOUND, body)
                .contains("No provider route"));
        }
        assert!(http_error(reqwest::StatusCode::BAD_REQUEST, named.as_bytes())
            .contains("no-data-collection"));
        assert!(http_error(reqwest::StatusCode::BAD_REQUEST, b"")
            .contains("rejected"));
        // An auth failure mentioning privacy stays an auth failure.
        let auth = serde_json::to_string(&serde_json::json!({
            "error": {"message": "privacy provider list requires authentication"}
        }))
        .unwrap();
        assert!(http_error(reqwest::StatusCode::UNAUTHORIZED, auth.as_bytes())
            .contains("did not accept"));
        assert!(http_error(reqwest::StatusCode::SERVICE_UNAVAILABLE, b"")
            .contains("temporarily"));
    }

    #[test]
    fn completion_errors_distinguish_budget_refusal_and_incomplete() {
        let finish = |reason: &str| {
            serde_json::json!({"choices":[{"finish_reason":reason,"message":{"content":"partial"}}]})
        };
        assert!(completion_text(finish("length"))
            .unwrap_err()
            .contains("output budget"));
        assert!(completion_text(finish("content_filter"))
            .unwrap_err()
            .contains("declined"));
        assert!(completion_text(finish("tool_calls"))
            .unwrap_err()
            .contains("complete text"));
        assert!(completion_text(
            serde_json::json!({"error":{"code":400},"choices":[{"finish_reason":"stop","message":{"content":"x"}}]})
        )
        .is_err());
    }

    #[test]
    fn cached_reasoning_policy_negotiates_only_advertised_efforts() {
        let path =
            std::env::temp_dir().join(format!("sonojot-ai-policy-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        cache_model_policy(
            &store,
            &serde_json::json!({"data":[
                {"id":"z-ai/glm-5.3-flash","reasoning":{"mandatory":true,"default_enabled":true,"default_effort":"max","supported_efforts":["max","high","low"]}},
                {"id":"other/quiet","reasoning":{"mandatory":false,"default_enabled":false,"supported_efforts":["low"]}},
                {"id":"other/plain"}
            ]}),
        );
        let glm = model_reasoning(&store, "z-ai/glm-5.3-flash").unwrap();
        assert!(glm.mandatory && glm.default_enabled);
        assert_eq!(negotiate_effort(&glm).as_deref(), Some("low"));
        let quiet = model_reasoning(&store, "other/quiet").unwrap();
        assert!(!quiet.mandatory && !quiet.default_enabled);
        assert_eq!(negotiate_effort(&quiet).as_deref(), Some("low"));
        assert!(model_reasoning(&store, "other/plain").is_none());
        assert!(model_reasoning(&store, "missing/model").is_none());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn fresh_policy_cache_answers_without_a_catalog_lookup() {
        let path =
            std::env::temp_dir().join(format!("sonojot-ai-policy-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        store
            .write(
                &store.root().join(POLICY_FILE),
                &PolicyCache {
                    version: 1,
                    fetched_at: now_unix(),
                    models: HashMap::from([(
                        "a/reasoner".into(),
                        ModelReasoning {
                            mandatory: true,
                            default_enabled: true,
                            default_effort: Some("max".into()),
                            supported_efforts: vec!["max".into(), "low".into()],
                            max_completion_tokens: None,
                        },
                    )]),
                    recommended_efforts: None,
                    proofreader_models: HashMap::new(),
                },
            )
            .unwrap();
        let cancel = AtomicBool::new(false);
        // A fresh cache is answered without a catalog refresh; the advisory
        // lookup is only exercised on the stale path, which cannot run offline.
        let reasoning = model_reasoning_fresh(&store, "a/reasoner", &cancel).unwrap();
        assert_eq!(negotiate_effort(&reasoning).as_deref(), Some("low"));
        assert!(model_reasoning_fresh(&store, "unknown/model", &cancel).is_none());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn negotiate_effort_never_assumes_unlisted_efforts() {
        let fixed = |efforts: &[&str], default: Option<&str>| ModelReasoning {
            mandatory: true,
            default_enabled: true,
            default_effort: default.map(str::to_owned),
            supported_efforts: efforts.iter().map(|e| e.to_string()).collect(),
            max_completion_tokens: None,
        };
        assert_eq!(
            negotiate_effort(&fixed(&["max", "high"], Some("max"))).as_deref(),
            Some("high")
        );
        assert_eq!(
            negotiate_effort(&fixed(&["max"], Some("max"))).as_deref(),
            Some("max")
        );
        assert_eq!(
            negotiate_effort(&fixed(&[], Some("max"))).as_deref(),
            Some("max")
        );
        assert_eq!(
            negotiate_effort(&fixed(&[], None)),
            None
        );
        let odd = fixed(&["x-large"], None);
        assert_eq!(negotiate_effort(&odd).as_deref(), Some("x-large"));
    }

    /// Live OpenRouter probe for the GLM reasoning-budget failure. Run only on
    /// request: `cargo test openrouter_glm_probe -- --ignored --nocapture`.
    /// It uses the saved DPAPI key through the normal loader, sends a short
    /// synthetic vocabulary prompt, and prints only finish reason, token usage
    /// and error category — never the key, response text or dictation data.
    /// Set SONOJOT_PROBE_BATCH=1 to include one batch-variant call.
    #[test]
    #[ignore = "live probe; parent runs it explicitly"]
    fn openrouter_glm_probe_reports_budget_and_reasoning() {
        let store = Store::open(None).expect("open the app data store");
        let key = {
            let _guard = store.lock().expect("lock the store");
            load_key_unlocked(&store).expect("a saved OpenRouter key is required")
        };
        let system = "Return only a JSON array of at most 4 objects: {\"heard\":\"word\",\"written\":\"Word\",\"reason\":\"short reason\"}. Suggest name/acronym casing only. Return [] if none.";
        let prompt = "[\"we reviewed the grock model and the acme roadmap\"]";
        let request = |model: &str, tokens: u32, effort: Option<&str>| {
            let mut body = serde_json::json!({
                "model": model,
                "messages": [{"role":"system","content":system},{"role":"user","content":prompt}],
                "max_tokens": tokens,
                "stream": false,
                "provider": {"data_collection":"deny"}
            });
            if let Some(effort) = effort {
                body["reasoning"] = serde_json::json!({"effort": effort});
            }
            body
        };
        let cancel = AtomicBool::new(false);
        let mut variants = vec![
            (
                "normal-default-768",
                request("z-ai/glm-5.3-flash", 768, None),
            ),
            (
                "normal-low-8192",
                request("z-ai/glm-5.3-flash", 8192, Some("low")),
            ),
        ];
        if std::env::var_os("SONOJOT_PROBE_BATCH").is_some() {
            variants.push((
                "batch-low-8192",
                request("z-ai/glm-5.3-flash:batch", 8192, Some("low")),
            ));
        }
        for (label, body) in variants {
            let model = body["model"].as_str().unwrap_or("?");
            match network(&cancel, async {
                request_json(
                    attribution(client(Duration::from_secs(120))?.post(COMPLETIONS_URL))
                        .bearer_auth(&key)
                        .json(&body),
                    512 * 1024,
                )
                .await
            }) {
                Ok(value) => {
                    let usage = &value["usage"];
                    println!(
                        "probe {label} model={model} finish_reason={:?} prompt_tokens={:?} completion_tokens={:?} reasoning_tokens={:?} content_len={:?}",
                        value.pointer("/choices/0/finish_reason"),
                        usage.get("prompt_tokens"),
                        usage.get("completion_tokens"),
                        usage.pointer("/completion_tokens_details/reasoning_tokens"),
                        value
                            .pointer("/choices/0/message/content")
                            .and_then(serde_json::Value::as_str)
                            .map(str::len)
                    );
                }
                Err(message) => println!("probe {label} model={model} error={message}"),
            }
        }
    }

    #[test]
    fn text_model_catalog_preserves_prices_and_filters_other_outputs() {
        let model = |id: &str, output: &str| serde_json::json!({"id":id,"name":id,"architecture":{"input_modalities":["text"],"output_modalities":[output]},"pricing":{"prompt":"0.000001","completion":"0.000004"}});
        let values = parse_models(serde_json::json!({"data":[model("provider/text", "text"),model("provider/image", "image"),model("provider/text:online", "text")]})).unwrap();
        assert_eq!(values.len(), 1);
        assert_eq!(values[0].id, "provider/text");
        assert_eq!(values[0].prompt_price.as_deref(), Some("0.000001"));
    }

    #[test]
    fn speech_settings_load_old_files_and_catalog_exposes_audio_input() {
        let old: AiSettings = serde_json::from_value(serde_json::json!({"backend":"local","model":"provider/text"})).unwrap();
        assert!(old.speech_model.is_empty());
        assert_eq!(old.quick_models, vec![AiQuickModel {
            id: "provider/text".into(), role: AiQuickModelRole::Text,
        }]);
        let current = AiSettings { speech_model: "provider/audio".into(), ..old };
        let serialized = serde_json::to_value(&current).unwrap();
        assert_eq!(serialized["speechModel"], "provider/audio");
        assert_eq!(serialized["model"], "provider/text");
        assert!(validate_settings(&AiSettings { speech_model: "invalid".into(), ..current }).is_err());
        let models = parse_models(serde_json::json!({"data":[
            {"id":"provider/audio", "architecture":{"input_modalities":["audio"],"output_modalities":["text"]}},
            {"id":"provider/multimodal", "architecture":{"input_modalities":["text","audio"],"output_modalities":["text","audio"]}},
            {"id":"provider/tts", "architecture":{"input_modalities":["text"],"output_modalities":["audio"]}},
            {"id":"provider/unknown", "architecture":{"output_modalities":["text"]}}
        ]})).unwrap();
        assert_eq!(models.len(), 2);
        assert!(models.iter().all(|m| m.input_modalities.iter().any(|v| v == "audio")));
        assert_eq!(serde_json::to_value(&models[0]).unwrap()["inputModalities"], serde_json::json!(["audio"]));
    }

    #[test]
    fn recommendations_are_bounded_data_only() {
        let document = serde_json::json!({"version":1,"models":[
            {"id":"a/text","description":"Short note."},
            {"id":"b/text"},
            {"id":"a/text","description":"duplicate is dropped"},
            {"id":"missing-slash"},
            {"id":"https://example.invalid/x"},
            {"id":"a/text:online"},
            {"id":"a/text:free"},
            {"id":"c/text","description":"see https://example.invalid"},
            {"id":"d/text","description":"<b>markup</b>"},
            {"id":"e/text","description":"x".repeat(241)}
        ]});
        let parsed = parse_recommendations(&document).unwrap();
        assert_eq!(
            parsed.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["a/text", "b/text", "c/text", "d/text", "e/text"]
        );
        assert_eq!(parsed[0].description.as_deref(), Some("Short note."));
        assert!(parsed[1].description.is_none());
        assert!(parsed[2].description.is_none());
        assert!(parsed[3].description.is_none());
        assert!(parsed[4].description.is_none());
        for bad in [
            serde_json::json!({"version":2,"models":[{"id":"a/b"}]}),
            serde_json::json!({"version":1,"models":[]}),
            serde_json::json!({"version":1,"models":[{"id":"not-valid"}]}),
            serde_json::json!({"version":1,"models":"a/b"}),
            serde_json::json!([{"id":"a/b"}]),
        ] {
            assert!(parse_recommendations(&bad).is_err());
        }
        let oversized = serde_json::json!({"version":1,"models":
            vec![serde_json::json!({"id":"a/b"}); MAX_RECOMMENDED_MODELS + 1]});
        assert!(parse_recommendations(&oversized).is_err());
    }

    #[test]
    fn bundled_recommendations_parse_to_the_seeded_list() {
        let bundled = bundled_recommendations();
        assert!(bundled.len() >= 5);
        assert!(bundled.iter().all(|m| valid_model(&m.id)));
        let ids: Vec<_> = bundled.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "z-ai/glm-5.3-flash",
                "openai/gpt-6-luna",
                "anthropic/claude-haiku-5.5",
                "openai/gpt-6.1-sol",
                "anthropic/claude-sonnet-5.5",
                "google/gemini-3.8-flash",
                "x-ai/grok-4.7",
            ]
        );
        let efforts: Vec<_> = bundled.iter().map(|m| m.effort.as_deref()).collect();
        assert_eq!(
            efforts,
            [Some("high"), Some("xhigh"), Some("xhigh"), Some("high"), Some("medium"), Some("high"), Some("high")]
        );
        // The bundled copy and the website file must stay identical.
        assert_eq!(
            include_str!("../../src/brand/recommendations.json").replace("\r\n", "\n"),
            website_recommendations().replace("\r\n", "\n")
        );
    }

    /// The website copy lives outside `rewrite/`, so tests read it at run time
    /// instead of embedding it (the notices generator rejects embedded files
    /// outside `rewrite/`).
    fn website_recommendations() -> String {
        std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/public/recommendations.json"))
            .unwrap()
    }

    #[test]
    fn recommendations_keep_version_one_and_optional_reasoning() {
        // The 0.27/0.28 document shape still parses, with no effort.
        let old = serde_json::json!({"version":1,"models":[{"id":"a/text"},{"id":"b/text"}]});
        let parsed = parse_recommendations(&old).unwrap();
        assert!(parsed.iter().all(|m| m.effort.is_none()));
        // The new shape adds an optional reasoning object per model.
        let new = serde_json::json!({"version":1,"models":[
            {"id":"a/text","reasoning":{"effort":"xhigh"}},
            {"id":"b/text","reasoning":{"effort":"turbo"}},
            {"id":"c/text","reasoning":"high"},
            {"id":"d/text","reasoning":{"effort":"none"}},
            {"id":"e/text"}
        ]});
        let parsed = parse_recommendations(&new).unwrap();
        assert_eq!(
            parsed.iter().map(|m| m.effort.as_deref()).collect::<Vec<_>>(),
            [Some("xhigh"), None, None, Some("none"), None]
        );
        // What 0.27/0.28 read from the new document: the same ids, in order.
        // Their parser is this one without the effort field, which only reads
        // `version`, `models[].id` and `models[].description` through
        // serde_json::Value lookups, so extra keys are never an error.
        let released = |value: &serde_json::Value| -> Vec<String> {
            value["models"]
                .as_array()
                .unwrap()
                .iter()
                .filter_map(|item| item.get("id")?.as_str().map(str::to_owned))
                .collect()
        };
        let website: serde_json::Value =
            serde_json::from_str(&website_recommendations()).unwrap();
        assert_eq!(website["version"], 1);
        assert_eq!(
            released(&website),
            parse_recommendations(&website).unwrap().into_iter().map(|m| m.id).collect::<Vec<_>>()
        );
    }

    #[test]
    fn curated_models_follow_document_order_and_carry_effort() {
        let model = |id: &str, prompt: Option<&str>, completion: Option<&str>| AiModel {
            supported_parameters: Vec::new(),
            training_required: false,
            input_modalities: vec!["text".into()],
            id: id.into(),
            name: id.into(),
            context_length: 0,
            prompt_price: prompt.map(str::to_owned),
            completion_price: completion.map(str::to_owned),
            description: None,
            reasoning_effort: None,
        };
        let catalog = vec![
            model("mid/model", Some("0.000002"), Some("0.00001")),
            model("cheap/model", Some("0.00000015"), Some("0.0000005")),
            model("unlisted/model", None, None),
            model("other/model", Some("0"), Some("0")),
        ];
        let curated = vec![
            RecommendedModel { id: "gone/model".into(), description: None, effort: None },
            RecommendedModel { id: "mid/model".into(), description: Some("note".into()), effort: Some("high".into()) },
            RecommendedModel { id: "unlisted/model".into(), description: None, effort: None },
            RecommendedModel { id: "cheap/model".into(), description: None, effort: None },
        ];
        let result = curated_models(catalog, curated);
        assert_eq!(
            result.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["mid/model", "unlisted/model", "cheap/model"]
        );
        assert_eq!(result[0].description.as_deref(), Some("note"));
        assert_eq!(result[0].reasoning_effort.as_deref(), Some("high"));
        assert_eq!(serde_json::to_value(&result[0]).unwrap()["reasoningEffort"], "high");
        assert!(result[1].reasoning_effort.is_none());
    }

    fn reasoning(efforts: &[&str], mandatory: bool, limit: Option<u32>) -> ModelReasoning {
        ModelReasoning {
            mandatory,
            default_enabled: mandatory,
            default_effort: Some("medium".into()),
            supported_efforts: efforts.iter().map(|e| e.to_string()).collect(),
            max_completion_tokens: limit,
        }
    }

    #[test]
    fn curated_effort_maps_to_listed_efforts() {
        let all = ["max", "xhigh", "high", "medium", "low"];
        assert_eq!(accepted_effort("xhigh", Some(&reasoning(&all, true, None))).as_deref(), Some("xhigh"));
        // Gemini-style lists without xhigh fall to the nearest listed effort.
        let gemini = ["high", "medium", "low"];
        assert_eq!(accepted_effort("xhigh", Some(&reasoning(&gemini, true, None))).as_deref(), Some("high"));
        // Equal distance prefers the lower effort.
        let gap = ["max", "high", "low"];
        assert_eq!(accepted_effort("medium", Some(&reasoning(&gap, true, None))).as_deref(), Some("low"));
        assert_eq!(accepted_effort("high", Some(&reasoning(&gap, true, None))).as_deref(), Some("high"));
        // `none` is never sent to a model that requires reasoning.
        assert_eq!(accepted_effort("none", Some(&reasoning(&["none", "low"], true, None))).as_deref(), Some("low"));
        assert_eq!(accepted_effort("none", Some(&reasoning(&[], true, None))), None);
        // Without catalog reasoning metadata nothing is sent.
        assert_eq!(accepted_effort("high", None), None);
        assert_eq!(accepted_effort("turbo", Some(&reasoning(&all, true, None))), None);
    }

    #[test]
    fn request_reasoning_budgets_curated_and_negotiated_efforts() {
        let all = ["max", "xhigh", "high", "medium", "low"];
        // Curated xhigh keeps 1024 visible tokens after a 95% reasoning share.
        let (budget, effort) = request_reasoning(Some("xhigh"), Some(&reasoning(&all, false, Some(128_000))), 1024);
        assert_eq!(effort.as_deref(), Some("xhigh"));
        assert_eq!(budget, 20_480);
        // Small requests still get the shared floor.
        let (budget, effort) = request_reasoning(Some("high"), Some(&reasoning(&all, true, Some(65_536))), 48);
        assert_eq!((budget, effort.as_deref()), (REASONING_SHARED_BUDGET, Some("high")));
        // The provider's output limit caps the budget.
        let (budget, _) = request_reasoning(Some("xhigh"), Some(&reasoning(&all, true, Some(16_000))), 4096);
        assert_eq!(budget, 16_000);
        // Unknown limits use the fallback cap.
        let (budget, _) = request_reasoning(Some("max"), Some(&reasoning(&all, true, None)), 8192);
        assert_eq!(budget, REASONING_BUDGET_FALLBACK_CAP);
        // A model outside the list keeps the low negotiated effort.
        let (budget, effort) = request_reasoning(None, Some(&reasoning(&all, true, None)), 1024);
        assert_eq!((budget, effort.as_deref()), (REASONING_SHARED_BUDGET, Some("low")));
        // Optional reasoning that is off by default stays off without a curated effort.
        let (budget, effort) = request_reasoning(None, Some(&reasoning(&all, false, None)), 1024);
        assert_eq!((budget, effort), (1024, None));
        // No catalog metadata: the request is unchanged even for a curated model.
        assert_eq!(request_reasoning(Some("high"), None, 1024), (1024, None));
    }

    #[test]
    fn text_requests_send_curated_effort_and_deny_data_collection() {
        let path =
            std::env::temp_dir().join(format!("sonojot-ai-effort-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        let catalog = serde_json::json!({"data":[
            {"id":"curated/model","reasoning":{"mandatory":false,"supported_efforts":["xhigh","high","low"]},
             "top_provider":{"max_completion_tokens":128000}},
            {"id":"manual/model","reasoning":{"mandatory":true,"default_enabled":true,"supported_efforts":["high","low"]}}
        ]});
        let curated = parse_recommendations(&serde_json::json!({"version":1,"models":[
            {"id":"curated/model","reasoning":{"effort":"xhigh"}}
        ]}))
        .unwrap();
        write_policy(&store, &catalog, Some(effort_map(&curated)));
        let cancel = AtomicBool::new(false);
        let body = text_request(&store, "curated/model", "system", "prompt", 1024, &cancel);
        assert_eq!(body["reasoning"], serde_json::json!({"effort":"xhigh"}));
        assert_eq!(body["max_tokens"], 20_480);
        assert_eq!(body["provider"]["data_collection"], "deny");
        // A model the user added from the full catalog keeps negotiation.
        let body = text_request(&store, "manual/model", "system", "prompt", 1024, &cancel);
        assert_eq!(body["reasoning"], serde_json::json!({"effort":"low"}));
        assert_eq!(body["provider"]["data_collection"], "deny");
        // A later catalog refresh keeps the curated efforts.
        cache_model_policy(&store, &catalog);
        assert_eq!(curated_effort(&store, "curated/model").as_deref(), Some("xhigh"));
        assert_eq!(curated_effort(&store, "manual/model"), None);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn curated_effort_falls_back_to_the_bundled_list() {
        let path =
            std::env::temp_dir().join(format!("sonojot-ai-bundled-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        assert_eq!(curated_effort(&store, "openai/gpt-6-luna").as_deref(), Some("xhigh"));
        assert_eq!(curated_effort(&store, "unknown/model"), None);
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn settings_default_local_and_roundtrip_without_credentials() {
        let path =
            std::env::temp_dir().join(format!("sonojot-ai-settings-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        assert_eq!(load_settings(&store).unwrap(), AiSettings::default());
        let settings = AiSettings {
            speech_model: "provider/speech".into(),
            backend: AiBackend::Local,
            model: "provider/model".into(),
            quick_models: vec![
                AiQuickModel { id: "provider/model".into(), role: AiQuickModelRole::Text },
                AiQuickModel { id: "provider/speech".into(), role: AiQuickModelRole::Speech },
            ],
        };
        let status = save_settings(&store, &settings).unwrap();
        assert_eq!(status.quick_models, settings.quick_models);
        let serialized = serde_json::to_value(&settings).unwrap();
        assert_eq!(serialized["quickModels"], serde_json::json!([
            {"id": "provider/model", "role": "text"},
            {"id": "provider/speech", "role": "speech"},
        ]));
        assert_eq!(serde_json::to_value(status).unwrap()["quickModels"], serialized["quickModels"]);
        assert_eq!(serde_json::from_value::<AiSettings>(serialized).unwrap(), settings);
        assert_eq!(load_settings(&store).unwrap(), settings);
        assert!(!settings_status(&store).unwrap().connected);
        assert!(!store.root().join(KEY_FILE).exists());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn quick_models_migrate_missing_field_without_changing_selected_models() {
        let path = std::env::temp_dir().join(format!("sonojot-ai-migration-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        for backend in [AiBackend::Local, AiBackend::OpenRouter] {
            for (model, speech_model, expected) in [
                ("", "", serde_json::json!([])),
                ("provider/text", "", serde_json::json!([
                    {"id": "provider/text", "role": "text"},
                ])),
                ("", "provider/speech", serde_json::json!([
                    {"id": "provider/speech", "role": "speech"},
                ])),
                ("provider/multimodal", "provider/multimodal", serde_json::json!([
                    {"id": "provider/multimodal", "role": "text"},
                    {"id": "provider/multimodal", "role": "speech"},
                ])),
            ] {
                let legacy = serde_json::json!({
                    "backend": backend, "model": model, "speechModel": speech_model,
                });
                let migrated: AiSettings = serde_json::from_value(legacy.clone()).unwrap();
                assert_eq!(migrated.backend, backend);
                assert_eq!(migrated.model, model);
                assert_eq!(migrated.speech_model, speech_model);
                assert_eq!(serde_json::to_value(&migrated.quick_models).unwrap(), expected);
                if backend == AiBackend::Local {
                    store.write(&store.root().join(SETTINGS_FILE), &legacy).unwrap();
                    assert_eq!(load_settings(&store).unwrap(), migrated);
                    assert_eq!(settings_status(&store).unwrap().quick_models, migrated.quick_models);
                    save_settings(&store, &migrated).unwrap();
                    let saved: serde_json::Value = store.read(&store.root().join(SETTINGS_FILE)).unwrap();
                    assert_eq!(saved["quickModels"], expected);
                    assert_eq!(load_settings(&store).unwrap(), migrated);
                }
            }
        }
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn explicit_empty_quick_models_stay_empty_across_saves_and_reads() {
        let path = std::env::temp_dir().join(format!("sonojot-ai-empty-list-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        let settings: AiSettings = serde_json::from_value(serde_json::json!({
            "backend": "local", "model": "provider/text", "speechModel": "provider/speech",
            "quickModels": [],
        })).unwrap();
        assert!(settings.quick_models.is_empty());
        for _ in 0..2 {
            assert!(save_settings(&store, &settings).unwrap().quick_models.is_empty());
            assert_eq!(load_settings(&store).unwrap(), settings);
        }
        drop(store);
        let reopened = Store::open(Some(path.clone())).unwrap();
        assert_eq!(load_settings(&reopened).unwrap(), settings);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn quick_models_reject_invalid_shapes_roles_and_unknown_fields() {
        for quick_models in [
            serde_json::Value::Null,
            serde_json::json!({"id": "provider/text", "role": "text"}),
            serde_json::json!([{"id": "provider/text", "role": "cleanup"}]),
            serde_json::json!([{"id": "provider/text", "role": "Text"}]),
            serde_json::json!([{"id": "provider/text", "role": null}]),
            serde_json::json!([{"id": "provider/text"}]),
            serde_json::json!([{"role": "text"}]),
            serde_json::json!([{"id": 42, "role": "text"}]),
            serde_json::json!([{"id": "provider/text", "role": "text", "extra": true}]),
        ] {
            assert!(serde_json::from_value::<AiSettings>(serde_json::json!({
                "backend": "local", "model": "", "quickModels": quick_models,
            })).is_err(), "accepted invalid quickModels: {quick_models}");
        }
        assert!(serde_json::from_value::<AiSettings>(serde_json::json!({
            "backend": "local", "model": "", "quickModels": [], "extra": true,
        })).is_err());
    }

    #[test]
    fn quick_model_validation_rejects_bad_ids_duplicates_and_oversized_lists() {
        let path = std::env::temp_dir().join(format!("sonojot-ai-invalid-list-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        let mut invalid_lists: Vec<Vec<AiQuickModel>> = [
            "".to_owned(),
            "missing-slash".into(),
            "https://example.invalid/model".into(),
            "provider/text:online".into(),
            "provider/with spaces".into(),
            "provider/text\n".into(),
            format!("provider/{}", "x".repeat(200)),
        ].into_iter().flat_map(|id| {
            [AiQuickModelRole::Text, AiQuickModelRole::Speech]
                .map(|role| vec![AiQuickModel { id: id.clone(), role }])
        }).collect();
        invalid_lists.push(vec![AiQuickModel { id: "provider/text".into(), role: AiQuickModelRole::Text }; 2]);
        invalid_lists.push((0..=MAX_QUICK_MODELS).map(|index| AiQuickModel {
            id: format!("provider/model-{index}"), role: AiQuickModelRole::Text,
        }).collect());
        for quick_models in invalid_lists {
            let invalid = AiSettings { quick_models, ..AiSettings::default() };
            save_settings(&store, &AiSettings::default()).unwrap();
            assert!(save_settings(&store, &invalid).is_err());
            assert_eq!(load_settings(&store).unwrap(), AiSettings::default());
            store.write(&store.root().join(SETTINGS_FILE), &invalid).unwrap();
            assert!(load_settings(&store).is_err());
            assert!(settings_status(&store).is_err());
        }
        let mut settings = AiSettings {
            quick_models: (0..MAX_QUICK_MODELS).map(|index| AiQuickModel {
                id: format!("provider/model-{index}"), role: AiQuickModelRole::Text,
            }).collect(),
            ..AiSettings::default()
        };
        save_settings(&store, &settings).unwrap();
        assert_eq!(load_settings(&store).unwrap(), settings);
        // A model that accepts both modalities can be listed once for each role.
        settings.quick_models = vec![
            AiQuickModel { id: "provider/multimodal".into(), role: AiQuickModelRole::Text },
            AiQuickModel { id: "provider/multimodal".into(), role: AiQuickModelRole::Speech },
        ];
        save_settings(&store, &settings).unwrap();
        assert_eq!(load_settings(&store).unwrap(), settings);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn removing_selected_quick_models_accepts_local_backend_and_empty_speech_model() {
        let path = std::env::temp_dir().join(format!("sonojot-ai-remove-models-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        save_key(&store, "synthetic-test-key-never-used-for-network", &AtomicBool::new(false)).unwrap();
        let mut settings: AiSettings = serde_json::from_value(serde_json::json!({
            "backend": "openrouter", "model": "provider/text", "speechModel": "provider/speech",
        })).unwrap();
        save_settings(&store, &settings).unwrap();
        settings.backend = AiBackend::Local;
        settings.speech_model.clear();
        settings.quick_models.clear();
        let status = save_settings(&store, &settings).unwrap();
        assert_eq!(status.backend, AiBackend::Local);
        assert_eq!(status.model, "provider/text");
        assert!(status.speech_model.is_empty());
        assert!(status.quick_models.is_empty());
        assert!(status.connected);
        assert_eq!(load_settings(&store).unwrap(), settings);
        disconnect(&store).unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn saved_key_roundtrip_stores_no_plaintext_and_disconnects_locally() {
        let path = std::env::temp_dir().join(format!("sonojot-ai-dpapi-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        let fake = "synthetic-test-key-never-used-for-network";
        save_key(&store, fake, &AtomicBool::new(false)).unwrap();
        let persisted = std::fs::read_to_string(store.root().join(KEY_FILE)).unwrap();
        assert!(!persisted.contains(fake));
        assert_eq!(load_key_unlocked(&store).unwrap(), fake);
        save_settings(
            &store,
            &AiSettings {
                speech_model: String::new(),
                backend: AiBackend::OpenRouter,
                model: "provider/model".into(),
                quick_models: vec![AiQuickModel { id: "provider/model".into(), role: AiQuickModelRole::Text }],
            },
        )
        .unwrap();
        let status = disconnect(&store).unwrap();
        assert_eq!(status.backend, AiBackend::Local);
        assert_eq!(status.quick_models, vec![AiQuickModel { id: "provider/model".into(), role: AiQuickModelRole::Text }]);
        assert!(!status.connected);
        assert!(!store.root().join(KEY_FILE).exists());
        drop(store);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[cfg(any(windows, target_os = "macos"))]
    #[test]
    fn meeting_speech_snapshot_is_independent_of_text_settings_and_later_edits() {
        let _gate = crate::resource_manager::tests::test_gate();
        let path = std::env::temp_dir().join(format!("sonojot-speech-snapshot-{}", uuid::Uuid::new_v4()));
        let store = Store::open(Some(path.clone())).unwrap();
        save_key(&store, "synthetic-speech-test-key-no-network", &AtomicBool::new(false)).unwrap();
        save_settings(&store, &AiSettings {
            backend: AiBackend::Local, model: "provider/text".into(), speech_model: "provider/audio-first".into(),
            quick_models: Vec::new(),
        }).unwrap();
        let meetings = crate::meetings::Meetings::new(store.clone());
        let detail = meetings.create_recording("Snapshot", "microphone").unwrap();
        let audio = meetings.recording_paths(&detail.id).unwrap().mixed;
        let mut writer = crate::recording::PcmWriter::create_at_rate(&audio, 16000).unwrap();
        writer.write_samples(&[0.0; 160]).unwrap();
        writer.checkpoint().unwrap();
        drop(writer);
        meetings.finish_recording(&detail.id, 0.01).unwrap();
        let job = meetings.begin_transcription(&detail.id, Some("openrouter")).unwrap();
        save_settings(&store, &AiSettings {
            backend: AiBackend::Local, model: "provider/text-other".into(), speech_model: "provider/audio-next".into(),
            quick_models: Vec::new(),
        }).unwrap();
        assert_eq!(job.asr_model(), "openrouter:provider/audio-first");
        meetings.cancel(&detail.id).unwrap();
        assert!(job.run_recognition(|_, _, _| panic!("remote must not call local ASR")).is_err());
        assert_eq!(meetings.get(&detail.id).unwrap().status, "cancelled");
        disconnect(&store).unwrap();
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn cancellation_drops_a_pending_network_future() {
        let cancelled = std::sync::Arc::new(AtomicBool::new(false));
        let trigger = cancelled.clone();
        let started = Instant::now();
        let worker = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(25));
            trigger.store(true, Ordering::Release);
        });
        assert_eq!(
            network(&cancelled, std::future::pending::<Result<(), String>>()).unwrap_err(),
            CANCELLED
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        worker.join().unwrap();
    }
}
