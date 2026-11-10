// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Fixtures shared by this crate's unit tests and by test packages outside
//! it.
//!
//! Compiled under `cfg(test)`, and under the `test-support` feature, which
//! is how a test package that depends on this crate reaches them
//! (`trace_commons_ingest::test_support`). The feature is for tests only:
//! `scripts/ci/check-release-test-support.sh` refuses any shipped build
//! that enables it. With it on, the service's test-only levers exist in the
//! library too (the NEAR AI introspection redirect, the NEAR access-key
//! checker override, the submit rate limiter's reset and limit hooks), which
//! is why that check exists.
//!
//! This module is a child of the service module, so it can build an
//! `AppState` and call the service's private functions. What a test outside
//! the crate can name is exactly what this file declares `pub` or
//! re-exports; nothing else of the service is opened.

use super::*;

// The service items a test outside the crate may name. Each is declared
// `pub` in the service module, which is itself crate-private, so this list
// is the whole of what the `test-support` feature opens.
pub use super::{
    ACCOUNT_RATE_LIMITER, AppState, SUBMIT_PER_PRINCIPAL_CONCURRENCY, app, authenticate_ctx,
    configure_submit_rate_limits_for_test, reset_account_rate_limiter_for_test,
    submit_principal_rate_limit_key, submit_rate_limit_count_for_test,
};

use trace_commons_protocol::llm::recording::{TraceFile, TraceResponse, TraceStep};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RecordedTraceContributionOptions, TraceRedactor,
};

/// The state nearly every test starts from: local storage under `root`, the
/// fixture tokens of `tenant-a` and `tenant-b`, and nothing else configured.
pub fn test_state(root: PathBuf) -> Arc<AppState> {
    test_state_with_options(root, None, None, false, false, false, false)
}

