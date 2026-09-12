//! Atomic local episode lifecycle. Grouping never upgrades member evidence authority.
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use anyhow::{Result, bail};
use chrono::Utc;

use super::episodes::{
    EPISODE_SCHEMA_VERSION, EpisodeAssessment, EpisodeDetail, EpisodeListEntry, EpisodeMember,
    EpisodeOverlap, EpisodeProvenance, LocalEpisode, MAX_EPISODES, canonical_members,
    members_digest, validate_episode_id, validate_snapshot_ids,
};
use super::{
    AnnotationProvenance, Index, LocalInsightStore, MutationEffects, TaskCategory, TaskOutcome,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EpisodeStoreError {
    NotFound,
    MissingMembers,
    RevisionConflict,
    Full,
    RevisionOverflow,
}
impl fmt::Display for EpisodeStoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NotFound => "insights_episode_not_found",
            Self::MissingMembers => "insights_episode_missing_members",
            Self::RevisionConflict => "insights_episode_revision_conflict",
            Self::Full => "insights_episode_limit_exceeded",
            Self::RevisionOverflow => "insights_episode_revision_overflow",
        })
    }
}
impl std::error::Error for EpisodeStoreError {}

pub(super) fn validate_index_episodes(index: &Index) -> Result<()> {
    if (index.version < 4 && !index.episodes.is_empty()) || index.episodes.len() > MAX_EPISODES {
        bail!("insights_store_invalid");
    }
    for (id, episode) in &index.episodes {
        episode.validate()?;
        if id != &episode.id
            || episode.members.iter().any(|member| {
                index
                    .reports
                    .get(&member.snapshot_id)
                    .is_none_or(|snapshot| {
                        snapshot.report.evidence[0].source_digest != member.source_digest
                    })
            })
        {
            bail!("insights_store_invalid");
        }
    }
    Ok(())
}

pub(super) fn invalidate_missing_members(index: &mut Index) -> MutationEffects {
    let mut effects = MutationEffects::default();
    index.episodes.retain(|id, episode| {
        let retain = episode
            .members
            .iter()
            .all(|member| index.reports.contains_key(&member.snapshot_id));
        if !retain {
            effects.invalidated_episode_ids.push(id.clone());
        }
        retain
    });
    effects
}

