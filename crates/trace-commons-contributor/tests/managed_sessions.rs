use serde_json::{Value, json};
use trace_commons_contributor::{
    config::ConfigStore,
    daemon::ipc::{DaemonShared, Request, handle_request},
};

// Each IPC launch test runs in a child with version-only executable fixtures on PATH.
// This exercises discovery without requiring installed tools or changing the
// parent test process environment while other tests are running.
fn native_fixture_child(test: &str) -> bool {
    if std::env::var("TRACE_MANAGED_TEST_CHILD").as_deref() == Ok(test) {
        return false;
    }
    static BIN: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    let bin = BIN.get_or_init(|| {
        let bin = tempfile::tempdir().unwrap();
        let source = bin.path().join("fixture.rs");
        std::fs::write(&source, r#"fn main() {
            if std::env::var("TRACE_MANAGED_FIXTURE_BAD_VERSION").as_deref() == Ok("true") { println!("0.0.0"); return; }
            let exe = std::env::current_exe().unwrap();
            let name = exe.file_stem().unwrap().to_str().unwrap();
            println!("{}", match name { "claude" => "2.1.289", "codex" => "0.160.0", _ => "1.19.0" });
        }"#).unwrap();
        let status = std::process::Command::new("rustc")
            .arg(&source).arg("-o").arg(bin.path().join(if cfg!(windows) { "fixture-native.exe" } else { "fixture-native" }))
            .status().unwrap();
        assert!(status.success());
    for tool in ["claude", "codex", "opencode"] {
        let path = bin.path().join(if cfg!(windows) {
            format!("{tool}.exe")
        } else {
            tool.into()
        });
        std::fs::copy(bin.path().join(if cfg!(windows) { "fixture-native.exe" } else { "fixture-native" }), path).unwrap();
    }
        bin
    });
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", test, "--nocapture"])
        .env("TRACE_MANAGED_TEST_CHILD", test)
        .env(
            "TRACE_MANAGED_FIXTURE_BAD_VERSION",
            (test == "unsupported_version_refuses_ticket_and_preserves_account").to_string(),
        )
        .env("PATH", bin.path())
        .status()
        .unwrap();
    assert!(status.success(), "fixture child failed: {test}");
    true
}

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
    if native_fixture_child(
        "cli_preparation_is_visible_in_snapshot_and_duplicate_request_creates_one_session",
    ) {
        return;
    }
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
    let map = call(&daemon, "tool_destinations", json!({}));
    assert_eq!(
        map["managed_sessions"],
        json!([{
            "id": prepared["session_id"], "tool": "claude", "connection": "subscription",
            "provider": "anthropic", "model": null, "collection_state": "off", "state": "starting"
        }])
    );
    assert_eq!(map["managed_copy"], snapshot["copy"]);
    let opencode = map["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["tool"] == "opencode")
        .unwrap();
    assert_eq!(opencode["installed"], true);
    assert_eq!(opencode["model_calls"]["to"], "unknown");
    assert_ne!(opencode["sessions"]["watch"], "watched");
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
    if native_fixture_child(
        "ticket_is_single_use_wrong_helper_cannot_report_and_restart_retains_account",
    ) {
        return;
    }
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
    if native_fixture_child(
        "native_login_locks_its_profile_without_changing_other_accounts_or_defaults",
    ) {
        return;
    }
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
    assert_eq!(
        call(&daemon, "tool_destinations", json!({}))["managed_sessions"],
        json!([])
    );
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
    if native_fixture_child("native_exit_receipt_reconciles_after_daemon_downtime") {
        return;
    }
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
            model: None,
            collection: None,
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
    if native_fixture_child("dismissing_a_row_does_not_make_its_launch_request_replayable") {
        return;
    }
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

#[test]
fn unsupported_version_refuses_ticket_and_preserves_account() {
    if native_fixture_child("unsupported_version_refuses_ticket_and_preserves_account") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude", "connection":"subscription", "label":"Personal"}),
    );
    for (method, params) in [
        (
            "managed_launch_prepare",
            json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":account["id"],"cwd":home.path(),"expected_generation":0,"save_default":false}),
        ),
        (
            "managed_account_reconnect",
            json!({"account_id":account["id"]}),
        ),
    ] {
        let result = handle_request(
            &daemon,
            &Request {
                id: 1,
                method: method.into(),
                params,
            },
        );
        assert_eq!(result.error.unwrap().code, "managed-unsupported-version");
    }
    let snapshot = call(&daemon, "managed_snapshot", json!({}));
    assert_eq!(snapshot["sessions"], json!([]));
    assert_eq!(snapshot["accounts"][0], account);
    assert_eq!(snapshot["tools"][0]["installed"], true);
}

