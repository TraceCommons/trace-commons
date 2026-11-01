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
    let managed_id = Uuid::new_v4();
    let capture =
        NativeCollection::snapshot(&store, ToolId::Claude, account, managed_id, None).unwrap();
    let current = claude(&root, managed_id);
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
    for (tool, collect, missing_receipt) in [
        ("claude", false, false),
        ("claude", true, false),
        ("codex", false, false),
        ("codex", true, false),
        ("codex", true, true),
    ] {
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
        let native_id = Uuid::new_v4();
        if tool == "claude" {
            let tmp = home.path().join("fixture-root");
            let src = claude(&tmp, native_id);
            std::fs::copy(src, &fixture).unwrap();
        } else {
            std::fs::write(&fixture, include_str!("../fixtures/codex/2026/07/01/rollout-2026-07-01T10-00-00-22222222-2222-2222-2222-222222222222.jsonl")).unwrap();
        }
        let script = r##"#!/usr/bin/env python3
import json, os, pathlib, subprocess, sys, tomllib, uuid
args = sys.argv[1:]
tool = pathlib.Path(sys.argv[0]).name
if args and args[0] == '--version':
    print('2.1.289 (Claude Code)' if tool == 'claude' else 'codex-cli 0.164.0')
    sys.exit(0)
if args and args[0] == 'auth':
    print('{"loggedIn":true,"authMethod":"claude.ai"}')
    sys.exit(0)
if args and args[0] == 'login':
    print('Logged in using ChatGPT')
    sys.exit(0)
native_id = os.environ['FIXTURE_NATIVE_ID']
if tool == 'claude' and '--session-id' in args:
    native_id = args[args.index('--session-id') + 1]
root = pathlib.Path(os.environ['CLAUDE_CONFIG_DIR' if tool == 'claude' else 'CODEX_HOME'])
path = (root / 'projects' / 'project' / (native_id + '.jsonl')) if tool == 'claude' else (root / 'sessions' / 'day' / ('rollout-2026-07-01T10-00-00-' + native_id + '.jsonl'))
path.parent.mkdir(parents=True, exist_ok=True)
records = [json.loads(line) for line in pathlib.Path(os.environ['FIXTURE_TRACE']).read_text().splitlines() if line]
for record in records:
    if tool == 'claude':
        record['sessionId'] = native_id
        record['cwd'] = os.getcwd()
    elif record.get('type') == 'session_meta':
        record['payload']['id'] = native_id
        record['payload']['cwd'] = os.getcwd()
path.write_text(''.join(json.dumps(record) + '\n' for record in records))
if tool == 'codex' and os.environ.get('FIXTURE_MISSING_RECEIPT') != '1':
    for arg in args:
        if arg.startswith('hooks.SessionStart='):
            hooks = tomllib.loads(arg)['hooks']['SessionStart']
            for group in hooks:
                for hook in group['hooks']:
                    payload = json.dumps({'session_id':native_id,'transcript_path':str(path.resolve()),'hook_event_name':'SessionStart','source':'startup'}).encode()
                    subprocess.run(hook['command'], shell=True, input=payload, check=False)
"##;
        let native = bin.join(tool);
        std::fs::write(&native, script).unwrap();
        std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
        let mut command = Command::new("python3");
        command
            .args([
                "-c",
                "import os,pty,sys; sys.exit(os.waitstatus_to_exitcode(pty.spawn(sys.argv[1:])))",
                env!("CARGO_BIN_EXE_near-ai"),
            ])
            .arg("--config-dir")
            .arg(home.path().join("state"))
            .args(["launch", tool, "--account", "Fixture", "--cwd"])
            .arg(&project)
            .arg(if collect {
                "--collect-traces"
            } else {
                "--no-collect-traces"
            })
            .env(
                "PATH",
                format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
            )
            .env("FIXTURE_TRACE", &fixture)
            .env("FIXTURE_NATIVE_ID", native_id.to_string())
            .env(
                "FIXTURE_MISSING_RECEIPT",
                if missing_receipt { "1" } else { "0" },
            )
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
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
            if !collect {
                "off"
            } else if missing_receipt {
                "failed"
            } else {
                "collected"
            },
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
            usize::from(collect && !missing_receipt),
            "{tool}: unchecked logs must remain private"
        );
        if collect && !missing_receipt {
            let expected = if tool == "claude" {
                format!("{}.jsonl", snapshot["sessions"][0]["id"].as_str().unwrap())
            } else {
                format!("rollout-2026-07-01T10-00-00-{native_id}.jsonl")
            };
            assert_eq!(
                found[0].path.file_name().unwrap().to_str().unwrap(),
                expected
            );
        }
        daemon.close();
    }
}

