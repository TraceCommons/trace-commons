// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drive the witness's job routes in process: what the retired
//! `POST /v1/witness` did in one request, as a submit and polls.

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, Response, StatusCode};
use tower::ServiceExt;
use trace_commons_protocol::witness_jobs::{
    WITNESS_FETCH_SECRET_HEADER, WITNESS_FETCH_SECRET_SHA256_HEADER, WITNESS_JOBS_PATH,
    WitnessJobAccepted,
};
use trace_commons_server::witness_service::http::WitnessLoadBound;
use trace_commons_server::witness_service::jobs::{
    JobLimits, RESULT_TTL, ResultSealer, WitnessJobs, sha256,
};

/// A job table sized from `load`, over a directory that lives as long as
/// the returned guard.
pub fn test_jobs(load: &WitnessLoadBound) -> (Arc<WitnessJobs>, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("a temp dir");
    let sealer = ResultSealer::open(dir.path(), zeroize::Zeroizing::new([0x24; 32]))
        .expect("a writable temp dir");
    let jobs = WitnessJobs::new(
        sealer,
        JobLimits {
            max_active: load.max_concurrent_requests(),
            max_active_background: load.max_background_requests(),
            max_stored: 64,
            max_result_bytes: 8 * 1024 * 1024,
            result_ttl: RESULT_TTL,
        },
        load.request_timeout(),
    );
    (Arc::new(jobs), dir)
}

/// Submit `body` as a job and poll it to its answer. A refusal at submit is
/// returned as it came.
pub async fn witness_through_jobs(router: Router, body: Vec<u8>) -> Response<Body> {
    let secret = [0x77u8; 32];
    let submit = Request::builder()
        .method(Method::POST)
        .uri(WITNESS_JOBS_PATH)
        .header("content-type", "application/json")
        .header(
            WITNESS_FETCH_SECRET_SHA256_HEADER,
            hex::encode(sha256(&secret)),
        )
        .body(Body::from(body))
        .expect("a well formed request");
    let response = router
        .clone()
        .oneshot(submit)
        .await
        .expect("the router is infallible");
    if response.status() != StatusCode::ACCEPTED {
        return response;
    }
    let accepted: WitnessJobAccepted = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("a small 202"),
    )
    .expect("a 202 names the job");
    loop {
        let poll = Request::builder()
            .method(Method::GET)
            .uri(format!("{WITNESS_JOBS_PATH}/{}", accepted.job_id))
            .header(WITNESS_FETCH_SECRET_HEADER, hex::encode(secret))
            .body(Body::empty())
            .expect("a well formed request");
        let response = router
            .clone()
            .oneshot(poll)
            .await
            .expect("the router is infallible");
        if response.status() != StatusCode::ACCEPTED {
            return response;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}
