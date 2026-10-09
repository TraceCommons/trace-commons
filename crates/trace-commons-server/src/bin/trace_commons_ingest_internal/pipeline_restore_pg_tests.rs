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
//! `pipeline.py promote remote-restore` (spec B-D2) runs the same two tests
//! with a remote store in place of the artifact root
//! (`TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE`): the seed writes into the
//! live store under a namespace of its own, `pipeline_remote_restore_run`
//! copies that namespace into the scratch store as ciphertext, and the resume
//! runs on the scratch store and writes a report for `promote` instead of
//! emitting `pipeline_restore_drill`.
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
    assemble_production_harness_service, fixture_envelope, id_hash, load_corpus, main_corpus_path,
    production_test_package, production_test_package_awarding, sha256_bytes, write_atomically,
};
use super::pipeline_http_pg_tests::{
    PIPELINE_HTTP_RUNTIME_ROLE, PassThroughPipelinePrivacyBoundary, account_owner_backend,
    assemble_compatibility_pipeline_service_with, expire_run_lease, join_within, mains_database,
    mains_database_at, pilot_runtime_login, pipeline_http_database_url, post_trace,
    qualification_candidate_package, runtime_backend, runtime_backend_at, serve_pipeline_app,
    tenant_tx, wait_for_pipeline_ready, wait_for_run_complete, wait_for_settle_selection,
};
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::{BundlePackage, InstrumentId, Phase, TenantStorageRef};
use trace_commons_server::db::postgres::{TRACE_COMMONS_RLS_TABLES, registered_migrations};
use trace_commons_server::versioned_pipeline::{PipelineCrashPoint, pipeline_tenant_storage_ref};
use trace_commons_server::versioned_pipeline_bundle::MINIMAL_INDEX_ID;
use trace_commons_server::versioned_pipeline_credit::RecordingSettlementAdapter;
use trace_commons_server::versioned_pipeline_harness::{HarnessAssembly, HarnessDependencies};
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_production::PRODUCTION_COMPATIBILITY_INDEX_ID;
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

/// The variables `pipeline.py restore-drill` sets for both tests, and
/// `pipeline.py promote remote-restore` too, with a remote store in place of
/// the artifact root.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RestoreConfig {
    artifacts: RestoreArtifacts,
    master_key_hex: String,
    fingerprint_path: PathBuf,
}

/// Where the seed writes its artifacts and the resume reads them.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RestoreArtifacts {
    /// `pipeline.py restore-drill`: a directory, copied between the two.
    Local(PathBuf),
    /// `promote remote-restore` (spec B-D2): a remote store, the live one for
    /// the seed and the scratch one for the resume, restored between the two
    /// by `pipeline_remote_restore_run`.
    Remote(RemoteArtifacts),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteArtifacts {
    kind: RemoteStoreKind,
    location: RemoteStoreLocation,
    /// Where the resume writes what it measured, in place of the
    /// `pipeline_restore_drill` result (`promote` builds its own result from
    /// it). The seed ignores it.
    resume_report_path: Option<PathBuf>,
}

impl RestoreConfig {
    /// `Ok(None)` when none of the variables is set (nothing to run). The
    /// master key and the fingerprint path, with exactly one of: the artifact
    /// root; or a remote store, its kind, and the drill's namespace (and the
    /// double's root exactly when the kind is the double).
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        let artifacts = match (var(ARTIFACT_ROOT_VAR), var(REMOTE_ARTIFACT_STORE_VAR)) {
            (None, None) => None,
            (Some(root), None) => Some(RestoreArtifacts::Local(PathBuf::from(root))),
            (None, Some(store)) => {
                let (Some(kind), Some(namespace)) =
                    (var(REMOTE_STORE_KIND_VAR), var(REMOTE_NAMESPACE_VAR))
                else {
                    return Err("restore_environment_incomplete");
                };
                Some(RestoreArtifacts::Remote(RemoteArtifacts {
                    kind: RemoteStoreKind::from_vars(&kind, var(REMOTE_DOUBLE_ROOT_VAR))?,
                    location: RemoteStoreLocation::parse(&store, &namespace)?,
                    resume_report_path: var(REMOTE_RESUME_REPORT_PATH_VAR).map(PathBuf::from),
                }))
            }
            (Some(_), Some(_)) => return Err("restore_environment_incomplete"),
        };
        match (
            artifacts,
            var(TEST_MASTER_KEY_VAR),
            var(RESTORE_FINGERPRINT_PATH_VAR),
        ) {
            (None, None, None) => Ok(None),
            (Some(artifacts), Some(master_key_hex), Some(fingerprint_path)) => Ok(Some(Self {
                artifacts,
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
    /// The package's `NoveltyUtility` delta is `0` (the pilot's): Score
    /// awards nothing, so the three counts above are `0` and the drill
    /// covers no settlement leg, ledger event, or credit audit.
    credit_delta_zero: bool,
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
            || !fingerprint.credit_counts_match_delta()
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

    /// A seed under a zero delta settled and credited nothing; any other
    /// seed settled and credited at least one leg.
    fn credit_counts_match_delta(&self) -> bool {
        let counts = [
            self.adapter_request_count,
            self.completed_settlement_count,
            self.completed_credit_event_count,
        ];
        if self.credit_delta_zero {
            counts.iter().all(|count| *count == 0)
        } else {
            self.completed_settlement_count > 0 && self.completed_credit_event_count > 0
        }
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
// Remote stores (`promote remote-restore`, spec B-D2).
// ---------------------------------------------------------------------------

/// The remote store the seed writes to (the live store) or the resume reads
/// from (the scratch store), as `bucket[/prefix]`; its kind; and the drill's
/// own namespace under it, fresh for each run, so the drill writes nothing
/// beside the store's own objects and lists only its own.
const REMOTE_ARTIFACT_STORE_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_ARTIFACT_STORE";
const REMOTE_STORE_KIND_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_STORE_KIND";
const REMOTE_NAMESPACE_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_NAMESPACE";
/// The directory the directory double keeps its buckets in.
const REMOTE_DOUBLE_ROOT_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_DOUBLE_ROOT";
/// The remote resume's report (`REMOTE_RESUME_REPORT_SCHEMA`).
const REMOTE_RESUME_REPORT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_RESUME_REPORT_PATH";
/// `pipeline_remote_restore_run`'s two stores and its report.
const REMOTE_SOURCE_STORE_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_SOURCE_STORE";
const REMOTE_SCRATCH_STORE_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_SCRATCH_STORE";
const REMOTE_COPY_REPORT_PATH_VAR: &str = "TRACE_COMMONS_PIPELINE_REMOTE_COPY_REPORT_PATH";
const REMOTE_RESUME_REPORT_SCHEMA: &str = "trace_commons.pipeline_remote_restore_resume.v1";

/// Which `GcsObjectClient` a remote store is opened with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RemoteStoreKind {
    /// Google Cloud Storage (`ProdGcsObjectClient`, the `gcs-client` build).
    Gcs,
    /// `DirectoryGcsObjectClient` over this directory: the drill's processes
    /// run locally against it. Its kind is not a remote store, so `promote`
    /// never passes a result measured on it.
    DirectoryDouble(PathBuf),
}

impl RemoteStoreKind {
    fn from_vars(kind: &str, double_root: Option<String>) -> Result<Self, &'static str> {
        match (kind, double_root) {
            ("gcs", None) => Ok(Self::Gcs),
            ("gcs_directory_double", Some(root)) => Ok(Self::DirectoryDouble(PathBuf::from(root))),
            ("gcs" | "gcs_directory_double", _) => Err("restore_environment_incomplete"),
            _ => Err("remote_restore_store_kind_invalid"),
        }
    }

    /// The report's `object_store_kind`.
    fn label(&self) -> &'static str {
        match self {
            Self::Gcs => "gcs",
            Self::DirectoryDouble(_) => "gcs_directory_double",
        }
    }
}

/// A store name split into its bucket and the key prefix the drill writes
/// under: the name's own prefix segments, then the namespace.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteStoreLocation {
    bucket: String,
    prefix: String,
}

impl RemoteStoreLocation {
    /// `name` is `promote`'s store name shape (`_store_name`): a bucket name,
    /// then optional prefix segments; `namespace` is one prefix segment.
    fn parse(name: &str, namespace: &str) -> Result<Self, &'static str> {
        let mut segments = name.split('/');
        let bucket = segments.next().unwrap_or_default();
        let prefix: Vec<&str> = segments.collect();
        if !is_bucket_name(bucket)
            || !prefix.iter().all(|segment| is_prefix_segment(segment))
            || !is_prefix_segment(namespace)
        {
            return Err("remote_restore_store_name_invalid");
        }
        let mut prefix = prefix;
        prefix.push(namespace);
        Ok(Self {
            bucket: bucket.to_string(),
            prefix: prefix.join("/"),
        })
    }
}

/// `[a-z0-9][a-z0-9._-]{1,61}[a-z0-9]`, as `promote` checks it.
fn is_bucket_name(value: &str) -> bool {
    let bytes = value.as_bytes();
    let edge = |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    (3..=63).contains(&bytes.len())
        && edge(&bytes[0])
        && edge(&bytes[bytes.len() - 1])
        && bytes
            .iter()
            .all(|byte| edge(byte) || matches!(byte, b'.' | b'_' | b'-'))
}

/// `[A-Za-z0-9_-][A-Za-z0-9._-]{0,127}`, never `.` or `..`, as `promote`
/// checks it.
fn is_prefix_segment(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= 128
        && bytes[0] != b'.'
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

/// `promote`'s `_overlaps`: one name's segments are a leading part of the
/// other's.
fn remote_store_names_overlap(first: &str, second: &str) -> bool {
    let first: Vec<&str> = first.split('/').collect();
    let second: Vec<&str> = second.split('/').collect();
    let shorter = first.len().min(second.len());
    first[..shorter] == second[..shorter]
}

/// A bucket as a directory, one file per object, for the drill's processes
/// to share without a bucket. Versioning is reported off: a directory has no
/// such policy. Test-only, and only ever selected by
/// `TRACE_COMMONS_PIPELINE_REMOTE_STORE_KIND=gcs_directory_double`.
struct DirectoryGcsObjectClient {
    root: PathBuf,
}

impl DirectoryGcsObjectClient {
    fn new(root: PathBuf) -> Self {
        Self { root }
    }

    fn path(&self, key: &str) -> anyhow::Result<PathBuf> {
        anyhow::ensure!(
            !key.is_empty()
                && key
                    .split('/')
                    .all(|segment| !segment.is_empty() && segment != "." && segment != ".."),
            "gcs_directory_double_key_invalid"
        );
        Ok(self.root.join(key))
    }
}

impl trace_commons_server::trace_artifact_gcs::GcsObjectClient for DirectoryGcsObjectClient {
    fn put_object(
        &self,
        key: &str,
        body: bytes::Bytes,
        _metadata: BTreeMap<String, String>,
    ) -> anyhow::Result<()> {
        let path = self.path(key)?;
        std::fs::create_dir_all(path.parent().expect("a key path has a parent"))?;
        let temporary = path.with_extension(format!("tmp-{}", Uuid::new_v4()));
        std::fs::write(&temporary, &body)?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    }

    fn get_object(
        &self,
        key: &str,
    ) -> anyhow::Result<trace_commons_server::trace_artifact_gcs::GcsObjectFetch> {
        match std::fs::read(self.path(key)?) {
            Ok(body) => Ok(trace_commons_server::trace_artifact_gcs::GcsObjectFetch {
                body: bytes::Bytes::from(body),
                metadata: BTreeMap::new(),
            }),
            // As `InMemoryGcsObjectClient` and the bucket's 404 answer.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Err(anyhow::Error::from(
                trace_commons_server::trace_artifact_store::TraceArtifactIntegrityError::new(
                    "GcsGetFailed: not found".to_string(),
                ),
            )),
            Err(error) => Err(error.into()),
        }
    }

    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        match std::fs::remove_file(self.path(key)?) {
            Ok(()) => Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error.into()),
        }
    }

    fn restore_deleted_object(&self, _key: &str) -> anyhow::Result<bool> {
        Ok(false)
    }

    fn list_object_keys(&self, prefix: &str) -> anyhow::Result<Vec<String>> {
        fn walk(root: &Path, dir: &Path, keys: &mut Vec<String>) -> std::io::Result<()> {
            let entries = match std::fs::read_dir(dir) {
                Ok(entries) => entries,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
                Err(error) => return Err(error),
            };
            for entry in entries {
                let path = entry?.path();
                if path.is_dir() {
                    walk(root, &path, keys)?;
                } else if path.is_file() {
                    let relative = path
                        .strip_prefix(root)
                        .expect("a walked path lies under its root");
                    let key = relative
                        .components()
                        .map(|component| component.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    keys.push(key);
                }
            }
            Ok(())
        }
        let mut keys = Vec::new();
        walk(&self.root, &self.root, &mut keys)?;
        keys.retain(|key| key.starts_with(prefix));
        keys.sort();
        Ok(keys)
    }

    fn bucket_versioning_enabled(&self) -> anyhow::Result<bool> {
        Ok(false)
    }
}

