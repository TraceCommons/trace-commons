// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Connect near.ai: binding an unbound passkey account to a NEAR AI login
//! (Z2 native passkey identity, slice S3).
//!
//! Every test here is PostgreSQL-backed and self-skips without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL`; the anchor resolves through the
//! login-resolver role, so `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` is
//! needed as well, exactly as for the rest of the account suite.
//!
//! What is real: the bind and provisioning handlers over the real router and
//! account middleware, the ceremony rows, the device signatures (a real
//! Ed25519 key signing the bytes the server hands back), the shared preimage
//! functions, and the database writes. What is stubbed: NEAR AI's
//! `GET /users/me`, through the `cfg(test)` introspection seam, and the
//! passkey itself -- the unbound account is written by S2's own
//! `create_passkey_origin_account` with a placeholder passkey, because no
//! part of bind reads it.
//!
//! The readiness check reads process-global environment variables, which
//! [`ProvisioningEnv`] sets and restores; like the rest of the PostgreSQL
//! suite these tests assume `--test-threads=1`.

use super::*;
use crate::account_routes::ACCOUNT_UNBOUND;
use axum::body::Body;
use std::sync::atomic::{AtomicUsize, Ordering};
use tower::ServiceExt;

const BIND_START: &str = "/v1/account/near-ai/provision/bind/start";
const BIND_FINISH: &str = "/v1/account/near-ai/provision/bind/finish";
const PROVISION_START: &str = "/v1/account/near-ai/provision/start/v2";
const PROVISION_FINISH: &str = "/v1/account/near-ai/provision/finish/v2";

// --- Fixtures ----------------------------------------------------------------

