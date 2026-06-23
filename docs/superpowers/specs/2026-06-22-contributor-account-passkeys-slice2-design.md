# Slice 2 — Passkeys & Account Security Management (Design Spec)

Date: 2026-06-22
Status: Approved for planning
Slice: 2 of 3
Depends on: Slice 1 (`contributor-account-slice1` — account core, device-link login,
session cookie, dual-auth `resolve_account_ctx`, narrow `trace_login_resolver` role,
read-back endpoints). This branch is stacked on the Slice 1 branch.

## Context

Slice 1 gave contributors a durable pseudonymous account reachable two ways: an
Ironclaw device-minted single-use login link (browser session) and the device
bearer token. Re-entering the web session today always requires Ironclaw to mint
a fresh link. Slice 2 adds **passkeys / WebAuthn** as a durable, phishing-resistant
web credential so a contributor can log back in on their own — plus the account
security management surface (multiple passkeys, list / rename / remove, "this
device") and the session **rotation-on-use** that was deferred from Slice 1
(residual risk #1) pending a re-auth primitive.

This is **Slice 2 of 3**. Slice 3 (login-with-NEAR + multi-device linking + credit
consolidation) remains deferred; its linking gate depends on a strong authenticator
existing first, which is exactly what this slice provides.

## Repo invariants this design must honor

- PostgreSQL-only; forced RLS via `trace_current_tenant_id()`.
- Hash-only / label-only audit and logs.
- Fail-closed by default with a safe missing-control name.
- Tenant scoping is auth-derived; no client-supplied account/principal input.
- Match Slice 1 patterns (session cookie format, `resolve_account_ctx`, the audit
  helpers, the uniform-deny + timing-floor + rate-limit treatment for
  unauthenticated surfaces, the resolver pattern).

### Slice 1 lessons folded in from the start

- **Resolver needs a role-scoped permissive RLS policy, not just a column GRANT.**
  A no-tenant-context resolver read of an RLS-forced table returns zero rows unless
  the role has a `... FOR SELECT TO <role> USING (true)` policy (the Slice 1
  regression). The `credential_id → tenant` lookup here gets that policy in its
  first migration, plus a `SET ROLE`-based regression test that exercises it under
  the real resolver role (not a superuser connection).
- **No `ensure_trace_tenant` on a pre-auth, client-supplied tenant.** The
  unauthenticated passkey-login path resolves the tenant from the verified
  credential via the resolver and never upserts `trace_tenants` before the
  credential is verified (the Codex-found Slice 1 bug, avoided here by construction).

## Resolved decisions

- **Credential model: usernameless discoverable credentials (resident keys).**
  Accounts are pseudonymous (no username/email), so login is "Sign in with a
  passkey" → the authenticator presents the credential → the server resolves the
  account from the credential id. WebAuthn `user.id` = the opaque `account_id`
  (no PII); `user.name`/`displayName` = the public handle if set, else a generic
  "TraceCommons contributor" label.
- **Attestation: `none`.** No need to fingerprint authenticators for pseudonymous
  contributors; `none` attestation is the privacy-preserving default.
- **Library: `webauthn-rs` 0.5 (MPL-2.0).** Opinionated, hard-to-misuse server
  ceremony API. `openssl` is already in the tree, so no new native crypto lib on
  the margin (vs `passkey-rs`, which would add a whole new RustCrypto stack).
  Approved as a direct dependency.
- **Recovery: authenticator-only.** The ways back in are (a) any enrolled passkey,
  (b) the Ironclaw device-link. No PII, no recovery codes, no operator override.
  Losing all passkeys *and* the device → unrecoverable by design. The management
  UI nudges enrolling ≥2 passkeys.
- **Session rotation: interval rotation with a grace window** (not per-request —
  see Session Rotation).

## Components

- **`crates/trace-commons-server/src/account_passkey.rs`** (new sibling module) —
  the `webauthn-rs` `Webauthn` builder (RP id/origin/name from config), ceremony
  orchestration, the in-process ceremony-state store, and the credential DB ops
  (as `Database`/`PgBackend` methods, each its own tenant-scoped tx).
- **Migration V32** — `trace_webauthn_credentials` table + the resolver grant and
  role-scoped permissive policy on it; new tables registered in
  `TRACE_COMMONS_RLS_TABLES`; `client_kind` enum extended with `'passkey'`; the two
  `trace_sessions` rotation columns.
- **Thin axum handlers** in `trace-commons-ingest.rs` beside the Slice 1 account
  handlers; new routes registered next to `/v1/account/*`.
- **No new external dependency beyond `webauthn-rs`.**

## Data model

### `trace_webauthn_credentials` (V32, tenant_id + forced RLS, V30 conventions)

```
tenant_id      TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE
credential_id  TEXT NOT NULL          -- WebAuthn credential id (b64url); globally UNIQUE
account_id     UUID NOT NULL          -- FK (tenant_id, account_id) -> trace_accounts ON DELETE CASCADE
passkey        JSONB NOT NULL         -- serialized webauthn-rs Passkey (public key + sign count live inside)
label          TEXT                   -- user-chosen, e.g. "iPhone"; nullable
created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
last_used_at   TIMESTAMPTZ
revoked_at     TIMESTAMPTZ            -- soft-delete on removal
PRIMARY KEY (tenant_id, credential_id)
UNIQUE (credential_id)                -- global: the no-context login resolution key
```
Index `(tenant_id, account_id) WHERE revoked_at IS NULL` for the management list /
active-credential expansion. Forced RLS + `trace_corpus_tenant_isolation`. Registered
in `TRACE_COMMONS_RLS_TABLES`.

### `trace_sessions` additions (rotation)

```
prev_token_hash        TEXT          CHECK (prev_token_hash ~ '^sha256:[0-9a-f]{64}$')  -- nullable
prev_token_valid_until TIMESTAMPTZ                                                       -- nullable
```

### Ceremony state — in-process, not a table

The two-request WebAuthn ceremony's in-flight `webauthn-rs` state
(`PasskeyRegistration` / `DiscoverableAuthentication`) is held in an in-memory map
keyed by a high-entropy opaque ceremony id carried in a short-lived (~3 min,
single-use) `HttpOnly` cookie. Single-host pilot stance (documented, like the rate
limiter); a ceremony lost to a restart just means the user retries. This
deliberately avoids a no-tenant-context DB table for the *login* ceremony (which
has no account/tenant until the assertion identifies the credential).

### Resolver extension (login tenant bootstrap)

Passkey login is unauthenticated; the assertion yields only a `credential_id`, so
`credential_id → tenant_id` is the same no-context lookup the login-link resolver
does. Extend the existing narrow `trace_login_resolver` role (V32):

```sql
GRANT SELECT (tenant_id, credential_id) ON trace_webauthn_credentials TO trace_login_resolver;
DROP POLICY IF EXISTS trace_login_resolver_credential_read ON trace_webauthn_credentials;
CREATE POLICY trace_login_resolver_credential_read ON trace_webauthn_credentials
    FOR SELECT TO trace_login_resolver
    USING (true);
```
Permissive policies OR together: only the resolver role gets the widened read; the
PUBLIC/runtime role keeps full tenant isolation; the column GRANT confines the
resolver to `(tenant_id, credential_id)`. The resolver returns **tenant only**; the
assertion is verified under the RLS-scoped tenant tx where the full `passkey` blob
is loaded. A new `resolve_credential_tenant(credential_id) -> Option<String>` on the
resolver pool mirrors `resolve_login_link_tenant`.

## Flows / ceremonies

### A. Enrollment (authenticated)

Guarded by `resolve_account_ctx` (the contributor already has a session — device-link
or an existing passkey).

- `POST /v1/account/passkeys/register/start` — `start_passkey_registration(user.id =
  account_id, user.name/displayName = handle-or-generic, exclude_credentials =
  account's existing credential ids)`. Returns creation options; stashes the
  `PasskeyRegistration` state in the ceremony map under a ceremony-id cookie.
- `POST /v1/account/passkeys/register/finish` — load ceremony state; `finish_passkey_
  registration` (verify, `none` attestation); INSERT the `Passkey` into
  `trace_webauthn_credentials` (tenant+account scoped) with optional `label`. Audit
  `account_passkey_enrolled`. A failed/tampered attestation → reject, no row written.

### B. Login (unauthenticated, discoverable) — the tenant bootstrap

- `POST /account/passkey/login/start` — no auth. `start_discoverable_authentication()`
  → request options with **no** `allowCredentials`. Stashes the
  `DiscoverableAuthentication` state under a ceremony-id cookie.
- `POST /account/passkey/login/finish` — no auth. The assertion yields a
  `credential_id`. **Bootstrap:** `resolve_credential_tenant(credential_id)` (narrow
  resolver pool) → tenant. Open the RLS-scoped tenant tx, load the stored `Passkey`,
  `finish_discoverable_authentication` (verify signature + **sign-counter regression
  check** = clone/replay defense), update `sign_count` + `last_used_at`, issue the
  session cookie exactly as Slice 1 redeem (`{b64url(tenant)}.{secret}`,
  `client_kind='passkey'`). 303 to `/account`. Audit `account_passkey_login`.
- **Unauthenticated-surface hardening (same as redeem):** every failure (unknown
  credential, bad assertion, counter regression, expired/replayed ceremony,
  cross-origin, resolver-miss, resolver-unconfigured) → one uniform non-enumerating
  deny behind the 250 ms timing floor + rate limiter (per-IP + global + per-credential
  ceiling). **No `ensure_trace_tenant`** before the credential is verified.

### C. Management (authenticated, via `resolve_account_ctx`)

- `GET /v1/account/passkeys` — list `{id, label, created_at, last_used_at,
  this_device}`. `this_device` is true when a credential id matches the one that
  authenticated the *current* session (passkey sessions only).
- `PATCH /v1/account/passkeys/{id}` — rename (`label` only). Audit `account_passkey_renamed`.
- `DELETE /v1/account/passkeys/{id}` — soft-delete (`revoked_at`). Always allowed
  (device-link remains the recovery path); response includes `remaining_credentials`
  so the client can warn before the last one. Audit `account_passkey_removed`.

## Session rotation-on-use

Shrinks the replay window of a *stolen* session cookie. **Interval rotation with a
grace window** (per-request rotation rejected for multi-tab/race fragility):

- On a validated request, if the current token is older than `ROTATION_INTERVAL`
  (~12h), mint a fresh secret: set `token_hash` = new, move the old hash to
  `prev_token_hash` with `prev_token_valid_until = now() + GRACE` (~2 min), and set a
  new cookie on the response.
- Validation accepts a request whose hash matches **either** the current `token_hash`
  **or** (`prev_token_hash` AND `now() < prev_token_valid_until`). The grace absorbs
  in-flight / multi-tab requests carrying the about-to-be-retired cookie.
- A stolen cookie is therefore useful for at most `ROTATION_INTERVAL + GRACE` once the
  legitimate user is active. The absolute 7-day `expires_at` and the 3-day idle cap
  still apply on top.
- Rotation is wired into the session-validation path (the same place that updates
  `last_seen_at`), so it applies to all session kinds (device-link and passkey), not
  just passkey-origin sessions. Audit `account_session_rotated` (hash-only) on rotation.

## Config (validated at startup; fail-closed if the passkey feature is enabled but unset)

- `TRACE_COMMONS_WEBAUTHN_RP_ID` — registrable domain (e.g. `tracecommons.ai`).
- `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN` — allowed ceremony origin(s) (e.g.
  `https://app.tracecommons.ai`).
- `TRACE_COMMONS_WEBAUTHN_RP_NAME` — display name in the passkey prompt.

These bind every ceremony; a mismatched origin fails verification in `webauthn-rs`,
which is what makes passkeys non-phishable (a credential for the RP id won't assert
on a look-alike domain).

## Error handling / fail-closed

- **Unauthenticated login surface**: uniform non-enumerating deny for every failure,
  behind the timing floor + rate limiter; no pre-verify tenant write.
- **Authenticated surface** (enroll + manage): standard `resolve_account_ctx` guard;
  granular-but-safe errors are fine. `exclude_credentials` prevents double-registering
  the same authenticator.
- **Ceremony state**: single-use, short TTL; lost/expired ceremony → retry.
- **Resolver pool / DB unconfigured** → fail closed (uniform deny / 503); never a
  runtime-pool fallback.
- **Sign-counter regression** on login → deny (cloned-authenticator signal).

## Audit events (all hash-only / label-only, via `trace_account_audit`)

`account_passkey_enrolled`, `account_passkey_login` (success), the login failure
variants collapsed to a generic `account_passkey_login_denied` (no per-cause detail),
`account_passkey_renamed`, `account_passkey_removed`, `account_session_rotated`. Actor
= `account-actor:{id}` (cookie) or the resolved principal. Never the credential public
key, a raw (un-hashed) credential id, or any ceremony secret.

## Testing strategy

- **Ceremony round-trips (DB-backed, real Postgres + `webauthn-rs`'s software-authenticator
  test harness):** enroll → discoverable login → session issued; rename + remove reflected
  in the list.
- **Security regressions:**
  - sign-counter regression assertion → deny (clone defense);
  - a credential enrolled under account A cannot authenticate as account B (cross-account);
  - `resolve_credential_tenant` works **under the real resolver role** via `SET ROLE`
    (the Slice 1 regression pattern, applied to the new grant + permissive policy —
    proves it before pilot, not under superuser);
  - forged/expired ceremony id → uniform deny **and no `trace_tenants` write** (the
    Codex-found Slice 1 bug, regression-guarded on this surface too);
  - rotation: a request past `ROTATION_INTERVAL` rotates the cookie and the old hash
    stays valid only within `GRACE`, then denies.
- **Type/RLS:** new table in `TRACE_COMMONS_RLS_TABLES`; forced-RLS assertion;
  resolver least-privilege (`(tenant_id, credential_id)` readable, other columns
  rejected — `SET ROLE` + `has_column_privilege`).
- **Full CI sweep:** `cargo check` / `cargo test --no-run` (`-D warnings`) + clippy
  allow-list + pilot-bootstrap smoke. (CI needs PostgreSQL ≥ 15.)

## Build decomposition (for the plan)

One cohesive slice, naturally ~3 phases:
1. Schema (V32) + resolver extension (grant + permissive policy + `SET ROLE` test) +
   `account_passkey.rs` scaffolding + `webauthn-rs` config + `Webauthn` builder.
2. Enrollment ceremony + discoverable-login ceremony + the credential→tenant bootstrap
   + the unauthenticated-surface hardening reuse.
3. Management endpoints (list / rename / remove) + session rotation-on-use.

## Residual risks / accepted tradeoffs

1. **In-process ceremony state + single-instance** — like the Slice 1 rate limiter;
   fine for the single-host pilot, revisit for multi-instance.
2. **Authenticator-only recovery** — no out-of-band recovery by design (no PII);
   mitigated by multi-passkey enrollment + the always-present device-link.
3. **Interval rotation, not per-request** — bounds stolen-cookie replay to
   `ROTATION_INTERVAL + GRACE` rather than a single request; chosen for multi-tab
   robustness.
4. **`webauthn-rs` is MPL-2.0** — file-level copyleft; fine to depend on, flagged for
   IP awareness.
5. **Device-revocation → session propagation** (carried from Slice 1, deferred):
   still not tied in; tracked for the device-revocation-integration follow-up.
