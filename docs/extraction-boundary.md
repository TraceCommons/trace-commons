# TraceDAO Extraction Boundary

## Keep In Ironclaw

- Local trace capture and recorded-trace conversion.
- Contributor-side redaction and privacy filter preflight.
- Signed contribution envelope creation.
- Local queueing, retry telemetry, and safe queue diagnostics.
- Upload client, local revoke/status sync, and credit notice UX.
- CLI commands that operate on local contribution state or call a remote
  TraceDAO endpoint.

## Move To TraceDAO Server

- Hosted ingest, review, admin, worker, and issuer HTTP surfaces.
- Tenant registry, tenant policies, and tenant access grants.
- Durable submission metadata, object refs, derived rows, vector rows, audit
  rows, credit ledger rows, tombstones, retention jobs, export manifests, export
  jobs, and revocation propagation.
- Service-owned object storage and object-primary read/write modes.
- PostgreSQL RLS readiness and tenant-bound server enforcement.
- Reviewer/admin APIs, maintenance workers, and export/benchmark/ranker worker
  routes.

## Remaining Temporary Bridge

The first repo split intentionally keeps `tracedao-server` dependent on the
local Ironclaw crate for the shared contribution envelope/protocol compatibility
surface. Database storage, object refs, RLS diagnostics, and encrypted
artifact-store code are now server-owned in this repo.

The next extraction pass should:

1. Move contribution envelope, auth claim, status, and policy DTOs into a shared
   `tracedao-protocol` crate.
2. Leave Ironclaw depending only on the shared protocol crate plus remote client
   helpers.
3. Remove the Ironclaw path dependency from `crates/tracedao-server/Cargo.toml`.
