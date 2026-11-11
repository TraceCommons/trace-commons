//! Embedding hosts may permit only one blocking credential worker.

use std::sync::Arc;
use std::time::Duration;

use ironwire_core::protocol::BackendId;
use ironwire_creds::ClaudeLogin;
use ironwire_proxy::state::BackendRegistry;
use ironwire_upstream::anthropic::AnthropicBackend;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[test]
fn first_party_status_and_probe_finish_with_one_blocking_worker() {
    let home = tempfile::tempdir().expect("login directory");
    std::fs::write(
        home.path().join(".credentials.json"),
        r#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-INERT"}}"#,
    )
    .expect("credential fixture");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .expect("single blocking worker runtime");
    let completed = runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(10), async {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .expect("mock listener");
            let base = format!("http://{}", listener.local_addr().expect("mock address"));
            let mock = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.expect("probe connection");
                let mut head = Vec::new();
                let mut chunk = [0; 1024];
                while !head.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                    let count = socket.read(&mut chunk).await.expect("probe request");
                    assert!(count > 0, "probe closed before headers");
                    head.extend_from_slice(&chunk[..count]);
                }
                let body = br#"{"data":[]}"#;
                let response = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
                    body.len()
                );
                socket.write_all(response.as_bytes()).await.expect("response head");
                socket.write_all(body).await.expect("response body");
            });
            let login = ClaudeLogin::in_dir(home.path().to_str().expect("login path"))
                .without_keychain();
            let backend = Arc::new(
                AnthropicBackend::subscription_for(
                    BackendId::from("claude-one-worker"),
                    "single-worker login".into(),
                    login,
                    Some(base),
                    30,
                )
                .expect("backend"),
            );
            let mut registry = BackendRegistry::new();
            registry.push(backend.clone());
            let statuses = registry.statuses().await;
            assert_eq!(statuses.len(), 1);
            assert!(statuses[0].authenticated);
            registry.probe(backend).await.expect("probe");
            mock.await.expect("mock finished");
        })
        .await
    });
    // An old nested spawn_blocking implementation must fail the timeout
    // assertion, rather than hang the test while Runtime waits for its worker.
    runtime.shutdown_background();
    assert!(
        completed.is_ok(),
        "registry deadlocked its only blocking worker"
    );
}
