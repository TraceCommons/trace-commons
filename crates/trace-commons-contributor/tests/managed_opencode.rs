use trace_commons_contributor::{
    config::ConfigStore,
    managed::{ConnectionKind, ToolId, accounts::AccountStore},
    source::opencode::parse_export,
};

#[test]
fn qualified_119_export_preserves_producer_version() {
    let bytes = include_bytes!("fixtures/opencode/completed-1.19.0.json");
    let transcript = parse_export(bytes).expect("qualified real export");
    assert_eq!(transcript.agent_version.as_deref(), Some("1.19.0"));
    assert!(!transcript.events.is_empty());
}

#[test]
fn opencode_saved_api_account_is_supported_but_subscription_is_refused() {
    let tool: ToolId = serde_json::from_str("\"opencode\"").expect("OpenCode tool");
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    assert!(accounts.add(tool, ConnectionKind::ApiKey, "Work").is_ok());
    assert!(
        accounts
            .add(tool, ConnectionKind::NearAi, "Private")
            .is_ok()
    );
    assert!(
        accounts
            .add(tool, ConnectionKind::Subscription, "Unsupported")
            .is_err()
    );
}

#[test]
fn launch_requires_an_explicit_model_and_preserves_collection_choice() {
    use serde_json::json;
    use trace_commons_contributor::managed::sessions::{LaunchRequest, SessionRegistry};
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Opencode, ConnectionKind::ApiKey, "Work")
        .unwrap();
    let mut sessions = SessionRegistry::open(&store).unwrap();
    let mut value = json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":account.id,"cwd":tmp.path(),"expected_generation":0});
    assert!(
        sessions
            .prepare(serde_json::from_value(value.clone()).unwrap(), &account)
            .is_err()
    );
    value["model"] = json!("gpt-5");
    value["collect_traces"] = json!(true);
    let request: LaunchRequest = serde_json::from_value(value).unwrap();
    let prepared = sessions.prepare(request, &account).unwrap();
    let context = sessions
        .redeem(prepared.session_id, &prepared.ticket.unwrap())
        .unwrap();
    assert_eq!(context.session.model.as_deref(), Some("gpt-5"));
    assert!(context.session.collect_traces);
}

#[cfg(unix)]
#[test]
fn isolated_launch_exports_only_its_owned_session_and_stages_validated_bytes() {
    use std::os::unix::fs::PermissionsExt;
    use trace_commons_contributor::managed::opencode::OpenCodeSession;
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Opencode, ConnectionKind::ApiKey, "Work")
        .unwrap();
    let native = tmp.path().join("opencode");
    // The fixture importer preserves the seed, and exporter must receive its exact ID.
    std::fs::write(&native, r#"#!/bin/sh
if [ "$1" = --version ]; then echo '1.19.0'; exit 0; fi
shift
case "$1" in
import) cp "$2" "$HOME/fixture-seed.json";;
export) python3 -c 'import json,os,sys; v=json.load(open(os.environ["HOME"]+"/fixture-seed.json")); assert v["info"]["id"] == sys.argv[1]; print(json.dumps(v))' "$2";;
esac
"#).unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let run =
        OpenCodeSession::new(&store, &account, &native, uuid::Uuid::new_v4(), tmp.path()).unwrap();
    run.prepare().unwrap();
    let command = run
        .command("gpt-5", "https://api.openai.com/v1", "synthetic-key")
        .unwrap();
    let env: std::collections::BTreeMap<_, _> = command.get_envs().collect();
    assert!(
        env[std::ffi::OsStr::new("HOME")]
            .unwrap()
            .to_str()
            .unwrap()
            .contains(&account.id.to_string())
    );
    assert_eq!(
        env[std::ffi::OsStr::new("OPENCODE_DISABLE_PROJECT_CONFIG")].unwrap(),
        "true"
    );
    let config: serde_json::Value = serde_json::from_str(
        env[std::ffi::OsStr::new("OPENCODE_CONFIG_CONTENT")]
            .unwrap()
            .to_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(config["model"], "trace-managed/gpt-5");
    assert!(!config.to_string().contains("synthetic-key"));
    let args: Vec<_> = command.get_args().collect();
    assert!(args.iter().any(|a| *a == "--session"));
    run.collect(&store).unwrap();
    let settings = trace_commons_contributor::daemon::settings::DaemonSettings::default();
    let sources = trace_commons_contributor::source::all_sources(&settings.source_roots(&store));
    let source = sources.iter().find(|s| s.name() == "opencode").unwrap();
    let refs = source.discover().unwrap();
    assert_eq!(refs.len(), 1);
    assert_eq!(
        source.load(&refs[0]).unwrap().agent_version.as_deref(),
        Some("1.19.0")
    );
    // A wrong-session export may never replace an already staged trace.
    std::fs::write(
        &native,
        "#!/bin/sh\nprintf '%s' '{\"info\":{\"id\":\"ses_other\"},\"messages\":[]}'\n",
    )
    .unwrap();
    assert!(run.collect(&store).is_err());
    assert_eq!(source.discover().unwrap().len(), 1);
}

