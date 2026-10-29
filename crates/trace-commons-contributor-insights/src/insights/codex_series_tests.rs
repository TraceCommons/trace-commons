use serde_json::{Value, json};
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, DigestKey, DigestKeyStore, InMemoryDigestKeyStore, MAX_SERIES_TURNS, ToolKind,
};

use super::*;
use crate::insights::session_drill::{SeriesUnavailable, session_drill};
use crate::insights::session_facts::EditMode;
use crate::insights::week_glance::dated_weeks;
use crate::insights::week_patterns::week_patterns;
use crate::insights::week_rollup::{CodexObserved, Feed, SessionBody, SessionInput, week_rollup};

const A: &str = "model-a";
const B: &str = "model-b";

fn key() -> DigestKey {
    InMemoryDigestKeyStore::with_seed([9; 32])
        .load_or_create()
        .unwrap()
}

fn ts(secs: u32) -> String {
    format!(
        "2026-01-05T10:{:02}:{:02}.000Z",
        (secs / 60) % 60,
        secs % 60
    )
}

fn started(secs: u32) -> Value {
    json!({"timestamp": ts(secs), "type": "event_msg", "payload": {"type": "task_started"}})
}

fn complete(secs: u32) -> Value {
    json!({"timestamp": ts(secs), "type": "event_msg", "payload": {"type": "task_complete"}})
}

fn aborted(secs: u32, reason: &str) -> Value {
    json!({"timestamp": ts(secs), "type": "event_msg",
        "payload": {"type": "turn_aborted", "reason": reason}})
}

fn context(secs: u32, model: &str, policy: &str) -> Value {
    json!({"timestamp": ts(secs), "type": "turn_context", "payload": {
        "model": model, "approval_policy": policy, "approvals_reviewer": "auto_review"}})
}

fn usage(input: u64, cached: u64, write: u64, output: u64) -> Value {
    json!({"input_tokens": input, "cached_input_tokens": cached,
        "cache_write_input_tokens": write, "output_tokens": output,
        "reasoning_output_tokens": 0, "total_tokens": input + output})
}

/// A `token_count` event whose running total is `total` and whose last
/// response used the given counters.
fn tokens(secs: u32, total: u64, input: u64, cached: u64, write: u64, output: u64) -> Value {
    json!({"timestamp": ts(secs), "type": "event_msg", "payload": {"type": "token_count",
        "info": {"total_token_usage": {"total_tokens": total},
            "last_token_usage": usage(input, cached, write, output)}}})
}

fn patch_call(secs: u32, id: &str, patch: &str) -> Value {
    json!({"timestamp": ts(secs), "type": "response_item", "payload": {
        "type": "custom_tool_call", "name": "apply_patch", "call_id": id, "input": patch}})
}

fn function_call(secs: u32, id: &str, name: &str, arguments: Value) -> Value {
    json!({"timestamp": ts(secs), "type": "response_item", "payload": {
        "type": "function_call", "name": name, "call_id": id,
        "arguments": arguments.to_string()}})
}

fn shell_call(secs: u32, id: &str, cmd: &str) -> Value {
    function_call(secs, id, "exec_command", json!({"cmd": cmd}))
}

fn output(secs: u32, id: &str, text: &str) -> Value {
    json!({"timestamp": ts(secs), "type": "response_item", "payload": {
        "type": "function_call_output", "call_id": id, "output": text}})
}

fn update(path: &str, old: &str, new: &str) -> String {
    format!("*** Begin Patch\n*** Update File: {path}\n@@\n-{old}\n+{new}\n*** End Patch\n")
}

fn scan(rows: &[Value]) -> (CodexTurnSeriesEvidence, SessionFacts) {
    let key = key();
    let mut scan = CodexTurnSeriesScan::new(&key);
    for row in rows {
        scan.observe(row).unwrap();
    }
    let (evidence, facts) = scan.finish().unwrap();
    evidence.validate().unwrap();
    (evidence, facts)
}

