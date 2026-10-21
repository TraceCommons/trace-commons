use serde_json::Value;
use trace_commons_protocol::insights_usage_series::{
    KeyedDigest, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
};

use super::*;
use crate::insights::analytics_constants::CLASSIFIER_VERSION;
use crate::insights::session_facts::{ExtensionClassCounts, ModelFacts, SessionShape};

fn facts() -> SessionFacts {
    SessionFacts {
        models: vec![
            ModelFacts {
                label_ix: 0,
                edits: Some(3),
                denied: Some(1),
                undone: Some(0),
                interrupted: Some(1),
            },
            ModelFacts {
                label_ix: 1,
                edits: Some(2),
                denied: None,
                undone: Some(1),
                interrupted: Some(0),
            },
        ],
        shape: SessionShape {
            edited_files: ExtensionClassCounts {
                test: 1,
                doc: 0,
                source: 2,
                config: 0,
            },
            renamed_files: 0,
            lines_added: Some(40),
            lines_removed: Some(12),
            failed_before_first_edit_then_passed: 1,
        },
        task: InferredTask::BugFix,
        classifier_version: CLASSIFIER_VERSION,
        active_secs: Some(600),
        finished: Some(true),
        user_turns: Some(5),
        edit_mode: EditMode::Prompting,
        capped: false,
        delegated_unread: false,
        reviewer: false,
    }
}

fn prompts(n: usize) -> Vec<Scrubbed> {
    (0..n).map(|i| Scrubbed(format!("prompt {i}"))).collect()
}

fn ev(prompt_ix: u32, kind: EventKind) -> EventMarker {
    EventMarker { prompt_ix, kind }
}

fn input<'a>(
    facts: &'a SessionFacts,
    prompts: &'a [Scrubbed],
    final_line: Option<&'a Scrubbed>,
    exts: &'a [&'a str],
    events: &'a [EventMarker],
) -> DigestInput<'a> {
    DigestInput {
        facts,
        series: None,
        prompts,
        final_line,
        edited_extensions: exts,
        events,
    }
}

// ---- selection ----

#[test]
fn twelve_or_fewer_prompts_are_all_sent_in_order() {
    assert_eq!(select_prompts(0, &[]), Vec::<usize>::new());
    assert_eq!(select_prompts(5, &[]), vec![0, 1, 2, 3, 4]);
    assert_eq!(select_prompts(12, &[]), (0..12).collect::<Vec<_>>());
    // Events change nothing when everything fits.
    assert_eq!(
        select_prompts(12, &[ev(6, EventKind::Denial)]),
        (0..12).collect::<Vec<_>>()
    );
}

#[test]
fn more_than_twelve_without_events_fills_the_middle_evenly() {
    // First 4, last 4, and 4 spread evenly over the 22 middle prompts 4..=25.
    assert_eq!(
        select_prompts(30, &[]),
        vec![0, 1, 2, 3, 6, 12, 17, 23, 26, 27, 28, 29]
    );
    // Five middle prompts 4..=8: four of them.
    assert_eq!(
        select_prompts(13, &[]),
        vec![0, 1, 2, 3, 4, 5, 7, 8, 9, 10, 11, 12]
    );
    assert_eq!(
        select_prompts(20, &[]),
        vec![0, 1, 2, 3, 5, 8, 11, 14, 16, 17, 18, 19]
    );
    for n in 13..200 {
        let picked = select_prompts(n, &[]);
        assert_eq!(picked.len(), MAX_PROMPT_EXCERPTS, "n {n}");
        assert!(picked.windows(2).all(|w| w[0] < w[1]), "n {n}");
    }
}

