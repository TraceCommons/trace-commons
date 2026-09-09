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

use sha2::{Digest as _, Sha256};

/// Which signature scheme a receipt carries.
///
/// NEAR AI issues both. Only the ed25519 signer is bound into a TDX quote --
/// the gateway's or the serving model's, depending on the receipt's
/// `signature_kind`; see [`ReceiptSignatureKind`]. The binding is
/// `report_data == signing_address || nonce` inside that quote, read at the
/// fixed v4 TDX offset rather than from a JSON field, because a
/// `model_attestations` entry carries no `report_data` field at all.
///
/// The ECDSA signer has no such binding available. Not because the report
/// omits ECDSA -- `signing_algo` is a query parameter of the report endpoint
/// and ECDSA is its *default* -- but because the ECDSA attestations that
/// query returns list the model enclave's own address, not the ECDSA signer
/// a receipt recovers to. Measured live 2026-09-05: attested `0xe5d0fec4..`,
/// receipt signer `0x614bc66f..`.
///
/// So `Ed25519` is the form that lets a verified receipt mean "signed inside
/// an attested enclave", and `Ecdsa` remains for receipts already issued and
/// for callers that have not moved.
///
/// Discriminated by the explicit wire field, never guessed from the length
/// of `signing_address`: a 20-byte address and a 32-byte key are
/// distinguishable by length, and a guess that happened to be right would
/// be a control that could not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptAlgo {
    /// EIP-191 secp256k1 recovery; `signing_address` is a 20-byte `0x` address.
    Ecdsa,
    /// Plain Ed25519 over the raw `text` bytes -- NOT EIP-191 prefixed;
    /// `signing_address` is the 32-byte public key, 64 hex chars, no `0x`.
    Ed25519,
}

impl ReceiptAlgo {
    /// The spelling NEAR AI uses in the `signing_algo` field and query param.
    pub fn as_wire(self) -> &'static str {
        match self {
            Self::Ecdsa => "ecdsa",
            Self::Ed25519 => "ed25519",
        }
    }

    /// Parse the wire spelling. Exact match; case is not folded, because a
    /// provider that started sending `ECDSA` would be sending something this
    /// crate has not seen, and silently accepting it is how a second spelling
    /// gets a second meaning.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "ecdsa" => Some(Self::Ecdsa),
            "ed25519" => Some(Self::Ed25519),
            _ => None,
        }
    }
}

/// Which attested key signed a receipt -- the *source* of the signer, not the
/// algorithm.
///
/// NEAR AI issues two legitimate kinds of receipt for the **same hosted
/// model**, and which one arrives is decided by the protocol the inference
/// call used rather than by the model:
///
/// | request protocol | `signature_kind` | signer |
/// |---|---|---|
/// | Chat Completions (`/v1/chat/completions`) | `provider_tee` | the per-model ed25519 key in the report's `model_attestations` |
/// | Responses API (`/v1/responses`) | `gateway` | the ed25519 key in the report's `gateway_attestation` |
///
/// Both were captured live against `Qwen/Qwen3.6-35B-A3B-FP8`. This matters
/// operationally rather than academically: the Codex CLI speaks the Responses
/// API exclusively -- it dropped `wire_api = "chat"` -- so a deployment whose
/// contributors use Codex sees only `gateway` receipts, and a verifier that
/// checks `model_attestations` alone refuses every one of them.
///
/// Each kind has **exactly one** correct key source. A `gateway` receipt
/// checked against a model key, or a `provider_tee` receipt checked against
/// the gateway key, must not verify: accepting either would let a key
/// attested for one role vouch for the other.
///
/// [`Self::Unrecognised`] is the explicit fail-closed arm. It covers a value
/// this crate has not seen *and* the absence of the field, and it is not a
/// licence to try every key: an unrecognised kind names no source, so a
/// receipt carrying one can be checked against nothing and is refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiptSignatureKind {
    /// Signed by the gateway's own attested ed25519 key. The Responses API
    /// form.
    Gateway,
    /// Signed by the serving model's per-model attested ed25519 key. The Chat
    /// Completions form.
    ProviderTee,
    /// Something else, or nothing at all. Refused wherever a key source has to
    /// be chosen.
    Unrecognised,
}

impl ReceiptSignatureKind {
    /// The spelling NEAR AI uses in the `signature_kind` field, or `None` for
    /// [`Self::Unrecognised`], which has no spelling to send.
    #[must_use]
    pub fn as_wire(self) -> Option<&'static str> {
        match self {
            Self::Gateway => Some("gateway"),
            Self::ProviderTee => Some("provider_tee"),
            Self::Unrecognised => None,
        }
    }

    /// Parse the wire spelling. Exact match, case not folded, for the reason
    /// [`ReceiptAlgo::from_wire`] gives.
    ///
    /// Anything else is [`Self::Unrecognised`] rather than an error, so every
    /// caller has to handle the refusing arm explicitly rather than an error
    /// type one of them could map to a default.
    #[must_use]
    pub fn from_wire(s: &str) -> Self {
        match s {
            "gateway" => Self::Gateway,
            "provider_tee" => Self::ProviderTee,
            _ => Self::Unrecognised,
        }
    }
}

/// A receipt as the provider returns it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptPayload {
    /// The signed text: two or three `:`-separated parts.
    pub text: String,
    /// 65-byte secp256k1 signature, hex, optionally `0x`-prefixed.
    pub signature: String,
    /// The address the provider claims signed it, hex, `0x`-prefixed.
    pub signing_address: String,
    /// Which scheme `signature` and `signing_address` are in. See
    /// [`ReceiptAlgo`] for why this is explicit rather than inferred.
    pub signing_algo: ReceiptAlgo,
    /// Which attested key the provider says signed it. See
    /// [`ReceiptSignatureKind`]: this decides which key source the signer is
    /// checked against, and it is read off the wire rather than guessed from
    /// the shape of `text` or the protocol we think we used.
    pub signature_kind: ReceiptSignatureKind,
}

/// What a verified receipt establishes.
///
/// The hashes are re-rendered from the verified receipt in lowercase hex. The
/// address's provenance depends on `signing_algo`: on `Ecdsa` it is the
/// *recovered* signer, not the claimed one -- they are equal by the time this
/// exists. On `Ed25519` there is no recovery; it is the *claimed* key, whose
/// signature verified against it. Same trust value -- a caller can bind a
/// receipt to a known enclave key either way -- reached by a different
/// mechanism. It must not reach a log line or an audit row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceiptVerdict {
    pub request_sha256: String,
    pub response_sha256: String,
    pub signing_address: String,
    /// The model the receipt binds, when it carried one. `None` for the
    /// two-part form, which binds no model at all.
    pub model: Option<String>,
    pub signing_algo: ReceiptAlgo,
    /// Carried through from the payload unchanged.
    ///
    /// A signature verifies against the key that made it and says nothing
    /// about which role that key holds, so this is the provider's *claim*
    /// about the signer's source -- the claim a caller resolves to a key set,
    /// never a fact the signature established. What makes it safe to route on
    /// is that each kind resolves to exactly one attested key set: a claim of
    /// the wrong kind sends the signer to a set it is not in, and the
    /// comparison fails.
    pub signature_kind: ReceiptSignatureKind,
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
    /// `signing_address` is not a 32-byte hex Ed25519 public key.
    #[error("receipt ed25519 key is not 32 bytes of hex")]
    Ed25519KeyMalformed,
    /// The signature is not 64 bytes of hex.
    #[error("receipt ed25519 signature is not 64 bytes of hex")]
    Ed25519SignatureMalformed,
    /// Well-formed key and signature, and the signature does not verify
    /// over the text. Tampering, a different signer, or a prefix that
    /// should not have been applied -- this variant cannot say which, and
    /// it carries nothing.
    #[error("receipt ed25519 signature does not verify")]
    Ed25519SignatureInvalid,
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

    // The signature check is scheme-specific; everything after it -- the
    // two digests and the model -- is not.
    let signing_address = match payload.signing_algo {
        ReceiptAlgo::Ecdsa => {
            let claimed = decode_address(&payload.signing_address)
                .ok_or(ReceiptError::SigningAddressMalformed)?;
            let recovered = recover_eip191_signer(payload.text.as_bytes(), &payload.signature)?;
            if recovered != claimed {
                return Err(ReceiptError::SignerMismatch);
            }
            format!("0x{}", hex::encode(recovered))
        }
        ReceiptAlgo::Ed25519 => {
            verify_ed25519_signature(payload)?;
            // Verified against the key as given; re-rendered lowercase so
            // two spellings of one key compare equal downstream.
            payload.signing_address.to_ascii_lowercase()
        }
    };

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
        signing_address,
        model: bound_model.map(str::to_string),
        signing_algo: payload.signing_algo,
        signature_kind: payload.signature_kind,
    })
}

