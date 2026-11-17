//! Lifecycle contract for applications hosting their own proxy.

use ironwire_proxy::embed::{EmbedError, start};

fn home() -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    // No update or provider probes in a lifecycle test.
    std::fs::write(home.path().join("config.toml"), "[updates]\ncheck = false\n[[backends]]\nid = 'nearai'\nkind = 'nearai'\nenabled = false\n[[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n[[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n[[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n[[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n").unwrap();
    home
}

#[tokio::test]
async fn a_host_can_start_and_stop_the_proxy_on_an_ephemeral_port() {
    let home = home();
    let proxy = start(home.path(), Some(0)).await.expect("starts");
    let port = proxy.port();
    assert_ne!(port, 0);
    let response = reqwest::get(format!("http://127.0.0.1:{port}/_ironwire/health"))
        .await
        .unwrap();
    assert!(response.status().is_success());
    proxy.shutdown().await;
    assert!(
        tokio::net::TcpListener::bind(("127.0.0.1", port))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn a_second_start_against_the_same_home_is_refused() {
    let home = home();
    let first = start(home.path(), Some(0)).await.expect("starts");
    let second = start(home.path(), Some(0)).await;
    assert!(matches!(second, Err(EmbedError::Lock { .. })));
    first.shutdown().await;
}

#[test]
fn an_empty_home_needs_no_preparation() {
    const CHILD: &str = "IRONWIRE_EMPTY_HOME_TEST";
    if let Some(home) = std::env::var_os(CHILD) {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let home = std::path::Path::new(&home);
                let proxy = start(home, Some(0)).await.expect("empty home starts");
                assert!(home.join("control.token").exists());
                assert!(!home.join("config.toml").exists());
                proxy.shutdown().await;
            });
        return;
    }
    // Isolate credential discovery and background HTTP from the developer's
    // account without mutating process-global environment in parallel tests.
    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "an_empty_home_needs_no_preparation",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, home.path())
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CODEX_HOME", home.path())
        .env("PATH", "")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "127.0.0.1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[tokio::test]
async fn a_busy_port_refuses_without_stealing_another_homes_lock() {
    let first_home = home();
    let first = start(first_home.path(), Some(0)).await.unwrap();
    let second_home = home();
    assert!(matches!(
        start(second_home.path(), Some(first.port())).await,
        Err(EmbedError::PortInUse { .. })
    ));
    assert!(!second_home.path().join("endpoint.json").exists());
    let second = start(second_home.path(), Some(0))
        .await
        .expect("failed start released ownership");
    second.shutdown().await;
    first.shutdown().await;
}

#[tokio::test]
async fn home_ownership_is_atomic_during_concurrent_starts() {
    let home = home();
    let (a, b) = tokio::join!(start(home.path(), Some(0)), start(home.path(), Some(0)));
    match (a, b) {
        (Ok(proxy), Err(EmbedError::Lock { .. })) | (Err(EmbedError::Lock { .. }), Ok(proxy)) => {
            proxy.shutdown().await
        }
        _ => panic!("exactly one concurrent start must own the home"),
    }
}

#[tokio::test]
async fn shutdown_releases_ownership_and_preserves_the_control_token() {
    let home = home();
    let first = start(home.path(), Some(0)).await.unwrap();
    let token = std::fs::read(home.path().join("control.token")).unwrap();
    let endpoint =
        ironwire_core::discovery::Endpoint::read_from(&home.path().join("endpoint.json")).unwrap();
    assert_eq!(
        endpoint.control_url,
        format!("http://127.0.0.1:{}", first.port())
    );
    first.shutdown().await;
    assert!(!home.path().join("endpoint.json").exists());
    assert!(!home.path().join("daemon.lock").exists());
    let second = start(home.path(), Some(0)).await.unwrap();
    assert_eq!(
        std::fs::read(home.path().join("control.token")).unwrap(),
        token
    );
    second.shutdown().await;
}

#[tokio::test]
async fn dropping_the_handle_requests_cleanup_without_releasing_ownership_early() {
    let home = home();
    let proxy = start(home.path(), Some(0)).await.unwrap();
    drop(proxy);
    tokio::time::timeout(std::time::Duration::from_secs(3), async {
        loop {
            match start(home.path(), Some(0)).await {
                Ok(proxy) => {
                    proxy.shutdown().await;
                    break;
                }
                Err(EmbedError::Lock { .. }) => tokio::task::yield_now().await,
                Err(error) => panic!("unexpected restart error: {error}"),
            }
        }
    })
    .await
    .expect("drop drains and releases");
}

#[tokio::test]
async fn invalid_configuration_refuses_before_publishing_or_locking() {
    let home = home();
    std::fs::write(home.path().join("config.toml"), "not valid toml").unwrap();
    assert!(matches!(
        start(home.path(), Some(0)).await,
        Err(EmbedError::Config)
    ));
    assert!(!home.path().join("daemon.lock").exists());
    assert!(!home.path().join("control.token").exists());
}

