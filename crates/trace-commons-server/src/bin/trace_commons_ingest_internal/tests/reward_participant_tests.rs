// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! HTTP acceptance coverage for participant reward offers and reservations.
//! INTEGRATION: `tests.rs` includes this module after the reward routes are wired.

use crate::tests::*;

// Mission HTTP/PG coverage stays nested so it can reuse this suite's private
// real-router reward fixture without widening or splitting inherited helpers.
#[path = "mission_catalog_pg.rs"]
mod mission_catalog_pg;

use axum::body::Body;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE, COOKIE};
use axum::http::{HeaderMap, Request, StatusCode};
use chrono::{Duration, Utc};
use secrecy::SecretString;
use serde_json::{Value, json};
use sha2::Digest;
use tokio::net::TcpListener;
use tokio::task::JoinHandle;
use tower::ServiceExt;
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::Database;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::mission_rewards::{RewardActivityKind, RewardProgramTerms};
use trace_commons_server::reward_participant::RewardOfferManifest;
use uuid::Uuid;

const REWARD_PG_URL: &str = "TRACE_COMMONS_REWARDS_PG_TEST_URL";
const TENANT: &str = "tenant-a";

/// This is intentionally ignored in the normal suite: it is a real TCP + PostgreSQL
/// acceptance run. Unlike the older optional PostgreSQL tests, selecting it without
/// the dedicated loopback database is a failure, so a missing integration cannot
/// register as coverage.
#[tokio::test]
#[ignore = "requires isolated TRACE_COMMONS_REWARDS_PG_TEST_URL loopback reward_test_* database"]
async fn reward_participant_http_contract() {
    let fixture = RewardHttpFixture::start().await;
    public_offer_and_cookie_reservation_scenario(&fixture).await;
    reservation_status_and_history_scenario(&fixture).await;
    account_isolation_and_cursor_scenario(&fixture).await;
    replay_and_reservation_input_scenario(&fixture).await;
    request_parsing_and_account_auth_scenario(&fixture).await;
    device_and_native_session_scenario(&fixture).await;
    suspended_offer_scenario(&fixture).await;
    blocked_reservation_preserves_query_capacity_scenario(&fixture).await;
    assert_no_participant_side_effects(fixture.backends.admin.as_ref(), fixture.reservation_id)
        .await;
}

struct RewardHttpFixture {
    _temp: tempfile::TempDir,
    state: Arc<AppState>,
    base: String,
    server: JoinHandle<()>,
    client: reqwest::Client,
    backends: RewardTestBackends,
    cookie: String,
    offer: trace_commons_server::reward_participant::RewardOffer,
    reservation_id: Uuid,
    reservation_body: Value,
}

impl RewardHttpFixture {
    async fn start() -> Self {
        let backends = reward_backends().await;
        let temp = tempfile::tempdir().expect("temp dir");
        let database: Arc<dyn Database> = backends.participant.clone();
        let state = test_state_with_options(
            temp.path().to_path_buf(),
            Some(database),
            None,
            false,
            false,
            false,
            false,
        );
        let (base, server) = serve_loopback(state.clone()).await;

        provision_reward_tenant(backends.admin.as_ref(), TENANT).await;
        provision_participant_login(backends.admin.as_ref(), TENANT).await;
        let offer = publish_offer(backends.issuer.as_ref(), Uuid::new_v4()).await;
        let cookie = mint_redeem_session_cookie_value(&state, "token-a").await;
        let reservation_id = Uuid::new_v4();
        let reservation_body = json!({
            "reservation_id": reservation_id,
            "offer_version_hash": offer.offer_version_hash,
        });

        Self {
            _temp: temp,
            state,
            base,
            server,
            client: reqwest::Client::new(),
            backends,
            cookie,
            offer,
            reservation_id,
            reservation_body,
        }
    }

    fn cookie_header(&self) -> String {
        format!("tc_account_session={}", self.cookie)
    }
}

