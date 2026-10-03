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
//! The second half of the file (Task 10) drives the admin routes an operator
//! uses (`pipeline_activation`): qualification, activation, rollback,
//! containment, deactivation, the routing view, policy interventions, and the
//! drain report, with a production-qualified runtime and both trust stores.
//!
//! A report is read in one of two modes of the gate driver: `report` for a
//! deployment that runs it (`Some(ceiling)`, today's behaviour) and
//! `report_driver_off` for one that does not (`None`: a missing gate decision
//! is shown as `not_blocking["gate_decision_absent"]` and blocks nothing).
//!
//! The app is served as a plain router, not through `run_pipeline_app`, so no
//! pipeline worker runs: the pipeline's receipts stay pending, which is the
//! point of the rehearsal (the report counts only what no pipeline run owns).
//!
//! Nested inside `tests` beside `pipeline_http_pg_tests`, whose `pub(super)`
//! helpers it reuses: `sample_envelope`, `auth_headers`, and the other state
//! builders are private to `tests` and visible only to its descendants.

use super::*;

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use super::pipeline_http_pg_tests::{
    PassThroughPipelinePrivacyBoundary, TEST_PIPELINE_CREDIT_ISSUER, account_owner_backend,
    assemble_compatibility_pipeline_service_with, mains_database, pipeline_http_database_url,
    route_request, route_trace, runtime_backend, tenant_tx, write_routing_as_operator,
};
use tokio_postgres::types::ToSql;
use trace_commons_gate_api::SettlementAdapter;
use trace_commons_gate_api::pipeline::InstrumentId;
use trace_commons_server::versioned_pipeline_activation::{
    LegacyDrainReport, PipelineActivationStore, RoutingState,
};
use trace_commons_server::versioned_pipeline_bundle::MinimalPolicyBundle;
use trace_commons_server::versioned_pipeline_compat::CompatibilityBundleConfig;
use trace_commons_server::versioned_pipeline_credit::RecordingSettlementAdapter;
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_qualification::{PipelineCheckEmitter, evidence_hash};

/// The gate driver's attempt ceiling in these tests: the value `main`'s own
/// gate tests pass to `list_submissions_needing_gate_decision`. A report is
/// read with `Some(GATE_MAX_ATTEMPTS)` for a deployment that runs the gate
/// driver, and with `None` for one that does not.
const GATE_MAX_ATTEMPTS: i32 = 5;

/// The one label that a report holds in `not_blocking`, in both modes.
const ABSENT_LABEL: &str = "gate_decision_absent";

