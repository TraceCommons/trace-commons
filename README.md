# TraceCommons Server

Hosted server-side control plane for Trace Commons / TraceCommons.

This repository is the extraction point for the pieces that should not live
inside Ironclaw long term: public/private ingest, review queues, tenant access
grants, audit/retention/revocation workers, export jobs, upload-claim issuing,
object storage, and the production database schema.

## Current Extraction Status

This is a bridge extraction from Ironclaw's `gecko-pass` worktree. The hosted
server now owns its database and object-storage surface locally: the
`TraceCorpusStore` trait, PostgreSQL/libSQL backends, server migrations,
RLS diagnostics, and encrypted artifact-store provider code live in this repo.

The remaining temporary Ironclaw path dependency is only for the shared
contribution envelope/protocol compatibility surface while those DTOs move into
a small independent TraceCommons protocol crate.

## Binaries

- `trace-commons-ingest`: hosted ingest/review/admin/worker API.
- `trace-commons-upload-claim-issuer`: EdDSA/Ed25519 upload-claim issuer for hosted
  contributors.

## Repository Layout

- `crates/trace-commons-server`: Rust server binaries.
- `migrations`: TraceCommons server database schema, renumbered as this repo's
  first migration.
- `docs`: copied Trace Commons design, storage, and roadmap docs that now belong
  with the hosted server/control plane.

## Local Development

From this repository:

```bash
cargo check -p trace-commons-server --bins
cargo test -p trace-commons-server --test trace_corpus_storage_contract --test trace_corpus_db_store
```

The bridge dependency points at the adjacent Ironclaw checkout for protocol
compatibility only. Once protocol types are extracted, this repo should build
without an Ironclaw path dependency.
