// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Login-with-NEAR (Slice 3a) account ceremony support.
//!
//! This module hosts the server-side NEAR sign-in flow: issuing a NEP-413
//! challenge, verifying the wallet-signed message against the configured
//! `recipient`, resolving the signing public key to a tenant, and minting the
//! account session. It is fail-closed and tenant-scoped like the rest of the
//! account surface.
//!
//! Task 4 fills in the cryptographic boundary: NEP-413 payload encoding,
//! Ed25519 signature verification, and the NEAR RPC access-key resolution. The
//! begin/finish handlers and DB wiring are filled in by later Slice 3a tasks
//! (Task 6+). The configuration (`config::NearConfig`) and the
//! `CeremonyState::NearChallenge` variant that this module consumes already
//! exist.

use anyhow::{Context, Result, anyhow};
use base64::Engine;

use crate::config::NearConfig;

/// NEP-413 domain-separation tag. A NEAR wallet `signMessage` prepends this
/// `u32` to the Borsh-serialized payload so that an off-chain login signature
/// can never collide with a real on-chain transaction signature. It is
/// `2^31 + 413`.
const NEP413_TAG: u32 = 2_147_484_061;

/// Borsh-serializable NEP-413 payload. Field order is load-bearing: Borsh emits
/// each field in declaration order, and a NEAR wallet signs `sha256(borsh(..))`
/// over exactly this layout. Do not reorder.
#[derive(borsh::BorshSerialize)]
struct Nep413Payload {
    /// Domain-separation tag (`NEP413_TAG`). Emitted as a `u32` little-endian.
    tag: u32,
    /// The human-readable challenge message the wallet displayed and signed.
    message: String,
    /// Server-issued 32-byte challenge nonce. Emitted as 32 raw bytes.
    nonce: [u8; 32],
    /// The `recipient` the signature is bound to (our login origin).
    recipient: String,
    /// Optional callback URL. `None` in our flow; pinned for layout fidelity.
    callback_url: Option<String>,
}

/// Parse a NEAR `ed25519:<base58>` public key string into its raw 32 bytes.
///
/// Rejects keys without the `ed25519:` prefix, malformed base58, or any decoded
/// length other than 32 bytes.
pub fn parse_near_ed25519_pubkey(s: &str) -> Result<[u8; 32]> {
    let rest = s
        .strip_prefix("ed25519:")
        .ok_or_else(|| anyhow!("unsupported NEAR key curve"))?;
    let bytes = bs58::decode(rest)
        .into_vec()
        .map_err(|_| anyhow!("malformed NEAR public key"))?;
    let arr: [u8; 32] = bytes
        .try_into()
        .map_err(|_| anyhow!("NEAR public key has wrong length"))?;
    Ok(arr)
}

/// Borsh-serialize a NEP-413 payload to the exact byte layout a NEAR wallet
/// signs over. The output is `u32 LE tag` followed by each subsequent field in
/// declaration order.
fn nep413_payload_bytes(
    message: &str,
    nonce: &[u8; 32],
    recipient: &str,
    callback_url: Option<&str>,
) -> Result<Vec<u8>> {
    let payload = Nep413Payload {
        tag: NEP413_TAG,
        message: message.to_string(),
        nonce: *nonce,
        recipient: recipient.to_string(),
        callback_url: callback_url.map(|s| s.to_string()),
    };
    // Borsh serialization of this fixed struct is infallible in practice; we
    // propagate rather than panic so the crypto module has zero panic paths.
    borsh::to_vec(&payload).context("NEP-413 payload serialization failed")
}

/// Verify a NEP-413 wallet signature.
///
/// Reconstructs the signed payload from `(message, nonce, recipient,
/// callback_url)`, takes its SHA-256 digest, and Ed25519-verifies the
/// base64-encoded signature against the supplied `ed25519:<base58>` public key.
///
/// Returns a generic `Err` on every failure path (bad key, bad signature
/// encoding, signature mismatch) so the verification boundary never leaks which
/// step failed.
///
/// NOTE (residual risk): the byte layout is canonically correct (borsh of the
/// documented NEP-413 struct + the `2^31+413` tag), validated by the
/// `nep413_payload_bytes_layout_pin` byte-pin and a ring sign/verify round-trip.
/// Those exercise only our own encoder. A captured real-wallet vector (a real
/// `{message, nonce, recipient, publicKey, signature}` produced by a NEAR wallet
/// / near-api-js) should be added as the gold-standard cross-check before first
/// contributor traffic. See the slice-3a spec residual-risks section.
pub fn verify_nep413(
    public_key: &str,
    message: &str,
    nonce: &[u8; 32],
    recipient: &str,
    callback_url: Option<&str>,
    signature_b64: &str,
) -> Result<()> {
    let public_key_bytes = parse_near_ed25519_pubkey(public_key)
        .map_err(|_| anyhow!("NEP-413 verification failed"))?;

    let payload = nep413_payload_bytes(message, nonce, recipient, callback_url)
        .map_err(|_| anyhow!("NEP-413 verification failed"))?;

    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(&payload);

    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| anyhow!("NEP-413 verification failed"))?;

    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &public_key_bytes)
        .verify(digest.as_slice(), &signature)
        .map_err(|_| anyhow!("NEP-413 verification failed"))
}

