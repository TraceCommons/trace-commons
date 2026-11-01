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
use super::signals::{CallKind, ToolCall, short_digest};
use super::store::{SessionState, Store};

/// How long the hook waits for its input.
pub const BUDGET: Duration = Duration::from_secs(3);
/// How long the hook waits for a folder's state lock before giving up
/// on its write. Giving up, like any other failure to write, drops that
/// turn's calls (`Stop`) or that session's evidence (`SessionEnd`) without
/// a record, so the economizer learns from one session fewer; it never undoes another writer's change, and the agent
/// is never held at the end of a turn.
const HOOK_LOCK_WAIT: Duration = Duration::from_millis(500);
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
    // Held for the whole event, so concurrent hooks of one session (parallel
    // tool calls) apply one after another.
    let _lock = store.lock_session(session_id).ok()?;
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
            let mut call = ToolCall::from_hook(input, event == "PostToolUseFailure")?;
            // Claude Code reports a command that exited non-zero through
            // `PostToolUseFailure`, so one it reports through `PostToolUse`
            // exited 0.
            if harness == Harness::Claude && !call.failed && call.exit_code.is_none() {
                call.exit_code = matches!(call.kind, CallKind::Command { .. }).then_some(0);
            }
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
                let calls = session.turn.calls;
                // Any failure to write (a busy lock, a full disk, an I/O
                // error) drops this turn's calls from the envelope,
                // unrecorded; the turn is still reset below.
                let _ = store.try_update_repo(repo, HOOK_LOCK_WAIT, |state| {
                    state.envelope.add(calls);
                    (true, ())
                });
            }
            session.turn = Default::default();
            store.save_session(session_id, &session).ok()?;
            None
        }
        "SessionEnd" => {
            let session = store
                .session(session_id)
                .filter(|state| state.repo == repo)?;
            let digest = short_digest("session", session_id);
            // Any failure to write (a busy lock, a full disk, an I/O
            // error) drops this session's evidence, unrecorded; its file is
            // removed either way, as nothing would fold it later.
            let _ = store.try_update_repo(repo, HOOK_LOCK_WAIT, |state| {
                state
                    .tools
                    .fold_session(harness, session.prompts, &session.used_tools);
                state.facts.fold_session(&digest, &session.observations);
                state.sessions = state.sessions.saturating_add(1);
                (true, ())
            });
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

/// The event on stdin, read for at most [`BUDGET`].
fn read_input() -> Option<String> {
    String::from_utf8(crate::managed::read_stdin_bounded(BUDGET, MAX_INPUT)?).ok()
}
