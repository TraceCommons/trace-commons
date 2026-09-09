//! What the contributor's NEAR AI account has left, and the session that
//! reading it requires.
//!
//! The credential ceremony next door mints an inference key and is done. This
//! is the other half of the same account, and it cannot be served by that key:
//! `GET /v1/organizations/{org}/usage/balance` is `session_token`-only, as is
//! every management route on cloud-api, and an `sk-` key is a 401 on all of
//! them. So a balance needs a session, and a session needs a refresh token
//! kept on disk -- with everything that implies, which
//! [`crate::daemon::settings::NearAiSession`] states plainly.
//!
//! The honesty rules here are the ones `routing::ironwire`'s spend figure
//! already holds itself to, for the same reason: a number about somebody's
//! money is worse than no number when it is wrong.
//!
//! - **A stale balance is not a balance.** Every failure -- no session, a
//!   refused refresh, a service that did not answer, a body this build cannot
//!   parse -- reaches a *named* state. None of them returns the last good
//!   figure, and none of them returns zero.
//! - **Not knowing is not zero.** `remaining` is nullable in the service's own
//!   schema: an organization with no credit ceiling configured genuinely has
//!   no remaining balance to report. That null survives all the way to the
//!   socket as a null, because rendering it as `0` would tell a contributor
//!   they are out of credit when nobody ever set a limit.
//! - **The observation time travels with the number**, so a shell can say how
//!   old the figure is instead of implying it is current.
//!
//! Nothing here logs a token, an account name, or an organization id. The
//! states are labels, and the numbers are numbers.

use super::api::{CloudApi, OrganizationBalance};
use super::loopback::SessionTokens;
use crate::daemon::ipc::DaemonShared;
use crate::daemon::settings::NearAiSession;
use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::OnceLock,
    time::{Duration, Instant},
};

/// How long a fetched balance is served again without going back to the
/// service.
///
/// Short on purpose. The point of the cache is that a shell repainting a
/// settings pane three times in a second causes one HTTP call, not that a
/// figure is kept around; anything long enough to be worth calling a cache
/// would be long enough to be the "figure from an hour ago" this module
/// exists not to serve.
const BALANCE_TTL: Duration = Duration::from_secs(60);

/// How long one exchanged access token is reused before another is minted.
///
/// The service does not tell us the access token's lifetime -- the exchange
/// response carries an expiry for the *refresh* token only -- so this is a
/// deliberately conservative floor rather than a decoded claim. A stale one
/// costs a 401 and a single retry, which [`read`] performs; guessing long from
/// an unverified JWT body would cost correctness.
const ACCESS_TOKEN_REUSE: Duration = Duration::from_secs(300);

/// The units the service reports in: fixed scale 9, USD.
///
/// Carried on the wire rather than assumed by each shell, and not converted
/// here: a daemon that pre-rounded to cents would be deciding how much
/// precision a contributor is allowed to see.
const BALANCE_CURRENCY: &str = "USD";
const BALANCE_SCALE: u8 = 9;

/// The `state` labels this module puts on the wire.
///
/// Declared once and returned by [`BalanceReport::state`], so the copy module
/// that renders a sentence per state can import them rather than respell
/// them. A label spelled twice is two labels that have not disagreed yet, and
/// the failure mode of a typo on this particular set is a real balance
/// rendering as "could not be read".
pub const LABEL_BALANCE_NO_SESSION: &str = "no_session";
/// [`BalanceReport::SessionExpired`].
pub const LABEL_BALANCE_SESSION_EXPIRED: &str = "session_expired";
/// [`BalanceReport::NoOrganization`].
pub const LABEL_BALANCE_NO_ORGANIZATION: &str = "no_organization";
/// [`BalanceReport::Unavailable`].
pub const LABEL_BALANCE_UNAVAILABLE: &str = "unavailable";
/// [`BalanceReport::Known`].
pub const LABEL_BALANCE_KNOWN: &str = "known";

