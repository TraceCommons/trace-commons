# Attestation verification, extracted and correctly pinned Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move TDX quote verification, measurement pinning and EIP-191 signer
recovery out of the AGPL server crate into a new `MIT OR Apache-2.0` crate, and
pin the measurement field that is actually stable.

**Architecture:** A new `crates/trace-commons-attestation` holds the generic
verification: quote parsing and DCAP verification, the measurement types and
comparison, and EIP-191 recovery. `near_attestation` in the server keeps what is
genuinely NEAR-specific -- the `AttestationReport` JSON envelope, the
`UnverifiedJsonMeasurements` claims, the env-var configuration, the HTTP client
and the drill -- and re-exports the moved items so existing call sites keep
working.

**Tech Stack:** Rust. `dcap-qvl` 0.6 (Phala), `k256`, `sha2`, `sha3`.

**Spec:** [`docs/superpowers/specs/2026-09-02-redaction-witness-service-design.md`](../specs/2026-09-02-redaction-witness-service-design.md)
-- read "Attestation and pinning" and "The licensing cost" before starting.

## Why this slice

Two facts from reconnaissance, both verified against the tree:

**We pin the wrong values.** `check_measurements` compares against `mrtd` and
`rtmr0..3`. But RTMR3 is extended with an `instance-id` seeded from `getrandom`
at deployment, so it is unpinnable across *instances*, not merely across
upgrades. RTMR0's event chain hashes SMBIOS tables that scale with `-m` and
`-cpu`, so resizing a CVM fails a pinned RTMR0 closed. The stable identity of
what code runs is **`MRCONFIGID`**, which commits to the compose hash, the
20-byte app id and the key-provider identity. `dcap-qvl` already exposes
`mr_config_id: [u8; 48]` on the parsed report; we never copy it out.

**A contributor cannot verify a witness today.** The spec requires the client to
verify the witness's quote *before* sending raw bytes, and refuse if it cannot.
All of that code is AGPL, and the contributor crates are `MIT OR Apache-2.0`
because they ship inside proprietary harnesses. `tests/license_boundary.rs`
enforces the direction. Nothing client-side can exist until the extraction does.

Both are pure library work. No enclave, no network, no deployment.

## Global Constraints

- The new crate is **`MIT OR Apache-2.0`**: `license.workspace = true` in its
  `Cargo.toml`, and its `.rs` files carry **no** copyright/SPDX header -- match
  `crates/trace-commons-protocol/src/canonical_json.rs`, which opens directly
  with `//!` docs. Do not copy the two-line AGPL header from the files you move;
  removing it is part of moving them.
- **Permissive code may flow into AGPL crates. Never the reverse.** The new
  crate must not depend on `trace-commons-server`, `-gate-api` or
  `-gate-enclave`. If `tests/license_boundary.rs` fails, remove the dependency;
  do not edit the expected sets to match the diff, except for the one deliberate
  addition this plan calls for in Task 1.
- **Behaviour-preserving.** Tasks 1-3 are moves. Every existing test must keep
  passing without being rewritten to match new behaviour. If a test needs
  changing beyond an import path or a re-export, stop and report it.
- **Every negative assertion names its specific error variant.** Not
  `assert!(x.is_err())`. Eighteen assertions on this project turned out
  structurally incapable of failing.
- **Mutation-check every guard.** Break the thing it protects, watch it go red,
  revert, report what the failure looked like. A check nobody has seen fail is
  not yet a check.
- No emojis. Short imperative commit subjects, no `feat:`/`fix:` prefixes.
- Verify with `RUSTFLAGS="-D warnings" cargo test --workspace` and **confirm the
  run terminated** -- a failure aborts it and leaves a plausible but truncated
  pass count. Plain `cargo check` does not apply `-D warnings`; CI does.
- Clippy with the repo allow-list, and `cargo fmt --all`:
  `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`

## File structure

- **Create** `crates/trace-commons-attestation/Cargo.toml`
- **Create** `crates/trace-commons-attestation/src/lib.rs` -- module root
- **Create** `crates/trace-commons-attestation/src/receipt.rs` (Task 1, moved)
- **Create** `crates/trace-commons-attestation/src/quote.rs` (Task 2, moved)
- **Create** `crates/trace-commons-attestation/src/measurements.rs` (Task 3, moved)
- **Modify** `Cargo.toml` (workspace members), `crates/trace-commons-server/Cargo.toml`
- **Modify** `crates/trace-commons-server/src/near_attestation/mod.rs` -- re-exports
- **Modify** `crates/trace-commons-server/tests/license_boundary.rs` -- permissive list
- **Modify** `.github/workflows/ci.yml` -- permissive-standalone job

