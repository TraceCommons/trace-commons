use super::*;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

fn request(method: &str, params: Value) -> Request {
    Request {
        id: 1,
        method: method.into(),
        params,
    }
}
fn shared() -> (tempfile::TempDir, DaemonShared) {
    let (dir, store) = crate::config::tests_support::temp_store();
    (dir, DaemonShared::load(store).unwrap())
}
fn options() -> Value {
    json!({"ceremony_id":"server-ceremony","expires_in_secs":300,"public_key":{"publicKey":{
        "rp":{"id":"tracecommons.ai","name":"Trace Commons"},
        "challenge":URL_SAFE_NO_PAD.encode([7;32]),
        "user":{"id":URL_SAFE_NO_PAD.encode([3;16]),"name":"My passkey","displayName":"My passkey"},
        "pubKeyCredParams":[{"type":"public-key","alg":-7}],
        "authenticatorSelection":{"userVerification":"required","residentKey":"preferred"},
        "excludeCredentials":[{"type":"public-key","id":"AQID"}]
    }}})
}
#[test]
fn options_preserve_exclusions_and_reject_wrong_rp_or_algorithm() {
    let parsed = normalize_options(Action::Create, &options()).unwrap();
    assert_eq!(parsed["exclude_credentials"], json!(["AQID"]));
    assert_eq!(parsed["user_verification"], "required");
    let mut wrong = options();
    wrong["public_key"]["publicKey"]["rp"]["id"] = json!("attacker.example");
    assert!(normalize_options(Action::Create, &wrong).is_err());
    let mut wrong = options();
    wrong["public_key"]["publicKey"]["pubKeyCredParams"][0]["alg"] = json!(-257);
    assert!(normalize_options(Action::Create, &wrong).is_err());
}
#[test]
fn one_encoder_assembles_registration_and_refuses_noncanonical_bytes() {
    let params = json!({"credential_id":"AQID","raw_client_data_json":"BAUG","raw_attestation_object":"BwgJ"});
    let encoded = credential(Action::Create, &params).unwrap();
    assert_eq!(
        encoded,
        json!({"id":"AQID","rawId":"AQID","type":"public-key","response":{"clientDataJSON":"BAUG","attestationObject":"BwgJ"},"extensions":{}})
    );
    let mut bad = params;
    bad["credential_id"] = json!("AQID=");
    assert!(credential(Action::Create, &bad).is_err());
}
#[tokio::test]
async fn cancelled_and_wrong_action_ceremonies_are_consumed() {
    let (_dir, shared) = shared();
    let snapshot = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let pending = Pending {
        action: Action::Create,
        origin: "https://commons.example".into(),
        server_id: "s".into(),
        challenge: "AQID".into(),
        expires: std::time::Instant::now() + std::time::Duration::from_secs(30),
        snapshot,
        label: None,
        session: None,
    };
    shared
        .native_identity
        .lock()
        .unwrap()
        .insert("local".into(), pending);
    let result = handle(
        &shared,
        &request("passkey_cancel", json!({"ceremony":"local"})),
    )
    .await;
    assert_eq!(result.result.unwrap()["cancelled"], true);
    let result = handle(
        &shared,
        &request("passkey_create_complete", json!({"ceremony":"local"})),
    )
    .await;
    assert!(result.error.is_some());
}
#[test]
fn session_origin_survives_rotation_and_signout_invalidates_empty_snapshot() {
    let (_dir, shared) = shared();
    let before = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let session = json!({"access_token":"tcn1_synthetic", "expires_at":Utc::now()+chrono::Duration::hours(6),"account_id":uuid::Uuid::new_v4().to_string(),"ingest_origin":"https://commons.example","binding_state":"unbound"});
    commons_credentials::replace(
        &shared.store,
        &before,
        &serde_json::to_vec(&session).unwrap(),
        None,
    )
    .unwrap();
    let loaded = account_auth::try_load_session_with_snapshot(&shared.store)
        .unwrap()
        .unwrap();
    account_auth::store_rotated_token(&shared.store, &loaded, "tcn1_rotated".into()).unwrap();
    let raw = commons_credentials::load(&shared.store, Kind::Account)
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&raw).unwrap()["ingest_origin"],
        "https://commons.example"
    );
    commons_credentials::clear(&shared.store, &[Kind::Account]).unwrap();
    assert!(
        commons_credentials::replace(
            &shared.store,
            &before,
            &serde_json::to_vec(&session).unwrap(),
            None
        )
        .is_err()
    );
    assert!(shared.store.load_config().unwrap().is_none());
}

