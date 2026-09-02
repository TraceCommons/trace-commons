//! Reading IronWire's discovery pointer.
//!
//! IronWire writes `~/.ironwire/endpoint.json` when its daemon binds, and
//! removes it on a clean stop (`nearai/ironwire#18`):
//!
//! ```json
//! { "control_url": "http://127.0.0.1:8463",
//!   "token_path": "/custom/home/control.token" }
//! ```
//!
//! Two facts the machine already knows and a contributor should not be asked
//! for: which port the control API is on, and where its token is.
//!
//! # The path is fixed
//!
//! IronWire writes the pointer at `~/.ironwire/endpoint.json` **regardless of
//! `$IRONWIRE_HOME`** -- the environment variable moves the token, not the
//! pointer. So this module resolves the pointer from the home directory alone
//! and never consults `IRONWIRE_HOME`. Looking for it under `$IRONWIRE_HOME`
//! would find nothing on exactly the machines the variable is set on.
//!
//! # A missing pointer is not an error
//!
//! No file, unreadable file, malformed JSON, a `control_url` naming something
//! this reader will not talk to: every one of them means the same thing --
//! there is nothing here to discover -- and every one of them yields `None`.
//! No error type crosses this boundary, because there is no caller for whom
//! "IronWire is not installed" is a failure.
//!
//! # A stale pointer is not worse than no pointer
//!
//! A daemon that crashed leaves the file behind naming a port nothing is
//! listening on. That is why nothing here is trusted beyond what a refused
//! connection would cost:
//!
//! * the **port** is never used to override a port a contributor declared
//!   (see [`super::settings::ironwire_ledger_for`]); it is offered for
//!   discovery, where the worst case is one refused connection -- the same
//!   cost as a daemon that never ran;
//! * the **`control_url`** is accepted only when it names loopback, so a
//!   pointer written by something else cannot send the app's token to a
//!   remote host;
//! * the **token** is never read from the pointer. The pointer names a path;
//!   the caller opens that path itself, at call time, exactly as it opens
//!   every other resolved token path.

use std::path::{Path, PathBuf};

use serde::Deserialize;

/// The directory IronWire keeps its pointer in, under the user's home.
pub const POINTER_DIR: &str = ".ironwire";

/// The pointer file itself.
pub const POINTER_FILE: &str = "endpoint.json";

/// The largest pointer this reader will read.
///
/// The real file is under 200 bytes. A bound because this is another
/// process's output arriving at a path anything on the machine can write,
/// and `read_to_string` on an unbounded path is how a daemon eats a
/// filesystem.
const MAX_POINTER_BYTES: u64 = 64 * 1024;

/// What a running IronWire said about itself.
///
/// Never carries a token. `token_path` is where one may be found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IronWirePointer {
    /// The loopback port the control API answers on.
    pub port: u16,
    /// Where the daemon says it wrote `control.token`, when it said. A
    /// pointer without one still names a port; the token then resolves the
    /// way it did before this file existed.
    pub token_path: Option<PathBuf>,
}

/// The pointer file as it appears on disk. Only the fields we use.
#[derive(Debug, Deserialize)]
struct PointerFile {
    #[serde(default)]
    control_url: Option<String>,
    #[serde(default)]
    token_path: Option<String>,
}

/// Where the pointer lives, given a home directory.
///
/// Split out from [`pointer_path`] so the composition is testable without a
/// real home directory, and so a test can state the fixed layout that
/// `IRONWIRE_HOME` does not move.
#[must_use]
pub fn pointer_path_in(home: &Path) -> PathBuf {
    home.join(POINTER_DIR).join(POINTER_FILE)
}

/// Where the pointer lives on this machine, when a home directory resolves.
///
/// Under `cfg(test)` this reads a per-process override instead of the real
/// home directory. The whole module is about a file at a fixed absolute path,
/// and a test suite that read it for real would pass or fail according to
/// whether the developer running it happens to have IronWire installed. The
/// override defaults to *no pointer*, so every test in this crate that does
/// not opt in sees the state of a machine without IronWire.
#[cfg(not(test))]
fn pointer_path() -> Option<PathBuf> {
    dirs::home_dir().map(|home| pointer_path_in(&home))
}

