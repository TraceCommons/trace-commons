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
//!   so they are counted on every row; a Codex session with no per-turn
//!   series (read before the reader, or trimmed by the counter store's
//!   Codex budget) is one of them, since which model served it is
//!   unknown; a session read as no edits while a model's `edits` is
//!   unknown (Codex shell-only work) is one of them,
//!   while a known no-edit session is in no question and no count. Every
//!   other reason is counted on the
//!   rows whose task types the session's task belongs to. A session whose
//!   majority model's `edits` or `undone` is unknown is counted in
//!   `unknown_facts` on its rejection rows only.
//! - `too_few_edits` counts the sessions of cells that reach the session
//!   floor but not the edit floor.
//! - A median of an even count is the mean of the two middle values,
//!   rounded half-up. A cell's figure is held in fixed point at
//!   `1 / FIXED_SCALE` (10^-18) of a unit, rounded half-up once there; a
//!   one-type figure is then exact to the wire, and rounded half-up to
//!   permille, micros or seconds once.
//! - `basis` is set on a rejection row that compares two models, `None`
//!   elsewhere. A cost figure's `sessions` is its priced sessions, the
//!   median's sample; a time figure's is its finished sessions.
//! - Across task types (Q5, Q6), a type's weight is the sessions of that
//!   type in the question's cells, across every model with at least one
//!   qualifying cell in the question (the spec's "compared models": the
//!   mix rule only removes models from that set, so the weights do not
//!   depend on their own outcome). A model with no qualifying cell anywhere
//!   is never compared and does not weigh in. `mix_too_different` counts every session in the question of a model
//!   with a qualifying cell whose qualifying types carry less than
//!   `MODEL_ANSWER_MIN_MIX_COVERAGE` of the weight. Q1-Q4 have one type,
//!   so every model with a qualifying cell covers it.
//! - Q6: a cell qualifies on `MODEL_ANSWER_MIN_SESSIONS` finished sessions
//!   with known time and turns; unfinished sessions count as sessions and
//!   in the weights, not in the medians. Its figure is the weighted mean of
//!   the per-type median active seconds; the turns beside it are weighted
//!   the same way and do not order. A session of `Unknown` edit mode, or a
//!   finished one with unknown time or turns, is counted in
//!   `unknown_facts`; a session of the other known mode is in the other
//!   comparison and is not counted as left out.
//! - Q6's `edit_mode` is chosen before the mix rule: `NotPrompting` when at
//!   least two models have a qualifying cell there, `Prompting` in every
//!   other case (including when neither mode has two). The mix rule and the
//!   bootstrap then run within that mode only, so a qualifier that fails
//!   the mix leaves the row on `NotPrompting` reading not enough sessions.
//! - The bootstrap compares the leader and the next model only. Per draw it
//!   resamples the leader's qualifying cells, then the next model's, each
//!   in task order and each cell's sessions in `session_ref` order, with
//!   one SplitMix64 stream per question from the fixed seed; the type
//!   weights stay those of the whole sample. A draw whose figure is
//!   undefined (a resample with no edits) is not a lead. A row with no
//!   clear difference still carries both figures, in the total order.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use trace_commons_protocol::insights_pricing::PricingAccounting;

use super::analytics_constants::{
    MODEL_ANSWER_BOOTSTRAP_DRAWS, MODEL_ANSWER_BOOTSTRAP_SEED, MODEL_ANSWER_LEAD_SHARE,
    MODEL_ANSWER_MIN_EDITS, MODEL_ANSWER_MIN_MIX_COVERAGE, MODEL_ANSWER_MIN_SESSIONS,
    MODEL_ANSWER_WINDOW_WEEKS,
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
            // No edit seen is not no edit made when a model's edits could
            // not be seen (Codex shell-only work): its task is unknown.
            if facts.models.iter().any(|model| model.edits.is_none()) {
                bump(&mut prepared.unknown_facts);
            }
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

/// A finished session's place in the time figure.
#[derive(Debug, Clone, Copy)]
struct TimePoint {
    active_secs: u64,
    user_turns: u64,
}

/// One session's place in the time question: `None` when it did not finish.
type TimeSample = Option<TimePoint>;

/// Per model, per task type, the sessions in `session_ref` order.
type Cells<T> = BTreeMap<String, BTreeMap<InferredTask, Vec<T>>>;

fn push<T>(cells: &mut Cells<T>, model: &str, task: InferredTask, sample: T) {
    cells
        .entry(model.to_owned())
        .or_default()
        .entry(task)
        .or_default()
        .push(sample);
}

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
        push(
            &mut cells,
            model,
            candidate.facts.task,
            RejectionSample {
                edits,
                denied: candidate.own.denied,
                undone,
                interrupted: candidate.own.interrupted,
            },
        );
    }
    (cells, left_out)
}

