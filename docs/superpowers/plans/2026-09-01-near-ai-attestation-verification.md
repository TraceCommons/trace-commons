# NEAR AI attestation verification — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the repo's unverified claim that NEAR AI inference runs in an Intel TDX enclave into a check that fails when it is untrue.

**Architecture:** A verifier module in `trace-commons-server` fetches NEAR AI's attestation report with a nonce we generate, verifies the Intel TDX quote against Intel collateral with `dcap-qvl`, confirms our nonce is in the quote's report data, and pins the image measurements against operator-configured expected values. A second half performs a minimal completion, fetches its receipt from `/v1/signature/{chat_id}`, and verifies the ECDSA signature over the request and response hashes, tying the signer to the attested key. Both are driven by an admin drill that emits hash-only evidence, following the existing `/v1/admin/*-drill` pattern.

**Tech Stack:** Rust, `dcap-qvl` (TDX quote verification), `k256` (secp256k1 recovery, already a dependency), `sha3` (keccak256, added in `23364e49`), `sha2`, `reqwest`, `hex`.

**Spec:** [`docs/superpowers/specs/2026-09-01-near-ai-attestation-design.md`](../specs/2026-09-01-near-ai-attestation-design.md) — read the "What exists today" and "The attestation object" sections before starting. They were rewritten on 2026-09-01 from live probes and are accurate; earlier drafts were not.

## Scope

**In scope:** verifying *our own* NEAR AI calls — the path the pilot's scorer uses.

**Out of scope, deliberately:**
- Contributor trace attestation. That needs client-side capture and PR #513's `routing_metadata` presence category. This plan builds the verifier it will reuse; it does not touch envelopes.
- NVIDIA GPU evidence (`nvidia_payload` / NRAS). The Intel TDX quote is the larger half and is verifiable without a third-party attestation service in the path. A follow-on adds the GPU half.
- Changing `PerplexityScorer` or the scoring hot path. The drill makes its own completion. Production-path receipt capture is a later slice, and the trait seam is why: a local GPU scorer has no receipt, so receipts do not belong on that trait.
- Gating anything. Nothing refuses a submission based on these results in this plan. The drill reports; policy comes later.

## Global Constraints

- **Hash-only evidence.** Stored rows and log strings carry hashes and labels only — never the API key, the raw report, the signature, the completion text, or a signing address in a log line. Copy the hash-only discipline from the existing drills.
- **Fail closed.** A configured check whose dependency is missing refuses with a named missing-control, never a silent pass. In particular: no expected-measurement set configured means the measurement check **refuses**, not skips.
- **License:** `trace-commons-server` is AGPL-3.0-or-later. Every new `.rs` file needs the two-line copyright + SPDX header. Do not add a dependency from a permissive crate onto an AGPL one.
- **No emojis** in code, commits, or reports. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- **Verify with** `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`. Plain `cargo check` does not catch what CI catches. Run `cargo fmt --all` and the repo clippy line before committing.
- **The rustfmt post-edit hook rewrites whole files.** Check `git show --stat` after every commit and confirm it contains only what you meant.
- **Network in tests.** No test may require the live NEAR AI service. Every test in this plan runs against the checked-in fixture. One `#[ignore]`d integration test may hit the network; it must not run by default.
- **Attestation material never reaches scoring or PII scrubbing.** Quotes, receipts, signatures and signing addresses must never enter an event's `redacted_content`, and never reach the perplexity scorer, the novelty/dedup path, or the privacy filter. This is not a preference; see Task 6 for why it would silently corrupt both.

## File structure

