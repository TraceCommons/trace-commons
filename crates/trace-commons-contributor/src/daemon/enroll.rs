//! Enrollment, consent, and the NEAR AI first-use notice over the daemon
//! socket.
//!
//! `enroll` and `set_consent_scopes` are the socket-facing entry points onto
//! `commands::enroll_core`, the same non-interactive enrollment
//! implementation the CLI's `login` command drives after resolving its own
//! interactive consent prompt. Neither shell hand-rolls a second copy of the
//! network calls: a socket caller and a terminal caller enrol identically.
//!
//! Nothing here ever puts an invite link, a grant, a URL, or key material
//! into a response or an error: only scope names, counts, and the
//! already-public `tenant_id` / `device_key_id` identifiers (the same ones
//! `whoami` prints).

use serde_json::json;

use super::health::LABEL_NEAR_AI_NOTICE_PENDING;
use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::commands::{EnrollOutcome, enroll_core};
use crate::consent::{VALID_SCOPES, validate_scopes};

/// `(scope name, human description, grants_data_use)`, in the order
/// `consent_options` walks `VALID_SCOPES`. `public_attribution` maps to an
/// empty allowed-use set in `consent::scopes_to_allowed_uses`, hence
/// `grants_data_use: false` -- presenting it beside four real data-use scopes
/// with equal weight would mislead in both directions.
const DESCRIPTIONS: [(&str, &str, bool); 5] = [
    (
        "debugging_evaluation",
        "Researchers read traces to find where coding agents fail, and score agents against each other.",
        true,
    ),
    (
        "benchmark_only",
        "Parts of your sessions may become benchmark problems that agents are scored against.",
        true,
    ),
    (
        "ranking_training",
        "Used to train models that rank or grade what an agent produced. Not models that write code.",
        true,
    ),
    (
        "model_training",
        "Your traces become training data for models that write code, potentially including commercial ones.",
        true,
    ),
    (
        "public_attribution",
        "Lists your handle publicly as a contributor. Does not change how any trace is used.",
        false,
    ),
];

/// The consent scope list with human descriptions, sourced from
/// [`VALID_SCOPES`] so three shells (CLI, tray, window) cannot each hardcode
/// a copy that drifts from the protocol.
pub fn consent_options() -> serde_json::Value {
    let scopes: Vec<serde_json::Value> = VALID_SCOPES
        .iter()
        .map(|name| {
            let (_, description, grants_data_use) = DESCRIPTIONS
                .iter()
                .find(|(n, _, _)| n == name)
                .expect("every VALID_SCOPES entry has a DESCRIPTIONS row");
            json!({
                "name": name,
                "description": description,
                // VALID_SCOPES[0] is documented as the always-on floor scope.
                "always_on": *name == VALID_SCOPES[0],
                "grants_data_use": grants_data_use,
            })
        })
        .collect();
    json!({ "scopes": scopes })
}

/// Parse a `scopes` params array into wire-name strings. Absent is not an
/// error (it means "floor scope only"); present-but-malformed is.
fn parse_scope_names(params: &serde_json::Value) -> Result<Vec<String>, &'static str> {
    match params.get("scopes") {
        None => Ok(Vec::new()),
        Some(v) => {
            let arr = v.as_array().ok_or("scopes-invalid")?;
            arr.iter()
                .map(|item| item.as_str().map(str::to_string).ok_or("scopes-invalid"))
                .collect()
        }
    }
}

/// Enroll this device: an invite link or an instance-signed grant, plus the
/// chosen consent scopes, exactly as `login` does for a terminal caller.
///
/// This performs a real network call (registering the device with the
/// issuer), so it is async; it is reached only through
/// `handle_request_async` and `handle_local`, never through the synchronous
/// `handle_request` -- see the "Sync vs. async dispatch" note on `ipc`.
pub(super) async fn handle_enroll(shared: &DaemonShared, req: &Request) -> Response {
    let grant = req.params.get("grant").and_then(|v| v.as_str());
    let invite = req.params.get("invite").and_then(|v| v.as_str());
    let allowed_hosts = req.params.get("allowed_hosts").and_then(|v| v.as_str());

    if grant.is_some() && invite.is_some() {
        return Response::err(
            req.id,
            ERR_BAD_PARAMS,
            "grant-and-invite-mutually-exclusive",
        );
    }

    let scope_names = match parse_scope_names(&req.params) {
        Ok(names) => names,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let consent_scopes = match validate_scopes(&scope_names) {
        Ok(s) => s,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "scopes-invalid"),
    };

    match enroll_core(&shared.store, grant, invite, allowed_hosts, consent_scopes).await {
        Ok(EnrollOutcome::AwaitingGrant { device_key_id }) => Response::ok(
            req.id,
            json!({ "enrolled": false, "device_key_id": device_key_id }),
        ),
        Ok(EnrollOutcome::Enrolled(cfg)) => Response::ok(
            req.id,
            json!({
                "enrolled": true,
                "tenant_id": cfg.tenant_id,
                "device_key_id": cfg.device_key_id,
                "consent_scopes": cfg.consent_scopes,
            }),
        ),
        // Never echo the underlying error: it can carry an issuer response
        // body or a URL.
        Err(_e) => Response::err(req.id, ERR_UNAVAILABLE, "enroll-failed"),
    }
}

