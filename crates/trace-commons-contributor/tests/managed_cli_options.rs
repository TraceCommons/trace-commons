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
fn saved_options_round_trip_through_ipc_and_restart() {
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["cli_options"],
        json!({"claude":"", "codex":"", "opencode":""})
    );
    let options = r#"--model custom -c 'model_reasoning_effort="high"' --sandbox workspace-write"#;
    let revision = call(&daemon, "managed_snapshot", json!({}))["revision"]
        .as_u64()
        .unwrap();
    let saved = call(
        &daemon,
        "managed_cli_options_set",
        json!({"tool":"codex", "options":options}),
    );
    assert_eq!(saved["tool"], "codex");
    assert_eq!(saved["options"], options);
    assert!(saved["revision"].as_u64().unwrap() > revision);
    assert!(
        call(&daemon, "managed_snapshot", json!({}))["revision"]
            .as_u64()
            .unwrap()
            > revision
    );
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["cli_options"]["codex"],
        options
    );
    drop(daemon);
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["cli_options"]["codex"],
        options
    );
    for invalid in [
        "--model 'unterminated",
        "-c model_provider=other",
        "--profile other",
        "-- --config model_provider=other",
    ] {
        let response = handle_request(
            &daemon,
            &Request {
                id: 2,
                method: "managed_cli_options_set".into(),
                params: json!({"tool":"codex", "options":invalid}),
            },
        );
        assert_eq!(response.error.unwrap().code, "managed-cli-options-invalid");
        assert_eq!(
            call(&daemon, "managed_snapshot", json!({}))["cli_options"]["codex"],
            options
        );
    }
    call(
        &daemon,
        "managed_cli_options_set",
        json!({"tool":"codex", "options":""}),
    );
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["cli_options"]["codex"],
        ""
    );
}

