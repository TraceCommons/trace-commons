//! The payout-link IPC flow against a stub commons
//! (`crate::payout_link::test_support`) and a stub NEAR AI cloud-api on
//! 127.0.0.1. Every org, token and name here is synthetic.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{Value, json};

use super::*;
use crate::payout_link::test_support::{self as ingest, Server as Ingest};

const SESSION: &str = "tcn1_dGVuYW50.account";
/// Every access token the stub cloud mints carries this, so a leak anywhere
/// is one substring search.
const TOKEN_MARK: &str = "SENTINEL-near-ai-access";
const NAME_MARK: &str = "NAME-SENTINEL";

const ORG_A: &str = "0a0a0a0a-1111-4222-8333-444444444444";
const ORG_B: &str = "0b0b0b0b-1111-4222-8333-444444444444";
const ORG_C: &str = "0c0c0c0c-1111-4222-8333-444444444444";
const ORG_D: &str = "0d0d0d0d-1111-4222-8333-444444444444";

fn org(id: &str, name: &str, role: &str, active: bool) -> Value {
    json!({
        "id": id, "name": name, "description": null, "role": role,
        "is_active": active, "created_at": "2026-10-01T00:00:00Z",
    })
}

fn users_me(orgs: Vec<Value>) -> Value {
    json!({
        "id": "synthetic-user", "email": "synthetic@example.invalid",
        "username": "synthetic", "is_active": true, "auth_provider": "github",
        "organizations": orgs, "workspaces": [],
    })
}

struct CloudState {
    users_me: Mutex<Value>,
    rotations: Mutex<u32>,
    /// The bearer each `/users/me` read presented, in order.
    me_bearers: Mutex<Vec<String>>,
}

struct Cloud {
    api: CloudApi,
    state: Arc<CloudState>,
}

fn bearer(headers: &HeaderMap) -> String {
    headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .unwrap_or_default()
        .to_owned()
}

async fn refresh(State(state): State<Arc<CloudState>>) -> (StatusCode, Json<Value>) {
    let mut rotations = state.rotations.lock().unwrap();
    *rotations += 1;
    let n = *rotations;
    (
        StatusCode::OK,
        Json(json!({
            "access_token": format!("{TOKEN_MARK}-{n}"),
            "refresh_token": format!("rt-synthetic-rotated-{n}"),
            "refresh_token_expiration": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        })),
    )
}

async fn me(State(state): State<Arc<CloudState>>, headers: HeaderMap) -> Json<Value> {
    state.me_bearers.lock().unwrap().push(bearer(&headers));
    Json(state.users_me.lock().unwrap().clone())
}

