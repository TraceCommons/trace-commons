# Trace Commons Upload-Claim Issuer

## Purpose

`trace-commons-upload-claim-issuer` is the standalone Ed25519 (EdDSA) issuer that
mints short-lived upload claims for the Trace Commons ingest endpoint. Tenant
workloads present a workload JWT (signed by an external workload-identity key)
and receive a `Bearer` upload claim bound to a specific tenant, principal,
consent-scope allowlist, and allowed-use allowlist. The hosted ingest service
verifies these claims against the issuer's published keyset before accepting
any envelope. The published keyset is served by the issuer itself at
`/.well-known/trace-commons-ed25519-keyset.json`; consumers cache it through
their existing guarded-refresh path. See `docs/trace-commons.md` for the
envelope contract and `docs/trace-commons-storage.md` for the storage contract.

## Environment variables

| Variable | Description | Required | Default |
| --- | --- | --- | --- |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND` | Listen address (`host:port`) | no | `127.0.0.1:3917` |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_PEM` | EdDSA PKCS#8 private key (inline) | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_FILE` | Path to EdDSA PKCS#8 private key PEM | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_PEM` | EdDSA SPKI public key (inline) | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_FILE` | Path to EdDSA SPKI public key PEM | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID` | Key id published on the keyset and in the `kid` header of issued JWTs | yes | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER` | `iss` claim on issued JWTs | yes | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE` | `aud` claim on issued JWTs | yes | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_TTL_SECONDS` | Upload-claim TTL ceiling | no | `300` |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_PEM` | EdDSA SPKI public key for the workload identity issuer (inline) | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_FILE` | Path to workload public key PEM | one of `_PEM` / `_FILE` | — |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_ISSUER` | Required `iss` on the inbound workload JWT | no | unchecked |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_AUDIENCE` | Required `aud` on the inbound workload JWT | no | unchecked |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUIRE_TENANT_ACCESS_GRANTS` | When truthy (`1`/`true`/`yes`/`on`), require an active contributor grant in the tenant-access-grant store | no | `false` |
| `TRACE_COMMONS_ONBOARDING_DEVICE_KEY_REGISTRY_ENABLED` | When truthy, enable `POST /v1/onboard` device-key registration against PostgreSQL | no | `false` |
| `TRACE_COMMONS_ONBOARDING_INGEST_URL` | Ingest URL returned by successful onboarding responses | yes when onboarding registry is enabled | — |
| `TRACE_COMMONS_ONBOARDING_COMMUNITY_URL` | Optional community URL returned by onboarding | no | — |
| `TRACE_COMMONS_ONBOARDING_PROFILE_URL` | Optional contributor profile URL returned by onboarding | no | — |
| `TRACE_COMMONS_ONBOARDING_LEADERBOARD_URL` | Optional leaderboard URL returned by onboarding | no | — |
| `DATABASE_URL` | PostgreSQL URL for the tenant-access-grant store and onboarding device-key registry | yes when grants or onboarding registry are required | — |
| `DATABASE_POOL_SIZE` | Pool size for the grant/registry store | no | `5` |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SHUTDOWN_GRACE_SECONDS` | Window allowed for in-flight requests to drain after SIGTERM / Ctrl-C | no | `30` |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUEST_TIMEOUT_SECONDS` | Per-request timeout | no | `10` |
| `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_REQUEST_BYTES` | Body-size limit on `POST /v1/trace-upload-claim` | no | `65536` |
| `RUST_LOG` | `tracing-subscriber` filter | no | `trace_commons_upload_claim_issuer=info,trace_commons_server=info` |

Inline `_PEM` and file `_FILE` variants are mutually exclusive for each key.
The issuer fails closed at startup when required material is missing or
malformed and never falls back to a less-restricted backend.

## Routes

