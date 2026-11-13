//! Versioned, account-bound sync encryption. Identity credentials never enter a KDF.
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use hkdf::Hkdf;
use hpke::{
    kdf::HkdfSha256, kem::X25519HkdfSha256, Deserializable, Kem as KemTrait, OpModeR, OpModeS,
    Serializable,
};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::{Zeroize, ZeroizeOnDrop, Zeroizing};

const BAD: &str = "Sync data could not be authenticated. Your local words are unchanged.";
/// A well-formed code that fails to open the wrap is almost always a mistyped
/// or replaced code, so say that instead of suggesting the server sent bad data.
const WRONG_RECOVERY: &str = "This recovery code does not unlock this account. Check it, or use your newest code if you replaced it. Your local words are unchanged.";
pub const MAX_ENVELOPE: usize = 512 * 1024;
const HEADER: usize = 21;
type Kem = X25519HkdfSha256;
type HpkeAead = hpke::aead::ChaCha20Poly1305;

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct Secret(pub [u8; 32]);
impl Secret {
    pub fn random() -> Self {
        Self(random())
    }
}
pub fn random<const N: usize>() -> [u8; N] {
    let mut value = [0; N];
    OsRng.fill_bytes(&mut value);
    value
}
pub fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut h = Sha256::new();
    for part in parts {
        h.update((part.len() as u32).to_be_bytes());
        h.update(part);
    }
    h.finalize().into()
}
fn context(account: &str, object: &str) -> Vec<u8> {
    let mut out = b"sonojot-sync/v1".to_vec();
    for part in [account.as_bytes(), object.as_bytes()] {
        out.extend_from_slice(&(part.len() as u32).to_be_bytes());
        out.extend_from_slice(part);
    }
    out
}
pub fn derive(input: &[u8], account: &str, purpose: &str) -> Secret {
    let mut key = Secret([0; 32]);
    Hkdf::<Sha256>::new(Some(b"sonojot-sync/v1"), input)
        .expand(&context(account, purpose), &mut key.0)
        .expect("32-byte HKDF output");
    key
}
pub fn seal(
    key: &Secret,
    kid: u32,
    account: &str,
    object: &str,
    rev: u64,
    data: &[u8],
) -> Result<Vec<u8>, String> {
    seal_nonce(key, kid, account, object, rev, data, random())
}
fn seal_nonce(
    key: &Secret,
    kid: u32,
    account: &str,
    object: &str,
    rev: u64,
    data: &[u8],
    nonce: [u8; 12],
) -> Result<Vec<u8>, String> {
    if data.len() > MAX_ENVELOPE - 16384 || rev == 0 {
        return Err(BAD.into());
    }
    let needed = HEADER + 16 + 12 + data.len();
    let bucket = if needed <= 65536 { 4096 } else { 16384 };
    let total = needed.div_ceil(bucket) * bucket;
    let mut plain = Zeroizing::new(vec![0; total - HEADER - 16]);
    plain[..8].copy_from_slice(&rev.to_be_bytes());
    plain[8..12].copy_from_slice(&(data.len() as u32).to_be_bytes());
    plain[12..12 + data.len()].copy_from_slice(data);
    let mut out = b"SJV1\x01".to_vec();
    out.extend_from_slice(&kid.to_be_bytes());
    out.extend_from_slice(&nonce);
    let mut aad = out.clone();
    aad.extend_from_slice(&context(account, object));
    let encrypted = ChaCha20Poly1305::new((&key.0).into())
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plain,
                aad: &aad,
            },
        )
        .map_err(|_| BAD)?;
    out.extend_from_slice(&encrypted);
    Ok(out)
}
pub fn envelope_kid(data: &[u8]) -> Result<u32, String> {
    if data.len() < 4096
        || data.len() > MAX_ENVELOPE
        || &data[..5] != b"SJV1\x01"
        || data.len() % (if data.len() <= 65536 { 4096 } else { 16384 }) != 0
    {
        return Err(BAD.into());
    }
    Ok(u32::from_be_bytes(data[5..9].try_into().map_err(|_| BAD)?))
}
pub fn open(
    key: &Secret,
    account: &str,
    object: &str,
    expected_rev: u64,
    data: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    envelope_kid(data)?;
    let mut aad = data[..HEADER].to_vec();
    aad.extend_from_slice(&context(account, object));
    let plain = Zeroizing::new(
        ChaCha20Poly1305::new((&key.0).into())
            .decrypt(
                Nonce::from_slice(&data[9..21]),
                Payload {
                    msg: &data[HEADER..],
                    aad: &aad,
                },
            )
            .map_err(|_| BAD)?,
    );
    if plain.len() < 12 {
        return Err(BAD.into());
    }
    let rev = u64::from_be_bytes(plain[..8].try_into().map_err(|_| BAD)?);
    let len = u32::from_be_bytes(plain[8..12].try_into().map_err(|_| BAD)?) as usize;
    if rev != expected_rev
        || rev == 0
        || len > plain.len() - 12
        || plain[12 + len..].iter().any(|&b| b != 0)
    {
        return Err(BAD.into());
    }
    Ok(Zeroizing::new(plain[12..12 + len].to_vec()))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keyring {
    #[serde(default = "initial_epoch")]
    pub epoch: u64,
    pub schema: u32,
    pub active: u32,
    #[serde(default)]
    pub vocabulary_cutover: u64,
    pub keys: std::collections::BTreeMap<u32, Secret>,
    #[serde(default)]
    pub devices: std::collections::BTreeMap<String, [u8; 32]>,
}
fn initial_epoch() -> u64 {
    1
}
impl Keyring {
    pub fn new() -> Self {
        Self {
            schema: 1,
            epoch: 1,
            active: 1,
            vocabulary_cutover: 0,
            keys: [(1, Secret::random())].into(),
            devices: Default::default(),
        }
    }
    pub fn rotate(&mut self) -> Result<(), String> {
        if self.keys.len() >= 64 {
            return Err("This keyring has reached its safe rotation limit. Export your words before resetting sync.".into());
        }
        self.active = self.active.checked_add(1).ok_or(BAD)?;
        self.keys.insert(self.active, Secret::random());
        Ok(())
    }
    // The caller authenticates every current object and atomically binds that inventory
    // to the rotation. Key ids remain monotonic even when their entries are pruned.
    pub fn rotate_retaining(
        &mut self,
        used: &std::collections::BTreeSet<u32>,
    ) -> Result<(), String> {
        let next = self
            .keys
            .keys()
            .next_back()
            .copied()
            .unwrap_or(self.active)
            .max(self.active)
            .checked_add(1)
            .ok_or(BAD)?;
        if used
            .iter()
            .any(|id| !self.keys.contains_key(id) || *id >= next)
        {
            return Err(
                "An object uses an unknown encryption key. Update and unlock before re-keying."
                    .into(),
            );
        }
        if used.len() >= 64 {
            return Err("Too many current object keys. Export and remove unused synced objects before re-keying.".into());
        }
        self.keys.retain(|id, _| used.contains(id));
        self.keys.insert(next, Secret::random());
        self.active = next;
        Ok(())
    }
    pub fn seal(&self, ak: &Secret, account: &str, rev: u64) -> Result<Vec<u8>, String> {
        if self.devices.len() > 64 || self.keys.len() > 256 {
            return Err(BAD.into());
        }
        let plain = Zeroizing::new(serde_json::to_vec(self).map_err(|_| BAD)?);
        seal(
            &derive(&ak.0, account, "keyring"),
            0,
            account,
            "keyring",
            rev,
            &plain,
        )
    }
    pub fn open(ak: &Secret, account: &str, rev: u64, bytes: &[u8]) -> Result<Self, String> {
        let ring: Self = serde_json::from_slice(&open(
            &derive(&ak.0, account, "keyring"),
            account,
            "keyring",
            rev,
            bytes,
        )?)
        .map_err(|_| BAD)?;
        if ring.schema != 1
            || ring.epoch == 0
            || ring.devices.len() > 64
            || ring.keys.len() > 256
            || !ring.keys.contains_key(&ring.active)
            || ring.keys.contains_key(&0)
        {
            return Err(BAD.into());
        }
        Ok(ring)
    }
}

/// Changes require explicit pairing/recovery or an authenticated transition
/// from the locally pinned predecessor and a trusted device.
/// Roster edits alone do not change this checkpoint.
pub fn keyset_pin(ring: &Keyring) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(b"sonojot-keyset/v1");
    h.update(ring.epoch.to_be_bytes());
    h.update(ring.active.to_be_bytes());
    h.update(ring.vocabulary_cutover.to_be_bytes());
    for (id, key) in &ring.keys {
        h.update(id.to_be_bytes());
        h.update(key.0);
    }
    h.finalize().into()
}
pub fn server_auth(ak: &Secret, account: &str) -> Secret {
    derive(&ak.0, account, "server-auth")
}
pub fn server_auth_verifier(proof: &Secret) -> [u8; 32] {
    Sha256::digest(proof.0).into()
}

