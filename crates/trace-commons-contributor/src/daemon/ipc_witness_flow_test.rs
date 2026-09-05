// Real native window-review pipeline; only Intel DCAP crypto uses a one-shot fixture.
// Does not prove a live provider receipt, NEAR wallet ceremony, or v2 admission.
use super::*;
use crate::witness::transport::{WITNESS_CERTIFICATE_HEADER, WITNESS_SIGNATURE_HEADER};
use axum::{
    Json, Router,
    extract::Query,
    routing::{get, post},
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

#[tokio::test]
async fn provisioned_near_window_review_builds_over_http_and_uploads_exact_approved_bytes() {
    let body = "{\"type\":\"user\",\"message\":{\"role\":\"user\",\"content\":\"synthetic onboarding task\"},\"cwd\":\"/synthetic/project\",\"timestamp\":\"2026-08-08T10:00:00Z\",\"sessionId\":\"sess-1\",\"uuid\":\"a1\"}\n";
    let (s, id, dir) = shared_with_session(body);
    let project = dir.path().join("project");
    std::fs::create_dir(&project).unwrap();
    let path = project.join("sess-1.jsonl");
    std::fs::write(&path, body).unwrap();
    {
        let mut queue = s.queue.lock().unwrap();
        let mut entry = queue.get(id).unwrap().clone();
        entry.path = path;
        entry.session_hash = crate::source::session_hash(body.as_bytes());
        *queue = super::super::super::queue::Queue::default();
        queue.upsert(entry, 500).unwrap();
    }
    {
        let mut settings = s.settings.lock().unwrap();
        settings.claude_source = Some(super::super::super::settings::SourceDeclaration::Watch {
            path: dir.path().to_path_buf(),
        });
        settings.ironwire_attested_bodies = true;
    }
    let device = crate::identity::DeviceIdentity::load_or_generate(&s.store).unwrap();
    let (_, signer) = crate::witness::transport::signed_fixture(Vec::new());
    let calls = Arc::new(Mutex::new(Vec::<String>::new()));
    let exact = Arc::new(Mutex::new(Vec::<u8>::new()));
    let uploads = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let certified_headers = Arc::new(Mutex::new(None::<(String, String)>));
    let guards = Arc::new(Mutex::new(Vec::new()));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.device_key_id = device.device_key_id.clone();
    cfg.user_subject = device.device_key_id.clone();
    cfg.instance_id.clear();
    cfg.tenant_id = format!("near-{}", "ab".repeat(32));
    cfg.issuer_url = base.clone();
    cfg.ingest_url = base.clone();
    cfg.audience = "trace-commons-upload".into();
    cfg.allowed_hosts = Some("127.0.0.1".into());
    cfg.consent_scopes = vec!["debugging_evaluation".into()];
    cfg.inference_receipt_check_attestation = true;
    cfg.witness = Some(crate::config::WitnessSettings {
        url: base.clone(),
        signing_address: signer.clone(),
        expected_measurements: vec![format!("mrtd={}", "aa".repeat(48))],
        admission_evidence: true,
    });
    s.store.save_config(&cfg).unwrap();
    let claim = {
        let calls = calls.clone();
        let public_key = device.public_key_b64.clone();
        let cfg = cfg.clone();
        move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
            let calls = calls.clone();
            let public_key = public_key.clone();
            let cfg = cfg.clone();
            async move {
                use base64::Engine as _;
                let base64 = base64::engine::general_purpose::STANDARD;
                let key = base64.decode(public_key).unwrap();
                let signature = base64
                    .decode(headers["x-trace-device-signature"].as_bytes())
                    .unwrap();
                ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
                    .verify(&body, &signature)
                    .unwrap();
                assert_eq!(headers["x-trace-device-key-id"], cfg.device_key_id);
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                assert_eq!(request["tenant_id"], cfg.tenant_id);
                assert_eq!(
                    request["consent_scopes"],
                    serde_json::json!(["debugging_evaluation"])
                );
                calls.lock().unwrap().push("claim".into());
                Json(serde_json::json!({
                    "access_token": "synthetic-claim", "token_type": "Bearer",
                    "expires_at": Utc::now() + chrono::Duration::seconds(300), "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging", "evaluation", "aggregate_analytics"]
                }))
            }
        }
    };
    let router = Router::new()
        .route("/v1/trace-upload-claim", post(claim))
        .route("/v1/attestation", get({ let calls = calls.clone(); let guards = guards.clone(); move |Query(query): Query<HashMap<String,String>>| { let calls=calls.clone(); let guards=guards.clone(); let signer=signer.clone(); async move {
            calls.lock().unwrap().push("attestation".into());
            let mut report_data = vec![0u8;64];
            report_data[..8].copy_from_slice(crate::witness::WITNESS_QUOTE_DOMAIN);
            report_data[8..28].copy_from_slice(&trace_commons_attestation::address::decode_address(&signer).unwrap());
            report_data[28..60].copy_from_slice(&hex::decode(&query["nonce"]).unwrap());
            let quote = trace_commons_attestation::quote::VerifiedQuote {report_data,mrtd:"aa".repeat(48),mr_config_id:"00".repeat(48),rtmr:std::array::from_fn(|_|"00".repeat(48)),tcb_status:"UpToDate".into(),advisory_ids:Vec::new()};
            let guard=crate::witness::verify::register_quote_fixture(quote);
            let quote_hex=hex::encode(&guard.0);
            guards.lock().unwrap().push(guard);
            Json(serde_json::json!({"quote_hex":quote_hex,"signing_address":signer}))
        }}}))
        .route("/v1/attestation-collateral", post({let calls=calls.clone(); move || {let calls=calls.clone();async move {
            calls.lock().unwrap().push("collateral".into());
            include_str!("../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json")
        }}}))
        .route("/v1/witness", post({let calls=calls.clone();let exact=exact.clone();let certified_headers=certified_headers.clone();let cfg=cfg.clone();move |Json(request):Json<serde_json::Value>|{let calls=calls.clone();let exact=exact.clone();let certified_headers=certified_headers.clone();let cfg=cfg.clone();async move{
            calls.lock().unwrap().push("witness".into());
            let raw=serde_json::from_value(request["raw_contribution"].clone()).unwrap();
            let redactor=crate::envelope::build_redactor_with(&cfg,None,None).unwrap();
            let mut envelope=crate::envelope::redact_to_envelope(&redactor,raw).await.unwrap();
            crate::envelope::apply_granted_scopes(&mut envelope,&serde_json::from_value::<Vec<_>>(request["granted_scopes"].clone()).unwrap(),&serde_json::from_value::<Vec<_>>(request["granted_uses"].clone()).unwrap());
            let mut bytes=serde_json::to_vec_pretty(&envelope).unwrap(); bytes.push(b'\n');
            *exact.lock().unwrap()=bytes.clone();
            let (response,_)=crate::witness::transport::signed_fixture(bytes);
            *certified_headers.lock().unwrap() = Some((response.certificate_json.clone(), response.signature_hex.clone()));
            ([(WITNESS_CERTIFICATE_HEADER,response.certificate_json),(WITNESS_SIGNATURE_HEADER,response.signature_hex)],response.envelope_bytes)
        }}}))
        .route("/v1/traces",post({let calls=calls.clone();let uploads=uploads.clone();let certified_headers=certified_headers.clone();move |headers:axum::http::HeaderMap,body:axum::body::Bytes|{let calls=calls.clone();let uploads=uploads.clone();let certified_headers=certified_headers.clone();async move{
            assert_eq!(headers["authorization"],"Bearer synthetic-claim");
            let expected = certified_headers.lock().unwrap().clone().unwrap();
            assert_eq!(headers[WITNESS_CERTIFICATE_HEADER], expected.0);
            assert_eq!(headers[WITNESS_SIGNATURE_HEADER], expected.1);
            calls.lock().unwrap().push("upload".into()); uploads.lock().unwrap().push(body.to_vec());
            Json(serde_json::json!({"status":"accepted","credit_points_pending":0.0,"explanation":[]}))
        }}}));
    let server = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    let response = handle_request_async(
        &s,
        &req(
            "witness_preview_request",
            serde_json::json!({"entry_id":id,"raw_session_confirmed":true}),
        ),
    )
    .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(
        *calls.lock().unwrap(),
        vec!["claim", "attestation", "collateral", "witness"]
    );
    let approved = handle_approve(&s, &req("approve", serde_json::json!({"entry_id":id}))).await;
    assert_eq!(approved.result.unwrap()["approved"], 1);
    let entry = s.queue.lock().unwrap().get(id).unwrap().clone();
    let settings = s.settings.lock().unwrap().clone();
    let opts = crate::submit::SubmitOptions {
        dry_run: false,
        pii_filter: None,
        no_reasoning: false,
        machine_readable: true,
        unenrolled_preview: false,
        remediate_quarantined: false,
        verdict: None,
    };
    let mut context = crate::submit::SubmitContext::new(&s.store, &cfg, &opts, None).unwrap();
    let mut state = super::super::super::state::DaemonState::new();
    let mut health = Default::default();
    let mut uploader = super::super::super::uploader::Uploader {
        ctx: &mut context,
        store: &s.store,
        settings: &settings,
        state: &mut state,
        health: &mut health,
    };
    let roots = s.source_roots_with_routing();
    let sources = crate::source::all_sources(&roots);
    let (source, reference) = super::super::super::find_session(&sources, &entry).unwrap();
    let outcome = uploader
        .upload_entry(source, &reference, &entry, Utc::now())
        .await
        .unwrap();
    assert_eq!(
        uploads.lock().unwrap().as_slice(),
        &[exact.lock().unwrap().clone()],
        "{outcome:?}"
    );
    assert_eq!(
        *calls.lock().unwrap(),
        vec![
            "claim",
            "attestation",
            "collateral",
            "witness",
            "claim",
            "upload"
        ]
    );
    server.abort();
}
