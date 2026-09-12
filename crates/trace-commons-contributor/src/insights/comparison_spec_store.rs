//! Atomic local persistence for immutable retrospective specifications.

use anyhow::Result;

use super::comparison_specs::{
    ComparisonSpecificationDraftInput, ComparisonSpecificationError, ComparisonSpecificationV1,
    CutoffTaskEvidenceV1, DescriptiveComparisonResultV1, DescriptiveOutcome,
    ExactComparisonStratumV1, ObservedAttributedTokens, QualifiedSourceAttribution,
    comparison_task_fact_from_detail, project_descriptive_comparison,
};
use super::comparison_tasks::{ComparisonTaskDetail, ComparisonTaskOutcome};
use super::{Index, LocalInsightStore};

const MAX_COMPARISON_SPECIFICATIONS: usize = 64;

pub(super) fn validate_index_comparison_specifications(index: &Index) -> Result<()> {
    if (index.version < 8 && !index.comparison_specifications.is_empty())
        || index.comparison_specifications.len() > MAX_COMPARISON_SPECIFICATIONS
    {
        return Err(ComparisonSpecificationError::StoreInvalid.into());
    }
    for (id, specification) in &index.comparison_specifications {
        specification.validate()?;
        if id != &specification.id {
            return Err(ComparisonSpecificationError::StoreInvalid.into());
        }
    }
    Ok(())
}

impl LocalInsightStore {
    fn comparison_specification_cutoff_evidence(
        details: &[ComparisonTaskDetail],
        cutoff: chrono::DateTime<chrono::Utc>,
    ) -> Result<Vec<CutoffTaskEvidenceV1>> {
        details
            .iter()
            .filter(|detail| {
                detail
                    .task
                    .material_recorded_at
                    .is_some_and(|recorded| recorded <= cutoff)
                    && detail
                        .task
                        .outcome
                        .as_ref()
                        .is_none_or(|outcome| outcome.recorded_at <= cutoff)
            })
            .map(|detail| {
                Ok(CutoffTaskEvidenceV1 {
                    task_id: detail.task.id.clone(),
                    material_revision: detail.task.material_revision,
                    material_digest: detail.task.material_digest.clone(),
                    substantive_material_digest:
                        super::comparison_specs::substantive_material_digest(&detail.task)?,
                    outcome: detail.task.outcome.as_ref().map_or(
                        DescriptiveOutcome::Unassessed,
                        |outcome| match outcome.value {
                            ComparisonTaskOutcome::Pending => DescriptiveOutcome::Pending,
                            ComparisonTaskOutcome::Accepted => DescriptiveOutcome::Accepted,
                            ComparisonTaskOutcome::Partial => DescriptiveOutcome::Partial,
                            ComparisonTaskOutcome::Rejected => DescriptiveOutcome::Rejected,
                            ComparisonTaskOutcome::Unknown => DescriptiveOutcome::Unknown,
                        },
                    ),
                    outcome_recorded_at: detail
                        .task
                        .outcome
                        .as_ref()
                        .map(|outcome| outcome.recorded_at),
                })
            })
            .collect()
    }

    fn comparison_specification_resolved_facts(
        &self,
    ) -> Result<Vec<super::comparison_specs::ComparisonTaskFactV1>> {
        self.comparison_task_list()?
            .iter()
            .map(|detail| {
                comparison_task_fact_from_detail(
                    detail,
                    QualifiedSourceAttribution::Unavailable {
                        reason: "source-attribution-pending-qualification".into(),
                    },
                    ObservedAttributedTokens::Unavailable {
                        reason: "observed-tokens-unavailable".into(),
                    },
                )
            })
            .collect()
    }

    pub fn comparison_specification_preview(
        &self,
        evidence_cutoff: chrono::DateTime<chrono::Utc>,
        cohort_labels: Vec<String>,
        date_start: chrono::NaiveDate,
        date_end: chrono::NaiveDate,
        stratum: ExactComparisonStratumV1,
    ) -> Result<(ComparisonSpecificationV1, DescriptiveComparisonResultV1)> {
        let specification = ComparisonSpecificationV1::create(
            uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now(),
            Self::comparison_specification_cutoff_evidence(
                &self.comparison_task_list()?,
                evidence_cutoff,
            )?,
            ComparisonSpecificationDraftInput {
                evidence_cutoff,
                cohort_labels,
                date_start,
                date_end,
                stratum,
            },
        )?;
        let facts = self.comparison_specification_resolved_facts()?;
        let result = project_descriptive_comparison(&specification, &facts)?;
        Ok((specification, result))
    }

