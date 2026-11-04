use chrono::Utc;
use serde_json::{Value, json};
use std::collections::BTreeSet;
use std::path::Path;

use super::Harness;
use super::facts::{FactBook, Observations};
use super::hook::handle;
use super::launch::{
    CLAUDE_EVENTS, CODEX_EVENTS, Launch, MIN_CLAUDE, MIN_CODEX, MIN_OPENCODE, testing,
};
use super::loop_guard::{
    Envelope, StuckJudge, Thresholds, TurnState, Verdict, redirect_text, should_redirect,
    should_redirect_judged,
};
use super::signals::{ToolCall, failure_signature, is_check_command};
use super::store::{RepoState, Store, repo_key};
use super::tools::{self, ToolLedger};

fn bash(command: &str, stdout: &str) -> ToolCall {
    ToolCall::from_hook(
        &json!({"tool_name": "Bash", "tool_input": {"command": command},
                "tool_response": {"stdout": stdout, "stderr": "", "interrupted": false}}),
        false,
    )
    .unwrap()
}

fn bash_failed(command: &str, error: &str) -> ToolCall {
    ToolCall::from_hook(
        &json!({"tool_name": "Bash", "tool_input": {"command": command}, "error": error}),
        true,
    )
    .unwrap()
}

fn edit(path: &str) -> ToolCall {
    ToolCall::from_hook(
        &json!({"tool_name": "Edit", "tool_input": {"file_path": path, "old_string": "a", "new_string": "b"},
                "tool_response": {"filePath": path}}),
        false,
    )
    .unwrap()
}

fn read(path: &str, content: &str) -> ToolCall {
    ToolCall::from_hook(
        &json!({"tool_name": "Read", "tool_input": {"file_path": path},
                "tool_response": {"content": content}}),
        false,
    )
    .unwrap()
}

// ---- signals ----

#[test]
fn a_failed_call_reads_its_error_and_a_piped_failure_is_still_a_failure() {
    let failed = bash_failed(
        "cargo test",
        "Exit code 101\nerror[E0425]: cannot find value",
    );
    assert!(failed.failed && failed.is_failure());
    assert!(failed.output.contains("E0425"));

    let piped = bash(
        "pytest -q | tail -3",
        "FAILED tests/test_a.py::test_x - assert 1 == 2\n1 failed",
    );
    assert!(!piped.failed);
    assert!(piped.is_failure() && piped.is_check());

    assert!(!bash("ls", "a.txt\nb.txt").is_failure());
}

#[test]
fn file_content_is_never_a_failure() {
    let call = read(
        "/r/tests/test_a.py",
        "def test():\n    assert False  # Error: on purpose",
    );
    assert!(!call.is_failure());
}

#[test]
fn the_same_failure_with_other_numbers_and_paths_has_one_signature() {
    let a = failure_signature("FAILED /home/a/r/tests/t.py::test_x - assert 41 == 42 at 0x7f00");
    let b = failure_signature("FAILED /tmp/b/tests/t.py::test_x - assert 1 == 2 at 0x1234");
    let c = failure_signature("TypeError: unsupported operand");
    assert!(a.is_some());
    assert_eq!(a, b);
    assert_ne!(a, c);
    // A digest, never the text.
    assert!(!a.unwrap().contains("FAILED"));
}

#[test]
fn checks_are_found_inside_compound_commands() {
    assert!(is_check_command("cd app && python -m pytest -q"));
    assert!(is_check_command("cargo test -p trace-commons-contributor"));
    assert!(is_check_command("./node_modules/.bin/tsc --noEmit"));
    assert!(!is_check_command("git status"));
    assert!(!is_check_command("cargo fmt"));
}

// ---- loop guard ----

/// A turn that edits one file over and over and hits the same failure.
fn stuck_turn() -> TurnState {
    let mut turn = TurnState::default();
    for _ in 0..7 {
        turn.record(&edit("/r/src/lib.rs"));
        turn.record(&bash_failed(
            "cargo test",
            "Exit code 101\nthread 'x' panicked at src/lib.rs:10:5: assertion failed",
        ));
        turn.record(&read("/r/src/lib.rs", "fn main() {}"));
    }
    turn
}

fn envelope(samples: &[u32]) -> Envelope {
    let mut envelope = Envelope::default();
    for sample in samples {
        envelope.add(*sample);
    }
    envelope
}

#[test]
fn a_churning_turn_that_repeats_its_failure_looks_stuck() {
    let turn = stuck_turn();
    assert_eq!(turn.calls, 21);
    assert_eq!(turn.edit_churn(), 7);
    assert_eq!(turn.failures, 7);
    assert_eq!(turn.repeated_failures, 6);
    assert!(turn.looks_stuck(&Thresholds::default()));
    assert!(!turn.ended_well());
}

#[test]
fn a_busy_turn_with_changing_failures_is_left_alone() {
    let mut turn = TurnState::default();
    for i in 0..10 {
        turn.record(&edit(&format!("/r/src/m{i}.rs")));
        turn.record(&bash_failed(
            "cargo test",
            &format!("Exit code 101\nerror: unknown thing{}", "x".repeat(i + 1)),
        ));
    }
    assert!(turn.calls >= 20);
    assert_eq!(turn.repeated_failures, 0);
    assert!(!turn.looks_stuck(&Thresholds::default()));
}

#[test]
fn a_short_turn_is_never_stuck() {
    let mut turn = TurnState::default();
    for _ in 0..4 {
        turn.record(&edit("/r/a.rs"));
        turn.record(&bash_failed("cargo test", "Exit code 101\nerror: same"));
    }
    assert!(!turn.looks_stuck(&Thresholds::default()));
}

#[test]
fn the_envelope_is_a_nearest_rank_percentile_once_it_has_enough_samples() {
    let thresholds = Thresholds::default();
    assert_eq!(envelope(&[3, 4, 5, 6]).limit(&thresholds), None);
    assert_eq!(
        envelope(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]).limit(&thresholds),
        Some(9)
    );
    assert_eq!(envelope(&[10, 10, 10, 10, 10]).limit(&thresholds), Some(10));
    let mut long = Envelope::default();
    for i in 0..80 {
        long.add(i);
    }
    assert_eq!(long.samples.len(), Envelope::KEEP);
    assert_eq!(long.samples[0], 30);
}

#[test]
fn a_stuck_turn_is_redirected_once_and_only_past_the_envelope() {
    let thresholds = Thresholds::default();
    let mut turn = stuck_turn();
    // No envelope yet: the folder is unknown, so nothing happens.
    assert!(!should_redirect(
        &mut turn,
        &Envelope::default(),
        &thresholds
    ));
    // Turns here usually end within 30 calls: 21 is not past that.
    assert!(!should_redirect(
        &mut turn,
        &envelope(&[30; 5]),
        &thresholds
    ));
    // Within 12: redirected, then never again in this turn.
    assert!(should_redirect(&mut turn, &envelope(&[12; 5]), &thresholds));
    assert_eq!(turn.redirects, 1);
    assert!(!should_redirect(
        &mut turn,
        &envelope(&[12; 5]),
        &thresholds
    ));
}