/// The ten labels of the report, in the order of the plan's table and the two
/// counts that the task review added.
const DRAIN_LABELS: [&str; 10] = [
    "awaiting_pii_backstop",
    "gate_decision_pending",
    "gate_decision_exhausted",
    "quarantine_review_pending",
    "vector_index_pending",
    "delayed_credit_unsettled",
    "revocation_propagation_pending",
    "near_outbox_pending",
    "near_payout_unqueued",
    "withdrawal_completion_pending",
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

    /// The report of `tenant` for a deployment that runs the gate driver, with
    /// the gate ceiling of these tests and no retention policy held by a legal
    /// hold.
    async fn report(&self, tenant: &str) -> LegacyDrainReport {
        self.report_with_held(tenant, &[]).await
    }

    /// The report of `tenant` for a deployment whose gate driver is off, with
    /// no retention policy held by a legal hold.
    async fn report_driver_off(&self, tenant: &str) -> LegacyDrainReport {
        self.report_for(tenant, None, &[]).await
    }

    /// The report of `tenant` with the gate driver on and
    /// `held_retention_policy_ids` held.
    async fn report_with_held(
        &self,
        tenant: &str,
        held_retention_policy_ids: &[String],
    ) -> LegacyDrainReport {
        self.report_for(tenant, Some(GATE_MAX_ATTEMPTS), held_retention_policy_ids)
            .await
    }

    /// The report of `tenant` for the gate driver state `gate_driver_max_attempts`
    /// (`Some(ceiling)`: the driver runs; `None`: it is off).
    async fn report_for(
        &self,
        tenant: &str,
        gate_driver_max_attempts: Option<i32>,
        held_retention_policy_ids: &[String],
    ) -> LegacyDrainReport {
        self.store
            .legacy_drain_report(tenant, gate_driver_max_attempts, held_retention_policy_ids)
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

    /// A text column of one row on the owner connection.
    async fn owner_text(&self, tenant: &str, sql: &str, params: &[&(dyn ToSql + Sync)]) -> String {
        let mut client = self.owner.trace_pool_for_test().get().await.unwrap();
        let tx = tenant_tx(&mut client, tenant).await;
        let text: String = tx
            .query_one(sql, params)
            .await
            .unwrap_or_else(|error| panic!("query failed: {error}: {sql}"))
            .get(0);
        tx.commit().await.unwrap();
        text
    }

    /// The durable account of the principal of `token` in `tenant`, minted
    /// through `main`'s own call.
    async fn account_of(&self, tenant: &str, token: &str) -> Uuid {
        self.owner
            .create_or_reuse_account(tenant, &static_token_principal_ref(token))
            .await
            .expect("mint the contributor's account")
    }

    /// A delayed credit of `points` for `submission_id` from the reviewer, a
    /// settlement-eligible `training_utility` event.
    async fn award_training_utility(&self, submission_id: Uuid, points: f32, tag: &str) {
        self.call_ok(
            "POST",
            &format!("/v1/review/{submission_id}/credit-events"),
            &self.reviewer,
            serde_json::json!({
                "event_type": "training_utility",
                "credit_points_delta": points,
                "reason": "drain report payout test",
                "external_ref": format!("training-utility:{tag}"),
            }),
        )
        .await;
    }

    /// A live settlement run through the admin route, with `main`'s NEAR
    /// contract named in the request: a finalized batch with a contract, and
    /// before it the repair of the batches that are already finalized.
    async fn settle_with_near_contract(&self) -> serde_json::Value {
        self.call_ok(
            "POST",
            "/v1/admin/credit-settlements",
            &self.admin,
            serde_json::json!({
                "dry_run": false,
                "policy_version": "trace-credit-policy-v1",
                "reason": "drain report payout test",
                "near_contract_id": "trace-credits.testnet",
            }),
        )
        .await
    }

    /// A settlement batch in `status` whose `line_items_json` is `line_items`,
    /// with a NEAR contract and a payout instrument or neither.
    async fn seed_batch_with_items(
        &self,
        tenant: &str,
        status: &str,
        near_contract_id: Option<&str>,
        instrument_id: Option<&str>,
        line_items: serde_json::Value,
    ) -> Uuid {
        let batch_id = Uuid::new_v4();
        let source_list_hash = format!("sha256:{}", batch_id.simple());
        self.owner_execute(
            tenant,
            "INSERT INTO trace_credit_settlement_batches (
                tenant_id, settlement_batch_id, policy_version, status, reason_hash,
                source_list_hash, settled_credit_points, settled_credit_micros,
                actor_principal_ref, line_items_json, near_contract_id, instrument_id
             ) VALUES ($1, $2, 'trace-credit-policy-v1', $3, 'sha256:seeded', $4, '0', 0,
                       'principal_sha256:seeded', $5, $6, $7)",
            &[
                &tenant,
                &batch_id,
                &status,
                &source_list_hash,
                &line_items,
                &near_contract_id,
                &instrument_id,
            ],
        )
        .await;
        batch_id
    }

    /// What the withdrawal worker would complete now under `state`: a dry run
    /// of `POST /v1/workers/revocation-propagation`
    /// (`withdrawal_completions_checked`).
    async fn withdrawals_the_worker_would_complete(&self, state: &Arc<AppState>) -> u64 {
        let (status, body) = route_request(
            state.clone(),
            "POST",
            "/v1/workers/revocation-propagation",
            auth_headers(&self.admin),
            Some(serde_json::json!({
                "purpose": "drain report withdrawal agreement",
                "dry_run": true,
                "limit": 100,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["withdrawal_completions_checked"]
            .as_u64()
            .expect("withdrawal_completions_checked")
    }

    /// How many pipeline runs `submission_id` has.
    async fn runs_of(&self, tenant: &str, submission_id: Uuid) -> usize {
        self.run_states(tenant, &[submission_id]).await.len()
    }

    /// How many of `submission_ids` have an active submitted envelope and no
    /// gate decision, whatever their status, attempts, or pipeline runs: the
    /// raw rows that the absent gate count selects from, so that a test can
    /// tell a submission that never met the predicate from one that the count
    /// left out.
    async fn rows_without_a_gate_decision(&self, tenant: &str, submission_ids: &[Uuid]) -> u64 {
        self.owner_count(
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
            &[&tenant, &submission_ids],
        )
        .await
    }
}

fn total(report: &LegacyDrainReport) -> u64 {
    report.pending.values().sum()
}

/// `report`, read with the gate driver on, counts exactly `expected` and zero
/// for every other label, `drained` says whether that is all zero, and the
/// absent gate count is zero: with the driver on a missing gate decision is
/// counted in `pending`, never in `not_blocking`.
fn assert_counts(report: &LegacyDrainReport, expected: &[(&str, u64)], context: &str) {
    assert!(
        report.gate_driver_enabled,
        "{context}: a report read with the gate driver on says so"
    );
    assert_pending_and_not_blocking(report, expected, 0, context);
}

/// `report`, read with the gate driver off, counts exactly `expected` and zero
/// for every other label (the two gate labels are zero whatever is waiting for
/// a decision), `drained` says whether that is all zero, and
/// `not_blocking["gate_decision_absent"]` is `absent`.
fn assert_counts_driver_off(
    report: &LegacyDrainReport,
    expected: &[(&str, u64)],
    absent: u64,
    context: &str,
) {
    assert!(
        !report.gate_driver_enabled,
        "{context}: a report read with the gate driver off says so"
    );
    assert!(
        expected.iter().all(|(label, _)| !matches!(
            *label,
            "gate_decision_pending" | "gate_decision_exhausted"
        )),
        "{context}: with the driver off the two gate labels are zero, not expected counts"
    );
    assert_pending_and_not_blocking(report, expected, absent, context);
}

fn assert_pending_and_not_blocking(
    report: &LegacyDrainReport,
    expected: &[(&str, u64)],
    absent: u64,
    context: &str,
) {
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
        report.pending.len(),
        DRAIN_LABELS.len(),
        "{context}: pending holds the ten labels and no other: {:?}",
        report.pending
    );
    assert_eq!(
        report.not_blocking,
        BTreeMap::from([(ABSENT_LABEL.to_string(), absent)]),
        "{context}: not_blocking holds the one label, a zero too"
    );
    assert_eq!(
        report.drained,
        expected.iter().all(|(_, count)| *count == 0),
        "{context}: drained reads pending alone"
    );
    assert_eq!(
        report.evidence_hash,
        expected_evidence_hash(report),
        "{context}: the evidence hash is the canonical hash of the mode, pending, and not_blocking"
    );
}

/// The canonical hash that `report.evidence_hash` must be.
fn expected_evidence_hash(report: &LegacyDrainReport) -> String {
    evidence_hash(&serde_json::json!({
        "schema": "trace_commons.pipeline_legacy_drain.v1",
        "gate_driver_enabled": report.gate_driver_enabled,
        "pending": report.pending,
        "not_blocking": report.not_blocking,
    }))
    .expect("the evidence hash")
}

/// `off` and `on` are reports of the same rows in the two modes: the eight
/// labels that are not about the gate agree, the gate driver's two labels are
/// zero with the driver off, the number the driver-off report shows as absent
/// is exactly what the driver-on report counts in its two gate labels, and the
/// two hashes differ.
fn assert_modes_agree(off: &LegacyDrainReport, on: &LegacyDrainReport, context: &str) {
    assert!(
        on.gate_driver_enabled && !off.gate_driver_enabled,
        "{context}"
    );
    assert_eq!(off.routing_state, on.routing_state, "{context}");
    for label in DRAIN_LABELS {
        if matches!(label, "gate_decision_pending" | "gate_decision_exhausted") {
            assert_eq!(
                off.pending[label], 0,
                "{context}: {label} with the driver off"
            );
        } else {
            assert_eq!(
                off.pending[label], on.pending[label],
                "{context}: {label} does not depend on the gate driver"
            );
        }
    }
    assert_eq!(
        off.not_blocking[ABSENT_LABEL],
        on.pending["gate_decision_pending"] + on.pending["gate_decision_exhausted"],
        "{context}: absent is what the two gate labels count under any ceiling"
    );
    assert_eq!(on.not_blocking[ABSENT_LABEL], 0, "{context}");
    assert_eq!(
        off.drained,
        off.pending.values().all(|count| *count == 0),
        "{context}"
    );
    assert_ne!(
        off.evidence_hash, on.evidence_hash,
        "{context}: the two modes of the same rows hash apart"
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
    assert_eq!(before.pending["near_payout_unqueued"], 0);
    assert_eq!(before.pending["withdrawal_completion_pending"], 0);
    assert!(!before.drained);
    assert!(before.gate_driver_enabled);
    assert_eq!(
        before.not_blocking,
        BTreeMap::from([(ABSENT_LABEL.to_string(), 0)]),
        "with the gate driver on, nothing is shown as absent"
    );
    // The same rows for a deployment whose gate driver is off: what the
    // driver-on report counts as waiting for a gate decision is shown as
    // absent and blocks nothing; the rest of the report is the same.
    let before_off = fixture.report_driver_off(tenant).await;
    assert_modes_agree(&before_off, &before, "before the switch");
    assert_eq!(
        before_off.not_blocking[ABSENT_LABEL],
        gate_selected.len() as u64,
        "absent is the gate driver's own selection (the ceiling is never reached here)"
    );
    assert!(
        !before_off.drained,
        "the quarantine, vector, and credit work still block"
    );
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
        with_pipeline_rows.not_blocking, before.not_blocking,
        "and show nothing as absent"
    );
    let with_pipeline_rows_off = fixture.report_driver_off(tenant).await;
    assert_eq!(
        with_pipeline_rows_off.pending, before_off.pending,
        "driver off: the pipeline's receipts change no count"
    );
    assert_eq!(
        with_pipeline_rows_off.not_blocking, before_off.not_blocking,
        "driver off: P1 and P2 have an envelope and no gate decision, and are not absent here"
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
    assert!(after.gate_driver_enabled);
    assert_eq!(
        after.not_blocking,
        BTreeMap::from([(ABSENT_LABEL.to_string(), 0)])
    );
    assert_eq!(
        after.evidence_hash,
        evidence_hash(&serde_json::json!({
            "schema": "trace_commons.pipeline_legacy_drain.v1",
            "gate_driver_enabled": true,
            "pending": after.pending,
            "not_blocking": after.not_blocking,
        }))
        .expect("the evidence hash"),
        "the evidence hash is the canonical hash of the mode, the pending map, and the not_blocking map"
    );
    assert_ne!(
        after.evidence_hash, before.evidence_hash,
        "other counts, another hash"
    );
    // With the gate driver off the same rows are drained too, and the real
    // gate runs left nothing to show as absent.
    let after_off = fixture.report_driver_off(tenant).await;
    assert_counts_driver_off(&after_off, &[], 0, "after the legacy work, driver off");
    assert!(after_off.drained);
    assert_modes_agree(&after_off, &after, "after the legacy work");
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
    // With the gate driver off, L3's missing gate decision is shown as absent
    // and blocks nothing; its vector index work still blocks.
    let again_off = fixture.report_driver_off(tenant).await;
    assert_modes_agree(&again_off, &again_pending, "L3 under the row legacy");
    assert!(again_off.not_blocking[ABSENT_LABEL] > 0);
    assert_eq!(again_off.pending["vector_index_pending"], 1);
    assert!(!again_off.drained);

    // (8) The check of the plan's Task 12. It names no package: a mechanics
    // check. Its facts are counts and booleans.
    PipelineCheckEmitter::emit_pass_from_env(
        "pipeline_legacy_drain",
        None,
        serde_json::json!({
            "legacy_receipts": 3,
            "pipeline_receipts": 2,
            "pending_before": pending_before,
            "pending_after_drain": total(&after),
            "pipeline_rows_counted": total(&with_pipeline_rows) - total(&before),
            "gate_decisions_absent_before": before_off.not_blocking[ABSENT_LABEL],
            "gate_decisions_absent_after_drain": after_off.not_blocking[ABSENT_LABEL],
            "drained_with_gate_driver_off": after_off.drained,
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
    let a_off_before = fixture.report_driver_off(&fixture.tenant).await;
    assert_modes_agree(&a_off_before, &a_before, "tenant A");
    let b_before = fixture.report(&fixture.other_tenant).await;
    assert_eq!(
        b_before.routing_state, None,
        "tenant B has no routing row, and tenant A's row is not B's"
    );
    assert_eq!(total(&b_before), 0);
    assert!(b_before.drained);
    assert_counts(&b_before, &[], "tenant B before its upload");
    let b_off_before = fixture.report_driver_off(&fixture.other_tenant).await;
    assert_counts_driver_off(&b_off_before, &[], 0, "tenant B before its upload");
    assert!(b_off_before.drained);

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
    let b_off_after = fixture.report_driver_off(&fixture.other_tenant).await;
    assert_modes_agree(&b_off_after, &b_after, "tenant B");
    assert!(
        !b_off_after.drained,
        "the quarantine blocks with the driver off too"
    );
    let a_after = fixture.report(&fixture.tenant).await;
    assert_eq!(
        a_after.pending, a_before.pending,
        "tenant B's quarantine is not tenant A's"
    );
    assert_eq!(
        a_after.not_blocking, a_before.not_blocking,
        "nor is it in tenant A's not_blocking"
    );
    assert_eq!(a_after.evidence_hash, a_before.evidence_hash);
    let a_off_after = fixture.report_driver_off(&fixture.tenant).await;
    assert_eq!(a_off_after.pending, a_off_before.pending);
    assert_eq!(
        a_off_after.not_blocking, a_off_before.not_blocking,
        "tenant B's missing gate decision is not tenant A's absent count"
    );
    assert_eq!(a_off_after.evidence_hash, a_off_before.evidence_hash);
    // A tenant the database has never seen reads zero, not an error, in both
    // modes.
    let unknown_tenant = format!("tenant-drain-unknown-{suffix}");
    for gate_driver_max_attempts in [Some(GATE_MAX_ATTEMPTS), None] {
        let unknown = fixture
            .store
            .legacy_drain_report(&unknown_tenant, gate_driver_max_attempts, &[])
            .await
            .expect("a tenant with no rows");
        assert_eq!(total(&unknown), 0);
        assert_eq!(unknown.not_blocking[ABSENT_LABEL], 0);
        assert_eq!(unknown.routing_state, None);
        assert!(unknown.drained);
        assert_eq!(
            unknown.gate_driver_enabled,
            gate_driver_max_attempts.is_some()
        );
    }
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

/// A missing gate decision is owed work only in a deployment that runs the gate
/// driver (owner decision, 2026-10-02). With the driver on (`Some(ceiling)`) the
/// report counts it as `gate_decision_pending` below the ceiling and
/// `gate_decision_exhausted` at it, and both block `drained`. With the driver
/// off (`None`) no code makes the decision, so both labels are zero, the same
/// number is shown as `not_blocking["gate_decision_absent"]` whatever the
/// attempts, and it does not change `drained`. Real legacy receipts: W waits
/// for a decision, X has its attempts at the ceiling.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_missing_gate_decision_blocks_the_drain_only_when_the_gate_driver_runs() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();
    write_routing_as_operator(tenant, "legacy").await;

    // (1) Two accepted legacy receipts, neither with a gate decision. The
    // legacy vector index worker then indexes both, so that no other work is
    // owed.
    let waiting = clean_envelope(&format!("drain_absent_waiting_{suffix}")).await;
    let at_ceiling = clean_envelope(&format!("drain_absent_ceiling_{suffix}")).await;
    for envelope in [&waiting, &at_ceiling] {
        let receipt = fixture.upload_ok(&fixture.contributor, envelope).await;
        assert_eq!(receipt["status"], "accepted", "{receipt}");
        assert_eq!(
            fixture.runs_of(tenant, envelope.submission_id).await,
            0,
            "a legacy receipt"
        );
    }
    fixture
        .seed_gate_attempts(tenant, at_ceiling.submission_id, GATE_MAX_ATTEMPTS)
        .await;
    let indexed = fixture
        .call_ok(
            "POST",
            "/v1/workers/vector-index",
            &fixture.worker,
            serde_json::json!({
                "purpose": "drain report gate driver mode",
                "dry_run": false,
                "limit": 500,
            }),
        )
        .await;
    assert_eq!(indexed["pending_after_count"], 0, "{indexed}");
    assert_eq!(
        fixture.gate_selected(tenant).await,
        vec![waiting.submission_id],
        "the gate driver selects W; X is at its ceiling"
    );
    let both = vec![waiting.submission_id, at_ceiling.submission_id];
    assert_eq!(fixture.rows_without_a_gate_decision(tenant, &both).await, 2);

    // (2) The gate driver runs: W is pending, X is exhausted, both block.
    let on = fixture.report(tenant).await;
    assert_eq!(on.routing_state, Some(RoutingState::Legacy));
    assert_counts(
        &on,
        &[("gate_decision_pending", 1), ("gate_decision_exhausted", 1)],
        "the gate driver runs",
    );
    assert!(!on.drained);
    assert!(on.gate_driver_enabled);
    assert_eq!(on.not_blocking[ABSENT_LABEL], 0);

    // (3) The gate driver is off, the same rows: both are absent, neither
    // blocks, and the report is drained.
    let off = fixture.report_driver_off(tenant).await;
    assert_eq!(off.routing_state, Some(RoutingState::Legacy));
    assert_counts_driver_off(&off, &[], 2, "the gate driver is off");
    assert_eq!(off.pending["gate_decision_pending"], 0);
    assert_eq!(off.pending["gate_decision_exhausted"], 0);
    assert_eq!(off.not_blocking[ABSENT_LABEL], 2);
    assert!(off.drained, "{:?} {:?}", off.pending, off.not_blocking);
    assert!(!off.gate_driver_enabled);
    assert_modes_agree(&off, &on, "W and X");
    assert_ne!(
        on.evidence_hash, off.evidence_hash,
        "the two modes hash apart"
    );
    assert_eq!(
        off.evidence_hash,
        evidence_hash(&serde_json::json!({
            "schema": "trace_commons.pipeline_legacy_drain.v1",
            "gate_driver_enabled": false,
            "pending": off.pending,
            "not_blocking": { "gate_decision_absent": 2 },
        }))
        .expect("the evidence hash"),
        "the hash names the mode and the not_blocking map"
    );
    let off_again = fixture.report_driver_off(tenant).await;
    assert_eq!(
        off_again.evidence_hash, off.evidence_hash,
        "the same counts in the same mode hash the same"
    );

    // (3b) Task 10: `GET legacy-drain` reads the mode and the ceiling from
    // this process's own gate driver configuration: off by default, on with
    // the driver's `max_attempts`, which moves X between the two labels.
    let with_driver = |max_attempts: Option<i32>| {
        let mut state = fixture.state.clone();
        Arc::make_mut(&mut state).perplexity_score_driver = max_attempts.map(gate_driver_config);
        state
    };
    let routed_off = routed_drain_report(&with_driver(None), &fixture.admin).await;
    assert!(!routed_off.gate_driver_enabled);
    assert_eq!(routed_off.evidence_hash, off.evidence_hash);
    let routed_on =
        routed_drain_report(&with_driver(Some(GATE_MAX_ATTEMPTS)), &fixture.admin).await;
    assert_eq!(routed_on.evidence_hash, on.evidence_hash);
    assert_counts(
        &routed_on,
        &[("gate_decision_pending", 1), ("gate_decision_exhausted", 1)],
        "the route, the process's driver at the ceiling of X's attempts",
    );
    let routed_higher =
        routed_drain_report(&with_driver(Some(GATE_MAX_ATTEMPTS + 1)), &fixture.admin).await;
    assert_counts(
        &routed_higher,
        &[("gate_decision_pending", 2)],
        "the route, the process's driver with a higher ceiling",
    );

    // (4) A quarantined legacy submission waits for a reviewer. With the gate
    // driver off the report is not drained because of it, and the absent
    // count does not hide it.
    let quarantined =
        legacy_quarantine_envelope(&format!("drain absent quarantine {suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &quarantined).await["status"],
        "quarantined"
    );
    assert_eq!(fixture.runs_of(tenant, quarantined.submission_id).await, 0);
    // The gate driver selects W and the quarantined submission Q (each has an
    // envelope and no decision, and the driver's selection has no status
    // filter); X is at its ceiling and is not selected. Three submissions
    // have an envelope and no decision.
    let mut expected_selected = vec![waiting.submission_id, quarantined.submission_id];
    expected_selected.sort();
    let selected = fixture.gate_selected(tenant).await;
    assert_eq!(
        selected, expected_selected,
        "the gate driver selects exactly W and Q"
    );
    let three = vec![
        waiting.submission_id,
        at_ceiling.submission_id,
        quarantined.submission_id,
    ];
    assert_eq!(
        fixture.rows_without_a_gate_decision(tenant, &three).await,
        3
    );
    let absent = 3;
    let off_quarantine = fixture.report_driver_off(tenant).await;
    assert_counts_driver_off(
        &off_quarantine,
        &[("quarantine_review_pending", 1)],
        absent,
        "a quarantined submission, the gate driver off",
    );
    assert!(!off_quarantine.drained);
    assert_eq!(off_quarantine.pending["quarantine_review_pending"], 1);
    let on_quarantine = fixture.report(tenant).await;
    assert_counts(
        &on_quarantine,
        &[
            ("quarantine_review_pending", 1),
            ("gate_decision_pending", selected.len() as u64),
            ("gate_decision_exhausted", 1),
        ],
        "a quarantined submission, the gate driver runs",
    );
    assert_modes_agree(&off_quarantine, &on_quarantine, "a quarantined submission");

    // (5) A submission whose submitted envelope is deleted has nothing for the
    // gate to score: it is not absent, and not exhausted with the driver on.
    assert_eq!(
        fixture
            .owner_execute(
                tenant,
                "UPDATE trace_object_refs SET deleted_at = now()
                  WHERE tenant_id = $1 AND submission_id = $2
                    AND artifact_kind = 'submitted_envelope'",
                &[&tenant, &at_ceiling.submission_id],
            )
            .await,
        1
    );
    let off_deleted = fixture.report_driver_off(tenant).await;
    assert_eq!(off_deleted.not_blocking[ABSENT_LABEL], absent - 1);
    let on_deleted = fixture.report(tenant).await;
    assert_eq!(on_deleted.pending["gate_decision_exhausted"], 0);
    assert_modes_agree(&off_deleted, &on_deleted, "X without its envelope");
}

/// The absent gate count leaves out a submission that a pipeline run owns, as
/// every count does: P (a pipeline receipt) and L (a legacy receipt) have an
/// active submitted envelope and no gate decision, and with the gate driver off
/// the report shows L alone as absent. The raw rows are counted too, so the
/// test cannot pass because P never met the predicate. P stays out in every
/// state the test puts it in, and whatever the attempts of either.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_absent_gate_count_leaves_out_a_pipeline_owned_submission() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();

    write_routing_as_operator(tenant, "legacy").await;
    let legacy = clean_envelope(&format!("drain_absent_twin_l_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &legacy).await["status"],
        "accepted"
    );
    write_routing_as_operator(tenant, "pipeline").await;
    let pipeline = clean_envelope(&format!("drain_absent_twin_p_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &pipeline).await["status"],
        "processing"
    );
    assert_eq!(fixture.runs_of(tenant, legacy.submission_id).await, 0);
    assert_eq!(fixture.runs_of(tenant, pipeline.submission_id).await, 1);
    let (l, p) = (legacy.submission_id, pipeline.submission_id);
    let twins = vec![l, p];

    // (1) The pipeline receipt as the pipeline wrote it: both twins meet the
    // predicate but for the run, and the report shows L alone. L's own vector
    // index work is the only other thing owed.
    assert_eq!(
        fixture.rows_without_a_gate_decision(tenant, &twins).await,
        2,
        "both twins have an envelope and no gate decision"
    );
    assert_eq!(fixture.gate_selected(tenant).await, vec![l]);
    let off = fixture.report_driver_off(tenant).await;
    assert_counts_driver_off(
        &off,
        &[("vector_index_pending", 1)],
        1,
        "a pipeline twin, the gate driver off",
    );
    let on = fixture.report(tenant).await;
    assert_counts(
        &on,
        &[("gate_decision_pending", 1), ("vector_index_pending", 1)],
        "a pipeline twin, the gate driver runs",
    );
    assert_modes_agree(&off, &on, "a pipeline twin");

    // (2) A review approval makes a pipeline receipt `accepted`: the same.
    fixture.set_status(tenant, p, "accepted").await;
    assert_eq!(
        fixture.rows_without_a_gate_decision(tenant, &twins).await,
        2
    );
    let accepted_off = fixture.report_driver_off(tenant).await;
    assert_counts_driver_off(
        &accepted_off,
        &[("vector_index_pending", 1)],
        1,
        "an accepted pipeline twin, the gate driver off",
    );
    assert_eq!(fixture.gate_selected(tenant).await, vec![l]);

    // (3) Both twins at the attempt ceiling: whatever the attempts, L is
    // absent and P is not.
    for id in [l, p] {
        fixture
            .seed_gate_attempts(tenant, id, GATE_MAX_ATTEMPTS)
            .await;
    }
    assert_eq!(
        fixture.rows_without_a_gate_decision(tenant, &twins).await,
        2
    );
    let ceiling_off = fixture.report_driver_off(tenant).await;
    assert_counts_driver_off(
        &ceiling_off,
        &[("vector_index_pending", 1)],
        1,
        "twins at the ceiling, the gate driver off",
    );
    let ceiling_on = fixture.report(tenant).await;
    assert_counts(
        &ceiling_on,
        &[("gate_decision_exhausted", 1), ("vector_index_pending", 1)],
        "twins at the ceiling, the gate driver runs",
    );
    assert_modes_agree(&ceiling_off, &ceiling_on, "twins at the ceiling");
    assert!(fixture.gate_selected(tenant).await.is_empty());
}

/// A settlement line item as `main` writes it into `line_items_json`: a held
/// payout (`near_outbox_id` null, a hold label) or a queued one (an outbox id).
fn line_item(near_outbox_id: Option<Uuid>, hold_reason: Option<&str>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "credit_account_ref": format!("account:{}", Uuid::new_v4()),
        "credit_account_hash": format!("sha256:{}", Uuid::new_v4().simple()),
        "settled_credit_delta_micros": 1_000_000,
        "source_credit_event_ids": [],
        "source_submission_ids": [],
        "source_list_hash": format!("sha256:{}", Uuid::new_v4().simple()),
        "near_status": "pending",
        "near_outbox_id": near_outbox_id,
    });
    if let Some(reason) = hold_reason {
        item["near_payout_hold_reason"] = serde_json::json!(reason);
    }
    item
}

/// `near_payout_unqueued` follows the settlement repair
/// (`repair_missing_near_credit_outbox_items_for_finalized_batches`), through
/// real legacy code. A legacy credit of a settlement-eligible type on an
/// accepted submission whose account has no payout target is settled by the
/// settlement route: the batch is finalized with a hold and no outbox row, so
/// `delayed_credit_unsettled` and `near_outbox_pending` read zero while the
/// payout is not queued. Enrolling a payout target and running the settlement
/// route again queues it. A payout queued at settlement time whose outbox row
/// is lost (the one row this test deletes through the owner connection, as the
/// repair's own comment describes a row lost after the batch write) is counted
/// until the next settlement run writes the row again. Seeded batches that the
/// repair leaves alone are not counted: a batch with no NEAR contract, a batch
/// that is not finalized, and a pipeline batch (an `instrument_id`) whose line
/// items look just like a held payout.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_or_lost_legacy_payout_is_counted_until_the_settlement_repair_queues_it() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();
    write_routing_as_operator(tenant, "legacy").await;
    let account = fixture.account_of(tenant, &fixture.contributor).await;

    // A credit to settle, for an account with no payout target.
    let first = clean_envelope(&format!("drain_payout_1_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &first).await["status"],
        "accepted"
    );
    fixture
        .award_training_utility(first.submission_id, 1.0, &format!("payout-1-{suffix}"))
        .await;
    assert_counts(
        &fixture.report(tenant).await,
        &[
            ("gate_decision_pending", 1),
            ("vector_index_pending", 1),
            ("delayed_credit_unsettled", 1),
        ],
        "a credit to settle",
    );

    // The settlement finalizes the batch and holds the payout.
    let settled = fixture.settle_with_near_contract().await;
    assert_eq!(settled["settled_source_event_count"], 1, "{settled}");
    assert_eq!(settled["near_outbox_item_count"], 0, "{settled}");
    assert_eq!(
        fixture
            .owner_text(
                tenant,
                "SELECT near_contract_id || ':' || (line_items_json -> 0 ->> 'near_payout_hold_reason')
                   FROM trace_credit_settlement_batches
                  WHERE tenant_id = $1 AND status = 'finalized'",
                &[&tenant],
            )
            .await,
        "trace-credits.testnet:none_enrolled",
        "a finalized batch with a contract and a held line item"
    );
    let held = fixture.report(tenant).await;
    assert_counts(
        &held,
        &[
            ("gate_decision_pending", 1),
            ("vector_index_pending", 1),
            ("near_payout_unqueued", 1),
        ],
        "a held payout: the credit reads settled and no outbox row exists",
    );
    assert!(!held.drained);

    // Another run does not queue it: the account still has no payout target.
    let again = fixture.settle_with_near_contract().await;
    assert_eq!(again["settled_source_event_count"], 0, "{again}");
    assert_eq!(
        fixture.report(tenant).await.pending["near_payout_unqueued"],
        1
    );

    // The account enrolls a payout target. Nothing is queued until a
    // settlement run repairs the batch.
    fixture
        .owner
        .insert_near_identity(
            tenant,
            account,
            &format!("ed25519:drain-payout-{suffix}"),
            "drain-payout.near",
            None,
        )
        .await
        .expect("enroll a payout target");
    assert_eq!(
        fixture.report(tenant).await.pending["near_payout_unqueued"],
        1
    );
    fixture.settle_with_near_contract().await;
    let repaired = fixture.report(tenant).await;
    assert_eq!(
        repaired.pending["near_payout_unqueued"], 0,
        "{:?}",
        repaired.pending
    );
    assert_eq!(
        repaired.pending["near_outbox_pending"], 1,
        "the repair wrote a pending outbox row"
    );

    // A payout queued at settlement time, with the target enrolled.
    let second = clean_envelope(&format!("drain_payout_2_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &second).await["status"],
        "accepted"
    );
    fixture
        .award_training_utility(second.submission_id, 0.5, &format!("payout-2-{suffix}"))
        .await;
    let settled = fixture.settle_with_near_contract().await;
    assert_eq!(settled["settled_source_event_count"], 1, "{settled}");
    assert_eq!(settled["near_outbox_item_count"], 1, "{settled}");
    let queued = fixture.report(tenant).await;
    assert_eq!(queued.pending["near_payout_unqueued"], 0);
    assert_eq!(queued.pending["near_outbox_pending"], 2);
    // Its outbox row is lost: the line item names a row the table lacks.
    assert_eq!(
        fixture
            .owner_execute(
                tenant,
                "DELETE FROM trace_near_credit_outbox o
                  USING trace_credit_settlement_batches b
                  WHERE o.tenant_id = $1 AND b.tenant_id = o.tenant_id
                    AND b.settlement_batch_id = o.settlement_batch_id
                    AND b.line_items_json -> 0 ->> 'near_outbox_id' = o.near_outbox_id::text",
                &[&tenant],
            )
            .await,
        1
    );
    let lost = fixture.report(tenant).await;
    assert_eq!(
        lost.pending["near_payout_unqueued"], 1,
        "{:?}",
        lost.pending
    );
    assert_eq!(lost.pending["near_outbox_pending"], 1);
    fixture.settle_with_near_contract().await;
    let rewritten = fixture.report(tenant).await;
    assert_eq!(rewritten.pending["near_payout_unqueued"], 0);
    assert_eq!(rewritten.pending["near_outbox_pending"], 2);

    // What the repair leaves alone: no contract, not finalized, and the
    // pipeline's batch (an instrument id; the batch is seeded as the
    // pipeline's payout batch, with the held and lost items the legacy
    // repair would act on in a batch of `main`'s).
    let items = || {
        serde_json::json!([
            line_item(None, Some("none_enrolled")),
            line_item(Some(Uuid::new_v4()), None),
        ])
    };
    fixture
        .seed_batch_with_items(tenant, "finalized", None, None, items())
        .await;
    fixture
        .seed_batch_with_items(
            tenant,
            "failed",
            Some("trace-credits.testnet"),
            None,
            items(),
        )
        .await;
    fixture
        .seed_batch_with_items(
            tenant,
            "finalized",
            Some("trace-credits.testnet"),
            Some("trace_credit"),
            items(),
        )
        .await;
    assert_eq!(
        fixture.report(tenant).await.pending["near_payout_unqueued"],
        0,
        "no contract, not finalized, and a pipeline batch are not counted"
    );
    // The same items in a finalized batch of `main`'s with a contract are.
    fixture
        .seed_batch_with_items(
            tenant,
            "finalized",
            Some("trace-credits.testnet"),
            None,
            items(),
        )
        .await;
    assert_eq!(
        fixture.report(tenant).await.pending["near_payout_unqueued"],
        2,
        "a held item and an item whose outbox row is missing"
    );
}

/// `withdrawal_completion_pending` follows the withdrawal worker
/// (`list_incomplete_source_session_withdrawals`): a version of a withdrawn
/// source session that has no withdrawal tombstone or still holds content, with
/// no revocation propagation item at all. The count is tenant-wide: the
/// pipeline's version (it has a run) is counted too, because the same worker
/// completes it. The legacy worker route completes both, and the count reads
/// zero. From that complete state, each way that the worker's selection finds a
/// version incomplete is made true on its own, and the report and a dry run of
/// the worker agree on it, including the legal hold's retention policies,
/// which the caller passes.
///
/// The state is seeded the way the pipeline's own withdrawal test seeds it
/// (`a_withdrawal_completion_runs_the_pipeline_follow_up_and_stops_listing_the_version`):
/// the version claims a source session, and the session is withdrawn while the
/// version is not, which is what an account merge leaves when the other
/// account had withdrawn the session.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_incomplete_withdrawal_is_counted_until_the_legacy_worker_completes_it() {
    let Some(fixture) = DrainFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.clone();
    let tenant = tenant.as_str();
    let suffix = Uuid::new_v4().simple().to_string();
    write_routing_as_operator(tenant, "legacy").await;
    let account = fixture.account_of(tenant, &fixture.contributor).await;
    let legacy = clean_envelope(&format!("drain_withdraw_l_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &legacy).await["status"],
        "accepted"
    );
    write_routing_as_operator(tenant, "pipeline").await;
    let pipeline = clean_envelope(&format!("drain_withdraw_p_{suffix}")).await;
    assert_eq!(
        fixture.upload_ok(&fixture.contributor, &pipeline).await["status"],
        "processing"
    );
    for (envelope, label) in [(&legacy, "l"), (&pipeline, "p")] {
        let digest: [u8; 32] = Sha256::digest(format!("{tenant}:{label}").as_bytes())
            .as_slice()
            .try_into()
            .unwrap();
        assert_eq!(
            fixture
                .owner
                .claim_trace_source_session(tenant, account, &digest, envelope.submission_id)
                .await
                .expect("claim the source session"),
            StorageTraceSourceSessionStatus::Active
        );
    }
    assert_eq!(
        fixture.report(tenant).await.pending["withdrawal_completion_pending"],
        0
    );
    assert_eq!(
        fixture
            .owner_execute(
                tenant,
                "UPDATE trace_source_sessions SET withdrawn_at = NOW()
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account],
            )
            .await,
        2
    );

    // Two incomplete versions and no revocation propagation item.
    let incomplete = fixture.report(tenant).await;
    assert_eq!(
        incomplete.pending["withdrawal_completion_pending"], 2,
        "the legacy version and the pipeline's: {:?}",
        incomplete.pending
    );
    assert_eq!(incomplete.pending["revocation_propagation_pending"], 0);
    assert!(!incomplete.drained);
    assert_eq!(
        fixture
            .withdrawals_the_worker_would_complete(&fixture.state)
            .await,
        2
    );

    // The legacy worker completes both.
    for _ in 0..3 {
        let ran = fixture
            .call_ok(
                "POST",
                "/v1/workers/revocation-propagation",
                &fixture.admin,
                serde_json::json!({
                    "purpose": "drain report withdrawal",
                    "dry_run": false,
                    "limit": 100,
                }),
            )
            .await;
        assert_eq!(ran["withdrawal_completions_failed"], 0, "{ran}");
        let report = fixture.report(tenant).await;
        if report.pending["withdrawal_completion_pending"] == 0
            && report.pending["revocation_propagation_pending"] == 0
        {
            break;
        }
    }
    let complete = fixture.report(tenant).await;
    assert_eq!(
        complete.pending["withdrawal_completion_pending"], 0,
        "{:?}",
        complete.pending
    );
    assert_eq!(
        fixture
            .withdrawals_the_worker_would_complete(&fixture.state)
            .await,
        0
    );

    // From the complete state, each way a version is incomplete, alone: the
    // report and the worker's dry run agree, and each is put back.
    let id = legacy.submission_id;
    let derived_id = fixture
        .owner_text(
            tenant,
            "SELECT derived_id::text FROM trace_derived_records
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &id],
        )
        .await
        .parse::<Uuid>()
        .unwrap();
    let policy = fixture
        .owner_text(
            tenant,
            "SELECT retention_policy_id FROM trace_submissions
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &id],
        )
        .await;
    let held_state = {
        let mut state = fixture.state.clone();
        Arc::make_mut(&mut state).legal_hold_retention_policy_ids =
            Arc::new(BTreeSet::from([policy.clone()]));
        state
    };
    let steps: Vec<(&str, String, String)> = vec![
        (
            "no withdrawal tombstone",
            "DELETE FROM trace_withdrawals WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
            "INSERT INTO trace_withdrawals (tenant_id, submission_id, withdrawn_at, prior_status, distribution_reach)
             VALUES ($1, $2, NOW(), 'accepted', 'not_distributed')"
                .to_string(),
        ),
        (
            "an object ref not deleted",
            "UPDATE trace_object_refs SET deleted_at = NULL
              WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
            "UPDATE trace_object_refs SET deleted_at = NOW()
              WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
        ),
        (
            "a derived record not revoked",
            "UPDATE trace_derived_records SET status = 'current'
              WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
            "UPDATE trace_derived_records SET status = 'revoked'
              WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
        ),
        (
            "a gate decision with a dedup assignment",
            "INSERT INTO trace_gate_decisions (
                tenant_id, decision_id, submission_id, gate_policy_version, gate_version_hash,
                perplexity_micros, tail_fraction_micros, perplexity_passed, novelty_score_micros,
                nearest_neighbor_hash, novelty_passed, embedding_evidence_hash,
                attestation_chain_hash, dedup_simhash
             ) VALUES ($1, gen_random_uuid(), $2, 'v', 'h', 0, 0, true, 0, 'h', true, 'h', 'h', 7)"
                .to_string(),
            "DELETE FROM trace_gate_decisions WHERE tenant_id = $1 AND submission_id = $2"
                .to_string(),
        ),
    ];
    for (what, make_incomplete, put_back) in &steps {
        fixture
            .owner_execute(tenant, make_incomplete, &[&tenant, &id])
            .await;
        assert_eq!(
            fixture.report(tenant).await.pending["withdrawal_completion_pending"],
            1,
            "{what}"
        );
        assert_eq!(
            fixture
                .withdrawals_the_worker_would_complete(&fixture.state)
                .await,
            1,
            "{what}"
        );
        fixture
            .owner_execute(tenant, put_back, &[&tenant, &id])
            .await;
        assert_eq!(
            fixture.report(tenant).await.pending["withdrawal_completion_pending"],
            0,
            "{what}, put back"
        );
    }
    // A vector entry that is not invalidated (and not deleted).
    let vector_entry_id = Uuid::new_v4();
    let source_hash = format!("sha256:{}", Uuid::new_v4().simple());
    fixture
        .owner_execute(
            tenant,
            "INSERT INTO trace_vector_entries (
                tenant_id, submission_id, derived_id, vector_entry_id, vector_store,
                embedding_model, embedding_dimension, embedding_version, source_projection,
                source_hash, status
             ) VALUES ($1, $2, $3, $4, 'x', 'm', 8, 'v1', 'canonical_summary', $5, 'active')",
            &[&tenant, &id, &derived_id, &vector_entry_id, &source_hash],
        )
        .await;
    assert_eq!(
        fixture.report(tenant).await.pending["withdrawal_completion_pending"],
        1
    );
    assert_eq!(
        fixture
            .withdrawals_the_worker_would_complete(&fixture.state)
            .await,
        1
    );
    for assignment in ["status = 'invalidated'", "deleted_at = NOW()"] {
        fixture
            .owner_execute(
                tenant,
                "UPDATE trace_vector_entries
                    SET status = 'active', deleted_at = NULL, invalidated_at = NULL
                  WHERE tenant_id = $1 AND vector_entry_id = $2",
                &[&tenant, &vector_entry_id],
            )
            .await;
        fixture
            .owner_execute(
                tenant,
                &format!(
                    "UPDATE trace_vector_entries SET {assignment}
                      WHERE tenant_id = $1 AND vector_entry_id = $2"
                ),
                &[&tenant, &vector_entry_id],
            )
            .await;
        assert_eq!(
            fixture.report(tenant).await.pending["withdrawal_completion_pending"],
            0,
            "{assignment}: an invalidated or deleted entry is not owed"
        );
        assert_eq!(
            fixture
                .withdrawals_the_worker_would_complete(&fixture.state)
                .await,
            0,
            "{assignment}"
        );
    }
    // A token attachment not deleted: owed, unless a legal hold keeps its
    // retention policy, which the caller passes as the worker is given it.
    fixture
        .owner_execute(
            tenant,
            "INSERT INTO trace_token_bundles (
                tenant_id, submission_id, revision, owner_ref, manifest_digest,
                witness_headers, manifest, state, expires_at
             ) VALUES ($1, $2, 'r1', 'principal_sha256:seeded', $3, '{}'::jsonb, '{}'::jsonb,
                       'committed', NOW() + INTERVAL '1 day')",
            &[&tenant, &id, &"0".repeat(64)],
        )
        .await;
    fixture
        .owner_execute(
            tenant,
            "INSERT INTO trace_token_attachments (
                tenant_id, submission_id, revision, artifact_id, object_ref
             ) VALUES ($1, $2, 'r1', 'a1', '{}'::jsonb)",
            &[&tenant, &id],
        )
        .await;
    assert_eq!(
        fixture.report(tenant).await.pending["withdrawal_completion_pending"],
        1
    );
    assert_eq!(
        fixture
            .withdrawals_the_worker_would_complete(&fixture.state)
            .await,
        1
    );
    assert_eq!(
        fixture
            .report_with_held(tenant, std::slice::from_ref(&policy))
            .await
            .pending["withdrawal_completion_pending"],
        0,
        "the legal hold keeps the attachment out of the count"
    );
    assert_eq!(
        fixture
            .withdrawals_the_worker_would_complete(&held_state)
            .await,
        0
    );
    // Task 10: `GET legacy-drain` passes this process's legal-hold list, as
    // the withdrawal worker gets it.
    for (state, owed, what) in [
        (&fixture.state, 1, "no legal hold"),
        (&held_state, 0, "the legal hold of the process"),
    ] {
        assert_eq!(
            routed_drain_report(state, &fixture.admin).await.pending["withdrawal_completion_pending"],
            owed,
            "the route, {what}"
        );
    }
}

// ---------------------------------------------------------------------------
// The admin routes (PR 5, Task 10, P5-D11): qualification, activation,
// rollback, containment, deactivation, the routing view, policy
// interventions, and the legacy drain, each behind an admin credential and
// for the credential's tenant only. Every input of the gate and of the
// qualification comes from the server's state; a request names a bundle, a
// reason, and signed check results, nothing else.
// ---------------------------------------------------------------------------

/// The gate configuration the route tests start ingest with, and the one their
/// production-compatible packages hold: the floors of the runtime suite's
/// `production_compatible_config`.
const ROUTE_MAIN_GATE: MainGateConfig = MainGateConfig {
    perplexity_floor_micros: Some(1_000),
    tail_fraction_floor_micros: Some(1_000),
    novelty_floor_micros: Some(1_000),
    embed_insert_novelty_micros: 50_000,
    top_k: 8,
    chunk_target_tokens: 2048,
    chunk_max_tokens: 3072,
    chunk_cap: 16,
    chunk_min_tokens: 64,
    novelty_utility_microcredits: 0,
};

/// The index ids of A (the assembly's default package) and B. The index id is
/// outside `main`'s gate configuration, so B holds `ROUTE_MAIN_GATE` too and is
/// another bundle with A's dependencies (the runtime suite's `two_qualified_bundles`).
const ROUTE_INDEX_A: &str = "qualified_production_index.v1";
const ROUTE_INDEX_B: &str = "qualified_production_index_b.v1";
const ROUTE_PACKAGE_KEY_ID: &str = "route-package-release-key";
const ROUTE_CHECK_KEY_ID: &str = "route-check-attestation-key";

/// The configuration of a production-compatible package with `index_id`.
fn route_config(index_id: &str) -> CompatibilityBundleConfig {
    CompatibilityBundleConfig::production_compatible(
        "qualified_production_perplexity.v1".to_string(),
        "qualified_production_projection.v1".to_string(),
        index_id.to_string(),
        &ROUTE_MAIN_GATE,
    )
    .expect("a production-compatible configuration")
}

/// The package of `route_config(index_id)`, naming the qualified test scorer
/// and embedder that `qualified_compatibility_pipeline_service` holds.
fn route_package(index_id: &str) -> trace_commons_gate_api::pipeline::BundlePackage {
    MinimalPolicyBundle::compatibility_package(
        &route_config(index_id),
        &QualifiedTestScorer(trace_commons_gate_api::ReferencePerplexityScorer::new()),
        &QualifiedTestEmbedder(trace_commons_gate_api::ReferenceEmbedder::new()),
    )
    .expect("the package builds")
}

/// The runtime suite's `qualified_production_service`, through the seam a
/// boot uses: a compatibility service whose every dependency reports itself
/// production-qualified (`qualified_compatibility_pipeline_service`), with
/// A as its default package.
struct QualifiedRouteAssembler;

impl IngestPipelineRuntimeAssembler for QualifiedRouteAssembler {
    fn assemble(
        &self,
        context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        qualified_compatibility_pipeline_service(
            context.backend,
            context.artifact_store,
            &route_config(ROUTE_INDEX_A),
            Some(context.object_store_name),
            context.novelty_utility_checks,
            context.unqualified_routing_allowed,
        )
    }
}

/// `QualifiedRouteAssembler`'s service, assembled as a boot assembles one:
/// unqualified routing off (production routing), and the pipeline's credit
/// issuer `issuer`. A service without an issuer processes no tenant (the
/// assembly refuses a compatibility runtime that does, with no issuer).
fn assemble_route_service(
    backend: Arc<PgBackend>,
    configured_store: &ConfiguredTraceArtifactStore,
    issuer: Option<&str>,
) -> Arc<PipelineService> {
    let connections = TraceCorpusDbConnections {
        database: backend.clone() as Arc<dyn Database>,
        postgres: backend,
    };
    assemble_ingest_pipeline_runtime(
        Some(&QualifiedRouteAssembler),
        Some(&connections),
        Some(configured_store),
        false,
        PipelineLeaseConfig::default(),
        issuer.is_some(),
        false,
        false,
        None,
        TEST_NEAR_CONFIRMATION_INTERVAL,
        TEST_NEAR_PAYOUT_CONTROLS,
        &PipelineNoveltyUtilityChecks {
            issuer_principal_ref: issuer.map(str::to_string),
            ..PipelineNoveltyUtilityChecks::default()
        },
        ROUTE_MAIN_GATE,
    )
    .expect("assemble the qualified runtime")
    .expect("an assembler was given, so a service is returned")
}

/// Every adapter kind production and no risky flag: the profile the test
/// state's override holds, so that the gate's infrastructure term passes.
fn all_production_infrastructure()
-> trace_commons_server::versioned_pipeline_qualification::ProductionInfrastructureProfile {
    use trace_commons_server::versioned_pipeline_qualification::{
        ProductionAdapterKind, ProductionInfrastructureProfile,
    };
    ProductionInfrastructureProfile {
        authoritative_metadata: ProductionAdapterKind::Production,
        artifact_store: ProductionAdapterKind::Production,
        key_wrapper: ProductionAdapterKind::Production,
        authentication: ProductionAdapterKind::Production,
        plaintext_fallback: false,
        best_effort_database_mirror: false,
        static_bearer_authentication: false,
        hs256_bridge_authentication: false,
        unversioned_policy_dependencies: false,
        live_external_payout_enabled: false,
    }
}

/// The in-process gate driver's configuration with `max_attempts`, as
/// `parse_perplexity_score_driver_config_from_env` builds it when
/// `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED` is on.
fn gate_driver_config(max_attempts: i32) -> PerplexityScoreDriverConfig {
    PerplexityScoreDriverConfig {
        interval: StdDuration::from_secs(60),
        batch_size: 10,
        knobs: PerplexityDriverKnobs {
            skip_duplicates: true,
            skip_duplicate_threshold_micros: 0,
            max_attempts,
        },
        backoff_base_seconds: 0,
    }
}

/// `GET /v1/admin/pipeline/legacy-drain` through `state` with `token`, which
/// must answer a report.
async fn routed_drain_report(state: &Arc<AppState>, token: &str) -> LegacyDrainReport {
    let (status, body) = route_request(
        state.clone(),
        "GET",
        "/v1/admin/pipeline/legacy-drain",
        auth_headers(token),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    serde_json::from_value(body).expect("the route answers a drain report")
}

/// The fields an event of the routing view may carry, and no other.
const ROUTING_EVENT_FIELDS: [&str; 10] = [
    "event_id",
    "action",
    "previous_state",
    "resulting_state",
    "previous_bundle_id",
    "resulting_bundle_id",
    "actor_principal_ref",
    "reason_code",
    "evidence_hash",
    "recorded_at",
];

/// One tenant (with an admin and a contributor) and a bystander tenant (with
/// an admin), over one database and one artifact store, served by a plain
/// router (no worker). The state holds a production-qualified runtime, both
/// trust stores, a test code revision, `ROUTE_MAIN_GATE`, the two tenants on
/// the receipts list with unqualified routing off, and the test-only
/// infrastructure override (every kind production): what a production
/// deployment would hold. A, the runtime's default package, and B are signed
/// by the package key; check results are signed by the check key, which is
/// another key. `bodies` keeps every answer body of `call`.
struct RouteFixture {
    runtime: Arc<PgBackend>,
    owner: Arc<PgBackend>,
    state: Arc<AppState>,
    configured_store: ConfiguredTraceArtifactStore,
    tenant: String,
    admin: String,
    contributor: String,
    other_tenant: String,
    other_admin: String,
    revision: String,
    a: trace_commons_gate_api::pipeline::BundlePackage,
    b: trace_commons_gate_api::pipeline::BundlePackage,
    package_pkcs8: Vec<u8>,
    check_pkcs8: Vec<u8>,
    bodies: std::sync::Mutex<Vec<String>>,
    dir: tempfile::TempDir,
}

impl RouteFixture {
    async fn new() -> Option<Self> {
        use trace_commons_server::versioned_pipeline_qualification::trusted_key_for_pkcs8;

        let runtime = runtime_backend(6).await?;
        let owner = account_owner_backend()
            .await
            .expect("the same variable runtime_backend read is set");
        let suffix = Uuid::new_v4().simple().to_string();
        let tenant = format!("tenant-routes-{suffix}");
        let other_tenant = format!("tenant-routes-bystander-{suffix}");
        let admin = format!("token-routes-admin-{suffix}");
        let contributor = format!("token-routes-contributor-{suffix}");
        let other_admin = format!("token-routes-bystander-admin-{suffix}");
        let dir = tempfile::tempdir().expect("temp dir");
        let artifacts = test_artifact_store(dir.path());
        let configured_store = ConfiguredTraceArtifactStore::legacy(artifacts.clone());
        let service = assemble_route_service(
            runtime.clone(),
            &configured_store,
            Some(TEST_PIPELINE_CREDIT_ISSUER),
        );
        let a = service.default_package().clone();
        assert_eq!(
            a.bundle_id,
            route_package(ROUTE_INDEX_A).bundle_id,
            "A is the package the assembly built"
        );
        let b = route_package(ROUTE_INDEX_B);
        assert_ne!(a.bundle_id, b.bundle_id, "B is another bundle");
        let key = || {
            ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
                .expect("a key pair")
                .as_ref()
                .to_vec()
        };
        let package_pkcs8 = key();
        let check_pkcs8 = key();
        let revision = sha256_prefixed("pr5-admin-routes-revision");
        let mut tokens = BTreeMap::new();
        insert_token(&mut tokens, &tenant, &admin, TokenRole::Admin);
        insert_token(&mut tokens, &tenant, &contributor, TokenRole::Contributor);
        insert_token(&mut tokens, &other_tenant, &other_admin, TokenRole::Admin);
        let mut state = test_state_with_options(
            dir.path().to_path_buf(),
            Some(mains_database().await),
            Some(artifacts),
            false,
            false,
            false,
            false,
        );
        let state_mut = Arc::make_mut(&mut state);
        state_mut.tokens = Arc::new(tokens);
        state_mut.require_db_mirror_writes = true;
        state_mut.pipeline_service = Some(service);
        state_mut.pipeline_product = Some(Arc::new(PipelineProductStore::new(runtime.clone())));
        state_mut.pipeline_store = Some(Arc::new(PgPipelineStore::new(runtime.clone())));
        state_mut.pipeline_activation = routing_store(&runtime);
        state_mut.pipeline_qualification =
            Some(Arc::new(PipelineQualificationStore::new(runtime.clone())));
        state_mut.pipeline_package_trust = Some(Arc::new(
            BundlePackageTrustStore::new([trusted_key_for_pkcs8(
                ROUTE_PACKAGE_KEY_ID,
                &package_pkcs8,
            )
            .expect("the key")])
            .expect("the package trust store"),
        ));
        state_mut.pipeline_check_trust = Some(Arc::new(
            CheckResultTrustStore::new([
                trusted_key_for_pkcs8(ROUTE_CHECK_KEY_ID, &check_pkcs8).expect("the key")
            ])
            .expect("the check trust store"),
        ));
        state_mut.pipeline_code_revision_hash = Some(revision.clone());
        state_mut.pipeline_main_gate = ROUTE_MAIN_GATE;
        state_mut.pipeline_unqualified_routing = false;
        state_mut.pipeline_infrastructure_override = Some(all_production_infrastructure());
        state_mut.tenant_rollout_gates = TraceTenantRolloutGates::for_feature(
            TraceTenantRolloutFeature::PipelineReceipts,
            &[tenant.as_str(), other_tenant.as_str()],
        );
        Some(Self {
            runtime,
            owner,
            state,
            configured_store,
            tenant,
            admin,
            contributor,
            other_tenant,
            other_admin,
            revision,
            a,
            b,
            package_pkcs8,
            check_pkcs8,
            bodies: std::sync::Mutex::new(Vec::new()),
            dir,
        })
    }

    /// The fixture's state with `change` applied, as a second process would
    /// hold it.
    fn with(&self, change: impl FnOnce(&mut AppState)) -> Arc<AppState> {
        let mut state = self.state.clone();
        change(Arc::make_mut(&mut state));
        state
    }

    /// `method path` through a plain router over `state`, with `token` as the
    /// bearer when one is given and `body` as JSON. The answer body is kept.
    async fn call(
        &self,
        state: &Arc<AppState>,
        method: &str,
        path: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let headers = token.map(auth_headers).unwrap_or_default();
        let (status, answer) = route_request(state.clone(), method, path, headers, body).await;
        self.bodies
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(answer.to_string());
        (status, answer)
    }

    /// `call` through the fixture's state with the tenant's admin credential.
    async fn admin_call(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        self.call(&self.state, method, path, Some(&self.admin), body)
            .await
    }

    /// `package`, signed by the package key.
    fn signed(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
    ) -> trace_commons_server::versioned_pipeline_qualification::SignedBundlePackage {
        trace_commons_server::versioned_pipeline_qualification::sign_bundle_package(
            package.clone(),
            ROUTE_PACKAGE_KEY_ID,
            &self.package_pkcs8,
        )
        .expect("the package signs")
    }

    /// A full set of passing check results for `package` on the fixture's
    /// revision, each signed by the check key: the four package checks name
    /// `package`, every other check names none (P5-D15).
    fn attestations(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
    ) -> Vec<trace_commons_server::versioned_pipeline_qualification::PipelineCheckAttestation> {
        self.attestations_signed_by(package, ROUTE_CHECK_KEY_ID, &self.check_pkcs8)
    }

    /// `attestations`, each signed by the key `pkcs8` under `key_id`.
    fn attestations_signed_by(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
        key_id: &str,
        pkcs8: &[u8],
    ) -> Vec<trace_commons_server::versioned_pipeline_qualification::PipelineCheckAttestation> {
        self.attestations_with(package, key_id, pkcs8, 3_600, None)
    }

    /// `attestations`, each signed by the check key with
    /// `maximum_age_seconds`, and with the result of `stale_check` (when one
    /// is named) observed two hours before now, so that it is stale under a
    /// maximum age of one hour.
    fn attestations_aged(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
        maximum_age_seconds: u64,
        stale_check: Option<&str>,
    ) -> Vec<trace_commons_server::versioned_pipeline_qualification::PipelineCheckAttestation> {
        self.attestations_with(
            package,
            ROUTE_CHECK_KEY_ID,
            &self.check_pkcs8,
            maximum_age_seconds,
            stale_check,
        )
    }

    fn attestations_with(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
        key_id: &str,
        pkcs8: &[u8],
        maximum_age_seconds: u64,
        stale_check: Option<&str>,
    ) -> Vec<trace_commons_server::versioned_pipeline_qualification::PipelineCheckAttestation> {
        use trace_commons_server::versioned_pipeline_qualification::{
            PROMOTION_PACKAGE_CHECKS, PROMOTION_REQUIRED_CHECKS, PipelineCheckResult,
            PipelineCheckStatus, sign_check_result,
        };

        let dir = tempfile::tempdir().expect("temp dir");
        let emitter =
            PipelineCheckEmitter::new(dir.path().to_path_buf(), "admin_routes", &self.revision)
                .expect("the emitter's run id and revision are valid");
        PROMOTION_REQUIRED_CHECKS
            .iter()
            .map(|check_id| {
                emitter
                    .emit(
                        check_id,
                        PipelineCheckStatus::Pass,
                        PROMOTION_PACKAGE_CHECKS
                            .contains(check_id)
                            .then_some(package),
                        &[],
                        serde_json::json!({ "check": check_id }),
                    )
                    .expect("the check result is written");
                let bytes = std::fs::read(dir.path().join(format!("{check_id}.result.json")))
                    .expect("the result file");
                let mut result: PipelineCheckResult =
                    serde_json::from_slice(&bytes).expect("the written result reads back");
                if stale_check == Some(*check_id) {
                    result.observed_at -= chrono::Duration::hours(2);
                }
                sign_check_result(result, maximum_age_seconds, None, None, key_id, pkcs8)
                    .expect("the result signs")
            })
            .collect()
    }

    /// The body of `POST qualifications` for `package`.
    fn qualify_body(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
    ) -> serde_json::Value {
        serde_json::json!({
            "signed_package": self.signed(package),
            "attestations": self.attestations(package),
        })
    }

    /// The value of `expected_record_id` for `tenant` now, as an operator
    /// reads it: the `activation_record_id` of the routing view
    /// (`PipelineActivationStore::routing_view`, what `GET routing` answers),
    /// or `none` for a tenant with no routing row. Read through the store, so
    /// the read leaves no answer body and no read audit behind.
    async fn record_id_in_force(&self, tenant: &str) -> serde_json::Value {
        self.store()
            .routing_view(tenant, 1)
            .await
            .expect("the routing view reads")
            .routing
            .map_or_else(
                || serde_json::json!("none"),
                |routing| serde_json::json!(routing.activation_record_id),
            )
    }

    /// The body of `POST activate` (and `rollback`) for `package`, with the
    /// record id that is in force for the fixture's tenant when the body is
    /// built (`record_id_in_force`). Build it just before the request.
    async fn activate_body(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
        reason_code: &str,
    ) -> serde_json::Value {
        let in_force = self.record_id_in_force(&self.tenant).await;
        self.activate_body_expecting(package, reason_code, &in_force)
    }

    /// The nine routes, each with a body it accepts, and whether it needs a
    /// pipeline runtime (`GET routing` and `GET legacy-drain` need only the
    /// routing store).
    async fn routes(&self) -> Vec<(&'static str, String, Option<serde_json::Value>, bool)> {
        vec![
            ("GET", "/v1/admin/pipeline/routing".to_string(), None, false),
            (
                "POST",
                "/v1/admin/pipeline/qualifications".to_string(),
                Some(self.qualify_body(&self.a)),
                true,
            ),
            (
                "POST",
                "/v1/admin/pipeline/activate".to_string(),
                Some(self.activate_body(&self.a, "activate_bundle_a").await),
                true,
            ),
            (
                "POST",
                "/v1/admin/pipeline/rollback".to_string(),
                Some(self.activate_body(&self.a, "roll_back_to_a").await),
                true,
            ),
            (
                "POST",
                "/v1/admin/pipeline/contain".to_string(),
                Some(serde_json::json!({ "reason_code": "contain_for_incident" })),
                true,
            ),
            (
                "POST",
                "/v1/admin/pipeline/deactivate".to_string(),
                Some(serde_json::json!({ "reason_code": "deactivate_to_legacy" })),
                true,
            ),
            (
                "POST",
                "/v1/admin/pipeline/policy-interventions".to_string(),
                Some(serde_json::json!({
                    "bundle_id": self.a.bundle_id,
                    "phase": "score",
                    "action": "suspend",
                    "reason_code": "suspend_score",
                })),
                true,
            ),
            (
                "GET",
                format!(
                    "/v1/admin/pipeline/policy-interventions?bundle_id={}",
                    self.a.bundle_id
                ),
                None,
                true,
            ),
            (
                "GET",
                "/v1/admin/pipeline/legacy-drain".to_string(),
                None,
                false,
            ),
        ]
    }

    /// `POST /v1/traces` of a fresh clean envelope by the contributor through
    /// a plain router over `state`.
    async fn upload(
        &self,
        state: &Arc<AppState>,
        tag: &str,
    ) -> (StatusCode, serde_json::Value, Uuid) {
        let envelope = clean_envelope(&format!("admin_routes_{tag}")).await;
        let (status, body) = route_trace(
            state,
            &self.contributor,
            &serde_json::to_vec(&envelope).unwrap(),
        )
        .await;
        (status, body, envelope.submission_id)
    }

    fn store(&self) -> PipelineActivationStore {
        PipelineActivationStore::new(self.runtime.clone())
    }

    /// The qualification of `package` for `tenant` on the fixture's revision.
    async fn qualification(
        &self,
        tenant: &str,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
    ) -> Option<trace_commons_server::versioned_pipeline_qualification::BundleQualificationRecord>
    {
        PipelineQualificationStore::new(self.runtime.clone())
            .qualification(tenant, &package.bundle_id, &self.revision)
            .await
            .expect("the qualification reads")
    }

    /// `tenant` has no routing row, no event, and no qualification of A or B.
    async fn assert_untouched(&self, tenant: &str, context: &str) {
        assert_eq!(
            self.store()
                .routing(tenant)
                .await
                .expect("the routing reads"),
            None,
            "{context}: no routing row"
        );
        assert!(
            self.store()
                .events(tenant, 10)
                .await
                .expect("the events read")
                .is_empty(),
            "{context}: no event"
        );
        for package in [&self.a, &self.b] {
            assert_eq!(
                self.qualification(tenant, package).await,
                None,
                "{context}: no qualification"
            );
        }
    }

    /// Qualifies `package` for the tenant through the route.
    async fn qualify(&self, package: &trace_commons_gate_api::pipeline::BundlePackage) {
        let (status, record) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/qualifications",
                Some(self.qualify_body(package)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{record}");
    }

    /// The brief's flow: qualify A and B (two rows on the state's revision),
    /// activate A (an upload is `processing`), activate B, roll back to A,
    /// contain (an upload is `503`), deactivate (an upload gets a legacy
    /// receipt), and read the routing view: `legacy`, A active, and the five
    /// events newest first, each with only the allowed fields.
    async fn qualify_activate_roll_back_contain_and_deactivate(&self) {
        let tenant = self.tenant.as_str();
        for package in [&self.a, &self.b] {
            let (status, record) = self
                .admin_call(
                    "POST",
                    "/v1/admin/pipeline/qualifications",
                    Some(self.qualify_body(package)),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{record}");
            assert_eq!(record["bundle_id"], package.bundle_id);
            assert_eq!(record["metadata"]["code_revision_hash"], self.revision);
            let stored = self
                .qualification(tenant, package)
                .await
                .expect("the qualification row");
            assert_eq!(stored.metadata.code_revision_hash, self.revision);
        }

        let (status, routing) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.activate_body(&self.a, "activate_bundle_a").await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
        assert_eq!(routing["routing_state"], "pipeline");
        let (status, receipt, _) = self.upload(&self.state, "activated").await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        assert_eq!(receipt["status"], "processing", "{receipt}");

        let (status, routing) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.activate_body(&self.b, "activate_bundle_b").await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
        assert_eq!(routing["routing_state"], "pipeline");

        let (status, routing) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/rollback",
                Some(self.activate_body(&self.a, "roll_back_to_a").await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
        assert_eq!(routing["routing_state"], "pipeline");

        let (status, routing) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/contain",
                Some(serde_json::json!({ "reason_code": "contain_for_incident" })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
        assert_eq!(routing["routing_state"], "contained");
        let (status, refused, _) = self.upload(&self.state, "contained").await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
        assert_eq!(refused["error"], "pipeline_receipt_intake_contained");

        // A deactivation of a contained tenant names its expectation: the
        // record id that the containment answered.
        let contained_record_id = routing["activation_record_id"].clone();
        assert_eq!(contained_record_id, self.record_id_in_force(tenant).await);
        let (status, routing) = self
            .admin_call(
                "POST",
                "/v1/admin/pipeline/deactivate",
                Some(serde_json::json!({
                    "reason_code": "deactivate_to_legacy",
                    "expected_record_id": contained_record_id,
                })),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
        assert_eq!(routing["routing_state"], "legacy");
        let legacy_record_id = routing["activation_record_id"].clone();
        assert_ne!(legacy_record_id, contained_record_id);
        let (status, receipt, legacy_id) = self.upload(&self.state, "deactivated").await;
        assert_eq!(status, StatusCode::OK, "{receipt}");
        assert_ne!(
            receipt["status"], "processing",
            "a legacy receipt: {receipt}"
        );
        assert_eq!(
            self.store()
                .ownership(tenant, legacy_id)
                .await
                .expect("the ownership reads")
                .map(|row| row.owner),
            Some(trace_commons_server::versioned_pipeline_activation::ReceiptOwner::Legacy)
        );

        let (status, view) = self
            .admin_call("GET", "/v1/admin/pipeline/routing", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
        assert_eq!(
            view.as_object()
                .expect("an object")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "routing_state",
                "activation_record_id",
                "active_bundle_id",
                "events"
            ]),
            "{view}"
        );
        assert_eq!(view["routing_state"], "legacy");
        assert_eq!(
            view["activation_record_id"], legacy_record_id,
            "the view shows the record id that the last change answered"
        );
        assert_eq!(view["events"][0]["event_id"], legacy_record_id);
        assert_eq!(view["active_bundle_id"], self.a.bundle_id);
        let events = view["events"].as_array().expect("the events");
        assert_eq!(
            events
                .iter()
                .map(|event| event["action"].as_str().expect("an action"))
                .collect::<Vec<_>>(),
            vec!["deactivate", "contain", "rollback", "activate", "activate"],
            "five events, newest first"
        );
        assert_eq!(events[2]["resulting_bundle_id"], self.a.bundle_id);
        assert_eq!(events[3]["resulting_bundle_id"], self.b.bundle_id);
        assert_eq!(events[4]["resulting_bundle_id"], self.a.bundle_id);
        for event in events {
            assert_eq!(
                event
                    .as_object()
                    .expect("an event object")
                    .keys()
                    .map(String::as_str)
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(ROUTING_EVENT_FIELDS),
                "{event}"
            );
        }
    }

    /// The tenant's control-plane read audit reasons for `surface`, from the
    /// file audit log (`read_all_audit_events`), oldest first.
    fn read_audits(&self, tenant: &str, surface: &str) -> Vec<String> {
        let prefix = format!("surface={surface};");
        read_all_audit_events(self.dir.path(), tenant)
            .expect("the audit log reads")
            .into_iter()
            .filter(|event| event.kind == "read")
            .filter_map(|event| event.reason)
            .filter(|reason| reason.starts_with(&prefix))
            .collect()
    }

    /// After the flow (A qualified): a suspension and a resumption of A's
    /// Score policy, the list of both, the drain report (with reviewer reads
    /// from the database), and the routing view, for a test that reads every
    /// answer and log line.
    async fn intervene_and_read(&self) {
        for (action, resulting) in [("suspend", "suspended"), ("resume", "runnable")] {
            let (status, record) = self
                .admin_call(
                    "POST",
                    "/v1/admin/pipeline/policy-interventions",
                    Some(serde_json::json!({
                        "bundle_id": self.a.bundle_id,
                        "phase": "score",
                        "action": action,
                        "reason_code": format!("{action}_score_for_hygiene"),
                    })),
                )
                .await;
            assert_eq!(status, StatusCode::OK, "{record}");
            assert_eq!(record["resulting_status"], resulting);
        }
        let (status, listed) = self
            .admin_call(
                "GET",
                &format!(
                    "/v1/admin/pipeline/policy-interventions?bundle_id={}",
                    self.a.bundle_id
                ),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        assert_eq!(listed["interventions"].as_array().map(Vec::len), Some(2));
        let (status, report) = self
            .call(
                &self.with(|state| state.db_reviewer_reads = true),
                "GET",
                "/v1/admin/pipeline/legacy-drain",
                Some(&self.admin),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{report}");
        let (status, view) = self
            .admin_call("GET", "/v1/admin/pipeline/routing", None)
            .await;
        assert_eq!(status, StatusCode::OK, "{view}");
    }

    /// One call of each refusal the routes have, for a test that reads every
    /// answer and log line: no credential, a contributor credential, no
    /// runtime, a missing trust store, a missing revision, a body with a
    /// tenant field, too many attestations, a body above the ingest limit, a
    /// bundle the tenant does not have, `terminate`, the drain precondition,
    /// the drain report, and the bystander's own containment.
    async fn refuse_every_way(&self) {
        let activate = Some(self.activate_body(&self.a, "activate_bundle_a").await);
        let expected = self.record_id_in_force(&self.tenant).await;
        let cases: Vec<(
            Arc<AppState>,
            &str,
            &str,
            Option<&str>,
            Option<serde_json::Value>,
        )> = vec![
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/activate",
                None,
                activate.clone(),
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.contributor.as_str()),
                activate.clone(),
            ),
            (
                self.with(|state| {
                    state.pipeline_service = None;
                    state.pipeline_product = None;
                }),
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.admin.as_str()),
                activate.clone(),
            ),
            (
                self.with(|state| state.pipeline_check_trust = None),
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.admin.as_str()),
                activate.clone(),
            ),
            (
                self.with(|state| state.pipeline_code_revision_hash = None),
                "POST",
                "/v1/admin/pipeline/qualifications",
                Some(self.admin.as_str()),
                Some(self.qualify_body(&self.a)),
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/contain",
                Some(self.admin.as_str()),
                Some(serde_json::json!({
                    "reason_code": "contain_for_incident",
                    "tenant_id": self.other_tenant,
                })),
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.admin.as_str()),
                Some(serde_json::json!({
                    "bundle_id": self.a.bundle_id,
                    "reason_code": "activate_bundle_a",
                    "attestations": vec![self.attestations(&self.a)[0].clone(); 65],
                    "expected_record_id": expected.clone(),
                })),
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/activate",
                Some(self.admin.as_str()),
                Some(serde_json::json!({
                    "bundle_id": format!("sha256:{}", "0".repeat(64)),
                    "reason_code": "activate_unknown_bundle",
                    "attestations": self.attestations(&self.a),
                    "expected_record_id": expected.clone(),
                })),
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/policy-interventions",
                Some(self.admin.as_str()),
                Some(serde_json::json!({
                    "bundle_id": self.a.bundle_id,
                    "phase": "score",
                    "action": "terminate",
                    "reason_code": "terminate_score",
                })),
            ),
            (
                self.state.clone(),
                "GET",
                "/v1/admin/pipeline/legacy-drain",
                Some(self.admin.as_str()),
                None,
            ),
            (
                self.with(|state| state.db_reviewer_reads = true),
                "GET",
                "/v1/admin/pipeline/legacy-drain",
                Some(self.admin.as_str()),
                None,
            ),
            (
                self.state.clone(),
                "POST",
                "/v1/admin/pipeline/contain",
                Some(self.other_admin.as_str()),
                Some(serde_json::json!({ "reason_code": "contain_bystander" })),
            ),
        ];
        for (state, method, path, token, body) in cases {
            let (status, _) = self.call(&state, method, path, token, body).await;
            assert_ne!(
                status,
                StatusCode::INTERNAL_SERVER_ERROR,
                "{method} {path}: a refusal has a label"
            );
        }
        let oversized = serde_json::json!({
            "reason_code": "x".repeat(MAX_INGEST_BODY_BYTES),
        });
        let (status, refused) = self
            .call(
                &self.state,
                "POST",
                "/v1/admin/pipeline/contain",
                Some(&self.admin),
                Some(oversized),
            )
            .await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{refused}");
    }
}

/// Each route answers a request without a credential as the operational
/// summary does, answers `403 admin token required` to a contributor, and
/// (but for the routing view and the drain report, which need only the
/// routing store) `404` in a process with no pipeline runtime.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_admin_routes_require_an_admin_and_a_runtime() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let reference = fixture
        .call(
            &fixture.state,
            "GET",
            "/v1/admin/pipeline/operational-summary",
            None,
            None,
        )
        .await;
    assert_eq!(reference.0, StatusCode::UNAUTHORIZED, "{}", reference.1);
    let routes = fixture.routes().await;
    for (method, path, body, _) in &routes {
        assert_eq!(
            fixture
                .call(&fixture.state, method, path, None, body.clone())
                .await,
            reference,
            "{method} {path} without a credential"
        );
        assert_eq!(
            fixture
                .call(
                    &fixture.state,
                    method,
                    path,
                    Some(&fixture.contributor),
                    body.clone()
                )
                .await,
            (
                StatusCode::FORBIDDEN,
                serde_json::json!({ "error": "admin token required" })
            ),
            "{method} {path} with a contributor credential"
        );
    }
    let no_runtime = fixture.with(|state| {
        state.pipeline_service = None;
        state.pipeline_product = None;
        state.db_reviewer_reads = true;
    });
    for (method, path, body, needs_runtime) in &routes {
        let (status, answer) = fixture
            .call(
                &no_runtime,
                method,
                path,
                Some(&fixture.admin),
                body.clone(),
            )
            .await;
        if *needs_runtime {
            assert_eq!(
                (status, answer),
                (
                    StatusCode::NOT_FOUND,
                    serde_json::json!({ "error": "pipeline runtime not configured" })
                ),
                "{method} {path} with no runtime"
            );
        } else {
            assert_eq!(status, StatusCode::OK, "{method} {path}: {answer}");
        }
    }
    fixture
        .assert_untouched(&fixture.tenant, "the refused calls")
        .await;
}

/// With the real profile of a test deployment (a local artifact store, static
/// tokens), a valid signed package with valid signed results is refused at
/// the qualification with the first blocker, and writes nothing; a bundle that
/// a production profile qualified is refused at the activation the same way.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_test_deployment_cannot_qualify_or_activate_through_the_route() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let real = fixture.with(|state| state.pipeline_infrastructure_override = None);
    let profile = pipeline_activation::infrastructure_profile_from_state(&real);
    assert!(profile.static_bearer_authentication, "{profile:?}");
    assert_ne!(
        profile.artifact_store,
        trace_commons_server::versioned_pipeline_qualification::ProductionAdapterKind::Production,
        "{profile:?}"
    );

    let (status, refused) = fixture
        .call(
            &real,
            "POST",
            "/v1/admin/pipeline/qualifications",
            Some(&fixture.admin),
            Some(fixture.qualify_body(&fixture.a)),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "artifact_store_not_production");
    fixture
        .assert_untouched(&fixture.tenant, "a refused qualification")
        .await;

    fixture.qualify(&fixture.a).await;
    let (status, refused) = fixture
        .call(
            &real,
            "POST",
            "/v1/admin/pipeline/activate",
            Some(&fixture.admin),
            Some(fixture.activate_body(&fixture.a, "activate_bundle_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{refused}");
    assert_eq!(refused["error"], "artifact_store_not_production");
    assert_eq!(
        fixture.store().routing(&fixture.tenant).await.unwrap(),
        None
    );
    assert!(
        fixture
            .store()
            .events(&fixture.tenant, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The brief's flow through the routes alone: qualify, activate, an upload to
/// the pipeline, activate another bundle, roll back, contain, deactivate, and
/// the routing view.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_routes_qualify_activate_roll_back_contain_and_deactivate() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture
        .qualify_activate_roll_back_contain_and_deactivate()
        .await;
}

/// No trust store or no deployed revision: the qualification, the activation,
/// and the rollback are refused before anything is read or written; so are
/// more than 64 attestations and a body above the ingest limit.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_routes_refuse_without_a_trust_store_or_a_revision() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let calls = [
        (
            "/v1/admin/pipeline/qualifications",
            fixture.qualify_body(&fixture.a),
        ),
        (
            "/v1/admin/pipeline/activate",
            fixture.activate_body(&fixture.a, "activate_bundle_a").await,
        ),
        (
            "/v1/admin/pipeline/rollback",
            fixture.activate_body(&fixture.a, "roll_back_to_a").await,
        ),
    ];
    for (what, state) in [
        (
            "no check trust store",
            fixture.with(|state| state.pipeline_check_trust = None),
        ),
        (
            "no package trust store",
            fixture.with(|state| state.pipeline_package_trust = None),
        ),
    ] {
        for (path, body) in &calls {
            let (status, refused) = fixture
                .call(
                    &state,
                    "POST",
                    path,
                    Some(&fixture.admin),
                    Some(body.clone()),
                )
                .await;
            assert_eq!(
                (status, refused),
                (
                    StatusCode::SERVICE_UNAVAILABLE,
                    serde_json::json!({ "error": "pipeline_trust_store_missing" })
                ),
                "{what}: {path}"
            );
        }
    }
    let no_revision = fixture.with(|state| state.pipeline_code_revision_hash = None);
    for (path, body) in &calls {
        let (status, refused) = fixture
            .call(
                &no_revision,
                "POST",
                path,
                Some(&fixture.admin),
                Some(body.clone()),
            )
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": "bundle_runtime_revision_unknown" })
            ),
            "no revision: {path}"
        );
    }
    let attestation = fixture.attestations(&fixture.a)[0].clone();
    let expected = fixture.record_id_in_force(&fixture.tenant).await;
    for (path, body) in [
        (
            "/v1/admin/pipeline/qualifications",
            serde_json::json!({
                "signed_package": fixture.signed(&fixture.a),
                "attestations": vec![attestation.clone(); 65],
            }),
        ),
        (
            "/v1/admin/pipeline/activate",
            serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "reason_code": "activate_bundle_a",
                "attestations": vec![attestation.clone(); 65],
                "expected_record_id": expected,
            }),
        ),
    ] {
        let (status, refused) = fixture.admin_call("POST", path, Some(body)).await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::PAYLOAD_TOO_LARGE,
                serde_json::json!({ "error": "pipeline_evidence_too_large" })
            ),
            "65 attestations: {path}"
        );
    }
    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/qualifications",
            Some(serde_json::json!({
                "signed_package": fixture.signed(&fixture.a),
                "attestations": ["x".repeat(MAX_INGEST_BODY_BYTES)],
            })),
        )
        .await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{refused}");
    fixture
        .assert_untouched(&fixture.tenant, "the refused calls")
        .await;
}

