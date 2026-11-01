use chrono::{Duration, TimeZone};
use trace_commons_protocol::insights_pricing::{
    ModelPriceEntry, PRICING_TABLE_SCHEMA_VERSION, PricingAccounting, PricingCurrency,
    PricingTable, PricingTableProvenance, TokenRate,
};
use trace_commons_protocol::insights_usage_series::{CacheWrite, TurnRecord, UsageSeries};

use super::*;
use crate::insights::analytics_constants::{
    CLASSIFIER_VERSION, LEDGER_MONEY_IN_INSIGHTS, MODEL_ANSWER_BOOTSTRAP_DRAWS,
    MODEL_ANSWER_BOOTSTRAP_SEED, MODEL_ANSWER_WINDOW_WEEKS,
};
use crate::insights::codex_series::CodexTurnSeriesEvidence;
use crate::insights::session_facts::{
    ExtensionClassCounts, InferredTask, ModelFacts, SessionFacts, SessionShape,
};
use crate::insights::session_labels::LabelledTask;
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
        automated: false,
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

    // One more edit and model-a qualifies. Both rates are zero, so the two
    // figures are shown with no clear difference.
    sessions[4] = solo("a-4", "model-a", InferredTask::BugFix, 6, Some(0), 0);
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::NoClearDifference);
    assert!(q1.leader.is_some() && q1.next.is_some());
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

/// An automated session (a Codex session its harness spawned itself) is
/// in no comparison, whatever its facts, and is counted as left out on
/// every row under its own reason, facts or not.
#[test]
fn automated_sessions_are_left_out_of_every_comparison() {
    let base = answer(&bugfix_pair());
    let mut sessions = bugfix_pair();
    let automated = |mut session: SessionInput| {
        session.automated = true;
        session
    };
    // Enough to qualify as a third model, and to move model-a's rate.
    sessions.extend(many("auto-c", 5, |id| {
        automated(solo(&id, "model-c", InferredTask::BugFix, 10, Some(9), 9))
    }));
    sessions.push(automated(solo(
        "auto-a",
        "model-a",
        InferredTask::BugFix,
        10,
        Some(9),
        9,
    )));
    sessions.push(automated(claude(
        "auto-none",
        &["model-a"],
        &[Some(0)],
        None,
    )));
    let answers = answer(&sessions);
    for (with, without) in answers.rows.iter().zip(&base.rows) {
        let mut expected = without.clone();
        expected.left_out.automated = 7;
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

// Across task types, noise and order (Task 8).

/// One session per entry of `undone`, each with `edits` edits and no
/// denials, served wholly by `label`.
fn spread(
    prefix: &str,
    label: &str,
    task: InferredTask,
    edits: u32,
    undone: &[u32],
) -> Vec<SessionInput> {
    undone
        .iter()
        .enumerate()
        .map(|(ix, undone)| {
            solo(
                &format!("{prefix}-{ix}"),
                label,
                task,
                edits,
                Some(0),
                *undone,
            )
        })
        .collect()
}

/// A finished BugFix session in `mode`, taking `secs` and `turns`.
fn timed(session_ref: &str, label: &str, mode: EditMode, secs: u32, turns: u32) -> SessionInput {
    with_facts(
        solo(session_ref, label, InferredTask::BugFix, 10, Some(0), 0),
        |facts| {
            facts.edit_mode = mode;
            facts.active_secs = Some(secs);
            facts.user_turns = Some(turns);
        },
    )
}

fn dated(mut session: SessionInput, at: DateTime<Utc>) -> SessionInput {
    if let SessionBody::Claude { series, .. } = &mut session.body {
        for turn in &mut series.turns {
            turn.at = Some(at);
        }
    }
    session
}

fn finish(figure: &ModelFigure) -> (u32, u32) {
    match figure.figure {
        Figure::FinishTime {
            median_active_secs,
            median_user_turns,
        } => (median_active_secs, median_user_turns),
        other => panic!("not a finish time: {other:?}"),
    }
}

fn ranked(row: &ModelAnswerRow) -> (String, String) {
    (
        row.leader.as_ref().unwrap().label.clone(),
        row.next.as_ref().unwrap().label.clone(),
    )
}

#[test]
fn a_docs_heavy_model_does_not_win_across_types_on_mix_alone() {
    // model-a: 40 Docs at 2%, 10 BugFix at 20%; model-b: 10 Docs at 3%, 40
    // BugFix at 15%, 100 edits a session. Pooled, model-a leads (280/5000
    // against 630/5000); weighted by the shared mix (50 Docs, 50 BugFix),
    // model-a is 110 permille and model-b 90.
    let mut sessions = spread("ad", "model-a", InferredTask::Docs, 100, &[2; 40]);
    sessions.extend(spread(
        "ab",
        "model-a",
        InferredTask::BugFix,
        100,
        &[20; 10],
    ));
    sessions.extend(spread("bd", "model-b", InferredTask::Docs, 100, &[3; 10]));
    sessions.extend(spread(
        "bb",
        "model-b",
        InferredTask::BugFix,
        100,
        &[15; 40],
    ));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.state, AnswerState::Answered);
    assert_eq!(q5.basis, Some(RejectionBasis::Rejected));
    assert_eq!(ranked(q5), ("model-b".to_owned(), "model-a".to_owned()));
    let b = figure_of(q5, "model-b");
    assert_eq!((rate(b), b.sessions, b.edits), (90, 50, Some(5_000)));
    assert_eq!(rate(figure_of(q5, "model-a")), 110);
    assert_eq!(q5.left_out.mix_too_different, 0);
    assert_eq!(counts(q5), vec![("model-a", 50), ("model-b", 50)]);
}

#[test]
fn a_model_covering_under_eighty_percent_of_the_mix_is_not_compared() {
    // model-a qualifies in BugFix only: with model-b or model-c, their
    // shared BugFix carries 10 of 18 sessions (55%). model-b and model-c
    // qualify in both. model-a's 3 Docs sessions are under the floor.
    let mut sessions = spread("ab", "model-a", InferredTask::BugFix, 10, &[0; 5]);
    sessions.extend(spread("ad", "model-a", InferredTask::Docs, 10, &[0; 3]));
    sessions.extend(spread("bb", "model-b", InferredTask::BugFix, 10, &[1; 5]));
    sessions.extend(spread("bd", "model-b", InferredTask::Docs, 10, &[1; 5]));
    sessions.extend(spread("cb", "model-c", InferredTask::BugFix, 10, &[3; 5]));
    sessions.extend(spread("cd", "model-c", InferredTask::Docs, 10, &[3; 5]));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.left_out.mix_too_different, 5);
    assert_eq!(q5.left_out.too_few_sessions, 3);
    assert_eq!(q5.state, AnswerState::Answered);
    assert_eq!(ranked(q5), ("model-b".to_owned(), "model-c".to_owned()));
    assert_eq!(
        counts(q5),
        vec![("model-a", 8), ("model-b", 10), ("model-c", 10)]
    );
    // In BugFix alone model-a is compared, and leads.
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.leader.as_ref().unwrap().label, "model-a");
    assert_eq!(q1.left_out.mix_too_different, 0);
}

