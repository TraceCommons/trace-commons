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

// ---- the contribution-mission slot, fed from the activity catalogue ----

/// A published activity catalogue whose missions carry `predicates` (one
/// per mission, `null` for none), with a digest that verifies. The policy
/// runs from yesterday for thirty days.
pub(super) fn predicate_catalogue(predicates: &[serde_json::Value]) -> serde_json::Value {
    let today = Utc::now().date_naive();
    predicate_catalogue_between(
        predicates,
        today - chrono::Duration::days(1),
        today + chrono::Duration::days(30),
    )
}

/// [`predicate_catalogue`] with the policy's dates given.
fn predicate_catalogue_between(
    predicates: &[serde_json::Value],
    starts_on: chrono::NaiveDate,
    ends_before: chrono::NaiveDate,
) -> serde_json::Value {
    let missions: Vec<serde_json::Value> = predicates
        .iter()
        .enumerate()
        .map(|(i, predicate)| {
            let mut mission = serde_json::json!({
                "id": format!("m-{i}"),
                "title": format!("Mission {i}"),
                "required_contributions": 1,
            });
            if !predicate.is_null() {
                mission["predicate"] = predicate.clone();
            }
            mission
        })
        .collect();
    let policy: trace_commons_protocol::activity_missions::ActivityPolicy =
        serde_json::from_value(serde_json::json!({
            "schema_version": 1, "policy_id": "test-policy",
            "starts_on": starts_on,
            "ends_before": ends_before,
            "qualification": "accepted", "missions": missions,
            "daily": null, "levels": null, "badges": null,
        }))
        .unwrap();
    serde_json::to_value(
        trace_commons_protocol::activity_missions::ActivityCatalogue::new(Some(policy)).unwrap(),
    )
    .unwrap()
}

fn claude_rust() -> serde_json::Value {
    serde_json::json!({"version":1,"tools":["claude-code"],"languages":["rust"],"min_sessions":2})
}

fn parsed(raw: serde_json::Value) -> trace_commons_protocol::activity_missions::ActivityCatalogue {
    serde_json::from_value(raw).unwrap()
}

/// Only missions with a version-1 predicate reach the matcher, each with
/// its predicate as criteria. A mission without one, or with a version
/// this build does not read, is left out, so fitting a mission never means
/// fitting everything.
#[test]
fn the_mission_slot_takes_only_missions_with_a_supported_predicate() {
    use crate::daemon::activity_missions::contribution_catalogue;
    let today = Utc::now().date_naive();
    let v2 = serde_json::json!({"version":2,"tools":["codex"],"repo_size":"large"});
    let raw = contribution_catalogue(
        &parsed(predicate_catalogue(&[
            serde_json::Value::Null,
            claude_rust(),
            v2,
        ])),
        today,
    )
    .expect("one mission carries a supported predicate");
    assert_eq!(
        raw,
        serde_json::json!({"schema_version": 1, "missions": [{
            "mission_id": "m-1", "title": "Mission 1",
            "criteria": {"tools": ["claude-code"], "tool_families": [], "languages": ["rust"], "min_sessions": 2},
        }]})
    );
    crate::contribution_missions::ContributionMissionCatalogue::from_value(&raw)
        .expect("the client catalogue reads it");

    // Nothing to match on: no slot, never a slot that fits everything.
    assert_eq!(
        contribution_catalogue(
            &parsed(predicate_catalogue(&[serde_json::Value::Null])),
            today
        ),
        None
    );
    assert_eq!(
        contribution_catalogue(&parsed(activity_empty_catalogue()), today),
        None
    );
    // Outside the policy's dates its missions are not on offer.
    let catalogue = parsed(predicate_catalogue(&[claude_rust()]));
    assert!(contribution_catalogue(&catalogue, today).is_some());
    assert_eq!(
        contribution_catalogue(&catalogue, today + chrono::Duration::days(30)),
        None
    );
    assert_eq!(
        contribution_catalogue(&catalogue, today - chrono::Duration::days(2)),
        None
    );
}

