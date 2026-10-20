use chrono::{Duration, TimeZone};
use trace_commons_protocol::insights_pricing::{
    ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION, PricingAccounting, PricingCurrency,
    PricingTable, PricingTableProvenance, TokenRate,
};
use trace_commons_protocol::insights_usage_series::{CacheWrite, TurnRecord, UsageSeries};

use super::*;
use crate::insights::analytics_constants::{
    CLASSIFIER_VERSION, LEDGER_MONEY_IN_INSIGHTS, MODEL_ANSWER_WINDOW_WEEKS,
};
use crate::insights::codex_series::CodexTurnSeriesEvidence;
use crate::insights::session_facts::{
    ExtensionClassCounts, InferredTask, ModelFacts, SessionFacts, SessionShape,
};
use crate::insights::session_pricing::pricing_provider;
use crate::insights::week_rollup::{CodexObserved, SessionBody};

const MILLION: u32 = 1_000_000;
const USD_NANOS: u64 = 1_000_000_000;

fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 20, 12, 0, 0).unwrap()
}

fn day_ago() -> DateTime<Utc> {
    now() - Duration::days(1)
}

/// One synthetic Claude entry: every category at `usd_per_million`.
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
/// Any other label is unpriced.
fn catalog() -> PricingCatalog {
    let at = |second| Utc.timestamp_opt(second, 0).unwrap();
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
        published_at: at(101),
        provenance: PricingTableProvenance {
            publisher: "Synthetic fixture".to_owned(),
            source_url: "https://example.invalid/prices/1".to_owned(),
            retrieved_at: at(100),
            content_sha256:
                "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            reviewed_by: "Synthetic reviewer".to_owned(),
            reviewed_at: at(101),
            effective_from_basis: None,
        },
        entries,
    };
    table.table_id = table.canonical_table_id().unwrap();
    PricingCatalog::from_tables(vec![table]).unwrap()
}

fn turn(ordinal: u32, label: Option<u16>, tokens: u32) -> TurnRecord {
    TurnRecord {
        ordinal,
        at: Some(day_ago()),
        uncached_input: Some(tokens),
        cache_read: Some(0),
        cache_write: Some(CacheWrite::Split { m5: 0, h1: 0 }),
        output: Some(0),
        model_label_ix: label,
        gap_secs: None,
        msg_key: None,
    }
}

fn mf(
    label_ix: u16,
    edits: Option<u32>,
    denied: Option<u32>,
    undone: Option<u32>,
    interrupted: Option<u32>,
) -> ModelFacts {
    ModelFacts {
        label_ix,
        edits,
        denied,
        undone,
        interrupted,
    }
}

fn facts(task: InferredTask, models: Vec<ModelFacts>) -> SessionFacts {
    SessionFacts {
        models,
        shape: SessionShape {
            edited_files: ExtensionClassCounts::default(),
            renamed_files: 0,
            lines_added: Some(0),
            lines_removed: Some(0),
            failed_before_first_edit_then_passed: 0,
        },
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

fn input(session_ref: &str, body: SessionBody, facts: Option<SessionFacts>) -> SessionInput {
    SessionInput {
        session_ref: session_ref.to_owned(),
        import_seq: 0,
        placed_at: None,
        harness_session: None,
        body,
        facts,
    }
}

/// A Claude session; each turn is a million uncached input tokens.
fn claude(
    session_ref: &str,
    labels: &[&str],
    turn_labels: &[Option<u16>],
    facts: Option<SessionFacts>,
) -> SessionInput {
    let turns = turn_labels
        .iter()
        .enumerate()
        .map(|(ix, label)| turn(ix as u32, *label, MILLION))
        .collect();
    input(
        session_ref,
        SessionBody::Claude {
            series: UsageSeries {
                turns,
                ..UsageSeries::default()
            },
            model_labels: labels.iter().map(|label| (*label).to_owned()).collect(),
        },
        facts,
    )
}

/// A session served wholly by one model.
fn solo(
    session_ref: &str,
    label: &str,
    task: InferredTask,
    edits: u32,
    denied: Option<u32>,
    undone: u32,
) -> SessionInput {
    claude(
        session_ref,
        &[label],
        &[Some(0)],
        Some(facts(
            task,
            vec![mf(0, Some(edits), denied, Some(undone), Some(0))],
        )),
    )
}

fn many(prefix: &str, n: usize, make: impl Fn(String) -> SessionInput) -> Vec<SessionInput> {
    (0..n).map(|ix| make(format!("{prefix}-{ix}"))).collect()
}

fn with_facts(mut session: SessionInput, edit: impl FnOnce(&mut SessionFacts)) -> SessionInput {
    edit(session.facts.as_mut().unwrap());
    session
}

fn answer(sessions: &[SessionInput]) -> ModelAnswers {
    model_answers(sessions, &BTreeMap::new(), &catalog(), now())
}

fn row(answers: &ModelAnswers, question: ModelQuestion) -> &ModelAnswerRow {
    answers
        .rows
        .iter()
        .find(|row| row.question == question)
        .unwrap()
}

fn counts(row: &ModelAnswerRow) -> Vec<(&str, u32)> {
    row.sessions_by_model
        .iter()
        .map(|(label, count)| (label.as_str(), *count))
        .collect()
}

fn figure_of<'a>(row: &'a ModelAnswerRow, label: &str) -> &'a ModelFigure {
    [&row.leader, &row.next]
        .into_iter()
        .flatten()
        .find(|figure| figure.label == label)
        .unwrap()
}

