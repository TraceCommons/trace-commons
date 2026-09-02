//! Converts the Antigravity API's step list into Trajectory-v1 records, the
//! format the existing `source::trajectory` reader already consumes.
//!
//! Trajectory-v1 has exactly five roles -- `meta`, `user`, `reasoning`,
//! `assistant`, `tool` -- and the reader rejects any other. A step kind that
//! maps onto none of them therefore emits NO record rather than an invented
//! sixth role or an empty `assistant` (which would put a turn in the
//! transcript that never happened). The cost of a step kind Google adds
//! later is that one step, never the conversation: the fixture already
//! carries `CHECKPOINT`, `CONVERSATION_HISTORY`, `KNOWLEDGE_ARTIFACTS` and
//! `SYSTEM_MESSAGE`, and the enum is wider than any one capture.
//!
//! Dropping is only safe for a step whose CALLS are not announced. A step
//! kind that answers an announced tool call must still answer it, or the
//! transcript shows the agent calling a tool and never receiving a result
//! -- a misrepresentation of the conversation, which is the defect this
//! whole design exists to avoid. `no_announced_call_is_left_without_a_result`
//! enforces that directly, so a future step kind in this position fails the
//! suite rather than quietly thinning the trace.
//!
//! **Pairing and extraction are separate decisions.** PAIRING is a rule:
//! any step whose `metadata.toolCall.id` resolves to an announced,
//! unanswered call IS that call's result, whatever its kind is named --
//! `edit_file`, `write_to_file`, `grep_search` and `codebase_search` are
//! everyday Antigravity tools that appear in neither capture, and every
//! real coding session will have them. EXTRACTION stays an enumeration:
//! only a kind listed in [`TOOL_RESULT_STEPS`] contributes vendor text, and
//! only from the paths named there. An unmapped kind answers its call with
//! a fixed marker naming the step-kind enum and nothing else -- no
//! `result`/`output`/`content` guessing, because those keys are untyped and
//! guessing them stages whatever the vendor put there unreviewed, which is
//! exactly the fall-through this module refuses everywhere else.
//!
//! `thinkingSignature` -- present both at the top of a `plannerResponse` and
//! inside each of its `toolCalls` -- is opaque encrypted model internals and
//! is never copied. Nothing is passed through wholesale: a tool CALL is
//! rebuilt field by field (`id`, `name`, `argumentsJson`), and a tool
//! RESULT is built from the paths enumerated in [`TOOL_RESULT_STEPS`]. A
//! field Google adds to either in a later build is therefore staged only
//! once somebody adds it here.
//!
//! **Pairing is by id, not position.** Every tool-result step observed
//! carries `metadata.toolCall.id`, the same id the preceding
//! `PLANNER_RESPONSE` announced; all 19 results in the multi-turn capture
//! resolve exactly, asserted by a test rather than assumed. The positional
//! rule the design assumed is kept only as a fallback for a capture where
//! the id is absent. This matters because the
//! reader fails closed on an orphaned `tool_call_id` and would reject the
//! whole staged file, so a result whose id resolves to nothing is dropped
//! rather than emitted.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::client::TrajectoryDescription;

pub(crate) const ERR_NO_CONTENT: &str = "antigravity-no-content";

const SOURCE_NAME: &str = "antigravity";

/// Step kinds whose tool-result payload may be EXTRACTED, with the field
/// that payload lives under and the exact paths inside it that may be
/// staged. A field not listed within a listed kind contributes nothing.
///
/// This list does not decide what is a tool result -- pairing does, by id,
/// for any step kind. A kind absent here still answers its call, with the
/// fixed marker from [`unmapped_result_marker`] and none of its payload.
///
/// The staged set is an enumeration and never a fall-through. An earlier
/// version serialized the whole payload for the arms with no principal
/// text, which meant a field Google adds to `runCommand` or
/// `listDirectory` in a later build would be staged the day it appeared,
/// with nobody having chosen it. That is the same bet the module doc
/// comment already refuses for tool CALLS, and these fixtures alone turned
/// up four identifier categories nobody had enumerated, so the bet is not
/// hypothetical. Adding a field here is now a decision in code.
///
/// `GENERIC` is the agent's own tool surface -- `manage_task` in the
/// capture -- and belongs here for the same reason as the rest: its calls
/// ARE announced by a `PLANNER_RESPONSE`, so omitting its results would
/// leave the transcript showing the agent calling a tool and never getting
/// an answer. Its path skips `generic.result.stepRenderInfo`, a
/// UI-rendering blob with no place in a trace.
const TOOL_RESULT_STEPS: &[(&str, &str, &[&[&str]])] = &[
    (
        "CORTEX_STEP_TYPE_LIST_DIRECTORY",
        "listDirectory",
        &[&["directoryPathUri"], &["results"]],
    ),
    ("CORTEX_STEP_TYPE_VIEW_FILE", "viewFile", &[&["content"]]),
    (
        "CORTEX_STEP_TYPE_RUN_COMMAND",
        "runCommand",
        // The command and its exit code are the point of the step; the
        // output alone would not say what was run or whether it worked.
        // `taskDetails` is a sibling of the payload, not a member, and so
        // was never in scope -- its `~/.gemini` log URI stays out.
        &[
            &["commandLine"],
            &["cwd"],
            &["exitCode"],
            &["combinedOutput", "full"],
        ],
    ),
    (
        "CORTEX_STEP_TYPE_GENERIC",
        "generic",
        &[&["result", "result"]],
    ),
];