---

### Task 1: The crate, and EIP-191 recovery

**Files:**
- Create: `crates/trace-commons-attestation/Cargo.toml`, `src/lib.rs`, `src/receipt.rs`
- Modify: `Cargo.toml`, `crates/trace-commons-server/Cargo.toml`,
  `crates/trace-commons-server/src/near_attestation/mod.rs`,
  `crates/trace-commons-server/tests/license_boundary.rs`, `.github/workflows/ci.yml`
- Delete: `crates/trace-commons-server/src/near_attestation/receipt.rs`

**Interfaces:**
- Produces: `trace_commons_attestation::receipt::{ReceiptPayload, ReceiptVerdict,
  ReceiptError, verify_receipt, recover_eip191_signer, decode_address}`

`receipt.rs` is 607 lines and its only non-std imports are `k256`, `sha2` and
`sha3` -- no server types. It moves whole. Start here because it proves the
crate, the manifest, the licence boundary and the CI wiring on the file least
likely to fight back.

**Two existing call sites break and must be updated**, both in the module that
shipped in #533:

- `crates/trace-commons-server/src/redaction_witness/certificate.rs:53` --
  `use crate::near_attestation::receipt::{ReceiptError, decode_address, recover_eip191_signer};`
- `crates/trace-commons-server/src/redaction_witness/verification.rs:57` --
  `use crate::near_attestation::receipt::decode_address;`

Keep them working by re-exporting from `near_attestation/mod.rs` rather than
editing every consumer:

```rust
pub use trace_commons_attestation::receipt;
```

- [ ] **Step 1: Create the crate and add it to the workspace**

`crates/trace-commons-attestation/Cargo.toml`:

```toml
[package]
name = "trace-commons-attestation"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
k256 = "0.13"
sha2 = "0.10"
sha3 = "0.10"
```

Add `"crates/trace-commons-attestation"` to the workspace `members` list in the
root `Cargo.toml`, and add
`trace-commons-attestation = { path = "../trace-commons-attestation" }` to
`crates/trace-commons-server/Cargo.toml`.

- [ ] **Step 2: Move the file**

`git mv crates/trace-commons-server/src/near_attestation/receipt.rs crates/trace-commons-attestation/src/receipt.rs`,
declare `pub mod receipt;` in `src/lib.rs`, **strip the two-line AGPL header**,
and fix any `use super::` or `use crate::` paths the move breaks.

- [ ] **Step 3: Add the licence-boundary entry**

In `crates/trace-commons-server/tests/license_boundary.rs`, add
`"trace-commons-attestation"` to the permissive list that begins with
`"trace-commons-protocol"` (around line 239). Leave the AGPL set -- currently
`trace-commons-gate-api`, `trace-commons-gate-enclave`, `trace-commons-server` --
unchanged. This is the one edit to the expected sets this plan authorises.

- [ ] **Step 4: Add the standalone CI check**

In `.github/workflows/ci.yml`, in the `cargo-check-permissive-standalone` job,
add alongside the existing lines (around line 157):

```yaml
      - run: cargo check -p trace-commons-attestation --no-default-features
```

This job is the only thing that builds a permissive crate the way an outside
consumer gets it. A crate not listed here is not covered.

- [ ] **Step 5: Run the suite** -- `RUSTFLAGS="-D warnings" cargo test --workspace`.
  Every previously passing test must still pass, unchanged.

- [ ] **Step 6: Mutation-check the boundary**

Add `trace-commons-server = { path = "../trace-commons-server" }` to the new
crate's dependencies and run `cargo test -p trace-commons-server --test license_boundary`.
Confirm it fails naming the violation, then revert. Report the failure text. The
boundary test is the specification; a boundary nobody has seen refuse is not yet
a boundary.

- [ ] **Step 7: Commit**

---

### Task 2: Quote verification, and the `dcap-qvl` dependency

