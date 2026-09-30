// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The unbound-account gate (Z2 native passkey identity, slice S1).
//!
//! The in-memory tests run everywhere. They drive the account routes through
//! the real `account_auth_middleware`, built from the same route groups as
//! production, either on the production surface (for refusals, which never
//! reach a handler) or with a probe layer standing in for the handlers (to see
//! that a route was let through without running handlers against a stub
//! database). The PostgreSQL tests self-skip without
//! `TRACE_COMMONS_PG_TEST_DATABASE_URL` and run in the `ingest-bin-postgres`
//! CI job.

use super::*;
use crate::account_routes::{
    ACCOUNT_UNBOUND, RegisteredAccountRoute, UNBOUND_ACCOUNT_ROUTE_POLICY, UnboundAccess,
};
use axum::body::Body;
use std::collections::BTreeSet;
use tower::ServiceExt;
use trace_commons_server::account_binding::AccountBindingState;

fn gate_tenant() -> String {
    format!("nearai-{}", "b".repeat(64))
}

const GATE_SECRET: &str = "unbound-gate-test-secret";

/// One live session on `gate_tenant()` whose binding state every validation
/// answers with (`None` is legacy: no binding row).
fn gate_db(binding: Option<AccountBindingState>) -> (Arc<NativeAuthTestDb>, Uuid) {
    let db = Arc::new(NativeAuthTestDb::default());
    let account = Uuid::new_v4();
    db.insert_session(
        &gate_tenant(),
        account,
        &hash_secret(GATE_SECRET),
        NATIVE_SESSION_CLIENT_KIND,
        Utc::now() + Duration::days(1),
    );
    *db.binding.lock().unwrap() = binding;
    (db, account)
}

/// How the test session is presented: the native `tcn1_` bearer or the
/// browser cookie. Both must be gated identically.
#[derive(Clone, Copy, Debug)]
enum Transport {
    Native,
    Cookie,
}

