//! Local mission proposals. Structural validation never authorizes publication,
//! execution, spending, or rewards, and does not verify an external claim.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MISSION_DRAFT_VERSION: u32 = 1;
pub const MAX_MISSION_DRAFT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionDraft {
    pub schema_version: u32,
    pub author_id: String,
    pub title: String,
    pub source_urls: Vec<String>,
    pub claim_to_test: String,
    pub task: String,
    pub starting_artifact: StartingArtifact,
    pub evaluator_id: String,
    pub rubric_version: String,
    pub success_criteria: Vec<String>,
    pub required_evidence: Vec<String>,
    pub allowed_models: Vec<String>,
    pub allowed_tools: Vec<String>,
    pub budget: MissionBudget,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartingArtifact {
    pub url: String,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MissionBudget {
    pub max_duration_seconds: u32,
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
}

/// UI review status, not an authorization token or an attestation. Only local
/// structural validation produces this result; no publication path consumes it.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub struct MissionDraftReview {
    pub schema_version: u32,
    pub proposal_sha256: String,
    pub status: DraftStatus,
    pub publication_authorized: bool,
    pub external_sources_verified: bool,
    pub required_reviews: Vec<String>,
}

#[derive(Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftStatus {
    NeedsCuratorReview,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum DraftError {
    #[error("mission-draft-too-large")]
    TooLarge,
    #[error("mission-draft-invalid-json")]
    InvalidJson,
    #[error("mission-draft-version-unsupported")]
    Version,
    #[error("mission-draft-field-invalid")]
    Field,
    #[error("mission-draft-url-invalid")]
    Url,
    #[error("mission-draft-budget-invalid")]
    Budget,
}

impl MissionDraft {
    pub fn parse(bytes: &[u8]) -> Result<Self, DraftError> {
        if bytes.len() > MAX_MISSION_DRAFT_BYTES {
            return Err(DraftError::TooLarge);
        }
        let draft: Self = serde_json::from_slice(bytes).map_err(|_| DraftError::InvalidJson)?;
        draft.review()?;
        Ok(draft)
    }

    pub fn review(&self) -> Result<MissionDraftReview, DraftError> {
        if self.schema_version != MISSION_DRAFT_VERSION {
            return Err(DraftError::Version);
        }
        for label in [&self.author_id, &self.evaluator_id, &self.rubric_version] {
            if label.is_empty()
                || label.len() > 128
                || !label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
            {
                return Err(DraftError::Field);
            }
        }
        for (text, limit) in [
            (&self.title, 200),
            (&self.claim_to_test, 4000),
            (&self.task, 8000),
        ] {
            if text.trim().is_empty() || text.len() > limit || text.chars().any(|c| c == '\0') {
                return Err(DraftError::Field);
            }
        }
        for items in [
            &self.success_criteria,
            &self.required_evidence,
            &self.allowed_models,
            &self.allowed_tools,
        ] {
            if items.is_empty()
                || items.len() > 16
                || items
                    .iter()
                    .any(|s| s.trim().is_empty() || s.len() > 1000 || s.contains('\0'))
            {
                return Err(DraftError::Field);
            }
        }
        if self.source_urls.is_empty() || self.source_urls.len() > 8 {
            return Err(DraftError::Field);
        }
        for source in self
            .source_urls
            .iter()
            .chain(std::iter::once(&self.starting_artifact.url))
        {
            let url = url::Url::parse(source).map_err(|_| DraftError::Url)?;
            if source.len() > 2048
                || url.scheme() != "https"
                || url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
            {
                return Err(DraftError::Url);
            }
        }
        let hash = &self.starting_artifact.sha256;
        if hash.len() != 64
            || !hash
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(DraftError::Field);
        }
        if self.budget.max_duration_seconds == 0
            || self.budget.max_duration_seconds > 86_400
            || self.budget.max_input_tokens == 0
            || self.budget.max_input_tokens > 10_000_000
            || self.budget.max_output_tokens == 0
            || self.budget.max_output_tokens > 10_000_000
        {
            return Err(DraftError::Budget);
        }
        // Typed serialization has fixed field order; bind the full proposal,
        // not just its title or URL. No caller-supplied review status is accepted.
        let bytes = serde_json::to_vec(self).map_err(|_| DraftError::InvalidJson)?;
        if bytes.len() > MAX_MISSION_DRAFT_BYTES {
            return Err(DraftError::TooLarge);
        }
        Ok(MissionDraftReview {
            schema_version: MISSION_DRAFT_VERSION,
            proposal_sha256: format!("{:x}", Sha256::digest(&bytes)),
            status: DraftStatus::NeedsCuratorReview,
            publication_authorized: false,
            external_sources_verified: false,
            required_reviews: vec![
                "source_claim_and_artifact".into(),
                "reproducibility_and_rights".into(),
                "evaluator_and_conflicts".into(),
                "execution_and_budget".into(),
            ],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn draft() -> MissionDraft {
        serde_json::from_str(include_str!("../tests/fixtures/mission-draft.json")).unwrap()
    }

    #[test]
    fn valid_draft_remains_unverified_and_requires_review() {
        let review = draft().review().unwrap();
        assert_eq!(review.status, DraftStatus::NeedsCuratorReview);
        assert!(!review.publication_authorized);
        assert!(!review.external_sources_verified);
        assert!(
            !serde_json::to_string(&review)
                .unwrap()
                .contains("example.com")
        );
    }
    #[test]
    fn reviewed_digest_binds_task_rubric_and_budget() {
        let original = draft().review().unwrap().proposal_sha256;
        for change in 0..3 {
            let mut modified = draft();
            match change {
                0 => modified.task.push_str(" Change the task."),
                1 => modified.rubric_version = "rubric-2".into(),
                _ => modified.budget.max_output_tokens += 1,
            }
            assert_ne!(modified.review().unwrap().proposal_sha256, original);
        }
    }
    #[test]
    fn refuses_missing_rubric_evidence_and_unbounded_budget() {
        let mut value = draft();
        value.success_criteria.clear();
        assert_eq!(value.review(), Err(DraftError::Field));
        value = draft();
        value.required_evidence.clear();
        assert_eq!(value.review(), Err(DraftError::Field));
        value = draft();
        value.budget.max_input_tokens = u64::MAX;
        assert_eq!(value.review(), Err(DraftError::Budget));
    }
    #[test]
    fn refuses_embedded_authority_and_credential_urls() {
        let mut json = serde_json::to_value(draft()).unwrap();
        json["publication_authorized"] = true.into();
        assert!(MissionDraft::parse(&serde_json::to_vec(&json).unwrap()).is_err());
        let mut value = draft();
        value.source_urls = vec!["https://user:secret@example.com/paper".into()];
        assert_eq!(value.review(), Err(DraftError::Url));
        assert_eq!(
            MissionDraft::parse(&vec![b' '; MAX_MISSION_DRAFT_BYTES + 1]).unwrap_err(),
            DraftError::TooLarge
        );
    }
}
