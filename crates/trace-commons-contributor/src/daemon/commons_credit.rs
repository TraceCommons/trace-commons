//! IPC handler for `commons_credit_summary` (K7).
//!
//! Relays two server reads the daemon never made before this, each on the
//! credential that can make it:
//!
//! - the deployment's settlement posture, from the DEVICE route
//!   `GET /v1/contributors/me/settlement-posture` (#1119), so "Settlement
//!   off" renders for a device-only contributor too;
//! - this contributor's points on the commons' ledger, from the ACCOUNT
//!   route `GET /v1/account/credit-summary`, only when an account session
//!   is stored.
//!
//! See `crate::commons_credit` for the HTTP clients and the account-session
//! rationale; this module is the IPC-side half: request shape, the
//! account-session credential handling `daemon::withdraw` already
//! established, a short cache, and the always-succeeds contract.
//!
//! # A short cache
//!
//! A shell may call this every time a screen opens, and the server allows
//! the account route 30 calls per window. The whole answer is cached per
//! daemon for [`KNOWN_TTL`] when both halves were read, and for the shorter
//! [`UNKNOWN_TTL`] otherwise, so a server that is down is retried soon but
//! not hammered. An answer carrying a `credential_warning` is never cached:
//! the next call should try to store the rotated session again.
//!
//! # This is a relay, not a mutation: it always succeeds
//!
//! Unlike `withdraw`, nothing here changes state on the server, so there is
//! no reason to make a caller handle a distinct `account-session-required`
//! error. Like `near_ai_balance`, each half answers `"known"` or
//! `"unknown"` (`posture_state`, `points_state`) rather than an IPC error --
//! a missing session, a transport failure, and a non-200 response all
//! collapse to `"unknown"` for the half they affect. Every numeric and
//! string field of an unknown half is present and `null`,
//! matching `near_ai_balance`'s own rule: an absent key would be ambiguous
//! between "this build is too old to know" and "this daemon knows it does
//! not know", and a client must never render `null` as zero.
//!
//! # Naming: these are the COMMONS' figures, not near.ai's
//!
//! #1118's decision makes the commons record the source of pending credit;
//! dollars stay on near.ai as a link-out. Every figure here carries a
//! `commons_` prefix so a client preserves that source and never presents
//! points as a near.ai dollar balance. See `crate::commons_credit`'s module doc.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::ipc::{DaemonShared, Request, Response};
use super::withdraw::{account_session, keep_rotated_token};
use crate::commons_credit::{CommonsCreditSummary, CommonsSettlementPosture};

/// How long an answer with both halves known is served from the cache.
const KNOWN_TTL: Duration = Duration::from_secs(60);
/// How long an answer with either half unknown is served from the cache: a
/// short backoff, so a signed-out or unreachable daemon does not call out on
/// every screen open, but a fresh sign-in shows up within seconds.
const UNKNOWN_TTL: Duration = Duration::from_secs(15);

type Cache = Mutex<HashMap<PathBuf, (Instant, Duration, serde_json::Value)>>;

/// Keyed by the daemon's state directory, so two daemons in one process
/// (the test suite) never share an answer.
fn cache() -> &'static Cache {
    static CACHE: OnceLock<Cache> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The commons' credit summary, as a client should read it: fixed keys,