/// A concrete URI for a path template: every `{param}` becomes a fixed value.
fn concrete_path(template: &str) -> String {
    template
        .split('/')
        .map(|segment| {
            if segment.starts_with('{') && segment.ends_with('}') {
                "00000000-0000-4000-8000-000000000000"
            } else {
                segment
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

fn gate_request(method: &str, path: &str, transport: Transport) -> axum::http::Request<Body> {
    let tenant = gate_tenant();
    let builder = axum::http::Request::builder()
        .method(method)
        .uri(concrete_path(path))
        .header(CONTENT_TYPE, "application/json")
        .header("sec-fetch-site", "same-origin");
    let builder = match transport {
        Transport::Native => builder.header(
            AUTHORIZATION,
            format!("Bearer {}", native_token_value(&tenant, GATE_SECRET)),
        ),
        Transport::Cookie => builder.header(
            axum::http::header::COOKIE,
            format!(
                "{ACCOUNT_SESSION_COOKIE}={}.{GATE_SECRET}",
                base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(tenant.as_bytes())
            ),
        ),
    };
    builder.body(Body::from("{}")).expect("request")
}

/// Stands in for every handler: answers 418 and reports the binding state the
/// middleware handed on. Reaching it means the gate let the request through.
async fn gate_probe(request: Request, _next: Next) -> axum::response::Response {
    let binding = request
        .extensions()
        .get::<AccountBindingState>()
        .map(|binding| binding.label())
        .unwrap_or("missing");
    (StatusCode::IM_A_TEAPOT, [("x-gate-probe", binding)]).into_response()
}

/// The production route groups, behind the real auth middleware, with the
/// probe in place of the handlers.
fn gate_probe_router(state: &Arc<AppState>) -> Router {
    let (general, rewards) = account_route_groups();
    general
        .merge(rewards)
        .route_layer_fn(gate_probe)
        .authenticated(state.clone())
        .into_router()
        .with_state(state.clone())
}

fn production_account_router(state: &Arc<AppState>) -> Router {
    authenticated_account_surface(state.clone())
        .into_router()
        .with_state(state.clone())
}

struct GateResponse {
    status: StatusCode,
    label: Option<String>,
    probe: Option<String>,
    headers: HeaderMap,
}

impl GateResponse {
    fn is_gate_refusal(&self) -> bool {
        self.status == StatusCode::FORBIDDEN && self.label.as_deref() == Some(ACCOUNT_UNBOUND)
    }
}

async fn send(router: Router, request: axum::http::Request<Body>) -> GateResponse {
    let response = router.oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let probe = headers
        .get("x-gate-probe")
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .expect("body");
    let label = serde_json::from_slice::<serde_json::Value>(&body)
        .ok()
        .and_then(|value| {
            value
                .get("error")
                .and_then(|e| e.as_str())
                .map(str::to_string)
        });
    GateResponse {
        status,
        label,
        probe,
        headers,
    }
}

fn all_routes(state: &Arc<AppState>) -> Vec<RegisteredAccountRoute> {
    authenticated_account_surface(state.clone())
        .registered()
        .to_vec()
}

fn access_of(route: &RegisteredAccountRoute) -> UnboundAccess {
    UNBOUND_ACCOUNT_ROUTE_POLICY
        .iter()
        .find(|(m, p, _)| *m == route.method && *p == route.path)
        .map(|(_, _, access)| *access)
        .expect("classified (see the classification test)")
}

fn gate_audits(db: &NativeAuthTestDb) -> Vec<(String, serde_json::Value)> {
    db.account_audits
        .lock()
        .unwrap()
        .iter()
        .filter(|(action, _, _)| action == "account_unbound_gate_denied")
        .map(|(_, actor, metadata)| (actor.clone(), metadata.clone()))
        .collect()
}

/// Every route on the authenticated account surface, read back from the
/// router definition itself, carries exactly one unbound classification, and
/// every classification names a route that exists. A route added without a
/// classification fails here.
#[test]
fn every_authenticated_account_route_is_classified_for_unbound_accounts() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());

    let registered = all_routes(&state);
    let registered_set: BTreeSet<(&str, &str)> =
        registered.iter().map(|r| (r.method, r.path)).collect();
    assert_eq!(
        registered_set.len(),
        registered.len(),
        "a route is registered twice"
    );
    assert!(
        registered_set.len() >= 30,
        "the surface record looks truncated: {} routes",
        registered_set.len()
    );

    let classified: Vec<(&str, &str)> = UNBOUND_ACCOUNT_ROUTE_POLICY
        .iter()
        .map(|(method, path, _)| (*method, *path))
        .collect();
    let classified_set: BTreeSet<(&str, &str)> = classified.iter().copied().collect();
    assert_eq!(
        classified_set.len(),
        classified.len(),
        "a route is classified twice"
    );

    let unclassified: Vec<_> = registered_set.difference(&classified_set).collect();
    assert!(
        unclassified.is_empty(),
        "account routes with no unbound-account classification (add each to \
         UNBOUND_ACCOUNT_ROUTE_POLICY as Allowed or Refused): {unclassified:?}"
    );
    let stale: Vec<_> = classified_set.difference(&registered_set).collect();
    assert!(
        stale.is_empty(),
        "UNBOUND_ACCOUNT_ROUTE_POLICY classifies routes that are not registered: {stale:?}"
    );
}

/// The allowlist is exactly the spec's, the bind routes (S3) included. Pinned
/// as literals so widening it is a visible diff to this test.
#[test]
fn the_unbound_allowlist_is_the_specs() {
    let allowed: BTreeSet<(&str, &str)> = UNBOUND_ACCOUNT_ROUTE_POLICY
        .iter()
        .filter(|(_, _, access)| *access == UnboundAccess::Allowed)
        .map(|(method, path, _)| (*method, *path))
        .collect();
    let expected: BTreeSet<(&str, &str)> = [
        ("GET", "/v1/account/binding"),
        ("GET", "/v1/account/contribution-status"),
        ("POST", "/v1/account/logout"),
        ("POST", "/v1/account/sessions/revoke-all"),
        ("GET", "/v1/account/passkeys"),
        ("PATCH", "/v1/account/passkeys/{credential_id}"),
        ("POST", "/v1/account/near-ai/provision/bind/start"),
        ("POST", "/v1/account/near-ai/provision/bind/finish"),
    ]
    .into_iter()
    .collect();
    assert_eq!(allowed, expected);
}

/// The spec's S1 test, on the production router: an unbound (or closed)
/// session, over either transport, gets `403 account_unbound` on every route
/// not on the allowlist, and each refusal writes one label-only audit row.
#[tokio::test]
async fn the_production_surface_refuses_an_unbound_session_off_the_allowlist() {
    for transport in [Transport::Native, Transport::Cookie] {
        for binding in [AccountBindingState::Unbound, AccountBindingState::Closed] {
            let (db, account) = gate_db(Some(binding));
            let (_temp, state) = native_test_state(db.clone());
            let mut refused = 0;
            for route in all_routes(&state) {
                if access_of(&route) != UnboundAccess::Refused {
                    continue;
                }
                refused += 1;
                let response = send(
                    production_account_router(&state),
                    gate_request(route.method, route.path, transport),
                )
                .await;
                assert!(
                    response.is_gate_refusal(),
                    "{transport:?} {binding:?} {} {} must be refused, got {} {:?}",
                    route.method,
                    route.path,
                    response.status,
                    response.label
                );
                assert_eq!(
                    response
                        .headers
                        .get(axum::http::header::CACHE_CONTROL)
                        .and_then(|v| v.to_str().ok()),
                    Some("no-store")
                );
            }
            assert!(refused >= 25, "{refused}");
            let audits = gate_audits(&db);
            assert_eq!(audits.len(), refused, "{transport:?} {binding:?}");
            for (actor, metadata) in audits {
                assert_eq!(actor, format!("account-actor:{account}"));
                assert_eq!(metadata, serde_json::json!({}));
            }
        }
    }
}

/// With the handlers replaced by a probe: an unbound session reaches exactly
/// the allowlist, and the binding state reaches the handler.
#[tokio::test]
async fn an_unbound_session_reaches_exactly_the_allowlist() {
    for transport in [Transport::Native, Transport::Cookie] {
        for binding in [AccountBindingState::Unbound, AccountBindingState::Closed] {
            let (db, _account) = gate_db(Some(binding));
            let (_temp, state) = native_test_state(db);
            for route in all_routes(&state) {
                let response = send(
                    gate_probe_router(&state),
                    gate_request(route.method, route.path, transport),
                )
                .await;
                match access_of(&route) {
                    UnboundAccess::Allowed => assert_eq!(
                        response.probe.as_deref(),
                        Some(binding.label()),
                        "{transport:?} {binding:?} {} {} is on the allowlist, got {} {:?}",
                        route.method,
                        route.path,
                        response.status,
                        response.label
                    ),
                    UnboundAccess::Refused => {
                        assert!(
                            response.is_gate_refusal(),
                            "{} {}",
                            route.method,
                            route.path
                        );
                        assert_eq!(response.probe, None);
                    }
                }
            }
        }
    }
}

/// Absence of a binding row is legacy: a legacy account, like a bound one,
/// reaches every route as before, and the gate writes nothing.
#[tokio::test]
async fn legacy_and_bound_sessions_reach_every_route() {
    for transport in [Transport::Native, Transport::Cookie] {
        for binding in [None, Some(AccountBindingState::Bound)] {
            let (db, _account) = gate_db(binding);
            let (_temp, state) = native_test_state(db.clone());
            let expected = binding.unwrap_or(AccountBindingState::Legacy).label();
            for route in all_routes(&state) {
                let response = send(
                    gate_probe_router(&state),
                    gate_request(route.method, route.path, transport),
                )
                .await;
                assert_eq!(
                    response.probe.as_deref(),
                    Some(expected),
                    "{transport:?} {binding:?} {} {} was not let through: {} {:?}",
                    route.method,
                    route.path,
                    response.status,
                    response.label
                );
            }
            assert!(gate_audits(&db).is_empty());
        }
    }
}

/// A failure reading the binding state refuses every route, allowlisted or
/// not: there is no state to decide on, so nothing is reachable.
#[tokio::test]
async fn a_failed_binding_read_refuses_every_route() {
    let (db, _account) = gate_db(Some(AccountBindingState::Unbound));
    db.binding_read_fails
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let (_temp, state) = native_test_state(db);
    for route in all_routes(&state) {
        let response = send(
            gate_probe_router(&state),
            gate_request(route.method, route.path, Transport::Native),
        )
        .await;
        assert_eq!(
            response.status,
            StatusCode::INTERNAL_SERVER_ERROR,
            "{} {}",
            route.method,
            route.path
        );
        assert_eq!(response.probe, None);
    }
}

/// A refused request still hands back a rotated native token. The rotation
/// already happened in the database, so dropping the new token would sign the
/// app out after the grace window.
#[tokio::test]
async fn a_gate_refusal_still_returns_the_rotated_native_token() {
    let (db, _account) = gate_db(Some(AccountBindingState::Unbound));
    *db.rotate_to.lock().unwrap() = Some("rotated-gate-secret".to_string());
    let (_temp, state) = native_test_state(db);
    let response = send(
        production_account_router(&state),
        gate_request("GET", "/v1/account/traces", Transport::Native),
    )
    .await;
    assert!(response.is_gate_refusal());
    assert_eq!(
        response
            .headers
            .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some(native_token_value(&gate_tenant(), "rotated-gate-secret").as_str())
    );
}

/// And a refused cookie request still carries the rotated cookie.
#[tokio::test]
async fn a_gate_refusal_still_returns_the_rotated_cookie() {
    let (db, _account) = gate_db(Some(AccountBindingState::Unbound));
    *db.rotate_to.lock().unwrap() = Some("rotated-gate-secret".to_string());
    let (_temp, state) = native_test_state(db);
    let response = send(
        production_account_router(&state),
        gate_request("GET", "/v1/account/traces", Transport::Cookie),
    )
    .await;
    assert!(response.is_gate_refusal());
    let set_cookie = response
        .headers
        .get(axum::http::header::SET_COOKIE)
        .and_then(|v| v.to_str().ok())
        .expect("rotated cookie attached");
    assert!(
        set_cookie.contains("rotated-gate-secret"),
        "the rotated secret is what the browser must swap to"
    );
    assert_set_cookies_are_host_bound(&response.headers);
    assert_clears_legacy_session_cookie(&response.headers);
}

#[tokio::test]
async fn the_binding_route_reports_the_callers_state() {
    for (binding, label) in [
        (None, "legacy"),
        (Some(AccountBindingState::Unbound), "unbound"),
        (Some(AccountBindingState::Bound), "bound"),
        (Some(AccountBindingState::Closed), "closed"),
    ] {
        let (db, _account) = gate_db(binding);
        let (_temp, state) = native_test_state(db);
        let response = app(state.clone())
            .oneshot(gate_request(
                "GET",
                "/v1/account/binding",
                Transport::Native,
            ))
            .await
            .expect("router");
        assert_eq!(response.status(), StatusCode::OK, "{label}");
        assert_eq!(
            response
                .headers()
                .get(axum::http::header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let body = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("body");
        let value: serde_json::Value = serde_json::from_slice(&body).expect("json");
        assert_eq!(value, serde_json::json!({ "binding_state": label }));
    }
}

#[tokio::test]
async fn the_binding_route_requires_a_session() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let response = app(state)
        .oneshot(
            axum::http::Request::builder()
                .method("GET")
                .uri("/v1/account/binding")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("router");
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

/// The spec's contribution-status answer for an unbound account, on the
/// production router.
#[tokio::test]
async fn contribution_status_answers_identity_unlinked_for_an_unbound_account() {
    let (db, _account) = gate_db(Some(AccountBindingState::Unbound));
    let (_temp, state) = native_test_state(db);
    let response = send(
        production_account_router(&state),
        gate_request("GET", "/v1/account/contribution-status", Transport::Native),
    )
    .await;
    assert_eq!(response.status, StatusCode::FORBIDDEN);
    assert_eq!(response.label.as_deref(), Some("account_identity_unlinked"));
}

// --- PostgreSQL -----------------------------------------------------------

async fn pg_gate_fixture_tenant(admin: &deadpool_postgres::Object) -> String {
    let tenant = format!("nearai-{}", Uuid::new_v4().simple().to_string().repeat(2));
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .expect("tenant");
    tenant
}

async fn pg_insert_native_session(
    admin: &deadpool_postgres::Object,
    tenant: &str,
    account: Uuid,
    secret: &str,
) {
    admin
        .execute(
            "INSERT INTO trace_sessions
                (tenant_id, session_id, account_id, token_hash, client_kind, expires_at)
             VALUES ($1, $2, $3, $4, 'native', now() + interval '1 day')",
            &[&tenant, &Uuid::new_v4(), &account, &hash_secret(secret)],
        )
        .await
        .expect("session");
}

/// Create a passkey-origin account through the S2 helper, in a tenant
/// transaction, and commit.
async fn pg_passkey_account(backend: &PgBackend, tenant: &str) -> Uuid {
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("client");
    let tx = client.transaction().await.expect("tx");
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .expect("tenant context");
    let account = Uuid::new_v4();
    PgBackend::insert_passkey_origin_account(&tx, account)
        .await
        .expect("insert passkey account");
    tx.commit().await.expect("commit");
    account
}

#[tokio::test]
async fn pg_account_bindings_are_forced_rls_and_select_insert_only_to_the_runtime() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    let row = admin
        .query_one(
            "SELECT relrowsecurity, relforcerowsecurity FROM pg_class
              WHERE oid = 'public.trace_account_bindings'::regclass",
            &[],
        )
        .await
        .expect("table exists");
    assert!(row.get::<_, bool>(0), "RLS enabled");
    assert!(row.get::<_, bool>(1), "RLS forced");
    let policy: String = admin
        .query_one(
            "SELECT pg_get_expr(polqual, polrelid) FROM pg_policy
              WHERE polrelid = 'public.trace_account_bindings'::regclass
                AND polname = 'trace_corpus_tenant_isolation'",
            &[],
        )
        .await
        .expect("tenant policy")
        .get(0);
    assert!(policy.contains("trace_current_tenant_id()"), "{policy}");

    // S1 (V97) granted SELECT; S2 (V98) grants INSERT for native passkey
    // create/finish. Flipping a row to bound is S3's grant, and a row is only
    // ever removed by the cascade from its account.
    for (privilege, expected) in [
        ("SELECT", true),
        ("INSERT", true),
        ("UPDATE", false),
        ("DELETE", false),
        ("TRUNCATE", false),
    ] {
        let held: bool = admin
            .query_one(
                "SELECT has_table_privilege('trace_ingest_runtime',
                        'public.trace_account_bindings', $1)",
                &[&privilege],
            )
            .await
            .expect("privilege")
            .get(0);
        assert_eq!(held, expected, "trace_ingest_runtime {privilege}");
    }
}

/// `validate_session` reads the binding row in the same query: no row is
/// legacy, and the stored state is what the gate sees.
#[tokio::test]
async fn pg_validate_session_carries_the_binding_state() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    let tenant = pg_gate_fixture_tenant(&admin).await;

    let legacy = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &legacy],
        )
        .await
        .expect("legacy account");
    pg_insert_native_session(&admin, &tenant, legacy, &format!("pg-gate-legacy-{tenant}")).await;
    let passkey = pg_passkey_account(&backend, &tenant).await;
    pg_insert_native_session(
        &admin,
        &tenant,
        passkey,
        &format!("pg-gate-passkey-{tenant}"),
    )
    .await;

    let session = backend
        .validate_session(&tenant, &hash_secret(&format!("pg-gate-legacy-{tenant}")))
        .await
        .expect("validate")
        .expect("live");
    assert_eq!(session.binding, AccountBindingState::Legacy);

    let session = backend
        .validate_session(&tenant, &hash_secret(&format!("pg-gate-passkey-{tenant}")))
        .await
        .expect("validate")
        .expect("live");
    assert_eq!(session.account_id, passkey);
    assert_eq!(session.binding, AccountBindingState::Unbound);

    admin
        .execute(
            "UPDATE trace_account_bindings SET state = 'bound', bound_at = now()
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &passkey],
        )
        .await
        .expect("bind");
    let session = backend
        .validate_session(&tenant, &hash_secret(&format!("pg-gate-passkey-{tenant}")))
        .await
        .expect("validate")
        .expect("live");
    assert_eq!(session.binding, AccountBindingState::Bound);

    // The table's CHECKs hold the state and its timestamp together.
    let err = admin
        .execute(
            "UPDATE trace_account_bindings SET state = 'unbound'
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &passkey],
        )
        .await
        .expect_err("unbound with a bound_at is refused");
    assert_eq!(
        err.as_db_error().map(|db| db.code()),
        Some(&tokio_postgres::error::SqlState::CHECK_VIOLATION),
        "{err:?}"
    );

    admin
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("cleanup");
}

