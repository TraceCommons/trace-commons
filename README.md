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
labels, calibration reports, and PostgreSQL-backed ranking evidence reads/writes
behind the same DB mirror cutover gates.

## Trace Credits

Trace Credits are non-transferable account credits backed by reviewed utility
evidence. Uploads and ranker scores do not settle credit directly. Utility
workers record hash-only attestations for accepted traces, admins run settlement
batches, and optional NEAR receipt calls are queued only after off-chain
settlement finalizes.

The NEAR path is intentionally an outbox of deterministic method-call payloads
for a non-transferable receipt contract. Workers mark outbox items submitted,
confirmed, or failed after contract submission. The server ledger remains
authoritative; NEAR payloads contain batch ids, account hashes, source-list
hashes, policy versions, amounts, and issuer-signature hashes, never trace
bodies or raw contributor identity.

Ranking evidence is stored separately from settlement. Workers can register
feature hashes, model predictions, and lab/reviewer labels; admins can inspect
the calibration report before deciding which predictions are trustworthy enough
to influence settlement policy. With the DB mirror configured, ranking evidence
is dual-written to PostgreSQL and `TRACE_COMMONS_DB_REVIEWER_READS=true` serves
admin ranking lists and calibration reports from the tenant-scoped DB mirror.

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