fn fixture_rows() -> Vec<Value> {
    let bytes = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../trace-commons-contributor/fixtures/insights/codex-turn-series/session.jsonl"),
    )
    .unwrap();
    String::from_utf8(bytes)
        .unwrap()
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn sum_total(evidence: &CodexTurnSeriesEvidence) -> u64 {
    evidence
        .series
        .turns
        .iter()
        .map(|turn| turn.known_usage().unwrap().total())
        .sum()
}

fn model<'a>(
    evidence: &CodexTurnSeriesEvidence,
    facts: &'a SessionFacts,
    label: &str,
) -> &'a ModelFacts {
    let ix = evidence
        .model_labels
        .iter()
        .position(|known| known == label)
        .unwrap();
    facts
        .models
        .iter()
        .find(|m| usize::from(m.label_ix) == ix)
        .unwrap()
}

#[test]
fn repeated_token_counts_make_one_turn_per_increase() {
    let rows = fixture_rows();
    let final_total = 4_800;
    let (e, f) = scan(&rows);
    // Six token_count events with info, three increases.
    assert_eq!(e.series.turns.len(), 3);
    assert_eq!(sum_total(&e), final_total);
    assert_eq!(e.model_labels, vec![A.to_string()]);
    assert!(e.series.turns.iter().all(|t| t.model_label_ix == Some(0)));
    assert_eq!(
        e.series.turns.iter().map(|t| t.ordinal).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    // Each call sits on the turn whose response made it.
    assert_eq!(
        e.series
            .tool_calls
            .iter()
            .map(|c| (c.tool, c.turn_ordinal, c.paired, c.success))
            .collect::<Vec<_>>(),
        vec![
            (ToolKind::Bash, 0, true, Some(false)),
            (ToolKind::ApplyPatch, 1, true, Some(true)),
            (ToolKind::Bash, 2, true, Some(true)),
        ]
    );
    let a = model(&e, &f, A);
    assert_eq!(a.edits, Some(1));
    assert_eq!(a.denied, None);
    assert_eq!(f.user_turns, Some(1));
    assert_eq!(f.finished, Some(true));
    assert_eq!(f.active_secs, Some(41));
    assert_eq!(f.edit_mode, EditMode::NotPrompting);
    assert_eq!(f.shape.edited_files.source, 1);
    assert_eq!(f.shape.lines_added, Some(1));
    assert_eq!(f.shape.lines_removed, Some(1));
    assert_eq!(f.shape.failed_before_first_edit_then_passed, 1);
    assert!(!f.capped && !f.reviewer);
}

#[test]
fn a_codex_turn_total_is_input_plus_output_never_the_write_twice() {
    let (e, _) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, 1_500, 1_000, 400, 100, 500),
    ]);
    let usage = e.series.turns[0].known_usage().unwrap();
    assert_eq!(usage.context(), 1_000);
    assert_eq!(usage.total(), 1_000 + 500);
}

#[test]
fn a_decreasing_total_makes_that_turn_unknown_never_negative() {
    let (e, _) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, 1_000, 900, 0, 0, 100),
        tokens(2, 400, 300, 0, 0, 100),
        tokens(3, 400, 300, 0, 0, 100),
        tokens(4, 700, 250, 0, 0, 50),
    ]);
    let turns = &e.series.turns;
    assert_eq!(turns.len(), 3);
    assert!(turns[0].known_usage().is_some());
    assert!(turns[1].usage_unknown());
    assert_eq!(turns[1].uncached_input, None);
    assert_eq!(turns[1].output, None);
    // The baseline was reset to 400, so 700 is an increase.
    assert_eq!(turns[2].known_usage().unwrap().total(), 300);
}

