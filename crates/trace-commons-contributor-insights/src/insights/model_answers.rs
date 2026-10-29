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
//! - `left_out` counts sessions. `automated` sessions (started by a tool,
//!   not the person: `SessionInput::automated`) are compared nowhere and
//!   counted on every row, before any other reason, facts or not.
//!   `unknown_facts` sessions have no known task,
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
//! - `too_few_sessions` counts the sessions of cells under the session
//!   floor (for time, every session of a cell with too few finished; for
//!   cost, the priced sessions of a cell with too few priced), whatever
//!   the model. `too_few_edits` counts the sessions of cells that reach the
//!   session floor but not the edit floor. Every session of the two models
//!   shown is then in a figure or one reason: a qualifying cell outside
//!   their shared types is counted in `mix_too_different`.
//! - A median of an even count is the mean of the two middle values,
//!   rounded half-up. A cell's figure is held in fixed point at
//!   `1 / FIXED_SCALE` (10^-18) of a unit, rounded half-up once there; a
//!   one-type figure is then exact to the wire, and rounded half-up to
//!   permille, micros or seconds once.
//! - The NEAR AI charge (owner decision D5, settled 2026-10-20) is its own
//!   figure, `near_ai_charge`, on the Docs cost row and on that row's
//!   `leader` and `next`: the median charge per session over the Docs
//!   sessions sent through NEAR AI whose charge is known, with how many
//!   were sent and how many charges are unknown. It is never added to a
//!   list-priced figure, and a session's charge and its list estimate never
//!   meet in one figure: the list-priced figures take unrouted sessions
//!   only. The row's figure counts every such session, whichever model
//!   served it. A model's counts only the sessions whose route is verified
//!   and whose calls all named that model as served, with it as the
//!   majority label (`attributed_to`): a transcript label may not name the
//!   model NEAR AI served, so any other session's charge is on the row's
//!   figure alone. The median is held to the session floor
//!   (`MODEL_ANSWER_MIN_SESSIONS` known charges); under it the counts stand
//!   and the median is `None`. A session with no call through NEAR AI is in
//!   no charge figure; with none in the window the figure is `None`, never
//!   zero. With `LEDGER_MONEY_IN_INSIGHTS` held (owner decision D5) the cost
//!   row carries no money at all: `not_priced`, no figure, no charge.
//! - `basis` is set on a rejection row that compares two models, `None`
//!   elsewhere. A cost figure's `sessions` is its priced sessions, the
//!   median's sample; a time figure's is its finished sessions.
//! - Across task types (Q5, Q6), two models are compared over the task
//!   types where both have a qualifying cell, each type weighted by the two
//!   models' sessions in it, so a model worse in every shared type never
//!   leads on the types only it qualifies in. A pair whose shared types
//!   carry less than `MODEL_ANSWER_MIN_MIX_COVERAGE` of the two models'
//!   sessions in the question is not compared. A model with no qualifying
//!   cell anywhere is never compared. `mix_too_different` counts the
//!   sessions in the qualifying cells of a model that no other model with
//!   a qualifying cell can be compared with. Q1-Q4 have one type, so every
//!   pair shares all of it.
//! - The leader is the model lower in the most of its comparisons, then
//!   higher in the fewest, then with more sessions in the question, then
//!   first in label byte order; the next model is, of the models compared
//!   with it, the one first in that same standing. Both figures are the
//!   pair's, the lower first. With one type this is the total order.
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
//! - The bootstrap compares the leader and the next model only, over their
//!   shared types. Per draw it resamples the leader's cells there, then the
//!   next model's, each in task order and each cell's sessions in
//!   `session_ref` order, with one SplitMix64 stream per question from the
//!   fixed seed; the type weights stay the pair's, from the whole sample. A draw whose figure is
//!   undefined (a resample with no edits) is not a lead. A row with no
//!   clear difference still carries both figures, in the total order.
//! - The person's own labels (`session_labels`, by `session_ref`): a task
//!   they set replaces the inferred task wherever one is read, after the
//!   rules are re-run on the stored shape, so it decides which rows the
//!   session is on (a session inferred as no edits that they name is on
//!   that task's rows). An outcome they set is counted in `kept`, on the
//!   rejection rows only (Q1-Q3, Q5): per model, over the row's sessions
//!   that are compared (by route) and whose majority model it is, whatever
//!   else leaves them out of the rate. It never moves a rate, an order or a
//!   state, and a session with no outcome is in no `kept` figure.