impl Cloud {
    async fn start(orgs: Vec<Value>) -> Self {
        let state = Arc::new(CloudState {
            users_me: Mutex::new(users_me(orgs)),
            rotations: Mutex::new(0),
            me_bearers: Mutex::new(Vec::new()),
        });
        let router = Router::new()
            .route("/v1/users/me/access-tokens", post(refresh))
            .route("/v1/users/me", get(me))
            .with_state(Arc::clone(&state));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api =
            CloudApi::for_test(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        Self { api, state }
    }

    fn set_orgs(&self, orgs: Vec<Value>) {
        *self.state.users_me.lock().unwrap() = users_me(orgs);
    }

    fn me_reads(&self) -> Vec<String> {
        self.state.me_bearers.lock().unwrap().clone()
    }
}

struct Fixture {
    shared: DaemonShared,
    dir: tempfile::TempDir,
    ingest: Ingest,
}

impl Fixture {
    /// Enrolled against the stub commons, signed in to the account, with a
    /// NEAR AI session when `near_ai` is set.
    async fn new(near_ai: bool) -> Self {
        let ingest = ingest::spawn().await;
        let (dir, store) = crate::config::tests_support::temp_store();
        let shared = DaemonShared::load(store).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest.base.clone();
        shared.store.save_config(&cfg).unwrap();
        let session = crate::account_auth::AccountSession {
            access_token: SESSION.to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        shared
            .store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        if near_ai {
            let now = Utc::now();
            let mut settings = shared.settings.lock().unwrap();
            settings.near_ai_session = Some(NearAiSession {
                refresh_token: "rt-synthetic-payout".into(),
                refresh_token_expires_at: Some(now + chrono::Duration::hours(1)),
                stored_at: now,
                user_agent: "Mozilla/5.0 Test".into(),
            });
            settings.save_for_test(&shared.store).unwrap();
        }
        Self {
            shared,
            dir,
            ingest,
        }
    }

    fn seen(&self) -> Vec<ingest::Seen> {
        self.ingest.seen.lock().unwrap().clone()
    }

    /// Every byte this daemon wrote under its state directory.
    fn files(&self) -> String {
        let mut all = String::new();
        let mut stack = vec![self.dir.path().to_path_buf()];
        while let Some(path) = stack.pop() {
            if path.is_dir() {
                for entry in std::fs::read_dir(&path).unwrap() {
                    stack.push(entry.unwrap().path());
                }
            } else if let Ok(bytes) = std::fs::read(&path) {
                all.push_str(&String::from_utf8_lossy(&bytes));
            }
        }
        all
    }

    fn audit_rows(&self) -> Vec<(String, Option<String>)> {
        audit::load(&self.shared.store)
            .unwrap()
            .into_iter()
            .map(|entry| (entry.action, entry.detail))
            .collect()
    }
}

fn req(method: &str, params: Value) -> Request {
    Request {
        id: 7,
        method: method.to_string(),
        params,
    }
}

fn ok(response: Response) -> Value {
    if let Some(error) = &response.error {
        panic!("expected success, got {}", error.message);
    }
    response.result.expect("a result")
}

fn refusal(response: Response) -> String {
    response.error.expect("a refusal").message
}

async fn list(f: &Fixture, cloud: &Cloud) -> Value {
    ok(orgs(&f.shared, &req("payout_link_orgs", json!({})), &cloud.api).await)
}

fn start_params(organization_id: &str, revision: &Value, version: i64) -> Value {
    json!({
        "rail_id": "near_ai",
        "organization_id": organization_id,
        "orgs_revision": revision,
        "expected_current_version": version,
    })
}

async fn start_link(f: &Fixture, cloud: &Cloud, params: Value) -> Response {
    start(&f.shared, &req("payout_link_start", params), &cloud.api).await
}

async fn confirm(f: &Fixture, params: Value) -> Response {
    super::super::ipc::handle_request_async(&f.shared, &req("payout_link_confirm", params)).await
}

fn confirm_link(organization_id: &str) -> Value {
    json!({
        "rail_id": "near_ai", "action": "link",
        "organization_id": organization_id, "confirmed": true,
    })
}

async fn status(f: &Fixture) -> Value {
    ok(
        super::super::ipc::handle_request_async(&f.shared, &req("payout_rails_status", json!({})))
            .await,
    )
}

/// List, start and confirm `organization_id`, as the app would.
async fn link(f: &Fixture, cloud: &Cloud, organization_id: &str) {
    let listed = list(f, cloud).await;
    let version = status(f).await["rails"][0]["version"].as_i64().unwrap();
    ok(start_link(
        f,
        cloud,
        start_params(organization_id, &listed["orgs_revision"], version),
    )
    .await);
    ok(confirm(f, confirm_link(organization_id)).await);
}

#[tokio::test]
async fn org_listing_keeps_only_owned_active_orgs_and_never_sends_names_to_ingest() {
    let cloud = Cloud::start(vec![
        org(ORG_A, &format!("Owned {NAME_MARK} A"), "owner", true),
        org(ORG_B, &format!("Admin {NAME_MARK} B"), "admin", true),
        org(ORG_C, &format!("Inactive {NAME_MARK} C"), "owner", false),
        org("not-a-uuid", &format!("Odd {NAME_MARK}"), "owner", true),
        org(ORG_D, &format!("Member {NAME_MARK} D"), "member", true),
    ])
    .await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;
    let organizations = listed["organizations"].as_array().unwrap();
    assert_eq!(organizations.len(), 1, "{listed}");
    assert_eq!(organizations[0]["id"], ORG_A);
    assert_eq!(organizations[0]["name"], format!("Owned {NAME_MARK} A"));
    assert_eq!(listed["orgs_revision"].as_str().unwrap().len(), 64);
    // Listing talks to NEAR AI only.
    assert!(f.seen().is_empty());

    ok(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await);
    ok(confirm(&f, confirm_link(ORG_A)).await);
    let seen = f.seen();
    assert_eq!(seen.len(), 2);
    for request in &seen {
        assert!(!request.body.contains(NAME_MARK), "{}", request.path);
        assert!(!request.body.contains("Owned"), "{}", request.path);
        for other in [ORG_B, ORG_C, ORG_D] {
            assert!(!request.body.contains(other), "{}", request.path);
        }
    }
    assert!(seen[0].body.contains(ORG_A));
    // Only the chosen org's name is kept on this device, for status after a
    // restart; the names of orgs not chosen are never written.
    let files = f.files();
    assert!(files.contains(&format!("Owned {NAME_MARK} A")));
    for other in ["Admin", "Inactive", "Odd", "Member", ORG_B, ORG_C, ORG_D] {
        assert!(!files.contains(other), "{other} written to disk");
    }
}

async fn status_of(shared: &DaemonShared) -> Value {
    ok(
        super::super::ipc::handle_request_async(shared, &req("payout_rails_status", json!({})))
            .await,
    )
}

#[tokio::test]
async fn the_linked_org_name_survives_a_restart_without_listing_again() {
    let cloud = Cloud::start(vec![
        org(ORG_A, &format!("First {NAME_MARK} A"), "owner", true),
        org(ORG_B, "Synthetic org B", "owner", true),
    ])
    .await;
    let f = Fixture::new(true).await;
    link(&f, &cloud, ORG_A).await;

    // Kept in its own file, 0600, beside the link record; never in the
    // link record itself.
    let path = f.shared.store.daemon_path(DAEMON_PAYOUT_ORG_NAMES_FILE);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "{mode:o}");
    }
    let link_record = f
        .shared
        .store
        .read_daemon_file(DAEMON_PAYOUT_LINK_FILE)
        .unwrap()
        .unwrap();
    assert!(!String::from_utf8_lossy(&link_record).contains(NAME_MARK));