pub(super) fn test_state_with_options(
    root: PathBuf,
    db_mirror: Option<Arc<dyn Database>>,
    artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_replay_export_reads: bool,
    db_audit_reads: bool,
) -> Arc<AppState> {
    test_state_with_options_and_policies(
        root,
        db_mirror,
        artifact_store,
        db_contributor_reads,
        db_reviewer_reads,
        db_replay_export_reads,
        db_audit_reads,
        BTreeMap::new(),
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn test_state_with_options_and_policies(
    root: PathBuf,
    db_mirror: Option<Arc<dyn Database>>,
    artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_replay_export_reads: bool,
    db_audit_reads: bool,
    tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
) -> Arc<AppState> {
    test_state_with_options_policies_and_export_guardrails(
        root,
        db_mirror,
        artifact_store,
        db_contributor_reads,
        db_reviewer_reads,
        db_replay_export_reads,
        false,
        db_audit_reads,
        tenant_policies,
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn test_state_with_options_policies_and_export_guardrails(
    root: PathBuf,
    db_mirror: Option<Arc<dyn Database>>,
    artifact_store: Option<Arc<LocalEncryptedTraceArtifactStore>>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_replay_export_reads: bool,
    db_replay_export_require_object_refs: bool,
    db_audit_reads: bool,
    tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
    require_tenant_submission_policy: bool,
    require_export_guardrails: bool,
) -> Arc<AppState> {
    test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
        root,
        db_mirror,
        artifact_store.map(ConfiguredTraceArtifactStore::legacy),
        db_contributor_reads,
        db_reviewer_reads,
        db_replay_export_reads,
        db_replay_export_require_object_refs,
        db_audit_reads,
        false,
        tenant_policies,
        require_tenant_submission_policy,
        require_export_guardrails,
        false,
        false,
    )
}

#[allow(clippy::too_many_arguments)]
pub(super) fn test_state_with_configured_artifact_store_policies_export_guardrails_and_required_db_writes(
    root: PathBuf,
    db_mirror: Option<Arc<dyn Database>>,
    artifact_store: Option<ConfiguredTraceArtifactStore>,
    db_contributor_reads: bool,
    db_reviewer_reads: bool,
    db_replay_export_reads: bool,
    db_replay_export_require_object_refs: bool,
    db_audit_reads: bool,
    db_tenant_policy_reads: bool,
    tenant_policies: BTreeMap<String, TenantSubmissionPolicy>,
    require_tenant_submission_policy: bool,
    require_export_guardrails: bool,
    require_db_mirror_writes: bool,
    require_derived_export_object_refs: bool,
) -> Arc<AppState> {
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, "tenant-a", "token-a", TokenRole::Contributor);
    insert_token(&mut tokens, "tenant-a", "token-a-2", TokenRole::Contributor);
    insert_token(&mut tokens, "tenant-a", "token-a-3", TokenRole::Contributor);
    insert_token(
        &mut tokens,
        "tenant-a",
        "review-token-a",
        TokenRole::Reviewer,
    );
    insert_token(&mut tokens, "tenant-a", "admin-token-a", TokenRole::Admin);
    // A second admin tenant, used only by the settlement in-process-lock
    // test. That test must HOLD the settlement lock while calling the
    // handler, and the lock is process-global and keyed by tenant -- so
    // holding tenant-a's lock fails every other settlement test that runs in
    // parallel with it. Giving it its own tenant keeps the contention real
    // and local.
    insert_token(
        &mut tokens,
        "tenant-lock",
        "admin-token-lock",
        TokenRole::Admin,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "export-worker-token-a",
        TokenRole::ExportWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "retention-worker-token-a",
        TokenRole::RetentionWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "vector-worker-token-a",
        TokenRole::VectorWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "benchmark-worker-token-a",
        TokenRole::BenchmarkWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "utility-worker-token-a",
        TokenRole::UtilityWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "process-eval-worker-token-a",
        TokenRole::ProcessEvalWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "revocation-worker-token-a",
        TokenRole::RevocationWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "competition-read-worker-token-a",
        TokenRole::CompetitionReadWorker,
    );
    insert_token(
        &mut tokens,
        "tenant-a",
        "account-trust-worker-token-a",
        TokenRole::AccountTrustWorker,
    );
    insert_token(&mut tokens, "tenant-b", "token-b", TokenRole::Contributor);
    insert_token(
        &mut tokens,
        "tenant-b",
        "review-token-b",
        TokenRole::Reviewer,
    );
    insert_token(&mut tokens, "tenant-b", "admin-token-b", TokenRole::Admin);
    configure_unbounded_submit_limits_for_test(&tokens);
    Arc::new(AppState {
        inference_connection_catalog: Arc::new(Vec::new()),
        activity_missions_policy: None,
        credit_estimate_table: Arc::new(
            trace_commons_protocol::local_credit_estimate::LocalEstimateTable::built_in(),
        ),
        credit_settlement_table: None,
        payout_rails: Arc::new(trace_commons_server::payout_worker::PayoutRails::disabled()),
        payout_store: None,
        payout_control_token_hash: None,
        referral_store: None,
        referral_config: Default::default(),
        referral_network_mark_key: None,
        first_run_estimate: Default::default(),
        witness_accepted_list_path: None,
        witness_accepted_list_directory: None,
        witness_configuration_path: None,
        first_run_copy: Default::default(),
        first_run_stats: None,
        witness_blocked: None,
        root,
        near_provisioning_enabled: false,
        near_account_identity: None,
        near_ai_signup_policy_template: None,
        near_provisioning_public_origin: None,
        near_provisioning_admission_ready: false,
        near_attestation_client: None,
        near_attestation_key_report_client: None,
        near_attestation_verification_clock: None,
        near_ai_introspection_base_url: None,
        driver_liveness: Arc::new(
            trace_commons_server::driver_liveness::DriverLivenessRegistry::default(),
        ),
        tokens: Arc::new(tokens),
        signed_token_verifier: None,
        managed_eddsa_keyset_refresh: None,
        require_eddsa_signed_tokens: false,
        require_managed_eddsa_signed_tokens: false,
        require_tenant_access_grants: false,
        tenant_policies: Arc::new(tenant_policies),
        require_tenant_submission_policy,
        db_mirror,
        pipeline_service: None,
        pipeline_product: None,
        pipeline_store: None,
        pipeline_activation: None,
        pipeline_qualification: None,
        pipeline_package_trust: None,
        pipeline_check_trust: None,
        pipeline_code_revision_hash: None,
        pipeline_main_gate: TEST_MAIN_GATE,
        pipeline_infrastructure_override: None,
        // The test assemblers build their services with this flag set
        // (`unqualified_routing_allowed`); a test of production routing
        // builds its state and its service with it off.
        pipeline_unqualified_routing: true,
        pipeline_runtime_selection: PipelineRuntimeSelection::None,
        pipeline_runtime_required: false,
        pipeline_worker_ready: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        pipeline_drain_tenant_ids: Arc::new(BTreeSet::new()),
        pipeline_index_rebuilds: Arc::default(),
        pipeline_default_routing: None,
        pipeline_worker_pass_stats: Arc::default(),
        pipeline_worker_concurrency: pipeline_runtime::PIPELINE_WORKER_DEFAULT_CONCURRENCY,
        pipeline_worker_wake: Arc::default(),
        db_contributor_reads,
        db_reviewer_reads,
        db_reviewer_require_object_refs: false,
        db_replay_export_reads,
        db_replay_export_require_object_refs,
        db_audit_reads,
        db_tenant_policy_reads,
        require_db_mirror_writes,
        require_postgres_trace_rls_ready: false,
        postgres_runtime_role_sha256: None,
        require_derived_export_object_refs,
        object_primary_submit_review: false,
        object_primary_replay_export: false,
        object_primary_derived_exports: false,
        require_db_reconciliation_clean: false,
        require_export_guardrails,
        privacy_filter_backend: PrivacyFilterBackendTag::None,
        require_privacy_filter: false,
        community_leaderboard_enabled: false,
        community_snapshot_interval: None,
        community_analytics_publication_basis:
            CommunityAnalyticsPublicationBasis::ApprovedNoiseMechanism,
        accept_medium_risk_submissions: false,
        allow_legacy_segment_resume: false,
        community_tenant_ids: Arc::new(Vec::new()),
        tenant_rollout_gates: TraceTenantRolloutGates::default(),
        max_export_items_per_request: DEFAULT_TRACE_COMMONS_MAX_EXPORT_ITEMS_PER_REQUEST,
        submit_principal_limits: SubmitPrincipalLimits::default(),
        large_body_bytes: Arc::new(tokio::sync::Semaphore::new(LARGE_BODY_GLOBAL_BYTES)),
        // The whole budget: fixture principals are shared across parallel
        // tests, and the focused share tests set their own.
        large_body_principal_bytes: Arc::new(LargeBodyPrincipalBytes::new(LARGE_BODY_GLOBAL_BYTES)),
        upload_body_idle_timeout: UPLOAD_BODY_IDLE_TIMEOUT,
        submission_attempts: Arc::new(HourlySubmissionAttempts::default()),
        analytics_min_cell_count: 0,
        analytics_broad_release_noise: None,
        analytics_broad_release_privacy_accounting: None,
        credit_settlement_max_micros_per_account: None,
        credit_settlement_allowed_policy_versions: Arc::new(BTreeSet::new()),
        credit_settlement_require_issuer_approval: false,
        credit_settlement_issuer_approval_max_age: None,
        credit_settlement_require_central_issuer_profile: false,
        credit_settlement_central_issuer_principal_refs: Arc::new(BTreeSet::new()),
        credit_settlement_require_rollout_smoke_ready: false,
        credit_settlement_near_contract_id: None,
        credit_settlement_require_near_contract: false,
        submission_quota: TraceSubmissionQuotaConfig::default(),
        legal_hold_retention_policy_ids: Arc::new(BTreeSet::new()),
        artifact_store,
        require_object_store_versioning: false,
        near_credit_submitter: None,
        near_credit_submitter_timeout_ms: None,
        near_credit_submitter_auth_configured: false,
        near_credit_confirmer: None,
        near_credit_confirmer_timeout_ms: None,
        near_credit_confirmer_auth_configured: false,
        near_credit_require_adapter_auth: false,
        near_settlement_mode: NearSettlementMode::Http,
        near_credit_outbox_scheduler: None,
        benchmark_registry_submitter: None,
        benchmark_registry_submitter_timeout_ms: None,
        benchmark_registry_submitter_auth_configured: false,
        benchmark_registry_confirmer: None,
        benchmark_registry_confirmer_timeout_ms: None,
        benchmark_registry_confirmer_auth_configured: false,
        benchmark_registry_require_adapter_auth: false,
        benchmark_evaluator: None,
        benchmark_evaluator_timeout_ms: None,
        process_evaluator: None,
        process_evaluator_timeout_ms: None,
        remote_object_deleter: None,
        remote_object_deleter_timeout_ms: None,
        require_external_remote_object_deleter: false,
        worker_cache_invalidator: None,
        worker_cache_invalidator_timeout_ms: None,
        require_external_worker_cache_invalidator: false,
        vector_embedder: None,
        vector_embedder_timeout_ms: None,
        require_external_vector_embedder: false,
        vector_searcher: None,
        vector_searcher_timeout_ms: None,
        require_external_vector_searcher: false,
        export_job_scheduler: None,
        retention_maintenance_scheduler: None,
        vector_index_scheduler: None,
        perplexity_score_driver: None,
        pii_backstop_driver: None,
        witness_bypass: None,
        witness_capture_pin: None,
        admission: None,
        account_admission: None,
        client_upload_trust_database: None,
        account_trust_shadow_policy: None,
        near_ai_starter_mode: starter_advance::StarterMode::Disabled,
        account_trust_applied_policy: None,
        account_trust_scheduler: None,
        benchmark_registry_scheduler: None,
        benchmark_pipeline_scheduler: None,
        credit_cycle_scheduler: None,
        credit_settlement_scheduler: None,
        process_evaluation_scheduler: None,
        revocation_propagation_scheduler: None,
        community_snapshot_invalidation_scheduler: None,
        ranking_calibration_max_age: None,
        ranking_require_calibration_dataset_registry: false,
        ranking_require_active_calibration_dataset: false,
        ranking_require_server_feature_provenance: false,
        ranking_min_confidence_threshold: DEFAULT_TRACE_RANKING_MIN_CONFIDENCE_THRESHOLD,
        ranking_max_average_absolute_error_micros:
            TRACE_RANKING_DEFAULT_MAX_AVERAGE_ABSOLUTE_ERROR_MICROS,
        ranking_min_label_count: DEFAULT_TRACE_RANKING_MIN_LABEL_COUNT,
        ranking_min_label_source_count: DEFAULT_TRACE_RANKING_MIN_LABEL_SOURCE_COUNT,
        ranking_min_pairwise_label_count: DEFAULT_TRACE_RANKING_MIN_PAIRWISE_LABEL_COUNT,
        ranking_min_pairwise_accuracy_micros: DEFAULT_TRACE_RANKING_MIN_PAIRWISE_ACCURACY_MICROS,
        ranking_max_labeler_issue_rate_micros: None,
        ranking_min_labeler_reliability_label_count: None,
        gate_service: Arc::new(InMemoryGateService::new(
            "in_memory_default",
            "sha256:in_memory_default",
        )),
        revocation_propagation_max_attempts: DEFAULT_REVOCATION_PROPAGATION_MAX_ATTEMPTS,
        novelty_utility_credit_points_delta: DEFAULT_NOVELTY_UTILITY_CREDIT_POINTS_DELTA,
        novelty_utility_require_production_gate: false,
        novelty_utility_settle_from: None,
        scoring_driver_credit_issuer_principal_ref: None,
        account_ceremony_store: Arc::new(CeremonyStore::new()),
        account_native_requests: Arc::new(CeremonyStore::with_ttl(NATIVE_AUTH_REQUEST_TTL)),
        account_native_codes: Arc::new(CeremonyStore::with_ttl(NATIVE_AUTH_CODE_TTL)),
        account_near_config: None,
        attestation_signing: None,
        legacy_invite_link: None,
        #[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
        dedup_vector_index: None,
        #[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
        dedup_vector_index_id_map: Arc::new(
            std::sync::Mutex::new(std::collections::HashMap::new()),
        ),
        #[cfg(any(feature = "local-gpu-models", feature = "near-ai-scorer"))]
        dedup_vector_index_counter: Arc::new(std::sync::atomic::AtomicU64::new(1)),
        near_access_key_checker_override: None,
    })
}

pub(super) fn configure_unbounded_submit_limits_for_test(tokens: &BTreeMap<String, TenantAuth>) {
    for auth in tokens.values() {
        let key =
            submit_principal_rate_limit_key(&auth.tenant_id, auth.auth_method, &auth.principal_ref);
        configure_submit_rate_limits_for_test(&key, u32::MAX, u32::MAX);
        let key = large_body_principal_key(&auth.tenant_id, auth.auth_method, &auth.principal_ref);
        configure_submit_rate_limits_for_test(&key, u32::MAX, u32::MAX);
    }
}

pub async fn postgres_backend_for_ingest_test() -> Option<Arc<PgBackend>> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url:
            trace_commons_server::config::DatabaseConfig::login_resolver_url_from_env(),
        gate_driver_url: trace_commons_server::config::DatabaseConfig::gate_driver_url_from_env(),
        pii_backstop_driver_url:
            trace_commons_server::config::DatabaseConfig::pii_backstop_driver_url_from_env(),
        invite_registry_url: None,
    };
    let backend = match PgBackend::new(&config).await {
        Ok(backend) => Arc::new(backend),
        Err(error) => {
            eprintln!("skipping: database unavailable ({error})");
            return None;
        }
    };
    if let Err(error) = backend.run_migrations().await {
        eprintln!("skipping: migrations failed ({error})");
        return None;
    }
    // Reset the process-global account-surface rate limiter at the START of every
    // DB-backed test. The account/session/rotation tests all drive handlers
    // that share the `ACCOUNT_RATE_LIMITER` singleton (keyed by IP / global /
    // per-credential buckets); without this, a test inherits hit counts from
    // whatever ran before it and flakes with spurious rate-limit denials under
    // default parallelism. The
    // reset is harmless for tests that don't touch the limiter (it clears empty
    // maps) and runs before any test logic, so no test can accumulate state across
    // this boundary. This makes each DB-backed account test independent of
    // limiter state. NOTE: it does NOT remove the repo-wide requirement that the
    // DB-backed ingest suite run `--test-threads=1`; the shared `tenant-a` rows
    // (cleaned per-test via `cleanup_pg_trace_tenant`) still serialize those tests.
    reset_account_rate_limiter_for_db_test().await;
    Some(backend)
}

