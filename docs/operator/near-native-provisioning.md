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
  (nonempty array of measurement pin strings). **This repo has two measurement
  pin spellings and they are not interchangeable.** This field takes the
  `ExpectedMeasurements` form -- `field=<96 hex>`, fields joined by commas:

  ```json
  {
    "url": "https://witness.example",
    "signing_address": "0xabababababababababababababababababababab",
    "expected_measurements": [
      "mrtd=<96 hex>,rtmr0=<96 hex>,rtmr1=<96 hex>,rtmr2=<96 hex>,rtmr3=<96 hex>"
    ]
  }
  ```

  The other spelling, `mrtd:<hex>+mrconfigid:<hex>`, is a `WitnessPin` and
  belongs to `TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS` -- a different
  variable, compared verbatim against a certificate. The two describe the same
  witness at the same URL under the same signing address, which is exactly why
  the wrong one looks right. Using it here leaves the JSON well formed and the
  endpoint not ready; the boot warning names `witness_measurement_syntax` for
  this case, distinct from `witness_json_malformed`.
- `TRACE_COMMONS_NEAR_PROVISIONING_RECEIPT_ENDPOINT` (optional): the provider's
  receipt-service base URL, published to clients so a contributor never has to
  be told it out of band. HTTPS, with no credentials, query or fragment, or it
  is not published at all. A commons serving no attested inference leaves this
  unset and its contributors stay unattested, which is the behaviour they
  already had.

### When the endpoint says not ready

`GET /v1/account/near/provision/capabilities` answers
`{"ready":false,"funding_available":false}` whenever any one of the gates above
declines, and the response deliberately does not say which -- it is served to
unauthenticated clients. The **log** says which: on the first refusal for each
control the service warns once with a `control` field carrying a bare name and
nothing else. The names, in the order they are checked:

| `control` | What to look at |
|---|---|
| `near_provisioning_enabled` | `TRACE_COMMONS_NEAR_PROVISIONING_ENABLED` |
| `near_provisioning_admission_ready` | Admission configuration; provisioning is gated behind it |
| `witness_public_origin` | `..._PUBLIC_ORIGIN` absent, not HTTPS, or carrying a path, query, fragment or credentials |
| `near_sign_in` | The NEAR sign-in configuration |
| `account_registry_db` | The account-registry database handle |
| `witness_json_absent` | `..._WITNESS_JSON` is unset |
| `witness_json_malformed` | `..._WITNESS_JSON` is not valid JSON, or has unknown or missing fields |
| `witness_url` | The `url` inside the witness JSON |
| `witness_signing_address` | The `signing_address` inside it |
| `witness_measurements_absent` | `expected_measurements` is an empty array |
| `witness_measurement_syntax` | An entry is not the `field=<96 hex>` form -- see above |
| `issuer` | `..._ISSUER_URL` / `..._AUDIENCE` |

Each control is named once per process, and a control tripped after an earlier
one is fixed is named in turn. No environment value, URL, signing address or
measurement string ever appears in these lines.

The native daemon enforces a host allowlist for every request in this flow.
It no longer requires a contributor to set one: with `TRACE_COMMONS_ALLOWED_HOSTS`
unset, the daemon derives an enforcing allowlist per signup step from the
Commons address the person typed on the signup screen, extended only by the
issuer and witness hosts that same origin publishes in its capabilities
response. Nothing else is reachable, and an origin the daemon cannot take a
host from is refused rather than admitted. This requirement previously fell on
the contributor, and no shipped application set the variable, so wallet signup
was impossible from Finder, the Start Menu or a flatpak.

An operator who does set `TRACE_COMMONS_ALLOWED_HOSTS` still governs the flow
in full: the list is used exactly as configured and must contain the Commons,
issuer and witness hosts, or those steps are refused. A configured
receipt-service host is validated against the same variable, so an operator who
sets `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` alongside an enforcing
allowlist must list its host too. The daemon also validates every published
measurement set before persisting witness settings. The integrated native
settings type must retain `admission_evidence=true`.

A refusal in this flow now names its class. `near_account_capabilities` returns
`reason` alongside `ready: false`: `address_refused` (rejected here, before any
request left the process -- not HTTPS, carrying credentials or a query, or not
on a configured host list), `unreachable` (dialled, no usable answer), or
`unsupported` (the Commons answered and does not offer wallet signup, or
published trust material the client will not accept).

