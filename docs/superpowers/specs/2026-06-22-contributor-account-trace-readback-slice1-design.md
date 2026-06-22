# Slice 1 — Contributor Account & Self-Trace Read-Back (Design Spec)

Date: 2026-06-22
Status: Approved for planning
Slice: 1 of 3

## Context

Contributors today authenticate to `trace-commons-server` via device-key-issued
EdDSA signed tokens; a request resolves to a `TenantAuth` carrying `tenant_id`
and a hashed `principal_ref`. Existing `/v1/contributors/me/*` endpoints expose
submission **status**, **credit balance**, and **credit events**, and there is an
optional public-profile opt-in (`trace_contributor_profiles`). There is **no**
path to read back the actual stored trace content, and **no** durable account
identity (each device key is effectively its own pseudonymous principal).

This feature lets a human examine **their own** submitted traces — both
lifecycle metadata **and** the stored privacy-scrubbed trace content — through a
robustly-authenticated web session, while device clients (Ironclaw) can also read
back via their existing device bearer token.

The work is decomposed into three slices, each with its own spec → plan → build
cycle. This spec covers **Slice 1 only**:

- **Slice 1 (this spec):** account core + device-link bootstrap login + read-back
  API scoped to an account's principals.
- **Slice 2 (future):** passkey / WebAuthn enrollment + login.
- **Slice 3 (future):** NEAR login + linking multiple device identities under one
  account + credit consolidation.

Slice 1 is designed so Slices 2–3 graft on without rework (see Graft Points).

## Repo invariants this design must honor

- PostgreSQL-only; every Trace Commons table has **forced RLS** with the
  `trace_corpus_tenant_isolation` policy on `trace_current_tenant_id()`.
- **Hash-only / label-only** audit and logs: never store or log raw URLs, tokens,
  codes, session secrets, KMS ARNs, object keys, ciphertext, trace bodies, or
  contributor identity.
- **Fail-closed**: when a required gate is configured but its dependency is
  missing, refuse with a safe missing-control name; never fall through to an
  unauthenticated or elevated state.
- Tenant scoping driven by auth-derived context; envelope tenant fields are
  attribution only.
- Match existing module/handler/migration patterns; do not split
  `trace-commons-ingest.rs`.

## Resolved decision: account creation timing

**Explicit-at-onboarding (create-or-reuse at MINT under authenticated tenant
RLS).** The redeem path carries **no tenant context** (no bearer, no cookie), so
`trace_current_tenant_id()` is NULL there. Create-on-first-redeem would force an
`INSERT` into RLS-forced tables from an unauthenticated public endpoint — exactly
the service-role RLS-bypass / fail-open surface the repo forbids. Therefore the
account + principal row is created at **mint time** (`POST /v1/account/login-links`,
authenticated by the device bearer, tenant + principal known), and redeem is a
pure *attach-session* operation. Mint is idempotent under concurrency via
`ON CONFLICT DO NOTHING` on `UNIQUE(tenant_id, principal_ref)`.

## Components

- **`migrations/V30__trace_accounts.sql`** — four tables + one audit table,
  following the V26/V29 template (FORCE RLS, `trace_current_tenant_id()`,
  tenant-leading PK/index, `sha256:`-shaped CHECK constraints, nullable-TIMESTAMPTZ
  soft-deletes). The migration header justifies why `trace_login_links` is
  separate from V29 `onboarding_invites` (different lifecycle, actor, TTL, and it
  is account-bearing).
- **`crates/trace-commons-server/src/account_session.rs`** (new sibling module) —
  cookie parse/serialize, CSPRNG generation, session validation, account/principal
  resolution, login-link mint/consume DB ops, the `AccountCtx` resolver, and
  `visible_submission_records_for_account`. Thin Axum handlers and `.route()` lines
  stay inline in `trace-commons-ingest.rs` next to `/v1/contributors/me/*`.
- **Restricted-role tenant resolver** — the single RLS-bypass touchpoint, used only
  to resolve `tenant_id` from a `code_hash` during the unauthenticated redeem
  (see Hardening D).
- **No new dependencies.** Cookie + CSPRNG hand-rolled (<30 LOC) over in-tree
  `rand`/crypto, per the repo dependency policy. Flagged to escalate to a vetted
  session/cookie crate (with explicit approval) if hand-rolling proves insufficient
  under code review.

