# Passkeys & Account Security (Slice 2) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add usernameless passkeys (enroll + discoverable login), account-security management (list/rename/remove passkeys, "this device"), and session rotation-on-use to the contributor-account feature.

**Architecture:** `webauthn-rs` 0.5 drives the WebAuthn ceremonies; in-flight ceremony state is held in-process (single-host pilot). Unauthenticated passkey login resolves `credential_id → tenant` through the existing narrow `trace_login_resolver` role (extended with a column GRANT + role-scoped permissive RLS policy, exactly like Slice 1's working login-link resolver) and then issues the same Slice 1 session cookie. Management + enroll are guarded by `resolve_account_ctx`; session rotation wires into the existing `validate_session` path.

**Tech Stack:** Rust, axum 0.8, PostgreSQL (forced RLS), `webauthn-rs` 0.5 (MPL-2.0, approved), reuses Slice 1's `cookie`/`rand`/`sha2`/`base64`. Spec: `docs/superpowers/specs/2026-06-22-contributor-account-passkeys-slice2-design.md`.

**Branch/worktree:** Execute on `contributor-account-slice2` (stacked on `contributor-account-slice1-impl`).

**Verification gates (before every Rust commit):**
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
PostgreSQL-backed tests self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`. The prior slice's agents stood up a throwaway Postgres to actually run them — **do the same** for the resolver `SET ROLE` test and the ceremony round-trips, and note any DB-skip in the task summary.

**`webauthn-rs` 0.5 API note:** the exact ceremony method names/types (`WebauthnBuilder`, `start_passkey_registration`, `finish_passkey_registration`, `start_discoverable_authentication`, `finish_discoverable_authentication`, `Passkey`, `DiscoverableAuthentication`/`PasskeyRegistration` state types, the `RegisterPublicKeyCredential` / `PublicKeyCredential` request bodies) MUST be verified against the `webauthn-rs` 0.5 docs at implementation time — pin the exact patch version in Cargo.lock and follow its README/examples. This plan specifies *what each step does*, not the library's exact symbol names.

---

## File Structure

| File | Responsibility | Create/Modify |
|---|---|---|
| `migrations/V32__webauthn_credentials.sql` | `trace_webauthn_credentials` table + RLS + resolver grant/policy; `trace_sessions` rotation + `auth_credential_id` columns; widen `client_kind` CHECK to include `'passkey'` | Create |
| `crates/trace-commons-server/Cargo.toml` | add `webauthn-rs = "0.5"` | Modify (`[dependencies]`) |
| `crates/trace-commons-server/src/db/postgres.rs` | register table in `TRACE_COMMONS_RLS_TABLES` (`:25`); wire V32 into `run_migrations` (`:823` pattern); add V32 to coverage-test arrays (`:2158`); `resolve_credential_tenant` (resolver pool, mirrors `resolve_login_link_tenant` `:175`); credential DB ops; `NewSession` + session-insert extension; rotation in `validate_session` (`:1816`) | Modify |
| `crates/trace-commons-server/src/db/mod.rs` | `NewSession` (`:407`) gains `auth_credential_id: Option<Uuid>`; new `Database` trait methods for credential ops + `validate_session_with_rotation` | Modify |
| `crates/trace-commons-server/src/config.rs` | WebAuthn RP id/origin/name config + env loaders (mirror `login_resolver_url` `:75`) | Modify |
| `crates/trace-commons-server/src/account_passkey.rs` | `Webauthn` builder from config; in-process ceremony store; pure helpers | Create |
| `crates/trace-commons-server/src/lib.rs` | `pub mod account_passkey;` | Modify |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | `Webauthn` on `AppState`; enroll/login/manage handlers; routes near `/v1/account/*` (`:5875` neighborhood); rotation cookie attach | Modify |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | ceremony round-trips + security regressions | Modify |
| `docs/operator/login-resolver-role.md` | extend: the resolver role now also reads `trace_webauthn_credentials` | Modify |

**Design boundary:** WebAuthn ceremony config + the in-process ceremony store + pure helpers live in `account_passkey.rs` (lib). All DB work goes through `PgBackend`/`Database` methods (each its own tenant tx). Thin axum handlers stay in `trace-commons-ingest.rs` beside the Slice 1 account handlers.

---

## PHASE 1 — Schema, resolver extension, scaffolding

### Task 1: V32 migration + dependency + RLS registration

**Files:** Create `migrations/V32__webauthn_credentials.sql`; modify `Cargo.toml`, `db/postgres.rs`; test in `tests.rs`.

- [ ] **Step 1 — failing test** (`tests.rs`, DB-backed, mirrors the V30 `account_migration_applies_and_enforces_rls`): assert `trace_webauthn_credentials` has `relforcerowsecurity = true`, and that `trace_sessions` now has columns `prev_token_hash`, `prev_token_valid_until`, `auth_credential_id`, `token_issued_at` (query `information_schema.columns`). Also assert `client_kind` CHECK accepts `'passkey'` (insert is fine, or check the constraint text).

- [ ] **Step 2 — run → FAIL** (table/columns absent; or compile fails if you also add the const entry first). Note DB-skip if no PG.

- [ ] **Step 3 — write the migration.** Follow V30 conventions exactly (tenant_id `TEXT` FK→`trace_tenants ON DELETE CASCADE`, locally-owned ids `UUID`, `sha256:`-shaped CHECKs, forced RLS + `trace_corpus_tenant_isolation` policy). Schema per spec §Data model:
  - `trace_webauthn_credentials(tenant_id TEXT, credential_id TEXT, account_id UUID, passkey JSONB, label TEXT, created_at, last_used_at, revoked_at; PK(tenant_id, credential_id); FK(tenant_id, account_id)->trace_accounts ON DELETE CASCADE; UNIQUE(credential_id))` + index `(tenant_id, account_id) WHERE revoked_at IS NULL` + ENABLE/FORCE RLS + policy.
  - `ALTER TABLE trace_sessions ADD COLUMN prev_token_hash TEXT CHECK (prev_token_hash ~ '^sha256:[0-9a-f]{64}$'), ADD COLUMN prev_token_valid_until TIMESTAMPTZ, ADD COLUMN auth_credential_id TEXT, ADD COLUMN token_issued_at TIMESTAMPTZ NOT NULL DEFAULT now();` (prev_*/auth_credential_id nullable; no FK on auth_credential_id — it's a label compared for `this_device`, and credentials soft-delete). **`token_issued_at`** tracks when the CURRENT token was issued (mint or last rotation) so rotation cadence is measured from the last rotation, NOT from `created_at` — reusing `created_at` would make a session re-rotate on every request once past the interval. Existing rows default to `now()` at migration time (they rotate one interval later; fine).
  - Widen `client_kind`: drop + recreate the CHECK to `IN ('web','device','passkey')` (find the existing constraint name in V30; `ALTER TABLE trace_sessions DROP CONSTRAINT <name>, ADD CONSTRAINT <name> CHECK (client_kind IN ('web','device','passkey'))`).
  - **Resolver extension** (mirror V30's `trace_login_resolver_cross_tenant_read` exactly):
    ```sql
    GRANT SELECT (tenant_id, credential_id) ON trace_webauthn_credentials TO trace_login_resolver;
    DROP POLICY IF EXISTS trace_login_resolver_credential_read ON trace_webauthn_credentials;
    CREATE POLICY trace_login_resolver_credential_read ON trace_webauthn_credentials
        FOR SELECT TO trace_login_resolver
        USING (true);
    ```
  Add `webauthn-rs = "0.5"` to `Cargo.toml [dependencies]`. Register `trace_webauthn_credentials` in `TRACE_COMMONS_RLS_TABLES` (`postgres.rs:25`). Wire V32 into `run_migrations` (copy the V31 block at `:823`, version `32`, name `webauthn_credentials`). Add `trace_webauthn_credentials` to BOTH coverage-test arrays in `trace_commons_rls_registry_matches_migration_policy_coverage` (`:2158`).

- [ ] **Step 4 — run → PASS** (or DB-skip). Run the registry coverage lib test (no DB) — it must pass.

- [ ] **Step 5 — gates + commit** `Add V32 webauthn_credentials table, session rotation columns, resolver grant`.

---

### Task 2: `resolve_credential_tenant` + SET ROLE regression test (the Slice 1 lesson)

**Files:** `db/postgres.rs` (resolver-pool method); `tests.rs`.

- [ ] **Step 1 — failing test** (DB-backed, the load-bearing one): mirror the Slice 1 `login_resolver_reads_tenant_across_rls_under_set_role` test. Insert a `trace_webauthn_credentials` row (tenant-scoped). On a raw connection, `SET ROLE trace_login_resolver;` then `SELECT tenant_id FROM trace_webauthn_credentials WHERE credential_id = $1;` → assert it RETURNS the tenant. Also assert an out-of-grant column read (e.g. `account_id`) is rejected (`has_column_privilege` false or the SELECT errors). `RESET ROLE;`. This must FAIL without the V32 permissive policy and PASS with it — verify both (drop policy → fail, re-add → pass) against a throwaway PG.

- [ ] **Step 2 — run → FAIL** if policy/method missing.

- [ ] **Step 3 — implement** `pub async fn resolve_credential_tenant(&self, credential_id: &str) -> anyhow::Result<Option<String>>` on `PgBackend`, identical shape to `resolve_login_link_tenant` (`:175`): use `login_resolver_pool` (fail-closed `missing-control` error when `None`, never the runtime pool), `SELECT tenant_id FROM trace_webauthn_credentials WHERE credential_id = $1`. Add the same safety comment (globally-UNIQUE credential_id; redeem re-confirms under RLS; do not add non-unique lookup columns to the grant).

- [ ] **Step 4 — run → PASS** (verify fail-without/pass-with explicitly; report it).

- [ ] **Step 5 — gates + commit** `Add credential->tenant resolver under role-scoped RLS policy`.

---

### Task 3: WebAuthn config + `Webauthn` instance + `account_passkey.rs` scaffolding

**Files:** `config.rs`, `account_passkey.rs` (create), `lib.rs`, `trace-commons-ingest.rs` (AppState).

- [ ] **Step 1 — failing test** (unit, no DB): a test that builds the `Webauthn` instance from a config with a valid RP id/origin/name and asserts it constructs; and a test that the in-process ceremony store round-trips a stashed value by ceremony id and that a second `take` (single-use) returns `None`, and an expired entry returns `None`.

- [ ] **Step 2 — run → FAIL** (module/types absent).

- [ ] **Step 3 — implement:**
  - `config.rs`: add WebAuthn RP config (id/origin/name). Put it wherever app-level (non-DB) config lives; if only `DatabaseConfig` exists, add a small `WebauthnConfig` struct + env loaders `TRACE_COMMONS_WEBAUTHN_RP_ID/ORIGIN/NAME` mirroring `login_resolver_url_from_env` (`:75`). The feature is **fail-closed**: if any passkey route is reached but RP config is unset, deny with a safe missing-control name (don't build a half-configured `Webauthn`). Note where you put it.
  - `account_passkey.rs`: a `build_webauthn(cfg) -> Result<Webauthn>` using `WebauthnBuilder::new(rp_id, &rp_origin_url)?...build()?` (verify 0.5 API), and an in-process `CeremonyStore` — a `Mutex<HashMap<String, (CeremonyState, Instant)>>` in a `LazyLock` (or stored on AppState), with `put(id, state)`, `take(id) -> Option<state>` (single-use + TTL ~3 min eviction), keyed by a high-entropy opaque ceremony id (`generate_login_code()`-style from Slice 1). The stored value is an enum over the two webauthn-rs state types. Document the single-instance limitation.
  - `lib.rs`: `pub mod account_passkey;`.
  - `trace-commons-ingest.rs`: build the `Webauthn` at startup (when RP config present) and store `Option<Arc<Webauthn>>` on `AppState`; an `account_webauthn(state)` accessor that fails closed (503/uniform deny) when `None`, mirroring `account_db` (Slice 1).

- [ ] **Step 4 — run → PASS.**

- [ ] **Step 5 — gates + commit** `Add webauthn config, Webauthn instance, and in-process ceremony store`.

---

## PHASE 2 — Ceremonies

### Task 4: Credential DB ops (`Database`/`PgBackend` methods)

**Files:** `db/mod.rs` (trait), `db/postgres.rs` (impl), `tests.rs`.

- [ ] **Step 1 — failing tests** (DB-backed): `insert_webauthn_credential` then `load_webauthn_credential_for_account` round-trips; `list_account_credentials` returns active (non-revoked) rows; `rename` updates label; `revoke` soft-deletes (excluded from list); `load_webauthn_credential_by_id` (used at login under the resolved tenant) returns the passkey blob + account_id; cross-account: a credential under account A is not returned for account B.

- [ ] **Step 2 — run → FAIL.**

- [ ] **Step 3 — implement** as `Database` methods (each its own `ensure_trace_tenant`+`begin_trace_tenant_transaction` tenant tx, sibling pattern; parameterized; the LOGIN-path loader takes the resolver-derived tenant and is called AFTER tenant resolution, so it operates under RLS):
  - `insert_webauthn_credential(tenant_id, account_id, credential_id, passkey_json, label)`
  - `load_webauthn_credential_for_login(tenant_id, credential_id) -> Option<{account_id, passkey_json}>` (RLS-scoped; the credential row's tenant must match)
  - `update_webauthn_credential_after_login(tenant_id, credential_id, updated_passkey_json, last_used_at)` (persist the post-`finish` Passkey, which carries the new sign count)
  - `list_account_credentials(tenant_id, account_id) -> Vec<{credential_id, label, created_at, last_used_at}>` (active only)
  - `rename_account_credential(tenant_id, account_id, credential_id, label) -> bool`
  - `revoke_account_credential(tenant_id, account_id, credential_id) -> {revoked: bool, remaining: i64}`

- [ ] **Step 4 — run → PASS.**

- [ ] **Step 5 — gates + commit** `Add webauthn credential DB operations`.

---

### Task 5: Enrollment ceremony (authenticated)

**Files:** `trace-commons-ingest.rs` (handlers + routes), `account_passkey.rs` (helpers), `tests.rs`.

- [ ] **Step 1 — failing test** (DB-backed, using webauthn-rs's software-authenticator test helper if available, else a structured fixture): an authenticated `resolve_account_ctx` session does `register/start` → gets options + a ceremony cookie; `register/finish` with a valid attestation → a credential row exists for the account. Assert `exclude_credentials` includes already-enrolled ids on a second start.

- [ ] **Step 2 — run → FAIL.**

- [ ] **Step 3 — implement:**
  - `POST /v1/account/passkeys/register/start` — guard `resolve_account_ctx`; `account_webauthn(state)`; `start_passkey_registration(user.id=account_id (as Uuid), user_name=handle-or-"contributor", user_display_name=…, exclude_credentials=account's existing ids)`; stash the returned `PasskeyRegistration` in `CeremonyStore` under a fresh ceremony id; set a short-lived `HttpOnly` ceremony cookie; return the creation-options JSON. Audit nothing yet (or a coarse `passkey_register_started`).
  - `POST /v1/account/passkeys/register/finish` — guard; read ceremony cookie → `CeremonyStore.take(id)` (single-use; missing/expired → error); parse the `RegisterPublicKeyCredential` body; `finish_passkey_registration` → `Passkey`; serialize to JSON; `insert_webauthn_credential(ctx.tenant_id, ctx.account_id, credential_id, passkey_json, label)`. Audit `account_passkey_enrolled` (actor `ctx.actor_ref`, hash-only). Failed attestation → reject, no row.
  - Routes near the Slice 1 account routes.

- [ ] **Step 4 — run → PASS.**

- [ ] **Step 5 — gates + commit** `Add passkey enrollment ceremony`.

---

### Task 6: Discoverable login ceremony + tenant bootstrap + session issuance

**Files:** `trace-commons-ingest.rs` (handlers + routes), `tests.rs`.

This endpoint is UNAUTHENTICATED — it gets the full redeem-style hardening.

- [ ] **Step 1 — failing tests** (DB-backed): enroll a passkey (Task 5 path), then `login/start` (no auth) → request options + ceremony cookie; `login/finish` with a valid assertion → a `tc_account_session` cookie is issued (`client_kind='passkey'`, `auth_credential_id` set), 303. Security regressions: (a) **sign-counter regression** assertion → uniform deny; (b) a credential enrolled under account A's tenant cannot mint a session for a different tenant/account (cross-account); (c) forged/expired ceremony id → uniform deny AND **no `trace_tenants` row created** for any forged tenant (the Slice 1 Codex bug — assert no write); (d) unknown credential id → uniform deny identical to (c).

- [ ] **Step 2 — run → FAIL.**

- [ ] **Step 3 — implement:**
  - `POST /account/passkey/login/start` — no auth. Rate-limit (per-IP + global) + within the timing floor; `account_webauthn`; `start_discoverable_authentication()` (no allowCredentials); stash `DiscoverableAuthentication` state in `CeremonyStore`; set ceremony cookie; return request-options JSON. Any failure → uniform deny.
  - `POST /account/passkey/login/finish` — no auth, wrapped in the **timing floor** (inner/outer split like `confirm_login_handler` `:12603`), and rate-limited (per-IP + global + per-credential ceiling `~5/min` keyed on the asserted credential id). Steps: parse the `PublicKeyCredential` assertion → extract `credential_id`; `CeremonyStore.take(ceremony_id)` (missing/expired → uniform deny); **resolve tenant**: `account_db(state).resolve_credential_tenant(credential_id)` (resolver pool; `None`/`Err` → uniform deny); open the tenant context via the credential loader: `load_webauthn_credential_for_login(tenant, credential_id)` (`None` → uniform deny); `finish_discoverable_authentication(assertion, state, &[credential])` (verify signature + **sign-counter regression** → deny); `update_webauthn_credential_after_login(...)` with the post-finish Passkey + `last_used_at`; mint a session secret (`generate_session_secret`), `hash_secret`, and issue a session via a new `Database` method `issue_passkey_session(tenant, account_id, NewSession{ token_hash, client_kind:"passkey", expires_at, auth_credential_id: Some(credential_id-hash-or-id) })` that INSERTs into `trace_sessions` (mirror `redeem_login_link`'s insert `:1773`, now including `auth_credential_id`); build the **identical Slice 1 session cookie** (`{b64url(tenant)}.{secret}`, Secure/HttpOnly/SameSite=Strict/Path=/, 7d) + 303 to `/account` + no-store/no-referrer. Audit `account_passkey_login`.
  - **CRITICAL invariants:** every failure → one uniform non-enumerating deny (reuse `redeem_generic_deny` or an analogous fixed response) behind the floor; **NO `ensure_trace_tenant`** anywhere on this path before the credential is verified (resolver returns tenant only; the loader runs under RLS and writes nothing if the credential/tenant don't match).
  - `auth_credential_id` storage: decide whether to store the raw credential_id or a hash. Since `credential_id` is already an opaque public identifier (not a secret) and is the PK of `trace_webauthn_credentials`, storing it directly enables the `this_device` join. Store the credential_id (document that it is non-secret). Audit still never logs it raw beyond what hash-only rows allow — keep `auth_credential_id` out of audit `safe_metadata`.

- [ ] **Step 4 — run → PASS** (verify the no-`trace_tenants`-write regression explicitly).

- [ ] **Step 5 — gates + commit** `Add discoverable passkey login with credential-tenant bootstrap`.

---

## PHASE 3 — Management + rotation

### Task 7: Management endpoints (list / rename / remove) + this-device

**Files:** `trace-commons-ingest.rs` (handlers + routes), `tests.rs`.

- [ ] **Step 1 — failing tests** (DB-backed): list returns the account's active credentials with `this_device=true` for the credential that authenticated the current (passkey) session and false otherwise; rename updates the label; remove soft-deletes and returns `remaining_credentials`; a credential of another account is never listed/renamable/removable (404/empty); device-link session lists with all `this_device=false`.

- [ ] **Step 2 — run → FAIL.**

- [ ] **Step 3 — implement** (all guarded by `resolve_account_ctx`):
  - `GET /v1/account/passkeys` → `list_account_credentials(ctx.tenant_id, ctx.account_id)`; set `this_device` by comparing each `credential_id` against the current session's `auth_credential_id`. To get the current session's `auth_credential_id` into the handler, extend `AccountCtx` (or the cookie validation path) to surface it — add `auth_credential_id: Option<String>` to `AccountCtx`, populated by `resolve_account_ctx_cookie` from the validated session row (the bearer path sets `None`). Audit `account_traces`-style read audit not required; a coarse `append_account_audit` "passkey_list" read is optional — keep hash-only.
  - `PATCH /v1/account/passkeys/{credential_id}` → `rename_account_credential`; 404 if not owned. Audit `account_passkey_renamed`.
  - `DELETE /v1/account/passkeys/{credential_id}` → `revoke_account_credential`; return `{ removed: bool, remaining_credentials }`; 404 if not owned. Audit `account_passkey_removed`.

- [ ] **Step 4 — run → PASS.**

- [ ] **Step 5 — gates + commit** `Add passkey management endpoints with this-device`.

---

### Task 8: Session rotation-on-use

**Files:** `db/mod.rs`, `db/postgres.rs` (`validate_session`), `account_session.rs` (`AccountCtx`), `trace-commons-ingest.rs` (attach rotated cookie), `tests.rs`.

**Integration subtlety (call out):** rotation must emit a `Set-Cookie` on the HTTP response, but `validate_session` is a DB method and `resolve_account_ctx` returns an `AccountCtx`, not a response. Approach: `validate_session` becomes (or is wrapped by) `validate_session_with_rotation` returning `Option<{account_id, rotated_secret: Option<String>}>`. `resolve_account_ctx_cookie` puts the new cookie string into a new `AccountCtx.rotated_cookie: Option<String>` field. Add a helper `attach_account_cookie(ctx, response)` (or finalize each account response through it) that sets the `Set-Cookie` when present. Every `/v1/account/*` and `/account/*` authenticated handler must route its response through that helper. (Alternative: a tower middleware that stashes the cookie in a response extension and attaches it — note as an option; pick the lower-churn one and document it.)

- [ ] **Step 1 — failing tests** (DB-backed): a session whose `created_at`/token age is past `ROTATION_INTERVAL` → next validated request rotates: `token_hash` changes, `prev_token_hash` holds the old hash with `prev_token_valid_until = now()+GRACE`, and a `Set-Cookie` is present on the response; a request with the OLD cookie within `GRACE` still validates (multi-tab); the OLD cookie after `GRACE` is denied; a fresh (un-aged) session does NOT rotate (no Set-Cookie). Use injectable/short `ROTATION_INTERVAL`/`GRACE` constants (or a test seam) so the test doesn't wait hours.

- [ ] **Step 2 — run → FAIL.**

- [ ] **Step 3 — implement:** add `ROTATION_INTERVAL` (~12h) + `GRACE` (~2 min) constants (make them overridable for tests). Extend the `validate_session` SELECT to also match `(prev_token_hash = $1 AND prev_token_valid_until > now())`. On a live hit whose **`token_issued_at < now() - ROTATION_INTERVAL`** (NOT `created_at` — see Task 1), in the same tx: generate a new secret, `UPDATE trace_sessions SET prev_token_hash = token_hash, prev_token_valid_until = now()+GRACE, token_hash = $new, token_issued_at = now(), last_seen_at = now() WHERE …`, and return the new secret so the cookie can be re-issued (value `{b64url(tenant)}.{new_secret}`). Setting `token_issued_at = now()` on rotation means the next rotation is one full interval away — a freshly rotated session does NOT immediately re-rotate (assert this in Step 1). Carry `auth_credential_id` through unchanged. Audit `account_session_rotated` (hash-only). Wire the rotated cookie through `AccountCtx.rotated_cookie` + the attach helper on all account handlers.

- [ ] **Step 4 — run → PASS.**

- [ ] **Step 5 — gates + commit** `Add session rotation-on-use with grace window`.

---

### Task 9: Regression sweep, operator doc, full gates

**Files:** `tests.rs`, `docs/operator/login-resolver-role.md`.

- [ ] **Step 1 — consolidate the security regression suite** (ensure all present + green against real PG): SET-ROLE resolver least-privilege (credential path); cross-account credential isolation (enroll A, cannot login/list/manage as B); sign-counter regression deny; forged-ceremony uniform-deny + no-`trace_tenants`-write; rotation interval+grace behavior; RLS forced on the new table; uniform-deny byte-identity across the passkey-login failure modes.

- [ ] **Step 2 — operator doc:** extend `docs/operator/login-resolver-role.md` to note the resolver role now also has `SELECT (tenant_id, credential_id)` on `trace_webauthn_credentials` + the `trace_login_resolver_credential_read` policy (same provisioning; no extra login role needed). Add the three `TRACE_COMMONS_WEBAUTHN_RP_*` env vars to the deployment runbook.

- [ ] **Step 3 — full sweep** (report each):
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
# with throwaway PG:
TRACE_COMMONS_PG_TEST_DATABASE_URL=... cargo test -p trace-commons-server
scripts/operator/pilot-bootstrap-smoke.sh
```

- [ ] **Step 4 — commit** `Finalize passkey slice: regression suite and operator docs`.

---

## Done criteria

All 9 tasks committed; full sweep green; the resolver `SET ROLE` test passes under the real role (verified fail-without/pass-with); cross-account, sign-counter-regression, forged-ceremony-no-write, and rotation regressions pass; `webauthn-rs` pinned; no raw credential public keys / ceremony secrets / un-hashed material in any audit row or log; passkey-login does no pre-verify `ensure_trace_tenant`; the new table is forced-RLS + registered + coverage-tested.

## Residual risks (carried, documented)

In-process ceremony state + single-instance limiter; authenticator-only recovery; interval rotation (not per-request); `webauthn-rs` MPL-2.0; device-revocation→session propagation still deferred from Slice 1.
