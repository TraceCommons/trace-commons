//! Operator admin endpoint for the upload-claim issuer.
//!
//! Mounted on an optional second `axum::serve` bind (defaults to
//! disabled; required to be a loopback address unless explicitly opted
//! into via env). Returns counts-only readiness; no field carries
//! contributor identity, raw invite codes, or any operator-secret
//! material.
//!
//! `/v1/admin/allowlist-status` is the only route. The issuer keeps its
//! existing public router (`/health`, keyset, `/v1/trace-upload-claim`)
//! unchanged.

use std::sync::Arc;

use axum::Json;
use axum::extract::State;
use axum::routing::get;
use axum::{Router, http::StatusCode};
use serde_json::{Value, json};

use crate::trace_upload_claim_allowlist::{AllowlistSource, DenialCounter};

/// Shared state for the admin router. Cheap to clone (all `Arc`).
#[derive(Clone)]
pub struct AdminState {
    pub source: Option<Arc<dyn AllowlistSource>>,
    pub denial_counter: Arc<DenialCounter>,
    pub max_stale_seconds: u64,
}

/// Build the admin router. Wires the single status endpoint and applies
/// no auth middleware — the caller is responsible for binding only on
/// loopback (or explicitly opting into a public bind).
pub fn admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/v1/admin/allowlist-status", get(allowlist_status_handler))
        .with_state(state)
}

