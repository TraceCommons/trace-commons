//! Atomic lifecycle for local comparison tasks.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fmt;

use anyhow::{Result, bail};
use chrono::Utc;

use super::comparison_tasks::*;
use super::{Index, LocalInsightStore};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComparisonTaskStoreError {
    NotFound,
    MissingEpisode,
    RevisionConflict,
    MaterialDigestConflict,
    StaleEvidence,
    Full,
    RevisionOverflow,
    DuplicateEpisode,
}

impl fmt::Display for ComparisonTaskStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "insights_comparison_task_not_found",
            Self::MissingEpisode => "insights_comparison_task_missing_episode",
            Self::RevisionConflict => "insights_comparison_task_revision_conflict",
            Self::MaterialDigestConflict => "insights_comparison_task_material_digest_conflict",
            Self::StaleEvidence => "insights_comparison_task_stale_evidence",
            Self::Full => "insights_comparison_task_limit_exceeded",
            Self::RevisionOverflow => "insights_comparison_task_revision_overflow",
            Self::DuplicateEpisode => "insights_comparison_task_duplicate_episode",
        })
    }
}
impl std::error::Error for ComparisonTaskStoreError {}

pub(super) fn validate_index_comparison_tasks(index: &Index) -> Result<()> {
    if (index.version < 7 && !index.comparison_tasks.is_empty())
        || index.comparison_tasks.len() > MAX_COMPARISON_TASKS
    {
        bail!("insights_store_invalid");
    }
    for (id, task) in &index.comparison_tasks {
        task.validate()?;
        if id != &task.id {
            bail!("insights_store_invalid");
        }
    }
    Ok(())
}

fn resolve_bindings(index: &Index, episode_ids: &[String]) -> Result<Vec<FrozenEpisodeBinding>> {
    if episode_ids.is_empty() || episode_ids.len() > MAX_TASK_EPISODES {
        return Err(ComparisonTaskValidationError::EpisodeLimit.into());
    }
    let mut ids = BTreeSet::new();
    let mut bindings = Vec::with_capacity(episode_ids.len());
    for id in episode_ids {
        if !ids.insert(id) {
            return Err(ComparisonTaskStoreError::DuplicateEpisode.into());
        }
        let episode = index
            .episodes
            .get(id)
            .ok_or(ComparisonTaskStoreError::MissingEpisode)?;
        bindings.push(FrozenEpisodeBinding::from_episode(episode)?);
    }
    bindings.sort_by(|a, b| a.episode_id.cmp(&b.episode_id));
    Ok(bindings)
}

fn checked<'a>(
    index: &'a mut Index,
    id: &str,
    expected_revision: u64,
) -> Result<&'a mut LocalComparisonTaskV1> {
    validate_uuid(id)?;
    let task = index
        .comparison_tasks
        .get_mut(id)
        .ok_or(ComparisonTaskStoreError::NotFound)?;
    if task.revision != expected_revision {
        return Err(ComparisonTaskStoreError::RevisionConflict.into());
    }
    Ok(task)
}

fn advance(task: &mut LocalComparisonTaskV1, material: bool) -> Result<()> {
    task.revision = task
        .revision
        .checked_add(1)
        .ok_or(ComparisonTaskStoreError::RevisionOverflow)?;
    let now = Utc::now().max(task.updated_at);
    if material {
        task.material_revision = task
            .material_revision
            .checked_add(1)
            .ok_or(ComparisonTaskStoreError::RevisionOverflow)?;
        task.material_digest = material_digest(&task.episodes, task.context.as_ref())?;
        task.material_recorded_at = Some(now);
    }
    task.updated_at = now;
    Ok(())
}

fn canonical_evidence(task: &LocalComparisonTaskV1) -> BTreeSet<(String, String)> {
    task.episodes
        .iter()
        .flat_map(|episode| episode.members.iter())
        .map(|member| (member.snapshot_id.clone(), member.source_digest.clone()))
        .collect()
}

