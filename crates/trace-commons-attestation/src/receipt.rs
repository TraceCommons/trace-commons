//! Verification of a NEAR AI inference receipt.
//!
//! Quote verification (the server's `near_attestation::quote`) establishes
//! that the *endpoint* is a genuine Intel TDX enclave running an image we
//! pinned. That says nothing about any particular inference. A receipt is the
//! other half: alongside a completion, NEAR AI returns a short `text` carrying
//! the SHA-256 of the request body and of the response body, signed by the
//! enclave's signing key. Verifying it binds one specific request/response
//! pair to that key.
//!
//! The mechanism follows NEAR AI's own reference verifier
//! (`nearai/nearai-cloud-verifier`, `py/chat_verifier.py`):
//!
//! 1. `text` splits on `:` into two or three parts. With three, the hashes are
//!    `parts[1]` and `parts[2]` -- a leading part shifts them. With two, they
//!    are `parts[0]` and `parts[1]`.
//! 2. Both are SHA-256 hex: of the request body *as sent*, and of the
//!    **entire raw response body** as received.
//! 3. `signature` is an EIP-191 `personal_sign` over `text`:
//!    `keccak256("\x19Ethereum Signed Message:\n" + len(text) + text)`, then
//!    secp256k1 public-key recovery. The signer's Ethereum address is the
//!    last 20 bytes of `keccak256(uncompressed_pubkey[1..])`.
//! 4. The recovered address must equal `signing_address`, case-insensitively.
//!
//! Two places where this departs from that reference verifier, both settled
//! by a real captured triple rather than by reading:
//!
//! - The second hash is over the **whole response body bytes**, not over
//!   `choices[0].message.content`. Reading the parsed content instead is a
//!   verifier that always fails; against a thinking model whose `content` is
//!   `null` it does not even parse. `crates/trace-commons-server/tests/
//!   near_ai_live_receipt.rs` pins this against real bytes.
//! - The three-part form's leading part is the **model name**, not an opaque
//!   request identifier. The reference verifier discards it; this one checks
//!   it against the model the caller asked for, so the receipt binds the
//!   model as well as the bytes. A mismatch is
//!   [`ReceiptError::ModelMismatch`] -- a receipt for a completion some other
//!   model served, which is exactly the substitution nobody would otherwise
//!   notice.
//!
//! Two deliberate choices where this is stricter or looser than the prose:
//!
//! - The hash fields are compared as *decoded bytes*, so an upper- or
//!   mixed-case hex hash from some future provider build verifies rather than
//!   being refused as malformed. The comparison is still exact.
//! - The recovery byte is accepted in both encodings, 27/28 (Ethereum) and
//!   0/1 (raw ECDSA). A verifier that handled only one would reject valid
//!   receipts from a provider using the other, and that failure would only
//!   show up in production against live data we cannot replay.
//!
//! Nothing in this module may be logged. The `text`, the signature, the
//! signing address and the request and response bodies are all caller data;
//! errors here name a condition and carry no payload beyond a part count or a
//! recovery byte.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha2::{Digest as _, Sha256};
use sha3::Keccak256;

/// A receipt as the provider returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptPayload {
    /// The signed text: two or three `:`-separated parts.
    pub text: String,
    /// 65-byte secp256k1 signature, hex, optionally `0x`-prefixed.
    pub signature: String,
    /// The address the provider claims signed it, hex, `0x`-prefixed.
    pub signing_address: String,
}

/// What a verified receipt establishes.
///
/// The hashes are re-rendered from the verified receipt in lowercase hex. The
/// address is the *recovered* signer, not the claimed one -- they are equal by
/// the time this exists. It is here so a caller can bind a receipt to a known
/// enclave key; it must not reach a log line or an audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptVerdict {
    pub request_sha256: String,
    pub response_sha256: String,
    pub signing_address: String,
    /// The model the receipt binds, when it carried one. `None` for the
    /// two-part form, which binds no model at all.
    pub model: Option<String>,
}

