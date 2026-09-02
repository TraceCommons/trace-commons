# The redaction witness client Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a contributor verify a witness enclave's measurement, and only
then hand it a raw session -- receiving back a redacted envelope and a
certificate that both the client and the server can verify.

**Architecture:** A new `witness` module in the permissive
`trace-commons-contributor` crate, wired in at `redact_to_envelope`'s two
production call sites. Verification uses `trace-commons-attestation`. The
ordering -- verify, then send -- is enforced by a `VerifiedWitness` type whose
constructor is the verification and whose consumer is the only function that
transmits raw bytes. Three prerequisite changes land first: the certificate's
binding and the witness's wire shape, a byte-body call on the operator client,
and a collateral route on ingest.

**Tech Stack:** Rust. `reqwest` 0.12 and `ring` (already dependencies of the
contributor crate), `trace-commons-attestation` for DCAP verification, the
existing `redaction_witness` certificate types on the server side.

**Spec:** [`docs/superpowers/specs/2026-09-02-witness-client-design.md`](../specs/2026-09-02-witness-client-design.md)
-- read "The one property this design exists to hold", "The three gaps in the
merged service" (especially "Bytes as received, and everything that follows
from it"), "What the certificate is actually buying", "Fail closed, and what
that means at each gate" and "What cannot be verified until a real instance
exists" before starting.

**Also read:** `deploy/witness/README.md`, especially "What to pin, and what not
to" and "What in this document is unverified". It is authoritative over this
plan on pinning policy.

## Why the ordering is the whole task

The contributor sends the enclave a raw, unredacted session. That is the
largest disclosure in this system, and it is acceptable only because the
measurement was verified first. A design that sends and then checks has not
weakened the property; it has removed it, because the bytes are gone before the
check runs.

So this plan does not assert the ordering in a comment. Task 3 builds a
`VerifiedWitness` whose fields are private and whose only constructor performs
the verification, and Task 5 puts the raw-sending function in a module that
cannot construct one. **A test that merely proves a check exists somewhere is
not the test this plan asks for.** The tests here assert that the send is
unreachable without the verification -- by ordering observed at a recording
transport, and by the absence of any other constructor.

## Global Constraints

- **Permissive crates stay permissive.** `trace-commons-contributor`,
  `-contributor-ffi`, `-contributor-gtk`, `-protocol` and `-attestation` are
  `MIT OR Apache-2.0` and must never gain a dependency on
  `trace-commons-server`, `-gate-api` or `-gate-enclave`.
  `crates/trace-commons-server/tests/license_boundary.rs` enforces it. **Never
  edit its expected sets to match a diff** -- those sets are the
  specification. (The existing `cfg(not(windows))` dev-dependency on
  `trace-commons-server` is already pinned there and is not to be widened.)
- **New dependencies need explicit human approval before any code is
  written.** The one this plan needs is
  `trace-commons-attestation` as a path dependency of
  `trace-commons-contributor`, which adds **59 packages** to that crate's tree
  (see the spec's Open questions for the list, the counting method and the
  three consequences). Do not begin Task 3 without it.
- **Ship disabled.** `ContributorConfig.witness` is `Option<_>`,
  `#[serde(default)]`, `None`. Absent means the witness path does not execute
  at all and `redact_to_envelope` runs locally byte for byte as today. No
  discovery, no server-pushed enablement, no default that could move under a
  contributor.
- **Fail closed with a named missing control.** A configured-but-unsatisfiable
  gate refuses the submission -- never a silent fall back to local redaction.
  The label set is fixed in the spec's table; use those exact strings.
- **No re-serialisation between certification and submission.** The certificate
  binds the serialised envelope bytes as the witness emitted them. Nothing on
  the client may deserialise, re-serialise, re-order, pretty-print or append to
  them before they reach `POST /v1/traces`. This is falsified by code that
  exists today -- `Client::call_json` and `Client::call_raw`
  (`operator-client/src/client.rs:68`, `:102`) both do `request.json(body)` --
  so a byte-body call is part of the work, not an assumption.
- **Hash-only, label-only diagnostics.** No raw bytes, no redacted bytes, no
  session content, no byte counts derived from content, no witness URL, no
  signature and no signing address in any log line, error string or refusal
  label. The existing `submit.rs` refusal labels are the pattern.
- **Every negative assertion names its specific error variant.** Not
  `assert!(x.is_err())`. Roughly 26 structurally unfalsifiable assertions have
  been found on this project.
- **Mutation-check every guard.** Break it, watch it go red, revert, and report
  the actual failure text in the commit or the task report. A mutation that
  survives is a finding, not a relief.
- **Verify with `RUSTFLAGS="-D warnings" cargo test --workspace`**, capturing
  cargo's own exit code (a `tail` in a pipe has reported success over a failed
  build on this machine). A protocol-envelope change moves a golden digest
  pinned in the contributor crate, so envelope-touching work is verified
  workspace-wide, never per-crate.
- **`crates/trace-commons-contributor-gtk` is excluded from the workspace.**
  `cargo test --workspace` neither builds nor tests it; it has its own
  workspace and lockfile. Any dependency change to `trace-commons-contributor`
  must be verified there separately with `cargo check` in that directory, and
  its vendored Flatpak set drifts with it.
- **AGPL two-line header on every new `.rs` file in `trace-commons-server`.**
  Files in the contributor crate take no such header.
- **No emoji.** Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- **Prefix every command with its own explicit `cd`** to the worktree. A green
  exit code from the wrong tree is indistinguishable from a green one from the
  right tree, and this has cost time repeatedly on this project.

