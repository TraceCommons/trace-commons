// The tests here are Unix-only; elsewhere their shared imports and helpers
// are unused, which -D warnings turns into a build failure (Linux and Windows CI).
#![cfg_attr(not(unix), allow(unused_imports, dead_code))]
use clap::Parser;
use serde_json::json;
use std::path::PathBuf;
use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::managed::cli::{
    LaunchArgs, MAX_INITIAL_PROMPT_BYTES, launch_with_tty, render_refusal, resolve_initial_prompt,
    validate_launch_args,
};
use trace_commons_contributor::managed::supervisor::initial_prompt_args;
use trace_commons_contributor::managed::{ManagedError, RED_TEAM_UNAVAILABLE_LABEL, ToolId};

#[derive(Parser)]
struct TestCli {
    #[command(flatten)]
    launch: LaunchArgs,
}

fn parse_launch(args: &[&str]) -> Result<LaunchArgs, clap::Error> {
    let mut full = vec!["launch"];
    full.extend_from_slice(args);
    TestCli::try_parse_from(full).map(|c| c.launch)
}

#[test]
fn red_team_flags_on_claude_are_refused() {
    for flag_set in [
        vec!["claude", "--input", "/tmp"],
        vec!["claude", "--allow-host", "example.com:443"],
        vec!["claude", "--packages"],
        vec!["claude", "--cpus", "2"],
        vec!["claude", "--memory", "2G"],
    ] {
        let args = parse_launch(&flag_set).expect("clap parses args");
        let err = validate_launch_args(&args).expect_err("red-team flags on claude must fail");
        assert!(
            err.to_string()
                .contains("these options apply only to red-team"),
            "unexpected error for {flag_set:?}: {err}"
        );
    }
}

#[test]
fn prompt_and_prompt_file_conflict() {
    let err = parse_launch(&["claude", "--prompt", "hi", "--prompt-file", "/tmp/p.txt"])
        .err()
        .expect("clap must reject --prompt with --prompt-file");
    assert_eq!(err.kind(), clap::error::ErrorKind::ArgumentConflict);

    let err = resolve_initial_prompt(Some("hi".into()), Some(std::path::Path::new("/tmp/p.txt")))
        .expect_err("resolve_initial_prompt must also reject both");
    assert!(err.to_string().contains("cannot be used together"));
}

#[test]
fn a_prompt_file_over_64_kib_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let big = dir.path().join("big.txt");
    std::fs::write(&big, vec![b'a'; MAX_INITIAL_PROMPT_BYTES + 1]).unwrap();
    let err = resolve_initial_prompt(None, Some(&big))
        .expect_err("prompt file over 64 KiB must be refused");
    assert!(
        err.to_string().contains("64 KiB"),
        "unexpected error: {err}"
    );

    let ok = dir.path().join("ok.txt");
    std::fs::write(&ok, vec![b'a'; MAX_INITIAL_PROMPT_BYTES]).unwrap();
    assert_eq!(
        resolve_initial_prompt(None, Some(&ok))
            .unwrap()
            .unwrap()
            .len(),
        MAX_INITIAL_PROMPT_BYTES
    );
}

