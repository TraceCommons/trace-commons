//! External terminal handoff. Only short-lived launch authority crosses argv.
//!
//! The contributor chooses which terminal app opens a session, from a closed
//! set per platform ([`TerminalApp::offered`]). An app is offered only when it
//! takes both a command and a working directory from its own command line or
//! scripting interface; `docs/managed-sessions.md` lists the evidence and
//! the apps left out.
//!
//! On macOS each app has a fixed AppleScript template; the only thing
//! interpolated into it is the shell command, as an escaped AppleScript
//! string literal. On Windows and Linux the terminal is started with an argv
//! vector ([`plan`]): no shell parses the folder, the helper's path or the
//! ticket. No app name, path or command ever comes from a setting.
use super::ManagedError;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use uuid::Uuid;

/// The terminal apps a session can open in. Closed: anything else is refused
/// by [`TerminalApp::parse`], never read as an app or a command. Each
/// platform offers only its own ([`TerminalApp::offered`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalApp {
    Terminal,
    Ghostty,
    Iterm2,
    WindowsTerminal,
    GnomeTerminal,
    Konsole,
    Kitty,
    Wezterm,
    Alacritty,
    Warp,
    Rex,
}

impl TerminalApp {
    /// Every app any platform offers.
    pub const ALL: [TerminalApp; 11] = [
        TerminalApp::Terminal,
        TerminalApp::Ghostty,
        TerminalApp::Iterm2,
        TerminalApp::WindowsTerminal,
        TerminalApp::GnomeTerminal,
        TerminalApp::Konsole,
        TerminalApp::Kitty,
        TerminalApp::Wezterm,
        TerminalApp::Alacritty,
        TerminalApp::Warp,
        TerminalApp::Rex,
    ];
    /// macOS, in the order a picker shows them.
    pub const MACOS: [TerminalApp; 8] = [
        TerminalApp::Terminal,
        TerminalApp::Ghostty,
        TerminalApp::Iterm2,
        TerminalApp::Kitty,
        TerminalApp::Wezterm,
        TerminalApp::Alacritty,
        TerminalApp::Warp,
        TerminalApp::Rex,
    ];
    /// Windows.
    pub const WINDOWS: [TerminalApp; 1] = [TerminalApp::WindowsTerminal];
    /// Linux, in the order a picker shows them.
    pub const LINUX: [TerminalApp; 6] = [
        TerminalApp::GnomeTerminal,
        TerminalApp::Konsole,
        TerminalApp::Ghostty,
        TerminalApp::Kitty,
        TerminalApp::Wezterm,
        TerminalApp::Alacritty,
    ];

    /// The apps this platform offers, in picker order. Empty where managed
    /// sessions cannot open a terminal.
    pub fn offered() -> &'static [TerminalApp] {
        if cfg!(target_os = "macos") {
            &Self::MACOS
        } else if cfg!(windows) {
            &Self::WINDOWS
        } else if cfg!(target_os = "linux") {
            &Self::LINUX
        } else {
            &[]
        }
    }

    /// The wire value.
    pub fn id(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "terminal",
            TerminalApp::Ghostty => "ghostty",
            TerminalApp::Iterm2 => "iterm2",
            TerminalApp::WindowsTerminal => "windows_terminal",
            TerminalApp::GnomeTerminal => "gnome_terminal",
            TerminalApp::Konsole => "konsole",
            TerminalApp::Kitty => "kitty",
            TerminalApp::Wezterm => "wezterm",
            TerminalApp::Alacritty => "alacritty",
            TerminalApp::Warp => "warp",
            TerminalApp::Rex => "rex",
        }
    }

    /// From the wire value, among the apps this platform offers. Anything
    /// else, another platform's app included, is `None`.
    pub fn parse(value: &str) -> Option<Self> {
        Self::offered()
            .iter()
            .copied()
            .find(|app| app.id() == value)
    }

    /// The app's own name, which shells put in `{destination}` and
    /// `{terminal}`.
    pub fn name(self) -> &'static str {
        match self {
            TerminalApp::Terminal => "Terminal",
            TerminalApp::Ghostty => "Ghostty",
            TerminalApp::Iterm2 => "iTerm2",
            TerminalApp::WindowsTerminal => "Windows Terminal",
            TerminalApp::GnomeTerminal => "GNOME Terminal",
            TerminalApp::Konsole => "Konsole",
            TerminalApp::Kitty => "kitty",
            TerminalApp::Wezterm => "WezTerm",
            TerminalApp::Alacritty => "Alacritty",
            TerminalApp::Warp => "Warp",
            TerminalApp::Rex => "Rex",
        }
    }

    /// How LaunchServices knows a macOS app. Installation is checked by
    /// this, never by a name or a path. `None` for the other platforms'.
    #[cfg(any(target_os = "macos", test))]
    fn bundle_id(self) -> Option<&'static str> {
        match self {
            TerminalApp::Terminal => Some("com.apple.Terminal"),
            TerminalApp::Ghostty => Some("com.mitchellh.ghostty"),
            TerminalApp::Iterm2 => Some("com.googlecode.iterm2"),
            TerminalApp::Kitty => Some("net.kovidgoyal.kitty"),
            TerminalApp::Wezterm => Some("com.github.wez.wezterm"),
            TerminalApp::Alacritty => Some("org.alacritty"),
            TerminalApp::Warp => Some("dev.warp.Warp-Stable"),
            TerminalApp::Rex => Some(super::rex::BUNDLE_ID),
            _ => None,
        }
    }

    /// The program a Windows or Linux launch starts, looked up on `PATH`.
    /// `None` for an app that is not started by program name here.
    #[cfg(any(windows, target_os = "linux", test))]
    fn executable(self) -> Option<&'static str> {
        match self {
            TerminalApp::WindowsTerminal => Some("wt.exe"),
            TerminalApp::GnomeTerminal => Some("gnome-terminal"),
            TerminalApp::Konsole => Some("konsole"),
            TerminalApp::Ghostty => Some("ghostty"),
            TerminalApp::Kitty => Some("kitty"),
            TerminalApp::Wezterm => Some("wezterm"),
            TerminalApp::Alacritty => Some("alacritty"),
            TerminalApp::Terminal | TerminalApp::Iterm2 | TerminalApp::Warp | TerminalApp::Rex => {
                None
            }
        }
    }

    /// The `osascript -e` lines that open `command` in this macOS app.
    /// Fixed templates: `command` appears once, as an escaped string
    /// literal. Empty for another platform's app.
    ///
    /// Ghostty runs its `command` as `bash -c "exec -l <command>"` with the
    /// login environment, so the session process replaces the shell. `wait
    /// after command` keeps the window, and anything the helper printed, on
    /// screen after it exits, as Terminal's window stays.
    #[cfg(any(target_os = "macos", test))]
    fn script(self, command: &str) -> Vec<String> {
        let literal = applescript_literal(command);
        let Some(id) = self.bundle_id() else {
            return vec![];
        };
        match self {
            // Addressed by bundle id, like the others, so the app that
            // receives the command is the one the installed check found.
            // The new window's tty and window id are printed, and read
            // back only as a bare device path and digits. Any failure there
            // prints less: the launch stands and the session simply cannot
            // be shown.
            TerminalApp::Terminal => [
                vec![format!("tell application id \"{id}\"")],
                open_window(&literal),
                vec!["return found".into(), "end tell".into()],
            ]
            .concat(),
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
            _ => vec![],
        }
    }
}

/// Terminal's lines that set `wid` to the id of the window holding the tab
/// whose tty is the AppleScript value `tty` (a quoted literal or a
/// variable), stopping at the first match, or leave it `missing value`. A
/// window's id never changes; its position does, whenever one comes
/// forward.
#[cfg(any(target_os = "macos", test))]
fn find_window(tty: &str) -> Vec<String> {
    vec![
        "repeat with w in windows".into(),
        "repeat with x in tabs of w".into(),
        format!("if tty of x is {tty} then"),
        "set wid to id of w".into(),
        "exit repeat".into(),
        "end if".into(),
        "end repeat".into(),
        "if wid is not missing value then exit repeat".into(),
        "end repeat".into(),
    ]
}

/// Terminal's lines that set `tw` to window `wid` and `t` to its tab on
/// `tty`, or leave `t` `missing value`. Run just before `t` is used, so a
/// window that moved since the scan cannot put another tab there.
#[cfg(any(target_os = "macos", test))]
fn find_tab(tty: &str) -> Vec<String> {
    vec![
        "if wid is not missing value then".into(),
        "set tw to window id wid".into(),
        "repeat with x in tabs of tw".into(),
        format!("if tty of x is \"{tty}\" then"),
        "set t to x".into(),
        "exit repeat".into(),
        "end if".into(),
        "end repeat".into(),
        "end if".into(),
    ]
}

/// Terminal's lines that select tab `t` and bring its window, `tw`,
/// forward.
#[cfg(any(target_os = "macos", test))]
fn raise_tab() -> Vec<String> {
    vec![
        "set selected of t to true".into(),
        "if miniaturized of tw then set miniaturized of tw to false".into(),
        "set index of tw to 1".into(),
    ]
}

/// Terminal's lines that run the AppleScript literal `literal` in a new
/// window and set `found` to its tab's tty and, on the next line, the id of
/// Terminal's front window when its selected tab is this one (no other
/// window is scanned). If another window is in front by then, the row gets
/// no window id, which is normal. Run after the command is
/// typed, so each read is inside a `try`: a failure prints less, the launch
/// stands, and the session simply cannot be shown.
#[cfg(any(target_os = "macos", test))]
fn open_window(literal: &str) -> Vec<String> {
    [vec![
        format!("set t to do script \"{literal}\""),
        "activate".into(),
        "set found to \"\"".into(),
        "try".into(),
        "set found to tty of t".into(),
        "end try".into(),
        // The window id, read only from the front window and only
        // when its selected tab is this one: no scan of other windows.
        "try".into(),
        "if found is not \"\" and tty of selected tab of front window is found then".into(),
        "set found to found & linefeed & ((id of front window) as text)".into(),
        "end if".into(),
        "end try".into(),
    ]]
    .concat()
}

