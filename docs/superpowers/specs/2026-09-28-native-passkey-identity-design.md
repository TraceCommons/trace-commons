# Native Passkey Identity for the macOS App (Z2) — Design

Date: 2026-09-28
Status: design proposed. The five decisions Zaki made on 2026-09-28 are
implemented here, not reopened. The items under "Decisions needed" are new.
Implementation not started
Item: Z2 in #1118 ("Native passkey identity: account-less passkey creation, a
native bearer from passkey login, AASA/webcredentials, the passkey-to-near.ai
binding"), gap 1
Builds on: Slice 2 passkeys
([`2026-06-22-contributor-account-passkeys-slice2-design.md`](2026-06-22-contributor-account-passkeys-slice2-design.md)),
Slice 3a's strong-authenticator gate
([`2026-06-23-contributor-account-near-login-slice3a-design.md`](2026-06-23-contributor-account-near-login-slice3a-design.md)),
Slice 3b consolidation
([`2026-06-24-contributor-account-consolidation-slice3b-design.md`](2026-06-24-contributor-account-consolidation-slice3b-design.md)),
the loopback native session (`tcn1_`), the NEAR AI login provisioning (#836,
V63), and earned account trust
([`2026-09-26-earned-account-trust-design.md`](2026-09-26-earned-account-trust-design.md))
Scope: `trace-commons-server` (ingest routes, `account_passkey.rs`,
`account_native_auth.rs`, `db/postgres_account_onboarding.rs`, one or two
migrations), `trace-commons-protocol` (one preimage function), the community
site (`community/public`), and the IPC surface the macOS client needs. No
production code in this PR.

In this document **F** is `crates/trace-commons-server/src/bin/trace-commons-ingest.rs`
and **NP** is `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/near_provisioning.rs`.
Line numbers are against `origin/main` at the time of writing.

## Problem

The WYSIWYG design's join screen offers "Sign in with a passkey: Create a
passkey that can be connected later." Nothing on the server can do that today:

- **Passkey registration needs an account first.** `register/start` sits
  behind `account_auth_middleware` (F:7609-7616, layer at F:7661-7664), takes
  `Extension<AccountCtx>` (F:18725-18728), and uses the existing account's
  UUID as the WebAuthn user id (F:18756-18764). "Create a passkey, connect
  later" has no account to attach to.
- **Ceremonies are bound by a cookie.** Both registration and login stash
  server state under an opaque id carried in the `tc_passkey_ceremony` cookie
  (F:18642, 3-minute max-age F:18647), `Secure; HttpOnly; SameSite=Strict`
  (F:18778-18786, F:19837-19845). A native client has no cookie jar for this.
- **Login issues a browser cookie, not a native bearer.** Passkey login
  finish mints `client_kind='passkey'` (F:20019-20023) and returns the
  `tc_account_session` cookie with a 303 (F:20042-20086). The native app
  authenticates `/v1/account/*` with a `tcn1_` bearer
  (F:15965-15971, `account_native_auth.rs:96`), which it can currently obtain
  only through the loopback PKCE redeem (F:17368-17389) or the NEAR AI
  provisioning finish (NP:1238-1257).
