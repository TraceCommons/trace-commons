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
//! "The host part is installed" is a separate file, `host-unit`, which only
//! the systemd user unit's daemon writes ([`runs_as_host_unit`]) and which it
//! leaves behind when it stops. A window in the sandbox that finds it, with
//! nobody listening on the socket, reports the host daemon as not running
//! rather than starting a second daemon in the sandbox. Every other daemon --
//! a one-off `daemon run`, a GTK window hosting its own on the host -- writes
//! no marker and removes its socket on close, so a stale socket alone never
//! reads as "installed". `daemon uninstall` removes the marker;
//! `/run/user/<uid>` is cleared at logout, so it never outlives the login
//! either way.

use std::path::{Path, PathBuf};

/// Present in every Flatpak sandbox, absent everywhere else.
pub const FLATPAK_INFO_PATH: &str = "/.flatpak-info";
/// The host link's socket, inside the runtime directory.
pub const HOST_LINK_SOCK_FILE: &str = "daemon.sock";
/// The cross-sandbox watcher lock, inside the runtime directory.
pub const WATCHER_LOCK_FILE: &str = "watcher.lock";
/// "The host part is installed", inside the runtime directory. Written only
/// by the systemd user unit's daemon; see the module doc.
pub const HOST_UNIT_MARKER_FILE: &str = "host-unit";

/// Whether a process with this `INVOCATION_ID` and this `/proc/self/cgroup`
/// text is the daemon systemd started for `unit`.
///
/// `INVOCATION_ID` alone is not enough: every process a service spawns
/// inherits it. The cgroup names the unit the process actually runs in --
/// `0::/user.slice/user-1000.slice/user@1000.service/app.slice/<unit>` under
/// cgroup v2, one `<n>:<controllers>:<path>` line each under v1 -- and a
/// process started from a terminal or a desktop launcher runs in a scope or
/// another service instead.
pub fn is_host_unit(invocation_id: Option<&std::ffi::OsStr>, cgroup: &str, unit: &str) -> bool {
    if invocation_id.is_none_or(|id| id.is_empty()) {
        return false;
    }
    let suffix = format!("/{unit}");
    cgroup.lines().any(|line| {
        // `<hierarchy-id>:<controllers>:<path>`; the path may hold colons.
        line.splitn(3, ':')
            .nth(2)
            .is_some_and(|path| path.ends_with(&suffix))
    })
}

/// Whether this process is the systemd user unit's daemon.
pub fn runs_as_host_unit() -> bool {
    if !cfg!(target_os = "linux") || runs_sandboxed() {
        return false;
    }
    let Ok(cgroup) = std::fs::read_to_string("/proc/self/cgroup") else {
        return false;
    };
    is_host_unit(
        std::env::var_os("INVOCATION_ID").as_deref(),
        &cgroup,
        crate::channel::SYSTEMD_UNIT_NAME,
    )
}

/// Remove the "host part is installed" marker from a runtime directory, for
/// `daemon uninstall`. A missing marker is not an error. Never follows a
/// symlink: `remove_file` unlinks the link itself.
pub fn forget_host_unit(runtime: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(runtime.join(HOST_UNIT_MARKER_FILE)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

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
    /// Nobody listens, and the systemd unit's marker is there: the host part
    /// is installed and stopped, crashed, or restarting.
    Down,
    /// Nobody listens and no marker: no installed host part has run this
    /// login. A socket file some other daemon left does not change that.
    Absent,
}

/// Look at the host link's socket, and beside it the unit's marker. Never
/// creates anything.
#[cfg(unix)]
pub fn probe(sock: &Path) -> LinkState {
    if std::fs::symlink_metadata(sock).is_ok()
        && std::os::unix::net::UnixStream::connect(sock).is_ok()
    {
        return LinkState::Listening;
    }
    match std::fs::symlink_metadata(sock.with_file_name(HOST_UNIT_MARKER_FILE)) {
        Ok(_) => LinkState::Down,
        Err(_) => LinkState::Absent,
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

/// Why a daemon may not watch.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum ClaimError {
    /// Another daemon on this login already holds `watcher.lock`.
    WatcherHeld,
    /// `watcher.lock` is not a regular file this daemon opened itself (a
    /// symlink, a directory, a file swapped in under it), or locking it
    /// failed for a reason other than another holder. The sandbox can write
    /// this directory, so this is refused rather than followed, and the
    /// daemon does not start unarbitrated.
    LockUnusable,
}

/// A daemon's hold on the runtime directory, for its whole life.
#[cfg(unix)]
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
    host_unit: bool,
) -> Result<Option<Claim>, ClaimError> {
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
    let lock = open_lock(&runtime.join(WATCHER_LOCK_FILE))?;
    match lock.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => return Err(ClaimError::WatcherHeld),
        Err(std::fs::TryLockError::Error(_)) => {
            tracing::warn!("watcher lock could not be taken");
            return Err(ClaimError::LockUnusable);
        }
    }
    if host_unit && !sandboxed && write_marker(&runtime.join(HOST_UNIT_MARKER_FILE)).is_err() {
        // Costs only the "installed" signal while this daemon is down; the
        // link itself still serves.
        tracing::warn!("host unit marker could not be written");
    }
    Ok(Some(Claim {
        _lock: lock,
        socket: (!sandboxed).then(|| runtime.join(HOST_LINK_SOCK_FILE)),
    }))
}

