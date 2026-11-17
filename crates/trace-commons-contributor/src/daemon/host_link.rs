//! The host link: how a Flatpak-sandboxed window reaches the daemon running
//! on the host.
//!
//! Design: `docs/superpowers/specs/2026-11-17-linux-flatpak-host-daemon-design.md`.
//!
//! The daemon's primary socket lives in its 0700 state directory, beside the
//! device key, so a sandbox is never granted that directory. A host daemon on
//! its default state directory therefore serves a second listener at
//! `$XDG_RUNTIME_DIR/<state-dir-name>/daemon.sock`, and the Flatpak manifest
//! grants exactly `xdg-run/<state-dir-name>`. The same protocol and the same
//! dispatcher run behind it (`ipc::serve_connection`); the only additions are
//! a 0700 check on the runtime directory and an `SO_PEERCRED` uid check on
//! every accepted connection.
//!
//! The runtime directory also holds `watcher.lock`, which every daemon on its
//! default state directory -- on the host or in the sandbox -- holds for its
//! life. The sandbox and the host have different state directories and so
//! different `daemon.lock` files; this lock is what keeps two daemons from
//! watching the same sessions.
//!
//! On a clean stop the host daemon leaves its host-link socket file behind.
//! A socket file nobody listens on tells a sandboxed window "the host part is
//! installed and down", which must not start a second daemon in the sandbox.
//! `/run/user/<uid>` is cleared at logout, so the marker never outlives the
//! login.

use std::path::{Path, PathBuf};

/// Present in every Flatpak sandbox, absent everywhere else.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";
/// The host link's socket, inside the runtime directory.
pub const HOST_LINK_SOCK_FILE: &str = "daemon.sock";
/// The cross-sandbox watcher lock, inside the runtime directory.
pub const WATCHER_LOCK_FILE: &str = "watcher.lock";

/// Whether this process runs inside a Flatpak sandbox.
pub fn runs_sandboxed() -> bool {
    cfg!(target_os = "linux") && Path::new(FLATPAK_INFO_PATH).exists()
}

/// The runtime directory for this channel, from a value of
/// `XDG_RUNTIME_DIR`. `None` when unset, empty or relative: a relative
/// runtime directory would resolve against whatever the current directory
/// happens to be, which is not a place to put a socket.
pub fn runtime_dir_from(xdg_runtime_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let base = PathBuf::from(xdg_runtime_dir?);
    if !base.is_absolute() {
        return None;
    }
    Some(base.join(crate::channel::STATE_DIR_NAME))
}

/// The runtime directory for this channel, from the environment.
pub fn runtime_dir() -> Option<PathBuf> {
    runtime_dir_from(std::env::var_os("XDG_RUNTIME_DIR").as_deref())
}

/// The host link's socket path, from the environment. Inside the sandbox
/// this is the same path as on the host: Flatpak binds `xdg-run/...` at the
/// same location and passes `XDG_RUNTIME_DIR` through unchanged.
pub fn socket_path() -> Option<PathBuf> {
    runtime_dir().map(|dir| dir.join(HOST_LINK_SOCK_FILE))
}

/// What is at the host link's socket path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkState {
    /// A daemon accepts connections there.
    Listening,
    /// A socket file is there and nobody listens: the host part is installed
    /// and stopped, crashed, or restarting.
    Down,
    /// Nothing is there: no host part has run this login.
    Absent,
}

/// Look at the host link's socket. Never creates anything.
#[cfg(unix)]
pub fn probe(sock: &Path) -> LinkState {
    if std::fs::symlink_metadata(sock).is_err() {
        return LinkState::Absent;
    }
    match std::os::unix::net::UnixStream::connect(sock) {
        Ok(_) => LinkState::Listening,
        Err(_) => LinkState::Down,
    }
}

/// Where a window should get its daemon from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Route {
    /// Attach to the host daemon over this socket, whether or not it is
    /// listening right now. A host daemon that is down is reported by the
    /// window's existing not-running state, never replaced by a second one.
    Host(PathBuf),
    /// The ordinary path: the state directory's own socket, or host the
    /// daemon in-process.
    Local,
}

/// The preference order. Outside a sandbox nothing changes. Inside one, a
/// host link -- listening or left behind by a stopped host daemon -- wins
/// over running a daemon in the sandbox.
pub fn route(sandboxed: bool, sock: Option<&Path>, probe: impl Fn(&Path) -> LinkState) -> Route {
    if !sandboxed {
        return Route::Local;
    }
    let Some(sock) = sock else {
        return Route::Local;
    };
    match probe(sock) {
        LinkState::Listening | LinkState::Down => Route::Host(sock.to_path_buf()),
        LinkState::Absent => Route::Local,
    }
}

/// Another daemon on this login already holds `watcher.lock`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct WatcherHeld;

/// A daemon's hold on the runtime directory, for its whole life.
#[derive(Debug)]
pub(crate) struct Claim {
    /// Held, never read: dropping it releases the watcher lock.
    _lock: std::fs::File,
    /// Where to serve the host link. `None` in a sandbox, which never serves
    /// one: a sandboxed daemon is not the host part, and its socket file left
    /// behind would read as "host part installed".
    pub(crate) socket: Option<PathBuf>,
}

