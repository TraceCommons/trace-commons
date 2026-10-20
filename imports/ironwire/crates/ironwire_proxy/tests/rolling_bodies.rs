//! Bodies are held for the current exchange of a session, not for the session.
//!
//! Only the final call of a session is ever attested downstream, so retaining
//! every turn's full prompts and completions for `retain_days` would hold the
//! maximum possible amount of the user's content to buy something nothing
//! uses. Each new exchange releases the previous one's bodies, which collapses
//! the honest description from "IronWire keeps all your prompts for 90 days"
//! to "IronWire holds the current exchange until the next one replaces it".
//!
//! What survives the release is the pair of digests. They are hashes, not
//! content, and they are the thing a provider's receipt is checked against --
//! so a released row reads as "I know what was hashed, I no longer hold the
//! bytes" rather than as a pointer to a file that is gone.

use std::sync::Arc;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use ironwire_core::config::Config;
use ironwire_creds::ConsentLedger;
use ironwire_ledger::Ledger;
use ironwire_ledger::bodies::BodyStore;
use ironwire_proxy::server::app;
use ironwire_proxy::state::{AppState, BackendRegistry};
use ironwire_upstream::anthropic::AnthropicBackend;
use secrecy::SecretString;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tower::ServiceExt;

/// Distinct per turn, so a body that outlived its turn is visible rather than
/// hidden behind identical bytes.
fn reply(turn: usize) -> String {
    format!(
        concat!(
            "{{\"usage\":{{\"input_tokens\":10,\"output_tokens\":2}},\"id\":\"msg_{}\",",
            "\"type\":\"message\",\"role\":\"assistant\",\"model\":\"claude-opus-4-6\",",
            "\"content\":[{{\"type\":\"text\",\"text\":\"turn {} caf\u{e9}\"}}],\"stop_reason\":\"end_turn\"}}"
        ),
        turn, turn
    )
}

/// An upstream that answers `turns` sequential requests.
async fn upstream(turns: usize) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind loopback");
    let addr = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        for turn in 0..turns {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let mut raw = Vec::new();
            let mut chunk = [0u8; 4096];
            loop {
                let Ok(read) = socket.read(&mut chunk).await else {
                    return;
                };
                if read == 0 {
                    break;
                }
                raw.extend_from_slice(&chunk[..read]);
                if let Some(start) = raw.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&raw[..start]).to_ascii_lowercase();
                    let length: usize = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if raw.len() >= start + 4 + length {
                        break;
                    }
                }
            }
            let body = reply(turn);
            let head = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(head.as_bytes()).await;
            let _ = socket.write_all(body.as_bytes()).await;
            let _ = socket.flush().await;
        }
    });
    format!("http://{addr}")
}

fn state_for(base_url: &str, ledger: Ledger, bodies: Arc<BodyStore>) -> AppState {
    let mut registry = BackendRegistry::new();
    registry.push(Arc::new(
        AnthropicBackend::api_key(
            SecretString::from("sk-ant-test-key"),
            Some(base_url.to_string()),
            30,
        )
        .expect("client builds"),
    ));
    AppState::new(
        registry,
        Config::default(),
        ConsentLedger::default(),
        "test-token".to_string(),
    )
    .with_ledger(Some(ledger))
    .with_bodies(Some(bodies))
}

/// One turn through the façade, tagged with a session id the client owns.
async fn turn(state: &AppState, session: &str, n: usize) {
    let body = format!(
        "{{\"messages\":[{{\"role\":\"user\",\"content\":\"turn {n} d\u{e9}j\u{e0}\"}}],  \"max_tokens\":64,\"model\":\"claude-opus-4-6\"}}"
    );
    let request = Request::builder()
        .method("POST")
        .uri("/anthropic/v1/messages")
        .header("content-type", "application/json")
        .header("x-ironwire-session-id", session)
        .body(Body::from(body))
        .expect("request builds");
    let response = app(state.clone()).oneshot(request).await.expect("served");
    assert_eq!(response.status(), StatusCode::OK);
    let _ = axum::body::to_bytes(response.into_body(), usize::MAX).await;
}

/// Files on disk, as pairs.
fn pairs_on_disk(dir: &std::path::Path) -> Vec<String> {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .expect("listable")
        .filter_map(|entry| {
            let name = entry.ok()?.file_name().to_str()?.to_string();
            name.strip_suffix(".req").map(ToString::to_string)
        })
        .collect();
    stems.sort();
    stems
}

#[tokio::test]
async fn a_session_of_many_turns_leaves_exactly_one_exchange_holding_bodies() {
    let base = upstream(6).await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let home = tempfile::tempdir().expect("tempdir");
    let dir = home.path().join("bodies");
    let store = Arc::new(BodyStore::open(&dir).expect("store opens"));
    let state = state_for(&base, ledger.clone(), Arc::clone(&store));

    for n in 0..6 {
        turn(&state, "session-a", n).await;
    }

    assert_eq!(pairs_on_disk(&dir).len(), 1, "one pair of files survives");
    let rows = ledger.recent(10).expect("reads");
    assert_eq!(rows.len(), 6, "every turn is still on the record");
    let holding: Vec<_> = rows.iter().filter(|r| r.body_ref.is_some()).collect();
    assert_eq!(holding.len(), 1, "exactly one row still claims bodies");
    // `recent` is newest first, so the survivor is the final call.
    assert_eq!(
        holding[0].started_at, rows[0].started_at,
        "and it is the last turn, not an arbitrary one"
    );

    // The released turns kept what a receipt is checked against.
    for released in rows.iter().filter(|r| r.body_ref.is_none()) {
        assert!(
            released.request_sha256.is_some() && released.response_sha256.is_some(),
            "a released row still says what was hashed"
        );
        assert!(
            released.upstream_id.is_some(),
            "and how to fetch the receipt"
        );
    }

    // The survivor's files are the ones it names, and hold the last turn.
    let reference = holding[0].body_ref.clone().expect("a reference");
    let (_, response) = store.read(&reference).expect("bodies read back");
    assert_eq!(response, reply(5).as_bytes());
}

#[tokio::test]
async fn one_sessions_rotation_does_not_touch_another_sessions_bodies() {
    let base = upstream(5).await;
    let ledger = Ledger::in_memory().expect("ledger opens");
    let home = tempfile::tempdir().expect("tempdir");
    let dir = home.path().join("bodies");
    let store = Arc::new(BodyStore::open(&dir).expect("store opens"));
    let state = state_for(&base, ledger.clone(), Arc::clone(&store));

    turn(&state, "session-b", 0).await;
    let after_b = pairs_on_disk(&dir);
    assert_eq!(after_b.len(), 1);

    for n in 1..5 {
        turn(&state, "session-a", n).await;
    }

    let remaining = pairs_on_disk(&dir);
    assert_eq!(remaining.len(), 2, "one pair per live session, no more");
    assert!(
        remaining.contains(&after_b[0]),
        "session-b's only turn kept its bodies through session-a's four"
    );
    let rows = ledger.recent(10).expect("reads");
    let b_row = rows
        .iter()
        .find(|r| r.client_session_id.as_deref() == Some("session-b"))
        .expect("session-b's row");
    assert_eq!(b_row.body_ref.as_deref(), Some(after_b[0].as_str()));
}
