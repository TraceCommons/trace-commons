// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `scripts/operator/legacy-invite-counts.sql`: the read-only inventory of
//! legacy invite identities that docs/operator/account-trust.md asks for
//! before account admission is enabled.
//!
//! * **Parity, no database.** The script carries a literal copy of the
//!   account namespaces; this fails when it drifts from `ANCHOR_NAMESPACES`.
//! * **PostgreSQL.** Fresh databases, one migrated to the current schema and
//!   one stopped at V80 (before V81's pooled marker and links), seeded and
//!   measured through the real script with the real `psql`. On the current
//!   schema the script's readiness-blocker total is checked against
//!   `trace_account_admission_linkage_ready()` itself, both ways. Skipped
//!   without `TRACE_COMMONS_LEGACY_INVITE_COUNTS_PG_TEST_URL`, which must name
//!   a loopback server whose role may create databases.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use trace_commons_protocol::admission::ANCHOR_NAMESPACES;
use trace_commons_server::db::postgres::apply_and_record_migration;
use uuid::Uuid;

const ENV: &str = "TRACE_COMMONS_LEGACY_INVITE_COUNTS_PG_TEST_URL";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/operator/legacy-invite-counts.sql")
}

fn script() -> String {
    std::fs::read_to_string(script_path()).expect("read scripts/operator/legacy-invite-counts.sql")
}

/// The quoted literals between `-- BEGIN <name>` and `-- END <name>`.
fn marked_list(sql: &str, name: &str) -> Vec<String> {
    let begin = format!("-- BEGIN {name}");
    let end = format!("-- END {name}");
    let start = sql
        .find(&begin)
        .unwrap_or_else(|| panic!("the script must mark its copy of {name} with `{begin}`"));
    assert_eq!(sql.matches(&begin).count(), 1, "one copy of {name} only");
    let rest = &sql[start + begin.len()..];
    let stop = rest
        .find(&end)
        .unwrap_or_else(|| panic!("the script must close its copy of {name} with `{end}`"));
    rest[..stop]
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("--"))
        .map(|line| {
            let line = line.trim_end_matches(',');
            assert!(
                line.len() >= 2 && line.starts_with('\'') && line.ends_with('\''),
                "every line of {name} must be one quoted literal, got `{line}`"
            );
            line[1..line.len() - 1].to_string()
        })
        .collect()
}

#[test]
fn the_script_namespaces_are_exactly_the_anchor_namespaces() {
    let mut in_sql = marked_list(&script(), "ANCHOR_NAMESPACES");
    let mut in_protocol: Vec<String> = ANCHOR_NAMESPACES.iter().map(|v| v.to_string()).collect();
    in_sql.sort();
    in_protocol.sort();
    assert_eq!(in_sql, in_protocol);
}