## Data model (V30, exact repo convention)

Convention applied to all tables: `tenant_id UUID NOT NULL REFERENCES
trace_tenants(tenant_id) ON DELETE CASCADE` as **column 1**; `ENABLE` then `FORCE
ROW LEVEL SECURITY`; `DROP POLICY IF EXISTS trace_corpus_tenant_isolation ...;
CREATE POLICY trace_corpus_tenant_isolation USING (tenant_id =
trace_current_tenant_id()) WITH CHECK (tenant_id = trace_current_tenant_id())`;
tenant-leading indexes; `trace_` prefix; `sha256:`-shaped CHECKs on hash columns;
nullable-TIMESTAMPTZ soft-deletes; grants to the restricted runtime role.

Column types: `tenant_id` is **`TEXT`** to match `trace_tenants.tenant_id TEXT
PRIMARY KEY` (every existing `tenant_id` FK column in the repo is `TEXT`).
Locally-owned identifiers (`account_id`, `link_id`, `session_id`) are `UUID` —
they do not FK to a TEXT column. `account_id` must be the same type (`UUID`) in
both the parent `trace_accounts` and every child that carries the composite
`FOREIGN KEY (tenant_id, account_id)`.

```sql
-- trace_accounts: durable pseudonymous identity (no PII)
trace_accounts(
  tenant_id   TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
  account_id  UUID NOT NULL,
  created_at  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  closed_at   TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, account_id)
)

-- trace_account_principals: the union-scoping primitive
trace_account_principals(
  tenant_id     TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
  account_id    UUID NOT NULL,
  principal_ref TEXT NOT NULL,
  linked_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  unlinked_at   TIMESTAMPTZ,                       -- soft-delete; ACTIVE = unlinked_at IS NULL
  PRIMARY KEY (tenant_id, account_id, principal_ref),
  FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id),
  UNIQUE (tenant_id, principal_ref)                -- a principal links to AT MOST ONE account
)
CREATE INDEX idx_trace_account_principals_active
  ON trace_account_principals (tenant_id, account_id)
  WHERE unlinked_at IS NULL;                       -- the ONLY index used by set-expansion

-- trace_login_links: device-minted, ephemeral ~5min single-use
trace_login_links(
  tenant_id             TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
  link_id               UUID NOT NULL,
  account_id            UUID NOT NULL,             -- resolved at mint
  code_hash             TEXT NOT NULL CHECK (code_hash ~ '^sha256:[0-9a-f]{64}$'),
  created_principal_ref TEXT NOT NULL,             -- audit metadata only
  created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at            TIMESTAMPTZ NOT NULL,
  consumed_at           TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, link_id),
  FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id),
  UNIQUE (code_hash)
)
CREATE INDEX idx_trace_login_links_unconsumed
  ON trace_login_links (code_hash) WHERE consumed_at IS NULL;
CREATE INDEX idx_trace_login_links_active
  ON trace_login_links (tenant_id, created_principal_ref) WHERE consumed_at IS NULL;

-- trace_sessions: ~7d revocable browser sessions
trace_sessions(
  tenant_id    TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
  session_id   UUID NOT NULL,
  account_id   UUID NOT NULL,
  token_hash   TEXT NOT NULL CHECK (token_hash ~ '^sha256:[0-9a-f]{64}$'),
  client_kind  TEXT NOT NULL DEFAULT 'web' CHECK (client_kind IN ('web','device')),
  created_at   TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  last_seen_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  expires_at   TIMESTAMPTZ NOT NULL,
  revoked_at   TIMESTAMPTZ,
  PRIMARY KEY (tenant_id, session_id),
  FOREIGN KEY (tenant_id, account_id) REFERENCES trace_accounts(tenant_id, account_id),
  UNIQUE (token_hash)
)
CREATE INDEX idx_trace_sessions_account
  ON trace_sessions (tenant_id, account_id) WHERE revoked_at IS NULL;

-- trace_account_audit: V26 audit-pair convention, hash-only / label-only payload
trace_account_audit(
  tenant_id      TEXT NOT NULL REFERENCES trace_tenants(tenant_id) ON DELETE CASCADE,
  audit_sequence BIGINT NOT NULL,
  action         TEXT NOT NULL,        -- label, e.g. login_link_mint / redeem / session_revoke
  actor_ref      TEXT NOT NULL,        -- resolved principal_ref or account-actor:{id}; never raw identity
  outcome        TEXT NOT NULL,        -- label, e.g. created / reused / denied
  safe_metadata  JSONB NOT NULL DEFAULT '{}'::jsonb,  -- hash-only / label-only
  created_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
  PRIMARY KEY (tenant_id, audit_sequence)
)
```

