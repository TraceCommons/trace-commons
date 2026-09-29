# Browser passkey step-up page (Z2 S7)

A native session (`tcn1_`) is weak. It can read and withdraw, but once the
account holds a passkey it can never add or remove an authenticator or change
the payout: the Slice 3a gate refuses it with `403`
`a passkey or NEAR sign-in is required to change authenticators`. Those
changes need a strong browser session, which only a browser passkey sign-in
mints. This page is where the native app sends the person for one.

The page is served by ingest. It runs the existing browser passkey login
(`/account/passkey/login/{start,finish}`) and then the existing passkey and
payout routes. It adds no API, and nothing comes back to the app: the native
token is never upgraded. When the person is done, the app carries on with its
own weak session and re-reads whatever changed.

## A short-lived session

The page starts its sign-in at `POST /account/passkey/login/start?purpose=step_up`.
The purpose is bound into the ceremony, and the session that `finish` mints
lasts **15 minutes** (`STEP_UP_SESSION_TTL_MINUTES`) instead of the ordinary
seven days: both the session row's `expires_at` and the cookie's `Max-Age`.

- The 15 minutes are absolute. Activity only records `last_seen_at` for the
  idle cap (three days, which never binds here), and rotation-on-use swaps the
  secret without moving `expires_at`. A rotated cookie's `Max-Age` is capped at
  what is left of the session, so it cannot outlive the row either.
- `purpose` is an allowlist of one. `purpose=step_up` exactly, once; any other
  value, an empty value or a repeat gets the uniform passkey-login deny. A
  sign-in without `purpose` is unchanged: seven days.
- The sign-in audit row carries `"purpose": "step_up"` beside
  `"client_kind": "passkey"`. The lifetime is a constant, as the other session
  lifetimes are; it is not an operator setting.
- Nothing else in this tree calls the browser passkey login; the page is its
  only in-tree caller.

## URL contract (for the native client, C1)

```
GET https://ingest.tracecommons.ai/account/step-up
GET https://ingest.tracecommons.ai/account/step-up?return=<action>
```

Open it in the default browser or an `ASWebAuthenticationSession`. It must be
a real browser context on the ingest origin, because the passkey ceremony runs
there and the session is a `SameSite=Strict` cookie on that origin.

| Query | Meaning |
|---|---|
| none | After sign-in, show all three sections. |
| `return=add-passkey` | After sign-in, show only "add a passkey" (`POST /v1/account/passkeys/register/{start,finish}`). |
| `return=remove-passkey` | After sign-in, show only the passkey list with remove buttons (`GET /v1/account/passkeys`, `DELETE /v1/account/passkeys/{credential_id}`). |
| `return=change-payout` | After sign-in, show only the linked NEAR accounts, to pick where credit is paid (`GET /v1/account/near-identities`, `PATCH /v1/account/near-identities/{public_key}/payout`). |

Rules the client can rely on:

- `return` names a section of the page, never a URL. It must be exactly one of
  the three values above: case-sensitive, no whitespace, at most once. Any
  other value, a repeated `return`, or any other query parameter gets a `400`
  "This link is not valid" page that runs no script and does not echo the
  input. The client must not send an account id, tenant, token, or callback;
  there is nowhere to put one.
- There is no redirect back to the app. The page ends by offering "Sign out of
  this browser" (`POST /v1/account/logout`), which ends the browser session;
  the person then returns to the app themselves. An
  `ASWebAuthenticationSession` should therefore be dismissible by the user;
  there is no callback scheme to wait for.
- Status codes: `200` the page; `400` the invalid-link page; `503` the
  unavailable page (below). All three are `text/html`.
- The person signs in with any passkey registered for the account under the
  relying party `tracecommons.ai`. The page does not know which account the app
  holds and does not check; it acts on whichever account the passkey signs in.

## Unavailable state

If the WebAuthn relying party (`TRACE_COMMONS_WEBAUTHN_RP_*`) or the account
database is not configured, the page is a scriptless `503` saying passkey
sign-in is not available. It never offers a sign-in the server cannot finish,
and it never falls back to another mechanism.

## Relying-party origin (operator)

The page runs its ceremony on the ingest origin, so that origin must be on the
WebAuthn origin list alongside the apex:

```sh
export TRACE_COMMONS_WEBAUTHN_RP_ID="tracecommons.ai"
export TRACE_COMMONS_WEBAUTHN_RP_ORIGIN="https://tracecommons.ai,https://ingest.tracecommons.ai"
```

`ingest.tracecommons.ai` is a subdomain of the RP ID, so passkeys created
under `tracecommons.ai` (browser or native) sign in from it. Without the second
entry the page loads but every sign-in is refused with the uniform deny. As of
the S0 read on 2026-09-28 the pilot's origin was the single value
`https://tracecommons.ai`, so this change is needed before the page works on the
pilot.

## Response headers

Every response from `/account/step-up`, whatever it renders:

- `Content-Security-Policy`: `default-src 'none'`, `base-uri 'none'`,
  `form-action 'none'`, `frame-ancestors 'none'`. The ceremony page adds
  `script-src 'sha256-...'` and `style-src 'sha256-...'` (the one inline script
  and the one inline style block, pinned by hash, no `'unsafe-inline'`),
  `connect-src 'self'` and `require-trusted-types-for 'script'`. The
  unavailable and invalid pages have no `script-src` at all.
- `Cache-Control: no-store`
- `Referrer-Policy: no-referrer`
- `X-Content-Type-Options: nosniff`
- `X-Frame-Options: DENY`

No external script, style, font or image is loaded.

## Logs and audit

The page logs fixed labels only: `account_step_up_page_served` (with the
allowlisted action label or `menu`), `account_step_up_return_refused`, and
`account_step_up_unavailable`. No query value, cookie, address or account
detail. The page is unauthenticated and has no tenant, so it writes no audit
row itself; the sign-in and each change are audited by the routes it calls
(`account_passkey_login`, `account_passkey_enrolled`, `account_passkey_removed`,
`account_payout_designated`, and `account_authenticator_gate_denied` for a
refused weak session).

## Copy

All page text is in one table, `STEP_UP_COPY` in
`crates/trace-commons-server/src/bin/trace_commons_ingest_internal/step_up_page.rs`.
It is **proposed** and awaits approval.

## Limits

- Change payout lists only NEAR identities already linked to the account; the
  page does not link one.
- The browser session it mints lasts 15 minutes (see above), or less if the
  person signs out on the page.
- Not exercised in a real browser by the tests: the in-process tests drive the
  same HTTP calls the script makes, with a software authenticator reporting the
  ingest origin.
