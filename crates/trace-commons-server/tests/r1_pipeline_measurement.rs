// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `scripts/operator/r1-pipeline-measurement.sql`: how many submissions carry
//! a certified redaction pipeline on the R1 allowlist.
//!
//! Two kinds of test live here.
//!
//! * **Parity, no database.** The SQL is run with `psql` against deployments
//!   that may be older than this build, so it cannot call into the protocol
//!   crate. It carries literal copies of three lists instead, and these tests
//!   fail when a copy drifts from its source: the R1 allowlist
//!   (`FULL_REDACTION_PIPELINE_VERSIONS`), the anchored tenant namespaces
//!   (`ANCHOR_NAMESPACES`), and the suffixes the server appends to a stored
//!   version (`SERVER_RESCRUB_PIPELINE_SUFFIX`,
//!   `NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX`).
//! * **PostgreSQL.** Two fresh databases, one migrated to the current schema
//!   and one stopped at V75 (before `trace_witness_certificate_evidence`),
//!   are seeded with fixtures and measured through the real script with the
//!   real `psql`. Skipped without `TRACE_COMMONS_R1_MEASUREMENT_PG_TEST_URL`,
//!   which must name a loopback server whose role may create databases.
//!
//! The evidence rows here are written directly, not through certificate
//! verification. That is the right boundary for this test: the script reads
//! what the store holds, and the store only ever holds a row that
//! `TraceWitnessCertificateEvidenceWrite::from_verified` produced
//! (`witness_evidence_pg` covers that side).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use trace_commons_protocol::admission::ANCHOR_NAMESPACES;
use trace_commons_protocol::trace_contribution::{
    FULL_REDACTION_PIPELINE_VERSIONS, NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX,
    SERVER_RESCRUB_PIPELINE_SUFFIX,
};
use trace_commons_server::db::postgres::apply_and_record_migration;
use uuid::Uuid;

const ENV: &str = "TRACE_COMMONS_R1_MEASUREMENT_PG_TEST_URL";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn script_path() -> PathBuf {
    repo_root().join("scripts/operator/r1-pipeline-measurement.sql")
}

fn script() -> String {
    std::fs::read_to_string(script_path())
        .expect("read scripts/operator/r1-pipeline-measurement.sql")
}

/// The quoted literals between `-- BEGIN <name>` and `-- END <name>`.
fn marked_list(sql: &str, name: &str) -> Vec<String> {
    let begin = format!("-- BEGIN {name}");
    let end = format!("-- END {name}");
    let start = sql
        .find(&begin)
        .unwrap_or_else(|| panic!("the script must mark its copy of {name} with `{begin}`"));
    let rest = &sql[start + begin.len()..];
    let stop = rest
        .find(&end)
        .unwrap_or_else(|| panic!("the script must close its copy of {name} with `{end}`"));
    assert_eq!(
        sql.matches(&begin).count(),
        1,
        "exactly one copy of {name} may exist in the script"
    );
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
            let inner = &line[1..line.len() - 1];
            assert!(
                !inner.contains('\''),
                "a literal in {name} must not contain a quote"
            );
            inner.to_string()
        })
        .collect()
}

#[test]
fn the_script_allowlist_is_exactly_the_protocol_allowlist() {
    let mut in_sql = marked_list(&script(), "FULL_REDACTION_PIPELINE_VERSIONS");
    let mut in_protocol: Vec<String> = FULL_REDACTION_PIPELINE_VERSIONS
        .iter()
        .map(|v| v.to_string())
        .collect();
    in_sql.sort();
    in_protocol.sort();
    assert_eq!(
        in_sql, in_protocol,
        "scripts/operator/r1-pipeline-measurement.sql must list exactly \
         FULL_REDACTION_PIPELINE_VERSIONS; update the script when the allowlist changes"
    );
}

#[test]
fn the_script_namespaces_are_exactly_the_anchor_namespaces() {
    let mut in_sql = marked_list(&script(), "ANCHOR_NAMESPACES");
    let mut in_protocol: Vec<String> = ANCHOR_NAMESPACES.iter().map(|v| v.to_string()).collect();
    in_sql.sort();
    in_protocol.sort();
    assert_eq!(in_sql, in_protocol);
}