/// The time question's sessions of one `edit_mode`. A session of the other
/// known mode is in the other comparison, not left out; a session of
/// `Unknown` mode, or a finished one whose time or turns are unknown, is
/// counted in `unknown_facts`.
fn time_cells(
    prepared: &Prepared,
    tasks: &[InferredTask],
    mode: EditMode,
) -> (Cells<TimeSample>, LeftOut) {
    let mut left_out = prepared.left_out(tasks);
    let mut cells: Cells<TimeSample> = BTreeMap::new();
    for candidate in prepared.of_tasks(tasks) {
        if candidate.facts.edit_mode == EditMode::Unknown {
            bump(&mut left_out.unknown_facts);
            continue;
        }
        if candidate.facts.edit_mode != mode {
            continue;
        }
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
        let sample = match candidate.facts.finished {
            None => {
                bump(&mut left_out.unknown_facts);
                continue;
            }
            Some(false) => None,
            Some(true) => match (candidate.facts.active_secs, candidate.facts.user_turns) {
                (Some(active_secs), Some(user_turns)) => Some(TimePoint {
                    active_secs: u64::from(active_secs),
                    user_turns: u64::from(user_turns),
                }),
                _ => {
                    bump(&mut left_out.unknown_facts);
                    continue;
                }
            },
        };
        push(&mut cells, model, candidate.facts.task, sample);
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
        push(&mut cells, model, candidate.facts.task, candidate);
    }
    (cells, left_out)
}

fn sessions_of<T>(by_task: &BTreeMap<InferredTask, Vec<T>>) -> u32 {
    saturating_len(by_task.values().map(Vec::len).sum())
}

fn session_counts<T>(cells: &Cells<T>) -> Vec<(String, u32)> {
    cells
        .iter()
        .map(|(label, by_task)| (label.clone(), sessions_of(by_task)))
        .collect()
}

/// Each task type's weight: the contributor's sessions of that type in the
/// question, across every model.
fn mix_weights<T>(cells: &Cells<T>) -> BTreeMap<InferredTask, u64> {
    weights_of(cells.values())
}

/// Each task type's weight across the entrants: the models with at least
/// one qualifying cell in the question.
fn entrant_weights<T, U>(cells: &Cells<T>, entrants: &[Entrant<U>]) -> BTreeMap<InferredTask, u64> {
    weights_of(
        entrants
            .iter()
            .filter_map(|entrant| cells.get(&entrant.label)),
    )
}

