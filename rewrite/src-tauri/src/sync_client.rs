use reqwest::{Client, Method, Url};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use zeroize::Zeroizing;
// The private build supplies this value. Public source has no service default.
pub const STAGING: &str = match option_env!("SONOJOT_SYNC_ORIGIN") {
    Some(origin) => origin,
    None => "",
};

pub const AUTH_ORIGIN: &str = match option_env!("SONOJOT_AUTH_ORIGIN") {
    Some(origin) => origin,
    None => "",
};

pub fn validate_staging_origin(origin: &str) -> Result<(), String> {
    if origin.is_empty() {
        return Err(
            "Sync is not configured in this build. Install an official nightly or Sync Test build."
                .into(),
        );
    }
    let valid = Url::parse(origin).is_ok_and(|url| {
        let host = url.host_str().unwrap_or_default();
        let labels: Vec<_> = host.split('.').collect();
        url.scheme() == "https"
            && labels.len() == 4
            && labels[0] == "sonojot-sync-staging"
            && !labels[1].is_empty()
            && labels[2..] == ["workers", "dev"]
            && url.username().is_empty()
            && url.password().is_none()
            && url.port().is_none()
            && url.path() == "/"
            && url.query().is_none()
            && url.fragment().is_none()
            && origin == url.origin().ascii_serialization()
    });
    if valid {
        Ok(())
    } else {
        Err("This build requires an HTTPS staging sync origin configured at build time.".into())
    }
}
#[cfg(feature = "synctest")]
pub static TEST_OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
pub const RATE_LIMITED: &str = "Too many sync attempts. Try again later.";
pub(crate) fn rate_limit_delay(error: &str) -> Option<u64> {
    if error == RATE_LIMITED {
        return Some(61);
    }
    error
        .strip_prefix(RATE_LIMITED)?
        .strip_prefix(" Retry in ")?
        .strip_suffix(" seconds.")?
        .parse::<u64>()
        .ok()
        .map(|n| n.clamp(1, 86400))
}
fn retry_after(value: &str) -> Option<u64> {
    value.parse::<u64>().ok().or_else(|| {
        time::OffsetDateTime::parse(value, &time::format_description::well_known::Rfc2822)
            .ok().map(|date| date.unix_timestamp()
                .saturating_sub(time::OffsetDateTime::now_utc().unix_timestamp()).max(0) as u64)
    }).map(|seconds| seconds.clamp(1, 86400))
}
#[derive(Debug)]
pub struct Failure {
    pub status: u16,
    pub message: String,
}
impl From<Failure> for String {
    fn from(e: Failure) -> String {
        e.message
    }
}
#[derive(Clone)]
pub struct SyncClient {
    client: Client,
    origin: String,
    token: Zeroizing<String>,
    proof: Option<Zeroizing<String>>,
    rotation: bool,
    #[cfg(feature = "synctest")]
    test_token: Option<Zeroizing<String>>,
}
#[derive(Serialize, Deserialize, Clone)]
pub struct Wrap {
    pub kind: String,
    #[serde(rename = "ref")]
    pub reference: String,
    pub ciphertext: String,
    pub rev: u64,
}
impl SyncClient {
    pub fn new(origin: &str, token: &str) -> Result<Self, String> {
        let url = Url::parse(origin).map_err(|_| "Enter a valid sync server address.")?;
        let local = cfg!(test) && url.scheme() == "http" && url.host_str() == Some("127.0.0.1");
        if !local {
            crate::flavor::require_sync()?;
            if origin.trim_end_matches('/') != STAGING {
                return Err("This build connects only to staging sync.".into());
            }
        }
        if !(url.scheme() == "https" || local)
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || url.path() != "/"
        {
            return Err("Use an HTTPS sync server origin without a path or credentials.".into());
        }
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(8))
            .timeout(Duration::from_secs(20))
            .user_agent("SonoJot Sync")
            .build()
            .map_err(|_| "Cannot prepare sync.")?;
        Ok(Self {
            client,
            origin: origin.trim_end_matches('/').into(),
            token: Zeroizing::new(token.into()),
            proof: None,
            rotation: false,
            #[cfg(feature = "synctest")]
            test_token: None,
        })
    }
    #[cfg(feature = "synctest")]
    pub fn with_test_token(mut self, token: String) -> Self {
        self.test_token = Some(Zeroizing::new(token));
        self
    }
    pub fn with_proof(mut self, ak: &crate::sync_crypto::Secret, account: &str) -> Self {
        use base64::Engine;
        self.proof = Some(Zeroizing::new(
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(crate::sync_crypto::server_auth(ak, account).0),
        ));
        self
    }
    pub fn for_rotation(mut self) -> Self {
        self.rotation = true;
        self
    }
    pub async fn request(
        &self,
        method: Method,
        path: &str,
        json: Option<&Value>,
        data: Option<Vec<u8>>,
        rev: Option<u64>,
        epoch: u64,
    ) -> Result<(u64, Vec<u8>), Failure> {
        let fail = || Failure {
            status: 0,
            message: "Sync is offline. Your local words are safe; try Sync now when connected."
                .into(),
        };
        #[cfg(feature = "synctest")]
        if TEST_OFFLINE.load(std::sync::atomic::Ordering::Relaxed) {
            return Err(fail());
        }
        let mut req = self
            .client
            .request(method, format!("{}{}", self.origin, path));
        if path.starts_with("/v1/blobs/") {
            req = req.timeout(Duration::from_secs(180));
        }
        if self.rotation {
            req = req.header("X-Sync-Rotation", "1");
        }
        if !self.token.is_empty() {
            req = req.bearer_auth(&*self.token)
        }
        if let Some(proof) = &self.proof {
            req = req.header("X-Sync-Auth", &**proof);
        }
        #[cfg(feature = "synctest")]
        if let Some(token) = &self.test_token {
            req = req.header("X-Sync-Test-Token", &**token);
        }
        if let Some(j) = json {
            req = req.json(j)
        }
        if let Some(d) = data {
            req = req
                .header("Content-Type", "application/octet-stream")
                .body(d)
        }
        if let Some(r) = rev {
            req = req
                .header("If-Match", format!("\"{r}\""))
                .header("X-Sync-Epoch", epoch)
        }
        let mut response = req.send().await.map_err(|_| fail())?;
        let status = response.status();
        if status.as_u16() == 429 {
            let delay = response
                .headers()
                .get("Retry-After")
                .and_then(|v| v.to_str().ok())
                .and_then(retry_after)
                .unwrap_or(61);
            return Err(Failure {
                status: 429,
                message: format!("{RATE_LIMITED} Retry in {delay} seconds."),
            });
        }
        if !status.is_success() {
            return Err(Failure{status:status.as_u16(),message:match status.as_u16(){401=>"Sign in again to continue syncing.",403=>"Unlock this device or sign in again before changing this account.",409=>"Sync keys or account state changed. Unlock this device again.",410=>"This meeting no longer syncs. Sync again to update its status.",412=>"Content changed on another device. Retry sync to merge it.",413=>"Free sync holds 512 KiB of vocabulary. Your local words are safe.",507=>"Meeting storage is full. An older meeting needs a confirmed complete local copy before it can stop syncing.",429=>RATE_LIMITED,503=>"This sync service or sign-in method is unavailable. Your local words are safe.",_=>"Sync could not finish. Your local words are safe; try again."}.into()});
        }
        let rev = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim_matches('"').parse().ok())
            .unwrap_or(0);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| fail())? {
            let limit = if path.starts_with("/v1/blobs/") {
                crate::sync_crypto::MAX_BLOB_ENVELOPE
            } else {
                2 * 1024 * 1024
            };
            if bytes.len() + chunk.len() > limit {
                return Err(Failure {
                    status: 413,
                    message: "Sync response exceeds the supported size.".into(),
                });
            }
            bytes.extend_from_slice(&chunk)
        }
        Ok((rev, bytes))
    }
    pub async fn json<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        data: Option<&Value>,
    ) -> Result<T, String> {
        let (_, bytes) = self.request(method, path, data, None, None, 0).await?;
        serde_json::from_slice(&bytes).map_err(|_| "Sync returned an unsupported response.".into())
    }
    /// Long read-only inventories can span several server rate-limit windows.
    /// A retry never repeats a mutation or restarts an already verified inventory.
    pub(crate) async fn read_with_retry(&self, path: &str) -> Result<(u64, Vec<u8>), Failure> {
        self.read_with_backoff(path, Duration::from_secs(61)).await
    }
    async fn read_with_backoff(
        &self,
        path: &str,
        delay: Duration,
    ) -> Result<(u64, Vec<u8>), Failure> {
        for attempt in 0..3 {
            match self.request(Method::GET, path, None, None, None, 0).await {
                Err(error) if error.status == 429 && attempt < 2 => tokio::time::sleep(delay).await,
                result => return result,
            }
        }
        unreachable!()
    }
    pub async fn wraps(&self) -> Result<Vec<Wrap>, String> {
        self.json(Method::GET, "/v1/wraps", None).await
    }
    pub async fn put_wrap(
        &self,
        kind: &str,
        reference: &str,
        ciphertext: String,
        previous: u64,
    ) -> Result<u64, String> {
        let v:Value=self.json(Method::PUT,"/v1/wraps",Some(&serde_json::json!({"kind":kind,"ref":reference,"ciphertext":ciphertext,"previous":previous}))).await?;
        v["rev"]
            .as_u64()
            .ok_or_else(|| "Invalid wrap revision.".into())
    }
}

