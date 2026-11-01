//! The status line a managed Claude Code session shows: IronWire's own
//! line for the session's proxy, served by this executable.
//!
//! A managed session's proxy is embedded (`ironwire_proxy::embed`), one per
//! session, each in its own home under `<config dir>/managed-routes/<session
//! id>`. Claude Code runs the status-line command on every render, so the
//! launcher passes it `--settings` naming `<this executable> statusline
//! --home <that home>` ([`settings`]), and the `statusline` subcommand
//! ([`run`]) prints IronWire's line for the proxy that owns that home.
//!
//! It never fails loudly. Every failure prints nothing and exits 0, and the
//! whole run is bounded by IronWire's [`BUDGET`]: a status line that stalls
//! or shouts is worse than none.

use std::io::Write;
use std::path::Path;

pub use ironwire_proxy::statusline::BUDGET;

/// The line for the proxy that owns `home`, given the document Claude Code
/// sent on stdin, if any. `None` when there is nothing to say or no way to
/// find out within [`BUDGET`].
pub async fn line(home: &Path, document: Option<&str>) -> Option<String> {
    let asked = document.and_then(ironwire_proxy::statusline::client_model);
    tokio::time::timeout(
        BUDGET,
        ironwire_proxy::statusline::line_for_home(home, asked.as_deref()),
    )
    .await
    .ok()
    .flatten()
}

/// The `statusline` subcommand: print the line, or nothing. Never an error.
pub async fn run(home: Option<&Path>) {
    let Some(home) = home else {
        return;
    };
    let document = read_document();
    if let Some(line) = line(home, document.as_deref()).await {
        // A closed stdout is not worth a panic's message on stderr.
        let _ = writeln!(std::io::stdout().lock(), "{line}");
    }
}

/// The document on stdin, read for at most [`BUDGET`]. Claude Code writes it
/// and closes stdin; a terminal, or a writer that never closes, yields none.
/// The reader is a detached thread, so one blocked on stdin never holds the
/// process open.
fn read_document() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = String::new();
        let read = std::io::stdin().read_to_string(&mut buffer).ok();
        let _ = sender.send(read.map(|_| buffer));
    });
    receiver.recv_timeout(BUDGET).ok().flatten()
}

/// The shell Claude Code runs a status-line command in.
///
/// Claude Code 2.1.302 spawns it with `shell: true` on Unix, which is
/// `/bin/sh -c`. On Windows it uses Git Bash when it finds one and
/// PowerShell otherwise; a launcher cannot tell which, so a Windows command
/// is written for Git Bash with forward slashes, and is left unquoted, which
/// both shells read the same way, whenever the paths allow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shell {
    Posix,
    WindowsGitBash,
}

impl Shell {
    pub(crate) fn current() -> Self {
        if cfg!(windows) {
            Self::WindowsGitBash
        } else {
            Self::Posix
        }
    }
}