/// `null` for a half whose state is not `known`, and a source-labelled name
/// on every figure. See this module's doc for the rules.
pub(super) async fn handle_commons_credit_summary(
    shared: &DaemonShared,
    req: &Request,
) -> Response {
    let key = shared.store.dir().to_path_buf();
    if let Some(value) = cached(&key) {
        return Response::ok(req.id, value);
    }
    let observed_at = chrono::Utc::now();
    let Ok(Some(cfg)) = shared.store.load_config() else {
        let value = summary_value(None, None, observed_at);
        remember(key, UNKNOWN_TTL, &value);
        return Response::ok(req.id, value);
    };
    let posture = crate::commons_credit::fetch_settlement_posture(&shared.store, &cfg).await;
    let mut credential_persisted = true;
    let points = match account_session(shared) {
        Ok(Some(mut session)) => {
            let call = crate::commons_credit::call_credit_summary(
                &cfg.ingest_url,
                cfg.allowed_hosts.as_deref(),
                &session.session.access_token,
            )
            .await;
            // Stored whatever the result, exactly like `withdraw` and
            // `account_admission`: the server rotates a native session on a
            // refusal too, and dropping this signs the contributor out at
            // the next grace expiry.
            credential_persisted = keep_rotated_token(shared, &mut session, call.rotated_token);
            call.result
        }
        // No session, or the credential store could not be read: a fact
        // about the machine, not a caller error -- see the module doc.
        _ => None,
    };
    let both_known = posture.is_some() && points.is_some();
    let mut value = summary_value(posture, points, observed_at);
    if credential_persisted {
        remember(
            key,
            if both_known { KNOWN_TTL } else { UNKNOWN_TTL },
            &value,
        );
    } else {
        super::public_run::attach_credential_warning(&mut value);
    }
    Response::ok(req.id, value)
}

fn cached(key: &PathBuf) -> Option<serde_json::Value> {
    let cache = cache().lock().ok()?;
    let (at, ttl, value) = cache.get(key)?;
    (at.elapsed() < *ttl).then(|| value.clone())
}

fn remember(key: PathBuf, ttl: Duration, value: &serde_json::Value) {
    if let Ok(mut cache) = cache().lock() {
        cache.insert(key, (Instant::now(), ttl, value.clone()));
    }
}