    // A restarted daemon has listed nothing, and still shows the name.
    let reads = cloud.me_reads().len();
    let restarted = DaemonShared::load(f.shared.store.clone()).unwrap();
    let after = status_of(&restarted).await;
    assert_eq!(after["rails"][0]["recipient_ref_check"], "match");
    assert_eq!(
        after["rails"][0]["organization_name"],
        format!("First {NAME_MARK} A")
    );
    assert_eq!(cloud.me_reads().len(), reads, "status asks NEAR AI nothing");

    // Reading the list again refreshes the saved name (a rename), and still
    // keeps only the linked org's.
    cloud.set_orgs(vec![
        org(ORG_A, &format!("Renamed {NAME_MARK} A"), "owner", true),
        org(ORG_B, "Synthetic org B", "owner", true),
    ]);
    list(&f, &cloud).await;
    let restarted = DaemonShared::load(f.shared.store.clone()).unwrap();
    assert_eq!(
        status_of(&restarted).await["rails"][0]["organization_name"],
        format!("Renamed {NAME_MARK} A")
    );
    let saved = std::fs::read_to_string(&path).unwrap();
    assert!(!saved.contains("First"), "{saved}");
    assert!(!saved.contains("Synthetic org B"), "{saved}");
}

#[tokio::test]
async fn forgetting_near_ai_clears_the_saved_org_name() {
    let cloud = Cloud::start(vec![org(
        ORG_A,
        &format!("Forget {NAME_MARK} A"),
        "owner",
        true,
    )])
    .await;
    let f = Fixture::new(true).await;
    link(&f, &cloud, ORG_A).await;
    assert!(f.files().contains(NAME_MARK));

    let forgot = super::super::ipc::handle_request_async(
        &f.shared,
        &req("near_ai_credential_forget", json!({})),
    )
    .await;
    assert!(
        forgot.error.is_none(),
        "{:?}",
        forgot.error.map(|e| e.message)
    );

    assert!(
        !f.files().contains(NAME_MARK),
        "the name outlived the forget"
    );
    assert!(
        f.shared
            .store
            .read_daemon_file(DAEMON_PAYOUT_ORG_NAMES_FILE)
            .unwrap()
            .is_none()
    );
    // Nor does this process still show it from memory, or after a restart.
    assert_eq!(
        status(&f).await["rails"][0]["organization_name"],
        Value::Null
    );
    let restarted = DaemonShared::load(f.shared.store.clone()).unwrap();
    assert_eq!(
        status_of(&restarted).await["rails"][0]["organization_name"],
        Value::Null
    );
}