fn rate(figure: &ModelFigure) -> u32 {
    match figure.figure {
        Figure::RatePermille { value } => value,
        other => panic!("not a rate: {other:?}"),
    }
}

/// Five BugFix sessions per model: model-a 10 edits, 1 denied, 1 undone each
/// (200 permille rejected); model-b 10 edits, 3 denied, 0 undone (300).
fn bugfix_pair() -> Vec<SessionInput> {
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::BugFix, 10, Some(1), 1)
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::BugFix, 10, Some(3), 0)
    }));
    sessions
}

#[test]
fn four_sessions_are_not_enough_and_five_are() {
    let mut sessions = bugfix_pair();
    sessions.pop();
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::NotEnoughSessions);
    assert_eq!(counts(q1), vec![("model-a", 5), ("model-b", 4)]);
    assert!(q1.leader.is_none() && q1.next.is_none());

    let answers = answer(&bugfix_pair());
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::Answered);
    assert_eq!(q1.basis, Some(RejectionBasis::Rejected));
    let leader = q1.leader.as_ref().unwrap();
    let next = q1.next.as_ref().unwrap();
    assert_eq!(
        (
            leader.label.as_str(),
            leader.sessions,
            leader.edits,
            rate(leader)
        ),
        ("model-a", 5, Some(50), 200)
    );
    assert_eq!(
        (next.label.as_str(), next.sessions, next.edits, rate(next)),
        ("model-b", 5, Some(50), 300)
    );
    assert_eq!(leader.interrupts_per_session_permille, Some(0));
}

#[test]
fn twenty_nine_edits_are_too_few_for_a_rejection_figure() {
    let mut sessions: Vec<SessionInput> = [6, 6, 6, 6, 5]
        .iter()
        .enumerate()
        .map(|(ix, edits)| {
            solo(
                &format!("a-{ix}"),
                "model-a",
                InferredTask::BugFix,
                *edits,
                Some(0),
                0,
            )
        })
        .collect();
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::BugFix, 6, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::NotEnoughSessions);
    assert_eq!(q1.left_out.too_few_edits, 5);
    assert_eq!(counts(q1), vec![("model-a", 5), ("model-b", 5)]);

    // One more edit and model-a qualifies.
    sessions[4] = solo("a-4", "model-a", InferredTask::BugFix, 6, Some(0), 0);
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::Answered);
    assert_eq!(q1.left_out.too_few_edits, 0);
}

#[test]
fn one_session_with_unknown_denied_puts_the_whole_question_on_undone() {
    let mut sessions = bugfix_pair();
    sessions[9] = solo("b-4", "model-b", InferredTask::BugFix, 10, None, 0);
    for session in sessions.iter_mut().skip(5) {
        *session = with_facts(session.clone(), |facts| facts.models[0].undone = Some(3));
    }
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.basis, Some(RejectionBasis::Undone));
    // Undone only, for every model: model-a 5/50, model-b 15/50.
    assert_eq!(rate(figure_of(q1, "model-a")), 100);
    assert_eq!(rate(figure_of(q1, "model-b")), 300);
    assert_eq!(q1.leader.as_ref().unwrap().label, "model-a");
}

#[test]
fn fifty_fifty_and_three_way_sessions_count_for_no_model() {
    let bugfix = |models| Some(facts(InferredTask::BugFix, models));
    let both = vec![
        mf(0, Some(5), Some(0), Some(0), Some(0)),
        mf(1, Some(5), Some(0), Some(0), Some(0)),
    ];
    let three = vec![
        mf(0, Some(5), Some(0), Some(0), Some(0)),
        mf(1, Some(5), Some(0), Some(0), Some(0)),
        mf(2, Some(5), Some(0), Some(0), Some(0)),
    ];
    let sessions = vec![
        claude(
            "half",
            &["model-a", "model-b"],
            &[Some(0), Some(1)],
            bugfix(both),
        ),
        claude(
            "third",
            &["model-a", "model-b", "model-c"],
            &[Some(0), Some(1), Some(2)],
            bugfix(three),
        ),
    ];
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.no_majority, 2);
    assert!(q1.sessions_by_model.is_empty());
}

