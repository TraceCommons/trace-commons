//! Local file primitives shared by the contributor and its Insights store:
//! the durable 0600 atomic write, and the no-follow, nonblocking opener for
//! a file the contributor selected.
//!
//! Moved here from `trace_commons_contributor::config` and
//! `trace_commons_contributor::evidence_import`, which re-export both at
//! their old paths.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use uuid::Uuid;

/// Write `body` to `path` atomically (temp file in the same dir, then
/// rename), setting 0600 permissions on unix.
pub fn write_atomic_0600(dir: &Path, path: &Path, body: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .context("destination path has no file name")?
        .to_string_lossy();
    let tmp_path = dir.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
    {
        #[cfg(unix)]
        let mut tmp = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)
                .with_context(|| format!("creating temp file {}", tmp_path.display()))?
        };
        #[cfg(not(unix))]
        let mut tmp = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating temp file {}", tmp_path.display()))?;
        tmp.write_all(body)
            .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
        tmp.sync_all()
            .with_context(|| format!("syncing temp file {}", tmp_path.display()))?;
    }
    if let Err(e) = durable_rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e)
            .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()));
    }
    Ok(())
}

#[cfg(not(windows))]
fn durable_rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    std::fs::rename(source, destination)
}

// Test-only hook so a test can observe (and act on) a retry attempt
// deterministically instead of racing a fixed sleep against the retry
// budget. Thread-local: `cargo test` runs each test on its own thread, so
// setting this on the test's thread does not leak into other tests.
#[cfg(all(windows, test))]
thread_local! {
    static RETRY_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut(u32)>>> =
        std::cell::RefCell::new(None);
}

#[cfg(windows)]
fn durable_rename(source: &Path, destination: &Path) -> std::io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    // Windows readers and file scanners can briefly deny replacement. Retry
    // only lock/access errors, preserving the old file and the same synced
    // temporary file throughout. Permanent failures remain errors.
    for attempt in 0..=10 {
        // SAFETY: both paths are owned, NUL-terminated UTF-16 buffers, alive
        // throughout this synchronous call. Write-through completes publication
        // before an old OS credential may be retired.
        let moved = unsafe {
            MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if moved != 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        // ERROR_ACCESS_DENIED, ERROR_SHARING_VIOLATION, ERROR_LOCK_VIOLATION.
        if attempt == 10 || !matches!(error.raw_os_error(), Some(5 | 32 | 33)) {
            return Err(error);
        }
        #[cfg(test)]
        RETRY_OBSERVER.with(|observer| {
            if let Some(on_retry) = observer.borrow_mut().as_mut() {
                on_retry(attempt);
            }
        });
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    unreachable!("the last attempt returns")
}

/// Create a scratch file in `dir` for capturing a child process's output:
/// readable and writable, and gone once its last handle closes.
///
/// On Windows the file is named `.capture-<uuid>.tmp` and stays listed in
/// `dir` until its last handle closes. It is opened shared with no other
/// opener, marked temporary and deleted on close: the flags
/// `tempfile::tempfile()` uses on Windows, with a different name. On unix it
/// is created owner-only and its name removed at once, so it is anonymous
/// while in use, as `tempfile::tempfile()` leaves it.
///
/// `tempfile` names come from `fastrand`, which is not seeded from the OS,
/// and it retries only when the name already exists. On Windows a name
/// whose file is still pending deletion fails with ERROR_ACCESS_DENIED
/// instead, which it does not retry; parallel daemons on CI hit that a few
/// times in a hundred runs. A v4 UUID name, from the OS random source, does
/// not collide. A denial can still come from elsewhere (a filter driver, or
/// a pending-delete name this code did not make), so up to
/// [`CAPTURE_ATTEMPTS`] tries are made, each under a fresh name.
pub fn create_capture_file(dir: &Path) -> std::io::Result<std::fs::File> {
    create_capture_file_with(|| capture_path(dir))
}

/// How many names [`create_capture_file`] tries before it gives up.
pub const CAPTURE_ATTEMPTS: u32 = 3;

fn create_capture_file_with(
    mut next_path: impl FnMut() -> std::path::PathBuf,
) -> std::io::Result<std::fs::File> {
    let mut attempt = 1;
    loop {
        match open_capture_file(&next_path()) {
            Err(error) if attempt < CAPTURE_ATTEMPTS && capture_retryable(&error) => {
                attempt += 1;
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            result => return result,
        }
    }
}

/// A name already taken, or (on Windows) ERROR_ACCESS_DENIED, which is
/// what a create at a pending-delete name returns. Neither says anything
/// about the next, freshly named attempt.
fn capture_retryable(error: &std::io::Error) -> bool {
    error.kind() == std::io::ErrorKind::AlreadyExists
        || (cfg!(windows) && error.raw_os_error() == Some(5))
}

fn capture_path(dir: &Path) -> std::path::PathBuf {
    dir.join(format!(".capture-{}.tmp", Uuid::new_v4().simple()))
}

#[cfg(windows)]
fn open_capture_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_ATTRIBUTE_TEMPORARY, FILE_FLAG_DELETE_ON_CLOSE,
    };
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .share_mode(0)
        .custom_flags(FILE_ATTRIBUTE_TEMPORARY | FILE_FLAG_DELETE_ON_CLOSE)
        .open(path)
}

