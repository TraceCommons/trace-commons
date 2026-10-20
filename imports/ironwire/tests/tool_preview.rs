//! Conformance: a client can be told what wiring a tool would do before it happens.
//!
//! `ironwire_agents` splits working an edit out from making it, and the CLI
//! spends that split on a question: it prints the plan and waits. Over the
//! control API the same split has to survive, or a menu bar and a third-party
//! client can only edit somebody's config first and describe it afterwards.
//! What is asserted here is the part that cannot be seen in a response body:
//! that a preview leaves the file alone, and that a commit against a file that
//! has moved since is refused rather than writing a change nobody was shown.
#![cfg(unix)]

use tokio::io::{AsyncBufReadExt, BufReader};

/// A daemon on a private HOME, its control token, and its port.
struct Daemon {
    child: tokio::process::Child,
    port: u16,
    token: String,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

async fn start(home: &std::path::Path, codex_home: &std::path::Path) -> Daemon {
    start_with(home, codex_home, &[]).await
}

/// As [`start`], with more of the daemon's environment.
async fn start_with(
    home: &std::path::Path,
    codex_home: &std::path::Path,
    env: &[(&str, &std::path::Path)],
) -> Daemon {
    start_moving(home, codex_home, env, &[]).await
}

/// As [`start_with`], with each default login in `moved` switched on and
/// relocated to the `config_dir` beside its id.
async fn start_moving(
    home: &std::path::Path,
    codex_home: &std::path::Path,
    env: &[(&str, &std::path::Path)],
    moved: &[(&str, &str)],
) -> Daemon {
    let runtime = home.join("runtime");
    std::fs::create_dir_all(&runtime).unwrap();
    let mut config = String::from("[updates]\ncheck = false\n");
    for (id, kind) in [
        ("nearai", "nearai"),
        ("claude-sub", "claude-subscription"),
        ("codex-sub", "codex-subscription"),
        ("anthropic-key", "anthropic-api"),
        ("openai-key", "openai-api"),
    ] {
        let rest = match moved.iter().find(|(login, _)| *login == id) {
            Some((_, dir)) => format!("config_dir = '{dir}'"),
            None => "enabled = false".to_string(),
        };
        config.push_str(&format!(
            "[[backends]]\nid = '{id}'\nkind = '{kind}'\n{rest}\n"
        ));
    }
    std::fs::write(runtime.join("config.toml"), config).unwrap();
    let paths = ironwire_core::config::PathsConfig::rooted_at(runtime.clone());
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironwire"))
        .args(["serve", "--port", "0"])
        .env_clear()
        .env("HOME", home)
        .env("IRONWIRE_HOME", &runtime)
        .env("CODEX_HOME", codex_home)
        .env("PATH", "")
        .envs(env.iter().copied())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let line = lines
                .next_line()
                .await
                .unwrap()
                .expect("CLI stays alive until listening");
            if let Some(port) = line.strip_prefix("IronWire listening on http://127.0.0.1:") {
                break port.parse::<u16>().unwrap();
            }
        }
    })
    .await
    .unwrap();
    let token = std::fs::read_to_string(paths.control_token_file()).unwrap();
    Daemon {
        child,
        port,
        token: token.trim().to_string(),
    }
}

async fn post_tools(daemon: &Daemon, body: serde_json::Value) -> (u16, serde_json::Value) {
    let response = reqwest::Client::new()
        .post(format!("http://127.0.0.1:{}/_ironwire/tools", daemon.port))
        .bearer_auth(&daemon.token)
        .json(&body)
        .send()
        .await
        .unwrap();
    let status = response.status().as_u16();
    (status, response.json().await.unwrap())
}

