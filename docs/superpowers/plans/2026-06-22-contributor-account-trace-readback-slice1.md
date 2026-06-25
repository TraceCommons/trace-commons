# Contributor Account & Self-Trace Read-Back (Slice 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let a contributor examine their own submitted traces — lifecycle metadata and stored privacy-scrubbed content — through a robustly-authenticated browser session, with device clients able to read back via their existing device bearer token.

**Architecture:** A device-authenticated mint endpoint creates-or-reuses a durable pseudonymous account and issues a single-use login link; redeeming the link (unauthenticated, via a narrowly-scoped tenant resolver) attaches a hash-stored browser session. A dual-auth `AccountCtx` resolver (session cookie OR device bearer) expands the account's active principal set and gates new `/v1/account/*` read endpoints that reuse the existing submission-record and encrypted-artifact read paths. All new tables are tenant-scoped with forced RLS; all audit is hash-only.

**Tech Stack:** Rust, axum 0.8, PostgreSQL (forced RLS via `trace_current_tenant_id()`), `cookie` 0.18 (promoted to direct dep), existing `rand 0.8` + `sha2 0.10` for CSPRNG/hashing. Spec: `docs/superpowers/specs/2026-06-22-contributor-account-trace-readback-slice1-design.md`.

**Branch/worktree:** Execute on `contributor-account-slice1`.

**Verification gates (run before every commit that touches Rust):**
```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```
PostgreSQL-backed tests require `TRACE_COMMONS_PG_TEST_DATABASE_URL` (or `DATABASE_URL`); they self-skip when unset, so a worker without a DB still gets a clean compile gate but MUST note skipped DB tests in the task summary.

---

## File Structure

| File | Responsibility | Create/Modify |
|---|---|---|
| `migrations/V30__trace_accounts.sql` | 4 account tables + audit table + RLS policies + grants + `trace_login_resolver` role | Create |
| `crates/trace-commons-server/Cargo.toml` | Promote `cookie = "0.18"` to a direct dependency | Modify (`:9-40` deps block) |
| `crates/trace-commons-server/src/account_session.rs` | `AccountId`, `AccountPrincipalSet` newtype, `AccountCtx`, cookie ser/de, CSPRNG, session validation, login-link + session DB ops, `visible_submission_records_for_account` | Create |
| `crates/trace-commons-server/src/lib.rs` | `pub mod account_session;` (or `mod` if internal) | Modify |
| `crates/trace-commons-server/src/db/postgres.rs` | (a) register the 5 new tables in `const TRACE_COMMONS_RLS_TABLES` (`:19`) so the startup force-RLS verifier covers them; (b) dedicated `trace_login_resolver` pool + `resolve_login_link_tenant(code_hash)`; (c) a `pub` `PgBackend` wrapper exposing a tenant-scoped transaction to callers outside the `db` module | Modify |
| `crates/trace-commons-server/src/db/trace_corpus_pg.rs` | `begin_trace_tenant_transaction` lives here (`:1205`) but is `pub(super)` / called as `Self::begin_trace_tenant_transaction` on `PgBackend`; account DB ops must route through a `PgBackend` method, NOT call it cross-module | Reference (modify only if adding a wrapper) |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` | thin axum handlers (`mint`, `redeem` GET/POST, `account_traces_list`, `account_trace_detail`, `account_trace_content`), `.route(...)` wiring at `:5875`, `AccountCtx` extraction | Modify |
| `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` | unit/integration tests for all handlers + isolation regression suite | Modify |
| `crates/trace-commons-server/tests/trace_account_compile_fail.rs` (or `trybuild` case) | compile-fail proof that `AccountPrincipalSet` can't reach legacy helpers | Create |

**Design boundary:** all account/session/login-link *pure* logic (newtypes, cookie ser/de, CSPRNG, hashing, the visibility filter) lives in `account_session.rs` as testable free functions taking explicit inputs. **All DB work goes through `PgBackend` methods** (the `db` module owns `begin_trace_tenant_transaction`, which is `pub(super)`): add new `pub async fn` methods on `PgBackend` for each account DB op (create-or-reuse account, insert/consume login link, insert/lookup/revoke session, expand principal set) rather than calling the transaction helper cross-module. `trace-commons-ingest.rs` holds only thin axum glue (extract → call `PgBackend`/module → map errors → JSON), matching the existing contributor-handler shape at `trace-commons-ingest.rs:11476`.

**RLS registration (do not skip):** every new RLS-forced table MUST be added to `const TRACE_COMMONS_RLS_TABLES` at `src/db/postgres.rs:19`. The startup force-RLS verifier (postgres.rs:746/1666/1703/1736) iterates this list and asserts each table exists with forced RLS + the `trace_corpus_tenant_isolation` policy; a table absent from the const sits outside the repo's central RLS guarantee even if the migration is correct.

**No per-table GRANTs:** the repo has ZERO `GRANT` statements in migrations — the access model is `FORCE ROW LEVEL SECURITY` + `trace_current_tenant_id()` only. The 5 account tables follow that and need NO runtime-role GRANT. The single exception is the deliberate column-scoped `GRANT SELECT (...) ON trace_login_links TO trace_login_resolver` in Task 4 (the one intentional grant in the whole repo).

---

## Task 1: Migration V30 — tables, RLS, grants, resolver role

**Files:**
- Create: `migrations/V30__trace_accounts.sql`
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (new `#[tokio::test] account_migration_applies_and_enforces_rls`)

