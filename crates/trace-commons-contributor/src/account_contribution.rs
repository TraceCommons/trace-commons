//! Explicit account contribution operations, independent of automatic folder grants.
pub const REFRESH_LINE: &str =
    "Refresh to check contribution readiness. Sign in to your account if required.";
pub const CHECKING_LINE: &str = "Checking contribution status…";
pub const UNAVAILABLE_LINE: &str =
    "Sign in to your account and retry. Contribution status or invite redemption is unavailable.";
pub const PENDING_CREDIT_LINE: &str =
    "Accepted contributions may earn pending credit. Credit redemption is not available.";

use crate::{
    account_auth,
    config::{ConfigStore, ContributorConfig, allowlist_for},
};
use anyhow::{Result, bail};
use reqwest::Method;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributionStatus {
    pub authority: String,
    #[serde(default)]
    pub policy_version: Option<String>,
    pub ready: bool,
    #[serde(default)]
    pub refusal_label: Option<String>,
    #[serde(default)]
    pub retry_after_seconds: Option<i64>,
}
impl ContributionStatus {
    pub fn line(&self) -> &'static str {
        if self.ready {
            "Ready to contribute. Accepted contributions may earn pending credit."
        } else if self.refusal_label.as_deref() == Some("account_limit_reached") {
            crate::private_inference_copy::queue_outcome_line("account_limit_reached")
        } else {
            "Contributions are currently unavailable. Refresh status or redeem an invite."
        }
    }
}
#[derive(Serialize, Deserialize)]
pub struct InviteRedemption {
    pub authority: String,
    pub trust_version: i64,
}
#[derive(Serialize)]
struct InviteRequest<'a> {
    invite_code: &'a str,
    idempotency_key: Uuid,
}

async fn call<B: Serialize + ?Sized, T: DeserializeOwned>(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    method: Method,
    route: &str,
    body: Option<&B>,
    expected_scope: Option<&str>,
) -> Result<T> {
    let loaded =
        crate::daemon::run_blocking(|| account_auth::try_load_session_with_snapshot(store))?
            .ok_or_else(|| anyhow::anyhow!("account-sign-in-required"))?;
    if crate::daemon::commons_credentials::account_snapshot(store, cfg)? != loaded.snapshot {
        bail!("account-session-changed");
    }
    if expected_scope.is_some_and(|scope| {
        crate::daemon::commons_credentials::scope_for_snapshot(&loaded.snapshot)
            .ok()
            .as_deref()
            != Some(scope)
    }) {
        bail!("account-session-changed");
    }
    let client = trace_commons_operator_client::Client::builder(
        &cfg.ingest_url,
        "TRACE_COMMONS_CONTRIBUTOR_UNUSED_BEARER_ENV",
    )
    .bearer_token(&loaded.session.access_token)
    .host_allowlist(allowlist_for(cfg.allowed_hosts.as_deref()))
    .timeout(std::time::Duration::from_secs(10))
    .build()
    .map_err(|_| anyhow::anyhow!("account-contribution-unavailable"))?;
    let reply = client
        .call_json_with_response_header::<B, T>(
            method,
            route,
            &[],
            body,
            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
        )
        .await;
    // Reject a response for a principal that signed out or changed during the request.
    if crate::daemon::commons_credentials::snapshot(
        store,
        crate::daemon::commons_credentials::Kind::Account,
    )? != loaded.snapshot
    {
        bail!("account-session-changed");
    }
    if let Some(token) = reply.response_header {
        crate::daemon::run_blocking(|| account_auth::store_rotated_token(store, &loaded, token))?;
    }
    if reply.status != Some(reqwest::StatusCode::OK) {
        bail!("account-contribution-refused");
    }
    reply
        .result
        .map_err(|_| anyhow::anyhow!("account-contribution-unavailable"))
}
pub async fn status(store: &ConfigStore, cfg: &ContributorConfig) -> Result<ContributionStatus> {
    call::<(), _>(
        store,
        cfg,
        Method::GET,
        "/v1/account/contribution-status",
        None,
        None,
    )
    .await
}
/// Callers retain this UUID with the entered code until success, including transport retries.
pub async fn redeem(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    invite_code: &str,
    idempotency_key: Uuid,
) -> Result<InviteRedemption> {
    if invite_code.trim().is_empty() || invite_code.len() > 512 {
        bail!("account-invite-invalid");
    }
    call(
        store,
        cfg,
        Method::POST,
        "/v1/account/invites/redeem",
        Some(&InviteRequest {
            invite_code: invite_code.trim(),
            idempotency_key,
        }),
        None,
    )
    .await
}

