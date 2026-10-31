//! A bounded, local, payload-free record of the daemon's panics and of the
//! warn/error tracing events raised inside it, for diagnosing a failure
//! after the fact.
//!
//! Why it exists: the macOS app hosts the daemon (and IronWire's proxy)
//! in-process with stdout and stderr on `/dev/null`, so a panic in a
//! request handler left nothing behind. Pointing stderr at a file is not
//! the fix, because Rust's default panic hook prints the panic *message*,
//! and a message can quote request content -- `byte index N is not a char
//! boundary ... of `<the prompt>`` is the classic case. Prompt text must
//! never reach disk this way.
//!
//! So everything written here is static code information, never data:
//!
//! * a panic is its source location (file:line:column), the thread's name
//!   and a timestamp -- never the payload;
//! * a tracing event is its level, target (module path) and callsite
//!   file:line -- never its message or its fields. IronWire's events carry
//!   paths and upstream error strings (which can hold URLs) in fields, and
//!   a message is `fmt::Arguments` that a subscriber sees only through
//!   `Debug`, so a literal cannot be told from an interpolation. The
//!   callsite plus the pinned revision identifies the event exactly.
//!
//! Source paths are shortened to `<crate>/src/...`: a dependency's
//! `file!()` is absolute and runs through the build machine's home
//! directory, which carries a username. A path whose kept part could still
//! be a username is cut to its file name (see [`short_source_path`]).
//!
//! Two files in the state directory, both mode 0600, append-only and each
//! capped at [`DIAGNOSTICS_LOG_CAP_BYTES`]: `daemon-panics.log` for panics
//! and `daemon-errors.log` for warn/error events, so a storm of repeated
//! warnings cannot rotate a panic record away. At the cap a file is renamed
//! to `<name>.1` (replacing the previous one) and a fresh file starts; if
//! that rename fails, the file starts over in place rather than stop
//! recording. Nothing reads them back: they are for a person looking at the
//! state directory.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use crate::config::ConfigStore;

/// File name of the warn/error event log inside the daemon's state directory.
pub const DIAGNOSTICS_LOG_FILE: &str = "daemon-errors.log";

/// File name of the panic log inside the daemon's state directory. Panics
/// have their own file so a storm of repeated warn events cannot rotate the
/// one panic record out of reach.
pub const DIAGNOSTICS_PANIC_LOG_FILE: &str = "daemon-panics.log";

/// Size at which a current file is rotated to `.1`.
pub const DIAGNOSTICS_LOG_CAP_BYTES: u64 = 1024 * 1024;

/// How many times the panic hook tries the lock, 1 ms apart, before it
/// appends without it.
const HOOK_LOCK_TRIES: usize = 5;

/// One bounded, append-only log file plus its single rotated predecessor.
pub struct DiagnosticsLog {
    path: PathBuf,
    cap: u64,
    // Serializes writers, and holds the open file between appends so an
    // event costs one `write`, not an open/stat/close per line.
    state: Mutex<Option<OpenLog>>,
}

struct OpenLog {
    file: std::fs::File,
    /// Bytes in the file, tracked here rather than re-read per line. The
    /// daemon's lock means one process writes a state directory's log.
    len: u64,
}

impl DiagnosticsLog {
    pub fn new(path: PathBuf, cap: u64) -> Self {
        Self {
            path,
            cap,
            state: Mutex::new(None),
        }
    }

    /// The rotated predecessor: the same name with `.1` appended.
    pub fn rotated_path(&self) -> PathBuf {
        let mut name = self.path.as_os_str().to_owned();
        name.push(".1");
        PathBuf::from(name)
    }