fn weights_of<'c, T: 'c>(
    cells: impl Iterator<Item = &'c BTreeMap<InferredTask, Vec<T>>>,
) -> BTreeMap<InferredTask, u64> {
    let mut weights: BTreeMap<InferredTask, u64> = BTreeMap::new();
    for by_task in cells {
        for (task, samples) in by_task {
            let weight = weights.entry(*task).or_default();
            *weight = weight.saturating_add(samples.len() as u64);
        }
    }
    weights
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

/// Figures are compared in fixed point: one unit is `1 / FIXED_SCALE` of
/// the figure (a rate, USD micros, or seconds).
const FIXED_SCALE: u128 = 1_000_000_000_000_000_000;

/// `num / den` rounded half-up. `None` when `den` is zero or on overflow.
fn div_half_up(num: u128, den: u128) -> Option<u128> {
    if den == 0 {
        return None;
    }
    Some(num.checked_mul(2)?.checked_add(den)? / den.checked_mul(2)?)
}

/// A cell's figure, in fixed point, from its sessions.
trait Point: Copy {
    /// `None` when the figure is undefined (no edits, no sessions).
    fn cell_value(points: &[Self]) -> Option<u128>;
}

/// A session's rejected count under the question's basis, and its edits.
#[derive(Debug, Clone, Copy)]
struct RejectionPoint {
    rejected: u64,
    edits: u64,
}

impl Point for RejectionPoint {
    fn cell_value(points: &[Self]) -> Option<u128> {
        let (rejected, edits) = points.iter().fold((0u128, 0u128), |(r, e), point| {
            (r + u128::from(point.rejected), e + u128::from(point.edits))
        });
        div_half_up(rejected.checked_mul(FIXED_SCALE)?, edits)
    }
}

/// A priced session's cost in USD micros.
#[derive(Debug, Clone, Copy)]
struct CostPoint(u64);

impl Point for CostPoint {
    fn cell_value(points: &[Self]) -> Option<u128> {
        let mut values: Vec<u64> = points.iter().map(|point| point.0).collect();
        u128::from(median(&mut values)?).checked_mul(FIXED_SCALE)
    }
}

impl Point for TimePoint {
    fn cell_value(points: &[Self]) -> Option<u128> {
        let mut values: Vec<u64> = points.iter().map(|point| point.active_secs).collect();
        u128::from(median(&mut values)?).checked_mul(FIXED_SCALE)
    }
}

/// The weighted mean of the cells' figures, weights renormalized over the
/// cells given. With one cell, its figure.
fn weighted<P: Point>(
    cells: &[(InferredTask, Vec<P>)],
    weights: &BTreeMap<InferredTask, u64>,
) -> Option<u128> {
    let mut sum = 0u128;
    let mut total = 0u128;
    for (task, points) in cells {
        let weight = u128::from(*weights.get(task)?);
        sum = sum.checked_add(weight.checked_mul(P::cell_value(points)?)?)?;
        total = total.checked_add(weight)?;
    }
    div_half_up(sum, total)
}

/// A model with at least one qualifying cell: its qualifying cells and all
/// its sessions in the question.
struct Entrant<T> {
    label: String,
    cells: Vec<(InferredTask, Vec<T>)>,
    in_question: u32,
}

/// The entrants whose qualifying task types carry at least
/// `MODEL_ANSWER_MIN_MIX_COVERAGE` of the question's weight; the others'
/// sessions are counted in `mix_too_different`. A one-type question's
/// entrants always pass.
fn covering<T>(
    entrants: Vec<Entrant<T>>,
    weights: &BTreeMap<InferredTask, u64>,
    left_out: &mut LeftOut,
) -> Vec<Entrant<T>> {
    let total = weights
        .values()
        .fold(0u64, |sum, weight| sum.saturating_add(*weight));
    let mut kept = Vec::new();
    for entrant in entrants {
        let covered = entrant.cells.iter().fold(0u64, |sum, (task, _)| {
            sum.saturating_add(weights.get(task).copied().unwrap_or(0))
        });
        if MODEL_ANSWER_MIN_MIX_COVERAGE.reached_by(covered, total) {
            kept.push(entrant);
        } else {
            left_out.mix_too_different = left_out
                .mix_too_different
                .saturating_add(entrant.in_question);
        }
    }
    kept
}

/// A compared model: its wire figure, the cells its figure is drawn from,
/// and the keys of the total order.
struct Contender<P> {
    figure: ModelFigure,
    cells: Vec<(InferredTask, Vec<P>)>,
    value: u128,
    /// Edits (rejection), priced sessions (cost) or finished sessions
    /// (time).
    support: u64,
    sessions: u64,
}

impl<P> Contender<P> {
    /// Lower figure first, then more support, then more sessions, then
    /// label byte order: a total order, so iteration order never decides.
    fn order(&self, other: &Self) -> Ordering {
        self.value
            .cmp(&other.value)
            .then(other.support.cmp(&self.support))
            .then(other.sessions.cmp(&self.sessions))
            .then(
                self.figure
                    .label
                    .as_bytes()
                    .cmp(other.figure.label.as_bytes()),
            )
    }
}

/// SplitMix64: the bootstrap's inline PRNG.
fn split_mix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    z ^ (z >> 31)
}

/// An index in `0..len` from the next draw (`len > 0`).
fn pick(state: &mut u64, len: usize) -> usize {
    ((u128::from(split_mix64(state)) * len as u128) >> 64) as usize
}

fn resampled<P: Point>(
    contender: &Contender<P>,
    weights: &BTreeMap<InferredTask, u64>,
    state: &mut u64,
    scratch: &mut Vec<(InferredTask, Vec<P>)>,
) -> Option<u128> {
    scratch.clear();
    for (task, points) in &contender.cells {
        let drawn = (0..points.len())
            .map(|_| points[pick(state, points.len())])
            .collect();
        scratch.push((*task, drawn));
    }
    weighted(scratch, weights)
}

