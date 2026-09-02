# Witness ingest integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a verified redaction-witness certificate keep a submission out of the PII-backstop hold -- and nothing else. Ship it disabled.

**Architecture:** No new binary and no new pipeline. A certificate arrives in two request headers on `POST /v1/traces`, is verified against the raw request body with the existing `verify_witness_certificate`, and is consulted at exactly one call site: `corpus_status_with_pii_backstop_hold` in `trace-commons-ingest.rs`. The witness service is changed to certify the *submitted envelope bytes* rather than a flat transcript string, because the certificate must bind what the server actually holds.

**Tech Stack:** Rust. `axum` (a `Bytes` extractor replacing `Json` on the submit handler), the existing `redaction_witness` module, `trace-commons-protocol` for redaction and residual risk.

**Spec:** [`docs/superpowers/specs/2026-09-02-witness-ingest-integration-design.md`](../specs/2026-09-02-witness-ingest-integration-design.md) -- read "The ceiling, and why the obvious reading of it is wrong", "What a bypass saves", "The binding problem" and "The decision, precisely" before starting.

## The one thing that must not be got wrong

The ceiling in `deploy/witness/README.md` and at
`crates/trace-commons-server/src/redaction_witness/verification.rs:246-283`
says a certificate may at most license skipping the backstop's *classifier*
stage, never the trailing deterministic sweep.

**The spec establishes that this is already satisfied by not entering the
hold**, because `rescrub_trace_envelope` -- the deterministic sweep over
`redacted_content` and `structured_payload`, plus `residual_envelope_scan` --
runs synchronously at `trace-commons-ingest.rs:12911`, *before* the hold is
decided at `:12954`. The only thing the async backstop adds is the classifier.

That ordering is the entire safety argument. **Task 3 pins it with a test, and
that test is the most important one in this plan.** If a future change moves
the hold decision above `rescrub_trace_envelope`, this feature becomes a
wholesale bypass and the test must go red.

Also: **do not put the bypass in the backstop driver.**
`run_pii_backstop_driver_tick` gates the whole tick on a live classifier canary
(`trace-commons-ingest.rs:40645`), so a per-trace skip there is unreachable,
and the driver only ever sees traces that are already held.

## What this will and will not achieve, so no task over-claims it

For a witnessed trace, skipping the hold removes 100% of the backstop cost.
**For this pilot today it drains nothing:** no client emits a certificate, the
witness has never run on a real CVM, the decision is a submit-path decision and
the 248 held traces are already past it, and the driver's canary gate -- not
per-trace classifier work -- is what stops the queue. This slice is a trust
change. No commit message, doc or operator surface may describe it as the fix
for the backlog.

## Global Constraints

- **Ship disabled.** `TRACE_COMMONS_WITNESS_BYPASS_ENABLED` defaults to
  **false**. With it off, every content-bearing trace holds exactly as today
  and an arriving certificate is ignored.
- **Fail closed on configuration.** Bypass enabled with no pinned signing
  address, no measurement set, or an empty policy allowlist is a **boot
  refusal** naming the missing control. Follow
  `crates/trace-commons-server/src/near_attestation/measurements.rs:28-42`:
  `EXPECTED_MEASUREMENTS_ENV`, `EXPECTED_MEASUREMENTS_CONTROL`,
  `expected_measurements_from_env()` whose `Ok(None)` "is *not* an
  acceptance". The witness half's control name already exists:
  `EXPECTED_MEASUREMENT_CONTROL = "witness_expected_measurement"` at
  `verification.rs:60`.
- **Fail open on the submission, closed on the bypass.** A malformed,
  unsigned, unpinned or non-verifying certificate refuses **the bypass** and
  holds the trace as today. It must never reject a submission that would
  otherwise be accepted -- that turns a witness outage into a submission
  outage.