/// An activation through the route keeps the startup checks of a tenant
/// bundle (`check_runnable_package`): a qualified bundle whose compatibility
/// configuration is not `main`'s gate configuration, and a service with no
/// pipeline credit issuer, are refused with the startup labels.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activation_through_the_route_keeps_the_startup_checks() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let other_gate = fixture.with(|state| {
        state.pipeline_main_gate = MainGateConfig {
            perplexity_floor_micros: Some(2_000),
            ..ROUTE_MAIN_GATE
        }
    });
    let no_issuer = fixture.with(|state| {
        state.pipeline_service = Some(assemble_route_service(
            fixture.runtime.clone(),
            &fixture.configured_store,
            None,
        ))
    });
    for (state, label) in [
        (&other_gate, "pipeline_runtime_main_gate_config_mismatch"),
        (&no_issuer, "pipeline_credit_issuer_principal_missing"),
    ] {
        let (status, refused) = fixture
            .call(
                state,
                "POST",
                "/v1/admin/pipeline/activate",
                Some(&fixture.admin),
                Some(fixture.activate_body(&fixture.a, "activate_bundle_a").await),
            )
            .await;
        assert_eq!(
            (status, refused),
            (StatusCode::CONFLICT, serde_json::json!({ "error": label }))
        );
        assert_eq!(
            fixture.store().routing(&fixture.tenant).await.unwrap(),
            None,
            "{label}: no routing row"
        );
    }
    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(fixture.activate_body(&fixture.a, "activate_bundle_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{routing}");
    assert_eq!(routing["routing_state"], "pipeline");
}