## What is blocked on a real CVM, and what is not

Tasks 1 through 7 are **not** blocked. Every one is testable against test
doubles, a locally-served `axum` router, and `VerifiedQuote` values constructed
directly (its fields are public). None requires a deployed enclave.

**Task 8 is blocked on a deployment** and is the only one that is. It is where
a measurement is read off a running instance, a pin is published, the size
ceiling on the real path is probed, and the feature becomes enableable by
anyone. Until it runs the client is inert by construction: with no pin
configured it refuses to send, which is the intended state and not a gap.

## File structure

- **Modify** `crates/trace-commons-server/src/witness_service/mod.rs` and `http.rs` -- the certificate binding and the wire shape (Task 1)
- **Modify** `crates/trace-commons-protocol/src/trace_contribution.rs` -- receive `apply_granted_scopes` (Task 1)
- **Modify** `crates/trace-commons-operator-client/src/client.rs` -- a byte-body call (Task 7)
- **Modify** `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` -- the collateral route (Task 2)
- **Create** `crates/trace-commons-contributor/src/witness/mod.rs` -- config, errors, the seam
- **Create** `crates/trace-commons-contributor/src/witness/verify.rs` -- `VerifiedWitness` and its constructor (Task 3)
- **Create** `crates/trace-commons-contributor/src/witness/transport.rs` -- attestation, collateral and the raw send (Tasks 4, 5)
- **Modify** `crates/trace-commons-contributor/src/envelope.rs`, `submit.rs`, `daemon/preview.rs`, `daemon/queue.rs`, `config.rs` (Tasks 6, 7)
- **Create** `docs/operator/witness-client.md` (Task 8)

---

### Task 1: Bind the certificate to the envelope the client will send

**Files:**
- Modify: `crates/trace-commons-server/src/witness_service/mod.rs`
- Modify: `crates/trace-commons-server/src/witness_service/http.rs`
- Modify: `crates/trace-commons-protocol/src/trace_contribution.rs` (receive `apply_granted_scopes`)
- Modify: `crates/trace-commons-contributor/src/envelope.rs` (re-export it from its old home)

**The binding is wrong today.** `witness_service/mod.rs:124` has the
certificate committing to `redacted_artifact: String` -- a redacted transcript.
The server never holds that string; it holds a serialised
`TraceContributionEnvelope`. Nothing the server has can be checked against the
hash the certificate names, so `verify_witness_certificate`'s `redacted_bytes`
has no correct argument. **The certificate must bind the serialised envelope
bytes**, which is the resolution the parallel ingest-integration plan reached
and is adopted here rather than re-argued.

Do not "fix" this by having the server rebuild a digest input from a
deserialised envelope. A serde round trip between the bytes and the hash is the
failure this task exists to remove; moving it from the client to the server is
not removing it.

**Which forces a bigger change than a parameter swap.** The witness stops
redacting text and starts building the envelope. It takes a
`RawTraceContribution` plus the granted scope and use lists, runs
`redact_trace`, applies the grants, serialises **once**, digests those exact
bytes, signs, and returns the bytes verbatim alongside the certificate. Both
types are in the permissive protocol crate, so the AGPL witness may use them.

`apply_granted_scopes` is at `trace-commons-contributor/src/envelope.rs:671`.
An AGPL crate may legally depend on a permissive one, but **do not depend on
`trace-commons-contributor` here**: it would pull `reqwest`, `notify`,
`sysinfo` and `tempfile` into the enclave image, and
`deploy/witness/README.md` names that dependency tree as the largest single
attack-surface reduction available to this deployment. Move the function to
`trace-commons-protocol` and re-export it from its current home so no other
caller moves.

**Why the grants must be in the request rather than stamped afterwards** is in
Task 6; the short version is that `stamp_granted_scopes` is a byte change after
certification.

**Keep the text path.** `TranscriptRedactor` and the existing
`String`-in/`String`-out route stay -- the deployment README's smoke tests and
the existing suite drive them. This adds a second request shape.

**Also preserve the structured pipeline's own rules**, which a text pass would
break: per-event walking, tool-payload profiles by tool name, payload
canonicalization, `redaction_hash`, the trace card, and the S5 rule under which
`outcome.human_correction` is never rewritten and a credential-shaped one is
refused rather than masked.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn the_certificate_covers_the_bytes_the_response_returns() {
    let response = witness_contribution(raw_with_secret(), granted(), &redactor, &signer, &enclave)
        .await
        .expect("the fixture redacts");
    // Ground truth from the returned bytes, not from anything the witness
    // computed internally, and not from a re-serialisation of the parsed
    // envelope -- a test that re-serialises cannot catch a serialisation bug.
    assert_eq!(
        response.certificate.claimed_redacted_sha256(),
        hex::encode(sha2::Sha256::digest(&response.envelope_bytes)),
    );
}

#[tokio::test]
async fn the_returned_bytes_deserialise_to_a_submittable_envelope_carrying_the_grants() {
    let response = witness_contribution(raw_with_secret(), granted(), &redactor, &signer, &enclave)
        .await
        .unwrap();
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).expect("the bytes are an envelope");
    assert_eq!(envelope.consent.granted_scopes, granted().scopes);
    // And the client must not need to stamp anything afterwards: if this is
    // empty the client is forced into a post-certification write.
    assert!(!envelope.consent.granted_scopes.is_empty());
}

