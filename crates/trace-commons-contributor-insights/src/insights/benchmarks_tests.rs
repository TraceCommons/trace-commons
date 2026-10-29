use chrono::TimeZone;
use trace_commons_protocol::insights_pricing::{
    ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION, PricingCurrency, PricingTable,
    PricingTableProvenance, TokenRate,
};
use trace_commons_protocol::insights_usage_series::{
    CacheWrite, KeyedDigest, ToolCallRecord, ToolKind, TurnRecord, UsageSeries,
};

use super::*;
use crate::insights::analytics_constants::CLASSIFIER_VERSION;
use crate::insights::session_facts::{EditMode, ExtensionClassCounts, ModelFacts};
use crate::insights::session_labels::SessionOutcome;
use crate::insights::session_pricing::pricing_provider;

const MILLION: u32 = 1_000_000;
const USD_NANOS: u64 = 1_000_000_000;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 29, 12, 0, 0).unwrap()
}

fn day_ago() -> DateTime<Utc> {
    now() - Duration::days(1)
}

fn entry(accounting: PricingAccounting, model: &str, usd_per_million: u64) -> ModelPriceEntry {
    ModelPriceEntry {
        provider: pricing_provider(accounting).to_owned(),
        model: model.to_owned(),
        accounting,
        effective_from: Utc.timestamp_opt(0, 0).unwrap(),
        effective_until: None,
        prompt_tokens_above: None,
        prompt_tokens_at_most: None,
        rates: accounting
            .required_categories()
            .iter()
            .map(|category| TokenRate {
                category: *category,
                usd_nanos_per_million_tokens: usd_per_million * USD_NANOS,
            })
            .collect(),
    }
}

/// model-a at 1 USD, model-b at 3 USD, model-c at 5 USD per million tokens.
fn catalog() -> PricingCatalog {
    let mut entries = Vec::new();
    for accounting in [PricingAccounting::ClaudeCode, PricingAccounting::Codex] {
        entries.push(entry(accounting, "model-a", 1));
        entries.push(entry(accounting, "model-b", 3));
        entries.push(entry(accounting, "model-c", 5));
    }
    let mut table = PricingTable {
        schema_version: PRICING_TABLE_SCHEMA_VERSION,
        table_id: String::new(),
        version: 1,
        currency: PricingCurrency::Usd,
        published_at: day_ago() - Duration::hours(12),
        provenance: PricingTableProvenance {
            publisher: "Synthetic fixture".to_owned(),
            source_url: "https://example.invalid/prices/1".to_owned(),
            retrieved_at: day_ago() - Duration::days(1),
            content_sha256:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            reviewed_by: "Synthetic reviewer".to_owned(),
            reviewed_at: day_ago() - Duration::hours(12),
            effective_from_basis: None,
            source_notes: None,
        },
        entries,
    };
    table.table_id = table.canonical_table_id().unwrap();
    PricingCatalog::from_tables(vec![table]).unwrap()
}

fn turn(ordinal: u32, label: Option<u16>, tokens: u32, at: DateTime<Utc>) -> TurnRecord {
    TurnRecord {
        ordinal,
        at: Some(at),
        uncached_input: Some(tokens),
        cache_read: Some(0),
        cache_write: Some(CacheWrite::Split { m5: 0, h1: 0 }),
        output: Some(0),
        model_label_ix: label,
        gap_secs: None,
        msg_key: None,
    }
}

fn shape(files: ExtensionClassCounts) -> SessionShape {
    SessionShape {
        edited_files: files,
        renamed_files: 0,
        lines_added: Some(0),
        lines_removed: Some(0),
        failed_before_first_edit_then_passed: 0,
    }
}

fn facts(task: InferredTask, models: Vec<ModelFacts>) -> SessionFacts {
    SessionFacts {
        models,
        shape: shape(ExtensionClassCounts {
            source: 1,
            ..ExtensionClassCounts::default()
        }),
        task,
        classifier_version: CLASSIFIER_VERSION,
        active_secs: Some(600),
        finished: Some(true),
        user_turns: Some(3),
        edit_mode: EditMode::NotPrompting,
        capped: false,
        delegated_unread: false,
        reviewer: false,
    }
}