/// A route acts on the credential's tenant only: the bystander's admin
/// contains its own tenant and leaves the tenant's routing as it was, and a
/// body that names a tenant is refused (`deny_unknown_fields`) before anything
/// is read.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_route_never_acts_on_another_tenant() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let other = fixture.other_tenant.as_str();
    fixture.qualify(&fixture.a).await;
    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(fixture.activate_body(&fixture.a, "activate_bundle_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{routing}");
    let before = fixture.store().routing(tenant).await.unwrap();
    let events_before = fixture.store().events(tenant, 10).await.unwrap();

    let (status, contained) = fixture
        .call(
            &fixture.state,
            "POST",
            "/v1/admin/pipeline/contain",
            Some(&fixture.other_admin),
            Some(serde_json::json!({ "reason_code": "contain_bystander" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{contained}");
    assert_eq!(contained["routing_state"], "contained");
    assert_eq!(fixture.store().routing(tenant).await.unwrap(), before);
    assert_eq!(
        fixture.store().events(tenant, 10).await.unwrap(),
        events_before
    );
    assert_eq!(
        fixture
            .store()
            .routing(other)
            .await
            .unwrap()
            .map(|row| row.routing_state),
        Some(RoutingState::Contained)
    );

    let bystander_record_id = fixture.record_id_in_force(other).await;
    for (path, body) in [
        (
            "/v1/admin/pipeline/contain",
            serde_json::json!({ "reason_code": "contain_named_tenant", "tenant_id": tenant }),
        ),
        (
            "/v1/admin/pipeline/deactivate",
            serde_json::json!({ "reason_code": "deactivate_named_tenant", "tenant_id": tenant }),
        ),
        (
            "/v1/admin/pipeline/activate",
            serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "reason_code": "activate_named_tenant",
                "attestations": fixture.attestations(&fixture.a),
                "expected_record_id": bystander_record_id,
                "tenant_id": tenant,
            }),
        ),
    ] {
        let (status, refused) = fixture
            .call(
                &fixture.state,
                "POST",
                path,
                Some(&fixture.other_admin),
                Some(body),
            )
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!({ "error": "pipeline_request_invalid" })
            ),
            "{path} with a tenant field"
        );
    }
    assert_eq!(fixture.store().routing(tenant).await.unwrap(), before);
    assert_eq!(
        fixture.store().events(tenant, 10).await.unwrap(),
        events_before
    );
    assert_eq!(
        fixture.store().events(other, 10).await.unwrap().len(),
        1,
        "the bystander's one containment"
    );

    let (status, view) = fixture
        .call(
            &fixture.state,
            "GET",
            "/v1/admin/pipeline/routing",
            Some(&fixture.other_admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["routing_state"], "contained");
    assert_eq!(view["active_bundle_id"], serde_json::Value::Null);
    assert_eq!(view["events"].as_array().map(Vec::len), Some(1));
}

/// `suspend` and `resume` of the Score policy, the list of both, and
/// `terminate`, which PR 5 refuses; a bundle the tenant does not have is `404`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_policy_routes_suspend_resume_and_list() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let intervene = |action: &str, bundle_id: &str| {
        serde_json::json!({
            "bundle_id": bundle_id,
            "phase": "score",
            "action": action,
            "reason_code": format!("{action}_score_for_test"),
        })
    };
    for (action, resulting) in [("suspend", "suspended"), ("resume", "runnable")] {
        let (status, record) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/policy-interventions",
                Some(intervene(action, &fixture.a.bundle_id)),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{record}");
        assert_eq!(record["action"], action);
        assert_eq!(record["phase"], "score");
        assert_eq!(record["bundle_id"], fixture.a.bundle_id);
        assert_eq!(record["resulting_status"], resulting);
    }
    let list_path = format!(
        "/v1/admin/pipeline/policy-interventions?bundle_id={}",
        fixture.a.bundle_id
    );
    let (status, listed) = fixture.admin_call("GET", &list_path, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(
        listed["interventions"]
            .as_array()
            .expect("the interventions")
            .iter()
            .map(|record| record["action"].as_str().expect("an action"))
            .collect::<Vec<_>>(),
        vec!["suspend", "resume"]
    );

    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/policy-interventions",
            Some(intervene("terminate", &fixture.a.bundle_id)),
        )
        .await;
    assert_eq!(
        (status, refused),
        (
            StatusCode::CONFLICT,
            serde_json::json!({ "error": "policy_intervention_not_supported" })
        )
    );
    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/policy-interventions",
            Some(intervene("suspend", &fixture.b.bundle_id)),
        )
        .await;
    assert_eq!(
        (status, refused),
        (
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "bundle_package_missing" })
        ),
        "B is not the tenant's bundle"
    );
    let (status, listed) = fixture.admin_call("GET", &list_path, None).await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    assert_eq!(listed["interventions"].as_array().map(Vec::len), Some(2));
}