struct FixedJudge(Option<Verdict>);

impl StuckJudge for FixedJudge {
    fn judge(&self, _turn: &TurnState, _envelope: Option<u32>) -> Option<Verdict> {
        self.0
    }
}

#[test]
fn a_judge_decides_a_turn_worth_judging() {
    let thresholds = Thresholds::default();
    let known = envelope(&[30; 5]);
    for (verdict, expected) in [
        (Some(Verdict::Stuck { confidence: 0.9 }), true),
        (Some(Verdict::Stuck { confidence: 0.5 }), false),
        (Some(Verdict::Productive), false),
        (Some(Verdict::Unsure), false),
        (None, false),
    ] {
        let mut turn = stuck_turn();
        let judge = FixedJudge(verdict);
        assert_eq!(
            should_redirect_judged(&mut turn, &known, &thresholds, Some(&judge)),
            expected,
            "{verdict:?}"
        );
    }
    // A judge never acts in a folder with no envelope.
    let mut turn = stuck_turn();
    let judge = FixedJudge(Some(Verdict::Stuck { confidence: 1.0 }));
    assert!(!should_redirect_judged(
        &mut turn,
        &Envelope::default(),
        &thresholds,
        Some(&judge)
    ));
}

#[test]
fn a_turn_ends_well_only_when_its_latest_check_passed() {
    let mut turn = TurnState::default();
    turn.record(&edit("/r/a.rs"));
    turn.record(&bash_failed("cargo test", "Exit code 101\nerror: x"));
    assert!(!turn.ended_well());
    turn.record(&edit("/r/a.rs"));
    turn.record(&bash("cargo test", "test result: ok. 3 passed"));
    assert!(turn.ended_well());
    assert!(!TurnState::default().ended_well());

    // A check that fails and then passes, with no edit between, ends well.
    let mut turn = TurnState::default();
    turn.record(&bash_failed("pytest", "bash: pytest: command not found"));
    assert!(!turn.ended_well());
    turn.record(&bash("python -m pytest", "2 passed"));
    assert!(turn.ended_well());
}

#[test]
fn the_redirect_names_no_file_and_never_asks_to_stop() {
    let text = redirect_text(&stuck_turn());
    assert!(!text.contains("lib.rs"), "{text}");
    assert!(text.contains("Keep working"));
    assert!(text.contains("7 edit(s)"));
}

// ---- tools ----

fn used(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_owned()).collect()
}

#[test]
fn an_unused_tool_is_denied_after_enough_prompts_over_two_sessions() {
    let thresholds = tools::Thresholds::default();
    let mut ledger = ToolLedger::default();
    ledger.fold_session(Harness::Claude, 10, &used(&["Bash", "Read"]));
    assert!(
        ledger.deny_list(Harness::Claude, &thresholds).is_empty(),
        "one session never decides"
    );
    ledger.fold_session(Harness::Claude, 1, &used(&["Bash"]));
    let denied = ledger.deny_list(Harness::Claude, &thresholds);
    assert!(denied.contains(&"Agent".to_owned()), "{denied:?}");
    assert!(denied.contains(&"TodoWrite".to_owned()));
    // Web access and notebooks need three times as long.
    assert!(!denied.contains(&"WebFetch".to_owned()));
    assert!(!denied.contains(&"NotebookEdit".to_owned()));
    // The work surface is never a candidate.
    for protected in ["Bash", "Read", "Edit", "Write", "Grep", "Glob", "Skill"] {
        assert!(!denied.iter().any(|name| name == protected));
        assert!(
            !tools::CLAUDE_CANDIDATES
                .iter()
                .any(|(name, _)| *name == protected)
        );
    }
}

#[test]
fn many_short_sessions_also_count() {
    let thresholds = tools::Thresholds::default();
    let mut ledger = ToolLedger::default();
    for _ in 0..5 {
        ledger.fold_session(Harness::Claude, 1, &used(&[]));
    }
    let denied = ledger.deny_list(Harness::Claude, &thresholds);
    assert!(denied.contains(&"Monitor".to_owned()));
    assert!(!denied.contains(&"WebSearch".to_owned()));
    for _ in 0..10 {
        ledger.fold_session(Harness::Claude, 1, &used(&[]));
    }
    assert!(
        ledger
            .deny_list(Harness::Claude, &thresholds)
            .contains(&"WebSearch".to_owned())
    );
}

#[test]
fn a_tool_used_once_or_given_back_is_never_denied() {
    let thresholds = tools::Thresholds::default();
    let mut ledger = ToolLedger::default();
    ledger.fold_session(Harness::Claude, 3, &used(&["Agent"]));
    ledger.fold_session(Harness::Claude, 0, &used(&[]));
    for _ in 0..20 {
        ledger.fold_session(Harness::Claude, 10, &used(&[]));
    }
    let denied = ledger.deny_list(Harness::Claude, &thresholds);
    assert!(!denied.contains(&"Agent".to_owned()));
    assert!(denied.contains(&"CronList".to_owned()));
    let restored = ledger.restore(Harness::Claude, &thresholds);
    assert_eq!(restored, denied);
    for _ in 0..20 {
        ledger.fold_session(Harness::Claude, 10, &used(&[]));
    }
    assert!(ledger.deny_list(Harness::Claude, &thresholds).is_empty());
}

// ---- facts ----

fn repair_session() -> Observations {
    let mut observations = Observations::default();
    observations.record(&bash_failed(
        "cd app && pytest -q",
        "Exit code 127\n/bin/bash: line 1: pytest: command not found",
    ));
    observations.record(&bash_failed(
        "cd app && python -m pytest -q",
        "Exit code 1\nFAILED tests/test_a.py::test_x",
    ));
    observations
}

#[test]
fn a_not_found_program_that_runs_through_a_launcher_is_a_repair() {
    let observations = repair_session();
    assert!(observations.missing.contains("pytest"));
    assert_eq!(
        observations.repairs,
        [("pytest".to_owned(), "python -m".to_owned())]
            .into_iter()
            .collect()
    );
}

#[test]
fn a_fact_is_told_after_three_sessions_and_dropped_after_two_contradictions() {
    let mut book = FactBook::default();
    book.fold_session("s1", &repair_session());
    book.fold_session("s1", &repair_session());
    book.fold_session("s2", &repair_session());
    assert_eq!(book.render(), None, "two distinct sessions are not enough");
    book.fold_session("s3", &repair_session());
    let told = book.render().unwrap();
    assert!(
        told.contains("Run `pytest` as `python -m pytest` here"),
        "{told}"
    );

    let mut works = Observations::default();
    works.record(&bash("pytest -q", "3 passed"));
    book.fold_session("s4", &works);
    assert!(book.render().is_some(), "one contradiction is not enough");
    book.fold_session("s5", &works);
    assert_eq!(book.render(), None);
    assert!(book.facts.is_empty());
}

