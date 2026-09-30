// INTEGRATION: include as a child of near_wallet_page.rs with #[cfg(test)]
// #[path = "near_wallet_page_tests.rs"] mod tests; these exercise the production
// HTTP client against synthetic loopback servers, without contacting a wallet.

use std::collections::BTreeSet;
use std::time::Duration;

use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::daemon::nearai_credential::near_wallet::{NearWalletChallenge, NearWalletError};
use crate::daemon::nearai_credential::near_wallet_page::{
    ASSET_PREFIX, Assets, MANIFEST, MAX_ADAPTER_BYTES, asset_path, fetch_script, render,
};

const DEADLINE: Duration = Duration::from_secs(3);
const SCRIPT: &[u8] = b"globalThis.syntheticWallet = true;\n";

#[test]
#[ignore = "requires Playwright/Chromium and anonymous downloads of hash-pinned adapters"]
fn pinned_wallet_controls_obey_production_csp() {
    let now = chrono::Utc::now().timestamp_millis() as u64;
    let challenge = NearWalletChallenge::new(34567, now).unwrap();
    let url = reqwest::Url::parse(&challenge.browser_url().unwrap()).unwrap();
    let fixture = tempfile::tempdir().unwrap();
    let fixture_path = fixture.path().join("wallet-page.json");
    std::fs::write(
        &fixture_path,
        serde_json::to_vec(&serde_json::json!({
            "html": render(&challenge).unwrap(),
            "state": url.fragment().unwrap(),
        }))
        .unwrap(),
    )
    .unwrap();
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/ci/test-near-wallet-controls.cjs");
    let output = std::process::Command::new("node")
        .arg(script)
        .arg(fixture_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "browser controls failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn connector_bundle_matches_the_reviewed_release() {
    assert_eq!(
        digest(crate::daemon::nearai_credential::near_wallet_page::SDK.as_bytes()),
        "fd2c7d0dee183b6b9b0ea826fb2551f10988e1fb47e480c965b4bc2bb27af03f"
    );
}

const WALLET_IDS: [&str; 6] = [
    "hot-wallet",
    "meteor-wallet",
    "near-mobile",
    "nightly-wallet",
    "hana-wallet",
    "near-cli",
];

fn configuration(html: &str) -> Value {
    let json = html
        .split_once("<script id=\"challenge\" type=\"application/json\">")
        .unwrap()
        .1
        .split_once("</script>")
        .unwrap()
        .0;
    serde_json::from_str(json).unwrap()
}

#[test]
fn assets_accept_only_bundled_scripts_and_reviewed_wallet_ids() {
    let manifest: Value = serde_json::from_str(MANIFEST).unwrap();
    let wallets = manifest["wallets"].as_array().unwrap();
    let actual: BTreeSet<_> = wallets
        .iter()
        .map(|wallet| wallet["id"].as_str().unwrap())
        .collect();
    assert_eq!(actual, BTreeSet::from(WALLET_IDS));
    assert_eq!(wallets.len(), actual.len(), "duplicate wallet IDs");
    for name in ["connector", "browser"].into_iter().chain(WALLET_IDS) {
        let path = format!("{ASSET_PREFIX}{name}.js");
        assert_eq!(asset_path(&path), Some(path.as_str()));
        for nonce in ["Cache-9".to_owned(), "A".repeat(64)] {
            assert_eq!(
                asset_path(&format!("{path}?nonce={nonce}")),
                Some(path.as_str())
            );
        }
    }
}

#[test]
fn assets_reject_traversal_aliases_fragments_and_unknown_queries() {
    for name in [
        "",
        "unknown.js",
        "my-near-wallet.js",
        "walletconnect.js",
        "trezu.js",
        "../connector.js",
        "%2e%2e/connector.js",
        "x/../connector.js",
        "%63onnector.js",
        "connector.js/",
        "connector.js#fragment",
        "connector.js?",
        "connector.js?nonce=",
        "connector.js?cache=1",
        "connector.js?nonce=a&other=b",
        "connector.js?other=b&nonce=a",
        "connector.js?nonce=a&nonce=b",
        "connector.js?nonce=a_b",
        "connector.js?nonce=a%2Db",
        "connector.js?nonce=a?b",
        "connector.js?nonce=a#b",
        "connector.js?nonce=a/b",
        "connector.js?nonce=é",
        "connector.js?nonce=a\r\n",
        "/connector.js",
        "Connector.js",
        "connector.js\0",
    ] {
        assert_eq!(
            asset_path(&format!("{ASSET_PREFIX}{name}")),
            None,
            "{name:?}"
        );
    }
    assert_eq!(
        asset_path(&format!(
            "{ASSET_PREFIX}connector.js?nonce={}",
            "a".repeat(65)
        )),
        None
    );
    for path in [
        "/connector.js",
        "/near-ai/near-wallet/assetsx/connector.js",
        "https://example.invalid/near-ai/near-wallet/assets/connector.js",
        "//example.invalid/near-ai/near-wallet/assets/connector.js",
    ] {
        assert_eq!(asset_path(path), None);
    }
}

#[test]
fn rendered_manifest_keeps_reviewed_wallets_and_binds_versions_to_digests() {
    let challenge = NearWalletChallenge::new(54321, 1_780_000_000_000).unwrap();
    let config = configuration(&render(&challenge).unwrap());
    let bundled: Value = serde_json::from_str(MANIFEST).unwrap();
    let wallets = config["manifest"]["wallets"].as_array().unwrap();
    assert_eq!(wallets.len(), WALLET_IDS.len());
    for wallet in wallets {
        let id = wallet["id"].as_str().unwrap();
        assert!(WALLET_IDS.contains(&id));
        let label = format!("{} {}", id, wallet["name"].as_str().unwrap())
            .chars()
            .filter(|character| character.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        for excluded in ["mynear", "walletconnect", "trezu"] {
            assert!(!label.contains(excluded));
        }
        let original = bundled["wallets"]
            .as_array()
            .unwrap()
            .iter()
            .find(|original| original["id"] == id)
            .unwrap();
        let digest = original["sha256"].as_str().unwrap();
        assert_eq!(digest.len(), 64);
        assert!(digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_eq!(wallet["executor"], format!("{ASSET_PREFIX}{id}.js"));
        assert_eq!(
            wallet["version"],
            format!("{digest}.{}", config["cspNonce"].as_str().unwrap())
        );
        assert!(wallet.get("sha256").is_none());
    }
}

#[test]
fn rendered_configuration_omits_state_and_executable_scripts_share_the_csp_nonce() {
    let challenge = NearWalletChallenge::new(54321, 1_780_000_000_000).unwrap();
    let browser_url = url::Url::parse(&challenge.browser_url().unwrap()).unwrap();
    let state = browser_url.fragment().unwrap();
    let html = render(&challenge).unwrap();
    let config = configuration(&html);
    assert!(config.get("state").is_none());
    assert!(
        !html.contains(state),
        "ceremony state entered the HTTP response"
    );
    assert_eq!(config["nonce"], challenge.browser_configuration()["nonce"]);
    let nonce = config["cspNonce"].as_str().unwrap();
    assert_eq!(STANDARD.decode(nonce).unwrap().len(), 32);
    let csp = html
        .split_once("http-equiv=\"Content-Security-Policy\" content=\"")
        .unwrap()
        .1
        .split_once('"')
        .unwrap()
        .0;
    let scripts = csp
        .split(';')
        .map(str::trim)
        .find(|directive| directive.starts_with("script-src "))
        .unwrap();
    assert_eq!(scripts, format!("script-src 'nonce-{nonce}'"));
    let executable: Vec<_> = html
        .split("<script ")
        .skip(1)
        .map(|script| script.split_once('>').unwrap().0)
        .filter(|attributes| !attributes.contains("type=\"application/json\""))
        .collect();
    assert_eq!(executable.len(), 2);
    for (attributes, asset) in executable.iter().zip(["connector.js", "browser.js"]) {
        assert!(attributes.contains(&format!("nonce=\"{nonce}\"")));
        assert!(attributes.contains(&format!("src=\"{ASSET_PREFIX}{asset}\"")));
    }
    let another = configuration(&render(&challenge).unwrap());
    assert_ne!(config["cspNonce"], another["cspNonce"]);
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn fixed_response(status: u16, body: &[u8]) -> Vec<u8> {
    let mut reply = format!(
        "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    reply.extend_from_slice(body);
    reply
}

fn chunked_response(body: &[u8], finished: bool) -> Vec<u8> {
    let mut reply =
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n".to_vec();
    for chunk in body.chunks(16 * 1024) {
        reply.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
        reply.extend_from_slice(chunk);
        reply.extend_from_slice(b"\r\n");
    }
    if finished {
        reply.extend_from_slice(b"0\r\n\r\n");
    }
    reply
}

struct ScriptServer {
    url: String,
    requests: mpsc::UnboundedReceiver<String>,
    task: JoinHandle<()>,
}

impl ScriptServer {
    async fn start(reply: Vec<u8>, keep_open: bool) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/adapter.js", listener.local_addr().unwrap());
        let (record, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 1024];
            loop {
                let count = socket.read(&mut buffer).await.unwrap();
                assert!(count > 0, "request ended before headers");
                request.extend_from_slice(&buffer[..count]);
                assert!(request.len() <= 16 * 1024);
                if request.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
            }
            record.send(String::from_utf8(request).unwrap()).unwrap();
            if socket.write_all(&reply).await.is_ok() && keep_open {
                // No EOF/terminal chunk: the size guard, not a completed body
                // or the client's longer timeout, must end the request.
                let _ = socket.read(&mut [0u8; 1]).await;
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    async fn assert_anonymous_get(&mut self) {
        let request = timeout(DEADLINE, self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request.lines().next(), Some("GET /adapter.js HTTP/1.1"));
        for line in request.lines().skip(1) {
            if let Some((name, _)) = line.split_once(':') {
                assert!(
                    !["authorization", "proxy-authorization", "cookie"]
                        .iter()
                        .any(|forbidden| name.eq_ignore_ascii_case(forbidden))
                );
            }
        }
    }

    async fn fetch(&self, expected_digest: &str) -> Result<String, NearWalletError> {
        let assets = Assets::new().unwrap();
        timeout(
            DEADLINE,
            fetch_script(&assets.client, &self.url, expected_digest),
        )
        .await
        .expect("fetch waited for EOF or its asset timeout")
    }
}

impl Drop for ScriptServer {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[tokio::test]
async fn fetch_accepts_the_exact_digest_without_credentials() {
    let mut server = ScriptServer::start(fixed_response(200, SCRIPT), false).await;
    assert_eq!(
        server.fetch(&digest(SCRIPT)).await.unwrap().as_bytes(),
        SCRIPT
    );
    server.assert_anonymous_get().await;
}

#[tokio::test]
async fn fetch_accepts_fixed_and_chunked_bodies_at_the_exact_byte_limit() {
    let body = vec![b' '; MAX_ADAPTER_BYTES];
    for reply in [fixed_response(200, &body), chunked_response(&body, true)] {
        let mut server = ScriptServer::start(reply, false).await;
        assert_eq!(server.fetch(&digest(&body)).await.unwrap().as_bytes(), body);
        server.assert_anonymous_get().await;
    }
}

#[tokio::test]
async fn fetch_rejects_hash_mismatch_and_non_utf8_scripts() {
    for (body, expected) in [
        (SCRIPT.to_vec(), digest(b"a different reviewed adapter")),
        (vec![0xff, 0xfe], digest(&[0xff, 0xfe])),
    ] {
        let mut server = ScriptServer::start(fixed_response(200, &body), false).await;
        assert_eq!(
            server.fetch(&expected).await,
            Err(NearWalletError::Unavailable)
        );
        server.assert_anonymous_get().await;
    }
}

#[tokio::test]
async fn fetch_rejects_non_success_statuses_even_with_the_expected_body() {
    for status in [403, 404, 503] {
        let mut server = ScriptServer::start(fixed_response(status, SCRIPT), false).await;
        assert_eq!(
            server.fetch(&digest(SCRIPT)).await,
            Err(NearWalletError::Unavailable)
        );
        server.assert_anonymous_get().await;
    }
}

#[tokio::test]
async fn fetch_rejects_redirect_without_requesting_the_destination() {
    let mut destination = ScriptServer::start(fixed_response(200, SCRIPT), false).await;
    let reply = format!(
        "HTTP/1.1 302 Found\r\nLocation: {}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        destination.url
    );
    let mut server = ScriptServer::start(reply.into_bytes(), false).await;
    assert_eq!(
        server.fetch(&digest(SCRIPT)).await,
        Err(NearWalletError::Unavailable)
    );
    server.assert_anonymous_get().await;
    assert!(matches!(
        destination.requests.try_recv(),
        Err(mpsc::error::TryRecvError::Empty)
    ));
}

#[tokio::test]
async fn fetch_rejects_oversized_content_length_before_reading_a_body() {
    let reply = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        MAX_ADAPTER_BYTES + 1
    );
    let mut server = ScriptServer::start(reply.into_bytes(), true).await;
    assert_eq!(
        server.fetch(&digest(&[])).await,
        Err(NearWalletError::Unavailable)
    );
    server.assert_anonymous_get().await;
}

#[tokio::test]
async fn fetch_rejects_oversized_chunked_and_unframed_streams_before_eof() {
    let body = vec![b' '; MAX_ADAPTER_BYTES + 1];
    let mut unframed = b"HTTP/1.1 200 OK\r\nConnection: close\r\n\r\n".to_vec();
    unframed.extend_from_slice(&body);
    for reply in [chunked_response(&body, false), unframed] {
        let mut server = ScriptServer::start(reply, true).await;
        assert_eq!(
            server.fetch(&digest(&body)).await,
            Err(NearWalletError::Unavailable)
        );
        server.assert_anonymous_get().await;
    }
}
