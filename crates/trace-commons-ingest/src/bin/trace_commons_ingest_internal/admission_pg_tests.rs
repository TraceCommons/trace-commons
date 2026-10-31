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

/// A synthetic EIP-191 signer standing in for a real provider-TEE/gateway
/// signer. `pub(super)`: shared with `pipeline_http_pg_tests`, which needs
/// the same NEAR evidence/witness chain to reach a real, evidence-verified
/// upload for a pipeline-routed, admission-gated tenant.
pub(super) struct FixtureSigner(SigningKey);
impl FixtureSigner {
    pub(super) fn new(seed: &str) -> Self {
        Self(SigningKey::from_slice(&Keccak256::digest(seed.as_bytes())).unwrap())
    }
    pub(super) fn address(&self) -> String {
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
/// `pub(super)`: see `FixtureSigner`.
pub(super) struct FixtureEnclave(pub(super) String);
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
      GRANT trace_account_admission_runtime TO admission_ingest_runtime;
      REVOKE ALL ON trace_accounts,trace_account_principals,trace_near_provisioned_devices,device_keys,trace_near_account_anchors,trace_account_trust,trace_account_invite_grants,trace_account_admission_budget,trace_account_admission_submissions FROM admission_ingest_runtime;

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
            issuer_display_name: None,
            credit_range: None,
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
            raw_contribution: raw.clone(),
            granted: witness_service::GrantedConsent {
                scopes: vec![
                    ConsentScope::DebuggingEvaluation,
                    ConsentScope::ModelTraining,
                ],
                uses: vec![TraceAllowedUse::Debugging, TraceAllowedUse::Evaluation],
            },
            offered_receipt: Some(receipt.clone()),
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
    assert!(
        serde_json::from_slice::<serde_json::Value>(&response.envelope_bytes)
            .unwrap()
            .get("source_session")
            .is_none(),
        "legacy bytes omit the new field entirely"
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
    Arc::make_mut(&mut state).account_admission = Some(admission::AccountAdmissionConfig {
        policy: trace_commons_server::account_trust::parse_bounded_policy(
            r#"{"version":"legacy-cutover-fixture","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
            &["legacy-cutover-fixture"],
        ).unwrap(),
        lease_seconds: 60,
        providers: None,
    });
    client.batch_execute("REVOKE EXECUTE ON FUNCTION trace_transition_admission(TEXT,UUID,UUID,TEXT) FROM admission_ingest_runtime").await.unwrap();
    // Account role now supplies all identity/admission privileges, including V59 transition.
    // Coexistence (V81, decision A of 2026-09-27): account admission governs
    // only `near-`/`nearai-`. A legacy invite identity keeps contributing
    // exactly as before the switch, on the path it always used.
    let mut coexisting = sample_envelope().await;
    make_metadata_only_low_risk(&mut coexisting);
    require_ok(
        post(
            {
                let mut cutover = invited_state.clone();
                Arc::make_mut(&mut cutover).account_admission = state.account_admission.clone();
                cutover
            },
            "/v1/traces",
            serde_json::to_vec(&coexisting).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    let unauthenticated_status = app(state.clone())
        .oneshot(
            axum::http::Request::builder()
                .uri("/v1/account/contribution-status")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        !unauthenticated_status.status().is_success(),
        "advisory status stays behind account authentication"
    );
    let legacy_retry = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            response.envelope_bytes.clone(),
            headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(
        accepted, legacy_retry,
        "completed legacy receipt is readable after cutover"
    );
    let mut altered_raw = response.envelope_bytes.clone();
    altered_raw.push(b' ');
    assert_eq!(
        post(state.clone(), "/v1/traces", altered_raw, HeaderMap::new())
            .await
            .status(),
        StatusCode::CONFLICT,
        "legacy UUID is bound to exact bytes across ledgers"
    );
    let account_rows: i64 = client
        .query_one(
            "SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        account_rows, 0,
        "legacy retry and conflict never reserve account budget"
    );
    let admitted: TraceContributionEnvelope =
        serde_json::from_slice(&response.envelope_bytes).unwrap();
    // Simulate a process crash after the trace receipt landed but before V59
    // marked its lease completed. The old charge stays; retrying with the
    // original signed headers acquires a new V59 lease, never account debt.
    client.execute("UPDATE trace_admission_submissions SET status='processing',lease_expires_at=now()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&admitted.submission_id]).await.unwrap();
    let mut altered_expired = response.envelope_bytes.clone();
    altered_expired.push(b' ');
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            altered_expired,
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let mut changed_signature = headers.clone();
    changed_signature.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        HeaderValue::from_static("invalid-replay-signature"),
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            response.envelope_bytes.clone(),
            changed_signature
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "an offered changed replay signature cannot replace stored authority"
    );
    let resumed = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            response.envelope_bytes.clone(),
            headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(
        accepted, resumed,
        "expired legacy lease resumes with original headers"
    );
    // Read per tenant, not from the database-wide singleton: other tests in
    // this module charge the V59 ledger too, and libtest runs them in parallel.
    let legacy_charge_after_expiry: i64 = client
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        legacy_charge_after_expiry, 20,
        "V59 retains the crashed processing charge"
    );

    let mut released = sample_envelope().await;
    make_metadata_only_low_risk(&mut released);
    assert!(
        released.source_session.is_none(),
        "historical bodies lack session metadata"
    );
    let released_body = serde_json::to_vec(&released).unwrap();
    let released_hash = hash_hex(&released_body);
    client.execute("INSERT INTO trace_admission_submissions(tenant_id,submission_id,anchor_hash,body_hash,kind,status,lease_id,lease_expires_at,last_cost_bound,attempt_held,ever_processed) VALUES($1,$2,$3,$4,'window','reserved',$5,now()+interval '60 seconds',10,FALSE,FALSE)", &[&tenant,&released.submission_id,&anchor,&released_hash,&Uuid::new_v4()]).await.unwrap();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            released_body.clone(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT,
        "a live legacy lease remains busy after cutover"
    );
    client.execute("UPDATE trace_admission_submissions SET status='released' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&released.submission_id]).await.unwrap();
    let mut altered_released = released_body.clone();
    altered_released.push(b' ');
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            altered_released,
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            released_body.clone(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::OK,
        "released legacy reservation is reclaimable under the old ledger"
    );
    let total_legacy_charge: i64 = client
        .query_one(
            "SELECT cost_bound_used FROM trace_admission_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        total_legacy_charge, 30,
        "released retry contributes one new bound"
    );
    let account_rows: i64 = client
        .query_one(
            "SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        account_rows, 0,
        "no legacy recovery creates account authority"
    );
    // A freshly offered proof remains checked in account mode using its
    // independent provider policy, with no legacy environment lookup.
    raw.submission_id = Uuid::new_v4();
    raw.trace_id = Uuid::new_v4();
    // Account admission requires a source-session identity on every new use.
    raw.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    let (fresh, mut fresh_evidence, _) = witness
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
    // First use must land inside this window on a slow, parallel CI runner; a
    // short one made the test fail at random. Expiry is exercised below by
    // re-signing the same evidence into the past, not by waiting it out.
    fresh_evidence.expires_at = Utc::now().timestamp() + 120;
    let fresh_signature = signer
        .sign_eip191(&fresh_evidence.signing_bytes().unwrap())
        .unwrap();
    let account_providers = state.admission.as_ref().unwrap().providers.clone();
    Arc::make_mut(&mut state)
        .account_admission
        .as_mut()
        .unwrap()
        .providers = Some(account_providers);
    let mut offered = HeaderMap::new();
    offered.insert(
        AUTHORIZATION,
        HeaderValue::from_static("Bearer admission-fixture-token"),
    );
    offered.insert(
        trace_commons_server::redaction_witness::request::CERTIFICATE_HEADER,
        serde_json::to_string(&witness_service::http::certificate_json(
            &fresh.certificate,
            fresh.residual_risk_verdict(),
        ))
        .unwrap()
        .parse()
        .unwrap(),
    );
    offered.insert(
        trace_commons_server::redaction_witness::request::SIGNATURE_HEADER,
        fresh.signature_hex.parse().unwrap(),
    );
    offered.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&fresh_evidence)
            .unwrap()
            .parse()
            .unwrap(),
    );
    offered.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        fresh_signature.parse().unwrap(),
    );
    let authenticated = authenticate_ctx(&state, &offered).unwrap();
    let fresh_envelope: TraceContributionEnvelope =
        serde_json::from_slice(&fresh.envelope_bytes).unwrap();
    let mut first_use_expired = fresh_evidence.clone();
    first_use_expired.issued_at = Utc::now().timestamp() - 120;
    first_use_expired.expires_at = Utc::now().timestamp() - 60;
    let mut expired_headers = offered.clone();
    expired_headers.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&first_use_expired)
            .unwrap()
            .parse()
            .unwrap(),
    );
    expired_headers.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        signer
            .sign_eip191(&first_use_expired.signing_bytes().unwrap())
            .unwrap()
            .parse()
            .unwrap(),
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            fresh.envelope_bytes.clone(),
            expired_headers
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "first use still requires unexpired evidence"
    );
    let first_use_rows: i64 = client.query_one("SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&fresh_envelope.submission_id]).await.unwrap().get(0);
    assert_eq!(
        first_use_rows, 0,
        "refused first use creates no account binding or charge"
    );
    let mut attempt = admission::reserve(
        &state,
        &authenticated,
        &offered,
        &fresh.envelope_bytes,
        &fresh_envelope,
    )
    .await
    .unwrap()
    .unwrap();
    let resolved = trace_commons_server::account_trust::resolve_contribution_account(
        db.as_ref(),
        &tenant,
        &principal,
    )
    .await
    .unwrap();
    // Revocation makes the advisory read fail, but the charged reservation
    // already produced its Attempt and remains releasable without a new read.
    client
        .execute(
            "UPDATE device_keys SET revoked_at=clock_timestamp() WHERE device_key_id=$1",
            &[&device],
        )
        .await
        .unwrap();
    assert!(
        db.account_admission_status(
            &resolved,
            &principal,
            &state.account_admission.as_ref().unwrap().policy
        )
        .await
        .unwrap()
        .is_none()
    );
    attempt.finish(db.as_ref(), false).await.unwrap();
    drop(attempt);
    client
        .execute(
            "UPDATE device_keys SET revoked_at=NULL WHERE device_key_id=$1",
            &[&device],
        )
        .await
        .unwrap();
    // Fresh verified evidence admits without debiting the allowance (owner
    // decision of 2026-10-11): no budget row, and the row records why.
    let charged: i64 = client
        .query_one(
            "SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(charged, 0, "an attested reservation debits nothing");
    let (kind, last_charged, status): (String, bool, String) = {
        let row = client
            .query_one(
                "SELECT authority_kind,last_charged,status FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2",
                &[&tenant, &fresh_envelope.submission_id],
            )
            .await
            .unwrap();
        (row.get(0), row.get(1), row.get(2))
    };
    assert_eq!(
        (kind.as_str(), last_charged, status.as_str()),
        ("attested", false, "released"),
        "pre-processing release succeeds even after revocation"
    );
    // The signed evidence accepted above now expires: the same evidence,
    // re-signed with a window that has already closed. Released, completed,
    // and crashed leases all retain account/body binding.
    let mut lapsed_evidence = fresh_evidence.clone();
    lapsed_evidence.issued_at = Utc::now().timestamp() - 120;
    lapsed_evidence.expires_at = Utc::now().timestamp() - 60;
    offered.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        serde_json::to_string(&lapsed_evidence)
            .unwrap()
            .parse()
            .unwrap(),
    );
    offered.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        signer
            .sign_eip191(&lapsed_evidence.signing_bytes().unwrap())
            .unwrap()
            .parse()
            .unwrap(),
    );
    Arc::make_mut(&mut state).account_admission.as_mut().unwrap().policy = trace_commons_server::account_trust::parse_bounded_policy(
        r#"{"version":"account-retry-wave2","processing_cost_bound":10,"bounded_allowance":100,"period":{"mode":"lifetime"},"growth_rule":"none"}"#, &["account-retry-wave2"]).unwrap();
    let account_receipt = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            fresh.envelope_bytes.clone(),
            offered.clone(),
        )
        .await,
    )
    .await;
    let spent = || async {
        client.query_one("SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1", &[&tenant]).await.unwrap().get::<_,i64>(0)
    };
    assert_eq!(
        spent().await,
        0,
        "released attested recovery keeps its attested authority and debits nothing"
    );
    assert_eq!(
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                fresh.envelope_bytes.clone(),
                offered.clone()
            )
            .await
        )
        .await,
        account_receipt,
        "completed account retry accepts its original expired evidence"
    );
    assert_eq!(spent().await, 0, "completed retry has zero debit");
    assert_eq!(
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                fresh.envelope_bytes.clone(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        account_receipt,
        "headerless completed retry remains valid"
    );
    for status in ["reserved", "processing"] {
        client.execute("UPDATE trace_account_admission_submissions SET status=$3,lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&fresh_envelope.submission_id,&status]).await.unwrap();
        assert_eq!(
            require_ok(
                post(
                    state.clone(),
                    "/v1/traces",
                    fresh.envelope_bytes.clone(),
                    offered.clone()
                )
                .await
            )
            .await,
            account_receipt,
            "expired account lease recovers with authentic stale evidence"
        );
        assert_eq!(
            spent().await,
            0,
            "expired attested recovery reserves on its recorded authority, with no debit"
        );
    }
    let mut altered = fresh.envelope_bytes.clone();
    altered.push(b' ');
    assert_eq!(
        post(state.clone(), "/v1/traces", altered, offered.clone())
            .await
            .status(),
        StatusCode::CONFLICT,
        "stored UUID rejects different body bytes"
    );
    let foreign_account = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &foreign_account],
        )
        .await
        .unwrap();
    client.execute("UPDATE trace_account_admission_submissions SET account_id=$3 WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&fresh_envelope.submission_id,&foreign_account]).await.unwrap();
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            fresh.envelope_bytes.clone(),
            offered.clone()
        )
        .await
        .status(),
        StatusCode::CONFLICT,
        "foreign account binding cannot be resumed"
    );
    client.execute("UPDATE trace_account_admission_submissions SET account_id=$3 WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&fresh_envelope.submission_id,&account]).await.unwrap();
    let mut invalid_signature = offered.clone();
    invalid_signature.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        HeaderValue::from_static("invalid"),
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            fresh.envelope_bytes.clone(),
            invalid_signature
        )
        .await
        .status(),
        StatusCode::FORBIDDEN,
        "completed replay never ignores an invalid offered signature"
    );
    for wrong_anchor in [false, true] {
        let mut mismatched = fresh_evidence.clone();
        if wrong_anchor {
            mismatched.account_anchor_sha256 = "f".repeat(64);
        } else {
            mismatched.artifact_sha256 = "e".repeat(64);
        }
        let mut mismatched_headers = offered.clone();
        mismatched_headers.insert(
            trace_commons_protocol::admission::EVIDENCE_HEADER,
            serde_json::to_string(&mismatched).unwrap().parse().unwrap(),
        );
        mismatched_headers.insert(
            trace_commons_protocol::admission::SIGNATURE_HEADER,
            signer
                .sign_eip191(&mismatched.signing_bytes().unwrap())
                .unwrap()
                .parse()
                .unwrap(),
        );
        assert_eq!(
            post(
                state.clone(),
                "/v1/traces",
                fresh.envelope_bytes.clone(),
                mismatched_headers
            )
            .await
            .status(),
            StatusCode::FORBIDDEN,
            "signed proof must retain account and exact body binding"
        );
    }
    assert_eq!(spent().await, 0, "conflicts and bad proofs never debit");
    offered.remove(trace_commons_protocol::admission::SIGNATURE_HEADER);
    assert!(
        admission::reserve(
            &state,
            &authenticated,
            &offered,
            &fresh.envelope_bytes,
            &fresh_envelope
        )
        .await
        .is_err(),
        "offered partial proof is never ignored"
    );
    // Invites remove cumulative account debt, but retain the hourly pacing quota.
    let invite_hash = format!("sha256:{}", hash_hex(Uuid::new_v4().as_bytes()));
    client.execute("INSERT INTO trace_account_trust(tenant_id,account_id,authority,trust_version) VALUES($1,$2,'invited',1) ON CONFLICT(tenant_id,account_id) DO UPDATE SET authority='invited',trust_version=1", &[&tenant,&account]).await.unwrap();
    client.execute("INSERT INTO trace_account_invite_grants(tenant_id,account_id,invite_subject_hash,trust_version) VALUES($1,$2,$3,1)", &[&tenant,&account,&invite_hash]).await.unwrap();
    Arc::make_mut(&mut state)
        .submission_quota
        .max_per_tenant_per_hour = 1;
    let mut quota_envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut quota_envelope);
    quota_envelope.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    let quota_response = post(
        state.clone(),
        "/v1/traces",
        serde_json::to_vec(&quota_envelope).unwrap(),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(
        quota_response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "invited accounts still enforce hourly quota"
    );
    Arc::make_mut(&mut state)
        .submission_quota
        .max_per_tenant_per_hour = 0;
    Arc::make_mut(&mut state).account_admission = None;
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
    assert_eq!(
        row.get::<_, i64>(0),
        1,
        "released window recovery consumes one trial attempt"
    );
    assert_eq!(
        row.get::<_, i64>(1),
        30,
        "original, crashed retry and released retry each retain their correct bound"
    );
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
        url: SecretString::from(url.clone()),
        pool_size: 4,
        ssl_mode: trace_commons_server::config::SslMode::Prefer,
        login_resolver_url: Some(SecretString::from(url.clone())),
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: Some(SecretString::from(url)),
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
///
/// `pub(super)`: shared with `pipeline_http_pg_tests`, which needs the same
/// NEAR account fixture for its own admission-gated pipeline tests, rather
/// than keeping a second copy of this SQL.
pub(super) async fn provision_synthetic_near_account(
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

/// Adds a second device -- and its own principal -- to the account
/// `provision_synthetic_near_account` already provisioned under `tenant` and
/// `anchor` (its bare, unprefixed return value). V58 keys
/// `trace_near_provisioned_devices` on `(tenant_id, principal_ref)`, and
/// nothing makes `anchor_hash` unique per principal, so an account can have
/// more than one provisioned device/principal sharing the same anchor -- this
/// is the fixture for that shape: `admission::anchor`'s lookup resolves each
/// principal to the SAME `anchor_hash`, so `admission::reserve`'s
/// completed-lookup (keyed on `(tenant, anchor, submission, body_hash)`, not
/// on principal) cannot by itself distinguish a retry from this device from
/// one from the device `provision_synthetic_near_account` provisioned.
///
/// `pub(super)`: shared with `pipeline_http_pg_tests`.
pub(super) async fn provision_second_device_on_the_same_account(
    db: &PgBackend,
    tenant: &str,
    anchor: &str,
    principal: &str,
) -> String {
    let prefixed = format!("sha256:{anchor}");
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_near_account_anchors
              WHERE tenant_id = $1 AND anchor_hash = $2",
            &[&tenant, &prefixed],
        )
        .await
        .unwrap()
        .get(0);
    let device_bytes: [u8; 32] = sha2::Sha256::digest(Uuid::new_v4().as_bytes()).into();
    let device =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&device_bytes);
    client.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near')",&[&device,&tenant,&base64::engine::general_purpose::STANDARD.encode(device_bytes)]).await.unwrap();
    client
        .execute(
            "INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3)",
            &[&tenant, &account, &principal],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5)",&[&tenant,&principal,&account,&device,&prefixed]).await.unwrap();
    device
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

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn invalid_or_withdrawn_source_session_refuses_before_budget_and_staging() {
    use trace_commons_server::trace_corpus_storage::{TraceCorpusStore, TraceSourceSessionStatus};

    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, mut state, _) = anchor_state(db.clone(), &[(tenant.as_str(), token)]);
    Arc::make_mut(&mut state).require_db_mirror_writes = true;
    Arc::make_mut(&mut state).account_admission = Some(admission::AccountAdmissionConfig {
        policy: trace_commons_server::account_trust::parse_bounded_policy(
            r#"{"version":"z4-source-refusal","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
            &["z4-source-refusal"],
        ).unwrap(),
        lease_seconds: 60,
        providers: None,
    });
    let mut missing = sample_envelope().await;
    make_metadata_only_low_risk(&mut missing);
    missing.source_session = None;
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&missing).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::UNPROCESSABLE_ENTITY,
    );
    assert!(!submission_metadata_path(&state.root, &tenant, missing.submission_id).exists());

    let native = trace_commons_protocol::trace_contribution::SourceSessionIdentity {
        adapter: "opencode".into(),
        native_id: format!("ses_{}", Uuid::new_v4().simple()),
    };
    let digest = session_digest(&canonical_source_session(&native).unwrap());
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account_id: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    let original = insert_account_test_submission_with_status(
        db.as_ref(),
        &tenant,
        &principal_for(token),
        trace_commons_server::trace_corpus_storage::TraceCorpusStatus::Quarantined,
    )
    .await;
    let staged_original = stage_trace_object_file(
        state.as_ref(),
        &tenant,
        TraceCorpusStatus::Quarantined,
        original,
    );
    assert_eq!(
        db.claim_trace_source_session(&tenant, account_id, &digest, original)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    // A second version of the same session. Withdrawing `original` must
    // account for it too: its own audit event, not only its deletion.
    let sibling = insert_account_test_submission_with_status(
        db.as_ref(),
        &tenant,
        &principal_for(token),
        trace_commons_server::trace_corpus_storage::TraceCorpusStatus::Accepted,
    )
    .await;
    assert_eq!(
        db.claim_trace_source_session(&tenant, account_id, &digest, sibling)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    let mut withdraw = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/v1/account/traces/{original}/withdraw"))
        .body(Body::empty())
        .unwrap();
    withdraw
        .headers_mut()
        .extend(account_session_headers(&state, token).await);
    assert_eq!(
        app(state.clone()).oneshot(withdraw).await.unwrap().status(),
        StatusCode::OK
    );
    assert!(!staged_original.exists());
    for id in [original, sibling] {
        let revoke_events: i64 = client
            .query_one(
                "SELECT count(*) FROM trace_audit_events
                  WHERE submission_id = $1 AND action = 'revoke'",
                &[&id],
            )
            .await
            .unwrap()
            .get(0);
        assert_eq!(
            revoke_events, 1,
            "every withdrawn version records its own hash-only revoke event"
        );
    }

    // A plain submission-level tombstone with no source session is refused
    // with its own label, not the source-session one.
    let mut tombstoned = sample_envelope().await;
    make_metadata_only_low_risk(&mut tombstoned);
    tombstoned.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("ses_{}", Uuid::new_v4().simple()),
        },
    );
    db.record_trace_withdrawal(
        &tenant,
        tombstoned.submission_id,
        Utc::now(),
        "received",
        "not_distributed",
    )
    .await
    .unwrap();
    let tombstone_response = post(
        state.clone(),
        "/v1/traces",
        serde_json::to_vec(&tombstoned).unwrap(),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(tombstone_response.status(), StatusCode::CONFLICT);
    let tombstone_body = axum::body::to_bytes(tombstone_response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&tombstone_body).unwrap()["error"],
        "submission_withdrawn"
    );

    // Recreate both service state and PostgreSQL handles, then authenticate a
    // new browser session. The digest must survive every one of those changes.
    let reconnected = admission_pg_admin().await;
    let (_restart_temp, mut restarted, _) = anchor_state(reconnected, &[(tenant.as_str(), token)]);
    Arc::make_mut(&mut restarted).root = state.root.clone();
    Arc::make_mut(&mut restarted).require_db_mirror_writes = true;
    Arc::make_mut(&mut restarted).account_admission = state.account_admission.clone();
    let mut status = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/account/source-sessions/status")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&native).unwrap()))
        .unwrap();
    status
        .headers_mut()
        .extend(account_session_headers(&restarted, token).await);
    let status_response = app(restarted.clone()).oneshot(status).await.unwrap();
    assert_eq!(status_response.status(), StatusCode::OK);
    let status_body = axum::body::to_bytes(status_response.into_body(), 4096)
        .await
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&status_body).unwrap()["status"],
        "withdrawn"
    );

    let mut resumed = sample_envelope_with_user_input("changed content after reconnect").await;
    make_metadata_only_low_risk(&mut resumed);
    set_metadata_only_tool_name(&mut resumed, "changed-resumed-content");
    resumed.source_session = Some(native);
    assert_ne!(resumed.submission_id, original);
    assert_eq!(
        post(
            restarted.clone(),
            "/v1/traces",
            serde_json::to_vec(&resumed).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT,
    );
    assert!(!submission_metadata_path(&restarted.root, &tenant, resumed.submission_id).exists());
    assert!(
        !restarted
            .root
            .join(trace_envelope_object_key(
                &tenant,
                TraceCorpusStatus::Accepted,
                resumed.submission_id
            ))
            .exists()
    );
    let reserved: i64 = client
        .query_one(
            "SELECT count(*) FROM trace_account_admission_submissions WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        reserved, 0,
        "source refusal must not reserve account budget"
    );
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

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn account_replacement_is_default_off_and_validates_offered_evidence() {
    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, mut state, _) = anchor_state(db.clone(), &[(tenant.as_str(), token)]);
    Arc::make_mut(&mut state).require_db_mirror_writes = true;
    let mut envelope = sample_envelope().await;
    make_metadata_only_low_risk(&mut envelope);
    envelope.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    let body = serde_json::to_vec(&envelope).unwrap();
    assert_eq!(
        post(state.clone(), "/v1/traces", body.clone(), HeaderMap::new())
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "flag off retains the zero-evidence refusal"
    );
    Arc::make_mut(&mut state).account_admission = Some(admission::AccountAdmissionConfig {
        policy: trace_commons_server::account_trust::parse_bounded_policy(
            r#"{"version":"account-http-fixture","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
            &["account-http-fixture"],
        ).unwrap(),
        lease_seconds: 60,
        providers: None,
    });
    let mut malformed = HeaderMap::new();
    malformed.insert(
        trace_commons_protocol::admission::EVIDENCE_HEADER,
        HeaderValue::from_static("{"),
    );
    assert_eq!(
        post(state.clone(), "/v1/traces", body.clone(), malformed)
            .await
            .status(),
        StatusCode::FORBIDDEN,
        "offered malformed legacy evidence is still refused"
    );
    let accepted = post(state.clone(), "/v1/traces", body.clone(), HeaderMap::new()).await;
    assert_eq!(
        accepted.status(),
        StatusCode::OK,
        "authenticated account can reserve without interactive evidence"
    );
    let mut changed: serde_json::Value = serde_json::from_slice(&body).unwrap();
    changed["trace_id"] = serde_json::json!(Uuid::new_v4());
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&changed).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT,
        "same UUID with different exact bytes conflicts"
    );
    Arc::make_mut(&mut state).account_admission = Some(admission::AccountAdmissionConfig {
        policy: trace_commons_server::account_trust::parse_bounded_policy(
            r#"{"version":"account-http-fixed","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"fixed","seconds":3600},"growth_rule":"none"}"#,
            &["account-http-fixed"],
        ).unwrap(),
        lease_seconds: 60,
        providers: None,
    });
    let mut fixed_first = sample_envelope().await;
    make_metadata_only_low_risk(&mut fixed_first);
    fixed_first.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&fixed_first).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::OK,
    );
    let mut fixed_second = sample_envelope().await;
    make_metadata_only_low_risk(&mut fixed_second);
    fixed_second.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    let fixed_refusal = post(
        state.clone(),
        "/v1/traces",
        serde_json::to_vec(&fixed_second).unwrap(),
        HeaderMap::new(),
    )
    .await;
    assert_eq!(fixed_refusal.status(), StatusCode::TOO_MANY_REQUESTS);
    let fixed_body = axum::body::to_bytes(fixed_refusal.into_body(), 4096)
        .await
        .unwrap();
    let fixed_error: serde_json::Value = serde_json::from_slice(&fixed_body).unwrap();
    assert_eq!(fixed_error["error"], "account_limit_reached");
    assert!(
        fixed_error["retry_after_seconds"]
            .as_i64()
            .is_some_and(|seconds| (1..=3600).contains(&seconds))
    );
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account_id: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    let used_before_invite: i64 = client.query_one("SELECT sum(cost_used)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap().get(0);
    let invite_hash = format!("sha256:{}", hash_hex(Uuid::new_v4().as_bytes()));
    db.insert_invite_grant(trace_commons_server::db::InviteGrantWrite {
        invite_subject_hash: invite_hash.clone(),
        policy_label: "bounded-elevation".into(),
        tenant_mode: trace_commons_server::trace_invite_registry::InviteTenantMode::Fixed,
        fixed_tenant_id: Some(tenant.clone()),
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
        issuer_display_name: None,
        credit_range: None,
    })
    .await
    .unwrap();
    let redemption_key = Uuid::new_v4();
    let redemption = db
        .redeem_account_invite(&tenant, account_id, &invite_hash, redemption_key)
        .await
        .unwrap();
    assert!(matches!(
        redemption,
        trace_commons_server::db::AccountInviteRedemption::Invited { trust_version: 2 }
    ));
    assert_eq!(
        db.redeem_account_invite(&tenant, account_id, &invite_hash, redemption_key)
            .await
            .unwrap(),
        redemption
    );
    let authority: String = client
        .query_one(
            "SELECT authority FROM trace_account_trust WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account_id],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        authority, "invited",
        "real bounded-first redemption elevates authority"
    );
    let mut status_request = axum::http::Request::builder()
        .uri("/v1/account/contribution-status")
        .body(Body::empty())
        .unwrap();
    status_request
        .headers_mut()
        .extend(account_session_headers(&state, token).await);
    let status_bytes = require_ok(app(state.clone()).oneshot(status_request).await.unwrap()).await;
    let status_json: serde_json::Value = serde_json::from_slice(&status_bytes).unwrap();
    assert_eq!(status_json["authority"], "invited");
    assert_eq!(status_json["ready"], true);
    let mut invited = sample_envelope().await;
    make_metadata_only_low_risk(&mut invited);
    invited.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("test-{}", Uuid::new_v4().simple()),
        },
    );
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&invited).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::OK,
        "invited deterministic-only path needs no fabricated witness"
    );
    let total: i64 = client.query_one("SELECT sum(cost_used)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2", &[&tenant,&account_id]).await.unwrap().get(0);
    assert_eq!(
        total, used_before_invite,
        "invited submission bypasses cumulative debit"
    );
    let consumed: i32 = client
        .query_one(
            "SELECT consumed_uses FROM onboarding_invite_grants WHERE invite_subject_hash=$1",
            &[&invite_hash],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(consumed, 1);
}

