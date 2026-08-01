# DB-Authoritative Invites Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make PostgreSQL the source of truth for contributor invites, with an authenticated admin API on the upload-claim issuer for their lifecycle, replacing the operator-edited allowlist file.

**Architecture:** A new tenant-less `onboarding_invite_grants` table (V42) governed by two RLS policies: a GUC predicate (`trace_current_invite_subject()`) for the in-transaction redemption re-check on the runtime pool, and a permissive `TO trace_invite_registry` policy on a separate narrow pool for cache refresh and admin CRUD. A `DbInviteRegistry` caches live invites in-process and is invalidated synchronously on write, so a minted code is redeemable in the same instant. The existing `AllowlistSnapshot` is left alone and keeps serving TEE instance entries.

**Tech Stack:** Rust, axum, deadpool-postgres, tokio-postgres, jsonwebtoken (EdDSA), rand 0.8, PostgreSQL 15+ with FORCE ROW LEVEL SECURITY.

Spec: `docs/superpowers/specs/2026-08-01-db-authoritative-invites-design.md`

## Global Constraints

- **No new dependencies.** `rand = "0.8"`, `ring = "0.17"`, `jsonwebtoken = "9"`, `chrono`, `deadpool-postgres 0.14`, and `uuid` are already present and are the only crates this plan needs. Adding any direct dependency requires explicit approval first.
- **PostgreSQL only.** No libsql feature flags, no dual-backend testing. A single `cargo check -p trace-commons-server` covers the backend surface.
- **Verify with warnings-as-errors.** CI applies `RUSTFLAGS=-D warnings`; plain `cargo check` does not. Every verification step in this plan uses the `RUSTFLAGS` form. Never claim green without it.
- **Clippy allow-list is fixed.** Run `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen it.
- **Hash-only audit and logging.** No raw invite codes, contributor identity, credential values, tenant secrets, or connection strings in any stored row, log line, error string, or admin response. `note_label` and `issued_by_label` are operator free text and must never be returned to non-admin callers.
- **Fail-closed.** When a required gate is configured but its dependency is missing, refuse with a named missing-control label. Never fall back to the file once authoritative mode is on.
- **No emojis** in commits, PRs, code, or docs. Commit subjects are short and imperative, with no `feat:` / `fix:` prefix.
- **Migration numbering.** V42 is the next free number. Numbers 30-34 are already applied to the shared `trace_commons_test` database; do not reuse them. `run_migrations` in `db/postgres.rs` is hand-rolled — every new migration must be wired into it explicitly or it will never run.
- **Work happens in the worktree** `.worktrees/db-authoritative-invites` on branch `db-authoritative-invites`. Use paths relative to that worktree root; do not write into the main checkout.

### Deviation from the spec, already decided

The spec listed two new database URLs (`TRACE_COMMONS_INVITE_REGISTRY_DB_URL` and `TRACE_COMMONS_INVITE_ADMIN_DB_URL`). This plan uses **one**: `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL`, with the role named `trace_invite_registry` rather than `trace_invite_admin`.

Reason: the registry's cache refresh needs to read all live invites, which is the same cross-invite visibility the admin API needs, so they share one narrow pool. The runtime pool still performs the authoritative in-transaction re-check under the GUC predicate, so every security property in the spec is preserved. The consequence to document: redemption requires the registry pool to be configured, and its absence yields `InviteRegistryNotConfigured`.

## File Structure

**Created:**

| File | Responsibility |
|---|---|
| `migrations/V42__onboarding_invite_grants.sql` | Table, constraints, indexes, `trace_current_invite_subject()`, both RLS policies, `trace_invite_registry` role |
| `crates/trace-commons-server/src/trace_invite_registry.rs` | `InviteEntry`, `InviteTenantMode`, `InviteRegistry` trait, `DbInviteRegistry` cache + refresh + invalidation, `generate_invite_code` |
| `crates/trace-commons-server/src/trace_invite_admin.rs` | Admin JWT verification and the four invite admin routes |
| `crates/trace-commons-server/tests/trace_invite_registry_pg.rs` | Postgres-backed store and RLS tests exercised under `SET ROLE` |

**Modified:**

| File | Change |
|---|---|
| `crates/trace-commons-server/src/lib.rs` | Declare the two new modules |
| `crates/trace-commons-server/src/config.rs` | `invite_registry_url` field, env reader, accessor |
| `crates/trace-commons-server/src/db/mod.rs` | `InviteGrantRecord`, `InviteGrantWrite`, `InviteGrantInsertOutcome`, store method declarations |
| `crates/trace-commons-server/src/db/postgres.rs` | `invite_registry_pool`, V42 in `run_migrations`, table in `TRACE_COMMONS_RLS_TABLES`, four store methods, redemption re-check |
| `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` | Registry in issuer state, redemption through the registry, both tenant modes |
| `crates/trace-commons-server/src/trace_upload_claim_allowlist.rs` | Reject `kind: "invite"` entries when authoritative mode is on |
| `crates/trace-commons-protocol/src/onboarding.rs` | Three new `TraceOnboardErrorCode` variants |
| `crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs` | `--import-file-invites`, `--mint-invites` subcommands |
| `docs/operator/pilot-allowlist.md` | Rewrite the provisioning section; add role provisioning and cutover |

`trace_upload_claim_issuer.rs` is already 5.7k lines. Registry and admin logic go in the two new modules rather than growing it further; only the redemption call site and state field change there.

---

### Task 1: Migration V42 — schema, roles, RLS

**Files:**
- Create: `migrations/V42__onboarding_invite_grants.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (`TRACE_COMMONS_RLS_TABLES` near line 143; `run_migrations`, after the V41 block)
- Test: `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: table `onboarding_invite_grants`; function `trace_current_invite_subject() RETURNS TEXT`; role `trace_invite_registry`; policies `invite_lookup` and `trace_invite_registry_all`.

- [ ] **Step 1: Write the migration**

Create `migrations/V42__onboarding_invite_grants.sql`:

```sql
-- V42: DB-authoritative contributor invites.
--
-- Deliberately tenant-less: an invite has no tenant until it is redeemed, and
-- lookup is by invite hash alone. V29 `onboarding_invites` is untouched and
-- keeps counting redemptions per tenant. This table answers "may this code be
-- redeemed"; V29 answers "how many times has it been redeemed under this
-- tenant".
--
-- Hash-only: no raw invite codes, no contributor identity, no credential
-- values. `note_label` and `issued_by_label` are operator free text and are
-- never returned to non-admin callers.

CREATE TABLE IF NOT EXISTS onboarding_invite_grants (
    invite_subject_hash     TEXT PRIMARY KEY
        CHECK (invite_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    policy_label            TEXT NOT NULL,
    tenant_mode             TEXT NOT NULL CHECK (tenant_mode IN ('fixed', 'derived')),
    fixed_tenant_id         TEXT,
    tenant_template_id      TEXT,
    policy_version          TEXT NOT NULL,
    allowed_consent_scopes  TEXT[] NOT NULL DEFAULT '{}',
    allowed_uses            TEXT[] NOT NULL DEFAULT '{}',
    max_uses                INTEGER NOT NULL DEFAULT 3 CHECK (max_uses > 0),
    expires_at              TIMESTAMPTZ,
    issuance_source         TEXT NOT NULL,
    issued_by_label         TEXT,
    credential_binding_hash TEXT
        CHECK (credential_binding_hash IS NULL
               OR credential_binding_hash ~ '^sha256:[0-9a-f]{64}$'),
    note_label              TEXT,
    revoked_at              TIMESTAMPTZ,
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    -- Enforce the tenant_mode / tenant-column pairing in BOTH directions so
    -- neither column can be populated for the wrong mode.
    CONSTRAINT onboarding_invite_grants_tenant_mode_pairing CHECK (
        (tenant_mode = 'fixed'
            AND fixed_tenant_id IS NOT NULL
            AND tenant_template_id IS NULL)
        OR
        (tenant_mode = 'derived'
            AND tenant_template_id IS NOT NULL
            AND fixed_tenant_id IS NULL)
    )
);

-- One verified credential yields at most one live invite per pool. Revoking
-- frees the binding for reissue.
CREATE UNIQUE INDEX IF NOT EXISTS idx_onboarding_invite_grants_credential
    ON onboarding_invite_grants (policy_label, credential_binding_hash)
    WHERE credential_binding_hash IS NOT NULL AND revoked_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_onboarding_invite_grants_live
    ON onboarding_invite_grants (policy_label, created_at DESC)
    WHERE revoked_at IS NULL;

-- Redemption-path predicate. The issuer sets this GUC to the hash of the code
-- the caller actually presented, transaction-locally. A lookup can therefore
-- only ever return the row for a code the caller already knows: the hot path
-- cannot enumerate live invites. Mirrors V35's trace_current_instance_subject().
CREATE OR REPLACE FUNCTION trace_current_invite_subject()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT NULLIF(current_setting('trace_commons.invite_subject', true), '');
$$;

ALTER TABLE onboarding_invite_grants ENABLE ROW LEVEL SECURITY;
ALTER TABLE onboarding_invite_grants FORCE ROW LEVEL SECURITY;

DROP POLICY IF EXISTS invite_lookup ON onboarding_invite_grants;
CREATE POLICY invite_lookup ON onboarding_invite_grants
    FOR SELECT
    USING (invite_subject_hash = trace_current_invite_subject());

-- Cross-invite reader/writer role for the registry cache refresh and the admin
-- API. NOBYPASSRLS is load-bearing: the permissive policy below is what
-- authorizes the role, not a bypass, so the runtime/PUBLIC role stays confined
-- to the GUC predicate above. Mirrors trace_gate_driver (V36).
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_invite_registry') THEN
        CREATE ROLE trace_invite_registry NOLOGIN NOBYPASSRLS;
    END IF;
END $$;
ALTER ROLE trace_invite_registry SET statement_timeout = '5s';

GRANT SELECT, INSERT, UPDATE ON onboarding_invite_grants TO trace_invite_registry;

DROP POLICY IF EXISTS trace_invite_registry_all ON onboarding_invite_grants;
CREATE POLICY trace_invite_registry_all ON onboarding_invite_grants
    FOR ALL TO trace_invite_registry
    USING (true) WITH CHECK (true);
```

Note the deliberate absence of `DELETE` in the GRANT: revocation is a soft `UPDATE` of `revoked_at`, and invite history is audit-relevant.

- [ ] **Step 2: Wire V42 into `run_migrations` and the RLS table list**

In `crates/trace-commons-server/src/db/postgres.rs`, append `"onboarding_invite_grants"` to `TRACE_COMMONS_RLS_TABLES`, then add this block after the existing V41 block in `run_migrations`, following the identical shape:

```rust
let already_applied = client
    .query_opt(
        "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
        &[&42_i32],
    )
    .await?
    .is_some();
if !already_applied {
    client
        .batch_execute(include_str!(
            "../../../../migrations/V42__onboarding_invite_grants.sql"
        ))
        .await?;
    client
        .execute(
            "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
            &[&42_i32, &"onboarding_invite_grants"],
        )
        .await?;
}
```

- [ ] **Step 3: Write the failing RLS test**

Create `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`. These tests must run under `SET ROLE`, never on a bare superuser connection — a superuser connection silently satisfies every policy and hides exactly this class of bug, which is how a resolver policy gap was missed previously.

```rust
//! Postgres-backed tests for the V42 invite-grant table.
//!
//! Skipped unless TRACE_COMMONS_PG_TEST_DATABASE_URL (or DATABASE_URL) is set.
//! CI does not run these; run them locally against a real PostgreSQL.

use secrecy::SecretString;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};

// DatabaseConfig has no Default impl and secrecy 0.10 uses From, not new.
// This mirrors postgres_test_config() in tests/trace_corpus_pg_store.rs
// exactly; keep the two in step when fields are added.
fn postgres_test_config() -> Option<DatabaseConfig> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    Some(DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: DatabaseConfig::login_resolver_url_from_env(),
        gate_driver_url: DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url: DatabaseConfig::pii_backstop_driver_url_from_env(),
        // Task 3 adds `invite_registry_url` to DatabaseConfig. This literal is
        // exhaustive, so Task 3 Step 1 must add the field here too or this
        // file stops compiling. That is expected and is called out there.
    })
}

const TEST_HASH_A: &str =
    "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TEST_HASH_B: &str =
    "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

/// Insert two rows, then read them back as a NON-superuser role with the
/// invite_lookup policy in force. Without the GUC set the role must see
/// nothing; with it set the role must see exactly the matching row.
#[tokio::test]
async fn invite_lookup_policy_confines_reads_to_the_presented_hash() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");

    // Seed as the owner, outside any policy-constrained role.
    for hash in [TEST_HASH_A, TEST_HASH_B] {
        client
            .execute(
                "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, 'test-pool', 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
                &[&hash],
            )
            .await
            .expect("seed");
    }

    let tx = client.transaction().await.expect("tx");
    // A NOBYPASSRLS role: policies actually apply. Superuser would not.
    tx.batch_execute("SET LOCAL ROLE trace_invite_registry_test_reader")
        .await
        .expect("set role");

    // No GUC set: the invite_lookup predicate matches nothing.
    let rows = tx
        .query("SELECT invite_subject_hash FROM onboarding_invite_grants", &[])
        .await
        .expect("query");
    assert_eq!(
        rows.len(),
        0,
        "runtime role must see no invites without trace_commons.invite_subject set"
    );

    // GUC set to A: exactly one row, and it is A.
    tx.execute(
        "SELECT set_config('trace_commons.invite_subject', $1, true)",
        &[&TEST_HASH_A],
    )
    .await
    .expect("set guc");
    let rows = tx
        .query("SELECT invite_subject_hash FROM onboarding_invite_grants", &[])
        .await
        .expect("query");
    assert_eq!(rows.len(), 1, "exactly the presented invite is visible");
    assert_eq!(rows[0].get::<_, String>(0), TEST_HASH_A);

    tx.rollback().await.expect("rollback");
}