/// Whether the leader leads the next model in at least
/// `MODEL_ANSWER_LEAD_SHARE` of the draws. One PRNG from the fixed seed per
/// question; each draw resamples the leader's cells, then the next model's,
/// each cell's sessions in `session_ref` order.
fn clear_lead<P: Point>(
    leader: &Contender<P>,
    next: &Contender<P>,
    weights: &BTreeMap<InferredTask, u64>,
    draws: &mut u32,
) -> bool {
    let mut state = MODEL_ANSWER_BOOTSTRAP_SEED;
    let mut scratch = Vec::new();
    let mut leads = 0u64;
    for _ in 0..MODEL_ANSWER_BOOTSTRAP_DRAWS {
        *draws = draws.saturating_add(1);
        let lead = resampled(leader, weights, &mut state, &mut scratch);
        let trail = resampled(next, weights, &mut state, &mut scratch);
        if let (Some(lead), Some(trail)) = (lead, trail)
            && lead < trail
        {
            leads += 1;
        }
    }
    MODEL_ANSWER_LEAD_SHARE.reached_by(leads, u64::from(MODEL_ANSWER_BOOTSTRAP_DRAWS))
}

/// Sorts the contenders and settles the row: fewer than two is not enough
/// sessions; otherwise the bootstrap decides between no clear difference
/// and an answer, and both figures are shown either way.
fn settle<P: Point>(
    row: &mut ModelAnswerRow,
    mut contenders: Vec<Contender<P>>,
    weights: &BTreeMap<InferredTask, u64>,
    draws: &mut u32,
) {
    contenders.sort_by(|a, b| a.order(b));
    let [leader, next, ..] = contenders.as_slice() else {
        row.state = AnswerState::NotEnoughSessions;
        return;
    };
    row.state = if clear_lead(leader, next, weights, draws) {
        AnswerState::Answered
    } else {
        AnswerState::NoClearDifference
    };
    row.leader = Some(leader.figure.clone());
    row.next = Some(next.figure.clone());
}

/// A fixed-point figure, rounded half-up once to whole units.
fn fixed_units(value: u128) -> Option<u128> {
    div_half_up(value, FIXED_SCALE)
}

