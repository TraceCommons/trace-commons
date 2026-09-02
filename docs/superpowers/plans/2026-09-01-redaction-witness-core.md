# Redaction witness — correspondence core Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and prove the redaction witness's security core — the correspondence check and the certificate it issues — as pure, fully testable logic, before any enclave exists to run it in.

**Architecture:** A new `redaction_witness` module in `trace-commons-server`. Three pieces: an exact correspondence check that applies a span list to raw bytes and requires byte equality with the submitted redacted artifact; a certificate type covering the redacted artifact's hash plus the inference facts; and the server-side verification of that certificate. The witness's HTTP surface, its dstack packaging, and the client's span emission are **not** in this plan.

**Tech Stack:** Rust. Reuses `near_attestation::receipt` (already on `main`) for receipt verification, `sha2`, `k256`/`ed25519`, and the existing hash-only conventions.

**Status: complete.** Shipped in #533. This plan is kept as the record of
what was built and how it was proved; it is not a plan to execute.

**Spec:** the design this plan was written against has been replaced by
[`docs/superpowers/specs/2026-09-02-redaction-witness-service-design.md`](../specs/2026-09-02-redaction-witness-service-design.md).
The verification core below shipped unchanged and is reused. What did not
survive is the rationale: the witness no longer checks a client-supplied span
list, because it performs the redaction itself, and it no longer binds a
certificate to a NEAR AI inference receipt, because no trace population in this
repo carries one.

## Why this slice, and what it deliberately excludes

The witness's value rests entirely on one property: that the redacted artifact provably derives from the raw bytes an inference receipt covers. That property is **pure logic**. It needs no enclave, no network, no dstack, and no client change to build or to prove.

Everything excluded here is real work that depends on decisions or hardware this slice does not:

- **The witness HTTP service and its dstack packaging.** Needs the deployment path settled. The spec records that path as *assumed*, not verified.
- **The client's span emission.** `SafePrivacyFilterSummary` keeps `span_count` and labels only — spans themselves are never retained. Emitting them is new work in a permissive crate, and it must never send them to our server.
- **Whole-trace versus per-turn witnessing.** An open item in the spec; it shapes the service's API, not this logic.

Building the core first means that when those decisions land, the thing they wrap is already proven.

## Global Constraints

- `trace-commons-server` is **AGPL-3.0-or-later**. Every new `.rs` file carries the two-line copyright + SPDX header matching its neighbours.
- **Hash-only.** Never log or store raw bytes, redacted bytes, a span list, a signature, or a signing address. Span lists are especially sensitive: their *shape* reveals what the detector found.
- **Fail closed.** Any check that cannot be completed refuses with a named reason. There is no "assume corresponding".
- **Every negative assertion names its specific error variant.** Not `assert!(x.is_err())`. Fifteen assertions on this project turned out structurally incapable of failing, several because a coarse predicate was satisfied by the wrong error.
- **Mutation-check every guard.** Break the thing it protects, watch it go red, revert, and report what the failure looked like. A check nobody has seen fail is not yet a check.
- No emojis. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace` and **confirm the run terminated** — a failure aborts it and leaves a plausible but truncated pass count.

## File structure

- **Create** `crates/trace-commons-server/src/redaction_witness/mod.rs` — module root, shared error types.
- **Create** `crates/trace-commons-server/src/redaction_witness/correspondence.rs` — the span application and byte-equality check.
- **Create** `crates/trace-commons-server/src/redaction_witness/certificate.rs` — the certificate type, its canonical signing bytes, and verification.
- **Modify** `crates/trace-commons-server/src/lib.rs` — declare the module.

---

### Task 1: The span list, and applying it

**Files:**
- Create: `crates/trace-commons-server/src/redaction_witness/mod.rs`
- Create: `crates/trace-commons-server/src/redaction_witness/correspondence.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`

**Interfaces:**
- Produces: `RedactionSpan { start: usize, end: usize, replacement: String }`, `apply_spans(raw: &str, spans: &[RedactionSpan]) -> Result<String, CorrespondenceError>`

**The decision this task encodes.** Offsets are **codepoint** indices, not byte indices. The existing privacy-filter adapter already learned this — its offsets are codepoint-based, and a byte-index reading corrupts any non-ASCII trace. Applying spans must therefore operate on `char_indices`, and a span boundary that does not fall on a character boundary is an error, not a silent truncation.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn spans_apply_in_order_and_produce_the_expected_text() { /* ... */ }

#[test]
fn overlapping_spans_are_refused() {
    // Two spans covering the same region cannot both be applied, and applying
    // one silently would change what the other asserts.
    let err = apply_spans("hello world", &[span(0, 5, "X"), span(3, 8, "Y")]).unwrap_err();
    assert_eq!(err, CorrespondenceError::OverlappingSpans { first: 0, second: 1 });
}

#[test]
fn a_span_past_the_end_is_refused() {
    let err = apply_spans("short", &[span(0, 99, "X")]).unwrap_err();
    assert_eq!(err, CorrespondenceError::SpanOutOfRange { index: 0, end: 99, len: 5 });
}

#[test]
fn offsets_are_codepoints_not_bytes() {
    // The whole test. "é" is one codepoint and two bytes; a byte-index
    // implementation produces different output or splits the character.
    let out = apply_spans("café latte", &[span(0, 4, "REDACTED")]).unwrap();
    assert_eq!(out, "REDACTED latte");
}

#[test]
fn a_span_boundary_inside_a_character_is_refused_not_truncated() { /* ... */ }

#[test]
fn an_empty_span_list_returns_the_input_unchanged() { /* ... */ }
```