#[test]
fn finish_time_uses_not_prompting_when_two_models_qualify_there() {
    let mut sessions = Vec::new();
    for ix in 0..5 {
        sessions.push(timed(
            &format!("an-{ix}"),
            "model-a",
            EditMode::NotPrompting,
            100,
            2,
        ));
        sessions.push(timed(
            &format!("bn-{ix}"),
            "model-b",
            EditMode::NotPrompting,
            300,
            4,
        ));
        sessions.push(timed(
            &format!("ap-{ix}"),
            "model-a",
            EditMode::Prompting,
            900,
            9,
        ));
        sessions.push(timed(
            &format!("bp-{ix}"),
            "model-b",
            EditMode::Prompting,
            50,
            1,
        ));
    }
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.edit_mode, Some(EditMode::NotPrompting));
    assert_eq!(q6.state, AnswerState::Answered);
    assert_eq!(ranked(q6), ("model-a".to_owned(), "model-b".to_owned()));
    assert_eq!(finish(figure_of(q6, "model-a")), (100, 2));
    assert_eq!(finish(figure_of(q6, "model-b")), (300, 4));
    assert_eq!(figure_of(q6, "model-a").sessions, 5);
    assert_eq!(counts(q6), vec![("model-a", 5), ("model-b", 5)]);
}

#[test]
fn finish_time_falls_back_to_prompting_and_never_mixes_modes() {
    let mut sessions = Vec::new();
    for ix in 0..5 {
        sessions.push(timed(
            &format!("an-{ix}"),
            "model-a",
            EditMode::NotPrompting,
            100,
            2,
        ));
        sessions.push(timed(
            &format!("ap-{ix}"),
            "model-a",
            EditMode::Prompting,
            400,
            5,
        ));
        sessions.push(timed(
            &format!("bp-{ix}"),
            "model-b",
            EditMode::Prompting,
            200,
            3,
        ));
    }
    for ix in 0..2 {
        sessions.push(timed(
            &format!("bn-{ix}"),
            "model-b",
            EditMode::NotPrompting,
            1,
            1,
        ));
    }
    for ix in 0..3 {
        sessions.push(timed(
            &format!("au-{ix}"),
            "model-a",
            EditMode::Unknown,
            1,
            1,
        ));
    }
    // Unfinished sessions stay out of the medians but count as sessions.
    sessions.push(with_facts(
        timed("ap-open", "model-a", EditMode::Prompting, 1, 1),
        |facts| facts.finished = Some(false),
    ));
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.edit_mode, Some(EditMode::Prompting));
    assert_eq!(ranked(q6), ("model-b".to_owned(), "model-a".to_owned()));
    assert_eq!(finish(figure_of(q6, "model-b")), (200, 3));
    assert_eq!(finish(figure_of(q6, "model-a")), (400, 5));
    assert_eq!(figure_of(q6, "model-a").sessions, 5);
    assert_eq!(counts(q6), vec![("model-a", 6), ("model-b", 5)]);
    assert_eq!(q6.left_out.unknown_facts, 3);
}

/// Six sessions a model, 10 edits each: model-a undoes 0 or 10 (500
/// permille), model-b 0, 10 or 1 (517). The gap is far inside the noise.
fn noisy_pair() -> Vec<SessionInput> {
    let mut sessions = spread(
        "a",
        "model-a",
        InferredTask::BugFix,
        10,
        &[0, 10, 0, 10, 0, 10],
    );
    sessions.extend(spread(
        "b",
        "model-b",
        InferredTask::BugFix,
        10,
        &[0, 10, 0, 10, 1, 10],
    ));
    sessions
}

#[test]
fn the_same_rows_give_the_same_answer_in_any_input_order() {
    let mut sessions = noisy_pair();
    sessions.extend(spread(
        "ad",
        "model-a",
        InferredTask::Docs,
        10,
        &[1, 3, 0, 2, 5, 1],
    ));
    sessions.extend(spread(
        "bd",
        "model-b",
        InferredTask::Docs,
        10,
        &[2, 2, 4, 0, 1, 3],
    ));
    for ix in 0..6 {
        let secs = 100 + 37 * ix;
        sessions.push(timed(
            &format!("at-{ix}"),
            "model-a",
            EditMode::NotPrompting,
            secs,
            ix,
        ));
        sessions.push(timed(
            &format!("bt-{ix}"),
            "model-b",
            EditMode::NotPrompting,
            secs + 20,
            ix,
        ));
    }
    let base = answer(&sessions);
    let len = sessions.len();
    let orders: Vec<Vec<usize>> = vec![
        (0..len).rev().collect(),
        (0..len).map(|ix| (ix + 7) % len).collect(),
        (0..len)
            .filter(|ix| ix % 2 == 1)
            .chain((0..len).filter(|ix| ix % 2 == 0))
            .collect(),
        (0..len).map(|ix| (ix * 5 + 3) % len).collect(),
        (0..len).map(|ix| (len - 1 - ix + 11) % len).collect(),
    ];
    for order in orders {
        let mut seen = order.clone();
        seen.sort_unstable();
        assert_eq!(seen, (0..len).collect::<Vec<_>>(), "not a permutation");
        let shuffled: Vec<SessionInput> = order.iter().map(|ix| sessions[*ix].clone()).collect();
        assert_eq!(answer(&shuffled), base);
    }
}

