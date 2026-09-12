//! First-party local projection of validated, host-selected Insights card facts.

use trace_commons_protocol::insights::{ExecutionMode, ProviderManifest};
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
    #[error("insights_card_provider_unavailable")]
    Unavailable,
}

/// A separate capability from descriptive report evaluation. Implementations
/// must perform no external IO and return only payload-free errors. The host
/// selects trusted implementations; this in-process seam is not a sandbox or
/// a product installation, selection, or remote-execution mechanism.
pub trait LocalQuestionCardProvider {
    fn manifest(&self) -> ProviderManifest;
    fn evaluate_cards(
        &self,
        request: &InsightCardRequest,
    ) -> Result<InsightCardResult, QuestionCardProjectionError>;
}

/// Validate the host-selected request before invoking a provider, then bind its
/// complete result to that request and the shared deterministic rubric.
pub fn dispatch_question_cards(
    provider: &dyn LocalQuestionCardProvider,
    request: &InsightCardRequest,
) -> Result<InsightCardResult, QuestionCardProjectionError> {
    request
        .validate()
        .map_err(|_| QuestionCardProjectionError::InvalidRequest)?;
    if provider.manifest() != request.expected_provider {
        return Err(QuestionCardProjectionError::IncompatibleProvider);
    }
    let result = provider.evaluate_cards(request)?;
    result
        .validate_for(request)
        .map_err(|_| QuestionCardProjectionError::InvalidProjection)?;
    Ok(result)
}

pub struct FirstPartyQuestionCardProvider;

impl LocalQuestionCardProvider for FirstPartyQuestionCardProvider {
    fn manifest(&self) -> ProviderManifest {
        ProviderManifest {
            id: FIRST_PARTY_PROVIDER_ID.into(),
            version: FIRST_PARTY_PROVIDER_VERSION.into(),
            rubric_version: INSIGHT_CARD_RUBRIC_VERSION.into(),
            execution_mode: ExecutionMode::Local,
            schema_version: 1,
        }
    }

    fn evaluate_cards(
        &self,
        request: &InsightCardRequest,
    ) -> Result<InsightCardResult, QuestionCardProjectionError> {
        request
            .validate()
            .map_err(|_| QuestionCardProjectionError::InvalidRequest)?;
        if request.expected_provider != self.manifest() {
            return Err(QuestionCardProjectionError::IncompatibleProvider);
        }
        Ok(InsightCardResult {
            schema_version: INSIGHT_CARD_SCHEMA_VERSION,
            provider: self.manifest(),
            input_digest: request
                .input_digest()
                .map_err(|_| QuestionCardProjectionError::InvalidRequest)?,
            cards: expected_cards(request).map_err(map_projection_error)?,
        })
    }
}

/// Project all requested cards through the curated first-party local capability.
pub fn project_first_party_question_cards(
    request: &InsightCardRequest,
) -> Result<InsightCardResult, QuestionCardProjectionError> {
    dispatch_question_cards(&FirstPartyQuestionCardProvider, request)
}

fn map_projection_error(_: CardValidationError) -> QuestionCardProjectionError {
    QuestionCardProjectionError::InvalidProjection
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use sha2::{Digest, Sha256};
    use std::cell::Cell;
    use trace_commons_protocol::insights::{EvidenceRef, ProviderManifest};
    use trace_commons_protocol::insights_cards::{
        CardSourceFormat, CardState, CardTimeEvidence, CountFact, EpisodeAssessmentInput,
        EpisodeAssessmentProvenance, EpisodeCardInput, EpisodeCategory, EpisodeMemberInput,
        EpisodeOutcome, InsightQuestionId, ModelFact, SnapshotCardInput, TimeEvidenceScope,
        TimeRecordCoordinates, TimestampEventRef, TimestampExtremum, episode_members_digest,
    };

    use super::*;

    struct TestProvider {
        manifest: ProviderManifest,
        calls: Cell<u32>,
        alter: fn(&mut InsightCardResult),
        fail: bool,
    }

    impl LocalQuestionCardProvider for TestProvider {
        fn manifest(&self) -> ProviderManifest {
            self.manifest.clone()
        }
        fn evaluate_cards(
            &self,
            request: &InsightCardRequest,
        ) -> Result<InsightCardResult, QuestionCardProjectionError> {
            self.calls.set(self.calls.get() + 1);
            if self.fail {
                return Err(QuestionCardProjectionError::Unavailable);
            }
            let mut result = InsightCardResult {
                schema_version: INSIGHT_CARD_SCHEMA_VERSION,
                provider: self.manifest(),
                input_digest: request.input_digest().unwrap(),
                cards: expected_cards(request).unwrap(),
            };
            (self.alter)(&mut result);
            Ok(result)
        }
    }

    #[test]
    fn alternate_local_provider_preserves_the_common_card_contract() {
        let mut request = request();
        request.expected_provider.id = "another-local-analyzer".into();
        let provider = TestProvider {
            manifest: request.expected_provider.clone(),
            calls: Cell::new(0),
            alter: |_| {},
            fail: false,
        };
        let result = dispatch_question_cards(&provider, &request).unwrap();
        assert_eq!(provider.calls.get(), 1);
        assert_eq!(result.provider.id, "another-local-analyzer");
        assert_eq!(result.cards, expected_cards(&request).unwrap());
        result.validate_for(&request).unwrap();
    }

    #[test]
    fn bad_requests_and_manifest_mismatch_do_not_invoke_the_provider() {
        let mut request = request();
        let provider = TestProvider {
            manifest: request.expected_provider.clone(),
            calls: Cell::new(0),
            alter: |_| {},
            fail: false,
        };
        request.questions.push(request.questions[0]);
        assert_eq!(
            dispatch_question_cards(&provider, &request),
            Err(QuestionCardProjectionError::InvalidRequest)
        );
        request.questions.pop();
        request.expected_provider.id = "another-local-analyzer".into();
        assert_eq!(
            dispatch_question_cards(&provider, &request),
            Err(QuestionCardProjectionError::IncompatibleProvider)
        );
        assert_eq!(provider.calls.get(), 0);
    }

    #[test]
    fn provider_failures_and_forged_results_cannot_escape_dispatch() {
        let request = request();
        let mut provider = TestProvider {
            manifest: request.expected_provider.clone(),
            calls: Cell::new(0),
            alter: |_| {},
            fail: true,
        };
        assert_eq!(
            dispatch_question_cards(&provider, &request),
            Err(QuestionCardProjectionError::Unavailable)
        );
        provider.fail = false;
        let mutations: [fn(&mut InsightCardResult); 5] = [
            |r| r.input_digest = "f".repeat(64),
            |r| r.provider.id = "impostor".into(),
            |r| {
                r.cards.pop();
            },
            |r| r.cards[0].evidence_ids.push("f".repeat(64)),
            |r| {
                r.cards[0].rows[0].value = Some(
                    trace_commons_protocol::insights_cards::CardValue::Count(999),
                )
            },
        ];
        for mutate in mutations {
            provider.alter = mutate;
            assert_eq!(
                dispatch_question_cards(&provider, &request),
                Err(QuestionCardProjectionError::InvalidProjection)
            );
        }
    }

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
        assert_eq!(
            result.cards[3].rows[0].missing_reason,
            Some(trace_commons_protocol::insights_cards::MissingReason::PricingUnavailable)
        );
        let encoded = serde_json::to_vec(&result).unwrap();
        assert_eq!(
            format!("{:x}", Sha256::digest(encoded)),
            "596b6560fc83d69819e1701861d61ea982da34b291833194e87b086f47536fd3"
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