#[test]
fn the_script_is_read_only_and_rolls_back() {
    let sql = script();
    assert!(sql.contains("BEGIN TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY;"));
    assert!(sql.trim_end().ends_with("ROLLBACK;"));
    for verb in [
        "INSERT ",
        "UPDATE ",
        "DELETE ",
        "TRUNCATE ",
        "ALTER ",
        "DROP ",
        "GRANT ",
    ] {
        let code: String = sql
            .lines()
            .filter(|line| !line.trim_start().starts_with("--"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!code.contains(verb), "the script must not contain `{verb}`");
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

fn admin_url() -> Option<String> {
    let url = std::env::var(ENV).ok()?;
    assert!(
        url.contains("@127.0.0.1") || url.contains("://127.0.0.1"),
        "{ENV} must name a loopback server: this suite creates and drops databases"
    );
    Some(url)
}

fn with_database(url: &str, database: &str) -> String {
    let (base, query) = match url.split_once('?') {
        Some((base, query)) => (base, Some(query)),
        None => (url, None),
    };
    let scheme_end = base.find("://").expect("a URL") + 3;
    let host_part_end = base[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(base.len());
    let mut out = format!("{}/{database}", &base[..host_part_end]);
    if let Some(query) = query {
        out.push('?');
        out.push_str(query);
    }
    out
}

fn with_user(url: &str, user: &str) -> String {
    let scheme_end = url.find("://").expect("a URL") + 3;
    let authority_end = url[scheme_end..]
        .find('/')
        .map(|i| scheme_end + i)
        .unwrap_or(url.len());
    let authority = &url[scheme_end..authority_end];
    let host = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host)| host);
    format!(
        "{}{user}@{host}{}",
        &url[..scheme_end],
        &url[authority_end..]
    )
}

async fn connect(url: &str) -> tokio_postgres::Client {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("connect to the test server");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    client
}

struct FreshDatabase {
    admin: String,
    name: String,
    url: String,
}

impl FreshDatabase {
    async fn create(admin: &str) -> Self {
        let name = format!("legacy_counts_test_{}", Uuid::new_v4().simple());
        connect(admin)
            .await
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .await
            .expect("create a fresh database (the role needs CREATEDB)");
        let url = with_database(admin, &name);
        Self {
            admin: admin.to_string(),
            name,
            url,
        }
    }

    async fn drop_now(self) {
        let _ = connect(&self.admin)
            .await
            .batch_execute(&format!(
                "DROP DATABASE IF EXISTS {} WITH (FORCE)",
                self.name
            ))
            .await;
    }
}

async fn migrate_through(url: &str, through: i32) {
    let mut files: Vec<(i32, String, PathBuf)> = std::fs::read_dir(repo_root().join("migrations"))
        .expect("read migrations/")
        .filter_map(|entry| {
            let path = entry.ok()?.path();
            let stem = path.file_stem()?.to_str()?.to_string();
            let rest = stem.strip_prefix('V')?;
            let (version, name) = rest.split_once("__")?;
            Some((version.parse().ok()?, name.to_string(), path))
        })
        .filter(|(version, _, _)| *version <= through)
        .collect();
    files.sort_by_key(|(version, _, _)| *version);
    // Migrations create cluster-wide roles; serialize fresh databases.
    static MIGRATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serial = MIGRATING.lock().await;
    assert!(
        files.len() >= 80,
        "expected the migration history to be present"
    );
    let mut client = connect(url).await;
    client
        .batch_execute(
            "CREATE TABLE IF NOT EXISTS _trace_commons_migrations (
                version INTEGER PRIMARY KEY,
                name TEXT NOT NULL,
                applied_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
            );",
        )
        .await
        .expect("create the migration ledger");
    for (version, name, path) in files {
        let sql = std::fs::read_to_string(&path).expect("read a migration");
        apply_and_record_migration(&mut client, version, &name, &sql)
            .await
            .unwrap_or_else(|error| panic!("apply V{version} ({name}): {error}"));
    }
}

fn run_psql(url: &str) -> std::process::Output {
    Command::new("psql")
        .arg(url)
        .args(["-X", "-q", "-A", "-t", "-F", "|", "-v", "ON_ERROR_STOP=1"])
        .env("PGOPTIONS", "-c default_transaction_read_only=on")
        .arg("-f")
        .arg(script_path())
        .output()
        .expect("run psql (is it installed?)")
}

fn measure(url: &str) -> BTreeMap<(String, String), String> {
    let output = run_psql(url);
    assert!(
        output.status.success(),
        "the count failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let mut rows = BTreeMap::new();
    for line in String::from_utf8(output.stdout)
        .expect("utf-8 output")
        .lines()
        .filter(|l| !l.trim().is_empty())
    {
        let fields: Vec<&str> = line.split('|').collect();
        assert_eq!(
            fields.len(),
            3,
            "every row is section|bucket|value: `{line}`"
        );
        let key = (fields[0].to_string(), fields[1].to_string());
        assert!(
            rows.insert(key.clone(), fields[2].to_string()).is_none(),
            "duplicate output row {key:?}"
        );
    }
    rows
}

fn get<'a>(rows: &'a BTreeMap<(String, String), String>, section: &str, bucket: &str) -> &'a str {
    rows.get(&(section.to_string(), bucket.to_string()))
        .unwrap_or_else(|| panic!("missing output row {section}|{bucket}; got {rows:#?}"))
}

fn hash(label: &str) -> String {
    use sha2::{Digest, Sha256};
    format!("sha256:{}", hex::encode(Sha256::digest(label.as_bytes())))
}