/// Decode a `file://` workspace URI into a filesystem path. The scheme and
/// its authority are stripped and percent-escapes are decoded, because the
/// value feeds the redactor's path-prefix stripping: a still-escaped `%20`
/// would not match the paths appearing in trace content, and the prefix
/// would silently fail to strip.
pub(super) fn cwd_from_workspace_uri(uri: &str) -> Option<String> {
    let path = uri.strip_prefix("file://").unwrap_or(uri);
    // `file:///path` leaves the leading slash; `file://host/path` is not a
    // shape this API produces and is left alone rather than guessed at.
    let decoded = percent_decode(path);
    if decoded.is_empty() {
        None
    } else {
        Some(decoded)
    }
}

/// Minimal percent-decoder. Bytes are decoded, then the result is required
/// to be valid UTF-8; a sequence that is not is left as written rather than
/// replaced, since a mangled path is worse than an unstripped one.
fn percent_decode(raw: &str) -> String {
    if !raw.contains('%') {
        return raw.to_string();
    }
    let bytes = raw.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hi = (bytes[i + 1] as char).to_digit(16);
            let lo = (bytes[i + 2] as char).to_digit(16);
            if let (Some(hi), Some(lo)) = (hi, lo) {
                out.push((hi * 16 + lo) as u8);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| raw.to_string())
}

fn non_empty_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
}

/// The step's own `metadata.createdAt`. Every record the reader accepts
/// needs a timestamp and nothing may be inherited from a neighbour, so a
/// step without one emits no record at all.
fn created_at(step: &Value) -> Option<&str> {
    non_empty_str(step.get("metadata")?, "createdAt")
}

/// The model that generated the conversation, taken from the first step
/// that names one. The first step is a `USER_INPUT`, which has no
/// `generatorModel` at all, so "the first step's" would always be absent.
fn model_of(steps: &[Value]) -> Option<String> {
    steps.iter().find_map(|step| {
        step.get("metadata")
            .and_then(|m| non_empty_str(m, "generatorModel"))
            .map(|s| s.to_string())
    })
}

/// Tool calls announced by an assistant record, in announcement order, with
/// whether a result has already claimed each. Only ids actually emitted are
/// tracked -- pairing against an id that never reached the file would
/// orphan the result and cost the whole conversation.
#[derive(Default)]
struct PendingCalls(Vec<(String, bool)>);

impl PendingCalls {
    fn announce(&mut self, id: &str) {
        self.0.push((id.to_string(), false));
    }

    fn contains(&self, id: &str) -> bool {
        self.0.iter().any(|(known, _)| known == id)
    }

    /// Whether `id` names a call that was announced and is still waiting
    /// for its result. This is the PAIRING rule on its own: a step of an
    /// unmapped kind is that call's result when this is true, and is
    /// dropped when it is not. No positional fallback here -- pairing an
    /// unrecognised step kind by position would attach an arbitrary step to
    /// a call on nothing but ordering.
    fn is_unanswered(&self, id: &str) -> bool {
        self.0
            .iter()
            .any(|(known, answered)| known == id && !*answered)
    }

    /// Claim `id` if it was announced, else fall back to the most recent
    /// unanswered call -- the positional rule, used only when the result
    /// carries no usable id. Returns `None` when neither resolves, which
    /// drops the result rather than orphaning it.
    fn claim(&mut self, id: Option<&str>) -> Option<String> {
        if let Some(id) = id {
            if let Some(entry) = self
                .0
                .iter_mut()
                .rev()
                .find(|(known, answered)| known == id && !*answered)
            {
                entry.1 = true;
                return Some(entry.0.clone());
            }
            // An id that was announced but is already answered is a repeat,
            // not a new pairing; falling through to the positional rule
            // would attach it to an unrelated call.
            if self.contains(id) {
                return None;
            }
        }
        let entry = self.0.iter_mut().rev().find(|(_, answered)| !*answered)?;
        entry.1 = true;
        Some(entry.0.clone())
    }
}

/// Build the `assistant` record announcing a step's tool calls, if it has
/// any usable ones. Returns `None` when every call was unusable: an
/// `assistant` with an empty `tool_calls` array is a malformed record to the
/// reader, not an empty one.
fn tool_call_record(planner: &Value, timestamp: &str, pending: &mut PendingCalls) -> Option<Value> {
    let calls = planner.get("toolCalls")?.as_array()?;
    let mut emitted = Vec::new();
    for call in calls {
        let Some(id) = non_empty_str(call, "id") else {
            continue;
        };
        let Some(name) = non_empty_str(call, "name") else {
            continue;
        };
        // The reader rejects the whole file on a repeated id, so a repeat
        // is dropped here instead. It stays unanswerable, which costs one
        // tool result rather than the conversation.
        if pending.contains(id) {
            continue;
        }
        // Args are a stringified JSON object per the schema; a call without
        // them is announced with an empty object rather than dropped.
        let args = call
            .get("argumentsJson")
            .and_then(|v| v.as_str())
            .unwrap_or("{}");
        pending.announce(id);
        emitted.push(json!({"id": id, "name": name, "args": args}));
    }
    if emitted.is_empty() {
        return None;
    }
    Some(json!({
        "role": "assistant",
        "content": Value::Null,
        "tool_calls": emitted,
        "timestamp": timestamp,
    }))
}