/// Change consent scopes after enrollment, as the consent prompt at login
/// already promises. Purely a local config write -- no network call -- so
/// this stays on the synchronous `handle_request` path.
pub(super) fn handle_set_consent_scopes(shared: &DaemonShared, req: &Request) -> Response {
    let scope_names = match parse_scope_names(&req.params) {
        Ok(names) => names,
        Err(m) => return Response::err(req.id, ERR_BAD_PARAMS, m),
    };
    let scopes = match validate_scopes(&scope_names) {
        Ok(s) => s,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "scopes-invalid"),
    };
    let Ok(Some(mut cfg)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, "not-logged-in");
    };
    cfg.consent_scopes = scopes.clone();
    if shared.store.save_config(&cfg).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, "config-write-failed");
    }
    Response::ok(req.id, json!({ "consent_scopes": scopes }))
}

/// Record that the NEAR AI first-use disclosure was shown in a UI, and clear
/// the health label blocking on it. This is the only way an app-only
/// contributor (never touching the CLI, which shows the same notice on
/// stdout) can become unstuck.
pub(super) fn handle_acknowledge_near_ai_notice(shared: &DaemonShared, req: &Request) -> Response {
    match shared.store.ensure_near_ai_notice_shown() {
        Ok(_created) => {
            shared
                .health
                .lock()
                .expect("health lock")
                .resolve(LABEL_NEAR_AI_NOTICE_PENDING);
            Response::ok(req.id, json!({ "acknowledged": true }))
        }
        Err(_e) => Response::err(req.id, ERR_UNAVAILABLE, "notice-write-failed"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn consent_options_lists_every_valid_scope_with_a_description() {
        let v = consent_options();
        let scopes = v["scopes"].as_array().unwrap();
        assert_eq!(scopes.len(), VALID_SCOPES.len());
        for s in scopes {
            assert!(!s["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn consent_options_marks_the_floor_scope_as_always_on() {
        let v = consent_options();
        let floor = v["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "debugging_evaluation")
            .unwrap();
        assert_eq!(floor["always_on"], true);
    }

    #[test]
    fn consent_options_marks_public_attribution_as_granting_no_data_use() {
        // It maps to an empty allowed-use set, so presenting it beside four
        // real data-use scopes with equal weight misleads in both directions.
        let v = consent_options();
        let pa = v["scopes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "public_attribution")
            .unwrap();
        assert_eq!(pa["grants_data_use"], false);
    }

    #[test]
    fn consent_options_never_marks_a_non_floor_scope_always_on() {
        let v = consent_options();
        for s in v["scopes"].as_array().unwrap() {
            if s["name"] != "debugging_evaluation" {
                assert_eq!(s["always_on"], false, "{s:?}");
            }
        }
    }

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
    async fn enroll_with_both_grant_and_invite_is_a_param_error() {
        let s = shared();
        let r = handle_enroll(
            &s,
            &req(
                "enroll",
                json!({"grant": "g", "invite": "https://issuer.example/onboard#CODE"}),
            ),
        )
        .await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[tokio::test]
    async fn enroll_with_neither_grant_nor_invite_reports_the_device_key_id() {
        let s = shared();
        let r = handle_enroll(&s, &req("enroll", json!({}))).await;
        assert!(r.error.is_none(), "{:?}", r.error);
        let v = r.result.unwrap();
        assert_eq!(v["enrolled"], false);
        assert!(v["device_key_id"].as_str().is_some());
        assert!(
            s.store.load_config().unwrap().is_none(),
            "no grant or invite means nothing was enrolled"
        );
    }

    #[tokio::test]
    async fn enroll_rejects_an_unknown_scope_name() {
        let s = shared();
        let r = handle_enroll(&s, &req("enroll", json!({"scopes": ["not-a-real-scope"]}))).await;
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn set_consent_scopes_refuses_when_not_enrolled() {
        let s = shared();
        let r = handle_set_consent_scopes(
            &s,
            &req("set_consent_scopes", json!({"scopes": ["model_training"]})),
        );
        assert_eq!(r.error.unwrap().code, ERR_UNAVAILABLE);
    }

    #[test]
    fn set_consent_scopes_rejects_an_unknown_scope_name() {
        let s = shared();
        let r = handle_set_consent_scopes(
            &s,
            &req(
                "set_consent_scopes",
                json!({"scopes": ["not-a-real-scope"]}),
            ),
        );
        assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
    }

    #[test]
    fn acknowledging_the_near_ai_notice_clears_the_blocking_health_label() {
        let s = shared();
        s.health
            .lock()
            .unwrap()
            .fail(LABEL_NEAR_AI_NOTICE_PENDING, chrono::Utc::now());
        let r =
            handle_acknowledge_near_ai_notice(&s, &req("acknowledge_near_ai_notice", json!({})));
        assert!(r.error.is_none(), "{:?}", r.error);
        assert!(s.store.near_ai_notice_shown());
        assert!(s.health.lock().unwrap().ok());
    }
}