#[test]
fn paths_and_arguments_never_become_facts() {
    let mut observations = Observations::default();
    observations.record(&bash_failed(
        "./venv/bin/pytest -q",
        "Exit code 127\n./venv/bin/pytest: command not found",
    ));
    observations.record(&bash("TOKEN=abc /usr/bin/python -m pytest", "1 passed"));
    assert!(observations.missing.is_empty());
    assert!(observations.repairs.is_empty());
}

#[test]
fn a_program_is_not_found_only_by_its_own_name() {
    let mut observations = Observations::default();
    observations.record(&bash_failed(
        "npm i -g pnpm && pnpm test",
        "Exit code 127\nbash: pnpm: command not found",
    ));
    assert!(observations.missing.contains("pnpm"));
    assert!(!observations.missing.contains("npm"), "pnpm is not npm");
    let mut zsh = Observations::default();
    zsh.record(&bash_failed("pytest -q", "zsh: command not found: pytest"));
    assert!(zsh.missing.contains("pytest"));
    assert!(!zsh.missing.contains("py"));
}

/// Over the cap, the oldest untold fact goes, so a folder full of one-off
/// repairs still learns a new one.
#[test]
fn a_full_fact_book_still_learns_a_new_repair() {
    use super::facts::MAX_KEPT;
    let mut book = FactBook::default();
    for i in 0..MAX_KEPT {
        let mut once = Observations::default();
        once.repairs
            .insert((format!("tool{i}"), "python -m".to_owned()));
        book.fold_session(&format!("old{i}"), &once);
    }
    assert_eq!(book.facts.len(), MAX_KEPT);
    for session in ["s1", "s2", "s3"] {
        book.fold_session(session, &repair_session());
        assert!(
            book.facts.iter().any(|fact| fact.program == "pytest"),
            "the new fact is kept ({session})"
        );
        assert_eq!(book.facts.len(), MAX_KEPT);
    }
    assert!(book.render().unwrap().contains("python -m pytest"));
    assert!(
        !book.facts.iter().any(|fact| fact.program == "tool0"),
        "the oldest one-off went first"
    );
}

/// With every kept fact told, a new repair still survives its own fold,
/// the oldest told fact goes, and the most recently confirmed are told.
#[test]
fn a_book_of_told_facts_still_learns_a_new_repair() {
    use super::facts::{CONFIRMATIONS, MAX_KEPT, MAX_TOLD};
    let mut book = FactBook::default();
    for i in 0..MAX_KEPT {
        let mut once = Observations::default();
        once.repairs
            .insert((format!("tool{i}"), "python -m".to_owned()));
        for c in 0..CONFIRMATIONS {
            book.fold_session(&format!("old{i}-{c}"), &once);
        }
    }
    assert!(book.facts.iter().all(|fact| fact.told()));
    book.fold_session("s1", &repair_session());
    assert_eq!(book.facts.len(), MAX_KEPT);
    assert!(
        book.facts.iter().any(|fact| fact.program == "pytest"),
        "the new fact survives its own fold"
    );
    assert!(
        !book.facts.iter().any(|fact| fact.program == "tool0"),
        "the oldest told fact went"
    );
    let told: Vec<_> = book.told().map(|fact| fact.program.clone()).collect();
    assert_eq!(told.len(), MAX_TOLD);
    assert_eq!(
        told[0],
        format!("tool{}", MAX_KEPT - 1),
        "newest told first"
    );
}

/// Codex reports a failed command through `PostToolUse`; its exit status
/// marks it failed, check or not.
#[test]
fn a_non_zero_exit_is_a_failure_for_any_command() {
    let codex = |response: Value| {
        ToolCall::from_hook(
            &json!({"tool_name": "exec_command", "tool_input": {"cmd": "python repro.py"},
                    "tool_response": response}),
            false,
        )
        .unwrap()
    };
    assert!(codex(json!({"output": "Traceback", "exit_code": 1})).is_failure());
    assert!(codex(json!({"output": "x", "metadata": {"exit_code": 2}})).is_failure());
    assert!(codex(json!("Exit code: 1\nWall time: 0.1 seconds\nOutput:\nboom")).is_failure());
    assert!(!codex(json!({"output": "Traceback in a log", "exit_code": 0})).is_failure());
}

/// A passing check's output may name an exception or log `error:`: a pass
/// line is not a failure, and an unpiped check that exited 0 passed.
#[test]
fn a_passing_check_is_not_a_failure() {
    assert!(
        !bash(
            "pytest -v | tail",
            "tests/a.py::test_raises_TimeoutException PASSED"
        )
        .is_failure()
    );
    assert!(!bash("cargo test | tail", "test error_path ... ok").is_failure());
    let exited_zero = ToolCall::from_hook(
        &json!({"tool_name": "Bash", "tool_input": {"command": "cargo test -- --nocapture"},
                "tool_response": {"stdout": "error: logged by a test\ntest result: ok", "exit_code": 0}}),
        false,
    )
    .unwrap();
    assert!(!exited_zero.is_failure());
    assert!(bash("pytest | tail", "FAILED tests/a.py::test_x").is_failure());
    // A sequence hides the check's exit behind the last command's, so its
    // markers still count even when the whole exited 0.
    for command in [
        "cargo test; echo done",
        "pytest > out.txt\ntail out.txt",
        "cargo test || true",
        "cargo test &",
    ] {
        let sequenced = ToolCall::from_hook(
            &json!({"tool_name": "Bash", "tool_input": {"command": command},
                    "tool_response": {"stdout": "error[E0308]: mismatched types", "exit_code": 0}}),
            false,
        )
        .unwrap();
        assert!(sequenced.is_failure(), "{command}");
    }
    // `&&` and the `2>&1` / `&>` redirections keep the check's own exit, so
    // an exit of 0 there means it passed.
    for command in [
        "cargo test 2>&1",
        "cd crate && cargo test",
        "cargo test &> out.txt",
    ] {
        let own_exit = ToolCall::from_hook(
            &json!({"tool_name": "Bash", "tool_input": {"command": command},
                    "tool_response": {"stdout": "error: logged\nFAILED x (expected)", "exit_code": 0}}),
            false,
        )
        .unwrap();
        assert!(!own_exit.is_failure(), "{command}");
    }
}

/// Source printed by a command is content: `error:` or `Exception` in it is
/// not a failure. A check's output still is.
#[test]
fn a_command_printing_source_is_not_a_failure() {
    for (command, output) in [
        (
            "cat src/main.rs",
            "    // error: never here\n    panic!(\"error: x\")",
        ),
        ("rg Exception src", "src/a.py:3: raise Exception('x')"),
        ("git diff", "+    return Err(\"error: bad\")"),
    ] {
        assert!(!bash(command, output).is_failure(), "{command}");
    }
    assert!(bash("cargo test | tail -3", "error: test failed").is_failure());
}

