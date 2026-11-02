// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/account/near-ai-login`: recording the NEAR AI login on an
//! existing account (owner decision 2026-11-02, option (a) on #1841).
//!
//! Real: the router and account middleware, a native session, the store
//! over PostgreSQL. Stubbed: NEAR AI's `/users/me`, through the `cfg(test)`
//! introspection seam. Every value is synthetic. PostgreSQL-backed,
//! self-skipping without `TRACE_COMMONS_PG_TEST_DATABASE_URL`, and run with
//! `--test-threads=1`; each test uses tenants of its own.

use super::*;
use axum::body::Body;
use std::sync::Mutex;
use tower::ServiceExt;

const RECORD: &str = "/v1/account/near-ai-login";
const NEAR_AI_TOKEN: &str = "synthetic-near-ai-access-token-TOKENSENTINEL";

fn identity() -> Arc<trace_commons_server::near_account_identity::NearAccountIdentity> {
    use base64::Engine as _;
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("r".repeat(32)))
            .expect("fixture SecretsCrypto");
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "near-ai-login-record-fixture",
        ),
    );
    Arc::new(
        trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([7u8; 32])),
            Some(kek),
        )
        .expect("fixture identity"),
    )
}

/// What the stub `/users/me` answers, and how often it was asked.
struct NearAiStub {
    subject: String,
    /// `None` answers 200; `Some(status)` answers that status.
    status: Option<u16>,
    calls: usize,
}

type Stub = Arc<Mutex<NearAiStub>>;