/// Terminal's lines that bring the tab on `tty` to the front, in whichever
/// window holds it now, or fail when it is gone. `tty` has passed
/// `valid_tty`.
#[cfg(any(target_os = "macos", test))]
fn focus_script(tty: &str) -> Vec<String> {
    let id = TerminalApp::Terminal
        .bundle_id()
        .unwrap_or("com.apple.Terminal");
    [
        vec![
            format!("tell application id \"{id}\""),
            "set wid to missing value".into(),
            "set t to missing value".into(),
        ],
        find_window(&format!("\"{tty}\"")),
        find_tab(tty),
        vec!["if t is missing value then error \"no tab\"".into()],
        raise_tab(),
        vec!["activate".into(), "end tell".into()],
    ]
    .concat()
}

/// The stored choice (`managed_terminal` in the settings file). A value
/// this build does not know -- a hand edit, a newer build's app, or another
/// platform's -- reads as no app: no destination, and a launch is refused.
/// It is never read as the default, and a save keeps it as it was read.
///
/// A file written before the key existed loads as the platform's default:
/// Terminal on macOS, Windows Terminal on Windows (both what shipped before
/// the choice), GNOME Terminal on Linux.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalSetting {
    Terminal,
    Ghostty,
    Iterm2,
    WindowsTerminal,
    GnomeTerminal,
    Konsole,
    Kitty,
    Wezterm,
    Alacritty,
    Warp,
    Rex,
    #[serde(other)]
    Unrecognized,
}

impl Default for TerminalSetting {
    fn default() -> Self {
        if cfg!(windows) {
            TerminalSetting::WindowsTerminal
        } else if cfg!(target_os = "linux") {
            TerminalSetting::GnomeTerminal
        } else {
            TerminalSetting::Terminal
        }
    }
}

impl TerminalSetting {
    /// The chosen app, when this platform offers it.
    pub fn app(self) -> Option<TerminalApp> {
        let app = match self {
            TerminalSetting::Terminal => TerminalApp::Terminal,
            TerminalSetting::Ghostty => TerminalApp::Ghostty,
            TerminalSetting::Iterm2 => TerminalApp::Iterm2,
            TerminalSetting::WindowsTerminal => TerminalApp::WindowsTerminal,
            TerminalSetting::GnomeTerminal => TerminalApp::GnomeTerminal,
            TerminalSetting::Konsole => TerminalApp::Konsole,
            TerminalSetting::Kitty => TerminalApp::Kitty,
            TerminalSetting::Wezterm => TerminalApp::Wezterm,
            TerminalSetting::Alacritty => TerminalApp::Alacritty,
            TerminalSetting::Warp => TerminalApp::Warp,
            TerminalSetting::Rex => TerminalApp::Rex,
            TerminalSetting::Unrecognized => return None,
        };
        TerminalApp::offered().contains(&app).then_some(app)
    }
}

impl From<TerminalApp> for TerminalSetting {
    fn from(app: TerminalApp) -> Self {
        match app {
            TerminalApp::Terminal => TerminalSetting::Terminal,
            TerminalApp::Ghostty => TerminalSetting::Ghostty,
            TerminalApp::Iterm2 => TerminalSetting::Iterm2,
            TerminalApp::WindowsTerminal => TerminalSetting::WindowsTerminal,
            TerminalApp::GnomeTerminal => TerminalSetting::GnomeTerminal,
            TerminalApp::Konsole => TerminalSetting::Konsole,
            TerminalApp::Kitty => TerminalSetting::Kitty,
            TerminalApp::Wezterm => TerminalSetting::Wezterm,
            TerminalApp::Alacritty => TerminalSetting::Alacritty,
            TerminalApp::Warp => TerminalSetting::Warp,
            TerminalApp::Rex => TerminalSetting::Rex,
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

/// The terminal apps installed here, among those offered. On macOS asked
/// of LaunchServices by bundle id in one bounded call that launches none
/// of them; on Windows and Linux, by the app's program on `PATH`. Empty in
/// a Flatpak sandbox, elsewhere, and when the question cannot be answered.
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
    #[cfg(any(windows, target_os = "linux"))]
    {
        if sandboxed() {
            return vec![];
        }
        TerminalApp::offered()
            .iter()
            .copied()
            .filter(|app| app.executable().and_then(external_terminal).is_some())
            .collect()
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        vec![]
    }
}

/// The last answer to "which terminal apps are installed", reused for
/// [`InstalledCache::FRESH`] so a snapshot does not start a probe process
/// each time a shell asks. A launch in an app the answer lacks asks again,
/// so an app installed a moment ago is not refused for the window's length.
#[derive(Debug, Default)]
pub struct InstalledCache {
    answer: Option<(std::time::Instant, Vec<TerminalApp>)>,
}

impl InstalledCache {
    pub const FRESH: std::time::Duration = std::time::Duration::from_secs(30);

    /// The installed apps at `now`. `launching` is the app a launch is
    /// about to open, if any; `probe` asks the OS.
    pub fn get(
        &mut self,
        now: std::time::Instant,
        launching: Option<TerminalApp>,
        probe: impl FnOnce() -> Vec<TerminalApp>,
    ) -> Vec<TerminalApp> {
        if let Some((at, apps)) = &self.answer {
            let fresh = now.saturating_duration_since(*at) <= Self::FRESH;
            let covers = launching.is_none_or(|app| apps.contains(&app));
            if fresh && covers {
                return apps.clone();
            }
        }
        let apps = probe();
        self.answer = Some((now, apps.clone()));
        apps
    }
}

/// A JavaScript for Automation script printing one line per app in
/// [`TerminalApp::MACOS`]: its bundle's path, or an empty line. Built only
/// from the fixed bundle ids.
#[cfg(any(target_os = "macos", test))]
fn installed_probe() -> String {
    let ids = TerminalApp::MACOS
        .iter()
        .filter_map(|app| app.bundle_id())
        .map(|id| format!("'{id}'"))
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
    TerminalApp::MACOS
        .into_iter()
        .zip(lines)
        .filter(|(_, path)| {
            let path = Path::new(path.trim());
            path.is_absolute() && path.is_dir()
        })
        .map(|(app, _)| app)
        .collect()
}

/// What a macOS launch runs: a scripted app's `osascript -e` lines
/// (Terminal, Ghostty, iTerm2), the `open` plan of an app that takes its
/// command as argv (kitty, WezTerm, Alacritty), or a Warp Tab Config.
#[cfg(target_os = "macos")]
#[derive(Debug, Clone, PartialEq, Eq)]
enum MacosLaunch {
    Script(Vec<String>),
    Open(Plan),
    Warp(WarpTab),
    Rex(super::rex::RexTab),
}

/// A Warp launch: a Tab Config, `<name>.toml` in `~/.warp/tab_configs`,
/// opened with `warp://tab_config/<name>`. Warp opens it as a new tab in
/// its active window (a new window when it has none) and runs the command
/// there. A URL, not an Apple Event, so macOS asks for no permission
/// (owner, 2026-11-03: sessions as tabs in one window, with no prompts).
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct WarpTab {
    /// `near-ai-` and a fresh uuid: only `[a-z0-9-]`, so it is safe in the
    /// file name and the URL, and no two launches share a file.
    name: String,
    contents: String,
}

#[cfg(any(target_os = "macos", test))]
impl WarpTab {
    /// The Tab Config that runs `command` in `cwd`, or in Warp's default
    /// folder when there is none.
    fn new(command: &str, cwd: Option<&Path>) -> Result<Self, ManagedError> {
        let name = format!("near-ai-{}", Uuid::new_v4());
        let directory = match cwd {
            Some(cwd) => format!(
                "directory = {}\n",
                toml_string(cwd.to_str().ok_or(ManagedError::InvalidRequest)?)
            ),
            None => String::new(),
        };
        let contents = format!(
            "name = {name}\ntitle = \"Near.ai\"\n\n[[panes]]\nid = \"main\"\ntype = \"terminal\"\n{directory}commands = [{command}]\n",
            name = toml_string(&name),
            command = toml_string(command),
        );
        Ok(WarpTab { name, contents })
    }

    fn url(&self) -> String {
        format!("warp://tab_config/{}", self.name)
    }
}

/// A TOML basic string: `\\` and `\"` escaped, and every control
/// character as `\uXXXX`, so a folder name never becomes TOML.
#[cfg(any(target_os = "macos", test))]
fn toml_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Warp's Tab Config folder, `~/.warp/tab_configs`.
#[cfg(target_os = "macos")]
fn warp_tab_configs() -> Option<PathBuf> {
    dirs::home_dir().map(|home| home.join(".warp").join("tab_configs"))
}

/// Write `tab`, owner-only, and open it in Warp. The file holds a
/// single-use launch ticket, so it is removed a minute later, once Warp
/// has read it, whether or not the launch went ahead.
#[cfg(target_os = "macos")]
fn open_warp_tab(tab: &WarpTab) -> Result<(), ManagedError> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let folder = warp_tab_configs().ok_or(ManagedError::LaunchUnknown)?;
    std::fs::create_dir_all(&folder).map_err(|_| ManagedError::LaunchUnknown)?;
    let path = folder.join(format!("{}.toml", tab.name));
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&path)
        .and_then(|mut file| file.write_all(tab.contents.as_bytes()));
    if written.is_err() {
        let _ = std::fs::remove_file(&path);
        return Err(ManagedError::LaunchUnknown);
    }
    let _ = std::thread::Builder::new()
        .name("warp-tab-config".into())
        .spawn({
            let path = path.clone();
            move || {
                std::thread::sleep(std::time::Duration::from_secs(60));
                let _ = std::fs::remove_file(path);
            }
        });
    let mut open = std::process::Command::new("/usr/bin/open");
    open.arg(tab.url());
    match super::profiles::bounded_output(open) {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(ManagedError::LaunchUnknown),
    }
}

