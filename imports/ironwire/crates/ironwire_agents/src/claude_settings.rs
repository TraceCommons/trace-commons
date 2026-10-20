//! Editing `~/.claude/settings.json` without taking anything over.
//!
//! The counterpart to [`crate::codex_config`], and it follows the same rule:
//! this is a file the user owns. `serde_json` is built with `preserve_order`,
//! so a round trip keeps their key order and every key we do not model — but
//! order is the easy half. The hard half is the two *slots* we want, each of
//! which the user may already be using:
//!
//! - `statusLine`, a single slot: someone who has written their own has put
//!   work into it, and replacing it to advertise ourselves would be the rudest
//!   thing in this codebase.
//! - `env.ANTHROPIC_BASE_URL`, which is what actually routes Claude Code here.
//!   A value already there is another proxy, or a deliberate choice, and taking
//!   it over would silently move someone's traffic.
//!
//! So, for both: we fill the slot when it is empty, we leave it alone when it
//! is not, and we remove only what we put there.
//!
//! And either slot may be skipped, because not every caller can fill both. The
//! status-line command is the caller's to name: the `ironwire` CLI names its own
//! `statusline` subcommand, a host embedding this crate names whatever command
//! of its own serves the line, and a host with none asks for the routing alone.
//! See [`connect`].

use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use crate::endpoint;

/// Marks the status line as ours, so `disconnect` can remove exactly what
/// `connect` added and nothing else. A command string is not enough — the user
/// may have edited it, and a heuristic match on our binary's name would happily
/// delete a line somebody wrote themselves around `ironwire statusline`.
const OURS: &str = "ironwire";

/// The setting that points Claude Code at a different endpoint.
const BASE_URL: &str = "ANTHROPIC_BASE_URL";

/// A slot that already held something the user put there.
///
/// Not an error, and not a failure of the edit: their value stays, and the
/// caller says what they could do by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupied {
    /// Which setting — `statusLine` or `ANTHROPIC_BASE_URL`.
    pub slot: &'static str,
    /// What is in it, so the caller can name it back to them.
    pub current: String,
}

/// What an edit would do, so the caller can show it before doing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The full file contents after the edit.
    pub contents: String,
    /// Human-readable lines describing what changed.
    pub changes: Vec<String>,
    /// Slots left alone because the user was already using them.
    pub occupied: Vec<Occupied>,
}

impl Edit {
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    /// What is in a slot we left alone, if we left that one alone.
    pub fn occupied_slot(&self, slot: &str) -> Option<&str> {
        self.occupied
            .iter()
            .find(|o| o.slot == slot)
            .map(|o| o.current.as_str())
    }
}

/// Point Claude Code at IronWire, and add our status line, in one edit.
///
/// Both slots are optional, and for the same reason: this file is touched for
/// more than one reason and by more than one caller.
///
/// `base_url` is `None` for a caller that wants only the status line — the
/// daemon is not the only reason to touch this file. `command` is `None` for
/// the inverse: a caller that wants only the routing, because it cannot honour
/// a status line. The command is written verbatim and is the caller's choice:
/// the `ironwire` CLI passes its own `statusline` subcommand, and a host that
/// embeds this crate passes the command of its own that serves the line
/// (`crate::tools::StatusLine::Command`). A host with no such command declines
/// the slot rather than filling it with a command that prints nothing.
///
/// Declining does not report `statusLine` as [`Occupied`]: a slot we never
/// wanted is not one the user took from us. It does take out a status line we
/// installed on an earlier connect — see [`clear_status_line`].
///
/// `base_url` may name a login (`/anthropic/account/{id}`, see
/// [`crate::endpoint`]), for the Claude Code that runs with a config dir of its
/// own. Either form of ours already in the file is ours to update, except that a
/// plain URL never replaces one naming a login — see [`keep_binding`].
///
/// # Errors
///
/// Returns the parse error when the existing file is not valid JSON — we will
/// not rewrite a file we cannot read, because the user's own syntax error would
/// then look like ours.
pub fn connect(
    existing: &str,
    command: Option<&str>,
    base_url: Option<&str>,
) -> Result<Edit, serde_json::Error> {
    let mut root = parse(existing)?;
    let mut changes = Vec::new();
    let mut occupied = Vec::new();

    match command {
        Some(command) => set_status_line(&mut root, command, &mut changes, &mut occupied),
        None => clear_status_line(&mut root, &mut changes),
    }

    if let Some(url) = base_url {
        set_base_url(&mut root, url, &mut changes, &mut occupied);
    }

    Ok(Edit {
        contents: render(&root),
        changes,
        occupied,
    })
}