async fn stub_near_ai(stub: Stub) -> String {
    let users_me = move || {
        let stub = stub.clone();
        async move {
            use axum::response::IntoResponse;
            let mut stub = stub.lock().unwrap();
            stub.calls += 1;
            match stub.status {
                Some(status) => StatusCode::from_u16(status).unwrap().into_response(),
                None => axum::Json(serde_json::json!({
                    "id": stub.subject, "auth_provider": "github",
                    "email": "synthetic@example.invalid",
                }))
                .into_response(),
            }
        }
    };
    let router = axum::Router::new().route("/users/me", axum::routing::get(users_me));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

struct Harness {
    backend: Arc<PgBackend>,
    state: Arc<AppState>,
    stub: Stub,
    _root: tempfile::TempDir,
}

struct Account {
    tenant: String,
    id: Uuid,
    token: String,
}

async fn harness() -> Option<Harness> {
    let backend = required_postgres_backend_for_ingest_test().await?;
    let stub: Stub = Arc::new(Mutex::new(NearAiStub {
        subject: fresh_subject(),
        status: None,
        calls: 0,
    }));
    let base = stub_near_ai(stub.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let mut state = test_state(root.path().to_path_buf());
    let inner = Arc::get_mut(&mut state).unwrap();
    inner.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    inner.near_account_identity = Some(identity());
    inner.near_ai_introspection_base_url = Some(base);
    Some(Harness {
        backend,
        state,
        stub,
        _root: root,
    })
}

fn fresh_subject() -> String {
    format!("synthetic-subject-{}", Uuid::new_v4().simple())
}

impl Harness {
    /// An open account in a fresh `near-` (wallet) tenant, with a weak
    /// native session and no NEAR AI login anchor.
    async fn wallet_account(&self) -> Account {
        self.account_in(&trace_commons_server::near_account_identity::random_near_tenant_id())
            .await
    }

    async fn account_in(&self, tenant: &str) -> Account {
        let id = Uuid::new_v4();
        {
            let mut client = self.backend.trace_pool_for_test().get().await.unwrap();
            let tx = client.transaction().await.unwrap();
            tx.execute(
                "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.execute(
                "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT DO NOTHING",
                &[&tenant],
            )
            .await
            .unwrap();
            tx.execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &id],
            )
            .await
            .unwrap();
            tx.commit().await.unwrap();
        }
        let secret = generate_session_secret();
        self.backend
            .issue_native_session(
                tenant,
                id,
                trace_commons_server::db::NewSession {
                    token_hash: &hash_secret(&secret),
                    client_kind: NATIVE_SESSION_CLIENT_KIND,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                trace_commons_server::db::RedeemAudit {
                    action: "account_native_login".to_string(),
                    outcome: "success".to_string(),
                    metadata: serde_json::json!({}),
                },
            )
            .await
            .unwrap();
        Account {
            tenant: tenant.to_owned(),
            id,
            token: native_token_value(tenant, &secret),
        }
    }

    async fn call(
        &self,
        bearer: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, HeaderMap, serde_json::Value) {
        let mut request = axum::http::Request::builder()
            .method("POST")
            .uri(RECORD)
            .header(CONTENT_TYPE, "application/json");
        if let Some(bearer) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let response = app(self.state.clone())
            .oneshot(request.body(Body::from(body.to_string())).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn record(&self, account: &Account) -> (StatusCode, serde_json::Value) {
        let (status, _, body) = self
            .call(
                Some(&account.token),
                serde_json::json!({ "near_ai_access_token": NEAR_AI_TOKEN }),
            )
            .await;
        (status, body)
    }

    fn stub(&self) -> std::sync::MutexGuard<'_, NearAiStub> {
        self.stub.lock().unwrap()
    }

    /// The account's NEAR AI login anchors.
    async fn anchors(&self, account: &Account) -> Vec<String> {
        trace_commons_server::db::PayoutStore::payout_account_login_anchors(
            self.backend.as_ref(),
            &account.tenant,
            account.id,
        )
        .await
        .unwrap()
    }

    /// Every anchor and account-audit row of the tenant, as text.
    async fn dump(&self, tenant: &str) -> String {
        let client = self
            .backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let mut out = String::new();
        for table in ["trace_near_account_anchors", "trace_account_audit"] {
            for row in client
                .query(
                    &format!("SELECT row_to_json(t)::TEXT FROM {table} t WHERE tenant_id = $1"),
                    &[&tenant],
                )
                .await
                .unwrap()
            {
                out.push_str(row.get::<_, &str>(0));
                out.push('\n');
            }
        }
        out
    }
}

#[tokio::test]
async fn a_wallet_account_records_its_near_ai_login_and_a_repeat_changes_nothing() {
    let Some(h) = harness().await else { return };
    let account = h.wallet_account().await;
    let subject = h.stub().subject.clone();
    let (status, headers, body) = h
        .call(
            Some(&account.token),
            serde_json::json!({ "near_ai_access_token": NEAR_AI_TOKEN }),
        )
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "recorded", "{body}");
    assert_eq!(
        headers.get(axum::http::header::CACHE_CONTROL).unwrap(),
        "no-store"
    );
    assert_eq!(
        h.anchors(&account).await,
        vec![identity().login_index_label(&subject)]
    );
    let dump = h.dump(&account.tenant).await;
    assert!(dump.contains("near_ai_login_recorded"), "{dump}");
    // The anchor row names its account, as every anchor row does; the
    // audit row and the answer name nothing.
    let audit: Vec<_> = dump
        .lines()
        .filter(|line| line.contains("near_ai_login_recorded"))
        .collect();
    assert_eq!(audit.len(), 1, "{dump}");
    assert!(audit[0].contains(r#""actor_ref":"sha256:"#), "{dump}");
    for secret in [subject.as_str(), NEAR_AI_TOKEN] {
        assert!(!dump.contains(secret), "{dump}");
    }
    for secret in [
        subject.as_str(),
        NEAR_AI_TOKEN,
        "github",
        &account.id.to_string(),
    ] {
        assert!(!audit[0].contains(secret), "{dump}");
        assert!(!body.to_string().contains(secret), "{body}");
    }

    let (status, body) = h.record(&account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "already_recorded", "{body}");
    assert_eq!(h.dump(&account.tenant).await, dump);
}

#[tokio::test]
async fn another_login_or_a_taken_one_is_refused_with_one_label() {
    let Some(h) = harness().await else { return };
    let first = h.wallet_account().await;
    let (status, _) = h.record(&first).await;
    assert_eq!(status, StatusCode::OK);

    // The same login on a second account: taken.
    let second = h.wallet_account().await;
    let (status, taken) = h.record(&second).await;
    assert_eq!(status, StatusCode::CONFLICT, "{taken}");
    assert_eq!(taken["error"], "near_ai_login_unavailable", "{taken}");
    assert!(h.anchors(&second).await.is_empty());

    // A different login on the first account: it already has one.
    h.stub().subject = fresh_subject();
    let before = h.dump(&first.tenant).await;
    let (status, other) = h.record(&first).await;
    assert_eq!(status, StatusCode::CONFLICT, "{other}");
    assert_eq!(other, taken, "the two causes answer alike");
    assert_eq!(h.dump(&first.tenant).await, before);
}

#[tokio::test]
async fn an_account_outside_a_near_tenant_is_refused_before_the_token_is_spent() {
    let Some(h) = harness().await else { return };
    let legacy = h
        .account_in(&format!("tenant-legacy-{}", Uuid::new_v4().simple()))
        .await;
    let (status, body) = h.record(&legacy).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "near_ai_login_account_ineligible", "{body}");
    assert_eq!(h.stub().calls, 0);
    assert!(!h.dump(&legacy.tenant).await.contains("near_ai_login"));
}

#[tokio::test]
async fn an_unverified_token_a_bad_body_and_a_device_bearer_write_nothing() {
    let Some(h) = harness().await else { return };
    let account = h.wallet_account().await;

    h.stub().status = Some(401);
    let (status, body) = h.record(&account).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "near_ai_login_unverified", "{body}");
    h.stub().status = None;
    let calls = h.stub().calls;

    for bad in [
        serde_json::json!({}),
        serde_json::json!({ "near_ai_access_token": "" }),
        serde_json::json!({ "near_ai_access_token": NEAR_AI_TOKEN, "account_id": "x" }),
    ] {
        let (status, _, body) = h.call(Some(&account.token), bad.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{bad} {body}");
        assert_eq!(body["error"], "invalid_near_ai_login_request", "{body}");
    }
    let (status, _, _) = h
        .call(
            None,
            serde_json::json!({ "near_ai_access_token": NEAR_AI_TOKEN }),
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let (status, _, _) = h
        .call(
            Some("token-a"),
            serde_json::json!({ "near_ai_access_token": NEAR_AI_TOKEN }),
        )
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    assert_eq!(h.stub().calls, calls, "no refused call spends the token");
    assert!(h.anchors(&account).await.is_empty());
}