/// The issuer variables `near_ai_login_readiness_named` reads, set for one
/// test and restored when it ends, so a later test in the same process sees
/// the environment it would have seen without this suite.
struct ProvisioningEnv {
    previous: Vec<(&'static str, Option<String>)>,
}

impl ProvisioningEnv {
    fn publish() -> Self {
        let vars = [
            (
                "TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL",
                "https://issuer.example",
            ),
            ("TRACE_COMMONS_NEAR_PROVISIONING_AUDIENCE", "upload"),
        ];
        let previous = vars
            .iter()
            .map(|(name, _)| (*name, std::env::var(name).ok()))
            .collect();
        for (name, value) in vars {
            // SAFETY: the PostgreSQL suite runs single-threaded.
            unsafe { std::env::set_var(name, value) };
        }
        Self { previous }
    }
}

impl Drop for ProvisioningEnv {
    fn drop(&mut self) {
        for (name, value) in self.previous.drain(..) {
            // SAFETY: as above.
            unsafe {
                match value {
                    Some(value) => std::env::set_var(name, value),
                    None => std::env::remove_var(name),
                }
            }
        }
    }
}

fn identity() -> trace_commons_server::near_account_identity::NearAccountIdentity {
    use base64::Engine as _;
    let crypto =
        trace_commons_server::secrets::SecretsCrypto::new(SecretString::from("b".repeat(32)))
            .expect("fixture SecretsCrypto");
    let kek: Arc<dyn trace_commons_server::trace_artifact_kek::KmsKeyWrapper> = Arc::new(
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(
            crypto,
            "near-ai-bind-fixture",
        ),
    );
    trace_commons_server::near_account_identity::NearAccountIdentity::from_parts(
        Some(&base64::engine::general_purpose::STANDARD.encode([5u8; 32])),
        Some(kek),
    )
    .expect("fixture identity")
}

/// A NEAR AI subject nobody has used, per call.
fn fresh_subject() -> String {
    format!("auth0|z2-s3-bind-{}", Uuid::new_v4().simple())
}

/// A stand-in for NEAR AI's `GET /users/me` answering `subject` to any token,
/// and a count of how often it was asked. The count is how a test proves a
/// refused finish never spent the token.
async fn stub_near_ai(subject: String) -> (String, Arc<AtomicUsize>) {
    let hits = Arc::new(AtomicUsize::new(0));
    let counter = hits.clone();
    let router = axum::Router::new().route(
        "/users/me",
        axum::routing::get(move || {
            let subject = subject.clone();
            let counter = counter.clone();
            async move {
                counter.fetch_add(1, Ordering::SeqCst);
                axum::Json(serde_json::json!({ "id": subject, "auth_provider": "github" }))
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind stub");
    let base = format!("http://{}", listener.local_addr().expect("addr"));
    tokio::spawn(async move { axum::serve(listener, router).await.expect("stub") });
    (base, hits)
}

struct Harness {
    backend: Arc<PgBackend>,
    state: Arc<AppState>,
    hits: Arc<AtomicUsize>,
    subject: String,
    near_ai_base: String,
    _env: ProvisioningEnv,
    // The state writes under this; it lives exactly as long as the state.
    _root: tempfile::TempDir,
}

impl Harness {
    fn anchor_hash(&self) -> String {
        identity().login_index_label(&self.subject)
    }

    fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    async fn admin(&self) -> deadpool_postgres::Object {
        self.backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("admin")
    }
}

async fn harness() -> Option<Harness> {
    let backend = postgres_backend_for_ingest_test().await?;
    let env = ProvisioningEnv::publish();
    let subject = fresh_subject();
    let (near_ai_base, hits) = stub_near_ai(subject.clone()).await;
    let temp = tempfile::tempdir().expect("temp dir");
    let mut state = test_state(temp.path().to_path_buf());
    let settings = Arc::get_mut(&mut state).expect("fresh state is uniquely owned");
    settings.near_provisioning_enabled = true;
    settings.near_provisioning_admission_ready = true;
    settings.near_account_identity = Some(Arc::new(identity()));
    settings.db_mirror = Some(backend.clone() as Arc<dyn Database>);
    settings.near_ai_introspection_base_url = Some(near_ai_base.clone());
    Some(Harness {
        backend,
        state,
        hits,
        subject,
        near_ai_base,
        _env: env,
        _root: temp,
    })
}

/// An unbound passkey-origin account, written by S2's own one-transaction
/// creator, and a native token for it.
struct Unbound {
    tenant: String,
    account_id: Uuid,
    token: String,
}

async fn unbound_account(backend: &PgBackend) -> Unbound {
    let tenant = trace_commons_server::near_account_identity::random_near_ai_tenant_id();
    let account_id = Uuid::new_v4();
    let secret = generate_session_secret();
    let outcome = backend
        .create_passkey_origin_account(trace_commons_server::db::NewPasskeyOriginAccount {
            tenant_id: &tenant,
            account_id,
            credential_id: &format!("z2-s3-{}", Uuid::new_v4().simple()),
            passkey: &serde_json::json!({ "fixture": "bind reads no passkey" }),
            label: None,
            session: trace_commons_server::db::NewSession {
                token_hash: &hash_secret(&secret),
                client_kind: NATIVE_SESSION_CLIENT_KIND,
                expires_at: Utc::now() + Duration::hours(1),
            },
            ceiling: i64::MAX,
        })
        .await
        .expect("create an unbound account");
    assert_eq!(
        outcome,
        trace_commons_server::db::PasskeyOriginAccountOutcome::Created
    );
    Unbound {
        token: native_token_value(&tenant, &secret),
        tenant,
        account_id,
    }
}

/// A daemon's device key.
struct Device {
    pair: ring::signature::Ed25519KeyPair,
}

impl Device {
    fn new() -> Self {
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = ring::signature::Ed25519KeyPair::generate_pkcs8(&rng).expect("pkcs8");
        Self {
            pair: ring::signature::Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("key"),
        }
    }

    fn public_bytes(&self) -> [u8; 32] {
        use ring::signature::KeyPair as _;
        self.pair
            .public_key()
            .as_ref()
            .try_into()
            .expect("32 bytes")
    }

    fn public_b64(&self) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(self.public_bytes())
    }

    fn sign(&self, bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(self.pair.sign(bytes).as_ref())
    }
}

fn pkce() -> (String, String) {
    use base64::Engine as _;
    use sha2::Digest as _;
    let verifier = "v".repeat(64);
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(sha2::Sha256::digest(verifier.as_bytes()));
    (verifier, challenge)
}

struct Reply {
    status: StatusCode,
    bytes: Vec<u8>,
}

impl Reply {
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.bytes).unwrap_or_default()
    }
}

async fn send(
    state: &Arc<AppState>,
    method: &str,
    uri: &str,
    body: Option<&serde_json::Value>,
    bearer: Option<&str>,
) -> Reply {
    let mut builder = axum::http::Request::builder().method(method).uri(uri);
    if let Some(token) = bearer {
        builder = builder.header(AUTHORIZATION, format!("Bearer {token}"));
    }
    let body = match body {
        Some(body) => {
            builder = builder.header(CONTENT_TYPE, "application/json");
            Body::from(serde_json::to_vec(body).expect("json"))
        }
        None => Body::empty(),
    };
    let response = app(state.clone())
        .oneshot(builder.body(body).expect("request"))
        .await
        .expect("router is infallible");
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body")
        .to_vec();
    Reply { status, bytes }
}

async fn assert_uniform_deny(reply: &Reply, what: &str) {
    let response = native_generic_deny();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .expect("body")
        .to_vec();
    assert_eq!(reply.status, status, "{what}: status");
    assert_eq!(reply.bytes, bytes, "{what}: body");
}

/// A started ceremony as the client sees it.
struct Started {
    ceremony_id: String,
    nonce: [u8; 32],
    expires_at: i64,
    signing_bytes: Vec<u8>,
    challenge: String,
    verifier: String,
}

async fn start(state: &Arc<AppState>, uri: &str, bearer: Option<&str>, device: &Device) -> Started {
    use base64::Engine as _;
    let (verifier, challenge) = pkce();
    let reply = send(
        state,
        "POST",
        uri,
        Some(&serde_json::json!({
            "device_public_key": device.public_b64(),
            "code_challenge": challenge,
            "code_challenge_method": "S256",
        })),
        bearer,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{uri}: {:?}", reply.json());
    let body = reply.json();
    let decode = |field: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(body[field].as_str().expect(field))
            .expect(field)
    };
    Started {
        ceremony_id: body["ceremony_id"].as_str().expect("id").to_string(),
        nonce: decode("nonce").try_into().expect("32-byte nonce"),
        expires_at: body["expires_at"].as_i64().expect("expiry"),
        signing_bytes: decode("device_signing_bytes"),
        challenge,
        verifier,
    }
}

async fn finish(
    state: &Arc<AppState>,
    uri: &str,
    bearer: Option<&str>,
    started: &Started,
    device: &Device,
    signature: &str,
) -> Reply {
    send(
        state,
        "POST",
        uri,
        Some(&serde_json::json!({
            "ceremony_id": started.ceremony_id,
            "code_verifier": started.verifier,
            "device_public_key": device.public_b64(),
            "device_signature": signature,
            "access_token": "near-ai-access-token-fixture",
        })),
        bearer,
    )
    .await
}

/// The whole bind as a well-behaved client runs it.
async fn bind(h: &Harness, account: &Unbound, device: &Device) -> Reply {
    let started = start(&h.state, BIND_START, Some(&account.token), device).await;
    let signature = device.sign(&started.signing_bytes);
    finish(
        &h.state,
        BIND_FINISH,
        Some(&account.token),
        &started,
        device,
        &signature,
    )
    .await
}

/// The unauthenticated provisioning an existing NEAR AI contributor ran:
/// account X.
async fn provision(h: &Harness, device: &Device) -> serde_json::Value {
    let started = start(&h.state, PROVISION_START, None, device).await;
    let signature = device.sign(&started.signing_bytes);
    let reply = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        device,
        &signature,
    )
    .await;
    assert_eq!(
        reply.status,
        StatusCode::OK,
        "provision: {:?}",
        reply.json()
    );
    reply.json()
}

async fn binding_row(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
) -> (String, bool) {
    let row = admin
        .query_one(
            "SELECT state, bound_at IS NOT NULL FROM trace_account_bindings
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account],
        )
        .await
        .expect("binding row");
    (row.get(0), row.get(1))
}

async fn count(admin: &deadpool_postgres::Object, sql: &str, params: &[&str]) -> i64 {
    let params: Vec<&(dyn tokio_postgres::types::ToSql + Sync)> = params
        .iter()
        .map(|p| p as &(dyn tokio_postgres::types::ToSql + Sync))
        .collect();
    admin
        .query_one(sql, &params)
        .await
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .get(0)
}