#[cfg(unix)]
#[tokio::test]
async fn the_home_and_token_are_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let home = home();
    let proxy = start(home.path(), Some(0)).await.unwrap();
    assert_eq!(
        std::fs::metadata(home.path()).unwrap().permissions().mode() & 0o777,
        0o700
    );
    assert_eq!(
        std::fs::metadata(home.path().join("control.token"))
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    proxy.shutdown().await;
}

#[tokio::test]
async fn shutdown_drains_the_inflight_response_before_releasing_the_home() {
    use axum::{
        Router,
        body::Body,
        routing::{get, post},
    };
    use std::sync::Arc;
    use tokio::sync::Notify;
    let release = Arc::new(Notify::new());
    let gate = release.clone();
    let upstream = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .unwrap();
    let upstream_port = upstream.local_addr().unwrap().port();
    let upstream_task = tokio::spawn(async move {
        let app = Router::new()
            .route("/v1/models", get(|| async { axum::Json(serde_json::json!({"data": []})) }))
            .route("/v1/chat/completions", post(move || {
                let gate = gate.clone();
                async move {
                    axum::response::Response::builder().header("content-type", "text/event-stream")
                        .body(Body::from_stream(async_stream::stream! {
                            yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"first\"}}]}\n\n"));
                            gate.notified().await;
                            yield Ok(bytes::Bytes::from_static(b"data: [DONE]\n\n"));
                        })).unwrap()
                }
            }));
        axum::serve(upstream, app).await.unwrap();
    });
    let home = home();
    let config_path = home.path().join("config.toml");
    let mut config = std::fs::read_to_string(&config_path).unwrap();
    config.push_str(&format!("\n[[backends]]\nid = 'test-local'\nkind = 'local'\nbase_url = 'http://127.0.0.1:{upstream_port}/v1'\nmodels = [{{ name = 'test-model', tier = 'frontier' }}]\n"));
    std::fs::write(&config_path, config).unwrap();
    let proxy = start(home.path(), Some(0)).await.unwrap();
    let response = reqwest::Client::new().post(format!("http://127.0.0.1:{}/openai/v1/chat/completions", proxy.port()))
        .header("X-IronWire-Route", "test-local")
        .json(&serde_json::json!({"model":"test-model", "stream":true,"messages":[{"role":"user","content":"fixture"}]}))
        .send().await.unwrap();
    assert!(response.status().is_success());
    let mut body = response;
    assert!(body.chunk().await.unwrap().is_some());
    let stop = tokio::spawn(proxy.shutdown());
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(
        !stop.is_finished(),
        "shutdown must drain the outstanding stream"
    );
    assert!(matches!(
        start(home.path(), Some(0)).await,
        Err(EmbedError::Lock { .. })
    ));
    release.notify_one();
    assert!(body.text().await.unwrap().contains("[DONE]"));
    tokio::time::timeout(std::time::Duration::from_secs(3), stop)
        .await
        .unwrap()
        .unwrap();
    assert!(!home.path().join("daemon.lock").exists());
    upstream_task.abort();
    let _ = upstream_task.await;
}

#[tokio::test]
async fn spend_limits_without_capture_refuse_before_creating_a_token() {
    let home = home();
    std::fs::write(
        home.path().join("config.toml"),
        "[capture]\nenabled = false\n[limits]\ndaily_spend_usd = 1.0\n",
    )
    .unwrap();
    assert!(matches!(
        start(home.path(), Some(0)).await,
        Err(EmbedError::Config)
    ));
    assert!(!home.path().join("control.token").exists());
}

