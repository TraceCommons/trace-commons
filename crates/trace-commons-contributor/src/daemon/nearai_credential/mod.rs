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

/// `handle_status`, plus the thing the shell is actually waiting for.
///
/// A shell polls this until it reads `complete`, and the honest answer to
/// "what changed when it did" used to be "nothing yet": the ceremony had
/// written the key to disk and the daemon would notice on its next poll tick,
/// up to a minute later. So the contributor saw a ceremony succeed and a
/// screen that still said their calls were being answered by accounts already
/// set up on their computer -- which reads as failure, and invites minting a
/// second key.
///
/// Reconciling here is what closes that: the poll that reports `complete` has
/// already cycled the proxy onto the new key, so the state the shell reads
/// next is the true one. It is not a second mechanism -- the poll tick still
/// does this, and a shell that never polls status still converges. This only
/// makes the common path prompt.
///
/// Guarded on `complete` because status is polled repeatedly and stays
/// `complete` forever; reconciling on `waiting_for_browser` would take the
/// lifecycle lock once a second for five minutes to discover nothing changed.
/// Reconcile is idempotent, so the repeat `complete` polls that do reach it
/// are a cheap no-op.
pub async fn handle_status_async(shared: &DaemonShared, req: &Request) -> Response {
    let response = handle_status(shared, req);
    if response
        .result
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(|v| v.as_str())
        == Some("complete")
    {
        shared.reconcile_private_inference().await;
    }
    response
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

/// `handle_forget`, plus the one thing it cannot do synchronously: take the
/// key back out of the running proxy.
///
/// The same shape as `handle_set_settings_async`, and for the same reason --
/// the sync handler is what defines the answer, and the async wrapper is only
/// what makes it true of the machine before it is returned. Withdrawing
/// something is the half of this pair that must not wait: a contributor who
/// forgets a credential has said stop using it, and "stopped, at some point
/// in the next minute" is not that.
pub async fn handle_forget_async(shared: &DaemonShared, req: &Request) -> Response {
    let response = handle_forget(shared, req);
    if response.error.is_none() {
        shared.reconcile_private_inference().await;
    }
    response
}
