// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `POST /v1/public/skills-catalog/suggestions`: an app reports one public
//! skill the Skills catalog lacks (spec 2026-11-15, "Reporting missing
//! skills").
//!
//! Unauthenticated and outside tenant context, like `/v1/usage-events`: a
//! suggestion carries no identity by design, only a public GitHub repository,
//! a path to a `SKILL.md` and a commit, validated whole against the
//! protocol's `SkillSuggestion`. It is a candidate, never content: the
//! crawler later fetches the repository itself and publishes nothing that
//! fails the catalog's checks.
//!
//! Abuse limits: per address and global rate limits in memory (the address
//! is never stored), and V160's function refuses new rows while 10,000 are
//! pending. Logs carry fixed labels only, never the repository, the path or
//! the address.

use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use trace_commons_protocol::skills_catalog::SkillSuggestion;
use trace_commons_server::db::{Database, SkillsCatalogSuggestionOutcome};

use super::{ACCOUNT_RATE_LIMITER, AppState, api_error, client_ip_for_rate_limit};

/// Suggestions per minute from one address. An app sends at most four a day.
const SUGGESTIONS_PER_IP_LIMIT: u32 = 10;
/// Suggestions per minute from everyone.
const SUGGESTIONS_GLOBAL_LIMIT: u32 = 600;

pub(super) async fn skills_catalog_suggestion_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let client_ip = client_ip_for_rate_limit(&headers);
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("skills-suggestions-ip:{client_ip}"),
        SUGGESTIONS_PER_IP_LIMIT,
    ) || !ACCOUNT_RATE_LIMITER
        .check_global("skills-suggestions-global", SUGGESTIONS_GLOBAL_LIMIT)
    {
        return api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    record(state.db_mirror.as_ref(), &body).await
}

/// Everything after the rate limit: parse, store, answer.
pub(super) async fn record(
    db: Option<&Arc<dyn Database>>,
    body: &[u8],
) -> axum::response::Response {
    let suggestion = match SkillSuggestion::parse(body) {
        Ok(suggestion) => suggestion,
        Err(label) => return api_error(StatusCode::BAD_REQUEST, label).into_response(),
    };
    let Some(db) = db else {
        return api_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "skills_catalog_unavailable",
        )
        .into_response();
    };
    match db.record_skills_catalog_suggestion(&suggestion).await {
        Ok(SkillsCatalogSuggestionOutcome::Stored | SkillsCatalogSuggestionOutcome::Counted) => {
            StatusCode::ACCEPTED.into_response()
        }
        Ok(SkillsCatalogSuggestionOutcome::Full) => {
            tracing::warn!(
                reason = "skills_catalog_suggestions_full",
                "skills catalog suggestion not stored"
            );
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "skills_catalog_suggestions_full",
            )
            .into_response()
        }
        Err(_) => {
            tracing::warn!(
                reason = "skills_catalog_suggestion_store_failed",
                "skills catalog suggestion not stored"
            );
            api_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "skills_catalog_unavailable",
            )
            .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn status_and_body(response: axum::response::Response) -> (StatusCode, String) {
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[test]
    fn the_route_literal_is_the_protocol_path() {
        assert_eq!(
            trace_commons_protocol::skills_catalog::SUGGESTIONS_PATH,
            "/v1/public/skills-catalog/suggestions"
        );
        assert!(
            include_str!("../trace-commons-ingest.rs")
                .contains("\"/v1/public/skills-catalog/suggestions\",")
        );
    }

    #[tokio::test]
    async fn a_malformed_suggestion_is_refused_with_its_label() {
        for (body, label) in [
            (&b"not json"[..], "skills-suggestion-malformed"),
            (
                br#"{"repository":"acme/skills","path":"x/SKILL.md","revision":"1111111111111111111111111111111111111111","install_id":"x"}"#,
                "skills-suggestion-malformed",
            ),
            (
                br#"{"repository":"https://github.com/acme/skills","path":"x/SKILL.md","revision":"1111111111111111111111111111111111111111"}"#,
                "skills-suggestion-repository",
            ),
            (
                br#"{"repository":"acme/skills","path":"../SKILL.md","revision":"1111111111111111111111111111111111111111"}"#,
                "skills-suggestion-path",
            ),
        ] {
            let (status, text) = status_and_body(record(None, body).await).await;
            assert_eq!(status, StatusCode::BAD_REQUEST);
            assert!(text.contains(label), "{text}");
        }
    }

    #[tokio::test]
    async fn without_a_database_a_valid_suggestion_is_unavailable() {
        let body = br#"{"repository":"acme/skills","path":"x/SKILL.md","revision":"1111111111111111111111111111111111111111"}"#;
        let (status, text) = status_and_body(record(None, body).await).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert!(text.contains("skills_catalog_unavailable"));
    }
}