#[test]
fn a_gap_within_noise_reads_no_clear_difference_with_both_figures() {
    let answers = answer(&noisy_pair());
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::NoClearDifference);
    assert_eq!(rate(figure_of(q1, "model-a")), 500);
    assert_eq!(rate(figure_of(q1, "model-b")), 517);
    assert_eq!(ranked(q1), ("model-a".to_owned(), "model-b".to_owned()));
}

#[test]
fn a_clear_gap_names_the_leader() {
    let mut sessions = spread("a", "model-a", InferredTask::BugFix, 10, &[1, 2, 1, 2, 1]);
    sessions.extend(spread(
        "b",
        "model-b",
        InferredTask::BugFix,
        10,
        &[7, 8, 7, 8, 7],
    ));
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::Answered);
    assert_eq!(ranked(q1), ("model-a".to_owned(), "model-b".to_owned()));
    assert_eq!(rate(figure_of(q1, "model-a")), 140);
    assert_eq!(rate(figure_of(q1, "model-b")), 740);
}

#[test]
fn ties_break_by_edits_then_sessions_then_label_bytes() {
    // Identical figures, edits and sessions: label byte order.
    let mut sessions = spread("b", "model-b", InferredTask::BugFix, 10, &[2; 5]);
    sessions.extend(spread("a", "model-a", InferredTask::BugFix, 10, &[2; 5]));
    let q1 = answer(&sessions).rows[0].clone();
    assert_eq!(q1.state, AnswerState::NoClearDifference);
    assert_eq!(ranked(&q1), ("model-a".to_owned(), "model-b".to_owned()));

    // Same rate and edits, more sessions first: model-b's 6 sessions of 10
    // edits against model-a's 5 of 12, both 200 permille over 60 edits.
    let mut sessions = spread("b", "model-b", InferredTask::BugFix, 10, &[2; 6]);
    sessions.extend(spread(
        "a",
        "model-a",
        InferredTask::BugFix,
        12,
        &[3, 3, 2, 2, 2],
    ));
    let q1 = answer(&sessions).rows[0].clone();
    assert_eq!(ranked(&q1), ("model-b".to_owned(), "model-a".to_owned()));

    // Same rate, more edits first, before more sessions.
    let mut sessions = spread("b", "model-b", InferredTask::BugFix, 20, &[4; 5]);
    sessions.extend(spread("a", "model-a", InferredTask::BugFix, 10, &[2; 6]));
    let q1 = answer(&sessions).rows[0].clone();
    assert_eq!(ranked(&q1), ("model-b".to_owned(), "model-a".to_owned()));
}

#[test]
fn the_window_is_thirteen_weeks_ending_now_whatever_week_is_on_screen() {
    let twelve = now() - Duration::weeks(12);
    let fourteen = now() - Duration::weeks(14);
    let mut sessions: Vec<SessionInput> = bugfix_pair()
        .into_iter()
        .map(|session| dated(session, twelve))
        .collect();
    sessions.extend(
        spread("old", "model-b", InferredTask::BugFix, 10, &[9; 5])
            .into_iter()
            .map(|session| dated(session, fourteen)),
    );
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(counts(q1), vec![("model-a", 5), ("model-b", 5)]);
    assert_eq!(rate(figure_of(q1, "model-b")), 300);
    assert_eq!(answers.window_start, now() - Duration::weeks(13));
}

