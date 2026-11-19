//! The daemon's replies to the first run's calls, recorded for the shells'
//! tests (`tests/fixtures/first-run/`, read by the Windows
//! `FirstRunDaemonTests`).
//!
//! Each request goes through the real dispatcher (`handle_request_async`)
//! on a temporary store, with loopback test servers standing in for the
//! issuer. Each whole response frame (`id` plus `result` or `error`) is
//! printed between `BEGIN first-run/<name>` and `END first-run/<name>`
//! under `--nocapture` and compared to the committed file, so a wire change
//! fails here until the file is re-recorded from that output. Values that
//! differ on every run are replaced before the comparison and printing
//! (`steady`); the README names each one.

use std::path::PathBuf;

use axum::{Json, Router, http::StatusCode, routing::post};
use serde_json::{Value, json};

use super::ipc::{DaemonShared, Request, handle_request_async};

fn shared() -> (tempfile::TempDir, DaemonShared) {
    let (dir, store) = crate::config::tests_support::temp_store();
    (dir, DaemonShared::load(store).unwrap())
}

async fn call(s: &DaemonShared, method: &str, params: Value) -> Value {
    let response = handle_request_async(
        s,
        &Request {
            id: 1,
            method: method.to_string(),
            params,
        },
    )
    .await;
    serde_json::to_value(response).unwrap()
}

async fn serve(router: Router) -> (u16, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (port, task)
}

/// An issuer whose `/v1/onboard` enrolls and whose `/v1/invite/lookup`
/// answers `lookup`.
async fn issuer(lookup: Value) -> (u16, tokio::task::JoinHandle<()>) {
    serve(
        Router::new()
            .route(
                "/v1/onboard",
                post(|| async {
                    Json(json!({
                        "schema_version": "trace_commons.onboard_response.v1",
                        "tenant_id": "tenant-mock",
                        "ingest_url": "https://ingest.invalid",
                        "issuer_url": "https://issuer.invalid",
                        "audience": "trace-commons-upload",
                        "device_key_id": "sha256:mockdevice",
                    }))
                }),
            )
            .route(
                "/v1/invite/lookup",
                post(move || {
                    let lookup = lookup.clone();
                    async move { Json(lookup) }
                }),
            ),
    )
    .await
}

/// Replace the values that change on every run with fixed ones, by field
/// name: the sign-in attempt's id, the loopback port and `state` nonce in
/// its page address, the time a grant was given, and the platform's
/// default terminal.
fn steady(value: &mut Value) {
    match value {
        Value::Object(map) => {
            for (key, field) in map.iter_mut() {
                match (key.as_str(), &*field) {
                    ("attempt_id", Value::String(_)) => {
                        *field = json!("00000000-0000-4000-8000-000000000001");
                    }
                    ("browser_url", Value::String(url)) => {
                        let mut url = reqwest::Url::parse(url).unwrap();
                        assert_eq!(url.host_str(), Some("127.0.0.1"), "{url}");
                        url.set_port(Some(49152)).unwrap();
                        let pairs: Vec<(String, String)> = url
                            .query_pairs()
                            .map(|(k, v)| {
                                let v = if k == "state" {
                                    "fixture-state".into()
                                } else {
                                    v.into_owned()
                                };
                                (k.into_owned(), v)
                            })
                            .collect();
                        url.query_pairs_mut().clear().extend_pairs(pairs);
                        *field = json!(url.to_string());
                    }
                    // Each platform has its own default terminal.
                    ("managed_terminal", Value::String(_)) => {
                        *field = json!("terminal");
                    }
                    ("granted_at", Value::String(_)) => {
                        *field = json!("2026-11-19T00:00:00Z");
                    }
                    _ => steady(field),
                }
            }
        }
        Value::Array(items) => items.iter_mut().for_each(steady),
        _ => {}
    }
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/first-run")
        .join(format!("{name}.json"))
}

/// Print one reply between its markers and note it if the committed file
/// differs (or is missing), so one run names every file to re-record.
fn check(name: &str, mut reply: Value, stale: &mut Vec<String>) {
    steady(&mut reply);
    println!("BEGIN first-run/{name}");
    println!("{}", serde_json::to_string_pretty(&reply).unwrap());
    println!("END first-run/{name}");
    let committed = std::fs::read_to_string(fixture_path(name))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok());
    if committed.as_ref() != Some(&reply) {
        stale.push(name.to_string());
    }
}