use std::cmp::{Ordering, Reverse};
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use trace_commons_protocol::insights_pricing::PricingAccounting;

use super::analytics_constants::LEDGER_MONEY_IN_INSIGHTS;
use super::analytics_constants::{
    MODEL_ANSWER_BOOTSTRAP_DRAWS, MODEL_ANSWER_BOOTSTRAP_SEED, MODEL_ANSWER_LEAD_SHARE,
    MODEL_ANSWER_MIN_EDITS, MODEL_ANSWER_MIN_MIX_COVERAGE, MODEL_ANSWER_MIN_SESSIONS,
    MODEL_ANSWER_WINDOW_WEEKS,
};
use super::pricing_catalog::PricingCatalog;
use super::session_facts::{EditMode, InferredTask, ModelFacts, SessionFacts};
use super::session_labels::{SessionLabel, SessionOutcome};
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
    /// On a cost figure only: what NEAR AI charged for this model's
    /// sessions through it, apart from `figure` and never added to it: only
    /// sessions whose route is verified and whose calls all named this
    /// model as served. `None` when there are none.
    pub near_ai_charge: Option<NearAiChargeFigure>,
    /// On a rejection figure only: what the person said became of this
    /// model's work in the row's sessions. `None` elsewhere and when they
    /// gave none of those sessions an outcome.
    pub kept: Option<KeptFigure>,
}

/// The outcomes the person gave one model's sessions in a row (their own
/// labels, `session_labels::SessionOutcome`): an acceptance signal beside
/// the rejection rate, never folded into it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct KeptFigure {
    /// Sessions with an outcome; the three counts below sum to it.
    pub labelled: u32,
    pub kept: u32,
    pub partly_kept: u32,
    pub thrown_away: u32,
    /// `kept / labelled` in permille, rounded half-up once. Partly kept is
    /// not kept.
    pub kept_permille: u32,
}

/// One session's NEAR AI charge, as its route tally kept it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionCharge {
    /// Everything NEAR AI charged in the session, in integer USD micros;
    /// `None` when it is unknown (a call without a charge, or an overflow).
    pub usd_micros: Option<u64>,
    /// The normalized label of the one model every NEAR AI call in the
    /// session named as served; `None` when a call named none, or calls
    /// named more than one.
    pub served_model: Option<String>,
}

/// What a NEAR AI charge figure is: the proxy ledger's `cost_usd` (owner
/// decision D5, settled 2026-10-20), never a list price.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ChargeBasis {
    NearAiLedger,
}

