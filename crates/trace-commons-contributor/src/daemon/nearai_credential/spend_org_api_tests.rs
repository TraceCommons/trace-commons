//! The three management calls choosing a spending organization needs:
//! list the account's organizations, mint a key in a named one, and revoke
//! the key it replaces. Every shape here is the one the deployed service's
//! OpenAPI document declares (`ListOrganizationsResponse`,
//! `UserOrganizationResponse`, and `DELETE
//! /v1/workspaces/{workspace_id}/api-keys/{key_id}` answering 204).

use super::*;
use axum::{
    Json, Router,
    extract::{Path, Request},
    http::StatusCode,
    routing::{delete, get, post},
};
use std::sync::{Arc, Mutex};

fn session() -> SessionTokens {
    SessionTokens {
        access_token: "session-jwt".into(),
        refresh_token: "rt_example".into(),
    }
}

async fn spawn(router: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{address}")
}

fn org(id: &str, name: &str, active: bool) -> serde_json::Value {
    serde_json::json!({
        "id": id, "name": name, "role": "owner", "is_active": active,
        "owner_id": "u", "settings": {},
        "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
    })
}

fn listing(organizations: Vec<serde_json::Value>) -> serde_json::Value {
    let total = organizations.len();
    serde_json::json!({"organizations": organizations, "total": total, "limit": 100, "offset": 0})
}

/// Every request line the stub saw, path and query, in order.
type Seen = Arc<Mutex<Vec<String>>>;

fn recording(router: Router, seen: Seen) -> Router {
    router.layer(axum::middleware::from_fn(
        move |request: Request, next: axum::middleware::Next| {
            let seen = seen.clone();
            async move {
                let bearer = request
                    .headers()
                    .get("authorization")
                    .and_then(|value| value.to_str().ok())
                    .unwrap_or_default()
                    .to_string();
                seen.lock().unwrap().push(format!(
                    "{} {} {bearer}",
                    request.method(),
                    request.uri()
                ));
                next.run(request).await
            }
        },
    ))
}

