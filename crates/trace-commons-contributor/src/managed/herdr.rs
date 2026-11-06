//! Grouping managed sessions in herdr, a terminal multiplexer that runs
//! inside the person's own terminal (owner decisions D3-D6, 2026-11-03;
//! `docs/superpowers/specs/2026-11-03-rex-herdr-terminal-integration.md`).
//!
//! With "Group sessions in herdr" on, a launch opens the helper as a new tab
//! in the herdr workspace labelled [`GROUP_LABEL`], through one
//! `layout.apply` request on herdr's owner-only socket. The pane gets the
//! helper's argv (no shell) and the folder; the launch ticket goes in the
//! pane's environment ([`super::terminal::LAUNCH_TICKET_ENV`]), never in
//! argv, because herdr saves each pane's argv to disk and replays it after a
//! restart. When no herdr client is running, the chosen terminal opens
//! `herdr` (nothing secret) and the launch waits for its socket.
//!
//! herdr is a separate program the daemon talks to; no herdr code is linked.
//! Only unix sockets are spoken here: herdr's Windows named pipe is left for
//! when the Windows shell leaves its freeze (#1173 D13).
// Off unix only the refusals in `launch` and `focus` run, so the protocol
// below is compiled there but never called.
#![cfg_attr(not(unix), allow(dead_code))]
use super::ManagedError;
use super::terminal::{TerminalHandle, TerminalTab};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

/// The label of the herdr workspace every session opens in. Found by label,
/// and created again when the person has closed it.
pub const GROUP_LABEL: &str = crate::brand::APP_NAME;

/// herdr's own name, which shells put in `{destination}` for a grouped
/// launch.
pub const NAME: &str = "herdr";

/// The oldest herdr socket protocol this build speaks: the one its
/// `layout.apply` pane `env` and `workspace.create` replies were read from
/// (herdr 0.9.5). An older herdr is refused before the ticket is sent.
const MIN_PROTOCOL: u64 = 22;

/// How long a launch waits for a herdr it started to answer, counted on the
/// clock from the first try, so slow tries do not stretch it.
const ATTACH_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
const ATTACH_STEP: std::time::Duration = std::time::Duration::from_millis(200);

/// The largest reply read from herdr. A snapshot of a busy server is a few
/// hundred KiB; anything past this is refused, not buffered.
#[cfg(unix)]
const REPLY_LIMIT: u64 = 8 * 1024 * 1024;

/// Held while a launch finds or creates the group workspace and opens its
/// tab in it.
#[cfg(unix)]
static GROUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// What went wrong talking to herdr.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CallError {
    /// No socket, or no answer on it.
    Unreachable,
    /// Something is at herdr's socket path that this account cannot trust:
    /// another owner, open to others, or not a socket. Refused rather than
    /// attached around, since starting herdr would not replace it.
    Untrusted,
    /// herdr answered with an error.
    Refused,
    /// herdr answered with something this build cannot read.
    Malformed,
}

/// One request and its reply's `result`. A trait so tests answer for herdr.
pub(crate) trait Exchange {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError>;
}

/// herdr's terminal ids: `term_` and lowercase hex.
pub fn valid_terminal_id(text: &str) -> bool {
    text.strip_prefix("term_").is_some_and(|rest| {
        (1..=32).contains(&rest.len())
            && rest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// herdr's public workspace, tab and pane ids (`w1`, `w1:t2`, ...): short,
/// and only letters, digits, `:`, `_` and `-`. Only ever sent back to herdr,
/// as a JSON string.
fn valid_public_id(text: &str) -> bool {
    (1..=64).contains(&text.len())
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'_' | b'-'))
}

/// The `herdr` program: on the daemon's `PATH`, or where its installers put
/// it (Homebrew, `install.sh`). A daemon started by launchd has a short
/// `PATH`, so the fixed folders are checked too.
pub fn executable() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        let mut folders: Vec<PathBuf> =
            std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()).collect();
        folders.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
        if let Some(home) = dirs::home_dir() {
            folders.push(home.join(".local").join("bin"));
        }
        folders
            .into_iter()
            .filter(|folder| folder.is_absolute())
            .map(|folder| folder.join("herdr"))
            .find(|path| path.is_file())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// herdr's default socket: `$XDG_CONFIG_HOME/herdr/herdr.sock`, else
/// `~/.config/herdr/herdr.sock`, on Linux and macOS alike. Named herdr
/// sessions (`HERDR_SOCKET_PATH`, `sessions/<name>/`) are not used.
fn socket_path() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| dirs::home_dir().map(|home| home.join(".config")))?;
    Some(config.join("herdr").join("herdr.sock"))
}

/// What is at herdr's socket path.
#[cfg(unix)]
#[derive(Debug, PartialEq, Eq)]
enum SocketState {
    Absent,
    Trusted,
    Untrusted,
}

#[cfg(unix)]
fn socket_state(path: &Path) -> SocketState {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => SocketState::Absent,
        Err(_) => SocketState::Untrusted,
        Ok(_) if trusted_socket(path) => SocketState::Trusted,
        Ok(_) => SocketState::Untrusted,
    }
}