#[test]
fn events_take_slots_first_and_the_fill_takes_the_rest() {
    // Window 2..=5 adds 4 and 5; the two slots left are spread over the
    // unchosen middle prompts 6..=25.
    assert_eq!(
        select_prompts(30, &[ev(5, EventKind::Undo)]),
        vec![0, 1, 2, 3, 4, 5, 11, 21, 26, 27, 28, 29]
    );
    // A full set of event picks leaves nothing to fill.
    assert_eq!(
        select_prompts(30, &[ev(15, EventKind::Interrupt)]),
        vec![0, 1, 2, 3, 12, 13, 14, 15, 26, 27, 28, 29]
    );
}

#[test]
fn an_event_adds_the_four_prompts_up_to_its_own() {
    // The event happened in the turn answering prompt 15: send 12..=15.
    assert_eq!(
        select_prompts(30, &[ev(15, EventKind::Interrupt)]),
        vec![0, 1, 2, 3, 12, 13, 14, 15, 26, 27, 28, 29]
    );
}

#[test]
fn event_windows_overlapping_the_edges_or_each_other_deduplicate() {
    // Two events with overlapping windows 10..=13 and 12..=15.
    assert_eq!(
        select_prompts(30, &[ev(13, EventKind::Denial), ev(15, EventKind::Denial)]),
        vec![0, 1, 2, 3, 10, 11, 12, 13, 26, 27, 28, 29]
    );
}

#[test]
fn many_events_never_exceed_twelve_and_earlier_events_win() {
    let events = [
        ev(10, EventKind::Denial),
        ev(20, EventKind::Interrupt),
        ev(30, EventKind::Undo),
    ];
    let picked = select_prompts(50, &events);
    assert_eq!(picked.len(), MAX_PROMPT_EXCERPTS);
    assert_eq!(picked, vec![0, 1, 2, 3, 7, 8, 9, 10, 46, 47, 48, 49]);
    // Events out of session order are taken in session order.
    let reversed = [events[2], events[1], events[0]];
    assert_eq!(select_prompts(50, &reversed), picked);
}

#[test]
fn an_event_past_the_last_prompt_is_ignored() {
    assert_eq!(
        select_prompts(20, &[ev(99, EventKind::Denial)]),
        select_prompts(20, &[])
    );
}

#[test]
fn an_event_window_at_the_start_does_not_underflow() {
    assert_eq!(
        select_prompts(20, &[ev(1, EventKind::Denial)]),
        select_prompts(20, &[])
    );
}

#[test]
fn the_digest_carries_selected_excerpts_with_their_positions() {
    let facts = facts();
    let p = prompts(30);
    let events = [ev(15, EventKind::Interrupt)];
    let built = build_judgement_digest(&input(&facts, &p, None, &[], &events)).unwrap();
    let ixs: Vec<u32> = built.digest.excerpts.iter().map(|e| e.ix).collect();
    assert_eq!(ixs, vec![0, 1, 2, 3, 12, 13, 14, 15, 26, 27, 28, 29]);
    assert_eq!(built.digest.excerpts[4].text, "prompt 12");
    assert_eq!(built.digest.prompts_total, 30);
    assert_eq!(built.digest.events, events.to_vec());
    assert_eq!(built.digest.excerpts_dropped, 0);
}

// ---- truncation ----

#[test]
fn truncation_counts_characters_not_bytes() {
    assert_eq!(truncate_chars("abc", 5), "abc");
    assert_eq!(truncate_chars("abcdef", 3), "abc");
    // 239 ASCII characters then a two-byte and a four-byte character.
    let text = format!("{}é😀tail", "a".repeat(239));
    let cut = truncate_chars(&text, 240);
    assert_eq!(cut.chars().count(), 240);
    assert!(cut.ends_with('é'));
    let cut = truncate_chars(&text, 241);
    assert_eq!(cut.chars().count(), 241);
    assert!(cut.ends_with('😀'));
}