#[test]
fn rates_use_only_the_majority_models_own_edits() {
    let mut sessions = many("a", 5, |id| {
        claude(
            &id,
            &["model-a", "model-b"],
            &[Some(0), Some(0), Some(1)],
            Some(facts(
                InferredTask::BugFix,
                vec![
                    mf(0, Some(10), Some(1), Some(1), Some(0)),
                    mf(1, Some(10), Some(5), Some(5), Some(0)),
                ],
            )),
        )
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::BugFix, 10, Some(4), 4)
    }));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    let a = figure_of(q1, "model-a");
    assert_eq!((rate(a), a.edits, a.sessions), (200, Some(50), 5));
    let b = figure_of(q1, "model-b");
    assert_eq!((rate(b), b.edits, b.sessions), (800, Some(50), 5));
}

#[test]
fn session_cost_sums_each_models_tokens_at_its_own_rate() {
    // model-a's sessions: two model-a turns at 1 USD and one model-b turn at
    // 3 USD per million, a million tokens each: 5 USD. model-b's: 3 USD.
    let docs = |models| Some(facts(InferredTask::Docs, models));
    let mut sessions = many("a", 5, |id| {
        claude(
            &id,
            &["model-a", "model-b"],
            &[Some(0), Some(0), Some(1)],
            docs(vec![
                mf(0, Some(1), Some(0), Some(0), Some(0)),
                mf(1, Some(1), Some(0), Some(0), Some(0)),
            ]),
        )
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::Docs, 1, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(q4.state, AnswerState::Answered);
    assert_eq!(q4.basis, None);
    let leader = q4.leader.as_ref().unwrap();
    let next = q4.next.as_ref().unwrap();
    assert_eq!(leader.label, "model-b");
    assert_eq!(leader.figure, Figure::UsdMicros { value: 3_000_000 });
    assert_eq!(leader.sessions, 5);
    assert_eq!(leader.edits, None);
    assert_eq!(next.label, "model-a");
    assert_eq!(next.figure, Figure::UsdMicros { value: 5_000_000 });
    assert_eq!(answers.pricing_tables.len(), 1);
    assert_eq!(answers.pricing_tables[0].version, 1);
}

#[test]
fn synthetic_and_dated_labels_normalize_into_one_model() {
    let mut sessions = many("a", 5, |id| {
        let mut session = claude(
            &id,
            &["Model-A-20261001", "model-a", "<synthetic>"],
            &[Some(0), Some(1), Some(2), None],
            Some(facts(
                InferredTask::BugFix,
                vec![
                    mf(0, Some(3), Some(0), Some(1), Some(0)),
                    mf(1, Some(3), Some(0), Some(1), Some(0)),
                ],
            )),
        );
        // The two unlabelled turns hold no tokens: the `<synthetic>` shape.
        if let SessionBody::Claude { series, .. } = &mut session.body {
            series.turns[2] = turn(2, Some(2), 0);
            series.turns[3] = turn(3, None, 0);
        }
        session
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::BugFix, 6, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.no_majority, 0);
    assert_eq!(counts(q1), vec![("model-a", 5), ("model-b", 5)]);
    let a = figure_of(q1, "model-a");
    assert_eq!(a.edits, Some(30));
    assert_eq!(rate(a), 333);
}

#[test]
fn delegated_rows_leave_rejection_and_cost_but_keep_time() {
    let delegated =
        |session: SessionInput| with_facts(session, |facts| facts.delegated_unread = true);
    let mut sessions = bugfix_pair();
    sessions.push(delegated(solo(
        "a-delegated",
        "model-a",
        InferredTask::BugFix,
        10,
        Some(9),
        9,
    )));
    sessions.push(delegated(solo(
        "a-docs",
        "model-a",
        InferredTask::Docs,
        1,
        Some(0),
        0,
    )));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.delegated, 1);
    assert_eq!(rate(figure_of(q1, "model-a")), 200);
    assert_eq!(row(&answers, ModelQuestion::DocsCost).left_out.delegated, 1);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.left_out.delegated, 0);
    assert_eq!(counts(q6), vec![("model-a", 7), ("model-b", 5)]);
}

#[test]
fn capped_rows_leave_every_figure() {
    let capped = |session: SessionInput| with_facts(session, |facts| facts.capped = true);
    let mut sessions = bugfix_pair();
    sessions.push(capped(solo(
        "a-capped",
        "model-a",
        InferredTask::BugFix,
        10,
        Some(9),
        9,
    )));
    sessions.push(capped(solo(
        "a-docs",
        "model-a",
        InferredTask::Docs,
        1,
        Some(0),
        0,
    )));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.capped, 1);
    assert_eq!(rate(figure_of(q1, "model-a")), 200);
    assert_eq!(row(&answers, ModelQuestion::DocsCost).left_out.capped, 1);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.left_out.capped, 2);
    assert_eq!(counts(q6), vec![("model-a", 5), ("model-b", 5)]);
}

