# TraceDAO Server

Hosted server-side control plane for Trace Commons / TraceDAO.

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
credit receipt outbox. The next ranking substrate is also server-owned: model
version records, hash-only feature records, prediction records, frontier/reviewer
labels, calibration reports, persisted model-promotion calibration runs, and
PostgreSQL-backed ranking evidence reads/writes behind the same DB mirror
cutover gates.

## Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run settlement
batches, and optional NEAR receipt calls are queued only after off-chain
settlement finalizes. Settlement retries repair missing NEAR outbox rows from
finalized batches, and reviewer/admin credit summaries report tenant-wide
settled line items while contributor summaries stay principal-scoped. With the
DB mirror configured, utility attestations, settlement batches, credit holds,
and NEAR receipt outbox rows are dual-written to PostgreSQL;
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves the admin credit control-plane
lists from the tenant-scoped DB mirror.

The NEAR path is intentionally an outbox of deterministic method-call payloads
for a non-transferable receipt contract. Configure
`TRACE_COMMONS_NEAR_CREDIT_SUBMITTER_URL` to let the scoped submit worker hand
pending or failed-retry calls to an operator-owned NEAR relayer; the server then
records the public transaction hash or a hashed failure. Workers can still
manually mark items submitted, confirmed, or failed for fallback operations. The
server ledger remains authoritative; NEAR payloads contain batch ids, account
hashes, source-list hashes, policy versions, amounts, and issuer-signature
hashes, never trace bodies or raw contributor identity.

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
credits wait for calibration/drift review before settlement. A scoped promotion
run lets utility workers promote calibrated candidate models through the same
server-owned gate without generic admin access. Prediction-credit and promotion
automation runs now persist a hash-only worker-run ledger with running/completed
lifecycle status, limits, counts, result refs, and skipped reason aggregates for
admin review. With the DB mirror
configured, ranking evidence, calibration runs, and ranking worker runs are
dual-written to PostgreSQL and
`TRACE_COMMONS_DB_REVIEWER_READS=true` serves admin ranking lists, calibration
reports, model-risk reports, credit-readiness reports, calibration-run history,
and worker-run history from the tenant-scoped DB mirror.

## Binaries

- `tracedao-ingest`: hosted ingest/review/admin/worker API.
- `tracedao-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/tracedao-protocol`: shared TraceDAO protocol DTOs and redaction helpers.
- `crates/tracedao-server`: Rust server binaries.
- `migrations`: TraceDAO server database schema, renumbered as this repo's
  first migration.
- `docs`: copied Trace Commons design, storage, and roadmap docs that now belong
  with the hosted server/control plane.

## Local Development

From this repository:

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

This repo now builds without an Ironclaw path dependency. Ironclaw should depend
on the shared `tracedao-protocol` crate when the client-side integration is
rewired.