#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn rejected_foreign_session_claim_preserves_victim_bytes_after_sibling_withdrawal() {
    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, mut state, _) = anchor_state(db.clone(), &[(tenant.as_str(), token)]);
    Arc::make_mut(&mut state).require_db_mirror_writes = true;
    Arc::make_mut(&mut state).account_admission = Some(admission::AccountAdmissionConfig {
        policy: trace_commons_server::account_trust::parse_bounded_policy(
            r#"{"version":"ownership-fixture","processing_cost_bound":10,"bounded_allowance":100,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
            &["ownership-fixture"],
        ).unwrap(), lease_seconds: 60, providers: None,
    });
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let victim_account = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &victim_account],
        )
        .await
        .unwrap();
    client.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,'principal:victim')", &[&tenant,&victim_account]).await.unwrap();
    let victim = insert_account_test_submission_with_status(
        db.as_ref(),
        &tenant,
        "principal:victim",
        StorageTraceCorpusStatus::Quarantined,
    )
    .await;
    let victim_object =
        stage_trace_object_file(&state, &tenant, TraceCorpusStatus::Quarantined, victim);
    let victim_bytes = std::fs::read(&victim_object).unwrap();
    let before = db
        .get_trace_submission(&tenant, victim)
        .await
        .unwrap()
        .unwrap();
    let mut own = sample_envelope().await;
    make_metadata_only_low_risk(&mut own);
    own.source_session = Some(
        trace_commons_protocol::trace_contribution::SourceSessionIdentity {
            adapter: "opencode".into(),
            native_id: format!("ownership-{}", Uuid::new_v4().simple()),
        },
    );
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&own).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    let mut attack = own.clone();
    attack.submission_id = victim;
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&attack).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::CONFLICT
    );
    let count: i64 = client.query_one("SELECT count(*) FROM trace_submission_sessions WHERE tenant_id=$1 AND submission_id=$2", &[&tenant,&victim]).await.unwrap().get(0);
    assert_eq!(count, 0, "rejected request leaves no victim mapping");
    let mut withdraw = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/v1/account/traces/{}/withdraw", own.submission_id))
        .body(Body::empty())
        .unwrap();
    withdraw
        .headers_mut()
        .extend(account_session_headers(&state, token).await);
    require_ok(app(state.clone()).oneshot(withdraw).await.unwrap()).await;
    assert_eq!(std::fs::read(victim_object).unwrap(), victim_bytes);
    assert_eq!(
        db.get_trace_submission(&tenant, victim)
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert!(
        db.get_trace_withdrawal(&tenant, victim)
            .await
            .unwrap()
            .is_none()
    );
}