#[test]
fn prompts_are_cut_to_240_characters_and_the_final_line_to_160() {
    let facts = facts();
    let p = vec![
        Scrubbed("x".repeat(500)),
        Scrubbed("😀".repeat(300)),
        Scrubbed("short".to_owned()),
    ];
    let fin = Scrubbed(format!("{}\nsecond line never sent", "é".repeat(200)));
    let built = build_judgement_digest(&input(&facts, &p, Some(&fin), &[], &[])).unwrap();
    let ex = &built.digest.excerpts;
    assert_eq!(ex[0].text.chars().count(), PROMPT_EXCERPT_CHARS);
    assert_eq!(ex[1].text, "😀".repeat(240));
    assert_eq!(ex[2].text, "short");
    let line = built.digest.final_line.as_deref().unwrap();
    assert_eq!(line, "é".repeat(160));
    assert!(
        !String::from_utf8(built.bytes.clone())
            .unwrap()
            .contains("second line")
    );
}

#[test]
fn only_the_first_line_of_a_short_final_message_is_sent() {
    let facts = facts();
    let fin = Scrubbed("Done.\r\nDetails follow".to_owned());
    let built = build_judgement_digest(&input(&facts, &[], Some(&fin), &[], &[])).unwrap();
    assert_eq!(built.digest.final_line.as_deref(), Some("Done."));
}

// ---- byte cap ----

#[test]
fn over_the_cap_whole_excerpts_drop_from_the_middle_out() {
    let facts = facts();
    // Twelve excerpts of 240 four-byte characters: about 11.5 KiB.
    let p: Vec<Scrubbed> = (0..12)
        .map(|i| Scrubbed(format!("{i:02}{}", "😀".repeat(300))))
        .collect();
    let built = build_judgement_digest(&input(&facts, &p, None, &[], &[])).unwrap();
    assert!(
        built.bytes.len() <= DIGEST_BYTE_CAP,
        "{}",
        built.bytes.len()
    );
    let kept: Vec<u32> = built.digest.excerpts.iter().map(|e| e.ix).collect();
    let dropped = built.digest.excerpts_dropped as usize;
    assert!(dropped > 0);
    assert_eq!(kept.len() + dropped, 12);
    // What survives is a prefix and a suffix of the selection, balanced
    // within one.
    let head = kept.iter().take_while(|&&ix| (ix as usize) < 6).count();
    let tail = kept.len() - head;
    assert_eq!(&kept[..head], &(0..head as u32).collect::<Vec<_>>()[..]);
    assert_eq!(
        &kept[head..],
        &((12 - tail) as u32..12).collect::<Vec<_>>()[..]
    );
    assert!(head.abs_diff(tail) <= 1, "head {head} tail {tail}");
    // Every surviving excerpt is whole.
    for ex in &built.digest.excerpts {
        assert_eq!(ex.text.chars().count(), PROMPT_EXCERPT_CHARS);
    }
    // One more excerpt would not have fit.
    assert!(built.bytes.len() + 4 * 240 > DIGEST_BYTE_CAP);
}

#[test]
fn under_the_cap_nothing_is_dropped() {
    let facts = facts();
    let p: Vec<Scrubbed> = (0..12).map(|_| Scrubbed("a".repeat(240))).collect();
    let built = build_judgement_digest(&input(&facts, &p, None, &[], &[])).unwrap();
    assert_eq!(built.digest.excerpts.len(), 12);
    assert_eq!(built.digest.excerpts_dropped, 0);
}