fn mf(edits: Option<u32>, denied: Option<u32>, undone: Option<u32>) -> ModelFacts {
    ModelFacts {
        label_ix: 0,
        edits,
        denied,
        undone,
        interrupted: Some(0),
    }
}

/// A Claude session served wholly by `label`, `turns` turns of a million
/// uncached input tokens each.
fn session(id: &str, label: &str, turns: u32, facts: Option<SessionFacts>) -> SessionInput {
    SessionInput {
        session_ref: id.to_owned(),
        import_seq: 0,
        placed_at: None,
        harness_session: None,
        automated: false,
        body: SessionBody::Claude {
            series: UsageSeries {
                turns: (0..turns)
                    .map(|ix| turn(ix, Some(0), MILLION, day_ago()))
                    .collect(),
                ..UsageSeries::default()
            },
            model_labels: vec![label.to_owned()],
        },
        facts,
    }
}

/// A bug fix by `label`: `edits` edits, `undone` undone, none denied,
/// `active` seconds, `turns` million-token turns.
fn fix(id: &str, label: &str, edits: u32, undone: u32, active: u32, turns: u32) -> SessionInput {
    let mut f = facts(
        InferredTask::BugFix,
        vec![mf(Some(edits), Some(0), Some(undone))],
    );
    f.active_secs = Some(active);
    session(id, label, turns, Some(f))
}

fn many(prefix: &str, n: usize, make: impl Fn(String) -> SessionInput) -> Vec<SessionInput> {
    (0..n).map(|ix| make(format!("{prefix}-{ix}"))).collect()
}

fn run(sessions: &[SessionInput]) -> Benchmarks {
    run_with(sessions, &BTreeMap::new(), 4, None)
}

fn run_with(
    sessions: &[SessionInput],
    labels: &BTreeMap<String, SessionLabel>,
    weeks: u32,
    tool: Option<BenchTool>,
) -> Benchmarks {
    benchmarks(
        sessions,
        &BTreeMap::new(),
        labels,
        &catalog(),
        weeks,
        tool,
        now(),
    )
}

fn row(answer: &Benchmarks, key: BenchCategory) -> &BenchCategoryRow {
    answer.categories.iter().find(|row| row.key == key).unwrap()
}

fn model<'a>(row: &'a BenchCategoryRow, label: &str) -> &'a BenchModel {
    row.models
        .iter()
        .find(|model| model.label == label)
        .unwrap()
}

// ---- category mapping ----

fn files(test: u32, doc: u32, source: u32, config: u32) -> ExtensionClassCounts {
    ExtensionClassCounts {
        test,
        doc,
        source,
        config,
    }
}

fn shaped(task: InferredTask, edited: ExtensionClassCounts) -> SessionFacts {
    let mut f = facts(task, vec![mf(Some(1), Some(0), Some(0))]);
    f.shape = shape(edited);
    f
}

#[test]
fn inferred_tasks_map_to_their_categories() {
    let cases = [
        (
            InferredTask::BugFix,
            files(0, 0, 1, 0),
            BenchCategory::BugFix,
        ),
        (
            InferredTask::Refactor,
            files(0, 0, 3, 0),
            BenchCategory::Refactor,
        ),
        (InferredTask::Tests, files(2, 0, 0, 0), BenchCategory::Tests),
        (InferredTask::Docs, files(0, 2, 0, 0), BenchCategory::Docs),
        (
            InferredTask::NoEdits,
            files(0, 0, 0, 0),
            BenchCategory::Understand,
        ),
        (
            InferredTask::Other,
            files(0, 0, 2, 1),
            BenchCategory::Feature,
        ),
        (
            InferredTask::Other,
            files(0, 0, 1, 1),
            BenchCategory::Feature,
        ),
        (InferredTask::Other, files(0, 0, 1, 2), BenchCategory::Setup),
        (InferredTask::Other, files(0, 0, 0, 1), BenchCategory::Setup),
    ];
    for (task, edited, want) in cases {
        assert_eq!(
            bench_category(None, &shaped(task, edited)),
            want,
            "{task:?} {edited:?}"
        );
    }
}