/// The macOS launch with its outside effect passed in. Refuses an
/// unrecognized choice, and an app that was not found, before running
/// anything. Never falls back to another app. A scripted app gets
/// `command`, the one shell string; an argv app gets `argv` and `cwd`.
/// `run` returns what was printed; only Terminal's script prints a tab,
/// which is read back as a bare tty path and window id and nothing else.
#[cfg(target_os = "macos")]
fn launch_macos(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    command: &str,
    argv: Vec<std::ffi::OsString>,
    cwd: Option<&Path>,
    run: impl FnOnce(MacosLaunch) -> Result<Vec<u8>, ManagedError>,
) -> Result<Option<TerminalTab>, ManagedError> {
    let app = choice.app().ok_or(ManagedError::InvalidRequest)?;
    if !installed.contains(&app) {
        return Err(ManagedError::TerminalNotFound);
    }
    let how = match app {
        TerminalApp::Warp => MacosLaunch::Warp(WarpTab::new(command, cwd)?),
        TerminalApp::Rex => MacosLaunch::Rex(super::rex::RexTab::new(&argv, cwd)?),
        _ => match open_plan(app, cwd, argv) {
            Some(plan) => MacosLaunch::Open(plan),
            None => MacosLaunch::Script(app.script(command)),
        },
    };
    // Once the app is asked, anything that goes wrong may follow a typed
    // command, so it reads as `LaunchUnknown`, never as a refusal from
    // before anything ran; only the refusals above release a held launch.
    // The one exception is Rex's CLI not being found, which is known before
    // Rex is asked anything.
    let printed = run(how).map_err(|error| match error {
        ManagedError::TerminalNotFound if app == TerminalApp::Rex => error,
        _ => ManagedError::LaunchUnknown,
    })?;
    Ok(match app {
        TerminalApp::Terminal => terminal_tab(&printed),
        TerminalApp::Rex => super::rex::tab(&printed),
        _ => None,
    })
}

/// Run a macOS launch: `osascript` with the script's lines, or `open` with
/// the plan's argv. Both return as soon as the app has the command.
#[cfg(target_os = "macos")]
fn run_macos(how: MacosLaunch) -> Result<Vec<u8>, ManagedError> {
    let command = match how {
        MacosLaunch::Script(lines) => {
            let mut script = std::process::Command::new("/usr/bin/osascript");
            for line in &lines {
                script.args(["-e", line]);
            }
            script
        }
        MacosLaunch::Open(plan) => {
            let mut open = std::process::Command::new(&plan.program);
            open.args(&plan.args);
            open
        }
        MacosLaunch::Warp(tab) => return open_warp_tab(&tab).map(|()| Vec::new()),
        MacosLaunch::Rex(tab) => return super::rex::open_tab(&tab),
    };
    let output = super::profiles::bounded_output(command)?;
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(ManagedError::LaunchUnknown)
    }
}

/// What a Windows or Linux launch starts, and what a macOS launch in kitty,
/// WezTerm or Alacritty hands to `open`: `program` with `args`, each one
/// argv element, never joined into a string a shell would parse.
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
struct Plan {
    program: PathBuf,
    args: Vec<std::ffi::OsString>,
}

/// The helper's own arguments, after its path.
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn helper_args(config: &Path, session: Uuid, ticket: &str) -> Vec<std::ffi::OsString> {
    let mut args = helper_args_without_ticket(config, session);
    args.extend(["--ticket".into(), ticket.into()]);
    args
}

/// The helper's arguments with no ticket, for a terminal that hands it the
/// ticket in [`LAUNCH_TICKET_ENV`] instead (herdr).
pub(crate) fn helper_args_without_ticket(config: &Path, session: Uuid) -> Vec<std::ffi::OsString> {
    vec![
        "--config-dir".into(),
        config.as_os_str().to_owned(),
        "redeem-launch".into(),
        "--session-id".into(),
        session.to_string().into(),
    ]
}

/// The variable a terminal that can set one gives the helper the launch
/// ticket in, instead of argv (`redeem-launch` reads it when `--ticket` is
/// absent). The helper removes it before starting the tool.
pub const LAUNCH_TICKET_ENV: &str = "NEAR_AI_LAUNCH_TICKET";

/// The terminal's own arguments, before the command: `cwd` as its working
/// directory when given, keeping the window after the command exits (as
/// Terminal's window stays on macOS), and whatever marks where the command
/// starts. `None` for an app that is not started by argv (Terminal and
/// iTerm2, which are scripted). The same on every platform an app is
/// offered on; on macOS it follows `open --args`.
#[cfg(any(windows, target_os = "linux", target_os = "macos", test))]
fn terminal_args(app: TerminalApp, cwd: Option<&Path>) -> Option<Vec<std::ffi::OsString>> {
    use std::ffi::OsString;
    let with_value = |flag: &str, value: &Path| {
        let mut joined = OsString::from(flag);
        joined.push(value.as_os_str());
        joined
    };
    let mut args: Vec<OsString> = Vec::new();
    match app {
        // `--working-directory=DIRNAME`, then `--`: "interpret all
        // subsequent options as a program and arguments".
        TerminalApp::GnomeTerminal => {
            args.extend(cwd.map(|cwd| with_value("--working-directory=", cwd)));
            args.push("--".into());
        }
        // `-e` "will catch all following arguments", so it is last.
        // `--hold`: "Do not close the initial session automatically when
        // it ends".
        TerminalApp::Konsole => {
            if let Some(cwd) = cwd {
                args.extend(["--workdir".into(), cwd.as_os_str().to_owned()]);
            }
            args.extend(["--hold".into(), "-e".into()]);
        }
        // Every config key is a flag; `-e` takes the rest as the command
        // with shell expansion disabled.
        TerminalApp::Ghostty => {
            args.extend(cwd.map(|cwd| with_value("--working-directory=", cwd)));
            args.extend(["--wait-after-command=true".into(), "-e".into()]);
        }
        TerminalApp::WindowsTerminal => {
            args.extend(["new-tab".into(), "--title".into(), "NEAR AI".into()]);
            if let Some(cwd) = cwd {
                args.extend(["-d".into(), cwd.as_os_str().to_owned()]);
            }
        }
        // `kitty [options] [program-to-run ...]`: `--directory` "Change to
        // the specified directory when launching", `--hold` "Remain open,
        // at a shell prompt, after child process exits". The first
        // argument that is not an option is the program.
        TerminalApp::Kitty => {
            if let Some(cwd) = cwd {
                args.extend(["--directory".into(), cwd.as_os_str().to_owned()]);
            }
            args.push("--hold".into());
        }
        // `--config 'exit_behavior="Hold"'` overrides the config file and
        // keeps the pane; `start --always-new-process` is "don't try to ask
        // an existing wezterm GUI instance to start the command", so the
        // override applies; `--cwd`; then `--` and the program.
        TerminalApp::Wezterm => {
            args.extend([
                "--config".into(),
                "exit_behavior=\"Hold\"".into(),
                "start".into(),
                "--always-new-process".into(),
            ]);
            if let Some(cwd) = cwd {
                args.extend(["--cwd".into(), cwd.as_os_str().to_owned()]);
            }
            args.push("--".into());
        }
        // `--working-directory`, `--hold` "Remain open after child process
        // exits", and `-e` "Command and args to execute (must be last
        // argument)".
        TerminalApp::Alacritty => {
            if let Some(cwd) = cwd {
                args.extend(["--working-directory".into(), cwd.as_os_str().to_owned()]);
            }
            args.extend(["--hold".into(), "-e".into()]);
        }
        TerminalApp::Terminal | TerminalApp::Iterm2 | TerminalApp::Warp | TerminalApp::Rex => {
            return None;
        }
    }
    Some(args)
}

