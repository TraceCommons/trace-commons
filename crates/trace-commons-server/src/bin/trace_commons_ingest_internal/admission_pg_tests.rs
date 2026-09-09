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
    let mut admin_config = config(url.clone());
    admin_config.invite_registry_url = Some(SecretString::from(url));
    let admin = PgBackend::new(&admin_config).await.unwrap();
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
    // V61 gave this table three NOT NULL columns with no default: the sealed
    // account name and the two labels naming which pepper indexed it and which
    // key sealed it. Provisioning writes all three; this fixture stands one
    // provisioned account up by hand, so it has to write them too. They are
    // deliberately obvious placeholders -- nothing in this test reads them, and
    // a value that looked like a real seal would invite someone to trust it.
    client.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,$5,$6)",&[&tenant,&prefixed,&account,&serde_json::json!({"fixture":"not a real seal"}),&"fixture-pepper-ref",&"fixture-key-ref"]).await.unwrap();
    client.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near')",&[&device,&tenant,&base64::engine::general_purpose::STANDARD.encode(device_bytes)]).await.unwrap();
    client.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",&[&tenant,&account,&principal]).await.unwrap();
    client.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)",&[&tenant,&principal,&account,&device,&prefixed]).await.unwrap();
    assert_eq!(
        db.get_near_provisioned_anchor(&tenant, &principal)
            .await
            .unwrap(),
        Some(prefixed)
    );
    use ring::signature::KeyPair as _;
    let provider = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let provider_key = hex::encode(provider.public_key().as_ref());
    let signer = Arc::new(FixtureSigner::new("admission-route-witness"));
    let trust = AdmissionProviderTrust::new(
        [provider_key.clone()],
        Vec::new(),
        ["synthetic-model".into()],
        1,
    )
    .unwrap();
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
    // Resolve an actual registry invite, then use its resolved tenant with a
    // synthetic authenticated token. No receipt or witness header is needed.
    use trace_commons_server::db::InviteGrantWrite;
    use trace_commons_server::trace_invite_registry::InviteTenantMode;
    let invite_hash = format!("sha256:{}", hash_hex(Uuid::new_v4().as_bytes()));
    let invite_tenant = format!("invited-{}", Uuid::new_v4());
    admin
        .insert_invite_grant(InviteGrantWrite {
            invite_subject_hash: invite_hash.clone(),
            policy_label: "v012-test".into(),
            tenant_mode: InviteTenantMode::Fixed,
            fixed_tenant_id: Some(invite_tenant),
            tenant_template_id: None,
            policy_version: "v1".into(),
            allowed_consent_scopes: vec!["model_training".into()],
            allowed_uses: vec!["research".into()],
            max_uses: 1,
            expires_at: None,
            issuance_source: "operator".into(),
            issued_by_label: None,
            credential_binding_hash: None,
            note_label: None,
        })
        .await
        .unwrap();
    let redeemed = admin
        .redeem_invite_grant(&invite_hash, "v012-invited-user")
        .await
        .unwrap()
        .unwrap();
    // Exercise the same durable enrollment transaction the issuer calls,
    // including idempotency and global use-count enforcement.
    use trace_commons_server::db::{DeviceKeyWrite, OnboardDeviceKeyError};
    client
        .execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
            &[&redeemed.tenant_id],
        )
        .await
        .unwrap();
    let enrollment_seed: [u8; 32] = sha2::Sha256::digest(Uuid::new_v4().as_bytes()).into();
    let enrollment = |seed: [u8; 32], invite: &str| {
        let key = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed).unwrap();
        DeviceKeyWrite {
            device_key_id: trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
                key.public_key().as_ref(),
            ),
            tenant_id: redeemed.tenant_id.clone(),
            public_key: base64::engine::general_purpose::STANDARD.encode(key.public_key().as_ref()),
            invite_subject_hash: invite.into(),
            client_info: serde_json::json!({}),
            allowed_consent_scopes: Some(redeemed.allowed_consent_scopes.clone()),
            allowed_uses: Some(redeemed.allowed_uses.clone()),
        }
    };
    admin
        .onboard_device_key(enrollment(enrollment_seed, &invite_hash), 1)
        .await
        .unwrap();
    admin
        .onboard_device_key(enrollment(enrollment_seed, &invite_hash), 1)
        .await
        .unwrap();
    let second_seed: [u8; 32] = sha2::Sha256::digest(Uuid::new_v4().as_bytes()).into();
    assert!(matches!(
        admin
            .onboard_device_key(enrollment(second_seed, &invite_hash), 1)
            .await,
        Err(OnboardDeviceKeyError::InviteAlreadyConsumed)
    ));
    let unknown_invite = format!("sha256:{}", hash_hex(Uuid::new_v4().as_bytes()));
    // Registry authorization precedes enrollment; the storage transaction also
    // supports legacy file-authorized invites and cannot reject all absent rows.
    assert!(
        admin
            .redeem_invite_grant(&unknown_invite, "v012-unknown-user")
            .await
            .unwrap()
            .is_none()
    );
    client.execute("UPDATE onboarding_invite_grants SET expires_at=now()-interval '1 second' WHERE invite_subject_hash=$1", &[&invite_hash]).await.unwrap();
    assert!(
        admin
            .redeem_invite_grant(&invite_hash, "v012-expired-user")
            .await
            .unwrap()
            .is_none()
    );
    // Expiring the invite prevents new enrollments; the already enrolled
    // contributor still follows ordinary authenticated contribution policy.
    let mut invited_tokens = BTreeMap::new();
    insert_token(
        &mut invited_tokens,
        &redeemed.tenant_id,
        "admission-fixture-token",
        TokenRole::Contributor,
    );
    let invited_root = tempfile::tempdir().unwrap();
    let mut invited_state =
        test_state_with_tokens(invited_root.path().to_path_buf(), invited_tokens);
    Arc::make_mut(&mut invited_state).db_mirror = Some(db.clone());
    Arc::make_mut(&mut invited_state).require_db_mirror_writes = true;
    for source in ["opencode", "second-client"] {
        let mut ordinary = sample_envelope().await;
        make_metadata_only_low_risk(&mut ordinary);
        ordinary
            .ironclaw
            .feature_flags
            .insert("agent".into(), source.into());
        require_ok(
            post(
                invited_state.clone(),
                "/v1/traces",
                serde_json::to_vec(&ordinary).unwrap(),
                HeaderMap::new(),
            )
            .await,
        )
        .await;
    }
    // An authenticated account without redeemed invite authority cannot use
    // the historical trial window to contribute an ordinary artifact.
    let mut window = sample_envelope().await;
    make_metadata_only_low_risk(&mut window);
    let window_id = window.submission_id;
    let window_body = serde_json::to_vec(&window).unwrap();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            window_body.clone(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
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
        signature: hex::encode(provider.sign(receipt_text.as_bytes()).as_ref()),
        signing_address: provider_key.clone(),
        signing_algo: trace_commons_server::near_attestation::receipt::ReceiptAlgo::Ed25519,
        signature_kind:
            trace_commons_server::near_attestation::receipt::ReceiptSignatureKind::ProviderTee,
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
    const METADATA_SENTINEL: &str = "witness-sentinel@example.com";
    raw.conversation_id = Some(METADATA_SENTINEL.into());
    raw.ironclaw
        .feature_flags
        .insert("agent".into(), "opencode".into());
    raw.ironclaw
        .feature_flags
        .insert("note".into(), METADATA_SENTINEL.into());
    raw.replay.replay_notes.push(METADATA_SENTINEL.into());
    raw.value.explanation.push(METADATA_SENTINEL.into());
    let mut event = raw.events.last().unwrap().clone();
    event.event_id = Uuid::new_v4();
    event.event_type = TraceContributionEventType::HttpExchange;
    event.content = Some(response_body.into());
    event.structured_payload = serde_json::json!({"request":{"method":"POST","body":request_body},"response":{"status":200}});
    raw.events = vec![event];
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
    // Each missing binding fails before any submission is admitted.
    for missing in [
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        trace_commons_server::redaction_witness::request::CERTIFICATE_HEADER,
        trace_commons_server::redaction_witness::request::SIGNATURE_HEADER,
    ] {
        let mut incomplete = headers.clone();
        incomplete.remove(missing);
        assert_eq!(
            post(
                state.clone(),
                "/v1/traces",
                response.envelope_bytes.clone(),
                incomplete
            )
            .await
            .status(),
            StatusCode::FORBIDDEN,
            "missing {missing}"
        );
    }
    let mut changed_approved_bytes = response.envelope_bytes.clone();
    changed_approved_bytes.push(b'\n');
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            changed_approved_bytes,
            headers.clone()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
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
    let artifact: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).unwrap();
    let receipt: serde_json::Value = serde_json::from_slice(&accepted).unwrap();
    let stored_path = state.root.join(format!(
        "tenants/{}/objects/{}/{}.json",
        tenant_storage_key(&tenant),
        receipt["status"].as_str().unwrap(),
        artifact.submission_id
    ));
    let stored = std::fs::read_to_string(stored_path).unwrap();
    assert!(
        !stored.contains(METADATA_SENTINEL),
        "stored artifact retained metadata PII"
    );
    let stored: TraceContributionEnvelope = serde_json::from_str(&stored).unwrap();
    for event in &stored.events {
        if event.event_type == TraceContributionEventType::HttpExchange {
            assert!(event.redacted_content.is_none());
            for side in ["request", "response"] {
                assert!(event.structured_payload[side].get("body").is_none());
                assert!(event.structured_payload[side].get("headers").is_none());
            }
        }
    }
    let mut later_ordinary = window.clone();
    later_ordinary.submission_id = Uuid::new_v4();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&later_ordinary).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "successful attestation must not establish a permanent ordinary-upload bypass"
    );
    assert_eq!(
        accepted,
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                response.envelope_bytes.clone(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        "exact completed retry is a receipt read, not a new contribution"
    );
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
    assert_eq!(row.get::<_, i64>(0), 0);
    assert_eq!(row.get::<_, i64>(1), 10);
    assert!(
        !db.lookup_completed_submission_admission(
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

/// A migrated admission database, connected as the owning role.
///
/// These two tests do not need the restricted `admission_ingest_runtime` role
/// the end-to-end matrix above builds: `admission::anchor` reads one row
/// through explicit tenant and principal predicates, and what they are about
/// is which row it finds, not which grant reaches it.
async fn admission_pg_admin() -> Arc<PgBackend> {
    let url = std::env::var("TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL")
        .expect("explicit isolated URL required");
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
    assert!(parsed.path().starts_with("/admission_test"));
    let admin = PgBackend::new(&DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    admin.run_migrations().await.unwrap();
    Arc::new(admin)
}

/// One synthetic provisioned NEAR account in the shape V61 leaves behind: a
/// tenant id drawn at random by `random_near_tenant_id`, and an anchor that is
/// a blind index with no arithmetic relationship to it. Provisioning itself is
/// covered by `account_onboarding_pg`; this stands the rows up directly so the
/// admission read can be examined on its own.
///
/// Returns the tenant id, the stored anchor without its `sha256:` prefix, and
/// the device key id, so a caller can revoke the device.
async fn provision_synthetic_near_account(
    db: &PgBackend,
    principal: &str,
) -> (String, String, String) {
    let tenant = trace_commons_server::near_account_identity::random_near_tenant_id();
    let anchor = hash_hex(Uuid::new_v4().as_bytes());
    let prefixed = format!("sha256:{anchor}");
    assert_ne!(
        tenant.strip_prefix("near-"),
        Some(anchor.as_str()),
        "the fixture must not reproduce the pre-V61 coupling it exists to rule out"
    );
    let account = Uuid::new_v4();
    let device_bytes: [u8; 32] = sha2::Sha256::digest(Uuid::new_v4().as_bytes()).into();
    let device =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&device_bytes);
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
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
    client.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,$5,$6)",&[&tenant,&prefixed,&account,&serde_json::json!({"fixture":"not a real seal"}),&"fixture-pepper-ref",&"fixture-key-ref"]).await.unwrap();
    client.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near')",&[&device,&tenant,&base64::engine::general_purpose::STANDARD.encode(device_bytes)]).await.unwrap();
    client.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",&[&tenant,&account,&principal]).await.unwrap();
    client.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)",&[&tenant,&principal,&account,&device,&prefixed]).await.unwrap();
    (tenant, anchor, device)
}

/// An `AppState` whose only relevant part is the mirror `admission::anchor`
/// reads, plus a `TenantCtx` per configured token.
fn anchor_state(
    db: Arc<PgBackend>,
    principals: &[(&str, &str)],
) -> (tempfile::TempDir, Arc<AppState>, Vec<TenantCtx>) {
    let mut tokens = BTreeMap::new();
    for (tenant, token) in principals {
        insert_token(&mut tokens, tenant, token, TokenRole::Contributor);
    }
    let contexts = principals
        .iter()
        .map(|(_, token)| TenantCtx::from_auth(tokens.get(*token).unwrap().clone()))
        .collect();
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state_with_tokens(temp.path().to_path_buf(), tokens);
    Arc::make_mut(&mut state).db_mirror = Some(db);
    (temp, state, contexts)
}

fn principal_for(token: &str) -> String {
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, "near-fixture", token, TokenRole::Contributor);
    tokens.get(token).unwrap().principal_ref.clone()
}

