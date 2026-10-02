// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The legacy drain report (PR 5, Task 6). Before a tenant's legacy writer
//! could ever be retired, the work the legacy path still owes for the receipts
//! it already took must be finished by the legacy code. These tests drive real
//! legacy receipts and real legacy routes (`main`'s review decision, gate
//! evaluation, vector index, delayed credit, and settlement routes) over a
//! real PostgreSQL, and read `PipelineActivationStore::legacy_drain_report`
//! before and after, so a count that reads zero while the legacy path still
//! owes work fails here.
//!
//! The app is served as a plain router, not through `run_pipeline_app`, so no
//! pipeline worker runs: the pipeline's receipts stay pending, which is the
//! point of the rehearsal (the report counts only what no pipeline run owns).
//!
//! Nested inside `tests` beside `pipeline_http_pg_tests`, whose `pub(super)`
//! helpers it reuses: `sample_envelope`, `auth_headers`, and the other state
//! builders are private to `tests` and visible only to its descendants.

use super::*;

use std::collections::BTreeMap;
use std::sync::Arc;

use super::pipeline_http_pg_tests::{
    PassThroughPipelinePrivacyBoundary, account_owner_backend,
    assemble_compatibility_pipeline_service_with, mains_database, pipeline_http_database_url,
    route_request, route_trace, runtime_backend, tenant_tx, write_routing_as_operator,
};
use tokio_postgres::types::ToSql;
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::InstrumentId;
use trace_commons_server::versioned_pipeline_activation::{
    LegacyDrainReport, PipelineActivationStore, RoutingState,
};
use trace_commons_server::versioned_pipeline_credit::RecordingSettlementAdapter;
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_qualification::{PipelineCheckEmitter, evidence_hash};

/// The gate driver's attempt ceiling in these tests: the value `main`'s own
/// gate tests pass to `list_submissions_needing_gate_decision`.
const GATE_MAX_ATTEMPTS: i32 = 5;

/// The eight labels of the report, in the order of the plan's table.
const DRAIN_LABELS: [&str; 8] = [
    "awaiting_pii_backstop",
    "gate_decision_pending",
    "gate_decision_exhausted",
    "quarantine_review_pending",
    "vector_index_pending",
    "delayed_credit_unsettled",
    "revocation_propagation_pending",
    "near_outbox_pending",
];

/// A login whose only privilege source is membership in `trace_gate_driver`,
/// the role `main`'s gate driver pool connects as (V36, V45, V105), and a
/// backend whose gate driver pool connects as it: the gate enumeration
/// (`list_submissions_needing_gate_decision`) the report must agree with then
/// reads every table through that role's grants and policies, as it does in
/// production. A login of this suite's own: the roles of the other suites are
/// left as they are.
async fn gate_driver_backend() -> PgBackend {
    const GATE_DRIVER_LOGIN: &str = "trace_gate_driver_pr5_drain_test";
    let url = pipeline_http_database_url()
        .await
        .expect("the suite's database variable is set");
    let (owner, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect as the database owner");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    owner
        .batch_execute(&format!(
            "DO $$ BEGIN
                 IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = '{GATE_DRIVER_LOGIN}') THEN
                     CREATE ROLE {GATE_DRIVER_LOGIN} LOGIN NOSUPERUSER NOBYPASSRLS;
                 END IF;
             END $$;
             GRANT trace_gate_driver TO {GATE_DRIVER_LOGIN};"
        ))
        .await
        .expect("provision the gate driver login");
    let mut gate_url = reqwest::Url::parse(&url).expect("parse the suite's URL");
    gate_url
        .set_username(GATE_DRIVER_LOGIN)
        .expect("set the gate driver user");
    let mut config = DatabaseConfig::from_postgres_url(&url, 2);
    config.gate_driver_url = Some(SecretString::from(gate_url.to_string()));
    PgBackend::new(&config)
        .await
        .expect("connect with a gate driver pool")
}