fn overlap_components(index: &Index) -> BTreeMap<String, Vec<String>> {
    let evidence = index
        .comparison_tasks
        .iter()
        .map(|(id, task)| (id.clone(), canonical_evidence(task)))
        .collect::<BTreeMap<_, _>>();
    let mut reverse: BTreeMap<(String, String), Vec<String>> = BTreeMap::new();
    for (id, values) in &evidence {
        for value in values {
            reverse.entry(value.clone()).or_default().push(id.clone());
        }
    }
    let mut adjacency: BTreeMap<String, BTreeSet<String>> = evidence
        .keys()
        .map(|id| (id.clone(), BTreeSet::new()))
        .collect();
    for ids in reverse.values() {
        for left in ids {
            for right in ids {
                if left != right {
                    adjacency
                        .get_mut(left)
                        .expect("known task")
                        .insert(right.clone());
                }
            }
        }
    }
    let mut result = BTreeMap::new();
    for id in evidence.keys() {
        let mut seen = BTreeSet::from([id.clone()]);
        let mut queue = VecDeque::from([id.clone()]);
        while let Some(current) = queue.pop_front() {
            for candidate in &adjacency[&current] {
                if seen.insert(candidate.clone()) {
                    queue.push_back(candidate.clone());
                }
            }
        }
        seen.remove(id);
        result.insert(id.clone(), seen.into_iter().collect());
    }
    result
}

fn overlap_effects(ids: impl IntoIterator<Item = String>) -> super::MutationEffects {
    let stale_comparison_task_ids = ids
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    let stale_comparison_tasks = stale_comparison_task_ids
        .iter()
        .map(|task_id| super::StaleComparisonTaskEffect {
            task_id: task_id.clone(),
            reasons: vec![super::ComparisonTaskMutationReason::OverlapChanged],
        })
        .collect();
    super::MutationEffects {
        invalidated_episode_ids: Vec::new(),
        stale_comparison_task_ids,
        stale_comparison_tasks,
    }
}

fn current_binding_reasons(
    index: &Index,
    task: &LocalComparisonTaskV1,
) -> BTreeSet<ComparisonTaskStaleReason> {
    let mut reasons = BTreeSet::new();
    for binding in &task.episodes {
        match index.episodes.get(&binding.episode_id) {
            None => {
                reasons.insert(ComparisonTaskStaleReason::EpisodeMissing);
            }
            Some(current) => {
                if current.revision != binding.revision {
                    reasons.insert(ComparisonTaskStaleReason::EpisodeRevisionChanged);
                }
                if current.membership_revision != binding.membership_revision
                    || current.members != binding.members
                {
                    reasons.insert(ComparisonTaskStaleReason::EpisodeMembershipChanged);
                }
            }
        }
        if binding.members.iter().any(|member| {
            index
                .reports
                .get(&member.snapshot_id)
                .is_none_or(|snapshot| {
                    snapshot.report.evidence[0].source_digest != member.source_digest
                })
        }) {
            reasons.insert(ComparisonTaskStaleReason::SnapshotMissingOrReplaced);
        }
    }
    reasons
}

fn detail(
    index: &Index,
    task: &LocalComparisonTaskV1,
    overlaps: &[String],
) -> ComparisonTaskDetail {
    let mut reasons = current_binding_reasons(index, task);
    if task
        .context
        .as_ref()
        .is_none_or(|context| !context.is_complete())
    {
        reasons.insert(ComparisonTaskStaleReason::ContextIncomplete);
    }
    reasons.insert(ComparisonTaskStaleReason::AttributionPendingQualification);
    if task.outcome.as_ref().is_some_and(|value| {
        value.material_revision != task.material_revision
            || value.material_digest != task.material_digest
    }) {
        reasons.insert(ComparisonTaskStaleReason::OutcomeMaterialChanged);
    }
    if task
        .independence_confirmation
        .as_ref()
        .is_some_and(|value| {
            value.material_revision != task.material_revision
                || value.material_digest != task.material_digest
        })
    {
        reasons.insert(ComparisonTaskStaleReason::IndependenceMaterialChanged);
    }
    if !overlaps.is_empty() {
        reasons.insert(ComparisonTaskStaleReason::OverlappingTaskEvidence);
    }
    ComparisonTaskDetail {
        task: task.clone(),
        stale_reasons: reasons.into_iter().collect(),
        overlapping_task_ids: overlaps.to_vec(),
        resolved_at: Utc::now(),
    }
}

