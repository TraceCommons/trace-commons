// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `pipeline.py restore-drill` (P4-D3: no launcher binary, no cargo
//! feature). Two ignored tests are the implementation, and `pipeline.py`
//! starts each one with `--ignored --exact`, with the database dump, the
//! restore, and the artifact copy in between:
//!
//! - `pipeline_restore_seed` serves the shared ingest app on
//!   `<db>_pilot` (`pipeline_http_database_url`) twice, with the
//!   compatibility bundle and one index and one `trace_credit` recording
//!   adapter across both lifetimes (controller ruling PF-2, as
//!   `real_http_receipt_completes_and_resumes_after_restart` does; ruling
//!   T10-4: the bundle whose Trace Credit leg calls its adapter and then
//!   writes a ledger event). The first lifetime has no crash point and
//!   completes, and credits, the first fixture of `main`'s minimal corpus.
//!   The first lifetime also completes, and credits, the same fixture for a
//!   second tenant (`SECOND_TENANT`), and waits until both tenants' credit
//!   events are in `main`'s audit chain.
//!   The second has the crash point `AfterSettleSelection` and is stopped
//!   once a second receipt's Settle selection is durable. It then writes the
//!   seed fingerprint: the authoritative rows (the Trace Credit ledger
//!   included), the artifact tree, the index entry set, the pending run, the
//!   adapter request count, the completed run's leg and credit event
//!   counts, the number of tables `TRACE_COMMONS_RLS_TABLES` names, the
//!   runtime login's privilege set, every tenant's rows in those tables, and
//!   the audit events `main`'s verifier accepts, all as hashes or counts.
//! - `pipeline_restore_resume` runs against the restored database, named
//!   directly by `TRACE_COMMONS_PG_TEST_DATABASE_URL` (no `_pilot`, no drop,
//!   no migration), and the copied artifact root. Before anything resumes it
//!   proves the restore kept what the seed recorded: every table in
//!   `TRACE_COMMONS_RLS_TABLES` enables and forces RLS with the tenant
//!   policy's predicate on `trace_current_tenant_id()` (the diagnostic
//!   `trace_corpus_pg_rls.rs` reads), the runtime login holds the same
//!   privileges (every type, every table, column, sequence, and function),
//!   every tenant's rows are the seed's, and every tenant's audit chain
//!   verifies (Zaki's review of #1166, Major 2). It then rebuilds the index
//!   from sealed commands, starts
//!   the app, and proves the pending run completes once, with its durable
//!   selection honored, one adapter request and one ledger event per leg,
//!   and no duplicate effect, as a `NOBYPASSRLS` runtime login on tables
//!   that force RLS. Then
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
    ARTIFACT_ROOT_VAR, COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS, TEST_MASTER_KEY_VAR,
    fixture_envelope, id_hash, load_corpus, main_corpus_path, sha256_bytes, write_atomically,
};
use super::pipeline_http_pg_tests::{
    PIPELINE_HTTP_RUNTIME_ROLE, PassThroughPipelinePrivacyBoundary, account_owner_backend,
    assemble_compatibility_pipeline_service_with, expire_run_lease, join_within, mains_database,
    mains_database_at, pilot_runtime_login, post_trace, qualification_candidate_package,
    runtime_backend, runtime_backend_at, serve_pipeline_app, tenant_tx, wait_for_pipeline_ready,
    wait_for_run_complete, wait_for_settle_selection,
};
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::{InstrumentId, Phase};
use trace_commons_server::db::postgres::{TRACE_COMMONS_RLS_TABLES, registered_migrations};
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

/// A second routed tenant with one completed run, so the restore is checked
/// across tenants and not only for `RESTORE_TENANT`.
const SECOND_TENANT: &str = "tenant-b";
const SECOND_TOKEN: &str = "token-b";

/// A third tenant, on no list, that holds only the activation state of
/// delivery PR 5 (final fix wave G23): a routing row and its event (a
/// containment), a suspended Score policy and its intervention, a
/// qualification row, and an index rebuild fence. The seed writes one row of
/// each (`seed_activation_state`), so the restore is checked over rows in the
/// five tables and not over empty sets; the resume requires each row
/// (`require_activation_state`), and the tenant fingerprint covers their
/// content. It takes no receipt, so it changes nothing the drill measures
/// for the other two tenants.
const ACTIVATION_TENANT: &str = "tenant-c";

/// The tables `seed_activation_state` writes one row in, for
/// `ACTIVATION_TENANT`.
const ACTIVATION_STATE_TABLES: [&str; 5] = [
    "pipeline_tenant_routing",
    "pipeline_activation_events",
    "pipeline_policy_interventions",
    "pipeline_bundle_qualifications",
    "pipeline_index_rebuild_fences",
];

/// Every row-level security policy in the `public` schema, one line each,
/// sorted: its table, name, command, permissive or restrictive, its roles'
/// names sorted (`PUBLIC` for oid 0), and its `USING` and `WITH CHECK`
/// expressions as `pg_get_expr` reads them back. The named tenant policy's
/// predicate is not the whole isolation: PostgreSQL ORs permissive
/// policies, so a policy added beside it (`USING (true)`), or one whose
/// roles widen to `PUBLIC`, opens the table too (review of this wave, I1).
const RLS_POLICY_SET_SQL: &str = r"
    WITH entries AS (
        SELECT concat_ws('|',
                   c.relname::TEXT,
                   pol.polname::TEXT,
                   pol.polcmd::TEXT,
                   CASE WHEN pol.polpermissive THEN 'permissive' ELSE 'restrictive' END,
                   (SELECT string_agg(role_name, ',' ORDER BY role_name)
                      FROM (SELECT CASE WHEN r = 0 THEN 'PUBLIC' ELSE r::regrole::TEXT END
                                   AS role_name
                              FROM unnest(pol.polroles) AS r) AS roles),
                   COALESCE(pg_get_expr(pol.polqual, pol.polrelid), ''),
                   COALESCE(pg_get_expr(pol.polwithcheck, pol.polrelid), '')) AS entry
          FROM pg_policy pol
          JOIN pg_class c ON c.oid = pol.polrelid
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public'
    )
    SELECT COALESCE(string_agg(entry, E'\n' ORDER BY entry), ''), COUNT(*) FROM entries";