Reference conventions from `migrations/V29__onboarding_invites.sql:25-30` (RLS block) and `migrations/V1__trace_commons_schema.sql:11-15` (`trace_tenants.tenant_id TEXT PRIMARY KEY`).

- [ ] **Step 1: Write the failing test**

In `tests.rs`, add (uses `postgres_backend_for_ingest_test()` at `tests.rs:14` and `cleanup_pg_trace_tenant` at `tests.rs:37`):

```rust
#[tokio::test]
async fn account_migration_applies_and_enforces_rls() {
    let Some(backend) = postgres_backend_for_ingest_test().await else { return };
    // run_migrations() inside the helper already applied V30.
    let mut client = backend.raw_pool_for_tests_and_diagnostics().get().await.expect("conn");
    // Every new table must have relforcerowsecurity = true.
    for table in ["trace_accounts","trace_account_principals","trace_login_links","trace_sessions","trace_account_audit"] {
        let row = client
            .query_one("SELECT relforcerowsecurity FROM pg_class WHERE relname = $1", &[&table])
            .await.expect("table exists");
        let forced: bool = row.get(0);
        assert!(forced, "{table} must FORCE ROW LEVEL SECURITY");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `TRACE_COMMONS_PG_TEST_DATABASE_URL=... cargo test -p trace-commons-server --test ... account_migration_applies_and_enforces_rls -- --nocapture`
Expected: FAIL — `table exists` panics (tables absent) or migration not found. (If no DB: test self-skips and returns; proceed but note it.)

- [ ] **Step 3: Write the migration**

Create `migrations/V30__trace_accounts.sql`. Header comment in the V29 style explaining why login-links are separate from `onboarding_invites` (different lifecycle/actor/TTL, account-bearing). Use the exact column types from the spec §Data model (note: `tenant_id TEXT`, locally-owned ids `UUID`). For EACH of the five tables emit:

```sql
ALTER TABLE <t> ENABLE ROW LEVEL SECURITY;
ALTER TABLE <t> FORCE ROW LEVEL SECURITY;
DROP POLICY IF EXISTS trace_corpus_tenant_isolation ON <t>;
CREATE POLICY trace_corpus_tenant_isolation ON <t>
    USING (tenant_id = trace_current_tenant_id())
    WITH CHECK (tenant_id = trace_current_tenant_id());