/// `GET legacy-drain` is the store's report for the tenant, in the mode of
/// this process's gate driver: labels, counts, a state, a time, and a hash.
/// It is refused unless the tenant's legacy records in the database are
/// authoritative (database writes required and database reviewer reads).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_legacy_drain_route_returns_the_report() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    for (what, driver, ceiling) in [
        ("the gate driver off", None, None),
        ("the gate driver on", Some(gate_driver_config(7)), Some(7)),
    ] {
        let state = fixture.with(|state| {
            state.db_reviewer_reads = true;
            state.perplexity_score_driver = driver;
        });
        let (status, body) = fixture
            .call(
                &state,
                "GET",
                "/v1/admin/pipeline/legacy-drain",
                Some(&fixture.admin),
                None,
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{what}: {body}");
        let object = body.as_object().expect("an object");
        assert_eq!(
            object.keys().map(String::as_str).collect::<BTreeSet<_>>(),
            BTreeSet::from([
                "generated_at",
                "routing_state",
                "gate_driver_enabled",
                "pending",
                "not_blocking",
                "drained",
                "evidence_hash",
            ]),
            "{what}"
        );
        for map in ["pending", "not_blocking"] {
            for (label, count) in object[map].as_object().expect("a map") {
                assert!(
                    label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
                    "{what}: {label}"
                );
                assert!(count.is_u64(), "{what}: {label} is a count");
            }
        }
        assert!(object["routing_state"].is_null(), "{what}: no routing row");
        assert!(object["drained"].is_boolean());
        let routed: LegacyDrainReport =
            serde_json::from_value(body).expect("the route answers a drain report");
        let expected = fixture
            .store()
            .legacy_drain_report(tenant, ceiling, &[])
            .await
            .expect("the store's report");
        assert_eq!(
            LegacyDrainReport {
                generated_at: expected.generated_at,
                ..routed
            },
            expected,
            "{what}"
        );
        assert_eq!(expected.gate_driver_enabled, ceiling.is_some(), "{what}");
    }

    for (what, state) in [
        (
            "best-effort database writes",
            fixture.with(|state| {
                state.db_reviewer_reads = true;
                state.require_db_mirror_writes = false;
            }),
        ),
        ("reviewer reads from the file store", fixture.state.clone()),
    ] {
        let (status, refused) = fixture
            .call(
                &state,
                "GET",
                "/v1/admin/pipeline/legacy-drain",
                Some(&fixture.admin),
                None,
            )
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": "legacy_drain_records_not_authoritative" })
            ),
            "{what}"
        );
    }
}

