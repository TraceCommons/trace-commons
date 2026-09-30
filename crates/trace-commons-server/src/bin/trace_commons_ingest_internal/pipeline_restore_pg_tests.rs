// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py restore-drill` (P4-D3: no launcher binary, no cargo
//! feature). Two ignored tests are the implementation, and `pipeline.py`
//! starts each one with `--ignored --exact`, with the database dump, the
//! restore, and the artifact copy in between:
//!
//! - `pipeline_restore_seed` serves the shared ingest app on
//!   `<db>_pilot` (`pipeline_http_database_url`) twice, with one index and
//!   one set of recording adapters across both lifetimes (controller ruling
//!   PF-2, as `real_http_receipt_completes_and_resumes_after_restart` does).
//!   The first lifetime has no crash point and completes the first fixture
//!   of `main`'s minimal corpus. The second has the crash point
//!   `AfterSettleSelection` and is stopped once a second receipt's Settle
//!   selection is durable. It then writes the seed fingerprint: the
//!   authoritative rows, the artifact tree, the index entry set, the pending
//!   run, and the adapter request count, all as hashes or counts.
//! - `pipeline_restore_resume` runs against the restored database, named
//!   directly by `TRACE_COMMONS_PG_TEST_DATABASE_URL` (no `_pilot`, no drop,
//!   no migration), and the copied artifact root. It proves the restore kept
//!   what the seed recorded, rebuilds the index from sealed commands, starts
//!   the app, and proves the pending run completes once with no duplicate
//!   effect, as a `NOBYPASSRLS` runtime login on tables that force RLS. Then
//!   it emits `pipeline_restore_drill`, with the safe blocker
//!   `filesystem_restore_local_only`: a local filesystem copy is not a
//!   remote restore.
//!
//! The fingerprints port `ef97a459:scripts/operator/pipeline-backup-restore-smoke.sh`:
//! the authoritative SQL (lines 134-173), run here in tenant transactions as
//! the runtime login, and the artifact tree hash (lines 191-205), taken here
//! over the files in relative-path order. Nested inside `tests` beside
//! `pipeline_http_pg_tests` and `pipeline_corpus_pg_tests`, whose
//! `pub(super)` helpers it reuses.

use super::*;

use std::path::{Path, PathBuf};

use super::pipeline_corpus_pg_tests::{
    ARTIFACT_ROOT_VAR, TEST_MASTER_KEY_VAR, fixture_envelope, id_hash, load_corpus,
    main_corpus_path, sha256_bytes, write_atomically,
};
use super::pipeline_http_pg_tests::{
    PIPELINE_HTTP_RUNTIME_ROLE, assemble_test_pipeline_service, expire_run_lease, join_within,
    pilot_runtime_login, post_trace, runtime_backend, runtime_backend_at, serve_pipeline_app,
    tenant_tx, wait_for_pipeline_ready, wait_for_run_complete, wait_for_settle_selection,
};
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::InstrumentId;
use trace_commons_server::db::postgres::registered_migrations;
use trace_commons_server::versioned_pipeline::{PipelineCrashPoint, pipeline_tenant_storage_ref};
use trace_commons_server::versioned_pipeline_bundle::MINIMAL_INDEX_ID;
use trace_commons_server::versioned_pipeline_credit::RecordingSettlementAdapter;
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_qualification::{
    PipelineCheckEmitter, PipelineCheckStatus,
};

const RESTORE_FINGERPRINT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_RESTORE_FINGERPRINT_PATH";
const RESTORE_FINGERPRINT_SCHEMA: &str = "trace_commons.pipeline_restore_fingerprint.v1";
const RESTORE_CHECK_ID: &str = "pipeline_restore_drill";

/// A local filesystem copy stands in for the object store's restore: a
/// promotion blocker, which `qualify` copies into its report.
const RESTORE_SAFE_BLOCKERS: [&str; 1] = ["filesystem_restore_local_only"];

/// The tenant the drill routes to the pipeline, and its contributor token
/// (`test_state_with_options`'s static tokens).
const RESTORE_TENANT: &str = "tenant-a";
const RESTORE_TOKEN: &str = "token-a";

