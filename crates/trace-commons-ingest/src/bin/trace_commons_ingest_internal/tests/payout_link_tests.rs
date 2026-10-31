// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout link routes (payout spec section 5): start, confirm and
//! status, on the contributor's signed-in account session.
//!
//! The owner superseded the passkey confirmation (D5) on 2026-10-23: a
//! signed-in session links, changes and unlinks, with no passkey and no
//! step-up page. What still stands behind a link is the server's own check
//! with NEAR AI (`/users/me` with the person's token, then a zero-target
//! probe), the "changed since shown" version check, and an audit row for
//! every request and confirmation.
//!
//! Real: the router and account middleware, a native session, the payout
//! store over PostgreSQL, the recipient seal, the `near_ai` rail and its
//! transfer client. Stubbed: NEAR AI, by one loopback server answering
//! `GET /users/me` (through the `cfg(test)` introspection seam) and the
//! #1271 transfer route (through the rail's loopback client). Every value is
//! synthetic.
//!
//! PostgreSQL-backed, self-skipping without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL`, and, like the rest of the suite,
//! run with `--test-threads=1`. Each test uses tenants of its own, never the
//! shared `tenant-a`, so its payout rows cannot block another test's tenant
//! cleanup.

use super::*;
use axum::body::Body;
use std::sync::Mutex;
use tower::ServiceExt;
use trace_commons_gate_api::{RailId, RecipientKind};
use trace_commons_protocol::payout_link::near_ai_recipient_ref;
use trace_commons_server::db::{
    PayoutAuditAction, PayoutLinkInsert, PayoutLinkState, PayoutLinkWrite, PayoutStore,
    SealedRecipient,
};
use trace_commons_server::near_ai_rail_config::{NearAiRailConfig, NearAiRailSettings};
use trace_commons_server::near_ai_transfer::{NearAiCreditSource, NearAiTransferCredentials};
use trace_commons_server::payout_pass::PayoutRecipientOpener;
use trace_commons_server::payout_rate::PayoutMode;
use trace_commons_server::payout_recipient_seal::SealedRecipientOpener;
use trace_commons_server::payout_worker::PayoutRails;

const LINK_REQUESTS: &str = "/v1/account/payout-rails/near_ai/link-requests";
const STATUS: &str = "/v1/account/payout-rails";
const NEAR_AI_TOKEN: &str = "synthetic-near-ai-access-token-TOKENSENTINEL";
const NAME_SENTINEL: &str = "ORGNAMESENTINEL";
const CREDENTIAL: &str = "oc_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

fn org(n: u128) -> Uuid {
    Uuid::from_u128(0x4ec1_0000_0000_4000_8000_0000_0000_0000 + n)
}

fn identity() -> Arc<trace_commons_server::near_account_identity::NearAccountIdentity> {
    use base64::Engine as _;
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("p".repeat(32)))
            .expect("fixture SecretsCrypto");
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "payout-link-fixture",
        ),
    );
    Arc::new(
        trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
            Some(&base64::engine::general_purpose::STANDARD.encode([6u8; 32])),
            Some(kek),
        )
        .expect("fixture identity"),
    )
}