#[test]
fn the_bootstrap_runs_exactly_the_configured_draws() {
    let mut draws = BTreeMap::new();
    let answers = model_answers_counting(
        &bugfix_pair(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &BTreeMap::new(),
        &catalog(),
        now(),
        LEDGER_MONEY_IN_INSIGHTS,
        &mut draws,
    );
    assert_eq!(answers, answer(&bugfix_pair()));
    let per_question: Vec<(ModelQuestion, u32)> = ModelQuestion::ALL
        .iter()
        .map(|question| (*question, draws.get(question).copied().unwrap_or(0)))
        .collect();
    assert_eq!(
        per_question,
        vec![
            (ModelQuestion::BugFixRejected, MODEL_ANSWER_BOOTSTRAP_DRAWS),
            (ModelQuestion::RefactorRejected, 0),
            (ModelQuestion::TestsRejected, 0),
            (ModelQuestion::DocsCost, 0),
            (
                ModelQuestion::RejectedAcrossTasks,
                MODEL_ANSWER_BOOTSTRAP_DRAWS
            ),
            (ModelQuestion::FinishTime, MODEL_ANSWER_BOOTSTRAP_DRAWS),
        ]
    );
    assert_eq!(MODEL_ANSWER_BOOTSTRAP_DRAWS, 1_000);
}

#[test]
fn the_seed_is_pinned() {
    assert_eq!(MODEL_ANSWER_BOOTSTRAP_SEED, 0x7472_6163_655f_6d61);
    // SplitMix64's published sequence from seed zero.
    let mut state = 0u64;
    assert_eq!(split_mix64(&mut state), 0xe220_a839_7b1d_cdaf);
    assert_eq!(split_mix64(&mut state), 0x6e78_9e6a_a1b9_65f4);
    assert_eq!(split_mix64(&mut state), 0x06c4_5d18_8009_454f);
}

/// A finished session of `task` in `mode`, taking `secs` and two turns.
fn timed_in(
    session_ref: &str,
    label: &str,
    task: InferredTask,
    mode: EditMode,
    secs: u32,
) -> SessionInput {
    with_facts(timed(session_ref, label, mode, secs, 2), |facts| {
        facts.task = task;
    })
}

#[test]
fn finish_time_keeps_not_prompting_when_a_qualifier_there_fails_the_mix() {
    // Not prompting: model-a qualifies in BugFix (5) but not Docs (4);
    // model-b qualifies in both. Their shared BugFix carries 10 of their 19
    // sessions, so neither can be compared with the other: their 15
    // sessions in qualifying cells are mix too different, model-a's 4 Docs
    // too few. Two models qualify there, so the row stays on not_prompting
    // with none compared, even though prompting would compare two.
    let mut sessions = Vec::new();
    for ix in 0..5 {
        for (prefix, label, task, mode, secs) in [
            (
                "anb",
                "model-a",
                InferredTask::BugFix,
                EditMode::NotPrompting,
                100,
            ),
            (
                "bnb",
                "model-b",
                InferredTask::BugFix,
                EditMode::NotPrompting,
                300,
            ),
            (
                "bnd",
                "model-b",
                InferredTask::Docs,
                EditMode::NotPrompting,
                300,
            ),
            (
                "apb",
                "model-a",
                InferredTask::BugFix,
                EditMode::Prompting,
                100,
            ),
            (
                "bpb",
                "model-b",
                InferredTask::BugFix,
                EditMode::Prompting,
                900,
            ),
        ] {
            sessions.push(timed_in(&format!("{prefix}-{ix}"), label, task, mode, secs));
        }
    }
    for ix in 0..4 {
        sessions.push(timed_in(
            &format!("and-{ix}"),
            "model-a",
            InferredTask::Docs,
            EditMode::NotPrompting,
            100,
        ));
    }
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.edit_mode, Some(EditMode::NotPrompting));
    assert_eq!(q6.state, AnswerState::NotEnoughSessions);
    assert!(q6.leader.is_none());
    assert_eq!(q6.left_out.mix_too_different, 15);
    assert_eq!(q6.left_out.too_few_sessions, 4);
    assert_eq!(counts(q6), vec![("model-a", 9), ("model-b", 10)]);
}

#[test]
fn finish_time_uses_prompting_when_neither_mode_has_two_qualifiers() {
    // Not prompting has more sessions, but only model-a qualifies there.
    let mut sessions = Vec::new();
    for ix in 0..5 {
        sessions.push(timed(
            &format!("an-{ix}"),
            "model-a",
            EditMode::NotPrompting,
            100,
            2,
        ));
    }
    for ix in 0..2 {
        sessions.push(timed(
            &format!("bn-{ix}"),
            "model-b",
            EditMode::NotPrompting,
            100,
            2,
        ));
    }
    sessions.push(timed("ap-0", "model-a", EditMode::Prompting, 100, 2));
    sessions.push(timed("bp-0", "model-b", EditMode::Prompting, 100, 2));
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.edit_mode, Some(EditMode::Prompting));
    assert_eq!(q6.state, AnswerState::NotEnoughSessions);
    assert_eq!(counts(q6), vec![("model-a", 1), ("model-b", 1)]);
}

#[test]
fn a_model_with_no_qualifying_cell_does_not_weigh_in_the_mix() {
    // model-a and model-b qualify in BugFix only; model-c has 4 Docs
    // sessions and qualifies nowhere. Weighted across every model, BugFix
    // would carry 10 of 14 (71%) and remove both; weighted across the
    // models with a qualifying cell, it carries all of it.
    let mut sessions = spread("ab", "model-a", InferredTask::BugFix, 10, &[0; 5]);
    sessions.extend(spread("bb", "model-b", InferredTask::BugFix, 10, &[3; 5]));
    sessions.extend(spread("cd", "model-c", InferredTask::Docs, 10, &[0; 4]));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.left_out.mix_too_different, 0);
    assert_eq!(ranked(q5), ("model-a".to_owned(), "model-b".to_owned()));
    assert_eq!(
        counts(q5),
        vec![("model-a", 5), ("model-b", 5), ("model-c", 4)]
    );
}

#[test]
fn a_model_worse_in_every_shared_type_never_leads_across_types() {
    // model-a: 10 BugFix and 10 Docs at 10%, 5 Refactor at 30%. model-b: 10
    // BugFix and 10 Docs at 11%, 2 Refactor (under the session floor). Each
    // model over its own types, model-a reads 130 permille and model-b 110;
    // over the types both qualify in, model-a is 100 and model-b 110.
    let mut sessions = spread("ab", "model-a", InferredTask::BugFix, 100, &[10; 10]);
    sessions.extend(spread("ad", "model-a", InferredTask::Docs, 100, &[10; 10]));
    sessions.extend(spread(
        "ar",
        "model-a",
        InferredTask::Refactor,
        100,
        &[30; 5],
    ));
    sessions.extend(spread(
        "bb",
        "model-b",
        InferredTask::BugFix,
        100,
        &[11; 10],
    ));
    sessions.extend(spread("bd", "model-b", InferredTask::Docs, 100, &[11; 10]));
    sessions.extend(spread(
        "br",
        "model-b",
        InferredTask::Refactor,
        100,
        &[0; 2],
    ));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.state, AnswerState::Answered);
    assert_eq!(ranked(q5), ("model-a".to_owned(), "model-b".to_owned()));
    let a = figure_of(q5, "model-a");
    assert_eq!((rate(a), a.sessions, a.edits), (100, 20, Some(2_000)));
    let b = figure_of(q5, "model-b");
    assert_eq!((rate(b), b.sessions, b.edits), (110, 20, Some(2_000)));

    // The same shape in finish time: model-a 100 s in BugFix and Docs and
    // 1000 s in Refactor, model-b 110 s with 2 Refactor sessions.
    let mut sessions = Vec::new();
    for (prefix, label, task, n, secs) in [
        ("ab", "model-a", InferredTask::BugFix, 10, 100),
        ("ad", "model-a", InferredTask::Docs, 10, 100),
        ("ar", "model-a", InferredTask::Refactor, 5, 1_000),
        ("bb", "model-b", InferredTask::BugFix, 10, 110),
        ("bd", "model-b", InferredTask::Docs, 10, 110),
        ("br", "model-b", InferredTask::Refactor, 2, 1),
    ] {
        for ix in 0..n {
            sessions.push(timed_in(
                &format!("{prefix}-{ix}"),
                label,
                task,
                EditMode::NotPrompting,
                secs,
            ));
        }
    }
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.state, AnswerState::Answered);
    assert_eq!(ranked(q6), ("model-a".to_owned(), "model-b".to_owned()));
    assert_eq!(finish(figure_of(q6, "model-a")), (100, 2));
    assert_eq!(figure_of(q6, "model-a").sessions, 20);
    assert_eq!(finish(figure_of(q6, "model-b")), (110, 2));
}