- **Hash-only.** No raw bytes, no redacted bytes, no signature, no signing
  address, no contributor identity in any log line, audit row or error string.
  The one renderable value is a **measurement**, and only after the signature
  has verified -- the precedent and its justification are at
  `verification.rs:195-207`.
- **Never claim clean.** No name, comment, error string, receipt sentence or
  response field may say or imply a witnessed trace is clean. It says a known
  program in a pinned enclave reached a `Low` pass verdict.
- AGPL two-line copyright + SPDX header on every new `.rs` file in
  `trace-commons-server`.
- **Every negative assertion names its specific error variant.** Not
  `assert!(x.is_err())`. Roughly 26 structurally unfalsifiable assertions have
  been found on this project.
- **Mutation-check every guard.** Break it, watch it go red, revert, report the
  actual failure text. A mutation that survives is a finding, not a relief.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace`, **capture
  cargo's own exit code** (a `tail` in a pipe has reported success over a
  failed build on this machine), and confirm the run **terminated** -- a
  failure aborts it and leaves a plausible but truncated pass count. The gtk
  crate is a separate workspace and excluded from `--workspace`; no task here
  touches it.
- **The PostgreSQL suite is broken on `main`** (~104 failures) and CI never
  runs it, so a no-env-var baseline proves nothing about pg paths. **No task in
  this plan adds a migration or a pg-backed test**, deliberately: the bypass
  writes `Accepted` through the existing status path and stores no new column.
  If a task finds it needs one, stop and re-plan rather than adding an
  unverifiable test.
- **Prefix every command with its own explicit `cd`** to the worktree. A green
  exit code from the wrong tree is indistinguishable from a green one from the
  right tree; this has cost time repeatedly on this project.
- No emoji. Short imperative commit subjects, no `feat:`/`fix:` prefixes.

## File structure

- **Modify** `crates/trace-commons-server/src/witness_service/mod.rs` -- certify envelope bytes
- **Create** `crates/trace-commons-server/src/redaction_witness/config.rs` -- the pin and allowlist from env
- **Create** `crates/trace-commons-server/src/redaction_witness/request.rs` -- header decode
- **Modify** `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` -- the submit handler, the hold decision, the receipt
- **Modify** `deploy/pilot-gcp/ingest.env.template`, `docs/operator/pii-backstop.md`, `deploy/witness/README.md`

---

### Task 1: Make the witness certify the bytes the server will hold

**Files:**
- Modify: `crates/trace-commons-server/src/witness_service/mod.rs`

**The problem.** `WitnessResponse::redacted_artifact` is a `String` of redacted
transcript text (`witness_service/mod.rs:124`). The submit handler receives a
`TraceContributionEnvelope`. `verify_witness_certificate` compares the
certificate's digest against bytes the server holds, and the server never holds
that string. There is no mapping between them anywhere in the tree and, because
the envelope splits and canonicalises content, none is constructible.

So the witness must take a serialised envelope and return the serialised
redacted envelope, and the contributor must POST exactly those bytes. Then
`redacted_bytes` is the request body and the binding is trivial.

**What changes.** `witness()` currently runs `DeterministicTraceRedactor` over
its input as flat text. It must instead deserialise the input as a
`TraceContributionEnvelope`, run the envelope redaction path
(`rescrub_trace_envelope_with`, plus the classifier in `full-pipeline` mode via
the same two-stage order the originating pass uses), and serialise the result.
`check_correspondence` stays exactly where it is and keeps its current job:
binding the certificate's digest to the bytes actually being returned.

**Serialisation is part of the contract.** Two serialisations of the same
envelope that differ by one byte produce different digests and the server
refuses. The witness returns the bytes; the contributor forwards them
unmodified; nothing re-encodes in between. State this in the module doc.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn the_certified_artifact_deserialises_as_the_envelope_the_server_will_receive() {
    let raw = serde_json::to_vec(&envelope_with_a_secret()).expect("raw envelope serialises");
    let response = witness(
        WitnessRequest { raw_transcript: String::from_utf8(raw).unwrap(), consent: consent_with_message_text() },
        &TestSigner::default(),
        &TestEnclave::default(),
    )
    .await
    .expect("witness succeeds");

    // Ground truth from outside the code under test: the artifact must parse
    // as an envelope, because the server will parse exactly these bytes.
    let parsed: TraceContributionEnvelope =
        serde_json::from_str(&response.redacted_artifact).expect("artifact is an envelope");
    assert_eq!(parsed.submission_id, envelope_with_a_secret().submission_id);
}

#[tokio::test]
async fn the_certificate_digest_is_over_the_returned_bytes_exactly() {
    let response = witness(request_with_a_secret(), &TestSigner::default(), &TestEnclave::default())
        .await
        .expect("witness succeeds");
    let mut hasher = Sha256::new();
    hasher.update(response.redacted_artifact.as_bytes());
    let digest = hex::encode(hasher.finalize());
    assert_eq!(
        response.certificate.claimed_redacted_sha256(),
        digest,
        "the certificate must cover the exact bytes returned, not a re-encoding",
    );
}

#[tokio::test]
async fn an_input_that_is_not_an_envelope_refuses_by_name() {
    let err = witness(
        WitnessRequest { raw_transcript: "not json".to_string(), consent: ConsentMetadata::default() },
        &TestSigner::default(),
        &TestEnclave::default(),
    )
    .await
    .expect_err("a non-envelope input must refuse");
    assert_eq!(err, WitnessError::MalformedTranscript, "{err}");
}
```

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** -- re-serialise the envelope a second time
  after `check_correspondence` runs (so the digest covers a different byte
  sequence than the one returned) and confirm
  `the_certificate_digest_is_over_the_returned_bytes_exactly` goes red. Report
  the failure text.
