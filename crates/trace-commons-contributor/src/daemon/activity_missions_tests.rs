fn activity_empty_catalogue() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,"kind":"trace_activity","state":"unconfigured",
        "policy_sha256":null,"policy":null,"rewards_enabled":false,
        "credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"
    })
}
fn activity_status() -> serde_json::Value {
    serde_json::json!({
        "schema_version":1,"kind":"trace_activity","policy_sha256":"a".repeat(64),
        "observed_at":"2026-10-02T00:00:00Z","source":"account_contributed_submissions",
        "qualification":"accepted","coverage_starts_on":"2026-10-01","month_starts_on":"2026-10-01",
        "monthly_contributions":2,"daily":null,"completed_days":null,"current_streak":null,
        "level":null,"levels_configured":false,"badges":null,"rewards_enabled":false,
        "credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"
    })
}
fn activity_sign_in(s: &DaemonShared) {
    let session = crate::account_auth::AccountSession {
        access_token: "tcn1_dGVuYW50.original".into(),
        expires_at: Utc::now() + chrono::Duration::hours(6),
        account_id: "account".into(),
    };
    s.store
        .write_daemon_file(
            crate::config::ACCOUNT_SESSION_FILE,
            &serde_json::to_vec(&session).unwrap(),
        )
        .unwrap();
}

#[tokio::test]
async fn activity_missions_reject_private_inputs_before_http_and_require_account_for_status() {
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let app = Router::new().fallback(move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            StatusCode::OK
        }
    });
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    for method in ["activity_missions_catalogue", "activity_missions_status"] {
        assert!(METHODS.contains(&method));
        for params in [
            serde_json::json!({"profile":{}}),
            serde_json::json!({"matched":true}),
            serde_json::json!({"session_id":"private"}),
            serde_json::json!({"account_id":"other"}),
            serde_json::json!({"ingest_url":base}),
            serde_json::json!({"limit":20}),
            serde_json::Value::Null,
        ] {
            let r = handle_request_async(&s, &req(method, params)).await;
            assert!(r.result.is_none());
            assert_eq!(r.error.unwrap().code, ERR_BAD_PARAMS);
        }
    }
    let r = handle_request_async(&s, &req("activity_missions_status", serde_json::json!({}))).await;
    assert!(r.result.is_none());
    assert_eq!(r.error.unwrap().message, "account-session-required");
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
}

#[tokio::test]
async fn activity_missions_public_catalogue_is_anonymous_and_preserves_null_rewards_without_mutation()
 {
    let app = Router::new().route(
        "/v1/activity-missions",
        get(
            |headers: HeaderMap, Query(query): Query<BTreeMap<String, String>>| async move {
                assert!(query.is_empty());
                for name in [
                    "authorization",
                    "cookie",
                    "x-api-key",
                    "x-tenant-id",
                    "x-profile",
                ] {
                    assert!(!headers.contains_key(name));
                }
                Json(activity_empty_catalogue())
            },
        ),
    );
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    activity_sign_in(&s);
    let before = persisted_files(s.store.dir());
    let r = handle_request_async(
        &s,
        &req("activity_missions_catalogue", serde_json::json!({})),
    )
    .await;
    assert!(r.error.is_none());
    let result = r.result.unwrap();
    assert_eq!(result["catalogue"], activity_empty_catalogue());
    assert_eq!(
        result["disclosure"],
        "Matching stays on this Mac; no activity profile or match result is sent. Missions change no capture or contribution permissions and send no traces. Progress uses contributions made through your existing consent. Mission rewards are disabled, and no mission credit is available. Any future mission credit would remain pending and conditional until settlement."
    );
    assert_eq!(persisted_files(s.store.dir()), before);
    server.abort();
}

#[tokio::test]
async fn activity_missions_status_is_account_authenticated_and_preserves_rotations_on_success_and_failure()
 {
    for status in [StatusCode::OK, StatusCode::SERVICE_UNAVAILABLE] {
        let app=Router::new().route("/v1/account/activity-missions/status",get(move |headers:HeaderMap,Query(query):Query<BTreeMap<String,String>>|async move {
            assert!(query.is_empty());assert_eq!(headers["authorization"],"Bearer tcn1_dGVuYW50.original");
            assert!(!headers.contains_key("cookie"));assert!(!headers.contains_key("x-tenant-id"));
            (status,[(trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,"tcn1_dGVuYW50.rotated")],Json(activity_status()))
        }));
        let (base, server) = mission_server(app).await;
        let s = shared();
        configure_catalogue(&s, &base);
        activity_sign_in(&s);
        let policy = s.policy.lock().unwrap().clone();
        let state = s.state.lock().unwrap().clone();
        let r =
            handle_request_async(&s, &req("activity_missions_status", serde_json::json!({}))).await;
        assert_eq!(
            crate::account_auth::try_load_token(&s.store)
                .unwrap()
                .as_deref(),
            Some("tcn1_dGVuYW50.rotated")
        );
        if status == StatusCode::OK {
            assert!(r.error.is_none());
            assert_eq!(r.result.unwrap()["status"], activity_status());
        } else {
            assert!(r.result.is_none());
            assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
        }
        assert_eq!(*s.policy.lock().unwrap(), policy);
        assert_eq!(*s.state.lock().unwrap(), state);
        server.abort();
    }
}

