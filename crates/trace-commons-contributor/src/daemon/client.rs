//! A blocking, one-shot client for a *running* daemon's socket.
//!
//! This exists because of a failure the per-command reviews could not see.
//! Every `daemon <verb>` CLI command used to build its own `DaemonShared`
//! from the on-disk files, mutate that private copy, and write the files
//! back. A running daemon loads those same files exactly once, at
//! `DaemonShared::load`, and never re-reads them -- so the CLI's edit had no
//! effect on the daemon's behaviour, and the daemon's own next `save()`
//! silently overwrote it. Concretely:
//!
//! - `daemon project <path> --mode ignore` did not disarm auto-upload.
//! - `daemon pause` was erased by the watcher's next unconditional
//!   `daemon-state.json` rewrite, one poll interval later.
//! - `daemon approve` / `daemon dismiss` were discarded by the next
//!   `queue.save()` from the daemon's stale in-memory copy.
//! - `daemon status` printed `health: ok` unconditionally, because
//!   `HealthState` is in-memory only and a fresh `DaemonShared::load`
//!   always starts at `HealthState::default()`.
//!
//! So the control surface we ship as the answer for headless machines could
//! not actually control anything, and could not report anything true. The
//! fix is that the CLI talks to the daemon when one is there, over the same
//! socket the native applications use, and only falls back to direct
//! file access when nothing is running -- where writing the files *is* the
//! correct way to prime the next daemon start.
//!
//! One request, one response, one connection, then close. No subscription:
//! a CLI command has no use for the event stream.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use anyhow::{Context, Result, bail};

use super::ipc::Response;
use crate::config::{ConfigStore, DAEMON_SOCK_FILE};

/// How long to wait for a running daemon to answer. Generous because
/// `"preview"` runs the whole redaction pipeline (and, when a privacy
/// filter is configured, a network round trip) before it answers, and
/// because the daemon answers requests on the same runtime that may
/// currently be mid-scan.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Send one request to a running daemon and return its response.
///
/// `Ok(None)` means *no daemon is running* -- either the socket file does
/// not exist, or it is a stale file left behind by a crashed daemon and
/// nothing is listening on it. That is not an error: the caller is expected
/// to fall back to reading and writing the state files directly, which is
/// what a one-shot command against a stopped daemon should do.
///
/// Every other failure -- a daemon that accepts the connection and then
/// closes it, a truncated or unparseable frame -- is a real error and
/// surfaces as one. Falling back to the file path there would reintroduce
/// exactly the bug this module exists to fix: a command that silently has
/// no effect on the daemon actually running.
pub fn try_call(
    store: &ConfigStore,
    method: &str,
    params: &serde_json::Value,
) -> Result<Option<Response>> {
    let sock = store.daemon_path(DAEMON_SOCK_FILE);
    if !sock.exists() {
        return Ok(None);
    }
    let mut stream = match UnixStream::connect(&sock) {
        Ok(s) => s,
        // A socket file with nothing listening: a crashed daemon left it
        // behind. Nothing is running.
        Err(_) => return Ok(None),
    };
    stream.set_read_timeout(Some(REQUEST_TIMEOUT)).ok();
    stream.set_write_timeout(Some(REQUEST_TIMEOUT)).ok();

    let mut line = serde_json::to_string(&serde_json::json!({
        "id": 1,
        "method": method,
        "params": params,
    }))
    .context("serializing the daemon request")?;
    line.push('\n');
    stream
        .write_all(line.as_bytes())
        .context("sending the request to the running daemon")?;
    stream.flush().ok();

    let mut reply = String::new();
    BufReader::new(&stream)
        .read_line(&mut reply)
        .context("reading the running daemon's response")?;
    if reply.trim().is_empty() {
        bail!("the running daemon closed the connection without answering");
    }
    let response: Response =
        serde_json::from_str(reply.trim()).context("parsing the running daemon's response")?;
    Ok(Some(response))
}

/// Whether a daemon is currently listening on this store's socket.
pub fn is_running(store: &ConfigStore) -> bool {
    let sock = store.daemon_path(DAEMON_SOCK_FILE);
    sock.exists() && UnixStream::connect(&sock).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    #[test]
    fn no_socket_means_no_daemon_rather_than_an_error() {
        let (_d, store) = temp_store();
        assert!(
            try_call(&store, "status", &serde_json::json!({}))
                .unwrap()
                .is_none()
        );
        assert!(!is_running(&store));
    }

    #[test]
    fn a_stale_socket_file_with_nothing_listening_means_no_daemon() {
        let (_d, store) = temp_store();
        // A plain file where the socket would be: `connect` fails, and that
        // is "nothing is running", not a hard error.
        store.write_daemon_file(DAEMON_SOCK_FILE, b"").unwrap();
        assert!(
            try_call(&store, "status", &serde_json::json!({}))
                .unwrap()
                .is_none()
        );
        assert!(!is_running(&store));
    }
}
