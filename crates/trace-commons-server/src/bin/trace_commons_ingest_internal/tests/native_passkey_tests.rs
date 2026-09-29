// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Native passkey creation and sign-in (Z2 native passkey identity, slice S2).
//!
//! The in-memory tests run everywhere; the PostgreSQL tests self-skip without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL` and need the login-resolver URL too
//! (native sign-in resolves the credential's tenant through the resolver
//! role, exactly as the browser sign-in does). Every ceremony is a real
//! WebAuthn ceremony driven by the dev-only SoftPasskey authenticator.

use super::*;
use crate::account_routes::ACCOUNT_UNBOUND;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_server::account_native_passkey::UnboundAccountCeiling;

type SoftAuthenticator = webauthn_authenticator_rs::WebauthnAuthenticator<
    webauthn_authenticator_rs::softpasskey::SoftPasskey,
>;

const CREATE_START: &str = "/v1/account/native/passkey/create/start";
const CREATE_FINISH: &str = "/v1/account/native/passkey/create/finish";
const LOGIN_START: &str = "/v1/account/native/passkey/login/start";
const LOGIN_FINISH: &str = "/v1/account/native/passkey/login/finish";
const REGISTER_START: &str = "/v1/account/passkeys/native/register/start";
const REGISTER_FINISH: &str = "/v1/account/passkeys/native/register/finish";

/// A test state together with the temp directory it writes under. The
/// directory lives exactly as long as the state and is removed when this is
/// dropped. Derefs to the `Arc<AppState>` every helper takes.
struct NativeState {
    state: Arc<AppState>,
    _root: tempfile::TempDir,
}

impl std::ops::Deref for NativeState {
    type Target = Arc<AppState>;

    fn deref(&self) -> &Arc<AppState> {
        &self.state
    }
}

impl NativeState {
    /// Mutate the state before any clone of it escapes.
    fn configure(&mut self, change: impl FnOnce(&mut AppState)) {
        change(Arc::get_mut(&mut self.state).expect("fresh state is uniquely owned"));
    }
}

/// A state with the test relying party and the given unbound ceiling.
fn native_state(db: Option<Arc<dyn Database>>, ceiling: Option<i64>) -> NativeState {
    let root = tempfile::tempdir().expect("temp dir");
    let mut state = NativeState {
        state: test_state_with_webauthn(root.path().to_path_buf(), db),
        _root: root,
    };
    state.configure(|state| {
        state.account_unbound_ceiling = Arc::new(match ceiling {
            Some(limit) => UnboundAccountCeiling::with_limit(limit),
            None => UnboundAccountCeiling::disabled(),
        })
    });
    state
}

/// A second state over the same database and the SAME ceremony store, with a
/// different ceiling: a ceremony started on `from` can be finished on it.
fn sibling_state(from: &Arc<AppState>, ceiling: Option<i64>) -> NativeState {
    let mut state = native_state(from.db_mirror.clone(), ceiling);
    state.configure(|state| state.account_ceremony_store = from.account_ceremony_store.clone());
    state
}

struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes).unwrap_or_default()
    }
}

async fn send(state: &Arc<AppState>, request: axum::http::Request<Body>) -> Reply {
    let response = app(state.clone())
        .oneshot(request)
        .await
        .expect("router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    Reply {
        status,
        headers,
        bytes,
    }
}

fn post(uri: &str, body: &serde_json::Value, bearer: Option<&str>) -> axum::http::Request<Body> {
    let builder = axum::http::Request::builder()
        .method("POST")
        .uri(uri)
        .header(CONTENT_TYPE, "application/json");
    let builder = match bearer {
        Some(token) => builder.header(AUTHORIZATION, format!("Bearer {token}")),
        None => builder,
    };
    builder
        .body(Body::from(serde_json::to_vec(body).expect("json")))
        .expect("request")
}

fn get(uri: &str, bearer: &str) -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .header(AUTHORIZATION, format!("Bearer {bearer}"))
        .body(Body::empty())
        .expect("request")
}

/// The body every native passkey refusal answers with, byte for byte.
async fn uniform_deny_bytes() -> (StatusCode, Vec<u8>) {
    let response = native_generic_deny();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("body")
        .to_vec();
    (status, bytes)
}

async fn assert_uniform_deny(reply: &Reply, what: &str) {
    let (status, bytes) = uniform_deny_bytes().await;
    assert_eq!(reply.status, status, "{what}: status");
    assert_eq!(reply.bytes, bytes, "{what}: body");
    assert_eq!(
        reply
            .headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store"),
        "{what}: no-store"
    );
}

/// A native account created end to end: the returned token, and what the
/// test needs to sign in with the same passkey again.
struct Created {
    token: String,
    tenant: String,
    account_id: Uuid,
    credential_id: String,
    authenticator: SoftAuthenticator,
}

async fn create_start(state: &Arc<AppState>, label: Option<&str>) -> Reply {
    let body = match label {
        Some(label) => serde_json::json!({ "label": label }),
        None => serde_json::json!({}),
    };
    send(state, post(CREATE_START, &body, None)).await
}

