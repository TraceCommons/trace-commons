//! Resolve deterministic card inputs from one validated local-store snapshot.
//!
//! This module reads only persisted metadata. It never reopens imported source
//! files, and episode assessments remain separate user-reported evidence.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use trace_commons_protocol::insights::{ExecutionMode, MetricId, ProviderManifest};
use trace_commons_protocol::insights_cards::{
    CardSourceFormat, CardTimeEvidence, CountFact, EpisodeAssessmentInput,
    EpisodeAssessmentProvenance, EpisodeCardInput, EpisodeCategory, EpisodeMemberInput,
    EpisodeOutcome, INSIGHT_CARD_RUBRIC_VERSION, INSIGHT_CARD_SCHEMA_VERSION, InsightCardRequest,
    InsightQuestionId, MAX_CARD_EPISODES, MAX_CARD_EVIDENCE, ModelFact, SnapshotCardInput,
    TimeEvidenceScope, TimeRecordCoordinates, TimestampEventRef, TimestampExtremum,
};

use super::{
    AnnotationProvenance, LocalInsight, LocalInsightStore, SourceFormat, TaskCategory, TaskOutcome,
    episodes, models, time_evidence,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum CardStoreError {
    #[error("insights_card_invalid_selection")]
    InvalidSelection,
    #[error("insights_card_snapshot_limit")]
    SnapshotLimit,
    #[error("insights_card_snapshot_not_found")]
    MissingSnapshot,
    #[error("insights_card_episode_limit")]
    EpisodeLimit,
    #[error("insights_card_episode_not_found")]
    MissingEpisode,
}

/// Validate caller-controlled selection before resolving or opening storage.
pub fn validate_card_selection(
    questions: &[InsightQuestionId],
    snapshot_ids: &[String],
    episode_ids: &[String],
) -> std::result::Result<(), CardStoreError> {
    canonical_questions(questions)?;
    validate_snapshot_selection(snapshot_ids)?;
    validate_episode_selection(episode_ids)?;
    Ok(())
}

/// Construct the read-only response input for an absent store and an empty
/// evidence selection without creating a directory or lock file.
pub fn empty_card_request(
    questions: &[InsightQuestionId],
) -> std::result::Result<InsightCardRequest, CardStoreError> {
    let request = InsightCardRequest {
        schema_version: INSIGHT_CARD_SCHEMA_VERSION,
        expected_provider: card_provider(),
        questions: canonical_questions(questions)?,
        evidence: Vec::new(),
        snapshots: Vec::new(),
        episodes: Vec::new(),
    };
    request
        .validate()
        .map_err(|_| CardStoreError::InvalidSelection)?;
    Ok(request)
}

impl LocalInsightStore {
    /// Resolve explicit snapshots plus every selected episode member as one
    /// canonical union while holding a single store lock.
    pub fn resolve_card_request(
        &self,
        questions: &[InsightQuestionId],
        snapshot_ids: &[String],
        episode_ids: &[String],
    ) -> Result<InsightCardRequest> {
        validate_card_selection(questions, snapshot_ids, episode_ids)?;
        let questions = canonical_questions(questions)?;

        let (_lock, index) = self.locked()?;
        let mut resolved_ids: BTreeSet<String> = snapshot_ids.iter().cloned().collect();
        let mut selected_episodes = Vec::with_capacity(episode_ids.len());
        for id in episode_ids {
            let episode = index
                .episodes
                .get(id)
                .ok_or(CardStoreError::MissingEpisode)?;
            for member in &episode.members {
                resolved_ids.insert(member.snapshot_id.clone());
            }
            selected_episodes.push(episode);
        }
        if resolved_ids.len() > MAX_CARD_EVIDENCE {
            return Err(CardStoreError::SnapshotLimit.into());
        }
        for id in snapshot_ids {
            if !index.reports.contains_key(id) {
                return Err(CardStoreError::MissingSnapshot.into());
            }
        }

        let snapshots = resolved_ids
            .iter()
            .map(|id| {
                index
                    .reports
                    .get(id)
                    .ok_or(CardStoreError::MissingSnapshot.into())
                    .and_then(snapshot_input)
            })
            .collect::<Result<Vec<_>>>()?;
        let evidence = snapshots
            .iter()
            .map(|snapshot| snapshot.evidence.clone())
            .collect();
        selected_episodes.sort_by(|left, right| left.id.cmp(&right.id));
        let episodes = selected_episodes
            .into_iter()
            .map(episode_input)
            .collect::<Result<Vec<_>>>()?;
        let request = InsightCardRequest {
            schema_version: INSIGHT_CARD_SCHEMA_VERSION,
            expected_provider: card_provider(),
            questions,
            evidence,
            snapshots,
            episodes,
        };
        request
            .validate()
            .map_err(|_| CardStoreError::InvalidSelection)?;
        Ok(request)
    }
}

fn card_provider() -> ProviderManifest {
    ProviderManifest {
        id: "trace-commons-local".into(),
        version: "1".into(),
        rubric_version: INSIGHT_CARD_RUBRIC_VERSION.into(),
        execution_mode: ExecutionMode::Local,
        schema_version: trace_commons_protocol::insights::INSIGHT_SCHEMA_VERSION,
    }
}

fn canonical_questions(
    questions: &[InsightQuestionId],
) -> std::result::Result<Vec<InsightQuestionId>, CardStoreError> {
    if questions.is_empty() {
        return Err(CardStoreError::InvalidSelection);
    }
    if questions.len() > InsightQuestionId::ALL.len() {
        return Err(CardStoreError::InvalidSelection);
    }
    let unique: BTreeSet<_> = questions.iter().copied().collect();
    if unique.len() != questions.len() {
        return Err(CardStoreError::InvalidSelection);
    }
    Ok(unique.into_iter().collect())
}

fn validate_snapshot_selection(ids: &[String]) -> std::result::Result<(), CardStoreError> {
    if ids.len() > MAX_CARD_EVIDENCE {
        return Err(CardStoreError::SnapshotLimit);
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        if !is_digest(id) {
            return Err(CardStoreError::InvalidSelection);
        }
        if !unique.insert(id) {
            return Err(CardStoreError::InvalidSelection);
        }
    }
    Ok(())
}

fn validate_episode_selection(ids: &[String]) -> std::result::Result<(), CardStoreError> {
    if ids.len() > MAX_CARD_EPISODES {
        return Err(CardStoreError::EpisodeLimit);
    }
    let mut unique = BTreeSet::new();
    for id in ids {
        episodes::validate_episode_id(id).map_err(|_| CardStoreError::InvalidSelection)?;
        if !unique.insert(id) {
            return Err(CardStoreError::InvalidSelection);
        }
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn snapshot_input(insight: &LocalInsight) -> Result<SnapshotCardInput> {
    let models = model_input(insight.model_observations.as_ref());
    Ok(SnapshotCardInput {
        evidence: insight.report.evidence[0].clone(),
        source_format: source_format(insight.source_format),
        normalized_events: metric_fact(insight, MetricId::Events)?,
        tool_calls: metric_fact(insight, MetricId::ToolCalls)?,
        tool_failures: metric_fact(insight, MetricId::ToolFailures)?,
        models: models.0,
        model_labels_omitted: models.1,
        omitted_model_records: models.2,
        model_eligible_records: models.3,
        model_observed_records: models.4,
        time: insight.time_evidence.as_ref().map(time_input),
    })
}

fn metric_fact(insight: &LocalInsight, id: MetricId) -> Result<CountFact> {
    let metric = insight
        .report
        .metrics
        .iter()
        .find(|metric| metric.id == id)
        .ok_or_else(|| anyhow!("insights_store_invalid"))?;
    Ok(CountFact {
        value: metric.value,
        observed: metric.coverage.observed,
        eligible: metric.coverage.total,
    })
}

fn model_input(
    observation: Option<&models::ModelObservations>,
) -> (Vec<ModelFact>, bool, u64, u64, u64) {
    let Some(observation) = observation else {
        return (Vec::new(), false, 0, 0, 0);
    };
    let mut counts = BTreeMap::<&str, u64>::new();
    for declaration in &observation.declarations {
        *counts.entry(&declaration.model).or_default() += 1;
    }
    let facts = counts
        .into_iter()
        .map(|(label, observed_records)| ModelFact {
            label: label.into(),
            observed_records,
        })
        .collect();
    (
        facts,
        observation.model_labels_omitted,
        observation.omitted_declarations,
        observation.candidate_records,
        observation.valid_declarations,
    )
}

fn source_format(value: SourceFormat) -> CardSourceFormat {
    match value {
        SourceFormat::Codex => CardSourceFormat::Codex,
        SourceFormat::ClaudeCode => CardSourceFormat::ClaudeCode,
        SourceFormat::Trajectory => CardSourceFormat::Trajectory,
    }
}

fn time_input(value: &time_evidence::RecordedTimeEvidence) -> CardTimeEvidence {
    CardTimeEvidence {
        schema_version: value.schema_version,
        scope: match value.scope {
            time_evidence::TimeEvidenceScope::RecordedEventTimestampsOnly => {
                TimeEvidenceScope::RecordedEventTimestampsOnly
            }
        },
        source_format: source_format(value.source_format),
        source_digest: value.source_digest.clone(),
        coordinates: match value.coordinates {
            time_evidence::TimeRecordCoordinates::JsonlPhysicalLinesOneBased => {
                TimeRecordCoordinates::JsonlPhysicalLinesOneBased
            }
            time_evidence::TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased => {
                TimeRecordCoordinates::TrajectoryArrayIndexesZeroBased
            }
        },
        total_eligible_records: value.total_eligible_records,
        valid_timestamps: value.valid_timestamps,
        missing_timestamps: value.missing_timestamps,
        invalid_timestamps: value.invalid_timestamps,
        earliest: value.earliest.as_ref().map(extremum_input),
        latest: value.latest.as_ref().map(extremum_input),
    }
}

fn extremum_input(value: &time_evidence::TimestampExtremum) -> TimestampExtremum {
    TimestampExtremum {
        recorded_at: value.recorded_at,
        event_refs: value
            .event_refs
            .iter()
            .map(|reference| TimestampEventRef {
                record_index: reference.record_index,
            })
            .collect(),
        omitted_event_refs: value.omitted_event_refs,
    }
}

fn episode_input(value: &episodes::LocalEpisode) -> Result<EpisodeCardInput> {
    let members = value
        .members
        .iter()
        .map(|member| EpisodeMemberInput {
            snapshot_id: member.snapshot_id.clone(),
            source_digest: member.source_digest.clone(),
        })
        .collect::<Vec<_>>();
    let members_digest = episodes::members_digest(&value.members)?;
    let assessment = value
        .manual_assessment
        .as_ref()
        .map(|assessment| EpisodeAssessmentInput {
            category: category(assessment.category),
            outcome: outcome(assessment.outcome),
            provenance: match assessment.provenance {
                AnnotationProvenance::UserReported => EpisodeAssessmentProvenance::UserReported,
            },
            membership_revision: assessment.membership_revision,
            members_digest: assessment.members_digest.clone(),
        });
    Ok(EpisodeCardInput {
        id: value.id.clone(),
        revision: value.revision,
        membership_revision: value.membership_revision,
        members_digest,
        members,
        assessment,
    })
}

fn category(value: TaskCategory) -> EpisodeCategory {
    match value {
        TaskCategory::Refactor => EpisodeCategory::Refactor,
        TaskCategory::Tests => EpisodeCategory::Tests,
        TaskCategory::Docs => EpisodeCategory::Docs,
        TaskCategory::Debugging => EpisodeCategory::Debugging,
        TaskCategory::Other => EpisodeCategory::Other,
        TaskCategory::Unknown => EpisodeCategory::Unknown,
    }
}

fn outcome(value: TaskOutcome) -> EpisodeOutcome {
    match value {
        TaskOutcome::Accepted => EpisodeOutcome::Accepted,
        TaskOutcome::Partial => EpisodeOutcome::Partial,
        TaskOutcome::Rejected => EpisodeOutcome::Rejected,
        TaskOutcome::Unknown => EpisodeOutcome::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;

    fn source(path: &Path, label: &str, timestamp: &str) {
        fs::write(
            path,
            serde_json::json!([
                {"role":"meta","source":"fixture","model":label},
                {"role":"user","timestamp":timestamp,"content":format!("PRIVATE-{label}")},
                {"role":"assistant","timestamp":"2026-09-11T00:00:01Z","content":"PRIVATE"}
            ])
            .to_string(),
        )
        .unwrap();
    }

    fn fixture() -> (tempfile::TempDir, LocalInsightStore, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let ids = ["model-b", "model-a"]
            .into_iter()
            .map(|label| {
                let path = root.path().join(label);
                source(&path, label, "2026-09-11T00:00:00Z");
                store.import(SourceFormat::Trajectory, &path).unwrap().id
            })
            .collect();
        (root, store, ids)
    }

    #[test]
    fn codex_without_turn_context_keeps_observed_models_unavailable() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("codex.jsonl");
        fs::write(
            &path,
            concat!(
                "{\"type\":\"session_meta\",\"timestamp\":\"2026-09-11T00:00:00Z\",\"payload\":{\"id\":\"fixture\",\"model_provider\":\"openai\",\"model\":\"ignored\"}}\n",
                "{\"type\":\"response_item\",\"timestamp\":\"2026-09-11T00:00:01Z\",\"payload\":{\"type\":\"message\",\"role\":\"assistant\",\"model\":\"ignored\",\"content\":[]}}\n"
            ),
        )
        .unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let saved = store.import(SourceFormat::Codex, &path).unwrap();
        let request = store
            .resolve_card_request(
                &[InsightQuestionId::ObservedModels],
                std::slice::from_ref(&saved.id),
                &[],
            )
            .unwrap();
        assert_eq!(request.snapshots[0].model_eligible_records, 0);
        assert_eq!(request.snapshots[0].model_observed_records, 0);
        assert!(request.snapshots[0].models.is_empty());
        let card = super::super::cards::project_first_party_question_cards(&request)
            .unwrap()
            .cards
            .remove(0);
        assert_eq!(
            card.state,
            trace_commons_protocol::insights_cards::CardState::Unavailable
        );
        assert!(
            card.rows.is_empty(),
            "zero candidates are not a zero-model row"
        );
        let model_coverage = card
            .coverage
            .iter()
            .find(|item| {
                item.unit == trace_commons_protocol::insights_cards::CoverageUnit::ModelRecords
            })
            .unwrap();
        assert_eq!(model_coverage.observed, 0);
        assert_eq!(model_coverage.eligible, 0);
    }

    #[test]
    fn concurrent_episode_replacement_never_mixes_members_and_snapshot_facts() {
        use std::sync::{Arc, Barrier};

        let (root, store, ids) = fixture();
        let episode = store.episode_create(&[ids[0].clone()]).unwrap();
        let reader = LocalInsightStore::open(&root.path().join("store")).unwrap();
        // Reading cards must continue to use saved evidence during the race.
        for label in ["model-a", "model-b"] {
            fs::remove_file(root.path().join(label)).unwrap();
        }
        let phase = Arc::new(Barrier::new(2));
        std::thread::scope(|scope| {
            let phase_writer = Arc::clone(&phase);
            let id = &episode.id;
            let ids = &ids;
            let writer = scope.spawn(move || {
                let mut revision = 1;
                phase_writer.wait();
                for _ in 0..32 {
                    let next = &ids[(revision % 2) as usize];
                    match store.episode_replace_members(id, revision, std::slice::from_ref(next)) {
                        Ok(updated) => revision = updated.revision,
                        Err(error) => assert_eq!(error.to_string(), "insights_store_busy"),
                    }
                }
                revision
            });
            phase.wait();
            for _ in 0..32 {
                let request = match reader.resolve_card_request(
                    &InsightQuestionId::ALL,
                    &[],
                    std::slice::from_ref(id),
                ) {
                    Ok(request) => request,
                    Err(error) => {
                        assert_eq!(error.to_string(), "insights_store_busy");
                        continue;
                    }
                };
                // Every field must belong to one complete store revision,
                // regardless of how the reader and writer are scheduled.
                request.validate().unwrap();
                assert_eq!(request.episodes.len(), 1);
                assert_eq!(request.snapshots.len(), 1);
                let selected = &request.episodes[0];
                assert!((1..=33).contains(&selected.revision));
                assert_eq!(selected.membership_revision, selected.revision);
                let expected_id = &ids[((selected.revision - 1) % 2) as usize];
                assert_eq!(&selected.members[0].snapshot_id, expected_id);
                assert_eq!(&request.snapshots[0].evidence.id, expected_id);
                assert_eq!(request.evidence[0], request.snapshots[0].evidence);
                assert_eq!(
                    selected.members[0].source_digest,
                    request.evidence[0].source_digest
                );
                super::super::cards::project_first_party_question_cards(&request)
                    .unwrap()
                    .validate_for(&request)
                    .unwrap();
            }
            let final_revision = writer.join().unwrap();
            let final_request = reader
                .resolve_card_request(&InsightQuestionId::ALL, &[], std::slice::from_ref(id))
                .unwrap();
            assert_eq!(final_request.episodes[0].revision, final_revision);
            assert_eq!(
                final_request.snapshots[0].evidence.id,
                ids[((final_revision - 1) % 2) as usize]
            );
        });
    }

    #[test]
    fn empty_and_selected_requests_are_valid_canonical_and_metadata_only() {
        let (root, store, ids) = fixture();
        let empty = empty_card_request(&[InsightQuestionId::RecordedActivity]).unwrap();
        assert!(empty.evidence.is_empty() && empty.snapshots.is_empty());
        empty.validate().unwrap();
        let empty_store = LocalInsightStore::open(&root.path().join("empty-store")).unwrap();
        assert_eq!(
            empty_store
                .resolve_card_request(&[InsightQuestionId::RecordedActivity], &[], &[])
                .unwrap(),
            empty
        );
        assert_eq!(
            validate_card_selection(&[], &[], &[]),
            Err(CardStoreError::InvalidSelection)
        );

        let selected = store
            .resolve_card_request(
                &[
                    InsightQuestionId::ObservedModels,
                    InsightQuestionId::RecordedActivity,
                ],
                &[ids[0].clone(), ids[1].clone()],
                &[],
            )
            .unwrap();
        assert_eq!(
            selected.questions,
            vec![
                InsightQuestionId::RecordedActivity,
                InsightQuestionId::ObservedModels
            ]
        );
        assert!(
            selected
                .snapshots
                .windows(2)
                .all(|w| w[0].evidence.id < w[1].evidence.id)
        );
        assert_eq!(selected.evidence.len(), 2);
        assert!(
            selected
                .snapshots
                .iter()
                .all(|snapshot| snapshot.time.is_some())
        );
        let json = serde_json::to_string(&selected).unwrap();
        assert!(!json.contains("PRIVATE"));
        assert!(!json.contains(root.path().to_string_lossy().as_ref()));
        selected.validate().unwrap();
    }

    #[test]
    fn duplicate_invalid_and_missing_selections_fail_instead_of_being_dropped() {
        let (_root, store, ids) = fixture();
        assert_eq!(
            validate_card_selection(&[], &[], &[]),
            Err(CardStoreError::InvalidSelection)
        );
        assert_eq!(
            validate_card_selection(
                &[
                    InsightQuestionId::RecordedActivity,
                    InsightQuestionId::RecordedActivity,
                ],
                &[],
                &[],
            ),
            Err(CardStoreError::InvalidSelection)
        );
        assert_eq!(
            validate_card_selection(
                &[InsightQuestionId::RecordedActivity],
                &[ids[0].clone(), ids[0].clone()],
                &[],
            ),
            Err(CardStoreError::InvalidSelection)
        );
        assert_eq!(
            validate_card_selection(
                &[InsightQuestionId::RecordedActivity],
                &vec!["f".repeat(64); MAX_CARD_EVIDENCE + 1],
                &[],
            ),
            Err(CardStoreError::SnapshotLimit)
        );
        let missing_snapshot = store
            .resolve_card_request(
                &[InsightQuestionId::RecordedActivity],
                &["f".repeat(64)],
                &[],
            )
            .unwrap_err();
        assert_eq!(
            missing_snapshot.downcast_ref::<CardStoreError>(),
            Some(&CardStoreError::MissingSnapshot)
        );
        let missing_episode = store
            .resolve_card_request(
                &[InsightQuestionId::EpisodeOutcomes],
                &[],
                &["d0c18c96-6093-49f5-bb6f-6092ef0630b9".into()],
            )
            .unwrap_err();
        assert_eq!(
            missing_episode.downcast_ref::<CardStoreError>(),
            Some(&CardStoreError::MissingEpisode)
        );
    }

    #[test]
    fn episode_members_expand_the_union_and_real_changes_change_the_digest() {
        let (_root, store, ids) = fixture();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let first = store
            .resolve_card_request(
                &[InsightQuestionId::EpisodeOutcomes],
                &[ids[1].clone()],
                std::slice::from_ref(&episode.id),
            )
            .unwrap();
        assert_eq!(first.snapshots.len(), 2);
        assert_eq!(first.episodes[0].members.len(), 1);
        let first_digest = first.input_digest().unwrap();

        store
            .annotate(&ids[0], TaskCategory::Docs, TaskOutcome::Accepted)
            .unwrap();
        assert_eq!(
            store
                .resolve_card_request(
                    &[InsightQuestionId::EpisodeOutcomes],
                    &[ids[1].clone()],
                    std::slice::from_ref(&episode.id),
                )
                .unwrap()
                .input_digest()
                .unwrap(),
            first_digest,
            "snapshot assessment is not promoted into episode evidence"
        );

        let assessed = store
            .episode_annotate(
                &episode.id,
                episode.revision,
                TaskCategory::Tests,
                TaskOutcome::Partial,
            )
            .unwrap();
        let annotated = store
            .resolve_card_request(
                &[InsightQuestionId::EpisodeOutcomes],
                &[ids[1].clone()],
                std::slice::from_ref(&episode.id),
            )
            .unwrap();
        assert_ne!(annotated.input_digest().unwrap(), first_digest);
        assert_eq!(
            annotated.episodes[0].assessment.as_ref().unwrap().outcome,
            EpisodeOutcome::Partial
        );

        let unchanged = store
            .episode_annotate(
                &episode.id,
                assessed.revision,
                TaskCategory::Tests,
                TaskOutcome::Partial,
            )
            .unwrap();
        assert_eq!(unchanged.revision, assessed.revision);
        assert_eq!(
            store
                .resolve_card_request(
                    &[InsightQuestionId::EpisodeOutcomes],
                    &[ids[1].clone()],
                    std::slice::from_ref(&episode.id),
                )
                .unwrap()
                .input_digest()
                .unwrap(),
            annotated.input_digest().unwrap()
        );

        let changed = store
            .episode_replace_members(&episode.id, assessed.revision, &ids)
            .unwrap();
        let replaced = store
            .resolve_card_request(
                &[InsightQuestionId::EpisodeOutcomes],
                &[],
                std::slice::from_ref(&episode.id),
            )
            .unwrap();
        assert_ne!(
            replaced.input_digest().unwrap(),
            annotated.input_digest().unwrap()
        );
        assert_eq!(
            replaced.episodes[0].membership_revision,
            changed.membership_revision
        );
        assert!(replaced.episodes[0].assessment.is_none());
        assert_eq!(replaced.snapshots.len(), 2);
    }

    #[test]
    fn deletion_invalidation_and_legacy_unknown_time_are_observed_from_store_state() {
        let (_root, store, ids) = fixture();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let index_path = store.dir.join("index.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&index_path).unwrap()).unwrap();
        legacy["version"] = 4.into();
        for report in legacy["reports"].as_object_mut().unwrap().values_mut() {
            report.as_object_mut().unwrap().remove("time_evidence");
        }
        fs::write(&index_path, serde_json::to_vec(&legacy).unwrap()).unwrap();
        let request = store
            .resolve_card_request(
                &[InsightQuestionId::RecordedActivity],
                &[ids[0].clone()],
                &[],
            )
            .unwrap();
        assert!(request.snapshots[0].time.is_none());
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&fs::read(&index_path).unwrap()).unwrap()["version"],
            4
        );

        assert!(store.delete(&ids[0]).unwrap());
        assert!(
            store
                .resolve_card_request(
                    &[InsightQuestionId::EpisodeOutcomes],
                    &[],
                    std::slice::from_ref(&episode.id),
                )
                .is_err(),
            "episode invalidated by member deletion cannot be silently dropped"
        );
    }
}
