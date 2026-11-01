//! The counter store's half of `insights_submitted`
//! (`daemon::insights_submitted`): each submitted session's stored facts,
//! looked up by the address the upload queue recorded for it. A child of
//! `insights_week` so it reads the store as `benchmarks_value` does,
//! without widening the store's types.
//!
//! A row is found by the keyed session digest the pass stores it under
//! (`session_key` of the harness and the session file's address), the same
//! lookup `locate` makes the other way. Nothing is read but the store; no
//! path is kept, and nothing here crosses the socket: the caller turns the
//! summaries into counts and fixed labels.

use std::collections::BTreeMap;
use std::path::Path;

use trace_commons_protocol::insights_pricing::PricingAccounting;
use trace_commons_protocol::insights_usage_series::session_key;

use super::{CounterPass, CounterRow, NeverFolders, RowBody, never_digests, route_map, row_tool};
use crate::insights::benchmarks::{BenchCategory, bench_category};
use crate::insights::model_answers::ModelRoute;
use crate::insights::pricing_catalog::PricingCatalog;
use crate::insights::session_facts::{ExtensionClassCounts, ModelFacts, SessionFacts};
use crate::insights::session_labels::LabelledTask;
use crate::insights::session_pricing::{ModelPrice, PricingTableRef, price_session};
use crate::insights::task_classifier::reclassify;

/// One submitted session as the store holds it: fixed labels and counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StoredSession {
    /// `claude_code` or `codex`.
    pub tool: &'static str,
    /// The normalized label serving a strict majority of its counted turns,
    /// as Benchmarks picks it; `None` when no label has a majority.
    pub model: Option<String>,
    /// No majority because the turns are split between models (`true`),
    /// rather than because no turn names one.
    pub mixed_models: bool,
    /// The kind of work, as Benchmarks names it: the person's own task label
    /// when set, else the inferred task and shape. `None` with no facts.
    pub task: Option<BenchCategory>,
    /// Distinct edited files per extension class; `None` with no facts.
    pub files: Option<ExtensionClassCounts>,
    /// Edit calls, undone and declined, over every model; each `None` when
    /// any model's count is unknown, or there are no facts.
    pub edits: Option<u32>,
    pub undone: Option<u32>,
    pub denied: Option<u32>,
    pub lines_added: Option<u32>,
    pub lines_removed: Option<u32>,
    /// The session at API list prices; `None` when any counted turn could
    /// not be priced, there is no per-turn series, or the session went
    /// through NEAR AI (`near_ai`).
    pub usd_micros: Option<u64>,
    /// A route tally says some of it went through NEAR AI: its list price
    /// is not what it cost (owner decision D5), so it is not list-priced.
    pub near_ai: bool,
    /// The tables `usd_micros` was priced with.
    pub tables: Vec<PricingTableRef>,
}

/// What the store says about one submitted session's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum StoredLookup {
    Found(Box<StoredSession>),
    /// No row under that address: never read (an adapter the pass does not
    /// read, a file gone before a pass reached it), or aged out or evicted.
    NoRow,
    /// The row is in a Never folder: it enters no answer.
    NeverFolder,
}

/// One submitted session's address, as the upload queue recorded it.
pub(crate) struct StoredAddress<'a> {
    /// The adapter name (`claude-code`, `codex`).
    pub source: &'a str,
    pub path: &'a Path,
}

impl CounterPass {
    /// When a pass last finished, RFC 3339, for the answer's `updated_at`.
    pub(crate) fn submitted_updated_at(&self) -> Option<String> {
        self.updated_at()
    }

