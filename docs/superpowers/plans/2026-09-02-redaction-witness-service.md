# The redaction witness service Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the service that performs a redaction inside an attested enclave and signs a certificate the server can verify — so a trace can be trusted without our ever holding the raw bytes.

**Architecture:** A new binary in `trace-commons-server` (AGPL, like the rest of that crate) that holds no database, no queue and no state beyond its signing key. It redacts with the same code ingest uses, computes the residual-risk verdict with the same function ingest uses, signs the existing `WitnessCertificate`, and serves a nonce-bound attestation of itself. It runs on Phala/dstack, CPU-only.

**Tech Stack:** Rust. `axum` for the surface, the existing `redaction_witness` module for correspondence and certificates, `trace-commons-protocol` for redaction and residual risk, dstack's guest agent for the key and the quote.

**Spec:** [`docs/superpowers/specs/2026-09-02-redaction-witness-service-design.md`](../specs/2026-09-02-redaction-witness-service-design.md) — read "The organising principle", "The certificate", "Attestation and pinning" and "Trust model" before starting.

## Why this exists, stated once so no task loses it

Ingest decides whether to hold a trace for its PII backstop from
`envelope.privacy.residual_pii_risk` — a **client-asserted** field.
`rescrub_trace_envelope` re-derives it server-side precisely because a client's
word is not evidence. The backstop is also the slowest thing in the pipeline.

A witness running in an enclave whose measurement a contributor pinned *before*
sending anything can assert that verdict with something better than its word.
That is the whole point: **not that the trace is clean, but that a known
program in a known enclave said so.**

Two claims that are easy to conflate, and this plan must not:

- A **NEAR AI receipt** proves an exchange happened inside an attested enclave.
  Provenance of *inference*.
- A **witness certificate** proves the artifact we hold derives from raw bytes
  by redaction alone, and carries the verdict a known program reached about it.
  Correspondence of *redaction*.

A receipt says nothing about whether what leaves the machine is a faithful
redaction. Nothing in this plan may describe a witnessed trace as "verified
clean": the witness attests **mechanics and a verdict**, never sufficiency.

## What already exists, verified on `main`

- `crates/trace-commons-server/src/redaction_witness/` — `apply_spans`,
  `check_correspondence` → `CorrespondenceProof`, `WitnessCertificate` with
  length-prefixed `signing_bytes`, and `verify_witness_certificate` →
  `VerifiedWitnessCertificate`. Merged in #533, reviewed, mutation-checked.
  **The server half is done; this plan builds the half that issues.**