```

Do NOT add any runtime-role `GRANT` — the repo has none; `FORCE ROW LEVEL SECURITY` + `trace_current_tenant_id()` is the entire access model (verified: zero `GRANT` statements across all migrations). Include the indexes from the spec (`idx_trace_account_principals_active`, `idx_trace_login_links_unconsumed`, `idx_trace_login_links_active`, `idx_trace_sessions_account`, and the list index `(tenant_id, created_at DESC, submission_id)` belongs to the existing submissions table — do NOT add it here; confirm in Task 8 whether it already exists).

Then register all five new tables in `const TRACE_COMMONS_RLS_TABLES` at `src/db/postgres.rs:19` (see File Structure → RLS registration). Extend the Step 1 test to also assert each new table name is present in that const (or add a second test asserting the startup verifier covers them), so the central RLS guarantee — not just table-level `relforcerowsecurity` — is locked in.

- [ ] **Step 4: Run test to verify it passes**

Run the same command. Expected: PASS (or self-skip without DB).

- [ ] **Step 5: Commit**

```bash
git add migrations/V30__trace_accounts.sql crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs
git commit -m "Add V30 contributor-account tables with forced RLS"
```

---

## Task 2: `cookie` dependency + `account_session.rs` skeleton & newtypes

**Files:**
- Modify: `crates/trace-commons-server/Cargo.toml`
- Create: `crates/trace-commons-server/src/account_session.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`
- Test: inline `#[cfg(test)] mod tests` in `account_session.rs`

- [ ] **Step 1: Write the failing test** (inline in `account_session.rs`)

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_principal_set_membership() {
        let set = AccountPrincipalSet::from_iter(["principal_abc".to_string()]);
        assert!(set.contains("principal_abc"));
        assert!(!set.contains("principal_xyz"));
    }
    #[test]
    fn account_actor_ref_is_not_sha_shaped() {
        let actor = account_actor_ref(&AccountId::from_uuid(uuid::Uuid::nil()));
        assert!(actor.starts_with("account-actor:"));
        assert!(!actor.starts_with("principal_"));
    }
}
```

- [ ] **Step 2: Run to verify it fails**

Run: `cargo test -p trace-commons-server --lib account_session -- --nocapture`
Expected: FAIL — module/types not defined.

- [ ] **Step 3: Implement skeleton**

In `Cargo.toml` deps block (near `base64 = "0.22"` at `:15`), add `cookie = "0.18"` (default features only; do NOT enable `signed`/`private`). Add `pub mod account_session;` to `lib.rs`.

In `account_session.rs` define:

```rust
use std::collections::BTreeSet;

/// Durable pseudonymous account id (locally-owned UUID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountId(uuid::Uuid);
impl AccountId {
    pub fn from_uuid(u: uuid::Uuid) -> Self { Self(u) }
    pub fn as_uuid(&self) -> uuid::Uuid { self.0 }
}

/// The ONLY ownership-bearing principal set for the account read surface.
/// Producible only here (and by `AccountCtx`), never convertible into the
/// shape the legacy `visible_submission_records(&TenantAuth, _)` helper wants.
#[derive(Debug, Clone)]
pub struct AccountPrincipalSet(BTreeSet<String>);
impl AccountPrincipalSet {
    pub(crate) fn from_iter<I: IntoIterator<Item = String>>(it: I) -> Self {
        Self(it.into_iter().collect())
    }
    pub fn contains(&self, principal_ref: &str) -> bool { self.0.contains(principal_ref) }
    pub fn as_slice(&self) -> Vec<String> { self.0.iter().cloned().collect() }
    pub fn is_empty(&self) -> bool { self.0.is_empty() }
}

/// Actor/audit ref for the cookie path. Reserved-prefix literal, NOT hashed,
/// so it is structurally incapable of equalling any `principal_<sha>` ref.
pub fn account_actor_ref(account: &AccountId) -> String {
    format!("account-actor:{}", account.as_uuid())
}
```

- [ ] **Step 4: Run to verify it passes**

Run: `cargo test -p trace-commons-server --lib account_session`. Expected: PASS.

- [ ] **Step 5: Verify gates + commit**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
git add crates/trace-commons-server/Cargo.toml crates/trace-commons-server/Cargo.lock \
        crates/trace-commons-server/src/account_session.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add account_session module skeleton and AccountPrincipalSet newtype"
```

---

## Task 3: CSPRNG code + hashing helpers

**Files:** Modify `account_session.rs`; tests inline.