/// Every session of the two models shown is in a figure or in a
/// `left_out` reason: a cell under the session floor is too few sessions,
/// and a qualifying cell outside the pair's shared types is mix too
/// different.
#[test]
fn left_out_explains_every_session_of_the_models_shown() {
    // model-a: 10 BugFix, 10 Docs, 5 Refactor; model-b: 10 BugFix, 10
    // Docs, 2 Refactor. Compared over BugFix and Docs.
    let mut sessions = spread("ab", "model-a", InferredTask::BugFix, 100, &[10; 10]);
    sessions.extend(spread("ad", "model-a", InferredTask::Docs, 100, &[10; 10]));
    sessions.extend(spread(
        "ar",
        "model-a",
        InferredTask::Refactor,
        100,
        &[30; 5],
    ));
    sessions.extend(spread(
        "bb",
        "model-b",
        InferredTask::BugFix,
        100,
        &[11; 10],
    ));
    sessions.extend(spread("bd", "model-b", InferredTask::Docs, 100, &[11; 10]));
    sessions.extend(spread(
        "br",
        "model-b",
        InferredTask::Refactor,
        100,
        &[0; 2],
    ));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.left_out.too_few_sessions, 2);
    assert_eq!(q5.left_out.mix_too_different, 5);
    let shown = q5.leader.as_ref().unwrap().sessions + q5.next.as_ref().unwrap().sessions;
    let in_question: u32 = counts(q5).iter().map(|(_, count)| count).sum();
    assert_eq!(
        shown + q5.left_out.too_few_sessions + q5.left_out.mix_too_different,
        in_question
    );
    // In Refactor alone model-b's two sessions are too few.
    let q2 = row(&answers, ModelQuestion::RefactorRejected);
    assert_eq!(q2.state, AnswerState::NotEnoughSessions);
    assert_eq!(q2.left_out.too_few_sessions, 2);

    // The same in one type, in finish time and in cost: a cell under the
    // floor.
    let mut sessions = bugfix_pair();
    sessions.pop();
    let answers = answer(&sessions);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.left_out.too_few_sessions, 4);
    let mut sessions = Vec::new();
    for (prefix, label, n) in [
        ("a", "model-a", 5),
        ("b", "model-b", 5),
        ("c", "model-c", 3),
    ] {
        for ix in 0..n {
            sessions.push(timed(
                &format!("{prefix}-{ix}"),
                label,
                EditMode::NotPrompting,
                100,
                2,
            ));
        }
    }
    let answers = answer(&sessions);
    let q6 = row(&answers, ModelQuestion::FinishTime);
    assert_eq!(q6.edit_mode, Some(EditMode::NotPrompting));
    assert_eq!(q6.left_out.too_few_sessions, 3);
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::Docs, 1, Some(0), 0)
    });
    sessions.extend(many("b", 3, |id| {
        solo(&id, "model-b", InferredTask::Docs, 1, Some(0), 0)
    }));
    let answers = answer(&sessions);
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(q4.state, AnswerState::NotEnoughSessions);
    assert_eq!(q4.left_out.too_few_sessions, 3);
}

#[test]
fn two_models_whose_shared_types_carry_under_eighty_percent_are_not_compared() {
    // model-a qualifies in BugFix and Docs, model-b in BugFix and Refactor.
    // Their shared BugFix carries 20 of their 40 sessions.
    let mut sessions = spread("ab", "model-a", InferredTask::BugFix, 10, &[1; 10]);
    sessions.extend(spread("ad", "model-a", InferredTask::Docs, 10, &[1; 10]));
    sessions.extend(spread("bb", "model-b", InferredTask::BugFix, 10, &[3; 10]));
    sessions.extend(spread(
        "br",
        "model-b",
        InferredTask::Refactor,
        10,
        &[3; 10],
    ));
    let answers = answer(&sessions);
    let q5 = row(&answers, ModelQuestion::RejectedAcrossTasks);
    assert_eq!(q5.state, AnswerState::NotEnoughSessions);
    assert!(q5.leader.is_none() && q5.next.is_none());
    assert_eq!(q5.left_out.mix_too_different, 40);
    // In BugFix alone they are compared.
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(ranked(q1), ("model-a".to_owned(), "model-b".to_owned()));
}

