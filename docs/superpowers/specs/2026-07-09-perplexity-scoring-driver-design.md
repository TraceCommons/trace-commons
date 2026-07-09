# Perplexity Scoring Driver — Design

Date: 2026-07-09
Status: Implemented

## Purpose

The pilot's perplexity gate (NEAR AI Qwen3.6-35B, `GATE_SERVICE=enclave_near_ai`)
is fully configured but **never runs**: the enclave gate is exposed only as an
async worker endpoint (`POST /v1/workers/gate/evaluate`, one submission at a
time, EdDSA vector-operator auth), and nothing on the pilot triggers it. As a
result `trace_gate_decisions` is empty — no submitted trace has ever been
perplexity-scored. The lightweight embedding "duplicate_precheck" runs at
submit time and produces the novelty/duplicate numbers currently visible; the
expensive 35B perplexity pass does not.

This slice builds the missing driver: an in-process background loop in
`trace-commons-ingest` that finds submissions lacking a gate decision, runs the
enclave gate per submission (recording perplexity + novelty), and records the
decision. It makes perplexity scoring actually happen on ongoing traffic and
backfills the existing submissions.

The gate **floor stays 0** (non-gating): perplexity is recorded, never used to
reject. Setting a calibrated floor to enable gating is an explicitly separate,
out-of-scope step (see Non-goals).

## Decisions (settled during brainstorming)

| Question | Decision |
|---|---|
| Where it runs | In-process scheduled loop inside `trace-commons-ingest`, following the existing scheduler-worker pattern. No HTTP, no worker token. |
| Shared logic | The HTTP worker handler's core (fetch → decrypt → evaluate → record) is extracted into `evaluate_and_record_gate(...)`, used by both the handler and the loop. |
| Failure mode | Fail-safe: a scoring error records nothing, bumps an attempt counter, backs off, and retries next tick. Never rejects or re-quarantines. Bounded max-attempts. |
| Pacing | Small batch per tick (default 5), scored sequentially; interval default 45s. Batch size and interval are a throughput knob, env-configurable. |
| Cost controls | Skip the 35B pass on precheck-flagged near-duplicates (record `skipped_duplicate`); idempotent score cache keyed on canonical trace hash. Both toggleable. Token cap per trace already exists (`max_tokens`/`tail_cutoff`). |
| Calibration | Out of scope. Floor calibration is an offline bakeoff activity (a27 recipe); skip-duplicates on live traffic does not undermine it. |

## Architecture

New in-process loop `perplexity_score_driver`, spawned at ingest startup when
enabled, modeled on the existing interval schedulers in
`trace-commons-ingest.rs`. It shares one evaluation path with the HTTP worker:

```
evaluate_and_record_gate(state, tenant_id, submission_id) -> Result<GateOutcome>
```

extracted from `gate_evaluate_worker_handler` so the handler and the loop call
identical logic. `GateOutcome` distinguishes `Scored`, `SkippedDuplicate`,
`Cached`, and `Failed(label)` for the loop's bookkeeping and the handler's
response.

## Config (all env; driver OFF by default — no behavior change for other deployments)

- `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` — default `false`. The pilot sets `1`.
- `TRACE_COMMONS_PERPLEXITY_DRIVER_INTERVAL_SECONDS` — default `45`.
- `TRACE_COMMONS_PERPLEXITY_DRIVER_BATCH_SIZE` — default `5`.
- `TRACE_COMMONS_PERPLEXITY_DRIVER_MAX_ATTEMPTS` — default `5`.
- `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATES` — default `true`.
- `TRACE_COMMONS_PERPLEXITY_DRIVER_SKIP_DUPLICATE_THRESHOLD_MICROS` — default
  `900000` (0.9). A submission whose precheck `duplicate_score` (micros) is
  at/above this is skipped when `SKIP_DUPLICATES` is on.

## Finding work (the ungated set)

New store method:

```
list_submissions_needing_gate_decision(limit: i64)
    -> Result<Vec<GateWorkItem>>   // { tenant_id, submission_id }
```

Cross-tenant enumeration query returning submissions that (a) have an active
`SubmittedEnvelope` object ref, (b) have **no** row in `trace_gate_decisions`,
(c) whose attempt counter is below `MAX_ATTEMPTS` and whose backoff has elapsed,
ordered oldest-first, limited to `limit`.

Correction (2026-07-09 research): the repo has NO shared RLS-bypass pool and the
existing schedulers operate per-tenant, not cross-tenant. So this slice adds a
dedicated **`trace_gate_driver`** Postgres role (NOLOGIN NOBYPASSRLS) with a
role-scoped **permissive** cross-tenant SELECT policy on the four tables the
enumeration reads (`trace_submissions`, `trace_gate_decisions`,
`trace_object_refs`, `trace_gate_evaluation_attempts`), following the
`trace_login_resolver` pattern already in the repo (V30/V32/V33). The driver
uses a dedicated pool from `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` connected as
that role. Because the role stays `NOBYPASSRLS`, the permissive `USING (true)`
policy (not BYPASSRLS) is what authorizes the read; a superuser test connection
hides the policy gap, so the PG-gated test must connect/`SET ROLE` as
`trace_gate_driver` to exercise it. Only ENUMERATION is cross-tenant; all
per-submission work (decrypt, score, insert decision, bump attempts) runs
per-tenant via the normal `db_mirror` once the tenant id is known.

