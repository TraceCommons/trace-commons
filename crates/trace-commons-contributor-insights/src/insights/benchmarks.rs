//! Benchmarks from the person's own sessions: for each kind of work, how
//! each model they used did on Quality (share of its edits kept), Value
//! (list-priced cost per session) and Speed (active time per session), the
//! two leading models on each, and one tip with evidence behind it.
//!
//! Product owner, 2026-10-29: a "Benchmarks" tab modelled on a public model
//! leaderboard, but over the person's own sessions, money at API list
//! prices. Models are compared within a kind of work only, and only once
//! each has [`BENCHMARK_MIN_SESSIONS`] sessions with a known figure there.
//!
//! Local only: no I/O, no network, no persistence. The output carries fixed
//! snake_case keys, declared (normalized) model labels, counts, rates in
//! permille, seconds and money in integer USD micros; never an id, digest,
//! path, file name or backend.
//!
//! Rules this module settles:
//! - The window is the `weeks` ending now; a session is in it by its latest
//!   dated turn (or `placed_at`), as in `model_answers`.
//! - `sessions` counts every session of the chosen tool in the window. Each
//!   is either on exactly one category or counted under exactly one
//!   [`BenchLeftOut`] reason, so the categories' `sessions` and the
//!   `left_out` counts sum to it. With a tool chosen, a session of the
//!   other tool, or of no known tool, is not counted at all.
//! - A session's category is the person's own task label when set, else the
//!   inferred task and shape ([`bench_category`]). `review` has no inferred
//!   source: the facts hold no command text, so nothing tells a review from
//!   reading, and the label set has no review word yet; it is empty until a
//!   label can name it.
//! - A session's model is the normalized label serving a strict majority of
//!   its counted turns; its figures are that model's own counts.
//! - `kept_permille`: `(edits - undone - denied) / edits` summed over the
//!   model's sessions whose `edits` and `undone` are known (`denied` unknown
//!   reads as none declined), rounded half-up; `None` when none is known or
//!   the known sessions made no edit.
//! - `usd_micros_median`: the median list-priced cost over the model's
//!   sessions priced in full with no route tally (a routed session's list
//!   price is not what it cost). `active_secs_median`: the median active
//!   seconds over its sessions where that is known. A median of an even
//!   count is the two middle values' mean, rounded half-up.
//! - A model qualifies for a pick when the figure's own sample (known kept,
//!   priced, timed sessions) has at least `min_sessions` sessions. A pick is
//!   `None` with fewer than two qualifying models. Kept is better higher,
//!   cost and time lower; ties go to more sessions, then label byte order.
//! - The tip is the first kind, in [`Tip`] order, whose evidence holds.
//! - A Claude session that delegated to sub-agents is counted on its own
//!   work (its turns, edits and model) and counted in `delegated`, since its
//!   figures are partial; nothing of the sub-agents' is added to it.
//! - The same rules answer over Claude's sub-agent transcripts, read as
//!   their own population: the caller passes those rows instead of the
//!   person's sessions, and figures are computed within them only.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::Serialize;
use trace_commons_protocol::insights_pricing::PricingAccounting;

use super::analytics_constants::{MODEL_ANSWER_MIN_EDITS, MODEL_ANSWER_MIN_SESSIONS};
use super::model_answers::{ModelRoute, latest_at, median, own_counts, permille};
use super::pricing_catalog::PricingCatalog;
use super::session_facts::{InferredTask, SessionFacts, SessionShape};
use super::session_labels::{LabelledTask, SessionLabel};
use super::session_pricing::{ModelPrice, PricingTableRef, price_session};
use super::task_classifier::reclassify;
use super::week_rollup::{SessionBody, SessionInput, claude_resumes};