/// The rows a bind can write into a tenant, other than sessions and audit.
async fn linked_rows(admin: &deadpool_postgres::Object, tenant: &str) -> [i64; 4] {
    [
        count(
            admin,
            "SELECT count(*) FROM trace_near_account_anchors WHERE tenant_id = $1",
            &[tenant],
        )
        .await,
        count(
            admin,
            "SELECT count(*) FROM trace_near_provisioned_devices WHERE tenant_id = $1",
            &[tenant],
        )
        .await,
        count(
            admin,
            "SELECT count(*) FROM device_keys WHERE tenant_id = $1",
            &[tenant],
        )
        .await,
        count(
            admin,
            "SELECT count(*) FROM trace_account_principals WHERE tenant_id = $1",
            &[tenant],
        )
        .await,
    ]
}

async fn audit_actions(admin: &deadpool_postgres::Object, tenant: &str) -> Vec<String> {
    admin
        .query(
            "SELECT action FROM trace_account_audit WHERE tenant_id = $1 ORDER BY 1",
            &[&tenant],
        )
        .await
        .expect("audit")
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

async fn audit_metadata(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    action: &str,
) -> Vec<serde_json::Value> {
    admin
        .query(
            "SELECT safe_metadata FROM trace_account_audit WHERE tenant_id = $1 AND action = $2",
            &[&tenant, &action],
        )
        .await
        .expect("audit")
        .into_iter()
        .map(|row| row.get(0))
        .collect()
}

async fn anchors_for(admin: &deadpool_postgres::Object, anchor_hash: &str) -> Vec<(String, Uuid)> {
    admin
        .query(
            "SELECT tenant_id, account_id FROM trace_near_account_anchors WHERE anchor_hash = $1",
            &[&anchor_hash],
        )
        .await
        .expect("anchors")
        .into_iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect()
}

/// Remove what a test created. The anchor and provisioned-device tables do
/// not cascade from the tenant, so they go first; a cleanup that fails is a
/// test failure rather than a silent leftover.
async fn drop_tenants(admin: &deadpool_postgres::Object, tenants: &[&str]) {
    for tenant in tenants {
        for table in [
            "trace_near_provisioned_devices",
            "trace_near_account_anchors",
            "trace_tenants",
        ] {
            admin
                .execute(
                    &format!("DELETE FROM {table} WHERE tenant_id = $1"),
                    &[tenant],
                )
                .await
                .unwrap_or_else(|error| panic!("cleanup {table}: {error}"));
        }
    }
}

async fn binding_state_via(h: &Harness, token: &str) -> Reply {
    send(&h.state, "GET", "/v1/account/binding", None, Some(token)).await
}

// --- Tests -------------------------------------------------------------------

/// Unbound becomes bound, and every row the bind writes is in the passkey
/// account's own tenant and names that account.
#[tokio::test]
async fn pg_bind_moves_an_unbound_account_to_bound_in_its_own_tenant() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let account = unbound_account(&h.backend).await;
    let device = Device::new();
    assert_eq!(linked_rows(&admin, &account.tenant).await, [0; 4]);

    let reply = bind(&h, &account, &device).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    let body = reply.json();
    assert_eq!(body["outcome"], "bound");
    assert_eq!(body["binding_state"], "bound");
    assert_eq!(body["tenant_id"], account.tenant.as_str());
    assert_eq!(body["account_id"], account.account_id.to_string());
    assert_eq!(body["anchor_hash"], h.anchor_hash().as_str());
    assert_eq!(h.hits(), 1, "introspected exactly once");

    // The state flipped, with bound_at.
    assert_eq!(
        binding_row(&admin, &account.tenant, account.account_id).await,
        ("bound".to_string(), true)
    );
    // The anchor, device, provisioned device and principal: one each, all in
    // this tenant, all naming this account.
    assert_eq!(linked_rows(&admin, &account.tenant).await, [1; 4]);
    assert_eq!(
        anchors_for(&admin, &h.anchor_hash()).await,
        vec![(account.tenant.clone(), account.account_id)],
        "one anchor in the world for this subject, and it is this account's"
    );
    for table in [
        "trace_near_account_anchors",
        "trace_near_provisioned_devices",
        "trace_account_principals",
    ] {
        let elsewhere = count(
            &admin,
            &format!(
                "SELECT count(*) FROM {table} WHERE account_id::text = $1 AND tenant_id <> $2"
            ),
            &[&account.account_id.to_string(), &account.tenant],
        )
        .await;
        assert_eq!(elsewhere, 0, "{table} row outside the account's tenant");
        let wrong_account = count(
            &admin,
            &format!(
                "SELECT count(*) FROM {table} WHERE tenant_id = $1 AND account_id::text <> $2"
            ),
            &[&account.tenant, &account.account_id.to_string()],
        )
        .await;
        assert_eq!(wrong_account, 0, "{table} row for another account");
    }
    let origin: String = admin
        .query_one(
            "SELECT onboarding_origin FROM device_keys WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("device key")
        .get(0);
    assert_eq!(origin, "near_ai");
    let source: String = admin
        .query_one(
            "SELECT identity_source FROM trace_near_account_anchors WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await
        .expect("anchor")
        .get(0);
    assert_eq!(source, "near_ai_login");

    // Hash-only audit: started and bound, with label metadata only.
    let actions = audit_actions(&admin, &account.tenant).await;
    assert!(actions.contains(&"account_binding_started".to_string()));
    assert!(actions.contains(&"account_bound".to_string()));
    assert_eq!(
        audit_metadata(&admin, &account.tenant, "account_bound").await,
        vec![serde_json::json!({ "identity": "near_ai_login" })]
    );
    assert_eq!(
        audit_metadata(&admin, &account.tenant, "account_binding_started").await,
        vec![serde_json::json!({})]
    );
    for metadata in audit_metadata(&admin, &account.tenant, "account_bound").await {
        let text = metadata.to_string();
        assert!(!text.contains(&h.subject) && !text.contains(&h.anchor_hash()));
    }

    // The returned session is a native session on the same account.
    let token = body["access_token"].as_str().expect("token");
    let state = binding_state_via(&h, token).await;
    assert_eq!(state.status, StatusCode::OK);
    assert_eq!(state.json()["binding_state"], "bound");

    // A later NEAR AI provisioning for the same subject lands in this account.
    let later = provision(&h, &Device::new()).await;
    assert_eq!(later["tenant_id"], account.tenant.as_str());
    assert_eq!(later["account_id"], account.account_id.to_string());

    drop_tenants(&admin, &[&account.tenant]).await;
}

/// A bound account's gate lifts: a route the gate refuses an unbound account
/// is reached, by the original session and by the one bind returned. And a
/// second bind is refused as already bound.
#[tokio::test]
async fn pg_a_bound_accounts_gate_lifts_and_it_cannot_bind_again() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let account = unbound_account(&h.backend).await;

    let before = send(
        &h.state,
        "GET",
        "/v1/account/traces",
        None,
        Some(&account.token),
    )
    .await;
    assert_eq!(before.status, StatusCode::FORBIDDEN);
    assert_eq!(before.json()["error"], ACCOUNT_UNBOUND);

    let reply = bind(&h, &account, &Device::new()).await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    let fresh = reply.json()["access_token"]
        .as_str()
        .expect("token")
        .to_string();

    for token in [&account.token, &fresh] {
        let after = send(&h.state, "GET", "/v1/account/traces", None, Some(token)).await;
        assert_eq!(after.status, StatusCode::OK, "{:?}", after.json());
    }

    let again = send(
        &h.state,
        "POST",
        BIND_START,
        Some(&serde_json::json!({
            "device_public_key": Device::new().public_b64(),
            "code_challenge": pkce().1,
            "code_challenge_method": "S256",
        })),
        Some(&account.token),
    )
    .await;
    assert_eq!(again.status, StatusCode::CONFLICT);
    assert_eq!(again.json()["error"], "account_already_bound");

    drop_tenants(&admin, &[&account.tenant]).await;
}

/// A legacy account (no binding row) is not a passkey account: bind refuses
/// it at start, before any ceremony is stored.
#[tokio::test]
async fn pg_a_legacy_account_cannot_bind() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let x = provision(&h, &Device::new()).await;
    let token = x["access_token"].as_str().expect("token");
    assert_eq!(
        binding_state_via(&h, token).await.json()["binding_state"],
        "legacy"
    );
    let reply = send(
        &h.state,
        "POST",
        BIND_START,
        Some(&serde_json::json!({
            "device_public_key": Device::new().public_b64(),
            "code_challenge": pkce().1,
            "code_challenge_method": "S256",
        })),
        Some(token),
    )
    .await;
    assert_eq!(reply.status, StatusCode::CONFLICT);
    assert_eq!(reply.json()["error"], "account_already_bound");
    drop_tenants(&admin, &[x["tenant_id"].as_str().expect("tenant")]).await;
}