fn fixed_permille(value: u128) -> Option<u32> {
    u32::try_from(div_half_up(value.checked_mul(1_000)?, FIXED_SCALE)?).ok()
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

/// Questions 1-3 (one task type) and 5 (every task type with edits).
fn rejection_row(
    prepared: &Prepared,
    question: ModelQuestion,
    tasks: &[InferredTask],
    draws: &mut u32,
) -> ModelAnswerRow {
    let (cells, mut left_out) = rejection_cells(prepared, tasks);
    let mut entrants = Vec::new();
    for (label, by_task) in &cells {
        let mut qualifying = Vec::new();
        for (task, samples) in by_task {
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
            qualifying.push((*task, samples.clone()));
        }
        if !qualifying.is_empty() {
            entrants.push(Entrant {
                label: label.clone(),
                cells: qualifying,
                in_question: sessions_of(by_task),
            });
        }
    }
    let weights = entrant_weights(&cells, &entrants);
    let compared = covering(entrants, &weights, &mut left_out);
    let mut row = empty_row(question, session_counts(&cells), left_out);
    if compared.len() < 2 {
        return row;
    }
    let basis = if compared.iter().all(|entrant| {
        entrant
            .cells
            .iter()
            .all(|(_, samples)| samples.iter().all(|sample| sample.denied.is_some()))
    }) {
        RejectionBasis::Rejected
    } else {
        RejectionBasis::Undone
    };
    let mut contenders = Vec::new();
    for entrant in compared {
        let mut edits = 0u64;
        let mut sessions = 0u64;
        let mut interrupts = Some(0u64);
        let mut cells = Vec::new();
        for (task, samples) in &entrant.cells {
            let mut points = Vec::new();
            for sample in samples {
                let rejected = match basis {
                    // `Rejected` is chosen only when every compared
                    // session's `denied` is known, so no unknown reads as 0.
                    RejectionBasis::Rejected => {
                        u64::from(sample.denied.unwrap_or(0)) + u64::from(sample.undone)
                    }
                    RejectionBasis::Undone => u64::from(sample.undone),
                };
                points.push(RejectionPoint {
                    rejected,
                    edits: u64::from(sample.edits),
                });
                edits = edits.saturating_add(u64::from(sample.edits));
                sessions += 1;
                interrupts = interrupts
                    .zip(sample.interrupted)
                    .map(|(sum, count)| sum.saturating_add(u64::from(count)));
            }
            cells.push((*task, points));
        }
        let Some(value) = weighted(&cells, &weights) else {
            continue;
        };
        let Some(rate) = fixed_permille(value) else {
            continue;
        };
        contenders.push(Contender {
            figure: ModelFigure {
                label: entrant.label,
                sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
                edits: Some(u32::try_from(edits).unwrap_or(u32::MAX)),
                figure: Figure::RatePermille { value: rate },
                interrupts_per_session_permille: interrupts.and_then(|sum| permille(sum, sessions)),
            },
            cells,
            value,
            support: edits,
            sessions,
        });
    }
    row.basis = Some(basis);
    settle(&mut row, contenders, &weights, draws);
    row
}

/// Question 4: median cost per Docs session over priced sessions.
fn cost_row(
    prepared: &Prepared,
    tables: &mut BTreeSet<PricingTableRef>,
    draws: &mut u32,
) -> ModelAnswerRow {
    let (cells, left_out) = cost_cells(prepared, &[InferredTask::Docs]);
    let weights = mix_weights(&cells);
    let mut row = empty_row(ModelQuestion::DocsCost, session_counts(&cells), left_out);
    let enough = cells
        .values()
        .filter(|by_task| sessions_of(by_task) >= MODEL_ANSWER_MIN_SESSIONS)
        .count();
    let mut contenders = Vec::new();
    for (label, by_task) in &cells {
        let mut priced_cells = Vec::new();
        let mut priced_count = 0u64;
        for (task, samples) in by_task {
            let priced: Vec<CostPoint> = samples
                .iter()
                .filter_map(|candidate| candidate.cost_usd_micros.map(CostPoint))
                .collect();
            for candidate in samples
                .iter()
                .filter(|candidate| candidate.cost_usd_micros.is_some())
            {
                tables.extend(candidate.tables.iter().cloned());
            }
            if saturating_len(priced.len()) < MODEL_ANSWER_MIN_SESSIONS {
                continue;
            }
            priced_count += priced.len() as u64;
            priced_cells.push((*task, priced));
        }
        if priced_cells.is_empty() {
            continue;
        }
        let Some(value) = weighted(&priced_cells, &weights) else {
            continue;
        };
        let Some(usd) = fixed_units(value).and_then(|units| u64::try_from(units).ok()) else {
            continue;
        };
        contenders.push(Contender {
            figure: ModelFigure {
                label: label.clone(),
                sessions: u32::try_from(priced_count).unwrap_or(u32::MAX),
                edits: None,
                figure: Figure::UsdMicros { value: usd },
                interrupts_per_session_permille: None,
            },
            cells: priced_cells,
            value,
            support: priced_count,
            sessions: u64::from(sessions_of(by_task)),
        });
    }
    if enough < 2 {
        return row;
    }
    if contenders.len() < 2 {
        row.state = AnswerState::NotPriced;
        return row;
    }
    settle(&mut row, contenders, &weights, draws);
    row
}

/// Question 6's cells in one `edit_mode`, and the entrants: the models with
/// at least one qualifying cell there, before the mix rule.
fn time_entrants(
    prepared: &Prepared,
    mode: EditMode,
) -> (Cells<TimeSample>, LeftOut, Vec<Entrant<TimeSample>>) {
    let (cells, left_out) = time_cells(prepared, &EDIT_TASKS, mode);
    let mut entrants = Vec::new();
    for (label, by_task) in &cells {
        let mut qualifying = Vec::new();
        for (task, samples) in by_task {
            let finished = samples.iter().filter(|sample| sample.is_some()).count();
            if saturating_len(finished) >= MODEL_ANSWER_MIN_SESSIONS {
                qualifying.push((*task, samples.clone()));
            }
        }
        if !qualifying.is_empty() {
            entrants.push(Entrant {
                label: label.clone(),
                cells: qualifying,
                in_question: sessions_of(by_task),
            });
        }
    }
    (cells, left_out, entrants)
}

/// Question 6's contenders among the entrants of one `edit_mode`, after the
/// mix rule.
fn time_contenders(
    cells: &Cells<TimeSample>,
    entrants: Vec<Entrant<TimeSample>>,
    left_out: &mut LeftOut,
) -> (BTreeMap<InferredTask, u64>, Vec<Contender<TimePoint>>) {
    let weights = entrant_weights(cells, &entrants);
    let mut contenders = Vec::new();
    for entrant in covering(entrants, &weights, left_out) {
        let sessions = entrant
            .cells
            .iter()
            .map(|(_, samples)| samples.len() as u64)
            .sum::<u64>();
        let cells: Vec<(InferredTask, Vec<TimePoint>)> = entrant
            .cells
            .into_iter()
            .map(|(task, samples)| (task, samples.into_iter().flatten().collect()))
            .collect();
        let finished = cells.iter().map(|(_, points)| points.len() as u64).sum();
        // Turns beside the time, weighted the same way; not compared.
        let turns: Vec<(InferredTask, Vec<CostPoint>)> = cells
            .iter()
            .map(|(task, points)| {
                (
                    *task,
                    points
                        .iter()
                        .map(|point| CostPoint(point.user_turns))
                        .collect(),
                )
            })
            .collect();
        let (Some(value), Some(turns)) = (weighted(&cells, &weights), weighted(&turns, &weights))
        else {
            continue;
        };
        let wire = |fixed: u128| {
            fixed_units(fixed)
                .and_then(|units| u32::try_from(units).ok())
                .unwrap_or(u32::MAX)
        };
        contenders.push(Contender {
            figure: ModelFigure {
                label: entrant.label,
                sessions: u32::try_from(finished).unwrap_or(u32::MAX),
                edits: None,
                figure: Figure::FinishTime {
                    median_active_secs: wire(value),
                    median_user_turns: wire(turns),
                },
                interrupts_per_session_permille: None,
            },
            cells,
            value,
            support: finished,
            sessions,
        });
    }
    (weights, contenders)
}

/// Question 6: sessions of one `edit_mode`, `NotPrompting` when at least
/// two models have a qualifying cell there, else `Prompting`; `Unknown`
/// never. The mode is chosen before the mix rule.
fn time_row(prepared: &Prepared, draws: &mut u32) -> ModelAnswerRow {
    let not_prompting = time_entrants(prepared, EditMode::NotPrompting);
    let (mode, (cells, mut left_out, entrants)) = if not_prompting.2.len() >= 2 {
        (EditMode::NotPrompting, not_prompting)
    } else {
        (
            EditMode::Prompting,
            time_entrants(prepared, EditMode::Prompting),
        )
    };
    let (weights, contenders) = time_contenders(&cells, entrants, &mut left_out);
    let mut row = empty_row(ModelQuestion::FinishTime, session_counts(&cells), left_out);
    row.edit_mode = Some(mode);
    settle(&mut row, contenders, &weights, draws);
    row
}

pub fn model_answers(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
) -> ModelAnswers {
    model_answers_counting(sessions, routes, catalog, now, &mut BTreeMap::new())
}

/// [`model_answers`], counting each question's bootstrap draws: the seam
/// the draw-count test reads.
fn model_answers_counting(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
    draws: &mut BTreeMap<ModelQuestion, u32>,
) -> ModelAnswers {
    let window_start = now - Duration::weeks(MODEL_ANSWER_WINDOW_WEEKS);
    let prepared = prepare(sessions, routes, catalog, window_start, now);
    let mut tables = BTreeSet::new();
    let rows = ModelQuestion::ALL
        .iter()
        .map(|question| {
            let draws = draws.entry(*question).or_default();
            match question {
                ModelQuestion::BugFixRejected => {
                    rejection_row(&prepared, *question, &[InferredTask::BugFix], draws)
                }
                ModelQuestion::RefactorRejected => {
                    rejection_row(&prepared, *question, &[InferredTask::Refactor], draws)
                }
                ModelQuestion::TestsRejected => {
                    rejection_row(&prepared, *question, &[InferredTask::Tests], draws)
                }
                ModelQuestion::DocsCost => cost_row(&prepared, &mut tables, draws),
                ModelQuestion::RejectedAcrossTasks => {
                    rejection_row(&prepared, *question, &EDIT_TASKS, draws)
                }
                ModelQuestion::FinishTime => time_row(&prepared, draws),
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