- [ ] **Step 6: Commit** -- `Certify the submitted envelope bytes`

---

### Task 2: Load the pin, the allowlist and the switch, fail-closed

**Files:**
- Create: `crates/trace-commons-server/src/redaction_witness/config.rs`
- Modify: `crates/trace-commons-server/src/redaction_witness/mod.rs`

**Interfaces:**
- Produces: `pub struct WitnessBypassConfig { pin: WitnessPin, allowed_policy_versions: BTreeSet<String> }`, `pub const BYPASS_ENABLED_ENV`, `SIGNING_ADDRESS_ENV`, `EXPECTED_MEASUREMENTS_ENV`, `ALLOWED_POLICY_VERSIONS_ENV`, `pub const SIGNING_ADDRESS_CONTROL`, `POLICY_ALLOWLIST_CONTROL`, and `pub fn witness_bypass_config_from_env() -> Result<Option<WitnessBypassConfig>, WitnessBypassConfigError>`

**Follow the NEAR AI precedent exactly** --
`near_attestation/measurements.rs:28-42`. `Ok(None)` means the switch is off,
which is not an acceptance of anything; there is no `Ok(None)` that means
"enabled but unpinned". `WitnessPin::new` already refuses an empty or blank
measurement set and a malformed address, so this module composes rather than
re-validates.

The policy allowlist gets its own control name because it is its own hole. A
`deterministic-only` witness never ran a classifier; admitting its alias means
no classifier ever sees that trace's prose. Nothing in code can distinguish
that from a deliberate operator choice, so the refusal is on *emptiness* and
the danger is documented in Task 6.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_switch_off_yields_no_config_and_no_error() {
    let config = witness_bypass_config_from_values(None, Some(ADDRESS), Some(MEASUREMENT), Some(ALIAS))
        .expect("an absent switch is not an error");
    assert!(config.is_none(), "the bypass must be off by default");
}