async fn tenant(client: &tokio_postgres::Client, tenant: &str) {
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await
        .unwrap();
}

async fn invite(
    client: &tokio_postgres::Client,
    tenant_id: &str,
    label: &str,
    max_uses: i32,
) -> String {
    tenant(client, tenant_id).await;
    let invite = hash(label);
    client
        .execute(
            "INSERT INTO onboarding_invites (tenant_id, invite_subject_hash, max_uses, consumed_uses)
             VALUES ($1, $2, $3, 0)",
            &[&tenant_id, &invite, &max_uses],
        )
        .await
        .unwrap();
    invite
}

async fn device(client: &tokio_postgres::Client, tenant_id: &str, invite: &str, revoked: bool) {
    let id = hash(&format!("device:{}", Uuid::new_v4()));
    client
        .execute(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, invite_subject_hash, revoked_at)
             VALUES ($1, $2, 'AAAA', $3, CASE WHEN $4 THEN now() END)",
            &[&id, &tenant_id, &invite, &revoked],
        )
        .await
        .unwrap();
}

async fn account(client: &tokio_postgres::Client, tenant_id: &str, anchored: bool) -> Uuid {
    tenant(client, tenant_id).await;
    let account = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant_id, &account],
        )
        .await
        .unwrap();
    if anchored {
        client
            .execute(
                "INSERT INTO trace_near_account_anchors
                    (tenant_id, account_id, anchor_hash, sealed_account_name,
                     index_pepper_ref, account_name_key_ref)
                 VALUES ($1, $2, $3, '{}'::jsonb, 'p', 'k')",
                &[&tenant_id, &account, &hash(&format!("anchor:{account}"))],
            )
            .await
            .unwrap();
    }
    account
}

fn near(prefix: &str) -> String {
    format!("{prefix}-{}", Uuid::new_v4().simple().to_string().repeat(2))
}

/// The fleet both schemas share: legacy invite tenants of every shape, an
/// instance enrollment, and accounts in each namespace.
struct Fleet {
    individual_a: String,
    individual_a_invite: String,
}

async fn seed_fleet(client: &tokio_postgres::Client) -> Fleet {
    // Individual A: two live devices, one revoked, one legacy account.
    let individual_a = "tenant-individual-a".to_string();
    let a_invite = invite(client, &individual_a, "a", 3).await;
    device(client, &individual_a, &a_invite, false).await;
    device(client, &individual_a, &a_invite, false).await;
    device(client, &individual_a, &a_invite, true).await;
    account(client, &individual_a, false).await;
    // Individual B: one device.
    let b_invite = invite(client, "tenant-individual-b", "b", 3).await;
    device(client, "tenant-individual-b", &b_invite, false).await;
    // Pooled P: a shared event code, five devices.
    let p_invite = invite(client, "tenant-event-p", "p", 2000).await;
    for _ in 0..5 {
        device(client, "tenant-event-p", &p_invite, false).await;
    }
    // Unmarked shared code H, and a catch-all tenant M two invites route to.
    let h_invite = invite(client, "tenant-event-h", "h", 2000).await;
    device(client, "tenant-event-h", &h_invite, false).await;
    let m1 = invite(client, "tenant-catch-all", "m1", 3).await;
    invite(client, "tenant-catch-all", "m2", 3).await;
    device(client, "tenant-catch-all", &m1, false).await;
    // An instance enrollment: invite-origin device, no invite redeemed.
    tenant(client, "tenant-instance").await;
    device(client, "tenant-instance", &hash("instance"), false).await;
    // Accounts: two wallet (one anchored), one NEAR AI (anchored).
    account(client, &near("near"), true).await;
    account(client, &near("near"), false).await;
    account(client, &near("nearai"), true).await;
    Fleet {
        individual_a,
        individual_a_invite: a_invite,
    }
}