#[tokio::test]
async fn activity_missions_refuse_malformed_reward_activation_and_untrusted_or_redirected_sources()
{
    for (method, path, body) in [
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["rewards_enabled"] = true.into();
            v
        }),
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["state"] = "configured".into();
            v
        }),
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["schema_version"] = 2.into();
            v
        }),
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["policy_sha256"] = "0".repeat(64).into();
            v
        }),
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["credit_points_pending"] = 100.into();
            v
        }),
        ("activity_missions_catalogue", "/v1/activity-missions", {
            let mut v = activity_empty_catalogue();
            v["credit_condition"] = "active".into();
            v
        }),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            {
                let mut v = activity_status();
                v["schema_version"] = 2.into();
                v
            },
        ),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            {
                let mut v = activity_status();
                v["rewards_enabled"] = true.into();
                v
            },
        ),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            {
                let mut v = activity_status();
                v["credit_points_pending"] = 100.into();
                v
            },
        ),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            {
                let mut v = activity_status();
                v["kind"] = "skill_evaluation".into();
                v
            },
        ),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            {
                let mut v = activity_status();
                v["month_starts_on"] = "2026-10-02".into();
                v
            },
        ),
    ] {
        let app = Router::new().route(
            path,
            get(move || {
                let body = body.clone();
                async move { Json(body) }
            }),
        );
        let (base, server) = mission_server(app).await;
        let s = shared();
        configure_catalogue(&s, &base);
        activity_sign_in(&s);
        let r = handle_request_async(&s, &req(method, serde_json::json!({}))).await;
        assert!(r.result.is_none());
        assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
        server.abort();
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let count = calls.clone();
    let target = Router::new().fallback(move || {
        let count = count.clone();
        async move {
            count.fetch_add(1, Ordering::SeqCst);
            Json(activity_empty_catalogue())
        }
    });
    let (target_base, target_server) = mission_server(target).await;
    let app = Router::new().route(
        "/v1/activity-missions",
        get(move || {
            let target_base = target_base.clone();
            async move { (StatusCode::FOUND, [("location", target_base)]) }
        }),
    );
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let r = handle_request_async(
        &s,
        &req("activity_missions_catalogue", serde_json::json!({})),
    )
    .await;
    assert!(r.result.is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    server.abort();
    target_server.abort();
    configure_catalogue(&s, "https://unapproved.example");
    let r = handle_request_async(
        &s,
        &req("activity_missions_catalogue", serde_json::json!({})),
    )
    .await;
    assert!(r.result.is_none());
    assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
}

#[tokio::test]
async fn activity_missions_configured_catalogue_and_daily_progress_are_preserved() {
    let catalogue = serde_json::json!({
        "schema_version":1,"kind":"trace_activity","state":"configured",
        "policy_sha256":"1f6ea4a79dea33fcfbf6d068d1536074d04b35af7a27a6353ff362f695afdf73",
        "policy":{"schema_version":1,"policy_id":"weekly","starts_on":"2026-10-01","ends_before":"2026-11-01","qualification":"accepted","missions":[{"id":"one","title":"One contribution","required_contributions":1}],"daily":null,"levels":null,"badges":null},
        "rewards_enabled":false,"credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"
    });
    let mut status = activity_status();
    status["daily"] = serde_json::json!({"day":"2026-10-02","mission_id":"one","contributions":2,"required_contributions":1,"complete":true});
    status["completed_days"] = 1.into();
    status["current_streak"] = 1.into();
    status["levels_configured"] = true.into();
    status["level"] = "starter".into();
    status["badges"] = serde_json::json!([{"id":"daily","achieved":true}]);
    for (method, path, mut body, key) in [
        (
            "activity_missions_catalogue",
            "/v1/activity-missions",
            catalogue,
            "catalogue",
        ),
        (
            "activity_missions_status",
            "/v1/account/activity-missions/status",
            status,
            "status",
        ),
    ] {
        let expected = body.clone();
        body["display_hint"] = serde_json::json!({"future": "RESPONSE-EXTENSION-SECRET"});
        if key == "status" {
            body["daily"]["display_hint"] = "DAILY-EXTENSION-SECRET".into();
            body["badges"][0]["display_hint"] = "BADGE-EXTENSION-SECRET".into();
        }
        let app = Router::new().route(
            path,
            get(move || {
                let body = body.clone();
                async move { Json(body) }
            }),
        );
        let (base, server) = mission_server(app).await;
        let s = shared();
        configure_catalogue(&s, &base);
        activity_sign_in(&s);
        let r = handle_request_async(&s, &req(method, serde_json::json!({}))).await;
        assert!(r.error.is_none());
        assert_eq!(r.result.unwrap()[key], expected);
        server.abort();
    }
}

#[tokio::test]
async fn activity_missions_declared_and_chunked_response_bounds_refuse_valid_oversized_json() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let body =
        serde_json::to_string(&activity_empty_catalogue()).unwrap() + &" ".repeat(128 * 1024);
    // Whitespace keeps the JSON valid, so refusing cannot be attributed to
    // unknown fields or semantic validation instead of the byte bound.
    let app_body = body.clone();
    let app = Router::new().route(
        "/v1/activity-missions",
        get(move || {
            let body = app_body.clone();
            async move { body }
        }),
    );
    let (base, server) = mission_server(app).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let r = handle_request_async(
        &s,
        &req("activity_missions_catalogue", serde_json::json!({})),
    )
    .await;
    assert!(r.result.is_none());
    assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
    server.abort();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0; 1024];
        let request_bytes = socket.read(&mut request).await.unwrap();
        assert!(request_bytes > 0);
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
            .await
            .unwrap();
        socket
            .write_all(format!("{:X}\r\n", body.len()).as_bytes())
            .await
            .unwrap();
        socket.write_all(body.as_bytes()).await.unwrap();
        socket.write_all(b"\r\n0\r\n\r\n").await.unwrap();
    });
    configure_catalogue(&s, &format!("http://{address}"));
    let r = handle_request_async(
        &s,
        &req("activity_missions_catalogue", serde_json::json!({})),
    )
    .await;
    assert!(r.result.is_none());
    assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
    server.await.unwrap();
}

