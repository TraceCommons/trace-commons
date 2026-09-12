//! Shared vocabulary and text rendering for validated deterministic cards.
//! Native shells reuse copy keys and typed values without recomputing metrics.

use std::collections::BTreeMap;
use trace_commons_protocol::insights_cards::*;

pub fn question_copy(value: InsightQuestionId) -> (&'static str, &'static str) {
    match value {
        InsightQuestionId::RecordedActivity => {
            ("card_question_recorded_activity", "Recorded activity")
        }
        InsightQuestionId::EpisodeOutcomes => {
            ("card_question_episode_outcomes", "Your episode outcomes")
        }
        InsightQuestionId::ObservedModels => {
            ("card_question_observed_models", "Observed model labels")
        }
        InsightQuestionId::EstimatedCost => ("card_question_estimated_cost", "Estimated cost"),
    }
}

pub fn row_copy(value: CardRowId) -> (&'static str, &'static str) {
    match value {
        CardRowId::SavedSnapshots => ("card_row_saved_snapshots", "Saved snapshots"),
        CardRowId::NormalizedEvents => ("card_row_normalized_events", "Normalized events"),
        CardRowId::ToolCalls => ("card_row_tool_calls", "Observed tool calls"),
        CardRowId::ToolFailures => ("card_row_tool_failures", "Tool results reporting failure"),
        CardRowId::TimestampEligible => (
            "card_row_timestamp_eligible",
            "Records eligible for timestamp coverage",
        ),
        CardRowId::TimestampValid => ("card_row_timestamp_valid", "Records with valid timestamps"),
        CardRowId::TimestampMissing => ("card_row_timestamp_missing", "Records missing timestamps"),
        CardRowId::TimestampInvalid => (
            "card_row_timestamp_invalid",
            "Records with invalid timestamps",
        ),
        CardRowId::EarliestRecordedAt => {
            ("card_row_earliest_recorded_at", "Earliest recorded event")
        }
        CardRowId::LatestRecordedAt => ("card_row_latest_recorded_at", "Latest recorded event"),
        CardRowId::RecordSpan => ("card_row_record_span", "Span between recorded events"),
        CardRowId::EligibleEpisodes => ("card_row_eligible_episodes", "Selected episode groups"),
        CardRowId::AssessedEpisodes => ("card_row_assessed_episodes", "Groups assessed by you"),
        CardRowId::AcceptedEpisodes => ("card_row_accepted_episodes", "Reported accepted"),
        CardRowId::PartialEpisodes => ("card_row_partial_episodes", "Reported partial"),
        CardRowId::RejectedEpisodes => ("card_row_rejected_episodes", "Reported rejected"),
        CardRowId::ExplicitUnknownEpisodes => (
            "card_row_explicit_unknown_episodes",
            "Explicitly assessed as unknown",
        ),
        CardRowId::UnassessedEpisodes => (
            "card_row_unassessed_episodes",
            "Groups without an assessment",
        ),
        CardRowId::OverlappingEpisodes => {
            ("card_row_overlapping_episodes", "Groups sharing snapshots")
        }
        CardRowId::DistinctSnapshots => ("card_row_distinct_snapshots", "Distinct saved snapshots"),
        CardRowId::ObservedModel => ("card_row_observed_model", "Declared model"),
        CardRowId::EstimatedCost => ("card_row_estimated_cost", "Estimated cost"),
    }
}

pub fn coverage_copy(value: CoverageUnit) -> (&'static str, &'static str) {
    match value {
        CoverageUnit::SavedSnapshots => ("card_coverage_saved_snapshots", "Saved snapshots"),
        CoverageUnit::NormalizedEvents => ("card_coverage_normalized_events", "Normalized events"),
        CoverageUnit::ToolCallEvents => (
            "card_coverage_tool_call_events",
            "Events with recognized activity",
        ),
        CoverageUnit::ToolResults => (
            "card_coverage_tool_results",
            "Tool results with explicit outcomes",
        ),
        CoverageUnit::EpisodeGroups => ("card_coverage_episode_groups", "Episode groups"),
        CoverageUnit::TimestampEvidenceSnapshots => (
            "card_coverage_timestamp_evidence_snapshots",
            "Snapshots with timestamp evidence",
        ),
        CoverageUnit::TimestampRecords => (
            "card_coverage_timestamp_records",
            "Source records with valid timestamps",
        ),
        CoverageUnit::ModelRecords => (
            "card_coverage_model_records",
            "Supported model metadata records",
        ),
    }
}