/// Verify an ed25519 receipt's signature over its raw `text`.
///
/// Plain Ed25519, no EIP-191 prefix. Confirmed against a live NEAR AI
/// receipt: the raw text verifies, the prefixed form does not. Public
/// because a caller holding an attestation-report key may want the
/// signature check alone, without bodies.
///
/// The `0x` handling is asymmetric on purpose: `signature` accepts an
/// optional `0x` prefix, `signing_address` does not. NEAR AI renders
/// ed25519 keys without one, so a `0x`-prefixed key is not a spelling this
/// verifier tolerates -- it fails closed as [`ReceiptError::Ed25519KeyMalformed`].
///
/// # Errors
///
/// Malformed key or signature by name, before any cryptography runs; then
/// [`ReceiptError::Ed25519SignatureInvalid`] for a well-formed signature
/// that does not verify.
pub fn verify_ed25519_signature(payload: &ReceiptPayload) -> Result<(), ReceiptError> {
    let key =
        hex::decode(&payload.signing_address).map_err(|_| ReceiptError::Ed25519KeyMalformed)?;
    if key.len() != 32 {
        return Err(ReceiptError::Ed25519KeyMalformed);
    }
    let sig = hex::decode(
        payload
            .signature
            .strip_prefix("0x")
            .unwrap_or(&payload.signature),
    )
    .map_err(|_| ReceiptError::Ed25519SignatureMalformed)?;
    if sig.len() != 64 {
        return Err(ReceiptError::Ed25519SignatureMalformed);
    }
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(payload.text.as_bytes(), &sig)
        .map_err(|_| ReceiptError::Ed25519SignatureInvalid)
}

/// Recover the 20-byte Ethereum address that produced `signature_hex` over
/// `message` under EIP-191.
///
/// A thin wrapper over [`crate::eip191::recover_eip191_signer`], which is
/// where this lives now and is **not** behind this feature -- a
/// redaction-witness certificate is signed the same way and has nothing to do
/// with inference receipts. Kept here, mapping into [`ReceiptError`], so that
/// every existing caller and every existing `match` on a `ReceiptError`
/// variant is unchanged.
pub fn recover_eip191_signer(
    message: &[u8],
    signature_hex: &str,
) -> Result<[u8; 20], ReceiptError> {
    crate::eip191::recover_eip191_signer(message, signature_hex).map_err(ReceiptError::from)
}

impl From<crate::eip191::Eip191Error> for ReceiptError {
    fn from(error: crate::eip191::Eip191Error) -> Self {
        // Exhaustive rather than a catch-all, so a new variant over there is
        // a compile error here and gets a deliberate mapping instead of
        // silently becoming "malformed".
        match error {
            crate::eip191::Eip191Error::SignatureMalformed => ReceiptError::SignatureMalformed,
            crate::eip191::Eip191Error::RecoveryIdUnsupported { v } => {
                ReceiptError::RecoveryIdUnsupported { v }
            }
            crate::eip191::Eip191Error::SignatureUnrecoverable => {
                ReceiptError::SignatureUnrecoverable
            }
        }
    }
}

/// A gateway ed25519 signing key in the one spelling this crate compares:
/// 64 lowercase hex characters, no `0x`.
///
/// Surrounding whitespace is trimmed and case is folded, because those are
/// spellings of one key rather than different keys -- an operator pasting a
/// value out of an attestation report must not get a pin that can never
/// match. Everything else is refused: a `0x` prefix is not a spelling NEAR AI
/// renders, and a short or non-hex string could compare or `strip_prefix`
/// against something it is not.
///
/// `None` rather than an error type: every caller has its own name for a key
/// that is not one, and this function has nothing to add to it.
#[must_use]
pub fn normalize_ed25519_key(key: &str) -> Option<String> {
    let key = key.trim().to_ascii_lowercase();
    if key.len() != 64 || !key.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    Some(key)
}

/// Whether a verified receipt's signer is a given attested gateway key.
///
/// Case-insensitive, because a report renders the key lowercase and a receipt
/// might not. Exact otherwise: a prefix, a suffix, or another scheme's
/// address does not match. The empty-key guard is what stops an unset or
/// unparsed key from matching everything, which is the way this check would
/// fail open.
///
/// Lives here rather than beside either caller: the contributor compares a
/// receipt against a key it read from a live report, and the witness compares
/// one against a key an operator pinned. Same comparison, and it must not be
/// written twice.
#[must_use]
pub fn signer_is_attested(receipt_signer: &str, attested_key: &str) -> bool {
    !attested_key.is_empty() && receipt_signer.eq_ignore_ascii_case(attested_key)
}

/// Why a key could not be read out of an attestation report.
///
/// Carries no payload. A report holds keys and identifiers, and an error
/// string is the one place they must not end up.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum AttestedKeyError {
    /// The report, or the attestation object inside it, is not the shape this
    /// reader expects.
    #[error("attestation report is not the expected JSON shape")]
    Malformed,
    /// The attestation's `signing_algo` is not `ed25519`.
    #[error("attestation is not ed25519")]
    NotEd25519,
    /// `report_data` committed to a nonce other than the one we sent.
    #[error("attestation was issued for a different nonce")]
    NonceMismatch,
    /// `report_data` does not begin with the `signing_address` it lists.
    #[error("attestation report_data does not commit to the listed key and nonce")]
    ReportDataMismatch,
    /// `signing_address` is not 32 bytes of hex.
    #[error("attestation signing key is not 32 bytes of hex")]
    KeyMalformed,
    /// The report carries no ed25519 attestation for the model asked about.
    #[error("attestation report carries no ed25519 attestation for this model")]
    ModelNotAttested,
    /// The receipt named a `signature_kind` this crate does not recognise, or
    /// named none at all, so no attested key source could be chosen for it.
    ///
    /// Refused rather than answered by trying every source: see
    /// [`attested_keys_for_receipt`].
    #[error("receipt signature kind names no attested key source")]
    SignatureKindUnrecognised,
}

/// Byte offset of `report_data` inside a v4 TDX quote: the 48-byte quote
/// header, then TDReport10's 520 bytes of SVNs and measurements. It is 64
/// bytes long. The same offset `quote.rs`'s tests read, and the reason it can
/// be read without verifying anything is that it is a fixed field position,
/// not a parse.
const QUOTE_REPORT_DATA_OFFSET: usize = 568;
const QUOTE_REPORT_DATA_LEN: usize = 64;

/// `report_data` as the attestation's own `intel_quote` carries it, lowercase
/// hex, or `None` when there is no `intel_quote` field at all.
///
/// `Some(Err(..))` is not modelled: a quote that is present but cannot be read
/// at this offset returns `None` from the hex decode path only after the
/// caller has decided what an unreadable quote means. See
/// [`attested_ed25519_key`], which refuses it.
///
/// The version and TEE type are checked before the offset is trusted. 568 is
/// where `report_data` lives in a **v4 TDX** quote; in some other quote
/// version or TEE those bytes are a different field, and reading them anyway
/// would be inventing a binding out of unrelated measurement bytes.
fn quote_report_data(attestation: &serde_json::Value) -> Option<Result<String, AttestedKeyError>> {
    let quote_hex = attestation.get("intel_quote")?.as_str()?;
    let read = || {
        let quote = hex::decode(quote_hex).ok()?;
        // Header: u16 LE version, u16 LE attestation key type, u32 LE TEE
        // type. 0x81 is TDX.
        let version = u16::from_le_bytes([*quote.first()?, *quote.get(1)?]);
        let tee_type = u32::from_le_bytes([
            *quote.get(4)?,
            *quote.get(5)?,
            *quote.get(6)?,
            *quote.get(7)?,
        ]);
        if version != 4 || tee_type != 0x81 {
            return None;
        }
        let end = QUOTE_REPORT_DATA_OFFSET + QUOTE_REPORT_DATA_LEN;
        Some(hex::encode(quote.get(QUOTE_REPORT_DATA_OFFSET..end)?))
    };
    Some(read().ok_or(AttestedKeyError::Malformed))
}

/// The ed25519 signing key one attestation object binds to `expected_nonce`.
///
/// `attestation` is a single attestation object -- `gateway_attestation`, or
/// one entry of `model_attestations`. Both are bound the same way: the TDX
/// quote's `report_data` is `signing_address || request_nonce`, so a key read
/// out of an attestation whose `report_data` carries the nonce *we* chose is
/// one attested for us, now, rather than one copied out of an older report.
///
/// # Where `report_data` actually comes from
///
/// **Out of the `intel_quote`, at the fixed v4 TDX offset.** A
/// `model_attestations` entry carries **no `report_data` field** -- verified
/// against live captures, which is the only way this was going to be found
/// out. Only `gateway_attestation` has one, as a convenience echo of what its
/// own quote already says.
///
/// So the quote is the source, and the JSON field, where present, is checked
/// **against** it: a disagreement is [`AttestedKeyError::ReportDataMismatch`],
/// because a provider echo that does not match the structure it claims to
/// summarise is not something to pick a winner between. Where there is no
/// `intel_quote` the field is used alone; where there is neither, the
/// attestation is [`AttestedKeyError::Malformed`].
///
/// An `intel_quote` that is present but unreadable at that offset -- not hex,
/// too short, or not a v4 TDX quote -- is **refused**, not skipped in favour
/// of the field. Ignoring a structure we cannot read while trusting the
/// provider's summary of it is the fail-open direction.
///
/// **This does not verify the quote.** It reads the report's self-description
/// and checks its internal consistency. Until quote verification runs, a key
/// from here is a claim by the provider, not a proof.
///
/// The order of the checks is load-bearing. The algorithm is refused first,
/// then the key's shape -- a short or empty `signing_address` would
/// `strip_prefix` trivially against any `report_data` and make the binding
/// check vacuous -- and only then is `report_data` split.
///
/// # Errors
///
/// [`AttestedKeyError`] for each of those conditions; see its variants.
pub fn attested_ed25519_key(
    attestation: &serde_json::Value,
    expected_nonce: &str,
) -> Result<String, AttestedKeyError> {
    let field = |name: &str| {
        attestation
            .get(name)
            .and_then(serde_json::Value::as_str)
            .ok_or(AttestedKeyError::Malformed)
    };

    if ReceiptAlgo::from_wire(field("signing_algo")?) != Some(ReceiptAlgo::Ed25519) {
        return Err(AttestedKeyError::NotEd25519);
    }
    let key =
        normalize_ed25519_key(field("signing_address")?).ok_or(AttestedKeyError::KeyMalformed)?;
    // `request_nonce` is the provider's own label for the value; required for
    // shape, but the binding trusted here is `report_data` itself, checked
    // below against the key it names and the nonce we asked for.
    let _ = field("request_nonce")?;

    let from_quote = quote_report_data(attestation).transpose()?;
    let from_field = attestation
        .get("report_data")
        .and_then(serde_json::Value::as_str)
        .map(str::to_ascii_lowercase);
    let report_data = match (from_quote, from_field) {
        (Some(quote), Some(echo)) if quote != echo => {
            return Err(AttestedKeyError::ReportDataMismatch);
        }
        (Some(quote), _) => quote,
        (None, Some(echo)) => echo,
        (None, None) => return Err(AttestedKeyError::Malformed),
    };

    let attested_nonce = report_data
        .strip_prefix(key.as_str())
        .ok_or(AttestedKeyError::ReportDataMismatch)?;
    if attested_nonce != expected_nonce.to_ascii_lowercase() {
        return Err(AttestedKeyError::NonceMismatch);
    }
    Ok(key)
}

