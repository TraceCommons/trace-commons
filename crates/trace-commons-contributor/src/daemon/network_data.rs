//! Content-free network read surfaces. Prices are never relabelled as billed spend.
//! Proof detail reads stored labels only; invite lookup never enrolls or redeems.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use trace_commons_protocol::invite_lookup::{
    CREDIT_RANGE_UNIT_POINTS, InviteLookupResponse, valid_credit_range, valid_issuer_display_name,
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::routing::RoutingLedger;

pub(crate) async fn handle_summary(shared: &DaemonShared, req: &Request) -> Response {
    let (since, window_hours) = match req.params.get("since") {
        None | Some(serde_json::Value::Null) => {
            (Utc::now() - chrono::Duration::hours(24), Some(24))
        }
        Some(value) => match value
            .as_str()
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        {
            Some(at) if at <= Utc::now() => (at.with_timezone(&Utc), None),
            _ => return Response::err(req.id, ERR_BAD_PARAMS, "summary-since-invalid"),
        },
    };
    let ledger = shared.routing_ledger();
    let mut summary = match ledger.as_ref() {
        Some(ledger) => ledger.read_summary(since).await,
        None => None,
    };
    if let Some(summary) = summary.as_mut() {
        normalize_summary(summary);
    }
    Response::ok(
        req.id,
        serde_json::json!({
            "readable": summary.is_some(), "window_hours": window_hours,
            "observed_at": ledger.and_then(|ledger| ledger.last_summary_at()),
            "summary": summary,
        }),
    )
}

fn normalize_summary(summary: &mut crate::routing::ironwire::SummaryView) {
    for group in &mut summary.groups {
        // JSON array boundaries distinguish tuples even when display labels
        // sanitize to the same value. Only the digest crosses the socket.
        let key =
            serde_json::to_vec(&(&group.model, &group.backend, &group.route, &group.work_kind))
                .expect("summary tuple serializes");
        group.group_id = format!("sha256:{:x}", Sha256::digest(key));
        // Preserve future work classifications in opaque identity only.
        group.work_kind = None;
        group.model = group
            .model
            .as_deref()
            .map(|model| super::inference_map::model_label(Some(model)));
        if group.backend != "nearai" {
            // An opaque identifier, not anonymization: this unsalted digest
            // can still be matched against guessed backend labels.
            group.backend = format!("sha256:{:x}", Sha256::digest(group.backend.as_bytes()));
        }
    }
}

pub(crate) fn handle_proof(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req
        .params
        .get("call_id")
        .and_then(serde_json::Value::as_i64)
        .filter(|id| *id > 0)
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "call-id-invalid");
    };
    let ledger = shared.routing_ledger();
    let readable = ledger
        .as_ref()
        .and_then(|ledger| ledger.last_refresh_at())
        .is_some();
    let row = ledger.filter(|_| readable).and_then(|ledger| {
        ledger
            .exchanges_since(Utc::now() - chrono::Duration::hours(24))
            .into_iter()
            .find(|row| row.id == Some(id))
    });
    Response::ok(
        req.id,
        serde_json::json!({
            "call_id": id, "readable": readable, "found": row.is_some(),
            "proof": row.as_ref().map_or("unrecorded", super::inference_map::proof_label),
            "checked_at": null, "checks": null,
        }),
    )
}

pub(crate) async fn handle_model_spend(shared: &DaemonShared, req: &Request) -> Response {
    Response::ok(
        req.id,
        super::nearai_credential::balance::read_model_spend(shared).await,
    )
}

pub(crate) fn handle_private_ai(shared: &DaemonShared, req: &Request) -> Response {
    let (on, lifecycle) = match super::settings::DaemonSettings::load(&shared.store) {
        Ok(_) => (
            Some(
                shared
                    .settings
                    .lock()
                    .expect("settings lock")
                    .private_inference,
            ),
            shared.private_inference_value(),
        ),
        Err(_) => (None, serde_json::json!({"state":null,"port":null})),
    };
    Response::ok(
        req.id,
        serde_json::json!({
            "on": on, "state": lifecycle["state"], "port": lifecycle["port"],
            "disclosure": crate::private_inference_copy::OFFER_EXPOSURE,
            "view": {"what": crate::private_inference_copy::OFFER_WHAT,
                "no_repoint": crate::private_inference_copy::OFFER_NO_REPOINT},
        }),
    )
}

