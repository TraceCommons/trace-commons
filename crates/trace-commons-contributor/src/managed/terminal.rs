//! External terminal handoff. Only short-lived launch authority crosses argv.
use super::ManagedError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// What the daemon keeps, from its own launch, to find a session's terminal
/// window again. A launch that cannot name its window keeps none, and that
/// session cannot be shown. One variant per terminal the daemon can script;
/// another terminal adds its own here, with its own `focus` arm.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum TerminalHandle {
    /// A Terminal window, by the id AppleScript returned for the window
    /// holding the tab the launch opened.
    AppleTerminal { window_id: u32 },
}

/// The window id the launch script printed, or none. Only a bare positive
/// decimal is taken, so nothing the script prints can become part of a
/// later script.
pub fn apple_terminal_window(output: &[u8]) -> Option<TerminalHandle> {
    let text = std::str::from_utf8(output).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    if text.is_empty() || text.len() > 10 || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    match text.parse::<u32>() {
        Ok(window_id) if window_id > 0 => Some(TerminalHandle::AppleTerminal { window_id }),
        _ => None,
    }
}

/// Bring a session's window to the front. Refused when the window is gone
/// or this build cannot script the terminal that holds it.
pub fn focus(handle: &TerminalHandle) -> Result<(), ManagedError> {
    match handle {
        #[cfg(target_os = "macos")]
        TerminalHandle::AppleTerminal { window_id } => {
            // `window_id` is a u32, so the script holds only digits from it.
            let script = format!(
                "tell application \"Terminal\"\n\
                 set w to window id {window_id}\n\
                 if miniaturized of w then set miniaturized of w to false\n\
                 set index of w to 1\n\
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
        // The window is found by the new tab's tty, a string compare, and
        // printed as its id. Any failure there prints nothing: the launch
        // stands and the session simply cannot be shown.
        let mut script = std::process::Command::new("/usr/bin/osascript");
        script.args([
            "-e",
            &format!(
                "tell application \"Terminal\"\n\
                 set t to do script \"{literal}\"\n\
                 activate\n\
                 set found to \"\"\n\
                 try\n\
                 set wanted to tty of t\n\
                 repeat with w in windows\n\
                 if wanted is in (tty of tabs of w) then set found to (id of w) as text\n\
                 end repeat\n\
                 end try\n\
                 return found\n\
                 end tell"
            ),
        ]);
        let output = super::profiles::bounded_output(script)?;
        if output.status.success() {
            Ok(apple_terminal_window(&output.stdout))
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