/// Fill `statusLine`, unless someone else is using it.
fn set_status_line(
    root: &mut Map<String, Value>,
    command: &str,
    changes: &mut Vec<String>,
    occupied: &mut Vec<Occupied>,
) {
    match root.get("statusLine") {
        Some(current) if is_ours(current) => {
            // Ours already, but the path may have moved between installs.
            if current.get("command").and_then(Value::as_str) != Some(command) {
                root.insert("statusLine".to_string(), entry(command));
                changes.push(format!("statusLine: updated to `{command}`"));
            }
        }
        Some(current) => occupied.push(Occupied {
            slot: "statusLine",
            current: current
                .get("command")
                .and_then(Value::as_str)
                .unwrap_or("a status line of your own")
                .to_string(),
        }),
        None => {
            root.insert("statusLine".to_string(), entry(command));
            changes.push(format!("statusLine: `{command}` (added)"));
        }
    }
}

/// Take out a status line we installed, for a caller that cannot serve one.
///
/// Declining is not only about the connect in front of us. A host that declines
/// today may have installed one yesterday, through the only API there was
/// before declining existed, and that command names the executable it cannot
/// run — so skipping the slot would leave the stale command in the file for
/// good. Declining has to fix an install as well as avoid making one.
///
/// Only ours, by the `installedBy` marker [`disconnect`] uses: "remove only
/// what we put there" is not suspended by declining, and a line the user wrote
/// is not ours to take out on a connect any more than it is ours to replace.
///
/// The marker names IronWire rather than a particular binary, so in a home
/// shared with the `ironwire` CLI this takes out the CLI's working line too.
/// That is the identity [`disconnect`] has always used — nothing here can tell
/// the two apart — and `ironwire connect claude` puts it back.
fn clear_status_line(root: &mut Map<String, Value>, changes: &mut Vec<String>) {
    if root.get("statusLine").is_some_and(is_ours) {
        root.remove("statusLine");
        changes.push("statusLine: removed (this caller cannot serve one)".to_string());
    }
}

/// Fill `env.ANTHROPIC_BASE_URL`, unless someone else is using it.
fn set_base_url(
    root: &mut Map<String, Value>,
    url: &str,
    changes: &mut Vec<String>,
    occupied: &mut Vec<Occupied>,
) {
    // A non-object `env` is not ours to fix. Reporting it as occupied is
    // honest: we did not set the variable, and the user needs to know why.
    if let Some(existing) = root.get("env")
        && !existing.is_object()
    {
        occupied.push(Occupied {
            slot: BASE_URL,
            current: "an `env` that is not an object".to_string(),
        });
        return;
    }

    match root.get("env").and_then(|e| e.get(BASE_URL)) {
        // Ours, but the port may have changed since we wrote it — or the
        // login it is bound to, which is a connect naming this directory.
        Some(current) if current.as_str().is_some_and(points_at_us) => {
            let url = keep_binding(current.as_str().unwrap_or_default(), url);
            if current.as_str() != Some(url.as_str()) {
                insert_base_url(root, &url);
                changes.push(format!("env.{BASE_URL}: updated to `{url}`"));
            }
        }
        Some(current) => occupied.push(Occupied {
            slot: BASE_URL,
            current: current
                .as_str()
                .unwrap_or("a value of your own")
                .to_string(),
        }),
        None => {
            insert_base_url(root, url);
            changes.push(format!("env.{BASE_URL}: `{url}` (added)"));
        }
    }
}