## Flows

### Mint — `POST /v1/account/login-links` (authenticated device bearer)

1. `authenticate()` → `TenantAuth` (tenant + device `principal_ref`). Honor
   `device_keys.revoked_at` in the hot path — a revoked device cannot mint.
2. Outstanding-link cap (`(tenant_id, created_principal_ref)` unconsumed,
   non-expired) + per-device-principal rate limit → else `429` generic.
3. Create-or-reuse account under authenticated tenant RLS:
   `SELECT account_id FROM trace_account_principals WHERE tenant_id=$t AND
   principal_ref=$p AND unlinked_at IS NULL`; if absent, `INSERT INTO
   trace_accounts ... RETURNING account_id`, then `INSERT INTO
   trace_account_principals ... ON CONFLICT (tenant_id, principal_ref) DO NOTHING`,
   re-select.
4. CSPRNG `code` ≥128-bit (160 recommended), URL-safe base64, no truncation. Store
   `sha256(code)` + `account_id` + `created_principal_ref` + `expires_at =
   NOW()+5min`.
5. Return `{ url }`. Audit: hash-only mint event (actor principal only; never raw
   code/url).

### Redeem — `GET /account/login?code=...` → `POST /account/login/confirm`

- **GET** renders an "Activate" interstitial, **no consumption**. Headers
  `Cache-Control: no-store`, `Referrer-Policy: no-referrer`. IP-rate-limited.
- **POST /account/login/confirm** (same-origin enforced via `Origin` /
  `Sec-Fetch-Site`; IP + global + per-code rate-limited; fixed-latency floor):
  1. Restricted-role resolver (separate role + pool) returns `tenant_id` by
     `code_hash`.
  2. `begin_trace_tenant_transaction(tenant_id)` on the runtime pool.
  3. Atomic consume with explicit tenant re-confirmation:
     ```sql
     UPDATE trace_login_links SET consumed_at = NOW()
     WHERE code_hash = $1 AND tenant_id = $resolved_tenant
           AND consumed_at IS NULL AND expires_at > NOW()
     RETURNING account_id, created_principal_ref;
     ```
     `rows_affected != 1` → generic deny.
  4. CSPRNG session secret ≥128-bit. `INSERT INTO trace_sessions` storing
     `sha256(secret)`, `client_kind='web'`, `expires_at = NOW()+7d`.
  5. Cookie `Secure; HttpOnly; SameSite=Strict`, raw secret as value. Never accept
     a client-supplied session id.
  6. `303` to a code-free account view. Audit: hash-only redeem event
     (created-vs-reused label; actor principal only).
- **Error taxonomy:** {unknown, expired, consumed, wrong-tenant} collapse to **one**
  generic "link invalid or expired" with uniform status and timing. Granular errors
  only on the authenticated mint path.

### Session validation (every `/v1/account/*` cookie request)

Inside the tenant transaction: `token_hash` lookup **AND** `expires_at > NOW()`
**AND** `revoked_at IS NULL`, every request. Enforce idle cap via `last_seen_at`
(auto-revoke past the cap; otherwise update `last_seen_at`). Any session-store/DB
error → **deny** (401/500), never fall through. Logout → set `revoked_at`.
Account-level revoke-all: `UPDATE trace_sessions SET revoked_at=NOW() WHERE
tenant_id=$t AND account_id=$a`.

## Dual-auth read-back API

**`AccountCtx` resolver** (guards the whole `/v1/account/*` group) →
`{ account_id, principal_set: AccountPrincipalSet, auth_method }`:

- Bearer + Cookie both present → **400 ambiguous credentials** (no silent
  precedence).
- Bearer only → existing `authenticate()` → device `principal_ref` → resolve
  `account_id`; principal set = active-membership expansion (`unlinked_at IS NULL`).