pub fn missing_copy(value: MissingReason) -> (&'static str, &'static str) {
    match value {
        MissingReason::NoEligibleEvidence => (
            "card_missing_no_eligible_evidence",
            "No eligible evidence in this selection.",
        ),
        MissingReason::NoObservedValue => (
            "card_missing_no_observed_value",
            "The selected evidence does not provide this value.",
        ),
        MissingReason::InsufficientTimestamps => (
            "card_missing_insufficient_timestamps",
            "At least two recorded timestamps are needed for a span.",
        ),
        MissingReason::UsageNotPersisted => (
            "card_missing_usage_not_persisted",
            "Saved usage evidence is not available.",
        ),
        MissingReason::PricingUnavailable => (
            "card_missing_pricing_unavailable",
            "Applicable versioned pricing evidence is not available.",
        ),
    }
}

pub fn limitation_copy(value: CardLimitation) -> (&'static str, &'static str) {
    match value {
        CardLimitation::UserSelectedEpisodeGroups => (
            "card_limitation_user_selected_episode_groups",
            "These outcomes are your reports about groups of snapshots you selected.",
        ),
        CardLimitation::EpisodesNotIndependentTasks => (
            "card_limitation_episodes_not_independent_tasks",
            "Episode groups do not establish independent tasks.",
        ),
        CardLimitation::EpisodeGroupsOverlap => (
            "card_limitation_episode_groups_overlap",
            "Some selected groups share snapshots; their outcomes are dependent descriptions.",
        ),
        CardLimitation::TimestampsAreRecordSpan => (
            "card_limitation_timestamps_are_record_span",
            "The span runs from the earliest to the latest recorded event and can include idle gaps.",
        ),
        CardLimitation::RecordSpanIsNotActiveTime => (
            "card_limitation_record_span_is_not_active_time",
            "A recorded span does not measure active effort or time saved.",
        ),
        CardLimitation::ToolFailuresAreNotRejections => (
            "card_limitation_tool_failures_are_not_rejections",
            "A failed tool result does not establish rejected code or a failed task.",
        ),
        CardLimitation::ModelDeclarationsAreObservedMetadata => (
            "card_limitation_model_declarations_are_observed_metadata",
            "Model names come from trace metadata; they do not attribute work, cost, or outcomes.",
        ),
        CardLimitation::CostUnavailableWithoutPersistedUsageAndPricing => (
            "card_limitation_cost_unavailable_without_persisted_usage_and_pricing",
            "Cost needs saved usage, model attribution, and applicable versioned pricing.",
        ),
    }
}

pub fn state_copy(value: CardState) -> (&'static str, &'static str) {
    match value {
        CardState::Observed => ("card_state_observed", "Observed"),
        CardState::Partial => ("card_state_partial", "Partial coverage"),
        CardState::Unavailable => ("card_state_unavailable", "Unavailable"),
    }
}

