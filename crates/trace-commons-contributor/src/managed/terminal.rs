//! External terminal handoff. Only short-lived launch authority crosses argv.
//!
//! On macOS the contributor chooses which terminal app opens a session, from
//! a closed set ([`TerminalApp`]). Each app has a fixed AppleScript template;
//! the only thing interpolated into it is the shell command, as an escaped
//! AppleScript string literal. No app name, path or command comes from a
//! setting. Windows and Linux keep their one destination each.
use super::ManagedError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The terminal apps a session can open in on macOS. Closed: anything else
/// is refused by [`TerminalApp::parse`], never read as an app or a command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalApp {
    Terminal,
    Ghostty,
    Iterm2,
}

impl TerminalApp {
    /// Every choice, in the order a picker shows them.
    pub const ALL: [TerminalApp; 3] = [
        TerminalApp::Terminal,
        TerminalApp::Ghostty,
        TerminalApp::Iterm2,
    ];

    /// The wire value.
    pub fn id(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "terminal",
            TerminalApp::Ghostty => "ghostty",
            TerminalApp::Iterm2 => "iterm2",
        }
    }

    /// From the wire value. Anything else is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|app| app.id() == value)
    }

    /// The app's own name, which shells put in `{destination}` and
    /// `{terminal}`.
    pub fn name(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "Terminal",
            TerminalApp::Ghostty => "Ghostty",
            TerminalApp::Iterm2 => "iTerm2",
        }
    }

    /// How LaunchServices knows the app. Installation is checked by this,
    /// never by a name or a path.
    #[cfg(any(target_os = "macos", test))]
    fn bundle_id(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "com.apple.Terminal",
            TerminalApp::Ghostty => "com.mitchellh.ghostty",
            TerminalApp::Iterm2 => "com.googlecode.iterm2",
        }
    }

    /// The `osascript -e` lines that open `command` in this app. Fixed
    /// templates: `command` appears once, as an escaped string literal.
    ///
    /// Ghostty runs its `command` as `bash -c "exec -l <command>"` with the
    /// login environment, so the session process replaces the shell. `wait
    /// after command` keeps the window, and anything the helper printed, on
    /// screen after it exits, as Terminal's window stays.
    #[cfg(any(target_os = "macos", test))]
    fn script(self, command: &str) -> Vec<String> {
        let literal = applescript_literal(command);
        let id = self.bundle_id();
        match self {
            TerminalApp::Terminal => vec![
                format!("tell application id \"{id}\" to do script \"{literal}\""),
                format!("tell application id \"{id}\" to activate"),
            ],
            TerminalApp::Ghostty => vec![
                format!("tell application id \"{id}\""),
                "set sessionConfiguration to new surface configuration".into(),
                format!("set command of sessionConfiguration to \"{literal}\""),
                "set wait after command of sessionConfiguration to true".into(),
                "new window with configuration sessionConfiguration".into(),
                "activate".into(),
                "end tell".into(),
            ],
            TerminalApp::Iterm2 => vec![
                format!("tell application id \"{id}\""),
                format!("create window with default profile command \"{literal}\""),
                "activate".into(),
                "end tell".into(),
            ],
        }
    }
}

/// The stored choice (`managed_terminal` in the settings file). A value
/// this build does not know -- a hand edit, or a newer build's app -- loads
/// as `Unrecognized`: no destination, and a launch is refused. It is never
/// read as Terminal, and a save keeps it unrecognized.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalSetting {
    #[default]
    Terminal,
    Ghostty,
    Iterm2,
    #[serde(other)]
    Unrecognized,
}

impl TerminalSetting {
    pub fn app(self) -> Option<TerminalApp> {
        match self {
            TerminalSetting::Terminal => Some(TerminalApp::Terminal),
            TerminalSetting::Ghostty => Some(TerminalApp::Ghostty),
            TerminalSetting::Iterm2 => Some(TerminalApp::Iterm2),
            TerminalSetting::Unrecognized => None,
        }
    }
}

impl From<TerminalApp> for TerminalSetting {
    fn from(app: TerminalApp) -> Self {
        match app {
            TerminalApp::Terminal => TerminalSetting::Terminal,
            TerminalApp::Ghostty => TerminalSetting::Ghostty,
            TerminalApp::Iterm2 => TerminalSetting::Iterm2,
        }
    }
}