/// #783. V58 made `tenant_id` a function of `anchor_hash` and `admission::anchor`
/// asserted the two were equal. V61 deliberately destroyed that relationship --
/// the tenant id is now random and the anchor a blind index -- which left the
/// equality unsatisfiable and refused every `near-` tenant, so invite-free
/// contribution could not be switched on at all.
///
/// This is the assertion the equality fails: an account provisioned in the
/// post-V61 shape resolves to its own anchor.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn a_salted_anchor_resolves_for_its_provisioned_principal() {
    let db = admission_pg_admin().await;
    let token = "anchor-fixture-owner-783";
    let (tenant, anchor, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, state, contexts) = anchor_state(db, &[(tenant.as_str(), token)]);
    assert_eq!(
        admission::anchor(&state, &contexts[0])
            .await
            .map_err(|(status, _)| status)
            .expect("a provisioned near- tenant must resolve its own anchor"),
        Some(anchor),
    );
}

/// What binds a request to an anchor, now that the equality is gone: the row
/// is looked up by the authenticated tenant AND the authenticated principal,
/// and only through an unrevoked `near`-origin device key on a linked
/// principal of an open account. Nothing in the request body or headers
/// selects it.
///
/// Delete that lookup -- return the tenant suffix, a constant, or any value
/// not read from the row -- and this fails: two accounts would stop resolving
/// to distinct anchors, a principal with no provisioning would be admitted,
/// and revoking the device would no longer refuse.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn the_anchor_lookup_is_what_binds_a_request_to_its_account() {
    let db = admission_pg_admin().await;
    let (first_token, second_token, stranger_token) = (
        "anchor-binding-first-783",
        "anchor-binding-second-783",
        "anchor-binding-stranger-783",
    );
    let (first_tenant, first_anchor, first_device) =
        provision_synthetic_near_account(&db, &principal_for(first_token)).await;
    let (second_tenant, second_anchor, _) =
        provision_synthetic_near_account(&db, &principal_for(second_token)).await;
    assert_ne!(first_anchor, second_anchor);
    let (_temp, state, contexts) = anchor_state(
        db.clone(),
        &[
            (first_tenant.as_str(), first_token),
            (second_tenant.as_str(), second_token),
            // Authenticated in the first tenant, but never provisioned there.
            (first_tenant.as_str(), stranger_token),
        ],
    );

    assert_eq!(
        admission::anchor(&state, &contexts[0])
            .await
            .map_err(|(status, _)| status)
            .unwrap(),
        Some(first_anchor),
        "each tenant must resolve the anchor stored for it"
    );
    assert_eq!(
        admission::anchor(&state, &contexts[1])
            .await
            .map_err(|(status, _)| status)
            .unwrap(),
        Some(second_anchor),
        "a second account must not resolve to the first account's anchor"
    );
    assert!(
        admission::anchor(&state, &contexts[2]).await.is_err(),
        "a principal with no provisioned device in this tenant has no anchor"
    );

    db.raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE device_keys SET revoked_at=now() WHERE device_key_id=$1",
            &[&first_device],
        )
        .await
        .unwrap();
    assert!(
        admission::anchor(&state, &contexts[0]).await.is_err(),
        "revoking the provisioned device must withdraw the anchor"
    );
}
