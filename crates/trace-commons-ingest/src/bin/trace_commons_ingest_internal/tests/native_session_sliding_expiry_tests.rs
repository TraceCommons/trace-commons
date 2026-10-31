// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Sliding expiry for native account sessions.
//!
//! A native session's token rotates on use once it is older than
//! `NATIVE_SESSION_ROTATION_INTERVAL_SECS`, and each rotation moves its
//! `expires_at` to `now + NATIVE_SESSION_TTL_HOURS`, never past
//! `created_at + NATIVE_SESSION_MAX_LIFETIME_DAYS`. The middleware hands the
//! new expiry back beside the rotated token.
//!
//! The PostgreSQL tests self-skip without `TRACE_COMMONS_PG_TEST_DATABASE_URL`
//! and run in the `ingest-bin-postgres` CI job.

use super::*;
use axum::body::Body;
use tower::ServiceExt;
use trace_commons_protocol::ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER;
use trace_commons_server::account_native_auth::NATIVE_SESSION_MAX_LIFETIME_DAYS;

// --- in-memory: the middleware's wire --------------------------------------

fn sliding_tenant() -> String {
    format!("nearai-{}", "e".repeat(64))
}

const SLIDING_SECRET: &str = "native-sliding-expiry-test-secret";

fn sliding_state(db: Arc<NativeAuthTestDb>) -> (tempfile::TempDir, Arc<AppState>) {
    let temp = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = db;
    let state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    (temp, state)
}

fn native_request(tenant: &str, secret: &str) -> axum::http::Request<Body> {
    axum::http::Request::builder()
        .method("GET")
        .uri("/v1/account/inference-connection/offers")
        .header(
            AUTHORIZATION,
            format!("Bearer {}", native_token_value(tenant, secret)),
        )
        .body(Body::empty())
        .expect("request")
}

/// A rotated native token travels with its new expiry, RFC 3339 in UTC, on
/// the same response.
#[tokio::test]
async fn a_rotated_native_token_carries_its_expiry() {
    let db = Arc::new(NativeAuthTestDb::default());
    let expires_at = (Utc::now() + Duration::hours(12)).trunc_subsecs(0);
    db.insert_session(
        &sliding_tenant(),
        Uuid::new_v4(),
        &hash_secret(SLIDING_SECRET),
        NATIVE_SESSION_CLIENT_KIND,
        expires_at,
    );
    *db.rotate_to.lock().unwrap() = Some("rotated-sliding-secret".to_string());
    let (_temp, state) = sliding_state(db);
    let response = app(state.clone())
        .oneshot(native_request(&sliding_tenant(), SLIDING_SECRET))
        .await
        .expect("router");
    assert_eq!(response.status(), StatusCode::OK);
    let headers = response.headers();
    assert_eq!(
        headers
            .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
            .and_then(|v| v.to_str().ok()),
        Some(native_token_value(&sliding_tenant(), "rotated-sliding-secret").as_str())
    );
    let sent = headers
        .get(ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER)
        .and_then(|v| v.to_str().ok())
        .expect("the expiry rides with the rotated token");
    let parsed = DateTime::parse_from_rfc3339(sent)
        .expect("RFC 3339")
        .with_timezone(&Utc);
    assert_eq!(parsed, expires_at);
}

/// No rotation, no expiry header: the expiry only ever moves with the token.
#[tokio::test]
async fn an_unrotated_native_response_carries_no_expiry() {
    let db = Arc::new(NativeAuthTestDb::default());
    db.insert_session(
        &sliding_tenant(),
        Uuid::new_v4(),
        &hash_secret(SLIDING_SECRET),
        NATIVE_SESSION_CLIENT_KIND,
        Utc::now() + Duration::hours(12),
    );
    let (_temp, state) = sliding_state(db);
    let response = app(state.clone())
        .oneshot(native_request(&sliding_tenant(), SLIDING_SECRET))
        .await
        .expect("router");
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response
            .headers()
            .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
            .is_none()
    );
    assert!(
        response
            .headers()
            .get(ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER)
            .is_none()
    );
}

// --- PostgreSQL --------------------------------------------------------------

