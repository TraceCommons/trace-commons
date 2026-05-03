# TraceCommons Server

Hosted server-side control plane for Trace Commons / TraceCommons.

This repository is the extraction point for the pieces that should not live
inside Ironclaw long term: public/private ingest, review queues, tenant access
grants, audit/retention/revocation workers, export jobs, upload-claim issuing,
object storage, and the production database schema.

## Current Extraction Status

This is a bridge extraction from Ironclaw's `gecko-pass` worktree. The hosted
server now owns its database, object-storage, and shared protocol surface
locally: the `TraceCorpusStore` trait, PostgreSQL backend, server migrations,
RLS diagnostics, encrypted artifact-store provider code, contribution envelope
DTOs, status DTOs, and deterministic redaction helpers live in this repo.
It also owns the first server-side Trace Credits settlement surface: hash-only
utility attestations, admin-triggered dry-run/final settlement batches, credit
holds, contributor pending/settled/held projections, and a NEAR non-transferable
credit receipt outbox. Utility workers can also run a bounded credit-cycle
coordinator that sequences calibration, model promotion, prediction credit,
settlement, and NEAR outbox submission checks for one model/policy/target. The
`POST /v1/workers/credit-cycle/scheduler/run` route lets utility-worker cron
jobs select the next eligible candidate or active model for a target/policy and
run at most a bounded number of credit cycles while skipping live claims and
models whose current ranking evidence is not yet promotable. Scheduler
`preflight_only` requests return eligible/skipped candidate decisions without
creating worker rows, credit events, settlement batches, or NEAR outbox rows. The
next ranking substrate is also server-owned: model
version records, hash-only feature records, prediction records, frontier/reviewer
labels, calibration reports, persisted model-promotion calibration runs, and
PostgreSQL-backed ranking evidence reads/writes behind the same DB mirror
cutover gates.

## Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run settlement
batches, and optional NEAR receipt calls are queued only after off-chain
settlement finalizes. The worker `POST /v1/workers/credit-cycle/run` route can
run the production credit path in bounded steps for a single model/version:
calibration, model promotion, prediction credit, settlement, then a NEAR outbox
dry-run or explicit submit. Settlement retries repair missing NEAR outbox rows
from finalized batches, and revocation propagation can append deterministic
negative ledger rows plus `reverse_credit_receipt` NEAR outbox calls for settled
revoked sources. Reviewer/admin credit summaries report tenant-wide settled line
items while contributor summaries stay principal-scoped. With the DB mirror
configured, utility attestations, settlement batches, credit holds, and NEAR
receipt outbox rows are dual-written to PostgreSQL;
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves the admin credit control-plane
lists from the tenant-scoped DB mirror.

The NEAR path is intentionally an outbox of deterministic method-call payloads
for a non-transferable receipt contract. The payload builder only emits
`settle_credit_receipt`, `reverse_credit_receipt`, and `freeze_credit_account`
calls, rejects malformed NEAR account ids, and rejects any other NEAR credit
method. Configure
`TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` to let the scoped submit worker hand
pending or failed-retry calls to an operator-owned NEAR relayer; the server then
records the public transaction hash or a hashed failure. Workers can still
manually mark items submitted, confirmed, or failed for fallback operations. The
server ledger remains authoritative; NEAR payloads contain batch ids, account
hashes, source-list hashes, policy versions, amounts, and issuer-signature
hashes, never trace bodies or raw contributor identity.
`GET /v1/admin/config-status` exposes only safe NEAR readiness fields for this
path: whether a submitter is configured, the configured submit timeout, outbox
submit bounds, and the credit-cycle step count. It does not expose the relayer
URL, bearer token, hosts, or contributor identity.

Ranking evidence is stored separately from settlement. Workers can register
feature hashes, model predictions, lab/reviewer labels, and calibration runs
that record aggregate error, confidence, threshold policy, per-source quality
gates, joined-evidence hashes, reason codes, and a hash-only report digest.
Prediction writes must name a registered active or candidate model, match its
policy and feature schema, and reference an existing feature vector hash for the
same source. Admins can inspect calibration reports, persisted calibration runs,
active-model risk reports, ranking-credit readiness reports, and operational
summary blocker counts before deciding whether model-derived credit can settle.
Settlement excludes `ranking_utility` credit events unless the request names an
active model version with a fresh promotable calibration run for the same policy,
target use, and registered calibration dataset, and every credit event is bound
to a matching `ranking_prediction:<uuid>` reference with a settlement score that
matches the stored prediction. Prediction-credit workers also require the active
model/target pair to have no uncleared model-risk report codes by default so new
credits wait for calibration/drift review before settlement. Readiness reports
and settlement re-check the same active-model risk codes, so manually appended
prediction-bound ranking credits cannot settle while current evidence is still
at risk. Settlement responses report aggregate ranking-credit exclusion reason
counts for dry-runs and live runs. A scoped promotion
run lets utility workers promote calibrated candidate models through the same
server-owned gate without generic admin access, and a scoped calibration run
lets utility workers schedule bounded calibration passes across active or
candidate models. Calibration, prediction-credit, promotion, and full
credit-cycle automation runs now persist a hash-only worker-run ledger with
running/completed/failed lifecycle status, limits, counts, result refs, skipped
reason aggregates, and safe hashed fatal-error refs for admin review. Live
non-dry-run ranking schedulers reject overlapping active non-stale runs before
appending a new running row; stale running rows surface as operational-summary
blockers until an admin append-finalizes them through the stale recovery API,
which also writes a hash-only audit event for the recovery action.
With the DB mirror configured, ranking evidence, calibration runs, and ranking
worker runs are dual-written to PostgreSQL. Maintenance backfill mirrors
file-backed ranking model versions, feature/prediction/label evidence,
calibration runs, and worker-run rows into the DB. Maintenance reconciliation
compares ranking model versions, feature/prediction/label evidence,
calibration report hashes, and worker-run lifecycle rows across file and DB
storage, feeding any missing or drifted rows into `blocking_gaps` before DB
reviewer reads or credit-bearing ranking paths are promoted.
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves admin ranking lists, calibration
reports, model-risk reports, credit-readiness reports, calibration-run history,
and worker-run history from the tenant-scoped DB mirror.

## Binaries

- `trace-commons-ingest`: hosted ingest/review/admin/worker API.
- `trace-commons-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/trace-commons-protocol`: shared TraceCommons protocol DTOs and redaction helpers.
- `crates/trace-commons-server`: Rust server binaries.
- `migrations`: TraceCommons server database schema, renumbered as this repo's
  first migration.
- `docs`: copied Trace Commons design, storage, and roadmap docs that now belong
  with the hosted server/control plane.

## Local Development

From this repository:

```bash
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo now builds without an Ironclaw path dependency. Ironclaw should depend
on the shared `trace-commons-protocol` crate when the client-side integration is
rewired.
