//! Where a local reader finds the running daemon.
//!
//! IronWire's control API needs two things: the port it is listening on, and
//! the token in `$IRONWIRE_HOME/control.token`. A shell knows both, because a
//! shell has `$IRONWIRE_HOME`.
//!
//! A desktop application does not. An app launched from Finder, the Dock or a
//! desktop entry inherits the session manager's environment, not a shell
//! profile's, so `$IRONWIRE_HOME` is simply absent however carefully the user
//! set it. Such a reader falls back to `~/.ironwire`, finds nothing when the
//! home is elsewhere, and cannot tell that case apart from "IronWire is not
//! running" -- the two look identical from outside.
//!
//! So the daemon leaves a pointer at a path that never moves:
//! `~/.ironwire/endpoint.json`, written whatever `$IRONWIRE_HOME` says. It
//! names where the token is; it never carries the token. Anything able to read
//! the pointer can already read the token beside it, so this widens nothing --
//! it only removes the guessing.
//!
//! The file is written when the daemon binds and removed when it stops. A
//! stale one left by a crash costs a reader one refused connection, which is
//! the same answer it would have got from a daemon that was not running.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The fixed directory a reader looks in, regardless of `$IRONWIRE_HOME`.
///
/// Deliberately not [`crate::config::PathsConfig`]: that resolves the
/// environment, which is the thing this file exists to work around.
#[must_use]
pub fn discovery_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| discovery_dir_under(&h))
}

/// The discovery directory beneath a given home.
///
/// Split out so the placement is testable without mutating the environment --
/// which this crate forbids, and which a test could only do to prove a
/// negative. The environment-independence is structural instead: this function
/// takes the home it is given, and its only caller passes
/// [`dirs::home_dir`]. Nothing here reads a variable.
#[must_use]
pub fn discovery_dir_under(home: &Path) -> PathBuf {
    home.join(".ironwire")
}

/// `~/.ironwire/endpoint.json`.
#[must_use]
pub fn discovery_file() -> Option<PathBuf> {
    discovery_dir().map(|d| d.join("endpoint.json"))
}

/// What a local reader needs to reach the control API.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Base URL of the control API, always loopback.
    pub control_url: String,
    /// Absolute path of the control token file.
    ///
    /// The path, never the token. A reader opens it itself, which keeps the
    /// credential in one place with one set of permissions.
    pub token_path: PathBuf,
}

impl Endpoint {
    /// The endpoint a daemon on `port` with its token at `token_path` offers.
    #[must_use]
    pub fn new(port: u16, token_path: impl Into<PathBuf>) -> Self {
        Self {
            control_url: format!("http://127.0.0.1:{port}"),
            token_path: token_path.into(),
        }
    }

    /// Publish this endpoint at the fixed discovery path.
    ///
    /// Creates `~/.ironwire` if it is not there -- which it may not be, when
    /// `$IRONWIRE_HOME` points somewhere else and nothing has ever used the
    /// conventional directory.
    ///
    /// # Errors
    ///
    /// Fails when no home directory can be determined, and propagates write
    /// failures.
    pub fn publish(&self) -> std::io::Result<PathBuf> {
        let dir = discovery_dir().ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "no home directory for the discovery pointer",
            )
        })?;
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("endpoint.json");
        let body = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        crate::atomic::write(&path, &body)?;
        Ok(path)
    }

    /// Read whatever a daemon last published, if anything.
    ///
    /// Returns `None` rather than an error for every absent or unreadable
    /// case: to a reader, "no pointer" and "a pointer I cannot parse" both
    /// mean the same thing, which is that discovery did not work and the
    /// caller should fall back to asking.
    #[must_use]
    pub fn read() -> Option<Self> {
        Self::read_from(&discovery_file()?)
    }

    /// [`Self::read`], against an explicit path. Used by tests.
    #[must_use]
    pub fn read_from(path: &Path) -> Option<Self> {
        let body = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&body).ok()
    }

    /// Remove the pointer, if it is there.
    ///
    /// Best effort by design: a daemon shutting down should not fail because
    /// a file it was going to delete is already gone.
    pub fn withdraw() {
        if let Some(path) = discovery_file() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_endpoint_names_the_loopback_port() {
        let e = Endpoint::new(8463, "/custom/home/control.token");
        assert_eq!(e.control_url, "http://127.0.0.1:8463");
        assert_eq!(e.token_path, PathBuf::from("/custom/home/control.token"));
    }

    #[test]
    fn a_published_endpoint_reads_back_identically() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        let written = Endpoint::new(9000, "/somewhere/control.token");
        let body = serde_json::to_string_pretty(&written).expect("serialises");
        crate::atomic::write(&path, &body).expect("writes");

        assert_eq!(Endpoint::read_from(&path), Some(written));
    }

    /// The whole reason this module exists.
    ///
    /// A reader that resolved `$IRONWIRE_HOME` would look in the wrong place
    /// for exactly the population this serves. That independence is
    /// structural -- nothing in this module reads a variable -- so what a test
    /// can pin is the placement: the pointer sits beside the conventional
    /// home's other state, whatever `$IRONWIRE_HOME` was set to.
    #[test]
    fn discovery_sits_under_the_conventional_home() {
        assert_eq!(
            discovery_dir_under(Path::new("/home/someone")),
            PathBuf::from("/home/someone/.ironwire")
        );
        assert_eq!(
            discovery_dir_under(Path::new("/other/home")),
            PathBuf::from("/other/home/.ironwire")
        );
    }

    /// The pointer names the token; it never carries it.
    #[test]
    fn a_published_endpoint_contains_no_token_material() {
        let e = Endpoint::new(8463, "/home/someone/.ironwire/control.token");
        let body = serde_json::to_string(&e).expect("serialises");
        assert!(
            !body.contains("token\":\"") || body.contains("token_path"),
            "only a path may appear"
        );
        // Field-level, so a future field carrying a secret fails this.
        let parsed: serde_json::Value = serde_json::from_str(&body).expect("parses");
        let keys: Vec<&str> = parsed
            .as_object()
            .expect("object")
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["control_url", "token_path"]);
    }

    #[test]
    fn an_absent_pointer_reads_as_none_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert_eq!(Endpoint::read_from(&dir.path().join("nothing.json")), None);
    }

    #[test]
    fn an_unparsable_pointer_reads_as_none() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        std::fs::write(&path, "{ not json").expect("writes");
        assert_eq!(Endpoint::read_from(&path), None);
    }
}
