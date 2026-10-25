use chrono::{Duration, Utc};
use std::sync::Arc;
use trace_commons_contributor::{
    config::ConfigStore,
    daemon::{
        ipc::DaemonShared,
        queue::QueueState,
        settings::{DaemonSettings, SourceDeclaration},
        watcher,
    },
    managed::{AccountId, ToolId, native_collection::NativeCollection, profiles::NativeProfiles},
    source::{SOURCE_CLAUDE_CODE, SOURCE_CODEX, SOURCE_OPENCODE, all_sources},
};
use uuid::Uuid;

fn claude(root: &std::path::Path, id: Uuid) -> std::path::PathBuf {
    let path = root
        .join("projects/-Users-test-project")
        .join(format!("{id}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!(r#"{{"type":"user","message":{{"role":"user","content":"Please fix the test"}},"cwd":"/Users/test/project","timestamp":"2026-08-08T10:00:00Z","version":"2.0.1","sessionId":"{id}","uuid":"u1"}}
"#)).unwrap();
    path
}

#[tokio::test]
async fn opted_in_native_trace_reaches_review_even_when_automatic_discovery_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let old = claude(&root, Uuid::new_v4());
    let capture =
        NativeCollection::snapshot(&store, ToolId::Claude, account, Uuid::new_v4()).unwrap();
    let current = claude(&root, Uuid::new_v4());
    capture.collect(&store).unwrap();
    assert!(old.exists());
    assert!(current.exists());
    let settings = DaemonSettings {
        claude_source: Some(SourceDeclaration::Off),
        codex_source: Some(SourceDeclaration::Off),
        opencode_source: Some(SourceDeclaration::Off),
        ..Default::default()
    };
    settings.save(&store).unwrap();
    let sources = all_sources(&settings.source_roots(&store));
    let refs = sources
        .iter()
        .find(|source| source.name() == SOURCE_CLAUDE_CODE)
        .unwrap()
        .discover()
        .unwrap();
    assert_eq!(
        refs.len(),
        1,
        "old opted-out transcript must not be swept into a later opt-in"
    );
    assert_ne!(refs[0].path, current);
    assert_eq!(refs[0].path.file_name(), current.file_name());
    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let now = Utc::now() + Duration::hours(2);
    watcher::tick(&shared, now).await.unwrap();
    watcher::tick(&shared, now).await.unwrap();
    let queue = shared.queue.lock().unwrap();
    assert_eq!(queue.all().len(), 1);
    assert_eq!(queue.all()[0].source, SOURCE_CLAUDE_CODE);
    assert_eq!(queue.all()[0].state, QueueState::Pending);
}

#[test]
fn private_native_profiles_are_not_passively_discovered_without_collection() {
    use trace_commons_contributor::managed::{ConnectionKind, accounts::AccountStore};
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountStore::open(&store)
        .unwrap()
        .add(ToolId::Claude, ConnectionKind::Subscription, "Fixture")
        .unwrap();
    let root = NativeProfiles::new(&store)
        .profile_root(account.id)
        .unwrap();
    claude(&root, Uuid::new_v4());
    let settings = DaemonSettings {
        claude_source: Some(SourceDeclaration::Watch {
            path: dir.path().join("ordinary-claude"),
        }),
        codex_source: Some(SourceDeclaration::Off),
        ..Default::default()
    };
    let sources = all_sources(&settings.source_roots(&store));
    let found = sources
        .iter()
        .find(|source| source.name() == SOURCE_CLAUDE_CODE)
        .unwrap()
        .discover()
        .unwrap();
    assert!(
        found.is_empty(),
        "watching normal Claude logs must not sweep in unchecked managed profiles"
    );
}

#[tokio::test]
async fn explicitly_staged_opencode_export_is_reviewed_with_automatic_discovery_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let root = store
        .dir()
        .join(trace_commons_contributor::managed::opencode::STAGING);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("managed.json"),
        include_bytes!("fixtures/opencode/completed-1.19.0.json"),
    )
    .unwrap();
    let settings = DaemonSettings {
        claude_source: Some(SourceDeclaration::Off),
        codex_source: Some(SourceDeclaration::Off),
        opencode_source: Some(SourceDeclaration::Off),
        ..Default::default()
    };
    settings.save(&store).unwrap();
    let shared = Arc::new(DaemonShared::load(store).unwrap());
    let now = Utc::now() + Duration::hours(2);
    watcher::tick(&shared, now).await.unwrap();
    watcher::tick(&shared, now).await.unwrap();
    let queue = shared.queue.lock().unwrap();
    assert_eq!(queue.all().len(), 1);
    assert_eq!(queue.all()[0].source, SOURCE_OPENCODE);
    assert_eq!(queue.all()[0].state, QueueState::Pending);
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_native_launch_collects_only_when_the_launch_checkbox_is_on() {
    use serde_json::json;
    use std::{
        os::unix::fs::PermissionsExt,
        process::{Command, Stdio},
        time::{Duration as StdDuration, Instant},
    };
    use trace_commons_contributor::daemon::{
        ipc::{Request, handle_request},
        start_embedded,
    };
    for tool in ["claude", "codex"] {
        for collect in [false, true] {
            let home = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(home.path().join("state")).unwrap();
            DaemonSettings {
                claude_source: Some(SourceDeclaration::Off),
                codex_source: Some(SourceDeclaration::Off),
                opencode_source: Some(SourceDeclaration::Off),
                ..Default::default()
            }
            .save(&store)
            .unwrap();
            let daemon = start_embedded(store).await.unwrap();
            let response = handle_request(
                &daemon.shared,
                &Request {
                    id: 1,
                    method: "managed_account_add".into(),
                    params: json!({"tool":tool,"connection":"subscription","label":"Fixture"}),
                },
            );
            assert!(response.error.is_none());
            let bin = home.path().join("bin");
            std::fs::create_dir(&bin).unwrap();
            let project = home.path().join("project");
            std::fs::create_dir(&project).unwrap();
            let fixture = home.path().join("native-transcript");
            let (script, target) = if tool == "claude" {
                let tmp = home.path().join("fixture-root");
                let src = claude(&tmp, Uuid::new_v4());
                std::fs::copy(src, &fixture).unwrap();
                (
                    "#!/bin/sh\ncase \"$1\" in\n--version) echo '2.1.289 (Claude Code)'; exit 0;;\nauth) echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0;;\nesac\nmkdir -p \"$CLAUDE_CONFIG_DIR/projects/project\"\ncp \"$FIXTURE_TRACE\" \"$CLAUDE_CONFIG_DIR/projects/project/new-session.jsonl\"\n",
                    "new-session.jsonl",
                )
            } else {
                std::fs::copy("fixtures/codex/2026/07/01/rollout-2026-07-01T10-00-00-22222222-2222-2222-2222-222222222222.jsonl",&fixture).unwrap();
                (
                    "#!/bin/sh\ncase \"$1\" in\n--version) echo 'codex-cli 0.160.0'; exit 0;;\nlogin) echo 'Logged in using ChatGPT'; exit 0;;\nesac\nmkdir -p \"$CODEX_HOME/sessions/day\"\ncp \"$FIXTURE_TRACE\" \"$CODEX_HOME/sessions/day/rollout-new-session.jsonl\"\n",
                    "rollout-new-session.jsonl",
                )
            };
            let native = bin.join(tool);
            std::fs::write(&native, script).unwrap();
            std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
            let mut command = Command::new("python3");
            command.args(["-c","import os,pty,sys; sys.exit(os.waitstatus_to_exitcode(pty.spawn(sys.argv[1:])))",env!("CARGO_BIN_EXE_near-ai")]).arg("--config-dir").arg(home.path().join("state")).args(["launch",tool,"--account","Fixture","--cwd"]).arg(&project).arg(if collect {"--collect-traces"} else {"--no-collect-traces"}).env("PATH",format!("{}:{}",bin.display(),std::env::var("PATH").unwrap())).env("FIXTURE_TRACE",&fixture).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            let mut child = command.spawn().unwrap();
            let deadline = Instant::now() + StdDuration::from_secs(15);
            let status = loop {
                if let Some(status) = child.try_wait().unwrap() {
                    break status;
                }
                assert!(Instant::now() < deadline, "native launch did not exit");
                tokio::time::sleep(StdDuration::from_millis(20)).await;
            };
            assert!(status.success(), "{tool}: opted-in={collect}");
            let snapshot = handle_request(
                &daemon.shared,
                &Request {
                    id: 2,
                    method: "managed_snapshot".into(),
                    params: json!({}),
                },
            )
            .result
            .unwrap();
            assert_eq!(
                snapshot["sessions"][0]["collection"],
                if collect { "collected" } else { "off" },
                "{tool}"
            );
            let sources = all_sources(
                &daemon
                    .shared
                    .settings
                    .lock()
                    .unwrap()
                    .source_roots(&daemon.shared.store),
            );
            let found = sources
                .iter()
                .filter(|source| {
                    source.name()
                        == if tool == "claude" {
                            SOURCE_CLAUDE_CODE
                        } else {
                            SOURCE_CODEX
                        }
                })
                .flat_map(|source| source.discover().unwrap())
                .collect::<Vec<_>>();
            assert_eq!(
                found.len(),
                usize::from(collect),
                "{tool}: unchecked logs must remain private"
            );
            if collect {
                assert_eq!(found[0].path.file_name().unwrap(), target);
            }
            daemon.close();
        }
    }
}
