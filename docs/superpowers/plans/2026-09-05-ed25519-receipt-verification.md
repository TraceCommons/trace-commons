# Ed25519 Receipt Verification Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Verify NEAR AI receipts against the key that is actually bound into the gateway's TDX quote — the ed25519 one — instead of an ECDSA key that appears in no attestation report.

**Architecture:** `ReceiptPayload` gains a `signing_algo` discriminator and `verify_receipt` dispatches on it: the existing EIP-191 secp256k1 recovery path for `Ecdsa`, and a new plain-Ed25519-over-raw-text path for `Ed25519` via `ring` (already in the graph — zero new packages). The witness wire shape accepts an optional `signing_algo` (defaulting to `ecdsa`) so a new client never breaks against an old witness; the contributor then switches its fetch to `signing_algo=ed25519` and forwards the discriminator. An optional, config-gated check compares the receipt signer to the gateway address in a freshly-nonced attestation report.

**Tech Stack:** Rust; `ring 0.17` (`signature::UnparsedPublicKey` with `ED25519`); existing `k256`/`sha3` EIP-191 path untouched.

**Spec:** `docs/superpowers/specs/2026-09-05-ed25519-receipt-verification-design.md`

## Global Constraints

- Verify with `RUSTFLAGS='-D warnings'`. Plain `cargo check` does not apply it; CI does.
- `cargo --workspace` misses two configurations CI gates. After ANY change to `-attestation`, `-contributor` or `-protocol`, also run the four permissive crates with `--no-default-features` and the GTK workspace with `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`. Both broke CI on 2026-09-04.
- Clippy allow-list, verbatim: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- No emojis. Commit subjects short and imperative, no `feat:`/`fix:` prefix.
- Hash-only logging: never log a receipt, a signing key, a chat id, or a body.
- License boundary: `-attestation`, `-contributor`, `-protocol` are MIT OR Apache-2.0; `-server` is AGPL. Never add a server or gate dependency to a permissive crate. Never edit the expected sets in `crates/trace-commons-server/tests/license_boundary.rs`.
- **The only dependency change permitted is adding `ring = "0.17"` as a direct dependency of `trace-commons-attestation`.** It is already a direct dependency of `-contributor` and `-server` and already in `-attestation`'s transitive graph, so it adds no packages. `git diff --stat -- '*Cargo.lock'` must show no new package lines.
- **The ed25519 signature is plain Ed25519 over the raw `text` bytes.** It is NOT EIP-191 prefixed. Confirmed against a live receipt: the raw text verifies, the prefixed form does not. Applying the EIP-191 prefix on the ed25519 path is the single most likely implementation error.
- The witness request body is `#[serde(deny_unknown_fields)]`. A field the witness does not know is a 400 on every submission, not a degraded one.

## Live fixture, captured 2026-09-04

Every value below came from a real NEAR AI receipt for `chat_id ee64b242d74f4c7eb59b05b046f33f7b`, model `Qwen/Qwen3.6-35B-A3B-FP8`, and was cross-checked against the ECDSA receipt for the same chat and against the IronWire ledger — all three bind identical bytes.

```
text            81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e
signature       838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c
signing_address cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6
signing_algo    ed25519
```

The ed25519 `signing_address` is the 32-byte public key as 64 hex chars with **no `0x`**. The ECDSA form is a 20-byte `0x`-prefixed address. The two are distinguishable by length alone, but the plan discriminates by the explicit `signing_algo` field, never by guessing from length.

---

### Task 1: Ed25519 verification in the attestation crate

One atomic change: the discriminator, the new verify path, and every existing construction site set to `Ecdsa` so behaviour is preserved everywhere.

**Files:**
- Modify: `crates/trace-commons-attestation/Cargo.toml` (add `ring = "0.17"`)
- Modify: `crates/trace-commons-attestation/src/receipt.rs`
- Modify (add `signing_algo: ReceiptAlgo::Ecdsa` to every `ReceiptPayload {` literal):
  - `crates/trace-commons-server/src/witness_service/inference.rs` (5 sites)
  - `crates/trace-commons-server/src/witness_service/http.rs` (2 sites)
  - `crates/trace-commons-server/tests/witness_certificate_cross_implementation.rs` (2 sites)
  - `crates/trace-commons-server/tests/near_ai_live_receipt.rs` (1 site)
  - `crates/trace-commons-server/src/near_attestation/drill.rs` (1 site)
  - `crates/trace-commons-server/src/near_attestation/client.rs` (1 site)
  - `crates/trace-commons-contributor/src/witness/transport.rs` (2 sites)
  - `crates/trace-commons-contributor/src/routing/receipt.rs` (1 site)
