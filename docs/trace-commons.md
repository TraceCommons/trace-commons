# Trace Commons

Trace Commons is an opt-in pipeline for contributing locally scrubbed IronClaw
traces to a user-owned register of agent work. Capture and redaction both happen
on the contributor's machine; only the scrubbed envelope reaches the server.
There, two gates decide whether a record enters the register — a **novelty**
gate (is this genuinely different from everything already filed?) and a
**substance** gate (is this real work rather than template-shaped filler?). Both
must pass. Accepted envelopes are signed, dated, and filed, and frontier labs,
auditors, and regulators can later query the register under selective
disclosure.

This document is the authoritative reference for **server ingest, the threat
model, auth/upload claims, and the operational API surface**, and it doubles as
the operator guide for the hosted ingestion service. The envelope *schema* and
its consent/privacy/allowed-use semantics live in
[`docs/trace-spec.md`](trace-spec.md) (the protocol crate
`crates/trace-commons-protocol` is the machine-checkable source of truth);
storage boundaries live in `docs/trace-commons-storage.md`; the phase plan lives
in `docs/trace-commons-roadmap.md`.

> **Two audiences, two halves.** Parts 1–2 are conceptual and contributor-facing
> — read them to understand what Trace Commons is and how to contribute traces.
> Parts 3–4 are the operator reference and the security/status surface — read
> them to deploy, configure, and reason about the trust boundaries. If you only
> want to understand the system, stop after Part 2.

---

# Part 1 — Understanding Trace Commons

## What it is

A trace is a recorded run of agent work. Trace Commons lets a contributor offer
a redacted version of that run to a shared register. The defining property is
**local-first**: the raw trace never leaves the contributor's machine. A
deterministic redaction pass runs locally and produces an
`ironclaw.trace_contribution.v1` envelope — consent metadata, redacted summary,
scores, and provenance — and only that envelope is ever uploaded.

The register is not a bulk corpus dump. Accepted records are queried through
scoped, audited, API-mediated access, so contributors, reviewers, and downstream
consumers each see only what they are authorized to see.

This contract is distinct from replay-trace fixtures used in deterministic
tests. Replay traces drive tests; contribution envelopes carry consent,
redaction metadata, replayability metadata, gate scores, revocation, and
contributor credit.

## The two gates

Every submitted envelope is judged on two independent axes before it can enter
the register:

- **Novelty** — is the trace meaningfully different from what is already filed?
  Exact canonical-summary hash matches are the strongest duplicate signal;
  beyond that, the server falls back to deterministic redacted-summary
  similarity, and (when configured) private vector-backed nearest-neighbor
  scoring.
- **Substance** — is the trace substantive work rather than boilerplate or
  template-shaped filler?

A record must pass both. Anything that looks medium- or high-privacy-risk is
held for manual review rather than auto-accepted.

In Phase A, gating runs on regular GPU hardware inside NEAR AI's TEE-hosted
vLLM. The Phase B milestone moves scoring inside attested hardware that even the
server's operators cannot read.

## Trace Credit

A **Trace Credit** is the signed, on-chain record that a contributor's envelope
was accepted into the register. Credits are non-transferable, settle on NEAR,
and are how recognition flows back when buyers later pay to query the evidence.

Credit is deliberately staged so that an accepted submission is not treated as
"paid" until downstream value is proven:

- The client computes a **local pending estimate** from a trace value
  scorecard. The scorecard keeps privacy risk, quality, replayability, capped
  novelty, duplicate penalty, coverage, difficulty, dependability, and
  correction value as separate components before producing the online score.
- The initial credit event records that estimate as **pending**, not settled.
- Later utility — benchmark conversion, ranker training, regression catches,
  reviewer adjustments — appends **delayed credit events** to an append-only
  ledger.
- Only an explicit settlement step turns eligible pending credit into settled,
  non-transferable account credit on NEAR.