async fn create_finish(
    state: &Arc<AppState>,
    ceremony_id: &str,
    credential: &webauthn_rs::prelude::RegisterPublicKeyCredential,
) -> Reply {
    send(
        state,
        post(
            CREATE_FINISH,
            &serde_json::json!({ "ceremony_id": ceremony_id, "credential": credential }),
            None,
        ),
    )
    .await
}

/// `create/start` then a software attestation, stopping before `finish`.
async fn started_creation(
    state: &Arc<AppState>,
) -> (
    String,
    webauthn_rs::prelude::RegisterPublicKeyCredential,
    SoftAuthenticator,
) {
    let start = create_start(state, Some("My trace passkey")).await;
    assert_eq!(start.status, StatusCode::OK, "create/start");
    let start = start.json();
    let ceremony_id = start["ceremony_id"]
        .as_str()
        .expect("ceremony id")
        .to_string();
    let mut authenticator = new_software_authenticator();
    let credential = softpasskey_register(&mut authenticator, &start["public_key"]);
    (ceremony_id, credential, authenticator)
}

async fn create_native_account(state: &Arc<AppState>) -> Created {
    let (ceremony_id, credential, authenticator) = started_creation(state).await;
    let finish = create_finish(state, &ceremony_id, &credential).await;
    assert_eq!(finish.status, StatusCode::OK, "create/finish");
    let body = finish.json();
    assert_eq!(body["binding_state"], "unbound");
    assert_eq!(body["token_type"], "Bearer");
    let token = body["access_token"].as_str().expect("token").to_string();
    let (tenant, _) = native_token_parts(&token).expect("a tcn1_ token");
    Created {
        token,
        tenant,
        account_id: body["account_id"]
            .as_str()
            .and_then(|id| Uuid::parse_str(id).ok())
            .expect("account id"),
        credential_id: credential_id_to_string(&webauthn_rs::prelude::CredentialID::from(
            credential.raw_id.as_ref(),
        )),
        authenticator,
    }
}

async fn login_start(state: &Arc<AppState>) -> (String, serde_json::Value) {
    let start = send(state, post(LOGIN_START, &serde_json::json!({}), None)).await;
    assert_eq!(start.status, StatusCode::OK, "login/start");
    let body = start.json();
    (
        body["ceremony_id"]
            .as_str()
            .expect("ceremony id")
            .to_string(),
        body["public_key"].clone(),
    )
}

async fn login_finish(
    state: &Arc<AppState>,
    ceremony_id: &str,
    assertion: &webauthn_rs::prelude::PublicKeyCredential,
) -> Reply {
    send(
        state,
        post(
            LOGIN_FINISH,
            &serde_json::json!({ "ceremony_id": ceremony_id, "credential": assertion }),
            None,
        ),
    )
    .await
}

/// Native sign-in with `created`'s passkey; returns the finish reply.
async fn native_login(state: &Arc<AppState>, created: &mut Created) -> Reply {
    let (ceremony_id, challenge) = login_start(state).await;
    let assertion = softpasskey_authenticate_discoverable(
        &mut created.authenticator,
        &challenge,
        &created.credential_id,
        Some(created.account_id),
    );
    login_finish(state, &ceremony_id, &assertion).await
}

async fn pg_admin(backend: &PgBackend) -> deadpool_postgres::Object {
    backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin")
}

async fn pg_count(admin: &deadpool_postgres::Object, sql: &str, param: &str) -> i64 {
    admin.query_one(sql, &[&param]).await.expect("count").get(0)
}

/// Every row a created account can own, keyed by its tenant.
async fn rows_in_tenant(admin: &deadpool_postgres::Object, tenant: &str) -> [i64; 6] {
    [
        pg_count(
            admin,
            "SELECT count(*) FROM trace_tenants WHERE tenant_id = $1",
            tenant,
        )
        .await,
        pg_count(
            admin,
            "SELECT count(*) FROM trace_accounts WHERE tenant_id = $1",
            tenant,
        )
        .await,
        pg_count(
            admin,
            "SELECT count(*) FROM trace_account_bindings WHERE tenant_id = $1",
            tenant,
        )
        .await,
        pg_count(
            admin,
            "SELECT count(*) FROM trace_webauthn_credentials WHERE tenant_id = $1",
            tenant,
        )
        .await,
        pg_count(
            admin,
            "SELECT count(*) FROM trace_sessions WHERE tenant_id = $1",
            tenant,
        )
        .await,
        pg_count(
            admin,
            "SELECT count(*) FROM trace_account_audit WHERE tenant_id = $1",
            tenant,
        )
        .await,
    ]
}

async fn total_tenants(admin: &deadpool_postgres::Object) -> i64 {
    admin
        .query_one("SELECT count(*) FROM trace_tenants", &[])
        .await
        .expect("count")
        .get(0)
}

async fn drop_tenant(admin: &deadpool_postgres::Object, tenant: &str) {
    admin
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("cleanup");
}

async fn bind_in_place(admin: &deadpool_postgres::Object, created: &Created) {
    // Stands in for S3's bind: the one transition this slice does not write.
    admin
        .execute(
            "UPDATE trace_account_bindings SET state = 'bound', bound_at = now()
              WHERE tenant_id = $1 AND account_id = $2",
            &[&created.tenant, &created.account_id],
        )
        .await
        .expect("bind");
}