/// A turn's counters must add up to what the running total grew by: when
/// `last_token_usage` and the `total_token_usage` delta disagree, the turn's
/// usage is unknown, never a silent partial.
#[test]
fn a_last_usage_that_disagrees_with_the_total_delta_is_unknown() {
    let (e, _) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, 1_000, 900, 0, 0, 100),
        // The total grew by 800; the last response claims 300.
        tokens(2, 1_800, 250, 0, 0, 50),
        // The total grew by 300 and the last response says 300.
        tokens(3, 2_100, 250, 0, 0, 50),
    ]);
    let turns = &e.series.turns;
    assert_eq!(turns.len(), 3);
    assert_eq!(turns[0].known_usage().unwrap().total(), 1_000);
    assert!(turns[1].usage_unknown());
    assert_eq!(turns[1].uncached_input, None);
    assert_eq!(turns[2].known_usage().unwrap().total(), 300);

    // The last response's own total must agree too.
    let mut row = tokens(1, 1_000, 900, 0, 0, 100);
    row["payload"]["info"]["last_token_usage"]["total_tokens"] = json!(700);
    let (e, _) = scan(&[started(0), context(0, A, "never"), row]);
    assert!(e.series.turns[0].usage_unknown());
}

#[test]
fn each_turn_carries_the_model_of_the_turn_context_before_it() {
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, 10, 5, 0, 0, 5),
        context(2, B, "never"),
        tokens(3, 20, 5, 0, 0, 5),
        tokens(4, 30, 5, 0, 0, 5),
        context(5, A, "never"),
        tokens(6, 40, 5, 0, 0, 5),
    ]);
    assert_eq!(e.model_labels, vec![A.to_string(), B.to_string()]);
    assert_eq!(
        e.series
            .turns
            .iter()
            .map(|t| t.model_label_ix)
            .collect::<Vec<_>>(),
        vec![Some(0), Some(1), Some(1), Some(0)]
    );
    assert_eq!(f.models.len(), 2);
}

#[test]
fn counters_follow_the_pricing_mapping() {
    let (input, cached, w, output) = (10_000u64, 6_000u64, 1_500u64, 700u64);
    let (e, _) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, input + output, input, cached, w, output),
    ]);
    let t = &e.series.turns[0];
    // The write is its own category, never inside the uncached count
    // (`session_pricing`'s module doc).
    assert_eq!(t.uncached_input, Some((input - cached - w) as u32));
    assert_eq!(t.cache_read, Some(cached as u32));
    assert_eq!(t.cache_write, Some(CacheWrite::TotalOnly(w as u32)));
    assert_eq!(t.output, Some(output as u32));

    // Cached and written tokens that exceed the input are not a reading.
    let (e, _) = scan(&[
        started(0),
        context(0, A, "never"),
        tokens(1, 100, 50, 40, 20, 50),
    ]);
    assert!(e.series.turns[0].usage_unknown());
    // A missing write field is unknown, never zero.
    let mut row = tokens(1, 100, 50, 0, 0, 50);
    row["payload"]["info"]["last_token_usage"]
        .as_object_mut()
        .unwrap()
        .remove("cache_write_input_tokens");
    let (e, _) = scan(&[started(0), context(0, A, "never"), row]);
    assert!(e.series.turns[0].usage_unknown());
}

#[test]
fn shell_only_sessions_have_edits_unknown_not_zero() {
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        shell_call(1, "c1", "PRIVATE_CMD edit-in-place"),
        output(2, "c1", "Process exited with code 0"),
        tokens(3, 10, 5, 0, 0, 5),
        context(4, B, "never"),
        tokens(5, 20, 5, 0, 0, 5),
        complete(6),
    ]);
    let m = model(&e, &f, A);
    assert_eq!(m.edits, None);
    // A model with neither a patch nor a shell call made no edit.
    assert_eq!(model(&e, &f, B).edits, Some(0));
    assert_eq!(f.finished, Some(false));
}