/// One principal/lifecycle/configuration across a composed operation, allowing rotation.
pub(crate) struct Operation<'a> {
    store: &'a ConfigStore,
    cfg: &'a ContributorConfig,
    pub scope: String,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}
impl<'a> Operation<'a> {
    pub async fn open(store: &'a ConfigStore, cfg: &'a ContributorConfig) -> Result<Self> {
        let loaded =
            crate::daemon::run_blocking(|| account_auth::try_load_session_with_snapshot(store))?
                .ok_or_else(|| anyhow::anyhow!("account-sign-in-required"))?;
        if crate::daemon::commons_credentials::account_snapshot(store, cfg)? != loaded.snapshot {
            bail!("account-session-changed");
        }
        Ok(Self {
            store,
            cfg,
            scope: crate::daemon::commons_credentials::scope_for_snapshot(&loaded.snapshot)?,
            expires_at: loaded.session.expires_at,
        })
    }
    pub async fn status(&self) -> Result<ContributionStatus> {
        call::<(), _>(
            self.store,
            self.cfg,
            Method::GET,
            "/v1/account/contribution-status",
            None,
            Some(&self.scope),
        )
        .await
    }
    pub async fn redeem(&self, code: &str, key: Uuid) -> Result<InviteRedemption> {
        if code.trim().is_empty() || code.len() > 512 {
            bail!("account-invite-invalid");
        }
        call(
            self.store,
            self.cfg,
            Method::POST,
            "/v1/account/invites/redeem",
            Some(&InviteRequest {
                invite_code: code.trim(),
                idempotency_key: key,
            }),
            Some(&self.scope),
        )
        .await
    }
    pub async fn redeem_and_status(&self, code: &str, key: Uuid) -> Result<ContributionStatus> {
        self.redeem(code, key).await?;
        self.status().await
    }
}