/// A session classified as no edits only because its edits could not be
/// seen (Codex shell-only work: `edits` unknown) has an unknown task, so it
/// is counted as left out on every row, never dropped silently as a known
/// no-edit session is.
#[test]
fn a_no_edit_session_whose_edits_are_unknown_is_counted_as_left_out() {
    let base = answer(&bugfix_pair());
    let mut sessions = bugfix_pair();
    sessions.push(claude(
        "a-shell-only",
        &["model-a"],
        &[Some(0)],
        Some(facts(
            InferredTask::NoEdits,
            vec![mf(0, None, None, Some(0), Some(0))],
        )),
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
    for (with, without) in answers.rows.iter().zip(&base.rows) {
        let mut expected = without.clone();
        expected.left_out.unknown_facts += 1;
        assert_eq!(with, &expected, "{:?}", with.question);
    }
}

/// Five list-priced Docs sessions on each of model-a and model-b, all
/// unobserved, plus model-a Docs sessions through NEAR AI, verified and
/// served by model-a, with the charges given (`None` is an unknown charge).
fn docs_with_charges(
    charges: &[Option<u64>],
) -> (
    Vec<SessionInput>,
    BTreeMap<String, ModelRoute>,
    BTreeMap<String, SessionCharge>,
) {
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::Docs, 1, Some(0), 0)
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::Docs, 1, Some(0), 0)
    }));
    let mut routes = BTreeMap::new();
    let mut charged = BTreeMap::new();
    for (ix, charge) in charges.iter().enumerate() {
        let id = format!("near-{ix}");
        sessions.push(solo(&id, "model-a", InferredTask::Docs, 1, Some(0), 0));
        routes.insert(id.clone(), ModelRoute::RoutedVerified);
        charged.insert(id, served(*charge, Some("model-a")));
    }
    (sessions, routes, charged)
}

fn served(usd_micros: Option<u64>, model: Option<&str>) -> SessionCharge {
    SessionCharge {
        usd_micros,
        served_model: model.map(str::to_owned),
    }
}

#[test]
fn the_near_ai_charge_is_its_own_figure_never_summed_with_list_prices() {
    // List prices would put each model-a session at 1 USD; the charges are
    // far from that, so any mixing would move a figure.
    let (sessions, routes, charges) = docs_with_charges(&[
        Some(2_000),
        Some(9_000),
        Some(4_000),
        Some(3_000),
        Some(5_000),
        None,
    ]);
    let with = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let without = model_answers(&sessions, &routes, &catalog(), now());
    let q4 = row(&with, ModelQuestion::DocsCost);
    let plain = row(&without, ModelQuestion::DocsCost);
    // The API-price figures are those of the unrouted sessions alone.
    assert_eq!(q4.state, AnswerState::Answered);
    for label in ["model-a", "model-b"] {
        assert_eq!(
            figure_of(q4, label).figure,
            figure_of(plain, label).figure,
            "{label}"
        );
        assert_eq!(figure_of(q4, label).sessions, 5);
    }
    assert_eq!(q4.left_out.routed, 6);
    // The charge is the median of the known charges, on its own.
    let expected = NearAiChargeFigure {
        basis: ChargeBasis::NearAiLedger,
        median: Some(Figure::UsdMicros { value: 4_000 }),
        routed_sessions: 6,
        charge_unknown: 1,
    };
    assert_eq!(q4.near_ai_charge, Some(expected));
    assert_eq!(figure_of(q4, "model-a").near_ai_charge, Some(expected));
    assert_eq!(figure_of(q4, "model-b").near_ai_charge, None);
    assert_eq!(plain.near_ai_charge, None, "no charges given, none shown");
    // No other question carries a charge.
    for other in with
        .rows
        .iter()
        .filter(|r| r.question != ModelQuestion::DocsCost)
    {
        assert_eq!(other.near_ai_charge, None, "{:?}", other.question);
    }
}

#[test]
fn the_near_ai_charge_payload_has_its_shape() {
    let (sessions, routes, charges) = docs_with_charges(&[
        Some(1_500),
        Some(2_500),
        Some(2_000),
        Some(1_000),
        Some(3_000),
    ]);
    let answers = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let wire = serde_json::to_value(&answers).unwrap();
    let q4 = &wire["rows"][3];
    assert_eq!(q4["question"], "docs_cost");
    let charge = serde_json::json!({
        "basis": "near_ai_ledger",
        "median": {"kind": "usd_micros", "value": 2_000},
        "routed_sessions": 5,
        "charge_unknown": 0,
    });
    assert_eq!(q4["near_ai_charge"], charge);
    let leader_or_next = if q4["leader"]["label"] == "model-a" {
        &q4["leader"]
    } else {
        &q4["next"]
    };
    assert_eq!(leader_or_next["near_ai_charge"], charge);
    // The list-price figure is untouched: a kind and an integer only.
    assert_eq!(leader_or_next["figure"]["kind"], "usd_micros");
    assert_eq!(leader_or_next["figure"].as_object().unwrap().len(), 2);
    // Every other row and figure says null, never zero.
    for (ix, row) in wire["rows"].as_array().unwrap().iter().enumerate() {
        if ix != 3 {
            assert!(row["near_ai_charge"].is_null(), "{row}");
        }
    }
}

#[test]
fn an_all_unknown_charge_reads_unknown_never_zero() {
    let (sessions, routes, charges) = docs_with_charges(&[None, None]);
    let answers = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let q4 = row(&answers, ModelQuestion::DocsCost);
    assert_eq!(
        q4.near_ai_charge,
        Some(NearAiChargeFigure {
            basis: ChargeBasis::NearAiLedger,
            median: None,
            routed_sessions: 2,
            charge_unknown: 2,
        })
    );
    let wire = serde_json::to_value(q4).unwrap();
    assert!(wire["near_ai_charge"]["median"].is_null(), "{wire}");
}