/// herdr's socket, when it is a socket this account owns that nobody else
/// can open. herdr creates it 0600; anything looser is not herdr's.
#[cfg(unix)]
fn trusted_socket(path: &Path) -> bool {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let Ok(metadata) = std::fs::symlink_metadata(path) else {
        return false;
    };
    metadata.file_type().is_socket()
        && Some(metadata.uid()) == crate::daemon::ironwire_pointer::current_uid()
        && metadata.mode() & 0o077 == 0
}

/// A connection to herdr's socket.
#[cfg(unix)]
struct Socket {
    reader: std::io::BufReader<std::os::unix::net::UnixStream>,
    next: u64,
}

#[cfg(unix)]
impl Socket {
    fn connect() -> Result<Self, CallError> {
        let path = socket_path().ok_or(CallError::Unreachable)?;
        match socket_state(&path) {
            SocketState::Absent => return Err(CallError::Unreachable),
            SocketState::Untrusted => return Err(CallError::Untrusted),
            SocketState::Trusted => {}
        }
        let stream =
            std::os::unix::net::UnixStream::connect(&path).map_err(|_| CallError::Unreachable)?;
        let timeout = Some(std::time::Duration::from_secs(5));
        stream
            .set_read_timeout(timeout)
            .and_then(|()| stream.set_write_timeout(timeout))
            .map_err(|_| CallError::Unreachable)?;
        Ok(Socket {
            reader: std::io::BufReader::new(stream),
            next: 1,
        })
    }
}

#[cfg(unix)]
impl Exchange for Socket {
    fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        use std::io::{BufRead, Read, Write};
        let id = format!("near-ai-{}", self.next);
        self.next += 1;
        let mut line = json!({"id": id, "method": method, "params": params}).to_string();
        line.push('\n');
        self.reader
            .get_mut()
            .write_all(line.as_bytes())
            .map_err(|_| CallError::Unreachable)?;
        let mut reply = Vec::new();
        (&mut self.reader)
            .take(REPLY_LIMIT)
            .read_until(b'\n', &mut reply)
            .map_err(|_| CallError::Unreachable)?;
        if reply.last() != Some(&b'\n') {
            return Err(CallError::Malformed);
        }
        result_of(&reply, &id)
    }
}

/// The `result` of herdr's reply to request `id`.
fn result_of(reply: &[u8], id: &str) -> Result<Value, CallError> {
    let reply: Value = serde_json::from_slice(reply).map_err(|_| CallError::Malformed)?;
    if reply.get("id").and_then(Value::as_str) != Some(id) {
        return Err(CallError::Malformed);
    }
    if reply.get("error").is_some() {
        return Err(CallError::Refused);
    }
    reply.get("result").cloned().ok_or(CallError::Malformed)
}

/// `ping`, answered by a `pong` from a herdr that speaks at least
/// [`MIN_PROTOCOL`].
fn ping(herdr: &mut dyn Exchange) -> Result<(), CallError> {
    let pong = herdr.call("ping", json!({}))?;
    let protocol = pong.get("protocol").and_then(Value::as_u64);
    if pong.get("type").and_then(Value::as_str) == Some("pong")
        && protocol.is_some_and(|p| p >= MIN_PROTOCOL)
    {
        Ok(())
    } else {
        Err(CallError::Malformed)
    }
}

/// `session.snapshot`'s snapshot.
fn snapshot(herdr: &mut dyn Exchange) -> Result<Value, CallError> {
    let result = herdr.call("session.snapshot", json!({}))?;
    if result.get("type").and_then(Value::as_str) != Some("session_snapshot") {
        return Err(CallError::Malformed);
    }
    result.get("snapshot").cloned().ok_or(CallError::Malformed)
}

/// The id of the first workspace labelled [`GROUP_LABEL`].
fn group_workspace(snapshot: &Value) -> Option<String> {
    snapshot
        .get("workspaces")?
        .as_array()?
        .iter()
        .filter(|w| w.get("label").and_then(Value::as_str) == Some(GROUP_LABEL))
        .filter_map(|w| w.get("workspace_id").and_then(Value::as_str))
        .find(|id| valid_public_id(id))
        .map(str::to_string)
}

/// The terminal id of the pane `pane_id`.
fn terminal_of_pane(snapshot: &Value, pane_id: &str) -> Option<String> {
    snapshot
        .get("panes")?
        .as_array()?
        .iter()
        .find(|p| p.get("pane_id").and_then(Value::as_str) == Some(pane_id))?
        .get("terminal_id")
        .and_then(Value::as_str)
        .filter(|id| valid_terminal_id(id))
        .map(str::to_string)
}

