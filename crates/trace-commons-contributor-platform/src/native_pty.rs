//! A native CLI under a pseudo-terminal, for reading a screen a person would
//! see (the Claude Code `/usage` panel). macOS only: the constants below are
//! Darwin's, and the prompt problem this serves exists only there.
//!
//! The child is a session leader with the PTY as its controlling terminal,
//! which also makes it the leader of a fresh process group. A
//! [`ProcessGroupGuard`] owns that group, so a timeout, an early return or a
//! panic kills the child and every descendant. No `libc` crate: the handful
//! of calls are declared here, as `native_process_group` does.

use std::ffi::{c_int, c_short, c_uint, c_ulong, c_void};
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use crate::native_process_group::ProcessGroupGuard;

#[repr(C)]
struct WinSize {
    rows: u16,
    cols: u16,
    x_pixels: u16,
    y_pixels: u16,
}

#[repr(C)]
struct PollFd {
    fd: c_int,
    events: c_short,
    revents: c_short,
}

const POLLIN: c_short = 0x1;
const POLLHUP: c_short = 0x10;
const F_SETFD: c_int = 2;
const FD_CLOEXEC: c_int = 1;
// _IO('t', 97) on Darwin.
const TIOCSCTTY: c_ulong = 0x2000_7461;
const EINTR: i32 = 4;
const SIGTERM: c_int = 15;
/// How long a terminal session gets to exit on SIGTERM, so a config file it
/// is writing is not cut off, before the group is killed.
const GRACE: Duration = Duration::from_millis(1500);

unsafe extern "C" {
    fn openpty(
        master: *mut c_int,
        slave: *mut c_int,
        name: *mut c_void,
        termios: *const c_void,
        size: *const WinSize,
    ) -> c_int;
    fn setsid() -> c_int;
    fn ioctl(fd: c_int, request: c_ulong, ...) -> c_int;
    fn fcntl(fd: c_int, command: c_int, ...) -> c_int;
    fn poll(fds: *mut PollFd, count: c_uint, timeout_ms: c_int) -> c_int;
    fn killpg(group: c_int, signal: c_int) -> c_int;
}

/// A child process attached to a PTY. Dropping it kills the child's whole
/// process group and reaps the child.
pub struct PtyChild {
    /// `None` once terminated: closing it ends any wait on output drain.
    master: Option<File>,
    child: Child,
    group: ProcessGroupGuard,
}

impl std::fmt::Debug for PtyChild {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PtyChild").finish_non_exhaustive()
    }
}

fn cloexec(fd: c_int) -> Result<(), &'static str> {
    // SAFETY: fd is an open descriptor this function's caller owns.
    if unsafe { fcntl(fd, F_SETFD, FD_CLOEXEC) } == -1 {
        return Err("native-pty-unavailable");
    }
    Ok(())
}

impl PtyChild {
    /// Spawn `command` on a new PTY of `cols` by `rows`. The command's stdio
    /// is replaced by the terminal; its environment and working directory
    /// are the caller's to set.
    pub fn spawn(command: &mut Command, cols: u16, rows: u16) -> Result<Self, &'static str> {
        let size = WinSize {
            rows,
            cols,
            x_pixels: 0,
            y_pixels: 0,
        };
        let (mut master, mut slave) = (-1, -1);
        // SAFETY: valid out-pointers; null name and termios are permitted.
        let opened = unsafe {
            openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        };
        if opened != 0 || master < 0 || slave < 0 {
            return Err("native-pty-unavailable");
        }
        // SAFETY: openpty returned two fresh descriptors owned from here on.
        let master = unsafe { File::from_raw_fd(master) };
        // SAFETY: as above.
        let slave = unsafe { File::from_raw_fd(slave) };
        // Neither original descriptor may leak into the child or anything it
        // starts; the child gets duplicates on 0, 1 and 2 only.
        cloexec(master.as_raw_fd())?;
        cloexec(slave.as_raw_fd())?;
        let stdin = slave.try_clone().map_err(|_| "native-pty-unavailable")?;
        let stdout = slave.try_clone().map_err(|_| "native-pty-unavailable")?;
        command
            .stdin(Stdio::from(stdin))
            .stdout(Stdio::from(stdout))
            .stderr(Stdio::from(slave));
        // SAFETY: runs in the forked child before exec and calls only
        // async-signal-safe functions. stdio is already on 0, 1 and 2.
        unsafe {
            command.pre_exec(|| {
                if setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                if ioctl(0, TIOCSCTTY, 0) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut child = command.spawn().map_err(|_| "native-pty-spawn-failed")?;
        // setsid made the child the leader of a new group whose ID is its PID.
        let group = match ProcessGroupGuard::new(Some(child.id())) {
            Ok(group) => group,
            Err(reason) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(reason);
            }
        };
        Ok(Self {
            master: Some(master),
            child,
            group,
        })
    }

    /// Read what the terminal has, waiting at most `wait`. `Ok(0)` means no
    /// output within `wait`; `Err` means the terminal closed or failed.
    pub fn read_some(&mut self, buf: &mut [u8], wait: Duration) -> Result<usize, &'static str> {
        let master = self.master.as_mut().ok_or("native-pty-closed")?;
        let mut fds = PollFd {
            fd: master.as_raw_fd(),
            events: POLLIN,
            revents: 0,
        };
        let millis = c_int::try_from(wait.as_millis()).unwrap_or(c_int::MAX);
        // SAFETY: one valid pollfd for the duration of the call.
        let ready = unsafe { poll(&mut fds, 1, millis) };
        if ready < 0 {
            if std::io::Error::last_os_error().raw_os_error() == Some(EINTR) {
                return Ok(0);
            }
            return Err("native-pty-read-failed");
        }
        if ready == 0 {
            return Ok(0);
        }
        if fds.revents & POLLIN == 0 && fds.revents & POLLHUP != 0 {
            return Err("native-pty-closed");
        }
        match master.read(buf) {
            Ok(0) => Err("native-pty-closed"),
            Ok(read) => Ok(read),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => Ok(0),
            // Darwin reports EIO on the master once the child side is gone.
            Err(_) => Err("native-pty-closed"),
        }
    }