/// The four authoritative tables, as the port's `fingerprint` reads them
/// (lines 134-173), in one statement. Run in a tenant transaction as the
/// runtime login, so RLS admits only that tenant's rows.
const AUTHORITATIVE_FINGERPRINT_SQL: &str = r"
    SELECT COALESCE((
          SELECT string_agg(
            concat_ws('|', run_id, submission_id, trace_id, bundle_id,
                      request_idempotency_key, request_content_hash,
                      source_object_ref_id, next_phase, state,
                      COALESCE(index_command_hash, '')),
            E'\n' ORDER BY run_id
          )
          FROM pipeline_runs
        ), '') || E'\n--outcomes--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', outcome_id, run_id, phase, bundle_id,
                      outcome_schema_id, outcome_schema_version,
                      decision::text, evidence::text, evaluation::text),
            E'\n' ORDER BY run_id, phase
          )
          FROM phase_outcomes
        ), '') || E'\n--settlements--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', tenant_id, run_id, instrument_id, atomic_units,
                      operation_ref_hash, operation_state,
                      COALESCE(result_ref_hash, ''),
                      COALESCE(credit_event_id::text, ''),
                      COALESCE(settlement_batch_id::text, ''),
                      payout_rail, payout_state),
            E'\n' ORDER BY tenant_id, run_id, instrument_id
          )
          FROM pipeline_run_settlements
        ), '') || E'\n--packages--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', tenant_id, bundle_id, package::text),
            E'\n' ORDER BY tenant_id, bundle_id
          )
          FROM pipeline_bundle_packages
        ), '')";

// ---------------------------------------------------------------------------
// Configuration and the seed fingerprint file.
// ---------------------------------------------------------------------------

/// The variables `pipeline.py restore-drill` sets for both tests.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreConfig {
    artifact_root: PathBuf,
    master_key_hex: String,
    fingerprint_path: PathBuf,
}

impl RestoreConfig {
    /// `Ok(None)` when none of the three variables is set (nothing to run);
    /// all three otherwise.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        match (
            var(ARTIFACT_ROOT_VAR),
            var(TEST_MASTER_KEY_VAR),
            var(RESTORE_FINGERPRINT_PATH_VAR),
        ) {
            (None, None, None) => Ok(None),
            (Some(artifact_root), Some(master_key_hex), Some(fingerprint_path)) => Ok(Some(Self {
                artifact_root: PathBuf::from(artifact_root),
                master_key_hex,
                fingerprint_path: PathBuf::from(fingerprint_path),
            })),
            _ => Err("restore_environment_incomplete"),
        }
    }

    fn from_env() -> Option<Self> {
        Self::from_vars(|name| std::env::var(name).ok()).unwrap_or_else(|label| panic!("{label}"))
    }
}

/// What the seed recorded and the resume must find again. Hash-only: no run
/// id, tenant id, path, or key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreFingerprint {
    schema: String,
    database_fingerprint: String,
    artifact_fingerprint: String,
    index_entry_set_hash: String,
    pending_run_id_hash: String,
    adapter_request_count: usize,
}

impl RestoreFingerprint {
    fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let fingerprint: Self =
            serde_json::from_slice(bytes).map_err(|_| "restore_fingerprint_invalid")?;
        let hashes = [
            &fingerprint.database_fingerprint,
            &fingerprint.artifact_fingerprint,
            &fingerprint.index_entry_set_hash,
            &fingerprint.pending_run_id_hash,
        ];
        if fingerprint.schema != RESTORE_FINGERPRINT_SCHEMA
            || !hashes.into_iter().all(|hash| is_sha256_label(hash))
        {
            return Err("restore_fingerprint_invalid");
        }
        Ok(fingerprint)
    }

    fn to_bytes(&self) -> Vec<u8> {
        let mut bytes = trace_commons_protocol::canonical_json::to_canonical_vec(
            &serde_json::to_value(self).expect("the restore fingerprint serialises"),
        )
        .expect("the restore fingerprint serialises");
        bytes.push(b'\n');
        bytes
    }
}

fn is_sha256_label(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    })
}

// ---------------------------------------------------------------------------
// Fingerprints.
// ---------------------------------------------------------------------------