**Files:**
- Create: `crates/trace-commons-attestation/src/quote.rs`
- Modify: `crates/trace-commons-attestation/Cargo.toml`,
  `crates/trace-commons-server/Cargo.toml`,
  `crates/trace-commons-server/src/near_attestation/mod.rs`
- Delete: `crates/trace-commons-server/src/near_attestation/quote.rs`

**Interfaces:**
- Consumes: the crate from Task 1
- Produces: `trace_commons_attestation::quote::{Collateral, QuoteVerifyError,
  VerifiedQuote, parse_collateral, verify_quote}`

`quote.rs` is 318 lines; its only non-std imports are `dcap_qvl` and `sha2`.

**The dependency move is the substance of this task.** In
`crates/trace-commons-server/Cargo.toml`, `dcap-qvl` is declared at line 34 with
`default-features = false` and features `["std", "ring", "default-x509"]`, and
the feature `near-attestation-collateral = ["dcap-qvl/report"]` at line 117
forwards to its collateral client. Move the dependency to the new crate, and
forward the feature through it so the server's feature keeps working:

```toml
# crates/trace-commons-attestation/Cargo.toml
[features]
default = []
collateral-client = ["dcap-qvl/report"]
```

```toml
# crates/trace-commons-server/Cargo.toml
near-attestation-collateral = ["trace-commons-attestation/collateral-client"]
```

Preserve the existing comments explaining why `report` and `rustcrypto` are off
and why `danger-allow-tcb-override` must stay off -- they are the reasoning, not
decoration.

- [ ] **Step 1: Move the file and the dependency**, re-export
  `pub use trace_commons_attestation::quote;` from `near_attestation/mod.rs`.
- [ ] **Step 2: Run the suite**, plus the feature build CI exercises:
  `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --features near-attestation-collateral`
  and `cargo check -p trace-commons-attestation --no-default-features`.
- [ ] **Step 3: Re-audit licences**

`dcap-qvl` is Phala's crate and it now sits in a permissive crate's manifest,
where its licence must be compatible with `MIT OR Apache-2.0` distribution
rather than only with an AGPL work. Run:

```bash
cargo deny check licenses
cargo deny check licenses --features near-ai-scorer
cargo deny check licenses --features local-gpu-models
```

Report the actual output. If `deny.toml` needs a change, **stop and report it
rather than editing** -- a licence exception is the user's call.

- [ ] **Step 4: Mutation-check the feature forwarding**

Build the server with `--features near-attestation-collateral` and confirm the
drill's collateral path is compiled in rather than refusing with the missing
control `near_ai_attestation_collateral_client`. Then build without the feature
and confirm it refuses with that exact control name. Report both.

- [ ] **Step 5: Commit**

---

### Task 3: Measurements, and the NEAR-specific seam

**Files:**
- Create: `crates/trace-commons-attestation/src/measurements.rs`
- Modify: `crates/trace-commons-server/src/near_attestation/mod.rs`
- Delete: `crates/trace-commons-server/src/near_attestation/measurements.rs`

**Interfaces:**
- Consumes: `quote::VerifiedQuote` (Task 2)
- Produces: `trace_commons_attestation::measurements::{MeasurementField,
  ExpectedMeasurements, ExpectedMeasurementsError, MeasurementMismatch,
  MeasurementVerdict, check_measurements, check_measurements_opt}`

**This is the only file with a real coupling, and it is the decision this task
encodes.** `measurements.rs:40` reads `use super::UnverifiedJsonMeasurements` --
a type describing NEAR AI's JSON attestation envelope, which has nothing to do
with TDX. `json_claim_anomalies` (line 417) and `JsonClaimAnomaly` (line 390)
exist to compare that envelope's self-reported claims against the verified
quote.

Split on that line:

- **Move** the measurement types, `check_measurements` and
  `check_measurements_opt` -- they consume `VerifiedQuote` and nothing else.
- **Keep in the server** `JsonClaimAnomaly`, `json_claim_anomalies`, and the env
  constants `EXPECTED_MEASUREMENTS_ENV`
  (`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`) and
  `EXPECTED_MEASUREMENTS_CONTROL` (`near_ai_expected_measurements`). Their names
  say NEAR AI; a witness will need its own, and moving them would create a
  generic-looking constant that is not generic.

