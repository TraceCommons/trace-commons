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
pub const GROUP_LABEL: &str = "Near.ai";

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

/// The `rex` CLI: inside the app bundle LaunchServices names, else the copy
/// the app keeps under Application Support. Never looked up on `PATH`.
#[cfg(target_os = "macos")]
fn cli() -> Option<std::path::PathBuf> {
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
/// helper process; its stdout when it succeeds.
#[cfg(target_os = "macos")]
fn rex(cli: &Path, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>, ManagedError> {
    let mut command = std::process::Command::new(cli);
    command.args(args);
    let output = match input {
        Some(input) => super::profiles::bounded_output_with_input(command, input)?,
        None => super::profiles::bounded_output(command)?,
    };
    if output.status.success() {
        Ok(output.stdout)
    } else {
        Err(ManagedError::LaunchUnknown)
    }
}

/// Bring Rex.app forward through LaunchServices, starting it when it is not
/// running. No Apple Event, so macOS asks for no permission.
#[cfg(target_os = "macos")]
fn raise_app() -> Result<(), ManagedError> {
    let mut open = std::process::Command::new("/usr/bin/open");
    open.args(["-b", BUNDLE_ID]);
    match super::profiles::bounded_output(open) {
        Ok(output) if output.status.success() => Ok(()),
        _ => Err(ManagedError::LaunchUnknown),
    }
}

/// Open `tab` in Rex: start or raise the app, find the "Near.ai" session or
/// make it, open the tab there and select it. Prints the ids for [`tab`].
/// `TerminalNotFound` only when the CLI is missing, before Rex is asked
/// anything.
#[cfg(target_os = "macos")]
pub fn open_tab(tab: &RexTab) -> Result<Vec<u8>, ManagedError> {
    let cli = cli().ok_or(ManagedError::TerminalNotFound)?;
    raise_app()?;
    // The server may still be starting with the app: wait for a listing.
    let mut listing = Err(ManagedError::LaunchUnknown);
    for _ in 0..50 {
        listing = rex(&cli, &["ls", "--json"], None);
        if listing.is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let session = match group_session(&listing?) {
        Some(session) => session,
        None => {
            let mut args = vec!["new", GROUP_LABEL, "--json"];
            let cwd = tab.cwd.as_deref().and_then(Path::to_str);
            if let Some(cwd) = cwd {
                args.extend(["-c", cwd]);
            }
            first_id("session", &rex(&cli, &args, None)?).ok_or(ManagedError::LaunchUnknown)?
        }
    };
    let reply = rex(
        &cli,
        &["api", "call", "-s", &session, "session.new_window", "-"],
        Some(&tab.request),
    )?;
    let Some((window, block)) = new_window_reply(&reply) else {
        // The tab may be open; it simply cannot be shown.
        return Ok(Vec::new());
    };
    let _ = rex(&cli, &["focus", "-s", &session, &block], None);
    Ok(printed(&session, &window, &block))
}

/// Show: select the block's tab in its session, then raise the app. A
/// block Rex no longer has (the tab was closed) is `NotFound`.
#[cfg(target_os = "macos")]
pub fn focus(session: &str, block: &str) -> Result<(), ManagedError> {
    if !valid_id("session", session) || !valid_id("block", block) {
        return Err(ManagedError::InvalidRequest);
    }
    let cli = cli().ok_or(ManagedError::NotFound)?;
    rex(&cli, &["focus", "-s", session, block], None).map_err(|_| ManagedError::NotFound)?;
    raise_app().map_err(|_| ManagedError::NotFound)
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
               {{"session_id":"{SESSION}","name":"Near.ai","windows":3}}]"#
        );
        assert_eq!(group_session(listing.as_bytes()), Some(SESSION.into()));
        let nested = format!(r#"{{"sessions":[{{"id":"{SESSION}","label":"Near.ai"}}]}}"#);
        assert_eq!(group_session(nested.as_bytes()), Some(SESSION.into()));
        assert_eq!(
            group_session(br#"[{"id":"session:x","label":"Near.ai"}]"#),
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