- **Create** `crates/trace-commons-server/src/near_attestation/mod.rs` — report types, parsing, the nonce and measurement checks. No network.
- **Create** `crates/trace-commons-server/src/near_attestation/quote.rs` — the `dcap-qvl` bridge: verify a quote, return report data and measurements.
- **Create** `crates/trace-commons-server/src/near_attestation/receipt.rs` — receipt fetch and EIP-191 signature verification.
- **Create** `crates/trace-commons-server/src/near_attestation/client.rs` — the HTTP calls, behind a trait so tests use the fixture.
- **Modify** `crates/trace-commons-server/src/lib.rs` — declare the module.
- **Modify** `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — the drill route.
- **Exists already** `crates/trace-commons-server/tests/fixtures/near_ai_attestation_report.json` — a real trimmed report captured 2026-09-01. Its `_fixture_nonce` is the nonce bound into `intel_quote`. Do not regenerate it; do not hand-edit the quote.

---

### Task 1: Parse the report and check the nonce binding

**Files:**
- Create: `crates/trace-commons-server/src/near_attestation/mod.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`

**Interfaces:**
- Produces: `AttestationReport` (deserialized), `AttestationReport::from_json(&str) -> Result<Self>`, `AttestationReport::quote_bytes(&self) -> Result<Vec<u8>>`, `AttestationReport::measurements(&self) -> Measurements`, and `Measurements { mrtd, rtmr0, rtmr1, rtmr2, rtmr3, compose_hash, os_image_hash, mr_aggregated }` — all `String`.

- [ ] **Step 1: Write the failing tests**

Put them in the module's own `#[cfg(test)] mod tests`. Load the fixture with `include_str!("../../tests/fixtures/near_ai_attestation_report.json")`.

```rust
const FIXTURE: &str = include_str!("../../tests/fixtures/near_ai_attestation_report.json");

#[test]
fn parses_a_real_report() {
    let r = AttestationReport::from_json(FIXTURE).expect("fixture parses");
    assert_eq!(r.signing_algo, "ecdsa");
    assert!(r.signing_address.starts_with("0x"));
    assert!(!r.intel_quote.is_empty());
}

#[test]
fn quote_bytes_decode_from_hex() {
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let q = r.quote_bytes().expect("quote decodes");
    // The fixture's quote is 10,012 hex chars.
    assert_eq!(q.len(), 10_012 / 2);
}

#[test]
fn the_fixtures_nonce_is_bound_into_the_quote() {
    // This is the property that makes the report fresh rather than replayable:
    // the nonce we asked for is inside the signed quote, not merely echoed
    // beside it. If this ever passes for a nonce we did not send, the check
    // that matters is gone.
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let nonce = fixture_nonce();
    assert!(r.quote_binds_nonce(&nonce).unwrap());
}

#[test]
fn a_nonce_we_did_not_send_is_not_bound() {
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let other = "0".repeat(64);
    assert!(!r.quote_binds_nonce(&other).unwrap());
}

#[test]
fn an_echoed_nonce_alone_does_not_satisfy_the_binding() {
    // Defends the exact confusion this check exists to prevent: request_nonce
    // is JSON beside the quote and proves nothing. Rewrite the echo to a value
    // that is NOT in the quote and the binding must still fail.
    let mut v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    let forged = "a".repeat(64);
    v["request_nonce"] = serde_json::json!(forged);
    let r = AttestationReport::from_json(&v.to_string()).unwrap();
    assert!(!r.quote_binds_nonce(&forged).unwrap());
}

fn fixture_nonce() -> String {
    let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
    v["_fixture_nonce"].as_str().unwrap().to_string()
}
```

- [ ] **Step 2: Run to verify they fail**

```bash
cargo test -p trace-commons-server --lib near_attestation
```
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement**

`AttestationReport` derives `Deserialize` with `#[serde(deny_unknown_fields)]` **off** — the real service sends fields we trim and fields we do not model, and rejecting them would break on any upstream addition. Model only what is used: `model_name`, `signing_address`, `signing_algo`, `signing_public_key`, `request_nonce`, `intel_quote`, and `info` (with `compose_hash`, `os_image_hash`, `mr_aggregated`, `instance_id`, `app_id`, and `tcb_info`).

`quote_binds_nonce(&self, nonce: &str) -> Result<bool>` decodes the quote from hex and searches the raw bytes for the nonce decoded from hex. Search the **bytes**, not the hex string: a hex-substring match would also be satisfied by an unlucky alignment, and the fixture makes both pass so only the byte form is honest. Reject a nonce that is not exactly 64 hex characters with an error rather than returning `false` — a malformed nonce is a caller bug, and returning `false` would let a caller conclude "not attested" when the truth is "you asked wrong".