/// Why a receipt was refused.
///
/// Each variant names one specific condition. A receipt that is *malformed*
/// and one that is *validly signed but bound to different content* are
/// different failures with different operational meanings, and callers must be
/// able to tell them apart.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReceiptError {
    /// `text` did not split into two or three `:`-separated parts.
    #[error("receipt text has {parts} colon-separated parts, expected 2 or 3")]
    TextPartCount { parts: usize },
    /// The request-hash position is not 32 bytes of hex.
    #[error("receipt request hash is not 64 hex characters")]
    RequestHashMalformed,
    /// The response-hash position is not 32 bytes of hex.
    #[error("receipt response hash is not 64 hex characters")]
    ResponseHashMalformed,
    /// The signature is not 65 bytes of hex.
    #[error("receipt signature is not 65 bytes of hex")]
    SignatureMalformed,
    /// The 65th signature byte is neither 0/1 nor 27/28.
    #[error("receipt signature recovery byte {v} is neither 0/1 nor 27/28")]
    RecoveryIdUnsupported { v: u8 },
    /// No public key recovers from this signature over this digest.
    #[error("no signer recovers from the receipt signature")]
    SignatureUnrecoverable,
    /// `signing_address` is not a 20-byte hex address.
    #[error("receipt signing address is not a 20-byte hex address")]
    SigningAddressMalformed,
    /// The signature verifies, but for a different key than claimed.
    #[error("receipt was signed by a different key than the one claimed")]
    SignerMismatch,
    /// The receipt is validly signed, but not over this request body.
    #[error("receipt request hash does not match the request body")]
    RequestHashMismatch,
    /// The receipt is validly signed, but not over this response body.
    #[error("receipt response hash does not match the response body")]
    ResponseHashMismatch,
    /// The receipt is validly signed, but names a different model than the
    /// one the caller asked for.
    ///
    /// Carries neither name: the requested model is configuration and the
    /// bound one is provider data, and this module puts no payload in an
    /// error.
    #[error("receipt binds a different model than the one requested")]
    ModelMismatch,
}

/// Verify a receipt against the request body as sent and the response body as
/// received.
///
/// Both must be the exact bytes on the wire. Re-serializing the request from a
/// parsed form changes its digest, and passing anything read *out* of the
/// response -- the assistant message content in particular -- is not what the
/// receipt hashes.
///
/// `expected_model` is the model the caller asked for. It is compared against
/// the receipt's leading part when there is one; a two-part receipt binds no
/// model and `expected_model` is then unused.
pub fn verify_receipt(
    payload: &ReceiptPayload,
    request_body: &[u8],
    response_body: &[u8],
    expected_model: &str,
) -> Result<ReceiptVerdict, ReceiptError> {
    let parts: Vec<&str> = payload.text.split(':').collect();
    let (bound_model, request_hex, response_hex) = match parts.len() {
        2 => (None, parts[0], parts[1]),
        3 => (Some(parts[0]), parts[1], parts[2]),
        n => return Err(ReceiptError::TextPartCount { parts: n }),
    };

    let signed_request_hash =
        decode_sha256_hex(request_hex).ok_or(ReceiptError::RequestHashMalformed)?;
    let signed_response_hash =
        decode_sha256_hex(response_hex).ok_or(ReceiptError::ResponseHashMalformed)?;
    let claimed_address =
        decode_address(&payload.signing_address).ok_or(ReceiptError::SigningAddressMalformed)?;

    let recovered = recover_eip191_signer(payload.text.as_bytes(), &payload.signature)?;
    if recovered != claimed_address {
        return Err(ReceiptError::SignerMismatch);
    }

    if Sha256::digest(request_body).as_slice() != signed_request_hash {
        return Err(ReceiptError::RequestHashMismatch);
    }
    if Sha256::digest(response_body).as_slice() != signed_response_hash {
        return Err(ReceiptError::ResponseHashMismatch);
    }
    // Last, so a receipt bound to different bytes is reported as that rather
    // than as a model problem: the bytes are the stronger statement.
    if let Some(model) = bound_model {
        if model != expected_model {
            return Err(ReceiptError::ModelMismatch);
        }
    }

    Ok(ReceiptVerdict {
        request_sha256: hex::encode(signed_request_hash),
        response_sha256: hex::encode(signed_response_hash),
        signing_address: format!("0x{}", hex::encode(recovered)),
        model: bound_model.map(str::to_string),
    })
}