/// A `local_shell_call` item: Codex's built-in shell, its command an
/// argument list, paired with its `function_call_output` by `call_id`.
fn local_shell_call(secs: u32, id: &str, command: &[&str]) -> Value {
    json!({"timestamp": ts(secs), "type": "response_item", "payload": {
        "type": "local_shell_call", "call_id": id, "status": "completed",
        "action": {"type": "exec", "command": command}}})
}

#[test]
fn shell_command_and_local_shell_sessions_have_edits_unknown_not_zero() {
    // `shell_command`, the shell our managed Codex declares.
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        function_call(
            1,
            "c1",
            "shell_command",
            json!({"command": "PRIVATE_CMD edit-in-place && mv a b"}),
        ),
        output(2, "c1", "Process exited with code 0"),
        tokens(3, 10, 5, 0, 0, 5),
        complete(4),
    ]);
    assert_eq!(model(&e, &f, A).edits, None);
    assert_eq!(f.shape.renamed_files, 1);
    assert_eq!(e.series.tool_calls[0].tool, ToolKind::Bash);
    assert!(e.series.tool_calls[0].paired);

    // `local_shell_call`, paired with its output by `call_id`.
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        local_shell_call(1, "c1", &["bash", "-lc", "PRIVATE_CMD edit-in-place"]),
        output(2, "c1", "Process exited with code 1"),
        local_shell_call(3, "c2", &["bash", "-lc", "PRIVATE_CMD edit-in-place"]),
        output(4, "c2", "Process exited with code 0"),
        tokens(5, 10, 5, 0, 0, 5),
        complete(6),
    ]);
    assert_eq!(model(&e, &f, A).edits, None);
    let calls = &e.series.tool_calls;
    assert_eq!(
        calls
            .iter()
            .map(|c| (c.tool, c.paired, c.success))
            .collect::<Vec<_>>(),
        vec![
            (ToolKind::Bash, true, Some(false)),
            (ToolKind::Bash, true, Some(true)),
        ]
    );

    // A stopped local shell command is an interrupt, as an exec one is.
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        local_shell_call(1, "c1", &["bash", "-lc", "PRIVATE_CMD long"]),
        output(5, "c1", "aborted by user after 4.0s"),
        aborted(6, "interrupted"),
    ]);
    assert_eq!(model(&e, &f, A).interrupted, Some(1));
}

#[test]
fn apply_patch_hunks_count_edits_and_feed_undone() {
    let two_files = "*** Begin Patch\n*** Add File: docs/PRIVATE_NEW.md\n+PRIVATE_A\n+PRIVATE_B\n\
*** Update File: src/PRIVATE_X.rs\n*** Move to: src/PRIVATE_Y.rs\n@@\n-PRIVATE_OLD\n+PRIVATE_NEW\n*** End Patch\n";
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        patch_call(1, "p1", two_files),
        output(2, "p1", "Exit code: 0"),
        tokens(3, 10, 5, 0, 0, 5),
        complete(4),
        // A prompt between the edit and its restore: undone.
        started(10),
        context(10, A, "never"),
        patch_call(
            11,
            "p2",
            &update("src/PRIVATE_Y.rs", "PRIVATE_NEW", "PRIVATE_OLD"),
        ),
        output(12, "p2", "Exit code: 0"),
        tokens(13, 20, 5, 0, 0, 5),
        complete(14),
    ]);
    let a = model(&e, &f, A);
    assert_eq!(a.edits, Some(3));
    assert_eq!(a.undone, Some(1));
    assert_eq!(f.shape.renamed_files, 1);
    assert_eq!(f.shape.edited_files.doc, 1);
    // A moved file is counted once, as its target.
    assert_eq!(f.shape.edited_files.source, 1);
    assert_eq!(f.shape.lines_added, Some(4));
    assert_eq!(f.shape.lines_removed, Some(2));
    // The undo came after the last prompt.
    assert_eq!(f.finished, Some(false));
    let calls = &e.series.tool_calls;
    assert_eq!(calls[0].path_ext.as_deref(), Some("md"));
    assert!(calls.iter().all(|c| c.tool == ToolKind::ApplyPatch));

    // The same restore with no prompt between is the model's own retry.
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        patch_call(1, "p1", &update("src/x.rs", "a", "b")),
        output(2, "p1", "Exit code: 0"),
        patch_call(3, "p2", &update("src/x.rs", "b", "a")),
        output(4, "p2", "Exit code: 0"),
        tokens(5, 10, 5, 0, 0, 5),
        complete(6),
    ]);
    assert_eq!(model(&e, &f, A).undone, Some(0));
    assert_eq!(f.finished, Some(true));

    // A deleted file's removed lines are not in the transcript.
    let (_, f) = scan(&[
        started(0),
        context(0, A, "never"),
        patch_call(
            1,
            "p1",
            "*** Begin Patch\n*** Delete File: src/x.rs\n*** End Patch\n",
        ),
        output(2, "p1", "Exit code: 0"),
        tokens(3, 10, 5, 0, 0, 5),
    ]);
    assert_eq!(f.shape.lines_removed, None);
    assert_eq!(f.shape.lines_added, None);
}

