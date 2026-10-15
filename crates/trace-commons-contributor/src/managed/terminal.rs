//! External terminal handoff. Only short-lived launch authority crosses argv.
use super::ManagedError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the daemon keeps, from its own launch, to find a session's terminal
/// tab again. A launch that cannot name its tab keeps none, and that
/// session cannot be shown. One variant per terminal the daemon can script;
/// another terminal adds its own here, with its own `focus` arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalHandle {
    /// A Terminal tab, by the tty device the launch's tab reported. The tool
    /// holds that tty for as long as it runs, so no other tab can have it
    /// while the session can be shown, and a tab dragged into another window
    /// is still found.
    AppleTerminal {
        #[serde(deserialize_with = "tty_path")]
        tty: String,
    },
}

/// A bare tty device path, `/dev/tty` and up to 16 letters or digits. Nothing
/// else is ever put in a script, whether it came from the launch script's
/// output or from a file on disk.
fn valid_tty(text: &str) -> bool {
    text.strip_prefix("/dev/tty").is_some_and(|rest| {
        (1..=16).contains(&rest.len()) && rest.bytes().all(|b| b.is_ascii_alphanumeric())
    })
}

fn tty_path<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    let text = String::deserialize(d)?;
    if valid_tty(&text) {
        Ok(text)
    } else {
        Err(serde::de::Error::custom("not a tty path"))
    }
}

/// The tty the launch script printed, or none.
pub fn apple_terminal_tab(output: &[u8]) -> Option<TerminalHandle> {
    let text = std::str::from_utf8(output).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    valid_tty(text).then(|| TerminalHandle::AppleTerminal {
        tty: text.to_string(),
    })
}

/// Bring a session's tab to the front, in whichever window holds it now.
/// Refused when the tab is gone or this build cannot script the terminal
/// that holds it.
pub fn focus(handle: &TerminalHandle) -> Result<(), ManagedError> {
    match handle {
        #[cfg(target_os = "macos")]
        TerminalHandle::AppleTerminal { tty } => {
            if !valid_tty(tty) {
                return Err(ManagedError::InvalidRequest);
            }
            // `tty` passed `valid_tty`, so the script holds only letters,
            // digits and slashes from it.
            let script = format!(
                "tell application \"Terminal\"\n\
                 set done to false\n\
                 repeat with w in windows\n\
                 repeat with t in tabs of w\n\
                 if tty of t is \"{tty}\" then\n\
                 set selected of t to true\n\
                 if miniaturized of w then set miniaturized of w to false\n\
                 set index of w to 1\n\
                 set done to true\n\
                 exit repeat\n\
                 end if\n\
                 end repeat\n\
                 if done then exit repeat\n\
                 end repeat\n\
                 if not done then error \"no tab\"\n\
                 activate\n\
                 end tell"
            );
            let mut command = std::process::Command::new("/usr/bin/osascript");
            command.args(["-e", &script]);
            let output = super::profiles::bounded_output(command)?;
            if output.status.success() {
                Ok(())
            } else {
                Err(ManagedError::NotFound)
            }
        }
        #[cfg(not(target_os = "macos"))]
        TerminalHandle::AppleTerminal { .. } => Err(ManagedError::UnsupportedVersion),
    }
}

pub fn shell_command(
    helper: &Path,
    config: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<String, ManagedError> {
    if !helper.is_absolute()
        || !config.is_absolute()
        || ticket.is_empty()
        || !ticket.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err(ManagedError::InvalidRequest);
    }
    let quote = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let helper = helper.to_str().ok_or(ManagedError::InvalidRequest)?;
    let config = config.to_str().ok_or(ManagedError::InvalidRequest)?;
    Ok(format!(
        "{} --config-dir {} redeem-launch --session-id {} --ticket {}",
        quote(helper),
        quote(config),
        session,
        quote(ticket)
    ))
}

pub fn helper_path() -> Result<PathBuf, ManagedError> {
    let exe = std::env::current_exe().map_err(|_| ManagedError::UnsupportedVersion)?;
    let parent = exe.parent().ok_or(ManagedError::UnsupportedVersion)?;
    let name = if cfg!(windows) {
        "near-ai.exe"
    } else {
        "near-ai"
    };
    let candidates = [parent.join(name), parent.join("../Resources").join(name)];
    candidates
        .into_iter()
        .find(|p| p.is_file())
        .and_then(|p| p.canonicalize().ok())
        .ok_or(ManagedError::UnsupportedVersion)
}

pub fn destination() -> Option<&'static str> {
    if helper_path().is_err() {
        return None;
    }
    #[cfg(target_os = "macos")]
    {
        Some("Terminal")
    }
    #[cfg(windows)]
    {
        external_terminal("wt.exe").map(|_| "Windows Terminal")
    }
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/.flatpak-info").exists() {
            None
        } else {
            external_terminal("x-terminal-emulator").map(|_| "System terminal")
        }
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        None
    }
}

/// Open the session's launcher in the terminal. The handle names the window
/// it opened in, when the terminal can say; a launch that cannot is still a
/// launch, with no handle.
pub fn launch(
    config: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<Option<TerminalHandle>, ManagedError> {
    let command = shell_command(&helper_path()?, config, session, ticket)?;
    #[cfg(target_os = "macos")]
    {
        // AppleScript receives the shell command as a string literal. Escape
        // independently of shell quoting; user paths never become script code.
        let literal = command
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', "\\n")
            .replace('\r', "\\r");
        // The new tab's tty is printed, and read back only as a bare device
        // path. Any failure there prints nothing: the launch stands and the
        // session simply cannot be shown.
        let mut script = std::process::Command::new("/usr/bin/osascript");
        script.args([
            "-e",
            &format!(
                "tell application \"Terminal\"\n\
                 set t to do script \"{literal}\"\n\
                 activate\n\
                 set found to \"\"\n\
                 try\n\
                 set found to tty of t\n\
                 end try\n\
                 return found\n\
                 end tell"
            ),
        ]);
        let output = super::profiles::bounded_output(script)?;
        if output.status.success() {
            Ok(apple_terminal_tab(&output.stdout))
        } else {
            Err(ManagedError::LaunchUnknown)
        }
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        let _ = command;
        if destination().is_none() {
            return Err(ManagedError::UnsupportedVersion);
        }
        #[cfg(windows)]
        let mut child = std::process::Command::new(
            external_terminal("wt.exe").ok_or(ManagedError::UnsupportedVersion)?,
        );
        #[cfg(windows)]
        child.args(["new-tab", "--title", "NEAR AI"]);
        #[cfg(target_os = "linux")]
        let mut child = std::process::Command::new(
            external_terminal("x-terminal-emulator").ok_or(ManagedError::UnsupportedVersion)?,
        );
        #[cfg(target_os = "linux")]
        child.arg("-e");
        let mut process = child
            .arg(helper_path()?)
            .arg("--config-dir")
            .arg(config)
            .arg("redeem-launch")
            .arg("--session-id")
            .arg(session.to_string())
            .arg("--ticket")
            .arg(ticket)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|_| ManagedError::LaunchUnknown)?;
        std::thread::spawn(move || {
            let _ = process.wait();
        });
        Ok(None)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = command;
        Err(ManagedError::UnsupportedVersion)
    }
}

#[cfg(any(windows, target_os = "linux"))]
fn external_terminal(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p.is_absolute())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
}
