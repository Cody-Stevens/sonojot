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

pub fn validate_staging_origin(origin: &str) -> Result<(), String> {
    if origin.is_empty() {
        return Err("Sync is not configured in this build. Install an official nightly or Sync Test build.".into());
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
    if valid { Ok(()) } else {
        Err("This build requires an HTTPS staging sync origin configured at build time.".into())
    }
}
#[cfg(feature = "synctest")]
pub static TEST_OFFLINE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
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
        if TEST_OFFLINE.load(std::sync::atomic::Ordering::Relaxed) { return Err(fail()); }
        let mut req = self
            .client
            .request(method, format!("{}{}", self.origin, path));
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
        if !status.is_success() {
            return Err(Failure{status:status.as_u16(),message:match status.as_u16(){401=>"Sign in again to continue syncing.",403=>"Unlock this device or sign in again before changing this account.",409=>"Sync keys or account state changed. Unlock this device again.",412=>"Words changed on another device. Retry sync to merge them.",413=>"Free sync holds 512 KiB of vocabulary. Your local words are safe.",429=>"Too many sync attempts. Wait a minute and try again.",503=>"This sync service or sign-in method is unavailable. Your local words are safe.",_=>"Sync could not finish. Your local words are safe; try again."}.into()});
        }
        let rev = response
            .headers()
            .get("etag")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.trim_matches('"').parse().ok())
            .unwrap_or(0);
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| fail())? {
            if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
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
    fn stable_cannot_construct_a_network_sync_client() {
        assert_eq!(SyncClient::new(STAGING, "").is_ok(), crate::flavor::sync_enabled());
        assert!(SyncClient::new("https://sync.sonojot.com", "").is_err());
        assert!(SyncClient::new("https://example.com", "").is_err());
    }

    #[test]
    fn staging_origin_must_be_explicit_and_cannot_select_production() {
        assert!(validate_staging_origin("").unwrap_err().contains("not configured"));
        assert!(validate_staging_origin("https://sonojot-sync-staging.example.workers.dev").is_ok());
        for origin in [
            "https://sync.sonojot.com", "https://sonojot-sync.example.workers.dev",
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
        assert_eq!(crate::flavor::sync_enabled(), crate::flavor::require_sync().is_ok());
    }
}
