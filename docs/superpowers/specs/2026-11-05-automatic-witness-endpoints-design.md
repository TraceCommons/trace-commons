# Signed automatic witness endpoint updates

The owner approved publishing new domains with measurements and switching automatically. One client release introduces this capability; later compatible domain and CVM moves use signed configuration rather than app updates.

## Trust and consent boundary

Keep the configured witness URL as a stable enrollment identity. A signed configuration applies only when that exact enrollment URL appears among its enrollment aliases (trailing slashes do not distinguish identities). Each ordered active endpoint carries its own signing-address/measurement pairs and validity windows. A server response, DNS record or HTTP redirect alone never authorizes a new endpoint.

The existing compiled witness-list public keys also verify this new, schema-separated document. Unsigned, tampered, unsupported, unrelated or older documents cannot replace the verified cache. The cache is verified on every read and has an independent strictly increasing version counter. Serialize compare-version plus atomic write across concurrent processes, and fail closed on an existing invalid cache. Recheck configuration and current candidate validity after probing, immediately before raw submission. Validity-window activation must release relevant holds even without a version change. All entries must parse before installation. Each active candidate must be valid when checked. An empty or expired active set in a matching configuration is an authoritative withdrawal and fails closed; it does not fall back to signup pins.

Signed routing updates preserve existing scopes, uses, approvals and grants, and do not mutate persisted WitnessSettings. This owner decision supersedes the previous confirmation requirement only for runtime routing authenticated by the commons signing key. Manual changes to configured witness identity continue to invalidate grants. Original signup pins must exist; signed configuration never enables an unpinned client. An explicit nonempty/enforcing administrator host allowlist remains authoritative; empty CSV retains its established unset-equivalent meaning. A derived witness-transport allowlist may include the selected signed host, without widening ingestion, receipt or other HTTP destinations.

## Protocol and compatibility

Add `GET /v1/public/witness-configuration`, a global anonymous fixed-origin route. Its optional configured local file is read per request, bounded to 64 KiB, checked for signed shape and served byte for byte. It performs no remote fetch and logs no paths or URLs. Existing V1 measurement-list routes and signing commands remain unchanged for older clients.

The new schema is `trace-commons.witness-configuration.v1`, using the existing signed payload/signatures envelope. Fields: schema, positive monotonically increasing version, issued_at, enrollment_urls, ordered endpoints. Each endpoint has url and existing AcceptedEntry values (measurements, signing_address, not_before/not_after, optional commit). Bounds: 32 enrollment URLs, 8 active endpoints, 32 entries per endpoint, 64 KiB served document. Empty endpoints support withdrawal; enrollment aliases cannot be empty. Active endpoints must be among the enrollment aliases. Duplicate URLs after trailing-slash normalization are invalid. URLs are strict HTTPS, at most 2048 bytes, without userinfo, query, fragment, controls, whitespace or backslashes; original valid text is retained. Unknown fields are refused.

The operator tool gains configuration-payload, configuration-sign and configuration-verify using the same owner-controlled KMS key. The CLI validates actual measurement/address parsers and verifies KMS signatures locally before writing.

## Client behavior

Fetch and refresh alongside the existing V1 list at startup, daily and after measurement refusal. A missing route on an older server keeps legacy behavior. Fetch failure keeps the verified cache. Configuration version changes independently release relevant measurement holds through the supervisor.

For ordinary and restricted token-bundle witnessing, choose a currently valid signed endpoint in order, enforce host policy, obtain a fresh nonce-bound attestation and verify its quote, expected measurement and corresponding signer. Only then transmit raw data. Failover is permitted during the probe stage; once a raw POST starts, never retry it at another endpoint within that uploader/review invocation due to timeout, busy response, job/poll error or ambiguous completion. A separately scheduled later attempt may resolve a newer configuration and must attest afresh; persistent per-session endpoint affinity is not added. Keep size, source-profile, receipt and restricted-token consent checks before their existing boundaries.

## Deployment

Do not publish the currently failing custom hostname as the sole active destination. The initial signed configuration can retain the working Phala URL, with both existing and future stable URLs enrolled. Later, after verified TLS/attestation and synthetic checks, publish a higher version preferring the stable domain and retaining a healthy fallback as needed. Old clients still require the old URL and V1 list until their support period ends. No new UI notice or confirmation flow is introduced.