/// The registry role's permissive policy must expose every row, so cache
/// refresh and admin listing work.
#[tokio::test]
async fn registry_role_sees_all_invites() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let pool = backend.trace_pool_for_test();
    let mut client = pool.get().await.expect("client");
    for hash in [TEST_HASH_A, TEST_HASH_B] {
        client
            .execute(
                "INSERT INTO onboarding_invite_grants (
                     invite_subject_hash, policy_label, tenant_mode,
                     tenant_template_id, policy_version, issuance_source
                 ) VALUES ($1, 'test-pool', 'derived', 'tmpl-1', 'v1', 'operator')
                 ON CONFLICT (invite_subject_hash) DO NOTHING",
                &[&hash],
            )
            .await
            .expect("seed");
    }

    let tx = client.transaction().await.expect("tx");
    tx.batch_execute("SET LOCAL ROLE trace_invite_registry")
        .await
        .expect("set role");
    let rows = tx
        .query(
            "SELECT invite_subject_hash FROM onboarding_invite_grants
              WHERE invite_subject_hash IN ($1, $2)",
            &[&TEST_HASH_A, &TEST_HASH_B],
        )
        .await
        .expect("query");
    assert_eq!(rows.len(), 2, "registry role must see all invites");
    tx.rollback().await.expect("rollback");
}

/// The tenant_mode pairing constraint must reject every wrong combination.
#[tokio::test]
async fn tenant_mode_pairing_constraint_rejects_mismatches() {
    let Some(config) = postgres_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");

    // fixed mode with no fixed_tenant_id
    let err = client
        .execute(
            "INSERT INTO onboarding_invite_grants (
                 invite_subject_hash, policy_label, tenant_mode,
                 policy_version, issuance_source
             ) VALUES ($1, 'p', 'fixed', 'v1', 'operator')",
            &[&TEST_HASH_A],
        )
        .await;
    assert!(err.is_err(), "fixed mode requires fixed_tenant_id");

    // derived mode carrying a fixed_tenant_id
    let err = client
        .execute(
            "INSERT INTO onboarding_invite_grants (
                 invite_subject_hash, policy_label, tenant_mode,
                 tenant_template_id, fixed_tenant_id, policy_version, issuance_source
             ) VALUES ($1, 'p', 'derived', 'tmpl', 'tenant-x', 'v1', 'operator')",
            &[&TEST_HASH_B],
        )
        .await;
    assert!(err.is_err(), "derived mode must not carry fixed_tenant_id");
}
```

The first test needs a NOBYPASSRLS role that is *not* `trace_invite_registry` (so the permissive policy does not apply) to stand in for the runtime role. Add this to the test-database setup documented in Step 5, and add `PgBackend::trace_pool_for_test()` as a `#[doc(hidden)]` accessor beside the existing `trace_pool()` in `postgres.rs`:

```rust
#[doc(hidden)]
pub fn trace_pool_for_test(&self) -> Pool {
    self.pool.clone()
}
```

- [ ] **Step 4: Run the tests to verify they fail**

```bash
cd .worktrees/db-authoritative-invites
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: compile failure on `trace_pool_for_test` not existing, or — once that compiles — failures because `onboarding_invite_grants` does not exist yet. Either is a valid red.

- [ ] **Step 5: Provision the test role, then run the tests green**

The stand-in runtime role must exist in the test database once:

```bash
psql "$TRACE_COMMONS_PG_TEST_DATABASE_URL" -c \
  "DO \$\$ BEGIN
     IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_invite_registry_test_reader') THEN
       CREATE ROLE trace_invite_registry_test_reader NOLOGIN NOBYPASSRLS;
     END IF;
   END \$\$;
   GRANT SELECT ON onboarding_invite_grants TO trace_invite_registry_test_reader;"
```

Run that only after the migration has been applied once by the first test run. Then:

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: all three tests PASS. If `invite_lookup_policy_confines_reads_to_the_presented_hash` passes while returning rows without the GUC, the policy is wrong — do not proceed.

- [ ] **Step 6: Verify the build under CI flags**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
```

Expected: both succeed with no warnings.

- [ ] **Step 7: Commit**

```bash
git add migrations/V42__onboarding_invite_grants.sql \
        crates/trace-commons-server/src/db/postgres.rs \
        crates/trace-commons-server/tests/trace_invite_registry_pg.rs
git commit -m "Add tenant-less invite-grant table with GUC-scoped lookup policy

The redemption path reads through trace_current_invite_subject(), so it can
only ever see the invite whose code was presented. Cross-invite visibility is
confined to a separate NOBYPASSRLS trace_invite_registry role."
```

---

### Task 2: `InviteEntry` and the registry cache

**Files:**
- Create: `crates/trace-commons-server/src/trace_invite_registry.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks. Uses `chrono::{DateTime, Utc}` and `rand 0.8`.
- Produces:
  - `pub enum InviteTenantMode { Fixed, Derived }`
  - `pub struct InviteEntry` with public fields `invite_subject_hash: String`, `policy_label: String`, `tenant_mode: InviteTenantMode`, `fixed_tenant_id: Option<String>`, `tenant_template_id: Option<String>`, `policy_version: String`, `allowed_consent_scopes: Vec<String>`, `allowed_uses: Vec<String>`, `max_uses: u32`, `expires_at: Option<DateTime<Utc>>`, `issuance_source: String`, `issued_by_label: Option<String>`, `credential_binding_hash: Option<String>`, `note_label: Option<String>`, `revoked_at: Option<DateTime<Utc>>`
  - `pub enum InviteRegistryError { Stale { age_seconds: u64 }, Backend(String) }`
  - `pub struct InviteRegistryStatus { live: usize, cache_age_seconds: u64, stale: bool, max_stale_seconds: u64 }`
  - `pub trait InviteRegistry: Send + Sync` with `lookup`, `note_write`, `note_revoke`, `status`
  - `pub struct InviteCache` with `replace_all`, `lookup`, `note_write`, `note_revoke`, `status`
  - `pub fn generate_invite_code() -> String`

This task is pure in-memory logic with no database, so it is fully unit-testable.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-server/src/trace_invite_registry.rs` containing only the test module for now:

```rust
//! In-process invite registry: cache, invalidation, and code generation.
//!
//! The cache is a latency optimization and never a correctness boundary.
//! Expiry, revocation, and use-count are re-checked inside the redemption
//! transaction, so a revoke racing a redemption is resolved by the database.

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration as ChronoDuration, Utc};
    use std::time::{Duration, Instant};

    fn entry(hash: &str) -> InviteEntry {
        InviteEntry {
            invite_subject_hash: hash.to_string(),
            policy_label: "test-pool".to_string(),
            tenant_mode: InviteTenantMode::Derived,
            fixed_tenant_id: None,
            tenant_template_id: Some("tmpl-1".to_string()),
            policy_version: "v1".to_string(),
            allowed_consent_scopes: vec!["model_training".to_string()],
            allowed_uses: vec!["research".to_string()],
            max_uses: 3,
            expires_at: None,
            issuance_source: "operator".to_string(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: None,
            revoked_at: None,
        }
    }

    #[test]
    fn note_write_makes_an_invite_immediately_visible() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(Vec::new(), Instant::now());
        assert!(cache.lookup("sha256:aa").unwrap().is_none());

        cache.note_write(entry("sha256:aa"));

        let found = cache.lookup("sha256:aa").unwrap();
        assert!(
            found.is_some(),
            "a minted invite must be redeemable with no refresh window"
        );
    }

    #[test]
    fn note_revoke_immediately_removes_an_invite() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(vec![entry("sha256:aa")], Instant::now());
        assert!(cache.lookup("sha256:aa").unwrap().is_some());

        cache.note_revoke("sha256:aa");

        assert!(cache.lookup("sha256:aa").unwrap().is_none());
    }

    #[test]
    fn expired_invites_are_not_returned() {
        let cache = InviteCache::new(Duration::from_secs(60));
        let mut expired = entry("sha256:aa");
        expired.expires_at = Some(Utc::now() - ChronoDuration::seconds(1));
        cache.replace_all(vec![expired], Instant::now());

        assert!(cache.lookup("sha256:aa").unwrap().is_none());
    }

    #[test]
    fn lookup_fails_closed_once_the_cache_is_stale() {
        let cache = InviteCache::new(Duration::from_secs(60));
        // Loaded 61 seconds ago: past max_stale.
        cache.replace_all(
            vec![entry("sha256:aa")],
            Instant::now() - Duration::from_secs(61),
        );

        match cache.lookup("sha256:aa") {
            Err(InviteRegistryError::Stale { age_seconds }) => {
                assert!(age_seconds >= 61);
            }
            other => panic!("stale cache must fail closed, got {other:?}"),
        }
    }

    #[test]
    fn staleness_boundary_matches_the_allowlist_source() {
        // FileAllowlistSource treats `age > max_stale` as stale, so exactly
        // max_stale is still fresh. Keep the two in agreement.
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(
            vec![entry("sha256:aa")],
            Instant::now() - Duration::from_secs(59),
        );
        assert!(cache.lookup("sha256:aa").is_ok());
    }

    #[test]
    fn status_reports_live_count_and_staleness() {
        let cache = InviteCache::new(Duration::from_secs(60));
        cache.replace_all(vec![entry("sha256:aa"), entry("sha256:bb")], Instant::now());
        let status = cache.status();
        assert_eq!(status.live, 2);
        assert!(!status.stale);
        assert_eq!(status.max_stale_seconds, 60);
    }

    #[test]
    fn generated_codes_use_the_runbook_alphabet_and_length() {
        let code = generate_invite_code();
        assert_eq!(code.len(), 16);
        assert!(
            code.chars().all(|c| c.is_ascii_uppercase() || ('2'..='9').contains(&c)),
            "codes must stay within [A-Z2-9]: {code}"
        );
    }

    #[test]
    fn generated_codes_do_not_repeat() {
        let a = generate_invite_code();
        let b = generate_invite_code();
        assert_ne!(a, b);
    }
}
```

Declare the module in `crates/trace-commons-server/src/lib.rs` beside the existing `pub mod trace_upload_claim_allowlist;`:

```rust
pub mod trace_invite_registry;
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --lib trace_invite_registry
```

Expected: compile errors — `InviteEntry`, `InviteCache`, `InviteRegistryError`, `generate_invite_code` are not defined.

- [ ] **Step 3: Write the implementation**

Prepend to `crates/trace-commons-server/src/trace_invite_registry.rs`, above the test module:

```rust
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use rand::Rng;
use rand::rngs::OsRng;

/// Unambiguous invite-code alphabet. Matches the operator runbook's
/// `tr -dc 'A-Z2-9'`: no 0/O, no 1/I/L.
const INVITE_CODE_ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ23456789";
const INVITE_CODE_LEN: usize = 16;

/// How the redeemed tenant is resolved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteTenantMode {
    /// Use `fixed_tenant_id` verbatim. Imported pilot invites.
    Fixed,
    /// Derive from `tenant_template_id` plus the redeeming user subject.
    Derived,
}

#[derive(Debug, Clone)]
pub struct InviteEntry {
    pub invite_subject_hash: String,
    pub policy_label: String,
    pub tenant_mode: InviteTenantMode,
    pub fixed_tenant_id: Option<String>,
    pub tenant_template_id: Option<String>,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub max_uses: u32,
    pub expires_at: Option<DateTime<Utc>>,
    pub issuance_source: String,
    pub issued_by_label: Option<String>,
    pub credential_binding_hash: Option<String>,
    pub note_label: Option<String>,
    pub revoked_at: Option<DateTime<Utc>>,
}

impl InviteEntry {
    /// Live means neither revoked nor past expiry. Checked in the cache for
    /// latency and re-checked in the redemption transaction for correctness.
    pub fn is_live(&self, now: DateTime<Utc>) -> bool {
        self.revoked_at.is_none() && self.expires_at.is_none_or(|exp| exp > now)
    }
}

#[derive(Debug)]
pub enum InviteRegistryError {
    /// Cache is older than `max_stale` and has not reloaded. Fail closed.
    Stale { age_seconds: u64 },
    Backend(String),
}

impl std::fmt::Display for InviteRegistryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stale { age_seconds } => {
                write!(f, "invite registry cache stale ({age_seconds}s)")
            }
            // Never interpolate a connection string or row content here.
            Self::Backend(label) => write!(f, "invite registry backend error: {label}"),
        }
    }
}

impl std::error::Error for InviteRegistryError {}

#[derive(Debug, Clone)]
pub struct InviteRegistryStatus {
    pub live: usize,
    pub cache_age_seconds: u64,
    pub stale: bool,
    pub max_stale_seconds: u64,
}

pub trait InviteRegistry: Send + Sync {
    fn lookup(&self, invite_subject_hash: &str)
    -> Result<Option<InviteEntry>, InviteRegistryError>;
    fn note_write(&self, entry: InviteEntry);
    fn note_revoke(&self, invite_subject_hash: &str);
    fn status(&self) -> InviteRegistryStatus;
}

struct CacheInner {
    entries: HashMap<String, InviteEntry>,
    loaded_at: Instant,
}

/// Process-local cache of live invites. Restart-resets, exactly like the
/// allowlist `DenialCounter`; the database is the durable backstop.
pub struct InviteCache {
    inner: RwLock<CacheInner>,
    max_stale: Duration,
}

impl InviteCache {
    pub fn new(max_stale: Duration) -> Self {
        Self {
            inner: RwLock::new(CacheInner {
                entries: HashMap::new(),
                // Treat a never-loaded cache as maximally stale so lookups
                // fail closed until the first refresh succeeds.
                loaded_at: Instant::now() - max_stale - Duration::from_secs(1),
            }),
            max_stale,
        }
    }

    /// Install a fresh snapshot from the registry pool.
    pub fn replace_all(&self, entries: Vec<InviteEntry>, loaded_at: Instant) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner.entries = entries
            .into_iter()
            .map(|e| (e.invite_subject_hash.clone(), e))
            .collect();
        inner.loaded_at = loaded_at;
    }

    pub fn lookup(
        &self,
        invite_subject_hash: &str,
    ) -> Result<Option<InviteEntry>, InviteRegistryError> {
        let inner = self.inner.read().expect("InviteCache poisoned");
        let age = inner.loaded_at.elapsed();
        // `>` not `>=`, matching FileAllowlistSource's staleness comparison
        // exactly so the two surfaces agree at sub-second precision.
        if age > self.max_stale {
            return Err(InviteRegistryError::Stale {
                age_seconds: age.as_secs(),
            });
        }
        let now = Utc::now();
        Ok(inner
            .entries
            .get(invite_subject_hash)
            .filter(|e| e.is_live(now))
            .cloned())
    }

    /// Make a freshly minted invite visible with no refresh window. Called
    /// immediately after the admin write commits.
    pub fn note_write(&self, entry: InviteEntry) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner.entries.insert(entry.invite_subject_hash.clone(), entry);
    }

    pub fn note_revoke(&self, invite_subject_hash: &str) {
        let mut inner = self.inner.write().expect("InviteCache poisoned");
        inner.entries.remove(invite_subject_hash);
    }

    pub fn status(&self) -> InviteRegistryStatus {
        let inner = self.inner.read().expect("InviteCache poisoned");
        let age = inner.loaded_at.elapsed();
        let now = Utc::now();
        InviteRegistryStatus {
            live: inner.entries.values().filter(|e| e.is_live(now)).count(),
            cache_age_seconds: age.as_secs(),
            stale: age > self.max_stale,
            max_stale_seconds: self.max_stale.as_secs(),
        }
    }
}

/// Generate an invite code from the OS CSPRNG. The raw code exists in exactly
/// one admin response body and is never stored, logged, or retrievable
/// afterward; only `hash_invite_code()` of it reaches the database.
pub fn generate_invite_code() -> String {
    let mut rng = OsRng;
    (0..INVITE_CODE_LEN)
        .map(|_| {
            let idx = rng.gen_range(0..INVITE_CODE_ALPHABET.len());
            INVITE_CODE_ALPHABET[idx] as char
        })
        .collect()
}
```

If the toolchain rejects `Option::is_none_or`, replace that call with `self.expires_at.map_or(true, |exp| exp > now)`.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --lib trace_invite_registry
```

Expected: all 8 tests PASS.

- [ ] **Step 5: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: clean.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_invite_registry.rs \
        crates/trace-commons-server/src/lib.rs
git commit -m "Add in-process invite cache with write-through invalidation

note_write makes a minted invite redeemable with no refresh window, which is
what self-serve issuance needs. Staleness uses the same > comparison as
FileAllowlistSource so both surfaces agree."
```

---

### Task 3: Registry pool and store methods

**Files:**
- Modify: `crates/trace-commons-server/src/config.rs`
- Modify: `crates/trace-commons-server/src/db/mod.rs`
- Modify: `crates/trace-commons-server/src/db/postgres.rs`
- Test: `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`

**Interfaces:**
- Consumes: `InviteEntry`, `InviteTenantMode` from Task 2; the V42 table from Task 1.
- Produces, on `PgBackend`:
  - `pub async fn lookup_invite_grant_in_tx(tx: &Transaction<'_>, invite_subject_hash: &str) -> Result<Option<InviteEntry>, DatabaseError>` — runtime pool, sets the GUC
  - `pub async fn list_invite_grants(&self) -> Result<Vec<InviteEntry>, DatabaseError>` — registry pool
  - `pub async fn insert_invite_grant(&self, write: InviteGrantWrite) -> Result<InviteGrantInsertOutcome, DatabaseError>` — registry pool
  - `pub async fn revoke_invite_grant(&self, invite_subject_hash: &str) -> Result<bool, DatabaseError>` — registry pool
- Also produces `pub struct InviteGrantWrite` (same fields as `InviteEntry` minus `revoked_at`) and `pub enum InviteGrantInsertOutcome { Inserted, CredentialAlreadyBound, AlreadyExists }` in `db/mod.rs`, and `DatabaseConfig::invite_registry_url()`.

- [ ] **Step 1: Add the config surface**

In `crates/trace-commons-server/src/config.rs`, add the field to `DatabaseConfig` beside `pii_backstop_driver_url`, and mirror the existing reader and accessor exactly:

```rust
/// Narrow, SEPARATE connection string for the `trace_invite_registry` role
/// (NOLOGIN base, NOBYPASSRLS, permissive policy from V42). Serves both the
/// invite cache refresh and the admin invite API. `None` keeps invite
/// redemption fail-closed under authoritative mode. NEVER aliased to `url`.
pub invite_registry_url: Option<SecretString>,
```

```rust
/// `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL`. A blank value is treated as
/// unset so a misconfigured deploy fails closed rather than building a pool
/// from an empty string. Mirrors `gate_driver_url_from_env`.
pub fn invite_registry_url_from_env() -> Option<SecretString> {
    std::env::var("TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(SecretString::from)
}

pub fn invite_registry_url(&self) -> Option<&str> {
    self.invite_registry_url.as_ref().map(|s| s.expose_secret())
}
```

Wire `invite_registry_url: Self::invite_registry_url_from_env()` into the same constructor that sets the other three.

`DatabaseConfig` has no `Default` impl, so every struct literal is exhaustive and adding this field breaks each one. Find them and fix them:

```bash
grep -rn "pii_backstop_driver_url:" --include=*.rs crates/ | grep -v "src/config.rs"
```

In production and binary code use `DatabaseConfig::invite_registry_url_from_env()`; in tests use `None` unless the test needs the registry pool. Task 1's `postgres_test_config()` in `tests/trace_invite_registry_pg.rs` is one of the literals that must be updated here — add `invite_registry_url: DatabaseConfig::invite_registry_url_from_env()` to it.

- [ ] **Step 2: Add the pool**

In `crates/trace-commons-server/src/db/postgres.rs`, add the field to `PgBackend` and build it in `PgBackend::new`, mirroring `gate_driver_pool` exactly:

```rust
/// Narrow, SEPARATE pool for the invite registry cache refresh and the
/// admin invite API. Built only when `invite_registry_url` is configured;
/// its DB user is the operator-provisioned `trace_invite_registry` role
/// (NOLOGIN base, NOBYPASSRLS, permissive policy from V42). `None` keeps
/// invite redemption fail-closed. NEVER aliased to `pool`.
invite_registry_pool: Option<Pool>,
```

```rust
let invite_registry_pool = match config.invite_registry_url() {
    Some(invite_registry_url) => {
        let invite_registry_config = invite_registry_url
            .parse::<tokio_postgres::Config>()
            .map_err(|e| {
                DatabaseError::Pool(format!("invalid invite-registry PostgreSQL URL: {e}"))
            })?;
        let invite_registry_manager =
            deadpool_postgres::Manager::new(invite_registry_config, tokio_postgres::NoTls);
        let invite_registry_pool = Pool::builder(invite_registry_manager).max_size(2).build()?;
        Some(invite_registry_pool)
    }
    None => None,
};
```

Add `invite_registry_pool` to the `Ok(Self { .. })` literal.

- [ ] **Step 3: Write the failing store tests**

Append to `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`:

```rust
use trace_commons_server::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use trace_commons_server::trace_invite_registry::InviteTenantMode;

const TEST_HASH_C: &str =
    "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";
const TEST_HASH_D: &str =
    "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
const TEST_CRED: &str =
    "sha256:eeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeeee";

fn registry_test_config() -> Option<DatabaseConfig> {
    let mut config = postgres_test_config()?;
    // The registry pool is required for these tests. Fall back to the same
    // URL when no dedicated registry URL is configured locally; the RLS tests
    // above are what prove the roles are distinct in production.
    let registry_url = std::env::var("TRACE_COMMONS_INVITE_REGISTRY_TEST_DATABASE_URL")
        .ok()
        .or_else(|| std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok())
        .or_else(|| std::env::var("DATABASE_URL").ok())?;
    config.invite_registry_url = Some(SecretString::from(registry_url));
    Some(config)
}

fn derived_write(hash: &str) -> InviteGrantWrite {
    InviteGrantWrite {
        invite_subject_hash: hash.to_string(),
        policy_label: "test-pool".to_string(),
        tenant_mode: InviteTenantMode::Derived,
        fixed_tenant_id: None,
        tenant_template_id: Some("tmpl-1".to_string()),
        policy_version: "v1".to_string(),
        allowed_consent_scopes: vec!["model_training".to_string()],
        allowed_uses: vec!["research".to_string()],
        max_uses: 3,
        expires_at: None,
        issuance_source: "operator".to_string(),
        issued_by_label: None,
        credential_binding_hash: None,
        note_label: None,
    }
}

#[tokio::test]
async fn insert_then_list_round_trips_every_field() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let outcome = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    let all = backend.list_invite_grants().await.expect("list");
    let found = all
        .iter()
        .find(|e| e.invite_subject_hash == TEST_HASH_C)
        .expect("inserted invite present in listing");
    assert_eq!(found.tenant_mode, InviteTenantMode::Derived);
    assert_eq!(found.tenant_template_id.as_deref(), Some("tmpl-1"));
    assert_eq!(found.fixed_tenant_id, None);
    assert_eq!(found.allowed_consent_scopes, vec!["model_training"]);
    assert_eq!(found.allowed_uses, vec!["research"]);
    assert_eq!(found.max_uses, 3);
    assert!(found.revoked_at.is_none());
}

#[tokio::test]
async fn a_second_live_invite_for_one_credential_is_refused() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let mut first = derived_write(TEST_HASH_C);
    first.credential_binding_hash = Some(TEST_CRED.to_string());
    let _ = backend.insert_invite_grant(first).await.expect("first insert");

    let mut second = derived_write(TEST_HASH_D);
    second.credential_binding_hash = Some(TEST_CRED.to_string());
    let outcome = backend
        .insert_invite_grant(second)
        .await
        .expect("second insert must not error");
    assert!(
        matches!(outcome, InviteGrantInsertOutcome::CredentialAlreadyBound),
        "one credential must not mint two live invites"
    );
}

#[tokio::test]
async fn revoking_frees_the_credential_binding_for_reissue() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let mut first = derived_write(TEST_HASH_C);
    first.credential_binding_hash = Some(TEST_CRED.to_string());
    let _ = backend.insert_invite_grant(first).await.expect("insert");

    let revoked = backend
        .revoke_invite_grant(TEST_HASH_C)
        .await
        .expect("revoke");
    assert!(revoked, "revoking a live invite reports true");

    let mut second = derived_write(TEST_HASH_D);
    second.credential_binding_hash = Some(TEST_CRED.to_string());
    let outcome = backend.insert_invite_grant(second).await.expect("reissue");
    assert!(matches!(outcome, InviteGrantInsertOutcome::Inserted));

    // Revoking an already-revoked invite is a no-op, not an error.
    let again = backend
        .revoke_invite_grant(TEST_HASH_C)
        .await
        .expect("second revoke");
    assert!(!again);
}

#[tokio::test]
async fn listing_excludes_revoked_invites() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");
    let _ = backend.revoke_invite_grant(TEST_HASH_C).await.expect("revoke");

    let all = backend.list_invite_grants().await.expect("list");
    assert!(
        !all.iter().any(|e| e.invite_subject_hash == TEST_HASH_C),
        "cache refresh must not load revoked invites"
    );
}
```

Each test writes the same fixed hashes, so add a cleanup at the top of every one of these four tests, immediately after `run_migrations`:

```rust
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'test-pool'",
            &[],
        )
        .await
        .expect("cleanup");
```

- [ ] **Step 4: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: compile errors — `InviteGrantWrite`, `InviteGrantInsertOutcome`, and the four store methods do not exist.

- [ ] **Step 5: Add the types to `db/mod.rs`**