#[test]
fn the_persons_label_comes_first() {
    let inferred = shaped(InferredTask::Docs, files(0, 2, 0, 0));
    let cases = [
        (LabelledTask::BugFix, BenchCategory::BugFix),
        (LabelledTask::Refactor, BenchCategory::Refactor),
        (LabelledTask::Tests, BenchCategory::Tests),
        (LabelledTask::Docs, BenchCategory::Docs),
        (LabelledTask::Other, BenchCategory::Feature),
    ];
    for (label, want) in cases {
        assert_eq!(bench_category(Some(label), &inferred), want, "{label:?}");
    }
    // Other on a config-heavy session is setup, on a no-edit one understand.
    let config = shaped(InferredTask::Tests, files(0, 0, 0, 3));
    assert_eq!(
        bench_category(Some(LabelledTask::Other), &config),
        BenchCategory::Setup
    );
    let reading = shaped(InferredTask::NoEdits, files(0, 0, 0, 0));
    assert_eq!(
        bench_category(Some(LabelledTask::Other), &reading),
        BenchCategory::Understand
    );
    // A no-edit session labelled as a fix is a fix.
    assert_eq!(
        bench_category(Some(LabelledTask::BugFix), &reading),
        BenchCategory::BugFix
    );
}

#[test]
fn review_is_never_inferred() {
    for task in [
        InferredTask::BugFix,
        InferredTask::Refactor,
        InferredTask::Tests,
        InferredTask::Docs,
        InferredTask::Other,
        InferredTask::NoEdits,
    ] {
        for edited in [files(0, 0, 0, 0), files(1, 1, 1, 1), files(0, 0, 0, 5)] {
            assert_ne!(
                bench_category(None, &shaped(task, edited)),
                BenchCategory::Review
            );
        }
    }
}

// ---- envelope ----

#[test]
fn eight_categories_in_wire_order_and_every_session_accounted_for() {
    let mut sessions = many("a", 2, |id| fix(&id, "model-a", 10, 0, 100, 1));
    let mut automated = fix("auto", "model-a", 10, 0, 100, 1);
    automated.automated = true;
    sessions.push(automated);
    sessions.push(session("no-facts", "model-a", 1, None));
    // Two models, one turn each: no strict majority.
    let mut mixed = fix("mixed", "model-a", 10, 0, 100, 2);
    if let SessionBody::Claude {
        series,
        model_labels,
    } = &mut mixed.body
    {
        model_labels.push("model-b".to_owned());
        series.turns[1].model_label_ix = Some(1);
    }
    sessions.push(mixed);
    let mut capped = fix("capped", "model-a", 10, 0, 100, 1);
    capped.facts.as_mut().unwrap().capped = true;
    sessions.push(capped);
    // Older than four weeks: outside the window, not counted.
    let mut old = fix("old", "model-a", 10, 0, 100, 1);
    if let SessionBody::Claude { series, .. } = &mut old.body {
        series.turns[0].at = Some(now() - Duration::weeks(6));
    }
    sessions.push(old);

    let answer = run(&sessions);
    let keys: Vec<BenchCategory> = answer.categories.iter().map(|row| row.key).collect();
    assert_eq!(keys, BenchCategory::ALL);
    let wire = serde_json::to_value(&answer).unwrap();
    let keys: Vec<&str> = wire["categories"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| row["key"].as_str().unwrap())
        .collect();
    assert_eq!(
        keys,
        [
            "bug_fix",
            "feature",
            "refactor",
            "tests",
            "docs",
            "understand",
            "review",
            "setup"
        ]
    );
    assert_eq!(answer.weeks, 4);
    assert_eq!(answer.min_sessions, 5);
    assert_eq!(answer.sessions, 6);
    assert_eq!(row(&answer, BenchCategory::BugFix).sessions, 2);
    assert_eq!(
        answer.left_out,
        BenchLeftOut {
            automated: 1,
            no_facts: 1,
            mixed_models: 1,
            capped: 1,
            ..BenchLeftOut::default()
        }
    );
    let in_rows: u32 = answer.categories.iter().map(|row| row.sessions).sum();
    let left = answer.left_out;
    let left_sum = left.automated
        + left.no_facts
        + left.no_model
        + left.reviewer
        + left.unknown_task
        + left.mixed_models
        + left.routed
        + left.capped
        + left.delegated;
    assert_eq!(in_rows + left_sum, answer.sessions);

    // Thirteen weeks takes the old session in.
    let answer = run_with(&sessions, &BTreeMap::new(), 13, None);
    assert_eq!(answer.sessions, 7);
    assert_eq!(row(&answer, BenchCategory::BugFix).sessions, 3);
}