/// The refresh is due at once, then every [`MISSION_SLOT_REFRESH`] after
/// an attempt; a clock that moved back does not postpone it past one
/// interval.
#[test]
fn the_mission_slot_schedule_runs_at_start_and_every_interval() {
    use crate::daemon::activity_missions::{MISSION_SLOT_REFRESH, MissionSlotSchedule};
    use crate::daemon::mission_matching::MISSION_CATALOGUE_MAX_AGE;
    assert!(MISSION_SLOT_REFRESH * 3 < MISSION_CATALOGUE_MAX_AGE);
    let start = Utc::now();
    let interval = chrono::TimeDelta::from_std(MISSION_SLOT_REFRESH).unwrap();
    let mut schedule = MissionSlotSchedule::default();
    assert!(schedule.due(start));
    schedule.attempted(start);
    assert!(!schedule.due(start + interval - chrono::TimeDelta::seconds(1)));
    assert!(schedule.due(start + interval));
    assert!(
        schedule.due(start - chrono::TimeDelta::seconds(1)),
        "clock moved back"
    );
    schedule.unconfigured();
    assert!(schedule.due(start), "a later enrollment fetches at once");
}

/// A mock ingest whose `/v1/activity-missions` answer can be swapped, and
/// which counts requests and checks each is anonymous.
pub(super) async fn activity_server(
    answer: Arc<std::sync::Mutex<(StatusCode, serde_json::Value)>>,
    calls: Arc<AtomicUsize>,
) -> (String, tokio::task::JoinHandle<()>) {
    let app = Router::new().route(
        "/v1/activity-missions",
        get(
            move |headers: HeaderMap, Query(query): Query<BTreeMap<String, String>>| {
                let answer = answer.clone();
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                    assert!(query.is_empty());
                    for name in ["authorization", "cookie", "x-tenant-id", "x-profile"] {
                        assert!(!headers.contains_key(name));
                    }
                    let (status, body) = answer.lock().unwrap().clone();
                    (status, Json(body))
                }
            },
        ),
    );
    mission_server(app).await
}

fn slot_missions(s: &DaemonShared) -> Option<Vec<String>> {
    crate::daemon::mission_matching::live_catalogue(&s.mission_catalogue, Utc::now())
        .map(|c| c.missions.into_iter().map(|m| m.mission_id).collect())
}

/// A predicate-bearing catalogue fills the slot; a failed fetch keeps it
/// (it ages out on its own); a catalogue with nothing to match, or none at
/// all, empties it.
#[tokio::test]
async fn the_mission_slot_refresh_fills_keeps_and_clears() {
    use crate::daemon::activity_missions::{
        MISSION_SLOT_REFRESH, MissionSlotSchedule, refresh_mission_slot,
    };
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust(), serde_json::Value::Null]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer.clone(), calls.clone()).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let mut schedule = MissionSlotSchedule::default();
    let step = chrono::TimeDelta::from_std(MISSION_SLOT_REFRESH).unwrap();
    let mut now = Utc::now();
    let before = persisted_files(s.store.dir());

    refresh_mission_slot(&s, now, &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert_eq!(slot_missions(&s), Some(vec!["m-0".to_string()]));

    // Not due again until the interval has passed.
    refresh_mission_slot(
        &s,
        now + step - chrono::TimeDelta::seconds(1),
        &mut schedule,
    )
    .await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A failed fetch, or a catalogue whose digest does not verify, keeps
    // what is there.
    let kept = format!("{:?}", s.mission_catalogue.lock().unwrap());
    for failure in [
        (StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({})),
        (StatusCode::OK, {
            let mut wrong = predicate_catalogue(&[claude_rust()]);
            wrong["policy_sha256"] = "0".repeat(64).into();
            wrong
        }),
    ] {
        *answer.lock().unwrap() = failure;
        now += step;
        refresh_mission_slot(&s, now, &mut schedule).await;
        assert_eq!(format!("{:?}", s.mission_catalogue.lock().unwrap()), kept);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 3);

    // Missions without a predicate: nothing to match, so no slot.
    *answer.lock().unwrap() = (
        StatusCode::OK,
        predicate_catalogue(&[serde_json::Value::Null]),
    );
    now += step;
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert!(s.mission_catalogue.lock().unwrap().is_none());

    // Filled again, then an unconfigured policy empties it.
    *answer.lock().unwrap() = (StatusCode::OK, predicate_catalogue(&[claude_rust()]));
    now += step;
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert!(s.mission_catalogue.lock().unwrap().is_some());
    *answer.lock().unwrap() = (StatusCode::OK, activity_empty_catalogue());
    now += step;
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert!(s.mission_catalogue.lock().unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 6);

    // The slot is memory only.
    assert_eq!(persisted_files(s.store.dir()), before);
    server.abort();
}