pub(super) fn comparison_task_details(index: &Index) -> Vec<ComparisonTaskDetail> {
    let overlaps = overlap_components(index);
    index
        .comparison_tasks
        .values()
        .map(|task| detail(index, task, &overlaps[&task.id]))
        .collect()
}

impl LocalInsightStore {
    pub fn comparison_task_create(&self, episode_ids: &[String]) -> Result<LocalComparisonTaskV1> {
        Ok(self.comparison_task_create_with_effects(episode_ids)?.value)
    }

    pub fn comparison_task_create_with_effects(
        &self,
        episode_ids: &[String],
    ) -> Result<super::SnapshotMutation<LocalComparisonTaskV1>> {
        let (_lock, mut index) = self.locked()?;
        if index.comparison_tasks.len() >= MAX_COMPARISON_TASKS {
            return Err(ComparisonTaskStoreError::Full.into());
        }
        let episodes = resolve_bindings(&index, episode_ids)?;
        let now = Utc::now();
        let task = LocalComparisonTaskV1 {
            schema_version: COMPARISON_TASK_SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            material_revision: 1,
            material_digest: material_digest(&episodes, None)?,
            created_at: now,
            updated_at: now,
            material_recorded_at: Some(now),
            episodes,
            context: None,
            outcome: None,
            independence_confirmation: None,
        };
        task.validate()?;
        index.comparison_tasks.insert(task.id.clone(), task.clone());
        let mutation_effects = overlap_effects(overlap_components(&index)[&task.id].clone());
        self.save(&index)?;
        Ok(super::SnapshotMutation {
            value: task,
            mutation_effects,
        })
    }

    pub fn comparison_task_list(&self) -> Result<Vec<ComparisonTaskDetail>> {
        let (_lock, index) = self.locked()?;
        Ok(comparison_task_details(&index))
    }

    pub fn comparison_task_explain(&self, id: &str) -> Result<ComparisonTaskDetail> {
        validate_uuid(id)?;
        let (_lock, index) = self.locked()?;
        let task = index
            .comparison_tasks
            .get(id)
            .ok_or(ComparisonTaskStoreError::NotFound)?;
        let overlaps = overlap_components(&index);
        Ok(detail(&index, task, &overlaps[&task.id]))
    }

    pub fn comparison_task_replace_episodes(
        &self,
        id: &str,
        expected_revision: u64,
        episode_ids: &[String],
    ) -> Result<LocalComparisonTaskV1> {
        Ok(self
            .comparison_task_replace_episodes_with_effects(id, expected_revision, episode_ids)?
            .value)
    }

    pub fn comparison_task_replace_episodes_with_effects(
        &self,
        id: &str,
        expected_revision: u64,
        episode_ids: &[String],
    ) -> Result<super::SnapshotMutation<LocalComparisonTaskV1>> {
        let (_lock, mut index) = self.locked()?;
        let before = overlap_components(&index).remove(id).unwrap_or_default();
        checked(&mut index, id, expected_revision)?;
        let episodes = resolve_bindings(&index, episode_ids)?;
        let task = index
            .comparison_tasks
            .get_mut(id)
            .expect("checked task remains");
        if task.episodes == episodes {
            return Ok(super::SnapshotMutation {
                value: task.clone(),
                mutation_effects: super::MutationEffects::default(),
            });
        }
        let outcome_was_current = task.outcome.as_ref().is_some_and(|outcome| {
            outcome.material_revision == task.material_revision
                && outcome.material_digest == task.material_digest
        });
        let assessment_only = task.episodes.len() == episodes.len()
            && task.episodes.iter().zip(&episodes).all(|(old, new)| {
                old.episode_id == new.episode_id
                    && old.membership_revision == new.membership_revision
                    && old.members == new.members
            });
        task.episodes = episodes;
        let prior_material_recorded_at = task.material_recorded_at;
        advance(task, true)?;
        if assessment_only && outcome_was_current {
            if let Some(outcome) = &mut task.outcome {
                outcome.material_revision = task.material_revision;
                outcome.material_digest = task.material_digest.clone();
            }
        }
        if assessment_only {
            task.material_recorded_at = prior_material_recorded_at;
        }
        task.validate()?;
        let result = task.clone();
        let after = overlap_components(&index).remove(id).unwrap_or_default();
        let mutation_effects = overlap_effects(before.into_iter().chain(after));
        self.save(&index)?;
        Ok(super::SnapshotMutation {
            value: result,
            mutation_effects,
        })
    }

