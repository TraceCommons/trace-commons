//! Verifies that the daemon's Windows named pipe actually excludes other
//! users. This is a test tool, not a contributor-facing command.
//!
//! # Why this binary exists
//!
//! `daemon::win_pipe` builds a DACL granting the creating user's SID alone,
//! and on Windows that DACL is the *only* access control on the daemon
//! socket -- there is no 0700 directory doing the work as there is on unix.
//! A cross-compile establishes the FFI signatures and the control flow and
//! says nothing about whether the descriptor actually denies anybody. That
//! is a runtime property of the Windows security model, and the only way to
//! establish it is to have a second, unprivileged account attempt the
//! connection and be refused.
//!
//! So this binary does exactly that, and CI runs it on `windows-latest`
//! (see the `windows-pipe-acl` job). Without it the control ships on the
//! strength of an argument rather than an observation, which for the one
//! thing standing between a local process and a contributor's transcripts
//! is not good enough.
//!
//! # Why impersonation rather than launching a second process
//!
//! `Start-Process -Credential` needs the target account to hold "log on as
//! a batch job", which is extra runner setup that fails in its own
//! confusing ways and would be easy to mistake for a passing test.
//! `LogonUser` + `ImpersonateLoggedOnUser` changes the identity of this
//! thread only, in one process, with no such requirement -- so a failure
//! here is a failure of the DACL and not of the test harness.
//!
//! # Modes
//!
//! - `serve <state-dir>`: create the pipe and hold it open. Prints `READY`.
//! - `connect <state-dir>`: attempt to open the pipe as the current user.
//! - `connect-as <user> <password> <state-dir>`: attempt to open the pipe
//!   while impersonating another local account.
//!
//! The last two print exactly one of `CONNECTED` or `DENIED <code>` so the
//! CI script can assert on the outcome rather than on a message.

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "win-pipe-acl-probe verifies a Windows-only security control and does \
         nothing on this platform"
    );
    std::process::exit(2);
}

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;

    let args: Vec<String> = std::env::args().collect();
    let usage = "usage: win-pipe-acl-probe serve <dir> | connect <dir> | \
                 connect-as <user> <password> <dir>";

    match args.get(1).map(String::as_str) {
        Some("serve") => {
            let dir = args.get(2).expect(usage);
            windows_impl::serve(PathBuf::from(dir));
        }
        Some("connect") => {
            let dir = args.get(2).expect(usage);
            windows_impl::report(windows_impl::attempt_open(&PathBuf::from(dir)));
        }
        Some("connect-as") => {
            let user = args.get(2).expect(usage);
            let password = args.get(3).expect(usage);
            let dir = args.get(4).expect(usage);
            windows_impl::connect_as(user, password, &PathBuf::from(dir));
        }
        _ => {
            eprintln!("{usage}");
            std::process::exit(2);
        }
    }
}

#[cfg(windows)]
mod windows_impl {
    use std::path::{Path, PathBuf};

    use trace_commons_contributor::config::ConfigStore;
    use trace_commons_contributor::daemon::win_pipe;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::Security::{
        ImpersonateLoggedOnUser, LOGON32_LOGON_INTERACTIVE, LOGON32_PROVIDER_DEFAULT, LogonUserW,
        RevertToSelf,
    };

    /// The exit code for "the pipe refused us", which is the *expected*
    /// outcome for the second user and therefore the one CI asserts on.
    const ERROR_ACCESS_DENIED: i32 = 5;

    pub fn serve(dir: PathBuf) {
        let store = ConfigStore::open(dir).expect("opening the state directory");
        let rt = tokio::runtime::Runtime::new().expect("building a runtime");
        let _pipe = rt
            .block_on(win_pipe::bind(&store))
            .expect("creating the restricted pipe");
        // The name goes to stderr, not stdout: stdout carries only the
        // CONNECTED/DENIED verdict so the CI script can assert on it
        // exactly.
        eprintln!("pipe: {}", win_pipe::pipe_name(&store));
        println!("READY");
        use std::io::Write;
        std::io::stdout().flush().ok();
        // Hold the pipe open. The CI script kills this process when the
        // connect attempts are done.
        std::thread::sleep(std::time::Duration::from_secs(300));
    }

    /// Attempt to open the daemon pipe. `Ok(())` means it opened, which for
    /// a second user is a FAILING result.
    pub fn attempt_open(dir: &Path) -> Result<(), i32> {
        let store = ConfigStore::open(dir.to_path_buf()).expect("opening the state directory");
        let name = win_pipe::pipe_name(&store);
        match std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&name)
        {
            Ok(_) => Ok(()),
            Err(e) => Err(e.raw_os_error().unwrap_or(-1)),
        }
    }

    pub fn report(outcome: Result<(), i32>) {
        match outcome {
            Ok(()) => println!("CONNECTED"),
            Err(code) => println!("DENIED {code}"),
        }
    }

    /// Attempt the open while impersonating `user`.
    ///
    /// Impersonation is reverted before reporting, so a bug in the reporting
    /// path cannot leave the thread running as somebody else.
    pub fn connect_as(user: &str, password: &str, dir: &Path) {
        let wide = |s: &str| -> Vec<u16> { s.encode_utf16().chain(std::iter::once(0)).collect() };
        let user_w = wide(user);
        // "." is the local machine, which is what a CI-created account is on.
        let domain_w = wide(".");
        let password_w = wide(password);

        let mut token: HANDLE = std::ptr::null_mut();
        // SAFETY: all three strings are NUL-terminated and outlive the call;
        // `token` is a valid out-pointer.
        let ok = unsafe {
            LogonUserW(
                user_w.as_ptr(),
                domain_w.as_ptr(),
                password_w.as_ptr(),
                LOGON32_LOGON_INTERACTIVE,
                LOGON32_PROVIDER_DEFAULT,
                &mut token,
            )
        };
        if ok == 0 {
            // A harness failure, not a verdict. Say so loudly rather than
            // printing DENIED, which CI would read as the control working.
            eprintln!(
                "HARNESS-FAILURE: cannot log on as {user} (os error {})",
                std::io::Error::last_os_error()
            );
            std::process::exit(3);
        }

        // SAFETY: `token` is a valid logon token from the call above.
        let impersonating = unsafe { ImpersonateLoggedOnUser(token) };
        if impersonating == 0 {
            eprintln!(
                "HARNESS-FAILURE: cannot impersonate {user} (os error {})",
                std::io::Error::last_os_error()
            );
            // SAFETY: closing the token we opened.
            unsafe { CloseHandle(token) };
            std::process::exit(3);
        }

        let outcome = attempt_open(dir);

        // SAFETY: paired with the successful `ImpersonateLoggedOnUser`.
        unsafe { RevertToSelf() };
        // SAFETY: closing the token we opened.
        unsafe { CloseHandle(token) };

        report(outcome);
        // Make the expected refusal legible in the exit code too, so a CI
        // step that forgets to inspect stdout still fails loudly rather
        // than passing by omission.
        if let Err(code) = outcome
            && code == ERROR_ACCESS_DENIED
        {
            std::process::exit(0);
        }
        if outcome.is_ok() {
            std::process::exit(1);
        }
    }
}