    pub fn comparison_specification_save(
        &self,
        evidence_cutoff: chrono::DateTime<chrono::Utc>,
        cohort_labels: Vec<String>,
        date_start: chrono::NaiveDate,
        date_end: chrono::NaiveDate,
        stratum: ExactComparisonStratumV1,
    ) -> Result<ComparisonSpecificationV1> {
        let (_lock, mut index) = self.locked()?;
        if index.comparison_specifications.len() >= MAX_COMPARISON_SPECIFICATIONS {
            return Err(ComparisonSpecificationError::StoreFull.into());
        }
        let cutoff_task_evidence = Self::comparison_specification_cutoff_evidence(
            &super::comparison_task_store::comparison_task_details(&index),
            evidence_cutoff,
        )?;
        let specification = ComparisonSpecificationV1::create(
            uuid::Uuid::new_v4().to_string(),
            chrono::Utc::now(),
            cutoff_task_evidence,
            ComparisonSpecificationDraftInput {
                evidence_cutoff,
                cohort_labels,
                date_start,
                date_end,
                stratum,
            },
        )?;
        index
            .comparison_specifications
            .insert(specification.id.clone(), specification.clone());
        self.save(&index)?;
        Ok(specification)
    }

    pub fn comparison_specification_list(&self) -> Result<Vec<ComparisonSpecificationV1>> {
        let (_lock, index) = self.locked()?;
        Ok(index.comparison_specifications.into_values().collect())
    }

    pub fn comparison_specification_get(&self, id: &str) -> Result<ComparisonSpecificationV1> {
        let (_lock, index) = self.locked()?;
        index
            .comparison_specifications
            .get(id)
            .cloned()
            .ok_or_else(|| ComparisonSpecificationError::NotFound.into())
    }

    pub fn comparison_specification_evaluate(
        &self,
        id: &str,
    ) -> Result<DescriptiveComparisonResultV1> {
        let specification = self.comparison_specification_get(id)?;
        let facts = self.comparison_specification_resolved_facts()?;
        project_descriptive_comparison(&specification, &facts)
    }