/// Take the watcher lock, and say where to serve the host link.
///
/// `Ok(None)` -- no arbitration, no host link -- when there is no runtime
/// directory, when this daemon is not on the platform default state
/// directory (a custom `TRACE_COMMONS_CONTRIBUTOR_DIR` daemon is somebody
/// running a second instance on purpose, and must not take the default's
/// runtime path), or when the runtime directory cannot be prepared. Only a
/// lock held by somebody else is an error.
///
/// In a sandbox the runtime directory is never created here: it exists only
/// when the manifest grants it, and creating one inside the sandbox's own
/// `/run/user` would arbitrate with nobody.
#[cfg(unix)]
pub(crate) fn claim(
    store_dir: &Path,
    platform_default: &Path,
    runtime: Option<PathBuf>,
    sandboxed: bool,
) -> Result<Option<Claim>, WatcherHeld> {
    let Some(runtime) = runtime else {
        return Ok(None);
    };
    if !same_dir(store_dir, platform_default) {
        return Ok(None);
    }
    if sandboxed {
        if !runtime.is_dir() {
            return Ok(None);
        }
    } else if prepare_private_dir(&runtime).is_err() {
        tracing::warn!("host link runtime directory unavailable");
        return Ok(None);
    }
    let Ok(lock) = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(runtime.join(WATCHER_LOCK_FILE))
    else {
        tracing::warn!("watcher lock unavailable");
        return Ok(None);
    };
    if lock.try_lock().is_err() {
        return Err(WatcherHeld);
    }
    Ok(Some(Claim {
        _lock: lock,
        socket: (!sandboxed).then(|| runtime.join(HOST_LINK_SOCK_FILE)),
    }))
}