/// What the NEAR AI stub answers, and what it saw.
struct NearAiStub {
    subject: String,
    /// `(org id, role, is_active)` per org on `/users/me`.
    orgs: Vec<(Uuid, &'static str, bool)>,
    /// The probe's reply: `None` is a 200 `no_op`.
    probe_error: Option<(u16, &'static str)>,
    users_me_calls: usize,
    probe_calls: usize,
}

type Stub = Arc<Mutex<NearAiStub>>;

async fn stub_near_ai(stub: Stub) -> String {
    let users_me = {
        let stub = stub.clone();
        move || {
            let stub = stub.clone();
            async move {
                let mut stub = stub.lock().unwrap();
                stub.users_me_calls += 1;
                let orgs: Vec<_> = stub
                    .orgs
                    .iter()
                    .map(|(id, role, active)| {
                        serde_json::json!({
                            "id": id.to_string(), "name": NAME_SENTINEL,
                            "description": NAME_SENTINEL, "role": role,
                            "is_active": active, "created_at": "2026-10-01T00:00:00Z",
                        })
                    })
                    .collect();
                axum::Json(serde_json::json!({
                    "id": stub.subject, "email": "synthetic@example.invalid",
                    "username": NAME_SENTINEL, "auth_provider": "github",
                    "organizations": orgs, "workspaces": [],
                }))
            }
        }
    };
    let transfer = {
        let stub = stub.clone();
        move || {
            let stub = stub.clone();
            async move {
                use axum::response::IntoResponse;
                let mut stub = stub.lock().unwrap();
                stub.probe_calls += 1;
                match stub.probe_error {
                    None => axum::Json(serde_json::json!({
                        "amount": 0, "recipient_cumulative": 0, "no_op": true,
                    }))
                    .into_response(),
                    Some((status, code)) => (
                        StatusCode::from_u16(status).unwrap(),
                        axum::Json(serde_json::json!({"error": {"type": code, "message": code}})),
                    )
                        .into_response(),
                }
            }
        }
    };
    let router = axum::Router::new()
        .route("/users/me", axum::routing::get(users_me))
        .route(
            "/organizations/{org}/credit/transfer",
            axum::routing::post(transfer),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    base
}

fn near_ai_rails(base: &str, mode: PayoutMode) -> PayoutRails {
    if mode == PayoutMode::Disabled {
        return PayoutRails::disabled();
    }
    PayoutRails::from_config_at_loopback(
        &NearAiRailConfig::Enabled(NearAiRailSettings {
            mode,
            credentials: NearAiTransferCredentials {
                token: SecretString::from(CREDENTIAL),
                credential_id: Uuid::from_u128(2),
                sender_org: Uuid::from_u128(3),
                source: NearAiCreditSource::Payment,
            },
            account_ceiling: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(
                1_000_000_000_000,
            ),
        }),
        base,
    )
    .unwrap()
}

struct Harness {
    backend: Arc<PgBackend>,
    state: Arc<AppState>,
    stub: Stub,
    _root: tempfile::TempDir,
}

/// A signed-in account: a tenant of its own, an open account row with no
/// passkey and no binding row, and a weak native session for it.
struct Account {
    tenant: String,
    id: Uuid,
    token: String,
}

async fn harness_with(mode: PayoutMode) -> Option<Harness> {
    let backend = required_postgres_backend_for_ingest_test().await?;
    let stub: Stub = Arc::new(Mutex::new(NearAiStub {
        subject: format!("synthetic-subject-{}", Uuid::new_v4().simple()),
        orgs: vec![(org(1), "owner", true), (org(2), "owner", true)],
        probe_error: None,
        users_me_calls: 0,
        probe_calls: 0,
    }));
    let base = stub_near_ai(stub.clone()).await;
    let root = tempfile::tempdir().unwrap();
    let mut state = test_state(root.path().to_path_buf());
    let inner = Arc::get_mut(&mut state).unwrap();
    inner.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    inner.payout_store = Some(backend.clone() as Arc<dyn PayoutStore>);
    inner.near_account_identity = Some(identity());
    inner.near_ai_introspection_base_url = Some(base.clone());
    inner.payout_rails = Arc::new(near_ai_rails(&base, mode));
    Some(Harness {
        backend,
        state,
        stub,
        _root: root,
    })
}

async fn harness() -> Option<Harness> {
    harness_with(PayoutMode::DryRun).await
}

impl Harness {
    async fn account(&self) -> Account {
        self.account_in(&format!("payout-link-{}", Uuid::new_v4().simple()))
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
        method: &str,
        uri: &str,
        bearer: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut request = axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(CONTENT_TYPE, "application/json");
        if let Some(bearer) = bearer {
            request = request.header(AUTHORIZATION, format!("Bearer {bearer}"));
        }
        let body = body.map_or_else(Body::empty, |body| Body::from(body.to_string()));
        let response = app(self.state.clone())
            .oneshot(request.body(body).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 256 * 1024)
            .await
            .unwrap();
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    async fn start(
        &self,
        account: &Account,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        self.call("POST", LINK_REQUESTS, Some(&account.token), Some(body))
            .await
    }

    async fn confirm(&self, account: &Account, handle: &str) -> (StatusCode, serde_json::Value) {
        self.call(
            "POST",
            &format!("{LINK_REQUESTS}/{handle}/confirm"),
            Some(&account.token),
            None,
        )
        .await
    }

    async fn status(&self, account: &Account) -> (StatusCode, serde_json::Value) {
        self.call("GET", STATUS, Some(&account.token), None).await
    }

    async fn version(&self, account: &Account) -> i64 {
        let (status, body) = self.status(account).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        body["rails"][0]["version"].as_i64().unwrap()
    }

    /// Starts and confirms a link to `org` and returns the idempotency key.
    async fn link(&self, account: &Account, org: Uuid) -> Uuid {
        let key = Uuid::new_v4();
        let version = self.version(account).await;
        let (status, body) = self.start(account, link_body(key, org, version)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let (status, body) = self
            .confirm(account, body["handle"].as_str().unwrap())
            .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        key
    }

    async fn links(
        &self,
        account: &Account,
        states: &[PayoutLinkState],
    ) -> Vec<trace_commons_server::db::PayoutLink> {
        self.backend
            .payout_links(&account.tenant, &RailId::near_ai(), states)
            .await
            .unwrap()
            .into_iter()
            .filter(|link| link.account_id == account.id)
            .collect()
    }

    async fn request_rows(&self, account: &Account) -> i64 {
        self.backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap()
            .query_one(
                "SELECT count(*) FROM trace_payout_link_requests WHERE tenant_id = $1",
                &[&account.tenant],
            )
            .await
            .unwrap()
            .get(0)
    }

    async fn audit(&self, account: &Account) -> Vec<PayoutAuditAction> {
        self.backend
            .payout_audit_actions(&account.tenant, &RailId::near_ai())
            .await
            .unwrap()
    }

    fn stub(&self) -> std::sync::MutexGuard<'_, NearAiStub> {
        self.stub.lock().unwrap()
    }

    /// Every payout row of the tenant, as text.
    async fn dump(&self, tenant: &str) -> String {
        let client = self
            .backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .unwrap();
        let mut out = String::new();
        for table in [
            "trace_payout_links",
            "trace_payout_link_requests",
            "trace_payout_audit",
        ] {
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

fn link_body(key: Uuid, org: Uuid, version: i64) -> serde_json::Value {
    serde_json::json!({
        "action": "link",
        "idempotency_key": key,
        "expected_current_version": version,
        "proof": {
            "rail": "near_ai",
            "organization_id": org,
            "near_ai_access_token": NEAR_AI_TOKEN,
        },
    })
}

fn unlink_body(key: Uuid, version: i64) -> serde_json::Value {
    serde_json::json!({
        "action": "unlink", "idempotency_key": key, "expected_current_version": version,
    })
}

#[tokio::test]
async fn a_link_request_with_an_owned_active_org_is_pending_confirmation_and_changes_nothing() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let key = Uuid::new_v4();
    let (status, body) = h.start(&account, link_body(key, org(1), 0)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "pending_confirmation", "{body}");
    assert_eq!(body["rail_id"], "near_ai");
    assert_eq!(body["action"], "link");
    assert_eq!(body["recipient_ref"], near_ai_recipient_ref(key, org(1)));
    let handle = body["handle"].as_str().unwrap();
    assert_eq!(handle.len(), 64, "{body}");
    assert!(!body.to_string().contains(&org(1).to_string()), "{body}");
    assert_eq!(h.stub().users_me_calls, 1);
    assert_eq!(h.stub().probe_calls, 1);
    // Nothing changed yet: no link, one request.
    assert!(
        h.links(
            &account,
            &[PayoutLinkState::Waiting, PayoutLinkState::Active]
        )
        .await
        .is_empty()
    );
    assert_eq!(h.request_rows(&account).await, 1);
    let (_, status) = h.status(&account).await;
    assert_eq!(status["rails"][0]["linked"], false, "{status}");
    assert_eq!(
        status["rails"][0]["request_state"], "pending_confirmation",
        "{status}"
    );

    // A replay of the same request answers the same handle and adds nothing.
    let (status, replay) = h.start(&account, link_body(key, org(1), 0)).await;
    assert_eq!(status, StatusCode::OK, "{replay}");
    assert_eq!(replay["handle"], body["handle"]);
    assert_eq!(h.request_rows(&account).await, 1);
    // The same key with a different org is a conflict.
    let (status, conflict) = h.start(&account, link_body(key, org(2), 0)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{conflict}");
    assert_eq!(conflict["error"], "idempotency_conflict");
}

#[tokio::test]
async fn an_account_links_from_its_weak_signed_in_session() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    // The session is the weak native kind.
    let mut headers = HeaderMap::new();
    headers.insert(
        AUTHORIZATION,
        HeaderValue::from_str(&format!("Bearer {}", account.token)).unwrap(),
    );
    let ctx = resolve_account_ctx(h.state.as_ref(), &headers)
        .await
        .unwrap();
    assert!(!ctx.is_strong_session());

    let key = h.link(&account, org(1)).await;
    let waiting = h.links(&account, &[PayoutLinkState::Waiting]).await;
    assert_eq!(waiting.len(), 1);
    assert_eq!(
        waiting[0].recipient.recipient_ref.as_deref(),
        Some(near_ai_recipient_ref(key, org(1)).as_str())
    );
    // The link opens to the org chosen, with the deployment's key: the pass
    // can pay it.
    let opener = SealedRecipientOpener::new(identity());
    assert_eq!(
        opener.open(&waiting[0]).unwrap().expose(),
        org(1).to_string()
    );
    let (_, status) = h.status(&account).await;
    let rail = &status["rails"][0];
    assert_eq!(rail["linked"], true, "{status}");
    assert_eq!(rail["state"], "waiting", "{status}");
    assert_eq!(rail["version"], 1, "{status}");
    assert_eq!(rail["recipient_ref"], near_ai_recipient_ref(key, org(1)));
    assert!(rail["request_state"].is_null(), "{status}");
}

#[tokio::test]
async fn a_non_owner_or_inactive_org_is_refused() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    h.stub().orgs = vec![
        (org(1), "admin", true),
        (org(2), "owner", false),
        (org(3), "member", true),
    ];
    for target in [org(1), org(2), org(3), org(4)] {
        let (status, body) = h
            .start(&account, link_body(Uuid::new_v4(), target, 0))
            .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error"], "org_not_owned", "{body}");
    }
    assert_eq!(h.stub().probe_calls, 0, "a refused org is never probed");
    assert_eq!(h.request_rows(&account).await, 0);
}

#[tokio::test]
async fn a_nearai_tenant_with_a_different_subject_is_refused() {
    let Some(h) = harness().await else { return };
    let tenant = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
    let account = h.account_in(&tenant).await;
    let anchored = format!("synthetic-anchored-{}", Uuid::new_v4().simple());
    h.backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "INSERT INTO trace_near_account_anchors
                (tenant_id, account_id, anchor_hash, sealed_account_name, index_pepper_ref,
                 account_name_key_ref, identity_source, auth_provider)
             VALUES ($1, $2, $3, $4, 'fixture-pepper', 'fixture-key', 'near_ai_login',
                     'github')",
            &[
                &tenant,
                &account.id,
                &identity().login_index_label(&anchored),
                &serde_json::json!({"fixture": true}),
            ],
        )
        .await
        .unwrap();
    let (status, body) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error"], "subject_mismatch", "{body}");
    assert_eq!(h.stub().probe_calls, 0);
    assert_eq!(h.request_rows(&account).await, 0);

    // The anchored subject itself passes.
    h.stub().subject = anchored;
    let (status, body) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
}

#[tokio::test]
async fn the_org_name_and_token_are_never_stored_or_returned() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let key = h.link(&account, org(1)).await;
    let (_, status) = h.status(&account).await;
    let dump = h.dump(&account.tenant).await;
    assert!(!dump.is_empty());
    for (what, text) in [("rows", &dump), ("status", &status.to_string())] {
        assert!(!text.contains(NAME_SENTINEL), "{what}: {text}");
        assert!(!text.contains("TOKENSENTINEL"), "{what}: {text}");
        assert!(!text.contains(&org(1).to_string()), "{what}: {text}");
        assert!(
            !text.contains(&org(1).simple().to_string()),
            "{what}: {text}"
        );
        assert!(!text.contains(&key.to_string()), "{what}: {text}");
        assert!(!text.contains(&h.stub().subject), "{what}: {text}");
    }
}

#[tokio::test]
async fn probe_404_and_409_and_403_map_to_their_outcomes() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    for (probe, status_want, label) in [
        (
            (404, "organization_unavailable"),
            StatusCode::FORBIDDEN,
            "near_ai_org_unavailable",
        ),
        (
            (409, "credential_transfer_limit_exceeded"),
            StatusCode::FORBIDDEN,
            "near_ai_org_not_permitted",
        ),
        (
            (409, "organization_transfer_limit_exceeded"),
            StatusCode::SERVICE_UNAVAILABLE,
            "payout_unavailable",
        ),
        (
            (403, "forbidden"),
            StatusCode::SERVICE_UNAVAILABLE,
            "payout_unavailable",
        ),
    ] {
        h.stub().probe_error = Some(probe);
        let (status, body) = h
            .start(&account, link_body(Uuid::new_v4(), org(1), 0))
            .await;
        assert_eq!(status, status_want, "{probe:?} {body}");
        assert_eq!(body["error"], label, "{probe:?} {body}");
    }
    assert_eq!(h.request_rows(&account).await, 0);
}

#[tokio::test]
async fn confirm_needs_the_same_account_session_and_no_passkey() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let other = h.account_in(&account.tenant).await;
    let (_, body) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    let handle = body["handle"].as_str().unwrap().to_owned();

    // Another account in the same tenant cannot confirm it.
    let (status, body) = h.confirm(&other, &handle).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "not_found");
    // No session at all.
    let (status, _) = h
        .call(
            "POST",
            &format!("{LINK_REQUESTS}/{handle}/confirm"),
            None,
            None,
        )
        .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    // A device bearer is not an account session.
    let (status, _) = h
        .call(
            "POST",
            &format!("{LINK_REQUESTS}/{handle}/confirm"),
            Some("token-a"),
            None,
        )
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    assert!(
        h.links(&account, &[PayoutLinkState::Waiting])
            .await
            .is_empty()
    );