/// Without a config there is no origin to ask: nothing is sent, and the
/// first tick after enrollment fetches.
#[tokio::test]
async fn the_mission_slot_refresh_waits_for_enrollment() {
    use crate::daemon::activity_missions::{MissionSlotSchedule, refresh_mission_slot};
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust()]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer, calls.clone()).await;
    let s = shared();
    let mut schedule = MissionSlotSchedule::default();
    let now = Utc::now();
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert!(s.mission_catalogue.lock().unwrap().is_none());
    assert!(schedule.due(now));

    configure_catalogue(&s, &base);
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(5), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(s.mission_catalogue.lock().unwrap().is_some());

    // Unenroll empties it, and with no config left the next due refresh
    // sends nothing and fills nothing.
    crate::daemon::unenroll::unenroll(&s).unwrap();
    assert!(s.mission_catalogue.lock().unwrap().is_none());
    refresh_mission_slot(&s, now + chrono::TimeDelta::days(1), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    assert!(s.mission_catalogue.lock().unwrap().is_none());
    server.abort();
}

/// Enrolling again well inside the refresh interval still fetches on the
/// next tick: the earlier enrollment's schedule does not carry over, whether
/// a tick saw no config in between or the new enrollment (a new device key)
/// replaced the old one between two ticks.
#[tokio::test]
async fn the_mission_slot_refills_on_the_first_tick_after_enrolling_again() {
    use crate::daemon::activity_missions::{MissionSlotSchedule, refresh_mission_slot};
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust()]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer, calls.clone()).await;
    let s = shared();
    let mut schedule = MissionSlotSchedule::default();
    let now = Utc::now();
    configure_catalogue(&s, &base);
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // A tick sees no config, then the next one sees the new enrollment.
    crate::daemon::unenroll::unenroll(&s).unwrap();
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(10), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    configure_catalogue(&s, &base);
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(15), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(s.mission_catalogue.lock().unwrap().is_some());

    // Unenrolled and enrolled again between two ticks: no tick ever sees
    // the gap, but the enrollment's device key is new.
    crate::daemon::unenroll::unenroll(&s).unwrap();
    configure_catalogue(&s, &base);
    let mut cfg = s.store.load_config().unwrap().unwrap();
    cfg.device_key_id = "sha256:another-device-key".into();
    s.store.save_config(&cfg).unwrap();
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(20), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    assert!(s.mission_catalogue.lock().unwrap().is_some());

    // The same enrollment inside the interval is not asked again.
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(25), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    server.abort();
}

/// A slot filled the day before the policy's `ends_before` stops being live
/// at that day's start (UTC), though it is younger than
/// `MISSION_CATALOGUE_MAX_AGE` and no fetch has run since: the policy's
/// missions are no longer on offer.
#[tokio::test]
async fn the_mission_slot_is_not_live_past_the_policy_end() {
    use crate::daemon::activity_missions::{MissionSlotSchedule, refresh_mission_slot};
    use crate::daemon::mission_matching::{MISSION_CATALOGUE_MAX_AGE, live_catalogue};
    let now = Utc::now();
    let today = now.date_naive();
    let tomorrow = today + chrono::Duration::days(1);
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue_between(&[claude_rust()], today, tomorrow),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer, calls.clone()).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let mut schedule = MissionSlotSchedule::default();
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let end = tomorrow.and_time(chrono::NaiveTime::MIN).and_utc();
    assert!(end - now < chrono::TimeDelta::from_std(MISSION_CATALOGUE_MAX_AGE).unwrap());
    assert!(live_catalogue(&s.mission_catalogue, now).is_some());
    assert!(live_catalogue(&s.mission_catalogue, end - chrono::TimeDelta::seconds(1)).is_some());
    assert!(
        live_catalogue(&s.mission_catalogue, end).is_none(),
        "the policy has ended: unknown, never its old missions"
    );
    server.abort();
}