/// The port's artifact fingerprint (lines 191-205): SHA-256 over each file's
/// relative path and the SHA-256 of its bytes. Files go in the order of
/// their relative paths (`/`-separated strings), the order
/// `pipeline_tooling.restore.artifact_fingerprint` uses too.
fn artifact_fingerprint(root: &Path) -> String {
    use sha2::Digest as _;
    fn collect(root: &Path, dir: &Path, files: &mut Vec<(String, PathBuf)>) {
        for entry in std::fs::read_dir(dir).expect("restore_artifact_root_unreadable") {
            let path = entry.expect("restore_artifact_root_unreadable").path();
            if path.is_dir() {
                collect(root, &path, files);
            } else if path.is_file() {
                let relative = path
                    .strip_prefix(root)
                    .expect("a walked path lies under its root")
                    .components()
                    .map(|component| {
                        component
                            .as_os_str()
                            .to_str()
                            .expect("restore_artifact_path_not_utf8")
                    })
                    .collect::<Vec<_>>()
                    .join("/");
                files.push((relative, path));
            }
        }
    }
    let mut files = Vec::new();
    collect(root, root, &mut files);
    files.sort();
    let mut digest = sha2::Sha256::new();
    for (relative, path) in files {
        digest.update(relative.as_bytes());
        digest.update(sha2::Sha256::digest(
            std::fs::read(path).expect("restore_artifact_unreadable"),
        ));
    }
    format!("sha256:{}", hex::encode(digest.finalize()))
}

/// The port's authoritative fingerprint over the four tables, for
/// `RESTORE_TENANT`, hashed with SHA-256.
async fn database_fingerprint(backend: &Arc<PgBackend>) -> String {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_fingerprint_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let text: String = tx
        .query_one(AUTHORITATIVE_FINGERPRINT_SQL, &[])
        .await
        .expect("restore_fingerprint_query_failed")
        .get(0);
    tx.commit().await.expect("restore_fingerprint_query_failed");
    sha256_bytes(text.as_bytes())
}

/// One run of `RESTORE_TENANT`, with the phases it has an outcome for (one
/// entry per outcome row) and its settlement leg count.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunRow {
    run_id: Uuid,
    submission_id: Uuid,
    state: String,
    next_phase: String,
    settle_selected: bool,
    phases: Vec<String>,
    settlement_count: usize,
}

async fn tenant_runs(backend: &Arc<PgBackend>) -> Vec<RunRow> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_runs_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let rows = tx
        .query(
            "SELECT r.run_id, r.submission_id, r.state, r.next_phase,
                    r.settle_selection_hash IS NOT NULL,
                    COALESCE((SELECT array_agg(o.phase ORDER BY o.phase)
                                FROM phase_outcomes o
                               WHERE o.tenant_id = r.tenant_id AND o.run_id = r.run_id),
                             '{}'::TEXT[]),
                    (SELECT COUNT(*) FROM pipeline_run_settlements s
                      WHERE s.tenant_id = r.tenant_id AND s.run_id = r.run_id)
               FROM pipeline_runs r
              WHERE r.tenant_id = $1
              ORDER BY r.created_at, r.run_id",
            &[&RESTORE_TENANT],
        )
        .await
        .expect("restore_runs_query_failed");
    tx.commit().await.expect("restore_runs_query_failed");
    rows.iter()
        .map(|row| RunRow {
            run_id: row.get(0),
            submission_id: row.get(1),
            state: row.get(2),
            next_phase: row.get(3),
            settle_selected: row.get(4),
            phases: row.get(5),
            settlement_count: usize::try_from(row.get::<_, i64>(6))
                .expect("a count is not negative"),
        })
        .collect()
}

/// Every column of `run_id`'s outcomes and settlement legs, in a stable
/// order: equal before and after the resume only if neither changed.
async fn run_effect_rows(backend: &Arc<PgBackend>, run_id: Uuid) -> String {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_run_rows_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let text: String = tx
        .query_one(
            r"SELECT COALESCE((SELECT string_agg(row_to_json(o)::text, E'\n' ORDER BY o.phase)
                                FROM phase_outcomes o
                               WHERE o.tenant_id = $1 AND o.run_id = $2), '')
                  || E'\n--settlements--\n'
                  || COALESCE((SELECT string_agg(row_to_json(s)::text, E'\n'
                                                 ORDER BY s.instrument_id)
                                 FROM pipeline_run_settlements s
                                WHERE s.tenant_id = $1 AND s.run_id = $2), '')",
            &[&RESTORE_TENANT, &run_id],
        )
        .await
        .expect("restore_run_rows_query_failed")
        .get(0);
    tx.commit().await.expect("restore_run_rows_query_failed");
    text
}