// ---- store ----

#[test]
fn the_store_is_keyed_by_a_digest_and_refuses_unsafe_names() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let key = repo_key(Path::new("/home/a/secret-project"), Harness::Claude);
    assert!(!key.contains("secret"));
    assert_eq!(
        key,
        repo_key(Path::new("/home/a/secret-project"), Harness::Claude)
    );
    assert_ne!(key, repo_key(Path::new("/home/a/other"), Harness::Claude));

    let mut state = RepoState::default();
    state.envelope.add(7);
    store.save_repo(&key, &state).unwrap();
    assert_eq!(store.repo(&key), state);
    assert!(store.save_repo("../escape", &state).is_err());
    assert_eq!(store.repo("../escape"), RepoState::default());

    assert!(!store.enabled());
    store.set_enabled(true).unwrap();
    assert!(store.enabled());
    store.set_enabled(false).unwrap();
    store.set_enabled(false).unwrap();
    assert!(!store.enabled());
}

#[cfg(unix)]
#[test]
fn the_store_is_owner_only() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    store.save_repo("abc", &RepoState::default()).unwrap();
    let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(store.root()), 0o700);
    assert_eq!(mode(&store.root().join("repos")), 0o700);
    assert_eq!(mode(&store.root().join("repos/abc.json")), 0o600);
}

// ---- hook ----

fn event(name: &str, session: &str) -> Value {
    json!({"hook_event_name": name, "session_id": session, "cwd": "/r",
           "transcript_path": "/t.jsonl"})
}

fn tool_event(name: &str, session: &str, call: Value) -> Value {
    let mut value = event(name, session);
    value
        .as_object_mut()
        .unwrap()
        .extend(call.as_object().unwrap().clone());
    value
}

/// One whole session: `prompts` prompts, the pytest repair, and an Edit.
fn run_session(store: &Store, repo: &str, session: &str, prompts: u32) {
    let now = Utc::now();
    handle(
        store,
        repo,
        Harness::Claude,
        &event("SessionStart", session),
        now,
    );
    for _ in 0..prompts {
        handle(
            store,
            repo,
            Harness::Claude,
            &event("UserPromptSubmit", session),
            now,
        );
    }
    handle(
        store,
        repo,
        Harness::Claude,
        &tool_event(
            "PostToolUseFailure",
            session,
            json!({"tool_name": "Bash", "tool_input": {"command": "pytest"},
                   "error": "Exit code 127\nbash: pytest: command not found"}),
        ),
        now,
    );
    handle(
        store,
        repo,
        Harness::Claude,
        &tool_event(
            "PostToolUse",
            session,
            json!({"tool_name": "Bash", "tool_input": {"command": "python -m pytest"},
                   "tool_response": {"stdout": "2 passed", "stderr": ""}}),
        ),
        now,
    );
    handle(store, repo, Harness::Claude, &event("Stop", session), now);
    handle(
        store,
        repo,
        Harness::Claude,
        &event("SessionEnd", session),
        now,
    );
}

#[test]
fn the_hook_does_nothing_while_the_economizer_is_off() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    run_session(&store, "repo1", "s1", 12);
    assert_eq!(store.repo("repo1"), RepoState::default());
    assert!(!store.root().join("sessions").exists());
}

#[test]
fn parallel_hooks_of_one_session_lose_no_update() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("SessionStart", "p1"),
        Utc::now(),
    );
    let calls = 16;
    std::thread::scope(|scope| {
        for i in 0..calls {
            let store = &store;
            scope.spawn(move || {
                let tool = if i == 0 { "WebFetch" } else { "Read" };
                handle(
                    store,
                    "repo1",
                    Harness::Claude,
                    &tool_event(
                        "PostToolUse",
                        "p1",
                        json!({"tool_name": tool, "tool_input": {"file_path": "/r/a"},
                               "tool_response": "x"}),
                    ),
                    Utc::now(),
                );
            });
        }
    });
    let session = store.session("p1").unwrap();
    assert_eq!(session.turn.calls, calls);
    assert!(session.used_tools.contains("WebFetch"));
}

/// A session's lock file outlives its state, so a late hook of the same
/// session locks the same file; the stale prune removes it later.
#[test]
fn a_session_lock_is_left_for_the_stale_prune() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    run_session(&store, "repo1", "s1", 2);
    let lock = store.root().join("sessions").join("s1.lock");
    assert!(lock.exists(), "kept after SessionEnd");
    assert!(!store.root().join("sessions").join("s1.json").exists());
    store.prune_sessions(Utc::now());
    assert!(lock.exists(), "a fresh lock is kept");
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&lock)
        .unwrap()
        .set_modified(old)
        .unwrap();
    store.prune_sessions(Utc::now());
    assert!(!lock.exists(), "a stale orphan lock is removed");
}

/// Only `SessionStart` makes session state: a hook of a session that has
/// ended leaves nothing behind.
#[test]
fn a_late_hook_after_session_end_leaves_no_state() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    run_session(&store, "repo1", "s1", 1);
    let state = store.root().join("sessions").join("s1.json");
    assert!(!state.exists());
    // A prompt may restart a session left idle past the prune; the other
    // events of a session with no file do nothing.
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("Stop", "s1"),
        Utc::now(),
    );
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &tool_event(
            "PostToolUse",
            "s1",
            json!({"tool_name": "Read", "tool_input": {"file_path": "/r/a"}, "tool_response": "x"}),
        ),
        Utc::now(),
    );
    assert!(!state.exists(), "no orphan session state");
}

/// For Claude Code a `PostToolUse` command exited 0, whatever its output
/// prints: an `Exit code: 1` line in a log it shows is not its status.
#[test]
fn claudes_known_exit_status_wins_over_printed_text() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("SessionStart", "c1"),
        Utc::now(),
    );
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &tool_event(
            "PostToolUse",
            "c1",
            json!({"tool_name": "Bash", "tool_input": {"command": "cat ci.log"},
                   "tool_response": {"stdout": "Exit code: 1\nbuild failed", "stderr": ""}}),
        ),
        Utc::now(),
    );
    let session = store.session("c1").unwrap();
    assert_eq!(session.turn.calls, 1);
    assert!(session.turn.failure_signatures.is_empty(), "not a failure");
}

