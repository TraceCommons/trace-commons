use super::*;
use chrono::Utc;
use std::time::Duration;

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
/// Store a signed-in session retained for `origin`, as a sign-in leaves one.
fn store_session(store: &ConfigStore, origin: &str, access_token: &str) {
    let fresh = commons_credentials::snapshot(store, Kind::Account).unwrap();
    let session = json!({"access_token":access_token,"expires_at":Utc::now()+chrono::Duration::hours(12),"account_id":uuid::Uuid::new_v4().to_string(),"ingest_origin":origin});
    commons_credentials::replace(store, &fresh, &serde_json::to_vec(&session).unwrap(), None)
        .unwrap();
}

#[test]
fn session_origin_survives_rotation_and_signout_invalidates_empty_snapshot() {
    let (_dir, shared) = shared();
    let before = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    let session = json!({"access_token":"tcn1_synthetic", "expires_at":Utc::now()+chrono::Duration::hours(6),"account_id":uuid::Uuid::new_v4().to_string(),"ingest_origin":"https://commons.example"});
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
async fn corrupt_config_does_not_prevent_local_logout_or_restore_a_stale_session() {
    let (_dir, shared) = shared();
    store_session(&shared.store, "https://commons.example", "tcn1_secret");
    let before = commons_credentials::snapshot(&shared.store, Kind::Account).unwrap();
    std::fs::write(shared.store.daemon_path("contributor.json"), b"not-json").unwrap();
    let response = handle(&shared, &request("account_sign_out", json!({}))).await;
    assert_eq!(response.result.unwrap()["signed_out"], true);
    assert!(
        !shared
            .store
            .daemon_path(crate::config::ACCOUNT_SESSION_FILE)
            .exists()
    );
    assert!(commons_credentials::replace(&shared.store, &before, b"{}", None).is_err());
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
    assert_eq!(
        resolve_origin(&shared.store, &json!({})).unwrap(),
        "https://commons.example"
    );
    let (_other, store) = crate::config::tests_support::temp_store();
    assert_eq!(
        resolve_origin(&store, &json!({})).unwrap(),
        crate::channel::NATIVE_ACCOUNT_ORIGIN
    );
    assert!(
        resolve_origin(
            &store,
            &json!({"ingest_url":"https://environment-only.example"})
        )
        .is_err()
    );
}

#[test]
fn unconfigured_identity_refuses_ipc_and_retained_untrusted_origins() {
    let (_dir, shared) = shared();
    let native = crate::channel::NATIVE_ACCOUNT_ORIGIN;
    for origin in [
        "https://attacker.example".to_string(),
        format!("{native}.attacker.example"),
        format!("{native}:8443"),
    ] {
        assert!(resolve_origin(&shared.store, &json!({"ingest_url":origin})).is_err());
    }
    assert_eq!(resolve_origin(&shared.store, &json!({})).unwrap(), native);
    store_session(&shared.store, "https://attacker.example", "tcn1_secret");
    assert!(resolve_origin(&shared.store, &json!({})).is_err());
}

#[tokio::test]
async fn unconfigured_logout_never_sends_retained_token_to_untrusted_origin() {
    let (_dir, shared) = shared();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("https://{}", listener.local_addr().unwrap());
    store_session(&shared.store, &origin, "tcn1_secret");
    let response = handle(&shared, &request("account_sign_out", json!({}))).await;
    assert_eq!(response.result.unwrap()["signed_out"], true);
    assert!(
        account_auth::try_load_token(&shared.store)
            .unwrap()
            .is_none()
    );
    assert!(
        tokio::time::timeout(Duration::from_millis(20), listener.accept())
            .await
            .is_err()
    );
}

/// An unknown error never reaches the shell; it answers `account-unavailable`.
#[test]
fn an_unknown_error_never_reaches_the_shell() {
    assert_eq!(
        super::ipc_label("account-sign-in-refused"),
        "account-sign-in-refused"
    );
    assert_eq!(
        super::ipc_label("near_ai_account_mismatch"),
        "account-unavailable"
    );
    assert_eq!(
        super::ipc_label("https://commons.example/v1 failed"),
        "account-unavailable"
    );
}