pub async fn reset_account_rate_limiter_for_db_test() {
    let _lock = submit_rate_limit_test_lock().lock().await;
    reset_account_rate_limiter_for_test();
}

pub fn submit_rate_limit_test_lock() -> &'static tokio::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<tokio::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| tokio::sync::Mutex::new(()))
}

/// Build a service-owned-remote artifact store whose backing provider is a
/// fresh on-disk directory + a local-master KEK. This is the only
/// configured artifact store shape that produces v2 (KEK-wrapped) artifacts
/// the gate worker is allowed to read; the file-system provider keeps the
/// fixture self-contained without standing up a real remote.
pub fn fixture_gate_worker_artifact_store(
    artifact_root: &Path,
) -> (ConfiguredTraceArtifactStore, String) {
    let object_store = TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE.to_string();
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let crypto = SecretsCrypto::new(SecretString::from(key.clone())).expect("fixture crypto");
    let kek_crypto = SecretsCrypto::new(SecretString::from(key)).expect("fixture kek crypto");
    let kek = LocalMasterKeyWrapper::new(kek_crypto, "trace-commons-gate-worker-test-v1");
    let provider_config = TraceArtifactProviderConfig::service_owned_remote(object_store.clone())
        .expect("provider config");
    let provider = FileRemoteTraceArtifactProvider::new(artifact_root.to_path_buf());
    let store = Arc::new(ServiceOwnedTraceArtifactStore::new(
        provider_config,
        crypto,
        kek,
        provider,
    ));
    let configured = ConfiguredTraceArtifactStore::new(object_store.clone(), store);
    (configured, object_store)
}

