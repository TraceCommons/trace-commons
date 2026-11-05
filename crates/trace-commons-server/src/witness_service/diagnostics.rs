// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `GET /v1/diagnostics`: what this witness has been doing, label-only.
//!
//! # Why it exists
//!
//! The witness's CVM logs are not readable by the operator as deployed, so
//! when the live witness began refusing every new request after ~122 s on
//! 2026-11-05, nothing could say whether its classifier key was refused, its
//! endpoint was slow, or it was down. This route says, in counts.
//!
//! # What it serves, and what it never does
//!
//! Counts, gauges, durations and identities that are already public:
//!
//! - the build line (version and commit, the same string `--version` prints),
//!   the witness measurement and signing address (both already served by
//!   `/v1/attestation` and carried by every certificate), and the redaction
//!   pipeline alias (carried by every certificate);
//! - uptime;
//! - request-slot occupancy and how requests ended: completed by status class,
//!   refused as saturated, abandoned at the timeout;
//! - for the NEAR AI classifier: requests by outcome kind (ok, timeout,
//!   transport, 401/403, 402/429, 5xx, 413, other 4xx, body shape), requests
//!   on the wire now, and recent successful latencies by body-size bucket.
//!
//! Never: request bodies or any part of them, request or trace identifiers,
//! keys, classifier hostnames, peer addresses, or per-request records. Every
//! value is an aggregate over all callers.
//!
//! # Unauthenticated, like `/v1/attestation`
//!
//! The witness has no operator credential, by design: it is deliberately
//! denied an identity to correlate against content, and adding one for this
//! route would add a secret to the measured configuration. What an anonymous
//! reader learns -- aggregate load, failure kinds, latency -- is no more than
//! timing any request already reveals. Like `/v1/attestation` it sits outside
//! the request-slot bound, so it still answers when the witness is saturated,
//! which is when it is needed; it does constant work and reads no body.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

use axum::Router;
use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use super::http::WitnessLoadBound;

/// Identity this witness reports. All of it is already public.
#[derive(Clone, Debug)]
pub struct WitnessDiagnosticsInfo {
    /// The build line: package version, commit and build time.
    pub build: String,
    /// The `mrtd:..+mrconfigid:..` label certificates carry.
    pub witness_measurement: String,
    /// The certificate signing address.
    pub signing_address: String,
    /// The redaction pipeline alias certificates carry.
    pub redaction_pipeline: String,
    /// Whether the NEAR AI classifier's counters apply to this witness.
    pub near_ai_classifier: bool,
}

/// How the request-slot bound has been used. Written by
/// [`WitnessLoadBound`]'s middleware.
pub struct LoadCounters {
    max_concurrent: usize,
    completed_2xx: AtomicU64,
    completed_4xx: AtomicU64,
    completed_5xx: AtomicU64,
    saturated: AtomicU64,
    timed_out: AtomicU64,
}

impl LoadCounters {
    pub(crate) fn new(max_concurrent: usize) -> Self {
        Self {
            max_concurrent,
            completed_2xx: AtomicU64::new(0),
            completed_4xx: AtomicU64::new(0),
            completed_5xx: AtomicU64::new(0),
            saturated: AtomicU64::new(0),
            timed_out: AtomicU64::new(0),
        }
    }