/// Every table in the `public` schema, one line each, sorted: its name and
/// its two row-level security flags (`relrowsecurity`,
/// `relforcerowsecurity`). The per-table checks read only the pipeline
/// tables and `TRACE_COMMONS_RLS_TABLES`, and the policy set lives in
/// `pg_policy`, which holds neither flag; this covers every other table
/// (Zaki's re-review of #1166, Medium), a table that appears or goes, and a
/// table dropped from `TRACE_COMMONS_RLS_TABLES` whose flags change.
const RLS_FLAG_SET_SQL: &str = r"
    WITH entries AS (
        SELECT concat_ws('|',
                   c.relname::TEXT,
                   c.relrowsecurity::TEXT,
                   c.relforcerowsecurity::TEXT) AS entry
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
    )
    SELECT COALESCE(string_agg(entry, E'\n' ORDER BY entry), ''), COUNT(*) FROM entries";

/// Every privilege the current login holds in the `public` schema, one line
/// each, sorted: each privilege type on each table, view, and foreign table;
/// each column privilege no table-level grant already gives; each sequence
/// privilege; `EXECUTE` on each function; and the schema's own `USAGE` and
/// `CREATE`. Run as the runtime login, so grants through its group
/// memberships and to `PUBLIC` count, and a `REVOKE` of any one of them
/// changes the text.
const RUNTIME_PRIVILEGES_SQL: &str = r"
    WITH relations AS (
        SELECT c.oid, c.relname::TEXT AS relname, c.relkind
          FROM pg_class c
          JOIN pg_namespace n ON n.oid = c.relnamespace
         WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p', 'v', 'm', 'f', 'S')
    ), entries AS (
        SELECT 'table|' || r.relname || '|' || p.privilege AS entry
          FROM relations r
         CROSS JOIN unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'DELETE', 'TRUNCATE',
                                 'REFERENCES', 'TRIGGER']) AS p(privilege)
         WHERE r.relkind <> 'S' AND has_table_privilege(r.oid, p.privilege)
        UNION ALL
        SELECT 'column|' || r.relname || '|' || a.attname::TEXT || '|' || p.privilege
          FROM relations r
          JOIN pg_attribute a ON a.attrelid = r.oid AND a.attnum > 0 AND NOT a.attisdropped
         CROSS JOIN unnest(ARRAY['SELECT', 'INSERT', 'UPDATE', 'REFERENCES']) AS p(privilege)
         WHERE r.relkind <> 'S'
           AND has_column_privilege(r.oid, a.attnum, p.privilege)
           AND NOT has_table_privilege(r.oid, p.privilege)
        UNION ALL
        SELECT 'sequence|' || r.relname || '|' || p.privilege
          FROM relations r
         CROSS JOIN unnest(ARRAY['USAGE', 'SELECT', 'UPDATE']) AS p(privilege)
         WHERE r.relkind = 'S' AND has_sequence_privilege(r.oid, p.privilege)
        UNION ALL
        SELECT 'function|' || f.oid::regprocedure::TEXT || '|EXECUTE'
          FROM pg_proc f
          JOIN pg_namespace n ON n.oid = f.pronamespace
         WHERE n.nspname = 'public' AND has_function_privilege(f.oid, 'EXECUTE')
        UNION ALL
        SELECT 'schema|public|' || p.privilege
          FROM unnest(ARRAY['USAGE', 'CREATE']) AS p(privilege)
         WHERE has_schema_privilege('public', p.privilege)
    )
    SELECT COALESCE(string_agg(entry, E'\n' ORDER BY entry), ''), COUNT(*) FROM entries";

/// Every table the pipeline migrations create (V92 to V95, V105 to V108,
/// V110, V111, and V113), sorted: the set the resume requires to enable and
/// force RLS in the restored database. A new pipeline table must be added
/// here, or the drill fails.
const PIPELINE_TABLES: [&str; 20] = [
    "phase_outcomes",
    "pipeline_activation_events",
    "pipeline_active_bundles",
    "pipeline_admission_usage",
    "pipeline_attempt_artifacts",
    "pipeline_bundle_packages",
    "pipeline_bundle_policy_status",
    "pipeline_bundle_qualifications",
    "pipeline_export_snapshot_items",
    "pipeline_export_snapshots",
    "pipeline_index_invalidations",
    "pipeline_index_rebuild_fences",
    "pipeline_policy_interventions",
    "pipeline_receipt_artifacts",
    "pipeline_receipt_ownership",
    "pipeline_review_assessments",
    "pipeline_review_claims",
    "pipeline_run_settlements",
    "pipeline_runs",
    "pipeline_tenant_routing",
];

/// The four authoritative tables, as the port's `fingerprint` reads them
/// (lines 134-173), then the tenant's Trace Credit ledger rows (ruling
/// T10-4), in one statement. Run in a tenant transaction as the runtime
/// login, so RLS admits only that tenant's rows.
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
        ), '') || E'\n--ledger--\n' || COALESCE((
          SELECT string_agg(
            concat_ws('|', tenant_id, credit_event_id, submission_id, trace_id,
                      credit_account_ref, event_type, points_delta, reason,
                      COALESCE(external_ref, ''), actor_principal_ref, actor_role,
                      settlement_state, (occurred_at AT TIME ZONE 'UTC')::text,
                      COALESCE(pipeline_run_id::text, ''),
                      COALESCE(score_outcome_id::text, ''),
                      COALESCE(instrument_id, '')),
            E'\n' ORDER BY tenant_id, credit_event_id
          )
          FROM trace_credit_ledger
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
/// id, tenant id, path, or key. The two `completed_*` counts are the
/// completed run's settlement legs and Trace Credit ledger events: the
/// pending run must reach the same after the resume (same bundle, same
/// award).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RestoreFingerprint {
    schema: String,
    database_fingerprint: String,
    artifact_fingerprint: String,
    index_entry_set_hash: String,
    pending_run_id_hash: String,
    adapter_request_count: usize,
    completed_settlement_count: usize,
    completed_credit_event_count: usize,
    /// `TRACE_COMMONS_RLS_TABLES`'s length, all isolated in the seed.
    rls_table_count: usize,
    rls_policy_set_hash: String,
    rls_policy_count: usize,
    /// Every `public` table's two RLS flags (`RLS_FLAG_SET_SQL`), hashed,
    /// and how many tables.
    rls_flag_set_hash: String,
    rls_flag_table_count: usize,
    runtime_privilege_set_hash: String,
    runtime_privilege_count: usize,
    tenant_fingerprint: String,
    tenant_count: usize,
    /// Hashed audit events `main`'s verifier chained, across every tenant.
    audit_event_count: usize,
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
            &fingerprint.runtime_privilege_set_hash,
            &fingerprint.tenant_fingerprint,
            &fingerprint.rls_policy_set_hash,
            &fingerprint.rls_flag_set_hash,
        ];
        if fingerprint.schema != RESTORE_FINGERPRINT_SCHEMA
            || !hashes.into_iter().all(|hash| is_sha256_label(hash))
            || fingerprint.completed_settlement_count == 0
            || fingerprint.completed_credit_event_count == 0
            || fingerprint.rls_table_count == 0
            || fingerprint.rls_policy_count == 0
            || fingerprint.rls_flag_table_count == 0
            || fingerprint.runtime_privilege_count == 0
            || fingerprint.tenant_count < 2
            || fingerprint.audit_event_count == 0
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

/// One run of a tenant, with the phases it has an outcome for (one
/// entry per outcome row), its settlement leg count, its Trace Credit legs,
/// its ledger events, and the legs whose `credit_event_id` names one of
/// those events.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RunRow {
    run_id: Uuid,
    submission_id: Uuid,
    state: String,
    next_phase: String,
    settle_selected: bool,
    phases: Vec<String>,
    settlement_count: usize,
    trace_credit_legs: usize,
    credit_events: usize,
    linked_credit_legs: usize,
}

