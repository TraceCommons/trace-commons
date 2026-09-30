//! Wallet signup must work in a daemon started the way a shipped app starts
//! one: from Finder, the Start Menu, or a flatpak sandbox, with no
//! `TRACE_COMMONS_ALLOWED_HOSTS` anywhere in the environment.
//!
//! `account_onboarding::client` refuses outright when the host allowlist is
//! not enforcing, and an unset `TRACE_COMMONS_ALLOWED_HOSTS` produces exactly
//! that: `HostAllowlist::from_env` returns a permissive list, `is_enforcing`
//! is false, and every signup step is refused before a single packet leaves.
//! Nothing in `macos/`, `windows/`, the GTK crate, the FFI crate or the
//! flatpak manifest sets that variable, so the first "Check availability"
//! click failed in every shipped application and said only that wallet
//! signup was "unavailable for this commons".
//!
//! These tests therefore spawn the *real daemon binary* with the variable
//! explicitly removed, and talk to it over its socket exactly as a native
//! shell does. An in-process harness would prove nothing here: it inherits
//! whatever the test runner's environment happens to hold, which is the very
//! thing the bug is about.
//!
//! What "signup can proceed" means without a live HTTPS commons to point at:
//! the daemon must get far enough to actually attempt the connection, and
//! must say so. The distinction between "this address was refused before
//! anything was sent" and "that address did not answer" is the whole
//! observable difference between the broken and the fixed daemon, and it is
//! also the diagnosis a person and their support need.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// A daemon running as a separate OS process, with the allowlist environment
/// variable removed rather than merely unset-by-luck.
struct AppDaemon {
    _dir: tempfile::TempDir,
    store_dir: PathBuf,
    child: Child,
}

impl AppDaemon {
    fn start() -> Self {
        // Short base path: a unix socket path is capped at 104 bytes, and the
        // daemon refuses to bind past it.
        let dir = tempfile::Builder::new().prefix("tcw").tempdir().unwrap();
        let store_dir = dir.path().to_path_buf();
        let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-contributor"));
        command
            .arg("--config-dir")
            .arg(&store_dir)
            .arg("daemon")
            .arg("run")
            // The point of the whole file. A shipped application sets none of
            // these, so neither does this harness.
            .env_remove("TRACE_COMMONS_ALLOWED_HOSTS")
            .env_remove("TRACE_COMMONS_CONTRIBUTOR_DIR")
            .env_remove("TRACE_COMMONS_INFERENCE_RECEIPT_ENDPOINT");
        let child = command.spawn().unwrap();
        let daemon = Self {
            _dir: dir,
            store_dir,
            child,
        };
        daemon.wait_for_socket();
        daemon
    }

    fn socket(&self) -> PathBuf {
        self.store_dir.join("daemon.sock")
    }

    fn wait_for_socket(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        while Instant::now() < deadline {
            if socket_answers(&self.socket()) {
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("the daemon never bound its socket");
    }

    async fn call(&self, method: &str, params: serde_json::Value) -> serde_json::Value {
        let stream = UnixStream::connect(self.socket()).await.unwrap();
        let (r, w) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut writer = w;
        let line = serde_json::json!({"id": 1, "method": method, "params": params}).to_string();
        writer.write_all(line.as_bytes()).await.unwrap();
        writer.write_all(b"\n").await.unwrap();
        writer.flush().await.unwrap();
        let mut reply = String::new();
        reader.read_line(&mut reply).await.unwrap();
        serde_json::from_str(&reply).unwrap()
    }
}

impl Drop for AppDaemon {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn socket_answers(path: &Path) -> bool {
    std::os::unix::net::UnixStream::connect(path).is_ok()
}

/// An https origin on a port nothing is listening on: shape-valid, reachable
/// only if the daemon actually tries.
fn closed_https_origin() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);
    format!("https://127.0.0.1:{port}")
}

#[tokio::test]
async fn a_daemon_with_no_allowed_hosts_env_attempts_the_signup_endpoint() {
    let daemon = AppDaemon::start();
    let reply = daemon
        .call(
            "near_account_capabilities",
            serde_json::json!({"ingest_url": closed_https_origin()}),
        )
        .await;
    let result = &reply["result"];
    // Not ready, because nothing is listening -- but the reason must be that
    // the address did not answer, which is only sayable if the request was
    // actually attempted. A daemon that refused on its own allowlist never
    // opened a socket and must not be able to claim this.
    assert_eq!(result["ready"], false, "{reply}");
    assert_eq!(
        result["reason"], "unreachable",
        "a daemon started with no TRACE_COMMONS_ALLOWED_HOSTS must reach the \
         network rather than refuse its own address: {reply}"
    );
}

#[tokio::test]
async fn an_address_the_daemon_will_not_dial_is_named_as_such() {
    let daemon = AppDaemon::start();
    // Plain http is refused before anything is sent, and must not be
    // reported with the same word as a commons that did not answer.
    let reply = daemon
        .call(
            "near_account_capabilities",
            serde_json::json!({"ingest_url": "http://commons.example"}),
        )
        .await;
    assert_eq!(reply["result"]["ready"], false, "{reply}");
    assert_eq!(reply["result"]["reason"], "address_refused", "{reply}");
}

#[tokio::test]
async fn the_three_refusal_reasons_are_reported_distinctly() {
    let daemon = AppDaemon::start();
    let refused = daemon
        .call(
            "near_account_capabilities",
            serde_json::json!({"ingest_url": "https://commons.example/path?query=1"}),
        )
        .await;
    let unreachable = daemon
        .call(
            "near_account_capabilities",
            serde_json::json!({"ingest_url": closed_https_origin()}),
        )
        .await;
    assert_ne!(
        refused["result"]["reason"], unreachable["result"]["reason"],
        "a refused address and an unanswered one must not read alike"
    );
}