/// A bind transaction that fails after the state flip and the anchor insert
/// leaves the account unbound with no anchor, device, principal or session.
/// Two failure points: a device key already registered to another tenant
/// (refused mid-transaction), and a session token hash that collides (the
/// transaction's last insert).
#[tokio::test]
async fn pg_a_bind_that_aborts_mid_transaction_leaves_the_account_unbound() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let login = trace_commons_server::near_ai_login::introspect_login(
        &h.near_ai_base,
        &SecretString::from("token".to_string()),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("stub introspection");

    // A device key held by another tenant.
    let elsewhere = unbound_account(&h.backend).await;
    let held = Device::new();
    let held_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
        &held.public_bytes(),
    );
    admin
        .execute(
            "INSERT INTO device_keys (device_key_id, tenant_id, public_key, onboarding_origin)
             VALUES ($1, $2, $3, 'near_ai')",
            &[&held_id, &elsewhere.tenant, &held.public_b64()],
        )
        .await
        .expect("device key elsewhere");
    // A session hash that already exists.
    let existing_hash = native_token_parts(&elsewhere.token).expect("token").1;

    for (what, device, token_hash) in [
        (
            "device key registered to another tenant",
            held.public_bytes(),
            hash_secret(&generate_session_secret()),
        ),
        (
            "session hash collision",
            Device::new().public_bytes(),
            existing_hash,
        ),
    ] {
        let account = unbound_account(&h.backend).await;
        let sessions_before = count(
            &admin,
            "SELECT count(*) FROM trace_sessions WHERE tenant_id = $1",
            &[&account.tenant],
        )
        .await;
        let result = h
            .backend
            .bind_near_ai_login(
                &account.tenant,
                account.account_id,
                &login,
                &device,
                trace_commons_server::db::NewSession {
                    token_hash: &token_hash,
                    client_kind: NATIVE_SESSION_CLIENT_KIND,
                    expires_at: Utc::now() + Duration::hours(1),
                },
                &identity(),
            )
            .await;
        assert!(result.is_err(), "{what}: the bind must fail");
        assert_eq!(
            binding_row(&admin, &account.tenant, account.account_id).await,
            ("unbound".to_string(), false),
            "{what}: still unbound"
        );
        assert_eq!(linked_rows(&admin, &account.tenant).await, [0; 4], "{what}");
        assert!(
            anchors_for(&admin, &h.anchor_hash()).await.is_empty(),
            "{what}: no anchor anywhere"
        );
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_sessions WHERE tenant_id = $1",
                &[&account.tenant],
            )
            .await,
            sessions_before,
            "{what}: no session"
        );
        assert!(
            !audit_actions(&admin, &account.tenant)
                .await
                .contains(&"account_bound".to_string()),
            "{what}: no bound audit"
        );
        drop_tenants(&admin, &[&account.tenant]).await;
    }
    drop_tenants(&admin, &[&elsewhere.tenant]).await;
}

