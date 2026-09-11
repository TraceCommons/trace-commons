// INTEGRATION: register under nearai_credential/api.rs with #[cfg(test)]
// #[path = "near_wallet_api_tests.rs"] mod near_wallet_tests;
// Requires CloudApi::sign_in_near(VerifiedNearWalletSignIn), returning the
// session and refresh_token_expires_at fields, and the common bounded decoder.
// These are real local HTTP round trips using a synthetic Ed25519 signature;
// they do not contact Cloud, prove on-chain ownership, or open a real wallet.

use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use ring::signature::{Ed25519KeyPair, KeyPair};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::daemon::nearai_credential::api::CloudApi;
use crate::daemon::nearai_credential::near_wallet::{
    NearWalletChallenge, VerifiedNearWalletSignIn,
};

const RESPONSE_LIMIT: usize = 256 * 1024;
const DEADLINE: Duration = Duration::from_secs(3);
const ACCOUNT: &str = "synthetic-login.near";
const ACCESS: &str = "synthetic-wallet-access-token";
const REFRESH: &str = "synthetic-wallet-refresh-token";

fn public_key(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut input = bytes.to_vec();
    let zeros = input.iter().take_while(|byte| **byte == 0).count();
    let mut digits = Vec::new();
    let mut start = zeros;
    while start < input.len() {
        let mut remainder = 0u16;
        for byte in &mut input[start..] {
            let value = remainder * 256 + u16::from(*byte);
            *byte = (value / 58) as u8;
            remainder = value % 58;
        }
        digits.push(ALPHABET[remainder as usize]);
        while start < input.len() && input[start] == 0 {
            start += 1;
        }
    }
    digits.extend(std::iter::repeat_n(b'1', zeros));
    digits.reverse();
    format!("ed25519:{}", String::from_utf8(digits).unwrap())
}

fn signed_proof() -> (VerifiedNearWalletSignIn, Value) {
    let now = u64::try_from(Utc::now().timestamp_millis()).unwrap();
    let mut challenge = NearWalletChallenge::new(54321, now).unwrap();
    let url = url::Url::parse(&challenge.browser_url().unwrap()).unwrap();
    let query = challenge.browser_configuration();
    assert_eq!(query["message"], "Sign in to NEAR AI Cloud");
    assert_eq!(query["recipient"], "cloud.near.ai");
    let nonce: Vec<u8> = serde_json::from_value(query["nonce"].clone()).unwrap();
    assert_eq!(nonce.len(), 32);
    assert_eq!(&nonce[..8], now.to_be_bytes());
    // Spell NEP-413's Borsh preimage independently of the adapter's private
    // signer. Only the public browser configuration supplies ceremony-specific fields.
    let mut preimage = ((1u32 << 31) + 413).to_le_bytes().to_vec();
    let append = |bytes: &mut Vec<u8>, value: &str| {
        bytes.extend_from_slice(&(value.len() as u32).to_le_bytes());
        bytes.extend_from_slice(value.as_bytes());
    };
    append(&mut preimage, query["message"].as_str().unwrap());
    preimage.extend_from_slice(&nonce);
    append(&mut preimage, query["recipient"].as_str().unwrap());
    preimage.push(0);
    let key = Ed25519KeyPair::from_seed_unchecked(&[7; 32]).unwrap();
    let public_key = public_key(key.public_key().as_ref());
    let signature = STANDARD.encode(key.sign(&Sha256::digest(preimage)).as_ref());
    let callback = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("accountId", ACCOUNT)
        .append_pair("publicKey", &public_key)
        .append_pair("signature", &signature)
        .append_pair("state", url.fragment().unwrap())
        .finish();
    let proof = challenge.verify_callback(&callback, now).unwrap();
    let expected = json!({
        "signed_message": {
            "accountId": ACCOUNT, "publicKey": public_key,
            "signature": signature, "state": url.fragment().unwrap()
        },
        "payload": {
            "message": "Sign in to NEAR AI Cloud", "nonce": nonce,
            "recipient": "cloud.near.ai"
        }
    });
    (proof, expected)
}

fn success() -> Value {
    json!({
        "access_token": ACCESS,
        "refresh_token": REFRESH,
        "refresh_token_expiration": (Utc::now() + chrono::Duration::hours(1)).to_rfc3339(),
        "user": {"provider": "near", "email": format!("{ACCOUNT}@near"), "message": "Authenticated"}
    })
}

struct Request {
    head: String,
    body: Vec<u8>,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.head.lines().skip(1).find_map(|line| {
            let (key, value) = line.split_once(':')?;
            key.eq_ignore_ascii_case(name).then_some(value.trim())
        })
    }
}