/// Rows that exist more than once where one logical effect allows one: a
/// second outcome for a run's phase, a second settlement leg for a run's
/// instrument, a credit event shared by two legs, and a second ledger event
/// for a run's instrument.
async fn duplicated_rows(backend: &Arc<PgBackend>) -> i64 {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_duplicates_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let duplicates: i64 = tx
        .query_one(
            "SELECT
                 (SELECT COUNT(*) - COUNT(DISTINCT (run_id, phase))
                    FROM phase_outcomes WHERE tenant_id = $1)
               + (SELECT COUNT(*) - COUNT(DISTINCT (run_id, instrument_id))
                    FROM pipeline_run_settlements WHERE tenant_id = $1)
               + (SELECT COUNT(credit_event_id) - COUNT(DISTINCT credit_event_id)
                    FROM pipeline_run_settlements WHERE tenant_id = $1)
               + (SELECT COUNT(*) - COUNT(DISTINCT (pipeline_run_id, instrument_id))
                    FROM trace_credit_ledger
                   WHERE tenant_id = $1 AND pipeline_run_id IS NOT NULL)",
            &[&RESTORE_TENANT],
        )
        .await
        .expect("restore_duplicates_query_failed")
        .get(0);
    tx.commit().await.expect("restore_duplicates_query_failed");
    duplicates
}

/// The recording adapters one app lifetime settles through: one per
/// instrument the minimal package or `TestAssembler`'s caps name.
struct RecordingAdapters {
    storage_rebate: Arc<RecordingSettlementAdapter>,
    trace_credit: Arc<RecordingSettlementAdapter>,
}

impl RecordingAdapters {
    fn new() -> Self {
        Self {
            storage_rebate: RecordingSettlementAdapter::new(
                InstrumentId::new("storage_rebate").expect("a valid instrument id"),
                "recording_storage_rebate_restore_test_only",
                "none",
            ),
            trace_credit: RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_restore_test_only",
                "none",
            ),
        }
    }

    fn registry(&self) -> Vec<Arc<dyn SettlementAdapter>> {
        vec![
            self.storage_rebate.clone() as Arc<dyn SettlementAdapter>,
            self.trace_credit.clone() as Arc<dyn SettlementAdapter>,
        ]
    }

    /// The run of every request either adapter received.
    fn request_runs(&self) -> Vec<Uuid> {
        self.storage_rebate
            .requests()
            .iter()
            .chain(self.trace_credit.requests().iter())
            .map(|request| request.run_id())
            .collect()
    }
}

/// An `AppState` serving `service` for `RESTORE_TENANT`, as
/// `real_http_receipt_completes_and_resumes_after_restart` builds one: the
/// runtime login is the database for both the receipt route and the
/// pipeline.
fn restore_app_state(
    state_dir: &Path,
    backend: &Arc<PgBackend>,
    artifacts: &Arc<LocalEncryptedTraceArtifactStore>,
    service: Arc<PipelineService>,
) -> Arc<AppState> {
    let mut state = test_state_with_options(
        state_dir.to_path_buf(),
        Some(backend.clone() as Arc<dyn Database>),
        Some(artifacts.clone()),
        true,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.pipeline_service = Some(service);
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[RESTORE_TENANT],
    );
    state
}

/// `TRACE_COMMONS_PG_TEST_DATABASE_URL`, taken as the restored database
/// itself: unlike `pipeline_http_database_url`, this appends no `_pilot`,
/// drops nothing, and migrates nothing. The database must already record
/// every registered migration. The runtime login is provisioned again
/// (`provision_runtime_login` is idempotent, and roles are per server,
/// so the seed's login is the one it checks). Returns `None` only when the
/// variable is unset; every failure after that panics.
async fn restored_database_url() -> Option<String> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL").ok()?;
    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect to the restored database");
    let connection = tokio::spawn(async move {
        let _ = connection.await;
    });
    let recorded: Vec<i32> = client
        .query(
            "SELECT version FROM _trace_commons_migrations ORDER BY version",
            &[],
        )
        .await
        .expect("restore_migration_history_unreadable")
        .iter()
        .map(|row| row.get(0))
        .collect();
    let mut registered: Vec<i32> = registered_migrations()
        .iter()
        .map(|(version, ..)| *version)
        .collect();
    registered.sort_unstable();
    assert_eq!(
        recorded, registered,
        "restore_database_not_at_latest_migration"
    );
    drop(client);
    let _ = connection.await;
    pilot_runtime_login::provision_runtime_login(&url, PIPELINE_HTTP_RUNTIME_ROLE, &[]).await;
    Some(url)
}

/// Every `pipeline_*` table (and `phase_outcomes`) in the database `backend`
/// reaches: `(tables, tables that enable and force RLS, tables the current
/// login may SELECT)`.
async fn pipeline_table_security(backend: &Arc<PgBackend>) -> (i64, i64, i64) {
    let row = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_rls_connection_failed")
        .query_one(
            r"SELECT COUNT(*),
                     COUNT(*) FILTER (WHERE c.relrowsecurity AND c.relforcerowsecurity),
                     COUNT(*) FILTER (WHERE has_table_privilege(c.oid, 'SELECT'))
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = 'public'
                 AND c.relkind IN ('r', 'p')
                 AND (c.relname LIKE 'pipeline\_%' OR c.relname = 'phase_outcomes')",
            &[],
        )
        .await
        .expect("restore_rls_query_failed");
    (row.get(0), row.get(1), row.get(2))
}

