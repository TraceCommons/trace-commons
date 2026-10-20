//! The raw-evidence endpoint is separate from status/log/report surfaces.
use crate::state::AppState;
use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum Request {
    Status {
        session: String,
    },
    List {
        session: String,
    },
    Find {
        session: String,
        request_digest: String,
        response_digest: String,
    },
    Acquire {
        session: String,
        captures: Vec<String>,
        owner: String,
        seconds: i64,
    },
    Read {
        lease: String,
        owner: String,
        capture: String,
    },
    Renew {
        lease: String,
        owner: String,
        snapshot_digest: String,
        seconds: i64,
    },
    Release {
        capture_store_id: String,
        lease: String,
        owner: String,
        snapshot_digest: String,
    },
}
pub(super) async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<Request>,
) -> Response {
    if let Err(response) = super::authorize(&state, &headers) {
        return *response;
    }
    let Some(spool) = state.token_spool else {
        return (StatusCode::SERVICE_UNAVAILABLE, "token-capture-unavailable").into_response();
    };
    let work = tokio::task::spawn_blocking(move || {
        use ironwire_ledger::token_spool::SpoolError;
        let now = chrono::Utc::now().timestamp();
        match request {
            Request::Status { session } => spool.status(&session, now),
            Request::List { session } => {
                serde_json::to_value(spool.list(&session, now)?).map_err(|_| SpoolError::Invalid)
            }
            Request::Find { session, request_digest, response_digest } => {
                serde_json::to_value(spool.find(&session, &request_digest, &response_digest, now)?).map_err(|_| SpoolError::Invalid)
            }
            Request::Acquire {
                session,
                captures,
                owner,
                seconds,
            } => serde_json::to_value(spool.acquire(&session, &captures, &owner, now, seconds)?)
                .map_err(|_| SpoolError::Invalid),
            Request::Read {
                lease,
                owner,
                capture,
            } => {
                let (request, response) = spool.read(&lease, &owner, &capture, now)?;
                Ok(serde_json::json!({"request":request,"response":response}))
            }
            Request::Renew { lease, owner, snapshot_digest, seconds } => Ok(serde_json::json!({"expires_at":spool.renew(&lease, &owner, &snapshot_digest, now, seconds)?})),
            Request::Release {
                capture_store_id,
                lease,
                owner,
                snapshot_digest,
            } => {
                if spool.store_id()? != capture_store_id { return Err(SpoolError::Unavailable); }
                spool.release(&lease, &owner, &snapshot_digest, now)?;
                Ok(serde_json::json!({"released":true}))
            }
        }
    })
    .await;
    match work {
        Ok(Ok(value)) => Json(value).into_response(),
        Ok(Err(error)) => (
            StatusCode::CONFLICT,
            Json(serde_json::json!({"error":error.to_string()})),
        )
            .into_response(),
        Err(_) => (StatusCode::INTERNAL_SERVER_ERROR, "token-capture-storage").into_response(),
    }
}
