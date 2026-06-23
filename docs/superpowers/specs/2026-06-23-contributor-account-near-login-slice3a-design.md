# Slice 3a — Login-with-NEAR & the Strong-Authenticator Gate (Design Spec)

Date: 2026-06-23
Status: Approved for planning
Slice: 3a of the contributor-account feature
Depends on: Slice 1 (account core, session cookie, narrow `trace_login_resolver` role,
device-link login) and Slice 2 (passkey ceremonies, in-process ceremony store,
`resolve_account_ctx` auth middleware + rotation, the credential→tenant resolver
pattern). This branch is stacked on the Slice 2 branch.

## Context

Slice 3 (NEAR login + multi-device principal linking + credit consolidation) was
too large for one spec. It is split:

- **Slice 3a (this spec) — the authenticator layer:** login-with-NEAR (a NEP-413
  account-bound authenticator that mirrors the Slice 2 passkey architecture) plus a
  **strong-authenticator gate** on adding/removing any authenticator.
- **Slice 3b (deferred) — the principal/credit layer:** the device-principal merge
  (link a second Ironclaw device's submission-`principal_ref` into an account via the
  Slice 1 login-link as proof-of-control, close the merged account), credit re-keying
  from per-principal to per-account, the NEAR settlement worker, and payout to the
  3a-bound NEAR account.

This split is deliberate: re-pointing a principal that already has submissions and
per-principal credit is intrinsically a credit-attribution operation, so it belongs
with consolidation (3b), not the auth layer (3a).

### Why NEAR, and what it binds

The strategic goal (per the credit model) is settling perplexity-based credit to a
contributor's real on-chain NEAR account. "Login with NEAR" via **NEP-413 Sign-in**
proves control of a **named NEAR account** (e.g. `alice.near`), which becomes the
payout destination 3b settles to. 3a captures and authenticates that binding.

## Repo invariants this design must honor

- PostgreSQL-only; forced RLS via `trace_current_tenant_id()`.
- Hash-only / label-only audit and logs.
- Fail-closed with a safe missing-control name.
- Tenant scoping auth-derived; no client-supplied account/principal input.
- Reuse Slice 1/2 patterns: the session cookie shape, `resolve_account_ctx` middleware,
  the narrow-resolver + role-scoped permissive policy (the Slice 1 RLS lesson), the
  uniform-deny + timing-floor + rate-limit treatment for unauthenticated surfaces, the
  in-process ceremony store, hash-only audit, and the no-`ensure_trace_tenant`-before-
  verification invariant (the Codex-found Slice 1 bug class).

## Resolved decisions

- **NEAR identity = NEP-413 account binding** (named on-chain account), not a bare
  public key — so 3a sets up 3b's on-chain payout destination.
- **RPC only at enroll, never at login.** The single NEAR RPC call proves the public
  key controls the named account at *enroll* time. Login verifies the NEP-413 signature
  **offline** against the stored key (no network dependency on NEAR at login).
- **Enroll-from-session, login-resolves-existing, no auto-create** — NEAR mirrors
  passkeys: the account already exists (from device mint); NEAR is an added authenticator
  + payout identity, never a second account-bootstrap path.
- **A NEAR identity is structurally a passkey** — a stored credential authenticating to
  an existing account, bootstrapping its tenant via the narrow resolver. 3a reuses the
  Slice 2 architecture almost wholesale.
- **Strong-authenticator gate** with a bootstrapping carve-out (below).
- **Dependency:** add `bs58` (base58 decode for NEAR keys) — tiny, vetted, MIT. Borsh
  reconstruction of the fixed NEP-413 payload is hand-rolled (~25 LOC, no `borsh` dep).
  Reuse in-tree `ring`/`sha2`/`base64`/`reqwest`/`serde_json`.

## Components

- **Migration V33** — `trace_near_identities` table + the resolver grant + role-scoped
  permissive policy on it; `client_kind` widened to include `'near'`. New table
  registered in `TRACE_COMMONS_RLS_TABLES` + coverage arrays.
- **`crates/trace-commons-server/src/account_near.rs`** (new sibling module) — NEP-413
  payload reconstruction + offline Ed25519 verification, base58 key decode, the NEAR RPC
  `view_access_key_list` client + full-access check, and the NEAR DB ops (as
  `Database`/`PgBackend` methods). Config plumbing for RPC URL / network / recipient.