#[tokio::test]
async fn a_single_org_still_needs_a_confirm() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic only org", "owner", true)]).await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;
    assert_eq!(listed["organizations"].as_array().unwrap().len(), 1);

    // Nothing is preselected: a start must name the org.
    let mut no_org = start_params(ORG_A, &listed["orgs_revision"], 0);
    no_org.as_object_mut().unwrap().remove("organization_id");
    assert_eq!(
        refusal(start_link(&f, &cloud, no_org).await),
        "organization_id"
    );
    assert!(f.seen().is_empty());

    let started =
        ok(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await);
    assert_eq!(started["outcome"], "pending_confirmation");
    assert_eq!(started["organization_name"], "Synthetic only org");
    // Started is not linked: the commons was asked to start, nothing else.
    assert_eq!(f.seen().len(), 1);
    assert!(!f.ingest.stub.lock().unwrap().linked);

    let mut unconfirmed = confirm_link(ORG_A);
    unconfirmed.as_object_mut().unwrap().remove("confirmed");
    assert_eq!(
        refusal(confirm(&f, unconfirmed).await),
        ERR_CONFIRMATION_REQUIRED
    );
    let mut false_confirmed = confirm_link(ORG_A);
    false_confirmed["confirmed"] = json!(false);
    assert_eq!(
        refusal(confirm(&f, false_confirmed).await),
        ERR_CONFIRMATION_REQUIRED
    );
    assert_eq!(f.seen().len(), 1, "no confirm reached the commons");

    let confirmed = ok(confirm(&f, confirm_link(ORG_A)).await);
    assert_eq!(confirmed["outcome"], "confirmed");
    assert_eq!(confirmed["version"], 1);
    assert!(f.ingest.stub.lock().unwrap().linked);
}

#[tokio::test]
async fn start_sends_the_fresh_token_once_and_drops_it() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org", "owner", true)]).await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;
    let started =
        ok(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await);
    let confirmed = ok(confirm(&f, confirm_link(ORG_A)).await);
    let after = status(&f).await;

    // One exchange per NEAR AI read: the listing's token, then a fresh one
    // for the start, each spent once on /users/me.
    let first = format!("{TOKEN_MARK}-1");
    let fresh = format!("{TOKEN_MARK}-2");
    assert_eq!(cloud.me_reads(), vec![first.clone(), fresh.clone()]);

    let seen = f.seen();
    let carrying: Vec<_> = seen
        .iter()
        .filter(|request| request.body.contains(TOKEN_MARK))
        .collect();
    assert_eq!(carrying.len(), 1, "the token is sent exactly once");
    assert!(carrying[0].path.ends_with("/near_ai/link-requests"));
    let body: Value = serde_json::from_str(&carrying[0].body).unwrap();
    assert_eq!(body["proof"]["near_ai_access_token"], fresh);
    assert!(
        !carrying[0].body.contains(&first),
        "never the listing's token"
    );
    // The commons sees the account session, never a NEAR AI token, as bearer.
    assert!(seen.iter().all(|r| r.auth == format!("Bearer {SESSION}")));

    // Dropped: in no IPC answer, file or audit row.
    for answer in [&listed, &started, &confirmed, &after] {
        assert!(!answer.to_string().contains(TOKEN_MARK), "{answer}");
    }
    assert!(!f.files().contains(TOKEN_MARK));
    assert!(!format!("{:?}", f.audit_rows()).contains(TOKEN_MARK));
}

