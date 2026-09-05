// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
use super::*;
use axum::body::Body;
use k256::ecdsa::SigningKey;
use sha3::Keccak256;
use tower::ServiceExt;
use trace_commons_protocol::admission::{AdmissionBinding, REQUEST_METADATA_KEY, hash_hex};
use trace_commons_server::{
    admission_evidence::AdmissionProviderTrust,
    admission_ledger::AdmissionLimits,
    witness_service::{self, Enclave, SeamUnavailable, Signer},
};

struct FixtureSigner(SigningKey);
impl FixtureSigner {
    fn new(seed: &str) -> Self {
        Self(SigningKey::from_slice(&Keccak256::digest(seed.as_bytes())).unwrap())
    }
    fn address(&self) -> String {
        let point = self.0.verifying_key().to_encoded_point(false);
        format!(
            "0x{}",
            hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..])
        )
    }
}
impl Signer for FixtureSigner {
    fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
        let mut hash = Keccak256::new();
        hash.update(b"\x19Ethereum Signed Message:\n");
        hash.update(message.len().to_string().as_bytes());
        hash.update(message);
        let (signature, recovery) = self.0.sign_prehash_recoverable(&hash.finalize()).unwrap();
        Ok(format!(
            "0x{}{:02x}",
            hex::encode(signature.to_bytes()),
            recovery.to_byte() + 27
        ))
    }
}
struct FixtureEnclave(String);
#[async_trait::async_trait]
impl Enclave for FixtureEnclave {
    fn signing_address(&self) -> &str {
        &self.0
    }
    async fn measurement(&self) -> Result<String, SeamUnavailable> {
        Ok("synthetic-admission-measurement".into())
    }
    async fn attestation_quote(&self, _: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
        Ok(vec![1; 32])
    }
}
async fn post(
    state: Arc<AppState>,
    path: &str,
    body: Vec<u8>,
    headers: HeaderMap,
) -> axum::response::Response {
    let mut request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header(AUTHORIZATION, "Bearer admission-fixture-token")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(body))
        .unwrap();
    request.headers_mut().extend(headers);
    app(state).oneshot(request).await.unwrap()
}
async fn require_ok(response: axum::response::Response) -> Vec<u8> {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&body)
    );
    body.to_vec()
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn actual_postgres_challenge_witness_ingest_and_terminal_retry() {
    let url = std::env::var("TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL")
        .expect("explicit isolated URL required");
    let mut parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test"));
    let config = |url: String| DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    let admin = PgBackend::new(&config(url)).await.unwrap();
    admin.run_migrations().await.unwrap();
    let client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    client.batch_execute("DO $$ BEGIN IF NOT EXISTS(SELECT 1 FROM pg_roles WHERE rolname='admission_ingest_runtime') THEN CREATE ROLE admission_ingest_runtime LOGIN NOSUPERUSER NOBYPASSRLS; END IF; END $$;
      GRANT USAGE ON SCHEMA public TO admission_ingest_runtime;
      GRANT SELECT,INSERT,UPDATE,DELETE ON ALL TABLES IN SCHEMA public TO admission_ingest_runtime;
      REVOKE ALL ON trace_admission_receipts,trace_admission_global_budget FROM admission_ingest_runtime;
      GRANT USAGE,SELECT ON ALL SEQUENCES IN SCHEMA public TO admission_ingest_runtime;
      GRANT EXECUTE ON FUNCTION trace_reserve_admission(TEXT,TEXT,UUID,TEXT,TEXT,TEXT,BIGINT,BIGINT,BIGINT,BIGINT,UUID,BIGINT),trace_transition_admission(TEXT,UUID,UUID,TEXT) TO admission_ingest_runtime;").await.unwrap();
    parsed.set_username("admission_ingest_runtime").unwrap();
    let db = Arc::new(PgBackend::new(&config(parsed.into())).await.unwrap());
    assert!(db.admission_runtime_ready().await.unwrap());
    assert!(!admin.admission_runtime_ready().await.unwrap());
    // Explicit synthetic provisioned identity; B's account_onboarding_pg suite
    // separately verifies the wallet/device proof that creates this mapping.
    let anchor = hash_hex(Uuid::new_v4().as_bytes());
    let tenant = format!("near-{anchor}");
    let prefixed = format!("sha256:{anchor}");
    let account = Uuid::new_v4();
    let device_bytes: [u8; 32] = sha2::Sha256::digest(Uuid::new_v4().as_bytes()).into();
    let device =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&device_bytes);
    let mut tokens = BTreeMap::new();
    insert_token(
        &mut tokens,
        &tenant,
        "admission-fixture-token",
        TokenRole::Contributor,
    );
    let principal = tokens
        .get("admission-fixture-token")
        .unwrap()
        .principal_ref
        .clone();
    client
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1)",
            &[&tenant],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &account],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id) VALUES($1,$2,$3)",&[&tenant,&prefixed,&account]).await.unwrap();
    client.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near')",&[&device,&tenant,&base64::engine::general_purpose::STANDARD.encode(device_bytes)]).await.unwrap();
    client.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",&[&tenant,&account,&principal]).await.unwrap();
    client.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)",&[&tenant,&principal,&account,&device,&prefixed]).await.unwrap();
    assert_eq!(
        db.get_near_provisioned_anchor(&tenant, &principal)
            .await
            .unwrap(),
        Some(prefixed)
    );
    let provider = FixtureSigner::new("admission-route-provider");
    let signer = Arc::new(FixtureSigner::new("admission-route-witness"));
    let trust = AdmissionProviderTrust::new([provider.address()]).unwrap();
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state_with_tokens(temp.path().to_path_buf(), tokens);
    let state_mut = Arc::make_mut(&mut state);
    state_mut.db_mirror = Some(db.clone());
    state_mut.require_db_mirror_writes = true;
    state_mut.accept_medium_risk_submissions = true;
    state_mut.admission = Some(admission::AdmissionConfig {
        limits: AdmissionLimits {
            window_attempts: 1,
            account_cost_limit: 100,
            global_cost_limit: 1000,
            processing_cost_bound: 10,
            lease_seconds: 60,
            challenge_ttl_seconds: 60,
        },
        providers: trust.clone(),
    });
    // Window admission uses the real route and consumes exactly one attempt.
    let mut window = sample_envelope().await;
    make_metadata_only_low_risk(&mut window);
    let window_id = window.submission_id;
    let window_body = serde_json::to_vec(&window).unwrap();
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            window_body.clone(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            window_body.clone(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    let mut next = window.clone();
    next.submission_id = Uuid::new_v4();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&next).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
    let challenge = require_ok(
        post(
            state.clone(),
            "/v1/admission/challenge",
            Vec::new(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    let value: serde_json::Value = serde_json::from_slice(&challenge).unwrap();
    let binding = AdmissionBinding::parse(value["binding"].as_str().unwrap()).unwrap();
    assert_eq!(binding.account_anchor_sha256, anchor);
    let request_body=serde_json::json!({"model":"synthetic-model","metadata":{REQUEST_METADATA_KEY:binding.encode().unwrap()},"messages":[{"role":"user","content":"please summarize the successful build"}]}).to_string();
    let response_body = "{\"answer\":\"build succeeded\"}";
    let receipt_text = format!(
        "synthetic-model:{}:{}",
        hash_hex(request_body.as_bytes()),
        hash_hex(response_body.as_bytes())
    );
    let receipt = trace_commons_server::near_attestation::receipt::ReceiptPayload {
        text: receipt_text.clone(),
        signature: provider.sign_eip191(receipt_text.as_bytes()).unwrap(),
        signing_address: provider.address(),
    };
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RawTraceContribution, TraceContributionEventType,
    };
    let mut raw = RawTraceContribution::from_capture_turns(
        &[RawTraceCaptureTurn {
            user_input: "summarize build".into(),
            response: None,
            tool_calls: Vec::new(),
            started_at: Utc::now(),
            completed_at: Some(Utc::now()),
            state: Some("Completed".into()),
        }],
        RecordedTraceContributionOptions {
            include_message_text: true,
            pseudonymous_contributor_id: Some("sha256:synthetic-admission".into()),
            ..Default::default()
        },
    );
    let mut event = raw.events.last().unwrap().clone();
    event.event_id = Uuid::new_v4();
    event.event_type = TraceContributionEventType::HttpExchange;
    event.content = Some(response_body.into());
    event.structured_payload = serde_json::json!({"request":{"method":"POST","body":request_body},"response":{"status":200}});
    raw.events.push(event);
    let witness = witness_service::surface::WitnessService::new(
        Arc::new(witness_service::DeterministicRedaction::new(Vec::new())),
        signer.clone(),
        Arc::new(FixtureEnclave(signer.address())),
        1024 * 1024,
    )
    .with_contribution_redactor(Arc::new(
        witness_service::PipelineContributionRedaction::deterministic_only(Vec::new()),
    ))
    .with_admission_provider_trust(trust);
    let (response, evidence, signature) = witness
        .witness_admission_contribution(witness_service::WitnessContributionRequest {
            raw_contribution: raw,
            granted: witness_service::GrantedConsent {
                scopes: vec![
                    ConsentScope::DebuggingEvaluation,
                    ConsentScope::ModelTraining,
                ],
                uses: vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation],
            },
            offered_receipt: Some(receipt),
        })
        .await
        .unwrap();
    Arc::make_mut(&mut state).witness_bypass =
        trace_commons_server::redaction_witness::config::witness_bypass_config_from_values(
            Some("true"),
            Some(&signer.address()),
            Some("synthetic-admission-measurement"),
            Some(&evidence.redaction_policy_version),
            None,
        )
        .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert(
        trace_commons_server::redaction_witness::request::CERTIFICATE_HEADER,
        serde_json::to_string(&witness_service::http::certificate_json(
            &response.certificate,
            response.residual_risk_verdict(),
        ))
        .unwrap()
        .parse()
        .unwrap(),
    );
    headers.insert(
        trace_commons_server::redaction_witness::request::SIGNATURE_HEADER,
        response.signature_hex.parse().unwrap(),
    );
    headers.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&evidence).unwrap().parse().unwrap(),
    );
    headers.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        signature.parse().unwrap(),
    );
    let accepted = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            response.envelope_bytes.clone(),
            headers.clone(),
        )
        .await,
    )
    .await;
    // A terminal replay is an authenticated immutable receipt read even when
    // the original short-lived evidence is now expired or unavailable.
    let mut expired = evidence.clone();
    expired.issued_at = Utc::now().timestamp() - 120;
    expired.expires_at = Utc::now().timestamp() - 60;
    headers.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&expired).unwrap().parse().unwrap(),
    );
    headers.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        signer
            .sign_eip191(&expired.signing_bytes().unwrap())
            .unwrap()
            .parse()
            .unwrap(),
    );
    let repeated = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            response.envelope_bytes.clone(),
            headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(accepted, repeated);
    let mut changed: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).unwrap();
    changed.submission_id = Uuid::new_v4();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&changed).unwrap(),
            headers
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let row = client
        .query_one(
            "SELECT attempts_used,cost_bound_used FROM trace_admission_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap();
    assert_eq!(row.get::<_, i64>(0), 1);
    assert_eq!(row.get::<_, i64>(1), 20);
    assert!(
        db.lookup_completed_submission_admission(
            &tenant,
            &anchor,
            window_id,
            &hash_hex(&window_body)
        )
        .await
        .unwrap()
    );
    assert!(
        !db.lookup_completed_submission_admission(
            &tenant,
            &"0".repeat(64),
            window_id,
            &hash_hex(&window_body)
        )
        .await
        .unwrap()
    );
}