#[tokio::test]
async fn a_reserialisation_of_the_envelope_would_break_the_digest() {
    // The guard the whole "bytes as received" rule rests on. If a serde round
    // trip happens to be byte-stable for this fixture the rule is untestable
    // and the fixture is wrong, so this test asserts the round trip DIFFERS
    // and that the certificate follows the original -- pick a fixture with a
    // structured_payload whose key order or float formatting moves.
    let response = witness_contribution(raw_with_reordering_payload(), granted(), &redactor, &signer, &enclave)
        .await
        .unwrap();
    let round_tripped = serde_json::to_vec(
        &serde_json::from_slice::<TraceContributionEnvelope>(&response.envelope_bytes).unwrap(),
    )
    .unwrap();
    assert_ne!(
        round_tripped, response.envelope_bytes,
        "fixture cannot detect a re-serialisation; choose one whose round trip moves bytes"
    );
    assert_eq!(
        response.certificate.claimed_redacted_sha256(),
        hex::encode(sha2::Sha256::digest(&response.envelope_bytes)),
    );
    assert_ne!(
        response.certificate.claimed_redacted_sha256(),
        hex::encode(sha2::Sha256::digest(&round_tripped)),
    );
}

#[tokio::test]
async fn a_correction_is_not_rewritten_by_the_structured_path() {
    // The S5 rule. A text pass over a serialized contribution would scrub it.
    let response = witness_contribution(
        raw_with_correction("the model missed that the retry was idempotent"),
        granted(), &redactor, &signer, &enclave,
    ).await.unwrap();
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).unwrap();
    assert_eq!(
        envelope.outcome.human_correction.as_deref(),
        Some("the model missed that the retry was idempotent")
    );
}

#[tokio::test]
async fn a_credential_shaped_correction_is_refused_by_name() {
    let err = witness_contribution(raw_with_correction(SECRET), granted(), &redactor, &signer, &enclave)
        .await
        .expect_err("a credential in a correction is refused, not masked");
    assert_eq!(err, WitnessError::RedactionFailed);
}

#[tokio::test]
async fn the_verdict_matches_what_the_envelope_carries() {
    let response = witness_contribution(raw_with_secret(), granted(), &redactor, &signer, &enclave)
        .await
        .unwrap();
    let envelope: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).unwrap();
    assert_eq!(
        response.certificate.residual_risk_verdict(),
        envelope.privacy.residual_pii_risk,
        "two sources of truth for the field the design turns on"
    );
}

#[tokio::test]
async fn raw_bytes_never_appear_in_a_response_or_an_error() {
    // A distinctive secret, asserted absent from the serialized response and
    // from every error rendering, on the success path AND on a refusal.
}
```

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement** -- the second request shape (`raw_contribution` and
  the grant lists in place of `raw_transcript`), a `ContributionRedactor` seam
  holding a `DeterministicTraceRedactor`, the single serialisation, and the
  digest over its output. `deny_unknown_fields` stays; a body carrying both
  request shapes is refused.
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** -- serialise a second time for the digest and
  confirm the re-serialisation test goes red; route the correction through the
  text pass and confirm the S5 test does; drop the grant application and confirm
  the submittable-envelope test does. Report all three failure texts.
- [ ] **Step 6: Commit** -- `Bind a witness certificate to the envelope bytes it emits`

---

### Task 2: Serve Intel collateral from ingest

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`

**Why here and not on the witness.** `verify_quote` takes collateral as a
parameter and nothing supplies it to a client. `dcap-qvl`'s own collateral
client cannot go in the contributor crate: its `report` feature pulls a second
`reqwest` with the aws-lc-rs rustls provider alongside this workspace's ring
one, and rustls then **panics at the first TLS use** unless a binary installs a
default explicitly -- the server's manifest records this and `main()` does the
installing. `trace-commons-contributor` is a library inside a CLI, a GTK
binary, a Swift app and a Windows shell; a landmine only a `main()` can defuse
does not belong in it.

Ingest already builds with `near-attestation-collateral`, already installs the
ring provider, and already talks to a PCCS. Putting it on the witness instead
would give the enclave an outbound Intel dependency and enlarge the image, and
therefore the measurement.

**This does not create a trust dependency.** Collateral is Intel-signed and its
validity window is evaluated against the clock the *client* passes, so an
intermediary can withhold it but cannot forge it -- and a client with no
collateral refuses.

**Route:** `POST /v1/attestation-collateral`, body `{"quote_hex": "..."}`,
response the collateral JSON. Unauthenticated and outside tenant context, like
`GET /v1/source`: a contributor needs it *before* they have decided to trust
anything, and requiring a claim to obtain it would make verification depend on
enrollment. Hash-only logging; the quote is public but the route must not log
which client asked for what.

- [ ] **Step 1: Write the failing tests** -- through the real router (a handler
  can be correct and unreachable; that exact defect has been found here):