#[tokio::test]
async fn activity_missions_status_after_signout_does_not_return_previous_account_progress() {
    let s = Arc::new(shared());
    let clear = s.clone();
    let app = Router::new().route(
        "/v1/account/activity-missions/status",
        get(move || {
            let clear = clear.clone();
            async move {
                crate::account_auth::clear_token(&clear.store).unwrap();
                Json(activity_status())
            }
        }),
    );
    let (base, server) = mission_server(app).await;
    configure_catalogue(&s, &base);
    activity_sign_in(&s);
    let r = handle_request_async(&s, &req("activity_missions_status", serde_json::json!({}))).await;
    assert!(
        r.result.is_none(),
        "progress from a signed-out account must not escape"
    );
    assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
    assert!(
        crate::account_auth::try_load_token(&s.store)
            .unwrap()
            .is_none()
    );
    server.abort();
}

#[tokio::test]
async fn activity_missions_rotation_cannot_adopt_a_replacement_account_session() {
    let s = Arc::new(shared());
    let replace = s.clone();
    let app = Router::new().route(
        "/v1/account/activity-missions/status",
        get(move || {
            let replace = replace.clone();
            async move {
                // A new browser session can replace the local native session
                // without changing enrollment or first clearing the credential.
                let replacement = crate::account_auth::AccountSession {
                    access_token: "tcn1_dGVuYW50.replacement".into(),
                    account_id: "replacement-account".into(),
                    expires_at: Utc::now() + chrono::Duration::hours(8),
                };
                replace
                    .store
                    .write_daemon_file(
                        crate::config::ACCOUNT_SESSION_FILE,
                        &serde_json::to_vec(&replacement).unwrap(),
                    )
                    .unwrap();
                (
                    [(
                        trace_commons_protocol::ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
                        "tcn1_dGVuYW50.late-original",
                    )],
                    Json(activity_status()),
                )
            }
        }),
    );
    let (base, server) = mission_server(app).await;
    configure_catalogue(&s, &base);
    activity_sign_in(&s);
    let r = handle_request_async(&s, &req("activity_missions_status", serde_json::json!({}))).await;
    assert!(
        r.result.is_none(),
        "late progress belongs to the original account"
    );
    assert_eq!(r.error.unwrap().message, "activity-missions-unavailable");
    assert_eq!(
        crate::account_auth::try_load_session_with_snapshot(&s.store)
            .unwrap()
            .unwrap()
            .session
            .account_id,
        "replacement-account"
    );
    server.abort();
}