/// The EIP-191 `personal_sign` digest of `message`.
///
/// The length in the preamble is the **byte** length rendered as decimal
/// ASCII, not the character count. For any message outside ASCII the two
/// differ, and a verifier that used the character count would recover a
/// different -- and therefore rejected -- signer.
fn eip191_digest(message: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"\x19Ethereum Signed Message:\n");
    hasher.update(message.len().to_string().as_bytes());
    hasher.update(message);
    hasher.finalize().into()
}

/// Recover the 20-byte Ethereum address that produced `signature_hex` over
/// `message` under EIP-191.
pub fn recover_eip191_signer(
    message: &[u8],
    signature_hex: &str,
) -> Result<[u8; 20], ReceiptError> {
    let raw = hex::decode(strip_0x(signature_hex)).map_err(|_| ReceiptError::SignatureMalformed)?;
    if raw.len() != 65 {
        return Err(ReceiptError::SignatureMalformed);
    }
    let v = raw[64];
    let recovery_byte = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        other => return Err(ReceiptError::RecoveryIdUnsupported { v: other }),
    };
    let recovery_id =
        RecoveryId::from_byte(recovery_byte).ok_or(ReceiptError::RecoveryIdUnsupported { v })?;
    let signature =
        Signature::from_slice(&raw[..64]).map_err(|_| ReceiptError::SignatureMalformed)?;

    let digest = eip191_digest(message);
    let key = VerifyingKey::recover_from_prehash(&digest, &signature, recovery_id)
        .map_err(|_| ReceiptError::SignatureUnrecoverable)?;

    Ok(address_of(&key))
}