```rust
#[tokio::test]
async fn the_collateral_route_is_reachable_without_authentication() {
    let response = router().oneshot(collateral_request(FIXTURE_QUOTE_HEX)).await.unwrap();
    assert_ne!(response.status(), StatusCode::UNAUTHORIZED);
    assert_ne!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_malformed_quote_is_refused_by_name_and_echoes_nothing() {
    let response = router().oneshot(collateral_request("nothex")).await.unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(response)["error"], "attestation_collateral_quote_malformed");
}

#[tokio::test]
async fn a_build_without_the_collateral_client_refuses_by_missing_control() {
    // The existing COLLATERAL_CLIENT_CONTROL path. Never a 200 with an empty body.
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- remove the route from the router and confirm
  the reachability test goes red; make the malformed arm return 200 and confirm
  the second does.
- [ ] **Step 6: Commit** -- `Serve Intel collateral so a contributor can verify a quote`

---

### Task 3: `VerifiedWitness`, and the pin that gates it

**Files:**
- Create: `crates/trace-commons-contributor/src/witness/mod.rs`
- Create: `crates/trace-commons-contributor/src/witness/verify.rs`
- Modify: `crates/trace-commons-contributor/Cargo.toml`, `src/lib.rs`

**Blocked on the dependency approval named in Global Constraints.** Do not
start otherwise.

**Interfaces:**
- Produces: `struct VerifiedWitness` (private fields, no public constructor
  beyond the one below), `fn verify_witness(evidence: &AttestationEvidence,
  collateral: &Collateral, nonce: &WitnessNonce, now_unix: u64, pin:
  &WitnessTrust) -> Result<VerifiedWitness, WitnessTrustError>`,
  `struct WitnessTrust` (a signing address and a **list** of
  `ExpectedMeasurements`).

**The checks, in order, each with its own error variant:**

1. `verify_quote(quote, collateral, now_unix)` -- DCAP. `WitnessQuoteUnverified`.
2. Report data equals `b"tcwitns1"` at 0, the pinned address's 20 bytes at 8,
   this client's 32-byte nonce at 28, zeroes after. Reconstructed and compared
   whole, not field by field. Nonce wrong: `WitnessQuoteReplayed`. Address
   wrong: `WitnessSignerUnexpected`.
3. `check_measurements_opt` against each pinned set in turn; any match passes.
   None configured: `MeasurementVerdict::Refused` becomes
   `WitnessMeasurementUnpinned { control: "witness_expected_measurement" }`.
   Configured and no match: `WitnessMeasurementUnpinned` with the reported
   value, which is a public image identifier and is deliberately carried.

**Why a list of sets.** dstack derives the signing key from a stable app id, so
an image upgrade moves the measurement and leaves the signing address. A pin
that held one measurement would break every client on every upgrade;
`ExpectedMeasurements` pins one value per register, so admitting an upgrade
means holding several of them.

**Pin `mrtd` and `mrconfigid`. Not `rtmr3` -- it carries a per-deployment random
instance-id. Not `rtmr0` -- it hashes SMBIOS tables that change on a VM
resize.** `ExpectedMeasurements` will accept either, so the *documentation* of
this config is where that policy lives; see Task 8.

- [ ] **Step 1: Write the failing tests** -- `VerifiedQuote`'s fields are
  public, so these construct one directly and need no real quote:

```rust
fn quote_for(address: &str, nonce: &[u8; 32], mrtd: &str, mrconfigid: &str) -> VerifiedQuote {
    let mut report_data = [0u8; 64];
    report_data[..8].copy_from_slice(b"tcwitns1");
    report_data[8..28].copy_from_slice(&decode_address(address).unwrap());
    report_data[28..60].copy_from_slice(nonce);
    VerifiedQuote {
        report_data: report_data.to_vec(),
        mrtd: mrtd.into(),
        mr_config_id: mrconfigid.into(),
        rtmr: ["00".repeat(48), "00".repeat(48), "00".repeat(48), "ff".repeat(48)],
        tcb_status: "UpToDate".into(),
        advisory_ids: Vec::new(),
    }
}

#[test]
fn a_quote_bound_to_someone_elses_nonce_is_a_replay() {
    let err = check_quote(&quote_for(ADDRESS, &OTHER_NONCE, MRTD, MRCONFIGID), &OUR_NONCE, &trust())
        .expect_err("a quote that does not carry our nonce proves nothing about now");
    assert_eq!(err, WitnessTrustError::WitnessQuoteReplayed);
}

#[test]
fn an_unpinned_client_refuses_and_names_the_missing_control() {
    let err = check_quote(&quote_for(ADDRESS, &OUR_NONCE, MRTD, MRCONFIGID), &OUR_NONCE, &no_pins())
        .expect_err("no pin configured is a refusal, not a pass");
    assert_eq!(
        err,
        WitnessTrustError::WitnessMeasurementUnpinned {
            control: "witness_expected_measurement",
            reported: None,
        }
    );
}

#[test]
fn a_second_pinned_set_admits_an_upgrade_without_admitting_a_stranger() {
    let trust = trust_with([(MRTD, MRCONFIGID), (MRTD, MRCONFIGID_NEXT)]);
    check_quote(&quote_for(ADDRESS, &OUR_NONCE, MRTD, MRCONFIGID_NEXT), &OUR_NONCE, &trust)
        .expect("the new measurement was allowlisted before the rollout");
    let err = check_quote(&quote_for(ADDRESS, &OUR_NONCE, MRTD, MRCONFIGID_STRANGER), &OUR_NONCE, &trust)
        .expect_err("a set that only ever grows stops being a pin");
    assert!(matches!(err, WitnessTrustError::WitnessMeasurementUnpinned { .. }));
}

#[test]
fn rtmr3_drift_does_not_fail_a_pin_on_mrtd_and_mrconfigid() {
    // Two instances of byte-identical code differ in RTMR3. Pinning it would
    // fail closed on the second replica; this test is what says we do not.
    let mut second = quote_for(ADDRESS, &OUR_NONCE, MRTD, MRCONFIGID);
    second.rtmr[3] = "ab".repeat(48);
    check_quote(&second, &OUR_NONCE, &trust()).expect("rtmr3 is not pinned");
}