/// Edits and undone count one unit, the patch's file section: a section of
/// three hunks that a later patch restores hunk by hunk is one edit undone.
#[test]
fn a_multi_hunk_section_restored_whole_is_one_edit_undone() {
    let forward = "*** Begin Patch\n*** Update File: src/x.rs\n@@\n-a\n+b\n@@\n-c\n+d\n@@\n-e\n+f\n*** End Patch\n";
    let back = "*** Begin Patch\n*** Update File: src/x.rs\n@@\n-b\n+a\n@@\n-d\n+c\n@@\n-f\n+e\n*** End Patch\n";
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        patch_call(1, "p1", forward),
        output(2, "p1", "Exit code: 0"),
        tokens(3, 10, 5, 0, 0, 5),
        complete(4),
        started(10),
        context(10, A, "never"),
        patch_call(11, "p2", back),
        output(12, "p2", "Exit code: 0"),
        tokens(13, 20, 5, 0, 0, 5),
        complete(14),
    ]);
    let a = model(&e, &f, A);
    assert_eq!(a.edits, Some(2));
    assert_eq!(a.undone, Some(1));
    assert!(a.undone <= a.edits);
}

#[test]
fn codex_denied_is_unknown_in_every_mode() {
    for policy in ["never", "on-request", "untrusted"] {
        let mut ctx = context(0, A, policy);
        ctx["payload"]["approvals_reviewer"] = json!("user");
        let (e, f) = scan(&[
            started(0),
            ctx,
            patch_call(1, "p1", &update("src/x.rs", "a", "b")),
            output(2, "p1", "Exit code: 0"),
            tokens(3, 10, 5, 0, 0, 5),
            complete(4),
        ]);
        let m = model(&e, &f, A);
        assert_eq!(m.denied, None, "{policy}");
        assert_eq!(m.edits, Some(1));
        // `never` puts no edit to the contributor; the others are
        // unknown while no Codex denial marker is confirmed.
        let expected = if policy == "never" {
            EditMode::NotPrompting
        } else {
            EditMode::Unknown
        };
        assert_eq!(f.edit_mode, expected, "{policy}");
    }
}

#[test]
fn turn_aborted_and_exec_aborted_by_user_are_interrupts() {
    let (e, f) = scan(&[
        started(0),
        context(0, A, "never"),
        shell_call(1, "c1", "PRIVATE_CMD long"),
        output(5, "c1", "aborted by user after 4.0s"),
        // The turn abort that follows the exec abort is the same interrupt.
        aborted(6, "interrupted"),
        started(10),
        context(10, B, "never"),
        tokens(11, 10, 5, 0, 0, 5),
        aborted(12, "interrupted"),
        started(20),
        context(20, B, "never"),
        shell_call(21, "c2", "PRIVATE_CMD other"),
        // An exec abort with no turn abort after it still counts.
        output(22, "c2", "aborted by user after 1.0s"),
        started(30),
        context(30, B, "never"),
        aborted(31, "replaced"),
    ]);
    assert_eq!(model(&e, &f, A).interrupted, Some(1));
    assert_eq!(model(&e, &f, B).interrupted, Some(2));
    assert_eq!(f.user_turns, Some(4));
}