/// Whether two paths name one directory. Canonical when both resolve, as
/// given otherwise: the systemd unit names the directory explicitly, and a
/// trailing slash or a symlinked `~/.config` must not read as a custom one.
fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Create the runtime directory, or take an existing one, at 0700. Refuses a
/// directory this user does not own: its mode would not be ours to set, and
/// a socket in somebody else's directory is reachable by them.
#[cfg(unix)]
fn prepare_private_dir(dir: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
    if !dir.exists() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() {
        return Err(std::io::Error::other("not-a-directory"));
    }
    if Some(meta.uid()) != super::ironwire_pointer::current_uid() {
        return Err(std::io::Error::other("not-owned"));
    }
    // Flatpak's `:create` may have made it before this daemon ran, with a
    // wider mode. It is ours, so tighten it.
    if meta.permissions().mode() & 0o777 != 0o700 {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Bind the host link's socket, replacing a file a stopped daemon left. Only
/// the holder of `watcher.lock` gets here, so nothing else is serving it.
#[cfg(unix)]
pub(crate) fn bind(sock: &Path) -> std::io::Result<tokio::net::UnixListener> {
    let _ = std::fs::remove_file(sock);
    tokio::net::UnixListener::bind(sock)
}

/// Whether a connection's peer runs as `uid`. A failed lookup is a refusal.
#[cfg(unix)]
pub(crate) fn peer_allowed(stream: &tokio::net::UnixStream, uid: u32) -> bool {
    stream
        .peer_cred()
        .map(|cred| cred.uid() == uid)
        .unwrap_or(false)
}

/// Serve the host link: the primary socket's dispatcher behind a peer uid
/// check. Runs until aborted.
#[cfg(unix)]
pub(crate) async fn serve(
    listener: tokio::net::UnixListener,
    shared: std::sync::Arc<super::ipc::DaemonShared>,
) {
    let Some(uid) = super::ironwire_pointer::current_uid() else {
        tracing::warn!("host link closed: own uid unknown");
        return;
    };
    loop {
        let Ok((stream, _addr)) = listener.accept().await else {
            continue;
        };
        if !peer_allowed(&stream, uid) {
            tracing::warn!("host link refused a peer");
            continue;
        }
        let shared = std::sync::Arc::clone(&shared);
        tokio::spawn(async move {
            let _ = super::ipc::serve_connection(stream, shared).await;
        });
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn the_runtime_dir_is_the_channel_directory_under_xdg_runtime_dir() {
        assert_eq!(
            runtime_dir_from(Some(OsStr::new("/run/user/1000"))),
            Some(PathBuf::from("/run/user/1000").join(crate::channel::STATE_DIR_NAME))
        );
    }

    #[test]
    fn no_runtime_dir_without_an_absolute_xdg_runtime_dir() {
        assert_eq!(runtime_dir_from(None), None);
        assert_eq!(runtime_dir_from(Some(OsStr::new(""))), None);
        assert_eq!(runtime_dir_from(Some(OsStr::new("run/user/1000"))), None);
    }

    #[test]
    fn probe_tells_listening_from_down_from_absent() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join(HOST_LINK_SOCK_FILE);
        assert_eq!(probe(&sock), LinkState::Absent);

        let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
        assert_eq!(probe(&sock), LinkState::Listening);

        // A stopped daemon's socket file, nobody accepting.
        drop(listener);
        assert_eq!(probe(&sock), LinkState::Down);

        // Any file there reads as down, not as absent.
        std::fs::remove_file(&sock).unwrap();
        std::fs::write(&sock, b"").unwrap();
        assert_eq!(probe(&sock), LinkState::Down);
    }

    #[test]
    fn probing_never_creates_the_runtime_directory() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join("missing").join(HOST_LINK_SOCK_FILE);
        assert_eq!(probe(&sock), LinkState::Absent);
        assert!(!dir.path().join("missing").exists());
    }

    #[test]
    fn outside_a_sandbox_the_host_link_is_never_preferred() {
        let sock = Path::new("/run/user/1000/trace-commons/daemon.sock");
        for state in [LinkState::Listening, LinkState::Down, LinkState::Absent] {
            assert_eq!(route(false, Some(sock), |_| state), Route::Local);
        }
    }

    #[test]
    fn in_a_sandbox_a_host_link_wins_listening_or_down() {
        let sock = Path::new("/run/user/1000/trace-commons/daemon.sock");
        assert_eq!(
            route(true, Some(sock), |_| LinkState::Listening),
            Route::Host(sock.to_path_buf())
        );
        assert_eq!(
            route(true, Some(sock), |_| LinkState::Down),
            Route::Host(sock.to_path_buf())
        );
    }

    #[test]
    fn in_a_sandbox_with_no_host_part_the_local_path_runs() {
        let sock = Path::new("/run/user/1000/trace-commons/daemon.sock");
        assert_eq!(route(true, Some(sock), |_| LinkState::Absent), Route::Local);
        assert_eq!(
            route(true, None, |_| panic!("nothing to probe")),
            Route::Local
        );
    }

    fn mode(path: &Path) -> u32 {
        std::fs::metadata(path).unwrap().permissions().mode() & 0o777
    }

    #[test]
    fn a_host_daemon_on_the_default_directory_claims_and_serves() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");

        let claim = claim(state.path(), state.path(), Some(runtime.clone()), false)
            .unwrap()
            .expect("claimed");
        assert_eq!(claim.socket, Some(runtime.join(HOST_LINK_SOCK_FILE)));
        assert_eq!(mode(&runtime), 0o700);
        assert!(runtime.join(WATCHER_LOCK_FILE).exists());
    }

    #[test]
    fn a_second_claim_on_the_same_login_is_refused_until_the_first_drops() {
        let state = tempfile::tempdir().unwrap();
        let sandbox_state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");

        // The window hosting in the sandbox took it first.
        std::fs::create_dir(&runtime).unwrap();
        let sandboxed = claim(
            sandbox_state.path(),
            sandbox_state.path(),
            Some(runtime.clone()),
            true,
        )
        .unwrap()
        .expect("sandbox claimed");
        assert_eq!(
            sandboxed.socket, None,
            "a sandbox never serves the host link"
        );

        // The host daemon, on its own state directory, is refused.
        assert_eq!(
            claim(state.path(), state.path(), Some(runtime.clone()), false).unwrap_err(),
            WatcherHeld
        );

        drop(sandboxed);
        assert!(
            claim(state.path(), state.path(), Some(runtime), false)
                .unwrap()
                .is_some()
        );
    }

    #[test]
    fn a_custom_state_directory_neither_claims_nor_serves() {
        let state = tempfile::tempdir().unwrap();
        let default = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        assert!(
            claim(state.path(), default.path(), Some(runtime.clone()), false)
                .unwrap()
                .is_none()
        );
        assert!(!runtime.exists());
    }

    #[test]
    fn no_runtime_dir_means_no_claim() {
        let state = tempfile::tempdir().unwrap();
        assert!(
            claim(state.path(), state.path(), None, false)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_sandbox_without_the_granted_directory_claims_nothing_and_creates_nothing() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        assert!(
            claim(state.path(), state.path(), Some(runtime.clone()), true)
                .unwrap()
                .is_none()
        );
        assert!(!runtime.exists());
    }

    #[test]
    fn a_wider_runtime_directory_flatpak_created_is_tightened() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o755)).unwrap();
        claim(state.path(), state.path(), Some(runtime.clone()), false)
            .unwrap()
            .expect("claimed");
        assert_eq!(mode(&runtime), 0o700);
    }

    #[tokio::test]
    async fn the_host_link_admits_only_its_own_uid() {
        let (a, _b) = tokio::net::UnixStream::pair().unwrap();
        let uid = super::super::ironwire_pointer::current_uid().unwrap();
        assert!(peer_allowed(&a, uid));
        assert!(!peer_allowed(&a, uid.wrapping_add(1)));
    }

    #[tokio::test]
    async fn binding_replaces_a_socket_file_a_stopped_daemon_left() {
        let dir = tempfile::tempdir().unwrap();
        let sock = dir.path().join(HOST_LINK_SOCK_FILE);
        std::fs::write(&sock, b"").unwrap();
        let _listener = bind(&sock).unwrap();
        assert_eq!(probe(&sock), LinkState::Listening);
    }
}
