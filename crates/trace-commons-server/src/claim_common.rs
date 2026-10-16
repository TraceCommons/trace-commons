// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Plumbing shared by the self-serve invite claim surfaces
//! ([`crate::near_legion_claim`] and [`crate::celestine_sloth_claim`]).
//!
//! Deliberately narrow. The two modules stay parallel on purpose: the chains,
//! the signature schemes, the address formats and the ownership queries all
//! differ, and the module doc on `celestine_sloth_claim` explains why folding
//! those together would be a chain-shaped enum rather than reuse. Nothing here
//! touches a `ClaimError`, a handler, or a signature scheme. What lives here is
//! the part that was already verbatim-identical in both: environment reading,
//! the refusal encoding, and the CORS layer.

use axum::Json;
use axum::http::StatusCode;

/// Read an environment variable, trimmed, treating blank as absent.
pub(crate) fn non_blank_env(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Split a comma-separated denylist, trimming entries and dropping empties.
pub(crate) fn parse_denylist(raw: &str) -> Vec<String> {
    raw.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// Whether an enable-flag environment variable is set to a truthy value.
///
/// The accepted set is exactly `1`, `true` and `yes`, case-insensitively, and
/// an absent or unrecognised value is false. Widening it is a behaviour change
/// to a fail-closed switch: an operator who writes `on` gets a 404 surface, and
/// that is the safe direction.
pub(crate) fn truthy_env(key: &str) -> bool {
    std::env::var(key)
        .ok()
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            v == "1" || v == "true" || v == "yes"
        })
        .unwrap_or(false)
}

/// A claim refusal that can be rendered on the wire.
///
/// Implemented by each surface's own `ClaimError`. The error enums stay
/// separate -- their variants are the surfaces' distinct vocabularies, and the
/// labels are wire values the claim pages switch on -- but the encoding of a
/// refusal into a response is the same act in both.
pub(crate) trait ClaimRefusal: Copy {
    /// Stable public wire label. Never includes internal detail.
    fn public_label(self) -> &'static str;
    /// HTTP status this refusal is returned with.
    fn status(self) -> u16;
}