/// Two passkey accounts bind the same NEAR AI account at once. The anchor's
/// uniqueness and the existing race handling decide: exactly one is bound,
/// the other takes the existing-account branch into the winner and is closed.
#[tokio::test]
async fn pg_two_concurrent_binds_of_one_near_ai_account_have_one_winner() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let one = unbound_account(&h.backend).await;
    let two = unbound_account(&h.backend).await;
    let (d1, d2) = (Device::new(), Device::new());
    let s1 = start(&h.state, BIND_START, Some(&one.token), &d1).await;
    let s2 = start(&h.state, BIND_START, Some(&two.token), &d2).await;
    let (sig1, sig2) = (d1.sign(&s1.signing_bytes), d2.sign(&s2.signing_bytes));
    let (r1, r2) = tokio::join!(
        finish(&h.state, BIND_FINISH, Some(&one.token), &s1, &d1, &sig1),
        finish(&h.state, BIND_FINISH, Some(&two.token), &s2, &d2, &sig2),
    );
    assert_eq!(r1.status, StatusCode::OK, "{:?}", r1.json());
    assert_eq!(r2.status, StatusCode::OK, "{:?}", r2.json());
    let outcomes = [
        r1.json()["outcome"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        r2.json()["outcome"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
    ];
    let mut sorted = outcomes.clone();
    sorted.sort();
    assert_eq!(sorted, ["bound", "existing_account"], "exactly one winner");
    let (winner, loser, loser_reply) = if outcomes[0] == "bound" {
        (&one, &two, &r2)
    } else {
        (&two, &one, &r1)
    };

    assert_eq!(
        binding_row(&admin, &winner.tenant, winner.account_id).await,
        ("bound".to_string(), true)
    );
    assert_eq!(
        binding_row(&admin, &loser.tenant, loser.account_id).await,
        ("closed".to_string(), false)
    );
    assert_eq!(
        anchors_for(&admin, &h.anchor_hash()).await,
        vec![(winner.tenant.clone(), winner.account_id)]
    );
    assert_eq!(linked_rows(&admin, &loser.tenant).await, [0; 4]);
    // The loser was handed the winner's session, as provisioning would have.
    assert_eq!(loser_reply.json()["tenant_id"], winner.tenant.as_str());
    assert_eq!(
        loser_reply.json()["account_id"],
        winner.account_id.to_string()
    );

    drop_tenants(&admin, &[&one.tenant, &two.tenant]).await;
}

/// The refuse branch. NEAR AI subject S already anchors account X. Passkey
/// account P binds S: P is closed in its own tenant, the response is X's
/// session exactly as provisioning grants it, and nothing crosses between
/// the two tenants -- not P's passkey, not a row naming P in X's tenant, not a
/// row naming X in P's.
#[tokio::test]
async fn pg_binding_an_existing_near_ai_account_refuses_and_returns_its_session() {
    for x_is_strong in [true, false] {
        let Some(h) = harness().await else {
            return;
        };
        let admin = h.admin().await;
        let x = provision(&h, &Device::new()).await;
        let x_tenant = x["tenant_id"].as_str().expect("tenant").to_string();
        let x_account = Uuid::parse_str(x["account_id"].as_str().expect("account")).expect("uuid");
        if x_is_strong {
            admin
                .execute(
                    "INSERT INTO trace_webauthn_credentials
                        (tenant_id, credential_id, account_id, passkey, label, created_at)
                     VALUES ($1, $2, $3, '{}'::jsonb, NULL, now())",
                    &[
                        &x_tenant,
                        &format!("z2-s3-x-{}", Uuid::new_v4().simple()),
                        &x_account,
                    ],
                )
                .await
                .expect("X's passkey");
        }
        let x_credentials_before = count(
            &admin,
            "SELECT count(*) FROM trace_webauthn_credentials WHERE tenant_id = $1",
            &[&x_tenant],
        )
        .await;

        let p = unbound_account(&h.backend).await;
        let p_credential: String = admin
            .query_one(
                "SELECT credential_id FROM trace_webauthn_credentials WHERE tenant_id = $1",
                &[&p.tenant],
            )
            .await
            .expect("P's passkey")
            .get(0);

        let reply = bind(&h, &p, &Device::new()).await;
        assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
        let body = reply.json();
        assert_eq!(body["outcome"], "existing_account");
        assert_eq!(body["binding_state"], "legacy", "X has no binding row");
        assert_eq!(body["tenant_id"], x_tenant.as_str());
        assert_eq!(body["account_id"], x_account.to_string());
        let x_token = body["access_token"].as_str().expect("token");
        assert_eq!(native_token_parts(x_token).expect("tcn1_").0, x_tenant);
        // X's session works, as provisioning's would.
        let resolved = binding_state_via(&h, x_token).await;
        assert_eq!(resolved.status, StatusCode::OK);
        assert_eq!(resolved.json()["binding_state"], "legacy");

        // P: closed, sessions and credential revoked, nothing linked.
        assert_eq!(
            binding_row(&admin, &p.tenant, p.account_id).await,
            ("closed".to_string(), false)
        );
        assert_eq!(
            binding_state_via(&h, &p.token).await.status,
            StatusCode::UNAUTHORIZED,
            "P's sessions are revoked"
        );
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_webauthn_credentials
                  WHERE tenant_id = $1 AND revoked_at IS NULL",
                &[&p.tenant],
            )
            .await,
            0
        );
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_accounts WHERE tenant_id = $1 AND closed_at IS NOT NULL",
                &[&p.tenant],
            )
            .await,
            1
        );
        assert_eq!(linked_rows(&admin, &p.tenant).await, [0; 4]);
        let reason = if x_is_strong {
            "anchor_claimed_strong"
        } else {
            "anchor_claimed"
        };
        assert_eq!(
            audit_metadata(&admin, &p.tenant, "account_binding_refused").await,
            vec![serde_json::json!({ "reason": reason })]
        );

        // Nothing crossed. P's passkey is still P's, in P's tenant, and X
        // gained no credential. No row in X's tenant names P's account, and no
        // row in P's tenant names X's.
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_webauthn_credentials WHERE credential_id = $1 AND tenant_id = $2",
                &[&p_credential, &p.tenant],
            )
            .await,
            1
        );
        assert_eq!(
            count(
                &admin,
                "SELECT count(*) FROM trace_webauthn_credentials WHERE tenant_id = $1",
                &[&x_tenant],
            )
            .await,
            x_credentials_before
        );
        for table in [
            "trace_accounts",
            "trace_account_bindings",
            "trace_webauthn_credentials",
            "trace_sessions",
            "trace_account_principals",
            "trace_near_account_anchors",
            "trace_near_provisioned_devices",
        ] {
            for (tenant, foreign) in [(&x_tenant, p.account_id), (&p.tenant, x_account)] {
                assert_eq!(
                    count(
                        &admin,
                        &format!(
                            "SELECT count(*) FROM {table} WHERE tenant_id = $1 AND account_id::text = $2"
                        ),
                        &[tenant, &foreign.to_string()],
                    )
                    .await,
                    0,
                    "{table}: a row in {tenant} names the other account"
                );
            }
        }
        // The anchor is X's, unchanged; X's tenant got exactly the
        // provisioning audit, P's got the refusal.
        assert_eq!(
            anchors_for(&admin, &h.anchor_hash()).await,
            vec![(x_tenant.clone(), x_account)]
        );
        assert_eq!(
            audit_metadata(&admin, &x_tenant, "near_ai_login_provisioned")
                .await
                .len(),
            2,
            "X's original provisioning and the one the refused bind granted"
        );
        assert!(
            !audit_actions(&admin, &x_tenant)
                .await
                .iter()
                .any(|action| action.starts_with("account_bind")),
            "no bind audit in X's tenant"
        );

        drop_tenants(&admin, &[&x_tenant, &p.tenant]).await;
    }
}