/// The prune removes only idle session files: a live session older than
/// the stale age keeps its state, and one whose file does not read starts
/// over rather than switching off.
#[test]
fn a_long_live_session_survives_the_prune() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    let old = Utc::now() - chrono::Duration::days(30);
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("SessionStart", "long"),
        old,
    );
    store.prune_sessions(Utc::now());
    assert!(store.session("long").is_some(), "written just now, so live");
    let file = store.root().join("sessions").join("long.json");
    std::fs::write(&file, b"{\"schema\": 99}").unwrap();
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("UserPromptSubmit", "long"),
        Utc::now(),
    );
    assert_eq!(store.session("long").unwrap().prompts, 1, "started over");
    let stale = std::time::SystemTime::now() - std::time::Duration::from_secs(8 * 86_400);
    std::fs::File::options()
        .write(true)
        .open(&file)
        .unwrap()
        .set_modified(stale)
        .unwrap();
    store.prune_sessions(Utc::now());
    assert!(!file.exists(), "an idle file is pruned");
}

/// A session left idle past the prune starts over at its next prompt, and
/// a session file of another folder is left alone.
#[test]
fn an_idle_session_restarts_at_its_next_prompt_and_others_are_kept() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("UserPromptSubmit", "idle"),
        Utc::now(),
    );
    assert_eq!(store.session("idle").unwrap().prompts, 1, "restarted");
    handle(
        &store,
        "repo2",
        Harness::Claude,
        &event("UserPromptSubmit", "idle"),
        Utc::now(),
    );
    handle(
        &store,
        "repo2",
        Harness::Claude,
        &event("Stop", "idle"),
        Utc::now(),
    );
    let kept = store.session("idle").unwrap();
    assert_eq!(
        (kept.repo.as_str(), kept.prompts),
        ("repo1", 1),
        "another folder's file is kept"
    );
}

/// Give back takes the folder's lock: it waits for a writer holding it.
#[test]
fn give_back_waits_for_the_folder_lock() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store
        .try_update_repo("repo1", std::time::Duration::from_secs(30), |_| (true, ()))
        .unwrap();
    assert!(
        store
            .try_update_repo("repo1", std::time::Duration::from_millis(10), |_| (
                false,
                ()
            ))
            .is_ok()
    );
    std::thread::scope(|scope| {
        let held = store.try_update_repo("repo1", std::time::Duration::from_secs(30), |_| {
            let busy = scope
                .spawn(|| {
                    store.try_update_repo("repo1", std::time::Duration::from_millis(50), |_| {
                        (false, ())
                    })
                })
                .join()
                .unwrap();
            (false, busy.is_err())
        });
        assert!(
            held.unwrap(),
            "a second writer is refused while one holds it"
        );
    });
}

#[test]
fn sessions_teach_the_folder_facts_tools_and_envelope() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    for session in ["s1", "s2", "s3"] {
        assert_eq!(
            handle(
                &store,
                "repo1",
                Harness::Claude,
                &event("SessionStart", session),
                Utc::now()
            )
            .map(|out| out["hookSpecificOutput"]["hookEventName"].clone()),
            if session == "s1" {
                None
            } else {
                Some(json!("SessionStart"))
            }
            .filter(|_| false),
        );
        run_session(&store, "repo1", session, 6);
        assert!(store.session(session).is_none(), "folded and removed");
    }
    let state = store.repo("repo1");
    assert_eq!(state.sessions, 3);
    assert_eq!(state.envelope.samples, vec![2, 2, 2]);
    assert!(
        state
            .tools
            .deny_list(Harness::Claude, &tools::Thresholds::default())
            .contains(&"Agent".to_owned())
    );

    let out = handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("SessionStart", "s4"),
        Utc::now(),
    )
    .unwrap();
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], "SessionStart");
    let context = out["hookSpecificOutput"]["additionalContext"]
        .as_str()
        .unwrap();
    assert!(context.contains("`python -m pytest`"), "{context}");
    // Another folder learned nothing.
    assert_eq!(store.repo("repo2"), RepoState::default());
}

#[test]
fn a_redirect_answers_the_event_it_was_given() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    store.set_enabled(true).unwrap();
    let mut state = RepoState::default();
    for _ in 0..5 {
        state.envelope.add(10);
    }
    store.save_repo("repo1", &state).unwrap();
    let now = Utc::now();
    for name in ["SessionStart", "UserPromptSubmit"] {
        handle(&store, "repo1", Harness::Claude, &event(name, "s1"), now);
    }
    let mut answers = Vec::new();
    for _ in 0..8 {
        for call in [
            tool_event(
                "PostToolUse",
                "s1",
                json!({"tool_name": "Edit", "tool_input": {"file_path": "/r/a.rs"},
                       "tool_response": {}}),
            ),
            tool_event(
                "PostToolUseFailure",
                "s1",
                json!({"tool_name": "Bash", "tool_input": {"command": "cargo test"},
                       "error": "Exit code 101\nthread 'x' panicked at src/a.rs:1:1: boom"}),
            ),
            tool_event(
                "PostToolUse",
                "s1",
                json!({"tool_name": "Grep", "tool_input": {"pattern": "boom"},
                       "tool_response": {"content": "a.rs"}}),
            ),
        ] {
            if let Some(out) = handle(&store, "repo1", Harness::Claude, &call, now) {
                answers.push((call["hook_event_name"].clone(), out));
            }
        }
    }
    assert_eq!(answers.len(), 1, "one redirect per turn: {answers:?}");
    let (asked, out) = &answers[0];
    assert_eq!(out["hookSpecificOutput"]["hookEventName"], *asked);
    assert!(
        out["hookSpecificOutput"]["additionalContext"]
            .as_str()
            .unwrap()
            .contains("Automated process check")
    );
    // A stuck turn that ends teaches the envelope nothing.
    handle(&store, "repo1", Harness::Claude, &event("Stop", "s1"), now);
    assert_eq!(store.repo("repo1").envelope.samples.len(), 5);
}

// ---- launch ----

fn launch(harness: Harness, cwd: &Path) -> Launch<'_> {
    Launch {
        harness,
        version: Some(match harness {
            Harness::Claude => MIN_CLAUDE,
            Harness::Codex => MIN_CODEX,
            Harness::Opencode => MIN_OPENCODE,
        }),
        resumed: false,
        collecting: false,
        cwd,
    }
}

#[test]
fn only_a_fresh_uncollected_recent_launch_is_eligible() {
    let cwd = Path::new("/r");
    let claude = launch(Harness::Claude, cwd);
    assert!(claude.eligible());
    assert!(
        !Launch {
            version: Some((2, 1, 299)),
            ..claude
        }
        .eligible()
    );
    assert!(
        !Launch {
            version: Some((3, 0, 0)),
            ..claude
        }
        .eligible()
    );
    assert!(
        !Launch {
            version: None,
            ..claude
        }
        .eligible()
    );
    assert!(
        !Launch {
            resumed: true,
            ..claude
        }
        .eligible()
    );
    assert!(
        !Launch {
            collecting: true,
            ..claude
        }
        .eligible()
    );

    let codex = launch(Harness::Codex, cwd);
    assert_eq!(codex.eligible(), cfg!(unix));
    assert!(
        !Launch {
            version: Some((0, 163, 9)),
            ..codex
        }
        .eligible()
    );
    assert!(
        !Launch {
            collecting: true,
            ..codex
        }
        .eligible()
    );

    // OpenCode 2 could not be checked, so only 1.x from 1.19.5 runs it.
    let opencode = launch(Harness::Opencode, cwd);
    assert!(opencode.eligible());
    assert!(
        !Launch {
            version: Some((1, 19, 4)),
            ..opencode
        }
        .eligible()
    );
    assert!(
        !Launch {
            version: Some((2, 1, 4)),
            ..opencode
        }
        .eligible()
    );
    assert!(
        !Launch {
            resumed: true,
            ..opencode
        }
        .eligible()
    );
}

