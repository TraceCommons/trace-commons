//! `ironwire statusline`, end to end: what Claude Code actually reads.
//!
//! The rendering moved into `ironwire_proxy::statusline` so embedding hosts can
//! print the same line. These pin the bytes the CLI prints, so the move cannot
//! quietly change them.

use std::process::Stdio;

use ironwire_proxy::control::{
    BackendView, BalanceView, HeadroomView, HealthView, LastRouteView, StatusView,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const TOKEN: &str = "cli-statusline-token";

fn status(port: u16) -> StatusView {
    StatusView {
        version: "0.1.0".into(),
        port,
        tracked_conversations: 1,
        pin: None,
        backends: vec![BackendView {
            id: "claude-sub".into(),
            name: "Claude subscription".into(),
            kind: "subscription".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom: HeadroomView::Observed {
                used_pct: 61.0,
                observed_secs_ago: 5,
                resets_in_secs: None,
            },
            health: HealthView::default(),
            models: vec![],
            connect_command: None,
        }],
        balance: BalanceView::default(),
        privacy: None,
        catalog_serial: 0,
        update: ironwire_update::UpdateStatus::UpToDate,
        last_route: Some(LastRouteView {
            backend: "nearai".into(),
            model: Some("glm-5".into()),
            from: Some("claude-sub".into()),
            rung: ironwire_core::policy::Rung::CrossFamily,
            at: chrono::Utc::now(),
        }),
        usage: ironwire_usage::UsageReport::default(),
        cross_account_fallback: false,
    }
}

/// A one-route daemon: `/_ironwire/status` for the right bearer token, 401
/// otherwise. Raw HTTP/1.1 so the binary's tests need no server framework.
async fn spawn_daemon() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let body = serde_json::to_string(&status(port)).unwrap();
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let body = body.clone();
            tokio::spawn(async move {
                let mut request = Vec::new();
                let mut buffer = [0u8; 4096];
                while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                    match socket.read(&mut buffer).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => request.extend_from_slice(&buffer[..n]),
                    }
                }
                let request = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let response = if request.starts_with("get /_ironwire/status ")
                    && request.contains(&format!("authorization: bearer {TOKEN}"))
                {
                    format!(
                        "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                        body.len()
                    )
                } else {
                    "HTTP/1.1 401 Unauthorized\r\ncontent-length: 0\r\nconnection: close\r\n\r\n"
                        .to_string()
                };
                let _ = socket.write_all(response.as_bytes()).await;
            });
        }
    });
    port
}

fn home_for(port: u16) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        format!("[server]\nport = {port}\n[updates]\ncheck = false\n"),
    )
    .unwrap();
    std::fs::write(home.path().join("control.token"), TOKEN).unwrap();
    home
}

async fn statusline(home: &std::path::Path, stdin: &str) -> std::process::Output {
    let mut child = tokio::process::Command::new(env!("CARGO_BIN_EXE_ironwire"))
        .arg("statusline")
        .env_clear()
        .env("HOME", home)
        .env("IRONWIRE_HOME", home)
        .env("PATH", "")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    input.write_all(stdin.as_bytes()).await.unwrap();
    drop(input);
    tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
        .await
        .expect("a status line never blocks")
        .unwrap()
}

#[tokio::test]
async fn the_cli_prints_the_line_claude_code_has_always_read() {
    let port = spawn_daemon().await;
    let home = home_for(port);
    let output = statusline(
        home.path(),
        r#"{"session_id":"x","model":{"id":"claude-opus-5","display_name":"Opus 5"}}"#,
    )
    .await;
    assert!(output.status.success());
    assert!(output.stderr.is_empty(), "{output:?}");
    assert_eq!(
        String::from_utf8(output.stdout).unwrap(),
        "ironwire → nearai (glm-5 for claude-opus-5) · switched from claude-sub · Claude 61%\n"
    );
}

/// Quiet, and still exit 0: a status line that reports its own failure into
/// someone's editor has made their problem worse.
#[tokio::test]
async fn the_cli_prints_nothing_when_the_daemon_refuses_it() {
    let port = spawn_daemon().await;
    let home = home_for(port);
    std::fs::write(home.path().join("control.token"), "a-different-token").unwrap();
    let output = statusline(home.path(), "").await;
    assert!(output.status.success());
    assert!(output.stdout.is_empty(), "{output:?}");
    assert!(output.stderr.is_empty(), "{output:?}");
}
