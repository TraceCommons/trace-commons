// INTEGRATION: register inside balance.rs with
// #[cfg(test)] #[path = "balance_regression_tests.rs"] mod regression_tests;
// This child uses the existing private read_with/cache seams through crate
// paths. All credentials and balances are synthetic; HTTP is loopback-only.

use crate::config::tests_support::temp_store;
use crate::daemon::ipc::{DaemonShared, Request};
use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::nearai_credential::balance::{BalanceReport, cache, read_with};
use crate::daemon::nearai_credential::handle_forget;
use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use chrono::Utc;
use serde_json::{Value, json};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;
use tokio::time::timeout;

const DEADLINE: Duration = Duration::from_secs(15);
const FIRST_BALANCE: i64 = 101;
const REPLACEMENT_BALANCE: i64 = 202;
const SELECTED_BALANCE: i64 = 707;
const UNSELECTED_BALANCE: i64 = 909;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Account {
    First,
    Replacement,
}

impl Account {
    fn label(self) -> &'static str {
        match self {
            Self::First => "first",
            Self::Replacement => "replacement",
        }
    }
}

struct Fixture {
    shared: DaemonShared,
    _dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let (dir, store) = temp_store();
        Self {
            shared: DaemonShared::load(store).expect("load isolated daemon"),
            _dir: dir,
        }
    }

    // Install a completed connection in both stores, without a browser or
    // proxy. The pending HTTP response always belongs to the prior account.
    fn connect(&self, account: Account, organization: &str) {
        let label = account.label();
        let now = Utc::now();
        let mut settings = self.shared.settings.lock().expect("fixture settings lock");
        settings.near_ai_session = Some(NearAiSession {
            refresh_token: format!("rt-synthetic-{label}"),
            refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
            stored_at: now,
            user_agent: "Mozilla/5.0 Test".into(),
        });
        settings.near_ai_inference = Some(NearAiInferenceCredential {
            key: format!("sk-synthetic-{label}"),
            key_id: format!("key-{label}"),
            key_prefix: "sk-synthetic".to_owned(),
            organization_id: organization.to_owned(),
            workspace_id: format!("workspace-{label}"),
            minted_at: now,
        });
        settings
            .save_for_test(&self.shared.store)
            .expect("save synthetic connection");
    }

    fn forget(&self) {
        let response = handle_forget(
            &self.shared,
            &Request {
                id: 1,
                method: "near_ai_credential_forget".to_owned(),
                params: json!({}),
            },
        );
        assert!(response.error.is_none(), "forget must succeed");
        assert!(
            self.shared
                .settings
                .lock()
                .unwrap()
                .near_ai_session
                .is_none()
        );
    }

    async fn read(&self, cloud: &LocalCloud) -> BalanceReport {
        timeout(DEADLINE, read_with(&self.shared, &cloud.api))
            .await
            .expect("balance read timed out")
    }
}

struct ResponseGate {
    entered: oneshot::Sender<()>,
    release: oneshot::Receiver<()>,
}

struct HeldBalance {
    entered: oneshot::Receiver<()>,
    release: oneshot::Sender<()>,
}

impl HeldBalance {
    fn new() -> (ResponseGate, Self) {
        let (entered_tx, entered_rx) = oneshot::channel();
        let (release_tx, release_rx) = oneshot::channel();
        (
            ResponseGate {
                entered: entered_tx,
                release: release_rx,
            },
            Self {
                entered: entered_rx,
                release: release_tx,
            },
        )
    }

    async fn wait(self) -> oneshot::Sender<()> {
        timeout(DEADLINE, self.entered)
            .await
            .expect("balance did not reach the response barrier")
            .expect("balance response barrier closed early");
        self.release
    }
}

struct Tokens {
    account: Account,
    refresh: String,
    access: String,
    rotations: usize,
}

struct ServerState {
    tokens: Mutex<Vec<Tokens>>,
    first_listed: &'static str,
    held_balance: Mutex<Option<ResponseGate>>,
    held_refresh: Mutex<Option<ResponseGate>>,
    held_payout_orgs: Mutex<Option<ResponseGate>>,
    balance_requests: Mutex<Vec<(Account, String)>>,
}

struct LocalCloud {
    api: CloudApi,
    state: Arc<ServerState>,
    server: JoinHandle<()>,
}

