//! Cancellation cleanup for a native CLI spawned in its own Unix process group.
//! A guard is only constructed from the PID of a newly spawned child whose
//! command used `process_group(0)`, never from an IPC-supplied process ID.

use std::ffi::c_int;

const SIGKILL: c_int = 9;
// Darwin and Linux both use ESRCH=3 for an absent process/process group.
const ESRCH: i32 = 3;

unsafe extern "C" {
    fn getpgrp() -> c_int;
    fn getpgid(pid: c_int) -> c_int;
    fn killpg(group: c_int, signal: c_int) -> c_int;
}

/// Owns cleanup for a fresh, isolated child process group. The direct child
/// still belongs to the caller, which must wait for it after explicit stop.
#[must_use = "keep the guard alive for the entire native child operation"]
#[derive(Debug)]
pub struct ProcessGroupGuard {
    group: Option<c_int>,
}

impl ProcessGroupGuard {
    /// `pid` must come directly from a child spawned with `process_group(0)`.
    /// Refuses the current process group and any live child that is not its
    /// group's leader, so cancellation cannot signal the host's group.
    pub fn new(pid: Option<u32>) -> Result<Self, &'static str> {
        let group = pid
            .and_then(|pid| c_int::try_from(pid).ok())
            .filter(|pid| *pid > 1)
            .ok_or("native-process-group-invalid")?;
        // SAFETY: these calls read process metadata; group is a positive PID.
        let current = unsafe { getpgrp() };
        if current == group {
            return Err("native-process-group-not-isolated");
        }
        // SAFETY: getpgid accepts the validated positive PID without pointers.
        let observed = unsafe { getpgid(group) };
        if observed >= 0 && observed != group {
            return Err("native-process-group-not-isolated");
        }
        if observed < 0 {
            if std::io::Error::last_os_error().raw_os_error() != Some(ESRCH) {
                return Err("native-process-group-unavailable");
            }
            // The wrapper can exit before the guard is created while a
            // descendant still holds the group. Check the owned group, not
            // another PID or the caller's group. Signal zero sends no signal.
            // SAFETY: group is the fresh isolated child's positive group ID.
            if unsafe { killpg(group, 0) } != 0 {
                if std::io::Error::last_os_error().raw_os_error() == Some(ESRCH) {
                    return Ok(Self { group: None });
                }
                return Err("native-process-group-unavailable");
            }
        }
        Ok(Self { group: Some(group) })
    }

    /// Synchronously stop the owned group, including descendants holding pipes.
    /// Idempotent; callers then reap their direct child through its own handle.
    pub fn terminate(&mut self) -> Result<(), &'static str> {
        let Some(group) = self.group else {
            return Ok(());
        };
        // SAFETY: construction accepts only the fresh isolated group and
        // refuses zero/negative IDs and the host group. No pointers are used.
        let result = unsafe { killpg(group, SIGKILL) };
        if result == 0 || std::io::Error::last_os_error().raw_os_error() == Some(ESRCH) {
            self.group = None;
            Ok(())
        } else {
            Err("native-process-group-stop-failed")
        }
    }
}

impl Drop for ProcessGroupGuard {
    fn drop(&mut self) {
        let _ = self.terminate();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    unsafe extern "C" {
        fn kill(pid: c_int, signal: c_int) -> c_int;
    }

    fn running(pid: u32) -> bool {
        // SAFETY: test fixture supplied this positive PID; signal zero only
        // checks existence. A zombie cannot retain running descendant work.
        if unsafe { kill(pid as c_int, 0) } != 0 {
            return std::io::Error::last_os_error().raw_os_error() != Some(ESRCH);
        }
        let output = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output();
        !output.is_ok_and(|output| {
            String::from_utf8_lossy(&output.stdout)
                .trim_start()
                .starts_with('Z')
        })
    }

    struct Fixture {
        child: Child,
        descendant: Option<u32>,
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            // SAFETY: the test created this dedicated group and owns its child.
            unsafe {
                killpg(self.child.id() as c_int, SIGKILL);
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
            if let Some(pid) = self.descendant {
                if running(pid) {
                    // SAFETY: this PID was emitted only by the owned fixture.
                    unsafe {
                        kill(pid as c_int, SIGKILL);
                    }
                }
            }
        }
    }

    #[test]
    fn invalid_ids_and_the_hosts_process_group_are_refused() {
        assert!(ProcessGroupGuard::new(None).is_err());
        assert!(ProcessGroupGuard::new(Some(0)).is_err());
        assert!(ProcessGroupGuard::new(Some(1)).is_err());
        assert!(ProcessGroupGuard::new(Some(u32::MAX)).is_err());
        // SAFETY: reads the test runner's own process-group ID.
        let host = unsafe { getpgrp() };
        assert!(ProcessGroupGuard::new(Some(host as u32)).is_err());
    }

    #[test]
    fn explicit_stop_is_idempotent_and_stops_wrapper_descendants() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("descendant.pid");
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 60 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; wait",
                "fixture",
            ])
            .arg(&path)
            .process_group(0)
            .spawn()
            .unwrap();
        let mut fixture = Fixture {
            child,
            descendant: None,
        };
        let mut guard = ProcessGroupGuard::new(Some(fixture.child.id())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let descendant = loop {
            if let Ok(bytes) = std::fs::read_to_string(&path) {
                if let Ok(pid) = bytes.parse::<u32>() {
                    break pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture descendant was not started"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        fixture.descendant = Some(descendant);
        assert!(running(descendant));
        // Explicit stop is idempotent and also stops the wrapper descendants.
        guard.terminate().unwrap();
        guard.terminate().unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while running(descendant) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !running(descendant),
            "the descendant survived native cancellation"
        );
        assert!(fixture.child.wait().is_ok());
    }

    #[test]
    fn cancellation_drop_stops_descendants_without_an_async_cleanup_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("descendant.pid");
        let child = Command::new("/bin/sh")
            .args([
                "-c",
                "sleep 60 & descendant=$!; printf '%s' \"$descendant\" > \"$1\"; wait",
                "fixture",
            ])
            .arg(&path)
            .process_group(0)
            .spawn()
            .unwrap();
        let mut fixture = Fixture {
            child,
            descendant: None,
        };
        let guard = ProcessGroupGuard::new(Some(fixture.child.id())).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        let descendant = loop {
            if let Ok(bytes) = std::fs::read_to_string(&path) {
                if let Ok(pid) = bytes.parse::<u32>() {
                    break pid;
                }
            }
            assert!(
                Instant::now() < deadline,
                "fixture descendant was not started"
            );
            std::thread::sleep(Duration::from_millis(10));
        };
        fixture.descendant = Some(descendant);
        drop(guard);
        let deadline = Instant::now() + Duration::from_secs(2);
        while running(descendant) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !running(descendant),
            "dropping a collector left its descendant alive"
        );
        assert!(fixture.child.wait().is_ok());
    }
}