/// A metadata-only, Low-risk envelope that allows model training, under a
/// fresh submission id and a distinguishing tool name: the legacy path
/// accepts it, the pipeline's Admission accepts it, and `main`'s vector index
/// and delayed credit apply to it.
async fn clean_envelope(tool_name: &str) -> TraceContributionEnvelope {
    let mut envelope = sample_envelope().await;
    envelope.submission_id = Uuid::new_v4();
    envelope.trace_id = Uuid::new_v4();
    make_metadata_only_low_risk(&mut envelope);
    set_metadata_only_tool_name(&mut envelope, tool_name);
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

/// An envelope the legacy path quarantines: the consent flags declare text and
/// payloads and the residual risk is Medium (`main`'s quarantine fixture).
async fn legacy_quarantine_envelope(text: &str) -> TraceContributionEnvelope {
    let mut envelope = sample_envelope_with_user_input(text).await;
    envelope.submission_id = Uuid::new_v4();
    envelope.trace_id = Uuid::new_v4();
    envelope.consent.message_text_included = true;
    envelope.consent.tool_payloads_included = true;
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    envelope
}

/// A clean envelope whose residual risk is Medium: the pipeline's Admission
/// quarantines it (`quarantined_pipeline_run`).
async fn pipeline_quarantine_envelope(tool_name: &str) -> TraceContributionEnvelope {
    let mut envelope = clean_envelope(tool_name).await;
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    envelope
}

/// Two tenants over one database, one artifact store, and one app: `tenant`
/// is the one the tests route and report on, `other_tenant` a bystander. The
/// legacy path runs `main`'s service-owned (KEK-wrapped) artifact store, which
/// the gate worker reads, and a pipeline service on the same store takes the
/// receipts of a tenant whose row says `pipeline`. No worker runs.
struct DrainFixture {
    owner: Arc<PgBackend>,
    gate: PgBackend,
    store: Arc<PipelineActivationStore>,
    state: Arc<AppState>,
    tenant: String,
    contributor: String,
    reviewer: String,
    worker: String,
    admin: String,
    other_tenant: String,
    other_contributor: String,
    _dir: tempfile::TempDir,
    _artifact_dir: tempfile::TempDir,
}

impl DrainFixture {
    async fn new() -> Option<Self> {
        let runtime = runtime_backend(6).await?;
        let owner = account_owner_backend()
            .await
            .expect("the same variable runtime_backend read is set");
        let gate = gate_driver_backend().await;
        let suffix = Uuid::new_v4().simple().to_string();
        let tenant = format!("tenant-drain-{suffix}");
        let other_tenant = format!("tenant-drain-other-{suffix}");
        let contributor = format!("token-drain-{suffix}");
        let reviewer = format!("token-drain-review-{suffix}");
        let worker = format!("token-drain-worker-{suffix}");
        let admin = format!("token-drain-admin-{suffix}");
        let other_contributor = format!("token-drain-other-{suffix}");
        let dir = tempfile::tempdir().expect("temp dir");
        let artifact_dir = tempfile::tempdir().expect("artifact dir");
        // The gate worker reads only KEK-wrapped (v2) envelopes, so the legacy
        // path and the pipeline both store into the service-owned store the
        // gate-worker tests use.
        let (configured_store, _) = fixture_gate_worker_artifact_store(artifact_dir.path());
        let trace_credit: Arc<dyn SettlementAdapter> = RecordingSettlementAdapter::new(
            InstrumentId::trace_credit(),
            "recording_trace_credit_drain_test_only",
            "none",
        );
        let service = assemble_compatibility_pipeline_service_with(
            runtime.clone(),
            &configured_store,
            IsolatedPipelineIndex::new(),
            2_500_000,
            Arc::new(PassThroughPipelinePrivacyBoundary),
            vec![trace_credit],
            None,
        );
        service
            .register_default_bundle(&tenant)
            .await
            .expect("register the default bundle");
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, &tenant, &contributor, TokenRole::Contributor);
        insert_token(&mut tokens, &tenant, &reviewer, TokenRole::Reviewer);
        insert_token(&mut tokens, &tenant, &worker, TokenRole::VectorWorker);
        insert_token(&mut tokens, &tenant, &admin, TokenRole::Admin);
        insert_token(
            &mut tokens,
            &other_tenant,
            &other_contributor,
            TokenRole::Contributor,
        );
        let mut state = test_state_with_configured_artifact_store_policies_and_export_guardrails(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(configured_store),
            true,
            true,
            false,
            false,
            false,
            false,
            BTreeMap::new(),
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.tokens = Arc::new(tokens);
        state_mut.require_db_mirror_writes = true;
        state_mut.gate_service = Arc::new(InMemoryGateService::new(
            "drain_gate_v1",
            "sha256:drain_gate_v1",
        ));
        state_mut.pipeline_service = Some(service);
        state_mut.pipeline_activation = routing_store(&runtime);
        state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
            TraceTenantRolloutFeature::PipelineReceipts,
            &[tenant.as_str()],
        );
        let store = routing_store(&runtime).expect("a routing store");
        Some(Self {
            owner,
            gate,
            store,
            state,
            tenant,
            contributor,
            reviewer,
            worker,
            admin,
            other_tenant,
            other_contributor,
            _dir: dir,
            _artifact_dir: artifact_dir,
        })
    }

    /// `POST /v1/traces` of `envelope` by `token`, through the plain router.
    async fn upload(
        &self,
        token: &str,
        envelope: &TraceContributionEnvelope,
    ) -> (StatusCode, serde_json::Value) {
        route_trace(&self.state, token, &serde_json::to_vec(envelope).unwrap()).await
    }

    /// An upload that must succeed, with its receipt.
    async fn upload_ok(
        &self,
        token: &str,
        envelope: &TraceContributionEnvelope,
    ) -> serde_json::Value {
        let (status, receipt) = self.upload(token, envelope).await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        receipt
    }

    /// A route call with `token`, which must answer 200.
    async fn call_ok(
        &self,
        method: &str,
        uri: &str,
        token: &str,
        body: serde_json::Value,
    ) -> serde_json::Value {
        let (status, body) = route_request(
            self.state.clone(),
            method,
            uri,
            auth_headers(token),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{method} {uri}: {body}");
        body
    }

    /// The report of `tenant`, with the gate ceiling of these tests.
    async fn report(&self, tenant: &str) -> LegacyDrainReport {
        self.store
            .legacy_drain_report(tenant, GATE_MAX_ATTEMPTS)
            .await
            .expect("the drain report reads")
    }

    /// The submissions of `tenant` that `main`'s gate driver would pick up:
    /// what `list_submissions_needing_gate_decision` selects, with no backoff
    /// and no limit that could cut the list.
    async fn gate_selected(&self, tenant: &str) -> Vec<Uuid> {
        use trace_commons_server::db::Database as _;

        let mut ids: Vec<Uuid> = self
            .gate
            .list_submissions_needing_gate_decision(
                chrono::Utc::now(),
                GATE_MAX_ATTEMPTS,
                0,
                100_000,
            )
            .await
            .expect("list as the gate driver")
            .into_iter()
            .filter(|item| item.tenant_id == tenant)
            .map(|item| item.submission_id)
            .collect();
        ids.sort();
        ids
    }

    /// How many `trace_submissions` rows of `tenant` have `status`, pipeline
    /// receipts included: the number the report must not simply copy.
    async fn submission_rows_with_status(&self, tenant: &str, status: &str) -> i64 {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let count = tx
            .query_one(
                "SELECT COUNT(*) FROM trace_submissions WHERE tenant_id = $1 AND status = $2",
                &[&tenant, &status],
            )
            .await
            .unwrap()
            .get(0);
        tx.commit().await.unwrap();
        count
    }

    /// The `trace_submissions` status of one submission.
    async fn status_of(&self, tenant: &str, submission_id: Uuid) -> String {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let status = tx
            .query_one(
                "SELECT status FROM trace_submissions
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant, &submission_id],
            )
            .await
            .expect("the submission row")
            .get(0);
        tx.commit().await.unwrap();
        status
    }

    /// The pipeline run states of `submission_ids`, sorted.
    async fn run_states(&self, tenant: &str, submission_ids: &[Uuid]) -> Vec<String> {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let rows = tx
            .query(
                "SELECT state FROM pipeline_runs
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                  ORDER BY state",
                &[&tenant, &submission_ids],
            )
            .await
            .unwrap();
        tx.commit().await.unwrap();
        rows.iter().map(|row| row.get(0)).collect()
    }

    /// A statement on the owner connection in a tenant transaction of `tenant`:
    /// the fixture rows a test seeds, which no legacy route would write.
    async fn owner_execute(&self, tenant: &str, sql: &str, params: &[&(dyn ToSql + Sync)]) -> u64 {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let rows = tx
            .execute(sql, params)
            .await
            .unwrap_or_else(|error| panic!("seed statement failed: {error}: {sql}"));
        tx.commit().await.unwrap();
        rows
    }

    /// A count on the owner connection in a tenant transaction of `tenant`.
    async fn owner_count(&self, tenant: &str, sql: &str, params: &[&(dyn ToSql + Sync)]) -> u64 {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let count: i64 = tx
            .query_one(sql, params)
            .await
            .unwrap_or_else(|error| panic!("count failed: {error}: {sql}"))
            .get(0);
        tx.commit().await.unwrap();
        u64::try_from(count).unwrap()
    }

    async fn set_status(&self, tenant: &str, submission_id: Uuid, status: &str) {
        let updated = self
            .owner_execute(
                tenant,
                "UPDATE trace_submissions SET status = $3
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant, &submission_id, &status],
            )
            .await;
        assert_eq!(updated, 1);
    }

    /// A credit ledger row of `event_type` and `points_delta` (the stored
    /// text) on a submission.
    async fn seed_ledger_event(
        &self,
        tenant: &str,
        submission_id: Uuid,
        trace_id: Uuid,
        event_type: &str,
        points_delta: &str,
    ) -> Uuid {
        let event_id = Uuid::new_v4();
        self.owner_execute(
            tenant,
            "INSERT INTO trace_credit_ledger (
                tenant_id, credit_event_id, submission_id, trace_id, credit_account_ref,
                event_type, points_delta, reason, actor_principal_ref, actor_role,
                settlement_state
             ) VALUES ($1, $2, $3, $4, 'principal_sha256:seeded', $5, $6,
                       'seeded for the drain report test', 'principal_sha256:seeded',
                       'system', 'pending')",
            &[
                &tenant,
                &event_id,
                &submission_id,
                &trace_id,
                &event_type,
                &points_delta,
            ],
        )
        .await;
        event_id
    }

    /// A settlement batch in `status` that names `source_events`, with the
    /// payout `instrument_id` of the versioned pipeline, or none for `main`'s.
    async fn seed_batch(
        &self,
        tenant: &str,
        status: &str,
        source_events: &[Uuid],
        instrument_id: Option<&str>,
    ) -> Uuid {
        let batch_id = Uuid::new_v4();
        let source_list_hash = format!("sha256:{}", batch_id.simple());
        let source_events = source_events.to_vec();
        self.owner_execute(
            tenant,
            "INSERT INTO trace_credit_settlement_batches (
                tenant_id, settlement_batch_id, policy_version, status, reason_hash,
                source_list_hash, settled_credit_points, settled_credit_micros,
                actor_principal_ref, source_credit_event_ids, instrument_id
             ) VALUES ($1, $2, 'trace-credit-policy-v1', $3, 'sha256:seeded', $4, '0', 0,
                       'principal_sha256:seeded', $5, $6)",
            &[
                &tenant,
                &batch_id,
                &status,
                &source_list_hash,
                &source_events,
                &instrument_id,
            ],
        )
        .await;
        batch_id
    }

    /// A NEAR outbox row of a batch in `status`, with the batch's payout
    /// `instrument_id` (the pipeline's payout rows have one).
    async fn seed_outbox(
        &self,
        tenant: &str,
        batch_id: Uuid,
        status: &str,
        instrument_id: Option<&str>,
    ) {
        let account_hash = format!("sha256:{}", Uuid::new_v4().simple());
        self.owner_execute(
            tenant,
            "INSERT INTO trace_near_credit_outbox (
                tenant_id, near_outbox_id, settlement_batch_id, credit_account_hash,
                near_call_json, status, instrument_id
             ) VALUES ($1, $2, $3, $4, '{}'::jsonb, $5, $6)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &batch_id,
                &account_hash,
                &status,
                &instrument_id,
            ],
        )
        .await;
    }

    /// A revocation propagation item of a submission in `status`.
    async fn seed_revocation_item(
        &self,
        tenant: &str,
        submission_id: Uuid,
        trace_id: Uuid,
        status: &str,
    ) {
        let key = format!("drain-seed-{}", Uuid::new_v4().simple());
        self.owner_execute(
            tenant,
            "INSERT INTO trace_revocation_propagation_items (
                tenant_id, propagation_item_id, source_submission_id, trace_id, target_kind,
                target_json, action, status, idempotency_key, reason
             ) VALUES ($1, $2, $3, $4, 'object_ref', '{}'::jsonb, 'invalidate_metadata',
                       $5, $6, 'seeded for the drain report test')",
            &[
                &tenant,
                &Uuid::new_v4(),
                &submission_id,
                &trace_id,
                &status,
                &key,
            ],
        )
        .await;
    }

    /// A current `duplicate_precheck` derived record with a summary hash: the
    /// record the legacy vector index looks for.
    async fn seed_derived_record(&self, tenant: &str, submission_id: Uuid, trace_id: Uuid) {
        let summary_hash = format!("sha256:{}", Uuid::new_v4().simple());
        self.owner_execute(
            tenant,
            "INSERT INTO trace_derived_records (
                tenant_id, derived_id, submission_id, trace_id, status, worker_kind,
                worker_version, input_hash, canonical_summary_hash
             ) VALUES ($1, $2, $3, $4, 'current', 'duplicate_precheck', 'seeded',
                       'sha256:seeded', $5)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &submission_id,
                &trace_id,
                &summary_hash,
            ],
        )
        .await;
    }

    async fn seed_gate_attempts(&self, tenant: &str, submission_id: Uuid, attempts: i32) {
        self.owner_execute(
            tenant,
            "INSERT INTO trace_gate_evaluation_attempts (
                tenant_id, submission_id, attempts, last_attempt_at
             ) VALUES ($1, $2, $3, now())",
            &[&tenant, &submission_id, &attempts],
        )
        .await;
    }

    /// How many active vector entries the tenant has.
    async fn active_vector_entries(&self, tenant: &str) -> u64 {
        self.owner_count(
            tenant,
            "SELECT COUNT(*) FROM trace_vector_entries
              WHERE tenant_id = $1 AND status = 'active'",
            &[&tenant],
        )
        .await
    }

    /// What the legacy vector index worker would index now: a dry run of
    /// `POST /v1/workers/vector-index` (`checked_count`, with a limit above
    /// the backlog).
    async fn vector_worker_would_index(&self) -> u64 {
        self.call_ok(
            "POST",
            "/v1/workers/vector-index",
            &self.worker,
            serde_json::json!({
                "purpose": "drain report agreement",
                "dry_run": true,
                "limit": 500,
            }),
        )
        .await["checked_count"]
            .as_u64()
            .expect("checked_count")
    }

    /// What the legacy settlement worker would settle now: a dry run of
    /// `POST /v1/admin/credit-settlements` (`eligible_source_event_count`).
    async fn settlement_would_settle(&self) -> u64 {
        self.call_ok(
            "POST",
            "/v1/admin/credit-settlements",
            &self.admin,
            serde_json::json!({
                "dry_run": true,
                "policy_version": "trace-credit-policy-v1",
                "reason": "drain report agreement",
            }),
        )
        .await["eligible_source_event_count"]
            .as_u64()
            .expect("eligible_source_event_count")
    }

    /// How many pipeline runs `submission_id` has.
    async fn runs_of(&self, tenant: &str, submission_id: Uuid) -> usize {
        self.run_states(tenant, &[submission_id]).await.len()
    }
}