#[tokio::test]
async fn no_config_origin_is_pinned_and_completion_never_grants_consent() {
    let (_dir, shared) = shared();
    let before = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let reply = json!({"access_token":"tcn1_synthetic", "token_type":"Bearer", "expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"unbound"});
    persist_session(&shared.store, &before, "https://commons.example", &reply).unwrap();
    assert!(shared.store.load_config().unwrap().is_none());
    assert_eq!(
        resolve_origin(&shared.store, &json!({})).unwrap(),
        "https://commons.example"
    );
    assert!(
        resolve_origin(
            &shared.store,
            &json!({"ingest_url":"https://attacker.example"})
        )
        .is_err()
    );
    let status = handle(&shared, &request("account_session_status", json!({}))).await;
    let encoded = serde_json::to_string(&status).unwrap();
    assert!(!encoded.contains("tcn1_"));
    assert_eq!(status.result.unwrap()["signed_in"], true);
    commons_credentials::clear(&shared.store, &[Kind::Account]).unwrap();
    let logout = handle(&shared, &request("account_sign_out", json!({}))).await;
    assert_eq!(logout.result.unwrap()["signed_out"], true);
    assert!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .is_none()
    );
    assert!(persist_session(&shared.store, &before, "https://commons.example", &reply).is_err());
}

#[test]
fn assertion_encoder_preserves_all_apple_bytes() {
    let value = credential(Action::Login, &json!({"credential_id":"AQID","raw_client_data_json":"BAUG","raw_authenticator_data":"BwgJ","signature":"CgsM","user_handle":"DQ4P"})).unwrap();
    assert_eq!(
        value["response"],
        json!({"clientDataJSON":"BAUG","authenticatorData":"BwgJ","signature":"CgsM","userHandle":"DQ4P"})
    );
}

#[test]
fn bind_proof_is_account_bound_and_not_a_provisioning_proof() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let identity = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
    let nonce = base64::engine::general_purpose::STANDARD.encode([5u8; 32]);
    let account = uuid::Uuid::new_v4();
    let bind = super::super::nearai_onboarding::bind_device_proof(
        &identity,
        "ceremony",
        &nonce,
        "challenge",
        100,
        1,
        &account,
    )
    .unwrap();
    let provision = super::super::nearai_onboarding::device_proof_for_ceremony(
        &identity,
        "ceremony",
        &nonce,
        "challenge",
        100,
        1,
    )
    .unwrap();
    assert_ne!(bind, provision);
    let other = super::super::nearai_onboarding::bind_device_proof(
        &identity,
        "ceremony",
        &nonce,
        "challenge",
        100,
        1,
        &uuid::Uuid::new_v4(),
    )
    .unwrap();
    assert_ne!(bind, other);
    assert!(
        super::super::nearai_onboarding::bind_device_proof(
            &identity,
            "ceremony",
            &nonce,
            "challenge",
            100,
            100,
            &account
        )
        .is_err()
    );
}

async fn server_once(
    status: &str,
    body: Value,
    rotation: Option<&str>,
) -> (String, tokio::task::JoinHandle<String>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let body = body.to_string();
    let extra = rotation
        .map(|s| format!("{}: {}\r\n", ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER, s))
        .unwrap_or_default();
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    );
    let worker = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut raw = Vec::new();
        loop {
            let mut buf = [0; 4096];
            let n = socket.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            raw.extend_from_slice(&buf[..n]);
            if let Some(header_end) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                let headers = String::from_utf8_lossy(&raw[..header_end]).to_lowercase();
                let length = headers
                    .lines()
                    .find_map(|l| l.strip_prefix("content-length: "))
                    .and_then(|n| n.parse::<usize>().ok())
                    .unwrap_or(0);
                if raw.len() >= header_end + 4 + length {
                    break;
                }
            }
        }
        socket.write_all(response.as_bytes()).await.unwrap();
        String::from_utf8(raw).unwrap()
    });
    (origin, worker)
}

