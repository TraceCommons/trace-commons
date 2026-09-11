// INTEGRATION: add `pub mod funding;` in nearai_credential/mod.rs. The API seam
// is CloudApi::funding_organization(&SessionTokens, &str) -> anyhow::Result<
// FundingOrganization>, whose public fields are id: String, name: String and
// is_active: bool. Dispatch the IPC body through parse_expected, then read;
// serialize FundingReport. Re-read with both expected fields on browser launch.
//! A verified organization destination for managing credits in Cloud.
//!
//! This module never selects a payer, chooses a network, or creates a payment.
//! Its revision identifies public inference-key metadata, not a credential or
//! proof of wallet ownership. A browser URL is returned only after checking the
//! selected key and retained session again following the management request.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::daemon::ipc::DaemonShared;
use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::nearai_credential::loopback::SessionTokens;
use crate::daemon::nearai_credential::session;
use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};

const CLOUD_ORIGIN: &str = "https://cloud.near.ai";
const MAX_ORGANIZATION_ID_BYTES: usize = 128;
const MAX_ORGANIZATION_NAME_BYTES: usize = 256;

/// Every refusal carries a fixed label and no destination from an older read.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum FundingReport {
    InvalidRequest,
    NoSession,
    NoInferenceKey,
    NoOrganization,
    SessionExpired,
    Changed,
    Unavailable,
    Ready {
        organization_id: String,
        organization_name: String,
        observed_at: DateTime<Utc>,
        connection_revision: String,
        browser_url: String,
    },
}

/// The destination the user actually saw before choosing Manage credits.
/// Constructed only by the strict IPC parser, with no caller-supplied URL.
pub struct ExpectedFunding {
    organization_id: String,
    connection_revision: String,
}

/// Accept an initial read (`{}`), or exactly both fields of a displayed report.
/// Unknown fields, nulls and partial expectations are refused rather than
/// degrading a click into an unbound initial read.
pub fn parse_expected(params: &Value) -> Result<Option<ExpectedFunding>, FundingReport> {
    let object = params.as_object().ok_or(FundingReport::InvalidRequest)?;
    if object.is_empty() {
        return Ok(None);
    }
    if object.len() != 2 {
        return Err(FundingReport::InvalidRequest);
    }
    let organization_id = object
        .get("expected_organization_id")
        .and_then(Value::as_str)
        .filter(|id| credits_url(id).is_some())
        .ok_or(FundingReport::InvalidRequest)?;
    let connection_revision = object
        .get("expected_connection_revision")
        .and_then(Value::as_str)
        .filter(|revision| {
            revision.len() == 64
                && revision
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        })
        .ok_or(FundingReport::InvalidRequest)?;
    Ok(Some(ExpectedFunding {
        organization_id: organization_id.to_owned(),
        connection_revision: connection_revision.to_owned(),
    }))
}

/// Build the one permitted browser destination from a bounded public ID.
pub fn credits_url(organization_id: &str) -> Option<String> {
    if organization_id.is_empty()
        || organization_id.len() > MAX_ORGANIZATION_ID_BYTES
        || !organization_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return None;
    }
    Some(format!(
        "{CLOUD_ORIGIN}/dashboard/organizations/{organization_id}/credits"
    ))
}

fn revision(key: &NearAiInferenceCredential) -> String {
    let minted_at = key.minted_at.to_rfc3339();
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-funding-context-v1");
    for field in [&key.key_id, &key.organization_id, &minted_at] {
        hash.update((field.len() as u64).to_be_bytes());
        hash.update(field.as_bytes());
    }
    format!("{:x}", hash.finalize())
}

fn report_for(error: &anyhow::Error) -> FundingReport {
    match error.to_string().as_str() {
        "near_ai_credential_session_missing" => FundingReport::NoSession,
        "near_ai_credential_session_expired" => FundingReport::SessionExpired,
        "near_ai_credential_session_changed" => FundingReport::Changed,
        "near_ai_credential_no_organization" => FundingReport::NoOrganization,
        _ => FundingReport::Unavailable,
    }
}