/// Kristi's #1021 edge: both accounts held one source session and only the
/// absorbed account had withdrawn it. The merge joins the survivor's accepted
/// version to a withdrawn session, and the withdrawal must win exactly as a
/// withdrawal does: the version is revoked and tombstoned, its bytes are
/// deleted, and it gets its own hash-only revoke audit event.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn merge_completes_a_withdrawal_the_absorbed_account_made() {
    use trace_commons_server::trace_corpus_storage::{TraceCorpusStore, TraceSourceSessionStatus};

    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, mut state, _) = anchor_state(db.clone(), &[(tenant.as_str(), token)]);
    Arc::make_mut(&mut state).require_db_mirror_writes = true;
    let session_headers = account_session_headers(&state, token).await;
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let survivor: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_account_principals
              WHERE tenant_id = $1 AND principal_ref = $2",
            &[&tenant, &principal_for(token)],
        )
        .await
        .unwrap()
        .get(0);
    let absorbed = Uuid::new_v4();
    client
        .execute(
            "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
            &[&tenant, &absorbed],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref)
             VALUES($1,$2,'principal:merge-absorbed')",
            &[&tenant, &absorbed],
        )
        .await
        .unwrap();

    let digest = [0x5du8; 32];
    let absorbed_version = insert_account_test_submission_with_status(
        db.as_ref(),
        &tenant,
        "principal:merge-absorbed",
        StorageTraceCorpusStatus::Accepted,
    )
    .await;
    assert_eq!(
        db.claim_trace_source_session(&tenant, absorbed, &digest, absorbed_version)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );
    db.withdraw_trace_source_session(&tenant, absorbed, absorbed_version, Utc::now())
        .await
        .unwrap()
        .unwrap();
    let survivor_version = insert_account_test_submission_with_status(
        db.as_ref(),
        &tenant,
        &principal_for(token),
        StorageTraceCorpusStatus::Accepted,
    )
    .await;
    let survivor_object = stage_trace_object_file(
        &state,
        &tenant,
        TraceCorpusStatus::Accepted,
        survivor_version,
    );
    assert_eq!(
        db.claim_trace_source_session(&tenant, survivor, &digest, survivor_version)
            .await
            .unwrap(),
        TraceSourceSessionStatus::Active
    );

    let code_hash = format!(
        "sha256:{}{}",
        Uuid::new_v4().simple(),
        Uuid::new_v4().simple()
    );
    client
        .execute(
            "INSERT INTO trace_login_links (tenant_id, link_id, account_id, code_hash,
                created_principal_ref, created_at, expires_at, consumed_at)
             VALUES ($1, $2, $3, $4, 'principal:merge-link', now(), now() + interval '1 hour', NULL)",
            &[&tenant, &Uuid::new_v4(), &absorbed, &code_hash],
        )
        .await
        .unwrap();
    let staged = db
        .stage_merge_proposal(&tenant, survivor, &code_hash)
        .await
        .unwrap()
        .unwrap();
    let mut confirm = axum::http::Request::builder()
        .method("POST")
        .uri("/v1/account/merge/confirm")
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::to_vec(&serde_json::json!({ "proposal_id": staged.proposal_id })).unwrap(),
        ))
        .unwrap();
    confirm.headers_mut().extend(session_headers);
    require_ok(app(state.clone()).oneshot(confirm).await.unwrap()).await;

    assert_eq!(
        db.get_trace_submission(&tenant, survivor_version)
            .await
            .unwrap()
            .unwrap()
            .status,
        StorageTraceCorpusStatus::Revoked,
        "the survivor's version of the withdrawn session is withdrawn at merge"
    );
    assert!(
        db.get_trace_withdrawal(&tenant, survivor_version)
            .await
            .unwrap()
            .is_some(),
        "it carries its own withdrawal tombstone"
    );
    assert!(!survivor_object.exists(), "its bytes are deleted");
    let revoke_events: i64 = client
        .query_one(
            "SELECT count(*) FROM trace_audit_events
              WHERE submission_id = $1 AND action = 'revoke'",
            &[&survivor_version],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(
        revoke_events, 1,
        "it records one hash-only revoke event, as a withdrawal does"
    );
    assert!(
        db.list_incomplete_source_session_withdrawals(&tenant, Some(survivor), &[], 100)
            .await
            .unwrap()
            .is_empty()
    );
}

