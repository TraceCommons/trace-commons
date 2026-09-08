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
//! `expires_at`, so the credential inference uses does not expire and needs no
//! refresh story at all. That remains the point of doing it this way.
//!
//! The session is nonetheless **retained**, which is a change from how this
//! module first shipped and is worth stating rather than leaving to be
//! noticed. The reason is the account balance: `usage/balance` is a management
//! route like every other, `session_token`-only, and the `sk-` key is a 401 on
//! it. There is no narrower credential that can read what a contributor has
//! left. So the refresh token is kept -- rotated at
//! `POST /v1/users/me/access-tokens`, no browser round trip -- and it is a
//! wider credential than the key beside it: it can mint further API keys and
//! read organization and workspace state. See
//! [`crate::daemon::settings::NearAiSession`] for the whole of that statement,
//! and [`balance`] for the rules the reading itself keeps.

pub mod api;
pub mod balance;
pub mod ceremony;
pub mod loopback;

use crate::daemon::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};

/// The state labels `near_ai_credential_status` reports, and the only
/// vocabulary a shell branches on.
///
/// ONE LABEL, NOT THREE BOOLEANS. A shell asking "is a key stored", "is a
/// ceremony running" and "did the last one fail" separately is a shell
/// deciding, three times, in three languages, what this decides once -- and
/// the first of those decisions to drift is the one that offers a second
/// sign-in to somebody who already has a key.
///
/// Spelled here, beside the code that produces them, and re-exported by
/// [`crate::private_inference_copy`] rather than respelled there: a label
/// written twice is two labels that have not disagreed yet.
///
/// There is deliberately no `unknown` label. A running daemon has already
/// loaded the settings it would have to read, so it always knows; the
/// unknown case belongs to a SHELL whose call failed or whose daemon is too
/// old to answer, and its sentences live on the shell side of the table --
/// see `credential_state_line`.
pub const LABEL_CREDENTIAL_ABSENT: &str = "absent";
/// A ceremony is in flight for this configuration directory.
pub const LABEL_CREDENTIAL_OBTAINING: &str = "obtaining";
/// The most recent ceremony ended without a key, and none is stored.
pub const LABEL_CREDENTIAL_FAILED: &str = "failed";
/// The most recent ceremony was stopped by the contributor.
pub const LABEL_CREDENTIAL_CANCELLED: &str = "cancelled";
/// A key is stored on this machine.
pub const LABEL_CREDENTIAL_PRESENT: &str = "present";

/// The one state this machine is in, resolved in one place.
///
/// The precedence is the whole of the decision, and it is here rather than
/// in three shells:
///
/// 1. **A ceremony in flight wins over everything**, including a key that is
///    already stored. Somebody with a browser tab open asked for this and is
///    waiting on it; a card saying "a key is kept here" while they wait
///    describes the past.
/// 2. A stored key is `present`.
/// 3. Otherwise the last attempt's ending is reported -- `failed` or
///    `cancelled` -- because "nothing is stored" and "your attempt failed
///    two minutes ago" are different things to be told.
/// 4. Otherwise `absent`.
///
/// A finished ceremony whose key was later forgotten reads `absent`, which
/// is what it is: `complete` describes an attempt, and the question here is
/// what this machine holds now.
fn state_from(attempt: Option<&str>, stored: bool) -> &'static str {
    match attempt {
        Some("starting" | "waiting_for_browser") => LABEL_CREDENTIAL_OBTAINING,
        _ if stored => LABEL_CREDENTIAL_PRESENT,
        Some("failed") => LABEL_CREDENTIAL_FAILED,
        Some("cancelled") => LABEL_CREDENTIAL_CANCELLED,
        _ => LABEL_CREDENTIAL_ABSENT,
    }
}

/// This machine's credential state, as `near_ai_credential_status` reports it.
pub fn credential_state(shared: &DaemonShared) -> &'static str {
    let stored = shared
        .settings
        .lock()
        .expect("settings lock")
        .near_ai_inference
        .is_some();
    state_from(ceremony::attempt_status(shared.store.dir()), stored)
}

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
    if reports_a_finished_ceremony(&response) {
        shared.reconcile_private_inference().await;
    }
    response
}

/// The field `handle_status` writes the ceremony's lifecycle word into.
///
/// Named once because two places depend on the spelling and they are not
/// adjacent: `handle_status` writes it, `handle_status_async` reads it back to
/// decide whether to reconcile. When those were two literals, renaming the
/// field on the writing side left the reading side matching a key that was no
/// longer emitted -- so the wrapper silently never reconciled, the proxy kept
/// running on the old key, and every test still passed.
const ATTEMPT_STATUS_FIELD: &str = "attempt_status";