type RemoteClient = Arc<
    trace_commons_server::trace_artifact_gcs::PrefixedGcsObjectClient<
        Arc<dyn trace_commons_server::trace_artifact_gcs::GcsObjectClient>,
    >,
>;
type RemoteProvider =
    trace_commons_server::trace_artifact_gcs::GcsRemoteTraceArtifactProvider<RemoteClient>;

/// The bucket's client, confined to the location's prefix.
async fn remote_client(kind: &RemoteStoreKind, location: &RemoteStoreLocation) -> RemoteClient {
    let bucket: Arc<dyn trace_commons_server::trace_artifact_gcs::GcsObjectClient> = match kind {
        RemoteStoreKind::Gcs => gcs_bucket_client(&location.bucket).await,
        RemoteStoreKind::DirectoryDouble(root) => {
            Arc::new(DirectoryGcsObjectClient::new(root.join(&location.bucket)))
        }
    };
    Arc::new(
        trace_commons_server::trace_artifact_gcs::PrefixedGcsObjectClient::new(
            bucket,
            location.prefix.clone(),
        )
        .unwrap_or_else(|_| panic!("remote_restore_store_name_invalid")),
    )
}

/// `main()`'s rustls provider choice, made here because a test binary never
/// runs `main()`: with `ring` and `aws-lc-rs` both compiled in (the pilot's
/// feature set), rustls panics at the first TLS connection, the GCS client's
/// or the KMS key wrapper's, unless a provider is installed. An error means
/// one already is.
fn install_tls_provider() {
    let _ = rustls::crypto::ring::default_provider().install_default();
}

#[cfg(feature = "gcs-client")]
async fn gcs_bucket_client(
    bucket: &str,
) -> Arc<dyn trace_commons_server::trace_artifact_gcs::GcsObjectClient> {
    install_tls_provider();
    Arc::new(
        trace_commons_server::trace_artifact_gcs::prod_client::ProdGcsObjectClient::try_new(
            bucket.to_string(),
        )
        .await
        .unwrap_or_else(|_| panic!("remote_restore_gcs_client_unavailable")),
    )
}

#[cfg(not(feature = "gcs-client"))]
async fn gcs_bucket_client(
    _bucket: &str,
) -> Arc<dyn trace_commons_server::trace_artifact_gcs::GcsObjectClient> {
    panic!("remote_restore_gcs_client_not_compiled")
}

/// The provider the store writes through, keyed as `remote_gcs` keys the
/// live store (`TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE`).
fn remote_provider(client: &RemoteClient, location: &RemoteStoreLocation) -> RemoteProvider {
    trace_commons_server::trace_artifact_gcs::GcsRemoteTraceArtifactProvider::new(
        client.clone(),
        location.bucket.clone(),
        TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE,
    )
}

/// The key wrapper the deployment selects (`TRACE_COMMONS_KEK_PROVIDER`:
/// GCP KMS on the pilot, the local master key otherwise). On GCS it must be
/// a production one (`require_production_kek`).
async fn remote_kek(
    kind: &RemoteStoreKind,
    master_key_hex: &str,
) -> Box<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper + Send + Sync> {
    install_tls_provider();
    let kek =
        build_selected_kek_wrapper_async(secrecy::SecretString::from(master_key_hex.to_string()))
            .await
            .unwrap_or_else(|_| panic!("remote_restore_kek_unavailable"));
    require_production_kek(kind, kek.is_production_trust_boundary())
        .unwrap_or_else(|label| panic!("{label}"));
    kek
}

