//! Choosing which organization Private AI spends from, against a stub that
//! keeps the deployed service's rules: the refresh token rotates on every
//! exchange, a key is minted in a workspace of the named organization, and a
//! revoke is a DELETE answered 204.

use super::*;
use crate::config::tests_support::temp_store;
use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};
use axum::{
    Json, Router,
    extract::{Path, Request},
    http::StatusCode,
    response::IntoResponse,
    routing::{delete, get, post},
};
use serde_json::Value;
use std::sync::{Arc, Mutex};

struct Fixture {
    shared: DaemonShared,
    _dir: tempfile::TempDir,
}

fn key(id: &str, organization: &str) -> NearAiInferenceCredential {
    NearAiInferenceCredential {
        key: format!("sk-synthetic-{id}"),
        key_id: id.into(),
        key_prefix: "sk-syn".into(),
        organization_id: organization.into(),
        workspace_id: format!("ws-{organization}"),
        minted_at: Utc::now(),
    }
}

impl Fixture {
    /// A signed-in install whose key was minted into `organization`.
    fn signed_in(organization: &str) -> Self {
        let (dir, store) = temp_store();
        let shared = DaemonShared::load(store).unwrap();
        {
            let mut settings = shared.settings.lock().unwrap();
            settings.near_ai_inference = Some(key("key-old", organization));
            settings.near_ai_session = Some(NearAiSession {
                refresh_token: "rt_first".into(),
                refresh_token_expires_at: None,
                stored_at: Utc::now(),
                user_agent: "Mozilla/5.0 Test".into(),
            });
            settings.save_for_test(&shared.store).unwrap();
        }
        Self { shared, _dir: dir }
    }

    fn stored_key(&self) -> NearAiInferenceCredential {
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&self.shared.store)
            .unwrap()
            .near_ai_inference
            .unwrap()
    }

    fn memory_key(&self) -> NearAiInferenceCredential {
        self.shared
            .settings
            .lock()
            .unwrap()
            .near_ai_inference
            .clone()
            .unwrap()
    }
}

#[derive(Default)]
struct Service {
    requests: Mutex<Vec<String>>,
    live_refresh: Mutex<Vec<String>>,
    rotations: Mutex<u32>,
    organizations: Mutex<Value>,
    organizations_status: Mutex<Option<StatusCode>>,
    mint_status: Mutex<Option<StatusCode>>,
    revoke_status: Mutex<Option<StatusCode>>,
    /// The NEAR AI user `/v1/users/me` names; `None` answers 500.
    user: Mutex<Option<String>>,
}

impl Service {
    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
    fn deletes(&self) -> Vec<String> {
        self.requests()
            .into_iter()
            .filter(|line| line.starts_with("DELETE"))
            .collect()
    }
    fn mints(&self) -> usize {
        self.requests()
            .iter()
            .filter(|line| line.starts_with("POST /v1/workspaces/"))
            .count()
    }
}

fn org(id: &str, name: &str) -> Value {
    serde_json::json!({
        "id": id, "name": name, "role": "owner", "is_active": true,
        "owner_id": "u", "settings": {},
        "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
    })
}

fn listing(organizations: Vec<Value>) -> Value {
    let total = organizations.len();
    serde_json::json!({"organizations": organizations, "total": total, "limit": 100, "offset": 0})
}

fn two_orgs() -> Value {
    listing(vec![
        org("org-a", "First example org"),
        org("org-b", "Second example org"),
    ])
}

