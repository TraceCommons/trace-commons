//! Red Team's Windows host requirements and the one-click "Enable Red Team"
//! elevation (owner decision, 2026-11-21).
//!
//! Red Team on Windows runs in a Hyper-V virtual machine: the edition must
//! have Hyper-V, the feature must be on, and the user must be in the
//! Hyper-V Administrators group. When the feature is off or the user is not
//! in the group, the app offers "Enable Red Team", which runs the fixed script
//! shipped beside the executable, elevated through a UAC prompt. The only
//! value that reaches the script is the daemon's own user SID, validated
//! here; nothing from an IPC request or the environment does.
//!
//! Everything in this file but [`elevated_enable`] and [`user_in_hyperv_administrators`]
//! is plain logic, built and tested on every platform.

use super::ManagedError;
use serde::Serialize;
use std::path::{Path, PathBuf};

/// The script, shipped in the signed package beside the executable.
pub const ENABLE_SCRIPT: &str = "enable-red-team.ps1";

/// The script's exit code when it finished and the user must sign out and in.
pub const EXIT_SIGN_OUT: i32 = 0;
/// `ERROR_SUCCESS_REBOOT_REQUIRED`: Hyper-V was turned on; restart Windows.
pub const EXIT_RESTART: i32 = 3010;
/// The script refused: Windows Home has no Hyper-V.
pub const EXIT_HOME: i32 = 10;
/// `ERROR_CANCELLED`, which `ShellExecuteExW` reports when the UAC prompt is declined.
pub const ERROR_CANCELLED: i32 = 1223;

/// Why this Windows host cannot run Red Team yet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HostProblem {
    /// Windows Home has no Hyper-V and cannot get it.
    HomeEdition,
    /// The Hyper-V feature is off.
    HyperVMissing,
    /// The user is not in the Hyper-V Administrators group (or has not
    /// signed in again since being added).
    NotInHyperVAdministrators,
}

impl HostProblem {
    /// Whether the app offers "Enable Red Team": everything but Windows Home.
    pub fn enable_offered(self) -> bool {
        !matches!(self, Self::HomeEdition)
    }
}

/// The problem `rts setup`'s check names, from its error text.
pub fn classify_setup_error(message: &str) -> Option<HostProblem> {
    if message.contains("Windows Home is not supported") {
        Some(HostProblem::HomeEdition)
    } else if message.contains("Hyper-V is not available") {
        Some(HostProblem::HyperVMissing)
    } else {
        None
    }
}

/// Whether `sid` is a user or domain account SID: `S-1-5-21-` and at least
/// three more sub-authorities, digits only. Nothing else may reach the
/// elevated command line.
pub fn valid_user_sid(sid: &str) -> bool {
    let Some(rest) = sid.strip_prefix("S-1-5-21-") else {
        return false;
    };
    let parts: Vec<&str> = rest.split('-').collect();
    parts.len() >= 4
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.len() <= 10 && p.bytes().all(|b| b.is_ascii_digit()))
}

/// The script beside `exe`: `<exe dir>\enable-red-team.ps1`, a regular file
/// (never a link) whose resolved path is still under the executable's
/// directory.
pub fn resolve_script(exe: &Path) -> Result<PathBuf, ManagedError> {
    let dir = exe.parent().ok_or(ManagedError::ToolUnavailable)?;
    let dir = std::fs::canonicalize(dir).map_err(|_| ManagedError::ToolUnavailable)?;
    let script = dir.join(ENABLE_SCRIPT);
    let meta = std::fs::symlink_metadata(&script).map_err(|_| ManagedError::ToolUnavailable)?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err(ManagedError::ToolUnavailable);
    }
    let resolved = std::fs::canonicalize(&script).map_err(|_| ManagedError::ToolUnavailable)?;
    if !resolved.starts_with(&dir) {
        return Err(ManagedError::ToolUnavailable);
    }
    Ok(resolved)
}

/// The parameter string `ShellExecuteExW` passes to `powershell.exe`:
/// a fixed set of flags, the quoted script path, and the validated SID.
pub fn elevation_parameters(script: &Path, sid: &str) -> Result<String, ManagedError> {
    if !valid_user_sid(sid) {
        return Err(ManagedError::InvalidRequest);
    }
    let script = script.to_str().ok_or(ManagedError::InvalidRequest)?;
    if script.is_empty() || script.contains(['"', '\r', '\n', '\0']) {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(format!(
        "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"{script}\" -UserSid {sid}"
    ))
}

/// How an "Enable Red Team" run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnableOutcome {
    /// Done: sign out of Windows and back in for the group to apply.
    SignOutNeeded,
    /// Hyper-V was turned on: restart Windows.
    RestartNeeded,
    /// Windows Home: nothing to enable.
    HomeUnsupported,
    /// The user declined the UAC prompt.
    Declined,
    Failed,
}