pub fn auth_headers(token: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    let value = format!("Bearer {token}");
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&value).expect("valid auth header"),
    );
    headers
}

pub async fn sample_envelope_with_user_input(content: &str) -> TraceContributionEnvelope {
    let trace = TraceFile {
        model_name: "test-model".to_string(),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: content.to_string(),
            },
            expected_tool_results: Vec::new(),
            timestamp: None,
        }],
    };
    let raw = trace_commons_protocol::trace_contribution::RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions {
            include_message_text: true,
            pseudonymous_contributor_id: Some("sha256:contributor".to_string()),
            tenant_scope_ref: Some("tenant_sha256:client".to_string()),
            ..Default::default()
        },
    );
    DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed")
}

pub fn make_metadata_only_low_risk(envelope: &mut TraceContributionEnvelope) {
    envelope.privacy.residual_pii_risk = ResidualPiiRisk::Low;
    envelope.consent.message_text_included = false;
    envelope.consent.tool_payloads_included = false;
    for event in &mut envelope.events {
        event.redacted_content = None;
        event.structured_payload = serde_json::Value::Null;
    }
}

pub fn set_metadata_only_user_message(envelope: &mut TraceContributionEnvelope, content: &str) {
    for event in &mut envelope.events {
        if event.event_type
            == trace_commons_protocol::trace_contribution::TraceContributionEventType::UserMessage
        {
            event.redacted_content = Some(content.to_string());
            return;
        }
    }
}

