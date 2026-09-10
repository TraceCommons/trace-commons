// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI enrolment ceremony, client half against server half, over a
//! real PostgreSQL.
//!
//! #836 shipped as two halves that had never been run against each other. The
//! client runs the ceremony (`daemon/nearai_onboarding.rs`); the server
//! verifies it (`near-ai/provision/{start,finish}`). Each half had a suite and
//! each suite passed, which is exactly the shape that cannot see a
//! disagreement **between** them.
//!
//! One shared function, `near_ai_provisioning_device_bytes`, computes the
//! device-proof preimage on both sides, so the byte layout cannot diverge.
//! What that does not close is whether both halves feed it the same arguments
//! — and they did not. The server returned the ceremony nonce as hex while the
//! client decoded it as standard base64; hex characters are all in the base64
//! alphabet and 64 is a multiple of 4, so the decode *succeeded* and yielded 48
//! bytes, `try_into::<[u8; 32]>` failed, and every enrolment refused
//! `near_ai_enroll_invalid` before the device ever signed. Neither half's tests
//! could see it.
//!
//! # Why the assertions are shaped the way they are
//!
//! `the_start_nonce_is_the_shape_the_client_decodes` asserts the **field
//! shape**, not just the outcome. A test that only checked "enrolment
//! succeeds" would have caught this bug but reported it as a signature failure
//! or a refused finish, sending the next reader to the wrong half. When this
//! recurs it will recur as an encoding, so the failure message names the
//! encoding.
//!
//! # What is real
//!
//! Real: the server's `start` and `finish` handlers over the real router, the
//! real ceremony row in a real PostgreSQL, the real device signature, the
//! shared preimage function, and the client's own marshalling via
//! `device_proof_for_ceremony` — the seam that exists so this test does not
//! have to reimplement the client's half.
//!
//! Stubbed: NEAR AI introspection only, and only through the `cfg(test)` seam
//! on `AppState`, which does not exist in a shipped binary. Nothing else about
//! the ceremony is stood in for; the preimage, the signature and the ceremony
//! storage are the things under test.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_contributor::daemon::nearai_onboarding::device_proof_for_ceremony;
use trace_commons_contributor::identity::DeviceIdentity;

/// The subject the stubbed NEAR AI `/me` resolves the token to.
const STUB_SUBJECT: &str = "auth0|nearai-ceremony-fixture";

/// A migrated PostgreSQL on the isolated URL this job supplies.
///
/// Deliberately the same variable the sibling suite uses: one database per CI
/// job, and a suite that silently invents its own would not be running against
/// the schema the job migrated.
async fn ceremony_pg_admin() -> Arc<PgBackend> {
    let url = std::env::var("TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL")
        .expect("explicit isolated URL required");
    let parsed = reqwest::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("127.0.0.1"));
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