/// Pure predicate over a parsed `view_access_key_list` JSON-RPC response:
/// returns `true` iff some entry has `public_key == public_key` and its
/// `access_key.permission` is the string `"FullAccess"`.
///
/// A `FunctionCall` permission is encoded as an object (not the string
/// `"FullAccess"`), so a plain string-equality check correctly rejects
/// function-call-only keys. Absent keys and malformed JSON yield `false`.
pub fn key_list_has_full_access(rpc_json: &serde_json::Value, public_key: &str) -> bool {
    let Some(keys) = rpc_json
        .get("result")
        .and_then(|r| r.get("keys"))
        .and_then(|k| k.as_array())
    else {
        return false;
    };
    keys.iter().any(|entry| {
        entry.get("public_key").and_then(|p| p.as_str()) == Some(public_key)
            && entry
                .get("access_key")
                .and_then(|a| a.get("permission"))
                .and_then(|p| p.as_str())
                == Some("FullAccess")
    })
}

/// Resolve whether `public_key` is a FullAccess key on `account_id` by querying
/// the configured NEAR JSON-RPC endpoint's `view_access_key_list`.
///
/// Thin wrapper around [`key_list_has_full_access`]: any network or parse error
/// propagates as `Err`, which the Task 6 caller treats as fail-closed.
pub async fn near_account_has_full_access_key(
    cfg: &NearConfig,
    account_id: &str,
    public_key: &str,
) -> Result<bool> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": "tc",
        "method": "query",
        "params": {
            "request_type": "view_access_key_list",
            "finality": "final",
            "account_id": account_id,
        },
    });

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .context("NEAR RPC client build failed")?;
    let resp = client
        .post(&cfg.rpc_url)
        .json(&body)
        .send()
        .await
        .context("NEAR RPC request failed")?;
    let resp = resp
        .error_for_status()
        .context("NEAR RPC returned error status")?;
    let json: serde_json::Value = resp
        .json()
        .await
        .context("NEAR RPC response was not valid JSON")?;

    Ok(key_list_has_full_access(&json, public_key))
}

/// Test seam for the NEAR access-key binding check.
///
/// The enroll-finish handler must prove that the signing public key is a
/// FullAccess key on the named NEAR account, which in production means a live
/// `view_access_key_list` JSON-RPC call ([`near_account_has_full_access_key`]).
/// That network call cannot run in a hermetic test, so the handler consults an
/// optional override of this trait first and only falls back to the live call
/// when no override is present.
///
/// The production path does NOT install an override (the live RPC is used). The
/// override field is `#[cfg(test)]`-only on `AppState`, so this seam has zero
/// effect on — and adds zero surface to — the production binary.
#[async_trait::async_trait]
pub trait NearAccessKeyChecker: Send + Sync {
    /// Return `true` iff `public_key` is a FullAccess key on `account_id`.
    /// Any error is fail-closed by the caller (treated as "not proven").
    async fn has_full_access_key(
        &self,
        cfg: &NearConfig,
        account_id: &str,
        public_key: &str,
    ) -> Result<bool>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::KeyPair;