struct SlidingFixture {
    backend: Arc<PgBackend>,
    tenant: String,
}

/// A fresh tenant with one account.
async fn sliding_fixture() -> Option<(SlidingFixture, Uuid)> {
    let backend = postgres_backend_for_ingest_test().await?;
    let admin = backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    let tenant = format!("nearai-{}", Uuid::new_v4().simple().to_string().repeat(2));
    admin
        .execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES ($1)",
            &[&tenant],
        )
        .await
        .expect("tenant");
    let account = Uuid::new_v4();
    admin
        .execute(
            "INSERT INTO trace_accounts (tenant_id, account_id) VALUES ($1, $2)",
            &[&tenant, &account],
        )
        .await
        .expect("account");
    Some((SlidingFixture { backend, tenant }, account))
}

/// The ages are in seconds before now. `expires_in` is seconds after now and
/// may be negative.
struct SessionShape<'a> {
    client_kind: &'a str,
    created_ago: i64,
    issued_ago: i64,
    seen_ago: i64,
    expires_in: i64,
    revoked: bool,
}

impl SessionShape<'static> {
    /// A native session two hours old, minted with today's 12h lifetime:
    /// past the rotation cadence, ten hours left.
    fn native_aged() -> Self {
        SessionShape {
            client_kind: NATIVE_SESSION_CLIENT_KIND,
            created_ago: 2 * 3600,
            issued_ago: 2 * 3600,
            seen_ago: 60,
            expires_in: 10 * 3600,
            revoked: false,
        }
    }
}

impl SlidingFixture {
    /// A secret unique to this run's tenant: `token_hash` is globally unique,
    /// and a failed run leaves its rows behind.
    fn secret(&self, name: &str) -> String {
        format!("{name}-{}", self.tenant)
    }

    async fn insert(&self, account: Uuid, secret: &str, shape: &SessionShape<'_>) {
        let admin = self
            .backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("admin");
        admin
            .execute(
                "INSERT INTO trace_sessions
                    (tenant_id, session_id, account_id, token_hash, client_kind,
                     created_at, token_issued_at, last_seen_at, expires_at, revoked_at)
                 VALUES ($1, $2, $3, $4, $5,
                         now() - make_interval(secs => $6),
                         now() - make_interval(secs => $7),
                         now() - make_interval(secs => $8),
                         now() + make_interval(secs => $9),
                         CASE WHEN $10 THEN now() ELSE NULL END)",
                &[
                    &self.tenant,
                    &Uuid::new_v4(),
                    &account,
                    &hash_secret(secret),
                    &shape.client_kind,
                    &(shape.created_ago as f64),
                    &(shape.issued_ago as f64),
                    &(shape.seen_ago as f64),
                    &(shape.expires_in as f64),
                    &shape.revoked,
                ],
            )
            .await
            .expect("session");
    }

    /// `(token_hash, created_at, expires_at, revoked)` for the row whose current
    /// or previous token is `secret`.
    async fn row(&self, secret: &str) -> (String, DateTime<Utc>, DateTime<Utc>, bool) {
        let admin = self
            .backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("admin");
        let hash = hash_secret(secret);
        let row = admin
            .query_one(
                "SELECT token_hash, created_at, expires_at, revoked_at IS NOT NULL AS revoked
                   FROM trace_sessions
                  WHERE tenant_id = $1 AND (token_hash = $2 OR prev_token_hash = $2)",
                &[&self.tenant, &hash],
            )
            .await
            .expect("row");
        (
            row.get("token_hash"),
            row.get("created_at"),
            row.get("expires_at"),
            row.get("revoked"),
        )
    }

    async fn rotation_audits(&self) -> i64 {
        let admin = self
            .backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("admin");
        admin
            .query_one(
                "SELECT count(*) FROM trace_account_audit
                  WHERE tenant_id = $1 AND action = 'account_session_rotated'",
                &[&self.tenant],
            )
            .await
            .expect("audit count")
            .get(0)
    }

    async fn validate(&self, secret: &str) -> Option<trace_commons_server::db::ValidatedSession> {
        self.backend
            .validate_session(&self.tenant, &hash_secret(secret))
            .await
            .expect("validate")
    }