- [ ] **Step 4: Run the tests**

```bash
cargo test -p trace-commons-server --lib near_attestation
```
Expected: PASS, five tests.

- [ ] **Step 5: Commit**

```bash
cargo fmt --all
git add crates/trace-commons-server/src/near_attestation/ crates/trace-commons-server/src/lib.rs crates/trace-commons-server/tests/fixtures/
git commit -m "Read NEAR AI's attestation report and check the nonce is in the quote

The report echoes request_nonce as JSON beside the quote, which proves
nothing on its own. What makes it fresh rather than replayable is that
the nonce is inside the signed quote, so that is what this checks, and a
test forges the echo to prove the echo is not what satisfies it."
```

---

### Task 2: Verify the TDX quote

**Files:**
- Create: `crates/trace-commons-server/src/near_attestation/quote.rs`
- Modify: `crates/trace-commons-server/Cargo.toml` — add `dcap-qvl`

**Interfaces:**
- Consumes: `AttestationReport::quote_bytes` (Task 1)
- Produces: `verify_quote(quote: &[u8], collateral: &Collateral, now_unix: u64) -> Result<VerifiedQuote>` and `VerifiedQuote { report_data: Vec<u8>, mrtd: String, rtmr: [String; 4], tcb_status: String }`

**Dependency note:** `dcap-qvl` is pre-approved (see `~/.claude/approved-dependencies.md`). Add it as `dcap-qvl = { version = "0.6", default-features = false, features = ["ring", "std"] }` — `ring` because the crate is already a dependency of this crate, and **not** the default feature set, which pulls the async PCCS client and a second HTTP stack. Verify what the lockfile actually gains and report it; if it adds more than a handful of crates or a second `reqwest`/`tokio` tree, stop and report rather than proceeding.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_real_quote_verifies_and_exposes_its_report_data() {
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let q = r.quote_bytes().unwrap();
    let collateral = fixture_collateral();
    let v = verify_quote(&q, &collateral, FIXTURE_CAPTURED_AT).expect("real quote verifies");
    // report_data[32..64] is the nonce, per NEAR AI's verifier README.
    assert_eq!(hex::encode(&v.report_data[32..64]), fixture_nonce());
}

#[test]
fn a_tampered_quote_does_not_verify() {
    // The single most important test here. If a mutated quote still verifies,
    // this module is decoration.
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let mut q = r.quote_bytes().unwrap();
    let last = q.len() - 1;
    q[last] ^= 0xff;
    assert!(verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT).is_err());
}

#[test]
fn a_truncated_quote_does_not_verify() {
    let r = AttestationReport::from_json(FIXTURE).unwrap();
    let q = r.quote_bytes().unwrap();
    assert!(verify_quote(&q[..q.len() / 2], &fixture_collateral(), FIXTURE_CAPTURED_AT).is_err());
}
```

- [ ] **Step 2: Obtain the collateral for the fixture**

Verification is offline but needs collateral matching the quote. Fetch it once and check it in beside the report as `near_ai_attestation_collateral.json`, with a note recording where it came from and when.

```bash
# Use dcap-qvl's own collateral client, or Intel PCS, whichever the crate
# documents for this quote's FMSPC. Record the exact command in the fixture note.
```

If the collateral cannot be obtained offline-reproducibly — for example it expires and the test would start failing on a date rather than on a code change — **stop and report that** rather than checking in something that will rot. A time-bombed test is worse than no test. `FIXTURE_CAPTURED_AT` exists to pin `now` so expiry does not make the suite fail with the calendar.

- [ ] **Step 3: Implement**

Thin wrapper over `dcap_qvl::verify::verify`. Map its error into a named error type; do not `anyhow!("{e}")` the underlying text into a log, since collateral errors can carry URLs.

- [ ] **Step 4: Run**

```bash
cargo test -p trace-commons-server --lib near_attestation::quote
```
Expected: PASS, three tests. **Confirm the tamper test fails when you invert the tamper** — comment out the XOR and check the test fails. A tamper test that passes on unmodified input proves nothing.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "Verify the TDX quote rather than trusting the endpoint that served it

Checks the quote against Intel collateral and reads the nonce out of
report_data, so freshness rests on the hardware signature rather than on
NEAR AI's word. A tampered and a truncated quote are both required to
fail, because a verifier that accepts them is decoration."
```