#[test]
fn an_opencode_launch_gets_its_plugin_program_and_tool_switches() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let cwd = dir.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let opencode = launch(Harness::Opencode, &cwd);
    let exe = Path::new("/opt/tc/near ai");
    assert!(
        super::launch::opencode(&store, exe, &opencode).is_none(),
        "off by default"
    );
    store.set_enabled(true).unwrap();
    assert!(super::launch::opencode(&store, exe, &launch(Harness::Claude, &cwd)).is_none());

    let plan = super::launch::opencode(&store, exe, &opencode).unwrap();
    let key = repo_key(&cwd, Harness::Opencode);
    let root = store.root().to_str().unwrap();
    assert_eq!(
        plan.argv,
        [
            "/opt/tc/near ai",
            "economizer-hook",
            "--state",
            root,
            "--repo",
            &key,
            "--harness",
            "opencode"
        ]
    );
    assert!(plan.tools_off.is_empty());

    let mut state = RepoState::default();
    for _ in 0..15 {
        state
            .tools
            .fold_session(Harness::Opencode, 1, &used(&["bash", "task"]));
    }
    store.save_repo(&key, &state).unwrap();
    let plan = testing::opencode(&store, exe, &opencode).unwrap();
    assert_eq!(plan.tools_off, ["todowrite", "webfetch"]);
    assert_eq!(
        plan.tools(),
        json!({"todowrite": false, "webfetch": false})
            .as_object()
            .unwrap()
            .clone()
    );

    // The settings are one JSON literal: a quote in a path stays a string.
    let odd = super::launch::OpencodePlan {
        argv: vec!["/x/\"; process.exit(1); \"".into()],
        tools_off: vec![],
    };
    let plugin = odd.plugin("ses_1");
    let line = plugin
        .lines()
        .find(|line| line.starts_with("const SETTINGS = "))
        .unwrap();
    let literal = line
        .trim_start_matches("const SETTINGS = ")
        .trim_end_matches(';');
    let settings: Value = serde_json::from_str(literal).unwrap();
    assert_eq!(settings["argv"][0], "/x/\"; process.exit(1); \"");
    assert_eq!(settings["session"], "ses_1");
}

#[test]
fn opencode_tool_calls_read_its_argument_names() {
    let edit = ToolCall::from_hook(
        &json!({"tool_name": "edit", "tool_input": {"filePath": "/r/src/a.rs"}, "tool_response": "ok"}),
        false,
    )
    .unwrap();
    assert_eq!(
        edit.kind,
        super::signals::CallKind::Edit {
            paths: vec!["/r/src/a.rs".into()]
        }
    );
    let bash = ToolCall::from_hook(
        &json!({"tool_name": "bash", "tool_input": {"command": "pytest", "description": "run"},
                "error": "bash: pytest: command not found"}),
        true,
    )
    .unwrap();
    assert!(bash.is_failure());
    assert_eq!(
        bash.kind,
        super::signals::CallKind::Command {
            command: "pytest".into()
        }
    );
}

#[test]
fn a_claude_launch_gets_every_hook_and_the_folders_deny_list() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let cwd = dir.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let claude = launch(Harness::Claude, &cwd);
    let exe = Path::new("/opt/tc/near-ai");
    assert!(
        super::launch::claude_settings(&store, exe, &claude).is_none(),
        "off by default"
    );
    store.set_enabled(true).unwrap();
    // Only Claude Code takes `--settings`.
    assert!(super::launch::claude_settings(&store, exe, &launch(Harness::Codex, &cwd)).is_none());

    let settings = testing::claude(&store, exe, &claude).unwrap();
    assert!(
        !settings.contains_key("permissions"),
        "nothing learned, nothing denied"
    );
    let hooks = settings["hooks"].as_object().unwrap();
    assert_eq!(hooks.len(), CLAUDE_EVENTS.len());
    let command = hooks["PostToolUseFailure"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    let key = repo_key(&cwd, Harness::Claude);
    assert!(
        command.starts_with("/opt/tc/near-ai economizer-hook --state "),
        "{command}"
    );
    assert!(
        command.ends_with(&format!(" --repo {key} --harness claude")),
        "{command}"
    );
    assert!(
        !command.contains("project"),
        "the folder is named by its key: {command}"
    );

    let mut state = RepoState::default();
    for _ in 0..2 {
        state
            .tools
            .fold_session(Harness::Claude, 10, &used(&["Bash"]));
    }
    store.save_repo(&key, &state).unwrap();
    let settings = testing::claude(&store, exe, &claude).unwrap();
    let deny: Vec<&str> = settings["permissions"]["deny"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    assert!(deny.contains(&"Agent"), "{deny:?}");
    assert!(!deny.contains(&"WebFetch"));
}

#[test]
fn a_codex_launch_gets_its_hooks_and_feature_switches() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let cwd = dir.path().join("project");
    std::fs::create_dir_all(&cwd).unwrap();
    let codex = launch(Harness::Codex, &cwd);
    let exe = Path::new("/opt/tc/near-ai");
    let plan = testing::codex(&store, exe, &codex).unwrap();
    let events: Vec<&str> = plan.hooks.iter().map(|hook| hook.event).collect();
    assert_eq!(events, CODEX_EVENTS);
    // Codex clamps SessionEnd to 3 s and trusts only the clamped hash.
    assert!(plan.hooks.iter().all(|hook| hook.timeout
        == if hook.event == "SessionEnd" {
            3
        } else {
            super::launch::HOOK_TIMEOUT_SECS
        }));
    assert!(
        plan.hooks[0].command.ends_with(" --harness codex"),
        "{}",
        plan.hooks[0].command
    );
    assert!(plan.feature_args().is_empty());

    // Codex learns on its own: Claude Code's evidence is not its own.
    let mut claude = RepoState::default();
    for _ in 0..20 {
        claude.tools.fold_session(Harness::Claude, 10, &used(&[]));
    }
    store
        .save_repo(&repo_key(&cwd, Harness::Claude), &claude)
        .unwrap();
    assert!(
        testing::codex(&store, exe, &codex)
            .unwrap()
            .features_off
            .is_empty()
    );

    let mut state = RepoState::default();
    for _ in 0..15 {
        state
            .tools
            .fold_session(Harness::Codex, 1, &used(&["Bash", "spawn_agent"]));
    }
    store
        .save_repo(&repo_key(&cwd, Harness::Codex), &state)
        .unwrap();
    let plan = testing::codex(&store, exe, &codex).unwrap();
    assert_eq!(
        plan.features_off,
        ["goals", "view_image", "image_generation"]
    );
    assert_eq!(
        plan.feature_args(),
        [
            "-c",
            "features.goals=false",
            "-c",
            "features.view_image=false",
            "-c",
            "features.image_generation=false"
        ]
    );
}