    /// Each address's stored session, in order. `Err` is the fixed reason
    /// the store, or its key, could not be read; with no key or no store yet
    /// every address is `NoRow`.
    pub(crate) fn submitted_lookups(
        &self,
        addresses: &[StoredAddress<'_>],
        never_project_keys: &NeverFolders,
        routing_available: bool,
    ) -> Result<Vec<StoredLookup>, &'static str> {
        let (key, store) = self.visible_store()?;
        let (Some(key), Some(store)) = (key, store) else {
            return Ok(addresses.iter().map(|_| StoredLookup::NoRow).collect());
        };
        let never = never_digests(&key, never_project_keys);
        let user_labels = self.readable_labels(&store.key_fingerprint);
        let catalog = crate::insights::pricing_catalog::shared_pricing_catalog();
        Ok(addresses
            .iter()
            .map(|address| {
                let digest = session_key(&key, address.source, &address.path.to_string_lossy());
                let Some(row) = store.rows.get(&digest) else {
                    return StoredLookup::NoRow;
                };
                if never.contains(&row.project) {
                    return StoredLookup::NeverFolder;
                }
                let route = route_map(&[row], &store.routing, routing_available)
                    .into_values()
                    .next()
                    .unwrap_or(ModelRoute::Unobserved);
                let label = user_labels.get(&digest).and_then(|label| label.task);
                StoredLookup::Found(Box::new(stored_session(row, route, label, catalog)))
            })
            .collect())
    }
}

/// One count summed over every model; `None` when any model's is unknown.
fn summed(facts: &SessionFacts, count: impl Fn(&ModelFacts) -> Option<u32>) -> Option<u32> {
    facts.models.iter().try_fold(0u32, |sum, model| {
        count(model).map(|n| sum.saturating_add(n))
    })
}

fn stored_session(
    row: &CounterRow,
    route: ModelRoute,
    label: Option<LabelledTask>,
    catalog: &PricingCatalog,
) -> StoredSession {
    let facts = row.facts.clone().map(|mut facts| {
        reclassify(&mut facts);
        facts
    });
    let priced = match &row.body {
        RowBody::Claude {
            model_labels,
            series,
        } => Some(price_session(
            catalog,
            PricingAccounting::ClaudeCode,
            model_labels,
            &series.turns,
        )),
        RowBody::Codex {
            series: Some(evidence),
            ..
        } => Some(price_session(
            catalog,
            PricingAccounting::Codex,
            &evidence.model_labels,
            &evidence.series.turns,
        )),
        _ => None,
    };
    let model_turns =
        |model: &ModelPrice| u64::from(model.priced_turns) + u64::from(model.unpriced_turns);
    let (model, mixed_models) = match &priced {
        Some(price) => {
            let all_turns = price.by_model.values().map(model_turns).sum::<u64>()
                + u64::from(price.unlabelled_turns);
            let majority = price
                .by_model
                .iter()
                .find(|(_, model)| model_turns(model) * 2 > all_turns)
                .map(|(label, _)| label.clone());
            let mixed = majority.is_none() && price.by_model.len() > 1;
            (majority, mixed)
        }
        None => (None, false),
    };
    let near_ai = route != ModelRoute::Unobserved;
    let (usd_micros, tables) = match priced {
        Some(price) if !near_ai && price.total_usd_micros.is_some() => {
            (price.total_usd_micros, price.tables)
        }
        _ => (None, Vec::new()),
    };
    StoredSession {
        tool: row_tool(row),
        model,
        mixed_models,
        task: facts.as_ref().map(|facts| bench_category(label, facts)),
        files: facts.as_ref().map(|facts| facts.shape.edited_files),
        edits: facts.as_ref().and_then(|facts| summed(facts, |m| m.edits)),
        undone: facts.as_ref().and_then(|facts| summed(facts, |m| m.undone)),
        denied: facts.as_ref().and_then(|facts| summed(facts, |m| m.denied)),
        lines_added: facts.as_ref().and_then(|facts| facts.shape.lines_added),
        lines_removed: facts.as_ref().and_then(|facts| facts.shape.lines_removed),
        usd_micros,
        near_ai,
        tables,
    }
}

/// The tables the found sessions were priced with, distinct and sorted,
/// for the answer's `pricing.tables`.
pub(crate) fn tables_of<'a>(
    sessions: impl Iterator<Item = &'a StoredSession>,
) -> Vec<&'a PricingTableRef> {
    let mut by_id: BTreeMap<(&str, u32), &PricingTableRef> = BTreeMap::new();
    for session in sessions {
        for table in &session.tables {
            by_id.insert((table.table_id.as_str(), table.version), table);
        }
    }
    by_id.into_values().collect()
}