async fn pg_state(ceiling_headroom: Option<i64>) -> Option<(Arc<PgBackend>, NativeState)> {
    let backend = postgres_backend_for_ingest_test().await?;
    reset_account_rate_limiter_for_test();
    let current = backend
        .count_unbound_passkey_accounts()
        .await
        .expect("V98 count");
    let db: Arc<dyn Database> = backend.clone();
    let state = native_state(Some(db), ceiling_headroom.map(|room| current + room));
    Some((backend, state))
}

// --- In-memory -------------------------------------------------------------

/// Unset means creation is disabled: start refuses with the uniform deny and
/// never reaches the database.
#[tokio::test]
async fn an_unset_ceiling_disables_creation() {
    reset_account_rate_limiter_for_test();
    let state = native_state(None, None);
    let reply = create_start(&state, None).await;
    assert_uniform_deny(&reply, "create/start, ceiling unset").await;
}

/// Every refusal of the unauthenticated native routes is the same response:
/// malformed and unknown fields, an unknown ceremony, a label over the bound,
/// an unconfigured relying party.
#[tokio::test]
async fn native_refusals_are_byte_identical() {
    reset_account_rate_limiter_for_test();
    let state = native_state(None, Some(10));
    let cases: Vec<(&str, &str, serde_json::Value)> = vec![
        (
            "create/start, long label",
            CREATE_START,
            serde_json::json!({ "label": "x".repeat(65) }),
        ),
        (
            "create/start, unknown field",
            CREATE_START,
            serde_json::json!({ "tenant_id": "t" }),
        ),
        ("create/finish, empty", CREATE_FINISH, serde_json::json!({})),
        (
            "create/finish, unknown ceremony",
            CREATE_FINISH,
            serde_json::json!({ "ceremony_id": "never-issued", "credential": dummy_register_finish_body().credential }),
        ),
        ("login/finish, empty", LOGIN_FINISH, serde_json::json!({})),
        (
            "login/finish, not json",
            LOGIN_FINISH,
            serde_json::Value::String("nope".to_string()),
        ),
    ];
    for (what, uri, body) in cases {
        let reply = send(&state, post(uri, &body, None)).await;
        assert_uniform_deny(&reply, what).await;
    }

    // No relying party: both starts refuse the same way.
    let mut no_rp = native_state(None, Some(10));
    no_rp.configure(|state| state.account_webauthn = None);
    for uri in [CREATE_START, LOGIN_START] {
        let reply = send(&no_rp, post(uri, &serde_json::json!({}), None)).await;
        assert_uniform_deny(&reply, uri).await;
    }
}

/// `finish` holds the timing floor on a refusal, as `native/token` does.
#[tokio::test]
async fn finish_refusals_hold_the_timing_floor() {
    reset_account_rate_limiter_for_test();
    let state = native_state(None, Some(10));
    for uri in [CREATE_FINISH, LOGIN_FINISH] {
        let started = std::time::Instant::now();
        let reply = send(&state, post(uri, &serde_json::json!({}), None)).await;
        assert_uniform_deny(&reply, uri).await;
        assert!(started.elapsed() >= REDEEM_MIN_LATENCY, "{uri}");
    }
}

/// The start routes are rate limited per IP, with the uniform deny.
#[tokio::test]
async fn native_starts_are_rate_limited_per_ip() {
    reset_account_rate_limiter_for_test();
    let state = native_state(None, None);
    let mut last = None;
    for _ in 0..NATIVE_PASSKEY_PER_IP_LIMIT + 1 {
        last = Some(send(&state, post(LOGIN_START, &serde_json::json!({}), None)).await);
    }
    assert_uniform_deny(&last.expect("sent"), "login/start over the per-IP limit").await;
    reset_account_rate_limiter_for_test();
}

// --- PostgreSQL ------------------------------------------------------------

