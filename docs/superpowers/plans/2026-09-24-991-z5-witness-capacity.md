# #991 Z5 Witness Capacity and Pacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Keep automatic contribution backlogs from occupying every witness slot, and publish a stable retry contract so clients can pace without losing a session.

**Architecture:** Retain the witness's existing global semaphore, immediate `503 witness_saturated` refusal, `Retry-After`, body-read coverage, and request timeout. Add a second semaphore for requests that explicitly identify themselves as background; background work needs both permits, while ordinary interactive requests need only the global permit. Publish the header and refusal constants in the permissive protocol crate for Kristi's client/daemon work; this is cooperative capacity partitioning, not an authentication or abuse-control boundary.

**Tech Stack:** Rust, Axum middleware, Tokio semaphores, existing protocol crate. No new dependencies, identity store, request log, or database migration.

**Spec:** `docs/superpowers/plans/2026-09-24-991-zaki-server.md` (Z5 and global constraints); `docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md` (Open: witness capacity and back-pressure); `deploy/witness/README.md` (current bounds).

## Global Constraints

- Preserve anonymous witness operation. Do not send an account, invite, session ID, or transcript-derived value in the workload header or logs.
- The global `TRACE_COMMONS_WITNESS_MAX_CONCURRENT_REQUESTS` bound (default 4), immediate saturation response, and `TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS` (default 300) already exist. No waiting queue is allowed inside the witness.
- Reserve interactive capacity from **cooperating background clients**. Because all witness POST routes are unauthenticated and legacy interactive/automatic calls use the same routes, a caller can omit or forge the workload header; neither this header nor its reservation proves trust or stops an abusive caller. The global bound remains authoritative for all callers.
- Absent workload header means legacy/interactive behavior so deployed clients continue to work. Only the exact background value opts into the background budget; reject malformed or unknown values with a fixed 400 label before body reading.
- The numeric reservation is operator capacity configuration, not an invite trust or consent policy. Client choice of automatic versus interactive is a product/client decision; coordinate the client change with Kristi and do not claim end-to-end pacing before it lands.
- Saturation must carry no certificate or signature, and a fixed `Retry-After` in seconds. The client must retain the pending session, use bounded jittered retry, and show a delayed state; this plan only publishes the server contract and testable server behavior.
- New server `.rs` files require AGPL headers. New protocol `.rs` files remain MIT/Apache and must never depend on AGPL crates.

## Review Focus

1. A background request can use the last nonreserved slot but cannot occupy the reserved slot; an interactive request can then complete.
2. Background permits release after success, refusal, timeout, and cancellation; no slot leak or unbounded waiter list remains.
3. Every expensive witness POST route uses the same admission middleware, including token bundle and admission paths; attestation remains reachable.
4. Missing, unknown, duplicated, or non-UTF8 workload headers cannot create an unbounded or misclassified path.
5. A retryable saturation response contains only the fixed label and delay, never certificate bytes, raw content, or request-derived telemetry.

## Client handoff for Kristi

Only unattended automatic witness POSTs should send
`x-trace-witness-workload: background`. Interactive previews leave the
header absent. Preserve the `VerifiedWitness` pin and existing request bytes.
On the exact `503 witness_saturated` status and error pair, parse a bounded
integer `Retry-After`, add jitter, cap attempts, retain the queued or held
session, and show a delayed state. A policy `403`, timeout `504`, or malformed
header `400` is not retryable saturation. This client work is dependent and
is not implemented in this server slice. Server reservation protects capacity
only after automatic clients adopt the header; the caller-declared header
cannot prioritize against an adversarial anonymous request. The global bound
remains the spend cap, and no backlog pacing or UI completion is claimed here.

---

### Task 1: Publish the small shared pacing contract

**Files:**
- Create: `crates/trace-commons-protocol/src/witness_pacing.rs` (permissive header-free license style).
- Modify: `crates/trace-commons-protocol/src/lib.rs`.
- Test: module unit tests in the new file.

