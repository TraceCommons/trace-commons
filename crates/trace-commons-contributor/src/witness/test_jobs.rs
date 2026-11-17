//! Serve a test witness written against the retired `POST /v1/witness` as
//! the job routes that replaced it.
//!
//! A test double's `/v1/witness` handler still decides the answer; this
//! wrapper runs it at submit and hands the answer back on the first poll,
//! the way a witness hands back a finished job. Every other path falls
//! through to the double untouched.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::Body;
use axum::extract::{Path, Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use sha2::Digest as _;
use tower::ServiceExt as _;
use trace_commons_protocol::witness_jobs::{
    WITNESS_FETCH_SECRET_HEADER, WITNESS_FETCH_SECRET_SHA256_HEADER, WITNESS_JOBS_PATH,
};

struct Finished {
    secret_hash: String,
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

#[derive(Clone)]
struct Jobs {
    inner: Router,
    finished: Arc<Mutex<HashMap<String, Finished>>>,
    next: Arc<AtomicU64>,
}

/// `inner`, with `POST /v1/witness/jobs` and `GET`/`DELETE
/// /v1/witness/jobs/{id}` in front of its `POST /v1/witness`.
pub(crate) fn serve_as_jobs(inner: Router) -> Router {
    let state = Jobs {
        inner: inner.clone(),
        finished: Arc::default(),
        next: Arc::default(),
    };
    Router::new()
        .route(WITNESS_JOBS_PATH, post(submit))
        .route(
            "/v1/witness/jobs/{job_id}",
            axum::routing::get(poll).delete(cancel),
        )
        .with_state(state)
        .fallback_service(inner)
}

fn gone() -> Response {
    (
        StatusCode::NOT_FOUND,
        axum::Json(serde_json::json!({"error": "witness_job_gone"})),
    )
        .into_response()
}

async fn submit(State(jobs): State<Jobs>, request: Request) -> Response {
    let (mut parts, body) = request.into_parts();
    let Some(secret_hash) = parts
        .headers
        .remove(WITNESS_FETCH_SECRET_SHA256_HEADER)
        .and_then(|value| value.to_str().ok().map(str::to_string))
    else {
        return (
            StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({"error": "witness_fetch_secret_malformed"})),
        )
            .into_response();
    };
    parts.uri = "/v1/witness".parse().unwrap();
    let response = jobs
        .inner
        .clone()
        .oneshot(Request::from_parts(parts, body))
        .await
        .unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let body = axum::body::to_bytes(response.into_body(), 256 * 1024 * 1024)
        .await
        .unwrap_or_default()
        .to_vec();
    let job_id = format!("{:032x}", jobs.next.fetch_add(1, Ordering::SeqCst) + 1);
    jobs.finished.lock().unwrap().insert(
        job_id.clone(),
        Finished {
            secret_hash,
            status,
            headers,
            body,
        },
    );
    (
        StatusCode::ACCEPTED,
        axum::Json(serde_json::json!({
            "job_id": job_id,
            "poll_after_secs": 1,
            "queue_position": 0,
        })),
    )
        .into_response()
}

fn presented(headers: &HeaderMap) -> Option<String> {
    let secret = hex::decode(headers.get(WITNESS_FETCH_SECRET_HEADER)?.to_str().ok()?).ok()?;
    Some(hex::encode(sha2::Sha256::digest(secret)))
}

async fn poll(
    State(jobs): State<Jobs>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut finished = jobs.finished.lock().unwrap();
    let matches = finished
        .get(&job_id)
        .is_some_and(|job| Some(&job.secret_hash) == presented(&headers).as_ref());
    if !matches {
        return gone();
    }
    let job = finished.remove(&job_id).unwrap();
    let mut response = Response::new(Body::from(job.body));
    *response.status_mut() = job.status;
    *response.headers_mut() = job.headers;
    response
}

async fn cancel(
    State(jobs): State<Jobs>,
    Path(job_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let mut finished = jobs.finished.lock().unwrap();
    let matches = finished
        .get(&job_id)
        .is_some_and(|job| Some(&job.secret_hash) == presented(&headers).as_ref());
    if matches {
        finished.remove(&job_id);
        StatusCode::NO_CONTENT.into_response()
    } else {
        gone()
    }
}
