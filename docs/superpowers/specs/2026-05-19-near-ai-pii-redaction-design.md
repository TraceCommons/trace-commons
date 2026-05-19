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

`privacy_filter_adapter_from_env()` currently has signature
`fn() -> Option<Arc<dyn PrivacyFilterAdapter>>` and silently returns `None`
when nothing is set. It changes to:

```rust
fn() -> Result<Option<Arc<dyn PrivacyFilterAdapter>>, PrivacyFilterConfigError>
```

with a new dedicated error type (not `TraceContributionError`, which is
reserved for per-trace redaction failures):

```rust
#[derive(Debug, thiserror::Error)]
pub enum PrivacyFilterConfigError {
    #[error("unknown TRACE_PRIVACY_FILTER_BACKEND value: {value}")]
    UnknownBackend { value: String },
    #[error("missing required env var for backend {backend}: {var}")]
    MissingEnv { backend: &'static str, var: &'static str },
    #[error("invalid env var {var}: {reason}")]
    InvalidEnv { var: &'static str, reason: String },
    #[error("backend {backend} requires the {feature} cargo feature")]
    FeatureDisabled { backend: &'static str, feature: &'static str },
}
```

Every existing call site (notably the two in `trace_contribution.rs` —
the canary entry point around line 1522 and the redactor construction
around line 1965) becomes fallible. Binaries propagate the error and
refuse to start; failure to construct the adapter must never silently
fall back to deterministic-only redaction.

Dispatch:

1. Read `TRACE_PRIVACY_FILTER_BACKEND`.
2. If unset or empty → `Ok(None)` (deterministic redaction only,
   current behavior). The default `cargo check -p tracedao-protocol`
   build with no features set and no env vars set continues to compile
   and behave exactly as today.
3. If `=sidecar` → require `TRACE_PRIVACY_FILTER_COMMAND`. Construct
   `CommandPrivacyFilterAdapter`. See "Env var naming" below for the
   `IRONCLAW_TRACE_PRIVACY_FILTER_*` back-compat policy.
4. If `=near-ai` → require `TRACE_NEAR_AI_PRIVACY_API_KEY`; construct
   `NearAiPrivacyFilterAdapter`. Optional knobs:
   - `TRACE_NEAR_AI_PRIVACY_BASE_URL` (default
     `https://cloud-api.near.ai/v1`; supports the
     `privacy-filter.completions.near.ai/v1` faster path).
   - `TRACE_NEAR_AI_PRIVACY_MODEL` (default `openai/privacy-filter`).
   - `TRACE_NEAR_AI_PRIVACY_TIMEOUT_MS` (default `10000`).
   - `TRACE_NEAR_AI_PRIVACY_MAX_INPUT_BYTES` (default
     `PRIVACY_FILTER_SIDECAR_DEFAULT_MAX_INPUT_BYTES`).
   - If the `near-ai-privacy-filter` cargo feature is not built in,
     return `FeatureDisabled { backend: "near-ai", feature: "near-ai-privacy-filter" }`.
5. Any other value → `UnknownBackend`.
6. If a required env var is missing/empty → `MissingEnv`.

We deliberately do not auto-detect from env presence — explicit
backend selection prevents deployment-by-environment-drift.

### Env var naming

The repo is mid-rename from Ironclaw to Trace Commons. The sidecar
backend currently reads `IRONCLAW_TRACE_PRIVACY_FILTER_*`. This spec
standardizes on the `TRACE_` prefix for **all** privacy-filter env vars
going forward:

- New canonical: `TRACE_PRIVACY_FILTER_BACKEND`,
  `TRACE_PRIVACY_FILTER_COMMAND`, `TRACE_PRIVACY_FILTER_ARGS`,
  `TRACE_PRIVACY_FILTER_TIMEOUT_MS`, `TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES`,
  `TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES`,
  `TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES`.