`check_measurements` must keep refusing `UnverifiedJsonMeasurements` -- it takes
a `&VerifiedQuote`, and that is what makes an unverified measurement
inexpressible. Do not add a convenience overload that accepts the JSON type.

- [ ] **Step 1: Split and move**, re-export from `near_attestation/mod.rs` so
  existing paths keep resolving.
- [ ] **Step 2: Run the suite.** The measurement tests move with the code; the
  JSON-anomaly tests stay in the server.
- [ ] **Step 3: Mutation-check the type seam**

Write a throwaway test that tries to pass an `UnverifiedJsonMeasurements` where
`check_measurements` wants a `&VerifiedQuote`, confirm it does not compile, and
report the compiler error. Delete the test. A verified-only signature that
nobody has seen reject an unverified value is an assumption.

- [ ] **Step 4: Commit**

---

### Task 4: Pin `MRCONFIGID`

**Files:**
- Modify: `crates/trace-commons-attestation/src/quote.rs`,
  `crates/trace-commons-attestation/src/measurements.rs`

**Interfaces:**
- Produces: `VerifiedQuote.mr_config_id: String`, `MeasurementField::MrConfigId`

**This is the whole point of the plan.** `dcap-qvl` 0.6.3 exposes
`pub mr_config_id: [u8; 48]` on the TD10 report (`src/quote.rs:252` in that
crate). Copy it into `VerifiedQuote` beside the existing `mrtd` and `rtmr0..3`,
hex-encoded the same way, and add the matching `MeasurementField` variant so an
operator can pin it.

Do not remove the RTMR fields. RTMR0-2 stay pinnable, and an operator may still
want them; what changes is which field the witness deployment pins. Record in
the doc comment on the new variant why it is the stable one and why RTMR3 is
not:

- RTMR3 is extended with an `instance-id` seeded from `getrandom` per
  deployment, so it differs between two instances of identical code.
- RTMR0 hashes SMBIOS tables that scale with `-m` and `-cpu`, so it changes when
  the VM is resized without any code change.
- `MRCONFIGID` (V2) commits to the compose hash, the 20-byte app id and the
  key-provider identity.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn a_verified_quote_carries_mr_config_id() {
    // The field exists on the parsed report and we never copied it out.
    // Fixture: the live NEAR AI triple already in the test fixtures.
    let quote = verified_quote_from_fixture();
    assert_eq!(quote.mr_config_id.len(), 96, "48 bytes, hex-encoded");
}

#[test]
fn a_pinned_mr_config_id_that_does_not_match_is_refused() {
    // Name the variant. Not is_err().
}

#[test]
fn mr_config_id_parses_as_a_measurement_field() {
    assert_eq!(
        "mrconfigid".parse::<MeasurementField>().unwrap(),
        MeasurementField::MrConfigId
    );
}
```

Use the existing fixture at
`crates/trace-commons-server/tests/fixtures/near_ai_live_triple.json` if the
moved tests already reach it; if the new crate cannot, construct the quote the
same way the existing `quote.rs` tests do. **Do not invent a fixture path** --
two earlier briefs on this project named test fixtures that did not exist.

- [ ] **Step 2: Run to verify they fail**
- [ ] **Step 3: Implement**
- [ ] **Step 4: Run to verify they pass**
- [ ] **Step 5: Mutation-check**

Point `mr_config_id` at the `mrtd` bytes instead. The parse test will still
pass; confirm the mismatch test goes red. If it does not, the mismatch test is
not observing what its name claims -- fix the test, and say so. On this project
a digest comparison passed its entire suite under a `starts_with` mutation
because the test meant to catch it truncated the wrong input.

- [ ] **Step 6: Commit**

---

## Not in this plan

- The witness service itself, its dstack packaging, and its nonce-bound quote
  route. Blocked on nothing in this plan, but a separate slice.
- Any client-side verification. This plan makes it *possible*; it does not build
  it. `client.rs` (501 lines) stays in the server -- it is coupled to server
  config and HTTP, and the spec's recommendation is to rewrite it on
  `trace_commons_operator_client::Client` rather than move it.
- `drill.rs` (1,395 lines) stays in the server. It is the NEAR AI admin drill,
  not generic verification.
- Changing what the pilot pins. Task 4 makes `MRCONFIGID` pinnable; deciding to
  pin it in production is an operator change, not a code change.