#[tokio::test]
async fn counts_on_the_current_schema_match_readiness() {
    let Some(admin) = admin_url() else {
        println!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, i32::MAX).await;
    let client = connect(&db.url).await;
    let fleet = seed_fleet(&client).await;
    client
        .execute(
            "INSERT INTO trace_legacy_invite_pooled_tenants (tenant_id, reason_label)
             VALUES ('tenant-event-p', 'shared')",
            &[],
        )
        .await
        .unwrap();
    // Individual A is linked to a wallet account, then claimed by another.
    let owner_tenant = near("near");
    let owner = account(&client, &owner_tenant, true).await;
    client
        .execute(
            "INSERT INTO trace_legacy_invite_links
                (link_id, tenant_id, account_id, legacy_tenant_id, device_key_id,
                 invite_subject_hash, nonce, issued_at, device_signature, linked_at,
                 server_kid, server_signature)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 'sig', 1, 'k', 'sig')",
            &[
                &Uuid::new_v4(),
                &owner_tenant,
                &owner,
                &fleet.individual_a,
                &hash("some-device"),
                &fleet.individual_a_invite,
                &"0".repeat(64),
            ],
        )
        .await
        .unwrap();
    let claimant_tenant = near("nearai");
    let claimant = account(&client, &claimant_tenant, true).await;
    client
        .execute(
            "INSERT INTO trace_legacy_invite_link_conflicts (tenant_id, account_id, legacy_tenant_id)
             VALUES ($1, $2, $3)",
            &[&claimant_tenant, &claimant, &fleet.individual_a],
        )
        .await
        .unwrap();
    // A claim left over on a tenant later marked pooled: pooled tenants are
    // never ambiguous, so this must count nowhere.
    client
        .execute(
            "INSERT INTO trace_legacy_invite_link_conflicts (tenant_id, account_id, legacy_tenant_id)
             VALUES ($1, $2, 'tenant-event-p')",
            &[&claimant_tenant, &claimant],
        )
        .await
        .unwrap();

    let rows = measure(&db.url);
    let ready: bool = client
        .query_one("SELECT trace_account_admission_linkage_ready()", &[])
        .await
        .unwrap()
        .get(0);

    assert_eq!(get(&rows, "source", "v81_legacy_invite_link"), "1");
    assert_eq!(get(&rows, "legacy_invite_tenants", "total"), "5");
    assert_eq!(get(&rows, "legacy_invite_tenants", "pooled"), "1");
    assert_eq!(get(&rows, "legacy_invite_tenants", "individual"), "4");
    assert_eq!(get(&rows, "legacy_invite_tenants", "unclassified"), "0");
    assert_eq!(
        get(&rows, "legacy_invite_tenants", "review_before_linking"),
        "2",
        "the unmarked shared code and the catch-all tenant; not the pooled one"
    );
    assert_eq!(
        get(
            &rows,
            "legacy_invite_tenants",
            "instance_enrolled_not_invite"
        ),
        "1"
    );
    assert_eq!(get(&rows, "devices", "individual:active"), "5");
    assert_eq!(get(&rows, "devices", "individual:revoked"), "1");
    assert_eq!(get(&rows, "devices", "pooled:active"), "5");
    assert_eq!(get(&rows, "devices_per_tenant", "individual:1"), "3");
    assert_eq!(get(&rows, "devices_per_tenant", "individual:2"), "1");
    assert_eq!(get(&rows, "devices_per_tenant", "pooled:4-10"), "1");
    assert_eq!(get(&rows, "accounts", "wallet:open"), "3");
    assert_eq!(get(&rows, "accounts", "wallet:anchored"), "2");
    assert_eq!(get(&rows, "accounts", "near_ai:open"), "2");
    assert_eq!(get(&rows, "accounts", "near_ai:anchored"), "2");
    assert_eq!(get(&rows, "accounts", "legacy:open"), "1");
    assert_eq!(get(&rows, "links", "individual:linked"), "1");
    assert_eq!(get(&rows, "links", "individual:unlinked"), "3");
    assert_eq!(get(&rows, "links", "pooled:linked"), "0");
    assert_eq!(get(&rows, "links", "accounts_holding_a_link"), "1");
    assert_eq!(
        get(
            &rows,
            "ambiguous",
            "tenants_claimed_by_more_than_one_account"
        ),
        "1"
    );
    assert_eq!(get(&rows, "ambiguous", "open_second_claims"), "1");
    assert_eq!(get(&rows, "readiness_blockers", "4_ambiguous_tenants"), "1");
    assert_eq!(get(&rows, "readiness_blockers", "total"), "1");
    assert!(!ready, "the function agrees: one ambiguity blocks");

    client
        .execute(
            // Only the individual tenant's claim: the pooled tenant's stays
            // open, and must not block either.
            "UPDATE trace_legacy_invite_link_conflicts SET resolved_at = now()
              WHERE legacy_tenant_id = $1",
            &[&fleet.individual_a],
        )
        .await
        .unwrap();
    let rows = measure(&db.url);
    let ready: bool = client
        .query_one("SELECT trace_account_admission_linkage_ready()", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(get(&rows, "readiness_blockers", "total"), "0");
    assert!(
        ready,
        "the function agrees: coexistence alone does not block"
    );

    // Nothing identifying reaches the output.
    let rendered = format!("{rows:?}");
    for secret in [
        "tenant-individual-a",
        "tenant-event",
        "tenant-catch-all",
        "sha256:",
        &owner.to_string(),
    ] {
        assert!(!rendered.contains(secret), "`{secret}` leaked: {rendered}");
    }
    drop(client);
    db.drop_now().await;
}

