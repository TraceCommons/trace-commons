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