// ---------------------------------------------------------------------------
// The two ignored tests.
// ---------------------------------------------------------------------------

/// `pipeline.py restore-drill`, before the dump: one completed run and one
/// run stopped after its durable Settle selection, then the seed
/// fingerprint.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the seed of `pipeline.py restore-drill`: run it through that command"]
async fn pipeline_restore_seed() {
    let Some(config) = RestoreConfig::from_env() else {
        return;
    };
    let (corpus, _) = load_corpus(&main_corpus_path()).unwrap_or_else(|label| panic!("{label}"));
    let [completed_fixture, pending_fixture, ..] = corpus.fixtures.as_slice() else {
        panic!("restore_corpus_too_small");
    };
    let completed_envelope = fixture_envelope(completed_fixture).await;
    let pending_envelope = fixture_envelope(pending_fixture).await;

    let runtime = runtime_backend(8)
        .await
        .expect("restore_database_url_missing");
    let state_dir = tempfile::tempdir().expect("temp dir");
    let artifacts = test_artifact_store_with_key(&config.artifact_root, &config.master_key_hex);
    // Shared by both lifetimes (PF-2): the in-memory index and the recording
    // adapters see each other's writes only when the same instances back
    // both apps.
    let index = IsolatedPipelineIndex::new();
    let adapters = RecordingAdapters::new();
    let start = |crash_point: Option<PipelineCrashPoint>| {
        let service = assemble_test_pipeline_service(
            runtime.clone(),
            artifacts.clone(),
            index.clone(),
            adapters.registry(),
            crash_point,
        );
        restore_app_state(state_dir.path(), &runtime, &artifacts, service)
    };
    let client = reqwest::Client::new();

    // Lifetime 1: no crash point, so the first fixture completes. A crash
    // point fires at the first run that reaches it, so it cannot be set here.
    let (base, stop, server) = serve_pipeline_app(start(None)).await;
    wait_for_pipeline_ready(&client, &base).await;
    let body = serde_json::to_vec(&completed_envelope).expect("the envelope serialises");
    let (status, receipt) = post_trace(&client, &base, RESTORE_TOKEN, &body).await;
    assert!(
        status == reqwest::StatusCode::OK && receipt["status"] == "processing",
        "restore_seed_receipt_refused"
    );
    wait_for_run_complete(&runtime, RESTORE_TENANT, completed_envelope.submission_id).await;
    stop.send(())
        .expect("send shutdown to the seed's first app");
    join_within(server, 20, "the seed's first app").await;

    // Lifetime 2: stopped once the second receipt's Settle selection is
    // durable, right before the injected crash.
    let (base, stop, server) =
        serve_pipeline_app(start(Some(PipelineCrashPoint::AfterSettleSelection))).await;
    wait_for_pipeline_ready(&client, &base).await;
    let body = serde_json::to_vec(&pending_envelope).expect("the envelope serialises");
    let (status, receipt) = post_trace(&client, &base, RESTORE_TOKEN, &body).await;
    assert!(
        status == reqwest::StatusCode::OK && receipt["status"] == "processing",
        "restore_seed_receipt_refused"
    );
    wait_for_settle_selection(&runtime, RESTORE_TENANT, pending_envelope.submission_id).await;
    stop.send(())
        .expect("send shutdown to the seed's second app");
    join_within(server, 20, "the seed's second app").await;
    // P3's time shortcut, as the restart test takes it: the crashed run's
    // lease expires now instead of five minutes from now. Not a processor
    // call; a restore taken after a longer outage finds it expired anyway.
    expire_run_lease(&runtime, RESTORE_TENANT, pending_envelope.submission_id).await;

    // The seed is the shape the drill needs: one complete run with four
    // outcomes, and one leased run with a durable Settle selection, three
    // outcomes, and no settlement request yet.
    let runs = tenant_runs(&runtime).await;
    let [completed, pending] = runs.as_slice() else {
        panic!("restore_seed_run_count");
    };
    // `assert!`, not `assert_eq!`: a failure names a label, never an id.
    assert!(
        completed.submission_id == completed_envelope.submission_id,
        "restore_seed_run_order"
    );
    assert_eq!(completed.state, "complete", "restore_seed_run_not_complete");
    assert_eq!(
        completed.phases,
        ["admission", "review", "score", "settle"],
        "restore_seed_outcomes_unexpected"
    );
    assert!(
        pending.submission_id == pending_envelope.submission_id,
        "restore_seed_run_order"
    );
    assert_eq!(
        (
            pending.state.as_str(),
            pending.next_phase.as_str(),
            pending.settle_selected
        ),
        ("leased", "settle", true),
        "restore_seed_run_not_pending_at_settle"
    );
    assert_eq!(
        pending.phases,
        ["admission", "review", "score"],
        "restore_seed_outcomes_unexpected"
    );
    let request_runs = adapters.request_runs();
    assert!(
        request_runs.iter().all(|run| *run == completed.run_id)
            && request_runs.len() == completed.settlement_count
            && completed.settlement_count > 0,
        "restore_seed_adapter_requests_unexpected"
    );
    let tenant = pipeline_tenant_storage_ref(RESTORE_TENANT);
    let entry_set_hash = index.entry_set_hash(&tenant, MINIMAL_INDEX_ID);
    assert_ne!(
        entry_set_hash,
        IsolatedPipelineIndex::new().entry_set_hash(&tenant, MINIMAL_INDEX_ID),
        "restore_seed_index_empty"
    );
    let artifact_fingerprint = artifact_fingerprint(&config.artifact_root);

    let fingerprint = RestoreFingerprint {
        schema: RESTORE_FINGERPRINT_SCHEMA.to_string(),
        database_fingerprint: database_fingerprint(&runtime).await,
        artifact_fingerprint,
        index_entry_set_hash: entry_set_hash,
        pending_run_id_hash: id_hash(pending.run_id),
        adapter_request_count: request_runs.len(),
    };
    write_atomically(&config.fingerprint_path, &fingerprint.to_bytes());
}