/// Send an upload's headers, declaring a body that is never sent. A handler
/// that polls the body waits on it; one that answers first does so without it.
pub async fn send_upload_headers_without_body(
    addr: std::net::SocketAddr,
    method: &str,
    path: &str,
    bearer: Option<&str>,
) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connects to the test server");
    let authorization = bearer
        .map(|token| format!("Authorization: Bearer {token}\r\n"))
        .unwrap_or_default();
    stream
        .write_all(
            format!(
                "{method} {path} HTTP/1.1\r\nHost: ingest.test\r\n{authorization}\
                 Content-Type: application/json\r\nContent-Length: 1024\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("request headers write");
    stream
}

/// The status line of the response, or `None` if the server has not answered
/// within `wait`.
pub async fn response_status_within(
    stream: &mut tokio::net::TcpStream,
    wait: StdDuration,
) -> Option<String> {
    use tokio::io::AsyncReadExt;

    let mut buffer = vec![0u8; 64];
    let read = tokio::time::timeout(wait, stream.read(&mut buffer))
        .await
        .ok()?
        .expect("response reads");
    let text = String::from_utf8_lossy(&buffer[..read]).into_owned();
    text.lines().next().map(str::to_owned)
}

/// The NEP-413 recipient the test NEAR config binds challenges to.
pub const NEAR_TEST_RECIPIENT: &str = "app.tracecommons.test";

/// `main`'s gate configuration the assembly tests hand the runtime: `main`'s
/// defaults, the floors `CompatibilityAssembler` builds with, and no
/// `NoveltyUtility` delta.
pub(super) const TEST_MAIN_GATE: trace_commons_server::versioned_pipeline_compat::MainGateConfig =
    trace_commons_server::versioned_pipeline_compat::MainGateConfig {
        perplexity_floor_micros: Some(2_000_000),
        tail_fraction_floor_micros: Some(0),
        novelty_floor_micros: Some(500_000),
        embed_insert_novelty_micros: TEST_EMBED_INSERT_NOVELTY_MICROS,
        top_k: 5,
        chunk_target_tokens: 2048,
        chunk_max_tokens: 3072,
        chunk_cap: 16,
        chunk_min_tokens: 64,
        novelty_utility_microcredits: 0,
        credit_rule: trace_commons_server::versioned_pipeline_compat::CompatibilityCreditRule::Flat,
    };

/// The `main` gate configuration `config` holds, for a test assembler that
/// builds `config` itself.
#[cfg(test)]
pub(super) fn main_gate_of(
    config: &trace_commons_server::versioned_pipeline_compat::CompatibilityBundleConfig,
) -> trace_commons_server::versioned_pipeline_compat::MainGateConfig {
    // The configuration's own inverse, every field included: a hand-copied
    // field list here dropped the credit rule once, and the production
    // corpus harness then refused a graded package (stage 2, 2026-11-03).
    config.main_gate()
}

/// `main`'s default index-insert threshold
/// (`TRACE_COMMONS_GATE_EMBED_INSERT_NOVELTY_MICROS`), as the assembly tests
/// hand it to the runtime.
pub(super) const TEST_EMBED_INSERT_NOVELTY_MICROS: u64 = 50_000;

/// The witness key and certificate signer the witness-receipt tests use.
///
/// Certificates are built from the real digest of the real bytes and signed
/// with the real key, so nothing here can pass by a fixture agreeing with a
/// bug.
pub mod witness_receipt_fixture {
    use super::*;
    use k256::ecdsa::SigningKey;
    use sha3::Keccak256;
    use trace_commons_server::redaction_witness::request::{CERTIFICATE_HEADER, SIGNATURE_HEADER};

    pub const MEASUREMENT: &str =
        "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

    pub fn signing_key() -> SigningKey {
        let bytes = Keccak256::digest(b"witness-receipt-test-witness");
        SigningKey::from_slice(&bytes).expect("seed is a valid scalar")
    }

    pub fn signing_address() -> String {
        let point = signing_key().verifying_key().to_encoded_point(false);
        let digest = Keccak256::digest(&point.as_bytes()[1..]);
        format!("0x{}", hex::encode(&digest[12..]))
    }

    pub fn certificate_for_policy(body: &[u8], verdict: &str, policy: &str) -> (String, String) {
        let json = serde_json::json!({
            "redacted_sha256": hex::encode(sha2::Sha256::digest(body)),
            "residual_risk_verdict": verdict,
            "redaction_policy_version": policy,
            "witness_measurement": MEASUREMENT,
            "timestamp": chrono::Utc::now().timestamp(),
        });
        sign_certificate_json(json)
    }

    pub fn sign_certificate_json(json: serde_json::Value) -> (String, String) {
        let encoded = serde_json::to_string(&json).expect("the certificate serialises");

        // Sign the same way the enclave does: the decoder rebuilds the
        // certificate and the signature must recover over ITS signing bytes,
        // so this goes through the decoder rather than reconstructing them.
        let mut headers = HeaderMap::new();
        headers.insert(CERTIFICATE_HEADER, encoded.parse().expect("a header value"));
        headers.insert(SIGNATURE_HEADER, "0x00".parse().expect("a header value"));
        let (certificate, _) =
            trace_commons_server::redaction_witness::request::witness_headers(&headers)
                .expect("the fixture certificate decodes")
                .expect("both headers are present");

        let message = certificate.signing_bytes();
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");
        hasher.update(message.len().to_string().as_bytes());
        hasher.update(&message);
        let (signature, recovery) = signing_key()
            .sign_prehash_recoverable(&hasher.finalize())
            .expect("prehash signs");
        let signature_hex = format!(
            "0x{}{:02x}",
            hex::encode(signature.to_bytes()),
            recovery.to_byte() + 27
        );
        (encoded, signature_hex)
    }
}

/// The migrated PostgreSQL the ignored ceremony suites start from.
#[path = "migrated_pg_fixture.rs"]
pub mod migrated_pg_fixture;

/// The NEAR AI provisioning harness: a database-backed state, a NEAR AI
/// stand-in, a device key and the router calls the provisioning tests make.
///
/// These fixtures set process environment variables and restore them; like
/// the rest of the PostgreSQL suite, tests that use them assume
/// `--test-threads=1`.
pub mod near_ai_provisioning_fixture {
    use super::*;
    use axum::body::Body;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt;

    /// The issuer variables `near_ai_login_readiness_named` reads, set for one
    /// test and restored when it ends, so a later test in the same process sees
    /// the environment it would have seen without this suite.
    pub struct ProvisioningEnv {
        pub previous: Vec<(&'static str, Option<String>)>,
    }

    impl ProvisioningEnv {
        pub fn publish() -> Self {
            let vars = [
                (
                    "TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL",
                    "https://issuer.example",
                ),
                ("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE", "upload"),
            ];
            let previous = vars
                .iter()
                .map(|(name, _)| (*name, std::env::var(name).ok()))
                .collect();
            for (name, value) in vars {
                // SAFETY: the PostgreSQL suite runs single-threaded.
                unsafe { std::env::set_var(name, value) };
            }
            Self { previous }
        }
    }

    impl Drop for ProvisioningEnv {
        fn drop(&mut self) {
            for (name, value) in self.previous.drain(..) {
                // SAFETY: as above.
                unsafe {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    pub fn identity() -> trace_commons_server::near_account_identity::NearAccountIdentity {
        use base64::Engine as _;
        let crypto =
            trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("b".repeat(32)))
                .expect("fixture SecretsCrypto");
        let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
            trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
                crypto,
                "near-ai-bind-fixture",
            ),
        );
        trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([5u8; 32])),
            Some(kek),
        )
        .expect("fixture identity")
    }

    /// A NEAR AI subject nobody has used, per call.
    pub fn fresh_subject() -> String {
        format!("auth0|z2-s3-bind-{}", Uuid::new_v4().simple())
    }

    /// A stand-in for NEAR AI's `GET /users/me` answering `subject` to any token,
    /// and a count of how often it was asked. The count is how a test proves a
    /// refused finish never spent the token.
    ///
    /// The subject is read per request from `subject`, so a test can stand for a
    /// Mac that is signed in to a different near.ai identity than the first.
    pub async fn stub_near_ai(
        subject: Arc<std::sync::Mutex<String>>,
    ) -> (String, Arc<AtomicUsize>) {
        let hits = Arc::new(AtomicUsize::new(0));
        let counter = hits.clone();
        let router = axum::Router::new().route(
            "/users/me",
            axum::routing::get(move || {
                let subject = subject.lock().expect("subject").clone();
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    axum::Json(serde_json::json!({ "id": subject, "auth_provider": "github" }))
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind stub");
        let base = format!("http://{}", listener.local_addr().expect("addr"));
        tokio::spawn(async move { axum::serve(listener, router).await.expect("stub") });
        (base, hits)
    }

    pub struct Harness {
        pub backend: Arc<PgBackend>,
        pub state: Arc<AppState>,
        /// What the NEAR AI stub answers now.
        pub live_subject: Arc<std::sync::Mutex<String>>,
        pub _env: ProvisioningEnv,
        // The state writes under this; it lives exactly as long as the state.
        pub _root: tempfile::TempDir,
    }

    impl Harness {
        /// From now on the NEAR AI stub answers `subject`: this Mac's near.ai
        /// sign-in is someone else's.
        pub fn sign_in_to_near_ai_as(&self, subject: &str) {
            *self.live_subject.lock().expect("subject") = subject.to_string();
        }

        pub async fn admin(&self) -> deadpool_postgres::Object {
            self.backend
                .raw_pool_for_tests_and_diagnostics()
                .get()
                .await
                .expect("admin")
        }
    }

    pub async fn harness() -> Option<Harness> {
        harness_with_admission(true).await
    }

    /// [`harness`], with the admission ledger configured or not.
    pub async fn harness_with_admission(admission_ready: bool) -> Option<Harness> {
        let backend = postgres_backend_for_ingest_test().await?;
        let env = ProvisioningEnv::publish();
        let live_subject = Arc::new(std::sync::Mutex::new(fresh_subject()));
        let (near_ai_base, _hits) = stub_near_ai(live_subject.clone()).await;
        let temp = tempfile::tempdir().expect("temp dir");
        let mut state = test_state(temp.path().to_path_buf());
        let settings = Arc::get_mut(&mut state).expect("fresh state is uniquely owned");
        settings.near_provisioning_enabled = true;
        settings.near_provisioning_admission_ready = admission_ready;
        settings.near_account_identity = Some(Arc::new(identity()));
        settings.db_mirror = Some(backend.clone() as Arc<dyn Database>);
        settings.near_ai_introspection_base_url = Some(near_ai_base);
        Some(Harness {
            backend,
            state,
            live_subject,
            _env: env,
            _root: temp,
        })
    }

    /// A daemon's device key.
    pub struct Device {
        pub pair: ring::signature::Ed25519KeyPair,
    }

    impl Default for Device {
        fn default() -> Self {
            Self::new()
        }
    }

    impl Device {
        pub fn new() -> Self {
            let rng = ring::rand::SystemRandom::new();
            let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("pkcs8");
            Self {
                pair: ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("key"),
            }
        }

        pub fn public_bytes(&self) -> [u8; 32] {
            use ring::signature::KeyPair as _;
            self.pair
                .public_key()
                .as_ref()
                .try_into()
                .expect("32 bytes")
        }

        pub fn public_b64(&self) -> String {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(self.public_bytes())
        }

        pub fn sign(&self, bytes: &[u8]) -> String {
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD.encode(self.pair.sign(bytes).as_ref())
        }
    }

    pub fn pkce() -> (String, String) {
        use base64::Engine as _;
        use sha2::Digest as _;
        let verifier = "v".repeat(64);
        let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .encode(sha2::Sha256::digest(verifier.as_bytes()));
        (verifier, challenge)
    }

    pub struct Reply {
        pub status: StatusCode,
        pub bytes: Vec<u8>,
        /// The headers a client acts on: whether to cache, and when to retry.
        pub cache_control: Option<String>,
        pub retry_after: Option<String>,
    }

    impl Reply {
        /// The status, body and client-facing headers of a router answer.
        pub async fn read(response: axum::response::Response) -> Self {
            let header = |name: axum::http::HeaderName| {
                response
                    .headers()
                    .get(name)
                    .map(|value| value.to_str().expect("ascii").to_string())
            };
            let cache_control = header(axum::http::header::CACHE_CONTROL);
            let retry_after = header(axum::http::header::RETRY_AFTER);
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body")
                .to_vec();
            Self {
                status,
                bytes,
                cache_control,
                retry_after,
            }
        }

        pub fn json(&self) -> serde_json::Value {
            serde_json::from_slice(&self.bytes).unwrap_or_default()
        }
    }

    pub async fn send(
        state: &Arc<AppState>,
        method: &str,
        uri: &str,
        body: Option<&serde_json::Value>,
        bearer: Option<&str>,
    ) -> Reply {
        let mut builder = axum::http::Request::builder().method(method).uri(uri);
        if let Some(token) = bearer {
            builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
        }
        let body = match body {
            Some(body) => {
                builder = builder.header(CONTENT_TYPE, "application/json");
                Body::from(serde_json::to_vec(body).expect("json"))
            }
            None => Body::empty(),
        };
        let response = app(state.clone())
            .oneshot(builder.body(body).expect("request"))
            .await
            .expect("router is infallible");
        Reply::read(response).await
    }

    pub async fn anchors_for(
        admin: &deadpool_postgres::Object,
        anchor_hash: &str,
    ) -> Vec<(String, Uuid)> {
        admin
            .query(
                "SELECT tenant_id, account_id FROM trace_near_account_anchors WHERE anchor_hash = $1",
                &[&anchor_hash],
            )
            .await
            .expect("anchors")
            .into_iter()
            .map(|row| (row.get(0), row.get(1)))
            .collect()
    }

    /// Remove what a test created. The anchor and provisioned-device tables do
    /// not cascade from the tenant, so they go first; a cleanup that fails is a
    /// test failure rather than a silent leftover.
    pub async fn drop_tenants(admin: &deadpool_postgres::Object, tenants: &[&str]) {
        for tenant in tenants {
            for table in [
                "trace_near_provisioned_devices",
                "trace_near_account_anchors",
                "trace_tenants",
            ] {
                admin
                    .execute(
                        &format!("DELETE FROM {table} WHERE tenant_id = $1"),
                        &[tenant],
                    )
                    .await
                    .unwrap_or_else(|error| panic!("cleanup {table}: {error}"));
            }
        }
    }

    pub fn unseen_subject() -> String {
        format!("auth0|open-sign-up-{}", Uuid::new_v4().simple())
    }

    /// One environment variable set for a test and restored when it ends.
    pub struct EnvVar {
        name: &'static str,
        previous: Option<String>,
    }

    impl EnvVar {
        pub fn set(name: &'static str, value: Option<&str>) -> Self {
            let previous = std::env::var(name).ok();
            // SAFETY: the PostgreSQL suite runs single-threaded.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
            Self { name, previous }
        }
    }

    impl Drop for EnvVar {
        fn drop(&mut self) {
            // SAFETY: as above.
            unsafe {
                match self.previous.take() {
                    Some(value) => std::env::set_var(self.name, value),
                    None => std::env::remove_var(self.name),
                }
            }
        }
    }

    pub const LEGACY_CAPABILITIES: &str = "/v1/account/near/provision/capabilities";
}