    /// Append one line (a newline is added), rotating first if it would
    /// take the file past the cap.
    pub fn append_line(&self, line: &str) -> std::io::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.append_locked(&mut state, line)
    }

    /// The same as [`Self::append_line`] for the panic hook, which must
    /// neither block on a writer (it may be the panicking thread itself) nor
    /// stall a worker: after a few tries it appends without the lock, one
    /// `write` under `O_APPEND`, skipping rotation for that one line.
    fn append_line_from_hook(&self, line: &str) {
        for _ in 0..HOOK_LOCK_TRIES {
            match self.state.try_lock() {
                Ok(mut state) => {
                    let _ = self.append_locked(&mut state, line);
                    return;
                }
                Err(std::sync::TryLockError::Poisoned(poisoned)) => {
                    let mut state = poisoned.into_inner();
                    let _ = self.append_locked(&mut state, line);
                    return;
                }
                Err(std::sync::TryLockError::WouldBlock) => {
                    std::thread::sleep(std::time::Duration::from_millis(1));
                }
            }
        }
        if let Ok(mut file) = open_owner_only(&self.path, false) {
            let _ = file.write_all(&line_bytes(line));
        }
    }

    fn append_locked(&self, state: &mut Option<OpenLog>, line: &str) -> std::io::Result<()> {
        let bytes = line_bytes(line);
        let incoming = bytes.len() as u64;
        if state.is_none() {
            let file = open_owner_only(&self.path, false)?;
            let len = file.metadata()?.len();
            *state = Some(OpenLog { file, len });
        }
        let current = state.as_ref().map_or(0, |open| open.len);
        if current > 0 && current + incoming > self.cap {
            // Close first: Windows refuses to rename an open file.
            *state = None;
            let file = match self.rotate() {
                Ok(()) => open_owner_only(&self.path, false)?,
                // Without a rotation the cap can only hold by starting this
                // file over. Losing the older lines beats losing every line
                // from here on, which is what giving up here would do.
                Err(_) => open_owner_only(&self.path, true)?,
            };
            *state = Some(OpenLog { file, len: 0 });
        }
        let open = state.as_mut().expect("opened above");
        // One write per line, so a concurrent `O_APPEND` writer (the panic
        // hook's lock-free fallback) cannot split it.
        let written = open.file.write_all(&bytes);
        if written.is_err() {
            // Reopen next time rather than keep a handle that failed.
            *state = None;
        } else {
            open.len += incoming;
        }
        written
    }

    /// Move the current file to `.1`, replacing the previous one: one
    /// predecessor, never two. A `.1` that `rename` cannot replace (a
    /// read-only file on some platforms) is removed and the rename retried.
    fn rotate(&self) -> std::io::Result<()> {
        let rotated = self.rotated_path();
        match std::fs::rename(&self.path, &rotated) {
            Ok(()) => Ok(()),
            Err(_) => {
                std::fs::remove_file(&rotated)?;
                std::fs::rename(&self.path, &rotated)
            }
        }
    }
}

fn line_bytes(line: &str) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(line.len() + 1);
    bytes.extend_from_slice(line.as_bytes());
    bytes.push(b'\n');
    bytes
}

/// Open the log for appending, or, with `truncate`, empty it first. Either
/// way the file is owner-only.
fn open_owner_only(path: &Path, truncate: bool) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true);
    if truncate {
        options.write(true).truncate(true);
    } else {
        options.append(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    // `mode` applies only on creation; a file that already existed wider
    // (an older build, a hand-made file) is narrowed.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o777 != 0o600 {
            file.set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
    }
    Ok(file)
}