/// Every log line written on this thread while it lives: a scoped default
/// subscriber (`tracing::subscriber::set_default`) at every level. The route
/// calls of a test run on the test's own thread (the router is called in
/// place, not spawned), so their lines are all here.
struct CapturedLogs {
    sink: Arc<std::sync::Mutex<Vec<u8>>>,
    _guard: tracing::subscriber::DefaultGuard,
}

struct CapturedLogWriter(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLogWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl CapturedLogs {
    fn start() -> Self {
        let sink = Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = sink.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .with_writer(move || CapturedLogWriter(writer.clone()))
            .finish();
        Self {
            sink,
            _guard: tracing::subscriber::set_default(subscriber),
        }
    }

    fn lines(&self) -> Vec<String> {
        String::from_utf8_lossy(
            &self
                .sink
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
        .lines()
        .map(str::to_string)
        .collect()
    }
}

/// Every answer of the routes in the flow and in each refusal, and every log
/// line written meanwhile, holds no credential and no tenant id; the log lines
/// of the successful actions are among them.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn no_route_response_or_log_line_holds_a_token_or_a_tenant_id() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let logs = CapturedLogs::start();
    fixture
        .qualify_activate_roll_back_contain_and_deactivate()
        .await;
    fixture.intervene_and_read().await;
    fixture.refuse_every_way().await;
    let lines = logs.lines();
    drop(logs);
    for action in ["policy_suspend", "policy_resume"] {
        assert!(
            lines.iter().any(
                |line| line.contains("pipeline admin action recorded") && line.contains(action)
            ),
            "the {action} line is logged"
        );
    }
    let bodies = fixture
        .bodies
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone();
    let actions = lines
        .iter()
        .filter(|line| line.contains("pipeline admin action recorded"))
        .count();
    assert!(
        actions >= 10,
        "the qualifications, activations, rollback, containments, deactivation, \
         suspension, and resumption are logged ({actions} lines)"
    );
    assert!(bodies.len() >= 20, "{} answers", bodies.len());
    for (what, secret) in [
        ("the admin token", fixture.admin.as_str()),
        ("the contributor token", fixture.contributor.as_str()),
        ("the bystander's admin token", fixture.other_admin.as_str()),
        ("the tenant id", fixture.tenant.as_str()),
        ("the bystander's tenant id", fixture.other_tenant.as_str()),
    ] {
        for (index, body) in bodies.iter().enumerate() {
            assert!(!body.contains(secret), "answer {index} holds {what}");
        }
        for (index, line) in lines.iter().enumerate() {
            assert!(!line.contains(secret), "log line {index} holds {what}");
        }
    }
}

/// The qualification and the activation take only results that a key of the
/// check trust store signed, unchanged since: results signed by the package
/// key (a key the check store does not hold) and a result changed after
/// signing are refused with the check store's labels, and a package signed by
/// the check key with the package store's. Nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_routes_take_only_results_a_check_key_signed() {
    use trace_commons_server::versioned_pipeline_qualification::{
        CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL, CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL,
        PACKAGE_SIGNER_UNTRUSTED_LABEL, sign_bundle_package,
    };

    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let by_package_key =
        fixture.attestations_signed_by(&fixture.a, ROUTE_PACKAGE_KEY_ID, &fixture.package_pkcs8);
    let mut changed = fixture.attestations(&fixture.a);
    changed[0].maximum_age_seconds += 1;
    let signed_by_check_key =
        sign_bundle_package(fixture.a.clone(), ROUTE_CHECK_KEY_ID, &fixture.check_pkcs8)
            .expect("the package signs");
    for (what, body, label) in [
        (
            "results signed by the package key",
            serde_json::json!({
                "signed_package": fixture.signed(&fixture.a),
                "attestations": by_package_key,
            }),
            CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL,
        ),
        (
            "a result changed after signing",
            serde_json::json!({
                "signed_package": fixture.signed(&fixture.a),
                "attestations": changed,
            }),
            CHECK_ATTESTATION_SIGNATURE_INVALID_LABEL,
        ),
        (
            "a package signed by the check key",
            serde_json::json!({
                "signed_package": signed_by_check_key,
                "attestations": fixture.attestations(&fixture.a),
            }),
            PACKAGE_SIGNER_UNTRUSTED_LABEL,
        ),
    ] {
        let (status, refused) = fixture
            .admin_call("POST", "/v1/admin/pipeline/qualifications", Some(body))
            .await;
        assert_eq!(
            (status, refused),
            (StatusCode::CONFLICT, serde_json::json!({ "error": label })),
            "{what}"
        );
    }
    fixture
        .assert_untouched(&fixture.tenant, "refused qualifications")
        .await;

    fixture.qualify(&fixture.a).await;
    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "reason_code": "activate_bundle_a",
                "attestations": by_package_key,
                "expected_record_id": fixture.record_id_in_force(&fixture.tenant).await,
            })),
        )
        .await;
    assert_eq!(
        (status, refused),
        (
            StatusCode::CONFLICT,
            serde_json::json!({ "error": CHECK_ATTESTATION_SIGNER_UNTRUSTED_LABEL })
        )
    );
    assert_eq!(
        fixture.store().routing(&fixture.tenant).await.unwrap(),
        None
    );
}

/// A request cannot supply an input of the gate or of the qualification: a
/// body with a field for the revision, the promotion, the readiness, the
/// dependencies, the infrastructure, the tenant, or the actor is refused
/// (`deny_unknown_fields`), even where the state lacks the value. Nothing is
/// written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_request_cannot_supply_a_gate_input() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let no_revision = fixture.with(|state| state.pipeline_code_revision_hash = None);
    let fields = [
        ("code_revision_hash", serde_json::json!(fixture.revision)),
        (
            "runtime_code_revision_hash",
            serde_json::json!(fixture.revision),
        ),
        ("promotion", serde_json::json!({ "ready": true })),
        ("readiness", serde_json::json!({ "readiness_ok": true })),
        ("dependencies", serde_json::json!({})),
        ("infrastructure", serde_json::json!({})),
        ("metadata", serde_json::json!({})),
        ("tenant_id", serde_json::json!(fixture.other_tenant)),
        (
            "actor_principal_ref",
            serde_json::json!("principal_sha256:00"),
        ),
    ];
    for state in [&fixture.state, &no_revision] {
        for (path, body) in [
            (
                "/v1/admin/pipeline/qualifications",
                fixture.qualify_body(&fixture.a),
            ),
            (
                "/v1/admin/pipeline/activate",
                fixture.activate_body(&fixture.a, "activate_bundle_a").await,
            ),
            (
                "/v1/admin/pipeline/rollback",
                fixture.activate_body(&fixture.a, "roll_back_to_a").await,
            ),
        ] {
            for (field, value) in &fields {
                let mut body = body.clone();
                body[*field] = value.clone();
                let (status, refused) = fixture
                    .call(state, "POST", path, Some(&fixture.admin), Some(body))
                    .await;
                assert_eq!(
                    (status, refused),
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        serde_json::json!({ "error": "pipeline_request_invalid" })
                    ),
                    "{path} with {field}"
                );
            }
        }
    }
    fixture
        .assert_untouched(&fixture.tenant, "bodies with a gate input")
        .await;
}

/// An activation through the route reads the tenant's readiness from its
/// operational summary, and a rollback reads none: with a pipeline run of
/// the tenant waiting longer than `ACTIVATION_MAX_WORK_AGE_SECONDS`, the
/// activation is refused with `activation_readiness_failed` and changes
/// nothing, and the rollback to the same bundle succeeds.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activation_through_the_route_reads_the_tenants_readiness() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    for (package, reason) in [
        (&fixture.a, "activate_bundle_a"),
        (&fixture.b, "activate_bundle_b"),
    ] {
        let (status, routing) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(fixture.activate_body(package, reason).await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
    }
    let (status, receipt, submission_id) = fixture.upload(&fixture.state, "waiting").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["status"], "processing");
    let mut client = fixture.owner.trace_pool_for_test().get().await.unwrap();
    let tx = tenant_tx(&mut client, tenant).await;
    assert_eq!(
        tx.execute(
            "UPDATE pipeline_runs SET phase_started_at = NOW() - INTERVAL '1 hour'
              WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission_id],
        )
        .await
        .expect("age the waiting run"),
        1
    );
    tx.commit().await.unwrap();
    drop(client);

    let before = fixture.store().routing(tenant).await.unwrap();
    let events_before = fixture.store().events(tenant, 10).await.unwrap();
    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(fixture.activate_body(&fixture.a, "activate_bundle_a").await),
        )
        .await;
    assert_eq!(
        (status, refused),
        (
            StatusCode::CONFLICT,
            serde_json::json!({
                "error": trace_commons_server::versioned_pipeline_activation::ACTIVATION_READINESS_FAILED_LABEL
            })
        )
    );
    assert_eq!(fixture.store().routing(tenant).await.unwrap(), before);
    assert_eq!(
        fixture.store().events(tenant, 10).await.unwrap(),
        events_before
    );

    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/rollback",
            Some(fixture.activate_body(&fixture.a, "roll_back_to_a").await),
        )
        .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "a rollback reads no readiness: {routing}"
    );
    assert_eq!(routing["routing_state"], "pipeline");
    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["active_bundle_id"], fixture.a.bundle_id);
}

/// Fix round 1: each of the three read routes records one control-plane read
/// audit, as its siblings `operational-summary` and `forensic` do, under its
/// own surface and with the number of items it answered (never an id); a
/// refused read records none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_read_routes_record_a_control_plane_read() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    fixture.qualify(&fixture.a).await;
    let (status, record) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/policy-interventions",
            Some(serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "phase": "score",
                "action": "suspend",
                "reason_code": "suspend_score_for_audit",
            })),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{record}");
    let (status, _) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/contain",
            Some(serde_json::json!({ "reason_code": "contain_for_audit" })),
        )
        .await;
    assert_eq!(status, StatusCode::OK);
    for surface in [
        "pipeline_routing",
        "pipeline_policy_interventions",
        "pipeline_legacy_drain",
    ] {
        assert!(
            fixture.read_audits(tenant, surface).is_empty(),
            "{surface}: no read yet; the actions record none"
        );
    }

    // Refused reads record nothing.
    let (status, _) = fixture
        .admin_call("GET", "/v1/admin/pipeline/legacy-drain", None)
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "the drain precondition");
    let (status, _) = fixture
        .admin_call(
            "GET",
            "/v1/admin/pipeline/policy-interventions?bundle_id=not-a-bundle",
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    for surface in ["pipeline_policy_interventions", "pipeline_legacy_drain"] {
        assert!(
            fixture.read_audits(tenant, surface).is_empty(),
            "{surface}: a refused read records nothing"
        );
    }

    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["events"].as_array().map(Vec::len), Some(1));
    let (status, listed) = fixture
        .admin_call(
            "GET",
            &format!(
                "/v1/admin/pipeline/policy-interventions?bundle_id={}",
                fixture.a.bundle_id
            ),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{listed}");
    let (status, report) = fixture
        .call(
            &fixture.with(|state| state.db_reviewer_reads = true),
            "GET",
            "/v1/admin/pipeline/legacy-drain",
            Some(&fixture.admin),
            None,
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{report}");
    for (surface, reason) in [
        ("pipeline_routing", "surface=pipeline_routing;item_count=1"),
        (
            "pipeline_policy_interventions",
            "surface=pipeline_policy_interventions;item_count=1",
        ),
        (
            "pipeline_legacy_drain",
            "surface=pipeline_legacy_drain;item_count=1",
        ),
    ] {
        assert_eq!(
            fixture.read_audits(tenant, surface),
            vec![reason.to_string()],
            "{surface}: one read, one audit row"
        );
    }
    assert!(
        fixture
            .read_audits(&fixture.other_tenant, "pipeline_routing")
            .is_empty(),
        "the bystander's audit log has no row of the tenant's read"
    );
}

/// Fix round 1: a bundle id that is not `sha256:` and 64 lowercase hex digits
/// is `422 pipeline_request_invalid` before any store call: in an activation,
/// a rollback, and the intervention list. A NUL byte in the id, which
/// PostgreSQL refuses as text, never reaches it. Nothing is read or written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_malformed_bundle_id_is_refused_before_any_store_call() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    let attestations = fixture.attestations(&fixture.a);
    let expected = fixture.record_id_in_force(tenant).await;
    let malformed = [
        ("a word", "not-a-bundle".to_string()),
        ("upper-case hex", format!("sha256:{}", "A".repeat(64))),
        ("63 digits", format!("sha256:{}", "0".repeat(63))),
        ("a NUL byte", format!("sha256:\u{0}{}", "0".repeat(63))),
    ];
    for (what, bundle_id) in &malformed {
        for path in ["/v1/admin/pipeline/activate", "/v1/admin/pipeline/rollback"] {
            let (status, refused) = fixture
                .admin_call(
                    "POST",
                    path,
                    Some(serde_json::json!({
                        "bundle_id": bundle_id,
                        "reason_code": "malformed_bundle_id",
                        "attestations": attestations,
                        "expected_record_id": expected,
                    })),
                )
                .await;
            assert_eq!(
                (status, refused),
                (
                    StatusCode::UNPROCESSABLE_ENTITY,
                    serde_json::json!({ "error": "pipeline_request_invalid" })
                ),
                "{path}: {what}"
            );
        }
    }
    for (what, query) in [
        ("a word", "not-a-bundle".to_string()),
        ("a NUL byte", format!("sha256%3A%00{}", "0".repeat(63))),
    ] {
        let (status, refused) = fixture
            .admin_call(
                "GET",
                &format!("/v1/admin/pipeline/policy-interventions?bundle_id={query}"),
                None,
            )
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                serde_json::json!({ "error": "pipeline_request_invalid" })
            ),
            "the intervention list: {what}"
        );
    }
    fixture
        .assert_untouched(tenant, "malformed bundle ids")
        .await;
    assert!(
        fixture
            .read_audits(tenant, "pipeline_policy_interventions")
            .is_empty(),
        "no list was read"
    );
}