**Interfaces:**
- Produce `pub const WITNESS_WORKLOAD_HEADER: &str = "x-trace-witness-workload"`, `pub const WITNESS_BACKGROUND_WORKLOAD: &str = "background"`, `pub const WITNESS_SATURATED_ERROR: &str = "witness_saturated"`, and `pub const WITNESS_SATURATED_RETRY_AFTER_SECS: u32 = 30`. Define `pub fn is_witness_saturation(status: u16, error: &str) -> bool` as exact `(503, "witness_saturated")` matching, so a client never retries a 403 policy refusal just because it got an error body.
- Preserve existing witness request and certificate wire shapes. A background client adds one header; the response remains `503 {"error":"witness_saturated"}` with `Retry-After: 30`.

- [x] **Step 1: Write literal contract tests** for all four exported values and the exact status/error pair; assert 403, 504, and near-miss labels are false. These pin the wire values a separately released client will consume.
- [x] **Step 2: Run** `RUSTFLAGS='-D warnings' cargo test -p trace-commons-protocol --lib witness_pacing`; expect the new module/function to be absent.
- [x] **Step 3: Add the module and export it** from `lib.rs`. Use only standard Rust; do not import server types.

  ```rust
  pub const WITNESS_WORKLOAD_HEADER: &str = "x-trace-witness-workload";
  pub const WITNESS_BACKGROUND_WORKLOAD: &str = "background";
  pub const WITNESS_SATURATED_ERROR: &str = "witness_saturated";
  pub const WITNESS_SATURATED_RETRY_AFTER_SECS: u32 = 30;

  pub fn is_witness_saturation(status: u16, error: &str) -> bool {
      status == 503 && error == WITNESS_SATURATED_ERROR
  }
  ```
- [x] **Step 4: Re-run** the focused protocol test for client coordination; commit with the Z5 slice.

### Task 2: Reserve capacity for declared background requests

**Files:**
- Modify/test: `crates/trace-commons-server/src/witness_service/http.rs`.

**Interfaces:**
- Consume Task 1 constants. Extend `WitnessLoadBound` with an `Arc<Semaphore>` for background work and a constructor/configuration method that accepts `max_concurrent_requests`, `reserved_interactive_slots`, and `request_timeout`. Preserve `WitnessLoadBound::new(max, timeout)` for existing tests/callers; have it use one reserved slot when `max > 1` and zero when `max == 1`, or add a named constructor for the binary's explicit setting.
- Define `pub fn with_reservation(max_concurrent_requests: usize, reserved_interactive_slots: usize, request_timeout: Duration) -> Result<Self, &'static str>`. Require `max_concurrent_requests > 0` and `reserved_interactive_slots < max_concurrent_requests`; zero reservation is valid for operators explicitly preserving the former shared-pool policy. The background semaphore size is `max_concurrent_requests - reserved_interactive_slots`. Have the existing `new(max, timeout)` delegate with the default `usize::from(max > 1)` so existing tests and one-slot users retain a working path.

- [x] **Step 1: Add router tests** using `ParkingRedactor`: with global 2 and reserved 1, hold one background request, assert a second background request receives the exact 503 label plus `Retry-After: 30` and no cert/signature, then assert an ordinary POST reaches the redactor while the first is held. Test each POST path (`/v1/witness`, `/v1/witness/token-bundle`, `/v1/witness/admission`) for middleware coverage with a cheap saturation probe.
- [x] **Step 2: Add malformed-header tests** for unknown, duplicate, and non-UTF8 values, expecting HTTP 400 with a fixed `witness_workload_malformed` label and no certificate. Add a missing-header positive control and a full-global-pool refusal test so the reservation never bypasses the global bound.
- [x] **Step 3: Run** `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --lib witness_service::http::tests`; expect the new reservation tests to fail against the current single semaphore.
- [x] **Step 4: In `bound_witness_load`, parse the workload header before reading the body.** For exact `background`, acquire a background permit with `try_acquire_owned` and then the global permit with `try_acquire_owned`; for absent, acquire only the global permit. Return the same fixed saturation refusal if either permit is unavailable. Keep both owned permits alive through `tokio::time::timeout(..., next.run(request))` so body reading and classifier work stay bounded; permit drops handle timeout and cancellation. Use shared protocol constants for the header and saturation response.

  ```rust
  let background = match request.headers().get_all(WITNESS_WORKLOAD_HEADER).iter().collect::<Vec<_>>().as_slice() {
      [] => false,
      [value] if value.as_bytes() == WITNESS_BACKGROUND_WORKLOAD.as_bytes() => true,
      _ => return Refusal::new(StatusCode::BAD_REQUEST, "witness_workload_malformed").into_response(),
  };
  let _background_permit = if background {
      match Arc::clone(&load.background_permits).try_acquire_owned() {
          Ok(permit) => Some(permit),
          Err(_) => return saturated_refusal(),
      }
  } else {
      None
  };
  let Ok(_global_permit) = Arc::clone(&load.permits).try_acquire_owned() else {
      return saturated_refusal();
  };
  // Keep both guards in scope around the existing timeout/next.run block.
  ```