/// Only an explicit, validated answer reaches the existing hosting lifecycle.
pub(crate) async fn handle_set_private_ai(shared: &DaemonShared, req: &Request) -> Response {
    let Some(on) = req.params.get("on").and_then(serde_json::Value::as_bool) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "private-ai-on-invalid");
    };
    if on
        && req
            .params
            .get("confirmed")
            .and_then(serde_json::Value::as_bool)
            != Some(true)
    {
        return Response::err(req.id, ERR_BAD_PARAMS, "confirmation-required");
    }
    let mut params = serde_json::json!({"private_inference": on});
    if on {
        // The enable confirmation above is the only offer acknowledgement.
        params["private_inference_offer_seen"] = serde_json::json!(true);
    }
    let settings = Request {
        id: req.id,
        method: "set_settings".to_string(),
        params,
    };
    let result = super::ipc::handle_set_settings_async(shared, &settings).await;
    if result.error.is_some() {
        result
    } else {
        handle_private_ai(shared, req)
    }
}

pub(crate) async fn handle_invite_lookup(shared: &DaemonShared, req: &Request) -> Response {
    let Some(invite) = req
        .params
        .get("code")
        .and_then(serde_json::Value::as_str)
        .filter(|s| s.len() <= 4096)
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "invite-invalid");
    };
    // The same check `invite_issuer_host` makes, so a host a shell showed
    // is one this lookup asks about.
    let Some((parsed, url)) = crate::commands::acceptable_invite(invite) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "invite-invalid");
    };
    let configured = match shared.store.load_config() {
        Ok(cfg) => cfg.and_then(|cfg| cfg.allowed_hosts),
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, "invite-lookup-unavailable"),
    };
    let allowlist = crate::config::allowlist_for(configured.as_deref());
    if allowlist.check(&url).is_err() {
        return Response::err(req.id, ERR_UNAVAILABLE, "invite-host-not-allowed");
    }
    let Ok(client) = crate::issuer_client::IssuerClient::new(allowlist) else {
        return Response::err(req.id, ERR_UNAVAILABLE, "invite-lookup-unavailable");
    };
    let Ok(answer) = client.lookup_invite(&parsed.issuer_url, &parsed.code).await else {
        return Response::err(req.id, ERR_UNAVAILABLE, "invite-lookup-unavailable");
    };
    if !valid_invite_answer(&answer) {
        return Response::err(req.id, ERR_UNAVAILABLE, "invite-lookup-unavailable");
    }
    Response::ok(
        req.id,
        serde_json::to_value(answer).expect("invite answer serializes"),
    )
}