/// Create the `env` object only once we know we are writing into it — an empty
/// `env: {}` left behind by a no-op edit is litter in someone else's file.
fn insert_base_url(root: &mut Map<String, Value>, url: &str) {
    let env = root
        .entry("env".to_string())
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(env) = env.as_object_mut() {
        env.insert(BASE_URL.to_string(), Value::String(url.to_string()));
    }
}

/// Whether a base URL is one of ours — the plain façade or a login-bound one.
///
/// Loopback and our own path: anything else is another proxy or a deliberate
/// choice, and overwriting it would move someone's traffic without telling
/// them. See [`crate::endpoint::parse_anthropic`].
fn points_at_us(url: &str) -> bool {
    endpoint::parse_anthropic(url).is_some()
}

/// The URL to write over one of ours that is already there.
///
/// A URL naming a login is more specific than one that does not. A connect
/// that names no login — the menu bar's, or one run where this file is the
/// default login's — must not quietly unbind the Claude Code that reads this
/// file, which would move its traffic onto the default login's subscription.
/// So it keeps the login and follows only the port. A connect that does name
/// one is a person pointing this directory at that login, and wins.
fn keep_binding(current: &str, wanted: &str) -> String {
    match (
        endpoint::parse_anthropic(current),
        endpoint::parse_anthropic(wanted),
    ) {
        (
            Some(endpoint::Endpoint {
                account: Some(account),
                ..
            }),
            Some(endpoint::Endpoint {
                port,
                account: None,
            }),
        ) => endpoint::anthropic_url(port, Some(&account)),
        _ => wanted.to_string(),
    }
}

/// Remove what [`connect`] added, and only that.
///
/// # Errors
///
/// Returns the parse error when the existing file is not valid JSON.
pub fn disconnect(existing: &str) -> Result<Edit, serde_json::Error> {
    let mut root = parse(existing)?;
    let mut changes = Vec::new();

    if root.get("statusLine").is_some_and(is_ours) {
        root.remove("statusLine");
        changes.push("statusLine: removed".to_string());
    }

    let ours = root
        .get("env")
        .and_then(|e| e.get(BASE_URL))
        .and_then(Value::as_str)
        .is_some_and(points_at_us);
    if ours && let Some(env) = root.get_mut("env").and_then(Value::as_object_mut) {
        env.remove(BASE_URL);
        changes.push(format!("env.{BASE_URL}: removed"));
        // Our variable was the only thing in there, so the block was ours too.
        if env.is_empty() {
            root.remove("env");
        }
    }

    Ok(Edit {
        contents: render(&root),
        changes,
        occupied: Vec::new(),
    })
}

fn entry(command: &str) -> Value {
    serde_json::json!({
        "type": "command",
        "command": command,
        // Our own marker. Claude Code ignores keys it does not know, and this
        // is what makes removal exact rather than a guess.
        "installedBy": OURS,
    })
}

fn is_ours(value: &Value) -> bool {
    value.get("installedBy").and_then(Value::as_str) == Some(OURS)
}

/// An absent or blank file is an empty object, not a parse failure: "you have
/// no settings yet" is the commonest state, not an error.
fn parse(existing: &str) -> Result<Map<String, Value>, serde_json::Error> {
    if existing.trim().is_empty() {
        return Ok(Map::new());
    }
    serde_json::from_str(existing)
}