---

### Task 3: Pin the measurements

**Files:**
- Modify: `crates/trace-commons-server/src/near_attestation/mod.rs`

**Interfaces:**
- Consumes: `Measurements` (Task 1), `VerifiedQuote` (Task 2)
- Produces: `ExpectedMeasurements::from_env() -> Result<Option<Self>>`, `check_measurements(expected: &ExpectedMeasurements, actual: &Measurements) -> MeasurementVerdict`

Config key: `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`, a comma-separated list of `key=value` pairs over `mrtd`, `rtmr0`..`rtmr3`, `compose_hash`, `os_image_hash`, `mr_aggregated`. Only the keys named are checked.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn matching_measurements_pass() { /* expected built from the fixture's own values */ }

#[test]
fn one_changed_measurement_fails_and_names_which() {
    // The verdict must name the field. "Attestation failed" sends an operator
    // to the wrong place; "mr_aggregated differs" sends them to the image.
    let verdict = check_measurements(&expected, &actual_with_changed_mr_aggregated);
    assert!(matches!(verdict, MeasurementVerdict::Mismatch { ref field, .. } if field == "mr_aggregated"));
}

#[test]
fn an_absent_expected_set_refuses_rather_than_passing() {
    // Fail closed. An operator who has not pinned anything must get a refusal
    // with a named missing control, never a green tick that means nothing.
    assert!(matches!(ExpectedMeasurements::from_env_value(None), Ok(None)));
    assert!(matches!(
        check_measurements_opt(None, &actual),
        MeasurementVerdict::Refused { ref control, .. } if control == "near_ai_expected_measurements"
    ));
}

#[test]
fn an_expected_set_naming_an_unknown_field_is_a_config_error() {
    // Otherwise a typo silently narrows the check: `mrtdd=...` would be
    // ignored and the operator would believe mrtd was pinned.
    assert!(ExpectedMeasurements::from_env_value(Some("mrtdd=abc")).is_err());
}

#[test]
fn comparison_is_case_insensitive_on_hex_but_not_lenient_on_length() { /* ... */ }
```

- [ ] **Step 2-4: Run, implement, run**

```bash
cargo test -p trace-commons-server --lib near_attestation
```

The typo test is the one that matters most: a pinning mechanism that silently ignores a misspelled key gives an operator false confidence, which is worse than no pinning.

- [ ] **Step 5: Commit**

```bash
git commit -m "Pin the image measurements, and refuse when nothing is pinned"
```

---

### Task 4: Verify an inference receipt

**Files:**
- Create: `crates/trace-commons-server/src/near_attestation/receipt.rs`

**Interfaces:**
- Produces: `verify_receipt(payload: &ReceiptPayload, request_body: &[u8], response_text: &str) -> Result<ReceiptVerdict>` where `ReceiptPayload { text: String, signature: String, signing_address: String }`

The mechanism, from NEAR AI's reference verifier (`nearai/nearai-cloud-verifier`, `py/chat_verifier.py`):

1. `text` splits on `:` into two or three parts. Three: hashes are `parts[1]`, `parts[2]`. Two: `parts[0]`, `parts[1]`. Any other count is an error.
2. Both are lowercase SHA-256 hex — of the request body **as sent**, and of the response text.
3. `signature` is an EIP-191 `personal_sign` over `text`: `keccak256("\x19Ethereum Signed Message:\n" + len(text) + text)`, then secp256k1 public-key recovery, then the Ethereum address is the last 20 bytes of `keccak256(pubkey[1..])`.
4. Compare the recovered address to `signing_address`, case-insensitively.

- [ ] **Step 1: Write the failing tests**

Build a fixture by signing a known `text` with a known key using `k256`, so the test is self-contained and needs no live receipt.

```rust
#[test]
fn a_valid_receipt_verifies_and_binds_both_hashes() { /* ... */ }

