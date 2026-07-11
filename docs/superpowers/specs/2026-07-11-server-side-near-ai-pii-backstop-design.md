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

## Data flow

1. **Ingest (behavior unchanged):** deterministic rescrub → store envelope. When
   the envelope has `privacy.message_text_included = true`, the submission is
   enrolled as `pii_backstop = pending`. Envelopes without message text are marked
   `not_applicable` (nothing to scan) and never block release.
2. **Backstop tick (async):** claim a batch of `pending` submissions →
   for each, load the stored (post-deterministic-rescrub) envelope → extract the
   `content` / `human_correction` message-text fields → run the **chunked** NEAR AI
   pass via `trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter`
   (the codepoint-safe, chunking, retrying adapter) → apply the returned spans to
   re-redact the text → write a new `RescrubbedEnvelope` storage artifact with
   `privacy.redaction_pipeline_version` suffixed `+near-ai-pii-backstop-v1` →
   update `privacy.redaction_counts` / `pii_labels_present` / `residual_pii_risk`
   → mark `pii_backstop = done`.
3. **Release gate:** a trace with `pii_backstop = pending` (or `failed`) is
   **held** — not consumer-readable, not exportable, not eligible to leave the
   quarantine lane — until `done` or an operator waiver. Envelopes marked
   `not_applicable`/`done` pass the gate.

## Components (independently testable)

- **`pii_backstop` module** — work enumeration, the driver tick, attempt/backoff
  bookkeeping, and the re-redaction + metadata-update logic. Pure re-redaction
  (envelope + spans → rescrubbed envelope) is a separate unit from the DB/driver
  plumbing so it is testable without a DB or network.
- **State**: a dedicated `trace_pii_backstop` table (keeps the backstop's
  bookkeeping separate from gate-scoring state): `submission_id` (PK),
  `status` (`pending`|`done`|`failed`|`not_applicable`), `attempt_count`,
  `last_error_hash`, `updated_at`. Forced RLS with a driver-role read/claim
  policy, mirroring the gate-driver attempts table added in the perplexity
  scoring-driver work.
- **Reused as-is**: the protocol NEAR AI adapter (with its once-per-cycle
  `run_privacy_filter_canary` self-test before processing real traces), the
  `RescrubbedEnvelope` storage path, the driver DB role, the release/visibility
  check (extended with the backstop-status predicate).

## Fail posture

- **Fail-closed on release, never on ingest.** Ingest always succeeds (subject to
  its existing checks); the backstop runs afterward.
- NEAR AI transient failure → the adapter's own retry/backoff, then the driver's
  attempt bookkeeping re-queues the submission on the next tick.
- After `max_attempts`, status becomes `failed`: the trace stays **held** and
  flagged, surfaced to operators (review/quarantine surface). It is never released
  with unredacted residual PII.
- If the backstop is enabled but `TRACE_NEAR_AI_PRIVACY_API_KEY` is missing/blank,
  refuse at boot with a safe missing-control name (fail-closed configuration, per
  repo convention) rather than silently disabling the backstop.

## Configuration

Server crate enables the protocol `near-ai-privacy-filter` feature.

- `TRACE_COMMONS_PII_BACKSTOP_ENABLED` (default `0`).
- `TRACE_NEAR_AI_PRIVACY_API_KEY` (+ optional `TRACE_NEAR_AI_PRIVACY_BASE_URL` /
  `_MODEL` / `_TIMEOUT_MS` / `_MAX_INPUT_BYTES`) — read from env, never persisted.
- `TRACE_COMMONS_PII_BACKSTOP_MAX_ATTEMPTS` (default 5),
  `..._BATCH_SIZE`, `..._TICK_INTERVAL_SECONDS` — mirror the scoring driver knobs.
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