/// The tab that holds terminal `terminal_id` now.
fn tab_of_terminal(snapshot: &Value, terminal_id: &str) -> Option<String> {
    snapshot
        .get("panes")?
        .as_array()?
        .iter()
        .find(|p| p.get("terminal_id").and_then(Value::as_str) == Some(terminal_id))?
        .get("tab_id")
        .and_then(Value::as_str)
        .filter(|id| valid_public_id(id))
        .map(str::to_string)
}

/// The pane a launch opens: the helper's argv (no ticket), its folder, the
/// ticket for its environment, and the tab's label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pane {
    pub argv: Vec<String>,
    pub cwd: String,
    pub ticket: String,
    pub label: String,
}

impl Pane {
    /// The pane for `helper` with `args` in `cwd`. Refuses a relative or
    /// non-UTF-8 path and a ticket that is not hex.
    pub fn new(
        helper: &Path,
        args: Vec<std::ffi::OsString>,
        cwd: &Path,
        ticket: &str,
    ) -> Result<Self, ManagedError> {
        if !helper.is_absolute() || !cwd.is_absolute() || !super::terminal::valid_ticket(ticket) {
            return Err(ManagedError::InvalidRequest);
        }
        let text = |s: &std::ffi::OsStr| s.to_str().map(str::to_string);
        let mut argv = vec![text(helper.as_os_str()).ok_or(ManagedError::InvalidRequest)?];
        for arg in &args {
            argv.push(text(arg).ok_or(ManagedError::InvalidRequest)?);
        }
        Ok(Pane {
            argv,
            cwd: text(cwd.as_os_str()).ok_or(ManagedError::InvalidRequest)?,
            ticket: ticket.to_string(),
            label: tab_label(cwd),
        })
    }

    /// The `layout.apply` pane node.
    fn node(&self) -> Value {
        json!({
            "type": "pane",
            "command": self.argv,
            "cwd": self.cwd,
            "env": {super::terminal::LAUNCH_TICKET_ENV: self.ticket},
            "label": self.label,
        })
    }
}

/// A tab's label: the folder's name, without control characters, at most 48
/// characters; [`GROUP_LABEL`] for a folder with no usable name.
pub(crate) fn tab_label(cwd: &Path) -> String {
    let name: String = cwd
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default()
        .chars()
        .filter(|c| !c.is_control())
        .take(48)
        .collect();
    if name.trim().is_empty() {
        GROUP_LABEL.to_string()
    } else {
        name
    }
}

/// Where `layout.apply` puts the new tab: added to the group workspace, or
/// in place of the shell tab a workspace this launch created starts with.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Target {
    Workspace(String),
    ReplaceTab(String),
}

fn apply_params(target: &Target, pane: &Pane) -> Value {
    let mut params = json!({
        "tab_label": pane.label,
        "focus": true,
        "root": pane.node(),
    });
    match target {
        Target::Workspace(id) => params["workspace_id"] = json!(id),
        Target::ReplaceTab(id) => params["tab_id"] = json!(id),
    }
    params
}

/// A herdr that answers `ping`.
fn reach<E: Exchange>(connect: &mut impl FnMut() -> Result<E, CallError>) -> Result<E, CallError> {
    let mut herdr = connect()?;
    ping(&mut herdr)?;
    Ok(herdr)
}