/// A resumed OpenCode session exports its root's whole conversation
/// (`ses_<root>`) and stages it as the resumed row's own file, leaving the
/// root row's earlier export in place.
#[cfg(unix)]
#[test]
fn resumed_session_stages_under_its_own_row_without_replacing_the_roots_export() {
    use std::os::unix::fs::PermissionsExt;
    use trace_commons_contributor::managed::opencode::{OpenCodeSession, STAGING};
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let account = AccountStore::open(&store)
        .unwrap()
        .add(ToolId::Opencode, ConnectionKind::ApiKey, "Work")
        .unwrap();
    let native = tmp.path().join("opencode");
    std::fs::write(&native, r#"#!/bin/sh
if [ "$1" = --version ]; then echo '1.19.0'; exit 0; fi
shift
case "$1" in
import) cp "$2" "$HOME/fixture-seed.json";;
export) python3 -c 'import json,os,sys; v=json.load(open(os.environ["HOME"]+"/fixture-seed.json")); assert v["info"]["id"] == sys.argv[1]; print(json.dumps(v))' "$2";;
esac
"#).unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let root = uuid::Uuid::new_v4();
    let first = OpenCodeSession::new(&store, &account, &native, root, tmp.path()).unwrap();
    first.prepare().unwrap();
    first.collect(&store).unwrap();
    let staged = store.dir().join(STAGING);
    let earlier = std::fs::read(staged.join(format!("{root}.json"))).unwrap();

    let row = uuid::Uuid::new_v4();
    let resumed = OpenCodeSession::new(&store, &account, &native, root, tmp.path()).unwrap();
    resumed.collect_as(&store, row).unwrap();
    assert_eq!(
        std::fs::read(staged.join(format!("{root}.json"))).unwrap(),
        earlier,
        "the root's export is not replaced"
    );
    assert!(staged.join(format!("{row}.json")).is_file());
    let settings = trace_commons_contributor::daemon::settings::DaemonSettings::default();
    let sources = trace_commons_contributor::source::all_sources(&settings.source_roots(&store));
    let source = sources.iter().find(|s| s.name() == "opencode").unwrap();
    let refs = source.discover().unwrap();
    assert_eq!(refs.len(), 2);
    for reference in refs {
        assert_eq!(
            source.load(&reference).unwrap().conversation_id,
            Some(format!("ses_{}", root.simple()))
        );
    }
}