    fn test_keypair() -> ring::signature::Ed25519KeyPair {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
        ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn pubkey_string(kp: &ring::signature::Ed25519KeyPair) -> String {
        format!(
            "ed25519:{}",
            bs58::encode(kp.public_key().as_ref()).into_string()
        )
    }

    #[test]
    fn parse_near_ed25519_pubkey_roundtrip_and_rejects_malformed() {
        let kp = test_keypair();
        let s = pubkey_string(&kp);
        let parsed = parse_near_ed25519_pubkey(&s).unwrap();
        assert_eq!(parsed.as_slice(), kp.public_key().as_ref());

        // Missing prefix.
        assert!(parse_near_ed25519_pubkey(&bs58::encode([0u8; 32]).into_string()).is_err());
        // Bad base58 character ('0' and 'l' are not in the base58 alphabet).
        assert!(parse_near_ed25519_pubkey("ed25519:0l0l").is_err());
        // Decoded length != 32.
        let short = format!("ed25519:{}", bs58::encode([0u8; 16]).into_string());
        assert!(parse_near_ed25519_pubkey(&short).is_err());
    }

    #[test]
    fn nep413_payload_bytes_layout_pin() {
        let nonce = [1u8; 32];
        let got = nep413_payload_bytes("hi", &nonce, "app.example", None).unwrap();

        let mut expected: Vec<u8> = Vec::new();
        // tag = 2_147_484_061 = 2^31 + 413 = 0x8000_019D, little-endian.
        expected.extend_from_slice(&[0x9d, 0x01, 0x00, 0x80]);
        // message "hi": u32 LE len + bytes
        expected.extend_from_slice(&[0x02, 0x00, 0x00, 0x00, b'h', b'i']);
        // nonce: 32 raw bytes
        expected.extend_from_slice(&[0x01; 32]);
        // recipient "app.example": u32 LE len (11) + bytes
        expected.extend_from_slice(&[0x0b, 0x00, 0x00, 0x00]);
        expected.extend_from_slice(b"app.example");
        // callback_url: None
        expected.push(0x00);
        assert_eq!(got, expected, "NEP-413 None layout drifted");

        // callback_url Some("u"): 0x01 (Some) + u32 LE len (1) + 'u'
        let got_some = nep413_payload_bytes("hi", &nonce, "app.example", Some("u")).unwrap();
        let mut expected_some = expected.clone();
        expected_some.pop(); // drop the trailing None byte
        expected_some.extend_from_slice(&[0x01, 0x01, 0x00, 0x00, 0x00, b'u']);
        assert_eq!(got_some, expected_some, "NEP-413 Some layout drifted");
    }

    fn sign_nep413(
        kp: &ring::signature::Ed25519KeyPair,
        message: &str,
        nonce: &[u8; 32],
        recipient: &str,
        callback_url: Option<&str>,
    ) -> String {
        use sha2::{Digest, Sha256};
        let payload = nep413_payload_bytes(message, nonce, recipient, callback_url).unwrap();
        let digest = Sha256::digest(&payload);
        let sig = kp.sign(digest.as_slice());
        base64::engine::general_purpose::STANDARD.encode(sig.as_ref())
    }

    #[test]
    fn verify_nep413_roundtrip_and_tamper() {
        let kp = test_keypair();
        let pk = pubkey_string(&kp);
        let nonce = [7u8; 32];
        let recipient = "app.tracecommons.ai";
        let message = "login challenge";
        let sig = sign_nep413(&kp, message, &nonce, recipient, None);

        // Happy path.
        verify_nep413(&pk, message, &nonce, recipient, None, &sig).unwrap();

        // Tampered message.
        assert!(verify_nep413(&pk, "other", &nonce, recipient, None, &sig).is_err());
        // Tampered nonce.
        let mut bad_nonce = nonce;
        bad_nonce[0] ^= 0xff;
        assert!(verify_nep413(&pk, message, &bad_nonce, recipient, None, &sig).is_err());
        // Tampered recipient.
        assert!(verify_nep413(&pk, message, &nonce, "evil.example", None, &sig).is_err());
        // Tampered pubkey (different keypair).
        let other = pubkey_string(&test_keypair());
        assert!(verify_nep413(&other, message, &nonce, recipient, None, &sig).is_err());
        // Garbage signature.
        assert!(verify_nep413(&pk, message, &nonce, recipient, None, "not-base64!!").is_err());
    }

    #[test]
    fn verify_nep413_rejects_wrong_tag_signature() {
        // A signature produced over a payload with tag 0 (e.g. a forged or
        // non-NEP-413 structure) must NOT verify against the real-tag
        // reconstruction. This pins the domain-separation guarantee.
        let kp = test_keypair();
        let pk = pubkey_string(&kp);
        let nonce = [3u8; 32];
        let recipient = "app.tracecommons.ai";
        let message = "login challenge";

        // Hand-build a payload with tag = 0 and sign its digest.
        use sha2::{Digest, Sha256};
        let mut wrong_payload: Vec<u8> = Vec::new();
        wrong_payload.extend_from_slice(&0u32.to_le_bytes());
        wrong_payload.extend_from_slice(&(message.len() as u32).to_le_bytes());
        wrong_payload.extend_from_slice(message.as_bytes());
        wrong_payload.extend_from_slice(&nonce);
        wrong_payload.extend_from_slice(&(recipient.len() as u32).to_le_bytes());
        wrong_payload.extend_from_slice(recipient.as_bytes());
        wrong_payload.push(0x00);
        let digest = Sha256::digest(&wrong_payload);
        let sig =
            base64::engine::general_purpose::STANDARD.encode(kp.sign(digest.as_slice()).as_ref());

        assert!(verify_nep413(&pk, message, &nonce, recipient, None, &sig).is_err());
    }

    #[test]
    fn key_list_has_full_access_predicate() {
        let json = serde_json::json!({
            "result": {
                "keys": [
                    {
                        "public_key": "ed25519:full",
                        "access_key": { "permission": "FullAccess" }
                    },
                    {
                        "public_key": "ed25519:fc",
                        "access_key": { "permission": { "FunctionCall": { "allowance": null } } }
                    }
                ]
            }
        });

        assert!(key_list_has_full_access(&json, "ed25519:full"));
        assert!(!key_list_has_full_access(&json, "ed25519:fc"));
        assert!(!key_list_has_full_access(&json, "ed25519:absent"));
        assert!(!key_list_has_full_access(
            &serde_json::json!({}),
            "ed25519:full"
        ));
        assert!(!key_list_has_full_access(
            &serde_json::Value::Null,
            "ed25519:full"
        ));
    }
}
