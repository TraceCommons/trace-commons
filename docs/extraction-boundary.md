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

## Shared Protocol Boundary

The server repo no longer depends on the local Ironclaw crate. Shared
contribution envelope/protocol compatibility now lives in the local
`crates/trace-commons-protocol` crate alongside the hosted server.

The next Ironclaw-side extraction pass should:

1. Point Ironclaw client-side trace submission code at `trace-commons-protocol` for
   contribution envelope, status, consent, policy, and redaction-compatible DTOs.
2. Keep local capture, queueing, upload client, revoke/status sync, and credit
   notice UX in Ironclaw.
3. Move any remaining remote-hosted route/client contract types into
   `trace-commons-protocol` only when both repos need them.