fn total(report: &LegacyDrainReport) -> u64 {
    report.pending.values().sum()
}

/// `report` counts exactly `expected` and zero for every other label, and
/// `drained` says whether that is all zero.
fn assert_counts(report: &LegacyDrainReport, expected: &[(&str, u64)], context: &str) {
    for label in DRAIN_LABELS {
        let want = expected
            .iter()
            .find(|(name, _)| *name == label)
            .map_or(0, |(_, count)| *count);
        assert_eq!(
            report.pending[label], want,
            "{context}: {label} in {:?}",
            report.pending
        );
    }
    assert_eq!(
        report.drained,
        expected.iter().all(|(_, count)| *count == 0)
    );
}

/// Review Focus 5 (it emits the check `pipeline_legacy_drain`): the report
/// counts the legacy work that is really pending and reaches zero when the
/// legacy code has done it, while the pipeline's own runs are still pending.
///
/// 1. Under the row `legacy`: L1 (the legacy path quarantines it) and L2 (it
///    accepts it and a reviewer awards it a delayed credit): two legacy
///    receipts with no run.
/// 2. The report counts what the legacy workers would select, and the gate
///    count is the number `list_submissions_needing_gate_decision` selects.
/// 3. Under the row `pipeline`: P1 (clean) and P2 (the pipeline's Admission
///    quarantines it) have runs, and their `trace_submissions` rows (P2's is
///    `quarantined`) change no count.
/// 4. L1 retried with its first body: the legacy receipt, still no run.
/// 5. The legacy work, through the legacy routes: the review decision, gate
///    evaluation, the vector index worker, and credit settlement.
/// 6. Every count is zero, `drained` is true, and P1 and P2 still wait.
/// 7. Under the row `legacy` again, the legacy writer still takes a receipt
///    (L3), and the report is above zero again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_legacy_drain_report_counts_real_pending_work_and_reaches_zero() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();

    // (1) Two legacy receipts under the row `legacy`.
    write_routing_as_operator(tenant, "legacy").await;
    let l1 = legacy_quarantine_envelope(&format!("drain rehearsal quarantine {suffix}")).await;
    let l1_body = serde_json::to_vec(&l1).unwrap();
    let l1_receipt = fixture.upload_ok(&fixture.contributor, &l1).await;
    assert_eq!(l1_receipt["status"], "quarantined", "{l1_receipt}");
    let l2 = clean_envelope(&format!("drain_l2_{suffix}")).await;
    let l2_receipt = fixture.upload_ok(&fixture.contributor, &l2).await;
    assert_ne!(l2_receipt["status"], "processing", "L2 is a legacy receipt");
    assert_eq!(l2_receipt["status"], "accepted", "{l2_receipt}");
    for id in [l1.submission_id, l2.submission_id] {
        assert_eq!(fixture.runs_of(tenant, id).await, 0, "a legacy receipt");
    }
    // L2 earns a delayed credit from a reviewer: the legacy path's own
    // follow-up record, settled later by the settlement worker.
    fixture
        .call_ok(
            "POST",
            &format!("/v1/review/{}/credit-events", l2.submission_id),
            &fixture.reviewer,
            serde_json::json!({
                "event_type": "training_utility",
                "credit_points_delta": 1.0,
                "reason": "drain rehearsal training utility",
                "external_ref": format!("training-utility:drain:{suffix}"),
            }),
        )
        .await;

    // (2) The report before the switch.
    let before = fixture.report(tenant).await;
    assert_eq!(before.routing_state, Some(RoutingState::Legacy));
    assert_eq!(
        before
            .pending
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        {
            let mut labels = DRAIN_LABELS.to_vec();
            labels.sort_unstable();
            labels
        },
        "every label is present, whatever its count"
    );
    assert_eq!(before.pending["quarantine_review_pending"], 1);
    let gate_selected = fixture.gate_selected(tenant).await;
    assert!(
        gate_selected.contains(&l2.submission_id),
        "the accepted legacy receipt waits for its gate decision"
    );
    assert_eq!(
        before.pending["gate_decision_pending"],
        gate_selected.len() as u64,
        "the gate count is the gate driver's own selection"
    );
    assert_eq!(before.pending["gate_decision_exhausted"], 0);
    assert_eq!(before.pending["awaiting_pii_backstop"], 0);
    assert_eq!(
        before.pending["vector_index_pending"], 1,
        "L2 is accepted, with a current precheck record and no vector entry"
    );
    assert_eq!(before.pending["delayed_credit_unsettled"], 1);
    assert_eq!(before.pending["revocation_propagation_pending"], 0);
    assert_eq!(before.pending["near_outbox_pending"], 0);
    assert!(!before.drained);
    let pending_before = total(&before);

    // (3) Two pipeline receipts under the row `pipeline`.
    write_routing_as_operator(tenant, "pipeline").await;
    let p1 = clean_envelope(&format!("drain_p1_{suffix}")).await;
    let p1_receipt = fixture.upload_ok(&fixture.contributor, &p1).await;
    assert_eq!(p1_receipt["status"], "processing", "{p1_receipt}");
    let p2 = pipeline_quarantine_envelope(&format!("drain_p2_{suffix}")).await;
    let p2_receipt = fixture.upload_ok(&fixture.contributor, &p2).await;
    assert_eq!(p2_receipt["status"], "processing", "{p2_receipt}");
    for id in [p1.submission_id, p2.submission_id] {
        assert_eq!(fixture.runs_of(tenant, id).await, 1, "a pipeline receipt");
    }
    assert_eq!(
        fixture.status_of(tenant, p2.submission_id).await,
        "quarantined",
        "the pipeline's quarantine is a trace_submissions row too"
    );
    assert_eq!(
        fixture
            .submission_rows_with_status(tenant, "quarantined")
            .await,
        2,
        "L1 and P2 are both quarantined rows"
    );
    let with_pipeline_rows = fixture.report(tenant).await;
    assert_eq!(
        with_pipeline_rows.routing_state,
        Some(RoutingState::Pipeline)
    );
    assert_eq!(
        with_pipeline_rows.pending, before.pending,
        "the pipeline's receipts, P2's quarantine included, change no count"
    );
    assert_eq!(
        fixture.gate_selected(tenant).await,
        gate_selected,
        "the gate driver leaves the pipeline's receipts out too"
    );

    // (4) L1 retried with its first body: the legacy receipt, no run.
    let (status, again) = route_trace(&fixture.state, &fixture.contributor, &l1_body).await;
    assert_eq!(status, StatusCode::OK, "{again}");
    assert_eq!(again, l1_receipt, "the retry returns the first receipt");
    assert_eq!(fixture.runs_of(tenant, l1.submission_id).await, 0);
    assert_eq!(fixture.report(tenant).await.pending, before.pending);

    // (5) The legacy work, through the legacy routes.
    // L1: `main`'s review decision route.
    let decided = fixture
        .call_ok(
            "POST",
            &format!("/v1/review/{}/decision", l1.submission_id),
            &fixture.reviewer,
            serde_json::json!({
                "decision": "approve",
                "reason": "drain rehearsal: redaction reviewed",
                "credit_points_pending": 1.0,
            }),
        )
        .await;
    assert_eq!(decided["status"], "accepted", "{decided}");
    // The gate decisions: every submission the gate driver selects, through
    // `main`'s gate evaluation route.
    let to_gate = fixture.gate_selected(tenant).await;
    assert!(to_gate.contains(&l2.submission_id));
    for submission_id in &to_gate {
        let decision = fixture
            .call_ok(
                "POST",
                "/v1/workers/gate/evaluate",
                &fixture.worker,
                serde_json::json!({ "submission_id": submission_id }),
            )
            .await;
        assert_eq!(decision["perplexity_passed"], true, "{decision}");
    }
    assert!(
        fixture.gate_selected(tenant).await.is_empty(),
        "the gate driver has nothing left to select"
    );
    // The vector index and the delayed credit settlement.
    let indexed = fixture
        .call_ok(
            "POST",
            "/v1/workers/vector-index",
            &fixture.worker,
            serde_json::json!({
                "purpose": "drain rehearsal vector index",
                "dry_run": false,
                "limit": 500,
            }),
        )
        .await;
    assert_eq!(indexed["pending_after_count"], 0, "{indexed}");
    let settled = fixture
        .call_ok(
            "POST",
            "/v1/admin/credit-settlements",
            &fixture.admin,
            serde_json::json!({
                "dry_run": false,
                "policy_version": "trace-credit-policy-v1",
                "reason": "drain rehearsal settlement",
            }),
        )
        .await;
    assert_eq!(settled["settled_source_event_count"], 1, "{settled}");

    // (6) Zero, while the pipeline's runs still wait.
    let after = fixture.report(tenant).await;
    assert_eq!(after.routing_state, Some(RoutingState::Pipeline));
    assert_eq!(
        after.pending.values().copied().collect::<Vec<_>>(),
        vec![0; DRAIN_LABELS.len()],
        "{:?}",
        after.pending
    );
    assert!(after.drained);
    assert_eq!(
        after.evidence_hash,
        evidence_hash(&serde_json::json!({
            "schema": "trace_commons.pipeline_legacy_drain.v1",
            "pending": after.pending,
        }))
        .expect("the evidence hash"),
        "the evidence hash is the canonical hash of the pending map"
    );
    assert_ne!(
        after.evidence_hash, before.evidence_hash,
        "other counts, another hash"
    );
    let pipeline_ids = [p1.submission_id, p2.submission_id];
    let states = fixture.run_states(tenant, &pipeline_ids).await;
    assert_eq!(states.len(), 2);
    assert!(
        states.iter().all(|state| state != "complete"),
        "the pipeline's runs are still pending work: {states:?}"
    );
    for id in [l1.submission_id, l2.submission_id] {
        assert_eq!(fixture.runs_of(tenant, id).await, 0, "still legacy");
    }

    // (7) The legacy writer is still on: the row `legacy`, one more receipt.
    write_routing_as_operator(tenant, "legacy").await;
    let l3 = clean_envelope(&format!("drain_l3_{suffix}")).await;
    let l3_receipt = fixture.upload_ok(&fixture.contributor, &l3).await;
    assert_eq!(l3_receipt["status"], "accepted", "{l3_receipt}");
    assert_eq!(fixture.runs_of(tenant, l3.submission_id).await, 0);
    let again_pending = fixture.report(tenant).await;
    assert_eq!(again_pending.routing_state, Some(RoutingState::Legacy));
    assert!(
        again_pending.pending["gate_decision_pending"] > 0,
        "{:?}",
        again_pending.pending
    );
    assert_eq!(
        again_pending.pending["gate_decision_pending"],
        fixture.gate_selected(tenant).await.len() as u64
    );
    assert!(!again_pending.drained);

    // (8) The check of the plan's Task 12.
    PipelineCheckEmitter::emit_pass_from_env(
        "pipeline_legacy_drain",
        None,
        serde_json::json!({
            "legacy_receipts": 3,
            "pipeline_receipts": 2,
            "pending_before": pending_before,
            "pending_after_drain": total(&after),
            "pipeline_rows_counted": total(&with_pipeline_rows) - total(&before),
        }),
    );
}

