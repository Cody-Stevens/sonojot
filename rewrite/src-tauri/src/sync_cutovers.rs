//! Paged, encrypted re-key inventory. The keyring pins the entire ordered checkpoint.
use crate::{
    sync_account::Credentials,
    sync_crypto::{self as crypto, Keyring, Secret},
};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Checkpoint {
    pub epoch: u64,
    pub nonce: String,
    pub pages: usize,
    pub sha256: [u8; 32],
}
impl Checkpoint {
    fn object(&self, index: usize) -> String {
        format!("rekey/{}/{}/cutovers/{index}", self.epoch, self.nonce)
    }
    fn path(&self, index: usize) -> String {
        format!("/v1/rekey-pages/{}/{}/{index}", self.epoch, self.nonce)
    }
    fn validate(&self) -> Result<(), String> {
        if self.epoch < 2
            || self.pages == 0
            || self.pages > 128
            || uuid::Uuid::parse_str(&self.nonce).is_err()
        {
            return Err("Invalid re-key checkpoint.".into());
        }
        Ok(())
    }
}
fn pages(objects: &BTreeMap<String, u64>) -> Result<Vec<zeroize::Zeroizing<Vec<u8>>>, String> {
    let entries: Vec<_> = objects.iter().collect();
    if entries.len() > 128 * 1024 {
        return Err("Re-key inventory exceeds 131072 text objects.".into());
    }
    entries
        .chunks(1024)
        .map(|chunk| {
            let map: BTreeMap<_, _> = chunk.iter().copied().collect();
            serde_json::to_vec(&map)
                .map(zeroize::Zeroizing::new)
                .map_err(|_| "Cannot encode re-key checkpoint.".into())
        })
        .collect()
}
fn digest(pages: &[zeroize::Zeroizing<Vec<u8>>]) -> [u8; 32] {
    let mut h = Sha256::new();
    for page in pages {
        h.update((page.len() as u64).to_be_bytes());
        h.update(page.as_slice());
    }
    h.finalize().into()
}
pub async fn publish(c: &Credentials, ak: &Secret, ring: &mut Keyring) -> Result<(), String> {
    ring.cutover_checkpoint = None;
    if ring.object_cutovers.is_empty() {
        return Ok(());
    }
    let pages = pages(&ring.object_cutovers)?;
    let checkpoint = Checkpoint {
        epoch: ring.epoch,
        nonce: uuid::Uuid::new_v4().to_string(),
        pages: pages.len(),
        sha256: digest(&pages),
    };
    let key = crypto::derive(&ak.0, &c.account, "meeting-cutovers");
    let client = c.client()?;
    for (i, page) in pages.iter().enumerate() {
        let sealed = crypto::seal(&key, 0, &c.account, &checkpoint.object(i), 1, page)?;
        client
            .request(
                Method::PUT,
                &checkpoint.path(i),
                None,
                Some(sealed),
                Some(0),
                ring.epoch - 1,
            )
            .await?;
    }
    ring.cutover_checkpoint = Some(checkpoint);
    Ok(())
}
pub async fn hydrate(
    store: &crate::store::Store,
    c: &Credentials,
    ring: &mut Keyring,
) -> Result<(), String> {
    let Some(checkpoint) = &ring.cutover_checkpoint else {
        return Ok(());
    };
    checkpoint.validate()?;
    if checkpoint.epoch > ring.epoch {
        return Err("Re-key checkpoint is from a future epoch.".into());
    }
    let ak = c.ak.as_ref().ok_or("Unlock sync first.")?;
    let key = crypto::derive(&ak.0, &c.account, "meeting-cutovers");
    let scope = format!(
        "{:x}",
        Sha256::digest(
            serde_json::to_vec(&(&c.server, &c.account, checkpoint))
                .map_err(|_| "Cannot identify checkpoint cache.")?
        )
    );
    let directory = store.root().join("sync-cutover-cache");
    store.check_owned(&directory)?;
    std::fs::create_dir_all(&directory).map_err(|_| "Cannot create checkpoint cache.")?;
    let mut pages = Vec::new();
    let mut objects = BTreeMap::new();
    let mut length = 0;
    for i in 0..checkpoint.pages {
        use base64::Engine;
        let path = directory.join(format!("{i}.json"));
        // Fixed page slots bound the cache to 128 encrypted pages. Scope pins the
        // server, account and authenticated checkpoint; every hit still opens AEAD.
        let cached = store
            .read::<(String, String)>(&path)
            .ok()
            .filter(|(saved, _)| saved == &scope)
            .and_then(|(_, encoded)| {
                base64::engine::general_purpose::STANDARD
                    .decode(encoded)
                    .ok()
            })
            .and_then(|bytes| {
                crypto::open(&key, &c.account, &checkpoint.object(i), 1, &bytes).ok()
            });
        let plain = match cached {
            Some(plain) => plain,
            None => {
                let (_, bytes) = c.client()?.read_with_retry(&checkpoint.path(i)).await?;
                let plain = crypto::open(&key, &c.account, &checkpoint.object(i), 1, &bytes)?;
                store.write(
                    &path,
                    &(
                        &scope,
                        base64::engine::general_purpose::STANDARD.encode(&bytes),
                    ),
                )?;
                plain
            }
        };
        length += plain.len();
        if length > 32 * 1024 * 1024 {
            return Err("Re-key checkpoint exceeds its size limit.".into());
        }
        let page: BTreeMap<String, u64> =
            serde_json::from_slice(&plain).map_err(|_| "Invalid re-key page.")?;
        if page.len() > 1024
            || page
                .iter()
                .any(|(id, rev)| id.len() > 256 || id.starts_with("blob/") || *rev == 0)
        {
            return Err("Invalid re-key page entries.".into());
        }
        for (id, rev) in page {
            if objects.insert(id, rev).is_some() {
                return Err("Duplicate re-key object.".into());
            }
        }
        pages.push(plain);
    }
    if digest(&pages) != checkpoint.sha256 {
        return Err("Re-key checkpoint hash differs.".into());
    }
    ring.object_cutovers = objects;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn sync_cutovers_thousands_of_objects_keep_wrap_small_and_pin_pages() {
        let mut ring = Keyring::new();
        ring.object_cutovers = (0..12000)
            .map(|i| (format!("m/synthetic-{i:05}/raw/synthetic-base"), 1))
            .collect();
        let pages = pages(&ring.object_cutovers).unwrap();
        assert_eq!(pages.len(), 12);
        ring.cutover_checkpoint = Some(Checkpoint {
            epoch: 2,
            nonce: uuid::Uuid::new_v4().to_string(),
            pages: pages.len(),
            sha256: digest(&pages),
        });
        let ak = Secret::random();
        let sealed = ring.seal(&ak, "synthetic-account", 2).unwrap();
        assert!(sealed.len() * 4 / 3 < 44000);
        let opened = Keyring::open(&ak, "synthetic-account", 2, &sealed).unwrap();
        assert!(opened.object_cutovers.is_empty());
        assert_eq!(crypto::keyset_pin(&ring), crypto::keyset_pin(&opened));
        let pin = crypto::keyset_pin(&ring);
        ring.cutover_checkpoint.as_mut().unwrap().sha256[0] ^= 1;
        assert_ne!(pin, crypto::keyset_pin(&ring));
        assert!(opened
            .object_key("m/synthetic-00001/raw/synthetic-base", 2, 2)
            .is_err());
    }
}