/// Whether this status response is the one a shell is waiting for.
///
/// Split out of [`handle_status_async`] so it can be tested without standing
/// up a daemon: the wrapper's only judgement is this predicate, and the
/// failure worth catching is it reading a field nobody writes.
fn reports_a_finished_ceremony(response: &Response) -> bool {
    response
        .result
        .as_ref()
        .and_then(|v| v.get(ATTEMPT_STATUS_FIELD))
        .and_then(|v| v.as_str())
        == Some(ceremony::STATUS_COMPLETE)
}

/// What this machine holds, and -- for a caller that can name the attempt --
/// how that attempt is going.
///
/// `state` is always present, and a caller that names no attempt still gets
/// one. It used to refuse instead, on the reasoning that the ceremony's
/// existence is not readable by every caller of the socket; that guard is
/// intact and is why `attempt_id` and `attempt_status` are echoed only to a
/// caller that already knew the id. What changed is that the RESTING state
/// is not a secret and was the only thing a shell needed most of the time:
/// refusing it left every shell to infer "no key here" from an error, which
/// is the inference that ends with a contributor holding two keys.
///
/// The browser URL is still handed out exactly once, by `start`, and no poll
/// re-serves it.
pub fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    let attempt = req.params.get("attempt_id").and_then(|v| v.as_str());
    let mut body = serde_json::json!({ "state": credential_state(shared) });
    // `attempt_status` and not `status`: the ceremony's own lifecycle word
    // sits beside a `state` that is a fact about the machine, and one field
    // called `status` next to another called `state` is a shell reading the
    // wrong one.
    if let Some(attempt) = ceremony::status(shared.store.dir(), attempt) {
        body["attempt_id"] = serde_json::Value::String(attempt.attempt_id);
        body[ATTEMPT_STATUS_FIELD] = serde_json::Value::String(attempt.status.to_string());
    }
    Response::ok(req.id, body)
}

pub fn handle_cancel(shared: &DaemonShared, req: &Request) -> Response {
    let attempt = req.params.get("attempt_id").and_then(|v| v.as_str());
    match ceremony::cancel(shared.store.dir(), attempt) {
        Some(status) => Response::ok(req.id, serde_json::to_value(&status).unwrap_or_default()),
        None => Response::err(req.id, ERR_BAD_PARAMS, "near_ai_credential_unknown"),
    }
}