/// The whole arc in one daemon, because what each step proves is what the file
/// on disk says after the step before it.
#[tokio::test]
async fn a_preview_reports_the_change_without_making_it() {
    let home = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let config = codex_home.path().join("config.toml");
    let original = "model = \"gpt-5-codex\"\n";
    std::fs::write(&config, original).unwrap();
    let backup = codex_home.path().join("config.toml.ironwire-backup");

    let daemon = start(home.path(), codex_home.path()).await;

    let (status, preview) = post_tools(
        &daemon,
        serde_json::json!({"id": "codex", "connect": true, "dry_run": true}),
    )
    .await;
    assert_eq!(status, 200, "{preview}");
    assert_eq!(preview["applied"], false);
    assert_eq!(preview["path"], config.display().to_string());
    assert!(
        !preview["changes"].as_array().unwrap().is_empty(),
        "a preview that names no change is not a preview: {preview}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        original,
        "the preview wrote to the file it was only asked about"
    );
    assert!(!backup.exists(), "the preview took a backup, so it wrote");

    // A plan the caller was never shown must not be committable in its place.
    let stale = "0".repeat(64);
    let (status, refusal) = post_tools(
        &daemon,
        serde_json::json!({"id": "codex", "connect": true, "as_previewed": stale}),
    )
    .await;
    assert_eq!(status, 409, "{refusal}");
    assert!(
        refusal["error"]
            .as_str()
            .unwrap()
            .contains("Preview it again"),
        "{refusal}"
    );
    assert_eq!(
        std::fs::read_to_string(&config).unwrap(),
        original,
        "a refused commit still wrote"
    );

    // The digest the preview handed back commits exactly what it described.
    let (status, written) = post_tools(
        &daemon,
        serde_json::json!({
            "id": "codex",
            "connect": true,
            "as_previewed": preview["digest"].as_str().unwrap(),
        }),
    )
    .await;
    assert_eq!(status, 200, "{written}");
    assert_eq!(written["applied"], true);
    assert_eq!(written["changes"], preview["changes"]);
    let after = std::fs::read_to_string(&config).unwrap();
    assert!(after.contains("model_provider = \"ironwire\""), "{after}");
    assert!(
        after.contains(original.trim()),
        "the user's own key survived"
    );
    assert_eq!(std::fs::read_to_string(&backup).unwrap(), original);
}