async fn start(organizations: Value) -> (CloudApi, Arc<Service>) {
    let service = Arc::new(Service::default());
    *service.organizations.lock().unwrap() = organizations;
    *service.user.lock().unwrap() = Some(USER_A.into());
    service.live_refresh.lock().unwrap().push("rt_first".into());
    let state = service.clone();
    let router = Router::new()
        .route(
            "/v1/users/me/access-tokens",
            post({
                let state = state.clone();
                move |request: Request| {
                    let state = state.clone();
                    async move {
                        let offered = request
                            .headers()
                            .get("authorization")
                            .and_then(|v| v.to_str().ok())
                            .unwrap_or_default()
                            .trim_start_matches("Bearer ")
                            .to_string();
                        let mut live = state.live_refresh.lock().unwrap();
                        if !live.contains(&offered) {
                            return StatusCode::UNAUTHORIZED.into_response();
                        }
                        let mut rotations = state.rotations.lock().unwrap();
                        *rotations += 1;
                        let rotated = format!("rt_rotated_{rotations}");
                        live.clear();
                        live.push(rotated.clone());
                        Json(serde_json::json!({
                            "access_token": format!("jwt_{rotations}"),
                            "refresh_token": rotated,
                            "refresh_token_expiration": "2027-01-01T00:00:00Z",
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/v1/users/me",
            get({
                let state = state.clone();
                move || {
                    let state = state.clone();
                    async move {
                        match state.user.lock().unwrap().clone() {
                            Some(id) => Json(serde_json::json!({
                                "id": id, "email": "someone@example.invalid",
                                "username": "someone", "is_active": true,
                                "auth_provider": "github", "organizations": [],
                            }))
                            .into_response(),
                            None => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
                        }
                    }
                }
            }),
        )
        .route(
            "/v1/organizations",
            get({
                let state = state.clone();
                move || {
                    let state = state.clone();
                    async move {
                        if let Some(status) = *state.organizations_status.lock().unwrap() {
                            return status.into_response();
                        }
                        Json(state.organizations.lock().unwrap().clone()).into_response()
                    }
                }
            }),
        )
        .route(
            "/v1/organizations/{org}/workspaces",
            get(|Path(org): Path<String>| async move {
                Json(serde_json::json!({
                    "workspaces": [{
                        "id": format!("ws-{org}"), "name": "default", "organization_id": org,
                        "created_by_user_id": "u", "is_active": true,
                        "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
                    }],
                    "total": 1, "limit": 50, "offset": 0,
                }))
            }),
        )
        .route(
            "/v1/workspaces/{workspace}/api-keys",
            post({
                let state = state.clone();
                move |Path(workspace): Path<String>| {
                    let state = state.clone();
                    async move {
                        if let Some(status) = *state.mint_status.lock().unwrap() {
                            return status.into_response();
                        }
                        Json(serde_json::json!({
                            "id": format!("key-new-{workspace}"), "key": "sk-minted-synthetic",
                            "key_prefix": "sk-min", "workspace_id": workspace,
                            "created_by_user_id": "u", "created_at": "2026-09-08T00:00:00Z",
                            "is_active": true,
                        }))
                        .into_response()
                    }
                }
            }),
        )
        .route(
            "/v1/workspaces/{workspace}/api-keys/{key}",
            delete({
                let state = state.clone();
                move || {
                    let state = state.clone();
                    async move {
                        state
                            .revoke_status
                            .lock()
                            .unwrap()
                            .unwrap_or(StatusCode::NO_CONTENT)
                    }
                }
            }),
        )
        .layer(axum::middleware::from_fn(
            move |request: Request, next: axum::middleware::Next| {
                let state = state.clone();
                async move {
                    state.requests.lock().unwrap().push(format!(
                        "{} {}",
                        request.method(),
                        request.uri().path()
                    ));
                    next.run(request).await
                }
            },
        ));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    (
        CloudApi::for_test(&format!("http://{address}")).unwrap(),
        service,
    )
}

// ---- Listing and the prompt ----

/// The common case in production: one organization. Nothing new appears --
/// no prompt, no picker.
#[tokio::test]
async fn a_single_organization_account_gets_no_prompt_and_no_picker() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(listing(vec![org("org-a", "Only example org")])).await;
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["state"], "listed");
    assert_eq!(value["organizations"].as_array().unwrap().len(), 1);
    assert_eq!(value["multiple"], false);
    assert_eq!(value["prompt"], false);
    assert_eq!(value["current_organization_id"], "org-a");
}

#[tokio::test]
async fn several_organizations_and_no_choice_prompt_once_and_mark_the_current_one() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["state"], "listed");
    assert_eq!(value["multiple"], true);
    assert_eq!(value["explicit"], false);
    assert_eq!(value["prompt"], true);
    let listed = value["organizations"].as_array().unwrap();
    assert_eq!(listed[0]["id"], "org-a");
    assert_eq!(listed[0]["current"], true);
    assert_eq!(listed[1]["current"], false);
    assert_eq!(listed[1]["name"], "Second example org");
    // The prompt's sentence names where it is spending now.
    assert!(
        value["view"]["prompt_line"]
            .as_str()
            .unwrap()
            .contains("First example org"),
        "{value}"
    );
}

/// An unknown count is not "multiple": a failed listing shows nothing new.
#[tokio::test]
async fn a_failed_listing_shows_no_prompt() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.organizations_status.lock().unwrap() = Some(StatusCode::INTERNAL_SERVER_ERROR);
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["state"], "unavailable");
    assert_eq!(value["prompt"], false);
    assert_eq!(value["multiple"], false);
    assert!(value["organizations"].as_array().unwrap().is_empty());
    // And the stored key is untouched by a read.
    assert_eq!(fixture.stored_key().key_id, "key-old");
}

#[tokio::test]
async fn no_session_lists_nothing_and_never_reaches_the_network() {
    let (dir, store) = temp_store();
    let shared = DaemonShared::load(store).unwrap();
    let (api, service) = start(two_orgs()).await;
    let value = organizations_with(&shared, &api, false).await.to_value();
    assert_eq!(value["state"], "no_session");
    assert_eq!(value["prompt"], false);
    assert!(service.requests().is_empty());
    drop(dir);
}

/// A listing rides the balance's session handling: the rotated refresh
/// token is persisted, so the next read still works.
#[tokio::test]
async fn listing_persists_the_rotation_it_spent() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    organizations_with(&fixture.shared, &api, true).await;
    let session =
        crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&fixture.shared.store)
            .unwrap()
            .near_ai_session
            .unwrap();
    assert_eq!(session.refresh_token, "rt_rotated_1");
    // A second, fresh read inside the reuse window spends no new exchange.
    organizations_with(&fixture.shared, &api, true).await;
    assert_eq!(*service.rotations.lock().unwrap(), 1);
}

