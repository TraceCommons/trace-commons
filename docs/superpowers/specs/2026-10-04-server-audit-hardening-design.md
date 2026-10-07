# Server audit hardening design

## Goal

Close the three confirmed `main` audit findings without changing tenant,
authorization, or licensing boundaries:

1. Pilot access logs must not retain one-time login codes, full client
   addresses (kept to /24 or /48), or raw tenant identifiers (hashed), and slow
   or oversized request bodies must be bounded at the public edge.
2. Large ingest and token-bundle bodies must not be extracted before bearer
   authentication, and one token must not be able to make ingest buffer
   unbounded bodies: the pre-body gate holds a per-principal and a
   deployment-wide in-flight slot. Token-bundle `begin` and `put` take the
   attachment cap as their body limit. All other API bodies keep a small
   explicit ceiling.
3. Public/account rate-limit keying must canonicalize trusted proxy addresses,
   coarsen IPv6 clients to `/64`, bound key cardinality, avoid a full-map scan
   on every request, and give the login interstitial a global ceiling. Keys
   derived from an authenticated principal are bounded in their own table, so
   an anonymous key flood cannot fold them into the anonymous overflow bucket.

## Constraints

- `/v1/source` remains unauthenticated and outside fail-closed gates.
- Existing handler authorization and tenant-access-grant checks remain in
  place; the new upload preflight is an additional early rejection boundary.
- The pilot continues to trust `X-Forwarded-For` only because Caddy overwrites
  it and the ingest service binds loopback. Invalid or absent values share one
  conservative bucket.
- No new dependency is introduced.
- New Rust code in `trace-commons-server` retains the AGPL header policy.

## Verification contract

- Regression tests prove unauthenticated upload requests are denied without
  their body being read, each upload route admits exactly its ceiling and
  refuses one byte more, a principal past its in-flight cap is refused before
  its body is read, client keys are canonical and bounded, principal keys
  survive an anonymous key flood, stale key pruning is cadence-limited, and
  the interstitial global counter is wired.
- The rendered Caddy template is adapted by Caddy and its adapted JSON proves
  the `code` query parameter is replaced, tenant query values are hashed,
  client addresses are masked, and a request-body read timeout and size
  ceiling are configured.
- Focused ingest tests, warning-denied checks, formatting, and the license
  boundary test pass.
