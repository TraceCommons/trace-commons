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
    handle(
        &store,
        "repo1",
        Harness::Claude,
        &event("UserPromptSubmit", "s1"),
        now,
    );
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
    assert!(
        restore(&store, Harness::Claude, "../escape")
            .unwrap()
            .is_empty()
    );
}
