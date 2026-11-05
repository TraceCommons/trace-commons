//! Ends an owned worker when the process that owns it ends, however it ends.
//!
//! `kill_on_drop` covers every exit that runs Rust drops. A process killed by
//! SIGKILL, or by SIGTERM's default action, drops nothing, and the worker
//! would keep running with no owner. The worker is a separate binary whose
//! code this crate does not control, so it cannot be asked to watch its
//! parent itself, and macOS has no `PR_SET_PDEATHSIG`.
//!
//! So each launch gets a small watchdog: forked from the worker's own process
//! just before it execs, it holds the read end of a pipe whose only write end
//! stays in the owner. The owner never writes. When the owner ends, the kernel
//! closes the write end, the watchdog reads end-of-file and kills the worker
//! (its parent). When the worker ends first, the watchdog is reparented and
//! exits within one poll interval.

use std::ffi::{c_int, c_short};
use std::io::{PipeReader, PipeWriter};
use std::os::fd::AsRawFd;

const SIGKILL: c_int = 9;
const POLLIN: c_short = 0x1;
/// How often the watchdog checks that the worker is still its parent.
const POLL_MS: c_int = 1000;
/// Bound on the descriptors the watchdog closes, for a soft limit raised to
/// something huge or unlimited.
const MAX_CLOSE: c_int = 1 << 20;

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

#[cfg(target_os = "linux")]
type NfdsT = std::ffi::c_ulong;
#[cfg(not(target_os = "linux"))]
type NfdsT = std::ffi::c_uint;

unsafe extern "C" {
    fn fork() -> c_int;
    fn getpid() -> c_int;
    fn getppid() -> c_int;
    fn getdtablesize() -> c_int;
    fn close(fd: c_int) -> c_int;
    fn poll(fds: *mut PollFd, nfds: NfdsT, timeout: c_int) -> c_int;
    fn kill(pid: c_int, signal: c_int) -> c_int;
    fn _exit(status: c_int) -> !;
}

/// The owner's end of the link. Held for as long as the owner owns the
/// worker; the kernel closes it when the owner ends.
#[derive(Debug)]
pub(super) struct OwnerLink {
    _write: PipeWriter,
    // Kept open until the link goes, so the descriptor number the pre-exec
    // closure captured stays valid for the spawn.
    _read: PipeReader,
}

/// Arrange for `command`'s process to start a watchdog before it execs.
/// The returned link must outlive the spawn, and is what keeps the worker
/// alive: dropping it ends the worker too.
pub(super) fn arm(command: &mut tokio::process::Command) -> std::io::Result<OwnerLink> {
    // Both ends are close-on-exec, so the worker never holds either.
    let (read, write) = std::io::pipe()?;
    let read_fd = read.as_raw_fd();
    // SAFETY: the closure runs in the forked child before exec and calls only
    // async-signal-safe functions (fork, getpid, getppid, getdtablesize,
    // close, poll, kill, _exit); it allocates nothing and takes no lock.
    unsafe {
        command.pre_exec(move || {
            let worker = getpid();
            match fork() {
                -1 => Err(std::io::Error::last_os_error()),
                0 => watch(worker, read_fd),
                _ => Ok(()),
            }
        });
    }
    Ok(OwnerLink {
        _write: write,
        _read: read,
    })
}

/// The watchdog. Never returns.
fn watch(worker: c_int, read_fd: c_int) -> ! {
    // SAFETY: async-signal-safe calls only (see `arm`). The watchdog never
    // execs, so close-on-exec does not apply: it closes every inherited
    // descriptor itself, so it holds no lock, socket or write end of its own
    // pipe.
    unsafe {
        let limit = getdtablesize().clamp(0, MAX_CLOSE);
        for fd in 0..limit {
            if fd != read_fd {
                close(fd);
            }
        }
        loop {
            if getppid() != worker {
                // The worker has ended and this process was reparented.
                _exit(0);
            }
            let mut owner = PollFd {
                fd: read_fd,
                events: POLLIN,
                revents: 0,
            };
            // Nothing is ever written, so readiness means end-of-file: the
            // owner is gone. A timeout or EINTR loops.
            if poll(&mut owner, 1, POLL_MS) > 0 {
                if getppid() == worker {
                    kill(worker, SIGKILL);
                }
                _exit(0);
            }
        }
    }
}
