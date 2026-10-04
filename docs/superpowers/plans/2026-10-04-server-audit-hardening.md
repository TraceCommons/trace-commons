# Server audit hardening implementation plan

> **Spec:** `docs/superpowers/specs/2026-10-04-server-audit-hardening-design.md`
> **Execution:** native inline

## Global constraints

- Preserve the dirty primary checkout; work only on the isolated branch.
- Follow strict RED-GREEN TDD for Rust behavior.
- Keep auth, tenant grants, `/v1/source`, and split-license boundaries intact.

## Task 1: Bound and canonicalize rate limiting

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Steps:**
1. Add tests for rightmost trusted-hop parsing, IPv6 `/64`, invalid fallback,
   bounded distinct-key tracking, and interstitial global limiting.
2. Run the focused tests and observe the old behavior fail.
3. Reuse `invite_lookup::lookup_client_key`; replace the raw map with a bounded
   table whose stale-entry scan runs at most once per second and whose excess
   keys share a fail-closed overflow bucket.
4. Add a global interstitial counter and run the focused tests green.

**Completion:** focused limiter and interstitial tests pass.

## Task 2: Authenticate large uploads before extraction

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
- Modify: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Steps:**
1. Add router tests showing malformed unauthenticated large-upload requests
   currently reach extractor rejection and that ordinary routes reject bodies
   above the small API ceiling.
2. Run them RED.
3. Add a bearer-auth preflight middleware to the trace and token-bundle upload
   method routers, retain handler auth/grant checks, give only those methods the
   ingest ceiling, and make the router-wide ceiling 2 MiB.
4. Run the focused router tests GREEN, plus existing malformed-body tests.

**Completion:** auth rejection precedes extraction and body caps remain
route-appropriate.

## Task 3: Harden the pilot edge configuration

**Files:**
- Modify: `deploy/pilot-gcp/Caddyfile.template`
- Modify: `docs/operator/pilot-gcp-deployment.md`

**Steps:**
1. Render and adapt the current template, then query the adapted JSON for the
   absent `code` redaction and read-body timeout (RED).
2. Configure a server read-body timeout and Caddy log filtering that replaces
   the `code` query value on both public hosts.
3. Document the load-bearing edge controls.
4. Re-render and adapt with Caddy; assert the adapted JSON contains both
   controls (GREEN).

**Completion:** `caddy adapt` succeeds and the adapted configuration proves
the controls.

## Task 4: Whole-branch verification

1. Run formatting and warning-denied focused checks/tests.
2. Run the complete ingest binary test suite and license-boundary suite.
3. Review the full diff against this plan and spec; fix Important/Critical
   findings once under RED-GREEN.

## Review focus

- Axum layer ordering really rejects authentication before body extraction.
- Proxy trust comments match the deployed overwrite behavior and do not imply
  direct public access is safe.
- Overflow behavior is fail-closed and strictly bounded.
- Caddy redaction changes log output, not just template text.