/// On GCS the unwrap count is evidence about the deployment's key wrapper,
/// so the wrapper must be a production trust boundary: under the local
/// master key the drill would pass and prove nothing about KMS. The
/// directory double runs on the local key and can never pass anyway.
fn require_production_kek(
    kind: &RemoteStoreKind,
    production_trust_boundary: bool,
) -> Result<(), &'static str> {
    match kind {
        RemoteStoreKind::Gcs if !production_trust_boundary => {
            Err("remote_restore_kek_not_production")
        }
        _ => Ok(()),
    }
}

fn remote_crypto(master_key_hex: &str) -> SecretsCrypto {
    SecretsCrypto::new(secrecy::SecretString::from(master_key_hex.to_string()))
        .unwrap_or_else(|_| panic!("remote_restore_master_key_invalid"))
}

/// The service-owned store over one remote location, as the deployment
/// configures it, and a provider over the same client to fingerprint it.
struct RemoteRestoreStore {
    configured: ConfiguredTraceArtifactStore,
    provider: RemoteProvider,
}

impl RemoteRestoreStore {
    async fn open(
        kind: &RemoteStoreKind,
        location: &RemoteStoreLocation,
        master_key_hex: &str,
    ) -> Self {
        let client = remote_client(kind, location).await;
        let kek = remote_kek(kind, master_key_hex).await;
        let kek_status = kek.safe_status();
        let store = ServiceOwnedTraceArtifactStore::new(
            TraceArtifactProviderConfig::service_owned_remote(
                TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE,
            )
            .expect("the service remote store name is valid"),
            remote_crypto(master_key_hex),
            kek,
            remote_provider(&client, location),
        );
        Self {
            configured: ConfiguredTraceArtifactStore::service_remote_for_test(
                Arc::new(store),
                "gcs",
                kek_status,
            ),
            provider: remote_provider(&client, location),
        }
    }

    fn configured(&self) -> &ConfiguredTraceArtifactStore {
        &self.configured
    }

    fn is_empty(&self) -> bool {
        self.provider
            .stored_object_keys()
            .unwrap_or_else(|_| panic!("remote_restore_list_failed"))
            .is_empty()
    }

    fn fingerprint(&self) -> String {
        trace_commons_server::versioned_pipeline_remote_restore::stored_artifact_fingerprint(
            &self.provider,
        )
        .unwrap_or_else(|error| panic!("{error}"))
        .fingerprint
    }
}

/// The artifact store one lifetime of the drill runs on.
enum RestoreStore {
    Local {
        root: PathBuf,
        configured: ConfiguredTraceArtifactStore,
    },
    Remote(RemoteRestoreStore),
}

impl RestoreStore {
    async fn open(config: &RestoreConfig) -> Self {
        match &config.artifacts {
            RestoreArtifacts::Local(root) => Self::Local {
                root: root.clone(),
                configured: ConfiguredTraceArtifactStore::legacy(test_artifact_store_with_key(
                    root,
                    &config.master_key_hex,
                )),
            },
            RestoreArtifacts::Remote(remote) => Self::Remote(
                RemoteRestoreStore::open(&remote.kind, &remote.location, &config.master_key_hex)
                    .await,
            ),
        }
    }

    fn configured(&self) -> &ConfiguredTraceArtifactStore {
        match self {
            Self::Local { configured, .. } => configured,
            Self::Remote(remote) => remote.configured(),
        }
    }

    /// The local tree's fingerprint, or the remote store's stored-object
    /// fingerprint: each is the same function of the same objects on both
    /// sides of its restore.
    fn fingerprint(&self) -> String {
        match self {
            Self::Local { root, .. } => artifact_fingerprint(root),
            Self::Remote(remote) => remote.fingerprint(),
        }
    }
}

/// What `pipeline_remote_restore_run` reads from its environment.
#[derive(Debug, Clone, PartialEq, Eq)]
struct RemoteCopyConfig {
    kind: RemoteStoreKind,
    source: RemoteStoreLocation,
    scratch: RemoteStoreLocation,
    master_key_hex: String,
    report_path: PathBuf,
}

impl RemoteCopyConfig {
    /// `Ok(None)` when none of the variables is set; all of them (the
    /// double's root exactly for the double) otherwise. The scratch store is
    /// never the source, nor inside it, nor around it.
    fn from_vars(var: impl Fn(&str) -> Option<String>) -> Result<Option<Self>, &'static str> {
        let names = [
            REMOTE_STORE_KIND_VAR,
            REMOTE_SOURCE_STORE_VAR,
            REMOTE_SCRATCH_STORE_VAR,
            REMOTE_NAMESPACE_VAR,
            TEST_MASTER_KEY_VAR,
            REMOTE_COPY_REPORT_PATH_VAR,
        ];
        let values: Vec<Option<String>> = names.iter().map(|name| var(name)).collect();
        if values.iter().all(Option::is_none) && var(REMOTE_DOUBLE_ROOT_VAR).is_none() {
            return Ok(None);
        }
        let [
            Some(kind),
            Some(source),
            Some(scratch),
            Some(namespace),
            Some(master_key_hex),
            Some(report_path),
        ] = <[Option<String>; 6]>::try_from(values).expect("six names")
        else {
            return Err("remote_restore_environment_incomplete");
        };
        let kind =
            RemoteStoreKind::from_vars(&kind, var(REMOTE_DOUBLE_ROOT_VAR)).map_err(|label| {
                match label {
                    "restore_environment_incomplete" => "remote_restore_environment_incomplete",
                    other => other,
                }
            })?;
        if remote_store_names_overlap(&source, &scratch) {
            return Err("remote_restore_scratch_overlaps_live_store");
        }
        Ok(Some(Self {
            kind,
            source: RemoteStoreLocation::parse(&source, &namespace)?,
            scratch: RemoteStoreLocation::parse(&scratch, &namespace)?,
            master_key_hex,
            report_path: PathBuf::from(report_path),
        }))
    }
}

/// The copy step: every object the seed wrote under the source's namespace
/// restored into the scratch store's (`restore_remote_artifacts`), measured,
/// and the measurement written to the report path and returned.
async fn run_remote_copy(config: &RemoteCopyConfig) -> serde_json::Value {
    let source = remote_client(&config.kind, &config.source).await;
    let scratch = remote_client(&config.kind, &config.scratch).await;
    let kek = remote_kek(&config.kind, &config.master_key_hex).await;
    let copy = trace_commons_server::versioned_pipeline_remote_restore::restore_remote_artifacts(
        &remote_provider(&source, &config.source),
        remote_provider(&scratch, &config.scratch),
        TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE,
        remote_crypto(&config.master_key_hex),
        kek,
    )
    .unwrap_or_else(|error| panic!("{error}"));
    let report = serde_json::json!({
        "schema": trace_commons_server::versioned_pipeline_remote_restore::REMOTE_RESTORE_COPY_SCHEMA,
        "object_store_kind": config.kind.label(),
        "object_count": copy.object_count,
        "artifact_fingerprint": copy.artifact_fingerprint,
        "restored_artifact_fingerprint": copy.restored_artifact_fingerprint,
        "kek_unwrap_verified_count": copy.kek_unwrap_verified_count,
        "versioning_enabled": copy.versioning_enabled,
    });
    let mut bytes = trace_commons_protocol::canonical_json::to_canonical_vec(&report)
        .expect("the copy report serialises");
    bytes.push(b'\n');
    write_atomically(&config.report_path, &bytes);
    report
}

