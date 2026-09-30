//! Explicit local groups of whole saved snapshots, not inferred task boundaries.
//! Membership and assessments are cache consistency bindings, never authority
//! to attribute work, verify success, or count independent tasks.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::{AnnotationProvenance, LocalInsight, TaskCategory, TaskOutcome};

pub const EPISODE_SCHEMA_VERSION: u32 = 1;
pub const MAX_EPISODES: usize = 256;
pub const MAX_EPISODE_MEMBERS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EpisodeProvenance {
    UserSelectedWholeSnapshots,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeMember {
    pub snapshot_id: String,
    pub source_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeAssessment {
    pub category: TaskCategory,
    pub outcome: TaskOutcome,
    pub provenance: AnnotationProvenance,
    pub recorded_at: DateTime<Utc>,
    pub membership_revision: u64,
    pub members_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalEpisode {
    pub schema_version: u32,
    pub id: String,
    pub revision: u64,
    pub membership_revision: u64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub provenance: EpisodeProvenance,
    pub members: Vec<EpisodeMember>,
    pub manual_assessment: Option<EpisodeAssessment>,
}

/// Membership metadata only; listing does not expand repeated saved reports.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeListEntry {
    pub episode: LocalEpisode,
    pub overlapping_episode_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeOverlap {
    pub snapshot_id: String,
    pub episode_ids: Vec<String>,
}

/// The store resolves all fields from one validated index read. Member evidence
/// keeps its snapshot-level authority and does not become an episode verdict.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EpisodeDetail {
    pub episode: LocalEpisode,
    pub members: Vec<LocalInsight>,
    pub overlap: Vec<EpisodeOverlap>,
    pub resolved_at: DateTime<Utc>,
}

/// Fixed public-safe labels. No identifiers or input values appear in errors.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EpisodeValidationError {
    #[error("insights_episode_invalid")]
    Invalid,
    #[error("insights_episode_member_limit")]
    MemberLimit,
    #[error("insights_episode_duplicate_member")]
    DuplicateMember,
}

type Result<T> = std::result::Result<T, EpisodeValidationError>;

pub fn validate_episode_id(id: &str) -> Result<()> {
    let parsed = Uuid::parse_str(id).map_err(|_| EpisodeValidationError::Invalid)?;
    if parsed.get_version_num() != 4
        || parsed.get_variant() != uuid::Variant::RFC4122
        || parsed.hyphenated().to_string() != id
    {
        return Err(EpisodeValidationError::Invalid);
    }
    Ok(())
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

/// Validate caller selection before resolving reports or acquiring the store lock.
/// Order is not meaningful, but repeated IDs are errors rather than deduplicated.
pub fn validate_snapshot_ids(ids: &[String]) -> Result<()> {
    check_member_count(ids.len())?;
    let mut seen = BTreeSet::new();
    for id in ids {
        if !is_digest(id) {
            return Err(EpisodeValidationError::Invalid);
        }
        if !seen.insert(id) {
            return Err(EpisodeValidationError::DuplicateMember);
        }
    }
    Ok(())
}

fn check_member_count(count: usize) -> Result<()> {
    if count == 0 || count > MAX_EPISODE_MEMBERS {
        return Err(EpisodeValidationError::MemberLimit);
    }
    Ok(())
}

/// Canonicalize a new user selection. Cache reads use strict validation below
/// instead: they must never silently repair unsorted or repeated memberships.
pub fn canonical_members(mut members: Vec<EpisodeMember>) -> Result<Vec<EpisodeMember>> {
    check_member_count(members.len())?;
    members.sort();
    validate_members(&members)?;
    Ok(members)
}

pub fn validate_members(members: &[EpisodeMember]) -> Result<()> {
    check_member_count(members.len())?;
    for member in members {
        if !is_digest(&member.snapshot_id) || !is_digest(&member.source_digest) {
            return Err(EpisodeValidationError::Invalid);
        }
    }
    for pair in members.windows(2) {
        if pair[0].snapshot_id == pair[1].snapshot_id {
            return Err(EpisodeValidationError::DuplicateMember);
        }
        if pair[0].snapshot_id > pair[1].snapshot_id {
            return Err(EpisodeValidationError::Invalid);
        }
    }
    Ok(())
}

/// SHA-256 of ASCII `trace-commons-episode-members-v1\0`, then the u64
/// big-endian member count, then each canonical pair's 64 ASCII ID bytes and
/// 64 ASCII source-digest bytes. Fixed widths prevent ambiguous concatenation.
/// This binds user assessment to membership, not to mutable member annotations.
pub fn members_digest(members: &[EpisodeMember]) -> Result<String> {
    validate_members(members)?;
    let mut hash = Sha256::new();
    hash.update(b"trace-commons-episode-members-v1\0");
    hash.update((members.len() as u64).to_be_bytes());
    for member in members {
        hash.update(member.snapshot_id.as_bytes());
        hash.update(member.source_digest.as_bytes());
    }
    Ok(format!("{:x}", hash.finalize()))
}

impl LocalEpisode {
    /// Structural validation. The store must additionally resolve every member
    /// against its validated saved report and enforce the per-store episode cap.
    pub fn validate(&self) -> Result<()> {
        validate_episode_id(&self.id)?;
        if self.schema_version != EPISODE_SCHEMA_VERSION
            || self.membership_revision == 0
            || self.membership_revision > self.revision
            || self.created_at.timestamp() < 0
            || self.updated_at < self.created_at
        {
            return Err(EpisodeValidationError::Invalid);
        }
        let digest = members_digest(&self.members)?;
        if let Some(assessment) = &self.manual_assessment
            && (assessment.membership_revision != self.membership_revision
                || assessment.members_digest != digest
                || assessment.recorded_at < self.created_at
                || assessment.recorded_at > self.updated_at)
        {
            return Err(EpisodeValidationError::Invalid);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn member(index: u64) -> EpisodeMember {
        EpisodeMember {
            snapshot_id: format!("{index:064x}"),
            source_digest: format!("{:064x}", index + 100),
        }
    }
    fn episode() -> LocalEpisode {
        LocalEpisode {
            schema_version: EPISODE_SCHEMA_VERSION,
            id: "d0c18c96-6093-49f5-bb6f-6092ef0630b9".into(),
            revision: 1,
            membership_revision: 1,
            created_at: DateTime::from_timestamp(100, 0).unwrap(),
            updated_at: DateTime::from_timestamp(100, 0).unwrap(),
            provenance: EpisodeProvenance::UserSelectedWholeSnapshots,
            members: vec![member(1), member(2)],
            manual_assessment: None,
        }
    }

    #[test]
    fn canonical_members_reject_duplicates_and_pin_digest_encoding() {
        let ordered = canonical_members(vec![member(2), member(1)]).unwrap();
        assert_eq!(ordered, episode().members);
        assert!(members_digest(&[member(2), member(1)]).is_err());
        let mut changed = member(1);
        changed.source_digest = "ff".repeat(32);
        assert_eq!(
            canonical_members(vec![member(1), changed]),
            Err(EpisodeValidationError::DuplicateMember)
        );
        assert_eq!(
            members_digest(&ordered).unwrap(),
            "3afe50600d78b2e817fbe734cb291c848447a791143c6fafc29458885ac55354"
        );
        assert_ne!(
            members_digest(&ordered).unwrap(),
            members_digest(&[member(1)]).unwrap()
        );
    }

    #[test]
    fn assessment_is_separate_and_bound_to_membership_revision_digest_and_dates() {
        let mut value = episode();
        value.validate().unwrap();
        let absent = serde_json::to_value(&value).unwrap();
        value.manual_assessment = Some(EpisodeAssessment {
            category: TaskCategory::Unknown,
            outcome: TaskOutcome::Unknown,
            provenance: AnnotationProvenance::UserReported,
            recorded_at: value.updated_at,
            membership_revision: value.membership_revision,
            members_digest: members_digest(&value.members).unwrap(),
        });
        value.validate().unwrap();
        assert_ne!(absent, serde_json::to_value(&value).unwrap());
        value.revision = 2;
        value.validate().unwrap();
        let mut stale = value.clone();
        stale.membership_revision = 2;
        assert!(stale.validate().is_err());
        let mut changed = value.clone();
        changed.members[1] = member(3);
        assert!(changed.validate().is_err());
        for seconds in [99, 101] {
            let mut invalid = value.clone();
            invalid.manual_assessment.as_mut().unwrap().recorded_at =
                DateTime::from_timestamp(seconds, 0).unwrap();
            assert!(invalid.validate().is_err());
        }
        let roundtrip: LocalEpisode =
            serde_json::from_slice(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(value, roundtrip);
    }

    #[test]
    fn read_validation_refuses_corrupt_structure_without_normalizing() {
        let value = episode();
        for id in [
            "private-input",
            "D0C18C96-6093-49F5-BB6F-6092EF0630B9",
            "00000000-0000-0000-0000-000000000000",
        ] {
            let mut invalid = value.clone();
            invalid.id = id.into();
            assert_eq!(
                invalid.validate().unwrap_err().to_string(),
                "insights_episode_invalid"
            );
        }
        let mut invalid = value.clone();
        invalid.members.reverse();
        assert!(invalid.validate().is_err());
        for (revision, membership) in [(0, 0), (1, 0), (1, 2)] {
            invalid = value.clone();
            invalid.revision = revision;
            invalid.membership_revision = membership;
            assert!(invalid.validate().is_err());
        }
        invalid = value.clone();
        invalid.schema_version = 2;
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.created_at = DateTime::from_timestamp(-1, 0).unwrap();
        assert!(invalid.validate().is_err());
        invalid = value.clone();
        invalid.updated_at = DateTime::from_timestamp(99, 0).unwrap();
        assert!(invalid.validate().is_err());
        let mut json = serde_json::to_value(value).unwrap();
        json["provenance"] = "inferred_task_boundary".into();
        assert!(serde_json::from_value::<LocalEpisode>(json).is_err());
    }

    #[test]
    fn bounded_selection_requires_distinct_valid_members() {
        let members: Vec<_> = (0..MAX_EPISODE_MEMBERS as u64).map(member).collect();
        canonical_members(members.clone()).unwrap();
        validate_snapshot_ids(
            &members
                .iter()
                .map(|m| m.snapshot_id.clone())
                .collect::<Vec<_>>(),
        )
        .unwrap();
        assert_eq!(
            canonical_members(vec![]),
            Err(EpisodeValidationError::MemberLimit)
        );
        let oversized = (0..=MAX_EPISODE_MEMBERS as u64).map(member).collect();
        assert_eq!(
            canonical_members(oversized),
            Err(EpisodeValidationError::MemberLimit)
        );
        assert_eq!(
            validate_snapshot_ids(&[member(1).snapshot_id.clone(), member(1).snapshot_id]),
            Err(EpisodeValidationError::DuplicateMember)
        );
        assert!(validate_snapshot_ids(&["private malformed input".into()]).is_err());
        let mut invalid = member(1);
        invalid.source_digest = "AA".repeat(32);
        assert!(canonical_members(vec![invalid]).is_err());
    }
}