```rust
use crate::trace_invite_registry::InviteTenantMode;

/// Insert payload for an invite grant. Mirrors `InviteEntry` minus
/// `revoked_at`, which is only ever set by `revoke_invite_grant`.
#[derive(Debug, Clone)]
pub struct InviteGrantWrite {
    pub invite_subject_hash: String,
    pub policy_label: String,
    pub tenant_mode: InviteTenantMode,
    pub fixed_tenant_id: Option<String>,
    pub tenant_template_id: Option<String>,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub max_uses: u32,
    pub expires_at: Option<chrono::DateTime<chrono::Utc>>,
    pub issuance_source: String,
    pub issued_by_label: Option<String>,
    pub credential_binding_hash: Option<String>,
    pub note_label: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InviteGrantInsertOutcome {
    Inserted,
    /// The partial unique index refused the write: this credential already has
    /// a live invite in this pool.
    CredentialAlreadyBound,
    /// An invite with this hash already exists. Makes the file import
    /// idempotent.
    AlreadyExists,
}
```

- [ ] **Step 6: Implement the store methods**

In `crates/trace-commons-server/src/db/postgres.rs`, add an accessor and a row mapper, then the four methods:

```rust
fn invite_registry_pool(&self) -> Result<Pool, DatabaseError> {
    self.invite_registry_pool
        .clone()
        .ok_or_else(|| DatabaseError::Pool("invite registry pool not configured".to_string()))
}

fn invite_entry_from_row(row: tokio_postgres::Row) -> Result<InviteEntry, DatabaseError> {
    let mode: String = row.get("tenant_mode");
    let tenant_mode = match mode.as_str() {
        "fixed" => InviteTenantMode::Fixed,
        "derived" => InviteTenantMode::Derived,
        other => {
            return Err(DatabaseError::Serialization(format!(
                "unknown invite tenant_mode {other:?}"
            )));
        }
    };
    let max_uses: i32 = row.get("max_uses");
    Ok(InviteEntry {
        invite_subject_hash: row.get("invite_subject_hash"),
        policy_label: row.get("policy_label"),
        tenant_mode,
        fixed_tenant_id: row.get("fixed_tenant_id"),
        tenant_template_id: row.get("tenant_template_id"),
        policy_version: row.get("policy_version"),
        allowed_consent_scopes: row.get("allowed_consent_scopes"),
        allowed_uses: row.get("allowed_uses"),
        max_uses: max_uses as u32,
        expires_at: row.get("expires_at"),
        issuance_source: row.get("issuance_source"),
        issued_by_label: row.get("issued_by_label"),
        credential_binding_hash: row.get("credential_binding_hash"),
        note_label: row.get("note_label"),
        revoked_at: row.get("revoked_at"),
    })
}

const INVITE_GRANT_COLUMNS: &str = "invite_subject_hash, policy_label, tenant_mode,
    fixed_tenant_id, tenant_template_id, policy_version, allowed_consent_scopes,
    allowed_uses, max_uses, expires_at, issuance_source, issued_by_label,
    credential_binding_hash, note_label, revoked_at";

impl PgBackend {
    /// Cache-refresh and admin listing. Runs on the registry pool, whose
    /// permissive V42 policy is what authorizes cross-invite reads. Excludes
    /// revoked and expired rows: the cache only ever holds live invites.
    pub async fn list_invite_grants(&self) -> Result<Vec<InviteEntry>, DatabaseError> {
        let pool = self.invite_registry_pool()?;
        let client = pool.get().await?;
        let rows = client
            .query(
                &format!(
                    "SELECT {INVITE_GRANT_COLUMNS}
                       FROM onboarding_invite_grants
                      WHERE revoked_at IS NULL
                        AND (expires_at IS NULL OR expires_at > NOW())"
                ),
                &[],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        rows.into_iter().map(Self::invite_entry_from_row).collect()
    }

    pub async fn insert_invite_grant(
        &self,
        write: crate::db::InviteGrantWrite,
    ) -> Result<crate::db::InviteGrantInsertOutcome, DatabaseError> {
        let pool = self.invite_registry_pool()?;
        let client = pool.get().await?;
        let tenant_mode = match write.tenant_mode {
            InviteTenantMode::Fixed => "fixed",
            InviteTenantMode::Derived => "derived",
        };
        let max_uses = i32::try_from(write.max_uses).map_err(|_| {
            DatabaseError::Serialization("invite max_uses out of range".to_string())
        })?;
        let inserted = client
            .query_opt(
                "INSERT INTO onboarding_invite_grants (
                    invite_subject_hash, policy_label, tenant_mode, fixed_tenant_id,
                    tenant_template_id, policy_version, allowed_consent_scopes,
                    allowed_uses, max_uses, expires_at, issuance_source,
                    issued_by_label, credential_binding_hash, note_label
                 ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)
                 ON CONFLICT (invite_subject_hash) DO NOTHING
                 RETURNING invite_subject_hash",
                &[
                    &write.invite_subject_hash,
                    &write.policy_label,
                    &tenant_mode,
                    &write.fixed_tenant_id,
                    &write.tenant_template_id,
                    &write.policy_version,
                    &write.allowed_consent_scopes,
                    &write.allowed_uses,
                    &max_uses,
                    &write.expires_at,
                    &write.issuance_source,
                    &write.issued_by_label,
                    &write.credential_binding_hash,
                    &write.note_label,
                ],
            )
            .await;

        match inserted {
            Ok(Some(_)) => Ok(crate::db::InviteGrantInsertOutcome::Inserted),
            Ok(None) => Ok(crate::db::InviteGrantInsertOutcome::AlreadyExists),
            Err(e) => {
                // 23505 unique_violation from the partial credential index.
                // Report it as a typed outcome, not an opaque 500, and never
                // echo the credential hash into the error.
                let is_unique_violation = e
                    .code()
                    .map(|c| c == &tokio_postgres::error::SqlState::UNIQUE_VIOLATION)
                    .unwrap_or(false);
                if is_unique_violation {
                    Ok(crate::db::InviteGrantInsertOutcome::CredentialAlreadyBound)
                } else {
                    Err(DatabaseError::Postgres(e))
                }
            }
        }
    }

    /// Soft revoke. Returns true only when this call is what revoked it, so a
    /// second revoke is a reported no-op rather than an error.
    pub async fn revoke_invite_grant(
        &self,
        invite_subject_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let pool = self.invite_registry_pool()?;
        let client = pool.get().await?;
        let updated = client
            .execute(
                "UPDATE onboarding_invite_grants
                    SET revoked_at = NOW(), updated_at = NOW()
                  WHERE invite_subject_hash = $1 AND revoked_at IS NULL",
                &[&invite_subject_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(updated == 1)
    }

    /// Authoritative in-transaction re-check on the RUNTIME pool. Sets the
    /// GUC the V42 invite_lookup policy reads, so this can only ever return
    /// the invite whose code the caller presented.
    pub async fn lookup_invite_grant_in_tx(
        tx: &deadpool_postgres::Transaction<'_>,
        invite_subject_hash: &str,
    ) -> Result<Option<InviteEntry>, DatabaseError> {
        tx.execute(
            "SELECT set_config('trace_commons.invite_subject', $1, true)",
            &[&invite_subject_hash],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {INVITE_GRANT_COLUMNS}
                       FROM onboarding_invite_grants
                      WHERE invite_subject_hash = $1
                        AND revoked_at IS NULL
                        AND (expires_at IS NULL OR expires_at > NOW())
                      FOR SHARE"
                ),
                &[&invite_subject_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        row.map(Self::invite_entry_from_row).transpose()
    }
}
```

`FOR SHARE` is what makes a revoke racing a redemption resolve deterministically: the revoking `UPDATE` blocks until the redemption transaction commits or rolls back.

- [ ] **Step 7: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: all seven tests PASS (three from Task 1, four new).

- [ ] **Step 8: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-server/src/config.rs \
        crates/trace-commons-server/src/db/mod.rs \
        crates/trace-commons-server/src/db/postgres.rs \
        crates/trace-commons-server/tests/trace_invite_registry_pg.rs
git commit -m "Add invite-grant store methods on a narrow registry pool

Cross-invite reads and writes run as trace_invite_registry; the authoritative
redemption re-check runs on the runtime pool under the GUC policy with FOR
SHARE, so a revoke racing a redemption is resolved by the database."
```

---

### Task 4: `DbInviteRegistry` — refresh loop and trait impl

**Files:**
- Modify: `crates/trace-commons-server/src/trace_invite_registry.rs`

**Interfaces:**
- Consumes: `InviteCache`, `InviteRegistry`, `InviteRegistryError` (Task 2); `PgBackend::list_invite_grants` (Task 3).
- Produces: `pub struct DbInviteRegistry` with `pub async fn new(backend: Arc<PgBackend>, refresh_interval: Duration, max_stale: Duration) -> Result<Self, InviteRegistryError>`, `pub async fn refresh_once(&self) -> Result<usize, InviteRegistryError>`, and `pub fn spawn_refresh_task(self: Arc<Self>) -> tokio::task::JoinHandle<()>`. Implements `InviteRegistry`.

- [ ] **Step 1: Write the failing test**

Append to the `mod tests` block in `trace_invite_registry.rs`:

```rust
    #[test]
    fn a_never_refreshed_registry_fails_closed() {
        // A cache that has never loaded must be treated as maximally stale,
        // so a registry whose first refresh failed cannot silently authorize
        // nothing-is-valid as everything-is-invalid-but-fresh.
        let cache = InviteCache::new(Duration::from_secs(60));
        match cache.lookup("sha256:aa") {
            Err(InviteRegistryError::Stale { .. }) => {}
            other => panic!("unloaded cache must be stale, got {other:?}"),
        }
    }
```

- [ ] **Step 2: Run the test to verify it fails or passes**

```bash
cargo test -p trace-commons-server --lib trace_invite_registry::tests::a_never_refreshed_registry_fails_closed
```

Expected: PASS, because `InviteCache::new` already backdates `loaded_at`. If it FAILS, the backdating in Task 2 Step 3 was dropped — restore it before continuing. This test exists to pin that behavior against future edits.

- [ ] **Step 3: Implement `DbInviteRegistry`**

Append to `trace_invite_registry.rs`, above the test module:

```rust
use std::sync::Arc;

use crate::db::postgres::PgBackend;

/// DB-backed invite registry. The cache is refreshed on a timer from the
/// narrow registry pool and invalidated synchronously by the admin write
/// path, so a minted code is redeemable in the same instant.
pub struct DbInviteRegistry {
    backend: Arc<PgBackend>,
    cache: InviteCache,
    refresh_interval: Duration,
}

impl DbInviteRegistry {
    /// Warms the cache once before returning. A failed warm is an error: the
    /// issuer must not come up believing it has a usable registry.
    pub async fn new(
        backend: Arc<PgBackend>,
        refresh_interval: Duration,
        max_stale: Duration,
    ) -> Result<Self, InviteRegistryError> {
        let registry = Self {
            backend,
            cache: InviteCache::new(max_stale),
            refresh_interval,
        };
        registry.refresh_once().await?;
        Ok(registry)
    }

    /// Reload every live invite. Returns the count loaded.
    pub async fn refresh_once(&self) -> Result<usize, InviteRegistryError> {
        let entries = self
            .backend
            .list_invite_grants()
            .await
            // Label only. Never let a connection string or row content reach
            // this string; it surfaces in operator-visible status output.
            .map_err(|_| InviteRegistryError::Backend("invite-registry-query-failed".to_string()))?;
        let count = entries.len();
        self.cache.replace_all(entries, Instant::now());
        Ok(count)
    }

    /// Background refresh. A failed refresh leaves the previous snapshot in
    /// place and lets it age into staleness, which then fails closed —
    /// matching FileAllowlistSource's posture exactly.
    pub fn spawn_refresh_task(self: Arc<Self>) -> tokio::task::JoinHandle<()> {
        let interval = self.refresh_interval;
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            ticker.tick().await; // the first tick fires immediately; skip it
            loop {
                ticker.tick().await;
                let _ = self.refresh_once().await;
            }
        })
    }
}

impl InviteRegistry for DbInviteRegistry {
    fn lookup(
        &self,
        invite_subject_hash: &str,
    ) -> Result<Option<InviteEntry>, InviteRegistryError> {
        self.cache.lookup(invite_subject_hash)
    }

    fn note_write(&self, entry: InviteEntry) {
        self.cache.note_write(entry);
    }

    fn note_revoke(&self, invite_subject_hash: &str) {
        self.cache.note_revoke(invite_subject_hash);
    }

    fn status(&self) -> InviteRegistryStatus {
        self.cache.status()
    }
}
```

- [ ] **Step 4: Run the full unit-test module**

```bash
cargo test -p trace-commons-server --lib trace_invite_registry
```

Expected: all 9 tests PASS.

- [ ] **Step 5: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_invite_registry.rs
git commit -m "Wire the invite cache to the registry pool with a refresh timer

A failed refresh leaves the previous snapshot to age into staleness and then
fail closed, matching the file allowlist source's posture."
```

---

### Task 5: Admin JWT verification for the issuer

