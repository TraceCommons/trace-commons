// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Envelopes far above the old 16 MB cap, through the real router.
//!
//! The synthetic envelope is written to a temporary file event by event, so
//! the test process never holds it as a struct: the only copies in memory are
//! the ones ingest makes. That is what lets the ignored probe below measure the
//! server's peak resident set from outside, with `/usr/bin/time -l`.

use super::*;

/// Words for synthetic event text. Ordinary prose: nothing here matches a
/// redaction pattern, so the server's re-scrub does its full scan and changes
/// nothing, as it would for a clean real session.
const WORDS: &[&str] = &[
    "the",
    "build",
    "failed",
    "because",
    "module",
    "was",
    "missing",
    "after",
    "refactor",
    "please",
    "check",
    "workspace",
    "function",
    "returns",
    "error",
    "when",
    "input",
    "empty",
    "test",
    "passes",
    "locally",
    "but",
    "not",
    "in",
    "continuous",
    "integration",
    "runner",
    "rename",
    "variable",
    "parser",
    "handles",
    "nested",
    "arrays",
    "correctly",
    "now",
];

fn synthetic_text(seed: usize, bytes: usize) -> String {
    let mut text = String::with_capacity(bytes + 16);
    let mut index = seed;
    while text.len() < bytes {
        index = index
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        text.push_str(WORDS[(index >> 33) % WORDS.len()]);
        text.push(' ');
    }
    text
}

/// What [`write_synthetic_envelope`] wrote.
pub(super) struct SyntheticEnvelope {
    pub(super) submission_id: Uuid,
    pub(super) bytes: u64,
    pub(super) events: usize,
    pub(super) string_leaves: usize,
}

/// Write an envelope of about `target_bytes` to `path`, one event at a time.
///
/// Leaf density is set to look like a tool-heavy coding session rather than
/// one giant string: each event carries about 3 KB of message text, and every
/// other event a structured payload with five more string leaves.
pub(super) async fn write_synthetic_envelope(path: &Path, target_bytes: u64) -> SyntheticEnvelope {
    let mut base = sample_envelope().await;
    let template = base.events[0].clone();
    base.events.clear();
    let submission_id = base.submission_id;
    let shell = serde_json::to_string(&base).expect("the base envelope serialises");
    let marker = "\"events\":[]";
    let split = shell
        .find(marker)
        .expect("the envelope serialises an events array")
        + marker.len()
        - 1;
    let (prefix, suffix) = shell.split_at(split);

    let file = std::fs::File::create(path).expect("the fixture file is created");
    let mut out = std::io::BufWriter::with_capacity(1 << 20, file);
    out.write_all(prefix.as_bytes()).expect("prefix writes");
    let mut written = (prefix.len() + suffix.len()) as u64;
    let mut events = 0usize;
    let mut string_leaves = 0usize;
    while written < target_bytes {
        let mut event = template.clone();
        event.event_id =
            Uuid::from_u128(0x5eed_0000_0000_0000_0000_0000_0000_0000 + events as u128);
        event.redacted_content = Some(synthetic_text(events, 3 * 1024));
        string_leaves += 1;
        if events.is_multiple_of(2) {
            event.structured_payload = serde_json::json!({
                "command": synthetic_text(events + 1, 40),
                "args": [synthetic_text(events + 2, 24), synthetic_text(events + 3, 24)],
                "output": {"stdout": synthetic_text(events + 4, 900), "exit_code": 0},
                "cwd": "workspace",
            });
            string_leaves += 5;
        }
        let encoded = serde_json::to_vec(&event).expect("an event serialises");
        if events > 0 {
            out.write_all(b",").expect("separator writes");
            written += 1;
        }
        out.write_all(&encoded).expect("event writes");
        written += encoded.len() as u64;
        events += 1;
    }
    out.write_all(suffix.as_bytes()).expect("suffix writes");
    out.flush().expect("fixture flushes");
    drop(out);
    let bytes = std::fs::metadata(path).expect("fixture metadata").len();
    SyntheticEnvelope {
        submission_id,
        bytes,
        events,
        string_leaves,
    }
}