#[test]
fn a_charge_outside_the_docs_task_or_window_is_not_counted() {
    let (mut sessions, mut routes, mut charges) = docs_with_charges(&[Some(1_000); 5]);
    sessions.push(solo(
        "near-bug",
        "model-a",
        InferredTask::BugFix,
        1,
        Some(0),
        0,
    ));
    routes.insert("near-bug".into(), ModelRoute::RoutedVerified);
    charges.insert("near-bug".into(), served(Some(50_000), Some("model-a")));
    sessions.push(with_facts(
        solo("near-auto", "model-a", InferredTask::Docs, 1, Some(0), 0),
        |_| {},
    ));
    sessions.last_mut().unwrap().automated = true;
    routes.insert("near-auto".into(), ModelRoute::RoutedVerified);
    charges.insert("near-auto".into(), served(Some(70_000), Some("model-a")));
    let answers = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let charge = row(&answers, ModelQuestion::DocsCost)
        .near_ai_charge
        .unwrap();
    assert_eq!(charge.routed_sessions, 5);
    assert_eq!(charge.median, Some(Figure::UsdMicros { value: 1_000 }));
}

/// The review example: a Docs session that declares one model but
/// was served another through NEAR AI. Its charge paid for the model that
/// served it, so it is on the row's figure only, never on the declared
/// label's. So is a charge whose route is not verified, whose served model
/// is not one label, or not named: only a verified route whose one served
/// model is the label attributes a charge to it.
#[test]
fn a_charge_is_on_a_model_only_when_its_verified_served_model_is_that_label() {
    let (mut sessions, mut routes, mut charges) = docs_with_charges(&[Some(1_000); 5]);
    let mut add = |id: &str, route: ModelRoute, charge: SessionCharge| {
        sessions.push(solo(id, "model-a", InferredTask::Docs, 1, Some(0), 0));
        routes.insert(id.to_owned(), route);
        charges.insert(id.to_owned(), charge);
    };
    // Declares model-a, served another model, verified.
    add(
        "glm-served",
        ModelRoute::RoutedVerified,
        served(Some(90_000), Some("glm-5")),
    );
    // Served model-a, but not proven.
    for (id, route) in [
        ("unverified", ModelRoute::RoutedUnverified),
        ("check-failed", ModelRoute::CheckFailed),
        ("mixed", ModelRoute::Mixed),
    ] {
        add(id, route, served(Some(80_000), Some("model-a")));
    }
    // Verified, but its calls named no single served model.
    add(
        "no-single-model",
        ModelRoute::RoutedVerified,
        served(Some(70_000), None),
    );
    let answers = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let q4 = row(&answers, ModelQuestion::DocsCost);
    // The row counts every Docs session sent through NEAR AI.
    let on_row = q4.near_ai_charge.unwrap();
    assert_eq!(on_row.routed_sessions, 10);
    // model-a's figure counts only the five verified sessions served by it.
    let on_model = figure_of(q4, "model-a").near_ai_charge.unwrap();
    assert_eq!(on_model.routed_sessions, 5);
    assert_eq!(on_model.median, Some(Figure::UsdMicros { value: 1_000 }));
}

/// A charge figure is held to the session floor, as every other figure
/// is: under it the figure counts its sessions and shows no median, so a
/// single session's exact charge never crosses as a median.
#[test]
fn a_charge_under_the_session_floor_has_no_median() {
    let (sessions, routes, charges) = docs_with_charges(&[Some(1_234)]);
    let answers = model_answers_with_charges(&sessions, &routes, &charges, &catalog(), now());
    let q4 = row(&answers, ModelQuestion::DocsCost);
    let expected = NearAiChargeFigure {
        basis: ChargeBasis::NearAiLedger,
        median: None,
        routed_sessions: 1,
        charge_unknown: 0,
    };
    assert_eq!(q4.near_ai_charge, Some(expected));
    assert_eq!(figure_of(q4, "model-a").near_ai_charge, Some(expected));
}

/// `LEDGER_MONEY_IN_INSIGHTS` (owner decision D5) gates every money
/// figure: held, the cost row shows no list-priced figure, no charge and no
/// pricing table, and the other rows are unchanged.
#[test]
fn held_ledger_money_leaves_no_money_figure() {
    let (sessions, routes, charges) = docs_with_charges(&[Some(1_000); 5]);
    let shown = model_answers_counting(
        &sessions,
        &routes,
        &charges,
        &BTreeMap::new(),
        &catalog(),
        now(),
        true,
        &mut BTreeMap::new(),
    );
    let held = model_answers_counting(
        &sessions,
        &routes,
        &charges,
        &BTreeMap::new(),
        &catalog(),
        now(),
        false,
        &mut BTreeMap::new(),
    );
    let q4 = row(&held, ModelQuestion::DocsCost);
    assert!(row(&shown, ModelQuestion::DocsCost).leader.is_some());
    assert_eq!(q4.state, AnswerState::NotPriced);
    assert_eq!((&q4.leader, &q4.next), (&None, &None));
    assert_eq!(q4.near_ai_charge, None);
    assert!(held.pricing_tables.is_empty());
    let wire = serde_json::to_string(&held).unwrap();
    assert!(!wire.contains("usd_micros"), "{wire}");
    for (with, without) in held.rows.iter().zip(&shown.rows) {
        if with.question != ModelQuestion::DocsCost {
            assert_eq!(with, without, "{:?}", with.question);
        }
    }
}

fn labelled(
    sessions: &[SessionInput],
    labels: &[(&str, Option<LabelledTask>, Option<SessionOutcome>)],
) -> ModelAnswers {
    let labels: BTreeMap<String, SessionLabel> = labels
        .iter()
        .map(|(session_ref, task, outcome)| {
            (
                (*session_ref).to_owned(),
                SessionLabel {
                    task: *task,
                    outcome: *outcome,
                },
            )
        })
        .collect();
    model_answers_labelled(
        sessions,
        &BTreeMap::new(),
        &BTreeMap::new(),
        &labels,
        &catalog(),
        now(),
    )
}

