//! The Insights judgement seal (design section 3).
//!
//! A contributor seals each judgement request to the service's per-boot
//! sealing key, which the service's quote binds (see
//! [`crate::report_data::ReportDataLayout::Judgement`]). The service's answer
//! comes back sealed under a key only that one request's sender can derive.
//! Neither the claim nor the digest nor the answer crosses dstack-gateway, or
//! any instance a request is misrouted to, in the clear.
//!
//! ```text
//! request  = 0x01 || eph_pk (33, SEC1 compressed) || ciphertext || tag (16)
//! response = 0x01 || ciphertext || tag (16)
//!
//! shared = x( eph_sk * seal_pk )                     (secp256k1, 32 bytes)
//! prk    = HKDF-Extract(salt = "trace_commons.insights_judgement.seal.v1", shared)
//! k_req  = HKDF-Expand(prk, "request"  || eph_pk || seal_pk, 32)
//! k_resp = HKDF-Expand(prk, "response" || eph_pk || seal_pk, 32)
//! AEAD   = ChaCha20-Poly1305, nonce = 12 zero bytes,
//!          AAD = "tcjudge1/request" || 0x01  or  "tcjudge1/response" || 0x01
//! ```
//!
//! **The zero nonce is sound only because every key seals exactly one
//! message.** `eph_sk` is fresh per request, so `k_req` and `k_resp` are too,
//! and each is used once in each direction. The API enforces that by type:
//! [`seal_request`] returns a [`ResponseOpener`] whose [`ResponseOpener::open`]
//! consumes it, and [`open_request`] returns a [`ResponseSealer`] whose
//! [`ResponseSealer::seal`] consumes it. Neither is `Clone`. Do not add a
//! reusable variant.
//!
//! **The types do not stop a replay.** [`open_request`] is a pure function:
//! a captured sealed request opened twice gives two sealers with the same
//! `k_resp`, and two responses sealed with them reuse the keystream and the
//! one-time Poly1305 key, enough to forge a response to the honest client.
//! So the service must record the request nonce (inside the opened
//! plaintext) before it calls [`ResponseSealer::seal`], must refuse a
//! nonce it has already recorded, and must never seal anything, an error
//! included, for a request it has not recorded. Per-response randomness in
//! the key schedule would remove this duty from the service; that is open
//! on the design spec.
//!
//! It is **not** RFC 9180 HPKE, which defines no secp256k1 KEM; it keeps
//! HPKE's base-mode shape (ephemeral-static DH, a key schedule bound to both
//! public keys, one AEAD message per key) using only `k256` and `ring`, which
//! this crate already depends on. The shared point is computed with k256's
//! default point arithmetic, not its `ecdh` feature. Frozen vectors computed
//! outside this crate pin the construction. Moving to an HPKE crate is open
//! decision S3 in the design.
//!
//! There is no length cap here. The service caps the body it reads before
//! calling [`open_request`], and the protocol crate caps the plaintext.

use k256::elliptic_curve::sec1::ToEncodedPoint;
use ring::{aead, hkdf, rand::SecureRandom};

use crate::report_data::SEAL_PUBLIC_KEY_LEN;

/// The first byte of every sealed request and response.
pub const SEAL_VERSION: u8 = 0x01;
/// The ChaCha20-Poly1305 tag length.
pub const SEAL_TAG_LEN: usize = 16;
/// Bytes a sealed request adds to its plaintext: version, `eph_pk`, tag.
pub const SEALED_REQUEST_OVERHEAD: usize = 1 + SEAL_PUBLIC_KEY_LEN + SEAL_TAG_LEN;
/// Bytes a sealed response adds to its plaintext: version, tag.
pub const SEALED_RESPONSE_OVERHEAD: usize = 1 + SEAL_TAG_LEN;

const HKDF_SALT: &[u8] = b"trace_commons.insights_judgement.seal.v1";
const REQUEST_INFO: &[u8] = b"request";
const RESPONSE_INFO: &[u8] = b"response";
const REQUEST_AAD: &[u8] = b"tcjudge1/request\x01";
const RESPONSE_AAD: &[u8] = b"tcjudge1/response\x01";
/// A uniformly random 32-byte string is a valid secp256k1 scalar with
/// probability about 1 - 2^-128, so this bound is never reached by a working
/// generator; it only stops a broken one from looping.
const KEY_GENERATION_ATTEMPTS: usize = 16;