#[tokio::test]
async fn a_changed_org_list_refuses_the_confirm() {
    let cloud = Cloud::start(vec![
        org(ORG_A, "Synthetic org A", "owner", true),
        org(ORG_B, "Synthetic org B", "owner", true),
    ])
    .await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;

    // Renamed after it was shown: the start refuses before the commons.
    cloud.set_orgs(vec![
        org(ORG_A, "Synthetic org A renamed", "owner", true),
        org(ORG_B, "Synthetic org B", "owner", true),
    ]);
    assert_eq!(
        refusal(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await),
        ERR_ORGS_CHANGED
    );
    // Gone after it was shown: the same.
    cloud.set_orgs(vec![org(ORG_A, "Synthetic org A", "owner", true)]);
    assert_eq!(
        refusal(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await),
        ERR_ORGS_CHANGED
    );
    assert!(f.seen().is_empty(), "nothing reached the commons");

    // A confirm naming another org than the one started is refused, and
    // the started request is still the one that can be confirmed.
    let relisted = list(&f, &cloud).await;
    cloud.set_orgs(vec![
        org(ORG_A, "Synthetic org A", "owner", true),
        org(ORG_B, "Synthetic org B", "owner", true),
    ]);
    // The list changed again between relist and start.
    assert_eq!(
        refusal(
            start_link(
                &f,
                &cloud,
                start_params(ORG_A, &relisted["orgs_revision"], 0)
            )
            .await
        ),
        ERR_ORGS_CHANGED
    );
    let shown = list(&f, &cloud).await;
    ok(start_link(&f, &cloud, start_params(ORG_A, &shown["orgs_revision"], 0)).await);
    assert_eq!(refusal(confirm(&f, confirm_link(ORG_B)).await), ERR_CHANGED);
    let mut as_unlink = confirm_link(ORG_A);
    as_unlink["action"] = json!("unlink");
    assert_eq!(refusal(confirm(&f, as_unlink).await), ERR_CHANGED);
    assert_eq!(f.seen().len(), 1, "no confirm reached the commons");
    ok(confirm(&f, confirm_link(ORG_A)).await);
}

#[tokio::test]
async fn status_flags_a_recipient_ref_mismatch() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(true).await;

    let before = status(&f).await;
    assert_eq!(before["rails"][0]["linked"], false);
    assert_eq!(before["rails"][0]["recipient_ref_check"], Value::Null);
    assert_eq!(before["rails"][0]["organization_id"], Value::Null);

    link(&f, &cloud, ORG_A).await;
    let matched = status(&f).await;
    let rail = &matched["rails"][0];
    assert_eq!(rail["rail_id"], "near_ai");
    assert_eq!(rail["linked"], true);
    assert_eq!(rail["version"], 1);
    assert_eq!(rail["recipient_ref_check"], "match");
    assert_eq!(rail["organization_name"], "Synthetic org A");
    assert_eq!(rail["organization_id"], ORG_A);

    // The commons now reports another org's ref.
    f.ingest.stub.lock().unwrap().recipient_ref = Some(near_ai_recipient_ref(
        Uuid::new_v4(),
        Uuid::parse_str(ORG_A).unwrap(),
    ));
    let mismatched = status(&f).await;
    assert_eq!(mismatched["rails"][0]["recipient_ref_check"], "mismatch");
    assert_eq!(mismatched["rails"][0]["organization_id"], Value::Null);
    assert_eq!(
        mismatched["rails"][0]["organization_name"],
        Value::Null,
        "a name is shown only for the org the commons holds"
    );

    // Linked from another device: this one cannot tell.
    f.shared
        .store
        .write_daemon_file(DAEMON_PAYOUT_LINK_FILE, b"{}")
        .unwrap();
    let unknown = status(&f).await;
    assert_eq!(unknown["rails"][0]["recipient_ref_check"], "unknown");
    assert_eq!(unknown["rails"][0]["organization_id"], Value::Null);
}