Mirror `sha256_prefixed` usage from `trace-commons-ingest.rs:55774`. Reuse `rand 0.8` (as in `src/secrets.rs`) and `sha2 0.10`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn generated_code_is_high_entropy_and_url_safe() {
    let a = generate_login_code();
    let b = generate_login_code();
    assert_ne!(a, b);
    assert!(a.len() >= 27); // >=160 bits base64url, unpadded
    assert!(a.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'));
}
#[test]
fn hash_is_sha256_prefixed_shape() {
    let h = hash_secret("abc");
    assert!(h.starts_with("sha256:"));
    assert_eq!(h.len(), "sha256:".len() + 64);
}
```

- [ ] **Step 2: Run → FAIL** (`generate_login_code`/`hash_secret` undefined).

- [ ] **Step 3: Implement**

```rust
use rand::RngCore;
use sha2::{Digest, Sha256};

pub fn generate_login_code() -> String {
    let mut bytes = [0u8; 20]; // 160 bits
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}
pub fn generate_session_secret() -> String { generate_login_code() } // same entropy source
pub fn hash_secret(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}
```
(Confirm `base64::Engine` import + that `hex` is already a dep — grep `Cargo.toml`; if not, use the existing `sha256_prefixed` helper's hex approach instead. Do NOT add `hex` without checking.)

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit**

```bash
git commit -am "Add CSPRNG login-code and secret hashing to account_session"
```

---

## Task 4: Restricted-role resolver (`trace_login_resolver`)

**This is the single highest-risk item (spec Hardening D / Residual #3). Implement and review carefully.**

**Files:**
- Modify the migration if the role belongs in SQL (`V30`) AND the pool wiring in the DB module (confirm home: search for where the runtime pool + `DatabaseConfig` are built — likely `src/db/postgres.rs` / `src/db/mod.rs`).
- Test: `tests.rs` (PostgreSQL-backed).

- [ ] **Step 1: Failing test**

```rust
#[tokio::test]
async fn login_resolver_role_cannot_touch_other_tables() {
    let Some(backend) = postgres_backend_for_ingest_test().await else { return };
    // Assert the role exists, lacks BYPASSRLS, and has SELECT only on trace_login_links.
    let mut client = backend.raw_pool_for_tests_and_diagnostics().get().await.expect("conn");
    let row = client.query_one(
        "SELECT rolbypassrls FROM pg_roles WHERE rolname = 'trace_login_resolver'", &[]
    ).await.expect("resolver role exists");
    let bypass: bool = row.get(0);
    assert!(!bypass, "resolver role must NOT have BYPASSRLS");
    // No INSERT privilege on trace_login_links:
    let can_insert: bool = client.query_one(
        "SELECT has_table_privilege('trace_login_resolver','trace_login_links','INSERT')", &[]
    ).await.expect("priv check").get(0);
    assert!(!can_insert, "resolver must not write");
}
```

- [ ] **Step 2: Run → FAIL** (role absent).

- [ ] **Step 3: Implement**

**Role provisioning is an operator step, not a migration** (the repo creates no roles in migrations). The operator runbook creates the role with LOGIN + password and a dedicated connection string; the migration only establishes the column-scoped privilege (the single intentional GRANT in the repo). Append to `V30` (after table creation):
```sql
-- Narrow resolver privilege: redeem runs with NO tenant context, so it needs a
-- single-table SELECT to map a globally-unique code_hash -> tenant_id. The role
-- itself is operator-provisioned (NOLOGIN base, no BYPASSRLS, no writes, no other
-- table) and runs on a SEPARATE pool, never the runtime pool. This GRANT is the
-- ONLY GRANT statement in the repo and is deliberate.
DO $$ BEGIN
  IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_login_resolver') THEN
    CREATE ROLE trace_login_resolver NOLOGIN NOBYPASSRLS;
  END IF;
END $$;
GRANT SELECT (tenant_id, account_id, code_hash) ON trace_login_links TO trace_login_resolver;
```
If the deployment forbids `CREATE ROLE` in-migration, drop the `DO $$` block and have the operator create the role first; keep the `GRANT`. Document the LOGIN/password + `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` provisioning in the operator runbook (see Operator follow-ups). Flag the chosen path in the task summary.

In the DB module, add a small separate pool built from a new config field (e.g. `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL`) and:
```rust
/// Resolve tenant for a login code via the narrow resolver pool. Returns the
/// tenant only; the caller re-confirms tenant inside an RLS-scoped tx.
pub async fn resolve_login_link_tenant(&self, code_hash: &str) -> anyhow::Result<Option<String>> {
    let client = self.login_resolver_pool.get().await?;
    let row = client
        .query_opt("SELECT tenant_id FROM trace_login_links WHERE code_hash = $1", &[&code_hash])
        .await?;
    Ok(row.map(|r| r.get::<_, String>(0)))
}
```
Fail-closed: if `login_resolver_pool` is unconfigured while the account feature is enabled, refuse redeem with a safe missing-control name (do not fall back to the runtime pool).

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit**

```bash
git commit -am "Add narrow trace_login_resolver role and separate resolver pool"
```

---

## Task 5: Login-link mint — DB ops + `POST /v1/account/login-links`

**Files:** `account_session.rs` (DB ops as functions taking a tenant-scoped client), `trace-commons-ingest.rs` (handler + route), `tests.rs`.

Reuse: auth via `authenticate_ctx_with_tenant_access_grant` (`trace-commons-ingest.rs:42954`); tenant tx via a new `pub` `PgBackend` method that internally calls `begin_trace_tenant_transaction` (`src/db/trace_corpus_pg.rs:1205`, which is `pub(super)` — do NOT call it cross-module; wrap it on `PgBackend` per the Design boundary note); `device_keys.revoked_at` check (find the existing revoked-device guard and call it).

- [ ] **Step 1: Failing test** — `mint` creates account+principal on first call, reuses on second (idempotent), and stores only the code hash.

```rust
#[tokio::test]
async fn mint_creates_then_reuses_account_and_stores_hash_only() {
    let Some(backend) = postgres_backend_for_ingest_test().await else { return };
    let state = test_state_with_pg(/* helper that injects backend */).await;
    let h = auth_headers("device-token-a");
    let Json(first) = mint_login_link_handler(State(state.clone()), h.clone()).await.expect("mint 1");
    let Json(second) = mint_login_link_handler(State(state.clone()), h).await.expect("mint 2");
    assert_eq!(first.account_id, second.account_id, "same device principal => same account");
    // url contains a code that is NOT what's stored (store holds sha256 only)
    assert!(first.url.contains("/account/login?code="));
}
```
(If no `test_state_with_pg` helper exists, add one beside `test_state` at `tests.rs:10` following `postgres_backend_for_ingest_test`.)

- [ ] **Step 2: Run → FAIL** (`mint_login_link_handler` undefined).

- [ ] **Step 3: Implement**

In `account_session.rs`: `create_or_reuse_account(tx, tenant_id, principal_ref) -> AccountId` doing the spec §Mint step-3 SQL (`SELECT ... WHERE unlinked_at IS NULL`; else `INSERT trace_accounts RETURNING`, then `INSERT trace_account_principals ON CONFLICT (tenant_id, principal_ref) DO NOTHING`, re-select). Add `insert_login_link(tx, tenant_id, account_id, code_hash, created_principal_ref, expires_at)` and the outstanding-link cap query.

In `trace-commons-ingest.rs` (handler beside `credit_handler:11476`):
```rust
async fn mint_login_link_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
) -> ApiResult<Json<MintLoginLinkResponse>> {
    let tenant = authenticate_ctx_with_tenant_access_grant(state.as_ref(), &headers).await?;
    // reject revoked device key (reuse existing guard); enforce outstanding-link cap + rate limit -> 429
    // open tenant tx; create_or_reuse_account; generate code; insert hash + expires_at = now()+5min
    // audit: append_audit_event_with_db_mirror(..., action mint, actor principal only) — NEVER raw code/url
    // return { account_id, url }
}
```
Register at `trace-commons-ingest.rs:5875`:
```rust
.route("/v1/account/login-links", post(mint_login_link_handler))
```

- [ ] **Step 4: Run → PASS** (+ compile gates).

- [ ] **Step 5: Commit** `git commit -am "Add login-link mint endpoint with create-or-reuse account"`

---

## Task 6: Redeem — `GET /account/login` interstitial + `POST /account/login/confirm`

**Files:** `account_session.rs` (cookie build, session insert, atomic consume), `trace-commons-ingest.rs` (two handlers + routes), `tests.rs`.

- [ ] **Step 1: Failing tests** — (a) confirm consumes single-use; (b) second confirm denied; (c) unknown/expired/consumed/wrong-tenant all return the same generic error.

```rust
#[tokio::test]
async fn redeem_is_single_use_and_nonenumerating() {
    let Some(backend) = postgres_backend_for_ingest_test().await else { return };
    let state = test_state_with_pg(/* ... */).await;
    // mint, extract code from url
    let code = mint_and_extract_code(&state, "device-token-a").await;
    let ok = confirm_login_handler(State(state.clone()), same_origin_headers(), form(&code)).await;
    assert!(ok.is_ok(), "first confirm succeeds and Set-Cookie present");
    let again = confirm_login_handler(State(state.clone()), same_origin_headers(), form(&code)).await;
    let unknown = confirm_login_handler(State(state.clone()), same_origin_headers(), form("sha-nope")).await;
    // identical generic error for re-use and unknown:
    assert_eq!(error_status(&again), error_status(&unknown));
}
```

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement**

`GET /account/login` handler: render minimal "Activate" HTML interstitial, **no consumption**, headers `Cache-Control: no-store`, `Referrer-Policy: no-referrer`. IP-rate-limited.

`POST /account/login/confirm`: enforce same-origin (`Origin`/`Sec-Fetch-Site`); rate-limit (IP + global + per-code); apply fixed-latency floor (record start, sleep-to-floor before returning across ALL branches). Then:
1. `resolve_login_link_tenant(hash_secret(code))` → tenant (None → generic deny after floor).
2. `begin_trace_tenant_transaction(tenant)` on runtime pool.
3. Atomic consume (spec Hardening D SQL) with explicit `tenant_id = $resolved` predicate; `rows_affected != 1` → generic deny.
4. `generate_session_secret()`; `INSERT trace_sessions` storing `hash_secret(secret)`, `client_kind='web'`, `expires_at = now()+7d`.
5. Build `Set-Cookie` via `cookie::Cookie::build` with `Secure`, `HttpOnly`, `SameSite::Strict`, `Path=/`; value = raw secret.
6. `303` redirect to a code-free account view URL.
7. Audit redeem (created-vs-reused label; actor principal only).

Routes at `:5875`:
```rust
.route("/account/login", get(login_interstitial_handler))
.route("/account/login/confirm", post(confirm_login_handler))
```

- [ ] **Step 4: Run → PASS** (+ gates).

- [ ] **Step 5: Commit** `git commit -am "Add single-use login redeem with session cookie issuance"`

---

## Task 7: `AccountCtx` resolver — dual-auth (cookie OR device bearer)

**Files:** `account_session.rs` (`resolve_account_ctx`), `trace-commons-ingest.rs` (extraction glue), `tests.rs`.

- [ ] **Step 1: Failing tests** — (a) bearer-only resolves to account + active principal set; (b) cookie-only resolves + sets `principal_ref = account-actor:{id}`, role contributor; (c) BOTH present → `400 ambiguous credentials`; (d) expired/revoked/idle-capped session → `401`.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement**

```rust
pub enum AccountAuthMethod { SessionCookie, DeviceBearer }
pub struct AccountCtx {
    pub account_id: AccountId,
    pub principal_set: AccountPrincipalSet,
    pub auth_method: AccountAuthMethod,
    pub tenant_id: String,
    pub actor_ref: String, // device principal_ref OR account_actor_ref(..)
}
```
`resolve_account_ctx(state, headers)`:
- If both `Authorization: Bearer` and the session cookie are present → `Err(400 ambiguous credentials)`.
- Bearer only → `authenticate_ctx_with_tenant_access_grant` → device `principal_ref`; open tenant tx; resolve `account_id` via active membership; expand principal set; `actor_ref = device principal_ref`.
- Cookie only → parse cookie with `cookie` crate; `hash_secret(value)`; look up session WHERE `token_hash=$h AND expires_at>now() AND revoked_at IS NULL`; enforce idle cap on `last_seen_at` (auto-revoke past cap → `401`), else update `last_seen_at`; resolve `account_id`; expand principal set; `actor_ref = account_actor_ref(account)`; role defaults to low-privilege contributor (never review/admin).
- **Active-membership expansion is the ONLY sanctioned query** (spec Hardening A): `SELECT principal_ref FROM trace_account_principals WHERE tenant_id=trace_current_tenant_id() AND account_id=$a AND unlinked_at IS NULL`.
- Any store/DB error → deny (401/500), never fall through.

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** `git commit -am "Add dual-auth AccountCtx resolver with active-membership expansion"`

---

## Task 8: Account-scoped visibility + compile-fail guarantee

**Files:** `account_session.rs` (`visible_submission_records_for_account`), `crates/trace-commons-server/tests/trace_account_compile_fail.rs`, `tests.rs`.

- [ ] **Step 1: Failing tests**

Runtime: A's principal set returns only A's submissions; legacy-principal submission NOT returned; `account-actor:{id}` matches zero records; unlinked principal → excluded (404 behavior verified in Task 9/10).

```rust
#[test]
fn account_visibility_excludes_legacy_and_others() {
    let set = AccountPrincipalSet::from_iter(["principal_a".to_string()]);
    let recs = vec![
        rec_with_principal("principal_a"),
        rec_with_principal("principal_b"),
        rec_with_principal(&legacy_principal_ref()),
    ];
    let out = visible_submission_records_for_account(&set, recs);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].auth_principal_ref, "principal_a");
}
```

Compile-fail (new file `tests/trace_account_compile_fail.rs`, or a `trybuild` case): assert that an `AccountPrincipalSet` / `AccountCtx` cannot be passed where `&TenantAuth` is expected by `visible_submission_records` / `can_access_submission`. If `trybuild` is not already a dev-dep, do NOT add it — instead encode the guarantee as a documented `// must not compile` example plus a unit test asserting the functions have no `From<AccountPrincipalSet> for TenantAuth` path. (Decide and note which approach; check dev-deps first.)

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement**