/// The helper S2 will call writes the account and its binding row together
/// or not at all, so an absent row can never mean a passkey account.
#[tokio::test]
async fn pg_a_passkey_account_is_never_written_without_its_binding() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    let tenant = pg_gate_fixture_tenant(&admin).await;
    let counts = |account: Uuid| {
        let admin = &admin;
        async move {
            let row = admin
                .query_one(
                    "SELECT (SELECT count(*) FROM trace_accounts WHERE account_id = $1),
                            (SELECT count(*) FROM trace_account_bindings WHERE account_id = $1)",
                    &[&account],
                )
                .await
                .expect("counts");
            (row.get::<_, i64>(0), row.get::<_, i64>(1))
        }
    };

    // Rolled back: neither row.
    let mut client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("client");
    let account = Uuid::new_v4();
    {
        let tx = client.transaction().await.expect("tx");
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await
        .expect("tenant context");
        PgBackend::insert_passkey_origin_account(&tx, account)
            .await
            .expect("insert");
        tx.rollback().await.expect("rollback");
    }
    assert_eq!(counts(account).await, (0, 0));

    // No tenant context: refused, nothing written.
    {
        let tx = client.transaction().await.expect("tx");
        PgBackend::insert_passkey_origin_account(&tx, account)
            .await
            .expect_err("no tenant context");
    }
    assert_eq!(counts(account).await, (0, 0));

    // Committed: both, in the same tenant, unbound.
    let account = pg_passkey_account(&backend, &tenant).await;
    assert_eq!(counts(account).await, (1, 1));
    let row = admin
        .query_one(
            "SELECT b.tenant_id, b.origin, b.state, b.bound_at IS NULL
               FROM trace_account_bindings b
               JOIN trace_accounts a USING (tenant_id, account_id)
              WHERE b.account_id = $1",
            &[&account],
        )
        .await
        .expect("binding");
    assert_eq!(row.get::<_, String>(0), tenant);
    assert_eq!(row.get::<_, String>(1), "passkey");
    assert_eq!(row.get::<_, String>(2), "unbound");
    assert!(row.get::<_, bool>(3));

    // The binding row goes with its account.
    admin
        .execute(
            "DELETE FROM trace_accounts WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account],
        )
        .await
        .expect("delete account");
    assert_eq!(counts(account).await, (0, 0));

    admin
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("cleanup");
}

