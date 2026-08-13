# NEAR benchmark corpus handoff — design

Status: approved design, not yet planned or implemented.
Date: 2026-08-13.

## Purpose

Hand the `tenant-zaki-pilot` trace corpus to NEAR as a raw redacted dataset for
benchmarking. NEAR receives redacted trace envelopes, not derived benchmark
artifacts.

This is a one-off operator handoff built on the existing replay-export
subsystem. It is not a new product surface and not a recurring pipeline.

## Corpus definition

The exported set is fixed by deployed configuration and existing code, not by
operator preference. Three independent filters apply, and they compose:

1. `enforce_dataset_export_guardrails` — with
   `TRACE_COMMONS_REQUIRE_EXPORT_GUARDRAILS=true` (set on the pilot), the
   request is rejected with 400 unless it carries an explicit `purpose`,
   `status=accepted`, `privacy_risk=low`, and an explicit `consent_scope`.
2. `is_export_eligible()` — `status == Accepted && !is_revoked()`.
3. `record_matches_export_policy_abac(..., TraceAllowedUse::Evaluation)` — per
   record, against the tenant policy.

Measured against the pilot database on 2026-08-13:

| property | value |
|---|---|
| submissions in tenant | 352 |
| `accepted` | 351 |
| `rejected` | 1 |
| `quarantined` | 0 |
| revoked / withdrawn | 0 |
| carrying `evaluation` in `allowed_uses` | 352 |
| accepted + `privacy_risk=low` + not revoked | 331 |
| of those, carrying `debugging_evaluation` | 331 |
| of those, holding a live `submitted_envelope` object ref | 331 |
| **exportable corpus** | **331** |

The 21 excluded records are 17 `medium` and 3 `high` privacy-risk accepted
traces, plus 1 `rejected`. The guardrail's `privacy_risk=low` requirement is
what excludes the medium and high tiers; they cannot be exported through this
path under the deployed configuration.

This disposes of the credential-exposure concern structurally rather than
procedurally. The server-side re-scrub previously recovered credential-shaped
material in this tenant, and those records carry the elevated risk tiers that
the guardrail refuses. No manual pre-handoff review of individual traces is
required, because the records that would warrant it cannot leave.

The tenant contains no quarantined and no revoked traces, so a
quarantine-inclusive export would select the same rows. No change to
`is_export_eligible()` is required or proposed. That invariant is shared with
`is_benchmark_eligible()` and stays as-is.

## Architecture

Three stages, only the middle one requiring new code.

### 1. Extract — existing route, no changes

`POST /v1/workers/replay-export` with the export-worker bearer credential and
all four guardrail parameters supplied explicitly:

- `purpose=near_benchmark_handoff`
- `status=accepted`
- `privacy_risk=low`
- `consent_scope=debugging_evaluation`

All four are mandatory under the deployed guardrail configuration; omitting any
one returns 400. They are not redundant narrowing — they are the request
contract.

Because the filters are mandatory rather than chosen, they cannot double as
drift detection. Corpus drift is detected instead by comparing the returned
`item_count` against a freshly queried eligible count taken immediately before
the run.

The route creates a one-shot export grant, a validated export job, a manifest,
and a `dataset_export` audit event, and returns `TraceReplayDatasetExport`
containing the redacted envelopes.

### 2. Package — new converter

A script under `scripts/operator/` (not an admin subcommand — a subcommand
implies a supported product surface this does not warrant) that reads the
`TraceReplayDatasetExport` JSON and emits:

- `corpus.jsonl` — one redacted envelope per line.
- `handoff-manifest.json` — `export_id`, `source_submission_ids_hash`,
  `item_count`, the consent basis, and per-trace `submission_id`,
  `privacy_risk`, and a `redaction_counts` summary.

The manifest is provenance rather than risk triage. Since every exported record
is `privacy_risk=low` by construction, the per-trace labels document what was
sent rather than flagging records for differential handling. The
`source_submission_ids_hash` is what lets a later audit prove which submissions
this handoff covered without retaining contributor identity.

The converter is pure data transformation with no database or network access,
so it is testable against a fixture without a live corpus.

### 3. Deliver

**Mechanism: a GCS object with a time-limited, named grant.** The operator
uploads `corpus.jsonl` and `handoff-manifest.json` to a dedicated bucket and
grants the recipient read on those objects.

The recipient already holds operator SSH on the pilot host, so this is **not**
an access restriction and should not be described as one — he could read the
corpus off the box regardless. GCS is chosen for three other reasons:

1. **A defined artifact.** The handoff is a named, checksummed pair of objects,
   not "a file that was on the server at some point." What was handed over
   stays answerable later.
2. **An audit trail.** Cloud Audit Logs record the fetch. A `scp` off the host
   does not, so nothing else can answer "was it actually collected, and when."
3. **An expiry.** The grant lapses on its own. A file left in a home directory
   persists until someone remembers it.

Concretely:

- Upload to a bucket used **only** for third-party handoffs — not the artifact
  bucket behind `trace_artifact_store`. A misconfigured grant on a shared
  bucket exposes far more than the handoff.
