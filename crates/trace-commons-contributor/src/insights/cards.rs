//! First-party local projection of validated, host-selected Insights card facts.

use trace_commons_protocol::insights::ExecutionMode;
use trace_commons_protocol::insights_cards::{
    CardValidationError, INSIGHT_CARD_RUBRIC_VERSION, INSIGHT_CARD_SCHEMA_VERSION,
    InsightCardRequest, InsightCardResult, expected_cards,
};

const FIRST_PARTY_PROVIDER_ID: &str = "trace-commons-local";
const FIRST_PARTY_PROVIDER_VERSION: &str = "1";

/// Safe fixed labels only. Provider inputs and validation details are never forwarded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum QuestionCardProjectionError {
    #[error("insights_card_request_invalid")]
    InvalidRequest,
    #[error("insights_card_provider_incompatible")]
    IncompatibleProvider,
    #[error("insights_card_projection_invalid")]
    InvalidProjection,
}

/// Project all requested cards through the curated first-party local capability.
pub fn project_first_party_question_cards(
    request: &InsightCardRequest,
) -> Result<InsightCardResult, QuestionCardProjectionError> {
    request
        .validate()
        .map_err(|_| QuestionCardProjectionError::InvalidRequest)?;
    let provider = &request.expected_provider;
    if provider.id != FIRST_PARTY_PROVIDER_ID
        || provider.version != FIRST_PARTY_PROVIDER_VERSION
        || provider.rubric_version != INSIGHT_CARD_RUBRIC_VERSION
        || provider.execution_mode != ExecutionMode::Local
        || provider.schema_version != 1
    {
        return Err(QuestionCardProjectionError::IncompatibleProvider);
    }
    let cards = expected_cards(request).map_err(map_projection_error)?;
    let result = InsightCardResult {
        schema_version: INSIGHT_CARD_SCHEMA_VERSION,
        provider: provider.clone(),
        input_digest: request
            .input_digest()
            .map_err(|_| QuestionCardProjectionError::InvalidRequest)?,
        cards,
    };
    result
        .validate_for(request)
        .map_err(|_| QuestionCardProjectionError::InvalidProjection)?;
    Ok(result)
}