#[cfg(test)]
mod gate_tests {
    use super::*;
    #[test]
    fn rate_limit_retry_after_accepts_seconds_and_http_dates() {
        assert_eq!(retry_after("120"), Some(120));
        assert_eq!(retry_after("0"), Some(1));
        assert_eq!(retry_after("999999"), Some(86400));
        assert_eq!(retry_after("invalid"), None);
        let date = (time::OffsetDateTime::now_utc() + time::Duration::seconds(120))
            .format(&time::format_description::well_known::Rfc2822).unwrap().replace("+0000", "GMT");
        assert!((119..=120).contains(&retry_after(&date).unwrap()));
        assert_eq!(rate_limit_delay(RATE_LIMITED),Some(61));
        assert_eq!(rate_limit_delay("unrelated failure"),None);
    }
    #[test]
    fn stable_cannot_construct_a_network_sync_client() {
        assert_eq!(
            SyncClient::new(STAGING, "").is_ok(),
            crate::flavor::sync_enabled()
        );
        assert!(SyncClient::new("https://sync.sonojot.com", "").is_err());
        assert!(SyncClient::new("https://example.com", "").is_err());
    }

    #[test]
    fn staging_origin_must_be_explicit_and_cannot_select_production() {
        assert!(validate_staging_origin("")
            .unwrap_err()
            .contains("not configured"));
        assert!(
            validate_staging_origin("https://sonojot-sync-staging.example.workers.dev").is_ok()
        );
        for origin in [
            "https://sync.sonojot.com",
            "https://sonojot-sync.example.workers.dev",
            "http://sonojot-sync-staging.example.workers.dev",
            "https://sonojot-sync-staging.example.workers.dev.evil.com",
            "https://sonojot-sync-staging.example.workers.dev/path",
            "https://user@sonojot-sync-staging.example.workers.dev",
            "https://sonojot-sync-staging.example.workers.dev?query",
            "https://sonojot-sync-staging.example.workers.dev#fragment",
            "https://sonojot-sync-staging.example.workers.dev:8443",
        ] {
            assert!(validate_staging_origin(origin).is_err(), "{origin}");
            assert!(SyncClient::new(origin, "").is_err(), "{origin}");
        }
    }