fn staged_ids(store: &ConfigStore, source_name: &str) -> Vec<String> {
    let settings = DaemonSettings {
        claude_source: Some(SourceDeclaration::Off),
        codex_source: Some(SourceDeclaration::Off),
        opencode_source: Some(SourceDeclaration::Off),
        ..Default::default()
    };
    let mut ids = Vec::new();
    for source in all_sources(&settings.source_roots(store))
        .iter()
        .filter(|source| source.name() == source_name)
    {
        for session in source.discover().unwrap() {
            let transcript = source.load(&session).unwrap();
            ids.push(
                transcript
                    .source_session
                    .expect("native identity preserved")
                    .native_id,
            );
        }
    }
    ids.sort();
    ids
}

#[test]
fn overlapping_claude_collections_keep_exact_ids_and_exclude_foreign_optout_in_same_cwd() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let first_id = Uuid::new_v4();
    let second_id = Uuid::new_v4();
    let foreign_id = Uuid::new_v4();
    let first =
        NativeCollection::snapshot(&store, ToolId::Claude, account, first_id, None).unwrap();
    let second =
        NativeCollection::snapshot(&store, ToolId::Claude, account, second_id, None).unwrap();
    let foreign = claude(&root, foreign_id);
    let a = claude(&root, first_id);
    let b = claude(&root, second_id);
    first.collect(&store).unwrap();
    assert_eq!(
        staged_ids(&store, SOURCE_CLAUDE_CODE),
        vec![first_id.to_string()]
    );
    second.collect(&store).unwrap();
    let mut expected = vec![first_id.to_string(), second_id.to_string()];
    expected.sort();
    assert_eq!(staged_ids(&store, SOURCE_CLAUDE_CODE), expected);
    assert!(foreign.exists() && a.exists() && b.exists());
}

#[test]
fn claude_filename_and_parent_metadata_must_match_the_assigned_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let id = Uuid::new_v4();
    let capture = NativeCollection::snapshot(&store, ToolId::Claude, account, id, None).unwrap();
    let unrelated = claude(&root, Uuid::new_v4());
    let assigned = unrelated.with_file_name(format!("{id}.jsonl"));
    std::fs::rename(unrelated, &assigned).unwrap();
    assert!(capture.collect(&store).is_err());
    assert!(assigned.exists());
    assert!(staged_ids(&store, SOURCE_CLAUDE_CODE).is_empty());
}

fn codex(root: &std::path::Path, filename_id: Uuid, metadata_id: Uuid) -> std::path::PathBuf {
    let path = root
        .join("sessions/day")
        .join(format!("rollout-2026-07-01T10-00-00-{filename_id}.jsonl"));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let bytes=include_str!("../fixtures/codex/2026/07/01/rollout-2026-07-01T10-00-00-22222222-2222-2222-2222-222222222222.jsonl").replace("22222222-2222-2222-2222-222222222222",&metadata_id.to_string());
    std::fs::write(&path, bytes).unwrap();
    std::fs::canonicalize(path).unwrap()
}