/// One balance, and when it was observed.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct BalanceReading {
    /// What is left, in nano-dollars, or `null` when the service reports no
    /// remaining balance. Never rendered as zero by anything downstream.
    pub remaining_nanos: Option<i64>,
    /// The configured ceiling, in nano-dollars, or `null` when unset.
    pub spend_limit_nanos: Option<i64>,
    pub total_spent_nanos: i64,
    pub total_requests: i64,
    pub total_tokens: i64,
    /// This daemon's clock at the moment the service answered. Not the
    /// service's own `updated_at`: what a shell needs to say is how long ago
    /// *we* asked.
    pub observed_at: DateTime<Utc>,
}

/// Every way this can come out, each with a name.
///
/// Deliberately not `Result<Option<..>>`. A shell has to render four different
/// sentences -- "you have not connected an account", "your session expired,
/// connect again", "we could not reach NEAR AI", "your account has no
/// organization" -- and collapsing them into an error plus a null would make
/// all four read as the same shrug.
#[derive(Debug, Clone, Copy)]
pub enum BalanceReport {
    /// No session is stored. Either the account was never connected, or it was
    /// forgotten. Not an error: it is the default state of a fresh install.
    NoSession,
    /// The refresh token was refused. Recoverable, and the recovery is to run
    /// the credential ceremony again -- which does not require forgetting
    /// first, because the ceremony overwrites both records.
    ///
    /// **The stored token is not cleared by this.** A service having a bad
    /// afternoon and answering 401 must not be able to delete a contributor's
    /// session; the only things that remove it are a successful rotation and
    /// an explicit forget.
    SessionExpired,
    /// The account has no active organization. Real rather than theoretical:
    /// `get_or_create_oauth_user` creates the organization and workspace
    /// best-effort, logging on failure without failing user creation.
    NoOrganization,
    /// The service did not answer, or answered something this build cannot
    /// read. Says nothing about the balance, which is the point.
    Unavailable,
    Known(BalanceReading),
}

impl BalanceReport {
    /// The label a shell switches on.
    #[must_use]
    pub fn state(&self) -> &'static str {
        match self {
            Self::NoSession => LABEL_BALANCE_NO_SESSION,
            Self::SessionExpired => LABEL_BALANCE_SESSION_EXPIRED,
            Self::NoOrganization => LABEL_BALANCE_NO_ORGANIZATION,
            Self::Unavailable => LABEL_BALANCE_UNAVAILABLE,
            Self::Known(_) => LABEL_BALANCE_KNOWN,
        }
    }

    /// The IPC body.
    ///
    /// The numeric keys are present and `null` in every non-`known` state
    /// rather than absent. An absent key is ambiguous between "this daemon is
    /// too old to know" and "this daemon knows it does not know"; a null is
    /// only ever the second.
    #[must_use]
    pub fn to_value(self) -> serde_json::Value {
        let mut value = serde_json::json!({
            "state": self.state(),
            "currency": BALANCE_CURRENCY,
            "scale": BALANCE_SCALE,
            "remaining_nanos": serde_json::Value::Null,
            "spend_limit_nanos": serde_json::Value::Null,
            "total_spent_nanos": serde_json::Value::Null,
            "total_requests": serde_json::Value::Null,
            "total_tokens": serde_json::Value::Null,
            "observed_at": serde_json::Value::Null,
        });
        if let Self::Known(reading) = self
            && let Some(object) = value.as_object_mut()
            && let Ok(serde_json::Value::Object(fields)) = serde_json::to_value(reading)
        {
            object.extend(fields);
        }
        value
    }
}

/// Per-directory cached state.
#[derive(Default)]
struct Cached {
    /// The access token last exchanged, and when. In memory only: it is a
    /// second credential, it is short-lived, and the exchange route is
    /// authenticated by the refresh token rather than by it, so there is
    /// nothing on disk to gain.
    access_token: Option<(String, Instant)>,
    balance: Option<(BalanceReading, Instant)>,
}