fn map_projection_error(_: CardValidationError) -> QuestionCardProjectionError {
    QuestionCardProjectionError::InvalidProjection
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use sha2::{Digest, Sha256};
    use trace_commons_protocol::insights::{EvidenceRef, ProviderManifest};
    use trace_commons_protocol::insights_cards::{
        CardSourceFormat, CardState, CardTimeEvidence, CountFact, EpisodeAssessmentInput,
        EpisodeAssessmentProvenance, EpisodeCardInput, EpisodeCategory, EpisodeMemberInput,
        EpisodeOutcome, InsightQuestionId, ModelFact, SnapshotCardInput, TimeEvidenceScope,
        TimeRecordCoordinates, TimestampEventRef, TimestampExtremum, episode_members_digest,
    };

    use super::*;

    fn provider() -> ProviderManifest {
        ProviderManifest {
            id: FIRST_PARTY_PROVIDER_ID.into(),
            version: FIRST_PARTY_PROVIDER_VERSION.into(),
            rubric_version: INSIGHT_CARD_RUBRIC_VERSION.into(),
            execution_mode: ExecutionMode::Local,
            schema_version: 1,
        }
    }

    fn evidence(index: u64) -> EvidenceRef {
        EvidenceRef {
            id: format!("{index:064x}"),
            source_digest: format!("{:064x}", index + 100),
        }
    }

    fn empty_snapshot(evidence: EvidenceRef) -> SnapshotCardInput {
        SnapshotCardInput {
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
            models: Vec::new(),
            model_labels_omitted: false,
            omitted_model_records: 0,
            model_eligible_records: 0,
            model_observed_records: 0,
            time: None,
        }
    }

    fn request() -> InsightCardRequest {
        let evidence = evidence(1);
        InsightCardRequest {
            schema_version: 1,
            expected_provider: provider(),
            questions: InsightQuestionId::ALL.to_vec(),
            evidence: vec![evidence.clone()],
            snapshots: vec![empty_snapshot(evidence)],
            episodes: Vec::new(),
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
        let members_digest = episode_members_digest(&members).unwrap();
        EpisodeCardInput {
            id: id.into(),
            revision: 1,
            membership_revision: 1,
            members_digest: members_digest.clone(),
            members,
            assessment: outcome.map(|outcome| EpisodeAssessmentInput {
                category: EpisodeCategory::Unknown,
                outcome,
                provenance: EpisodeAssessmentProvenance::UserReported,
                membership_revision: 1,
                members_digest,
            }),
        }
    }

    #[test]
    fn all_question_projection_is_stable_and_partial_without_timestamps() {
        let request = request();
        let result = project_first_party_question_cards(&request).unwrap();
        assert_eq!(
            result
                .cards
                .iter()
                .map(|card| card.question)
                .collect::<Vec<_>>(),
            InsightQuestionId::ALL
        );
        assert_eq!(result.cards[0].state, CardState::Partial);
        assert_eq!(result.cards[1].state, CardState::Unavailable);
        assert_eq!(result.cards[2].state, CardState::Unavailable);
        assert_eq!(result.cards[3].state, CardState::Unavailable);
        let encoded = serde_json::to_vec(&result).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded)),
            "96c474acf1669b1cb18f2c1e4382b4997ce7fbb2c9f01c7a460eb5bcd402b630"
        );
    }

    #[test]
    fn timestamp_envelope_uses_checked_millisecond_span() {
        let mut request = request();
        let first = DateTime::parse_from_rfc3339("2026-09-11T00:00:00.123456789Z")
            .unwrap()
            .with_timezone(&Utc);
        let last = DateTime::parse_from_rfc3339("2026-09-11T00:00:01.124456789Z")
            .unwrap()
            .with_timezone(&Utc);
        request.snapshots[0].time = Some(CardTimeEvidence {
            schema_version: 1,
            scope: TimeEvidenceScope::RecordedEventTimestampsOnly,
            source_format: CardSourceFormat::Codex,
            source_digest: request.evidence[0].source_digest.clone(),
            coordinates: TimeRecordCoordinates::JsonlPhysicalLinesOneBased,
            total_eligible_records: 2,
            valid_timestamps: 2,
            missing_timestamps: 0,
            invalid_timestamps: 0,
            earliest: Some(TimestampExtremum {
                recorded_at: first,
                event_refs: vec![TimestampEventRef { record_index: 1 }],
                omitted_event_refs: 0,
            }),
            latest: Some(TimestampExtremum {
                recorded_at: last,
                event_refs: vec![TimestampEventRef { record_index: 2 }],
                omitted_event_refs: 0,
            }),
        });
        let result = project_first_party_question_cards(&request).unwrap();
        assert_eq!(
            result.cards[0].rows[10].value,
            Some(trace_commons_protocol::insights_cards::CardValue::Milliseconds(1001))
        );
    }

    #[test]
    fn model_union_is_bounded_and_source_omission_stays_visible() {
        let mut request = request();
        request.questions = vec![InsightQuestionId::ObservedModels];
        request.evidence.clear();
        request.snapshots.clear();
        for source in 0..3_u64 {
            let evidence = evidence(source + 1);
            let mut snapshot = empty_snapshot(evidence.clone());
            snapshot.models = (0..32_u64)
                .map(|index| ModelFact {
                    label: format!("model-{:03}", source * 32 + index),
                    observed_records: 1,
                })
                .collect();
            snapshot.model_eligible_records = 32;
            snapshot.model_observed_records = 32;
            request.evidence.push(evidence);
            request.snapshots.push(snapshot);
        }
        let result = project_first_party_question_cards(&request).unwrap();
        assert_eq!(result.cards[0].rows.len(), 64);
        assert!(result.cards[0].rows_omitted);
        assert_eq!(result.cards[0].state, CardState::Partial);
    }

    #[test]
    fn episode_projection_counts_overlap_unknown_and_absence_separately() {
        let mut request = request();
        request.questions = vec![InsightQuestionId::EpisodeOutcomes];
        let evidence = request.evidence[0].clone();
        request.episodes = vec![
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
        let result = project_first_party_question_cards(&request).unwrap();
        let denominator = result.cards[0].episode_denominator.as_ref().unwrap();
        assert_eq!(
            (
                denominator.assessed,
                denominator.unassessed,
                denominator.explicit_unknown
            ),
            (2, 1, 1)
        );
        assert_eq!(
            result.cards[0].rows[7].value,
            Some(trace_commons_protocol::insights_cards::CardValue::Count(3))
        );
    }

    #[test]
    fn rejects_non_first_party_capability_with_safe_error() {
        let mut request = request();
        request.expected_provider.id = "other-local".into();
        assert_eq!(
            project_first_party_question_cards(&request),
            Err(QuestionCardProjectionError::IncompatibleProvider)
        );
        request.expected_provider.rubric_version = "wrong".into();
        assert_eq!(
            project_first_party_question_cards(&request),
            Err(QuestionCardProjectionError::InvalidRequest)
        );
    }
}
