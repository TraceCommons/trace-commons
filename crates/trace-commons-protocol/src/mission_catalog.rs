//! INTEGRATION: Root must add `pub mod mission_catalog;` to `lib.rs`.
//! Bounded public wire types for discovering and retrieving published mission
//! packages. These types validate bytes and structure only; the server endpoint
//! remains the authority for publication and availability, while model spend
//! requires separate local user approval.

use std::{collections::HashSet, fmt};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::mission_evaluation::{MISSION_EVALUATION_PACKAGE_MAX_BYTES, MissionEvaluationPackage};

/// The only supported public mission-catalog schema.
pub const MISSION_CATALOG_SCHEMA_VERSION: u32 = 1;
/// Maximum encoded publication envelope size.
pub const MISSION_PUBLICATION_MAX_BYTES: usize = 256 * 1024;
/// Maximum encoded catalog page size.
pub const MISSION_CATALOG_PAGE_MAX_BYTES: usize = 128 * 1024;
/// Default number of entries requested from the public catalog.
pub const MISSION_CATALOG_DEFAULT_LIMIT: i32 = 20;
/// Maximum number of entries requested from the public catalog.
pub const MISSION_CATALOG_MAX_LIMIT: i32 = 50;
/// Maximum Unicode scalar values in a task preview.
pub const MISSION_CATALOG_TASK_PREVIEW_MAX_CHARS: usize = 240;

/// A server-produced immutable package envelope.
///
/// `package_json` preserves the exact canonical JSON bytes the publisher
/// stored. Parsing this value does not authenticate its source or make a
/// mission available; callers must obtain it from an authoritative endpoint.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionPublication {
    pub schema_version: u32,
    pub package_json: String,
    pub package_sha256: String,
    pub published_at: DateTime<Utc>,
}

/// Public catalog request bounds. A cursor is an exclusive mission ID bound.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionCatalogQuery {
    #[serde(default = "default_limit")]
    pub limit: i32,
    pub before: Option<Uuid>,
}

/// A bounded plaintext preview of a mission currently returned by the server.
///
/// Clients must render `task_preview` as plaintext, never as HTML. This entry
/// carries neither a full package nor any authority to execute one.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionCatalogEntry {
    pub mission_id: Uuid,
    pub program_id: Uuid,
    pub package_sha256: String,
    pub offer_version_hash: String,
    pub task_preview: String,
    pub published_at: DateTime<Utc>,
}

/// A page returned by the authoritative public mission-catalog endpoint.
///
/// Structural validation does not establish that an entry is currently
/// available, published, executable, or approved for model spend.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct MissionCatalogPage {
    pub schema_version: u32,
    pub entries: Vec<MissionCatalogEntry>,
    pub next_cursor: Option<Uuid>,
}

/// Stable public-wire validation failures. Labels deliberately contain no
/// package content, identity, or endpoint details.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionCatalogError {
    PublicationInputTooLarge,
    PageInputTooLarge,
    MalformedJson,
    UnsupportedSchemaVersion,
    PackageTooLarge,
    InvalidPackage,
    InvalidPackageSha256,
    PackageSha256Mismatch,
    PackageNotCanonical,
    InvalidLimit,
    InvalidBefore,
    InvalidMissionId,
    InvalidProgramId,
    InvalidEntryPackageSha256,
    InvalidOfferVersionHash,
    InvalidTaskPreview,
    TooManyEntries,
    InvalidOrder,
    DuplicateProgramId,
    InvalidBeforeBoundary,
    InvalidNextCursor,
}

