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

use chrono::Utc;
use serde_json::json;

use super::audit::{self, AuditEntry};
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
/// Deliberately does NOT accept `allowed_hosts` from the caller, unlike the
/// CLI's `--allowed-hosts` flag. `config::allowlist_for` gives a
/// caller-supplied CSV precedence over the `TRACE_COMMONS_ALLOWED_HOSTS` env
/// var, and an empty CSV degrades to permissive -- so a socket caller could
/// otherwise neutralize an operator's env-configured allowlist and have that
/// neutralization persisted into `contributor.json` for every later command.
/// A native application has no legitimate reason to override host
/// enforcement; only always pass `None` here so the env setting governs.
///
/// This performs a real network call (registering the device with the
/// issuer), so it is async; it is reached only through
/// `handle_request_async` and `handle_local`, never through the synchronous
/// `handle_request` -- see the "Sync vs. async dispatch" note on `ipc`.
pub(super) async fn handle_enroll(shared: &DaemonShared, req: &Request) -> Response {
    let grant = req.params.get("grant").and_then(|v| v.as_str());
    let invite = req.params.get("invite").and_then(|v| v.as_str());

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

    match enroll_core(&shared.store, grant, invite, None, consent_scopes).await {
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
///
/// Audited (`consent-scopes-changed`): this can silently widen consent to
/// e.g. `model_training`, at least as consequential as arming auto-upload.
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
    // The audit entry goes down FIRST, before the config is rewritten, the
    // way `acknowledge_near_ai_notice` does it.
    //
    // The reverse order saved the config, appended, and on an append
    // failure restored the previous config best-effort -- but the disk-full
    // or permissions failure that broke the append breaks that restore just
    // as reliably, and the daemon reads its consent scopes from disk. A
    // widened scope set could survive with no record of the widening, which
    // is the exact outcome this entry exists to prevent. Recording first
    // means there is nothing to roll back.
    //
    // Label data, not secret: these are wire-name scope identifiers, the
    // same ones already returned in this very response and in `status`.
    if let Err(_e) = audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: "consent-scopes-changed".to_string(),
            project_label: None,
            detail: Some(scopes.join(",")),
        },
    ) {
        return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
    }
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
///
/// Audited (`near-ai-notice-acknowledged`): this asserts, on the socket
/// caller's unverified word, that a third-party-scan disclosure was shown to
/// someone. A notice nobody actually saw is not a notice, so defeating this
/// gate is at least as consequential as arming auto-upload, and less
/// recoverable: traces may already have gone out under the false
/// acknowledgment before anyone notices.
pub(super) fn handle_acknowledge_near_ai_notice(shared: &DaemonShared, req: &Request) -> Response {
    // The audit entry goes down FIRST, before the marker exists. There is
    // no "un-acknowledge" operation to roll back to, so ordering is what
    // makes this fail-closed: if the record cannot be persisted, the gate
    // is never cleared and traces cannot go out under an acknowledgment
    // nobody can see. The reverse order would leave the gate cleared with
    // no record of who cleared it, which is precisely the failure this
    // entry exists to prevent.
    if let Err(_e) = audit::append(
        &shared.store,
        &AuditEntry {
            at: Utc::now(),
            action: "near-ai-notice-acknowledged".to_string(),
            project_label: None,
            detail: None,
        },
    ) {
        return Response::err(req.id, ERR_UNAVAILABLE, "audit-write-failed");
    }
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

    /// Start a real `/v1/onboard` responder on an ephemeral `127.0.0.1` port
    /// and return its base URL.
    async fn spawn_onboard_mock() -> String {
        use axum::{Json, Router, routing::post};
        let router = Router::new().route(
            "/v1/onboard",
            post(|| async move {
                Json(serde_json::json!({
                    "schema_version": "trace_commons.onboard_response.v1",
                    "tenant_id": "tenant-mock",
                    "ingest_url": "https://ingest.invalid",
                    "issuer_url": "https://issuer.invalid",
                    "audience": "trace-commons-upload",
                    "device_key_id": "sha256:mockdevice",
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn enroll_ignores_a_caller_supplied_allowed_hosts_and_the_env_allowlist_still_governs() {
        // Regression for a real vulnerability: `config::allowlist_for` gives
        // a caller-supplied CSV precedence over TRACE_COMMONS_ALLOWED_HOSTS,
        // and an empty CSV degrades to permissive. Before the fix,
        // `handle_enroll` forwarded a socket-supplied `allowed_hosts`
        // straight into `enroll_core`, so a socket caller could pass a CSV
        // that excludes the real target host and have the request refused
        // by an allowlist mismatch it invented, or (worse) pass an empty
        // string and have that persisted into `contributor.json`. Since the
        // fix drops the parameter entirely, a mock issuer running on a real
        // `127.0.0.1` port must be reachable regardless of what a caller
        // puts in `allowed_hosts` -- the request must not be pre-refused by
        // a host list the caller invented.
        let base = spawn_onboard_mock().await;
        let s = shared();
        let r = handle_enroll(
            &s,
            &req(
                "enroll",
                json!({
                    "invite": format!("{base}/onboard#SOME-CODE"),
                    // An attacker-controlled value that, if honored, would
                    // make the allowlist either permissive (empty string) or
                    // would exclude the real host -- neither must have any
                    // effect now that the parameter does not exist on the
                    // wire contract.
                    "allowed_hosts": "",
                }),
            ),
        )
        .await;
        assert!(r.error.is_none(), "{:?}", r.error);
        assert_eq!(r.result.unwrap()["enrolled"], true);
        // The caller's "allowed_hosts" value must never reach the saved
        // config either: enroll_core is always called with `None`, and the
        // invite path itself never persists `allowed_hosts` (only the grant
        // path's `login --allowed-hosts` flag does).
        let cfg = s.store.load_config().unwrap().unwrap();
        assert_eq!(cfg.allowed_hosts, None);
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

    fn enrolled_shared() -> DaemonShared {
        let s = shared();
        s.store
            .save_config(&crate::config::ContributorConfig {
                inference_receipt_endpoint: None,
                inference_receipt_check_attestation: false,
                schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
                issuer_url: "https://issuer.invalid".to_string(),
                ingest_url: "https://ingest.invalid".to_string(),
                audience: "aud".to_string(),
                tenant_id: "tenant-1".to_string(),
                instance_id: "instance-1".to_string(),
                user_subject: "alice".to_string(),
                device_key_id: "sha256:aa".to_string(),
                consent_scopes: vec!["debugging_evaluation".to_string()],
                pii_filter: None,
                allowed_hosts: None,
                display_handle: None,
                public_bio: None,
                public_since: None,
                witness: None,
            })
            .unwrap();
        s
    }

    #[test]
    fn set_consent_scopes_appends_an_audit_entry() {
        // A socket caller widening its own consent (e.g. to model_training)
        // is at least as consequential as arming auto-upload, and gets the
        // same visibility.
        let s = enrolled_shared();
        let r = handle_set_consent_scopes(
            &s,
            &req("set_consent_scopes", json!({"scopes": ["model_training"]})),
        );
        assert!(r.error.is_none(), "{:?}", r.error);
        let entries = audit::load(&s.store).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "consent-scopes-changed");
        assert!(
            entries[0]
                .detail
                .as_deref()
                .unwrap()
                .contains("model_training")
        );
    }

    #[test]
    fn acknowledging_the_near_ai_notice_appends_an_audit_entry() {
        // The caller asserts, on its own unverified word, that a
        // third-party disclosure was shown to someone -- exactly the kind
        // of consequential-and-otherwise-invisible action this log exists
        // for.
        let s = shared();
        let r =
            handle_acknowledge_near_ai_notice(&s, &req("acknowledge_near_ai_notice", json!({})));
        assert!(r.error.is_none(), "{:?}", r.error);
        let entries = audit::load(&s.store).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].action, "near-ai-notice-acknowledged");
    }

    /// Make the audit log unappendable: `audit::load` reads it as UTF-8 and
    /// fails on bytes that are not, so every subsequent `append` fails.
    fn break_the_audit_log(store: &crate::config::ConfigStore) {
        store
            .write_daemon_file(crate::config::DAEMON_AUDIT_FILE, &[0xff, 0xfe, 0xff])
            .unwrap();
    }

    #[test]
    fn widening_consent_is_rolled_back_when_its_audit_entry_cannot_be_written() {
        // A consent widening that leaves no record of itself is the exact
        // outcome the entry exists to prevent, so it is fail-closed like
        // the other audited socket actions rather than best-effort.
        let s = enrolled_shared();
        break_the_audit_log(&s.store);
        let r = handle_set_consent_scopes(
            &s,
            &req("set_consent_scopes", json!({"scopes": ["model_training"]})),
        );
        let err = r.error.expect("an unwritable audit log must fail the call");
        assert_eq!(err.message, "audit-write-failed");
        assert_eq!(
            s.store.load_config().unwrap().unwrap().consent_scopes,
            vec!["debugging_evaluation".to_string()],
            "the widening must not stand without a record of it"
        );
    }

    #[test]
    fn the_near_ai_notice_gate_stays_closed_when_its_audit_entry_cannot_be_written() {
        // There is no un-acknowledge operation to roll back to, so the
        // record goes down before the marker: a failure leaves the gate
        // closed rather than cleared-and-unrecorded.
        let s = shared();
        break_the_audit_log(&s.store);
        let r =
            handle_acknowledge_near_ai_notice(&s, &req("acknowledge_near_ai_notice", json!({})));
        let err = r.error.expect("an unwritable audit log must fail the call");
        assert_eq!(err.message, "audit-write-failed");
        assert!(
            !s.store.near_ai_notice_shown(),
            "the gate must not be cleared without a record of who cleared it"
        );
    }
}
