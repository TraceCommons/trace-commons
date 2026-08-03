# Trace Specification

**The trace record — what clients submit and what customers train against.**

This document is the authoritative definition of the Trace Commons trace
*record*: the `ironclaw.trace_contribution.v1` envelope. It serves two
audiences:

- **Clients** (contributor side, Ironclaw) — how to capture, scrub, and shape
  a trace into a valid submission envelope.
- **Customers** (consumer side, frontier labs / auditors / evaluators) — what a
  trace looks like when it reaches you for training, what each field means, and
  what your consent scope and allowed-use entitle you to read and do.

## Scope and relationship to other documents

This spec owns the **format**. The surrounding control plane is documented
elsewhere and those documents remain authoritative for their areas:

| Document | Authoritative for |
|---|---|
| **This document** (`docs/trace-spec.md`) | The trace envelope schema, consent/privacy/allowed-use semantics, the submission contract, and the consumer (filtered-envelope) contract. |
| `docs/trace-commons.md` | Server ingest, threat model, auth/upload claims, and the operational API surface. |
| `docs/trace-commons-storage.md` | Storage boundaries, PostgreSQL schema, retention, RLS. |
| `docs/trace-commons-roadmap.md` | Phase plan and the production-gap queue. |

**Field source of truth.** Every field table below is derived from
`crates/trace-commons-protocol/src/trace_contribution.rs`. If a table here ever
disagrees with that crate, **the crate wins** and the table is a bug. The
protocol crate is the machine-checkable contract; this document is its
human-readable normative companion.

### Version pinning

| Constant | Value | Meaning |
|---|---|---|
| `schema_version` | `ironclaw.trace_contribution.v1` | The on-the-wire envelope schema. A submission whose `schema_version` does not match is treated as invalid (`schema_validity = 0`). |
| `consent.policy_version` | `2026-04-24` | The consent policy the contributor agreed to. |
| `redaction_pipeline_version` | `ironclaw-deterministic-secret-path-v3` (+ optional suffixes) | The scrubbing pipeline that produced the envelope. Server re-scrub appends `+server-rescrub-v2`. |

## The contract in one paragraph

Trace contribution is **off by default**. Raw traces stay on the contributor's
machine. Capture and deterministic redaction both happen locally; only the
scrubbed `ironclaw.trace_contribution.v1` envelope crosses the wire. The server
re-scrubs every envelope, then applies two gates — **novelty** (is this
genuinely different from what is already filed?) and **substance** (is this
substantive work rather than template-shaped filler?). Both must pass. Accepted
records are signed, dated, and filed. Customers never receive raw traces: they
receive a **filtered projection of the accepted envelope**, gated by the
contributor's consent scope intersected with the customer's allowed-use grant.

---

# Part 1 — The submission contract (clients)

## Lifecycle

```
 raw session (local, never leaves device)
        │
        ▼  deterministic redaction + optional privacy-filter sidecar
 redacted envelope (ironclaw.trace_contribution.v1)
        │
        ▼  submit over HTTPS with an upload claim
 server re-scrub  →  novelty gate  →  substance gate
        │
        ▼
 status: accepted | quarantined | rejected
```

What stays local: the raw session, message text, and tool payloads (unless the
contributor explicitly opts those fields in). What crosses the wire: only the
envelope, after local redaction.

## Client obligations (MUST / MUST NOT)

These are the local-first invariants. A client that violates them produces an
envelope that the server may quarantine or reject, and — more importantly —
breaks the contributor's trust contract.

- **MUST** keep contribution off by default and require an explicit standing
  opt-in policy with an ingestion endpoint before any autonomous submission.
- **MUST** run deterministic local redaction before building the envelope, and
  record the pipeline version in `privacy.redaction_pipeline_version`.
- **MUST** use stable, structure-preserving placeholders (`<PRIVATE_EMAIL_1>`,
  `<PRIVATE_LOCAL_PATH_1>`) rather than flattening every entity to one token.
- **MUST NOT** include raw message text unless the contributor opted into
  message text; likewise tool payloads. Setting `message_text_included` /
  `tool_payloads_included` is a factual declaration, not a default. The
  server **MUST** correct under-reported declarations upward to match the
  envelope payload before risk classification and PII-backstop decisions
  (over-reporting is left alone).