/// The argv that opens the helper in `app` with `cwd` as the terminal's
/// working directory. `program` is the terminal's own executable. Each
/// path is one element, verbatim: a folder holding spaces, quotes, `$`,
/// backticks or non-ASCII text reaches the terminal unchanged.
///
/// Windows Terminal reads a `;` anywhere in its command line as the start
/// of another command, so any argument holding one is refused rather than
/// escaped.
#[cfg(any(windows, target_os = "linux", test))]
fn plan(
    app: TerminalApp,
    program: &Path,
    helper: &Path,
    config: &Path,
    cwd: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<Plan, ManagedError> {
    if !program.is_absolute()
        || !helper.is_absolute()
        || !config.is_absolute()
        || !cwd.is_absolute()
        || !valid_ticket(ticket)
    {
        return Err(ManagedError::InvalidRequest);
    }
    let mut command = vec![helper.as_os_str().to_owned()];
    command.extend(helper_args(config, session, ticket));
    program_plan(app, program, cwd, command)
}

/// The argv that opens `command` (a program and its arguments, each one
/// element) in `app` with `cwd` as the terminal's working directory.
#[cfg(any(windows, target_os = "linux", test))]
fn program_plan(
    app: TerminalApp,
    program: &Path,
    cwd: &Path,
    command: Vec<std::ffi::OsString>,
) -> Result<Plan, ManagedError> {
    if !program.is_absolute() || !cwd.is_absolute() {
        return Err(ManagedError::InvalidRequest);
    }
    let mut args = terminal_args(app, Some(cwd)).ok_or(ManagedError::InvalidRequest)?;
    args.extend(command);
    if app == TerminalApp::WindowsTerminal
        && args.iter().any(|arg| arg.to_string_lossy().contains(';'))
    {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(Plan {
        program: program.to_path_buf(),
        args,
    })
}

/// A launch ticket is hex, and never empty.
pub(crate) fn valid_ticket(ticket: &str) -> bool {
    !ticket.is_empty() && ticket.bytes().all(|b| b.is_ascii_hexdigit())
}

/// How a macOS app that takes its command as argv (kitty, WezTerm,
/// Alacritty) is opened: `/usr/bin/open -n -b <bundle id> --args`, then the
/// terminal's arguments and `command`, each one argv element. `-n` opens a
/// new instance even when one is running, and `-b` addresses the app by
/// the bundle id the installed check found, as the scripts do. `open`
/// passes everything after `--args` to the app's `main` unparsed. `None`
/// for a scripted app, Ghostty included: on macOS it is scripted.
#[cfg(any(target_os = "macos", test))]
fn open_plan(
    app: TerminalApp,
    cwd: Option<&Path>,
    command: Vec<std::ffi::OsString>,
) -> Option<Plan> {
    if !matches!(
        app,
        TerminalApp::Kitty | TerminalApp::Wezterm | TerminalApp::Alacritty
    ) {
        return None;
    }
    let id = app.bundle_id()?;
    let mut args: Vec<std::ffi::OsString> =
        vec!["-n".into(), "-b".into(), id.into(), "--args".into()];
    args.extend(terminal_args(app, cwd)?);
    args.extend(command);
    Some(Plan {
        program: PathBuf::from("/usr/bin/open"),
        args,
    })
}

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
    /// A Rex tab: the "Near.ai" Rex session, the Rex window (a tab) the
    /// launch opened in it, and its one block. Each id is checked as Rex
    /// prints it (`session:`, `window:` or `block:` and a UUID) wherever it
    /// came from.
    Rex {
        #[serde(deserialize_with = "rex_session")]
        session: String,
        #[serde(deserialize_with = "rex_window")]
        window: String,
        #[serde(deserialize_with = "rex_block")]
        block: String,
    },
    /// A herdr pane, by its terminal id (`term_` and hex). herdr's public tab
    /// and pane ids are positional and move when a pane does; the terminal
    /// id stays with the process.
    Herdr {
        #[serde(deserialize_with = "herdr_terminal")]
        terminal: String,
    },
}

fn checked<'de, D: serde::Deserializer<'de>>(
    d: D,
    valid: impl Fn(&str) -> bool,
) -> Result<String, D::Error> {
    let text = String::deserialize(d)?;
    if valid(&text) {
        Ok(text)
    } else {
        Err(serde::de::Error::custom("not an id"))
    }
}

fn rex_session<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    checked(d, |t| super::rex::valid_id("session", t))
}

fn rex_window<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    checked(d, |t| super::rex::valid_id("window", t))
}

fn rex_block<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    checked(d, |t| super::rex::valid_id("block", t))
}

fn herdr_terminal<'de, D: serde::Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    checked(d, super::herdr::valid_terminal_id)
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

/// The tab a launch opened, as its script printed it: the tab's tty, and
/// the id of the window that held it then, when that could be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalTab {
    pub handle: TerminalHandle,
    /// Terminal's window id. Kept on the row by owner decision
    /// (2026-11-03) although the window picker it was recorded for is gone;
    /// nothing in production reads it.
    pub window: Option<u32>,
}

/// The tab a launch script printed: a bare tty path, then optionally the
/// window id on a line of its own. Anything else names no tab.
pub fn terminal_tab(output: &[u8]) -> Option<TerminalTab> {
    let text = std::str::from_utf8(output).ok()?;
    let text = text.strip_suffix('\n').unwrap_or(text);
    let (tty, window) = match text.split_once('\n') {
        Some((tty, window)) => {
            // Digits only: `u32::from_str` would also take a leading `+`.
            if !window.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            (tty, Some(window.parse::<u32>().ok()?))
        }
        None => (text, None),
    };
    valid_tty(tty).then(|| TerminalTab {
        handle: TerminalHandle::AppleTerminal {
            tty: tty.to_string(),
        },
        window,
    })
}

/// The tty a launch script printed, or none.
pub fn apple_terminal_tab(output: &[u8]) -> Option<TerminalHandle> {
    terminal_tab(output).map(|tab| tab.handle)
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
            let mut command = std::process::Command::new("/usr/bin/osascript");
            for line in focus_script(tty) {
                command.args(["-e", &line]);
            }
            let output = super::profiles::bounded_output(command)?;
            if output.status.success() {
                Ok(())
            } else {
                Err(ManagedError::NotFound)
            }
        }
        #[cfg(not(target_os = "macos"))]
        TerminalHandle::AppleTerminal { .. } => Err(ManagedError::UnsupportedVersion),
        #[cfg(target_os = "macos")]
        TerminalHandle::Rex { session, block, .. } => super::rex::focus(session, block),
        #[cfg(not(target_os = "macos"))]
        TerminalHandle::Rex { .. } => Err(ManagedError::UnsupportedVersion),
        TerminalHandle::Herdr { terminal } => super::herdr::focus(terminal),
    }
}