/// One lock over every directory's cache, held across the whole read.
///
/// Coarse deliberately. The refresh token **rotates on every exchange**, so
/// two concurrent reads would race to spend the same token and the loser would
/// persist a value the service had already retired -- locking a contributor
/// out of a balance until they re-ran the ceremony. Serialising the reads
/// costs nothing worth measuring: a daemon serves one config directory, and a
/// balance read happens when a settings pane opens.
fn cache() -> &'static tokio::sync::Mutex<HashMap<PathBuf, Cached>> {
    static CACHE: OnceLock<tokio::sync::Mutex<HashMap<PathBuf, Cached>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Drop a directory's cached access token and balance.
///
/// Called on forget, so a re-connected account cannot be shown the previous
/// account's figure, and a forgotten one cannot be served a balance from a
/// session that no longer exists.
pub fn invalidate(dir: &Path) {
    if let Ok(mut map) = cache().try_lock() {
        map.remove(dir);
        return;
    }
    // The lock is held by a read in flight. That read is about to write its
    // own entry, so blocking here would only trade a stale cache for a stalled
    // forget; instead the entry is left and the *stored* session is what the
    // forget removes. A subsequent read finds no session and reports
    // `no_session`, which is the state that matters.
    tracing::debug!("near ai balance cache busy during invalidate");
}

/// The refresh token this directory has stored, if any.
fn stored_session(shared: &DaemonShared) -> Option<NearAiSession> {
    shared
        .settings
        .lock()
        .expect("settings lock")
        .near_ai_session
        .clone()
}

/// Map a label this module's callees raise onto the state a shell renders.
fn report_for(error: &anyhow::Error) -> BalanceReport {
    match error.to_string().as_str() {
        "near_ai_credential_session_expired" | "near_ai_credential_session_missing" => {
            BalanceReport::SessionExpired
        }
        "near_ai_credential_no_organization" => BalanceReport::NoOrganization,
        _ => BalanceReport::Unavailable,
    }
}

/// Read the balance, refreshing the session if the access token is stale.
///
/// Infallible from the caller's perspective by construction: there is no
/// failure this can produce that is not one of [`BalanceReport`]'s named
/// states, and none of those states is a number.
pub async fn read(shared: &DaemonShared) -> BalanceReport {
    match CloudApi::live() {
        Ok(api) => read_with(shared, &api).await,
        // A client this build could not construct is a machine whose trust
        // store would not load. Nothing about the balance is known, and that
        // is what is said.
        Err(_) => BalanceReport::Unavailable,
    }
}