```rust
pub fn visible_submission_records_for_account(
    set: &AccountPrincipalSet,
    records: Vec<TraceCommonsSubmissionRecord>,
) -> Vec<TraceCommonsSubmissionRecord> {
    records.into_iter().filter(|r| set.contains(&r.auth_principal_ref)).collect()
}
```
No `can_review()` short-circuit, no `legacy_principal_ref()` wildcard. `TraceCommonsSubmissionRecord` lives in `trace-commons-ingest.rs`; expose the helper so it can borrow the type (either move the type to the lib crate or keep the helper in the binary and unit-test it there — pick the lower-churn option; the type is currently in the binary, so keeping the function in the binary next to `visible_submission_records:43166` and only the newtype in the module is acceptable. Note the chosen split.)

- [ ] **Step 4: Run → PASS** (and confirm the compile-fail case actually fails to compile).

- [ ] **Step 5: Commit** `git commit -am "Add account-scoped visibility predicate with type-level surface separation"`

---

## Task 9: `GET /v1/account/traces` (list + keyset pagination) and `GET /v1/account/traces/{id}` (detail)

**Files:** `trace-commons-ingest.rs` (handlers + routes), `account_session.rs` (keyset query helper), `tests.rs`.

- [ ] **Step 1: Failing tests** — list returns only the account's submissions, ordered `(created_at DESC, submission_id DESC)`; cursor round-trips; cross-account request yields empty/404; detail returns metadata for owned id, **uniform 404** for non-owned or nonexistent id (no existence oracle).

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement**