/// POST the file at `path` to `/v1/traces` over loopback, streamed in 1 MiB
/// chunks with an exact `Content-Length`, and return the status code and the
/// response body. The client side never holds more than one chunk.
pub(super) async fn post_envelope_file(
    addr: std::net::SocketAddr,
    token: &str,
    path: &Path,
) -> (u16, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let length = std::fs::metadata(path).expect("fixture metadata").len();
    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connects to the test server");
    stream
        .write_all(
            format!(
                "POST /v1/traces HTTP/1.1\r\nHost: ingest.test\r\nAuthorization: Bearer {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {length}\r\n\
                 Connection: close\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("request headers write");
    let mut file = tokio::fs::File::open(path).await.expect("fixture opens");
    let mut chunk = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut chunk).await.expect("fixture reads");
        if read == 0 {
            break;
        }
        // The server may answer early (a refusal before the body is read) and
        // close; the response is still read below.
        if stream.write_all(&chunk[..read]).await.is_err() {
            break;
        }
    }
    let mut response = Vec::new();
    let _ = stream.read_to_end(&mut response).await;
    let response = String::from_utf8_lossy(&response).to_string();
    let status = response
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or(0);
    let body = response
        .split_once("\r\n\r\n")
        .map(|(_, body)| body.to_string())
        .unwrap_or_default();
    (status, body)
}

