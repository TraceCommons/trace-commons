# Server-side NEAR AI PII redaction backstop — design

## Goal

Add a **defense-in-depth** server-side pass that runs the NEAR AI privacy
classifier over the message text of already-ingested trace envelopes, re-redacts
any residual prose PII (names, emails, phones, addresses, locations) the client
missed, and **holds the trace out of consumer reach until the pass completes**.

Client-side redaction stays primary. This is a backstop for:

- Clients whose PII pass failed open (e.g. Ironclaw falls back to
  deterministic-only redaction when its privacy-filter sidecar errors —
  `crates/ironclaw_reborn_traces/src/contribution.rs` `apply_privacy_filter_to_text`).
- Clients that never ran a prose-PII pass at all (deterministic-only uploads).
- Prose PII that pattern-based deterministic redaction structurally cannot catch.

## Non-goals

- **Not** a replacement for client-side redaction. Only the redacted envelope
  still leaves the contributor's machine; this operates on what already arrived.
- **Not** a change to the deterministic rescrub (`rescrub_trace_envelope`), which
  keeps running synchronously at ingest as today.
- **Not** applied to structured tool payloads — those are covered by the
  deterministic pass only. The NEAR AI pass sees message text (`content` /
  `human_correction`) alone, matching the client contributor CLI's behavior.
- **Not** a synchronous ingest-path change. Ingest latency and availability must
  not depend on NEAR AI.

## Threat model note

This sends server-held message text to the NEAR AI Cloud classifier. That
server→NEAR AI text-exposure relationship **already exists**: the perplexity gate
scorer (`TRACE_COMMONS_GATE_SERVICE=enclave_near_ai`) already sends trace text to
NEAR AI for scoring. The backstop reuses that existing TEE-hosted trust boundary
rather than opening a new one. No raw PII, span text, or message bodies ever
reach audit rows or logs (hash/label-only, per repo convention).

## Architecture

Fold a new **backstop stage** into the existing in-process driver task that
already runs perplexity scoring, rather than standing up a separate binary:

- `spawn_perplexity_score_driver_task` (bin/trace-commons-ingest.rs:8072) — the
  spawned loop. Gains a sibling backstop tick.
- `run_perplexity_score_driver_tick` (…:35551) — the scoring tick. A parallel
  `run_pii_backstop_driver_tick` is added and called from the same loop, gated by
  its own enable flag and knobs.

The backstop reuses the same infrastructure the scoring driver uses: the
`tc_gate_driver`-style DB role and its permissive read policies, the
`FOR UPDATE SKIP LOCKED` work-claim pattern, and per-item attempt bookkeeping.

## Release mechanism — hold via status, not a new consumer predicate

Consumer visibility in this codebase is **not centrally gated**: export, benchmark,
ranker-training, process-evaluation, utility-credit, and DB-mirror paths each test
`status == TraceCorpusStatus::Accepted` inline (~20 call sites). Adding a separate
"backstop-done" predicate would require patching all of them, and any missed site
would *fail open* (release an un-backstopped trace).

Instead, the backstop **holds the trace in a non-`Accepted` status until it
completes**, so every existing `status == Accepted` check holds it by construction
(fail-closed if a site is missed). Concretely, add a new corpus status
`AwaitingPiiBackstop`:

- At ingest, a submission that would become `Accepted` (`status_for_risk` →
  `Accepted`) **and** has `consent.message_text_included = true` is instead stored
  as `AwaitingPiiBackstop` when the backstop is enabled. Submissions that ingest as
  `Quarantined` (risk-based) keep that status and the existing reviewer flow.
- The backstop driver processes `AwaitingPiiBackstop` submissions; on `done` it
  re-runs `status_for_risk` against the post-backstop risk and transitions to
  `Accepted` (or `Quarantined` if the backstop *raised* risk to Medium/High).
- `AwaitingPiiBackstop` is treated as "not consumer-visible" everywhere `Accepted`
  is the gate — no per-site predicate edits. Reviewer/quarantine surfaces are
  unaffected (they key on `Quarantined`).

## Data flow

1. **Ingest (behavior mostly unchanged):** deterministic rescrub → compute status
   via `status_for_risk`. If the result is `Accepted`, the envelope has
   `consent.message_text_included = true`, and the backstop is enabled, store as
   `AwaitingPiiBackstop` and enrol a `trace_pii_backstop` row as `pending`.
   Otherwise behave exactly as today (Low→Accepted, Medium/High→Quarantined).
2. **Backstop tick (async):** claim a batch of `pending` submissions → for each,
   load the stored (post-deterministic-rescrub) envelope via
   `read_envelope_by_record` → run the **chunked NEAR AI prose filter** over the
   redactable text fields (`events[*].redacted_content`, `outcome.human_correction`)
   using `NearAiPrivacyFilterAdapter` (the codepoint-safe, chunking, retrying
   adapter) → re-redact those fields, merge `redaction_counts` / `pii_labels_present`,
   bump `residual_pii_risk` monotonically, append `+near-ai-pii-backstop-v1` to
   `redaction_pipeline_version`, recompute `redaction_hash` → re-store via
   `store_envelope(.., "rescrubbed-envelope", ..)` + mirror a `RescrubbedEnvelope`
   object ref → transition status via `status_for_risk` (post-backstop risk) →
   mark `trace_pii_backstop = done`. This mirrors the reviewer-approve re-store flow
   (ingest.rs:33882-33892).
3. **Hold:** because the trace is `AwaitingPiiBackstop` until step 2 finishes, the
   existing `status == Accepted` gates hold it out of every consumer/export path
   automatically. On failure (below) it stays held.