/// Sessions with a known figure a model needs in a category before it is
/// compared there: the Models answers' floor.
pub const BENCHMARK_MIN_SESSIONS: u32 = MODEL_ANSWER_MIN_SESSIONS;
/// The windows the method answers over, in weeks.
pub const BENCHMARK_WEEKS: [u32; 2] = [4, 13];
pub const BENCHMARK_DEFAULT_WEEKS: u32 = 4;
/// `test_first`: sessions each group needs.
pub const TEST_FIRST_MIN_GROUP: usize = 3;
/// `cheaper_same_quality`: how far below the quality leader's kept rate a
/// model may be and still read as the same quality.
pub const SAME_QUALITY_PERMILLE: u32 = 30;
/// `cheaper_same_quality`: the cheap model's median cost is at most this
/// share of the leader's.
pub const CHEAPER_COST_RATIO_PERMILLE: u32 = 500;
/// `fewer_undos`: the worse undone rate is at least this multiple of the
/// better one.
pub const FEWER_UNDOS_RATIO_PERMILLE: u32 = 2_000;

/// The kinds of work, in wire order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchCategory {
    BugFix,
    Feature,
    Refactor,
    Tests,
    Docs,
    Understand,
    Review,
    Setup,
}

impl BenchCategory {
    pub const ALL: [BenchCategory; 8] = [
        BenchCategory::BugFix,
        BenchCategory::Feature,
        BenchCategory::Refactor,
        BenchCategory::Tests,
        BenchCategory::Docs,
        BenchCategory::Understand,
        BenchCategory::Review,
        BenchCategory::Setup,
    ];
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BenchTool {
    ClaudeCode,
    Codex,
}

impl BenchTool {
    /// The tool for its wire word.
    pub fn from_wire(wire: &str) -> Option<Self> {
        match wire {
            "claude_code" => Some(BenchTool::ClaudeCode),
            "codex" => Some(BenchTool::Codex),
            _ => None,
        }
    }
}

/// Whether config files are most of a session's edited files.
fn config_dominates(shape: &SessionShape) -> bool {
    let files = shape.edited_files;
    u64::from(files.config) * 2 > u64::from(files.total())
}

/// A session's category: the person's own task label when set, else the
/// inferred task (`facts.task`, current under the classifier) and shape.
/// `Other` (inferred or labelled) is `setup` when config files are most of
/// the edited files, else `feature`; a labelled `Other` on a session with
/// no edits is `understand`. Never `review`: see the module doc.
pub fn bench_category(label: Option<LabelledTask>, facts: &SessionFacts) -> BenchCategory {
    let task = match label {
        Some(LabelledTask::Other) if facts.task == InferredTask::NoEdits => InferredTask::NoEdits,
        Some(label) => label.inferred(),
        None => facts.task,
    };
    match task {
        InferredTask::BugFix => BenchCategory::BugFix,
        InferredTask::Refactor => BenchCategory::Refactor,
        InferredTask::Tests => BenchCategory::Tests,
        InferredTask::Docs => BenchCategory::Docs,
        InferredTask::NoEdits => BenchCategory::Understand,
        InferredTask::Other if config_dominates(&facts.shape) => BenchCategory::Setup,
        InferredTask::Other => BenchCategory::Feature,
    }
}

/// One model's figures in a category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchModel {
    pub label: String,
    pub tool: BenchTool,
    pub sessions: u32,
    pub kept_permille: Option<u32>,
    pub usd_micros_median: Option<u64>,
    pub active_secs_median: Option<u64>,
    /// The samples behind each figure (additive): sessions with known kept
    /// counts, priced sessions, sessions with known active time.
    pub kept_sessions: u32,
    pub priced_sessions: u32,
    pub timed_sessions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PickEntry {
    pub label: String,
    pub tool: BenchTool,
    /// Permille kept, USD micros or seconds, by the pick.
    pub figure: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pick {
    pub leader: PickEntry,
    pub next: PickEntry,
}

/// One tip, the strongest kind with evidence, in this order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Tip {
    /// `bug_fix`: sessions where a command failed before the first edit and
    /// passed after it took less active time. `sessions` is both groups;
    /// `sessions_with` (additive) the test-first group.
    TestFirst {
        faster_permille: u32,
        sessions_without: u32,
        sessions: u32,
        sessions_with: u32,
    },
    CheaperSameQuality {
        cheap: String,
        expensive: String,
        cost_ratio_permille: u32,
    },
    FewerUndos {
        better: String,
        worse: String,
        ratio_permille: u32,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BenchCategoryRow {
    pub key: BenchCategory,
    pub sessions: u32,
    pub models: Vec<BenchModel>,
    pub quality: Option<Pick>,
    pub value: Option<Pick>,
    pub speed: Option<Pick>,
    pub tip: Option<Tip>,
}

/// Sessions in the window on no category, by reason. Each session is
/// counted under the first reason that holds, in field order.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct BenchLeftOut {
    /// Started by a tool, not the person.
    pub automated: u32,
    /// Stored without facts.
    pub no_facts: u32,
    /// No per-turn series or no labelled turn: which model served it is
    /// unknown.
    pub no_model: u32,
    /// The Codex automatic reviewer's sessions.
    pub reviewer: u32,
    /// Read as no edits while a model's edits could not be seen.
    pub unknown_task: u32,
    /// No model served a strict majority of the turns.
    pub mixed_models: u32,
    /// Sent through a route whose label may not name the served model.
    pub routed: u32,
    /// The scan hit a cap.
    pub capped: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Benchmarks {
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub weeks: u32,
    pub sessions: u32,
    pub min_sessions: u32,
    /// Always eight, in [`BenchCategory::ALL`] order.
    pub categories: Vec<BenchCategoryRow>,
    pub left_out: BenchLeftOut,
    /// Sessions on a category that delegated to sub-agents (Claude): each
    /// is counted on its own turns, edits and model only, so its figures
    /// are partial. The sub-agents' work is never added to them.
    pub delegated: u32,
    pub pricing_tables: Vec<PricingTableRef>,
}

/// A session on a category, with its model's own figures.
struct Entry {
    category: BenchCategory,
    tool: BenchTool,
    model: String,
    edits: Option<u32>,
    undone: Option<u32>,
    denied: Option<u32>,
    cost_usd_micros: Option<u64>,
    tables: Vec<PricingTableRef>,
    active_secs: Option<u32>,
    test_first: bool,
    /// Delegated to sub-agents whose work is not in these figures.
    delegated: bool,
}

fn bump(count: &mut u32) {
    *count = count.saturating_add(1);
}

fn len_u32(len: usize) -> u32 {
    u32::try_from(len).unwrap_or(u32::MAX)
}

fn tool_of(body: &SessionBody) -> Option<BenchTool> {
    match body {
        SessionBody::Claude { .. } => Some(BenchTool::ClaudeCode),
        SessionBody::Codex { .. } => Some(BenchTool::Codex),
        SessionBody::Unknown { .. } => None,
    }
}

/// The sessions' entries and the left-out counts, in `session_ref` order.
fn entries(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    labels: &BTreeMap<String, SessionLabel>,
    catalog: &PricingCatalog,
    tool: Option<BenchTool>,
    window_start: DateTime<Utc>,
    now: DateTime<Utc>,
) -> (Vec<Entry>, BenchLeftOut, u32) {
    // A Claude transcript a later one resumes is that one's start: the
    // conversation is one trace, read from its fullest transcript.
    let resumes = claude_resumes(sessions);
    let mut ordered: Vec<&SessionInput> = sessions
        .iter()
        .zip(&resumes)
        .filter(|(_, resume)| !resume.continued)
        .map(|(session, _)| session)
        .collect();
    ordered.sort_by(|a, b| {
        a.session_ref
            .as_bytes()
            .cmp(b.session_ref.as_bytes())
            .then(a.import_seq.cmp(&b.import_seq))
    });
    let mut out = Vec::new();
    let mut left_out = BenchLeftOut::default();
    let mut counted = 0u32;
    for session in ordered {
        let Some(at) = latest_at(session) else {
            continue;
        };
        if at < window_start || at > now {
            continue;
        }
        let session_tool = tool_of(&session.body);
        if tool.is_some() && session_tool != tool {
            continue;
        }
        bump(&mut counted);
        if session.automated {
            bump(&mut left_out.automated);
            continue;
        }
        let Some(facts) = &session.facts else {
            bump(&mut left_out.no_facts);
            continue;
        };
        let (accounting, model_labels, turns) = match &session.body {
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
            SessionBody::Codex { series: None, .. } | SessionBody::Unknown { .. } => {
                bump(&mut left_out.no_model);
                continue;
            }
        };
        let Some(session_tool) = session_tool else {
            bump(&mut left_out.no_model);
            continue;
        };
        let mut facts = facts.clone();
        reclassify(&mut facts);
        if facts.reviewer {
            bump(&mut left_out.reviewer);
            continue;
        }
        let label = labels
            .get(&session.session_ref)
            .and_then(|label| label.task);
        let category = bench_category(label, &facts);
        if category == BenchCategory::Understand
            && facts.models.iter().any(|model| model.edits.is_none())
        {
            bump(&mut left_out.unknown_task);
            continue;
        }
        let price = price_session(catalog, accounting, model_labels, turns);
        if price.by_model.is_empty() {
            bump(&mut left_out.no_model);
            continue;
        }
        let model_turns =
            |model: &ModelPrice| u64::from(model.priced_turns) + u64::from(model.unpriced_turns);
        let all_turns = price.by_model.values().map(model_turns).sum::<u64>()
            + u64::from(price.unlabelled_turns);
        let Some(model) = price
            .by_model
            .iter()
            .find(|(_, model)| model_turns(model) * 2 > all_turns)
            .map(|(label, _)| label.clone())
        else {
            bump(&mut left_out.mixed_models);
            continue;
        };
        let route = routes
            .get(&session.session_ref)
            .copied()
            .unwrap_or(ModelRoute::Unobserved);
        if !route.compared() {
            bump(&mut left_out.routed);
            continue;
        }
        if facts.capped {
            bump(&mut left_out.capped);
            continue;
        }
        // A session that delegated is counted on its own work: its own
        // turns price it, its own edits and model place it. The sub-agents'
        // transcripts are not in its facts, so it is marked partial
        // (`delegated`), never left out.
        let own = own_counts(&facts.models, model_labels, &model);
        let priced = route.priced_at_list() && price.total_usd_micros.is_some();
        out.push(Entry {
            category,
            tool: session_tool,
            model,
            edits: own.edits,
            undone: own.undone,
            denied: own.denied,
            cost_usd_micros: price.total_usd_micros.filter(|_| priced),
            tables: if priced { price.tables } else { Vec::new() },
            active_secs: facts.active_secs,
            test_first: facts.shape.failed_before_first_edit_then_passed > 0,
            delegated: facts.delegated_unread,
        });
    }
    (out, left_out, counted)
}

/// A model's figures and the sums behind them.
struct ModelSums {
    row: BenchModel,
    edits: u64,
    undone: u64,
}

fn model_figures(label: &str, tool: BenchTool, entries: &[&Entry]) -> ModelSums {
    let (mut edits, mut kept_losses, mut undone, mut kept_sessions) = (0u64, 0u64, 0u64, 0u32);
    let mut costs = Vec::new();
    let mut secs = Vec::new();
    for entry in entries {
        if let (Some(e), Some(u)) = (entry.edits, entry.undone) {
            bump(&mut kept_sessions);
            let lost = u64::from(u) + u64::from(entry.denied.unwrap_or(0));
            edits += u64::from(e);
            kept_losses += lost.min(u64::from(e));
            undone += u64::from(u);
        }
        if let Some(cost) = entry.cost_usd_micros {
            costs.push(cost);
        }
        if let Some(active) = entry.active_secs {
            secs.push(u64::from(active));
        }
    }
    let kept_permille = if kept_sessions == 0 {
        None
    } else {
        permille(edits - kept_losses, edits)
    };
    ModelSums {
        row: BenchModel {
            label: label.to_owned(),
            tool,
            sessions: len_u32(entries.len()),
            kept_permille,
            usd_micros_median: median(&mut costs),
            active_secs_median: median(&mut secs),
            kept_sessions,
            priced_sessions: len_u32(costs.len()),
            timed_sessions: len_u32(secs.len()),
        },
        edits,
        undone,
    }
}

/// The two leading qualifying models on one figure; `None` with fewer than
/// two. `higher_is_better` for kept.
fn pick(
    models: &[ModelSums],
    min_sessions: u32,
    figure: impl Fn(&BenchModel) -> (Option<u64>, u32),
    higher_is_better: bool,
) -> Option<Pick> {
    let mut qualifying: Vec<(&BenchModel, u64)> = models
        .iter()
        .filter_map(|model| match figure(&model.row) {
            (Some(value), sample) if sample >= min_sessions => Some((&model.row, value)),
            _ => None,
        })
        .collect();
    qualifying.sort_by(|(a, x), (b, y)| {
        let by_figure = if higher_is_better { y.cmp(x) } else { x.cmp(y) };
        by_figure
            .then(b.sessions.cmp(&a.sessions))
            .then(a.label.as_bytes().cmp(b.label.as_bytes()))
            .then(a.tool.cmp(&b.tool))
    });
    let entry = |(model, value): (&BenchModel, u64)| PickEntry {
        label: model.label.clone(),
        tool: model.tool,
        figure: value,
    };
    let mut ranked = qualifying.into_iter();
    let leader = ranked.next()?;
    let next = ranked.next()?;
    Some(Pick {
        leader: entry(leader),
        next: entry(next),
    })
}

fn quality_figure(model: &BenchModel) -> (Option<u64>, u32) {
    (model.kept_permille.map(u64::from), model.kept_sessions)
}

fn value_figure(model: &BenchModel) -> (Option<u64>, u32) {
    (model.usd_micros_median, model.priced_sessions)
}

fn speed_figure(model: &BenchModel) -> (Option<u64>, u32) {
    (model.active_secs_median, model.timed_sessions)
}

/// `bug_fix`: test-first sessions against the rest, by median active time.
fn test_first_tip(entries: &[&Entry]) -> Option<Tip> {
    let mut with = Vec::new();
    let mut without = Vec::new();
    for entry in entries {
        let Some(active) = entry.active_secs else {
            continue;
        };
        if entry.test_first {
            with.push(u64::from(active));
        } else {
            without.push(u64::from(active));
        }
    }
    if with.len() < TEST_FIRST_MIN_GROUP || without.len() < TEST_FIRST_MIN_GROUP {
        return None;
    }
    let (sessions_with, sessions_without) = (len_u32(with.len()), len_u32(without.len()));
    let with_median = median(&mut with)?;
    let without_median = median(&mut without)?;
    if with_median >= without_median {
        return None;
    }
    Some(Tip::TestFirst {
        faster_permille: permille(without_median - with_median, without_median)?,
        sessions_without,
        sessions: sessions_with.saturating_add(sessions_without),
        sessions_with,
    })
}

/// A model near the quality leader's kept rate at no more than half its
/// median cost; the cheapest such model.
fn cheaper_same_quality_tip(models: &[ModelSums], quality: Option<&Pick>, min: u32) -> Option<Tip> {
    let quality = quality?;
    let leader = models
        .iter()
        .map(|model| &model.row)
        .find(|model| model.label == quality.leader.label && model.tool == quality.leader.tool)?;
    let leader_kept = leader.kept_permille?;
    let leader_cost = leader
        .usd_micros_median
        .filter(|_| leader.priced_sessions >= min)?;
    if leader_cost == 0 {
        return None;
    }
    models
        .iter()
        .map(|model| &model.row)
        .filter(|model| !(model.label == leader.label && model.tool == leader.tool))
        .filter(|model| model.kept_sessions >= min && model.priced_sessions >= min)
        .filter_map(|model| Some((model, model.kept_permille?, model.usd_micros_median?)))
        .filter(|(_, kept, cost)| {
            kept.saturating_add(SAME_QUALITY_PERMILLE) >= leader_kept
                && u128::from(*cost) * 1_000
                    <= u128::from(leader_cost) * u128::from(CHEAPER_COST_RATIO_PERMILLE)
        })
        .min_by(|(a, _, x), (b, _, y)| x.cmp(y).then(a.label.as_bytes().cmp(b.label.as_bytes())))
        .and_then(|(model, _, cost)| {
            Some(Tip::CheaperSameQuality {
                cheap: model.label.clone(),
                expensive: leader.label.clone(),
                cost_ratio_permille: permille(cost, leader_cost)?,
            })
        })
}

/// Undone rates of the qualifying models with enough edits: the lowest
/// nonzero against the highest, when the highest is at least twice it.
fn fewer_undos_tip(models: &[ModelSums], min: u32) -> Option<Tip> {
    let rates: Vec<(&ModelSums, u64, u64)> = models
        .iter()
        .filter(|model| {
            model.row.kept_sessions >= min && model.edits >= u64::from(MODEL_ANSWER_MIN_EDITS)
        })
        .map(|model| (model, model.undone, model.edits))
        .collect();
    // Compare a/b against c/d by cross-multiplying.
    let lower = |(_, u1, e1): &&(&ModelSums, u64, u64), (_, u2, e2): &&(&ModelSums, u64, u64)| {
        (u128::from(*u1) * u128::from(*e2)).cmp(&(u128::from(*u2) * u128::from(*e1)))
    };
    let better = rates
        .iter()
        .filter(|(_, undone, _)| *undone > 0)
        .min_by(lower)?;
    let worse = rates.iter().max_by(lower)?;
    if std::ptr::eq(better.0, worse.0) {
        return None;
    }
    // (worse undone / worse edits) / (better undone / better edits).
    let num = u128::from(worse.1) * u128::from(better.2);
    let den = u128::from(better.1) * u128::from(worse.2);
    let ratio = u32::try_from((num * 2_000 + den) / (den * 2)).ok()?;
    if ratio < FEWER_UNDOS_RATIO_PERMILLE {
        return None;
    }
    Some(Tip::FewerUndos {
        better: better.0.row.label.clone(),
        worse: worse.0.row.label.clone(),
        ratio_permille: ratio,
    })
}

fn category_row(
    category: BenchCategory,
    entries: &[&Entry],
    min_sessions: u32,
) -> BenchCategoryRow {
    let mut by_model: BTreeMap<(BenchTool, &str), Vec<&Entry>> = BTreeMap::new();
    for entry in entries {
        by_model
            .entry((entry.tool, entry.model.as_str()))
            .or_default()
            .push(entry);
    }
    let mut models: Vec<ModelSums> = by_model
        .iter()
        .map(|((tool, label), of_model)| model_figures(label, *tool, of_model))
        .collect();
    models.sort_by(|a, b| {
        b.row
            .sessions
            .cmp(&a.row.sessions)
            .then(a.row.label.as_bytes().cmp(b.row.label.as_bytes()))
            .then(a.row.tool.cmp(&b.row.tool))
    });
    let quality = pick(&models, min_sessions, quality_figure, true);
    let value = pick(&models, min_sessions, value_figure, false);
    let speed = pick(&models, min_sessions, speed_figure, false);
    let tip = match category {
        BenchCategory::BugFix => test_first_tip(entries),
        _ => None,
    }
    .or_else(|| cheaper_same_quality_tip(&models, quality.as_ref(), min_sessions))
    .or_else(|| fewer_undos_tip(&models, min_sessions));
    BenchCategoryRow {
        key: category,
        sessions: len_u32(entries.len()),
        models: models.into_iter().map(|model| model.row).collect(),
        quality,
        value,
        speed,
        tip,
    }
}

/// The benchmarks over `sessions` in the `weeks` ending `now`, for one tool
/// or both. `routes` and `labels` are by `session_ref`; a session absent
/// from `routes` is unobserved, from `labels` unlabelled.
pub fn benchmarks(
    sessions: &[SessionInput],
    routes: &BTreeMap<String, ModelRoute>,
    labels: &BTreeMap<String, SessionLabel>,
    catalog: &PricingCatalog,
    weeks: u32,
    tool: Option<BenchTool>,
    now: DateTime<Utc>,
) -> Benchmarks {
    let window_start = now - Duration::weeks(i64::from(weeks));
    let (entries, left_out, counted) =
        entries(sessions, routes, labels, catalog, tool, window_start, now);
    let mut tables = BTreeSet::new();
    for entry in &entries {
        tables.extend(entry.tables.iter().cloned());
    }
    let categories = BenchCategory::ALL
        .into_iter()
        .map(|category| {
            let of: Vec<&Entry> = entries
                .iter()
                .filter(|entry| entry.category == category)
                .collect();
            category_row(category, &of, BENCHMARK_MIN_SESSIONS)
        })
        .collect();
    Benchmarks {
        window_start,
        window_end: now,
        weeks,
        sessions: counted,
        min_sessions: BENCHMARK_MIN_SESSIONS,
        categories,
        left_out,
        delegated: len_u32(entries.iter().filter(|entry| entry.delegated).count()),
        pricing_tables: tables.into_iter().collect(),
    }
}

#[cfg(test)]
#[path = "benchmarks_tests.rs"]
mod tests;
