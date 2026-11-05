// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `GET /v1/public/witness-accepted-list`: the signed list of witness
//! measurements clients accept beside their sign-up pins (spec 2026-11-05,
//! section 7; `trace_commons_protocol::witness_accepted_list`).
//!
//! The server does not sign. The operator signs the list with the commons'
//! list-signing key (runbook `docs/operator/witness-accepted-list.md`) and
//! installs the signed file at the path in
//! [`WITNESS_ACCEPTED_LIST_PATH_ENV`]. This route serves that file's bytes
//! exactly as installed; re-serialising them would break the signature.
//!
//! The file is read on every request, so installing a new list needs no
//! restart. Fail closed: with no setting the route answers 404; a setting
//! whose file is missing, unreadable, too large, or not a well-formed signed
//! list answers 503. An unsigned or mangled list is never served. Only fixed
//! labels are returned or logged, never the path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use trace_commons_protocol::witness_accepted_list::{
    WITNESS_ACCEPTED_LIST_MAX_BYTES, check_served_shape,
};

use super::{AppState, api_error};

/// The setting naming the signed list file.
pub(super) const WITNESS_ACCEPTED_LIST_PATH_ENV: &str = "TRACE_COMMONS_WITNESS_ACCEPTED_LIST_PATH";

/// The 404 label: this commons publishes no list.
pub(super) const LABEL_NOT_PUBLISHED: &str = "witness_accepted_list_not_published";
/// The 503 label: a list is configured but cannot be served.
pub(super) const LABEL_UNAVAILABLE: &str = "witness_accepted_list_unavailable";

/// The configured path, or `None` when the setting is absent or blank.
pub(super) fn path_from_env() -> Option<Arc<PathBuf>> {
    path_from_value(
        std::env::var(WITNESS_ACCEPTED_LIST_PATH_ENV)
            .ok()
            .as_deref(),
    )
}

pub(super) fn path_from_value(value: Option<&str>) -> Option<Arc<PathBuf>> {
    let value = value?.trim();
    (!value.is_empty()).then(|| Arc::new(PathBuf::from(value)))
}

/// Answers `GET /v1/public/witness-accepted-list`.
pub(super) async fn witness_accepted_list_handler(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let path = state.witness_accepted_list_path.clone();
    match path {
        None => api_error(StatusCode::NOT_FOUND, LABEL_NOT_PUBLISHED).into_response(),
        Some(path) => match tokio::task::spawn_blocking(move || read_servable(&path)).await {
            Ok(Some(bytes)) => served(bytes),
            _ => {
                tracing::warn!(event = LABEL_UNAVAILABLE, "witness accepted list");
                api_error(StatusCode::SERVICE_UNAVAILABLE, LABEL_UNAVAILABLE).into_response()
            }
        },
    }
}

/// The file's bytes, if it exists, is within the size bound and is a
/// well-formed signed list.
pub(super) fn read_servable(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(WITNESS_ACCEPTED_LIST_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    check_served_shape(&bytes).ok()?;
    Some(bytes)
}

fn served(bytes: Vec<u8>) -> axum::response::Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    // A short cache, so a newly installed list reaches clients within
    // minutes; clients refresh about daily anyway.
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}