#[tokio::test]
async fn unlink_starts_and_confirms_without_a_token_or_a_near_ai_call() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(true).await;
    link(&f, &cloud, ORG_A).await;
    let reads = cloud.me_reads().len();

    let started = ok(super::super::ipc::handle_request_async(
        &f.shared,
        &req(
            "payout_link_unlink",
            json!({"rail_id": "near_ai", "expected_current_version": 1}),
        ),
    )
    .await);
    assert_eq!(started["action"], "unlink");
    assert_eq!(started["outcome"], "pending_confirmation");
    let body: Value = serde_json::from_str(&f.seen().last().unwrap().body).unwrap();
    assert_eq!(body["action"], "unlink");
    assert!(body.get("proof").is_none_or(Value::is_null), "{body}");

    let confirmed = ok(confirm(
        &f,
        json!({"rail_id": "near_ai", "action": "unlink", "confirmed": true}),
    )
    .await);
    assert_eq!(confirmed["action"], "unlink");
    assert_eq!(confirmed["version"], 2);
    assert_eq!(cloud.me_reads().len(), reads, "unlink asks NEAR AI nothing");
    // The local record of the link is gone with it.
    let local = f
        .shared
        .store
        .read_daemon_file(DAEMON_PAYOUT_LINK_FILE)
        .unwrap()
        .unwrap();
    let local: Value = serde_json::from_slice(&local).unwrap();
    assert!(local["rails"].as_object().unwrap().is_empty(), "{local}");
    // And so is the org's saved name.
    assert!(!f.files().contains("Synthetic org A"));
}

#[tokio::test]
async fn a_wallet_only_account_is_sent_to_near_ai_sign_in() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(false).await;
    assert_eq!(
        refusal(orgs(&f.shared, &req("payout_link_orgs", json!({})), &cloud.api).await),
        ERR_NEAR_AI_SIGN_IN_REQUIRED
    );
    assert_eq!(
        refusal(start_link(&f, &cloud, start_params(ORG_A, &json!("a".repeat(64)), 0)).await),
        ERR_NEAR_AI_SIGN_IN_REQUIRED
    );
    assert!(cloud.me_reads().is_empty());
    assert!(f.seen().is_empty());
}

#[tokio::test]
async fn the_shown_version_is_required_and_passed_through() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;
    for bad in [json!(null), json!(-1), json!("3"), json!(1.5)] {
        let mut params = start_params(ORG_A, &listed["orgs_revision"], 0);
        params["expected_current_version"] = bad.clone();
        assert_eq!(
            refusal(start_link(&f, &cloud, params).await),
            "expected_current_version",
            "{bad}"
        );
    }
    assert_eq!(
        refusal(
            super::super::ipc::handle_request_async(
                &f.shared,
                &req("payout_link_unlink", json!({"rail_id": "near_ai"})),
            )
            .await
        ),
        "expected_current_version"
    );
    assert!(f.seen().is_empty());

    ok(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 3)).await);
    let body: Value = serde_json::from_str(&f.seen()[0].body).unwrap();
    assert_eq!(body["expected_current_version"], 3);
}

#[tokio::test]
async fn a_refused_confirm_keeps_the_commons_label_and_ends_the_request() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(true).await;
    let listed = list(&f, &cloud).await;
    ok(start_link(&f, &cloud, start_params(ORG_A, &listed["orgs_revision"], 0)).await);
    f.ingest.stub.lock().unwrap().confirm_reply =
        Some((StatusCode::CONFLICT, json!({"error": "version_conflict"})));
    assert_eq!(
        refusal(confirm(&f, confirm_link(ORG_A)).await),
        "version_conflict"
    );
    assert_eq!(
        refusal(confirm(&f, confirm_link(ORG_A)).await),
        ERR_START_REQUIRED
    );
    // A start refusal from the commons' own NEAR AI check keeps its label.
    f.ingest.stub.lock().unwrap().start_reply =
        Some((StatusCode::FORBIDDEN, json!({"error": "org_not_owned"})));
    let relisted = list(&f, &cloud).await;
    assert_eq!(
        refusal(
            start_link(
                &f,
                &cloud,
                start_params(ORG_A, &relisted["orgs_revision"], 0)
            )
            .await
        ),
        "org_not_owned"
    );
}

