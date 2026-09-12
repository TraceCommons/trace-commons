//! Immutable retrospective comparison specifications and descriptive results.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::TaskCategory;
use super::comparison_tasks::{
    ComparisonTaskDetail, ComparisonTaskOutcome, ComparisonTaskStaleReason, ContextString,
    LocalComparisonTaskV1, MAX_CONTEXT_LABEL_BYTES,
};

const MAX_FACTS: usize = 256;
const MAX_LABEL_BYTES: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonSpecificationError {
    Invalid,
    TaskLimit,
    StoreInvalid,
    StoreFull,
    NotFound,
    ResultStale,
    NoSavedTasks,
}

impl std::fmt::Display for ComparisonSpecificationError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Invalid => "insights-comparison-specification-invalid",
            Self::TaskLimit => "insights-comparison-task-limit",
            Self::StoreInvalid => "insights-comparison-specification-store-invalid",
            Self::StoreFull => "insights-comparison-specification-store-full",
            Self::NotFound => "insights-comparison-specification-not-found",
            Self::ResultStale => "insights-comparison-result-stale",
            Self::NoSavedTasks => "insights-comparison-no-saved-tasks",
        })
    }
}

impl std::error::Error for ComparisonSpecificationError {}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SpecificationProvenance {
    RetrospectiveUserSpecification,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExactComparisonStratumV1 {
    pub project_id: String,
    pub language: String,
    pub configuration_fingerprint: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonSpecificationDraftInput {
    pub evidence_cutoff: DateTime<Utc>,
    pub cohort_labels: Vec<String>,
    pub date_start: NaiveDate,
    pub date_end: NaiveDate,
    pub stratum: ExactComparisonStratumV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonSpecificationV1 {
    pub schema_version: u32,
    pub id: String,
    pub provenance: SpecificationProvenance,
    pub created_at: DateTime<Utc>,
    pub evidence_cutoff: DateTime<Utc>,
    pub cutoff_task_evidence: Vec<CutoffTaskEvidenceV1>,
    pub category: String,
    pub cohort_labels: Vec<String>,
    pub date_start: NaiveDate,
    pub date_end: NaiveDate,
    pub stratum: ExactComparisonStratumV1,
    pub task_rule: String,
    pub attempt_rule: String,
    pub outcome_rubric_version: String,
    pub estimands: Vec<String>,
    pub estimator_state: EstimatorSpecificationState,
    pub specification_digest: String,
    pub saved_record_digest: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CutoffTaskEvidenceV1 {
    pub task_id: String,
    pub material_revision: u64,
    pub material_digest: String,
    pub substantive_material_digest: String,
    pub outcome: DescriptiveOutcome,
    pub outcome_recorded_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DescriptiveOutcome {
    Pending,
    Accepted,
    Partial,
    Rejected,
    Unknown,
    Unassessed,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum QualifiedSourceAttribution {
    Qualified {
        rule: QualifiedSourceRule,
        declared_cohort: String,
        bound_material_revision: u64,
        bound_material_digest: String,
    },
    Unavailable {
        reason: String,
    },
    WriterFixtureOnly {
        profile_id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum QualifiedSourceRule {
    #[cfg(test)]
    SyntheticReleasedRuleV1,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EstimatorSpecificationState {
    NotYetCalibrated,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "status")]
pub enum ObservedAttributedTokens {
    Available { tokens: u64 },
    Unavailable { reason: String },
}

/// Canonical task-level facts resolved by the task eligibility layer.
///
/// A source adapter must only emit `Qualified` after a released source rule
/// validates the bound material. Writer-fixture attribution is unavailable.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ComparisonTaskFactV1 {
    pub task_id: String,
    pub material_revision: u64,
    pub material_digest: String,
    pub substantive_material_digest: String,
    pub category: String,
    pub task_date: NaiveDate,
    pub material_recorded_at: Option<DateTime<Utc>>,
    pub outcome_recorded_at: Option<DateTime<Utc>>,
    pub stratum: Option<ExactComparisonStratumV1>,
    pub outcome: DescriptiveOutcome,
    pub evidence_current: bool,
    pub independence_current: bool,
    pub overlaps_other_task: bool,
    pub source_attribution: QualifiedSourceAttribution,
    pub observed_attributed_tokens: ObservedAttributedTokens,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "snake_case")]
pub enum ComparisonExclusionReason {
    EvidenceAfterCutoff,
    CutoffTimeUnavailable,
    CategoryMismatch,
    DateOutsideWindow,
    StratumMismatch,
    ContextUnavailable,
    CohortUnavailable,
    CohortNotSelected,
    EvidenceStale,
    IndependenceUnconfirmed,
    OverlappingTaskEvidence,
    SourceAttributionUnavailable,
    SourceAttributionStale,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ExcludedComparisonTask {
    pub task_id: String,
    pub reasons: Vec<ComparisonExclusionReason>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct OutcomeCounts {
    pub accepted: u64,
    pub partial: u64,
    pub rejected: u64,
    pub pending: u64,
    pub unknown: u64,
    pub unassessed: u64,
    /// Denominator for accepted/partial/rejected proportions only.
    pub assessed: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct UsageCoverage {
    pub tasks_with_observed_attributed_tokens: u64,
    pub tasks_without_observed_attributed_tokens: u64,
    pub observed_attributed_tokens: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CohortDescriptiveResult {
    pub cohort_label: String,
    pub included_tasks: u64,
    pub outcomes: OutcomeCounts,
    pub usage: UsageCoverage,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct DescriptiveComparisonResultV1 {
    pub schema_version: u32,
    pub specification_id: String,
    pub specification_digest: String,
    pub specification_record_digest: String,
    pub audit_digest: String,
    pub estimation_input_digest: String,
    pub included_task_ids: Vec<String>,
    pub excluded_tasks: Vec<ExcludedComparisonTask>,
    pub cohorts: Vec<CohortDescriptiveResult>,
}

impl DescriptiveComparisonResultV1 {
    pub fn validate(&self) -> Result<()> {
        let specification_id =
            uuid::Uuid::parse_str(&self.specification_id).map_err(|_| invalid())?;
        if self.schema_version != 1
            || specification_id.is_nil()
            || specification_id.to_string() != self.specification_id
            || !valid_digest(&self.specification_digest)
            || !valid_digest(&self.specification_record_digest)
            || !valid_digest(&self.audit_digest)
            || !valid_digest(&self.estimation_input_digest)
            || !self
                .included_task_ids
                .windows(2)
                .all(|pair| pair[0] < pair[1])
            || !self
                .excluded_tasks
                .windows(2)
                .all(|pair| pair[0].task_id < pair[1].task_id)
            || !self
                .cohorts
                .windows(2)
                .all(|pair| pair[0].cohort_label < pair[1].cohort_label)
        {
            return Err(invalid());
        }
        let included = self.included_task_ids.iter().collect::<BTreeSet<_>>();
        if included.len() != self.included_task_ids.len()
            || self.excluded_tasks.iter().any(|task| {
                uuid::Uuid::parse_str(&task.task_id)
                    .ok()
                    .is_none_or(|id| id.is_nil() || id.to_string() != task.task_id)
                    || included.contains(&task.task_id)
                    || task.reasons.is_empty()
                    || !task.reasons.windows(2).all(|pair| pair[0] < pair[1])
            })
        {
            return Err(invalid());
        }
        let mut total = 0u64;
        for row in &self.cohorts {
            if !valid_label(&row.cohort_label)
                || row.outcomes.assessed
                    != row
                        .outcomes
                        .accepted
                        .checked_add(row.outcomes.partial)
                        .and_then(|value| value.checked_add(row.outcomes.rejected))
                        .ok_or_else(invalid)?
                || row.included_tasks
                    != row
                        .outcomes
                        .assessed
                        .checked_add(row.outcomes.pending)
                        .and_then(|value| value.checked_add(row.outcomes.unknown))
                        .and_then(|value| value.checked_add(row.outcomes.unassessed))
                        .ok_or_else(invalid)?
                || row.included_tasks
                    != row
                        .usage
                        .tasks_with_observed_attributed_tokens
                        .checked_add(row.usage.tasks_without_observed_attributed_tokens)
                        .ok_or_else(invalid)?
            {
                return Err(invalid());
            }
            total = total.checked_add(row.included_tasks).ok_or_else(invalid)?;
        }
        if total != u64::try_from(self.included_task_ids.len()).map_err(|_| invalid())? {
            return Err(invalid());
        }
        Ok(())
    }
}

fn invalid() -> anyhow::Error {
    ComparisonSpecificationError::Invalid.into()
}

fn valid_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_LABEL_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/+-".contains(&byte))
}

fn valid_context_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_CONTEXT_LABEL_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._:/+-".contains(&byte))
}

fn digest(domain: &[u8], value: &impl Serialize) -> Result<String> {
    let bytes = serde_json::to_vec(value)?;
    let mut hash = Sha256::new();
    hash.update(domain);
    hash.update((bytes.len() as u64).to_be_bytes());
    hash.update(bytes);
    Ok(format!("{:x}", hash.finalize()))
}

#[derive(Serialize)]
struct SavedSpecificationDigestFields<'a> {
    schema_version: u32,
    id: &'a str,
    provenance: &'a SpecificationProvenance,
    created_at: DateTime<Utc>,
    evidence_cutoff: DateTime<Utc>,
    cutoff_task_evidence: &'a [CutoffTaskEvidenceV1],
    category: &'a str,
    cohort_labels: &'a [String],
    date_start: NaiveDate,
    date_end: NaiveDate,
    stratum: &'a ExactComparisonStratumV1,
    task_rule: &'a str,
    attempt_rule: &'a str,
    outcome_rubric_version: &'a str,
    estimands: &'a [String],
    estimator_state: &'a EstimatorSpecificationState,
    specification_digest: &'a str,
}

#[derive(Serialize)]
struct AnalyticalSpecificationDigestFields<'a> {
    category: &'a str,
    cohort_labels: &'a [String],
    date_start: NaiveDate,
    date_end: NaiveDate,
    stratum: &'a ExactComparisonStratumV1,
    task_rule: &'a str,
    attempt_rule: &'a str,
    outcome_rubric_version: &'a str,
    estimands: &'a [String],
    estimator_state: &'a EstimatorSpecificationState,
}

impl ExactComparisonStratumV1 {
    pub fn validate(&self) -> Result<()> {
        let id = uuid::Uuid::parse_str(&self.project_id).map_err(|_| invalid())?;
        if id.is_nil()
            || id.to_string() != self.project_id
            || !valid_context_label(&self.language)
            || !valid_digest(&self.configuration_fingerprint)
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl ComparisonSpecificationV1 {
    pub fn create(
        id: String,
        created_at: DateTime<Utc>,
        mut cutoff_task_evidence: Vec<CutoffTaskEvidenceV1>,
        input: ComparisonSpecificationDraftInput,
    ) -> Result<Self> {
        cutoff_task_evidence.sort_by(|left, right| left.task_id.cmp(&right.task_id));
        let mut value = Self {
            schema_version: 1,
            id,
            provenance: SpecificationProvenance::RetrospectiveUserSpecification,
            created_at,
            evidence_cutoff: input.evidence_cutoff,
            cutoff_task_evidence,
            category: "refactor".into(),
            cohort_labels: input.cohort_labels,
            date_start: input.date_start,
            date_end: input.date_end,
            stratum: input.stratum,
            task_rule: "one-user-confirmed-work-item-v1".into(),
            attempt_rule: "canonical-snapshot-union-v1".into(),
            outcome_rubric_version: "categorical-user-report-v1".into(),
            estimands: vec!["categorical-outcome-distribution-v1".into()],
            estimator_state: EstimatorSpecificationState::NotYetCalibrated,
            specification_digest: String::new(),
            saved_record_digest: String::new(),
        };
        value.validate_without_digest()?;
        value.specification_digest = digest(
            b"trace-commons-retrospective-comparison-specification-v1\0",
            &AnalyticalSpecificationDigestFields {
                category: &value.category,
                cohort_labels: &value.cohort_labels,
                date_start: value.date_start,
                date_end: value.date_end,
                stratum: &value.stratum,
                task_rule: &value.task_rule,
                attempt_rule: &value.attempt_rule,
                outcome_rubric_version: &value.outcome_rubric_version,
                estimands: &value.estimands,
                estimator_state: &value.estimator_state,
            },
        )?;
        value.saved_record_digest = digest(
            b"trace-commons-retrospective-comparison-saved-record-v1\0",
            &SavedSpecificationDigestFields {
                schema_version: value.schema_version,
                id: &value.id,
                provenance: &value.provenance,
                created_at: value.created_at,
                evidence_cutoff: value.evidence_cutoff,
                cutoff_task_evidence: &value.cutoff_task_evidence,
                category: &value.category,
                cohort_labels: &value.cohort_labels,
                date_start: value.date_start,
                date_end: value.date_end,
                stratum: &value.stratum,
                task_rule: &value.task_rule,
                attempt_rule: &value.attempt_rule,
                outcome_rubric_version: &value.outcome_rubric_version,
                estimands: &value.estimands,
                estimator_state: &value.estimator_state,
                specification_digest: &value.specification_digest,
            },
        )?;
        Ok(value)
    }

    fn validate_without_digest(&self) -> Result<()> {
        let id = uuid::Uuid::parse_str(&self.id).map_err(|_| invalid())?;
        if id.is_nil()
            || id.to_string() != self.id
            || self.schema_version != 1
            || self.evidence_cutoff > self.created_at
            || self.cutoff_task_evidence.len() > MAX_FACTS
            || !self
                .cutoff_task_evidence
                .windows(2)
                .all(|pair| pair[0].task_id < pair[1].task_id)
            || self.category != "refactor"
            || self.date_start > self.date_end
            || self.task_rule != "one-user-confirmed-work-item-v1"
            || self.attempt_rule != "canonical-snapshot-union-v1"
            || self.outcome_rubric_version != "categorical-user-report-v1"
            || self.estimands != ["categorical-outcome-distribution-v1"]
            || self.estimator_state != EstimatorSpecificationState::NotYetCalibrated
            || self.cohort_labels.len() != 2
            || !self.cohort_labels.windows(2).all(|pair| pair[0] < pair[1])
            || self.cohort_labels.iter().any(|label| !valid_label(label))
        {
            return Err(invalid());
        }
        for evidence in &self.cutoff_task_evidence {
            evidence.validate(self.evidence_cutoff)?;
        }
        self.stratum.validate()
    }

    pub fn validate(&self) -> Result<()> {
        self.validate_without_digest()?;
        let rebuilt = Self::create(
            self.id.clone(),
            self.created_at,
            self.cutoff_task_evidence.clone(),
            ComparisonSpecificationDraftInput {
                evidence_cutoff: self.evidence_cutoff,
                cohort_labels: self.cohort_labels.clone(),
                date_start: self.date_start,
                date_end: self.date_end,
                stratum: self.stratum.clone(),
            },
        )?;
        if rebuilt.specification_digest != self.specification_digest
            || rebuilt.saved_record_digest != self.saved_record_digest
        {
            return Err(invalid());
        }
        Ok(())
    }
}

impl CutoffTaskEvidenceV1 {
    fn validate(&self, cutoff: DateTime<Utc>) -> Result<()> {
        let id = uuid::Uuid::parse_str(&self.task_id).map_err(|_| invalid())?;
        if id.is_nil()
            || id.to_string() != self.task_id
            || self.material_revision == 0
            || !valid_digest(&self.material_digest)
            || !valid_digest(&self.substantive_material_digest)
            || self
                .outcome_recorded_at
                .is_some_and(|recorded| recorded > cutoff)
            || (matches!(self.outcome, DescriptiveOutcome::Unassessed)
                != self.outcome_recorded_at.is_none())
        {
            return Err(invalid());
        }
        Ok(())
    }
}

pub(super) fn substantive_material_digest(task: &LocalComparisonTaskV1) -> Result<String> {
    let episodes = task
        .episodes
        .iter()
        .map(|episode| {
            (
                &episode.episode_id,
                episode.membership_revision,
                &episode.members_digest,
                &episode.members,
            )
        })
        .collect::<Vec<_>>();
    digest(
        b"trace-commons-comparison-substantive-material-v1\0",
        &(episodes, &task.context),
    )
}

impl ComparisonTaskFactV1 {
    pub fn validate(&self) -> Result<()> {
        let id = uuid::Uuid::parse_str(&self.task_id).map_err(|_| invalid())?;
        if id.is_nil()
            || id.to_string() != self.task_id
            || self.material_revision == 0
            || !valid_digest(&self.material_digest)
            || !valid_digest(&self.substantive_material_digest)
            || !valid_label(&self.category)
            || (matches!(self.outcome, DescriptiveOutcome::Unassessed)
                != self.outcome_recorded_at.is_none())
        {
            return Err(invalid());
        }
        if let Some(stratum) = &self.stratum {
            stratum.validate()?;
        }
        match &self.source_attribution {
            QualifiedSourceAttribution::Qualified {
                rule: _,
                declared_cohort,
                bound_material_revision,
                bound_material_digest,
            } => {
                if !valid_label(declared_cohort)
                    || *bound_material_revision == 0
                    || !valid_digest(bound_material_digest)
                {
                    return Err(invalid());
                }
            }
            QualifiedSourceAttribution::Unavailable { reason } => {
                if !valid_label(reason) {
                    return Err(invalid());
                }
            }
            QualifiedSourceAttribution::WriterFixtureOnly { profile_id } => {
                if !valid_label(profile_id) {
                    return Err(invalid());
                }
            }
        }
        if let ObservedAttributedTokens::Unavailable { reason } = &self.observed_attributed_tokens
            && !valid_label(reason)
        {
            return Err(invalid());
        }
        Ok(())
    }

    fn declared_cohort(&self) -> Option<&str> {
        match &self.source_attribution {
            QualifiedSourceAttribution::Qualified {
                declared_cohort, ..
            } => Some(declared_cohort),
            QualifiedSourceAttribution::Unavailable { .. }
            | QualifiedSourceAttribution::WriterFixtureOnly { .. } => None,
        }
    }
}

/// Resolve a persisted task detail without upgrading pending or fixture-only
/// source evidence into comparison eligibility.
pub fn comparison_task_fact_from_detail(
    detail: &ComparisonTaskDetail,
    source_attribution: QualifiedSourceAttribution,
    observed_attributed_tokens: ObservedAttributedTokens,
) -> Result<ComparisonTaskFactV1> {
    detail.task.validate()?;
    let context = detail.task.context.as_ref();
    let stratum = context.and_then(|context| {
        let ContextString::Known { value: language } = &context.language else {
            return None;
        };
        context.is_complete().then(|| ExactComparisonStratumV1 {
            project_id: context.project_id.clone(),
            language: language.clone(),
            configuration_fingerprint: context.configuration_fingerprint.clone(),
        })
    });
    let evidence_current = !detail.stale_reasons.iter().any(|reason| {
        matches!(
            reason,
            ComparisonTaskStaleReason::EpisodeMissing
                | ComparisonTaskStaleReason::EpisodeRevisionChanged
                | ComparisonTaskStaleReason::EpisodeMembershipChanged
                | ComparisonTaskStaleReason::SnapshotMissingOrReplaced
                | ComparisonTaskStaleReason::OutcomeMaterialChanged
        )
    });
    let independence_current =
        detail
            .task
            .independence_confirmation
            .as_ref()
            .is_some_and(|confirmation| {
                confirmation.material_revision == detail.task.material_revision
                    && confirmation.material_digest == detail.task.material_digest
            });
    let outcome = match detail.task.outcome.as_ref() {
        None => DescriptiveOutcome::Unassessed,
        Some(outcome) => match outcome.value {
            ComparisonTaskOutcome::Pending => DescriptiveOutcome::Pending,
            ComparisonTaskOutcome::Accepted => DescriptiveOutcome::Accepted,
            ComparisonTaskOutcome::Partial => DescriptiveOutcome::Partial,
            ComparisonTaskOutcome::Rejected => DescriptiveOutcome::Rejected,
            ComparisonTaskOutcome::Unknown => DescriptiveOutcome::Unknown,
        },
    };
    let category = match context.map(|context| context.category) {
        Some(TaskCategory::Refactor) => "refactor",
        Some(TaskCategory::Tests) => "tests",
        Some(TaskCategory::Docs) => "docs",
        Some(TaskCategory::Debugging) => "debugging",
        Some(TaskCategory::Other) => "other",
        Some(TaskCategory::Unknown) | None => "unknown",
    };
    let fact = ComparisonTaskFactV1 {
        task_id: detail.task.id.clone(),
        material_revision: detail.task.material_revision,
        material_digest: detail.task.material_digest.clone(),
        substantive_material_digest: substantive_material_digest(&detail.task)?,
        category: category.into(),
        task_date: context
            .map(|context| context.task_date)
            .unwrap_or_else(|| detail.task.created_at.date_naive()),
        material_recorded_at: detail.task.material_recorded_at,
        outcome_recorded_at: detail
            .task
            .outcome
            .as_ref()
            .map(|outcome| outcome.recorded_at),
        stratum,
        outcome,
        evidence_current,
        independence_current,
        overlaps_other_task: !detail.overlapping_task_ids.is_empty(),
        source_attribution,
        observed_attributed_tokens,
    };
    fact.validate()?;
    Ok(fact)
}

#[derive(Serialize)]
struct IncludedEstimationFact<'a> {
    task_id: &'a str,
    cohort: &'a str,
    outcome: &'a DescriptiveOutcome,
}

pub fn project_descriptive_comparison(
    specification: &ComparisonSpecificationV1,
    facts: &[ComparisonTaskFactV1],
) -> Result<DescriptiveComparisonResultV1> {
    specification.validate()?;
    if facts.len() > MAX_FACTS {
        return Err(ComparisonSpecificationError::TaskLimit.into());
    }
    let mut ordered = facts.to_vec();
    ordered.sort_by(|left, right| left.task_id.cmp(&right.task_id));
    if ordered
        .windows(2)
        .any(|pair| pair[0].task_id == pair[1].task_id)
    {
        return Err(invalid());
    }
    for fact in &ordered {
        fact.validate()?;
    }

    let selected = specification.cohort_labels.iter().collect::<BTreeSet<_>>();
    let mut included = Vec::new();
    let mut excluded = Vec::new();
    for fact in &ordered {
        let mut reasons = BTreeSet::new();
        let cutoff_match = specification
            .cutoff_task_evidence
            .binary_search_by(|evidence| evidence.task_id.cmp(&fact.task_id))
            .ok()
            .map(|index| &specification.cutoff_task_evidence[index])
            .is_some_and(|evidence| {
                evidence.substantive_material_digest == fact.substantive_material_digest
                    && evidence.outcome == fact.outcome
                    && evidence.outcome_recorded_at == fact.outcome_recorded_at
            });
        if fact.material_recorded_at.is_none() {
            reasons.insert(ComparisonExclusionReason::CutoffTimeUnavailable);
        } else if fact
            .material_recorded_at
            .is_some_and(|recorded| recorded > specification.evidence_cutoff)
            || !cutoff_match
        {
            reasons.insert(ComparisonExclusionReason::EvidenceAfterCutoff);
        }
        if fact.category != specification.category {
            reasons.insert(ComparisonExclusionReason::CategoryMismatch);
        }
        if fact.task_date < specification.date_start || fact.task_date > specification.date_end {
            reasons.insert(ComparisonExclusionReason::DateOutsideWindow);
        }
        match &fact.stratum {
            None => {
                reasons.insert(ComparisonExclusionReason::ContextUnavailable);
            }
            Some(stratum) if stratum != &specification.stratum => {
                reasons.insert(ComparisonExclusionReason::StratumMismatch);
            }
            Some(_) => {}
        }
        match fact.declared_cohort() {
            None => {
                reasons.insert(ComparisonExclusionReason::CohortUnavailable);
            }
            Some(cohort) if !selected.iter().any(|selected| selected.as_str() == cohort) => {
                reasons.insert(ComparisonExclusionReason::CohortNotSelected);
            }
            Some(_) => {}
        }
        if !fact.evidence_current {
            reasons.insert(ComparisonExclusionReason::EvidenceStale);
        }
        if !fact.independence_current {
            reasons.insert(ComparisonExclusionReason::IndependenceUnconfirmed);
        }
        if fact.overlaps_other_task {
            reasons.insert(ComparisonExclusionReason::OverlappingTaskEvidence);
        }
        match &fact.source_attribution {
            QualifiedSourceAttribution::Unavailable { .. }
            | QualifiedSourceAttribution::WriterFixtureOnly { .. } => {
                reasons.insert(ComparisonExclusionReason::SourceAttributionUnavailable);
            }
            QualifiedSourceAttribution::Qualified {
                bound_material_revision,
                bound_material_digest,
                ..
            } if *bound_material_revision != fact.material_revision
                || bound_material_digest != &fact.material_digest =>
            {
                reasons.insert(ComparisonExclusionReason::SourceAttributionStale);
            }
            QualifiedSourceAttribution::Qualified { .. } => {}
        }
        if reasons.is_empty() {
            included.push(fact);
        } else {
            excluded.push(ExcludedComparisonTask {
                task_id: fact.task_id.clone(),
                reasons: reasons.into_iter().collect(),
            });
        }
    }

    let mut rows = specification
        .cohort_labels
        .iter()
        .map(|label| {
            (
                label.clone(),
                CohortDescriptiveResult {
                    cohort_label: label.clone(),
                    included_tasks: 0,
                    outcomes: OutcomeCounts::default(),
                    usage: UsageCoverage::default(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    for fact in &included {
        let row = rows
            .get_mut(fact.declared_cohort().unwrap())
            .ok_or_else(invalid)?;
        row.included_tasks = row.included_tasks.checked_add(1).ok_or_else(invalid)?;
        let counter = match fact.outcome {
            DescriptiveOutcome::Accepted => &mut row.outcomes.accepted,
            DescriptiveOutcome::Partial => &mut row.outcomes.partial,
            DescriptiveOutcome::Rejected => &mut row.outcomes.rejected,
            DescriptiveOutcome::Pending => &mut row.outcomes.pending,
            DescriptiveOutcome::Unknown => &mut row.outcomes.unknown,
            DescriptiveOutcome::Unassessed => &mut row.outcomes.unassessed,
        };
        *counter = counter.checked_add(1).ok_or_else(invalid)?;
        if matches!(
            fact.outcome,
            DescriptiveOutcome::Accepted
                | DescriptiveOutcome::Partial
                | DescriptiveOutcome::Rejected
        ) {
            row.outcomes.assessed = row.outcomes.assessed.checked_add(1).ok_or_else(invalid)?;
        }
        match fact.observed_attributed_tokens {
            ObservedAttributedTokens::Available { tokens } => {
                row.usage.tasks_with_observed_attributed_tokens = row
                    .usage
                    .tasks_with_observed_attributed_tokens
                    .checked_add(1)
                    .ok_or_else(invalid)?;
                row.usage.observed_attributed_tokens = row
                    .usage
                    .observed_attributed_tokens
                    .checked_add(tokens)
                    .ok_or_else(invalid)?;
            }
            ObservedAttributedTokens::Unavailable { .. } => {
                row.usage.tasks_without_observed_attributed_tokens = row
                    .usage
                    .tasks_without_observed_attributed_tokens
                    .checked_add(1)
                    .ok_or_else(invalid)?;
            }
        }
    }
    let estimation = included
        .iter()
        .map(|fact| IncludedEstimationFact {
            task_id: &fact.task_id,
            cohort: fact.declared_cohort().unwrap(),
            outcome: &fact.outcome,
        })
        .collect::<Vec<_>>();
    let estimation_input_digest = digest(
        b"trace-commons-comparison-estimation-input-v1\0",
        &estimation,
    )?;
    let audit_digest = digest(
        b"trace-commons-comparison-audit-v1\0",
        &(&ordered, &excluded),
    )?;
    let result = DescriptiveComparisonResultV1 {
        schema_version: 1,
        specification_id: specification.id.clone(),
        specification_digest: specification.specification_digest.clone(),
        specification_record_digest: specification.saved_record_digest.clone(),
        audit_digest,
        estimation_input_digest,
        included_task_ids: included.iter().map(|fact| fact.task_id.clone()).collect(),
        excluded_tasks: excluded,
        cohorts: rows.into_values().collect(),
    };
    result.validate()?;
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, day, 12, 0, 0)
            .single()
            .unwrap()
    }

    fn stratum() -> ExactComparisonStratumV1 {
        ExactComparisonStratumV1 {
            project_id: "00000000-0000-4000-8000-000000000001".into(),
            language: "rust".into(),
            configuration_fingerprint: "11".repeat(32),
        }
    }

    fn spec() -> ComparisonSpecificationV1 {
        ComparisonSpecificationV1::create(
            "00000000-0000-4000-8000-000000000010".into(),
            at(12),
            (1..=6)
                .map(|suffix| CutoffTaskEvidenceV1 {
                    task_id: format!("00000000-0000-4000-8000-{suffix:012}"),
                    material_revision: 2,
                    material_digest: format!("{suffix:02x}").repeat(32),
                    substantive_material_digest: format!("{suffix:02x}").repeat(32),
                    outcome: match suffix {
                        1 => DescriptiveOutcome::Accepted,
                        2 => DescriptiveOutcome::Pending,
                        3 => DescriptiveOutcome::Unknown,
                        4 => DescriptiveOutcome::Unassessed,
                        5 => DescriptiveOutcome::Partial,
                        _ => DescriptiveOutcome::Rejected,
                    },
                    outcome_recorded_at: (suffix != 4).then(|| at(9)),
                })
                .collect(),
            ComparisonSpecificationDraftInput {
                evidence_cutoff: at(11),
                cohort_labels: vec!["model-a".into(), "model-b".into()],
                date_start: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                date_end: NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                stratum: stratum(),
            },
        )
        .unwrap()
    }

    fn fact(suffix: u8, cohort: &str, outcome: DescriptiveOutcome) -> ComparisonTaskFactV1 {
        ComparisonTaskFactV1 {
            task_id: format!("00000000-0000-4000-8000-{suffix:012}"),
            material_revision: 2,
            material_digest: format!("{suffix:02x}").repeat(32),
            substantive_material_digest: format!("{suffix:02x}").repeat(32),
            category: "refactor".into(),
            task_date: NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            material_recorded_at: Some(at(8)),
            outcome_recorded_at: (!matches!(outcome, DescriptiveOutcome::Unassessed))
                .then(|| at(9)),
            stratum: Some(stratum()),
            outcome,
            evidence_current: true,
            independence_current: true,
            overlaps_other_task: false,
            source_attribution: QualifiedSourceAttribution::Qualified {
                rule: QualifiedSourceRule::SyntheticReleasedRuleV1,
                declared_cohort: cohort.into(),
                bound_material_revision: 2,
                bound_material_digest: format!("{suffix:02x}").repeat(32),
            },
            observed_attributed_tokens: ObservedAttributedTokens::Unavailable {
                reason: "usage-unavailable".into(),
            },
        }
    }

    #[test]
    fn restored_material_cannot_reenter_a_saved_cutoff() {
        let specification = spec();
        let mut restored = fact(4, "model-a", DescriptiveOutcome::Unassessed);
        restored.material_recorded_at = Some(specification.evidence_cutoff);
        assert_eq!(
            project_descriptive_comparison(&specification, std::slice::from_ref(&restored))
                .unwrap()
                .included_task_ids,
            std::slice::from_ref(&restored.task_id)
        );
        // Restoring context A after A -> B -> A can restore the substantive
        // digest, but its newly recorded material still postdates the cutoff.
        restored.material_recorded_at = Some(at(12));
        let result = project_descriptive_comparison(&specification, &[restored]).unwrap();
        assert!(result.included_task_ids.is_empty());
        assert_eq!(
            result.excluded_tasks[0].reasons,
            [ComparisonExclusionReason::EvidenceAfterCutoff]
        );
    }

    #[test]
    fn specification_is_explicitly_retrospective_and_digest_bound() {
        let value = spec();
        assert_eq!(
            value.provenance,
            SpecificationProvenance::RetrospectiveUserSpecification
        );
        assert!(value.evidence_cutoff < value.created_at);
        value.validate().unwrap();
        let bytes = serde_json::to_vec(&value).unwrap();
        assert_eq!(
            serde_json::from_slice::<ComparisonSpecificationV1>(&bytes).unwrap(),
            value
        );

        let mut forged = value;
        forged.task_rule = "changed-after-save".into();
        assert!(forged.validate().is_err());

        let mut cutoff = spec().cutoff_task_evidence;
        cutoff.pop();
        let different_record = ComparisonSpecificationV1::create(
            "00000000-0000-4000-8000-000000000011".into(),
            at(12),
            cutoff,
            ComparisonSpecificationDraftInput {
                evidence_cutoff: at(11),
                cohort_labels: vec!["model-a".into(), "model-b".into()],
                date_start: NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                date_end: NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                stratum: stratum(),
            },
        )
        .unwrap();
        assert_eq!(
            spec().specification_digest,
            different_record.specification_digest,
            "audit-only cutoff membership must not enter future estimator seed inputs"
        );
        assert_ne!(
            spec().saved_record_digest,
            different_record.saved_record_digest
        );
    }

    #[test]
    fn outcomes_keep_conditional_denominators_and_usage_unknowns() {
        let mut facts = vec![
            fact(1, "model-a", DescriptiveOutcome::Accepted),
            fact(2, "model-a", DescriptiveOutcome::Pending),
            fact(3, "model-a", DescriptiveOutcome::Unknown),
            fact(4, "model-a", DescriptiveOutcome::Unassessed),
            fact(5, "model-b", DescriptiveOutcome::Partial),
            fact(6, "model-b", DescriptiveOutcome::Rejected),
        ];
        facts[0].observed_attributed_tokens = ObservedAttributedTokens::Available { tokens: 120 };
        let result = project_descriptive_comparison(&spec(), &facts).unwrap();
        assert_eq!(result.included_task_ids.len(), 6);
        assert!(result.excluded_tasks.is_empty());
        assert_eq!(result.cohorts[0].outcomes.accepted, 1);
        assert_eq!(result.cohorts[0].outcomes.pending, 1);
        assert_eq!(result.cohorts[0].outcomes.unknown, 1);
        assert_eq!(result.cohorts[0].outcomes.unassessed, 1);
        assert_eq!(result.cohorts[0].outcomes.assessed, 1);
        assert_eq!(
            result.cohorts[0]
                .usage
                .tasks_with_observed_attributed_tokens,
            1
        );
        assert_eq!(
            result.cohorts[0]
                .usage
                .tasks_without_observed_attributed_tokens,
            3
        );
        assert_eq!(result.cohorts[1].outcomes.assessed, 2);
    }

    #[test]
    fn import_order_is_canonical_and_exclusions_do_not_change_estimation_input() {
        let included = fact(1, "model-a", DescriptiveOutcome::Accepted);
        let mut excluded = fact(2, "model-b", DescriptiveOutcome::Rejected);
        excluded.independence_current = false;
        let first =
            project_descriptive_comparison(&spec(), &[excluded.clone(), included.clone()]).unwrap();
        let second =
            project_descriptive_comparison(&spec(), &[included.clone(), excluded.clone()]).unwrap();
        assert_eq!(first, second);

        excluded.observed_attributed_tokens = ObservedAttributedTokens::Unavailable {
            reason: "different-provenance".into(),
        };
        let changed = project_descriptive_comparison(&spec(), &[included, excluded]).unwrap();
        assert_ne!(first.audit_digest, changed.audit_digest);
        assert_eq!(
            first.estimation_input_digest,
            changed.estimation_input_digest
        );
    }

    #[test]
    fn all_eligibility_failures_are_explicit_and_sorted() {
        let mut value = fact(1, "other-model", DescriptiveOutcome::Accepted);
        value.outcome_recorded_at = Some(at(12));
        value.task_date = NaiveDate::from_ymd_opt(2026, 8, 1).unwrap();
        value.stratum.as_mut().unwrap().language = "swift".into();
        value.evidence_current = false;
        value.independence_current = false;
        value.overlaps_other_task = true;
        value.source_attribution = QualifiedSourceAttribution::Unavailable {
            reason: "fixture-only".into(),
        };
        let result = project_descriptive_comparison(&spec(), &[value]).unwrap();
        assert!(result.included_task_ids.is_empty());
        assert_eq!(result.excluded_tasks.len(), 1);
        assert_eq!(result.excluded_tasks[0].reasons.len(), 8);
        assert!(result.cohorts.iter().all(|row| row.included_tasks == 0));
    }

    #[test]
    fn fixture_only_cannot_be_claimed_as_qualified() {
        let mut value = fact(1, "model-a", DescriptiveOutcome::Accepted);
        value.source_attribution = QualifiedSourceAttribution::WriterFixtureOnly {
            profile_id: "codex-writer-fixture-v1".into(),
        };
        let result = project_descriptive_comparison(&spec(), &[value]).unwrap();
        assert_eq!(
            result.excluded_tasks[0].reasons,
            [
                ComparisonExclusionReason::CohortUnavailable,
                ComparisonExclusionReason::SourceAttributionUnavailable,
            ]
        );
    }

    #[test]
    fn observed_token_overflow_fails_instead_of_wrapping() {
        let mut first = fact(1, "model-a", DescriptiveOutcome::Accepted);
        first.observed_attributed_tokens = ObservedAttributedTokens::Available { tokens: u64::MAX };
        let mut second = fact(2, "model-a", DescriptiveOutcome::Pending);
        second.observed_attributed_tokens = ObservedAttributedTokens::Available { tokens: 1 };
        assert!(project_descriptive_comparison(&spec(), &[first, second]).is_err());
    }
}