Backfill is automatic: on the first enabled tick the existing submissions are
simply the oldest members of this set.

**Attempt bookkeeping** lives in a new small table, isolated from
`trace_submissions`:

```sql
CREATE TABLE trace_gate_evaluation_attempts (
    tenant_id     TEXT NOT NULL,
    submission_id UUID NOT NULL,
    attempts      INT  NOT NULL DEFAULT 0,
    last_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_error_label TEXT,
    PRIMARY KEY (tenant_id, submission_id)
);
```

RLS-forced like every Trace Commons table, with the driver-role policy. Backoff
is `last_attempt_at + f(attempts)` (exponential, capped). A submission that
reaches `MAX_ATTEMPTS` drops out of the set and is logged (hash-only) as
`gate_scoring_exhausted`.

## Score cache (idempotent re-scoring)

Keyed on the canonical trace hash already present on the submission record. A
content-identical resubmission (same hash) reuses the recorded gate decision
rather than paying for a second 35B pass. Implemented by, before scoring, looking
up an existing decision for the same canonical hash within the tenant; on hit,
copy its perplexity/novelty values into a new decision row for this submission
(so each submission still has its own decision row) marked `Cached`.

## Data flow per tick

1. `list_submissions_needing_gate_decision(BATCH_SIZE)`.
2. For each item, sequentially: `evaluate_and_record_gate`:
   - If `SKIP_DUPLICATES` and the submission's precheck duplicate score is at/above
     the duplicate threshold → record a `skipped_duplicate` gate decision (no 35B
     call), outcome `SkippedDuplicate`.
   - Else, cache lookup by canonical trace hash → on hit, record a `Cached`
     decision from the cached values (no 35B call).
   - Else, decrypt the submitted envelope via the artifact store → `gate_service.evaluate`
     (35B perplexity + local embedding novelty) → insert `StorageTraceGateDecisionRow`
     (floor 0 ⇒ `perplexity_passed = true`; the perplexity/novelty micros are recorded),
     outcome `Scored`.
   - On any error → bump attempt counter, set `last_error_label`, leave the
     submission ungated; outcome `Failed`.
3. Sleep `INTERVAL_SECONDS`.

The loop never opens a long transaction; each submission is its own unit of work.
Submission accept/quarantine status (set at submit time) is never modified by the
driver.

## Error handling & safety

- **Fail-safe, not fail-closed:** a scoring failure records nothing, retries with
  backoff, and never rejects or re-quarantines. (The gate's own `evaluate` is
  fail-closed *for gating*; here, because the floor is 0 and we are recording not
  gating, a failure must not change a submission's fate.)
- **Hash-only logging:** submission-id hash, tenant hash, attempt count, and a
  fixed error label only. Never the trace content, the envelope, or the NEAR AI
  response body.
- **Bounded work:** `BATCH_SIZE` per tick and `MAX_ATTEMPTS` per submission cap
  both burst load and infinite retries.
- **Disabled by default:** other deployments and CI are unaffected; the driver is
  inert unless `..._ENABLED` is set.

## Testing

- **Unit (CI):**
  - `list_submissions_needing_gate_decision` returns only submissions lacking a
    decision, respects the attempt cap and backoff, oldest-first, limit honored.
  - Duplicate-skip branch records `skipped_duplicate` without invoking the scorer.
  - Cache branch reuses values by canonical hash without invoking the scorer.
  - `evaluate_and_record_gate` with a mock gate service records a decision with
    `perplexity_passed=true` at floor 0.
  - Attempt bookkeeping: a mock-scorer failure bumps attempts + sets the error
    label and leaves the submission ungated; at `MAX_ATTEMPTS` it exits the set.
- **Integration (CI):** the driver loop with an in-memory store + mock scorer
  drains a backlog of 3 submissions and then idles (no further scorer calls). No
  live NEAR AI in tests.
- **PG-gated:** the ungated-set query under the driver role via `SET ROLE`
  (following the RLS-resolver test convention), plus the new table's RLS.
- **Manual (pilot):** enable the flag, restart ingest, watch the two existing
  submissions get real 35B perplexity scores recorded in `trace_gate_decisions`,
  verify via the tenant-scoped query.

## Non-goals (explicitly out of scope)

- **Setting a gate floor / enabling gating.** The floor stays 0. Calibrating and
  setting `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` is a separate offline
  bakeoff step (a27 runbook).
- **Changing the HTTP worker endpoint's behavior** beyond extracting the shared
  `evaluate_and_record_gate` fn (byte-for-byte behavior preserved).
- **Re-scoring already-scored submissions** (idempotent: a submission with a
  decision is never re-processed).
- **Concurrency / drain-all pacing.** Sequential small batches only for v1.