- **Thin axum handlers** in `trace-commons-ingest.rs` beside the Slice 2 passkey handlers;
  the authenticated ones go behind the existing `resolve_account_ctx` middleware.
- **No new external dependency beyond `bs58`.** `reqwest` (already present) for the RPC.

## Data model

### `trace_near_identities` (V33, tenant_id + forced RLS, V30/V32 conventions)

```
tenant_id        TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE
public_key       TEXT NOT NULL            -- "ed25519:<base58>"; the login lookup key
near_account_id  TEXT NOT NULL            -- e.g. "alice.near" (3b payout destination)
account_id       UUID NOT NULL            -- FK (tenant_id, account_id) -> trace_accounts ON DELETE CASCADE
label            TEXT
created_at       TIMESTAMPTZ NOT NULL DEFAULT now()
last_used_at     TIMESTAMPTZ
revoked_at       TIMESTAMPTZ              -- soft-delete on removal
PRIMARY KEY (tenant_id, public_key)
UNIQUE (public_key)                        -- globally unique: the no-context login resolution key
```
Index `(tenant_id, account_id) WHERE revoked_at IS NULL` for management list / active
expansion. Forced RLS + `trace_corpus_tenant_isolation`. Registered in
`TRACE_COMMONS_RLS_TABLES`.

### Resolver extension (login tenant bootstrap — third application of the Slice 1 lesson)

```sql
GRANT SELECT (tenant_id, public_key) ON trace_near_identities TO trace_login_resolver;
DROP POLICY IF EXISTS trace_login_resolver_near_read ON trace_near_identities;
CREATE POLICY trace_login_resolver_near_read ON trace_near_identities
    FOR SELECT TO trace_login_resolver
    USING (true);
```
Plus a `resolve_near_public_key_tenant(public_key) -> Option<String>` on the resolver
pool, mirroring `resolve_login_link_tenant` / `resolve_credential_tenant` (fail-closed
when the pool is unconfigured; returns tenant only). A `SET ROLE` regression test ships
with the migration, verified fail-without/pass-with under the real role.

### Sessions

`client_kind` widens to `IN ('web','device','passkey','near')`. A NEAR-authenticated
session sets `client_kind='near'`. Session **strength** (Section: gate) is derived from
`client_kind`: `'passkey'` and `'near'` are strong; `'web'` (device-link) is weak.

## NEP-413 verification (the crypto)

Given a wallet `signMessage` response `{accountId, publicKey, signature}` and the
server's stashed challenge `{nonce, recipient}`:
1. base58-decode `publicKey` (`ed25519:<base58>`) → 32 bytes. Malformed → reject.
2. Reconstruct the signed payload by borsh-serializing, in order:
   `tag: u32 = 2_147_484_061` (= 2³¹ + 413, NEAR LE u32), `message: String`,
   `nonce: [u8;32]`, `recipient: String`, `callbackUrl: Option<String>`
   (borsh = u32 LE tag; String = u32 LE length + UTF-8 bytes; [u8;32] = 32 raw bytes;
   Option = 1-byte tag (0/1) + value). Then `SHA-256` the borsh bytes.
3. `ring` Ed25519 verify the 64-byte `signature` over that 32-byte digest.
The `2³¹+413` tag is the NEP-413 anti-collision prefix that makes a signed login message
structurally unusable as a real NEAR transaction; it MUST be exactly this value, and a
payload whose tag mismatches is rejected.

## Flows / ceremonies

**Route naming:** the NEAR routes intentionally parallel the Slice 2 passkey routes
(`/v1/account/passkeys/register/{start,finish}`, `/account/passkey/login/{start,finish}`).
Use `/v1/account/near/enroll/{start,finish}` (authenticated, behind the middleware) and
`/account/near/login/{start,finish}` (unauthenticated, on the main router). The planner
should keep the enroll-vs-register / singular-vs-plural choice deliberate and consistent;
the authenticated NEAR routes go behind the existing `resolve_account_ctx` middleware, the
login routes stay outside it (like passkey login).

### Enroll — `POST /v1/account/near/enroll/{start,finish}` (authenticated; gated)

- *start*: `resolve_account_ctx` + the strong-auth gate (Section: gate). Generate a
  32-byte `nonce`; stash `{nonce, recipient}` in the ceremony store under a fresh
  ceremony-cookie id (short TTL, single-use). Return `{nonce, recipient}` for the wallet's
  `signMessage`.