/// An AppleScript string literal's body. Escaped independently of the
/// shell quoting inside it, so a path never becomes script code.
#[cfg(any(target_os = "macos", test))]
fn applescript_literal(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
}

/// The terminal apps installed on this Mac, asked of LaunchServices by
/// bundle id in one bounded call that launches none of them. Empty
/// elsewhere, and when the question cannot be answered.
pub fn installed() -> Vec<TerminalApp> {
    #[cfg(target_os = "macos")]
    {
        let mut probe = std::process::Command::new("/usr/bin/osascript");
        probe.args(["-l", "JavaScript", "-e", &installed_probe()]);
        let Ok(output) = super::profiles::bounded_output(probe) else {
            return vec![];
        };
        if !output.status.success() {
            return vec![];
        }
        parse_installed(&String::from_utf8_lossy(&output.stdout))
    }
    #[cfg(not(target_os = "macos"))]
    {
        vec![]
    }
}

/// A JavaScript for Automation script printing one line per app in
/// [`TerminalApp::ALL`]: its bundle's path, or an empty line. Built only
/// from the fixed bundle ids.
#[cfg(any(target_os = "macos", test))]
fn installed_probe() -> String {
    let ids = TerminalApp::ALL
        .iter()
        .map(|app| format!("'{}'", app.bundle_id()))
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "ObjC.import('AppKit'); [{ids}].map(function (id) {{ \
         var url = $.NSWorkspace.sharedWorkspace.URLForApplicationWithBundleIdentifier(id); \
         return url.isNil() ? '' : ObjC.unwrap(url.path); }}).join('\\n')"
    )
}

/// An app counts as installed when LaunchServices names a bundle that is
/// still on disk.
#[cfg(any(target_os = "macos", test))]
fn parse_installed(stdout: &str) -> Vec<TerminalApp> {
    let lines: Vec<&str> = stdout.trim_end_matches('\n').split('\n').collect();
    TerminalApp::ALL
        .into_iter()
        .zip(lines)
        .filter(|(_, path)| {
            let path = Path::new(path.trim());
            path.is_absolute() && path.is_dir()
        })
        .map(|(app, _)| app)
        .collect()
}

/// The macOS launch with its outside effect passed in. Refuses an
/// unrecognized choice, and an app that is not installed, before running
/// anything. Never falls back to another app.
#[cfg(any(target_os = "macos", test))]
fn launch_macos(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    command: &str,
    run: impl FnOnce(Vec<String>) -> Result<(), ManagedError>,
) -> Result<(), ManagedError> {
    let app = choice.app().ok_or(ManagedError::InvalidRequest)?;
    if !installed.contains(&app) {
        return Err(ManagedError::TerminalNotInstalled);
    }
    run(app.script(command))
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

/// Where a session opens, by name. On macOS, the chosen app's, and none
/// for a choice this build does not know; elsewhere `choice` is ignored.
pub fn destination(choice: TerminalSetting) -> Option<&'static str> {
    if helper_path().is_err() {
        return None;
    }
    destination_for(choice)
}

/// Whether this build can open a terminal at all. On macOS that needs only
/// the helper: a choice that is not installed is refused at launch, with
/// its own label, rather than hiding the button.
pub fn launch_available() -> bool {
    destination(TerminalSetting::Terminal).is_some()
}

fn destination_for(choice: TerminalSetting) -> Option<&'static str> {
    #[cfg(target_os = "macos")]
    {
        choice.app().map(TerminalApp::name)
    }
    #[cfg(windows)]
    {
        let _ = choice;
        external_terminal("wt.exe").map(|_| "Windows Terminal")
    }
    #[cfg(target_os = "linux")]
    {
        let _ = choice;
        if std::path::Path::new("/.flatpak-info").exists() {
            None
        } else {
            external_terminal("x-terminal-emulator").map(|_| "System terminal")
        }
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = choice;
        None
    }
}