#[cfg(test)]
fn pointer_path() -> Option<PathBuf> {
    test_support::override_path()
}

/// Read the pointer a running IronWire left, if there is one.
///
/// Infallible: every failure is "nothing to discover". See the module doc.
#[must_use]
pub fn read_pointer() -> Option<IronWirePointer> {
    let path = pointer_path()?;
    // Checked before reading rather than after: the point is not to hold the
    // bytes at all. A pointer that is not a regular file (a directory, a
    // fifo that would block a read forever) is not one of ours either.
    let metadata = std::fs::metadata(&path).ok()?;
    if !metadata.is_file() || metadata.len() > MAX_POINTER_BYTES {
        return None;
    }
    let body = std::fs::read_to_string(&path).ok()?;
    parse_pointer(&body)
}

/// Parse a pointer document. `None` for anything this reader will not act on.
///
/// Pure, so the whole decision table above is testable without a filesystem.
#[must_use]
pub fn parse_pointer(body: &str) -> Option<IronWirePointer> {
    let parsed: PointerFile = serde_json::from_str(body).ok()?;
    // A pointer with no usable `control_url` is rejected whole, `token_path`
    // included. The document describes one daemon: if this reader cannot
    // reach that daemon, the token beside it is not a fact about anything it
    // can talk to, and carrying it forward would resolve a contributor's
    // token path from a daemon they can never connect to -- a confidently
    // wrong answer, which is the one outcome a stale pointer must not be
    // able to produce.
    let port = loopback_port(parsed.control_url.as_deref()?)?;
    let token_path = parsed
        .token_path
        .filter(|p| !p.is_empty())
        .map(PathBuf::from)
        // A relative token path would resolve against the daemon's working
        // directory, which is not ours. Nothing to do with it but ignore it.
        .filter(|p| p.is_absolute());
    Some(IronWirePointer { port, token_path })
}