**Files:**
- Create: `crates/trace-commons-server/src/trace_invite_admin.rs`
- Modify: `crates/trace-commons-server/src/lib.rs`

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces: `pub struct AdminClaims { pub sub: String, pub role: String, pub iss: String, pub aud: String, pub jti: String, pub exp: usize }`, and `pub fn verify_admin_token(token: &str, decoding_key: &jsonwebtoken::DecodingKey, expected_iss: &str, expected_aud: &str) -> Result<AdminClaims, AdminAuthError>` with `pub enum AdminAuthError { Malformed, WrongAlgorithm, NotAdmin, Expired, Invalid }`.

The issuer has no admin authentication today; `/v1/admin/allowlist-status` relies on loopback binding. That is acceptable for a counts-only endpoint and is not acceptable for a route that mints credentials. The issuer signs these tokens with its own key, so it verifies them with `signing_decoding_key()` (`trace_upload_claim_issuer.rs:626`).

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-server/src/trace_invite_admin.rs`:

```rust
//! Admin authentication and invite lifecycle routes for the upload-claim
//! issuer.
//!
//! Unlike `/v1/admin/allowlist-status`, these routes mint and revoke
//! credentials, so they are gated on an EdDSA admin JWT rather than on
//! loopback binding alone.

#[cfg(test)]
mod tests {
    use super::*;
    use jsonwebtoken::{Algorithm, DecodingKey, EncodingKey, Header, encode};

    const ISS: &str = "trace-commons-upload-claim-issuer";
    const AUD: &str = "trace-commons-issuer-admin";

    /// Ed25519 PKCS#8 v2 test keypair. Generated once for tests only; never
    /// used by any deployment.
    fn test_keys() -> (EncodingKey, DecodingKey) {
        let (private_pem, public_pem) = generate_test_ed25519_pem();
        (
            EncodingKey::from_ed_pem(private_pem.as_bytes()).expect("encoding key"),
            DecodingKey::from_ed_pem(public_pem.as_bytes()).expect("decoding key"),
        )
    }

    fn sign(enc: &EncodingKey, role: &str, exp_offset_secs: i64) -> String {
        let exp = (chrono::Utc::now().timestamp() + exp_offset_secs) as usize;
        let claims = AdminClaims {
            sub: "operator-1".to_string(),
            role: role.to_string(),
            iss: ISS.to_string(),
            aud: AUD.to_string(),
            jti: "jti-1".to_string(),
            exp,
        };
        encode(&Header::new(Algorithm::EdDSA), &claims, enc).expect("sign")
    }

    #[test]
    fn an_admin_role_token_verifies() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        let claims = verify_admin_token(&token, &dec, ISS, AUD).expect("verifies");
        assert_eq!(claims.role, "admin");
        assert_eq!(claims.sub, "operator-1");
    }

    #[test]
    fn a_non_admin_role_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "reviewer", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, AUD),
            Err(AdminAuthError::NotAdmin)
        ));
    }

    #[test]
    fn an_expired_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", -300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, AUD),
            Err(AdminAuthError::Expired)
        ));
    }

    #[test]
    fn a_wrong_audience_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, ISS, "some-other-audience"),
            Err(AdminAuthError::Invalid)
        ));
    }

    #[test]
    fn a_wrong_issuer_token_is_refused() {
        let (enc, dec) = test_keys();
        let token = sign(&enc, "admin", 300);
        assert!(matches!(
            verify_admin_token(&token, &dec, "someone-else", AUD),
            Err(AdminAuthError::Invalid)
        ));
    }

    #[test]
    fn a_garbage_token_is_refused_without_panicking() {
        let (_, dec) = test_keys();
        assert!(matches!(
            verify_admin_token("not-a-jwt", &dec, ISS, AUD),
            Err(AdminAuthError::Malformed)
        ));
    }

    /// Ring generates PKCS#8 v2 Ed25519 keys, which is what the issuer's own
    /// key loading requires.
    fn generate_test_ed25519_pem() -> (String, String) {
        use ring::signature::{Ed25519KeyPair, KeyPair};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("generate");
        let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse");
        let private_pem = pem_wrap("PRIVATE KEY", pkcs8.as_ref());
        // SubjectPublicKeyInfo prefix for Ed25519.
        let mut spki = vec![
            0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
        ];
        spki.extend_from_slice(pair.public_key().as_ref());
        let public_pem = pem_wrap("PUBLIC KEY", &spki);
        (private_pem, public_pem)
    }

    fn pem_wrap(label: &str, der: &[u8]) -> String {
        use base64::Engine as _;
        let b64 = base64::engine::general_purpose::STANDARD.encode(der);
        let body = b64
            .as_bytes()
            .chunks(64)
            .map(|c| String::from_utf8_lossy(c).to_string())
            .collect::<Vec<_>>()
            .join("\n");
        format!("-----BEGIN {label}-----\n{body}\n-----END {label}-----\n")
    }
}
```

Declare the module in `lib.rs`:

```rust
pub mod trace_invite_admin;
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --lib trace_invite_admin
```

Expected: compile errors — `AdminClaims`, `verify_admin_token`, `AdminAuthError` are not defined.

- [ ] **Step 3: Implement verification**

Prepend to `trace_invite_admin.rs`, above the test module:

```rust
use jsonwebtoken::errors::ErrorKind as JwtErrorKind;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminClaims {
    pub sub: String,
    pub role: String,
    pub iss: String,
    pub aud: String,
    pub jti: String,
    pub exp: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminAuthError {
    Malformed,
    WrongAlgorithm,
    NotAdmin,
    Expired,
    Invalid,
}

impl AdminAuthError {
    /// Public label. Deliberately coarse: an unauthenticated caller learns
    /// only that they are not authorized, never why.
    pub fn public_label(self) -> &'static str {
        match self {
            Self::NotAdmin => "AdminRoleRequired",
            _ => "AdminTokenInvalid",
        }
    }
}

/// Verify an EdDSA admin token minted by this issuer's own signing key.
/// Rejects any algorithm other than EdDSA before touching the signature, so a
/// caller cannot downgrade to `none` or to an HMAC the public key would
/// satisfy.
pub fn verify_admin_token(
    token: &str,
    decoding_key: &DecodingKey,
    expected_iss: &str,
    expected_aud: &str,
) -> Result<AdminClaims, AdminAuthError> {
    let header = jsonwebtoken::decode_header(token).map_err(|_| AdminAuthError::Malformed)?;
    if header.alg != Algorithm::EdDSA {
        return Err(AdminAuthError::WrongAlgorithm);
    }
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_issuer(&[expected_iss]);
    validation.set_audience(&[expected_aud]);
    validation.set_required_spec_claims(&["exp", "iss", "aud"]);

    let decoded = jsonwebtoken::decode::<AdminClaims>(token, decoding_key, &validation).map_err(
        |e| match e.kind() {
            JwtErrorKind::ExpiredSignature => AdminAuthError::Expired,
            JwtErrorKind::Base64(_) | JwtErrorKind::Json(_) => AdminAuthError::Malformed,
            _ => AdminAuthError::Invalid,
        },
    )?;

    if decoded.claims.role != "admin" {
        return Err(AdminAuthError::NotAdmin);
    }
    Ok(decoded.claims)
}
```

`jti` is carried and returned but not yet replay-checked; single-use enforcement is a separate concern from this slice and the tokens are short-lived.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --lib trace_invite_admin
```

Expected: all 6 tests PASS. If `a_garbage_token_is_refused_without_panicking` returns `Invalid` rather than `Malformed`, adjust the assertion to accept either — the distinction is not security-relevant, but the test must not be deleted.

- [ ] **Step 5: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server/src/trace_invite_admin.rs \
        crates/trace-commons-server/src/lib.rs
git commit -m "Verify EdDSA admin tokens on the issuer

Routes that mint credentials need more than a loopback bind. The algorithm is
pinned before the signature is checked so a caller cannot downgrade to none."
```

---

### Task 6: Admin invite routes

**Files:**
- Modify: `crates/trace-commons-server/src/trace_invite_admin.rs`
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer_admin.rs`

**Interfaces:**
- Consumes: `verify_admin_token`, `AdminAuthError` (Task 5); `DbInviteRegistry`, `generate_invite_code`, `InviteEntry`, `InviteTenantMode` (Tasks 2 and 4); `PgBackend::insert_invite_grant`, `revoke_invite_grant` (Task 3); `hash_invite_code` from `trace_upload_claim_allowlist`.
- Produces: `pub struct InviteAdminState { pub backend: Arc<PgBackend>, pub registry: Arc<DbInviteRegistry>, pub decoding_key: Arc<DecodingKey>, pub expected_iss: String, pub expected_aud: String, pub default_policy_label: String }` and `pub fn invite_admin_router(state: InviteAdminState) -> Router`.

- [ ] **Step 1: Write the failing tests**

Append to the `mod tests` block in `trace_invite_admin.rs`:

```rust
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test]
    async fn creating_an_invite_without_a_token_is_refused() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let app = invite_admin_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn a_created_invite_is_returned_once_and_is_immediately_live() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let registry = state.registry.clone();
        let token = state.test_admin_token();
        let app = invite_admin_router(state);

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1",
                            "policy_version":"v1","max_uses":3,
                            "allowed_consent_scopes":["model_training"],
                            "allowed_uses":["research"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);

        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let code = json["invite_code"].as_str().expect("raw code returned once");
        assert_eq!(code.len(), 16);

        // The registry must already know it: no refresh window.
        let hash = crate::trace_upload_claim_allowlist::hash_invite_code(code);
        assert_eq!(json["invite_subject_hash"], hash);
        assert!(
            registry.lookup(&hash).expect("lookup").is_some(),
            "a minted invite must be immediately redeemable"
        );
    }

    #[tokio::test]
    async fn listing_never_returns_raw_codes() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let token = state.test_admin_token();
        let app = invite_admin_router(state);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let text = String::from_utf8(body.to_vec()).unwrap();
        assert!(
            !text.contains("invite_code"),
            "listing must never carry a raw code: {text}"
        );
    }

    #[tokio::test]
    async fn revoking_removes_the_invite_from_the_registry() {
        let Some(state) = test_invite_admin_state().await else {
            eprintln!("skipping: no test database configured");
            return;
        };
        let registry = state.registry.clone();
        let token = state.test_admin_token();
        let app = invite_admin_router(state);

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/admin/invites")
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"tenant_mode":"derived","tenant_template_id":"tmpl-1",
                            "policy_version":"v1"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = axum::body::to_bytes(created.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        let hash = json["invite_subject_hash"].as_str().unwrap().to_string();
        assert!(registry.lookup(&hash).unwrap().is_some());

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/v1/admin/invites/{hash}/revoke"))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert!(
            registry.lookup(&hash).unwrap().is_none(),
            "revoke must take effect with no refresh window"
        );
    }
```

Add the test helper beside them. It returns `None` when no database is configured so the suite stays skippable:

```rust
    /// Builds real state against the test database. Returns None when no test
    /// database is configured.
    async fn test_invite_admin_state() -> Option<TestInviteAdminState> {
        use std::sync::Arc;
        use std::time::Duration;
        use secrecy::SecretString;
        use crate::config::{DatabaseConfig, SslMode};
        use crate::db::postgres::PgBackend;
        use crate::trace_invite_registry::DbInviteRegistry;

        let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .ok()?;
        let config = DatabaseConfig {
            url: SecretString::from(url.clone()),
            invite_registry_url: Some(SecretString::from(url)),
            pool_size: 4,
            ssl_mode: SslMode::Prefer,
            login_resolver_url: None,
            gate_driver_url: None,
            pii_backstop_driver_url: None,
        };
        let backend = Arc::new(PgBackend::new(&config).await.ok()?);
        backend.run_migrations().await.ok()?;
        let registry = Arc::new(
            DbInviteRegistry::new(
                backend.clone(),
                Duration::from_secs(60),
                Duration::from_secs(300),
            )
            .await
            .ok()?,
        );
        let (private_pem, public_pem) = generate_test_ed25519_pem();
        Some(TestInviteAdminState {
            inner: InviteAdminState {
                backend,
                registry,
                decoding_key: Arc::new(
                    DecodingKey::from_ed_pem(public_pem.as_bytes()).ok()?,
                ),
                expected_iss: ISS.to_string(),
                expected_aud: AUD.to_string(),
                default_policy_label: "test-pool".to_string(),
            },
            signing_pem: private_pem,
        })
    }

    /// Wrapper carrying the private key so tests can mint their own tokens.
    struct TestInviteAdminState {
        inner: InviteAdminState,
        signing_pem: String,
    }

    impl TestInviteAdminState {
        fn test_admin_token(&self) -> String {
            let enc = jsonwebtoken::EncodingKey::from_ed_pem(self.signing_pem.as_bytes())
                .expect("encoding key");
            sign(&enc, "admin", 300)
        }
        fn registry_handle(&self) -> std::sync::Arc<crate::trace_invite_registry::DbInviteRegistry> {
            self.inner.registry.clone()
        }
    }
```

In the four tests above, replace `state.registry.clone()` with `state.registry_handle()`, `state.test_admin_token()` stays as written, and `invite_admin_router(state)` becomes `invite_admin_router(state.inner)`.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --lib trace_invite_admin
```

Expected: compile errors — `InviteAdminState` and `invite_admin_router` are not defined.

- [ ] **Step 3: Implement the routes**

Append to `trace_invite_admin.rs`, above the test module:

```rust
use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::{DateTime, Duration as ChronoDuration, Utc};
use serde_json::{Value, json};