/// `pipeline.py restore-drill`, after the restore and the artifact copy:
/// the restored state equals the seed's, and the pending run completes once
/// on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "the resume of `pipeline.py restore-drill`: run it through that command"]
async fn pipeline_restore_resume() {
    let Some(config) = RestoreConfig::from_env() else {
        return;
    };
    let seed = RestoreFingerprint::parse(
        &std::fs::read(&config.fingerprint_path).expect("restore_fingerprint_unreadable"),
    )
    .unwrap_or_else(|label| panic!("{label}"));
    let url = restored_database_url()
        .await
        .expect("restore_database_url_missing");
    // `runtime_backend_at` checks the login is neither SUPERUSER nor
    // BYPASSRLS before anything else connects.
    let runtime = runtime_backend_at(&url, 8).await;
    let (tables, forced, readable) = pipeline_table_security(&runtime).await;
    assert!(tables > 0, "restore_pipeline_tables_missing");
    assert_eq!(forced, tables, "restore_rls_not_forced");
    assert_eq!(readable, tables, "restore_privileges_lost");

    // What the restore kept.
    let database_fingerprint = database_fingerprint(&runtime).await;
    assert_eq!(
        database_fingerprint, seed.database_fingerprint,
        "restore_database_fingerprint_mismatch"
    );
    let artifact_fingerprint = artifact_fingerprint(&config.artifact_root);
    assert_eq!(
        artifact_fingerprint, seed.artifact_fingerprint,
        "restore_artifact_fingerprint_mismatch"
    );
    let before = tenant_runs(&runtime).await;
    let pending: Vec<&RunRow> = before
        .iter()
        .filter(|run| run.state != "complete")
        .collect();
    let [pending] = pending.as_slice() else {
        panic!("restore_pending_run_missing");
    };
    let pending = (*pending).clone();
    assert_eq!(
        id_hash(pending.run_id),
        seed.pending_run_id_hash,
        "restore_pending_run_missing"
    );
    let completed: Vec<RunRow> = before
        .iter()
        .filter(|run| run.state == "complete")
        .cloned()
        .collect();
    let mut completed_rows = Vec::new();
    for run in &completed {
        completed_rows.push(run_effect_rows(&runtime, run.run_id).await);
    }

    // A fresh index rebuilt from the sealed commands, before the app starts
    // (no withdrawal can race it: nothing else runs yet).
    let state_dir = tempfile::tempdir().expect("temp dir");
    let artifacts = test_artifact_store_with_key(&config.artifact_root, &config.master_key_hex);
    let index = IsolatedPipelineIndex::new();
    let adapters = RecordingAdapters::new();
    let service = assemble_test_pipeline_service(
        runtime.clone(),
        artifacts.clone(),
        index.clone(),
        adapters.registry(),
        None,
    );
    let tenant = pipeline_tenant_storage_ref(RESTORE_TENANT);
    let rebuild = service
        .rebuild_index_from_authoritative_commands(RESTORE_TENANT, index.clone())
        .await
        .expect("restore_index_rebuild_failed");
    assert_eq!(
        rebuild.command_count,
        completed.len(),
        "restore_index_rebuild_command_count"
    );
    let index_entry_set_hash = index.entry_set_hash(&tenant, MINIMAL_INDEX_ID);
    assert_eq!(
        index_entry_set_hash, seed.index_entry_set_hash,
        "restore_index_entry_set_mismatch"
    );

    // The app on the restored state resumes the pending run by itself.
    let package = service.default_package().clone();
    let state = restore_app_state(state_dir.path(), &runtime, &artifacts, service.clone());
    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    wait_for_pipeline_ready(&client, &base).await;
    wait_for_run_complete(&runtime, RESTORE_TENANT, pending.submission_id).await;
    stop.send(()).expect("send shutdown to the restored app");
    join_within(server, 20, "the restored app").await;

    // One logical effect.
    let after = tenant_runs(&runtime).await;
    assert_eq!(after.len(), before.len(), "restore_run_count_changed");
    assert!(
        after.iter().all(|run| run.state == "complete"
            && run.phases == ["admission", "review", "score", "settle"]),
        "restore_outcomes_unexpected"
    );
    for (run, rows) in completed.iter().zip(&completed_rows) {
        // `assert!`: the rows carry ids a failure message must not print.
        assert!(
            &run_effect_rows(&runtime, run.run_id).await == rows,
            "restore_completed_run_changed"
        );
    }
    let resumed = after
        .iter()
        .find(|run| run.run_id == pending.run_id)
        .expect("restore_pending_run_missing");
    let request_runs = adapters.request_runs();
    assert!(
        request_runs.iter().all(|run| *run == pending.run_id)
            && request_runs.len() == resumed.settlement_count,
        "restore_adapter_requests_unexpected"
    );
    let legs: usize = after.iter().map(|run| run.settlement_count).sum();
    let duplicate_requests = (seed.adapter_request_count + request_runs.len()).saturating_sub(legs);
    assert_eq!(
        seed.adapter_request_count + request_runs.len(),
        legs,
        "restore_adapter_request_count_mismatch"
    );
    let duplicate_effects =
        duplicated_rows(&runtime).await + i64::try_from(duplicate_requests).expect("a small count");
    assert_eq!(duplicate_effects, 0, "restore_duplicate_effect");
    let pending_runs_resumed = before
        .iter()
        .filter(|run| run.state != "complete")
        .filter(|run| {
            after
                .iter()
                .any(|done| done.run_id == run.run_id && done.state == "complete")
        })
        .count();
    assert_eq!(pending_runs_resumed, 1, "restore_pending_run_not_resumed");
    // The resumed run's index write landed: the live index equals a rebuild
    // from every sealed command, the resumed run's included.
    let rebuilt = IsolatedPipelineIndex::new();
    service
        .rebuild_index_from_authoritative_commands(RESTORE_TENANT, rebuilt.clone())
        .await
        .expect("restore_index_rebuild_failed");
    assert_eq!(
        index.entry_set_hash(&tenant, MINIMAL_INDEX_ID),
        rebuilt.entry_set_hash(&tenant, MINIMAL_INDEX_ID),
        "restore_resumed_index_mismatch"
    );
    assert_ne!(
        index.entry_set_hash(&tenant, MINIMAL_INDEX_ID),
        seed.index_entry_set_hash,
        "restore_resumed_index_unchanged"
    );

    PipelineCheckEmitter::emit_from_env(
        RESTORE_CHECK_ID,
        PipelineCheckStatus::Pass,
        Some(&package),
        &RESTORE_SAFE_BLOCKERS,
        serde_json::json!({
            "database_fingerprint": database_fingerprint,
            "artifact_fingerprint": artifact_fingerprint,
            "index_entry_set_hash": index_entry_set_hash,
            "pending_runs_resumed": pending_runs_resumed,
            "duplicate_effects": duplicate_effects,
        }),
    );
}