/// The stored column is the envelope's version after the server appended its
/// own passes. The script strips them to recover what the client claimed; a
/// current suffix missing from its list would leave every claimed version
/// unrecognised. Historical spellings may stay on the list.
#[test]
fn the_script_strips_every_current_server_suffix() {
    let in_sql = marked_list(&script(), "SERVER_PIPELINE_SUFFIXES");
    for current in [
        SERVER_RESCRUB_PIPELINE_SUFFIX,
        NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX,
    ] {
        assert!(
            in_sql.iter().any(|s| s == current),
            "SERVER_PIPELINE_SUFFIXES in the script is missing `{current}`"
        );
    }
}

// ---------------------------------------------------------------------------
// PostgreSQL
// ---------------------------------------------------------------------------

fn admin_url() -> Option<String> {
    let url = std::env::var(ENV).ok()?;
    assert!(
        url.contains("@127.0.0.1")
            || url.contains("@localhost")
            || url.contains("://127.0.0.1")
            || url.contains("://localhost"),
        "{ENV} must name a loopback server: this suite creates and drops databases"
    );
    Some(url)
}

/// `url` with its database name replaced.
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

/// `url` connecting as `user`, with no password.
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

/// A database of its own, dropped by [`FreshDatabase::drop_now`].
struct FreshDatabase {
    admin: String,
    name: String,
    url: String,
}

impl FreshDatabase {
    async fn create(admin: &str) -> Self {
        let name = format!("r1_measure_test_{}", Uuid::new_v4().simple());
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

/// Every migration file with a version at or below `through`, applied in
/// version order exactly as the runner applies them.
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
    // Migrations create cluster-wide roles and grant on them; two fresh
    // databases migrating at once race on those catalog rows.
    static MIGRATING: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _serial = MIGRATING.lock().await;
    assert!(
        files.len() >= 75,
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

/// Runs the script through `psql`, read-only, and returns its rows keyed by
/// `(section, bucket)`.
fn measure(url: &str, window: Option<(&str, &str)>) -> BTreeMap<(String, String), String> {
    let output = run_psql(url, window);
    assert!(
        output.status.success(),
        "the measurement failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("utf-8 output");
    let mut rows = BTreeMap::new();
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split('|').collect();
        assert_eq!(
            fields.len(),
            3,
            "every output row is section|bucket|value: `{line}`"
        );
        let key = (fields[0].to_string(), fields[1].to_string());
        assert!(
            rows.insert(key.clone(), fields[2].to_string()).is_none(),
            "duplicate output row {key:?}"
        );
    }
    rows
}

fn run_psql(url: &str, window: Option<(&str, &str)>) -> std::process::Output {
    let mut command = Command::new("psql");
    command
        .arg(url)
        .args(["-X", "-q", "-A", "-t", "-F", "|", "-v", "ON_ERROR_STOP=1"])
        .env("PGOPTIONS", "-c default_transaction_read_only=on");
    if let Some((start, end)) = window {
        command
            .arg("-v")
            .arg(format!("window_start={start}"))
            .arg("-v")
            .arg(format!("window_end={end}"));
    }
    command.arg("-f").arg(script_path());
    command.output().expect("run psql (is it installed?)")
}

fn get<'a>(rows: &'a BTreeMap<(String, String), String>, section: &str, bucket: &str) -> &'a str {
    rows.get(&(section.to_string(), bucket.to_string()))
        .unwrap_or_else(|| panic!("missing output row {section}|{bucket}; got {rows:#?}"))
}

const NEAR_AI: &str = "ironclaw-deterministic-secret-path-v3+privacy-filter-near-ai-v1";
const SELF_HOSTED: &str = "ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1";
const SIDECAR_V2: &str = "ironclaw-deterministic-secret-path-v3+privacy-filter-sidecar-v2";
const SIDECAR_V1: &str = "ironclaw-deterministic-secret-path-v3+privacy-filter-sidecar-v1";
const DETERMINISTIC: &str = "ironclaw-deterministic-secret-path-v3";
/// Free text a contributor could have typed into its own envelope. Must never
/// reach the output.
const HOSTILE_CLAIM: &str = "/Users/alice/secret-project token=abc";

fn tenant_near() -> String {
    format!("near-{}", "a".repeat(64))
}
fn tenant_nearai() -> String {
    format!("nearai-{}", "b".repeat(64))
}
const TENANT_OTHER: &str = "tenant-invite-1";

struct Submission {
    tenant: String,
    claimed: String,
    certified: Option<&'static str>,
    received_at: &'static str,
    status_reason: Option<&'static str>,
}

async fn insert_submission(client: &tokio_postgres::Client, s: &Submission) -> Uuid {
    let id = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
            &[&s.tenant],
        )
        .await
        .expect("insert tenant");
    client
        .execute(
            "INSERT INTO trace_submissions (
                tenant_id, submission_id, trace_id, auth_principal_ref, schema_version,
                consent_policy_version, retention_policy_id, status, privacy_risk,
                redaction_pipeline_version, redaction_hash, received_at, last_status_reason)
             VALUES ($1, $2, $3, 'principal:r1', 'v1', 'v1', 'standard', 'accepted', 'low',
                     $4, 'sha256:test', $5::text::timestamptz, $6)",
            &[
                &s.tenant,
                &id,
                &Uuid::new_v4(),
                &s.claimed,
                &s.received_at,
                &s.status_reason,
            ],
        )
        .await
        .expect("insert submission");
    if let Some(certified) = s.certified {
        let certificate = serde_json::json!({
            "redacted_sha256": "d".repeat(64),
            "residual_risk_verdict": "low",
            "redaction_policy_version": certified,
            "witness_measurement": "c".repeat(64),
            "timestamp": 1_788_000_000i64,
        })
        .to_string()
        .into_bytes();
        client
            .execute(
                "INSERT INTO trace_witness_certificate_evidence (
                    tenant_id, submission_id, certificate_json, signature_header,
                    raw_body_sha256, artifact_sha256, certificate_version, inference_class,
                    issued_at)
                 VALUES ($1, $2, $3, '\\x00'::bytea, $4, $4, 1, 'unattested', NOW())",
                &[&s.tenant, &id, &certificate, &"d".repeat(64)],
            )
            .await
            .expect("insert witness evidence");
    }
    id
}