/// Why a seal did not open, or could not be made. Label only: no variant
/// carries a byte of key, nonce, ciphertext or plaintext, and `Debug`
/// delegates to `Display`.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum SealError {
    /// The first byte is not [`SEAL_VERSION`].
    #[error("the sealed body has an unsupported seal version")]
    VersionUnsupported,
    /// Too short, or a public key that is not a point on secp256k1.
    #[error("the sealed body is malformed")]
    Malformed,
    /// The AEAD did not open: another key, another direction, or tampering.
    #[error("the sealed body does not open under this key")]
    DoesNotOpen,
    /// The system random source failed.
    #[error("the system random source failed")]
    KeyGeneration,
}

impl std::fmt::Debug for SealError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// The service's per-boot sealing public key, as the attestation route
/// returns it and the quote binds it. Always a valid, non-identity point.
#[derive(Clone, PartialEq, Eq)]
pub struct SealPublicKey(k256::PublicKey);

impl SealPublicKey {
    /// Parse a SEC1-compressed key. The identity, an uncompressed prefix and
    /// an x with no point on the curve are all [`SealError::Malformed`].
    pub fn from_bytes(bytes: &[u8; SEAL_PUBLIC_KEY_LEN]) -> Result<Self, SealError> {
        k256::PublicKey::from_sec1_bytes(bytes)
            .map(Self)
            .map_err(|_| SealError::Malformed)
    }

    /// The SEC1-compressed encoding, the bytes the boot binding hashes.
    pub fn to_bytes(&self) -> [u8; SEAL_PUBLIC_KEY_LEN] {
        let encoded = self.0.to_encoded_point(true);
        let mut bytes = [0u8; SEAL_PUBLIC_KEY_LEN];
        bytes.copy_from_slice(encoded.as_bytes());
        bytes
    }
}

impl std::fmt::Debug for SealPublicKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Public, but per boot and linkable to one instance; label only, as
        // the repo's diagnostics are.
        formatter.write_str("SealPublicKey(<withheld>)")
    }
}

/// The service's per-boot sealing secret. Generated in memory at boot, never
/// from `GetKey`, never written or logged. Not `Clone`.
pub struct SealSecretKey(k256::SecretKey);

impl SealSecretKey {
    /// A fresh key from the system random source.
    pub fn generate() -> Result<Self, SealError> {
        random_secret_key().map(Self)
    }

    /// The public half, for the attestation route and the boot binding.
    pub fn public_key(&self) -> SealPublicKey {
        SealPublicKey(self.0.public_key())
    }

    #[cfg(test)]
    fn from_test_bytes(bytes: &[u8; 32]) -> Self {
        Self(k256::SecretKey::from_slice(bytes).expect("a test scalar"))
    }
}

impl std::fmt::Debug for SealSecretKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SealSecretKey(<withheld>)")
    }
}

/// Held by the sender of one request; opens that request's response, once.
pub struct ResponseOpener(aead::LessSafeKey);

impl ResponseOpener {
    /// Open the response to the request that produced this opener. Consumes
    /// the opener: `k_resp` opens one message.
    pub fn open(self, sealed: &[u8]) -> Result<Vec<u8>, SealError> {
        if sealed.len() < SEALED_RESPONSE_OVERHEAD {
            return Err(SealError::Malformed);
        }
        if sealed[0] != SEAL_VERSION {
            return Err(SealError::VersionUnsupported);
        }
        open_in_place(&self.0, RESPONSE_AAD, sealed[1..].to_vec())
    }
}

impl std::fmt::Debug for ResponseOpener {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResponseOpener(<withheld>)")
    }
}

/// Held by the service for one opened request; seals that request's
/// response, once.
pub struct ResponseSealer(aead::LessSafeKey);

impl ResponseSealer {
    /// Seal the response. Consumes the sealer: `k_resp` seals one message.
    pub fn seal(self, plaintext: &[u8]) -> Result<Vec<u8>, SealError> {
        let mut sealed = Vec::with_capacity(plaintext.len() + SEALED_RESPONSE_OVERHEAD);
        sealed.push(SEAL_VERSION);
        sealed.extend_from_slice(&seal_in_place(&self.0, RESPONSE_AAD, plaintext.to_vec())?);
        Ok(sealed)
    }
}

impl std::fmt::Debug for ResponseSealer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ResponseSealer(<withheld>)")
    }
}