#[tokio::test]
async fn stale_legacy_lock_files_do_not_block_a_restart() {
    let home = home();
    std::fs::write(home.path().join("daemon.lock"), "1\n").unwrap();
    let proxy = start(home.path(), Some(0)).await.unwrap();
    proxy.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn startup_announcement_finishes_before_health_can_report_readiness() {
    use std::io::{Read, Write};
    let home = home();
    let mut announced = false;
    let proxy = ironwire_proxy::embed::start_with(home.path(), Some(0), |port, _| {
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        client
            .set_read_timeout(Some(std::time::Duration::from_millis(100)))
            .unwrap();
        client
            .write_all(
                b"GET /_ironwire/health HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .unwrap();
        let error = client
            .read(&mut [0u8; 1])
            .expect_err("health must not answer before announcement completes");
        assert!(matches!(
            error.kind(),
            std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
        ));
        announced = true;
    })
    .await
    .unwrap();
    assert!(announced);
    assert!(
        reqwest::get(format!(
            "http://127.0.0.1:{}/_ironwire/health",
            proxy.port()
        ))
        .await
        .unwrap()
        .status()
        .is_success()
    );
    proxy.shutdown().await;
}

#[cfg(unix)]
#[tokio::test]
async fn startup_reports_the_canonical_home_for_discovery() {
    let home = home();
    let alias_dir = tempfile::tempdir().unwrap();
    let alias = alias_dir.path().join("home");
    std::os::unix::fs::symlink(home.path(), &alias).unwrap();
    let proxy = ironwire_proxy::embed::start_with(&alias, Some(0), |_, report| {
        assert_eq!(report.home, std::fs::canonicalize(home.path()).unwrap());
    })
    .await
    .unwrap();
    proxy.shutdown().await;
}

#[tokio::test]
async fn embedded_hosts_ignore_standalone_upgrade_commands_even_with_checks_enabled() {
    use ironwire_update::{CheckedAt, UpdateStatus};
    let home = home();
    let config_path = home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)
        .unwrap()
        .replace("check = false", "check = true");
    std::fs::write(config_path, config).unwrap();
    let paths = ironwire_core::config::PathsConfig::rooted_at(home.path().to_owned());
    let cache = paths.update_cache_file();
    ironwire_update::save_cache(
        &cache,
        &CheckedAt {
            at: chrono::Utc::now(),
            status: UpdateStatus::Available {
                latest: "99.0.0".to_owned(),
                summary: None,
                upgrade_command: Some("brew upgrade ironwire".to_owned()),
            },
        },
    )
    .unwrap();
    let before = std::fs::read(&cache).unwrap();
    for with_announcement in [false, true] {
        let proxy = if with_announcement {
            ironwire_proxy::embed::start_with(home.path(), Some(0), |_, _| {})
                .await
                .unwrap()
        } else {
            start(home.path(), Some(0)).await.unwrap()
        };
        let token = std::fs::read_to_string(paths.control_token_file()).unwrap();
        let status: serde_json::Value = reqwest::Client::new()
            .get(format!(
                "http://127.0.0.1:{}/_ironwire/status",
                proxy.port()
            ))
            .bearer_auth(token.trim())
            .send()
            .await
            .unwrap()
            .error_for_status()
            .unwrap()
            .json()
            .await
            .unwrap();
        assert_eq!(status["update"], serde_json::json!({"state": "unknown"}));
        proxy.shutdown().await;
        assert_eq!(
            std::fs::read(&cache).unwrap(),
            before,
            "host does not rewrite the CLI's cache"
        );
    }
}

#[tokio::test]
async fn a_host_can_decline_update_checks_without_writing_the_home_configuration() {
    use ironwire_proxy::embed::{EmbedOptions, UpdateChecks, start_with_options};
    let home = home();
    let config_path = home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)
        .unwrap()
        .replace("check = false", "check = true");
    std::fs::write(&config_path, &config).unwrap();

    let mut observed = None;
    let proxy = start_with_options(
        home.path(),
        Some(0),
        EmbedOptions::default().with_update_checks(UpdateChecks::Off),
        |_, report| observed = Some(report.update_checks),
    )
    .await
    .expect("starts");
    assert_eq!(
        observed,
        Some(false),
        "a declining host makes no release or catalog request"
    );
    assert!(!proxy.startup_report().update_checks);
    proxy.shutdown().await;

    assert_eq!(
        std::fs::read_to_string(&config_path).unwrap(),
        config,
        "declining is expressed in code, never by editing the host's home"
    );
}

#[tokio::test]
async fn a_host_that_does_not_decline_still_follows_the_configuration() {
    use ironwire_proxy::embed::{EmbedOptions, start_with_options};
    let home = home();
    let config_path = home.path().join("config.toml");
    let config = std::fs::read_to_string(&config_path)
        .unwrap()
        .replace("check = false", "check = true");
    std::fs::write(&config_path, &config).unwrap();

    let proxy = start_with_options(home.path(), Some(0), EmbedOptions::default(), |_, _| {})
        .await
        .expect("starts");
    assert!(
        proxy.startup_report().update_checks,
        "the default must stay transparent for standalone users"
    );
    proxy.shutdown().await;
}

/// A backend that records that it was asked for its model catalogue.
async fn spawn_probe_recorder() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    let probes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = std::sync::Arc::clone(&probes);
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let counter = std::sync::Arc::clone(&counter);
            tokio::spawn(async move {
                let mut chunk = [0u8; 8192];
                let read = socket.read(&mut chunk).await.unwrap_or(0);
                if String::from_utf8_lossy(&chunk[..read]).contains("/models") {
                    counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                }
                let body = "{\"data\":[]}";
                let head = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                    body.len()
                );
                let _ = socket.write_all(head.as_bytes()).await;
                let _ = socket.write_all(body.as_bytes()).await;
                let _ = socket.flush().await;
            });
        }
    });
    (format!("http://{addr}"), probes)
}