/// `pipeline.py promote remote-restore`, between the restore of the seed's
/// database and the resume: restores the seed's objects from the live store
/// into the scratch store at the provider, as ciphertext, and writes the
/// measurement. Hash-only: no store, bucket, or key name reaches the report
/// or a panic.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "the artifact restore of `pipeline.py promote remote-restore`: run it through that command"]
async fn pipeline_remote_restore_run() {
    let Some(config) = RemoteCopyConfig::from_vars(|name| std::env::var(name).ok())
        .unwrap_or_else(|label| panic!("{label}"))
    else {
        return;
    };
    run_remote_copy(&config).await;
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

    /// Settled as a package that `awards_credit` settles a run: every leg a
    /// Trace Credit leg credited once, or, under a zero delta, no leg and no
    /// ledger event at all.
    fn settled_as_awarded(&self, awards_credit: bool) -> bool {
        if awards_credit {
            self.credited_once() && self.trace_credit_legs == self.settlement_count
        } else {
            self.settlement_count == 0 && self.trace_credit_legs == 0 && self.credit_events == 0
        }
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

/// Whether `RESTORE_TENANT`'s run for `submission_id` has a durable Settle
/// selection that includes it in the index (both floors passed), and whether
/// that selection has at least one settlement operation.
async fn settle_selection(backend: &Arc<PgBackend>, submission_id: Uuid) -> (bool, bool) {
    let mut client = backend
        .trace_pool_for_test()
        .get()
        .await
        .expect("restore_selection_rows_connection_failed");
    let tx = tenant_tx(&mut client, RESTORE_TENANT).await;
    let row = tx
        .query_one(
            r"SELECT COALESCE(settle_selection::jsonb -> 'decision'
                         -> 'index_membership' ->> 'kind' = 'include', false),
                     COALESCE(jsonb_array_length(
                         settle_selection::jsonb -> 'decision' -> 'settlement_operations') > 0,
                     false)
                FROM pipeline_runs
               WHERE tenant_id = $1 AND submission_id = $2",
            &[&RESTORE_TENANT, &submission_id],
        )
        .await
        .expect("restore_selection_rows_query_failed");
    tx.commit()
        .await
        .expect("restore_selection_rows_query_failed");
    (row.get(0), row.get(1))
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

/// What the drill's services run on (spec B-D1). One value backs every
/// service of one test, so the services of the seed's two lifetimes see each
/// other's index writes.
enum RestoreDependencies {
    /// The compatibility candidate (the `compatibility` bundle of
    /// `pipeline.py run`: a 2_500_000 microcredit `NoveltyUtility` delta)
    /// over an isolated index and one recording `trace_credit` adapter.
    Reference {
        index: Arc<IsolatedPipelineIndex>,
        adapters: RecordingAdapters,
    },
    /// The production assembly serving the signed production `package`
    /// (`TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY=production`), over
    /// `PipelineGateComponents::from_env`'s components on the operator host.
    /// Its settlement adapter is the production one, which records nothing:
    /// the drill counts settlement from the ledger instead. `rebuilt` is a
    /// second, empty index the resume rebuilds into for its last comparison.
    Production {
        package: Box<BundlePackage>,
        dependencies: HarnessDependencies,
        rebuilt: Option<RebuildIndex>,
    },
}

/// The resume's rebuild target: a second index, empty when the resume starts.
struct RebuildIndex {
    reader: Arc<dyn trace_commons_gate_api::IdentifiedIndexReader>,
    writer: Arc<dyn trace_commons_gate_api::IdentifiedIndexWriter>,
}

impl RestoreDependencies {
    fn reference() -> Self {
        Self::Reference {
            index: IsolatedPipelineIndex::new(),
            adapters: RecordingAdapters::new(),
        }
    }

    fn assembly(&self) -> HarnessAssembly {
        match self {
            Self::Reference { .. } => HarnessAssembly::Reference,
            Self::Production { .. } => HarnessAssembly::Production,
        }
    }

    /// Whether Score awards Trace Credit to a run that passes its floors:
    /// the reference candidate always does; a production package does unless
    /// its `NoveltyUtility` delta, `main`'s, is `0`, as on the pilot.
    fn awards_credit(&self) -> bool {
        match self {
            Self::Reference { .. } => true,
            Self::Production { package, .. } => {
                trace_commons_server::versioned_pipeline_bundle::package_compatibility_config(
                    package,
                )
                .unwrap_or_else(|| panic!("harness_production_package_invalid"))
                .novelty_utility_microcredits
                    > 0
            }
        }
    }

    /// The package the drill's result names: the one every service serves.
    fn package(&self) -> BundlePackage {
        match self {
            Self::Reference { .. } => {
                qualification_candidate_package().expect("restore_candidate_package_invalid")
            }
            Self::Production { package, .. } => package.as_ref().clone(),
        }
    }

    /// The service, on the runtime login, over `artifacts`, with the
    /// pass-through privacy boundary and `crash_point`.
    fn service(
        &self,
        runtime: &Arc<PgBackend>,
        store: &ConfiguredTraceArtifactStore,
        crash_point: Option<PipelineCrashPoint>,
    ) -> Arc<PipelineService> {
        match self {
            Self::Reference { index, adapters } => assemble_compatibility_pipeline_service_with(
                runtime.clone(),
                store,
                index.clone(),
                COMPATIBILITY_NOVELTY_UTILITY_MICROCREDITS,
                Arc::new(PassThroughPipelinePrivacyBoundary),
                adapters.registry(),
                crash_point,
            ),
            Self::Production {
                package,
                dependencies,
                ..
            } => assemble_production_harness_service(
                runtime.clone(),
                store,
                package,
                dependencies.clone(),
                crash_point,
            ),
        }
    }

    /// The live index's entries for `tenant`, as one hash: the isolated
    /// index's entry set, or the production index's snapshot hash (entry
    /// ids and content hashes, order-free), read through the trait.
    fn index_hash(&self, tenant: &TenantStorageRef) -> String {
        match self {
            Self::Reference { index, .. } => index.entry_set_hash(tenant, MINIMAL_INDEX_ID),
            Self::Production { dependencies, .. } => {
                index_snapshot_hash(dependencies.index_reader().as_ref(), tenant)
            }
        }
    }

    /// The hash an index with no entries for `tenant` has.
    fn empty_index_hash(&self, tenant: &TenantStorageRef) -> String {
        match self {
            Self::Reference { .. } => {
                IsolatedPipelineIndex::new().entry_set_hash(tenant, MINIMAL_INDEX_ID)
            }
            Self::Production { .. } => {
                index_snapshot_hash(IsolatedPipelineIndex::new().as_ref(), tenant)
            }
        }
    }

    /// The live index's writer, for the resume's rebuild.
    fn index_writer(&self) -> Arc<dyn trace_commons_gate_api::IdentifiedIndexWriter> {
        match self {
            Self::Reference { index, .. } => index.clone(),
            Self::Production { dependencies, .. } => dependencies.index_writer(),
        }
    }

    /// A second, empty index: its writer for a rebuild, and the hash of what
    /// the rebuild wrote, read after it.
    fn rebuilt_index(
        &self,
    ) -> (
        Arc<dyn trace_commons_gate_api::IdentifiedIndexWriter>,
        Box<dyn Fn(&TenantStorageRef) -> String + '_>,
    ) {
        match self {
            Self::Reference { .. } => {
                let rebuilt = IsolatedPipelineIndex::new();
                let reader = rebuilt.clone();
                (
                    rebuilt,
                    Box::new(move |tenant| reader.entry_set_hash(tenant, MINIMAL_INDEX_ID)),
                )
            }
            Self::Production { rebuilt, .. } => {
                let rebuilt = rebuilt.as_ref().expect("restore_rebuild_index_missing");
                (
                    rebuilt.writer.clone(),
                    Box::new(move |tenant| index_snapshot_hash(rebuilt.reader.as_ref(), tenant)),
                )
            }
        }
    }

    /// The run of every request the recording adapter received; `None` in
    /// production mode, whose adapter records nothing.
    fn request_runs(&self) -> Option<Vec<Uuid>> {
        match self {
            Self::Reference { adapters, .. } => Some(adapters.request_runs()),
            Self::Production { .. } => None,
        }
    }
}

/// The production compatibility index's snapshot hash for `tenant`.
fn index_snapshot_hash(
    reader: &dyn trace_commons_gate_api::IdentifiedIndexReader,
    tenant: &TenantStorageRef,
) -> String {
    reader
        .snapshot(tenant, PRODUCTION_COMPATIBILITY_INDEX_ID)
        .expect("restore_index_snapshot_failed")
        .snapshot_hash
}

/// The dependencies the two ignored tests run on: the reference ones, or,
/// under `TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY=production`, the signed
/// production package served over `PipelineGateComponents::from_env`'s
/// components, whose index opens at `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`
/// (`pipeline.py` gives the seed and the resume one each, inside the run).
/// With `rebuild`, a second, empty usearch index beside it is the resume's
/// rebuild target.
async fn restore_dependencies_from_env(rebuild: bool) -> RestoreDependencies {
    match HarnessAssembly::from_env().unwrap_or_else(|error| panic!("{error}")) {
        HarnessAssembly::Reference => RestoreDependencies::reference(),
        HarnessAssembly::Production => production_restore_dependencies_from_env(rebuild).await,
    }
}

#[cfg(feature = "near-ai-scorer")]
async fn production_restore_dependencies_from_env(rebuild: bool) -> RestoreDependencies {
    use trace_commons_server::versioned_pipeline_harness::{
        harness_dependencies_from_env, open_harness_rebuild_index, verified_package_from_lookup,
    };
    let lookup = |var: &str| std::env::var(var).ok();
    let package = verified_package_from_lookup(&lookup).unwrap_or_else(|error| panic!("{error}"));
    let dependencies = harness_dependencies_from_env()
        .await
        .unwrap_or_else(|error| panic!("{error}"));
    let rebuilt = rebuild.then(|| {
        let index = open_harness_rebuild_index().unwrap_or_else(|error| panic!("{error}"));
        RebuildIndex {
            reader: index.clone(),
            writer: index,
        }
    });
    RestoreDependencies::Production {
        package: Box::new(package),
        dependencies,
        rebuilt,
    }
}

/// Never reached: `HarnessAssembly::from_env` refuses production mode
/// without `near-ai-scorer`.
#[cfg(not(feature = "near-ai-scorer"))]
async fn production_restore_dependencies_from_env(_rebuild: bool) -> RestoreDependencies {
    panic!(
        "{}",
        trace_commons_server::versioned_pipeline_harness::HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL
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
    artifacts: &ConfiguredTraceArtifactStore,
    runtime: &Arc<PgBackend>,
    service: Arc<PipelineService>,
) -> Arc<AppState> {
    // `test_state_with_options`'s defaults, with the drill's own store.
    let mut state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
        state_dir.to_path_buf(),
        Some(mains.clone()),
        Some(artifacts.clone()),
        false,
        false,
        false,
        false,
        false,
        false,
        BTreeMap::new(),
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
    let dependencies = restore_dependencies_from_env(false).await;
    restore_seed(&config, &dependencies).await;
}

/// The seed over `dependencies`: the body of `pipeline_restore_seed`.
async fn restore_seed(config: &RestoreConfig, dependencies: &RestoreDependencies) {
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
    let store = RestoreStore::open(config).await;
    // A remote seed writes into a namespace of its own, fresh for the run:
    // anything already there is not this seed's.
    if let RestoreStore::Remote(remote) = &store {
        assert!(remote.is_empty(), "remote_restore_seed_store_not_empty");
    }
    let artifacts = store.configured();
    // Shared by both lifetimes (PF-2): the index and the recording adapter
    // see each other's writes only when the same instances back both apps.
    let start = |crash_point: Option<PipelineCrashPoint>| {
        let service = dependencies.service(&runtime, artifacts, crash_point);
        restore_app_state(state_dir.path(), &mains, artifacts, &runtime, service)
    };
    let client = reqwest::Client::new();

    // Lifetime 1: no crash point, so the first fixture completes. A crash
    // point fires at the first run that reaches it, so it cannot be set here.
    let first = start(None);
    let (base, stop, server) = serve_pipeline_app(first.clone()).await;
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
    // so the chain the restore must keep is not empty. A zero delta credits
    // nothing, so there is no credit event to wait for.
    let awards_credit = dependencies.awards_credit();
    for tenant in [RESTORE_TENANT, SECOND_TENANT] {
        if awards_credit {
            wait_for_credit_audited(&runtime, tenant).await;
        } else {
            // No run writes a `CreditMutate` event, so the seed appends one
            // hash-only read event through `main`'s mirrored audit log
            // instead: a chain the restore must keep.
            append_control_plane_read_audit(
                &first,
                &system_audit_tenant(tenant, "pipeline_restore_seed"),
                "pipeline_restore_seed",
                1,
            )
            .await
            .expect("restore_seed_audit_append_failed");
        }
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
    // The resume must have an index write and, unless the delta is zero,
    // legs to settle. The reference candidate always selects both; a
    // production package selects neither when the second fixture misses its
    // floors under the deployment's scorer and embedder, and the drill then
    // fails here, naming that, rather than in the resume.
    let (includes, settles) = settle_selection(&runtime, pending_envelope.submission_id).await;
    assert!(includes, "restore_seed_pending_selection_excludes");
    assert!(
        settles || !awards_credit,
        "restore_seed_pending_selection_settles_nothing"
    );
    assert!(
        !settles || awards_credit,
        "restore_seed_pending_selection_settles_under_zero_delta"
    );
    stop.send(())
        .expect("send shutdown to the seed's second app");
    join_within(server, 20, "the seed's second app").await;
    // P3's time shortcut, as the restart test takes it: the crashed run's
    // lease expires now instead of five minutes from now. Not a processor
    // call; a restore taken after a longer outage finds it expired anyway.
    expire_run_lease(&runtime, RESTORE_TENANT, pending_envelope.submission_id).await;
    // PR 5's activation state, for a tenant of its own (G23), written before
    // the fingerprints are taken so that they cover it.
    seed_activation_state(&runtime, &dependencies.service(&runtime, artifacts, None)).await;
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
        completed.settled_as_awarded(awards_credit),
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
            && second.settled_as_awarded(awards_credit),
        "restore_seed_second_tenant_run_not_credited"
    );
    // The adapter saw the completed run's legs and the second tenant's, and
    // nothing of the pending run. Only `RESTORE_TENANT`'s count is recorded:
    // the resume adds the pending run's to it. The production adapter
    // records nothing (its effect is the ledger row), so in production mode
    // the count is the completed run's settlement legs, which the ledger
    // assertions above already tie to one credit event each.
    let completed_requests = match dependencies.request_runs() {
        Some(request_runs) => {
            let completed_requests = request_runs
                .iter()
                .filter(|run| **run == completed.run_id)
                .count();
            assert!(
                request_runs
                    .iter()
                    .all(|run| *run == completed.run_id || *run == second.run_id)
                    && completed_requests == completed.settlement_count
                    && request_runs.len() - completed_requests == second.settlement_count,
                "restore_seed_adapter_requests_unexpected"
            );
            completed_requests
        }
        None => completed.settlement_count,
    };
    assert!(
        (completed.settlement_count > 0) == awards_credit,
        "restore_seed_adapter_requests_unexpected"
    );
    let tenant = pipeline_tenant_storage_ref(RESTORE_TENANT);
    let entry_set_hash = dependencies.index_hash(&tenant);
    assert_ne!(
        entry_set_hash,
        dependencies.empty_index_hash(&tenant),
        "restore_seed_index_empty"
    );
    let artifact_fingerprint = store.fingerprint();

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
        credit_delta_zero: !awards_credit,
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
    let url = restored_database_url()
        .await
        .expect("restore_database_url_missing");
    let dependencies = restore_dependencies_from_env(true).await;
    restore_resume(&config, &url, &dependencies).await;
}

/// The resume over `dependencies` against the restored database at `url`:
/// the body of `pipeline_restore_resume`.
async fn restore_resume(config: &RestoreConfig, url: &str, dependencies: &RestoreDependencies) {
    if let RestoreArtifacts::Remote(remote) = &config.artifacts {
        assert!(
            remote.resume_report_path.is_some(),
            "remote_restore_resume_report_path_missing"
        );
    }
    let seed = RestoreFingerprint::parse(
        &std::fs::read(&config.fingerprint_path).expect("restore_fingerprint_unreadable"),
    )
    .unwrap_or_else(|label| panic!("{label}"));
    // `runtime_backend_at` checks the login is neither SUPERUSER nor
    // BYPASSRLS before anything else connects.
    let runtime = runtime_backend_at(url, 8).await;
    let mains = mains_database_at(url).await;
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
        PgBackend::new(&DatabaseConfig::from_postgres_url(url, 2))
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
    let store = RestoreStore::open(config).await;
    let artifact_fingerprint = store.fingerprint();
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
    let artifacts = store.configured();
    let service = dependencies.service(&runtime, artifacts, None);
    let tenant = pipeline_tenant_storage_ref(RESTORE_TENANT);
    let rebuild = service
        .rebuild_index_from_authoritative_commands(RESTORE_TENANT, dependencies.index_writer())
        .await
        .expect("restore_index_rebuild_failed");
    assert_eq!(
        rebuild.command_count,
        completed.len(),
        "restore_index_rebuild_command_count"
    );
    let index_entry_set_hash = dependencies.index_hash(&tenant);
    assert_eq!(
        index_entry_set_hash, seed.index_entry_set_hash,
        "restore_index_entry_set_mismatch"
    );

    // The app on the restored state resumes the pending run by itself. The
    // drill is one of the four checks that test the qualification
    // candidate (P5-D15), so its result names that package: the service the
    // drill resumed on serves exactly it.
    let package = dependencies.package();
    assert!(
        *service.default_package() == package,
        "restore_service_not_the_candidate"
    );
    let state = restore_app_state(
        state_dir.path(),
        &mains,
        artifacts,
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
    // run (same bundle, same award), and at least one, as the seed recorded;
    // none under a zero delta, which the seed recorded too.
    let awards_credit = dependencies.awards_credit();
    assert!(
        seed.credit_delta_zero != awards_credit,
        "restore_credit_delta_changed"
    );
    assert!(
        (resumed.settlement_count > 0) == awards_credit
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
        resumed.settled_as_awarded(awards_credit)
            && resumed.credit_events == seed.completed_credit_event_count,
        "restore_pending_run_credit_events"
    );
    // The adapter was asked for the resumed run's legs and nothing else. In
    // production mode (an adapter that records nothing) the ledger answers
    // instead: the legs settled after the restore are the resumed run's.
    let resumed_requests = match dependencies.request_runs() {
        Some(request_runs) => {
            assert!(
                request_runs.iter().all(|run| *run == pending.run_id)
                    && request_runs.len() == resumed.settlement_count,
                "restore_adapter_requests_unexpected"
            );
            request_runs.len()
        }
        None => resumed.settlement_count,
    };
    let legs: usize = after.iter().map(|run| run.settlement_count).sum();
    assert_eq!(
        seed.adapter_request_count + resumed_requests,
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
    let (rebuilt, rebuilt_hash) = dependencies.rebuilt_index();
    service
        .rebuild_index_from_authoritative_commands(RESTORE_TENANT, rebuilt)
        .await
        .expect("restore_index_rebuild_failed");
    assert_eq!(
        dependencies.index_hash(&tenant),
        rebuilt_hash(&tenant),
        "restore_resumed_index_mismatch"
    );
    assert_ne!(
        dependencies.index_hash(&tenant),
        seed.index_entry_set_hash,
        "restore_resumed_index_unchanged"
    );

    // A remote resume reports to `promote remote-restore`, which writes
    // `pipeline_remote_restore` from it; the production run's own
    // `pipeline_restore_drill` comes from its package checks, so nothing
    // is emitted here.
    if let RestoreArtifacts::Remote(remote) = &config.artifacts {
        let report = serde_json::json!({
            "schema": REMOTE_RESUME_REPORT_SCHEMA,
            "database_fingerprint": database_fingerprint,
            "artifact_fingerprint": artifact_fingerprint,
            "pending_runs_resumed": pending_runs_resumed,
            "duplicate_effects": duplicate_effects,
        });
        let mut bytes = trace_commons_protocol::canonical_json::to_canonical_vec(&report)
            .expect("the resume report serialises");
        bytes.push(b'\n');
        write_atomically(
            remote
                .resume_report_path
                .as_deref()
                .unwrap_or_else(|| panic!("remote_restore_resume_report_path_missing")),
            &bytes,
        );
        return;
    }
    let mut evidence = serde_json::json!({
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
    });
    // A reference result's evidence is exactly what it always was.
    if dependencies.assembly() == HarnessAssembly::Production {
        evidence["harness_assembly"] = serde_json::json!(dependencies.assembly().label());
    }
    // Named only when true: a drill that settled no leg says so, and one
    // that did reads as it always has.
    if seed.credit_delta_zero {
        evidence["credit_delta_zero"] = serde_json::json!(true);
    }
    PipelineCheckEmitter::emit_from_env(
        RESTORE_CHECK_ID,
        PipelineCheckStatus::Pass,
        Some(&package),
        &RESTORE_SAFE_BLOCKERS,
        evidence,
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
            artifacts: RestoreArtifacts::Local(PathBuf::from("/tmp/root")),
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
        credit_delta_zero: false,
    };
    assert_eq!(
        RestoreFingerprint::parse(&fingerprint.to_bytes()),
        Ok(fingerprint.clone())
    );
    // A zero-delta seed settled and credited nothing, and says so; a seed
    // that says so and counts a leg or an event is refused.
    let uncredited = RestoreFingerprint {
        adapter_request_count: 0,
        completed_settlement_count: 0,
        completed_credit_event_count: 0,
        credit_delta_zero: true,
        ..fingerprint.clone()
    };
    assert_eq!(
        RestoreFingerprint::parse(&uncredited.to_bytes()),
        Ok(uncredited.clone())
    );
    for count in [
        "adapter_request_count",
        "completed_settlement_count",
        "completed_credit_event_count",
    ] {
        let mut value = serde_json::to_value(&uncredited).unwrap();
        value[count] = 1.into();
        assert_eq!(
            RestoreFingerprint::parse(&serde_json::to_vec(&value).unwrap()),
            Err("restore_fingerprint_invalid"),
            "{count}"
        );
    }
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

/// An embedder double that puts every distinct text on its own axis, so
/// any two of the drill's fixtures are fully novel to each other. The
/// reference embedder puts the first two corpus fixtures at novelty 0.44,
/// under the production test package's 0.5 floor, so the pending run would
/// select nothing to settle.
struct DistinctTextEmbedder;

impl trace_commons_gate_api::Embedder for DistinctTextEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        use sha2::Digest as _;
        let digest = sha2::Sha256::digest(plaintext);
        let axis = u64::from_be_bytes(digest[..8].try_into().expect("eight bytes")) % 1024;
        let mut vector = vec![0.0f32; 1024];
        vector[axis as usize] = 1.0;
        Ok(vector)
    }
}

/// Production restore dependencies over doubles, never production-qualified
/// (the operator's run has `from_env`'s NEAR AI, fastembed and usearch index,
/// and a second usearch index, in their place):
/// their service is the production assembly serving the production
/// package, their index hash is the snapshot read through the trait, and
/// they record no adapter requests, so the drill counts settlement from the
/// ledger. Needs no database: assembly opens no connection.
fn production_restore_doubles(package: BundlePackage) -> RestoreDependencies {
    let index = IsolatedPipelineIndex::new();
    let rebuilt = IsolatedPipelineIndex::new();
    RestoreDependencies::Production {
        package: Box::new(package),
        dependencies: HarnessDependencies::Doubles(
            trace_commons_server::versioned_pipeline_harness::HarnessDoubles {
                scorer: Arc::new(trace_commons_gate_api::ReferencePerplexityScorer::new()),
                embedder: Arc::new(DistinctTextEmbedder),
                index_reader: index.clone(),
                index_writer: index,
            },
        ),
        rebuilt: Some(RebuildIndex {
            reader: rebuilt.clone(),
            writer: rebuilt,
        }),
    }
}

#[tokio::test]
async fn production_restore_dependencies_serve_the_production_package() {
    let unused_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let runtime = Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(
            &format!("postgres://nobody@127.0.0.1:{unused_port}/none"),
            1,
        ))
        .await
        .unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let artifacts = ConfiguredTraceArtifactStore::legacy(test_artifact_store_with_key(
        dir.path(),
        &trace_commons_server::secrets::keychain::generate_master_key_hex(),
    ));
    let package = production_test_package();
    let dependencies = production_restore_doubles(package.clone());
    assert_eq!(dependencies.assembly(), HarnessAssembly::Production);
    assert_eq!(dependencies.package(), package);
    assert!(dependencies.request_runs().is_none());
    for crash_point in [None, Some(PipelineCrashPoint::AfterSettleSelection)] {
        let service = dependencies.service(&runtime, &artifacts, crash_point);
        assert!(*service.default_package() == package);
        let qualification = service.bundle_qualification(&package).unwrap();
        assert_eq!(qualification.scorer.identity, "near_ai_perplexity_scorer");
        assert_eq!(qualification.embedder.identity, "fastembed_text_embedder");
        // Only `PipelineGateComponents::from_env` qualifies the adapters.
        assert!(!qualification.scorer.production_qualified);
        assert!(!qualification.embedder.production_qualified);
    }
    let tenant = pipeline_tenant_storage_ref(RESTORE_TENANT);
    assert_eq!(
        dependencies.index_hash(&tenant),
        dependencies.empty_index_hash(&tenant)
    );
    let (_, rebuilt_hash) = dependencies.rebuilt_index();
    assert_eq!(
        rebuilt_hash(&tenant),
        dependencies.empty_index_hash(&tenant)
    );
    // The reference dependencies serve the reference candidate, as before.
    let reference = RestoreDependencies::reference();
    assert_eq!(reference.assembly(), HarnessAssembly::Reference);
    assert_eq!(
        reference.package(),
        qualification_candidate_package().unwrap()
    );
    assert_eq!(reference.request_runs(), Some(Vec::new()));
}

/// The restore drill end to end in production mode over doubles:
/// the seed, then the resume against the same database (no dump and
/// restore between, which `pipeline.py restore-drill` adds), through the
/// bodies the two ignored tests run. Ignored: it writes the drill's fixed
/// tenants into the shared pilot-shaped database, which the rest of the
/// suite also uses, so it runs alone:
/// `cargo test -p trace-commons-server --bin trace-commons-ingest
/// production_restore_drill_resumes_once_over_doubles -- --ignored --exact`
/// against a fresh `TRACE_COMMONS_PG_TEST_DATABASE_URL`.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes the drill's fixed tenants into the shared database: run alone"]
async fn production_restore_drill_resumes_once_over_doubles() {
    let Some(url) = pipeline_http_database_url().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let config = RestoreConfig {
        artifacts: RestoreArtifacts::Local(dir.path().join("artifacts")),
        master_key_hex: trace_commons_server::secrets::keychain::generate_master_key_hex(),
        fingerprint_path: dir.path().join("fingerprint.json"),
    };
    let dependencies = production_restore_doubles(production_test_package());
    restore_seed(&config, &dependencies).await;
    let resume = production_restore_doubles(production_test_package());
    restore_resume(&config, &url, &resume).await;
}

/// The production restore drill over doubles under a package whose
/// `NoveltyUtility` delta is `0`, the pilot's: Score awards nothing, so
/// neither run settles a leg or writes a ledger event, and the drill
/// checks the restore and the one resume without them. Ignored for the
/// reason `production_restore_drill_resumes_once_over_doubles` is.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "writes the drill's fixed tenants into the shared database: run alone"]
async fn production_restore_drill_resumes_once_without_credit_over_doubles() {
    let Some(url) = pipeline_http_database_url().await else {
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let config = RestoreConfig {
        artifacts: RestoreArtifacts::Local(dir.path().join("artifacts")),
        master_key_hex: trace_commons_server::secrets::keychain::generate_master_key_hex(),
        fingerprint_path: dir.path().join("fingerprint.json"),
    };
    let dependencies = production_restore_doubles(production_test_package_awarding(0));
    restore_seed(&config, &dependencies).await;
    let resume = production_restore_doubles(production_test_package_awarding(0));
    restore_resume(&config, &url, &resume).await;
}

fn vars_from(set: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> + use<> {
    let set: BTreeMap<String, String> = set
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect();
    move |name: &str| set.get(name).cloned()
}

/// `promote remote-restore` replaces the artifact root with a remote store:
/// the store's name, its kind, and the drill's namespace under it, never
/// both an artifact root and a remote store.
#[test]
fn restore_config_reads_a_remote_store_in_place_of_an_artifact_root() {
    let remote = [
        (REMOTE_ARTIFACT_STORE_VAR, "tracecommons-scratch/drill"),
        (REMOTE_STORE_KIND_VAR, "gcs"),
        (
            REMOTE_NAMESPACE_VAR,
            "pipeline-remote-restore-q0000000a-1234abcd",
        ),
        (TEST_MASTER_KEY_VAR, "00"),
        (RESTORE_FINGERPRINT_PATH_VAR, "/tmp/fingerprint.json"),
    ];
    let expected_location = RemoteStoreLocation {
        bucket: "tracecommons-scratch".to_string(),
        prefix: "drill/pipeline-remote-restore-q0000000a-1234abcd".to_string(),
    };
    assert_eq!(
        RestoreConfig::from_vars(vars_from(&remote)),
        Ok(Some(RestoreConfig {
            artifacts: RestoreArtifacts::Remote(RemoteArtifacts {
                kind: RemoteStoreKind::Gcs,
                location: expected_location.clone(),
                resume_report_path: None,
            }),
            master_key_hex: "00".to_string(),
            fingerprint_path: PathBuf::from("/tmp/fingerprint.json"),
        }))
    );
    let mut with_report = remote.to_vec();
    with_report.push((REMOTE_RESUME_REPORT_PATH_VAR, "/tmp/resume.json"));
    let Ok(Some(RestoreConfig {
        artifacts: RestoreArtifacts::Remote(read),
        ..
    })) = RestoreConfig::from_vars(vars_from(&with_report))
    else {
        panic!("a remote config with a resume report reads");
    };
    assert_eq!(
        read.resume_report_path,
        Some(PathBuf::from("/tmp/resume.json"))
    );

    let mut double = remote.to_vec();
    double[1] = (REMOTE_STORE_KIND_VAR, "gcs_directory_double");
    double.push((REMOTE_DOUBLE_ROOT_VAR, "/tmp/buckets"));
    let Ok(Some(RestoreConfig {
        artifacts: RestoreArtifacts::Remote(read),
        ..
    })) = RestoreConfig::from_vars(vars_from(&double))
    else {
        panic!("a directory-double config reads");
    };
    assert_eq!(
        read.kind,
        RemoteStoreKind::DirectoryDouble(PathBuf::from("/tmp/buckets"))
    );

    let refused = |set: Vec<(&str, &str)>, label: &str| {
        assert_eq!(
            RestoreConfig::from_vars(vars_from(&set)),
            Err(label),
            "{set:?}"
        );
    };
    // Both an artifact root and a remote store.
    let mut both = remote.to_vec();
    both.push((ARTIFACT_ROOT_VAR, "/tmp/root"));
    refused(both, "restore_environment_incomplete");
    // A remote store without its kind or namespace.
    for missing in [REMOTE_STORE_KIND_VAR, REMOTE_NAMESPACE_VAR] {
        refused(
            remote
                .iter()
                .copied()
                .filter(|(key, _)| *key != missing)
                .collect(),
            "restore_environment_incomplete",
        );
    }
    // A double with no root, a root with no double, an unknown kind.
    let mut no_root = remote.to_vec();
    no_root[1] = (REMOTE_STORE_KIND_VAR, "gcs_directory_double");
    refused(no_root, "restore_environment_incomplete");
    let mut stray_root = remote.to_vec();
    stray_root.push((REMOTE_DOUBLE_ROOT_VAR, "/tmp/buckets"));
    refused(stray_root, "restore_environment_incomplete");
    let mut unknown = remote.to_vec();
    unknown[1] = (REMOTE_STORE_KIND_VAR, "s3");
    refused(unknown, "remote_restore_store_kind_invalid");
    // A store name or namespace outside the shapes.
    for name in [
        "gs://bucket",
        "Bucket",
        "bucket/../live",
        "bucket//x",
        "bucket/",
    ] {
        let mut bad = remote.to_vec();
        bad[0] = (REMOTE_ARTIFACT_STORE_VAR, name);
        refused(bad, "remote_restore_store_name_invalid");
    }
    for namespace in ["", "a/b", "..", "has space"] {
        let mut bad = remote.to_vec();
        bad[2] = (REMOTE_NAMESPACE_VAR, namespace);
        refused(bad, "remote_restore_store_name_invalid");
    }
}

#[test]
fn remote_store_names_take_a_bucket_and_an_optional_prefix() {
    assert_eq!(
        RemoteStoreLocation::parse("tracecommons-artifacts", "ns"),
        Ok(RemoteStoreLocation {
            bucket: "tracecommons-artifacts".to_string(),
            prefix: "ns".to_string(),
        })
    );
    assert_eq!(
        RemoteStoreLocation::parse("tracecommons-artifacts/a/b_c", "ns"),
        Ok(RemoteStoreLocation {
            bucket: "tracecommons-artifacts".to_string(),
            prefix: "a/b_c/ns".to_string(),
        })
    );
    // `promote`'s overlap rule: one name is the other or holds it.
    let overlaps = |a: &str, b: &str| remote_store_names_overlap(a, b);
    assert!(overlaps("live", "live"));
    assert!(overlaps("live", "live/scratch"));
    assert!(overlaps("live/a", "live"));
    assert!(!overlaps("live/a", "live/ab"));
    assert!(!overlaps("live", "scratch"));
}

#[test]
fn remote_copy_config_requires_every_variable_or_none() {
    let full = [
        (REMOTE_STORE_KIND_VAR, "gcs"),
        (REMOTE_SOURCE_STORE_VAR, "tracecommons-artifacts"),
        (REMOTE_SCRATCH_STORE_VAR, "tracecommons-scratch/drill"),
        (REMOTE_NAMESPACE_VAR, "ns"),
        (TEST_MASTER_KEY_VAR, "00"),
        (REMOTE_COPY_REPORT_PATH_VAR, "/tmp/copy.json"),
    ];
    assert_eq!(RemoteCopyConfig::from_vars(vars_from(&[])), Ok(None));
    assert_eq!(
        RemoteCopyConfig::from_vars(vars_from(&full)),
        Ok(Some(RemoteCopyConfig {
            kind: RemoteStoreKind::Gcs,
            source: RemoteStoreLocation {
                bucket: "tracecommons-artifacts".to_string(),
                prefix: "ns".to_string(),
            },
            scratch: RemoteStoreLocation {
                bucket: "tracecommons-scratch".to_string(),
                prefix: "drill/ns".to_string(),
            },
            master_key_hex: "00".to_string(),
            report_path: PathBuf::from("/tmp/copy.json"),
        }))
    );
    for index in 0..full.len() {
        let mut partial = full.to_vec();
        partial.remove(index);
        assert_eq!(
            RemoteCopyConfig::from_vars(vars_from(&partial)),
            Err("remote_restore_environment_incomplete"),
            "{partial:?}"
        );
    }
    let mut same = full.to_vec();
    same[2] = (REMOTE_SCRATCH_STORE_VAR, "tracecommons-artifacts/scratch");
    assert_eq!(
        RemoteCopyConfig::from_vars(vars_from(&same)),
        Err("remote_restore_scratch_overlaps_live_store")
    );
}

#[test]
fn a_directory_double_round_trips_lists_and_reports_no_versioning() {
    use trace_commons_server::trace_artifact_gcs::GcsObjectClient as _;
    let root = tempfile::tempdir().expect("temp dir");
    let client = DirectoryGcsObjectClient::new(root.path().join("bucket"));
    client
        .put_object("a/b/1", bytes::Bytes::from_static(b"one"), BTreeMap::new())
        .unwrap();
    client
        .put_object("a/2", bytes::Bytes::from_static(b"two"), BTreeMap::new())
        .unwrap();
    client
        .put_object("c/3", bytes::Bytes::from_static(b"three"), BTreeMap::new())
        .unwrap();
    assert_eq!(&client.get_object("a/b/1").unwrap().body[..], b"one");
    assert_eq!(client.list_object_keys("a/").unwrap(), ["a/2", "a/b/1"]);
    assert!(client.delete_object("a/2").unwrap());
    assert!(!client.delete_object("a/2").unwrap());
    let missing = client.get_object("a/2").map(|_| ()).unwrap_err();
    assert!(
        trace_commons_server::trace_artifact_store::is_trace_artifact_integrity_error(&missing)
    );
    assert!(!client.bucket_versioning_enabled().unwrap());
    for key in ["", "a//b", "../x", "a/./b", "/a"] {
        assert!(
            client
                .put_object(key, bytes::Bytes::new(), BTreeMap::new())
                .is_err(),
            "{key:?}"
        );
    }
}

/// The copy step end to end over the directory double, in one process: the
/// seed's objects (written through the drill's own store) come back in the
/// scratch store with the same fingerprint, every one unwraps, and the
/// double reports its kind and no versioning.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_remote_copy_step_restores_a_seeded_store_over_the_directory_double() {
    let root = tempfile::tempdir().expect("temp dir");
    let master_key_hex = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let kind = RemoteStoreKind::DirectoryDouble(root.path().to_path_buf());
    let source = RemoteStoreLocation::parse("tracecommons-live", "ns").unwrap();
    let seeded = RemoteRestoreStore::open(&kind, &source, &master_key_hex).await;
    assert!(seeded.is_empty(), "a fresh namespace holds nothing");
    for index in 0..3 {
        seeded
            .configured()
            .store
            .put_serialized_json(
                "tenant:sha256:a",
                TraceArtifactKind::ContributionEnvelope,
                &format!("object-{index}"),
                br#"{"n":1}"#,
            )
            .expect("the seed writes");
    }
    let seeded_fingerprint = seeded.fingerprint();
    let report_path = root.path().join("reports").join("copy.json");
    let report = run_remote_copy(&RemoteCopyConfig {
        kind,
        source,
        scratch: RemoteStoreLocation::parse("tracecommons-scratch/drill", "ns").unwrap(),
        master_key_hex,
        report_path: report_path.clone(),
    })
    .await;
    assert_eq!(
        report,
        serde_json::json!({
            "schema": "trace_commons.pipeline_remote_restore_copy.v1",
            "object_store_kind": "gcs_directory_double",
            "object_count": 3,
            "artifact_fingerprint": seeded_fingerprint,
            "restored_artifact_fingerprint": seeded_fingerprint,
            "kek_unwrap_verified_count": 3,
            "versioning_enabled": false,
        })
    );
    let written: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(written, report);
    assert!(
        root.path().join("tracecommons-scratch/drill/ns").is_dir(),
        "the restore lands under the scratch prefix and namespace"
    );
}

/// A drill on GCS proves the restored objects unwrap under the deployment's
/// key wrapper only when that wrapper is a production one: under the local
/// master key every count would pass and say nothing about KMS. The double
/// runs on the local key.
/// The remote drill's key wrapper and GCS client open TLS connections from a
/// test binary, which never runs `main()`'s rustls provider install. With two
/// providers compiled in (the pilot's feature set), rustls panics at the
/// first TLS use unless one is installed, so building the key wrapper
/// installs it. Run alone (`--exact`): another test may install it first.
#[tokio::test]
async fn the_remote_key_wrapper_installs_the_tls_provider() {
    let _kek = remote_kek(
        &RemoteStoreKind::DirectoryDouble(PathBuf::from("/nonexistent")),
        &trace_commons_server::secrets::keychain::generate_master_key_hex(),
    )
    .await;
    assert!(rustls::crypto::CryptoProvider::get_default().is_some());
}

#[test]
fn a_gcs_drill_refuses_a_key_wrapper_that_is_not_production() {
    assert_eq!(
        require_production_kek(&RemoteStoreKind::Gcs, false),
        Err("remote_restore_kek_not_production")
    );
    assert_eq!(require_production_kek(&RemoteStoreKind::Gcs, true), Ok(()));
    let double = RemoteStoreKind::DirectoryDouble(PathBuf::from("/tmp/buckets"));
    assert_eq!(require_production_kek(&double, false), Ok(()));
}