fn render(root: &Map<String, Value>) -> String {
    let mut out = serde_json::to_string_pretty(root).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

/// The settings file of the Claude Code this process would start:
/// `CLAUDE_CONFIG_DIR`'s when that is set, as it is for Claude Code itself.
///
/// For asking whether *that* Claude Code was wired — never for choosing a
/// file to write the plain URL into. Such a directory is a login of its own,
/// wired with [`path_in`] and the URL naming it; the plain URL belongs in
/// [`default_dir`]'s file alone, which is why the control API's Claude Code
/// entry reads that one.
#[must_use]
pub fn path() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CLAUDE_CONFIG_DIR")
        && !dir.is_empty()
    {
        return Some(path_in(Path::new(&dir)));
    }
    Some(path_in(&default_dir()?))
}

/// The settings file of the Claude Code that runs with
/// `CLAUDE_CONFIG_DIR=dir`.
#[must_use]
pub fn path_in(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

/// Where Claude Code keeps everything when `CLAUDE_CONFIG_DIR` is unset —
/// the default login's directory. Never this process's `CLAUDE_CONFIG_DIR`.
#[must_use]
pub fn default_dir() -> Option<PathBuf> {
    Some(dirs::home_dir()?.join(".claude"))
}

/// Whether these settings currently route Claude Code through IronWire,
/// bound to a login or not.
#[must_use]
pub fn is_wired(existing: &str) -> bool {
    base_url(existing).is_some_and(|url| points_at_us(&url))
}

/// The base URL these settings point Claude Code at, if they set one.
///
/// An unparseable file says nothing: callers report on the user's setup and
/// must not fail over a syntax error in a file they do not own.
#[must_use]
pub fn base_url(existing: &str) -> Option<String> {
    let root = serde_json::from_str::<Value>(existing).ok()?;
    Some(root.get("env")?.get(BASE_URL)?.as_str()?.to_string())
}

/// The login these settings bind Claude Code to, when they point at a
/// login-bound URL of ours.
#[must_use]
pub fn account(existing: &str) -> Option<String> {
    endpoint::parse_anthropic(&base_url(existing)?)?.account
}

#[cfg(test)]
mod tests {
    use super::*;

    const COMMAND: &str = "/usr/local/bin/ironwire statusline";
    const URL: &str = "http://127.0.0.1:8463/anthropic";

    fn parsed(edit: &Edit) -> Value {
        serde_json::from_str(&edit.contents).expect("still JSON")
    }

    #[test]
    fn an_empty_file_gains_a_status_line() {
        let edit = connect("", Some(COMMAND), None).expect("valid");
        assert!(edit.contents.contains("statusLine"));
        assert!(edit.contents.contains(COMMAND));
        assert!(edit.occupied.is_empty());
    }

    #[test]
    fn every_other_setting_survives_the_edit() {
        let existing = r#"{"model":"opus","permissions":{"allow":["Bash(ls:*)"]}}"#;
        let edit = connect(existing, Some(COMMAND), Some(URL)).expect("valid");
        let parsed = parsed(&edit);
        assert_eq!(parsed["model"], "opus");
        assert_eq!(parsed["permissions"]["allow"][0], "Bash(ls:*)");
    }

    /// The rule this module exists for. Someone's own status line represents
    /// work, and it occupies the only slot there is.
    #[test]
    fn a_status_line_of_their_own_is_never_replaced() {
        let existing = r#"{"statusLine":{"type":"command","command":"~/bin/my-prompt.sh"}}"#;
        let edit = connect(existing, Some(COMMAND), None).expect("valid");
        assert!(edit.is_noop(), "changed: {:?}", edit.changes);
        assert_eq!(edit.occupied_slot("statusLine"), Some("~/bin/my-prompt.sh"));
        assert_eq!(parsed(&edit)["statusLine"]["command"], "~/bin/my-prompt.sh");
    }

    /// The case an embedding host needs. The command written into `statusLine`
    /// is the calling binary plus a subcommand, and a host that implements no
    /// such subcommand would be pointing Claude Code at something that prints
    /// nothing. It still wants the routing.
    #[test]
    fn a_caller_that_cannot_run_a_status_line_still_gets_the_routing() {
        let edit = connect("", None, Some(URL)).expect("valid");
        let parsed = parsed(&edit);
        assert_eq!(parsed["env"][BASE_URL], URL);
        assert!(parsed.get("statusLine").is_none(), "{}", edit.contents);
        assert_eq!(edit.changes.len(), 1, "changed: {:?}", edit.changes);
        assert!(edit.occupied.is_empty());
    }

    /// Declining is not the same as being refused. A slot we never asked for
    /// is not one the user took from us, so it is not reported back as theirs.
    #[test]
    fn declining_the_status_line_leaves_theirs_alone_without_calling_it_occupied() {
        let existing = r#"{"statusLine":{"type":"command","command":"~/bin/my-prompt.sh"}}"#;
        let edit = connect(existing, None, Some(URL)).expect("valid");
        assert_eq!(parsed(&edit)["statusLine"]["command"], "~/bin/my-prompt.sh");
        assert_eq!(edit.occupied_slot("statusLine"), None);
    }

    /// Declining the status line is not a licence over the other slot: a base
    /// URL the user set is still theirs, and still reported.
    #[test]
    fn declining_the_status_line_still_leaves_a_base_url_of_their_own() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy.corp.internal"}}"#;
        let edit = connect(existing, None, Some(URL)).expect("valid");
        assert!(edit.is_noop(), "changed: {:?}", edit.changes);
        assert_eq!(
            edit.occupied_slot(BASE_URL),
            Some("https://proxy.corp.internal")
        );
        assert_eq!(
            parsed(&edit)["env"][BASE_URL],
            "https://proxy.corp.internal"
        );
    }

    /// The case an upgrade lands in. `plan_connect` was the only API before
    /// declining existed, so a host that used it already has a status line
    /// naming the executable it cannot run. Declining has to fix that install,
    /// not just avoid making another one.
    #[test]
    fn declining_removes_a_status_line_we_installed_before() {
        let installed = connect("", Some(COMMAND), Some(URL)).expect("valid");
        assert!(parsed(&installed)["statusLine"]["command"] == COMMAND);

        let declined = connect(&installed.contents, None, Some(URL)).expect("valid");
        assert!(parsed(&declined).get("statusLine").is_none());
        assert_eq!(
            declined.changes,
            vec!["statusLine: removed (this caller cannot serve one)"]
        );
    }

    /// "Remove only what we put there" is not suspended by declining. A line
    /// the user wrote is not ours to take out on a connect any more than it is
    /// ours to replace.
    #[test]
    fn declining_does_not_remove_a_status_line_of_their_own() {
        let existing = r#"{"statusLine":{"type":"command","command":"~/bin/my-prompt.sh"},"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic"}}"#;
        let edit = connect(existing, None, Some(URL)).expect("valid");
        assert!(edit.is_noop(), "changed: {:?}", edit.changes);
        assert_eq!(parsed(&edit)["statusLine"]["command"], "~/bin/my-prompt.sh");
    }

    /// A host that declined once and connects again must not accumulate an
    /// empty edit, and must still not acquire a status line.
    #[test]
    fn declining_twice_changes_nothing_the_second_time() {
        let first = connect("", None, Some(URL)).expect("valid");
        let second = connect(&first.contents, None, Some(URL)).expect("valid");
        assert!(second.is_noop(), "changed: {:?}", second.changes);
        assert!(parsed(&second).get("statusLine").is_none());
    }

    #[test]
    fn installing_twice_changes_nothing_the_second_time() {
        let first = connect("", Some(COMMAND), Some(URL)).expect("valid");
        let second = connect(&first.contents, Some(COMMAND), Some(URL)).expect("valid");
        assert!(second.is_noop(), "changed: {:?}", second.changes);
    }

    #[test]
    fn a_moved_binary_updates_the_command() {
        let first = connect("", Some("/old/path/ironwire statusline"), None).expect("valid");
        let second = connect(&first.contents, Some(COMMAND), None).expect("valid");
        assert!(!second.is_noop());
        assert!(second.contents.contains(COMMAND));
    }

    #[test]
    fn an_empty_file_gains_the_base_url() {
        let edit = connect("", Some(COMMAND), Some(URL)).expect("valid");
        assert_eq!(parsed(&edit)["env"][BASE_URL], URL);
    }

    #[test]
    fn an_existing_env_block_keeps_its_other_variables() {
        let existing = r#"{"env":{"FOO":"bar"}}"#;
        let edit = connect(existing, Some(COMMAND), Some(URL)).expect("valid");
        let parsed = parsed(&edit);
        assert_eq!(parsed["env"]["FOO"], "bar");
        assert_eq!(parsed["env"][BASE_URL], URL);
    }

    /// The routing equivalent of the status-line rule: a base URL already set
    /// is another proxy, and taking it over would move their traffic silently.
    #[test]
    fn a_base_url_of_their_own_is_never_replaced() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy.corp.internal"}}"#;
        let edit = connect(existing, Some(COMMAND), Some(URL)).expect("valid");
        assert_eq!(
            edit.occupied_slot(BASE_URL),
            Some("https://proxy.corp.internal")
        );
        assert_eq!(
            parsed(&edit)["env"][BASE_URL],
            "https://proxy.corp.internal"
        );
    }

    #[test]
    fn a_port_change_moves_our_own_base_url() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9999/anthropic"}}"#;
        let edit = connect(existing, Some(COMMAND), Some(URL)).expect("valid");
        assert!(!edit.is_noop());
        assert_eq!(parsed(&edit)["env"][BASE_URL], URL);
    }

    /// A loopback URL that is not ours — someone else's proxy on localhost —
    /// is still theirs.
    #[test]
    fn a_loopback_url_with_another_path_is_not_ours() {
        assert!(points_at_us(URL));
        assert!(points_at_us("http://localhost:1234/anthropic"));
        assert!(!points_at_us("http://127.0.0.1:8082"));
        assert!(!points_at_us("http://127.0.0.1:8082/v1"));
        assert!(!points_at_us("https://api.anthropic.com"));
        assert!(!points_at_us("http://127.0.0.1:notaport/anthropic"));
    }

    /// Reporting the slot as occupied beats writing into a shape we do not
    /// understand, or worse, replacing it.
    #[test]
    fn an_env_that_is_not_an_object_is_left_alone() {
        let existing = r#"{"env":"inherit"}"#;
        let edit = connect(existing, Some(COMMAND), Some(URL)).expect("valid");
        assert!(edit.occupied_slot(BASE_URL).is_some());
        assert_eq!(parsed(&edit)["env"], "inherit");
    }

    #[test]
    fn disconnect_removes_ours_and_leaves_the_rest() {
        let existing = connect(r#"{"model":"opus"}"#, Some(COMMAND), Some(URL)).expect("valid");
        let removed = disconnect(&existing.contents).expect("valid");
        let parsed = parsed(&removed);
        assert!(parsed.get("statusLine").is_none());
        // The whole `env` block was ours, so it goes too.
        assert!(parsed.get("env").is_none());
        assert_eq!(parsed["model"], "opus");
    }

    #[test]
    fn disconnect_keeps_an_env_block_that_holds_more_than_ours() {
        let existing =
            connect(r#"{"env":{"FOO":"bar"}}"#, Some(COMMAND), Some(URL)).expect("valid");
        let removed = disconnect(&existing.contents).expect("valid");
        let parsed = parsed(&removed);
        assert_eq!(parsed["env"]["FOO"], "bar");
        assert!(parsed["env"].get(BASE_URL).is_none());
    }

    /// Neither slot is ours to take back out if we did not put it there.
    #[test]
    fn disconnect_leaves_settings_we_did_not_install() {
        let existing = r#"{"statusLine":{"type":"command","command":"~/bin/mine.sh"},"env":{"ANTHROPIC_BASE_URL":"https://proxy.corp.internal"}}"#;
        let removed = disconnect(existing).expect("valid");
        assert!(removed.is_noop());
        let parsed = parsed(&removed);
        assert_eq!(parsed["statusLine"]["command"], "~/bin/mine.sh");
        assert_eq!(parsed["env"][BASE_URL], "https://proxy.corp.internal");
    }

    #[test]
    fn invalid_json_is_refused_rather_than_rewritten() {
        assert!(connect("{ not json", Some(COMMAND), Some(URL)).is_err());
        assert!(disconnect("{ not json").is_err());
    }

    const BOUND: &str = "http://127.0.0.1:8463/anthropic/account/claude-work";

    #[test]
    fn a_settings_file_pointed_at_a_login_bound_url_counts_as_ours() {
        let existing = format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{BOUND}"}}}}"#);
        assert!(is_wired(&existing));
        assert_eq!(account(&existing).as_deref(), Some("claude-work"));
        assert_eq!(
            account(&format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{URL}"}}}}"#)),
            None
        );

        // Ours to take back out, like the plain one.
        let removed = disconnect(&existing).expect("valid");
        assert!(
            parsed(&removed).get("env").is_none(),
            "{}",
            removed.contents
        );
    }

    #[test]
    fn wiring_a_login_directory_writes_its_bound_url() {
        let fresh = connect("", Some(COMMAND), Some(BOUND)).expect("valid");
        assert_eq!(parsed(&fresh)["env"][BASE_URL], BOUND);

        // The plain URL in that file is ours, so binding the login replaces it.
        let plain = connect("", Some(COMMAND), Some(URL)).expect("valid");
        let bound = connect(&plain.contents, Some(COMMAND), Some(BOUND)).expect("valid");
        assert_eq!(parsed(&bound)["env"][BASE_URL], BOUND);
        assert_eq!(
            bound.changes,
            vec![format!("env.{BASE_URL}: updated to `{BOUND}`")]
        );
    }

    /// Binding a login is not a licence over the existing rule: a URL that is
    /// not ours stays, and is reported as theirs.
    #[test]
    fn a_login_never_takes_a_url_that_is_not_ours() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy.corp.internal"}}"#;
        let edit = connect(existing, Some(COMMAND), Some(BOUND)).expect("valid");
        assert_eq!(
            edit.occupied_slot(BASE_URL),
            Some("https://proxy.corp.internal")
        );
        assert_eq!(
            parsed(&edit)["env"][BASE_URL],
            "https://proxy.corp.internal"
        );
    }

    /// The menu bar's connect names no login. Run against a work directory's
    /// file, replacing its URL with the plain one would move that Claude Code
    /// onto the default login's subscription without a word.
    #[test]
    fn a_connect_that_names_no_login_does_not_unbind_one() {
        let existing = format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{BOUND}"}}}}"#);
        let same_port = connect(&existing, Some(COMMAND), Some(URL)).expect("valid");
        assert_eq!(parsed(&same_port)["env"][BASE_URL], BOUND);

        // It still follows the daemon to a new port.
        let moved = connect(
            &existing,
            Some(COMMAND),
            Some("http://127.0.0.1:9000/anthropic"),
        )
        .expect("valid");
        assert_eq!(
            parsed(&moved)["env"][BASE_URL],
            "http://127.0.0.1:9000/anthropic/account/claude-work"
        );
    }

    #[test]
    fn a_login_directory_keeps_its_settings_beside_its_credentials() {
        let dir = std::path::Path::new("/home/alice/.claude-work");
        assert_eq!(path_in(dir), dir.join("settings.json"));
    }
}