async fn allowlist_status_handler(State(state): State<AdminState>) -> (StatusCode, Json<Value>) {
    let Some(source) = state.source.as_ref() else {
        return (StatusCode::OK, Json(json!({ "configured": false })));
    };
    match source.snapshot() {
        Ok(snapshot) => {
            let snapshot_age = snapshot.loaded_at.elapsed();
            let snapshot_age_seconds = snapshot_age.as_secs();
            // Match the issuer handler's `snapshot_age > max_stale`
            // comparison exactly so the admin endpoint's `stale` flag
            // agrees with the issuance gate's decision at the same
            // moment, even at sub-second precision.
            let stale = snapshot_age > std::time::Duration::from_secs(state.max_stale_seconds);
            (
                StatusCode::OK,
                Json(json!({
                    "configured": true,
                    "source_label": snapshot.source_label,
                    "policy_label": snapshot.policy_label,
                    "entries": snapshot.subject_hashes.len(),
                    "snapshot_age_seconds": snapshot_age_seconds,
                    "denials_last_hour": state.denial_counter.count_in_window(),
                    "max_stale_seconds": state.max_stale_seconds,
                    "stale": stale,
                })),
            )
        }
        Err(err) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({
                "configured": true,
                "stale": true,
                "error": err.to_string(),
                "denials_last_hour": state.denial_counter.count_in_window(),
                "max_stale_seconds": state.max_stale_seconds,
            })),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use axum::body::{Body, to_bytes};
    use axum::http::{Method, Request};
    use tower::ServiceExt;

    use crate::trace_upload_claim_allowlist::{
        AllowlistEntry, AllowlistError, AllowlistFile, AllowlistSnapshot, hash_invite_code,
    };
    use chrono::Utc;
    use std::sync::Mutex;
    use std::time::Instant;

    /// Test source backed by a fixed snapshot — no filesystem coupling.
    /// `set_snapshot` lets a test rotate it mid-run; `set_error` flips
    /// the source into a permanent error state without a cached fallback.
    struct StaticSource {
        snapshot: Mutex<Option<AllowlistSnapshot>>,
        error: Mutex<Option<&'static str>>,
    }

    impl StaticSource {
        fn new(snap: AllowlistSnapshot) -> Self {
            Self {
                snapshot: Mutex::new(Some(snap)),
                error: Mutex::new(None),
            }
        }
    }

    impl AllowlistSource for StaticSource {
        fn snapshot(&self) -> Result<AllowlistSnapshot, AllowlistError> {
            if let Some(label) = *self.error.lock().unwrap() {
                return Err(AllowlistError::Malformed(label.into()));
            }
            self.snapshot
                .lock()
                .unwrap()
                .clone()
                .ok_or(AllowlistError::NoSnapshotYet)
        }
    }

    fn fixture_snapshot(codes: &[&str], policy_label: &str) -> AllowlistSnapshot {
        let entries: Vec<AllowlistEntry> = codes
            .iter()
            .map(|c| AllowlistEntry {
                subject_hash: hash_invite_code(c),
                tenant_id: "tenant-a".into(),
                note_label: None,
                max_uses: 1,
            })
            .collect();
        let file = AllowlistFile {
            version: 1,
            generated_at: Utc::now(),
            policy_label: policy_label.into(),
            entries,
        };
        AllowlistSnapshot::from_file(file, "fixture".into(), Instant::now())
            .expect("snapshot builds")
    }

    async fn get_status(router: Router) -> (StatusCode, Value) {
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri("/v1/admin/allowlist-status")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("request completes");
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024)
            .await
            .expect("body reads");
        let body: Value = serde_json::from_slice(&bytes).expect("json body");
        (status, body)
    }

    #[tokio::test]
    async fn status_returns_configured_false_when_no_source() {
        let state = AdminState {
            source: None,
            denial_counter: Arc::new(DenialCounter::new(Duration::from_secs(3600))),
            max_stale_seconds: 3600,
        };
        let (status, body) = get_status(admin_router(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body, json!({ "configured": false }));
    }

    #[tokio::test]
    async fn status_returns_counts_and_no_identifying_fields() {
        let snap = fixture_snapshot(&["INV-1", "INV-2", "INV-3"], "pilot-test");
        let state = AdminState {
            source: Some(Arc::new(StaticSource::new(snap))),
            denial_counter: Arc::new(DenialCounter::new(Duration::from_secs(3600))),
            max_stale_seconds: 3600,
        };
        let (status, body) = get_status(admin_router(state.clone())).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["configured"], json!(true));
        assert_eq!(body["entries"], json!(3));
        assert_eq!(body["policy_label"], json!("pilot-test"));
        assert_eq!(body["source_label"], json!("fixture"));
        assert_eq!(body["denials_last_hour"], json!(0));
        assert_eq!(body["max_stale_seconds"], json!(3600));
        assert_eq!(body["stale"], json!(false));

        // Identity-revealing fields must not appear under any name.
        let text = body.to_string();
        assert!(
            !text.contains("subject_hash"),
            "subject hashes leaked: {text}"
        );
        assert!(!text.contains("tenant_id"), "tenant_id leaked: {text}");
        assert!(!text.contains("note_label"), "note_label leaked: {text}");
        assert!(!text.contains("INV-"), "raw invite codes leaked: {text}");
    }

    #[tokio::test]
    async fn status_reflects_denial_counter() {
        let snap = fixture_snapshot(&["INV-1"], "pilot-test");
        let counter = Arc::new(DenialCounter::new(Duration::from_secs(3600)));
        counter.record();
        counter.record();
        let state = AdminState {
            source: Some(Arc::new(StaticSource::new(snap))),
            denial_counter: counter,
            max_stale_seconds: 3600,
        };
        let (_, body) = get_status(admin_router(state)).await;
        assert_eq!(body["denials_last_hour"], json!(2));
    }

    #[tokio::test]
    async fn status_marks_stale_when_snapshot_age_exceeds_max() {
        let snap = fixture_snapshot(&["INV-1"], "pilot-test");
        let state = AdminState {
            source: Some(Arc::new(StaticSource::new(snap))),
            denial_counter: Arc::new(DenialCounter::new(Duration::from_secs(3600))),
            max_stale_seconds: 0, // any non-zero age is stale
        };
        // Give loaded_at.elapsed() a moment to clear zero.
        tokio::time::sleep(Duration::from_millis(5)).await;
        let (status, body) = get_status(admin_router(state)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["stale"], json!(true));
    }
}