#[cfg(unix)]
fn record_codex_start(capture: &mut NativeCollection, native_id: Uuid, path: &std::path::Path) {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let mut command = Command::new("codex");
    capture.configure_codex(&mut command).unwrap();
    let config = command
        .get_args()
        .find(|arg| arg.to_string_lossy().starts_with("hooks.SessionStart="))
        .expect("exact vetted startup hook")
        .to_string_lossy()
        .into_owned();
    let config: toml::Table = config.parse().unwrap();
    let collector = config["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let argv =
        trace_commons_contributor::managed::options::parse(ToolId::Claude, collector).unwrap();
    assert_eq!(argv[1], "managed-session-bind");
    // configure runs inside this test harness; retain all generated hook
    // arguments and replace only its executable with the real CLI helper.
    let mut child = Command::new(env!("CARGO_BIN_EXE_near-ai"))
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(serde_json::to_vec(&serde_json::json!({"session_id":native_id,"transcript_path":path,"hook_event_name":"SessionStart","source":"startup"})).unwrap().as_slice()).unwrap();
    assert!(child.wait().unwrap().success());
}

#[cfg(unix)]
#[test]
fn overlapping_codex_collections_use_start_receipts_and_exclude_foreign_optout() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let a_id = Uuid::new_v4();
    let b_id = Uuid::new_v4();
    let foreign_id = Uuid::new_v4();
    let mut a = codex_capture(&store, account);
    let mut b = codex_capture(&store, account);
    let foreign = codex(&root, foreign_id, foreign_id);
    let first = codex(&root, a_id, a_id);
    let second = codex(&root, b_id, b_id);
    record_codex_start(&mut a, a_id, &first);
    record_codex_start(&mut b, b_id, &second);
    a.collect(&store).unwrap();
    assert_eq!(staged_ids(&store, SOURCE_CODEX), vec![a_id.to_string()]);
    b.collect(&store).unwrap();
    let mut expected = vec![a_id.to_string(), b_id.to_string()];
    expected.sort();
    assert_eq!(staged_ids(&store, SOURCE_CODEX), expected);
    assert!(foreign.exists());
}

#[cfg(unix)]
#[test]
fn codex_missing_preexisting_and_mismatched_bindings_fail_collection_without_sweeping_logs() {
    for mode in [
        "missing",
        "preexisting",
        "receipt_id",
        "filename_id",
        "metadata_id",
        "outside_profile",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("state")).unwrap();
        let account = AccountId(Uuid::new_v4());
        let root = NativeProfiles::new(&store).profile_root(account).unwrap();
        let native_id = Uuid::new_v4();
        let other = Uuid::new_v4();
        let preexisting = if mode == "preexisting" {
            Some(codex(&root, native_id, native_id))
        } else {
            None
        };
        let mut capture = codex_capture(&store, account);
        let path = preexisting.unwrap_or_else(|| {
            let target = if mode == "outside_profile" {
                dir.path().join("outside")
            } else {
                root.clone()
            };
            codex(
                &target,
                if mode == "filename_id" {
                    other
                } else {
                    native_id
                },
                if mode == "metadata_id" {
                    other
                } else {
                    native_id
                },
            )
        });
        if mode != "missing" {
            record_codex_start(
                &mut capture,
                if mode == "receipt_id" {
                    other
                } else {
                    native_id
                },
                &path,
            );
        }
        assert!(capture.collect(&store).is_err(), "{mode}");
        assert!(path.exists());
        assert!(staged_ids(&store, SOURCE_CODEX).is_empty(), "{mode}");
    }
}

#[cfg(not(unix))]
#[test]
fn unsupported_codex_binding_refuses_collection_and_preserves_launch_arguments() {
    use std::process::Command;
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let mut capture = codex_capture(&store, account);
    let native_id = Uuid::new_v4();
    let transcript = codex(&root, native_id, native_id);
    let mut command = Command::new("codex");
    command
        .args(["exec", "synthetic prompt"])
        .env("CODEX_HOME", &root);
    let before: Vec<_> = command.get_args().map(|a| a.to_owned()).collect();
    assert!(capture.configure_codex(&mut command).is_err());
    assert_eq!(
        command.get_args().collect::<Vec<_>>(),
        before.iter().collect::<Vec<_>>()
    );
    assert!(
        command
            .get_envs()
            .any(|(key, value)| key == "CODEX_HOME" && value == Some(root.as_os_str()))
    );
    assert!(capture.collect(&store).is_err());
    assert!(transcript.exists());
    assert!(staged_ids(&store, SOURCE_CODEX).is_empty());
}