#[test]
fn codex_tool_names_mark_their_feature_package_used() {
    assert_eq!(
        tools::used_candidates(Harness::Codex, "spawn_agent"),
        ["multi_agent"]
    );
    assert_eq!(
        tools::used_candidates(Harness::Codex, "update_goal"),
        ["goals"]
    );
    assert_eq!(
        tools::used_candidates(Harness::Codex, "view_image"),
        ["view_image"]
    );
    assert!(tools::used_candidates(Harness::Codex, "Bash").is_empty());
    assert_eq!(tools::used_candidates(Harness::Claude, "Agent"), ["Agent"]);
    assert!(tools::used_candidates(Harness::Claude, "agent").is_empty());
}

#[test]
fn codex_patches_count_as_edits() {
    let patch = "apply_patch <<'PATCH'\n*** Begin Patch\n*** Update File: src/a.rs\n@@\n-a\n+b\n*** End Patch\nPATCH\ncargo test";
    let call = ToolCall::from_hook(
        &json!({"tool_name": "Bash", "tool_input": {"command": patch},
                "tool_response": "error: test failed"}),
        false,
    )
    .unwrap();
    assert!(call.is_failure() && call.is_check());
    let mut turn = TurnState::default();
    turn.record(&call);
    assert_eq!(turn.edit_count(), 1);
    assert!(!turn.ended_well(), "the check after the patch failed");

    let native = ToolCall::from_hook(
        &json!({"tool_name": "apply_patch", "tool_input": {"input": "*** Begin Patch\n*** Add File: b.rs\n*** Update File: c.rs\n*** End Patch"},
                "tool_response": "Success"}),
        false,
    )
    .unwrap();
    let mut turn = TurnState::default();
    turn.record(&native);
    assert_eq!(turn.edits.len(), 2);
}

// ---- summary ----

#[test]
fn the_summary_lists_each_known_folder_once_with_what_it_learned() {
    use super::summary::{ToldCommand, restore, summarize};
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let busy = dir.path().join("work").join("busy-project");
    let quiet = dir.path().join("work").join("quiet");
    let unknown = dir.path().join("work").join("never-ran");
    for folder in [&busy, &quiet, &unknown] {
        std::fs::create_dir_all(folder).unwrap();
    }

    // Busy: six sessions, so unused tools are left out, and a command
    // confirmed in three of them is told.
    let busy_key = repo_key(&busy, Harness::Claude);
    let mut state = RepoState::default();
    for n in 0..6 {
        state
            .tools
            .fold_session(Harness::Claude, 5, &used(&["Bash", "Agent"]));
        if n < 3 {
            state
                .facts
                .fold_session(&format!("s{n}"), &repair_session());
        }
        state.sessions += 1;
    }
    store.save_repo(&busy_key, &state).unwrap();
    // Quiet: one Codex session, nothing decided yet.
    let mut quiet_state = RepoState::default();
    quiet_state
        .tools
        .fold_session(Harness::Codex, 2, &used(&["shell"]));
    quiet_state.sessions = 1;
    store
        .save_repo(&repo_key(&quiet, Harness::Codex), &quiet_state)
        .unwrap();

    let summary = summarize(
        &store,
        [
            (Harness::Codex, quiet.clone()),
            (Harness::Claude, busy.clone()),
            (Harness::Claude, busy.clone()),
            (Harness::Claude, unknown.clone()),
        ],
    );
    assert!(!summary.enabled);
    assert_eq!(summary.folders_truncated, 0);
    assert_eq!(summary.folders.len(), 2, "{summary:?}");
    let first = &summary.folders[0];
    assert_eq!(first.folder, "busy-project");
    assert_eq!(first.harness, Harness::Claude);
    assert_eq!(first.key, busy_key);
    assert_eq!(first.sessions, 6);
    assert!(first.left_out.contains(&"TodoWrite".to_owned()));
    assert!(!first.left_out.contains(&"Agent".to_owned()), "used here");
    assert_eq!(
        first.told,
        [ToldCommand {
            program: "pytest".into(),
            prefix: "python -m".into()
        }]
    );
    assert_eq!(first.forming, 0);
    assert!(first.given_back.is_empty());
    let second = &summary.folders[1];
    assert_eq!(
        (second.folder.as_str(), second.harness),
        ("quiet", Harness::Codex)
    );
    assert!(second.left_out.is_empty());

    // Nothing crosses but the label and the digest.
    let wire = serde_json::to_string(&summary).unwrap();
    assert!(!wire.contains(&dir.path().to_string_lossy().into_owned()));

    // Giving back clears the left-out list and keeps the tools out of it.
    let given = restore(&store, Harness::Claude, &busy_key).unwrap();
    assert_eq!(given, first.left_out);
    let after = summarize(&store, [(Harness::Claude, busy.clone())]);
    assert!(after.folders[0].left_out.is_empty());
    assert_eq!(after.folders[0].given_back, given);
    assert!(
        restore(&store, Harness::Claude, &busy_key)
            .unwrap()
            .is_empty()
    );
    // A name that cannot be a store key is refused; a well-formed one
    // that names no folder gives back nothing.
    assert!(restore(&store, Harness::Claude, "../escape").is_err());
    assert!(
        restore(&store, Harness::Claude, "repo-unknown")
            .unwrap()
            .is_empty()
    );
}

#[test]
fn update_repo_writes_only_when_asked_and_serializes_writers() {
    const LONG: std::time::Duration = std::time::Duration::from_secs(30);
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let key = repo_key(Path::new("/home/a/project"), Harness::Claude);
    let read = store
        .try_update_repo(&key, LONG, |state| (false, state.sessions))
        .unwrap();
    assert_eq!(read, 0);
    assert_eq!(store.repo(&key), RepoState::default());
    assert!(
        store
            .try_update_repo("../escape", LONG, |_| (true, ()))
            .is_err()
    );

    // Writers in parallel each add one session; none is lost.
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                store
                    .try_update_repo(&key, LONG, |state| {
                        state.sessions += 1;
                        (true, ())
                    })
                    .unwrap();
            });
        }
    });
    assert_eq!(store.repo(&key).sessions, 8);
}

