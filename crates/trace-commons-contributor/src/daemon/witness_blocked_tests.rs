//! The daemon's count of submissions the witness kept from being sent, and
//! the report that rides on the history read (`refresh_history`).

use std::sync::{Arc, Mutex};

use axum::response::IntoResponse as _;
use axum::{Json, Router, routing::post};
use trace_commons_protocol::witness_blocked::{
    WITNESS_BLOCKED_HEADER, WitnessBlockedCounts, WitnessBlockedLabel,
};

use super::state::DaemonState;
use super::*;

/// One local server standing in for both the issuer and ingest. Records the
/// witness-blocked header of each status read; answers the read with `503`
/// while `fail` is set.
async fn serve(seen: Arc<Mutex<Vec<Option<String>>>>, fail: Arc<Mutex<bool>>) -> String {
    let router = Router::new()
        .route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging", "evaluation"],
                }))
            }),
        )
        .route(
            "/v1/contributors/me/submission-status",
            post(move |headers: axum::http::HeaderMap| {
                let seen = seen.clone();
                let fail = fail.clone();
                async move {
                    seen.lock().unwrap().push(
                        headers
                            .get(WITNESS_BLOCKED_HEADER)
                            .map(|v| v.to_str().unwrap().to_string()),
                    );
                    if *fail.lock().unwrap() {
                        return axum::http::StatusCode::SERVICE_UNAVAILABLE.into_response();
                    }
                    Json(serde_json::json!([])).into_response()
                }
            }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    format!("http://{addr}")
}

fn enrolled(url: &str) -> (tempfile::TempDir, Arc<ipc::DaemonShared>) {
    let (dir, store) = crate::config::tests_support::temp_store();
    let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.issuer_url = url.to_string();
    cfg.ingest_url = url.to_string();
    cfg.device_key_id = device.device_key_id.clone();
    store.save_config(&cfg).unwrap();
    let shared = Arc::new(ipc::DaemonShared::load(store).unwrap());
    (dir, shared)
}

fn blocked(label: Option<WitnessBlockedLabel>) -> AppliedDecision {
    AppliedDecision {
        witness_blocked: label,
        ..AppliedDecision::default()
    }
}

#[test]
fn only_a_blocked_decision_is_counted() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = ipc::DaemonShared::load(store).unwrap();
    assert!(!note_witness_blocked(&shared, &blocked(None)));
    assert!(note_witness_blocked(
        &shared,
        &blocked(Some(WitnessBlockedLabel::Busy))
    ));
    assert!(note_witness_blocked(
        &shared,
        &blocked(Some(WitnessBlockedLabel::Busy))
    ));
    assert!(note_witness_blocked(
        &shared,
        &blocked(Some(WitnessBlockedLabel::TimedOut))
    ));
    let counts = shared.state.lock().unwrap().witness_blocked;
    assert_eq!(
        counts,
        WitnessBlockedCounts {
            busy: 2,
            timed_out: 1,
            unreachable: 0,
        }
    );
}

#[tokio::test]
async fn the_history_read_carries_the_counts_and_clears_them() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(false));
    let url = serve(seen.clone(), fail.clone()).await;
    let (_dir, shared) = enrolled(&url);
    {
        let mut state = shared.state.lock().unwrap();
        state
            .witness_blocked
            .record(WitnessBlockedLabel::Unreachable);
        state.witness_blocked.record(WitnessBlockedLabel::Busy);
    }

    refresh_history(&shared, Utc::now()).await.unwrap();

    // No receipts on this device: the read went out only to carry the report.
    assert_eq!(
        *seen.lock().unwrap(),
        vec![Some("busy=1,unreachable=1".to_string())]
    );
    assert!(shared.state.lock().unwrap().witness_blocked.is_empty());
    assert!(
        DaemonState::load(&shared.store)
            .unwrap()
            .witness_blocked
            .is_empty(),
        "the cleared counts are what is on disk"
    );
}

#[tokio::test]
async fn a_failed_history_read_keeps_the_counts_for_the_next_one() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(true));
    let url = serve(seen.clone(), fail.clone()).await;
    let (_dir, shared) = enrolled(&url);
    shared
        .state
        .lock()
        .unwrap()
        .witness_blocked
        .record(WitnessBlockedLabel::TimedOut);

    // A failed read serves the cache; it is not an error of the pass.
    refresh_history(&shared, Utc::now()).await.unwrap();
    assert_eq!(seen.lock().unwrap().len(), 1);
    assert_eq!(shared.state.lock().unwrap().witness_blocked.timed_out, 1);
    assert_eq!(
        DaemonState::load(&shared.store)
            .unwrap()
            .witness_blocked
            .timed_out,
        1,
        "the restored counts survive a restart"
    );

    // One more blocked send meanwhile, then a read that succeeds.
    shared
        .state
        .lock()
        .unwrap()
        .witness_blocked
        .record(WitnessBlockedLabel::TimedOut);
    *fail.lock().unwrap() = false;
    refresh_history(&shared, Utc::now()).await.unwrap();
    assert_eq!(
        seen.lock().unwrap().last().cloned().flatten().as_deref(),
        Some("timed_out=2")
    );
    assert!(shared.state.lock().unwrap().witness_blocked.is_empty());
}

#[tokio::test]
async fn nothing_to_report_and_no_receipts_sends_nothing() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let fail = Arc::new(Mutex::new(false));
    let url = serve(seen.clone(), fail).await;
    let (_dir, shared) = enrolled(&url);
    refresh_history(&shared, Utc::now()).await.unwrap();
    assert!(seen.lock().unwrap().is_empty());
}