impl RunRow {
    /// Exactly one ledger event for each Trace Credit leg, each named by its
    /// leg, and at least one such leg.
    fn credited_once(&self) -> bool {
        self.trace_credit_legs > 0
            && self.credit_events == self.trace_credit_legs
            && self.linked_credit_legs == self.trace_credit_legs
    }
}

async fn tenant_runs(backend: &Arc<PgBackend>, tenant_id: &str) -> Vec<RunRow> {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_runs_connection_failed");
    let tx = tenant_tx(&mut client, tenant_id).await;
    let rows = tx
        .query(
            "SELECT r.run_id, r.submission_id, r.state, r.next_phase,
                    r.settle_selection_hash IS NOT NULL,
                    COALESCE((SELECT array_agg(o.phase ORDER BY o.phase)
                                FROM phase_outcomes o
                               WHERE o.tenant_id = r.tenant_id AND o.run_id = r.run_id),
                             '{}'::TEXT[]),
                    (SELECT COUNT(*) FROM pipeline_run_settlements s
                      WHERE s.tenant_id = r.tenant_id AND s.run_id = r.run_id),
                    (SELECT COUNT(*) FROM pipeline_run_settlements s
                      WHERE s.tenant_id = r.tenant_id AND s.run_id = r.run_id
                        AND s.instrument_id = 'trace_credit'),
                    (SELECT COUNT(*) FROM trace_credit_ledger l
                      WHERE l.tenant_id = r.tenant_id AND l.pipeline_run_id = r.run_id),
                    (SELECT COUNT(*) FROM pipeline_run_settlements s
                       JOIN trace_credit_ledger l
                         ON l.tenant_id = s.tenant_id
                        AND l.credit_event_id = s.credit_event_id
                        AND l.pipeline_run_id = s.run_id
                      WHERE s.tenant_id = r.tenant_id AND s.run_id = r.run_id)
               FROM pipeline_runs r
              WHERE r.tenant_id = $1
              ORDER BY r.created_at, r.run_id",
            &[&tenant_id],
        )
        .await
        .expect("restore_runs_query_failed");
    tx.commit().await.expect("restore_runs_query_failed");
    let count = |row: &tokio_postgres::Row, index: usize| {
        usize::try_from(row.get::<_, i64>(index)).expect("a count is not negative")
    };
    rows.iter()
        .map(|row| RunRow {
            run_id: row.get(0),
            submission_id: row.get(1),
            state: row.get(2),
            next_phase: row.get(3),
            settle_selected: row.get(4),
            phases: row.get(5),
            settlement_count: count(row, 6),
            trace_credit_legs: count(row, 7),
            credit_events: count(row, 8),
            linked_credit_legs: count(row, 9),
        })
        .collect()
}

/// Every column of `run_id`'s outcomes, settlement legs, and Trace Credit
/// ledger rows, in a stable order: equal before and after the resume only if
/// none of them changed.
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
                                WHERE s.tenant_id = $1 AND s.run_id = $2), '')
                  || E'\n--ledger--\n'
                  || COALESCE((SELECT string_agg(row_to_json(l)::text, E'\n'
                                                 ORDER BY l.credit_event_id)
                                 FROM trace_credit_ledger l
                                WHERE l.tenant_id = $1 AND l.pipeline_run_id = $2), '')",
            &[&RESTORE_TENANT, &run_id],
        )
        .await
        .expect("restore_run_rows_query_failed")
        .get(0);
    tx.commit().await.expect("restore_run_rows_query_failed");
    text
}

/// Every column of `run_id`'s committed Admission, Review, and Score
/// outcomes, and its durable Settle selection and hash: what
/// `AfterSettleSelection` left behind, which the resume must honor
/// unchanged (ruling T10-6).
async fn committed_selection_rows(backend: &Arc<PgBackend>, run_id: Uuid) -> String {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_selection_rows_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let text: String = tx
        .query_one(
            r"SELECT COALESCE((SELECT string_agg(row_to_json(o)::text, E'\n' ORDER BY o.phase)
                                FROM phase_outcomes o
                               WHERE o.tenant_id = $1 AND o.run_id = $2
                                 AND o.phase IN ('admission', 'review', 'score')), '')
                  || E'\n--selection--\n'
                  || COALESCE((SELECT settle_selection_hash || '|' || settle_selection::text
                                 FROM pipeline_runs
                                WHERE tenant_id = $1 AND run_id = $2), '')",
            &[&RESTORE_TENANT, &run_id],
        )
        .await
        .expect("restore_selection_rows_query_failed")
        .get(0);
    tx.commit()
        .await
        .expect("restore_selection_rows_query_failed");
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

/// The one recording adapter an app lifetime settles through: the
/// compatibility bundle pins only `trace_credit`.
struct RecordingAdapters {
    trace_credit: Arc<RecordingSettlementAdapter>,
}

impl RecordingAdapters {
    fn new() -> Self {
        Self {
            trace_credit: RecordingSettlementAdapter::new(
                InstrumentId::trace_credit(),
                "recording_trace_credit_restore_test_only",
                "none",
            ),
        }
    }