    async fn cleanup(&self) {
        cleanup_pg_trace_tenant(self.backend.as_ref(), &self.tenant).await;
    }
}

/// Within a second: the database clock and this process's clock agree to
/// well inside that on one host.
fn near(actual: DateTime<Utc>, expected: DateTime<Utc>) -> bool {
    (actual - expected).num_milliseconds().abs() < 5_000
}

/// Rotation moves the expiry to a fresh native lifetime from now, and the
/// validated session reports the new value, the one the database stored.
#[tokio::test]
async fn pg_native_rotation_extends_the_expiry() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-extend");
    fx.insert(account, secret, &SessionShape::native_aged())
        .await;
    let (_, _, before, _) = fx.row(secret).await;

    let validated = fx.validate(secret).await.expect("live session");
    let rotated = validated
        .rotated_secret
        .clone()
        .expect("aged token rotates");
    let (hash, _, after, revoked) = fx.row(secret).await;
    assert_eq!(hash, hash_secret(&rotated), "the row carries the new token");
    assert!(!revoked);
    assert!(after > before, "rotation extends the expiry");
    assert!(
        near(
            after,
            Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS)
        ),
        "a fresh native lifetime from now: {after}"
    );
    assert_eq!(
        validated.expires_at, after,
        "the caller is told the stored expiry, not the pre-rotation one"
    );
    fx.cleanup().await;
}

/// The extension never passes `created_at + NATIVE_SESSION_MAX_LIFETIME_DAYS`.
#[tokio::test]
async fn pg_native_rotation_stops_at_the_absolute_cap() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-cap");
    // Thirty days less one hour old: a full 12h extension would overshoot
    // the cap by eleven hours.
    let created_ago = NATIVE_SESSION_MAX_LIFETIME_DAYS * 86_400 - 3600;
    fx.insert(
        account,
        secret,
        &SessionShape {
            created_ago,
            expires_in: 1800,
            ..SessionShape::native_aged()
        },
    )
    .await;
    let validated = fx.validate(secret).await.expect("live session");
    assert!(validated.rotated_secret.is_some());
    let (_, created_at, after, _) = fx.row(secret).await;
    assert_eq!(
        after,
        created_at + Duration::days(NATIVE_SESSION_MAX_LIFETIME_DAYS),
        "capped at the absolute lifetime"
    );
    assert_eq!(validated.expires_at, after);

    // At the cap, a further rotation leaves the expiry where it is.
    let next = validated.rotated_secret.unwrap();
    let admin = fx
        .backend
        .raw_pool_for_tests_and_diagnostics()
        .get()
        .await
        .expect("admin");
    admin
        .execute(
            "UPDATE trace_sessions SET token_issued_at = now() - interval '2 hours'
              WHERE tenant_id = $1 AND token_hash = $2",
            &[&fx.tenant, &hash_secret(&next)],
        )
        .await
        .expect("age the token");
    let again = fx.validate(&next).await.expect("still live");
    assert!(again.rotated_secret.is_some());
    let (_, _, capped, _) = fx.row(&next).await;
    assert_eq!(capped, after, "never past the cap, and never shortened");
    fx.cleanup().await;
}

/// A token younger than the cadence neither rotates nor extends.
#[tokio::test]
async fn pg_a_fresh_native_token_neither_rotates_nor_extends() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-fresh");
    fx.insert(
        account,
        secret,
        &SessionShape {
            issued_ago: 60,
            ..SessionShape::native_aged()
        },
    )
    .await;
    let (_, _, before, _) = fx.row(secret).await;
    let validated = fx.validate(secret).await.expect("live session");
    assert!(validated.rotated_secret.is_none());
    let (_, _, after, _) = fx.row(secret).await;
    assert_eq!(after, before);
    fx.cleanup().await;
}