    /// Type `bytes` into the terminal.
    pub fn write_all(&mut self, bytes: &[u8]) -> Result<(), &'static str> {
        let master = self.master.as_mut().ok_or("native-pty-closed")?;
        master
            .write_all(bytes)
            .and_then(|()| master.flush())
            .map_err(|_| "native-pty-write-failed")
    }

    /// Whether the direct child is still running.
    pub fn running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Ask the group to exit, give it a short grace period, then kill the
    /// whole group and reap the direct child.
    pub fn terminate(&mut self) {
        if self.running() {
            if let Ok(group) = c_int::try_from(self.child.id()) {
                // SAFETY: the child is not yet reaped, so its PID still names
                // the isolated group validated at spawn, never the host's.
                unsafe {
                    killpg(group, SIGTERM);
                }
                // Keep reading: an exiting process can wait for its terminal
                // output to drain.
                let deadline = std::time::Instant::now() + GRACE;
                let mut sink = [0u8; 4096];
                while self.running() && std::time::Instant::now() < deadline {
                    if self
                        .read_some(&mut sink, Duration::from_millis(20))
                        .is_err()
                    {
                        std::thread::sleep(Duration::from_millis(20));
                    }
                }
            }
        }
        let _ = self.group.terminate();
        self.master = None;
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Instant;

    fn read_until(child: &mut PtyChild, needle: &str, within: Duration) -> String {
        let deadline = Instant::now() + within;
        let mut seen = Vec::new();
        let mut buf = [0u8; 4096];
        while Instant::now() < deadline {
            match child.read_some(&mut buf, Duration::from_millis(50)) {
                Ok(read) => seen.extend_from_slice(&buf[..read]),
                Err(_) => break,
            }
            if String::from_utf8_lossy(&seen).contains(needle) {
                break;
            }
        }
        String::from_utf8_lossy(&seen).into_owned()
    }

    #[test]
    fn the_child_has_a_terminal_of_the_requested_size_and_reads_typed_input() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "test -t 0 && stty size && read line && echo \"got:$line\" && sleep 30",
        ]);
        let mut child = PtyChild::spawn(&mut command, 160, 50).unwrap();
        let size = read_until(&mut child, "160", Duration::from_secs(5));
        assert!(size.contains("50 160"), "{size:?}");
        child.write_all(b"hello\r").unwrap();
        let echoed = read_until(&mut child, "got:hello", Duration::from_secs(5));
        assert!(echoed.contains("got:hello"), "{echoed:?}");
        assert!(child.running());
        child.terminate();
        assert!(!child.running());
    }

    #[test]
    fn terminate_lets_the_child_finish_on_sigterm_first() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("cleaned-up");
        let ready = dir.path().join("ready");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!(
                "trap \"touch '{}'; exit 0\" TERM; touch '{}'; while :; do sleep 0.05; done",
                marker.display(),
                ready.display()
            ),
        ]);
        let mut child = PtyChild::spawn(&mut command, 80, 24).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !ready.exists() {
            assert!(Instant::now() < deadline, "child never started");
            std::thread::sleep(Duration::from_millis(20));
        }
        child.terminate();
        assert!(marker.exists());
    }

    #[test]
    fn dropping_the_child_kills_its_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("descendant.pid");
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            &format!("sleep 60 & echo $! > '{}'; wait", pid_file.display()),
        ]);
        let child = PtyChild::spawn(&mut command, 80, 24).unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let pid = loop {
            if let Ok(text) = std::fs::read_to_string(&pid_file) {
                if let Ok(pid) = text.trim().parse::<i32>() {
                    break pid;
                }
            }
            assert!(Instant::now() < deadline, "descendant never started");
            std::thread::sleep(Duration::from_millis(20));
        };
        drop(child);
        unsafe extern "C" {
            fn kill(pid: c_int, signal: c_int) -> c_int;
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        // The descendant is reparented to launchd once killed; signal zero
        // fails with ESRCH when it is gone.
        // SAFETY: probes a PID this test's own fixture emitted.
        while unsafe { kill(pid, 0) } == 0 {
            assert!(Instant::now() < deadline, "descendant survived");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