const ALL_ROUTES: [ModelRoute; 7] = [
    ModelRoute::Unobserved,
    ModelRoute::Unrecorded,
    ModelRoute::Outside,
    ModelRoute::Mixed,
    ModelRoute::CheckFailed,
    ModelRoute::RoutedVerified,
    ModelRoute::RoutedUnverified,
];

#[test]
fn compared_and_left_out_route_categories() {
    let sessions: Vec<SessionInput> = ALL_ROUTES
        .iter()
        .enumerate()
        .map(|(ix, _)| {
            solo(
                &format!("r-{ix}"),
                "model-a",
                InferredTask::BugFix,
                10,
                Some(0),
                0,
            )
        })
        .collect();
    let routes: BTreeMap<String, ModelRoute> = ALL_ROUTES
        .iter()
        .enumerate()
        .map(|(ix, route)| (format!("r-{ix}"), *route))
        .collect();
    let answers = model_answers(&sessions, &routes, &catalog(), now());
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    // Unobserved, Outside and RoutedVerified are compared.
    assert_eq!(counts(q1), vec![("model-a", 3)]);
    assert_eq!(q1.left_out.routed, 4);
    // Absent from the map means Unobserved.
    let answers = model_answers(&sessions, &BTreeMap::new(), &catalog(), now());
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(counts(q1), vec![("model-a", 7)]);
    assert_eq!(q1.left_out.routed, 0);
}

#[test]
fn routed_sessions_never_enter_the_cost_question() {
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::Docs, 1, Some(0), 0)
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::Docs, 1, Some(0), 0)
    }));
    let mut routes = BTreeMap::new();
    for (ix, route) in ALL_ROUTES.iter().enumerate().skip(1) {
        let id = format!("routed-{ix}");
        sessions.push(solo(&id, "model-c", InferredTask::Docs, 1, Some(0), 0));
        routes.insert(id, *route);
    }
    let answers = model_answers(&sessions, &routes, &catalog(), now());
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(q4.left_out.routed, 6);
    assert_eq!(counts(q4), vec![("model-a", 5), ("model-b", 5)]);
    assert_eq!(q4.state, AnswerState::Answered);
}

