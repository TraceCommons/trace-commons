# Automatic witness endpoints implementation plan

Goal: Authorize automatic endpoint and measurement changes through signed configuration, with one initial client release.

Spec: ../specs/2026-11-05-automatic-witness-endpoints-design.md

- [x] Protocol and operator CLI: add strictly bounded schema, endpoint-owned pins, alias enrollment matching, signature verification/canonical payload and KMS tooling. Tests cover tampering, schema/field validation, URL and collection bounds, endpoint membership, withdrawal and V1 compatibility.
- [x] Ingest: serve an optional local configuration file on the fixed public route, unchanged bytes and bounded reads. Test missing/corrupt/unsigned/oversized and atomic replacement plus legacy coexistence.
- [x] Contributor: verified monotonic cache and refresh; independently track configuration versions in supervisor; preserve stable enrollment identity/grants while selecting runtime endpoints. Apply explicit host policy and verify candidates before raw bytes. Route both ordinary and token-bundle paths. Test expiry, withdrawal, paired pins, unchanged settings, pre-send failover, no retry after raw send, old-server compatibility and scheduling.
- [x] Run final combined targeted tests, license boundary, manifest and format checks, scoped clippy with consistent Rust flags. Independent security review after tests; fix material findings and rerun affected checks.
- [x] Build operator tool, sign and verify an initial configuration retaining only healthy active endpoints; stage without enabling unsupported code. Update PR title/body and runbooks around final behavior. Keep custom-domain gateway blocker distinct from code/release state.