    #[test]
    fn compiled_sync_origin_matches_the_build_environment() {
        // --nocapture can verify the compiled value without contacting the server.
        println!("Compiled SONOJOT_SYNC_ORIGIN: {STAGING:?}");
        assert_eq!(STAGING, option_env!("SONOJOT_SYNC_ORIGIN").unwrap_or(""));
        assert_eq!(
            crate::flavor::sync_enabled(),
            crate::flavor::require_sync().is_ok()
        );
    }

    #[cfg(feature = "synctest")]
    #[test]
    fn sync_inventory_retries_rate_limits_without_restarting() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            for status in ["429 Too Many Requests", "200 OK"] {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut bytes = [0; 2048];
                let size = stream.read(&mut bytes).unwrap();
                assert!(std::str::from_utf8(&bytes[..size])
                    .unwrap()
                    .starts_with("GET /v1/inventory?after=synthetic-250 "));
                write!(
                    stream,
                    "HTTP/1.1 {status}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                )
                .unwrap();
            }
        });
        let client = SyncClient::new(&origin, "").unwrap();
        let (_, bytes) = tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(client.read_with_backoff("/v1/inventory?after=synthetic-250", Duration::ZERO))
            .unwrap();
        assert_eq!(bytes, b"{}");
        server.join().unwrap();
    }
}
