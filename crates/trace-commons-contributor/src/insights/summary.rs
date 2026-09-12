//! Deterministic observations over currently saved, explicitly selected sessions.
//! Reads only the validated derived store, never source files. These counts are
//! neither completed tasks nor a representative sample of a model's work.
use std::path::Path;

use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights::{Coverage, EvidenceRef, MetricId, ProviderManifest};

use super::{LocalInsight, SourceFormat, TaskCategory, TaskOutcome, service::list_saved};

#[derive(Debug, Serialize, Deserialize)]
pub struct SavedInsightsSummary {
    pub schema_version: u32,
    pub scope: SummaryScope,
    pub limitations: Vec<SummaryLimitation>,
    pub provider: ProviderManifest,
    pub saved_snapshots: u64,
    /// Times of analysis, never the range of human or model activity.
    pub snapshot_analysis_range: Option<SnapshotAnalysisRange>,
    pub user_reported: UserReportedSummary,
    pub metrics: Vec<MetricSummary>,
    pub snapshots: Vec<SnapshotEvidence>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryScope {
    AllSavedSelectedSessionSnapshots,
}

/// Machine-readable caveats travel with the result, including over native FFI.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SummaryLimitation {
    SelectedSavedSessionsAreNotVerifiedTasks,
    AssessmentsAreUserReported,
    ObservedSumsRequireBothCoverages,
    AnalysisDatesAreNotActivityTime,
    SourceFormatsAreNotModelIdentity,
    NoModelRankingsTimeSavingsOrCost,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageUnit {
    SessionSnapshots,
    NormalizedEvents,
    ToolResults,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotAnalysisRange {
    pub oldest: DateTime<Utc>,
    pub newest: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SnapshotEvidence {
    pub id: String,
    pub source_format: SourceFormat,
    pub analyzed_at: DateTime<Utc>,
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UserReportedSummary {
    pub assessed_snapshots: u64,
    /// No annotation at all; not an explicitly chosen Unknown category/outcome.
    pub unassessed_snapshots: u64,
    pub categories: Vec<CategoryCount>,
    pub outcomes: Vec<OutcomeCount>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct CategoryCount {
    pub category: TaskCategory,
    pub snapshots: u64,
    pub evidence_snapshot_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct OutcomeCount {
    pub outcome: TaskOutcome,
    pub snapshots: u64,
    pub evidence_snapshot_ids: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct MetricSummary {
    pub id: MetricId,
    /// Sum only available values; None for an empty or entirely missing cohort.
    /// A known zero remains Some(0). Always interpret alongside BOTH coverages.
    pub observed_value_sum: Option<u64>,
    pub available_snapshots: u64,
    pub missing_snapshots: u64,
    /// Original evidence denominator, distinct from snapshot availability. A
    /// present value may still have only partial recognized-record coverage.
    pub record_coverage: Coverage,
    pub coverage_unit: CoverageUnit,
    /// Snapshots contributing a known value, not all snapshots in the cohort.
    pub evidence_snapshot_ids: Vec<String>,
}

pub fn read_saved(store_dir: Option<&Path>) -> Result<SavedInsightsSummary> {
    summarize(list_saved(store_dir)?)
}

fn count(len: usize) -> Result<u64> {
    u64::try_from(len).map_err(|_| anyhow!("insights-summary-overflow"))
}

fn add(a: u64, b: u64) -> Result<u64> {
    a.checked_add(b)
        .ok_or_else(|| anyhow!("insights-summary-overflow"))
}

/// Input is the store's validated, alias-deduplicated set. Keep private so raw
/// unvalidated caller reports cannot be aggregated through another entry point.
fn summarize(mut snapshots: Vec<LocalInsight>) -> Result<SavedInsightsSummary> {
    snapshots.sort_by(|a, b| a.id.cmp(&b.id));
    let total = count(snapshots.len())?;
    let annotations: Vec<_> = snapshots
        .iter()
        .filter_map(|s| s.manual_annotation.as_ref().map(|a| (&s.id, a)))
        .collect();
    let assessed = count(annotations.len())?;
    let categories = [
        TaskCategory::Refactor,
        TaskCategory::Tests,
        TaskCategory::Docs,
        TaskCategory::Debugging,
        TaskCategory::Other,
        TaskCategory::Unknown,
    ]
    .into_iter()
    .map(|category| {
        let ids: Vec<_> = annotations
            .iter()
            .filter(|(_, a)| a.category == category)
            .map(|(id, _)| (*id).clone())
            .collect();
        Ok(CategoryCount {
            category,
            snapshots: count(ids.len())?,
            evidence_snapshot_ids: ids,
        })
    })
    .collect::<Result<_>>()?;
    let outcomes = [
        TaskOutcome::Accepted,
        TaskOutcome::Partial,
        TaskOutcome::Rejected,
        TaskOutcome::Unknown,
    ]
    .into_iter()
    .map(|outcome| {
        let ids: Vec<_> = annotations
            .iter()
            .filter(|(_, a)| a.outcome == outcome)
            .map(|(id, _)| (*id).clone())
            .collect();
        Ok(OutcomeCount {
            outcome,
            snapshots: count(ids.len())?,
            evidence_snapshot_ids: ids,
        })
    })
    .collect::<Result<_>>()?;
    let metrics = [
        MetricId::Sessions,
        MetricId::Events,
        MetricId::InputTokens,
        MetricId::OutputTokens,
        MetricId::ToolCalls,
        MetricId::ToolFailures,
        MetricId::KnownOutcomes,
    ]
    .into_iter()
    .map(|id| {
        let mut summary = MetricSummary {
            id,
            observed_value_sum: None,
            available_snapshots: 0,
            missing_snapshots: 0,
            coverage_unit: match id {
                MetricId::Events | MetricId::ToolCalls => CoverageUnit::NormalizedEvents,
                MetricId::ToolFailures => CoverageUnit::ToolResults,
                _ => CoverageUnit::SessionSnapshots,
            },
            record_coverage: Coverage {
                observed: 0,
                total: 0,
            },
            evidence_snapshot_ids: Vec::new(),
        };
        for snapshot in &snapshots {
            let metric = snapshot
                .report
                .metrics
                .iter()
                .find(|m| m.id == id)
                .ok_or_else(|| anyhow!("insights-summary-metric-missing"))?;
            summary.record_coverage.observed =
                add(summary.record_coverage.observed, metric.coverage.observed)?;
            summary.record_coverage.total =
                add(summary.record_coverage.total, metric.coverage.total)?;
            if let Some(value) = metric.value {
                summary.observed_value_sum =
                    Some(add(summary.observed_value_sum.unwrap_or(0), value)?);
                summary.available_snapshots = add(summary.available_snapshots, 1)?;
                summary.evidence_snapshot_ids.push(snapshot.id.clone());
            } else {
                summary.missing_snapshots = add(summary.missing_snapshots, 1)?;
            }
        }
        Ok(summary)
    })
    .collect::<Result<_>>()?;
    let snapshot_analysis_range =
        snapshots
            .iter()
            .map(|s| s.analyzed_at)
            .min()
            .map(|oldest| SnapshotAnalysisRange {
                oldest,
                newest: snapshots
                    .iter()
                    .map(|s| s.analyzed_at)
                    .max()
                    .unwrap_or(oldest),
            });
    Ok(SavedInsightsSummary {
        schema_version: 1,
        scope: SummaryScope::AllSavedSelectedSessionSnapshots,
        limitations: vec![
            SummaryLimitation::SelectedSavedSessionsAreNotVerifiedTasks,
            SummaryLimitation::AssessmentsAreUserReported,
            SummaryLimitation::ObservedSumsRequireBothCoverages,
            SummaryLimitation::AnalysisDatesAreNotActivityTime,
            SummaryLimitation::SourceFormatsAreNotModelIdentity,
            SummaryLimitation::NoModelRankingsTimeSavingsOrCost,
        ],
        provider: ProviderManifest::first_party(),
        saved_snapshots: total,
        snapshot_analysis_range,
        user_reported: UserReportedSummary {
            assessed_snapshots: assessed,
            unassessed_snapshots: total - assessed,
            categories,
            outcomes,
        },
        metrics,
        snapshots: snapshots
            .into_iter()
            .map(|s| SnapshotEvidence {
                id: s.id,
                source_format: s.source_format,
                analyzed_at: s.analyzed_at,
                evidence: s.report.evidence,
            })
            .collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::{LocalInsightStore, analyze_file};
    use super::*;

    fn fixture(path: &Path, suffix: &str) {
        std::fs::write(path, format!(
            "{{\"role\":\"meta\",\"source\":\"fixture\"}}\n{{\"role\":\"user\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"content\":\"PRIVATE-{suffix}\"}}\n"
        )).unwrap();
    }

    fn metric(summary: &SavedInsightsSummary, id: MetricId) -> &MetricSummary {
        summary.metrics.iter().find(|m| m.id == id).unwrap()
    }

    #[test]
    fn empty_summary_does_not_initialize_state_or_claim_zero_unknowns() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("absent");
        let summary = read_saved(Some(&path)).unwrap();
        assert!(!path.exists());
        assert_eq!(summary.saved_snapshots, 0);
        assert!(summary.snapshot_analysis_range.is_none());
        assert!(
            summary
                .metrics
                .iter()
                .all(|m| m.observed_value_sum.is_none()
                    && m.available_snapshots == 0
                    && m.missing_snapshots == 0)
        );
        assert_eq!(summary.user_reported.unassessed_snapshots, 0);
    }

    #[test]
    fn history_reflects_alias_dedup_annotation_replacement_and_deletion_without_source_reads() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().canonicalize().unwrap();
        let source = root.join("source.jsonl");
        let alias = root.join("alias.jsonl");
        let path = root.join("insights");
        fixture(&source, "one");
        std::fs::copy(&source, &alias).unwrap();
        let store = LocalInsightStore::open(&path).unwrap();
        let first = store.import(SourceFormat::Trajectory, &source).unwrap();
        store.import(SourceFormat::Trajectory, &alias).unwrap();
        let unassessed = read_saved(Some(&path)).unwrap();
        assert_eq!(unassessed.saved_snapshots, 1);
        assert_eq!(unassessed.user_reported.unassessed_snapshots, 1);
        assert!(
            unassessed
                .user_reported
                .outcomes
                .iter()
                .all(|c| c.snapshots == 0)
        );
        store
            .annotate(&first.id, TaskCategory::Unknown, TaskOutcome::Unknown)
            .unwrap();
        let unknown = read_saved(Some(&path)).unwrap();
        assert_eq!(unknown.user_reported.unassessed_snapshots, 0);
        assert_eq!(
            unknown
                .user_reported
                .outcomes
                .iter()
                .find(|c| c.outcome == TaskOutcome::Unknown)
                .unwrap()
                .snapshots,
            1
        );
        store
            .annotate(&first.id, TaskCategory::Docs, TaskOutcome::Rejected)
            .unwrap();
        let rejected = read_saved(Some(&path)).unwrap();
        assert_eq!(
            rejected
                .user_reported
                .categories
                .iter()
                .find(|c| c.category == TaskCategory::Docs)
                .unwrap()
                .evidence_snapshot_ids,
            std::slice::from_ref(&first.id)
        );
        assert_eq!(
            rejected
                .user_reported
                .outcomes
                .iter()
                .find(|c| c.outcome == TaskOutcome::Rejected)
                .unwrap()
                .snapshots,
            1
        );
        assert!(
            metric(&rejected, MetricId::KnownOutcomes)
                .observed_value_sum
                .is_none()
        );
        fixture(&source, "two");
        let replacement = store.import(SourceFormat::Trajectory, &source).unwrap();
        let two = read_saved(Some(&path)).unwrap();
        assert_eq!(two.saved_snapshots, 2, "alias retains the earlier snapshot");
        assert_eq!(two.user_reported.assessed_snapshots, 1);
        assert_eq!(two.user_reported.unassessed_snapshots, 1);
        store.import(SourceFormat::Trajectory, &alias).unwrap();
        store.delete(&first.id).unwrap();
        store
            .annotate(&replacement.id, TaskCategory::Tests, TaskOutcome::Accepted)
            .unwrap();
        store.clear_annotation(&replacement.id).unwrap();
        std::fs::remove_file(&source).unwrap();
        std::fs::remove_file(&alias).unwrap();
        let final_summary = read_saved(Some(&path)).unwrap();
        assert_eq!(final_summary.saved_snapshots, 1);
        assert_eq!(final_summary.user_reported.unassessed_snapshots, 1);
        assert_eq!(final_summary.snapshots[0].id, replacement.id);
        assert!(
            !serde_json::to_string(&final_summary)
                .unwrap()
                .contains("PRIVATE")
        );
        assert_eq!(
            serde_json::to_string(&final_summary).unwrap(),
            serde_json::to_string(&read_saved(Some(&path)).unwrap()).unwrap()
        );
        store.delete(&replacement.id).unwrap();
        assert_eq!(read_saved(Some(&path)).unwrap().saved_snapshots, 0);
    }

    #[test]
    fn snapshot_availability_does_not_erase_partial_record_coverage_or_known_zero() {
        let temp = tempfile::tempdir().unwrap();
        let file = temp.path().join("fixture.jsonl");
        fixture(&file, "first");
        let mut first = analyze_file(SourceFormat::Trajectory, &file).unwrap();
        let mut second = first.clone();
        second.id = "second".into();
        let modify = |snapshot: &mut LocalInsight, value, observed, total| {
            let metric = snapshot
                .report
                .metrics
                .iter_mut()
                .find(|m| m.id == MetricId::ToolCalls)
                .unwrap();
            metric.value = value;
            metric.coverage = Coverage { observed, total };
        };
        modify(&mut first, Some(0), 1, 3);
        modify(&mut second, None, 0, 2);
        let summary = summarize(vec![second.clone(), first.clone()]).unwrap();
        let calls = metric(&summary, MetricId::ToolCalls);
        assert_eq!(calls.observed_value_sum, Some(0));
        assert_eq!((calls.available_snapshots, calls.missing_snapshots), (1, 1));
        assert_eq!(
            calls.record_coverage,
            Coverage {
                observed: 1,
                total: 5
            }
        );
        assert_eq!(calls.evidence_snapshot_ids, std::slice::from_ref(&first.id));
        let tokens = metric(&summary, MetricId::InputTokens);
        assert_eq!(tokens.observed_value_sum, None);
        assert_eq!(
            (tokens.available_snapshots, tokens.missing_snapshots),
            (0, 2)
        );
        modify(&mut first, Some(u64::MAX), 1, 1);
        modify(&mut second, Some(1), 1, 1);
        assert_eq!(
            summarize(vec![first.clone(), second.clone()])
                .unwrap_err()
                .to_string(),
            "insights-summary-overflow"
        );
        modify(&mut first, Some(0), 0, u64::MAX);
        modify(&mut second, None, 0, 1);
        assert_eq!(
            summarize(vec![first, second]).unwrap_err().to_string(),
            "insights-summary-overflow"
        );
    }
}