/// The provider, witness and signer one attested upload needs, built the way
/// `actual_postgres_challenge_witness_ingest_and_terminal_retry` builds them.
struct AttestedUploads {
    provider: ring::signature::Ed25519KeyPair,
    provider_key: String,
    signer: Arc<FixtureSigner>,
    trust: AdmissionProviderTrust,
    witness: witness_service::surface::WitnessService,
}

impl AttestedUploads {
    fn new() -> Self {
        use ring::signature::KeyPair as _;
        let provider = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[9; 32]).unwrap();
        let provider_key = hex::encode(provider.public_key().as_ref());
        let signer = Arc::new(FixtureSigner::new("attested-allowance-witness"));
        let trust = AdmissionProviderTrust::new(
            [provider_key.clone()],
            Vec::new(),
            ["synthetic-model".into()],
            1,
        )
        .unwrap();
        let witness = witness_service::surface::WitnessService::new(
            Arc::new(witness_service::DeterministicRedaction::new(Vec::new())),
            signer.clone(),
            Arc::new(FixtureEnclave(signer.address())),
            1024 * 1024,
        )
        .with_contribution_redactor(Arc::new(
            witness_service::PipelineContributionRedaction::deterministic_only(Vec::new()),
        ))
        .with_admission_provider_trust(trust.clone());
        Self {
            provider,
            provider_key,
            signer,
            trust,
            witness,
        }
    }

    /// A fresh challenge, a provider receipt bound to it, and the witnessed
    /// envelope with its four headers. Installs the witness pin on first use.
    async fn upload(
        &self,
        state: &mut Arc<AppState>,
        source: Option<trace_commons_protocol::trace_contribution::SourceSessionIdentity>,
    ) -> (Vec<u8>, HeaderMap) {
        use trace_commons_protocol::trace_contribution::{
            RawTraceCaptureTurn, RawTraceContribution, TraceContributionEventType,
        };
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
        let request_body = serde_json::json!({"model":"synthetic-model","metadata":{REQUEST_METADATA_KEY:binding.encode().unwrap()},"messages":[{"role":"user","content":"please summarize the successful build"}]}).to_string();
        let response_body = "{\"answer\":\"build succeeded\"}";
        let receipt_text = format!(
            "synthetic-model:{}:{}",
            hash_hex(request_body.as_bytes()),
            hash_hex(response_body.as_bytes())
        );
        let receipt = trace_commons_server::near_attestation::receipt::ReceiptPayload {
            text: receipt_text.clone(),
            signature: hex::encode(self.provider.sign(receipt_text.as_bytes()).as_ref()),
            signing_address: self.provider_key.clone(),
            signing_algo: trace_commons_server::near_attestation::receipt::ReceiptAlgo::Ed25519,
            signature_kind:
                trace_commons_server::near_attestation::receipt::ReceiptSignatureKind::ProviderTee,
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
                pseudonymous_contributor_id: Some("sha256:synthetic-attested".into()),
                ..Default::default()
            },
        );
        raw.submission_id = Uuid::new_v4();
        raw.trace_id = Uuid::new_v4();
        raw.source_session = source;
        raw.ironclaw
            .feature_flags
            .insert("agent".into(), "opencode".into());
        let mut event = raw.events.last().unwrap().clone();
        event.event_id = Uuid::new_v4();
        event.event_type = TraceContributionEventType::HttpExchange;
        event.content = Some(response_body.into());
        event.structured_payload = serde_json::json!({"request":{"method":"POST","body":request_body},"response":{"status":200}});
        raw.events = vec![event];
        let (response, evidence, signature) = self
            .witness
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
        if state.witness_bypass.is_none() {
            Arc::make_mut(state).witness_bypass =
                trace_commons_server::redaction_witness::config::witness_bypass_config_from_values(
                    Some("true"),
                    Some(&self.signer.address()),
                    Some("synthetic-admission-measurement"),
                    Some(&evidence.redaction_policy_version),
                    None,
                )
                .unwrap();
        }
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
        (response.envelope_bytes, headers)
    }
}