#[test]
fn a_quote_naming_another_signer_is_refused_before_the_measurement_is_read() {
    let err = check_quote(&quote_for(OTHER_ADDRESS, &OUR_NONCE, MRTD, MRCONFIGID), &OUR_NONCE, &trust())
        .expect_err("a quote for a machine that did not sign proves nothing");
    assert_eq!(err, WitnessTrustError::WitnessSignerUnexpected);
}

#[test]
fn there_is_no_way_to_build_a_verified_witness_but_verification() {
    // Structural, not behavioural: this is the guard the whole design rests
    // on. Asserted by grep over the module's own source, which is the only
    // thing that can see a constructor a future edit adds.
    let source = include_str!("verify.rs");
    let constructors = source.matches("VerifiedWitness {").count();
    assert_eq!(constructors, 1, "a second constructor bypasses the verification");
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- drop the nonce comparison and confirm the
  replay test goes red; change `check_measurements_opt` to
  `check_measurements` with an empty set and confirm the unpinned test does;
  add `mrtd`+`rtmr3` to the default pin documentation fixture and confirm the
  RTMR3 test goes red. Report all three failure texts.
- [ ] **Step 6: Commit** -- `Verify a witness measurement before trusting it`

---

### Task 4: Fetch the evidence

**Files:**
- Modify: `crates/trace-commons-contributor/src/witness/mod.rs`
- Create: `crates/trace-commons-contributor/src/witness/transport.rs`

**Interfaces:**
- Produces: `trait WitnessTransport { async fn attestation(&self, nonce: &WitnessNonce) -> Result<AttestationEvidence, WitnessTrustError>; async fn collateral(&self, quote: &[u8]) -> Result<Collateral, WitnessTrustError>; }`, an HTTP implementation, and a test double.

The nonce is generated here, from `ring::rand::SystemRandom` -- already a
dependency. **32 bytes, fresh per verification, never reused across
submissions**, because a reused nonce turns a replayed quote into an accepted
one for as long as the reuse lasts.

The witness host passes the existing `HostAllowlist` (`config::allowlist_for`),
the same gate `issuer_url` and `ingest_url` pass, and a host outside it is
`WitnessHostNotAllowed` before any request is made.

- [ ] **Step 1: Write the failing tests** -- driven against a local `axum`
  router, using the crate's existing `axum`/`tower` dev-dependencies:

```rust
#[tokio::test]
async fn the_nonce_on_the_wire_is_the_one_we_will_check_against() {
    let (transport, seen) = recording_transport();
    let nonce = WitnessNonce::fresh().unwrap();
    transport.attestation(&nonce).await.unwrap();
    assert_eq!(seen.lock().unwrap()[0], hex::encode(nonce.as_bytes()));
}

#[tokio::test]
async fn two_verifications_never_reuse_a_nonce() {
    let a = WitnessNonce::fresh().unwrap();
    let b = WitnessNonce::fresh().unwrap();
    assert_ne!(a.as_bytes(), b.as_bytes());
}

#[tokio::test]
async fn a_host_outside_the_allowlist_is_refused_before_any_request() {
    let (transport, seen) = recording_transport_for("https://not-allowed.example");
    let err = transport.attestation(&WitnessNonce::fresh().unwrap()).await.unwrap_err();
    assert_eq!(err, WitnessTrustError::WitnessHostNotAllowed);
    assert!(seen.lock().unwrap().is_empty(), "a refused host was still contacted");
}

#[tokio::test]
async fn an_unreachable_attestation_route_refuses_by_name() {
    assert_eq!(
        dead_transport().attestation(&nonce()).await.unwrap_err(),
        WitnessTrustError::WitnessAttestationUnavailable
    );
}

#[tokio::test]
async fn missing_collateral_refuses_rather_than_verifying_without_it() {
    assert_eq!(
        transport_with_no_collateral().collateral(b"quote").await.unwrap_err(),
        WitnessTrustError::WitnessCollateralUnavailable
    );
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- make the allowlist check happen after the
  request and confirm the third test goes red (it asserts nothing was
  contacted, so it can); make `WitnessNonce::fresh` return a constant and
  confirm the reuse test does.
- [ ] **Step 6: Commit** -- `Fetch a nonce-bound quote and its collateral`

---

### Task 5: Send the raw session, and only to a verified witness

**Files:**
- Modify: `crates/trace-commons-contributor/src/witness/transport.rs`
- Modify: `crates/trace-commons-contributor/src/witness/mod.rs`

**Interfaces:**
- Produces: `async fn witness_contribution(witness: &VerifiedWitness, raw: RawTraceContribution) -> Result<WitnessedEnvelope, WitnessTrustError>`, where `WitnessedEnvelope` holds the envelope, the certificate fields and the signature.

**This is the task the ordering property lives in.** The function takes
`&VerifiedWitness`, `VerifiedWitness`'s fields are private, and Task 3's module
is the only place one can be built. There is no code path from "we have a
witness URL" to "we sent raw bytes" that does not pass through the
verification, and that is a property of the types rather than of a review.

**The client verifies the certificate it is about to forward.** It is the only
party holding both the input and the returned artifact. A witness that returned
an artifact its own certificate does not cover is undetectable on the server,
which would check that certificate against bytes it holds and find them
consistent, having never seen what was sent. Check the signature recovers to
the pinned address, and the digest matches the returned envelope bytes **as
received on the wire** -- not a re-serialisation of a parsed envelope, which
would compare the certificate against bytes nobody will ever send.

Refuse locally above `MAX_ENVELOPE_BYTES` before sending, by name. The client
already refuses raw contributions above that in `raw_contribution_size_ok`, so
this bound is not new; naming it on this path is.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn nothing_raw_is_sent_before_the_attestation_is_verified() {
    // Ordering asserted at a recording transport, not inferred from a check
    // existing. The transport records the sequence of routes it was asked
    // for; the raw body must never precede the attestation.
    let (transport, calls) = ordering_transport();
    let _ = run_witness_path(&transport, raw_with_secret()).await;
    let calls = calls.lock().unwrap();
    let attested = calls.iter().position(|c| c == "attestation");
    let sent = calls.iter().position(|c| c == "witness");
    assert!(
        attested.is_some() && sent.map(|s| attested.unwrap() < s).unwrap_or(true),
        "raw bytes were offered before the enclave was verified: {calls:?}"
    );
}

#[tokio::test]
async fn a_failed_verification_sends_nothing_at_all() {
    let (transport, bodies) = ordering_transport_with_bad_measurement();
    let err = run_witness_path(&transport, raw_with_secret()).await.unwrap_err();
    assert!(matches!(err, WitnessTrustError::WitnessMeasurementUnpinned { .. }));
    let sent = bodies.lock().unwrap().concat();
    assert!(
        !sent.contains(SECRET),
        "a refusal still disclosed the transcript, which is the failure this design exists to prevent"
    );
}

#[tokio::test]
async fn an_artifact_the_certificate_does_not_cover_is_refused() {
    let err = witness_contribution(&verified(), raw_with_secret())
        .await
        .expect_err("only the client can catch this; the server cannot");
    assert_eq!(err, WitnessTrustError::WitnessCertificateMismatched);
}

#[tokio::test]
async fn a_certificate_signed_by_another_key_is_refused() {
    assert_eq!(
        witness_contribution(&verified(), raw_with_secret()).await.unwrap_err(),
        WitnessTrustError::WitnessCertificateUnverified
    );
}

#[tokio::test]
async fn an_oversized_contribution_is_refused_locally_and_never_offered() {
    let (transport, bodies) = ordering_transport();
    let err = run_witness_path(&transport, oversized_raw()).await.unwrap_err();
    assert_eq!(err, WitnessTrustError::WitnessPayloadTooLarge);
    assert!(bodies.lock().unwrap().is_empty());
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- **the load-bearing one:** change
  `witness_contribution` to take the URL rather than `&VerifiedWitness` and
  confirm the ordering test goes red. Then swap the digest check to compare the
  certificate against what the witness said rather than against the returned
  envelope, and confirm the mismatch test does. If either survives, the
  ordering property is not actually enforced and that is a finding to report
  before continuing.
- [ ] **Step 6: Commit** -- `Send a raw session only to a verified witness`

---

### Task 6: Wire it in at the two envelope-build sites

**Files:**
- Modify: `crates/trace-commons-contributor/src/config.rs`
- Modify: `crates/trace-commons-contributor/src/envelope.rs`
- Modify: `crates/trace-commons-contributor/src/submit.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/preview.rs`

`redact_to_envelope` has exactly two production call sites -- `submit.rs:558`
and `daemon/preview.rs:602` -- and every other call is in tests. Both take the
witness path when it is configured and the local path when it is not.

**The claim is minted before the witness call.** Today `submit.rs` redacts
first and mints afterwards, then calls `stamp_granted_scopes` (`:1217`) to write
the granted scopes and uses into the finished envelope. That write is a byte
change after certification and breaks the digest, so for a witnessed submission
the order inverts: mint, pass the granted scope and use lists into the witness
request, and stamp nothing.

**The preview path has no claim at all.** `daemon/preview.rs:602` builds the
envelope before any claim exists, which is fine today and is not fine for a
witnessed preview. Mint one first, or refuse with `witness_claim_unavailable`.
This is a real behaviour change to the desktop shells' preview, and it is named
here rather than discovered mid-task.

`ContributorConfig` gains `#[serde(default)] pub witness: Option<WitnessSettings>`
holding the URL, the signing address and the measurement sets, with
`TRACE_COMMONS_WITNESS_URL`, `TRACE_COMMONS_WITNESS_SIGNING_ADDRESS` and
`TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` as the environment spelling and a
`--witness-url` flag. `#[serde(default)]` is required, not decorative: this
struct is read from a file the previous release wrote.

**Absent means the path does not execute.** Not "runs and falls back" -- the
witness module is not entered, and the local redaction is byte for byte what it
is today.

**Configured but unsatisfiable refuses the submission.** Not a fall back to
local redaction: the contributor's bytes would stay home, but the envelope
would then carry a self-reported risk while the contributor believed it carried
a certificate, and the operator would see an uncertified submission from
someone enrolled as certified. Silence about a downgrade is the failure this
design is aimed at.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn an_unconfigured_client_builds_exactly_the_envelope_it_builds_today() {
    let with_none = build_envelope(&config_without_witness(), transcript()).await.unwrap();
    let baseline = redact_to_envelope(&redactor(), raw_from(transcript())).await.unwrap();
    assert_eq!(
        serde_json::to_vec(&with_none).unwrap(),
        serde_json::to_vec(&baseline).unwrap(),
        "the witness feature changed the default path"
    );
}

#[tokio::test]
async fn a_witness_url_without_a_pin_refuses_the_submission() {
    let err = build_envelope(&config_with_url_but_no_pin(), transcript()).await.unwrap_err();
    assert_eq!(
        refusal_label(&err),
        "witness_expected_measurement",
        "an unpinned witness must refuse, never quietly redact locally"
    );
}

#[tokio::test]
async fn a_witness_url_without_a_pin_never_reaches_the_network() {
    let (transport, calls) = ordering_transport();
    let _ = build_envelope_with(&config_with_url_but_no_pin(), transport, transcript()).await;
    assert!(calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn a_witnessed_submission_mints_its_claim_before_it_sends_anything_raw() {
    let (transport, calls) = ordering_transport();
    let _ = build_envelope_with(&config_with_witness(), transport, transcript()).await;
    let calls = calls.lock().unwrap();
    assert!(
        calls.iter().position(|c| c == "mint").unwrap()
            < calls.iter().position(|c| c == "witness").unwrap(),
        "grants must be inside the certified bytes, not stamped after: {calls:?}"
    );
}

#[tokio::test]
async fn a_witnessed_preview_without_a_claim_refuses_by_name() {
    let err = preview_with(&config_with_witness(), no_claim(), transcript()).await.unwrap_err();
    assert_eq!(refusal_label(&err), "witness_claim_unavailable");
}

#[tokio::test]
async fn a_configured_witness_replaces_the_local_redaction_rather_than_supplementing_it() {
    // The local redactor is still built (the canary and the residual-secret
    // sweep need it) but must not have produced the envelope that is sent.
    let (envelope, local_redactions) = build_envelope_counting_local_passes(&config_with_witness()).await;
    assert_eq!(local_redactions, 0);
    assert_eq!(envelope.privacy.redaction_pipeline_version, WITNESS_POLICY_VERSION);
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- make the unsatisfiable case fall back to
  local redaction and confirm both refusal tests go red; make the unconfigured
  case route through the witness module and confirm the first test does; move
  the mint back after the witness call and confirm the ordering test does.
- [ ] **Step 6: Commit** -- `Build the envelope through a witness when one is pinned`

---

### Task 7: Carry the certificate to ingest, including through approval

**Files:**
- Modify: `crates/trace-commons-operator-client/src/client.rs` (a byte-body call)
- Modify: `crates/trace-commons-contributor/src/submit.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/queue.rs`
- Modify: `crates/trace-commons-contributor/src/daemon/approved_envelope.rs`

**The client has no way to send bytes verbatim, and this is where that is
fixed.** `Client::call_json` (`operator-client/src/client.rs:68`) delegates to
`Client::call_raw` (`:102`), and `call_raw` takes `Req: Serialize` and does
`request.json(body)`. Both re-serialise, so both destroy the digest. Add a
byte-body call that sets `Content-Type: application/json`, passes the bytes
through untouched, and keeps the host allowlist and bearer handling the other
two have. Witnessed submissions use it; nothing else changes.

**A re-mint cannot restamp, so it refuses.** `upload_with_retry` (`:1251`)
re-stamps granted scopes after a 401/403 re-mint, deliberately, so a stale
grant is not resent. On a witnessed submission that write breaks the digest,
and silently re-witnessing would send the raw session a second time on the
strength of a verification made for a different exchange. Refuse with
`witness_claim_expired`; the contributor re-runs, which re-verifies and
re-witnesses explicitly.

**Headers, not an envelope field.** `x-trace-witness-certificate` (the
certificate's fields as compact JSON) and `x-trace-witness-signature` (65 bytes
of `0x`-prefixed hex) on `POST /v1/traces`. A new field on
`TraceContributionEnvelope` would move the golden digest pinned in the
contributor crate and would make the certificate part of the bytes the
certificate is over.

**The approved-envelope path.** The desktop shells build the envelope in
`preview` and upload those exact bytes later. The certificate is obtained at
build time and must be stored beside them and travel with them, and the
existing approval fingerprint must cover it -- otherwise an approved envelope
could be paired with a certificate for something else. This is what makes the
service spec's "the `approved_envelope` path cannot be witnessed" false: it is
unwitnessable only if you witness at upload time.

**Rollout overlap.** `envelope.privacy.residual_pii_risk` stays exactly as it
is, client-computed, and ingest treats it exactly as it does today. The
certificate is additional evidence; a server that ignores the headers accepts
the submission unchanged and nothing here depends on the server-side plan
having run.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_witnessed_submission_carries_the_certificate_in_headers_and_not_in_the_body() {
    let sent = capture_submission(witnessed()).await;
    assert!(sent.headers.contains_key("x-trace-witness-certificate"));
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert!(body.get("witness_certificate").is_none(), "the envelope grew a field it is hashed over");
}

#[tokio::test]
async fn an_unwitnessed_submission_sends_byte_identical_bytes_and_no_new_headers() {
    let sent = capture_submission(unwitnessed()).await;
    assert_eq!(sent.body, baseline_submission_bytes());
    assert!(!sent.headers.keys().any(|k| k.as_str().starts_with("x-trace-witness")));
}

#[tokio::test]
async fn the_bytes_on_the_wire_are_the_bytes_the_certificate_covers() {
    // The test that catches a re-serialisation. Compares the captured request
    // body against the witness's bytes byte for byte -- not field by field,
    // and not by parsing both sides, which is the comparison that would pass
    // over exactly the bug being hunted.
    let (sent, witnessed_bytes) = capture_submission_and_witness_bytes().await;
    assert_eq!(sent.body, witnessed_bytes);
    assert_eq!(
        hex::encode(sha2::Sha256::digest(&sent.body)),
        certificate_digest_of(&sent),
    );
}

#[tokio::test]
async fn a_re_mint_refuses_rather_than_restamping_certified_bytes() {
    let err = upload_witnessed_against_a_server_that_401s_once().await.unwrap_err();
    assert_eq!(refusal_label(&err), "witness_claim_expired");
}

#[tokio::test]
async fn an_approved_envelope_uploads_the_certificate_it_was_approved_with() {
    let sent = capture_submission(approved_then_uploaded()).await;
    assert_eq!(sent.headers["x-trace-witness-signature"], APPROVED_SIGNATURE);
}

#[tokio::test]
async fn an_approved_envelope_whose_certificate_is_gone_refuses_rather_than_uploading_bare() {
    let err = upload_approved_without_certificate().await.unwrap_err();
    assert_eq!(refusal_label(&err), "witness_certificate_missing");
}

#[tokio::test]
async fn the_residual_risk_field_is_unchanged_by_witnessing() {
    // Both shapes on the wire at once is the rollout, and the fields do not
    // overlap.
    assert!(witnessed_body()["privacy"]["residual_pii_risk"].is_string());
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** -- **the load-bearing one:** route a witnessed
  submission back through `call_json` and confirm the byte-equality test goes
  red. If it passes, the test is comparing parsed values somewhere and is not
  the guard it claims to be. Then drop the certificate from the approval
  fingerprint and confirm the foreign-certificate test goes red; make the
  re-mint restamp and confirm `witness_claim_expired` does.
- [ ] **Step 6: Commit** -- `Carry the witness certificate alongside the envelope`

---

### Task 8: The first real deployment -- BLOCKED until a CVM exists

**Files:**
- Create: `docs/operator/witness-client.md`
- Modify: `deploy/witness/README.md` (the client-facing half of the upgrade order)
- Modify: `docs/operator/README.md` (the runbook index)

**This is the only task in this plan that a deployment blocks**, and it cannot
be completed, faked or partially credited without one. Tasks 1-7 ship a client
that refuses to send because nothing is pinned, which is the correct inert
state and not a gap.

What this task does, and none of it is derivable from the tree:

- Read a measurement off a running instance's `/v1/attestation` -- **not** off
  the boot log, and **not** from `build-app-compose.sh`'s local derivation
  until that derivation has been compared against a live instance's
  `tcb_info.compose_hash`. `deploy/witness/README.md` records that the
  derivation is unconfirmed.
- Confirm the quote parses, carries a report body in the layout Task 3
  reconstructs, and that ingest's PCCS can produce collateral for that
  platform. None of this has been done against a live dstack agent by anyone on
  this project.
- Confirm whether the deployment's config-id is v1 or v2. Either pins the
  compose hash, so the pin is sound either way, but **no contributor-facing
  text may claim app-id binding until v2 is confirmed.**
- Probe the real request-body ceiling on the path with a payload at the 16 MB
  local bound. This does not gate the client -- `raw_contribution_size_ok`
  already refuses above 16 MB before redaction, so the witness's 64 MiB bound
  is four times what this client can offer -- but it must be a measured number
  before anyone raises that ceiling.
- Write the contributor-facing configuration document: what to pin (`mrtd`,
  `mrconfigid`), why not `rtmr3` (a per-deployment random instance-id, so two
  replicas differ) and why not `rtmr0` (SMBIOS tables that move on a resize),
  and the upgrade order -- **the new measurement is added to every client's
  pinned set before the fleet rolls**, or every correctly-pinned contributor
  refuses the new deployment and it will look like an outage.
- State, in that document and in exactly these terms, that **the image is not
  reproducibly buildable and has never been reproduced.** A pin proves the
  deployment has not changed under the contributor and that two contributors
  are talking to the same enclave. It does not prove the running code is the
  code in this repository. Do not write "verifiable against source".
- State that the witness sees raw sessions and what its compromise costs. A
  contributor turning this on is making that trade and is entitled to read it
  in one place.

- [ ] **Step 1: Deploy one instance and capture its measurement from `/v1/attestation`**
- [ ] **Step 2: Verify the quote end to end with the Task 3 code path and real collateral**
- [ ] **Step 3: Probe the body ceiling at the local 16 MB bound**
- [ ] **Step 4: Write `docs/operator/witness-client.md` and index it**
- [ ] **Step 5: Commit** -- `Document pinning a witness from a contributor client`

---

## Not in this plan

- **Ingest weighing the certificate.** Whether a verified certificate lets a
  trace skip any part of the PII backstop is the server-side plan named in the
  service plan's "Not in this plan". Nothing here depends on it, and a client
  emitting certificates at a server that ignores them loses nothing.
- **Refusing a `deterministic-only` witness.** Its certificate is honest and
  narrower, and a contributor sending raw bytes for a certificate a strict
  server will refuse gets nothing for the disclosure. Arguably the right
  default; the policy alias is readable only after the exchange, so it is left
  as an open question in the spec rather than guessed at here.
- **Pinning `tcb_status`.** `VerifiedQuote` carries Intel's verdict and its
  advisory IDs, and refusing anything but `UpToDate` is a stronger check. The
  pin config is shaped so it can be added; deciding it needs an operator who
  has seen how often that verdict moves.
- **Distributing the pin.** A contributor cannot learn a trustworthy
  measurement from the thing they are trying to verify. Where a pin comes from
  -- release notes, a signed statement, this repository -- is a distribution
  question this project has not answered for anything yet.