/// Shells learn of every change to what the slot renders, including the
/// ones no fetch makes: when the slot ages out after failed fetches, the
/// first tick past the age limit -- due or not -- publishes `queue_changed`
/// and `status_changed`, once.
#[tokio::test]
async fn the_mission_slot_publishes_when_it_ages_out() {
    use crate::daemon::activity_missions::{
        MISSION_SLOT_REFRESH, MissionSlotSchedule, refresh_mission_slot,
    };
    use crate::daemon::ipc::{EVENT_QUEUE_CHANGED, EVENT_STATUS_CHANGED};
    use crate::daemon::mission_matching::{MISSION_CATALOGUE_MAX_AGE, live_catalogue};
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust()]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer.clone(), calls.clone()).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let mut events = s.events.subscribe();
    let mut published = || {
        let mut names = Vec::new();
        while let Ok(event) = events.try_recv() {
            names.push(event.event);
        }
        names
    };
    let step = chrono::TimeDelta::from_std(MISSION_SLOT_REFRESH).unwrap();
    let max_age = chrono::TimeDelta::from_std(MISSION_CATALOGUE_MAX_AGE).unwrap();
    let mut schedule = MissionSlotSchedule::default();
    let now = Utc::now();

    refresh_mission_slot(&s, now, &mut schedule).await;
    assert_eq!(
        published(),
        vec![EVENT_QUEUE_CHANGED.to_string(), EVENT_STATUS_CHANGED.to_string()]
    );

    // Every later fetch fails: the slot is kept, and nothing changes for a
    // shell while it is live.
    *answer.lock().unwrap() = (StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({}));
    let mut at = now;
    while at + step <= now + max_age {
        at += step;
        refresh_mission_slot(&s, at, &mut schedule).await;
        assert!(published().is_empty());
    }
    assert!(live_catalogue(&s.mission_catalogue, at).is_some());
    let fetched = calls.load(Ordering::SeqCst);

    // Past the age limit, on a tick that is not due to fetch.
    let past = now + max_age + chrono::TimeDelta::seconds(1);
    assert!(!schedule.due(past));
    refresh_mission_slot(&s, past, &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), fetched);
    assert!(live_catalogue(&s.mission_catalogue, past).is_none());
    assert_eq!(
        published(),
        vec![EVENT_QUEUE_CHANGED.to_string(), EVENT_STATUS_CHANGED.to_string()]
    );
    refresh_mission_slot(&s, past + chrono::TimeDelta::seconds(1), &mut schedule).await;
    assert!(published().is_empty(), "published once, not on every tick");
    server.abort();
}

/// A fetch that passed its enrollment re-check just before `unenroll` can
/// still write the slot just after `unenroll` emptied it. The next tick,
/// which finds no config, empties it again and tells shells, so the old
/// enrollment's missions do not stay live under no enrollment, or under the
/// next one if its first fetch fails.
#[tokio::test]
async fn the_mission_slot_is_emptied_on_a_tick_without_a_config() {
    use crate::daemon::activity_missions::{
        MissionSlotSchedule, contribution_catalogue, refresh_mission_slot,
    };
    use crate::daemon::ipc::{EVENT_QUEUE_CHANGED, EVENT_STATUS_CHANGED};
    use crate::daemon::mission_matching::receive_catalogue;
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust()]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer, calls.clone()).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let mut schedule = MissionSlotSchedule::default();
    let now = Utc::now();
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert!(slot_missions(&s).is_some());

    // Unenroll, then the racing write lands.
    crate::daemon::unenroll::unenroll(&s).unwrap();
    let raw = contribution_catalogue(
        &parsed(predicate_catalogue(&[claude_rust()])),
        now.date_naive(),
    )
    .unwrap();
    receive_catalogue(&s.mission_catalogue, &raw, now).unwrap();
    assert!(slot_missions(&s).is_some());

    let mut events = s.events.subscribe();
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(5), &mut schedule).await;
    assert!(s.mission_catalogue.lock().unwrap().is_none());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
    let mut names = Vec::new();
    while let Ok(event) = events.try_recv() {
        names.push(event.event);
    }
    assert_eq!(
        names,
        vec![EVENT_QUEUE_CHANGED.to_string(), EVENT_STATUS_CHANGED.to_string()]
    );
    server.abort();
}