#[test]
fn a_receipt_whose_request_hash_does_not_match_is_rejected() {
    // This is what stops a receipt being moved onto a different trace.
}

#[test]
fn a_receipt_whose_response_hash_does_not_match_is_rejected() {}

#[test]
fn a_signature_by_a_different_key_is_rejected() {}

#[test]
fn the_three_part_form_reads_the_hashes_from_the_right_positions() {
    // Guards a real off-by-one: with a leading part the hashes shift, and
    // reading parts[0..2] would compare the prefix against a hash and still
    // "work" for the two-part case, so only this test catches it.
}

#[test]
fn a_text_with_one_or_four_parts_is_an_error_not_a_pass() {}
```

- [ ] **Step 2-4: Run, implement, run**

Note for the implementer: `k256`'s recovery needs the `RecoveryId` from the signature's 65th byte, which for Ethereum is `v` and may be 27/28 or 0/1. Handle both, and test both — a receipt that fails only against one provider's `v` encoding is a bug that will not appear until production.

- [ ] **Step 5: Commit**

```bash
git commit -m "Verify a NEAR AI inference receipt against the request and response"
```

---

### Task 5: The drill, and its evidence

**Files:**
- Create: `crates/trace-commons-server/src/near_attestation/client.rs`
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
- Modify: `docs/operator/README.md`; create `docs/operator/near-attestation-drill.md`

**Interfaces:**
- Consumes: everything above.
- Produces: `POST /v1/admin/near-attestation-drill`

Find an existing `/v1/admin/*-drill` handler and copy its shape — auth, evidence structure, error codes. Do not invent a new pattern.

The drill:
1. Generates a fresh 32-byte nonce.
2. Fetches `/v1/attestation/report?model=&nonce=&signing_algo=ecdsa`.
3. Verifies the quote (Task 2) and that report data carries the nonce (Task 1).
4. Checks measurements (Task 3), refusing if nothing is pinned.
5. Performs one minimal completion, captures `chat_id` and the exact request bytes and response text.
6. Fetches `/v1/signature/{chat_id}` and verifies the receipt (Task 4).
7. Confirms the receipt's `signing_address` is the one in the verified attestation.

**Evidence is hash-only:** the nonce, the measurement verdict per field, the tcb status, a hash of the signing address, and pass/fail per step. Never the API key, the completion text, the receipt, or the raw report.

**Step 5 costs money.** Keep the completion minimal — a handful of tokens, `max_tokens` small — and say so in the runbook. If `TRACE_COMMONS_NEAR_AI_API_KEY` is absent the drill must refuse with a named missing control, not skip to a pass.

- [ ] **Step 1: Write the failing tests**

Drive the handler directly with a stub `AttestationClient` (the trait in `client.rs`) returning the fixture. Follow the direct-handler-call idiom already used in this binary's tests; there is no HTTP-level harness.

```rust
#[tokio::test]
async fn the_drill_refuses_when_no_measurements_are_pinned() {}

#[tokio::test]
async fn the_drill_fails_when_the_report_does_not_carry_our_nonce() {
    // The replay case: a stub returning a report bound to some other nonce.
}

#[tokio::test]
async fn the_drill_fails_when_the_receipt_signer_is_not_the_attested_key() {
    // The substitution case: valid attestation, valid receipt, different key.
    // Without this check the two halves verify independently and prove nothing
    // together.
}

#[tokio::test]
async fn drill_evidence_carries_no_secret() {
    let text = serde_json::to_string(&evidence).unwrap();
    for forbidden in ["sk-", "Bearer", "0x"] {
        assert!(!text.contains(forbidden), "evidence leaked {forbidden}");
    }
}
```

- [ ] **Step 2-4: Run, implement, run**

- [ ] **Step 5: Wire the drill into the rollout-smoke evidence path**, as `CLAUDE.md` requires of every drill. Find how an existing drill is wired and follow it.

- [ ] **Step 6: Write the runbook** — what the drill proves, what it costs, what each failure means, and specifically: a measurement mismatch after a NEAR AI image upgrade is expected and is fixed by re-pinning after review, **not** by disabling the check. Say that plainly, because a deployment that responds to a red drill by turning it off is worse off than one that never had it.

- [ ] **Step 7: Commit**

```bash
git commit -m "Prove the inference endpoint is the enclave we think it is"
```

---

### Task 6: Lock the scoring invariant before there is data to break it

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/chunker.rs` — tests only

**Why this task exists.** Verified at `crates/trace-commons-gate-enclave/src/chunker.rs:85-98`: the chunker iterates **every** event, reads `event_type` and `redacted_content`, and renders `"{event_type}: {content}\n"` into the text that both the perplexity scorer and the novelty/dedup signal consume. There is **no filter by event type**. So the moment attestation material lands in `redacted_content` — on any event, of any type — it is scored.

That would be silently destructive in three ways, and none of them would look like a bug:

1. **Perplexity.** A TDX quote is ~5 KB of high-entropy hex. It has no natural-language structure, so it scores as extremely surprising and would drag a trace's perplexity wherever the aggregation takes it. The signal would move for a reason that has nothing to do with the contributor's work.
2. **Novelty and dedup.** Every trace from the same enclave carries near-identical attestation bytes. That is a large block of *identical* text across unrelated traces — exactly the shape the duplicate-cluster penalty is built to punish. Honest contributors would be penalised for the provenance evidence proving their work was real.
3. **The privacy filter.** Attestation blobs contain no PII, so scrubbing them is pure waste — and it is waste in the scarcest place. The classify budget is per-request tokens signalled as a generic 502, and a multi-kilobyte opaque blob is the failure mode that already wedges the backstop queue.

This plan puts no attestation data in an envelope, so none of this can happen yet. The point is to fix the invariant while it is cheap, so the slice that adds receipts inherits a guard rather than discovering this in production scores.

**Interfaces:** none — tests only.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn only_redacted_content_reaches_the_scored_text() {
    // The chunker takes every event with no type filter, so the ONLY thing
    // keeping non-content fields out of perplexity and dedup is that it reads
    // `redacted_content` and nothing else. Attestation material will live in a
    // sibling field; this asserts that adding one changes no scored byte.
    let plain = envelope_json_with_extra_fields(&[], &[]);
    let with_extra = envelope_json_with_extra_fields(
        &[("attestation_receipt", "0xdeadbeef...")],
        &[("intel_quote", "aabbcc...")],
    );
    assert_eq!(
        render_all_events(&plain),
        render_all_events(&with_extra),
        "a non-content field changed the scored text; attestation data would be scored"
    );
}
```

Build both envelopes with identical `event_type` and `redacted_content`, differing only in extra sibling keys — one on the event, one on the envelope.

- [ ] **Step 2: Run it**

```bash
cargo test -p trace-commons-gate-enclave --lib chunker
```

Expect PASS on today's code — the chunker already reads only `redacted_content`. **That is the point: this is a regression guard, not a bug fix.** Prove it can fail before trusting it: temporarily make the renderer append any extra event field, confirm the test fails, then revert. Report that you did this; a guard nobody has seen fail is a guard nobody should rely on.

- [ ] **Step 3: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/chunker.rs
git commit -m "Assert only redacted_content reaches the scored text

The chunker renders every event with no filter by type, so nothing but
its choice of field keeps other data out of perplexity and dedup.
Attestation material is about to arrive as a sibling field, and it is
high-entropy and near-identical across traces from one enclave -- it
would read as maximally surprising to the scorer and as a duplicate
block to the dedup pass, penalising honest contributors for carrying
proof their work was real. Locking the property now, while no data
exists to break it."
```

---

## Not in this plan

- Any envelope change, any contributor-facing surface, any gating decision.
- NVIDIA GPU evidence.
- Production scoring-path receipt capture.
- A rotating keyset. NEAR AI publishes none; `?signing_address=` returning 404 on mismatch is the working substitute and is what Task 5 step 7 relies on.