List query filters `auth_principal_ref = ANY($principal_set)` ordered `(created_at DESC, submission_id DESC)`; cursor = base64 of `(created_at, submission_id)`; capped default + max `limit`; no offset. Confirm/add the supporting index `(tenant_id, created_at DESC, submission_id)` on the submissions table — check whether it already exists before adding a migration; if needed, add to a follow-up migration `V31` (do NOT retro-edit V30 once committed). Detail: fetch by id, check membership via `principal_set.contains`, else `404`. Audit: `append_control_plane_read_audit(..., "account_traces_list", item_count)` and `"account_trace_detail", 1`.

Routes at `:5875`:
```rust
.route("/v1/account/traces", get(account_traces_list_handler))
.route("/v1/account/traces/{submission_id}", get(account_trace_detail_handler))
```
(axum 0.8 path param syntax is `{submission_id}`.)

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** `git commit -am "Add account trace list and detail read endpoints"`

---

## Task 10: `GET /v1/account/traces/{id}/content` (sealed content read-back)

**Files:** `trace-commons-ingest.rs` (handler + route), `tests.rs`.

Reuse `read_envelope_by_record` (`trace-commons-ingest.rs:47840`) / `read_envelope_from_object_ref` (`:48233`) — NO new decryption path. Audit via `append_trace_content_read_audit_per_source` (`:50768`).