fn valid_invite_answer(answer: &InviteLookupResponse) -> bool {
    if answer.valid {
        answer.reason_label.is_none()
            && answer
                .issuer_display_name
                .as_deref()
                .is_none_or(valid_issuer_display_name)
            && answer.credit_range.as_ref().is_none_or(|range| {
                valid_credit_range(range.min, range.max) && range.unit == CREDIT_RANGE_UNIT_POINTS
            })
    } else {
        answer.issuer_display_name.is_none()
            && answer.credit_range.is_none()
            && matches!(
                answer.reason_label.as_deref(),
                Some("malformed" | "not_found" | "expired" | "exhausted" | "revoked")
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json, Router,
        routing::{get, post},
    };
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    fn shared() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        (dir, DaemonShared::load(store).unwrap())
    }

    fn request(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 1,
            method: method.to_string(),
            params,
        }
    }

    async fn serve(router: Router) -> (u16, tokio::task::JoinHandle<()>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let task = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        (port, task)
    }

    fn sample() -> serde_json::Value {
        serde_json::from_str(include_str!(
            "../../tests/fixtures/ironwire/sample-summary.json"
        ))
        .unwrap()
    }

    #[test]
    fn summary_group_identity_survives_colliding_sanitized_model_labels() {
        let mut body = sample();
        let mut second = body["groups"][0].clone();
        body["groups"][0]["model"] = serde_json::json!("https://secret.example/one");
        second["model"] = serde_json::json!("https://secret.example/two");
        body["groups"].as_array_mut().unwrap().push(second);
        let mut summary: crate::routing::ironwire::SummaryView =
            serde_json::from_value(body).unwrap();
        normalize_summary(&mut summary);
        let last = summary.groups.last().unwrap();
        assert_eq!(summary.groups[0].model, last.model);
        assert_ne!(summary.groups[0].group_id, last.group_id);
        assert_eq!(summary.groups[0].total.calls, 3);
        assert_eq!(last.total.calls, 3);
        assert!(
            !serde_json::to_string(&summary)
                .unwrap()
                .contains("secret.example")
        );
        let same: crate::routing::ironwire::SummaryView = serde_json::from_value(sample()).unwrap();
        assert_eq!(same.groups.len() + 1, summary.groups.len());
    }

    #[test]
    fn summary_work_kind_is_opaque_but_preserves_group_identity() {
        let mut body = sample();
        let mut second = body["groups"][0].clone();
        body["groups"][0]["work_kind"] = serde_json::json!("PRIVATE-WORK-ONE");
        second["work_kind"] = serde_json::json!("PRIVATE-WORK-TWO");
        body["groups"].as_array_mut().unwrap().push(second);
        let mut summary: crate::routing::ironwire::SummaryView =
            serde_json::from_value(body).unwrap();
        normalize_summary(&mut summary);
        assert!(summary.groups.iter().all(|group| group.work_kind.is_none()));
        assert_ne!(
            summary.groups[0].group_id,
            summary.groups.last().unwrap().group_id
        );
        assert!(
            !serde_json::to_string(&summary)
                .unwrap()
                .contains("PRIVATE-WORK")
        );
    }

    const SINCE: &str = "2026-10-01T00:00:00+02:00";

    #[tokio::test]
    async fn summary_reads_authenticated_window_and_preserves_partial_pricing_and_labels() {
        let mut body = sample();
        body["since"] = serde_json::json!(SINCE);
        body["groups"][0]["model"] = serde_json::json!("https://evil.example/PROMPT-SECRET");
        body["groups"][0]["backend"] = serde_json::json!("ACCOUNT-SECRET");
        body["groups"][0]["work_kind"] = serde_json::json!("FUTURE-WORK-SECRET");
        let router = Router::new().route(
            "/_ironwire/summary",
            get(
                move |headers: axum::http::HeaderMap,
                      axum::extract::Query(query): axum::extract::Query<
                    std::collections::HashMap<String, String>,
                >| {
                    let body = body.clone();
                    async move {
                        assert_eq!(headers["authorization"], "Bearer CONTROL-SECRET");
                        assert_eq!(
                            DateTime::parse_from_rfc3339(&query["since"]).unwrap(),
                            DateTime::parse_from_rfc3339(SINCE).unwrap()
                        );
                        Json(body)
                    }
                },
            ),
        );
        let (port, task) = serve(router).await;
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
            port,
            "CONTROL-SECRET".to_string(),
        ));
        let result = super::super::ipc::handle_request_async(
            &s,
            &request("inference_summary", serde_json::json!({"since":SINCE})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(result["readable"], true);
        assert!(result["window_hours"].is_null());
        assert!(result["observed_at"].is_string());
        assert_eq!(result["summary"]["groups"][0]["model"], "unknown");
        assert!(result["summary"]["groups"][0]["work_kind"].is_null());
        assert_eq!(
            result["summary"]["groups"][0]["backend"],
            format!("sha256:{:x}", Sha256::digest(b"ACCOUNT-SECRET"))
        );
        assert_eq!(result["summary"]["groups"][0]["calls"], 3);
        assert_eq!(result["summary"]["groups"][0]["priced_calls"], 2);
        assert_eq!(result["summary"]["routed"]["proof"]["gateway_only"], 1);
        assert!(!result.to_string().contains("SECRET"));
        task.abort();
        let stale = handle_summary(
            &s,
            &request("inference_summary", serde_json::json!({"since":SINCE})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(stale["readable"], false);
        assert!(stale["summary"].is_null());
        assert_eq!(stale["observed_at"], result["observed_at"]);
    }

    #[tokio::test]
    async fn summary_capture_disabled_is_distinct_from_unreadable() {
        let mut body = sample();
        body["since"] = serde_json::json!(SINCE);
        body["enabled"] = serde_json::json!(false);
        body["groups"] = serde_json::json!([]);
        body["routed"] = body["outside"].clone();
        let router = Router::new().route(
            "/_ironwire/summary",
            get(move || {
                let body = body.clone();
                async move { Json(body) }
            }),
        );
        let (port, task) = serve(router).await;
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
            port,
            String::new(),
        ));
        let value = handle_summary(
            &s,
            &request("inference_summary", serde_json::json!({"since":SINCE})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(value["readable"], true);
        assert_eq!(value["summary"]["enabled"], false);
        task.abort();
    }

    #[tokio::test]
    async fn summary_refuses_status_malformed_oversized_and_redirect_without_following() {
        use axum::http::{HeaderMap, HeaderValue, StatusCode};
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (target, target_task) = serve(Router::new().route(
            "/leak",
            get(move || {
                let calls = observed.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    "leaked"
                }
            }),
        ))
        .await;
        for (status, body, redirect) in [
            (StatusCode::UNAUTHORIZED, "SECRET".to_string(), false),
            (StatusCode::NOT_FOUND, "SECRET".to_string(), false),
            (StatusCode::OK, "{not-json-SECRET".to_string(), false),
            (StatusCode::OK, "x".repeat(1024 * 1024 + 1), false),
            (StatusCode::TEMPORARY_REDIRECT, String::new(), true),
        ] {
            let router = Router::new().route(
                "/_ironwire/summary",
                get(move || {
                    let body = body.clone();
                    async move {
                        let mut headers = HeaderMap::new();
                        if redirect {
                            headers.insert(
                                "location",
                                HeaderValue::from_str(&format!("http://127.0.0.1:{target}/leak"))
                                    .unwrap(),
                            );
                        }
                        (status, headers, body)
                    }
                }),
            );
            let (port, task) = serve(router).await;
            let (_dir, s) = shared();
            s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
                port,
                "CONTROL-SECRET".to_string(),
            ));
            let value = handle_summary(
                &s,
                &request("inference_summary", serde_json::json!({"since":SINCE})),
            )
            .await
            .result
            .unwrap();
            assert_eq!(value["readable"], false);
            assert!(value["summary"].is_null());
            assert!(!value.to_string().contains("SECRET"));
            task.abort();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        target_task.abort();
    }

    #[tokio::test]
    async fn summary_rejects_inconsistent_counts_costs_and_unknown_routes() {
        for (field, value) in [
            ("calls", serde_json::json!(-1)),
            ("priced_calls", serde_json::json!(4)),
            ("cost_usd", serde_json::json!(-0.1)),
            ("cost_usd", serde_json::json!(1e100)),
            ("route", serde_json::json!("SECRET")),
        ] {
            let mut body = sample();
            body["since"] = serde_json::json!(SINCE);
            body["groups"][0][field] = value;
            let router = Router::new().route(
                "/_ironwire/summary",
                get(move || {
                    let body = body.clone();
                    async move { Json(body) }
                }),
            );
            let (port, task) = serve(router).await;
            let (_dir, s) = shared();
            s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
                port,
                String::new(),
            ));
            let value = handle_summary(
                &s,
                &request("inference_summary", serde_json::json!({"since":SINCE})),
            )
            .await
            .result
            .unwrap();
            assert_eq!(value["readable"], false, "{field}");
            task.abort();
        }
    }

    #[tokio::test]
    async fn private_ai_requires_confirmed_enable_and_off_changes_only_hosting_settings() {
        let (_dir, s) = shared();
        let before = serde_json::to_value(s.settings.lock().unwrap().clone()).unwrap();
        for params in [
            serde_json::json!({"on":true}),
            serde_json::json!({"on":true,"confirmed":false}),
        ] {
            let response =
                super::super::ipc::handle_request_async(&s, &request("set_private_ai", params))
                    .await;
            assert_eq!(response.error.unwrap().message, "confirmation-required");
            assert_eq!(
                serde_json::to_value(s.settings.lock().unwrap().clone()).unwrap(),
                before
            );
        }
        let value = super::super::ipc::handle_request_async(
            &s,
            &request("set_private_ai", serde_json::json!({"on":false})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(value["on"], false);
        assert_eq!(value["state"], "off");
        let after = serde_json::to_value(s.settings.lock().unwrap().clone()).unwrap();
        assert_eq!(after["private_inference_offer_seen"], false);
        assert!(
            !super::super::settings::DaemonSettings::load(&s.store)
                .unwrap()
                .private_inference_offer_seen
        );
        assert_eq!(after, before);
        assert!(s.store.load_config().unwrap().is_none());
        assert_eq!(
            value["disclosure"],
            crate::private_inference_copy::OFFER_EXPOSURE
        );
    }

    #[tokio::test]
    async fn invite_lookup_rejects_plaintext_hostname_without_sending_code() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (port, task) = serve(Router::new().route(
            "/v1/invite/lookup",
            post(move || {
                let calls = observed.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({"valid":true}))
                }
            }),
        ))
        .await;
        let (_dir, s) = shared();
        assert!(s.store.load_config().unwrap().is_none());
        let response = handle_invite_lookup(
            &s,
            &request(
                "invite_lookup",
                serde_json::json!({
                    "code":format!("http://localhost:{port}/onboard#SECRET-CODE")
                }),
            ),
        )
        .await;
        task.abort();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(response.error.unwrap().message, "invite-invalid");
    }

    #[tokio::test]
    async fn invite_lookup_rejects_remote_plaintext_before_loading_config() {
        let (_dir, s) = shared();
        std::fs::write(s.store.dir().join("contributor.json"), "malformed").unwrap();
        for host in ["issuer.example", "192.0.2.1", "[2001:db8::1]"] {
            let response = handle_invite_lookup(
                &s,
                &request(
                    "invite_lookup",
                    serde_json::json!({
                        "code":format!("http://{host}/onboard#SECRET-CODE")
                    }),
                ),
            )
            .await;
            assert_eq!(response.error.unwrap().message, "invite-invalid", "{host}");
        }
    }

    #[tokio::test]
    async fn invite_lookup_accepts_https_and_literal_loopback_transport_before_config() {
        let (_dir, s) = shared();
        std::fs::write(s.store.dir().join("contributor.json"), "malformed").unwrap();
        for origin in ["https://issuer.example", "http://127.0.0.1", "http://[::1]"] {
            let response = handle_invite_lookup(
                &s,
                &request(
                    "invite_lookup",
                    serde_json::json!({
                        "code":format!("{origin}/onboard#SECRET-CODE")
                    }),
                ),
            )
            .await;
            assert_eq!(
                response.error.unwrap().message,
                "invite-lookup-unavailable",
                "{origin}"
            );
        }
    }

    #[tokio::test]
    async fn invite_lookup_uses_post_only_and_leaves_identity_settings_and_config_untouched() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let router = Router::new().route("/v1/invite/lookup", post(move |uri: axum::http::Uri, Json(body): Json<serde_json::Value>| {
            let calls = observed.clone(); async move {
                calls.fetch_add(1, Ordering::SeqCst); assert!(uri.query().is_none());
                assert_eq!(body["invite_code"], "SAMPLE-CODE");
                Json(serde_json::json!({"valid":true,"issuer_display_name":"Sample Pilot","credit_range":{"min":1,"max":5,"unit":"points_per_accepted_trace"}}))
            }
        }));
        let (port, task) = serve(router).await;
        let (dir, s) = shared();
        let before = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect::<std::collections::BTreeSet<_>>();
        let value = super::super::ipc::handle_request_async(
            &s,
            &request(
                "invite_lookup",
                serde_json::json!({"code":format!("http://127.0.0.1:{port}/onboard#SAMPLE-CODE")}),
            ),
        )
        .await
        .result
        .unwrap();
        assert_eq!(value["valid"], true);
        assert_eq!(value["credit_range"]["unit"], "points_per_accepted_trace");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            before,
            std::fs::read_dir(dir.path())
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect()
        );
        assert!(s.store.load_config().unwrap().is_none());
        assert!(!value.to_string().contains("SAMPLE-CODE"));
        task.abort();
    }

    #[tokio::test]
    async fn malformed_invites_are_refused_without_transport_and_hostile_metadata_is_discarded() {
        let (_dir, s) = shared();
        for invite in [
            "SAMPLE-CODE",
            "file:///etc/passwd#CODE",
            "https://example.com/onboard",
            "https://user:SECRET@example.com/onboard#CODE",
        ] {
            let response = handle_invite_lookup(
                &s,
                &request("invite_lookup", serde_json::json!({"code":invite})),
            )
            .await;
            assert_eq!(response.error.unwrap().message, "invite-invalid");
        }
        for answer in [
            serde_json::json!({"valid":true,"issuer_display_name":"SECRET\nname"}),
            serde_json::json!({"valid":true,"credit_range":{"min":-1,"max":5,"unit":"points_per_accepted_trace"}}),
            serde_json::json!({"valid":true,"credit_range":{"min":1,"max":5,"unit":"USD"}}),
            serde_json::json!({"valid":false,"reason_label":"SECRET"}),
        ] {
            let router = Router::new().route(
                "/v1/invite/lookup",
                post(move || {
                    let answer = answer.clone();
                    async move { Json(answer) }
                }),
            );
            let (port, task) = serve(router).await;
            let response = handle_invite_lookup(
                &s,
                &request(
                    "invite_lookup",
                    serde_json::json!({"code":format!("http://127.0.0.1:{port}/onboard#CODE")}),
                ),
            )
            .await;
            assert_eq!(response.error.unwrap().message, "invite-lookup-unavailable");
            task.abort();
        }
    }
    #[tokio::test]
    async fn summary_timeout_and_bad_since_never_become_empty_measured_data() {
        let router = Router::new().route(
            "/_ironwire/summary",
            get(|| async {
                tokio::time::sleep(std::time::Duration::from_secs(4)).await;
                Json(sample())
            }),
        );
        let (port, task) = serve(router).await;
        let (_dir, s) = shared();
        s.install_routing_ledger_for_test(crate::routing::ironwire::IronWireLedger::new(
            port,
            String::new(),
        ));
        let start = std::time::Instant::now();
        let value = handle_summary(&s, &request("inference_summary", serde_json::json!({})))
            .await
            .result
            .unwrap();
        assert!(start.elapsed() < std::time::Duration::from_secs(4));
        assert_eq!(value["readable"], false);
        assert!(value["summary"].is_null());
        for since in [
            serde_json::json!("SECRET"),
            serde_json::json!(123),
            serde_json::json!("9999-01-01T00:00:00Z"),
        ] {
            let response = handle_summary(
                &s,
                &request("inference_summary", serde_json::json!({"since":since})),
            )
            .await;
            assert_eq!(response.error.unwrap().message, "summary-since-invalid");
        }
        task.abort();
    }

    #[tokio::test]
    async fn confirmed_enable_uses_owned_lifecycle_without_granting_body_consent() {
        let (_dir, s) = shared();
        let home = tempfile::tempdir().unwrap();
        s.adopt_runtime();
        s.install_private_inference_for_test(home.path().to_path_buf(), 0);
        let before = s.settings.lock().unwrap().clone();
        let response = super::super::ipc::handle_request_async(
            &s,
            &request(
                "set_private_ai",
                serde_json::json!({"on":true,"confirmed":true}),
            ),
        )
        .await;
        let value = response.result.unwrap();
        assert_eq!(value["on"], true);
        assert_ne!(value["state"], "off");
        assert_eq!(value["state"], s.private_inference_value()["state"]);
        let after = s.settings.lock().unwrap().clone();
        assert_eq!(after.ironwire, before.ironwire);
        assert_eq!(
            after.ironwire_attested_bodies,
            before.ironwire_attested_bodies
        );
        assert_eq!(after.token_capture_enabled, before.token_capture_enabled);
        assert!(after.private_inference_offer_seen);
        assert!(s.store.load_config().unwrap().is_none());
        let result = super::super::ipc::handle_request_async(
            &s,
            &request("set_private_ai", serde_json::json!({"on":false})),
        )
        .await
        .result
        .unwrap();
        assert_eq!(result["on"], false);
        assert!(s.settings.lock().unwrap().private_inference_offer_seen);
        assert!(
            super::super::settings::DaemonSettings::load(&s.store)
                .unwrap()
                .private_inference_offer_seen
        );
        assert!(matches!(result["state"].as_str(), Some("off" | "stopping")));
        s.stop_private_inference().await;
    }

    #[test]
    fn unreadable_settings_remain_unknown_and_do_not_mutate_on_read() {
        let (_dir, s) = shared();
        std::fs::write(
            s.store.dir().join("daemon-settings.json"),
            "SECRET malformed",
        )
        .unwrap();
        let response = handle_private_ai(&s, &request("private_ai", serde_json::json!({})))
            .result
            .unwrap();
        assert!(response["on"].is_null());
        assert!(response["state"].is_null());
        assert!(response["port"].is_null());
    }

    #[tokio::test]
    async fn invite_transport_errors_and_redirects_remain_fixed_unavailable_labels() {
        use axum::http::{HeaderMap, HeaderValue, StatusCode};
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let (target, target_task) = serve(Router::new().route(
            "/leak",
            post(move || {
                let calls = observed.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    "leak"
                }
            }),
        ))
        .await;
        let (_dir, s) = shared();
        for status in [
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::NOT_FOUND,
            StatusCode::TEMPORARY_REDIRECT,
        ] {
            let router = Router::new().route(
                "/v1/invite/lookup",
                post(move || async move {
                    let mut headers = HeaderMap::new();
                    headers.insert(
                        "location",
                        HeaderValue::from_str(&format!("http://127.0.0.1:{target}/leak")).unwrap(),
                    );
                    (status, headers, "SECRET response body")
                }),
            );
            let (port, task) = serve(router).await;
            let response = handle_invite_lookup(&s, &request("invite_lookup", serde_json::json!({"code":format!("http://127.0.0.1:{port}/onboard#SECRET-CODE")}))).await;
            assert_eq!(response.error.unwrap().message, "invite-lookup-unavailable");
            task.abort();
        }
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        target_task.abort();
    }

    #[tokio::test]
    async fn invite_host_override_cannot_bypass_the_configured_allowlist() {
        let calls = Arc::new(AtomicUsize::new(0));
        let observed = calls.clone();
        let router = Router::new().route(
            "/v1/invite/lookup",
            post(move || {
                let calls = observed.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    Json(serde_json::json!({"valid":true}))
                }
            }),
        );
        let (port, task) = serve(router).await;
        let (_dir, s) = shared();
        let cfg = serde_json::from_value(serde_json::json!({
            "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url":"https://issuer.example", "ingest_url":"https://ingest.example",
            "audience":"trace-commons-upload", "tenant_id":"tenant", "instance_id":"instance",
            "user_subject":"user", "device_key_id":"sha256:00", "consent_scopes":[],
            "allowed_hosts":"trusted.example"
        }))
        .unwrap();
        s.store.save_config(&cfg).unwrap();
        let before = std::fs::read(s.store.dir().join("contributor.json")).unwrap();
        let response = handle_invite_lookup(&s, &request("invite_lookup", serde_json::json!({
            "code":format!("http://127.0.0.1:{port}/onboard#CODE"), "allowed_hosts":"127.0.0.1"
        }))).await;
        assert_eq!(response.error.unwrap().message, "invite-host-not-allowed");
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        assert_eq!(
            before,
            std::fs::read(s.store.dir().join("contributor.json")).unwrap()
        );
        task.abort();
    }
    #[tokio::test]
    async fn invite_oversized_success_or_refusal_body_is_unavailable() {
        use axum::http::StatusCode;
        let (_dir, s) = shared();
        for status in [StatusCode::OK, StatusCode::TOO_MANY_REQUESTS] {
            let router = Router::new().route(
                "/v1/invite/lookup",
                post(move || async move {
                    (
                        status,
                        serde_json::json!({"valid":true,"padding":"x".repeat(64 * 1024 + 1)})
                            .to_string(),
                    )
                }),
            );
            let (port, task) = serve(router).await;
            let response = handle_invite_lookup(
                &s,
                &request(
                    "invite_lookup",
                    serde_json::json!({"code":format!("http://127.0.0.1:{port}/onboard#CODE")}),
                ),
            )
            .await;
            assert_eq!(response.error.unwrap().message, "invite-lookup-unavailable");
            task.abort();
        }
    }
}