impl LocalCloud {
    async fn start(first_listed: &'static str, gate: Option<ResponseGate>) -> Self {
        let state = Arc::new(ServerState {
            tokens: Mutex::new(
                [Account::First, Account::Replacement]
                    .into_iter()
                    .map(|account| Tokens {
                        account,
                        refresh: format!("rt-synthetic-{}", account.label()),
                        access: String::new(),
                        rotations: 0,
                    })
                    .collect(),
            ),
            first_listed,
            held_balance: Mutex::new(gate),
            held_refresh: Mutex::new(None),
            held_payout_orgs: Mutex::new(None),
            balance_requests: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/v1/users/me/access-tokens", post(refresh))
            .route("/v1/organizations", get(organizations))
            .route("/v1/users/me", get(payout_user))
            .route(
                "/v1/organizations/{organization}/usage/balance",
                get(balance),
            )
            .with_state(Arc::clone(&state));
        let listener = timeout(DEADLINE, tokio::net::TcpListener::bind("127.0.0.1:0"))
            .await
            .expect("listener timed out")
            .expect("bind loopback listener");
        let address = listener.local_addr().expect("read loopback address");
        let api = CloudApi::for_test(&format!("http://{address}"))
            .expect("construct loopback-only client");
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .await
                .expect("serve synthetic balances");
        });
        Self { api, state, server }
    }
}

impl Drop for LocalCloud {
    fn drop(&mut self) {
        self.server.abort();
    }
}

fn bearer(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
}

fn access_account(state: &ServerState, headers: &HeaderMap) -> Option<Account> {
    let tokens = state.tokens.lock().expect("synthetic token lock");
    tokens
        .iter()
        .find(|entry| !entry.access.is_empty() && bearer(headers) == Some(entry.access.as_str()))
        .map(|entry| entry.account)
}

async fn refresh(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let answer = {
        let mut tokens = state.tokens.lock().expect("synthetic token lock");
        let Some(entry) = tokens
            .iter_mut()
            .find(|entry| bearer(&headers) == Some(entry.refresh.as_str()))
        else {
            return (StatusCode::UNAUTHORIZED, Json(json!({"error": "refresh"})));
        };
        entry.rotations += 1;
        entry.refresh = format!("rt-synthetic-{}-{}", entry.account.label(), entry.rotations);
        entry.access = format!(
            "access-synthetic-{}-{}",
            entry.account.label(),
            entry.rotations
        );
        (
            StatusCode::OK,
            Json(json!({
                "access_token": entry.access, "refresh_token": entry.refresh,
                "refresh_token_expiration": Utc::now() + chrono::Duration::hours(1)
            })),
        )
    };
    let gate = state.held_refresh.lock().unwrap().take();
    if let Some(gate) = gate {
        gate.entered
            .send(())
            .expect("notify refresh response barrier");
        timeout(DEADLINE, gate.release)
            .await
            .expect("refresh response was not released")
            .expect("refresh controller disappeared");
    }
    answer
}

async fn organizations(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(account) = access_account(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "access"})));
    };
    let first = match account {
        Account::First => state.first_listed,
        Account::Replacement => "org-replacement",
    };
    (
        StatusCode::OK,
        Json(json!({
            "organizations": [
                {"id": first, "is_active": true},
                {"id": "org-selected", "is_active": true}
            ], "total": 2, "limit": 50, "offset": 0
        })),
    )
}

async fn balance(
    State(state): State<Arc<ServerState>>,
    Path(organization): Path<String>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(account) = access_account(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "access"})));
    };
    let remaining = match (account, organization.as_str()) {
        (Account::First, "org-first") => FIRST_BALANCE,
        (Account::First, "org-selected") => SELECTED_BALANCE,
        (Account::First, "org-unselected") => UNSELECTED_BALANCE,
        (Account::Replacement, "org-replacement") => REPLACEMENT_BALANCE,
        _ => {
            return (
                StatusCode::FORBIDDEN,
                Json(json!({"error": "organization"})),
            );
        }
    };
    state
        .balance_requests
        .lock()
        .expect("request record lock")
        .push((account, organization));
    let gate = state.held_balance.lock().expect("balance gate lock").take();
    if let Some(gate) = gate {
        gate.entered
            .send(())
            .expect("notify balance response barrier");
        timeout(DEADLINE, gate.release)
            .await
            .expect("held balance was never released")
            .expect("balance controller disappeared");
    }
    (
        StatusCode::OK,
        Json(json!({
            "total_spent": 0, "remaining": remaining, "spend_limit": 1000,
            "total_requests": 0, "total_tokens": 0
        })),
    )
}

fn assert_known(report: BalanceReport, expected: i64) {
    let BalanceReport::Known(reading) = report else {
        panic!(
            "expected a current synthetic balance, got {}",
            report.state()
        );
    };
    assert_eq!(reading.remaining_nanos, Some(expected));
}