    // The weak session that started it confirms it.
    let (status, body) = h.confirm(&account, &handle).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["outcome"], "confirmed");
    assert_eq!(body["version"], 1);
    assert_eq!(
        h.links(&account, &[PayoutLinkState::Waiting]).await.len(),
        1
    );
    // A handle confirms once.
    let (status, _) = h.confirm(&account, &handle).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn an_expired_or_second_request_is_discarded() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let (_, first) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    let (_, second) = h
        .start(&account, link_body(Uuid::new_v4(), org(2), 0))
        .await;
    // The first is superseded by the second.
    let (status, body) = h.confirm(&account, first["handle"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    // An expired one answers expired.
    h.backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_payout_link_requests
                SET created_at = clock_timestamp() - interval '10 minutes',
                    expires_at = clock_timestamp() - interval '1 minute'
              WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .unwrap();
    let (status, body) = h
        .confirm(&account, second["handle"].as_str().unwrap())
        .await;
    assert_eq!(status, StatusCode::GONE, "{body}");
    assert_eq!(body["error"], "expired");
    assert!(
        h.links(&account, &[PayoutLinkState::Waiting])
            .await
            .is_empty()
    );
    assert_eq!(h.request_rows(&account).await, 0);
}

#[tokio::test]
async fn a_link_that_changed_since_it_was_shown_is_refused() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    // Shown with no link (version 0), but another session links first.
    let (_, shown) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    h.link(&account, org(2)).await;
    let (status, body) = h.confirm(&account, shown["handle"].as_str().unwrap()).await;
    assert!(
        status == StatusCode::CONFLICT || status == StatusCode::NOT_FOUND,
        "{body}"
    );
    // A start against a stale version is refused before NEAR AI is asked.
    let calls = h.stub().users_me_calls;
    let (status, body) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "version_conflict");
    assert_eq!(h.stub().users_me_calls, calls);
    // A start shown at version 1, confirmed after another change, conflicts.
    let (_, stale) = h.start(&account, unlink_body(Uuid::new_v4(), 1)).await;
    h.backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_payout_links SET state = 'active' WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .unwrap();
    let (_, other) = h.start(&account, unlink_body(Uuid::new_v4(), 1)).await;
    let (status, body) = h.confirm(&account, other["handle"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let (status, body) = h.confirm(&account, stale["handle"].as_str().unwrap()).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "version_conflict");
    let (status, body) = h.start(&account, unlink_body(Uuid::new_v4(), 1)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "version_conflict");
}