/// The NEAR AI charge per session, over the sessions sent through NEAR AI:
/// a figure of its own beside the API-price figure, never summed with it.
/// A session's charge is everything NEAR AI charged in it; a session with
/// no call through NEAR AI is in no charge figure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct NearAiChargeFigure {
    pub basis: ChargeBasis,
    /// The median charge per session over the sessions whose charge is
    /// known, as `Figure::UsdMicros`. `None` when no charge is known.
    pub median: Option<Figure>,
    /// Sessions sent through NEAR AI, known charge or not.
    pub routed_sessions: u32,
    /// Of those, the sessions whose charge is unknown (a call without one).
    pub charge_unknown: u32,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct LeftOut {
    pub unknown_facts: u32,
    /// Sessions known to be automated (`SessionInput::automated`): never
    /// compared, counted on every row.
    pub automated: u32,
    pub delegated: u32,
    pub capped: u32,
    pub routed: u32,
    pub reviewer: u32,
    pub no_majority: u32,
    pub unpriced: u32,
    /// Sessions of cells under the session floor (`MODEL_ANSWER_MIN_SESSIONS`
    /// sessions, finished ones for time, priced ones for cost).
    pub too_few_sessions: u32,
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
    /// On the Docs cost row only: the NEAR AI charge over every Docs
    /// session sent through NEAR AI, whichever model served it. `None`
    /// elsewhere and when no such session is in the window.
    pub near_ai_charge: Option<NearAiChargeFigure>,
    /// On the rejection rows only: each model with at least one session in
    /// the row the person gave an outcome, by label, whatever the row's
    /// state. Empty elsewhere and when none has an outcome.
    pub kept_by_model: Vec<(String, KeptFigure)>,
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
    pub(crate) fn compared(self) -> bool {
        matches!(
            self,
            ModelRoute::Unobserved | ModelRoute::Outside | ModelRoute::RoutedVerified
        )
    }

    /// The cost question list-prices sessions with no route tally only:
    /// list prices do not describe a routed session, and what NEAR AI
    /// charged for one is its own figure (`NearAiChargeFigure`).
    pub(crate) fn priced_at_list(self) -> bool {
        self == ModelRoute::Unobserved
    }
}

/// The majority model's own counts, summed over every declared label that
/// normalizes to it. `None` is unknown, never zero.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OwnCounts {
    pub(crate) edits: Option<u32>,
    pub(crate) denied: Option<u32>,
    pub(crate) undone: Option<u32>,
    pub(crate) interrupted: Option<u32>,
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
    /// The NEAR AI charge: `None` with no call through NEAR AI.
    near_ai_charge: Option<SessionCharge>,
    /// The outcome the person gave the session, if any.
    outcome: Option<SessionOutcome>,
}

struct Prepared {
    /// In `session_ref` order, so every cell visits its sessions in one
    /// order whatever the input order.
    candidates: Vec<Candidate>,
    unknown_facts: u32,
    automated: u32,
    reviewer: BTreeMap<InferredTask, u32>,
}