/// [`read`] against an explicit client.
///
/// The seam exists so the whole state machine -- rotation, the single retry, a
/// refused refresh, an account with no organization -- is exercised against a
/// stub rather than asserted about by reading it. `read` is the only shipping
/// caller and it passes the pinned live client.
async fn read_with(shared: &DaemonShared, api: &CloudApi) -> BalanceReport {
    let dir = shared.store.dir().to_path_buf();
    let mut map = cache().lock().await;
    let entry = map.entry(dir).or_default();

    if let Some((reading, at)) = entry.balance
        && at.elapsed() < BALANCE_TTL
    {
        return BalanceReport::Known(reading);
    }
    // Whatever happens below, the previous figure is not what this call
    // returns. Cleared here rather than on each failure path so no branch can
    // forget to, which is the mistake the ironwire spend figure was written to
    // avoid.
    entry.balance = None;

    let Some(stored) = stored_session(shared) else {
        entry.access_token = None;
        return BalanceReport::NoSession;
    };

    // A refresh token whose recorded expiry has passed is still offered to the
    // service rather than refused locally. The expiry we hold came from an
    // earlier exchange and this machine's clock may simply be wrong; the
    // service is the only thing that actually knows, and its answer is a 401
    // we already have a name for.
    let mut access_token = match entry.access_token.take() {
        Some((token, at)) if at.elapsed() < ACCESS_TOKEN_REUSE => token,
        _ => match super::exchange(shared, api, &stored).await {
            Ok(token) => token,
            Err(error) => return report_for(&error),
        },
    };

    let mut refreshed_once = false;
    loop {
        let session = SessionTokens {
            access_token: access_token.clone(),
            // Never sent: the management calls below authenticate with the
            // access token, and this field exists only because the type does.
            refresh_token: String::new(),
        };
        match fetch(api, &session).await {
            Ok(balance) => {
                let reading = BalanceReading {
                    remaining_nanos: balance.remaining,
                    spend_limit_nanos: balance.spend_limit,
                    total_spent_nanos: balance.total_spent,
                    total_requests: balance.total_requests,
                    total_tokens: balance.total_tokens,
                    observed_at: Utc::now(),
                };
                let entry = map.entry(shared.store.dir().to_path_buf()).or_default();
                entry.access_token = Some((access_token, Instant::now()));
                entry.balance = Some((reading, Instant::now()));
                return BalanceReport::Known(reading);
            }
            // A 401 on a management call with a reused access token means the
            // token aged out, not that the session is gone. One exchange and
            // one retry distinguishes the two; a second 401 is the session
            // itself, and is reported as such.
            Err(error)
                if !refreshed_once && error.to_string() == "near_ai_credential_session_expired" =>
            {
                refreshed_once = true;
                let stored = match stored_session(shared) {
                    Some(stored) => stored,
                    None => return BalanceReport::NoSession,
                };
                match super::exchange(shared, api, &stored).await {
                    Ok(token) => access_token = token,
                    Err(error) => return report_for(&error),
                }
            }
            Err(error) => return report_for(&error),
        }
    }
}

