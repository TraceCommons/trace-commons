//! Versioned inputs and presentation-neutral results for deterministic local Insights cards.
//!
//! The host selects and validates saved evidence. These contracts grant no access to source
//! content and make no ranking, independence, cost, active-time, or time-saved claim.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::insights::{EvidenceRef, ExecutionMode, ProviderManifest};

pub const INSIGHT_CARD_SCHEMA_VERSION: u32 = 1;
pub const INSIGHT_CARD_RUBRIC_VERSION: &str = "deterministic-question-cards-v1";
pub const MAX_CARD_EVIDENCE: usize = 1024;
pub const MAX_CARD_EPISODES: usize = 256;
pub const MAX_CARD_EPISODE_MEMBERS: usize = 64;
pub const MAX_CARD_ROWS: usize = 64;
pub const MAX_CARD_MODEL_LABELS: usize = 32;
pub const MAX_EXTREMUM_EVENT_REFS: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InsightQuestionId {
    RecordedActivity,
    EpisodeOutcomes,
    ObservedModels,
    EstimatedCost,
}

impl InsightQuestionId {
    pub const ALL: [Self; 4] = [
        Self::RecordedActivity,
        Self::EpisodeOutcomes,
        Self::ObservedModels,
        Self::EstimatedCost,
    ];