pub fn ui_copy() -> BTreeMap<String, String> {
    let mut copy = BTreeMap::new();
    copy.insert("card_title".into(), "Questions about saved evidence".into());
    copy.insert(
        "card_selection_notice".into(),
        "Choose saved snapshots and episode groups to include. An empty selection has no evidence."
            .into(),
    );
    copy.insert("card_update".into(), "Update cards".into());
    copy.insert("card_choose_evidence".into(), "Choose evidence".into());
    for value in [
        InsightQuestionId::RecordedActivity,
        InsightQuestionId::EpisodeOutcomes,
        InsightQuestionId::ObservedModels,
        InsightQuestionId::EstimatedCost,
    ] {
        let (key, text) = question_copy(value);
        copy.insert(key.into(), text.into());
    }
    for value in [
        CardRowId::SavedSnapshots,
        CardRowId::NormalizedEvents,
        CardRowId::ToolCalls,
        CardRowId::ToolFailures,
        CardRowId::TimestampEligible,
        CardRowId::TimestampValid,
        CardRowId::TimestampMissing,
        CardRowId::TimestampInvalid,
        CardRowId::EarliestRecordedAt,
        CardRowId::LatestRecordedAt,
        CardRowId::RecordSpan,
        CardRowId::EligibleEpisodes,
        CardRowId::AssessedEpisodes,
        CardRowId::AcceptedEpisodes,
        CardRowId::PartialEpisodes,
        CardRowId::RejectedEpisodes,
        CardRowId::ExplicitUnknownEpisodes,
        CardRowId::UnassessedEpisodes,
        CardRowId::OverlappingEpisodes,
        CardRowId::DistinctSnapshots,
        CardRowId::ObservedModel,
        CardRowId::EstimatedCost,
    ] {
        let (key, text) = row_copy(value);
        copy.insert(key.into(), text.into());
    }
    for value in [
        CoverageUnit::SavedSnapshots,
        CoverageUnit::NormalizedEvents,
        CoverageUnit::ToolCallEvents,
        CoverageUnit::ToolResults,
        CoverageUnit::EpisodeGroups,
        CoverageUnit::TimestampEvidenceSnapshots,
        CoverageUnit::TimestampRecords,
        CoverageUnit::ModelRecords,
    ] {
        let (key, text) = coverage_copy(value);
        copy.insert(key.into(), text.into());
    }
    for value in [
        MissingReason::NoEligibleEvidence,
        MissingReason::NoObservedValue,
        MissingReason::InsufficientTimestamps,
        MissingReason::UsageNotPersisted,
        MissingReason::PricingUnavailable,
    ] {
        let (key, text) = missing_copy(value);
        copy.insert(key.into(), text.into());
    }
    for value in [
        CardLimitation::UserSelectedEpisodeGroups,
        CardLimitation::EpisodesNotIndependentTasks,
        CardLimitation::EpisodeGroupsOverlap,
        CardLimitation::TimestampsAreRecordSpan,
        CardLimitation::RecordSpanIsNotActiveTime,
        CardLimitation::ToolFailuresAreNotRejections,
        CardLimitation::ModelDeclarationsAreObservedMetadata,
        CardLimitation::CostUnavailableWithoutPersistedUsageAndPricing,
    ] {
        let (key, text) = limitation_copy(value);
        copy.insert(key.into(), text.into());
    }
    for value in [
        CardState::Observed,
        CardState::Partial,
        CardState::Unavailable,
    ] {
        let (key, text) = state_copy(value);
        copy.insert(key.into(), text.into());
    }
    copy.insert(
        "card_coverage".into(),
        "Coverage (observed / eligible)".into(),
    );
    copy.insert("card_evidence".into(), "Saved snapshot evidence".into());
    copy.insert("card_episodes".into(), "Episode evidence".into());
    copy.insert(
        "card_more_models".into(),
        "Some model labels or declaration references were omitted by limits.".into(),
    );
    copy
}