pub fn outcome_from_exit(code: i32) -> EnableOutcome {
    match code {
        EXIT_SIGN_OUT => EnableOutcome::SignOutNeeded,
        EXIT_RESTART => EnableOutcome::RestartNeeded,
        EXIT_HOME => EnableOutcome::HomeUnsupported,
        _ => EnableOutcome::Failed,
    }
}

/// The "Enable Red Team" flow with its effects passed in: which problem the
/// host has, the daemon's own user SID, the shipped script, and the elevated
/// run. Elevates only for a problem the script fixes, and only with a validated
/// SID and a script found beside the executable.
pub fn enable_with(
    problem: Option<HostProblem>,
    sid: impl FnOnce() -> Option<String>,
    script: impl FnOnce() -> Result<PathBuf, ManagedError>,
    elevate: impl FnOnce(&Path, &str) -> Result<EnableOutcome, ManagedError>,
) -> Result<EnableOutcome, ManagedError> {
    match problem {
        None => return Err(ManagedError::InvalidRequest),
        Some(HostProblem::HomeEdition) => return Ok(EnableOutcome::HomeUnsupported),
        Some(_) => {}
    }
    let sid = sid()
        .filter(|s| valid_user_sid(s))
        .ok_or(ManagedError::InvalidRequest)?;
    let script = script()?;
    elevate(&script, &sid)
}

/// [`enable_with`] on this host: the daemon's own SID from rts and the
/// script beside the running executable. Windows only.
#[cfg(windows)]
pub fn enable(problem: Option<HostProblem>) -> Result<EnableOutcome, ManagedError> {
    enable_with(
        problem,
        || rts::sys::fs::current_user_sid().ok(),
        || resolve_script(&std::env::current_exe().map_err(|_| ManagedError::ToolUnavailable)?),
        elevated_enable,
    )
}

/// Whether the current process token has the Hyper-V Administrators group
/// (`S-1-5-32-578`) enabled. A user added to the group is not in it until they
/// sign out and back in, which is what this reports.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn user_in_hyperv_administrators() -> bool {
    use windows_sys::Win32::Foundation::{FALSE, LocalFree};
    use windows_sys::Win32::Security::Authorization::ConvertStringSidToSidW;
    use windows_sys::Win32::Security::CheckTokenMembership;
    let sid: Vec<u16> = "S-1-5-32-578".encode_utf16().chain([0]).collect();
    let mut psid = std::ptr::null_mut();
    // SAFETY: a NUL-terminated SID string and a valid out-pointer.
    if unsafe { ConvertStringSidToSidW(sid.as_ptr(), &mut psid) } == FALSE {
        return false;
    }
    let mut member = FALSE;
    // SAFETY: a null token means the calling thread's effective token; `psid`
    // is the SID just allocated, freed below.
    let ok = unsafe { CheckTokenMembership(std::ptr::null_mut(), psid, &mut member) };
    // SAFETY: `psid` came from ConvertStringSidToSidW.
    unsafe { LocalFree(psid) };
    ok != FALSE && member != FALSE
}