// ---- Switching ----

#[tokio::test]
async fn switching_mints_in_the_chosen_organization_moves_the_key_then_revokes_the_old_one() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    let outcome = select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap();
    assert!(outcome.switched);
    assert!(!outcome.revoke_pending);

    let stored = fixture.stored_key();
    assert_eq!(stored.organization_id, "org-b");
    assert_eq!(stored.key_id, "key-new-ws-org-b");
    // The running daemon took the new key too: that is what moves the proxy.
    assert_eq!(fixture.memory_key().key_id, "key-new-ws-org-b");

    // Mint first, revoke last, and only the old key.
    let requests = service.requests();
    let mint = requests
        .iter()
        .position(|line| line == "POST /v1/workspaces/ws-org-b/api-keys")
        .expect("minted in the chosen organization");
    let revoke = requests
        .iter()
        .position(|line| line == "DELETE /v1/workspaces/ws-org-a/api-keys/key-old")
        .expect("the old key is revoked");
    assert!(mint < revoke, "{requests:?}");
    assert_eq!(service.deletes().len(), 1, "{requests:?}");

    // The choice is remembered, so the prompt goes.
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["explicit"], true);
    assert_eq!(value["prompt"], false);
    assert_eq!(value["current_organization_id"], "org-b");
    assert_eq!(value["revoke_pending"], false);
}

/// Minting is the only step that can fail before anything changes, and when
/// it does, nothing does.
#[tokio::test]
async fn a_failed_mint_changes_nothing() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.mint_status.lock().unwrap() = Some(StatusCode::FORBIDDEN);
    let error = select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, LABEL_SPEND_ORG_REFUSED);
    assert_eq!(fixture.stored_key().key_id, "key-old");
    assert_eq!(fixture.memory_key().key_id, "key-old");
    assert!(service.deletes().is_empty(), "{:?}", service.requests());
    assert_eq!(chosen_organization(&fixture.shared.store), None);
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["prompt"], true);
    assert_eq!(value["revoke_pending"], false);
}

/// The old key could not be revoked: the switch stands on the new key, and
/// the revoke is recorded and retried rather than forgotten.
#[tokio::test]
async fn a_failed_revoke_keeps_the_new_key_and_is_retried() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.revoke_status.lock().unwrap() = Some(StatusCode::INTERNAL_SERVER_ERROR);
    let outcome = select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap();
    assert!(outcome.switched);
    assert!(outcome.revoke_pending);
    assert_eq!(fixture.stored_key().organization_id, "org-b");
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["revoke_pending"], true);
    assert_eq!(value["prompt"], false);

    // The service recovers; the next listing finishes the job.
    *service.revoke_status.lock().unwrap() = None;
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["revoke_pending"], false);
    let deletes = service.deletes();
    assert!(
        deletes
            .iter()
            .all(|line| line == "DELETE /v1/workspaces/ws-org-a/api-keys/key-old"),
        "only the old key is ever revoked: {deletes:?}"
    );
    // Nothing is retried once it is done.
    organizations_with(&fixture.shared, &api, true).await;
    assert_eq!(service.deletes().len(), deletes.len());
}

/// Choosing the organization already in use mints nothing; it only makes the
/// choice explicit, which is what silences the prompt.
#[tokio::test]
async fn choosing_the_current_organization_mints_nothing_and_ends_the_prompt() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    let outcome = select_with(&fixture.shared, &api, "org-a", Duration::ZERO)
        .await
        .unwrap();
    assert!(!outcome.switched);
    assert_eq!(service.mints(), 0);
    assert!(service.deletes().is_empty());
    assert_eq!(fixture.stored_key().key_id, "key-old");
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["explicit"], true);
    assert_eq!(value["prompt"], false);
}

