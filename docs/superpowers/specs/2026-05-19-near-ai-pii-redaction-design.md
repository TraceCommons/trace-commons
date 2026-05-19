# NEAR AI Hosted PII Redaction Backend

Status: draft
Date: 2026-05-19
Owner: server/privacy

## Motivation

Trace ingest already routes raw trace text through a `PrivacyFilterAdapter`
trait (see `crates/tracedao-protocol/src/trace_contribution.rs`). Today the
only production impl is `CommandPrivacyFilterAdapter`, which spawns a local
sidecar subprocess. The pilot is single-host on GCP, and we want to operate
PII redaction without packaging and securing a local PII model on every
host. NEAR AI Cloud already hosts a privacy-classifier model
(`openai/privacy-filter`) inside the TEE we use for perplexity scoring;
adopting it for PII brings the same TEE attestation story to the redaction
gate and removes a local-binary dependency from the pilot deployment.

This spec adds a second backend — `NearAiPrivacyFilterAdapter` — implementing
the existing `PrivacyFilterAdapter` trait, selectable via explicit backend
configuration. No envelope schema changes are required.

## Non-goals

- Replacing the deterministic key-based redactor in
  `crates/tracedao-protocol/src/redaction.rs`. The hosted classifier
  composes on top of the existing pipeline; it does not replace key-based
  redaction.
- Batch input (`"input": [...]`) support. The current trait is
  one-text-per-call; batching is a follow-up.
- Envelope schema changes. `SafePrivacyFilterSummary` already carries every
  field downstream code needs.
- Changing the canary / pipeline-version structure beyond adding a new
  suffix.

## Backend selection

Selection is **explicit**, never inferred. Fail-closed when configuration is
incomplete.

`privacy_filter_adapter_from_env()` gains a new dispatch step:

1. Read `TRACE_PRIVACY_FILTER_BACKEND`.
2. If unset or empty → no adapter (current behavior; deterministic
   redaction only).
3. If `=sidecar` → require `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND`;
   construct `CommandPrivacyFilterAdapter` as today.
4. If `=near-ai` → require `TRACE_NEAR_AI_PRIVACY_API_KEY`; construct
   `NearAiPrivacyFilterAdapter`. Optional knobs:
   - `TRACE_NEAR_AI_PRIVACY_BASE_URL` (default
     `https://cloud-api.near.ai/v1`; supports the
     `privacy-filter.completions.near.ai/v1` faster path).
   - `TRACE_NEAR_AI_PRIVACY_MODEL` (default `openai/privacy-filter`).
   - `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` (default `10000`).
   - `TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES` (default
     `PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES`).
5. Any other value → return a typed configuration error (fail-closed). The
   binary refuses to start; it must not silently disable redaction.
6. If `=near-ai` and the API key env var is missing/empty → typed error,
   same fail-closed semantics.

Sidecar env vars (`IRONCLAW_TRACE_PRIVACY_FILTER_*`) continue to work
unchanged when `BACKEND=sidecar`. We deliberately do not auto-detect from
their presence — explicit backend selection prevents
deployment-by-environment-drift.

## Module layout

- New module `crates/tracedao-protocol/src/privacy_filter_near_ai.rs`
  behind a new Cargo feature `near-ai-privacy-filter`. Pattern mirrors the
  existing `near-ai-scorer` feature in `tracedao-gate-enclave`.
- `reqwest` is added as an optional dep of `tracedao-protocol`, gated on
  this feature, with `default-features = false` and
  `features = ["json", "rustls-tls-native-roots"]` (async; not blocking —
  the trait is `async`). This is a new direct dep on this crate but
  `reqwest` is already in the workspace via `tracedao-gate-enclave`, so
  it is **not** a new transitive surface and does not require fresh
  approval under the dependency policy. I will note the addition in the
  PR description for explicit review.
- The dispatch glue in `privacy_filter_adapter_from_env()` lives in the
  existing `trace_contribution.rs` and is compiled unconditionally; the
  `near-ai` branch is `#[cfg(feature = "near-ai-privacy-filter")]` and
  returns a typed "feature not built in" error otherwise (still
  fail-closed).

## Request/response handling

Request body:

```json
{
  "model": "openai/privacy-filter",
  "input": "<trace text>"
}
```

Response of interest:

```json
{ "data": [ { "index": 0, "spans": [ { "category": "...", "start": N, "end": M, "score": F, "text": "..." } ], "usage": { ... } } ] }
```

For each `span`:

1. Validate `start <= end <= text.len()` (byte offsets, NEAR follows the
   OpenAI convention).
2. Validate that `start` and `end` land on UTF-8 char boundaries via
   `str::is_char_boundary`. If not, return
   `RedactionFailed { reason: "near-ai privacy classifier returned non-utf8 span boundary" }`.
   No silent fallback.
3. Build the redacted output by walking spans left-to-right and replacing
   each span with `[REDACTED:{category}]`. Overlapping spans collapse to
   the widest span (start = min(starts), end = max(ends)) and use the
   highest-score category. Adjacent same-category spans are left as-is
   (two adjacent replacements is fine).