/// Resolve a dotted path within a payload, or `None` if any segment is
/// absent or JSON null.
fn at_path<'a>(payload: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut here = payload;
    for key in path {
        here = here.get(key)?;
    }
    if here.is_null() { None } else { Some(here) }
}

/// The text a tool result contributes, built from the enumerated `paths`
/// and nothing else. A single path resolving to a string is used verbatim
/// -- that is a file's contents or a tool's own answer, and wrapping it in
/// JSON would only obscure it. Anything else is a JSON object holding the
/// paths that resolved, keyed by their dotted names.
///
/// `None` when no path resolves at all. That drops the result rather than
/// staging an unenumerated payload, which is the fail-closed direction: a
/// renamed field costs one tool output, where a fall-through would stage
/// whatever replaced it sight unseen.
fn tool_result_content(payload: &Value, paths: &[&[&str]]) -> Option<String> {
    if let [single] = paths {
        if let Some(Value::String(text)) = at_path(payload, single) {
            if !text.is_empty() {
                return Some(text.clone());
            }
        }
    }
    let mut kept = serde_json::Map::new();
    for path in paths {
        if let Some(value) = at_path(payload, path) {
            kept.insert(path.join("."), value.clone());
        }
    }
    if kept.is_empty() {
        return None;
    }
    serde_json::to_string(&Value::Object(kept)).ok()
}

/// The content staged for a step that answers an announced call but whose
/// kind is not in [`TOOL_RESULT_STEPS`]: a fixed marker naming the
/// step-kind enum and carrying none of the step's payload. The call is
/// answered -- the transcript never shows a call going unanswered -- and
/// nobody's unreviewed vendor fields are staged to do it.
///
/// The kind string arrives from a process this command does not own, and
/// lands in trace content, so it is bounded to the shape the enum actually
/// has (`[A-Z_]{0,64}`) and replaced with `unknown` when it is anything
/// else. Nothing else from the step is used.
fn unmapped_result_marker(step_type: &str) -> String {
    let bounded = step_type.len() <= 64
        && step_type
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_');
    let kind = if bounded { step_type } else { "unknown" };
    format!("[unmapped tool result: {kind}]")
}