/// create/start writes nothing; finish writes exactly one unbound account with
/// its credential, weak native session and hash-only audit row, and the
/// token it returns resolves.
#[tokio::test]
async fn pg_native_create_writes_one_unbound_account_at_finish_only() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;

    let before = total_tenants(&admin).await;
    let (ceremony_id, credential, _authenticator) = started_creation(&state).await;
    assert_eq!(total_tenants(&admin).await, before, "start writes no row");

    let finish = create_finish(&state, &ceremony_id, &credential).await;
    assert_eq!(finish.status, StatusCode::OK);
    assert_eq!(
        finish
            .headers
            .get(axum::http::header::CACHE_CONTROL)
            .and_then(|v| v.to_str().ok()),
        Some("no-store")
    );
    let body = finish.json();
    let token = body["access_token"].as_str().expect("token");
    assert!(is_native_token(token));
    assert_eq!(body["expires_in_secs"], NATIVE_SESSION_TTL_HOURS * 3600);
    let (tenant, token_hash) = native_token_parts(token).expect("token parts");
    assert!(
        tenant.starts_with("nearai-"),
        "server-minted nearai- tenant"
    );
    assert_eq!(rows_in_tenant(&admin, &tenant).await, [1, 1, 1, 1, 1, 1]);

    let account_id = Uuid::parse_str(body["account_id"].as_str().unwrap()).unwrap();
    let row = admin
        .query_one(
            "SELECT b.state, b.origin, c.label, s.client_kind, s.auth_credential_id,
                    c.credential_id, s.token_hash
               FROM trace_account_bindings b
               JOIN trace_webauthn_credentials c USING (tenant_id, account_id)
               JOIN trace_sessions s USING (tenant_id, account_id)
              WHERE b.tenant_id = $1 AND b.account_id = $2",
            &[&tenant, &account_id],
        )
        .await
        .expect("rows");
    assert_eq!(row.get::<_, String>(0), "unbound");
    assert_eq!(row.get::<_, String>(1), "passkey");
    assert_eq!(
        row.get::<_, Option<String>>(2).as_deref(),
        Some("My trace passkey")
    );
    assert_eq!(row.get::<_, String>(3), NATIVE_SESSION_CLIENT_KIND);
    assert_eq!(
        row.get::<_, Option<String>>(4),
        Some(row.get::<_, String>(5))
    );
    assert_eq!(row.get::<_, String>(6), token_hash);
    let audit = admin
        .query_one(
            "SELECT action, actor_ref, safe_metadata FROM trace_account_audit WHERE tenant_id = $1",
            &[&tenant],
        )
        .await
        .expect("audit");
    assert_eq!(audit.get::<_, String>(0), "account_passkey_created");
    assert_eq!(
        audit.get::<_, String>(1),
        format!("account-actor:{account_id}")
    );
    assert_eq!(
        audit.get::<_, serde_json::Value>(2),
        serde_json::json!({ "binding": "unbound", "labeled": true })
    );

    // The token resolves, as a native session, to an unbound account.
    let binding = send(&state, get("/v1/account/binding", token)).await;
    assert_eq!(binding.status, StatusCode::OK);
    assert_eq!(
        binding.json(),
        serde_json::json!({ "binding_state": "unbound" })
    );

    drop_tenant(&admin, &tenant).await;
}

/// The new account is unbound, and S1's gate refuses it every route off the
/// allowlist, including the native add-passkey route.
#[tokio::test]
async fn pg_a_created_account_is_unbound_and_gated() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let created = create_native_account(&state).await;

    for request in [
        get("/v1/account/traces", &created.token),
        get("/v1/account/credit-summary", &created.token),
        post(REGISTER_START, &serde_json::json!({}), Some(&created.token)),
        post(
            "/v1/account/invites/redeem",
            &serde_json::json!({ "invite_code": "AAAAAAAAAAAAAAAA", "idempotency_key": Uuid::new_v4() }),
            Some(&created.token),
        ),
    ] {
        let uri = request.uri().to_string();
        let reply = send(&state, request).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(reply.json()["error"], ACCOUNT_UNBOUND, "{uri}");
    }
    let passkeys = send(&state, get("/v1/account/passkeys", &created.token)).await;
    assert_eq!(passkeys.status, StatusCode::OK);
    let listed = passkeys.json();
    assert_eq!(listed["passkeys"].as_array().map(Vec::len), Some(1));
    assert_eq!(
        listed["passkeys"][0]["this_device"], true,
        "auth_credential_id is carried"
    );

    drop_tenant(&admin, &created.tenant).await;
}

/// An abandoned ceremony writes nothing, and neither does a finish whose
/// attestation fails or a replay of a finish that succeeded.
#[tokio::test]
async fn pg_abandoned_failed_and_replayed_creations_write_nothing() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let before = total_tenants(&admin).await;

    // Abandoned after start.
    let _abandoned = started_creation(&state).await;
    // A finish whose attestation answers a different challenge.
    let (ceremony_a, _credential_a, _) = started_creation(&state).await;
    let (_ceremony_b, credential_b, _) = started_creation(&state).await;
    let wrong = create_finish(&state, &ceremony_a, &credential_b).await;
    assert_uniform_deny(&wrong, "finish with another ceremony's attestation").await;
    assert_eq!(total_tenants(&admin).await, before);

    // A successful finish, then the same body again: single use.
    let (ceremony, credential, _) = started_creation(&state).await;
    let first = create_finish(&state, &ceremony, &credential).await;
    assert_eq!(first.status, StatusCode::OK);
    let (tenant, _) = native_token_parts(first.json()["access_token"].as_str().unwrap()).unwrap();
    let replay = create_finish(&state, &ceremony, &credential).await;
    assert_uniform_deny(&replay, "replayed create/finish").await;
    assert_eq!(
        total_tenants(&admin).await,
        before + 1,
        "the replay wrote nothing"
    );

    // The same attestation under a fresh ceremony fails on the credential id.
    let (fresh, _, _) = started_creation(&state).await;
    let reused = create_finish(&state, &fresh, &credential).await;
    assert_uniform_deny(&reused, "an attestation replayed into a new ceremony").await;
    assert_eq!(total_tenants(&admin).await, before + 1);

    drop_tenant(&admin, &tenant).await;
}

