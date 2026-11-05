# Witness custom domain implementation plan

Goal: Establish and validate witness.tracecommons.ai without disrupting the existing URL.

Spec: ../specs/2026-11-05-witness-custom-domain-design.md

- [x] Add digest-pinned in-CVM ingress, preserving the deployed witness image and old listener; regenerate and check app-compose.json.
- [x] Verify image digest and YAML syntax; manifest and license boundary tests pass. Record authoritative deployed compose hash and publish new measurement to legacy signed list and ingest allowlists. `--prepare-only` applied immediately on managed KMS; this gotcha is recorded.
- [ ] Deploy with sealed env retained and logs/sysinfo private. Create DNS-only CNAME and current-instance TXT; add the printed ACME-account CAA. Check real certificate, evidence and nonce-bound attestation, then run synthetic submit/poll on both URLs.
- [x] Record deployment, rollback, client migration limitations and future host-cutover procedure; obtain independent review.
- [ ] Resolve Phala custom-SNI routing failure before enabling discovery. Initial custom-domain synthetic succeeded, but repeated TLS checks now fail before reaching the healthy ingress. Keep legacy discovery and list URL unchanged.

Review focus: old-client trust/list availability; retained sealed API key; no external TLS termination; authoritative measurement; DNS instance id on each redeploy; forward-only list versions during rollback.