/// Seal a request to the service's sealing key under a fresh ephemeral key.
/// Returns the bytes to send and the one opener for the response.
pub fn seal_request(
    seal_public_key: &SealPublicKey,
    plaintext: &[u8],
) -> Result<(Vec<u8>, ResponseOpener), SealError> {
    seal_request_with_ephemeral(seal_public_key, plaintext, random_secret_key()?)
}

/// [`seal_request`] with the ephemeral key supplied, so the frozen vectors
/// can fix it. Private: a caller that chose `eph_sk` could reuse it, and a
/// reused `eph_sk` reuses `k_req` under a zero nonce.
fn seal_request_with_ephemeral(
    seal_public_key: &SealPublicKey,
    plaintext: &[u8],
    ephemeral: k256::SecretKey,
) -> Result<(Vec<u8>, ResponseOpener), SealError> {
    let eph_pk = SealPublicKey(ephemeral.public_key()).to_bytes();
    let seal_pk = seal_public_key.to_bytes();
    let (k_req, k_resp) = derive_keys(&ephemeral, &seal_public_key.0, &eph_pk, &seal_pk)?;
    drop(ephemeral);
    let mut sealed = Vec::with_capacity(plaintext.len() + SEALED_REQUEST_OVERHEAD);
    sealed.push(SEAL_VERSION);
    sealed.extend_from_slice(&eph_pk);
    sealed.extend_from_slice(&seal_in_place(&k_req, REQUEST_AAD, plaintext.to_vec())?);
    Ok((sealed, ResponseOpener(k_resp)))
}

/// Open a sealed request with the service's sealing secret. Returns the
/// plaintext and the one sealer for the response.
///
/// Opening the same bytes again returns a sealer with the same key. Record
/// the plaintext's request nonce, and refuse a repeat, before sealing
/// anything with it; see the module doc.
pub fn open_request(
    seal_secret_key: &SealSecretKey,
    sealed: &[u8],
) -> Result<(Vec<u8>, ResponseSealer), SealError> {
    if sealed.len() < SEALED_REQUEST_OVERHEAD {
        return Err(SealError::Malformed);
    }
    if sealed[0] != SEAL_VERSION {
        return Err(SealError::VersionUnsupported);
    }
    let mut eph_pk = [0u8; SEAL_PUBLIC_KEY_LEN];
    eph_pk.copy_from_slice(&sealed[1..1 + SEAL_PUBLIC_KEY_LEN]);
    let ephemeral = SealPublicKey::from_bytes(&eph_pk)?;
    let seal_pk = seal_secret_key.public_key().to_bytes();
    let (k_req, k_resp) = derive_keys(&seal_secret_key.0, &ephemeral.0, &eph_pk, &seal_pk)?;
    let plaintext = open_in_place(
        &k_req,
        REQUEST_AAD,
        sealed[1 + SEAL_PUBLIC_KEY_LEN..].to_vec(),
    )?;
    Ok((plaintext, ResponseSealer(k_resp)))
}

fn random_secret_key() -> Result<k256::SecretKey, SealError> {
    let rng = ring::rand::SystemRandom::new();
    for _ in 0..KEY_GENERATION_ATTEMPTS {
        let mut candidate = [0u8; 32];
        rng.fill(&mut candidate)
            .map_err(|_| SealError::KeyGeneration)?;
        let parsed = k256::SecretKey::from_slice(&candidate);
        candidate.fill(0);
        if let Ok(secret) = parsed {
            return Ok(secret);
        }
    }
    Err(SealError::KeyGeneration)
}

/// ECDH by point arithmetic, then the HKDF key schedule. Both sides call
/// this: the sender with (`eph_sk`, `seal_pk`), the service with (`seal_sk`,
/// `eph_pk`).
fn derive_keys(
    secret: &k256::SecretKey,
    peer: &k256::PublicKey,
    eph_pk: &[u8; SEAL_PUBLIC_KEY_LEN],
    seal_pk: &[u8; SEAL_PUBLIC_KEY_LEN],
) -> Result<(aead::LessSafeKey, aead::LessSafeKey), SealError> {
    let shared_point = (peer.to_projective() * *secret.to_nonzero_scalar()).to_affine();
    let encoded = shared_point.to_encoded_point(false);
    // Unreachable for a valid peer and a non-zero scalar on a prime-order
    // curve, and refused all the same: an identity has no x to derive from.
    if encoded.is_identity() {
        return Err(SealError::Malformed);
    }
    let shared_x = encoded.x().ok_or(SealError::Malformed)?;
    let prk = hkdf::Salt::new(hkdf::HKDF_SHA256, HKDF_SALT).extract(shared_x);
    let key = |label: &[u8]| -> Result<aead::LessSafeKey, SealError> {
        let info = [label, eph_pk.as_slice(), seal_pk.as_slice()];
        let okm = prk
            .expand(&info, &aead::CHACHA20_POLY1305)
            .map_err(|_| SealError::Malformed)?;
        Ok(aead::LessSafeKey::new(aead::UnboundKey::from(okm)))
    };
    Ok((key(REQUEST_INFO)?, key(RESPONSE_INFO)?))
}