/// A creation transaction that fails part-way writes no row at all: a
/// credential id that already exists fails after the tenant, account and
/// binding rows, and a session hash collision fails after the credential.
#[tokio::test]
async fn pg_a_creation_that_aborts_mid_transaction_leaves_no_rows() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let existing = create_native_account(&state).await;
    let existing_hash = native_token_parts(&existing.token).unwrap().1;
    let passkey = serde_json::json!({ "not": "verified; the insert fails first" });

    for (what, credential_id, token_hash) in [
        (
            "duplicate credential id",
            existing.credential_id.clone(),
            hash_secret(&generate_session_secret()),
        ),
        (
            "duplicate session hash",
            format!("fresh-{}", Uuid::new_v4().simple()),
            existing_hash.clone(),
        ),
    ] {
        let tenant = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
        let result = backend
            .create_passkey_origin_account(trace_commons_server::db::NewPasskeyOriginAccount {
                tenant_id: &tenant,
                account_id: Uuid::new_v4(),
                credential_id: &credential_id,
                passkey: &passkey,
                label: None,
                session: trace_commons_server::db::NewSession {
                    token_hash: &token_hash,
                    client_kind: NATIVE_SESSION_CLIENT_KIND,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                ceiling: i64::MAX,
            })
            .await;
        assert!(result.is_err(), "{what}");
        assert_eq!(rows_in_tenant(&admin, &tenant).await, [0; 6], "{what}");
    }

    // A strong session kind is refused before anything is written.
    let tenant = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
    let result = backend
        .create_passkey_origin_account(trace_commons_server::db::NewPasskeyOriginAccount {
            tenant_id: &tenant,
            account_id: Uuid::new_v4(),
            credential_id: &format!("fresh-{}", Uuid::new_v4().simple()),
            passkey: &passkey,
            label: None,
            session: trace_commons_server::db::NewSession {
                token_hash: &hash_secret(&generate_session_secret()),
                client_kind: "passkey",
                expires_at: Utc::now() + Duration::hours(1),
            },
            ceiling: i64::MAX,
        })
        .await;
    assert!(
        result.is_err(),
        "a passkey-origin account's session is native"
    );
    assert_eq!(rows_in_tenant(&admin, &tenant).await, [0; 6]);

    drop_tenant(&admin, &existing.tenant).await;
}

/// The ceiling at N-1 admits, at N refuses both halves, and a finish whose
/// ceremony started below the ceiling is refused when the count reached it
/// meanwhile. Unset refuses a finish too.
#[tokio::test]
async fn pg_the_ceiling_is_checked_at_start_and_again_at_finish() {
    // Headroom 1: the count is N-1, so exactly one more account fits.
    let Some((backend, state)) = pg_state(Some(1)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let n = state.account_unbound_ceiling.limit().expect("a ceiling");
    assert_eq!(
        backend.count_unbound_passkey_accounts().await.unwrap(),
        n - 1
    );

    // Two ceremonies start below the ceiling.
    let (first, first_credential, _) = started_creation(&state).await;
    let (second, second_credential, _) = started_creation(&state).await;
    // The first finish takes the last slot.
    let created = create_finish(&state, &first, &first_credential).await;
    assert_eq!(created.status, StatusCode::OK, "at N-1 creation succeeds");
    let (tenant, _) = native_token_parts(created.json()["access_token"].as_str().unwrap()).unwrap();
    assert_eq!(backend.count_unbound_passkey_accounts().await.unwrap(), n);

    // The second finish is re-checked inside its transaction and refused.
    let before = total_tenants(&admin).await;
    let refused = create_finish(&state, &second, &second_credential).await;
    assert_uniform_deny(&refused, "finish once the ceiling was reached").await;
    assert_eq!(
        total_tenants(&admin).await,
        before,
        "a refused finish writes nothing"
    );

    // At N, start refuses.
    let refused = create_start(&state, None).await;
    assert_uniform_deny(&refused, "start at the ceiling").await;

    // Unset: a ceremony started under a ceiling cannot be finished without one.
    drop_tenant(&admin, &tenant).await;
    let (third, third_credential, _) = started_creation(&state).await;
    let unset = sibling_state(&state, None);
    let refused = create_finish(&unset, &third, &third_credential).await;
    assert_uniform_deny(&refused, "finish with no ceiling configured").await;
    assert_eq!(total_tenants(&admin).await, before - 1);
}

/// Ceremonies are tagged by surface and single use: a browser ceremony cannot
/// be finished natively, a native one cannot be finished by the browser, a
/// creation ceremony cannot finish a sign-in (and the reverse), and a
/// finished sign-in cannot be replayed.
#[tokio::test]
async fn pg_ceremonies_cannot_cross_surfaces_or_be_reused() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let mut created = create_native_account(&state).await;

    // A browser sign-in ceremony (cookie-bound) presented to native finish.
    let (browser_challenge, browser_pair) = passkey_login_start(&state).await;
    let browser_id = browser_pair
        .split_once('=')
        .expect("cookie pair")
        .1
        .to_string();
    let assertion = softpasskey_authenticate_discoverable(
        &mut created.authenticator,
        &browser_challenge,
        &created.credential_id,
        Some(created.account_id),
    );
    let crossed = login_finish(&state, &browser_id, &assertion).await;
    assert_uniform_deny(&crossed, "browser ceremony on the native finish").await;

    // A native sign-in ceremony presented to the browser finish (as a cookie).
    let (native_id, native_challenge) = login_start(&state).await;
    let assertion = softpasskey_authenticate_discoverable(
        &mut created.authenticator,
        &native_challenge,
        &created.credential_id,
        Some(created.account_id),
    );
    let mut headers = HeaderMap::new();
    headers.insert(
        axum::http::header::COOKIE,
        HeaderValue::from_str(&format!("tc_passkey_ceremony={native_id}")).unwrap(),
    );
    let browser = account_passkey_login_finish_handler(
        State(state.clone()),
        headers,
        PasskeyAssertionBody(assertion),
    )
    .await;
    assert_eq!(
        browser.status(),
        StatusCode::BAD_REQUEST,
        "native ceremony on the browser finish"
    );
    assert!(
        browser
            .headers()
            .get(axum::http::header::SET_COOKIE)
            .is_none()
    );

    // A creation ceremony cannot finish a sign-in, nor a sign-in a creation.
    let (create_id, create_credential, _) = started_creation(&state).await;
    let (login_id, login_challenge) = login_start(&state).await;
    let assertion = softpasskey_authenticate_discoverable(
        &mut created.authenticator,
        &login_challenge,
        &created.credential_id,
        Some(created.account_id),
    );
    let crossed = login_finish(&state, &create_id, &assertion).await;
    assert_uniform_deny(&crossed, "creation ceremony on login/finish").await;
    let crossed = create_finish(&state, &login_id, &create_credential).await;
    assert_uniform_deny(&crossed, "sign-in ceremony on create/finish").await;

    // A sign-in that succeeded cannot be replayed.
    let (ceremony_id, challenge) = login_start(&state).await;
    let assertion = softpasskey_authenticate_discoverable(
        &mut created.authenticator,
        &challenge,
        &created.credential_id,
        Some(created.account_id),
    );
    let first = login_finish(&state, &ceremony_id, &assertion).await;
    assert_eq!(first.status, StatusCode::OK);
    let replay = login_finish(&state, &ceremony_id, &assertion).await;
    assert_uniform_deny(&replay, "replayed login/finish").await;

    drop_tenant(&admin, &created.tenant).await;
}