- *finish*: `resolve_account_ctx` + gate. Take the ceremony state (missing/expired/
  wrong-variant → 400). Verify the NEP-413 signature **offline** against the response's
  `publicKey`, with the stashed `nonce`/`recipient`. Then the **one NEAR RPC call**:
  `view_access_key_list` for `accountId`; confirm `publicKey` is present with
  **FullAccess** permission (proves control of the named account; a function-call-only
  key or absent key → reject — this is the binding check that prevents claiming someone
  else's account). Store `(near_account_id=accountId, public_key, account_id, label)`.
  Audit `account_near_enrolled` (hash-only). RPC failure / config unset → fail-closed deny.

### Login — `POST /account/near/login/{start,finish}` (unauthenticated; redeem-style hardening)

- *start*: rate-limited (per-IP + global); generate `nonce` + `recipient`; stash; return.
  Any failure → uniform deny.
- *finish*: inner/outer **timing-floor** split; rate-limited (per-IP + global +
  per-public-key ceiling). Parse `{accountId, publicKey, signature}` (malformed body →
  uniform deny via a custom extractor). Take the ceremony state (missing/expired →
  uniform deny). Verify the NEP-413 signature **offline**. `resolve_near_public_key_tenant
  (publicKey)` (narrow pool; `None`/`Err` → uniform deny) → tenant. Under the resolved
  tenant's RLS: load the active binding by `public_key` (`None`/revoked → uniform deny);
  confirm the bound `account_id`; bump `last_used_at`. Issue the **identical Slice 1
  session cookie** (`client_kind='near'`, `auth_credential_id` left NULL — NEAR strength
  comes from `client_kind`) + 303 + no-store/no-referrer. Audit `account_near_login`.
- **Invariants:** every failure → one uniform non-enumerating deny behind the floor; NO
  `ensure_trace_tenant` before verification; no RPC at login; only the public key / nonce
  are handled (no secret to store — the wallet holds the key). The session secret is
  generated + hash-stored exactly as Slice 1/2.

## The strong-authenticator gate

Adding or removing ANY authenticator — passkey enroll/remove (tightening the existing
Slice 2 handlers) and NEAR enroll/remove — requires a **strong** session (`client_kind`
∈ {`passkey`,`near`}), **except** the **bootstrapping carve-out**: if the account has
**zero** active strong authenticators (count of active passkeys + active NEAR identities
== 0), a **weak** (device-link) session may add the *first* one.

- Surfacing: `AccountCtx` exposes the session's strength (from the validated session's
  `client_kind`). The gate helper counts active strong authenticators for the account
  (active `trace_webauthn_credentials` + active `trace_near_identities`).
- Effects: the normal onboarding path (a fresh device-link account enrolls its first
  passkey/NEAR) still works via the carve-out; a stolen device-link session cannot add an
  attacker authenticator once the user has any strong authenticator; removal stays allowed
  (device-link remains the always-on recovery), so removing all strong authenticators just
  returns the account to the bootstrap state — no permanent lockout.
- A gate rejection returns `403` (a real authorization failure on an authenticated
  surface; not the uniform-deny of the unauthenticated login path) and audits a gate event.

## NEAR management (authenticated, gated, account-scoped)

- `GET /v1/account/near-identities` → `{ near_account_id, public_key, label, created_at,
  last_used_at, this_session }` (`this_session` true for the NEAR identity that
  authenticated the current session — derived analogously to passkey `this_device`).
- `PATCH /v1/account/near-identities/{public_key}` → rename label; unknown/not-owned →
  uniform 404. Audit `account_near_renamed`.
- `DELETE /v1/account/near-identities/{public_key}` → soft-delete (subject to the gate);
  returns `{ removed, remaining_strong_authenticators }`; unknown/not-owned → 404. Audit
  `account_near_removed`. Cross-account isolation via `account_id` scoping.

## Config (validated at startup; fail-closed if NEAR is used but unset)

- `TRACE_COMMONS_NEAR_RPC_URL` — a trusted NEAR RPC endpoint (read-only `query` calls).
- `TRACE_COMMONS_NEAR_NETWORK` — `mainnet` / `testnet` (for sanity + logging).
- `TRACE_COMMONS_NEAR_LOGIN_RECIPIENT` — the NEP-413 `recipient` bound into every
  challenge (our RP id). A login/enroll whose signed `recipient` differs is rejected.