- Back-compat: each canonical name falls back to the corresponding
  `IRONCLAW_TRACE_PRIVACY_FILTER_*` value if the canonical one is unset.
  When the back-compat path fires, log a one-shot warning at startup
  ("env var IRONCLAW_TRACE_PRIVACY_FILTER_X is deprecated; rename to
  TRACE_PRIVACY_FILTER_X"). Removal of the back-compat path is a
  follow-up after the pilot host is migrated.

## Module layout

- New module `crates/tracedao-protocol/src/privacy_filter_near_ai.rs`
  behind a new Cargo feature `near-ai-privacy-filter`. Pattern mirrors the
  existing `near-ai-scorer` feature in `tracedao-gate-enclave`.
- The dispatch glue in `privacy_filter_adapter_from_env()` lives in the
  existing `trace_contribution.rs` and is compiled unconditionally; the
  `near-ai` branch is `#[cfg(feature = "near-ai-privacy-filter")]` and
  returns `FeatureDisabled` otherwise (still fail-closed).

### Dependencies (explicit approval requested)

Per the dependency policy in CLAUDE.md, both additions below are
**new direct deps on `tracedao-protocol`** and need explicit approval
before plan stage proceeds. They are already in the workspace via
other crates, but the policy keys on direct deps, not transitive.

- **`reqwest` 0.12** as `optional = true, default-features = false,
  features = ["json", "rustls-tls-native-roots"]`, gated on the
  `near-ai-privacy-filter` feature. Already used in
  `tracedao-gate-enclave/src/perplexity_near_ai.rs`. Required for the
  async HTTP client.
- **`wiremock`** as a dev-dependency for HTTP-level tests. Pure-rust,
  widely adopted, no native deps. If approval is withheld, fall back to
  a hand-rolled `hyper::Server` test harness on `127.0.0.1:0` — that
  fallback decision should be made before plan stage to avoid a
  round-trip during implementation. **Default recommendation: use
  `wiremock`.**

Neither dep is added until the user approves them on this spec.

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

### Span accounting and label normalization

The new adapter constructs `SafePrivacyFilterRedaction` directly (no
round-trip through the sidecar JSON contract). To stay drop-in
compatible with the sidecar path, the counting model **matches the
sidecar exactly**:

- `summary.span_count` = the count of **raw spans returned by the API**,
  not the post-collapse count. This matches
  `safe_privacy_filter_redaction_from_output` at line 1460
  (`detected_spans.len()`).
- `summary.by_label` increments **once per raw span**, even when spans
  are collapsed for replacement in `redacted_text`. So
  `redacted_text.len()` and `span_count` may disagree when overlaps
  exist; that is the same divergence the sidecar would produce if
  fed identical spans.
- `report.increment("privacy_filter:{label}")` is called once per raw
  span. `report.pii_labels_present` is deduped (matches existing logic
  inside `safe_privacy_filter_redaction_from_output`).

Label normalization goes through the existing
`safe_privacy_filter_label(raw_label, &mut report)` function. The new
module owns its `RedactionReport` value, threads `&mut report` through
the per-span loop, and returns it inside `SafePrivacyFilterRedaction`.
Any category NEAR adds in the future that isn't in our allow-list
(lines 1483–1499) still maps to `"unknown"` with a warning pushed to
`report.warnings` — matching sidecar behavior.

NEAR's documented categories (`private_email`, `private_phone`,
`account_number`, `private_address`, `private_name`, `secret`) are all
already in the allow-list.

## Pipeline version

Add a new constant in `trace_contribution.rs`:

```rust
pub const PRIVACY_FILTER_NEAR_AI_PIPELINE_SUFFIX: &str = "privacy-filter-near-ai-v1";
```

Introduce a small enum to carry the backend identity:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivacyFilterBackendTag {
    None,
    Sidecar,
    NearAi,
}
```

`redaction_pipeline_version` changes from `fn(bool) -> String` to
`fn(PrivacyFilterBackendTag) -> String` and emits one of:

- `<deterministic>` (`None`)
- `<deterministic>+privacy-filter-sidecar-v1` (`Sidecar`)
- `<deterministic>+privacy-filter-near-ai-v1` (`NearAi`)

### Where the tag is stored

`DeterministicTraceRedactor` (the only `TraceRedactor` impl, around
line 1934) gains a `privacy_filter_backend: PrivacyFilterBackendTag`
field set at construction time. The constructor that wires a
`PrivacyFilterAdapter` in (today inferring `bool` from
`Option::is_some`) is updated to take the tag explicitly. The
adapter-builder path is the single place that knows which backend was
constructed, so it returns both the `Arc<dyn PrivacyFilterAdapter>`
and the `PrivacyFilterBackendTag` to the redactor constructor.

Existing call site at line 2234
(`redaction_pipeline_version(privacy_filter_summary.is_some())`) is
replaced with a read of `self.privacy_filter_backend`. The local
fallback when `privacy_filter_summary` is `None` after a successful
adapter call (which can happen for empty input) still emits the
backend-tagged version string — running an adapter that no-ops on
empty input is still "this backend was active for this envelope."

This is the only auditable schema change. Envelope structure is
unchanged; only the string value is new. Existing rows are unaffected.

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

## Operational semantics (explicit)

To prevent the implementer from inventing behavior, the following
choices are pinned now:

- **Retries: none.** A single HTTP attempt per `redact_text` call. A
  429 or 5xx response surfaces as `RedactionFailed`, the per-trace
  ingest path bubbles the error, and the trace is rejected. The
  pilot's NEAR AI usage is modest; we will add retry/backoff in a
  follow-up if rate-limit data justifies it.
- **Key rotation: restart-only.** The API key is read at adapter
  construction. To rotate, redeploy the binary. We do not poll or
  re-read env at runtime.
- **Runtime on/off: restart-only.** No SIGHUP, no admin endpoint to
  flip backends. To switch backends, change `TRACE_PRIVACY_FILTER_BACKEND`
  and restart.
- **Request-id propagation: out of scope for v1.** Not propagated to
  NEAR. Add later if NEAR exposes a server-side trace correlation hook
  we want to bind to.
- **Telemetry: out of scope for v1.** The sidecar path emits no
  per-call metrics today. We will not add metrics solely for the
  hosted backend; metrics for both backends are a single follow-up
  ticket.
- **Concurrency: rely on ingest's existing per-trace concurrency cap.**
  No semaphore at the adapter level for v1. Revisit if pilot traffic
  triggers 429s — at which point the right answer is likely a queue
  upstream of ingest, not a semaphore in the adapter.

## Open questions for plan stage

- Should the hand-rolled secret wrapper live in `tracedao-protocol` or
  reuse the `secrecy` crate? Decide during plan-writing — if `secrecy`
  is already transitively present we lean toward reuse; otherwise the
  hand-rolled newtype is fewer than 30 lines and avoids a new
  approval round.