/// Shorten a compile-time source path to `<crate>/src/...`, dropping any
/// directory above the crate (a dependency's path is absolute and runs
/// through the build machine's home directory).
///
/// Keeps only the file name when the path has no `src` component, and also
/// when the directory kept above `src` could be a username rather than a
/// crate: it sits within two components of an absolute root (`/Users/alice`,
/// `/home/bob`, `C:\Users\carol`, `D:\carol`) or directly under a `Users` or
/// `home` directory. Code generated under a `target/` directory keeps only
/// its file name too.
pub fn short_source_path(file: &str) -> String {
    let parts: Vec<&str> = file
        .split(['/', '\\'])
        .filter(|part| !part.is_empty())
        .collect();
    let file_name = || parts.last().copied().unwrap_or("?").to_string();
    let Some(src) = parts.iter().rposition(|part| *part == "src") else {
        return file_name();
    };
    if src == 0 {
        return file_name();
    }
    let crate_dir = src - 1;
    let drive = parts
        .first()
        .is_some_and(|first| first.len() == 2 && first.ends_with(':'));
    let absolute = file.starts_with(['/', '\\']) || drive;
    let depth_below_root = crate_dir.saturating_sub(usize::from(drive));
    let under_home = crate_dir >= 1
        && ["users", "home"].contains(&parts[crate_dir - 1].to_ascii_lowercase().as_str());
    let kept = &parts[crate_dir..];
    if (absolute && depth_below_root <= 1) || under_home || kept.contains(&"target") {
        return file_name();
    }
    kept.join("/")
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// A thread name is set by code, but keep the record to a plain token
/// anyway: no spaces, no control characters, bounded length.
fn thread_label(name: Option<&str>) -> String {
    let label: String = name
        .unwrap_or("unnamed")
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
        .take(64)
        .collect();
    if label.is_empty() {
        "unnamed".to_string()
    } else {
        label
    }
}

fn location_label(location: Option<&std::panic::Location<'_>>) -> String {
    match location {
        Some(at) => format!(
            "{}:{}:{}",
            short_source_path(at.file()),
            at.line(),
            at.column()
        ),
        None => "unknown".to_string(),
    }
}

/// Where the installed hook and subscriber write.
struct Sinks {
    events: Arc<DiagnosticsLog>,
    panics: Arc<DiagnosticsLog>,
}

/// `None` until `install`.
fn global_sink() -> &'static Mutex<Option<Sinks>> {
    static SINK: OnceLock<Mutex<Option<Sinks>>> = OnceLock::new();
    SINK.get_or_init(|| Mutex::new(None))
}

fn current_global_log(pick: fn(&Sinks) -> &Arc<DiagnosticsLog>) -> Option<Arc<DiagnosticsLog>> {
    // `try_lock`: the hook may run on a thread that panicked while
    // `install` held this lock. Missing a record beats a deadlock.
    let read = |sink: &Option<Sinks>| sink.as_ref().map(|s| Arc::clone(pick(s)));
    match global_sink().try_lock() {
        Ok(sink) => read(&sink),
        Err(std::sync::TryLockError::Poisoned(poisoned)) => read(&poisoned.into_inner()),
        Err(std::sync::TryLockError::WouldBlock) => None,
    }
}

/// The panic hook. Never reads `info.payload()` and never calls `tracing`
/// (that would re-enter the subscriber below).
///
/// It must not panic either: a panic inside a panic hook aborts. The one
/// call that can is `std::thread::current()`, and only once std has torn
/// down the thread's handle, which it does after every Rust thread-local
/// destructor has run (`rt::thread_cleanup`). A panic from a thread-local's
/// or a runtime's `Drop` therefore still gets its name; only code a foreign
/// TLS destructor runs after that point could reach the panicking branch,
/// and no host here runs Rust from one.
///
/// The record is written before anything slow: the stderr line, then, only
/// when `RUST_BACKTRACE` asks for one, a backtrace on stderr. Frames are
/// code, not data, but they go to stderr alone, never to the file.
fn panic_hook(info: &std::panic::PanicHookInfo<'_>) {
    let location = location_label(info.location());
    let thread = thread_label(std::thread::current().name());
    let line = format!("{} panic at {location} thread={thread}", timestamp());
    if let Some(log) = current_global_log(|sinks| &sinks.panics) {
        log.append_line_from_hook(&line);
    }
    // Keep a payload-free line on stderr for a terminal-run daemon, which
    // would otherwise see nothing now that the default hook is replaced.
    let _ = writeln!(
        std::io::stderr(),
        "trace-commons daemon: panic at {location} on thread {thread} (message withheld)"
    );
    let backtrace = std::backtrace::Backtrace::capture();
    if backtrace.status() == std::backtrace::BacktraceStatus::Captured {
        let _ = writeln!(std::io::stderr(), "{backtrace}");
    }
}