- **MUST NOT** serialize original PII-filter text, raw `detected_spans[*].text`,
  raw offsets, or unsafe span labels. Only the safe
  `SafePrivacyFilterSummary` (redacted text + allow-listed label counts +
  warnings) may be carried.
- **MUST** set `privacy.residual_pii_risk` honestly. Including message text or
  tool payloads raises the floor to `medium`; a detected secret forces `high`.
- **MUST** be hash-only in any identifier that could deanonymize: contributor
  identity is pseudonymous, the redaction is summarized as a `sha256:` hash, and
  no raw URLs, tokens, ARNs, account refs, or trace bodies appear in metadata
  fields.

## Envelope structure

The envelope is `TraceContributionEnvelope`. Top-level shape:

| Field | Type | Required | Set by | Notes |
|---|---|---|---|---|
| `schema_version` | string | yes | client | Must equal `ironclaw.trace_contribution.v1`. |
| `trace_id` | UUID | yes | client | Stable identity of the underlying trace. |
| `submission_id` | UUID | yes | client | Identity of *this* submission; idempotency key for retries. |
| `created_at` | RFC3339 datetime | yes | client | Envelope creation time. |
| `ironclaw` | `IronclawTraceMetadata` | yes | client | Engine/channel provenance. |
| `consent` | `ConsentMetadata` | yes | client | What the contributor agreed to. |
| `contributor` | `ContributorMetadata` | yes | client | Pseudonymous attribution + revocation handle. |
| `privacy` | `PrivacyMetadata` | yes | client | Redaction outcome and residual risk. |
| `events` | `[TraceContributionEvent]` | yes | client | The trajectory. The trainable substance. |
| `outcome` | `OutcomeMetadata` | yes | client | Task result and failure labels. |
| `replay` | `ReplayMetadata` | yes | client | Replayability and required tools. |
| `embedding_analysis` | `EmbeddingAnalysisMetadata?` | optional | **server** | Novelty/duplicate/cluster signals. Client may omit. |
| `value` | `ValueMetadata` | yes | client→**server** | Client estimates; server authors the final score. |
| `trace_card` | `TraceCard` | defaulted | client | Consent/use/retention projection used for ABAC. |
| `value_card` | `TraceValueCard` | defaulted | client→**server** | Human-readable scorecard. |
| `hindsight` | `HindsightRelabelingCandidate?` | optional | client | Hindsight relabeling hints. |
| `training_dynamics` | `TrainingDynamicsSignals?` | optional | client/server | Curriculum signals (confidence/variability). |
| `process_evaluation` | `ProcessEvaluationLabels?` | optional | **server** | Process-quality labels from evaluators. |

### `ironclaw` — `IronclawTraceMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `version` | string | yes | Protocol crate version that produced the envelope. |
| `engine_version` | string? | optional | Ironclaw engine build. |
| `feature_flags` | map<string,string> | optional | Active flags at capture time. |
| `channel` | `TraceChannel` | yes | One of `web`, `cli`, `telegram`, `slack`, `routine`, `other`. |
| `model_name` | string? | optional | Model that produced the trace. |

### `consent` — `ConsentMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `policy_version` | string | yes | Consent policy version (`2026-04-24`). |
| `scopes` | `[ConsentScope]` | yes | What uses the contributor authorized. See matrix in Part 3. |
| `message_text_included` | bool | yes | Whether raw redacted message text is present. Server corrects `false` → `true` when events/outcome carry message content. |
| `tool_payloads_included` | bool | yes | Whether tool payloads are present. Server corrects `false` → `true` when events carry tool content, structured payloads, or tool names. |
| `revocable` | bool | yes | Whether the contributor retains a revocation right. |

`ConsentScope` values: `debugging_evaluation`, `benchmark_only`,
`ranking_training`, `model_training`, `public_attribution`.

> `public_attribution` is special: it gates the community-handle surface only.
> It grants **no** trace-content allowed-uses. A claim scoped to *only*
> `public_attribution` cannot submit traces.