fn fresh_source() -> trace_commons_protocol::trace_contribution::SourceSessionIdentity {
    trace_commons_protocol::trace_contribution::SourceSessionIdentity {
        adapter: "opencode".into(),
        native_id: format!("test-{}", Uuid::new_v4().simple()),
    }
}

async fn error_label(response: axum::response::Response) -> (StatusCode, serde_json::Value) {
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default()["error"].clone(),
    )
}

/// The per-account hourly submission ceiling bounds an account whatever its
/// admission authority -- bounded, attested or invited -- and runs before
/// admission reserves: an upload it refuses leaves no admission row and
/// charges nothing. It does not change the account budget either: an
/// attested upload under the ceiling still debits nothing.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn the_hourly_ceiling_bounds_every_authority_and_charges_nothing() {
    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (_temp, mut state, _) = anchor_state(db.clone(), &[(tenant.as_str(), token)]);
    let uploads = AttestedUploads::new();
    {
        let state = Arc::make_mut(&mut state);
        state.require_db_mirror_writes = true;
        state.accept_medium_risk_submissions = true;
        state.admission = Some(admission::AdmissionConfig {
            limits: AdmissionLimits {
                window_attempts: 1,
                account_cost_limit: 100,
                global_cost_limit: 1000,
                processing_cost_bound: 10,
                lease_seconds: 60,
                challenge_ttl_seconds: 60,
            },
            providers: uploads.trust.clone(),
        });
        state.account_admission = Some(admission::AccountAdmissionConfig {
            policy: trace_commons_server::account_trust::parse_bounded_policy(
                r#"{"version":"hourly-ceiling","processing_cost_bound":10,"bounded_allowance":100,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
                &["hourly-ceiling"],
            )
            .unwrap(),
            lease_seconds: 60,
            providers: Some(uploads.trust.clone()),
        });
        state.submission_quota.max_per_account_per_hour = 2;
    }
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    let spent = || async {
        client
            .query_one(
                "SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1",
                &[&tenant],
            )
            .await
            .unwrap()
            .get::<_, i64>(0)
    };
    let admission_rows = |submission: Uuid| {
        let client = &client;
        let tenant = &tenant;
        async move {
            client
                .query_one(
                    "SELECT (SELECT COUNT(*) FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2)
                          + (SELECT COUNT(*) FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2)",
                    &[tenant, &submission],
                )
                .await
                .unwrap()
                .get::<_, i64>(0)
        }
    };
    let refused_by_the_ceiling = |response: axum::response::Response| async move {
        let retry_after = response
            .headers()
            .get(axum::http::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<i64>().ok());
        let (status, label) = error_label(response).await;
        assert_eq!(
            (status, label),
            (
                StatusCode::TOO_MANY_REQUESTS,
                serde_json::json!(SUBMIT_RATE_LIMITED_LABEL)
            )
        );
        let retry_after = retry_after.expect("the refusal carries Retry-After");
        assert!((1..=3600).contains(&retry_after), "{retry_after}");
    };
    let unattested = || async {
        let mut envelope = sample_envelope().await;
        make_metadata_only_low_risk(&mut envelope);
        envelope.source_session = Some(fresh_source());
        envelope
    };

    // A bounded upload is charged as before.
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&unattested().await).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(spent().await, 10);

    // An attested upload under the ceiling debits nothing, as before.
    let (attested, attested_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    let attested_receipt = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            attested.clone(),
            attested_headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(spent().await, 10, "an attested upload debits nothing");

    // The third upload of the hour is refused, attested or not, before
    // admission reserves: no admission row, no submission row, no charge.
    let (over, over_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    let over_submission = serde_json::from_slice::<TraceContributionEnvelope>(&over)
        .unwrap()
        .submission_id;
    refused_by_the_ceiling(post(state.clone(), "/v1/traces", over, over_headers).await).await;
    assert_eq!(admission_rows(over_submission).await, 0);
    assert_eq!(
        spent().await,
        10,
        "a refusal by the ceiling charges nothing"
    );

    // A retry of a recorded submission is not counted against itself.
    assert_eq!(
        require_ok(post(state.clone(), "/v1/traces", attested, attested_headers).await).await,
        attested_receipt,
        "a completed retry is still a receipt read at the ceiling"
    );

    // An invited account is bounded by the same ceiling.
    let invite_hash = format!("sha256:{}", hash_hex(Uuid::new_v4().as_bytes()));
    client.execute("INSERT INTO trace_account_trust(tenant_id,account_id,authority,trust_version) VALUES($1,$2,'invited',1) ON CONFLICT(tenant_id,account_id) DO UPDATE SET authority='invited',trust_version=1", &[&tenant,&account]).await.unwrap();
    client.execute("INSERT INTO trace_account_invite_grants(tenant_id,account_id,invite_subject_hash,trust_version) VALUES($1,$2,$3,1)", &[&tenant,&account,&invite_hash]).await.unwrap();
    let invited = unattested().await;
    refused_by_the_ceiling(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&invited).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(admission_rows(invited.submission_id).await, 0);

    // A ceiling of zero bounds nothing.
    Arc::make_mut(&mut state)
        .submission_quota
        .max_per_account_per_hour = 0;
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&invited).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(spent().await, 10, "an invited upload debits nothing");
}