fn large_envelope_test_state(root: &Path) -> Arc<AppState> {
    let store = test_artifact_store(root);
    let mut state = test_state_with_options(
        root.to_path_buf(),
        None,
        Some(store),
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::get_mut(&mut state).expect("fresh state");
    // As the pilot runs: the encrypted object is the only copy of the body.
    state_mut.object_primary_submit_review = true;
    state
}

/// Submit an envelope of `target_bytes` and, when `read_back`, read it back
/// from the store. Returns what was written, the submit time, and the
/// receipt's status.
async fn round_trip(
    target_bytes: u64,
    read_back: bool,
) -> (SyntheticEnvelope, StdDuration, String) {
    reset_account_rate_limiter_for_test();
    let temp = tempfile::tempdir().expect("temp dir");
    let fixture = temp.path().join("envelope.json");
    let written = write_synthetic_envelope(&fixture, target_bytes).await;
    let state = large_envelope_test_state(&temp.path().join("root"));
    let (addr, server) = serve_ingest_for_test(state.clone()).await;
    let started = std::time::Instant::now();
    let (status, body) = post_envelope_file(addr, "token-a", &fixture).await;
    let elapsed = started.elapsed();
    server.abort();
    assert_eq!(
        status, 200,
        "a {} byte envelope is accepted: {body}",
        written.bytes
    );
    let receipt: serde_json::Value = serde_json::from_str(&body).expect("the receipt is JSON");
    let record =
        read_submission_record(&temp.path().join("root"), "tenant-a", written.submission_id)
            .expect("the record reads")
            .expect("the record exists");
    // The status, and why: a residual scan that ran out of budget is its own
    // basis label.
    let receipt_status = format!(
        "{} basis={:?}",
        receipt["status"].as_str().unwrap_or_default(),
        record.residual_risk_basis
    );
    if read_back {
        let stored =
            read_envelope_by_record(state.as_ref(), &record).expect("the envelope reads back");
        assert_eq!(stored.events.len(), written.events);
    }
    (written, elapsed, receipt_status)
}

/// Above the old 16 MB cap, in the default run: a 24 MB envelope is accepted
/// and reads back whole.
#[tokio::test]
async fn an_envelope_above_the_old_cap_round_trips() {
    round_trip(24_000_000, true).await;
}

/// Manual probe, for the peak resident set of one large submission:
///
/// ```text
/// TC_LARGE_ENVELOPE_BYTES=200000000 /usr/bin/time -l \
///   target/debug/deps/trace_commons_ingest-<hash> \
///   ingest::app::tests::large_envelope_tests::large_envelope_submit_probe \
///   --exact --ignored --nocapture
/// ```
///
/// Ignored: a 500 MB run needs several GB of memory and minutes in a debug
/// build. Not selected by any CI filter.
#[tokio::test]
#[ignore = "manual memory probe; set TC_LARGE_ENVELOPE_BYTES"]
async fn large_envelope_submit_probe() {
    let target: u64 = std::env::var("TC_LARGE_ENVELOPE_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200_000_000);
    // `TC_LARGE_ENVELOPE_READ_BACK=0` measures the submission alone; the
    // read-back is what a downstream worker reading the stored envelope does.
    let read_back = std::env::var("TC_LARGE_ENVELOPE_READ_BACK").as_deref() != Ok("0");
    let started = std::time::Instant::now();
    let (written, elapsed, status) = round_trip(target, read_back).await;
    eprintln!(
        "large-envelope-probe bytes={} events={} string_leaves={} submit_secs={:.1} \
         total_secs={:.1} status={status} read_back={read_back}",
        written.bytes,
        written.events,
        written.string_leaves,
        elapsed.as_secs_f64(),
        started.elapsed().as_secs_f64()
    );
}

/// Send an upload's headers declaring `content_length` bytes, and none of the
/// body. A request the gates admit waits on its body; one they refuse is
/// answered at once.
async fn send_declared_upload(
    addr: std::net::SocketAddr,
    path: &str,
    token: &str,
    content_length: u64,
) -> tokio::net::TcpStream {
    use tokio::io::AsyncWriteExt;

    let mut stream = tokio::net::TcpStream::connect(addr)
        .await
        .expect("connects to the test server");
    stream
        .write_all(
            format!(
                "POST {path} HTTP/1.1\r\nHost: ingest.test\r\nAuthorization: Bearer {token}\r\n\
                 Content-Type: application/json\r\nContent-Length: {content_length}\r\n\r\n"
            )
            .as_bytes(),
        )
        .await
        .expect("request headers write");
    stream
}

fn state_with_token(root: &Path, token: &str, expires_at: Option<DateTime<Utc>>) -> Arc<AppState> {
    let mut tokens = BTreeMap::new();
    insert_token_with_expiry(
        &mut tokens,
        "tenant-a",
        token,
        TokenRole::Contributor,
        expires_at,
    )
    .expect("token inserts");
    let mut state = test_state(root.to_path_buf());
    Arc::make_mut(&mut state).tokens = Arc::new(tokens);
    state
}

#[test]
fn large_body_weight_is_the_declared_length_within_the_route_ceiling() {
    let headers = |value: Option<&str>| {
        let mut headers = HeaderMap::new();
        if let Some(value) = value {
            headers.insert(
                axum::http::header::CONTENT_LENGTH,
                HeaderValue::from_str(value).unwrap(),
            );
        }
        headers
    };
    let ceiling = MAX_INGEST_BODY_BYTES;
    let attachment = 8 * 1024 * 1024;
    let weight = |value: Option<&str>, hint: Option<u64>, limit: usize| {
        large_body_weight(&headers(value), hint, limit)
    };
    assert_eq!(weight(Some("4096"), None, ceiling), Some(4096));
    assert_eq!(weight(Some("0"), None, ceiling), Some(1));
    let at_ceiling = ceiling.to_string();
    assert_eq!(
        weight(Some(&at_ceiling), None, ceiling),
        Some(ceiling as u32)
    );
    // No length known, or a header that does not parse: the whole ceiling.
    assert_eq!(weight(None, None, ceiling), Some(ceiling as u32));
    assert_eq!(weight(Some("lots"), None, ceiling), Some(ceiling as u32));
    assert_eq!(weight(None, None, attachment), Some(attachment as u32));
    // Without a header, a body that knows its exact length is weighed by it.
    assert_eq!(weight(None, Some(2048), ceiling), Some(2048));
    // The header wins over the hint.
    assert_eq!(weight(Some("10"), Some(2048), ceiling), Some(10));
    // Over the ceiling: refused outright.
    let over = (ceiling + 1).to_string();
    assert_eq!(weight(Some(&over), None, ceiling), None);
    assert_eq!(weight(None, Some(ceiling as u64 + 1), ceiling), None);
    assert_eq!(weight(Some("99999999999999"), None, ceiling), None);
    assert_eq!(weight(Some("8388609"), None, attachment), None);
}

#[test]
fn large_body_byte_budget_parses_within_its_bounds() {
    assert_eq!(
        parse_large_body_byte_budget(None).unwrap(),
        LARGE_BODY_GLOBAL_BYTES
    );
    assert_eq!(
        parse_large_body_byte_budget(Some(&MAX_INGEST_BODY_BYTES.to_string())).unwrap(),
        MAX_INGEST_BODY_BYTES
    );
    for refused in [
        (MAX_INGEST_BODY_BYTES - 1).to_string(),
        (u32::MAX as u64 + 1).to_string(),
        "0".to_string(),
        "lots".to_string(),
    ] {
        let error = parse_large_body_byte_budget(Some(&refused)).unwrap_err();
        assert!(
            error
                .to_string()
                .contains(TRACE_COMMONS_LARGE_BODY_BYTE_BUDGET),
            "a refusal names the variable: {error}"
        );
    }
}

/// Uploads are admitted against a budget of body bytes, not just a count: an
/// upload that does not fit beside the bytes already in flight is refused
/// before its body is read (429 with `Retry-After`), and fits again once the
/// earlier upload is gone.
#[tokio::test]
async fn the_upload_byte_budget_refuses_what_does_not_fit_until_bytes_free() {
    const TOKEN: &str = "byte-budget-token";
    let temp = tempfile::tempdir().expect("temp dir");
    let mut state = state_with_token(temp.path(), TOKEN, None);
    let budget = Arc::new(tokio::sync::Semaphore::new(MAX_INGEST_BODY_BYTES));
    Arc::make_mut(&mut state).large_body_bytes = budget.clone();
    let (addr, server) = serve_ingest_for_test(state).await;

    let big = MAX_INGEST_BODY_BYTES as u64 - 1000;
    let mut holder = send_declared_upload(addr, "/v1/traces", TOKEN, big).await;
    tokio::time::timeout(StdDuration::from_secs(5), async {
        while budget.available_permits() > 1000 {
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
    })
    .await
    .expect("the stalled upload holds its declared bytes");
    assert_eq!(
        response_status_within(&mut holder, StdDuration::from_millis(300)).await,
        None,
        "the first upload fits and waits for its body"
    );

    let mut refused = send_declared_upload(addr, "/v1/traces", TOKEN, 2048).await;
    assert_eq!(
        response_status_within(&mut refused, StdDuration::from_secs(2)).await,
        Some("HTTP/1.1 429 Too Many Requests".to_string()),
        "2048 bytes do not fit beside the stalled upload"
    );

    drop(holder);
    tokio::time::timeout(StdDuration::from_secs(5), async {
        while budget.available_permits() < MAX_INGEST_BODY_BYTES {
            tokio::time::sleep(StdDuration::from_millis(10)).await;
        }
    })
    .await
    .expect("an abandoned upload gives its bytes back");
    let mut admitted = send_declared_upload(addr, "/v1/traces", TOKEN, 2048).await;
    assert_eq!(
        response_status_within(&mut admitted, StdDuration::from_millis(300)).await,
        None,
        "with the bytes free, the same upload is admitted and waits for its body"
    );
    server.abort();
}

/// A credential that expires while its upload's body is still arriving does
/// not turn the upload into a 401 after the server has read all of it: the
/// handler uses the context the pre-body gate authenticated.
#[tokio::test]
async fn an_upload_whose_claim_expires_mid_body_is_still_accepted() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    const TOKEN: &str = "expiring-upload-token";
    reset_account_rate_limiter_for_test();
    let temp = tempfile::tempdir().expect("temp dir");
    let state = state_with_token(
        temp.path(),
        TOKEN,
        Some(Utc::now() + Duration::milliseconds(1500)),
    );
    let (addr, server) = serve_ingest_for_test(state).await;
    let body = serde_json::to_vec(&sample_envelope().await).expect("envelope serialises");

    let mut stream = send_declared_upload(addr, "/v1/traces", TOKEN, body.len() as u64).await;
    // Past the credential's expiry, with the body not yet sent.
    tokio::time::sleep(StdDuration::from_millis(2000)).await;
    stream.write_all(&body).await.expect("body writes");
    let mut response = vec![0u8; 64];
    let read = tokio::time::timeout(StdDuration::from_secs(30), stream.read(&mut response))
        .await
        .expect("answered")
        .expect("reads");
    let status = String::from_utf8_lossy(&response[..read]).to_string();
    assert!(
        status.starts_with("HTTP/1.1 200"),
        "the upload began with a valid credential and is accepted: {status}"
    );

    // A new request with the now-expired credential is refused before its
    // body is read.
    let mut late = send_declared_upload(addr, "/v1/traces", TOKEN, 1024).await;
    assert_eq!(
        response_status_within(&mut late, StdDuration::from_secs(2)).await,
        Some("HTTP/1.1 403 Forbidden".to_string()),
        "an expired credential starts no new upload"
    );
    server.abort();
}

/// Every upload route admits a body of exactly its ceiling and refuses one
/// byte more, checked by declared length: an admitted request waits for its
/// body, a refused one is answered at once. Nothing is allocated at the
/// ceiling, which for an envelope route is hundreds of MB.
#[tokio::test]
async fn upload_routes_admit_their_ceiling_by_declared_length() {
    const TOKEN: &str = "ceiling-token";
    let temp = tempfile::tempdir().expect("temp dir");
    let state = state_with_token(temp.path(), TOKEN, None);
    let budget = state.large_body_bytes.clone();
    let (addr, server) = serve_ingest_for_test(state).await;
    let all_free = budget.available_permits();
    for (method, path, ceiling) in super::large_upload_route_ceilings() {
        let send = |length: u64| async move {
            use tokio::io::AsyncWriteExt;
            let mut stream = tokio::net::TcpStream::connect(addr)
                .await
                .expect("connects");
            stream
                .write_all(
                    format!(
                        "{method} {path} HTTP/1.1\r\nHost: ingest.test\r\n\
                         Authorization: Bearer {TOKEN}\r\nContent-Type: application/json\r\n\
                         Content-Length: {length}\r\n\r\n"
                    )
                    .as_bytes(),
                )
                .await
                .expect("headers write");
            stream
        };
        let mut at = send(ceiling as u64).await;
        assert_eq!(
            response_status_within(&mut at, StdDuration::from_millis(300)).await,
            None,
            "{method} {path} admits a declared {ceiling} bytes and waits for them"
        );
        // Give its bytes back, so the next request meets the body limit and
        // not the byte budget.
        drop(at);
        tokio::time::timeout(StdDuration::from_secs(5), async {
            while budget.available_permits() < all_free {
                tokio::time::sleep(StdDuration::from_millis(10)).await;
            }
        })
        .await
        .expect("an abandoned upload gives its bytes back");
        let mut over = send(ceiling as u64 + 1).await;
        assert_eq!(
            response_status_within(&mut over, StdDuration::from_secs(2)).await,
            Some("HTTP/1.1 413 Payload Too Large".to_string()),
            "{method} {path} refuses a declared {} bytes before reading them",
            ceiling + 1
        );
    }
    server.abort();
}

/// The object-ref hash is taken while serializing; it must be the hash and
/// length of exactly the pretty JSON it replaced.
#[tokio::test]
async fn pretty_json_hash_and_length_match_the_materialized_string() {
    let envelope = sample_envelope().await;
    let pretty = serde_json::to_string_pretty(&envelope).unwrap();
    assert_eq!(
        pretty_json_sha256_prefixed_and_len(&envelope).unwrap(),
        (sha256_prefixed(&pretty), pretty.len() as u64)
    );
}

/// The pilot's legacy-path shape for the memory probe: object-primary over a
/// service-owned remote store (v2, KEK-wrapped, the provider serializing a
/// record per object) with the PostgreSQL mirror on.
fn pilot_shaped_state(root: &Path, backend: Arc<dyn Database>) -> Arc<AppState> {
    let object_store = TRACE_COMMONS_SERVICE_REMOTE_OBJECT_STORE.to_string();
    let key = trace_commons_server::secrets::keychain::generate_master_key_hex();
    let crypto = SecretsCrypto::new(SecretString::from(key.clone())).expect("crypto");
    let kek = LocalMasterKeyWrapper::new(
        SecretsCrypto::new(SecretString::from(key)).expect("kek crypto"),
        "large-envelope-probe-v1",
    );
    let provider = FileRemoteTraceArtifactProvider::new(root.join("objects"));
    let store = Arc::new(ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote(object_store.clone())
            .expect("provider config"),
        crypto,
        kek,
        provider,
    ));
    let mut state = test_state_with_options(
        root.to_path_buf(),
        Some(backend),
        None,
        false,
        false,
        false,
        false,
    );
    let state_mut = Arc::get_mut(&mut state).expect("fresh state");
    state_mut.artifact_store = Some(ConfiguredTraceArtifactStore::new(object_store, store));
    state_mut.object_primary_submit_review = true;
    state_mut.require_db_mirror_writes = true;
    state
}

/// Manual probe, as `large_envelope_submit_probe`, through the pilot-shaped
/// legacy path (`pilot_shaped_state`). Needs
/// `TRACE_COMMONS_PG_TEST_DATABASE_URL`; refuses to pass without it.
#[tokio::test]
#[ignore = "manual memory probe; set TC_LARGE_ENVELOPE_BYTES and a PG test URL"]
async fn large_envelope_pilot_shaped_submit_probe() {
    let backend = postgres_backend_for_ingest_test()
        .await
        .expect("this probe needs TRACE_COMMONS_PG_TEST_DATABASE_URL");
    cleanup_pg_trace_tenant(&backend, "tenant-a").await;
    reset_account_rate_limiter_for_test();
    let target: u64 = std::env::var("TC_LARGE_ENVELOPE_BYTES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(200_000_000);
    let started = std::time::Instant::now();
    let temp = tempfile::tempdir().expect("temp dir");
    let fixture = temp.path().join("envelope.json");
    let written = write_synthetic_envelope(&fixture, target).await;
    let state = pilot_shaped_state(
        &temp.path().join("root"),
        backend.clone() as Arc<dyn Database>,
    );
    let (addr, server) = serve_ingest_for_test(state).await;
    let submit_started = std::time::Instant::now();
    let (status, body) = post_envelope_file(addr, "token-a", &fixture).await;
    let submit = submit_started.elapsed();
    server.abort();
    assert_eq!(status, 200, "accepted: {body}");
    let receipt: serde_json::Value = serde_json::from_str(&body).expect("receipt");
    eprintln!(
        "large-envelope-pilot-probe bytes={} events={} submit_secs={:.1} total_secs={:.1} status={}",
        written.bytes,
        written.events,
        submit.as_secs_f64(),
        started.elapsed().as_secs_f64(),
        receipt["status"].as_str().unwrap_or_default()
    );
    cleanup_pg_trace_tenant(&backend, "tenant-a").await;
}
