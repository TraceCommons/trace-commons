# AGPL section 13 source offer — `GET /v1/source`

## Why this endpoint exists

`trace-commons-server`, `trace-commons-gate-api`, and
`trace-commons-gate-enclave` are licensed AGPL-3.0-or-later. Section 13 of that
license says that if you run a **modified** version and let others interact with
it over a network, you must offer those users the Corresponding Source.

Pilot contributors are exactly those users: they reach the ingest API from the
contributor CLI or a desktop app and never see the repository. A notice in a
README they will not read does not discharge the obligation. So the offer is on
the wire.

## What it does

```
GET /v1/source
```

Unauthenticated. No tenant context. Outside every fail-closed gate. Returns:

```json
{
  "license": "AGPL-3.0-or-later",
  "source_url": "https://github.com/zmanian/trace-commons-server",
  "build_commit": "93f06c00",
  "build_time": "2026-08-28T00:00:00Z",
  "build_version": "0.6.0"
}
```

It reveals nothing a published release does not already reveal, which is why it
does not conflict with the hash-only logging convention.

`build_commit` is the point of it. A user exercising section 13 needs the source
of the *version they are talking to*, and `build_version` does not move when a
deploy does — the same reason `/health` reports the commit.

## Operator obligations

**Do not put it behind authentication, an allowlist, or a WAF rule.** A
credential requirement defeats the section it exists to satisfy. The
pilot-GCP deploy script curls it through the public ingress with `--fail` after
every deploy, so a proxy rule that swallows it breaks the deploy rather than
going unnoticed.

**If you deploy a modified build, you must point it at your own source.**
`TRACE_COMMONS_SOURCE_URL` is a constant in
`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`, not an
environment variable — deliberately, because section 13 obliges the operator of
a *modified* version, and anyone modifying the binary is already editing it. A
knob would only let an unmodified deploy point somewhere wrong. Change the
constant in your fork and make sure the source it names is actually reachable.

**Rate limiting is fine; blocking is not.** The endpoint is cheap and static,
but it is a public unauthenticated route like any other.

## Verifying

```bash
curl -sfS https://ingest.<host>/v1/source | jq .

# The commit must match what /health reports; a mismatch means one of them is
# stale and the source offer is pointing at the wrong version.
diff <(curl -sfS https://ingest.<host>/health   | jq -r .build_commit) \
     <(curl -sfS https://ingest.<host>/v1/source | jq -r .build_commit)
```

Note the host checkout's git state is not evidence of what is deployed; see
`deployment.md`. `build_commit` from this endpoint is.

## Related

- `LICENSE` at the repo root — the full split, crate by crate.
- `crates/trace-commons-server/tests/license_boundary.rs` — enforces that no
  permissive client crate links an AGPL crate in a shipped build.
- `deny.toml` — dependency-license audit (`cargo deny check licenses`).