    pub fn comparison_task_set_context(
        &self,
        id: &str,
        expected_revision: u64,
        input: ComparisonTaskContextInput,
    ) -> Result<LocalComparisonTaskV1> {
        let context = input.finalize()?;
        let (_lock, mut index) = self.locked()?;
        let task = checked(&mut index, id, expected_revision)?;
        if task.context.as_ref() == Some(&context) {
            return Ok(task.clone());
        }
        task.context = Some(context);
        advance(task, true)?;
        task.validate()?;
        let result = task.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn comparison_task_set_outcome(
        &self,
        id: &str,
        expected_revision: u64,
        value: ComparisonTaskOutcome,
    ) -> Result<LocalComparisonTaskV1> {
        let (_lock, mut index) = self.locked()?;
        let task = checked(&mut index, id, expected_revision)?;
        advance(task, false)?;
        task.outcome = Some(MaterialBoundOutcome {
            value,
            material_revision: task.material_revision,
            material_digest: task.material_digest.clone(),
            recorded_at: task.updated_at,
        });
        task.validate()?;
        let result = task.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn comparison_task_clear_outcome(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<LocalComparisonTaskV1> {
        let (_lock, mut index) = self.locked()?;
        let task = checked(&mut index, id, expected_revision)?;
        if task.outcome.is_none() {
            return Ok(task.clone());
        }
        advance(task, false)?;
        task.outcome = None;
        task.validate()?;
        let result = task.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn comparison_task_reconfirm(
        &self,
        id: &str,
        expected_revision: u64,
        displayed_material_digest: &str,
    ) -> Result<LocalComparisonTaskV1> {
        validate_digest(displayed_material_digest)?;
        let (_lock, mut index) = self.locked()?;
        let stale = {
            let task = index
                .comparison_tasks
                .get(id)
                .ok_or(ComparisonTaskStoreError::NotFound)?;
            if task.revision != expected_revision {
                return Err(ComparisonTaskStoreError::RevisionConflict.into());
            }
            !current_binding_reasons(&index, task).is_empty()
        };
        let task = checked(&mut index, id, expected_revision)?;
        if task.material_digest != displayed_material_digest {
            return Err(ComparisonTaskStoreError::MaterialDigestConflict.into());
        }
        if stale {
            return Err(ComparisonTaskStoreError::StaleEvidence.into());
        }
        advance(task, false)?;
        task.independence_confirmation = Some(IndependenceConfirmation {
            material_revision: task.material_revision,
            material_digest: task.material_digest.clone(),
            confirmed_at: task.updated_at,
        });
        task.validate()?;
        let result = task.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn comparison_task_delete(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<LocalComparisonTaskV1> {
        Ok(self
            .comparison_task_delete_with_effects(id, expected_revision)?
            .value)
    }

    pub fn comparison_task_delete_with_effects(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<super::SnapshotMutation<LocalComparisonTaskV1>> {
        let (_lock, mut index) = self.locked()?;
        let affected = overlap_components(&index).remove(id).unwrap_or_default();
        let result = checked(&mut index, id, expected_revision)?.clone();
        index.comparison_tasks.remove(id);
        let mutation_effects = overlap_effects(affected);
        self.save(&index)?;
        Ok(super::SnapshotMutation {
            value: result,
            mutation_effects,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::Path;

    use super::*;
    use crate::insights::{SourceFormat, TaskOutcome};

    fn source(path: &Path, content: &str) {
        fs::write(
            path,
            serde_json::json!([
                {"role":"meta","source":"fixture","model":"fixture"},
                {"role":"user","timestamp":"2026-01-01T00:00:00Z","content":content}
            ])
            .to_string(),
        )
        .unwrap();
    }

    fn setup() -> (tempfile::TempDir, LocalInsightStore, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let ids = ["a", "b"]
            .into_iter()
            .map(|name| {
                let path = root.path().join(name);
                source(&path, name);
                store.import(SourceFormat::Trajectory, &path).unwrap().id
            })
            .collect();
        (root, store, ids)
    }

    fn known(value: &str) -> ContextString {
        ContextString::Known {
            value: value.into(),
        }
    }
    fn complete_context(date: &str) -> ComparisonTaskContextInput {
        ComparisonTaskContextInput {
            project_id: "d0c18c96-6093-49f5-bb6f-6092ef0630b9".into(),
            category: crate::insights::TaskCategory::Refactor,
            task_date: date.parse().unwrap(),
            checkout_provenance: CheckoutProvenance::Unavailable,
            language: known("rust"),
            configuration: ComparisonConfigurationV1 {
                harness_id: known("codex"),
                harness_version: known("0.12.2"),
                reasoning_effort: ReasoningEffort::Medium,
                tool_policy_id: known("direct-v1"),
                tool_policy_version: known("1"),
                prompt_template_digest: ContextDigest::Known {
                    digest: "11".repeat(32),
                },
            },
        }
    }

    #[test]
    fn lifecycle_separates_cas_material_outcome_and_confirmation() {
        let (_root, store, ids) = setup();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let draft = store
            .comparison_task_create(std::slice::from_ref(&episode.id))
            .unwrap();
        assert_eq!((draft.revision, draft.material_revision), (1, 1));
        assert_eq!(draft.material_recorded_at, Some(draft.created_at));
        let mut legacy = serde_json::to_value(&draft).unwrap();
        legacy
            .as_object_mut()
            .unwrap()
            .remove("material_recorded_at");
        let legacy: LocalComparisonTaskV1 = serde_json::from_value(legacy).unwrap();
        assert_eq!(legacy.material_recorded_at, None);
        legacy.validate().unwrap();
        assert!(
            store
                .comparison_task_explain(&draft.id)
                .unwrap()
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::ContextIncomplete)
        );
        let context = store
            .comparison_task_set_context(&draft.id, 1, complete_context("2026-01-02"))
            .unwrap();
        assert_eq!(context.material_recorded_at, Some(context.updated_at));
        let outcome = store
            .comparison_task_set_outcome(&draft.id, 2, ComparisonTaskOutcome::Accepted)
            .unwrap();
        assert_eq!(outcome.material_revision, context.material_revision);
        assert_eq!(outcome.material_recorded_at, context.material_recorded_at);
        let confirmed = store
            .comparison_task_reconfirm(&draft.id, 3, &outcome.material_digest)
            .unwrap();
        assert_eq!(confirmed.material_recorded_at, context.material_recorded_at);
        let cleared = store.comparison_task_clear_outcome(&draft.id, 4).unwrap();
        assert_eq!(cleared.material_revision, confirmed.material_revision);
        assert_eq!(
            cleared.independence_confirmation,
            confirmed.independence_confirmation
        );
        assert_eq!(cleared.material_recorded_at, context.material_recorded_at);
        let pending = store
            .comparison_task_set_outcome(&draft.id, 5, ComparisonTaskOutcome::Pending)
            .unwrap();
        let changed = store
            .comparison_task_set_context(&draft.id, 6, complete_context("2026-01-03"))
            .unwrap();
        assert_eq!(changed.material_recorded_at, Some(changed.updated_at));
        assert_eq!(
            changed.outcome.unwrap().value,
            ComparisonTaskOutcome::Pending
        );
        let detail = store.comparison_task_explain(&draft.id).unwrap();
        assert!(
            detail
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::OutcomeMaterialChanged)
        );
        assert!(
            detail
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::IndependenceMaterialChanged)
        );
        assert!(
            store
                .comparison_task_reconfirm(&draft.id, 7, &pending.material_digest)
                .is_err()
        );
        store.comparison_task_delete(&draft.id, 7).unwrap();
        assert!(store.comparison_task_list().unwrap().is_empty());
    }

    #[test]
    fn overlap_is_transitive_and_deleted_upstream_evidence_is_retained_stale() {
        let (_root, store, ids) = setup();
        let a = store.episode_create(&ids[..1]).unwrap();
        let ab = store.episode_create(&ids).unwrap();
        let b = store.episode_create(&ids[1..]).unwrap();
        let first = store.comparison_task_create(&[a.id]).unwrap();
        let middle = store.comparison_task_create(&[ab.id]).unwrap();
        let last = store.comparison_task_create(&[b.id]).unwrap();
        let first_detail = store.comparison_task_explain(&first.id).unwrap();
        assert_eq!(
            first_detail.overlapping_task_ids,
            vec![last.id.clone(), middle.id.clone()]
                .into_iter()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect::<Vec<_>>()
        );
        let effects = store.delete_with_effects(&ids[0]).unwrap().mutation_effects;
        assert!(effects.stale_comparison_task_ids.contains(&first.id));
        let first_effect = effects
            .stale_comparison_tasks
            .iter()
            .find(|effect| effect.task_id == first.id)
            .unwrap();
        assert_eq!(
            first_effect.reasons,
            vec![
                crate::insights::ComparisonTaskMutationReason::EpisodeMissing,
                crate::insights::ComparisonTaskMutationReason::SnapshotMissingOrReplaced,
            ]
        );
        assert!(
            store
                .comparison_task_explain(&first.id)
                .unwrap()
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::EpisodeMissing)
        );
    }

    #[test]
    fn unrelated_import_does_not_report_false_missing_evidence_for_revision_stale_task() {
        let (root, store, ids) = setup();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let task = store
            .comparison_task_create(std::slice::from_ref(&episode.id))
            .unwrap();
        store
            .episode_annotate(
                &episode.id,
                1,
                crate::insights::TaskCategory::Refactor,
                TaskOutcome::Accepted,
            )
            .unwrap();

        let unrelated = root.path().join("unrelated");
        source(&unrelated, "unrelated");
        let effects = store
            .import_with_effects(SourceFormat::Trajectory, &unrelated)
            .unwrap()
            .mutation_effects;

        assert!(effects.stale_comparison_task_ids.is_empty());
        assert!(effects.stale_comparison_tasks.is_empty());
        let detail = store.comparison_task_explain(&task.id).unwrap();
        assert!(
            detail
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::EpisodeRevisionChanged)
        );
        assert!(
            !detail
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::EpisodeMissing)
        );
        assert!(
            !detail
                .stale_reasons
                .contains(&ComparisonTaskStaleReason::SnapshotMissingOrReplaced)
        );
    }

    #[test]
    fn deleting_current_episode_member_reports_missing_episode_to_frozen_task() {
        let (_root, store, ids) = setup();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let task = store
            .comparison_task_create(std::slice::from_ref(&episode.id))
            .unwrap();
        store
            .episode_replace_members(&episode.id, 1, &ids[1..])
            .unwrap();

        let effects = store.delete_with_effects(&ids[1]).unwrap().mutation_effects;
        let effect = effects
            .stale_comparison_tasks
            .iter()
            .find(|effect| effect.task_id == task.id)
            .unwrap();
        assert_eq!(
            effect.reasons,
            vec![crate::insights::ComparisonTaskMutationReason::EpisodeMissing]
        );
    }

    #[test]
    fn assessment_only_episode_refresh_preserves_bound_outcome_but_stales_confirmation() {
        let (_root, store, ids) = setup();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let task = store
            .comparison_task_create(std::slice::from_ref(&episode.id))
            .unwrap();
        let material_recorded_at = task.material_recorded_at;
        let task = store
            .comparison_task_set_outcome(&task.id, 1, ComparisonTaskOutcome::Unknown)
            .unwrap();
        let outcome_recorded_at = task.outcome.as_ref().unwrap().recorded_at;
        let task = store
            .comparison_task_reconfirm(&task.id, 2, &task.material_digest)
            .unwrap();
        let episode = store
            .episode_annotate(
                &episode.id,
                1,
                crate::insights::TaskCategory::Refactor,
                TaskOutcome::Accepted,
            )
            .unwrap();
        let refreshed = store
            .comparison_task_replace_episodes(&task.id, 3, std::slice::from_ref(&episode.id))
            .unwrap();
        assert_eq!(
            refreshed.outcome.as_ref().unwrap().material_revision,
            refreshed.material_revision
        );
        assert_eq!(
            refreshed.outcome.as_ref().unwrap().recorded_at,
            outcome_recorded_at
        );
        assert_eq!(refreshed.material_recorded_at, material_recorded_at);
        assert_ne!(
            refreshed
                .independence_confirmation
                .as_ref()
                .unwrap()
                .material_revision,
            refreshed.material_revision
        );
        let changed = store
            .comparison_task_set_context(&task.id, 4, complete_context("2026-01-03"))
            .unwrap();
        let stale_outcome_revision = changed.outcome.as_ref().unwrap().material_revision;
        let episode = store
            .episode_annotate(
                &episode.id,
                2,
                crate::insights::TaskCategory::Refactor,
                TaskOutcome::Partial,
            )
            .unwrap();
        let refreshed_stale = store
            .comparison_task_replace_episodes(&task.id, 5, std::slice::from_ref(&episode.id))
            .unwrap();
        assert_eq!(
            refreshed_stale.outcome.as_ref().unwrap().material_revision,
            stale_outcome_revision,
            "assessment-only refresh must not make an already-stale outcome current"
        );
    }

    #[test]
    fn legacy_promotion_corrupt_cache_and_failed_writes_fail_closed() {
        let (_root, store, ids) = setup();
        let path = store.dir.join("index.json");
        let mut legacy: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        legacy["version"] = 6.into();
        legacy.as_object_mut().unwrap().remove("comparison_tasks");
        let legacy_bytes = serde_json::to_vec(&legacy).unwrap();
        fs::write(&path, &legacy_bytes).unwrap();
        assert!(store.comparison_task_list().unwrap().is_empty());
        assert_eq!(
            fs::read(&path).unwrap(),
            legacy_bytes,
            "legacy read must not rewrite"
        );
        store.episode_create(&ids[..1]).unwrap();
        let upgraded: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(upgraded["version"], super::super::STORE_VERSION);

        let episode = store.episode_create(&ids[1..]).unwrap();
        let task = store.comparison_task_create(&[episode.id]).unwrap();
        store
            .fail_writes
            .store(true, std::sync::atomic::Ordering::Relaxed);
        assert!(
            store
                .comparison_task_set_outcome(&task.id, 1, ComparisonTaskOutcome::Accepted)
                .is_err()
        );
        store
            .fail_writes
            .store(false, std::sync::atomic::Ordering::Relaxed);
        assert_eq!(store.comparison_task_explain(&task.id).unwrap().task, task);

        let valid = fs::read(&path).unwrap();
        let mut corrupt: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        corrupt["comparison_tasks"][&task.id]["material_digest"] = "00".repeat(32).into();
        fs::write(&path, serde_json::to_vec(&corrupt).unwrap()).unwrap();
        assert!(store.comparison_task_list().is_err());
        let mut future: serde_json::Value = serde_json::from_slice(&valid).unwrap();
        future["version"] = (super::super::STORE_VERSION + 1).into();
        fs::write(&path, serde_json::to_vec(&future).unwrap()).unwrap();
        assert_eq!(
            store.comparison_task_list().unwrap_err().to_string(),
            "insights_store_version_unsupported"
        );
    }
}