/// One word for `shell`: as written when every character is one neither
/// shell treats specially, otherwise single-quoted, with each `'` closed,
/// escaped and reopened.
pub(crate) fn quote(word: &str, shell: Shell) -> String {
    let word = match shell {
        Shell::Posix => word.to_owned(),
        Shell::WindowsGitBash => word.replace('\\', "/"),
    };
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-'));
    if plain {
        word
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// The status-line command: `<exe> statusline --home <home>`, quoted for
/// `shell`. `None` for a path that is not UTF-8.
fn command_for(exe: &Path, home: &Path, shell: Shell) -> Option<String> {
    Some(format!(
        "{} statusline --home {}",
        quote(exe.to_str()?, shell),
        quote(home.to_str()?, shell)
    ))
}

/// The `--settings` JSON a managed Claude Code session is launched with:
/// a `statusLine` that runs this executable's `statusline` for `home`.
/// `None` when this executable's path cannot be found or written.
///
/// Settings from the command line take precedence over the profile's
/// `settings.json` and a project's, and nothing is written to disk.
pub fn settings(home: &Path) -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    settings_for(&exe, home, Shell::current())
}

fn settings_for(exe: &Path, home: &Path, shell: Shell) -> Option<String> {
    Some(
        serde_json::json!({
            "statusLine": {"type": "command", "command": command_for(exe, home, shell)?}
        })
        .to_string(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Paths with spaces, both quotes, `$`, a backtick and a backslash come
    /// back from `/bin/sh` exactly as given.
    #[cfg(unix)]
    #[test]
    fn a_quoted_path_reaches_the_program_unchanged() {
        let dir = tempfile::tempdir().unwrap();
        let odd = dir.path().join("Trace Commons's \"app\" $HOME `x` a\\b");
        std::fs::create_dir_all(&odd).unwrap();
        let exe = odd.join("print args");
        std::fs::write(
            &exe,
            "#!/bin/sh\nfor a in \"$@\"; do printf '%s\\n' \"$a\"; done\n",
        )
        .unwrap();
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&exe, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let home = odd.join("managed-routes").join("it's home");
        let command = command_for(&exe, &home, Shell::Posix).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &command])
            .output()
            .unwrap();
        assert!(output.status.success(), "{command}: {output:?}");
        assert_eq!(
            String::from_utf8(output.stdout).unwrap(),
            format!("statusline\n--home\n{}\n", home.display())
        );
    }

    #[test]
    fn a_plain_path_is_left_unquoted() {
        assert_eq!(
            command_for(
                Path::new("/opt/tc/bin/trace-commons-contributor"),
                Path::new("/home/a/.config/tc/managed-routes/1-2"),
                Shell::Posix
            )
            .as_deref(),
            Some(
                "/opt/tc/bin/trace-commons-contributor statusline --home /home/a/.config/tc/managed-routes/1-2"
            )
        );
    }

    /// Windows paths reach Git Bash with forward slashes, unquoted when they
    /// can be (PowerShell reads those the same way), quoted otherwise.
    #[test]
    fn a_windows_command_is_written_for_git_bash() {
        assert_eq!(
            command_for(
                Path::new(r"C:\Users\a\AppData\Local\Programs\tc\near-ai.exe"),
                Path::new(r"C:\Users\a\AppData\Roaming\tc\managed-routes\1-2"),
                Shell::WindowsGitBash
            )
            .as_deref(),
            Some(
                "C:/Users/a/AppData/Local/Programs/tc/near-ai.exe statusline --home C:/Users/a/AppData/Roaming/tc/managed-routes/1-2"
            )
        );
        assert_eq!(
            command_for(
                Path::new(r"C:\Users\Jo O'Neil\tc.exe"),
                Path::new(r"C:\Users\Jo O'Neil\routes\1"),
                Shell::WindowsGitBash
            )
            .as_deref(),
            Some(
                r"'C:/Users/Jo O'\''Neil/tc.exe' statusline --home 'C:/Users/Jo O'\''Neil/routes/1'"
            )
        );
    }

    #[test]
    fn the_settings_name_only_a_command_status_line() {
        let settings = settings_for(
            Path::new("/Applications/Trace Commons.app/Contents/MacOS/near-ai"),
            Path::new("/Users/a/tc/managed-routes/1"),
            Shell::Posix,
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&settings).unwrap();
        assert_eq!(
            value,
            serde_json::json!({"statusLine": {
                "type": "command",
                "command": "'/Applications/Trace Commons.app/Contents/MacOS/near-ai' statusline --home /Users/a/tc/managed-routes/1"
            }})
        );
    }

    /// A home with no proxy, an empty one, or a pointer to a closed port:
    /// no line, and promptly.
    #[tokio::test]
    async fn no_proxy_means_no_line_within_the_budget() {
        let dir = tempfile::tempdir().unwrap();
        let begun = std::time::Instant::now();
        assert_eq!(line(&dir.path().join("absent"), None).await, None);
        assert_eq!(line(dir.path(), Some("not json")).await, None);
        // A token and a pointer to a port nothing listens on.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = closed.local_addr().unwrap().port();
        drop(closed);
        std::fs::write(dir.path().join("control.token"), "t\n").unwrap();
        std::fs::write(
            dir.path().join("endpoint.json"),
            serde_json::json!({
                "control_url": format!("http://127.0.0.1:{port}"),
                "token_path": dir.path().join("control.token"),
            })
            .to_string(),
        )
        .unwrap();
        assert_eq!(line(dir.path(), None).await, None);
        assert!(begun.elapsed() < BUDGET * 3, "{:?}", begun.elapsed());
    }
}