**Verify the non-ASCII test actually discriminates** before trusting it: confirm a byte-index implementation produces a *different* answer for your chosen input. A test whose input happens to be ASCII-only proves nothing about the property it claims.

- [ ] **Step 2: Run to verify they fail** — `cargo test -p trace-commons-server --lib redaction_witness`
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check** — swap `char_indices` for byte slicing and confirm the non-ASCII test fails. Report the failure text.
- [ ] **Step 6: Commit**

---

### Task 2: The correspondence check

**Files:**
- Modify: `crates/trace-commons-server/src/redaction_witness/correspondence.rs`

**Interfaces:**
- Consumes: `apply_spans` (Task 1)
- Produces: `check_correspondence(raw: &str, redacted: &str, spans: &[RedactionSpan]) -> Result<CorrespondenceProof, CorrespondenceError>` where `CorrespondenceProof` carries `redacted_sha256: String` and nothing else.

**What this proves and does not.** It proves **faithfulness** — the redacted artifact derives from the raw one by redaction alone and was not fabricated, padded, or swapped for another session's output. It does **not** prove **sufficiency**: whether enough PII was removed remains the redaction policy's job and the backstop's. No name, comment, or error string in this module may imply otherwise.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_faithful_redaction_is_proved() { /* ... */ }

#[test]
fn a_fabricated_redacted_artifact_is_refused() {
    // The attack: submit real raw bytes with a redacted artifact from a
    // different session. Spans apply cleanly; the result does not match.
    let err = check_correspondence(RAW, "entirely different text", &spans()).unwrap_err();
    assert_eq!(err, CorrespondenceError::RedactedMismatch);
}

#[test]
fn extra_text_appended_to_the_redacted_artifact_is_refused() {
    // Byte equality, not prefix or containment. A previous check on this
    // project asserted `contains` and a longer string satisfied it as a prefix.
}

#[test]
fn a_span_that_hides_nothing_still_has_to_match() { /* ... */ }
```

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — relax the comparison to `starts_with` and confirm the appended-text test fails. That exact weakening shipped on this project once.
- [ ] **Step 6: Commit**

---

### Task 3: The certificate

**Files:**
- Create: `crates/trace-commons-server/src/redaction_witness/certificate.rs`

**Interfaces:**
- Produces: `WitnessCertificate` with `redacted_sha256`, `chat_id`, `prompt_tokens`, `completion_tokens`, `model`, `timestamp`, `redaction_policy_version`, `witness_measurement`; plus `signing_bytes(&self) -> Vec<u8>` and `verify(&self, signature, key) -> Result<(), CertificateError>`.

**Canonical signing bytes must not use JSON.** This repo's house pattern for stable bytes is a length-prefixed encoder — see `instance_enroll_attestation_signing_bytes` in `onboarding.rs:113` — precisely because `serde_json`'s map ordering is not guaranteed. A dependency enabling `serde_json/preserve_order` shifted every untyped-JSON digest in this workspace on 2026-09-01. **Follow the length-prefixed pattern; do not serialize a struct to JSON and hash it.**

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn signing_bytes_are_length_prefixed_and_unambiguous() {
    // The property that matters: no two distinct field sets can produce the
    // same bytes. Adjacent fields whose contents could be shifted between them
    // must not collide.
    let a = cert_with(/* model */ "ab", /* policy */ "c");
    let b = cert_with(/* model */ "a",  /* policy */ "bc");
    assert_ne!(a.signing_bytes(), b.signing_bytes());
}

#[test]
fn a_certificate_for_a_different_artifact_does_not_verify() { /* ... */ }

#[test]
fn a_tampered_token_count_does_not_verify() { /* ... */ }

#[test]
fn a_signature_by_a_different_key_is_refused() { /* ... */ }
```

The first test is the one to get right. Field-shift collision is the classic length-prefix failure and it is invisible to every other test.

- [ ] **Step 2-4: Run, implement, run**
- [ ] **Step 5: Mutation-check** — drop the length prefixes, concatenating fields directly, and confirm the collision test fails.
- [ ] **Step 6: Commit**

---

### Task 4: Server-side verification

**Files:**
- Modify: `crates/trace-commons-server/src/redaction_witness/mod.rs`

**Interfaces:**
- Produces: `verify_witness_certificate(cert, signature, expected_measurement, redacted_bytes) -> Result<(), WitnessError>`

This is what the server runs against an artifact it already holds. It must check, and name distinctly on failure: the signature; that `redacted_sha256` matches the bytes on hand; and that `witness_measurement` is one the operator pinned.

**The measurement check is the whole trust story.** A valid signature from an unpinned enclave proves only that *some* enclave signed it. Follow the fail-closed shape already established for NEAR AI measurements: no pinned set configured means **refuse**, with a named missing control — never skip, never pass.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn no_pinned_measurement_refuses_rather_than_passing() {
    assert!(matches!(verify_witness_certificate(&c, &sig, None, BYTES),
                     Err(WitnessError::Refused { control }) if control == "witness_expected_measurement"));
}

#[test]
fn a_certificate_from_an_unpinned_enclave_is_refused() { /* distinct from a bad signature */ }

#[test]
fn a_certificate_whose_hash_does_not_match_the_held_bytes_is_refused() { /* ... */ }
```

- [ ] **Step 2-6: Run, implement, run, mutation-check, commit**

---

## Not in this plan

- The witness HTTP service and its dstack packaging — blocked on the deployment path, which the spec records as **assumed** rather than verified.
- The client's span emission. Note it must reach the witness **only**; a span list must never be sent to our server, and nothing in this plan gives it a path there.
- Whole-trace versus per-turn witnessing.
- Any admission decision. Nothing here grants a contributor anything; it establishes a fact a later policy may use.