The adapter then constructs the existing `SafePrivacyFilterRedaction`
shape directly (does **not** round-trip through the sidecar JSON
contract); each span contributes one entry to `summary.by_label` and one
`report.increment("privacy_filter:{label}")`. Labels are normalized via
the existing `safe_privacy_filter_label()` function, so any category
NEAR adds in the future that isn't in our allow-list still maps to
`unknown` with a warning — matching sidecar behavior.

NEAR's documented categories (`private_email`, `private_phone`,
`account_number`, `private_address`, `private_name`, `secret`) are all
already in the allow-list.

## Pipeline version

Add a new constant in `trace_contribution.rs`:

```rust
pub const PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX: &str = "privacy-filter-near-ai-v1";
```

`redaction_pipeline_version()` becomes backend-aware: it takes the active
backend (`None | Sidecar | NearAi`) instead of a bool, and emits one of:

- `<deterministic>` (no adapter)
- `<deterministic>+privacy-filter-sidecar-v1`
- `<deterministic>+privacy-filter-near-ai-v1`

This is the only auditable schema change. Envelope structure is
unchanged; only the string value is new. Existing rows are unaffected.

Call sites of `redaction_pipeline_version(bool)` are updated to pass the
backend tag. The tag is recorded on the redactor at construction time.

## Fail-closed behavior

The new adapter returns `TraceContributionError::RedactionFailed` (no new
error variant needed) in these cases:

- HTTP transport error, DNS failure, connect/read timeout.
- Non-2xx response status.
- Response body that fails JSON deserialization or is missing
  `data[0].spans`.
- Span offsets that violate the validations above.
- Input larger than `max_input_bytes`.
- Empty/whitespace-only input → short-circuit `Ok(None)` (matches sidecar).

Auth: `Authorization: Bearer <key>`. The key is read once at adapter
construction and held in a `secrecy::SecretString`-style wrapper (if
`secrecy` is not already in deps, a hand-rolled newtype with manual
`Debug`/`Display` that prints only `***`). The key MUST NOT appear in:

- Any error message body.
- Any log line.
- Any panic message.
- `summary` / `report` JSON.

## Hash-only audit / logging

All error logs follow the existing sidecar convention:

- Log `status`, `response_body_hash` (sha256 hex prefix), `response_len`.
- Never log the request body, response body, headers, or API key.
- Never log the model name in user-visible audit rows beyond a fixed
  label (it's not secret, but stays out of structured audit for
  simplicity).

The canary path
(`run_privacy_filter_canary(adapter, ...)`) is reused verbatim with the
new adapter. Canary failure detection (canary values appearing in
`redacted_text`, `summary`, or `report`) automatically covers the new
backend; we add an integration test that runs the canary against a mock
NEAR API and asserts `healthy = true`.

## Testing

Unit tests in the new module:

- `applies_redacted_text_replacement_for_each_span`
- `handles_overlapping_spans`
- `handles_adjacent_spans`
- `rejects_non_char_boundary_span` → `RedactionFailed`
- `rejects_out_of_range_span` → `RedactionFailed`
- `short_circuits_empty_input` → `Ok(None)`
- `maps_unknown_category_to_unknown_with_warning`
- `near_categories_land_in_allow_list` (table-driven over the six
  documented categories)

HTTP-level tests using `wiremock` (proposed new dev-dep; widely adopted,
pure-rust, no native deps). I'll flag it in the PR description per the
dependency policy. If approval is withheld, we fall back to a hand-rolled
`hyper::Server` test harness on `127.0.0.1:0`.

- `surfaces_http_error_as_redaction_failed`
- `surfaces_timeout_as_redaction_failed`
- `does_not_leak_api_key_into_error_strings`
- `canary_run_via_mock_returns_healthy`
- `canary_run_with_leaking_mock_returns_unhealthy`

Existing tests in `trace_contribution.rs` for the sidecar branch are
untouched. The `redaction_pipeline_version` change is covered by adding
a `near-ai` case to its existing tests.

## Rollout

1. Land behind `near-ai-privacy-filter` feature flag, off by default.
2. Default pilot build flips on the feature alongside `near-ai-scorer`
   (the two travel together since both target NEAR AI Cloud).
3. Operator runbook update
   (`docs/operator/pilot-gcp-deployment.md`): document
   `TRACE_PRIVACY_FILTER_BACKEND=near-ai` and the new env vars.
4. First-traffic gate: run the canary against the hosted endpoint as
   part of pilot bootstrap; refuse to admit traces until the canary is
   healthy.

## Open questions for plan stage

- Should the hand-rolled secret wrapper live in `tracedao-protocol` or
  reuse the `secrecy` crate? Decide during plan-writing — if `secrecy`
  is already transitively present we lean toward reuse; otherwise the
  hand-rolled newtype is fewer than 30 lines.
- Concurrency: hosted endpoint will be the rate-limit choke point. Plan
  stage to confirm whether we need a semaphore at the adapter level or
  whether ingest's existing per-trace concurrency cap is sufficient.