- Cookie only → validate session → `account_id` → active-membership expansion.
  `auth_method = SessionCookie`; `TenantAuth.principal_ref = account-actor:{account_id}`
  (raw UUID literal, used for actor/audit only — **never** passed through
  `principal_storage_ref(...)`, so it is structurally incapable of matching any
  submission's `sha256:`-shaped `auth_principal_ref`). Default role =
  low-privilege contributor; never `can_review`/`can_admin`.
- **No endpoint accepts `account_id` or `principal_ref` as client input** — always
  auth-derived.

**Visibility predicate:** `visible_submission_records_for_account(AccountPrincipalSet,
records)` — SET membership only. The `can_review()` short-circuit and the
`legacy_principal_ref()` wildcard present in the existing
`visible_submission_records()` helper are **both removed** here. It accepts the
`AccountPrincipalSet` newtype, which is producible **only** by the `AccountCtx`
resolver; the legacy helpers take `&TenantAuth` and cannot accept it (type-level
guarantee enforced by a compile-fail test).

| Endpoint | Method | Returns | Notes |
|---|---|---|---|
| `/v1/account/traces` | GET | submission metadata list | Keyset pagination `?limit=N&cursor=<b64 of (created_at, submission_id) DESC>`; filter `auth_principal_ref = ANY($principal_set)`; index `(tenant_id, created_at DESC, submission_id)`; capped default + max limit; no offset. |
| `/v1/account/traces/{submission_id}` | GET | metadata detail | id checked against principal set; **uniform 404** on miss (no existence oracle). |
| `/v1/account/traces/{submission_id}/content` | GET | scrubbed envelope JSON | content semantics below. |

**Content semantics & controls (`/content`):**

- Routed through the existing `read_envelope_by_record()` /
  `read_envelope_from_object_ref()` path — inherits v1/v2 store dispatch, the
  `KekContext` bound to the caller's resolved `tenant_storage_ref` + `artifact_kind`,
  and `ciphertext_sha256` integrity. **No new decryption path.**
- **Contract: returns permanently-redacted (`[REDACTED]`) content.** Redaction is
  lossy and pre-encryption; **no un-scrubbed read-back path exists or is implied.**
  A round-trip test asserts `[REDACTED]` survives.
- **Fail-closed on KMS:** any unwrap/decrypt/hash-mismatch → generic label-only
  error (safe missing-control name) + hash-only audit row. Never ciphertext or
  plaintext; never echo object_key / KMS ARN / exception detail.
- **Bounds:** explicit max-bytes ceiling (generic oracle-safe error above it);
  buffered whole-object decrypt (GCM); per-account rate limit + concurrency cap.
- **Headers:** `Content-Type: application/json; charset=utf-8`, `Cache-Control:
  no-store`, `X-Content-Type-Options: nosniff`, no attachment disposition;
  same-origin enforced.

**Read-back error taxonomy:** `400` ambiguous credentials; `401` no/invalid
session or bearer (uniform, non-enumerating); `404` not-in-set OR nonexistent
(collapsed); `413`/generic over ceiling; `429` rate-limited; `500` fail-closed
(KMS/store/DB) with label-only reason.

## Security hardening (folded in from adversarial review)

- **A. Set-expansion staleness.** Every active-membership expansion of
  `trace_account_principals` — Slice 1 and forever — MUST filter
  `unlinked_at IS NULL` (backed by `idx_trace_account_principals_active`). This is
  a Slice-1 *contract*, not deferred to Slice 3. Regression test: a principal row
  with `unlinked_at` set must yield 404 for that principal's submissions and be
  absent from the expanded set.
- **B. Account-actor ref namespace disjoint.** Cookie path sets `principal_ref` to
  `account-actor:{account_id}` (not hashed), structurally incapable of equaling any
  `sha256:`-shaped ownership key. The principal **set** (never the actor ref)
  carries ownership. Test: `account-actor:{id}` matches zero submission records.
- **C. Type-level surface separation.** `visible_submission_records_for_account`
  takes `AccountPrincipalSet` (producible only by `AccountCtx`); legacy helpers
  cannot accept it. Compile-fail test asserts this.
- **D. Restricted-role resolver (highest-risk item).** A dedicated Postgres role
  `trace_login_resolver` is granted **exactly** `SELECT (tenant_id, account_id,
  code_hash) ON trace_login_links` and nothing else — no other table, no writes,
  no `BYPASSRLS`. It runs on a **separate, small connection pool** outside the
  RLS-guarded runtime pool, so the runtime RLS-bypass guard is unaffected. It reads
  by globally-`UNIQUE` `code_hash` and returns `tenant_id` only. The redeem handler
  then opens a fresh tenant transaction on the runtime pool and runs the consume
  UPDATE with an explicit `tenant_id = $resolved_tenant` predicate (belt-and-
  suspenders on top of RLS). The bypass-read and the RLS-scoped consume are in
  separate transactions on separate pools; they cannot diverge because `code_hash`
  is globally unique.
- **E. Cross-tenant code binding.** Prevented by D's explicit `tenant_id =
  $resolved_tenant` predicate + global `UNIQUE(code_hash)`.
- **F. Redeem rate-limiting key.** Per-IP token bucket on **both** the GET probe
  and the POST confirm, plus a coarse global ceiling, plus a per-`code_hash` hard
  attempt ceiling. Cryptographic backstop: 5-min TTL + ≥128-bit entropy.
- **G. Redeem timing oracle.** Confirm handler **always** executes the conditional
  UPDATE (affects zero rows on unknown code) — never SELECT-then-branch. Status,
  body, and a fixed minimum-latency floor are identical across {unknown, expired,
  consumed, wrong-tenant}.
- **H. Keyset pagination at cardinality > 1.** List query orders globally by
  `(created_at DESC, submission_id DESC)` with `= ANY($principal_set)` as an
  index-condition filter; index is tenant-leading `(tenant_id, created_at DESC,
  submission_id)` so the cursor totally-orders across any number of principals.
  Cursor = opaque base64 of `(created_at, submission_id)`; no offset.
- **I. Session bearer leak surface (accepted w/ mitigation).** Rotation-on-use and
  client-fingerprint binding are out of scope for Slice 1. Mitigations:
  `Secure; HttpOnly; SameSite=Strict`, `Referrer-Policy: no-referrer` on the
  account view, validate `expires_at` AND `revoked_at` every request, **enforced**
  idle cap via `last_seen_at` (auto-revoke past cap), and account-level revoke-all.

## Audit events (all hash-only / label-only)

| Event | Helper | Payload |
|---|---|---|
| login-link mint | `append_audit_event_with_db_mirror` | actor principal; never raw code/url |
| redeem (created-vs-reused) | `append_audit_event_with_db_mirror` | actor principal; account outcome label |
| session issue / revoke / revoke-all | `append_audit_event_with_db_mirror` | actor principal |
| list `/v1/account/traces` | `append_control_plane_read_audit` | `surface=account_traces_list`, `item_count` |
| detail `/{id}` | `append_control_plane_read_audit` | `surface=account_trace_detail`, `item_count=1` |
| content `/{id}/content` | `append_trace_content_read_audit_per_source` | `surface=account_trace_content`, `purpose_hash`, per-source `object_ref_ids` |

Actor is the resolved session/device `principal_ref` (`account-actor:{id}` for the
cookie path) so device-vs-browser reads are distinguishable without leaking
identity. Denied/failed reads and failed decrypts are also audited. Never log
object keys, KMS ARNs, ciphertext, raw codes/secrets, URLs, or contributor
identity.

## Fail-closed behaviors (consolidated)

- Account/principal INSERTs only under authenticated tenant context (mint); public
  redeem performs no account creation.
- Redeem consume = single atomic conditional UPDATE with explicit tenant
  re-confirm; `rows_affected != 1` → deny.
- Set-expansion always filters `unlinked_at IS NULL`.
- Session invalid/expired/revoked/idle-capped, or any session-store/DB error →
  deny; never fall through to unauthenticated or elevated state.
- Bearer + Cookie both present → reject.
- Restricted-role resolver: dedicated role, one-table SELECT grant, separate pool,
  never the runtime pool, no `BYPASSRLS`; returns `tenant_id` only; RLS-scoped
  consume re-confirms tenant.
- KMS unwrap/decrypt/hash-mismatch → generic label-only error + audit.
- Revoked device key → cannot mint, cannot read content.
- All redeem/session/read failure classes → non-enumerating uniform responses with
  a fixed-latency floor on redeem.

## Testing strategy

- **Migration/RLS:** V30 applies cleanly; each new table has forced RLS and the
  tenant-isolation policy; cross-tenant access denied.
- **Mint:** idempotent create-or-reuse; revoked device cannot mint; outstanding-link
  cap + rate limit enforced; only `code_hash` stored.
- **Redeem:** single-use (second confirm denied); expired/consumed/unknown/wrong-tenant
  all return the uniform generic error with identical timing floor; session cookie
  issued with correct flags; account creation never happens on redeem.
- **Session:** every-request expiry + revocation + idle-cap checks; logout and
  revoke-all; store/DB error → deny.
- **Read-back isolation (regression suite):** (a) two accounts, one tenant — A's
  session 404/empty on B's submission; (b) legacy-principal submission never
  returned; (c) review-role session confined to its own account on `/v1/account/*`;
  (d) unlinked-principal submission returns 404; (e) `account-actor:{id}` matches
  zero records.
- **Type-level:** compile-fail test that `AccountPrincipalSet`/`AccountCtx` cannot
  be fed to the legacy `visible_submission_records()` / `can_access_submission()`
  helpers.
- **Content:** `[REDACTED]` survives round-trip; KMS failure → generic label-only
  error + audit, no ciphertext/plaintext leak; max-bytes ceiling enforced.
- **Pagination:** keyset cursor totally-orders at cardinality 1 (and is verified to
  hold at N > 1 for the Slice-3 graft).
- Local verification per CLAUDE.md: `RUSTFLAGS="-D warnings" cargo check -p
  trace-commons-server --bins`, `RUSTFLAGS="-D warnings" cargo test -p
  trace-commons-server --no-run`, clippy with the repo allow-list, and the
  PostgreSQL-backed contract tests.

## Slice 2 / 3 graft points

- **Slice 2 (passkeys/WebAuthn):** a new authenticator attaches to an existing
  `account_id` (durable since create-at-mint). Add `trace_account_authenticators`
  (or extend principal semantics); session issue becomes auth-method-agnostic;
  `client_kind` extends with `'passkey'`. Session **rotation-on-use** grafts here
  once the passkey re-auth primitive exists. No change to read-back/RLS/audit shape.
- **Slice 3 (NEAR login + multi-device + credit consolidation):** linking a second
  principal = `INSERT INTO trace_account_principals`. Every read path already works
  at cardinality > 1 (set-expansion with `unlinked_at IS NULL`, `= ANY(set)` list
  query, tenant-leading keyset index). `UNIQUE(tenant_id, principal_ref)` prevents
  account-forking. **Linking gate:** require an existing strong authenticator
  (passkey/NEAR) before attaching a new principal, so a single stolen device token
  cannot pull other principals into "its" account. **Unlink** uses the present
  `unlinked_at` column, and the Slice-1 unlinked-principal-404 test already locks
  the read behavior. **Outstanding-link cap re-keyed** to `(tenant_id, account_id)`
  with a per-`created_principal_ref` sub-cap. Credit consolidation reads the union
  via the same predicate.

## Residual risks / accepted tradeoffs

1. **7-day session secret is a replayable bearer within its idle window.**
   Rotation-on-use / fingerprint binding deferred to Slice 2; mitigated by
   `SameSite=Strict` + `HttpOnly` + `Secure` + `no-referrer` + per-request
   revocation check + idle cap + revoke-all.
2. **Magic-link inherent bearer risk** (leaked URL within 5-min TTL, pre-confirm).
   Bounded by short TTL + single-use + GET/POST split + `no-referrer` + `no-store`
   + IP rate limit. Standard magic-link tradeoff; not eliminable for a URL-delivered
   link.
3. **Restricted-role resolver is a real (narrow) RLS-bypass surface.** Structurally
   required (unauthenticated redeem has no tenant context); reduced to a one-table,
   three-column SELECT grant keyed by a ≥128-bit secret hash, on a pool isolated
   from the runtime guard. Operational risk shifts to **role-provisioning
   correctness** — the single highest-priority implementation-review item.
4. **Per-account outstanding-link budget multiplies across devices in Slice 3**
   until the cap is re-keyed. Zero impact in Slice 1 (one principal per account).
5. **Hand-rolled cookie + CSPRNG (<30 LOC)** instead of a vetted session crate.
   Accepted under the repo dependency policy; flagged to escalate (with approval)
   to a vetted crate if hand-rolling proves insufficient under review.
6. **Redeem timing uniformity depends on a sleep-to-floor**, not genuinely
   constant-time DB work. Accepted: a fixed-latency floor above worst-case DB
   latency removes the practical oracle; the entropy backstop makes residual signal
   non-actionable.