- Object layout `near-benchmark-handoff/{export_id}/{corpus.jsonl,
  handoff-manifest.json}`, keyed by `export_id` so the objects tie back to the
  export grant and its `dataset_export` audit event.
- Record a SHA-256 of `corpus.jsonl` **in** `handoff-manifest.json` and report
  it out-of-band, so the recipient can verify he received what was sent and a
  later dispute has a fixed referent.
- **Prefer an IAM read grant to the recipient's Google identity over a signed
  URL.** A signed URL is a bearer capability: anyone holding the link has the
  corpus, it survives in chat logs and browser history, and it cannot be
  revoked before expiry. An IAM grant is attributable and revocable. If a
  signed URL is unavoidable, keep the TTL short and treat the URL itself as
  sensitive.
- Delete the objects once the fetch is confirmed in the audit log. Bucket
  versioning must be off, or deletion leaves a recoverable copy.
- Google-managed encryption at rest is sufficient here: the corpus is redacted,
  policy-excluded down to the `low` privacy-risk tier, and already covered by
  the `evaluation` allowed-use. CMEK is not proposed.

Withdrawal terms are stated in writing at handoff. `trace_withdrawals` and
`trace_community_withdrawal_evictions` let a contributor withdraw a trace from
this system, but nothing propagates that to a third party holding raw
envelopes. This limitation is contractual, not technical, and must be recorded
as such rather than implied to contributors. Delivering to an object the
operator can delete does **not** change this: once fetched, the copy is beyond
recall, and the expiry bounds re-fetch, not retention.

## Error handling

- **Missing object ref aborts the whole job.** With
  `TRACE_COMMONS_DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS=true`, a submission
  lacking an active `submitted_envelope` object ref fails the entire export
  job, not just that record. Coverage measured 331 of 331 on 2026-08-13, so the
  run is expected to succeed, but coverage must be re-checked immediately
  before any run rather than assumed from this document.
- **Export job failure.** The route fails the job row via
  `fail_export_job_with_internal_error`. Operators should use
  `/v1/admin/export/jobs/{id}/retry` rather than issuing a fresh grant, so one
  handoff maps to one grant lineage.
- **`item_count` below the pre-run eligible count.** Treat as a stop condition,
  not a warning. A short count means the corpus, consent data, object-ref
  coverage, or tenant policy changed since the pre-run query.
- **Empty or zero-row database reads.** Every operator query behind this work
  runs under forced RLS. A session without the correct
  `trace_commons.trace_tenant_id` GUC returns zero rows with no error — note
  the doubled `trace_` in the setting name. Every query must lead with a
  `SELECT trace_current_tenant_id()` self-check, and a zero result must be
  treated as an unproven read rather than an empty corpus.

## Testing

- Converter unit tests against a fixture `TraceReplayDatasetExport`: JSONL line
  count matches `item_count`; every manifest entry has a corresponding JSONL
  line; `source_submission_ids_hash` is carried through unmodified.
- A fixture exercising an envelope large enough to confirm line-oriented output
  handles it; the pilot corpus contains traces in the hundreds of kilobytes.
- No new server tests: stage 1 adds no server code.

Fixtures for this converter must not be authored from the converter's own
output. A fixture and its consumer written together will agree with each other
whether or not either is correct; derive the fixture from a real export run.

## Resolved preconditions

Recorded because each was a live risk during design and each was settled by
measurement rather than assumption.

- **Tenant policy.** `trace_tenant_policies` holds no row for this tenant and
  `TRACE_COMMONS_DB_TENANT_POLICY_READS=true`. This is safe only because
  `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY` is absent from the deployed
  environment and therefore false, so the tenant falls back to the development
  default instead of failing closed. If that variable is ever enabled, a policy
  row granting the `evaluation` use must exist before any export runs.
- **Consent basis.** All 331 exportable records carry the
  `debugging_evaluation` consent scope and the `evaluation` allowed use, which
  is what authorizes replay export under `docs/trace-commons.md`.
- **Object-ref coverage.** 331 of 331, with all 352 `submitted_envelope` refs
  live and none invalidated or deleted.

## Open question

**Contributor attribution.** The tenant holds 13 distinct contributor
principals. One accounts for 326 of 352 submissions across roughly two months;
the remainder hold between 1 and 8 each. If the dominant principal is not the
tenant operator, the great majority of this corpus belongs to another
contributor who submitted into an operator-owned tenant rather than
self-enrolling. Their submitted consent scope permits this export, but the
handoff should still be confirmed with them directly. Resolution: check
`trace_account_principals` for that principal before delivery.

## Non-goals

- Converting traces into benchmark tasks. That is the `benchmark_conversion`
  dataset kind and a different consent gate (`benchmark_generation`).
- Exporting the 21 excluded records. Loosening the guardrail or the export
  eligibility invariant to reach them is out of scope.
- A general third-party export path with enforceable withdrawal propagation.
  Worth revisiting if third-party handoffs become routine; not justified by a
  single handoff.
- Any change to quarantine handling or the PII backstop backlog. Those are
  separate concerns tracked elsewhere.