#[test]
fn a_tool_filter_leaves_the_other_tool_uncounted() {
    let sessions = many("a", 3, |id| fix(&id, "model-a", 10, 0, 100, 1));
    let claude = run_with(&sessions, &BTreeMap::new(), 4, Some(BenchTool::ClaudeCode));
    assert_eq!(claude.sessions, 3);
    let codex = run_with(&sessions, &BTreeMap::new(), 4, Some(BenchTool::Codex));
    assert_eq!(codex.sessions, 0);
    assert!(codex.categories.iter().all(|row| row.models.is_empty()));
    assert_eq!(codex.left_out, BenchLeftOut::default());
}

#[test]
fn a_label_moves_a_session_to_its_category() {
    let sessions = vec![fix("s", "model-a", 10, 0, 100, 1)];
    let labels = BTreeMap::from([(
        "s".to_owned(),
        SessionLabel {
            task: Some(LabelledTask::Docs),
            outcome: Some(SessionOutcome::Kept),
        },
    )]);
    let answer = run_with(&sessions, &labels, 4, None);
    assert_eq!(row(&answer, BenchCategory::BugFix).sessions, 0);
    assert_eq!(row(&answer, BenchCategory::Docs).sessions, 1);
}

#[test]
fn a_no_edit_session_whose_edits_could_not_be_seen_is_left_out() {
    let mut f = facts(InferredTask::NoEdits, vec![mf(None, None, None)]);
    f.shape = shape(ExtensionClassCounts::default());
    let answer = run(&[session("s", "model-a", 1, Some(f))]);
    assert_eq!(answer.left_out.unknown_task, 1);
    assert_eq!(row(&answer, BenchCategory::Understand).sessions, 0);
}

// ---- figures and qualification ----

#[test]
fn kept_is_over_sessions_with_known_edits_only() {
    // 10 edits, 1 undone, 1 denied: 800 permille; a session with unknown
    // edits moves nothing.
    let mut sessions = vec![session(
        "known",
        "model-a",
        1,
        Some(facts(
            InferredTask::BugFix,
            vec![mf(Some(10), Some(1), Some(1))],
        )),
    )];
    sessions.push(session(
        "unknown",
        "model-a",
        1,
        Some(facts(InferredTask::BugFix, vec![mf(None, None, None)])),
    ));
    // Denied unknown reads as none declined: 10 edits, 2 undone, 800 more.
    sessions.push(session(
        "no-denied",
        "model-a",
        1,
        Some(facts(
            InferredTask::BugFix,
            vec![mf(Some(10), None, Some(2))],
        )),
    ));
    let answer = run(&sessions);
    let a = model(row(&answer, BenchCategory::BugFix), "model-a");
    assert_eq!(a.sessions, 3);
    assert_eq!(a.kept_sessions, 2);
    assert_eq!(a.kept_permille, Some(800));

    // Only unknown edits: no figure, never zero.
    let answer = run(&sessions[1..2]);
    let a = model(row(&answer, BenchCategory::BugFix), "model-a");
    assert_eq!(a.kept_permille, None);
    assert_eq!(a.kept_sessions, 0);

    // Known, but no edit made: no figure.
    let zero = session(
        "zero",
        "model-a",
        1,
        Some(facts(
            InferredTask::BugFix,
            vec![mf(Some(0), Some(0), Some(0))],
        )),
    );
    let answer = run(&[zero]);
    assert_eq!(
        model(row(&answer, BenchCategory::BugFix), "model-a").kept_permille,
        None
    );
}