/// Open `watcher.lock` without following a symlink, in a directory the
/// sandbox can write. No `O_NOFOLLOW` without `libc`, so: create it with
/// `create_new` (`O_CREAT | O_EXCL`, which refuses any existing name,
/// a dangling symlink included), or else open the existing name without
/// `create` and require that what was opened is a regular file and the very
/// file the name holds now, not one a symlink led to.
#[cfg(unix)]
fn open_lock(path: &Path) -> Result<std::fs::File, ClaimError> {
    use std::os::unix::fs::MetadataExt;
    let unusable = |label: &str| {
        tracing::warn!("watcher lock unusable: {label}");
        ClaimError::LockUnusable
    };
    match std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => return Ok(file),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(_) => return Err(unusable("not-created")),
    }
    let named = std::fs::symlink_metadata(path).map_err(|_| unusable("vanished"))?;
    if !named.file_type().is_file() {
        return Err(unusable("not-a-regular-file"));
    }
    let file = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|_| unusable("not-opened"))?;
    let opened = file.metadata().map_err(|_| unusable("not-stated"))?;
    let named = std::fs::symlink_metadata(path).map_err(|_| unusable("vanished"))?;
    if !opened.file_type().is_file()
        || !named.file_type().is_file()
        || (opened.dev(), opened.ino()) != (named.dev(), named.ino())
    {
        return Err(unusable("replaced"));
    }
    Ok(file)
}

/// Write the empty "host part is installed" marker. Whatever is at the name
/// is unlinked first and the file made with `create_new`, so a symlink
/// planted there is replaced, never followed.
#[cfg(unix)]
fn write_marker(path: &Path) -> std::io::Result<()> {
    let _ = std::fs::remove_file(path);
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map(drop)
}

/// Whether two paths name one directory. Canonical when both resolve, as
/// given otherwise: the systemd unit names the directory explicitly, and a
/// trailing slash or a symlinked `~/.config` must not read as a custom one.
#[cfg(unix)]
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
    serve_as(listener, shared, uid).await
}