/// A ceremony started by account A cannot be finished by account B, neither
/// with A's signature nor with a signature the same device key makes for B.
#[tokio::test]
async fn pg_a_bind_ceremony_cannot_be_finished_by_another_account() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let a = unbound_account(&h.backend).await;
    let b = unbound_account(&h.backend).await;
    let device = Device::new();

    // B presents A's ceremony with A's signature.
    let started = start(&h.state, BIND_START, Some(&a.token), &device).await;
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&b.token),
        &started,
        &device,
        &device.sign(&started.signing_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "A's signature, B's session").await;

    // B holds the same device key and signs A's ceremony over B's account.
    let started = start(&h.state, BIND_START, Some(&a.token), &device).await;
    let for_b = trace_commons_protocol::onboarding::near_ai_bind_device_bytes(
        &started.nonce,
        &started.ceremony_id,
        &device.public_bytes(),
        &started.challenge,
        started.expires_at,
        &b.account_id,
    );
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&b.token),
        &started,
        &device,
        &device.sign(&for_b),
    )
    .await;
    assert_uniform_deny(&reply, "a signature for B over A's ceremony").await;

    assert_eq!(h.hits(), 0, "the token was never spent");
    for account in [&a, &b] {
        assert_eq!(
            binding_row(&admin, &account.tenant, account.account_id).await,
            ("unbound".to_string(), false)
        );
        assert_eq!(linked_rows(&admin, &account.tenant).await, [0; 4]);
    }
    assert_eq!(
        audit_metadata(&admin, &b.tenant, "account_binding_failed").await,
        vec![
            serde_json::json!({ "stage": "ceremony_account" }),
            serde_json::json!({ "stage": "ceremony_account" }),
        ]
    );
    drop_tenants(&admin, &[&a.tenant, &b.tenant]).await;
}

/// The two ceremonies do not accept each other: a provisioning preimage is
/// refused by bind, a bind preimage by provisioning, and a ceremony id from
/// one is refused by the other's finish.
#[tokio::test]
async fn pg_bind_and_provisioning_refuse_each_others_preimages_and_ceremonies() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let p = unbound_account(&h.backend).await;
    let device = Device::new();

    // Bind finish, signed over the provisioning preimage of the same inputs.
    let started = start(&h.state, BIND_START, Some(&p.token), &device).await;
    let provisioning_bytes = trace_commons_protocol::onboarding::near_ai_provisioning_device_bytes(
        &started.nonce,
        &started.ceremony_id,
        &device.public_bytes(),
        &started.challenge,
        started.expires_at,
    );
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&p.token),
        &started,
        &device,
        &device.sign(&provisioning_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "provisioning preimage at bind").await;

    // Provisioning finish, signed over the bind preimage of the same inputs.
    let started = start(&h.state, PROVISION_START, None, &device).await;
    let bind_bytes = trace_commons_protocol::onboarding::near_ai_bind_device_bytes(
        &started.nonce,
        &started.ceremony_id,
        &device.public_bytes(),
        &started.challenge,
        started.expires_at,
        &p.account_id,
    );
    let reply = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        &device,
        &device.sign(&bind_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "bind preimage at provisioning").await;

    // A bind ceremony, correctly signed, presented to provisioning finish.
    let started = start(&h.state, BIND_START, Some(&p.token), &device).await;
    let reply = finish(
        &h.state,
        PROVISION_FINISH,
        None,
        &started,
        &device,
        &device.sign(&started.signing_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "bind ceremony at provisioning finish").await;

    // A provisioning ceremony, correctly signed, presented to bind finish.
    let started = start(&h.state, PROVISION_START, None, &device).await;
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&p.token),
        &started,
        &device,
        &device.sign(&started.signing_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "provisioning ceremony at bind finish").await;

    assert_eq!(h.hits(), 0, "no refused finish spent the token");
    assert!(anchors_for(&admin, &h.anchor_hash()).await.is_empty());
    assert_eq!(
        binding_row(&admin, &p.tenant, p.account_id).await,
        ("unbound".to_string(), false)
    );
    assert_eq!(linked_rows(&admin, &p.tenant).await, [0; 4]);
    drop_tenants(&admin, &[&p.tenant]).await;
}

/// A leaked NEAR AI token is worth nothing at bind without the device
/// signature: no signature, a garbage one, one by another key, and one over
/// the wrong bytes are all refused before the token is spent.
#[tokio::test]
async fn pg_a_leaked_near_ai_token_without_the_device_signature_gets_nothing() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let p = unbound_account(&h.backend).await;
    let device = Device::new();
    let thief = Device::new();

    for (what, signature) in [
        ("empty signature", String::new()),
        ("garbage signature", "AAAA".repeat(22)),
        ("another key's signature", String::from("other")),
        ("a signature over other bytes", String::from("wrong")),
    ] {
        let started = start(&h.state, BIND_START, Some(&p.token), &device).await;
        let signature = match signature.as_str() {
            "other" => thief.sign(&started.signing_bytes),
            "wrong" => device.sign(b"not the ceremony"),
            _ => signature,
        };
        let reply = finish(
            &h.state,
            BIND_FINISH,
            Some(&p.token),
            &started,
            &device,
            &signature,
        )
        .await;
        assert_uniform_deny(&reply, what).await;
    }
    // The thief's own key, claimed as the device, against a ceremony that
    // committed to the real one.
    let started = start(&h.state, BIND_START, Some(&p.token), &device).await;
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&p.token),
        &started,
        &thief,
        &thief.sign(&started.signing_bytes),
    )
    .await;
    assert_uniform_deny(&reply, "a different device key").await;

    assert_eq!(h.hits(), 0, "the token was never introspected");
    assert!(anchors_for(&admin, &h.anchor_hash()).await.is_empty());
    assert_eq!(
        binding_row(&admin, &p.tenant, p.account_id).await,
        ("unbound".to_string(), false)
    );
    assert_eq!(linked_rows(&admin, &p.tenant).await, [0; 4]);
    drop_tenants(&admin, &[&p.tenant]).await;
}