/// Every key seals one message, so the nonce is fixed at zero. See the
/// module docs before calling this with a key that is not single-use.
fn zero_nonce() -> aead::Nonce {
    aead::Nonce::assume_unique_for_key([0u8; aead::NONCE_LEN])
}

fn seal_in_place(
    key: &aead::LessSafeKey,
    aad: &[u8],
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, SealError> {
    key.seal_in_place_append_tag(zero_nonce(), aead::Aad::from(aad), &mut buffer)
        .map_err(|_| SealError::Malformed)?;
    Ok(buffer)
}

fn open_in_place(
    key: &aead::LessSafeKey,
    aad: &[u8],
    mut buffer: Vec<u8>,
) -> Result<Vec<u8>, SealError> {
    let plaintext_len = key
        .open_in_place(zero_nonce(), aead::Aad::from(aad), &mut buffer)
        .map_err(|_| SealError::DoesNotOpen)?
        .len();
    buffer.truncate(plaintext_len);
    Ok(buffer)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Frozen vectors, computed outside this crate by Python's `cryptography`
    // (secp256k1 ECDH, HKDF-SHA256, ChaCha20-Poly1305) from the construction
    // in design section 3: seal_sk = 0x11 x 32, eph_sk = 0x22 x 32.
    const SEAL_SK: [u8; 32] = [0x11; 32];
    const EPH_SK: [u8; 32] = [0x22; 32];
    const SEAL_PK: &str = "034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";
    const EPH_PK: &str = "02466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27";
    const REQUEST_PLAINTEXT: &[u8] = br#"{"wire_version":1,"judgement":"vector"}"#;
    const RESPONSE_PLAINTEXT: &[u8] = br#"{"wire_version":1,"answer":"vector"}"#;
    const SEALED_REQUEST: &str = "0102466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f273394b368153b74a41a7172fa1bad084553e32c295265c570297962cab2827a9240a0f69e990bf892c3247434d260365d39a13733208800";
    const SEALED_RESPONSE: &str = "014e62ef9a785f1281f79053bada6136572f98813fe4f79796495855fbd05821467c06a50c9f5285ea9933145e2cfe90f097af2c25";

    fn seal_sk() -> SealSecretKey {
        SealSecretKey::from_test_bytes(&SEAL_SK)
    }

    fn eph_sk() -> k256::SecretKey {
        k256::SecretKey::from_slice(&EPH_SK).unwrap()
    }

    fn sealed_request() -> Vec<u8> {
        hex::decode(SEALED_REQUEST).unwrap()
    }

    #[test]
    fn the_constants_are_pinned_to_literals() {
        assert_eq!(SEAL_VERSION, 0x01);
        assert_eq!(SEAL_TAG_LEN, 16);
        assert_eq!(SEALED_REQUEST_OVERHEAD, 50);
        assert_eq!(SEALED_RESPONSE_OVERHEAD, 17);
    }

    #[test]
    fn the_public_keys_match_the_frozen_vector() {
        assert_eq!(hex::encode(seal_sk().public_key().to_bytes()), SEAL_PK);
        let eph = SealPublicKey(eph_sk().public_key());
        assert_eq!(hex::encode(eph.to_bytes()), EPH_PK);
    }

    #[test]
    fn a_sealed_request_matches_the_frozen_vector() {
        let (sealed, _opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), REQUEST_PLAINTEXT, eph_sk())
                .unwrap();
        assert_eq!(hex::encode(&sealed), SEALED_REQUEST);
        assert_eq!(
            sealed.len(),
            REQUEST_PLAINTEXT.len() + SEALED_REQUEST_OVERHEAD
        );
    }

    #[test]
    fn the_frozen_request_opens_and_its_response_matches_the_frozen_vector() {
        let (plaintext, sealer) = open_request(&seal_sk(), &sealed_request()).unwrap();
        assert_eq!(plaintext, REQUEST_PLAINTEXT);
        let response = sealer.seal(RESPONSE_PLAINTEXT).unwrap();
        assert_eq!(hex::encode(&response), SEALED_RESPONSE);
        assert_eq!(
            response.len(),
            RESPONSE_PLAINTEXT.len() + SEALED_RESPONSE_OVERHEAD
        );
    }

    #[test]
    fn the_frozen_response_opens_under_the_frozen_requests_opener() {
        let (_, opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), REQUEST_PLAINTEXT, eph_sk())
                .unwrap();
        let opened = opener.open(&hex::decode(SEALED_RESPONSE).unwrap()).unwrap();
        assert_eq!(opened, RESPONSE_PLAINTEXT);
    }

    #[test]
    fn a_round_trip_with_generated_keys() {
        let service = SealSecretKey::generate().unwrap();
        let (sealed, opener) = seal_request(&service.public_key(), b"request").unwrap();
        let (plaintext, sealer) = open_request(&service, &sealed).unwrap();
        assert_eq!(plaintext, b"request");
        let response = sealer.seal(b"response").unwrap();
        assert_eq!(opener.open(&response).unwrap(), b"response");
    }

    #[test]
    fn two_seals_of_one_plaintext_use_different_ephemeral_keys() {
        let service = SealSecretKey::generate().unwrap();
        let (one, _) = seal_request(&service.public_key(), b"same").unwrap();
        let (two, _) = seal_request(&service.public_key(), b"same").unwrap();
        assert_ne!(one[1..34], two[1..34]);
        assert_ne!(one, two);
    }

    #[test]
    fn two_generated_keys_differ() {
        let one = SealSecretKey::generate().unwrap().public_key().to_bytes();
        let two = SealSecretKey::generate().unwrap().public_key().to_bytes();
        assert_ne!(one, two);
    }

    #[test]
    fn a_tampered_ciphertext_or_tag_does_not_open() {
        let sealed = sealed_request();
        // First ciphertext byte, a middle one, and the last byte of the tag.
        for at in [34, 34 + REQUEST_PLAINTEXT.len() / 2, sealed.len() - 1] {
            let mut tampered = sealed.clone();
            tampered[at] ^= 0x01;
            assert_eq!(
                open_request(&seal_sk(), &tampered).err(),
                Some(SealError::DoesNotOpen),
                "byte {at}"
            );
        }
        let mut short_tag = sealed.clone();
        short_tag.pop();
        assert_eq!(
            open_request(&seal_sk(), &short_tag).err(),
            Some(SealError::DoesNotOpen)
        );
    }

    #[test]
    fn a_tampered_version_byte_is_refused() {
        for version in [0x00, 0x02, 0xff] {
            let mut tampered = sealed_request();
            tampered[0] = version;
            assert_eq!(
                open_request(&seal_sk(), &tampered).err(),
                Some(SealError::VersionUnsupported)
            );
        }
        let (_, opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), REQUEST_PLAINTEXT, eph_sk())
                .unwrap();
        let mut response = hex::decode(SEALED_RESPONSE).unwrap();
        response[0] = 0x02;
        assert_eq!(
            opener.open(&response).err(),
            Some(SealError::VersionUnsupported)
        );
    }

    #[test]
    fn a_tampered_ephemeral_key_does_not_open() {
        // Flipping the parity byte negates the point, which leaves the shared
        // x coordinate unchanged. The keys still differ, because both public
        // keys are in the HKDF info, so it still does not open.
        let mut negated = sealed_request();
        negated[1] ^= 0x01;
        assert_eq!(
            open_request(&seal_sk(), &negated).err(),
            Some(SealError::DoesNotOpen)
        );
        // Any other byte gives another point or none at all; neither opens.
        for at in [2, 17, 33] {
            let mut tampered = sealed_request();
            tampered[at] ^= 0x01;
            let err = open_request(&seal_sk(), &tampered).err();
            assert!(
                matches!(err, Some(SealError::DoesNotOpen | SealError::Malformed)),
                "byte {at}: {err:?}"
            );
        }
    }

    #[test]
    fn the_identity_and_malformed_points_are_refused() {
        // SEC1 cannot spell the identity in 33 bytes; a zero prefix is the
        // closest a 33-byte field gets, and it is refused. So is an
        // uncompressed prefix, and an x with no point on the curve
        // (x = 5 has none on secp256k1: 5^3 + 7 = 132 is not a square mod p).
        let mut off_curve = [0u8; 33];
        off_curve[0] = 0x02;
        off_curve[32] = 0x05;
        for bad in [[0u8; 33], [0x04; 33], off_curve] {
            assert_eq!(
                SealPublicKey::from_bytes(&bad).err(),
                Some(SealError::Malformed)
            );
            let mut sealed = sealed_request();
            sealed[1..34].copy_from_slice(&bad);
            assert_eq!(
                open_request(&seal_sk(), &sealed).err(),
                Some(SealError::Malformed)
            );
        }
    }

    #[test]
    fn short_inputs_are_malformed() {
        for len in [0, 1, 33, 34, SEALED_REQUEST_OVERHEAD - 1] {
            let sealed = &sealed_request()[..len];
            assert!(
                matches!(
                    open_request(&seal_sk(), sealed).err(),
                    Some(SealError::Malformed | SealError::VersionUnsupported)
                ),
                "length {len}"
            );
        }
        let (_, opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), REQUEST_PLAINTEXT, eph_sk())
                .unwrap();
        assert_eq!(
            opener.open(&[SEAL_VERSION; 16]).err(),
            Some(SealError::Malformed)
        );
    }

    #[test]
    fn an_empty_plaintext_seals_and_opens() {
        let (sealed, opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), b"", eph_sk()).unwrap();
        assert_eq!(sealed.len(), SEALED_REQUEST_OVERHEAD);
        let (plaintext, sealer) = open_request(&seal_sk(), &sealed).unwrap();
        assert!(plaintext.is_empty());
        assert!(opener.open(&sealer.seal(b"").unwrap()).unwrap().is_empty());
    }

    #[test]
    fn a_response_does_not_open_under_another_requests_keys() {
        let service = SealSecretKey::generate().unwrap();
        let (request_a, opener_a) = seal_request(&service.public_key(), b"a").unwrap();
        let (request_b, _opener_b) = seal_request(&service.public_key(), b"b").unwrap();
        let (_, _sealer_a) = open_request(&service, &request_a).unwrap();
        let (_, sealer_b) = open_request(&service, &request_b).unwrap();
        let response_b = sealer_b.seal(b"for b").unwrap();
        assert_eq!(
            opener_a.open(&response_b).err(),
            Some(SealError::DoesNotOpen)
        );
    }

    #[test]
    fn a_request_sealed_to_one_instance_does_not_open_at_another() {
        // The misrouting case: sealed to instance P's per-boot key, delivered
        // to instance M. M learns nothing, not even whether it was well formed
        // beyond its length.
        let instance_p = SealSecretKey::generate().unwrap();
        let instance_m = SealSecretKey::generate().unwrap();
        let (sealed, _) = seal_request(&instance_p.public_key(), b"for p only").unwrap();
        assert_eq!(
            open_request(&instance_m, &sealed).err(),
            Some(SealError::DoesNotOpen)
        );
    }

    #[test]
    fn a_request_ciphertext_does_not_open_as_its_response() {
        // A request's ciphertext reflected back as the response: k_req and
        // k_resp differ, and so do the two AADs, so it does not open.
        let (sealed, opener) =
            seal_request_with_ephemeral(&seal_sk().public_key(), REQUEST_PLAINTEXT, eph_sk())
                .unwrap();
        let mut as_response = vec![SEAL_VERSION];
        as_response.extend_from_slice(&sealed[34..]);
        assert_eq!(
            opener.open(&as_response).err(),
            Some(SealError::DoesNotOpen)
        );
    }

    #[test]
    fn nothing_renders_key_or_plaintext_bytes() {
        let key = seal_sk();
        let (sealed, opener) =
            seal_request_with_ephemeral(&key.public_key(), REQUEST_PLAINTEXT, eph_sk()).unwrap();
        let (_, sealer) = open_request(&key, &sealed).unwrap();
        for rendered in [
            format!("{key:?}"),
            format!("{opener:?}"),
            format!("{sealer:?}"),
            format!("{:?}", key.public_key()),
        ] {
            assert!(!rendered.contains("11111111"), "{rendered}");
            assert!(!rendered.contains("4f355bdc"), "{rendered}");
        }
        for err in [
            SealError::VersionUnsupported,
            SealError::Malformed,
            SealError::DoesNotOpen,
            SealError::KeyGeneration,
        ] {
            assert_eq!(format!("{err:?}"), format!("{err}"));
        }
    }
}