use crate::db::postgres::PgBackend;
use crate::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use crate::trace_invite_registry::{
    DbInviteRegistry, InviteEntry, InviteRegistry, InviteTenantMode, generate_invite_code,
};
use crate::trace_upload_claim_allowlist::hash_invite_code;

#[derive(Clone)]
pub struct InviteAdminState {
    pub backend: Arc<PgBackend>,
    pub registry: Arc<DbInviteRegistry>,
    pub decoding_key: Arc<DecodingKey>,
    pub expected_iss: String,
    pub expected_aud: String,
    pub default_policy_label: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateInviteRequest {
    pub tenant_mode: String,
    #[serde(default)]
    pub fixed_tenant_id: Option<String>,
    #[serde(default)]
    pub tenant_template_id: Option<String>,
    #[serde(default = "default_policy_version")]
    pub policy_version: String,
    #[serde(default)]
    pub allowed_consent_scopes: Vec<String>,
    #[serde(default)]
    pub allowed_uses: Vec<String>,
    #[serde(default = "default_max_uses")]
    pub max_uses: u32,
    /// Relative expiry. Absent means no expiry.
    #[serde(default)]
    pub expires_in_days: Option<i64>,
    #[serde(default = "default_issuance_source")]
    pub issuance_source: String,
    #[serde(default)]
    pub issued_by_label: Option<String>,
    #[serde(default)]
    pub credential_binding_hash: Option<String>,
    #[serde(default)]
    pub note_label: Option<String>,
}

fn default_policy_version() -> String {
    "v1".to_string()
}
fn default_max_uses() -> u32 {
    3
}
fn default_issuance_source() -> String {
    "operator".to_string()
}

pub fn invite_admin_router(state: InviteAdminState) -> Router {
    Router::new()
        .route(
            "/v1/admin/invites",
            post(create_invite_handler).get(list_invites_handler),
        )
        .route("/v1/admin/invites/{hash}/revoke", post(revoke_invite_handler))
        .route("/v1/admin/invite-registry-status", get(registry_status_handler))
        .with_state(state)
}

fn authorize(state: &InviteAdminState, headers: &HeaderMap) -> Result<(), (StatusCode, Json<Value>)> {
    let token = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty());
    let Some(token) = token else {
        return Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": "AdminTokenMissing" })),
        ));
    };
    match verify_admin_token(
        token,
        &state.decoding_key,
        &state.expected_iss,
        &state.expected_aud,
    ) {
        Ok(_) => Ok(()),
        Err(AdminAuthError::NotAdmin) => Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": AdminAuthError::NotAdmin.public_label() })),
        )),
        Err(e) => Err((
            StatusCode::UNAUTHORIZED,
            Json(json!({ "error": e.public_label() })),
        )),
    }
}

async fn create_invite_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
    Json(request): Json<CreateInviteRequest>,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }

    let (tenant_mode, fixed_tenant_id, tenant_template_id) =
        match (request.tenant_mode.as_str(), &request.fixed_tenant_id, &request.tenant_template_id) {
            ("fixed", Some(t), None) => (InviteTenantMode::Fixed, Some(t.clone()), None),
            ("derived", None, Some(t)) => (InviteTenantMode::Derived, None, Some(t.clone())),
            _ => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(json!({ "error": "InviteTenantModeMalformed" })),
                );
            }
        };

    if request.max_uses == 0 {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "InviteMaxUsesMalformed" })),
        );
    }
    if let Some(h) = &request.credential_binding_hash {
        if !is_canonical_sha256(h) {
            return (
                StatusCode::BAD_REQUEST,
                Json(json!({ "error": "InviteCredentialBindingMalformed" })),
            );
        }
    }

    let expires_at: Option<DateTime<Utc>> = request
        .expires_in_days
        .map(|d| Utc::now() + ChronoDuration::days(d));

    // The raw code exists here and in exactly one response body. It is never
    // stored, logged, or retrievable afterward.
    let code = generate_invite_code();
    let invite_subject_hash = hash_invite_code(&code);

    let write = InviteGrantWrite {
        invite_subject_hash: invite_subject_hash.clone(),
        policy_label: state.default_policy_label.clone(),
        tenant_mode,
        fixed_tenant_id: fixed_tenant_id.clone(),
        tenant_template_id: tenant_template_id.clone(),
        policy_version: request.policy_version.clone(),
        allowed_consent_scopes: request.allowed_consent_scopes.clone(),
        allowed_uses: request.allowed_uses.clone(),
        max_uses: request.max_uses,
        expires_at,
        issuance_source: request.issuance_source.clone(),
        issued_by_label: request.issued_by_label.clone(),
        credential_binding_hash: request.credential_binding_hash.clone(),
        note_label: request.note_label.clone(),
    };

    match state.backend.insert_invite_grant(write).await {
        Ok(InviteGrantInsertOutcome::Inserted) => {
            // Invalidate AFTER the commit so the cache never advertises an
            // invite the database rejected.
            state.registry.note_write(InviteEntry {
                invite_subject_hash: invite_subject_hash.clone(),
                policy_label: state.default_policy_label.clone(),
                tenant_mode,
                fixed_tenant_id,
                tenant_template_id,
                policy_version: request.policy_version,
                allowed_consent_scopes: request.allowed_consent_scopes,
                allowed_uses: request.allowed_uses,
                max_uses: request.max_uses,
                expires_at,
                issuance_source: request.issuance_source,
                issued_by_label: request.issued_by_label,
                credential_binding_hash: request.credential_binding_hash,
                note_label: request.note_label,
                revoked_at: None,
            });
            (
                StatusCode::CREATED,
                Json(json!({
                    "invite_code": code,
                    "invite_subject_hash": invite_subject_hash,
                    "expires_at": expires_at,
                })),
            )
        }
        Ok(InviteGrantInsertOutcome::CredentialAlreadyBound) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "InviteCredentialAlreadyBound" })),
        ),
        // A hash collision on a 16-character CSPRNG code is not a real event;
        // treat it as a transient failure rather than returning a code whose
        // grant fields belong to someone else.
        Ok(InviteGrantInsertOutcome::AlreadyExists) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteCodeCollision" })),
        ),
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}

fn is_canonical_sha256(value: &str) -> bool {
    match value.strip_prefix("sha256:") {
        Some(hex) => {
            hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        }
        None => false,
    }
}

async fn list_invites_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    match state.backend.list_invite_grants().await {
        Ok(entries) => {
            let items: Vec<Value> = entries
                .iter()
                .map(|e| {
                    json!({
                        "invite_subject_hash": e.invite_subject_hash,
                        "policy_label": e.policy_label,
                        "tenant_mode": match e.tenant_mode {
                            InviteTenantMode::Fixed => "fixed",
                            InviteTenantMode::Derived => "derived",
                        },
                        "policy_version": e.policy_version,
                        "max_uses": e.max_uses,
                        "expires_at": e.expires_at,
                        "issuance_source": e.issuance_source,
                        "issued_by_label": e.issued_by_label,
                        "note_label": e.note_label,
                        "credential_bound": e.credential_binding_hash.is_some(),
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(json!({ "count": items.len(), "invites": items })),
            )
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}
```

The listing returns `credential_bound: true/false` rather than the binding hash. The hash is a stable pseudonym for a real contributor, and a listing endpoint has no need for it.

```rust
async fn revoke_invite_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
    Path(hash): Path<String>,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    if !is_canonical_sha256(&hash) {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "InviteSubjectHashMalformed" })),
        );
    }
    match state.backend.revoke_invite_grant(&hash).await {
        Ok(revoked) => {
            // Drop it from the cache either way: if it was already revoked,
            // the cache must not be the thing still advertising it.
            state.registry.note_revoke(&hash);
            (StatusCode::OK, Json(json!({ "revoked": revoked })))
        }
        Err(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({ "error": "InviteRegistryBackendUnavailable" })),
        ),
    }
}

async fn registry_status_handler(
    State(state): State<InviteAdminState>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    if let Err((status, body)) = authorize(&state, &headers) {
        return (status, body);
    }
    let status = state.registry.status();
    (
        StatusCode::OK,
        Json(json!({
            "live": status.live,
            "cache_age_seconds": status.cache_age_seconds,
            "stale": status.stale,
            "max_stale_seconds": status.max_stale_seconds,
        })),
    )
}
```

Add `use serde::Deserialize;` to the existing `serde` import line at the top of the file.

- [ ] **Step 4: Mount the router beside the existing admin route**

In `crates/trace-commons-server/src/trace_upload_claim_issuer_admin.rs`, add an optional field to `AdminState` and merge the router in `admin_router`:

```rust
pub invite_admin: Option<crate::trace_invite_admin::InviteAdminState>,
```

```rust
pub fn admin_router(state: AdminState) -> Router {
    let invite_admin = state.invite_admin.clone();
    let router = Router::new()
        .route("/v1/admin/allowlist-status", get(allowlist_status_handler))
        .with_state(state);
    match invite_admin {
        Some(invite_state) => {
            router.merge(crate::trace_invite_admin::invite_admin_router(invite_state))
        }
        None => router,
    }
}
```

Update every existing `AdminState { .. }` construction (including in that file's own test module) to pass `invite_admin: None`.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --lib trace_invite_admin
cargo test -p trace-commons-server --lib trace_upload_claim_issuer_admin
```

Expected: all PASS. The four database-backed route tests skip with a printed message when no test database is configured; run them at least once with `TRACE_COMMONS_PG_TEST_DATABASE_URL` set and confirm they actually ran rather than skipped.

- [ ] **Step 6: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server/src/trace_invite_admin.rs \
        crates/trace-commons-server/src/trace_upload_claim_issuer_admin.rs
git commit -m "Add admin invite create, list, revoke, and status routes

Code generation moves server-side so entropy and hashing cannot drift from the
enforcement path. The raw code appears in exactly one response body; listings
report only whether a credential binding exists."
```

---

### Task 7: Redemption through the registry

**Files:**
- Modify: `crates/trace-commons-protocol/src/onboarding.rs`
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`
- Modify: `crates/trace-commons-server/src/db/postgres.rs`
- Test: `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`

**Interfaces:**
- Consumes: `InviteRegistry`, `InviteEntry`, `InviteTenantMode`, `InviteRegistryError` (Tasks 2 and 4); `PgBackend::lookup_invite_grant_in_tx` (Task 3); existing `derive_user_tenant_id`, `hash_invite_code`, `onboard_device_key`.
- Produces: three new `TraceOnboardErrorCode` variants; `InviteRegistry` handle on the issuer state; `PgBackend::redeem_invite_grant(...)`.

- [ ] **Step 1: Add the error codes**

In `crates/trace-commons-protocol/src/onboarding.rs`, add to `TraceOnboardErrorCode` and to `as_wire_str`:

```rust
    InviteExpired,
    InviteRegistryNotConfigured,
    InviteRegistryStale,
```

```rust
            Self::InviteExpired => "InviteExpired",
            Self::InviteRegistryNotConfigured => "InviteRegistryNotConfigured",
            Self::InviteRegistryStale => "InviteRegistryStale",
```

`InviteCredentialAlreadyBound` is admin-only and is already returned as a plain JSON error label in Task 6, so it does not belong in this client-facing enum.

- [ ] **Step 2: Write the failing redemption tests**

Append to `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`:

```rust
use trace_commons_protocol::onboarding::derive_user_tenant_id;

#[tokio::test]
async fn a_derived_mode_invite_provisions_the_derived_tenant() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'test-pool'",
            &[],
        )
        .await
        .expect("cleanup");

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");

    let outcome = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
        .await
        .expect("redeem");

    assert_eq!(
        outcome.tenant_id,
        derive_user_tenant_id("tmpl-1", "user-subject-1"),
        "derived mode must resolve the tenant from template + user subject"
    );
    assert_eq!(outcome.allowed_consent_scopes, vec!["model_training"]);
    assert_eq!(outcome.allowed_uses, vec!["research"]);
    assert_eq!(outcome.policy_version, "v1");
}

#[tokio::test]
async fn a_fixed_mode_invite_uses_its_tenant_verbatim() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'test-pool'",
            &[],
        )
        .await
        .expect("cleanup");

    let mut write = derived_write(TEST_HASH_C);
    write.tenant_mode = InviteTenantMode::Fixed;
    write.tenant_template_id = None;
    write.fixed_tenant_id = Some("tenant-zaki-pilot".to_string());
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let outcome = backend
        .redeem_invite_grant(TEST_HASH_C, "user-subject-1")
        .await
        .expect("redeem");
    assert_eq!(outcome.tenant_id, "tenant-zaki-pilot");
}

#[tokio::test]
async fn a_revoked_invite_cannot_be_redeemed_even_from_a_warm_cache() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'test-pool'",
            &[],
        )
        .await
        .expect("cleanup");

    let _ = backend
        .insert_invite_grant(derived_write(TEST_HASH_C))
        .await
        .expect("insert");
    let _ = backend.revoke_invite_grant(TEST_HASH_C).await.expect("revoke");

    // The cache is deliberately bypassed here: this asserts the database, not
    // the cache, is what refuses a revoked invite.
    let result = backend.redeem_invite_grant(TEST_HASH_C, "user-subject-1").await;
    assert!(
        result.is_err(),
        "the in-transaction re-check must refuse a revoked invite"
    );
}

#[tokio::test]
async fn an_expired_invite_cannot_be_redeemed() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'test-pool'",
            &[],
        )
        .await
        .expect("cleanup");

    let mut write = derived_write(TEST_HASH_C);
    write.expires_at = Some(chrono::Utc::now() - chrono::Duration::seconds(1));
    let _ = backend.insert_invite_grant(write).await.expect("insert");

    let result = backend.redeem_invite_grant(TEST_HASH_C, "user-subject-1").await;
    assert!(result.is_err(), "an expired invite must not redeem");
}
```

