// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use crate::tests::test_state;
use crate::{AppState, app};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use std::sync::Arc;
use tower::ServiceExt;
use trace_commons_server::mission_rewards::RewardError;

async fn request(state: Arc<AppState>, uri: &str) -> axum::response::Response {
    app(state)
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("response")
}

fn assert_protected(response: &axum::response::Response, status: StatusCode) {
    assert_eq!(response.status(), status);
    assert_eq!(response.headers()["cache-control"], "no-store");
    assert_eq!(response.headers()["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn mission_catalog_refuses_missing_database_with_protected_headers() {
    let temp = tempfile::tempdir().expect("temp dir");
    let response = request(test_state(temp.path().to_path_buf()), "/v1/missions").await;
    assert_protected(&response, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn mission_catalog_refuses_malformed_or_out_of_bounds_query_before_database_access() {
    let temp = tempfile::tempdir().expect("temp dir");
    for uri in [
        "/v1/missions?limit=invalid",
        "/v1/missions?limit=0",
        "/v1/missions?limit=51",
        "/v1/missions?before=00000000-0000-0000-0000-000000000000",
    ] {
        let response = request(test_state(temp.path().to_path_buf()), uri).await;
        assert_protected(&response, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn mission_publication_refuses_malformed_and_nil_paths_before_database_access() {
    let temp = tempfile::tempdir().expect("temp dir");
    for uri in [
        "/v1/missions/not-a-uuid",
        "/v1/missions/00000000-0000-0000-0000-000000000000",
    ] {
        let response = request(test_state(temp.path().to_path_buf()), uri).await;
        assert_protected(&response, StatusCode::BAD_REQUEST);
    }
}

#[test]
fn public_mission_error_hides_unavailable_publications_as_not_found() {
    assert_eq!(
        crate::rewards::public_mission_error(RewardError::OfferSuspended),
        RewardError::NotFound
    );
    assert_eq!(
        crate::rewards::public_mission_error(RewardError::ProgramClosed),
        RewardError::NotFound
    );
    assert_eq!(
        crate::rewards::public_mission_error(RewardError::OfferChanged),
        RewardError::OfferChanged
    );
}