- `crates/trace-commons-attestation` — quote verification, measurement pinning,
  EIP-191 recovery, now `MIT OR Apache-2.0` so a contributor client can use it
  (#540). `MRCONFIGID` is a pinnable `MeasurementField`.
- `trace-commons-protocol` (permissive) holds `DeterministicTraceRedactor`,
  `residual_risk_basis`, and `rescrub_envelope_prose_pii_with`. **Permissive
  code may flow into AGPL crates, so the witness may use all of it.**

## Global Constraints

- **The witness holds nothing.** No database, no queue, no disk state beyond its
  signing key. Raw bytes live in memory for one request and are never written,
  never logged, never included in an error. A witness that persists raw is a
  worse breach than the one it prevents.
- **Hash-only logging**, as everywhere in this repo: no raw bytes, no redacted
  bytes, no span list, no signature, no signing address in a log line. A span
  list is especially sensitive — its *shape* reveals what the detector found.
- **Fail closed.** Any check that cannot be completed refuses with a named
  reason. There is no "assume corresponding" and no "assume clean".
- **Never claim sufficiency.** No name, comment, error string or response field
  may imply the artifact is clean. It attests correspondence and a verdict.
- **Every negative assertion names its specific error variant.** Not
  `assert!(x.is_err())`. Twenty-two assertions on this project have turned out
  structurally incapable of failing.
- **Mutation-check every guard**, and prefer ground truth from outside the code
  under test — a review found two mutations surviving 46 tests because every
  test reached the function through another function that used it.
- AGPL two-line header on every new `.rs` file in `trace-commons-server`. No
  emoji. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace`, **capture
  cargo's own exit code** (a `tail` in a pipe reported success over a failed
  build on this machine), and confirm the run terminated.
- **Prefix every command with its own explicit `cd`** to the worktree. A green
  exit code from the wrong tree is indistinguishable from a green one from the
  right tree; this has cost time four times on this project.

## File structure

- **Modify** `crates/trace-commons-server/src/redaction_witness/certificate.rs` — reconcile with the spec
- **Create** `crates/trace-commons-server/src/bin/trace-commons-witness.rs` — the binary
- **Create** `crates/trace-commons-server/src/witness_service/mod.rs` — the surface, testable without a socket
- **Create** `crates/trace-commons-server/src/witness_service/enclave.rs` — key and quote, behind a trait
- **Create** `deploy/witness/` — the dstack deployment artifacts

---

### Task 1: Make the certificate say what the spec says

**Files:**
- Modify: `crates/trace-commons-server/src/redaction_witness/certificate.rs`
- Modify: `crates/trace-commons-server/src/redaction_witness/verification.rs` (only if the verified accessors change)

**The divergence this task fixes.** The certificate merged in #533 carries
`chat_id`, `prompt_tokens`, `completion_tokens` and `model` — the inference
fields. The spec that later merged carries instead:

```
redacted_sha256
residual_risk_verdict
redaction_policy_version
witness_measurement
timestamp
```

The spec is right and the reason is recorded in it: **no trace population in
this repo carries a NEAR AI receipt to populate those fields from.** A CLI
transcript is a local agent session; IronWire's ledger carries a routing record,
not a receipt. Fields no honest path can fill are an invitation to fill them
dishonestly.

`residual_risk_verdict` replaces them and is the field the whole design turns
on.

**Do not simply delete and re-add.** `signing_bytes` is length-prefixed
specifically so no two distinct field sets can produce the same bytes. Two
existing tests pin that, at `certificate.rs:430`
`signing_bytes_are_unambiguous_across_every_adjacent_string_pair` and
`certificate.rs:470` `an_empty_string_field_is_still_length_prefixed`.

Changing the field set changes every signature and changes which pairs are
adjacent. **Make sure the first test still varies real adjacent fields after
your edit**, rather than passing because the pair it shifted no longer exists —
a collision test that no longer has a collision to find is documentation.

**One judgement to make and record.** IronWire now records a provider response
id (`nearai/ironwire#19`), so a *future* IronWire-sourced trace could carry a
receipt. Decide whether the inference fields come back later as an optional
second profile, or stay out. **State your reasoning in the module doc** — the
next person will otherwise re-add them without knowing they were removed on
purpose.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_certificate_binds_the_residual_risk_verdict() {
    // Two certificates identical but for the verdict must not share
    // signing bytes -- otherwise the field the design turns on is not
    // actually signed.
}

// And keep the two existing collision tests meaningful against the new
// field set -- see the note above. They are at certificate.rs:430 and :470.

```

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** — drop `residual_risk_verdict` from
  `signing_bytes` and confirm the first test goes red; drop the length prefixes
  and confirm the second does. Report both failures.
- [ ] **Step 6: Commit**

---

### Task 2: Redact, judge, and certify

**Files:**
- Create: `crates/trace-commons-server/src/witness_service/mod.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`

**Interfaces:**
- Consumes: `DeterministicTraceRedactor`, `residual_risk_basis`,
  `check_correspondence`, `WitnessCertificate::from_proof` (Task 1's shape)
- Produces: `async fn witness(request: WitnessRequest, signer: &dyn Signer, enclave: &dyn Enclave) -> Result<WitnessResponse, WitnessError>`

**This is the whole service, as a function.** Keep it independent of `axum` so
it is testable without a socket — the HTTP layer in Task 4 should be a thin
adapter over this. The pattern this repo already uses for a gate is a trait
object at the seam; do the same for signing and for the enclave so tests
substitute them.

**The order of operations matters and is the design.** The witness *performs*
the redaction; it does not check a client's. That is because
`DeterministicTraceRedactor` holds a model-based classifier whose calls do not
reproduce, so a witness that recomputed and compared would fail on honest
submissions. Applying spans is still needed — `check_correspondence` is how the
witness proves to *itself* that what it is about to certify derives from what it
was given.

**Compute the verdict with `residual_risk_basis`**, the same function ingest
uses, from the permissive crate. Do not reimplement it: the value of the
certificate is that a known program reached the same verdict the server would
have, and two implementations drift.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn a_witnessed_artifact_and_its_certificate_agree() {
    // The certificate's digest must be of the bytes the response returns.
    // A certificate for a different artifact is the failure mode the whole
    // design exists to prevent, so assert the digest against the returned
    // bytes rather than against what the witness computed internally.
}

#[tokio::test]
async fn the_verdict_matches_what_the_shared_function_says() {
    // Ground truth from outside the code under test: call
    // residual_risk_basis directly on the same input and compare.
}

#[tokio::test]
async fn raw_bytes_never_appear_in_a_response_or_an_error() {
    // Feed a distinctive secret. Assert it appears in neither the serialized
    // response nor any error rendering, on the success path AND on a refusal.
}

#[tokio::test]
async fn a_redaction_failure_refuses_rather_than_certifying_what_it_has() {
    // Name the variant.
}
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — make the certificate carry the digest of the
  *pre*-redaction bytes and confirm the agreement test goes red; make the
  verdict a constant and confirm the shared-function test does.
- [ ] **Step 6: Commit**

---

### Task 3: The key and the quote

**Files:**
- Create: `crates/trace-commons-server/src/witness_service/enclave.rs`

**Interfaces:**
- Produces: `trait Enclave { fn signing_address(&self) -> &str; fn quote(&self, report_data: [u8; 64]) -> Result<Vec<u8>, EnclaveError>; }`, a dstack implementation, and a test double

**What dstack actually provides, verified — build to this, not to memory.**

- The app signing key is derived from a **stable app id**, not from the
  measurement. So an upgrade changes the measurement and **keeps the signing
  address**. Pin the two separately; that is what makes an upgrade a
  re-allowlisting rather than a fleet-wide break.
- `GetQuote(report_data)` is on the guest agent's **unix socket**, takes 64
  bytes, and is **not network-reachable**. The witness must proxy it.
- **Return the raw quote bytes.** dstack 0.5.9 rewired its v1 attestation
  envelope to msgpack, which we have no decoder for. Raw sidesteps it.
- The contributor supplies the nonce. A fixed report is a replay, so a
  contributor-chosen nonce is the entire point.
- Pin **MRTD and MRCONFIGID**. Not RTMR3 — it carries a per-deployment random
  `instance-id`. Not RTMR0 — it hashes SMBIOS tables that change when a VM is
  resized.

**Verify the socket path and method name against dstack's current source**
before writing the client. This brief is a summary of a summary.

- [ ] **Step 1: Write the failing tests** — against the test double, not a real
  socket: the nonce reaches `report_data`; the signing public key is bound
  alongside it; a quote request that fails yields a named error rather than an
  empty quote.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — drop the nonce from `report_data` and confirm
  the binding test goes red. **A quote that does not carry the caller's nonce is
  a replay**, and this is the test that says so.
- [ ] **Step 6: Commit**

---

### Task 4: The HTTP surface and the binary

**Files:**
- Create: `crates/trace-commons-server/src/bin/trace-commons-witness.rs`
- Modify: `crates/trace-commons-server/Cargo.toml`

**Two routes and nothing else:**

- `POST /v1/witness` — the raw transcript in, the redacted artifact and its
  certificate out. Thin over Task 2.
- `GET /v1/attestation?nonce=<hex>` — the nonce-bound quote and the signing
  address, so a contributor can verify the enclave **before** sending anything.

**No health route that reveals state, no metrics that count content, no
route that lists anything.** The witness's whole security posture is that it
holds nothing; a surface that can be asked what it has seen contradicts it.

Bound the request body deliberately and refuse over it with a named error: the
witness receives *raw* transcripts, larger than the 16 MB redacted-envelope cap
at a measured ~3.4:1 ratio, and 7% of real sessions already exceed that cap
before the multiplier.

- [ ] **Step 1: Write the failing tests** — route wiring through the real
  router (a handler can be correct and unreachable; that exact defect was found
  on this project today), an oversized body refused by name, and the attestation
  route rejecting a malformed nonce rather than padding it.
- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — remove the size bound and confirm the
  oversize test goes red; remove a route from the router and confirm the wiring
  test does.
- [ ] **Step 6: Commit**

---

### Task 5: The dstack deployment

**Files:**
- Create: `deploy/witness/docker-compose.yml`, `deploy/witness/app-compose.json`, `deploy/witness/README.md`

**This is the project's first real trusted-execution deployment**, so the
operator documentation is part of the deliverable rather than an afterthought.

The README must answer, for someone who has never deployed a CVM:

- How to build the image reproducibly enough that a measurement means something.
  **If it is not reproducible, say so plainly** — a measurement over a
  non-reproducible build pins a binary nobody can re-derive, which is worth
  knowing before anyone pins it.
- Which values to pin (MRTD, MRCONFIGID) and where an operator reads them.
- What changes on an upgrade — the measurement, not the signing address — and
  the rollout order that follows: allowlist the new measurement *before*
  deploying, so no client is broken by an upgrade it has not been told about.
- That the witness sees raw traces, and what that means if it is compromised.

- [ ] **Step 1: Write the compose and app-compose**
- [ ] **Step 2: Write the README, including the upgrade order**
- [ ] **Step 3: Verify the image builds and the binary starts with no state**
- [ ] **Step 4: Commit**

---

## Not in this plan

- **Ingest trusting a certificate to skip the PII backstop.** That is the
  payoff, and it is a server change with its own risks — four surfaces read the
  hold state, and a bypass must account for all of them. Its own plan, after
  this one runs.
- **The client half.** Verifying the witness measurement before sending, and
  emitting the raw transcript. It needs the permissive attestation crate (#540)
  and a decision about which shell ships it first.
- **Spans from the client.** The witness performs the redaction, so no span list
  leaves the contributor's machine — which retires the spec's open question
  about whether a span list's shape leaks what the detector found.
- **Whole-trace versus per-turn.** Phala documents no request-body ceiling and
  dstack-gateway is a TCP/TLS proxy with no body parser, so whole-transcript
  looks viable. **Measure with a real 60 MB POST before committing the API
  shape.**