async fn read_request(socket: &mut TcpStream) -> Request {
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 2048];
    loop {
        let count = socket.read(&mut buffer).await.unwrap();
        assert!(count > 0, "request ended before its body");
        bytes.extend_from_slice(&buffer[..count]);
        assert!(bytes.len() <= 16 * 1024, "unexpected request size");
        if let Some(end) = bytes.windows(4).position(|part| part == b"\r\n\r\n") {
            let head = String::from_utf8(bytes[..end].to_vec()).unwrap();
            let request = Request {
                head,
                body: Vec::new(),
            };
            let length: usize = request
                .header("content-length")
                .unwrap_or("0")
                .parse()
                .unwrap();
            if bytes.len() >= end + 4 + length {
                return Request {
                    body: bytes[end + 4..end + 4 + length].to_vec(),
                    ..request
                };
            }
        }
    }
}

enum Reply {
    Complete(u16, Vec<u8>),
    // No terminal chunk is sent: a bounded reader must decide without EOF.
    Unfinished(u16, Vec<u8>),
    Disconnect,
}

struct Server {
    api: CloudApi,
    requests: mpsc::UnboundedReceiver<Request>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(reply: Reply) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let api =
            CloudApi::for_test(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let (record, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                let request = timeout(DEADLINE, read_request(&mut socket)).await.unwrap();
                if record.send(request).is_err() {
                    return;
                }
                match &reply {
                    Reply::Disconnect => {}
                    Reply::Complete(status, body) => {
                        let head = format!(
                            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        if socket.write_all(head.as_bytes()).await.is_err() {
                            continue;
                        }
                        let _ = socket.write_all(body).await;
                    }
                    Reply::Unfinished(status, body) => {
                        let head = format!(
                            "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n"
                        );
                        if socket.write_all(head.as_bytes()).await.is_err() {
                            continue;
                        }
                        for chunk in body.chunks(16 * 1024) {
                            let frame = format!("{:x}\r\n", chunk.len());
                            if socket.write_all(frame.as_bytes()).await.is_err()
                                || socket.write_all(chunk).await.is_err()
                                || socket.write_all(b"\r\n").await.is_err()
                            {
                                break;
                            }
                        }
                        // Keep the accepted socket alive until the client
                        // closes it. Afterwards the listener detects retries.
                        let _ = socket.read(&mut [0u8; 1]).await;
                    }
                }
            }
        });
        Self {
            api,
            requests,
            task,
        }
    }

    async fn only_request(&mut self) -> Request {
        let request = timeout(DEADLINE, self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            timeout(Duration::from_millis(100), self.requests.recv())
                .await
                .is_err(),
            "request was retried"
        );
        request
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

fn assert_redacted(error: &anyhow::Error) {
    let rendered = format!("{error:?}");
    for sensitive in [
        ACCOUNT,
        ACCESS,
        REFRESH,
        "server-private-detail",
        "http://127.0.0.1",
    ] {
        assert!(
            !rendered.contains(sensitive),
            "error exposed synthetic sensitive input"
        );
    }
}

#[tokio::test]
async fn verified_wallet_proof_posts_exact_wire_and_accepts_matching_cloud_session() {
    let response = success();
    let expiry: DateTime<Utc> =
        serde_json::from_value(response["refresh_token_expiration"].clone()).unwrap();
    let mut server =
        Server::start(Reply::Complete(200, serde_json::to_vec(&response).unwrap())).await;
    let (proof, expected) = signed_proof();
    let authenticated = timeout(DEADLINE, server.api.sign_in_near(proof))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(authenticated.session.access_token, ACCESS);
    assert_eq!(authenticated.session.refresh_token, REFRESH);
    assert_eq!(authenticated.refresh_token_expires_at, expiry);
    let request = server.only_request().await;
    assert_eq!(
        request.head.lines().next(),
        Some("POST /v1/auth/near HTTP/1.1")
    );
    assert_eq!(request.header("user-agent"), Some("TraceCommons/0.12"));
    assert_eq!(request.header("content-type"), Some("application/json"));
    assert!(request.header("authorization").is_none());
    assert_eq!(
        serde_json::from_slice::<Value>(&request.body).unwrap(),
        expected
    );
    let debug = format!("{:?}", authenticated.session);
    assert!(!debug.contains(ACCESS) && !debug.contains(REFRESH));
}

#[tokio::test]
async fn wallet_login_rejects_cloud_account_and_provider_mismatches() {
    for (field, value) in [
        ("email", "other.near@near"),
        ("email", "SYNTHETIC-LOGIN.near@near"),
        ("provider", "github"),
        ("provider", "NEAR"),
    ] {
        let mut response = success();
        response["user"][field] = json!(value);
        let mut server =
            Server::start(Reply::Complete(200, serde_json::to_vec(&response).unwrap())).await;
        let error = timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
            .await
            .unwrap()
            .expect_err("mismatched identity accepted");
        assert_redacted(&error);
        server.only_request().await;
    }
}

#[tokio::test]
async fn wallet_login_rejects_empty_tokens_expired_refresh_and_missing_identity() {
    let mut responses = Vec::new();
    for field in ["access_token", "refresh_token"] {
        let mut response = success();
        response[field] = json!("");
        responses.push(response);
    }
    let mut expired = success();
    expired["refresh_token_expiration"] =
        json!((Utc::now() - chrono::Duration::minutes(1)).to_rfc3339());
    responses.push(expired);
    let mut missing = success();
    missing.as_object_mut().unwrap().remove("user");
    responses.push(missing);
    for response in responses {
        let mut server =
            Server::start(Reply::Complete(200, serde_json::to_vec(&response).unwrap())).await;
        let error = timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
            .await
            .unwrap()
            .expect_err("invalid session accepted");
        assert_redacted(&error);
        server.only_request().await;
    }
}

#[tokio::test]
async fn wallet_login_refusals_do_not_read_unfinished_bodies_or_expose_them() {
    for (status, label) in [
        (401, "near_ai_credential_session_expired"),
        (403, "near_ai_credential_refused"),
        (500, "near_ai_credential_refused"),
    ] {
        let body = format!("server-private-detail {ACCOUNT} {ACCESS} {REFRESH}").into_bytes();
        let mut server = Server::start(Reply::Unfinished(status, body)).await;
        let error = timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
            .await
            .expect("error response waited for its body")
            .expect_err("error status accepted");
        assert_eq!(error.to_string(), label);
        assert_redacted(&error);
        server.only_request().await;
    }
}

#[tokio::test]
async fn wallet_login_rejects_malformed_json_without_echoing_response() {
    let mut server = Server::start(Reply::Complete(200, b"{server-private-detail".to_vec())).await;
    let error = timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
        .await
        .unwrap()
        .expect_err("invalid JSON accepted");
    assert_eq!(error.to_string(), "near_ai_credential_unexpected");
    assert_redacted(&error);
    server.only_request().await;
}

#[tokio::test]
async fn wallet_login_does_not_retry_after_transport_loses_consumed_proof() {
    let mut server = Server::start(Reply::Disconnect).await;
    let error = timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
        .await
        .unwrap()
        .expect_err("closed transport accepted");
    assert_eq!(error.to_string(), "near_ai_credential_unavailable");
    assert_redacted(&error);
    server.only_request().await;
}

#[tokio::test]
async fn wallet_login_accepts_response_at_limit_and_rejects_chunked_overflow_before_eof() {
    let mut body = serde_json::to_vec(&success()).unwrap();
    body.resize(RESPONSE_LIMIT, b' ');
    let mut server = Server::start(Reply::Complete(200, body.clone())).await;
    assert!(
        timeout(DEADLINE, server.api.sign_in_near(signed_proof().0))
            .await
            .unwrap()
            .is_ok()
    );
    server.only_request().await;
    body.push(b' ');
    let mut oversized = Server::start(Reply::Unfinished(200, body)).await;
    let error = timeout(DEADLINE, oversized.api.sign_in_near(signed_proof().0))
        .await
        .expect("oversized stream waited for EOF")
        .expect_err("oversized response accepted");
    assert_eq!(error.to_string(), "near_ai_credential_unavailable");
    assert_redacted(&error);
    oversized.only_request().await;
}

#[tokio::test]
async fn session_exchange_rejects_chunked_overflow_before_eof_and_keeps_user_agent() {
    let mut server = Server::start(Reply::Unfinished(200, vec![b' '; RESPONSE_LIMIT + 1])).await;
    let error = timeout(DEADLINE, server.api.refresh_session(REFRESH))
        .await
        .expect("session decoder buffered beyond the limit")
        .expect_err("oversized response accepted");
    assert_eq!(error.to_string(), "near_ai_credential_unavailable");
    assert_redacted(&error);
    let request = server.only_request().await;
    assert_eq!(
        request.head.lines().next(),
        Some("POST /v1/users/me/access-tokens HTTP/1.1")
    );
    assert_eq!(request.header("user-agent"), Some("TraceCommons/0.12"));
    assert_eq!(
        request.header("authorization"),
        Some(format!("Bearer {REFRESH}").as_str())
    );
}
