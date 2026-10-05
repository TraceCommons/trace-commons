use serde_json::{Value, json};
use trace_commons_contributor::{
    config::ConfigStore,
    daemon::ipc::{DaemonShared, Request, handle_request},
};

fn call(daemon: &DaemonShared, method: &str, params: Value) -> Value {
    let response = handle_request(
        daemon,
        &Request {
            id: 1,
            method: method.into(),
            params,
        },
    );
    assert!(response.error.is_none(), "{:?}", response.error);
    response.result.unwrap()
}

#[test]
fn cli_preparation_is_visible_in_snapshot_and_duplicate_request_creates_one_session() {
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Personal"}),
    );
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":account["id"],"cwd":home.path(),"expected_generation":0,"save_default":false});
    let prepared = call(&daemon, "managed_launch_prepare", params.clone());
    let repeated = call(&daemon, "managed_launch_prepare", params.clone());
    assert_eq!(prepared["session_id"], repeated["session_id"]);
    let snapshot = call(&daemon, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(snapshot["sessions"][0]["state"], "starting");
    assert_eq!(snapshot["sessions"][0]["account_label"], "Personal");
    assert!(
        !snapshot
            .to_string()
            .contains(prepared["ticket"].as_str().unwrap())
    );
    let mut changed = params;
    changed["connection"] = json!("api_key");
    assert!(
        handle_request(
            &daemon,
            &Request {
                id: 2,
                method: "managed_launch_prepare".into(),
                params: changed
            }
        )
        .error
        .is_some()
    );
}

#[test]
fn ticket_is_single_use_wrong_helper_cannot_report_and_restart_retains_account() {
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"codex","connection":"subscription","label":"Work"}),
    );
    let prepared = call(
        &daemon,
        "managed_launch_prepare",
        json!({"request_id":uuid::Uuid::new_v4(),"tool":"codex","connection":"subscription","account_id":account["id"],"cwd":home.path(),"expected_generation":0,"save_default":false}),
    );
    let redeem = json!({"session_id":prepared["session_id"],"ticket":prepared["ticket"]});
    let context = call(&daemon, "managed_launch_redeem", redeem.clone());
    assert!(
        handle_request(
            &daemon,
            &Request {
                id: 3,
                method: "managed_launch_redeem".into(),
                params: redeem
            }
        )
        .error
        .is_some()
    );
    let mut report = json!({"session_id":prepared["session_id"],"proof":context["proof"],"state":"running","process_id":42,"process_identity":"fixture-process-start-1","exit_code":null});
    let mut forged = report.clone();
    forged["proof"] = json!("wrong");
    assert!(
        handle_request(
            &daemon,
            &Request {
                id: 4,
                method: "managed_session_report".into(),
                params: forged
            }
        )
        .error
        .is_some()
    );
    call(&daemon, "managed_session_report", report.clone());
    let snapshot = call(&daemon, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"][0]["state"], "running");
    drop(daemon);
    let restarted =
        DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let snapshot = call(&restarted, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"][0]["state"], "unknown");
    assert_eq!(snapshot["sessions"][0]["account_id"], account["id"]);
    assert!(
        handle_request(
            &restarted,
            &Request {
                id: 5,
                method: "managed_account_remove".into(),
                params: json!({"account_id":account["id"]})
            }
        )
        .error
        .is_some()
    );
    call(&restarted, "managed_session_report", report.clone());
    report["state"] = json!("exited");
    report["exit_code"] = json!(7);
    call(&restarted, "managed_session_report", report);
    let snapshot = call(&restarted, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"][0]["state"], "exited");
    assert_eq!(snapshot["sessions"][0]["exit_code"], 7);
}

#[test]
fn native_login_locks_its_profile_without_changing_other_accounts_or_defaults() {
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let a = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Personal"}),
    );
    let b = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Work"}),
    );
    let launch = call(
        &daemon,
        "managed_account_reconnect",
        json!({"account_id":a["id"]}),
    );
    assert!(launch["ticket"].is_string());
    let snapshot = call(&daemon, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"][0]["purpose"], "login");
    assert!(snapshot["defaults"].as_array().unwrap().is_empty());
    assert!(
        handle_request(
            &daemon,
            &Request {
                id: 9,
                method: "managed_account_reconnect".into(),
                params: json!({"account_id":a["id"]})
            }
        )
        .error
        .is_some()
    );
    let second = call(
        &daemon,
        "managed_account_reconnect",
        json!({"account_id":b["id"]}),
    );
    assert_ne!(second["session_id"], launch["session_id"]);
}

#[test]
fn native_exit_receipt_reconciles_after_daemon_downtime() {
    use trace_commons_contributor::managed::sessions::{
        LifecycleReport, SessionState, persist_exit,
    };
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let daemon = DaemonShared::load(store).unwrap();
    let a = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Personal"}),
    );
    let p = call(
        &daemon,
        "managed_launch_prepare",
        json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":a["id"],"cwd":home.path(),"expected_generation":0,"save_default":false}),
    );
    let context = call(
        &daemon,
        "managed_launch_redeem",
        json!({"session_id":p["session_id"],"ticket":p["ticket"]}),
    );
    drop(daemon);
    let store = ConfigStore::open(home.path().into()).unwrap();
    persist_exit(
        &store,
        &LifecycleReport {
            session_id: serde_json::from_value(p["session_id"].clone()).unwrap(),
            proof: context["proof"].as_str().unwrap().into(),
            state: SessionState::Exited,
            process_id: Some(123),
            process_identity: Some("123:456".into()),
            exit_code: Some(7),
        },
    )
    .unwrap();
    let restarted = DaemonShared::load(store).unwrap();
    let snapshot = call(&restarted, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"][0]["state"], "exited");
    assert_eq!(snapshot["sessions"][0]["exit_code"], 7);
    assert!(
        call(
            &restarted,
            "managed_account_reconnect",
            json!({"account_id":a["id"]})
        )["ticket"]
            .is_string()
    );
}

#[test]
fn native_shell_fixture_uses_the_shared_rust_copy() {
    let fixture: Value = serde_json::from_str(include_str!(
        "../../../tests/fixtures/managed/snapshot.json"
    ))
    .unwrap();
    assert_eq!(
        fixture["copy"],
        trace_commons_contributor::managed::copy::copy()
    );
    assert!(!fixture.to_string().contains("ticket"));
    assert!(!fixture.to_string().contains("proof"));
}

#[test]
fn dismissing_a_row_does_not_make_its_launch_request_replayable() {
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let a = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Personal"}),
    );
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":a["id"],"cwd":home.path(),"expected_generation":0,"save_default":false});
    let launch = call(&daemon, "managed_launch_prepare", params.clone());
    let context = call(
        &daemon,
        "managed_launch_redeem",
        json!({"session_id":launch["session_id"],"ticket":launch["ticket"]}),
    );
    call(
        &daemon,
        "managed_session_report",
        json!({"session_id":launch["session_id"],"proof":context["proof"],"state":"failed","process_id":null,"process_identity":null,"exit_code":null}),
    );
    call(
        &daemon,
        "managed_session_dismiss",
        json!({"session_id":launch["session_id"]}),
    );
    let repeated = call(&daemon, "managed_launch_prepare", params);
    assert_eq!(repeated["session_id"], launch["session_id"]);
    assert!(repeated["ticket"].is_null());
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["sessions"],
        json!([])
    );
}