/// Native sign-in mints a WEAK native session: it resolves, carries the
/// credential, reports the binding state, and is refused every authenticator
/// and payout change on an account that holds a passkey.
#[tokio::test]
async fn pg_native_sign_in_is_a_weak_native_session() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let mut created = create_native_account(&state).await;
    // Bound, so the unbound gate is out of the way and only strength decides.
    bind_in_place(&admin, &created).await;

    let login = native_login(&state, &mut created).await;
    assert_eq!(
        login.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&login.bytes)
    );
    let body = login.json();
    assert_eq!(body["binding_state"], "bound");
    assert_eq!(body["account_id"], created.account_id.to_string());
    let token = body["access_token"].as_str().expect("token").to_string();
    let (tenant, token_hash) = native_token_parts(&token).unwrap();
    assert_eq!(tenant, created.tenant);
    assert_eq!(
        session_kind_and_credential(backend.as_ref(), &tenant, &token_hash).await,
        Some((
            NATIVE_SESSION_CLIENT_KIND.to_string(),
            Some(created.credential_id.clone())
        ))
    );
    let audits = pg_count(
        &admin,
        "SELECT count(*) FROM trace_account_audit
          WHERE tenant_id = $1 AND action = 'account_passkey_native_login'
            AND safe_metadata = '{\"client_kind\": \"native\"}'::jsonb",
        &tenant,
    )
    .await;
    assert_eq!(audits, 1);

    let binding = send(&state, get("/v1/account/binding", &token)).await;
    assert_eq!(
        binding.json(),
        serde_json::json!({ "binding_state": "bound" })
    );

    // Weak: every authenticator and payout change is refused.
    for request in [
        post(REGISTER_START, &serde_json::json!({}), Some(&token)),
        post(
            "/v1/account/passkeys/register/start",
            &serde_json::json!({}),
            Some(&token),
        ),
        axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/v1/account/passkeys/{}", created.credential_id))
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap(),
        axum::http::Request::builder()
            .method("PATCH")
            .uri("/v1/account/near-identities/ed25519:abc/payout")
            .header(AUTHORIZATION, format!("Bearer {token}"))
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(r#"{"payout":true}"#))
            .unwrap(),
    ] {
        let uri = request.uri().to_string();
        let reply = send(&state, request).await;
        assert_eq!(reply.status, StatusCode::FORBIDDEN, "{uri}");
        assert_eq!(
            reply.json()["error"],
            "a passkey or NEAR sign-in is required to change authenticators",
            "{uri}"
        );
    }

    // Rotation, logout and revoke-all behave as for any native session.
    let logout = send(
        &state,
        post("/v1/account/logout", &serde_json::json!({}), Some(&token)),
    )
    .await;
    assert!(logout.status.is_success(), "{}", logout.status);
    let after = send(&state, get("/v1/account/binding", &token)).await;
    assert_eq!(after.status, StatusCode::UNAUTHORIZED);

    drop_tenant(&admin, &created.tenant).await;
}