#[test]
fn an_unpriced_model_reads_not_priced_never_not_enough_or_zero() {
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::Docs, 1, Some(0), 0)
    });
    sessions.extend(many("z", 5, |id| {
        solo(&id, "model-z", InferredTask::Docs, 1, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(q4.state, AnswerState::NotPriced);
    assert_eq!(q4.left_out.unpriced, 5);
    assert!(q4.leader.is_none() && q4.next.is_none());
    assert_eq!(counts(q4), vec![("model-a", 5), ("model-z", 5)]);

    // Below the session floor it is not enough sessions, priced or not.
    sessions.pop();
    let answers = answer(&sessions);
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(q4.state, AnswerState::NotEnoughSessions);
}

#[test]
fn unknown_facts_are_never_counted_as_zero() {
    let mut sessions = bugfix_pair();
    sessions.extend(many("c", 5, |id| {
        claude(&id, &["model-c"], &[Some(0)], None)
    }));
    // A session whose majority model's edits are unknown leaves the
    // rejection figures; it is not a session with no edits.
    sessions.push(claude(
        "a-unknown-edits",
        &["model-a"],
        &[Some(0)],
        Some(facts(
            InferredTask::BugFix,
            vec![mf(0, None, Some(0), Some(0), Some(0))],
        )),
    ));
    let answers = answer(&sessions);
    for row in &answers.rows {
        assert!(
            row.sessions_by_model
                .iter()
                .all(|(label, _)| label != "model-c")
        );
        assert!(row.left_out.unknown_facts >= 5, "{:?}", row.question);
    }
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.unknown_facts, 6);
    assert_eq!(counts(q1), vec![("model-a", 5), ("model-b", 5)]);
    assert_eq!(rate(figure_of(q1, "model-a")), 200);
}

#[test]
fn reviewer_and_no_edit_sessions_are_left_out() {
    let base = answer(&bugfix_pair());
    let mut sessions = bugfix_pair();
    sessions.push(with_facts(
        solo(
            "a-reviewer",
            "model-a",
            InferredTask::BugFix,
            10,
            Some(9),
            9,
        ),
        |facts| facts.reviewer = true,
    ));
    sessions.push(solo(
        "a-none",
        "model-a",
        InferredTask::NoEdits,
        0,
        Some(0),
        0,
    ));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.reviewer, 1);
    assert_eq!(rate(figure_of(q1, "model-a")), 200);
    for (with, without) in answers.rows.iter().zip(&base.rows) {
        let mut expected = without.clone();
        if with.question == ModelQuestion::BugFixRejected {
            expected.left_out.reviewer = 1;
        }
        if with.question == ModelQuestion::FinishTime
            || with.question == ModelQuestion::RejectedAcrossTasks
        {
            expected.left_out.reviewer = 1;
        }
        assert_eq!(with, &expected, "{:?}", with.question);
    }
}

#[test]
fn saved_snapshots_without_facts_are_left_out() {
    let observed = CodexObserved {
        first_at: day_ago(),
        last_at: day_ago(),
        input: 10,
        cached_input: 0,
        output: 1,
        total: 11,
        baseline_excluded: false,
    };
    let sessions = vec![
        input(
            "snapshot",
            SessionBody::Codex {
                observed: Some(observed),
                series: None,
            },
            None,
        ),
        claude("claude-snapshot", &["model-a"], &[Some(0)], None),
    ];
    let answers = answer(&sessions);
    for row in &answers.rows {
        assert_eq!(row.left_out.unknown_facts, 2, "{:?}", row.question);
        assert!(row.sessions_by_model.is_empty());
    }
}

#[test]
fn a_codex_session_counts_from_its_own_series() {
    let codex = |id: String| {
        input(
            &id,
            SessionBody::Codex {
                observed: None,
                series: Some(CodexTurnSeriesEvidence {
                    model_labels: vec!["model-a".to_owned()],
                    series: UsageSeries {
                        turns: vec![TurnRecord {
                            cache_write: Some(CacheWrite::TotalOnly(0)),
                            ..turn(0, Some(0), MILLION)
                        }],
                        ..UsageSeries::default()
                    },
                }),
            },
            Some(facts(
                InferredTask::Docs,
                vec![mf(0, Some(1), None, Some(0), Some(0))],
            )),
        )
    };
    let mut sessions = many("a", 5, codex);
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::Docs, 1, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(counts(q4), vec![("model-a", 5), ("model-b", 5)]);
    assert_eq!(q4.state, AnswerState::Answered);
    assert_eq!(
        figure_of(q4, "model-a").figure,
        Figure::UsdMicros { value: 1_000_000 }
    );
}

#[test]
fn the_answers_always_carry_six_rows_in_order() {
    let answers = answer(&[]);
    let questions: Vec<ModelQuestion> = answers.rows.iter().map(|row| row.question).collect();
    assert_eq!(questions, ModelQuestion::ALL.to_vec());
    assert_eq!(answers.window_end, now());
    assert_eq!(
        answers.window_start,
        now() - Duration::weeks(MODEL_ANSWER_WINDOW_WEEKS)
    );
    for row in &answers.rows {
        assert_eq!(row.state, AnswerState::NotEnoughSessions);
        assert_eq!(row.left_out, LeftOut::default());
    }
    let wire = serde_json::to_value(&answers).unwrap();
    assert_eq!(wire["rows"][0]["question"], "bug_fix_rejected");
    assert_eq!(wire["rows"][3]["question"], "docs_cost");
    assert_eq!(wire["rows"][5]["state"], "not_enough_sessions");
}

#[test]
fn ledger_money_is_settled() {
    const { assert!(LEDGER_MONEY_IN_INSIGHTS) };
}

#[test]
fn medians_and_rates_round_half_up_once() {
    assert_eq!(median(&mut []), None);
    assert_eq!(median(&mut [7, 1, 4]), Some(4));
    // Even count: the two middle values' mean, half-up.
    assert_eq!(median(&mut [4, 1, 2, 9]), Some(3));
    assert_eq!(median(&mut [1, 2]), Some(2));
    assert_eq!(permille(1, 3), Some(333));
    assert_eq!(permille(2, 3), Some(667));
    assert_eq!(permille(1, 2_000), Some(1));
    assert_eq!(permille(1, 0), None);
}