/// V100 on the runtime role, not a superuser.
///
/// A superuser passes every privilege check, so a bind that only works as one
/// would pass every other test here. This runs the bind (both branches)
/// through a backend connected as a NOSUPERUSER NOBYPASSRLS login whose only
/// privileges on `trace_account_bindings` come from membership in
/// `trace_ingest_runtime` (V97, V98, V100). Its other table grants stand in
/// for the pilot's V62-era runtime grants, which the provisioning writes
/// already rely on and which a fresh database does not have.
///
/// The control is a login with identical grants plus V97's and V98's
/// privileges on the binding table granted directly, but no membership:
/// everything but V100. Its bind must fail and write nothing, which is what
/// shows the first login's pass is V100's doing and not a privilege it
/// happened to hold.
#[tokio::test]
async fn pg_bind_works_as_the_runtime_role_and_not_without_v100() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let suffix = std::process::id();
    let runtime = format!("tc_z2_s3_runtime_{suffix}");
    let control = format!("tc_z2_s3_control_{suffix}");
    let tables = "trace_tenants, trace_accounts, trace_sessions, trace_account_audit, \
                  trace_webauthn_credentials, trace_near_identities, trace_near_account_anchors, \
                  trace_near_provisioned_devices, device_keys, trace_account_principals";
    for role in [&runtime, &control] {
        let _ = admin
            .batch_execute(&format!(
                "DROP OWNED BY {role}; DROP ROLE IF EXISTS {role};"
            ))
            .await;
        admin
            .batch_execute(&format!(
                "CREATE ROLE {role} LOGIN NOSUPERUSER NOBYPASSRLS;
                 GRANT USAGE ON SCHEMA public TO {role};
                 GRANT SELECT, INSERT ON {tables} TO {role};
                 GRANT UPDATE (revoked_at) ON trace_sessions, trace_webauthn_credentials TO {role};
                 GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO {role};"
            ))
            .await
            .unwrap_or_else(|error| panic!("create {role}: {error}"));
    }
    admin
        .batch_execute(&format!(
            "GRANT trace_ingest_runtime TO {runtime};
             GRANT SELECT, INSERT ON trace_account_bindings TO {control};
             GRANT UPDATE (closed_at) ON trace_accounts TO {control};"
        ))
        .await
        .expect("memberships");
    for role in [&runtime, &control] {
        let privileged: bool = admin
            .query_one(
                "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname = $1",
                &[role],
            )
            .await
            .expect("role")
            .get(0);
        assert!(!privileged, "{role} must be an ordinary login");
    }

    let as_role = |role: &str| {
        let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
            .or_else(|_| std::env::var("DATABASE_URL"))
            .expect("url");
        let mut url = reqwest::Url::parse(&url).expect("url");
        url.set_username(role).expect("user");
        url.set_password(None).expect("password");
        DatabaseConfig {
            url: SecretString::from(url.to_string()),
            pool_size: 2,
            ssl_mode: trace_commons_server::config::SslMode::Prefer,
            login_resolver_url:
                trace_commons_server::config::DatabaseConfig::login_resolver_url_from_env(),
            gate_driver_url: None,
            pii_backstop_driver_url: None,
            invite_registry_url: None,
        }
    };
    let runtime_backend = PgBackend::new(&as_role(&runtime)).await.expect("runtime");
    let control_backend = PgBackend::new(&as_role(&control)).await.expect("control");
    let whoami: bool = runtime_backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("runtime client")
        .query_one(
            "SELECT rolsuper FROM pg_roles WHERE rolname = current_user",
            &[],
        )
        .await
        .expect("current_user")
        .get(0);
    assert!(
        !whoami,
        "the runtime backend is not connected as a superuser"
    );

    let login = trace_commons_server::near_ai_login::introspect_login(
        &h.near_ai_base,
        &SecretString::from("token".to_string()),
        std::time::Duration::from_secs(5),
    )
    .await
    .expect("stub introspection");
    fn session(hash: &str) -> trace_commons_server::db::NewSession<'_> {
        trace_commons_server::db::NewSession {
            token_hash: hash,
            client_kind: NATIVE_SESSION_CLIENT_KIND,
            expires_at: Utc::now() + Duration::hours(1),
        }
    }
    let hashes: Vec<String> = (0..3)
        .map(|_| hash_secret(&generate_session_secret()))
        .collect();

    // The control first: everything but V100, so the flip is refused and the
    // transaction writes nothing.
    let refused = unbound_account(&h.backend).await;
    let result = control_backend
        .bind_near_ai_login(
            &refused.tenant,
            refused.account_id,
            &login,
            &Device::new().public_bytes(),
            session(&hashes[0]),
            &identity(),
        )
        .await;
    let error = format!("{:?}", result.expect_err("no UPDATE on the binding table"));
    assert!(error.contains("permission denied"), "{error}");
    assert_eq!(
        binding_row(&admin, &refused.tenant, refused.account_id).await,
        ("unbound".to_string(), false)
    );
    assert_eq!(linked_rows(&admin, &refused.tenant).await, [0; 4]);
    assert!(anchors_for(&admin, &h.anchor_hash()).await.is_empty());

    // The runtime role: bind in place ...
    let first = unbound_account(&h.backend).await;
    let outcome = runtime_backend
        .bind_near_ai_login(
            &first.tenant,
            first.account_id,
            &login,
            &Device::new().public_bytes(),
            session(&hashes[1]),
            &identity(),
        )
        .await
        .expect("bind as the runtime role");
    assert!(matches!(
        outcome,
        trace_commons_server::account_onboarding::NearAiBindOutcome::Bound(_)
    ));
    assert_eq!(
        binding_row(&admin, &first.tenant, first.account_id).await,
        ("bound".to_string(), true)
    );
    // ... and the existing-account branch, which closes the second account.
    let second = unbound_account(&h.backend).await;
    let outcome = runtime_backend
        .bind_near_ai_login(
            &second.tenant,
            second.account_id,
            &login,
            &Device::new().public_bytes(),
            session(&hashes[2]),
            &identity(),
        )
        .await
        .expect("refuse branch as the runtime role");
    assert!(matches!(
        outcome,
        trace_commons_server::account_onboarding::NearAiBindOutcome::ExistingAccount { .. }
    ));
    assert_eq!(
        binding_row(&admin, &second.tenant, second.account_id).await,
        ("closed".to_string(), false)
    );

    // V100 grants exactly (state, bound_at): not the table, not the key.
    for (column, expected) in [
        ("state", true),
        ("bound_at", true),
        ("origin", false),
        ("tenant_id", false),
        ("account_id", false),
        ("created_at", false),
    ] {
        let held: bool = admin
            .query_one(
                "SELECT has_column_privilege('trace_ingest_runtime',
                        'public.trace_account_bindings', $1, 'UPDATE')",
                &[&column],
            )
            .await
            .expect("privilege")
            .get(0);
        assert_eq!(held, expected, "UPDATE ({column})");
    }

    drop(runtime_backend);
    drop(control_backend);
    drop_tenants(&admin, &[&refused.tenant, &first.tenant, &second.tenant]).await;
    for role in [&runtime, &control] {
        admin
            .batch_execute(&format!("DROP OWNED BY {role}; DROP ROLE {role};"))
            .await
            .unwrap_or_else(|error| panic!("drop {role}: {error}"));
    }
}