Add `use chrono;` and `use trace_commons_protocol` to the test file's dependency list if the crate is not already a dev-dependency of `trace-commons-server`; `trace-commons-protocol` is a workspace path dependency, so it should already resolve.

- [ ] **Step 3: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: compile error — `redeem_invite_grant` does not exist.

- [ ] **Step 4: Implement `redeem_invite_grant`**

In `crates/trace-commons-server/src/db/postgres.rs`:

```rust
/// Resolved grant for a redeemed invite.
#[derive(Debug, Clone)]
pub struct InviteRedemption {
    pub tenant_id: String,
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
    pub max_uses: u32,
}

impl PgBackend {
    /// Authoritative redemption resolve. Runs on the RUNTIME pool under the
    /// V42 GUC policy, re-checks revocation and expiry inside the transaction,
    /// and holds FOR SHARE so a concurrent revoke serializes behind it.
    ///
    /// This does NOT increment the V29 counter; the caller does that in the
    /// same transaction via the existing onboard_device_key path.
    pub async fn redeem_invite_grant(
        &self,
        invite_subject_hash: &str,
        user_subject: &str,
    ) -> Result<InviteRedemption, DatabaseError> {
        let pool = self.trace_pool();
        let mut client = pool.get().await?;
        let tx = client.transaction().await.map_err(DatabaseError::Postgres)?;
        let entry = Self::lookup_invite_grant_in_tx(&tx, invite_subject_hash)
            .await?
            .ok_or_else(|| {
                // One label for absent, revoked, and expired: a caller must not
                // be able to distinguish "never existed" from "revoked".
                DatabaseError::Serialization("InviteNotValid".to_string())
            })?;

        let tenant_id = match entry.tenant_mode {
            InviteTenantMode::Fixed => entry.fixed_tenant_id.clone().ok_or_else(|| {
                DatabaseError::Serialization("invite fixed_tenant_id missing".to_string())
            })?,
            InviteTenantMode::Derived => {
                let template = entry.tenant_template_id.as_deref().ok_or_else(|| {
                    DatabaseError::Serialization("invite tenant_template_id missing".to_string())
                })?;
                trace_commons_protocol::onboarding::derive_user_tenant_id(template, user_subject)
            }
        };

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(InviteRedemption {
            tenant_id,
            policy_version: entry.policy_version,
            allowed_consent_scopes: entry.allowed_consent_scopes,
            allowed_uses: entry.allowed_uses,
            max_uses: entry.max_uses,
        })
    }
}
```

Export `InviteRedemption` from `db/mod.rs` alongside the other public store types.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: all 11 tests PASS.

- [ ] **Step 6: Wire the registry into the onboard handler**

In `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`:

Add to the issuer state struct (beside the existing allowlist source field):

```rust
/// DB-authoritative invite registry. `None` means the file allowlist is
/// still authoritative for invites (pre-cutover).
pub invite_registry: Option<Arc<DbInviteRegistry>>,
/// Cutover flag from TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE.
pub invite_registry_authoritative: bool,
```

In the onboard handler, immediately after the invite code is hashed and before the current allowlist `entry()` lookup (around line 1862, where `onboard_device_key` is called), insert:

```rust
if self.invite_registry_authoritative {
    let registry = self
        .invite_registry
        .as_ref()
        .ok_or_else(|| {
            IssuerError::onboard_error(
                StatusCode::SERVICE_UNAVAILABLE,
                TraceOnboardErrorCode::InviteRegistryNotConfigured,
            )
        })?;
    match registry.lookup(&invite_subject_hash) {
        Ok(Some(_)) => {}
        Ok(None) => {
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::InviteNotValid,
            ));
        }
        Err(InviteRegistryError::Stale { .. }) => {
            return Err(IssuerError::onboard_error(
                StatusCode::SERVICE_UNAVAILABLE,
                TraceOnboardErrorCode::InviteRegistryStale,
            ));
        }
        Err(InviteRegistryError::Backend(_)) => {
            return Err(IssuerError::internal());
        }
    }

    // The cache said yes; the database decides. Expiry, revocation, and the
    // tenant all come from the in-transaction re-check.
    let redemption = db
        .redeem_invite_grant(&invite_subject_hash, &user_subject)
        .await
        .map_err(|_| {
            IssuerError::onboard_error(
                StatusCode::FORBIDDEN,
                TraceOnboardErrorCode::InviteNotValid,
            )
        })?;

    return self
        .complete_onboard_with_redemption(request, redemption, invite_subject_hash)
        .await;
}
```

`complete_onboard_with_redemption` is a new private method that calls the existing `db.onboard_device_key(...)` with `tenant_id`, `max_uses`, `allowed_consent_scopes`, `allowed_uses`, and `policy_version` taken from the `InviteRedemption` rather than from the process-wide defaults. Extract the body of the current post-allowlist onboarding block into it so both paths share one implementation; do not duplicate the device-key provisioning logic.

Where `user_subject` is not already in scope at that point in the handler, use the same value the existing device-key path derives it from — `device_key_id_from_public_key_bytes` of the submitted device public key — so a derived tenant is stable per device, matching the instance-vouched enrollment model.

- [ ] **Step 7: Write the fail-closed test for a missing registry**

This is the case where a misconfigured deploy must take onboarding down rather
than silently reverting to the file. Add to the issuer's existing onboarding
test module in `trace_upload_claim_issuer.rs`, beside the other
`/v1/onboard` refusal tests:

```rust
    #[tokio::test]
    async fn onboard_fails_closed_when_authoritative_with_no_registry() {
        // Authoritative mode with no registry must refuse, NOT fall back to
        // the file allowlist. Silent fallback would let a revoked invite
        // redeem again after a config mistake.
        let state = test_issuer_state_with_allowlist();
        let state = IssuerState {
            invite_registry: None,
            invite_registry_authoritative: true,
            ..state
        };
        let app = onboard_router(Arc::new(state));

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/onboard")
                    .header("content-type", "application/json")
                    .body(Body::from(VALID_ONBOARD_REQUEST_JSON))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(json["error"], "InviteRegistryNotConfigured");
    }
```

Reuse whatever the surrounding tests already use to build issuer state and a
valid onboard body — `test_issuer_state_with_allowlist`,
`onboard_router`, and `VALID_ONBOARD_REQUEST_JSON` are placeholders for those
existing helpers. Read the neighbouring tests first and match their names
exactly; do not introduce parallel helpers.

Run it:

```bash
cargo test -p trace-commons-server --lib onboard_fails_closed_when_authoritative_with_no_registry
```

Expected: FAIL first (the guard is not reached or the field does not exist),
then PASS once Step 6's guard is in place.

- [ ] **Step 8: Verify under CI flags and clippy, and run the whole suite**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo test -p trace-commons-server
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: no new failures against the baseline captured before this task started. The issuer's existing onboarding tests must still pass — with `invite_registry_authoritative` defaulting to false, they exercise the unchanged file path.

- [ ] **Step 9: Commit**

```bash
git add crates/trace-commons-protocol/src/onboarding.rs \
        crates/trace-commons-server/src/trace_upload_claim_issuer.rs \
        crates/trace-commons-server/src/db/mod.rs \
        crates/trace-commons-server/src/db/postgres.rs \
        crates/trace-commons-server/tests/trace_invite_registry_pg.rs
git commit -m "Redeem invites through the registry when authoritative mode is on

The cache answers first for latency; the in-transaction re-check under FOR
SHARE is what decides. Grant scopes now come from the invite rather than from
process-wide defaults."
```

---

### Task 8: Import and mint subcommands

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs`
- Test: `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`

**Interfaces:**
- Consumes: `AllowlistFile` parsing from `trace_upload_claim_allowlist`; `PgBackend::insert_invite_grant`, `list_invite_grants` (Task 3); `generate_invite_code`, `hash_invite_code`.
- Produces: `pub async fn import_file_invites(backend: &PgBackend, path: &Path, policy_label: &str) -> anyhow::Result<ImportSummary>` with `pub struct ImportSummary { pub imported: usize, pub already_present: usize, pub skipped_non_invite: usize }`; CLI flags `--import-file-invites <path>` and `--mint-invites <count>`.

- [ ] **Step 1: Write the failing import test**

Append to `crates/trace-commons-server/tests/trace_invite_registry_pg.rs`:

```rust
#[tokio::test]
async fn importing_the_same_file_twice_is_idempotent() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'import-test'",
            &[],
        )
        .await
        .expect("cleanup");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        r#"{
            "version": 1,
            "generated_at": "2026-05-17T18:00:00Z",
            "policy_label": "import-test",
            "entries": [
                {"subject_hash": "sha256:1111111111111111111111111111111111111111111111111111111111111111",
                 "tenant_id": "tenant-zaki-pilot", "note_label": "batch-1", "max_uses": 3},
                {"kind": "instance", "instance_id": "inst-1",
                 "instance_public_key": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                 "max_enrollments": 5,
                 "policy_template": {"policy_version": "v1",
                                     "allowed_consent_scopes": [], "allowed_uses": []}}
            ]
        }"#,
    )
    .expect("write file");

    let first = import_file_invites(&backend, &path, "import-test")
        .await
        .expect("first import");
    assert_eq!(first.imported, 1);
    assert_eq!(first.already_present, 0);
    assert_eq!(
        first.skipped_non_invite, 1,
        "instance entries stay in the file and must not be imported"
    );

    let created_at: chrono::DateTime<chrono::Utc> = client
        .query_one(
            "SELECT created_at FROM onboarding_invite_grants WHERE policy_label = 'import-test'",
            &[],
        )
        .await
        .expect("row")
        .get(0);

    let second = import_file_invites(&backend, &path, "import-test")
        .await
        .expect("second import");
    assert_eq!(second.imported, 0);
    assert_eq!(second.already_present, 1);

    let created_at_after: chrono::DateTime<chrono::Utc> = client
        .query_one(
            "SELECT created_at FROM onboarding_invite_grants WHERE policy_label = 'import-test'",
            &[],
        )
        .await
        .expect("row")
        .get(0);
    assert_eq!(created_at, created_at_after, "re-import must not rewrite the row");
}

#[tokio::test]
async fn imported_invites_are_fixed_mode_and_keep_their_tenant() {
    let Some(config) = registry_test_config() else {
        eprintln!("skipping: no test database configured");
        return;
    };
    let backend = PgBackend::new(&config).await.expect("backend");
    backend.run_migrations().await.expect("migrations");
    let pool = backend.trace_pool_for_test();
    let client = pool.get().await.expect("client");
    client
        .execute(
            "DELETE FROM onboarding_invite_grants WHERE policy_label = 'import-test'",
            &[],
        )
        .await
        .expect("cleanup");

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("allowlist.json");
    std::fs::write(
        &path,
        r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"import-test",
            "entries":[{"subject_hash":"sha256:1111111111111111111111111111111111111111111111111111111111111111",
            "tenant_id":"tenant-zaki-pilot","note_label":"batch-1","max_uses":3}]}"#,
    )
    .expect("write file");

    let _ = import_file_invites(&backend, &path, "import-test")
        .await
        .expect("import");

    let all = backend.list_invite_grants().await.expect("list");
    let found = all
        .iter()
        .find(|e| e.policy_label == "import-test")
        .expect("imported invite present");
    assert_eq!(found.tenant_mode, InviteTenantMode::Fixed);
    assert_eq!(found.fixed_tenant_id.as_deref(), Some("tenant-zaki-pilot"));
    assert_eq!(found.max_uses, 3);
    assert_eq!(found.note_label.as_deref(), Some("batch-1"));
    assert_eq!(found.issuance_source, "import:file");
}
```

`tempfile` must already be a dev-dependency of the crate; confirm with `grep -n tempfile crates/trace-commons-server/Cargo.toml` before running. If it is absent, stop and request approval rather than adding it.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: compile error — `import_file_invites` is not defined or not importable.

- [ ] **Step 3: Implement the import**

Add to `crates/trace-commons-server/src/trace_invite_registry.rs` (so tests can import it from the library rather than the binary):

```rust
use std::path::Path;

use crate::db::{InviteGrantInsertOutcome, InviteGrantWrite};
use crate::trace_upload_claim_allowlist::AllowlistFile;

#[derive(Debug, Clone, Default)]
pub struct ImportSummary {
    pub imported: usize,
    pub already_present: usize,
    pub skipped_non_invite: usize,
}