    fn registry(&self) -> Vec<Arc<dyn SettlementAdapter>> {
        vec![self.trace_credit.clone() as Arc<dyn SettlementAdapter>]
    }

    /// The run of every request the adapter received.
    fn request_runs(&self) -> Vec<Uuid> {
        self.trace_credit
            .requests()
            .iter()
            .map(|request| request.run_id())
            .collect()
    }
}

/// The compatibility bundle's service (the `compatibility` bundle of
/// `pipeline.py run`: a 2_500_000 microcredit `NoveltyUtility` delta, the
/// pass-through privacy boundary), on the runtime login, over `artifacts`,
/// with the caller's index, adapters, and crash point.
fn compatibility_service(
    runtime: &Arc<PgBackend>,
    artifacts: &Arc<LocalEncryptedTraceArtifactStore>,
    index: &Arc<IsolatedPipelineIndex>,
    adapters: &RecordingAdapters,
    crash_point: Option<PipelineCrashPoint>,
) -> Arc<PipelineService> {
    assemble_compatibility_pipeline_service_with(
        runtime.clone(),
        &ConfiguredTraceArtifactStore::legacy(artifacts.clone()),
        index.clone(),
        COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS,
        Arc::new(PassThroughPipelinePrivacyBoundary),
        adapters.registry(),
        crash_point,
    )
}