/// Owner decision of 2026-10-11: an upload whose NEAR AI admission evidence
/// verifies is admitted without debiting the bounded account allowance, and
/// one with no `source_session` takes the V59 evidence ledger it took before
/// account admission existed. An unattested upload is capped exactly as
/// before.
///
/// Connects as the owning role on purpose. The restricted-role grant matrix
/// is the end-to-end test above; a second granting test in this file would
/// race it on the tables' `relacl` (#833).
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn attested_uploads_are_not_capped_by_the_account_allowance() {
    let db = admission_pg_admin().await;
    let token = "admission-fixture-token";
    let fresh_token = "attested-allowance-fresh-account";
    let (tenant, anchor, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let (fresh_tenant, _, _) =
        provision_synthetic_near_account(&db, &principal_for(fresh_token)).await;
    let (_temp, mut state, _) = anchor_state(
        db.clone(),
        &[
            (tenant.as_str(), token),
            (fresh_tenant.as_str(), fresh_token),
        ],
    );
    let uploads = AttestedUploads::new();
    {
        let state = Arc::make_mut(&mut state);
        state.require_db_mirror_writes = true;
        state.accept_medium_risk_submissions = true;
        state.admission = Some(admission::AdmissionConfig {
            limits: AdmissionLimits {
                window_attempts: 1,
                account_cost_limit: 100,
                // The V59 global budget is one row for the whole database,
                // frozen at its first caller's limit; match the test above.
                global_cost_limit: 1000,
                processing_cost_bound: 10,
                lease_seconds: 60,
                challenge_ttl_seconds: 60,
            },
            providers: uploads.trust.clone(),
        });
        state.account_admission = Some(admission::AccountAdmissionConfig {
            policy: trace_commons_server::account_trust::parse_bounded_policy(
                r#"{"version":"attested-allowance","processing_cost_bound":10,"bounded_allowance":10,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
                &["attested-allowance"],
            )
            .unwrap(),
            lease_seconds: 60,
            providers: Some(uploads.trust.clone()),
        });
    }
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account_id: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);
    let spent = || async {
        client
            .query_one(
                "SELECT COALESCE(sum(cost_used),0)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1",
                &[&tenant],
            )
            .await
            .unwrap()
            .get::<_, i64>(0)
    };
    let account_row = |submission: Uuid| {
        let client = &client;
        let tenant = &tenant;
        async move {
            client
                .query_opt(
                    "SELECT authority_kind,last_charged,status FROM trace_account_admission_submissions WHERE tenant_id=$1 AND submission_id=$2",
                    &[tenant, &submission],
                )
                .await
                .unwrap()
                .map(|row| {
                    (
                        row.get::<_, String>(0),
                        row.get::<_, bool>(1),
                        row.get::<_, String>(2),
                    )
                })
        }
    };
    let legacy_row = |submission: Uuid| {
        let client = &client;
        let tenant = &tenant;
        async move {
            client
                .query_opt(
                    "SELECT status FROM trace_admission_submissions WHERE tenant_id=$1 AND submission_id=$2",
                    &[tenant, &submission],
                )
                .await
                .unwrap()
                .map(|row| row.get::<_, String>(0))
        }
    };
    let submission_of = |bytes: &[u8]| {
        serde_json::from_slice::<TraceContributionEnvelope>(bytes)
            .unwrap()
            .submission_id
    };

    // An unattested upload spends the whole allowance; the next is refused.
    let mut unattested = sample_envelope().await;
    make_metadata_only_low_risk(&mut unattested);
    unattested.source_session = Some(fresh_source());
    require_ok(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&unattested).unwrap(),
            HeaderMap::new(),
        )
        .await,
    )
    .await;
    assert_eq!(spent().await, 10);
    let mut over = sample_envelope().await;
    make_metadata_only_low_risk(&mut over);
    over.source_session = Some(fresh_source());
    assert_eq!(
        error_label(
            post(
                state.clone(),
                "/v1/traces",
                serde_json::to_vec(&over).unwrap(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        (
            StatusCode::TOO_MANY_REQUESTS,
            serde_json::json!("account_limit_reached")
        ),
        "an unattested upload past the allowance is refused as before"
    );

    // An attested upload past the exhausted allowance is admitted, recorded
    // as attested, and debits nothing.
    let (attested, attested_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    let attested_receipt = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            attested.clone(),
            attested_headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(spent().await, 10, "an attested upload debits nothing");
    assert_eq!(
        account_row(submission_of(&attested)).await,
        Some(("attested".into(), false, "completed".into()))
    );
    assert_eq!(
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                attested.clone(),
                attested_headers.clone()
            )
            .await
        )
        .await,
        attested_receipt,
        "a completed attested retry is a receipt read"
    );

    // Offered evidence that does not verify is refused as before, and
    // reserves nothing: no signature, a wrong signature, or no witness.
    let (refused, refused_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    let mut bad_signature = refused_headers.clone();
    bad_signature.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        HeaderValue::from_static("invalid"),
    );
    let mut partial = refused_headers.clone();
    partial.remove(trace_commons_protocol::admission::SIGNATURE_HEADER);
    let mut unwitnessed = refused_headers.clone();
    unwitnessed.remove(trace_commons_server::redaction_witness::request::CERTIFICATE_HEADER);
    for headers in [bad_signature, partial, unwitnessed] {
        assert_eq!(
            error_label(post(state.clone(), "/v1/traces", refused.clone(), headers).await).await,
            (
                StatusCode::FORBIDDEN,
                serde_json::json!("admission_refused")
            ),
        );
    }
    assert_eq!(account_row(submission_of(&refused)).await, None);
    assert_eq!(spent().await, 10);

    // Release before processing, then a crashed (expired) lease: both
    // recover without a debit, and the retry keeps the attested authority.
    let (recovered, recovered_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    let recovered_envelope: TraceContributionEnvelope = serde_json::from_slice(&recovered).unwrap();
    let mut attempt = admission::reserve(
        &state,
        &authenticate_ctx(&state, &{
            let mut auth = recovered_headers.clone();
            auth.insert(
                AUTHORIZATION,
                HeaderValue::from_static("Bearer admission-fixture-token"),
            );
            auth
        })
        .unwrap(),
        &recovered_headers,
        &recovered,
        &recovered_envelope,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(
        account_row(recovered_envelope.submission_id).await,
        Some(("attested".into(), false, "reserved".into()))
    );
    attempt.finish(db.as_ref(), false).await.unwrap();
    drop(attempt);
    assert_eq!(
        account_row(recovered_envelope.submission_id).await,
        Some(("attested".into(), false, "released".into()))
    );
    assert_eq!(
        spent().await,
        10,
        "releasing an attested lease refunds nothing"
    );
    client
        .execute(
            "UPDATE trace_account_admission_submissions SET lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &recovered_envelope.submission_id],
        )
        .await
        .unwrap();
    let recovered_receipt = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            recovered.clone(),
            recovered_headers.clone(),
        )
        .await,
    )
    .await;
    client
        .execute(
            "UPDATE trace_account_admission_submissions SET status='reserved',lease_expires_at=clock_timestamp()-interval '1 second' WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant, &recovered_envelope.submission_id],
        )
        .await
        .unwrap();
    assert_eq!(
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                recovered.clone(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        recovered_receipt,
        "an expired attested lease recovers on the authority its row records"
    );
    assert_eq!(
        account_row(recovered_envelope.submission_id).await,
        Some(("attested".into(), false, "completed".into()))
    );
    assert_eq!(spent().await, 10, "attested recovery never debits");

    // The freeze rule guards the allowance, which an attested upload does
    // not touch: a budget row frozen by a policy mismatch refuses an
    // unattested upload but not an attested one.
    client
        .execute(
            "UPDATE trace_account_admission_budget SET cost_limit=cost_limit+1 WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();
    let mut frozen = sample_envelope().await;
    make_metadata_only_low_risk(&mut frozen);
    frozen.source_session = Some(fresh_source());
    assert_eq!(
        post(
            state.clone(),
            "/v1/traces",
            serde_json::to_vec(&frozen).unwrap(),
            HeaderMap::new()
        )
        .await
        .status(),
        StatusCode::FORBIDDEN
    );
    let (thawed, thawed_headers) = uploads.upload(&mut state, Some(fresh_source())).await;
    require_ok(post(state.clone(), "/v1/traces", thawed, thawed_headers).await).await;
    client
        .execute(
            "UPDATE trace_account_admission_budget SET cost_limit=cost_limit-1 WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();

    // No source_session: verified evidence takes the V59 ledger, exactly as
    // with account admission off. Nothing is written to the account ledger.
    let legacy_spent = || async {
        client
            .query_one(
                "SELECT COALESCE(sum(cost_bound_used),0)::bigint FROM trace_admission_accounts WHERE tenant_id=$1 AND anchor_hash=$2",
                &[&tenant, &anchor],
            )
            .await
            .unwrap()
            .get::<_, i64>(0)
    };
    let (sourceless, sourceless_headers) = uploads.upload(&mut state, None).await;
    let sourceless_receipt = require_ok(
        post(
            state.clone(),
            "/v1/traces",
            sourceless.clone(),
            sourceless_headers.clone(),
        )
        .await,
    )
    .await;
    assert_eq!(
        legacy_row(submission_of(&sourceless)).await.as_deref(),
        Some("completed")
    );
    assert_eq!(account_row(submission_of(&sourceless)).await, None);
    assert_eq!(
        legacy_spent().await,
        10,
        "the V59 ledger bounds it as today"
    );
    assert_eq!(spent().await, 10);
    assert_eq!(
        require_ok(
            post(
                state.clone(),
                "/v1/traces",
                sourceless.clone(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        sourceless_receipt,
        "a completed V59 retry is a receipt read after account admission too"
    );

    // No source_session and no evidence stays 422; no source_session and
    // evidence that does not verify is refused and reserves nothing.
    let mut bare = sample_envelope().await;
    make_metadata_only_low_risk(&mut bare);
    bare.source_session = None;
    assert_eq!(
        error_label(
            post(
                state.clone(),
                "/v1/traces",
                serde_json::to_vec(&bare).unwrap(),
                HeaderMap::new()
            )
            .await
        )
        .await,
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            serde_json::json!("source_session_invalid")
        ),
    );
    let (sourceless_bad, mut sourceless_bad_headers) = uploads.upload(&mut state, None).await;
    sourceless_bad_headers.insert(
        trace_commons_protocol::admission::SIGNATURE_HEADER,
        HeaderValue::from_static("invalid"),
    );
    assert_eq!(
        error_label(
            post(
                state.clone(),
                "/v1/traces",
                sourceless_bad.clone(),
                sourceless_bad_headers
            )
            .await
        )
        .await,
        (
            StatusCode::FORBIDDEN,
            serde_json::json!("admission_refused")
        ),
    );
    assert_eq!(legacy_row(submission_of(&sourceless_bad)).await, None);
    assert_eq!(account_row(submission_of(&sourceless_bad)).await, None);

    // contribution-status still reports the unattested allowance: a fresh
    // account is ready, the exhausted one is not.
    for (who, ready) in [(fresh_token, true), (token, false)] {
        let mut request = axum::http::Request::builder()
            .uri("/v1/account/contribution-status")
            .body(Body::empty())
            .unwrap();
        request
            .headers_mut()
            .extend(account_session_headers(&state, who).await);
        let body = require_ok(app(state.clone()).oneshot(request).await.unwrap()).await;
        let status: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(status["authority"], "bounded");
        assert_eq!(status["ready"], ready, "{who}");
    }

    // Account admission off: the same sourceless attested upload takes the
    // same V59 path it always did.
    Arc::make_mut(&mut state).account_admission = None;
    let (off, off_headers) = uploads.upload(&mut state, None).await;
    require_ok(post(state.clone(), "/v1/traces", off.clone(), off_headers).await).await;
    assert_eq!(
        legacy_row(submission_of(&off)).await.as_deref(),
        Some("completed")
    );
    assert_eq!(account_row(submission_of(&off)).await, None);
    assert_eq!(legacy_spent().await, 20);
    assert_eq!(spent().await, 10);
}

/// Rule 7 of the earned-trust spec ("How admission reads trust"): under a
/// `tiered-v1` admission policy, contribution-status adds the caller's own
/// growth view beside `authority:"bounded"`. A fresh applied evaluation shows
/// its tier, multiplier, the period's allowance, spend, remaining and reset,
/// and the next tier's requirements against the evaluation's counts. A stale
/// one shows tier 0 at the base allowance. `growth_rule:"none"` adds nothing:
/// the body is exactly the five fields it always had. `ready` is unchanged.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn contribution_status_reports_the_accounts_own_growth() {
    let db = admission_pg_admin().await;
    let token = "growth-status-fixture-token";
    let legacy_token = "growth-status-legacy-token";
    let (tenant, _, _) = provision_synthetic_near_account(&db, &principal_for(token)).await;
    let legacy_tenant = format!("tenant-{}", Uuid::new_v4());
    let (_temp, mut state, _) = anchor_state(
        db.clone(),
        &[
            (tenant.as_str(), token),
            (legacy_tenant.as_str(), legacy_token),
        ],
    );
    let version = format!(
        "growth-status-{}",
        &Uuid::new_v4().simple().to_string()[..12]
    );
    let tiered = serde_json::json!({
        "version": version,
        "processing_cost_bound": 10,
        "bounded_allowance": 10,
        "period": {"mode": "fixed", "seconds": 3600},
        "growth_rule": "tiered-v1",
        "growth": {
            "window_seconds": 84 * 86400,
            "weekly_cap": 10,
            "q_min_micros": null,
            "penalty_cooldown_seconds": 0,
            "evaluation_max_age_seconds": 3600,
            "allowance_ceiling": 60,
            "evaluator_versions": ["gate-v1"],
            "dedup_signal_versions": ["events.v2+simhash.v2"],
            "tiers": [
                {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1},
                {"units": 2, "active_weeks": 1, "age_seconds": 0, "multiplier": 3},
                {"units": 5, "active_weeks": 2, "age_seconds": 1209600, "multiplier": 6,
                 "attested_units": 1}
            ]
        }
    })
    .to_string();
    let set_policy = |state: &mut Arc<AppState>, json: &str, growth: bool| {
        let state = Arc::make_mut(state);
        state.account_admission = Some(admission::AccountAdmissionConfig {
            policy: trace_commons_server::account_trust::parse_bounded_policy(
                json,
                &[version.as_str()],
            )
            .unwrap(),
            lease_seconds: 60,
            providers: None,
        });
        state.account_trust_applied_policy = growth.then(|| {
            Arc::new(
                trace_commons_server::account_trust_rule::parse_growth_policy(
                    json,
                    &[version.as_str()],
                )
                .unwrap(),
            )
        });
    };
    let status = |state: Arc<AppState>, who: &'static str| async move {
        let mut request = axum::http::Request::builder()
            .uri("/v1/account/contribution-status")
            .body(Body::empty())
            .unwrap();
        request
            .headers_mut()
            .extend(account_session_headers(&state, who).await);
        let response = app(state).oneshot(request).await.unwrap();
        let code = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        (code, bytes)
    };
    let upload_allowance = |state: Arc<AppState>, who: &'static str| async move {
        let mut request = axum::http::Request::builder()
            .uri("/v1/account/upload-allowance")
            .body(Body::empty())
            .unwrap();
        request
            .headers_mut()
            .extend(account_session_headers(&state, who).await);
        let response = app(state).oneshot(request).await.unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .unwrap(),
            "no-store"
        );
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
    };
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    let account_id: Uuid = client
        .query_one(
            "SELECT account_id FROM trace_accounts WHERE tenant_id=$1",
            &[&tenant],
        )
        .await
        .unwrap()
        .get(0);

    // growth_rule none: exactly the five fields, and no growth read.
    let none = format!(
        r#"{{"version":"{version}","processing_cost_bound":10,"bounded_allowance":10,"period":{{"mode":"fixed","seconds":3600}},"growth_rule":"none"}}"#
    );
    set_policy(&mut state, &none, false);
    let (code, bytes) = status(state.clone(), token).await;
    assert_eq!(code, StatusCode::OK);
    let expected = serde_json::json!({
        "authority": "bounded",
        "policy_version": version,
        "ready": true,
        "refusal_label": null,
        "retry_after_seconds": null,
    });
    assert_eq!(
        std::str::from_utf8(&bytes).unwrap(),
        expected.to_string(),
        "growth_rule none answers byte for byte as before"
    );

    // tiered-v1 with no evaluation yet: tier 0 at the base allowance, no
    // counts, and the next tier's requirements.
    set_policy(&mut state, &tiered, true);
    let (code, bytes) = status(state.clone(), token).await;
    assert_eq!(code, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authority"], "bounded");
    assert_eq!(body["ready"], true);
    assert_eq!(body["allowance_tier"], 0);
    assert_eq!(body["allowance_multiplier"], 1);
    assert_eq!(body["effective_allowance"], 10);
    assert_eq!(body["allowance_spent"], 0);
    assert_eq!(body["allowance_remaining"], 10);
    assert!(body["period_resets_at"].is_string(), "{body}");
    assert!(body["evaluated_at"].is_null());
    assert!(body["allowance_tier_changed_at"].is_null());
    assert_eq!(body["next_tier"]["tier"], 1);
    assert_eq!(body["next_tier"]["multiplier"], 3);
    assert_eq!(body["next_tier"]["effective_allowance"], 30);
    assert_eq!(
        body["next_tier"]["required"],
        serde_json::json!({"units": 2, "active_weeks": 1, "age_seconds": 0, "attested_units": 0})
    );
    assert!(body["next_tier"]["current"].is_null(), "{body}");

    assert_eq!(
        upload_allowance(state.clone(), token).await,
        serde_json::json!({"allowance":null}),
        "no client daily limit is advertised"
    );

    // A fresh applied evaluation at tier 1, and 10 already spent this period.
    client
        .query_one("SELECT trace_account_trust_enable_external_growth()", &[])
        .await
        .unwrap();
    let generation: i64 = client
        .query_one(
            "SELECT generation FROM trace_account_trust_frontiers WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account_id],
        )
        .await
        .unwrap()
        .get(0);
    let as_of: DateTime<Utc> = client
        .query_one(
            "SELECT date_trunc('second', clock_timestamp()) - interval '60 seconds'",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    client
        .execute(
            "INSERT INTO trace_account_trust_evaluations(tenant_id,evaluation_id,account_id,
                growth_policy_version,mode,as_of,tier,previous_tier,effective_allowance,units,
                units_capped,attested_units,active_weeks,age_weeks,age_seconds,netted_withdrawn,
                netted_revoked,netted_quarantined,penalty_active,facts_digest,input_generation)
             VALUES($1,$2,$3,$4,'applied',$5,1,0,30,3,0,1,1,0,86400,0,0,0,FALSE,$6,$7)",
            &[
                &tenant,
                &Uuid::new_v4(),
                &account_id,
                &version,
                &as_of,
                &format!("sha256:{}", "4".repeat(64)),
                &generation,
            ],
        )
        .await
        .unwrap();
    client
        .execute(
            "INSERT INTO trace_account_admission_budget(tenant_id,account_id,period_id,
                policy_version,cost_limit,cost_bound,cost_used)
             VALUES($1,$2,$3 || ':fixed:3600:' ||
                floor(extract(epoch FROM clock_timestamp()) / 3600)::bigint::text,$3,10,10,10)",
            &[&tenant, &account_id, &version],
        )
        .await
        .unwrap();
    let (code, bytes) = status(state.clone(), token).await;
    assert_eq!(code, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["authority"], "bounded");
    assert_eq!(body["ready"], true, "20 of 30 left");
    assert_eq!(body["allowance_tier"], 1);
    assert_eq!(body["allowance_multiplier"], 3);
    assert_eq!(body["effective_allowance"], 30);
    assert_eq!(body["allowance_spent"], 10);
    assert_eq!(body["allowance_remaining"], 20);
    let resets: DateTime<Utc> = body["period_resets_at"].as_str().unwrap().parse().unwrap();
    assert_eq!(resets.timestamp() % 3600, 0, "a fixed-period boundary");
    assert!(resets > Utc::now() && resets <= Utc::now() + chrono::Duration::seconds(3600));
    let evaluated: DateTime<Utc> = body["evaluated_at"].as_str().unwrap().parse().unwrap();
    assert_eq!(evaluated, as_of);
    let changed: DateTime<Utc> = body["allowance_tier_changed_at"]
        .as_str()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(changed, as_of);
    assert_eq!(body["next_tier"]["tier"], 2);
    assert_eq!(body["next_tier"]["multiplier"], 6);
    assert_eq!(body["next_tier"]["effective_allowance"], 60);
    assert_eq!(
        body["next_tier"]["required"],
        serde_json::json!({"units": 5, "active_weeks": 2, "age_seconds": 1209600, "attested_units": 1})
    );
    assert_eq!(
        body["next_tier"]["current"],
        serde_json::json!({"units": 3, "active_weeks": 1, "age_seconds": 86400, "attested_units": 1})
    );
    // No raw identifier of the account or tenant in the body.
    let text = std::str::from_utf8(&bytes).unwrap();
    assert!(!text.contains(&tenant) && !text.contains(&account_id.to_string()));

    assert_eq!(
        upload_allowance(state.clone(), token).await,
        serde_json::json!({"allowance":null}),
        "earned trust grows the server budget; no client daily limit is advertised"
    );
    let unchanged_spend: i64 = client.query_one(
        "SELECT sum(cost_used)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2",
        &[&tenant, &account_id],
    ).await.unwrap().get(0);
    assert_eq!(
        unchanged_spend, 10,
        "reading the allowance does not erase server spend"
    );

    // Stale: older than evaluation_max_age_seconds reads as tier 0, and the
    // 10 already spent leaves nothing at the base allowance.
    client
        .execute(
            "UPDATE trace_account_trust_evaluations SET as_of=as_of - interval '2 hours'
              WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account_id],
        )
        .await
        .unwrap();
    let (code, bytes) = status(state.clone(), token).await;
    assert_eq!(code, StatusCode::OK);
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["ready"], false);
    assert_eq!(body["refusal_label"], "account_limit_reached");
    assert_eq!(body["allowance_tier"], 0);
    assert_eq!(body["allowance_multiplier"], 1);
    assert_eq!(body["effective_allowance"], 10);
    assert_eq!(body["allowance_spent"], 10);
    assert_eq!(body["allowance_remaining"], 0);
    assert!(body["evaluated_at"].is_null());
    assert!(body["allowance_tier_changed_at"].is_null());
    assert_eq!(body["next_tier"]["tier"], 1);
    assert!(body["next_tier"]["current"].is_null());

    assert_eq!(
        upload_allowance(state.clone(), token).await,
        serde_json::json!({"allowance":null}),
        "stale earned trust advertises no client daily limit either"
    );

    // With both admission ledgers off, the independent trust pool neither
    // reserves nor debits, and no client daily limit is advertised.
    // Optional explicit isolated worker login exercises the operator bootstrap's
    // real status-read permissions while auth/source storage stays on admin.
    let independent_database =
        if let Ok(url) = std::env::var("TRACE_COMMONS_CLIENT_UPLOAD_TRUST_TEST_RUNTIME_URL") {
            let source_url = std::env::var("TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL").unwrap();
            let source = DatabaseConfig {
                url: SecretString::from(source_url),
                pool_size: 4,
                ssl_mode: trace_commons_server::config::SslMode::Prefer,
                login_resolver_url: None,
                gate_driver_url: None,
                pii_backstop_driver_url: None,
                invite_registry_url: None,
            };
            let config = account_trust_growth_routes::trust_database_config_from_values(
                Some(&source),
                Some(&url),
                true,
                false,
                false,
                false,
                true,
            )
            .unwrap()
            .unwrap();
            Some(Arc::new(PgBackend::new(&config).await.unwrap()) as Arc<dyn Database>)
        } else {
            None
        };
    let mut independent = state.clone();
    let independent_mut = Arc::make_mut(&mut independent);
    independent_mut.client_upload_trust_database = independent_database;
    independent_mut.admission = None;
    independent_mut.account_admission = None;
    client.execute(
        "UPDATE trace_account_trust_evaluations SET as_of=clock_timestamp() WHERE tenant_id=$1 AND account_id=$2",
        &[&tenant, &account_id],
    ).await.unwrap();
    assert_eq!(
        upload_allowance(independent.clone(), token).await,
        serde_json::json!({"allowance":null}),
        "no client daily limit with both admission ledgers off"
    );
    let (code, bytes) = status(independent.clone(), token).await;
    assert_eq!(code, StatusCode::OK);
    let legacy_status: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(legacy_status["authority"], "legacy_evidence");
    assert!(
        legacy_status["policy_version"].is_null(),
        "client trust does not enable account admission"
    );
    assert!(independent.admission.is_none() && independent.account_admission.is_none());
    let unchanged_spend: i64 = client.query_one(
        "SELECT sum(cost_used)::bigint FROM trace_account_admission_budget WHERE tenant_id=$1 AND account_id=$2",
        &[&tenant, &account_id],
    ).await.unwrap().get(0);
    assert_eq!(unchanged_spend, 10);
    client.execute(
        "UPDATE trace_account_trust_evaluations SET as_of=as_of - interval '2 hours' WHERE tenant_id=$1 AND account_id=$2",
        &[&tenant, &account_id],
    ).await.unwrap();
    assert_eq!(
        upload_allowance(independent, token).await,
        serde_json::json!({"allowance":null}),
        "a stale evaluation advertises no client daily limit either"
    );

    // A legacy-namespace session is still refused before any read.
    let (code, bytes) = status(state.clone(), legacy_token).await;
    assert_eq!(code, StatusCode::FORBIDDEN);
    let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(body["error"], "account_identity_unlinked");
}