/// Convert one API step-list document into Trajectory-v1 records.
pub(crate) fn to_trajectory_v1(steps: &Value, desc: &TrajectoryDescription) -> Result<Vec<Value>> {
    let steps: &[Value] = steps
        .get("steps")
        .and_then(|v| v.as_array())
        .map(|v| v.as_slice())
        .unwrap_or(&[]);

    let cwd = desc
        .workspace_uri
        .as_deref()
        .and_then(cwd_from_workspace_uri);
    let mut out = vec![json!({
        "role": "meta",
        "source": SOURCE_NAME,
        "cwd": cwd,
        "model": model_of(steps),
    })];

    let mut pending = PendingCalls::default();
    let mut saw_turn = false;

    for step in steps {
        let step_type = step.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let Some(timestamp) = created_at(step) else {
            continue;
        };

        if step_type == "CORTEX_STEP_TYPE_USER_INPUT" {
            let Some(content) = step
                .get("userInput")
                .and_then(|u| non_empty_str(u, "userResponse"))
            else {
                continue;
            };
            out.push(json!({
                "role": "user",
                "content": content,
                "timestamp": timestamp,
            }));
            saw_turn = true;
            continue;
        }

        if step_type == "CORTEX_STEP_TYPE_PLANNER_RESPONSE" {
            let Some(planner) = step.get("plannerResponse") else {
                continue;
            };
            // Reasoning first: it is what the model did before speaking or
            // calling, and one step can carry both.
            if let Some(thinking) = non_empty_str(planner, "thinking") {
                out.push(json!({
                    "role": "reasoning",
                    "content": thinking,
                    "timestamp": timestamp,
                }));
            }
            if let Some(response) = non_empty_str(planner, "response") {
                out.push(json!({
                    "role": "assistant",
                    "content": response,
                    "timestamp": timestamp,
                }));
                saw_turn = true;
            }
            // A record may not carry both content and tool calls, so the
            // calls go in their own record. Not observed together, but the
            // shape is cheap to honour and expensive to get wrong.
            if let Some(record) = tool_call_record(planner, timestamp, &mut pending) {
                out.push(record);
                saw_turn = true;
            }
            continue;
        }

        let announced = step
            .get("metadata")
            .and_then(|m| m.get("toolCall"))
            .and_then(|c| non_empty_str(c, "id"));
        let mapped = TOOL_RESULT_STEPS
            .iter()
            .find(|(name, _, _)| *name == step_type);
        // An unmapped kind is a tool result when, and only when, it names
        // an announced call still waiting for one.
        let answers_a_call =
            mapped.is_none() && announced.is_some_and(|id| pending.is_unanswered(id));

        if mapped.is_some() || answers_a_call {
            let content = match mapped {
                Some((_, payload_field, principal)) => {
                    let Some(payload) = step.get(payload_field) else {
                        continue;
                    };
                    let Some(content) = tool_result_content(payload, principal) else {
                        continue;
                    };
                    content
                }
                None => unmapped_result_marker(step_type),
            };
            let Some(call_id) = pending.claim(announced) else {
                continue;
            };
            out.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": content,
                "timestamp": timestamp,
            }));
            continue;
        }

        // Every other step kind, recognised or not, emits nothing.
    }

    if !saw_turn {
        return Err(anyhow!(ERR_NO_CONTENT));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::antigravity::client::{AntigravityApi, FixtureApi, desc_fixture};
    use crate::source::{SessionEvent, SessionEventKind};

    async fn convert(cascade_id: &str) -> Vec<Value> {
        let doc = FixtureApi::new().fetch_steps(cascade_id).await.unwrap();
        to_trajectory_v1(&doc, &desc_fixture()).expect("must convert")
    }

    #[tokio::test]
    async fn every_user_turn_becomes_its_own_event_in_conversation_order() {
        let out = convert("multi-turn").await;

        let user_positions: Vec<usize> = out
            .iter()
            .enumerate()
            .filter(|(_, r)| r["role"] == "user")
            .map(|(i, _)| i)
            .collect();
        assert_eq!(
            user_positions.len(),
            2,
            "two user turns in, two out -- never collapsed"
        );
        assert!(
            user_positions[1] > user_positions[0] + 1,
            "the second user turn must be interleaved after the first turn's agent work, \
             not adjacent to it -- front-loading is the defect this design replaced"
        );
    }

    #[tokio::test]
    async fn each_user_turn_carries_its_own_real_timestamp() {
        let out = convert("multi-turn").await;
        let ts: Vec<&str> = out
            .iter()
            .filter(|r| r["role"] == "user")
            .map(|r| {
                r["timestamp"]
                    .as_str()
                    .expect("a user turn has a timestamp")
            })
            .collect();
        assert_eq!(ts.len(), 2);
        assert!(
            ts[0].starts_with("2026-08-29"),
            "first turn keeps its own day"
        );
        assert!(
            ts[1].starts_with("2026-08-31"),
            "second turn keeps its own day"
        );
        assert_ne!(ts[0], ts[1], "no timestamp inherited from a neighbour");
    }

    #[tokio::test]
    async fn nothing_from_the_model_internals_reaches_the_output() {
        let out = convert("multi-turn").await;
        let rendered = serde_json::to_string(&out).unwrap();
        assert!(
            !rendered.contains("REDACTED-THINKING-SIGNATURE"),
            "thinkingSignature is opaque model internals and must not be carried"
        );
        assert!(!rendered.contains("thinkingSignature"));
        // A `GENERIC` result is taken from `generic.result.result`, so the
        // sibling UI-rendering blob does not ride along with it.
        assert!(!rendered.contains("stepRenderInfo"));
    }

    #[tokio::test]
    async fn an_unrecognised_step_type_emits_no_record_and_does_not_fail_the_conversation() {
        let mut doc = FixtureApi::new().fetch_steps("multi-turn").await.unwrap();
        doc["steps"].as_array_mut().unwrap().push(json!({
            "type": "CORTEX_STEP_TYPE_SOMETHING_GOOGLE_ADDED_LATER",
            "metadata": {"createdAt": "2026-09-01T00:00:00Z"}
        }));
        let out = to_trajectory_v1(&doc, &desc_fixture())
            .expect("an unknown step type must not fail the conversation");
        let baseline = convert("multi-turn").await;
        assert_eq!(
            out.len(),
            baseline.len(),
            "an unknown step emits no record -- trajectory-v1 has no role for one"
        );
    }

    #[test]
    fn a_conversation_with_no_user_or_assistant_content_is_refused() {
        let doc = json!({"steps": []});
        let err = to_trajectory_v1(&doc, &desc_fixture())
            .unwrap_err()
            .to_string();
        assert_eq!(err, "antigravity-no-content");
    }

    #[tokio::test]
    async fn the_output_round_trips_through_the_trajectory_reader() {
        let out = convert("multi-turn").await;
        let bytes = serde_json::to_vec(&out).unwrap();

        let parsed = crate::source::trajectory::parse_trajectory(&bytes)
            .expect("the trajectory reader must accept what we write");
        assert_eq!(parsed.source, "antigravity");
        let users: Vec<&SessionEvent> = parsed
            .events
            .iter()
            .filter(|e| e.kind == SessionEventKind::User)
            .collect();
        assert_eq!(users.len(), 2, "both turns survive the round trip");

        // Count alone would pass on a reversed pair, and gate criterion 4
        // is order as well as count. The two turns are distinguishable by
        // content and by day, so both are asserted.
        assert_eq!(users[0].content.as_deref(), Some("Tell me about this repo"));
        assert_eq!(
            users[1].content.as_deref(),
            Some("What should we more on next?")
        );
        let days: Vec<String> = users
            .iter()
            .map(|e| {
                e.timestamp
                    .expect("a real timestamp")
                    .date_naive()
                    .to_string()
            })
            .collect();
        assert_eq!(days, vec!["2026-08-29", "2026-08-31"]);

        // The turns must also sit in the surrounding work rather than at
        // the front: the API's own step order is the only ordering applied.
        let positions: Vec<usize> = parsed
            .events
            .iter()
            .enumerate()
            .filter(|(_, e)| e.kind == SessionEventKind::User)
            .map(|(i, _)| i)
            .collect();
        assert_eq!(positions[0], 0);
        assert!(
            positions[1] > 20,
            "the first turn's agent work must lie between them, not after both"
        );

        // The whole parsed sequence, not just its user turns. Any silent
        // reordering, insertion or loss anywhere in the transcript moves
        // this and fails here.
        let kinds: Vec<&SessionEventKind> = parsed.events.iter().map(|e| &e.kind).collect();
        let expected_head = [
            SessionEventKind::User,
            SessionEventKind::ToolCall,
            SessionEventKind::ToolResult,
            SessionEventKind::ToolCall,
            SessionEventKind::ToolResult,
        ];
        assert!(
            kinds
                .iter()
                .zip(expected_head.iter())
                .all(|(got, want)| *got == want),
            "the transcript opens with the user turn and alternating call/result"
        );
        assert_eq!(
            kinds
                .iter()
                .filter(|k| ***k == SessionEventKind::ToolCall)
                .count(),
            kinds
                .iter()
                .filter(|k| ***k == SessionEventKind::ToolResult)
                .count(),
            "every call is answered, so the two counts match"
        );
    }

    #[tokio::test]
    async fn a_conversation_that_gains_turns_stages_different_bytes() {
        let before = convert("single-turn").await;
        let after = convert("multi-turn").await;

        let a = serde_json::to_vec(&before).unwrap();
        let b = serde_json::to_vec(&after).unwrap();
        assert_ne!(
            crate::source::session_hash(&a),
            crate::source::session_hash(&b),
            "a conversation that gained a turn must not be suppressed as a duplicate \
             of its earlier self -- the later turns would never be collected"
        );
    }

    /// The design's open question, answered against the capture: every tool
    /// result names its own call in `metadata.toolCall.id`, so pairing is
    /// exact rather than positional. Asserted, not assumed, because the
    /// reader rejects the whole file on one orphan.
    #[tokio::test]
    async fn every_tool_result_pairs_with_the_call_its_own_metadata_names() {
        let doc = FixtureApi::new().fetch_steps("multi-turn").await.unwrap();
        let expected: Vec<String> = doc["steps"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|s| {
                TOOL_RESULT_STEPS
                    .iter()
                    .any(|(name, _, _)| Some(*name) == s["type"].as_str())
            })
            .map(|s| {
                s["metadata"]["toolCall"]["id"]
                    .as_str()
                    .unwrap()
                    .to_string()
            })
            .collect();
        assert_eq!(expected.len(), 19, "the capture's tool-result steps");

        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        let paired: Vec<String> = out
            .iter()
            .filter(|r| r["role"] == "tool")
            .map(|r| r["tool_call_id"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(
            paired, expected,
            "each result carries the id its own metadata names, in order"
        );
    }

    /// The stronger invariant behind the per-type mapping: an announced
    /// call with no result shows the agent calling a tool and never
    /// receiving an answer, which misrepresents the conversation just as
    /// surely as a mis-ordered turn. A step kind whose results are dropped
    /// while its calls are announced fails HERE, without anyone having to
    /// notice the kind exists.
    #[tokio::test]
    async fn no_announced_call_is_left_without_a_result() {
        let out = convert("multi-turn").await;
        let announced: Vec<&str> = out
            .iter()
            .filter_map(|r| r["tool_calls"].as_array())
            .flatten()
            .filter_map(|c| c["id"].as_str())
            .collect();
        let answered: Vec<&str> = out
            .iter()
            .filter(|r| r["role"] == "tool")
            .filter_map(|r| r["tool_call_id"].as_str())
            .collect();
        assert!(!announced.is_empty());
        let unanswered: Vec<&&str> = announced
            .iter()
            .filter(|id| !answered.contains(id))
            .collect();
        assert!(
            unanswered.is_empty(),
            "every announced call must receive a result: {unanswered:?} did not"
        );
    }

    /// A synthesized conversation: one planner step announcing `call-1`,
    /// then a step of `step_type` claiming to answer it.
    fn one_call_answered_by(step_type: &str, payload: Value) -> Value {
        let mut step = json!({
            "type": step_type,
            "metadata": {"createdAt": "2026-09-01T00:00:01Z",
                         "toolCall": {"id": "call-1"}},
        });
        if let Some(obj) = payload.as_object() {
            for (k, v) in obj {
                step[k.as_str()] = v.clone();
            }
        }
        json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "plannerResponse": {"toolCalls": [
                 {"id": "call-1", "name": "edit_file", "argumentsJson": "{}"}
             ]}},
            step,
        ]})
    }

    fn tool_records(out: &[Value]) -> Vec<&Value> {
        out.iter().filter(|r| r["role"] == "tool").collect()
    }

    /// Antigravity's everyday tools -- `edit_file`, `write_to_file`,
    /// `grep_search`, `codebase_search` -- are in neither capture, so their
    /// step kinds are unmapped. Their calls ARE announced, so dropping
    /// their results would show the agent calling `edit_file` and receiving
    /// nothing. The call is answered by a marker, and the vendor payload
    /// stays out of the staged file entirely.
    #[test]
    fn an_unmapped_kind_answers_its_call_without_staging_its_payload() {
        let doc = one_call_answered_by(
            "CORTEX_STEP_TYPE_EDIT_FILE",
            json!({"editFile": {"result": "SECRET_PAYLOAD", "diff": "SECRET_DIFF"}}),
        );
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        let tool = tool_records(&out);
        assert_eq!(tool.len(), 1, "the announced call must be answered");
        assert_eq!(tool[0]["tool_call_id"], "call-1");
        assert_eq!(
            tool[0]["content"],
            "[unmapped tool result: CORTEX_STEP_TYPE_EDIT_FILE]"
        );
        let whole = serde_json::to_string(&out).unwrap();
        assert!(
            !whole.contains("SECRET_PAYLOAD") && !whole.contains("SECRET_DIFF"),
            "an unmapped payload is never staged: {whole}"
        );
    }

    /// The step kind lands in trace content and arrives from a process this
    /// command does not own, so it is bounded to the enum's own shape.
    #[test]
    fn the_marker_sanitizes_a_hostile_step_kind() {
        let long = "A".repeat(65);
        for hostile in [
            "CORTEX\n{\"role\":\"user\"}",
            "cortex_step_type_lowercase",
            "A/../../etc/passwd",
            long.as_str(),
        ] {
            let doc = one_call_answered_by(hostile, json!({}));
            let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
            let tool = tool_records(&out);
            assert_eq!(tool.len(), 1);
            assert_eq!(
                tool[0]["content"], "[unmapped tool result: unknown]",
                "a kind outside the enum's shape must not reach the marker: {hostile}"
            );
        }
        // A well-formed kind at the length boundary is still named.
        let boundary = "A".repeat(64);
        let doc = one_call_answered_by(&boundary, json!({}));
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        assert_eq!(
            tool_records(&out)[0]["content"],
            format!("[unmapped tool result: {boundary}]")
        );
    }

    /// Pairing an unmapped kind is by id ONLY. A step naming no announced
    /// call is not a tool result, and must not claim the most recent
    /// unanswered call by position -- that would attach an arbitrary step
    /// to a call on nothing but ordering.
    #[test]
    fn an_unmapped_kind_naming_no_announced_call_is_dropped() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "plannerResponse": {"response": "working on it", "toolCalls": [
                 {"id": "call-1", "name": "edit_file", "argumentsJson": "{}"}
             ]}},
            {"type": "CORTEX_STEP_TYPE_CHECKPOINT",
             "metadata": {"createdAt": "2026-09-01T00:00:01Z"}},
            {"type": "CORTEX_STEP_TYPE_EDIT_FILE",
             "metadata": {"createdAt": "2026-09-01T00:00:02Z",
                          "toolCall": {"id": "never-announced"}}},
        ]});
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        assert!(
            tool_records(&out).is_empty(),
            "neither step names an announced call, so neither is a result"
        );
    }

    /// The staged set of a tool result is an enumeration, not whatever the
    /// payload happens to hold. A field a later Antigravity build adds to a
    /// payload we already map must not ride along uninspected.
    #[tokio::test]
    async fn a_field_added_to_a_mapped_payload_is_not_staged() {
        let mut doc = FixtureApi::new().fetch_steps("multi-turn").await.unwrap();
        let mut touched = 0;
        for step in doc["steps"].as_array_mut().unwrap() {
            for field in ["runCommand", "listDirectory", "viewFile", "generic"] {
                if let Some(payload) = step.get_mut(field).and_then(|p| p.as_object_mut()) {
                    payload.insert(
                        "fieldGoogleAddedLater".to_string(),
                        json!("SENTINEL-MUST-NOT-BE-STAGED"),
                    );
                    touched += 1;
                }
            }
        }
        assert_eq!(touched, 19, "every mapped tool-result payload was seeded");

        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        let rendered = serde_json::to_string(&out).unwrap();
        assert!(!rendered.contains("SENTINEL-MUST-NOT-BE-STAGED"));
        assert!(!rendered.contains("fieldGoogleAddedLater"));
        // And the seeding did not simply suppress the results themselves.
        assert_eq!(out.iter().filter(|r| r["role"] == "tool").count(), 19);
    }

    /// The other half of the same rule: a payload whose enumerated paths
    /// have all been renamed away stages nothing, rather than falling
    /// through to whatever replaced them.
    #[test]
    fn a_payload_with_no_enumerated_path_left_stages_nothing() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_USER_INPUT",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "userInput": {"userResponse": "go"}},
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:01Z"},
             "plannerResponse": {"toolCalls": [
                 {"id": "c1", "name": "run_command", "argumentsJson": "{}"}]}},
            {"type": "CORTEX_STEP_TYPE_RUN_COMMAND",
             "metadata": {"createdAt": "2026-09-01T00:00:02Z",
                          "toolCall": {"id": "c1"}},
             "runCommand": {"renamedOutput": "SENTINEL-MUST-NOT-BE-STAGED"}},
        ]});
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        assert!(
            !serde_json::to_string(&out)
                .unwrap()
                .contains("SENTINEL-MUST-NOT-BE-STAGED")
        );
        assert!(out.iter().all(|r| r["role"] != "tool"));
    }

    /// A `runCommand` result keeps the command and its exit code beside the
    /// output -- the output alone would not say what ran or whether it
    /// worked -- and keeps nothing else.
    #[tokio::test]
    async fn a_run_command_result_stages_exactly_the_enumerated_fields() {
        let out = convert("multi-turn").await;
        let staged: Vec<serde_json::Map<String, Value>> = out
            .iter()
            .filter(|r| r["role"] == "tool")
            .filter_map(|r| r["content"].as_str())
            .filter_map(|c| serde_json::from_str::<Value>(c).ok())
            .filter_map(|v| match v {
                Value::Object(map) if map.contains_key("commandLine") => Some(map),
                _ => None,
            })
            .collect();
        assert_eq!(staged.len(), 5, "the capture's run_command results");
        for map in &staged {
            let mut keys: Vec<&str> = map.keys().map(|k| k.as_str()).collect();
            keys.sort();
            assert!(
                keys.iter()
                    .all(|k| ["combinedOutput.full", "commandLine", "cwd", "exitCode"].contains(k)),
                "unexpected staged key in {keys:?}"
            );
            assert!(map.contains_key("commandLine"));
            assert!(map.contains_key("combinedOutput.full"));
        }
    }

    /// Enumerating paths buys fail-closed staging at the cost of a silent
    /// failure mode: if the vendor renames a field, its path stops
    /// resolving, the result thins or disappears, and no other test here
    /// notices -- they all run against fixtures where the paths do resolve.
    ///
    /// Nothing can detect that rename today. What this DOES is make it fail
    /// loudly the moment somebody re-captures the fixtures, which is when a
    /// schema change actually enters the repo. The fixtures on this branch
    /// have already been re-captured once, so the trigger is real.
    ///
    /// It asserts more than "content is non-empty", deliberately. If
    /// `combinedOutput.full` alone were renamed, a `runCommand` result
    /// would still stage `commandLine`/`cwd`/`exitCode` and pass a
    /// non-emptiness check while having lost the command's entire output.
    /// So the OUTPUT-bearing path of each type is checked specifically.
    ///
    /// `exitCode` and `cwd` are deliberately NOT required: proto3 omits a
    /// zero `exitCode` entirely (fixture step 44 has none), so requiring
    /// every enumerated path to resolve would fail on a successful command.
    #[tokio::test]
    async fn every_mapped_step_in_the_fixtures_still_stages_its_output() {
        // The path carrying each type's actual tool output, as opposed to
        // the context fields staged beside it.
        let output_path: &[(&str, &str)] = &[
            ("CORTEX_STEP_TYPE_LIST_DIRECTORY", "results"),
            ("CORTEX_STEP_TYPE_VIEW_FILE", "content"),
            ("CORTEX_STEP_TYPE_RUN_COMMAND", "combinedOutput.full"),
            ("CORTEX_STEP_TYPE_GENERIC", "result.result"),
        ];
        const WHY: &str = "a mapped tool-result step staged no output. The likely cause is \
            that Antigravity renamed a field TOOL_RESULT_STEPS enumerates, so its path no \
            longer resolves. The fix is to update the path list to the new field name -- \
            NOT to relax this assertion, and NOT to restore a whole-payload fall-through, \
            which would stage the replacement field sight unseen.";

        for fixture in ["multi-turn", "single-turn"] {
            let doc = FixtureApi::new().fetch_steps(fixture).await.unwrap();
            let mapped: Vec<&Value> = doc["steps"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|s| {
                    TOOL_RESULT_STEPS
                        .iter()
                        .any(|(name, _, _)| Some(*name) == s["type"].as_str())
                })
                .collect();
            assert!(!mapped.is_empty(), "{fixture} has mapped tool-result steps");

            let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
            let results: Vec<&Value> = out.iter().filter(|r| r["role"] == "tool").collect();
            assert_eq!(
                results.len(),
                mapped.len(),
                "{fixture}: every mapped step must stage a tool record. {WHY}"
            );

            for (step, record) in mapped.iter().zip(results.iter()) {
                let step_type = step["type"].as_str().unwrap();
                let content = record["content"].as_str().expect("content is a string");
                assert!(
                    !content.is_empty(),
                    "{fixture}: {step_type} staged empty content. {WHY}"
                );
                let key = output_path
                    .iter()
                    .find(|(name, _)| *name == step_type)
                    .map(|(_, key)| *key)
                    .expect("every mapped type names its output path here");
                // A multi-path type always stages a JSON object keyed by
                // dotted path, so its output key must be present. A
                // single-path type stages that path's string verbatim, and
                // a rename there resolves to nothing at all -- dropping the
                // record, which the count assertion above already catches.
                let staged_output = match serde_json::from_str::<Value>(content) {
                    Ok(Value::Object(map)) => map.contains_key(key),
                    _ => true,
                };
                assert!(
                    staged_output,
                    "{fixture}: {step_type} staged no `{key}`. {WHY}"
                );
            }
        }
    }

    /// The positional fallback, for a capture where a result names no call.
    #[test]
    fn a_result_with_no_id_falls_back_to_the_most_recent_unanswered_call() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_USER_INPUT",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "userInput": {"userResponse": "go"}},
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:01Z"},
             "plannerResponse": {"toolCalls": [
                 {"id": "c1", "name": "view_file", "argumentsJson": "{}"}]}},
            {"type": "CORTEX_STEP_TYPE_VIEW_FILE",
             "metadata": {"createdAt": "2026-09-01T00:00:02Z"},
             "viewFile": {"content": "hello"}},
        ]});
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        let tool = out.iter().find(|r| r["role"] == "tool").expect("a result");
        assert_eq!(tool["tool_call_id"], "c1");
        assert_eq!(tool["content"], "hello");
    }

    /// A result naming a call that was never announced would orphan and,
    /// through the reader's fail-closed orphan check, cost the whole
    /// conversation. It is dropped instead.
    #[test]
    fn a_result_naming_an_unannounced_call_is_dropped_not_orphaned() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_USER_INPUT",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "userInput": {"userResponse": "go"}},
            {"type": "CORTEX_STEP_TYPE_VIEW_FILE",
             "metadata": {"createdAt": "2026-09-01T00:00:02Z",
                          "toolCall": {"id": "never-announced"}},
             "viewFile": {"content": "hello"}},
        ]});
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        assert!(out.iter().all(|r| r["role"] != "tool"));
        let bytes = serde_json::to_vec(&out).unwrap();
        crate::source::trajectory::parse_trajectory(&bytes)
            .expect("the reader must still accept the file");
    }

    /// The reader rejects a repeated `tool_call_id` outright, so a repeat is
    /// dropped at announcement rather than allowed to reach the file.
    #[test]
    fn a_repeated_tool_call_id_does_not_reach_the_file() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_USER_INPUT",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"},
             "userInput": {"userResponse": "go"}},
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:01Z"},
             "plannerResponse": {"toolCalls": [
                 {"id": "c1", "name": "view_file", "argumentsJson": "{}"}]}},
            {"type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
             "metadata": {"createdAt": "2026-09-01T00:00:03Z"},
             "plannerResponse": {"toolCalls": [
                 {"id": "c1", "name": "view_file", "argumentsJson": "{}"}]}},
        ]});
        let out = to_trajectory_v1(&doc, &desc_fixture()).unwrap();
        let calls: usize = out
            .iter()
            .filter_map(|r| r["tool_calls"].as_array())
            .map(|c| c.len())
            .sum();
        assert_eq!(calls, 1);
        let bytes = serde_json::to_vec(&out).unwrap();
        crate::source::trajectory::parse_trajectory(&bytes)
            .expect("the reader must still accept the file");
    }

    /// A `PLANNER_RESPONSE` carrying both thinking and a spoken response is
    /// the fixture's own shape at steps 22, 41 and 47; both must survive,
    /// reasoning first.
    #[tokio::test]
    async fn thinking_and_response_from_one_step_become_two_records_in_order() {
        let out = convert("multi-turn").await;
        let roles: Vec<&str> = out.iter().filter_map(|r| r["role"].as_str()).collect();
        let reasoning = roles.iter().filter(|r| **r == "reasoning").count();
        assert_eq!(reasoning, 4, "the capture's thinking steps");
        let assistants = roles.iter().filter(|r| **r == "assistant").count();
        assert_eq!(assistants, 22, "19 tool-call turns plus 3 spoken responses");
    }

    #[test]
    fn a_workspace_uri_becomes_a_decoded_filesystem_path() {
        assert_eq!(
            cwd_from_workspace_uri("file:///Users/a/My%20Code").as_deref(),
            Some("/Users/a/My Code")
        );
        assert_eq!(
            cwd_from_workspace_uri("file:///plain/path").as_deref(),
            Some("/plain/path")
        );
        // A malformed escape is left as written rather than mangled.
        assert_eq!(
            cwd_from_workspace_uri("file:///a/100%done").as_deref(),
            Some("/a/100%done")
        );
    }

    #[tokio::test]
    async fn the_meta_record_names_the_model_the_first_step_does_not_carry() {
        let out = convert("multi-turn").await;
        assert_eq!(out[0]["role"], "meta");
        assert_eq!(out[0]["source"], "antigravity");
        assert_eq!(out[0]["model"], "MODEL_PLACEHOLDER_M298");
        assert_eq!(out[0]["cwd"], "/Users/anonymized/code/trace-commons-server");
        // No `git_branch`: `source::trajectory` deliberately drops it, and
        // emitting a field the reader discards only invites somebody to
        // believe it survives into the trace.
        assert!(out[0].get("git_branch").is_none());
    }

    /// A conversation of nothing but dropped step kinds has no turn to
    /// stage, and must be refused rather than staged as a bare meta record.
    #[test]
    fn a_conversation_of_only_dropped_steps_is_refused() {
        let doc = json!({"steps": [
            {"type": "CORTEX_STEP_TYPE_CHECKPOINT",
             "metadata": {"createdAt": "2026-09-01T00:00:00Z"}},
        ]});
        let err = to_trajectory_v1(&doc, &desc_fixture())
            .unwrap_err()
            .to_string();
        assert_eq!(err, ERR_NO_CONTENT);
    }
}
