//! External terminal handoff. Only short-lived launch authority crosses argv.
use super::ManagedError;
use std::path::{Path, PathBuf};
use uuid::Uuid;

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

pub fn launch(config: &Path, session: Uuid, ticket: &str) -> Result<(), ManagedError> {
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
        let mut script = std::process::Command::new("/usr/bin/osascript");
        script
            .args([
                "-e",
                &format!("tell application \"Terminal\" to do script \"{literal}\""),
                "-e",
                "tell application \"Terminal\" to activate",
            ])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let output = super::profiles::bounded_output(script)?;
        if output.status.success() {
            Ok(())
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
        Ok(())
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
