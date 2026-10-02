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
) -> Result<T> {
    let loaded =
        crate::daemon::run_blocking(|| account_auth::try_load_session_with_snapshot(store))?
            .ok_or_else(|| anyhow::anyhow!("account-sign-in-required"))?;
    if crate::daemon::commons_credentials::account_snapshot(store, cfg)? != loaded.snapshot {
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
    )
    .await
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
        if req.method == "account_invite_redeem" {
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
            redeem(&shared.store, &cfg, code, key).await?;
        }
        let status = status(&shared.store, &cfg).await?;
        Ok::<_, anyhow::Error>(serde_json::json!({"status": status, "line": status.line()}))
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
    #[test]
    fn account_limit_copy_is_shared_and_has_no_invented_quota() {
        let value: ContributionStatus = serde_json::from_value(serde_json::json!({"authority":"bounded","ready":false,"refusal_label":"account_limit_reached","retry_after_seconds":60})).unwrap();
        assert_eq!(
            value.line(),
            crate::private_inference_copy::queue_outcome_line("account_limit_reached")
        );
    }
}