See [How credit works](#how-credit-works) for the mechanics and
[Credit settlement and NEAR issuance](#credit-settlement-and-near-issuance) for
the operator controls.

## Local-first guarantees

These rules are invariants. Capture is off by default, and nothing leaves the
machine without an explicit standing opt-in.

- Trace contribution is **off by default**.
- Raw traces stay local. The client submits only
  `ironclaw.trace_contribution.v1` envelopes produced by deterministic local
  redaction.
- Uploads require a standing opt-in policy with an explicit ingestion endpoint.
- Message text and tool payloads are **excluded** unless the user opts those
  fields in explicitly.
- Medium/high privacy-risk traces can be held for manual review by policy.
- A local PII sidecar (such as OpenAI Privacy Filter) may only contribute *safe
  summaries*: redacted text, allow-listed label counts, and warnings. It must
  never serialize original text or raw detected-span contents.

Redaction preserves useful within-trace structure rather than flattening
everything to one token: stable placeholders like `<PRIVATE_EMAIL_1>` and
`<PRIVATE_LOCAL_PATH_1>` keep distinct entities distinguishable. Tool-specific
structured redaction treats email, calendar, messaging, browser, filesystem, and
database payload fields as sensitive *before* generic secret/path scrubbing.

The sidecar contract and its sandboxing are described in
[Privacy Filter sidecar](#privacy-filter-sidecar).

## The submission envelope

The envelope is the contract between client and server. Its authoritative schema
and consent/privacy/allowed-use semantics are defined in
[`docs/trace-spec.md`](trace-spec.md), derived from the protocol crate
`crates/trace-commons-protocol/src/trace_contribution.rs` (if the two disagree,
the crate wins). The table below is a high-level orientation, not the normative
field list. The MVP intentionally reserves several fields for later processing
without implementing the whole central pipeline yet, so the on-the-wire shape is
stable even while the server side fills in.

| Field | Carries |
|-------|---------|
| `trace_card` | Consent scope, allowed uses, source channel, tool categories, retention policy, and revocation handle. |
| `value_card` | Score version, the full scorecard, limitations, and the user-visible credit explanation. |
| `canonical_summary_for_embedding` | A redacted-only summary used for embedding and duplicate detection. |
| `embedding_analysis` | Canonical-summary hash, vector IDs, nearest traces, clusters, duplicate score, novelty score, and coverage tags — filled once a private worker runs. |
| `hindsight` | Optional later labels (subgoal / recoverability) that keep failed traces useful. |
| `training_dynamics` | Optional dataset-cartography labels such as easy / ambiguous / hard. |
| `contributor` | Pseudonymous attribution only: `pseudonymous_contributor_id` and a separate `tenant_scope_ref`. **Never** an authorization input — see [Multitenant permissioning](#multitenant-permissioning-trust-model). |

## Lifecycle of a trace

1. **Capture** — a recorded trace exists locally.
2. **Redact** — deterministic local redaction (plus optional Privacy Filter
   sidecar) produces an envelope. Raw content stays on disk.
3. **Queue / submit** — under an enabled standing policy, the envelope is queued
   and flushed to the ingestion endpoint.
4. **Re-scrub and score** — the server treats every upload as untrusted: it
   validates the schema and consent, re-runs redaction, and recomputes privacy
   hashes and the credit estimate.
5. **Gate** — novelty and substance gates run. Low-risk passing traces are
   accepted; medium/high-risk traces are quarantined with zero immediate credit.
6. **Review** — reviewers decide quarantined records.
7. **Credit** — an accepted record gets pending credit; downstream utility later
   appends delayed credit, and settlement turns eligible credit into NEAR
   account credit.

Revocation and retention can move a record out of the register at any later
point, fanning out to every derived artifact (see
[Production hardening roadmap](#production-hardening-roadmap)).

## How credit works

Each local submission record stores append-only credit events. The first event
records the accepted submission estimate as pending; it is not treated as
settled final credit unless a later review or utility process finalizes it.

Delayed credit can be appended only through privileged, audited paths:

- **Benchmark conversion** and **ranker candidate/pair exports** append
  idempotent delayed utility events for the accepted sources they include.
- **Trusted offline utility jobs** can append `regression_catch`,
  `training_utility`, or `ranking_utility` credit for accepted traces.
- **Reviewer value and abuse penalties** stay on the reviewer/admin mutation
  paths.

Shapley-style or influence estimates can inform offline analysis but are never
exposed as direct, immediate payment logic.

Contributors refresh their credit/status by listing their own known submission
ids:

```http
POST /v1/contributors/me/submission-status
Authorization: Bearer <tenant-token>

{ "submission_ids": ["..."] }
```

The response contains only records visible to the authenticated principal.
Missing ids are simply omitted, which keeps cross-tenant and same-tenant
cross-principal probes indistinguishable from genuinely unknown submissions.

Status records keep estimates and settled credit separate:

- `credit_points_pending` — the online estimate.
- `credit_points_final` — present only when explicit final settlement exists.
- `credit_points_ledger`, `credit_points_total`, `delayed_credit_explanations`
  — present once review or downstream jobs award later utility credit.

When delayed ledger events exist, `credit_points_total` is computed as explicit
final credit **plus** the delayed ledger delta (not pending estimate plus
ledger). If a trace is later revoked, expired, or purged, status sync reports a
zero delayed ledger and a safe explanation that retained ledger events are
excluded.

Reviewers/admins append delayed credit once downstream utility is known:

```http
POST /v1/review/{submission_id}/credit-events
Authorization: Bearer <reviewer-token>

{
  "event_type": "benchmark_conversion",
  "credit_points_delta": 2.5,
  "reason": "Converted into replay benchmark run 2026-04-25",
  "external_ref": "benchmark-run:2026-04-25:trace-commons"
}
```

---

# Part 2 — Contributing traces

## Command-line interface

The client owns capture, redaction, queueing, and the standing policy. The
static submit token is read from `IRONCLAW_TRACE_SUBMIT_TOKEN` by default and is
never written into the policy file.

```bash
# Enable autonomous submission after local redaction.
ironclaw traces opt-in \
  --endpoint https://trace-ingest.internal/v1/traces \
  --scope debugging-evaluation

# Create a local redacted envelope from an existing recorded trace.
ironclaw traces preview --recorded-trace trace.json --output contribution.json

# Queue a redacted envelope (or preview and queue in one step).
ironclaw traces enqueue --envelope contribution.json
ironclaw traces preview --recorded-trace trace.json --enqueue

# Submit eligible queued envelopes using the standing policy.
ironclaw traces flush-queue

# See local credit totals and recent explanations.
ironclaw traces credit

# Acknowledge or snooze a due periodic credit notice.
ironclaw traces credit --notice --ack
ironclaw traces credit --notice --snooze-hours 24

# Inspect local queue readiness (no trace bodies exposed).
ironclaw traces queue-status

# Disable autonomous contribution.
ironclaw traces opt-out
```

`preview` is local and needs no opt-in. `enqueue`, `preview --enqueue`, and
autonomous flush all run through the same standing-policy gate: the policy must
be enabled, must have an ingestion endpoint, and must allow any message text or
tool payloads already present in the redacted envelope.

**Hosted tenants** can refresh short-lived EdDSA upload claims from an issuer
instead of holding a long-lived bearer token:

```bash
ironclaw traces opt-in \
  --endpoint https://trace-ingest.internal/v1/traces \
  --upload-token-issuer-url https://issuer.near.com/v1/trace-upload-claim \
  --upload-token-issuer-allowed-hosts issuer.near.com \
  --upload-token-audience trace-commons \
  --upload-token-tenant-id tenant-a \
  --upload-token-workload-token-env IRONCLAW_TRACE_WORKLOAD_TOKEN
```

When an issuer is configured, queue flush, explicit `traces submit`, status
sync, and remote revoke request short-lived bearer claims over HTTPS. The client
requires exact issuer-host allowlisting, rejects embedded URL
credentials/query/fragment/internal targets, requires the returned claim to be
an EdDSA/Ed25519 JWT with a `kid`, caches it only in process memory until its
refresh margin, and retries once with a forced refresh after a 401/403. Optional
workload credentials stay in the configured environment variable and are never
written to the policy.

Other contributor-facing helpers: `process-evaluation-submit`,
`tenant-access-grants-list` / `tenant-access-grant-create` /
`tenant-access-grant-revoke`, `tenant-principal-ref` (derives the stored
`principal_sha256:...` value without printing raw credentials), `tenant-policy-get`
/ `tenant-policy-set`, `benchmark-lifecycle-update`, `retention-jobs-list` /
`retention-job-items`, and `export-access-grants-list` / `export-jobs-list`.

## Autonomous submission policy

The local policy lives at `~/.ironclaw/trace_contributions/policy.json` and
controls: the endpoint and bearer-token environment variable, default consent
scope, whether redacted message text or tool payloads may be included, tool
filters, the minimum local submission score, whether medium-risk traces require
manual review, and the periodic credit-notice interval.

Queue writes are durable: same-directory temp files, file sync, atomic rename,
and best-effort parent-directory sync. A flush under an enabled policy compacts
duplicate queued envelopes and orphan held sidecars, quarantines malformed
queued files into a local `queue_malformed` directory (so they don't block valid
submissions), submits eligible traces, honors typed retry backoff for transient
failures, and prints a credit update when the notice interval has elapsed.

The runtime also schedules an autonomous post-turn contribution pass after a
response is persisted or a turn fails, and a long-running queue worker scans
opted-in scoped queues on an interval. Both verify thread/user ownership before
building an envelope, redact locally, queue, flush, and drain due credit-notice
outbox items — recording delivery success or a sanitized failure rather than
silently consuming notice state.

`queue-status` reports opt-in state, endpoint/credential presence, capture
toggles, queued/held counts, typed retry/manual-review/policy-hold counts, the
next retry time, durable flush telemetry, sanitized failure-class counts, and
the local credit summary — never trace bodies or raw observed values.

In the authenticated web gateway, policy, queue, ledger, and revocation state
are scoped under a hashed user/tenant directory rather than the global CLI
policy. The web settings panel has a Trace Commons tab and the chat composer has
a Trace button; both preflight the scoped standing policy and cannot widen
capture beyond it.

## Privacy Filter sidecar

A local Privacy Filter sidecar can be enabled with
`IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` and optional whitespace-split
`IRONCLAW_TRACE_PRIVACY_FILTER_ARGS`. The sidecar receives `{"text":"..."}` on
stdin and must return Privacy Filter-style JSON on stdout. IronClaw keeps only
the safe `redacted_text` and an aggregate `SafePrivacyFilterSummary` — dropping
raw `text`, raw span contents, raw offsets, and unsafe span labels. Unsupported
span labels are mapped to `unknown` so a malformed sidecar cannot smuggle
emails, paths, or tokens through label names.

The sidecar runs as an untrusted local subprocess with a cleared environment
except `PATH`, `LANG`, and `LC_ALL`. Sidecar failures are non-fatal: the client
falls back to deterministic local redaction rather than uploading raw content.

| Variable | Effect |
|----------|--------|
| `IRONCLAW_TRACE_PRIVACY_FILTER_COMMAND` | Sidecar executable. Enables the sidecar path. |
| `IRONCLAW_TRACE_PRIVACY_FILTER_ARGS` | Optional whitespace-split arguments. |
| `IRONCLAW_TRACE_PRIVACY_FILTER_TIMEOUT_MS` | Per-call timeout guardrail. |
| `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_INPUT_BYTES` | Max stdin payload. |
| `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDOUT_BYTES` | Max stdout accepted. |
| `IRONCLAW_TRACE_PRIVACY_FILTER_MAX_STDERR_BYTES` | Max stderr captured (hashed). |

---

# Part 3 — Operating the ingestion service

The ingestion binary (`trace-commons-ingest`) is a development/internal control
plane for the hosted register. It treats every upload as untrusted: it validates
the schema and revocable consent, re-runs deterministic redaction, recomputes
privacy hashes and credit estimates, enforces optional submission quotas, stores
accepted low-risk traces under the authenticated tenant, and quarantines
medium/high-risk traces with zero immediate credit.

On submit it also writes a derived redacted-only record: canonical summary +
hash, hash-based duplicate precheck, deterministic redacted-summary novelty
score, coverage tags (channel, tool, tool category, outcome, failure mode,
privacy risk), and aggregate analytics counts.

The current API is file-backed under `TRACE_COMMONS_DATA_DIR` for easy local
operation, with opt-in DB-backed read flags layered on top. This repo owns the
production storage path; the cutover sequence is in
[Storage](#storage-database-mirror-and-object-store).

## Running the service

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2026-04-27T00:00:00Z,tenant-a:reviewer:review-token-a,tenant-b:dev-token-b' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin trace-commons-ingest
```

Then point a client at it:

```bash
ironclaw traces opt-in --endpoint http://127.0.0.1:3907/v1/traces --scope debugging-evaluation
```

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_TENANT_TOKENS` | Static bearer tokens. See [Authentication](#authentication-and-tokens). | — |
| `TRACE_COMMONS_BIND` | Listen address. | — |
| `TRACE_COMMONS_DATA_DIR` | File-backed storage root. | — |
| `DATABASE_URL` | PostgreSQL connection for the DB mirror. | — |

## API surface

Health and contributor submission:

- `GET /health`
- `POST /v1/traces`
- `GET /v1/traces` — reviewer filters: status, privacy risk, consent scope,
  derived tool/tag metadata, export/provenance `purpose`
- `DELETE /v1/traces` / `DELETE /v1/traces/{submission_id}`
- `POST /v1/traces/{submission_id}/revoke`

Contributor self-service:

- `GET /v1/contributors/me/credit`
- `GET /v1/contributors/me/credit-events`
- `POST /v1/contributors/me/submission-status`

Review and analytics:

- `GET /v1/review/quarantine`, `GET /v1/review/active-learning`,
  `GET /v1/review/routing-summary`
- `POST /v1/review/batch-decisions` (up to 50 ids),
  `POST /v1/review/{submission_id}/decision`
- `POST` / `DELETE /v1/review/{submission_id}/lease`
- `POST /v1/review/{submission_id}/credit-events`
- `GET /v1/analytics/summary` — optional `release_scope=operator|broad`

Datasets, benchmarks, rankers (export and worker routes):

- `GET /v1/datasets/replay`, `GET /v1/datasets/replay/manifests`
- `POST /v1/benchmarks/convert`, `POST /v1/benchmarks/{conversion_id}/lifecycle`
- `POST /v1/workers/benchmark-convert`,
  `/v1/workers/benchmark-evaluations/run`,
  `/v1/workers/benchmark-registry-publications/run`
- benchmark registry outbox: `GET /v1/admin/benchmark-registry-outbox`,
  `POST /v1/workers/benchmark-registry-outbox/{submit,confirm,mark-status}`
- replay/ranker export workers: `GET|POST /v1/workers/replay-export`,
  `GET|POST /v1/workers/ranker/training-candidates`,
  `GET|POST /v1/workers/ranker/training-pairs`
- read-only ranker exports: `GET /v1/ranker/training-candidates`,
  `GET /v1/ranker/training-pairs`

Credit, settlement, NEAR:

- `POST /v1/workers/utility-credit`, `POST /v1/workers/utility-attestations`
- `GET /v1/admin/credit-attestations`,
  `GET|POST /v1/admin/credit-holds`,
  `POST /v1/admin/credit-holds/{hold_id}/release`,
  `GET /v1/admin/credit-risk-summary`
- `GET|POST /v1/admin/credit-settlements`,
  `GET|POST /v1/admin/credit-settlement-approvals`,
  `POST /v1/workers/credit-settlements/run`
- credit cycle: `POST /v1/workers/credit-cycle/run`,
  `POST /v1/workers/credit-cycle/scheduler/run`
- NEAR outbox: `GET /v1/admin/near-credit-outbox`,
  `POST /v1/workers/near-credit-outbox/{submit,confirm,mark-status}`

Ranking evidence and reports (full list under
[Ranking](#ranking-calibration-and-credit)):
`GET|POST /v1/admin/ranking/model-versions`,
`/v1/admin/ranking/calibration-datasets`, `/v1/admin/ranking/model-promotions`,
the `GET /v1/admin/ranking/*-report` family, `/v1/admin/ranking/worker-runs`, and
the `POST /v1/workers/ranking/*` evidence/credit/calibration routes.

Tenant, export-job, vector, retention, revocation, audit, and config:

- `GET|POST|PUT /v1/admin/tenant-policy`,
  `GET|POST /v1/admin/tenant-access-grants`,
  `POST /v1/admin/tenant-access-grants/{grant_id}/revoke`
- `GET /v1/admin/export/access-grants`, `GET /v1/admin/export/jobs`,
  `POST /v1/admin/export/jobs/{id}/{recover-stale,retry}`,
  `POST /v1/workers/export/jobs/{claim-next,claim-and-run,run-queued,retry-failed}`
- `GET /v1/admin/vector-entries`, `POST /v1/workers/vector-index`
- `GET /v1/admin/retention/jobs`, `GET /v1/admin/retention/jobs/{id}/items`,
  `POST /v1/workers/retention-maintenance`
- `POST /v1/workers/revocation-propagation`
- `POST /v1/admin/maintenance`
- `GET /v1/admin/config-status`
- the `GET|POST /v1/admin/rollout-smoke/*` and `POST /v1/admin/*-drill` surfaces
  — see [Operational readiness](#operational-readiness-drills-and-rollout-smoke)
- `GET /v1/audit/events` — optional `limit` (default 100, max 500)

Shared list-read policy: an omitted `limit` defaults to 100, and explicit limits
outside `1..=500` are rejected with a client error rather than silently clamped.

## Authentication and tokens

Authentication is layered as a deliberate bridge-to-production progression.
**Static tokens and HS256 claims are bridges for controlled pilots; production
upload claims are EdDSA/Ed25519-only.** Authorization is always derived from the
authenticated request, never from envelope fields.

**Static tokens.** Configure `TRACE_COMMONS_TENANT_TOKENS` as comma-separated
entries. Use `tenant_id:token` for contributor access or `tenant_id:role:token`
for a scoped role. Either form may append `;expires_at=<RFC3339>` (or
`;expires=...`) for a short-lived bridge. Expired tokens are rejected before
tenant attribution, and the principal hash is computed from the secret token
value only, not the expiry metadata.

Recognized roles: `contributor`, `reviewer`, `admin`, plus the worker roles
`export_worker`, `retention_worker`, `revocation_worker`, `vector_worker`,
`benchmark_worker`, `utility_worker`, and `process_eval_worker` (also accepted as
`process_evaluation_worker`). **Worker roles do not inherit reviewer
visibility** — each dedicated worker route rejects ordinary reviewer tokens
before any source, evaluator, registry, or settlement check. See
[Workers and schedulers](#workers-and-schedulers).

**Signed claims.** Instead of enumerating every token, the service can verify
signed JWT claims that bind tenant id, actor/principal, role, allowed consent
scopes/uses, issuer, audience, `sub`, and expiry. Claims must include
`tenant_id`, `exp`, and either `principal_ref` or `sub`; `role` defaults to
`contributor`. The verifier rejects unsupported JWT algorithms.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_SIGNED_TOKEN_SECRET` | Enables the HS256 bridge path. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_SECRETS` | Comma-separated `kid:secret` for HMAC rotation; tokens with a `kid` must match. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_PEM` / `_FILE` | EdDSA/Ed25519 public key (default key). | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_PUBLIC_KEY_FILES` | Comma-separated `kid:path` for keyed rotation. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_JSON` / `_FILE` / `_URL` | Managed keyset (entries: `kid`, `public_key_pem`, optional `not_before`/`not_after`). | — |
| `TRACE_COMMONS_SIGNED_TOKEN_ISSUER` / `_AUDIENCE` | Require matching `iss` / `aud`. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_MAX_TTL_SECONDS` | Require `iat`; reject tokens whose `exp - iat` exceeds the bound. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_REQUIRE_JTI` | Require every claim to carry a JWT ID. | `false` |
| `TRACE_COMMONS_SIGNED_TOKEN_REVOKED_JTIS` | Comma-separated emergency `jti` denylist. | — |
| `TRACE_COMMONS_REQUIRE_EDDSA_SIGNED_TOKENS` | Reject static + HS256 on every route; requires ≥1 EdDSA key at startup. | `false` |
| `TRACE_COMMONS_REQUIRE_MANAGED_EDDSA_SIGNED_TOKENS` | Stricter: accept only active `kid`-selected managed-keyset claims with issuer/audience checks; reject default/ad hoc keys, missing/unmanaged `kid`. | `false` |

**Guarded HTTPS keysets.** The `_URL` keyset source refreshes live after startup
so issuer-owned Ed25519 keys can rotate without a restart. It must use HTTPS,
requires an exact host allowlist, rejects embedded credentials / query /
fragment / internal targets, disables redirects, pins validated DNS, and
size-caps the response.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_ALLOWED_HOSTS` | Exact host allowlist (required for the URL source). | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_BEARER_TOKEN` | Optional fetch credential for the issuer endpoint. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_TIMEOUT_MS` | Startup + refresh fetch timeout. | — |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_REFRESH_INTERVAL_SECONDS` | Live refresh cadence. | 300 |
| `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL_MAX_STALE_SECONDS` | With managed-EdDSA mode, fail closed once the last good refresh is older than this; failed refreshes keep the last good keyset. | — |

`config-status` reports total/active/inactive/managed key-count aggregates and
refresh health (interval, max-stale, last success/failure, stale state) — never
key material, key ids, URLs, host allowlists, fetch credentials, or activation
timestamps. Submitted-trace audit rows record only the safe auth method
(`static_token` or `signed_claim`) plus the hashed actor principal.

**The standalone issuer.** The `trace-commons-upload-claim-issuer` binary is the
first production-shaped issuer for hosted tenants. It exposes
`POST /v1/trace-upload-claim`, `GET /health`, and
`GET /.well-known/trace-commons-ed25519-keyset.json`; authenticates workload
tokens with EdDSA/Ed25519 only (rejecting RSA); signs short-lived contributor
claims with `kid`, `iss`, `aud`, `iat`, `exp`, `jti`; and enforces workload
tenant/scope/allowed-use narrowing. Configure it with
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_*` variables (bind address, signing key PEM/file,
signing `kid`, issuer, audience, max TTL, workload public key, optional workload
issuer/audience checks). With
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS=true` it connects
through `DATABASE_URL` and refuses to mint a claim unless the workload actor has
an active contributor grant.

## Tenant policy and access grants

Two complementary mechanisms control what a tenant may submit and consume.

**Submission policy** (ABAC) restricts allowed consent scopes and trace-card
uses at ingest and export time:

```bash
TRACE_COMMONS_TENANT_POLICIES='{
  "tenant-a": {
    "allowed_consent_scopes": ["debugging_evaluation", "benchmark_only"],
    "allowed_uses": ["debugging", "evaluation", "benchmark_generation", "aggregate_analytics"]
  }
}' \
cargo run --bin trace-commons-ingest
```

Tenants without an entry keep the development default. Where a policy exists it
also gates downstream use: replay exports require the `evaluation` use,
benchmark conversion requires `benchmark_generation`, ranker exports require
`ranking_model_training`, and vector indexing requires at least one derived-use
permission. The `aggregate_analytics` use is intentionally **insufficient** for
vector indexing, because that retention class does not permit derived artifacts.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_TENANT_POLICIES` | JSON map of per-tenant allowed scopes/uses. | dev default |
| `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY` | Reject submissions/exports from tenants with no explicit policy. | `false` |
| `TRACE_COMMONS_DB_TENANT_POLICY_READS` | Read submission/export policy from DB `trace_tenant_policies` (+ `_TENANT_IDS` allowlist). | `false` |

Admin tokens manage the current tenant's policy through
`GET|POST|PUT /v1/admin/tenant-policy` (or `ironclaw traces tenant-policy-get/set`);
reads and writes are audited with hash-only policy metadata.

**Access grants** (`trace_tenant_access_grants`) add a DB-backed
hosted-tenant permissioning gate:

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS` | Require an active, unrevoked, unexpired grant with the same role as the request token across submit, credit/status reads, reviewer/audit reads, review mutations, export paths, non-revocation worker mutations, maintenance, and admin reads. Requires DB dual-write. | `false` |

Grant consent-scope and allowed-use allow-lists *intersect* with any token/claim
allow-lists and cannot upgrade a token's role. For signed EdDSA claims, any
issuer/audience/subject stored on the grant must also match the verified claim,
so an issuer-authorized grant cannot be replayed across those boundaries.
Static-token bridge grants ignore those signed-claim binding fields. Deprovision
and recovery routes (revocation, config-status, tenant-policy admin,
tenant-access-grant admin) stay available even under this gate.

## Storage: database mirror and object store

The default file-backed store is fine for local work. Production moves through
three stacked layers — **mirror metadata to PostgreSQL**, **serve reads from the
mirror**, then **store bodies in encrypted object storage** — each behind opt-in
flags so the cutover is observable.

### Layer 1 — DB dual-write

`TRACE_COMMONS_DB_DUAL_WRITE=true` builds a PostgreSQL-backed mirror from
`DATABASE_URL`. It writes tenant-scoped submissions, policies, access grants,
object refs, derived precheck records, export manifests + item snapshots, audit
events, the full credit/settlement/hold/attestation/NEAR-outbox control plane,
review state, revocation tombstones, retention ledgers, and ranking
evidence/calibration rows — with the redaction-count aggregates and derived
summary/tool/coverage metadata the DB-backed read paths need. API reads still
use the file store until you opt specific surfaces in.

### Layer 2 — DB-backed reads (canary by tenant)

Each read gate has a `*_TENANT_IDS` companion for tenant-by-tenant promotion.
Dependency gates must cover the same tenants before fail-closed object-ref or
object-primary modes can turn on.

| Variable | Effect |
|----------|--------|
| `TRACE_COMMONS_DB_CONTRIBUTOR_READS` | Serve `/v1/contributors/me/*` from the mirror. |
| `TRACE_COMMONS_DB_REVIEWER_READS` | Serve reviewer/admin metadata, queues, exports, credit/ranking lists, review leases from the mirror. |
| `TRACE_COMMONS_DB_REVIEWER_REQUIRE_OBJECT_REFS` | Make DB-backed review decisions fail closed without an active submitted-envelope object ref. |
| `TRACE_COMMONS_DB_REPLAY_EXPORT_READS` | Select replay-export eligibility/metadata from the mirror. |
| `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS` | Fail closed instead of falling back to a file body for replay export. |
| `TRACE_COMMONS_DERIVED_EXPORT_REQUIRE_OBJECT_REFS` | Require active, tenant/hash-verifiable object refs for every benchmark/ranker source. |
| `TRACE_COMMONS_DB_AUDIT_READS` | Serve `/v1/audit/events` from the mirror. |

Review decisions are allowed only for live quarantined submissions
(accepted/rejected/revoked/expired/purged are rejected before any body read, and
approvals are blocked for aggregate-only retention classes). Review leases are
tenant-scoped, bound to the authenticated principal, reclaimable by the same
principal or after expiry, and cleared when a trace leaves quarantine.

### Layer 3 — encrypted object store and object-primary cutover

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_ARTIFACT_KEY_HEX` | Enables encrypted trace object storage (required by every encrypted mode). | — |
| `TRACE_COMMONS_ENCRYPTED_ARTIFACTS` | Explicit guard for the legacy encrypted artifact sidecar (still needs the key). | — |
| `TRACE_COMMONS_ARTIFACT_DIR` | Sidecar / fallback artifact directory. | — |
| `TRACE_COMMONS_OBJECT_STORE` | `local_service` (service-owned local encrypted) or `remote_service`. | file |
| `TRACE_COMMONS_SERVICE_OBJECT_STORE_DIR` | Root for `local_service` (else `ARTIFACT_DIR`, else `DATA_DIR/service_object_store`). | — |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER` | `file_system` works; AWS/GCS/Azure are fail-closed scaffolds. | — |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET` | Absolute path used as the remote root (filesystem provider). | — |
| `TRACE_COMMONS_REMOTE_OBJECT_STORE_FILE_SYSTEM_VERSIONING` | Archive-on-delete + restore-after-delete rehearsal for the filesystem remote. | `false` |
| `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING` | Cloud-cutover startup guard: fail unless the store reports versioning + restore-after-delete. | `false` |
| `TRACE_COMMONS_REQUIRE_DB_MIRROR_WRITES` | During cutover: turn mirror-write failures into hard errors instead of silent file-only state. Requires DB dual-write. | `false` |
| `TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN` | Reject maintenance that omits `reconcile_db_mirror`, fail closed (409) on blocking gaps. Also requires required-mirror-writes before DB reader promotion. | `false` |

**Object-primary modes** skip plaintext envelope body files entirely. Each
requires DB dual-write, required mirror writes, the matching DB reads, object-ref
reads, and an enabled service-owned store; each has a `*_TENANT_IDS` companion.

| Variable | Effect |
|----------|--------|
| `TRACE_COMMONS_OBJECT_PRIMARY_SUBMIT_REVIEW` | Omit plaintext submitted/reviewed body files; keep compatibility metadata/audit. |
| `TRACE_COMMONS_OBJECT_PRIMARY_REPLAY_EXPORT` | Replay bodies read via DB object refs, no file fallback. |
| `TRACE_COMMONS_OBJECT_PRIMARY_DERIVED_EXPORTS` | Benchmark/ranker artifacts only in encrypted object storage; DB manifest/items remain the durable index. |

**Remote object deletion** (for revocation of disabled cloud refs):

| Variable | Effect |
|----------|--------|
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_URL` | Trusted deletion adapter; must return a canonical `sha256:` evidence hash before the ref is marked deleted. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_BEARER_TOKEN` | Bearer auth for the deleter. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_TIMEOUT_MS` | Call timeout. |
| `TRACE_COMMONS_REMOTE_OBJECT_DELETER_REQUIRE_EXTERNAL` | Require the adapter for promotion even before a disabled cloud alias is active. |

Tenant directories are a trust boundary in file-backed mode too: before
persisting tenant-local rows, the server validates embedded tenant ids and
storage refs, and normalizes any server-looking `contributor.tenant_scope_ref`
to the auth-derived tenant so a forged envelope reference cannot become a
read-time trap.

Maintenance reconciliation reports reader-projection parity, credit-ledger and
audit-event ID gaps, control-plane drift, object-ref integrity, and compact
`blocking_gaps` so operators can verify each layer before promoting it.

## Export guardrails and submission quotas

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS` | Require explicit low-risk, accepted-status, consent-scoped filters (plus ranking/model-training filters for ranker exports) and caller-supplied purposes. | `false` |
| `TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST` | Per-request item cap for replay/benchmark/ranker exports; requests above it are clamped. | 500 |
| `TRACE_COMMONS_MAX_SUBMISSIONS_PER_TENANT_PER_HOUR` | Hourly contributor-upload quota per tenant. | disabled |
| `TRACE_COMMONS_MAX_SUBMISSIONS_PER_PRINCIPAL_PER_HOUR` | Hourly contributor-upload quota per principal. | disabled |

Quotas apply only to contributor tokens, count active accepted/quarantined
submissions in the last hour, and never block idempotent retries of an existing
submission id. Revoked, expired, and purged submissions stop consuming quota.

Every export carries a deterministic `sha256:` hash of its source-item list,
mirrored into the audit `decision_inputs_hash` so reviewers get a stable proof
of which submissions fed a dataset without seeing trace content.

## Workers and schedulers

Each derived-data pipeline has a dedicated worker route (reviewer tokens
rejected) and, optionally, an in-process scheduler that calls that same route on
an interval without granting broader authority. Scheduler tokens, reasons,
purposes, and refs are never returned by `config-status`.

**Export jobs.** `claim-next`, `claim-and-run`, `run-queued` (bounded), and
`retry-failed` (bounded exponential backoff). An export job preserves a
`trace_export_job_request.v1` metadata snapshot for replayable execution.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_TOKEN` | Export-worker token; enables the in-process loop (retry-failed then run-queued). | — |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 60 |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_DATASET_KIND` | Optional dataset-kind narrowing. | — |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RUN_QUEUED_MAX_JOBS` | Per-pass run cap (1..=50). | 10 |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_JOBS` | Per-pass retry cap (1..=50). | — |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_FAILED_MAX_RETRY_COUNT` | Max retries per job (1..=25). | — |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_BASE_DELAY_SECONDS` | Backoff base (0..=86400). | — |
| `TRACE_COMMONS_EXPORT_JOB_SCHEDULER_RETRY_MAX_DELAY_SECONDS` | Backoff ceiling (≥ base). | — |

**Vector indexing.** `POST /v1/workers/vector-index` indexes eligible DB-mirrored
derived summaries (no expire/purge/reconcile side effects). Embeddings come only
from redacted summaries; aggregate-only traces are skipped.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_VECTOR_EMBEDDER_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Trusted private embedding adapter (hash-only request, no raw bodies). | — |
| `TRACE_COMMONS_VECTOR_EMBEDDER_REQUIRE_EXTERNAL` | Fail closed when no embedder is configured (else deterministic local fallback). | `false` |
| `TRACE_COMMONS_VECTOR_SEARCH_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Trusted private vector-search adapter; candidate ids are revalidated against active tenant DB metadata. | — |
| `TRACE_COMMONS_VECTOR_SEARCH_REQUIRE_EXTERNAL` | Fail closed at startup/worker when no search adapter is configured. | `false` |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_TOKEN` | Vector-worker token; enables the in-process indexer. Requires DB mirror. | — |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 60 |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_LIMIT` | Per-pass cap (1..=500). | — |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_DRY_RUN` | Count-only mode. | `false` |
| `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_PURPOSE` | Audited purpose. | — |

**Process evaluation.** Workers submit bounded process-quality metadata via
`POST /v1/workers/process-evaluation`, and schedulers run bounded batches via
`/v1/workers/process-evaluations/run`. Optional `ranking_label` output stores
only deterministic evidence/external-ref hashes.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_PROCESS_EVALUATOR_URL` | External evaluator adapter (derived summaries + hashes only). | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_TOKEN` | Process-eval worker token; enables the server-owned loop. | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_EVALUATOR_REF` | Required evaluator ref. | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_REQUIRE_EXTERNAL_EVALUATOR` | Require the adapter (startup fails without `PROCESS_EVALUATOR_URL`). | `true` |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 300 |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_LIMIT` | Per-pass cap (1..=100). | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_DRY_RUN` | Ledger-only, no evaluator/trace/label writes. | `false` |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_TARGET_USE` | Ranking target use (requires `_EXTERNAL_REF_PREFIX`). | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_EXTERNAL_REF_PREFIX` | Idempotent label external-ref prefix. | — |
| `TRACE_COMMONS_PROCESS_EVALUATION_SCHEDULER_UTILITY_CATEGORY` | Overrides the ranking-training default. | — |

**Benchmark conversion + registry.** Conversion produces tenant-scoped candidate
artifacts; publication requires `passed` evaluator metadata; a hash-only registry
outbox carries publish/revoke rows.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_BENCHMARK_EVALUATOR_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Trusted evaluator adapter (hash-only request). | — |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SUBMITTER_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Registry submit adapter. | — |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_CONFIRMATION_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Registry confirmation adapter. | — |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_REQUIRE_ADAPTER_AUTH` | Fail closed unless the matching adapter bearer token is configured. | `false` |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_TOKEN` | Benchmark-worker token; enables the outbox loop (submit + confirm). | — |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 60 |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_SUBMIT_LIMIT` / `_CONFIRM_LIMIT` | Per-pass caps (1..=500). | — |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_DRY_RUN` | Count-only audit mode. | `false` |
| `TRACE_COMMONS_BENCHMARK_REGISTRY_SCHEDULER_PURPOSE` | Audited purpose. | — |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_TOKEN` | Benchmark-worker token for the evaluate-then-publish pipeline. | — |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_EVALUATOR_REF` / `_REGISTRY_REF_PREFIX` | Required refs. | — |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_REQUIRE_EXTERNAL_EVALUATOR` | Require `BENCHMARK_EVALUATOR_URL` at startup. | `true` |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 60 |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_EVALUATION_LIMIT` / `_PUBLICATION_LIMIT` | Per-pass caps (1..=100). | — |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_MIN_SCORE` | Evaluator threshold. | — |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_DRY_RUN` | Rehearse counts only. | `false` |
| `TRACE_COMMONS_BENCHMARK_PIPELINE_SCHEDULER_REASON` | Audited reason. | — |

**Retention and revocation workers.** `POST /v1/workers/retention-maintenance`
(purge automation, reviewer tokens rejected) and
`POST /v1/workers/revocation-propagation` (idempotent fan-out of invalidation,
credit reversal, and physical deletion). See
[Retention and legal hold](#retention-and-legal-hold).

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Hash-only worker-queue invalidator for revocation propagation; must return `sha256:` evidence. | — |
| `TRACE_COMMONS_WORKER_CACHE_INVALIDATOR_REQUIRE_EXTERNAL` | Fail closed for production revocation propagation (startup + per-item). | `false` |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_TOKEN` | Retention-worker token; enables the in-process loop. | — |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 300 |
| `TRACE_COMMONS_RETENTION_MAINTENANCE_SCHEDULER_DRY_RUN` / `_PRUNE_EXPORT_CACHE` / `_MAX_EXPORT_AGE_HOURS` / `_PURGE_EXPIRED_BEFORE` / `_PURPOSE` | Bounded maintenance controls. | — |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_TOKEN` | Revocation-worker token; enables the in-process loop. Requires DB mirror. | — |
| `TRACE_COMMONS_REVOCATION_PROPAGATION_SCHEDULER_INTERVAL_SECONDS` / `_LIMIT` / `_DRY_RUN` / `_PURPOSE` | Bounded loop controls. | — |

## Analytics and the privacy budget

`GET /v1/analytics/summary` returns content-free aggregate counts by status,
risk, tool, and coverage. Cells below a configured threshold are suppressed
before the response leaves the service, and the response carries a
`privacy_budget` object with released/suppressed counts and broad-release
blocker reasons. `release_scope=broad` is a publication preflight that **fails
closed** unless the budget is broad-release ready, keyed count-noise is
configured, and the epsilon cap is not exhausted.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_ANALYTICS_MIN_CELL_COUNT` | Suppress aggregate cells below this count (`k`-anonymity). | disabled |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_KEY` | Required for broad release; absent → `noise_not_configured`. | — |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_NOISE_MAX_DELTA` | Positive integer count-noise bound. | 1 (when key present) |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_EPSILON_MICROS` | Epsilon charged per broad release (audited). | — |
| `TRACE_COMMONS_ANALYTICS_BROAD_RELEASE_MAX_EPSILON_MICROS` | Tenant epsilon cap; later releases fail with `privacy_budget_exhausted`. | — |

A rigorous DP mechanism/proof remains future work before stronger public
publication claims; this is `k`-anonymity plus keyed count-noise and an epsilon
ledger, not a formal DP guarantee.

## Retention and legal hold

Retention windows are derived from consent scope and allowed use, not driven
directly by the envelope's `trace_card.retention_policy` (which is a suggestion).
Review, maintenance, and all export/source-selection paths cross-check the
stored policy id and `expires_at` against the server-derived retention class
before reading bodies — and fail closed if file metadata tries to extend
`expires_at` beyond the allowed window.

| Variable | Effect |
|----------|--------|
| `TRACE_COMMONS_LEGAL_HOLD_RETENTION_POLICIES` | Comma-separated central retention policy IDs exempt from expiration/purge — honored only when the stored id matches the server-derived class. Exposed (IDs only) by `config-status`. |

A non-dry-run purge requires an explicit RFC3339 `purge_expired_before` cutoff
*and* a non-empty `purpose`; dry-run previews may omit purpose and never delete.
Revocation-propagation object deletes upsert a `PhysicalDeleteReceipt` row so
retries can backfill the receipt and reconciliation can distinguish invalidation
from proven payload removal.

## Credit settlement and NEAR issuance

Settlement converts eligible pending utility credit into non-transferable,
settled account credit, optionally writing a hash-only NEAR outbox row per
account. This is the most governed path in the service: the **central issuer
profile** layers several fail-closed gates so early issuance stays centralized.

Settlement runs through `POST /v1/admin/credit-settlements` (dry-run or final) or
the narrower worker route `POST /v1/workers/credit-settlements/run`. Approval is
source-list bound: operators run a dry-run/drill to obtain the canonical
`source_list_hash`, then record a hash-only approval via
`POST /v1/admin/credit-settlement-approvals` for the exact `policy_version`,
`source_list_hash`, and `evidence_hash`. Admin requests use `source_event_limit`;
worker/scheduler requests use `limit` — keep the value identical across
rehearsal, approval, and live settlement.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_CREDIT_SETTLEMENT_MAX_POINTS_PER_ACCOUNT` | Per-account live-settlement cap (points); `0` disables. Unset → `settlement_account_cap_missing` blocker. | unset |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_ALLOWED_POLICY_VERSIONS` | Comma-separated approved policy versions; live settlement rejects unlisted versions. | — |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ISSUER_APPROVAL` | Require a recorded source-list approval hash before live settlement. | `false` |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_ISSUER_APPROVAL_MAX_AGE_HOURS` | Require the recorded approval to still be fresh. | — |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_NEAR_CONTRACT_ID` | Central non-transferable credit contract; request `near_contract_id` must match. | — |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_NEAR_CONTRACT` | Fail live settlement unless the contract is configured. | `false` |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` | Allowlist of hashed issuer principals permitted to write positive credit / mutate external mirrors. | — |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE` | Require the full central issuer profile (managed-EdDSA, access grants, principal refs, rollout-smoke). | `false` |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_ROLLOUT_SMOKE_READY` | Require `rollout_smoke.ready: true` before live issuance. | `false` |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN` | Utility-worker token; enables the server-owned settlement loop. | — |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 300 |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_LIMIT` | Per-pass source cap (1..=500). | 100 |
| `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION` / `_DRY_RUN` (+ optional approval/contract/model/target gates) | Bounded scheduler controls. | — |

When the principal allowlist is configured, unlisted principals can still
inspect dry-runs but cannot record approvals, finalize settlement, or write
positive credit through any path (delayed-credit mutation, utility-credit jobs,
attestations, ranking prediction-credit, export-generated credit). Non-positive
reviewer adjustments such as abuse penalties stay available. `GET
/v1/admin/credit-risk-summary` gives a bounded, account-hashed pre-issuance view
of pending/held/over-cap credit; held accounts are released via
`POST /v1/admin/credit-holds/{hold_id}/release` (admin-only, non-empty reason).

**NEAR outbox adapters** deliver receipts. Submit records public transaction
hashes; confirm binds to the exact submitted hash; both keep payloads hash-only.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Trusted NEAR relayer (submit). | — |
| `TRACE_COMMONS_NEAR_CREDIT_CONFIRMATION_URL` / `_BEARER_TOKEN` / `_TIMEOUT_MS` | Relayer confirmation endpoint. | — |
| `TRACE_COMMONS_NEAR_CREDIT_REQUIRE_ADAPTER_AUTH` | Require adapter bearer auth for live submit/confirm and the manual mark-status fallback. | `false` |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_TOKEN` | Utility-worker token; enables the server-owned receipt loop. | — |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 60 |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_SUBMIT_LIMIT` / `_CONFIRM_LIMIT` | Per-pass caps (1..=500). | — |
| `TRACE_COMMONS_NEAR_CREDIT_OUTBOX_SCHEDULER_DRY_RUN` / `_PURPOSE` | Bounded loop controls. | — |

Credit holds also mirror account-state transitions into the outbox: the first
active hold enqueues `freeze_credit_account`; releasing the last active hold
enqueues `unfreeze_credit_account`. Operational summary and metrics surface
missing cap, required NEAR contract, adapter, adapter-auth, and central-issuer
gates as promotion blockers whenever positive delayed credit exists.

**Utility credit jobs.** Trusted offline workers append delayed credit for
accepted traces through `POST /v1/workers/utility-credit` (or the CLI wrapper
`ironclaw traces worker-utility-credit --bearer-token-env
TRACE_COMMONS_UTILITY_CREDIT_WORKER_TOKEN ...`). The route accepts admin or
utility-worker credentials, rejects ordinary reviewer tokens, and is limited to
`regression_catch`, `training_utility`, and `ranking_utility` event types — never
`reviewer_bonus` or `abuse_penalty`. For model-derived ranking credit, prefer
`POST /v1/workers/ranking/prediction-credit` so the server derives the amount and
external ref from the active-model prediction; a `ranking_utility` event is
credit-bearing only when its external ref is a single `ranking_prediction:<uuid>`
for the same source.

### Credit-cycle automation

For the full ranking-to-settlement sequence, `POST /v1/workers/credit-cycle/run`
delegates to the bounded calibration-run, model-promotion, prediction-credit,
credit-settlement, and NEAR outbox workers in order for one
model/policy/target — recording a `credit_cycle` worker-run row and rejecting
overlapping live non-stale cycles. The NEAR submit/confirm steps default to
dry-run unless explicitly enabled. `POST /v1/workers/credit-cycle/scheduler/run`
is the cron-style surface: it scans latest candidate and active models, prefers
candidates, runs a read-only calibration preflight (applying deployment-owned
floors even when a request supplies looser values), and reports per-candidate
decisions. `preflight_only: true` stops after eligibility checks without creating
any worker rows, credit events, settlement batches, or NEAR outbox rows.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_TOKEN` | Utility-worker token; enables the server-owned scan loop. Must be allowlisted for live cycles when the central issuer allowlist is set. | — |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_INTERVAL_SECONDS` | Loop interval. | 300 |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_TARGET_USE` | Target use scanned each pass. | `ranking_model_training` |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_LIMIT` | Bounded candidate-selection cap. | — |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_DRY_RUN` | Inspect-only mode. | `false` |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_PREFLIGHT_ONLY` | Eligibility decisions only; no side effects. | `false` |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_SUBMIT_NEAR_OUTBOX` | Run the live NEAR submit step. | `false` |
| `TRACE_COMMONS_CREDIT_CYCLE_SCHEDULER_CONFIRM_NEAR_OUTBOX` | Run the live NEAR confirm step. | `false` |

If central issuer approval or rollout-smoke readiness is required, keep the
scheduler in preflight/dry-run mode and finalize through the settlement route
after recording the source-list approval. `config-status` reports only safe
scheduler booleans, interval, target use, limit, and whether
model/policy/contract filters are configured — never the token, raw reason, or
NEAR contract id.

## Ranking, calibration, and credit

Ranking turns trace evidence into model-derived utility credit. It is offline
analysis: outputs may append delayed credit, but never become immediate payment
signals. Credit-bearing ranking models must clear server-owned floors that
workers can tighten but never loosen.

Models are immutable per `model_version` (changing feature schema, policy,
training/calibration dataset, or artifact hash needs a new version). Admins stage
`candidate` models, then activate through `POST /v1/admin/ranking/model-promotions`
only when the calibration run is promotable, the latest matching evidence is
still promotable, the holdout registry gate is satisfied, the backtest passes,
and any freshness window holds. Training and calibration dataset hashes must be
**disjoint** so the calibration set acts as holdout evidence. Calibration sample
counts use latest-per-`(submission, target_use, label_source)` effective labels;
a latest `disputed` label removes that source until a newer non-disputed label
arrives.

Label sources are authority-bound: utility workers write `frontier_lab`,
reviewer/admin tokens write `reviewer`, benchmark workers write `benchmark`,
process-evaluation workers write `system`, and admins may override for repairs.

Reports for evaluating a candidate without exposing trace content: the
adjudication, labeler-reliability, calibration, pairwise-evaluation,
model-backtest, model-risk, dataset-readiness, and credit-readiness report
routes under `GET /v1/admin/ranking/*`, plus the hash-only worker-run ledger at
`GET /v1/admin/ranking/worker-runs`.

| Variable | Effect | Default |
|----------|--------|---------|
| `TRACE_COMMONS_RANKING_MIN_LABEL_COUNT` | Min joined prediction/label count to promote or back credit (1..=1000000). | 1 |
| `TRACE_COMMONS_RANKING_MIN_LABEL_SOURCE_COUNT` | Min distinct label sources (1..=4); production typically 2. | 1 |
| `TRACE_COMMONS_RANKING_MIN_CONFIDENCE_THRESHOLD` | Per-prediction confidence floor (0..=1). | 0 |
| `TRACE_COMMONS_RANKING_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS` | Aggregate + per-source calibration-error ceiling (micros). | 1000000 |
| `TRACE_COMMONS_RANKING_CALIBRATION_MAX_AGE_HOURS` | Freshness window for promotion, prediction-credit, settlement. | unset |
| `TRACE_COMMONS_RANKING_REQUIRE_CALIBRATION_DATASET_REGISTRY` | Require a registered holdout calibration dataset. | `false` |
| `TRACE_COMMONS_RANKING_REQUIRE_ACTIVE_CALIBRATION_DATASET` | Require the registry row to be `active` (needs the registry gate). | `false` |
| `TRACE_COMMONS_RANKING_MIN_PAIRWISE_LABEL_COUNT` | Min joined pairwise preference labels (0..=1000000). | 0 |
| `TRACE_COMMONS_RANKING_MIN_PAIRWISE_ACCURACY_MICROS` | Min pairwise ordering accuracy (micros); nonzero requires pairwise label count > 0. | 500000 |
| `TRACE_COMMONS_RANKING_MAX_LABELER_ISSUE_RATE_MICROS` | Block credit if any calibration label source/actor exceeds this issue rate. | unset |
| `TRACE_COMMONS_RANKING_MIN_LABELER_RELIABILITY_LABEL_COUNT` | Min labeler total-label support floor. | unset |
| `TRACE_COMMONS_RANKING_REQUIRE_SERVER_FEATURE_PROVENANCE` | Require `feature_provenance:server_derived` for prediction-credit/readiness/settlement. | `false` |

Model-derived credit prefers `POST /v1/workers/ranking/prediction-credit` (one
idempotent `ranking_utility` event bound to `ranking_prediction:<uuid>` from a
positive active-model prediction). Raising production floors retroactively blocks
old calibration runs from activating models, minting credit, or settling.

## PostgreSQL row-level security

PostgreSQL RLS is forced on every Trace Commons table; tenant predicates go
through `trace_current_tenant_id()`, and the runtime serving role must be a
non-owner role without `BYPASSRLS`. Migrations may run as an owner/admin role.

| Variable | Effect |
|----------|--------|
| `TRACE_COMMONS_REQUIRE_POSTGRES_TRACE_RLS_READY` | Fail startup unless every table has the policy installed, RLS + FORCE RLS enabled, matching predicates, and a non-bypassing runtime role. Requires DB dual-write + `DATABASE_URL`. |
| `TRACE_COMMONS_POSTGRES_RUNTIME_ROLE_SHA256` | Pin the expected serving-role hash; startup, the RLS drill, and promotion gates block on mismatch without exposing role names. |

`config-status` and the RLS drill report catalog-only readiness (policy counts,
predicate mismatches, disabled/force-RLS tables, runtime-role hash + matched
booleans) — never row data or table contents. Operational summary promotes unsafe
diagnostics into `postgres_trace_rls_not_ready` and
`postgres_trace_runtime_role_hash_mismatch` blockers.

## Operational readiness: drills and rollout smoke

Each risky pipeline has a **drill** that produces hash-only evidence, and a
**rollout-smoke** preflight that aggregates required checks before promotion.
Drills never expose trace bodies, object keys, tokens, or raw operator reasons —
only readiness booleans, counts, blocker codes, and `sha256:` evidence hashes.

`GET /v1/admin/rollout-smoke/preflight` returns the required-check block plus the
latest evidence per check. `GET|POST /v1/admin/rollout-smoke/evidence` lists and
records evidence (`GET` supports `latest_only=true`; `POST` names one check, a
`passed`/`failed` status, a canonical `sha256:` hash, and an optional external
ref stored only as a hash).

| Drill route | Proves |
|-------------|--------|
| `POST /v1/admin/canary-read-drill` | submit-status, tenant isolation, contributor credit, reviewer metadata, replay selection, audit reads for a canary submission |
| `POST /v1/admin/object-primary-read-drill` | object-ref-only read path; no plaintext bodies; fallback tenant outside rollout |
| `POST /v1/admin/object-store-migration-drill` | write/read/delete cycle on the service-owned store (optional `require_versioning`) |
| `POST /v1/admin/db-reconciliation-drill` | clean file-vs-DB reconciliation without side effects |
| `POST /v1/admin/rollback-drill` | file vs DB submission/audit/tombstone parity without rewriting either side |
| `POST /v1/admin/key-rotation-drill` | managed-EdDSA rotation health (≥2 active managed keys, issuer/audience/JTI/TTL, fresh refresh) |
| `POST /v1/admin/postgres-rls-drill` | RLS readiness from the safe config-status diagnostics |
| `POST /v1/admin/retention-dry-run-drill` | retention/cache selection through the real dry-run path |
| `POST /v1/admin/vector-index-drill` | vector indexing through the real selector in dry-run |
| `POST /v1/admin/analytics-release-drill` | broad-release analytics preflight (cell count, noise, epsilon) |
| `POST /v1/admin/benchmark-readiness-drill` | benchmark artifacts, evaluator/registry/adapter-auth readiness |
| `POST /v1/admin/credit-settlement-drill` | settlement promotion (risk summary + dry-run selector + central issuer profile) |
| `POST /v1/admin/revocation-propagation-drill` | propagation through the real worker dry-run path |
| `POST /v1/admin/revocation-effects-drill` | post-live proof: reversed credit, NEAR reversals, deleted refs, queue invalidation |
| `POST /v1/admin/audit-chain-drill` | audit-chain verification without maintenance side effects |

Required smoke checks include `tenant_canary_isolation`,
`db_reconciliation_clean`, `rollback_flag_drill`, `key_rotation_drill`,
`revocation_propagation`, `delayed_credit_reversal`, `object_deletion_refs`,
`worker_queue_invalidation`, `retention_dry_run`, `vector_index`,
`analytics_release`, `ranking_model_readiness`, `credit_settlement`,
`object_store_migration`, `postgres_rls_readiness`, and
`audit_chain_verification`. The preflight reports recorded / passed / failed /
stale / missing evidence separately from promotion-gate readiness.

`POST /v1/admin/maintenance` is the operator workhorse for cutover. Its boolean
inputs — `reconcile_db_mirror`, `backfill_db_mirror`, `index_vectors`,
`verify_audit_chain`, `dry_run`, and `purge_expired_before` + `purpose` — drive
reconciliation reporting, file→DB backfill, vector indexing, audit-chain
verification, and bounded purge respectively. Maintenance audit reasons store
`purpose_hash` plus bounded counters, never raw operator text.

---

# Part 4 — Security and status

## Multitenant permissioning (trust model)

The single most important rule: **authorization is derived from the
authenticated request or runtime identity, never from fields inside a submitted
envelope.** Envelope fields such as `contributor.pseudonymous_contributor_id`
and `contributor.tenant_scope_ref` are attribution/provenance metadata only.

**Local capture.** Web preview and autonomous runtime capture use the
authenticated `user_id` as the trace scope; conversation history is read through
ownership checks before an envelope is built; local policy/queue/history/
revocation/credit live under `trace_contributions/users/<hash>`. The envelope
carries a stable pseudonymous contributor id and a separate pseudonymous tenant
scope reference — neither contains the raw user id.

**Ingestion service.** Every request binds to a tenant from AuthN/AuthZ
credentials (tenant-scoped token, mTLS, or EdDSA upload claim). The auth-derived
tenant id is the storage partition; envelope tenant references are never
partition keys. The service rejects requests where the authenticated tenant may
not submit for the claimed scope. RBAC/ABAC scopes contributors to their own
submissions and credit, reviewers to permitted tenants' quarantined traces, and
trainer jobs to approved slices through controlled jobs. The corpus is never a
raw bulk download; researchers, trainers, and reviewers read approved slices
through scoped API routes that write tenant-scoped manifests and read-audit rows.

## Threat-model checklist

Use this for any change touching trace capture, redaction, ingestion, review,
export, credit, or derived datasets.

- **Raw trace non-upload** — raw recorded traces never leave the client; only
  `ironclaw.trace_contribution.v1` envelopes produced after local redaction may
  be submitted.
- **Frontend untrusted** — treat gateway UI requests as user-controlled. Re-check
  auth, tenant ownership, policy scope, and conversation ownership on the server
  before previewing, queueing, submitting, listing, or revoking.
- **Sidecar output stripping** — reject/strip Privacy Filter fields that can
  carry original text, raw span text, unneeded raw offsets, or unknown nested
  payloads.
- **Token isolation** — submit/review/admin tokens must not appear in policy
  files, envelopes, queue files, sidecar stdin, logs, or exported datasets.
- **Tenant isolation** — every read/write binds to the auth-derived tenant and
  actor. Contributor-supplied `tenant_scope_ref`,
  `pseudonymous_contributor_id`, `submission_id`, and `revocation_handle` are not
  authorization inputs.
- **Role isolation** — contributors cannot list quarantine, append delayed
  credit, read analytics, export datasets, or probe other contributors'
  submissions. Reviewers/admins cannot bypass tenant scope.
- **Bulk export controls** — dataset export requires an authorized role, explicit
  purpose, consent/use filter, privacy-risk filter, review-state filter, output
  manifest, and a per-source audit event.
- **Delayed credit abuse** — delayed credit append must be privileged,
  append-only, audited, policy-bounded, and linked to a concrete downstream
  artifact or review decision.
- **Revocation propagation** — revocation blocks future status changes, review
  approval, vector indexing, benchmark conversion, ranking/training use, and
  export; existing derived artifacts need invalidation or removal.
- **Retention bypass** — retention jobs must cover central metadata, object
  storage, vector entries, benchmark artifacts, export caches, worker queues, and
  local references where applicable.
- **Canary secret tests** — feed synthetic API keys, bearer tokens, local paths,
  emails, tenant ids, user ids, and tool-payload secrets through regression
  fixtures and assert none survive in accepted envelopes or derived summaries.
- **Audit completeness** — any path that reads or mutates central trace content,
  credit, review state, export state, or revocation state must emit a
  tenant-scoped audit event.

**Protected web API endpoints** (authenticated gateway): `GET|PUT
/api/traces/policy`, `POST /api/traces/{preview,submit,flush}`, `GET
/api/traces/credit`, `GET|POST /api/traces/credit-notice`, `GET
/api/traces/queue-status`, `GET /api/traces/submissions`, `POST
/api/traces/submissions/{submission_id}/revoke`. Local preview stays available
without opt-in, but enqueue / manual-submit / autonomous acceptance preflight the
scoped standing policy and cannot widen capture beyond it.

## Production hardening roadmap

The current implementation is a usable MVP for local development and controlled
internal pilots. A production deployment needs the following before broad tenant
rollout.

**DB and object storage.** Promote the dual-write mirror into relational reads
for all surfaces and service-owned encrypted object storage for bodies. Keep
metadata/object keys tenant-scoped from the auth-derived tenant. Store immutable
submission records, append-only credit events, revocation tombstones, review
decisions, export manifests, and job state as separate records. Encrypt objects
at rest, require TLS, keep bucket access behind service identities, and never
expose raw bucket access or bulk snapshots. Version every derived artifact with
input envelope hash, worker version, policy version, and output id.

**Tenant RBAC/ABAC.** Move beyond static tokens to issuer-managed EdDSA upload
claims. Enforce RBAC for contributor/reviewer/admin/export-job/worker roles and
ABAC for consent scope, allowed use, privacy risk, review state, retention,
revocation, export purpose, and data residency. Keep vector workers under the
same ABAC as export/utility workers. Require explicit reasons for privileged
operations like tombstone deletion. Treat envelope contributor ids as
pseudonymous attribution only.

**Audit and reviewability.** Append-only audit events for every trace read,
write, review decision, credit mutation, revocation, export, purge, and derived
artifact — tamper-evident, queryable by reviewers without exposing content. Add
sampled reconciliation across object storage, metadata, vector ids, export
manifests, credit ledger, and tombstones.

**Retention and deletion.** Define windows by consent scope and allowed use.
Implement retention jobs that remove/tombstone metadata, objects, vectors,
benchmark artifacts, export caches, and queued outputs. Keep tombstones long
enough to prevent re-ingest after deletion. Block processing/export for
revoked/expired submissions and invalidate existing derived artifacts.

**Revocation propagation.** Treat revocation as a state transition fanning out to
object storage, review queues, vector indexes, benchmark sets, ranking/training
queues, export jobs, and credit ledgers. Make it idempotent and tenant-scoped.
Require workers to check central revocation state immediately before reading
content and before publishing a derived artifact. Reconcile derived artifacts
whose source is revoked.

**Vector / ranking / benchmark.** Embed only redacted summaries and approved
fields — never raw traces or unreviewed high-risk content. Keep vector ids
tenant-scoped and source-linked for deletion on revocation/retention. Keep
ranking/model-utility jobs offline; their outputs may append delayed credit, not
immediate payment. Convert approved traces to benchmark/replay datasets through
controlled jobs that record consent scope, review state, redaction version,
replay requirements, and manifest id, and fail closed on revoked/expired/
unapproved/missing-replayability sources.

**Privacy Filter sidecar operations.** Run sidecars as untrusted local
subprocesses/containers with timeouts, output-size limits, and no access to
Trace Commons credentials. Pass only the minimum text needed. Accept only the
safe projection. Treat failures as non-fatal warnings with deterministic
fallback. Add canary-secret tests.

## Implementation status

Legend: **MVP** = implemented for local/internal pilots; **Partial** = working
but with production hardening still open.

| Area | Status | Summary |
|------|--------|---------|
| Local opt-in policy / opt-out | MVP | CLI + scoped web/runtime policy files; submit tokens and issuer workload creds stay in env; hosted tenants can use a guarded HTTPS upload-claim issuer. |
| Local preview / queue / flush / credit | MVP | Local redacted envelopes, atomic queue writes, malformed-envelope quarantine, scoped `queue-status`, and acknowledgeable/snoozable periodic credit notices via a local retry outbox. |
| Deterministic local redaction | MVP | Generic secret/path scrubbing, stable placeholders, tool-aware payload handling, Privacy Filter safe projection. |
| Privacy Filter sidecar | MVP | Command/stdin/stdout path with safe projection, non-fatal fallback, minimal env, stderr hashing, IO limits, canary tests. Container sandboxing still open. |
| Autonomous post-turn / periodic contribution | MVP | Runtime queues/flushes scoped envelopes only under an enabled policy with an endpoint and an eligible envelope; periodic agent-loop worker with typed retry backoff, in-memory EdDSA claim refresh, compaction, and credit-notice drain. |
| Web settings + preview endpoints | MVP | Authenticated gateway endpoints and UI controls; server-side tenant/user checks are the trust boundary; queue/submit preflight scoped opt-in. |
| Private ingestion service | MVP | Validates schema/consent, re-runs redaction, computes hashes/credit, optional hourly quotas, stores accepted/quarantined records, serves review/status/export routes; can dark-launch DB dual-write + encrypted artifacts. |
| Tenant token roles | Partial | Static tokens (contributor/reviewer/admin + scoped workers) with optional expiry; HS256 and EdDSA signed claims with allow-lists; managed-keyset enforcement; tenant access grants binding signed-claim issuer/audience/sub. Fuller central RBAC/ABAC + RLS hardening open. |
| Contributor credit ledger + delayed credit | Partial | Append-only local/central events; pending kept separate from settled; reason-gated reviewer mutation; ABAC-gated utility credit; per-account cap + policy-version allowlist + issuer-approval gates; revocation reversal with NEAR reverse receipts. Deeper fraud-review + issuer governance open. |
| Quarantine / review workflow | Partial | Reviewer/admin decide quarantined traces; SLA/escalation + DB-backed leases; reason-required, lease-respecting, terminal/aggregate-only-safe decisions; bounded batch decisions; object-ref-backed body reads + `review_snapshot` mirror. Richer assignment policy + external router open. |
| Replay dataset export | Partial | Reviewer/admin export of approved slices with guardrails, purposes, access-grant checks, DB-backed selection, object-ref body reads, durable manifests + item snapshots, dedicated worker/scheduler routes. Cloud object storage + broader bulk controls + revocation of published artifacts open. |
| Analytics summary | Partial | Aggregate counts by status/risk/tool/coverage + process-eval aggregates; min-cell suppression, privacy-budget accounting, keyed count-noise, epsilon ledger, fail-closed broad release. Rigorous DP proof open. |
| Production DB + encrypted object storage | Partial | V1 PostgreSQL schema, `TraceCorpusStore`/`PgBackend`, DB mirror across all control planes, encrypted local + filesystem-remote object stores, object-primary modes, physical-delete execution, backfill/reconciliation diagnostics, RLS policies + FORCE RLS. Cloud remote read/write + parity enforcement + non-bypassing role hardening open. |
| Central audit log | Partial | Hash-chained file rows + maintenance verifier; DB mirror with canonical-payload recomputation and `audit_sequence`; typed hash-only metadata across all privileged surfaces; bounded reviewer reads at the storage boundary. |
| Retention enforcement | Partial | Server-derived retention ids/expiry; mismatch fail-closed across review/export/conversion; maintenance + dedicated worker mark/purge with legal-hold exemptions and durable ledger rows. Broader cloud purge rehearsal open. |
| Revocation propagation | Partial | Tenant-scoped first-writer-wins tombstones; DB invalidation of object refs/derived/vector/manifest rows; benchmark lifecycle-revoke + registry revoke outbox; settled-credit reversal with NEAR receipts; physical delete of service-owned payloads. Deployed invalidator/deleter ops + rollout rehearsal open. |
| Vector duplicate/novelty index | Partial | DB schema + dedicated worker + metadata indexer + object-ref gating + per-source content-read audits; exact-hash + deterministic-similarity scoring with optional private embedder/search adapters; stale/cross-profile neighbor diagnostics. Deployed vector-store ops + canary evidence open. |
| Ranking/model utility pipeline | Partial | Offline utility-credit worker; immutable model manifests, calibration runs, holdout registry, server-owned floors, backtest/risk/readiness reports, prediction-credit, worker-run ledger, credit-cycle automation. Deployed evaluator ops + gold/holdout stewardship open. |
| Benchmark conversion pipeline | Partial | Tenant-scoped candidate artifacts with lifecycle metadata, source-hash revalidation, audits, provenance, idempotent utility credit, evaluator/registry worker routes + outbox, readiness drill. Deployed external evaluator/registry adapter ops open. |
| Production sidecar operations | Partial | Timeout/IO limits, minimal env, stderr hashing, fallback, safe projection, canary coverage. Container sandboxing + deployment-specific isolation open. |

## Research hooks

The MVP envelope reserves fields for later processing without implementing the
whole central pipeline:

- `trace_card` — consent scope, allowed uses, source channel, tool categories,
  retention, revocation.
- `value_card` — score version, full scorecard, limitations, user-visible credit
  explanation.
- `embedding_analysis` — canonical-summary hashes, vector IDs, nearest traces,
  clusters, duplicate score, novelty score, coverage tags (filled by a private
  worker).
- `hindsight` — later subgoal/recoverability labels that keep failed traces
  useful.
- `training_dynamics` — future dataset-cartography labels (easy / ambiguous /
  hard).
- `canonical_summary_for_embedding` — redacted-only summaries for embedding and
  duplicate detection.