/// A Codex collection holding the binding a launcher prepares for every
/// Codex launch.
fn codex_capture(store: &ConfigStore, account: AccountId) -> NativeCollection {
    let id = Uuid::new_v4();
    NativeCollection::snapshot(
        store,
        ToolId::Codex,
        account,
        id,
        trace_commons_contributor::managed::session_binding::prepare(store, id).ok(),
    )
    .unwrap()
}

/// The staged copy a resumed row `row` published for Claude conversation
/// `native`, if any.
fn staged_claude(store: &ConfigStore, row: Uuid, native: Uuid) -> Option<std::path::PathBuf> {
    let path = store
        .dir()
        .join(trace_commons_contributor::managed::native_collection::STAGING)
        .join("claude")
        .join(row.to_string())
        .join("-Users-test-project")
        .join(format!("{native}.jsonl"));
    path.is_file().then_some(path)
}

/// Append one more Claude turn, recorded under `native`.
fn claude_turn(path: &std::path::Path, native: Uuid) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, r#"{{"type":"user","message":{{"role":"user","content":"Now run the suite"}},"cwd":"/Users/test/project","timestamp":"2026-08-08T11:00:00Z","version":"2.0.1","sessionId":"{native}","uuid":"u2"}}"#).unwrap();
}

/// A resumed Claude row keeps appending to its root's transcript. It stages
/// that one pre-existing file, whole and as it is now, under its own row
/// id, so the root's earlier staged copy is untouched. Every other file
/// that was there before the launch stays private.
#[test]
fn resumed_claude_row_stages_the_continued_root_transcript_under_its_own_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let root_id = Uuid::new_v4();
    let first = NativeCollection::snapshot(&store, ToolId::Claude, account, root_id, None).unwrap();
    let transcript = claude(&root, root_id);
    first.collect(&store).unwrap();
    let earlier = std::fs::read(staged_claude(&store, root_id, root_id).unwrap()).unwrap();

    let foreign = claude(&root, Uuid::new_v4());
    let resumed = Uuid::new_v4();
    let capture = NativeCollection::snapshot(&store, ToolId::Claude, account, resumed, None)
        .unwrap()
        .continuing(root_id);
    claude_turn(&transcript, root_id);
    capture.collect(&store).unwrap();

    let staged = staged_claude(&store, resumed, root_id).expect("staged under the new row");
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        std::fs::read(&transcript).unwrap()
    );
    assert_eq!(
        std::fs::read(staged_claude(&store, root_id, root_id).unwrap()).unwrap(),
        earlier,
        "the root's staged copy is not overwritten"
    );
    assert_eq!(
        staged_ids(&store, SOURCE_CLAUDE_CODE),
        vec![root_id.to_string(), root_id.to_string()],
        "both rows carry the root's identity; nothing foreign is staged"
    );
    assert!(foreign.exists() && transcript.exists());
}

/// A resumed Claude row whose root transcript is gone at exit fails
/// collection and stages nothing; the local profile is left as it is.
#[test]
fn resumed_claude_row_without_its_root_transcript_fails_collection() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let root_id = Uuid::new_v4();
    let transcript = claude(&root, root_id);
    let foreign = claude(&root, Uuid::new_v4());
    let resumed = Uuid::new_v4();
    let capture = NativeCollection::snapshot(&store, ToolId::Claude, account, resumed, None)
        .unwrap()
        .continuing(root_id);
    std::fs::remove_file(&transcript).unwrap();
    assert!(capture.collect(&store).is_err());
    assert!(foreign.exists());
    assert!(staged_ids(&store, SOURCE_CLAUDE_CODE).is_empty());
    assert!(
        trace_commons_contributor::managed::native_collection::roots(&store, ToolId::Claude)
            .is_empty()
    );
}