- Test: `crates/trace-commons-attestation/src/receipt.rs` (in-crate `#[cfg(test)] mod tests`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub enum ReceiptAlgo { Ecdsa, Ed25519 }
  impl ReceiptAlgo {
      /// The wire spelling NEAR AI uses in `signing_algo`.
      pub fn as_wire(self) -> &'static str;   // "ecdsa" | "ed25519"
      /// Parse the wire spelling; `None` for anything else.
      pub fn from_wire(s: &str) -> Option<Self>;
  }
  pub struct ReceiptPayload { pub text, pub signature, pub signing_address, pub signing_algo: ReceiptAlgo }
  // ReceiptVerdict gains: pub signing_algo: ReceiptAlgo
  // ReceiptError gains: Ed25519KeyMalformed, Ed25519SignatureMalformed, Ed25519SignatureInvalid
  ```
  Tasks 2 and 3 construct `ReceiptPayload` with the new field and read `ReceiptAlgo::from_wire`.

- [ ] **Step 1: Add the dependency and confirm it adds no packages**

In `crates/trace-commons-attestation/Cargo.toml`, next to `k256`:

```toml
# Ed25519 receipt verification. Already a direct dependency of
# trace-commons-contributor and trace-commons-server and already in this
# crate's graph, so this line adds no packages -- it makes an existing
# transitive edge a direct one. Measured before adding: `cargo tree -p
# trace-commons-attestation -e normal | grep ring` was non-empty.
ring = "0.17"
```

```bash
cargo check -p trace-commons-attestation
git diff --stat -- Cargo.lock
```

Expected: `Cargo.lock` shows a change only under `trace-commons-attestation`'s own `dependencies` list — no new `[[package]]` entry. If a new package appears, stop: the premise is wrong.

- [ ] **Step 2: Write the failing tests**

Add to the `tests` module in `crates/trace-commons-attestation/src/receipt.rs`:

```rust
const LIVE_ED25519_TEXT: &str = "81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e";
const LIVE_ED25519_SIGNATURE: &str = "838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c";
const LIVE_ED25519_KEY: &str = "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";

fn live_ed25519() -> ReceiptPayload {
    ReceiptPayload {
        text: LIVE_ED25519_TEXT.to_string(),
        signature: LIVE_ED25519_SIGNATURE.to_string(),
        signing_address: LIVE_ED25519_KEY.to_string(),
        signing_algo: ReceiptAlgo::Ed25519,
    }
}

/// A receipt NEAR AI actually issued, verified as plain Ed25519 over the
/// raw text. The signature came from a key this project never held. The
/// digest checks are exercised separately, because this test does not hold
/// NEAR AI's real bodies and cannot forge ones with the same digests.
#[test]
fn a_live_ed25519_receipt_signature_verifies_over_the_raw_text() {
    let payload = live_ed25519();
    verify_ed25519_signature(&payload).expect("NEAR AI's real signature verifies");
}

/// The EIP-191 prefix must NOT be applied on the ed25519 path. The live
/// signature is over the raw text; prefixing it makes verification fail.
/// This is the single most likely implementation mistake.
#[test]
fn the_ed25519_path_does_not_apply_the_eip191_prefix() {
    let mut prefixed = live_ed25519();
    prefixed.text = format!(
        "\x19Ethereum Signed Message:\n{}{}",
        LIVE_ED25519_TEXT.len(),
        LIVE_ED25519_TEXT
    );
    assert_eq!(
        verify_ed25519_signature(&prefixed),
        Err(ReceiptError::Ed25519SignatureInvalid),
        "a prefixed text must not verify; the signature is over the raw text"
    );
}

/// A validly signed ed25519 receipt over different bytes is refused by
/// the digest check, not the signature check -- the same ordering the
/// ECDSA path already pins.
#[test]
fn an_ed25519_receipt_over_other_bytes_is_refused() {
    let payload = live_ed25519();
    let err = verify_receipt(&payload, b"not the request", b"not the response", "Qwen/Qwen3.6-35B-A3B-FP8")
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
    assert_eq!(verify_ed25519_signature(&short_key), Err(ReceiptError::Ed25519KeyMalformed));

    let mut short_sig = live_ed25519();
    short_sig.signature = "8387".to_string();
    assert_eq!(verify_ed25519_signature(&short_sig), Err(ReceiptError::Ed25519SignatureMalformed));
}

/// The wire spelling round-trips, and anything else is `None` rather than
/// a guess.
#[test]
fn the_algo_wire_spelling_round_trips_and_rejects_the_unknown() {
    assert_eq!(ReceiptAlgo::from_wire("ecdsa"), Some(ReceiptAlgo::Ecdsa));
    assert_eq!(ReceiptAlgo::from_wire("ed25519"), Some(ReceiptAlgo::Ed25519));
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
    assert_eq!(verify_ed25519_signature(&wrong), Err(ReceiptError::Ed25519KeyMalformed));
}
```

- [ ] **Step 3: Run the tests and watch them fail to compile**

```bash
cargo test -p trace-commons-attestation --lib receipt
```

Expected: compile errors — `ReceiptAlgo`, `verify_ed25519_signature`, `signing_algo`, and the three new error variants do not exist.

- [ ] **Step 4: Add the discriminator, the errors, and the field**

In `crates/trace-commons-attestation/src/receipt.rs`, above `ReceiptPayload`:

```rust
/// Which signature scheme a receipt carries.
///
/// NEAR AI issues both. Only the ed25519 signer is bound into the gateway's
/// TDX quote (`report_data == signing_address || nonce`); the ECDSA signer
/// appears in no attestation report. So `Ed25519` is the form that lets a
/// verified receipt mean "signed inside an attested enclave", and `Ecdsa`
/// remains for receipts already issued and for callers that have not moved.
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
```

Add `pub signing_algo: ReceiptAlgo,` as the last field of `ReceiptPayload` and of `ReceiptVerdict`, with this doc on the payload field:

```rust
    /// Which scheme `signature` and `signing_address` are in. See
    /// [`ReceiptAlgo`] for why this is explicit rather than inferred.
    pub signing_algo: ReceiptAlgo,
```

Add to `ReceiptError`, after `SigningAddressMalformed`:

```rust
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
```

- [ ] **Step 5: Add the ed25519 verifier and dispatch on the discriminator**

Add to `receipt.rs`:

```rust
/// Verify an ed25519 receipt's signature over its raw `text`.
///
/// Plain Ed25519, no EIP-191 prefix. Confirmed against a live NEAR AI
/// receipt: the raw text verifies, the prefixed form does not. Public
/// because a caller holding an attestation-report key may want the
/// signature check alone, without bodies.
///
/// # Errors
///
/// Malformed key or signature by name, before any cryptography; then
/// [`ReceiptError::Ed25519SignatureInvalid`] for a well-formed signature
/// that does not verify.
pub fn verify_ed25519_signature(payload: &ReceiptPayload) -> Result<(), ReceiptError> {
    let key = hex::decode(&payload.signing_address).map_err(|_| ReceiptError::Ed25519KeyMalformed)?;
    if key.len() != 32 {
        return Err(ReceiptError::Ed25519KeyMalformed);
    }
    let sig = hex::decode(payload.signature.strip_prefix("0x").unwrap_or(&payload.signature))
        .map_err(|_| ReceiptError::Ed25519SignatureMalformed)?;
    if sig.len() != 64 {
        return Err(ReceiptError::Ed25519SignatureMalformed);
    }
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(payload.text.as_bytes(), &sig)
        .map_err(|_| ReceiptError::Ed25519SignatureInvalid)
}
```

In `verify_receipt`, replace the block that decodes the claimed address and recovers the signer with a dispatch. The existing code is:

```rust
    let claimed_address =
        decode_address(&payload.signing_address).ok_or(ReceiptError::SigningAddressMalformed)?;

    let recovered = recover_eip191_signer(payload.text.as_bytes(), &payload.signature)?;
    if recovered != claimed_address {
        return Err(ReceiptError::SignerMismatch);
    }
```

Replace it with:

```rust
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
```

and in the `Ok(ReceiptVerdict { .. })` at the end, replace `signing_address: format!("0x{}", hex::encode(recovered)),` with `signing_address,` and add `signing_algo: payload.signing_algo,`.

- [ ] **Step 6: Set every existing construction site to `Ecdsa`**

In each of the files listed under **Files**, add `signing_algo: ReceiptAlgo::Ecdsa,` to every `ReceiptPayload {` literal, importing `trace_commons_attestation::receipt::ReceiptAlgo` where needed. In `receipt.rs`'s own tests, the five existing literals also get `signing_algo: ReceiptAlgo::Ecdsa`. Where a `ReceiptVerdict {` literal exists, add `signing_algo: ReceiptAlgo::Ecdsa` there too.

This is behaviour-preserving: nothing that constructed an ECDSA receipt yesterday verifies differently today.

- [ ] **Step 7: Run the attestation tests**

```bash
RUSTFLAGS='-D warnings' cargo test -p trace-commons-attestation --lib
```

Expected: every new test passes, including `a_live_ed25519_receipt_signature_verifies_over_the_raw_text`, and every pre-existing receipt test still passes.

- [ ] **Step 8: Prove the prefix test is non-vacuous by mutation**

Temporarily change `verify_ed25519_signature` to verify over the EIP-191-prefixed message instead of the raw text:

```rust
    // MUTATION -- revert after running
    let prefixed = format!("\x19Ethereum Signed Message:\n{}{}", payload.text.len(), payload.text);
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key)
        .verify(prefixed.as_bytes(), &sig)
```

```bash
cargo test -p trace-commons-attestation --lib receipt 2>&1 | grep -E "FAILED|test result"
```

Expected: `a_live_ed25519_receipt_signature_verifies_over_the_raw_text` FAILS (the live signature no longer verifies) and `the_ed25519_path_does_not_apply_the_eip191_prefix` FAILS (the prefixed text now verifies). Revert the mutation. Re-run: all pass. Record both outcomes in your report.

- [ ] **Step 9: The whole workspace, plus the two hidden configurations**

```bash
cargo fmt --all
RUSTFLAGS='-D warnings' cargo check --workspace
RUSTFLAGS='-D warnings' cargo test -p trace-commons-attestation -p trace-commons-contributor -p trace-commons-server
cargo clippy --workspace --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
for c in trace-commons-protocol trace-commons-attestation trace-commons-contributor trace-commons-contributor-ffi; do RUSTFLAGS='-D warnings' cargo check -p $c --no-default-features; done
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git diff --stat -- '*Cargo.lock'
```

Expected: all clean; the lockfile diff shows no new `[[package]]`.

- [ ] **Step 10: Commit**

```bash
git add -A
git commit -m "Verify ed25519 receipts over the raw text

ReceiptPayload gains an explicit signing_algo, and verify_receipt
dispatches on it: the existing EIP-191 recovery path for Ecdsa, and plain
Ed25519 over the raw text for Ed25519. The prefix is deliberately not
applied on the new path -- a live NEAR AI receipt verifies raw and fails
prefixed, and a test pins that in both directions.

Every existing construction site is set to Ecdsa, so nothing that
verified yesterday verifies differently today.

ring becomes a direct dependency of this crate. It was already in the
crate's graph and is a direct dependency of the contributor and server
crates, so the lockfile gains no package."
```

---

### Task 2: The witness accepts the discriminator on the wire

The witness must accept `signing_algo` **before** any client sends it, because the request body is `deny_unknown_fields` and an unknown field is a 400 on every submission.

**Files:**
- Modify: `crates/trace-commons-server/src/witness_service/http.rs:258-272` (`InferenceReceiptBody` and its `From`)
- Test: `crates/trace-commons-server/tests/witness_certificate_cross_implementation.rs`

**Interfaces:**
- Consumes: `ReceiptAlgo::from_wire`, `ReceiptPayload.signing_algo` from Task 1.
- Produces: the witness accepts `"inference_receipt": {text, signature, signing_address, signing_algo?}` where `signing_algo` is optional and defaults to `"ecdsa"`; an unrecognised value is `400 witness_request_malformed`. Task 3 sends this field.

- [ ] **Step 1: Write the failing test**

Add to `crates/trace-commons-server/tests/witness_certificate_cross_implementation.rs`, beside the existing receipt tests:

```rust
/// A client may omit `signing_algo`, and gets ECDSA -- every receipt
/// issued before this field existed is ECDSA, and a witness that refused
/// them would refuse every existing client.
#[tokio::test]
async fn a_receipt_without_signing_algo_is_read_as_ecdsa() {
    let (app, _guard) = requiring_witness().await;
    let body = contribution_body_with_receipt(serde_json::json!({
        "text": "aaaa1111:bbbb2222",
        "signature": "0xcccc3333",
        "signing_address": "0xdddd444444444444444444444444444444444444"
    }));
    let response = post_witness(&app, body).await;
    // Refused for an unverifiable signature, NOT for a malformed request:
    // the field's absence was accepted and the payload reached the verifier.
    assert_eq!(response.status(), 403);
    assert_eq!(refusal_label(response).await, "witness_inference_receipt_unverified");
}

```rust
/// `signing_algo` is carried through to the verifier. This test cannot
/// hold NEAR AI's real bodies, so it proves the discriminator reached the
/// verifier the other way round: an ed25519 receipt over OTHER bytes is
/// refused for its digest, which means its signature was checked as
/// ed25519 and passed. Read as ECDSA, the same payload fails earlier, as
/// a malformed 20-byte address, and never reaches the digest.
#[tokio::test]
async fn an_ed25519_receipt_reaches_the_digest_check() {
    let (app, _guard) = requiring_witness().await;
    let body = contribution_body_with_receipt(serde_json::json!({
        "text": "81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e",
        "signature": "838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c",
        "signing_address": "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6",
        "signing_algo": "ed25519"
    }));
    let response = post_witness(&app, body).await;
    assert_eq!(response.status(), 403);
    assert_eq!(refusal_label(response).await, "witness_inference_receipt_unverified");
    // The distinguishing assertion: the server-side reason is a digest
    // mismatch. Read the structured refusal the existing suite already
    // exposes for this label and assert its `reason` is the request-hash
    // mismatch, not a signature or key error.
}
```

Look at how the existing cross-implementation tests read the refusal reason (there is a helper that decodes it) and use that for the final assertion. If no such reason is exposed, add the `reason` to the refusal's structured detail in `http.rs` as a fixed label — never the receipt itself.

- [ ] **Step 2: Run and watch them fail**

```bash
cargo test -p trace-commons-server --test witness_certificate_cross_implementation signing_algo
```

Expected: `a_receipt_without_signing_algo_is_read_as_ecdsa` may already pass (absence is the status quo); `an_unknown_signing_algo_is_refused_as_malformed` FAILS because `deny_unknown_fields` currently rejects ANY `signing_algo` as malformed — which looks like a pass but is for the wrong reason; `an_ed25519_receipt_reaches_the_digest_check` FAILS with 400.

- [ ] **Step 3: Accept the field**

In `crates/trace-commons-server/src/witness_service/http.rs`, `InferenceReceiptBody`:

```rust
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InferenceReceiptBody {
    text: String,
    signature: String,
    signing_address: String,
    /// Which scheme the receipt is in. Optional, defaulting to `ecdsa`:
    /// every receipt issued before this field existed is ECDSA, and a
    /// witness that required the field would refuse every existing client.
    /// An unrecognised value is a malformed request, never a guess.
    #[serde(default)]
    signing_algo: Option<String>,
}
```

Replace the `From<InferenceReceiptBody> for ReceiptPayload` impl with a fallible conversion:

```rust
impl TryFrom<InferenceReceiptBody> for ReceiptPayload {
    type Error = Refusal;

    fn try_from(body: InferenceReceiptBody) -> Result<Self, Refusal> {
        let signing_algo = match body.signing_algo.as_deref() {
            None => ReceiptAlgo::Ecdsa,
            Some(s) => ReceiptAlgo::from_wire(s)
                .ok_or_else(|| Refusal::new(StatusCode::BAD_REQUEST, "witness_request_malformed"))?,
        };
        Ok(ReceiptPayload {
            text: body.text,
            signature: body.signature,
            signing_address: body.signing_address,
            signing_algo,
        })
    }
}
```

and at the two call sites (`http.rs:322` and `:344`) replace `body.inference_receipt.map(Into::into)` with:

```rust
offered_receipt: body.inference_receipt.map(ReceiptPayload::try_from).transpose()?,
```

adjusting the surrounding `?` context so a malformed algo surfaces as the 400 refusal.

- [ ] **Step 4: Run the suite**

```bash
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test witness_certificate_cross_implementation
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --lib witness_service
```

Expected: all pass, including the three new tests.

- [ ] **Step 5: Mutation — drop the dispatch**

Temporarily make `try_from` ignore `signing_algo` and always produce `ReceiptAlgo::Ecdsa`. Run the suite. Expected: `an_ed25519_receipt_reaches_the_digest_check` FAILS (the ed25519 payload is now refused as a malformed address before the digest). Revert.

- [ ] **Step 6: Verify and commit**

```bash
cargo fmt --all
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
git add -A
git commit -m "Let the witness read which scheme a receipt is in

inference_receipt gains an optional signing_algo. Optional and defaulting
to ecdsa because every receipt issued before this field existed is ECDSA,
and the request body is deny_unknown_fields -- a witness that required
the field would 400 every existing client. An unrecognised value is a
malformed request, never a silent default.

The witness must accept this before any client sends it, which is why
this lands ahead of the contributor change."
```

- [ ] **Step 7: Deploy the witness — STOP AND CONFIRM FIRST**

This is a production deploy. Open the PR for Tasks 1 and 2, merge on green, then build and deploy exactly as the release plan's Tasks 3–5 did:

```bash
gh workflow run witness-image.yml --repo TraceCommons/trace-commons --ref main
# take the digest, pin it in deploy/witness/docker-compose.yml, regenerate app-compose.json
cd deploy/witness
phala deploy -c docker-compose.yml --cvm-id 8b8e6543-9743-41fc-ac05-a6b414888d5e \
  --no-public-logs --no-public-sysinfo --public-tcbinfo \
  -e TRACE_NEAR_AI_PRIVACY_API_KEY="$TRACE_NEAR_AI_PRIVACY_API_KEY"
```

Then read the manifest back, confirm `public_logs: False`, confirm the signing address is still `0x655a17fcf6d0b9069e1b1dd07a7f5535d0c76798`, and **record the new measurement** — it changes on every redeploy, and the pilot's `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` must be updated to match or every certificate is refused.

---

### Task 3: The contributor fetches and forwards ed25519

**Files:**
- Modify: `crates/trace-commons-contributor/src/routing/receipt.rs` (`parse_receipt_response`, `receipt_url`, module doc line 3)
- Modify: `crates/trace-commons-contributor/src/witness/transport.rs` (the `inference_receipt` JSON the client builds)
- Test: both files' in-crate test modules

**Interfaces:**
- Consumes: `ReceiptAlgo` from Task 1; the witness accepting `signing_algo` from Task 2.
- Produces: the contributor fetches `signing_algo=ed25519`, parses the response's `signing_algo`, and forwards it to the witness as `"signing_algo"`.

- [ ] **Step 1: Write the failing tests**

In `crates/trace-commons-contributor/src/routing/receipt.rs` tests:

```rust
/// The fetch asks for the scheme whose signer is bound into the gateway's
/// TDX quote. The ECDSA signer appears in no attestation report.
#[test]
fn the_receipt_url_asks_for_ed25519() {
    let url = receipt_url("https://cloud-api.near.ai/v1", "ee64b242d74f4c7eb59b05b046f33f7b", "Qwen/Qwen3.6-35B-A3B-FP8").unwrap();
    assert!(url.query().unwrap().contains("signing_algo=ed25519"), "{url}");
    assert!(!url.query().unwrap().contains("ecdsa"), "{url}");
}

/// The response's own `signing_algo` is what the payload records, not
/// what was asked for. A provider answering a different scheme than
/// requested is a fact to carry, not to overwrite.
#[test]
fn the_parsed_receipt_carries_the_scheme_the_provider_answered() {
    let body = r#"{"text":"81e9887990592366b55ef758cad3b3a097e890871bedc023a51b2828ed237cc3:6f7091a0fbe5917a631c70805833760fe63ceea3493466e3230bd830816a3f2e","signature":"838765bd299514ec80084d50b7cef9357172ce2923dd35aa837beed0c6af04e684673e61db6c0d3ae8d69476b680d94c8e1e36e05277a1b103c27a12f563eb0c","signing_address":"cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6","signing_algo":"ed25519","signature_kind":"gateway"}"#;
    let payload = parse_receipt_response(body).unwrap();
    assert_eq!(payload.signing_algo, ReceiptAlgo::Ed25519);
    assert_eq!(payload.signing_address, "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6");
}

/// A response with no `signing_algo` is ECDSA -- the pre-field form.
#[test]
fn a_response_without_signing_algo_is_ecdsa() {
    let body = r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd"}"#;
    assert_eq!(parse_receipt_response(body).unwrap().signing_algo, ReceiptAlgo::Ecdsa);
}

/// An unrecognised `signing_algo` is a malformed response, not a guess.
#[test]
fn an_unknown_signing_algo_in_the_response_is_malformed() {
    let body = r#"{"text":"a:b","signature":"0xcc","signing_address":"0xdd","signing_algo":"rsa"}"#;
    assert_eq!(parse_receipt_response(body).unwrap_err(), ReceiptFetchError::ResponseMalformed);
}
```

In `crates/trace-commons-contributor/src/witness/transport.rs` tests, beside the existing `the_offered_receipt_reaches_the_witness_in_the_field_it_reads`:

```rust
/// The scheme travels with the receipt. The witness reads
/// `signing_algo` and dispatches on it; a receipt sent without it is read
/// as ECDSA, so an ed25519 receipt sent bare would be refused as a
/// malformed 20-byte address.
#[test]
fn the_offered_receipt_carries_its_scheme_to_the_witness() {
    let receipt = ReceiptPayload {
        text: "aaaa1111:bbbb2222".to_string(),
        signature: "cccc3333".to_string(),
        signing_address: "dddd4444".to_string(),
        signing_algo: ReceiptAlgo::Ed25519,
    };
    let body = witness_request_body(&sample_contribution(), &sample_grants(), Some(&receipt));
    let document: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(document["inference_receipt"]["signing_algo"], "ed25519");
}
```

Use whatever the existing test in that module uses for a contribution and grants fixture in place of `sample_contribution()` / `sample_grants()` — copy the names from `the_offered_receipt_reaches_the_witness_in_the_field_it_reads`.

- [ ] **Step 2: Run and watch them fail**

```bash
cargo test -p trace-commons-contributor --lib routing::receipt
cargo test -p trace-commons-contributor --lib witness::transport
```

Expected: `the_receipt_url_asks_for_ed25519` FAILS (`ecdsa` present); the parse tests fail on the missing field handling; the transport test FAILS (`signing_algo` absent from the document).

- [ ] **Step 3: Implement**

In `routing/receipt.rs`:

- Line 3 module doc: change `signing_algo=ecdsa` to `signing_algo=ed25519` and add one sentence: "Ed25519, because that signer is the one bound into the gateway's attestation quote; the ECDSA signer appears in no report."
- In `receipt_url`, change `.append_pair("signing_algo", "ecdsa")` to `.append_pair("signing_algo", ReceiptAlgo::Ed25519.as_wire())`.
- In `parse_receipt_response`, after the three existing fields:

```rust
    let signing_algo = match receipt.get("signing_algo").and_then(serde_json::Value::as_str) {
        None => ReceiptAlgo::Ecdsa,
        Some(s) => ReceiptAlgo::from_wire(s).ok_or(ReceiptFetchError::ResponseMalformed)?,
    };
    Ok(ReceiptPayload { text: field("text")?, signature: field("signature")?, signing_address: field("signing_address")?, signing_algo })
```

In `witness/transport.rs`, where `witness_request_body` emits `"inference_receipt": {text, signature, signing_address}`, add `"signing_algo": receipt.signing_algo.as_wire()`.

- [ ] **Step 4: Run, then verify everything**

```bash
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib
RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test witness_certificate_cross_implementation
cargo fmt --all
cargo clippy --workspace --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
for c in trace-commons-protocol trace-commons-attestation trace-commons-contributor trace-commons-contributor-ffi; do RUSTFLAGS='-D warnings' cargo check -p $c --no-default-features; done
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: all clean. The cross-implementation suite is what proves the client's bytes meet the server's deserialiser.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Fetch the receipt whose signer is attested

The contributor asked NEAR AI for signing_algo=ecdsa, recovered a key
that appears in no attestation report, and forwarded it. It now asks for
ed25519 -- the signer bound into the gateway's TDX quote -- records the
scheme the provider actually answered rather than the one requested, and
carries it to the witness, which dispatches on it.

A response with no signing_algo is read as ECDSA, the pre-field form; an
unrecognised value is a malformed response, never a guess."
```

---

### Task 4: Optional check against the attestation report

Config-gated and off by default: it costs a second network call and depends on a report-freshness policy the spec leaves open.

**Files:**
- Create: `crates/trace-commons-contributor/src/routing/attestation_report.rs`
- Modify: `crates/trace-commons-contributor/src/routing/mod.rs` (declare the module)
- Modify: `crates/trace-commons-contributor/src/config.rs` (one new field)
- Modify: `crates/trace-commons-contributor/src/submit.rs` (call the check where the receipt is fetched)
- Test: the new file's in-crate test module

**Interfaces:**
- Consumes: `ReceiptVerdict.signing_address` and `.signing_algo` from Task 1.
- Produces:
  ```rust
  pub fn gateway_ed25519_key(report_json: &str, expected_nonce: &str) -> Result<String, AttestationReportError>;
  pub fn attestation_report_url(base: &str, model: &str, nonce: &str) -> Result<url::Url, AttestationReportError>;
  // config: ContributorConfig.inference_receipt_check_attestation: bool  (default false)
  // refusal label: "receipt_signer_not_attested"
  ```

- [ ] **Step 1: Write the failing tests**

In the new `attestation_report.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    /// The shape NEAR AI actually returns, reduced to the fields read.
    /// `report_data` is `signing_address || request_nonce`, which is what
    /// binds the key to a caller-chosen nonce inside the TDX quote.
    const NONCE: &str = "482934fb749d13aa81b2e543a253cf4d8cc847dab55a8d49989effd5023ddb5d";
    const KEY: &str = "cb6fc58f6bd685919fa42fb54d3fcfe03222e324bdda91f0bac6d5c73dc4f1c6";

    fn report(nonce_in_report_data: &str) -> String {
        format!(r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ed25519","request_nonce":"{NONCE}","report_data":"{KEY}{nonce_in_report_data}"}}}}"#)
    }

    #[test]
    fn the_gateway_key_is_read_when_report_data_binds_the_nonce_we_sent() {
        assert_eq!(gateway_ed25519_key(&report(NONCE), NONCE).unwrap(), KEY);
    }

    /// A report whose report_data carries a different nonce is stale or
    /// replayed, and its key is not accepted. This is the whole point of
    /// sending a nonce.
    #[test]
    fn a_report_for_a_different_nonce_is_refused() {
        let other = "0".repeat(64);
        assert_eq!(
            gateway_ed25519_key(&report(&other), NONCE).unwrap_err(),
            AttestationReportError::NonceMismatch
        );
    }

    /// report_data must equal key || nonce exactly. A report that lists the
    /// key but whose report_data does not commit to it is not a binding.
    #[test]
    fn report_data_that_does_not_commit_to_the_key_is_refused() {
        let body = format!(r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ed25519","request_nonce":"{NONCE}","report_data":"{}{NONCE}"}}}}"#, "f".repeat(64));
        assert_eq!(gateway_ed25519_key(&body, NONCE).unwrap_err(), AttestationReportError::ReportDataMismatch);
    }

    #[test]
    fn a_non_ed25519_gateway_is_refused() {
        let body = format!(r#"{{"gateway_attestation":{{"signing_address":"{KEY}","signing_algo":"ecdsa","request_nonce":"{NONCE}","report_data":"{KEY}{NONCE}"}}}}"#);
        assert_eq!(gateway_ed25519_key(&body, NONCE).unwrap_err(), AttestationReportError::NotEd25519);
    }

    #[test]
    fn the_report_url_carries_model_and_nonce() {
        let u = attestation_report_url("https://cloud-api.near.ai/v1", "Qwen/Qwen3.6-35B-A3B-FP8", NONCE).unwrap();
        assert_eq!(u.path(), "/v1/attestation/report");
        assert!(u.query().unwrap().contains(&format!("nonce={NONCE}")));
        assert!(u.query().unwrap().contains("model=Qwen%2FQwen3.6-35B-A3B-FP8"));
    }
}
```

- [ ] **Step 2: Run and watch them fail to compile**

```bash
cargo test -p trace-commons-contributor --lib routing::attestation_report
```

- [ ] **Step 3: Implement the parser**

`crates/trace-commons-contributor/src/routing/attestation_report.rs`:

```rust
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: MIT OR Apache-2.0

//! Reading the gateway signing key out of a NEAR AI attestation report.
//!
//! `GET {base}/attestation/report?model=..&nonce=..` returns, among much
//! else, a `gateway_attestation` whose `report_data` is
//! `signing_address || request_nonce`. That concatenation, inside a TDX
//! quote, is what binds the key to a nonce this client chose -- so a key
//! read from a report whose `report_data` carries OUR nonce is one that was
//! attested for us, now, rather than one copied from an old report.
//!
//! **This module does not verify the quote.** It reads the report's
//! self-description and checks its internal consistency. Until quote
//! verification exists, a key from here is a claim by NEAR AI, not a proof,
//! and the config gate that enables this check says so.
//!
//! Nothing here logs. The report holds keys and identifiers; none of them
//! belong on an operational surface.

use trace_commons_attestation::receipt::ReceiptAlgo;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttestationReportError {
    #[error("attestation report is not the expected JSON shape")]
    Malformed,
    #[error("attestation report gateway is not ed25519")]
    NotEd25519,
    #[error("attestation report was issued for a different nonce")]
    NonceMismatch,
    #[error("attestation report_data does not commit to the listed key and nonce")]
    ReportDataMismatch,
    #[error("attestation report base URL is not a valid https URL")]
    UrlInvalid,
}

/// The URL a fetch would call.
pub fn attestation_report_url(base: &str, model: &str, nonce: &str) -> Result<url::Url, AttestationReportError> {
    let mut url = url::Url::parse(base).map_err(|_| AttestationReportError::UrlInvalid)?;
    if url.scheme() != "https" {
        return Err(AttestationReportError::UrlInvalid);
    }
    {
        let mut segments = url.path_segments_mut().map_err(|_| AttestationReportError::UrlInvalid)?;
        segments.pop_if_empty().push("attestation").push("report");
    }
    url.query_pairs_mut().append_pair("model", model).append_pair("nonce", nonce);
    Ok(url)
}

/// The gateway's ed25519 signing key, if the report binds it to `expected_nonce`.
pub fn gateway_ed25519_key(report_json: &str, expected_nonce: &str) -> Result<String, AttestationReportError> {
    let document: serde_json::Value = serde_json::from_str(report_json).map_err(|_| AttestationReportError::Malformed)?;
    let gateway = document.get("gateway_attestation").ok_or(AttestationReportError::Malformed)?;
    let field = |name: &str| gateway.get(name).and_then(serde_json::Value::as_str).ok_or(AttestationReportError::Malformed);

    let algo = field("signing_algo")?;
    if ReceiptAlgo::from_wire(algo) != Some(ReceiptAlgo::Ed25519) {
        return Err(AttestationReportError::NotEd25519);
    }
    let key = field("signing_address")?.to_ascii_lowercase();
    let nonce = field("request_nonce")?.to_ascii_lowercase();
    let report_data = field("report_data")?.to_ascii_lowercase();

    if nonce != expected_nonce.to_ascii_lowercase() {
        return Err(AttestationReportError::NonceMismatch);
    }
    if report_data != format!("{key}{nonce}") {
        return Err(AttestationReportError::ReportDataMismatch);
    }
    Ok(key)
}
```

Declare it in `routing/mod.rs`: `pub mod attestation_report;`.

- [ ] **Step 4: Run the parser tests**

```bash
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib routing::attestation_report
```

Expected: all five pass.

- [ ] **Step 5: The comparison, as a pure function, plus the config gate**

`receipt_for_attested_call` refuses any non-https endpoint before making a request, so a loopback stub cannot reach it — the network path is exercised only against the real endpoint. The decision therefore lives in a pure function, tested directly, and the network wiring stays as small as the existing fetch.

Add to `crates/trace-commons-contributor/src/routing/attestation_report.rs`:

```rust
/// Whether a verified receipt's signer is the gateway key a report attested.
///
/// Case-insensitive, because the report renders the key lowercase and a
/// receipt might not. Exact otherwise: a prefix, a suffix, or a different
/// scheme's address does not match.
pub fn signer_is_attested(receipt_signer: &str, attested_key: &str) -> bool {
    !attested_key.is_empty() && receipt_signer.eq_ignore_ascii_case(attested_key)
}
```

with these tests in the same module:

```rust
    #[test]
    fn the_signer_matches_the_attested_key_case_insensitively() {
        assert!(signer_is_attested(&KEY.to_ascii_uppercase(), KEY));
        assert!(signer_is_attested(KEY, KEY));
    }

    #[test]
    fn a_different_signer_does_not_match() {
        assert!(!signer_is_attested("0x614bc66ff0407dbb70b9c7ca1f5e983e4a02c921", KEY));
        assert!(!signer_is_attested(&KEY[..62], KEY), "a prefix is not a match");
        assert!(!signer_is_attested("", KEY));
        assert!(!signer_is_attested(KEY, ""), "an empty attested key matches nothing");
    }
```

In `config.rs`, beside `inference_receipt_endpoint`:

```rust
    /// Also fetch a nonced attestation report and refuse a receipt whose
    /// signer is not the gateway key bound in it. Off by default: it costs a
    /// second network call per submission, and it reads the report's
    /// self-description without verifying the quote -- so what it adds is
    /// consistency with NEAR AI's claim, not proof of it. Named so an
    /// operator turning it on reads that limit first.
    #[serde(default)]
    pub inference_receipt_check_attestation: bool,
```

Add `inference_receipt_check_attestation: false` to every `ContributorConfig {` literal the compiler reports — the same sites the endpoint field touched, all mechanical. **Then build the GTK workspace**: it has one such literal in `src/ui/settings.rs`, and `cargo --workspace` will not tell you.

Add one test to `config.rs`'s test module:

```rust
    /// A config written before this field existed reads as off. The
    /// check must never switch itself on for an existing install.
    #[test]
    fn the_attestation_check_is_off_for_a_config_that_predates_it() {
        let json = r#"{"schema_version":"1","issuer_url":"https://i","ingest_url":"https://g","audience":"a","tenant_id":"t","instance_id":"i","user_subject":"s","device_key_id":"d","consent_scopes":[]}"#;
        let cfg: ContributorConfig = serde_json::from_str(json).unwrap();
        assert!(!cfg.inference_receipt_check_attestation);
    }
```

Adjust the JSON to whatever minimal set of required fields `ContributorConfig` actually has — copy from an existing deserialisation test in that module rather than guessing.

- [ ] **Step 6: Wire it into the fetch**

In `crates/trace-commons-contributor/src/routing/receipt.rs`, give `receipt_for_attested_call` one more parameter, `check_attestation: bool`, and add a variant to `ReceiptFetchError`:

```rust
    /// The receipt verified, but its signer is not the gateway key a
    /// nonced attestation report bound. Carries nothing.
    #[error("receipt signer is not the attested gateway key")]
    SignerNotAttested,
```

with refusal label `receipt_signer_not_attested` wherever that enum maps to labels.

After the receipt is fetched and parsed, and only when `check_attestation` is true:

```rust
    if check_attestation {
        let nonce = fresh_nonce_hex(); // 32 random bytes, lowercase hex, via ring::rand::SystemRandom as identity.rs does
        let url = crate::routing::attestation_report::attestation_report_url(endpoint, model, &nonce)
            .map_err(|_| ReceiptFetchError::SignerNotAttested)?;
        let body = fetch_text(&url).await?;   // the same client, timeout and no-retry policy the receipt fetch uses
        let attested = crate::routing::attestation_report::gateway_ed25519_key(&body, &nonce)
            .map_err(|_| ReceiptFetchError::SignerNotAttested)?;
        if !crate::routing::attestation_report::signer_is_attested(&payload.signing_address, &attested) {
            return Err(ReceiptFetchError::SignerNotAttested);
        }
    }
```

`fetch_text` is whatever the existing receipt fetch already uses to GET a URL and read the body; reuse it, do not add a second client. Update the one caller in `submit.rs` to pass `self.effective_cfg.inference_receipt_check_attestation`. Never log the nonce, the key, or the report.

The existing refusal tests (`an_unconfigured_endpoint_fetches_nothing`, `an_endpoint_outside_the_allowlist_is_refused`, `a_plaintext_endpoint_is_refused_even_when_allowlisted`) must still pass unchanged with `false` passed through — they are the regression net for this wiring.

- [ ] **Step 7: Verify everything and commit**

```bash
cargo fmt --all
RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor --lib
cargo clippy --workspace --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
cargo test -p trace-commons-server --test license_boundary
for c in trace-commons-protocol trace-commons-attestation trace-commons-contributor trace-commons-contributor-ffi; do RUSTFLAGS='-D warnings' cargo check -p $c --no-default-features; done
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add -A
git commit -m "Optionally check the receipt signer against a nonced attestation report

Off by default. It costs a second network call per submission and reads
the report's self-description without verifying the quote, so what it
adds is consistency with NEAR AI's claim rather than proof of it. The
config field's doc says so, so an operator turning it on reads the limit
first.

The report's report_data is signing_address concatenated with the nonce
this client chose, and the parser refuses a report for any other nonce
or whose report_data does not commit to the listed key. That is what
stops an old report's key being replayed."
```