/// The ed25519 signing keys a report attests **for one model**.
///
/// This is the set a hosted-model receipt's signer must be checked against,
/// and it is not the gateway key. NEAR AI signs each hosted model's receipts
/// with a **per-model** `provider_tee` key, and that key appears only in the
/// report's model-attestation container, and only when the report was
/// requested with `signing_algo=ed25519` -- without that query parameter the
/// endpoint answers with the ECDSA model attestations instead, which is how it
/// came to be missed. See [`model_attestation_entries`] for the container
/// name, which the provider has changed once. Checking a hosted-model receipt
/// against
/// `gateway_attestation.signing_address` refuses every real one, because the
/// gateway key signs no receipts.
///
/// Every returned key carries the full binding discipline of
/// [`attested_ed25519_key`]: `report_data == signing_address || nonce` for the
/// nonce this caller sent, read out of the entry's **own `intel_quote`**,
/// since a model entry carries no `report_data` field. Entries for other
/// models are skipped; an entry for
/// *this* model that is malformed, not ed25519, or bound to another nonce is
/// **refused outright** rather than skipped, so a report that mixes a sound
/// entry with a forged one cannot be answered with the sound half.
///
/// More than one key can come back: a model served by several enclaves has an
/// attestation each, and a receipt matching any of them was signed inside an
/// attested one.
///
/// # Errors
///
/// [`AttestedKeyError::Malformed`] when the report is not JSON or carries
/// neither model-attestation container -- which is what a gateway-only report
/// looks like, and it must never yield the gateway key;
/// [`AttestedKeyError::ModelNotAttested`] when no entry names this model; and
/// the binding errors of [`attested_ed25519_key`] when an entry that does name
/// it fails to bind.
/// The container names a report may carry its per-model attestation entries
/// under, newest first.
///
/// Two names for one thing, because the provider renamed it. Live on
/// 2026-09-08, `qwen3-8-27b.completions.near.ai` serves the entries under
/// **`all_attestations`** and carries no `model_attestations` at all (issue
/// #802); earlier captures, and the fixtures taken from them, use
/// `model_attestations`.
///
/// Both are read rather than one replacing the other. The observation window
/// for the rename is one provider deployment on one day, a rename that went
/// one way can go back, and a fallback costs nothing.
pub const MODEL_ATTESTATION_CONTAINERS: [&str; 2] = ["all_attestations", "model_attestations"];

/// The model-attestation entries of a parsed report, under whichever container
/// name it uses.
///
/// `None` when the report carries neither -- which is exactly what a
/// **gateway-only** report looks like, and the reason this returns `None`
/// rather than an empty slice: a gateway report attests no model, and
/// answering "no entries" would let a caller fall through to a
/// `ModelNotAttested` that reads like "this model is not served here" instead
/// of "you are looking at the wrong report".
///
/// The first container present wins; they are not merged. A report carrying
/// both is not something seen in the wild, and merging would let a stale
/// container contribute keys to a fresh answer.
pub fn model_attestation_entries(document: &serde_json::Value) -> Option<&Vec<serde_json::Value>> {
    MODEL_ATTESTATION_CONTAINERS
        .iter()
        .find_map(|name| document.get(*name).and_then(serde_json::Value::as_array))
}

pub fn model_ed25519_keys(
    report_json: &str,
    expected_nonce: &str,
    model: &str,
) -> Result<Vec<String>, AttestedKeyError> {
    let document: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| AttestedKeyError::Malformed)?;
    let entries = model_attestation_entries(&document).ok_or(AttestedKeyError::Malformed)?;

    let mut keys = Vec::new();
    for entry in entries {
        // An absent `model_name` is not a match, and one naming another model
        // is skipped. Exact comparison: a model identifier is a routing key,
        // and `Qwen3.8-27B` and `Qwen3.6-35B-A3B-FP8` are different
        // deployments with different signing keys.
        if entry.get("model_name").and_then(serde_json::Value::as_str) != Some(model) {
            continue;
        }
        keys.push(attested_ed25519_key(entry, expected_nonce)?);
    }
    if keys.is_empty() {
        return Err(AttestedKeyError::ModelNotAttested);
    }
    Ok(keys)
}

/// The ed25519 signing key a report attests **for the gateway itself**.
///
/// This is the key that signs a `gateway`-kind receipt -- the form the
/// Responses API (`/v1/responses`) returns, and therefore the only form a
/// Codex-driven session ever produces. It signs no `provider_tee` receipt,
/// exactly as a model key signs no `gateway` one; see
/// [`ReceiptSignatureKind`].
///
/// Same binding discipline as [`model_ed25519_keys`], applied to the
/// `gateway_attestation` object: `report_data == signing_address || nonce` for
/// the nonce this caller sent. Unlike a model entry, the gateway attestation
/// carries a `report_data` field as well as an `intel_quote`, and
/// [`attested_ed25519_key`] refuses a disagreement between them rather than
/// picking a winner.
///
/// One key, not a set: a report carries exactly one `gateway_attestation`. It
/// is returned as a `Vec` by [`attested_keys_for_receipt`] only so that both
/// kinds resolve to the same shape.
///
/// # Errors
///
/// [`AttestedKeyError::Malformed`] when the report is not JSON or carries no
/// `gateway_attestation`, and the binding errors of [`attested_ed25519_key`]
/// when the object is present but does not bind.
pub fn gateway_ed25519_key(
    report_json: &str,
    expected_nonce: &str,
) -> Result<String, AttestedKeyError> {
    let document: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| AttestedKeyError::Malformed)?;
    let gateway = document
        .get("gateway_attestation")
        .ok_or(AttestedKeyError::Malformed)?;
    attested_ed25519_key(gateway, expected_nonce)
}

/// The attested key set a receipt's signer must be checked against, chosen by
/// the receipt's own [`ReceiptSignatureKind`].
///
/// This is the router the two kinds need, and the reason it exists is that
/// **the same hosted model** signs with a different key depending on the
/// protocol the call used. Reading only `model_attestations` -- which is what
/// shipped -- refuses every legitimate Responses-API receipt, and Codex speaks
/// nothing else.
///
/// One report answers both: a single `GET /attestation/report?model=..&
/// signing_algo=ed25519&nonce=..` returns `gateway_attestation` *and*
/// `model_attestations`, so routing costs no extra fetch.
///
/// # Each kind has exactly one source
///
/// - [`ReceiptSignatureKind::Gateway`] resolves to [`gateway_ed25519_key`]
///   and to nothing else. `model` is not consulted.
/// - [`ReceiptSignatureKind::ProviderTee`] resolves to [`model_ed25519_keys`]
///   for the receipt's own model, and to nothing else.
/// - [`ReceiptSignatureKind::Unrecognised`] resolves to nothing and is
///   refused.
///
/// There is deliberately no arm that tries both. A `gateway` receipt that
/// verified against a model key, or the reverse, would mean a key attested for
/// one role could vouch for the other -- which is the property the two
/// separate attestations exist to deny.
///
/// # Errors
///
/// [`AttestedKeyError::SignatureKindUnrecognised`] for a kind that names no
/// source, including the receipt that carried no kind at all;
/// [`AttestedKeyError::ModelNotAttested`] for a `provider_tee` receipt that
/// binds no model, since there is then no entry to look up; and otherwise
/// whatever the chosen reader returns.
pub fn attested_keys_for_receipt(
    kind: ReceiptSignatureKind,
    report_json: &str,
    expected_nonce: &str,
    model: Option<&str>,
) -> Result<Vec<String>, AttestedKeyError> {
    match kind {
        ReceiptSignatureKind::Gateway => {
            Ok(vec![gateway_ed25519_key(report_json, expected_nonce)?])
        }
        ReceiptSignatureKind::ProviderTee => {
            // A `provider_tee` receipt with no model names no entry. Refused
            // rather than answered with some other model's keys, which is the
            // only other thing this could do.
            let model = model.ok_or(AttestedKeyError::ModelNotAttested)?;
            model_ed25519_keys(report_json, expected_nonce, model)
        }
        ReceiptSignatureKind::Unrecognised => Err(AttestedKeyError::SignatureKindUnrecognised),
    }
}