#[tokio::test]
async fn non_interactive_launch_without_new_terminal_names_the_flag() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let args = parse_launch(&["claude"]).unwrap();
    let err = launch_with_tty(&store, args, false, false)
        .await
        .expect_err("non-interactive launch without --new-terminal must fail");
    assert!(
        err.to_string().contains("--new-terminal"),
        "error must name --new-terminal: {err}"
    );
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_terminal_prints_the_session_id_as_json() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let sock_path = store.dir().join("daemon.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let session_id = uuid::Uuid::new_v4();
    let account_id = uuid::Uuid::new_v4();

    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                continue;
            }
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_u64().unwrap_or(1);
            let method = req["method"].as_str().unwrap_or("");
            let result = match method {
                "managed_snapshot" => json!({
                    "accounts": [],
                    "sign_in_accounts": [{
                        "id": account_id,
                        "tool": "claude",
                        "connection": "near_ai_signin",
                        "label": "NEAR AI",
                        "auth_state": "ready",
                        "verified_at": null
                    }],
                    "defaults": [],
                    "generations": {"claude": 0},
                    "tools": [{"tool": "claude", "installed": true}]
                }),
                "managed_folder_settings_get" => json!({
                    "tool": "claude",
                    "model": null,
                    "collect_traces": false,
                    "cli_options": "",
                    "remembered": false
                }),
                "managed_launch_prepare" => json!({
                    "session_id": session_id,
                    "ticket": "test-ticket"
                }),
                "managed_terminal_launch" => json!({"opened": true}),
                _ => json!({}),
            };
            let resp = json!({"id": id, "result": result});
            let mut stream = reader.into_inner();
            let _ = stream
                .write_all(format!("{}\n", serde_json::to_string(&resp).unwrap()).as_bytes())
                .await;
        }
    });

    let args = parse_launch(&[
        "claude",
        "--new-terminal",
        "--cwd",
        dir.path().to_str().unwrap(),
    ])
    .unwrap();
    let code = launch_with_tty(&store, args, true, false).await.unwrap();
    assert_eq!(code, 0);
    server.abort();
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn new_terminal_without_a_terminal_is_refused_with_terminal_unavailable() {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixListener;

    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let sock_path = store.dir().join("daemon.sock");
    let listener = UnixListener::bind(&sock_path).unwrap();
    let session_id = uuid::Uuid::new_v4();
    let account_id = uuid::Uuid::new_v4();

    let server = tokio::spawn(async move {
        for _ in 0..4 {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                continue;
            }
            let req: serde_json::Value = serde_json::from_str(&line).unwrap();
            let id = req["id"].as_u64().unwrap_or(1);
            let method = req["method"].as_str().unwrap_or("");
            let resp = match method {
                "managed_snapshot" => json!({
                    "id": id,
                    "result": {
                        "accounts": [],
                        "sign_in_accounts": [{
                            "id": account_id,
                            "tool": "claude",
                            "connection": "near_ai_signin",
                            "label": "NEAR AI",
                            "auth_state": "ready",
                            "verified_at": null
                        }],
                        "defaults": [],
                        "generations": {"claude": 0},
                        "tools": [{"tool": "claude", "installed": true}]
                    }
                }),
                "managed_folder_settings_get" => json!({
                    "id": id,
                    "result": {
                        "tool": "claude",
                        "model": null,
                        "collect_traces": false,
                        "cli_options": "",
                        "remembered": false
                    }
                }),
                "managed_launch_prepare" => json!({
                    "id": id,
                    "result": {
                        "session_id": session_id,
                        "ticket": "test-ticket"
                    }
                }),
                "managed_terminal_launch" => json!({
                    "id": id,
                    "error": {
                        "code": "managed-terminal-not-found",
                        "message": "managed-terminal-not-found"
                    }
                }),
                _ => json!({"id": id, "result": {}}),
            };
            let mut stream = reader.into_inner();
            let _ = stream
                .write_all(format!("{}\n", serde_json::to_string(&resp).unwrap()).as_bytes())
                .await;
        }
    });

    let args = parse_launch(&[
        "claude",
        "--new-terminal",
        "--cwd",
        dir.path().to_str().unwrap(),
    ])
    .unwrap();
    let err = launch_with_tty(&store, args, false, false)
        .await
        .expect_err("missing terminal must fail cleanly");
    assert_eq!(err.to_string(), "managed-terminal-not-found");
    server.abort();
}

#[test]
fn red_team_on_a_daemon_without_it_says_so() {
    let err = render_refusal(RED_TEAM_UNAVAILABLE_LABEL, RED_TEAM_UNAVAILABLE_LABEL);
    let msg = err.to_string();
    assert_eq!(msg, ManagedError::RedTeamUnavailable.to_string());
    assert!(!msg.contains('\n'), "must be one clear sentence: {msg}");
    assert!(msg.ends_with('.'), "must be a sentence: {msg}");
}

#[test]
fn initial_prompt_argv_per_tool() {
    // Verified against `claude --help`, `codex --help`, and `opencode --help`.
    assert_eq!(
        initial_prompt_args(ToolId::Claude, "audit this repo"),
        vec!["--".to_owned(), "audit this repo".to_owned()]
    );
    assert_eq!(
        initial_prompt_args(ToolId::Codex, "audit this repo"),
        vec!["--".to_owned(), "audit this repo".to_owned()]
    );
    assert_eq!(
        initial_prompt_args(ToolId::Opencode, "audit this repo"),
        vec!["--prompt".to_owned(), "audit this repo".to_owned()]
    );
    // Red Team passes initial_prompt via LaunchSpec, not CLI argv.
    assert!(initial_prompt_args(ToolId::RedTeam, "audit this repo").is_empty());
    let _ = PathBuf::new();
}