- [ ] **Step 1: Failing tests** — (a) owned id returns scrubbed envelope JSON and `[REDACTED]` survives round-trip; (b) non-owned/nonexistent → uniform `404`; (c) simulated KMS/decrypt failure → generic label-only `500`, no ciphertext/plaintext/object-key leak, audit row written; (d) over max-bytes ceiling → generic error.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement**

```rust
async fn account_trace_content_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    Path(submission_id): Path<Uuid>,
) -> ApiResult<Response> {
    let ctx = resolve_account_ctx(state.as_ref(), &headers).await?;
    // load record; if !ctx.principal_set.contains(&record.auth_principal_ref) -> 404 (uniform)
    // enforce per-account rate limit + concurrency cap; max-bytes ceiling
    // read_envelope_by_record(...) -> on any err: generic label-only 500 + content-read audit (failure) ; NEVER echo internals
    // on ok: audit success; return JSON with headers:
    //   Content-Type: application/json; charset=utf-8, Cache-Control: no-store,
    //   X-Content-Type-Options: nosniff ; same-origin enforced
}
```
Route at `:5875`:
```rust
.route("/v1/account/traces/{submission_id}/content", get(account_trace_content_handler))
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** `git commit -am "Add account trace content read-back with fail-closed decryption"`

---

## Task 11: Hardening — rate limits, timing floor, logout + revoke-all

**Files:** `account_session.rs`, `trace-commons-ingest.rs`, `tests.rs`.

- [ ] **Step 1: Failing tests** — (a) redeem confirm enforces a minimum latency floor across found/not-found (assert elapsed ≥ floor for both); (b) per-IP + per-code limits return `429` after threshold; (c) logout sets `revoked_at` and subsequent cookie use → `401`; (d) `revoke_all_sessions(account)` invalidates every session.

- [ ] **Step 2: Run → FAIL.**

- [ ] **Step 3: Implement** the rate-limit keying (reuse any existing limiter in the codebase — grep for current rate-limit middleware before writing a new one), the sleep-to-floor wrapper on confirm, and:
```rust
.route("/v1/account/logout", post(account_logout_handler))      // sets revoked_at for current session
.route("/v1/account/sessions/revoke-all", post(account_revoke_all_handler))
```

- [ ] **Step 4: Run → PASS.**

- [ ] **Step 5: Commit** `git commit -am "Add rate limiting, timing floor, and session revocation"`

---

## Task 12: Cross-account isolation regression suite + full-gate sweep

**Files:** `tests.rs`.

- [ ] **Step 1: Write the regression suite** (spec §Testing — all five must pass):
  - (a) two accounts in one tenant — A's session 404/empty on B's submission (list + detail + content);
  - (b) legacy-principal submission never returned on the account surface;
  - (c) a session whose underlying token role is reviewer is STILL confined to its own account on `/v1/account/*`;
  - (d) unlinked-principal submission returns 404 (insert a `trace_account_principals` row with `unlinked_at` set, assert excluded from the expanded set AND from all three read endpoints);
  - (e) `account-actor:{id}` used directly as an ownership filter matches zero records.

- [ ] **Step 2: Run → expect some FAIL if any gap remains; fix until green.**

- [ ] **Step 3: Full verification sweep**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
# With a DB:
TRACE_COMMONS_PG_TEST_DATABASE_URL=... cargo test -p trace-commons-server
scripts/operator/pilot-bootstrap-smoke.sh   # must still pass
```
Expected: all green (DB tests pass with a DB; otherwise note skips).

- [ ] **Step 4: Commit** `git commit -am "Add cross-account isolation regression suite for account read surface"`

---

## Operator follow-ups (out of code scope, note in PR)

- Provision the `trace_login_resolver` role's LOGIN/password + `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` on its own small pool, separate from the runtime pool (spec Residual #3 — highest-priority review item).
- Document the account feature env flags and the login-link TTL / session TTL / idle-cap values in the operator runbook.

## Done criteria

All 12 tasks committed; full verification sweep green; the five isolation regressions and the compile-fail guarantee pass; no raw codes/secrets/URLs/identity/object-keys/ARNs/ciphertext in any audit row or log; redeem performs no account creation; resolver role has no BYPASSRLS and no writes.