Note: the backstop uses the **async prose-filter path**, not `rescrub_trace_envelope`
(which is synchronous and deterministic-only by design). A new protocol helper
`rescrub_envelope_prose_pii_with(adapter, envelope).await` performs step 2's
field-level re-redaction + metadata merge, analogous to `rescrub_trace_envelope_with`
but running the async privacy-filter adapter.

## Components (independently testable)

- **`pii_backstop` module** — work enumeration, the driver tick, attempt/backoff
  bookkeeping, and the re-redaction + metadata-update logic. Pure re-redaction
  (envelope + spans → rescrubbed envelope) is a separate unit from the DB/driver
  plumbing so it is testable without a DB or network.
- **State**: a dedicated `trace_pii_backstop` table (tenant_id, submission_id PK,
  `attempts`, `last_attempt_at`, `last_error_label`) — the driver's attempt
  bookkeeping, cloned from `trace_gate_evaluation_attempts` (V36). The *hold* state
  is the corpus `status = AwaitingPiiBackstop` on the submission itself, not a
  column here. Forced RLS + a permissive `FOR SELECT ... USING (true)` reader
  policy, mirroring V36.
- **Reader role**: a dedicated `trace_pii_backstop_driver` role (NOBYPASSRLS,
  `statement_timeout`, permissive SELECT policies on the tables it enumerates),
  minted the same way as `trace_gate_driver` in V36 — kept separate so the backstop
  does not widen the gate driver's grants. Its URL comes from
  `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL`.
- **Reused as-is**: the protocol NEAR AI adapter (with its once-per-cycle
  `run_privacy_filter_canary` self-test before processing real traces), the
  `store_envelope` + reviewer-approve re-store pattern, the `RescrubbedEnvelope`
  object-ref kind (already defined, currently unwritten), and the existing
  `status == Accepted` gates (held by `AwaitingPiiBackstop` without per-site edits).
- **Writes** (attempt bumps + status transition + re-store) go through the
  tenant-scoped runtime pool with tenant context, never the cross-tenant reader
  pool — matching `bump_gate_evaluation_attempt`.

## Fail posture

- **Fail-closed on release, never on ingest.** Ingest always succeeds (subject to
  its existing checks); the backstop runs afterward.
- NEAR AI transient failure → the adapter's own retry/backoff, then the driver's
  attempt bookkeeping re-queues the submission on the next tick.
- After `max_attempts`, the driver stops re-queuing: the trace stays
  `AwaitingPiiBackstop` (**held** — never reaches `Accepted`, so no consumer/export
  path sees it) and is surfaced to operators via the backstop attempts table
  (`last_error_label`, `attempts >= max`). It is never released with unredacted
  residual PII. An operator can re-drive it (reset attempts) once NEAR AI recovers.
- If the backstop is enabled but `TRACE_NEAR_AI_PRIVACY_API_KEY` is missing/blank,
  refuse at boot with a safe missing-control name (fail-closed configuration, per
  repo convention) rather than silently disabling the backstop.

## Configuration

Server crate enables the protocol `near-ai-privacy-filter` feature.

- `TRACE_COMMONS_PII_BACKSTOP_ENABLED` (default `0`).
- `TRACE_NEAR_AI_PRIVACY_API_KEY` (+ optional `TRACE_NEAR_AI_PRIVACY_BASE_URL` /
  `_MODEL` / `_TIMEOUT_MS` / `_MAX_INPUT_BYTES`) — read from env, never persisted.
- `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` (default 5),
  `..._BATCH_SIZE`, `..._TICK_INTERVAL_SECONDS`, `..._BACKOFF_BASE_SECONDS` —
  mirror the scoring driver knobs (`parse_optional_scheduler_*_env`).
- `TRACE_COMMONS_PII_BACKSTOP_DRIVER_DATABASE_URL` — the cross-tenant reader URL
  for the `trace_pii_backstop_driver` role (fail-closed: the driver stays off if
  unset even when `..._ENABLED=1`; boot refuses if enabled without it).
- Scope: `message_text_included` envelopes only.

## Audit / privacy

Hash-only. Rows and logs carry label counts, submission-id (as the existing
tables already do under RLS), attempt counts, and error **hashes** — never raw
PII, span text, or message bodies. The per-cycle canary guards against a
misconfigured/no-op filter silently passing traces through unredacted.

## Testing

- **Unit**: work enumeration (only `pending`, message-text envelopes claimed);
  pure re-redaction (stored envelope + synthetic spans → residual PII removed,
  metadata updated, pipeline suffix appended); attempt/backoff transitions.
- **Integration**: a stored envelope carrying residual prose PII → one driver tick
  → the rescrubbed envelope has it removed, `status = done`, and the release gate
  now admits it; a control envelope with no PII → `done`, unchanged text.
- **Fail path**: NEAR AI 5xx (wiremock) → retries exhausted → `status = failed`,
  trace remains **held** (release gate refuses), operator-visible; missing API key
  with backstop enabled → boot refusal.
- **Release gate**: `pending`/`failed` submissions are excluded from consumer read
  / export / quarantine-exit; `done`/`not_applicable` pass.

## Rollout

Ships **disabled** (`TRACE_COMMONS_PII_BACKSTOP_ENABLED=0`). Enable on the pilot
after (a) the migration lands, (b) `TRACE_NEAR_AI_PRIVACY_API_KEY` is provisioned
server-side, and (c) a drill confirms the canary + a seeded residual-PII envelope
round-trips to `done`. Because the release gate is additive, enabling it can only
*hold* traces, never expose more — safe to turn on incrementally.