#[tokio::test]
async fn cached_balance_cannot_survive_forget_when_invalidation_is_busy() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    let cloud = LocalCloud::start("org-first", None).await;
    assert_known(fixture.read(&cloud).await, FIRST_BALANCE);

    // Keep the already-populated cache locked while the real forget handler
    // calls invalidate. Its try_lock must fail, so cache eviction cannot be
    // the only check protecting the next read.
    let held = timeout(DEADLINE, cache().lock())
        .await
        .expect("cache lock timed out");
    assert!(
        cache().try_lock().is_err(),
        "force the busy invalidation branch"
    );
    fixture.forget();
    drop(held);

    assert!(
        matches!(fixture.read(&cloud).await, BalanceReport::NoSession),
        "a forgotten account must not return its still-fresh cached balance"
    );
}

#[tokio::test]
async fn pending_balance_cannot_publish_or_cache_after_forget() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    let (gate, held) = HeldBalance::new();
    let cloud = LocalCloud::start("org-first", Some(gate)).await;
    let (report, ()) = timeout(DEADLINE, async {
        tokio::join!(fixture.read(&cloud), async {
            let release = held.wait().await;
            fixture.forget();
            release.send(()).expect("release forgotten account balance");
        })
    })
    .await
    .expect("pending balance forget race timed out");

    assert!(
        !matches!(report, BalanceReport::Known(_)),
        "a response arriving after forget must not publish the old balance"
    );
    assert!(
        matches!(fixture.read(&cloud).await, BalanceReport::NoSession),
        "a stale response must not leave a usable cached balance"
    );
}

#[tokio::test]
async fn pending_balance_cannot_publish_or_cache_for_a_replacement_account() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    let (gate, held) = HeldBalance::new();
    let cloud = LocalCloud::start("org-first", Some(gate)).await;
    let (report, ()) = timeout(DEADLINE, async {
        tokio::join!(fixture.read(&cloud), async {
            let release = held.wait().await;
            fixture.connect(Account::Replacement, "org-replacement");
            release.send(()).expect("release replaced account balance");
        })
    })
    .await
    .expect("pending balance replacement race timed out");

    assert!(
        !matches!(report, BalanceReport::Known(_)),
        "a response for the prior account must not publish after replacement"
    );
    assert_known(fixture.read(&cloud).await, REPLACEMENT_BALANCE);
    assert!(
        cloud
            .state
            .balance_requests
            .lock()
            .unwrap()
            .iter()
            .any(|(account, org)| *account == Account::Replacement && org == "org-replacement"),
        "the replacement balance must come from its own authenticated HTTP request"
    );
}

#[tokio::test]
async fn balance_uses_the_stored_keys_organization_instead_of_the_first_listed() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-selected");
    let cloud = LocalCloud::start("org-unselected", None).await;

    assert_known(fixture.read(&cloud).await, SELECTED_BALANCE);
    let requests = cloud
        .state
        .balance_requests
        .lock()
        .expect("request record lock");
    assert!(requests.iter().any(|(_, org)| org == "org-selected"));
    assert!(
        !requests.iter().any(|(_, org)| org == "org-unselected"),
        "organization ordering must not redirect the credential's balance read"
    );
}

const PAYOUT_ORG: &str = "a0c18c96-6093-49f5-bb6f-6092ef0630b9";

impl Fixture {
    fn allow_payout_listing(&self) {
        self.shared
            .store
            .save_config(&crate::commands::unenrolled_preview_config())
            .expect("save payout configuration");
        let session = crate::account_auth::AccountSession {
            access_token: "synthetic-commons-account-session".into(),
            expires_at: Utc::now() + chrono::Duration::hours(6),
            account_id: "synthetic-account".into(),
        };
        self.shared
            .store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .expect("save Commons account session");
    }

    async fn payout_organizations(&self, cloud: &LocalCloud) -> crate::daemon::ipc::Response {
        crate::daemon::payout_link::orgs(
            &self.shared,
            &Request {
                id: 2,
                method: "payout_link_orgs".into(),
                params: json!({}),
            },
            &cloud.api,
        )
        .await
    }
}

async fn payout_user(
    State(state): State<Arc<ServerState>>,
    headers: HeaderMap,
) -> (StatusCode, Json<Value>) {
    let Some(account) = access_account(&state, &headers) else {
        return (StatusCode::UNAUTHORIZED, Json(json!({"error": "access"})));
    };
    let gate = state.held_payout_orgs.lock().unwrap().take();
    if let Some(gate) = gate {
        gate.entered.send(()).expect("notify payout org response");
        timeout(DEADLINE, gate.release)
            .await
            .expect("payout org response was not released")
            .expect("payout org controller disappeared");
    }
    (
        StatusCode::OK,
        Json(json!({
            "id": "synthetic-user", "email": "synthetic@example.invalid",
            "username": "synthetic", "is_active": true, "auth_provider": "github",
            "organizations": [{
                "id": PAYOUT_ORG, "name": account.label(), "role": "owner", "is_active": true
            }],
            "workspaces": []
        })),
    )
}