/// Open `pane` as a tab in the group workspace. `connect` reaches herdr;
/// `attach` starts a herdr client in the chosen terminal when none answers;
/// `elapsed` reads a clock and `sleep` waits between tries, so the wait for
/// herdr ends [`ATTACH_WAIT`] after the terminal opened, however long each
/// try took. While waiting, every error is retried; the last one decides
/// the refusal.
///
/// Only the `layout.apply` request carries the ticket. Anything that fails
/// before it is sent is a refusal that releases the held launch
/// (`TerminalNotFound` or `UnsupportedVersion`); anything after it may have
/// started the helper, and reads as `LaunchUnknown`. A tab whose terminal id
/// cannot be read back is still a launch, with no handle.
pub(crate) fn launch_with<E: Exchange>(
    mut connect: impl FnMut() -> Result<E, CallError>,
    attach: impl FnOnce() -> Result<(), ManagedError>,
    mut elapsed: impl FnMut() -> std::time::Duration,
    mut sleep: impl FnMut(std::time::Duration),
    pane: &Pane,
) -> Result<Option<TerminalTab>, ManagedError> {
    let refusal = |error: CallError| match error {
        CallError::Unreachable => ManagedError::TerminalNotFound,
        CallError::Untrusted => ManagedError::HerdrSocketUntrusted,
        CallError::Refused | CallError::Malformed => ManagedError::UnsupportedVersion,
    };
    let mut herdr = match reach(&mut connect) {
        Ok(herdr) => herdr,
        Err(CallError::Unreachable) => {
            attach().map_err(|_| ManagedError::TerminalNotFound)?;
            // Counted from when the terminal has opened, and every error
            // retried: a herdr that is starting can briefly show its socket
            // before it is 0600, or close before a whole line.
            let began = elapsed();
            loop {
                let error = match reach(&mut connect) {
                    Ok(herdr) => break herdr,
                    Err(error) => error,
                };
                if elapsed().saturating_sub(began) >= ATTACH_WAIT {
                    return Err(refusal(error));
                }
                sleep(ATTACH_STEP);
            }
        }
        Err(error) => return Err(refusal(error)),
    };
    let before = snapshot(&mut herdr).map_err(|_| ManagedError::UnsupportedVersion)?;
    let target = match group_workspace(&before) {
        Some(id) => Target::Workspace(id),
        None => {
            let created = herdr
                .call(
                    "workspace.create",
                    json!({"label": GROUP_LABEL, "cwd": pane.cwd, "focus": true}),
                )
                .map_err(|_| ManagedError::UnsupportedVersion)?;
            let tab = created
                .get("tab")
                .and_then(|tab| tab.get("tab_id"))
                .and_then(Value::as_str)
                .filter(|id| valid_public_id(id))
                .ok_or(ManagedError::UnsupportedVersion)?;
            Target::ReplaceTab(tab.to_string())
        }
    };
    let applied = herdr
        .call("layout.apply", apply_params(&target, pane))
        .map_err(|_| ManagedError::LaunchUnknown)?;
    let pane_id = applied
        .get("layout")
        .and_then(|layout| layout.get("focused_pane_id"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let terminal = pane_id.and_then(|pane_id| {
        let after = snapshot(&mut herdr).ok()?;
        terminal_of_pane(&after, &pane_id)
    });
    Ok(terminal.map(|terminal| TerminalTab {
        handle: TerminalHandle::Herdr { terminal },
        window: None,
    }))
}

/// Open the session as a tab in herdr's group workspace, starting a herdr
/// client in `host` (the chosen terminal) when none is running.
pub fn launch(
    host: super::terminal::TerminalSetting,
    installed: &[super::terminal::TerminalApp],
    config: &Path,
    cwd: &Path,
    session: uuid::Uuid,
    ticket: &str,
) -> Result<Option<TerminalTab>, ManagedError> {
    #[cfg(unix)]
    {
        if !config.is_absolute() {
            return Err(ManagedError::InvalidRequest);
        }
        let program = executable().ok_or(ManagedError::TerminalNotFound)?;
        let pane = Pane::new(
            &super::terminal::helper_path()?,
            super::terminal::helper_args_without_ticket(config, session),
            cwd,
            ticket,
        )?;
        // One launch at a time finds or creates the group, so two at once
        // cannot each create a "Near AI Gateway" workspace, or each start a herdr.
        let _group = GROUP_LOCK
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let began = std::time::Instant::now();
        launch_with(
            Socket::connect,
            || super::terminal::open_program(host, installed, &program, cwd),
            || began.elapsed(),
            std::thread::sleep,
            &pane,
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (host, installed, config, cwd, session, ticket);
        Err(ManagedError::TerminalNotFound)
    }
}

/// Switch herdr's attached client to the tab holding `terminal`. herdr's own
/// window is the person's terminal, which this cannot raise without asking
/// macOS for Automation, so it is left where it is (D6).
pub(crate) fn focus_with<E: Exchange>(
    connect: impl FnOnce() -> Result<E, CallError>,
    terminal: &str,
) -> Result<(), ManagedError> {
    if !valid_terminal_id(terminal) {
        return Err(ManagedError::InvalidRequest);
    }
    let mut herdr = connect().map_err(|_| ManagedError::NotFound)?;
    let now = snapshot(&mut herdr).map_err(|_| ManagedError::NotFound)?;
    let tab = tab_of_terminal(&now, terminal).ok_or(ManagedError::NotFound)?;
    herdr
        .call("tab.focus", json!({"tab_id": tab}))
        .map_err(|_| ManagedError::NotFound)?;
    Ok(())
}

pub fn focus(terminal: &str) -> Result<(), ManagedError> {
    #[cfg(unix)]
    {
        focus_with(Socket::connect, terminal)
    }
    #[cfg(not(unix))]
    {
        let _ = terminal;
        Err(ManagedError::UnsupportedVersion)
    }
}

/// Whether "Group sessions in herdr" can be offered here: herdr is found,
/// and this platform speaks its socket.
pub fn available() -> bool {
    cfg!(unix) && executable().is_some()
}

// herdr is spoken only on unix, and these tests use unix paths.
#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::collections::VecDeque;

    /// A herdr that answers from a script and records every request.
    #[derive(Default)]
    struct Script {
        replies: VecDeque<Result<Value, CallError>>,
        seen: std::rc::Rc<std::cell::RefCell<Vec<(String, Value)>>>,
    }

    impl Exchange for Script {
        fn call(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
            self.seen.borrow_mut().push((method.to_string(), params));
            self.replies
                .pop_front()
                .unwrap_or(Err(CallError::Unreachable))
        }
    }

    fn pong() -> Result<Value, CallError> {
        Ok(json!({"type":"pong","version":"0.9.5","protocol":22}))
    }

    fn snap(workspaces: Value, panes: Value) -> Result<Value, CallError> {
        Ok(json!({"type":"session_snapshot","snapshot":{
            "version":"0.9.5","protocol":22,"workspaces":workspaces,"tabs":[],
            "panes":panes,"layouts":[],"agents":[]}}))
    }

    fn pane() -> Pane {
        Pane::new(
            Path::new("/Applications/Near.app/Contents/Resources/near-ai"),
            vec![
                "--config-dir".into(),
                "/Users/a/cfg".into(),
                "redeem-launch".into(),
                "--session-id".into(),
                "8d9b2a4e-5b0a-4f7e-9d43-3c1b8b3b6a11".into(),
            ],
            Path::new("/Users/a/my project"),
            "abc123",
        )
        .unwrap()
    }

    fn applied() -> Result<Value, CallError> {
        Ok(json!({"type":"layout_apply","layout":{
            "workspace_id":"w2","tab_id":"w2:t3","zoomed":false,
            "focused_pane_id":"w2:t3:p1","root":{"type":"pane"}}}))
    }

    /// D3: the ticket reaches herdr only as the pane's environment, never in
    /// its argv, which herdr saves to disk and replays after a restart. The
    /// pane runs the helper's argv in the folder, as a focused tab of the
    /// existing "Near AI Gateway" workspace, and the handle is the pane's terminal.
    #[test]
    fn a_launch_adds_a_tab_to_the_group_with_the_ticket_only_in_env() {
        let seen = std::rc::Rc::default();
        let script = Script {
            replies: VecDeque::from([
                pong(),
                snap(
                    json!([{"workspace_id":"w1","label":"other"},
                           {"workspace_id":"w2","label":"Near AI Gateway"}]),
                    json!([]),
                ),
                applied(),
                snap(
                    json!([]),
                    json!([{"pane_id":"w2:t3:p1","terminal_id":"term_19af3","tab_id":"w2:t3"}]),
                ),
            ]),
            seen: std::rc::Rc::clone(&seen),
        };
        let mut script = Some(script);
        let tab = launch_with(
            || script.take().ok_or(CallError::Unreachable),
            || panic!("herdr was running"),
            || std::time::Duration::ZERO,
            |_| {},
            &pane(),
        )
        .unwrap();
        assert_eq!(
            tab,
            Some(TerminalTab {
                handle: TerminalHandle::Herdr {
                    terminal: "term_19af3".into()
                },
                window: None,
            })
        );
        let seen = seen.borrow();
        let methods: Vec<&str> = seen.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(
            methods,
            [
                "ping",
                "session.snapshot",
                "layout.apply",
                "session.snapshot"
            ]
        );
        let params = &seen[2].1;
        assert_eq!(params["workspace_id"], "w2");
        assert!(params.get("tab_id").is_none());
        assert_eq!(params["focus"], true);
        assert_eq!(params["tab_label"], "my project");
        let root = &params["root"];
        assert_eq!(root["type"], "pane");
        assert_eq!(root["cwd"], "/Users/a/my project");
        assert_eq!(root["env"], json!({"NEAR_AI_LAUNCH_TICKET": "abc123"}));
        let argv: Vec<&str> = root["command"]
            .as_array()
            .unwrap()
            .iter()
            .map(|a| a.as_str().unwrap())
            .collect();
        assert_eq!(argv[0], "/Applications/Near.app/Contents/Resources/near-ai");
        assert!(argv.contains(&"redeem-launch"));
        assert!(!argv.iter().any(|a| a.contains("abc123")), "{argv:?}");
        assert!(!argv.contains(&"--ticket"));
        for (method, params) in seen.iter() {
            if method != "layout.apply" {
                assert!(!params.to_string().contains("abc123"), "{method}");
            }
        }
    }

    /// D5: with no "Near AI Gateway" workspace (never made, or closed), one is
    /// created, and the session's tab replaces the shell tab it starts with,
    /// so the group holds no stray shell.
    #[test]
    fn a_missing_group_is_created_and_its_first_tab_replaced() {
        let seen = std::rc::Rc::default();
        let mut script = Some(Script {
            replies: VecDeque::from([
                pong(),
                snap(
                    json!([{"workspace_id":"w1","label":"Near AI Gateway2"}]),
                    json!([]),
                ),
                Ok(json!({"type":"workspace_created",
                    "workspace":{"workspace_id":"w4","label":"Near AI Gateway"},
                    "tab":{"tab_id":"w4:t1"},"root_pane":{"pane_id":"w4:t1:p1"}})),
                applied(),
                snap(json!([]), json!([])),
            ]),
            seen: std::rc::Rc::clone(&seen),
        });
        let tab = launch_with(
            || script.take().ok_or(CallError::Unreachable),
            || panic!("running"),
            || std::time::Duration::ZERO,
            |_| {},
            &pane(),
        );
        assert_eq!(tab, Ok(None), "a tab with no terminal id has no handle");
        let seen = seen.borrow();
        assert_eq!(seen[2].0, "workspace.create");
        assert_eq!(
            seen[2].1,
            json!({"label":"Near AI Gateway","cwd":"/Users/a/my project","focus":true})
        );
        assert_eq!(seen[3].0, "layout.apply");
        assert_eq!(seen[3].1["tab_id"], "w4:t1");
        assert!(seen[3].1.get("workspace_id").is_none(), "one or the other");
    }

    /// D4: with no herdr answering, the chosen terminal opens herdr, and the
    /// launch waits for its socket; one that never answers is refused before
    /// the ticket was sent, so the launch can be tried again.
    #[test]
    fn an_absent_herdr_is_attached_in_the_chosen_terminal_and_waited_for() {
        let mut tries = 0;
        let mut attached = false;
        let tab = launch_with(
            || {
                tries += 1;
                if tries < 4 {
                    return Err(CallError::Unreachable);
                }
                Ok(Script {
                    replies: VecDeque::from([
                        pong(),
                        snap(
                            json!([{"workspace_id":"w1","label":"Near AI Gateway"}]),
                            json!([]),
                        ),
                        applied(),
                        snap(json!([]), json!([])),
                    ]),
                    ..Default::default()
                })
            },
            || {
                attached = true;
                Ok(())
            },
            || std::time::Duration::ZERO,
            |_| {},
            &pane(),
        );
        assert_eq!(tab, Ok(None));
        assert!(attached);
        assert_eq!(tries, 4);

        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let never = launch_with(
            || Err::<Script, _>(CallError::Unreachable),
            || Ok(()),
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(never, Err(ManagedError::TerminalNotFound));
        assert_eq!(clock.get(), ATTACH_WAIT);

        let no_host = launch_with(
            || Err::<Script, _>(CallError::Unreachable),
            || Err(ManagedError::TerminalNotFound),
            || panic!("no wait without a terminal"),
            |_| panic!("no wait without a terminal"),
            &pane(),
        );
        assert_eq!(no_host, Err(ManagedError::TerminalNotFound));
    }

    /// The wait ends on the clock, counting the time each try took (a try
    /// can sit on a socket timeout), not only the sleeps between them.
    #[test]
    fn the_wait_for_herdr_is_bounded_by_the_clock() {
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let mut tries = 0;
        let never = launch_with(
            || {
                tries += 1;
                clock.set(clock.get() + std::time::Duration::from_secs(4));
                Err::<Script, _>(CallError::Unreachable)
            },
            || Ok(()),
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(never, Err(ManagedError::TerminalNotFound));
        // The first try comes before herdr is started; the wait counts from
        // the start, so three slow tries follow it.
        assert_eq!(tries, 4);
    }

    /// A herdr this launch started can briefly look untrusted (its socket
    /// before it is set 0600) or answer half a line while it starts, so
    /// every error is retried until the wait ends (poldsam, #1886).
    #[test]
    fn a_starting_herdr_is_retried_through_any_error() {
        let mut tries = 0;
        let tab = launch_with(
            || {
                tries += 1;
                match tries {
                    1 => Err(CallError::Unreachable),
                    2 => Err(CallError::Untrusted),
                    3 => Ok(Script {
                        replies: VecDeque::from([Err(CallError::Malformed)]),
                        ..Default::default()
                    }),
                    _ => Ok(Script {
                        replies: VecDeque::from([
                            pong(),
                            snap(
                                json!([{"workspace_id":"w1","label":"Near AI Gateway"}]),
                                json!([]),
                            ),
                            applied(),
                            snap(json!([]), json!([])),
                        ]),
                        ..Default::default()
                    }),
                }
            },
            || Ok(()),
            || std::time::Duration::ZERO,
            |_| {},
            &pane(),
        );
        assert_eq!(tab, Ok(None));
        assert_eq!(tries, 4);

        // One that never reads right is refused by its last error.
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let old = launch_with(
            || {
                let mut script = Script::default();
                script.replies.push_back(Err(CallError::Malformed));
                if clock.get().is_zero() {
                    Err(CallError::Unreachable)
                } else {
                    Ok(script)
                }
            },
            || Ok(()),
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(old, Err(ManagedError::UnsupportedVersion));
    }

    /// A herdr this launch started whose socket is still untrusted when the
    /// wait ends is refused with the untrusted line, not "not found".
    #[test]
    fn a_socket_still_untrusted_after_the_wait_has_its_own_refusal() {
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let first = std::cell::Cell::new(true);
        let refused = launch_with(
            || {
                if first.replace(false) {
                    Err::<Script, _>(CallError::Unreachable)
                } else {
                    Err(CallError::Untrusted)
                }
            },
            || Ok(()),
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(refused, Err(ManagedError::HerdrSocketUntrusted));
    }

    /// The wait for herdr starts once the host terminal has opened, so a
    /// slow terminal does not use up herdr's time (poldsam, #1886).
    #[test]
    fn the_wait_for_herdr_starts_after_the_terminal_opens() {
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let attached_at = std::cell::Cell::new(std::time::Duration::ZERO);
        let never = launch_with(
            || Err::<Script, _>(CallError::Unreachable),
            || {
                clock.set(clock.get() + std::time::Duration::from_secs(9));
                attached_at.set(clock.get());
                Ok(())
            },
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(never, Err(ManagedError::TerminalNotFound));
        assert_eq!(clock.get() - attached_at.get(), ATTACH_WAIT);
    }

    /// A herdr too old to read, or a socket path held by something this
    /// account cannot trust, is refused before anything is attached or the
    /// ticket is sent; neither is waited around.
    #[test]
    fn an_old_herdr_or_an_untrusted_socket_is_refused() {
        let old = |protocol: Value| {
            let mut script = Some(Script {
                replies: VecDeque::from([Ok(
                    json!({"type":"pong","version":"0.8.0","protocol":protocol}),
                )]),
                ..Default::default()
            });
            launch_with(
                || script.take().ok_or(CallError::Unreachable),
                || panic!("herdr was running"),
                || std::time::Duration::ZERO,
                |_| {},
                &pane(),
            )
        };
        assert_eq!(old(json!(21)), Err(ManagedError::UnsupportedVersion));
        assert_eq!(old(Value::Null), Err(ManagedError::UnsupportedVersion));

        let untrusted = launch_with(
            || Err::<Script, _>(CallError::Untrusted),
            || panic!("not attached around"),
            || std::time::Duration::ZERO,
            |_| {},
            &pane(),
        );
        // Its own line (owner, 2026-11-03): "herdr's connection file can't be
        // trusted. Remove it or check its permissions."
        assert_eq!(untrusted, Err(ManagedError::HerdrSocketUntrusted));

        // An old herdr this launch started is retried like any herdr that
        // is starting, then refused for its version when the wait ends.
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let mut tries = 0;
        let became_old = launch_with(
            || {
                tries += 1;
                if tries == 1 {
                    return Err(CallError::Unreachable);
                }
                Ok(Script {
                    replies: VecDeque::from([Ok(json!({"type":"pong","protocol":3}))]),
                    ..Default::default()
                })
            },
            || Ok(()),
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &pane(),
        );
        assert_eq!(became_old, Err(ManagedError::UnsupportedVersion));
        assert_eq!(clock.get(), ATTACH_WAIT);
        assert!(tries > 2);
    }

    /// A refusal from `layout.apply`, or no answer to it, may follow a
    /// started helper: `LaunchUnknown`, which keeps the launch held. A
    /// failure before it releases the launch.
    #[test]
    fn only_a_failure_after_the_ticket_is_sent_is_launch_unknown() {
        let run = |replies: Vec<Result<Value, CallError>>| {
            let mut script = Some(Script {
                replies: replies.into(),
                ..Default::default()
            });
            launch_with(
                || script.take().ok_or(CallError::Unreachable),
                || Err(ManagedError::TerminalNotFound),
                || std::time::Duration::ZERO,
                |_| {},
                &pane(),
            )
        };
        let group = || {
            snap(
                json!([{"workspace_id":"w1","label":"Near AI Gateway"}]),
                json!([]),
            )
        };
        assert_eq!(
            run(vec![pong(), group(), Err(CallError::Refused)]),
            Err(ManagedError::LaunchUnknown)
        );
        assert_eq!(
            run(vec![pong(), group(), Err(CallError::Unreachable)]),
            Err(ManagedError::LaunchUnknown)
        );
        assert_eq!(
            run(vec![pong(), Err(CallError::Refused)]),
            Err(ManagedError::UnsupportedVersion)
        );
        assert_eq!(
            run(vec![Ok(json!({"type":"ok"}))]),
            Err(ManagedError::UnsupportedVersion),
            "not herdr"
        );
        assert_eq!(
            run(vec![
                pong(),
                snap(json!([]), json!([])),
                Ok(json!({"type":"workspace_created","tab":{"tab_id":"w1:t1\"}"}})),
            ]),
            Err(ManagedError::UnsupportedVersion),
            "an id that is not one is never sent back"
        );
    }

    /// Show re-reads where the terminal is now, since herdr's tab ids move
    /// with the tab, and switches to that tab; a closed tab is `NotFound`.
    #[test]
    fn show_focuses_the_tab_that_holds_the_terminal_now() {
        let seen = std::rc::Rc::default();
        let script = Script {
            replies: VecDeque::from([
                snap(
                    json!([]),
                    json!([{"pane_id":"w1:t1:p1","terminal_id":"term_aa","tab_id":"w1:t1"},
                           {"pane_id":"w2:t5:p2","terminal_id":"term_19af3","tab_id":"w2:t5"}]),
                ),
                Ok(json!({"type":"tab_info","tab":{"tab_id":"w2:t5"}})),
            ]),
            seen: std::rc::Rc::clone(&seen),
        };
        assert_eq!(focus_with(|| Ok(script), "term_19af3"), Ok(()));
        assert_eq!(
            seen.borrow()[1],
            ("tab.focus".into(), json!({"tab_id":"w2:t5"}))
        );

        let gone = Script {
            replies: VecDeque::from([snap(json!([]), json!([]))]),
            ..Default::default()
        };
        assert_eq!(
            focus_with(|| Ok(gone), "term_19af3"),
            Err(ManagedError::NotFound)
        );
        assert_eq!(
            focus_with(|| Err::<Script, _>(CallError::Unreachable), "term_19af3"),
            Err(ManagedError::NotFound),
            "herdr has stopped"
        );
        assert_eq!(
            focus_with(|| Ok(Script::default()), "term_\"x"),
            Err(ManagedError::InvalidRequest)
        );
    }

    #[test]
    fn ids_are_checked_before_they_are_kept_or_sent() {
        assert!(valid_terminal_id("term_19af3"));
        for bad in ["term_", "term_XYZ", "term_1 2", "19af3", "term_19af3\n", ""] {
            assert!(!valid_terminal_id(bad), "{bad:?}");
        }
        assert!(valid_public_id("w2:t3"));
        assert!(!valid_public_id("w2:t3\""));
        assert!(!valid_public_id(""));
    }

    #[test]
    fn a_pane_refuses_what_it_cannot_send_unchanged() {
        let helper = Path::new("/x/near-ai");
        let cwd = Path::new("/x");
        assert_eq!(
            Pane::new(Path::new("near-ai"), vec![], cwd, "ab").err(),
            Some(ManagedError::InvalidRequest)
        );
        assert_eq!(
            Pane::new(helper, vec![], Path::new("x"), "ab").err(),
            Some(ManagedError::InvalidRequest)
        );
        assert_eq!(
            Pane::new(helper, vec![], cwd, "not hex").err(),
            Some(ManagedError::InvalidRequest)
        );
        #[cfg(unix)]
        {
            use std::os::unix::ffi::OsStrExt;
            let odd = std::ffi::OsStr::from_bytes(b"/x/\xff");
            assert_eq!(
                Pane::new(helper, vec![], Path::new(odd), "ab").err(),
                Some(ManagedError::InvalidRequest),
                "JSON cannot carry it unchanged"
            );
        }
        assert_eq!(tab_label(Path::new("/")), GROUP_LABEL);
        assert_eq!(tab_label(Path::new("/a/b\nc")), "bc");
        assert_eq!(tab_label(&Path::new("/a").join("x".repeat(80))).len(), 48);
    }

    #[test]
    fn a_reply_must_answer_this_request() {
        assert_eq!(
            result_of(br#"{"id":"near-ai-1","result":{"type":"ok"}}"#, "near-ai-1"),
            Ok(json!({"type":"ok"}))
        );
        assert_eq!(
            result_of(br#"{"id":"near-ai-2","result":{}}"#, "near-ai-1"),
            Err(CallError::Malformed)
        );
        assert_eq!(
            result_of(
                br#"{"id":"near-ai-1","error":{"code":"x","message":"y"}}"#,
                "near-ai-1"
            ),
            Err(CallError::Refused)
        );
        assert_eq!(
            result_of(b"not json", "near-ai-1"),
            Err(CallError::Malformed)
        );
    }

    /// The socket is herdr's only when it is a socket this account owns that
    /// nobody else can open.
    #[cfg(unix)]
    #[test]
    fn only_an_owner_only_socket_is_trusted() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herdr.sock");
        let _listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(trusted_socket(&path));
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).unwrap();
        assert!(!trusted_socket(&path));
        let file = dir.path().join("plain");
        std::fs::write(&file, b"").unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert!(!trusted_socket(&file), "not a socket");
        let link = dir.path().join("link.sock");
        std::os::unix::fs::symlink(&path, &link).unwrap();
        assert!(!trusted_socket(&link), "a link is not followed");
        assert_eq!(socket_state(&path), SocketState::Untrusted, "0666");
        assert_eq!(socket_state(&dir.path().join("none")), SocketState::Absent);
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        assert_eq!(socket_state(&path), SocketState::Trusted);
    }

    /// One request and reply over a real unix socket, NDJSON as herdr
    /// speaks it, read by [`Socket`] and checked by id.
    #[cfg(unix)]
    #[test]
    fn the_socket_speaks_one_json_line_each_way() {
        use std::io::{BufRead, Write};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("herdr.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = std::io::BufReader::new(stream);
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            let request: Value = serde_json::from_str(&line).unwrap();
            let reply =
                json!({"id": request["id"], "result": {"type":"pong","version":"t","protocol":22}});
            let mut stream = reader.into_inner();
            stream.write_all(format!("{reply}\n").as_bytes()).unwrap();
            request
        });
        let stream = std::os::unix::net::UnixStream::connect(&path).unwrap();
        let mut socket = Socket {
            reader: std::io::BufReader::new(stream),
            next: 7,
        };
        assert_eq!(ping(&mut socket), Ok(()));
        let request = server.join().unwrap();
        assert_eq!(
            request,
            json!({"id":"near-ai-7","method":"ping","params":{}})
        );
    }
}