#[test]
fn medians_take_the_middle_and_round_an_even_count_half_up() {
    // Costs 1, 2, 3, 4 USD (turns); active 100, 200, 301, 400.
    let sessions: Vec<SessionInput> = [(1, 100), (2, 200), (3, 301), (4, 400)]
        .into_iter()
        .enumerate()
        .map(|(ix, (turns, active))| fix(&format!("s{ix}"), "model-a", 10, 0, active, turns))
        .collect();
    let answer = run(&sessions);
    let a = model(row(&answer, BenchCategory::BugFix), "model-a");
    assert_eq!(a.usd_micros_median, Some(2_500_000));
    assert_eq!(a.active_secs_median, Some(251));
    assert_eq!(a.priced_sessions, 4);
    assert_eq!(a.timed_sessions, 4);

    let answer = run(&sessions[..3]);
    let a = model(row(&answer, BenchCategory::BugFix), "model-a");
    assert_eq!(a.usd_micros_median, Some(2_000_000));
    assert_eq!(a.active_secs_median, Some(200));

    // An unpriced model has no cost figure, never zero.
    let answer = run(&[fix("u", "model-z", 10, 0, 100, 1)]);
    let z = model(row(&answer, BenchCategory::BugFix), "model-z");
    assert_eq!(z.usd_micros_median, None);
    assert_eq!(z.priced_sessions, 0);
    assert!(answer.pricing_tables.is_empty());
}

/// Five bug fixes each: model-a keeps 900 permille, costs 2 USD, takes
/// 600 s; model-b keeps 700, costs 3 USD (1 turn at 3 USD), takes 300 s.
fn pair(a: usize, b: usize) -> Vec<SessionInput> {
    let mut sessions = many("a", a, |id| fix(&id, "model-a", 10, 1, 600, 2));
    sessions.extend(many("b", b, |id| fix(&id, "model-b", 10, 3, 300, 1)));
    sessions
}

#[test]
fn picks_need_two_models_with_five_sessions_each() {
    let answer = run(&pair(5, 5));
    let fixes = row(&answer, BenchCategory::BugFix);
    assert_eq!(fixes.sessions, 10);
    let quality = fixes.quality.as_ref().unwrap();
    assert_eq!(
        (quality.leader.label.as_str(), quality.leader.figure),
        ("model-a", 900)
    );
    assert_eq!(
        (quality.next.label.as_str(), quality.next.figure),
        ("model-b", 700)
    );
    assert_eq!(quality.leader.tool, BenchTool::ClaudeCode);
    let value = fixes.value.as_ref().unwrap();
    assert_eq!(
        (value.leader.label.as_str(), value.leader.figure),
        ("model-a", 2_000_000)
    );
    assert_eq!(value.next.figure, 3_000_000);
    let speed = fixes.speed.as_ref().unwrap();
    assert_eq!(
        (speed.leader.label.as_str(), speed.leader.figure),
        ("model-b", 300)
    );
    assert_eq!(answer.pricing_tables.len(), 1);

    // Four sessions of one model: nothing is picked, the figures stand.
    let answer = run(&pair(5, 4));
    let fixes = row(&answer, BenchCategory::BugFix);
    assert!(fixes.quality.is_none() && fixes.value.is_none() && fixes.speed.is_none());
    assert_eq!(model(fixes, "model-b").kept_permille, Some(700));

    // One model alone: nothing to compare.
    let answer = run(&pair(9, 0));
    let fixes = row(&answer, BenchCategory::BugFix);
    assert!(fixes.quality.is_none() && fixes.value.is_none() && fixes.speed.is_none());
}

#[test]
fn models_are_listed_most_sessions_first() {
    let answer = run(&pair(5, 7));
    let labels: Vec<&str> = row(&answer, BenchCategory::BugFix)
        .models
        .iter()
        .map(|model| model.label.as_str())
        .collect();
    assert_eq!(labels, ["model-b", "model-a"]);
}

#[test]
fn a_model_qualifies_on_its_figures_own_sample() {
    // model-b has five sessions, but only four with a known kept count.
    let mut sessions = pair(5, 4);
    sessions.push(session(
        "b-unknown",
        "model-b",
        1,
        Some(facts(InferredTask::BugFix, vec![mf(None, None, None)])),
    ));
    let answer = run(&sessions);
    let fixes = row(&answer, BenchCategory::BugFix);
    assert_eq!(model(fixes, "model-b").sessions, 5);
    assert!(fixes.quality.is_none());
    assert!(fixes.value.is_some() && fixes.speed.is_some());
}

// ---- tips ----