#[test]
fn enabled_without_a_signing_address_refuses_by_control_name() {
    let err = witness_bypass_config_from_values(Some("true"), None, Some(MEASUREMENT), Some(ALIAS))
        .expect_err("an enabled bypass with no address must refuse");
    assert_eq!(
        err,
        WitnessBypassConfigError::MissingControl { control: SIGNING_ADDRESS_CONTROL },
        "{err}",
    );
}

#[test]
fn enabled_without_measurements_refuses_under_the_existing_control_name() {
    let err = witness_bypass_config_from_values(Some("true"), Some(ADDRESS), None, Some(ALIAS))
        .expect_err("an enabled bypass with no measurements must refuse");
    assert_eq!(
        err,
        WitnessBypassConfigError::MissingControl { control: EXPECTED_MEASUREMENT_CONTROL },
        "{err}",
    );
    assert_eq!(EXPECTED_MEASUREMENT_CONTROL, "witness_expected_measurement");
}

#[test]
fn enabled_with_an_empty_policy_allowlist_refuses_by_control_name() {
    let err = witness_bypass_config_from_values(Some("true"), Some(ADDRESS), Some(MEASUREMENT), Some("  ,  "))
        .expect_err("a blank allowlist is not an allowlist");
    assert_eq!(
        err,
        WitnessBypassConfigError::MissingControl { control: POLICY_ALLOWLIST_CONTROL },
        "{err}",
    );
}

#[test]
fn a_malformed_signing_address_surfaces_the_pins_own_variant() {
    let err = witness_bypass_config_from_values(Some("true"), Some("0xnothex"), Some(MEASUREMENT), Some(ALIAS))
        .expect_err("a malformed address must refuse");
    assert_eq!(
        err,
        WitnessBypassConfigError::Pin(WitnessPinError::SigningAddressMalformed),
        "{err}",
    );
}