/// One-time migration of file invite entries into the database. Idempotent on
/// `invite_subject_hash`, so re-running after a partial failure is safe.
/// Instance entries stay in the file and are counted, not imported.
///
/// Reports counts only: no raw codes, tenant secrets, or note text.
pub async fn import_file_invites(
    backend: &PgBackend,
    path: &Path,
    policy_label: &str,
) -> anyhow::Result<ImportSummary> {
    let raw = std::fs::read_to_string(path)?;
    let file: AllowlistFile = serde_json::from_str(&raw)?;
    if file.version != 1 {
        anyhow::bail!(
            "PilotAllowlistMalformed: unsupported version {} (expected 1)",
            file.version
        );
    }

    let mut summary = ImportSummary::default();
    for entry in file.entries {
        if entry.kind != "invite" {
            summary.skipped_non_invite += 1;
            continue;
        }
        let (Some(subject_hash), Some(tenant_id)) = (entry.subject_hash, entry.tenant_id) else {
            anyhow::bail!("PilotAllowlistMalformed: invite entry missing subject_hash or tenant_id");
        };
        let write = InviteGrantWrite {
            invite_subject_hash: subject_hash,
            policy_label: policy_label.to_string(),
            tenant_mode: InviteTenantMode::Fixed,
            fixed_tenant_id: Some(tenant_id),
            tenant_template_id: None,
            policy_version: "v1".to_string(),
            allowed_consent_scopes: Vec::new(),
            allowed_uses: Vec::new(),
            max_uses: entry.max_uses,
            expires_at: None,
            issuance_source: "import:file".to_string(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: entry.note_label,
        };
        match backend.insert_invite_grant(write).await? {
            InviteGrantInsertOutcome::Inserted => summary.imported += 1,
            InviteGrantInsertOutcome::AlreadyExists => summary.already_present += 1,
            InviteGrantInsertOutcome::CredentialAlreadyBound => {
                anyhow::bail!("unexpected credential binding conflict during file import");
            }
        }
    }
    Ok(summary)
}
```

Imported invites carry empty `allowed_consent_scopes` / `allowed_uses`, which preserves today's behavior exactly: the file never carried per-invite grant fields, so the process-wide defaults applied. The `complete_onboard_with_redemption` path from Task 7 must therefore fall back to the process-wide defaults when these vectors are empty. Add that fallback and note it in a comment.

- [ ] **Step 4: Add the CLI subcommands**

In `crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs`, following the existing `--hash-invite-code` flag's shape:

```rust
// --import-file-invites <path>: one-time migration. Prints counts only.
if let Some(path) = import_file_invites_arg {
    let backend = PgBackend::new(&database_config).await?;
    backend.run_migrations().await?;
    let summary = trace_commons_server::trace_invite_registry::import_file_invites(
        &backend,
        std::path::Path::new(&path),
        &policy_label,
    )
    .await?;
    println!(
        "imported={} already_present={} skipped_non_invite={}",
        summary.imported, summary.already_present, summary.skipped_non_invite
    );
    return Ok(());
}

// --mint-invites <count>: operator batch. Prints one raw code per line to
// stdout and nothing else; redirect to a file the operator deletes after
// handing the codes out.
if let Some(count) = mint_invites_arg {
    let backend = PgBackend::new(&database_config).await?;
    backend.run_migrations().await?;
    for _ in 0..count {
        let code = trace_commons_server::trace_invite_registry::generate_invite_code();
        let write = InviteGrantWrite {
            invite_subject_hash: hash_invite_code(&code),
            policy_label: policy_label.clone(),
            tenant_mode: InviteTenantMode::Derived,
            fixed_tenant_id: None,
            tenant_template_id: Some(mint_tenant_template.clone()),
            policy_version: "v1".to_string(),
            allowed_consent_scopes: mint_consent_scopes.clone(),
            allowed_uses: mint_allowed_uses.clone(),
            max_uses: mint_max_uses,
            expires_at: mint_expires_in_days.map(|d| Utc::now() + ChronoDuration::days(d)),
            issuance_source: "operator".to_string(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: mint_note_label.clone(),
        };
        match backend.insert_invite_grant(write).await? {
            InviteGrantInsertOutcome::Inserted => println!("{code}"),
            other => anyhow::bail!("invite mint failed: {other:?}"),
        }
    }
    return Ok(());
}
```

Add the corresponding argument parsing beside the existing `--hash-invite-code` handling, with `--mint-tenant-template` (required with `--mint-invites`), `--mint-max-uses` (default 3), `--mint-expires-in-days`, `--mint-note-label`, `--mint-consent-scopes` and `--mint-allowed-uses` (comma-separated, default empty).

These run against the database directly rather than through the admin API, so an operator can bootstrap before any admin token exists.

- [ ] **Step 5: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --test trace_invite_registry_pg
```

Expected: all 13 tests PASS.

- [ ] **Step 6: Verify the subcommands manually**

```bash
RUSTFLAGS="-D warnings" cargo run -p trace-commons-server \
  --bin trace-commons-upload-claim-issuer -- --mint-invites 2 \
  --mint-tenant-template tmpl-pilot
```

Expected: exactly two 16-character codes on stdout, nothing else. Confirm with `psql` that two rows exist and that neither raw code appears in any column.

- [ ] **Step 7: Verify under CI flags and clippy**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-server/src/trace_invite_registry.rs \
        crates/trace-commons-server/src/bin/trace-commons-upload-claim-issuer.rs \
        crates/trace-commons-server/tests/trace_invite_registry_pg.rs
git commit -m "Add file-invite import and server-side invite minting

Import is idempotent on invite hash so a partial run is safe to repeat.
Minting replaces generate-pilot-invites.py, keeping entropy and hashing in the
process that enforces the invite."
```

---

### Task 9: Fail closed on file invites, and operator documentation

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_allowlist.rs`
- Modify: `docs/operator/pilot-allowlist.md`
- Delete: `scripts/operator/generate-pilot-invites.py`

**Interfaces:**
- Consumes: everything from Tasks 1-8.
- Produces: `AllowlistSnapshot::from_file_with_invite_policy(file, source_label, loaded_at, reject_invite_entries: bool)`; `AllowlistSnapshot::from_file` keeps its current signature and delegates with `false`.

- [ ] **Step 1: Write the failing test**

Add to the existing `mod tests` in `trace_upload_claim_allowlist.rs`:

```rust
    #[test]
    fn invite_entries_are_refused_once_the_registry_is_authoritative() {
        // A stale file must never be able to re-authorize an invite that was
        // revoked in the database, so once the DB is authoritative an invite
        // entry in the file is a hard parse error rather than a silent skip.
        let file: AllowlistFile = serde_json::from_str(
            r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"p",
                "entries":[{"subject_hash":"sha256:1111111111111111111111111111111111111111111111111111111111111111",
                "tenant_id":"tenant-a"}]}"#,
        )
        .expect("parse");

        let result = AllowlistSnapshot::from_file_with_invite_policy(
            file,
            "test".to_string(),
            std::time::Instant::now(),
            true,
        );
        match result {
            Err(AllowlistError::Malformed(msg)) => {
                assert!(msg.contains("invite"), "error must name the offending kind: {msg}");
            }
            other => panic!("invite entries must be refused, got {other:?}"),
        }
    }

    #[test]
    fn instance_entries_still_load_when_invites_are_refused() {
        let file: AllowlistFile = serde_json::from_str(
            r#"{"version":1,"generated_at":"2026-05-17T18:00:00Z","policy_label":"p",
                "entries":[{"kind":"instance","instance_id":"inst-1",
                "instance_public_key":"AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
                "max_enrollments":5,
                "policy_template":{"policy_version":"v1",
                "allowed_consent_scopes":[],"allowed_uses":[]}}]}"#,
        )
        .expect("parse");

        let snapshot = AllowlistSnapshot::from_file_with_invite_policy(
            file,
            "test".to_string(),
            std::time::Instant::now(),
            true,
        )
        .expect("instance entries must still load");
        assert_eq!(snapshot.subject_hashes.len(), 0);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test -p trace-commons-server --lib trace_upload_claim_allowlist
```

Expected: compile error — `from_file_with_invite_policy` does not exist.

- [ ] **Step 3: Implement the policy**

Rename the body of `AllowlistSnapshot::from_file` to `from_file_with_invite_policy` with the extra `reject_invite_entries: bool` parameter, and add this immediately inside the per-entry loop, before the existing invite-entry handling:

```rust
if entry.kind == "invite" && reject_invite_entries {
    // The database is authoritative for invites. A file entry here would be
    // able to resurrect an invite that was revoked in the database, so this
    // is a hard error rather than a silent skip.
    return Err(AllowlistError::Malformed(
        "invite entries are not permitted once the invite registry is authoritative; \
         remove kind=\"invite\" entries from the allowlist file"
            .to_string(),
    ));
}
```

Keep the original name as a thin delegate so existing callers and tests are untouched:

```rust
pub fn from_file(
    file: AllowlistFile,
    source_label: String,
    loaded_at: Instant,
) -> Result<Self, AllowlistError> {
    Self::from_file_with_invite_policy(file, source_label, loaded_at, false)
}
```

Thread the flag from `FileAllowlistSource`, which reads it from the same `TRACE_COMMONS_INVITE_REGISTRY_AUTHORITATIVE` value the issuer uses, so the two can never disagree.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test -p trace-commons-server --lib trace_upload_claim_allowlist
```

Expected: the two new tests PASS and every pre-existing allowlist test still passes.

- [ ] **Step 5: Rewrite the operator runbook**

In `docs/operator/pilot-allowlist.md`, replace the "Provisioning invite codes" and "Manual single-invite flow" sections with the following, and keep everything about instance entries and the upload-claim refusal labels as-is.

````markdown
## Provisioning invite codes

Invites live in PostgreSQL. The allowlist file no longer carries them; it
keeps only `kind: "instance"` TEE entries and the `policy_label`.

### One-time role provisioning

The registry pool runs as a narrow role that cannot bypass RLS:

```sql
DO $$ BEGIN
    IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_invite_registry_login') THEN
        CREATE ROLE trace_invite_registry_login LOGIN PASSWORD '<generated>' NOBYPASSRLS;
    END IF;
END $$;
GRANT trace_invite_registry TO trace_invite_registry_login;
```

Point `TRACE_COMMONS_INVITE_REGISTRY_DATABASE_URL` at that login role. Without
it, redemption fails closed with `InviteRegistryNotConfigured`.

### Minting a batch

```bash
trace-commons-upload-claim-issuer --mint-invites 5 \
  --mint-tenant-template pilot-2026-08 \
  --mint-max-uses 3 \
  --mint-expires-in-days 30 \
  > /tmp/tracecommons-invite-codes.txt
```

Each line is one raw code. It is never stored and cannot be recovered: only its
hash reaches the database. Delete the file after handing the codes out.

No issuer restart is needed. The admin API invalidates the in-process cache on
write; the `--mint-invites` path relies on the refresh timer, so codes minted
by CLI become redeemable within one refresh interval.

### Revoking

```bash
curl -sS -X POST \
  -H "Authorization: Bearer $ADMIN_JWT" \
  "http://127.0.0.1:$ADMIN_PORT/v1/admin/invites/$INVITE_HASH/revoke"
```

Revocation takes effect immediately: the database refuses the redemption and
the cache entry is dropped in the same request.

### Checking registry health

```bash
curl -sS -H "Authorization: Bearer $ADMIN_JWT" \
  "http://127.0.0.1:$ADMIN_PORT/v1/admin/invite-registry-status"
```

`stale: true` means the cache has not reloaded within `max_stale_seconds` and
redemption is failing closed with `InviteRegistryStale`.
````

Add a "Cutover" section documenting the four staged steps from the spec, and a row for each new error label (`InviteExpired`, `InviteRegistryNotConfigured`, `InviteRegistryStale`) in the onboarding refusal table.

- [ ] **Step 6: Delete the retired script**

```bash
git rm scripts/operator/generate-pilot-invites.py
grep -rn "generate-pilot-invites" --exclude-dir=target --exclude-dir=.git .
```

Expected: the grep returns nothing after the runbook rewrite. If `scripts/operator/pilot-bootstrap-smoke.sh` or any CI workflow references it, fix that reference — the smoke job gates every PR and must not break.

- [ ] **Step 7: Full verification**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo test -p trace-commons-server
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
bash scripts/operator/pilot-bootstrap-smoke.sh
```

Compare the test failure count against the baseline captured before Task 1. Report the actual numbers; do not claim green from a filtered run.

- [ ] **Step 8: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_allowlist.rs \
        docs/operator/pilot-allowlist.md
git commit -m "Refuse file invite entries once the registry is authoritative

A stale allowlist file must not be able to resurrect an invite revoked in the
database, so an invite entry becomes a parse error rather than a silent skip.
Retires generate-pilot-invites.py in favor of --mint-invites."
```

---

## Baseline

Before starting Task 1, capture the test baseline so later comparisons are honest:

```bash
cd .worktrees/db-authoritative-invites
cargo test -p trace-commons-server 2>&1 | tail -30 > /tmp/invite-baseline.txt
cat /tmp/invite-baseline.txt
```

Record the pass/fail counts. Every "no new failures" claim in this plan means "compared against this file".

## Deferred to the self-serve issuance design

- Any public web surface, OAuth, domain email confirmation, or outbound mail.
- Rate limiting and CAPTCHA on public issuance.
- `jti` replay enforcement on admin tokens.
- Retiring the allowlist file's instance entries.
- The `near:` allowlist source, which stays reserved and unimplemented.