#[test]
fn the_largest_digest_without_excerpts_fits_the_cap() {
    // Every bounded field at its bound: the refusal case stays out of reach.
    let mut facts = facts();
    facts.models = (0..8)
        .map(|i| ModelFacts {
            label_ix: i,
            edits: Some(u32::MAX),
            denied: Some(u32::MAX),
            undone: Some(u32::MAX),
            interrupted: Some(u32::MAX),
        })
        .collect();
    let mut series = UsageSeries::default();
    for ordinal in 0..200u32 {
        series.push_turn(turn(ordinal));
        for tool in [
            ToolKind::Read,
            ToolKind::Edit,
            ToolKind::Write,
            ToolKind::MultiEdit,
            ToolKind::NotebookEdit,
            ToolKind::Bash,
            ToolKind::Grep,
            ToolKind::Glob,
            ToolKind::ApplyPatch,
            ToolKind::Other,
        ] {
            series.push_tool_call(call(u32::MAX - 200 + ordinal, tool, Some(false)));
        }
    }
    let events: Vec<EventMarker> = (0..500)
        .map(|i| ev(u32::MAX - i, EventKind::Interrupt))
        .collect();
    let fin = Scrubbed("😀".repeat(400));
    let exts = vec!["rs"; 1000];
    let built = build_judgement_digest(&DigestInput {
        facts: &facts,
        series: Some(&series),
        prompts: &[],
        final_line: Some(&fin),
        edited_extensions: &exts,
        events: &events,
    })
    .unwrap();
    assert!(built.digest.turn_outcomes_truncated);
    assert!(built.digest.events_truncated);
    assert_eq!(built.digest.turn_outcomes.len(), MAX_TURN_OUTCOMES);
    assert_eq!(built.digest.events.len(), MAX_DIGEST_EVENTS);
    assert!(
        built.bytes.len() <= DIGEST_BYTE_CAP,
        "{}",
        built.bytes.len()
    );
}

// ---- determinism and hash ----

#[test]
fn the_same_input_gives_the_same_bytes_and_hash() {
    let facts = facts();
    let series = sample_series();
    let p = prompts(20);
    let fin = Scrubbed("All tests pass.".to_owned());
    let exts = ["rs", "md", "toml"];
    let events = [ev(9, EventKind::Denial), ev(3, EventKind::Undo)];
    let make = || {
        build_judgement_digest(&DigestInput {
            facts: &facts,
            series: Some(&series),
            prompts: &p,
            final_line: Some(&fin),
            edited_extensions: &exts,
            events: &events,
        })
        .unwrap()
    };
    let a = make();
    let b = make();
    assert_eq!(a.bytes, b.bytes);
    assert_eq!(a.sha256, b.sha256);
    let expected: [u8; 32] = Sha256::digest(&a.bytes).into();
    assert_eq!(a.sha256, expected);
    assert_eq!(a.sha256_hex(), hex::encode(expected));
    // The bytes are the serialized digest, and round-trip.
    let back: JudgementDigest = serde_json::from_slice(&a.bytes).unwrap();
    assert_eq!(back, a.digest);
    assert_eq!(back.schema_version, JUDGEMENT_DIGEST_VERSION);
    // Events are carried in session order.
    assert_eq!(back.events, vec![events[1], events[0]]);
}

#[test]
fn a_changed_excerpt_changes_the_hash() {
    let facts = facts();
    let a = prompts(3);
    let mut b = prompts(3);
    b[1] = Scrubbed("prompt one".to_owned());
    let ha = build_judgement_digest(&input(&facts, &a, None, &[], &[])).unwrap();
    let hb = build_judgement_digest(&input(&facts, &b, None, &[], &[])).unwrap();
    assert_ne!(ha.sha256, hb.sha256);
}

// ---- counts ----