/// [`serve`] admitting only peers running as `uid`. A test passes a uid no
/// peer has, to see a refused connection get no answer.
#[cfg(unix)]
pub(crate) async fn serve_as(
    listener: tokio::net::UnixListener,
    shared: std::sync::Arc<super::ipc::DaemonShared>,
    uid: u32,
) {
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

        // A socket file some daemon left, nobody accepting, and no unit
        // marker: no installed host part. Reading this as "down" is what
        // kept a Flatpak attached to a dead socket for a whole login.
        drop(listener);
        assert_eq!(probe(&sock), LinkState::Absent);

        // The unit's marker is what says "installed": with it, down.
        std::fs::write(dir.path().join(HOST_UNIT_MARKER_FILE), b"").unwrap();
        assert_eq!(probe(&sock), LinkState::Down);

        // Installed and stopped, its socket already removed: still down.
        std::fs::remove_file(&sock).unwrap();
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

        let claim = claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            false,
            false,
        )
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
            false,
        )
        .unwrap()
        .expect("sandbox claimed");
        assert_eq!(
            sandboxed.socket, None,
            "a sandbox never serves the host link"
        );

        // The host daemon, on its own state directory, is refused.
        assert_eq!(
            claim(
                state.path(),
                state.path(),
                Some(runtime.clone()),
                false,
                false
            )
            .unwrap_err(),
            ClaimError::WatcherHeld
        );

        drop(sandboxed);
        assert!(
            claim(state.path(), state.path(), Some(runtime), false, false)
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
            claim(
                state.path(),
                default.path(),
                Some(runtime.clone()),
                false,
                false
            )
            .unwrap()
            .is_none()
        );
        assert!(!runtime.exists());
    }

    #[test]
    fn no_runtime_dir_means_no_claim() {
        let state = tempfile::tempdir().unwrap();
        assert!(
            claim(state.path(), state.path(), None, false, false)
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
            claim(
                state.path(),
                state.path(),
                Some(runtime.clone()),
                true,
                false
            )
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
        claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            false,
            false,
        )
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

    const UNIT: &str = "trace-commons-contributor.service";

    #[test]
    fn the_unit_is_recognised_from_its_cgroup_under_v2_and_v1() {
        let id = Some(OsStr::new("0123456789abcdef0123456789abcdef"));
        let v2 = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/trace-commons-contributor.service\n";
        assert!(is_host_unit(id, v2, UNIT));
        let v1 = "12:pids:/user.slice/user-1000.slice/user@1000.service\n\
                  1:name=systemd:/user.slice/user-1000.slice/user@1000.service/trace-commons-contributor.service\n";
        assert!(is_host_unit(id, v1, UNIT));
    }

    #[test]
    fn anything_but_the_unit_itself_is_not_the_unit() {
        let id = Some(OsStr::new("0123456789abcdef0123456789abcdef"));
        let unit_cgroup = "0::/user.slice/user-1000.slice/user@1000.service/app.slice/trace-commons-contributor.service\n";
        // No INVOCATION_ID, or an empty one: not started by systemd.
        assert!(!is_host_unit(None, unit_cgroup, UNIT));
        assert!(!is_host_unit(Some(OsStr::new("")), unit_cgroup, UNIT));
        // A terminal, a desktop launcher, another service that inherited an
        // INVOCATION_ID, a unit whose name only ends the same way.
        for cgroup in [
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.gnome.Terminal.slice/vte-spawn-1.scope\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/other.service\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/x-trace-commons-contributor.service\n",
            "0::/user.slice/user-1000.slice/user@1000.service/app.slice/trace-commons-contributor.service/child\n",
            "",
        ] {
            assert!(!is_host_unit(id, cgroup, UNIT), "{cgroup}");
        }
    }

    #[test]
    fn only_the_unit_writes_the_installed_marker() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");

        let one_off = claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            false,
            false,
        )
        .unwrap()
        .expect("claimed");
        assert!(!runtime.join(HOST_UNIT_MARKER_FILE).exists());
        drop(one_off);

        let _unit = claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            false,
            true,
        )
        .unwrap()
        .expect("claimed");
        assert!(runtime.join(HOST_UNIT_MARKER_FILE).is_file());
    }

    #[test]
    fn a_sandboxed_daemon_never_writes_the_installed_marker() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir(&runtime).unwrap();
        let _claim = claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            true,
            true,
        )
        .unwrap()
        .expect("claimed");
        assert!(!runtime.join(HOST_UNIT_MARKER_FILE).exists());
    }

    #[test]
    fn writing_the_marker_replaces_a_symlink_without_following_it() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir(&runtime).unwrap();
        let target = run.path().join("victim");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, runtime.join(HOST_UNIT_MARKER_FILE)).unwrap();

        let _unit = claim(
            state.path(),
            state.path(),
            Some(runtime.clone()),
            false,
            true,
        )
        .unwrap()
        .expect("claimed");
        let marker = std::fs::symlink_metadata(runtime.join(HOST_UNIT_MARKER_FILE)).unwrap();
        assert!(marker.file_type().is_file());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn uninstall_forgets_the_marker_and_never_follows_a_link() {
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir(&runtime).unwrap();
        std::fs::write(runtime.join(HOST_UNIT_MARKER_FILE), b"").unwrap();
        forget_host_unit(&runtime).unwrap();
        assert!(!runtime.join(HOST_UNIT_MARKER_FILE).exists());
        // Nothing there, nothing to do.
        forget_host_unit(&runtime).unwrap();
        forget_host_unit(&run.path().join("missing")).unwrap();

        let target = run.path().join("victim");
        std::fs::write(&target, b"keep").unwrap();
        std::os::unix::fs::symlink(&target, runtime.join(HOST_UNIT_MARKER_FILE)).unwrap();
        forget_host_unit(&runtime).unwrap();
        assert!(std::fs::symlink_metadata(runtime.join(HOST_UNIT_MARKER_FILE)).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"keep");
    }

    #[test]
    fn a_symlinked_watcher_lock_is_refused_and_its_target_untouched() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir(&runtime).unwrap();
        // A link to a file that exists, and one to a file that does not:
        // following the second would create it.
        let existing = run.path().join("existing");
        std::fs::write(&existing, b"keep").unwrap();
        let missing = run.path().join("created-through-link");
        for target in [&existing, &missing] {
            let lock = runtime.join(WATCHER_LOCK_FILE);
            let _ = std::fs::remove_file(&lock);
            std::os::unix::fs::symlink(target, &lock).unwrap();
            assert_eq!(
                claim(
                    state.path(),
                    state.path(),
                    Some(runtime.clone()),
                    false,
                    false
                )
                .unwrap_err(),
                ClaimError::LockUnusable,
                "{}",
                target.display()
            );
        }
        assert_eq!(std::fs::read(&existing).unwrap(), b"keep");
        assert!(!missing.exists());
    }

    #[test]
    fn a_watcher_lock_that_is_not_a_regular_file_is_refused() {
        let state = tempfile::tempdir().unwrap();
        let run = tempfile::tempdir().unwrap();
        let runtime = run.path().join("trace-commons");
        std::fs::create_dir_all(runtime.join(WATCHER_LOCK_FILE)).unwrap();
        assert_eq!(
            claim(state.path(), state.path(), Some(runtime), false, false).unwrap_err(),
            ClaimError::LockUnusable
        );
    }
}