NEAR enroll/login fail closed if RPC URL or recipient is unset.

## Error handling / fail-closed

- **Unauthenticated NEAR login**: uniform non-enumerating deny for every failure
  (unknown/revoked key, bad signature, tag/recipient/nonce mismatch, expired/replayed
  challenge, resolver-miss, config unset), behind the timing floor + rate limiter; no
  pre-verify `ensure_trace_tenant`.
- **Enroll**: gated; RPC fail-closed (down / key-not-full-access / account mismatch →
  reject). Offline-valid signature whose key isn't a full-access key of the claimed
  account → reject (the binding check).
- **NEP-413 tag** must be exactly `2³¹+413`; mismatch → reject.
- **Config unset** → NEAR surface fails closed.

## Audit events (all hash-only / label-only, via `trace_account_audit`)

`account_near_enrolled`, `account_near_login` (success), `account_near_login_denied`
(the collapsed login-failure variants, no per-cause detail), `account_near_renamed`,
`account_near_removed`, and an authenticator-gate-rejection event. Never the signature,
nonce, or session secret in metadata; `near_account_id`/`public_key` live only in the row
(public on-chain identifiers) and stay out of audit `safe_metadata`.

## Testing strategy

- **NEP-413 unit tests**: a known good vector (`{message, nonce, recipient}` + key +
  signature → verifies); the borsh-payload reconstruction round-trip — explicitly
  covering `callbackUrl = None` (the live NEP-413 sign-in path, encoded as the 1-byte
  `0` Option tag) AND a `Some` case; base58 decode incl. malformed → reject;
  tag-mismatch → reject. A `ring` Ed25519 test signer drives enroll→login end-to-end
  without a real wallet.
- **NEAR RPC**: mock `view_access_key_list` — full-access present → enroll ok; absent or
  function-call-only → reject; RPC error → fail-closed.
- **Security regressions (DB-backed, real PG)**: resolver `SET ROLE` least-privilege on
  `trace_near_identities` (fail-without/pass-with the permissive policy); cross-account
  isolation (a NEAR identity under A can't login/list/manage as B); forged/unknown public
  key → uniform deny + **no `trace_tenants` write**; uniform-deny byte-identity; the
  **strong-auth gate** (weak session blocked from adding a 2nd authenticator; carve-out
  allows the 1st; gate also applied to the tightened passkey-enroll path; removal returns
  to bootstrap); forced-RLS on the new table.
- **Full CI sweep** + the DB-suite `--test-threads=1` note (carried from Slice 2; the
  process-global rate limiter is test-reset).

## Build decomposition (for the plan)

One cohesive slice, ~3 phases:
1. Schema V33 (table + resolver grant/policy + SET ROLE test) + `bs58` dep + config + the
   `account_near.rs` NEP-413 verification module (base58, borsh payload, offline Ed25519,
   the RPC client + full-access check) with its unit vectors.
2. Enroll ceremony (with the RPC binding check) + NEAR login (start/finish) + the
   credential→tenant bootstrap + session issuance (`client_kind='near'`), reusing the
   Slice 2 uniform-deny/timing-floor/rate-limit machinery.
3. The strong-authenticator gate (incl. tightening passkey enroll/remove) + NEAR
   management (list/rename/remove + this_session) + the regression sweep + operator docs
   (the new env vars; the resolver role now also reads `trace_near_identities`).

## Residual risks / accepted tradeoffs

1. **NEAR RPC trust** — a malicious/compromised RPC could falsely confirm a key→account
   binding at enroll. Mitigated by pinning a trusted endpoint; documented. (Login is
   unaffected — it's offline against the stored binding.)
2. **`bs58` new dependency** — tiny, vetted, MIT; flagged for approval at plan time.
3. **NEP-413 recipient binding is weaker anti-phishing than WebAuthn origin binding** —
   inherent to NEAR sign-in; the recipient + nonce + short-TTL single-use challenge bound
   the exposure.
4. **In-process challenge store / single-instance** — carried from Slice 2.
5. **`near_account_id` is a public on-chain identifier stored in a row** — not PII in the
   contributor-secret sense, but it links a pseudonymous account to a named NEAR account;
   kept out of audit/logs, in the RLS-forced row only. Acceptable given the contributor
   opted in by enrolling NEAR for payout.
