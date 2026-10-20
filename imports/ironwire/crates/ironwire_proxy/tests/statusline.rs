//! A host that embeds IronWire serves the same status line the CLI does.
//!
//! The line is rendered by `ironwire_proxy::statusline`, so a host with its
//! own executable can put it in Claude Code's `statusLine` slot instead of
//! declining it. These tests hold the library to the three properties the CLI
//! has always promised: it never blocks, never invents, and is quiet when
//! there is nothing to say.

use std::time::{Duration, Instant};

use axum::{Json, Router, http::HeaderMap, http::StatusCode, routing::get};
use ironwire_proxy::control::{
    BackendView, BalanceView, HeadroomView, HealthView, LastRouteView, StatusView,
};
use ironwire_proxy::statusline;

const TOKEN: &str = "test-control-token";

fn status(port: u16) -> StatusView {
    StatusView {
        version: "0.1.0".into(),
        port,
        tracked_conversations: 1,
        pin: None,
        backends: vec![BackendView {
            id: "nearai".into(),
            name: "NEAR AI".into(),
            kind: "credits".into(),
            authenticated: true,
            consented: true,
            detail: None,
            headroom: HeadroomView::Unknown,
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
            model: None,
            from: None,
            rung: ironwire_core::policy::Rung::CrossFamily,
            at: chrono::Utc::now(),
        }),
        usage: ironwire_usage::UsageReport::default(),
        cross_account_fallback: false,
    }
}

/// A stand-in daemon: answers `/_ironwire/status` for the right token only.
async fn spawn_daemon() -> u16 {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let app = Router::new().route(
        "/_ironwire/status",
        get(move |headers: HeaderMap| async move {
            let authorised = headers.get("authorization").and_then(|v| v.to_str().ok())
                == Some(&format!("Bearer {TOKEN}"));
            if authorised {
                Ok(Json(status(port)))
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }),
    );
    tokio::spawn(async move { axum::serve(listener, app).await });
    port
}

/// A home laid out the way an embedded proxy leaves it: the token, and the
/// pointer naming the port it actually bound.
fn home_pointing_at(port: u16) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("control.token"), TOKEN).unwrap();
    let endpoint = ironwire_core::discovery::Endpoint::new(port, home.path().join("control.token"));
    std::fs::write(
        home.path().join("endpoint.json"),
        serde_json::to_string(&endpoint).unwrap(),
    )
    .unwrap();
    home
}

#[tokio::test]
async fn a_host_renders_the_line_from_a_port_and_token() {
    let port = spawn_daemon().await;
    assert_eq!(
        statusline::line(port, TOKEN, None).await.as_deref(),
        Some("ironwire → nearai")
    );
}

/// The case the home form exists for: an embedded proxy on an ephemeral port,
/// which its configuration file does not know. The pointer does.
#[tokio::test]
async fn a_host_renders_the_line_from_the_home_it_started_the_proxy_in() {
    let port = spawn_daemon().await;
    let home = home_pointing_at(port);
    assert_eq!(
        statusline::endpoint_for_home(home.path()),
        Some((port, TOKEN.to_string()))
    );
    assert_eq!(
        statusline::line_for_home(home.path(), Some("claude-opus-5"))
            .await
            .as_deref(),
        Some("ironwire → nearai")
    );
}

#[tokio::test]
async fn a_wrong_token_prints_nothing() {
    let port = spawn_daemon().await;
    assert!(
        statusline::line(port, "not-the-token", None)
            .await
            .is_none()
    );
    assert!(
        statusline::fetch_status(port, "not-the-token")
            .await
            .is_none()
    );
}

#[tokio::test]
async fn nothing_listening_prints_nothing() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    assert!(statusline::line(port, TOKEN, None).await.is_none());
}

/// It never blocks the UI: a daemon that accepts and never answers costs the
/// render loop the budget, not the transport's own timeout.
#[tokio::test]
async fn a_daemon_that_never_answers_costs_no_more_than_the_budget() {
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let held = tokio::spawn(async move {
        let mut open = Vec::new();
        while let Ok((socket, _)) = listener.accept().await {
            open.push(socket);
        }
    });
    let started = Instant::now();
    assert!(statusline::line(port, TOKEN, None).await.is_none());
    let elapsed = started.elapsed();
    assert!(
        elapsed >= statusline::BUDGET && elapsed < Duration::from_secs(2),
        "took {elapsed:?}"
    );
    held.abort();
}