| Method | Path | Description |
| --- | --- | --- |
| `GET` | `/health` | Returns `200 {"status":"ok","checks":{...}}` when the signing key signs cleanly and the workload public key parses; `503 {"status":"degraded","checks":{...}}` otherwise. Check names are stable labels; failure detail is hash-only. |
| `GET` | `/.well-known/trace-commons-ed25519-keyset.json` | Returns the issuer's public keyset (`kid`, `public_key_pem`). Consumers cache this through their guarded-refresh path. |
| `POST` | `/v1/trace-upload-claim` | Mints a Bearer upload claim for either an authenticated workload JWT or a registered device key. Registered devices send `x-trace-device-key-id: sha256:<64-hex>` and `x-trace-device-signature: <base64-ed25519-signature>` over the exact JSON request body. Body schema is `ironclaw.trace_upload_claim_request.v1`. |
| `POST` | `/v1/onboard` | Exchanges an invite code plus base64 Ed25519 device public key for tenant-scoped onboarding metadata and a registered device key. Same invite plus same device key is idempotent; exhausted retry budgets return `InviteAlreadyConsumed`. Body schema is `trace_commons.onboard_request.v1`; response schema is `trace_commons.onboard_response.v1`. |

## CLI subcommands

The binary accepts an optional subcommand. With no subcommand the HTTP issuer
starts.

| Subcommand | Behaviour |
| --- | --- |
| `--help` / `-h` | Print the usage summary. |
| `--generate-keypair` | Generate a fresh Ed25519 keypair and print the PKCS#8 private key PEM, SPKI public key PEM, and a suggested `kid` (UUID v4) to stdout. Output is not written to disk; the operator pipes it where they want. Exit 0 on success. |
| `--health-check` | Load env vars, attempt to build state and exercise the signing and workload keys, then print `OK` (exit 0) or `FAIL: <reason>` (exit 1). The reason is a hash-free stable label. Does not bind a listener. |
| `--mint-test-claim` | Mint a test upload claim for a hardcoded test tenant (`trace-upload-claim-issuer-test-tenant`) and principal (`principal:trace-upload-claim-issuer-test`) and print the JWT to stdout. For testing / deploy probes only — must not be exposed in production traffic paths. |
| `--hash-invite-code <CODE>` | Print the canonical `sha256:` invite subject hash for a raw pilot invite code. Prefer `--mint-invites` for normal batch operations. |
| `--import-file-invites <PATH> --policy-label <LABEL>` | One-time migration of an existing pilot allowlist JSON file's invite entries into the database. Idempotent on the invite hash. Instance entries stay in the file and are counted, not imported. Prints counts only. |
| `--mint-invites <COUNT> --mint-tenant-template <TEMPLATE> --policy-label <LABEL> [--mint-max-uses <N>] [--mint-expires-in-days <N>] [--mint-note-label <LABEL>] [--mint-consent-scopes <a,b,c>] [--mint-allowed-uses <a,b,c>]` | Mint `COUNT` server-side invites directly against the database, replacing the retired `scripts/operator/generate-pilot-invites.py`. Prints one raw 16-character code per line to stdout and nothing else. |

## Key rotation procedure

The MVP is single-key. Rotation today requires a brief restart window;
multi-key (no-downtime) rotation is a future enhancement.

1. **Generate a new keypair.**

   ```bash
   trace-commons-upload-claim-issuer --generate-keypair > new-key.pem
   ```

   The output contains a PKCS#8 private key, an SPKI public key, and a
   suggested `kid`. Store the private key in your secret manager; the public
   key and `kid` will be served by the issuer at
   `/.well-known/trace-commons-ed25519-keyset.json` once the deployment
   restarts with the new material.

2. **Pre-stage the new public key.** If your consumers cache the keyset
   aggressively, publish the new public key alongside the old one through your
   normal keyset-distribution channel before restart, so verifiers accept both
   the outgoing and incoming `kid` during the window where in-flight claims
   may have been signed by either key. The current single-key issuer cannot
   serve two keys at the same time on its own `.well-known` — operators with
   strict cache windows should mirror the public key through a CDN-fronted
   path.