- [x] **Step 5: Re-run** focused tests, including the existing `a_second_request_at_the_bound_is_refused_rather_than_queued`, timeout-recovery, sequential-slot, and attestation-availability tests. Commit after review.

### Task 3: Wire operator capacity settings and document rollout

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-witness.rs`.
- Modify: `deploy/witness/README.md`.

**Interfaces:**
- Add `TRACE_COMMONS_WITNESS_RESERVED_INTERACTIVE_SLOTS`/`--reserved-interactive-slots` as `Option<usize>`. When absent, select 1 for `max_concurrent_requests > 1` and 0 for a one-slot witness; when present, use the explicit value. `with_reservation` rejects `reserved >= max` before listener bind. Never silently increase the global limit.

- [x] **Step 1: Add binary unit tests** for a pure `resolved_reserved_interactive_slots(max: usize, configured: Option<usize>) -> Result<usize, &'static str>` helper: `(4,None) -> 1`, `(4,Some(0)) -> 0`, `(1,None) -> 0`, `(1,Some(1)) -> Err`, `(4,Some(4)) -> Err`, and `(0,None) -> Err`.
- [ ] **Step 2: Run** `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --bin trace-commons-witness reserved_interactive`; the initial RED command was interrupted to release the shared Cargo lock, so a missing-helper failure was not observed.
- [x] **Step 3: Resolve the optional setting and pass it into `WitnessLoadBound::with_reservation`**. Keep the existing global semaphore and timeout values unchanged. Update the operator table to explain the background budget, default, one-slot behavior, no internal queue, and exact 503/Retry-After contract.

  ```rust
  let reserved = resolved_reserved_interactive_slots(
      args.max_concurrent_requests,
      args.reserved_interactive_slots,
  )?;
  let load = WitnessLoadBound::with_reservation(
      args.max_concurrent_requests,
      reserved,
      Duration::from_secs(args.request_timeout_secs),
  )?;
  ```
- [x] **Step 4: Re-run** the focused binary tests and `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bin trace-commons-witness`. Commit after review.

### Task 4: Hand off client pacing and verify the server slice

**Files:**
- Modify: `docs/superpowers/plans/2026-09-24-991-zaki-server.md` execution record or a short Z5 handoff note in the Z5 plan after implementation.

- [x] Document for Kristi the exact client work: mark only unattended automatic witness POSTs with `x-trace-witness-workload: background`; preserve the `VerifiedWitness` pin and existing request bytes; on exact `503 witness_saturated`, parse a bounded integer `Retry-After`, add jitter and cap retries, retain the queued/held session, and show a delayed state. Interactive previews leave the header absent. Do not reinterpret policy 403 or malformed 400 as retryable saturation. This is a dependent client change, not implemented in the server slice.
- [x] Run `cargo fmt --all -- --check`, `git diff --check`, `RUSTFLAGS='-D warnings' cargo test -p trace-commons-protocol --lib witness_pacing`, and `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --lib witness_service::http::tests`.
- [x] Run `RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --test license_boundary` and the witness binary check. Record all pass/fail results. No dependency-license rerun is needed if manifests are unchanged.
- [x] State the deployment limit accurately: server-side reservation begins protecting capacity only after automatic clients send the background header. The header is caller-declared and cannot establish priority against an adversarial anonymous request; the global bound remains the spend cap. Do not claim backlog pacing or UI completion until the client change and its tests are present.