#[test]
fn request_user_input_waits_are_subtracted() {
    let (_, f) = scan(&[
        started(0),
        context(0, A, "never"),
        function_call(10, "q1", "request_user_input", json!({"questions": []})),
        output(70, "q1", "PRIVATE_TEXT answer"),
        function_call(80, "q2", "request_user_input_async", json!({})),
        output(90, "q2", "PRIVATE_TEXT answer"),
        tokens(100, 10, 5, 0, 0, 5),
        complete(100),
        // Time after the last model event is in no span.
        started(500),
        context(500, A, "never"),
        tokens(510, 20, 5, 0, 0, 5),
        complete(520),
    ]);
    assert_eq!(f.active_secs, Some(100 - 60 - 10 + 20));

    // A model event with no timestamp makes active time unknown.
    let mut undated = tokens(5, 10, 5, 0, 0, 5);
    undated.as_object_mut().unwrap().remove("timestamp");
    let (_, f) = scan(&[started(0), context(0, A, "never"), undated, complete(6)]);
    assert_eq!(f.active_secs, None);
}

#[test]
fn an_all_reviewer_session_is_marked_reviewer() {
    let reviewer = CODEX_REVIEWER_MODEL;
    let (_, f) = scan(&[
        started(0),
        context(0, reviewer, "never"),
        tokens(1, 10, 5, 0, 0, 5),
        context(2, reviewer, "never"),
        tokens(3, 20, 5, 0, 0, 5),
    ]);
    assert!(f.reviewer);
    let (_, f) = scan(&[
        started(0),
        context(0, reviewer, "never"),
        context(2, A, "never"),
        tokens(3, 20, 5, 0, 0, 5),
    ]);
    assert!(!f.reviewer);
    let (_, f) = scan(&[started(0), tokens(3, 20, 5, 0, 0, 5)]);
    assert!(!f.reviewer);
}

#[test]
fn past_the_turn_cap_the_series_is_truncated_and_facts_capped() {
    let key = key();
    let mut scan = CodexTurnSeriesScan::new(&key);
    scan.observe(&started(0)).unwrap();
    scan.observe(&context(0, A, "never")).unwrap();
    let turns = MAX_SERIES_TURNS as u64 + 5;
    for n in 1..=turns {
        scan.observe(&tokens(1, n * 2, 1, 0, 0, 1)).unwrap();
    }
    scan.observe(&shell_call(2, "late", "PRIVATE_CMD")).unwrap();
    let (e, f) = scan.finish().unwrap();
    e.validate().unwrap();
    assert_eq!(e.series.turns.len(), MAX_SERIES_TURNS);
    assert!(e.series.truncated);
    assert!(e.series.tool_calls.is_empty());
    assert!(f.capped);
}

#[test]
fn the_codex_series_and_facts_hold_no_readable_text() {
    let (e, f) = scan(&fixture_rows());
    let bytes = [
        serde_json::to_vec(&e).unwrap(),
        serde_json::to_vec(&f).unwrap(),
    ]
    .concat();
    let text = String::from_utf8_lossy(&bytes);
    // `apply_patch` appears only as the fixed tool label.
    for private in [
        "PRIVATE",
        "src/",
        "exec_command",
        "call-1",
        "turn-1",
        "never",
        "auto_review",
        "Process exited",
        "00000000-0000-4000",
    ] {
        assert!(!text.contains(private), "{private}");
    }
}