    pub fn comparison_result_explain(
        &self,
        specification_id: &str,
        audit_digest: &str,
    ) -> Result<DescriptiveComparisonResultV1> {
        let result = self.comparison_specification_evaluate(specification_id)?;
        if result.audit_digest != audit_digest {
            return Err(ComparisonSpecificationError::ResultStale.into());
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::comparison_tasks::{
        CheckoutProvenance, ComparisonConfigurationV1, ComparisonTaskContextInput, ContextDigest,
        ContextString, ReasoningEffort,
    };
    use crate::insights::{SourceFormat, TaskCategory};

    fn private_tempdir() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        dir
    }

    fn stratum() -> ExactComparisonStratumV1 {
        ExactComparisonStratumV1 {
            project_id: "00000000-0000-4000-8000-000000000001".into(),
            language: "rust".into(),
            configuration_fingerprint: "11".repeat(32),
        }
    }

    fn known(value: &str) -> ContextString {
        ContextString::Known {
            value: value.into(),
        }
    }

    fn complete_context() -> ComparisonTaskContextInput {
        ComparisonTaskContextInput {
            project_id: "00000000-0000-4000-8000-000000000001".into(),
            category: TaskCategory::Refactor,
            task_date: chrono::NaiveDate::from_ymd_opt(2026, 9, 5).unwrap(),
            checkout_provenance: CheckoutProvenance::Unavailable,
            language: known("rust"),
            configuration: ComparisonConfigurationV1 {
                harness_id: known("codex"),
                harness_version: known("0.154.0"),
                reasoning_effort: ReasoningEffort::Medium,
                tool_policy_id: known("direct-v1"),
                tool_policy_version: known("1"),
                prompt_template_digest: ContextDigest::Known {
                    digest: "22".repeat(32),
                },
            },
        }
    }

    #[test]
    fn save_list_get_evaluate_and_explain_are_digest_bound() {
        let dir = private_tempdir();
        let store = LocalInsightStore::open(dir.path()).unwrap();
        assert!(store.comparison_specification_list().unwrap().is_empty());
        assert!(!dir.path().join("index.json").exists());

        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(1);
        let saved = store
            .comparison_specification_save(
                cutoff,
                vec!["model-a".into(), "model-b".into()],
                chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                stratum(),
            )
            .unwrap();
        assert_eq!(
            store.comparison_specification_list().unwrap(),
            [saved.clone()]
        );
        assert_eq!(
            store.comparison_specification_get(&saved.id).unwrap(),
            saved
        );
        let result = store.comparison_specification_evaluate(&saved.id).unwrap();
        assert!(result.included_task_ids.is_empty());
        assert!(result.excluded_tasks.is_empty());
        assert_eq!(result.cohorts.len(), 2);
        assert_eq!(
            store
                .comparison_result_explain(&saved.id, &result.audit_digest)
                .unwrap(),
            result
        );
        assert_eq!(
            store
                .comparison_result_explain(&saved.id, &"ff".repeat(32))
                .unwrap_err()
                .to_string(),
            "insights-comparison-result-stale"
        );
    }

    #[test]
    fn corrupt_or_future_specification_store_is_rejected() {
        let dir = private_tempdir();
        let store = LocalInsightStore::open(dir.path()).unwrap();
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(1);
        let saved = store
            .comparison_specification_save(
                cutoff,
                vec!["model-a".into(), "model-b".into()],
                chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                stratum(),
            )
            .unwrap();
        let path = dir.path().join("index.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        json["comparison_specifications"][&saved.id]["specification_digest"] =
            serde_json::json!("00".repeat(32));
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(store.comparison_specification_list().is_err());

        json["version"] = serde_json::json!(super::super::STORE_VERSION + 1);
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        assert!(store.comparison_specification_list().is_err());
    }

    #[test]
    fn current_task_detail_projects_real_typed_qualification_exclusions() {
        let dir = private_tempdir();
        let source = dir.path().join("source.json");
        std::fs::write(
            &source,
            serde_json::json!([
                {"role":"meta","source":"fixture","model":"fixture"},
                {"role":"user","timestamp":"2026-09-05T00:00:00Z","content":"private"}
            ])
            .to_string(),
        )
        .unwrap();
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let snapshot = store.import(SourceFormat::Trajectory, &source).unwrap();
        let episode = store.episode_create(&[snapshot.id]).unwrap();
        let task = store.comparison_task_create(&[episode.id.clone()]).unwrap();
        let task = store
            .comparison_task_set_context(&task.id, task.revision, complete_context())
            .unwrap();
        store
            .comparison_task_reconfirm(&task.id, task.revision, &task.material_digest)
            .unwrap();
        let specification = store
            .comparison_specification_save(
                chrono::Utc::now(),
                vec!["model-a".into(), "model-b".into()],
                chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                ExactComparisonStratumV1 {
                    project_id: "00000000-0000-4000-8000-000000000001".into(),
                    language: "rust".into(),
                    configuration_fingerprint: complete_context()
                        .finalize()
                        .unwrap()
                        .configuration_fingerprint,
                },
            )
            .unwrap();
        let result = store
            .comparison_specification_evaluate(&specification.id)
            .unwrap();
        assert!(result.included_task_ids.is_empty());
        assert_eq!(
            result.excluded_tasks[0].reasons,
            [
                super::super::comparison_specs::ComparisonExclusionReason::CohortUnavailable,
                super::super::comparison_specs::ComparisonExclusionReason::SourceAttributionUnavailable,
            ]
        );

        store
            .episode_annotate(
                &episode.id,
                episode.revision,
                TaskCategory::Refactor,
                crate::insights::TaskOutcome::Accepted,
            )
            .unwrap();
        let current = store.comparison_task_explain(&task.id).unwrap().task;
        let refreshed = store
            .comparison_task_replace_episodes(
                &current.id,
                current.revision,
                std::slice::from_ref(&episode.id),
            )
            .unwrap();
        store
            .comparison_task_reconfirm(
                &refreshed.id,
                refreshed.revision,
                &refreshed.material_digest,
            )
            .unwrap();
        let after_assessment_refresh = store
            .comparison_specification_evaluate(&specification.id)
            .unwrap();
        assert!(
            !after_assessment_refresh.excluded_tasks[0].reasons.contains(
                &super::super::comparison_specs::ComparisonExclusionReason::EvidenceAfterCutoff
            )
        );

        let current = store.comparison_task_explain(&task.id).unwrap().task;
        store
            .comparison_task_reconfirm(&current.id, current.revision, &current.material_digest)
            .unwrap();
        let after_reconfirm = store
            .comparison_specification_evaluate(&specification.id)
            .unwrap();
        assert_eq!(
            after_reconfirm, after_assessment_refresh,
            "CAS-only reconfirmation is not new evidence"
        );

        let current = store.comparison_task_explain(&task.id).unwrap().task;
        let outcome_task = store
            .comparison_task_set_outcome(
                &current.id,
                current.revision,
                ComparisonTaskOutcome::Accepted,
            )
            .unwrap();
        let after_outcome = store
            .comparison_specification_evaluate(&specification.id)
            .unwrap();
        assert!(after_outcome.excluded_tasks[0].reasons.contains(
            &super::super::comparison_specs::ComparisonExclusionReason::EvidenceAfterCutoff
        ));

        let before_outcome =
            outcome_task.outcome.as_ref().unwrap().recorded_at - chrono::Duration::nanoseconds(1);
        let historical = store
            .comparison_specification_save(
                before_outcome,
                vec!["model-a".into(), "model-b".into()],
                chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                ExactComparisonStratumV1 {
                    project_id: "00000000-0000-4000-8000-000000000001".into(),
                    language: "rust".into(),
                    configuration_fingerprint: complete_context()
                        .finalize()
                        .unwrap()
                        .configuration_fingerprint,
                },
            )
            .unwrap();
        let historical_result = store
            .comparison_specification_evaluate(&historical.id)
            .unwrap();
        assert!(
            historical_result.excluded_tasks[0].reasons.contains(
                &super::super::comparison_specs::ComparisonExclusionReason::EvidenceAfterCutoff
            ),
            "an outcome recorded after the cutoff is an exclusion, not a save failure"
        );
    }

    #[test]
    fn legacy_task_without_material_time_has_typed_cutoff_exclusion() {
        let dir = private_tempdir();
        let source = dir.path().join("source.json");
        std::fs::write(
            &source,
            serde_json::json!([
                {"role":"meta","source":"fixture","model":"fixture"},
                {"role":"user","timestamp":"2026-09-05T00:00:00Z","content":"private"}
            ])
            .to_string(),
        )
        .unwrap();
        let store = LocalInsightStore::open(&dir.path().join("store")).unwrap();
        let snapshot = store.import(SourceFormat::Trajectory, &source).unwrap();
        let episode = store.episode_create(&[snapshot.id]).unwrap();
        let task = store.comparison_task_create(&[episode.id]).unwrap();
        let task = store
            .comparison_task_set_context(&task.id, task.revision, complete_context())
            .unwrap();
        store
            .comparison_task_reconfirm(&task.id, task.revision, &task.material_digest)
            .unwrap();
        let specification = store
            .comparison_specification_save(
                chrono::Utc::now(),
                vec!["model-a".into(), "model-b".into()],
                chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                chrono::NaiveDate::from_ymd_opt(2026, 9, 10).unwrap(),
                ExactComparisonStratumV1 {
                    project_id: "00000000-0000-4000-8000-000000000001".into(),
                    language: "rust".into(),
                    configuration_fingerprint: complete_context()
                        .finalize()
                        .unwrap()
                        .configuration_fingerprint,
                },
            )
            .unwrap();
        let path = store.dir.join("index.json");
        let mut json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        json["comparison_tasks"][&task.id]
            .as_object_mut()
            .unwrap()
            .remove("material_recorded_at");
        std::fs::write(&path, serde_json::to_vec(&json).unwrap()).unwrap();
        let result = store
            .comparison_specification_evaluate(&specification.id)
            .unwrap();
        assert!(result.excluded_tasks[0].reasons.contains(
            &super::super::comparison_specs::ComparisonExclusionReason::CutoffTimeUnavailable
        ));
    }
}