/// Removing a passkey revokes the native sessions it minted, and only those.
#[tokio::test]
async fn pg_removing_a_passkey_revokes_its_native_sessions() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let mut created = create_native_account(&state).await;
    bind_in_place(&admin, &created).await;

    let signed_in = native_login(&state, &mut created).await;
    assert_eq!(signed_in.status, StatusCode::OK);
    let signed_in = signed_in.json()["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    // A native session the passkey did not mint (auth_credential_id NULL).
    let other_secret = format!("unrelated-{}", Uuid::new_v4());
    admin
        .execute(
            "INSERT INTO trace_sessions
                (tenant_id, session_id, account_id, token_hash, client_kind, expires_at)
             VALUES ($1, $2, $3, $4, 'native', now() + interval '1 day')",
            &[
                &created.tenant,
                &Uuid::new_v4(),
                &created.account_id,
                &hash_secret(&other_secret),
            ],
        )
        .await
        .expect("unrelated session");
    let other = native_token_value(&created.tenant, &other_secret);

    // Removal needs a strong session: a browser passkey sign-in.
    let strong = passkey_login_cookie_value(
        &state,
        &mut created.authenticator,
        &created.credential_id,
        created.account_id,
    )
    .await;
    let removed = send(
        &state,
        axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/v1/account/passkeys/{}", created.credential_id))
            .header(
                axum::http::header::COOKIE,
                format!("tc_account_session={strong}"),
            )
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        removed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&removed.bytes)
    );

    for (what, token) in [
        ("create-time token", &created.token),
        ("sign-in token", &signed_in),
    ] {
        let reply = send(&state, get("/v1/account/binding", token)).await;
        assert_eq!(reply.status, StatusCode::UNAUTHORIZED, "{what} is revoked");
    }
    let reply = send(&state, get("/v1/account/binding", &other)).await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "a session the passkey did not mint survives"
    );

    drop_tenant(&admin, &created.tenant).await;
}

fn cookie_get(uri: &str, cookie_value: &str) -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method("GET")
        .uri(uri)
        .header(
            axum::http::header::COOKIE,
            format!("tc_account_session={cookie_value}"),
        )
        .body(Body::empty())
        .expect("request")
}

/// Removing a passkey revokes EVERY live session it minted, browser and
/// native, except the session making the removal request. Sessions minted by
/// another passkey survive.
#[tokio::test]
async fn pg_removing_a_passkey_revokes_every_session_it_minted_but_the_callers() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let mut created = create_native_account(&state).await;
    bind_in_place(&admin, &created).await;

    // P is the created passkey. The caller is a strong browser session from P.
    let caller = passkey_login_cookie_value(
        &state,
        &mut created.authenticator,
        &created.credential_id,
        created.account_id,
    )
    .await;
    // A second passkey Q, added from the caller's strong session.
    let (mut q_authenticator, q_credential) = enroll_passkey_for_token(&state, &caller).await;
    // Another browser session minted by P, and one minted by Q.
    let other_browser_from_p = passkey_login_cookie_value(
        &state,
        &mut created.authenticator,
        &created.credential_id,
        created.account_id,
    )
    .await;
    let browser_from_q = passkey_login_cookie_value(
        &state,
        &mut q_authenticator,
        &q_credential,
        created.account_id,
    )
    .await;
    // A native session minted by P (besides the create-time token).
    let native_from_p = native_login(&state, &mut created).await;
    assert_eq!(native_from_p.status, StatusCode::OK);
    let native_from_p = native_from_p.json()["access_token"]
        .as_str()
        .unwrap()
        .to_string();

    // Every session is live before the removal.
    for request in [
        cookie_get("/v1/account/binding", &caller),
        cookie_get("/v1/account/binding", &other_browser_from_p),
        cookie_get("/v1/account/binding", &browser_from_q),
        get("/v1/account/binding", &native_from_p),
        get("/v1/account/binding", &created.token),
    ] {
        assert_eq!(send(&state, request).await.status, StatusCode::OK);
    }

    let removed = send(
        &state,
        axum::http::Request::builder()
            .method("DELETE")
            .uri(format!("/v1/account/passkeys/{}", created.credential_id))
            .header(
                axum::http::header::COOKIE,
                format!("tc_account_session={caller}"),
            )
            .header("sec-fetch-site", "same-origin")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        removed.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&removed.bytes)
    );

    let status = |request| {
        let state = &state;
        async move { send(state, request).await.status }
    };
    assert_eq!(
        status(cookie_get("/v1/account/binding", &other_browser_from_p)).await,
        StatusCode::UNAUTHORIZED,
        "a browser session minted by the removed passkey is revoked"
    );
    assert_eq!(
        status(get("/v1/account/binding", &native_from_p)).await,
        StatusCode::UNAUTHORIZED,
        "a native session minted by the removed passkey is revoked"
    );
    assert_eq!(
        status(get("/v1/account/binding", &created.token)).await,
        StatusCode::UNAUTHORIZED,
        "the create-time native token is revoked"
    );
    assert_eq!(
        status(cookie_get("/v1/account/binding", &caller)).await,
        StatusCode::OK,
        "the caller's own session survives"
    );
    assert_eq!(
        status(cookie_get("/v1/account/binding", &browser_from_q)).await,
        StatusCode::OK,
        "a session minted by a different passkey survives"
    );

    drop_tenant(&admin, &created.tenant).await;
}