/// The person's task replaces the inferred one: sessions inferred as
/// Other, Refactor and no edits that they name bug fixes are on the
/// bug-fix row, and off the rows of the task they were inferred as.
#[test]
fn a_labelled_task_overrides_the_inferred_one() {
    let mut sessions = many("a", 5, |id| {
        solo(&id, "model-a", InferredTask::Other, 10, Some(1), 1)
    });
    sessions.extend(many("b", 5, |id| {
        solo(&id, "model-b", InferredTask::Refactor, 10, Some(3), 0)
    }));
    sessions.push(solo(
        "a-none",
        "model-a",
        InferredTask::NoEdits,
        0,
        Some(0),
        0,
    ));
    let inferred = answer(&sessions);
    assert_eq!(
        counts(row(&inferred, ModelQuestion::BugFixRejected)),
        vec![]
    );
    assert_eq!(
        counts(row(&inferred, ModelQuestion::RefactorRejected)),
        vec![("model-b", 5)]
    );

    let mut ids: Vec<String> = (0..5)
        .flat_map(|ix| [format!("a-{ix}"), format!("b-{ix}")])
        .collect();
    ids.push("a-none".into());
    let as_bug_fixes: Vec<(&str, Option<LabelledTask>, Option<SessionOutcome>)> = ids
        .iter()
        .map(|id| (id.as_str(), Some(LabelledTask::BugFix), None))
        .collect();
    let answers = labelled(&sessions, &as_bug_fixes);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(counts(q1), vec![("model-a", 6), ("model-b", 5)]);
    assert_eq!(q1.state, AnswerState::Answered);
    // The no-edit session adds a session and no edits: 50 edits, 10
    // rejected.
    assert_eq!(rate(figure_of(q1, "model-a")), 200);
    assert_eq!(rate(figure_of(q1, "model-b")), 300);
    assert_eq!(
        counts(row(&answers, ModelQuestion::RefactorRejected)),
        vec![]
    );

    // Labels that set nothing read exactly as the inferred answers.
    let unset: Vec<(&str, Option<LabelledTask>, Option<SessionOutcome>)> =
        ids.iter().map(|id| (id.as_str(), None, None)).collect();
    assert_eq!(labelled(&sessions, &unset), inferred);
}

/// The person's outcomes cross as `kept`, per model, on the rejection rows
/// only, and move no rate, order or state.
#[test]
fn outcomes_count_as_kept_beside_the_rejection_rate() {
    let sessions = bugfix_pair();
    let base = answer(&sessions);
    let answers = labelled(
        &sessions,
        &[
            ("a-0", None, Some(SessionOutcome::Kept)),
            ("a-1", None, Some(SessionOutcome::Kept)),
            ("a-2", None, Some(SessionOutcome::PartlyKept)),
            ("b-0", None, Some(SessionOutcome::ThrownAway)),
            // A label with no outcome is in no kept figure.
            ("b-1", Some(LabelledTask::BugFix), None),
        ],
    );
    let a = KeptFigure {
        labelled: 3,
        kept: 2,
        partly_kept: 1,
        thrown_away: 0,
        kept_permille: 667,
    };
    let b = KeptFigure {
        labelled: 1,
        kept: 0,
        partly_kept: 0,
        thrown_away: 1,
        kept_permille: 0,
    };
    for (with, without) in answers.rows.iter().zip(&base.rows) {
        let mut expected = without.clone();
        if matches!(
            with.question,
            ModelQuestion::BugFixRejected | ModelQuestion::RejectedAcrossTasks
        ) {
            expected.kept_by_model = vec![("model-a".into(), a), ("model-b".into(), b)];
            expected.leader.as_mut().unwrap().kept = Some(a);
            expected.next.as_mut().unwrap().kept = Some(b);
        }
        assert_eq!(with, &expected, "{:?}", with.question);
    }

    // A row that cannot compare still says each model's outcomes.
    let mut few = bugfix_pair();
    few.pop();
    let answers = labelled(&few, &[("a-0", None, Some(SessionOutcome::Kept))]);
    let q1 = row(&answers, ModelQuestion::BugFixRejected);
    assert_eq!(q1.state, AnswerState::NotEnoughSessions);
    assert_eq!(
        q1.kept_by_model,
        vec![(
            "model-a".to_owned(),
            KeptFigure {
                labelled: 1,
                kept: 1,
                partly_kept: 0,
                thrown_away: 0,
                kept_permille: 1_000,
            }
        )]
    );
}

/// `kept` is additive on the wire: every figure and row carries it, `null`
/// or empty where there is none, and it holds counts and a permille only.
#[test]
fn the_kept_payload_has_its_shape() {
    let unlabelled = serde_json::to_value(answer(&bugfix_pair())).unwrap();
    for row in unlabelled["rows"].as_array().unwrap() {
        assert_eq!(row["kept_by_model"], serde_json::json!([]), "{row}");
        for side in ["leader", "next"] {
            if row[side].is_object() {
                assert!(row[side]["kept"].is_null(), "{row}");
            }
        }
    }
    let wire = serde_json::to_value(labelled(
        &bugfix_pair(),
        &[("a-0", None, Some(SessionOutcome::PartlyKept))],
    ))
    .unwrap();
    let kept = serde_json::json!({
        "labelled": 1, "kept": 0, "partly_kept": 1, "thrown_away": 0, "kept_permille": 0,
    });
    let q1 = &wire["rows"][0];
    assert_eq!(q1["question"], "bug_fix_rejected");
    assert_eq!(q1["kept_by_model"], serde_json::json!([["model-a", kept]]));
    assert_eq!(q1["leader"]["label"], "model-a");
    assert_eq!(q1["leader"]["kept"], kept);
    assert!(q1["next"]["kept"].is_null());
    // The cost and time rows carry none.
    assert_eq!(wire["rows"][3]["kept_by_model"], serde_json::json!([]));
    assert_eq!(wire["rows"][5]["kept_by_model"], serde_json::json!([]));
}
