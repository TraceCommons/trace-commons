//! IPC handlers for `withdraw` and `withdraw_bulk`.
//!
//! Both call `crate::withdraw::call_withdraw` -- the server's
//! `POST /v1/account/traces/{submission_id}/withdraw` -- for real, update the
//! local history cache (`daemon::history::mark_withdrawn`) with what the
//! server reported, and hand the caller the tier that applied so a UI can
//! tell the contributor the truth rather than a generic success. See
//! `docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md`.
//!
//! # The account session
//!
//! That endpoint is authenticated by an **account session**, not the device
//! key this daemon holds -- withdrawal is meant to survive losing the device
//! that submitted a trace, and a stolen device key must not be worth the
//! ability to delete someone's contribution history.
//!
//! The session comes from `crate::account_auth`: the human completes the
//! ordinary browser login flow, and the browser hands this machine a
//! short-lived bearer token on a loopback redirect. [`account_session`]
//! reads that token, and treats an absent, unparseable, or expired one as
//! absent -- so both handlers below fail closed with
//! [`ERR_ACCOUNT_SESSION_REQUIRED`] rather than making a call that would 401.
//! That is a distinct, documented error rather than a generic failure so a
//! calling shell can route the contributor to `account login`.

use chrono::Utc;
use uuid::Uuid;

use super::history::{self, HistoryCache, STATUS_ACCEPTED, STATUS_QUARANTINED, STATUS_SUBMITTED};
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};

/// Distinct from every other error this daemon returns: a caller that sees
/// this knows exactly what is missing (an account session) rather than
/// having to guess from a generic `unavailable`.
pub const ERR_ACCOUNT_SESSION_REQUIRED: &str = "account-session-required";

/// The account session this daemon presents to the withdrawal endpoint,
/// with the credential snapshot a rotation is stored against.
///
/// `None` whenever the token is missing, unreadable, or expired (or about to
/// be): the caller must then report [`ERR_ACCOUNT_SESSION_REQUIRED`] rather
/// than fall back to the device key, which is deliberately not accepted for
/// withdrawal.
fn account_session(
    shared: &DaemonShared,
) -> anyhow::Result<Option<crate::account_auth::LoadedAccountSession>> {
    super::run_blocking(|| crate::account_auth::try_load_session_with_snapshot(&shared.store))
}

/// Keep the rotated session the server handed back, if it rotated one, and
/// present it from here on. The old token lasts only the server's short
/// grace after a rotation, so dropping this signs the contributor out.
///
/// `false` when a rotation came back and could not be stored.
fn keep_rotated_token(
    shared: &DaemonShared,
    session: &mut crate::account_auth::LoadedAccountSession,
    rotated_token: Option<String>,
) -> bool {
    let Some(rotated_token) = rotated_token else {
        return true;
    };
    let stored =
        super::public_run::persist_rotated_token(shared, session, Some(rotated_token.clone()))
            .is_ok();
    // Reloaded rather than patched, so a second rotation in the same batch is
    // stored against the snapshot the first one left.
    match super::run_blocking(|| crate::account_auth::try_load_session_with_snapshot(&shared.store))
    {
        Ok(Some(reloaded)) if stored => *session = reloaded,
        _ => session.session.access_token = rotated_token,
    }
    stored
}

fn parse_submission_id(params: &serde_json::Value) -> Result<Uuid, &'static str> {
    params
        .get("submission_id")
        .and_then(|v| v.as_str())
        .ok_or("submission_id-required")?
        .parse()
        .map_err(|_| "submission_id-invalid")
}

/// Statuses `withdraw_bulk` accepts: the same three the server reports on
/// submission-status read-back (see `daemon::history`'s `STATUS_*`
/// constants). `withdrawn` itself and the `other` bucket are deliberately
/// not accepted -- withdrawing what is already withdrawn is a no-op with
/// nothing to bulk, and `other` covers statuses this client has no stable
/// name for.
fn valid_bulk_status(s: &str) -> bool {
    matches!(s, STATUS_SUBMITTED | STATUS_QUARANTINED | STATUS_ACCEPTED)
}