/// The direction is part of what was previewed, not just the file.
///
/// A half-wired config — our status line in place, the base URL never set — has
/// a real connect *and* a real disconnect worked out from the same bytes. A
/// client shown the additions must not be able to commit the removal with the
/// digest it was answered with, which is what a digest over the file alone
/// would have allowed.
#[tokio::test]
async fn a_digest_confirms_the_direction_it_was_shown() {
    let home = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let claude = home.path().join(".claude");
    std::fs::create_dir_all(&claude).unwrap();
    let settings = claude.join("settings.json");
    let original = r#"{"statusLine":{"type":"command","command":"ironwire statusline","installedBy":"ironwire"}}"#;
    std::fs::write(&settings, original).unwrap();

    let daemon = start(home.path(), codex_home.path()).await;

    let (status, connect) = post_tools(
        &daemon,
        serde_json::json!({"id": "claude", "connect": true, "dry_run": true}),
    )
    .await;
    assert_eq!(status, 200, "{connect}");
    let adds_the_url = connect["changes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|change| change.as_str().unwrap().contains("ANTHROPIC_BASE_URL"));
    assert!(
        adds_the_url,
        "not the additions this test is about: {connect}"
    );

    let (status, disconnect) = post_tools(
        &daemon,
        serde_json::json!({"id": "claude", "connect": false, "dry_run": true}),
    )
    .await;
    assert_eq!(status, 200, "{disconnect}");
    assert!(
        !disconnect["changes"].as_array().unwrap().is_empty(),
        "the same bytes have to yield a real disconnect too, or this proves nothing: {disconnect}"
    );

    // The client was shown the connect. Sending its digest with the other
    // direction is a different edit, and must be refused.
    let (status, refusal) = post_tools(
        &daemon,
        serde_json::json!({
            "id": "claude",
            "connect": false,
            "as_previewed": connect["digest"].as_str().unwrap(),
        }),
    )
    .await;
    assert_eq!(
        status, 409,
        "a preview of the additions committed the removal: {refusal}"
    );
    assert_eq!(
        std::fs::read_to_string(&settings).unwrap(),
        original,
        "the status line was removed by a plan nobody was shown"
    );

    // The direction it was actually shown still commits.
    let (status, written) = post_tools(
        &daemon,
        serde_json::json!({
            "id": "claude",
            "connect": true,
            "as_previewed": connect["digest"].as_str().unwrap(),
        }),
    )
    .await;
    assert_eq!(status, 200, "{written}");
    assert_eq!(written["applied"], true);
    let after = std::fs::read_to_string(&settings).unwrap();
    assert!(after.contains("ANTHROPIC_BASE_URL"), "{after}");
    assert!(after.contains("statusLine"), "{after}");
}

/// The daemon's own `CLAUDE_CONFIG_DIR` names a login of its own — one whose
/// Claude Code the plain URL would hand to the default login's subscription.
/// So the control API's Claude Code is always the default login's, in
/// `~/.claude`, whatever environment the daemon was started from; that other
/// Claude Code is connected from a terminal, with the URL naming its login.
#[tokio::test]
async fn a_control_api_connect_never_writes_the_plain_url_into_another_logins_directory() {
    let home = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let work = home.path().join(".claude-work");
    std::fs::create_dir_all(&work).unwrap();
    let default = home.path().join(".claude").join("settings.json");

    let daemon = start_with(
        home.path(),
        codex_home.path(),
        &[("CLAUDE_CONFIG_DIR", work.as_path())],
    )
    .await;

    let (status, written) = post_tools(
        &daemon,
        serde_json::json!({"id": "claude", "connect": true}),
    )
    .await;
    assert_eq!(status, 200, "{written}");
    assert_eq!(written["applied"], true);
    assert_eq!(written["path"], default.display().to_string());
    assert!(
        !work.join("settings.json").exists(),
        "the plain URL went into the directory of a login of its own"
    );
    let after = std::fs::read_to_string(&default).unwrap();
    assert!(after.contains("/anthropic\""), "{after}");

    // Undone in the same file, and listed as that file.
    let (status, undone) = post_tools(
        &daemon,
        serde_json::json!({"id": "claude", "connect": false, "dry_run": true}),
    )
    .await;
    assert_eq!(status, 200, "{undone}");
    assert_eq!(undone["path"], default.display().to_string());

    let settings: serde_json::Value = reqwest::Client::new()
        .get(format!(
            "http://127.0.0.1:{}/_ironwire/settings",
            daemon.port
        ))
        .bearer_auth(&daemon.token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let claude = settings["tools"]
        .as_array()
        .unwrap()
        .iter()
        .find(|tool| tool["id"] == "claude")
        .cloned()
        .unwrap();
    assert_eq!(
        claude["config_path"],
        default.display().to_string(),
        "{claude}"
    );
}

/// Config can move a login into the very directory the tools path writes — the
/// default relocated to `~/.claude` itself, or to the daemon's `CODEX_HOME`.
/// That login is served only at the URL naming it, so a plain URL there would
/// take its client off its subscription without a word. The control API
/// refuses and names the command that wires it; a file that command already
/// bound stays bound through a connect from the menu.
#[tokio::test]
async fn a_control_api_connect_never_writes_the_plain_url_where_a_moved_login_reads() {
    let home = tempfile::tempdir().unwrap();
    let codex_home = tempfile::tempdir().unwrap();
    let claude_dir = home.path().join(".claude");
    std::fs::create_dir_all(&claude_dir).unwrap();
    let settings = claude_dir.join("settings.json");
    let codex_config = codex_home.path().join("config.toml");
    let codex_dir = codex_home.path().display().to_string();

    let daemon = start_moving(
        home.path(),
        codex_home.path(),
        &[],
        &[("claude-sub", "~/.claude"), ("codex-sub", &codex_dir)],
    )
    .await;

    for (tool, file, command) in [
        (
            "claude",
            &settings,
            "ironwire connect claude --config-dir ~/.claude".to_string(),
        ),
        (
            "codex",
            &codex_config,
            format!("ironwire connect codex --config-dir {codex_dir}"),
        ),
    ] {
        let (status, refused) =
            post_tools(&daemon, serde_json::json!({"id": tool, "connect": true})).await;
        assert_eq!(status, 400, "{refused}");
        assert_eq!(refused["reason"], "login-directory", "{refused}");
        let error = refused["error"].as_str().unwrap();
        assert!(error.contains(&command), "{error}");
        assert!(!file.exists(), "{tool}: the plain URL was written anyway");
    }

    let bound = format!(
        "{{\"env\":{{\"ANTHROPIC_BASE_URL\":\"http://127.0.0.1:{}/anthropic/account/claude-sub\"}}}}",
        daemon.port
    );
    std::fs::write(&settings, bound).unwrap();
    let (status, kept) = post_tools(
        &daemon,
        serde_json::json!({"id": "claude", "connect": true}),
    )
    .await;
    assert_eq!(status, 200, "{kept}");
    let after = std::fs::read_to_string(&settings).unwrap();
    assert!(after.contains("/anthropic/account/claude-sub\""), "{after}");
}
