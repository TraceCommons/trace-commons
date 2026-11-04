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
fn folder_settings_restore_model_trace_and_flags_without_an_account() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let lookup = json!({"tool":"opencode","cwd":project.path()});
    let initial = call(&daemon, "managed_folder_settings_get", lookup.clone());
    assert_eq!(initial["remembered"], false);
    call(
        &daemon,
        "set_settings",
        json!({"managed_folder_settings":[{"tool":"opencode","cwd":project.path(),"model":"provider/model","collect_traces":true,"cli_options":"--agent build"}]}),
    );
    let saved = call(&daemon, "managed_folder_settings_get", lookup.clone());
    assert_eq!(saved["model"], "provider/model");
    assert_eq!(saved["collect_traces"], true);
    assert_eq!(saved["cli_options"], "--agent build");
    assert_eq!(saved["remembered"], true);
    assert!(saved.get("account_id").is_none());
    let other = call(
        &daemon,
        "managed_folder_settings_get",
        json!({"tool":"codex","cwd":project.path()}),
    );
    assert_eq!(other["remembered"], false);
    drop(daemon);
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    assert_eq!(
        call(&daemon, "managed_folder_settings_get", lookup.clone())["model"],
        "provider/model"
    );
    assert_eq!(
        call(&daemon, "managed_folder_settings_reset", lookup)["remembered"],
        false
    );
}

#[test]
fn named_opencode_sign_in_model_can_be_remembered() {
    use trace_commons_contributor::managed::{
        ToolId,
        sessions::{LaunchRequest, SessionRegistry},
        signin,
    };
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let account = signin::entry(ToolId::Opencode).unwrap();
    let request: LaunchRequest = serde_json::from_value(json!({"tool":"opencode","connection":"near_ai_signin","account_id":account.id,"model":"provider/model","cwd":home.path(),"collect_traces":true,"expected_generation":0,"request_id":uuid::Uuid::new_v4()})).unwrap();
    let mut sessions = SessionRegistry::open(&store).unwrap();
    let prepared = sessions.prepare(request, &account).unwrap();
    let context = sessions
        .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
        .unwrap();
    assert_eq!(context.session.model.as_deref(), Some("provider/model"));
    assert!(context.session.collect_traces);
}

#[test]
fn canonical_folder_and_agent_isolate_choices_and_config_updates_merge() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    call(
        &daemon,
        "managed_cli_options_set",
        json!({"tool":"codex","options":"--sandbox workspace-write"}),
    );
    let inherited = call(
        &daemon,
        "managed_folder_settings_get",
        json!({"tool":"codex","cwd":project.path()}),
    );
    assert_eq!(inherited["cli_options"], "--sandbox workspace-write");
    let first = json!({"managed_folder_settings":[{"tool":"codex","cwd":project.path(),"model":"provider/model","cli_options":""}]});
    call(&daemon, "set_settings", first);
    call(
        &daemon,
        "set_settings",
        json!({"managed_folder_settings":[{"tool":"claude","cwd":other.path(),"cli_options":"--model sonnet"}]}),
    );
    let saved = call(
        &daemon,
        "managed_folder_settings_get",
        json!({"tool":"codex","cwd":project.path()}),
    );
    assert_eq!(saved["model"], "provider/model");
    assert_eq!(saved["cli_options"], "");
    #[cfg(unix)]
    {
        let link = home.path().join("link");
        std::os::unix::fs::symlink(project.path(), &link).unwrap();
        assert_eq!(
            call(
                &daemon,
                "managed_folder_settings_get",
                json!({"tool":"codex","cwd":link})
            )["model"],
            "provider/model"
        );
    }
    for bad in [
        json!({"account_id":"not-remembered"}),
        json!({"collect_traces":"yes"}),
        json!({"cli_options":"-c model_provider=other"}),
    ] {
        let mut entry = json!({"tool":"codex","cwd":project.path()});
        entry
            .as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let response = handle_request(
            &daemon,
            &Request {
                id: 1,
                method: "set_settings".into(),
                params: json!({"managed_folder_settings":[entry]}),
            },
        );
        assert!(response.error.is_some());
    }
    assert_eq!(
        call(
            &daemon,
            "managed_folder_settings_reset",
            json!({"tool":"codex","cwd":project.path()})
        )["cli_options"],
        "--sandbox workspace-write"
    );
    assert_eq!(
        call(
            &daemon,
            "managed_folder_settings_get",
            json!({"tool":"claude","cwd":other.path()})
        )["remembered"],
        true
    );
}

#[test]
fn all_agents_can_remember_collection_without_changing_account_selection() {
    use trace_commons_contributor::managed::{
        AccountView, ToolId,
        sessions::{LaunchRequest, SessionRegistry},
    };
    for tool in [ToolId::Claude, ToolId::Codex] {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().into()).unwrap();
        let account: AccountView=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":tool,"connection":"subscription","label":"Selected","auth_state":"ready","verified_at":null})).unwrap();
        let request: LaunchRequest=serde_json::from_value(json!({"request_id":uuid::Uuid::new_v4(),"tool":tool,"connection":"subscription","account_id":account.id,"cwd":home.path(),"collect_traces":true,"expected_generation":0})).unwrap();
        let mut sessions = SessionRegistry::open(&store).unwrap();
        let prepared = sessions.prepare(request, &account).unwrap();
        let context = sessions
            .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
            .unwrap();
        assert_eq!(context.session.account_id, account.id);
        assert!(context.session.collect_traces);
        assert_eq!(
            serde_json::to_value(context.session.collection).unwrap(),
            "pending"
        );
    }
}