/// A two-organization account whose workspaces and key mint answer for
/// whichever organization was asked.
fn two_org_service(seen: Seen) -> Router {
    let organizations = listing(vec![
        org("org-a", "First example org", true),
        org("org-b", "Second example org", true),
    ]);
    recording(
        Router::new()
            .route(
                "/v1/organizations",
                get(move || {
                    let organizations = organizations.clone();
                    async move { Json(organizations) }
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
                post(|Path(workspace): Path<String>| async move {
                    Json(serde_json::json!({
                        "id": format!("key-{workspace}"), "key": "sk-minted-secret",
                        "key_prefix": "sk-min", "workspace_id": workspace,
                        "created_by_user_id": "u", "created_at": "2026-09-08T00:00:00Z",
                        "is_active": true,
                    }))
                }),
            ),
        seen,
    )
}

#[tokio::test]
async fn listing_returns_only_active_organizations_with_their_names() {
    let organizations = listing(vec![
        org("org-a", "First example org", true),
        org("org-gone", "Inactive example org", false),
        org("org-b", "Second example org", true),
    ]);
    let seen: Seen = Default::default();
    let router = recording(
        Router::new().route(
            "/v1/organizations",
            get(move || {
                let organizations = organizations.clone();
                async move { Json(organizations) }
            }),
        ),
        seen.clone(),
    );
    let api = CloudApi::for_test(&spawn(router).await).unwrap();
    let listed = api.list_organizations(&session()).await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["org-a", "org-b"]);
    assert_eq!(listed[1].name, "Second example org");
    assert_eq!(listed[1].role, "owner");
    // Session-authenticated, and the page size is asked for rather than left
    // to the service's default, so a truncation is detectable.
    let seen = seen.lock().unwrap().clone();
    assert_eq!(
        seen,
        ["GET /v1/organizations?limit=100 Bearer session-jwt"],
        "{seen:?}"
    );
}

/// An account with exactly one organization -- the common case in
/// production -- lists as exactly one.
#[tokio::test]
async fn a_single_organization_account_lists_one() {
    let organizations = listing(vec![org("org-a", "Only example org", true)]);
    let router = Router::new().route(
        "/v1/organizations",
        get(move || {
            let organizations = organizations.clone();
            async move { Json(organizations) }
        }),
    );
    let api = CloudApi::for_test(&spawn(router).await).unwrap();
    let listed = api.list_organizations(&session()).await.unwrap();
    assert_eq!(listed.len(), 1);
}

/// A page that says there are more than it carries is not a count. Showing a
/// picker from half a list would hide the organization a contributor came to
/// choose, so it is refused rather than served.
#[tokio::test]
async fn a_truncated_listing_is_refused_rather_than_counted() {
    let mut organizations = listing(vec![org("org-a", "First example org", true)]);
    organizations["total"] = 101.into();
    let router = Router::new().route(
        "/v1/organizations",
        get(move || {
            let organizations = organizations.clone();
            async move { Json(organizations) }
        }),
    );
    let api = CloudApi::for_test(&spawn(router).await).unwrap();
    assert_eq!(
        api.list_organizations(&session())
            .await
            .unwrap_err()
            .to_string(),
        "near_ai_credential_unexpected"
    );
}

/// An id that could not be a path segment is dropped, and a name is cleaned
/// of control characters and bounded before anything can render it.
#[tokio::test]
async fn hostile_ids_are_dropped_and_names_are_bounded() {
    let long = "n".repeat(500);
    let organizations = listing(vec![
        org("../admin", "Traversal", true),
        org("org-a", &format!("Line\nbreak{long}"), true),
        org("org-b", "   ", true),
    ]);
    let router = Router::new().route(
        "/v1/organizations",
        get(move || {
            let organizations = organizations.clone();
            async move { Json(organizations) }
        }),
    );
    let api = CloudApi::for_test(&spawn(router).await).unwrap();
    let listed = api.list_organizations(&session()).await.unwrap();
    let ids: Vec<&str> = listed.iter().map(|o| o.id.as_str()).collect();
    assert_eq!(ids, ["org-a", "org-b"]);
    assert!(!listed[0].name.contains('\n'));
    assert!(listed[0].name.chars().count() <= MAX_ORGANIZATION_NAME_CHARS);
    // A blank name is not drawn as a blank row: the id stands in.
    assert_eq!(listed[1].name, "org-b");
}

#[tokio::test]
async fn minting_in_a_named_organization_mints_there() {
    let seen: Seen = Default::default();
    let api = CloudApi::for_test(&spawn(two_org_service(seen.clone())).await).unwrap();
    let minted = api
        .mint_inference_key_in(
            &session(),
            "trace-commons-device",
            OrganizationChoice::Exactly("org-b"),
        )
        .await
        .unwrap();
    assert_eq!(minted.organization_id, "org-b");
    assert_eq!(minted.workspace_id, "ws-org-b");
    assert_eq!(minted.key_id, "key-ws-org-b");
    let seen = seen.lock().unwrap().clone();
    assert!(
        seen.iter()
            .any(|line| line.starts_with("GET /v1/organizations/org-b/workspaces")),
        "{seen:?}"
    );
    assert!(
        !seen.iter().any(|line| line.contains("org-a/workspaces")),
        "{seen:?}"
    );
}

/// The target must be one the account can see and is active in. Nothing is
/// minted for an organization the listing did not offer.
#[tokio::test]
async fn minting_in_an_organization_the_account_cannot_see_mints_nothing() {
    let seen: Seen = Default::default();
    let api = CloudApi::for_test(&spawn(two_org_service(seen.clone())).await).unwrap();
    let error = api
        .mint_inference_key_in(&session(), "d", OrganizationChoice::Exactly("org-c"))
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(error, "near_ai_credential_no_organization");
    let seen = seen.lock().unwrap().clone();
    assert!(
        !seen.iter().any(|line| line.starts_with("POST")),
        "{seen:?}"
    );
}

/// The sign-in prefers a remembered choice when the account still has it,
/// and otherwise mints where it always has: the first active organization.
#[tokio::test]
async fn a_preferred_organization_is_used_when_present_and_ignored_when_not() {
    let api = CloudApi::for_test(&spawn(two_org_service(Default::default())).await).unwrap();
    let preferred = api
        .mint_inference_key_in(&session(), "d", OrganizationChoice::Prefer("org-b"))
        .await
        .unwrap();
    assert_eq!(preferred.organization_id, "org-b");
    let fallback = api
        .mint_inference_key_in(&session(), "d", OrganizationChoice::Prefer("org-c"))
        .await
        .unwrap();
    assert_eq!(fallback.organization_id, "org-a");
    let first = api.mint_inference_key(&session(), "d").await.unwrap();
    assert_eq!(first.organization_id, "org-a");
}

fn revoke_service(status: StatusCode, seen: Seen) -> Router {
    recording(
        Router::new().route(
            "/v1/workspaces/{workspace}/api-keys/{key}",
            delete(move || async move { status }),
        ),
        seen,
    )
}

#[tokio::test]
async fn revoking_sends_one_session_authenticated_delete() {
    let seen: Seen = Default::default();
    let api =
        CloudApi::for_test(&spawn(revoke_service(StatusCode::NO_CONTENT, seen.clone())).await)
            .unwrap();
    assert_eq!(
        api.revoke_inference_key(&session(), "ws-1", "key-1")
            .await
            .unwrap(),
        RevokeOutcome::Revoked
    );
    assert_eq!(
        seen.lock().unwrap().clone(),
        ["DELETE /v1/workspaces/ws-1/api-keys/key-1 Bearer session-jwt"]
    );
}

/// A key the service no longer has is already what a revoke wanted.
#[tokio::test]
async fn a_key_already_gone_is_not_a_failure() {
    let api =
        CloudApi::for_test(&spawn(revoke_service(StatusCode::NOT_FOUND, Default::default())).await)
            .unwrap();
    assert_eq!(
        api.revoke_inference_key(&session(), "ws-1", "key-1")
            .await
            .unwrap(),
        RevokeOutcome::AlreadyGone
    );
}

#[tokio::test]
async fn a_refused_revoke_is_an_error_and_a_401_names_the_session() {
    for (status, label) in [
        (StatusCode::FORBIDDEN, "near_ai_credential_refused"),
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            "near_ai_credential_refused",
        ),
        (
            StatusCode::UNAUTHORIZED,
            "near_ai_credential_session_expired",
        ),
    ] {
        let api =
            CloudApi::for_test(&spawn(revoke_service(status, Default::default())).await).unwrap();
        assert_eq!(
            api.revoke_inference_key(&session(), "ws-1", "key-1")
                .await
                .unwrap_err()
                .to_string(),
            label,
            "{status}"
        );
    }
}

/// Both ids become path segments. Neither leaves this process unless it is
/// one.
#[tokio::test]
async fn a_revoke_with_an_id_that_is_not_a_path_segment_never_leaves() {
    let seen: Seen = Default::default();
    let api =
        CloudApi::for_test(&spawn(revoke_service(StatusCode::NO_CONTENT, seen.clone())).await)
            .unwrap();
    for (workspace, key) in [
        ("../ws", "key-1"),
        ("ws-1", "a/b"),
        ("", "key-1"),
        ("ws-1", ""),
    ] {
        assert!(
            api.revoke_inference_key(&session(), workspace, key)
                .await
                .is_err()
        );
    }
    assert!(seen.lock().unwrap().is_empty());
}