- **There is no `apple-app-site-association`.** Nothing in the repo serves
  `/.well-known/apple-app-site-association`, and nothing in the repo calls
  `navigator.credentials` or `ASAuthorization*` (a `git grep` finds only this
  server's handlers and the Slice 2 docs). The macOS "Save a passkey?" sheet
  cannot appear for our RP without it.
- **There is no "Verify passkey" step.** The design's P-5 and its lane
  captions ("near.ai verifies the signature · account linked") describe near.ai
  checking a passkey signature. near.ai does no such thing, and #1118 already
  places "one passkey signing in on near.ai itself" after the cut-off.

## Decisions already made (2026-09-28)

These are inputs. The spec implements them.

1. **Creating a passkey makes a zero-trust commons account.** WebAuthn RP is our
   domain. No existing session is needed. The account can watch and preview; it
   cannot contribute or earn until it is bound.
2. **"Verify passkey" binds that account to near.ai through the existing NEAR
   AI login provisioning** (`/v1/account/near-ai/provision/*`, F:7818-7833,
   NP:971-1258). near.ai performs no WebAuthn verification. The design's
   "Verify" semantics and copy are rewritten below, and the copy is marked as
   proposed.
3. **A native session from passkey login is weak**, the same class as `tcn1_`.
   Changing authenticators or payout requires a browser passkey step-up.
4. **near.ai is the account**, and an invite attaches to it. A passkey account
   that binds to a near.ai login which already maps to a commons account is
   resolved by the Slice 3b consolidation rules (see "Binding to an account
   that already exists").
5. **The native macOS app is decided**, and `ASAuthorization` passkeys are in
   scope.

## Repo invariants this design must honor

- PostgreSQL-only; forced RLS through `trace_current_tenant_id()`.
- Hash-only and label-only audit and logs.
- Fail-closed with a safe missing-control name.
- Tenant scoping is auth-derived. No client-supplied account, tenant or
  principal input; the NEAR AI finish body already refuses one at the parse
  boundary with `deny_unknown_fields` (NP:1050-1068).
- No `ensure_trace_tenant` on a pre-verification, client-supplied tenant
  (the Slice 1 bug class; see the note at `db/postgres.rs:4110-4116`).
- The unauthenticated surfaces keep the uniform deny, the timing floor and the
  rate limiter (F:19745-19764, F:19867-19876, F:17460-17477).

## Summary of the design

| Step | Route | Auth | Creates |
|---|---|---|---|
| Create passkey | `POST /v1/account/native/passkey/create/{start,finish}` | none | at **finish** only: tenant, account (unbound), credential, weak native session |
| Sign in | `POST /v1/account/native/passkey/login/{start,finish}` | none | a weak `tcn1_` session |
| Connect near.ai ("Verify") | `POST /v1/account/near-ai/provision/bind/{start,finish}` | `tcn1_` of an **unbound** account | anchor + device + principal on the same account; account becomes `bound` |
| Add a passkey to a signed-in account | `POST /v1/account/passkeys/native/register/{start,finish}` | `tcn1_`, Slice 3a gate | a credential |
| Change authenticators or payout | existing browser routes | browser `passkey` cookie session (step-up) | unchanged |

The browser routes (`/account/passkey/login/*`, `/v1/account/passkeys/*`) are
unchanged. The native routes are siblings, not modes, for the same reason the
NEAR AI ceremony is a sibling of the wallet one (NP:971-981): a mode flag would
put a branch inside a path that has none.

## The account's binding state

### States

```
                create/finish
   (nothing) ─────────────────► unbound ──bind/finish (new anchor)──► bound
                                  │  ▲
                   bind/start     │  │ bind refused / expired / cancelled
                                  ▼  │
                               (binding: a pending ceremony row, not an account state)
                                  │
            bind/finish (anchor already has an account) ──► closed (see "existing account")
                                  │
            reaper (unbound past TTL, no live session) ──► deleted
```

- **unbound.** Created by passkey `create/finish`. Holds exactly: its tenant
  row, its account row, one `trace_webauthn_credentials` row, its sessions and
  its audit rows. No principal, no device key, no anchor, no submission, no
  invite grant, no trust fact.
- **binding** is deliberately *not* a stored account state. It is a pending
  ceremony row in the existing ceremony table, keyed to the account. The
  account row stays `unbound` until the one transaction that writes the anchor
  also flips it to `bound`. There is no moment at which an account is
  half-linked: either the anchor, device and principal rows exist and the state
  is `bound`, or none of them exist and the state is `unbound`.
- **bound.** Terminal. There is no unbind. From here the account is an
  ordinary `nearai-` account and every existing rule applies to it.
- **closed.** Only reached through the existing-account path below.
- **Legacy accounts have no binding row** and are never subject to the
  unbound gate. Absence of a row means "not a passkey-origin account", not
  "unbound". This matters: device-link accounts in `tenant-…` namespaces have
  no anchor and are fully functional today.

### Storage (migration M1)

A new table, forced RLS, registered in `TRACE_COMMONS_RLS_TABLES`
(`db/postgres.rs:173`) and the migration-policy coverage arrays:

```
trace_account_bindings
  tenant_id   TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE
  account_id  UUID NOT NULL      -- FK (tenant_id, account_id) -> trace_accounts ON DELETE CASCADE
  origin      TEXT NOT NULL CHECK (origin = 'passkey')
  state       TEXT NOT NULL CHECK (state IN ('unbound', 'bound', 'closed'))
  created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
  bound_at    TIMESTAMPTZ
  CHECK ((state = 'bound') = (bound_at IS NOT NULL))
  PRIMARY KEY (tenant_id, account_id)
```

`origin` has one value now; it exists so a later account-less origin is a
widened CHECK, not a new table. The migration number is the next free one at
plan time (the tree ends at V91 with V83 absent; confirm against the PG test
database, per the migration-numbering note).

### Tenant: created at passkey creation, reused at bind

The tenant is minted at `create/finish`, with the same generator the NEAR AI
login uses: `random_near_ai_tenant_id()` (`near_account_identity.rs:453-465`),
a `nearai-` prefix plus 32 random bytes. It is **not** created at bind. Three
reasons, each sufficient:

1. **The WebAuthn user handle is the account UUID, and it cannot change.**
   The authenticator stores the user id it was given at registration, and
   login refuses an assertion whose handle differs from the credential's
   account (F:19967-19973). Minting a new account at bind would strand the
   passkey. So the account must exist at creation, and an account needs a
   tenant (`trace_accounts` PK and FK, `migrations/V30__trace_accounts.sql`).
2. **A tenant outside the anchored namespaces is on a different admission
   path.** `is_anchored_tenant` (`crates/trace-commons-protocol/src/admission.rs:206-222`)
   decides which path a request is on by prefix. An unbound account in a
   `nearai-` tenant sits in the namespace that account admission governs, and
   with no anchor and no provisioned device it fails closed there. A new prefix
   would put it on the invite-free path whose rules were never written for it.
3. **Tenants cannot be renamed or merged across.** Every merge in the tree is
   within one tenant (Slice 3b "Out of scope"; `V82__near_account_merge.sql`
   checks `p_tenant = trace_current_tenant_id()`).

At bind, the NEAR AI provisioning writes the anchor **into the existing
tenant and account** instead of minting either. The V58/V63 tables it writes
(`trace_near_account_anchors`, `trace_near_provisioned_devices`,
`device_keys`, `trace_account_principals`) are the same ones the unauthenticated
provisioning writes today (`db/postgres_account_onboarding.rs:544-573`). Only
the tenant and account decision differs.

## Flow 1 — Create a passkey (unauthenticated)

### `POST /v1/account/native/passkey/create/start`

Unauthenticated. Body: `{ "label": "<optional, <= 64 chars>" }`.

1. Per-IP and global rate limits (new constants beside F:17402-17414), and the
   **unbound-account ceiling** (below). Any refusal is `native_generic_deny`
   (F:17466-17477).
2. `account_webauthn` unconfigured -> uniform deny (as login start does,
   F:19817-19822).
3. Draw a fresh `account_id = Uuid::new_v4()`. Call
   `start_passkey_registration(account_id, name, name, None)` with
   `name` = the trimmed label or the fixed `ACCOUNT_PASSKEY_USER_LABEL`
   (F:18654). The label becomes the name the macOS sheet and the keychain show,
   which is what the design's P-2 ("My trace passkey") is for. A user-typed
   label is theirs to show on their own device; the fixed-label rationale at
   F:18649-18653 was about not leaking an account identifier, which a
   user-chosen label is not.
4. Store `CeremonyState::NativeCreate { reg_state, account_id, label }` under a
   new ceremony id in the in-process store (`account_passkey.rs:90`, TTL
   `CEREMONY_TTL` = 3 minutes at `account_passkey.rs:65`).
5. Return `{ ceremony_id, expires_in_secs, public_key: CreationChallengeResponse }`
   with `Cache-Control: no-store`. **No row is written anywhere.** An abandoned
   ceremony costs one in-memory entry until its TTL.

### `POST /v1/account/native/passkey/create/finish`

Unauthenticated. Timing floor over the whole handler, as `native_token_handler`
does (F:17584-17596). Body:
`{ "ceremony_id", "credential": RegisterPublicKeyCredential }`. The label is
not accepted here; it was fixed at start.

1. Rate limits; per-ceremony-id ceiling (as F:17627-17632 does for codes).
2. `take` the ceremony (single use). It must be the `NativeCreate` variant; a
   browser `Registration` or `DiscoverableAuthentication` entry is refused, so
   a ceremony started on one surface can never be finished on the other.
3. `finish_passkey_registration`. Failure -> uniform deny, nothing written.
4. Re-check the unbound-account ceiling (it may have moved during the
   ceremony).
5. **One transaction**, under the newly minted tenant: insert
   `trace_tenants`, `trace_accounts(account_id)`, `trace_account_bindings`
   (`unbound`), `trace_webauthn_credentials` (label from the ceremony), a
   `trace_sessions` row with `client_kind='native'` and
   `auth_credential_id = credential_id`, and the audit row
   `account_passkey_created` `{ "binding": "unbound", "labeled": bool }`.
   Writing the tenant here is not the Slice 1 bug: the tenant id is
   server-minted from the OS RNG after the attestation verified, never
   client-supplied.
6. Return the same shape as `NativeTokenResponse` (F:17447-17458) plus
   `"binding_state": "unbound"`. The raw token appears only in this body.

`credential_id` is globally unique (`V32`), so a replayed attestation fails
the insert even if the ceremony store were bypassed.

## Flow 2 — Sign in with a passkey (unauthenticated, native)

### `POST /v1/account/native/passkey/login/start`

Identical to `account_passkey_login_start_handler` (F:19796-19860) except the
ceremony is stored as `CeremonyState::NativeDiscoverable` and the id is
returned in the body (`{ ceremony_id, expires_in_secs, public_key }`) instead of
a cookie. No timing floor, for the reason given at F:19800-19804.

### `POST /v1/account/native/passkey/login/finish`

Body `{ ceremony_id, credential: PublicKeyCredential }`. The verification is
`account_passkey_login_finish_inner` (F:19880-20006) step for step: rate
limits, single-use take (variant must be `NativeDiscoverable`), identify, the
per-credential ceiling (F:19931-19938), `resolve_credential_tenant` on the
narrow resolver with no tenant write (F:19940-19946), load under RLS, the
handle binding check (F:19967-19973), `finish_discoverable_authentication`
with its counter check, the counter update. It should be factored so both
handlers share that core and differ only in how the ceremony is recovered and
what is issued; two copies of a login verifier are two places for the checks
to drift.

What is issued differs:

- `client_kind = 'native'` (`NATIVE_SESSION_CLIENT_KIND`,
  `account_native_auth.rs:75`), **not** `'passkey'`. That is decision 3, and it
  is also what makes the token resolvable at all: `resolve_account_ctx_native`
  refuses any session row whose `client_kind` is not `'native'` (F:16032-16034)
  and pins the resulting context weak (F:16005-16008, F:16058).
- TTL `NATIVE_SESSION_TTL_HOURS` = 12 (`account_native_auth.rs:68`).
- `auth_credential_id = credential_id`, so the passkey list can mark
  `this_device`. `resolve_account_ctx_native` currently drops it
  (`auth_credential_id: None`, F:16057); it should carry the session row's
  value. The id is public (F:18889-18890).
- Audit `account_passkey_native_login` `{ "client_kind": "native" }`.
- Response: the `NativeTokenResponse` shape plus `binding_state`
  (`unbound`, `bound`, or `legacy` for an account with no binding row).

### Why no PKCE here

The loopback flow needs PKCE because its code travels over a redirect that any
local process can race (`account_native_auth.rs:22-38`). Here the credential
is the assertion itself, and the token comes back in the HTTP response body to
the caller that sent the assertion. There is no second channel to intercept.

### Ceremony binding without cookies

The cookie in the browser flow binds a ceremony to the browser that started
it. The native replacement binds it to the challenge:

- The ceremony id is a 160-bit CSPRNG value (`new_ceremony_id`,
  `account_passkey.rs:113`), returned in the start body and presented in the
  finish body.
- The stored state holds the challenge; `webauthn-rs` refuses a response whose
  `clientDataJSON.challenge` differs. Holding the id without an authenticator
  that signed that exact challenge gets nothing.
- Single use (`CeremonyStore::take`), 3-minute TTL, and a per-id attempt
  ceiling.
- Variant-tagged: `NativeCreate` and `NativeDiscoverable` entries are
  refused by the browser handlers, and the browser variants by the native
  ones.

The attack a SameSite cookie defends against in a browser is a third-party
page driving the victim's browser through a ceremony it started. The native
app runs ceremonies only on challenges it fetched itself, and a phishing page
cannot obtain an assertion for our RP ID. The browser path keeps its cookie
unchanged.

### Rotation, revocation, expiry, renewal

A passkey-minted native session **is** a `trace_sessions` row with
`client_kind='native'`, so it inherits everything that governs `tcn1_` today:

- **Rotation-on-use**, handed back in `ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER`
  (F:17394-17395) by `account_auth_middleware` (F:15875-15896).
- **Logout** revokes exactly that row (F:18553-18558); **revoke-all** revokes
  every session of the account.
- **The idle cap and absolute expiry** in `validate_session`.

One addition: removing a passkey (`DELETE /v1/account/passkeys/{id}`) should
also revoke the account's live sessions whose `auth_credential_id` is that
credential. Slice 2 did not tie these together (its residual risk 5); a native
session minted by a now-removed passkey outliving the removal is the case where
it matters most.

**Renewal.** A 12-hour token means a Touch ID prompt when it lapses. For a
**bound** account the daemon can renew silently the way it does today, by
re-running NEAR AI provisioning with its stored refresh token
(`crates/trace-commons-contributor/src/daemon/nearai_onboarding.rs`); the
anchor resolves to the same account. An **unbound** account can only renew
with another passkey assertion. That is acceptable: it can do nothing but
watch until it is bound.

### Step-up

A native session is weak, and a passkey-origin account always holds at least
one strong authenticator (its passkey), so the Slice 3a gate
(`require_authenticator_change_allowed`, F:18675-18704) refuses every
authenticator change and every payout change from it
(payout: F:19563-19564). No new check is needed for that.

The step-up path is a **browser** passkey sign-in: the app opens a page on an
allowed RP origin (in `ASWebAuthenticationSession` or the default browser),
the page runs the existing `/account/passkey/login/{start,finish}` ceremony,
which mints a strong `client_kind='passkey'` cookie session (F:20013-20037),
and the change is made there through the existing routes. The native token is
never upgraded; nothing crosses from the browser session into the app.

**That page does not exist.** No file in the repo calls
`navigator.credentials`. Slice S7 below builds a minimal one, served by ingest.
Until it ships, a passkey-origin account cannot change authenticators or
payout at all, which fails closed.

### Adding a passkey to a signed-in account

`POST /v1/account/passkeys/native/register/{start,finish}`, behind
`account_auth_middleware`, is the existing authenticated registration
(F:18725-18892) with the body-borne ceremony id instead of the cookie. It keeps
the Slice 3a gate on both halves (F:18731-18733, F:18831-18833), so a weak
native session can add the **first** strong authenticator (the carve-out) and
nothing after that. This is what a contributor who signed in with near.ai
first uses to add a passkey, and what "Other sign-in options" needs in
reverse. It is refused for unbound accounts (one passkey per unbound account).

## Flow 3 — Connect near.ai ("Verify")

### What it proves, and what it does not

The bind proves that the holder of this passkey account also holds a live
NEAR AI session for subject S, by the same means the existing provisioning
uses: the commons introspects a NEAR AI access token
(`near_ai_login.rs`, `introspect_login` called at NP:1230-1236), bound to a
server nonce, a PKCE challenge and a device signature over
`near_ai_provisioning_device_bytes` (`crates/trace-commons-protocol/src/onboarding.rs:232-252`).
near.ai verifies nothing about the passkey, and the passkey does not sign
anything for near.ai. After the bind, S's anchor names this account, so a
later NEAR AI sign-in on any Mac lands in it.

### `POST /v1/account/near-ai/provision/bind/start`

Behind `account_auth_middleware`, native token only (a cookie session is
refused: the device key lives in the daemon). Body is `NearAiStartRequest`
(NP:1042-1048). Refused with `account_already_bound` unless the account's
binding row is `unbound`. Otherwise identical to `near_ai_start`
(NP:1091-1143), with two differences:

- The stored pending row carries `purpose: "bind"` and the caller's
  `(tenant_id, account_id)`. The row lives in the same ceremony table
  (`store_ceremony_payload`, `db/postgres_account_onboarding.rs:198-214`).
- The device signs a **bind** preimage, a new
  `near_ai_bind_device_bytes` in the protocol crate with its own domain string
  (`trace_commons.near_ai_bind_device.v1`) and the account id as an extra
  length-prefixed part. `NearAiLoginPending` is not `deny_unknown_fields`, so a
  bind row would deserialize as a provisioning row if the preimages were
  shared; distinct domain strings make a signature for one useless for the
  other, and `NearAiLoginPending` gains `deny_unknown_fields` as well.

Audit `account_binding_started` `{}`.

### `POST /v1/account/near-ai/provision/bind/finish`

Behind `account_auth_middleware`. Body is `NearAiFinishRequest` (NP:1058-1068),
unchanged, still `deny_unknown_fields`: the account comes from the session, not
the body. Every check of `near_ai_finish` (NP:1166-1236) runs in the same
order: bounds, single-use take, PKCE, device key match, device signature (bind
preimage), then introspection last. Additionally the pending row's
`(tenant_id, account_id)` must equal the session's; a bind ceremony started by
account A cannot be finished by account B.

Then the anchor decides, under the existing race handling
(`provision_against_anchor`, `db/postgres_account_onboarding.rs:333-362`, which
takes a `pg_advisory_xact_lock` on the anchor and relies on the global
`UNIQUE (anchor_hash)`, lines 534-561):

**(a) The anchor is unclaimed: bind in place.** One transaction in the
account's own tenant: insert the anchor (`identity_source='near_ai_login'`)
for **this** account, claim the device key (`near_ai` origin), link the
principal, insert the provisioned-device row, insert a fresh native session,
flip the binding row to `bound` with `bound_at = now()`, and audit
`account_bound` `{ "identity": "near_ai_login" }`. This is
`near_ai_login_provision_in_tenant` (lines 515-581) with the tenant and
account supplied instead of minted: the `SELECT … existing` / `INSERT
trace_accounts` branch (lines 544-562) becomes "use the given account", and
everything after it is shared. If `ON CONFLICT (anchor_hash) DO NOTHING`
reports the anchor was claimed meanwhile, the transaction rolls back and the
handler takes path (b).

**(b) The anchor already belongs to account X.** See the next section.

Response: the provisioning shape (NP:1253-1257) plus
`"outcome": "bound" | "existing_account"` and `binding_state`.

### Binding to an account that already exists (decision 4)

near.ai is the account. If S already anchors account X, the contributor is X;
the passkey account P was a stand-in. The question is only what happens to P
and its passkey.

Slice 3b's merge rules, applied here:

| 3b rule | Here |
|---|---|
| The absorbed side proves control with a capability (device-B login link) | P proves control with its passkey session |
| The surviving side must pass the strong-authenticator gate (`merge/confirm`) | X is proved only by the NEAR AI login, which yields a **weak** native session. So X passes the gate only through the carve-out: X has **zero** active strong authenticators |
| Absorbed authenticators move to the survivor; absorbed account closes | P's one passkey moves to X; P closes |
| Irreversible; B == A is a no-op; closed accounts are refused | same |
| Within one tenant | **not** satisfied: P and X are in different tenants |

So:

- **X has one or more active strong authenticators: refuse to move the
  passkey.** A NEAR AI login alone must not add an authenticator to an account
  that already has one; that is exactly what the gate at F:18675-18704 exists
  to stop. P is closed (binding `closed`, sessions revoked, credential
  revoked). The response still carries X's native session: the NEAR AI login
  proved X, and the existing unauthenticated provisioning grants that session
  on the same proof today (NP:1237-1257), so this confers nothing new. The app
  tells the contributor the passkey was not added and that they can add one
  from the account's existing passkey (browser step-up).
- **X has zero strong authenticators: fold P's passkey into X** (slice S6).
  This is the 3b merge with the carve-out on the survivor. It needs two
  things 3b never needed:
  - **A cross-tenant move.** One SECURITY DEFINER function, owned by a
    NOLOGIN NOBYPASSRLS guard, following `V82__near_account_merge.sql`'s
    pattern, that in one transaction moves exactly one credential row from P's
    tenant to X's, closes P and revokes P's sessions, and refuses unless P is
    `unbound` with no principal, no device key, no submission, no invite grant
    and no trust fact. Forced RLS still applies to the guard; it gets a
    permissive policy scoped to its role on the two tables it touches, the
    same way the resolver does (Slice 2, "Resolver extension").
  - **A stored user handle.** The moved passkey's handle is P's UUID, and the
    login check compares the handle with the credential's account
    (F:19967-19973). Add `trace_webauthn_credentials.user_handle UUID NULL`,
    set on the moved row to P's UUID, and compare against
    `COALESCE(user_handle, account_id)`. The check keeps its purpose (the
    handle must match what was bound at registration); it just stops assuming
    the owning account never changes.

  Until S6 ships, this case takes the refuse branch above. After the refuse,
  X has zero strong authenticators, so the native "add a passkey" route lets
  the contributor enrol a new passkey on X from the same weak session (the
  carve-out). The cost is a second Touch ID prompt and a dead P credential
  left in their keychain.

**Why not merge the other way (X into P)?** X may hold submissions, credit,
invite grants and trust facts, all tenant-scoped. Moving those across tenants
is the thing no merge in this tree does. Moving one credential row is the
smallest cross-tenant operation that achieves "one person, one account".

**Invites.** Decision 4 says an invite attaches to the near.ai account. An
unbound account is refused at `/v1/account/invites/redeem` and at the
legacy-link routes (the unbound gate, below), so an invite can never be
attached to P and stranded when P closes.

### "Cancelling signs you out"

**Decision: Cancel signs out and leaves the unbound account inert.** Server
side, Cancel is the existing `POST /v1/account/logout` with the native token
(F:18553-18558), and the app deletes its stored token. The account, its
passkey and its binding row stay `unbound`. Signing in again with the passkey
returns to the Verify step. A reaper deletes unbound accounts that stay unbound
and unused past a TTL (slice S5).

Why inert rather than deleted:

- **The join screen promises it.** "Create a passkey that can be connected
  later" is a promise that an unbound passkey keeps working.
- **Deletion leaves a dead passkey the server cannot remove.** The credential
  lives in the contributor's keychain. Deleting the server row turns the next
  "Use existing passkey" into a uniform deny with no explanation. Inert keeps
  the passkey meaningful; the reaper produces the same dead credential, but
  only after a long idle period rather than on a tap of Cancel.
- **Nothing is half-linked either way.** An unbound account holds no anchor,
  device, principal or grant, so there is nothing to unwind. "No half-linked
  accounts" is guaranteed by the single bind transaction, not by Cancel.
- **The cost is bounded.** A few rows per account, capped by the unbound
  ceiling and reclaimed by the reaper.

A bind that fails (introspection refused, ceremony expired, device signature
wrong) leaves the account `unbound` with no write beyond the consumed
ceremony. The client shows the Verify step again.

## The unbound gate

An unbound account authenticates normally, and `account_auth_middleware` then
restricts it to an **allowlist**. Any `/v1/account/*` route not on the list is
refused with `403` and the label `account_unbound`, so a route added later is
refused by default rather than reachable by default.

Allowed while unbound:

| Route | Why |
|---|---|
| `GET /v1/account/contribution-status` | answers `account_identity_unlinked` (the existing `AdmissionRefusal` label, `admission.rs:792-797`), so the client's R3 check stays on |
| `GET /v1/account/passkeys`, `PATCH /v1/account/passkeys/{id}` | see and rename its one passkey |
| `POST /v1/account/logout`, `POST /v1/account/sessions/revoke-all` | Cancel and sign-out |
| `POST /v1/account/near-ai/provision/bind/{start,finish}` | the only way forward |
| `GET /v1/account/binding` (new) | `{ binding_state }` for the app's state machine |

Refused while unbound, among others: invites and legacy-link redeem, the
inference-connection routes, the reward reservation routes
(`rewards.rs:48-67`, which consume finite offers), NEAR wallet enroll and
payout, merge, passkey add and remove, traces and credit (empty anyway).

The gate reads the binding row inside the request's tenant transaction. A read
failure refuses (fail closed). Legacy accounts (no row) skip the gate. The
lookup should be folded into the session validation query rather than added
as a second round trip on every authenticated request.

## Sybil analysis

Passkey creation is unauthenticated and the attestation is `none` (Slice 2),
so the server cannot tell a platform authenticator from a script. Anyone can
create unbound accounts at the rate the limits allow. What can they consume?

| Resource | Exposure | Bound |
|---|---|---|
| Ceremony memory | one entry per `create/start` | per-IP and global limits, 3-minute TTL, single-host store (`account_passkey.rs:12-22`) |
| Database rows | tenant, account, binding, credential, session, audit per `create/finish` | per-IP and global limits on finish; the **unbound ceiling**; the reaper |
| Uploads, admission | none | no device key and no anchor: admission in `nearai-` fails closed; the gate refuses the routes |
| Credit, trust | none | trust facts come only from accepted submissions (earned-trust spec, "Facts, not scores"), which need a bound account |
| Invites, rewards, inference funding | none | unbound gate |
| NEAR AI introspection (outbound) | one call per bind attempt | bind requires a live NEAR AI token and a device signature before introspection runs (NP:1226-1229); existing `limited()` and per-ceremony limits (NP:1172, NP:1190) |

**The unbound ceiling.** A new config value,
`TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING`: when the count of `unbound`
rows reaches it, `create/start` and `create/finish` refuse with the uniform
deny, and ingest logs the label `unbound_account_ceiling_reached` once per
crossing. Unset means passkey creation is **disabled** (fail closed), so a
deployment opts in with a number. The count is a cross-tenant read, so it goes
through a definer function or the resolver role, not the runtime pool.

**The sybil unit does not change.** Earned trust's sybil analysis ("Sybil
accounts" in the earned-trust spec) counts anchored NEAR accounts: each is one
tier-0 allowance. A passkey adds no allowance until it binds, and binding
consumes a NEAR AI subject whose anchor is globally unique
(`UNIQUE (anchor_hash)`). N passkeys bound to N NEAR AI accounts are exactly
the N sybils that exist without this spec; N passkeys bound to one NEAR AI
account are one account (the second bind takes the existing-account branch).
A bound passkey account starts at tier 0 like any `nearai-` account, and
nothing in this design is a trust fact.

## Account creation and a leaked NEAR AI token

Bind accepts a NEAR AI token on the same terms as the existing provisioning,
so it inherits that path's exposure and adds none. Today, anyone holding a
live NEAR AI access token for S can provision their own device into S's
account (NP:1230-1257), receive a weak native session, and, if S's account
has no strong authenticator, add the first one through the carve-out. With
bind, the same attacker could instead bind S's anchor to a passkey account
they created: S would later sign in with near.ai and land in an account where
the attacker holds a passkey. That is the same outcome as the existing path.
Neither path logs, stores or returns the token (NP:1065-1067,
`near_ai_login.rs` module docs), and the token is short-lived. If NEAR AI later
offers a sender-constrained or audience-bound token, both paths should adopt
it together.

## AASA and the relying party

### Which host, which file

`webcredentials` association is checked against the RP ID. With
`webcredentials:tracecommons.ai` in the app's entitlement, macOS fetches
`https://tracecommons.ai/.well-known/apple-app-site-association` through
Apple's CDN. That host must be the RP ID exactly, and the file must be:

- served with `200`, **no redirects**;
- `Content-Type: application/json`;
- this body, with the values from deploy config:

```json
{ "webcredentials": { "apps": ["<TEAM_ID>.ai.tracecommons.shell"] } }
```

The bundle id in the tree is `ai.tracecommons.shell`
(`macos/scripts/info-plist.sh:58`). The Team ID is the one in the Developer
ID certificate that `make-release-dmg.sh` signs with (`MACOS_SIGNING_IDENTITY`,
`macos/scripts/make-release-dmg.sh:39-40`). It is not a secret: every signed
binary carries it.

### Where it is deployed

The apex `tracecommons.ai` is the community site on Cloudflare Pages
(`community/wrangler.toml`, `community/public`); ingest is
`ingest.tracecommons.ai` (`community/public/_worker.js:1`). So the file lives
in the Pages project:

- `community/public/.well-known/apple-app-site-association`, rendered at
  deploy time from `TC_APPLE_TEAM_ID` and `TC_MACOS_BUNDLE_ID` (default
  `ai.tracecommons.shell`) by a step the `deploy:pages` script runs before
  `wrangler pages deploy`. With `TC_APPLE_TEAM_ID` unset the step writes no
  file and the deploy says so: no association means the app's passkey calls
  fail, which is closed.
- `community/scripts/check-community-site.mjs` validates the rendered file's
  shape (one `webcredentials.apps` entry matching `^[A-Z0-9]{10}\.`).
- **`_worker.js` must answer this path itself.** `serveAsset`
  (`_worker.js:55-68`) falls back to `index.html` with `200` for any `GET`
  404 whose last segment has no dot. `apple-app-site-association` has no dot,
  so a missing file would be served as the site's HTML with a `200`. The worker
  should route `/.well-known/*` before that fallback: return the asset with an
  explicit `content-type: application/json`, or a real `404`. Setting the
  header in the worker rather than relying on `_headers` removes any question
  about whether `_headers` applies to responses from an advanced-mode worker.
- A deploy smoke: `curl -sS -D - https://tracecommons.ai/.well-known/apple-app-site-association`
  must show `200`, `content-type: application/json`, no `location`, and the
  expected app id.

If the RP ID were ever `ingest.tracecommons.ai`, ingest would serve the file
instead, from an unauthenticated route beside `/v1/source` (F:7726). The
recommendation below keeps it on the apex.

### The RP ID, and whether it changes

The design's sheet reads "'tracecommons.ai' supports passkeys". The system
sheet prints the RP ID, so the RP ID must be `tracecommons.ai` for that copy
to be true.

What the tree says the pilot runs:

- `deploy/pilot-gcp/ingest.env.template:170-172` sets
  `TRACE_COMMONS_WEBAUTHN_RP_ID=${TC_PUBLIC_HOST}` and
  `RP_ORIGIN=https://${TC_PUBLIC_HOST}`.
- `docs/operator/pilot-gcp-deployment.md:120` sets `TC_PUBLIC_HOST=tracecommons.ai`.

So **if the pilot was deployed from the template, its RP ID is already
`tracecommons.ai` and nothing changes.** This spec cannot confirm that: the
pilot's configuration lives in the running process's environment, not in the
tree. Changing the RP ID invalidates every existing passkey
(`docs/operator/deployment.md:220`; `config.rs:181-183`).

Options, if the live value differs:

1. **Count first.** `SELECT count(*) FROM trace_webauthn_credentials WHERE
   revoked_at IS NULL` on the pilot. No browser passkey UI exists in the tree,
   so this is likely zero. If it is zero, change the RP ID; nothing is lost.
2. **If non-zero, accept the loss** and have those contributors re-enrol after
   signing in another way (device link or near.ai). Recovery is
   authenticator-only by design (Slice 2).
3. **Run two relying parties** during a transition, trying the old one when
   the assertion's `rpIdHash` does not match. `webauthn-rs` builds one RP per
   `Webauthn` instance, so this is two instances and a dispatch; it is not
   worth it for a pilot with no passkey UI.

**Recommendation:** option 1, then set the RP ID to `tracecommons.ai` if it
is not already.

### Origins

`webauthn-rs` checks `clientDataJSON.origin` against the configured origins.
Three origins matter:

- **Native.** For a platform-credential request, Apple reports the origin as
  `https://` plus the RP ID. With `RP_ORIGIN=https://tracecommons.ai` that
  matches. This is Apple behaviour, not something in this tree; the S2 test
  plan verifies it against a real signed build before anything relies on it.
- **The step-up page**, served by ingest at `https://ingest.tracecommons.ai`.
  A subdomain of the RP ID is a valid origin for it, but it is a *different
  origin* from `RP_ORIGIN`.
- **The apex**, if a page there ever runs a ceremony.

So `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN` becomes a comma-separated list, the first
entry passed to `WebauthnBuilder::new` and the rest to
`append_allowed_origin` (present in the pinned `webauthn-rs` 0.5.5,
`src/lib.rs:332`). `allow_subdomains(true)` is **not** used: it would accept
any future subdomain, including one served by a third party.

### Client prerequisites (not server work)

- The app needs the `com.apple.developer.associated-domains` entitlement with
  `webcredentials:tracecommons.ai`, which requires a real Team signature and an
  embedded provisioning profile. Development bundles are ad-hoc signed
  (`macos/scripts/make-app-bundle.sh:171-183`), and the release signing script
  has never run (`macos/scripts/make-release-dmg.sh:55-62`). **Nothing
  passkey-related can be exercised end to end until a signed build with that
  entitlement exists.** Apple's `?mode=developer` association can shorten the
  CDN cache during development.
- The Tauri app is not in scope (decision 5).

## Audit

All rows in `trace_account_audit`, actor = `account-actor:{id}`, hash-only and
label-only. No credential id, public key, challenge, token, anchor hash, NEAR
AI subject or provider label.

| Action | Metadata | Written |
|---|---|---|
| `account_passkey_created` | `{ "binding": "unbound", "labeled": bool }` | create/finish, in the new tenant |
| `account_passkey_native_login` | `{ "client_kind": "native" }` | login/finish |
| `account_binding_started` | `{}` | bind/start |
| `account_bound` | `{ "identity": "near_ai_login" }` | bind/finish (a) |
| `account_binding_refused` | `{ "reason": "anchor_claimed_strong" \| "anchor_claimed" }` | bind/finish (b), in P's tenant |
| `near_ai_login_provisioned` | existing shape (`db/postgres_account_onboarding.rs:573`) | bind/finish (b), in X's tenant |
| `account_passkey_folded` | `{ "authenticators_moved": 1 }` | S6, both tenants |
| `account_unbound_gate_denied` | `{}` | the gate |

Nothing is written for an abandoned or failed create or login: there is no
tenant to write it under, which is the "no account row for abandoned
ceremonies" property. Failed unauthenticated attempts stay invisible, as the
existing login is (F:19737-19742).

**The reaper cannot audit into the tenant it deletes** (the cascade removes
the row). It reports counts in its worker response and one label-only log line
per run.

## Threat model deltas

1. **A new unauthenticated write surface.** `create/finish` is the first path
   that creates an account from nothing but an attestation. Mitigations: rate
   limits, the unbound ceiling (disabled when unset), the reaper, and the
   unbound gate making the account worthless until bound.
2. **Zero-trust accounts touch no tenant data but their own.** The tenant is
   fresh and random, so forced RLS isolates it by construction; the gate
   additionally keeps it off every feature route. The gate is an allowlist, so
   a route added later is refused for unbound accounts unless someone adds it.
3. **Weak native sessions.** Passkey-minted native tokens are weak like every
   `tcn1_`: a stolen one can read and withdraw
   (`account_native_auth.rs:70-75`), not change authenticators or payout. This
   is unchanged risk for bound accounts and less for unbound ones, which cannot
   withdraw what they never submitted.
4. **Ceremony binding moves from a cookie to the challenge.** See "Ceremony
   binding without cookies". Surface-tagged ceremony variants keep the two
   surfaces from finishing each other's ceremonies.
5. **Cross-tenant fold (S6 only).** The first cross-tenant write in the tree.
   Confined to one definer function that moves one row, with the refusal
   conditions checked inside it rather than by the caller.
6. **AASA misconfiguration** fails closed: the system refuses the ceremony and
   the app shows an error. A wrong `200` HTML body (the worker fallback) is the
   failure worth guarding, hence the worker route and the smoke.
7. **RP ID change** is a one-time, operator-visible loss of existing passkeys,
   decided under "Decisions needed".
8. **Single-host ceremony store.** The native ceremonies use the in-process
   store, with the limitation documented at `account_passkey.rs:12-22`. The
   bind ceremony is in the database already.

## Proposed copy (needs Zaki's approval)

The design's copy says things that are not true of this system. Proposed
replacements; none of these should ship without approval.

| Screen | Design says | Problem | Proposed |
|---|---|---|---|
| Join | "Create a passkey that can be connected later." | fine | keep |
| P-1 | "A passkey is your sign-in for Trace Commons and near.ai." | the passkey does not sign in to near.ai (post-cut-off in #1118) | "A passkey is how you sign in to Trace Commons on this Mac. Nothing about your sessions is sent by signing in." |
| P-2 | "Losing it means losing access to your account and any credit in it." | once near.ai is connected, a NEAR AI sign-in reaches the same account; before that the account holds no credit | "Until you connect near.ai, this passkey is the only way back into this account. After that, signing in with near.ai works too." |
| P-5 title | "Verify your passkey" | nothing verifies the passkey here | "Connect near.ai" |
| P-5 body | "Sign a message to prove the passkey is yours and unlock contributing and credit." | the step is a near.ai sign-in, not a signature | "Sign in to near.ai to connect it to this account. That unlocks contributing and credit. Trace Commons keeps no email or name from near.ai." (true: introspection keeps only the subject id and a provider label, `near_ai_login.rs:62-90`) |
| P-5 buttons | "Verify" / "Cancel" | | "Continue to near.ai" / "Not now" |
| P-5 footnote | "Cancelling signs you out." | | "Not now signs you out. Your passkey keeps working, and you can connect near.ai next time." |
| Lane | "near.ai verifies the signature · account linked" | near.ai verifies no signature | "near.ai sign-in checked · account connected" |
| Lane | "Same passkey signs in on near.ai" | post-cut-off | remove |
| P-6/P-7 | "Welcome back. Sign in with your passkey." / "Other sign-in options" | fine; the near.ai option reaches only a connected account or creates a new one | keep; options: "Sign in with near.ai", "Use a different passkey" |
| Existing-account result | (none) | new state | "This near.ai account already has a Trace Commons account, so you're signed in to it. Your new passkey wasn't added." (+ "Add a passkey" when allowed) |

## Slices

Each slice is independently shippable and leaves `main` safe with the
following slices absent. Server slices first; the client work cannot be
exercised without S2, S3 and S4 and a signed build.

### S0 — Confirm the pilot RP (operator, no code)

Read the running ingest's `TRACE_COMMONS_WEBAUTHN_RP_ID` and `_RP_ORIGIN`, and
the active credential count. Record the answer on #1118. Blocks S4's RP change
only.

### S1 — Binding state and the unbound gate (server)

- M1: `trace_account_bindings`, RLS registry, coverage arrays.
- The allowlist gate in `account_auth_middleware`; `GET /v1/account/binding`;
  the `account_identity_unlinked` answer on contribution-status for unbound
  accounts.
- Inert: no route creates an unbound row yet.
- Tests: forced-RLS assertion for the new table; a table-driven test that
  enumerates every route on `authenticated_account_routes` and
  `rewards::account_routes` and asserts an unbound session gets `403
  account_unbound` on each route not on the allowlist (so a new route fails
  this test until it is classified); legacy accounts (no row) reach every
  route as before; a DB error on the binding read refuses.

### S2 — Native passkey create and sign-in (server)

- `CeremonyState::{NativeCreate, NativeDiscoverable, NativeRegistration}`;
  `create/{start,finish}`, `login/{start,finish}`,
  `passkeys/native/register/{start,finish}`; the shared login core factored out
  of F:19880-20006; `auth_credential_id` carried on native contexts; passkey
  removal revokes that credential's sessions; the unbound ceiling config
  (unset = disabled); the RP origin list.
- Tests, with the `webauthn-authenticator-rs` software authenticator already
  used by the ingest tests (`crates/trace-commons-server/Cargo.toml:199`):
  create then sign in, token resolves as `NativeToken` and weak; `create/start`
  writes no row; an abandoned ceremony leaves no row; a replayed
  `create/finish` is denied and writes nothing; a browser ceremony id presented
  to a native finish (and the reverse) is denied; the uniform deny is
  byte-identical across failure modes; `create/finish` with the ceiling reached
  writes nothing; the created account is `unbound` and gated; a native session
  cannot register a second passkey on an account that has one (Slice 3a gate)
  but can register the first on a near.ai-first account; removing a passkey
  revokes its native sessions; `resolve_credential_tenant` under the real
  resolver role via `SET ROLE` for a credential in a freshly minted tenant.
- **Manual verification before S3 depends on it:** one create and one sign-in
  from a signed macOS build against a staging ingest, recording the
  `clientDataJSON.origin` Apple sends.

### S3 — Bind through NEAR AI provisioning (server + protocol)

- `near_ai_bind_device_bytes` in `trace-commons-protocol` (permissive crate;
  nothing crosses the license boundary), `deny_unknown_fields` on
  `NearAiLoginPending`, the bind routes, the in-place bind transaction, the
  refuse branch with X's session and P's closure, the audit rows.
- Tests (real PostgreSQL, the introspection base-URL test hook at NP:1034-1040):
  unbound -> bound in one transaction (kill the transaction after the anchor
  insert and assert no anchor, no device, state still `unbound`); a provisioning
  signature is refused by bind finish and a bind signature by provisioning
  finish; a bind ceremony started by A and finished by B is refused; a bound
  account's bind start is refused; two concurrent binds of one anchor from two
  passkey accounts end with one `bound` and one on the existing-account branch;
  existing X with a strong authenticator -> passkey not moved, P closed, X
  session returned; a later NEAR AI provisioning for S lands in the bound
  account; an unbound account's invite redeem is refused; after binding, the
  account appears to admission as an ordinary `nearai-` account at tier 0.

### S4 — AASA and the relying party (community site + ops)

- The rendered `.well-known` file, the worker route and header, the check
  script, the deploy smoke, env template and `deployment.md` updates for the
  origin list, and the RP change if S0 says one is needed.
- Tests: the check script refuses a missing or malformed file; a worker unit
  test that `/.well-known/apple-app-site-association` never returns the index
  fallback.

### S5 — Unbound-account reaper (server)

- A worker route (admin-gated during rollout, like the earned-trust passes,
  then a scoped bearer) calling one definer function that deletes tenants whose
  only account is passkey-origin, `unbound`, older than the TTL, with no live
  session, and nothing else in the tenant. Returns counts.
- Tests: an unbound idle account is reaped; a bound one, one with a live
  session, and a legacy account are not; the function refuses a tenant holding
  any other row.

### S6 — Fold into an existing account (server, optional)

- `user_handle` column, the cross-tenant definer function, the handle check
  change, and switching the zero-strong-authenticator branch from refuse to
  fold.
- Tests: after a fold the passkey signs in to X, P is closed, P's sessions are
  revoked; the function refuses when X gained a strong authenticator in the
  meantime, when P has any principal or device, and when called outside a
  bind finish.

### S7 — Browser step-up page (server)

- A minimal page on ingest (`/account/step-up`) that runs the existing browser
  passkey login and links to the passkey and payout management it already
  exposes. No new API.
- Tests: an ingest-origin ceremony succeeds with the origin list; the page
  serves with a strict CSP and no third-party scripts.

### C1 — macOS client (Kristi / native team)

Not designed here. The surface it needs:

- **Entitlement and signing** as above.
- **Daemon IPC** (the daemon holds tokens and the device key; the app only runs
  the `ASAuthorization` UI):
  - `passkey_create_begin {label?}` -> `{ceremony, rp_id, challenge, user_id, user_name}`
  - `passkey_create_complete {ceremony, credential_id, raw_client_data_json, raw_attestation_object}` -> `{binding_state}`
  - `passkey_login_begin {}` -> `{ceremony, rp_id, challenge}`
  - `passkey_login_complete {ceremony, credential_id, raw_client_data_json, raw_authenticator_data, signature, user_handle}` -> `{binding_state}`
  - `account_bind {}` -> reuses `near_ai_account_enroll`'s machinery
    (`crates/trace-commons-contributor/src/daemon/ipc.rs:3571`) against the
    bind routes; -> `{outcome, binding_state}`
  - `account_binding` -> `{binding_state}`; `account_sign_out` -> logout.
  - `passkey_add_begin` / `passkey_add_complete` for the authenticated add.
- **One encoder.** Converting Apple's raw fields into the WebAuthn JSON that
  `webauthn-rs` expects (base64url fields, `response.clientDataJSON` and so on)
  belongs in one Rust function in the daemon, not in Swift and Rust both.
- **C ABI**: the label/tone/action helpers for the new states, following the
  `tc_near_ai_enroll_line` / `_tone` pattern
  (`crates/trace-commons-contributor-ffi/src/lib.rs:3596-3621`).
- **Copy**: the approved version of "Proposed copy".

## Decisions needed from Zaki

1. **Copy.** Approve or amend the "Proposed copy" table, in particular
   replacing "Verify your passkey" with "Connect near.ai".
2. **RP ID.** After S0, confirm `tracecommons.ai` and accept invalidating any
   existing passkeys if the live value differs.
3. **Cancel semantics.** This spec chooses inert-plus-reaper over deletion.
   Confirm, and set the reaper TTL (proposed: 30 days since the last session).
4. **Existing-account fold (S6).** Ship refuse-only first and add the fold
   later, or not at all? The fold is the tree's first cross-tenant write.
5. **Unbound ceiling.** The initial number for the pilot.
6. **Apple Team ID.** Which developer account signs the macOS app, and so
   which Team ID goes in the AASA file.

## Open questions

- **Orphaned passkeys.** When P is closed or reaped, its credential stays in
  the contributor's keychain. Whether macOS 26 exposes a way for an app to
  signal an unknown credential (the browser Signal API's counterpart) is
  unverified; if not, the app should tell the contributor they can delete it
  in Passwords.
- **Associated domains under Developer ID.** Confirm that a Developer ID
  provisioning profile carries `webcredentials` for a non-App-Store macOS app
  as expected, before C1 depends on it.
- **The native origin.** Confirm Apple's `clientDataJSON.origin` for platform
  credentials in S2's manual check.
- **Label as WebAuthn user name.** The P-2 label becomes `user.name`. Should it
  be length- and charset-bounded beyond 64 characters, and should the server
  store it (it does today as `label`)?
- **Multi-instance ingest.** The native ceremonies inherit the in-process
  store's single-host limitation. The pilot is single-host; a second instance
  would need the ceremony table the NEAR AI ceremonies already use.
- **Session renewal for unbound accounts.** A Touch ID prompt every 12 hours
  of use is the cost of weak, short native tokens. If that proves too much for
  the watch-only state, lengthen the TTL for unbound accounts only (they can
  do nothing with it) rather than for all native tokens.
- **Tauri.** `AGENTS.md` names the Tauri app as the MVP client. This design is
  server-side and client-neutral apart from AASA's app id; a Tauri build would
  need its own entitlement and an entry in the `apps` array.