/// A local stand-in for NEAR AI's `GET /me`, and nothing else.
///
/// Returns its base URL. This is the one thing the test stubs: it is not our
/// half of anything, and reaching it at all requires the device proof to have
/// already verified, because introspection is deliberately the last step of
/// `finish`.
async fn stub_near_ai() -> String {
    let router = axum::Router::new().route(
        "/me",
        axum::routing::get(|| async {
            axum::Json(serde_json::json!({
                "id": STUB_SUBJECT,
                "auth_provider": "github",
            }))
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

/// Everything `near_ai_login_ready` requires, so the routes are mounted.
///
/// These are process-global, which is why this suite is `#[ignore]`d and run in
/// its own step rather than alongside the parallel unit tests.
fn publish_provisioning_material() {
    // SAFETY: single-threaded setup at the top of one serial, ignored test.
    unsafe {
        std::env::set_var(
            "TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON",
            serde_json::json!({
                "url": "https://witness.example",
                "signing_address": format!("0x{}", "ab".repeat(20)),
                "expected_measurements": [format!("mrtd={}", "cd".repeat(48))],
            })
            .to_string(),
        );
        std::env::set_var(
            "TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL",
            "https://issuer.example",
        );
        std::env::set_var("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE", "upload");
    }
}

/// An `AppState` with the NEAR AI login path ready and introspection pointed at
/// the stub.
async fn ceremony_state(db: Arc<PgBackend>, near_ai_base: String) -> Arc<AppState> {
    publish_provisioning_material();
    let temp = tempfile::tempdir().unwrap();
    let mut state = test_state(temp.path().to_path_buf());
    std::mem::forget(temp);
    let s = Arc::make_mut(&mut state);
    s.near_provisioning_enabled = true;
    s.near_provisioning_admission_ready = true;
    s.near_account_identity = Some(Arc::new(near_account_identity_for_test()));
    s.db_mirror = Some(db.clone() as Arc<dyn Database>);
    s.near_ai_introspection_base_url = Some(near_ai_base);
    state
}

async fn post_json(state: &Arc<AppState>, path: &str, body: serde_json::Value) -> (StatusCode, Vec<u8>) {
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(path)
        .header(CONTENT_TYPE, "application/json")
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    let response = app(state.clone()).oneshot(request).await.unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap()
        .to_vec();
    (status, bytes)
}

/// How many anchor rows exist for the stub subject. The thing a successful
/// enrolment is supposed to leave behind, and the thing a refused one must not.
async fn anchor_rows(db: &PgBackend) -> i64 {
    let client = db.raw_pool_for_tests_and_diagnostics().get().await.unwrap();
    client
        .query_one("SELECT count(*)::bigint FROM trace_near_account_anchors", &[])
        .await
        .unwrap()
        .get(0)
}

fn device() -> (tempfile::TempDir, DeviceIdentity) {
    let dir = tempfile::tempdir().unwrap();
    let store = trace_commons_contributor::config::ConfigStore::open(dir.path().to_path_buf())
        .expect("a config store");
    let identity = DeviceIdentity::load_or_generate(&store).expect("a device identity");
    (dir, identity)
}

fn challenge_pair() -> (String, String) {
    use base64::Engine as _;
    use sha2::Digest as _;
    let verifier = "a".repeat(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

/// The whole ceremony: the server's real `start`, the client's real
/// marshalling, the server's real `finish`, and the anchor row that proves it
/// completed.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn a_client_device_proof_enrols_against_the_real_handlers() {
    let db = ceremony_pg_admin().await;
    let before = anchor_rows(&db).await;
    let state = ceremony_state(db.clone(), stub_near_ai().await).await;
    let (_dir, identity) = device();
    let (verifier, challenge) = challenge_pair();

    let (status, body) = post_json(
        &state,
        "/v1/account/near-ai/provision/start",
        serde_json::json!({
            "device_public_key": identity.public_key_b64,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let nonce_wire = started["nonce"].as_str().expect("a nonce field").to_string();
    let ceremony_id = started["ceremony_id"].as_str().unwrap().to_string();
    let expires_at = started["expires_at"].as_i64().unwrap();

    // The field shape, named. See the module docs: when this recurs it will
    // recur as an encoding, and a bare "enrolment failed" would send the next
    // reader to the signature check instead of to this line.
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(&nonce_wire)
            .map(|bytes| bytes.len()),
        Ok(32),
        "the nonce the server returned is not what the client decodes: the \
         client reads this field with standard base64 and requires exactly 32 \
         bytes. A 64-character hex string decodes as valid base64 to 48 bytes, \
         which is the #836 mismatch."
    );

    // The client's own marshalling, through the seam that exists so this test
    // does not reimplement it.
    let device_signature = device_proof_for_ceremony(
        &identity,
        &ceremony_id,
        &nonce_wire,
        &challenge,
        expires_at,
        chrono::Utc::now().timestamp(),
    )
    .expect("the client must be able to build a proof for its own ceremony");

    let (status, body) = post_json(
        &state,
        "/v1/account/near-ai/provision/finish",
        serde_json::json!({
            "ceremony_id": ceremony_id,
            "code_verifier": verifier,
            "device_public_key": identity.public_key_b64,
            "device_signature": device_signature,
            "access_token": "stub-near-ai-jwt",
        }),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the server refused a proof its own start ceremony asked for: {}",
        String::from_utf8_lossy(&body)
    );
    let finished: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(finished["tenant_id"].as_str().unwrap().starts_with("nearai-"));
    assert!(!finished["anchor_hash"].as_str().unwrap().is_empty());
    assert_eq!(
        anchor_rows(&db).await,
        before + 1,
        "a successful enrolment must leave an anchor row"
    );
}

/// A refused finish leaves nothing behind.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn a_refused_finish_writes_no_anchor_row() {
    let db = ceremony_pg_admin().await;
    let state = ceremony_state(db.clone(), stub_near_ai().await).await;
    let (_dir, identity) = device();
    let (verifier, challenge) = challenge_pair();

    let (_, body) = post_json(
        &state,
        "/v1/account/near-ai/provision/start",
        serde_json::json!({
            "device_public_key": identity.public_key_b64,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }),
    )
    .await;
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let before = anchor_rows(&db).await;

    // A signature over a different ceremony id: well-formed, correctly
    // encoded, and not for this ceremony.
    let wrong = device_proof_for_ceremony(
        &identity,
        "some-other-ceremony",
        started["nonce"].as_str().unwrap(),
        &challenge,
        started["expires_at"].as_i64().unwrap(),
        chrono::Utc::now().timestamp(),
    )
    .expect("the client can build a proof for the wrong ceremony");

    let (status, _) = post_json(
        &state,
        "/v1/account/near-ai/provision/finish",
        serde_json::json!({
            "ceremony_id": started["ceremony_id"],
            "code_verifier": verifier,
            "device_public_key": identity.public_key_b64,
            "device_signature": wrong,
            "access_token": "stub-near-ai-jwt",
        }),
    )
    .await;
    assert_ne!(status, StatusCode::OK, "a proof for another ceremony was accepted");
    assert_eq!(
        anchor_rows(&db).await,
        before,
        "a refused finish wrote an anchor row"
    );
}

/// A client-asserted account is refused at the parse boundary.
///
/// The server half made this structural with `#[serde(deny_unknown_fields)]`
/// and has its own test over `from_value`. This crosses it from the wire: the
/// body goes through the real route, so it also pins that nothing upstream of
/// the deserialiser strips or tolerates the field.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_ADMISSION_INGEST_PG_TEST_URL"]
async fn a_finish_naming_an_account_is_refused_over_the_wire() {
    let db = ceremony_pg_admin().await;
    let state = ceremony_state(db.clone(), stub_near_ai().await).await;
    let (_dir, identity) = device();
    let (verifier, challenge) = challenge_pair();

    let (_, body) = post_json(
        &state,
        "/v1/account/near-ai/provision/start",
        serde_json::json!({
            "device_public_key": identity.public_key_b64,
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        }),
    )
    .await;
    let started: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let signature = device_proof_for_ceremony(
        &identity,
        started["ceremony_id"].as_str().unwrap(),
        started["nonce"].as_str().unwrap(),
        &challenge,
        started["expires_at"].as_i64().unwrap(),
        chrono::Utc::now().timestamp(),
    )
    .expect("a valid proof");
    let before = anchor_rows(&db).await;

    let (status, _) = post_json(
        &state,
        "/v1/account/near-ai/provision/finish",
        serde_json::json!({
            "ceremony_id": started["ceremony_id"],
            "code_verifier": verifier,
            "device_public_key": identity.public_key_b64,
            "device_signature": signature,
            "access_token": "stub-near-ai-jwt",
            // The whole point of the ceremony: the account is whatever the
            // commons resolves the token to, never what the client says.
            "account_id": "attacker.near",
        }),
    )
    .await;
    assert_ne!(
        status,
        StatusCode::OK,
        "a finish naming an account was accepted"
    );
    assert_eq!(anchor_rows(&db).await, before);
}