/// A browser cookie session never reaches the bind ceremony: the device key
/// lives in the daemon, which holds a native token. Both verbs answer `403
/// native_session_required` before anything is read or written -- for a
/// strong `passkey` cookie and for a native session row carried in a cookie
/// alike -- and a ceremony the native token started is left for the native
/// token to finish.
#[tokio::test]
async fn pg_a_cookie_session_is_refused_by_both_bind_verbs() {
    use base64::Engine as _;
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    let account = unbound_account(&h.backend).await;
    let device = Device::new();

    let mut cookies = Vec::new();
    for client_kind in ["passkey", "native"] {
        let secret = generate_session_secret();
        admin
            .execute(
                "INSERT INTO trace_sessions
                    (tenant_id, session_id, account_id, token_hash, client_kind, expires_at)
                 VALUES ($1, $2, $3, $4, $5, now() + interval '1 hour')",
                &[
                    &account.tenant,
                    &Uuid::new_v4(),
                    &account.account_id,
                    &hash_secret(&secret),
                    &client_kind,
                ],
            )
            .await
            .expect("cookie session");
        cookies.push(format!(
            "{ACCOUNT_SESSION_COOKIE}={}.{secret}",
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(account.tenant.as_bytes())
        ));
    }

    // A ceremony started by the native token, for the finish attempts.
    let started = start(&h.state, BIND_START, Some(&account.token), &device).await;
    let signature = device.sign(&started.signing_bytes);
    let finish_body = serde_json::json!({
        "ceremony_id": started.ceremony_id,
        "code_verifier": started.verifier,
        "device_public_key": device.public_b64(),
        "device_signature": signature,
        "access_token": "near-ai-access-token-fixture",
    });
    let start_body = serde_json::json!({
        "device_public_key": device.public_b64(),
        "code_challenge": pkce().1,
        "code_challenge_method": "S256",
    });
    let started_audits = audit_metadata(&admin, &account.tenant, "account_binding_started")
        .await
        .len();

    for cookie in &cookies {
        for (uri, body) in [(BIND_START, &start_body), (BIND_FINISH, &finish_body)] {
            let request = axum::http::Request::builder()
                .method("POST")
                .uri(uri)
                .header(CONTENT_TYPE, "application/json")
                .header(axum::http::header::COOKIE, cookie)
                .body(Body::from(serde_json::to_vec(body).expect("json")))
                .expect("request");
            let response = app(h.state.clone())
                .oneshot(request)
                .await
                .expect("router is infallible");
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .expect("body");
            let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
            assert_eq!(status, StatusCode::FORBIDDEN, "{uri}: {body}");
            assert_eq!(body["error"], "native_session_required", "{uri}");
        }
    }
    assert_eq!(h.hits(), 0, "no cookie request spent a NEAR AI token");
    assert_eq!(
        audit_metadata(&admin, &account.tenant, "account_binding_started")
            .await
            .len(),
        started_audits,
        "no cookie request started a ceremony"
    );
    assert!(
        audit_metadata(&admin, &account.tenant, "account_binding_failed")
            .await
            .is_empty(),
        "no cookie request reached a ceremony"
    );
    assert_eq!(
        binding_row(&admin, &account.tenant, account.account_id).await,
        ("unbound".to_string(), false)
    );
    assert_eq!(linked_rows(&admin, &account.tenant).await, [0; 4]);

    // The cookie finishes consumed nothing: the native token still finishes
    // its own ceremony.
    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&account.token),
        &started,
        &device,
        &signature,
    )
    .await;
    assert_eq!(reply.status, StatusCode::OK, "{:?}", reply.json());
    assert_eq!(reply.json()["outcome"], "bound");

    drop_tenants(&admin, &[&account.tenant]).await;
}

/// Bind spends a NEAR AI token exactly as provisioning does, so it draws on
/// provisioning's per-address budget rather than a second one of its own: a
/// caller that has used up the NEAR AI provisioning budget cannot start or
/// finish a bind either, and no NEAR AI token is spent.
#[tokio::test]
async fn pg_bind_shares_the_near_ai_provisioning_rate_budget() {
    let Some(h) = harness().await else {
        return;
    };
    let admin = h.admin().await;
    reset_account_rate_limiter_for_test();
    let account = unbound_account(&h.backend).await;
    let device = Device::new();
    // A ceremony started while the budget was open.
    let started = start(&h.state, BIND_START, Some(&account.token), &device).await;
    let signature = device.sign(&started.signing_bytes);

    // Provisioning's per-address budget, used up. These requests carry no
    // forwarded address, so they all share the one fallback key.
    for action in ["near-ai-start", "near-ai-finish"] {
        while ACCOUNT_RATE_LIMITER.check(&format!("near-provision-{action}:xff-absent"), 30) {}
    }

    let reply = send(
        &h.state,
        "POST",
        BIND_START,
        Some(&serde_json::json!({
            "device_public_key": device.public_b64(),
            "code_challenge": pkce().1,
            "code_challenge_method": "S256",
        })),
        Some(&account.token),
    )
    .await;
    assert_uniform_deny(&reply, "bind start over the provisioning budget").await;

    let reply = finish(
        &h.state,
        BIND_FINISH,
        Some(&account.token),
        &started,
        &device,
        &signature,
    )
    .await;
    assert_uniform_deny(&reply, "bind finish over the provisioning budget").await;
    assert_eq!(h.hits(), 0, "no NEAR AI token was spent");
    assert_eq!(
        binding_row(&admin, &account.tenant, account.account_id).await,
        ("unbound".to_string(), false)
    );

    reset_account_rate_limiter_for_test();
    drop_tenants(&admin, &[&account.tenant]).await;
}