/// The port of an `http://` URL naming loopback, or nothing.
///
/// Hand-parsed rather than pulled through a URL crate: the accepted shape is
/// this narrow on purpose, and a general parser would accept more of it. The
/// rejections are the feature -- `https`, a hostname, a path, a port of zero
/// all mean this is not the local control API we know how to read, and
/// guessing at one of those is how a token reaches somewhere it should not.
fn loopback_port(control_url: &str) -> Option<u16> {
    let rest = control_url.strip_prefix("http://")?;
    // Everything up to the first `/`, `?` or `#` is the authority.
    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .filter(|a| !a.is_empty())?;
    // No userinfo: `http://user@127.0.0.1:8463` is not a shape IronWire
    // writes, and stripping the userinfo to keep going would accept a URL
    // built to look like loopback to a careless reader.
    if authority.contains('@') {
        return None;
    }
    let (host, port) = authority.rsplit_once(':')?;
    if !matches!(host, "127.0.0.1" | "localhost" | "[::1]") {
        return None;
    }
    // Port 0 is the ask-the-kernel sentinel, not a port anything listens on
    // -- the same value `probe_routing` refuses in its parameters.
    match port.parse::<u16>() {
        Ok(port) if port > 0 => Some(port),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::path::{Path, PathBuf};
    use std::sync::{Mutex, MutexGuard, PoisonError};

    static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);
    static LOCK: Mutex<()> = Mutex::new(());

    pub(super) fn override_path() -> Option<PathBuf> {
        OVERRIDE
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Decide what [`super::read_pointer`] sees for the life of the guard.
    ///
    /// Serialized on a mutex because the override is process-wide and the
    /// harness runs tests on threads, and cleared on drop so a test that
    /// panics does not leave the next one reading its tempdir.
    pub(crate) struct PointerAt {
        _lock: MutexGuard<'static, ()>,
    }

    impl PointerAt {
        pub(crate) fn set(path: &Path) -> Self {
            let guard = Self::none();
            *OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.to_path_buf());
            guard
        }

        /// Hold the lock with no pointer in place: the state of a machine
        /// without IronWire. A test asserting that state needs the lock as
        /// much as one setting a path, or a concurrent `set` decides what
        /// it sees.
        pub(crate) fn none() -> Self {
            let lock = LOCK.lock().unwrap_or_else(PoisonError::into_inner);
            *OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner) = None;
            Self { _lock: lock }
        }
    }

    impl Drop for PointerAt {
        fn drop(&mut self) {
            *OVERRIDE.lock().unwrap_or_else(PoisonError::into_inner) = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An absolute token path on the platform the test is running on.
    ///
    /// `/t/control.token` is absolute on Unix and **relative on Windows**,
    /// which has no drive letter in it — so a fixture written that way is
    /// silently rejected by the `is_absolute` filter above, and the test
    /// reads as "the pointer named no token" rather than as a bad fixture.
    /// That is exactly what it did: two tests passed everywhere and failed
    /// only on `windows-latest`.
    fn absolute(tail: &str) -> String {
        if cfg!(windows) {
            format!(r"C:\{tail}")
        } else {
            format!("/{tail}")
        }
    }

    /// The fixed layout. `IRONWIRE_HOME` does not move this file -- IronWire
    /// writes it under the real home directory whatever that variable says
    /// -- so looking for it anywhere else would find nothing on exactly the
    /// machines the variable is set on.
    #[test]
    fn the_pointer_sits_under_the_home_directory_not_ironwire_home() {
        assert_eq!(
            pointer_path_in(Path::new("/home/x")),
            PathBuf::from("/home/x/.ironwire/endpoint.json"),
        );
    }

    #[test]
    fn a_real_pointer_yields_the_port_and_the_token_path() {
        let token = absolute("custom/home/control.token");
        // Built with serde, not by interpolation: a Windows path carries
        // backslashes, which are invalid JSON escapes.
        let body = serde_json::to_string(&serde_json::json!({
            "control_url": "http://127.0.0.1:8463",
            "token_path": token,
        }))
        .expect("fixture serialises");

        let parsed = parse_pointer(&body).expect("the shape IronWire writes must parse");
        assert_eq!(parsed.port, 8463);
        assert_eq!(parsed.token_path, Some(PathBuf::from(&token)));
    }

    /// Unknown keys are ignored rather than refused: IronWire may add
    /// fields, and a reader that failed on one would stop discovering the
    /// day it did.
    #[test]
    fn an_unknown_field_does_not_stop_the_pointer_being_read() {
        let parsed =
            parse_pointer(r#"{"control_url":"http://127.0.0.1:8463","pid":4242,"version":"9"}"#)
                .expect("an unknown key must not refuse the document");
        assert_eq!(parsed.port, 8463);
        assert_eq!(parsed.token_path, None);
    }

    /// Every one of these means "nothing to discover". The assertion names
    /// the case so a regression says which one.
    #[test]
    fn nothing_this_reader_will_act_on_yields_no_pointer() {
        for (body, why) in [
            ("", "an empty file"),
            ("not json at all", "a body that is not JSON"),
            ("{", "truncated JSON, as a half-written file would be"),
            ("[]", "JSON that is not an object"),
            (r#"{"token_path":"/x/control.token"}"#, "no control_url"),
            (r#"{"control_url":null}"#, "an explicitly null control_url"),
            (r#"{"control_url":""}"#, "an empty control_url"),
            (r#"{"control_url":"http://127.0.0.1"}"#, "no port"),
            (r#"{"control_url":"http://127.0.0.1:0"}"#, "port zero"),
            (
                r#"{"control_url":"http://127.0.0.1:70000"}"#,
                "a port above 65535",
            ),
            (
                r#"{"control_url":"http://127.0.0.1:http"}"#,
                "a non-numeric port",
            ),
            (
                r#"{"control_url":"https://127.0.0.1:8463"}"#,
                "https, which the control client does not speak",
            ),
            (
                r#"{"control_url":"http://evil.example.com:8463"}"#,
                "a remote host",
            ),
            (
                r#"{"control_url":"http://127.0.0.1.evil.example.com:8463"}"#,
                "a host merely starting with the loopback address",
            ),
            (
                r#"{"control_url":"http://evil.example.com@127.0.0.1:8463"}"#,
                "a host hidden in front of loopback userinfo",
            ),
            (
                r#"{"control_url":"http://127.0.0.1@evil.example.com:8463"}"#,
                "loopback userinfo in front of a remote host",
            ),
            (r#"{"control_url":"http://:8463"}"#, "an empty host"),
            (
                r#"{"control_url":"127.0.0.1:8463"}"#,
                "an authority with no scheme",
            ),
        ] {
            assert_eq!(parse_pointer(body), None, "must be ignored: {why}");
        }
    }

    /// A path relative to the daemon's working directory is not one this
    /// process can resolve, so it is dropped -- while the port beside it is
    /// still good.
    #[test]
    fn a_relative_or_empty_token_path_is_dropped_but_the_port_survives() {
        for body in [
            r#"{"control_url":"http://127.0.0.1:8463","token_path":"control.token"}"#,
            r#"{"control_url":"http://127.0.0.1:8463","token_path":""}"#,
            r#"{"control_url":"http://127.0.0.1:8463","token_path":null}"#,
        ] {
            let parsed = parse_pointer(body).expect("the port is still usable");
            assert_eq!(parsed.port, 8463);
            assert_eq!(parsed.token_path, None, "for {body}");
        }
    }

    /// `localhost` and `[::1]` are loopback too. IronWire writes the dotted
    /// form, but accepting only that would make this reader break on a
    /// version that changed its mind about spelling.
    #[test]
    fn the_other_loopback_spellings_are_accepted() {
        for url in ["http://localhost:8463", "http://[::1]:8463"] {
            let body = format!(r#"{{"control_url":"{url}"}}"#);
            assert_eq!(
                parse_pointer(&body).map(|p| p.port),
                Some(8463),
                "must accept {url}",
            );
        }
    }

    /// A trailing path or query on the control URL is IronWire's business,
    /// not a reason to refuse: the port is the fact being read.
    #[test]
    fn a_path_or_query_after_the_authority_does_not_refuse_the_pointer() {
        for url in [
            "http://127.0.0.1:8463/",
            "http://127.0.0.1:8463/_ironwire",
            "http://127.0.0.1:8463?x=1",
        ] {
            let body = format!(r#"{{"control_url":"{url}"}}"#);
            assert_eq!(
                parse_pointer(&body).map(|p| p.port),
                Some(8463),
                "must accept {url}",
            );
        }
    }

    /// The rule the whole module exists to obey. No file is the ordinary
    /// state of a machine without IronWire, and it must reach the caller as
    /// "nothing to discover" rather than as anything to handle.
    #[test]
    fn a_missing_pointer_file_is_not_an_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let _at = test_support::PointerAt::set(&dir.path().join("endpoint.json"));
        assert_eq!(read_pointer(), None);
    }

    #[test]
    fn a_pointer_on_disk_is_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        let token = absolute("t/control.token");
        std::fs::write(
            &path,
            serde_json::to_string(&serde_json::json!({
                "control_url": "http://127.0.0.1:9111",
                "token_path": token,
            }))
            .expect("fixture serialises"),
        )
        .expect("write pointer");
        let _at = test_support::PointerAt::set(&path);

        assert_eq!(
            read_pointer(),
            Some(IronWirePointer {
                port: 9111,
                token_path: Some(PathBuf::from(&token)),
            }),
        );
    }

    /// A directory at the pointer's path is not a pointer. Named because
    /// `read_to_string` on one returns an error on unix but the check that
    /// stops it is a deliberate one, not an accident of the platform.
    #[test]
    fn a_directory_where_the_pointer_should_be_is_not_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        std::fs::create_dir(&path).expect("mkdir");
        let _at = test_support::PointerAt::set(&path);
        assert_eq!(read_pointer(), None);
    }

    #[test]
    fn an_oversized_pointer_is_refused_rather_than_read() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("endpoint.json");
        let padding = "x".repeat(MAX_POINTER_BYTES as usize);
        std::fs::write(
            &path,
            format!(r#"{{"control_url":"http://127.0.0.1:8463","pad":"{padding}"}}"#),
        )
        .expect("write pointer");
        let _at = test_support::PointerAt::set(&path);
        assert_eq!(read_pointer(), None);
    }

    /// The default in this crate's tests is a machine without IronWire, so
    /// no test anywhere in the binary reads the developer's real
    /// `~/.ironwire` and passes or fails according to what they have
    /// installed.
    #[test]
    fn tests_see_no_pointer_unless_they_ask_for_one() {
        let _none = test_support::PointerAt::none();
        assert_eq!(pointer_path(), None);
        assert_eq!(read_pointer(), None);
    }
}