fn assert_current(stale: &[String]) {
    assert!(
        stale.is_empty(),
        "re-record tests/fixtures/first-run/{{{}}}.json from this test's --nocapture output",
        stale.join(",")
    );
}

/// Enrollment, the scopes, the automatic grant (given, refused for a
/// changed witness, and given by the older from-now method), its
/// withdrawal, the settings declaration and the terms acceptance.
#[tokio::test]
async fn first_run_join_replies_for_shell_fixtures() {
    let (port, task) = issuer(json!({"valid": true})).await;
    let (_dir, s) = shared();
    let mut stale = Vec::new();

    let enroll = call(
        &s,
        "enroll",
        json!({"invite": format!("http://127.0.0.1:{port}/onboard#SAMPLE-CODE")}),
    )
    .await;
    assert_eq!(enroll["result"]["enrolled"], true, "{enroll}");
    check("enroll", enroll, &mut stale);

    let scopes = call(
        &s,
        "set_consent_scopes",
        json!({"scopes": ["debugging_evaluation", "model_training"]}),
    )
    .await;
    assert!(scopes.get("error").is_none(), "{scopes}");
    check("set_consent_scopes", scopes, &mut stale);

    let grant = |witness: Value| {
        json!({
            "confirmed": true,
            "witness_signing_address": witness,
            "include_existing": true,
            "keep_folder_asks": true,
        })
    };
    let given = call(&s, "grant_automatic_including_existing", grant(Value::Null)).await;
    assert_eq!(given["result"]["granted"], true, "{given}");
    assert_eq!(given["result"]["include_existing"], true, "{given}");
    check("grant_automatic_including_existing", given, &mut stale);

    // Shown a witness, but none is configured.
    let changed = call(
        &s,
        "grant_automatic_including_existing",
        grant(json!("0xshown")),
    )
    .await;
    assert_eq!(
        changed["error"]["message"], "automatic-grant-witness-changed",
        "{changed}"
    );
    check(
        "grant_automatic_including_existing_witness_changed",
        changed,
        &mut stale,
    );

    let withdrawn = call(
        &s,
        "withdraw_automatic_grant",
        json!({"return_folders_to_ask": true}),
    )
    .await;
    assert_eq!(withdrawn["result"]["withdrawn"], true, "{withdrawn}");
    check("withdraw_automatic_grant", withdrawn, &mut stale);

    // The older method grants from now on only: a granted answer that does
    // not include the traces already on disk.
    let from_now = call(
        &s,
        "grant_automatic",
        json!({"confirmed": true, "witness_signing_address": null}),
    )
    .await;
    assert_eq!(from_now["result"]["granted"], true, "{from_now}");
    assert_eq!(from_now["result"]["include_existing"], false, "{from_now}");
    check("grant_automatic_from_now", from_now, &mut stale);

    let settings = call(
        &s,
        "set_settings",
        json!({"claude_source": {"mode": "off"}}),
    )
    .await;
    assert!(settings.get("error").is_none(), "{settings}");
    check("set_settings", settings, &mut stale);

    let terms = call(
        &s,
        "terms_accept",
        json!({
            "version": "2026-11-01",
            "content_sha256": "ab".repeat(32),
            "accepted_at": "2026-11-19T09:02:03Z",
        }),
    )
    .await;
    assert!(terms.get("error").is_none(), "{terms}");
    check("terms_accept", terms, &mut stale);

    task.abort();
    assert_current(&stale);
}