    pub(crate) fn saturated(&self) {
        self.saturated.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn timed_out(&self) {
        self.timed_out.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn finished(&self, status: u16) {
        let counter = match status {
            200..=399 => &self.completed_2xx,
            400..=499 => &self.completed_4xx,
            _ => &self.completed_5xx,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self, available: usize) -> LoadSnapshot {
        LoadSnapshot {
            max_concurrent: self.max_concurrent,
            in_use: self.max_concurrent.saturating_sub(available),
            completed_2xx: self.completed_2xx.load(Ordering::Relaxed),
            completed_4xx: self.completed_4xx.load(Ordering::Relaxed),
            completed_5xx: self.completed_5xx.load(Ordering::Relaxed),
            saturated_refusals: self.saturated.load(Ordering::Relaxed),
            timed_out: self.timed_out.load(Ordering::Relaxed),
        }
    }
}

/// The request-slot bound's state. Counts are since the process started.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct LoadSnapshot {
    pub max_concurrent: usize,
    pub in_use: usize,
    pub completed_2xx: u64,
    pub completed_4xx: u64,
    pub completed_5xx: u64,
    pub saturated_refusals: u64,
    pub timed_out: u64,
}

#[derive(Serialize)]
struct Diagnostics<'a> {
    schema: &'static str,
    build: &'a str,
    witness_measurement: &'a str,
    signing_address: &'a str,
    redaction_pipeline: &'a str,
    uptime_secs: u64,
    requests: LoadSnapshot,
    classifier: Option<Classifier>,
}

#[derive(Serialize)]
struct Classifier {
    backend: &'static str,
    #[serde(flatten)]
    snapshot: trace_commons_protocol::privacy_filter_near_ai::diagnostics::Snapshot,
}

struct DiagnosticsState {
    info: WitnessDiagnosticsInfo,
    load: WitnessLoadBound,
    started: Instant,
}

/// The diagnostics route, to merge beside
/// [`super::http::witness_router`]. `load` must be the same bound that router
/// was given, or the counts describe nothing.
pub fn diagnostics_router(load: WitnessLoadBound, info: WitnessDiagnosticsInfo) -> Router {
    Router::new()
        .route("/v1/diagnostics", get(diagnostics_handler))
        .with_state(Arc::new(DiagnosticsState {
            info,
            load,
            started: Instant::now(),
        }))
}

async fn diagnostics_handler(State(state): State<Arc<DiagnosticsState>>) -> Response {
    let info = &state.info;
    let classifier = info.near_ai_classifier.then(|| Classifier {
        backend: "near-ai",
        snapshot: trace_commons_protocol::privacy_filter_near_ai::diagnostics::global().snapshot(),
    });
    let body = Diagnostics {
        schema: "witness-diagnostics-v1",
        build: &info.build,
        witness_measurement: &info.witness_measurement,
        signing_address: &info.signing_address,
        redaction_pipeline: &info.redaction_pipeline,
        uptime_secs: state.started.elapsed().as_secs(),
        requests: state.load.snapshot(),
        classifier,
    };
    ([(header::CACHE_CONTROL, "no-store")], axum::Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::time::Duration;
    use tower::ServiceExt;

    fn info() -> WitnessDiagnosticsInfo {
        WitnessDiagnosticsInfo {
            build: "trace-commons-server 0.1.0 (abc1234)".to_string(),
            witness_measurement: "mrtd:00+mrconfigid:11".to_string(),
            signing_address: "0x0000000000000000000000000000000000000001".to_string(),
            redaction_pipeline: "pipeline-alias".to_string(),
            near_ai_classifier: true,
        }
    }

    async fn get_json(router: Router) -> (StatusCode, serde_json::Value, Option<String>) {
        let response = router
            .oneshot(Request::get("/v1/diagnostics").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let status = response.status();
        let cache = response
            .headers()
            .get(header::CACHE_CONTROL)
            .map(|v| v.to_str().unwrap().to_string());
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
            .await
            .unwrap();
        (status, serde_json::from_slice(&bytes).unwrap(), cache)
    }

    /// The response carries exactly the documented fields -- a new field is
    /// a decision about what this unauthenticated route discloses, so it has
    /// to change this test too.
    #[tokio::test]
    async fn serves_the_documented_fields_and_nothing_else() {
        let load = WitnessLoadBound::new(4, Duration::from_secs(300));
        let (status, json, cache) = get_json(diagnostics_router(load, info())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(cache.as_deref(), Some("no-store"));
        // Sorted: `serde_json/preserve_order` is on in this graph, so maps
        // iterate in insertion order.
        let sorted = |value: &serde_json::Value| -> Vec<String> {
            let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
            keys.sort();
            keys
        };
        let keys = sorted(&json);
        assert_eq!(
            keys,
            [
                "build",
                "classifier",
                "redaction_pipeline",
                "requests",
                "schema",
                "signing_address",
                "uptime_secs",
                "witness_measurement",
            ]
        );
        let requests = sorted(&json["requests"]);
        assert_eq!(
            requests,
            [
                "completed_2xx",
                "completed_4xx",
                "completed_5xx",
                "in_use",
                "max_concurrent",
                "saturated_refusals",
                "timed_out",
            ]
        );
        let classifier = sorted(&json["classifier"]);
        assert_eq!(
            classifier,
            ["backend", "in_flight", "latency_ms", "outcomes"]
        );
        let outcomes = sorted(&json["classifier"]["outcomes"]);
        assert_eq!(
            outcomes,
            [
                "auth_401_403",
                "body_shape",
                "ok",
                "other_4xx",
                "quota_402_429",
                "server_5xx",
                "timeout",
                "too_large_413",
                "transport",
            ]
        );
        assert_eq!(json["requests"]["max_concurrent"], 4);
        assert_eq!(json["requests"]["in_use"], 0);
    }

    #[tokio::test]
    async fn a_witness_without_the_near_ai_classifier_reports_none() {
        let load = WitnessLoadBound::new(1, Duration::from_secs(300));
        let (_, json, _) = get_json(diagnostics_router(
            load,
            WitnessDiagnosticsInfo {
                near_ai_classifier: false,
                ..info()
            },
        ))
        .await;
        assert!(json["classifier"].is_null());
    }

    #[test]
    fn statuses_fall_into_their_classes() {
        let counters = LoadCounters::new(2);
        for status in [200, 201, 400, 422, 500, 503, 504] {
            counters.finished(status);
        }
        counters.saturated();
        counters.timed_out();
        let snapshot = counters.snapshot(1);
        assert_eq!(snapshot.completed_2xx, 2);
        assert_eq!(snapshot.completed_4xx, 2);
        assert_eq!(snapshot.completed_5xx, 3);
        assert_eq!(snapshot.saturated_refusals, 1);
        assert_eq!(snapshot.timed_out, 1);
        assert_eq!(snapshot.in_use, 1);
    }
}