#[test]
fn native_collection_allows_parallel_account_runs_for_all_collection_choices() {
    use trace_commons_contributor::managed::{
        AccountView, ToolId,
        sessions::{LaunchRequest, SessionRegistry},
    };
    for tool in [ToolId::Claude, ToolId::Codex] {
        for (first_collects, second_collects) in [(true, false), (false, true), (true, true)] {
            let home = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(home.path().into()).unwrap();
            let account:AccountView=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":tool,"connection":"subscription","label":"Current","auth_state":"ready","verified_at":null})).unwrap();
            let request = |collects: bool, account: &AccountView| -> LaunchRequest {
                serde_json::from_value(json!({"request_id":uuid::Uuid::new_v4(),"tool":tool,"connection":"subscription","account_id":account.id,"cwd":home.path(),"cli_options":"","collect_traces":collects,"expected_generation":0})).unwrap()
            };
            let mut sessions = SessionRegistry::open(&store).unwrap();
            sessions
                .prepare(request(first_collects, &account), &account)
                .unwrap();
            assert!(
                sessions
                    .prepare(request(second_collects, &account), &account)
                    .is_ok(),
                "{tool:?} {first_collects} {second_collects}"
            );
            let mut other = account.clone();
            other.id = trace_commons_contributor::managed::AccountId(uuid::Uuid::new_v4());
            assert!(
                sessions
                    .prepare(request(second_collects, &other), &other)
                    .is_ok()
            );
        }
    }
}

#[test]
fn exact_collection_allows_running_unknown_and_parallel_optout_but_login_stays_locked() {
    use trace_commons_contributor::managed::{
        AccountView, ManagedError, ToolId,
        sessions::{LaunchRequest, LifecycleReport, SessionRegistry},
    };
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().into()).unwrap();
    let account:AccountView=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":"codex","connection":"subscription","label":"Current","auth_state":"ready","verified_at":null})).unwrap();
    let request = |collects: bool| -> LaunchRequest {
        serde_json::from_value(json!({"request_id":uuid::Uuid::new_v4(),"tool":"codex","connection":"subscription","account_id":account.id,"cwd":home.path(),"cli_options":"","collect_traces":collects,"expected_generation":0})).unwrap()
    };
    let mut sessions = SessionRegistry::open(&store).unwrap();
    let first_request = request(true);
    let first = sessions.prepare(first_request.clone(), &account).unwrap();
    assert_eq!(
        sessions
            .prepare(first_request, &account)
            .unwrap()
            .session_id,
        first.session_id
    );
    let context = sessions
        .redeem(first.session_id, first.ticket.as_deref().unwrap())
        .unwrap();
    for state in ["running", "unknown"] {
        let report:LifecycleReport=serde_json::from_value(json!({"session_id":first.session_id,"proof":context.proof,"state":state,"process_id":42,"process_identity":"42:1","exit_code":null})).unwrap();
        sessions.report(report).unwrap();
        assert!(sessions.prepare(request(false), &account).is_ok());
        assert!(sessions.prepare(request(true), &account).is_ok());
    }
    let login: LaunchRequest = serde_json::from_value(json!({
        "request_id":uuid::Uuid::new_v4(),"purpose":"login","tool":"codex","connection":"subscription",
        "account_id":account.id,"cwd":home.path(),"cli_options":"","collect_traces":false,"expected_generation":0
    })).unwrap();
    assert_eq!(
        sessions.prepare(login, &account).unwrap_err(),
        ManagedError::Conflict
    );
    let report:LifecycleReport=serde_json::from_value(json!({"session_id":first.session_id,"proof":context.proof,"state":"exited","process_id":42,"process_identity":"42:1","exit_code":0})).unwrap();
    sessions.report(report).unwrap();
    assert!(sessions.prepare(request(false), &account).is_ok());
    assert!(sessions.prepare(request(false), &account).is_ok());
    let opencode:AccountView=serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":ToolId::Opencode,"connection":"api_key","label":"Current","auth_state":"ready","verified_at":null})).unwrap();
    for _ in 0..2 {
        let request:LaunchRequest=serde_json::from_value(json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":opencode.id,"cwd":home.path(),"model":"provider/model","collect_traces":true,"expected_generation":0})).unwrap();
        sessions.prepare(request, &opencode).unwrap();
    }
}

/// The folder settings the macOS sheet reads say whether the folder has
/// Claude Code workflows, as the launch reads the project (owner,
/// 2026-11-01), so the sheet can show the workflows line without a scan
/// of its own. Only Claude Code has them.
#[test]
fn folder_settings_say_whether_the_project_has_claude_workflows() {
    let home = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    std::fs::create_dir(project.path().join(".git")).unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().into()).unwrap()).unwrap();
    let claude = json!({"tool":"claude","cwd":project.path()});
    assert_eq!(
        call(&daemon, "managed_folder_settings_get", claude.clone())["project_workflows"],
        false
    );
    std::fs::create_dir_all(project.path().join(".claude/workflows")).unwrap();
    std::fs::write(project.path().join(".claude/workflows/release.js"), "").unwrap();
    assert_eq!(
        call(&daemon, "managed_folder_settings_get", claude.clone())["project_workflows"],
        true
    );
    assert_eq!(
        call(&daemon, "managed_folder_settings_reset", claude)["project_workflows"],
        true
    );
    assert_eq!(
        call(
            &daemon,
            "managed_folder_settings_get",
            json!({"tool":"codex","cwd":project.path()})
        )["project_workflows"],
        false
    );
}