/// Whether a verified receipt's signer is one of a model's attested keys.
///
/// The set form of [`signer_is_attested`], with the same guard against an
/// empty key: an empty set matches nothing, and neither does an empty entry
/// inside one.
#[must_use]
pub fn signer_is_attested_for_model(receipt_signer: &str, attested_keys: &[String]) -> bool {
    attested_keys
        .iter()
        .any(|key| signer_is_attested(receipt_signer, key))
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
    use crate::eip191::{address_of, eip191_digest, strip_0x};
    use k256::ecdsa::SigningKey;
    use sha3::Keccak256;

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
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::Unrecognised,
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
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::Unrecognised,
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
                signing_algo: ReceiptAlgo::Ecdsa,
                signature_kind: ReceiptSignatureKind::Unrecognised,
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

    const LIVE_ED25519_TEXT: &str = "81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e";
    const LIVE_ED25519_SIGNATURE: &str = "838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c";
    const LIVE_ED25519_KEY: &str =
        "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";

    fn live_ed25519() -> ReceiptPayload {
        ReceiptPayload {
            text: LIVE_ED25519_TEXT.to_string(),
            signature: LIVE_ED25519_SIGNATURE.to_string(),
            signing_address: LIVE_ED25519_KEY.to_string(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: ReceiptSignatureKind::Gateway,
        }
    }

    /// A receipt NEAR AI actually issued, verified as plain Ed25519 over the
    /// raw text. The signature came from a key this project never held.
    ///
    /// This is also the guard against applying the EIP-191 prefix on the
    /// ed25519 path: a verifier that prefixes the text fails this test,
    /// because the live signature is over the raw bytes. A separate
    /// "pre-prefixed input is refused" test was tried and found vacuous -- a
    /// prefixing verifier double-prefixes it and fails too -- so this one
    /// test is deliberately the only guard, and it is the one that cannot be
    /// fooled.
    ///
    /// The digest checks are exercised separately, because this test does not
    /// hold NEAR AI's real bodies and cannot forge ones with the same digests.
    #[test]
    fn a_live_ed25519_receipt_signature_verifies_over_the_raw_text() {
        let payload = live_ed25519();
        verify_ed25519_signature(&payload).expect("NEAR AI's real signature verifies");
    }

    /// A validly signed ed25519 receipt over different bytes is refused for
    /// its digest, not its signature -- the signature verifies fine here, so
    /// this pins the digest check alone, not any ordering between the two.
    /// `a_bad_ed25519_signature_is_refused_before_the_digests_are_checked`,
    /// below, is what pins the ordering.
    #[test]
    fn an_ed25519_receipt_over_other_bytes_is_refused() {
        let payload = live_ed25519();
        let err = verify_receipt(
            &payload,
            b"not the request",
            b"not the response",
            "Qwen/Qwen3.6-35B-A3B-FP8",
        )
        .expect_err("the bodies do not hash to the bound values");
        assert_eq!(err, ReceiptError::RequestHashMismatch);
    }

    /// One flipped bit in the signature is refused as invalid, not as
    /// malformed: it is still 64 well-formed bytes.
    #[test]
    fn a_tampered_ed25519_signature_is_invalid_not_malformed() {
        let mut tampered = live_ed25519();
        let mut bytes = hex::decode(LIVE_ED25519_SIGNATURE).unwrap();
        bytes[0] ^= 0x01;
        tampered.signature = hex::encode(bytes);
        assert_eq!(
            verify_ed25519_signature(&tampered),
            Err(ReceiptError::Ed25519SignatureInvalid)
        );
    }

    /// A key that is not 32 bytes, or a signature that is not 64, is refused
    /// by name before any cryptography runs.
    #[test]
    fn malformed_ed25519_material_is_refused_by_name() {
        let mut short_key = live_ed25519();
        short_key.signing_address = "cb6f".to_string();
        assert_eq!(
            verify_ed25519_signature(&short_key),
            Err(ReceiptError::Ed25519KeyMalformed)
        );

        let mut short_sig = live_ed25519();
        short_sig.signature = "8387".to_string();
        assert_eq!(
            verify_ed25519_signature(&short_sig),
            Err(ReceiptError::Ed25519SignatureMalformed)
        );
    }

    /// The wire spelling round-trips, and anything else is `None` rather than
    /// a guess.
    #[test]
    fn the_algo_wire_spelling_round_trips_and_rejects_the_unknown() {
        assert_eq!(ReceiptAlgo::from_wire("ecdsa"), Some(ReceiptAlgo::Ecdsa));
        assert_eq!(
            ReceiptAlgo::from_wire("ed25519"),
            Some(ReceiptAlgo::Ed25519)
        );
        assert_eq!(ReceiptAlgo::from_wire("ECDSA"), None, "case is not folded");
        assert_eq!(ReceiptAlgo::from_wire("rsa"), None);
        assert_eq!(ReceiptAlgo::Ecdsa.as_wire(), "ecdsa");
        assert_eq!(ReceiptAlgo::Ed25519.as_wire(), "ed25519");
    }

    /// The ECDSA path is unchanged: every existing test in this module still
    /// passes with `signing_algo: ReceiptAlgo::Ecdsa`, and an ECDSA payload
    /// handed to the ed25519 verifier is refused as a malformed key -- a
    /// 20-byte address is not a 32-byte public key.
    #[test]
    fn an_ecdsa_payload_is_not_an_ed25519_one() {
        let mut wrong = live_ed25519();
        wrong.signing_address = "0x614bc66ff0407dbb70b9c7ca1f5e983e4a02c921".to_string();
        assert_eq!(
            verify_ed25519_signature(&wrong),
            Err(ReceiptError::Ed25519KeyMalformed)
        );
    }

    // ---- The ed25519 success path through `verify_receipt` -------------
    //
    // Every test above either calls `verify_ed25519_signature` directly or
    // takes the `RequestHashMismatch` early return in `verify_receipt`, so
    // neither `verify_receipt`'s `to_ascii_lowercase()` on the ed25519
    // verdict nor its `signing_algo: payload.signing_algo` line is ever
    // executed. A mutation dropping the lowercasing, or hardcoding `Ecdsa`
    // into the verdict, would pass every test above. The fixture below
    // drives `verify_receipt` all the way to `Ok` to close that gap.
    //
    // The key is generated once, offline, and pinned as a constant --
    // never generated at test time -- for the same reason every other fixed
    // key in this module is a constant: a random key makes a failure
    // unreproducible. Generated with a throwaway
    // `Ed25519KeyPair::generate_pkcs8(&SystemRandom::new())` in a scratch
    // test that printed the hex and was deleted; `generate_pkcs8` produces
    // a PKCS#8 v2 document, which is what `ring::signature::Ed25519KeyPair
    // ::from_pkcs8` here requires.

    /// A PKCS#8 v2 Ed25519 private key, generated once and pinned. This
    /// project never held the real NEAR AI signing key; this is a key of our
    /// own, used only to reach the success path with a receipt this test can
    /// construct end to end.
    const TEST_ED25519_PKCS8_HEX: &str = "3051020101300506032b6570042204202764e45a50c3d8868fc19eb9399ed8e502345ee694068543a055e4043c80061681210080b7c045fef35b623c5fad76f4b0aa2cd4c29f10a7863cdbf67d802e821b783d";

    /// A self-signed ed25519 receipt: real signature, over real bodies,
    /// reaching `Ok` through `verify_receipt`.
    ///
    /// The public key is embedded in `signing_address` UPPERCASE. `hex::encode`
    /// already emits lowercase, so a lowercase key would make
    /// `to_ascii_lowercase()` in `verify_receipt` a silent no-op and a
    /// mutation that dropped it would still pass. Uppercase forces the
    /// lowercasing to do visible work.
    fn self_signed_ed25519_receipt(request_body: &[u8], response_body: &[u8]) -> ReceiptPayload {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let key_pair =
            Ed25519KeyPair::from_pkcs8(&hex::decode(TEST_ED25519_PKCS8_HEX).unwrap()).unwrap();
        let text = format!(
            "{}:{}",
            hex::encode(Sha256::digest(request_body)),
            hex::encode(Sha256::digest(response_body))
        );
        // Load-bearing: signed over the raw text, never the EIP-191-prefixed
        // form. Prefixing here would be exactly how the deleted
        // `the_ed25519_path_does_not_apply_the_eip191_prefix` test became
        // vacuous -- a future maintainer "fixing" a red test by signing the
        // prefixed form instead of fixing the verifier. This fixture and the
        // live-receipt test above are the two guards against that.
        let signature = key_pair.sign(text.as_bytes());
        ReceiptPayload {
            text,
            signature: hex::encode(signature.as_ref()),
            signing_address: hex::encode(key_pair.public_key().as_ref()).to_ascii_uppercase(),
            signing_algo: ReceiptAlgo::Ed25519,
            signature_kind: ReceiptSignatureKind::Gateway,
        }
    }

    /// The ed25519 success path through `verify_receipt`, not just
    /// `verify_ed25519_signature`: the verdict's address is lowercased and
    /// its `signing_algo` is the one the payload carried, not a hardcoded
    /// `Ecdsa`. Bodies are chosen so a re-serialiser would change them --
    /// non-alphabetical keys, odd whitespace, a non-ASCII character -- the
    /// same fixture discipline the rest of this module uses.
    #[test]
    fn a_self_signed_ed25519_receipt_verifies_to_a_lowercase_verdict() {
        let request = r#"{"messages":[{"content":"café  ","role":"user"}],"model":"m"}"#.as_bytes();
        let response =
            r#"{"id":"c1","choices":[{"message":{"role":"assistant",  "content":"oké"}}]}"#
                .as_bytes();
        let payload = self_signed_ed25519_receipt(request, response);
        // Measured, not assumed: the fixture really does carry an uppercase
        // key, so the assertion below is exercising the lowercasing rather
        // than comparing a lowercase value against itself.
        assert!(payload.signing_address.contains(char::is_uppercase));

        let verdict = verify_receipt(&payload, request, response, "unused")
            .expect("a validly self-signed receipt over its own bodies verifies");
        assert_eq!(
            verdict.signing_address,
            payload.signing_address.to_lowercase()
        );
        assert_eq!(verdict.signing_algo, ReceiptAlgo::Ed25519);
    }

    /// A corrupted signature is refused as an invalid signature, not as a
    /// digest mismatch, even when the bodies handed in *also* don't match --
    /// the only way that specific error comes out is if the signature check
    /// runs before the digest checks. This is the ordering guard that
    /// `an_ed25519_receipt_over_other_bytes_is_refused` does not provide,
    /// because that test's signature is valid and both orderings would
    /// report the same digest mismatch.
    #[test]
    fn a_bad_ed25519_signature_is_refused_before_the_digests_are_checked() {
        let request = b"the real request";
        let response = b"the real response";
        let mut payload = self_signed_ed25519_receipt(request, response);
        let mut sig = hex::decode(&payload.signature).unwrap();
        sig[0] ^= 0x01;
        payload.signature = hex::encode(sig);

        let err = verify_receipt(&payload, b"not the request", b"not the response", "unused")
            .expect_err("neither the signature nor the bodies are right");
        assert_eq!(err, ReceiptError::Ed25519SignatureInvalid);
    }

    // ---------------------------------------------------------------------
    // Model attestations.
    //
    // Every key below is a value NEAR AI actually returned on 2026-09-06, not
    // one authored beside this code. That distinction is the whole point: the
    // defect this covers was shipped alongside tests proving that a report
    // parses and that a signature verifies under a key the *same fixture*
    // supplied, which is true of the wrong key as well as the right one.
    // ---------------------------------------------------------------------

    /// The gateway's ed25519 signing key -- the one the shipped code checked
    /// receipts against. It signs no receipt.
    const LIVE_GATEWAY_KEY: &str =
        "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";
    const MODEL_A: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
    /// The `provider_tee` key that actually signs `MODEL_A`'s receipts.
    const MODEL_A_KEY: &str = "aba45f0b8f90869baab26db02e8b01354bb8f8730769c60650cb7a635da602d4";
    const MODEL_B: &str = "Qwen/Qwen3.8-27B";
    /// And `MODEL_B`'s -- a different key, which is why one pin cannot serve.
    const MODEL_B_KEY: &str = "73cf225ab4f09154ad8b299d4ac89425c7f25468a42ba9a87d09fcd4e87b8bf5";

    const REPORT_NONCE: &str = "482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d";

    /// A **synthetic** `model_attestations` entry, carrying an explicit
    /// `report_data` field and no `intel_quote`.
    ///
    /// This is deliberately *not* the live shape: a real entry has no
    /// `report_data` field and carries the binding inside its quote. These
    /// exist for the negative and edge cases a capture cannot express -- a
    /// mismatched nonce, an ECDSA entry, a malformed key -- where being able
    /// to set `report_data` directly is the whole point. The live shape is
    /// covered from real bytes further down, and
    /// `a_live_model_attestation_carries_no_report_data_field` pins the
    /// difference so this fixture cannot quietly become the only thing tested.
    fn model_entry(model: &str, key: &str, algo: &str, nonce_in_report_data: &str) -> String {
        format!(
            r#"{{"model_name":"{model}","signing_address":"{key}","signing_algo":"{algo}","request_nonce":"{REPORT_NONCE}","report_data":"{key}{nonce_in_report_data}"}}"#
        )
    }

    fn report_with(entries: &[String]) -> String {
        format!(
            r#"{{"gateway_attestation":{{"signing_address":"{LIVE_GATEWAY_KEY}","signing_algo":"ed25519","request_nonce":"{REPORT_NONCE}","report_data":"{LIVE_GATEWAY_KEY}{REPORT_NONCE}"}},"model_attestations":[{}]}}"#,
            entries.join(",")
        )
    }

    fn live_report() -> String {
        report_with(&[
            model_entry(MODEL_A, MODEL_A_KEY, "ed25519", REPORT_NONCE),
            model_entry(MODEL_B, MODEL_B_KEY, "ed25519", REPORT_NONCE),
        ])
    }

    /// The defect, stated as an assertion: a receipt signer for a hosted model
    /// is the model's key and is **not** the gateway key. A verifier holding
    /// the gateway key refuses it.
    #[test]
    fn a_hosted_model_receipt_signer_is_not_the_gateway_key() {
        let keys = model_ed25519_keys(&live_report(), REPORT_NONCE, MODEL_A).expect("attested");
        assert_eq!(keys, vec![MODEL_A_KEY.to_string()]);
        assert!(
            signer_is_attested_for_model(MODEL_A_KEY, &keys),
            "the real signer must verify against the real model attestation"
        );
        assert!(
            !signer_is_attested(MODEL_A_KEY, LIVE_GATEWAY_KEY),
            "the gateway key is what the shipped check compared against, and \
             it refuses every real hosted-model receipt"
        );
    }

    /// Per-model selection. One report, two models, two different keys, and
    /// neither model's key is accepted for the other. A single pinned key
    /// cannot express this, which is why the pin had to become a set.
    #[test]
    fn each_model_gets_its_own_signer_set() {
        let report = live_report();
        let a = model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).expect("model a");
        let b = model_ed25519_keys(&report, REPORT_NONCE, MODEL_B).expect("model b");
        assert_eq!(a, vec![MODEL_A_KEY.to_string()]);
        assert_eq!(b, vec![MODEL_B_KEY.to_string()]);
        assert!(!signer_is_attested_for_model(MODEL_A_KEY, &b));
        assert!(!signer_is_attested_for_model(MODEL_B_KEY, &a));
    }

    /// A model the report says nothing about is not attested. Refused rather
    /// than answered with some other model's key, and rather than answered
    /// with an empty set that would match nothing but read as a success.
    #[test]
    fn a_model_the_report_does_not_name_is_refused() {
        assert_eq!(
            model_ed25519_keys(&live_report(), REPORT_NONCE, "Qwen/Qwen3.9-Nonexistent")
                .unwrap_err(),
            AttestedKeyError::ModelNotAttested
        );
    }

    /// An entry naming this model whose `report_data` carries a different
    /// nonce is stale or replayed; the whole lookup fails rather than the
    /// entry being quietly dropped.
    #[test]
    fn a_model_attestation_for_another_nonce_is_refused() {
        let report = report_with(&[model_entry(
            MODEL_A,
            MODEL_A_KEY,
            "ed25519",
            &"0".repeat(64),
        )]);
        assert_eq!(
            model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).unwrap_err(),
            AttestedKeyError::NonceMismatch
        );
    }

    /// `report_data` that does not begin with the key the entry lists is not
    /// a binding to that key.
    #[test]
    fn a_model_attestation_whose_report_data_names_another_key_is_refused() {
        let report = format!(
            r#"{{"model_attestations":[{{"model_name":"{MODEL_A}","signing_address":"{MODEL_A_KEY}","signing_algo":"ed25519","request_nonce":"{REPORT_NONCE}","report_data":"{}{REPORT_NONCE}"}}]}}"#,
            MODEL_B_KEY
        );
        assert_eq!(
            model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).unwrap_err(),
            AttestedKeyError::ReportDataMismatch
        );
    }

    /// Without `signing_algo=ed25519` on the request the endpoint answers with
    /// ECDSA model attestations. Those bind no receipt-signing key this crate
    /// can use, and are refused rather than read as if they were ed25519.
    #[test]
    fn an_ecdsa_model_attestation_is_refused() {
        let report = report_with(&[model_entry(MODEL_A, MODEL_A_KEY, "ecdsa", REPORT_NONCE)]);
        assert_eq!(
            model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).unwrap_err(),
            AttestedKeyError::NotEd25519
        );
    }

    /// A short or empty `signing_address` would `strip_prefix` against any
    /// `report_data` and make the binding check vacuous, so the shape is
    /// checked before the split.
    #[test]
    fn a_model_attestation_key_that_is_not_32_bytes_is_refused() {
        for bad in ["", "ab", &"z".repeat(64)] {
            let report = report_with(&[model_entry(MODEL_A, bad, "ed25519", REPORT_NONCE)]);
            assert_eq!(
                model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).unwrap_err(),
                AttestedKeyError::KeyMalformed,
                "key {bad:?} should be refused as malformed"
            );
        }
    }

    /// A report with no `model_attestations` array at all -- what the endpoint
    /// returns for a brokered model -- is malformed for this purpose, not an
    /// empty success.
    #[test]
    fn a_report_without_model_attestations_is_malformed() {
        assert_eq!(
            model_ed25519_keys(r#"{"gateway_attestation":{}}"#, REPORT_NONCE, MODEL_A).unwrap_err(),
            AttestedKeyError::Malformed
        );
        assert_eq!(
            model_ed25519_keys("not json", REPORT_NONCE, MODEL_A).unwrap_err(),
            AttestedKeyError::Malformed
        );
    }

    /// An empty attested set matches nothing. This is the way the set form
    /// would fail open.
    #[test]
    fn an_empty_attested_set_matches_nothing() {
        assert!(!signer_is_attested_for_model(MODEL_A_KEY, &[]));
        assert!(!signer_is_attested_for_model(MODEL_A_KEY, &[String::new()]));
        assert!(
            signer_is_attested_for_model(
                &MODEL_A_KEY.to_ascii_uppercase(),
                &[MODEL_A_KEY.to_string()]
            ),
            "case is a spelling of one key, not a different key"
        );
    }

    /// Several enclaves can serve one model; a receipt from any attested one
    /// is accepted, and one from an unattested key still is not.
    #[test]
    fn a_model_served_by_two_enclaves_yields_both_keys() {
        let report = report_with(&[
            model_entry(MODEL_A, MODEL_A_KEY, "ed25519", REPORT_NONCE),
            model_entry(MODEL_A, MODEL_B_KEY, "ed25519", REPORT_NONCE),
        ]);
        let keys = model_ed25519_keys(&report, REPORT_NONCE, MODEL_A).expect("both");
        assert_eq!(keys.len(), 2);
        assert!(signer_is_attested_for_model(MODEL_A_KEY, &keys));
        assert!(signer_is_attested_for_model(MODEL_B_KEY, &keys));
        assert!(!signer_is_attested_for_model(LIVE_GATEWAY_KEY, &keys));
    }

    // ---------------------------------------------------------------------
    // Live captures.
    //
    // Everything above is authored beside the code. Everything below is
    // bytes NEAR AI returned on 2026-09-06, and it is the half that matters:
    // the defect this file fixes shipped with tests proving a report parses
    // and a signature verifies under a key the same fixture supplied, which
    // is equally true of the wrong key.
    //
    // The reports are `GET /v1/attestation/report?model=..&
    // signing_algo=ed25519&nonce=..`, trimmed. What was removed:
    // `nvidia_payload`, `event_log`, `info`, `compose_manager_attestation`,
    // `ohttp_attestation`, `ohttp_key_config` and `signing_public_key` --
    // hundreds of kilobytes of GPU evidence and certificate chains that no
    // code here reads. What was kept is byte-exact, `intel_quote` included:
    // the binding this module checks lives *inside* that quote, so trimming
    // it would have destroyed the thing under test.
    //
    // The receipts are `GET /v1/signature/{chat_id}?model=..&
    // signing_algo=ed25519`, verbatim.
    // ---------------------------------------------------------------------

    const LIVE_REPORT_A: &str =
        include_str!("../tests/fixtures/near_ai_model_attestation_report_ed25519.json");
    const LIVE_REPORT_B: &str = include_str!(
        "../tests/fixtures/near_ai_model_attestation_report_ed25519_second_model.json"
    );
    const LIVE_RECEIPT_A: &str =
        include_str!("../tests/fixtures/near_ai_receipt_provider_tee_ed25519.json");
    const LIVE_RECEIPT_B: &str =
        include_str!("../tests/fixtures/near_ai_receipt_provider_tee_ed25519_second_model.json");

    /// The nonce that fetch actually sent, read back out of the capture. Each
    /// report was fetched with its own.
    fn live_nonce(report: &str) -> String {
        serde_json::from_str::<serde_json::Value>(report).expect("fixture is JSON")
            ["gateway_attestation"]["request_nonce"]
            .as_str()
            .expect("a captured nonce")
            .to_string()
    }

    fn live_receipt(json: &str) -> ReceiptPayload {
        let v: serde_json::Value = serde_json::from_str(json).expect("fixture is JSON");
        let s = |k: &str| v[k].as_str().expect("a captured field").to_string();
        ReceiptPayload {
            text: s("text"),
            signature: s("signature"),
            signing_address: s("signing_address"),
            signing_algo: ReceiptAlgo::from_wire(&s("signing_algo")).expect("a known algo"),
            // Off the fixture, never asserted to a constant here: which kind a
            // capture carries is the thing under test, and a helper that
            // hard-coded `provider_tee` would make the gateway fixture below
            // impossible to express.
            signature_kind: ReceiptSignatureKind::from_wire(&s("signature_kind")),
        }
    }

    /// The model a receipt binds: the leading part of its signed `text`.
    fn bound_model(receipt: &ReceiptPayload) -> &str {
        receipt.text.split(':').next().expect("a leading part")
    }

    /// The discovery that the synthetic fixtures hid, pinned so it cannot be
    /// un-learned: a real `model_attestations` entry has **no `report_data`
    /// field**. An implementation that requires one refuses every real report,
    /// which is how the first version of this module was written.
    #[test]
    fn a_live_model_attestation_carries_no_report_data_field() {
        for report in [LIVE_REPORT_A, LIVE_REPORT_B] {
            let v: serde_json::Value = serde_json::from_str(report).expect("fixture is JSON");
            let entry = &v["model_attestations"][0];
            assert!(
                entry.get("report_data").is_none(),
                "a live model attestation must still carry no report_data field; \
                 if this fails the provider changed shape and the quote-offset \
                 read below should be revisited"
            );
            assert!(
                entry.get("intel_quote").is_some(),
                "the binding has to come from somewhere, and the quote is it"
            );
            // The gateway, by contrast, does echo one.
            assert!(v["gateway_attestation"].get("report_data").is_some());
        }
    }

    /// 1. The real receipt's signer is attested by the real report for the
    ///    same model, over the real nonce, with the binding read out of the
    ///    real quote -- and the real signature verifies over the real text.
    #[test]
    fn a_live_receipts_signer_is_attested_by_the_live_report_for_its_model() {
        for (report, receipt) in [
            (LIVE_REPORT_A, LIVE_RECEIPT_A),
            (LIVE_REPORT_B, LIVE_RECEIPT_B),
        ] {
            let receipt = live_receipt(receipt);
            let model = bound_model(&receipt);

            // The signature is real: it verifies over its own text under the
            // key the receipt names. Without this the attestation assertion
            // below would only be comparing two strings.
            verify_ed25519_signature(&receipt)
                .expect("the captured receipt signature verifies over its captured text");
            assert_eq!(receipt.signing_algo, ReceiptAlgo::Ed25519);

            let keys = model_ed25519_keys(report, &live_nonce(report), model)
                .expect("the live report attests this model");
            assert!(
                signer_is_attested_for_model(&receipt.signing_address, &keys),
                "the signer of a real {model} receipt must be attested by the \
                 real report for {model}"
            );
        }
    }

    /// 2. Cross-model rejection, entirely from real bytes: the second model's
    ///    real receipt signer is not accepted against the first model's real
    ///    report, and vice versa. This is what one provider-wide key cannot
    ///    express.
    #[test]
    fn a_live_receipt_is_not_attested_by_another_models_live_report() {
        let a = live_receipt(LIVE_RECEIPT_A);
        let b = live_receipt(LIVE_RECEIPT_B);
        assert_ne!(a.signing_address, b.signing_address);

        let keys_a = model_ed25519_keys(LIVE_REPORT_A, &live_nonce(LIVE_REPORT_A), bound_model(&a))
            .expect("model a is attested");
        let keys_b = model_ed25519_keys(LIVE_REPORT_B, &live_nonce(LIVE_REPORT_B), bound_model(&b))
            .expect("model b is attested");

        assert!(!signer_is_attested_for_model(&b.signing_address, &keys_a));
        assert!(!signer_is_attested_for_model(&a.signing_address, &keys_b));

        // And the first report attests nothing at all for the second model.
        assert_eq!(
            model_ed25519_keys(LIVE_REPORT_A, &live_nonce(LIVE_REPORT_A), bound_model(&b))
                .unwrap_err(),
            AttestedKeyError::ModelNotAttested
        );
    }

    /// 3. The defect itself, entirely from real bytes. The gateway key in
    ///    these same reports reads back correctly through
    ///    `attested_ed25519_key` -- it is a real attested key, and that
    ///    function is not broken -- and it is **not** the signer of either
    ///    real receipt, nor a member of either model's attested set. A
    ///    verifier holding it refuses every real receipt.
    #[test]
    fn the_live_gateway_key_is_attested_yet_signs_neither_live_receipt() {
        for (report, receipt) in [
            (LIVE_REPORT_A, LIVE_RECEIPT_A),
            (LIVE_REPORT_B, LIVE_RECEIPT_B),
        ] {
            let nonce = live_nonce(report);
            let document: serde_json::Value = serde_json::from_str(report).expect("fixture");
            let gateway = attested_ed25519_key(&document["gateway_attestation"], &nonce)
                .expect("the gateway attestation binds its own key over our nonce");

            let receipt = live_receipt(receipt);
            let keys = model_ed25519_keys(report, &nonce, bound_model(&receipt)).expect("attested");

            assert_ne!(
                gateway, receipt.signing_address,
                "the gateway key is not the receipt signer"
            );
            assert!(
                !signer_is_attested(&receipt.signing_address, &gateway),
                "comparing a real receipt signer against the gateway key -- what \
                 the shipped code did -- refuses a valid receipt"
            );
            assert!(
                !signer_is_attested_for_model(&gateway, &keys),
                "and the gateway key is not in the model's attested set either"
            );
        }
    }

    /// The gateway's own `report_data` echo agrees with its own quote. Where
    /// both are present they are cross-checked, and on real bytes they match
    /// -- so the cross-check is a live control rather than a hypothetical.
    #[test]
    fn the_live_gateway_echo_agrees_with_its_own_quote() {
        for report in [LIVE_REPORT_A, LIVE_REPORT_B] {
            let document: serde_json::Value = serde_json::from_str(report).expect("fixture");
            let gateway = &document["gateway_attestation"];
            let from_quote = quote_report_data(gateway)
                .expect("the gateway carries a quote")
                .expect("a v4 TDX quote");
            assert_eq!(
                from_quote,
                gateway["report_data"].as_str().expect("the echo"),
                "the gateway's report_data field is exactly what its quote says"
            );
        }
    }

    /// A live report read against a nonce that is not the one it was fetched
    /// for is refused. The nonce is the whole reason the fetch sends one, and
    /// this proves the real quote's bytes are what carry it.
    #[test]
    fn a_live_report_read_for_the_wrong_nonce_is_refused() {
        let receipt = live_receipt(LIVE_RECEIPT_A);
        assert_eq!(
            model_ed25519_keys(LIVE_REPORT_A, &"0".repeat(64), bound_model(&receipt)).unwrap_err(),
            AttestedKeyError::NonceMismatch
        );
        // Including the *other* live report's nonce, which is a real value
        // that was genuinely attested -- just not for this fetch.
        assert_eq!(
            model_ed25519_keys(
                LIVE_REPORT_A,
                &live_nonce(LIVE_REPORT_B),
                bound_model(&receipt)
            )
            .unwrap_err(),
            AttestedKeyError::NonceMismatch
        );
    }

    /// An `intel_quote` that is present but unreadable is refused rather than
    /// skipped in favour of the provider's `report_data` echo. Built by
    /// corrupting a *real* report, so the only thing that changed is the
    /// quote.
    #[test]
    fn a_present_but_unreadable_quote_is_refused_not_ignored() {
        let receipt = live_receipt(LIVE_RECEIPT_A);
        let nonce = live_nonce(LIVE_REPORT_A);
        let mut document: serde_json::Value = serde_json::from_str(LIVE_REPORT_A).expect("fixture");

        for broken in [
            // Not hex.
            "nonsense".to_string(),
            // Hex, but far too short to reach the offset.
            "0400020081000000".to_string(),
            // A v4 quote of the right length whose TEE type is not TDX, so
            // offset 568 would be some other field entirely.
            {
                let real = document["model_attestations"][0]["intel_quote"]
                    .as_str()
                    .unwrap();
                let mut bytes = hex::decode(real).unwrap();
                bytes[4] = 0x00;
                hex::encode(bytes)
            },
        ] {
            document["model_attestations"][0]["intel_quote"] = serde_json::json!(broken);
            assert_eq!(
                model_ed25519_keys(&document.to_string(), &nonce, bound_model(&receipt))
                    .unwrap_err(),
                AttestedKeyError::Malformed,
                "an unreadable quote must refuse, not fall back to an echo"
            );
        }
    }

    // ---- Both signature kinds -------------------------------------------
    //
    // The correction this section pins: NEAR AI issues two legitimate kinds
    // of receipt for the *same* hosted model, and the protocol decides which.
    // A Chat Completions call gets `provider_tee`, signed by the model key; a
    // Responses API call gets `gateway`, signed by the gateway key. Codex
    // speaks only the Responses API, so a verifier that reads
    // `model_attestations` alone refuses every Codex-driven receipt.
    //
    // Both fixtures below are live bytes. The gateway receipt is the capture
    // whose signer is the same `cb6fc58f..` gateway key these reports attest;
    // it is a real signature over its real text, which is why
    // `verify_ed25519_signature` is asserted on it rather than asserted
    // around.
    // ---------------------------------------------------------------------

    const LIVE_RECEIPT_GATEWAY: &str =
        include_str!("../tests/fixtures/near_ai_receipt_gateway_ed25519.json");

    /// The two fixtures really are the two kinds, with two different signers.
    /// Everything below is only meaningful if this holds.
    #[test]
    fn the_two_live_receipts_are_two_kinds_signed_by_two_keys() {
        let gateway = live_receipt(LIVE_RECEIPT_GATEWAY);
        let provider = live_receipt(LIVE_RECEIPT_A);
        assert_eq!(gateway.signature_kind, ReceiptSignatureKind::Gateway);
        assert_eq!(provider.signature_kind, ReceiptSignatureKind::ProviderTee);
        assert_ne!(gateway.signing_address, provider.signing_address);
        // A real signature over its own text, under the key it names.
        verify_ed25519_signature(&gateway).expect("the live gateway receipt verifies");
        verify_ed25519_signature(&provider).expect("the live provider_tee receipt verifies");
    }

    /// A `gateway` receipt routes to the gateway key and is attested by it.
    #[test]
    fn a_gateway_receipt_verifies_against_the_gateway_key() {
        let receipt = live_receipt(LIVE_RECEIPT_GATEWAY);
        let nonce = live_nonce(LIVE_REPORT_A);
        let keys = attested_keys_for_receipt(receipt.signature_kind, LIVE_REPORT_A, &nonce, None)
            .expect("the report attests the gateway");
        assert!(signer_is_attested_for_model(
            &receipt.signing_address,
            &keys
        ));
    }

    /// ...and is refused against the model keys. This is the half that must
    /// not be reachable: a gateway-signed receipt admitted under a model key
    /// would mean either attested key could vouch for the other's role.
    #[test]
    fn a_gateway_receipt_is_refused_against_the_model_keys() {
        let receipt = live_receipt(LIVE_RECEIPT_GATEWAY);
        let provider = live_receipt(LIVE_RECEIPT_A);
        let model = bound_model(&provider);
        let nonce = live_nonce(LIVE_REPORT_A);
        let model_keys =
            model_ed25519_keys(LIVE_REPORT_A, &nonce, model).expect("the model is attested");
        assert!(!signer_is_attested_for_model(
            &receipt.signing_address,
            &model_keys
        ));
        // And routing it as `provider_tee` -- the shipped behaviour -- lands
        // it in exactly that set, which is why the shipped code refused it.
        let mis_routed = attested_keys_for_receipt(
            ReceiptSignatureKind::ProviderTee,
            LIVE_REPORT_A,
            &nonce,
            Some(model),
        )
        .expect("the model is attested");
        assert!(!signer_is_attested_for_model(
            &receipt.signing_address,
            &mis_routed
        ));
    }

    /// A `provider_tee` receipt routes to its own model's keys and is
    /// attested by them.
    #[test]
    fn a_provider_tee_receipt_verifies_against_its_model_key() {
        let receipt = live_receipt(LIVE_RECEIPT_A);
        let nonce = live_nonce(LIVE_REPORT_A);
        let keys = attested_keys_for_receipt(
            receipt.signature_kind,
            LIVE_REPORT_A,
            &nonce,
            Some(bound_model(&receipt)),
        )
        .expect("the model is attested");
        assert!(signer_is_attested_for_model(
            &receipt.signing_address,
            &keys
        ));
    }

    /// ...and is refused against the gateway key. The mirror of the case
    /// above, and the reason the router has no "try both" arm.
    #[test]
    fn a_provider_tee_receipt_is_refused_against_the_gateway_key() {
        let receipt = live_receipt(LIVE_RECEIPT_A);
        let nonce = live_nonce(LIVE_REPORT_A);
        let gateway = attested_keys_for_receipt(
            ReceiptSignatureKind::Gateway,
            LIVE_REPORT_A,
            &nonce,
            Some(bound_model(&receipt)),
        )
        .expect("the report attests the gateway");
        assert!(!signer_is_attested_for_model(
            &receipt.signing_address,
            &gateway
        ));
    }

    /// The `model` argument is ignored on the gateway arm rather than being a
    /// second thing that has to be right: the same key comes back whatever is
    /// passed, including a model this report attests nothing for.
    #[test]
    fn the_gateway_arm_does_not_consult_the_model() {
        let nonce = live_nonce(LIVE_REPORT_A);
        let plain =
            attested_keys_for_receipt(ReceiptSignatureKind::Gateway, LIVE_REPORT_A, &nonce, None)
                .expect("attested");
        for model in [Some("Qwen/Qwen3.6-35B-A3B-FP8"), Some("Nobody/Nothing")] {
            assert_eq!(
                attested_keys_for_receipt(
                    ReceiptSignatureKind::Gateway,
                    LIVE_REPORT_A,
                    &nonce,
                    model
                )
                .expect("attested"),
                plain
            );
        }
    }

    /// An unrecognised kind -- a value this crate has not seen, and the
    /// absence of the field -- names no key source and is refused. It is not
    /// answered by trying every key.
    #[test]
    fn an_unknown_signature_kind_is_refused() {
        assert_eq!(
            ReceiptSignatureKind::from_wire("provider_tee_v2"),
            ReceiptSignatureKind::Unrecognised
        );
        assert_eq!(
            ReceiptSignatureKind::from_wire(""),
            ReceiptSignatureKind::Unrecognised
        );
        // Case is not folded, for the reason `ReceiptAlgo::from_wire` gives.
        assert_eq!(
            ReceiptSignatureKind::from_wire("Gateway"),
            ReceiptSignatureKind::Unrecognised
        );
        let nonce = live_nonce(LIVE_REPORT_A);
        assert_eq!(
            attested_keys_for_receipt(
                ReceiptSignatureKind::Unrecognised,
                LIVE_REPORT_A,
                &nonce,
                Some("Qwen/Qwen3.6-35B-A3B-FP8"),
            )
            .unwrap_err(),
            AttestedKeyError::SignatureKindUnrecognised
        );
    }

    /// A `provider_tee` receipt binding no model names no entry to look up,
    /// and is refused rather than answered with some other model's keys.
    #[test]
    fn a_provider_tee_receipt_binding_no_model_is_refused() {
        let nonce = live_nonce(LIVE_REPORT_A);
        assert_eq!(
            attested_keys_for_receipt(
                ReceiptSignatureKind::ProviderTee,
                LIVE_REPORT_A,
                &nonce,
                None
            )
            .unwrap_err(),
            AttestedKeyError::ModelNotAttested
        );
    }

    /// The gateway arm carries the same nonce binding as the model arm. A
    /// report read for a nonce this caller did not send is refused, so a
    /// replayed report cannot attest a gateway signer either.
    #[test]
    fn the_gateway_arm_refuses_a_report_bound_to_another_nonce() {
        assert_eq!(
            attested_keys_for_receipt(
                ReceiptSignatureKind::Gateway,
                LIVE_REPORT_A,
                &live_nonce(LIVE_REPORT_B),
                None
            )
            .unwrap_err(),
            AttestedKeyError::NonceMismatch
        );
    }

    /// The kind reaches a caller through the verdict, unchanged. A witness
    /// routes on `verdict.signature_kind`, so a `verify_receipt` that dropped
    /// or rewrote it would silently send every receipt to one key source.
    #[test]
    fn the_verdict_carries_the_kind_through_unchanged() {
        for kind in [
            ReceiptSignatureKind::Gateway,
            ReceiptSignatureKind::ProviderTee,
            ReceiptSignatureKind::Unrecognised,
        ] {
            let mut payload = self_signed_ed25519_receipt(REQUEST_BODY, RESPONSE_BODY);
            payload.signature_kind = kind;
            let verdict = verify_receipt(&payload, REQUEST_BODY, RESPONSE_BODY, MODEL)
                .expect("the self-signed receipt verifies");
            assert_eq!(verdict.signature_kind, kind);
        }
    }

    // ---------------------------------------------------------------------
    // The container NEAR AI actually serves per-model entries under (#802).
    //
    // Captured 2026-09-08 from the live per-model host, with a nonce
    // generated for the capture. On today's provider deployment the entries
    // arrive under `all_attestations`; `model_attestations` is absent, and so
    // is `gateway_attestation` -- a per-model host serves no gateway entry.
    // ---------------------------------------------------------------------

    const LIVE_ALL_ATTESTATIONS_REPORT: &str =
        include_str!("../tests/fixtures/near_ai_all_attestations_report_ed25519.json");

    fn all_attestations_nonce() -> String {
        serde_json::from_str::<serde_json::Value>(LIVE_ALL_ATTESTATIONS_REPORT).unwrap()
            ["_fixture_nonce"]
            .as_str()
            .expect("the capture records the nonce it was taken with")
            .to_string()
    }

    /// The precondition, asserted rather than described: this capture really
    /// does carry the entries under the other name. If a future recapture
    /// arrives with `model_attestations` again, this fails and says so,
    /// rather than letting the fallback silently become the only path tested.
    #[test]
    fn the_live_capture_uses_all_attestations_and_carries_no_gateway_entry() {
        let document: serde_json::Value =
            serde_json::from_str(LIVE_ALL_ATTESTATIONS_REPORT).expect("the capture parses");
        assert!(
            document.get("all_attestations").is_some(),
            "the capture must carry the container this test exists for"
        );
        assert!(
            document.get("model_attestations").is_none(),
            "a capture carrying both names would not pin anything"
        );
        // A per-model host serves no gateway entry at all. Worth pinning
        // because it is why the drill's gateway step cannot pass against this
        // host, which is a separate question from the model keys.
        assert!(document.get("gateway_attestation").is_none());
    }

    /// The symptom from #802, as an assertion: the per-model key is derivable
    /// from what the provider serves today.
    #[test]
    fn the_per_model_key_is_read_from_the_all_attestations_container() {
        let nonce = all_attestations_nonce();
        let keys = model_ed25519_keys(LIVE_ALL_ATTESTATIONS_REPORT, &nonce, MODEL_B)
            .expect("the live capture attests this model");

        assert_eq!(keys, vec![MODEL_B_KEY.to_string()]);
        assert!(signer_is_attested_for_model(MODEL_B_KEY, &keys));
    }

    /// And the binding really came out of the quote, not out of a JSON echo.
    /// A live entry has no `report_data` field at all, so a key returned above
    /// can only have been read at the v4 TDX offset of the entry's own quote.
    #[test]
    fn the_all_attestations_entry_binds_through_its_quote_with_no_report_data_field() {
        let document: serde_json::Value =
            serde_json::from_str(LIVE_ALL_ATTESTATIONS_REPORT).unwrap();
        let entry = &document["all_attestations"][0];
        assert!(
            entry.get("report_data").is_none(),
            "a live entry carries no report_data field; see receipt.rs"
        );
        assert!(entry.get("intel_quote").is_some());

        let nonce = all_attestations_nonce();
        assert_eq!(
            attested_ed25519_key(entry, &nonce).expect("the entry binds"),
            MODEL_B_KEY
        );
        // The same entry against a nonce nobody sent must not bind, or the
        // check above proves only that the fixture is self-consistent.
        assert_eq!(
            attested_ed25519_key(entry, &"f".repeat(64)),
            Err(AttestedKeyError::NonceMismatch)
        );
    }

    /// The binding discipline is not relaxed by the container rename: an
    /// entry naming this model that fails to bind is refused outright, and the
    /// sound sibling beside it does not rescue the report.
    #[test]
    fn a_forged_entry_beside_a_sound_one_still_refuses_the_whole_report() {
        let nonce = all_attestations_nonce();
        let mut document: serde_json::Value =
            serde_json::from_str(LIVE_ALL_ATTESTATIONS_REPORT).unwrap();
        let mut forged = document["all_attestations"][0].clone();
        // Same model, a key the quote does not commit to.
        forged["signing_address"] = serde_json::json!("11".repeat(32));
        document["all_attestations"]
            .as_array_mut()
            .unwrap()
            .push(forged);

        assert_eq!(
            model_ed25519_keys(&document.to_string(), &nonce, MODEL_B),
            Err(AttestedKeyError::ReportDataMismatch),
            "answering with the sound half is the failure this refusal prevents"
        );
    }

    /// An entry for another model under the new container is skipped, not
    /// returned. Exact match on `model_name` survives the rename.
    #[test]
    fn another_models_entry_under_the_new_container_is_not_returned() {
        let nonce = all_attestations_nonce();
        let mut document: serde_json::Value =
            serde_json::from_str(LIVE_ALL_ATTESTATIONS_REPORT).unwrap();
        document["all_attestations"][0]["model_name"] = serde_json::json!(MODEL_A);

        assert_eq!(
            model_ed25519_keys(&document.to_string(), &nonce, MODEL_B),
            Err(AttestedKeyError::ModelNotAttested)
        );
    }

    /// **The one that matters most.** A gateway-shaped report yields no model
    /// keys at all -- never the gateway key. The gateway key signs no
    /// `provider_tee` receipt, and returning it here would silently restore
    /// the exact confusion this function was written to prevent.
    #[test]
    fn a_gateway_only_report_yields_no_model_keys_and_never_the_gateway_key() {
        let gateway_only = format!(
            r#"{{"gateway_attestation":{{"signing_address":"{LIVE_GATEWAY_KEY}","signing_algo":"ed25519","request_nonce":"{REPORT_NONCE}","report_data":"{LIVE_GATEWAY_KEY}{REPORT_NONCE}"}},"ohttp_attestation":{{}}}}"#
        );
        // Precondition: the gateway half of this report is genuinely sound,
        // so a failure below is the model lookup refusing and not the report
        // being unreadable.
        assert_eq!(
            gateway_ed25519_key(&gateway_only, REPORT_NONCE).expect("the gateway binds"),
            LIVE_GATEWAY_KEY
        );

        let error = model_ed25519_keys(&gateway_only, REPORT_NONCE, MODEL_B)
            .expect_err("a gateway-only report attests no model");
        assert_eq!(error, AttestedKeyError::Malformed);
    }

    /// An empty container is `ModelNotAttested`, not a silent empty success.
    /// An empty key set would make `signer_is_attested_for_model` answer
    /// `false` for everything, which reads as a refusal but is really "we
    /// never looked".
    #[test]
    fn an_empty_all_attestations_container_is_not_a_silent_pass() {
        let empty = r#"{"all_attestations":[]}"#;
        assert_eq!(
            model_ed25519_keys(empty, REPORT_NONCE, MODEL_B),
            Err(AttestedKeyError::ModelNotAttested)
        );
    }

    /// The fallback is a fallback, not a replacement: the older container
    /// still works. The observation window for the rename is one provider
    /// deployment on one day, and a rename that went one way can go back.
    #[test]
    fn the_older_model_attestations_container_still_works() {
        let keys =
            model_ed25519_keys(&live_report(), REPORT_NONCE, MODEL_A).expect("still attested");
        assert_eq!(keys, vec![MODEL_A_KEY.to_string()]);
    }

    /// And where a report somehow carries both, the newer container is the
    /// one read -- with the older one holding an entry that would bind, so a
    /// reader that consulted the wrong container would pass rather than fail.
    #[test]
    fn all_attestations_wins_when_a_report_carries_both_containers() {
        let nonce = all_attestations_nonce();
        let mut document: serde_json::Value =
            serde_json::from_str(LIVE_ALL_ATTESTATIONS_REPORT).unwrap();
        // A bindable entry for the same model, under the older name, carrying
        // a different key. Whichever container is consulted, a key comes back
        // -- so the assertion below is about *which*, not about success.
        let decoy_key = "22".repeat(32);
        document["model_attestations"] = serde_json::json!([{
            "model_name": MODEL_B,
            "signing_address": decoy_key,
            "signing_algo": "ed25519",
            "request_nonce": nonce,
            "report_data": format!("{decoy_key}{nonce}"),
        }]);

        assert_eq!(
            model_ed25519_keys(&document.to_string(), &nonce, MODEL_B),
            Ok(vec![MODEL_B_KEY.to_string()]),
            "the live container must win over the legacy one"
        );
    }
}
