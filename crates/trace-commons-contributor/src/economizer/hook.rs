//! The `economizer-hook` subcommand: the coding tool runs it on each hook
//! event of a managed session ([`super::launch`]), with the event's JSON on
//! stdin. Claude Code and Codex speak this contract themselves; OpenCode's
//! plugin translates its events into it.
//!
//! | Event | What it does |
//! | --- | --- |
//! | `SessionStart` | Starts the session's state; tells the agent the folder's facts. |
//! | `UserPromptSubmit` | Counts the prompt; starts a new turn. |
//! | `PostToolUse`, `PostToolUseFailure` | Folds the call; a stuck turn gets its redirect. |
//! | `Stop` | A turn that ended well teaches the folder's envelope. |
//! | `SessionEnd` | Folds the session into the folder's tools and facts. |
//!
//! It never fails loudly: any failure prints nothing and exits 0, so a
//! broken economizer leaves the session as if it were not there. Turning
//! the economizer off takes effect at the next event.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

use super::Harness;
use super::loop_guard::{self, Thresholds};
use super::signals::{ToolCall, short_digest};
use super::store::{SessionState, Store};

/// How long the hook waits for its input.
pub const BUDGET: Duration = Duration::from_secs(3);
/// The largest input read.
const MAX_INPUT: u64 = 4 * 1024 * 1024;

/// Handle one event of `harness` for the folder `repo`. Returns the JSON to
/// print, if any.
pub fn handle(
    store: &Store,
    repo: &str,
    harness: Harness,
    input: &Value,
    now: DateTime<Utc>,
) -> Option<Value> {
    if !store.enabled() {
        return None;
    }
    let event = input.get("hook_event_name")?.as_str()?;
    let session_id = input.get("session_id")?.as_str()?;
    let load = || {
        store
            .session(session_id)
            .filter(|state| state.repo == repo)
            .unwrap_or_else(|| SessionState::new(repo, now))
    };
    match event {
        "SessionStart" => {
            store.prune_sessions(now);
            let session = load();
            store.save_session(session_id, &session).ok()?;
            let context = store.repo(repo).facts.render()?;
            Some(context_output(event, &context))
        }
        "UserPromptSubmit" => {
            let mut session = load();
            session.prompts = session.prompts.saturating_add(1);
            session.turn = Default::default();
            store.save_session(session_id, &session).ok()?;
            None
        }
        "PostToolUse" | "PostToolUseFailure" => {
            let call = ToolCall::from_hook(input, event == "PostToolUseFailure")?;
            let mut session = load();
            if session.used_tools.len() < 512 {
                session.used_tools.insert(call.tool.clone());
            }
            session.turn.record(&call);
            session.observations.record(&call);
            let envelope = store.repo(repo).envelope;
            let redirect =
                loop_guard::should_redirect(&mut session.turn, &envelope, &Thresholds::default());
            store.save_session(session_id, &session).ok()?;
            redirect.then(|| context_output(event, &loop_guard::redirect_text(&session.turn)))
        }
        "Stop" => {
            let mut session = load();
            if session.turn.ended_well() {
                let mut state = store.repo(repo);
                state.envelope.add(session.turn.calls);
                store.save_repo(repo, &state).ok()?;
            }
            session.turn = Default::default();
            store.save_session(session_id, &session).ok()?;
            None
        }
        "SessionEnd" => {
            let session = store
                .session(session_id)
                .filter(|state| state.repo == repo)?;
            let mut state = store.repo(repo);
            state
                .tools
                .fold_session(harness, session.prompts, &session.used_tools);
            state
                .facts
                .fold_session(&short_digest("session", session_id), &session.observations);
            state.sessions = state.sessions.saturating_add(1);
            store.save_repo(repo, &state).ok()?;
            store.remove_session(session_id);
            None
        }
        _ => None,
    }
}

/// The output that adds `context` to the model's context. It names the
/// event it answers, as Claude Code requires.
fn context_output(event: &str, context: &str) -> Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": event,
            "additionalContext": context,
        }
    })
}

/// The subcommand: read the event, handle it, print the answer. Never an
/// error.
pub fn run(state: Option<&Path>, repo: Option<&str>, harness: Harness) {
    let (Some(state), Some(repo)) = (state, repo) else {
        return;
    };
    let Some(input) = read_input() else {
        return;
    };
    let Ok(input) = serde_json::from_str::<Value>(&input) else {
        return;
    };
    let store = Store::at(state.to_path_buf());
    if let Some(output) = handle(&store, repo, harness, &input, Utc::now()) {
        use std::io::Write;
        let _ = writeln!(std::io::stdout().lock(), "{output}");
    }
}

/// The event on stdin, read for at most [`BUDGET`]. The reader is a
/// detached thread, so one blocked on stdin never holds the process open.
fn read_input() -> Option<String> {
    use std::io::{IsTerminal, Read};
    if std::io::stdin().is_terminal() {
        return None;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut buffer = String::new();
        let read = std::io::stdin()
            .take(MAX_INPUT)
            .read_to_string(&mut buffer)
            .ok();
        let _ = sender.send(read.map(|_| buffer));
    });
    receiver.recv_timeout(BUDGET).ok().flatten()
}