#[test]
fn a_codex_drill_with_a_series_draws_it() {
    let (e, _) = scan(&fixture_rows());
    let body = SessionBody::Codex {
        observed: None,
        series: Some(e),
    };
    let drill = session_drill(
        "snap",
        &body,
        None,
        None,
        chrono::FixedOffset::east_opt(0).unwrap(),
    );
    assert!(drill.series.is_some());
    assert_eq!(drill.series_unavailable, None);
    assert_eq!(drill.turns, Some(3));
    // The chart is the series; the tokens are the observed change, the one
    // basis a Codex session has, and unknown with none observed.
    let charted: u64 = drill
        .series
        .as_ref()
        .unwrap()
        .iter()
        .map(|turn| {
            u64::from(turn.uncached.unwrap())
                + u64::from(turn.cache_read.unwrap())
                + turn.cache_write.unwrap()
                + u64::from(turn.output.unwrap())
        })
        .sum();
    assert_eq!(charted, 4_800);
    assert_eq!(drill.tokens, None);
}

#[test]
fn a_codex_drill_without_a_series_still_reads_not_recorded() {
    let drill = session_drill(
        "snap",
        &SessionBody::Codex {
            observed: None,
            series: None,
        },
        None,
        None,
        chrono::FixedOffset::east_opt(0).unwrap(),
    );
    assert_eq!(drill.series, None);
    assert_eq!(
        drill.series_unavailable,
        Some(SeriesUnavailable::NotRecorded)
    );
}

#[test]
fn weekly_codex_figures_ignore_the_series() {
    let (e, _) = scan(&fixture_rows());
    let at = |h| {
        chrono::DateTime::parse_from_rfc3339(&format!("2026-01-06T{h:02}:00:00Z"))
            .unwrap()
            .with_timezone(&chrono::Utc)
    };
    let input = |series: Option<CodexTurnSeriesEvidence>| SessionInput {
        session_ref: "c".into(),
        import_seq: 1,
        placed_at: None,
        harness_session: None,
        automated: false,
        facts: None,
        body: SessionBody::Codex {
            observed: Some(CodexObserved {
                first_at: at(9),
                last_at: at(10),
                input: 1_000,
                cached_input: 400,
                output: 200,
                total: 1_200,
                baseline_excluded: false,
            }),
            series,
        },
    };
    let with = [input(Some(e))];
    let without = [input(None)];
    let monday = chrono::NaiveDate::from_ymd_opt(2026, 1, 5).unwrap();
    let tz = chrono::FixedOffset::east_opt(0).unwrap();
    // Every weekly figure but `codex_by_day`, which is the series split by
    // day (product owner, 2026-10-28) and so reads it by design.
    let figures = |sessions: &[SessionInput]| {
        let mut rollup = week_rollup(Feed::Saved, sessions, monday, &tz);
        rollup.codex_by_day = None;
        rollup
    };
    assert_eq!(figures(&with), figures(&without));
    assert_ne!(
        week_rollup(Feed::Saved, &with, monday, &tz).codex_by_day,
        week_rollup(Feed::Saved, &without, monday, &tz).codex_by_day
    );
    assert_eq!(
        week_patterns(Feed::Saved, &with, monday, tz, 6),
        week_patterns(Feed::Saved, &without, monday, tz, 6)
    );
    assert_eq!(dated_weeks(&with, &tz), dated_weeks(&without, &tz));
}

/// With no `task_started` there is no span: the time was never measured, so
/// it is unknown, never zero seconds.
#[test]
fn a_session_with_no_task_start_has_unknown_active_time() {
    let (_, f) = scan(&[
        context(0, A, "never"),
        patch_call(1, "p1", &update("PRIVATE_PATH/a.rs", "x", "y")),
        output(2, "p1", "Exit code: 0"),
        tokens(3, 10, 5, 0, 0, 5),
        complete(4),
    ]);
    assert_eq!(f.user_turns, Some(0));
    assert_eq!(f.active_secs, None);
}