#[test]
fn test_first_when_both_groups_have_three_and_it_is_faster() {
    let with_first = |id: String, active: u32| {
        let mut s = fix(&id, "model-a", 10, 0, active, 1);
        s.facts
            .as_mut()
            .unwrap()
            .shape
            .failed_before_first_edit_then_passed = 1;
        s
    };
    let mut sessions = many("w", 3, |id| with_first(id, 300));
    sessions.extend(many("o", 3, |id| fix(&id, "model-a", 10, 0, 1_000, 1)));
    let answer = run(&sessions);
    assert_eq!(
        row(&answer, BenchCategory::BugFix).tip,
        Some(Tip::TestFirst {
            faster_permille: 700,
            sessions_without: 3,
            sessions: 6,
            sessions_with: 3,
        })
    );

    // Two in a group: no tip.
    let answer = run(&sessions[1..]);
    assert_eq!(row(&answer, BenchCategory::BugFix).tip, None);

    // Not faster: no tip.
    let mut slower = many("w", 3, |id| with_first(id, 1_000));
    slower.extend(many("o", 3, |id| fix(&id, "model-a", 10, 0, 1_000, 1)));
    let answer = run(&slower);
    assert_eq!(row(&answer, BenchCategory::BugFix).tip, None);

    // Only on bug fixes.
    let mut refactors: Vec<SessionInput> = sessions.clone();
    for s in &mut refactors {
        s.facts.as_mut().unwrap().task = InferredTask::Refactor;
    }
    let answer = run(&refactors);
    assert_eq!(row(&answer, BenchCategory::Refactor).tip, None);
}

#[test]
fn cheaper_same_quality_when_near_the_leader_at_half_the_cost() {
    // model-c keeps 900 at 5 USD; model-a keeps 880 (within 30) at 2 USD.
    let mut sessions = many("c", 5, |id| fix(&id, "model-c", 10, 1, 600, 1));
    sessions.extend(many("a", 5, |id| {
        let mut f = facts(InferredTask::Refactor, vec![mf(Some(25), Some(0), Some(3))]);
        f.active_secs = Some(600);
        session(&id, "model-a", 2, Some(f))
    }));
    for s in &mut sessions {
        s.facts.as_mut().unwrap().task = InferredTask::Refactor;
    }
    let answer = run(&sessions);
    let refactors = row(&answer, BenchCategory::Refactor);
    assert_eq!(model(refactors, "model-a").kept_permille, Some(880));
    assert_eq!(
        refactors.tip,
        Some(Tip::CheaperSameQuality {
            cheap: "model-a".to_owned(),
            expensive: "model-c".to_owned(),
            cost_ratio_permille: 400,
        })
    );

    // At 3 USD of 5 it is not half the cost: no such tip.
    let mut dearer = sessions.clone();
    for s in dearer.iter_mut().filter(|s| s.session_ref.starts_with('a')) {
        if let SessionBody::Claude { series, .. } = &mut s.body {
            series.turns.push(turn(2, Some(0), MILLION, day_ago()));
        }
    }
    let answer = run(&dearer);
    assert_eq!(row(&answer, BenchCategory::Refactor).tip, None);

    // 40 permille below the leader is not the same quality.
    let mut worse = sessions.clone();
    for s in worse.iter_mut().filter(|s| s.session_ref.starts_with('a')) {
        s.facts.as_mut().unwrap().models[0].undone = Some(4);
    }
    let answer = run(&worse);
    let refactors = row(&answer, BenchCategory::Refactor);
    assert_eq!(model(refactors, "model-a").kept_permille, Some(840));
    assert!(!matches!(
        refactors.tip,
        Some(Tip::CheaperSameQuality { .. })
    ));
}