#[test]
fn the_env_names_are_the_ones_the_operator_doc_states() {
    // The loader cannot be exercised against the real environment under a
    // parallel test runner -- `set_var` is process-wide. Asserting the
    // spelling is the falsifiable half; see the same note at
    // near_attestation/measurements.rs.
    assert_eq!(BYPASS_ENABLED_ENV, "TRACE_COMMONS_WITNESS_BYPASS_ENABLED");
    assert_eq!(SIGNING_ADDRESS_ENV, "TRACE_COMMONS_WITNESS_SIGNING_ADDRESS");
    assert_eq!(EXPECTED_MEASUREMENTS_ENV, "TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS");
    assert_eq!(ALLOWED_POLICY_VERSIONS_ENV, "TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS");
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- make the empty-allowlist branch return
  `Ok(Some(..))` with an empty set and confirm
  `enabled_with_an_empty_policy_allowlist_refuses_by_control_name` goes red.
  Then make the missing-address branch report
  `EXPECTED_MEASUREMENT_CONTROL` and confirm
  `enabled_without_a_signing_address_refuses_by_control_name` goes red -- the
  two controls must not be conflatable, because they send an operator to
  different lines of their config.
- [ ] **Step 6: Commit** -- `Load the witness bypass pin from configuration`

---

### Task 3: Decide the hold, and pin the ordering the safety argument rests on

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
- Modify: `crates/trace-commons-server/src/redaction_witness/verification.rs` (add the `residual_risk_verdict` / `redaction_policy_version` accessors)

**Interfaces:**
- Produces: `fn corpus_status_with_pii_backstop_hold(risk_status, consent, backstop_enabled, witness: Option<&VerifiedWitnessCertificate>, config: Option<&WitnessBypassConfig>) -> TraceCorpusStatus`

The accessors deliberately do not exist yet. `verification.rs:236-244` says
they get added "in the commit that introduces that caller, not before". This is
that commit. Add exactly the two the caller reads; add no others.

**The decision** skips the hold only when all of: the bypass is configured; a
`VerifiedWitnessCertificate` was produced; its verdict is `Low`; its policy
alias is in the allowlist; **and `risk_status` is already `Accepted`.** The last
is what keeps the server's own residual scan authoritative and it is free --
the function already returns `risk_status` unchanged unless it is `Accepted`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn the_synchronous_rescrub_runs_before_the_hold_is_decided() {
    // THE test of this plan. The bypass is only safe because
    // rescrub_trace_envelope -- the deterministic sweep over
    // redacted_content and structured_payload, plus residual_envelope_scan --
    // has already run on these bytes by the time the hold is decided. Read the
    // handler source and assert the order, because no runtime observation
    // distinguishes "ran before" from "ran at all".
    let source = include_str!("../trace-commons-ingest.rs");
    let rescrub = source.find("rescrub_trace_envelope(&mut envelope)").expect("submit rescrubs");
    let hold = source.find("corpus_status_with_pii_backstop_hold(").expect("submit decides the hold");
    assert!(
        rescrub < hold,
        "the hold decision must follow the synchronous rescrub; moving it above turns \
         this feature into a wholesale backstop bypass",
    );
}

#[test]
fn a_witness_emitted_credential_is_caught_before_the_bypass_can_act() {
    // Ground truth from outside: an envelope carrying a credential the prose
    // classifier would have written back. The synchronous pass must raise the
    // risk, so status_for_risk yields Quarantined and the bypass never sees
    // an Accepted status to skip the hold on.
    let mut envelope = envelope_with_event_text("aws key AKIAIOSFODNN7EXAMPLE in output");
    rescrub_trace_envelope(&mut envelope).expect("rescrub runs");
    let risk_status = status_for_risk(envelope.privacy.residual_pii_risk, false);
    assert_eq!(risk_status, TraceCorpusStatus::Quarantined);
    assert_eq!(
        corpus_status_with_pii_backstop_hold(risk_status, &envelope.consent, true, Some(&low_verdict_certificate()), Some(&bypass_config())),
        TraceCorpusStatus::Quarantined,
        "a verified certificate must never lift a quarantine",
    );
}

#[test]
fn a_verified_low_certificate_with_an_allowlisted_alias_skips_the_hold() {
    assert_eq!(
        corpus_status_with_pii_backstop_hold(
            TraceCorpusStatus::Accepted,
            &consent_with_message_text(),
            true,
            Some(&low_verdict_certificate()),
            Some(&bypass_config()),
        ),
        TraceCorpusStatus::Accepted,
    );
}

#[test]
fn a_medium_verdict_still_holds() {
    assert_eq!(
        corpus_status_with_pii_backstop_hold(
            TraceCorpusStatus::Accepted,
            &consent_with_message_text(),
            true,
            Some(&medium_verdict_certificate()),
            Some(&bypass_config()),
        ),
        TraceCorpusStatus::AwaitingPiiBackstop,
    );
}

#[test]
fn a_deterministic_only_policy_alias_is_not_allowlisted_and_still_holds() {
    // The sharpest edge in the design: a deterministic-only witness never ran
    // a classifier, so skipping the server's would mean no classifier ever
    // reads this trace's prose.
    let config = bypass_config_allowing(&["ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1"]);
    assert_eq!(
        corpus_status_with_pii_backstop_hold(
            TraceCorpusStatus::Accepted,
            &consent_with_message_text(),
            true,
            Some(&certificate_with_policy("ironclaw-deterministic-secret-path-v3")),
            Some(&config),
        ),
        TraceCorpusStatus::AwaitingPiiBackstop,
    );
}

#[test]
fn no_configured_bypass_holds_exactly_as_today() {
    assert_eq!(
        corpus_status_with_pii_backstop_hold(
            TraceCorpusStatus::Accepted,
            &consent_with_message_text(),
            true,
            Some(&low_verdict_certificate()),
            None,
        ),
        TraceCorpusStatus::AwaitingPiiBackstop,
    );
}

#[test]
fn a_bypassed_trace_keeps_its_pending_credit() {
    // Group D in the spec. The credit-zeroing branch keys on != Accepted, so a
    // bypassed trace takes the non-zeroing path. Silent failure otherwise.
    let mut envelope = accepted_content_bearing_envelope();
    apply_credit_estimate_to_envelope(&mut envelope);
    let before = envelope.value.credit_points_pending;
    assert!(before > 0.0, "the fixture must have credit to lose");
    let status = corpus_status_with_pii_backstop_hold(
        TraceCorpusStatus::Accepted, &envelope.consent, true,
        Some(&low_verdict_certificate()), Some(&bypass_config()),
    );
    assert_eq!(status, TraceCorpusStatus::Accepted);
    assert_eq!(envelope.value.credit_points_pending, before);
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- drop the `risk_status == Accepted`
  precondition and confirm `a_witness_emitted_credential_is_caught_before_the_bypass_can_act`
  goes red; drop the allowlist check and confirm
  `a_deterministic_only_policy_alias_is_not_allowlisted_and_still_holds` goes
  red; accept `Medium` as well as `Low` and confirm `a_medium_verdict_still_holds`
  goes red. Report all three failure texts. Then move the
  `corpus_status_with_pii_backstop_hold` call above the
  `rescrub_trace_envelope` call and confirm
  `the_synchronous_rescrub_runs_before_the_hold_is_decided` goes red -- if it
  does not, that test is documentation and must be rewritten.
- [ ] **Step 6: Commit** -- `Let a verified certificate skip the backstop hold`

---

### Task 4: Read the certificate off the request without disturbing the bytes

**Files:**
- Create: `crates/trace-commons-server/src/redaction_witness/request.rs`
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

**Interfaces:**
- Produces: `pub const CERTIFICATE_HEADER: &str = "x-trace-commons-witness-certificate"`, `pub const SIGNATURE_HEADER: &str = "x-trace-commons-witness-signature"`, `pub fn witness_headers(headers: &HeaderMap) -> Result<Option<(WitnessCertificate, String)>, WitnessHeaderError>`

**The submit handler must take `Bytes`, not `Json`.** `Json<T>` consumes the
body and the exact bytes are then unrecoverable, and the exact bytes are what
the certificate covers. Digest the body, then deserialise it. The existing
rejection behaviour for malformed JSON must be preserved -- this is the busiest
handler in the binary and a changed error shape there is a client-visible
regression.

**Decode the certificate field by field.** The certificate deliberately has no
`Serialize` impl: a `serde_json/preserve_order` change moved every untyped-JSON
digest in this workspace on 2026-09-01, and the length-prefixed encoder exists
so that cannot happen again. Parse named fields explicitly and rebuild the
signing bytes through that encoder.

**Both headers or neither.** One alone is a refusal of the bypass, never a
silent fallback to unwitnessed.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn neither_header_is_an_ordinary_submission() {
    assert!(witness_headers(&HeaderMap::new()).expect("no headers is not an error").is_none());
}

#[test]
fn a_certificate_without_a_signature_refuses_by_name() {
    let mut headers = HeaderMap::new();
    headers.insert(CERTIFICATE_HEADER, encoded_certificate().parse().unwrap());
    let err = witness_headers(&headers).expect_err("a half-present pair must refuse");
    assert_eq!(err, WitnessHeaderError::SignatureMissing, "{err}");
}

#[test]
fn a_signature_without_a_certificate_refuses_by_name() {
    let mut headers = HeaderMap::new();
    headers.insert(SIGNATURE_HEADER, "0x00".parse().unwrap());
    let err = witness_headers(&headers).expect_err("a half-present pair must refuse");
    assert_eq!(err, WitnessHeaderError::CertificateMissing, "{err}");
}

#[test]
fn a_certificate_that_is_not_base64url_refuses_by_name() {
    let mut headers = HeaderMap::new();
    headers.insert(CERTIFICATE_HEADER, "!!!not base64!!!".parse().unwrap());
    headers.insert(SIGNATURE_HEADER, "0x00".parse().unwrap());
    let err = witness_headers(&headers).expect_err("bad encoding must refuse");
    assert_eq!(err, WitnessHeaderError::CertificateNotBase64, "{err}");
}

#[test]
fn header_decoding_never_renders_content_in_its_error() {
    // Hash-only: a header is attacker-chosen and must not reach a log via a
    // refusal. Assert on both formatters, as verification.rs does.
    let mut headers = HeaderMap::new();
    headers.insert(CERTIFICATE_HEADER, "SECRETMARKER".parse().unwrap());
    headers.insert(SIGNATURE_HEADER, "0xSECRETMARKER".parse().unwrap());
    let err = witness_headers(&headers).expect_err("refuses");
    assert!(!format!("{err}").contains("SECRETMARKER"));
    assert!(!format!("{err:?}").contains("SECRETMARKER"));
}

#[tokio::test]
async fn a_submission_with_an_unverifiable_certificate_is_still_accepted() {
    // Fail open on the submission, closed on the bypass. A witness outage must
    // not become a submission outage.
    let response = submit_through_the_real_router(accepted_content_bearing_body(), garbage_witness_headers()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(stored_status(), TraceCorpusStatus::AwaitingPiiBackstop);
}

#[tokio::test]
async fn the_digest_is_taken_over_the_body_as_received() {
    // A re-serialised envelope has a different digest. Feed a body with
    // non-canonical key order and a matching certificate, and require it to
    // verify -- which it can only do if the handler digested the received
    // bytes rather than a round-trip through serde.
    let body = non_canonically_ordered_envelope_bytes();
    let (cert, sig) = certificate_over(&body);
    let response = submit_through_the_real_router(body, headers_for(&cert, &sig)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(stored_status(), TraceCorpusStatus::Accepted);
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- re-serialise the deserialised envelope and
  digest that instead of the received body; confirm
  `the_digest_is_taken_over_the_body_as_received` goes red. Make a missing
  signature fall through to `Ok(None)`; confirm
  `a_certificate_without_a_signature_refuses_by_name` goes red. Turn the
  unverifiable-certificate path into a `400`; confirm
  `a_submission_with_an_unverifiable_certificate_is_still_accepted` goes red.
- [ ] **Step 6: Commit** -- `Read a witness certificate off the submit request`

---

### Task 5: Tell the contributor which pass admitted their trace

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

Group C in the spec. `receipt_from_record` at `:56738` gives every accepted
trace the same explanation. A trace admitted on a witness certificate was
admitted on a different basis from one admitted on the server's own pass, and a
contributor is entitled to know which -- the same argument
`settlement_posture_explanation` records for #445, where three months of
`pending` credit were indistinguishable from "still working on it".

**The sentence must not claim clean.** It says a pinned enclave's redaction was
accepted in place of the server's queued re-check. Nothing stronger.

The record needs a flag for this. Reuse the existing
`record.residual_risk_basis` label vector rather than adding a column -- the
plan adds no migration, and a hash-safe label is what that field already holds.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_witness_admitted_receipt_says_so() {
    let receipt = receipt_from_record(&witness_admitted_record(), NearSettlementMode::Disabled);
    assert!(
        receipt.explanation.iter().any(|line| line.contains("attested redaction witness")),
        "{:?}", receipt.explanation,
    );
}

#[test]
fn an_ordinarily_accepted_receipt_is_unchanged() {
    let receipt = receipt_from_record(&ordinary_accepted_record(), NearSettlementMode::Disabled);
    assert!(!receipt.explanation.iter().any(|line| line.contains("witness")));
}

#[test]
fn no_receipt_line_claims_the_trace_is_clean() {
    for record in [witness_admitted_record(), ordinary_accepted_record(), held_record()] {
        for line in receipt_from_record(&record, NearSettlementMode::Disabled).explanation {
            let lower = line.to_ascii_lowercase();
            assert!(!lower.contains("verified clean"), "{line}");
            assert!(!lower.contains("no pii"), "{line}");
            assert!(!lower.contains("free of"), "{line}");
        }
    }
}

#[test]
fn no_receipt_line_carries_a_measurement_or_an_address() {
    let receipt = receipt_from_record(&witness_admitted_record(), NearSettlementMode::Disabled);
    for line in receipt.explanation {
        assert!(!line.contains("mrtd:"), "{line}");
        assert!(!line.contains("0x"), "{line}");
    }
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- put the witness sentence on the ordinary
  `Accepted` arm too and confirm `an_ordinarily_accepted_receipt_is_unchanged`
  goes red; put the measurement into the sentence and confirm
  `no_receipt_line_carries_a_measurement_or_an_address` goes red.
- [ ] **Step 6: Commit** -- `Say on the receipt when a witness admitted a trace`

---

### Task 6: Document the posture, and the one way to configure a hole

**Files:**
- Modify: `deploy/pilot-gcp/ingest.env.template`
- Modify: `docs/operator/pii-backstop.md`
- Modify: `deploy/witness/README.md` (the "The server side has no configuration surface yet" section is now wrong)

The env template block follows the shape the settlement block already uses:
commented-out, defaults stated, and a sentence saying what the current posture
means for a contributor.

The operator doc must state four things plainly:

1. **What it changes and what it does not.** A verified certificate keeps a
   trace out of the hold. It changes no risk tier, lifts no quarantine, and
   never means clean.
2. **It will not drain the queue.** No client emits a certificate; the backlog
   is already past the decision point; the driver's canary gates the tick
   before enumeration. An operator who turns this on expecting the backlog to
   move will conclude it is broken.
3. **Only `full-pipeline` aliases belong in the allowlist.** Admitting a
   `deterministic-only` alias means no classifier ever reads that trace's
   prose. The code cannot tell that apart from a deliberate choice.
4. **Pin before you enable.** The switch off with a pin configured is a valid
   staging posture: confirm the measurement matches a real deployment's
   `/v1/attestation`, then flip the switch. Enabling without a pin is a boot
   refusal, by design.

`deploy/witness/README.md` currently says "**There is no environment variable
to set today.**" Replace that section with the four names, and keep its
sentence that verification "arrives with the plan that lets a certificate
affect the PII backstop" only as history.

- [ ] **Step 1: Write the env template block and both doc sections**
- [ ] **Step 2: Verify the four variable names in the docs match the constants
  in `redaction_witness/config.rs` byte for byte.** A doc naming a variable the
  binary does not read is the failure this step exists to prevent -- grep each
  name out of the source and diff the two lists.
- [ ] **Step 3: Run `RUSTFLAGS="-D warnings" cargo test --workspace`, capture
  the exit code, confirm the run terminated**
- [ ] **Step 4: Commit** -- `Document the witness bypass posture`

---

## Not in this plan

- **The client half.** No contributor shell emits a certificate after this
  slice, so the feature is unreachable in production. It needs the permissive
  attestation crate (#540, landed) and a decision about which shell ships it
  first.
- **Draining the held backlog.** 248 traces are already past the decision point
  this plan changes. The drain-rate work is host CPU and the driver's canary
  gate; the disposition of the risk-verdict cohort is a policy decision.
- **The `approved_envelope` path.** It reuses a previously built envelope and
  never re-redacts, so it cannot be witnessed at submit. Witnessing would have
  to happen at build time and travel with the envelope.
- **Witnessing the quarantine re-scrub path** (`:19703`). It has no
  request-borne certificate.
- **Any migration or pg-backed test.** The bypass stores no new column, and the
  pg suite is broken on `main` with CI never running it, so such a test could
  not be verified here.