/// The two log paths for a store: warn/error events, then panics.
pub fn paths_for(store: &ConfigStore) -> (PathBuf, PathBuf) {
    (
        store.daemon_path(DIAGNOSTICS_LOG_FILE),
        store.daemon_path(DIAGNOSTICS_PANIC_LOG_FILE),
    )
}

/// Install the log for a daemon about to run against `store`, and return
/// the store it should run against. Every host (the CLI's `daemon run`, the
/// FFI handle, the GTK shell) calls this, so none can skip it or point it at
/// the wrong directory: the dev dry-run store is applied first (a no-op when
/// already applied), so a dry run never writes the real state directory.
pub fn install_for(store: ConfigStore) -> anyhow::Result<ConfigStore> {
    let store = crate::daemon::dev_dry_run_store(store)?;
    let (events, panics) = paths_for(&store);
    install(&events, &panics);
    Ok(store)
}

/// Install the panic hook and the warn/error subscriber, and point them at
/// `events_path` and `panics_path`. Safe to call more than once: the hook
/// and subscriber are installed on the first call only; a later call moves
/// the destination.
///
/// The hook **replaces** whatever was installed before rather than chaining
/// to it. The previous hook is std's default (nothing in this process sets
/// another), and that hook's whole output is the payload; chaining to it
/// would print exactly what this module exists to keep off disk.
pub fn install(events_path: &Path, panics_path: &Path) {
    {
        let mut sink = global_sink()
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let unchanged = sink
            .as_ref()
            .is_some_and(|s| s.events.path == events_path && s.panics.path == panics_path);
        if !unchanged {
            *sink = Some(Sinks {
                events: Arc::new(DiagnosticsLog::new(
                    events_path.to_path_buf(),
                    DIAGNOSTICS_LOG_CAP_BYTES,
                )),
                panics: Arc::new(DiagnosticsLog::new(
                    panics_path.to_path_buf(),
                    DIAGNOSTICS_LOG_CAP_BYTES,
                )),
            });
        }
    }
    static INSTALLED: std::sync::Once = std::sync::Once::new();
    INSTALLED.call_once(|| {
        std::panic::set_hook(Box::new(panic_hook));
        // An `Err` means another global subscriber is already in place;
        // leave it be. Nothing in this process sets one today.
        let _ = tracing::subscriber::set_global_default(MetadataOnlySubscriber { log: None });
    });
}

/// A `tracing` subscriber that persists warn/error events' metadata only:
/// level, target and callsite. Never the message, never a field. Spans are
/// not tracked at all.
pub struct MetadataOnlySubscriber {
    /// `None` writes wherever [`install`] last pointed the global sink.
    log: Option<Arc<DiagnosticsLog>>,
}

impl MetadataOnlySubscriber {
    /// A subscriber with its own destination, for a scoped
    /// `tracing::subscriber::with_default`.
    pub fn writing_to(log: Arc<DiagnosticsLog>) -> Self {
        Self { log: Some(log) }
    }

    fn wants(metadata: &tracing::Metadata<'_>) -> bool {
        // `Level` orders by verbosity: ERROR < WARN < INFO.
        metadata.is_event() && *metadata.level() <= tracing::Level::WARN
    }
}

impl tracing::Subscriber for MetadataOnlySubscriber {
    fn register_callsite(
        &self,
        metadata: &'static tracing::Metadata<'static>,
    ) -> tracing::subscriber::Interest {
        if Self::wants(metadata) {
            tracing::subscriber::Interest::always()
        } else {
            tracing::subscriber::Interest::never()
        }
    }

    fn enabled(&self, metadata: &tracing::Metadata<'_>) -> bool {
        Self::wants(metadata)
    }

    fn max_level_hint(&self) -> Option<tracing::level_filters::LevelFilter> {
        Some(tracing::level_filters::LevelFilter::WARN)
    }

    fn new_span(&self, _: &tracing::span::Attributes<'_>) -> tracing::span::Id {
        // Spans are never enabled, so this id is never acted on.
        tracing::span::Id::from_u64(1)
    }