#[cfg(unix)]
fn open_capture_file(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    // A file whose name could not be removed would stay visible to anything
    // that lists the directory, so that is an error rather than a capture.
    std::fs::remove_file(path)?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
fn open_capture_file(_path: &Path) -> std::io::Result<std::fs::File> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

/// Open a selected file for reading without following a final symlink or
/// reparse point and without blocking on a FIFO or device. Errors carry
/// fixed labels only.
pub fn open_import_file(path: &std::path::Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        // O_NONBLOCK | O_NOFOLLOW, spelled per-ABI rather than through `libc`,
        // which is not a direct dependency of this permissive crate and is not
        // worth becoming one for two constants. The consequence is deliberate
        // and is the safe direction: a target whose ABI is not enumerated here
        // gets no open at all, not an open with the wrong flags. Adding a
        // target means adding its pair, and the symlink and FIFO tests on
        // `trace_commons_contributor::evidence_import::read_import`, which
        // opens through this, are what prove a pair is right. Today that
        // leaves Linux on 32-bit arm, riscv64 and s390x, and every non-Linux
        // non-macOS unix, without local import; they refuse by name rather
        // than silently losing the confinement the flags provide.
        let flags = if cfg!(target_os = "macos") {
            0x4 | 0x100
        } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
            0x800 | 0x20000
        } else if cfg!(all(target_os = "linux", target_arch = "aarch64")) {
            0x800 | 0x8000
        } else {
            return Err(anyhow!("import-platform-unsupported"));
        };
        options.custom_flags(flags);
    }
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        // Refuse device/pipe and network namespaces before CreateFile can
        // connect or block. Ordinary drive paths (including verbatim disk
        // paths) and relative local paths retain regular-file validation.
        if let Some(Component::Prefix(prefix)) = path.components().next()
            && !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        {
            return Err(anyhow!("import-file-namespace-unsupported"));
        }
        use std::os::windows::fs::OpenOptionsExt;
        options.custom_flags(0x00200000); // FILE_FLAG_OPEN_REPARSE_POINT
    }
    #[cfg(not(any(unix, windows)))]
    return Err(anyhow!("import-platform-unsupported"));
    let file = options
        .open(path)
        .map_err(|_| anyhow!("import-file-unreadable"))?;
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if file
            .metadata()
            .map_err(|_| anyhow!("import-file-unreadable"))?
            .file_attributes()
            & 0x400
            != 0
        {
            return Err(anyhow!("import-file-not-regular"));
        }
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(windows)]
    #[test]
    fn atomic_config_write_waits_for_a_windows_reader_to_release_the_file() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        // Formerly a `ConfigStore` over a temporary directory; its
        // `daemon_path` is a plain join and opening it writes nothing, so the
        // directory still holds exactly the destination file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked-config.json");
        std::fs::write(&path, b"original").unwrap();
        let reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();

        // Deterministic handoff: release the reader the first time the
        // retry loop actually observes the lock (attempt 0), instead of
        // racing a fixed sleep against the retry budget. A prior version of
        // this test slept 100ms on the releasing thread against a ~550ms
        // retry budget, which is not a safe margin on a loaded CI runner.
        let released = std::rc::Rc::new(std::cell::Cell::new(false));
        let released_flag = released.clone();
        let reader_cell = std::cell::RefCell::new(Some(reader));
        RETRY_OBSERVER.with(|observer| {
            *observer.borrow_mut() = Some(Box::new(move |_attempt| {
                if reader_cell.borrow_mut().take().is_some() {
                    released_flag.set(true);
                }
            }));
        });

        write_atomic_0600(dir.path(), &path, b"replacement").unwrap();
        RETRY_OBSERVER.with(|observer| *observer.borrow_mut() = None);

        assert!(
            released.get(),
            "the retry loop never observed the lock to release it"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"replacement");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    fn atomic_config_write_returns_non_transient_windows_errors_without_retrying() {
        // Renaming a source that does not exist raises ERROR_FILE_NOT_FOUND
        // (2), which is not one of the retried codes (5, 32, 33). The retry
        // loop must fail on the first attempt rather than spending the full
        // retry budget on a permanent error.
        let dir = tempfile::tempdir().unwrap();
        let missing_source = dir.path().join("does-not-exist.tmp");
        let destination = dir.path().join("destination.json");

        let retries = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let retries_seen = retries.clone();
        RETRY_OBSERVER.with(|observer| {
            *observer.borrow_mut() = Some(Box::new(move |_attempt| {
                retries_seen.set(retries_seen.get() + 1);
            }));
        });

        let result = durable_rename(&missing_source, &destination);
        RETRY_OBSERVER.with(|observer| *observer.borrow_mut() = None);

        assert!(result.is_err(), "renaming a missing source must fail");
        assert_eq!(
            retries.get(),
            0,
            "a non-transient Windows error must not enter the retry loop"
        );
    }

    /// A fresh name per attempt: a taken name is retried under the next
    /// one, and the attempts stop at the bound.
    #[test]
    fn capture_file_retries_a_taken_name_under_a_fresh_one() {
        let dir = tempfile::tempdir().unwrap();
        let taken = dir.path().join("taken");
        std::fs::write(&taken, b"").unwrap();
        let fresh = dir.path().join("fresh");
        let mut names = vec![fresh, taken.clone()];
        create_capture_file_with(|| names.pop().unwrap()).unwrap();
        assert!(names.is_empty(), "the second name was never tried");

        let mut tries = 0;
        let refused = create_capture_file_with(|| {
            tries += 1;
            taken.clone()
        })
        .unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::AlreadyExists);
        assert_eq!(tries, CAPTURE_ATTEMPTS);
    }

    /// A failure that a new name cannot fix is returned at once.
    #[test]
    fn capture_file_does_not_retry_a_missing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("missing");
        let mut tries = 0;
        let refused = create_capture_file_with(|| {
            tries += 1;
            capture_path(&missing)
        })
        .unwrap_err();
        assert_eq!(refused.kind(), std::io::ErrorKind::NotFound);
        assert_eq!(tries, 1);
    }

    /// A capture file can be written through a clone (as a child's
    /// stdout is), read back after a rewind, and leaves nothing in `dir`
    /// once closed. On Windows it is listed under a v4 UUID name while held
    /// and nothing else can open it; on unix it has no name at all.
    #[test]
    fn capture_file_round_trips_and_leaves_nothing_behind() {
        use std::io::{Read, Seek};

        let dir = tempfile::tempdir().unwrap();
        let mut file = create_capture_file(dir.path()).unwrap();
        let mut clone = file.try_clone().unwrap();
        clone.write_all(b"captured").unwrap();
        file.rewind().unwrap();
        let mut read = String::new();
        file.read_to_string(&mut read).unwrap();
        assert_eq!(read, "captured");

        let listed: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        if cfg!(windows) {
            assert_eq!(listed.len(), 1, "{listed:?}");
            let name = listed[0].file_name().unwrap().to_str().unwrap();
            // `.capture-` + 32 hex digits of a v4 UUID + `.tmp`. A regression
            // to tempfile's fastrand names (`.tmpXXXXXX`) fails here.
            let id = name
                .strip_prefix(".capture-")
                .and_then(|rest| rest.strip_suffix(".tmp"))
                .unwrap_or_else(|| panic!("unexpected capture name {name}"));
            assert_eq!(Uuid::parse_str(id).unwrap().get_version_num(), 4, "{name}");
            assert!(std::fs::File::open(&listed[0]).is_err());
        } else {
            assert!(listed.is_empty(), "{listed:?}");
        }

        drop(file);
        drop(clone);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    /// The mechanism behind the managed-session flake: on Windows a create
    /// at a name whose file is still pending deletion fails with
    /// ERROR_ACCESS_DENIED (5), not ERROR_FILE_EXISTS, so a creator that
    /// retries only "already exists" gives up. A capture file retries it
    /// under a fresh name. Where the volume deletes with POSIX semantics
    /// the name is released at once and there is nothing to retry; the
    /// test then checks only that the create succeeds.
    #[cfg(windows)]
    #[test]
    fn capture_file_retries_a_delete_pending_name() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{
            FILE_FLAG_DELETE_ON_CLOSE, FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
        };

        let dir = tempfile::tempdir().unwrap();
        let pending = capture_path(dir.path());
        let share = FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE;
        let deleting = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .share_mode(share)
            .custom_flags(FILE_FLAG_DELETE_ON_CLOSE)
            .open(&pending)
            .unwrap();
        let holder = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(share)
            .open(&pending)
            .unwrap();
        // Closing the delete-on-close handle while another stays open
        // leaves the name pending deletion, on volumes that delete that way.
        drop(deleting);

        let fresh = capture_path(dir.path());
        let mut names = vec![fresh, pending.clone()];
        let file = create_capture_file_with(|| names.pop().unwrap()).unwrap();
        if names.is_empty() {
            // The pending name was refused and the fresh one used. Confirm
            // the refusal was error 5, the case tempfile does not retry.
            let refused = open_capture_file(&pending).unwrap_err();
            assert_eq!(refused.raw_os_error(), Some(5), "{refused:?}");
        }
        drop(file);
        drop(holder);
        assert!(!pending.exists());
    }

    #[cfg(windows)]
    #[test]
    fn atomic_config_write_preserves_old_bytes_when_a_windows_reader_stays_open() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::{FILE_SHARE_READ, FILE_SHARE_WRITE};

        // Formerly a `ConfigStore` over a temporary directory; its
        // `daemon_path` is a plain join and opening it writes nothing, so the
        // directory still holds exactly the destination file.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("locked-config.json");
        std::fs::write(&path, b"original").unwrap();
        let _reader = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
            .open(&path)
            .unwrap();
        assert!(write_atomic_0600(dir.path(), &path, b"replacement").is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