### `contributor` — `ContributorMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `pseudonymous_contributor_id` | string? | optional | Pseudonym; never a real identity. |
| `tenant_scope_ref` | string? | optional | Attribution only. The server normalizes any server-looking value to the auth-derived tenant before storage. |
| `credit_account_ref` | string? | optional | Where credit accrues. |
| `revocation_handle` | UUID | yes | The capability to later revoke this submission. |

> **Attribution, not authority.** Envelope `tenant_scope_ref` and contributor
> fields are *attribution* metadata. All read/write authorization is driven by
> the auth-derived tenant + actor context, never by envelope contents.

### `privacy` — `PrivacyMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `redaction_pipeline_version` | string | yes | Pipeline that scrubbed the envelope. Server appends `+server-rescrub-v2`. |
| `redaction_counts` | map<string,u32> | optional | Per-label redaction counts (counts only, never content). |
| `privacy_filter_summary` | `SafePrivacyFilterSummary?` | optional | Safe summary of a PII-filter pass. |
| `pii_labels_present` | [string] | optional | Allow-listed labels that were found and redacted. |
| `residual_pii_risk` | `ResidualPiiRisk` | yes | `low` / `medium` / `high`. |
| `redaction_hash` | string | yes | `sha256:` hash over events + counts. The dependability anchor. |
| `warnings` | [string] | optional | Human-readable redaction warnings. |

**How `residual_pii_risk` is derived** (server recomputes this; clients must
match): a detected secret forces `high`; otherwise including message text or
tool payloads yields `medium`; otherwise `low`. Only `low`-risk accepted traces
are eligible for consumer export.

`SafePrivacyFilterSummary`: `schema_version`, `output_mode`, `span_count`,
`by_label` (counts), `decoded_mismatch`. Never carries raw text or offsets.

### `events` — `[TraceContributionEvent]`

The trajectory, and the core trainable substance. Each event:

| Field | Type | Required | Notes |
|---|---|---|---|
| `event_id` | UUID | yes | |
| `parent_event_id` | UUID? | optional | Tree structure for sub-steps. |
| `event_type` | `TraceContributionEventType` | yes | See below. |
| `timestamp` | datetime | yes | |
| `redacted_content` | string? | optional | Present only if the relevant consent field was opted in. |
| `structured_payload` | JSON | optional | Redacted structured data (e.g. tool-call id, arg shape). |
| `tool_name` | string? | optional | For tool events. |
| `tool_category` | string? | optional | Coarse category (drives `trace_card.tool_categories`). |
| `tool_call_id` | string? | optional | Correlates a call with its result. |
| `latency_ms` | u64? | optional | |
| `token_counts` | `TokenCounts?` | optional | `input_tokens`, `output_tokens`. |
| `cost_usd` | decimal? | optional | |
| `success` | bool? | optional | Per-event success. |
| `failure_modes` | `[TraceFailureMode]` | optional | Per-event failure labels. |
| `side_effect` | `SideEffectLevel` | defaulted | `none`/`read_only`/`local_write`/`external_write`/`credential_use`/`unknown`. |

`TraceContributionEventType`: `user_message`, `assistant_message`, `tool_call`,
`tool_result`, `routing_decision`, `feedback`, `http_exchange`.

`TraceFailureMode`: `tool_selection_error`, `tool_argument_error`,
`tool_ordering_error`, `missing_verification`, `premature_termination`,
`looping_or_repetition`, `context_loss`, `privacy_policy_violation`,
`secret_exposure_attempt`, `user_intent_misread`, `unrecoverable_tool_failure`,
`bad_memory_retrieval`, `bad_routing_decision`, `unsafe_side_effect`,
`specification_ambiguity`, `environment_or_auth_failure`, `other(<string>)`.

### `outcome` — `OutcomeMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `user_feedback` | `UserFeedback` | yes | `thumbs_up`/`thumbs_down`/`correction`/`none`. |
| `task_success` | `TaskSuccess` | yes | `success`/`partial`/`failure`/`unknown`. |
| `error_taxonomy` | [string] | optional | Free-form error tags. |
| `failure_modes` | `[TraceFailureMode]` | optional | Trace-level failure labels. |
| `human_correction` | string? | optional | Redacted correction text, if opted in. A high-value training signal. |

### `replay` — `ReplayMetadata`