impl MissionCatalogError {
    /// Returns the stable, content-free refusal label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::PublicationInputTooLarge => "mission-catalog-publication-input-too-large",
            Self::PageInputTooLarge => "mission-catalog-page-input-too-large",
            Self::MalformedJson => "mission-catalog-malformed-json",
            Self::UnsupportedSchemaVersion => "mission-catalog-schema-version-unsupported",
            Self::PackageTooLarge => "mission-catalog-package-too-large",
            Self::InvalidPackage => "mission-catalog-package-invalid",
            Self::InvalidPackageSha256 => "mission-catalog-package-sha256-invalid",
            Self::PackageSha256Mismatch => "mission-catalog-package-sha256-mismatch",
            Self::PackageNotCanonical => "mission-catalog-package-not-canonical",
            Self::InvalidLimit => "mission-catalog-limit-invalid",
            Self::InvalidBefore => "mission-catalog-before-invalid",
            Self::InvalidMissionId => "mission-catalog-entry-mission-id-invalid",
            Self::InvalidProgramId => "mission-catalog-entry-program-id-invalid",
            Self::InvalidEntryPackageSha256 => "mission-catalog-entry-package-sha256-invalid",
            Self::InvalidOfferVersionHash => "mission-catalog-entry-offer-version-hash-invalid",
            Self::InvalidTaskPreview => "mission-catalog-entry-task-preview-invalid",
            Self::TooManyEntries => "mission-catalog-page-too-many-entries",
            Self::InvalidOrder => "mission-catalog-page-order-invalid",
            Self::DuplicateProgramId => "mission-catalog-page-program-duplicate",
            Self::InvalidBeforeBoundary => "mission-catalog-page-before-invalid",
            Self::InvalidNextCursor => "mission-catalog-page-next-cursor-invalid",
        }
    }
}

impl fmt::Display for MissionCatalogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for MissionCatalogError {}

impl MissionPublication {
    /// Decodes a bounded publication and validates its exact package binding.
    pub fn parse(bytes: &[u8]) -> Result<Self, MissionCatalogError> {
        if bytes.len() > MISSION_PUBLICATION_MAX_BYTES {
            return Err(MissionCatalogError::PublicationInputTooLarge);
        }
        let publication: Self =
            serde_json::from_slice(bytes).map_err(|_| MissionCatalogError::MalformedJson)?;
        publication.validate()?;
        Ok(publication)
    }

    /// Validates the envelope and returns its strictly canonical package.
    pub fn package(&self) -> Result<MissionEvaluationPackage, MissionCatalogError> {
        self.validate_envelope()?;
        let package = MissionEvaluationPackage::parse(self.package_json.as_bytes())
            .map_err(|_| MissionCatalogError::InvalidPackage)?;
        if package
            .package_sha256()
            .map_err(|_| MissionCatalogError::InvalidPackage)?
            != self.package_sha256
        {
            return Err(MissionCatalogError::PackageNotCanonical);
        }
        Ok(package)
    }

    fn validate_envelope(&self) -> Result<(), MissionCatalogError> {
        if self.schema_version != MISSION_CATALOG_SCHEMA_VERSION {
            return Err(MissionCatalogError::UnsupportedSchemaVersion);
        }
        if self.package_json.len() > MISSION_EVALUATION_PACKAGE_MAX_BYTES {
            return Err(MissionCatalogError::PackageTooLarge);
        }
        if !is_lowercase_sha256(&self.package_sha256) {
            return Err(MissionCatalogError::InvalidPackageSha256);
        }
        if sha256(self.package_json.as_bytes()) != self.package_sha256 {
            return Err(MissionCatalogError::PackageSha256Mismatch);
        }
        Ok(())
    }

    /// Validates the exact raw binding and canonical package representation.
    pub fn validate(&self) -> Result<(), MissionCatalogError> {
        self.package().map(|_| ())
    }
}

impl MissionCatalogQuery {
    /// Validates request bounds before a server call or page validation.
    pub fn validate(&self) -> Result<(), MissionCatalogError> {
        if !(1..=MISSION_CATALOG_MAX_LIMIT).contains(&self.limit) {
            return Err(MissionCatalogError::InvalidLimit);
        }
        if self.before.is_some_and(|before| before.is_nil()) {
            return Err(MissionCatalogError::InvalidBefore);
        }
        Ok(())
    }
}

impl MissionCatalogEntry {
    /// Validates a public entry without asserting server-side availability.
    pub fn validate(&self) -> Result<(), MissionCatalogError> {
        if self.mission_id.is_nil() {
            return Err(MissionCatalogError::InvalidMissionId);
        }
        if self.program_id.is_nil() {
            return Err(MissionCatalogError::InvalidProgramId);
        }
        if !is_lowercase_sha256(&self.package_sha256) {
            return Err(MissionCatalogError::InvalidEntryPackageSha256);
        }
        if !is_r1_sha256(&self.offer_version_hash) {
            return Err(MissionCatalogError::InvalidOfferVersionHash);
        }
        if self.task_preview.is_empty()
            || self.task_preview.chars().count() > MISSION_CATALOG_TASK_PREVIEW_MAX_CHARS
            || self
                .task_preview
                .chars()
                .any(|character| character.is_control() && !matches!(character, '\n' | '\t'))
        {
            return Err(MissionCatalogError::InvalidTaskPreview);
        }
        Ok(())
    }
}

