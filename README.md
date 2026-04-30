# TraceCommons Server

Hosted server-side control plane for Trace Commons / TraceCommons.

This repository is the extraction point for the pieces that should not live
inside Ironclaw long term: public/private ingest, review queues, tenant access
grants, audit/retention/revocation workers, export jobs, upload-claim issuing,
object storage, and the production database schema.

## Current Extraction Status

This is a bridge extraction from Ironclaw's `gecko-pass` worktree. The server
binaries are present here and temporarily depend on the local Ironclaw crate for
shared protocol, storage, database, and artifact-store types. That keeps the
first split honest and buildable locally while the next pass moves shared
TraceCommons protocol types into a small independent crate.

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
```

The bridge dependency points at the adjacent Ironclaw checkout. Once protocol
types are extracted, this repo should build without an Ironclaw path dependency.