/// End to end on PostgreSQL: an unbound account's native session is gated by
/// the real router and sees its state on the binding route; a legacy account
/// in the same tenant is not gated.
#[tokio::test]
async fn pg_the_real_router_gates_an_unbound_native_session() {
    let Some(backend) = postgres_backend_for_ingest_test().await else {
        return;
    };
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    let tenant = pg_gate_fixture_tenant(&admin).await;
    let passkey = pg_passkey_account(&backend, &tenant).await;
    pg_insert_native_session(
        &admin,
        &tenant,
        passkey,
        &format!("pg-route-passkey-{tenant}"),
    )
    .await;
    let legacy = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &legacy],
        )
        .await
        .expect("legacy account");
    pg_insert_native_session(
        &admin,
        &tenant,
        legacy,
        &format!("pg-route-legacy-{tenant}"),
    )
    .await;

    let temp = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = backend.clone();
    let state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    let request = |method: &str, uri: &str, secret: &str| {
        axum::http::Request::builder()
            .method(method)
            .uri(uri)
            .header(
                AUTHORIZATION,
                format!("Bearer {}", native_token_value(&tenant, secret)),
            )
            .body(Body::empty())
            .unwrap()
    };
    let read = |response: axum::response::Response| async move {
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice::<serde_json::Value>(&body).unwrap_or_default(),
        )
    };

    let (status, body) = read(
        app(state.clone())
            .oneshot(request(
                "GET",
                "/v1/account/traces",
                &format!("pg-route-passkey-{tenant}"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(body["error"], ACCOUNT_UNBOUND);

    let (status, body) = read(
        app(state.clone())
            .oneshot(request(
                "GET",
                "/v1/account/binding",
                &format!("pg-route-passkey-{tenant}"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "binding_state": "unbound" }));

    let audits: i64 = admin
        .query_one(
            "SELECT count(*) FROM trace_account_audit
              WHERE tenant_id = $1 AND action = 'account_unbound_gate_denied'
                AND actor_ref = $2 AND safe_metadata = '{}'::jsonb",
            &[&tenant, &format!("account-actor:{passkey}")],
        )
        .await
        .expect("audit")
        .get(0);
    assert_eq!(audits, 1);

    let (status, body) = read(
        app(state.clone())
            .oneshot(request(
                "GET",
                "/v1/account/binding",
                &format!("pg-route-legacy-{tenant}"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, serde_json::json!({ "binding_state": "legacy" }));
    let (status, body) = read(
        app(state.clone())
            .oneshot(request(
                "GET",
                "/v1/account/traces",
                &format!("pg-route-legacy-{tenant}"),
            ))
            .await
            .unwrap(),
    )
    .await;
    assert_ne!(body["error"], ACCOUNT_UNBOUND, "{status}");

    admin
        .execute("DELETE FROM trace_tenants WHERE tenant_id = $1", &[&tenant])
        .await
        .expect("cleanup");
}