/// Validate against the host-selected input before rendering any provider text.
pub fn render_text(
    result: &InsightCardResult,
    request: &InsightCardRequest,
) -> Result<String, CardValidationError> {
    result.validate_for(request)?;
    let mut lines = vec![format!(
        "Analyzer: {} / {}",
        result.provider.id, result.provider.rubric_version
    )];
    for card in &result.cards {
        lines.push(String::new());
        lines.push(format!(
            "{} — {}",
            question_copy(card.question).1,
            state_copy(card.state).1
        ));
        for row in &card.rows {
            let value = render_value(row)?;
            let label = row
                .label
                .as_deref()
                .map(|label| format!(" ({label})"))
                .unwrap_or_default();
            lines.push(format!("{}{}: {}", row_copy(row.id).1, label, value));
        }
        lines.push("Coverage (observed / eligible):".into());
        for coverage in &card.coverage {
            lines.push(format!(
                "  {}: {} / {}",
                coverage_copy(coverage.unit).1,
                coverage.observed,
                coverage.eligible
            ));
        }
        for limitation in &card.limitations {
            lines.push(limitation_copy(*limitation).1.into());
        }
        if card.rows_omitted {
            lines
                .push("Some model labels or declaration references were omitted by limits.".into());
        }
        if !card.evidence_ids.is_empty() {
            lines.push(format!(
                "Saved snapshot evidence: {}",
                card.evidence_ids.join(", ")
            ));
        }
        if !card.episode_ids.is_empty() {
            lines.push(format!("Episode evidence: {}", card.episode_ids.join(", ")));
        }
    }
    Ok(lines.join("\n"))
}

fn render_value(row: &CardRow) -> Result<String, CardValidationError> {
    Ok(match &row.value {
        Some(CardValue::Count(value)) => value.to_string(),
        Some(CardValue::Milliseconds(value)) => format!("{value} ms"),
        Some(CardValue::UnixMilliseconds(value)) => chrono::DateTime::from_timestamp_millis(*value)
            .ok_or(CardValidationError::InvalidRow)?
            .to_rfc3339(),
        None => missing_copy(row.missing_reason.ok_or(CardValidationError::InvalidRow)?)
            .1
            .into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_selection_renders_all_questions_and_rejects_forged_counts() {
        let mut provider = trace_commons_protocol::insights::ProviderManifest::first_party();
        provider.rubric_version = INSIGHT_CARD_RUBRIC_VERSION.into();
        let request = InsightCardRequest {
            schema_version: 1,
            expected_provider: provider.clone(),
            questions: InsightQuestionId::ALL.to_vec(),
            evidence: vec![],
            snapshots: vec![],
            episodes: vec![],
        };
        let result = InsightCardResult {
            schema_version: 1,
            provider,
            input_digest: request.input_digest().unwrap(),
            cards: expected_cards(&request).unwrap(),
        };
        let text = render_text(&result, &request).unwrap();
        for question in InsightQuestionId::ALL {
            assert!(text.contains(question_copy(question).1));
        }
        assert!(text.contains("Applicable versioned pricing evidence is not available."));
        assert!(text.contains("Episode groups do not establish independent tasks."));
        let mut forged = result;
        forged.cards[0].rows[0].value = Some(CardValue::Count(99));
        assert!(render_text(&forged, &request).is_err());
    }

    #[test]
    fn timestamps_preserve_milliseconds_and_refuse_out_of_range_values() {
        let mut row = CardRow {
            id: CardRowId::EarliestRecordedAt,
            unit: CardUnit::UnixMilliseconds,
            label: None,
            value: Some(CardValue::UnixMilliseconds(123)),
            missing_reason: None,
        };
        assert_eq!(render_value(&row).unwrap(), "1970-01-01T00:00:00.123+00:00");
        row.value = Some(CardValue::UnixMilliseconds(i64::MAX));
        assert_eq!(render_value(&row), Err(CardValidationError::InvalidRow));
    }

    #[test]
    fn unknown_cost_is_an_explanation_and_never_a_zero_value() {
        let mut row = CardRow {
            id: CardRowId::EstimatedCost,
            unit: CardUnit::UsDollars,
            label: None,
            value: None,
            missing_reason: Some(MissingReason::UsageNotPersisted),
        };
        assert_eq!(
            render_value(&row).unwrap(),
            "Saved usage evidence is not available."
        );
        row.missing_reason = None;
        assert_eq!(render_value(&row), Err(CardValidationError::InvalidRow));
    }
}