#[tokio::test]
async fn an_organization_the_account_does_not_have_is_refused_and_nothing_changes() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    let error = select_with(&fixture.shared, &api, "org-c", Duration::ZERO)
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, LABEL_SPEND_ORG_UNKNOWN);
    assert_eq!(service.mints(), 0);
    assert_eq!(fixture.stored_key().key_id, "key-old");
}

#[tokio::test]
async fn a_target_that_is_not_an_id_never_reaches_the_network() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    for target in ["", "../org-b", "org b", &"x".repeat(200)] {
        assert_eq!(
            select_with(&fixture.shared, &api, target, Duration::ZERO)
                .await
                .unwrap_err()
                .to_string(),
            LABEL_SPEND_ORG_INVALID
        );
    }
    assert!(service.requests().is_empty());
}

#[tokio::test]
async fn switching_without_a_key_is_refused() {
    let (dir, store) = temp_store();
    let shared = DaemonShared::load(store).unwrap();
    let (api, _) = start(two_orgs()).await;
    assert_eq!(
        select_with(&shared, &api, "org-b", Duration::ZERO)
            .await
            .unwrap_err()
            .to_string(),
        LABEL_SPEND_ORG_NO_SESSION
    );
    drop(dir);
}

/// A sign-in that replaces the key while a switch waits for its new key to
/// propagate wins: the switch publishes nothing, revokes only the key it
/// minted, and leaves the sign-in's key and the key it replaced alone.
#[tokio::test]
async fn a_key_replaced_during_the_switch_is_left_alone_and_the_minted_key_is_revoked() {
    let fixture = Arc::new(Fixture::signed_in("org-a"));
    let (api, service) = start(two_orgs()).await;
    let api = Arc::new(api);
    let switching = {
        let fixture = fixture.clone();
        let api = api.clone();
        tokio::spawn(async move {
            select_with(&fixture.shared, &api, "org-b", Duration::from_millis(400))
                .await
                .map_err(|error| error.to_string())
        })
    };
    // Wait until the switch has minted, then replace the key underneath it.
    // Bounded generously: the switch reads the identity first, and a loaded
    // machine is slow; the loop ends as soon as the mint is seen.
    for _ in 0..2000 {
        if service.mints() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    assert_eq!(service.mints(), 1);
    {
        let mut settings = crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(
            &fixture.shared.store,
        )
        .unwrap();
        settings.near_ai_inference = Some(key("key-sign-in", "org-a"));
        settings.save_for_test(&fixture.shared.store).unwrap();
        let mut memory = fixture.shared.settings.lock().unwrap();
        memory.near_ai_inference = settings.near_ai_inference.clone();
        memory.near_ai_session = settings.near_ai_session.clone();
        memory.cloud_credentials = settings.cloud_credentials.clone();
    }
    assert_eq!(
        switching.await.unwrap().unwrap_err(),
        LABEL_SPEND_ORG_UNAVAILABLE
    );
    assert_eq!(fixture.stored_key().key_id, "key-sign-in");
    assert_eq!(
        service.deletes(),
        ["DELETE /v1/workspaces/ws-org-b/api-keys/key-new-ws-org-b"]
    );
    assert!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .is_empty()
    );
    assert_eq!(chosen_organization(&fixture.shared.store), None);
}

/// The sweep never revokes the key in use, whatever the record says. A
/// pending entry naming the active key -- a switch whose publish failed, say
/// -- is dropped instead.
#[tokio::test]
async fn the_sweep_never_revokes_the_active_key() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    record_pending(
        &fixture.shared.store,
        &[PendingRevocation::new("ws-org-a", "key-old")],
    )
    .unwrap();
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["revoke_pending"], false);
    assert!(service.deletes().is_empty(), "{:?}", service.requests());
    assert!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .is_empty()
    );
}

/// The record is non-secret metadata in its own file, and an unreadable one
/// fails closed: no choice is believed, no revoke is attempted.
#[test]
fn an_unreadable_record_is_no_choice_and_no_revocation() {
    let (_dir, store) = temp_store();
    store
        .write_daemon_file(SPEND_ORG_FILE, b"{not json")
        .unwrap();
    assert_eq!(chosen_organization(&store), None);
    assert!(load(&store).is_err());
}