Contributors get the receipt endpoint from the commons. A client with none
saved adopts the published value at wallet signup, and again the first time it
prepares a bound inference session -- so an account enrolled before the commons
published one is not stranded. Nobody has to edit `contributor.json`, and no
contributor is asked to type a URL they have no way to choose.

`TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` remains the operator override on a
host you control, and it outranks the published value. Set to something invalid
it refuses outright rather than falling through to the server, so a typo cannot
quietly hand the choice back.

Both values face the same address rules -- HTTPS, no URL credentials, query or
fragment -- but **each is vetted by the list its own source controls**, and the
distinction is load-bearing:

- An endpoint from `TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT` is checked against
  the operator allowlist. It came from an operator, so it is checked at the
  operator's trust level.
- A **published** endpoint is checked against the list derived from the origin
  the person chose, alongside that origin's issuer and witness. It is *not*
  checked against the operator allowlist: native signup persists
  `allowed_hosts: null`, so that list is the environment's, and it is permissive
  on every machine where nobody set `TRACE_COMMONS_ALLOWED_HOSTS`. Vetting a
  server-supplied host with a permissive list is vetting it with nothing.

The same derived list is what admits the saved endpoint later, when a bound
session is prepared. Gating it against a permissive list there would refuse it
as invalid one line after adopting it.

Neither path degrades to permissive: a non-enforcing list refuses outright
rather than waving a value through. When an operator *has* set an allowlist,
that list governs both, and a published endpoint outside it is refused. The
client never guesses a receipt URL from the selected inference backend.

A published endpoint that is not a dialable address is **dropped, not fatal**.
Enrollment still succeeds and the contributor contributes unattested; a commons
naming a receipt service this client will not call must not cost anyone their
account.

An absent endpoint still allows identity enrollment and window-based history
contributions; preparing a new bound inference session requires one, and says
so in its own words rather than telling the contributor to check settings that
were never wrong.

The receipt client appends `/signature/{chat_id}` and the served-model query,
so one base serves every model a deployment routes to. It refuses redirects, so
publish the canonical provider endpoint.

`GET /v1/account/near/provision/capabilities` returns `ready: false` until the
whole dependency chain is configured. A ready response includes `issuer_url`,
`audience`, `network`, `witness`, `inference_receipt_endpoint` (null when none
is published), and `funding_available: false`. Readiness describes
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

## Existing-history onboarding window

New signup keeps `witness.admission_evidence=true` for inference preparation,
but this flag does not require a receipt for pre-inference history. Before any
receipt lookup or witness request, the client inspects the captured final
request: presence of `metadata.trace_commons_admission` selects the strict
admission profile; no captured request or an unbound legacy request selects
ordinary **signed** witness review. A malformed/present marker, expired binding,
receipt-fetch failure, or witness HTTP failure never switches an admission
request onto the window route. Malformed request JSON is refused when selecting
the account-bound profile. No unsigned local fallback is introduced.

A window-enabled deployment must allow receipt-less requests on its ordinary
`POST /v1/witness` route. In the current witness binary, leave
`TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE` unset/false on the witness
published for that onboarding path. This controls the ordinary route only in
practice: `/v1/witness/admission` independently always requires its trusted
provider receipt and bound challenge. A deployment that requires receipts on
the ordinary route cannot offer the existing-history window; do not silently
retry another witness or weaken a requiring invited deployment to hide that
mismatch. Both profiles retain the configured enclave pin and signed redaction
artifact, explicit raw-session disclosure, and immutable approval bytes.

Ingest remains authoritative: an ordinary signed artifact from a provisioned
account consumes a configured window attempt/cost reservation or is rejected
when unavailable/exhausted. Neither a client flag nor a v1 certificate grants
admission. Existing real-PostgreSQL
`actual_postgres_challenge_witness_ingest_and_terminal_retry` coverage exercises
that server reservation and exhaustion, while contributor tests exercise the
receipt-less signed review through stored approval, strict receipt failure,
and absence of HTTP profile fallback. No remaining allowance is inferred by
the client and no funding is promised.

The V59 processing reservation begins at ingest, **after** remote witness review.
Its account/global ceilings cover the configured ingest processing bound, not
all earlier witness redaction or provider inference expenses. Witness request
limits, concurrency controls and any deployment funding limits are separate;
do not describe the ingest ledger as an end-to-end cap on pre-review spending.