/// The Ethereum address of a secp256k1 public key: the last 20 bytes of the
/// keccak256 of the uncompressed encoding with its `0x04` tag removed.
fn address_of(key: &VerifyingKey) -> [u8; 20] {
    let point = key.to_encoded_point(false);
    let digest = Keccak256::digest(&point.as_bytes()[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&digest[12..]);
    address
}

fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

/// Decode a 32-byte hex digest, in either case. `None` if it is not one.
fn decode_sha256_hex(s: &str) -> Option<[u8; 32]> {
    if s.len() != 64 {
        return None;
    }
    let bytes = hex::decode(s).ok()?;
    bytes.try_into().ok()
}

/// Re-exported from [`crate::address`], which is outside this feature.
///
/// Decoding an address is hex; recovering one is a curve. Callers that only
/// need the former must not have to enable `receipt` to get it, so the
/// function lives there and is re-exported here for every existing caller.
pub use crate::address::decode_address;

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    /// Fixed test keys. Deliberately constants and never generated: a random
    /// key makes a failure unreproducible, and every input to these tests has
    /// to be pinned rather than assumed.
    const SIGNER_KEY_HEX: &str = "4c0883a69102937d6231471b5dbb6204fe5129617082792ae468d01a3f362318";
    const IMPOSTOR_KEY_HEX: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";

    const MODEL: &str = "Qwen/Qwen3.6-27B-FP8";
    const REQUEST_BODY: &[u8] = br#"{"model":"qwen3","messages":[{"role":"user","content":"hi"}]}"#;
    /// The *whole* response body, not the assistant content read out of it.
    /// The content here is `null`, as a thinking model's is, so a verifier
    /// that reached for `choices[0].message.content` could not even produce a
    /// string to hash.
    const RESPONSE_BODY: &[u8] =
        br#"{"choices":[{"message":{"content":null,"reasoning_content":"hm","role":"assistant"}}],"id":"c1"}"#;

    /// The common case: this request, this response, this model.
    fn verify(payload: &ReceiptPayload) -> Result<ReceiptVerdict, ReceiptError> {
        verify_receipt(payload, REQUEST_BODY, RESPONSE_BODY, MODEL)
    }

    /// Which encoding of the recovery byte to put in the 65th position.
    #[derive(Clone, Copy)]
    enum VEncoding {
        /// 27/28, as Ethereum wallets emit.
        Ethereum,
        /// 0/1, as raw ECDSA recovery ids.
        Raw,
    }

    fn key(hex_bytes: &str) -> SigningKey {
        SigningKey::from_slice(&hex::decode(hex_bytes).unwrap()).unwrap()
    }

    fn address_string(k: &SigningKey) -> String {
        format!("0x{}", hex::encode(address_of(k.verifying_key())))
    }

    fn sign(k: &SigningKey, text: &str, encoding: VEncoding) -> String {
        let digest = eip191_digest(text.as_bytes());
        let (signature, recovery_id) = k.sign_prehash_recoverable(&digest).unwrap();
        let mut raw = signature.to_bytes().to_vec();
        raw.push(match encoding {
            VEncoding::Ethereum => recovery_id.to_byte() + 27,
            VEncoding::Raw => recovery_id.to_byte(),
        });
        format!("0x{}", hex::encode(raw))
    }

    fn sha256_hex(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    fn two_part_text() -> String {
        format!("{}:{}", sha256_hex(REQUEST_BODY), sha256_hex(RESPONSE_BODY))
    }

    /// The form the live service actually returns: model, then both hashes.
    fn three_part_text(model: &str) -> String {
        format!(
            "{}:{}:{}",
            model,
            sha256_hex(REQUEST_BODY),
            sha256_hex(RESPONSE_BODY)
        )
    }

    /// A receipt over `text`, signed by the signer key with the Ethereum
    /// recovery encoding.
    fn receipt_over(text: &str) -> ReceiptPayload {
        let k = key(SIGNER_KEY_HEX);
        ReceiptPayload {
            text: text.to_string(),
            signature: sign(&k, text, VEncoding::Ethereum),
            signing_address: address_string(&k),
        }
    }

    // ---- Known answers -------------------------------------------------
    //
    // Everything else in this module is self-consistent: the tests sign with
    // `sign`, which calls `eip191_digest`, and compare against
    // `address_string`, which calls `address_of`. A receipt signed and
    // verified by the same two wrong functions still round-trips, so the
    // whole suite passed with `address_of` slicing `digest[..20]` instead of
    // `digest[12..]`, and again with the EIP-191 preamble removed. The
    // workspace caught both -- but only through a server-side test that
    // checks recovery against a real NEAR AI address, and that test is behind
    // the AGPL boundary. A third party vendoring this crate on its own, which
    // is the reason it exists, had nothing.
    //
    // The constants below are therefore taken from published sources and not
    // produced by this code. A vector we generated ourselves would move the
    // circularity rather than break it.

    /// Published key/address pair: the `privateKeyToAccount` example in the
    /// web3.js documentation.
    const WEB3_DOCS_KEY: &str = "348ce564d427a3311b6536bbcff9390d69395b06ed6c486954e971d960fe8709";
    /// The address that example prints, EIP-55 checksummed as published.
    const WEB3_DOCS_ADDRESS: &str = "0xb8CE9ab6943e0eCED004cDe8e3bBed6568B2Fa01";

    /// Second, independent key/address pair: Hardhat Network's account #0,
    /// derived from the published mnemonic "test test test test test test
    /// test test test test test junk" at m/44'/60'/0'/0/0.
    const HARDHAT_ACCOUNT_0_KEY: &str =
        "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
    const HARDHAT_ACCOUNT_0_ADDRESS: &str = "0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266";

    /// Published `personal_sign` digests: `web3.eth.accounts.hashMessage`
    /// examples from the web3.js documentation.
    const HASH_MESSAGE_HELLO_WORLD_CAPS: &str =
        "a1de988600a42c4b4ab089b619297c17d53cffae5d5120d82d8a92d0bb3b78f2";
    const HASH_MESSAGE_HELLO_WORLD: &str =
        "8144a6fa26be252b86456491fbcd43c1de7e022241845ffea1c3df066f7cfede";
    /// The same documentation's `skipPrefix: true` output for "Hello world":
    /// the bare keccak256 of the message, with no EIP-191 preamble. This is
    /// what `eip191_digest` must *not* produce.
    const KECCAK_HELLO_WORLD_UNPREFIXED: &str =
        "ed6c11b0b5b808960df26f5bfc471d04c1995b0ffd2055925ad1be28d6baadfd";

    #[test]
    fn address_derivation_matches_published_key_address_pairs() {
        // Two pairs from two unrelated sources. One could in principle be
        // mistranscribed; two agreeing with the same derivation could not.
        for (label, key_hex, published) in [
            ("web3.js docs", WEB3_DOCS_KEY, WEB3_DOCS_ADDRESS),
            (
                "hardhat account #0",
                HARDHAT_ACCOUNT_0_KEY,
                HARDHAT_ACCOUNT_0_ADDRESS,
            ),
        ] {
            let derived = address_string(&key(key_hex));
            // The published forms are EIP-55 checksummed and `address_of`
            // emits lowercase hex; the bytes are what is being asserted.
            assert!(
                derived.eq_ignore_ascii_case(published),
                "{label}: derived {derived}, published {published}"
            );
        }
        // And the two are different addresses, so the loop cannot be passing
        // by comparing one value against itself.
        assert!(!WEB3_DOCS_ADDRESS.eq_ignore_ascii_case(HARDHAT_ACCOUNT_0_ADDRESS));
    }

    #[test]
    fn eip191_digest_matches_published_personal_sign_hashes() {
        // `hashMessage` is `personal_sign`'s digest: what every wallet hashes
        // before signing. If ours differs by so much as the preamble, we
        // recover a different address from a real signature and reject an
        // honest signer.
        for (message, published) in [
            ("Hello World", HASH_MESSAGE_HELLO_WORLD_CAPS),
            ("Hello world", HASH_MESSAGE_HELLO_WORLD),
        ] {
            assert_eq!(
                hex::encode(eip191_digest(message.as_bytes())),
                published,
                "personal_sign digest of {message:?}"
            );
        }
        // The two messages differ only in one letter and hash differently, so
        // neither constant can be standing in for the other.
        assert_ne!(HASH_MESSAGE_HELLO_WORLD_CAPS, HASH_MESSAGE_HELLO_WORLD);
        // And the preamble is doing work: the same documentation's
        // skipPrefix output is the bare keccak256, which is what a digest
        // with the prefix dropped would collapse to.
        assert_ne!(
            hex::encode(eip191_digest(b"Hello world")),
            KECCAK_HELLO_WORLD_UNPREFIXED
        );
    }

    #[test]
    fn a_valid_receipt_verifies_and_binds_both_hashes() {
        let payload = receipt_over(&two_part_text());
        let verdict = verify(&payload).expect("verifies");
        assert_eq!(verdict.request_sha256, sha256_hex(REQUEST_BODY));
        assert_eq!(verdict.response_sha256, sha256_hex(RESPONSE_BODY));
        assert_eq!(
            verdict.signing_address,
            address_string(&key(SIGNER_KEY_HEX))
        );
        // A two-part receipt binds no model, and the verdict says so rather
        // than quietly reporting the one the caller asked for.
        assert_eq!(verdict.model, None);
    }

    #[test]
    fn the_response_hash_is_over_the_whole_body_not_the_message_content() {
        // The bug this replaced: hashing `choices[0].message.content`. Here
        // that field is `null`, so its stand-in is the empty string, and the
        // two digests are measured to differ rather than assumed to.
        let payload = receipt_over(&two_part_text());
        assert!(verify(&payload).is_ok());

        let content_digest = sha256_hex(b"");
        assert_ne!(content_digest, sha256_hex(RESPONSE_BODY));
        assert_eq!(
            verify_receipt(&payload, REQUEST_BODY, b"", MODEL).expect_err("refused"),
            ReceiptError::ResponseHashMismatch
        );
    }

    #[test]
    fn a_receipt_whose_request_hash_does_not_match_is_rejected() {
        // This is what stops a receipt being moved onto a different trace.
        let payload = receipt_over(&two_part_text());
        let err = verify_receipt(&payload, b"a different request body", RESPONSE_BODY, MODEL)
            .expect_err("must be refused");
        assert_eq!(err, ReceiptError::RequestHashMismatch);
    }

    #[test]
    fn a_receipt_whose_response_hash_does_not_match_is_rejected() {
        let payload = receipt_over(&two_part_text());
        let err = verify_receipt(&payload, REQUEST_BODY, b"a different completion", MODEL)
            .expect_err("must be refused");
        assert_eq!(err, ReceiptError::ResponseHashMismatch);
    }

    #[test]
    fn a_signature_by_a_different_key_is_rejected() {
        let text = two_part_text();
        let impostor = key(IMPOSTOR_KEY_HEX);
        let claimed = address_string(&key(SIGNER_KEY_HEX));
        // Measured, not assumed: the two keys really do have different addresses.
        assert_ne!(address_string(&impostor), claimed);
        let payload = ReceiptPayload {
            text: text.clone(),
            signature: sign(&impostor, &text, VEncoding::Ethereum),
            signing_address: claimed,
        };
        let err = verify(&payload).expect_err("must be refused");
        assert_eq!(err, ReceiptError::SignerMismatch);
    }

    #[test]
    fn the_three_part_form_reads_the_hashes_from_the_right_positions() {
        // Guards a real off-by-one: with a leading part the hashes shift, and
        // reading parts[0..2] would compare the leading part against the
        // request body and still "work" for the two-part case, so only this
        // test catches it.
        let payload = receipt_over(&three_part_text(MODEL));
        let verdict = verify(&payload).expect("verifies");
        assert_eq!(verdict.request_sha256, sha256_hex(REQUEST_BODY));
        assert_eq!(verdict.response_sha256, sha256_hex(RESPONSE_BODY));
        assert_eq!(verdict.model.as_deref(), Some(MODEL));
    }

    #[test]
    fn a_receipt_bound_to_a_different_model_is_rejected() {
        // The leading part is the model name, and checking it is what makes a
        // receipt unusable for a completion some other model served. A
        // verifier that discarded the part -- as NEAR AI's reference one does
        // -- would pass this.
        let other = "Qwen/Qwen3.6-35B-A3B-FP8";
        assert_ne!(other, MODEL);
        let payload = receipt_over(&three_part_text(other));
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::ModelMismatch
        );
    }

    #[test]
    fn a_model_mismatch_is_reported_only_once_the_bytes_agree() {
        // Both wrong: the caller must be told the bytes do not match, which is
        // the stronger statement and the one that changes what they do next.
        let payload = receipt_over(&three_part_text("some/other-model"));
        assert_eq!(
            verify_receipt(&payload, b"different bytes", RESPONSE_BODY, MODEL)
                .expect_err("refused"),
            ReceiptError::RequestHashMismatch
        );
    }

    #[test]
    fn a_text_with_one_or_four_parts_is_an_error_not_a_pass() {
        let one = sha256_hex(REQUEST_BODY);
        assert_eq!(one.split(':').count(), 1);
        let payload = receipt_over(&one);
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::TextPartCount { parts: 1 }
        );

        let four = format!(
            "{}:{}:{}:{}",
            sha256_hex(b"lead"),
            sha256_hex(b"extra"),
            sha256_hex(REQUEST_BODY),
            sha256_hex(RESPONSE_BODY)
        );
        let payload = receipt_over(&four);
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::TextPartCount { parts: 4 }
        );
    }

    #[test]
    fn both_recovery_byte_encodings_verify() {
        // A receipt that verified under only one of these would be a bug that
        // first appeared in production, against live data we cannot replay.
        let text = two_part_text();
        let k = key(SIGNER_KEY_HEX);
        let ethereum = sign(&k, &text, VEncoding::Ethereum);
        let raw = sign(&k, &text, VEncoding::Raw);
        // Measured: the two encodings really are different bytes here, so this
        // test is not silently signing the same thing twice.
        assert_ne!(ethereum, raw);

        for signature in [ethereum, raw] {
            let payload = ReceiptPayload {
                text: text.clone(),
                signature,
                signing_address: address_string(&k),
            };
            assert!(verify(&payload).is_ok());
        }
    }

    #[test]
    fn an_unsupported_recovery_byte_is_a_named_error() {
        let text = two_part_text();
        let mut payload = receipt_over(&text);
        let mut raw = hex::decode(strip_0x(&payload.signature)).unwrap();
        raw[64] = 5;
        payload.signature = format!("0x{}", hex::encode(raw));
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::RecoveryIdUnsupported { v: 5 }
        );
    }

    #[test]
    fn the_eip191_length_prefix_counts_bytes_not_characters() {
        // The leading part carries a character outside ASCII, so the byte
        // length and the character count of `text` differ. A verifier that
        // rendered the character count into the preamble digests something
        // else and recovers a different signer.
        let model = "vendor/mod\u{00e8}le-\u{4f60}";
        let text = three_part_text(model);
        // Measured, not reasoned: the two counts really do differ for this text.
        assert_ne!(text.len(), text.chars().count());

        let payload = receipt_over(&text);
        assert!(verify_receipt(&payload, REQUEST_BODY, RESPONSE_BODY, model).is_ok());

        // And the char-count preamble is genuinely a different digest, so the
        // assertion above is load-bearing.
        let mut char_count_hasher = Keccak256::new();
        char_count_hasher.update(b"\x19Ethereum Signed Message:\n");
        char_count_hasher.update(text.chars().count().to_string().as_bytes());
        char_count_hasher.update(text.as_bytes());
        let char_count_digest: [u8; 32] = char_count_hasher.finalize().into();
        assert_ne!(char_count_digest, eip191_digest(text.as_bytes()));
    }

    #[test]
    fn a_malformed_signature_is_distinguishable_from_a_rejected_one() {
        let text = two_part_text();
        let mut payload = receipt_over(&text);
        payload.signature = "0xdeadbeef".to_string();
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::SignatureMalformed
        );
    }

    #[test]
    fn a_malformed_signing_address_is_a_named_error() {
        let text = two_part_text();
        let mut payload = receipt_over(&text);
        payload.signing_address = "not-an-address".to_string();
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::SigningAddressMalformed
        );
    }

    #[test]
    fn a_hash_position_that_is_not_a_digest_is_named_for_its_position() {
        let response_hash = sha256_hex(RESPONSE_BODY);
        let payload = receipt_over(&format!("not-a-digest:{response_hash}"));
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::RequestHashMalformed
        );

        let request_hash = sha256_hex(REQUEST_BODY);
        let payload = receipt_over(&format!("{request_hash}:not-a-digest"));
        assert_eq!(
            verify(&payload).expect_err("refused"),
            ReceiptError::ResponseHashMalformed
        );
    }

    #[test]
    fn an_uppercase_hex_digest_still_verifies() {
        // Deliberately looser than the reference verifier's lowercase
        // assumption: the comparison is over decoded bytes, so a provider
        // build that emitted uppercase hex would not be refused as malformed.
        let text = format!(
            "{}:{}",
            sha256_hex(REQUEST_BODY).to_uppercase(),
            sha256_hex(RESPONSE_BODY).to_uppercase()
        );
        let payload = receipt_over(&text);
        let verdict = verify(&payload).expect("verifies");
        // The verdict re-renders in lowercase regardless of what came in.
        assert_eq!(verdict.request_sha256, sha256_hex(REQUEST_BODY));
    }

    #[test]
    fn the_claimed_address_is_compared_case_insensitively() {
        // EIP-55 checksummed addresses are mixed case; refusing them would
        // reject valid receipts.
        let text = two_part_text();
        let mut payload = receipt_over(&text);
        payload.signing_address = payload.signing_address.to_uppercase().replace("0X", "0x");
        assert!(payload.signing_address.contains(char::is_uppercase));
        assert!(verify(&payload).is_ok());
    }
}