/// Fix round 1: the qualification verifies the signed package against the
/// package trust store before any work on the package. A package whose
/// signature is broken is refused with the verification's label even when
/// it names dependencies this runtime does not hold; the same package
/// correctly signed reaches the dependency profile, which refuses it
/// (`bundle_dependency_missing`), so the label shows which ran first.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_qualification_verifies_the_package_before_it_reads_it() {
    use trace_commons_server::versioned_pipeline_qualification::PACKAGE_SIGNATURE_INVALID_LABEL;

    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let unknown_dependencies = MinimalPolicyBundle::compatibility_package(
        &route_config(ROUTE_INDEX_A),
        &trace_commons_gate_api::ReferencePerplexityScorer::new(),
        &trace_commons_gate_api::ReferenceEmbedder::new(),
    )
    .expect("the package builds");
    let signed = fixture.signed(&unknown_dependencies);
    let mut broken = signed.clone();
    broken.signature.signature_base64url =
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([7u8; 64]);
    for (what, signed_package, label) in [
        (
            "a broken signature",
            broken,
            PACKAGE_SIGNATURE_INVALID_LABEL,
        ),
        (
            "a good signature",
            signed,
            trace_commons_server::versioned_pipeline_bundle::PIPELINE_DEPENDENCY_MISSING_LABEL,
        ),
    ] {
        let (status, refused) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/qualifications",
                Some(serde_json::json!({
                    "signed_package": signed_package,
                    "attestations": fixture.attestations(&unknown_dependencies),
                })),
            )
            .await;
        assert_eq!(
            (status, refused),
            (StatusCode::CONFLICT, serde_json::json!({ "error": label })),
            "{what}"
        );
    }
    fixture
        .assert_untouched(&fixture.tenant, "refused qualifications")
        .await;
}

/// Fix round 1: the nine routes take a body of at most 1 MiB
/// (`PIPELINE_ADMIN_BODY_MAX_BYTES`), their own limit inside the router-wide
/// one: a body of exactly 1 MiB reaches the handler, one byte more is `413`
/// `pipeline_request_too_large`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_routes_take_a_body_of_at_most_one_mebibyte() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    assert_eq!(
        pipeline_activation::PIPELINE_ADMIN_BODY_MAX_BYTES,
        1_048_576
    );
    let expected = fixture.record_id_in_force(&fixture.tenant).await;
    let body_of = |length: usize| {
        let empty = serde_json::json!({
            "bundle_id": fixture.a.bundle_id,
            "reason_code": "",
            "attestations": [],
            "expected_record_id": expected,
        });
        let padding = length - empty.to_string().len();
        let body = serde_json::json!({
            "bundle_id": fixture.a.bundle_id,
            "reason_code": "a".repeat(padding),
            "attestations": [],
            "expected_record_id": expected,
        });
        assert_eq!(body.to_string().len(), length);
        body
    };
    let (status, answer) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(body_of(pipeline_activation::PIPELINE_ADMIN_BODY_MAX_BYTES)),
        )
        .await;
    assert_ne!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "a body of exactly the limit reaches the handler: {answer}"
    );
    assert_eq!(
        (status, answer),
        (
            StatusCode::NOT_FOUND,
            serde_json::json!({ "error": "bundle_package_missing" })
        ),
        "the tenant has no package A yet"
    );
    let (status, refused) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(body_of(
                pipeline_activation::PIPELINE_ADMIN_BODY_MAX_BYTES + 1,
            )),
        )
        .await;
    assert_eq!(
        (status, refused),
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            serde_json::json!({ "error": "pipeline_request_too_large" })
        )
    );
    fixture
        .assert_untouched(&fixture.tenant, "the two large bodies")
        .await;
}

/// `package` as the tenant stored it, rewritten by `edit` through an owner
/// connection with the immutability trigger off for the one update (a
/// tampered package, BND-002), as the runtime suite's
/// `rewrite_stored_bundle_package` does.
async fn tamper_stored_package(
    owner: &PgBackend,
    tenant: &str,
    bundle_id: &str,
    edit: impl FnOnce(&mut serde_json::Value),
) {
    let mut client = owner.trace_pool_for_test().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute(
        "ALTER TABLE pipeline_bundle_packages
             DISABLE TRIGGER pipeline_bundle_packages_reject_update",
    )
    .await
    .expect("disable the immutability trigger");
    let mut package: serde_json::Value = tx
        .query_one(
            "SELECT package FROM pipeline_bundle_packages
              WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant, &bundle_id],
        )
        .await
        .expect("the stored package")
        .get(0);
    edit(&mut package);
    assert_eq!(
        tx.execute(
            "UPDATE pipeline_bundle_packages SET package = $3
              WHERE tenant_id = $1 AND bundle_id = $2",
            &[&tenant, &bundle_id, &package],
        )
        .await
        .expect("tamper the stored package"),
        1
    );
    tx.batch_execute(
        "ALTER TABLE pipeline_bundle_packages
             ENABLE TRIGGER pipeline_bundle_packages_reject_update",
    )
    .await
    .expect("enable the immutability trigger");
    tx.commit().await.unwrap();
}

/// Final fix wave (K1, K2): every route that verifies attestations refuses a
/// set whose maximum age is above the server's ceiling (seven days) with
/// `bundle_qualification_evidence_age_above_ceiling`: the qualification, the
/// activation, and the rollback. An age that chrono cannot represent is
/// above it too and gets the same refusal, never `500`. Nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_evidence_age_above_the_ceiling_is_refused_at_every_route() {
    use trace_commons_server::versioned_pipeline_qualification::{
        QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL, QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS,
    };

    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    for maximum_age_seconds in [u64::MAX, QUALIFICATION_EVIDENCE_AGE_CEILING_SECONDS + 1] {
        let attestations = fixture.attestations_aged(&fixture.a, maximum_age_seconds, None);
        let expected = fixture.record_id_in_force(&fixture.tenant).await;
        for (path, body) in [
            (
                "/v1/admin/pipeline/qualifications",
                serde_json::json!({
                    "signed_package": fixture.signed(&fixture.a),
                    "attestations": attestations,
                }),
            ),
            (
                "/v1/admin/pipeline/activate",
                serde_json::json!({
                    "bundle_id": fixture.a.bundle_id,
                    "reason_code": "activate_bundle_a",
                    "attestations": attestations,
                    "expected_record_id": expected,
                }),
            ),
            (
                "/v1/admin/pipeline/rollback",
                serde_json::json!({
                    "bundle_id": fixture.a.bundle_id,
                    "reason_code": "roll_back_to_a",
                    "attestations": attestations,
                    "expected_record_id": expected,
                }),
            ),
        ] {
            let (status, refused) = fixture.admin_call("POST", path, Some(body)).await;
            assert_eq!(
                (status, refused),
                (
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": QUALIFICATION_EVIDENCE_AGE_ABOVE_CEILING_LABEL })
                ),
                "{path} with a maximum age of {maximum_age_seconds} s"
            );
        }
    }
    assert_eq!(
        fixture.store().routing(&fixture.tenant).await.unwrap(),
        None
    );
    assert!(
        fixture
            .store()
            .events(&fixture.tenant, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Final fix wave (K3): a promotion that is not ready is answered with the
/// route's own `409` label and, in `blockers`, the decision's blockers
/// (labels only). One stale result names its check, at the qualification,
/// the activation, and the rollback. The routing does not change.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refused_promotion_answers_its_blockers() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    for (package, reason) in [
        (&fixture.a, "activate_bundle_a"),
        (&fixture.b, "activate_bundle_b"),
    ] {
        let (status, routing) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(fixture.activate_body(package, reason).await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
    }
    let events_before = fixture.store().events(&fixture.tenant, 10).await.unwrap();
    let stale = fixture.attestations_aged(&fixture.a, 3_600, Some("pipeline_crash_matrix"));
    let blockers = serde_json::json!(["qualification_evidence_stale:pipeline_crash_matrix"]);
    let expected = fixture.record_id_in_force(&fixture.tenant).await;
    for (path, body, label) in [
        (
            "/v1/admin/pipeline/qualifications",
            serde_json::json!({
                "signed_package": fixture.signed(&fixture.a),
                "attestations": stale,
            }),
            "bundle_qualification_promotion_not_ready",
        ),
        (
            "/v1/admin/pipeline/activate",
            serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "reason_code": "activate_bundle_a",
                "attestations": stale,
                "expected_record_id": expected,
            }),
            "bundle_activation_promotion_not_ready",
        ),
        (
            "/v1/admin/pipeline/rollback",
            serde_json::json!({
                "bundle_id": fixture.a.bundle_id,
                "reason_code": "roll_back_to_a",
                "attestations": stale,
                "expected_record_id": expected,
            }),
            "bundle_activation_promotion_not_ready",
        ),
    ] {
        let (status, refused) = fixture.admin_call("POST", path, Some(body)).await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": label, "blockers": blockers })
            ),
            "{path}"
        );
    }
    assert_eq!(
        fixture.store().events(&fixture.tenant, 10).await.unwrap(),
        events_before
    );
    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["active_bundle_id"], fixture.b.bundle_id);
}

/// Final fix wave (K4): a stored package that no longer validates (a
/// tampered package, BND-002) is refused at the activation and the rollback
/// with the gate's label for it, `409 bundle_package_missing`, at the first
/// read of the package, never `500`. Nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_altered_stored_package_is_refused_as_missing() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    tamper_stored_package(
        &fixture.owner,
        &fixture.tenant,
        &fixture.a.bundle_id,
        |package| package["manifest"]["format_version"] = serde_json::Value::from(1),
    )
    .await;
    for (path, reason) in [
        ("/v1/admin/pipeline/activate", "activate_bundle_a"),
        ("/v1/admin/pipeline/rollback", "roll_back_to_a"),
    ] {
        let (status, refused) = fixture
            .admin_call(
                "POST",
                path,
                Some(fixture.activate_body(&fixture.a, reason).await),
            )
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": "bundle_package_missing" })
            ),
            "{path}"
        );
    }
    assert_eq!(
        fixture.store().routing(&fixture.tenant).await.unwrap(),
        None
    );
    assert!(
        fixture
            .store()
            .events(&fixture.tenant, 10)
            .await
            .unwrap()
            .is_empty()
    );
}

/// Final fix wave (G8): `activate` and `rollback` refuse a tenant that this
/// process does not list on its receipts list, with `409
/// pipeline_tenant_not_in_scope`, before the store call: the activated
/// tenant's every new upload would otherwise be `503
/// pipeline_tenant_not_served` here. A tenant on the drain list only is out
/// of scope too. Nothing is written.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activation_or_rollback_out_of_scope_is_refused() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    for (package, reason) in [
        (&fixture.a, "activate_bundle_a"),
        (&fixture.b, "activate_bundle_b"),
    ] {
        let (status, routing) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(fixture.activate_body(package, reason).await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
    }
    let before = fixture.store().routing(tenant).await.unwrap();
    let events_before = fixture.store().events(tenant, 10).await.unwrap();
    let only_the_bystander = || {
        TraceTenantRolloutGates::for_feature(
            TraceTenantRolloutFeature::PipelineReceipts,
            &[fixture.other_tenant.as_str()],
        )
    };
    for (what, state) in [
        (
            "not listed",
            fixture.with(|state| state.tenant_rollout_gates = only_the_bystander()),
        ),
        (
            "on the drain list only",
            fixture.with(|state| {
                state.tenant_rollout_gates = only_the_bystander();
                state.pipeline_drain_tenant_ids = Arc::new(BTreeSet::from([tenant.to_string()]));
            }),
        ),
    ] {
        for (path, body) in [
            (
                "/v1/admin/pipeline/activate",
                fixture.activate_body(&fixture.a, "activate_bundle_a").await,
            ),
            (
                "/v1/admin/pipeline/rollback",
                fixture.activate_body(&fixture.a, "roll_back_to_a").await,
            ),
        ] {
            let (status, refused) = fixture
                .call(&state, "POST", path, Some(&fixture.admin), Some(body))
                .await;
            assert_eq!(
                (status, refused),
                (
                    StatusCode::CONFLICT,
                    serde_json::json!({ "error": "pipeline_tenant_not_in_scope" })
                ),
                "{what}: {path}"
            );
        }
    }
    assert_eq!(fixture.store().routing(tenant).await.unwrap(), before);
    assert_eq!(
        fixture.store().events(tenant, 10).await.unwrap(),
        events_before
    );
}

/// Final fix wave (G11): the activation and the rollback check the package
/// trust store this process holds, as the qualification does: a bundle
/// whose qualification records a signing key that the store no longer holds
/// (the key was removed after the qualification) is refused with the
/// package verification's label, `409 bundle_package_signer_untrusted`, and
/// nothing is written. With the key back, the rollback goes through.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_package_key_no_longer_trusted_stops_activation_and_rollback() {
    use trace_commons_server::versioned_pipeline_qualification::{
        PACKAGE_SIGNER_UNTRUSTED_LABEL, trusted_key_for_pkcs8,
    };

    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    let tenant = fixture.tenant.as_str();
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    for (package, reason) in [
        (&fixture.a, "activate_bundle_a"),
        (&fixture.b, "activate_bundle_b"),
    ] {
        let (status, routing) = fixture
            .admin_call(
                "POST",
                "/v1/admin/pipeline/activate",
                Some(fixture.activate_body(package, reason).await),
            )
            .await;
        assert_eq!(status, StatusCode::OK, "{routing}");
    }
    let before = fixture.store().routing(tenant).await.unwrap();
    let events_before = fixture.store().events(tenant, 10).await.unwrap();
    let other_pkcs8 =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .expect("a key pair");
    let key_removed = fixture.with(|state| {
        state.pipeline_package_trust = Some(Arc::new(
            BundlePackageTrustStore::new([trusted_key_for_pkcs8(
                "another-package-release-key",
                other_pkcs8.as_ref(),
            )
            .expect("the key")])
            .expect("the package trust store"),
        ))
    });
    for (path, body) in [
        (
            "/v1/admin/pipeline/activate",
            fixture.activate_body(&fixture.a, "activate_bundle_a").await,
        ),
        (
            "/v1/admin/pipeline/rollback",
            fixture.activate_body(&fixture.a, "roll_back_to_a").await,
        ),
    ] {
        let (status, refused) = fixture
            .call(&key_removed, "POST", path, Some(&fixture.admin), Some(body))
            .await;
        assert_eq!(
            (status, refused),
            (
                StatusCode::CONFLICT,
                serde_json::json!({ "error": PACKAGE_SIGNER_UNTRUSTED_LABEL })
            ),
            "{path}"
        );
    }
    assert_eq!(fixture.store().routing(tenant).await.unwrap(), before);
    assert_eq!(
        fixture.store().events(tenant, 10).await.unwrap(),
        events_before
    );
    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/rollback",
            Some(fixture.activate_body(&fixture.a, "roll_back_to_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{routing}");
}

/// Final fix wave (A22, the owner's answer to question 8): a rollback of a
/// contained tenant keeps it contained. The active bundle is the one rolled
/// back to, a new upload is still `503 pipeline_receipt_intake_contained`, and
/// the event is a `rollback` from `contained` to `contained`. Uploads reopen
/// only through `activate` of that bundle, which reads the tenant's
/// readiness.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rollback_of_a_contained_tenant_keeps_it_contained() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    // Each body is built just before its request: it names the record id
    // that the change before it left in force.
    for (path, package, reason) in [
        (
            "/v1/admin/pipeline/activate",
            Some(&fixture.a),
            "activate_bundle_a",
        ),
        (
            "/v1/admin/pipeline/activate",
            Some(&fixture.b),
            "activate_bundle_b",
        ),
        ("/v1/admin/pipeline/contain", None, "contain_for_incident"),
    ] {
        let body = match package {
            Some(package) => fixture.activate_body(package, reason).await,
            None => serde_json::json!({ "reason_code": reason }),
        };
        let (status, answer) = fixture.admin_call("POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {answer}");
    }
    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/rollback",
            Some(fixture.activate_body(&fixture.a, "roll_back_to_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{routing}");
    assert_eq!(routing["routing_state"], "contained");
    let (status, refused, _) = fixture
        .upload(&fixture.state, "rolled_back_contained")
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert_eq!(refused["error"], "pipeline_receipt_intake_contained");
    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["routing_state"], "contained");
    assert_eq!(view["active_bundle_id"], fixture.a.bundle_id);
    assert_eq!(view["events"][0]["action"], "rollback");
    assert_eq!(view["events"][0]["previous_state"], "contained");
    assert_eq!(view["events"][0]["resulting_state"], "contained");

    let (status, routing) = fixture
        .admin_call(
            "POST",
            "/v1/admin/pipeline/activate",
            Some(fixture.activate_body(&fixture.a, "reopen_bundle_a").await),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{routing}");
    assert_eq!(routing["routing_state"], "pipeline");
    let (status, receipt, _) = fixture.upload(&fixture.state, "reopened").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["status"], "processing");
}

// Review round 1, point 2 (amendment A6, owner answer O2): the expectation
// of a routing change. `activate` and `rollback` must name the record id
// that is in force (`expected_record_id`: the `activation_record_id` of the
// routing view, or `none` for a tenant with no routing row). `contain` keeps
// an optional expectation. `deactivate` needs one when the tenant is
// contained.

impl RouteFixture {
    /// `activate_body` with the caller's `expected_record_id`: a request that
    /// an operator prepared at an earlier time.
    fn activate_body_expecting(
        &self,
        package: &trace_commons_gate_api::pipeline::BundlePackage,
        reason_code: &str,
        expected_record_id: &serde_json::Value,
    ) -> serde_json::Value {
        serde_json::json!({
            "bundle_id": package.bundle_id,
            "reason_code": reason_code,
            "attestations": self.attestations(package),
            "expected_record_id": expected_record_id,
        })
    }

    /// `POST path` with `body` and the tenant's admin credential answers
    /// `200`; returns the routing row of the answer.
    async fn change_routing(&self, path: &str, body: serde_json::Value) -> serde_json::Value {
        let (status, routing) = self.admin_call("POST", path, Some(body)).await;
        assert_eq!(status, StatusCode::OK, "{path}: {routing}");
        assert_eq!(
            routing["activation_record_id"],
            self.record_id_in_force(&self.tenant).await,
            "{path}: the answer carries the record id that is now in force"
        );
        routing
    }

    /// `POST path` with `body` and the tenant's admin credential is refused
    /// with `status` and `label`, and changes nothing: the routing row, the
    /// active bundle, and the events are as they were.
    async fn assert_change_refused(
        &self,
        path: &str,
        body: serde_json::Value,
        status: StatusCode,
        label: &str,
        context: &str,
    ) {
        let before = self.store().routing_view(&self.tenant, 100).await.unwrap();
        let answer = self.admin_call("POST", path, Some(body)).await;
        assert_eq!(
            answer,
            (status, serde_json::json!({ "error": label })),
            "{context}: {path}"
        );
        assert_eq!(
            self.store().routing_view(&self.tenant, 100).await.unwrap(),
            before,
            "{context}: {path}: the routing row, the active bundle, and the events"
        );
    }
}