pub fn shell_command(
    helper: &Path,
    config: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<String, ManagedError> {
    if !helper.is_absolute() || !config.is_absolute() || !valid_ticket(ticket) {
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

/// Where a session opens, by name: the chosen app's, and none for a choice
/// this build does not know.
pub fn destination(choice: TerminalSetting) -> Option<&'static str> {
    if helper_path().is_err() {
        return None;
    }
    destination_for(choice)
}

/// Whether this build can open a terminal at all. That needs only the
/// helper, and on Linux no Flatpak sandbox: a choice that was not found is
/// refused at launch, with its own line, rather than hiding the button.
pub fn launch_available() -> bool {
    helper_path().is_ok() && supported()
}

/// Whether this platform opens terminals at all.
fn supported() -> bool {
    #[cfg(any(windows, target_os = "linux"))]
    {
        !sandboxed()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        cfg!(target_os = "macos")
    }
}

/// A Flatpak sandbox cannot start a host terminal.
#[cfg(any(windows, target_os = "linux"))]
fn sandboxed() -> bool {
    cfg!(target_os = "linux") && Path::new("/.flatpak-info").exists()
}

fn destination_for(choice: TerminalSetting) -> Option<&'static str> {
    if !supported() {
        return None;
    }
    choice.app().map(TerminalApp::name)
}

/// Tool maintenance opens a terminal without needing a managed-session helper,
/// account, profile, ticket or credential. Currently offered by native macOS.
pub fn tool_setup_available() -> bool {
    cfg!(target_os = "macos")
}

/// The script tool setup runs under `/bin/bash -lc`: the vendor's
/// commands, then a login shell, so their output stays visible in every
/// supported macOS terminal. The body is built by `tool_setup`, never
/// accepted as an IPC parameter.
#[cfg(any(target_os = "macos", test))]
fn setup_script(body: &str) -> Result<String, ManagedError> {
    if body.is_empty() || body.len() > 16_384 || body.contains('\0') {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(format!(
        "PATH=\"$HOME/.local/bin:$HOME/.opencode/bin:$PATH\"; export PATH; {body}; exec \"${{SHELL:-/bin/zsh}}\" -l"
    ))
}

/// The setup script as one shell string, for a scripted terminal.
#[cfg(any(target_os = "macos", test))]
fn setup_shell_command(body: &str) -> Result<String, ManagedError> {
    let script = setup_script(body)?;
    Ok(format!("/bin/bash -lc '{}'", script.replace('\'', "'\\''")))
}

/// The setup script as argv, for a terminal that takes one.
#[cfg(any(target_os = "macos", test))]
fn setup_argv(body: &str) -> Result<Vec<std::ffi::OsString>, ManagedError> {
    Ok(vec![
        "/bin/bash".into(),
        "-lc".into(),
        setup_script(body)?.into(),
    ])
}

pub fn launch_tool_setup(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    body: &str,
) -> Result<(), ManagedError> {
    #[cfg(target_os = "macos")]
    {
        let command = setup_shell_command(body)?;
        launch_macos(
            choice,
            installed,
            &command,
            setup_argv(body)?,
            None,
            run_macos,
        )?;
        Ok(())
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (choice, installed, body);
        Err(ManagedError::TerminalNotFound)
    }
}

/// Open the session in the chosen terminal, which must be among
/// `installed`, with `cwd` as its working directory where the app takes
/// one by command line (Windows, Linux, and kitty, WezTerm and Alacritty
/// on macOS). The helper itself starts the tool in `cwd` either way, so
/// the macOS scripts stay as they were.
///
/// The handle names the window it opened in, when the terminal can say
/// (Terminal's tab, by tty); a launch that cannot is still a launch, with no
/// handle.
///
pub fn launch(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    config: &Path,
    cwd: &Path,
    session: Uuid,
    ticket: &str,
) -> Result<Option<TerminalTab>, ManagedError> {
    #[cfg(target_os = "macos")]
    {
        if !cwd.is_absolute() {
            return Err(ManagedError::InvalidRequest);
        }
        let helper = helper_path()?;
        let command = shell_command(&helper, config, session, ticket)?;
        let mut argv = vec![helper.into_os_string()];
        argv.extend(helper_args(config, session, ticket));
        launch_macos(choice, installed, &command, argv, Some(cwd), run_macos)
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        if !supported() {
            return Err(ManagedError::UnsupportedVersion);
        }
        let app = choice.app().ok_or(ManagedError::InvalidRequest)?;
        if !installed.contains(&app) {
            return Err(ManagedError::TerminalNotFound);
        }
        let program = app
            .executable()
            .and_then(external_terminal)
            .ok_or(ManagedError::TerminalNotFound)?;
        let plan = plan(app, &program, &helper_path()?, config, cwd, session, ticket)?;
        spawn_plan(&plan)?;
        Ok(None)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (choice, installed, config, cwd, session, ticket);
        Err(ManagedError::UnsupportedVersion)
    }
}

/// Start a Windows or Linux terminal and leave it running.
#[cfg(any(windows, target_os = "linux"))]
fn spawn_plan(plan: &Plan) -> Result<(), ManagedError> {
    let mut process = std::process::Command::new(&plan.program)
        .args(&plan.args)
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

/// Open `program`, with no arguments and nothing secret, in the chosen
/// terminal with `cwd` as its folder: how a herdr client is started when
/// none is running. The same refusals as [`launch`]; nothing is recorded.
pub(crate) fn open_program(
    choice: TerminalSetting,
    installed: &[TerminalApp],
    program: &Path,
    cwd: &Path,
) -> Result<(), ManagedError> {
    if !program.is_absolute() || !cwd.is_absolute() {
        return Err(ManagedError::InvalidRequest);
    }
    #[cfg(target_os = "macos")]
    {
        let text = program.to_str().ok_or(ManagedError::InvalidRequest)?;
        let command = format!("'{}'", text.replace('\'', "'\\''"));
        launch_macos(
            choice,
            installed,
            &command,
            vec![program.as_os_str().to_owned()],
            Some(cwd),
            run_macos,
        )
        .map(|_| ())
    }
    #[cfg(any(windows, target_os = "linux"))]
    {
        if !supported() {
            return Err(ManagedError::UnsupportedVersion);
        }
        let app = choice.app().ok_or(ManagedError::InvalidRequest)?;
        if !installed.contains(&app) {
            return Err(ManagedError::TerminalNotFound);
        }
        let terminal = app
            .executable()
            .and_then(external_terminal)
            .ok_or(ManagedError::TerminalNotFound)?;
        spawn_plan(&program_plan(
            app,
            &terminal,
            cwd,
            vec![program.as_os_str().to_owned()],
        )?)
    }
    #[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
    {
        let _ = (choice, installed);
        Err(ManagedError::UnsupportedVersion)
    }
}

#[cfg(any(windows, target_os = "linux"))]
pub(crate) fn external_terminal(name: &str) -> Option<PathBuf> {
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .filter(|p| p.is_absolute())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Owner, 2026-11-03: in Warp a session opens as a tab in the window
    /// already open, through a Tab Config and a `warp://` link, with no
    /// macOS prompt. The config runs the command in the folder, and a
    /// folder or command holding quotes, backslashes or newlines stays
    /// one TOML string.
    #[test]
    fn a_warp_launch_is_a_tab_config_that_runs_the_command_in_the_folder() {
        let cwd = Path::new("/Users/a/my \"odd\" folder\\x\n");
        let command = "'/x/near-ai' --config-dir '/c' redeem-launch --ticket 'ab'";
        let tab = WarpTab::new(command, Some(cwd)).unwrap();
        assert!(tab.name.starts_with("near-ai-"));
        assert!(
            tab.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        );
        assert_eq!(tab.url(), format!("warp://tab_config/{}", tab.name));
        assert_ne!(
            tab.name,
            WarpTab::new(command, Some(cwd)).unwrap().name,
            "one file per launch"
        );

        let parsed: toml::Value = toml::from_str(&tab.contents).unwrap();
        assert_eq!(parsed["name"].as_str(), Some(tab.name.as_str()));
        let pane = &parsed["panes"][0];
        assert_eq!(pane["type"].as_str(), Some("terminal"));
        assert_eq!(pane["directory"].as_str(), cwd.to_str());
        assert_eq!(pane["commands"][0].as_str(), Some(command));
        assert_eq!(pane["commands"].as_array().unwrap().len(), 1);

        let home =
            toml::from_str::<toml::Value>(&WarpTab::new(command, None).unwrap().contents).unwrap();
        assert!(
            home["panes"][0].get("directory").is_none(),
            "Warp's own folder"
        );
    }

    /// A Warp launch hands `run` the Tab Config, never a script or `open`
    /// argv, and opens no handle: Warp's tab cannot be found again.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_warp_launch_runs_its_tab_config() {
        let cwd = Path::new("/tmp/project");
        let mut seen = None;
        let opened = launch_macos(
            TerminalSetting::Warp,
            &[TerminalApp::Warp],
            "'/x/near-ai'",
            vec!["/x/near-ai".into()],
            Some(cwd),
            |how| {
                seen = Some(how);
                Ok(Vec::new())
            },
        );
        assert_eq!(opened, Ok(None));
        let Some(MacosLaunch::Warp(tab)) = seen else {
            panic!("a Tab Config");
        };
        let parsed: toml::Value = toml::from_str(&tab.contents).unwrap();
        assert_eq!(
            parsed["panes"][0]["directory"].as_str(),
            Some("/tmp/project")
        );
        assert_eq!(
            parsed["panes"][0]["commands"][0].as_str(),
            Some("'/x/near-ai'")
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Warp,
                &[TerminalApp::Terminal],
                "'/x'",
                vec![],
                None,
                |_| Ok(vec![])
            ),
            Err(ManagedError::TerminalNotFound),
            "not installed"
        );
    }

    /// Rex (owner, 2026-11-03): macOS only, found by its bundle, and never
    /// scripted or started through `open --args`: its own CLI opens a tab.
    #[test]
    fn rex_is_offered_on_macos_only_and_driven_by_its_cli() {
        assert_eq!(TerminalApp::Rex.bundle_id(), Some("com.superlogical.rex"));
        assert!(TerminalApp::MACOS.contains(&TerminalApp::Rex));
        assert!(!TerminalApp::LINUX.contains(&TerminalApp::Rex));
        assert!(!TerminalApp::WINDOWS.contains(&TerminalApp::Rex));
        assert!(
            TerminalApp::Rex.script("'/x/near-ai'").is_empty(),
            "no Apple Event"
        );
        assert_eq!(open_plan(TerminalApp::Rex, None, vec!["/x".into()]), None);
        assert_eq!(terminal_args(TerminalApp::Rex, None), None);
        assert_eq!(TerminalApp::Rex.executable(), None);
    }

    /// A Rex launch hands `run` the tab's request, with the argv as the
    /// command and the folder, and keeps the tab its output names. Only a
    /// missing CLI stays a refusal once Rex is asked; anything else is
    /// `LaunchUnknown`.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_rex_launch_runs_its_tab_and_keeps_the_ids_it_printed() {
        let session = "session:01a1a1cb-0d4e-7c1a-9f00-3b2a1c0d9e8f";
        let window = "window:01a1a1cb-e32b-7000-8000-000000000001";
        let block = "block:01a1a1cb-e327-7000-8000-000000000002";
        let mut seen = None;
        let opened = launch_macos(
            TerminalSetting::Rex,
            &[TerminalApp::Rex],
            "'/x/near-ai'",
            vec!["/x/near-ai".into(), "--ticket".into(), "ab".into()],
            Some(Path::new("/tmp/project")),
            |how| {
                seen = Some(how);
                Ok(format!("{session}\n{window}\n{block}\n").into_bytes())
            },
        )
        .unwrap()
        .expect("a tab");
        assert_eq!(
            opened.handle,
            TerminalHandle::Rex {
                session: session.into(),
                window: window.into(),
                block: block.into(),
            }
        );
        let Some(MacosLaunch::Rex(tab)) = seen else {
            panic!("a Rex tab");
        };
        let request: serde_json::Value = serde_json::from_slice(&tab.request).unwrap();
        assert_eq!(
            request["layout"]["block"]["options"]["command"],
            serde_json::json!(["/x/near-ai", "--ticket", "ab"])
        );
        assert_eq!(request["layout"]["block"]["options"]["cwd"], "/tmp/project");
        let refused = |error: ManagedError| {
            launch_macos(
                TerminalSetting::Rex,
                &[TerminalApp::Rex],
                "'/x'",
                vec!["/x".into()],
                None,
                |_| Err(error),
            )
        };
        assert_eq!(
            refused(ManagedError::TerminalNotFound),
            Err(ManagedError::TerminalNotFound)
        );
        assert_eq!(
            refused(ManagedError::InvalidRequest),
            Err(ManagedError::LaunchUnknown)
        );
    }

    /// herdr's attach step (D4) opens the herdr program alone in the chosen
    /// terminal, in the folder: no helper and no ticket.
    #[test]
    fn a_program_opens_alone_in_the_terminal() {
        let cwd = Path::new("/home/a/project");
        let plan = program_plan(
            TerminalApp::Kitty,
            Path::new("/usr/bin/kitty"),
            cwd,
            vec!["/usr/bin/herdr".into()],
        )
        .unwrap();
        let mut expected = terminal_args(TerminalApp::Kitty, Some(cwd)).unwrap();
        expected.push("/usr/bin/herdr".into());
        assert_eq!(plan.args, expected);
        assert_eq!(
            program_plan(
                TerminalApp::Kitty,
                Path::new("kitty"),
                cwd,
                vec!["/usr/bin/herdr".into()]
            ),
            Err(ManagedError::InvalidRequest)
        );
    }

    #[test]
    fn warp_is_offered_on_macos_only_and_opened_by_its_bundle() {
        assert_eq!(TerminalApp::Warp.bundle_id(), Some("dev.warp.Warp-Stable"));
        assert!(TerminalApp::MACOS.contains(&TerminalApp::Warp));
        assert!(!TerminalApp::LINUX.contains(&TerminalApp::Warp));
        assert!(!TerminalApp::WINDOWS.contains(&TerminalApp::Warp));
        assert!(
            TerminalApp::Warp.script("'/x/near-ai'").is_empty(),
            "no Apple Event"
        );
        assert_eq!(open_plan(TerminalApp::Warp, None, vec!["/x".into()]), None);
    }

    /// Every launch opens a window of its own (owner, 2026-11-03): Terminal
    /// runs the plain window script, with the command once and never typed
    /// into an existing tab, behind the same refusals as every other app.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_terminal_launch_opens_a_window_of_its_own() {
        let tab = TerminalTab {
            handle: TerminalHandle::AppleTerminal {
                tty: "/dev/ttys012".into(),
            },
            window: Some(4242),
        };
        let argv = || vec![std::ffi::OsString::from("/x/near-ai")];
        let cwd = Some(Path::new("/Users/x/app"));
        let mut ran = None;
        assert_eq!(
            launch_macos(
                TerminalSetting::Terminal,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                argv(),
                cwd,
                |how| {
                    ran = Some(how);
                    Ok(b"/dev/ttys012\n4242\n".to_vec())
                },
            ),
            Ok(Some(tab.clone()))
        );
        assert_eq!(
            ran,
            Some(MacosLaunch::Script(
                TerminalApp::Terminal.script("'/x/near-ai'")
            ))
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Terminal,
                &[TerminalApp::Ghostty],
                "'/x/near-ai'",
                argv(),
                cwd,
                |_| panic!("nothing runs when Terminal is missing"),
            ),
            Err(ManagedError::TerminalNotFound)
        );
        let mut ran = None;
        launch_macos(
            TerminalSetting::Ghostty,
            &[TerminalApp::Ghostty],
            "'/x/near-ai'",
            argv(),
            cwd,
            |how| {
                ran = Some(how);
                Ok(vec![])
            },
        )
        .unwrap();
        assert_eq!(
            ran,
            Some(MacosLaunch::Script(
                TerminalApp::Ghostty.script("'/x/near-ai'")
            ))
        );
    }

    /// Every Terminal script here compiles. Compiling sends no Apple Event;
    /// nothing here addresses Terminal.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_scripts_compile_on_this_mac() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("compiled.scpt");
        let compile = |language: &str, lines: &[String]| {
            let mut command = std::process::Command::new("/usr/bin/osacompile");
            command.args(["-l", language, "-o"]).arg(&out);
            for line in lines {
                command.args(["-e", line]);
            }
            let output = command.output().unwrap();
            assert!(
                output.status.success(),
                "{}: {lines:#?}",
                String::from_utf8_lossy(&output.stderr)
            );
        };
        for lines in [
            TerminalApp::Terminal.script("echo ok"),
            focus_script("/dev/ttys011"),
        ] {
            compile("AppleScript", &lines);
        }
    }

    /// Show scans the windows for its tab's window, by id, stopping at the
    /// match; a launch scans nothing and reads only the front window's id,
    /// when its selected tab is the launch's. Nothing types into or opens a
    /// tab in an existing window (owner, 2026-11-02). Built on every
    /// platform, so no builder is compiled for nothing.
    #[test]
    fn show_scans_and_a_launch_reads_only_the_front_window() {
        let focus = focus_script("/dev/ttys011");
        assert_eq!(
            focus.first().unwrap(),
            "tell application id \"com.apple.Terminal\""
        );
        assert!(focus.contains(&"set wid to id of w".to_string()));
        assert!(focus.contains(&"set tw to window id wid".to_string()));
        assert!(focus.contains(&"if tty of x is \"/dev/ttys011\" then".to_string()));
        assert!(focus.contains(&"if t is missing value then error \"no tab\"".to_string()));
        let launch = TerminalApp::Terminal.script("echo ok");
        for lines in [&focus, &launch] {
            assert!(
                !lines.iter().any(|l| l.contains("keystroke")
                    || l.contains("System Events")
                    || l.contains("\" in t")),
                "{lines:#?}"
            );
        }
        assert_eq!(
            launch,
            [
                vec!["tell application id \"com.apple.Terminal\"".to_string()],
                open_window("echo ok"),
                vec!["return found".into(), "end tell".into()],
            ]
            .concat()
        );
        assert!(
            !launch
                .iter()
                .any(|l| l.contains("repeat with w in windows")),
            "no scan of the windows"
        );
    }

    /// A launch prints its tab's tty and, on the next line, its window's
    /// id; anything else names no tab.
    #[test]
    fn a_launch_prints_its_tab_and_window() {
        let tab = |tty: &str| TerminalHandle::AppleTerminal { tty: tty.into() };
        assert_eq!(
            terminal_tab(b"/dev/ttys003\n4242\n"),
            Some(TerminalTab {
                handle: tab("/dev/ttys003"),
                window: Some(4242)
            })
        );
        assert_eq!(
            terminal_tab(b"/dev/ttys003\n"),
            Some(TerminalTab {
                handle: tab("/dev/ttys003"),
                window: None
            })
        );
        // Anything else is not what the script prints: no tab.
        for output in [
            &b""[..],
            b"/dev/ttys003\n\n",
            b"/dev/ttys003\n42 x",
            b"/dev/ttys003\n/dev/ttys004",
            b"/dev/ttys003\n42\n43",
            b"4242\n/dev/ttys003",
            b"ttys003\n42",
            b"/dev/ttys003\n99999999999",
            b"/dev/ttys003\n+42",
        ] {
            assert_eq!(terminal_tab(output), None, "{output:?}");
        }
    }

    #[test]
    fn setup_is_one_quoted_shell_invocation_and_keeps_the_terminal_open() {
        let command = setup_shell_command("printf '%s\\n' 'quoted $HOME `not-code`'").unwrap();
        assert!(command.starts_with("/bin/bash -lc '"));
        assert!(command.contains("$HOME/.local/bin:$HOME/.opencode/bin:$PATH"));
        assert!(command.contains("exec \"${SHELL:-/bin/zsh}\" -l"));
        for app in [
            TerminalApp::Terminal,
            TerminalApp::Ghostty,
            TerminalApp::Iterm2,
        ] {
            let script = app.script(&command);
            assert_eq!(
                script
                    .iter()
                    .filter(|line| line.contains(&applescript_literal(&command)))
                    .count(),
                1
            );
        }
        assert!(setup_shell_command("").is_err());
        assert!(setup_shell_command("bad\0command").is_err());
    }

    #[cfg(unix)]
    /// A config folder whose path tries to close the AppleScript string
    /// and run its own script, and to escape the shell quoting too, and
    /// carries `$`, backticks and non-ASCII text.
    fn hostile_command() -> String {
        shell_command(
            Path::new("/Applications/Near.ai Staging.app/Contents/MacOS/near-ai"),
            Path::new(
                "/Users/x/a\"; do shell script \"touch /tmp/pwn\" --\\\n'b/$HOME/`touch pwn`/año/日本語",
            ),
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
    fn the_choice_parses_only_this_platforms_apps() {
        for app in TerminalApp::offered() {
            assert_eq!(TerminalApp::parse(app.id()), Some(*app));
        }
        for app in TerminalApp::ALL {
            if !TerminalApp::offered().contains(&app) {
                assert_eq!(TerminalApp::parse(app.id()), None, "{app:?}");
            }
        }
        for refused in [
            "",
            "Terminal",
            "iTerm2",
            "Windows Terminal",
            "iterm",
            "/Applications/Ghostty.app",
            "/usr/bin/konsole",
            "ghostty; rm -rf /",
            " terminal",
        ] {
            assert_eq!(TerminalApp::parse(refused), None, "{refused:?}");
        }
        // Each platform's list, as the docs record it.
        assert_eq!(
            TerminalApp::MACOS.map(TerminalApp::name),
            [
                "Terminal",
                "Ghostty",
                "iTerm2",
                "kitty",
                "WezTerm",
                "Alacritty",
                "Warp",
                "Rex"
            ]
        );
        assert_eq!(
            TerminalApp::WINDOWS.map(TerminalApp::name),
            ["Windows Terminal"]
        );
        assert_eq!(
            TerminalApp::LINUX.map(TerminalApp::name),
            [
                "GNOME Terminal",
                "Konsole",
                "Ghostty",
                "kitty",
                "WezTerm",
                "Alacritty"
            ]
        );
    }

    #[test]
    fn an_unknown_stored_choice_is_unrecognized_never_the_default() {
        let read = |v: &str| serde_json::from_value::<TerminalSetting>(serde_json::json!(v));
        for app in TerminalApp::ALL {
            let setting = read(app.id()).unwrap();
            assert_eq!(setting, TerminalSetting::from(app));
            // Another platform's app reads as no app here.
            assert_eq!(
                setting.app(),
                TerminalApp::offered().contains(&app).then_some(app),
                "{app:?}"
            );
        }
        let unknown = read("/bin/sh").unwrap();
        assert_eq!(unknown, TerminalSetting::Unrecognized);
        assert_eq!(unknown.app(), None);
        // A save keeps it unrecognized rather than writing the default.
        let saved = serde_json::to_value(unknown).unwrap();
        assert_eq!(
            serde_json::from_value::<TerminalSetting>(saved).unwrap(),
            TerminalSetting::Unrecognized
        );
        // The default is what shipped before the choice, and is offered.
        let default = TerminalSetting::default().app();
        if cfg!(target_os = "macos") {
            assert_eq!(default, Some(TerminalApp::Terminal));
        } else if cfg!(windows) {
            assert_eq!(default, Some(TerminalApp::WindowsTerminal));
        } else if cfg!(target_os = "linux") {
            assert_eq!(default, Some(TerminalApp::GnomeTerminal));
        }
    }

    // The fixture paths are Unix paths, absolute only off Windows.
    #[cfg(unix)]
    #[test]
    fn every_script_holds_the_command_only_as_one_string_literal() {
        let command = hostile_command();
        // kitty, WezTerm and Alacritty take argv, never a script.
        for app in [
            TerminalApp::Terminal,
            TerminalApp::Ghostty,
            TerminalApp::Iterm2,
        ] {
            let lines = app.script(&command);
            let holding: Vec<&String> = lines
                .iter()
                .filter(|line| line.contains("near-ai"))
                .collect();
            assert_eq!(holding.len(), 1, "{app:?}: {lines:?}");
            let line = holding[0];
            // The fixed template, then one literal running to the line's end.
            let fixed = match app {
                TerminalApp::Terminal => "set t to do script ",
                TerminalApp::Ghostty => "set command of sessionConfiguration to ",
                TerminalApp::Iterm2 => "create window with default profile command ",
                other => unreachable!("{other:?} is not a macOS app"),
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

    /// The shell sees the hostile folder as one single-quoted word: a `'`
    /// closes and reopens the quoting, and `$`, backticks and non-ASCII
    /// text stay literal inside it.
    #[cfg(unix)]
    #[test]
    fn the_shell_command_single_quotes_the_whole_folder() {
        let command = hostile_command();
        let folder = r#"'/Users/x/a"; do shell script "touch /tmp/pwn" --\
'\''b/$HOME/`touch pwn`/año/日本語'"#;
        assert!(
            command.contains(&format!("--config-dir {folder} ")),
            "{command}"
        );
    }

    #[test]
    fn each_app_has_its_own_fixed_template() {
        let lines = |app: TerminalApp| app.script("'/x/near-ai' redeem-launch");
        assert_eq!(
            lines(TerminalApp::Terminal),
            [
                "tell application id \"com.apple.Terminal\"",
                "set t to do script \"'/x/near-ai' redeem-launch\"",
                "activate",
                "set found to \"\"",
                "try",
                "set found to tty of t",
                "end try",
                // The window id, from the front window only when its
                // selected tab is this one (#1853).
                "try",
                "if found is not \"\" and tty of selected tab of front window is found then",
                "set found to found & linefeed & ((id of front window) as text)",
                "end if",
                "end try",
                "return found",
                "end tell",
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

    #[cfg(target_os = "macos")]
    #[test]
    fn a_chosen_app_that_is_not_installed_is_refused_without_running() {
        let never = |_: MacosLaunch| -> Result<Vec<u8>, ManagedError> {
            panic!("nothing runs when the chosen app is missing")
        };
        let argv = || vec![std::ffi::OsString::from("/x/near-ai")];
        let cwd = Some(Path::new("/Users/x/app"));
        assert_eq!(
            launch_macos(
                TerminalSetting::Ghostty,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                argv(),
                cwd,
                never
            ),
            Err(ManagedError::TerminalNotFound)
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Iterm2,
                &[],
                "'/x/near-ai'",
                argv(),
                cwd,
                never
            ),
            Err(ManagedError::TerminalNotFound)
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Kitty,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                argv(),
                cwd,
                never
            ),
            Err(ManagedError::TerminalNotFound)
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Unrecognized,
                &TerminalApp::MACOS,
                "'/x/near-ai'",
                argv(),
                cwd,
                never
            ),
            Err(ManagedError::InvalidRequest)
        );
        let mut ran = None;
        launch_macos(
            TerminalSetting::Ghostty,
            &[TerminalApp::Ghostty],
            "'/x/near-ai'",
            argv(),
            cwd,
            |how| {
                ran = Some(how);
                // Whatever another app printed is never read as a tab.
                Ok(b"/dev/ttys001\n".to_vec())
            },
        )
        .unwrap();
        assert_eq!(
            ran,
            Some(MacosLaunch::Script(
                TerminalApp::Ghostty.script("'/x/near-ai'")
            ))
        );
        let mut ran = None;
        assert_eq!(
            launch_macos(
                TerminalSetting::Wezterm,
                &[TerminalApp::Wezterm],
                "'/x/near-ai'",
                argv(),
                cwd,
                |how| {
                    ran = Some(how);
                    Ok(b"/dev/ttys001\n".to_vec())
                },
            ),
            Ok(None)
        );
        assert_eq!(
            ran,
            Some(MacosLaunch::Open(
                open_plan(TerminalApp::Wezterm, cwd, argv()).unwrap()
            ))
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Terminal,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                argv(),
                cwd,
                |_| Ok(b"/dev/ttys001\n".to_vec())
            ),
            Ok(Some(TerminalTab {
                handle: TerminalHandle::AppleTerminal {
                    tty: "/dev/ttys001".into()
                },
                window: None
            }))
        );
        assert_eq!(
            launch_macos(
                TerminalSetting::Terminal,
                &[TerminalApp::Terminal],
                "'/x/near-ai'",
                argv(),
                cwd,
                |_| Ok(b"not a tty".to_vec())
            ),
            Ok(None)
        );
    }

    /// #1853 review: once the app is asked, any error reads as
    /// `LaunchUnknown`, so the daemon keeps the launch held; only the
    /// refusals from before anything runs are passed through.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_error_once_the_terminal_is_asked_is_launch_unknown() {
        let argv = || vec![std::ffi::OsString::from("/x/near-ai")];
        let cwd = Some(Path::new("/Users/x/app"));
        for error in [
            ManagedError::StorageUnavailable,
            ManagedError::UnsupportedVersion,
            ManagedError::TerminalNotFound,
        ] {
            for (choice, app) in [
                (TerminalSetting::Terminal, TerminalApp::Terminal),
                (TerminalSetting::Wezterm, TerminalApp::Wezterm),
            ] {
                assert_eq!(
                    launch_macos(choice, &[app], "'/x/near-ai'", argv(), cwd, |_| Err(error)),
                    Err(ManagedError::LaunchUnknown),
                    "{error:?}"
                );
            }
        }
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
        for app in TerminalApp::MACOS {
            assert!(probe.contains(app.bundle_id().unwrap()));
        }
        for app in TerminalApp::WINDOWS.into_iter().chain(TerminalApp::LINUX) {
            if !TerminalApp::MACOS.contains(&app) {
                assert_eq!(app.bundle_id(), None, "{app:?}");
                assert!(app.script("'/x/near-ai'").is_empty(), "{app:?}");
            }
        }

        // kitty, WezTerm and Alacritty take argv on macOS too: no script.
        for app in [
            TerminalApp::Kitty,
            TerminalApp::Wezterm,
            TerminalApp::Alacritty,
        ] {
            assert!(app.script("'/x/near-ai'").is_empty(), "{app:?}");
        }
    }

    /// The probe runs once per freshness window, however many snapshots
    /// ask; a launch whose app the cached answer lacks asks again, so an
    /// app installed a moment ago is not refused for the window's length.
    #[test]
    fn the_installed_apps_are_probed_once_per_window() {
        use std::cell::Cell;
        use std::time::{Duration, Instant};
        let probes = Cell::new(0);
        let answer = std::cell::RefCell::new(vec![TerminalApp::Terminal]);
        let probe = || {
            probes.set(probes.get() + 1);
            answer.borrow().clone()
        };
        let mut cache = InstalledCache::default();
        let start = Instant::now();
        assert_eq!(cache.get(start, None, probe), [TerminalApp::Terminal]);
        assert_eq!(
            cache.get(start + Duration::from_secs(5), None, probe),
            [TerminalApp::Terminal]
        );
        assert_eq!(probes.get(), 1, "a fresh answer is reused");
        // Terminal is in the answer: a launch in it does not ask again.
        cache.get(start, Some(TerminalApp::Terminal), probe);
        assert_eq!(probes.get(), 1);
        // Ghostty was installed since: a launch in it asks once more.
        *answer.borrow_mut() = vec![TerminalApp::Terminal, TerminalApp::Ghostty];
        assert_eq!(
            cache.get(start, Some(TerminalApp::Ghostty), probe),
            [TerminalApp::Terminal, TerminalApp::Ghostty]
        );
        assert_eq!(probes.get(), 2);
        // Past the window, a snapshot asks again.
        cache.get(
            start + InstalledCache::FRESH + Duration::from_secs(1),
            None,
            probe,
        );
        assert_eq!(probes.get(), 3);
    }

    #[cfg(unix)]
    /// Folders a launch must pass through unchanged: spaces, both quotes,
    /// `$`, backticks, a command substitution, a line break and non-ASCII.
    fn hostile_folders() -> Vec<PathBuf> {
        [
            "/home/x/My Projects/app",
            "/home/x/it's \"quoted\"",
            "/home/x/$HOME/$(touch pwn)",
            "/home/x/`touch pwn`",
            "/home/x/a\nb",
            "/home/x/proyecto-año/日本語",
            "/home/x/-e",
        ]
        .into_iter()
        .map(PathBuf::from)
        .collect()
    }

    #[cfg(unix)]
    fn plan_for(app: TerminalApp, cwd: &Path) -> Result<Plan, ManagedError> {
        plan(
            app,
            Path::new("/usr/bin/term"),
            Path::new("/opt/Near ai/near-ai"),
            Path::new("/home/x/.config/near ai"),
            cwd,
            Uuid::nil(),
            "abcdef01",
        )
    }

    #[cfg(unix)]
    /// The helper and its arguments, as the tail every plan ends with.
    fn helper_tail() -> Vec<std::ffi::OsString> {
        let mut tail = vec![std::ffi::OsString::from("/opt/Near ai/near-ai")];
        tail.extend(helper_args(
            Path::new("/home/x/.config/near ai"),
            Uuid::nil(),
            "abcdef01",
        ));
        tail
    }

    // The fixture paths are Unix paths, absolute only off Windows.
    #[cfg(unix)]
    #[test]
    fn each_terminal_gets_the_folder_and_the_helper_as_argv() {
        let cwd = Path::new("/home/x/My Projects/app");
        let head = |app| {
            let plan = plan_for(app, cwd).unwrap();
            assert_eq!(plan.program, Path::new("/usr/bin/term"));
            let tail = helper_tail();
            assert!(plan.args.ends_with(&tail), "{app:?}: {:?}", plan.args);
            plan.args[..plan.args.len() - tail.len()]
                .iter()
                .map(|a| a.to_str().unwrap().to_owned())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            head(TerminalApp::GnomeTerminal),
            ["--working-directory=/home/x/My Projects/app", "--"]
        );
        assert_eq!(
            head(TerminalApp::Konsole),
            ["--workdir", "/home/x/My Projects/app", "--hold", "-e"]
        );
        assert_eq!(
            head(TerminalApp::Ghostty),
            [
                "--working-directory=/home/x/My Projects/app",
                "--wait-after-command=true",
                "-e"
            ]
        );
        assert_eq!(
            head(TerminalApp::WindowsTerminal),
            [
                "new-tab",
                "--title",
                "NEAR AI",
                "-d",
                "/home/x/My Projects/app"
            ]
        );
        assert_eq!(
            head(TerminalApp::Kitty),
            ["--directory", "/home/x/My Projects/app", "--hold"]
        );
        assert_eq!(
            head(TerminalApp::Wezterm),
            [
                "--config",
                "exit_behavior=\"Hold\"",
                "start",
                "--always-new-process",
                "--cwd",
                "/home/x/My Projects/app",
                "--"
            ]
        );
        assert_eq!(
            head(TerminalApp::Alacritty),
            [
                "--working-directory",
                "/home/x/My Projects/app",
                "--hold",
                "-e"
            ]
        );
        // Terminal and iTerm2 are opened by script, never by argv.
        for app in [TerminalApp::Terminal, TerminalApp::Iterm2] {
            assert_eq!(plan_for(app, cwd), Err(ManagedError::InvalidRequest));
        }
    }

    // The fixture paths are Unix paths, absolute only off Windows.
    #[cfg(unix)]
    #[test]
    fn a_hostile_folder_is_one_verbatim_argument() {
        for cwd in hostile_folders() {
            for app in [
                TerminalApp::GnomeTerminal,
                TerminalApp::Konsole,
                TerminalApp::Ghostty,
                TerminalApp::WindowsTerminal,
                TerminalApp::Kitty,
                TerminalApp::Wezterm,
                TerminalApp::Alacritty,
            ] {
                let plan = plan_for(app, &cwd).unwrap();
                let tail = helper_tail();
                assert!(plan.args.ends_with(&tail), "{app:?} {cwd:?}");
                let head = &plan.args[..plan.args.len() - tail.len()];
                // The folder appears exactly once, whole, as the value of
                // the working-directory flag: never split, never quoted.
                let holding: Vec<_> = head
                    .iter()
                    .filter(|a| a.to_str().unwrap().contains("/home/x/"))
                    .collect();
                assert_eq!(holding.len(), 1, "{app:?} {cwd:?}: {head:?}");
                let held = holding[0].to_str().unwrap();
                let value = held.strip_prefix("--working-directory=").unwrap_or(held);
                assert_eq!(Path::new(value), cwd, "{app:?}");
            }
        }
    }

    // The fixture paths are Unix paths, absolute only off Windows.
    #[cfg(unix)]
    #[test]
    fn a_semicolon_is_refused_for_windows_terminal_only() {
        let cwd = Path::new("/home/x/a;b");
        assert_eq!(
            plan_for(TerminalApp::WindowsTerminal, cwd),
            Err(ManagedError::InvalidRequest)
        );
        let config_with = plan(
            TerminalApp::WindowsTerminal,
            Path::new("/usr/bin/wt.exe"),
            Path::new("/opt/near-ai"),
            Path::new("/home/x/c;fg"),
            Path::new("/home/x"),
            Uuid::nil(),
            "abcdef01",
        );
        assert_eq!(config_with, Err(ManagedError::InvalidRequest));
        for app in [
            TerminalApp::GnomeTerminal,
            TerminalApp::Konsole,
            TerminalApp::Ghostty,
        ] {
            assert!(plan_for(app, cwd).is_ok(), "{app:?}");
        }
    }

    // The fixture paths are Unix paths, absolute only off Windows.
    #[cfg(unix)]
    #[test]
    fn a_plan_refuses_relative_paths_and_a_non_hex_ticket() {
        let ok = |program: &str, helper: &str, config: &str, cwd: &str, ticket: &str| {
            plan(
                TerminalApp::Konsole,
                Path::new(program),
                Path::new(helper),
                Path::new(config),
                Path::new(cwd),
                Uuid::nil(),
                ticket,
            )
            .is_ok()
        };
        assert!(ok(
            "/usr/bin/konsole",
            "/opt/near-ai",
            "/c",
            "/w",
            "abcdef01"
        ));
        assert!(!ok("konsole", "/opt/near-ai", "/c", "/w", "abcdef01"));
        assert!(!ok("/usr/bin/konsole", "near-ai", "/c", "/w", "abcdef01"));
        assert!(!ok(
            "/usr/bin/konsole",
            "/opt/near-ai",
            "c",
            "/w",
            "abcdef01"
        ));
        assert!(!ok(
            "/usr/bin/konsole",
            "/opt/near-ai",
            "/c",
            "w",
            "abcdef01"
        ));
        assert!(!ok("/usr/bin/konsole", "/opt/near-ai", "/c", "/w", ""));
        assert!(!ok("/usr/bin/konsole", "/opt/near-ai", "/c", "/w", "a; rm"));
    }

    /// A folder that is not UTF-8 still reaches a Linux terminal byte for
    /// byte: argv carries it as an `OsString`, never via a `String`.
    #[cfg(unix)]
    #[test]
    fn a_non_utf8_folder_reaches_the_terminal_unchanged() {
        use std::os::unix::ffi::OsStrExt;
        let cwd = Path::new(std::ffi::OsStr::from_bytes(b"/home/x/\xff\xfe dir"));
        let plan = plan_for(TerminalApp::Konsole, cwd).unwrap();
        assert_eq!(plan.args[1], cwd.as_os_str());
    }

    #[test]
    fn the_programs_are_fixed_names() {
        assert_eq!(TerminalApp::WindowsTerminal.executable(), Some("wt.exe"));
        assert_eq!(
            TerminalApp::GnomeTerminal.executable(),
            Some("gnome-terminal")
        );
        assert_eq!(TerminalApp::Konsole.executable(), Some("konsole"));
        assert_eq!(TerminalApp::Ghostty.executable(), Some("ghostty"));
        assert_eq!(TerminalApp::Kitty.executable(), Some("kitty"));
        assert_eq!(TerminalApp::Wezterm.executable(), Some("wezterm"));
        assert_eq!(TerminalApp::Alacritty.executable(), Some("alacritty"));
        assert_eq!(TerminalApp::Terminal.executable(), None);
        assert_eq!(TerminalApp::Iterm2.executable(), None);
    }

    /// On macOS, kitty, WezTerm and Alacritty are opened with `open -n -b
    /// <bundle id> --args`, then the same terminal arguments as elsewhere
    /// and the command, each one argv element. The scripted apps get no
    /// plan.
    #[test]
    fn macos_argv_apps_are_opened_by_bundle_id_with_the_command_as_argv() {
        let cwd = Path::new("/Users/x/it's \"quoted\"/$HOME");
        let command: Vec<std::ffi::OsString> = vec![
            "/Applications/Near.ai Staging.app/near-ai".into(),
            "--ticket".into(),
            "ab".into(),
        ];
        for (app, id) in [
            (TerminalApp::Kitty, "net.kovidgoyal.kitty"),
            (TerminalApp::Wezterm, "com.github.wez.wezterm"),
            (TerminalApp::Alacritty, "org.alacritty"),
        ] {
            let plan = open_plan(app, Some(cwd), command.clone()).unwrap();
            assert_eq!(plan.program, Path::new("/usr/bin/open"));
            let mut expected: Vec<std::ffi::OsString> =
                vec!["-n".into(), "-b".into(), id.into(), "--args".into()];
            expected.extend(terminal_args(app, Some(cwd)).unwrap());
            expected.extend(command.clone());
            assert_eq!(plan.args, expected, "{app:?}");
            assert_eq!(
                plan.args
                    .iter()
                    .filter(|a| a.as_os_str() == cwd.as_os_str())
                    .count(),
                1,
                "{app:?}: the folder is one whole argument"
            );
        }
        for app in [
            TerminalApp::Terminal,
            TerminalApp::Ghostty,
            TerminalApp::Iterm2,
        ] {
            assert_eq!(open_plan(app, Some(cwd), command.clone()), None, "{app:?}");
        }
        // Tool setup names no folder: no working-directory flag at all.
        let plan = open_plan(TerminalApp::Alacritty, None, setup_argv("true").unwrap()).unwrap();
        assert_eq!(
            plan.args[4..6].to_vec(),
            vec![std::ffi::OsString::from("--hold"), "-e".into()]
        );
        assert_eq!(plan.args[6], "/bin/bash");
        assert_eq!(plan.args[7], "-lc");
    }

    /// The real probe on a Mac: Terminal ships with macOS, so LaunchServices
    /// always names it.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_probe_finds_terminal_on_this_mac() {
        assert!(installed().contains(&TerminalApp::Terminal));
    }
}