/// The report is the tenant's own: tenant B's quarantined legacy submission
/// does not change tenant A's report, and A's receipts do not appear in B's.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_drain_report_is_tenant_scoped() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let suffix = Uuid::new_v4().simple().to_string();
    write_routing_as_operator(&fixture.tenant, "legacy").await;
    let a_quarantined =
        legacy_quarantine_envelope(&format!("drain tenant a quarantine {suffix}")).await;
    assert_eq!(
        fixture
            .upload_ok(&fixture.contributor, &a_quarantined)
            .await["status"],
        "quarantined"
    );
    let a_before = fixture.report(&fixture.tenant).await;
    assert_eq!(a_before.pending["quarantine_review_pending"], 1);
    let b_before = fixture.report(&fixture.other_tenant).await;
    assert_eq!(
        b_before.routing_state, None,
        "tenant B has no routing row, and tenant A's row is not B's"
    );
    assert_eq!(total(&b_before), 0);
    assert!(b_before.drained);

    let b_quarantined =
        legacy_quarantine_envelope(&format!("drain tenant b quarantine {suffix}")).await;
    assert_eq!(
        fixture
            .upload_ok(&fixture.other_contributor, &b_quarantined)
            .await["status"],
        "quarantined"
    );
    let b_after = fixture.report(&fixture.other_tenant).await;
    assert_eq!(b_after.pending["quarantine_review_pending"], 1);
    assert!(!b_after.drained);
    let a_after = fixture.report(&fixture.tenant).await;
    assert_eq!(
        a_after.pending, a_before.pending,
        "tenant B's quarantine is not tenant A's"
    );
    assert_eq!(a_after.evidence_hash, a_before.evidence_hash);
    // A tenant the database has never seen reads zero, not an error.
    let unknown = fixture
        .store
        .legacy_drain_report(&format!("tenant-drain-unknown-{suffix}"), GATE_MAX_ATTEMPTS)
        .await
        .expect("a tenant with no rows");
    assert_eq!(total(&unknown), 0);
    assert_eq!(unknown.routing_state, None);
    assert!(unknown.drained);
}