pub(crate) async fn handle(
    shared: &crate::daemon::ipc::DaemonShared,
    req: &crate::daemon::ipc::Request,
) -> crate::daemon::ipc::Response {
    use crate::daemon::ipc::Response;
    let result = async {
        let cfg = shared
            .store
            .load_config()?
            .ok_or_else(|| anyhow::anyhow!("account-enrollment-required"))?;
        let operation = Operation::open(&shared.store, &cfg).await?;
        if let Some(expected) = req.params.get("account_scope").and_then(|v| v.as_str()) {
            if expected != operation.scope { bail!("account-session-changed"); }
        }
        let status = if req.method == "account_invite_redeem" {
            let code = req
                .params
                .get("invite_code")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("account-invite-invalid"))?;
            let key = req
                .params
                .get("idempotency_key")
                .and_then(|v| v.as_str())
                .and_then(|v| Uuid::parse_str(v).ok())
                .ok_or_else(|| anyhow::anyhow!("account-invite-invalid"))?;
            operation.redeem_and_status(code, key).await?
        } else { operation.status().await? };
        Ok::<_, anyhow::Error>(serde_json::json!({"status": status, "line": status.line(), "account_scope": operation.scope}))
    }
    .await;
    match result {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, "account-contribution-unavailable", UNAVAILABLE_LINE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::{cloud_credential_test_support::MemoryBackend, commons_credentials};
    use axum::{
        Json, Router,
        response::IntoResponse,
        routing::{get, post},
    };
    use std::sync::{Arc, Mutex};

    fn fixture() -> (tempfile::TempDir, Arc<ConfigStore>, ContributorConfig) {
        let (dir, store) = crate::config::tests_support::temp_store();
        commons_credentials::install_test_backend(&store, Arc::new(MemoryBackend::default()));
        let cfg = crate::commands::unenrolled_preview_config();
        store.save_config(&cfg).unwrap();
        (dir, Arc::new(store), cfg)
    }
    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }
    fn ready() -> serde_json::Value {
        serde_json::json!({"authority":"bounded","ready":true,"policy_version":"v1"})
    }

    #[tokio::test]
    async fn explicit_status_needs_no_armed_folders_and_stores_rotation() {
        let (_dir, store, mut cfg) = fixture();
        cfg.ingest_url = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(|| async {
                let mut response = Json(ready()).into_response();
                response.headers_mut().insert(
                    trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                    "synthetic-rotated".parse().unwrap(),
                );
                response
            }),
        ))
        .await;
        store.save_config(&cfg).unwrap();
        // Rebind the synthetic session to the fixture's final ingest authority.
        account_auth::clear_token(&store).unwrap();
        let session = account_auth::AccountSession {
            access_token: "synthetic-token".into(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "synthetic-account".into(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        let shared = crate::daemon::ipc::DaemonShared::load((*store).clone()).unwrap();
        let req = crate::daemon::ipc::Request {
            id: 1,
            method: "account_contribution_status".into(),
            params: serde_json::json!({}),
        };
        let reply = crate::daemon::ipc::handle_request_async(&shared, &req).await;
        assert!(reply.error.is_none());
        assert_eq!(reply.result.unwrap()["status"]["ready"], true);
        assert_eq!(
            account_auth::try_load_token(&store).unwrap().unwrap(),
            "synthetic-rotated"
        );
    }
    #[tokio::test]
    async fn logout_during_status_discards_answer_without_rotation() {
        let (_dir, store, mut cfg) = fixture();
        let changed = store.clone();
        cfg.ingest_url = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(move || {
                let changed = changed.clone();
                async move {
                    account_auth::clear_token(&changed).unwrap();
                    Json(ready())
                }
            }),
        ))
        .await;
        store.save_config(&cfg).unwrap();
        // Rebind the synthetic session to the fixture's final ingest authority.
        account_auth::clear_token(&store).unwrap();
        let session = account_auth::AccountSession {
            access_token: "synthetic-token".into(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "synthetic-account".into(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        assert_eq!(
            status(&store, &cfg).await.unwrap_err().to_string(),
            "account-session-changed"
        );
        assert!(account_auth::try_load_token(&store).unwrap().is_none());
    }
    #[tokio::test]
    async fn ingest_switch_during_status_discards_answer() {
        let (_dir, store, mut cfg) = fixture();
        let changed = store.clone();
        cfg.ingest_url = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(move || {
                let changed = changed.clone();
                async move {
                    let mut cfg = changed.load_config().unwrap().unwrap();
                    cfg.ingest_url = "https://other.invalid".into();
                    changed.save_config(&cfg).unwrap();
                    Json(ready())
                }
            }),
        ))
        .await;
        store.save_config(&cfg).unwrap();
        let session = account_auth::AccountSession {
            access_token: "synthetic-token".into(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "synthetic-account".into(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        assert_eq!(
            status(&store, &cfg).await.unwrap_err().to_string(),
            "account-session-changed"
        );
    }
    #[tokio::test]
    async fn redemption_retry_keeps_uuid_and_refusal_hides_invite() {
        let (_dir, store, mut cfg) = fixture();
        let keys = Arc::new(Mutex::new(Vec::new()));
        let seen = keys.clone();
        cfg.ingest_url = serve(Router::new().route(
            "/v1/account/invites/redeem",
            post(move |Json(body): Json<serde_json::Value>| {
                seen.lock().unwrap().push(body["idempotency_key"].clone());
                async { (reqwest::StatusCode::BAD_REQUEST, "sensitive-invite-code") }
            }),
        ))
        .await;
        store.save_config(&cfg).unwrap();
        // Rebind the synthetic session to the fixture's final ingest authority.
        account_auth::clear_token(&store).unwrap();
        let session = account_auth::AccountSession {
            access_token: "synthetic-token".into(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "synthetic-account".into(),
        };
        store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        let key = Uuid::new_v4();
        for _ in 0..2 {
            let error = redeem(&store, &cfg, "sensitive-invite-code", key)
                .await
                .err()
                .unwrap()
                .to_string();
            assert!(!error.contains("sensitive-invite-code"));
        }
        assert_eq!(
            *keys.lock().unwrap(),
            vec![serde_json::json!(key), serde_json::json!(key)]
        );
    }

    fn publish_native_session(store: &ConfigStore, account: &str) {
        let session = account_auth::AccountSession {
            access_token: "synthetic-native-token".into(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: account.into(),
        };
        let expected =
            commons_credentials::snapshot(store, commons_credentials::Kind::Account).unwrap();
        commons_credentials::replace(
            store,
            &expected,
            &serde_json::to_vec(&session).unwrap(),
            None,
        )
        .unwrap();
    }
    #[tokio::test]
    async fn cli_status_reads_opaque_native_credentials_and_calls_ingest() {
        let (_dir, store, mut cfg) = fixture();
        cfg.ingest_url = serve(Router::new().route(
            "/v1/account/contribution-status",
            get(|| async { Json(ready()) }),
        ))
        .await;
        store.save_config(&cfg).unwrap();
        publish_native_session(&store, "account-a");
        let raw = std::fs::read(store.daemon_path(crate::config::ACCOUNT_SESSION_FILE)).unwrap();
        assert!(
            serde_json::from_slice::<account_auth::AccountSession>(&raw).is_err(),
            "fixture must be an opaque credential record"
        );
        let report = crate::commands::account_status_value(&store).await.unwrap();
        assert_eq!(report["signed_in"], true);
        assert!(report["expires_at"].is_string());
        assert_eq!(report["contribution_status"]["ready"], true);
    }
    #[tokio::test]
    async fn composed_redemption_accepts_rotation_but_rejects_between_call_changes() {
        let (_dir, store, mut cfg) = fixture();
        cfg.ingest_url = serve(
            Router::new()
                .route(
                    "/v1/account/invites/redeem",
                    post(|| async {
                        let mut response =
                            Json(serde_json::json!({"authority":"invited","trust_version":1}))
                                .into_response();
                        response.headers_mut().insert(
                            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                            "synthetic-rotated".parse().unwrap(),
                        );
                        response
                    }),
                )
                .route(
                    "/v1/account/contribution-status",
                    get(|| async { Json(ready()) }),
                ),
        )
        .await;
        store.save_config(&cfg).unwrap();
        publish_native_session(&store, "account-a");
        let op = Operation::open(&store, &cfg).await.unwrap();
        assert!(
            op.redeem_and_status("SYNTHETICINVITE01", Uuid::new_v4())
                .await
                .unwrap()
                .ready
        );
        assert_eq!(
            commons_credentials::account_scope(&store).unwrap(),
            op.scope,
            "rotation preserves operation scope"
        );
        for change in ["replace", "logout-relogin", "ingest"] {
            publish_native_session(&store, "account-a");
            let op = Operation::open(&store, &cfg).await.unwrap();
            op.redeem("SYNTHETICINVITE01", Uuid::new_v4())
                .await
                .unwrap();
            match change {
                "replace" => publish_native_session(&store, "account-b"),
                "logout-relogin" => {
                    account_auth::clear_token(&store).unwrap();
                    publish_native_session(&store, "account-a");
                }
                _ => {
                    let mut changed = cfg.clone();
                    changed.ingest_url = "https://other.invalid".into();
                    store.save_config(&changed).unwrap();
                }
            }
            assert!(
                op.status().await.is_err(),
                "refresh must refuse {change} between successful POST and GET"
            );
            store.save_config(&cfg).unwrap();
        }
    }
    #[tokio::test]
    async fn account_scope_changes_without_device_tenant_change() {
        let (_dir, store, cfg) = fixture();
        publish_native_session(&store, "account-a");
        let first = commons_credentials::account_scope(&store).unwrap();
        publish_native_session(&store, "account-a");
        let relogin = commons_credentials::account_scope(&store).unwrap();
        assert_ne!(
            first, relogin,
            "a new login to the same principal is a new lifecycle"
        );
        account_auth::clear_token(&store).unwrap();
        let signed_out = commons_credentials::account_scope(&store).unwrap();
        assert_ne!(signed_out, relogin);
        publish_native_session(&store, "account-b");
        assert_ne!(
            commons_credentials::account_scope(&store).unwrap(),
            signed_out
        );
        assert_eq!(
            store.load_config().unwrap().unwrap().tenant_id,
            cfg.tenant_id
        );
        let before_ingest = commons_credentials::account_scope(&store).unwrap();
        let mut changed = cfg.clone();
        changed.ingest_url = "https://other.invalid".into();
        store.save_config(&changed).unwrap();
        assert_ne!(
            commons_credentials::account_scope(&store).unwrap(),
            before_ingest
        );
        assert!(!first.contains("synthetic-native-token"));
    }
    #[tokio::test]
    async fn unarmed_daemon_publishes_lifecycle_changes_without_network_status() {
        let (_dir, store, cfg) = fixture();
        publish_native_session(&store, "account-a");
        let shared = crate::daemon::ipc::DaemonShared::load((*store).clone()).unwrap();
        let mut events = shared.events.subscribe();
        crate::daemon::account_admission::refresh(&shared, chrono::Utc::now(), false).await;
        assert_eq!(
            events.try_recv().unwrap().event,
            crate::daemon::ipc::EVENT_STATUS_CHANGED
        );
        let first = shared.status_value()["account_scope"].clone();
        account_auth::clear_token(&store).unwrap();
        publish_native_session(&store, "account-b");
        crate::daemon::account_admission::refresh(&shared, chrono::Utc::now(), false).await;
        assert_eq!(
            events.try_recv().unwrap().event,
            crate::daemon::ipc::EVENT_STATUS_CHANGED
        );
        let fresh = shared.status_value();
        assert_ne!(fresh["account_scope"], first);
        assert_eq!(fresh["tenant_id"], cfg.tenant_id);
    }
    #[test]
    fn account_limit_copy_is_shared_and_has_no_invented_quota() {
        let value: ContributionStatus = serde_json::from_value(serde_json::json!({"authority":"bounded","ready":false,"refusal_label":"account_limit_reached","retry_after_seconds":60})).unwrap();
        assert_eq!(
            value.line(),
            crate::private_inference_copy::queue_outcome_line("account_limit_reached")
        );
    }
}