/// Encode a refusal as its status plus its public label, and nothing else.
pub(crate) fn claim_refusal<E: ClaimRefusal>(error: E) -> (StatusCode, Json<serde_json::Value>) {
    let status = StatusCode::from_u16(error.status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (
        status,
        Json(serde_json::json!({ "error": error.public_label() })),
    )
}

/// Allowed browser origins for a claim surface, from `origins_env_key`.
///
/// The claim pages are served from the community site, a different origin from
/// this issuer, so these routes need CORS of their own rather than relying on a
/// reverse proxy to add it. `default_origins` matches the community surface's
/// own default origin list.
pub(crate) fn claim_cors_layer(
    origins_env_key: &str,
    default_origins: &str,
) -> tower_http::cors::CorsLayer {
    use axum::http::HeaderValue;
    use axum::http::header::{ACCEPT, CONTENT_TYPE};

    let configured = std::env::var(origins_env_key).unwrap_or_else(|_| default_origins.to_string());
    let origins: Vec<HeaderValue> = configured
        .split(',')
        .map(str::trim)
        .filter(|o| !o.is_empty())
        .filter_map(|o| HeaderValue::from_str(o).ok())
        .collect();

    // `AllowOrigin::list` panics on a wildcard entry, and `*` is the most
    // natural thing an operator writes for "allow everything". Map it rather
    // than crash router construction at startup.
    let allow_origin = if configured.split(',').any(|o| o.trim() == "*") {
        tower_http::cors::AllowOrigin::any()
    } else {
        tower_http::cors::AllowOrigin::list(origins)
    };

    tower_http::cors::CorsLayer::new()
        .allow_origin(allow_origin)
        .allow_methods([axum::http::Method::GET, axum::http::Method::POST])
        .allow_headers([ACCEPT, CONTENT_TYPE])
        .max_age(std::time::Duration::from_secs(600))
}

/// The default CORS origin list both claim surfaces ship with.
pub(crate) const DEFAULT_CLAIM_CORS_ORIGINS: &str =
    "https://tracecommons.ai,http://localhost:4321,http://localhost:8788";

/// Test doubles shared by both claim surfaces' router tests.
///
/// The harnesses themselves stay per-module: they build different state types
/// from different config, and a shared one would be two constructors wearing a
/// single name. What is shared is what was byte-identical -- the grant-table
/// fake, the three-way chain answer, and the request/response plumbing.
#[cfg(test)]
pub(crate) mod claim_test_support {
    use anyhow::{Result, anyhow};
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::sync::Mutex;
    use tower::ServiceExt as _;

    /// In-memory stand-in for the grant table. Reproduces the one behaviour the
    /// handlers depend on: the V42 partial unique index refusing a second live
    /// grant for the same credential in the same pool.
    #[derive(Default)]
    pub(crate) struct FakeSink {
        pub(crate) grants: Mutex<Vec<crate::db::InviteGrantWrite>>,
        pub(crate) fail_count: bool,
        pub(crate) fail_insert: bool,
    }

    #[async_trait::async_trait]
    impl crate::near_legion_claim::InviteGrantSink for FakeSink {
        async fn count_live(&self, policy_label: &str) -> Result<u32> {
            if self.fail_count {
                return Err(anyhow!("count unavailable"));
            }
            let g = self.grants.lock().unwrap();
            Ok(g.iter().filter(|w| w.policy_label == policy_label).count() as u32)
        }

        async fn count_bound(&self, policy_label: &str) -> Result<u32> {
            // The fake carries no expiry semantics, so this matches count_live.
            // The distinction that matters in production -- an expired grant
            // still occupying the uniqueness index -- is a property of the SQL
            // predicates and belongs to a database test, not this fake.
            self.count_live(policy_label).await
        }

        async fn credential_bound_in_any(
            &self,
            policy_labels: &[String],
            credential_binding_hash: &str,
        ) -> Result<bool> {
            if self.fail_count {
                return Err(anyhow!("count unavailable"));
            }
            let g = self.grants.lock().unwrap();
            Ok(g.iter().any(|w| {
                policy_labels.contains(&w.policy_label)
                    && w.credential_binding_hash.as_deref() == Some(credential_binding_hash)
            }))
        }

        async fn insert(
            &self,
            write: crate::db::InviteGrantWrite,
        ) -> Result<crate::db::InviteGrantInsertOutcome> {
            if self.fail_insert {
                return Err(anyhow!("insert unavailable"));
            }
            let mut g = self.grants.lock().unwrap();
            let bound = g.iter().any(|w| {
                w.policy_label == write.policy_label
                    && w.credential_binding_hash.is_some()
                    && w.credential_binding_hash == write.credential_binding_hash
            });
            if bound {
                return Ok(crate::db::InviteGrantInsertOutcome::CredentialAlreadyBound);
            }
            g.push(write);
            Ok(crate::db::InviteGrantInsertOutcome::Inserted)
        }
    }

    /// What a faked chain lookup answers: holds, does not hold, or is down.
    #[derive(Clone, Copy)]
    pub(crate) enum Answer {
        Yes,
        No,
        Fails,
    }

    /// Drive one request through a claim router and decode the JSON body.
    pub(crate) async fn call_router(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if body.is_some() {
            builder = builder.header("content-type", "application/json");
        }
        let request = builder
            .body(
                body.map(|b| Body::from(b.to_string()))
                    .unwrap_or_else(Body::empty),
            )
            .unwrap();
        let response = app.oneshot(request).await.unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        let json = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, json)
    }

    /// The refusal label in a response body, or a readable stand-in.
    pub(crate) fn error_of(body: &serde_json::Value) -> &str {
        body["error"].as_str().unwrap_or("<no error field>")
    }
}

#[cfg(test)]
mod tests {
    use super::parse_denylist;

    /// The one test of the denylist parser both claim surfaces share; the
    /// inputs are a NEAR and a Cosmos list.
    #[test]
    fn denylist_parsing_trims_and_drops_empties() {
        assert_eq!(
            parse_denylist("a.near, b.near ,,  c.near  "),
            vec!["a.near", "b.near", "c.near"]
        );
        assert_eq!(
            parse_denylist("cosmos1a, cosmos1b ,,  cosmos1c  "),
            vec!["cosmos1a", "cosmos1b", "cosmos1c"]
        );
        assert!(parse_denylist("  ,, ").is_empty());
    }
}