/// Both commitments are pinned before their peer reveals a nonce. The SAS
/// authenticates the two public keys AND identities, including the recipient id.
#[derive(Clone, Serialize, Deserialize)]
pub struct PairTranscript {
    pub account: String,
    pub request: String,
    pub receiver_id: String,
    pub receiver: [u8; 32],
    pub sender_id: String,
    pub sender: [u8; 32],
    pub nonce_a: [u8; 32],
    pub nonce_b: [u8; 32],
}
impl PairTranscript {
    pub fn receiver_commit(&self) -> [u8; 32] {
        digest(&[
            b"pair-receiver/v2",
            self.account.as_bytes(),
            self.request.as_bytes(),
            self.receiver_id.as_bytes(),
            &self.receiver,
            &self.nonce_b,
        ])
    }
    pub fn sender_commit(&self) -> [u8; 32] {
        digest(&[
            b"pair-sender/v2",
            self.account.as_bytes(),
            self.request.as_bytes(),
            self.receiver_id.as_bytes(),
            &self.receiver,
            self.sender_id.as_bytes(),
            &self.sender,
            &self.nonce_a,
        ])
    }
    pub fn hash(&self) -> [u8; 32] {
        digest(&[
            b"pair-sas/v2",
            &self.receiver_commit(),
            &self.sender_commit(),
        ])
    }
    pub fn code(&self) -> String {
        let h = self.hash();
        format!(
            "{:06}",
            u64::from_be_bytes(h[..8].try_into().unwrap()) % 1_000_000
        )
    }
    pub fn binding(&self) -> String {
        use base64::Engine;
        format!(
            "pair/v2/{}/{}",
            self.request,
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(self.hash())
        )
    }
}
impl Default for Keyring {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Serialize, Deserialize, Zeroize, ZeroizeOnDrop)]
pub struct DeviceKey {
    pub secret: [u8; 32],
    pub public: [u8; 32],
}
impl DeviceKey {
    pub fn new() -> Self {
        let (sk, pk) = Kem::gen_keypair(&mut OsRng);
        Self {
            secret: sk.to_bytes().into(),
            public: pk.to_bytes().into(),
        }
    }
}
impl Default for DeviceKey {
    fn default() -> Self {
        Self::new()
    }
}
pub fn wrap_device(
    public: &[u8; 32],
    account: &str,
    binding: &str,
    plaintext: &[u8],
) -> Result<Vec<u8>, String> {
    let pk = <Kem as KemTrait>::PublicKey::from_bytes(public).map_err(|_| BAD)?;
    let ctx = context(account, binding);
    let (enc, mut sender) =
        hpke::setup_sender::<HpkeAead, HkdfSha256, Kem, _>(&OpModeS::Base, &pk, &ctx, &mut OsRng)
            .map_err(|_| BAD)?;
    let mut out = enc.to_bytes().to_vec();
    out.extend(sender.seal(plaintext, &ctx).map_err(|_| BAD)?);
    Ok(out)
}
pub fn unwrap_device(
    key: &DeviceKey,
    account: &str,
    binding: &str,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    if bytes.len() < 48 || bytes.len() > MAX_ENVELOPE {
        return Err(BAD.into());
    }
    let sk = <Kem as KemTrait>::PrivateKey::from_bytes(&key.secret).map_err(|_| BAD)?;
    let enc = <Kem as KemTrait>::EncappedKey::from_bytes(&bytes[..32]).map_err(|_| BAD)?;
    let ctx = context(account, binding);
    let mut receiver =
        hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(&OpModeR::Base, &sk, &enc, &ctx)
            .map_err(|_| BAD)?;
    Ok(Zeroizing::new(
        receiver.open(&bytes[32..], &ctx).map_err(|_| BAD)?,
    ))
}