3. **Update the issuer environment.** Replace
   `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_PEM` /
   `..._FILE`, the matching public-key var, and
   `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID` with the new material.

4. **Restart the issuer.** New claims are signed by the new key. Claims
   already issued by the previous key remain valid until their `exp`
   (default ceiling: `max_ttl_seconds`, 300 s by default).

5. **Optional: keep the old public key available for at least
   `max_claim_ttl + cache_refresh_window`** so any in-flight claims signed
   by the previous `kid` continue to verify on the consumer side. Retire the
   old public key from your distribution channel after that window.

Multi-key rotation (the issuer serving multiple active `kid`s at once and
selecting one for signing) is intentionally out of scope for this slice.
Until it lands, plan rotations during a known restart window.

## Deploy story

Build and run locally:

```bash
cargo build --release --bin trace-commons-upload-claim-issuer
./target/release/trace-commons-upload-claim-issuer
```

There is no committed `Dockerfile` for this binary. A minimal `docker run`
shape that supplies the required env vars from your secret manager:

```bash
docker run --rm \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_BIND=0.0.0.0:3917 \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PRIVATE_KEY_FILE=/etc/issuer/signing-private.pem \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_PUBLIC_KEY_FILE=/etc/issuer/signing-public.pem \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID="$KID" \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_ISSUER=trace-commons-upload-issuer \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_AUDIENCE=trace-commons-upload \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_PUBLIC_KEY_FILE=/etc/issuer/workload-public.pem \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_ISSUER=workload-issuer \
  -e TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_WORKLOAD_AUDIENCE=trace-claim-issuer \
  -v /path/to/keys:/etc/issuer:ro \
  -p 3917:3917 \
  trace-commons-upload-claim-issuer
```

Example systemd unit (excerpt):

```ini
[Service]
EnvironmentFile=/etc/trace-commons/upload-claim-issuer.env
ExecStart=/usr/local/bin/trace-commons-upload-claim-issuer
ExecStartPre=/usr/local/bin/trace-commons-upload-claim-issuer --health-check
Restart=on-failure
KillSignal=SIGTERM
TimeoutStopSec=60
```

`ExecStartPre` lets the unit fail fast if the configured material is unusable
before flipping the listener live.

## Operational checks

- `curl -fsS http://127.0.0.1:3917/health` — expect `200 {"status":"ok",...}`.
- `curl -fsS http://127.0.0.1:3917/.well-known/trace-commons-ed25519-keyset.json`
  — expect `200` with a `keys` array containing the active `kid` and SPKI PEM.
  Body never includes private material, `kty`, `crv`, or `x` JWK fields, or any
  `RSA` marker.
- `trace-commons-upload-claim-issuer --health-check` — exits `0` on `OK`; `1` with
  a hash-free reason on failure (`config-missing`, `config-invalid`,
  `signing-self-test-failed`, `workload-public-key-parse-failed`,
  `tenant-grant-db-unavailable`).
- `trace-commons-upload-claim-issuer --mint-test-claim` — mints a test JWT against
  the same env config the live service would use. Useful as a deploy smoke
  check; the resulting token is bound to a fixed test tenant and principal and
  should not be accepted by production ingest paths.

## Graceful shutdown

On `SIGTERM` or `SIGINT` (Ctrl-C) the issuer stops accepting new connections
and waits up to
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SHUTDOWN_GRACE_SECONDS` (default `30`) for
in-flight requests to complete. After the grace window expires any remaining
in-flight connections are dropped. The shutdown signal is logged at `info`
with the structured field `graceful_shutdown_secs`; if the grace window is
exceeded a `warn` is emitted. No secret material is included in either log.

Each request is bounded by
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_REQUEST_TIMEOUT_SECONDS` (default `10`)
and the body of `POST /v1/trace-upload-claim` is capped at
`TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_MAX_REQUEST_BYTES` (default `65536`).