#[tokio::test]
async fn audit_rows_are_labels_only() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic org A", "owner", true)]).await;
    let f = Fixture::new(true).await;
    link(&f, &cloud, ORG_A).await;
    ok(super::super::ipc::handle_request_async(
        &f.shared,
        &req(
            "payout_link_unlink",
            json!({"rail_id": "near_ai", "expected_current_version": 1}),
        ),
    )
    .await);
    ok(confirm(
        &f,
        json!({"rail_id": "near_ai", "action": "unlink", "confirmed": true}),
    )
    .await);
    let rows = f.audit_rows();
    assert_eq!(
        rows,
        vec![
            (AUDIT_LINK_STARTED.to_string(), None),
            (AUDIT_LINK_CONFIRMED.to_string(), None),
            (AUDIT_UNLINK_STARTED.to_string(), None),
            (AUDIT_UNLINK_CONFIRMED.to_string(), None),
        ]
    );
}

#[tokio::test]
async fn every_method_requires_an_account_session() {
    let f = Fixture::new(true).await;
    f.shared
        .store
        .remove_daemon_file(crate::config::ACCOUNT_SESSION_FILE)
        .unwrap();
    for (method, params) in [
        ("payout_rails_status", json!({})),
        ("payout_link_orgs", json!({})),
        (
            "payout_link_start",
            start_params(ORG_A, &json!("a".repeat(64)), 0),
        ),
        ("payout_link_confirm", confirm_link(ORG_A)),
        (
            "payout_link_unlink",
            json!({"rail_id": "near_ai", "expected_current_version": 0}),
        ),
    ] {
        assert!(
            super::super::ipc::METHODS.contains(&method),
            "{method} must be advertised"
        );
        let response =
            super::super::ipc::handle_request_async(&f.shared, &req(method, params)).await;
        assert_eq!(refusal(response), ERR_ACCOUNT_SESSION_REQUIRED, "{method}");
        // The synchronous dispatcher names the async requirement.
        let sync = super::super::ipc::handle_request(&f.shared, &req(method, json!({})));
        assert_eq!(refusal(sync), "payout-link-requires-async", "{method}");
    }
    assert!(f.seen().is_empty());
}

#[tokio::test]
async fn credit_monitoring_resolves_only_a_fresh_verified_payout_recipient() {
    let cloud = Cloud::start(vec![org(ORG_A, "Synthetic payout org", "owner", true)]).await;
    let f = Fixture::new(true).await;
    assert!(
        f.shared
            .settings
            .lock()
            .unwrap()
            .near_ai_inference
            .is_none()
    );
    assert_eq!(
        verified_near_ai_recipient(&f.shared, &cloud.api)
            .await
            .unwrap(),
        None
    );
    link(&f, &cloud, ORG_A).await;
    assert_eq!(
        verified_near_ai_recipient(&f.shared, &cloud.api)
            .await
            .unwrap(),
        Some(ORG_A.to_owned())
    );
    f.ingest.stub.lock().unwrap().recipient_ref = Some(near_ai_recipient_ref(
        Uuid::new_v4(),
        Uuid::parse_str(ORG_A).unwrap(),
    ));
    assert!(
        verified_near_ai_recipient(&f.shared, &cloud.api)
            .await
            .is_err(),
        "a stale local recipient cannot identify the server's payout destination"
    );
    f.ingest.stub.lock().unwrap().linked = false;
    assert_eq!(
        verified_near_ai_recipient(&f.shared, &cloud.api)
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn status_passes_each_stream_through_and_leaves_none_out() {
    let f = Fixture::new(true).await;
    let without = status(&f).await;
    assert!(
        without["rails"][0].get("streams").is_none(),
        "no streams keeps the pre-referral shape: {without}"
    );

    // Shared with the macOS app's test, which decodes the same bytes: the
    // amounts are decimal strings, as the commons sends them.
    let streams: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../tests/fixtures/payout/streams.json"
    ))
    .unwrap();
    let typed: Vec<trace_commons_protocol::payout_link::PayoutStreamStatus> =
        serde_json::from_value(streams.clone()).expect("the commons's own stream shape");
    assert_eq!(typed[0].owed, "700");
    f.ingest.stub.lock().unwrap().streams = Some(streams.clone());
    let with = status(&f).await;
    assert_eq!(with["rails"][0]["streams"], streams);
}