| Field | Type | Required | Notes |
|---|---|---|---|
| `replayable` | bool | yes | Whether the trace can be deterministically replayed. |
| `required_tools` | [string] | optional | Tools the replay depends on. |
| `tool_manifest_hashes` | map<string,string> | optional | Hash-pinned tool versions. |
| `expected_assertions` | [JSON] | optional | Assertions a replay must satisfy. |
| `replay_notes` | [string] | optional | Caveats (e.g. omitted tool args). |

### `value` — `ValueMetadata` (client estimate, server-authored final)

| Field | Type | Set by | Notes |
|---|---|---|---|
| `submission_score` | f32 | server | Authoritative online score. |
| `credit_points_pending` | f32 | server | Pending (pre-settlement) credit. |
| `credit_points_final` | f32? | server | Settled credit, once confirmed. |
| `explanation` | [string] | server | Human-readable scoring factors. |

Clients may populate `value` with a local estimate for UX, but the server
recomputes and **its values are authoritative**. See `compute_value_scorecard`
in the protocol crate for the exact formula.

### `trace_card` — `TraceCard`

The ABAC projection the server uses to gate downstream reads. Defaulted if
omitted, but clients should set it to reflect actual consent.

| Field | Type | Notes |
|---|---|---|
| `consent_scope` | `ConsentScope` | Primary scope. |
| `redaction_pipeline_version` | string | Mirrors `privacy`. |
| `source_channel` | string | Channel label. |
| `tool_categories` | [string] | Derived from event `tool_category`. |
| `allowed_uses` | `[TraceAllowedUse]` | The uses this trace permits. Defaults from scope — see Part 3. |
| `retention_policy` | string | e.g. `private_corpus_revocable`. |
| `revocation_handle` | string | Mirrors contributor handle. |

### `value_card` — `TraceValueCard`

Human-readable scorecard (`TraceValueScorecard`) with per-axis sub-scores
(`schema_validity`, `privacy_risk`, `quality`, `replayability`, `novelty`,
`duplicate_penalty`, `coverage_bonus`, `difficulty`, `dependability`,
`user_correction_value`, optional `process_eval_value`, optional
`downstream_utility`, `online_score`, `credit_points_estimate`) plus
`limitations` and `user_visible_explanation`. Server-authored at acceptance.

### Optional enrichment blocks

| Block | Type | Purpose |
|---|---|---|
| `hindsight` | `HindsightRelabelingCandidate` | Original goal, achieved subgoals, recoverability, benchmark/relabel candidacy. |
| `training_dynamics` | `TrainingDynamicsSignals` | `mean_confidence`, `variability`, `correctness`, `cartography_bucket` (`easy`/`ambiguous`/`hard`/`unknown`) — curriculum signals. |
| `process_evaluation` | `ProcessEvaluationLabels` | Evaluator name/version, `labels`, per-axis ratings (`tool_selection`, `tool_argument_quality`, `tool_ordering`, `verification`, `side_effect_safety`), `overall_score`. **Server/evaluator authored.** |

---

# Part 2 — The gating contract (server)

When an envelope is submitted, the server:

1. **Re-scrubs** the envelope (appends `+server-rescrub-v2` to the pipeline
   version, recomputes `redaction_hash` and `residual_pii_risk`, merges
   privacy warnings). Envelope contributor/tenant fields are treated as
   attribution only and normalized to the auth-derived tenant.
2. **Novelty gate** — embeds the trace and compares against the existing
   register (`embedding_analysis.novelty_score` / `duplicate_score`).
3. **Substance gate** — scores substantive work against a frontier model. In
   Phase A this runs on NEAR AI's TEE-hosted vLLM; Phase B moves scoring into
   attested hardware.

Both gates must pass. The resulting submission status is one of:

| Status | Meaning | Consumer-visible? |
|---|---|---|
| `accepted` | Passed both gates, filed into the register. | Eligible (if `low` risk + use match). |
| `quarantined` | Held for manual review (e.g. `medium`/`high` risk). | No. |
| `rejected` | Failed a gate. | No. |
| `revoked` | Contributor exercised revocation. | No; derived artifacts invalidated. |
| `expired` | Retention window elapsed. | No. |
| `purged` | Removed. | No. |