#[test]
fn the_record_holds_ids_only_and_drops_any_that_are_not_ids() {
    let (_dir, store) = temp_store();
    store
        .write_daemon_file(
            SPEND_ORG_FILE,
            serde_json::json!({
                "chosen_organization_id": "../evil",
                "pending_revocations": [
                    {"workspace_id": "ws-1", "key_id": "key-1", "recorded_at": "2026-10-23T00:00:00Z"},
                    {"workspace_id": "ws/2", "key_id": "key-2", "recorded_at": "2026-10-23T00:00:00Z"},
                ],
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();
    let record = load(&store).unwrap();
    assert_eq!(record.chosen_organization_id, None);
    assert_eq!(record.pending_revocations.len(), 1);
    assert_eq!(record.pending_revocations[0].key_id, "key-1");
}

/// The words the macOS app's `SpendOrganizations.Words` requires, all of
/// them, non-empty. The app draws nothing if one is missing, so a key
/// renamed here would silently remove the picker and the prompt.
#[test]
fn the_view_carries_every_word_the_mac_app_reads() {
    let view = crate::private_inference_copy::spend_org_view(Some("Example org"));
    let object = view.as_object().unwrap();
    let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "failed",
            "picker",
            "prompt_action",
            "prompt_action_accessibility",
            "prompt_dismiss",
            "prompt_dismiss_accessibility",
            "prompt_line",
            "prompt_title",
            "refused",
            "revoke_pending",
            "session_expired",
            "switching",
            "title",
            "what",
        ]
    );
    assert!(
        object
            .values()
            .all(|value| value.as_str().is_some_and(|text| !text.is_empty()))
    );
    assert!(
        view["prompt_line"]
            .as_str()
            .unwrap()
            .contains("Example org")
    );
    assert!(
        view["prompt_dismiss_accessibility"]
            .as_str()
            .unwrap()
            .contains("Example org")
    );
    let unnamed = crate::private_inference_copy::spend_org_view(None);
    assert!(!unnamed["prompt_line"].as_str().unwrap().contains('{'));
    assert!(
        !unnamed["prompt_dismiss_accessibility"]
            .as_str()
            .unwrap()
            .contains('{')
    );
}

/// Forgetting the sign-in forgets this record too. The sweep needs the
/// account's session, and a different account signed in afterwards would be
/// refused every revoke forever; the forget sentence already says keys are
/// not revoked at NEAR AI by forgetting.
#[tokio::test]
async fn forgetting_the_sign_in_forgets_the_choice_and_the_pending_revocations() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.revoke_status.lock().unwrap() = Some(StatusCode::INTERNAL_SERVER_ERROR);
    let outcome = select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap();
    assert!(outcome.revoke_pending);
    assert_eq!(
        chosen_organization(&fixture.shared.store).as_deref(),
        Some("org-b")
    );
    assert!(super::super::ceremony::forget(&fixture.shared.store).unwrap());
    assert!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .is_empty()
    );
    assert_eq!(chosen_organization(&fixture.shared.store), None);
}

// ---- Owner ruling on #1590: once an organization is chosen, no prompt ----

/// A choice recorded for this sign-in ends the prompt for good, even when
/// the key now spends from another organization (a sign-in that could not
/// use the remembered one, say). Only forgetting the sign-in brings it back.
#[tokio::test]
async fn a_recorded_choice_suppresses_the_prompt_even_when_the_key_is_elsewhere() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    remember_choice(&fixture.shared.store, "org-b").unwrap();
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["multiple"], true);
    assert_eq!(value["current_organization_id"], "org-a");
    assert_eq!(
        value["explicit"], false,
        "the key is not where the choice was"
    );
    assert_eq!(value["chosen"], true);
    assert_eq!(value["prompt"], false);
}

#[tokio::test]
async fn a_recorded_choice_of_the_current_organization_suppresses_the_prompt() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    remember_choice(&fixture.shared.store, "org-a").unwrap();
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["explicit"], true);
    assert_eq!(value["chosen"], true);
    assert_eq!(value["prompt"], false);
}

#[tokio::test]
async fn no_recorded_choice_with_several_organizations_prompts() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    let value = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(value["chosen"], false);
    assert_eq!(value["prompt"], true);
}

/// Choosing ends the prompt on the very next read, inside the listing's
/// cache window: the choice is read from the record on every call.
#[tokio::test]
async fn choosing_ends_the_prompt_on_the_next_read_without_waiting_out_the_cache() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    let before = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(before["prompt"], true);
    select_with(&fixture.shared, &api, "org-a", Duration::ZERO)
        .await
        .unwrap();
    let after = organizations_with(&fixture.shared, &api, false)
        .await
        .to_value();
    assert_eq!(after["prompt"], false);
}

// ---- Owner rule 2026-11-02: a sign-in to another account switches organization ----