    pub const fn metric_version(self) -> &'static str {
        match self {
            Self::RecordedActivity => "recorded-activity-v1",
            Self::EpisodeOutcomes => "episode-outcomes-v1",
            Self::ObservedModels => "observed-models-v1",
            Self::EstimatedCost => "estimated-cost-v1",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardSourceFormat {
    Codex,
    Trajectory,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeEvidenceScope {
    RecordedEventTimestampsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimeRecordCoordinates {
    JsonlPhysicalLinesOneBased,
    TrajectoryArrayIndexesZeroBased,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimestampEventRef {
    pub record_index: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TimestampExtremum {
    pub recorded_at: DateTime<Utc>,
    pub event_refs: Vec<TimestampEventRef>,
    pub omitted_event_refs: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardTimeEvidence {
    pub schema_version: u32,
    pub scope: TimeEvidenceScope,
    pub source_format: CardSourceFormat,
    pub source_digest: String,
    pub coordinates: TimeRecordCoordinates,
    pub total_eligible_records: u64,
    pub valid_timestamps: u64,
    pub missing_timestamps: u64,
    pub invalid_timestamps: u64,
    pub earliest: Option<TimestampExtremum>,
    pub latest: Option<TimestampExtremum>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CountFact {
    pub value: Option<u64>,
    pub observed: u64,
    pub eligible: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelFact {
    pub label: String,
    pub observed_records: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotCardInput {
    pub evidence: EvidenceRef,
    pub source_format: CardSourceFormat,
    pub normalized_events: CountFact,
    pub tool_calls: CountFact,
    pub tool_failures: CountFact,
    pub models: Vec<ModelFact>,
    pub model_labels_omitted: bool,
    pub omitted_model_records: u64,
    pub model_eligible_records: u64,
    pub model_observed_records: u64,
    pub time: Option<CardTimeEvidence>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeCategory {
    Refactor,
    Tests,
    Docs,
    Debugging,
    Other,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeOutcome {
    Accepted,
    Partial,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeAssessmentProvenance {
    UserReported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeAssessmentInput {
    pub category: EpisodeCategory,
    pub outcome: EpisodeOutcome,
    pub provenance: EpisodeAssessmentProvenance,
    pub membership_revision: u64,
    pub members_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeMemberInput {
    pub snapshot_id: String,
    pub source_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeCardInput {
    pub id: String,
    pub revision: u64,
    pub membership_revision: u64,
    pub members_digest: String,
    pub members: Vec<EpisodeMemberInput>,
    pub assessment: Option<EpisodeAssessmentInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightCardRequest {
    pub schema_version: u32,
    pub expected_provider: ProviderManifest,
    pub questions: Vec<InsightQuestionId>,
    pub evidence: Vec<EvidenceRef>,
    pub snapshots: Vec<SnapshotCardInput>,
    pub episodes: Vec<EpisodeCardInput>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardState {
    Observed,
    Partial,
    Unavailable,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardUnit {
    SavedSnapshots,
    NormalizedEvents,
    ToolCalls,
    ToolResults,
    EpisodeGroups,
    DistinctSavedSnapshots,
    TimestampRecords,
    UnixMilliseconds,
    Milliseconds,
    ModelRecords,
    UsDollars,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardRowId {
    SavedSnapshots,
    NormalizedEvents,
    ToolCalls,
    ToolFailures,
    TimestampEligible,
    TimestampValid,
    TimestampMissing,
    TimestampInvalid,
    EarliestRecordedAt,
    LatestRecordedAt,
    RecordSpan,
    EligibleEpisodes,
    AssessedEpisodes,
    AcceptedEpisodes,
    PartialEpisodes,
    RejectedEpisodes,
    ExplicitUnknownEpisodes,
    UnassessedEpisodes,
    OverlappingEpisodes,
    DistinctSnapshots,
    ObservedModel,
    EstimatedCost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "snake_case")]
pub enum CardValue {
    Count(u64),
    UnixMilliseconds(i64),
    Milliseconds(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MissingReason {
    NoEligibleEvidence,
    NoObservedValue,
    InsufficientTimestamps,
    UsageNotPersisted,
    PricingUnavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardRow {
    pub id: CardRowId,
    pub unit: CardUnit,
    pub label: Option<String>,
    pub value: Option<CardValue>,
    pub missing_reason: Option<MissingReason>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageUnit {
    SavedSnapshots,
    NormalizedEvents,
    ToolCallEvents,
    ToolResults,
    EpisodeGroups,
    TimestampEvidenceSnapshots,
    TimestampRecords,
    ModelRecords,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CardCoverage {
    pub unit: CoverageUnit,
    pub observed: u64,
    pub eligible: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeDenominator {
    pub eligible: u64,
    pub assessed: u64,
    pub unassessed: u64,
    pub explicit_unknown: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardLimitation {
    UserSelectedEpisodeGroups,
    EpisodesNotIndependentTasks,
    EpisodeGroupsOverlap,
    TimestampsAreRecordSpan,
    RecordSpanIsNotActiveTime,
    ToolFailuresAreNotRejections,
    ModelDeclarationsAreObservedMetadata,
    CostUnavailableWithoutPersistedUsageAndPricing,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightCard {
    pub question: InsightQuestionId,
    pub metric_version: String,
    pub state: CardState,
    pub rows: Vec<CardRow>,
    pub coverage: Vec<CardCoverage>,
    pub episode_denominator: Option<EpisodeDenominator>,
    pub evidence_ids: Vec<String>,
    pub episode_ids: Vec<String>,
    pub limitations: Vec<CardLimitation>,
    pub rows_omitted: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InsightCardResult {
    pub schema_version: u32,
    pub provider: ProviderManifest,
    pub input_digest: String,
    pub cards: Vec<InsightCard>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CardValidationError {
    #[error("insight-card-schema-unsupported")]
    SchemaVersion,
    #[error("insight-card-input-invalid")]
    InvalidInput,
    #[error("insight-card-input-too-large")]
    InputTooLarge,
    #[error("insight-card-provider-incompatible")]
    IncompatibleProvider,
    #[error("insight-card-evidence-invalid")]
    InvalidEvidence,
    #[error("insight-card-digest-invalid")]
    InvalidDigest,
    #[error("insight-card-missing")]
    MissingCard,
    #[error("insight-card-duplicate")]
    DuplicateCard,
    #[error("insight-card-row-invalid")]
    InvalidRow,
    #[error("insight-card-coverage-invalid")]
    InvalidCoverage,
}

impl InsightCardRequest {
    pub fn validate(&self) -> Result<(), CardValidationError> {
        if self.schema_version != INSIGHT_CARD_SCHEMA_VERSION {
            return Err(CardValidationError::SchemaVersion);
        }
        validate_provider(&self.expected_provider)?;
        if self.questions.is_empty() {
            return Err(CardValidationError::InvalidInput);
        }
        if self.questions.len() > InsightQuestionId::ALL.len()
            || self.evidence.len() > MAX_CARD_EVIDENCE
            || self.snapshots.len() > MAX_CARD_EVIDENCE
            || self.episodes.len() > MAX_CARD_EPISODES
        {
            return Err(CardValidationError::InputTooLarge);
        }
        ensure_strict(&self.questions)?;
        let evidence = evidence_map(&self.evidence)?;
        if self.snapshots.len() != self.evidence.len() {
            return Err(CardValidationError::InvalidEvidence);
        }
        let mut last_snapshot = None;
        for snapshot in &self.snapshots {
            if last_snapshot.is_some_and(|last: &str| last >= snapshot.evidence.id.as_str()) {
                return Err(CardValidationError::InvalidEvidence);
            }
            last_snapshot = Some(&snapshot.evidence.id);
            if evidence.get(snapshot.evidence.id.as_str())
                != Some(&snapshot.evidence.source_digest.as_str())
            {
                return Err(CardValidationError::InvalidEvidence);
            }
            validate_snapshot(snapshot)?;
        }
        let mut episode_ids = BTreeSet::new();
        let mut last_episode = None;
        for episode in &self.episodes {
            if !episode_ids.insert(episode.id.as_str())
                || last_episode.is_some_and(|last: &str| last >= episode.id.as_str())
            {
                return Err(CardValidationError::InvalidInput);
            }
            last_episode = Some(&episode.id);
            validate_episode(episode, &evidence)?;
        }
        Ok(())
    }

    pub fn input_digest(&self) -> Result<String, CardValidationError> {
        self.validate()?;
        // Validation fixes every vector's order and all structs deny unknown fields. The v1
        // digest deliberately pins serde_json's compact struct-field serialization; changing
        // this encoding requires a new schema/domain rather than an in-place rewrite.
        let bytes = serde_json::to_vec(self).map_err(|_| CardValidationError::InvalidInput)?;
        let mut hash = Sha256::new();
        hash.update(b"trace-commons-insight-card-input-v1\0");
        hash.update((bytes.len() as u64).to_be_bytes());
        hash.update(bytes);
        Ok(format!("{:x}", hash.finalize()))
    }

    /// Derive dependence from selected memberships instead of trusting a provider claim.
    pub fn episode_overlap(
        &self,
    ) -> Result<BTreeMap<String, BTreeSet<String>>, CardValidationError> {
        self.validate()?;
        let mut owners: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
        for episode in &self.episodes {
            for member in &episode.members {
                owners
                    .entry(member.snapshot_id.as_str())
                    .or_default()
                    .push(episode.id.as_str());
            }
        }
        let mut overlap: BTreeMap<String, BTreeSet<String>> = self
            .episodes
            .iter()
            .map(|episode| (episode.id.clone(), BTreeSet::new()))
            .collect();
        for episode_ids in owners.values().filter(|ids| ids.len() > 1) {
            for id in episode_ids {
                overlap
                    .get_mut(*id)
                    .expect("validated episode exists")
                    .extend(
                        episode_ids
                            .iter()
                            .filter(|other| other != &id)
                            .map(|other| (*other).to_owned()),
                    );
            }
        }
        Ok(overlap)
    }
}

impl InsightCardResult {
    pub fn validate_for(&self, request: &InsightCardRequest) -> Result<(), CardValidationError> {
        request.validate()?;
        if self.schema_version != INSIGHT_CARD_SCHEMA_VERSION {
            return Err(CardValidationError::SchemaVersion);
        }
        validate_provider(&self.provider)?;
        if self.provider != request.expected_provider {
            return Err(CardValidationError::IncompatibleProvider);
        }
        if self.input_digest != request.input_digest()? {
            return Err(CardValidationError::InvalidDigest);
        }
        let evidence = evidence_map(&request.evidence)?;
        let episodes: BTreeSet<_> = request.episodes.iter().map(|e| e.id.as_str()).collect();
        let has_overlap = request
            .episode_overlap()?
            .values()
            .any(|other_ids| !other_ids.is_empty());
        let mut seen = BTreeSet::new();
        for card in &self.cards {
            if !seen.insert(card.question) {
                return Err(CardValidationError::DuplicateCard);
            }
        }
        for (index, card) in self.cards.iter().enumerate() {
            if request.questions.get(index) != Some(&card.question) {
                return Err(CardValidationError::MissingCard);
            }
            validate_card(card, request, &evidence, &episodes, has_overlap)?;
        }
        if self.cards.len() != request.questions.len() {
            return Err(CardValidationError::MissingCard);
        }
        if self.cards != expected_cards(request)? {
            return Err(CardValidationError::InvalidRow);
        }
        Ok(())
    }
}

/// Compute the complete schema-v1 card projection from host-selected facts.
///
/// This proves consistency with the request, not that the selected facts are true. The host is
/// responsible for resolving them from current saved evidence before constructing the request.
pub fn expected_cards(
    request: &InsightCardRequest,
) -> Result<Vec<InsightCard>, CardValidationError> {
    request.validate()?;
    request
        .questions
        .iter()
        .map(|question| match question {
            InsightQuestionId::RecordedActivity => expected_activity_card(request),
            InsightQuestionId::EpisodeOutcomes => expected_episode_card(request),
            InsightQuestionId::ObservedModels => expected_models_card(request),
            InsightQuestionId::EstimatedCost => Ok(expected_cost_card(request)),
        })
        .collect()
}

fn expected_activity_card(
    request: &InsightCardRequest,
) -> Result<InsightCard, CardValidationError> {
    let snapshots = request.snapshots.len() as u64;
    let normalized = aggregate_count(request.snapshots.iter().map(|s| &s.normalized_events))?;
    let calls = aggregate_count(request.snapshots.iter().map(|s| &s.tool_calls))?;
    let failures = aggregate_count(request.snapshots.iter().map(|s| &s.tool_failures))?;
    let timed_snapshots = request
        .snapshots
        .iter()
        .filter(|s| s.time.is_some())
        .count() as u64;
    let mut time_eligible = 0_u64;
    let mut time_valid = 0_u64;
    let mut time_missing = 0_u64;
    let mut time_invalid = 0_u64;
    let mut earliest: Option<DateTime<Utc>> = None;
    let mut latest: Option<DateTime<Utc>> = None;
    for time in request
        .snapshots
        .iter()
        .filter_map(|snapshot| snapshot.time.as_ref())
    {
        time_eligible = checked_add(time_eligible, time.total_eligible_records)?;
        time_valid = checked_add(time_valid, time.valid_timestamps)?;
        time_missing = checked_add(time_missing, time.missing_timestamps)?;
        time_invalid = checked_add(time_invalid, time.invalid_timestamps)?;
        if let Some(value) = &time.earliest {
            earliest = Some(earliest.map_or(value.recorded_at, |old| old.min(value.recorded_at)));
        }
        if let Some(value) = &time.latest {
            latest = Some(latest.map_or(value.recorded_at, |old| old.max(value.recorded_at)));
        }
    }
    let earliest_ms = timestamp_millis(earliest)?;
    let latest_ms = timestamp_millis(latest)?;
    let span = match (earliest_ms, latest_ms) {
        (Some(first), Some(last)) if time_valid >= 2 => Some(
            u64::try_from(
                last.checked_sub(first)
                    .ok_or(CardValidationError::InvalidInput)?,
            )
            .map_err(|_| CardValidationError::InvalidInput)?,
        ),
        _ => None,
    };
    let time_missing_reason = if timed_snapshots == 0 || time_eligible == 0 {
        MissingReason::NoEligibleEvidence
    } else {
        MissingReason::NoObservedValue
    };
    let rows = vec![
        count_row(CardRowId::SavedSnapshots, snapshots),
        aggregate_row(CardRowId::NormalizedEvents, normalized),
        aggregate_row(CardRowId::ToolCalls, calls),
        aggregate_row(CardRowId::ToolFailures, failures),
        count_row(CardRowId::TimestampEligible, time_eligible),
        count_row(CardRowId::TimestampValid, time_valid),
        count_row(CardRowId::TimestampMissing, time_missing),
        count_row(CardRowId::TimestampInvalid, time_invalid),
        optional_row(
            CardRowId::EarliestRecordedAt,
            earliest_ms.map(CardValue::UnixMilliseconds),
            time_missing_reason,
        ),
        optional_row(
            CardRowId::LatestRecordedAt,
            latest_ms.map(CardValue::UnixMilliseconds),
            time_missing_reason,
        ),
        optional_row(
            CardRowId::RecordSpan,
            span.map(CardValue::Milliseconds),
            MissingReason::InsufficientTimestamps,
        ),
    ];
    let coverage = vec![
        coverage(CoverageUnit::SavedSnapshots, snapshots, snapshots),
        coverage(
            CoverageUnit::NormalizedEvents,
            normalized.observed,
            normalized.eligible,
        ),
        coverage(CoverageUnit::ToolCallEvents, calls.observed, calls.eligible),
        coverage(
            CoverageUnit::ToolResults,
            failures.observed,
            failures.eligible,
        ),
        coverage(
            CoverageUnit::TimestampEvidenceSnapshots,
            timed_snapshots,
            snapshots,
        ),
        coverage(CoverageUnit::TimestampRecords, time_valid, time_eligible),
    ];
    let state = if snapshots == 0 {
        CardState::Unavailable
    } else if coverage.iter().all(|item| item.observed == item.eligible)
        && rows.iter().all(|row| row.missing_reason.is_none())
    {
        CardState::Observed
    } else {
        CardState::Partial
    };
    Ok(card(
        request,
        CardCore {
            question: InsightQuestionId::RecordedActivity,
            state,
            rows,
            coverage,
            episode_denominator: None,
            limitations: vec![
                CardLimitation::TimestampsAreRecordSpan,
                CardLimitation::RecordSpanIsNotActiveTime,
                CardLimitation::ToolFailuresAreNotRejections,
            ],
            rows_omitted: false,
        },
    ))
}

#[derive(Clone, Copy)]
struct AggregateCount {
    value: Option<u64>,
    observed: u64,
    eligible: u64,
}

fn aggregate_count<'a>(
    facts: impl Iterator<Item = &'a CountFact>,
) -> Result<AggregateCount, CardValidationError> {
    let mut value = 0_u64;
    let mut observed = 0_u64;
    let mut eligible = 0_u64;
    for fact in facts {
        observed = checked_add(observed, fact.observed)?;
        eligible = checked_add(eligible, fact.eligible)?;
        if let Some(part) = fact.value {
            value = checked_add(value, part)?;
        }
    }
    Ok(AggregateCount {
        value: (observed > 0 || eligible == 0).then_some(value),
        observed,
        eligible,
    })
}

fn expected_episode_card(request: &InsightCardRequest) -> Result<InsightCard, CardValidationError> {
    let eligible = request.episodes.len() as u64;
    let mut accepted = 0_u64;
    let mut partial = 0_u64;
    let mut rejected = 0_u64;
    let mut unknown = 0_u64;
    for assessment in request
        .episodes
        .iter()
        .filter_map(|e| e.assessment.as_ref())
    {
        match assessment.outcome {
            EpisodeOutcome::Accepted => accepted += 1,
            EpisodeOutcome::Partial => partial += 1,
            EpisodeOutcome::Rejected => rejected += 1,
            EpisodeOutcome::Unknown => unknown += 1,
        }
    }
    let assessed = accepted + partial + rejected + unknown;
    let unassessed = eligible - assessed;
    let overlap = request.episode_overlap()?;
    let overlapping = overlap.values().filter(|ids| !ids.is_empty()).count() as u64;
    let distinct = request
        .episodes
        .iter()
        .flat_map(|episode| {
            episode
                .members
                .iter()
                .map(|member| member.snapshot_id.as_str())
        })
        .collect::<BTreeSet<_>>()
        .len() as u64;
    let rows = [
        (CardRowId::EligibleEpisodes, eligible),
        (CardRowId::AssessedEpisodes, assessed),
        (CardRowId::AcceptedEpisodes, accepted),
        (CardRowId::PartialEpisodes, partial),
        (CardRowId::RejectedEpisodes, rejected),
        (CardRowId::ExplicitUnknownEpisodes, unknown),
        (CardRowId::UnassessedEpisodes, unassessed),
        (CardRowId::OverlappingEpisodes, overlapping),
        (CardRowId::DistinctSnapshots, distinct),
    ]
    .into_iter()
    .map(|(id, value)| count_row(id, value))
    .collect();
    let denominator = EpisodeDenominator {
        eligible,
        assessed,
        unassessed,
        explicit_unknown: unknown,
    };
    let state = if eligible == 0 || assessed == 0 {
        CardState::Unavailable
    } else if assessed < eligible || unknown > 0 {
        CardState::Partial
    } else {
        CardState::Observed
    };
    let mut limitations = vec![
        CardLimitation::UserSelectedEpisodeGroups,
        CardLimitation::EpisodesNotIndependentTasks,
    ];
    if overlapping > 0 {
        limitations.push(CardLimitation::EpisodeGroupsOverlap);
    }
    Ok(card(
        request,
        CardCore {
            question: InsightQuestionId::EpisodeOutcomes,
            state,
            rows,
            coverage: vec![coverage(CoverageUnit::EpisodeGroups, assessed, eligible)],
            episode_denominator: Some(denominator),
            limitations,
            rows_omitted: false,
        },
    ))
}

fn expected_models_card(request: &InsightCardRequest) -> Result<InsightCard, CardValidationError> {
    let mut labels = BTreeMap::<String, u64>::new();
    let mut eligible = 0_u64;
    let mut observed = 0_u64;
    let mut observed_snapshots = 0_u64;
    let mut source_omission = false;
    for snapshot in &request.snapshots {
        eligible = checked_add(eligible, snapshot.model_eligible_records)?;
        observed = checked_add(observed, snapshot.model_observed_records)?;
        if snapshot.model_observed_records > 0 {
            observed_snapshots += 1;
        }
        source_omission |= snapshot.model_labels_omitted || snapshot.omitted_model_records > 0;
        for model in &snapshot.models {
            let entry = labels.entry(model.label.clone()).or_default();
            *entry = checked_add(*entry, model.observed_records)?;
        }
    }
    let rows_omitted = source_omission || labels.len() > MAX_CARD_ROWS;
    let rows = labels
        .into_iter()
        .take(MAX_CARD_ROWS)
        .map(|(label, value)| CardRow {
            id: CardRowId::ObservedModel,
            unit: CardUnit::ModelRecords,
            label: Some(label),
            value: Some(CardValue::Count(value)),
            missing_reason: None,
        })
        .collect();
    let state = if observed == 0 {
        CardState::Unavailable
    } else if observed < eligible || rows_omitted {
        CardState::Partial
    } else {
        CardState::Observed
    };
    Ok(card(
        request,
        CardCore {
            question: InsightQuestionId::ObservedModels,
            state,
            rows,
            coverage: vec![
                coverage(
                    CoverageUnit::SavedSnapshots,
                    observed_snapshots,
                    request.snapshots.len() as u64,
                ),
                coverage(CoverageUnit::ModelRecords, observed, eligible),
            ],
            episode_denominator: None,
            limitations: vec![CardLimitation::ModelDeclarationsAreObservedMetadata],
            rows_omitted,
        },
    ))
}

fn expected_cost_card(request: &InsightCardRequest) -> InsightCard {
    card(
        request,
        CardCore {
            question: InsightQuestionId::EstimatedCost,
            state: CardState::Unavailable,
            rows: vec![optional_row(
                CardRowId::EstimatedCost,
                None,
                MissingReason::PricingUnavailable,
            )],
            coverage: vec![coverage(
                CoverageUnit::SavedSnapshots,
                0,
                request.snapshots.len() as u64,
            )],
            episode_denominator: None,
            limitations: vec![CardLimitation::CostUnavailableWithoutPersistedUsageAndPricing],
            rows_omitted: false,
        },
    )
}

struct CardCore {
    question: InsightQuestionId,
    state: CardState,
    rows: Vec<CardRow>,
    coverage: Vec<CardCoverage>,
    episode_denominator: Option<EpisodeDenominator>,
    limitations: Vec<CardLimitation>,
    rows_omitted: bool,
}

fn card(request: &InsightCardRequest, core: CardCore) -> InsightCard {
    let episode_question = core.question == InsightQuestionId::EpisodeOutcomes;
    InsightCard {
        question: core.question,
        metric_version: core.question.metric_version().into(),
        state: core.state,
        rows: core.rows,
        coverage: core.coverage,
        episode_denominator: core.episode_denominator,
        evidence_ids: if episode_question {
            request
                .episodes
                .iter()
                .flat_map(|e| e.members.iter().map(|m| m.snapshot_id.clone()))
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect()
        } else {
            request.evidence.iter().map(|e| e.id.clone()).collect()
        },
        episode_ids: if episode_question {
            request.episodes.iter().map(|e| e.id.clone()).collect()
        } else {
            Vec::new()
        },
        limitations: core.limitations,
        rows_omitted: core.rows_omitted,
    }
}

fn count_row(id: CardRowId, value: u64) -> CardRow {
    CardRow {
        id,
        unit: row_dictionary(id).1,
        label: None,
        value: Some(CardValue::Count(value)),
        missing_reason: None,
    }
}
fn aggregate_row(id: CardRowId, fact: AggregateCount) -> CardRow {
    optional_row(
        id,
        fact.value.map(CardValue::Count),
        if fact.eligible == 0 {
            MissingReason::NoEligibleEvidence
        } else {
            MissingReason::NoObservedValue
        },
    )
}
fn optional_row(id: CardRowId, value: Option<CardValue>, missing: MissingReason) -> CardRow {
    CardRow {
        id,
        unit: row_dictionary(id).1,
        label: None,
        missing_reason: value.is_none().then_some(missing),
        value,
    }
}
fn coverage(unit: CoverageUnit, observed: u64, eligible: u64) -> CardCoverage {
    CardCoverage {
        unit,
        observed,
        eligible,
    }
}
fn checked_add(left: u64, right: u64) -> Result<u64, CardValidationError> {
    left.checked_add(right)
        .ok_or(CardValidationError::InvalidInput)
}
fn timestamp_millis(value: Option<DateTime<Utc>>) -> Result<Option<i64>, CardValidationError> {
    Ok(value.map(|at| at.timestamp_millis()))
}

pub const fn row_dictionary(id: CardRowId) -> (InsightQuestionId, CardUnit) {
    use CardRowId::*;
    match id {
        SavedSnapshots => (
            InsightQuestionId::RecordedActivity,
            CardUnit::SavedSnapshots,
        ),
        NormalizedEvents => (
            InsightQuestionId::RecordedActivity,
            CardUnit::NormalizedEvents,
        ),
        ToolCalls => (InsightQuestionId::RecordedActivity, CardUnit::ToolCalls),
        ToolFailures => (InsightQuestionId::RecordedActivity, CardUnit::ToolResults),
        TimestampEligible | TimestampValid | TimestampMissing | TimestampInvalid => (
            InsightQuestionId::RecordedActivity,
            CardUnit::TimestampRecords,
        ),
        EarliestRecordedAt | LatestRecordedAt => (
            InsightQuestionId::RecordedActivity,
            CardUnit::UnixMilliseconds,
        ),
        RecordSpan => (InsightQuestionId::RecordedActivity, CardUnit::Milliseconds),
        EligibleEpisodes
        | AssessedEpisodes
        | AcceptedEpisodes
        | PartialEpisodes
        | RejectedEpisodes
        | ExplicitUnknownEpisodes
        | UnassessedEpisodes
        | OverlappingEpisodes => (InsightQuestionId::EpisodeOutcomes, CardUnit::EpisodeGroups),
        DistinctSnapshots => (
            InsightQuestionId::EpisodeOutcomes,
            CardUnit::DistinctSavedSnapshots,
        ),
        ObservedModel => (InsightQuestionId::ObservedModels, CardUnit::ModelRecords),
        EstimatedCost => (InsightQuestionId::EstimatedCost, CardUnit::UsDollars),
    }
}

fn validate_provider(provider: &ProviderManifest) -> Result<(), CardValidationError> {
    let label = |s: &str| {
        !s.is_empty()
            && s.len() <= 128
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    };
    if provider.schema_version != crate::insights::INSIGHT_SCHEMA_VERSION
        || provider.execution_mode != ExecutionMode::Local
        || !label(&provider.id)
        || !label(&provider.version)
        || provider.rubric_version != INSIGHT_CARD_RUBRIC_VERSION
    {
        return Err(CardValidationError::IncompatibleProvider);
    }
    Ok(())
}

fn evidence_map(evidence: &[EvidenceRef]) -> Result<BTreeMap<&str, &str>, CardValidationError> {
    let mut map = BTreeMap::new();
    let mut last = None;
    for e in evidence {
        if last.is_some_and(|v: &str| v >= e.id.as_str())
            || !valid_digest(&e.id)
            || !valid_digest(&e.source_digest)
            || map
                .insert(e.id.as_str(), e.source_digest.as_str())
                .is_some()
        {
            return Err(CardValidationError::InvalidEvidence);
        }
        last = Some(&e.id);
    }
    Ok(map)
}

fn validate_snapshot(s: &SnapshotCardInput) -> Result<(), CardValidationError> {
    for fact in [&s.normalized_events, &s.tool_calls, &s.tool_failures] {
        validate_count(fact)?;
    }
    if s.models.len() > MAX_CARD_MODEL_LABELS
        || s.model_observed_records > s.model_eligible_records
        || (s.model_labels_omitted
            && (s.models.len() != MAX_CARD_MODEL_LABELS || s.omitted_model_records == 0))
    {
        return Err(CardValidationError::InvalidInput);
    }
    let mut last = None;
    for model in &s.models {
        if !valid_model_label(&model.label)
            || model.observed_records == 0
            || last.is_some_and(|v: &str| v >= model.label.as_str())
        {
            return Err(CardValidationError::InvalidInput);
        }
        last = Some(&model.label);
    }
    if s.models
        .iter()
        .try_fold(s.omitted_model_records, |sum, model| {
            sum.checked_add(model.observed_records)
        })
        != Some(s.model_observed_records)
    {
        return Err(CardValidationError::InvalidInput);
    }
    if let Some(time) = &s.time {
        validate_time(time, s)?;
    }
    Ok(())
}

fn validate_count(f: &CountFact) -> Result<(), CardValidationError> {
    if f.observed > f.eligible
        || (f.value.is_none() && f.observed != 0)
        || (f.value.is_some() && f.observed == 0 && !(f.eligible == 0 && f.value == Some(0)))
    {
        return Err(CardValidationError::InvalidCoverage);
    }
    Ok(())
}

fn validate_time(t: &CardTimeEvidence, s: &SnapshotCardInput) -> Result<(), CardValidationError> {
    if t.schema_version != 1
        || t.source_digest != s.evidence.source_digest
        || t.source_format != s.source_format
        || t.total_eligible_records > 16 * 1024 * 1024
        || t.valid_timestamps
            .checked_add(t.missing_timestamps)
            .and_then(|v| v.checked_add(t.invalid_timestamps))
            != Some(t.total_eligible_records)
    {
        return Err(CardValidationError::InvalidInput);
    }
    match (t.source_format, t.coordinates) {
        (CardSourceFormat::Codex, TimeRecordCoordinates::JsonlPhysicalLinesOneBased)
        | (CardSourceFormat::Trajectory, TimeRecordCoordinates::JsonlPhysicalLinesOneBased)
        | (CardSourceFormat::Trajectory, TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased) => {
        }
        _ => return Err(CardValidationError::InvalidInput),
    }
    if t.valid_timestamps == 0 {
        if t.earliest.is_some() || t.latest.is_some() {
            return Err(CardValidationError::InvalidInput);
        }
    } else {
        let (Some(first), Some(last)) = (&t.earliest, &t.latest) else {
            return Err(CardValidationError::InvalidInput);
        };
        if first.recorded_at > last.recorded_at {
            return Err(CardValidationError::InvalidInput);
        }
        validate_extremum(
            first,
            t.coordinates,
            t.total_eligible_records,
            t.valid_timestamps,
        )?;
        validate_extremum(
            last,
            t.coordinates,
            t.total_eligible_records,
            t.valid_timestamps,
        )?;
        let first_count = (first.event_refs.len() as u64)
            .checked_add(first.omitted_event_refs)
            .ok_or(CardValidationError::InvalidInput)?;
        let last_count = (last.event_refs.len() as u64)
            .checked_add(last.omitted_event_refs)
            .ok_or(CardValidationError::InvalidInput)?;
        if first.recorded_at == last.recorded_at {
            if first != last || first_count != t.valid_timestamps {
                return Err(CardValidationError::InvalidInput);
            }
        } else if first_count
            .checked_add(last_count)
            .is_none_or(|count| count > t.valid_timestamps)
            || first
                .event_refs
                .iter()
                .any(|left| last.event_refs.contains(left))
        {
            return Err(CardValidationError::InvalidInput);
        }
    }
    Ok(())
}

fn validate_extremum(
    e: &TimestampExtremum,
    coordinates: TimeRecordCoordinates,
    total_eligible: u64,
    valid_timestamps: u64,
) -> Result<(), CardValidationError> {
    if e.event_refs.is_empty()
        || e.event_refs.len() > MAX_EXTREMUM_EVENT_REFS
        || (e.omitted_event_refs > 0 && e.event_refs.len() != MAX_EXTREMUM_EVENT_REFS)
        || (e.event_refs.len() as u64)
            .checked_add(e.omitted_event_refs)
            .is_none_or(|count| count > valid_timestamps)
    {
        return Err(CardValidationError::InvalidInput);
    }
    let mut last = None;
    for r in &e.event_refs {
        let max_index = match coordinates {
            TimeRecordCoordinates::JsonlPhysicalLinesOneBased => 16 * 1024 * 1024,
            TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased => total_eligible,
        };
        if r.record_index == 0 || r.record_index > max_index {
            return Err(CardValidationError::InvalidInput);
        }
        if last.is_some_and(|v| v >= r.record_index) {
            return Err(CardValidationError::InvalidInput);
        }
        last = Some(r.record_index);
    }
    Ok(())
}

fn validate_episode(
    e: &EpisodeCardInput,
    evidence: &BTreeMap<&str, &str>,
) -> Result<(), CardValidationError> {
    if !valid_episode_id(&e.id)
        || e.revision == 0
        || e.membership_revision == 0
        || e.membership_revision > e.revision
        || e.members.is_empty()
        || e.members.len() > MAX_CARD_EPISODE_MEMBERS
        || !valid_digest(&e.members_digest)
    {
        return Err(CardValidationError::InvalidInput);
    }
    let mut last = None;
    for m in &e.members {
        if !valid_digest(&m.snapshot_id)
            || !valid_digest(&m.source_digest)
            || last.is_some_and(|v: &str| v >= m.snapshot_id.as_str())
            || evidence.get(m.snapshot_id.as_str()) != Some(&m.source_digest.as_str())
        {
            return Err(CardValidationError::InvalidEvidence);
        }
        last = Some(&m.snapshot_id);
    }
    if episode_members_digest(&e.members)? != e.members_digest {
        return Err(CardValidationError::InvalidDigest);
    }
    if let Some(a) = &e.assessment
        && (a.membership_revision != e.membership_revision || a.members_digest != e.members_digest)
    {
        return Err(CardValidationError::InvalidInput);
    }
    Ok(())
}

/// Digest compatible with the whole-snapshot episode membership contract.
pub fn episode_members_digest(
    members: &[EpisodeMemberInput],
) -> Result<String, CardValidationError> {
    if members.is_empty() || members.len() > MAX_CARD_EPISODE_MEMBERS {
        return Err(CardValidationError::InvalidInput);
    }
    let mut last = None;
    for member in members {
        if !valid_digest(&member.snapshot_id)
            || !valid_digest(&member.source_digest)
            || last.is_some_and(|value: &str| value >= member.snapshot_id.as_str())
        {
            return Err(CardValidationError::InvalidInput);
        }
        last = Some(&member.snapshot_id);
    }
    let mut h = Sha256::new();
    h.update(b"trace-commons-episode-members-v1\0");
    h.update((members.len() as u64).to_be_bytes());
    for m in members {
        h.update(m.snapshot_id.as_bytes());
        h.update(m.source_digest.as_bytes());
    }
    Ok(format!("{:x}", h.finalize()))
}

fn validate_card(
    card: &InsightCard,
    request: &InsightCardRequest,
    evidence: &BTreeMap<&str, &str>,
    episodes: &BTreeSet<&str>,
    has_overlap: bool,
) -> Result<(), CardValidationError> {
    if card.metric_version != card.question.metric_version() || card.rows.len() > MAX_CARD_ROWS {
        return Err(CardValidationError::InvalidRow);
    }
    if card.rows_omitted && card.question != InsightQuestionId::ObservedModels {
        return Err(CardValidationError::InvalidRow);
    }
    let mut row_ids = BTreeSet::new();
    let mut last_row = None;
    for row in &card.rows {
        if !row_ids.insert((row.id, row.label.as_deref()))
            || last_row.is_some_and(|last| last >= (row.id, row.label.as_deref()))
            || row_dictionary(row.id) != (card.question, row.unit)
        {
            return Err(CardValidationError::InvalidRow);
        }
        if row.label.as_ref().is_some_and(|s| !valid_model_label(s))
            || (row.label.is_some() != (row.id == CardRowId::ObservedModel))
            || (row.value.is_some() == row.missing_reason.is_some())
            || row
                .missing_reason
                .is_some_and(|reason| !missingness_dictionary(row.id).contains(&reason))
        {
            return Err(CardValidationError::InvalidRow);
        }
        if !value_matches_unit(row.value.as_ref(), row.unit)
            || (row.id == CardRowId::EstimatedCost && row.value.is_some())
        {
            return Err(CardValidationError::InvalidRow);
        }
        last_row = Some((row.id, row.label.as_deref()));
    }
    let allowed_coverage = coverage_dictionary(card.question);
    let mut coverage_units = BTreeSet::new();
    for c in &card.coverage {
        if c.observed > c.eligible
            || !allowed_coverage.contains(&c.unit)
            || !coverage_units.insert(c.unit)
        {
            return Err(CardValidationError::InvalidCoverage);
        }
    }
    if card.coverage.len() != allowed_coverage.len()
        || !card
            .coverage
            .iter()
            .zip(allowed_coverage)
            .all(|(actual, expected)| actual.unit == *expected)
    {
        return Err(CardValidationError::InvalidCoverage);
    }
    if let Some(d) = &card.episode_denominator {
        if card.question != InsightQuestionId::EpisodeOutcomes
            || d.assessed.checked_add(d.unassessed) != Some(d.eligible)
            || d.explicit_unknown > d.assessed
        {
            return Err(CardValidationError::InvalidCoverage);
        }
    } else if card.question == InsightQuestionId::EpisodeOutcomes {
        return Err(CardValidationError::InvalidCoverage);
    }
    validate_required_rows(card, request)?;
    validate_required_limitations(card, has_overlap)?;
    validate_state(card)?;
    ensure_refs(&card.evidence_ids, evidence.keys().copied())?;
    ensure_refs(&card.episode_ids, episodes.iter().copied())?;
    ensure_strict(&card.limitations)?;
    Ok(())
}

fn validate_state(card: &InsightCard) -> Result<(), CardValidationError> {
    let expected = match card.question {
        InsightQuestionId::RecordedActivity => {
            let saved = card
                .coverage
                .iter()
                .find(|coverage| coverage.unit == CoverageUnit::SavedSnapshots)
                .expect("required coverage");
            if saved.eligible == 0 {
                CardState::Unavailable
            } else if card.coverage.iter().all(|c| c.observed == c.eligible)
                && card.rows.iter().all(|row| row.missing_reason.is_none())
                && !card.rows_omitted
            {
                CardState::Observed
            } else {
                CardState::Partial
            }
        }
        InsightQuestionId::EpisodeOutcomes => {
            let d = card
                .episode_denominator
                .as_ref()
                .expect("required denominator");
            if d.eligible == 0 || d.assessed == 0 {
                CardState::Unavailable
            } else if d.assessed < d.eligible || d.explicit_unknown > 0 {
                CardState::Partial
            } else {
                CardState::Observed
            }
        }
        InsightQuestionId::ObservedModels => {
            let models = card
                .coverage
                .iter()
                .find(|coverage| coverage.unit == CoverageUnit::ModelRecords)
                .expect("required coverage");
            if models.observed == 0 {
                CardState::Unavailable
            } else if models.observed < models.eligible || card.rows_omitted {
                CardState::Partial
            } else {
                CardState::Observed
            }
        }
        InsightQuestionId::EstimatedCost => CardState::Unavailable,
    };
    if card.state != expected {
        return Err(CardValidationError::InvalidRow);
    }
    Ok(())
}

fn value_matches_unit(value: Option<&CardValue>, unit: CardUnit) -> bool {
    match value {
        None => true,
        Some(CardValue::Count(_)) => !matches!(
            unit,
            CardUnit::UnixMilliseconds | CardUnit::Milliseconds | CardUnit::UsDollars
        ),
        Some(CardValue::UnixMilliseconds(_)) => unit == CardUnit::UnixMilliseconds,
        Some(CardValue::Milliseconds(_)) => unit == CardUnit::Milliseconds,
    }
}

fn validate_required_rows(
    card: &InsightCard,
    request: &InsightCardRequest,
) -> Result<(), CardValidationError> {
    use CardRowId::*;
    let required: &[CardRowId] = match card.question {
        InsightQuestionId::RecordedActivity => &[
            SavedSnapshots,
            NormalizedEvents,
            ToolCalls,
            ToolFailures,
            TimestampEligible,
            TimestampValid,
            TimestampMissing,
            TimestampInvalid,
            EarliestRecordedAt,
            LatestRecordedAt,
            RecordSpan,
        ],
        InsightQuestionId::EpisodeOutcomes => &[
            EligibleEpisodes,
            AssessedEpisodes,
            AcceptedEpisodes,
            PartialEpisodes,
            RejectedEpisodes,
            ExplicitUnknownEpisodes,
            UnassessedEpisodes,
            OverlappingEpisodes,
            DistinctSnapshots,
        ],
        InsightQuestionId::ObservedModels => &[],
        InsightQuestionId::EstimatedCost => &[EstimatedCost],
    };
    if card.question != InsightQuestionId::ObservedModels
        && card
            .rows
            .iter()
            .map(|row| row.id)
            .ne(required.iter().copied())
    {
        return Err(CardValidationError::InvalidRow);
    }
    if card.question == InsightQuestionId::ObservedModels
        && card.rows.iter().any(|row| row.id != ObservedModel)
    {
        return Err(CardValidationError::InvalidRow);
    }
    if card.question == InsightQuestionId::EpisodeOutcomes {
        let counts: BTreeMap<_, _> = card
            .rows
            .iter()
            .filter_map(|row| match row.value {
                Some(CardValue::Count(value)) => Some((row.id, value)),
                _ => None,
            })
            .collect();
        let d = card.episode_denominator.as_ref().expect("checked");
        let assessed = request
            .episodes
            .iter()
            .filter_map(|episode| episode.assessment.as_ref());
        let mut outcome_counts = [0_u64; 4];
        for assessment in assessed {
            let index = match assessment.outcome {
                EpisodeOutcome::Accepted => 0,
                EpisodeOutcome::Partial => 1,
                EpisodeOutcome::Rejected => 2,
                EpisodeOutcome::Unknown => 3,
            };
            outcome_counts[index] += 1;
        }
        let expected_assessed: u64 = outcome_counts.iter().sum();
        let expected_eligible = request.episodes.len() as u64;
        let overlap = request.episode_overlap()?;
        let expected_overlapping =
            overlap.values().filter(|others| !others.is_empty()).count() as u64;
        let expected_distinct = request
            .episodes
            .iter()
            .flat_map(|episode| {
                episode
                    .members
                    .iter()
                    .map(|member| member.snapshot_id.as_str())
            })
            .collect::<BTreeSet<_>>()
            .len() as u64;
        let assessed_outcomes = [
            AcceptedEpisodes,
            PartialEpisodes,
            RejectedEpisodes,
            ExplicitUnknownEpisodes,
        ]
        .into_iter()
        .try_fold(0_u64, |sum, id| sum.checked_add(*counts.get(&id)?));
        if counts.get(&EligibleEpisodes) != Some(&d.eligible)
            || counts.get(&AssessedEpisodes) != Some(&d.assessed)
            || counts.get(&UnassessedEpisodes) != Some(&d.unassessed)
            || counts.get(&ExplicitUnknownEpisodes) != Some(&d.explicit_unknown)
            || assessed_outcomes != Some(d.assessed)
            || d.eligible != expected_eligible
            || d.assessed != expected_assessed
            || d.unassessed != expected_eligible - expected_assessed
            || d.explicit_unknown != outcome_counts[3]
            || counts.get(&AcceptedEpisodes) != Some(&outcome_counts[0])
            || counts.get(&PartialEpisodes) != Some(&outcome_counts[1])
            || counts.get(&RejectedEpisodes) != Some(&outcome_counts[2])
            || counts.get(&OverlappingEpisodes) != Some(&expected_overlapping)
            || counts.get(&DistinctSnapshots) != Some(&expected_distinct)
            || card.coverage[0].observed != expected_assessed
            || card.coverage[0].eligible != expected_eligible
        {
            return Err(CardValidationError::InvalidCoverage);
        }
    }
    if card.question == InsightQuestionId::EstimatedCost
        && (card.rows[0].value.is_some()
            || !matches!(
                card.rows[0].missing_reason,
                Some(MissingReason::UsageNotPersisted | MissingReason::PricingUnavailable)
            ))
    {
        return Err(CardValidationError::InvalidRow);
    }
    let expected_evidence: Vec<_> = match card.question {
        InsightQuestionId::EpisodeOutcomes => request
            .episodes
            .iter()
            .flat_map(|episode| {
                episode
                    .members
                    .iter()
                    .map(|member| member.snapshot_id.clone())
            })
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        _ => request
            .evidence
            .iter()
            .map(|evidence| evidence.id.clone())
            .collect(),
    };
    let expected_episodes: Vec<_> = if card.question == InsightQuestionId::EpisodeOutcomes {
        request
            .episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect()
    } else {
        Vec::new()
    };
    if card.evidence_ids != expected_evidence || card.episode_ids != expected_episodes {
        return Err(CardValidationError::InvalidEvidence);
    }
    Ok(())
}

fn validate_required_limitations(
    card: &InsightCard,
    has_overlap: bool,
) -> Result<(), CardValidationError> {
    let expected: Vec<CardLimitation> = match card.question {
        InsightQuestionId::RecordedActivity => vec![
            CardLimitation::TimestampsAreRecordSpan,
            CardLimitation::RecordSpanIsNotActiveTime,
            CardLimitation::ToolFailuresAreNotRejections,
        ],
        InsightQuestionId::EpisodeOutcomes => {
            let mut values = vec![
                CardLimitation::UserSelectedEpisodeGroups,
                CardLimitation::EpisodesNotIndependentTasks,
            ];
            if has_overlap {
                values.push(CardLimitation::EpisodeGroupsOverlap);
            }
            values
        }
        InsightQuestionId::ObservedModels => {
            vec![CardLimitation::ModelDeclarationsAreObservedMetadata]
        }
        InsightQuestionId::EstimatedCost => {
            vec![CardLimitation::CostUnavailableWithoutPersistedUsageAndPricing]
        }
    };
    if card.limitations != expected {
        return Err(CardValidationError::InvalidRow);
    }
    Ok(())
}

pub const fn coverage_dictionary(question: InsightQuestionId) -> &'static [CoverageUnit] {
    use CoverageUnit::*;
    match question {
        InsightQuestionId::RecordedActivity => &[
            SavedSnapshots,
            NormalizedEvents,
            ToolCallEvents,
            ToolResults,
            TimestampEvidenceSnapshots,
            TimestampRecords,
        ],
        InsightQuestionId::EpisodeOutcomes => &[EpisodeGroups],
        InsightQuestionId::ObservedModels => &[SavedSnapshots, ModelRecords],
        InsightQuestionId::EstimatedCost => &[SavedSnapshots],
    }
}

pub const fn missingness_dictionary(row: CardRowId) -> &'static [MissingReason] {
    use CardRowId::*;
    use MissingReason::*;
    match row {
        EarliestRecordedAt | LatestRecordedAt | RecordSpan => {
            &[NoEligibleEvidence, NoObservedValue, InsufficientTimestamps]
        }
        NormalizedEvents | ToolCalls | ToolFailures | ObservedModel => {
            &[NoEligibleEvidence, NoObservedValue]
        }
        EstimatedCost => &[UsageNotPersisted, PricingUnavailable],
        _ => &[],
    }
}

fn ensure_refs<'a>(
    values: &[String],
    allowed: impl Iterator<Item = &'a str>,
) -> Result<(), CardValidationError> {
    let allowed: BTreeSet<_> = allowed.collect();
    let mut last = None;
    for value in values {
        if last.is_some_and(|v: &str| v >= value.as_str()) || !allowed.contains(value.as_str()) {
            return Err(CardValidationError::InvalidEvidence);
        }
        last = Some(value);
    }
    Ok(())
}

fn ensure_strict<T: Ord>(values: &[T]) -> Result<(), CardValidationError> {
    if values.windows(2).any(|v| v[0] >= v[1]) {
        return Err(CardValidationError::InvalidInput);
    }
    Ok(())
}
fn valid_model_label(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 96
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:".contains(&b))
}
fn valid_episode_id(v: &str) -> bool {
    uuid::Uuid::parse_str(v).is_ok_and(|id| {
        id.get_version_num() == 4
            && id.get_variant() == uuid::Variant::RFC4122
            && id.hyphenated().to_string() == v
    })
}
fn valid_digest(v: &str) -> bool {
    v.len() == 64
        && v.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn provider() -> ProviderManifest {
        ProviderManifest {
            id: "trace-commons-local".into(),
            version: "1".into(),
            rubric_version: INSIGHT_CARD_RUBRIC_VERSION.into(),
            execution_mode: ExecutionMode::Local,
            schema_version: 1,
        }
    }
    fn request() -> InsightCardRequest {
        let evidence = EvidenceRef {
            id: "a".repeat(64),
            source_digest: "b".repeat(64),
        };
        InsightCardRequest {
            schema_version: 1,
            expected_provider: provider(),
            questions: vec![InsightQuestionId::EpisodeOutcomes],
            evidence: vec![evidence.clone()],
            snapshots: vec![SnapshotCardInput {
                evidence: evidence.clone(),
                source_format: CardSourceFormat::Codex,
                normalized_events: CountFact {
                    value: Some(0),
                    observed: 0,
                    eligible: 0,
                },
                tool_calls: CountFact {
                    value: Some(0),
                    observed: 0,
                    eligible: 0,
                },
                tool_failures: CountFact {
                    value: None,
                    observed: 0,
                    eligible: 3,
                },
                models: vec![],
                model_labels_omitted: false,
                omitted_model_records: 0,
                model_eligible_records: 0,
                model_observed_records: 0,
                time: None,
            }],
            episodes: vec![],
        }
    }
    #[test]
    fn every_deterministic_value_and_coverage_is_bound_to_selected_facts() {
        let mut request = request();
        request.questions = InsightQuestionId::ALL.to_vec();
        request.snapshots[0].models = vec![ModelFact {
            label: "declared-model".into(),
            observed_records: 1,
        }];
        request.snapshots[0].model_eligible_records = 1;
        request.snapshots[0].model_observed_records = 1;
        let result = InsightCardResult {
            schema_version: 1,
            provider: request.expected_provider.clone(),
            input_digest: request.input_digest().unwrap(),
            cards: expected_cards(&request).unwrap(),
        };
        result.validate_for(&request).unwrap();
        for (card_index, card) in result.cards.iter().enumerate() {
            for (row_index, row) in card.rows.iter().enumerate() {
                if let Some(CardValue::Count(count)) = row.value {
                    let mut forged = result.clone();
                    forged.cards[card_index].rows[row_index].value =
                        Some(CardValue::Count(count + 1));
                    assert!(
                        forged.validate_for(&request).is_err(),
                        "accepted invented {:?}",
                        row.id
                    );
                }
            }
            for coverage_index in 0..card.coverage.len() {
                let mut forged = result.clone();
                forged.cards[card_index].coverage[coverage_index].eligible += 1;
                assert!(
                    forged.validate_for(&request).is_err(),
                    "accepted invented coverage"
                );
            }
        }
        let mut forged = result;
        let models = forged
            .cards
            .iter_mut()
            .find(|card| card.question == InsightQuestionId::ObservedModels)
            .unwrap();
        let model = models
            .rows
            .iter_mut()
            .find(|row| row.id == CardRowId::ObservedModel)
            .unwrap();
        model.label = Some("invented-model".into());
        assert!(forged.validate_for(&request).is_err());
    }

    fn result(req: &InsightCardRequest) -> InsightCardResult {
        let count_row = |id| CardRow {
            id,
            unit: row_dictionary(id).1,
            label: None,
            value: Some(CardValue::Count(0)),
            missing_reason: None,
        };
        InsightCardResult {
            schema_version: 1,
            provider: provider(),
            input_digest: req.input_digest().unwrap(),
            cards: vec![InsightCard {
                question: InsightQuestionId::EpisodeOutcomes,
                metric_version: "episode-outcomes-v1".into(),
                state: CardState::Unavailable,
                rows: vec![
                    CardRowId::EligibleEpisodes,
                    CardRowId::AssessedEpisodes,
                    CardRowId::AcceptedEpisodes,
                    CardRowId::PartialEpisodes,
                    CardRowId::RejectedEpisodes,
                    CardRowId::ExplicitUnknownEpisodes,
                    CardRowId::UnassessedEpisodes,
                    CardRowId::OverlappingEpisodes,
                    CardRowId::DistinctSnapshots,
                ]
                .into_iter()
                .map(count_row)
                .collect(),
                coverage: vec![CardCoverage {
                    unit: CoverageUnit::EpisodeGroups,
                    observed: 0,
                    eligible: 0,
                }],
                episode_denominator: Some(EpisodeDenominator {
                    eligible: 0,
                    assessed: 0,
                    unassessed: 0,
                    explicit_unknown: 0,
                }),
                evidence_ids: vec![],
                episode_ids: vec![],
                limitations: vec![
                    CardLimitation::UserSelectedEpisodeGroups,
                    CardLimitation::EpisodesNotIndependentTasks,
                ],
                rows_omitted: false,
            }],
        }
    }
    fn episode(
        id: &str,
        evidence: &EvidenceRef,
        outcome: Option<EpisodeOutcome>,
    ) -> EpisodeCardInput {
        let members = vec![EpisodeMemberInput {
            snapshot_id: evidence.id.clone(),
            source_digest: evidence.source_digest.clone(),
        }];
        let digest = episode_members_digest(&members).unwrap();
        EpisodeCardInput {
            id: id.into(),
            revision: 1,
            membership_revision: 1,
            members_digest: digest.clone(),
            members,
            assessment: outcome.map(|outcome| EpisodeAssessmentInput {
                category: EpisodeCategory::Unknown,
                outcome,
                provenance: EpisodeAssessmentProvenance::UserReported,
                membership_revision: 1,
                members_digest: digest,
            }),
        }
    }

    #[test]
    fn validates_exact_binding_and_deterministic_digest() {
        let r = request();
        let out = result(&r);
        out.validate_for(&r).unwrap();
        assert_eq!(
            r.input_digest().unwrap(),
            "0073b73df4448c8d7cc81e1cc27260527fb4b50dc9fce86775bf0982bbc4485c"
        );
    }
    #[test]
    fn rejects_provider_rubric_evidence_and_digest_mismatch() {
        let mut r = request();
        let mut out = result(&r);
        out.provider.rubric_version = "other".into();
        assert_eq!(
            out.validate_for(&r),
            Err(CardValidationError::IncompatibleProvider)
        );
        out = result(&r);
        out.input_digest = "c".repeat(64);
        assert_eq!(
            out.validate_for(&r),
            Err(CardValidationError::InvalidDigest)
        );
        r.snapshots[0].evidence.source_digest = "d".repeat(64);
        assert_eq!(r.validate(), Err(CardValidationError::InvalidEvidence));
    }
    #[test]
    fn rejects_missing_and_duplicate_cards() {
        let r = request();
        let mut out = result(&r);
        out.cards.clear();
        assert_eq!(out.validate_for(&r), Err(CardValidationError::MissingCard));
        let mut r = request();
        r.questions = vec![
            InsightQuestionId::RecordedActivity,
            InsightQuestionId::EpisodeOutcomes,
        ];
        let mut out = result(&r);
        out.input_digest = r.input_digest().unwrap();
        out.cards.push(out.cards[0].clone());
        assert_eq!(
            out.validate_for(&r),
            Err(CardValidationError::DuplicateCard)
        );
    }
    #[test]
    fn rejects_invalid_numeric_coverage() {
        let r = request();
        let mut out = result(&r);
        out.cards[0].coverage[0].observed = 1;
        assert_eq!(
            out.validate_for(&r),
            Err(CardValidationError::InvalidCoverage)
        );
    }
    #[test]
    fn unknown_and_zero_are_distinct() {
        let mut r = request();
        let unknown = serde_json::to_value(&r.snapshots[0].tool_failures).unwrap();
        r.snapshots[0].tool_failures = CountFact {
            value: Some(0),
            observed: 3,
            eligible: 3,
        };
        r.validate().unwrap();
        let zero = serde_json::to_value(&r.snapshots[0].tool_failures).unwrap();
        assert_ne!(zero, unknown);
    }
    #[test]
    fn explicit_unknown_and_absent_have_separate_denominators() {
        let mut r = request();
        let evidence = r.evidence[0].clone();
        r.episodes = vec![
            episode(
                "00000000-0000-4000-8000-000000000001",
                &evidence,
                Some(EpisodeOutcome::Accepted),
            ),
            episode(
                "00000000-0000-4000-8000-000000000002",
                &evidence,
                Some(EpisodeOutcome::Unknown),
            ),
            episode("00000000-0000-4000-8000-000000000003", &evidence, None),
        ];
        let mut out = result(&r);
        out.cards[0].episode_denominator = Some(EpisodeDenominator {
            eligible: 3,
            assessed: 2,
            unassessed: 1,
            explicit_unknown: 1,
        });
        out.cards[0].coverage[0] = CardCoverage {
            unit: CoverageUnit::EpisodeGroups,
            observed: 2,
            eligible: 3,
        };
        for row in &mut out.cards[0].rows {
            let value = match row.id {
                CardRowId::EligibleEpisodes => 3,
                CardRowId::AssessedEpisodes => 2,
                CardRowId::ExplicitUnknownEpisodes => 1,
                CardRowId::UnassessedEpisodes => 1,
                CardRowId::AcceptedEpisodes => 1,
                CardRowId::OverlappingEpisodes => 3,
                CardRowId::DistinctSnapshots => 1,
                _ => 0,
            };
            row.value = Some(CardValue::Count(value));
        }
        out.cards[0].state = CardState::Partial;
        out.cards[0]
            .limitations
            .push(CardLimitation::EpisodeGroupsOverlap);
        out.cards[0].evidence_ids = vec![evidence.id];
        out.cards[0].episode_ids = r
            .episodes
            .iter()
            .map(|episode| episode.id.clone())
            .collect();
        out.input_digest = r.input_digest().unwrap();
        out.validate_for(&r).unwrap();
        out.cards[0].limitations.pop();
        assert_eq!(out.validate_for(&r), Err(CardValidationError::InvalidRow));
        out.cards[0]
            .limitations
            .push(CardLimitation::EpisodeGroupsOverlap);
        out.cards[0]
            .episode_denominator
            .as_mut()
            .unwrap()
            .unassessed = 0;
        assert_eq!(
            out.validate_for(&r),
            Err(CardValidationError::InvalidCoverage)
        );
    }

    #[test]
    fn rejects_wrong_value_type_and_missing_episode_caution() {
        let r = request();
        let mut out = result(&r);
        out.cards[0].rows[0].value = Some(CardValue::UnixMilliseconds(0));
        assert_eq!(out.validate_for(&r), Err(CardValidationError::InvalidRow));
        let mut out = result(&r);
        out.cards[0].limitations.clear();
        assert_eq!(out.validate_for(&r), Err(CardValidationError::InvalidRow));
    }

    #[test]
    fn cost_card_cannot_claim_a_numeric_value() {
        let mut r = request();
        r.questions = vec![InsightQuestionId::EstimatedCost];
        let mut out = InsightCardResult {
            schema_version: 1,
            provider: provider(),
            input_digest: r.input_digest().unwrap(),
            cards: vec![InsightCard {
                question: InsightQuestionId::EstimatedCost,
                metric_version: "estimated-cost-v1".into(),
                state: CardState::Unavailable,
                rows: vec![CardRow {
                    id: CardRowId::EstimatedCost,
                    unit: CardUnit::UsDollars,
                    label: None,
                    value: None,
                    missing_reason: Some(MissingReason::PricingUnavailable),
                }],
                coverage: vec![CardCoverage {
                    unit: CoverageUnit::SavedSnapshots,
                    observed: 0,
                    eligible: 1,
                }],
                episode_denominator: None,
                evidence_ids: vec![r.evidence[0].id.clone()],
                episode_ids: vec![],
                limitations: vec![CardLimitation::CostUnavailableWithoutPersistedUsageAndPricing],
                rows_omitted: false,
            }],
        };
        out.validate_for(&r).unwrap();
        out.cards[0].rows[0].missing_reason = None;
        out.cards[0].rows[0].value = Some(CardValue::Count(1));
        assert_eq!(out.validate_for(&r), Err(CardValidationError::InvalidRow));
    }

    #[test]
    fn accepts_trajectory_jsonl_time_and_rejects_incomplete_equal_extrema() {
        let mut r = request();
        let at = DateTime::parse_from_rfc3339("2026-09-11T00:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let extremum = TimestampExtremum {
            recorded_at: at,
            event_refs: vec![TimestampEventRef { record_index: 2 }],
            omitted_event_refs: 0,
        };
        r.snapshots[0].source_format = CardSourceFormat::Trajectory;
        r.snapshots[0].time = Some(CardTimeEvidence {
            schema_version: 1,
            scope: TimeEvidenceScope::RecordedEventTimestampsOnly,
            source_format: CardSourceFormat::Trajectory,
            source_digest: r.evidence[0].source_digest.clone(),
            coordinates: TimeRecordCoordinates::JsonlPhysicalLinesOneBased,
            total_eligible_records: 2,
            valid_timestamps: 1,
            missing_timestamps: 1,
            invalid_timestamps: 0,
            earliest: Some(extremum.clone()),
            latest: Some(extremum),
        });
        r.validate().unwrap();
        r.snapshots[0]
            .time
            .as_mut()
            .unwrap()
            .latest
            .as_mut()
            .unwrap()
            .omitted_event_refs = 1;
        assert_eq!(r.validate(), Err(CardValidationError::InvalidInput));
    }

    #[test]
    fn expected_projection_orders_all_questions_and_keeps_unknowns_explicit() {
        let mut r = request();
        r.questions = InsightQuestionId::ALL.to_vec();
        let cards = expected_cards(&r).unwrap();
        assert_eq!(
            cards.iter().map(|card| card.question).collect::<Vec<_>>(),
            InsightQuestionId::ALL
        );
        assert_eq!(cards[0].state, CardState::Partial);
        assert_eq!(
            cards[0].rows[8].missing_reason,
            Some(MissingReason::NoEligibleEvidence)
        );
        assert_eq!(cards[1].state, CardState::Unavailable);
        assert_eq!(cards[2].state, CardState::Unavailable);
        assert_eq!(cards[3].state, CardState::Unavailable);
        let result = InsightCardResult {
            schema_version: 1,
            provider: provider(),
            input_digest: r.input_digest().unwrap(),
            cards,
        };
        result.validate_for(&r).unwrap();
    }

    #[test]
    fn expected_model_projection_bounds_union_and_marks_omission() {
        let mut r = request();
        r.questions = vec![InsightQuestionId::ObservedModels];
        r.evidence.clear();
        r.snapshots.clear();
        for snapshot_index in 0..3_u64 {
            let evidence = EvidenceRef {
                id: format!("{snapshot_index:064x}"),
                source_digest: format!("{:064x}", snapshot_index + 10),
            };
            let models = (0..32_u64)
                .map(|model_index| ModelFact {
                    label: format!("model-{:03}", snapshot_index * 32 + model_index),
                    observed_records: 1,
                })
                .collect();
            r.evidence.push(evidence.clone());
            r.snapshots.push(SnapshotCardInput {
                evidence,
                source_format: CardSourceFormat::Codex,
                normalized_events: CountFact {
                    value: Some(0),
                    observed: 0,
                    eligible: 0,
                },
                tool_calls: CountFact {
                    value: Some(0),
                    observed: 0,
                    eligible: 0,
                },
                tool_failures: CountFact {
                    value: Some(0),
                    observed: 0,
                    eligible: 0,
                },
                models,
                model_labels_omitted: false,
                omitted_model_records: 0,
                model_eligible_records: 32,
                model_observed_records: 32,
                time: None,
            });
        }
        let cards = expected_cards(&r).unwrap();
        assert_eq!(cards[0].rows.len(), MAX_CARD_ROWS);
        assert!(cards[0].rows_omitted);
        assert_eq!(cards[0].state, CardState::Partial);
        assert_eq!(cards[0].rows[0].label.as_deref(), Some("model-000"));
        assert_eq!(cards[0].rows[63].label.as_deref(), Some("model-063"));
    }

    #[test]
    fn repeated_model_reference_omission_does_not_require_label_omission() {
        let mut r = request();
        r.questions = vec![InsightQuestionId::ObservedModels];
        r.snapshots[0].models = vec![ModelFact {
            label: "one-model".into(),
            observed_records: 1,
        }];
        r.snapshots[0].model_labels_omitted = false;
        r.snapshots[0].omitted_model_records = 1;
        r.snapshots[0].model_eligible_records = 2;
        r.snapshots[0].model_observed_records = 2;
        r.validate().unwrap();
        let cards = expected_cards(&r).unwrap();
        assert_eq!(cards[0].rows.len(), 1);
        assert!(cards[0].rows_omitted);
        assert_eq!(cards[0].state, CardState::Partial);
    }

    #[test]
    fn expected_episode_projection_derives_overlap_and_unknown_denominators() {
        let mut r = request();
        let evidence = r.evidence[0].clone();
        r.episodes = vec![
            episode(
                "00000000-0000-4000-8000-000000000001",
                &evidence,
                Some(EpisodeOutcome::Rejected),
            ),
            episode(
                "00000000-0000-4000-8000-000000000002",
                &evidence,
                Some(EpisodeOutcome::Unknown),
            ),
            episode("00000000-0000-4000-8000-000000000003", &evidence, None),
        ];
        let card = expected_cards(&r).unwrap().remove(0);
        assert_eq!(card.state, CardState::Partial);
        assert_eq!(
            card.episode_denominator,
            Some(EpisodeDenominator {
                eligible: 3,
                assessed: 2,
                unassessed: 1,
                explicit_unknown: 1
            })
        );
        assert!(
            card.limitations
                .contains(&CardLimitation::EpisodeGroupsOverlap)
        );
        assert_eq!(card.rows[7].value, Some(CardValue::Count(3)));
        assert_eq!(card.rows[8].value, Some(CardValue::Count(1)));
    }
}