const ACTIVATE: &str = "/v1/admin/pipeline/activate";
const ROLLBACK: &str = "/v1/admin/pipeline/rollback";
const CONTAIN: &str = "/v1/admin/pipeline/contain";
const DEACTIVATE: &str = "/v1/admin/pipeline/deactivate";

/// An `activate` and a `rollback` whose body has no `expected_record_id` are
/// `422 pipeline_request_invalid` and write nothing: an absent field, an
/// explicit `null`, a value that is neither a UUID nor `none`, and a body
/// that sends the earlier `expected_state` (an unknown field on these two
/// routes, with or without the record id). For a tenant with no routing row,
/// and for a `pipeline` tenant whose rollback would otherwise pass.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activate_or_a_rollback_without_expected_record_id_is_refused() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    for activated in [false, true] {
        if activated {
            for (package, reason) in [
                (&fixture.a, "activate_bundle_a"),
                (&fixture.b, "activate_bundle_b"),
            ] {
                let body = fixture.activate_body(package, reason).await;
                fixture.change_routing(ACTIVATE, body).await;
            }
        }
        let in_force = fixture.record_id_in_force(&fixture.tenant).await;
        let state = if activated { "pipeline" } else { "none" };
        for path in [ACTIVATE, ROLLBACK] {
            let complete = fixture.activate_body_expecting(&fixture.a, "no_expectation", &in_force);
            let edits: [(&str, Option<serde_json::Value>, Option<&str>); 7] = [
                ("no expected_record_id", None, None),
                ("a null", Some(serde_json::Value::Null), None),
                ("a word", Some(serde_json::json!("paused")), None),
                ("a state", Some(serde_json::json!(state)), None),
                ("a number", Some(serde_json::json!(17)), None),
                ("expected_state in its place", None, Some(state)),
                (
                    "expected_state beside the record id",
                    Some(in_force.clone()),
                    Some(state),
                ),
            ];
            for (what, record_id, expected_state) in edits {
                // `none` is the record id of a tenant with no row, so "a
                // state" is a malformed value only for a tenant that has one.
                if what == "a state" && !activated {
                    continue;
                }
                let mut body = complete.clone();
                let fields = body.as_object_mut().expect("an object");
                fields.remove("expected_record_id");
                if let Some(record_id) = record_id {
                    fields.insert("expected_record_id".to_string(), record_id);
                }
                if let Some(expected_state) = expected_state {
                    fields.insert(
                        "expected_state".to_string(),
                        serde_json::json!(expected_state),
                    );
                }
                fixture
                    .assert_change_refused(
                        path,
                        body,
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "pipeline_request_invalid",
                        &format!("{what}, routing {state}"),
                    )
                    .await;
            }
        }
    }
}

/// The reviewer's sequence: an operator prepares an `activate` while the
/// tenant is `pipeline`, an incident's `contain` commits, and the prepared
/// request arrives. It names the record id that the containment replaced, so
/// it is `409 pipeline_routing_state_changed`: the tenant stays contained, no
/// event is written, and a new upload is still refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activate_prepared_before_a_contain_does_not_reopen_intake() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let body = fixture.activate_body(&fixture.a, "activate_bundle_a").await;
    let activated = fixture.change_routing(ACTIVATE, body).await;
    let prepared = fixture.activate_body_expecting(
        &fixture.a,
        "activate_prepared_earlier",
        &activated["activation_record_id"],
    );
    let contained = fixture
        .change_routing(
            CONTAIN,
            serde_json::json!({ "reason_code": "contain_for_incident" }),
        )
        .await;
    assert_eq!(contained["routing_state"], "contained");

    fixture
        .assert_change_refused(
            ACTIVATE,
            prepared,
            StatusCode::CONFLICT,
            "pipeline_routing_state_changed",
            "an activate prepared before the containment",
        )
        .await;
    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["routing_state"], "contained");
    assert_eq!(
        view["activation_record_id"],
        contained["activation_record_id"]
    );
    assert_eq!(view["events"][0]["action"], "contain");
    let (status, refused, _) = fixture.upload(&fixture.state, "still_contained").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert_eq!(refused["error"], "pipeline_receipt_intake_contained");
}

/// The sequence that a comparison of the state lets through (plan review
/// G12): contain, activate, contain, and then an `activate` that was prepared
/// during the FIRST containment. The state is `contained` both times; the
/// record id is another one, so the prepared request is `409
/// pipeline_routing_state_changed` and the tenant stays contained. An
/// `activate` that names the id in force reopens intake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_activate_that_names_an_earlier_containment_is_refused() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let contain = serde_json::json!({ "reason_code": "contain_for_incident" });
    let first = fixture.change_routing(CONTAIN, contain.clone()).await;
    let prepared = fixture.activate_body_expecting(
        &fixture.a,
        "activate_prepared_earlier",
        &first["activation_record_id"],
    );
    // Another operator reopens intake (the body names the first containment,
    // which is in force), and a second incident contains the tenant again.
    let body = fixture.activate_body(&fixture.a, "reopen_bundle_a").await;
    assert_eq!(body["expected_record_id"], first["activation_record_id"]);
    let reopened = fixture.change_routing(ACTIVATE, body).await;
    assert_eq!(reopened["routing_state"], "pipeline");
    let second = fixture.change_routing(CONTAIN, contain).await;
    assert_eq!(second["routing_state"], "contained");
    assert_ne!(
        second["activation_record_id"],
        first["activation_record_id"]
    );

    fixture
        .assert_change_refused(
            ACTIVATE,
            prepared,
            StatusCode::CONFLICT,
            "pipeline_routing_state_changed",
            "an activate that names the first containment",
        )
        .await;
    let (status, refused, _) = fixture.upload(&fixture.state, "contained_again").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert_eq!(refused["error"], "pipeline_receipt_intake_contained");

    // The id in force for the contained tenant: intake opens.
    let body = fixture.activate_body_expecting(
        &fixture.a,
        "reopen_bundle_a_again",
        &second["activation_record_id"],
    );
    let reopened = fixture.change_routing(ACTIVATE, body).await;
    assert_eq!(reopened["routing_state"], "pipeline");
    let (status, receipt, _) = fixture.upload(&fixture.state, "reopened_again").await;
    assert_eq!(status, StatusCode::OK, "{receipt}");
    assert_eq!(receipt["status"], "processing");
}

/// A first activation of a tenant with no routing row names `none`: a record
/// id is refused (`409 pipeline_routing_state_changed`) and writes nothing,
/// `none` activates, and `none` is then refused, because the tenant has a
/// row. The routing view of a tenant with no row shows a null record id.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_first_activation_names_none() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let (status, view) = fixture
        .admin_call("GET", "/v1/admin/pipeline/routing", None)
        .await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["routing_state"], serde_json::Value::Null);
    assert_eq!(view["activation_record_id"], serde_json::Value::Null);
    assert_eq!(
        fixture.record_id_in_force(&fixture.tenant).await,
        serde_json::json!("none")
    );

    let some_record = serde_json::json!(Uuid::new_v4());
    fixture
        .assert_change_refused(
            ACTIVATE,
            fixture.activate_body_expecting(&fixture.a, "activate_bundle_a", &some_record),
            StatusCode::CONFLICT,
            "pipeline_routing_state_changed",
            "a record id for a tenant with no row",
        )
        .await;
    assert_eq!(
        fixture.store().routing(&fixture.tenant).await.unwrap(),
        None
    );
    let none = serde_json::json!("none");
    let activated = fixture
        .change_routing(
            ACTIVATE,
            fixture.activate_body_expecting(&fixture.a, "activate_bundle_a", &none),
        )
        .await;
    assert_eq!(activated["routing_state"], "pipeline");
    fixture
        .assert_change_refused(
            ACTIVATE,
            fixture.activate_body_expecting(&fixture.a, "activate_bundle_a", &none),
            StatusCode::CONFLICT,
            "pipeline_routing_state_changed",
            "none for a tenant that has a row",
        )
        .await;
}

/// A `rollback` prepared before another operator's rollback is refused. The
/// tenant is `pipeline` with B active when the first operator prepares a
/// rollback to A. A second operator rolls back to A and activates B again:
/// the state and the active bundle are what the first operator read, and the
/// record id is not. The prepared rollback is `409
/// pipeline_routing_state_changed`; one that names the id in force passes.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_rollback_prepared_before_another_operators_rollback_is_refused() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    fixture.qualify(&fixture.b).await;
    let mut in_force = serde_json::Value::Null;
    for (package, reason) in [
        (&fixture.a, "activate_bundle_a"),
        (&fixture.b, "activate_bundle_b"),
    ] {
        let body = fixture.activate_body(package, reason).await;
        in_force = fixture.change_routing(ACTIVATE, body).await["activation_record_id"].clone();
    }
    let prepared =
        fixture.activate_body_expecting(&fixture.a, "roll_back_prepared_earlier", &in_force);

    let body = fixture.activate_body(&fixture.a, "roll_back_to_a").await;
    let rolled_back = fixture.change_routing(ROLLBACK, body).await;
    assert_eq!(rolled_back["routing_state"], "pipeline");
    let body = fixture
        .activate_body(&fixture.b, "activate_bundle_b_again")
        .await;
    let again = fixture.change_routing(ACTIVATE, body).await;
    let view = fixture
        .store()
        .routing_view(&fixture.tenant, 10)
        .await
        .unwrap();
    assert_eq!(
        view.active_bundle_id.as_deref(),
        Some(fixture.b.bundle_id.as_str())
    );
    assert_eq!(
        view.routing.map(|routing| routing.routing_state),
        Some(RoutingState::Pipeline),
        "the state and the bundle that the first operator read"
    );

    fixture
        .assert_change_refused(
            ROLLBACK,
            prepared,
            StatusCode::CONFLICT,
            "pipeline_routing_state_changed",
            "a rollback prepared before another operator's rollback",
        )
        .await;
    let body = fixture.activate_body_expecting(
        &fixture.a,
        "roll_back_to_a_again",
        &again["activation_record_id"],
    );
    fixture.change_routing(ROLLBACK, body).await;
    assert_eq!(
        fixture
            .store()
            .routing_view(&fixture.tenant, 1)
            .await
            .unwrap()
            .active_bundle_id
            .as_deref(),
        Some(fixture.a.bundle_id.as_str())
    );
}

/// A `deactivate` of a contained tenant needs an expectation (plan review
/// G11): with none it is `409 pipeline_routing_expectation_required` and the
/// tenant stays contained, so a request prepared for a `pipeline` tenant
/// cannot reopen intake on the legacy path after an incident's `contain`.
/// With `expected_state = contained`, or with the record id in force, it
/// passes. An expectation that does not hold is `409
/// pipeline_routing_state_changed`. A `deactivate` of a `pipeline` tenant
/// needs none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_deactivate_of_a_contained_tenant_needs_an_expectation() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let contain = serde_json::json!({ "reason_code": "contain_for_incident" });
    let activated = {
        let body = fixture.activate_body(&fixture.a, "activate_bundle_a").await;
        fixture.change_routing(ACTIVATE, body).await
    };
    let contained = fixture.change_routing(CONTAIN, contain.clone()).await;

    fixture
        .assert_change_refused(
            DEACTIVATE,
            serde_json::json!({ "reason_code": "deactivate_to_legacy" }),
            StatusCode::CONFLICT,
            "pipeline_routing_expectation_required",
            "no expectation for a contained tenant",
        )
        .await;
    let (status, refused, _) = fixture.upload(&fixture.state, "not_deactivated").await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{refused}");
    assert_eq!(refused["error"], "pipeline_receipt_intake_contained");
    // Requests prepared while the tenant was `pipeline`.
    for (what, stale) in [
        (
            "the state before the containment",
            serde_json::json!({ "expected_state": "pipeline" }),
        ),
        (
            "the record id before the containment",
            serde_json::json!({ "expected_record_id": activated["activation_record_id"] }),
        ),
        (
            "the right state and an earlier record id",
            serde_json::json!({
                "expected_state": "contained",
                "expected_record_id": activated["activation_record_id"],
            }),
        ),
        (
            "the right record id and another state",
            serde_json::json!({
                "expected_state": "pipeline",
                "expected_record_id": contained["activation_record_id"],
            }),
        ),
    ] {
        let mut body = stale;
        body["reason_code"] = serde_json::json!("deactivate_prepared_earlier");
        fixture
            .assert_change_refused(
                DEACTIVATE,
                body,
                StatusCode::CONFLICT,
                "pipeline_routing_state_changed",
                what,
            )
            .await;
    }
    for unparsed in [serde_json::json!("paused"), serde_json::Value::Null] {
        for field in ["expected_state", "expected_record_id"] {
            let mut body = serde_json::json!({ "reason_code": "deactivate_to_legacy" });
            body[field] = unparsed.clone();
            fixture
                .assert_change_refused(
                    DEACTIVATE,
                    body,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "pipeline_request_invalid",
                    &format!("{field} = {unparsed}"),
                )
                .await;
        }
    }

    // Each form of the expectation passes from `contained`.
    let legacy = fixture
        .change_routing(
            DEACTIVATE,
            serde_json::json!({
                "reason_code": "deactivate_to_legacy",
                "expected_state": "contained",
            }),
        )
        .await;
    assert_eq!(legacy["routing_state"], "legacy");
    let contained = fixture.change_routing(CONTAIN, contain.clone()).await;
    let legacy = fixture
        .change_routing(
            DEACTIVATE,
            serde_json::json!({
                "reason_code": "deactivate_to_legacy",
                "expected_record_id": contained["activation_record_id"],
            }),
        )
        .await;
    assert_eq!(legacy["routing_state"], "legacy");
    let contained = fixture.change_routing(CONTAIN, contain).await;
    let legacy = fixture
        .change_routing(
            DEACTIVATE,
            serde_json::json!({
                "reason_code": "deactivate_to_legacy",
                "expected_state": "contained",
                "expected_record_id": contained["activation_record_id"],
            }),
        )
        .await;
    assert_eq!(legacy["routing_state"], "legacy");

    // From `pipeline` the expectation stays optional.
    let body = fixture.activate_body(&fixture.a, "activate_bundle_a").await;
    fixture.change_routing(ACTIVATE, body).await;
    let legacy = fixture
        .change_routing(
            DEACTIVATE,
            serde_json::json!({ "reason_code": "deactivate_to_legacy" }),
        )
        .await;
    assert_eq!(legacy["routing_state"], "legacy");
}

/// An emergency stop needs no read first: `contain` with no expectation
/// passes from every state (no row, `contained`, `pipeline`, `legacy`). The
/// expectation stays optional there: `expected_state`, `expected_record_id`,
/// or both, and when both are sent both must hold, else `409
/// pipeline_routing_state_changed`. A value that does not parse, a `null`
/// too, is `422 pipeline_request_invalid`.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_contain_with_no_expectation_passes_from_every_state() {
    let Some(fixture) = RouteFixture::new().await else {
        return;
    };
    fixture.qualify(&fixture.a).await;
    let contain = serde_json::json!({ "reason_code": "contain_for_incident" });
    // No row, then `contained`.
    for previous in [serde_json::Value::Null, serde_json::json!("contained")] {
        let contained = fixture.change_routing(CONTAIN, contain.clone()).await;
        assert_eq!(contained["routing_state"], "contained");
        let (_, view) = fixture
            .admin_call("GET", "/v1/admin/pipeline/routing", None)
            .await;
        assert_eq!(view["events"][0]["previous_state"], previous, "{view}");
    }
    // `pipeline`.
    let body = fixture.activate_body(&fixture.a, "activate_bundle_a").await;
    let activated = fixture.change_routing(ACTIVATE, body).await;
    assert_eq!(activated["routing_state"], "pipeline");
    let contained = fixture.change_routing(CONTAIN, contain.clone()).await;
    assert_eq!(contained["routing_state"], "contained");
    // `legacy`.
    let legacy = fixture
        .change_routing(
            DEACTIVATE,
            serde_json::json!({
                "reason_code": "deactivate_to_legacy",
                "expected_record_id": contained["activation_record_id"],
            }),
        )
        .await;
    assert_eq!(legacy["routing_state"], "legacy");
    let contained = fixture.change_routing(CONTAIN, contain.clone()).await;
    assert_eq!(contained["routing_state"], "contained");

    // The optional expectation of `contain`.
    let in_force = contained["activation_record_id"].clone();
    for (what, expectation) in [
        (
            "another state",
            serde_json::json!({ "expected_state": "pipeline" }),
        ),
        (
            "none for a tenant that has a row",
            serde_json::json!({ "expected_record_id": "none" }),
        ),
        (
            "an earlier record id",
            serde_json::json!({ "expected_record_id": legacy["activation_record_id"] }),
        ),
        (
            "the right state and an earlier record id",
            serde_json::json!({
                "expected_state": "contained",
                "expected_record_id": legacy["activation_record_id"],
            }),
        ),
        (
            "the right record id and another state",
            serde_json::json!({ "expected_state": "legacy", "expected_record_id": in_force }),
        ),
    ] {
        let mut body = expectation;
        body["reason_code"] = serde_json::json!("contain_prepared_earlier");
        fixture
            .assert_change_refused(
                CONTAIN,
                body,
                StatusCode::CONFLICT,
                "pipeline_routing_state_changed",
                what,
            )
            .await;
    }
    for unparsed in [serde_json::json!("paused"), serde_json::Value::Null] {
        for field in ["expected_state", "expected_record_id"] {
            let mut body = contain.clone();
            body[field] = unparsed.clone();
            fixture
                .assert_change_refused(
                    CONTAIN,
                    body,
                    StatusCode::UNPROCESSABLE_ENTITY,
                    "pipeline_request_invalid",
                    &format!("{field} = {unparsed}"),
                )
                .await;
        }
    }
    let contained = fixture
        .change_routing(
            CONTAIN,
            serde_json::json!({
                "reason_code": "contain_with_both",
                "expected_state": "contained",
                "expected_record_id": in_force,
            }),
        )
        .await;
    assert_eq!(contained["routing_state"], "contained");
    assert_ne!(contained["activation_record_id"], in_force);
}