/// The invite lookup: found, refused by the issuer, a code that is not an
/// invite, an issuer that does not answer usefully, and a host this
/// enrollment does not allow.
#[tokio::test]
async fn first_run_invite_lookup_replies_for_shell_fixtures() {
    let mut stale = Vec::new();

    let (port, task) = issuer(json!({
        "valid": true,
        "issuer_display_name": "Sample Pilot",
        "credit_range": {"min": 1, "max": 5, "unit": "points_per_accepted_trace"},
    }))
    .await;
    let (_dir, s) = shared();
    let code = json!({"code": format!("http://127.0.0.1:{port}/onboard#SAMPLE-CODE")});
    let found = call(&s, "invite_lookup", code).await;
    assert_eq!(found["result"]["valid"], true, "{found}");
    check("invite_lookup_valid", found, &mut stale);
    task.abort();

    let (port, task) = issuer(json!({"valid": false, "reason_label": "not_found"})).await;
    let code = json!({"code": format!("http://127.0.0.1:{port}/onboard#SAMPLE-CODE")});
    let not_found = call(&s, "invite_lookup", code).await;
    assert_eq!(not_found["result"]["valid"], false, "{not_found}");
    check("invite_lookup_not_found", not_found, &mut stale);
    task.abort();

    let malformed = call(&s, "invite_lookup", json!({"code": "x"})).await;
    assert_eq!(
        malformed["error"]["message"], "invite-invalid",
        "{malformed}"
    );
    check("invite_lookup_malformed", malformed, &mut stale);

    let (port, task) = serve(Router::new().route(
        "/v1/invite/lookup",
        post(|| async { StatusCode::INTERNAL_SERVER_ERROR }),
    ))
    .await;
    let code = json!({"code": format!("http://127.0.0.1:{port}/onboard#SAMPLE-CODE")});
    let unavailable = call(&s, "invite_lookup", code).await;
    assert_eq!(
        unavailable["error"]["message"], "invite-lookup-unavailable",
        "{unavailable}"
    );
    check("invite_lookup_unavailable", unavailable, &mut stale);
    task.abort();

    // An enrollment that allows only its own issuer's host.
    let mut cfg = crate::commands::unenrolled_preview_config();
    cfg.allowed_hosts = Some("issuer.invalid".to_string());
    s.store.save_config(&cfg).unwrap();
    let not_allowed = call(
        &s,
        "invite_lookup",
        json!({"code": "https://other.invalid/onboard#SAMPLE-CODE"}),
    )
    .await;
    assert_eq!(
        not_allowed["error"]["message"], "invite-host-not-allowed",
        "{not_allowed}"
    );
    check("invite_lookup_host_not_allowed", not_allowed, &mut stale);

    assert_current(&stale);
}

/// The browser sign-in: the resting status, a started attempt, its status
/// while it waits, its cancel, and the status once a session is kept. A
/// staging build starts a different login (NEAR AI's Google page), so this
/// records the production build's.
#[cfg(not(feature = "near-ai-staging"))]
#[tokio::test]
async fn first_run_near_ai_sign_in_replies_for_shell_fixtures() {
    let (_dir, s) = shared();
    let mut stale = Vec::new();

    let absent = call(&s, "near_ai_credential_status", json!({})).await;
    assert_eq!(absent["result"]["session_state"], "absent", "{absent}");
    check("near_ai_credential_status_absent", absent, &mut stale);

    let started = call(&s, "near_ai_credential_start", json!({})).await;
    let attempt = started["result"]["attempt_id"]
        .as_str()
        .unwrap_or_else(|| panic!("{started}"))
        .to_string();
    check("near_ai_credential_start", started, &mut stale);

    let waiting = call(
        &s,
        "near_ai_credential_status",
        json!({"attempt_id": attempt}),
    )
    .await;
    assert_eq!(
        waiting["result"]["attempt_status"], "waiting_for_browser",
        "{waiting}"
    );
    check("near_ai_credential_status_waiting", waiting, &mut stale);

    let cancelled = call(
        &s,
        "near_ai_credential_cancel",
        json!({"attempt_id": attempt}),
    )
    .await;
    assert!(cancelled.get("error").is_none(), "{cancelled}");
    check("near_ai_credential_cancel", cancelled, &mut stale);

    // A kept session, as a finished sign-in leaves one, on a machine with
    // no attempt left over. The ceremony that writes it needs NEAR AI's
    // service, so the session is written directly.
    let (_dir, s) = shared();
    s.settings.lock().unwrap().near_ai_session = Some(super::settings::NearAiSession {
        refresh_token: "rt_fixture-not-a-token".into(),
        refresh_token_expires_at: None,
        stored_at: chrono::Utc::now(),
        user_agent: "fixture".into(),
    });
    s.settings.lock().unwrap().save_for_test(&s.store).unwrap();
    let present = call(&s, "near_ai_credential_status", json!({})).await;
    assert_eq!(present["result"]["session_state"], "present", "{present}");
    assert!(!present.to_string().contains("rt_fixture"), "{present}");
    check("near_ai_credential_status_present", present, &mut stale);

    assert_current(&stale);
}