/// Open the session in a terminal. On macOS, in the chosen app, which must
/// be among `installed`; elsewhere both are ignored.
pub fn launch(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    config: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<(), ManagedError> {
    let command = shell_command(&helper_path()?, config, session, ticket)?;
    #[cfg(target_os = "macos")]
    {
        launch_macos(choice, installed, &command, |lines| {
            let mut script = std::process::Command::new("/usr/bin/osascript");
            for line in &lines {
                script.args(["-e", line]);
            }
            let output = super::profiles::bounded_output(script)?;
            if output.status.success() {
                Ok(())
            } else {
                Err(ManagedError::LaunchUnknown)
            }
        })
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        let _ = (command, installed);
        if destination(choice).is_none() {
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
        let _ = (command, choice, installed);
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A config folder whose path tries to close the AppleScript string
    /// and run its own script, and to escape the shell quoting too.
    fn hostile_command() -> String {
        shell_command(
            Path::new("/Applications/Trace Commons.app/Contents/MacOS/near-ai"),
            Path::new("/Users/x/a\"; do shell script \"touch /tmp/pwn\" --\\\n'b"),
            Uuid::nil(),
            "abcdef01",
        )
        .unwrap()
    }

    /// Undo `applescript_literal`, refusing anything it would not write:
    /// an unescaped quote ends the literal, so it must not appear.
    fn read_literal(body: &str) -> String {
        let mut out = String::new();
        let mut chars = body.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => match chars.next() {
                    Some('\\') => out.push('\\'),
                    Some('"') => out.push('"'),
                    Some('n') => out.push('\n'),
                    Some('r') => out.push('\r'),
                    other => panic!("unexpected escape {other:?}"),
                },
                '"' => panic!("an unescaped quote ends the literal early"),
                '\n' | '\r' => panic!("a raw line break in the literal"),
                c => out.push(c),
            }
        }
        out
    }

    #[test]
    fn the_choice_parses_only_its_three_values() {
        for app in TerminalApp::ALL {
            assert_eq!(TerminalApp::parse(app.id()), Some(app));
        }
        for refused in [
            "",
            "Terminal",
            "iTerm2",
            "iterm",
            "warp",
            "/Applications/Ghostty.app",
            "ghostty; rm -rf /",
            " terminal",
        ] {
            assert_eq!(TerminalApp::parse(refused), None, "{refused:?}");
        }
    }

    #[test]
    fn an_unknown_stored_choice_is_unrecognized_never_terminal() {
        let read = |v: &str| serde_json::from_value::<TerminalSetting>(serde_json::json!(v));
        assert_eq!(read("ghostty").unwrap(), TerminalSetting::Ghostty);
        assert_eq!(read("iterm2").unwrap(), TerminalSetting::Iterm2);
        assert_eq!(read("terminal").unwrap(), TerminalSetting::Terminal);
        let unknown = read("/bin/sh").unwrap();
        assert_eq!(unknown, TerminalSetting::Unrecognized);
        assert_eq!(unknown.app(), None);
        // A save keeps it unrecognized rather than writing Terminal.
        let saved = serde_json::to_value(unknown).unwrap();
        assert_eq!(
            serde_json::from_value::<TerminalSetting>(saved).unwrap(),
            TerminalSetting::Unrecognized
        );
        assert_eq!(
            TerminalSetting::default().app(),
            Some(TerminalApp::Terminal)
        );
    }

    #[test]
    fn every_script_holds_the_command_only_as_one_string_literal() {
        let command = hostile_command();
        for app in TerminalApp::ALL {
            let lines = app.script(&command);
            let holding: Vec<&String> = lines
                .iter()
                .filter(|line| line.contains("near-ai"))
                .collect();
            assert_eq!(holding.len(), 1, "{app:?}: {lines:?}");
            let line = holding[0];
            // The fixed template, then one literal running to the line's end.
            let fixed = match app {
                TerminalApp::Terminal => "tell application id \"com.apple.Terminal\" to do script ",
                TerminalApp::Ghostty => "set command of sessionConfiguration to ",
                TerminalApp::Iterm2 => "create window with default profile command ",
            };
            let rest = line.strip_prefix(fixed).expect("fixed template");
            assert!(rest.starts_with('"') && rest.ends_with('"'), "{app:?}");
            assert_eq!(read_literal(&rest[1..rest.len() - 1]), command, "{app:?}");
            // No other line carries anything from the command.
            for other in lines.iter().filter(|l| *l != line) {
                assert!(
                    !other.contains("pwn") && !other.contains("Users"),
                    "{other}"
                );
            }
        }
    }

    #[test]
    fn each_app_has_its_own_fixed_template() {
        let lines = |app: TerminalApp| app.script("'/x/near-ai' redeem-launch");
        assert_eq!(
            lines(TerminalApp::Terminal),
            [
                "tell application id \"com.apple.Terminal\" to do script \"'/x/near-ai' redeem-launch\"",
                "tell application id \"com.apple.Terminal\" to activate",
            ]
        );
        assert_eq!(
            lines(TerminalApp::Ghostty),
            [
                "tell application id \"com.mitchellh.ghostty\"",
                "set sessionConfiguration to new surface configuration",
                "set command of sessionConfiguration to \"'/x/near-ai' redeem-launch\"",
                "set wait after command of sessionConfiguration to true",
                "new window with configuration sessionConfiguration",
                "activate",
                "end tell",
            ]
        );
        assert_eq!(
            lines(TerminalApp::Iterm2),
            [
                "tell application id \"com.googlecode.iterm2\"",
                "create window with default profile command \"'/x/near-ai' redeem-launch\"",
                "activate",
                "end tell",
            ]
        );
    }

    #[test]
    fn the_literal_escapes_quotes_backslashes_and_line_breaks() {
        assert_eq!(applescript_literal("a\"b\\c\nd\re"), "a\\\"b\\\\c\\nd\\re");
        assert_eq!(read_literal(&applescript_literal("\\\"")), "\\\"");
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_destination_follows_the_choice() {
        assert_eq!(destination_for(TerminalSetting::Terminal), Some("Terminal"));
        assert_eq!(destination_for(TerminalSetting::Ghostty), Some("Ghostty"));
        assert_eq!(destination_for(TerminalSetting::Iterm2), Some("iTerm2"));
        assert_eq!(destination_for(TerminalSetting::Unrecognized), None);
    }

    #[test]
    fn a_chosen_app_that_is_not_installed_is_refused_without_running() {
        let never = |_: Vec<String>| -> Result<(), ManagedError> {
            panic!("nothing runs when the chosen app is missing")
        };
        assert_eq!(
            launch_macos(
                TerminalSetting::Ghostty,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                never
            ),
            Err(ManagedError::TerminalNotInstalled)
        );
        assert_eq!(
            launch_macos(TerminalSetting::Iterm2, &[], "'/x/near-ai'", never),
            Err(ManagedError::TerminalNotInstalled)
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Unrecognized,
                &TerminalApp::ALL,
                "'/x/near-ai'",
                never
            ),
            Err(ManagedError::InvalidRequest)
        );
        let mut ran = None;
        launch_macos(
            TerminalSetting::Ghostty,
            &[TerminalApp::Ghostty],
            "'/x/near-ai'",
            |lines| {
                ran = Some(lines);
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(ran, Some(TerminalApp::Ghostty.script("'/x/near-ai'")));
    }

    #[test]
    fn installed_apps_are_read_by_position_and_must_exist() {
        let here = std::env::temp_dir();
        let here = here.to_str().unwrap();
        assert_eq!(
            parse_installed(&format!("{here}\n\n{here}\n")),
            [TerminalApp::Terminal, TerminalApp::Iterm2]
        );
        assert_eq!(
            parse_installed(&format!("\n{here}\n")),
            [TerminalApp::Ghostty]
        );
        // Relative or missing paths do not count; nor does no answer.
        assert!(parse_installed("Terminal.app\n/nonexistent/Ghostty.app\n\n").is_empty());
        assert!(parse_installed("").is_empty());
        let probe = installed_probe();
        for app in TerminalApp::ALL {
            assert!(probe.contains(app.bundle_id()));
        }
    }

    /// The real probe on a Mac: Terminal ships with macOS, so LaunchServices
    /// always names it.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_probe_finds_terminal_on_this_mac() {
        assert!(installed().contains(&TerminalApp::Terminal));
    }
}