// ---------------------------------------------------------------------------
// Unit tests (no database).
// ---------------------------------------------------------------------------

#[test]
fn restore_config_requires_all_three_variables_or_none() {
    let vars = |set: &[(&str, &str)]| {
        let set: BTreeMap<String, String> = set
            .iter()
            .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
            .collect();
        move |name: &str| set.get(name).cloned()
    };
    assert_eq!(RestoreConfig::from_vars(vars(&[])), Ok(None));
    assert_eq!(
        RestoreConfig::from_vars(vars(&[
            (ARTIFACT_ROOT_VAR, "/tmp/root"),
            (TEST_MASTER_KEY_VAR, "00"),
            (RESTORE_FINGERPRINT_PATH_VAR, "/tmp/fingerprint.json"),
        ])),
        Ok(Some(RestoreConfig {
            artifact_root: PathBuf::from("/tmp/root"),
            master_key_hex: "00".to_string(),
            fingerprint_path: PathBuf::from("/tmp/fingerprint.json"),
        }))
    );
    for partial in [
        vec![(ARTIFACT_ROOT_VAR, "/tmp/root")],
        vec![(TEST_MASTER_KEY_VAR, "00")],
        vec![(RESTORE_FINGERPRINT_PATH_VAR, "/tmp/fingerprint.json")],
        vec![
            (ARTIFACT_ROOT_VAR, "/tmp/root"),
            (TEST_MASTER_KEY_VAR, "00"),
        ],
    ] {
        assert_eq!(
            RestoreConfig::from_vars(vars(&partial)),
            Err("restore_environment_incomplete")
        );
    }
}