    fn record(&self, _: &tracing::span::Id, _: &tracing::span::Record<'_>) {}

    fn record_follows_from(&self, _: &tracing::span::Id, _: &tracing::span::Id) {}

    fn event(&self, event: &tracing::Event<'_>) {
        let metadata = event.metadata();
        if !Self::wants(metadata) {
            return;
        }
        let log = match &self.log {
            Some(log) => Some(Arc::clone(log)),
            None => current_global_log(|sinks| &sinks.events),
        };
        let Some(log) = log else {
            return;
        };
        let file = metadata.file().map(short_source_path);
        let line = format!(
            "{} {} {} {}:{}",
            timestamp(),
            metadata.level(),
            metadata.target(),
            file.as_deref().unwrap_or("?"),
            metadata
                .line()
                .map_or_else(|| "?".to_string(), |n| n.to_string()),
        );
        let _ = log.append_line(&line);
    }

    fn enter(&self, _: &tracing::span::Id) {}

    fn exit(&self, _: &tracing::span::Id) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &str = "SECRET-PROMPT-7f3a9c-do-not-persist";
    const CHILD_ENV: &str = "TC_DIAGNOSTICS_LOG_CHILD_DIR";

    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).unwrap_or_default()
    }

    #[test]
    fn short_source_path_drops_the_build_machines_home_directory() {
        assert_eq!(
            short_source_path(
                "/Users/alice/.cargo/git/checkouts/ironwire-0123abcd/7ac9251/crates/ironwire_proxy/src/pipeline.rs"
            ),
            "ironwire_proxy/src/pipeline.rs"
        );
        assert_eq!(
            short_source_path(
                "/home/bob/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/hyper-1.6.0/src/proto/h1/role.rs"
            ),
            "hyper-1.6.0/src/proto/h1/role.rs"
        );
        assert_eq!(
            short_source_path("/rustc/abc123/library/core/src/str/mod.rs"),
            "core/src/str/mod.rs"
        );
        assert_eq!(
            short_source_path(
                r"C:\Users\carol\.cargo\git\checkouts\x\crates\ironwire_core\src\config.rs"
            ),
            "ironwire_core/src/config.rs"
        );
        // A workspace crate's path is already relative; it keeps the crate
        // directory onward too.
        assert_eq!(
            short_source_path("crates/trace-commons-contributor/src/daemon/ipc.rs"),
            "trace-commons-contributor/src/daemon/ipc.rs"
        );
        assert_eq!(short_source_path("/Users/alice/build.rs"), "build.rs");
        assert!(!short_source_path("/Users/alice/x/src/a.rs").contains("alice"));
    }

    #[test]
    fn short_source_path_never_keeps_a_home_level_src_parent() {
        // A `src` directly under a home directory (or a drive root) is not a
        // crate's `src`: the directory above it is a username. Generated code
        // under OUT_DIR is the usual way such a path reaches `file!()`.
        for (path, want) in [
            (
                "/Users/alice/src/tc/target/release/build/foo-1a2b/out/gen.rs",
                "gen.rs",
            ),
            (r"C:\Users\carol\src\x\build.rs", "build.rs"),
            (r"D:\carol\src\x\build.rs", "build.rs"),
            ("/home/bob/src/main.rs", "main.rs"),
            (r"C:\src\main.rs", "main.rs"),
            ("/src/main.rs", "main.rs"),
            ("/Volumes/Data/Users/dave/src/lib.rs", "lib.rs"),
            (
                "/opt/ci/projects/src/tc/target/debug/build/foo-1a2b/out/gen.rs",
                "gen.rs",
            ),
        ] {
            assert_eq!(short_source_path(path), want, "{path}");
        }
    }

    #[test]
    fn every_host_installs_into_its_stores_state_directory() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = ConfigStore::open(dir.path().to_path_buf()).expect("store");
        let (events, panics) = paths_for(&store);
        assert_eq!(events, store.daemon_path(DIAGNOSTICS_LOG_FILE));
        assert_eq!(panics, store.daemon_path(DIAGNOSTICS_PANIC_LOG_FILE));
        assert_ne!(events, panics);
    }

    #[test]
    fn the_hooks_fallback_append_lands_while_a_writer_holds_the_lock() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = DiagnosticsLog::new(dir.path().join(DIAGNOSTICS_PANIC_LOG_FILE), 1024);
        log.append_line("before").unwrap();
        let held = log.state.lock().unwrap();
        let started = std::time::Instant::now();
        log.append_line_from_hook("from-hook");
        assert!(started.elapsed() < std::time::Duration::from_millis(100));
        drop(held);
        log.append_line("after").unwrap();
        assert_eq!(read(&log.path), "before\nfrom-hook\nafter\n");
    }

    #[test]
    fn a_failed_rotation_still_records_the_line_and_keeps_the_cap() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = DiagnosticsLog::new(dir.path().join(DIAGNOSTICS_LOG_FILE), 100);
        // A non-empty directory where `.1` should go: `rename` over it fails
        // on every platform, and so does removing it as a file.
        std::fs::create_dir(log.rotated_path()).unwrap();
        std::fs::write(log.rotated_path().join("keep"), "x").unwrap();
        let line = "x".repeat(39);
        for tag in ['a', 'b', 'c', 'd', 'e'] {
            log.append_line(&format!("{tag}{line}")).unwrap();
        }
        let current = read(&log.path);
        assert!(current.contains(&format!("e{line}")), "{current}");
        assert!(std::fs::metadata(&log.path).unwrap().len() <= 100);
    }

    #[test]
    fn append_rotates_at_the_cap_and_keeps_one_previous_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = DiagnosticsLog::new(dir.path().join(DIAGNOSTICS_LOG_FILE), 100);
        let line = "x".repeat(39); // 40 bytes with the newline
        log.append_line(&format!("a{line}")).unwrap();
        log.append_line(&format!("b{line}")).unwrap();
        // 80 bytes so far; a third 41-byte line would pass 100.
        log.append_line(&format!("c{line}")).unwrap();
        let current = read(&log.path);
        let previous = read(&log.rotated_path());
        assert!(current.starts_with('c') && current.lines().count() == 1);
        assert!(previous.starts_with('a') && previous.lines().count() == 2);

        log.append_line(&format!("d{line}")).unwrap();
        log.append_line(&format!("e{line}")).unwrap();
        // c and d rotate over the a/b file; there is never a `.2`.
        let previous = read(&log.rotated_path());
        assert!(previous.starts_with('c'), "{previous}");
        assert!(read(&log.path).starts_with('e'));
        assert!(
            !dir.path()
                .join(format!("{DIAGNOSTICS_LOG_FILE}.2"))
                .exists()
        );
        for path in [&log.path, &log.rotated_path()] {
            assert!(std::fs::metadata(path).unwrap().len() <= 100);
        }
    }

    #[test]
    fn append_only_never_truncates_existing_lines() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DIAGNOSTICS_LOG_FILE);
        std::fs::write(&path, "earlier\n").unwrap();
        let log = DiagnosticsLog::new(path.clone(), 1024);
        log.append_line("later").unwrap();
        assert_eq!(read(&path), "earlier\nlater\n");
    }

    #[cfg(unix)]
    #[test]
    fn log_files_are_owner_only_even_if_they_existed_wider() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join(DIAGNOSTICS_LOG_FILE);
        std::fs::write(&path, "").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        let log = DiagnosticsLog::new(path.clone(), 16);
        log.append_line("0123456789").unwrap();
        log.append_line("0123456789").unwrap(); // rotates
        for p in [&path, &log.rotated_path()] {
            let mode = std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600, "{}", p.display());
        }
    }

    #[test]
    fn a_warn_event_records_its_callsite_and_never_its_message_or_fields() {
        let dir = tempfile::tempdir().expect("tempdir");
        let log = Arc::new(DiagnosticsLog::new(
            dir.path().join(DIAGNOSTICS_LOG_FILE),
            DIAGNOSTICS_LOG_CAP_BYTES,
        ));
        let subscriber = MetadataOnlySubscriber::writing_to(Arc::clone(&log));
        tracing::subscriber::with_default(subscriber, || {
            tracing::warn!(url = SECRET, error = %SECRET, "upstream failed: {}", SECRET);
            tracing::error!("plain literal {}", SECRET);
            tracing::info!("info is not persisted");
        });
        let body = read(&log.path);
        assert!(!body.contains(SECRET), "{body}");
        assert!(!body.contains("upstream failed"), "{body}");
        assert!(!body.contains("info is not persisted"), "{body}");
        let lines: Vec<&str> = body.lines().collect();
        assert_eq!(lines.len(), 2, "{body}");
        assert!(lines[0].contains(" WARN "), "{body}");
        assert!(lines[1].contains(" ERROR "), "{body}");
        for line in lines {
            assert!(
                line.contains("trace_commons_contributor::daemon::diagnostics_log::tests"),
                "{line}"
            );
            assert!(
                line.contains("trace-commons-contributor/src/daemon/diagnostics_log.rs:"),
                "{line}"
            );
        }
    }

    /// The installed hook, exercised in a child process: `set_hook` is
    /// process-wide, so running it in this test binary would swallow every
    /// other test's panic message. The parent re-runs this binary filtered
    /// to `installed_hook_child`, which installs twice, panics on a named
    /// thread with a secret in the message, and emits a warn event.
    #[test]
    fn installed_hook_records_location_never_payload_and_installs_once() {
        let dir = tempfile::tempdir().expect("tempdir");
        let exe = std::env::current_exe().expect("test binary");
        let out = std::process::Command::new(exe)
            .args([
                "--exact",
                "daemon::diagnostics_log::tests::installed_hook_child",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(CHILD_ENV, dir.path())
            .output()
            .expect("run child");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "child failed:\n{stdout}\n{stderr}");
        assert!(stdout.contains("1 passed"), "child did not run:\n{stdout}");
        assert!(
            !stderr.contains(SECRET),
            "payload reached stderr:\n{stderr}"
        );
        assert!(
            !stdout.contains(SECRET),
            "payload reached stdout:\n{stdout}"
        );
        assert!(stderr.contains("diagnostics_log.rs:"), "{stderr}");

        let events_path = dir.path().join(DIAGNOSTICS_LOG_FILE);
        let panics_path = dir.path().join(DIAGNOSTICS_PANIC_LOG_FILE);
        let events = read(&events_path);
        let body = read(&panics_path);
        assert!(!body.contains(SECRET), "{body}");
        assert!(!events.contains(SECRET), "{events}");
        let panics: Vec<&str> = body.lines().filter(|l| l.contains(" panic ")).collect();
        assert_eq!(panics.len(), 1, "one panic, one record:\n{body}");
        assert_eq!(body.lines().count(), 1, "only panics here:\n{body}");
        assert!(
            panics[0].contains("trace-commons-contributor/src/daemon/diagnostics_log.rs:"),
            "{body}"
        );
        assert!(panics[0].contains("thread=diag-test-worker"), "{body}");
        assert!(
            !events.contains(" panic "),
            "panics have their own file:\n{events}"
        );
        let warns = events.lines().filter(|l| l.contains(" WARN ")).count();
        assert_eq!(
            warns, 1,
            "the global subscriber records the event:\n{events}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            for path in [&events_path, &panics_path] {
                let mode = std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
                assert_eq!(mode, 0o600, "{}", path.display());
            }
        }
    }

    /// Only does anything as the child of the test above.
    #[test]
    fn installed_hook_child() {
        let Some(dir) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let events = Path::new(&dir).join(DIAGNOSTICS_LOG_FILE);
        let panics = Path::new(&dir).join(DIAGNOSTICS_PANIC_LOG_FILE);
        install(&events, &panics);
        install(&events, &panics);
        let joined = std::thread::Builder::new()
            .name("diag-test-worker".into())
            .spawn(|| {
                let prompt = format!("byte index 3 is not a char boundary of `{SECRET}`");
                panic!("{prompt}");
            })
            .expect("spawn")
            .join();
        assert!(joined.is_err());
        tracing::warn!(secret = SECRET, "leak {}", SECRET);
    }
}