#[tokio::test]
async fn concurrent_balance_and_payout_org_listing_share_the_management_session() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    fixture.allow_payout_listing();
    let cloud = LocalCloud::start("org-first", None).await;
    let (gate, held) = HeldBalance::new();
    *cloud.state.held_refresh.lock().unwrap() = Some(gate);

    let (balance, organizations) = timeout(DEADLINE, async {
        tokio::join!(fixture.read(&cloud), async {
            // Balance owns the refresh lock but its rotated session has not
            // reached the client. Poll the org lookup once at that exact
            // boundary: it must wait on the shared coordinator BEFORE taking
            // a session snapshot, not take the soon-retired token and wait
            // only on the refresh mutex. No scheduling delay is assumed.
            let release = held.wait().await;
            let mut listing = Box::pin(fixture.payout_organizations(&cloud));
            std::future::poll_fn(|cx| {
                assert!(std::future::Future::poll(listing.as_mut(), cx).is_pending());
                std::task::Poll::Ready(())
            })
            .await;
            release.send(()).expect("release concurrent refresh");
            listing.await
        })
    })
    .await
    .expect("concurrent management reads timed out");
    assert_known(balance, FIRST_BALANCE);
    assert!(organizations.error.is_none(), "{:?}", organizations.error);
    assert_eq!(
        organizations.result.unwrap()["organizations"][0]["id"],
        PAYOUT_ORG
    );
    {
        let held = cache().lock().await;
        let entry = held.get(fixture.shared.store.dir()).unwrap();
        assert!(
            entry.balance.is_some(),
            "same-login rotation keeps the observed balance"
        );
        assert!(
            entry.access_token.is_none(),
            "the fresh payout proof is not cached"
        );
    }
    assert_known(fixture.read(&cloud).await, FIRST_BALANCE);
    assert_eq!(
        cloud.state.balance_requests.lock().unwrap().len(),
        1,
        "the cached balance survives the payout's session rotation"
    );
    let tokens = cloud.state.tokens.lock().unwrap();
    let first = tokens
        .iter()
        .find(|entry| entry.account == Account::First)
        .unwrap();
    assert_eq!(
        first.rotations, 2,
        "payout retains its freshly exchanged proof token"
    );
}

#[tokio::test]
async fn pending_payout_org_listing_cannot_succeed_after_forget() {
    payout_org_listing_after_connection_change(false).await;
}

#[tokio::test]
async fn pending_payout_org_listing_cannot_succeed_after_relogin() {
    payout_org_listing_after_connection_change(true).await;
}

async fn payout_org_listing_after_connection_change(relogin: bool) {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    fixture.allow_payout_listing();
    let cloud = LocalCloud::start("org-first", None).await;
    let (gate, held) = HeldBalance::new();
    *cloud.state.held_payout_orgs.lock().unwrap() = Some(gate);
    let (response, ()) = timeout(DEADLINE, async {
        tokio::join!(fixture.payout_organizations(&cloud), async {
            let release = held.wait().await;
            if relogin {
                fixture.connect(Account::Replacement, "org-replacement");
            } else {
                fixture.forget();
            }
            release.send(()).expect("release stale payout org response");
        })
    })
    .await
    .expect("stale payout org read timed out");
    assert!(
        response.result.is_none(),
        "old account organizations must not escape"
    );
    assert_eq!(
        response
            .error
            .expect("a stale connection is refused")
            .message,
        if relogin {
            "payout-link-near-ai-session-changed"
        } else {
            "payout-link-near-ai-sign-in-required"
        }
    );
}

#[tokio::test]
async fn fresh_management_callback_keeps_token_paired_with_its_retained_session() {
    let fixture = Fixture::new();
    fixture.connect(Account::First, "org-first");
    let cloud = LocalCloud::start("org-first", None).await;
    super::with_fresh_management_session(&fixture.shared, &cloud.api, |tokens, retained| {
        // A controlled replacement at callback entry is later than the
        // exchange, before the callback can take any local snapshot. The
        // supplied retained session must still identify the old token.
        fixture.connect(Account::Replacement, "org-replacement");
        let current = fixture
            .shared
            .settings
            .lock()
            .unwrap()
            .near_ai_session
            .clone()
            .unwrap();
        let api = &cloud.api;
        async move {
            let organizations = api.owned_organizations(&tokens).await?;
            assert_eq!(organizations[0].name, "first");
            assert_eq!(retained.refresh_token, "rt-synthetic-first-1");
            assert!(
                current != retained,
                "a replacement login must differ from the token's supplied session"
            );
            Ok(())
        }
    })
    .await
    .expect("run synthetic token/session pairing callback");
}