async fn map_to_session(client: &tokio_postgres::Client, tenant: &str, session: u8, ids: &[Uuid]) {
    let account = Uuid::new_v4();
    let digest = vec![session; 32];
    client
        .execute(
            "WITH account AS (INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2) \
             RETURNING tenant_id, account_id) INSERT INTO trace_account_principals \
             (tenant_id, account_id, principal_ref) SELECT tenant_id, account_id, 'principal:r1' \
             FROM account ON CONFLICT DO NOTHING",
            &[&tenant, &account],
        )
        .await
        .expect("insert account");
    client
        .execute(
            "INSERT INTO trace_source_sessions (tenant_id, account_id, session_digest) VALUES ($1, $2, $3)",
            &[&tenant, &account, &digest],
        )
        .await
        .expect("insert source session");
    for id in ids {
        client
            .execute(
                "INSERT INTO trace_submission_sessions (tenant_id, submission_id, account_id, session_digest) \
                 VALUES ($1, $2, $3, $4)",
                &[&tenant, id, &account, &digest],
            )
            .await
            .expect("map submission to session");
    }
}

fn claimed(version: &str) -> String {
    format!("{version}+{SERVER_RESCRUB_PIPELINE_SUFFIX}")
}

const IN_WINDOW: &str = "2026-09-10T12:00:00Z";
const BEFORE_WINDOW: &str = "2026-08-01T12:00:00Z";
const WINDOW: (&str, &str) = ("2026-09-01T00:00:00Z", "2026-10-01T00:00:00Z");

