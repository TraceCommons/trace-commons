//! IPC handler for `commons_credit_summary` (K7).
//!
//! Relays `GET /v1/account/credit-summary` -- the commons' own settlement
//! posture and this contributor's points on its ledger -- which the daemon
//! never read before this. See `crate::commons_credit` for the HTTP client
//! this wraps and the full account-session rationale; this module is the
//! IPC-side half: request shape, the account-session credential handling
//! `daemon::withdraw` already established, and the always-succeeds contract.
//!
//! # This is a relay, not a mutation: it always succeeds
//!
//! Unlike `withdraw`, nothing here changes state on the server, so there is
//! no reason to make a caller handle a distinct `account-session-required`
//! error. Like `near_ai_balance`, this method answers with `state: "known"`
//! or `state: "unknown"` rather than an IPC error -- a missing session, a
//! transport failure, and a non-200 response all collapse to `"unknown"`.
//! Every numeric and string field is present and `null` in that state,
//! matching `near_ai_balance`'s own rule: an absent key would be ambiguous
//! between "this build is too old to know" and "this daemon knows it does
//! not know", and a client must never render `null` as zero.
//!
//! # Naming: these are the COMMONS' figures, not near.ai's
//!
//! #1118's open decision #1 asks which ledger is of record: the commons
//! (this route) or near.ai (`near_ai_balance`, dollars). Every field here
//! carries a `commons_` prefix precisely so a client cannot present one
//! ledger's numbers as the other's, and this daemon presents both without
//! preferring either. See `crate::commons_credit`'s module doc.

use super::ipc::{DaemonShared, Request, Response};
use super::withdraw::{account_session, keep_rotated_token};

/// The commons' credit summary, as a client should read it: fixed keys,
/// `null` when `state` is not `known`, and a source-labelled name on every
/// figure. See this module's doc for both rules.
pub(super) async fn handle_commons_credit_summary(
    shared: &DaemonShared,
    req: &Request,
) -> Response {
    let observed_at = chrono::Utc::now();
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::ok(req.id, unknown_value(observed_at));
    };
    let mut session = match account_session(shared) {
        Ok(Some(session)) => session,
        // No session, or the credential store could not be read: this is a
        // fact about the machine, not a caller error -- see the module doc.
        _ => return Response::ok(req.id, unknown_value(observed_at)),
    };
    let call = crate::commons_credit::call_credit_summary(
        &cfg.ingest_url,
        cfg.allowed_hosts.as_deref(),
        &session.session.access_token,
    )
    .await;
    // Stored whatever the result, exactly like `withdraw` and
    // `account_admission`: the server rotates a native session on a
    // refusal too, and dropping this signs the contributor out at the next
    // grace expiry.
    let _ = keep_rotated_token(shared, &mut session, call.rotated_token);
    match call.result {
        Some(summary) => Response::ok(
            req.id,
            serde_json::json!({
                "state": "known",
                "commons_settlement": summary.commons_settlement,
                "commons_settlement_explanation": summary.commons_settlement_explanation,
                "commons_graded": summary.commons_graded,
                "commons_points_earned_this_period": summary.commons_points_earned_this_period,
                "commons_points_lifetime_earned": summary.commons_points_lifetime_earned,
                "commons_pending_review": summary.commons_pending_review,
                "commons_currency_code": summary.commons_currency_code,
                "commons_currency_earned_this_period": summary.commons_currency_earned_this_period,
                "observed_at": observed_at,
            }),
        ),
        None => Response::ok(req.id, unknown_value(observed_at)),
    }
}

fn unknown_value(observed_at: chrono::DateTime<chrono::Utc>) -> serde_json::Value {
    serde_json::json!({
        "state": "unknown",
        "commons_settlement": null,
        "commons_settlement_explanation": null,
        "commons_graded": null,
        "commons_points_earned_this_period": null,
        "commons_points_lifetime_earned": null,
        "commons_pending_review": null,
        "commons_currency_code": null,
        "commons_currency_earned_this_period": null,
        "observed_at": observed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> DaemonShared {
        let (_d, store) = crate::config::tests_support::temp_store();
        std::mem::forget(_d);
        DaemonShared::load(store).unwrap()
    }

    fn req() -> Request {
        Request {
            id: 1,
            method: "commons_credit_summary".to_string(),
            params: serde_json::json!({}),
        }
    }

    /// No enrollment at all: unknown, not an error, and every figure is a
    /// `null` rather than an absent key.
    #[tokio::test]
    async fn with_no_config_the_state_is_unknown_with_every_key_present() {
        let s = shared();
        let r = handle_commons_credit_summary(&s, &req()).await;
        assert!(r.error.is_none(), "this method always succeeds");
        let v = r.result.unwrap();
        assert_eq!(v["state"], "unknown");
        assert!(v["commons_settlement"].is_null());
        assert!(v["commons_points_earned_this_period"].is_null());
        assert!(v["commons_pending_review"].is_null());
        assert!(v.get("observed_at").is_some());
    }

    /// No account session (device-key-only enrollment): the same `unknown`
    /// answer, never a distinct IPC error -- this is a relay, not a
    /// mutating action like `withdraw`.
    #[tokio::test]
    async fn with_no_account_session_the_state_is_unknown() {
        let s = shared();
        let cfg = crate::commands::unenrolled_preview_config();
        s.store.save_config(&cfg).unwrap();
        let r = handle_commons_credit_summary(&s, &req()).await;
        assert!(r.error.is_none());
        assert_eq!(r.result.unwrap()["state"], "unknown");
    }
}