/// Without the root's own launch, a fresh row still refuses a transcript
/// that existed before it started: only a resumed row may take its root's.
#[test]
fn a_fresh_row_still_refuses_a_preexisting_transcript() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let id = Uuid::new_v4();
    let transcript = claude(&root, id);
    let capture = NativeCollection::snapshot(&store, ToolId::Claude, account, id, None).unwrap();
    claude_turn(&transcript, id);
    assert!(capture.collect(&store).is_err());
    assert!(staged_ids(&store, SOURCE_CLAUDE_CODE).is_empty());
}

/// Append one more line to a Codex rollout, so it is modified.
#[cfg(unix)]
fn codex_turn(path: &std::path::Path) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(file, r#"{{"type":"response_item","timestamp":"2026-07-01T11:00:00Z","payload":{{"type":"message","role":"user","content":[{{"type":"input_text","text":"Now run the suite"}}]}}}}"#).unwrap();
}

/// The root launch of a Codex conversation: its binding records `thread`,
/// whose rollout is created in the profile. Returns the root row id and the
/// rollout.
#[cfg(unix)]
fn codex_root(
    store: &ConfigStore,
    account: AccountId,
    root: &std::path::Path,
    thread: Uuid,
) -> (Uuid, std::path::PathBuf) {
    let id = Uuid::new_v4();
    let mut capture = NativeCollection::snapshot(
        store,
        ToolId::Codex,
        account,
        id,
        trace_commons_contributor::managed::session_binding::prepare(store, id).ok(),
    )
    .unwrap();
    let rollout = codex(root, thread, thread);
    record_codex_start(&mut capture, thread, &rollout);
    capture.collect(store).unwrap();
    (id, rollout)
}

/// A resumed Codex row has no binding of its own: it stages the rollout its
/// root's binding recorded, as it is now, under its own row id. A foreign
/// rollout that was there before stays private.
#[cfg(unix)]
#[test]
fn resumed_codex_row_stages_the_root_bindings_transcript_under_its_own_id() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let thread = Uuid::new_v4();
    let (root_id, rollout) = codex_root(&store, account, &root, thread);
    let foreign = codex(&root, Uuid::new_v4(), Uuid::new_v4());

    let resumed = Uuid::new_v4();
    let capture = NativeCollection::snapshot(&store, ToolId::Codex, account, resumed, None)
        .unwrap()
        .continuing(root_id);
    codex_turn(&rollout);
    capture.collect(&store).unwrap();

    let staged = store
        .dir()
        .join(trace_commons_contributor::managed::native_collection::STAGING)
        .join("codex")
        .join(resumed.to_string())
        .join("day")
        .join(rollout.file_name().unwrap());
    assert_eq!(
        std::fs::read(&staged).unwrap(),
        std::fs::read(&rollout).unwrap()
    );
    assert_eq!(
        staged_ids(&store, SOURCE_CODEX),
        vec![thread.to_string(), thread.to_string()],
        "the root's and the resumed row's copies, nothing foreign"
    );
    assert!(foreign.exists() && rollout.exists());
}

/// A resumed Codex row whose root rollout is gone at exit fails collection
/// and stages nothing.
#[cfg(unix)]
#[test]
fn resumed_codex_row_without_its_root_transcript_fails_collection() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let account = AccountId(Uuid::new_v4());
    let root = NativeProfiles::new(&store).profile_root(account).unwrap();
    let thread = Uuid::new_v4();
    let (root_id, rollout) = codex_root(&store, account, &root, thread);
    let resumed = Uuid::new_v4();
    let capture = NativeCollection::snapshot(&store, ToolId::Codex, account, resumed, None)
        .unwrap()
        .continuing(root_id);
    std::fs::remove_file(&rollout).unwrap();
    assert!(capture.collect(&store).is_err());
    assert_eq!(
        trace_commons_contributor::managed::native_collection::roots(&store, ToolId::Codex),
        vec![
            store
                .dir()
                .join(trace_commons_contributor::managed::native_collection::STAGING)
                .join("codex")
                .join(root_id.to_string())
        ],
        "only the root's earlier copy"
    );
}
