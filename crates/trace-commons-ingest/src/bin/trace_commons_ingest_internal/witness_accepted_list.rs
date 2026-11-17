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
//! Explicit `witness_url` queries select a file from the optional directory,
//! keyed only by the SHA-256 of the URL without trailing slashes. A legacy
//! file is a fallback only when its signed payload names that same URL.
//!
//! The file is read on every request, so installing a new list needs no
//! restart. Fail closed: with no setting the route answers 404; a setting
//! whose file is missing, unreadable, too large, or not a well-formed signed
//! list answers 503. An unsigned or mangled list is never served. Only fixed
//! labels are returned or logged, never the path.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode, Uri};
use axum::response::IntoResponse;
use serde::Deserialize;
use sha2::{Digest as _, Sha256};
use trace_commons_protocol::witness_accepted_list::{
    AcceptedList, WITNESS_ACCEPTED_LIST_MAX_BYTES, check_served_shape,
};

use super::{AppState, api_error};

/// The setting naming the signed list file.
pub(super) const WITNESS_ACCEPTED_LIST_PATH_ENV: &str = "TRACE_COMMONS_WITNESS_ACCEPTED_LIST_PATH";

/// The setting naming the directory of URL-specific signed list files.
pub(super) const WITNESS_ACCEPTED_LIST_DIRECTORY_ENV: &str =
    "TRACE_COMMONS_WITNESS_ACCEPTED_LIST_DIRECTORY";

/// The 400 label: the optional witness URL query is invalid.
const LABEL_QUERY_INVALID: &str = "witness_accepted_list_query_invalid";

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

pub(super) fn directory_from_env() -> Option<Arc<PathBuf>> {
    path_from_value(
        std::env::var(WITNESS_ACCEPTED_LIST_DIRECTORY_ENV)
            .ok()
            .as_deref(),
    )
}

pub(super) fn path_from_value(value: Option<&str>) -> Option<Arc<PathBuf>> {
    let value = value?.trim();
    (!value.is_empty()).then(|| Arc::new(PathBuf::from(value)))
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ListQuery {
    witness_url: Option<String>,
}

/// Answers `GET /v1/public/witness-accepted-list` without fetching any URL.
pub(super) async fn witness_accepted_list_handler(
    State(state): State<Arc<AppState>>,
    uri: Uri,
) -> axum::response::Response {
    let witness_url = match requested_witness(&uri) {
        Ok(url) => url,
        Err(()) => return api_error(StatusCode::BAD_REQUEST, LABEL_QUERY_INVALID).into_response(),
    };
    let legacy = state.witness_accepted_list_path.clone();
    let directory = state.witness_accepted_list_directory.clone();
    let result = tokio::task::spawn_blocking(move || match witness_url {
        Some(url) => read_for_witness(
            directory.as_deref().map(PathBuf::as_path),
            legacy.as_deref().map(PathBuf::as_path),
            &url,
        ),
        // Unqualified requests retain the legacy file's behavior, including
        // its byte-for-byte body and missing-file 503.
        None => match legacy {
            Some(path) => read_servable(&path).ok_or(ServeError::Unavailable),
            None => Err(ServeError::NotPublished),
        },
    })
    .await;
    match result {
        Ok(Ok(bytes)) => served(bytes),
        Ok(Err(ServeError::NotPublished)) => {
            api_error(StatusCode::NOT_FOUND, LABEL_NOT_PUBLISHED).into_response()
        }
        _ => {
            tracing::warn!(event = LABEL_UNAVAILABLE, "witness accepted list");
            api_error(StatusCode::SERVICE_UNAVAILABLE, LABEL_UNAVAILABLE).into_response()
        }
    }
}

fn requested_witness(uri: &Uri) -> Result<Option<String>, ()> {
    let query = uri.query().unwrap_or_default();
    // A fully percent-encoded maximum URL plus the field name fits this
    // bound. Check before deserializing or allocating decoded query text.
    if query.len() > 3 * 2048 + 32 || !well_formed_query_encoding(query) {
        return Err(());
    }
    if query.is_empty() {
        return Ok(None);
    }
    let Query(query) = Query::<ListQuery>::try_from_uri(uri).map_err(|_| ())?;
    let url = query.witness_url.ok_or(())?;
    if url.len() > 2048
        || !url.starts_with("https://")
        || url.chars().any(|c| c.is_whitespace() || c.is_control())
        || url.contains('\\')
    {
        return Err(());
    }
    let parsed = reqwest::Url::parse(&url).map_err(|_| ())?;
    let authority = url
        .strip_prefix("https://")
        .unwrap_or_default()
        .split('/')
        .next()
        .unwrap_or_default();
    if parsed.scheme() != "https"
        || parsed.host_str().is_none()
        || !parsed.username().is_empty()
        || parsed.password().is_some()
        || authority.is_empty()
        || authority.contains('@')
        || parsed.query().is_some()
        || parsed.fragment().is_some()
    {
        return Err(());
    }
    Ok(Some(url.trim_end_matches('/').to_string()))
}

// Form decoding is otherwise permissive about malformed escapes and invalid
// UTF-8. Refuse both rather than route a lossy replacement of a requested URL.
fn well_formed_query_encoding(query: &str) -> bool {
    let mut decoded = Vec::with_capacity(query.len());
    let mut bytes = query.bytes();
    while let Some(byte) = bytes.next() {
        if byte == b'%' {
            let Some(high) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                return false;
            };
            let Some(low) = bytes.next().and_then(|b| (b as char).to_digit(16)) else {
                return false;
            };
            decoded.push((high * 16 + low) as u8);
        } else {
            decoded.push(byte);
        }
    }
    std::str::from_utf8(&decoded).is_ok()
}

enum ServeError {
    NotPublished,
    Unavailable,
}

fn read_for_witness(
    directory: Option<&Path>,
    legacy: Option<&Path>,
    url: &str,
) -> Result<Vec<u8>, ServeError> {
    if let Some(directory) = directory {
        // Never join a requested URL or URL path to an operator directory.
        let filename = format!("{}.json", hex::encode(Sha256::digest(url.as_bytes())));
        match read_document(&directory.join(filename)) {
            Ok((bytes, list)) if list.names_witness(url) => return Ok(bytes),
            Ok(_) | Err(ReadError::Unavailable) => return Err(ServeError::Unavailable),
            Err(ReadError::NotFound) => (),
        }
    }
    let Some(legacy) = legacy else {
        return Err(ServeError::NotPublished);
    };
    let (bytes, list) = read_document(legacy).map_err(|_| ServeError::Unavailable)?;
    if list.names_witness(url) {
        Ok(bytes)
    } else {
        Err(ServeError::NotPublished)
    }
}

enum ReadError {
    NotFound,
    Unavailable,
}

/// The file's bytes, if it exists, is within the size bound and is a
/// well-formed signed list.
pub(super) fn read_servable(path: &Path) -> Option<Vec<u8>> {
    read_document(path).ok().map(|(bytes, _)| bytes)
}

fn read_document(path: &Path) -> Result<(Vec<u8>, AcceptedList), ReadError> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            ReadError::NotFound
        } else {
            ReadError::Unavailable
        }
    })?;
    let mut bytes = Vec::new();
    file.take(WITNESS_ACCEPTED_LIST_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ReadError::Unavailable)?;
    let list = check_served_shape(&bytes).map_err(|_| ReadError::Unavailable)?;
    Ok((bytes, list))
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