/// `distribution_reach` as a wire string, straight from
/// `crate::withdraw::DistributionReach`'s own `Serialize` impl, so this
/// module cannot hand-maintain a second copy of the tier names that drifts
/// from the client's.
fn reach_label(reach: crate::withdraw::DistributionReach) -> serde_json::Value {
    serde_json::to_value(reach).unwrap_or(serde_json::Value::Null)
}

/// Withdraw one submission. See the module doc for why this always answers
/// [`ERR_ACCOUNT_SESSION_REQUIRED`] today.
pub(super) async fn handle_withdraw(shared: &DaemonShared, req: &Request) -> Response {
    let submission_id = match parse_submission_id(&req.params) {
        Ok(id) => id,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let mut session = match account_session(shared) {
        Ok(Some(session)) => session,
        Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED),
        Err(_) => {
            return Response::err(
                req.id,
                ERR_UNAVAILABLE,
                "commons_credential_storage_unavailable",
            );
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let call = crate::withdraw::call_withdraw(
        &cfg.ingest_url,
        cfg.allowed_hosts.as_deref(),
        &session.session.access_token,
        submission_id,
    )
    .await;
    // Whatever the result: the server rotates on a refusal too.
    let credential_persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
    match call.result {
        Ok(outcome) => {
            if let Ok(mut records) = HistoryCache::load(&shared.store) {
                if history::mark_withdrawn(&mut records, submission_id, Utc::now()) {
                    let _ = HistoryCache::save(&shared.store, &records);
                }
            }
            let mut value = serde_json::json!({
                "withdrawn": true,
                "distribution_reach": reach_label(outcome.distribution_reach),
                "token_deletion_note":outcome.token_deletion_state.map(|s| s.note()),
            });
            if !credential_persisted {
                super::public_run::attach_credential_warning(&mut value);
            }
            Response::ok(req.id, value)
        }
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "withdraw-failed"),
    }
}

/// Withdraw every submission currently at `status` in the local history
/// cache. See the module doc for why this always answers
/// [`ERR_ACCOUNT_SESSION_REQUIRED`] today.
///
/// A per-submission failure does not stop the batch: the response's
/// `failed` count is how a caller learns some did not go through, and the
/// contributor can retry with a plain `withdraw` on the ones that matter to
/// them (this method reports counts, not which ids failed, since submission
/// ids are not otherwise exposed over this socket beyond what `list_history`
/// already shows).
pub(super) async fn handle_withdraw_bulk(shared: &DaemonShared, req: &Request) -> Response {
    let status = match req.params.get("status").and_then(|v| v.as_str()) {
        Some(s) if valid_bulk_status(s) => s.to_string(),
        Some(_) => return Response::err(req.id, ERR_BAD_PARAMS, "status-invalid"),
        None => return Response::err(req.id, ERR_BAD_PARAMS, "status-required"),
    };
    let mut session = match account_session(shared) {
        Ok(Some(session)) => session,
        Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED),
        Err(_) => {
            return Response::err(
                req.id,
                ERR_UNAVAILABLE,
                "commons_credential_storage_unavailable",
            );
        }
    };
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    let mut records = match HistoryCache::load(&shared.store) {
        Ok(r) => r,
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, "history-read-failed"),
    };
    let targets: Vec<Uuid> = records
        .iter()
        .filter(|r| r.status == status)
        .map(|r| r.submission_id)
        .collect();

    let mut withdrawn = 0u32;
    let mut failed = 0u32;
    let mut credential_persisted = true;
    let now = Utc::now();
    for id in targets {
        let call = crate::withdraw::call_withdraw(
            &cfg.ingest_url,
            cfg.allowed_hosts.as_deref(),
            &session.session.access_token,
            id,
        )
        .await;
        // Before the next call, which must present the rotated token.
        credential_persisted &= keep_rotated_token(shared, &mut session, call.rotated_token);
        match call.result {
            Ok(_) => {
                if history::mark_withdrawn(&mut records, id, now) {
                    withdrawn += 1;
                }
            }
            Err(_) => failed += 1,
        }
    }
    if withdrawn > 0 {
        let _ = HistoryCache::save(&shared.store, &records);
    }
    let mut value = serde_json::json!({ "withdrawn": withdrawn, "failed": failed });
    if !credential_persisted {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shared() -> DaemonShared {
        let (_d, store) = crate::config::tests_support::temp_store();
        std::mem::forget(_d);
        DaemonShared::load(store).unwrap()
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    #[tokio::test]
    async fn withdraw_without_a_submission_id_is_a_param_error() {
        let s = shared();
        let r = handle_withdraw(&s, &req("withdraw", serde_json::json!({}))).await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn withdraw_with_a_malformed_submission_id_is_a_param_error() {
        let s = shared();
        let r = handle_withdraw(
            &s,
            &req(
                "withdraw",
                serde_json::json!({"submission_id": "not-a-uuid"}),
            ),
        )
        .await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn withdraw_with_a_valid_submission_id_reports_account_session_required() {
        // The daemon holds a device key, not an account session (see this
        // module's doc), so a structurally valid request still cannot be
        // authenticated today -- and must say so distinctly rather than
        // failing generically.
        let s = shared();
        let r = handle_withdraw(
            &s,
            &req(
                "withdraw",
                serde_json::json!({"submission_id": Uuid::new_v4().to_string()}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, ERR_ACCOUNT_SESSION_REQUIRED);
    }

    #[tokio::test]
    async fn withdraw_bulk_without_a_status_is_a_param_error() {
        let s = shared();
        let r = handle_withdraw_bulk(&s, &req("withdraw_bulk", serde_json::json!({}))).await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn withdraw_bulk_rejects_an_unrecognized_status() {
        let s = shared();
        let r = handle_withdraw_bulk(
            &s,
            &req(
                "withdraw_bulk",
                serde_json::json!({"status": "not-a-status"}),
            ),
        )
        .await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn withdraw_bulk_accepts_quarantined_and_reports_account_session_required() {
        let s = shared();
        let r = handle_withdraw_bulk(
            &s,
            &req(
                "withdraw_bulk",
                serde_json::json!({"status": "quarantined"}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, ERR_ACCOUNT_SESSION_REQUIRED);
    }

    #[test]
    fn account_session_token_is_none_until_the_contributor_signs_in() {
        // Fail closed: a store with no account session offers no token, so
        // withdrawal reports `account-session-required` rather than reaching
        // for the device key.
        let s = shared();
        assert!(account_session(&s).unwrap().is_none());
    }

    #[test]
    fn account_session_token_is_offered_once_a_live_session_is_stored() {
        // The other half of the seam: with a live token on disk, withdrawal
        // proceeds to the network call instead of the fail-closed error. If
        // this ever silently stopped working, `withdraw` would look "safe"
        // while being permanently broken.
        let s = shared();
        let session = crate::account_auth::AccountSession {
            access_token: "tcn1_dGVuYW50.secret".to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        assert_eq!(
            account_session(&s)
                .unwrap()
                .map(|loaded| loaded.session.access_token)
                .as_deref(),
            Some("tcn1_dGVuYW50.secret")
        );
    }

    #[tokio::test]
    async fn withdraw_with_an_expired_session_still_reports_account_session_required() {
        // An expired token must not be presented and 401: the contributor is
        // told to sign in again, which is the whole point of the distinct
        // error code.
        let s = shared();
        let session = crate::account_auth::AccountSession {
            access_token: "tcn1_dGVuYW50.secret".to_string(),
            expires_at: Utc::now() - chrono::TimeDelta::hours(1),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        let r = handle_withdraw(
            &s,
            &req(
                "withdraw",
                serde_json::json!({"submission_id": Uuid::new_v4().to_string()}),
            ),
        )
        .await;
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, ERR_ACCOUNT_SESSION_REQUIRED);
    }

    /// A withdrawal route, serving a rotated session on the first call only
    /// and recording the bearer each call presented.
    async fn rotating_ingest(
        status: axum::http::StatusCode,
        rotated: String,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use axum::{Json, Router, response::IntoResponse, routing::post};
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let router = Router::new().route(
            "/v1/account/traces/{submission_id}/withdraw",
            post({
                let seen = seen.clone();
                move |headers: axum::http::HeaderMap| {
                    let seen = seen.clone();
                    let rotated = rotated.clone();
                    async move {
                        let auth = headers["authorization"].to_str().unwrap().to_string();
                        let first = seen.lock().unwrap().is_empty();
                        seen.lock().unwrap().push(auth);
                        let body = if status.is_success() {
                            serde_json::json!({ "distribution_reach": "not_distributed" })
                        } else {
                            serde_json::json!({ "error": "SubmissionNotFound" })
                        };
                        let mut resp = (status, Json(body)).into_response();
                        if first {
                            resp.headers_mut().insert(
                                trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                                rotated.parse().unwrap(),
                            );
                        }
                        resp
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (format!("http://{addr}"), seen)
    }

    /// Signed in against `base`, with `records` quarantined in the history
    /// cache.
    fn signed_in(base: &str, records: &[Uuid]) -> DaemonShared {
        let s = shared();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = base.to_string();
        s.store.save_config(&cfg).unwrap();
        let session = crate::account_auth::AccountSession {
            access_token: "tcn1_dGVuYW50.original".to_string(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
        let records: Vec<history::HistoryRecord> = records
            .iter()
            .map(|id| history::HistoryRecord {
                submission_id: *id,
                submitted_at: Utc::now(),
                project_id: String::new(),
                project_label: "proj".into(),
                source: "claude".into(),
                session_hash: "sha256:test".into(),
                status: STATUS_QUARANTINED.into(),
                consent_scopes: Vec::new(),
                credit_points_pending: 0.0,
                credit_points_final: None,
                explanations: Vec::new(),
                last_refreshed_at: None,
                withdrawn_at: None,
            })
            .collect();
        HistoryCache::save(&s.store, &records).unwrap();
        s
    }

    fn stored_token(s: &DaemonShared) -> Option<String> {
        crate::account_auth::try_load_token(&s.store).unwrap()
    }

    /// The account route rotates an old session and hands the new token back.
    /// Dropping it signs the contributor out once the old one's grace ends.
    #[tokio::test]
    async fn withdraw_keeps_a_rotated_account_session() {
        let rotated = "tcn1_dGVuYW50.rotated".to_string();
        let (base, _) = rotating_ingest(axum::http::StatusCode::OK, rotated.clone()).await;
        let s = signed_in(&base, &[]);
        let r = handle_withdraw(
            &s,
            &req(
                "withdraw",
                serde_json::json!({"submission_id": Uuid::new_v4().to_string()}),
            ),
        )
        .await;
        assert!(r.error.is_none());
        assert_eq!(stored_token(&s), Some(rotated));
    }

    /// The middleware rotates before the handler answers, so a refusal can
    /// carry a rotation too.
    #[tokio::test]
    async fn withdraw_keeps_a_rotated_account_session_on_a_refusal() {
        let rotated = "tcn1_dGVuYW50.rotated".to_string();
        let (base, _) = rotating_ingest(axum::http::StatusCode::NOT_FOUND, rotated.clone()).await;
        let s = signed_in(&base, &[]);
        let r = handle_withdraw(
            &s,
            &req(
                "withdraw",
                serde_json::json!({"submission_id": Uuid::new_v4().to_string()}),
            ),
        )
        .await;
        assert_eq!(r.error.unwrap().message, "withdraw-failed");
        assert_eq!(stored_token(&s), Some(rotated));
    }

    /// In a batch, the calls after a rotation present the rotated token, and
    /// it is the one left stored.
    #[tokio::test]
    async fn withdraw_bulk_presents_and_keeps_a_rotated_account_session() {
        let rotated = "tcn1_dGVuYW50.rotated".to_string();
        let (base, seen) = rotating_ingest(axum::http::StatusCode::OK, rotated.clone()).await;
        let s = signed_in(&base, &[Uuid::new_v4(), Uuid::new_v4()]);
        let r = handle_withdraw_bulk(
            &s,
            &req(
                "withdraw_bulk",
                serde_json::json!({"status": "quarantined"}),
            ),
        )
        .await;
        assert_eq!(r.result.unwrap()["withdrawn"], 2);
        assert_eq!(
            seen.lock().unwrap().as_slice(),
            [
                "Bearer tcn1_dGVuYW50.original".to_string(),
                format!("Bearer {rotated}")
            ]
        );
        assert_eq!(stored_token(&s), Some(rotated));
    }
}