/// The edge of what `UpdateChecks` covers, pinned so the documentation cannot
/// quietly grow past it. Declining stops IronWire's own two requests. It does
/// not stop the startup catalogue probe, which is a request to a backend - and
/// a backend can be registered without the host naming it, so "the host
/// controls every request" would be a promise this option does not keep.
#[tokio::test]
async fn declining_update_checks_does_not_stop_the_startup_backend_probe() {
    use ironwire_proxy::embed::{EmbedOptions, UpdateChecks, start_with_options};
    let (base_url, probes) = spawn_probe_recorder().await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[updates]\ncheck = true\n\
             [[backends]]\nid = 'nearai'\nkind = 'nearai'\nbase_url = '{base_url}'\n\
             [[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n\
             [[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n\
             [[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n\
             [[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n"
        ),
    )
    .unwrap();

    let proxy = start_with_options(
        home.path(),
        Some(0),
        EmbedOptions::default().with_update_checks(UpdateChecks::Off),
        |_, report| assert!(!report.update_checks),
    )
    .await
    .expect("starts");

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while probes.load(std::sync::atomic::Ordering::SeqCst) == 0
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    proxy.shutdown().await;

    assert_eq!(
        probes.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the startup probe is outside this switch; say so rather than implying otherwise"
    );
}

/// A home whose only registered backend is NEAR AI, pointed at `base_url` by an
/// entry that names it.
fn home_naming_nearai(base_url: &str) -> tempfile::TempDir {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        format!(
            "[updates]\ncheck = false\n\
             [[backends]]\nid = 'nearai'\nkind = 'nearai'\nbase_url = '{base_url}'\n\
             [[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n\
             [[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n\
             [[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n\
             [[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n"
        ),
    )
    .unwrap();
    home
}

/// Start, give a probe that was going to happen time to happen, and stop.
async fn start_and_settle(
    home: &std::path::Path,
    probes: ironwire_proxy::embed::StartupProbes,
    seen: &std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    use ironwire_proxy::embed::{EmbedOptions, start_with_options};
    let proxy = start_with_options(
        home,
        Some(0),
        EmbedOptions::default().with_startup_probes(probes),
        |_, _| {},
    )
    .await
    .expect("starts");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while seen.load(std::sync::atomic::Ordering::SeqCst) == 0
        && std::time::Instant::now() < deadline
    {
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    proxy.shutdown().await;
}

/// A host can start without IronWire asking a provider anything.
///
/// `All` runs first as the control: same recorder, same wait, so the zero that
/// follows is an absence rather than an impatient test.
#[tokio::test]
async fn a_host_can_decline_the_startup_probe_entirely() {
    use ironwire_proxy::embed::StartupProbes;
    let (base_url, probed) = spawn_probe_recorder().await;

    let probing = home_naming_nearai(&base_url);
    start_and_settle(probing.path(), StartupProbes::All, &probed).await;
    assert_eq!(
        probed.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "the default still probes, and the recorder sees it"
    );

    probed.store(0, std::sync::atomic::Ordering::SeqCst);
    let declining = home_naming_nearai(&base_url);
    start_and_settle(declining.path(), StartupProbes::Off, &probed).await;
    assert_eq!(
        probed.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "declining means no request, not a later one"
    );
}

/// The middle position keeps the answer for a backend the host declared.
#[tokio::test]
async fn a_backend_the_configuration_names_is_still_probed() {
    use ironwire_proxy::embed::StartupProbes;
    let (base_url, probed) = spawn_probe_recorder().await;
    let home = home_naming_nearai(&base_url);
    start_and_settle(home.path(), StartupProbes::Configured, &probed).await;
    assert_eq!(
        probed.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "a host that declared this backend keeps the startup answer for it"
    );
}

/// The case the option exists for. NEAR AI is registered whether or not any
/// entry names it, so in a home that does not name it this probe is one the
/// host never asked for.
///
/// In a child process because the unnamed backend can only be pointed at the
/// recorder through the environment, which is process-global. The child runs
/// twice: once declining and once not, so the zero is measured against a one
/// from the same wiring.
#[test]
fn a_backend_no_configuration_names_is_not_probed() {
    const CHILD: &str = "IRONWIRE_UNNAMED_PROBE_TEST";
    if let Some(mode) = std::env::var_os(CHILD) {
        use ironwire_proxy::embed::{EmbedOptions, StartupProbes, start_with_options};
        let probes = if mode == "configured" {
            StartupProbes::Configured
        } else {
            StartupProbes::All
        };
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let home = tempfile::tempdir().unwrap();
                // Every backend discovery could produce is off. NEAR AI has no
                // entry at all - and is registered regardless, which is the
                // point.
                std::fs::write(
                    home.path().join("config.toml"),
                    "[updates]\ncheck = false\n\
                     [[backends]]\nid = 'claude-sub'\nkind = 'claude-subscription'\nenabled = false\n\
                     [[backends]]\nid = 'codex-sub'\nkind = 'codex-subscription'\nenabled = false\n\
                     [[backends]]\nid = 'anthropic-key'\nkind = 'anthropic-api'\nenabled = false\n\
                     [[backends]]\nid = 'openai-key'\nkind = 'openai-api'\nenabled = false\n",
                )
                .unwrap();
                let proxy = start_with_options(
                    home.path(),
                    Some(0),
                    EmbedOptions::default().with_startup_probes(probes),
                    |_, _| {},
                )
                .await
                .expect("starts");
                tokio::time::sleep(std::time::Duration::from_millis(750)).await;
                proxy.shutdown().await;
            });
        return;
    }

    // The recorder must outlive both children, so the runtime that owns it is
    // held for the whole test rather than block_on'd and dropped.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let (base_url, probed) = runtime.block_on(spawn_probe_recorder());

    let run = |mode: &str| {
        let home = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "a_backend_no_configuration_names_is_not_probed",
                "--nocapture",
            ])
            .env_clear()
            .env(CHILD, mode)
            .env("HOME", home.path())
            .env("USERPROFILE", home.path())
            .env("CODEX_HOME", home.path())
            .env("PATH", "")
            .env("IRONWIRE_NEARAI_BASE_URL", &base_url)
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "127.0.0.1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        probed.swap(0, std::sync::atomic::Ordering::SeqCst)
    };

    assert_eq!(
        run("configured"),
        0,
        "a backend no entry names is one the host never asked us to probe"
    );
    assert_eq!(
        run("all"),
        1,
        "and the same start does probe it under the default, so the zero means something"
    );
}

/// The backend list a running daemon reports.
async fn backends_reported_by(home: &std::path::Path, port: u16) -> Vec<serde_json::Value> {
    let paths = ironwire_core::config::PathsConfig::rooted_at(home.to_owned());
    let token = std::fs::read_to_string(paths.control_token_file()).unwrap();
    let status: serde_json::Value = reqwest::Client::new()
        .get(format!("http://127.0.0.1:{port}/_ironwire/status"))
        .bearer_auth(token.trim())
        .send()
        .await
        .unwrap()
        .error_for_status()
        .unwrap()
        .json()
        .await
        .unwrap();
    status["backends"]
        .as_array()
        .expect("a backend list")
        .clone()
}

/// A host with a credential it will not put in the process environment.
///
/// The whole point is that this never runs `set_var`: the key exists only in
/// this test's own memory, and the daemon still finds it. `api_key_env` names a
/// variable that is deliberately absent from the process, so a backend built at
/// all is a backend built from the host's answer.
#[tokio::test]
async fn a_host_supplies_a_backend_credential_without_the_process_environment() {
    use ironwire_proxy::embed::{EmbedOptions, StartupProbes, start_with_options};
    use secrecy::SecretString;

    const NAME: &str = "IRONWIRE_TEST_VENDOR_KEY";
    assert!(
        std::env::var_os(NAME).is_none(),
        "the point of the test is a name the environment cannot answer"
    );

    for supplied in [false, true] {
        let home = home();
        let config_path = home.path().join("config.toml");
        let mut config = std::fs::read_to_string(&config_path).unwrap();
        config.push_str(&format!(
            "[[backends]]\nid = 'vendor'\nkind = 'openai-compatible'\nbase_url = 'http://127.0.0.1:1/v1'\napi_key_env = '{NAME}'\n"
        ));
        std::fs::write(&config_path, &config).unwrap();

        let options = EmbedOptions::default().with_startup_probes(StartupProbes::Off);
        let options = if supplied {
            options.with_credentials(|name: &str| {
                (name == NAME).then(|| SecretString::from("sk-from-host".to_string()))
            })
        } else {
            options
        };
        let proxy = start_with_options(home.path(), Some(0), options, |_, _| {})
            .await
            .expect("starts");

        let vendor = backends_reported_by(home.path(), proxy.port())
            .await
            .into_iter()
            .find(|backend| backend["id"] == "vendor");
        proxy.shutdown().await;

        match supplied {
            true => assert_eq!(
                vendor.expect("the host's credential built the backend")["authenticated"],
                serde_json::json!(true)
            ),
            false => assert!(
                vendor.is_none(),
                "with no host source and no variable, there is no credential and no backend"
            ),
        }
    }
}

/// A host that owns credentials owns all of them, including the ones that come
/// from files rather than variables.
///
/// This home does not switch the subscription backends off, so on a developer
/// machine logged into Claude Code or Codex the default start registers them.
/// A host-owned start must register neither: a fresh user's request going to a
/// subscription they never chose, because the daemon found a login on disk, is
/// the failure this option exists to prevent.
#[tokio::test]
async fn a_host_that_owns_credentials_registers_no_file_discovered_backend() {
    use ironwire_proxy::embed::{
        EmbedOptions, HostSecret, StartupProbes, UpdateChecks, start_with_options,
    };

    let home = tempfile::tempdir().unwrap();
    // Only NEAR AI is switched off, so that the registry can be empty; the
    // subscription entries are deliberately left alone.
    std::fs::write(
        home.path().join("config.toml"),
        "[updates]\ncheck = false\n[[backends]]\nid = 'nearai'\nkind = 'nearai'\nenabled = false\n",
    )
    .unwrap();

    let proxy = start_with_options(
        home.path(),
        Some(0),
        EmbedOptions::default()
            .with_update_checks(UpdateChecks::Off)
            .with_startup_probes(StartupProbes::Off)
            .with_credentials(|_: &str| None::<HostSecret>),
        |_, _| {},
    )
    .await
    .expect("starts");

    let backends = backends_reported_by(home.path(), proxy.port()).await;
    assert!(
        backends.is_empty(),
        "a host that answers nothing has no backends, not the ones IronWire found for itself"
    );
    assert!(
        proxy.startup_report().no_backends,
        "and the report says so, which is what that field is for"
    );
    proxy.shutdown().await;
}

/// The credential that is neither a variable nor a subscription: the key Codex
/// stores after `codex login --api-key`, read from `auth.json` and reached
/// through an `.or_else` behind the environment lookup.
///
/// Run in a child process with `CODEX_HOME` planted, because that is the only
/// honest way to have a stored key without mutating this process's environment
/// while other tests read it — the same device the empty-home test uses.
#[test]
fn a_host_that_owns_credentials_is_not_offered_the_codex_stored_key() {
    const CHILD: &str = "IRONWIRE_CODEX_STORED_KEY_TEST";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use ironwire_proxy::embed::{
                    EmbedOptions, HostSecret, StartupProbes, UpdateChecks, start_with_options,
                };
                let options = || {
                    EmbedOptions::default()
                        .with_update_checks(UpdateChecks::Off)
                        .with_startup_probes(StartupProbes::Off)
                };
                for host_owned in [false, true] {
                    let home = tempfile::tempdir().unwrap();
                    let options = if host_owned {
                        options().with_credentials(|_: &str| None::<HostSecret>)
                    } else {
                        options()
                    };
                    let proxy = start_with_options(home.path(), Some(0), options, |_, _| {})
                        .await
                        .expect("starts");
                    let openai = backends_reported_by(home.path(), proxy.port())
                        .await
                        .into_iter()
                        .find(|backend| backend["id"] == "openai-key");
                    proxy.shutdown().await;
                    match host_owned {
                        false => assert_eq!(
                            openai.expect("the stored key is found by default")["authenticated"],
                            serde_json::json!(true),
                        ),
                        true => assert!(
                            openai.is_none(),
                            "a host that owns credentials is not handed a key off Codex's disk"
                        ),
                    }
                }
            });
        return;
    }

    let codex_home = tempfile::tempdir().unwrap();
    std::fs::write(
        codex_home.path().join("auth.json"),
        r#"{"auth_mode": "apiKey", "OPENAI_API_KEY": "sk-proj-EXAMPLE"}"#,
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_host_that_owns_credentials_is_not_offered_the_codex_stored_key",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("CODEX_HOME", codex_home.path())
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A host that owns the names can now leave the credential files readable, and
/// the two answers do not move together.
///
/// Run in a child process with `CODEX_HOME` and `ANTHROPIC_API_KEY` planted,
/// because both halves of this need a credential that actually exists and
/// neither may depend on how the developer running the suite happens to be
/// logged in. A test that asserted "the subscription is registered" against a
/// real login would pass on one machine and be vacuous on the next; this one
/// plants a ChatGPT-mode `auth.json` and so means the same thing everywhere.
/// The planted variable is the other half: it proves the environment is still
/// not read behind a host, rather than that it happened to be empty.
#[test]
fn a_host_can_own_credentials_and_still_permit_the_credential_files() {
    const CHILD: &str = "IRONWIRE_CREDENTIAL_FILES_TEST";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(child());
        return;
    }

    // A ChatGPT-mode login, which is a file and not a key: no name-keyed
    // source, host or environment, can supply it.
    let codex_home = tempfile::tempdir().unwrap();
    std::fs::write(
        codex_home.path().join("auth.json"),
        r#"{"auth_mode": "chatgpt", "tokens": {"access_token": "eyJACCESS", "refresh_token": "rt.1.EXAMPLE", "account_id": "36afe797-0000"}}"#,
    )
    .unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_host_can_own_credentials_and_still_permit_the_credential_files",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("CODEX_HOME", codex_home.path())
        .env("ANTHROPIC_API_KEY", "sk-ant-from-the-environment")
        .env_remove("NEARAI_API_KEY")
        .env_remove("OPENAI_API_KEY")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );

    async fn child() {
        use ironwire_proxy::embed::{
            CredentialFiles, EmbedOptions, HostSecret, StartupProbes, UpdateChecks,
            start_with_options,
        };

        // Every case: no outbound request, and a home that leaves every
        // discovered backend alone so registration is the thing under test.
        let base = || {
            EmbedOptions::default()
                .with_update_checks(UpdateChecks::Off)
                .with_startup_probes(StartupProbes::Off)
        };
        let host = |name: &str| {
            (name == "NEARAI_API_KEY").then(|| HostSecret::from("sk-nearai-from-host"))
        };

        // What each case expects: the subscription that lives in a file, the
        // key that lives in the process environment, and the name only the
        // host answers for.
        struct Case {
            what: &'static str,
            options: EmbedOptions,
            subscription: bool,
            from_environment: bool,
            from_host: bool,
        }
        let cases = vec![
            Case {
                what: "a host that owns credentials and says nothing about files",
                options: base().with_credentials(host),
                subscription: false,
                from_environment: false,
                from_host: true,
            },
            Case {
                what: "a host that owns credentials and opts into the files",
                options: base()
                    .with_credentials(host)
                    .with_credential_files(CredentialFiles::Discover),
                subscription: true,
                from_environment: false,
                from_host: true,
            },
            Case {
                what: "no host source at all",
                options: base(),
                subscription: true,
                from_environment: true,
                from_host: false,
            },
            Case {
                what: "no host source, files declined",
                options: base().with_credential_files(CredentialFiles::Off),
                subscription: false,
                from_environment: true,
                from_host: false,
            },
        ];

        for Case {
            what,
            options,
            subscription,
            from_environment,
            from_host,
        } in cases
        {
            let home = tempfile::tempdir().unwrap();
            std::fs::write(
                home.path().join("config.toml"),
                "[updates]\ncheck = false\n",
            )
            .unwrap();
            let proxy = start_with_options(home.path(), Some(0), options, |_, _| {})
                .await
                .expect("starts");
            let backends = backends_reported_by(home.path(), proxy.port()).await;
            proxy.shutdown().await;
            let registered = |id: &str| backends.iter().any(|backend| backend["id"] == id);
            let authenticated = |id: &str| {
                backends
                    .iter()
                    .find(|backend| backend["id"] == id)
                    .is_some_and(|backend| backend["authenticated"] == serde_json::json!(true))
            };

            assert_eq!(
                registered("codex-sub"),
                subscription,
                "{what}: the ChatGPT subscription on disk"
            );
            assert_eq!(
                registered("anthropic-key"),
                from_environment,
                "{what}: the key in the process environment"
            );
            assert_eq!(
                authenticated("nearai"),
                from_host,
                "{what}: the name the host answers for"
            );
        }
    }
}