/// Expired, revoked and idle-capped sessions are refused and never extended.
#[tokio::test]
async fn pg_dead_native_sessions_never_extend() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let cases = [
        (
            fx.secret("pg-sliding-expired"),
            SessionShape {
                expires_in: -60,
                ..SessionShape::native_aged()
            },
        ),
        (
            fx.secret("pg-sliding-revoked"),
            SessionShape {
                revoked: true,
                ..SessionShape::native_aged()
            },
        ),
        (
            fx.secret("pg-sliding-idle"),
            SessionShape {
                // Past the 3-day idle cap but inside the 30-day lifetime.
                created_ago: 5 * 86_400,
                seen_ago: 4 * 86_400,
                ..SessionShape::native_aged()
            },
        ),
    ];
    for (secret, shape) in &cases {
        fx.insert(account, secret, shape).await;
        let (_, _, before, _) = fx.row(secret).await;
        assert!(fx.validate(secret).await.is_none(), "{secret} is refused");
        let (hash, _, after, _) = fx.row(secret).await;
        assert_eq!(hash, hash_secret(secret), "{secret} did not rotate");
        assert_eq!(after, before, "{secret} did not extend");
    }
    assert_eq!(fx.rotation_audits().await, 0);
    fx.cleanup().await;
}

/// A browser session keeps its fixed 7-day expiry through a rotation:
/// the sliding window is a native-session rule only.
#[tokio::test]
async fn pg_a_browser_rotation_keeps_its_fixed_expiry() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-web");
    fx.insert(
        account,
        secret,
        &SessionShape {
            client_kind: "web",
            created_ago: 13 * 3600,
            issued_ago: 13 * 3600,
            seen_ago: 60,
            expires_in: 6 * 86_400,
            revoked: false,
        },
    )
    .await;
    let (_, _, before, _) = fx.row(secret).await;
    let validated = fx.validate(secret).await.expect("live session");
    assert!(
        validated.rotated_secret.is_some(),
        "a 13h browser token rotates"
    );
    let (_, _, after, _) = fx.row(secret).await;
    assert_eq!(after, before);
    assert_eq!(validated.expires_at, before);
    // And a browser token two hours old is not rotated by the native cadence.
    let young = &fx.secret("pg-sliding-web-young");
    fx.insert(
        account,
        young,
        &SessionShape {
            client_kind: "web",
            created_ago: 2 * 3600,
            issued_ago: 2 * 3600,
            seen_ago: 60,
            expires_in: 6 * 86_400,
            revoked: false,
        },
    )
    .await;
    assert!(fx.validate(young).await.unwrap().rotated_secret.is_none());
    fx.cleanup().await;
}

/// Hold the session row locked from a separate transaction until `waiters`
/// backends are queued behind it, run `release` in that transaction, commit.
async fn hold_row_until_waiting(
    fx: &SlidingFixture,
    secret: &str,
    waiters: i64,
    release: &str,
) -> tokio::task::JoinHandle<()> {
    let backend = fx.backend.clone();
    let tenant = fx.tenant.clone();
    let hash = hash_secret(secret);
    let release = release.to_string();
    let (locked_tx, locked_rx) = tokio::sync::oneshot::channel();
    let handle = tokio::spawn(async move {
        let mut holder = backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("holder");
        let tx = holder.transaction().await.expect("tx");
        tx.query_one(
            "SELECT 1 FROM trace_sessions
              WHERE tenant_id = $1 AND token_hash = $2 FOR UPDATE",
            &[&tenant, &hash],
        )
        .await
        .expect("lock the row");
        let _ = locked_tx.send(());
        let watcher = backend
            .raw_pool_for_tests_and_diagnostics()
            .get()
            .await
            .expect("watcher");
        let deadline = std::time::Instant::now() + StdDuration::from_secs(20);
        loop {
            let waiting: i64 = watcher
                .query_one(
                    "SELECT count(*) FROM pg_stat_activity
                      WHERE datname = current_database() AND wait_event_type = 'Lock'",
                    &[],
                )
                .await
                .expect("activity")
                .get(0);
            if waiting >= waiters {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "only {waiting} of {waiters} rotations queued on the row"
            );
            tokio::time::sleep(StdDuration::from_millis(20)).await;
        }
        if !release.is_empty() {
            tx.execute(release.as_str(), &[&tenant, &hash])
                .await
                .expect("release statement");
        }
        tx.commit().await.expect("release");
    });
    locked_rx.await.expect("row locked");
    handle
}