fn summary_value(
    posture: Option<CommonsSettlementPosture>,
    points: Option<CommonsCreditSummary>,
    observed_at: chrono::DateTime<chrono::Utc>,
) -> serde_json::Value {
    let state = |known: bool| if known { "known" } else { "unknown" };
    serde_json::json!({
        "posture_state": state(posture.is_some()),
        "commons_settlement": posture.as_ref().map(|p| p.commons_settlement.clone()),
        "commons_settlement_explanation":
            posture.as_ref().map(|p| p.commons_settlement_explanation.clone()),
        "commons_graded": posture.as_ref().map(|p| p.commons_graded),
        "points_state": state(points.is_some()),
        "commons_points_earned_this_period":
            points.as_ref().map(|p| p.commons_points_earned_this_period),
        "commons_points_lifetime_earned": points.as_ref().map(|p| p.commons_points_lifetime_earned),
        "commons_pending_review": points.as_ref().map(|p| p.commons_pending_review),
        "commons_currency_code": points.as_ref().and_then(|p| p.commons_currency_code.clone()),
        "commons_currency_earned_this_period":
            points.as_ref().and_then(|p| p.commons_currency_earned_this_period.clone()),
        "commons_period_start": points.as_ref().and_then(|p| p.commons_period_start),
        "commons_period_end": points.as_ref().and_then(|p| p.commons_period_end),
        "observed_at": observed_at,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commons_credit::tests::stub_posture_route;
    use crate::daemon::credential_store::{CredentialError, CredentialReference, SecretBackend};
    use axum::{
        Json, Router,
        http::{HeaderMap, StatusCode},
        response::IntoResponse,
        routing::get,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    const ALL_KEYS: &[&str] = &[
        "posture_state",
        "commons_settlement",
        "commons_settlement_explanation",
        "commons_graded",
        "points_state",
        "commons_points_earned_this_period",
        "commons_points_lifetime_earned",
        "commons_pending_review",
        "commons_currency_code",
        "commons_currency_earned_this_period",
        "commons_period_start",
        "commons_period_end",
        "observed_at",
    ];

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

    fn assert_every_key_present(v: &serde_json::Value) {
        for key in ALL_KEYS {
            assert!(v.get(*key).is_some(), "missing {key}: {v}");
        }
    }

    /// No enrollment at all: unknown, not an error, and every figure is a
    /// `null` rather than an absent key.
    #[tokio::test]
    async fn with_no_config_both_halves_are_unknown_with_every_key_present() {
        let s = shared();
        let r = handle_commons_credit_summary(&s, &req()).await;
        assert!(r.error.is_none(), "this method always succeeds");
        let v = r.result.unwrap();
        assert_every_key_present(&v);
        assert_eq!(v["posture_state"], "unknown");
        assert_eq!(v["points_state"], "unknown");
        assert!(v["commons_settlement"].is_null());
        assert!(v["commons_points_earned_this_period"].is_null());
        assert!(v["commons_pending_review"].is_null());
    }

    /// A route that answers the account credit-summary call, optionally
    /// rotating the session, and counts how often it was asked.
    fn credit_summary_route(
        router: Router,
        rotated: Option<&'static str>,
        calls: Arc<AtomicUsize>,
        on_call: Option<Arc<AtomicBool>>,
    ) -> Router {
        router.route(
            "/v1/account/credit-summary",
            get(move || {
                let calls = calls.clone();
                let on_call = on_call.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    if let Some(flag) = on_call {
                        flag.store(true, Ordering::SeqCst);
                    }
                    let mut headers = HeaderMap::new();
                    if let Some(rotated) = rotated {
                        headers.insert(
                            trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                            rotated.parse().unwrap(),
                        );
                    }
                    (
                        StatusCode::OK,
                        headers,
                        Json(serde_json::json!({
                            "points": {"earned_this_period": 7, "lifetime_earned": 30},
                            "currency": {"code": "USD", "earned_this_period": "0.70"},
                            "posture": {
                                "settlement": "disabled",
                                "graded": false,
                                "explanation": "Credit is recorded but not settled.",
                            },
                            "period": {
                                "start": "2026-08-01T00:00:00Z",
                                "end": "2026-08-31T00:00:00Z",
                            },
                            "pending_review": 1,
                        })),
                    )
                        .into_response()
                }
            }),
        )
    }

    async fn serve(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn enrolled_against(s: &DaemonShared, base: &str) {
        let device = crate::identity::DeviceIdentity::load_or_generate(&s.store).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.issuer_url = base.to_string();
        cfg.ingest_url = base.to_string();
        cfg.device_key_id = device.device_key_id;
        s.store.save_config(&cfg).unwrap();
    }

    fn sign_in(s: &DaemonShared) {
        let session = crate::account_auth::AccountSession {
            access_token: "tcn1_dGVuYW50.original".to_string(),
            expires_at: chrono::Utc::now() + chrono::TimeDelta::hours(6),
            account_id: "acct".to_string(),
        };
        s.store
            .write_daemon_file(
                crate::config::ACCOUNT_SESSION_FILE,
                &serde_json::to_vec(&session).unwrap(),
            )
            .unwrap();
    }

    /// The review's case: a device-only contributor (no account session)
    /// still reads the posture, from the device route. Points stay unknown.
    #[tokio::test]
    async fn a_device_only_contributor_still_reads_the_settlement_posture() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = serve(credit_summary_route(
            stub_posture_route(StatusCode::OK, "disabled", Default::default()),
            None,
            calls.clone(),
            None,
        ))
        .await;
        let s = shared();
        enrolled_against(&s, &base);

        let v = handle_commons_credit_summary(&s, &req())
            .await
            .result
            .unwrap();
        assert_every_key_present(&v);
        assert_eq!(v["posture_state"], "known");
        assert_eq!(v["commons_settlement"], "disabled");
        assert_eq!(v["points_state"], "unknown");
        assert!(v["commons_points_lifetime_earned"].is_null());
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "no account session, no account call"
        );
    }

    /// Handler-level rotation: the account route rotates the session, the
    /// handler stores it, and the points come through. The currency block is
    /// withheld because this posture is not graded.
    #[tokio::test]
    async fn the_handler_keeps_a_rotated_account_session_and_relays_points() {
        let rotated = "tcn1_dGVuYW50.rotated";
        let calls = Arc::new(AtomicUsize::new(0));
        let base = serve(credit_summary_route(
            stub_posture_route(StatusCode::OK, "disabled", Default::default()),
            Some(rotated),
            calls.clone(),
            None,
        ))
        .await;
        let s = shared();
        enrolled_against(&s, &base);
        sign_in(&s);

        let v = handle_commons_credit_summary(&s, &req())
            .await
            .result
            .unwrap();
        assert_eq!(v["points_state"], "known");
        assert_eq!(v["commons_points_lifetime_earned"], 30);
        assert_eq!(v["commons_period_start"], "2026-08-01T00:00:00Z");
        assert!(
            v["commons_currency_code"].is_null(),
            "ungraded: no dollar figure"
        );
        assert!(v.get("credential_warning").is_none());
        assert_eq!(
            crate::account_auth::try_load_token(&s.store)
                .unwrap()
                .as_deref(),
            Some(rotated)
        );
    }

    /// A second call inside the cache window is served locally: the server
    /// allows the account route 30 calls per window, and a shell may ask on
    /// every screen open.
    #[tokio::test]
    async fn a_repeat_call_inside_the_window_is_served_from_the_cache() {
        let calls = Arc::new(AtomicUsize::new(0));
        let base = serve(credit_summary_route(
            stub_posture_route(StatusCode::OK, "disabled", Default::default()),
            None,
            calls.clone(),
            None,
        ))
        .await;
        let s = shared();
        enrolled_against(&s, &base);
        sign_in(&s);

        let first = handle_commons_credit_summary(&s, &req())
            .await
            .result
            .unwrap();
        let second = handle_commons_credit_summary(&s, &req())
            .await
            .result
            .unwrap();
        assert_eq!(first, second);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[derive(Default)]
    struct ReadbackFaultBackend {
        values: Mutex<HashMap<String, Vec<u8>>>,
        fail_reads: Arc<AtomicBool>,
    }

    impl SecretBackend for ReadbackFaultBackend {
        fn read(&self, reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
            if self.fail_reads.load(Ordering::SeqCst) {
                return Err(CredentialError::Unavailable);
            }
            self.values
                .lock()
                .unwrap()
                .get(&reference.storage_key()?)
                .cloned()
                .ok_or(CredentialError::NoEntry)
        }

        fn write(
            &self,
            reference: &CredentialReference,
            bytes: &[u8],
        ) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .insert(reference.storage_key()?, bytes.to_vec());
            Ok(())
        }

        fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
            self.values
                .lock()
                .unwrap()
                .remove(&reference.storage_key()?);
            Ok(())
        }
    }

    /// A rotation the daemon could not store is reported, as every other
    /// account-route caller reports it, rather than dropped silently -- and
    /// that answer is not cached, so the next call tries again.
    #[tokio::test]
    async fn a_rotation_that_cannot_be_saved_attaches_a_credential_warning() {
        let backend = Arc::new(ReadbackFaultBackend::default());
        let fail_reads = backend.fail_reads.clone();
        let calls = Arc::new(AtomicUsize::new(0));
        let base = serve(credit_summary_route(
            stub_posture_route(StatusCode::OK, "disabled", Default::default()),
            Some("tcn1_dGVuYW50.rotated"),
            calls.clone(),
            Some(fail_reads),
        ))
        .await;
        let (_d, store) = crate::config::tests_support::temp_store();
        std::mem::forget(_d);
        let backend_for_store: Arc<dyn SecretBackend> = backend.clone();
        crate::daemon::commons_credentials::install_test_backend(&store, backend_for_store);
        let s = DaemonShared::load(store).unwrap();
        enrolled_against(&s, &base);
        sign_in(&s);

        let v = handle_commons_credit_summary(&s, &req())
            .await
            .result
            .unwrap();
        assert_eq!(
            v["credential_warning"],
            "commons_credential_storage_unavailable"
        );

        backend.fail_reads.store(false, Ordering::SeqCst);
        let _ = handle_commons_credit_summary(&s, &req()).await;
        assert_eq!(
            calls.load(Ordering::SeqCst),
            2,
            "a warned answer is not cached"
        );
    }
}