pub fn recovery_code() -> Zeroizing<String> {
    format_recovery(&random())
}

/// Re-key messages authenticate the sender against a previously decrypted roster.
/// HPKE Base alone would let the server encrypt a replacement account key.
pub fn wrap_from(
    sender: &DeviceKey,
    recipient: &[u8; 32],
    account: &str,
    binding: &str,
    plain: &[u8],
) -> Result<Vec<u8>, String> {
    wrap_from_rng(sender, recipient, account, binding, plain, &mut OsRng)
}
fn wrap_from_rng<R: rand::RngCore + rand::CryptoRng>(
    sender: &DeviceKey,
    recipient: &[u8; 32],
    account: &str,
    binding: &str,
    plain: &[u8],
    rng: &mut R,
) -> Result<Vec<u8>, String> {
    let sk = <Kem as KemTrait>::PrivateKey::from_bytes(&sender.secret).map_err(|_| BAD)?;
    let pk = <Kem as KemTrait>::PublicKey::from_bytes(&sender.public).map_err(|_| BAD)?;
    let receiver = <Kem as KemTrait>::PublicKey::from_bytes(recipient).map_err(|_| BAD)?;
    let ctx = context(account, binding);
    let (enc, mut tx) = hpke::setup_sender::<HpkeAead, HkdfSha256, Kem, _>(
        &OpModeS::Auth((sk, pk)),
        &receiver,
        &ctx,
        rng,
    )
    .map_err(|_| BAD)?;
    let mut out = enc.to_bytes().to_vec();
    out.extend(tx.seal(plain, &ctx).map_err(|_| BAD)?);
    Ok(out)
}
pub fn unwrap_from(
    receiver: &DeviceKey,
    sender: &[u8; 32],
    account: &str,
    binding: &str,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, String> {
    if bytes.len() < 48 || bytes.len() > MAX_ENVELOPE {
        return Err(BAD.into());
    }
    let sk = <Kem as KemTrait>::PrivateKey::from_bytes(&receiver.secret).map_err(|_| BAD)?;
    let pk = <Kem as KemTrait>::PublicKey::from_bytes(sender).map_err(|_| BAD)?;
    let enc = <Kem as KemTrait>::EncappedKey::from_bytes(&bytes[..32]).map_err(|_| BAD)?;
    let ctx = context(account, binding);
    let mut rx =
        hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(&OpModeR::Auth(pk), &sk, &enc, &ctx)
            .map_err(|_| BAD)?;
    Ok(Zeroizing::new(
        rx.open(&bytes[32..], &ctx).map_err(|_| BAD)?,
    ))
}
fn format_recovery(bytes: &[u8; 16]) -> Zeroizing<String> {
    let hex = bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
    let checksum = digest(&[b"recovery-check/v1", bytes]);
    Zeroizing::new(format!(
        "SJ1-{}-{}",
        hex.as_bytes()
            .chunks(4)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("-"),
        checksum[..4]
            .iter()
            .map(|b| format!("{b:02X}"))
            .collect::<String>()
    ))
}
fn recovery_bytes(code: &str) -> Result<Zeroizing<[u8; 16]>, String> {
    if code.len() > 80 {
        return Err("Recovery code is invalid.".into());
    }
    let cleaned = Zeroizing::new(code.trim().to_ascii_uppercase().replace('-', ""));
    if !cleaned.starts_with("SJ1") || cleaned.len() != 43 {
        return Err("Recovery code is invalid.".into());
    }
    let mut bytes = Zeroizing::new([0; 16]);
    let raw = cleaned.as_bytes();
    for (i, byte) in bytes.iter_mut().enumerate() {
        let nibble = |b: u8| match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        };
        *byte = (nibble(raw[3 + 2 * i]).ok_or("Recovery code is invalid.")? << 4)
            | nibble(raw[4 + 2 * i]).ok_or("Recovery code is invalid.")?;
    }
    let expected = format_recovery(&bytes);
    let normalized = Zeroizing::new(expected.replace('-', ""));
    if normalized.as_bytes().ct_eq(cleaned.as_bytes()).unwrap_u8() != 1 {
        return Err("Recovery code is invalid.".into());
    }
    Ok(bytes)
}
pub fn wrap_recovery(code: &str, account: &str, ak: &Secret) -> Result<Vec<u8>, String> {
    let bytes = recovery_bytes(code)?;
    seal(
        &derive(&bytes[..], account, "recovery"),
        0,
        account,
        "recovery",
        1,
        &ak.0,
    )
}
pub fn unwrap_recovery(code: &str, account: &str, wrapped: &[u8]) -> Result<Secret, String> {
    let bytes = recovery_bytes(code)?;
    let plain = open(
        &derive(&bytes[..], account, "recovery"),
        account,
        "recovery",
        1,
        wrapped,
    )
    .map_err(|_| WRONG_RECOVERY)?;
    Ok(Secret(plain.as_slice().try_into().map_err(|_| BAD)?))
}
/// Deterministic, synthetic interoperability fixtures. Never used by runtime code.
#[cfg(any(test, feature = "sync-test-vectors"))]
pub fn test_vectors() -> serde_json::Value {
    struct Fixed;
    impl rand::RngCore for Fixed {
        fn next_u32(&mut self) -> u32 {
            0x33333333
        }
        fn next_u64(&mut self) -> u64 {
            0x3333333333333333
        }
        fn fill_bytes(&mut self, d: &mut [u8]) {
            d.fill(0x33);
        }
        fn try_fill_bytes(&mut self, d: &mut [u8]) -> Result<(), rand::Error> {
            self.fill_bytes(d);
            Ok(())
        }
    }
    impl rand::CryptoRng for Fixed {}
    let hex = |b: &[u8]| b.iter().map(|b| format!("{b:02x}")).collect::<String>();
    let device = |ikm: &[u8]| {
        let (sk, pk) = Kem::derive_keypair(ikm);
        DeviceKey {
            secret: sk.to_bytes().into(),
            public: pk.to_bytes().into(),
        }
    };
    let sender = device(&[0x11; 32]);
    let receiver = device(&[0x22; 32]);
    let account = "vector-account";
    let ak = Secret([0x44; 32]);
    let code = format_recovery(&[0x55; 16]);
    let recovery_key = derive(&[0x55; 16], account, "recovery");
    let t = PairTranscript {
        account: account.into(),
        request: "vector-pair".into(),
        receiver_id: "device-b".into(),
        receiver: receiver.public,
        sender_id: "device-a".into(),
        sender: sender.public,
        nonce_a: [0x66; 32],
        nonce_b: [0x77; 32],
    };
    let auth = server_auth(&ak, account);
    serde_json::json!({
        "schema":2,"warning":"Public synthetic test inputs only. Never use these keys in an account.",
        "account":account,"akHex":hex(&ak.0),
        "envelope":{"keyHex":hex(&[0x88;32]),"kid":7,"object":"vocabulary","rev":9,"plaintextHex":hex(b"vector vocabulary"),"nonceHex":hex(&[0x99;12]),"ciphertextHex":hex(&seal_nonce(&Secret([0x88;32]),7,account,"vocabulary",9,b"vector vocabulary",[0x99;12]).unwrap())},
        "hpkeAuth":{"senderIkmHex":hex(&[0x11;32]),"receiverIkmHex":hex(&[0x22;32]),"ephemeralIkmHex":hex(&[0x33;32]),"senderSecretHex":hex(&sender.secret),"senderPublicHex":hex(&sender.public),"receiverSecretHex":hex(&receiver.secret),"receiverPublicHex":hex(&receiver.public),"binding":t.binding(),"plaintextHex":hex(&ak.0),"ciphertextHex":hex(&wrap_from_rng(&sender,&receiver.public,account,&t.binding(),&ak.0,&mut Fixed).unwrap())},
        "recovery":{"entropyHex":hex(&[0x55;16]),"code":&*code,"derivedKeyHex":hex(&recovery_key.0),"nonceHex":hex(&[0xaa;12]),"wrapHex":hex(&seal_nonce(&recovery_key,0,account,"recovery",1,&ak.0,[0xaa;12]).unwrap())},
        "pairing":{"transcript":t,"receiverCommitHex":hex(&t.receiver_commit()),"senderCommitHex":hex(&t.sender_commit()),"hashHex":hex(&t.hash()),"code":t.code()},
        "serverAuth":{"proofHex":hex(&auth.0),"verifierHex":hex(&server_auth_verifier(&auth))}
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn published_security_vectors_are_reproducible() {
        let vectors: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/sync-security-vectors.json")).unwrap();
        assert_eq!(super::test_vectors(), vectors);
    }
    #[test]
    fn pairing_v2_binds_both_keys_and_device_ids() {
        let t = super::PairTranscript {
            account: "account".into(),
            request: "pair".into(),
            receiver_id: "b".into(),
            receiver: [2; 32],
            sender_id: "a".into(),
            sender: [1; 32],
            nonce_a: [3; 32],
            nonce_b: [4; 32],
        };
        for n in 0..6 {
            let mut changed = t.clone();
            match n {
                0 => changed.sender[0] ^= 1,
                1 => changed.receiver[0] ^= 1,
                2 => changed.receiver_id = "server-picked-id".into(),
                3 => changed.sender_id = "mallory".into(),
                4 => changed.nonce_a[0] ^= 1,
                _ => changed.nonce_b[0] ^= 1,
            }
            assert_ne!(t.hash(), changed.hash());
            assert_ne!(t.code(), changed.code());
        }
    }
    #[test]
    fn keyring_rotation_limit_preserves_all_existing_keys() {
        let mut ring = super::Keyring::new();
        for _ in 1..64 {
            ring.rotate().unwrap();
        }
        let before = super::keyset_pin(&ring);
        assert!(ring.rotate().is_err());
        assert_eq!(super::keyset_pin(&ring), before);
        assert_eq!(ring.keys.len(), 64);
    }
    #[test]
    fn capped_keyring_prunes_only_unused_keys_and_rotates_without_reset() {
        let mut ring = super::Keyring::new();
        for _ in 1..64 {
            ring.rotate().unwrap();
        }
        let keep = ring.keys[&17].0;
        ring.rotate_retaining(&[17, 64].into()).unwrap();
        assert_eq!(ring.active, 65);
        assert_eq!(ring.keys.len(), 3);
        assert_eq!(ring.keys[&17].0, keep);
        let before = super::keyset_pin(&ring);
        assert!(ring.rotate_retaining(&[999].into()).is_err());
        assert_eq!(super::keyset_pin(&ring), before);
        ring.rotate_retaining(&Default::default()).unwrap();
        assert_eq!(ring.active, 66);
        assert_eq!(ring.keys.len(), 1);
    }

    #[test]
    fn authenticated_rotation_rejects_server_forgery_and_wrong_epoch() {
        let sender = super::DeviceKey::new();
        let recipient = super::DeviceKey::new();
        let attacker = super::DeviceKey::new();
        let good = super::wrap_from(
            &sender,
            &recipient.public,
            "account",
            "rekey/device/2",
            b"synthetic account key",
        )
        .unwrap();
        assert_eq!(
            &**super::unwrap_from(
                &recipient,
                &sender.public,
                "account",
                "rekey/device/2",
                &good
            )
            .unwrap(),
            b"synthetic account key"
        );
        assert!(super::unwrap_from(
            &recipient,
            &attacker.public,
            "account",
            "rekey/device/2",
            &good
        )
        .is_err());
        assert!(super::unwrap_from(
            &recipient,
            &sender.public,
            "account",
            "rekey/device/3",
            &good
        )
        .is_err());
        let forged = super::wrap_from(
            &attacker,
            &recipient.public,
            "account",
            "rekey/device/2",
            b"replacement",
        )
        .unwrap();
        assert!(super::unwrap_from(
            &recipient,
            &sender.public,
            "account",
            "rekey/device/2",
            &forged
        )
        .is_err());
    }
    use super::*;
    #[test]
    fn rfc9180_published_vector() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/hpke-vector.json")).unwrap();
        let hex = |s: &str| -> Vec<u8> {
            s.as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect()
        };
        let sk =
            <Kem as KemTrait>::PrivateKey::from_bytes(&hex(v["skRm"].as_str().unwrap())).unwrap();
        let enc =
            <Kem as KemTrait>::EncappedKey::from_bytes(&hex(v["enc"].as_str().unwrap())).unwrap();
        let mut receiver = hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(
            &OpModeR::Base,
            &sk,
            &enc,
            &hex(v["info"].as_str().unwrap()),
        )
        .unwrap();
        let row = &v["encryptions"][0];
        assert_eq!(
            receiver
                .open(
                    &hex(row["ct"].as_str().unwrap()),
                    &hex(row["aad"].as_str().unwrap())
                )
                .unwrap(),
            hex(row["pt"].as_str().unwrap())
        );
    }
    #[test]
    fn rfc9180_published_authenticated_vector() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/hpke-auth-vector.json")).unwrap();
        let hex = |s: &str| -> Vec<u8> {
            s.as_bytes()
                .chunks_exact(2)
                .map(|p| u8::from_str_radix(std::str::from_utf8(p).unwrap(), 16).unwrap())
                .collect()
        };
        let sk =
            <Kem as KemTrait>::PrivateKey::from_bytes(&hex(v["skRm"].as_str().unwrap())).unwrap();
        let enc =
            <Kem as KemTrait>::EncappedKey::from_bytes(&hex(v["enc"].as_str().unwrap())).unwrap();
        let mut receiver = hpke::setup_receiver::<HpkeAead, HkdfSha256, Kem>(
            &OpModeR::Auth(
                <Kem as KemTrait>::PublicKey::from_bytes(&hex(v["pkSm"].as_str().unwrap()))
                    .unwrap(),
            ),
            &sk,
            &enc,
            &hex(v["info"].as_str().unwrap()),
        )
        .unwrap();
        let row = &v["encryptions"][0];
        assert_eq!(
            receiver
                .open(
                    &hex(row["ct"].as_str().unwrap()),
                    &hex(row["aad"].as_str().unwrap())
                )
                .unwrap(),
            hex(row["pt"].as_str().unwrap())
        );
    }
    #[test]
    fn envelopes_round_trip_and_pad() {
        let key = Secret::random();
        for size in [0, 1, 4000, 65000, 90000, 490000] {
            let data = vec![7; size];
            let e = seal(&key, 9, "account", "words", 42, &data).unwrap();
            assert_eq!(envelope_kid(&e).unwrap(), 9);
            assert_eq!(*open(&key, "account", "words", 42, &e).unwrap(), data);
            assert_eq!(e.len() % 4096, 0);
        }
    }
    #[test]
    fn every_envelope_field_is_authenticated() {
        let key = Secret::random();
        let e = seal(&key, 1, "a", "words", 1, b"private words").unwrap();
        for index in [0, 4, 5, 9, 20, 21, 30, e.len() - 1] {
            let mut bad = e.clone();
            bad[index] ^= 1;
            assert!(open(&key, "a", "words", 1, &bad).is_err());
        }
        for cut in [0, 5, 20, 100, e.len() - 1] {
            assert!(open(&key, "a", "words", 1, &e[..cut]).is_err());
        }
        assert!(open(&key, "other", "words", 1, &e).is_err());
        assert!(open(&key, "a", "other", 1, &e).is_err());
        assert!(open(&key, "a", "words", 2, &e).is_err());
        assert!(open(&Secret::random(), "a", "words", 1, &e).is_err());
    }
    #[test]
    fn randomized_nonce_and_size_limits() {
        let key = Secret::random();
        assert_ne!(
            seal(&key, 1, "a", "w", 1, b"x").unwrap(),
            seal(&key, 1, "a", "w", 1, b"x").unwrap()
        );
        assert!(seal(&key, 1, "a", "w", 0, b"x").is_err());
        assert!(seal(&key, 1, "a", "w", 1, &vec![0; MAX_ENVELOPE]).is_err());
    }
    #[test]
    fn keyring_rotation_preserves_old_keys() {
        let ak = Secret::random();
        let mut ring = Keyring::new();
        let old = ring.keys[&1].0;
        ring.rotate().unwrap();
        let e = ring.seal(&ak, "a", 2).unwrap();
        let r = Keyring::open(&ak, "a", 2, &e).unwrap();
        assert_eq!(r.active, 2);
        assert_eq!(r.keys[&1].0, old);
        assert!(Keyring::open(&ak, "a", 3, &e).is_err());
    }
    #[test]
    fn hpke_wrap_roundtrip_and_binding() {
        let d = DeviceKey::new();
        let e = wrap_device(&d.public, "a", "pair/one", b"account key").unwrap();
        assert_eq!(
            &**unwrap_device(&d, "a", "pair/one", &e).unwrap(),
            b"account key"
        );
        assert!(unwrap_device(&d, "a", "pair/two", &e).is_err());
        assert!(unwrap_device(&DeviceKey::new(), "a", "pair/one", &e).is_err());
        let mut bad = e;
        bad[35] ^= 1;
        assert!(unwrap_device(&d, "a", "pair/one", &bad).is_err());
        assert!(wrap_device(&[0; 32], "a", "p", b"x").is_err());
    }
    #[test]
    fn recovery_roundtrip_and_checksum() {
        let code = recovery_code();
        let ak = Secret::random();
        let e = wrap_recovery(&code, "a", &ak).unwrap();
        assert_eq!(unwrap_recovery(&code, "a", &e).unwrap().0, ak.0);
        assert!(unwrap_recovery(&code, "b", &e).is_err());
        assert_eq!(unwrap_recovery(&recovery_code(), "a", &e).err().as_deref(), Some(WRONG_RECOVERY));
        assert_eq!(unwrap_recovery("SJ1-no", "a", &e).err().as_deref(), Some("Recovery code is invalid."));
        assert!(recovery_bytes("SJ1-no").is_err());
        assert!(recovery_bytes(&code.replace("SJ1", "SJ2")).is_err());
        assert!(recovery_bytes(&"é".repeat(30)).is_err());
    }
    #[test]
    fn published_envelope_vector() {
        let v: serde_json::Value =
            serde_json::from_str(include_str!("../tests/fixtures/sync-vectors.json")).unwrap();
        let key = Secret([0x11; 32]);
        let e = seal_nonce(
            &key,
            7,
            "vector-account",
            "vocabulary",
            3,
            b"{\"schema\":1,\"entries\":[]}",
            [0x22; 12],
        )
        .unwrap();
        let actual = Sha256::digest(&e)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();
        assert_eq!(actual, v["envelopeSha256"]);
    }
}