/// A status line runs on every render. It must not mint a token or create a
/// home on the way to printing nothing.
#[tokio::test]
async fn a_home_with_nothing_in_it_prints_nothing_and_creates_nothing() {
    let parent = tempfile::tempdir().unwrap();
    let missing = parent.path().join("not-yet");
    assert!(statusline::endpoint_for_home(&missing).is_none());
    assert!(statusline::line_for_home(&missing, None).await.is_none());
    assert!(!missing.exists());

    let empty = tempfile::tempdir().unwrap();
    assert!(
        statusline::line_for_home(empty.path(), None)
            .await
            .is_none()
    );
    assert!(!empty.path().join("control.token").exists());
}

/// The bearer token goes to loopback or nowhere. A pointer naming any other
/// host is not followed.
#[tokio::test]
async fn a_pointer_that_is_not_loopback_is_not_followed() {
    let home = home_pointing_at(1);
    // The pointer names this home's own token, so it is this home's pointer
    // -- and still refused, because it does not name loopback.
    let pointer = serde_json::json!({
        "control_url": "http://example.com:8463",
        "token_path": home.path().join("control.token"),
    });
    std::fs::write(home.path().join("endpoint.json"), pointer.to_string()).unwrap();
    assert!(statusline::endpoint_for_home(home.path()).is_none());
}

/// `~/.ironwire/endpoint.json` is written by `ironwire serve` whatever
/// `$IRONWIRE_HOME` says, so a host sharing `~/.ironwire` can find a pointer
/// there that belongs to a daemon in another home. That pointer names the
/// other home's token; this home's token must not be sent to its port. The
/// line falls back to the port in this home's configuration.
#[tokio::test]
async fn a_pointer_left_by_another_home_is_not_followed() {
    let ours = spawn_daemon().await;
    let theirs = spawn_daemon().await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("control.token"), TOKEN).unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        format!("[server]\nport = {ours}\n"),
    )
    .unwrap();
    let elsewhere = tempfile::tempdir().unwrap();
    let foreign =
        ironwire_core::discovery::Endpoint::new(theirs, elsewhere.path().join("control.token"));
    std::fs::write(
        home.path().join("endpoint.json"),
        serde_json::to_string(&foreign).unwrap(),
    )
    .unwrap();

    assert_eq!(
        statusline::endpoint_for_home(home.path()),
        Some((ours, TOKEN.to_string()))
    );
    assert_eq!(
        statusline::line_for_home(home.path(), None)
            .await
            .as_deref(),
        Some("ironwire → nearai")
    );
}

/// The same home spelled through a symlink is still the same home: the
/// pointer an embedded proxy wrote is followed, not mistaken for a foreign one.
#[cfg(unix)]
#[tokio::test]
async fn a_pointer_naming_this_home_by_another_path_is_followed() {
    let port = spawn_daemon().await;
    let home = home_pointing_at(port);
    let links = tempfile::tempdir().unwrap();
    let alias = links.path().join("alias");
    std::os::unix::fs::symlink(home.path(), &alias).unwrap();
    assert_eq!(
        statusline::endpoint_for_home(&alias),
        Some((port, TOKEN.to_string()))
    );
}

/// The real thing: a proxy embedded on an ephemeral port is found from its
/// home, and answers.
#[tokio::test]
async fn an_embedded_proxy_is_found_from_its_home() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join("config.toml"), "[updates]\ncheck = false\n[[backends]]\nid = 'nearai'\nkind = 'nearai'\nenabled = false\n[[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n[[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n[[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n[[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n").unwrap();
    let proxy = ironwire_proxy::embed::start(home.path(), Some(0))
        .await
        .expect("starts");
    let (port, token) = statusline::endpoint_for_home(home.path()).expect("discovered");
    assert_eq!(port, proxy.port());
    let view = statusline::fetch_status(port, &token)
        .await
        .expect("answers");
    assert_eq!(view.port, proxy.port());
    // Every backend is disabled, so there is nothing serving and nothing to
    // say: quiet, not an error.
    assert!(statusline::line_for_home(home.path(), None).await.is_none());
    proxy.shutdown().await;
}

#[test]
fn the_model_the_harness_asked_for_is_read_from_its_document() {
    assert_eq!(
        statusline::client_model(r#"{"model":{"id":"claude-opus-5"},"new":1}"#).as_deref(),
        Some("claude-opus-5")
    );
    assert!(statusline::client_model("not json").is_none());
    assert!(statusline::client_model(r#"{"session_id":"x"}"#).is_none());
}