/// Pinned: `pipeline_tooling.restore.artifact_fingerprint` gives the same
/// value for the same tree (`test_artifact_fingerprint_matches_the_rust_seed`),
/// so a Python check of the restored copy agrees with the seed's. `a-c.bin`
/// sorts before `a/b.bin` as a string (not as path components).
#[test]
fn artifact_fingerprint_hashes_sorted_relative_paths_and_bytes() {
    let root = tempfile::tempdir().expect("temp dir");
    std::fs::create_dir_all(root.path().join("a")).unwrap();
    std::fs::write(root.path().join("z.bin"), b"").unwrap();
    std::fs::write(root.path().join("a/b.bin"), b"one").unwrap();
    std::fs::write(root.path().join("a-c.bin"), b"two").unwrap();
    let pinned = "sha256:9be20bc7d0122e748a727bf0433d393ba50edb9e2be45a3e106ab10e52be2716";
    assert_eq!(artifact_fingerprint(root.path()), pinned);

    std::fs::write(root.path().join("a/b.bin"), b"onf").unwrap();
    assert_ne!(
        artifact_fingerprint(root.path()),
        pinned,
        "one changed byte changes it"
    );
    std::fs::write(root.path().join("a/b.bin"), b"one").unwrap();
    std::fs::rename(root.path().join("z.bin"), root.path().join("y.bin")).unwrap();
    assert_ne!(
        artifact_fingerprint(root.path()),
        pinned,
        "a renamed file changes it"
    );
}

#[test]
fn restore_fingerprint_file_refuses_unknown_fields_and_bad_hashes() {
    let hash = |label: &str| sha256_bytes(label.as_bytes());
    let fingerprint = RestoreFingerprint {
        schema: RESTORE_FINGERPRINT_SCHEMA.to_string(),
        database_fingerprint: hash("database"),
        artifact_fingerprint: hash("artifacts"),
        index_entry_set_hash: hash("index"),
        pending_run_id_hash: hash("run"),
        adapter_request_count: 1,
    };
    assert_eq!(
        RestoreFingerprint::parse(&fingerprint.to_bytes()),
        Ok(fingerprint.clone())
    );
    let edited = |edit: &dyn Fn(&mut serde_json::Value)| {
        let mut value = serde_json::to_value(&fingerprint).unwrap();
        edit(&mut value);
        RestoreFingerprint::parse(&serde_json::to_vec(&value).unwrap())
    };
    assert_eq!(
        edited(&|value| value["tenant_id"] = RESTORE_TENANT.into()),
        Err("restore_fingerprint_invalid")
    );
    assert_eq!(
        edited(&|value| value["schema"] = "trace_commons.pipeline_restore_fingerprint.v0".into()),
        Err("restore_fingerprint_invalid")
    );
    assert_eq!(
        edited(&|value| value["pending_run_id_hash"] = Uuid::nil().to_string().into()),
        Err("restore_fingerprint_invalid")
    );
    assert_eq!(
        RestoreFingerprint::parse(b"not json"),
        Err("restore_fingerprint_invalid")
    );
}