#[test]
fn counts_sum_models_and_read_the_series() {
    let facts = facts();
    let series = sample_series();
    let built = build_judgement_digest(&DigestInput {
        facts: &facts,
        series: Some(&series),
        prompts: &[],
        final_line: None,
        edited_extensions: &["rs", ".md", "src/lib.rs", "Cargo.toml", "tests/a.rs"],
        events: &[],
    })
    .unwrap();
    let c = &built.digest.counts;
    assert_eq!(c.edits, Some(5));
    assert_eq!(c.denied, None, "one model's denials are unknown");
    assert_eq!(c.undone, Some(1));
    assert_eq!(c.interrupted, Some(1));
    assert_eq!(c.task, InferredTask::BugFix);
    assert_eq!(
        c.stored_edited_files,
        FileClassCounts {
            test: 1,
            doc: 0,
            source: 2,
            config: 0,
        }
    );
    assert_eq!(c.tool_kinds.get(&ToolKind::Edit), Some(&2));
    assert_eq!(c.tool_kinds.get(&ToolKind::Bash), Some(&3));
    assert_eq!(c.tool_kinds.get(&ToolKind::Read), Some(&1));
    assert_eq!(
        (c.commands_passed, c.commands_failed, c.commands_unknown),
        (1, 1, 1)
    );
    // Turn 0 only read; turns 1 and 2 edited or ran commands.
    assert_eq!(
        built.digest.turn_outcomes,
        vec![TurnOutcome(1, 2, 1, 0, 1), TurnOutcome(2, 0, 0, 1, 0)]
    );
    assert_eq!(
        built.digest.edited_file_classes,
        FileClassCounts {
            test: 1,
            doc: 1,
            source: 2,
            config: 1,
        }
    );
}

#[test]
fn extension_classes_accept_bare_dotted_and_path_forms() {
    assert_eq!(extension_class_of("md"), ExtensionClass::Doc);
    assert_eq!(extension_class_of(".md"), ExtensionClass::Doc);
    assert_eq!(extension_class_of("MD"), ExtensionClass::Doc);
    assert_eq!(extension_class_of("toml"), ExtensionClass::Config);
    assert_eq!(extension_class_of("rs"), ExtensionClass::Source);
    assert_eq!(extension_class_of(""), ExtensionClass::Source);
    assert_eq!(
        extension_class_of("/Users/alice/code/acme-secret/README.md"),
        ExtensionClass::Doc
    );
    assert_eq!(
        extension_class_of("C:\\Users\\alice\\acme-secret\\tests\\a.rs"),
        ExtensionClass::Test
    );
}

// ---- privacy over the serialized bytes ----

const HOME: &str = "/Users/alice";
const REPO: &str = "acme-secret-repo";
const EMAIL: &str = "alice@example.com";
const HEX: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

#[test]
fn serialized_bytes_hold_no_path_home_repo_email_or_identifier() {
    let facts = facts();
    let key = KeyedDigest([0xab; 32]);
    let path = format!("{HOME}/code/{REPO}/src/main.rs");
    let mut series = UsageSeries::default();
    series.push_turn(TurnRecord {
        msg_key: Some(key),
        ..turn(0)
    });
    series.push_tool_call(ToolCallRecord {
        turn_ordinal: 0,
        tool: ToolKind::Edit,
        args_key: key,
        path_key: Some(key),
        // A caller stuffing a path where an extension belongs.
        path_ext: Some(path.clone()),
        result_bytes: Some(10),
        success: Some(true),
        paired: true,
    });
    let exts: Vec<&str> = vec![
        &path,
        HOME,
        REPO,
        EMAIL,
        HEX,
        "~/acme-secret-repo/notes.md",
        "git@github.com:alice/acme-secret-repo.git",
    ];
    let p = vec![Scrubbed("fix the failing test".to_owned())];
    let fin = Scrubbed("Fixed it.".to_owned());
    let built = build_judgement_digest(&DigestInput {
        facts: &facts,
        series: Some(&series),
        prompts: &p,
        final_line: Some(&fin),
        edited_extensions: &exts,
        events: &[ev(0, EventKind::Denial)],
    })
    .unwrap();
    let text = String::from_utf8(built.bytes.clone()).unwrap();
    for needle in [
        HOME,
        "/Users",
        "alice",
        REPO,
        "acme",
        EMAIL,
        "@",
        HEX,
        "9f86d0",
        "main.rs",
        "src/",
        "~/",
        "github",
        &hex::encode([0xab; 32]),
        "abababab",
    ] {
        assert!(!text.contains(needle), "{needle:?} leaked into {text}");
    }
    // Every string in the digest is an excerpt, the final line, or a fixed
    // label; nothing else carries text.
    let value: Value = serde_json::from_slice(&built.bytes).unwrap();
    let mut strings = Vec::new();
    collect_strings(&value, "", &mut strings);
    for (path, s) in strings {
        let allowed = path.starts_with("excerpts.") && path.ends_with(".text")
            || path == "final_line"
            || FIXED_LABELS.contains(&s.as_str());
        assert!(allowed, "free text {s:?} at {path}");
    }
    // Map keys are tool kinds only.
    let kinds = value["counts"]["tool_kinds"].as_object().unwrap();
    for k in kinds.keys() {
        assert!(FIXED_LABELS.contains(&k.as_str()), "{k}");
    }
}