/// What the sign-in ceremony does once it has stored a key minted in
/// `minted`: the key replaces the stored one, then what belonged to another
/// account is forgotten. `listed`: the account lists the remembered
/// organization, active or not.
fn sign_in_stored(fixture: &Fixture, remembered: Option<&str>, listed: bool, minted: &str) {
    {
        let mut settings = fixture.shared.settings.lock().unwrap();
        settings.near_ai_inference = Some(key("key-new", minted));
        settings.save_for_test(&fixture.shared.store).unwrap();
    }
    forget_other_account(&fixture.shared.store, remembered, listed);
}

#[test]
fn the_account_changed_only_when_the_remembered_organization_is_not_listed() {
    assert!(
        !account_changed(None, false),
        "no choice, nothing to forget"
    );
    assert!(!account_changed(Some("org-a"), true), "the same account");
    assert!(account_changed(Some("org-old"), false), "another account");
}

/// Signing in again to the same account lands in the remembered
/// organization, and the choice and its revocations stand: no prompt.
#[tokio::test]
async fn a_sign_in_to_the_same_account_keeps_the_choice() {
    let fixture = Fixture::signed_in("org-b");
    let (api, _) = start(two_orgs()).await;
    remember_choice(&fixture.shared.store, "org-b").unwrap();
    record_pending(
        &fixture.shared.store,
        &[PendingRevocation::new("ws-org-a", "key-older")],
    )
    .unwrap();
    sign_in_stored(&fixture, Some("org-b"), true, "org-b");
    assert_eq!(
        chosen_organization(&fixture.shared.store).as_deref(),
        Some("org-b")
    );
    assert_eq!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .len(),
        1
    );
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["chosen"], true);
    assert_eq!(value["prompt"], false);
}

/// The chosen organization is inactive for now, so the key lands in
/// another one of the same account. The account still lists it: the
/// choice and the revocations stand.
#[tokio::test]
async fn an_inactive_chosen_organization_in_the_same_account_keeps_the_choice() {
    let fixture = Fixture::signed_in("org-b");
    remember_choice(&fixture.shared.store, "org-b").unwrap();
    record_pending(
        &fixture.shared.store,
        &[PendingRevocation::new("ws-org-a", "key-older")],
    )
    .unwrap();
    sign_in_stored(&fixture, Some("org-b"), true, "org-a");
    let record = load(&fixture.shared.store).unwrap();
    assert_eq!(record.chosen_organization_id.as_deref(), Some("org-b"));
    assert_eq!(record.pending_revocations.len(), 1);
}

/// Another account with one organization: the key is there, the old choice
/// is forgotten, and nothing is asked.
#[tokio::test]
async fn a_sign_in_to_another_account_with_one_organization_uses_it() {
    let fixture = Fixture::signed_in("org-old");
    let (api, _) = start(listing(vec![org("org-a", "Only example org")])).await;
    remember_choice(&fixture.shared.store, "org-old").unwrap();
    sign_in_stored(&fixture, Some("org-old"), false, "org-a");
    assert_eq!(chosen_organization(&fixture.shared.store), None);
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["current_organization_id"], "org-a");
    assert_eq!(value["multiple"], false);
    assert_eq!(value["prompt"], false);
}

/// Another account with several organizations: the key is in the first
/// active one, the old choice is forgotten, and the chooser asks. The old
/// account's revocations are kept, since they may still be revocable; the
/// first sweep sends each once, and one the service answers `403` for --
/// another account's key -- is dropped, so `revoke_pending` clears.
#[tokio::test]
async fn a_sign_in_to_another_account_with_several_organizations_prompts() {
    let fixture = Fixture::signed_in("org-old");
    let (api, service) = start(two_orgs()).await;
    *service.revoke_status.lock().unwrap() = Some(StatusCode::FORBIDDEN);
    remember_choice(&fixture.shared.store, "org-old").unwrap();
    record_pending(
        &fixture.shared.store,
        &[PendingRevocation::new("ws-org-old", "key-older")],
    )
    .unwrap();
    sign_in_stored(&fixture, Some("org-old"), false, "org-a");
    let record = load(&fixture.shared.store).unwrap();
    assert_eq!(record.chosen_organization_id, None);
    assert_eq!(record.pending_revocations.len(), 1, "kept for the sweep");
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["current_organization_id"], "org-a");
    assert_eq!(value["chosen"], false);
    assert_eq!(value["prompt"], true);
    assert_eq!(value["revoke_pending"], false);
    assert!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .is_empty()
    );
    assert_eq!(
        service
            .requests()
            .iter()
            .filter(|line| line.starts_with("DELETE "))
            .count(),
        1
    );
}