#[test]
fn folders_with_the_same_name_are_told_apart_by_their_parent() {
    use super::summary::summarize;
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let work = dir.path().join("work").join("api");
    let oss = dir.path().join("oss").join("api");
    for folder in [&work, &oss] {
        std::fs::create_dir_all(folder).unwrap();
        let state = RepoState {
            sessions: 2,
            ..RepoState::default()
        };
        store
            .save_repo(&repo_key(folder, Harness::Claude), &state)
            .unwrap();
    }
    let labels = |summary: super::summary::Summary| -> Vec<String> {
        summary.folders.into_iter().map(|row| row.folder).collect()
    };
    let first = labels(summarize(
        &store,
        [
            (Harness::Claude, work.clone()),
            (Harness::Claude, oss.clone()),
        ],
    ));
    assert_eq!(first, ["oss/api", "work/api"]);
    // The same order whatever order the folders were named in.
    let again = labels(summarize(
        &store,
        [
            (Harness::Claude, oss.clone()),
            (Harness::Claude, work.clone()),
        ],
    ));
    assert_eq!(again, first);
    // A lone folder keeps its own name.
    assert_eq!(
        labels(summarize(&store, [(Harness::Claude, work)])),
        ["api"]
    );
}

#[test]
fn labels_grow_until_they_differ_and_never_name_the_home_folder() {
    use super::summary::summarize_in;
    let dir = tempfile::tempdir().unwrap();
    // Resolved first: on macOS the temp folder sits under `/var`, itself a
    // symlink to `/private/var`.
    let root = dir.path().canonicalize().unwrap();
    let store = Store::new(&root);
    let home = root.join("someone");
    let deep_a = home.join("a").join("x").join("api");
    let deep_b = home.join("b").join("x").join("api");
    let at_home = home.join("api");
    let elsewhere = root.join("srv").join("api");
    for folder in [&deep_a, &deep_b, &at_home, &elsewhere] {
        std::fs::create_dir_all(folder).unwrap();
        let state = RepoState {
            sessions: 1,
            ..RepoState::default()
        };
        store
            .save_repo(&repo_key(folder, Harness::Claude), &state)
            .unwrap();
    }
    let labels = |folders: Vec<&std::path::PathBuf>| -> Vec<String> {
        let mut out: Vec<String> = summarize_in(
            &store,
            folders
                .into_iter()
                .map(|path| (Harness::Claude, path.clone())),
            Some(&home),
        )
        .folders
        .into_iter()
        .map(|row| row.folder)
        .collect();
        out.sort();
        out
    };
    // One parent is not enough for these two; two are.
    assert_eq!(labels(vec![&deep_a, &deep_b]), ["a/x/api", "b/x/api"]);
    // A folder right under home shows `~`, never the account name.
    assert_eq!(labels(vec![&at_home, &elsewhere]), ["srv/api", "~/api"]);
    let all = labels(vec![&deep_a, &deep_b, &at_home, &elsewhere]);
    assert!(
        all.iter().all(|label| !label.contains("someone")),
        "{all:?}"
    );
    let unique: std::collections::BTreeSet<_> = all.iter().collect();
    assert_eq!(unique.len(), 4, "{all:?}");

    // Home given through a symlink still matches folders recorded under
    // its resolved path.
    #[cfg(unix)]
    {
        let link = root.join("home-link");
        std::os::unix::fs::symlink(&home, &link).unwrap();
        let mut through_link: Vec<String> = summarize_in(
            &store,
            [
                (Harness::Claude, at_home.clone()),
                (Harness::Claude, elsewhere.clone()),
            ],
            Some(&link),
        )
        .folders
        .into_iter()
        .map(|row| row.folder)
        .collect();
        through_link.sort();
        assert_eq!(through_link, ["srv/api", "~/api"]);

        // And the other way: home resolved, the folder recorded through
        // the symlink.
        let mut recorded_through_link: Vec<String> = summarize_in(
            &store,
            [
                (Harness::Claude, link.join("api")),
                (Harness::Claude, elsewhere.clone()),
            ],
            Some(&home),
        )
        .folders
        .into_iter()
        .map(|row| row.folder)
        .collect();
        recorded_through_link.sort();
        assert_eq!(recorded_through_link, ["srv/api", "~/api"]);

        // A project inside home that links out of it keeps its `~`.
        let outside = root.join("volume").join("someone").join("work");
        std::fs::create_dir_all(outside.join("api")).unwrap();
        let linked_work = home.join("work");
        std::os::unix::fs::symlink(&outside, &linked_work).unwrap();
        let linked_api = linked_work.join("api");
        let state = RepoState {
            sessions: 1,
            ..RepoState::default()
        };
        store
            .save_repo(&repo_key(&linked_api, Harness::Claude), &state)
            .unwrap();
        let other_work = root.join("x").join("work").join("api");
        std::fs::create_dir_all(&other_work).unwrap();
        store
            .save_repo(&repo_key(&other_work, Harness::Claude), &state)
            .unwrap();
        let mut linked_out: Vec<String> = summarize_in(
            &store,
            [
                (Harness::Claude, linked_api.clone()),
                (Harness::Claude, other_work.clone()),
            ],
            Some(&home),
        )
        .folders
        .into_iter()
        .map(|row| row.folder)
        .collect();
        linked_out.sort();
        assert_eq!(linked_out, ["x/work/api", "~/work/api"]);

        // Named both through the link and directly, a folder keeps the
        // spelling under home, whatever the order.
        let direct = outside.join("api");
        for order in [
            [linked_api.clone(), direct.clone()],
            [direct.clone(), linked_api.clone()],
        ] {
            let mut labels: Vec<String> = summarize_in(
                &store,
                order
                    .into_iter()
                    .chain([other_work.clone()])
                    .map(|path| (Harness::Claude, path)),
                Some(&home),
            )
            .folders
            .into_iter()
            .map(|row| row.folder)
            .collect();
            labels.sort();
            assert_eq!(labels, ["x/work/api", "~/work/api"]);
        }
    }
}

#[test]
fn giving_back_in_an_unknown_folder_leaves_no_lock_file() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let key = repo_key(dir.path(), Harness::Claude);
    let given = super::summary::restore(&store, Harness::Claude, &key).unwrap();
    assert!(given.is_empty());
    assert!(!store.root().join("repos").exists());
}

#[test]
fn the_hook_gives_up_on_a_held_lock_and_writes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::new(dir.path());
    let key = repo_key(dir.path(), Harness::Claude);
    store.save_repo(&key, &RepoState::default()).unwrap();
    let held = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(store.root().join("repos").join(format!("{key}.lock")))
        .unwrap();
    held.lock().unwrap();
    let started = std::time::Instant::now();
    let tried = store.try_update_repo(&key, std::time::Duration::from_millis(100), |state| {
        state.sessions += 1;
        (true, ())
    });
    assert!(tried.is_err());
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(store.repo(&key).sessions, 0);
    drop(held);
    store
        .try_update_repo(&key, std::time::Duration::from_millis(100), |state| {
            state.sessions += 1;
            (true, ())
        })
        .unwrap();
    assert_eq!(store.repo(&key).sessions, 1);
}
