# Native NEAR provisioning

This flow creates a Commons identity and registers a device after the wallet
signs a purpose-specific NEP-413 challenge and the daemon signs the same
PKCE-bound ceremony. It does not create a NEAR wallet, fund inference, grant an
invite, enable capture consent, or guarantee submission admission.

It is disabled by default. Readiness requires validated admission configuration,
PostgreSQL mirror writes and tenant RLS readiness, plus these operator settings:

- `TRACE_COMMONS_NEAR_PROVISIONING_ENABLED=true`
- `TRACE_COMMONS_NEAR_PROVISIONING_PUBLIC_ORIGIN`: the HTTPS Commons origin
  hosting `/account/near/provision/wallet`, with no path, query or credentials.
- `TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL`: HTTPS upload-claim issuer base URL.
- `TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE`: the configured upload-claim audience.
- `TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON`: JSON containing `url` (HTTPS),
  `signing_address` (0x plus 40 hex characters), and `expected_measurements`
  (nonempty array of attestation measurement pin strings).

The native daemon additionally requires an explicitly enforcing
`TRACE_COMMONS_ALLOWED_HOSTS` containing Commons, issuer, and witness hosts.
An unset or permissive allowlist refuses this trust-bootstrap flow. It also
validates every published measurement set before persisting witness settings.
The integrated native settings type must retain `admission_evidence=true`.

`GET /v1/account/near/provision/capabilities` returns `ready: false` until the
whole dependency chain is configured. A ready response includes `issuer_url`,
`audience`, `network`, `witness`, and `funding_available: false`. Readiness describes
the Commons identity and admission service, not inference funding availability.
Root ingest wiring must derive `near_provisioning_admission_ready` from the
validated admission configuration; its standalone default is deliberately false.

Native IPC uses `near_account_capabilities {ingest_url}` and
`near_account_start {ingest_url, account_id}`. Start returns `attempt_id`,
`browser_url`, and `status: waiting_for_wallet`. Status/cancel require the same
attempt ID. States are `starting`, `waiting_for_wallet`, `complete`, `failed`,
and `cancelled`. The daemon owns device signing, PKCE, and a random-state loopback
callback. Neither shell nor wallet page receives the device secret or verifier.
Only a verified server finish can publish the local config. An existing or
concurrently created config is never overwritten. Daemon restart loses the local
pending attempt; the durable server ceremony expires after five minutes.

The hosted page uses no third-party scripts. It opens the network-specific
MyNearWallet signing popup, checks both message origin and popup source, and
returns the signed assertion to the exact state-bound loopback callback. Session
tokens never appear in browser history. The transport follows the upstream
[MyNearWallet connector](https://github.com/near/wallet-selector/blob/main/packages/my-near-wallet/src/lib/mnw-connect.ts)
and [wallet signing payload](https://github.com/mynearwallet/my-near-wallet/blob/master/packages/frontend/src/utils/signMessage.ts).
Live wallet-provider interoperability still needs a configured test deployment;
local tests do not impersonate that external service.

V58 stores ceremony commitments and consumes each handle once with atomic
`DELETE RETURNING`. Verified provisioning, stable anchor/account/device mapping,
native session and hash-only audit are one PostgreSQL transaction. Unknown-key
ordinary login stays a uniform rejection. Revoked devices cannot be revived by
signup. Existing invite devices retain a real invite subject; NEAR devices store
NULL and an explicit origin, never a fabricated invite. Ingest must independently
require admission for every reserved `near-<64 lowercase hex>` tenant.

Run the real isolated database regression using a disposable local PostgreSQL:

```sh
TRACE_COMMONS_NEAR_PG_TEST_DATABASE_URL=postgresql://USER@127.0.0.1:PORT/TEST_DB \
  cargo test -p trace-commons-server --test account_onboarding_pg
cargo test -p trace-commons-server --lib account_onboarding
cargo test -p trace-commons-contributor --lib daemon::account_onboarding
cargo test -p trace-commons-server --bin trace-commons-ingest near_provisioning::tests
```

The database test creates a restricted `tc_near_runtime` role in that disposable
cluster. It never falls back to a deployment `DATABASE_URL`.