#[tokio::test]
async fn a_device_bearer_is_refused() {
    let Some(h) = harness().await else { return };
    let (status, _) = h
        .call(
            "POST",
            LINK_REQUESTS,
            Some("token-a"),
            Some(link_body(Uuid::new_v4(), org(1), 0)),
        )
        .await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    let (status, _) = h.call("GET", STATUS, Some("token-a"), None).await;
    assert!(
        status == StatusCode::UNAUTHORIZED || status == StatusCode::FORBIDDEN,
        "{status}"
    );
    assert_eq!(h.stub().users_me_calls, 0);
}

#[tokio::test]
async fn a_disabled_rail_answers_payout_unavailable_and_an_unknown_rail_is_refused() {
    let Some(h) = harness_with(PayoutMode::Disabled).await else {
        return;
    };
    let account = h.account().await;
    let (status, body) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 0))
        .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "{body}");
    assert_eq!(body["error"], "payout_unavailable");
    let (status, body) = h
        .call(
            "POST",
            "/v1/account/payout-rails/test_keyed/link-requests",
            Some(&account.token),
            Some(link_body(Uuid::new_v4(), org(1), 0)),
        )
        .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    assert_eq!(body["error"], "payout_rail_unknown");
    let (status, body) = h.status(&account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["rails"], serde_json::json!([]), "{body}");
    assert_eq!(h.stub().users_me_calls, 0);
}