#[tokio::test]
async fn measures_certified_versions_from_witness_evidence() {
    let Some(admin) = admin_url() else {
        eprintln!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, i32::MAX).await;
    let client = connect(&db.url).await;

    let sub = |tenant: String, claimed: String, certified, received_at| Submission {
        tenant,
        claimed,
        certified,
        received_at,
        status_reason: None,
    };
    let near_ai = insert_submission(
        &client,
        &sub(tenant_nearai(), claimed(NEAR_AI), Some(NEAR_AI), IN_WINDOW),
    )
    .await;
    insert_submission(
        &client,
        &sub(
            tenant_near(),
            claimed(SELF_HOSTED),
            Some(SELF_HOSTED),
            IN_WINDOW,
        ),
    )
    .await;
    let sidecar_v2 = insert_submission(
        &client,
        &sub(
            TENANT_OTHER.into(),
            claimed(SIDECAR_V2),
            Some(SIDECAR_V2),
            IN_WINDOW,
        ),
    )
    .await;
    insert_submission(
        &client,
        &sub(
            TENANT_OTHER.into(),
            claimed(SIDECAR_V1),
            Some(SIDECAR_V1),
            IN_WINDOW,
        ),
    )
    .await;
    let deterministic = insert_submission(
        &client,
        &sub(
            TENANT_OTHER.into(),
            claimed(DETERMINISTIC),
            Some(DETERMINISTIC),
            IN_WINDOW,
        ),
    )
    .await;
    // The witness's startup mode name. Not a certified value.
    insert_submission(
        &client,
        &sub(
            TENANT_OTHER.into(),
            claimed(DETERMINISTIC),
            Some("full-pipeline"),
            IN_WINDOW,
        ),
    )
    .await;
    // No witness, but the client claims a full pipeline: not evidence.
    insert_submission(
        &client,
        &sub(
            tenant_nearai(),
            format!(
                "{}+{}",
                claimed(NEAR_AI),
                NEAR_AI_PII_BACKSTOP_PIPELINE_SUFFIX
            ),
            None,
            IN_WINDOW,
        ),
    )
    .await;
    insert_submission(
        &client,
        &sub(TENANT_OTHER.into(), claimed(DETERMINISTIC), None, IN_WINDOW),
    )
    .await;
    insert_submission(
        &client,
        &sub(TENANT_OTHER.into(), claimed(HOSTILE_CLAIM), None, IN_WINDOW),
    )
    .await;
    // Outside the window: must not be counted.
    insert_submission(
        &client,
        &sub(
            tenant_near(),
            claimed(NEAR_AI),
            Some(NEAR_AI),
            BEFORE_WINDOW,
        ),
    )
    .await;

    // Session A: one pass, one deterministic-only. Session B: all pass.
    map_to_session(&client, TENANT_OTHER, 1, &[sidecar_v2, deterministic]).await;
    map_to_session(&client, &tenant_nearai(), 2, &[near_ai]).await;

    let rows = measure(&db.url, Some(WINDOW));
    let raw = format!("{rows:?}");
    db.drop_now().await;

    assert!(
        !raw.contains("alice") && !raw.contains("token"),
        "free text leaked: {raw}"
    );

    assert_eq!(get(&rows, "source", "witness_evidence_table"), "present");
    assert_eq!(get(&rows, "source", "session_mapping_table"), "present");
    assert_eq!(get(&rows, "totals", "submissions"), "9");
    assert_eq!(get(&rows, "totals", "distinct_trace_ids"), "9");
    assert_eq!(get(&rows, "totals", "r1_pass"), "3");
    assert_eq!(get(&rows, "totals", "r1_fail"), "6");
    assert_eq!(
        get(&rows, "totals", "claimed_allowlisted_not_evidence"),
        "4"
    );

    assert_eq!(get(&rows, "witness", "evidence_present"), "6");
    assert_eq!(get(&rows, "witness", "evidence_present_r1_pass"), "3");
    assert_eq!(get(&rows, "witness", "evidence_absent"), "3");

    assert_eq!(
        get(
            &rows,
            "certified_version",
            &format!("allowlisted:{NEAR_AI}")
        ),
        "1"
    );
    assert_eq!(
        get(
            &rows,
            "certified_version",
            &format!("allowlisted:{SELF_HOSTED}")
        ),
        "1"
    );
    assert_eq!(
        get(
            &rows,
            "certified_version",
            &format!("allowlisted:{SIDECAR_V2}")
        ),
        "1"
    );
    assert_eq!(
        get(
            &rows,
            "certified_version",
            &format!("not_allowlisted:{SIDECAR_V1}")
        ),
        "1"
    );
    assert_eq!(
        get(
            &rows,
            "certified_version",
            &format!("not_allowlisted:{DETERMINISTIC}")
        ),
        "1"
    );
    assert_eq!(
        get(&rows, "certified_version", "not_allowlisted:full-pipeline"),
        "1"
    );

    assert_eq!(get(&rows, "namespace", "nearai-:submissions"), "2");
    assert_eq!(get(&rows, "namespace", "nearai-:r1_pass"), "1");
    assert_eq!(get(&rows, "namespace", "near-:submissions"), "1");
    assert_eq!(get(&rows, "namespace", "near-:r1_pass"), "1");
    assert_eq!(get(&rows, "namespace", "other:submissions"), "6");
    assert_eq!(get(&rows, "namespace", "other:r1_pass"), "1");

    assert_eq!(
        get(&rows, "claimed_version_not_evidence", "unrecognised"),
        "1"
    );
    assert_eq!(
        get(
            &rows,
            "claimed_version_not_evidence",
            &format!("not_allowlisted:{DETERMINISTIC}")
        ),
        "3"
    );

    assert_eq!(get(&rows, "sessions", "sessions"), "2");
    assert_eq!(get(&rows, "sessions", "every_submission_r1_pass"), "1");
    assert_eq!(get(&rows, "sessions", "some_submission_r1_pass"), "2");
    assert_eq!(get(&rows, "sessions", "submissions_without_session"), "6");
}