#[test]
fn fewer_undos_when_one_model_undoes_twice_as_often() {
    // Same cost per session so no cheaper tip; model-a undoes 1 in 10,
    // model-b 3 in 10.
    let mut sessions = many("a", 5, |id| fix(&id, "model-b", 10, 1, 600, 1));
    sessions.extend(many("b", 5, |id| {
        let mut s = fix(&id, "model-b", 10, 3, 600, 1);
        if let SessionBody::Claude { model_labels, .. } = &mut s.body {
            model_labels[0] = "model-c".to_owned();
        }
        s
    }));
    let answer = run(&sessions);
    assert_eq!(
        row(&answer, BenchCategory::BugFix).tip,
        Some(Tip::FewerUndos {
            better: "model-b".to_owned(),
            worse: "model-c".to_owned(),
            ratio_permille: 3_000,
        })
    );

    // 1 in 10 against 3 in 20: 1.5 times, no tip.
    let mut close = sessions.clone();
    for s in close.iter_mut().filter(|s| s.session_ref.starts_with('b')) {
        s.facts.as_mut().unwrap().models[0].edits = Some(20);
    }
    let answer = run(&close);
    assert_eq!(row(&answer, BenchCategory::BugFix).tip, None);

    // Under the edit floor (5 x 5 = 25 < 30): no tip.
    let mut few = sessions.clone();
    for s in few.iter_mut().filter(|s| s.session_ref.starts_with('a')) {
        let m = &mut s.facts.as_mut().unwrap().models[0];
        m.edits = Some(5);
        m.undone = Some(0);
    }
    let answer = run(&few);
    assert_eq!(row(&answer, BenchCategory::BugFix).tip, None);
}

fn reading(id: &str, rereads: bool) -> SessionInput {
    let mut f = facts(InferredTask::NoEdits, vec![mf(Some(0), Some(0), Some(0))]);
    f.shape = shape(ExtensionClassCounts::default());
    let mut s = session(id, "model-a", 2, Some(f));
    let read = |turn: u32, path: u8| ToolCallRecord {
        turn_ordinal: turn,
        tool: ToolKind::Read,
        args_key: KeyedDigest([path; 32]),
        path_key: Some(KeyedDigest([path; 32])),
        path_ext: Some("rs".to_owned()),
        result_bytes: Some(100),
        success: Some(true),
        paired: true,
    };
    if let SessionBody::Claude { series, .. } = &mut s.body {
        series.tool_calls = if rereads {
            vec![read(0, 1), read(1, 1), read(1, 2)]
        } else {
            vec![read(0, 1), read(1, 2)]
        };
    }
    s
}

#[test]
fn rereads_when_half_the_reading_sessions_read_a_file_again() {
    let mut sessions: Vec<SessionInput> =
        (0..3).map(|ix| reading(&format!("r{ix}"), true)).collect();
    sessions.extend((0..2).map(|ix| reading(&format!("n{ix}"), false)));
    let answer = run(&sessions);
    let understand = row(&answer, BenchCategory::Understand);
    assert_eq!(understand.sessions, 5);
    assert_eq!(
        understand.tip,
        Some(Tip::Rereads {
            sessions_with: 3,
            sessions: 5,
            files: 1,
        })
    );

    // Two of five: no tip.
    let mut fewer: Vec<SessionInput> = (0..2).map(|ix| reading(&format!("r{ix}"), true)).collect();
    fewer.extend((0..3).map(|ix| reading(&format!("n{ix}"), false)));
    assert_eq!(row(&run(&fewer), BenchCategory::Understand).tip, None);

    // Under five reading sessions: no tip.
    assert_eq!(
        row(&run(&sessions[..4]), BenchCategory::Understand).tip,
        None
    );
}

#[test]
fn the_wire_shape_matches_the_contract() {
    let wire = serde_json::to_value(run(&pair(5, 5))).unwrap();
    let fixes = &wire["categories"][0];
    assert_eq!(fixes["key"], "bug_fix");
    let model = &fixes["models"][0];
    for key in [
        "label",
        "tool",
        "sessions",
        "kept_permille",
        "usd_micros_median",
        "active_secs_median",
    ] {
        assert!(model.get(key).is_some(), "{key}");
    }
    assert_eq!(model["tool"], "claude_code");
    for pick in ["quality", "value", "speed"] {
        for side in ["leader", "next"] {
            let entry = &fixes[pick][side];
            assert!(entry["label"].is_string(), "{pick}.{side}");
            assert!(entry["tool"].is_string(), "{pick}.{side}");
            assert!(entry["figure"].is_u64(), "{pick}.{side}");
        }
    }
    assert!(fixes.get("tip").is_some());
    let tip = serde_json::to_value(Tip::Rereads {
        sessions_with: 1,
        sessions: 2,
        files: 3,
    })
    .unwrap();
    assert_eq!(
        tip,
        serde_json::json!({"kind": "rereads", "sessions_with": 1, "sessions": 2, "files": 3})
    );
    assert_eq!(wire["categories"][6]["quality"], serde_json::Value::Null);
}