/// A login declared in `config.toml` is registered before anyone has logged in
/// to it, and `status` says what to run — for that login, not the default one.
///
/// In a child process because the Claude half reads the macOS Keychain first:
/// with `PATH` empty there is no `security` to ask, so the answer is the same on
/// every machine and nobody's real Keychain is touched. `HOME` is a temporary
/// directory, so the default logins are absent too.
#[test]
fn a_declared_login_nobody_has_logged_in_to_yet_says_how_to_log_in() {
    const CHILD: &str = "IRONWIRE_DECLARED_LOGIN_TEST";
    if std::env::var_os(CHILD).is_some() {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                use ironwire_proxy::embed::{
                    EmbedOptions, StartupProbes, UpdateChecks, start_with_options,
                };
                let claude_dir = tempfile::tempdir().unwrap();
                let codex_dir = tempfile::tempdir().unwrap();
                let home = tempfile::tempdir().unwrap();
                std::fs::write(
                    home.path().join("config.toml"),
                    format!(
                        "[updates]\ncheck = false\n\
                         [[backends]]\nid = 'nearai'\nkind = 'nearai'\nenabled = false\n\
                         [[backends]]\nid = 'claude-work'\nkind = 'claude-subscription'\n\
                         config_dir = '{}'\n\
                         [[backends]]\nid = 'codex-work'\nkind = 'codex-subscription'\n\
                         config_dir = '{}'\n",
                        claude_dir.path().display(),
                        codex_dir.path().display()
                    ),
                )
                .unwrap();
                let proxy = start_with_options(
                    home.path(),
                    Some(0),
                    EmbedOptions::default()
                        .with_update_checks(UpdateChecks::Off)
                        .with_startup_probes(StartupProbes::Off),
                    |_, _| {},
                )
                .await
                .expect("starts");
                let backends = backends_reported_by(home.path(), proxy.port()).await;
                proxy.shutdown().await;

                let ids: Vec<&str> = backends
                    .iter()
                    .map(|backend| backend["id"].as_str().unwrap())
                    .collect();
                assert_eq!(
                    ids,
                    vec!["claude-work", "codex-work"],
                    "registered though nobody is logged in, and no default login appeared"
                );
                for (id, command) in [
                    ("claude-work", "CLAUDE_CONFIG_DIR="),
                    ("codex-work", "CODEX_HOME="),
                ] {
                    let backend = backends.iter().find(|b| b["id"] == id).unwrap();
                    assert_eq!(backend["kind"], "subscription");
                    assert_eq!(backend["authenticated"], false, "{backend}");
                    let detail = backend["detail"].as_str().unwrap_or_default();
                    assert!(
                        detail.contains(command),
                        "{id} names the command for its own login: {detail}"
                    );
                    assert!(
                        backend["connect_command"]
                            .as_str()
                            .is_some_and(|c| c.contains("--config-dir")),
                        "{backend}"
                    );
                }
            });
        return;
    }

    let home = tempfile::tempdir().unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "a_declared_login_nobody_has_logged_in_to_yet_says_how_to_log_in",
            "--nocapture",
        ])
        .env_clear()
        .env(CHILD, "1")
        .env("HOME", home.path())
        .env("USERPROFILE", home.path())
        .env("CODEX_HOME", home.path())
        .env("PATH", "")
        .env("HTTP_PROXY", "http://127.0.0.1:1")
        .env("HTTPS_PROXY", "http://127.0.0.1:1")
        .env("ALL_PROXY", "http://127.0.0.1:1")
        .env("NO_PROXY", "127.0.0.1")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// An embedded start must be `tokio::spawn`-able, which is to say its future