/// A revoke refused for another reason stays pending and is retried.
#[tokio::test]
async fn a_revoke_refused_for_another_reason_stays_pending() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.revoke_status.lock().unwrap() = Some(StatusCode::INTERNAL_SERVER_ERROR);
    record_pending(
        &fixture.shared.store,
        &[PendingRevocation::new("ws-org-b", "key-older")],
    )
    .unwrap();
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["revoke_pending"], true);
    assert_eq!(
        load(&fixture.shared.store)
            .unwrap()
            .pending_revocations
            .len(),
        1
    );
}

/// A listing page that did not carry every organization cannot show the
/// remembered one is absent: it counts as listed.
#[tokio::test]
async fn a_short_listing_page_counts_the_remembered_organization_as_listed() {
    let session = crate::daemon::nearai_credential::loopback::SessionTokens {
        access_token: "session-jwt".into(),
        refresh_token: "rt_example".into(),
    };
    let mut page = listing(vec![org("org-a", "First example org")]);
    page["total"] = serde_json::json!(250);
    let (api, _) = start(page).await;
    let (_, listed) = api
        .mint_inference_key_listing(
            &session,
            "trace-commons-test",
            OrganizationChoice::Prefer("org-far"),
        )
        .await
        .unwrap();
    assert!(listed);
}

/// The sign-in's mint reports whether the account lists the remembered
/// organization at all: inactive is listed, absent is not.
#[tokio::test]
async fn the_mint_reports_whether_the_remembered_organization_is_listed() {
    let session = crate::daemon::nearai_credential::loopback::SessionTokens {
        access_token: "session-jwt".into(),
        refresh_token: "rt_example".into(),
    };
    let mut inactive = org("org-b", "Paused example org");
    inactive["is_active"] = serde_json::json!(false);
    let (api, _) = start(listing(vec![org("org-a", "First example org"), inactive])).await;
    let (minted, listed) = api
        .mint_inference_key_listing(
            &session,
            "trace-commons-test",
            OrganizationChoice::Prefer("org-b"),
        )
        .await
        .unwrap();
    assert_eq!(minted.organization_id, "org-a");
    assert!(listed, "an inactive organization is still the account's");
    let (_, listed) = api
        .mint_inference_key_listing(
            &session,
            "trace-commons-test",
            OrganizationChoice::Prefer("org-old"),
        )
        .await
        .unwrap();
    assert!(!listed, "another account does not list it");
}

/// A new GitHub or Google NEAR AI account with no active organization fails
/// the mint with its own control name, which the sign-in's line carries.
#[tokio::test]
async fn an_account_without_an_organization_is_labelled() {
    let (api, _) = start(listing(vec![])).await;
    let session = crate::daemon::nearai_credential::loopback::SessionTokens {
        access_token: "session-jwt".into(),
        refresh_token: "rt_example".into(),
    };
    let error = api
        .mint_inference_key_in(
            &session,
            "trace-commons-test",
            OrganizationChoice::Prefer("org-old"),
        )
        .await
        .map_err(|error| error.context(super::super::ceremony::FailedStep::Mint))
        .unwrap_err();
    let fields = super::super::ceremony::failure_fields(&error, "github").unwrap();
    assert_eq!(fields[0].1, "mint");
    assert_eq!(fields[1].1, "near_ai_credential_no_organization");
    let no_workspace = anyhow!("near_ai_credential_no_workspace")
        .context(super::super::ceremony::FailedStep::Mint);
    let fields = super::super::ceremony::failure_fields(&no_workspace, "google").unwrap();
    assert_eq!(fields[1].1, "near_ai_credential_no_workspace");
}

// ---- Spec 2026-11-02 B8 / F5: revocations are tagged with the identity that recorded them ----

const USER_A: &str = "user-a-6f1d2c3b";
const USER_B: &str = "user-b-9c0d1e2f";

fn tagged(store: &ConfigStore, workspace: &str, key: &str, user: &str) -> PendingRevocation {
    let mut entry = PendingRevocation::new(workspace, key);
    entry.identity = Some(identity_digest(store, user).unwrap());
    entry
}

/// The digest is local: salted per install, never the id itself, and the
/// same for the same user on this install.
#[test]
fn the_identity_digest_is_salted_and_never_the_id() {
    let (_dir, store) = temp_store();
    let (_other_dir, other) = temp_store();
    let a = identity_digest(&store, USER_A).unwrap();
    assert_eq!(a, identity_digest(&store, USER_A).unwrap());
    assert_ne!(a, identity_digest(&store, USER_B).unwrap());
    assert_ne!(
        a,
        identity_digest(&other, USER_A).unwrap(),
        "salted per install"
    );
    assert!(!a.contains(USER_A));
    assert!(a.starts_with("sha256:"));
}

