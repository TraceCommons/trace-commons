// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Public, byte-exact publication of operator-signed witness endpoints and
//! measurements. The server holds no signing key and fetches no endpoint.
//! Reads are bounded and repeated per request so atomic file replacement
//! publishes an update without a restart. Only fixed labels leave errors;
//! neither configured paths nor witness URLs enter logs.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::IntoResponse;
use trace_commons_protocol::witness_configuration::{
    WITNESS_CONFIGURATION_MAX_BYTES, check_configuration_served_shape,
};

use super::{AppState, api_error};

pub(super) const WITNESS_CONFIGURATION_PATH_ENV: &str = "TRACE_COMMONS_WITNESS_CONFIGURATION_PATH";
pub(super) const LABEL_NOT_PUBLISHED: &str = "witness_configuration_not_published";
pub(super) const LABEL_UNAVAILABLE: &str = "witness_configuration_unavailable";

pub(super) fn path_from_env() -> Option<Arc<PathBuf>> {
    path_from_value(
        std::env::var(WITNESS_CONFIGURATION_PATH_ENV)
            .ok()
            .as_deref(),
    )
}

pub(super) fn path_from_value(value: Option<&str>) -> Option<Arc<PathBuf>> {
    let value = value?.trim();
    (!value.is_empty()).then(|| Arc::new(PathBuf::from(value)))
}

pub(super) async fn witness_configuration_handler(
    State(state): State<Arc<AppState>>,
) -> axum::response::Response {
    let Some(path) = state.witness_configuration_path.clone() else {
        return api_error(StatusCode::NOT_FOUND, LABEL_NOT_PUBLISHED).into_response();
    };
    match tokio::task::spawn_blocking(move || read_servable(&path)).await {
        Ok(Some(bytes)) => served(bytes),
        _ => {
            tracing::warn!(event = LABEL_UNAVAILABLE, "witness configuration");
            api_error(StatusCode::SERVICE_UNAVAILABLE, LABEL_UNAVAILABLE).into_response()
        }
    }
}

fn read_servable(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(WITNESS_CONFIGURATION_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    check_configuration_served_shape(&bytes).ok()?;
    Some(bytes)
}

fn served(bytes: Vec<u8>) -> axum::response::Response {
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
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