/// A deployment migrated only to V75 has no certified version anywhere. The
/// script must say so and leave every R1 figure empty, never report zero.
#[tokio::test]
async fn a_pre_v76_schema_reports_r1_as_unmeasurable() {
    let Some(admin) = admin_url() else {
        eprintln!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, 75).await;
    let client = connect(&db.url).await;

    for (claimed_version, reason) in [
        (claimed(NEAR_AI), Some("witness_admitted")),
        (claimed(NEAR_AI), None),
        (claimed(DETERMINISTIC), None),
    ] {
        insert_submission(
            &client,
            &Submission {
                tenant: TENANT_OTHER.into(),
                claimed: claimed_version,
                certified: None,
                received_at: IN_WINDOW,
                status_reason: reason,
            },
        )
        .await;
    }

    let rows = measure(&db.url, None);
    db.drop_now().await;

    assert_eq!(get(&rows, "source", "witness_evidence_table"), "absent");
    assert_eq!(get(&rows, "source", "session_mapping_table"), "absent");
    assert_eq!(get(&rows, "totals", "submissions"), "3");
    assert_eq!(
        get(&rows, "totals", "r1_pass"),
        "",
        "unmeasurable is NULL, not 0"
    );
    assert_eq!(get(&rows, "totals", "r1_fail"), "");
    assert_eq!(get(&rows, "witness", "evidence_present"), "");
    assert_eq!(get(&rows, "witness", "status_reason_witness_admitted"), "1");
    assert_eq!(get(&rows, "namespace", "other:r1_pass"), "");
    assert_eq!(
        get(&rows, "totals", "claimed_allowlisted_not_evidence"),
        "2"
    );
    assert!(
        !rows
            .keys()
            .any(|(section, _)| section == "certified_version" || section == "sessions"),
        "no certified or session rows without their tables: {rows:#?}"
    );
}

/// Every table is FORCE ROW LEVEL SECURITY. A role that does not bypass it
/// sees no rows without a tenant context, which would read as "nothing
/// passes". The script must refuse instead.
#[tokio::test]
async fn a_role_subject_to_rls_is_refused_rather_than_counting_zero() {
    let Some(admin) = admin_url() else {
        eprintln!("skipping: {ENV} not configured");
        return;
    };
    let db = FreshDatabase::create(&admin).await;
    migrate_through(&db.url, i32::MAX).await;
    let client = connect(&db.url).await;
    client
        .batch_execute(&format!(
            "DO $$ BEGIN
                IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'r1_measure_rls_bound') THEN
                    CREATE ROLE r1_measure_rls_bound LOGIN NOBYPASSRLS;
                END IF;
             END $$;
             GRANT CONNECT ON DATABASE {name} TO r1_measure_rls_bound;
             GRANT USAGE ON SCHEMA public TO r1_measure_rls_bound;
             GRANT SELECT ON ALL TABLES IN SCHEMA public TO r1_measure_rls_bound;",
            name = db.name
        ))
        .await
        .expect("create an RLS-bound role");
    let bound_url = with_user(&db.url, "r1_measure_rls_bound");
    let output = run_psql(&bound_url, None);
    drop(client);
    db.drop_now().await;
    assert!(
        !output.status.success(),
        "an RLS-bound role must be refused; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("R1MeasurementRoleSubjectToRls"),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
