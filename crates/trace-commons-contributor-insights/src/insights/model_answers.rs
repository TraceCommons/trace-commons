//! Which model did what in the contributor's own sessions: the six Models
//! answers (spec section 6).
//!
//! Local only: no I/O, no network, no persistence. The input is the
//! sessions' stored tokens and facts; prices come from the catalog passed in,
//! applied when the answer is built. The output carries fixed snake_case
//! labels, declared model labels, counts, rates in permille and money in
//! integer USD micros; never an id, digest, path, file name or backend.

//!
//! Rules this module settles where the spec leaves a choice:
//! - A session is in the window when its latest dated turn (or, with none,
//!   its `placed_at`) is in `[now - MODEL_ANSWER_WINDOW_WEEKS, now]`. A
//!   session with no date at all is in no window and is not counted.
//! - `left_out` counts sessions. `unknown_facts` sessions have no known task,
//!   so they are counted on every row; every other reason is counted on the
//!   rows whose task types the session's task belongs to. A session whose
//!   majority model's `edits` or `undone` is unknown is counted in
//!   `unknown_facts` on its rejection rows only.
//! - `too_few_edits` counts the sessions of cells that reach the session
//!   floor but not the edit floor.
//! - A median of an even count is the mean of the two middle values,
//!   rounded half-up; rates are exact fractions until the wire figure,
//!   rounded half-up to permille once.
//! - `basis` is set on a rejection row that compares two models, `None`
//!   elsewhere. A cost figure's `sessions` is its priced sessions, the
//!   median's sample.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use trace_commons_protocol::insights_pricing::PricingAccounting;

use super::analytics_constants::{
    MODEL_ANSWER_MIN_EDITS, MODEL_ANSWER_MIN_SESSIONS, MODEL_ANSWER_WINDOW_WEEKS,
};
use super::pricing_catalog::PricingCatalog;
use super::session_facts::{EditMode, InferredTask, ModelFacts, SessionFacts};
use super::session_pricing::{ModelPrice, PricingTableRef, normalize_model_label, price_session};
use super::task_classifier::reclassify;
use super::week_rollup::{SessionBody, SessionInput};

/// How a session reached its model, from the route tally: the engine's
/// public mirror of the daemon's route categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelRoute {
    Unobserved,
    Unrecorded,
    Outside,
    Mixed,
    CheckFailed,
    RoutedVerified,
    RoutedUnverified,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelQuestion {
    BugFixRejected,
    RefactorRejected,
    TestsRejected,
    DocsCost,
    RejectedAcrossTasks,
    FinishTime,
}