#[tokio::test]
async fn status_lists_each_enabled_rail_without_a_recipient() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let (status, body) = h.status(&account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["contract_version"], "payout-link-v1");
    assert_eq!(
        body["rails"],
        serde_json::json!([{
            "rail_id": "near_ai", "mode": "dry_run", "linked": false,
            "recipient_ref": null, "state": null, "version": 0,
            "request_state": null, "unlink_requested": false,
            "recipient_unavailable": false,
        }])
    );
    let key = h.link(&account, org(1)).await;
    let (_, body) = h.status(&account).await;
    let rail = body["rails"][0].as_object().unwrap();
    let mut fields: Vec<_> = rail.keys().cloned().collect();
    fields.sort();
    assert_eq!(
        fields,
        [
            "linked",
            "mode",
            "rail_id",
            "recipient_ref",
            "recipient_unavailable",
            "request_state",
            "state",
            "unlink_requested",
            "version"
        ]
    );
    assert_eq!(rail["recipient_ref"], near_ai_recipient_ref(key, org(1)));
}

#[tokio::test]
async fn change_and_unlink_on_one_rail_leave_another_rails_link_alone() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    let keyed = RailId::new("test_keyed").unwrap();
    let digest = |value: &str| sha256_prefixed(value);
    let other_link = Uuid::new_v4();
    assert_eq!(
        h.backend
            .insert_payout_link(
                &account.tenant,
                &PayoutLinkWrite {
                    account_id: account.id,
                    rail_id: keyed.clone(),
                    link_id: other_link,
                    recipient_kind: RecipientKind::new("test_address").unwrap(),
                    recipient: SealedRecipient {
                        sealed: serde_json::json!({"v": 1}),
                        recipient_index: digest("index"),
                        recipient_key_ref: "key-test-1".into(),
                        index_pepper_ref: "pepper-test-1".into(),
                        subject_index: digest("subject"),
                        recipient_ref: None,
                    },
                    program: "keyed-program".into(),
                    state: PayoutLinkState::Active,
                    actor_ref_hash: digest("actor"),
                },
            )
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    let keyed_before = h
        .backend
        .payout_links(&account.tenant, &keyed, &[PayoutLinkState::Active])
        .await
        .unwrap();

    // Link, activate (as the pass would), change, then unlink, on near_ai.
    h.link(&account, org(1)).await;
    h.backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_payout_links SET state = 'active'
              WHERE tenant_id = $1 AND rail_id = 'near_ai'",
            &[&account.tenant],
        )
        .await
        .unwrap();
    h.link(&account, org(2)).await;
    assert_eq!(h.version(&account).await, 2);
    // A second change while one waits is refused.
    let (_, pending) = h
        .start(&account, link_body(Uuid::new_v4(), org(1), 2))
        .await;
    let (status, body) = h
        .confirm(&account, pending["handle"].as_str().unwrap())
        .await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "payout_link_change_pending");
    let (_, unlink) = h.start(&account, unlink_body(Uuid::new_v4(), 2)).await;
    let (status, body) = h
        .confirm(&account, unlink["handle"].as_str().unwrap())
        .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["action"], "unlink");
    assert_eq!(body["version"], 3);
    let active = h.links(&account, &[PayoutLinkState::Active]).await;
    assert_eq!(active.len(), 1);
    assert!(active[0].unlink_requested_at.is_some());

    // The keyed rail's link is exactly as it was.
    assert_eq!(
        h.backend
            .payout_links(&account.tenant, &keyed, &[PayoutLinkState::Active])
            .await
            .unwrap(),
        keyed_before
    );
    // An unlink with nothing linked on a fresh account.
    let fresh = h.account().await;
    let (status, body) = h.start(&fresh, unlink_body(Uuid::new_v4(), 0)).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["error"], "payout_not_linked");
}