/// Resolve the organization the ceremony would have chosen, then read its
/// balance.
async fn fetch(api: &CloudApi, session: &SessionTokens) -> Result<OrganizationBalance> {
    let organization = api.first_active_organization(session).await?;
    api.organization_balance(session, &organization).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::settings::DaemonSettings;
    use axum::{Json, Router, extract::Request, routing::get};
    use std::sync::{Arc, Mutex};

    /// A daemon whose config directory is its own, so the module-level cache
    /// -- keyed by directory -- cannot be shared between two tests running in
    /// the same process.
    fn shared() -> DaemonShared {
        let (dir, store) = crate::config::tests_support::temp_store();
        // Leak the tempdir for the lifetime of the test process; the store
        // borrows its path, and the cache key is that path.
        std::mem::forget(dir);
        DaemonShared::load(store).unwrap()
    }

    fn with_session(shared: &DaemonShared, refresh_token: &str) {
        let mut settings = shared.settings.lock().unwrap();
        settings.near_ai_session = Some(NearAiSession {
            refresh_token: refresh_token.into(),
            refresh_token_expires_at: None,
            stored_at: Utc::now(),
        });
        settings.save(&shared.store).unwrap();
    }

    /// Every bearer the stub was shown, in order, so a test can prove which
    /// credential class reached which route.
    type Bearers = Arc<Mutex<Vec<(String, String)>>>;

    struct Stub {
        origin: String,
        bearers: Bearers,
        /// Refresh tokens the stub still honours. Rotation removes the spent
        /// one, exactly as the service does, so a test that fails to persist a
        /// rotation is a test that fails.
        live_refresh: Arc<Mutex<Vec<String>>>,
        exchanges: Arc<Mutex<u32>>,
    }

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{address}")
    }

    /// The deployed shapes: a rotating exchange, a paginated organization
    /// list, and a balance in nano-dollars.
    async fn stub(organizations: serde_json::Value, balance: serde_json::Value) -> Stub {
        let bearers: Bearers = Default::default();
        let live_refresh = Arc::new(Mutex::new(vec!["rt_first".to_string()]));
        let exchanges = Arc::new(Mutex::new(0u32));

        let tokens = live_refresh.clone();
        let counter = exchanges.clone();
        let record = bearers.clone();
        let router = Router::new()
            .route(
                "/v1/users/me/access-tokens",
                axum::routing::post(move |request: Request| {
                    let tokens = tokens.clone();
                    let counter = counter.clone();
                    async move {
                        let offered = bearer(&request);
                        let mut held = tokens.lock().unwrap();
                        if !held.contains(&offered) {
                            return (
                                axum::http::StatusCode::UNAUTHORIZED,
                                Json(serde_json::json!({"error": {"message": "spent"}})),
                            )
                                .into_response();
                        }
                        let mut count = counter.lock().unwrap();
                        *count += 1;
                        let rotated = format!("rt_rotated_{count}");
                        // The token that authenticated this call is retired,
                        // which is what the service does and what makes a
                        // lost rotation fatal.
                        held.clear();
                        held.push(rotated.clone());
                        Json(serde_json::json!({
                            "access_token": format!("jwt_{count}"),
                            "refresh_token": rotated,
                            "refresh_token_expiration": "2026-09-15T00:00:00Z",
                        }))
                        .into_response()
                    }
                }),
            )
            .route(
                "/v1/organizations",
                get(move || {
                    let organizations = organizations.clone();
                    async move { Json(organizations) }
                }),
            )
            .route(
                "/v1/organizations/{org}/usage/balance",
                get(move || {
                    let balance = balance.clone();
                    async move { Json(balance) }
                }),
            )
            .layer(axum::middleware::from_fn(
                move |request: Request, next: axum::middleware::Next| {
                    let record = record.clone();
                    let path = request.uri().path().to_string();
                    let offered = bearer(&request);
                    async move {
                        record.lock().unwrap().push((path, offered));
                        next.run(request).await
                    }
                },
            ));
        Stub {
            origin: spawn(router).await,
            bearers,
            live_refresh,
            exchanges,
        }
    }

    fn bearer(request: &Request) -> String {
        request
            .headers()
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .trim_start_matches("Bearer ")
            .to_string()
    }

    use axum::response::IntoResponse;

    fn one_org() -> serde_json::Value {
        serde_json::json!({
            "organizations": [{
                "id": "org-1", "name": "alice-org-ab12", "owner_id": "u",
                "settings": {}, "role": "owner", "is_active": true,
                "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
            }],
            "total": 1, "limit": 50, "offset": 0,
        })
    }

    fn a_balance() -> serde_json::Value {
        serde_json::json!({
            "organization_id": "org-1",
            "total_spent": 1_500_000_000i64,
            "total_spent_display": "$1.50",
            "total_requests": 12,
            "total_tokens": 3456,
            "updated_at": "2026-09-08T00:00:00Z",
            "credit_limits": [],
            "remaining": 8_500_000_000i64,
            "remaining_display": "$8.50",
            "spend_limit": 10_000_000_000i64,
            "spend_limit_display": "$10.00",
        })
    }

    #[tokio::test]
    async fn a_retained_session_buys_a_balance_and_the_rotation_is_persisted_before_it_is_used() {
        let s = shared();
        with_session(&s, "rt_first");
        let stub = stub(one_org(), a_balance()).await;
        let api = CloudApi::for_test(&stub.origin).unwrap();

        let report = read_with(&s, &api).await;
        let BalanceReport::Known(reading) = report else {
            panic!("{}", report.state());
        };
        assert_eq!(reading.remaining_nanos, Some(8_500_000_000));
        assert_eq!(reading.spend_limit_nanos, Some(10_000_000_000));
        assert_eq!(reading.total_spent_nanos, 1_500_000_000);
        assert_eq!(reading.total_requests, 12);
        assert_eq!(reading.total_tokens, 3456);

        // The rotation reached disk, not only memory. The token that bought
        // this balance is dead at the service; a daemon restarting on the old
        // one would be locked out until the contributor re-ran the ceremony.
        let on_disk = DaemonSettings::load(&s.store)
            .unwrap()
            .near_ai_session
            .unwrap();
        assert_eq!(on_disk.refresh_token, "rt_rotated_1");
        assert_eq!(
            on_disk.refresh_token,
            *stub.live_refresh.lock().unwrap().first().unwrap(),
            "the stored token must be the one the service still honours"
        );
        // The first exchange is what learns the expiry: the OAuth finish
        // supplies none.
        assert!(on_disk.refresh_token_expires_at.is_some());
        assert_eq!(
            s.settings
                .lock()
                .unwrap()
                .near_ai_session
                .as_ref()
                .unwrap()
                .refresh_token,
            "rt_rotated_1",
            "the running daemon must not keep offering the retired token"
        );

        // The refresh token authenticated the exchange and nothing else; the
        // access token authenticated both management calls. Sending either to
        // the other's route is a 401 upstream.
        let bearers = stub.bearers.lock().unwrap().clone();
        assert_eq!(
            bearers,
            vec![
                ("/v1/users/me/access-tokens".to_string(), "rt_first".into()),
                ("/v1/organizations".to_string(), "jwt_1".into()),
                (
                    "/v1/organizations/org-1/usage/balance".to_string(),
                    "jwt_1".into()
                ),
            ],
            "{bearers:?}"
        );

        // A second read inside the TTL is served from the cache rather than
        // spending another rotation.
        assert!(matches!(read_with(&s, &api).await, BalanceReport::Known(_)));
        assert_eq!(*stub.exchanges.lock().unwrap(), 1);
    }

    #[tokio::test]
    async fn a_refused_refresh_is_a_named_state_that_neither_clears_nor_zeroes() {
        let s = shared();
        // A token the stub does not honour: a session seven days old, or one
        // revoked from the contributor's own account page.
        with_session(&s, "rt_stale");
        let stub = stub(one_org(), a_balance()).await;
        let api = CloudApi::for_test(&stub.origin).unwrap();

        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "session_expired");
        // Not zero, and not a number at all.
        let body = report.to_value();
        assert!(body["remaining_nanos"].is_null(), "{body}");
        assert!(body["total_spent_nanos"].is_null(), "{body}");
        assert!(body["observed_at"].is_null(), "{body}");

        // And the stored session survives. A service answering 401 for its own
        // reasons must not be able to delete a contributor's credential; the
        // recovery is to re-run the ceremony, which overwrites both records
        // without a forget first.
        assert_eq!(
            DaemonSettings::load(&s.store)
                .unwrap()
                .near_ai_session
                .unwrap()
                .refresh_token,
            "rt_stale"
        );
    }

    #[tokio::test]
    async fn an_account_with_no_organization_is_named_rather_than_reported_as_empty() {
        let s = shared();
        with_session(&s, "rt_first");
        // `get_or_create_oauth_user` creates the organization best-effort:
        // a failure only logs and does not fail the signup, so this account
        // is real.
        let empty = serde_json::json!({"organizations": [], "total": 0, "limit": 50, "offset": 0});
        let stub = stub(empty, a_balance()).await;
        let api = CloudApi::for_test(&stub.origin).unwrap();

        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "no_organization");
        assert!(report.to_value()["remaining_nanos"].is_null());
    }

    #[tokio::test]
    async fn an_organization_with_no_ceiling_reports_nothing_rather_than_zero() {
        let s = shared();
        with_session(&s, "rt_first");
        let mut balance = a_balance();
        // Nullable in the service's own schema. An organization with no credit
        // limit configured has no remaining balance to report, and saying "$0"
        // would tell the contributor they are out of credit.
        balance["remaining"] = serde_json::Value::Null;
        balance["spend_limit"] = serde_json::Value::Null;
        let stub = stub(one_org(), balance).await;
        let api = CloudApi::for_test(&stub.origin).unwrap();

        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "known");
        let body = report.to_value();
        assert!(body["remaining_nanos"].is_null(), "{body}");
        assert!(body["spend_limit_nanos"].is_null(), "{body}");
        // What the service did report is still reported.
        assert_eq!(body["total_spent_nanos"], 1_500_000_000i64);
    }

    #[tokio::test]
    async fn no_session_is_its_own_state_and_never_reaches_the_network() {
        let s = shared();
        let stub = stub(one_org(), a_balance()).await;
        let api = CloudApi::for_test(&stub.origin).unwrap();

        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "no_session");
        assert!(
            stub.bearers.lock().unwrap().is_empty(),
            "a daemon with nothing stored must not call the service"
        );
        assert!(report.to_value()["remaining_nanos"].is_null());
    }

    #[tokio::test]
    async fn a_service_that_does_not_answer_is_unavailable_rather_than_a_figure() {
        let s = shared();
        with_session(&s, "rt_first");
        // Bound and immediately dropped: a port nothing is listening on.
        let dead = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            drop(listener);
            format!("http://{address}")
        };
        let api = CloudApi::for_test(&dead).unwrap();
        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "unavailable");
        assert!(report.to_value()["remaining_nanos"].is_null());
        // Unreachable is not spent: the token stays put.
        assert_eq!(
            DaemonSettings::load(&s.store)
                .unwrap()
                .near_ai_session
                .unwrap()
                .refresh_token,
            "rt_first"
        );
    }

    #[tokio::test]
    async fn an_aged_access_token_is_re_exchanged_once_and_a_second_refusal_is_the_session() {
        let s = shared();
        with_session(&s, "rt_first");
        // Management routes reject every access token. The first 401 is
        // indistinguishable from an access token that simply aged out, so one
        // exchange and one retry is owed; the second 401 is the session.
        let refused: Arc<Mutex<u32>> = Default::default();
        let counter = refused.clone();
        let tokens = Arc::new(Mutex::new(vec!["rt_first".to_string()]));
        let held = tokens.clone();
        let router = Router::new()
            .route(
                "/v1/users/me/access-tokens",
                axum::routing::post(move |request: Request| {
                    let held = held.clone();
                    async move {
                        let offered = bearer(&request);
                        let mut live = held.lock().unwrap();
                        if !live.contains(&offered) {
                            return (
                                axum::http::StatusCode::UNAUTHORIZED,
                                Json(serde_json::json!({})),
                            )
                                .into_response();
                        }
                        live.clear();
                        live.push("rt_second".into());
                        Json(serde_json::json!({
                            "access_token": "jwt", "refresh_token": "rt_second",
                            "refresh_token_expiration": "2026-09-15T00:00:00Z",
                        }))
                        .into_response()
                    }
                }),
            )
            .route(
                "/v1/organizations",
                get(move || {
                    let counter = counter.clone();
                    async move {
                        *counter.lock().unwrap() += 1;
                        (
                            axum::http::StatusCode::UNAUTHORIZED,
                            Json(serde_json::json!({})),
                        )
                    }
                }),
            );
        let api = CloudApi::for_test(&spawn(router).await).unwrap();

        let report = read_with(&s, &api).await;
        assert_eq!(report.state(), "session_expired");
        assert_eq!(
            *refused.lock().unwrap(),
            2,
            "exactly one retry: none would misreport an aged access token, \
             and a loop would spend rotations against a service saying no"
        );
    }

    #[test]
    fn a_report_never_carries_a_number_it_does_not_have() {
        // Every non-`known` state carries the numeric keys as explicit nulls.
        // Absent would be ambiguous between "this daemon is too old to know"
        // and "this daemon knows it does not know".
        for report in [
            BalanceReport::NoSession,
            BalanceReport::SessionExpired,
            BalanceReport::NoOrganization,
            BalanceReport::Unavailable,
        ] {
            let body = report.to_value();
            for key in [
                "remaining_nanos",
                "spend_limit_nanos",
                "total_spent_nanos",
                "total_requests",
                "total_tokens",
                "observed_at",
            ] {
                assert!(body.get(key).is_some(), "{key} absent in {body}");
                assert!(body[key].is_null(), "{key} is {body}");
            }
            assert_eq!(body["currency"], "USD");
            assert_eq!(body["scale"], 9);
        }
    }
}