/// Forget the stored credential, and the session stored beside it.
///
/// `revoked: false` is not a placeholder. Forgetting is local: the key stays
/// valid at the service until the contributor revokes it there, and this does
/// not do it for them. Saying `removed` and meaning "revoked" would be the
/// kind of claim this codebase does not make.
///
/// The running daemon's own copy is cleared too, not just the file. The
/// settings on disk are what survive a restart, but the in-memory copy is what
/// every read in this process consults -- `reconcile_private_inference` hands
/// the inference key to IronWire from it, and [`balance::read`] takes the
/// refresh token from it. Clearing one and not the other would leave a
/// contributor told their credentials were forgotten while this process went
/// on using both until something restarted it.
///
/// The memory clear is conditional on the disk write having succeeded. The
/// other order -- clear memory, then fail to write -- reports a removal that
/// the next start silently undoes.
pub fn handle_forget(shared: &DaemonShared, req: &Request) -> Response {
    match ceremony::forget(&shared.store) {
        Ok(removed) => {
            {
                let mut settings = shared.settings.lock().expect("settings lock");
                settings.near_ai_inference = None;
                settings.near_ai_session = None;
            }
            // Any balance this directory had cached was read with the session
            // just removed. Serving it again would put a figure from a
            // forgotten account on screen.
            balance::invalidate(shared.store.dir());
            Response::ok(
                req.id,
                serde_json::json!({ "removed": removed, "revoked": false }),
            )
        }
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

/// What the contributor's NEAR AI account has left.
///
/// Always `ok`, never an IPC error, and that is the design rather than
/// laziness: every way of not knowing is a *named state* in the body -- see
/// [`balance::BalanceReport`] -- and an error response would collapse four
/// different things a shell must say into one. The one thing this can never
/// answer with is a number it does not have.
///
/// Async because it may exchange a refresh token and then make two management
/// calls; on the async dispatch path for the same reason `handle_start` is.
pub async fn handle_balance(shared: &DaemonShared, req: &Request) -> Response {
    Response::ok(req.id, balance::read(shared).await.to_value())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The precedence, pinned. Each arm here is a sentence a contributor
    /// reads, and the ordering is the part a shell would otherwise invent.
    #[test]
    fn a_ceremony_in_flight_outranks_a_key_already_stored() {
        // The case that matters: a contributor with a key who started
        // another ceremony is waiting on the browser, not looking at the key
        // they already had.
        assert_eq!(
            state_from(Some("waiting_for_browser"), true),
            LABEL_CREDENTIAL_OBTAINING
        );
        assert_eq!(
            state_from(Some("starting"), false),
            LABEL_CREDENTIAL_OBTAINING
        );
        assert_eq!(state_from(None, true), LABEL_CREDENTIAL_PRESENT);
        // A finished ceremony says nothing on its own: what is stored now is
        // the question, and `complete` answers the wrong one.
        assert_eq!(state_from(Some("complete"), true), LABEL_CREDENTIAL_PRESENT);
        assert_eq!(state_from(Some("complete"), false), LABEL_CREDENTIAL_ABSENT);
        // An ending is reported only when there is nothing to report
        // instead. A stored key outranks a failure that came after it.
        assert_eq!(state_from(Some("failed"), false), LABEL_CREDENTIAL_FAILED);
        assert_eq!(state_from(Some("failed"), true), LABEL_CREDENTIAL_PRESENT);
        assert_eq!(
            state_from(Some("cancelled"), false),
            LABEL_CREDENTIAL_CANCELLED
        );
        assert_eq!(state_from(None, false), LABEL_CREDENTIAL_ABSENT);
        // A lifecycle word from a later build claims nothing beyond what is
        // stored, and never invents an in-flight ceremony.
        assert_eq!(
            state_from(Some("a_status_from_a_later_build"), false),
            LABEL_CREDENTIAL_ABSENT
        );
    }

    /// The reconcile trigger reads the field the handler actually writes.
    ///
    /// This is the failure that made it worth a test: `handle_status` used to
    /// call the field `status`, PR #718's wrapper matched on `status`, and
    /// renaming the field to `attempt_status` on the writing side left the
    /// wrapper matching a key nobody emitted. Reconcile then never ran, a
    /// contributor's freshly minted key never reached the running proxy, and
    /// all 1634 tests still passed. The body here is built the way
    /// `handle_status` builds it -- through the same const -- so the two can
    /// no longer drift apart silently.
    #[test]
    fn a_completed_ceremony_is_recognised_through_the_field_the_handler_writes() {
        let mut body = serde_json::json!({ "state": LABEL_CREDENTIAL_PRESENT });
        body[ATTEMPT_STATUS_FIELD] =
            serde_json::Value::String(ceremony::STATUS_COMPLETE.to_string());
        assert!(
            reports_a_finished_ceremony(&Response::ok(1, body)),
            "the wrapper stopped seeing the field handle_status writes"
        );
    }

    /// Nothing else triggers a reconcile.
    ///
    /// A resting response carries no attempt at all, and an in-flight one
    /// carries a word that is not `complete`. Neither is a moment when a new
    /// key arrived, and reconciling on them would take the lifecycle lock
    /// once a second for the length of a browser sign-in.
    #[test]
    fn an_unfinished_or_absent_ceremony_triggers_nothing() {
        let resting = serde_json::json!({ "state": LABEL_CREDENTIAL_ABSENT });
        assert!(!reports_a_finished_ceremony(&Response::ok(1, resting)));

        for word in ["starting", "waiting_for_browser", "failed", "cancelled"] {
            let mut body = serde_json::json!({ "state": LABEL_CREDENTIAL_OBTAINING });
            body[ATTEMPT_STATUS_FIELD] = serde_json::Value::String(word.to_string());
            assert!(
                !reports_a_finished_ceremony(&Response::ok(1, body)),
                "{word} was treated as a finished ceremony"
            );
        }
    }

    /// The labels are wire values. A shell is built against these spellings
    /// and a rename is a shell that renders the unknown sentence forever.
    #[test]
    fn the_labels_are_the_documented_wire_values() {
        assert_eq!(LABEL_CREDENTIAL_ABSENT, "absent");
        assert_eq!(LABEL_CREDENTIAL_OBTAINING, "obtaining");
        assert_eq!(LABEL_CREDENTIAL_FAILED, "failed");
        assert_eq!(LABEL_CREDENTIAL_CANCELLED, "cancelled");
        assert_eq!(LABEL_CREDENTIAL_PRESENT, "present");
        // The field name is a wire value too. Three shells read it out of
        // this response, and it is deliberately not `status` -- beside a
        // `state` that means something else, that spelling is a shell
        // reading the wrong field.
        assert_eq!(ATTEMPT_STATUS_FIELD, "attempt_status");
        assert_eq!(ceremony::STATUS_COMPLETE, "complete");
    }
}