fn snapshot(
    shared: &DaemonShared,
) -> Result<(NearAiInferenceCredential, NearAiSession), FundingReport> {
    let locks = session::coordination(shared.store.dir()).map_err(|error| report_for(&error))?;
    let commit = locks.commit.lock().map_err(|error| report_for(&error))?;
    let settings =
        session::runtime_snapshot(shared, &commit).map_err(|error| report_for(&error))?;
    let retained = settings.near_ai_session.ok_or(FundingReport::NoSession)?;
    let key = settings
        .near_ai_inference
        .ok_or(FundingReport::NoInferenceKey)?;
    Ok((key, retained))
}

/// Verify the selected inference-key organization using the retained session.
/// The optional expected context binds a click to the destination already
/// displayed. Refresh rotation is persisted by the shared lifecycle before
/// its access token is used, and no commit lock spans either HTTP request.
pub async fn read(
    shared: &DaemonShared,
    api: &CloudApi,
    expected: Option<&ExpectedFunding>,
) -> FundingReport {
    let (key, retained) = match snapshot(shared) {
        Ok(connection) => connection,
        Err(report) => return report,
    };
    let Some(browser_url) = credits_url(&key.organization_id) else {
        return FundingReport::Unavailable;
    };
    let connection_revision = revision(&key);
    if let Some(expected) = expected
        && (expected.organization_id != key.organization_id
            || expected.connection_revision != connection_revision)
    {
        return FundingReport::Changed;
    }
    let access = match session::exchange(shared, api, &retained).await {
        Ok(access) => access,
        Err(error) => return report_for(&error),
    };
    let tokens = SessionTokens {
        access_token: access.access_token,
        // The management endpoint receives only the exchanged access token.
        refresh_token: String::new(),
    };
    let organization = match api
        .funding_organization(&tokens, &key.organization_id)
        .await
    {
        Ok(organization) => organization,
        Err(error) => return report_for(&error),
    };
    let (current_key, current_session) = match snapshot(shared) {
        Ok(connection) => connection,
        Err(report) => return report,
    };
    if current_key != key || current_session != access.retained {
        return FundingReport::Changed;
    }
    if organization.id != key.organization_id
        || organization.name.trim().is_empty()
        || organization.name.len() > MAX_ORGANIZATION_NAME_BYTES
        || organization.name.chars().any(char::is_control)
    {
        return FundingReport::Unavailable;
    }
    if !organization.is_active {
        return FundingReport::NoOrganization;
    }
    FundingReport::Ready {
        organization_id: organization.id,
        organization_name: organization.name,
        observed_at: Utc::now(),
        connection_revision,
        browser_url,
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    use axum::extract::{Path, State};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use chrono::Utc;
    use serde_json::{Value, json};
    use tokio::sync::oneshot;
    use tokio::task::JoinHandle;
    use tokio::time::timeout;

    use crate::config::tests_support::temp_store;
    use crate::daemon::ipc::{DaemonShared, Request};
    use crate::daemon::nearai_credential::api::CloudApi;
    use crate::daemon::nearai_credential::funding::{
        FundingReport, MAX_ORGANIZATION_ID_BYTES, MAX_ORGANIZATION_NAME_BYTES, credits_url,
        parse_expected, read, report_for, revision,
    };
    use crate::daemon::nearai_credential::handle_forget;
    use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};

    const DEADLINE: Duration = Duration::from_secs(10);
    const ORGANIZATION: &str = "org-synthetic-selected";

    #[tokio::test]
    async fn organization_lookup_rejects_oversized_stream_and_errors_before_eof() {
        use crate::daemon::nearai_credential::loopback::SessionTokens;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        for (status, body, expected) in [
            (
                200,
                vec![b' '; 256 * 1024 + 1],
                "near_ai_credential_unavailable",
            ),
            (401, Vec::new(), "near_ai_credential_session_expired"),
            (503, Vec::new(), "near_ai_credential_refused"),
        ] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api =
                CloudApi::for_test(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let (release, hold) = oneshot::channel::<()>();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = Vec::new();
                let mut byte = [0];
                while !request.ends_with(b"\r\n\r\n") {
                    socket.read_exact(&mut byte).await.unwrap();
                    request.extend_from_slice(&byte);
                    assert!(request.len() < 16 * 1024);
                }
                socket
                    .write_all(
                        format!(
                            "HTTP/1.1 {status} Synthetic\r\nTransfer-Encoding: chunked\r\n\r\n"
                        )
                        .as_bytes(),
                    )
                    .await
                    .unwrap();
                if !body.is_empty() {
                    socket
                        .write_all(format!("{:x}\r\n", body.len()).as_bytes())
                        .await
                        .unwrap();
                    let _ = socket.write_all(&body).await;
                    let _ = socket.write_all(b"\r\n").await;
                }
                // No terminating chunk: the client must refuse before EOF.
                let _ = hold.await;
            });
            let tokens = SessionTokens {
                access_token: "synthetic-access".into(),
                refresh_token: String::new(),
            };
            let result = timeout(
                Duration::from_secs(2),
                api.funding_organization(&tokens, ORGANIZATION),
            )
            .await;
            let _ = release.send(());
            server.abort();
            let error = result
                .expect("organization response waited for EOF")
                .err()
                .expect("response accepted");
            assert_eq!(error.to_string(), expected);
        }
    }

    fn key() -> NearAiInferenceCredential {
        NearAiInferenceCredential {
            key: "sk-synthetic-funding".into(),
            key_id: "key-synthetic-funding".into(),
            key_prefix: "sk-synthetic".into(),
            organization_id: ORGANIZATION.into(),
            workspace_id: "workspace-synthetic".into(),
            minted_at: chrono::DateTime::from_timestamp(1_780_000_000, 0).unwrap(),
        }
    }

    fn expected(key: &NearAiInferenceCredential) -> Value {
        json!({
            "expected_organization_id": key.organization_id,
            "expected_connection_revision": revision(key),
        })
    }

    #[test]
    fn request_requires_both_valid_expectations_or_an_empty_object() {
        assert!(matches!(parse_expected(&json!({})), Ok(None)));
        assert!(matches!(parse_expected(&expected(&key())), Ok(Some(_))));
        for value in [
            Value::Null,
            json!([]),
            json!("read"),
            json!({"url":"https://example.invalid"}),
            json!({"expected_organization_id": ORGANIZATION}),
            json!({"expected_connection_revision": "a".repeat(64)}),
            json!({"expected_organization_id":null,"expected_connection_revision":null}),
            json!({"expected_organization_id":42,"expected_connection_revision":"a".repeat(64)}),
            json!({"expected_organization_id":"../other","expected_connection_revision":"a".repeat(64)}),
            json!({"expected_organization_id":ORGANIZATION,"expected_connection_revision":"A".repeat(64)}),
            json!({"expected_organization_id":ORGANIZATION,"expected_connection_revision":"g".repeat(64)}),
            json!({"expected_organization_id":ORGANIZATION,"expected_connection_revision":"a".repeat(63)}),
            json!({"expected_organization_id":ORGANIZATION,"expected_connection_revision":"a".repeat(65)}),
            json!({"expected_organization_id":ORGANIZATION,"expected_connection_revision":"a".repeat(64),"extra":true}),
        ] {
            assert!(matches!(
                parse_expected(&value),
                Err(FundingReport::InvalidRequest)
            ));
        }
    }

    #[test]
    fn credits_urls_have_one_fixed_origin_and_one_bounded_path_segment() {
        for id in ["org-1_A".to_owned(), "a".repeat(MAX_ORGANIZATION_ID_BYTES)] {
            let url = url::Url::parse(&credits_url(&id).unwrap()).unwrap();
            assert_eq!(url.origin().ascii_serialization(), "https://cloud.near.ai");
            assert_eq!(url.path(), format!("/dashboard/organizations/{id}/credits"));
            assert!(url.query().is_none() && url.fragment().is_none());
            assert!(url.username().is_empty() && url.password().is_none());
        }
        for id in [
            "", ".", "..", "../org", "org/x", "org\\x", "%2f", "a?b", "a#b", "a@b", "a b", "a\n",
            "é", "a\0",
        ] {
            assert!(credits_url(id).is_none());
        }
        assert!(credits_url(&"a".repeat(MAX_ORGANIZATION_ID_BYTES + 1)).is_none());
    }

    #[test]
    fn revision_uses_framed_public_identity_without_hashing_credentials() {
        let original = key();
        let same = revision(&original);
        let mut changed = original.clone();
        changed.key = "sk-another-synthetic-secret".into();
        changed.key_prefix = "another-prefix".into();
        changed.workspace_id = "another-workspace".into();
        assert_eq!(same, revision(&changed));
        changed.key_id.push('x');
        assert_ne!(same, revision(&changed));
        changed = original.clone();
        changed.organization_id.push('x');
        assert_ne!(same, revision(&changed));
        changed = original.clone();
        changed.minted_at += chrono::Duration::nanoseconds(1);
        assert_ne!(same, revision(&changed));
        let mut left = original.clone();
        left.key_id = "ab".into();
        left.organization_id = "c".into();
        let mut right = original;
        right.key_id = "a".into();
        right.organization_id = "bc".into();
        assert_ne!(revision(&left), revision(&right));
    }

    #[test]
    fn failures_serialize_only_fixed_labels_and_never_a_previous_destination() {
        for report in [
            FundingReport::InvalidRequest,
            FundingReport::NoSession,
            FundingReport::NoInferenceKey,
            FundingReport::NoOrganization,
            FundingReport::SessionExpired,
            FundingReport::Changed,
            FundingReport::Unavailable,
        ] {
            let value = serde_json::to_value(report).unwrap();
            assert_eq!(value.as_object().unwrap().len(), 1);
            assert!(value["state"].is_string());
        }
        assert_eq!(
            report_for(&anyhow::anyhow!("untrusted upstream detail")),
            FundingReport::Unavailable
        );
    }

    struct Fixture {
        shared: Arc<DaemonShared>,
        _dir: tempfile::TempDir,
    }

    impl Fixture {
        fn new() -> Self {
            let (dir, store) = temp_store();
            Self {
                shared: Arc::new(DaemonShared::load(store).unwrap()),
                _dir: dir,
            }
        }

        fn install(&self, key: Option<NearAiInferenceCredential>, session: bool) {
            let now = Utc::now();
            let mut settings = self.shared.settings.lock().unwrap();
            settings.near_ai_inference = key;
            settings.near_ai_session = session.then(|| NearAiSession {
                refresh_token: "rt-synthetic-funding".into(),
                refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
                stored_at: now,
            });
            settings.save_for_test(&self.shared.store).unwrap();
        }

        async fn read(&self, cloud: &Cloud, params: Value) -> FundingReport {
            let expected =
                parse_expected(&params).unwrap_or_else(|_| panic!("valid fixture request"));
            timeout(DEADLINE, read(&self.shared, &cloud.api, expected.as_ref()))
                .await
                .expect("funding read timed out")
        }
    }

    struct Gate {
        entered: oneshot::Sender<()>,
        release: oneshot::Receiver<()>,
    }

    struct CloudState {
        shared: Arc<DaemonShared>,
        requests: Mutex<Vec<String>>,
        rotations: Mutex<u32>,
        refresh_status: StatusCode,
        response: Value,
        organization_status: StatusCode,
        gate: Mutex<Option<Gate>>,
    }

    struct Cloud {
        api: CloudApi,
        state: Arc<CloudState>,
        server: JoinHandle<()>,
    }

    impl Cloud {
        async fn start(
            fixture: &Fixture,
            response: Value,
            refresh_status: StatusCode,
            organization_status: StatusCode,
            gate: Option<Gate>,
        ) -> Self {
            let state = Arc::new(CloudState {
                shared: Arc::clone(&fixture.shared),
                requests: Mutex::new(Vec::new()),
                rotations: Mutex::new(0),
                refresh_status,
                response,
                organization_status,
                gate: Mutex::new(gate),
            });
            let router = Router::new()
                .route("/v1/users/me/access-tokens", post(refresh))
                .route("/v1/organizations/{id}", get(organization))
                .with_state(Arc::clone(&state));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let api =
                CloudApi::for_test(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
            let server = tokio::spawn(async move {
                axum::serve(listener, router).await.unwrap();
            });
            Self { api, state, server }
        }

        async fn normal(fixture: &Fixture) -> Self {
            Self::start(
                fixture,
                json!({"id":ORGANIZATION,"name":"Synthetic organization","is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                None,
            )
            .await
        }
    }

    impl Drop for Cloud {
        fn drop(&mut self) {
            self.server.abort();
        }
    }

    fn bearer(headers: &HeaderMap) -> &str {
        headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .strip_prefix("Bearer ")
            .unwrap()
    }

    async fn refresh(
        State(state): State<Arc<CloudState>>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        state.requests.lock().unwrap().push("refresh".into());
        if state.refresh_status != StatusCode::OK {
            return (
                state.refresh_status,
                Json(json!({"error":"synthetic refusal"})),
            );
        }
        let mut rotations = state.rotations.lock().unwrap();
        let previous = *rotations;
        let expected = if previous == 0 {
            "rt-synthetic-funding".into()
        } else {
            format!("rt-synthetic-rotated-{previous}")
        };
        assert!(
            bearer(&headers) == expected,
            "refresh must use the retained refresh token"
        );
        *rotations += 1;
        let rotation = *rotations;
        (
            StatusCode::OK,
            Json(json!({
                "access_token":format!("synthetic-access-{rotation}"),
                "refresh_token":format!("rt-synthetic-rotated-{rotation}"),
                "refresh_token_expiration":(Utc::now()+chrono::Duration::hours(1)).to_rfc3339(),
            })),
        )
    }

    async fn organization(
        State(state): State<Arc<CloudState>>,
        Path(id): Path<String>,
        headers: HeaderMap,
    ) -> (StatusCode, Json<Value>) {
        state
            .requests
            .lock()
            .unwrap()
            .push(format!("organization:{id}"));
        let rotations = *state.rotations.lock().unwrap();
        assert!(bearer(&headers) == format!("synthetic-access-{rotations}"));
        let retained = state
            .shared
            .settings
            .lock()
            .unwrap()
            .near_ai_session
            .clone()
            .unwrap();
        assert!(
            retained.refresh_token == format!("rt-synthetic-rotated-{rotations}"),
            "rotation must reach settings before the organization request"
        );
        let persisted = DaemonSettings::load_with_cloud_credentials(&state.shared.store).unwrap();
        assert!(
            persisted.near_ai_session.as_ref() == Some(&retained),
            "rotation must reach persistent storage before the organization request"
        );
        let gate = state.gate.lock().unwrap().take();
        if let Some(gate) = gate {
            let _ = gate.entered.send(());
            let _ = gate.release.await;
        }
        (state.organization_status, Json(state.response.clone()))
    }

    #[tokio::test]
    async fn verified_destination_is_bound_to_selected_key_and_contains_no_payer_or_tokens() {
        let fixture = Fixture::new();
        fixture.install(Some(key()), true);
        let cloud = Cloud::normal(&fixture).await;
        let before = Utc::now();
        let report = fixture.read(&cloud, json!({})).await;
        let value = serde_json::to_value(&report).unwrap();
        assert_eq!(value["state"], "ready");
        assert_eq!(value["organization_id"], ORGANIZATION);
        assert_eq!(value["organization_name"], "Synthetic organization");
        assert_eq!(value["browser_url"], credits_url(ORGANIZATION).unwrap());
        assert_eq!(value["connection_revision"], revision(&key()));
        let FundingReport::Ready { observed_at, .. } = report else {
            panic!("ready report expected")
        };
        assert!(observed_at >= before && observed_at <= Utc::now());
        for field in ["payer", "network", "access_token", "refresh_token", "key"] {
            assert!(value.get(field).is_none());
        }
        assert!(!value.to_string().contains("synthetic-access"));
        assert!(!value.to_string().contains("rt-synthetic"));
        assert_eq!(
            *cloud.state.requests.lock().unwrap(),
            ["refresh", &format!("organization:{ORGANIZATION}")]
        );
        assert!(matches!(
            fixture.read(&cloud, expected(&key())).await,
            FundingReport::Ready { .. }
        ));
        assert_eq!(*cloud.state.rotations.lock().unwrap(), 2);
    }

    #[tokio::test]
    async fn missing_credentials_and_stale_expectations_make_no_cloud_request() {
        let fixture = Fixture::new();
        let cloud = Cloud::normal(&fixture).await;
        assert_eq!(
            fixture.read(&cloud, json!({})).await,
            FundingReport::NoSession
        );
        fixture.install(Some(key()), false);
        assert_eq!(
            fixture.read(&cloud, json!({})).await,
            FundingReport::NoSession
        );
        fixture.install(None, true);
        assert_eq!(
            fixture.read(&cloud, json!({})).await,
            FundingReport::NoInferenceKey
        );
        fixture.install(Some(key()), true);
        let mut changed = key();
        changed.key_id.push('x');
        assert_eq!(
            fixture.read(&cloud, expected(&changed)).await,
            FundingReport::Changed
        );
        changed = key();
        changed.organization_id = "org-other".into();
        assert_eq!(
            fixture.read(&cloud, expected(&changed)).await,
            FundingReport::Changed
        );
        let mut invalid = key();
        invalid.organization_id = "../other".into();
        // The test store rejects invalid metadata, so exercise the URL boundary
        // directly instead of weakening persistence to manufacture that state.
        assert!(credits_url(&invalid.organization_id).is_none());
        fixture
            .shared
            .settings
            .lock()
            .unwrap()
            .cloud_storage_unavailable = true;
        assert_eq!(
            fixture.read(&cloud, json!({})).await,
            FundingReport::Unavailable
        );
        assert!(cloud.state.requests.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn cloud_refusals_and_invalid_organization_metadata_never_return_a_url() {
        for (response, refresh_status, organization_status, expected_report) in [
            (
                json!({}),
                StatusCode::UNAUTHORIZED,
                StatusCode::OK,
                FundingReport::SessionExpired,
            ),
            (
                json!({}),
                StatusCode::SERVICE_UNAVAILABLE,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
            (
                json!({}),
                StatusCode::OK,
                StatusCode::UNAUTHORIZED,
                FundingReport::SessionExpired,
            ),
            (
                json!({}),
                StatusCode::OK,
                StatusCode::FORBIDDEN,
                FundingReport::Unavailable,
            ),
            (
                json!({}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
            (
                json!({"id":"org-other","name":"Other","is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
            (
                json!({"id":ORGANIZATION,"name":"Synthetic","is_active":false}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::NoOrganization,
            ),
            (
                json!({"id":ORGANIZATION,"name":" \t","is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
            (
                json!({"id":ORGANIZATION,"name":"Synthetic\nextra","is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
            (
                json!({"id":ORGANIZATION,"name":"a".repeat(MAX_ORGANIZATION_NAME_BYTES+1),"is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                FundingReport::Unavailable,
            ),
        ] {
            let fixture = Fixture::new();
            fixture.install(Some(key()), true);
            let cloud = Cloud::start(
                &fixture,
                response,
                refresh_status,
                organization_status,
                None,
            )
            .await;
            assert_eq!(fixture.read(&cloud, json!({})).await, expected_report);
        }
    }

    #[tokio::test]
    async fn forget_or_replacement_during_lookup_suppresses_the_old_destination() {
        for mutation in ["forget", "key", "session"] {
            let fixture = Fixture::new();
            fixture.install(Some(key()), true);
            let (entered, arrived) = oneshot::channel();
            let (release, wait) = oneshot::channel();
            let cloud = Cloud::start(
                &fixture,
                json!({"id":ORGANIZATION,"name":"Synthetic","is_active":true}),
                StatusCode::OK,
                StatusCode::OK,
                Some(Gate {
                    entered,
                    release: wait,
                }),
            )
            .await;
            let pending = read(&fixture.shared, &cloud.api, None);
            tokio::pin!(pending);
            tokio::select! {
                result = timeout(DEADLINE, arrived) => { result.unwrap().unwrap(); },
                _ = &mut pending => panic!("lookup returned before the response gate"),
            }
            if mutation == "forget" {
                let response = handle_forget(
                    &fixture.shared,
                    &Request {
                        id: 1,
                        method: "near_ai_credential_forget".into(),
                        params: json!({}),
                    },
                );
                assert!(response.error.is_none());
            } else {
                let mut settings = fixture.shared.settings.lock().unwrap();
                if mutation == "key" {
                    settings.near_ai_inference.as_mut().unwrap().key_id = "key-replacement".into();
                } else {
                    settings.near_ai_session.as_mut().unwrap().refresh_token =
                        "rt-synthetic-replacement".into();
                }
                settings.save_for_test(&fixture.shared.store).unwrap();
            }
            release.send(()).unwrap();
            assert_eq!(
                timeout(DEADLINE, pending).await.unwrap(),
                if mutation == "forget" {
                    FundingReport::NoSession
                } else {
                    FundingReport::Changed
                }
            );
        }
    }
}