impl MissionCatalogPage {
    /// Decodes and validates a bounded catalog response for `query`.
    pub fn parse(bytes: &[u8], query: &MissionCatalogQuery) -> Result<Self, MissionCatalogError> {
        query.validate()?;
        if bytes.len() > MISSION_CATALOG_PAGE_MAX_BYTES {
            return Err(MissionCatalogError::PageInputTooLarge);
        }
        let page: Self =
            serde_json::from_slice(bytes).map_err(|_| MissionCatalogError::MalformedJson)?;
        page.validate_for(query)?;
        Ok(page)
    }

    /// Validates pagination invariants before forwarding a page to clients.
    pub fn validate_for(&self, query: &MissionCatalogQuery) -> Result<(), MissionCatalogError> {
        query.validate()?;
        if self.schema_version != MISSION_CATALOG_SCHEMA_VERSION {
            return Err(MissionCatalogError::UnsupportedSchemaVersion);
        }
        if self.entries.len() > query.limit as usize {
            return Err(MissionCatalogError::TooManyEntries);
        }

        let mut previous: Option<Uuid> = None;
        let mut program_ids = HashSet::with_capacity(self.entries.len());
        for entry in &self.entries {
            entry.validate()?;
            if previous.is_some_and(|id| id <= entry.mission_id) {
                return Err(MissionCatalogError::InvalidOrder);
            }
            if !program_ids.insert(entry.program_id) {
                return Err(MissionCatalogError::DuplicateProgramId);
            }
            if query
                .before
                .is_some_and(|before| entry.mission_id >= before)
            {
                return Err(MissionCatalogError::InvalidBeforeBoundary);
            }
            previous = Some(entry.mission_id);
        }
        if let Some(cursor) = self.next_cursor {
            if self.entries.len() != query.limit as usize
                || cursor.is_nil()
                || self.entries.last().map(|last| last.mission_id) != Some(cursor)
            {
                return Err(MissionCatalogError::InvalidNextCursor);
            }
        }
        Ok(())
    }
}

/// Returns the default public catalog page limit.
#[must_use]
pub const fn default_limit() -> i32 {
    MISSION_CATALOG_DEFAULT_LIMIT
}

fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn is_r1_sha256(value: &str) -> bool {
    value
        .strip_prefix("sha256:")
        .is_some_and(is_lowercase_sha256)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use crate::mission_catalog::*;
    use crate::mission_evaluation::{
        MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT, MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
        MISSION_EVALUATION_REQUIRED_MODEL_OWNER, MISSION_EVALUATION_SCHEMA_VERSION,
        MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID, MissionEvaluationPolicy,
        SkillDraft, render_skill,
    };

    fn package() -> MissionEvaluationPackage {
        let skill = SkillDraft {
            name: "repair-generated-sources".into(),
            description: "Repair generated files at their authoritative source.".into(),
            procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".into(),
        };
        MissionEvaluationPackage {
            schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
            mission_id: Uuid::from_u128(100),
            program_id: Uuid::from_u128(200),
            offer_version_hash: format!("sha256:{}", "a".repeat(64)),
            task: "Repair the generated manifest from its source schema.".into(),
            skill_sha256: sha256(render_skill(&skill).as_bytes()),
            skill,
            evaluator_id: MISSION_EVALUATOR_ID.into(),
            evaluation_contract_hash: "b".repeat(64),
            execution: MissionEvaluationPolicy {
                required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.into(),
                total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
                output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
                request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
                max_concurrency: 2,
            },
        }
    }

    fn publication() -> MissionPublication {
        let package_json = String::from_utf8(serde_json::to_vec(&package()).unwrap()).unwrap();
        MissionPublication {
            schema_version: MISSION_CATALOG_SCHEMA_VERSION,
            package_sha256: sha256(package_json.as_bytes()),
            package_json,
            published_at: DateTime::UNIX_EPOCH,
        }
    }

    fn query() -> MissionCatalogQuery {
        MissionCatalogQuery {
            limit: 20,
            before: None,
        }
    }

    fn entry(id: u128, program: u128) -> MissionCatalogEntry {
        MissionCatalogEntry {
            mission_id: Uuid::from_u128(id),
            program_id: Uuid::from_u128(program),
            package_sha256: "c".repeat(64),
            offer_version_hash: format!("sha256:{}", "d".repeat(64)),
            task_preview: "Repair the source manifest.".into(),
            published_at: DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn publication_round_trip_retains_exact_canonical_package_bytes() {
        let original = publication();
        let parsed = MissionPublication::parse(&serde_json::to_vec(&original).unwrap()).unwrap();
        assert_eq!(parsed, original);
        assert_eq!(
            parsed.package_json.as_bytes(),
            original.package_json.as_bytes()
        );
        assert_eq!(parsed.package().unwrap(), package());
    }

    #[test]
    fn publication_refuses_noncanonical_or_stale_package_bindings() {
        let mut candidate = publication();
        candidate.package_sha256.replace_range(..1, "c");
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::PackageSha256Mismatch)
        );

        let mut value = serde_json::to_value(package()).unwrap();
        value["skill"]["procedure"] = "Changed procedure.".into();
        candidate = publication();
        candidate.package_json = serde_json::to_string(&value).unwrap();
        candidate.package_sha256 = sha256(candidate.package_json.as_bytes());
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::InvalidPackage)
        );

        candidate = publication();
        candidate.package_json = serde_json::to_string_pretty(&package()).unwrap();
        candidate.package_sha256 = sha256(candidate.package_json.as_bytes());
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::PackageNotCanonical)
        );
    }

    #[test]
    fn publication_refuses_expanded_policy_unknown_fields_versions_and_bounds() {
        let mut value = serde_json::to_value(package()).unwrap();
        value["execution"]["max_concurrency"] = 3.into();
        let mut candidate = publication();
        candidate.package_json = serde_json::to_string(&value).unwrap();
        candidate.package_sha256 = sha256(candidate.package_json.as_bytes());
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::InvalidPackage)
        );

        value = serde_json::to_value(publication()).unwrap();
        value["unexpected"] = true.into();
        assert_eq!(
            MissionPublication::parse(&serde_json::to_vec(&value).unwrap()),
            Err(MissionCatalogError::MalformedJson)
        );
        let mut nested = serde_json::to_value(publication()).unwrap();
        nested["package_json"] = serde_json::to_string(&serde_json::json!({"unexpected": true}))
            .unwrap()
            .into();
        nested["package_sha256"] =
            sha256(nested["package_json"].as_str().unwrap().as_bytes()).into();
        assert_eq!(
            MissionPublication::parse(&serde_json::to_vec(&nested).unwrap()),
            Err(MissionCatalogError::InvalidPackage)
        );
        let mut nested_package = serde_json::to_value(package()).unwrap();
        nested_package["skill"]["unexpected"] = true.into();
        let mut candidate = publication();
        candidate.package_json = serde_json::to_string(&nested_package).unwrap();
        candidate.package_sha256 = sha256(candidate.package_json.as_bytes());
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::InvalidPackage)
        );
        let mut candidate = publication();
        candidate.schema_version = 2;
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::UnsupportedSchemaVersion)
        );
        candidate = publication();
        candidate.package_json = " ".repeat(MISSION_EVALUATION_PACKAGE_MAX_BYTES + 1);
        candidate.package_sha256 = sha256(candidate.package_json.as_bytes());
        assert_eq!(
            candidate.validate(),
            Err(MissionCatalogError::PackageTooLarge)
        );
        assert_eq!(
            MissionPublication::parse(&vec![b' '; MISSION_PUBLICATION_MAX_BYTES + 1]),
            Err(MissionCatalogError::PublicationInputTooLarge)
        );
    }

    #[test]
    fn catalog_accepts_first_next_and_empty_pages() {
        let limit_two = MissionCatalogQuery {
            limit: 2,
            before: None,
        };
        let first = MissionCatalogPage {
            schema_version: 1,
            entries: vec![entry(3, 30), entry(2, 20)],
            next_cursor: Some(Uuid::from_u128(2)),
        };
        assert_eq!(first.validate_for(&limit_two), Ok(()));
        let next_query = MissionCatalogQuery {
            limit: 2,
            before: Some(Uuid::from_u128(2)),
        };
        let final_page = MissionCatalogPage {
            schema_version: 1,
            entries: vec![entry(1, 10)],
            next_cursor: None,
        };
        assert_eq!(final_page.validate_for(&next_query), Ok(()));
        let full_final_page = MissionCatalogPage {
            schema_version: 1,
            entries: vec![entry(5, 50), entry(4, 40)],
            next_cursor: None,
        };
        assert_eq!(full_final_page.validate_for(&limit_two), Ok(()));
        let empty = MissionCatalogPage {
            schema_version: 1,
            entries: vec![],
            next_cursor: None,
        };
        assert_eq!(empty.validate_for(&limit_two), Ok(()));
    }

    #[test]
    fn catalog_refuses_pagination_and_entry_invariant_failures() {
        let mut page = MissionCatalogPage {
            schema_version: 1,
            entries: vec![entry(2, 20), entry(3, 30)],
            next_cursor: Some(Uuid::from_u128(3)),
        };
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::InvalidOrder)
        );
        page.entries = vec![entry(3, 30)];
        page.next_cursor = Some(Uuid::from_u128(3));
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::InvalidNextCursor)
        );
        page.entries.clear();
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::InvalidNextCursor)
        );
        page.next_cursor = None;
        page.entries = vec![entry(3, 30)];
        let before = MissionCatalogQuery {
            limit: 20,
            before: Some(Uuid::from_u128(3)),
        };
        assert_eq!(
            page.validate_for(&before),
            Err(MissionCatalogError::InvalidBeforeBoundary)
        );
        page.entries = (1..=21).rev().map(|id| entry(id, id + 100)).collect();
        page.next_cursor = Some(Uuid::from_u128(1));
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::TooManyEntries)
        );
        page.entries = vec![entry(1, 2)];
        page.next_cursor = Some(Uuid::nil());
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::InvalidNextCursor)
        );
        page.entries = vec![entry(2, 20), entry(1, 20)];
        page.next_cursor = None;
        assert_eq!(
            page.validate_for(&query()),
            Err(MissionCatalogError::DuplicateProgramId)
        );
    }

    #[test]
    fn catalog_enforces_limits_hashes_preview_and_page_size() {
        for limit in [0, 51] {
            assert_eq!(
                MissionCatalogQuery {
                    limit,
                    before: None
                }
                .validate(),
                Err(MissionCatalogError::InvalidLimit)
            );
        }
        for limit in [1, 20, 50] {
            assert_eq!(
                MissionCatalogQuery {
                    limit,
                    before: None
                }
                .validate(),
                Ok(())
            );
        }
        assert_eq!(
            MissionCatalogQuery {
                limit: 20,
                before: Some(Uuid::nil())
            }
            .validate(),
            Err(MissionCatalogError::InvalidBefore)
        );
        let mut item = entry(1, 2);
        item.package_sha256 = "A".repeat(64);
        assert_eq!(
            item.validate(),
            Err(MissionCatalogError::InvalidEntryPackageSha256)
        );
        item = entry(1, 2);
        item.offer_version_hash = "a".repeat(64);
        assert_eq!(
            item.validate(),
            Err(MissionCatalogError::InvalidOfferVersionHash)
        );
        item = entry(1, 2);
        item.task_preview = "漢".repeat(MISSION_CATALOG_TASK_PREVIEW_MAX_CHARS);
        assert_eq!(item.validate(), Ok(()));
        item.task_preview.push('x');
        assert_eq!(
            item.validate(),
            Err(MissionCatalogError::InvalidTaskPreview)
        );
        item.task_preview = "line\rtwo".into();
        assert_eq!(
            item.validate(),
            Err(MissionCatalogError::InvalidTaskPreview)
        );
        item.task_preview = "line\tone\nline two ".into();
        assert_eq!(item.validate(), Ok(()));
        assert_eq!(
            MissionCatalogPage::parse(&vec![b' '; MISSION_CATALOG_PAGE_MAX_BYTES + 1], &query()),
            Err(MissionCatalogError::PageInputTooLarge)
        );
    }
}
