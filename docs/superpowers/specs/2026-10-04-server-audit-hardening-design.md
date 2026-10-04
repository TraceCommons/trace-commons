# Server audit hardening design

## Goal

Close the three confirmed `main` audit findings without changing tenant,
authorization, or licensing boundaries:

1. Pilot access logs must not retain one-time login codes, and slow request
   bodies must be bounded at the public edge.
2. Large ingest and token-bundle bodies must not be extracted before bearer
   authentication. All other API bodies keep a small explicit ceiling.
3. Public/account rate-limit keying must canonicalize trusted proxy addresses,
   coarsen IPv6 clients to `/64`, bound key cardinality, avoid a full-map scan
   on every request, and give the login interstitial a global ceiling.

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

- Regression tests prove malformed unauthenticated upload requests are denied
  before extractor errors, client keys are canonical and bounded, stale key
  pruning is cadence-limited, and the interstitial global counter is wired.
- The rendered Caddy template is adapted by Caddy and its adapted JSON proves
  the `code` query parameter is replaced and a request-body read timeout is
  configured.
- Focused ingest tests, warning-denied checks, formatting, and the license
  boundary test pass.
