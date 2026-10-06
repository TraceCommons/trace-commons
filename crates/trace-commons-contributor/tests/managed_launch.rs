#![cfg(unix)]

use serde_json::json;
use std::{
    os::unix::fs::PermissionsExt,
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use trace_commons_contributor::{
    config::ConfigStore,
    daemon::{
        ipc::{Request, handle_request},
        start_embedded,
    },
};

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cli_launch_appears_in_ui_snapshot_and_reports_native_exit() {
    exercise_terminal_exit(false, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_hangup_records_exit_and_releases_the_account() {
    exercise_terminal_exit(true, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn terminal_hangup_during_setup_never_starts_a_native_session() {
    exercise_terminal_exit(false, true).await;
}

async fn exercise_terminal_exit(hangup: bool, setup_hangup: bool) {
    let home = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(home.path().join("state")).unwrap();
    let daemon = start_embedded(store).await.unwrap();
    let created = handle_request(
        &daemon.shared,
        &Request {
            id: 1,
            method: "managed_account_add".into(),
            params: json!({"tool":"claude","connection":"subscription","label":"Personal"}),
        },
    );
    assert!(created.error.is_none());
    let account = created.result.unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let project = home.path().join("project with spaces");
    std::fs::create_dir(&project).unwrap();
    let native = bin.join("claude");
    std::fs::write(&native, "#!/bin/sh\ncase \"$1\" in\n--version) if [ \"$FIXTURE_SLOW_SETUP\" = 1 ]; then printf '%s' \"$PPID\" > setup-helper; sleep 0.5; fi; echo '2.1.289 (Claude Code)'; exit 0;;\nauth) echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0;;\nesac\nprintf '%s' \"$CLAUDE_CONFIG_DIR\" > profile-root\nprintf '%s' \"$PPID\" > helper-pid\nwhile [ ! -f finish ]; do sleep 0.05; done\nexit 7\n").unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut child = Command::new("python3")
        .args([
            "-c",
            "import os,pty,sys; sys.exit(os.waitstatus_to_exitcode(pty.spawn(sys.argv[1:])))",
            env!("CARGO_BIN_EXE_near-ai"),
        ])
        .arg("--config-dir")
        .arg(home.path().join("state"))
        .args(["launch", "claude", "--account", "Personal", "--cwd"])
        .arg(&project)
        .env(
            "PATH",
            format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
        )
        .env("FIXTURE_SLOW_SETUP", if setup_hangup { "1" } else { "0" })
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    if setup_hangup {
        let marker = home
            .path()
            .join("state/managed-profiles")
            .join(account["id"].as_str().unwrap())
            .join("setup-helper");
        while !marker.exists() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let pid = std::fs::read_to_string(marker).unwrap();
        assert!(
            Command::new("/bin/kill")
                .args(["-HUP", pid.trim()])
                .status()
                .unwrap()
                .success()
        );
        while child.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let snapshot = handle_request(
            &daemon.shared,
            &Request {
                id: 3,
                method: "managed_snapshot".into(),
                params: json!({}),
            },
        )
        .result
        .unwrap();
        assert_eq!(snapshot["sessions"][0]["state"], "failed");
        assert!(!project.join("profile-root").exists());
        daemon.close();
        return;
    }
    let running = loop {
        let response = handle_request(
            &daemon.shared,
            &Request {
                id: 2,
                method: "managed_snapshot".into(),
                params: json!({}),
            },
        );
        let snapshot = response.result.unwrap();
        if let Some(row) = snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|r| r["state"] == "running")
        {
            break row.clone();
        }
        if child.try_wait().unwrap().is_some() || Instant::now() >= deadline {
            let _ = child.kill();
            panic!("CLI did not register a running session");
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    assert_eq!(running["account_id"], account["id"]);
    assert_eq!(running["account_label"], "Personal");
    assert_eq!(running["tool"], "claude");
    assert_eq!(
        running["cwd"],
        json!(std::fs::canonicalize(&project).unwrap())
    );
    while !project.join("helper-pid").exists() {
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        std::fs::read_to_string(project.join("profile-root"))
            .unwrap()
            .contains(account["id"].as_str().unwrap())
    );
    if hangup {
        let pid = std::fs::read_to_string(project.join("helper-pid")).unwrap();
        assert!(
            Command::new("/bin/kill")
                .args(["-HUP", pid.trim()])
                .status()
                .unwrap()
                .success()
        );
    } else {
        std::fs::write(project.join("finish"), b"").unwrap();
    }
    let expected_code = if hangup { 129 } else { 7 };
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(30)).await;
    };
    assert_eq!(status.code(), Some(expected_code));
    let snapshot = handle_request(
        &daemon.shared,
        &Request {
            id: 3,
            method: "managed_snapshot".into(),
            params: json!({}),
        },
    )
    .result
    .unwrap();
    assert_eq!(snapshot["sessions"][0]["state"], "exited");
    assert_eq!(snapshot["sessions"][0]["exit_code"], expected_code);
    daemon.close();
}

#[test]
fn launch_without_daemon_refuses_instead_of_starting_an_untracked_tool() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_near-ai"))
        .arg("--config-dir")
        .arg(home.path())
        .args(["--json", "launch", "claude", "--account", "Personal"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert!(value["error"].as_str().unwrap().contains("daemon"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn two_managed_accounts_and_a_standard_session_run_together() {
    struct ChildGuard(std::process::Child);
    impl Drop for ChildGuard {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    let home = tempfile::tempdir().unwrap();
    let state = home.path().join("state");
    let daemon = start_embedded(ConfigStore::open(state.clone()).unwrap())
        .await
        .unwrap();
    let bin = home.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let native = bin.join("claude");
    std::fs::write(&native, "#!/bin/sh\ncase \"$1\" in\n--version) echo '2.1.289 (Claude Code)'; exit 0;;\nauth) echo '{\"loggedIn\":true,\"authMethod\":\"claude.ai\"}'; exit 0;;\nesac\nprintf '%s' \"$CLAUDE_CONFIG_DIR\" > profile-root\nprintf '%s' \"$ANTHROPIC_API_KEY\" > inherited-key\nwhile [ ! -f finish ]; do sleep 0.05; done\n").unwrap();
    std::fs::set_permissions(&native, std::fs::Permissions::from_mode(0o700)).unwrap();
    let mut children = vec![];
    let mut projects = vec![];
    let mut ids = vec![];
    for label in ["Personal", "Work"] {
        let account = handle_request(
            &daemon.shared,
            &Request {
                id: 1,
                method: "managed_account_add".into(),
                params: json!({"tool":"claude","connection":"subscription","label":label}),
            },
        )
        .result
        .unwrap();
        ids.push(account["id"].as_str().unwrap().to_owned());
        let project = home.path().join(label);
        std::fs::create_dir(&project).unwrap();
        children.push(ChildGuard(Command::new("python3").args(["-c", "import os,pty,sys; sys.exit(os.waitstatus_to_exitcode(pty.spawn(sys.argv[1:])))", env!("CARGO_BIN_EXE_near-ai")]).arg("--config-dir").arg(&state).args(["launch","claude","--account",label,"--cwd"]).arg(&project).env("PATH",format!("{}:{}",bin.display(),std::env::var("PATH").unwrap())).env("ANTHROPIC_API_KEY","ordinary-fixture-key").env("CLAUDE_CONFIG_DIR","ordinary-profile").stdin(Stdio::null()).stdout(Stdio::from(std::fs::File::create(home.path().join(format!("{label}.out"))).unwrap())).stderr(Stdio::from(std::fs::File::create(home.path().join(format!("{label}.err"))).unwrap())).spawn().unwrap()));
        projects.push(project);
    }
    let standard = home.path().join("Standard");
    std::fs::create_dir(&standard).unwrap();
    children.push(ChildGuard(
        Command::new(&native)
            .current_dir(&standard)
            .env("CLAUDE_CONFIG_DIR", "ordinary-profile")
            .env("ANTHROPIC_API_KEY", "ordinary-fixture-key")
            .spawn()
            .unwrap(),
    ));
    projects.push(standard.clone());
    let deadline = Instant::now() + Duration::from_secs(15);
    while projects.iter().any(|p| !p.join("inherited-key").exists()) {
        // On a timeout, say which launches never reached the tool and what
        // they printed: the children's output is otherwise discarded.
        if Instant::now() >= deadline {
            let waiting: Vec<_> = projects.iter().filter(|p| !p.join("inherited-key").exists()).collect();
            let output: Vec<_> = ["Personal", "Work"].iter().flat_map(|label| ["out", "err"].map(|kind| format!("{label}.{kind}: {}", std::fs::read_to_string(home.path().join(format!("{label}.{kind}"))).unwrap_or_default()))).collect();
            panic!("launches never reached the tool: {waiting:?}\n{}", output.join("\n"));
        }
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    loop {
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
        if snapshot["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| s["state"] == "running")
            .count()
            == 2
        {
            break;
        }
        assert!(Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(30)).await;
    }
    for (project, id) in projects[..2].iter().zip(ids.iter()) {
        assert!(
            std::fs::read_to_string(project.join("profile-root"))
                .unwrap()
                .contains(id)
        );
        assert_eq!(
            std::fs::read_to_string(project.join("inherited-key")).unwrap(),
            ""
        );
    }
    assert_eq!(
        std::fs::read_to_string(standard.join("profile-root")).unwrap(),
        "ordinary-profile"
    );
    assert_eq!(
        std::fs::read_to_string(standard.join("inherited-key")).unwrap(),
        "ordinary-fixture-key"
    );
    for project in &projects {
        std::fs::write(project.join("finish"), b"").unwrap();
    }
    for child in &mut children {
        while child.0.try_wait().unwrap().is_none() {
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
    daemon.close();
}
