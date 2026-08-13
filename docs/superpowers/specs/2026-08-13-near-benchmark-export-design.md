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

The exported set is fixed by existing code, not by operator preference.
`is_export_eligible()` in `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
is `status == Accepted && !is_revoked()`, and
`run_dataset_replay_export_job` additionally filters each record through
`record_matches_export_policy_abac(..., TraceAllowedUse::Evaluation)`.

Measured against the pilot database on 2026-08-13:

| property | value |
|---|---|
| submissions in tenant | 352 |
| `accepted` | 351 |
| `rejected` | 1 |
| `quarantined` | 0 |
| revoked / withdrawn | 0 |
| carrying `evaluation` in `allowed_uses` | 352 |
| **export-eligible and evaluation-permitted** | **351** |

Privacy-risk distribution across the accepted set: 331 `low`, 17 `medium`,
3 `high`.

Consent basis: all 352 carry the `debugging_evaluation` consent scope; 8 also
carry `public_attribution`. The `evaluation` allowed-use present on every row is
what authorizes replay export under `docs/trace-commons.md`.

Because the tenant contains no quarantined and no revoked traces, a
quarantine-inclusive export would select the same 351 rows. No change to
`is_export_eligible()` is required or proposed. That invariant is shared with
`is_benchmark_eligible()` and stays as-is.

## Architecture

Three stages, only the middle one requiring new code.

### 1. Extract — existing route, no changes

`POST /v1/workers/replay-export` with the export-worker bearer credential and
`purpose=near_benchmark_handoff`. No `status`, `privacy_risk`, or
`consent_scope` query filters: all 351 already qualify, and adding redundant
filters would silently mask a corpus change on a later run.

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

The per-trace risk labels are the point of this stage. The corpus contains 3
`high` and 17 `medium` privacy-risk traces; shipping them labeled lets the
recipient handle them differently, rather than mixing them invisibly into 331
`low` records.

The converter is pure data transformation with no database or network access,
so it is testable against a fixture without a live corpus.

### 3. Deliver

Preconditions on delivery:

- The 3 `high` privacy-risk traces are reviewed by an operator before handoff.
  The server-side re-scrub previously recovered credential-shaped material in
  this tenant; these are the accepted records carrying that risk tier.
- Withdrawal terms are stated in writing. `trace_withdrawals` and
  `trace_community_withdrawal_evictions` let a contributor withdraw a trace from
  this system, but nothing propagates that to a third party holding raw
  envelopes. This limitation is contractual, not technical, and must be recorded
  as such rather than implied to contributors.

## Error handling

- **Export job failure.** The route already fails the job row via
  `fail_export_job_with_internal_error` and surfaces the error. The runbook
  should direct operators to `/v1/admin/export/jobs/{id}/retry` rather than
  re-issuing a fresh grant, so one handoff maps to one grant lineage.
- **Fewer than 351 items returned.** Treat as a stop condition, not a warning.
  A short count means the corpus, the consent data, or the tenant policy changed
  since this design was measured. Compare `item_count` against a freshly queried
  eligible count before packaging.
- **Empty or zero-row database reads.** Any operator query behind this work runs
  under forced RLS. A session without the correct `trace_commons.trace_tenant_id`
  GUC returns zero rows with no error. Every query in the runbook must lead with
  a `SELECT trace_current_tenant_id()` self-check, and a zero result must be
  treated as an unproven read rather than an empty corpus.

## Testing

- Converter unit tests against a fixture `TraceReplayDatasetExport`: JSONL line
  count matches `item_count`; every manifest entry has a corresponding JSONL
  line; risk labels round-trip.
- A fixture containing a `high` privacy-risk record, asserting it appears in the
  manifest with its label rather than being dropped or unlabeled.
- No new server tests: stage 1 adds no server code.

Fixtures for this converter must not be authored from the converter's own
output. A fixture and its consumer written together will agree with each other
whether or not either is correct; derive the fixture from a real export run.

## Open preconditions

Both must be resolved before implementation begins. Neither blocks writing the
implementation plan.

1. **Tenant policy source.** `trace_tenant_policies` contains no row for this
   tenant. If `TRACE_COMMONS_DB_TENANT_POLICY_READS` is enabled together with
   `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY`, replay export fails closed
   with no policy to satisfy. Resolution: inspect the deployed ingest
   environment for those two variables and for `TRACE_COMMONS_TENANT_POLICIES`.
   If policy is required and absent, create the tenant policy row granting the
   `evaluation` use before running the export.

2. **Contributor attribution.** The tenant holds 13 distinct contributor
   principals. One accounts for 326 of 352 submissions across roughly two
   months; the remainder hold between 1 and 8 each. If the dominant principal is
   not the tenant operator, the great majority of this corpus belongs to another
   contributor, and the handoff should be confirmed with them directly even
   though their submitted consent scope permits it. Resolution: check
   `trace_account_principals` for that principal before delivery.

## Non-goals

- Converting traces into benchmark tasks. That is the `benchmark_conversion`
  dataset kind and a different consent gate (`benchmark_generation`).
- A general third-party export path with enforceable withdrawal propagation.
  Worth revisiting if third-party handoffs become routine; not justified by a
  single handoff.
- Any change to `is_export_eligible()`, quarantine handling, or the PII backstop
  backlog. Those are separate concerns tracked elsewhere.