/// Optional installed-binary qualification. The executable must be a wrapper
/// that denies external network, e.g. sandbox-exec; never uses real credentials.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TC_OPENCODE_TEST_BIN pointing to an externally network-denied OpenCode 1.x or 2.x"]
async fn installed_opencode_round_trip_uses_selected_model_and_exact_session() {
    use axum::{Json, Router, routing::post};
    use std::sync::{Arc, Mutex};
    use trace_commons_contributor::managed::opencode::OpenCodeSession;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let observations = seen.clone();
    let app = Router::new().route("/v1/chat/completions", post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
        let observations = observations.clone();
        async move {
            observations.lock().unwrap().push((
                body["model"].as_str().unwrap().to_owned(),
                headers.get("x-ironwire-session-id").and_then(|v| v.to_str().ok()).map(str::to_owned),
            ));
            let a = serde_json::json!({"id":"fixture","object":"chat.completion.chunk","created":0,"model":"fixture-model","choices":[{"index":0,"delta":{"role":"assistant","content":"Synthetic."},"finish_reason":null}]});
            let b = serde_json::json!({"id":"fixture","object":"chat.completion.chunk","created":0,"model":"fixture-model","choices":[{"index":0,"delta":{},"finish_reason":"stop"}]});
            ([("content-type", "text/event-stream")], format!("data: {a}\n\ndata: {b}\n\ndata: [DONE]\n\n"))
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let mut accounts = AccountStore::open(&store).unwrap();
    let account = accounts
        .add(ToolId::Opencode, ConnectionKind::ApiKey, "Synthetic")
        .unwrap();
    let native = std::path::PathBuf::from(
        std::env::var_os("TC_OPENCODE_TEST_BIN").expect("network-denied executable wrapper"),
    );
    let run =
        OpenCodeSession::new(&store, &account, &native, uuid::Uuid::new_v4(), tmp.path()).unwrap();
    run.prepare()
        .expect("real native import and exact-ID export");
    let mut command = run
        .command(
            "fixture-model",
            &format!("http://127.0.0.1:{port}/v1"),
            "synthetic-key",
        )
        .unwrap();
    if std::env::var("TC_OPENCODE_TEST_MAJOR").as_deref() == Ok("2") {
        // v2 scopes standalone/session flags to the subcommand. The product
        // command is interactive; qualify its exact configuration via `run`.
        let mut run = std::process::Command::new(command.get_program());
        run.current_dir(command.get_current_dir().unwrap());
        for (key, value) in command.get_envs() {
            match value {
                Some(value) => {
                    run.env(key, value);
                }
                None => {
                    run.env_remove(key);
                }
            }
        }
        run.arg("run").args(command.get_args());
        command = run;
    } else {
        command.arg("run");
    }
    command.args(["--format", "json", "Return the synthetic word."]);
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tokio::process::Command::from(command)
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(
        output.status.success(),
        "native run failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!seen.lock().unwrap().is_empty());
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(model, _)| model == "fixture-model")
    );
    run.collect(&store)
        .expect("real native export validates and stages");
    let sources = trace_commons_contributor::source::all_sources(
        &trace_commons_contributor::daemon::settings::DaemonSettings::default()
            .source_roots(&store),
    );
    let source = sources.iter().find(|s| s.name() == "opencode").unwrap();
    let refs = source.discover().unwrap();
    assert_eq!(refs.len(), 1);
    let transcript = source.load(&refs[0]).unwrap();
    assert!(transcript.events.len() >= 2);
    assert!(
        seen.lock()
            .unwrap()
            .iter()
            .all(|(_, session)| session.as_deref() == transcript.conversation_id.as_deref())
    );
    match std::env::var("TC_OPENCODE_TEST_MAJOR").as_deref() {
        Ok("2") => assert_eq!(transcript.agent_version, None),
        _ => assert_eq!(transcript.agent_version.as_deref(), Some("1.19.0")),
    }
    server.abort();
}

#[test]
fn declared_and_managed_exports_share_one_source_and_both_load() {
    use trace_commons_contributor::{
        daemon::settings::{DaemonSettings, SourceDeclaration},
        source::all_sources,
    };
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let declared = tmp.path().join("ordinary");
    let managed = store
        .dir()
        .join(trace_commons_contributor::managed::opencode::STAGING);
    std::fs::create_dir(&declared).unwrap();
    std::fs::create_dir(&managed).unwrap();
    for root in [&declared, &managed] {
        std::fs::write(
            root.join("session.json"),
            include_bytes!("fixtures/opencode/completed-1.19.0.json"),
        )
        .unwrap();
    }
    let settings = DaemonSettings {
        opencode_source: Some(SourceDeclaration::Watch { path: declared }),
        ..Default::default()
    };
    let sources = all_sources(&settings.source_roots(&store));
    let sources: Vec<_> = sources.iter().filter(|s| s.name() == "opencode").collect();
    assert_eq!(sources.len(), 1);
    let refs = sources[0].discover().unwrap();
    assert_eq!(refs.len(), 2);
    for reference in refs {
        assert!(sources[0].load(&reference).is_ok());
        assert!(sources[0].session_at(&reference.path).unwrap().is_some());
    }
}

#[test]
fn automatic_opencode_off_preserves_only_explicitly_collected_exports() {
    use trace_commons_contributor::{
        daemon::settings::{DaemonSettings, SourceDeclaration},
        source::all_sources,
    };
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let root = store
        .dir()
        .join(trace_commons_contributor::managed::opencode::STAGING);
    let ordinary = tmp.path().join("ordinary-opencode");
    for folder in [&root, &ordinary] {
        std::fs::create_dir(folder).unwrap();
        std::fs::write(
            folder.join("session.json"),
            include_bytes!("fixtures/opencode/completed-1.19.0.json"),
        )
        .unwrap();
    }
    let mut settings = DaemonSettings {
        opencode_source: Some(SourceDeclaration::Watch {
            path: ordinary.clone(),
        }),
        ..Default::default()
    };
    let watching = all_sources(&settings.source_roots(&store));
    assert_eq!(
        watching
            .iter()
            .find(|source| source.name() == "opencode")
            .unwrap()
            .discover()
            .unwrap()
            .len(),
        2
    );

    settings.opencode_source = Some(SourceDeclaration::Off);
    let roots = settings.source_roots(&store);
    let identity = &roots.source_identities()["opencode"];
    assert!(identity.contains(&root.to_string_lossy().to_string()));
    assert!(!identity.contains(&ordinary.to_string_lossy().to_string()));
    let sources = all_sources(&roots);
    let source = sources
        .iter()
        .find(|source| source.name() == "opencode")
        .unwrap();
    let found = source.discover().unwrap();
    assert_eq!(found.len(), 1, "automatic discovery must remain off");
    assert!(found[0].path.starts_with(&root));
    assert!(
        source.load(&found[0]).is_ok(),
        "explicitly collected bytes remain reviewable"
    );
    assert!(
        source
            .session_at(&ordinary.join("session.json"))
            .unwrap()
            .is_none()
    );
}

#[test]
fn restart_before_ticket_redemption_settles_collection_as_failed() {
    use trace_commons_contributor::managed::sessions::{
        CollectionState, SessionRegistry, SessionState,
    };
    let tmp = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(tmp.path().join("state")).unwrap();
    let account = AccountStore::open(&store)
        .unwrap()
        .add(ToolId::Opencode, ConnectionKind::ApiKey, "Work")
        .unwrap();
    let mut registry = SessionRegistry::open(&store).unwrap();
    registry.prepare(serde_json::from_value(serde_json::json!({"request_id":uuid::Uuid::new_v4(),"tool":"opencode","connection":"api_key","account_id":account.id,"cwd":tmp.path(),"expected_generation":0,"model":"fixture-model","collect_traces":true})).unwrap(), &account).unwrap();
    drop(registry);
    let views = SessionRegistry::open(&store).unwrap().views().unwrap();
    assert_eq!(views[0].state, SessionState::Failed);
    assert_eq!(views[0].collection, CollectionState::Failed);
}