impl Prepared {
    fn left_out(&self, tasks: &[InferredTask]) -> LeftOut {
        LeftOut {
            unknown_facts: self.unknown_facts,
            automated: self.automated,
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
pub(crate) fn latest_at(session: &SessionInput) -> Option<DateTime<Utc>> {
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

pub(crate) fn own_counts(models: &[ModelFacts], labels: &[String], majority: &str) -> OwnCounts {
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
    charges: &BTreeMap<String, SessionCharge>,
    labels: &BTreeMap<String, SessionLabel>,
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
        automated: 0,
        reviewer: BTreeMap::new(),
    };
    for session in ordered {
        let Some(at) = latest_at(session) else {
            continue;
        };
        if at < window_start || at > now {
            continue;
        }
        // Started by a tool, not the person: compared nowhere, facts or not.
        if session.automated {
            bump(&mut prepared.automated);
            continue;
        }
        let Some(facts) = &session.facts else {
            bump(&mut prepared.unknown_facts);
            continue;
        };
        // The person's own labels; `labels` below are the declared models.
        let own_label = labels.get(&session.session_ref);
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
        // The person's task over the inferred one.
        if let Some(task) = own_label.and_then(|label| label.task) {
            facts.task = task.inferred();
        }
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
            near_ai_charge: charges.get(&session.session_ref).cloned(),
            outcome: own_label.and_then(|label| label.outcome),
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

fn saturating_len(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

/// `num / den` in permille, rounded half-up once. `None` when `den` is zero.
pub(crate) fn permille(num: u64, den: u64) -> Option<u32> {
    if den == 0 {
        return None;
    }
    let (num, den) = (u128::from(num), u128::from(den));
    u32::try_from((num * 2_000 + den) / (den * 2)).ok()
}

/// The median; of an even count, the two middle values' mean rounded
/// half-up. `None` when empty.
pub(crate) fn median(values: &mut [u64]) -> Option<u64> {
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

/// The weighted mean of the cells' figures under a pair's weights, which
/// name exactly the cells given. With one cell, its figure.
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

impl<T> Entrant<T> {
    /// Sessions in the qualifying cells outside the types `weights` names
    /// (all of them with no weights).
    fn unshared(&self, weights: Option<&BTreeMap<InferredTask, u64>>) -> u32 {
        let sessions = self
            .cells
            .iter()
            .filter(|(task, _)| weights.is_none_or(|weights| !weights.contains_key(task)))
            .map(|(_, samples)| samples.len())
            .sum();
        saturating_len(sessions)
    }

    /// The qualifying cells in the types a pair's weights name.
    fn shared<'e>(
        &'e self,
        weights: &'e BTreeMap<InferredTask, u64>,
    ) -> impl Iterator<Item = &'e (InferredTask, Vec<T>)> {
        self.cells
            .iter()
            .filter(|(task, _)| weights.contains_key(task))
    }
}

/// The weights two entrants are compared under: each task type both
/// qualify in, weighted by the two models' sessions there. `None` when
/// those types carry less than `MODEL_ANSWER_MIN_MIX_COVERAGE` of the two
/// models' sessions in the question. A one-type question's pairs always
/// pass.
fn pair_weights<T>(a: &Entrant<T>, b: &Entrant<T>) -> Option<BTreeMap<InferredTask, u64>> {
    let mut weights = BTreeMap::new();
    for (task, ours) in &a.cells {
        if let Some((_, theirs)) = b.cells.iter().find(|(other, _)| other == task) {
            weights.insert(
                *task,
                (ours.len() as u64).saturating_add(theirs.len() as u64),
            );
        }
    }
    let shared = weights
        .values()
        .fold(0u64, |sum, weight| sum.saturating_add(*weight));
    let total = u64::from(a.in_question).saturating_add(u64::from(b.in_question));
    MODEL_ANSWER_MIN_MIX_COVERAGE
        .reached_by(shared, total)
        .then_some(weights)
}

/// The entrants that can be compared with at least one other entrant.
/// With two or more entrants, the sessions in the qualifying cells of the
/// others are counted in `mix_too_different` (their other cells are
/// counted under the floor they missed); a lone entrant is kept, and reads
/// not enough sessions.
fn comparable<T>(entrants: Vec<Entrant<T>>, left_out: &mut LeftOut) -> Vec<Entrant<T>> {
    if entrants.len() < 2 {
        return entrants;
    }
    let paired: Vec<bool> = entrants
        .iter()
        .enumerate()
        .map(|(ix, entrant)| {
            entrants
                .iter()
                .enumerate()
                .any(|(other, rival)| other != ix && pair_weights(entrant, rival).is_some())
        })
        .collect();
    let mut kept = Vec::new();
    for (entrant, paired) in entrants.into_iter().zip(paired) {
        if paired {
            kept.push(entrant);
        } else {
            left_out.mix_too_different = left_out
                .mix_too_different
                .saturating_add(entrant.unshared(None));
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
/// each cell's sessions in `session_ref` order. Both hold only the pair's
/// shared types, and `weights` are the pair's.
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

/// One comparison of two entrants over their shared types: the pair's
/// weights and both figures, the lower in the total order first.
struct Pairing<P> {
    weights: BTreeMap<InferredTask, u64>,
    lower: (usize, Contender<P>),
    higher: (usize, Contender<P>),
}

impl<P> Pairing<P> {
    fn has(&self, ix: usize) -> bool {
        self.lower.0 == ix || self.higher.0 == ix
    }

    fn rival_of(&self, ix: usize) -> usize {
        if self.lower.0 == ix {
            self.higher.0
        } else {
            self.lower.0
        }
    }
}

/// Compares every pair of entrants over the types both qualify in, under
/// the pair's weights (`figure` builds an entrant's contender from its
/// cells there), and settles the row. With no pair whose two figures are
/// defined, not enough sessions. Otherwise the leader is the entrant that
/// is lower in the most of its comparisons, then higher in the fewest, then
/// has more sessions in the question, then comes first in label byte
/// order; the next model is, of the entrants compared with it, the one
/// first in that same standing. The two are shown as their comparison
/// reads them, the lower figure first, and the bootstrap over their shared
/// cells decides between no clear difference and an answer. With one task
/// type every figure is the model's whole cell, so this is the total order.
fn settle<T, P: Point>(
    row: &mut ModelAnswerRow,
    entrants: &[Entrant<T>],
    figure: impl Fn(&Entrant<T>, &BTreeMap<InferredTask, u64>) -> Option<Contender<P>>,
    draws: &mut u32,
) {
    let mut lower_in = vec![0u32; entrants.len()];
    let mut higher_in = vec![0u32; entrants.len()];
    let mut pairings = Vec::new();
    for (ix, entrant) in entrants.iter().enumerate() {
        for (other, rival) in entrants.iter().enumerate().skip(ix + 1) {
            let Some(weights) = pair_weights(entrant, rival) else {
                continue;
            };
            let (Some(ours), Some(theirs)) = (figure(entrant, &weights), figure(rival, &weights))
            else {
                continue;
            };
            let (lower, higher) = if ours.order(&theirs).is_le() {
                ((ix, ours), (other, theirs))
            } else {
                ((other, theirs), (ix, ours))
            };
            bump(&mut lower_in[lower.0]);
            bump(&mut higher_in[higher.0]);
            pairings.push(Pairing {
                weights,
                lower,
                higher,
            });
        }
    }
    let standing = |ix: usize| {
        (
            Reverse(lower_in[ix]),
            higher_in[ix],
            Reverse(entrants[ix].in_question),
            entrants[ix].label.as_bytes(),
        )
    };
    let Some(top) = pairings
        .iter()
        .flat_map(|pairing| [pairing.lower.0, pairing.higher.0])
        .min_by_key(|ix| standing(*ix))
    else {
        row.state = AnswerState::NotEnoughSessions;
        return;
    };
    let Some(pairing) = pairings
        .iter()
        .filter(|pairing| pairing.has(top))
        .min_by_key(|pairing| standing(pairing.rival_of(top)))
    else {
        row.state = AnswerState::NotEnoughSessions;
        return;
    };
    // The two models' qualifying cells outside their shared types are in
    // neither figure.
    for ix in [pairing.lower.0, pairing.higher.0] {
        row.left_out.mix_too_different = row
            .left_out
            .mix_too_different
            .saturating_add(entrants[ix].unshared(Some(&pairing.weights)));
    }
    let (leader, next) = (&pairing.lower.1, &pairing.higher.1);
    row.state = if clear_lead(leader, next, &pairing.weights, draws) {
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
        near_ai_charge: None,
        kept_by_model: Vec::new(),
    }
}

/// Each model's [`KeptFigure`] over the row's sessions of `tasks` that are
/// compared by route and have a majority model, by majority label. A model
/// with no session the person gave an outcome is absent.
fn kept_by_model(prepared: &Prepared, tasks: &[InferredTask]) -> BTreeMap<String, KeptFigure> {
    let mut counts: BTreeMap<String, [u32; 3]> = BTreeMap::new();
    for candidate in prepared.of_tasks(tasks) {
        let (Some(model), Some(outcome)) = (&candidate.majority, candidate.outcome) else {
            continue;
        };
        if !candidate.route.compared() {
            continue;
        }
        let slot = match outcome {
            SessionOutcome::Kept => 0,
            SessionOutcome::PartlyKept => 1,
            SessionOutcome::ThrownAway => 2,
        };
        bump(&mut counts.entry(model.clone()).or_default()[slot]);
    }
    counts
        .into_iter()
        .map(|(label, [kept, partly_kept, thrown_away])| {
            let labelled = kept.saturating_add(partly_kept).saturating_add(thrown_away);
            let figure = KeptFigure {
                labelled,
                kept,
                partly_kept,
                thrown_away,
                kept_permille: permille(u64::from(kept), u64::from(labelled)).unwrap_or(0),
            };
            (label, figure)
        })
        .collect()
}

/// The NEAR AI charge figure over `candidates`, or `None` when none went
/// through NEAR AI. Only the ledger's charge enters it, never a list price.
/// Its median is held to the session floor, as every figure is: under
/// `MODEL_ANSWER_MIN_SESSIONS` known charges it is `None`, so one session's
/// exact charge never crosses as a median.
fn near_ai_charge<'c>(
    candidates: impl Iterator<Item = &'c Candidate>,
) -> Option<NearAiChargeFigure> {
    let mut known = Vec::new();
    let mut routed = 0u32;
    let mut unknown = 0u32;
    for charge in candidates.filter_map(|candidate| candidate.near_ai_charge.as_ref()) {
        bump(&mut routed);
        match charge.usd_micros {
            Some(micros) => known.push(micros),
            None => bump(&mut unknown),
        }
    }
    let floor = usize::try_from(MODEL_ANSWER_MIN_SESSIONS).unwrap_or(usize::MAX);
    (routed > 0).then(|| NearAiChargeFigure {
        basis: ChargeBasis::NearAiLedger,
        median: (known.len() >= floor)
            .then(|| median(&mut known))
            .flatten()
            .map(|value| Figure::UsdMicros { value }),
        routed_sessions: routed,
        charge_unknown: unknown,
    })
}

/// Whether a session's NEAR AI charge is `label`'s: its route verified,
/// the one model its calls named as served `label`, and `label` its
/// majority. A transcript label may not name the model that served the
/// call (`ModelRoute::compared`), and a model override serves another model
/// under a verified route, so the label alone never attributes a charge.
fn attributed_to(candidate: &Candidate, label: &str) -> bool {
    candidate.route == ModelRoute::RoutedVerified
        && candidate.majority.as_deref() == Some(label)
        && candidate
            .near_ai_charge
            .as_ref()
            .and_then(|charge| charge.served_model.as_deref())
            == Some(label)
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
                left_out.too_few_sessions = left_out
                    .too_few_sessions
                    .saturating_add(saturating_len(samples.len()));
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
    let compared = comparable(entrants, &mut left_out);
    let mut row = empty_row(question, session_counts(&cells), left_out);
    let kept = kept_by_model(prepared, tasks);
    row.kept_by_model = kept
        .iter()
        .map(|(label, figure)| (label.clone(), *figure))
        .collect();
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
    let figure = |entrant: &Entrant<RejectionSample>, weights: &BTreeMap<InferredTask, u64>| {
        let mut edits = 0u64;
        let mut sessions = 0u64;
        let mut interrupts = Some(0u64);
        let mut cells = Vec::new();
        for (task, samples) in entrant.shared(weights) {
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
        let value = weighted(&cells, weights)?;
        let rate = fixed_permille(value)?;
        Some(Contender {
            figure: ModelFigure {
                kept: kept.get(&entrant.label).copied(),
                label: entrant.label.clone(),
                sessions: u32::try_from(sessions).unwrap_or(u32::MAX),
                edits: Some(u32::try_from(edits).unwrap_or(u32::MAX)),
                figure: Figure::RatePermille { value: rate },
                interrupts_per_session_permille: interrupts.and_then(|sum| permille(sum, sessions)),
                near_ai_charge: None,
            },
            cells,
            value,
            support: edits,
            sessions,
        })
    };
    row.basis = Some(basis);
    settle(&mut row, &compared, figure, draws);
    row
}

/// Question 4: median cost per Docs session over priced sessions.
///
/// `money` is `LEDGER_MONEY_IN_INSIGHTS` (owner decision D5): held, the row
/// is `not_priced` with no figure, no charge and no pricing table.
fn cost_row(
    prepared: &Prepared,
    tables: &mut BTreeSet<PricingTableRef>,
    draws: &mut u32,
    money: bool,
) -> ModelAnswerRow {
    let (cells, left_out) = cost_cells(prepared, &[InferredTask::Docs]);
    let mut row = empty_row(ModelQuestion::DocsCost, session_counts(&cells), left_out);
    if !money {
        row.state = AnswerState::NotPriced;
        return row;
    }
    // The Docs sessions sent through NEAR AI: the route tally leaves them
    // out of the list-priced figures, and their charge is its own figure.
    let docs: Vec<&Candidate> = prepared.of_tasks(&[InferredTask::Docs]).collect();
    row.near_ai_charge = near_ai_charge(docs.iter().copied());
    let enough = cells
        .values()
        .filter(|by_task| sessions_of(by_task) >= MODEL_ANSWER_MIN_SESSIONS)
        .count();
    let priced = |samples: &[&Candidate]| -> Vec<CostPoint> {
        samples
            .iter()
            .filter_map(|candidate| candidate.cost_usd_micros.map(CostPoint))
            .collect()
    };
    let mut entrants = Vec::new();
    for (label, by_task) in &cells {
        let mut qualifying = Vec::new();
        for (task, samples) in by_task {
            for candidate in samples
                .iter()
                .filter(|candidate| candidate.cost_usd_micros.is_some())
            {
                tables.extend(candidate.tables.iter().cloned());
            }
            let priced = saturating_len(priced(samples).len());
            if priced >= MODEL_ANSWER_MIN_SESSIONS {
                qualifying.push((*task, samples.clone()));
            } else {
                // Unpriced sessions are counted as unpriced already.
                row.left_out.too_few_sessions =
                    row.left_out.too_few_sessions.saturating_add(priced);
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
    if enough < 2 {
        return row;
    }
    if entrants.len() < 2 {
        row.state = AnswerState::NotPriced;
        return row;
    }
    let figure = |entrant: &Entrant<&Candidate>, weights: &BTreeMap<InferredTask, u64>| {
        let mut sessions = 0u64;
        let mut cells = Vec::new();
        for (task, samples) in entrant.shared(weights) {
            sessions += samples.len() as u64;
            cells.push((*task, priced(samples)));
        }
        let priced_count = cells.iter().map(|(_, points)| points.len() as u64).sum();
        let value = weighted(&cells, weights)?;
        let usd = fixed_units(value).and_then(|units| u64::try_from(units).ok())?;
        let label = entrant.label.as_str();
        Some(Contender {
            figure: ModelFigure {
                label: label.to_owned(),
                sessions: u32::try_from(priced_count).unwrap_or(u32::MAX),
                edits: None,
                figure: Figure::UsdMicros { value: usd },
                interrupts_per_session_permille: None,
                // Only a charge NEAR AI proved was for this model: the route
                // verified and its one served model this label. Any other
                // is on the row's figure alone (`attributed_to`).
                near_ai_charge: near_ai_charge(
                    docs.iter()
                        .copied()
                        .filter(|candidate| attributed_to(candidate, label)),
                ),
                kept: None,
            },
            cells,
            value,
            support: priced_count,
            sessions,
        })
    };
    settle(&mut row, &entrants, figure, draws);
    row
}

/// Question 6's cells in one `edit_mode`, and the entrants: the models with
/// at least one qualifying cell there, before the mix rule.
fn time_entrants(
    prepared: &Prepared,
    mode: EditMode,
) -> (Cells<TimeSample>, LeftOut, Vec<Entrant<TimeSample>>) {
    let (cells, mut left_out) = time_cells(prepared, &EDIT_TASKS, mode);
    let mut entrants = Vec::new();
    for (label, by_task) in &cells {
        let mut qualifying = Vec::new();
        for (task, samples) in by_task {
            let finished = samples.iter().filter(|sample| sample.is_some()).count();
            if saturating_len(finished) >= MODEL_ANSWER_MIN_SESSIONS {
                qualifying.push((*task, samples.clone()));
            } else {
                left_out.too_few_sessions = left_out
                    .too_few_sessions
                    .saturating_add(saturating_len(samples.len()));
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

/// An entrant's time contender over a pair's shared types: unfinished
/// sessions count as sessions, not in the medians.
fn time_figure(
    entrant: &Entrant<TimeSample>,
    weights: &BTreeMap<InferredTask, u64>,
) -> Option<Contender<TimePoint>> {
    let mut sessions = 0u64;
    let mut cells: Vec<(InferredTask, Vec<TimePoint>)> = Vec::new();
    for (task, samples) in entrant.shared(weights) {
        sessions += samples.len() as u64;
        cells.push((*task, samples.iter().flatten().copied().collect()));
    }
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
    let value = weighted(&cells, weights)?;
    let turns = weighted(&turns, weights)?;
    let wire = |fixed: u128| {
        fixed_units(fixed)
            .and_then(|units| u32::try_from(units).ok())
            .unwrap_or(u32::MAX)
    };
    Some(Contender {
        figure: ModelFigure {
            label: entrant.label.clone(),
            sessions: u32::try_from(finished).unwrap_or(u32::MAX),
            edits: None,
            figure: Figure::FinishTime {
                median_active_secs: wire(value),
                median_user_turns: wire(turns),
            },
            interrupts_per_session_permille: None,
            near_ai_charge: None,
            kept: None,
        },
        cells,
        value,
        support: finished,
        sessions,
    })
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
    let compared = comparable(entrants, &mut left_out);
    let mut row = empty_row(ModelQuestion::FinishTime, session_counts(&cells), left_out);
    row.edit_mode = Some(mode);
    settle(&mut row, &compared, time_figure, draws);
    row
}

pub fn model_answers(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
) -> ModelAnswers {
    model_answers_with_charges(sessions, routes, &BTreeMap::new(), catalog, now)
}

/// [`model_answers`] with each session's NEAR AI charge, by `session_ref`:
/// absent when the session sent no call through NEAR AI. The charges enter
/// only `near_ai_charge` figures. Money of either kind is shown only while
/// `LEDGER_MONEY_IN_INSIGHTS` holds (owner decision D5).
pub fn model_answers_with_charges(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    charges: &BTreeMap<String, SessionCharge>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
) -> ModelAnswers {
    model_answers_labelled(sessions, routes, charges, &BTreeMap::new(), catalog, now)
}

/// [`model_answers_with_charges`] with the person's own labels, by
/// `session_ref`: a task they set replaces the inferred one, and an outcome
/// they set is counted in the rejection rows' `kept` figures. A session with
/// no entry is read as before.
pub fn model_answers_labelled(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    charges: &BTreeMap<String, SessionCharge>,
    labels: &BTreeMap<String, SessionLabel>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
) -> ModelAnswers {
    model_answers_counting(
        sessions,
        routes,
        charges,
        labels,
        catalog,
        now,
        LEDGER_MONEY_IN_INSIGHTS,
        &mut BTreeMap::new(),
    )
}

/// [`model_answers`], counting each question's bootstrap draws: the seam
/// the draw-count test reads.
// Private, and every input is one the public entry points pass through.
#[allow(clippy::too_many_arguments)]
fn model_answers_counting(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    charges: &BTreeMap<String, SessionCharge>,
    labels: &BTreeMap<String, SessionLabel>,
    catalog: &PricingCatalog,
    now: DateTime<Utc>,
    money: bool,
    draws: &mut BTreeMap<ModelQuestion, u32>,
) -> ModelAnswers {
    let window_start = now - Duration::weeks(MODEL_ANSWER_WINDOW_WEEKS);
    let prepared = prepare(
        sessions,
        routes,
        charges,
        labels,
        catalog,
        window_start,
        now,
    );
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
                ModelQuestion::DocsCost => cost_row(&prepared, &mut tables, draws, money),
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