Server-authored fields after gating: `embedding_analysis`, `value`,
`value_card`, and (via evaluators) `process_evaluation`. Clients do not set
these authoritatively.

---

# Part 3 — The consumer contract (customers training against it)

## The filtered-envelope model

**Customers never receive raw traces, and never receive the raw envelope.** A
customer receives the **envelope body through the replay-export path**, and only
when *all* of the following hold:

1. Submission `status = accepted` (one of `accepted`, `quarantined`, `rejected`, `revoked`, `expired`, `purged`).
2. `privacy.residual_pii_risk = low`.
3. The contributor's `consent.scopes` grant the **allowed-use** the customer's
   token requests.
4. The customer's token (and, in hosted deployments, the tenant access grant)
   carries that allowed-use in its allow-list.

If any condition fails, the trace is skipped — silently, by design. The export
is fail-closed.

> **Derived artifacts are out of scope here.** Benchmark datasets and ranker
> candidate/pair exports are *derived* artifacts built from accepted traces,
> not envelopes, and have their own record shapes. They are governed by the
> same allowed-use gates (`benchmark_generation`, `ranking_model_training`) but
> their serialization is defined alongside the export pipeline, not in this
> document. This spec covers the envelope and its filtered consumer projection.

## Consent scope → allowed-use grant

Each `ConsentScope` grants a fixed default set of `TraceAllowedUse` values
(`default_allowed_uses_for_scope` in the protocol crate). A customer's effective
permission is the **intersection** of the contributor's granted uses and the
customer token's allowed-use allow-list — a grant can only ever narrow, never
widen.

| Consent scope | `debugging` | `evaluation` | `benchmark_generation` | `ranking_model_training` | `model_training` | `aggregate_analytics` |
|---|:---:|:---:|:---:|:---:|:---:|:---:|
| `debugging_evaluation` | ✓ | ✓ | | | | ✓ |
| `benchmark_only` | | ✓ | ✓ | | | ✓ |
| `ranking_training` | ✓ | ✓ | | ✓ | | ✓ |
| `model_training` | ✓ | ✓ | | ✓ | ✓ | ✓ |
| `public_attribution` | | | | | | |

What each allowed-use entitles a customer to do:

| Allowed-use | Entitles the customer to |
|---|---|
| `debugging` | Inspect the trace to diagnose agent behavior. |
| `evaluation` | Use the trace in evaluation / replay export. |
| `benchmark_generation` | Convert the trace into benchmark cases. |
| `ranking_model_training` | Use the trace for ranker candidate/pair training. |
| `model_training` | Use the trace for model training. |
| `aggregate_analytics` | Count it in aggregate analytics only — **not** derive per-trace artifacts. |

> `aggregate_analytics` is intentionally insufficient for any per-trace derived
> artifact (including vector indexing). It permits counting, not extraction.

## Retention classes

A trace's `trace_card.retention_policy` maps to a `TraceRetentionClass`, which
bounds what may be derived and for how long:

| Class | Revocable | Derived artifacts allowed? |
|---|:---:|:---:|
| `local_queue` | — | Local only; never filed. |
| `private_corpus_revocable` | ✓ | ✓ |
| `benchmark_revocable` | ✓ | ✓ |
| `training_revocable` | ✓ | ✓ |
| `aggregate_only` | ✗ | ✗ (analytics counting only) |

Approvals for an `aggregate_only` retention class are blocked from producing
derived corpus artifacts even if a use would otherwise permit them.

## Per-field consumer visibility

When a customer reads a filtered envelope, fields fall into three classes:

- **Trainable signal** (the reason to consume): `events` (trajectory),
  `outcome` (labels), `replay` (verification), `value_card` / `training_dynamics`
  (difficulty/curriculum), `process_evaluation` (process labels),
  `hindsight` (relabeling).
- **Provenance / interpretation**: `ironclaw` (channel, model), `schema_version`,
  `created_at`, `privacy.redaction_pipeline_version`, `trace_card`.
- **Attribution only — not training input, and pseudonymous**:
  `contributor.*`. Never treat contributor fields as features or as identity.

