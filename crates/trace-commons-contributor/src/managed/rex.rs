//! Opening managed sessions in Rex, Superlogical's terminal with a built-in
//! multiplexer (macOS, private beta; owner decisions D1, D2, D5-D8,
//! 2026-11-03; `docs/superpowers/specs/2026-11-03-rex-herdr-terminal-integration.md`).
//!
//! Every session opens as a tab (a Rex "window") of one Rex session labelled
//! [`GROUP_LABEL`], through the `rex` CLI that ships inside the app bundle:
//! `rex api call -s <session> session.new_window -`, with the request on
//! stdin. The tab runs the helper's argv with no shell, in the folder. Rex
//! has no way to give a program an environment variable, so the ticket is in
//! argv, as for kitty, WezTerm and Alacritty (D2); the `rex` process itself
//! never sees it. Nothing here sends an Apple Event: the CLI talks to Rex's
//! owner-only socket, and the app is raised by LaunchServices (`open -b`).
//!
//! Rex's CLI is a beta ("0.0") with no published contract (D8). Only the
//! calls the research verified are used, and every reply is read as loosely
//! as it can be while each id that is kept is checked exactly.
#[cfg(any(target_os = "macos", test))]
use super::ManagedError;
#[cfg(any(target_os = "macos", test))]
use super::terminal::{TerminalHandle, TerminalTab};
#[cfg(any(target_os = "macos", test))]
use serde_json::{Value, json};
#[cfg(any(target_os = "macos", test))]
use std::path::Path;

/// Rex's bundle id, by which LaunchServices finds and opens it.
#[cfg(any(target_os = "macos", test))]
pub const BUNDLE_ID: &str = "com.superlogical.rex";

/// The label of the Rex session every managed session opens a tab in.
#[cfg(any(target_os = "macos", test))]
pub const GROUP_LABEL: &str = crate::brand::APP_NAME;

/// A Rex id as Rex prints it: `kind`, a colon, and a lowercase hyphenated
/// UUID (`session:01a1a1cb-...`). Nothing else is kept or passed to `rex`.
pub fn valid_id(kind: &str, text: &str) -> bool {
    text.strip_prefix(kind)
        .and_then(|rest| rest.strip_prefix(':'))
        .and_then(|uuid| {
            uuid::Uuid::parse_str(uuid)
                .ok()
                .map(|parsed| parsed.hyphenated().to_string() == uuid)
        })
        .unwrap_or(false)
}

/// A Rex launch: the `session.new_window` request, minus the session it
/// goes to, which is found or made when the launch runs.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RexTab {
    /// The request's JSON, sent on stdin.
    pub request: Vec<u8>,
    /// The folder a newly made group session starts in, if any.
    pub cwd: Option<std::path::PathBuf>,
}

#[cfg(any(target_os = "macos", test))]
impl RexTab {
    /// The tab that runs `argv` (the program first) in `cwd`, or in Rex's
    /// default folder when there is none. Refuses a non-UTF-8 argument or
    /// folder, which JSON cannot carry unchanged.
    pub fn new(argv: &[std::ffi::OsString], cwd: Option<&Path>) -> Result<Self, ManagedError> {
        let command = argv
            .iter()
            .map(|arg| arg.to_str().map(str::to_string))
            .collect::<Option<Vec<_>>>()
            .filter(|command| !command.is_empty())
            .ok_or(ManagedError::InvalidRequest)?;
        let mut options = json!({
            "shell": "none",
            "command": command,
            "exit": {"on_completion": false},
        });
        if let Some(cwd) = cwd {
            if !cwd.is_absolute() {
                return Err(ManagedError::InvalidRequest);
            }
            options["cwd"] = json!(cwd.to_str().ok_or(ManagedError::InvalidRequest)?);
        }
        let label = cwd
            .map(super::herdr::tab_label)
            .unwrap_or_else(|| GROUP_LABEL.to_string());
        let request = json!({
            "window_label": label,
            "focus": true,
            "layout": {"block": {
                "flavor": "com.superlogical.terminal.shell",
                "label": "near-ai",
                "options": options,
            }},
        });
        Ok(RexTab {
            request: serde_json::to_vec(&request).map_err(|_| ManagedError::InvalidRequest)?,
            cwd: cwd.map(Path::to_path_buf),
        })
    }
}