impl Drop for RewardHttpFixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn public_offer_and_cookie_reservation_scenario(fixture: &RewardHttpFixture) {
    let offer = &fixture.offer;
    let public = fixture
        .client
        .get(format!(
            "{}/v1/reward-offers/{}",
            fixture.base, offer.program_id
        ))
        .send()
        .await
        .expect("public offer request");
    assert_eq!(public.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(public.headers());
    let public: Value = public.json().await.expect("public offer JSON");
    assert_eq!(public["program_id"], json!(offer.program_id));
    assert_eq!(public["award_units"], json!("7"));
    assert_eq!(public["capacity_remaining_units"], json!("70"));
    assert_eq!(public["manifest"]["schema_version"], json!(1));

    let reservation = fixture
        .client
        .post(format!(
            "{}/v1/account/reward-offers/{}/reservations",
            fixture.base, offer.program_id
        ))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .json(&fixture.reservation_body)
        .send()
        .await
        .expect("reservation request");
    assert_eq!(reservation.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(reservation.headers());
    let reservation: Value = reservation.json().await.expect("reservation JSON");
    assert_eq!(reservation["reservation_id"], json!(fixture.reservation_id));
    assert_eq!(reservation["state"], json!("reserved"));
    assert_eq!(reservation["pinned_units"], json!("7"));
    assert_eq!(reservation["awarded_units"], json!("0"));
    assert!(reservation.get("account_id").is_none());
    assert!(reservation.get("participant_hash").is_none());
    assert!(reservation.get("evidence_hash").is_none());
}

async fn reservation_status_and_history_scenario(fixture: &RewardHttpFixture) {
    let status = fixture
        .client
        .get(format!(
            "{}/v1/account/reward-reservations/{}",
            fixture.base, fixture.reservation_id
        ))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .send()
        .await
        .expect("reservation status request");
    assert_eq!(status.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(status.headers());
    assert_eq!(
        status.json::<Value>().await.expect("status JSON")["reservation_id"],
        json!(fixture.reservation_id)
    );

    let history = fixture
        .client
        .get(format!("{}/v1/account/rewards?limit=1", fixture.base))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .send()
        .await
        .expect("reward history request");
    assert_eq!(history.status(), reqwest::StatusCode::OK);
    let history: Value = history.json().await.expect("history JSON");
    assert_eq!(history["limit"], json!(1));
    assert_eq!(history["awarded_units"], json!("0"));
    assert_eq!(
        history["entries"][0]["reservation_id"],
        json!(fixture.reservation_id)
    );
    assert!(history["next_cursor"].is_null());
}

async fn account_isolation_and_cursor_scenario(fixture: &RewardHttpFixture) {
    // A separate account cannot read another account's reservation. It also has
    // its own empty history despite being in the same running server process.
    provision_reward_tenant(fixture.backends.admin.as_ref(), "tenant-b").await;
    let other_cookie = mint_redeem_session_cookie_value(&fixture.state, "token-b").await;
    provision_participant_login(fixture.backends.admin.as_ref(), "tenant-b").await;
    assert_http(
        fixture
            .client
            .get(format!(
                "{}/v1/account/reward-reservations/{}",
                fixture.base, fixture.reservation_id
            ))
            .header(
                COOKIE.as_str(),
                format!("tc_account_session={other_cookie}"),
            )
            .send(),
        StatusCode::NOT_FOUND,
    )
    .await;

    let cursor_offer = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    reserve_with_header(
        &fixture.client,
        &fixture.base,
        cursor_offer.program_id,
        cursor_offer.offer_version_hash.as_str(),
        COOKIE.as_str(),
        &fixture.cookie_header(),
    )
    .await;
    let first_page = fixture
        .client
        .get(format!("{}/v1/account/rewards?limit=1", fixture.base))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .send()
        .await
        .expect("first history page");
    assert_eq!(first_page.status(), reqwest::StatusCode::OK);
    let first_page: Value = first_page.json().await.expect("first history JSON");
    let cursor = first_page["next_cursor"]
        .as_str()
        .expect("second reservation produces a keyset cursor");
    assert_http(
        fixture
            .client
            .get(format!(
                "{}/v1/account/rewards?limit=1&before={cursor}",
                fixture.base
            ))
            .header(COOKIE.as_str(), fixture.cookie_header())
            .send(),
        StatusCode::OK,
    )
    .await;
    let other_history = fixture
        .client
        .get(format!("{}/v1/account/rewards?limit=1", fixture.base))
        .header(
            COOKIE.as_str(),
            format!("tc_account_session={other_cookie}"),
        )
        .send()
        .await
        .expect("other account history");
    assert_eq!(other_history.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(other_history.headers());
    assert!(
        other_history
            .json::<Value>()
            .await
            .expect("other history JSON")["entries"]
            .as_array()
            .expect("entries array")
            .is_empty()
    );
    let foreign_cursor = fixture
        .client
        .get(format!(
            "{}/v1/account/rewards?limit=1&before={cursor}",
            fixture.base
        ))
        .header(
            COOKIE.as_str(),
            format!("tc_account_session={other_cookie}"),
        )
        .send()
        .await
        .expect("foreign cursor request");
    assert_eq!(foreign_cursor.status(), reqwest::StatusCode::NOT_FOUND);
    assert_safe_reqwest_headers(foreign_cursor.headers());
}

async fn replay_and_reservation_input_scenario(fixture: &RewardHttpFixture) {
    // Idempotency is exact-payload replay only; changing the offer hash under the
    // same reservation id must remain a conflict instead of reserving twice.
    assert_http(
        fixture
            .client
            .post(format!(
                "{}/v1/account/reward-offers/{}/reservations",
                fixture.base, fixture.offer.program_id
            ))
            .header(COOKIE.as_str(), fixture.cookie_header())
            .json(&fixture.reservation_body)
            .send(),
        StatusCode::OK,
    )
    .await;
    assert_http(
        fixture
            .client
            .post(format!(
                "{}/v1/account/reward-offers/{}/reservations",
                fixture.base, fixture.offer.program_id
            ))
            .header(COOKIE.as_str(), fixture.cookie_header())
            .json(&json!({
                "reservation_id": fixture.reservation_id,
                "offer_version_hash": format!("sha256:{}", "b".repeat(64)),
            }))
            .send(),
        StatusCode::CONFLICT,
    )
    .await;

    // The participant endpoints accept only the compact reservation DTO. Neither
    // a caller-controlled account nor a caller-controlled participant identity is
    // meaningful at this boundary.
    for injected in ["account_id", "participant_hash"] {
        let mut body = fixture.reservation_body.clone();
        body[injected] = json!(Uuid::new_v4());
        assert_router_status(
            &fixture.state,
            Request::builder()
                .method("POST")
                .uri(format!(
                    "/v1/account/reward-offers/{}/reservations",
                    fixture.offer.program_id
                ))
                .header(COOKIE, fixture.cookie_header())
                .header(CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request"),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }

    let oversized = format!(
        "{{\"reservation_id\":\"{}\",\"offer_version_hash\":\"sha256:{}\"}}{}",
        Uuid::new_v4(),
        "a".repeat(64),
        " ".repeat(1025)
    );
    assert_router_status(
        &fixture.state,
        Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/account/reward-offers/{}/reservations",
                fixture.offer.program_id
            ))
            .header(COOKIE, fixture.cookie_header())
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from(oversized))
            .expect("request"),
        StatusCode::PAYLOAD_TOO_LARGE,
    )
    .await;
    assert_router_status(
        &fixture.state,
        Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/account/reward-offers/{}/reservations",
                fixture.offer.program_id
            ))
            .header(COOKIE, fixture.cookie_header())
            .header(CONTENT_TYPE, "application/json")
            .header("sec-fetch-site", "cross-site")
            .body(Body::from(
                json!({
                    "reservation_id": Uuid::new_v4(),
                    "offer_version_hash": fixture.offer.offer_version_hash,
                })
                .to_string(),
            ))
            .expect("request"),
        StatusCode::FORBIDDEN,
    )
    .await;
    assert_router_status(
        &fixture.state,
        Request::builder()
            .method("POST")
            .uri(format!(
                "/v1/account/reward-offers/{}/reservations",
                fixture.offer.program_id
            ))
            .header(COOKIE, fixture.cookie_header())
            .header(CONTENT_TYPE, "application/json")
            .body(Body::from("{invalid"))
            .expect("request"),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

async fn request_parsing_and_account_auth_scenario(fixture: &RewardHttpFixture) {
    for uri in [
        "/v1/reward-offers/not-a-uuid",
        "/v1/account/rewards?limit=101",
        "/v1/account/rewards?limit=1&unknown=true",
    ] {
        let mut request = Request::builder().method("GET").uri(uri);
        if request
            .headers_mut()
            .expect("headers")
            .get(COOKIE)
            .is_none()
        {
            request = request.header(COOKIE, fixture.cookie_header());
        }
        assert_router_status(
            &fixture.state,
            request.body(Body::empty()).expect("request"),
            StatusCode::BAD_REQUEST,
        )
        .await;
    }

    // No identity may be inferred from a missing, invalid, expired, or ambiguous
    // account credential. Device upload credentials never authorize accounts.
    for request in [
        Request::builder()
            .method("GET")
            .uri("/v1/account/rewards")
            .body(Body::empty())
            .expect("request"),
        Request::builder()
            .method("GET")
            .uri("/v1/account/rewards")
            .header(AUTHORIZATION, "Bearer invalid")
            .body(Body::empty())
            .expect("request"),
    ] {
        assert_router_status(&fixture.state, request, StatusCode::UNAUTHORIZED).await;
    }
    assert_router_status(
        &fixture.state,
        Request::builder()
            .method("GET")
            .uri("/v1/account/rewards")
            .header(AUTHORIZATION, "Bearer token-a")
            .header(COOKIE, fixture.cookie_header())
            .body(Body::empty())
            .expect("request"),
        StatusCode::BAD_REQUEST,
    )
    .await;
}

async fn device_and_native_session_scenario(fixture: &RewardHttpFixture) {
    // Device upload claims remain excluded from every account route, including
    // participant rewards. A native account session can reserve without
    // widening device credentials.
    let device_offer = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    assert_http(
        fixture
            .client
            .post(format!(
                "{}/v1/account/reward-offers/{}/reservations",
                fixture.base, device_offer.program_id
            ))
            .header(AUTHORIZATION.as_str(), "Bearer token-a")
            .json(&json!({
                "reservation_id": Uuid::new_v4(),
                "offer_version_hash": device_offer.offer_version_hash,
            }))
            .send(),
        StatusCode::UNAUTHORIZED,
    )
    .await;
    let native_offer = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    let native = native_account_token(&fixture.state).await;
    reserve_with_header(
        &fixture.client,
        &fixture.base,
        native_offer.program_id,
        native_offer.offer_version_hash.as_str(),
        AUTHORIZATION.as_str(),
        &format!("Bearer {native}"),
    )
    .await;

    // A valid-looking but expired native session is refused by the same
    // production session validation used by the ordinary account surface.
    expire_native_token(fixture.backends.admin.as_ref(), &native).await;
    assert_http(
        fixture
            .client
            .get(format!("{}/v1/account/rewards", fixture.base))
            .header(AUTHORIZATION.as_str(), format!("Bearer {native}"))
            .send(),
        StatusCode::UNAUTHORIZED,
    )
    .await;
}

async fn suspended_offer_scenario(fixture: &RewardHttpFixture) {
    let suspended = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    fixture
        .backends
        .issuer
        .reward_offer_suspend(TENANT, suspended.program_id, true)
        .await
        .expect("suspend offer");
    assert_http(
        fixture
            .client
            .post(format!(
                "{}/v1/account/reward-offers/{}/reservations",
                fixture.base, suspended.program_id
            ))
            .header(COOKIE.as_str(), fixture.cookie_header())
            .json(&json!({
                "reservation_id": Uuid::new_v4(),
                "offer_version_hash": suspended.offer_version_hash,
            }))
            .send(),
        StatusCode::CONFLICT,
    )
    .await;
}

async fn blocked_reservation_preserves_query_capacity_scenario(fixture: &RewardHttpFixture) {
    let offer = publish_offer(fixture.backends.issuer.as_ref(), Uuid::new_v4()).await;
    let mut admin = fixture
        .backends
        .admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .unwrap();
    let tx = admin.transaction().await.unwrap();
    tx.execute(
        "SELECT pg_advisory_xact_lock(hashtextextended($1,691))",
        &[&TENANT],
    )
    .await
    .unwrap();
    let request = fixture
        .client
        .post(format!(
            "{}/v1/account/reward-offers/{}/reservations",
            fixture.base, offer.program_id
        ))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .json(
            &json!({"reservation_id":Uuid::new_v4(),"offer_version_hash":offer.offer_version_hash}),
        );
    let pending = tokio::spawn(request.send());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let waiting: bool = tx.query_one("SELECT EXISTS(SELECT 1 FROM pg_locks WHERE locktype='advisory' AND NOT granted AND database=(SELECT oid FROM pg_database WHERE datname=current_database()))", &[]).await.unwrap().get(0);
            if waiting { break; }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    }).await.expect("first reservation reaches its database lock");
    let repeated = tokio::time::timeout(std::time::Duration::from_secs(2), fixture.client
        .post(format!("{}/v1/account/reward-offers/{}/reservations", fixture.base, offer.program_id))
        .header(COOKIE.as_str(), fixture.cookie_header())
        .json(&json!({"reservation_id":Uuid::new_v4(),"offer_version_hash":offer.offer_version_hash}))
        .send()).await.expect("same-tenant contention is refused before waiting on the pool").unwrap();
    assert_eq!(repeated.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
    assert_safe_reqwest_headers(repeated.headers());
    let public = tokio::time::timeout(
        std::time::Duration::from_secs(2),
        fixture
            .client
            .get(format!(
                "{}/v1/reward-offers/{}",
                fixture.base, offer.program_id
            ))
            .send(),
    )
    .await
    .expect("public read retains database capacity")
    .unwrap();
    assert_eq!(public.status(), reqwest::StatusCode::OK);
    tx.rollback().await.unwrap();
    assert_eq!(
        pending.await.unwrap().unwrap().status(),
        reqwest::StatusCode::OK
    );
}

struct RewardTestBackends {
    admin: Arc<PgBackend>,
    issuer: Arc<PgBackend>,
    participant: Arc<PgBackend>,
}

async fn reward_backends() -> RewardTestBackends {
    let url = std::env::var(REWARD_PG_URL).unwrap_or_else(|_| {
        panic!("{REWARD_PG_URL} is required; refusing to skip reward HTTP acceptance coverage")
    });
    let target = url
        .parse::<tokio_postgres::Config>()
        .expect("parse test URL");
    assert!(!target.get_hosts().is_empty());
    assert!(
        target.get_hosts().iter().all(|host| match host {
            tokio_postgres::config::Host::Tcp(host) => host
                .parse::<std::net::IpAddr>()
                .is_ok_and(|address| address.is_loopback()),
            #[cfg(unix)]
            tokio_postgres::config::Host::Unix(_) => true,
        }),
        "test hosts must be loopback or local sockets"
    );
    assert!(
        target
            .get_hostaddrs()
            .iter()
            .all(std::net::IpAddr::is_loopback)
    );
    assert!(
        target
            .get_dbname()
            .is_some_and(|name| name.starts_with("reward_test_"))
    );
    let admin = reward_backend_for_url(url.as_str()).await;
    admin
        .run_migrations()
        .await
        .expect("reward migrations apply");
    provision_test_logins(admin.as_ref()).await;
    RewardTestBackends {
        issuer: reward_backend_for_url(&reward_test_url_with_user(
            &url,
            "trace_reward_http_issuer",
        ))
        .await,
        participant: reward_backend_for_url(&reward_test_url_with_user(
            &url,
            "trace_reward_http_participant",
        ))
        .await,
        admin,
    }
}

fn reward_test_url_with_user(url: &str, user: &str) -> String {
    let mut parsed = reqwest::Url::parse(url).expect("parse test URL");
    parsed.set_username(user).expect("set test login role");
    parsed.into()
}

async fn reward_backend_for_url(url: &str) -> Arc<PgBackend> {
    let config = DatabaseConfig {
        url: SecretString::from(url.to_string()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: Some(SecretString::from(reward_test_url_with_user(
            url,
            "trace_reward_http_resolver",
        ))),
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    Arc::new(
        PgBackend::new(&config)
            .await
            .expect("reward test database available"),
    )
}

async fn provision_test_logins(admin: &PgBackend) {
    let client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("administrator connection");
    client
        .batch_execute(
            "DO $$ BEGIN\n\
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_reward_http_issuer') THEN\n\
                   CREATE ROLE trace_reward_http_issuer LOGIN NOSUPERUSER NOBYPASSRLS;\n\
               END IF;\n\
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_reward_http_participant') THEN\n\
                   CREATE ROLE trace_reward_http_participant LOGIN NOSUPERUSER NOBYPASSRLS;\n\
               END IF;\n\
               IF NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = 'trace_reward_http_resolver') THEN\n\
                   CREATE ROLE trace_reward_http_resolver LOGIN NOSUPERUSER NOBYPASSRLS;\n\
               END IF;\n\
             END $$;\n\
             ALTER ROLE trace_reward_http_issuer NOSUPERUSER NOBYPASSRLS INHERIT;\n\
             ALTER ROLE trace_reward_http_participant NOSUPERUSER NOBYPASSRLS INHERIT;\n\
             GRANT trace_reward_runtime TO trace_reward_http_issuer;\n\
             GRANT trace_reward_participant_runtime TO trace_reward_http_participant;\n\
             GRANT trace_login_resolver TO trace_reward_http_resolver;\n\
             GRANT SELECT, INSERT, UPDATE ON trace_tenants, trace_accounts, trace_account_principals,\n\
                 trace_login_links, trace_sessions, trace_account_audit TO trace_reward_http_participant;\n\
             GRANT USAGE, SELECT ON ALL SEQUENCES IN SCHEMA public TO trace_reward_http_participant;",
        )
        .await
        .expect("provision synthetic runtime logins");
    let row = client
        .query_one(
            "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = 'trace_reward_http_participant'",
            &[],
        )
        .await
        .expect("participant role properties");
    assert!(
        !row.get::<_, bool>(0),
        "participant runtime is not superuser"
    );
    assert!(
        !row.get::<_, bool>(1),
        "participant runtime has no RLS bypass"
    );
}

async fn provision_participant_login(admin: &PgBackend, tenant: &str) {
    let client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("administrator connection");
    client
        .execute(
            "INSERT INTO trace_reward_participant_logins (tenant_id, login_role)\n             VALUES ($1, 'trace_reward_http_participant')\n             ON CONFLICT (tenant_id, login_role) DO NOTHING",
            &[&tenant],
        )
        .await
        .expect("participant runtime receives tenant grant");
}

async fn provision_reward_tenant(admin: &PgBackend, tenant: &str) {
    let client = admin
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("administrator connection");
    client
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1) ON CONFLICT (tenant_id) DO NOTHING",
            &[&tenant],
        )
        .await
        .expect("seed reward tenant");
    let issuer = "trace_reward_http_issuer";
    let actor_hash = format!("sha256:{}", "1".repeat(64));
    client
        .execute(
            "INSERT INTO trace_reward_operators (tenant_id, login_role, actor_hash, operator_role)\n             VALUES ($1, $2, $3, 'issuer')\n             ON CONFLICT (tenant_id, login_role) DO NOTHING",
            &[&tenant, &issuer, &actor_hash],
        )
        .await
        .expect("seed issuer authorization");
}

async fn publish_offer(
    backend: &PgBackend,
    program_id: Uuid,
) -> trace_commons_server::reward_participant::RewardOffer {
    let text = |name: &str| format!("reward HTTP test {program_id} {name}");
    let hash = |value: &str| format!("sha256:{:x}", sha2::Sha256::digest(value.as_bytes()));
    let definition = text("definition");
    let rubric = text("rubric");
    let evidence = text("evidence");
    let rights = text("rights");
    let challenge = text("challenge");
    let evaluator = text("evaluator");
    let terms = RewardProgramTerms {
        schema_version: 1,
        activity_kind: RewardActivityKind::MissionCompletion,
        definition_hash: hash(&definition),
        rubric_hash: hash(&rubric),
        evaluator_policy_hash: hash(&evaluator),
        required_evidence_hash: hash(&evidence),
        rights_hash: hash(&rights),
        challenge_policy_hash: hash(&challenge),
        sponsor_hash: format!("sha256:{}", "1".repeat(64)),
        award_units: 7,
        capacity_units: 70,
        participant_cap_units: 14,
        closes_at: Utc::now() + Duration::days(1),
        reservation_ttl_seconds: 3600,
    };
    let manifest = RewardOfferManifest {
        schema_version: 1,
        definition,
        rubric,
        required_evidence: evidence,
        rights,
        challenge_policy: challenge,
        evaluator_policy: evaluator,
        reservation_terms: text("reservation terms"),
    };
    backend
        .reward_offer_publish(TENANT, program_id, &terms, &manifest)
        .await
        .expect("issuer publishes offer")
}

async fn serve_loopback(state: Arc<AppState>) -> (String, JoinHandle<()>) {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .expect("loopback bind");
    let address = listener.local_addr().expect("listener address");
    let task = tokio::spawn(async move {
        axum::serve(listener, app(state))
            .await
            .expect("loopback server");
    });
    (format!("http://{address}"), task)
}

async fn reserve_with_header(
    client: &reqwest::Client,
    base: &str,
    program: Uuid,
    version: &str,
    header: &str,
    credential: &str,
) {
    let response = client
        .post(format!(
            "{base}/v1/account/reward-offers/{program}/reservations"
        ))
        .header(header, credential)
        .json(&json!({
            "reservation_id": Uuid::new_v4(),
            "offer_version_hash": version,
        }))
        .send()
        .await
        .expect("reservation request");
    assert_eq!(response.status(), reqwest::StatusCode::OK);
    assert_safe_reqwest_headers(response.headers());
}

async fn native_account_token(state: &Arc<AppState>) -> String {
    let cookie = mint_redeem_session_cookie_value(state, "token-a").await;
    let headers = cookie_request_headers("tc_account_session", &cookie);
    let account = resolve_account_ctx(state.as_ref(), &headers)
        .await
        .expect("cookie account");
    let (verifier, challenge) = native_test_pkce();
    let (status, request_id) = native_start(state, &challenge, &native_loopback_redirect()).await;
    assert_eq!(status, StatusCode::OK);
    let request_id = request_id.expect("native request id");
    let code = issue_native_authorization_code(
        state.as_ref(),
        &request_id,
        TENANT,
        account.account_id.as_uuid(),
    )
    .expect("native code")
    .split("code=")
    .nth(1)
    .and_then(|value| value.split('&').next())
    .expect("native code query")
    .to_string();
    let (_, token) = native_exchange(state, &request_id, &code, &verifier).await;
    token.expect("native token")
}

async fn expire_native_token(backend: &PgBackend, token: &str) {
    let (_, token_hash) = native_token_parts(token).expect("well-formed native token");
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("diagnostic connection");
    client
        .execute(
            "UPDATE trace_sessions SET expires_at = clock_timestamp() - interval '1 second' WHERE token_hash = $1",
            &[&token_hash],
        )
        .await
        .expect("expire synthetic native session");
}

async fn assert_router_status(state: &Arc<AppState>, request: Request<Body>, expected: StatusCode) {
    let response = app(state.clone())
        .oneshot(request)
        .await
        .expect("router response");
    assert_eq!(response.status(), expected);
    assert_safe_axum_headers(response.headers());
}

async fn assert_http(
    request: impl std::future::Future<Output = Result<reqwest::Response, reqwest::Error>>,
    expected: StatusCode,
) {
    let response = request.await.expect("HTTP response");
    assert_eq!(response.status(), expected);
    assert_safe_reqwest_headers(response.headers());
}

fn assert_safe_axum_headers(headers: &HeaderMap) {
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
}

fn assert_safe_reqwest_headers(headers: &reqwest::header::HeaderMap) {
    assert_eq!(
        headers
            .get("cache-control")
            .and_then(|value| value.to_str().ok()),
        Some("no-store")
    );
    assert_eq!(
        headers
            .get("x-content-type-options")
            .and_then(|value| value.to_str().ok()),
        Some("nosniff")
    );
}

async fn assert_no_participant_side_effects(backend: &PgBackend, reservation: Uuid) {
    let client = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("diagnostic connection");
    let awards: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM trace_reward_awards WHERE reservation_id = $1",
            &[&reservation],
        )
        .await
        .expect("award count")
        .get(0);
    let decisions: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM trace_reward_decisions WHERE reservation_id = $1",
            &[&reservation],
        )
        .await
        .expect("decision count")
        .get(0);
    let credits: i64 = client
        .query_one(
            "SELECT COUNT(*) FROM trace_credit_ledger WHERE tenant_id = $1",
            &[&TENANT],
        )
        .await
        .expect("credit count")
        .get(0);
    assert_eq!(awards, 0, "reservation must not create an award");
    assert_eq!(
        decisions, 0,
        "reservation must not create a decision or evidence"
    );
    assert_eq!(credits, 0, "reservation must not create Trace Credit");
}