/// A slot filled under one enrollment is never read under another: the
/// first tick under a new enrollment empties it before fetching, so when
/// that first fetch fails, the new enrollment's mission fit is unknown
/// rather than the old enrollment's missions.
#[tokio::test]
async fn the_mission_slot_does_not_carry_over_to_a_new_enrollment() {
    use crate::daemon::activity_missions::{MissionSlotSchedule, refresh_mission_slot};
    let answer = Arc::new(std::sync::Mutex::new((
        StatusCode::OK,
        predicate_catalogue(&[claude_rust()]),
    )));
    let calls = Arc::new(AtomicUsize::new(0));
    let (base, server) = activity_server(answer.clone(), calls.clone()).await;
    let s = shared();
    configure_catalogue(&s, &base);
    let mut schedule = MissionSlotSchedule::default();
    let now = Utc::now();
    refresh_mission_slot(&s, now, &mut schedule).await;
    assert!(slot_missions(&s).is_some());

    // A new enrollment between two ticks, and its first fetch fails.
    let mut cfg = s.store.load_config().unwrap().unwrap();
    cfg.device_key_id = "sha256:another-device-key".into();
    s.store.save_config(&cfg).unwrap();
    *answer.lock().unwrap() = (StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({}));
    refresh_mission_slot(&s, now + chrono::TimeDelta::seconds(5), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    assert!(s.mission_catalogue.lock().unwrap().is_none());

    // The same enrollment's failed fetch keeps its own slot.
    *answer.lock().unwrap() = (StatusCode::OK, predicate_catalogue(&[claude_rust()]));
    refresh_mission_slot(&s, now + chrono::TimeDelta::days(1), &mut schedule).await;
    assert!(s.mission_catalogue.lock().unwrap().is_some());
    *answer.lock().unwrap() = (StatusCode::SERVICE_UNAVAILABLE, serde_json::json!({}));
    refresh_mission_slot(&s, now + chrono::TimeDelta::days(2), &mut schedule).await;
    assert_eq!(calls.load(Ordering::SeqCst), 4);
    assert!(s.mission_catalogue.lock().unwrap().is_some());
    server.abort();
}

/// A catalogue that comes back after the enrollment it was asked for has
/// gone -- `unenroll` ran, or the config moved to another server, while the
/// request was in flight -- is dropped: the slot stays as `unenroll` or the
/// move left it, checked on the very tick the response lands.
#[tokio::test]
async fn the_mission_slot_drops_a_catalogue_that_lands_after_the_enrollment_went() {
    use crate::daemon::activity_missions::{MissionSlotSchedule, refresh_mission_slot};
    for unenroll in [true, false] {
        let s = Arc::new(shared());
        let calls = Arc::new(AtomicUsize::new(0));
        let app = Router::new().route(
            "/v1/activity-missions",
            get({
                let s = s.clone();
                let calls = calls.clone();
                move || {
                    let s = s.clone();
                    let calls = calls.clone();
                    async move {
                        calls.fetch_add(1, Ordering::SeqCst);
                        if unenroll {
                            crate::daemon::unenroll::unenroll(&s).unwrap();
                        } else {
                            let mut cfg = s.store.load_config().unwrap().unwrap();
                            cfg.ingest_url = "http://127.0.0.1:9/v1/traces".into();
                            s.store.save_config(&cfg).unwrap();
                        }
                        Json(predicate_catalogue(&[claude_rust()]))
                    }
                }
            }),
        );
        let (base, server) = mission_server(app).await;
        configure_catalogue(&s, &base);
        let mut schedule = MissionSlotSchedule::default();
        refresh_mission_slot(&s, Utc::now(), &mut schedule).await;
        assert_eq!(calls.load(Ordering::SeqCst), 1, "unenroll: {unenroll}");
        assert!(
            s.mission_catalogue.lock().unwrap().is_none(),
            "unenroll: {unenroll}"
        );
        server.abort();
    }
}