/// must be `Send`.
///
/// Nothing else in this repository spawns one — the CLI and every other test
/// `.await` a start directly on the thread that made it, and a non-`Send`
/// future is perfectly happy that way. So the bounds that make it `Send` (on
/// `CredentialSource`, and on the internal environment lookup a `Credentials`
/// is built from) can be dropped without a single failure here, and the first
/// thing to notice would be an embedding host's build breaking. This test is
/// the thing that notices: it is a compile-time assertion wearing a runtime
/// test's clothes, and the runtime half is only there to prove the spawn was
/// real.
///
/// A host source is supplied deliberately, because that closure is the part
/// held across an await, and the credential-files knob with it, so the future
/// under test is the one a real embedder spawns rather than the simplest one.
#[tokio::test(flavor = "multi_thread")]
async fn an_embedded_start_can_be_spawned_onto_a_multi_thread_runtime() {
    use ironwire_proxy::embed::{
        CredentialFiles, EmbedOptions, HostSecret, StartupProbes, UpdateChecks, start_with_options,
    };

    let home = home();
    let started = tokio::spawn(async move {
        let proxy = start_with_options(
            home.path(),
            Some(0),
            EmbedOptions::default()
                .with_update_checks(UpdateChecks::Off)
                .with_startup_probes(StartupProbes::Off)
                .with_credentials(|_: &str| None::<HostSecret>)
                .with_credential_files(CredentialFiles::Off),
            |_, _| {},
        )
        .await
        .expect("starts");
        let port = proxy.port();
        proxy.shutdown().await;
        // The home outlives the proxy, and is dropped here rather than on the
        // thread that spawned this.
        drop(home);
        port
    })
    .await
    .expect("the spawned start did not panic");

    assert_ne!(started, 0, "the spawned start bound a real port");
}

/// Shapes `main` accepted must not stop an embedding host's proxy: a login
/// entry under an id of its own with no `config_dir`, and an entry under a
/// discovered id that names another kind. Both are ignored with a warning.
#[tokio::test]
async fn a_host_still_starts_on_config_shapes_main_accepted() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("config.toml"),
        "[updates]\ncheck = false\n\
         [[backends]]\nid = 'claude-work'\nkind = 'claude-subscription'\n\
         [[backends]]\nid = 'codex-work'\nkind = 'codex-subscription'\nenabled = false\n\
         [[backends]]\nid = 'claude-sub'\nkind = 'anthropic-api'\nenabled = false\n\
         [[backends]]\nid = 'nearai'\nkind = 'openai-api'\nenabled = false\n",
    )
    .unwrap();
    let proxy = start(home.path(), Some(0))
        .await
        .expect("a config main accepted still starts");
    proxy.shutdown().await;
}