/// The authenticated native add-passkey route: a weak native session may add
/// the FIRST strong authenticator (the carve-out) and nothing after; the
/// ceremony is bound to the account that started it; a cookie session is
/// refused.
#[tokio::test]
async fn pg_native_register_adds_only_the_first_passkey() {
    let Some((backend, state)) = pg_state(None).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let tenant = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .expect("tenant");
    // Two legacy (near.ai-first shaped) accounts with no strong authenticator.
    let mut tokens = Vec::new();
    for _ in 0..2 {
        let account = Uuid::new_v4();
        admin
            .execute(
                "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
                &[&tenant, &account],
            )
            .await
            .expect("account");
        let secret = format!("register-{}", Uuid::new_v4());
        admin
            .execute(
                "INSERT INTO trace_sessions
                    (tenant_id, session_id, account_id, token_hash, client_kind, expires_at)
                 VALUES ($1, $2, $3, $4, 'native', now() + interval '1 day')",
                &[&tenant, &Uuid::new_v4(), &account, &hash_secret(&secret)],
            )
            .await
            .expect("session");
        tokens.push((account, native_token_value(&tenant, &secret)));
    }
    let (account_a, token_a) = tokens[0].clone();
    let (_account_b, token_b) = tokens[1].clone();

    let register = |token: String| {
        let state = state.clone();
        async move {
            let start = send(
                &state,
                post(REGISTER_START, &serde_json::json!({}), Some(&token)),
            )
            .await;
            (start.status, start.json())
        }
    };

    // Started by A, finished by B: refused, nothing stored.
    let (status, start) = register(token_a.clone()).await;
    assert_eq!(status, StatusCode::OK, "{start}");
    let mut authenticator = new_software_authenticator();
    let credential = softpasskey_register(&mut authenticator, &start["public_key"]);
    let finish_body = serde_json::json!({
        "ceremony_id": start["ceremony_id"],
        "credential": credential,
        "label": "Laptop",
    });
    let crossed = send(&state, post(REGISTER_FINISH, &finish_body, Some(&token_b))).await;
    assert_eq!(crossed.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        pg_count(
            &admin,
            "SELECT count(*) FROM trace_webauthn_credentials WHERE tenant_id = $1",
            &tenant
        )
        .await,
        0
    );

    // A's own start and finish add the first passkey (the carve-out).
    let (status, start) = register(token_a.clone()).await;
    assert_eq!(status, StatusCode::OK);
    let credential = softpasskey_register(&mut authenticator, &start["public_key"]);
    let finished = send(
        &state,
        post(
            REGISTER_FINISH,
            &serde_json::json!({ "ceremony_id": start["ceremony_id"], "credential": credential }),
            Some(&token_a),
        ),
    )
    .await;
    assert_eq!(
        finished.status,
        StatusCode::OK,
        "{}",
        String::from_utf8_lossy(&finished.bytes)
    );
    assert!(finished.json()["credential_id"].is_string());
    let stored = backend
        .list_account_credentials(&tenant, account_a)
        .await
        .expect("list");
    assert_eq!(stored.len(), 1);

    // A second passkey from the same weak session is refused.
    let (status, _) = register(token_a.clone()).await;
    assert_eq!(status, StatusCode::FORBIDDEN);

    drop_tenant(&admin, &tenant).await;
}

/// The count definer function sees every tenant's unbound rows and nothing
/// else, and the runtime role can call it but cannot read the table across
/// tenants itself.
#[tokio::test]
async fn pg_the_unbound_count_is_cross_tenant_only_through_the_definer() {
    let Some((backend, state)) = pg_state(Some(5)).await else {
        return;
    };
    let admin = pg_admin(&backend).await;
    let before = backend.count_unbound_passkey_accounts().await.unwrap();
    let one = create_native_account(&state).await;
    let two = create_native_account(&state).await;
    assert_eq!(
        backend.count_unbound_passkey_accounts().await.unwrap(),
        before + 2
    );
    bind_in_place(&admin, &two).await;
    assert_eq!(
        backend.count_unbound_passkey_accounts().await.unwrap(),
        before + 1,
        "a bound account is not counted"
    );

    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("client");
    let tx = client.transaction().await.expect("tx");
    tx.execute("SET LOCAL ROLE trace_ingest_runtime", &[])
        .await
        .expect("set role");
    let through_definer: i64 = tx
        .query_one("SELECT public.trace_unbound_passkey_account_count()", &[])
        .await
        .expect("the runtime may call the count")
        .get(0);
    assert_eq!(through_definer, before + 1);
    let direct: i64 = tx
        .query_one("SELECT count(*) FROM trace_account_bindings", &[])
        .await
        .expect("the runtime may select under RLS")
        .get(0);
    assert_eq!(
        direct, 0,
        "no tenant context: forced RLS shows the runtime nothing"
    );
    tx.rollback().await.expect("rollback");

    drop_tenant(&admin, &one.tenant).await;
    drop_tenant(&admin, &two.tenant).await;
}