#[tokio::test]
async fn every_request_and_confirmation_writes_an_audit_row() {
    let Some(h) = harness().await else { return };
    let account = h.account().await;
    h.link(&account, org(1)).await;
    h.backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap()
        .execute(
            "UPDATE trace_payout_links SET state = 'active' WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .unwrap();
    let (_, unlink) = h.start(&account, unlink_body(Uuid::new_v4(), 1)).await;
    let (status, _) = h
        .confirm(&account, unlink["handle"].as_str().unwrap())
        .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        h.audit(&account).await,
        vec![
            PayoutAuditAction::LinkRequested,
            PayoutAuditAction::LinkCreated,
            PayoutAuditAction::LinkConfirmed,
            PayoutAuditAction::UnlinkRequested,
            PayoutAuditAction::LinkConfirmed,
        ]
    );
    // The audit rows name the account and link by hash, never in clear.
    let dump = h.dump(&account.tenant).await;
    let audit: Vec<_> = dump
        .lines()
        .filter(|line| line.contains("actor_ref_hash"))
        .collect();
    assert_eq!(audit.len(), 5, "{dump}");
    for line in audit {
        assert!(!line.contains(&account.id.to_string()), "{line}");
    }
}

/// Own and referral credit apart (referral rewards spec, R7): once the rail
/// runs a referral stream, status carries one entry per stream with owed,
/// sent, waiting and the hold that stops that stream alone.
#[tokio::test]
async fn status_splits_own_and_referral_credit_once_the_referral_stream_runs() {
    use trace_commons_server::db::{PayoutLineResolution, PayoutLineWrite, PayoutStream};
    use trace_commons_server::near_ai_rail::{
        near_ai_program_for_link, near_ai_referral_program_for_link,
    };
    use trace_commons_server::near_ai_rail_config::{NearAiReferralConfig, NearAiReferralSettings};

    let Some(mut h) = harness().await else { return };
    // The harness's rail, with a referral stream in `dry_run` and no
    // credential.
    let rails = PayoutRails::from_configs_at_loopback(
        &NearAiRailConfig::Enabled(NearAiRailSettings {
            mode: PayoutMode::DryRun,
            credentials: NearAiTransferCredentials {
                token: SecretString::from(CREDENTIAL),
                credential_id: Uuid::from_u128(2),
                sender_org: Uuid::from_u128(3),
                source: NearAiCreditSource::Payment,
            },
            account_ceiling: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(
                1_000_000_000_000,
            ),
        }),
        &NearAiReferralConfig::Enabled(NearAiReferralSettings {
            mode: PayoutMode::DryRun,
            credentials: None,
            shares_payout_credential: false,
            account_ceiling: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(1_000_000),
        }),
        "http://127.0.0.1:9",
    )
    .unwrap();
    Arc::get_mut(&mut h.state).unwrap().payout_rails = Arc::new(rails);

    let account = h.account().await;
    let digest = |value: &str| sha256_prefixed(value);
    let link_id = Uuid::new_v4();
    assert_eq!(
        h.backend
            .insert_payout_link(
                &account.tenant,
                &PayoutLinkWrite {
                    account_id: account.id,
                    rail_id: RailId::near_ai(),
                    link_id,
                    recipient_kind: RecipientKind::near_ai_org(),
                    recipient: SealedRecipient {
                        sealed: serde_json::json!({"v": 1}),
                        recipient_index: digest("index"),
                        recipient_key_ref: "key-test-1".into(),
                        index_pepper_ref: "pepper-test-1".into(),
                        subject_index: digest("subject"),
                        recipient_ref: None,
                    },
                    program: near_ai_program_for_link(link_id),
                    state: PayoutLinkState::Active,
                    actor_ref_hash: digest("actor"),
                },
            )
            .await
            .unwrap(),
        PayoutLinkInsert::Inserted
    );
    // Bonus of 500 owed, 200 of it sent on the referral program.
    {
        let mut client = h.backend.trace_pool_for_test().get().await.unwrap();
        let tx = client.transaction().await.unwrap();
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&account.tenant],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_payout_rate_epochs (rail_id, version, numerator, denominator)
             VALUES ('near_ai', 'v1', 1, 1) ON CONFLICT DO NOTHING",
            &[],
        )
        .await
        .unwrap();
        let referee = digest(&format!("referee:{}", Uuid::new_v4()));
        tx.execute(
            "INSERT INTO trace_referral_referees
                 (tenant_id, referee_ref_hash, account_id, code_index,
                  referee_account_created_at, applied_at)
             VALUES ($1, $2, $3, $4, clock_timestamp(), clock_timestamp())",
            &[&account.tenant, &referee, &account.id, &digest("code")],
        )
        .await
        .unwrap();
        tx.execute(
            "INSERT INTO trace_referral_bonus_accruals
                 (tenant_id, bonus_id, rail_id, account_id, referee_ref_hash, kind,
                  source_accrual_id, bps, base_units, bonus_units, rate_version, instrument_id)
             VALUES ($1, $2, 'near_ai', $3, $4, 'bonus', $5, 1000, 5000, 500, 'v1',
                     'near_ai_usd')",
            &[
                &account.tenant,
                &Uuid::new_v4(),
                &account.id,
                &referee,
                &Uuid::new_v4(),
            ],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
    }
    let line = PayoutLineWrite {
        line_id: Uuid::new_v4(),
        link_id,
        account_id: account.id,
        rail_id: RailId::near_ai(),
        target_total: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(200),
        delta: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(200),
        owed_through_accrual_count: 1,
        stream: PayoutStream::Referral,
        referral_program: Some(near_ai_referral_program_for_link(link_id)),
    };
    h.backend
        .insert_pending_payout_line(&account.tenant, &line)
        .await
        .unwrap();
    h.backend
        .resolve_payout_line(
            &account.tenant,
            line.line_id,
            &PayoutLineResolution::Confirmed {
                receipt_hash: digest(&format!("receipt:{}", line.line_id)),
                delta: trace_commons_gate_api::pipeline::AtomicUnits::from_raw(200),
            },
        )
        .await
        .unwrap();

    let (status, body) = h.status(&account).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        body["rails"][0]["streams"],
        serde_json::json!([
            {"stream": "earned", "owed": "0", "sent": "0", "waiting": "0"},
            {"stream": "referral", "owed": "500", "sent": "200", "waiting": "300"},
        ])
    );

    // A referral-stream hold shows on the referral stream alone.
    h.backend
        .open_payout_hold(
            &account.tenant,
            account.id,
            &RailId::near_ai(),
            "referral_code_suspended",
            &digest("actor"),
        )
        .await
        .unwrap();
    let (_, body) = h.status(&account).await;
    let streams = &body["rails"][0]["streams"];
    assert!(streams[0].get("hold_label").is_none(), "{body}");
    assert_eq!(
        streams[1]["hold_label"], "referral_code_suspended",
        "{body}"
    );
    // Amounts and labels only: no account, program or recipient.
    let rendered = body.to_string();
    assert!(!rendered.contains(&account.id.to_string()), "{rendered}");
    assert!(
        !rendered.contains(&near_ai_referral_program_for_link(link_id)),
        "{rendered}"
    );
}

#[tokio::test]
async fn an_enabled_rail_without_the_recipient_key_refuses_the_start() {
    let root = tempfile::tempdir().unwrap();
    let mut state = test_state(root.path().to_path_buf());
    // Disabled, the default: no key needed.
    super::super::payout_rails::require_payout_recipient_key(&state).unwrap();
    Arc::get_mut(&mut state).unwrap().payout_rails =
        Arc::new(near_ai_rails("http://127.0.0.1:9", PayoutMode::DryRun));
    let error = super::super::payout_rails::require_payout_recipient_key(&state)
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("payout_recipient_key_unconfigured"),
        "{error}"
    );
    Arc::get_mut(&mut state).unwrap().near_account_identity = Some(identity());
    super::super::payout_rails::require_payout_recipient_key(&state).unwrap();
}