/// Run the fixed script elevated through a UAC prompt and wait for it.
/// `script` is [`resolve_script`]'s result and `sid` the daemon's own user SID.
#[cfg(windows)]
#[allow(unsafe_code)]
pub fn elevated_enable(script: &Path, sid: &str) -> Result<EnableOutcome, ManagedError> {
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    use windows_sys::Win32::UI::Shell::{
        SEE_MASK_NOASYNC, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW, ShellExecuteExW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_HIDE;
    /// Thirty minutes: enabling a Windows feature can be slow.
    const WAIT_MS: u32 = 30 * 60 * 1000;

    let parameters = elevation_parameters(script, sid)?;
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    let powershell =
        Path::new(&system_root).join(r"System32\WindowsPowerShell\v1.0\powershell.exe");
    if !powershell.is_file() {
        return Err(ManagedError::ToolUnavailable);
    }
    let wide = |s: &std::ffi::OsStr| -> Vec<u16> {
        use std::os::windows::ffi::OsStrExt;
        s.encode_wide().chain([0]).collect()
    };
    let verb = wide(std::ffi::OsStr::new("runas"));
    let file = wide(powershell.as_os_str());
    let params = wide(std::ffi::OsStr::new(&parameters));
    // SAFETY: an all-zero SHELLEXECUTEINFOW is valid; the fields set below
    // point at NUL-terminated buffers that outlive the call.
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    // SAFETY: `info` is fully initialised for the fields ShellExecuteExW reads.
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        // SAFETY: reads the calling thread's last error.
        let code = unsafe { GetLastError() } as i32;
        return Ok(if code == ERROR_CANCELLED {
            EnableOutcome::Declined
        } else {
            EnableOutcome::Failed
        });
    }
    let process = info.hProcess;
    if process.is_null() {
        return Ok(EnableOutcome::Failed);
    }
    // SAFETY: `process` is the handle ShellExecuteExW returned (NOCLOSEPROCESS).
    let waited = unsafe { WaitForSingleObject(process, WAIT_MS) };
    let mut code = 0u32;
    let got = waited == WAIT_OBJECT_0 && unsafe { GetExitCodeProcess(process, &mut code) } != 0;
    // SAFETY: closing the handle we own, once.
    unsafe { CloseHandle(process) };
    Ok(if got {
        outcome_from_exit(code as i32)
    } else {
        EnableOutcome::Failed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";

    #[test]
    fn setup_errors_are_classified_by_the_text_rts_gives() {
        assert_eq!(
            classify_setup_error(
                "Windows Home is not supported: rts runs its sandbox in a Hyper-V virtual machine"
            ),
            Some(HostProblem::HomeEdition)
        );
        assert_eq!(
            classify_setup_error(
                "Hyper-V is not available on this computer (computecore.dll is missing): enable the Hyper-V feature"
            ),
            Some(HostProblem::HyperVMissing)
        );
        assert_eq!(
            classify_setup_error("the rts guest file x is missing"),
            None
        );
        assert_eq!(classify_setup_error(""), None);
    }

    #[test]
    fn only_windows_home_gets_no_enable_button() {
        assert!(!HostProblem::HomeEdition.enable_offered());
        assert!(HostProblem::HyperVMissing.enable_offered());
        assert!(HostProblem::NotInHyperVAdministrators.enable_offered());
    }

    #[test]
    fn a_user_sid_is_accepted_and_nothing_else() {
        assert!(valid_user_sid(SID));
        assert!(valid_user_sid("S-1-5-21-1-2-3-500"));
        for bad in [
            "",
            "S-1-5-32-544",
            "S-1-5-32-578",
            "S-1-5-18",
            "S-1-5-21-1-2-3",
            "S-1-5-21-1-2-3-",
            "S-1-5-21-1-2--3-500",
            "S-1-5-21-1-2-3-500 ",
            " S-1-5-21-1-2-3-500",
            "S-1-5-21-1-2-3-500; calc",
            "S-1-5-21-1-2-3-500\" -Command x",
            "S-1-5-21-1-2-3-50a",
            "S-1-5-21-1-2-3-12345678901",
            "s-1-5-21-1-2-3-500",
            "S-1-5-21-1-2-3-500\n",
            "S-1-5-21-１-2-3-500",
        ] {
            assert!(!valid_user_sid(bad), "{bad:?}");
        }
    }

    #[test]
    fn the_elevated_command_line_is_fixed_but_for_the_sid() {
        let script = Path::new(r"C:\Program Files\Near\enable-red-team.ps1");
        assert_eq!(
            elevation_parameters(script, SID).unwrap(),
            format!(
                "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File \"C:\\Program Files\\Near\\enable-red-team.ps1\" -UserSid {SID}"
            )
        );
    }

    #[test]
    fn the_elevated_command_line_refuses_a_bad_sid_or_script_path() {
        let script = Path::new("enable-red-team.ps1");
        assert!(elevation_parameters(script, "S-1-5-32-544").is_err());
        assert!(elevation_parameters(script, &format!("{SID} -Command x")).is_err());
        assert!(elevation_parameters(Path::new("a\"b.ps1"), SID).is_err());
        assert!(elevation_parameters(Path::new("a\nb.ps1"), SID).is_err());
        assert!(elevation_parameters(Path::new(""), SID).is_err());
    }

    #[test]
    fn the_script_is_found_only_as_a_regular_file_beside_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("near-ai.exe");
        std::fs::write(&exe, b"x").unwrap();
        assert!(resolve_script(&exe).is_err(), "missing script");
        std::fs::write(dir.path().join(ENABLE_SCRIPT), b"#").unwrap();
        let found = resolve_script(&exe).unwrap();
        assert_eq!(found.file_name().unwrap(), ENABLE_SCRIPT);
        assert!(found.starts_with(std::fs::canonicalize(dir.path()).unwrap()));
    }

    #[cfg(unix)]
    #[test]
    fn a_linked_script_is_refused_even_into_the_same_directory() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("near-ai.exe");
        std::fs::write(&exe, b"x").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        std::fs::write(elsewhere.path().join("evil.ps1"), b"#").unwrap();
        std::os::unix::fs::symlink(
            elsewhere.path().join("evil.ps1"),
            dir.path().join(ENABLE_SCRIPT),
        )
        .unwrap();
        assert!(resolve_script(&exe).is_err());
    }

    #[test]
    fn a_directory_named_like_the_script_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("near-ai.exe");
        std::fs::write(&exe, b"x").unwrap();
        std::fs::create_dir(dir.path().join(ENABLE_SCRIPT)).unwrap();
        assert!(resolve_script(&exe).is_err());
    }

    #[test]
    fn script_exit_codes_map_to_what_the_user_is_told() {
        assert_eq!(outcome_from_exit(0), EnableOutcome::SignOutNeeded);
        assert_eq!(outcome_from_exit(3010), EnableOutcome::RestartNeeded);
        assert_eq!(outcome_from_exit(10), EnableOutcome::HomeUnsupported);
        assert_eq!(outcome_from_exit(1), EnableOutcome::Failed);
        assert_eq!(outcome_from_exit(-1), EnableOutcome::Failed);
    }

    #[test]
    fn enabling_never_elevates_on_windows_home() {
        let mut elevated = false;
        let out = enable_with(
            Some(HostProblem::HomeEdition),
            || Some(SID.into()),
            || Ok(PathBuf::from("s.ps1")),
            |_, _| {
                elevated = true;
                Ok(EnableOutcome::SignOutNeeded)
            },
        );
        assert_eq!(out, Ok(EnableOutcome::HomeUnsupported));
        assert!(!elevated);
    }

    #[test]
    fn enabling_with_nothing_to_enable_is_refused_without_elevating() {
        let out = enable_with(
            None,
            || Some(SID.into()),
            || Ok(PathBuf::from("s.ps1")),
            |_, _| panic!("must not elevate"),
        );
        assert!(matches!(out, Err(ManagedError::InvalidRequest)));
    }

    #[test]
    fn enabling_refuses_a_sid_that_is_not_a_user_sid_before_elevating() {
        for sid in [
            None,
            Some("S-1-5-32-544".to_owned()),
            Some(format!("{SID}; x")),
        ] {
            let out = enable_with(
                Some(HostProblem::HyperVMissing),
                || sid.clone(),
                || Ok(PathBuf::from("s.ps1")),
                |_, _| panic!("must not elevate"),
            );
            assert!(out.is_err(), "{sid:?}");
        }
    }

    #[test]
    fn enabling_refuses_a_missing_script_before_elevating() {
        let out = enable_with(
            Some(HostProblem::NotInHyperVAdministrators),
            || Some(SID.into()),
            || Err(ManagedError::ToolUnavailable),
            |_, _| panic!("must not elevate"),
        );
        assert!(matches!(out, Err(ManagedError::ToolUnavailable)));
    }

    #[test]
    fn enabling_passes_only_the_script_and_the_sid_to_the_elevated_run() {
        let mut seen = None;
        let out = enable_with(
            Some(HostProblem::HyperVMissing),
            || Some(SID.into()),
            || Ok(PathBuf::from("enable-red-team.ps1")),
            |script, sid| {
                seen = Some((script.to_path_buf(), sid.to_owned()));
                Ok(EnableOutcome::RestartNeeded)
            },
        );
        assert_eq!(out, Ok(EnableOutcome::RestartNeeded));
        assert_eq!(
            seen,
            Some((PathBuf::from("enable-red-team.ps1"), SID.to_owned()))
        );
    }

    #[test]
    fn a_declined_prompt_is_its_own_outcome() {
        let out = enable_with(
            Some(HostProblem::HyperVMissing),
            || Some(SID.into()),
            || Ok(PathBuf::from("s.ps1")),
            |_, _| Ok(EnableOutcome::Declined),
        );
        assert_eq!(out, Ok(EnableOutcome::Declined));
    }

    /// The shipped script and this module agree: its exit codes, the group SID
    /// and the SID shape it re-checks, and it runs nothing from its input.
    #[test]
    fn the_shipped_script_matches_the_daemons_contract() {
        let script = include_str!("../../../../windows/packaging/enable-red-team.ps1");
        assert!(script.contains(&format!("exit {EXIT_HOME}")));
        assert!(script.contains(&format!("exit {EXIT_RESTART}")));
        assert!(script.contains("S-1-5-32-578"));
        assert!(script.contains("'^S-1-5-21(-[0-9]{1,10}){4,}$'"));
        assert!(script.contains("Microsoft-Hyper-V-All"));
        for banned in [
            "Invoke-Expression",
            "iex ",
            "Start-Process",
            "Invoke-WebRequest",
            "Invoke-RestMethod",
            "DownloadString",
            "-EncodedCommand",
        ] {
            assert!(!script.contains(banned), "{banned}");
        }
        // One parameter only.
        assert_eq!(script.matches("[Parameter(").count(), 1);
        assert!(script.contains("$UserSid"));
    }
}