`redacted_content` and tool payloads inside `events` are present **only** when
the contributor opted those fields in *and* the trace cleared `low` residual
risk after server re-scrub. A customer must be able to train on the structure of
a trace even when content fields are absent.

## How to interpret a trace for training

- **Trajectory**: the ordered `events` are the agent's actual path —
  user/assistant turns, tool calls and results, routing decisions. Use
  `parent_event_id` for sub-step structure and `tool_call_id` to pair calls
  with results.
- **Labels**: `outcome.task_success`, `outcome.user_feedback`,
  `outcome.human_correction`, and per-event/per-trace `failure_modes` are the
  supervision signal. `process_evaluation` ratings add process-quality labels.
- **Verification**: `replay.replayable` + `required_tools` +
  `tool_manifest_hashes` + `expected_assertions` let you confirm a trace
  actually behaves as recorded before training on it.
- **Curriculum**: `training_dynamics.cartography_bucket`
  (`easy`/`ambiguous`/`hard`) and `value_card` sub-scores (`difficulty`,
  `novelty`, `dependability`) support difficulty-aware sampling.
- **Cost/efficiency**: per-event `latency_ms`, `token_counts`, `cost_usd`
  support efficiency objectives.

---

# Part 4 — Versioning and compatibility

- **Schema version** (`ironclaw.trace_contribution.v1`) bumps only on a
  breaking change to the envelope shape. A submission with a non-matching
  `schema_version` scores `schema_validity = 0` and is treated as invalid.
- **Additive fields** are introduced as optional with serde defaults
  (`#[serde(default)]` / `skip_serializing_if`). Adding an optional field does
  **not** bump the schema version.
- **Consumers MUST ignore unknown fields** rather than failing. A consumer
  written against an older view of the schema must still parse newer envelopes.
- **Policy version** (`consent.policy_version`) and **redaction pipeline
  version** are tracked independently of the schema version; they record *what
  the contributor agreed to* and *how the trace was scrubbed*, and may advance
  without a schema bump.

When this document and `crates/trace-commons-protocol/src/trace_contribution.rs`
disagree, the crate is correct and this document must be corrected to match.

---

## Verification Summary

Every field table, enum, mapping, and behavioral claim in this document was
checked against `crates/trace-commons-protocol/src/trace_contribution.rs` and
the server crate (`crates/trace-commons-server/src/`) on 2026-05-29. The
redaction pipeline version constants below were re-checked on 2026-07-29, when
they moved to `-v3` / `+server-rescrub-v2`; the rest of the audit still dates
from 2026-05-29.

- **Claim groups checked:** ~60 (covering 18 top-level envelope fields, every
  sub-struct, all enum variant sets, the consent→allowed-use matrix, the
  retention-class table, the residual-risk rule, and the submission-status set
  — 250+ individual field/variant/cell facts).
- **Confirmed:** all schema field names, types, optionality, enum variants
  (incl. snake_case wire forms), version constants
  (`ironclaw.trace_contribution.v1`, policy `2026-04-24`,
  `ironclaw-deterministic-secret-path-v3`, `+server-rescrub-v2`), the
  residual-risk derivation (secret→high, text/payloads→medium, else low), and
  the submission-status set (`accepted`/`quarantined`/`rejected`/`revoked`/
  `expired`/`purged`, all present in the server crate).
- **Corrections made (4):**
  1. Consent matrix `ranking_training` row — added `debugging` ✓;
     `default_allowed_uses_for_scope` grants `Debugging` for
     `RankingTraining` (it was omitted).
  2. Consent matrix `model_training` row — removed `benchmark_generation` ✓;
     `ModelTraining` does **not** grant `BenchmarkGeneration` in code.
  3. Retention table `aggregate_only` row — changed revocable from ✓ to ✗;
     `retention_policy_for_allowed_use` sets `revocable: false` for
     `AggregateAnalytics`.
  4. Consumer condition 1 — removed the `current` qualifier (a derived-record
     status, not a submission status) and listed the actual submission-status
     set instead.
- **Unverifiable / by-reference:** the Phase A/B scoring-hardware claims and the
  novelty/substance gate behavior are documented in `docs/trace-commons.md` and
  not re-derived here; derived benchmark/ranker export record shapes are
  intentionally out of scope and deferred to the export pipeline.