/// An envelope the legacy path quarantines that also allows model training:
/// its delayed credit would be eligible if its status were not `quarantined`.
async fn legacy_quarantine_model_training_envelope(text: &str) -> TraceContributionEnvelope {
    let mut envelope = legacy_quarantine_envelope(text).await;
    envelope.consent.scopes = vec![ConsentScope::ModelTraining];
    envelope.trace_card.consent_scope = ConsentScope::ModelTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::ModelTraining];
    envelope
}

/// The two counts that `main` computes in memory from records agree with the
/// workers' own selection: the vector index count with what a dry run of
/// `POST /v1/workers/vector-index` would index, and the delayed credit count
/// with what a dry run of the settlement route would settle. One row of each
/// kind that the worker's predicate tells apart: an accepted record, one whose
/// allowed uses the index never takes, a quarantined one, an entry that is
/// invalidated, an event of an ineligible type, one on a quarantined
/// submission, one a finalized batch names, and one a failed batch names. The
/// one place the report counts more than the worker is a credit hold, which
/// the worker skips and the report does not: the credit is still owed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_vector_and_credit_counts_select_the_workers_own_sets() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();
    write_routing_as_operator(tenant, "legacy").await;

    // Three accepted records the index takes, one it never takes (the
    // submission's allowed uses are aggregate analytics only), one quarantined.
    let mut accepted = Vec::new();
    for index in 0..3 {
        let envelope = clean_envelope(&format!("drain_agree_{index}_{suffix}")).await;
        let receipt = fixture.upload_ok(&fixture.contributor, &envelope).await;
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        accepted.push(envelope);
    }
    let aggregate_only = clean_envelope(&format!("drain_agree_aggregate_{suffix}")).await;
    assert_eq!(
        fixture
            .upload_ok(&fixture.contributor, &aggregate_only)
            .await["status"],
        "accepted"
    );
    assert_eq!(
        fixture
            .owner_execute(
                tenant,
                "UPDATE trace_submissions SET allowed_uses = '[\"aggregate_analytics\"]'::jsonb
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant, &aggregate_only.submission_id],
            )
            .await,
        1
    );
    let held =
        legacy_quarantine_model_training_envelope(&format!("drain agree held {suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &held).await["status"],
        "quarantined"
    );

    // The vector index: three records, by the report and by the worker.
    assert_eq!(fixture.vector_worker_would_index().await, 3);
    assert_eq!(
        fixture.report(tenant).await.pending["vector_index_pending"],
        3
    );
    // One indexed through the route: two left, by both.
    let indexed = fixture
        .call_ok(
            "POST",
            "/v1/workers/vector-index",
            &fixture.worker,
            serde_json::json!({
                "purpose": "drain report agreement",
                "dry_run": false,
                "limit": 1,
            }),
        )
        .await;
    assert_eq!(indexed["vector_entries_indexed"], 1, "{indexed}");
    assert_eq!(indexed["pending_after_count"], 2, "{indexed}");
    assert_eq!(fixture.active_vector_entries(tenant).await, 1);
    assert_eq!(fixture.vector_worker_would_index().await, 2);
    assert_eq!(
        fixture.report(tenant).await.pending["vector_index_pending"],
        2
    );
    // An entry that is no longer active is pending again, by both: whether its
    // status changed, or its status is still `active` and a timestamp says it
    // is invalidated or deleted. Only dry runs: indexing the entry again for
    // real fails in this test's file-backed artifact store, which refuses a
    // second write of the entry's payload key with new ciphertext
    // (`FileRemoteTraceArtifactProvider::write_record`). Each is put back.
    for (what, set, restore) in [
        ("status", "status = 'invalidated'", "status = 'active'"),
        (
            "invalidated_at",
            "invalidated_at = now()",
            "invalidated_at = NULL",
        ),
        ("deleted_at", "deleted_at = now()", "deleted_at = NULL"),
    ] {
        let flip = |assignment: &'static str| {
            let sql = format!(
                "UPDATE trace_vector_entries SET {assignment}
                  WHERE tenant_id = $1 AND source_projection = 'canonical_summary'"
            );
            let fixture = &fixture;
            async move { fixture.owner_execute(tenant, &sql, &[&tenant]).await }
        };
        assert_eq!(flip(set).await, 1, "{what}");
        assert_eq!(fixture.vector_worker_would_index().await, 3, "{what}");
        assert_eq!(
            fixture.report(tenant).await.pending["vector_index_pending"],
            3,
            "{what}"
        );
        assert_eq!(flip(restore).await, 1, "{what}");
        assert_eq!(fixture.vector_worker_would_index().await, 2, "{what}");
        assert_eq!(
            fixture.report(tenant).await.pending["vector_index_pending"],
            2,
            "{what}"
        );
    }
    // The rest indexed: zero, by both.
    let indexed = fixture
        .call_ok(
            "POST",
            "/v1/workers/vector-index",
            &fixture.worker,
            serde_json::json!({
                "purpose": "drain report agreement",
                "dry_run": false,
                "limit": 500,
            }),
        )
        .await;
    assert_eq!(indexed["vector_entries_indexed"], 2, "{indexed}");
    assert_eq!(fixture.vector_worker_would_index().await, 0);
    assert_eq!(
        fixture.report(tenant).await.pending["vector_index_pending"],
        0
    );

    // Delayed credit: two eligible events, one of a type that never settles,
    // and one on the quarantined submission.
    for (index, (envelope, points)) in [(&accepted[0], 1.0), (&accepted[1], 0.5)]
        .into_iter()
        .enumerate()
    {
        fixture
            .call_ok(
                "POST",
                &format!("/v1/review/{}/credit-events", envelope.submission_id),
                &fixture.reviewer,
                serde_json::json!({
                    "event_type": "training_utility",
                    "credit_points_delta": points,
                    "reason": "drain report agreement",
                    "external_ref": format!("training-utility:agree:{index}:{suffix}"),
                }),
            )
            .await;
    }
    fixture
        .call_ok(
            "POST",
            &format!("/v1/review/{}/credit-events", accepted[2].submission_id),
            &fixture.reviewer,
            serde_json::json!({
                "event_type": "reviewer_bonus",
                "credit_points_delta": 0.25,
                "reason": "drain report agreement",
            }),
        )
        .await;
    fixture
        .call_ok(
            "POST",
            &format!("/v1/review/{}/credit-events", held.submission_id),
            &fixture.reviewer,
            serde_json::json!({
                "event_type": "training_utility",
                "credit_points_delta": 1.0,
                "reason": "drain report agreement",
                "external_ref": format!("training-utility:agree:held:{suffix}"),
            }),
        )
        .await;
    assert_eq!(fixture.settlement_would_settle().await, 2);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        2
    );

    // A hold on the contributor's credit account: the worker skips the
    // events, the report still owes them.
    let account: String = {
        let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let account = tx
            .query_one(
                "SELECT credit_account_ref FROM trace_credit_ledger
                  WHERE tenant_id = $1 AND event_type = 'training_utility'
                    AND submission_id = $2",
                &[&tenant, &accepted[0].submission_id],
            )
            .await
            .unwrap()
            .get(0);
        tx.commit().await.unwrap();
        account
    };
    let hold = fixture
        .call_ok(
            "POST",
            "/v1/admin/credit-holds",
            &fixture.admin,
            serde_json::json!({
                "credit_account_ref": account,
                "reason": "suspected_abuse",
                "reason_detail": "drain report agreement",
            }),
        )
        .await;
    assert_eq!(fixture.settlement_would_settle().await, 0);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        2,
        "held credit is still owed"
    );
    fixture
        .call_ok(
            "POST",
            &format!(
                "/v1/admin/credit-holds/{}/release",
                hold["hold_id"].as_str().unwrap()
            ),
            &fixture.admin,
            serde_json::json!({ "reason_detail": "drain report agreement" }),
        )
        .await;
    assert_eq!(fixture.settlement_would_settle().await, 2);

    // One settled through the route: one left, by both.
    let settled = fixture
        .call_ok(
            "POST",
            "/v1/admin/credit-settlements",
            &fixture.admin,
            serde_json::json!({
                "dry_run": false,
                "policy_version": "trace-credit-policy-v1",
                "reason": "drain report agreement",
                "source_event_limit": 1,
            }),
        )
        .await;
    assert_eq!(settled["settled_source_event_count"], 1, "{settled}");
    assert_eq!(fixture.settlement_would_settle().await, 1);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        1
    );
    // A failed batch names the event that is left: a batch that did not
    // finalize settles nothing, for the worker and for the report.
    let left: Uuid = {
        let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let left = tx
            .query_one(
                "SELECT credit_event_id FROM trace_credit_ledger l
                  WHERE tenant_id = $1 AND event_type = 'training_utility'
                    AND submission_id = ANY($2)
                    AND NOT EXISTS (
                        SELECT 1 FROM trace_credit_settlement_batches b
                         WHERE b.tenant_id = l.tenant_id AND b.status = 'finalized'
                           AND l.credit_event_id = ANY (b.source_credit_event_ids)
                    )",
                &[
                    &tenant,
                    &vec![accepted[0].submission_id, accepted[1].submission_id],
                ],
            )
            .await
            .unwrap()
            .get(0);
        tx.commit().await.unwrap();
        left
    };
    fixture.seed_batch(tenant, "failed", &[left], None).await;
    assert_eq!(fixture.settlement_would_settle().await, 1);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        1
    );
    // The rest settled: zero, by both. The event of the quarantined
    // submission is still unsettled and is owed nothing yet.
    let settled = fixture
        .call_ok(
            "POST",
            "/v1/admin/credit-settlements",
            &fixture.admin,
            serde_json::json!({
                "dry_run": false,
                "policy_version": "trace-credit-policy-v1",
                "reason": "drain report agreement",
            }),
        )
        .await;
    assert_eq!(settled["settled_source_event_count"], 1, "{settled}");
    assert_eq!(fixture.settlement_would_settle().await, 0);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        0
    );
    // The quarantined submission's event becomes owed when its review
    // accepts it: the report follows the status, as the worker does.
    fixture
        .call_ok(
            "POST",
            &format!("/v1/review/{}/decision", held.submission_id),
            &fixture.reviewer,
            serde_json::json!({
                "decision": "approve",
                "reason": "drain report agreement",
                "credit_points_pending": 1.0,
            }),
        )
        .await;
    assert_eq!(fixture.settlement_would_settle().await, 1);
    assert_eq!(
        fixture.report(tenant).await.pending["delayed_credit_unsettled"],
        1
    );
}