#[test]
fn argv_grouping_preserves_literal_shell_syntax_and_empty_values() {
    use trace_commons_contributor::managed::{ToolId, options::parse};
    assert_eq!(parse(ToolId::Claude, r#"--model 'custom model' --append-system-prompt "$(touch injected); $HOME *" --name=two\ words ''"#).unwrap(), ["--model", "custom model", "--append-system-prompt", "$(touch injected); $HOME *", "--name=two words", ""]);
    assert_eq!(
        parse(ToolId::Claude, "--text \"a\\qb\"").unwrap(),
        ["--text", "a\\qb"]
    );
    for value in ["'", "\"", "--model trailing\\", "nul\0value"] {
        assert!(parse(ToolId::Claude, value).is_err());
    }
    assert!(parse(ToolId::Claude, &"x".repeat(16 * 1024 + 1)).is_err());
}

#[test]
fn routing_and_session_overrides_are_rejected_in_every_spelling() {
    use trace_commons_contributor::managed::{ToolId, options::parse};
    for value in [
        "--settings={}",
        "--setting-sources project",
        "--session-id other",
        "--resume previous",
        "--continue",
        "-rSESSION",
        "-cSESSION",
        "-pc",
        "-prSESSION",
        "-vpc",
    ] {
        assert!(parse(ToolId::Claude, value).is_err(), "{value}");
    }
    for value in [
        "--config=model_provider=other",
        "-cmodel_provider=other",
        "-c 'model_providers . other . base_url=\"https://other\"'",
        "-c '\"model_provider\"=\"other\"'",
        "--profile=other",
        "-pother",
        "--cd=/other",
        "-C/other",
        "--oss",
        "-c profiles.other.model_provider=other",
        "resume",
        "resume previous",
        "resume --last",
        "fork previous",
        "--last",
    ] {
        assert!(parse(ToolId::Codex, value).is_err(), "{value}");
    }
    for value in [
        "--session other",
        "-sother",
        "--model other/model",
        "-mother/model",
        "--config=other",
        "--attach http://other",
        "--server http://other",
        "--server=http://other",
        "--standalone=false",
        "--no-standalone",
        "--directory /other",
        "-cSESSION",
    ] {
        assert!(parse(ToolId::Opencode, value).is_err(), "{value}");
    }
    assert!(parse(ToolId::Codex, "--model custom -c 'model_reasoning_effort=\"high\"' --dangerously-bypass-approvals-and-sandbox").is_ok());
    assert!(parse(ToolId::Opencode, "--agent build --log-level DEBUG").is_ok());
    assert!(parse(ToolId::Claude, "-dcore --model custom").is_ok());
}

/// Codex's `resume` and `fork` are refused where Codex reads a subcommand,
/// after any options too, but not as the value of a flag that takes one.
#[test]
fn codex_subcommand_words_are_refused_only_where_codex_reads_a_subcommand() {
    use trace_commons_contributor::managed::{ToolId, options::parse};
    for value in [
        "-m fork",
        "--model resume --search",
        "-c model_reasoning_effort=high -m fork",
        "-s read-only -a never --enable resume",
        "--add-dir fork",
    ] {
        assert!(parse(ToolId::Codex, value).is_ok(), "{value}");
    }
    for value in [
        "-m custom resume",
        "--search fork previous",
        "-i image.png resume",
        "-s read-only fork",
        "-m",
    ] {
        assert!(parse(ToolId::Codex, value).is_err(), "{value}");
    }
}

/// A saved value this build refuses is still shown, so Manage sessions
/// loads and the person can change it, and a launch with it is refused.
#[test]
fn a_saved_value_this_build_refuses_is_shown_but_never_launched() {
    use trace_commons_contributor::managed::{ToolId, options};
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut settings =
        trace_commons_contributor::daemon::settings::DaemonSettings::load(&store).unwrap();
    settings
        .managed_cli_options
        .insert(ToolId::Codex, "resume".into());
    settings
        .managed_cli_options
        .insert(ToolId::Claude, "--model custom".into());
    settings.save(&store).unwrap();
    let shown = options::snapshot(&store).unwrap();
    assert_eq!(shown[&ToolId::Codex], "resume");
    assert_eq!(shown[&ToolId::Claude], "--model custom");
    assert!(options::arguments(&store, ToolId::Codex).is_err());
    assert_eq!(
        options::arguments(&store, ToolId::Claude).unwrap(),
        ["--model", "custom"]
    );
}

#[test]
fn prepare_freezes_options_across_setting_changes_and_login_ignores_them() {
    use trace_commons_contributor::managed::{
        AccountView, ToolId,
        sessions::{LaunchRequest, SessionRegistry},
    };
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
    let mut settings =
        trace_commons_contributor::daemon::settings::DaemonSettings::load(&store).unwrap();
    settings
        .managed_cli_options
        .insert(ToolId::Claude, "--model first".into());
    settings.save(&store).unwrap();
    let account: AccountView = serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","label":"Test","auth_state":"ready","verified_at":null})).unwrap();
    let mut registry = SessionRegistry::open(&store).unwrap();
    let params = json!({"request_id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","account_id":account.id,"cwd":home.path(),"expected_generation":0,"save_default":false});
    let request: LaunchRequest = serde_json::from_value(params.clone()).unwrap();
    let prepared = registry.prepare(request.clone(), &account).unwrap();
    settings
        .managed_cli_options
        .insert(ToolId::Claude, "--model second".into());
    settings.save(&store).unwrap();
    assert_eq!(
        registry.prepare(request, &account).unwrap().session_id,
        prepared.session_id
    );
    let context = registry
        .redeem(prepared.session_id, prepared.ticket.as_deref().unwrap())
        .unwrap();
    assert_eq!(context.cli_options, ["--model", "first"]);
    let mut old = serde_json::to_value(&context).unwrap();
    old.as_object_mut().unwrap().remove("cli_options");
    let old: trace_commons_contributor::managed::sessions::LaunchContext =
        serde_json::from_value(old).unwrap();
    assert!(old.cli_options.is_empty());
    let other: AccountView = serde_json::from_value(json!({"id":uuid::Uuid::new_v4(),"tool":"claude","connection":"subscription","label":"Login","auth_state":"ready","verified_at":null})).unwrap();
    let login: LaunchRequest = serde_json::from_value(json!({"request_id":uuid::Uuid::new_v4(),"purpose":"login","tool":"claude","connection":"subscription","account_id":other.id,"cwd":home.path(),"expected_generation":0})).unwrap();
    let login = registry.prepare(login, &other).unwrap();
    assert!(
        registry
            .redeem(login.session_id, login.ticket.as_deref().unwrap())
            .unwrap()
            .cli_options
            .is_empty()
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(home.path().join("daemon-settings.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}

#[test]
fn codex_config_keys_are_compared_after_toml_decoding() {
    use trace_commons_contributor::managed::{ToolId, options::parse};
    for value in [
        r#"-c '"chatgpt_base_\u0075rl"="https://other"'"#,
        r#"--config='"model_prov\u0069der"="other"'"#,
    ] {
        assert!(parse(ToolId::Codex, value).is_err(), "{value}");
    }
}

#[tokio::test]
async fn socket_dispatch_uses_the_same_settings_transaction_as_the_ui() {
    use trace_commons_contributor::daemon::ipc::handle_request_async;
    let home = tempfile::tempdir().unwrap();
    let daemon = DaemonShared::load(ConfigStore::open(home.path().to_path_buf()).unwrap()).unwrap();
    let response = handle_request_async(
        &daemon,
        &Request {
            id: 8,
            method: "managed_cli_options_set".into(),
            params: json!({"tool":"claude", "options":"--model custom"}),
        },
    )
    .await;
    assert!(response.error.is_none(), "{:?}", response.error);
    assert_eq!(
        call(&daemon, "get_settings", json!({}))["managed_cli_options"]["claude"],
        "--model custom"
    );
    assert_eq!(
        call(&daemon, "managed_snapshot", json!({}))["cli_options"]["claude"],
        "--model custom"
    );
    let response = handle_request_async(
        &daemon,
        &Request {
            id: 9,
            method: "managed_cli_options_set".into(),
            params: json!({"tool":"claude", "options":"--settings={}"}),
        },
    )
    .await;
    assert_eq!(response.error.unwrap().code, "managed-cli-options-invalid");
}

#[test]
fn multiline_shell_continuations_do_not_insert_empty_arguments() {
    use trace_commons_contributor::managed::{ToolId, options::parse};
    assert_eq!(
        parse(
            ToolId::Codex,
            "--model custom \\\n --sandbox workspace-write"
        )
        .unwrap(),
        ["--model", "custom", "--sandbox", "workspace-write"]
    );
    assert!(parse(ToolId::Codex, "\\\n").unwrap().is_empty());
}