/// An `AppState` serving `service` for `RESTORE_TENANT` and `SECOND_TENANT`, as the corpus
/// harness builds one: `main`'s database (`mains`) and the pipeline service
/// (built by the caller) both run on the runtime login, with the pilot
/// ingest login's privileges only, never the owner superuser (PR 3's
/// `mains_database`).
fn restore_app_state(
    state_dir: &Path,
    mains: &Arc<dyn Database>,
    artifacts: &Arc<LocalEncryptedTraceArtifactStore>,
    runtime: &Arc<PgBackend>,
    service: Arc<PipelineService>,
) -> Arc<AppState> {
    let mut state = test_state_with_options(
        state_dir.to_path_buf(),
        Some(mains.clone()),
        Some(artifacts.clone()),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::make_mut(&mut state);
    state_mut.pipeline_service = Some(service);
    state_mut.pipeline_activation = routing_store(runtime);
    state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
        TraceTenantRolloutFeature::PipelineReceipts,
        &[RESTORE_TENANT, SECOND_TENANT],
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
    // The pilot ingest login's two groups, as the HTTP suite provisions this
    // same login (`pipeline_http_database_url`).
    pilot_runtime_login::provision_runtime_login(
        &url,
        PIPELINE_HTTP_RUNTIME_ROLE,
        &["trace_account_admission_runtime"],
    )
    .await;
    Some(url)
}

/// Every `pipeline_*` table (and `phase_outcomes`) in the database `backend`
/// reaches, sorted by name: `(name, enables and forces RLS, the current
/// login may SELECT)`.
async fn pipeline_table_security(backend: &Arc<PgBackend>) -> Vec<(String, bool, bool)> {
    backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_rls_connection_failed")
        .query(
            r"SELECT c.relname::TEXT,
                     c.relrowsecurity AND c.relforcerowsecurity,
                     has_table_privilege(c.oid, 'SELECT')
                FROM pg_class c
                JOIN pg_namespace n ON n.oid = c.relnamespace
               WHERE n.nspname = 'public'
                 AND c.relkind IN ('r', 'p')
                 AND (c.relname LIKE 'pipeline\_%' OR c.relname = 'phase_outcomes')
               ORDER BY c.relname",
            &[],
        )
        .await
        .expect("restore_rls_query_failed")
        .iter()
        .map(|row| (row.get(0), row.get(1), row.get(2)))
        .collect()
}

/// The runtime login's privilege set (`RUNTIME_PRIVILEGES_SQL`), hashed, and
/// how many privileges it holds.
async fn runtime_privileges(runtime: &Arc<PgBackend>) -> (String, usize) {
    hashed_catalog_lines(runtime, RUNTIME_PRIVILEGES_SQL, "restore_privileges").await
}

/// The policy set (`RLS_POLICY_SET_SQL`), hashed, and how many policies it
/// holds. The catalog is readable by any login; the runtime login reads it.
async fn rls_policy_set(runtime: &Arc<PgBackend>) -> (String, usize) {
    hashed_catalog_lines(runtime, RLS_POLICY_SET_SQL, "restore_rls_policy_set").await
}

/// Every `public` table's RLS flags (`RLS_FLAG_SET_SQL`), hashed, and how
/// many tables. The catalog is readable by any login; the runtime login
/// reads it.
async fn rls_flag_set(runtime: &Arc<PgBackend>) -> (String, usize) {
    hashed_catalog_lines(runtime, RLS_FLAG_SET_SQL, "restore_rls_flags").await
}

/// Runs `sql` (one text of sorted lines and their count) on `backend` and
/// returns the text's hash and the count; failures panic with
/// `<label>_connection_failed` or `<label>_query_failed`.
async fn hashed_catalog_lines(backend: &Arc<PgBackend>, sql: &str, label: &str) -> (String, usize) {
    let row = backend
        .trace_pool_for_test()
        .get()
        .await
        .unwrap_or_else(|_| panic!("{label}_connection_failed"))
        .query_one(sql, &[])
        .await
        .unwrap_or_else(|_| panic!("{label}_query_failed"));
    let text: String = row.get(0);
    let count = usize::try_from(row.get::<_, i64>(1)).expect("a count is not negative");
    (sha256_bytes(text.as_bytes()), count)
}

/// Every tenant's rows in every table of `TRACE_COMMONS_RLS_TABLES`.
struct TenantFingerprint {
    /// SHA-256 over one sorted line per table and tenant: the table, the
    /// tenant id's hash, its row count, and a hash of its rows' JSON.
    hash: String,
    /// The tenant ids found, raw: they stay in this process (the audit
    /// chain check reads each one) and are never printed or written.
    tenants: Vec<String>,
}

/// `TenantFingerprint`, read by `owner`, a login RLS does not bind (checked
/// first): a tenant's rows count whether or not a policy admits them, so a
/// restore that drops or changes another tenant's rows changes the hash.
async fn tenant_fingerprint(owner: &Arc<PgBackend>) -> TenantFingerprint {
    let client = owner
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_tenant_fingerprint_connection_failed");
    let sees_every_tenant: bool = client
        .query_one(
            "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .expect("restore_tenant_fingerprint_query_failed")
        .get(0);
    assert!(
        sees_every_tenant,
        "restore_tenant_fingerprint_reader_bound_by_rls"
    );
    let mut lines = Vec::new();
    let mut tenants = BTreeSet::new();
    for table in TRACE_COMMONS_RLS_TABLES {
        // `table` is one of this crate's own constant names, never input.
        let rows = client
            .query(
                &format!(
                    r#"SELECT t.tenant_id::TEXT, COUNT(*),
                              encode(sha256(convert_to(
                                  string_agg(row_to_json(t)::TEXT, E'\n'
                                             ORDER BY row_to_json(t)::TEXT),
                                  'UTF8')), 'hex')
                         FROM public."{table}" t
                        GROUP BY t.tenant_id"#
                ),
                &[],
            )
            .await
            .expect("restore_tenant_fingerprint_query_failed");
        for row in rows {
            let tenant: Option<String> = row.get(0);
            let count: i64 = row.get(1);
            let rows_hash: String = row.get(2);
            let tenant_hash = tenant
                .as_deref()
                .map(|tenant| sha256_bytes(tenant.as_bytes()))
                .unwrap_or_default();
            lines.push(format!("{table}|{tenant_hash}|{count}|{rows_hash}"));
            tenants.extend(tenant);
        }
    }
    lines.sort();
    TenantFingerprint {
        hash: sha256_bytes(lines.join("\n").as_bytes()),
        tenants: tenants.into_iter().collect(),
    }
}

/// `main`'s database audit chain of each tenant in `tenants`, by `main`'s
/// own verifier (`verify_db_audit_chain`, the check behind
/// `POST /v1/admin/audit-chain-drill`), with no file log: the drill's file
/// logs are in the seed's temporary state directory, which a database
/// restore does not carry, so every tenant's first hashed row must chain
/// from genesis. Panics with `label` on a chain that does not verify;
/// returns each tenant's hashed events: the rows the verifier chains and
/// checks. An unhashed (legacy) row is skipped by the verifier, so it is not
/// counted (review of this wave, M1).
async fn verified_audit_events(
    mains: &Arc<dyn Database>,
    tenants: &[String],
    label: &'static str,
) -> BTreeMap<String, usize> {
    let mut counts = BTreeMap::new();
    for tenant in tenants {
        let report = verify_db_audit_chain(mains.as_ref(), tenant, &[])
            .await
            .unwrap_or_else(|_| panic!("{label}"));
        assert!(report.verified, "{label}");
        counts.insert(
            tenant.clone(),
            report.event_count - report.legacy_event_count,
        );
    }
    counts
}

/// Polls, up to 60 s, until `tenant_id` has at least one Trace Credit leg
/// with a credit event and every such leg is marked audited: the worker's
/// `CreditMutate` audit event for each credit event is in `main`'s chain.
async fn wait_for_credit_audited(backend: &Arc<PgBackend>, tenant_id: &str) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    loop {
        let mut client = backend
            .trace_pool_for_test()
            .get()
            .await
            .expect("restore_seed_audit_wait_connection_failed");
        let tx = tenant_tx(&mut client, tenant_id).await;
        let row = tx
            .query_one(
                "SELECT COUNT(*), COUNT(*) FILTER (WHERE credit_audited_at IS NULL)
                   FROM pipeline_run_settlements
                  WHERE tenant_id = $1 AND credit_event_id IS NOT NULL",
                &[&tenant_id],
            )
            .await
            .expect("restore_seed_audit_wait_query_failed");
        tx.commit()
            .await
            .expect("restore_seed_audit_wait_query_failed");
        let (legs, unaudited): (i64, i64) = (row.get(0), row.get(1));
        if legs > 0 && unaudited == 0 {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "restore_seed_credit_audit_timeout"
        );
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

/// The RLS diagnostic `tests/trace_corpus_pg_rls.rs` reads
/// (`pg_trace_corpus_rls_diagnostics_report_policy_coverage`), run as the
/// runtime login over every table in `TRACE_COMMONS_RLS_TABLES`: each table
/// enables and forces RLS, and its `trace_corpus_tenant_isolation` policy's
/// `USING` and `WITH CHECK` expressions, read back with `pg_get_expr`, are
/// the tenant predicate on `trace_current_tenant_id()`. Each failure names
/// its own label and the tables (table names only). Returns the number of
/// tables checked.
async fn require_trace_tables_isolated(runtime: &Arc<PgBackend>, stage: &str) -> usize {
    let rls = runtime
        .trace_corpus_rls_diagnostics()
        .await
        .expect("restore_rls_diagnostics_failed")
        .expect("restore_rls_diagnostics_unavailable");
    let mut mismatched = rls
        .missing_policy_tables
        .iter()
        .chain(&rls.policy_expression_mismatch_tables)
        .collect::<Vec<_>>();
    mismatched.sort();
    mismatched.dedup();
    assert!(
        mismatched.is_empty(),
        "{stage}_rls_policy_predicate_mismatch: {mismatched:?}"
    );
    let mut unforced = rls
        .rls_disabled_tables
        .iter()
        .chain(&rls.force_rls_disabled_tables)
        .collect::<Vec<_>>();
    unforced.sort();
    unforced.dedup();
    assert!(
        unforced.is_empty(),
        "{stage}_rls_not_enabled_and_forced: {unforced:?}"
    );
    assert!(
        !rls.current_role_bypasses_rls && !rls.current_role_owns_trace_tables,
        "{stage}_runtime_role_bypasses_rls"
    );
    assert!(
        rls.tenant_context_transaction_local,
        "{stage}_tenant_context_not_transaction_local"
    );
    assert!(
        rls.production_ready() && rls.expected_table_count == TRACE_COMMONS_RLS_TABLES.len(),
        "{stage}_rls_not_production_ready"
    );
    rls.expected_table_count
}

/// Writes `ACTIVATION_TENANT`'s activation state, as the runtime login,
/// through the stores an operator's actions go through: the service's
/// default bundle (so the tenant has policy rows), a containment (the routing
/// row and its event), a suspension of the bundle's Score policy (the status
/// row and its intervention), and an index rebuild fence. The qualification
/// row is inserted directly, with the runtime login's own `INSERT` grant: a
/// qualification through `qualify_bundle` needs a signed package and a full
/// set of check results, which this drill does not hold. Its digests are
/// well formed and name nothing.
async fn seed_activation_state(runtime: &Arc<PgBackend>, service: &PipelineService) {
    let actor = format!("principal_sha256:{}", "c5".repeat(32));
    service
        .register_default_bundle(ACTIVATION_TENANT)
        .await
        .expect("restore_seed_activation_bundle_failed");
    let bundle_id = service.bundle_id().to_string();
    routing_store(runtime)
        .expect("a routing store")
        .contain(ACTIVATION_TENANT, &actor, "restore_drill_contain")
        .await
        .expect("restore_seed_containment_failed");
    service
        .intervene_policy(
            ACTIVATION_TENANT,
            &bundle_id,
            Phase::Score,
            "suspend",
            &actor,
            "restore_drill_suspend",
        )
        .await
        .expect("restore_seed_suspension_failed");
    let digest = sha256_bytes(b"restore-drill-qualification");
    let mut client = runtime
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_seed_qualification_connection_failed");
    let tx = tenant_tx(&mut client, ACTIVATION_TENANT).await;
    tx.execute(
        "INSERT INTO pipeline_bundle_qualifications (
            tenant_id, bundle_id, package_hash, signing_key_id, signature_hash,
            corpus_digest, input_digest, configuration_digest, code_revision_hash,
            runtime_dependency_digest, evidence_hash
         ) VALUES ($1, $2, $3, 'restore-drill-key', $3, $3, $3, $3, $3, $3, $3)",
        &[&ACTIVATION_TENANT, &bundle_id, &digest],
    )
    .await
    .expect("restore_seed_qualification_failed");
    tx.commit()
        .await
        .expect("restore_seed_qualification_failed");
    service
        .store()
        .set_index_rebuild_fence(
            ACTIVATION_TENANT,
            Uuid::new_v4(),
            std::time::Duration::from_secs(3600),
        )
        .await
        .expect("restore_seed_fence_failed");
}

/// `ACTIVATION_TENANT`'s activation state is in the database `runtime`
/// reaches, read as the runtime login in the tenant's own transaction: one
/// row in each of `ACTIVATION_STATE_TABLES`, the routing state `contained`,
/// and the Score policy of its active bundle not runnable. Each failure
/// names `<stage>_...` and, for a count, the table.
async fn require_activation_state(runtime: &Arc<PgBackend>, stage: &str) {
    let mut client = runtime
        .trace_pool_for_test()
        .get()
        .await
        .unwrap_or_else(|_| panic!("{stage}_activation_state_connection_failed"));
    let tx = tenant_tx(&mut client, ACTIVATION_TENANT).await;
    for table in ACTIVATION_STATE_TABLES {
        // `table` is one of this module's own constant names, never input.
        let rows: i64 = tx
            .query_one(
                &format!(r#"SELECT COUNT(*) FROM public."{table}" WHERE tenant_id = $1"#),
                &[&ACTIVATION_TENANT],
            )
            .await
            .unwrap_or_else(|_| panic!("{stage}_activation_state_query_failed"))
            .get(0);
        assert_eq!(rows, 1, "{stage}_activation_state_row_missing: {table}");
    }
    let routing_state: String = tx
        .query_one(
            "SELECT routing_state FROM pipeline_tenant_routing WHERE tenant_id = $1",
            &[&ACTIVATION_TENANT],
        )
        .await
        .unwrap_or_else(|_| panic!("{stage}_activation_state_query_failed"))
        .get(0);
    assert_eq!(routing_state, "contained", "{stage}_routing_state_changed");
    let score_runnable: bool = tx
        .query_one(
            "SELECT status.runnable
               FROM pipeline_active_bundles active
               JOIN pipeline_bundle_policy_status status
                 ON status.tenant_id = active.tenant_id
                AND status.bundle_id = active.bundle_id
              WHERE active.tenant_id = $1 AND status.phase = 'score'",
            &[&ACTIVATION_TENANT],
        )
        .await
        .unwrap_or_else(|_| panic!("{stage}_activation_state_query_failed"))
        .get(0);
    assert!(!score_runnable, "{stage}_policy_suspension_lost");
    tx.commit()
        .await
        .unwrap_or_else(|_| panic!("{stage}_activation_state_query_failed"));
}

// ---------------------------------------------------------------------------
// The two ignored tests.
// ---------------------------------------------------------------------------

/// `pipeline.py restore-drill`, before the dump: one completed run and one
/// run stopped after its durable Settle selection, the activation state of a
/// third tenant (`seed_activation_state`), then the seed fingerprint.
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
    // Migrates the suite's database and resets the account rate limiter.
    // The owner also reads every tenant's rows for the tenant fingerprint.
    let owner = account_owner_backend()
        .await
        .expect("the same variable runtime_backend read is set");
    let mains = mains_database().await;
    let state_dir = tempfile::tempdir().expect("temp dir");
    let artifacts = test_artifact_store_with_key(&config.artifact_root, &config.master_key_hex);
    // Shared by both lifetimes (PF-2): the in-memory index and the recording
    // adapter see each other's writes only when the same instances back both
    // apps.
    let index = IsolatedPipelineIndex::new();
    let adapters = RecordingAdapters::new();
    let start = |crash_point: Option<PipelineCrashPoint>| {
        let service = compatibility_service(&runtime, &artifacts, &index, &adapters, crash_point);
        restore_app_state(state_dir.path(), &mains, &artifacts, &runtime, service)
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
    // The second tenant's run: the same fixture, its own tenant, so the
    // restore is checked across tenants (Zaki's review of #1166, Major 2).
    let (status, receipt) = post_trace(&client, &base, SECOND_TOKEN, &body).await;
    assert!(
        status == reqwest::StatusCode::OK && receipt["status"] == "processing",
        "restore_seed_receipt_refused"
    );
    wait_for_run_complete(&runtime, SECOND_TENANT, completed_envelope.submission_id).await;
    // Both credit events are in `main`'s audit chain before the app stops,
    // so the chain the restore must keep is not empty.
    for tenant in [RESTORE_TENANT, SECOND_TENANT] {
        wait_for_credit_audited(&runtime, tenant).await;
    }
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
    // PR 5's activation state, for a tenant of its own (G23), written before
    // the fingerprints are taken so that they cover it.
    seed_activation_state(
        &runtime,
        &compatibility_service(&runtime, &artifacts, &index, &adapters, None),
    )
    .await;
    require_activation_state(&runtime, "restore_seed").await;

    // The seed is the shape the drill needs: one complete run with four
    // outcomes and a credited Trace Credit leg, and one leased run with a
    // durable Settle selection, three outcomes, and no settlement request or
    // ledger event yet.
    let runs = tenant_runs(&runtime, RESTORE_TENANT).await;
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
    assert!(
        completed.credited_once() && completed.trace_credit_legs == completed.settlement_count,
        "restore_seed_run_not_credited"
    );
    assert_eq!(
        pending.credit_events, 0,
        "restore_seed_pending_run_credited"
    );
    let second_runs = tenant_runs(&runtime, SECOND_TENANT).await;
    let [second] = second_runs.as_slice() else {
        panic!("restore_seed_second_tenant_run_count");
    };
    assert!(
        second.state == "complete"
            && second.phases == ["admission", "review", "score", "settle"]
            && second.credited_once()
            && second.trace_credit_legs == second.settlement_count,
        "restore_seed_second_tenant_run_not_credited"
    );
    // The adapter saw the completed run's legs and the second tenant's, and
    // nothing of the pending run. Only `RESTORE_TENANT`'s count is recorded:
    // the resume adds the pending run's to it.
    let request_runs = adapters.request_runs();
    let completed_requests = request_runs
        .iter()
        .filter(|run| **run == completed.run_id)
        .count();
    assert!(
        request_runs
            .iter()
            .all(|run| *run == completed.run_id || *run == second.run_id)
            && completed_requests == completed.settlement_count
            && request_runs.len() - completed_requests == second.settlement_count
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

    // What the resume compares across the restore, taken from the database
    // the dump reads: the RLS diagnostic over every trace table, the runtime
    // login's privileges, every tenant's rows, and every tenant's audit
    // chain, each of the two tenants with at least one hashed, verified
    // audit event.
    let rls_table_count = require_trace_tables_isolated(&runtime, "restore_seed").await;
    let (rls_policy_set_hash, rls_policy_count) = rls_policy_set(&runtime).await;
    let (rls_flag_set_hash, rls_flag_table_count) = rls_flag_set(&runtime).await;
    let (runtime_privilege_set_hash, runtime_privilege_count) = runtime_privileges(&runtime).await;
    let tenants = tenant_fingerprint(&owner).await;
    assert!(
        [RESTORE_TENANT, SECOND_TENANT]
            .iter()
            .all(|tenant| tenants.tenants.iter().any(|seen| seen == tenant)),
        "restore_seed_tenant_missing"
    );
    let audit_events =
        verified_audit_events(&mains, &tenants.tenants, "restore_seed_audit_chain_broken").await;
    assert!(
        [RESTORE_TENANT, SECOND_TENANT]
            .iter()
            .all(|tenant| audit_events.get(*tenant).is_some_and(|count| *count > 0)),
        "restore_seed_audit_chain_empty"
    );

    let fingerprint = RestoreFingerprint {
        schema: RESTORE_FINGERPRINT_SCHEMA.to_string(),
        database_fingerprint: database_fingerprint(&runtime).await,
        artifact_fingerprint,
        index_entry_set_hash: entry_set_hash,
        pending_run_id_hash: id_hash(pending.run_id),
        adapter_request_count: completed_requests,
        completed_settlement_count: completed.settlement_count,
        completed_credit_event_count: completed.credit_events,
        rls_table_count,
        rls_policy_set_hash,
        rls_policy_count,
        rls_flag_set_hash,
        rls_flag_table_count,
        runtime_privilege_set_hash,
        runtime_privilege_count,
        tenant_fingerprint: tenants.hash,
        tenant_count: tenants.tenants.len(),
        audit_event_count: audit_events.values().sum(),
    };
    write_atomically(&config.fingerprint_path, &fingerprint.to_bytes());
}

/// `pipeline.py restore-drill`, after the restore and the artifact copy:
/// the restored state equals the seed's, the third tenant's activation state
/// included (`require_activation_state`), and the pending run completes once
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
    let mains = mains_database_at(&url).await;
    let tables = pipeline_table_security(&runtime).await;
    assert_eq!(
        tables
            .iter()
            .map(|(name, ..)| name.as_str())
            .collect::<Vec<_>>(),
        PIPELINE_TABLES,
        "restore_pipeline_tables_unexpected"
    );
    assert!(
        tables.iter().all(|(_, forced, _)| *forced),
        "restore_rls_not_forced"
    );
    assert!(
        tables.iter().all(|(_, _, readable)| *readable),
        "restore_privileges_lost"
    );
    // Zaki's review of #1166, Major 2: the flags above are not the
    // isolation. Every trace table, `trace_credit_ledger` and the others the
    // drill reads included, must still carry the tenant predicate.
    let rls_tables_checked = require_trace_tables_isolated(&runtime, "restore").await;
    assert_eq!(
        rls_tables_checked, seed.rls_table_count,
        "restore_rls_table_count_changed"
    );
    // Every policy, not only the named one: an added permissive policy, or
    // one whose roles widened, opens a table the check above calls isolated.
    let (rls_policy_set_hash, rls_policy_count) = rls_policy_set(&runtime).await;
    assert!(
        rls_policy_set_hash == seed.rls_policy_set_hash
            && rls_policy_count == seed.rls_policy_count,
        "restore_rls_policy_set_changed"
    );
    // Every table's two RLS flags, not only the tables checked above (Zaki's
    // re-review of #1166, Medium): a table outside the pipeline set and
    // `TRACE_COMMONS_RLS_TABLES` whose RLS a restore disabled or un-forced.
    let (rls_flag_set_hash, rls_flag_table_count) = rls_flag_set(&runtime).await;
    assert!(
        rls_flag_set_hash == seed.rls_flag_set_hash
            && rls_flag_table_count == seed.rls_flag_table_count,
        "restore_rls_flags_changed"
    );
    // Every privilege of every type, not only SELECT.
    let (runtime_privilege_set_hash, runtime_privilege_count) = runtime_privileges(&runtime).await;
    assert!(
        runtime_privilege_set_hash == seed.runtime_privilege_set_hash
            && runtime_privilege_count == seed.runtime_privilege_count,
        "restore_runtime_privileges_changed"
    );

    // What the restore kept.
    let database_fingerprint = database_fingerprint(&runtime).await;
    assert_eq!(
        database_fingerprint, seed.database_fingerprint,
        "restore_database_fingerprint_mismatch"
    );
    // Every tenant's rows, read by the restored database's owner.
    let owner = Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(&url, 2))
            .await
            .expect("restore_owner_connection_failed"),
    );
    let tenants = tenant_fingerprint(&owner).await;
    // The audit chain is verified before the fingerprint is compared (wave
    // 2; review-fix concern 4): the fingerprint covers `trace_audit_events`
    // too, so a changed audit row would otherwise report as a fingerprint
    // mismatch rather than as the broken chain it is.
    let audit_events_verified: usize =
        verified_audit_events(&mains, &tenants.tenants, "restore_audit_chain_broken")
            .await
            .values()
            .sum();
    assert!(
        tenants.hash == seed.tenant_fingerprint && tenants.tenants.len() == seed.tenant_count,
        "restore_tenant_fingerprint_mismatch"
    );
    assert_eq!(
        audit_events_verified, seed.audit_event_count,
        "restore_audit_event_count_mismatch"
    );
    // The activation state of PR 5 came back with the rest (G23): the routing
    // row and its event, the suspension and its intervention, the
    // qualification, and the fence, each as the seed wrote it.
    require_activation_state(&runtime, "restore").await;
    let artifact_fingerprint = artifact_fingerprint(&config.artifact_root);
    assert_eq!(
        artifact_fingerprint, seed.artifact_fingerprint,
        "restore_artifact_fingerprint_mismatch"
    );
    let before = tenant_runs(&runtime, RESTORE_TENANT).await;
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
    assert_eq!(
        pending.credit_events, 0,
        "restore_pending_run_already_credited"
    );
    let pending_selection = committed_selection_rows(&runtime, pending.run_id).await;
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
    let service = compatibility_service(&runtime, &artifacts, &index, &adapters, None);
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

    // The app on the restored state resumes the pending run by itself. The
    // drill is one of the four checks that test the qualification
    // candidate (P5-D15), so its result names that package: the service the
    // drill resumed on serves exactly it.
    let package = qualification_candidate_package().expect("restore_candidate_package_invalid");
    assert!(
        *service.default_package() == package,
        "restore_service_not_the_candidate"
    );
    let state = restore_app_state(
        state_dir.path(),
        &mains,
        &artifacts,
        &runtime,
        service.clone(),
    );
    let (base, stop, server) = serve_pipeline_app(state).await;
    let client = reqwest::Client::new();
    wait_for_pipeline_ready(&client, &base).await;
    wait_for_run_complete(&runtime, RESTORE_TENANT, pending.submission_id).await;
    stop.send(()).expect("send shutdown to the restored app");
    join_within(server, 20, "the restored app").await;

    // One logical effect.
    let after = tenant_runs(&runtime, RESTORE_TENANT).await;
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
    // The durable selection is what the resume settled: the committed
    // outcomes and the selection itself are unchanged.
    assert!(
        committed_selection_rows(&runtime, pending.run_id).await == pending_selection,
        "restore_settle_selection_changed"
    );
    // Ruling T10-5: the resumed run settled the same legs as the completed
    // run (same bundle, same award), and at least one, as the seed recorded.
    assert!(
        resumed.settlement_count > 0
            && !completed.is_empty()
            && completed
                .iter()
                .all(|run| run.settlement_count == resumed.settlement_count)
            && resumed.settlement_count == seed.completed_settlement_count,
        "restore_pending_run_settlement_count"
    );
    // One ledger event for each credited leg, none of which existed before
    // the resume, as many as the completed run has.
    assert!(
        resumed.credited_once() && resumed.credit_events == seed.completed_credit_event_count,
        "restore_pending_run_credit_events"
    );
    let request_runs = adapters.request_runs();
    assert!(
        request_runs.iter().all(|run| *run == pending.run_id)
            && request_runs.len() == resumed.settlement_count,
        "restore_adapter_requests_unexpected"
    );
    let legs: usize = after.iter().map(|run| run.settlement_count).sum();
    assert_eq!(
        seed.adapter_request_count + request_runs.len(),
        legs,
        "restore_adapter_request_count_mismatch"
    );
    let duplicate_effects = duplicated_rows(&runtime).await;
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
            "rls_tables_checked": rls_tables_checked,
            "rls_policy_set_hash": rls_policy_set_hash,
            "rls_policy_count": rls_policy_count,
            "rls_flag_set_hash": rls_flag_set_hash,
            "rls_flag_table_count": rls_flag_table_count,
            "runtime_privilege_set_hash": runtime_privilege_set_hash,
            "tenant_fingerprint": tenants.hash,
            "tenant_count": tenants.tenants.len(),
            "audit_events_verified": audit_events_verified,
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
        completed_settlement_count: 1,
        completed_credit_event_count: 1,
        rls_table_count: TRACE_COMMONS_RLS_TABLES.len(),
        rls_policy_set_hash: hash("policies"),
        rls_policy_count: 1,
        rls_flag_set_hash: hash("flags"),
        rls_flag_table_count: 1,
        runtime_privilege_set_hash: hash("privileges"),
        runtime_privilege_count: 1,
        tenant_fingerprint: hash("tenants"),
        tenant_count: 2,
        audit_event_count: 2,
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
        edited(&|value| value["completed_credit_event_count"] = 0.into()),
        Err("restore_fingerprint_invalid")
    );
    assert_eq!(
        edited(&|value| value["completed_settlement_count"] = 0.into()),
        Err("restore_fingerprint_invalid")
    );
    // A one-tenant seed cannot show a restore that drops another tenant.
    assert_eq!(
        edited(&|value| value["tenant_count"] = 1.into()),
        Err("restore_fingerprint_invalid")
    );
    for count in [
        "rls_table_count",
        "rls_policy_count",
        "rls_flag_table_count",
        "runtime_privilege_count",
        "audit_event_count",
    ] {
        assert_eq!(
            edited(&|value| value[count] = 0.into()),
            Err("restore_fingerprint_invalid")
        );
    }
    for hash_field in [
        "runtime_privilege_set_hash",
        "tenant_fingerprint",
        "rls_policy_set_hash",
        "rls_flag_set_hash",
    ] {
        assert_eq!(
            edited(&|value| value[hash_field] = "not-a-hash".into()),
            Err("restore_fingerprint_invalid")
        );
    }
    assert_eq!(
        RestoreFingerprint::parse(b"not json"),
        Err("restore_fingerprint_invalid")
    );
}