/// A legacy record, written before tags, reads as untagged; a tagged entry
/// round-trips.
#[test]
fn a_legacy_entry_reads_as_untagged() {
    let (_dir, store) = temp_store();
    store
        .write_daemon_file(
            SPEND_ORG_FILE,
            br#"{"pending_revocations":[{"workspace_id":"ws-1","key_id":"key-1","recorded_at":"2026-11-01T00:00:00Z"}]}"#,
        )
        .unwrap();
    assert_eq!(load(&store).unwrap().pending_revocations[0].identity, None);
    record_pending(&store, &[tagged(&store, "ws-2", "key-2", USER_A)]).unwrap();
    let record = load(&store).unwrap();
    assert_eq!(record.pending_revocations.len(), 2);
    assert!(record.pending_revocations[1].identity.is_some());
}

/// A switch tags both of the keys it records with the identity that minted
/// them.
#[tokio::test]
async fn a_switch_tags_its_revocations_with_the_current_identity() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    *service.revoke_status.lock().unwrap() = Some(StatusCode::INTERNAL_SERVER_ERROR);
    select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap();
    let digest = identity_digest(&fixture.shared.store, USER_A).unwrap();
    let pending = load(&fixture.shared.store).unwrap().pending_revocations;
    assert!(!pending.is_empty());
    assert!(
        pending
            .iter()
            .all(|entry| entry.identity.as_deref() == Some(digest.as_str())),
        "{pending:?}"
    );
}

/// The sweep revokes only entries of the current identity and untagged
/// legacy ones. Another identity's entries stay parked: never sent with
/// this session, never dropped, and not reported pending.
#[tokio::test]
async fn the_sweep_parks_another_identitys_revocations() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    let store = &fixture.shared.store;
    record_pending(
        store,
        &[
            tagged(store, "ws-mine", "key-mine", USER_A),
            tagged(store, "ws-theirs", "key-theirs", USER_B),
            PendingRevocation::new("ws-legacy", "key-legacy"),
        ],
    )
    .unwrap();
    let value = organizations_with(&fixture.shared, &api, true)
        .await
        .to_value();
    assert_eq!(value["revoke_pending"], false);
    let deletes = service.deletes();
    assert!(deletes.contains(&"DELETE /v1/workspaces/ws-mine/api-keys/key-mine".to_string()));
    assert!(deletes.contains(&"DELETE /v1/workspaces/ws-legacy/api-keys/key-legacy".to_string()));
    assert!(
        !deletes.iter().any(|line| line.contains("theirs")),
        "{deletes:?}"
    );
    let pending = load(store).unwrap().pending_revocations;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key_id, "key-theirs", "parked, not dropped");

    // Signed in again as the other identity, the parked entry is revoked.
    *service.user.lock().unwrap() = Some(USER_B.into());
    organizations_with(&fixture.shared, &api, true).await;
    assert!(load(store).unwrap().pending_revocations.is_empty());
    assert!(
        service
            .deletes()
            .contains(&"DELETE /v1/workspaces/ws-theirs/api-keys/key-theirs".to_string())
    );
}

/// When the identity cannot be read, tagged entries wait for a sweep that
/// can read it; untagged legacy entries are still swept.
#[tokio::test]
async fn an_unread_identity_parks_every_tagged_entry() {
    let fixture = Fixture::signed_in("org-a");
    let (api, service) = start(two_orgs()).await;
    let store = &fixture.shared.store;
    record_pending(
        store,
        &[
            tagged(store, "ws-mine", "key-mine", USER_A),
            PendingRevocation::new("ws-legacy", "key-legacy"),
        ],
    )
    .unwrap();
    *service.user.lock().unwrap() = None;
    organizations_with(&fixture.shared, &api, true).await;
    let deletes = service.deletes();
    assert_eq!(
        deletes,
        ["DELETE /v1/workspaces/ws-legacy/api-keys/key-legacy"]
    );
    let pending = load(store).unwrap().pending_revocations;
    assert_eq!(pending.len(), 1);
    assert_eq!(pending[0].key_id, "key-mine");
}

/// Parked entries of another identity do not count toward the switch
/// limit: they cannot jam this identity's switches.
#[tokio::test]
async fn another_identitys_parked_entries_do_not_block_a_switch() {
    let fixture = Fixture::signed_in("org-a");
    let (api, _) = start(two_orgs()).await;
    let store = &fixture.shared.store;
    let parked: Vec<_> = (0..MAX_PENDING)
        .map(|n| {
            tagged(
                store,
                &format!("ws-theirs-{n}"),
                &format!("key-theirs-{n}"),
                USER_B,
            )
        })
        .collect();
    record_pending(store, &parked).unwrap();
    let outcome = select_with(&fixture.shared, &api, "org-b", Duration::ZERO)
        .await
        .unwrap();
    assert!(outcome.switched);
}