/// A launch the Launch sheet fell back to the default for, because NEAR
/// AI's catalogue omitted the folder's saved model, says
/// `keep_saved_model`: its other choices are remembered, the saved model
/// is not replaced by the fallback (#1754 review).
#[test]
fn a_fallback_launch_keeps_the_folders_saved_model() {
    if native_fixture_child("a_fallback_launch_keeps_the_folders_saved_model") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"opencode","connection":"api_key","label":"Current"}),
    );
    call(
        &daemon,
        "set_settings",
        json!({"managed_folder_settings":[{"tool":"opencode","cwd":project.path(),"model":"provider/saved","cli_options":""}]}),
    );
    let lookup = json!({"tool":"opencode","cwd":project.path()});
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":account["id"],"cwd":project.path(),"model":"provider/fallback","collect_traces":true,"cli_options":"--agent build","expected_generation":0,"keep_saved_model":true});
    call(&daemon, "managed_launch_prepare", params);
    let prefs = call(&daemon, "managed_folder_settings_get", lookup.clone());
    assert_eq!(prefs["model"], "provider/saved");
    assert_eq!(prefs["collect_traces"], true);
    assert_eq!(prefs["cli_options"], "--agent build");
    // Without the flag, the launch's model is remembered, as before.
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":account["id"],"cwd":project.path(),"model":"provider/chosen","collect_traces":true,"cli_options":"","expected_generation":0});
    let r = handle_request(
        &daemon,
        &Request {
            id: 1,
            method: "managed_launch_prepare".into(),
            params,
        },
    );
    // The first session may still hold the account; only the remembered
    // model matters here, and a refused launch remembers nothing.
    if r.error.is_none() {
        assert_eq!(
            call(&daemon, "managed_folder_settings_get", lookup)["model"],
            "provider/chosen"
        );
    }
}

#[test]
fn prepared_folder_choices_are_captured_without_remembering_an_account() {
    if native_fixture_child("prepared_folder_choices_are_captured_without_remembering_an_account") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"opencode","connection":"api_key","label":"Current"}),
    );
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":account["id"],"cwd":project.path(),"model":"provider/model","collect_traces":true,"cli_options":"--agent build","expected_generation":0});
    let prepared = call(&daemon, "managed_launch_prepare", params.clone());
    let lookup = json!({"tool":"opencode","cwd":project.path()});
    let prefs = call(&daemon, "managed_folder_settings_get", lookup.clone());
    assert_eq!(prefs["model"], "provider/model");
    assert_eq!(prefs["collect_traces"], true);
    assert_eq!(prefs["cli_options"], "--agent build");
    assert!(prefs.get("account_id").is_none());
    // A cached request can never overwrite a later change in preferences.
    call(
        &daemon,
        "set_settings",
        json!({"managed_folder_settings":[{"tool":"opencode","cwd":project.path(),"model":"provider/new","cli_options":""}]}),
    );
    assert_eq!(
        call(&daemon, "managed_launch_prepare", params)["session_id"],
        prepared["session_id"]
    );
    assert_eq!(
        call(&daemon, "managed_folder_settings_get", lookup)["model"],
        "provider/new"
    );
    let context = call(
        &daemon,
        "managed_launch_redeem",
        json!({"session_id":prepared["session_id"],"ticket":prepared["ticket"]}),
    );
    assert_eq!(context["cli_options"], json!(["--agent", "build"]));
    assert_eq!(context["session"]["model"], "provider/model");
    assert_eq!(context["session"]["collect_traces"], true);
    let settings = call(&daemon, "get_settings", json!({}));
    let entry = &settings["managed_folder_settings"][0];
    assert!(entry.get("account_id").is_none());
}

#[test]
fn failed_folder_preference_save_rolls_back_the_unredeemed_launch() {
    if native_fixture_child("failed_folder_preference_save_rolls_back_the_unredeemed_launch") {
        return;
    }
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let account = call(
        &daemon,
        "managed_account_add",
        json!({"tool":"claude","connection":"subscription","label":"Selected"}),
    );
    call(
        &daemon,
        "set_settings",
        json!({"notifications_enabled":true}),
    );
    let path = home.path().join("daemon-settings.json");
    let bytes = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    std::fs::create_dir(&path).unwrap();
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":account["id"],"cwd":home.path(),"cli_options":"","expected_generation":0});
    let refused = handle_request(
        &daemon,
        &Request {
            id: 1,
            method: "managed_launch_prepare".into(),
            params: params.clone(),
        },
    );
    assert!(refused.error.is_some());
    std::fs::remove_dir(&path).unwrap();
    std::fs::write(&path, bytes).unwrap();
    assert!(
        call(&daemon, "managed_snapshot", json!({}))["sessions"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let retry = call(&daemon, "managed_launch_prepare", params);
    assert!(retry["ticket"].is_string());
}
