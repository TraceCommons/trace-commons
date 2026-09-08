//! Obtaining a NEAR AI inference credential, on this machine, for this
//! contributor.
//!
//! The shape of the flow is forced by the service rather than chosen. Every
//! management route on cloud-api is authenticated by a session JWT and every
//! inference route by an `sk-` API key, and the two middlewares are disjoint
//! with no fallback in either direction -- a session cannot make an inference
//! call, and a key cannot read or mint anything. So signing in is not the end
//! of the flow, it is the beginning of one:
//!
//! ```text
//! browser sign in  ->  rt_ refresh token + session JWT   (loopback)
//!                  ->  GET  /v1/organizations            -> org
//!                  ->  GET  /v1/organizations/{org}/workspaces -> workspace
//!                  ->  POST /v1/workspaces/{ws}/api-keys -> sk- key  [store this]
//! ```
//!
//! The last call returns the plaintext key once and accepts an omitted
//! `expires_at`, so the credential we keep does not expire and the session
//! that minted it is discarded immediately. That is the point of doing it this
//! way: an inference credential with no refresh story at all beats a session
//! this daemon would have to keep alive for the rest of its life.

pub mod api;
pub mod ceremony;
pub mod loopback;

use crate::daemon::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};

/// Begin a ceremony and hand back where to open the browser.
///
/// Async, and on the async dispatch path, because it binds the listener
/// before it answers -- see [`ceremony::begin`] for why that ordering is not
/// negotiable.
pub async fn handle_start(shared: &DaemonShared, req: &Request) -> Response {
    let provider = req
        .params
        .get("provider")
        .and_then(|v| v.as_str())
        .unwrap_or("github");
    match ceremony::begin(&shared.store, provider).await {
        Ok(value) => Response::ok(req.id, value),
        // One label for every failure. The service's own refusal is a 400
        // with an empty body, so there is nothing more specific to say that
        // would be true, and a label that carried more would be carrying a
        // guess.
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "near_ai_credential_unavailable"),
    }
}

pub fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let attempt = req.params.get("attempt_id").and_then(|v| v.as_str());
    match ceremony::status(shared.store.dir(), attempt) {
        Some(status) => Response::ok(req.id, serde_json::to_value(&status).unwrap_or_default()),
        None => Response::err(req.id, ERR_BAD_PARAMS, "near_ai_credential_unknown"),
    }
}

pub fn handle_cancel(shared: &DaemonShared, req: &Request) -> Response {
    let attempt = req.params.get("attempt_id").and_then(|v| v.as_str());
    match ceremony::cancel(shared.store.dir(), attempt) {
        Some(status) => Response::ok(req.id, serde_json::to_value(&status).unwrap_or_default()),
        None => Response::err(req.id, ERR_BAD_PARAMS, "near_ai_credential_unknown"),
    }
}

/// Forget the stored credential.
///
/// `revoked: false` is not a placeholder. Forgetting is local: the key stays
/// valid at the service until the contributor revokes it there, and this
/// cannot do it for them because revoking needs a session and the session was
/// discarded the moment the key was minted. Saying `removed` and meaning
/// "revoked" would be the kind of claim this codebase does not make.
pub fn handle_forget(shared: &DaemonShared, req: &Request) -> Response {
    match ceremony::forget(&shared.store) {
        Ok(removed) => Response::ok(
            req.id,
            serde_json::json!({ "removed": removed, "revoked": false }),
        ),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "near_ai_credential_unavailable"),
    }
}