impl ModelQuestion {
    pub const ALL: [ModelQuestion; 6] = [
        ModelQuestion::BugFixRejected,
        ModelQuestion::RefactorRejected,
        ModelQuestion::TestsRejected,
        ModelQuestion::DocsCost,
        ModelQuestion::RejectedAcrossTasks,
        ModelQuestion::FinishTime,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnswerState {
    NotEnoughSessions,
    NotPriced,
    NoClearDifference,
    Answered,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionBasis {
    Rejected,
    Undone,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Figure {
    RatePermille {
        value: u32,
    },
    UsdMicros {
        value: u64,
    },
    FinishTime {
        median_active_secs: u32,
        median_user_turns: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelFigure {
    pub label: String,
    pub sessions: u32,
    pub edits: Option<u32>,
    pub figure: Figure,
    pub interrupts_per_session_permille: Option<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LeftOut {
    pub unknown_facts: u32,
    pub delegated: u32,
    pub capped: u32,
    pub routed: u32,
    pub reviewer: u32,
    pub no_majority: u32,
    pub unpriced: u32,
    pub too_few_edits: u32,
    pub mix_too_different: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelAnswerRow {
    pub question: ModelQuestion,
    pub state: AnswerState,
    pub basis: Option<RejectionBasis>,
    pub edit_mode: Option<EditMode>,
    pub leader: Option<ModelFigure>,
    pub next: Option<ModelFigure>,
    pub sessions_by_model: Vec<(String, u32)>,
    pub left_out: LeftOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ModelAnswers {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    /// Always six, in [`ModelQuestion::ALL`] order.
    pub rows: Vec<ModelAnswerRow>,
    pub pricing_tables: Vec<PricingTableRef>,
}

impl ModelRoute {
    /// Sessions in these categories are compared in the rejection and time
    /// questions; a label routed any other way may not name the model that
    /// served it.
    fn compared(self) -> bool {
        matches!(
            self,
            ModelRoute::Unobserved | ModelRoute::Outside | ModelRoute::RoutedVerified
        )
    }

    /// The cost question compares sessions with no route tally only: a
    /// tallied session's charge does not last the window, and list prices
    /// do not describe it.
    fn priced_at_list(self) -> bool {
        self == ModelRoute::Unobserved
    }
}

/// The majority model's own counts, summed over every declared label that
/// normalizes to it. `None` is unknown, never zero.
#[derive(Debug, Clone, Copy)]
struct OwnCounts {
    edits: Option<u32>,
    denied: Option<u32>,
    undone: Option<u32>,
    interrupted: Option<u32>,
}

/// A session in the window with facts, not the reviewer's, with edits.
struct Candidate {
    route: ModelRoute,
    facts: SessionFacts,
    /// The normalized label serving a strict majority of the counted turns.
    majority: Option<String>,
    own: OwnCounts,
    cost_usd_micros: Option<u64>,
    tables: Vec<PricingTableRef>,
}

struct Prepared {
    /// In `session_ref` order, so every cell visits its sessions in one
    /// order whatever the input order.
    candidates: Vec<Candidate>,
    unknown_facts: u32,
    reviewer: BTreeMap<InferredTask, u32>,
}

impl Prepared {
    fn left_out(&self, tasks: &[InferredTask]) -> LeftOut {
        LeftOut {
            unknown_facts: self.unknown_facts,
            reviewer: tasks
                .iter()
                .filter_map(|task| self.reviewer.get(task))
                .fold(0u32, |sum, count| sum.saturating_add(*count)),
            ..LeftOut::default()
        }
    }

    fn of_tasks<'p>(&'p self, tasks: &'p [InferredTask]) -> impl Iterator<Item = &'p Candidate> {
        self.candidates
            .iter()
            .filter(move |candidate| tasks.contains(&candidate.facts.task))
    }
}

/// Every task type the rejection and time questions cover across types.
const EDIT_TASKS: [InferredTask; 5] = [
    InferredTask::BugFix,
    InferredTask::Refactor,
    InferredTask::Tests,
    InferredTask::Docs,
    InferredTask::Other,
];

fn bump(count: &mut u32) {
    *count = count.saturating_add(1);
}

fn add(a: Option<u32>, b: Option<u32>) -> Option<u32> {
    Some(a?.saturating_add(b?))
}

/// The session's latest dated turn, or its `placed_at` when no turn is
/// dated.
fn latest_at(session: &SessionInput) -> Option<DateTime<Utc>> {
    let turns = match &session.body {
        SessionBody::Claude { series, .. } => Some(&series.turns),
        SessionBody::Codex {
            series: Some(evidence),
            ..
        } => Some(&evidence.series.turns),
        SessionBody::Codex { .. } | SessionBody::Unknown { .. } => None,
    };
    let from_turns = turns.and_then(|turns| turns.iter().filter_map(|turn| turn.at).max());
    let observed = match &session.body {
        SessionBody::Codex {
            observed: Some(observed),
            ..
        } => Some(observed.last_at),
        _ => None,
    };
    from_turns.max(observed).or(session.placed_at)
}

fn own_counts(models: &[ModelFacts], labels: &[String], majority: &str) -> OwnCounts {
    let mut own: Option<OwnCounts> = None;
    for model in models {
        let label = labels
            .get(usize::from(model.label_ix))
            .and_then(|label| normalize_model_label(label));
        if label.as_deref() != Some(majority) {
            continue;
        }
        own = Some(match own {
            None => OwnCounts {
                edits: model.edits,
                denied: model.denied,
                undone: model.undone,
                interrupted: model.interrupted,
            },
            Some(sum) => OwnCounts {
                edits: add(sum.edits, model.edits),
                denied: add(sum.denied, model.denied),
                undone: add(sum.undone, model.undone),
                interrupted: add(sum.interrupted, model.interrupted),
            },
        });
    }
    own.unwrap_or(OwnCounts {
        edits: None,
        denied: None,
        undone: None,
        interrupted: None,
    })
}

fn prepare(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    catalog: &PricingCatalog,
    window_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Prepared {
    let mut ordered: Vec<&SessionInput> = sessions.iter().collect();
    ordered.sort_by(|a, b| {
        a.session_ref
            .as_bytes()
            .cmp(b.session_ref.as_bytes())
            .then(a.import_seq.cmp(&b.import_seq))
    });
    let mut prepared = Prepared {
        candidates: Vec::new(),
        unknown_facts: 0,
        reviewer: BTreeMap::new(),
    };
    for session in ordered {
        let Some(at) = latest_at(session) else {
            continue;
        };
        if at < window_start || at > now {
            continue;
        }
        let Some(facts) = &session.facts else {
            bump(&mut prepared.unknown_facts);
            continue;
        };
        let (accounting, labels, turns) = match &session.body {
            SessionBody::Claude {
                series,
                model_labels,
            } => (PricingAccounting::ClaudeCode, model_labels, &series.turns),
            SessionBody::Codex {
                series: Some(evidence),
                ..
            } => (
                PricingAccounting::Codex,
                &evidence.model_labels,
                &evidence.series.turns,
            ),
            // No turns to attribute: which model served it is unknown.
            SessionBody::Codex { series: None, .. } | SessionBody::Unknown { .. } => {
                bump(&mut prepared.unknown_facts);
                continue;
            }
        };
        let mut facts = facts.clone();
        reclassify(&mut facts);
        if facts.reviewer {
            bump(prepared.reviewer.entry(facts.task).or_default());
            continue;
        }
        if facts.task == InferredTask::NoEdits {
            continue;
        }
        let price = price_session(catalog, accounting, labels, turns);
        let model_turns =
            |model: &ModelPrice| u64::from(model.priced_turns) + u64::from(model.unpriced_turns);
        let counted = price.by_model.values().map(model_turns).sum::<u64>()
            + u64::from(price.unlabelled_turns);
        let majority = price
            .by_model
            .iter()
            .find(|(_, model)| model_turns(model) * 2 > counted)
            .map(|(label, _)| label.clone());
        let own = match &majority {
            Some(label) => own_counts(&facts.models, labels, label),
            None => own_counts(&[], labels, ""),
        };
        prepared.candidates.push(Candidate {
            route: routes
                .get(&session.session_ref)
                .copied()
                .unwrap_or(ModelRoute::Unobserved),
            facts,
            majority,
            own,
            cost_usd_micros: price.total_usd_micros,
            tables: price.tables,
        });
    }
    prepared
}

/// One session's place in a rejection figure.
#[derive(Debug, Clone, Copy)]
struct RejectionSample {
    edits: u32,
    denied: Option<u32>,
    undone: u32,
    interrupted: Option<u32>,
}

type Cells<T> = BTreeMap<String, Vec<T>>;

fn rejection_cells(
    prepared: &Prepared,
    tasks: &[InferredTask],
) -> (Cells<RejectionSample>, LeftOut) {
    let mut left_out = prepared.left_out(tasks);
    let mut cells: Cells<RejectionSample> = BTreeMap::new();
    for candidate in prepared.of_tasks(tasks) {
        if !candidate.route.compared() {
            bump(&mut left_out.routed);
            continue;
        }
        let Some(model) = &candidate.majority else {
            bump(&mut left_out.no_majority);
            continue;
        };
        if candidate.facts.capped {
            bump(&mut left_out.capped);
            continue;
        }
        if candidate.facts.delegated_unread {
            bump(&mut left_out.delegated);
            continue;
        }
        let (Some(edits), Some(undone)) = (candidate.own.edits, candidate.own.undone) else {
            bump(&mut left_out.unknown_facts);
            continue;
        };
        cells
            .entry(model.clone())
            .or_default()
            .push(RejectionSample {
                edits,
                denied: candidate.own.denied,
                undone,
                interrupted: candidate.own.interrupted,
            });
    }
    (cells, left_out)
}

fn time_cells<'p>(
    prepared: &'p Prepared,
    tasks: &'p [InferredTask],
) -> (Cells<&'p Candidate>, LeftOut) {
    let mut left_out = prepared.left_out(tasks);
    let mut cells: Cells<&Candidate> = BTreeMap::new();
    for candidate in prepared.of_tasks(tasks) {
        if !candidate.route.compared() {
            bump(&mut left_out.routed);
            continue;
        }
        let Some(model) = &candidate.majority else {
            bump(&mut left_out.no_majority);
            continue;
        };
        // Delegated work is inside the parent's timeline, so it stays.
        if candidate.facts.capped {
            bump(&mut left_out.capped);
            continue;
        }
        cells.entry(model.clone()).or_default().push(candidate);
    }
    (cells, left_out)
}

fn cost_cells<'p>(
    prepared: &'p Prepared,
    tasks: &'p [InferredTask],
) -> (Cells<&'p Candidate>, LeftOut) {
    let mut left_out = prepared.left_out(tasks);
    let mut cells: Cells<&Candidate> = BTreeMap::new();
    for candidate in prepared.of_tasks(tasks) {
        if !candidate.route.priced_at_list() {
            bump(&mut left_out.routed);
            continue;
        }
        let Some(model) = &candidate.majority else {
            bump(&mut left_out.no_majority);
            continue;
        };
        if candidate.facts.capped {
            bump(&mut left_out.capped);
            continue;
        }
        if candidate.facts.delegated_unread {
            bump(&mut left_out.delegated);
            continue;
        }
        if candidate.cost_usd_micros.is_none() {
            bump(&mut left_out.unpriced);
        }
        cells.entry(model.clone()).or_default().push(candidate);
    }
    (cells, left_out)
}

fn session_counts<T>(cells: &Cells<T>) -> Vec<(String, u32)> {
    cells
        .iter()
        .map(|(label, samples)| (label.clone(), saturating_len(samples.len())))
        .collect()
}

fn saturating_len(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// `num / den` in permille, rounded half-up once. `None` when `den` is zero.
fn permille(num: u64, den: u64) -> Option<u32> {
    if den == 0 {
        return None;
    }
    let (num, den) = (u128::from(num), u128::from(den));
    u32::try_from((num * 2_000 + den) / (den * 2)).ok()
}

/// The median; of an even count, the two middle values' mean rounded
/// half-up. `None` when empty.
fn median(values: &mut [u64]) -> Option<u64> {
    values.sort_unstable();
    let len = values.len();
    let upper = *values.get(len / 2)?;
    if len % 2 == 1 {
        return Some(upper);
    }
    let lower = u128::from(values[len / 2 - 1]);
    u64::try_from((lower + u128::from(upper)).div_ceil(2)).ok()
}

/// One model's rejection figure as an exact fraction, with its sample.
struct RejectionFigure {
    label: String,
    num: u64,
    edits: u64,
    sessions: u32,
    interrupts_permille: Option<u32>,
}

impl RejectionFigure {
    /// `None` when the basis is `Rejected` and a sample's `denied` is
    /// unknown; the caller picks `Undone` then, so this never happens.
    fn of(label: &str, samples: &[RejectionSample], basis: RejectionBasis) -> Option<Self> {
        let mut num = 0u64;
        let mut edits = 0u64;
        let mut interrupts = Some(0u64);
        for sample in samples {
            let rejected = match basis {
                RejectionBasis::Rejected => u64::from(sample.denied?) + u64::from(sample.undone),
                RejectionBasis::Undone => u64::from(sample.undone),
            };
            num = num.saturating_add(rejected);
            edits = edits.saturating_add(u64::from(sample.edits));
            interrupts = interrupts
                .zip(sample.interrupted)
                .map(|(sum, count)| sum.saturating_add(u64::from(count)));
        }
        let sessions = saturating_len(samples.len());
        Some(Self {
            label: label.to_owned(),
            num,
            edits,
            sessions,
            interrupts_permille: interrupts.and_then(|sum| permille(sum, u64::from(sessions))),
        })
    }

    /// Lower rate first, then more edits, then more sessions, then label
    /// byte order: a total order, so iteration order never decides.
    fn order(&self, other: &Self) -> Ordering {
        let lhs = u128::from(self.num) * u128::from(other.edits);
        let rhs = u128::from(other.num) * u128::from(self.edits);
        lhs.cmp(&rhs)
            .then(other.edits.cmp(&self.edits))
            .then(other.sessions.cmp(&self.sessions))
            .then(self.label.as_bytes().cmp(other.label.as_bytes()))
    }

    fn wire(&self) -> Option<ModelFigure> {
        Some(ModelFigure {
            label: self.label.clone(),
            sessions: self.sessions,
            edits: Some(u32::try_from(self.edits).unwrap_or(u32::MAX)),
            figure: Figure::RatePermille {
                value: permille(self.num, self.edits)?,
            },
            interrupts_per_session_permille: self.interrupts_permille,
        })
    }
}

fn empty_row(
    question: ModelQuestion,
    sessions_by_model: Vec<(String, u32)>,
    left_out: LeftOut,
) -> ModelAnswerRow {
    ModelAnswerRow {
        question,
        state: AnswerState::NotEnoughSessions,
        basis: None,
        edit_mode: None,
        leader: None,
        next: None,
        sessions_by_model,
        left_out,
    }
}

fn rejection_row(
    prepared: &Prepared,
    question: ModelQuestion,
    task: InferredTask,
) -> ModelAnswerRow {
    let (cells, mut left_out) = rejection_cells(prepared, &[task]);
    let mut qualifying = Vec::new();
    for (label, samples) in &cells {
        if saturating_len(samples.len()) < MODEL_ANSWER_MIN_SESSIONS {
            continue;
        }
        let edits = samples.iter().fold(0u64, |sum, sample| {
            sum.saturating_add(u64::from(sample.edits))
        });
        if edits < u64::from(MODEL_ANSWER_MIN_EDITS) {
            left_out.too_few_edits = left_out
                .too_few_edits
                .saturating_add(saturating_len(samples.len()));
            continue;
        }
        qualifying.push((label.as_str(), samples.as_slice()));
    }
    let mut row = empty_row(question, session_counts(&cells), left_out);
    if qualifying.len() < 2 {
        return row;
    }
    let basis = if qualifying
        .iter()
        .all(|(_, samples)| samples.iter().all(|sample| sample.denied.is_some()))
    {
        RejectionBasis::Rejected
    } else {
        RejectionBasis::Undone
    };
    let mut figures: Vec<RejectionFigure> = qualifying
        .iter()
        .filter_map(|(label, samples)| RejectionFigure::of(label, samples, basis))
        .collect();
    figures.sort_by(|a, b| a.order(b));
    row.state = AnswerState::Answered;
    row.basis = Some(basis);
    row.leader = figures.first().and_then(RejectionFigure::wire);
    row.next = figures.get(1).and_then(RejectionFigure::wire);
    row
}

/// One model's median priced cost per session, with its sample.
struct CostFigure {
    label: String,
    median: u64,
    priced: u32,
    sessions: u32,
}

impl CostFigure {
    /// Lower cost first, then more priced sessions, then more sessions, then
    /// label byte order.
    fn order(&self, other: &Self) -> Ordering {
        self.median
            .cmp(&other.median)
            .then(other.priced.cmp(&self.priced))
            .then(other.sessions.cmp(&self.sessions))
            .then(self.label.as_bytes().cmp(other.label.as_bytes()))
    }

    fn wire(&self) -> ModelFigure {
        ModelFigure {
            label: self.label.clone(),
            sessions: self.priced,
            edits: None,
            figure: Figure::UsdMicros { value: self.median },
            interrupts_per_session_permille: None,
        }
    }
}

fn cost_row(prepared: &Prepared, tables: &mut BTreeSet<PricingTableRef>) -> ModelAnswerRow {
    let (cells, left_out) = cost_cells(prepared, &[InferredTask::Docs]);
    let mut row = empty_row(ModelQuestion::DocsCost, session_counts(&cells), left_out);
    let enough = cells
        .values()
        .filter(|samples| saturating_len(samples.len()) >= MODEL_ANSWER_MIN_SESSIONS)
        .count();
    let mut figures = Vec::new();
    for (label, samples) in &cells {
        let mut priced: Vec<u64> = samples
            .iter()
            .filter_map(|candidate| candidate.cost_usd_micros)
            .collect();
        for candidate in samples
            .iter()
            .filter(|candidate| candidate.cost_usd_micros.is_some())
        {
            tables.extend(candidate.tables.iter().cloned());
        }
        let priced_count = saturating_len(priced.len());
        if priced_count < MODEL_ANSWER_MIN_SESSIONS {
            continue;
        }
        if let Some(median) = median(&mut priced) {
            figures.push(CostFigure {
                label: label.clone(),
                median,
                priced: priced_count,
                sessions: saturating_len(samples.len()),
            });
        }
    }
    if enough < 2 {
        return row;
    }
    if figures.len() < 2 {
        row.state = AnswerState::NotPriced;
        return row;
    }
    figures.sort_by(|a, b| a.order(b));
    row.state = AnswerState::Answered;
    row.leader = figures.first().map(CostFigure::wire);
    row.next = figures.get(1).map(CostFigure::wire);
    row
}

pub fn model_answers(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
) -> ModelAnswers {
    let window_start = now - Duration::weeks(MODEL_ANSWER_WINDOW_WEEKS);
    let prepared = prepare(sessions, routes, catalog, window_start, now);
    let mut tables = BTreeSet::new();
    let rows = ModelQuestion::ALL
        .iter()
        .map(|question| match question {
            ModelQuestion::BugFixRejected => {
                rejection_row(&prepared, *question, InferredTask::BugFix)
            }
            ModelQuestion::RefactorRejected => {
                rejection_row(&prepared, *question, InferredTask::Refactor)
            }
            ModelQuestion::TestsRejected => {
                rejection_row(&prepared, *question, InferredTask::Tests)
            }
            ModelQuestion::DocsCost => cost_row(&prepared, &mut tables),
            ModelQuestion::RejectedAcrossTasks => {
                let (cells, left_out) = rejection_cells(&prepared, &EDIT_TASKS);
                empty_row(*question, session_counts(&cells), left_out)
            }
            ModelQuestion::FinishTime => {
                let (cells, left_out) = time_cells(&prepared, &EDIT_TASKS);
                empty_row(*question, session_counts(&cells), left_out)
            }
        })
        .collect();
    ModelAnswers {
        window_start,
        window_end: now,
        rows,
        pricing_tables: tables.into_iter().collect(),
    }
}

#[cfg(test)]
#[path = "model_answers_tests.rs"]
mod tests;