/// Two requests race to rotate the same aged token. Both are served, exactly
/// one rotation lands, the client is handed only the token that exists, and
/// the expiry moves once.
#[tokio::test]
async fn pg_concurrent_rotations_land_once() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-race");
    fx.insert(account, secret, &SessionShape::native_aged())
        .await;
    let holder = hold_row_until_waiting(&fx, secret, 2, "").await;
    let (first, second) = tokio::join!(fx.validate(secret), fx.validate(secret));
    holder.await.expect("holder");
    let first = first.expect("first request is served");
    let second = second.expect("second request is served on the old token's grace");
    let rotated: Vec<String> = [first.rotated_secret, second.rotated_secret]
        .into_iter()
        .flatten()
        .collect();
    assert_eq!(rotated.len(), 1, "exactly one request carries a new token");
    let (hash, _, after, _) = fx.row(secret).await;
    assert_eq!(
        hash,
        hash_secret(&rotated[0]),
        "the token handed out is the one stored"
    );
    assert!(near(
        after,
        Utc::now() + Duration::hours(NATIVE_SESSION_TTL_HOURS)
    ));
    assert_eq!(fx.rotation_audits().await, 1);
    fx.cleanup().await;
}

/// A sign-out that lands between a request's read and its rotation wins: the
/// rotation does not resurrect the session, hand out a token, or extend it.
#[tokio::test]
async fn pg_a_revoke_racing_a_rotation_is_not_undone() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-revoke-race");
    fx.insert(account, secret, &SessionShape::native_aged())
        .await;
    let (_, _, before, _) = fx.row(secret).await;
    let holder = hold_row_until_waiting(
        &fx,
        secret,
        1,
        "UPDATE trace_sessions SET revoked_at = now()
          WHERE tenant_id = $1 AND token_hash = $2",
    )
    .await;
    let raced = fx.validate(secret).await;
    holder.await.expect("holder");
    assert!(
        raced.as_ref().is_none_or(|s| s.rotated_secret.is_none()),
        "no token is handed out for a revoked session"
    );
    let (hash, _, after, revoked) = fx.row(secret).await;
    assert!(revoked, "the sign-out stands");
    assert_eq!(hash, hash_secret(secret), "the revoked row did not rotate");
    assert_eq!(after, before, "the revoked row did not extend");
    assert_eq!(fx.rotation_audits().await, 0);
    assert!(fx.validate(secret).await.is_none());
    fx.cleanup().await;
}

/// End to end through the router: the expiry header matches the database.
#[tokio::test]
async fn pg_the_router_returns_the_stored_extended_expiry() {
    let Some((fx, account)) = sliding_fixture().await else {
        return;
    };
    let secret = &fx.secret("pg-sliding-router");
    fx.insert(account, secret, &SessionShape::native_aged())
        .await;
    let temp = tempfile::tempdir().expect("temp dir");
    let db: Arc<dyn Database> = fx.backend.clone();
    let state = test_state_with_options(
        temp.path().to_path_buf(),
        Some(db),
        None,
        false,
        false,
        false,
        false,
    );
    let response = app(state.clone())
        .oneshot(native_request(&fx.tenant, secret))
        .await
        .expect("router");
    assert_eq!(response.status(), StatusCode::OK);
    let token = response
        .headers()
        .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .expect("rotated token")
        .to_string();
    let sent = response
        .headers()
        .get(ACCOUNT_NATIVE_SESSION_EXPIRES_AT_HEADER)
        .and_then(|v| v.to_str().ok())
        .expect("expiry header")
        .to_string();
    let sent = DateTime::parse_from_rfc3339(&sent)
        .expect("RFC 3339")
        .with_timezone(&Utc);
    let (_, _, stored, _) = fx.row(secret).await;
    assert_eq!(sent, stored.trunc_subsecs(0));
    // The new token works, and is the one now stored.
    let new_secret = token.rsplit_once('.').expect("tenant.secret").1;
    let (hash, _, _, _) = fx.row(new_secret).await;
    assert_eq!(hash, hash_secret(new_secret));
    fx.cleanup().await;
}