/// Before V81 there is no pooled marker, link or claim. Those figures are
/// NULL, never 0, and every legacy invite tenant is `unclassified`.
#[tokio::test]
async fn a_pre_v81_schema_reports_links_as_unmeasurable() {
    let Some(admin) = admin_url() else {
        println!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, 80).await;
    let client = connect(&db.url).await;
    seed_fleet(&client).await;
    let rows = measure(&db.url);
    drop(client);
    db.drop_now().await;

    assert_eq!(get(&rows, "source", "v81_legacy_invite_link"), "0");
    assert_eq!(get(&rows, "legacy_invite_tenants", "total"), "5");
    assert_eq!(
        get(&rows, "legacy_invite_tenants", "pooled"),
        "",
        "NULL, not 0"
    );
    assert_eq!(get(&rows, "legacy_invite_tenants", "individual"), "");
    assert_eq!(get(&rows, "legacy_invite_tenants", "unclassified"), "5");
    assert_eq!(
        get(&rows, "legacy_invite_tenants", "review_before_linking"),
        "3"
    );
    assert_eq!(get(&rows, "devices", "unclassified:active"), "10");
    assert_eq!(get(&rows, "accounts", "wallet:open"), "2");
    assert_eq!(get(&rows, "accounts", "near_ai:open"), "1");
    assert_eq!(get(&rows, "links", "individual:linked"), "");
    assert_eq!(
        get(
            &rows,
            "ambiguous",
            "tenants_claimed_by_more_than_one_account"
        ),
        ""
    );
    assert_eq!(get(&rows, "readiness_blockers", "total"), "");
    assert_eq!(
        get(
            &rows,
            "readiness_blockers",
            "3_legacy_accounts_without_an_invite"
        ),
        "0"
    );
}

/// Every table is FORCE ROW LEVEL SECURITY: an RLS-bound role would count
/// zero of everything. The script must refuse instead.
#[tokio::test]
async fn a_role_subject_to_rls_is_refused_rather_than_counting_zero() {
    let Some(admin) = admin_url() else {
        println!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, i32::MAX).await;
    let client = connect(&db.url).await;
    client
        .batch_execute(&format!(
            "DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'legacy_counts_rls_bound') THEN
                    CREATE ROLE legacy_counts_rls_bound LOGIN NOBYPASSRLS;
                END IF;
             END $$;
             GRANT CONNECT ON DATABASE {name} TO legacy_counts_rls_bound;
             GRANT USAGE ON SCHEMA public TO legacy_counts_rls_bound;
             GRANT SELECT ON ALL TABLES IN SCHEMA public TO legacy_counts_rls_bound;",
            name = db.name
        ))
        .await
        .expect("create an RLS-bound role");
    let output = run_psql(&with_user(&db.url, "legacy_counts_rls_bound"));
    drop(client);
    db.drop_now().await;
    assert!(
        !output.status.success(),
        "an RLS-bound role must be refused; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("LegacyInviteCountsRoleSubjectToRls"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
