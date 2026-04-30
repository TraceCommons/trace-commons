# TraceCommons Extraction Boundary

## Keep In Ironclaw

- Local trace capture and recorded-trace conversion.
- Contributor-side redaction and privacy filter preflight.
- Signed contribution envelope creation.
- Local queueing, retry telemetry, and safe queue diagnostics.
- Upload client, local revoke/status sync, and credit notice UX.
- CLI commands that operate on local contribution state or call a remote
  TraceCommons endpoint.

## Move To TraceCommons Server

- Hosted ingest, review, admin, worker, and issuer HTTP surfaces.
- Tenant registry, tenant policies, and tenant access grants.
- Durable submission metadata, object refs, derived rows, vector rows, audit
  rows, credit ledger rows, tombstones, retention jobs, export manifests, export
  jobs, and revocation propagation.
- Service-owned object storage and object-primary read/write modes.
- PostgreSQL RLS readiness and tenant-bound server enforcement.
- Reviewer/admin APIs, maintenance workers, and export/benchmark/ranker worker
  routes.

## Temporary Bridge

The first repo split intentionally keeps `trace-commons-server` dependent on the
local Ironclaw crate. This avoids rewriting the protocol and storage layers
while the server ownership boundary is created. The next extraction pass should:

1. Move contribution envelope, auth claim, status, and policy DTOs into a shared
   `trace-commons-protocol` crate.
2. Move server storage traits and database implementations into this repo.
3. Leave Ironclaw depending only on the shared protocol crate plus remote client
   helpers.
4. Remove the Ironclaw path dependency from `crates/trace-commons-server/Cargo.toml`.