fn resolve_members(index: &Index, ids: &[String]) -> Result<Vec<EpisodeMember>> {
    validate_snapshot_ids(ids)?;
    let members = ids
        .iter()
        .map(|id| {
            let snapshot = index
                .reports
                .get(id)
                .ok_or(EpisodeStoreError::MissingMembers)?;
            Ok(EpisodeMember {
                snapshot_id: id.clone(),
                source_digest: snapshot.report.evidence[0].source_digest.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(canonical_members(members)?)
}
fn checked_episode<'a>(
    index: &'a mut Index,
    id: &str,
    expected_revision: u64,
) -> Result<&'a mut LocalEpisode> {
    validate_episode_id(id)?;
    let episode = index
        .episodes
        .get_mut(id)
        .ok_or(EpisodeStoreError::NotFound)?;
    if episode.revision != expected_revision {
        return Err(EpisodeStoreError::RevisionConflict.into());
    }
    Ok(episode)
}
fn advance(episode: &mut LocalEpisode, membership: bool) -> Result<()> {
    episode.revision = episode
        .revision
        .checked_add(1)
        .ok_or(EpisodeStoreError::RevisionOverflow)?;
    if membership {
        episode.membership_revision = episode
            .membership_revision
            .checked_add(1)
            .ok_or(EpisodeStoreError::RevisionOverflow)?;
    }
    episode.updated_at = Utc::now().max(episode.updated_at);
    Ok(())
}
fn membership_index(index: &Index) -> BTreeMap<&str, Vec<&str>> {
    let mut reverse: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for episode in index.episodes.values() {
        for member in &episode.members {
            reverse
                .entry(&member.snapshot_id)
                .or_default()
                .push(&episode.id);
        }
    }
    reverse
}
fn overlap(reverse: &BTreeMap<&str, Vec<&str>>, episode: &LocalEpisode) -> Vec<EpisodeOverlap> {
    episode
        .members
        .iter()
        .filter_map(|member| {
            let episode_ids = reverse
                .get(member.snapshot_id.as_str())
                .into_iter()
                .flatten()
                .filter(|id| **id != episode.id)
                .map(|id| (*id).to_owned())
                .collect::<Vec<_>>();
            (!episode_ids.is_empty()).then(|| EpisodeOverlap {
                snapshot_id: member.snapshot_id.clone(),
                episode_ids,
            })
        })
        .collect()
}

impl LocalInsightStore {
    pub fn episode_create(&self, snapshot_ids: &[String]) -> Result<LocalEpisode> {
        validate_snapshot_ids(snapshot_ids)?;
        let (_lock, mut index) = self.locked()?;
        if index.episodes.len() >= MAX_EPISODES {
            return Err(EpisodeStoreError::Full.into());
        }
        let members = resolve_members(&index, snapshot_ids)?;
        let now = Utc::now();
        let episode = LocalEpisode {
            schema_version: EPISODE_SCHEMA_VERSION,
            id: uuid::Uuid::new_v4().to_string(),
            revision: 1,
            membership_revision: 1,
            created_at: now,
            updated_at: now,
            provenance: EpisodeProvenance::UserSelectedWholeSnapshots,
            members,
            manual_assessment: None,
        };
        episode.validate()?;
        index.episodes.insert(episode.id.clone(), episode.clone());
        self.save(&index)?;
        Ok(episode)
    }

    pub fn episode_list(&self) -> Result<Vec<EpisodeListEntry>> {
        let (_lock, index) = self.locked()?;
        let reverse = membership_index(&index);
        Ok(index
            .episodes
            .values()
            .map(|episode| {
                let overlapping_episode_ids = overlap(&reverse, episode)
                    .into_iter()
                    .flat_map(|overlap| overlap.episode_ids)
                    .collect::<BTreeSet<_>>()
                    .into_iter()
                    .collect();
                EpisodeListEntry {
                    episode: episode.clone(),
                    overlapping_episode_ids,
                }
            })
            .collect())
    }

    pub fn episode_explain(&self, id: &str) -> Result<EpisodeDetail> {
        validate_episode_id(id)?;
        let (_lock, index) = self.locked()?;
        let episode = index.episodes.get(id).ok_or(EpisodeStoreError::NotFound)?;
        let members = episode
            .members
            .iter()
            .map(|member| index.reports[&member.snapshot_id].clone())
            .collect();
        Ok(EpisodeDetail {
            episode: episode.clone(),
            members,
            overlap: overlap(&membership_index(&index), episode),
            resolved_at: Utc::now(),
        })
    }

    pub fn episode_replace_members(
        &self,
        id: &str,
        expected_revision: u64,
        snapshot_ids: &[String],
    ) -> Result<LocalEpisode> {
        validate_snapshot_ids(snapshot_ids)?;
        let (_lock, mut index) = self.locked()?;
        checked_episode(&mut index, id, expected_revision)?;
        let members = resolve_members(&index, snapshot_ids)?;
        let episode = index
            .episodes
            .get_mut(id)
            .expect("checked episode remains in locked index");
        if episode.members == members {
            return Ok(episode.clone());
        }
        advance(episode, true)?;
        episode.members = members;
        episode.manual_assessment = None;
        episode.validate()?;
        let result = episode.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn episode_annotate(
        &self,
        id: &str,
        expected_revision: u64,
        category: TaskCategory,
        outcome: TaskOutcome,
    ) -> Result<LocalEpisode> {
        let (_lock, mut index) = self.locked()?;
        let episode = checked_episode(&mut index, id, expected_revision)?;
        if episode
            .manual_assessment
            .as_ref()
            .is_some_and(|assessment| {
                assessment.category == category && assessment.outcome == outcome
            })
        {
            return Ok(episode.clone());
        }
        advance(episode, false)?;
        episode.manual_assessment = Some(EpisodeAssessment {
            category,
            outcome,
            provenance: AnnotationProvenance::UserReported,
            recorded_at: episode.updated_at,
            membership_revision: episode.membership_revision,
            members_digest: members_digest(&episode.members)?,
        });
        episode.validate()?;
        let result = episode.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn episode_clear_assessment(
        &self,
        id: &str,
        expected_revision: u64,
    ) -> Result<LocalEpisode> {
        let (_lock, mut index) = self.locked()?;
        let episode = checked_episode(&mut index, id, expected_revision)?;
        if episode.manual_assessment.is_none() {
            return Ok(episode.clone());
        }
        advance(episode, false)?;
        episode.manual_assessment = None;
        let result = episode.clone();
        self.save(&index)?;
        Ok(result)
    }

    pub fn episode_delete(&self, id: &str, expected_revision: u64) -> Result<LocalEpisode> {
        let (_lock, mut index) = self.locked()?;
        let result = checked_episode(&mut index, id, expected_revision)?.clone();
        index.episodes.remove(id);
        self.save(&index)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::super::{
        SourceFormat,
        outcomes::{OutcomeEvidence, parse_test_report},
    };
    use super::*;
    use std::fs;
    use std::path::Path;

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
    fn fixture() -> (tempfile::TempDir, LocalInsightStore, Vec<String>) {
        let root = tempfile::tempdir().unwrap();
        let store = LocalInsightStore::open(&root.path().join("store")).unwrap();
        let ids = ["a", "b", "c"]
            .into_iter()
            .map(|name| {
                let path = root.path().join(name);
                source(&path, name);
                store.import(SourceFormat::Trajectory, &path).unwrap().id
            })
            .collect();
        (root, store, ids)
    }
    fn bytes(store: &LocalInsightStore) -> Vec<u8> {
        fs::read(store.dir.join("index.json")).unwrap()
    }
    fn write_index(store: &LocalInsightStore, value: &serde_json::Value) {
        fs::write(
            store.dir.join("index.json"),
            serde_json::to_vec(value).unwrap(),
        )
        .unwrap();
    }
    fn is_error(error: anyhow::Error, expected: EpisodeStoreError) {
        assert_eq!(error.downcast_ref::<EpisodeStoreError>(), Some(&expected));
    }

    #[test]
    fn assessments_are_independent_revision_bound_and_membership_edits_clear_them() {
        let (_root, store, ids) = fixture();
        store
            .annotate(&ids[0], TaskCategory::Docs, TaskOutcome::Accepted)
            .unwrap();
        let episode = store.episode_create(&ids[..2]).unwrap();
        assert!(episode.manual_assessment.is_none());
        let annotated = store
            .episode_annotate(&episode.id, 1, TaskCategory::Tests, TaskOutcome::Unknown)
            .unwrap();
        assert_eq!((annotated.revision, annotated.membership_revision), (2, 1));
        assert_eq!(
            store
                .explain(&ids[0])
                .unwrap()
                .manual_annotation
                .unwrap()
                .category,
            TaskCategory::Docs
        );
        let before = bytes(&store);
        assert_eq!(
            store
                .episode_annotate(&episode.id, 2, TaskCategory::Tests, TaskOutcome::Unknown)
                .unwrap(),
            annotated
        );
        assert_eq!(
            store
                .episode_replace_members(&episode.id, 2, &[ids[1].clone(), ids[0].clone()])
                .unwrap(),
            annotated
        );
        assert_eq!(bytes(&store), before);
        is_error(
            store
                .episode_replace_members(&episode.id, 1, &ids)
                .unwrap_err(),
            EpisodeStoreError::RevisionConflict,
        );
        is_error(
            store.episode_delete(&episode.id, 1).unwrap_err(),
            EpisodeStoreError::RevisionConflict,
        );
        assert_eq!(bytes(&store), before);
        let replaced = store.episode_replace_members(&episode.id, 2, &ids).unwrap();
        assert_eq!((replaced.revision, replaced.membership_revision), (3, 2));
        assert!(replaced.manual_assessment.is_none());
        let before = bytes(&store);
        assert_eq!(
            store.episode_clear_assessment(&episode.id, 3).unwrap(),
            replaced
        );
        assert_eq!(bytes(&store), before);
        store.episode_delete(&episode.id, 3).unwrap();
        assert_eq!(store.list().unwrap().len(), 3);
        is_error(
            store.episode_delete(&episode.id, 3).unwrap_err(),
            EpisodeStoreError::NotFound,
        );
    }

    #[test]
    fn last_alias_replacement_and_deletion_remove_whole_overlapping_groups_atomically() {
        let (root, store, ids) = fixture();
        let alias = root.path().join("alias");
        fs::copy(root.path().join("a"), &alias).unwrap();
        store.import(SourceFormat::Trajectory, &alias).unwrap();
        let first = store.episode_create(&ids[..2]).unwrap();
        let second = store.episode_create(&ids[1..]).unwrap();
        let overlapping = store.episode_create(&ids[..1]).unwrap();
        store
            .episode_annotate(&first.id, 1, TaskCategory::Tests, TaskOutcome::Accepted)
            .unwrap();
        let stable = store.episode_explain(&first.id).unwrap().episode;
        let identical = store
            .import_with_effects(SourceFormat::Trajectory, &alias)
            .unwrap();
        assert!(
            identical
                .mutation_effects
                .invalidated_episode_ids
                .is_empty()
        );
        assert_eq!(store.episode_explain(&first.id).unwrap().episode, stable);
        source(&root.path().join("a"), "changed");
        assert!(
            store
                .import_with_effects(SourceFormat::Trajectory, &root.path().join("a"))
                .unwrap()
                .mutation_effects
                .invalidated_episode_ids
                .is_empty()
        );
        source(&alias, "changed again");
        let result = store
            .import_with_effects(SourceFormat::Trajectory, &alias)
            .unwrap();
        let mut expected = vec![first.id.clone(), overlapping.id];
        expected.sort();
        assert_eq!(result.mutation_effects.invalidated_episode_ids, expected);
        assert_eq!(store.episode_list().unwrap().len(), 1);
        assert_eq!(store.episode_list().unwrap()[0].episode.id, second.id);
        let removed = store.delete_with_effects(&ids[1]).unwrap();
        assert!(removed.value);
        assert_eq!(
            removed.mutation_effects.invalidated_episode_ids,
            vec![second.id]
        );
        assert!(store.episode_list().unwrap().is_empty());
        assert!(root.path().join("b").is_file());
        assert!(!store.delete_with_effects(&ids[1]).unwrap().value);
    }

    #[test]
    fn overlap_and_member_evidence_resolve_live_without_changing_episode_assessment() {
        let (root, store, ids) = fixture();
        let first = store.episode_create(&ids[..2]).unwrap();
        let second = store.episode_create(&ids[1..]).unwrap();
        let assessed = store
            .episode_annotate(&first.id, 1, TaskCategory::Other, TaskOutcome::Partial)
            .unwrap();
        let detail = store.episode_explain(&first.id).unwrap();
        assert_eq!(detail.overlap.len(), 1);
        assert_eq!(detail.overlap[0].snapshot_id, ids[1]);
        assert_eq!(detail.overlap[0].episode_ids, [second.id]);
        let report = parse_test_report(br#"{"schema_version":1,"runner":"fixture","passed":1,"failed":0,"skipped":0,"observed_at":"2026-01-01T00:00:00Z"}"#).unwrap();
        let snapshot = store
            .link_outcome(&ids[0], OutcomeEvidence::TestReport(report))
            .unwrap();
        let detail = store.episode_explain(&first.id).unwrap();
        assert_eq!(detail.episode, assessed);
        assert_eq!(
            detail
                .members
                .iter()
                .find(|member| member.id == ids[0])
                .unwrap()
                .outcome_links
                .len(),
            1
        );
        store
            .unlink_outcome(&ids[0], &snapshot.outcome_links[0].id)
            .unwrap();
        fs::remove_file(root.path().join("a")).unwrap();
        let detail = store.episode_explain(&first.id).unwrap();
        assert_eq!(detail.episode, assessed);
        assert!(
            detail
                .members
                .iter()
                .find(|member| member.id == ids[0])
                .unwrap()
                .outcome_links
                .is_empty()
        );
    }

    #[test]
    fn legacy_reads_do_not_infer_or_persist_episodes_and_invalid_bindings_fail_closed() {
        let (_root, store, ids) = fixture();
        let original: serde_json::Value = serde_json::from_slice(&bytes(&store)).unwrap();
        for version in [1, 2, 3, 4] {
            let mut legacy = original.clone();
            legacy["version"] = version.into();
            legacy.as_object_mut().unwrap().remove("episodes");
            for report in legacy["reports"].as_object_mut().unwrap().values_mut() {
                report.as_object_mut().unwrap().remove("time_evidence");
                if version < 3 {
                    report.as_object_mut().unwrap().remove("model_observations");
                    report.as_object_mut().unwrap().remove("outcome_links");
                }
            }
            write_index(&store, &legacy);
            let before = bytes(&store);
            assert!(store.episode_list().unwrap().is_empty());
            assert_eq!(bytes(&store), before);
            store.episode_create(&ids[..1]).unwrap();
            let migrated: serde_json::Value = serde_json::from_slice(&bytes(&store)).unwrap();
            assert_eq!(migrated["version"], super::super::STORE_VERSION);
            assert_eq!(migrated["episodes"].as_object().unwrap().len(), 1);
        }
        let valid: serde_json::Value = serde_json::from_slice(&bytes(&store)).unwrap();
        let id = valid["episodes"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap();
        let mut corrupt = valid.clone();
        corrupt["version"] = 3.into();
        for report in corrupt["reports"].as_object_mut().unwrap().values_mut() {
            report.as_object_mut().unwrap().remove("time_evidence");
        }
        write_index(&store, &corrupt);
        assert!(store.episode_list().is_err());
        let mut corrupt = valid.clone();
        corrupt["episodes"][id]["members"][0]["source_digest"] = "f".repeat(64).into();
        write_index(&store, &corrupt);
        assert!(store.list().is_err());
        let mut corrupt = valid;
        corrupt["reports"].as_object_mut().unwrap().remove(&ids[0]);
        write_index(&store, &corrupt);
        assert!(store.episode_list().is_err());
    }

    #[test]
    fn rejected_members_and_revision_overflow_leave_committed_bytes_unchanged() {
        let (_root, store, ids) = fixture();
        let episode = store.episode_create(&ids[..1]).unwrap();
        let before = bytes(&store);
        assert!(store.episode_create(&[]).is_err());
        assert!(
            store
                .episode_create(&[ids[0].clone(), ids[0].clone()])
                .is_err()
        );
        is_error(
            store.episode_create(&["f".repeat(64)]).unwrap_err(),
            EpisodeStoreError::MissingMembers,
        );
        assert_eq!(bytes(&store), before);
        let mut value: serde_json::Value = serde_json::from_slice(&before).unwrap();
        value["episodes"][&episode.id]["revision"] = u64::MAX.into();
        write_index(&store, &value);
        let before = bytes(&store);
        is_error(
            store
                .episode_annotate(
                    &episode.id,
                    u64::MAX,
                    TaskCategory::Docs,
                    TaskOutcome::Accepted,
                )
                .unwrap_err(),
            EpisodeStoreError::RevisionOverflow,
        );
        assert_eq!(bytes(&store), before);
    }
    #[test]
    fn maximum_overlap_is_bounded_and_store_cap_refuses_an_extra_group() {
        let (root, store, mut ids) = fixture();
        for n in ids.len()..super::super::episodes::MAX_EPISODE_MEMBERS {
            let path = root.path().join(format!("member-{n}"));
            source(&path, &format!("member-{n}"));
            ids.push(store.import(SourceFormat::Trajectory, &path).unwrap().id);
        }
        let base = store.episode_create(&ids).unwrap();
        let (_lock, mut index) = store.locked().unwrap();
        for _ in 1..MAX_EPISODES {
            let mut duplicate = base.clone();
            duplicate.id = uuid::Uuid::new_v4().to_string();
            index.episodes.insert(duplicate.id.clone(), duplicate);
        }
        store.save(&index).unwrap();
        drop(_lock);
        let list = store.episode_list().unwrap();
        assert_eq!(list.len(), MAX_EPISODES);
        assert!(
            list.iter()
                .all(|entry| entry.overlapping_episode_ids.len() == MAX_EPISODES - 1)
        );
        let before = bytes(&store);
        is_error(
            store.episode_create(&ids).unwrap_err(),
            EpisodeStoreError::Full,
        );
        assert_eq!(bytes(&store), before);
    }

    #[test]
    fn serialized_edits_from_one_revision_allow_only_one_commit() {
        let (_root, store, ids) = fixture();
        let episode = store.episode_create(&ids).unwrap();
        // Separate clients hold the same version, then acquire the stable lock
        // one after the other, as after a caller retries a busy-lock response.
        let other = LocalInsightStore::open(&store.dir).unwrap();
        let observed_revision = other.episode_explain(&episode.id).unwrap().episode.revision;
        store
            .episode_annotate(
                &episode.id,
                episode.revision,
                TaskCategory::Tests,
                TaskOutcome::Accepted,
            )
            .unwrap();
        let before = bytes(&store);
        is_error(
            other
                .episode_replace_members(&episode.id, observed_revision, &ids[..1])
                .unwrap_err(),
            EpisodeStoreError::RevisionConflict,
        );
        is_error(
            other
                .episode_delete(&episode.id, observed_revision)
                .unwrap_err(),
            EpisodeStoreError::RevisionConflict,
        );
        assert_eq!(bytes(&store), before);
    }
    #[test]
    fn persistence_failure_returns_no_effects_and_preserves_groups_and_snapshots() {
        let (root, store, ids) = fixture();
        let episode = store.episode_create(&ids[..2]).unwrap();
        let episode = store
            .episode_annotate(&episode.id, 1, TaskCategory::Tests, TaskOutcome::Accepted)
            .unwrap();
        let before = bytes(&store);
        store
            .fail_writes
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let failures = [
            store
                .episode_replace_members(&episode.id, 2, &ids)
                .unwrap_err(),
            store
                .episode_annotate(&episode.id, 2, TaskCategory::Docs, TaskOutcome::Rejected)
                .unwrap_err(),
            store.episode_delete(&episode.id, 2).unwrap_err(),
            store.delete_with_effects(&ids[0]).unwrap_err(),
        ];
        assert!(
            failures
                .iter()
                .all(|error| error.to_string() == "insights_store_write_failed")
        );
        source(&root.path().join("a"), "replacement not committed");
        assert_eq!(
            store
                .import_with_effects(SourceFormat::Trajectory, &root.path().join("a"))
                .unwrap_err()
                .to_string(),
            "insights_store_write_failed"
        );
        assert_eq!(bytes(&store), before);
        assert_eq!(store.episode_explain(&episode.id).unwrap().episode, episode);
        assert!(store.explain(&ids[0]).is_ok());
        store
            .fail_writes
            .store(false, std::sync::atomic::Ordering::Relaxed);
        let result = store
            .import_with_effects(SourceFormat::Trajectory, &root.path().join("a"))
            .unwrap();
        assert_eq!(
            result.mutation_effects.invalidated_episode_ids,
            [episode.id]
        );
        assert!(store.episode_list().unwrap().is_empty());
    }
}