/// The group session in `rex ls --json`'s output: the session id held by
/// the first object that also holds the string [`GROUP_LABEL`]. Read by
/// value, not by field name, because the listing's shape is not published.
#[cfg(any(target_os = "macos", test))]
fn group_session(listing: &[u8]) -> Option<String> {
    fn walk(value: &Value) -> Option<String> {
        match value {
            Value::Object(fields) => {
                let own: Vec<&str> = fields.values().filter_map(Value::as_str).collect();
                if own.contains(&GROUP_LABEL) {
                    if let Some(id) = own.iter().find(|text| valid_id("session", text)) {
                        return Some(id.to_string());
                    }
                }
                fields.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    walk(&serde_json::from_slice(listing).ok()?)
}

/// The first `kind` id in `output`, JSON or not: `rex new --json` names the
/// session it made, and how it prints it is not published.
#[cfg(any(target_os = "macos", test))]
fn first_id(kind: &str, output: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(output).ok()?;
    let prefix = format!("{kind}:");
    text.match_indices(&prefix).find_map(|(at, _)| {
        let end = (at + prefix.len() + 36).min(text.len());
        let candidate = text.get(at..end)?;
        valid_id(kind, candidate).then(|| candidate.to_string())
    })
}

/// The window and block `session.new_window` answered with:
/// `{"window_id":"window:...","block_ids":["block:..."],...}`.
#[cfg(any(target_os = "macos", test))]
fn new_window_reply(output: &[u8]) -> Option<(String, String)> {
    let reply: Value = serde_json::from_slice(output).ok()?;
    let window = reply.get("window_id")?.as_str()?;
    let block = reply.get("block_ids")?.as_array()?.first()?.as_str()?;
    (valid_id("window", window) && valid_id("block", block))
        .then(|| (window.to_string(), block.to_string()))
}

/// What a launch prints for [`tab`]: the session, window and block ids,
/// one per line.
#[cfg(any(target_os = "macos", test))]
fn printed(session: &str, window: &str, block: &str) -> Vec<u8> {
    format!("{session}\n{window}\n{block}\n").into_bytes()
}

/// The tab a Rex launch printed, or none when any id is not one.
#[cfg(any(target_os = "macos", test))]
pub fn tab(output: &[u8]) -> Option<TerminalTab> {
    let text = std::str::from_utf8(output).ok()?;
    let mut lines = text.lines();
    let (session, window, block) = (lines.next()?, lines.next()?, lines.next()?);
    if lines.next().is_some()
        || !valid_id("session", session)
        || !valid_id("window", window)
        || !valid_id("block", block)
    {
        return None;
    }
    Some(TerminalTab {
        handle: TerminalHandle::Rex {
            session: session.into(),
            window: window.into(),
            block: block.into(),
        },
        window: None,
    })
}

/// How long a launch waits for Rex's server to answer after the app was
/// opened, counted on the clock from the launch's start.
#[cfg(any(target_os = "macos", test))]
const SERVER_WAIT: std::time::Duration = std::time::Duration::from_secs(10);
#[cfg(any(target_os = "macos", test))]
const SERVER_STEP: std::time::Duration = std::time::Duration::from_millis(200);

/// The Rex API revision the calls here were recorded against (`rex
/// version`: "api: revision 10 (supports 0 to 10)"). A Rex that no longer
/// supports it is refused before the ticket is sent.
#[cfg(any(target_os = "macos", test))]
const API_REVISION: u64 = 10;

/// Whether `rex version`'s output says this Rex supports [`API_REVISION`]:
/// its "supports A to B" range holds it.
#[cfg(any(target_os = "macos", test))]
fn supports_revision(output: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(output) else {
        return false;
    };
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("api:"))
        .filter_map(|api| {
            let range = api.split("(supports ").nth(1)?.split(')').next()?;
            let (low, high) = range.split_once(" to ")?;
            Some((
                low.trim().parse::<u64>().ok()?,
                high.trim().parse::<u64>().ok()?,
            ))
        })
        .any(|(low, high)| (low..=high).contains(&API_REVISION))
}

/// Whether `rex client ls --json`'s output lists a connected app: an object
/// whose `kind` is "app". The server can run headless (the CLI starts one
/// on its own), and a tab opened there is shown by nobody.
#[cfg(any(target_os = "macos", test))]
fn app_connected(clients: &[u8]) -> bool {
    fn walk(value: &Value) -> bool {
        match value {
            Value::Object(fields) => {
                fields.get("kind").and_then(Value::as_str) == Some("app")
                    || fields.values().any(walk)
            }
            Value::Array(items) => items.iter().any(walk),
            _ => false,
        }
    }
    serde_json::from_slice(clients).is_ok_and(|value: Value| walk(&value))
}

/// Held while a launch finds or makes the group session and opens its tab
/// in it, so two launches at once cannot each make a "Near AI Gateway" session.
#[cfg(target_os = "macos")]
static GROUP_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The `rex` CLI found last, checked again before each use. A miss is not
/// kept, so a Rex installed later is found.
#[cfg(target_os = "macos")]
static CLI: std::sync::Mutex<Option<std::path::PathBuf>> = std::sync::Mutex::new(None);

/// The `rex` CLI: inside the app bundle LaunchServices names, else the copy
/// the app keeps under Application Support. Never looked up on `PATH`.
#[cfg(target_os = "macos")]
fn cli() -> Option<std::path::PathBuf> {
    let mut cached = CLI
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if let Some(path) = cached.as_ref().filter(|path| path.is_file()) {
        return Some(path.clone());
    }
    let found = find_cli();
    cached.clone_from(&found);
    found
}

#[cfg(target_os = "macos")]
fn find_cli() -> Option<std::path::PathBuf> {
    let mut probe = std::process::Command::new("/usr/bin/osascript");
    probe.args([
        "-l",
        "JavaScript",
        "-e",
        &format!(
            "ObjC.import('AppKit'); var url = $.NSWorkspace.sharedWorkspace.URLForApplicationWithBundleIdentifier('{BUNDLE_ID}'); url.isNil() ? '' : ObjC.unwrap(url.path)"
        ),
    ]);
    let bundle = super::profiles::bounded_output(probe)
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .map(std::path::PathBuf::from)
        .filter(|path| path.is_absolute());
    let in_bundle =
        bundle.map(|bundle| bundle.join("Contents/Helpers/Rex Server.rex/Contents/MacOS/rex"));
    let kept = dirs::home_dir().map(|home| {
        home.join(
            "Library/Application Support/com.superlogical.rex/RexServer/Versions/Current/Rex Server.app/Contents/MacOS/rex",
        )
    });
    [in_bundle, kept]
        .into_iter()
        .flatten()
        .find(|path| path.is_file())
}

/// Run `rex` with `args` (and `input` on stdin), bounded like every other
/// helper process; its stdout when it succeeds, none when it fails. Never
/// with `--autostart` (on by default), which would start a headless server
/// the app does not show: the app starts its own.
#[cfg(target_os = "macos")]
fn rex(cli: &Path, args: &[&str], input: Option<&[u8]>) -> Option<Vec<u8>> {
    let mut command = std::process::Command::new(cli);
    command.arg("--autostart=false").args(args);
    let output = match input {
        Some(input) => super::profiles::bounded_output_with_input(command, input),
        None => super::profiles::bounded_output(command),
    }
    .ok()?;
    output.status.success().then_some(output.stdout)
}

/// Bring Rex.app forward through LaunchServices, starting it when it is not
/// running. No Apple Event, so macOS asks for no permission.
#[cfg(target_os = "macos")]
fn raise_app() -> bool {
    let mut open = std::process::Command::new("/usr/bin/open");
    open.args(["-b", BUNDLE_ID]);
    super::profiles::bounded_output(open).is_ok_and(|output| output.status.success())
}

/// Open `tab` in Rex with its outside effects passed in: `rex` runs the CLI
/// (stdout, or none on failure), `raise` opens the app, `elapsed` reads the
/// time since the launch began and `sleep` waits between tries.
///
/// The app is opened, and the launch waits for it to connect to its server
/// as a client, so the tab opens where it is shown.
///
/// Only `session.new_window` carries the ticket. A failure before it is
/// sent is a refusal that releases the held launch: `TerminalNotFound`
/// when the app does not open, `RexNotReady` when it never connects,
/// `TerminalCallFailed` when a `rex` call fails or gives nothing, and
/// `UnsupportedVersion` when Rex does not support the API revision used
/// here or answers in a way this build cannot read.
/// A failure of that request may follow a started helper: `LaunchUnknown`.
/// A tab whose ids cannot be read back is still a launch, printing nothing.
#[cfg(any(target_os = "macos", test))]
fn open_tab_with(
    mut rex: impl FnMut(&[&str], Option<&[u8]>) -> Option<Vec<u8>>,
    raise: impl FnOnce() -> bool,
    mut elapsed: impl FnMut() -> std::time::Duration,
    mut sleep: impl FnMut(std::time::Duration),
    tab: &RexTab,
) -> Result<Vec<u8>, ManagedError> {
    if !raise() {
        return Err(ManagedError::TerminalNotFound);
    }
    // The app may still be starting: wait for it to connect.
    while !rex(&["client", "ls", "--json"], None).is_some_and(|c| app_connected(&c)) {
        if elapsed() >= SERVER_WAIT {
            return Err(ManagedError::RexNotReady);
        }
        sleep(SERVER_STEP);
    }
    // A call that fails or gives nothing is `TerminalCallFailed`; only a
    // reply this build cannot read is an unsupported version.
    let version = rex(&["version"], None).ok_or(ManagedError::TerminalCallFailed)?;
    if !supports_revision(&version) {
        return Err(ManagedError::UnsupportedVersion);
    }
    let listing = rex(&["ls", "--json"], None).ok_or(ManagedError::TerminalCallFailed)?;
    let session = match group_session(&listing) {
        Some(session) => session,
        None => {
            let mut args = vec!["new", GROUP_LABEL, "--json"];
            let cwd = tab.cwd.as_deref().and_then(Path::to_str);
            if let Some(cwd) = cwd {
                args.extend(["-c", cwd]);
            }
            let made = rex(&args, None).ok_or(ManagedError::TerminalCallFailed)?;
            first_id("session", &made).ok_or(ManagedError::UnsupportedVersion)?
        }
    };
    let reply = rex(
        &["api", "call", "-s", &session, "session.new_window", "-"],
        Some(&tab.request),
    )
    .ok_or(ManagedError::LaunchUnknown)?;
    let Some((window, block)) = new_window_reply(&reply) else {
        // The tab may be open; it simply cannot be shown.
        return Ok(Vec::new());
    };
    let _ = rex(&["focus", "-s", &session, &block], None);
    Ok(printed(&session, &window, &block))
}

/// Open `tab` in Rex: start or raise the app, find the "Near AI Gateway" session or
/// make it, open the tab there and select it. Prints the ids for [`tab`].
#[cfg(target_os = "macos")]
pub fn open_tab(tab: &RexTab) -> Result<Vec<u8>, ManagedError> {
    let cli = cli().ok_or(ManagedError::TerminalNotFound)?;
    let _group = GROUP_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let began = std::time::Instant::now();
    open_tab_with(
        |args, input| rex(&cli, args, input),
        raise_app,
        || began.elapsed(),
        std::thread::sleep,
        tab,
    )
}

/// Show: select the block's tab in its session, then raise the app. A
/// block Rex no longer has (the tab was closed) is `NotFound`.
#[cfg(target_os = "macos")]
pub fn focus(session: &str, block: &str) -> Result<(), ManagedError> {
    if !valid_id("session", session) || !valid_id("block", block) {
        return Err(ManagedError::InvalidRequest);
    }
    let cli = cli().ok_or(ManagedError::NotFound)?;
    rex(&cli, &["focus", "-s", session, block], None).ok_or(ManagedError::NotFound)?;
    if raise_app() {
        Ok(())
    } else {
        Err(ManagedError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SESSION: &str = "session:01a1a1cb-0d4e-7c1a-9f00-3b2a1c0d9e8f";
    const WINDOW: &str = "window:01a1a1cb-e32b-7000-8000-000000000001";
    const BLOCK: &str = "block:01a1a1cb-e327-7000-8000-000000000002";

    #[test]
    fn ids_are_exactly_kind_colon_uuid() {
        assert!(valid_id("session", SESSION));
        assert!(valid_id("block", BLOCK));
        for bad in [
            "session:01A1A1CB-0D4E-7C1A-9F00-3B2A1C0D9E8F",
            "session:01a1a1cb0d4e7c1a9f003b2a1c0d9e8f",
            "session:01a1a1cb-0d4e-7c1a-9f00-3b2a1c0d9e8f\n",
            "window:01a1a1cb-0d4e-7c1a-9f00-3b2a1c0d9e8f",
            "session:",
            "",
        ] {
            assert!(!valid_id("session", bad), "{bad:?}");
        }
    }

    /// The request runs the argv with no shell, in the folder, keeps the
    /// tab after the program exits, and asks for focus: what the research
    /// ran against the installed beta.
    #[cfg(unix)]
    #[test]
    fn the_request_runs_the_argv_with_no_shell_in_the_folder() {
        let argv: Vec<std::ffi::OsString> = vec![
            "/Applications/N.app/Contents/Resources/near-ai".into(),
            "--ticket".into(),
            "ab12".into(),
        ];
        let tab = RexTab::new(&argv, Some(Path::new("/Users/a/my \"odd\" project"))).unwrap();
        let request: Value = serde_json::from_slice(&tab.request).unwrap();
        assert_eq!(request["focus"], true);
        assert_eq!(request["window_label"], "my \"odd\" project");
        let block = &request["layout"]["block"];
        assert_eq!(block["flavor"], "com.superlogical.terminal.shell");
        let options = &block["options"];
        assert_eq!(options["shell"], "none");
        assert_eq!(options["cwd"], "/Users/a/my \"odd\" project");
        assert_eq!(options["exit"]["on_completion"], false);
        assert_eq!(
            options["command"],
            json!([
                "/Applications/N.app/Contents/Resources/near-ai",
                "--ticket",
                "ab12"
            ])
        );
        assert!(options.get("initial_input").is_none(), "never typed");

        let setup = RexTab::new(&argv, None).unwrap();
        let request: Value = serde_json::from_slice(&setup.request).unwrap();
        assert!(request["layout"]["block"]["options"].get("cwd").is_none());
        assert_eq!(request["window_label"], GROUP_LABEL);

        assert_eq!(RexTab::new(&[], None), Err(ManagedError::InvalidRequest));
        assert_eq!(
            RexTab::new(&argv, Some(Path::new("relative"))),
            Err(ManagedError::InvalidRequest)
        );
    }

    #[test]
    fn the_group_is_found_by_its_label_whatever_the_listing_calls_its_fields() {
        let listing = format!(
            r#"[{{"id":"session:01a1a1cb-0d4e-7c1a-9f00-000000000009","label":"work"}},
               {{"session_id":"{SESSION}","name":"Near AI Gateway","windows":3}}]"#
        );
        assert_eq!(group_session(listing.as_bytes()), Some(SESSION.into()));
        let nested = format!(r#"{{"sessions":[{{"id":"{SESSION}","label":"Near AI Gateway"}}]}}"#);
        assert_eq!(group_session(nested.as_bytes()), Some(SESSION.into()));
        assert_eq!(
            group_session(br#"[{"id":"session:x","label":"Near AI Gateway"}]"#),
            None
        );
        assert_eq!(group_session(b"[]"), None);
        assert_eq!(group_session(b"not json"), None);
    }

    #[test]
    fn a_made_session_is_read_from_rex_new_however_it_prints_it() {
        assert_eq!(
            first_id("session", format!("session={SESSION}\n").as_bytes()),
            Some(SESSION.into())
        );
        assert_eq!(
            first_id(
                "session",
                format!(r#"{{"session_id":"{SESSION}"}}"#).as_bytes()
            ),
            Some(SESSION.into())
        );
        assert_eq!(first_id("session", b"session:nope"), None);
    }

    #[test]
    fn the_new_window_reply_names_the_tab() {
        let reply = format!(r#"{{"window_id":"{WINDOW}","block_ids":["{BLOCK}"],"revision":2}}"#);
        assert_eq!(
            new_window_reply(reply.as_bytes()),
            Some((WINDOW.into(), BLOCK.into()))
        );
        assert_eq!(
            new_window_reply(br#"{"window_id":"window:1","block_ids":["block:2"]}"#),
            None
        );
        assert_eq!(new_window_reply(br#"{"window_id":"x"}"#), None);
    }

    #[test]
    fn a_printed_tab_is_three_checked_ids() {
        assert_eq!(
            tab(&printed(SESSION, WINDOW, BLOCK)),
            Some(TerminalTab {
                handle: TerminalHandle::Rex {
                    session: SESSION.into(),
                    window: WINDOW.into(),
                    block: BLOCK.into(),
                },
                window: None,
            })
        );
        assert_eq!(tab(b""), None);
        assert_eq!(tab(&printed(SESSION, BLOCK, WINDOW)), None);
        let mut extra = printed(SESSION, WINDOW, BLOCK);
        extra.extend_from_slice(b"more\n");
        assert_eq!(tab(&extra), None);
    }

    /// A stored handle is checked again when it is read back.
    #[test]
    fn a_stored_handle_with_a_bad_id_does_not_load() {
        let good = json!({"kind":"rex","session":SESSION,"window":WINDOW,"block":BLOCK});
        assert!(serde_json::from_value::<TerminalHandle>(good).is_ok());
        let bad = json!({"kind":"rex","session":SESSION,"window":WINDOW,"block":"block:1; rm"});
        assert!(serde_json::from_value::<TerminalHandle>(bad).is_err());
    }

    /// A `rex` that answers each command from a table (none for a command
    /// it does not hold, or once a reply runs out), and records each call:
    /// its args and its input.
    #[derive(Default)]
    struct FakeRex {
        replies: std::collections::HashMap<&'static str, Vec<Option<String>>>,
        calls: Vec<(Vec<String>, Option<Vec<u8>>)>,
    }

    impl FakeRex {
        /// A Rex with its app connected, revision 10, and `listing`.
        fn ready(listing: Option<String>) -> Self {
            let mut fake = FakeRex::default();
            fake.set("client", vec![Some(r#"[{"id":"c1","kind":"app"}]"#.into())]);
            fake.set(
                "version",
                vec![Some(
                    "rex 0.0.1\napi: revision 10 (supports 0 to 10)\n".into(),
                )],
            );
            fake.set("ls", vec![listing]);
            fake.set("api", vec![reply()]);
            fake.set("focus", vec![Some(String::new())]);
            fake
        }

        fn set(&mut self, command: &'static str, replies: Vec<Option<String>>) {
            self.replies.insert(command, replies);
        }

        fn call(&mut self, args: &[&str], input: Option<&[u8]>) -> Option<Vec<u8>> {
            self.calls.push((
                args.iter().map(|a| a.to_string()).collect(),
                input.map(<[u8]>::to_vec),
            ));
            let replies = self.replies.get_mut(*args.first()?)?;
            if replies.is_empty() {
                return None;
            }
            replies.remove(0).map(String::into_bytes)
        }

        fn commands(&self) -> Vec<String> {
            self.calls.iter().map(|(args, _)| args.join(" ")).collect()
        }
    }

    /// The launch's folder: absolute on every platform, as `RexTab::new`
    /// requires (`/tmp/project` is not absolute on Windows).
    fn folder() -> std::path::PathBuf {
        std::env::temp_dir().join("project")
    }

    fn rex_tab() -> RexTab {
        let argv: Vec<std::ffi::OsString> =
            vec!["/x/near-ai".into(), "--ticket".into(), "ab12".into()];
        RexTab::new(&argv, Some(&folder())).unwrap()
    }

    fn run(fake: &mut FakeRex) -> Result<Vec<u8>, ManagedError> {
        open_tab_with(
            |args, input| fake.call(args, input),
            || true,
            || std::time::Duration::ZERO,
            |_| {},
            &rex_tab(),
        )
    }

    fn reply() -> Option<String> {
        Some(format!(
            r#"{{"window_id":"{WINDOW}","block_ids":["{BLOCK}"]}}"#
        ))
    }

    fn group() -> Option<String> {
        Some(format!(
            r#"[{{"id":"{SESSION}","label":"Near AI Gateway"}}]"#
        ))
    }

    /// The launch waits for the app to connect, checks the API revision,
    /// opens the tab in the existing group session with the request (and
    /// with it the ticket) only on stdin of `session.new_window`, and
    /// selects the tab and prints its ids.
    #[test]
    fn a_launch_opens_its_tab_in_the_group_session() {
        let mut fake = FakeRex::ready(group());
        fake.set(
            "client",
            vec![
                None,
                Some(r#"[{"id":"c0","kind":"cli"}]"#.into()),
                Some(r#"{"clients":[{"id":"c1","kind":"app"}]}"#.into()),
            ],
        );
        assert_eq!(run(&mut fake), Ok(printed(SESSION, WINDOW, BLOCK)));
        assert_eq!(
            fake.commands(),
            [
                "client ls --json".to_string(),
                "client ls --json".into(),
                "client ls --json".into(),
                "version".into(),
                "ls --json".into(),
                format!("api call -s {SESSION} session.new_window -"),
                format!("focus -s {SESSION} {BLOCK}"),
            ]
        );
        for (args, input) in &fake.calls {
            assert!(!args.iter().any(|a| a.contains("ab12")), "{args:?}");
            assert_eq!(input.is_some(), args[0] == "api", "{args:?}");
        }
        let (_, input) = fake
            .calls
            .iter()
            .find(|(args, _)| args[0] == "api")
            .unwrap();
        assert_eq!(input.as_deref(), Some(rex_tab().request.as_slice()));
    }

    /// With no "Near AI Gateway" session, one is made in the folder.
    #[test]
    fn a_missing_group_session_is_made_in_the_folder() {
        let mut fake = FakeRex::ready(Some("[]".into()));
        fake.set(
            "new",
            vec![Some(format!(r#"{{"session_id":"{SESSION}"}}"#))],
        );
        assert_eq!(run(&mut fake), Ok(printed(SESSION, WINDOW, BLOCK)));
        assert!(fake.commands().contains(&format!(
            "new Near AI Gateway --json -c {}",
            folder().display()
        )));
    }

    /// Anything that fails before `session.new_window` is a refusal, so the
    /// launch can be tried again; a failure of that request, which may have
    /// started the helper, is `LaunchUnknown`.
    #[test]
    fn only_a_failure_after_the_ticket_is_sent_is_launch_unknown() {
        let sent = |fake: &FakeRex| fake.calls.iter().any(|(args, _)| args[0] == "api");

        let mut made_nothing = FakeRex::ready(Some("[]".into()));
        made_nothing.set("new", vec![Some("made".into())]);
        assert_eq!(
            run(&mut made_nothing),
            Err(ManagedError::UnsupportedVersion)
        );
        assert!(!sent(&made_nothing));

        // A call that fails or gives nothing is not a version problem
        // (owner, 2026-11-03): the general failure line, still a refusal.
        let mut new_failed = FakeRex::ready(Some("[]".into()));
        new_failed.set("new", vec![None]);
        assert_eq!(run(&mut new_failed), Err(ManagedError::TerminalCallFailed));

        let mut no_listing = FakeRex::ready(None);
        assert_eq!(run(&mut no_listing), Err(ManagedError::TerminalCallFailed));
        assert!(!sent(&no_listing));

        let mut no_version = FakeRex::ready(group());
        no_version.set("version", vec![None]);
        assert_eq!(run(&mut no_version), Err(ManagedError::TerminalCallFailed));
        assert!(!sent(&no_version));

        for version in [
            "api: revision 12 (supports 11 to 12)",
            "api: revision 10",
            "rex 0.1",
        ] {
            let mut newer = FakeRex::ready(group());
            newer.set("version", vec![Some(version.into())]);
            assert_eq!(
                run(&mut newer),
                Err(ManagedError::UnsupportedVersion),
                "{version}"
            );
            assert!(!sent(&newer), "{version}");
        }

        let mut call_failed = FakeRex::ready(group());
        call_failed.set("api", vec![None]);
        assert_eq!(run(&mut call_failed), Err(ManagedError::LaunchUnknown));

        let mut unreadable = FakeRex::ready(group());
        unreadable.set("api", vec![Some("{}".into())]);
        assert_eq!(run(&mut unreadable), Ok(Vec::new()), "open, not shown");
        assert!(
            !unreadable.calls.iter().any(|(args, _)| args[0] == "focus"),
            "nothing to select"
        );

        let mut called = false;
        let closed = open_tab_with(
            |_, _| {
                called = true;
                None
            },
            || false,
            || std::time::Duration::ZERO,
            |_| {},
            &rex_tab(),
        );
        assert_eq!(closed, Err(ManagedError::TerminalNotFound));
        assert!(!called, "the CLI is not run when the app does not open");
    }

    /// The wait for the app ends on the clock, counting the time each try
    /// took, not only the sleeps between them. A server with no app (one
    /// the CLI started headless) is waited past, never launched into.
    #[test]
    fn the_wait_for_the_app_is_bounded_by_the_clock() {
        let clock = std::cell::Cell::new(std::time::Duration::ZERO);
        let mut tries = 0;
        let never = open_tab_with(
            |args, _| {
                assert_eq!(args, ["client", "ls", "--json"], "nothing else is asked");
                tries += 1;
                clock.set(clock.get() + std::time::Duration::from_secs(3));
                Some(br#"[{"id":"c0","kind":"cli"}]"#.to_vec())
            },
            || true,
            || clock.get(),
            |step| clock.set(clock.get() + step),
            &rex_tab(),
        );
        // Rex is there but its app did not connect: "Rex didn't open in
        // time. Try again." (owner, 2026-11-03), not "wasn't found".
        assert_eq!(never, Err(ManagedError::RexNotReady));
        assert_eq!(tries, 4, "three slow tries pass the wait, a fourth ends it");
        assert!(clock.get() < SERVER_WAIT + std::time::Duration::from_secs(4));
    }

    #[test]
    fn the_api_revision_and_the_app_client_are_read_from_rex() {
        assert!(supports_revision(
            b"rex 0.0.1793645657-gd018d2\napi: revision 10 (supports 0 to 10)\n"
        ));
        assert!(supports_revision(b"api: revision 14 (supports 8 to 14)"));
        assert!(!supports_revision(b"api: revision 14 (supports 11 to 14)"));
        assert!(!supports_revision(b""));
        assert!(app_connected(br#"[{"kind":"cli"},{"kind":"app"}]"#));
        assert!(!app_connected(br#"[{"kind":"cli"}]"#));
        assert!(!app_connected(br#"[{"name":"app"}]"#));
        assert!(!app_connected(b"app"));
    }

    /// The request reaches the CLI on stdin, never in its argv.
    #[cfg(unix)]
    #[test]
    fn input_reaches_the_program_on_stdin() {
        let mut command = std::process::Command::new("/bin/cat");
        command.arg("-");
        let output =
            super::super::profiles::bounded_output_with_input(command, b"{\"a\":1}").unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"{\"a\":1}");
    }
}