/// Every count excludes a submission that a pipeline run owns, and counts the
/// legacy twin of it: P (a pipeline receipt) and L (a legacy receipt) are given
/// the same rows, each in the state that one count looks for, and the report
/// counts L alone. The raw rows are counted too, so the test cannot pass
/// because P never satisfied the predicate. The NEAR outbox is tenant-wide:
/// it counts `main`'s rows in the three open states and leaves out a
/// confirmed one, a disabled one, and the pipeline's payout row.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_submission_with_a_pipeline_run_is_never_counted_whatever_its_rows_say() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();

    write_routing_as_operator(tenant, "legacy").await;
    let legacy = clean_envelope(&format!("drain_twin_l_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &legacy).await["status"],
        "accepted"
    );
    write_routing_as_operator(tenant, "pipeline").await;
    let pipeline = clean_envelope(&format!("drain_twin_p_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &pipeline).await["status"],
        "processing"
    );
    assert_eq!(fixture.runs_of(tenant, legacy.submission_id).await, 0);
    assert_eq!(fixture.runs_of(tenant, pipeline.submission_id).await, 1);
    let (l, p) = (legacy.submission_id, pipeline.submission_id);
    let twins = vec![l, p];

    // (1) Accepted twins with the rows of the gate, vector, credit, and
    // revocation counts. A review approval makes a pipeline receipt
    // `accepted`; its derived record is seeded as the legacy path's mirror
    // writes L's. Each has an active submitted envelope ref and no decision.
    fixture.set_status(tenant, p, "accepted").await;
    assert_eq!(
        fixture
            .owner_execute(
                tenant,
                "UPDATE trace_submissions SET allowed_uses = '[\"model_training\"]'::jsonb
                  WHERE tenant_id = $1 AND submission_id = $2",
                &[&tenant, &p],
            )
            .await,
        1
    );
    fixture
        .seed_derived_record(tenant, p, pipeline.trace_id)
        .await;
    for (id, trace_id) in [(l, legacy.trace_id), (p, pipeline.trace_id)] {
        // Eligible: four types, positive. Not eligible: another type, a
        // negative delta, a zero delta.
        for event_type in [
            "training_utility",
            "ranking_utility",
            "regression_catch",
            "benchmark_conversion",
        ] {
            fixture
                .seed_ledger_event(tenant, id, trace_id, event_type, "0.5000")
                .await;
        }
        fixture
            .seed_ledger_event(tenant, id, trace_id, "novelty_utility", "2.5000")
            .await;
        fixture
            .seed_ledger_event(tenant, id, trace_id, "training_utility", "-1.0000")
            .await;
        fixture
            .seed_ledger_event(tenant, id, trace_id, "training_utility", "0.0000")
            .await;
        // The one a finalized batch names: not owed.
        let settled = fixture
            .seed_ledger_event(tenant, id, trace_id, "training_utility", "0.7500")
            .await;
        fixture
            .seed_batch(tenant, "finalized", &[settled], None)
            .await;
        // Three open states of revocation work and two closed ones.
        for status in ["pending", "in_progress", "failed", "done", "skipped"] {
            fixture
                .seed_revocation_item(tenant, id, trace_id, status)
                .await;
        }
    }
    let accepted_phase = fixture.report(tenant).await;
    assert_counts(
        &accepted_phase,
        &[
            ("gate_decision_pending", 1),
            ("vector_index_pending", 1),
            ("delayed_credit_unsettled", 4),
            ("revocation_propagation_pending", 3),
        ],
        "accepted twins",
    );
    assert_eq!(
        fixture.gate_selected(tenant).await,
        vec![l],
        "the gate driver selects L alone"
    );
    // The raw rows P would have been counted by.
    assert_eq!(
        fixture
            .owner_count(
                tenant,
                "SELECT COUNT(*) FROM trace_submissions s
                  WHERE s.tenant_id = $1 AND s.submission_id = ANY($2)
                    AND EXISTS (SELECT 1 FROM trace_object_refs o
                                 WHERE o.tenant_id = s.tenant_id
                                   AND o.submission_id = s.submission_id
                                   AND o.artifact_kind = 'submitted_envelope'
                                   AND o.invalidated_at IS NULL AND o.deleted_at IS NULL)
                    AND NOT EXISTS (SELECT 1 FROM trace_gate_decisions d
                                     WHERE d.tenant_id = s.tenant_id
                                       AND d.submission_id = s.submission_id)",
                &[&tenant, &twins],
            )
            .await,
        2,
        "both twins wait for a gate decision but for the run"
    );
    assert_eq!(
        fixture
            .owner_count(
                tenant,
                "SELECT COUNT(*) FROM trace_derived_records d
                   JOIN trace_submissions s
                     ON s.tenant_id = d.tenant_id AND s.submission_id = d.submission_id
                  WHERE d.tenant_id = $1 AND d.submission_id = ANY($2)
                    AND d.status = 'current' AND d.worker_kind = 'duplicate_precheck'
                    AND s.status = 'accepted'",
                &[&tenant, &twins],
            )
            .await,
        2
    );
    assert_eq!(
        fixture
            .owner_count(
                tenant,
                "SELECT COUNT(*) FROM trace_credit_ledger
                  WHERE tenant_id = $1 AND submission_id = ANY($2)
                    AND event_type IN ('training_utility', 'ranking_utility',
                                       'regression_catch', 'benchmark_conversion')
                    AND points_delta::numeric > 0",
                &[&tenant, &twins],
            )
            .await,
        10,
        "five eligible events for each twin, one of them settled"
    );
    assert_eq!(
        fixture
            .owner_count(
                tenant,
                "SELECT COUNT(*) FROM trace_revocation_propagation_items
                  WHERE tenant_id = $1 AND source_submission_id = ANY($2)
                    AND status IN ('pending', 'in_progress', 'failed')",
                &[&tenant, &twins],
            )
            .await,
        6
    );

    // (2) Both twins at the gate ceiling: exhausted, not pending, and out of
    // the gate driver's list.
    for id in [l, p] {
        fixture
            .seed_gate_attempts(tenant, id, GATE_MAX_ATTEMPTS)
            .await;
    }
    let exhausted_phase = fixture.report(tenant).await;
    assert_counts(
        &exhausted_phase,
        &[
            ("gate_decision_exhausted", 1),
            ("vector_index_pending", 1),
            ("delayed_credit_unsettled", 4),
            ("revocation_propagation_pending", 3),
        ],
        "twins at the gate ceiling",
    );
    assert!(fixture.gate_selected(tenant).await.is_empty());

    // (3) Both twins quarantined, then both awaiting the PII backstop.
    for (status, label) in [
        ("quarantined", "quarantine_review_pending"),
        ("awaiting_pii_backstop", "awaiting_pii_backstop"),
    ] {
        fixture.set_status(tenant, l, status).await;
        fixture.set_status(tenant, p, status).await;
        assert_eq!(fixture.submission_rows_with_status(tenant, status).await, 2);
        assert_counts(
            &fixture.report(tenant).await,
            &[
                (label, 1),
                ("gate_decision_exhausted", 1),
                ("revocation_propagation_pending", 3),
            ],
            status,
        );
    }

    // (4) The NEAR outbox, tenant-wide: `main`'s rows in the three open
    // states count, whatever submission their batch settled; a confirmed row,
    // a disabled row, and the pipeline's payout row (an instrument id, on a
    // batch of the same instrument) do not.
    let batch = fixture.seed_batch(tenant, "finalized", &[], None).await;
    for status in ["pending", "failed", "submitted", "confirmed", "disabled"] {
        fixture.seed_outbox(tenant, batch, status, None).await;
    }
    let payout_batch = fixture
        .seed_batch(tenant, "finalized", &[], Some("trace_credit"))
        .await;
    fixture
        .seed_outbox(tenant, payout_batch, "pending", Some("trace_credit"))
        .await;
    assert_counts(
        &fixture.report(tenant).await,
        &[
            ("awaiting_pii_backstop", 1),
            ("gate_decision_exhausted", 1),
            ("revocation_propagation_pending", 3),
            ("near_outbox_pending", 3),
        ],
        "the NEAR outbox",
    );
}