const FIXED_LABELS: &[&str] = &[
    "bug_fix",
    "refactor",
    "tests",
    "docs",
    "other",
    "no_edits",
    "prompting",
    "not_prompting",
    "unknown",
    "denial",
    "interrupt",
    "undo",
    "read",
    "edit",
    "write",
    "multi_edit",
    "notebook_edit",
    "bash",
    "grep",
    "glob",
    "apply_patch",
];

fn collect_strings(value: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match value {
        Value::String(s) => out.push((path.to_owned(), s.clone())),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                collect_strings(item, &format!("{path}.{i}"), out);
            }
        }
        Value::Object(map) => {
            for (k, v) in map {
                let p = if path.is_empty() {
                    k.clone()
                } else {
                    format!("{path}.{k}")
                };
                collect_strings(v, &p, out);
            }
        }
        _ => {}
    }
}

// ---- fixtures ----

fn turn(ordinal: u32) -> TurnRecord {
    TurnRecord {
        ordinal,
        at: None,
        uncached_input: Some(1),
        cache_read: Some(0),
        cache_write: None,
        output: Some(1),
        model_label_ix: Some(0),
        gap_secs: None,
        msg_key: None,
    }
}

fn call(turn_ordinal: u32, tool: ToolKind, success: Option<bool>) -> ToolCallRecord {
    ToolCallRecord {
        turn_ordinal,
        tool,
        args_key: KeyedDigest([1; 32]),
        path_key: None,
        path_ext: None,
        result_bytes: None,
        success,
        paired: success.is_some(),
    }
}

fn sample_series() -> UsageSeries {
    let mut s = UsageSeries::default();
    for ordinal in 0..3 {
        s.push_turn(turn(ordinal));
    }
    s.push_tool_call(call(0, ToolKind::Read, Some(true)));
    s.push_tool_call(call(1, ToolKind::Edit, Some(true)));
    s.push_tool_call(call(1, ToolKind::Edit, Some(false)));
    s.push_tool_call(call(1, ToolKind::Bash, Some(false)));
    s.push_tool_call(call(1, ToolKind::Bash, None));
    s.push_tool_call(call(2, ToolKind::Bash, Some(true)));
    s
}

#[test]
fn per_model_sums_are_unknown_when_no_model_was_declared() {
    let mut facts = facts();
    facts.models.clear();
    let built = build_judgement_digest(&input(&facts, &[], None, &[], &[])).unwrap();
    let c = &built.digest.counts;
    assert_eq!(
        (c.edits, c.denied, c.undone, c.interrupted),
        (None, None, None, None)
    );
}

#[test]
fn events_at_one_prompt_serialize_the_same_in_any_input_order() {
    let facts = facts();
    let a = [ev(2, EventKind::Undo), ev(2, EventKind::Denial)];
    let b = [ev(2, EventKind::Denial), ev(2, EventKind::Undo)];
    let ba = build_judgement_digest(&input(&facts, &[], None, &[], &a)).unwrap();
    let bb = build_judgement_digest(&input(&facts, &[], None, &[], &b)).unwrap();
    assert_eq!(ba.bytes, bb.bytes);
}