fn registration_params() -> Value {
    let data = json!({"type":"webauthn.create","challenge":"AQID","origin":"https://tracecommons.ai","crossOrigin":false});
    json!({"ceremony":"local","credential_id":"AQID","raw_client_data_json":URL_SAFE_NO_PAD.encode(data.to_string()),"raw_attestation_object":"BwgJ"})
}
fn put_pending(shared: &DaemonShared, origin: &str, action: Action, expires: Instant) {
    let snapshot = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    shared.native_identity.lock().unwrap().insert(
        "local".into(),
        Pending {
            action,
            origin: origin.into(),
            server_id: "actual-server-id".into(),
            challenge: "AQID".into(),
            expires,
            snapshot,
            label: None,
            session: None,
        },
    );
}
#[tokio::test]
async fn create_http_finish_uses_pinned_server_id_and_persists_only_in_rust() {
    let (_dir, shared) = shared();
    let account_id = uuid::Uuid::new_v4().to_string();
    let (origin,worker) = server_once("200 OK",json!({"access_token":"tcn1_secret", "token_type":"Bearer","expires_in_secs":43200,"account_id":account_id,"binding_state":"unbound"}),None).await;
    put_pending(
        &shared,
        &origin,
        Action::Create,
        Instant::now() + Duration::from_secs(30),
    );
    let response = handle(
        &shared,
        &request("passkey_create_complete", registration_params()),
    )
    .await;
    assert_eq!(response.result.unwrap(), json!({"binding_state":"unbound"}));
    let sent = worker.await.unwrap();
    assert!(sent.starts_with("POST /v1/account/native/passkey/create/finish "));
    let body: Value = serde_json::from_str(sent.split("\r\n\r\n").nth(1).unwrap()).unwrap();
    assert_eq!(body["ceremony_id"], "actual-server-id");
    assert_eq!(body["credential"]["response"]["attestationObject"], "BwgJ");
    assert_eq!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .as_deref(),
        Some("tcn1_secret")
    );
    assert!(shared.store.load_config().unwrap().is_none());
    assert!(
        handle(
            &shared,
            &request("passkey_create_complete", registration_params())
        )
        .await
        .error
        .is_some()
    );
}
#[tokio::test]
async fn expiration_wrong_action_wrong_challenge_and_signout_refuse_before_network() {
    let (_dir, shared) = shared();
    for case in ["expired", "wrong-action", "wrong-challenge", "signed-out"] {
        let expires = if case == "expired" {
            Instant::now() - Duration::from_secs(1)
        } else {
            Instant::now() + Duration::from_secs(30)
        };
        let action = if case == "wrong-action" {
            Action::Login
        } else {
            Action::Create
        };
        put_pending(&shared, "http://127.0.0.1:1", action, expires);
        if case == "signed-out" {
            commons_credentials::clear(&shared.store, &[Kind::Account]).unwrap();
        }
        let mut params = registration_params();
        if case == "wrong-challenge" {
            params["raw_client_data_json"] = json!(URL_SAFE_NO_PAD.encode(json!({"type":"webauthn.create","challenge":"wrong","origin":"https://tracecommons.ai"}).to_string()));
        }
        let response = handle(&shared, &request("passkey_create_complete", params)).await;
        let label = response.error.unwrap().message;
        assert_ne!(
            label, "passkey-finish-refused",
            "{case} must refuse before network"
        );
        assert!(!shared.native_identity.lock().unwrap().contains_key("local"));
        assert!(
            account_auth::try_load_token(&shared.store)
                .unwrap()
                .is_none()
        );
    }
}
#[tokio::test]
async fn authenticated_http_refusal_still_retains_rotated_token_and_origin() {
    let (_dir, shared) = shared();
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    persist_session(&shared.store,&expected,"https://commons.example",&json!({"access_token":"tcn1_original","token_type":"Bearer","expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"bound"})).unwrap();
    let mut session = account_auth::try_load_session_with_snapshot(&shared.store)
        .unwrap()
        .unwrap();
    let (origin, worker) = server_once(
        "403 Forbidden",
        json!({"error":"requires step up"}),
        Some("tcn1_rotated"),
    )
    .await;
    assert!(
        authenticated(
            &shared,
            &origin,
            &mut session,
            Method::POST,
            "/v1/account/passkeys/native/register/start",
            Some(&json!({}))
        )
        .await
        .is_err()
    );
    let request = worker.await.unwrap();
    assert!(
        request
            .to_lowercase()
            .contains("authorization: bearer tcn1_original")
    );
    assert_eq!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .as_deref(),
        Some("tcn1_rotated")
    );
    assert_eq!(
        resolve_origin(&shared.store, &json!({})).unwrap(),
        "https://commons.example"
    );
}

#[test]
fn native_login_refuses_other_enrollment_tenant_and_account_without_replacing_credentials() {
    let (_dir, shared) = shared();
    let cfg:crate::config::ContributorConfig = serde_json::from_value(json!({
        "schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,"issuer_url":"https://issuer.example","ingest_url":"https://commons.example","audience":"trace-commons-upload","tenant_id":"tenant-a","instance_id":"instance-a","user_subject":"device-a","device_key_id":"device-a","consent_scopes":[]
    })).unwrap();
    shared.store.save_config(&cfg).unwrap();
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut result = json!({"access_token":format!("tcn1_{}.secret",URL_SAFE_NO_PAD.encode("tenant-b")),"token_type":"Bearer","expires_in_secs":43200,"account_id":id,"binding_state":"bound"});
    assert!(persist_session(&shared.store, &expected, "https://commons.example", &result).is_err());
    assert!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .is_none()
    );
    result["access_token"] = json!(format!(
        "tcn1_{}.secret",
        URL_SAFE_NO_PAD.encode("tenant-a")
    ));
    persist_session(&shared.store, &expected, "https://commons.example", &result).unwrap();
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    result["account_id"] = json!(uuid::Uuid::new_v4().to_string());
    assert!(persist_session(&shared.store, &expected, "https://commons.example", &result).is_err());
    assert_eq!(
        account_auth::try_load_session_with_snapshot(&shared.store)
            .unwrap()
            .unwrap()
            .session
            .account_id,
        id
    );
    assert_eq!(
        shared.store.load_config().unwrap().unwrap().tenant_id,
        "tenant-a"
    );
}

#[tokio::test]
async fn corrupt_config_does_not_prevent_local_logout_or_restore_a_stale_finish() {
    let (_dir, shared) = shared();
    let fresh = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    persist_session(&shared.store,&fresh,"https://commons.example",&json!({"access_token":"tcn1_secret","token_type":"Bearer","expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"unbound"})).unwrap();
    put_pending(
        &shared,
        "https://commons.example",
        Action::Create,
        Instant::now() + Duration::from_secs(30),
    );
    let before = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    std::fs::write(shared.store.daemon_path("contributor.json"), b"not-json").unwrap();
    let response = handle(&shared, &request("account_sign_out", json!({}))).await;
    assert_eq!(response.result.unwrap()["signed_out"], true);
    assert!(shared.native_identity.lock().unwrap().is_empty());
    assert!(
        !shared
            .store
            .daemon_path(crate::config::ACCOUNT_SESSION_FILE)
            .exists()
    );
    assert!(commons_credentials::replace(&shared.store, &before, b"{}", None).is_err());
}

#[tokio::test]
async fn captured_account_cannot_send_after_an_origin_and_authority_switch() {
    let (_dir, shared) = shared();
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    persist_session(&shared.store,&expected,"https://old.example",&json!({"access_token":"tcn1_old","token_type":"Bearer","expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"unbound"})).unwrap();
    let mut old = account_auth::try_load_session_with_snapshot(&shared.store)
        .unwrap()
        .unwrap();
    persist_session(&shared.store,&old.snapshot,"https://new.example",&json!({"access_token":"tcn1_new","token_type":"Bearer","expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"unbound"})).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let error = authenticated(
        &shared,
        &origin,
        &mut old,
        Method::GET,
        "/v1/account/binding",
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(error.to_string(), "account-session-changed");
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
    assert_eq!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .as_deref(),
        Some("tcn1_new")
    );
}

#[tokio::test]
async fn enrolled_host_policy_refuses_before_any_bearer_request() {
    let (_dir, shared) = shared();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let cfg:crate::config::ContributorConfig=serde_json::from_value(json!({"schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,"issuer_url":"https://issuer.example","ingest_url":origin,"audience":"upload","tenant_id":"tenant-a","instance_id":"instance-a","user_subject":"device-a","device_key_id":"device-a","consent_scopes":[],"allowed_hosts":"allowed.example"})).unwrap();
    shared.store.save_config(&cfg).unwrap();
    let expected = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    persist_session(&shared.store,&expected,&origin,&json!({"access_token":format!("tcn1_{}.secret",URL_SAFE_NO_PAD.encode("tenant-a")),"token_type":"Bearer","expires_in_secs":43200,"account_id":uuid::Uuid::new_v4().to_string(),"binding_state":"bound"})).unwrap();
    let mut session = account_auth::try_load_session_with_snapshot(&shared.store)
        .unwrap()
        .unwrap();
    assert_eq!(
        authenticated(
            &shared,
            &origin,
            &mut session,
            Method::GET,
            "/v1/account/binding",
            None
        )
        .await
        .unwrap_err()
        .to_string(),
        "account-origin-refused"
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oversized_http_completion_never_publishes_credentials_or_enrollment() {
    let (_dir, shared) = shared();
    let (origin, worker) = server_once("200 OK", json!({"large":"x".repeat(256*1024)}), None).await;
    put_pending(
        &shared,
        &origin,
        Action::Create,
        Instant::now() + Duration::from_secs(30),
    );
    let response = handle(
        &shared,
        &request("passkey_create_complete", registration_params()),
    )
    .await;
    assert_eq!(response.error.unwrap().message, "passkey-finish-refused");
    worker.await.unwrap();
    assert!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .is_none()
    );
    assert!(shared.store.load_config().unwrap().is_none());
}

#[test]
fn enrolled_upload_endpoint_resolves_to_account_origin() {
    let (_dir, shared) = shared();
    let cfg:crate::config::ContributorConfig=serde_json::from_value(json!({"schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,"issuer_url":"https://issuer.example","ingest_url":"https://commons.example:8443/v1/traces","audience":"upload","tenant_id":"tenant-a","instance_id":"instance-a","user_subject":"device-a","device_key_id":"device-a","consent_scopes":[],"allowed_hosts":"commons.example"})).unwrap();
    shared.store.save_config(&cfg).unwrap();
    assert_eq!(
        resolve_origin(&shared.store, &json!({})).unwrap(),
        "https://commons.example:8443"
    );
    assert_eq!(
        resolve_origin(
            &shared.store,
            &json!({"ingest_url":"https://commons.example:8443/v1/traces"})
        )
        .unwrap(),
        "https://commons.example:8443"
    );
}

#[test]
fn explicit_enrollment_host_policy_wins_over_environment_without_bypassing_signup_policy() {
    const CHILD: &str = "TRACE_COMMONS_IDENTITY_HOST_POLICY_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output=std::process::Command::new(std::env::current_exe().unwrap())
            .args(["daemon::native_identity::tests::explicit_enrollment_host_policy_wins_over_environment_without_bypassing_signup_policy","--exact"])
            .env(CHILD,"1").env("TRACE_COMMONS_ALLOWED_HOSTS","environment-only.example")
            .output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stdout)
        );
        return;
    }
    let (_dir, shared) = shared();
    let cfg:crate::config::ContributorConfig=serde_json::from_value(json!({"schema_version":crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,"issuer_url":"https://issuer.example","ingest_url":"https://commons.example/v1/traces","audience":"upload","tenant_id":"tenant-a","instance_id":"instance-a","user_subject":"device-a","device_key_id":"device-a","consent_scopes":[],"allowed_hosts":"commons.example"})).unwrap();
    shared.store.save_config(&cfg).unwrap();
    let snapshot = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let origin = resolve_origin(&shared.store, &json!({})).unwrap();
    assert!(scoped_client(&shared.store, &snapshot, &origin, "synthetic").is_ok());
    let (_other, store) = crate::config::tests_support::temp_store();
    let snapshot = commons_credentials::snapshot(&store, Kind::Account).unwrap();
    let origin = resolve_origin(&store, &json!({"ingest_url":"https://commons.example"})).unwrap();
    assert!(scoped_client(&store, &snapshot, &origin, "synthetic").is_err());
    let allowed = resolve_origin(
        &store,
        &json!({"ingest_url":"https://environment-only.example"}),
    )
    .unwrap();
    assert!(scoped_client(&store, &snapshot, &allowed, "synthetic").is_ok());
}
