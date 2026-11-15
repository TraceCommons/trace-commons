//! The public Skills catalog: one signed document the operator publishes to a
//! bucket and every contributor daemon downloads whole and anonymously.
//!
//! Every URL in the document is rebuilt from its repository, commit and path,
//! never free text. Check results are facts about the published bytes; the
//! document carries no verdict such as "safe". A reader parses nothing in the
//! payload before a signature under a pinned key verifies, and refuses unknown
//! fields, so a new field is a new schema version.
//!
//! The signing domain is this document's own: a feature-manifest signature,
//! or a signature over the bare payload, never verifies as a catalog.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

pub use crate::feature_flags::{Environment, key_id_for, parse_public_key_hex};

/// Prefixed to the payload before signing.
pub const CATALOG_SIGNING_DOMAIN: &[u8] = b"trace-commons.skills-catalog.v1\n";
pub const SCHEMA_VERSION: u32 = 1;
/// What the facts in `checks` mean. Moves when any rule changes.
/// 2 (owner decision, 2026-11-15): download-and-execute patterns
/// (`pipe_to_shell`, `remote_script_exec`) block publication, so a published
/// skill can no longer carry them.
pub const CHECKS_VERSION: u32 = 2;
/// The served (signed) document, and so also its payload.
pub const MAX_CATALOG_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PUBLISHERS: usize = 1000;
pub const MAX_SKILLS: usize = 5000;
pub const MAX_VALIDITY_SECONDS: i64 = 30 * 24 * 60 * 60;
/// How far `issued_at` may be ahead of the reader's clock.
pub const MAX_CLOCK_SKEW_SECONDS: i64 = 5 * 60;
/// More than one is for a key rotation.
pub const MAX_SIGNATURES: usize = 4;
pub const MAX_NAME_BYTES: usize = 128;
pub const MAX_DESCRIPTION_BYTES: usize = 1024;
pub const MAX_PATH_BYTES: usize = 512;
pub const MAX_REPOSITORY_BYTES: usize = 140;
pub const MAX_URL_BYTES: usize = 512;
pub const MAX_PREVIOUS_HASHES: usize = 8;
/// Publication limits: a skill over any of these is not published.
pub const MAX_DOCUMENT_BYTES: u64 = 256 * 1024;
pub const MAX_DIRECTORY_BYTES: u64 = 4 * 1024 * 1024;
pub const MAX_DIRECTORY_FILES: u32 = 500;
/// Instruction patterns a published skill may be annotated with. Facts, not
/// verdicts.
pub const PATTERN_LABELS: &[&str] = &[
    "credential_paths",
    "encoded_blob",
    "exfiltration_endpoint",
    "instruction_override",
    "sudo",
];
/// Download-and-execute patterns. A skill whose instructions match one is
/// not published, so no published skill carries these labels.
pub const BLOCKING_PATTERN_LABELS: &[&str] = &["pipe_to_shell", "remote_script_exec"];
/// The anonymous suggestion route on ingest.
pub const SUGGESTIONS_PATH: &str = "/v1/public/skills-catalog/suggestions";
pub const MAX_SUGGESTION_BYTES: usize = 2048;

/// A refusal, by fixed label.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("skills catalog refused: {0}")]
pub struct CatalogError(pub &'static str);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// A publisher the clients already knew by a fixed id.
    Seed,
    /// A reviewed repository list or awesome-list.
    List,
    /// Listed by a plugin marketplace.
    Marketplace,
    /// Found only through an app's report.
    Suggested,
}

/// Public repository signals. Every count is optional: absent is unknown,
/// never zero.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySignals {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stars: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pushed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub archived: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commits_7d: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commits_28d: Option<u32>,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogPublisher {
    pub id: String,
    pub name: String,
    pub repository: String,
    pub url: String,
    pub instructions_url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills_root: Option<String>,
    pub source_kind: SourceKind,
    pub signals: RepositorySignals,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseStatus {
    Declared,
    Absent,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct License {
    pub status: LicenseStatus,
    /// An SPDX-shaped identifier, when one was declared. A declaration that
    /// is not one is `declared` with no identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spdx: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Frontmatter {
    /// The only value a published skill can carry; invalid frontmatter is
    /// not published.
    Valid,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BundledFiles {
    /// Files with a script extension in the skill's directory.
    pub scripts: u32,
    /// Files Git records as executable (mode 100755).
    pub executables: u32,
    /// Files with a compiled or archive extension.
    pub binaries: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillChecks {
    pub frontmatter: Frontmatter,
    pub document_bytes: u64,
    pub directory_bytes: u64,
    pub directory_files: u32,
    pub bundled: BundledFiles,
    /// Labels from [`PATTERN_LABELS`], sorted and unique.
    pub patterns: Vec<String>,
    /// No push to the repository for a year.
    pub inactive: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSkill {
    /// `<publisher_id>:<path>`.
    pub id: String,
    pub publisher_id: String,
    pub name: String,
    pub description: String,
    pub repository: String,
    pub path: String,
    /// The commit the hash was taken at.
    pub revision: String,
    /// SHA-256 of the exact `SKILL.md` bytes at `revision`.
    pub sha256: String,
    /// Earlier published hashes at this path, newest first.
    #[serde(default)]
    pub previous_sha256: Vec<String>,
    pub source_url: String,
    pub license: License,
    pub first_seen_at: DateTime<Utc>,
    pub last_changed_at: DateTime<Utc>,
    pub checks: SkillChecks,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogDocument {
    pub schema_version: u32,
    pub checks_version: u32,
    pub environment: Environment,
    /// `issued_at` as Unix seconds: monotonic by construction.
    pub revision: u64,
    pub issued_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub publishers: Vec<CatalogPublisher>,
    pub skills: Vec<CatalogSkill>,
}

/// The document as served: the signed payload text, carried as a string so
/// the bytes that were signed are the bytes that are verified.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedCatalog {
    pub payload: String,
    pub signatures: Vec<CatalogSignature>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CatalogSignature {
    /// [`key_id_for`] of the signing key.
    pub key_id: String,
    /// The 64-byte Ed25519 signature over [`signing_input`], lowercase hex.
    pub signature: String,
}

/// The bytes a key signs: [`CATALOG_SIGNING_DOMAIN`] then the payload.
pub fn signing_input(payload: &[u8]) -> Vec<u8> {
    [CATALOG_SIGNING_DOMAIN, payload].concat()
}

pub fn assemble_signed_catalog(payload: String, signatures: Vec<CatalogSignature>) -> Vec<u8> {
    serde_json::to_vec(&SignedCatalog {
        payload,
        signatures,
    })
    .expect("plain strings serialize")
}

/// SHA-256 of a payload, lowercase hex: a revision's content identity.
pub fn digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Verify a served document against pinned keys and return the payload
/// text. Nothing in the payload is read before a signature verifies.
pub fn open_signed_catalog(bytes: &[u8], keys: &[[u8; 32]]) -> Result<String, CatalogError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(CatalogError("skills-catalog-too-large"));
    }
    if keys.is_empty() {
        return Err(CatalogError("skills-catalog-no-trusted-key"));
    }
    let signed: SignedCatalog =
        serde_json::from_slice(bytes).map_err(|_| CatalogError("skills-catalog-json"))?;
    if signed.signatures.is_empty() || signed.signatures.len() > MAX_SIGNATURES {
        return Err(CatalogError("skills-catalog-signature-count"));
    }
    let message = signing_input(signed.payload.as_bytes());
    let verified = signed.signatures.iter().any(|signature| {
        let Ok(raw) = hex::decode(&signature.signature) else {
            return false;
        };
        raw.len() == 64
            && keys
                .iter()
                .filter(|key| key_id_for(key) == signature.key_id)
                .any(|key| {
                    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, key)
                        .verify(&message, &raw)
                        .is_ok()
                })
    });
    if !verified {
        return Err(CatalogError("skills-catalog-signature"));
    }
    Ok(signed.payload)
}

/// Signature, structure and time: what a client accepts. Returns the
/// document and the digest of its payload.
pub fn parse_signed_catalog(
    bytes: &[u8],
    keys: &[[u8; 32]],
    environment: Environment,
    now: DateTime<Utc>,
) -> Result<(CatalogDocument, String), CatalogError> {
    let (document, payload_digest) = decode_signed_catalog(bytes, keys, environment)?;
    if !document.is_current(now) {
        return Err(CatalogError("skills-catalog-not-current"));
    }
    Ok((document, payload_digest))
}

/// Signature and structure, not time: also used to keep an expired
/// cache's revision watermark.
pub fn decode_signed_catalog(
    bytes: &[u8],
    keys: &[[u8; 32]],
    environment: Environment,
) -> Result<(CatalogDocument, String), CatalogError> {
    let payload = open_signed_catalog(bytes, keys)?;
    let document = decode_payload(payload.as_bytes(), environment)?;
    Ok((document, digest(payload.as_bytes())))
}

/// Structural validation of an unsigned payload, as the exporter writes it
/// and as a reader checks it after the signature.
pub fn decode_payload(
    bytes: &[u8],
    environment: Environment,
) -> Result<CatalogDocument, CatalogError> {
    if bytes.len() > MAX_CATALOG_BYTES {
        return Err(CatalogError("skills-catalog-too-large"));
    }
    let document: CatalogDocument =
        serde_json::from_slice(bytes).map_err(|_| CatalogError("skills-catalog-json"))?;
    if document.environment != environment {
        return Err(CatalogError("skills-catalog-identity"));
    }
    document.validate()?;
    Ok(document)
}

impl CatalogDocument {
    pub fn is_current(&self, now: DateTime<Utc>) -> bool {
        self.issued_at <= now + chrono::Duration::seconds(MAX_CLOCK_SKEW_SECONDS)
            && now < self.expires_at
    }

    /// Every rule a reader enforces except the environment and time.
    pub fn validate(&self) -> Result<(), CatalogError> {
        if self.schema_version != SCHEMA_VERSION || self.checks_version != CHECKS_VERSION {
            return Err(CatalogError("skills-catalog-identity"));
        }
        if self.issued_at.timestamp() <= 0
            || u64::try_from(self.issued_at.timestamp()).ok() != Some(self.revision)
            || self.issued_at.timestamp_subsec_nanos() != 0
        {
            return Err(CatalogError("skills-catalog-revision"));
        }
        let validity = self.expires_at.signed_duration_since(self.issued_at);
        if validity <= chrono::Duration::zero()
            || validity > chrono::Duration::seconds(MAX_VALIDITY_SECONDS)
        {
            return Err(CatalogError("skills-catalog-validity"));
        }
        if self.publishers.len() > MAX_PUBLISHERS || self.skills.len() > MAX_SKILLS {
            return Err(CatalogError("skills-catalog-bounds"));
        }
        let latest = self.issued_at + chrono::Duration::seconds(MAX_CLOCK_SKEW_SECONDS);
        for publisher in &self.publishers {
            if !publisher_is_valid(publisher, latest) {
                return Err(CatalogError("skills-catalog-publisher"));
            }
        }
        if !self.publishers.windows(2).all(|w| w[0].id < w[1].id) {
            return Err(CatalogError("skills-catalog-order"));
        }
        for skill in &self.skills {
            let publisher = self.publishers.iter().find(|p| p.id == skill.publisher_id);
            if !publisher.is_some_and(|publisher| skill_is_valid(skill, publisher, latest)) {
                return Err(CatalogError("skills-catalog-skill"));
            }
        }
        if !self.skills.windows(2).all(|w| w[0].id < w[1].id) {
            return Err(CatalogError("skills-catalog-order"));
        }
        Ok(())
    }
}

fn publisher_is_valid(publisher: &CatalogPublisher, latest: DateTime<Utc>) -> bool {
    let repository_ok = normalize_repository(&publisher.repository).as_deref()
        == Some(publisher.repository.as_str());
    let id_ok = match publisher.source_kind {
        SourceKind::Seed => is_seed_publisher_id(&publisher.id),
        _ => publisher.id == generated_publisher_id(&publisher.repository),
    };
    let instructions_ok = match publisher.source_kind {
        SourceKind::Seed => is_public_https_url(&publisher.instructions_url),
        _ => publisher.instructions_url == repository_url(&publisher.repository),
    };
    let root_ok = publisher
        .skills_root
        .as_deref()
        .is_none_or(safe_directory_path);
    let signals = &publisher.signals;
    repository_ok
        && id_ok
        && instructions_ok
        && root_ok
        && publisher.url == repository_url(&publisher.repository)
        && text_ok(&publisher.name, MAX_NAME_BYTES, false)
        && signals.checked_at <= latest
        && signals.pushed_at.is_none_or(|at| at <= latest)
        && signals.created_at.is_none_or(|at| at <= latest)
}

fn skill_is_valid(
    skill: &CatalogSkill,
    publisher: &CatalogPublisher,
    latest: DateTime<Utc>,
) -> bool {
    let checks = &skill.checks;
    let patterns_ok = checks
        .patterns
        .iter()
        .all(|label| PATTERN_LABELS.contains(&label.as_str()))
        && checks.patterns.windows(2).all(|w| w[0] < w[1])
        && checks.patterns.len() <= PATTERN_LABELS.len();
    let license_ok = match skill.license.status {
        LicenseStatus::Declared => skill.license.spdx.as_deref().is_none_or(is_spdx_like),
        LicenseStatus::Absent | LicenseStatus::Unknown => skill.license.spdx.is_none(),
    };
    let unique_previous: BTreeSet<_> = skill.previous_sha256.iter().collect();
    skill.repository == publisher.repository
        && safe_skill_path(&skill.path)
        && skill.id == format!("{}:{}", skill.publisher_id, skill.path)
        && is_lower_hex(&skill.revision, 40)
        && is_lower_hex(&skill.sha256, 64)
        && skill.previous_sha256.len() <= MAX_PREVIOUS_HASHES
        && unique_previous.len() == skill.previous_sha256.len()
        && skill
            .previous_sha256
            .iter()
            .all(|hash| is_lower_hex(hash, 64) && hash != &skill.sha256)
        && skill.source_url == skill_source_url(&skill.repository, &skill.revision, &skill.path)
        && text_ok(&skill.name, MAX_NAME_BYTES, false)
        && text_ok(&skill.description, MAX_DESCRIPTION_BYTES, true)
        && license_ok
        && skill.first_seen_at <= skill.last_changed_at
        && skill.last_changed_at <= latest
        && patterns_ok
        && checks.document_bytes > 0
        && checks.document_bytes <= MAX_DOCUMENT_BYTES
        && checks.directory_bytes >= checks.document_bytes
        && checks.directory_bytes <= MAX_DIRECTORY_BYTES
        && checks.directory_files >= 1
        && checks.directory_files <= MAX_DIRECTORY_FILES
}

fn text_ok(value: &str, limit: usize, newlines: bool) -> bool {
    !value.trim().is_empty()
        && value.len() <= limit
        && value
            .chars()
            .all(|c| !c.is_control() || (newlines && c == '\n'))
}

fn is_lower_hex(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn is_spdx_like(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'+' | b'-'))
}

fn is_seed_publisher_id(value: &str) -> bool {
    (1..=64).contains(&value.len())
        && !value.starts_with("gh.")
        && value
            .bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && value
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'-' | b'_'))
}

fn is_public_https_url(value: &str) -> bool {
    value.len() <= MAX_URL_BYTES
        && value.starts_with("https://")
        && !value.chars().any(|c| c.is_whitespace() || c.is_control())
        && url::Url::parse(value).is_ok_and(|url| {
            url.username().is_empty() && url.password().is_none() && url.host_str().is_some()
        })
}

/// The id of a publisher that has no fixed (seed) id.
pub fn generated_publisher_id(repository: &str) -> String {
    format!("gh.{}", repository.replace('/', "."))
}

pub fn repository_url(repository: &str) -> String {
    format!("https://github.com/{repository}")
}

pub fn skill_source_url(repository: &str, revision: &str, path: &str) -> String {
    format!("https://github.com/{repository}/blob/{revision}/{path}")
}

/// A GitHub `owner/repo`, lowercased, from a bare reference or a github.com
/// URL. Credentials, other hosts, extra path segments and traversal are
/// refused.
pub fn normalize_repository(value: &str) -> Option<String> {
    let value = value.trim();
    let repo = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("git@github.com:"))
        .unwrap_or(value);
    if repo.contains("://") || repo.contains('@') {
        return None;
    }
    let repo = repo.strip_suffix('/').unwrap_or(repo);
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2 || repo.len() > MAX_REPOSITORY_BYTES {
        return None;
    }
    let ok = parts.iter().all(|part| {
        !part.is_empty()
            && *part != "."
            && *part != ".."
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    });
    ok.then(|| repo.to_ascii_lowercase())
}

fn safe_segment(part: &str) -> bool {
    !part.is_empty()
        && part != "."
        && part != ".."
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// A repository-relative path to a `SKILL.md`, in slash syntax.
pub fn safe_skill_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES
        && path.split('/').next_back() == Some("SKILL.md")
        && path.split('/').all(safe_segment)
}

/// A repository-relative directory in slash syntax.
pub fn safe_directory_path(path: &str) -> bool {
    path.len() <= MAX_PATH_BYTES && path.split('/').all(safe_segment)
}

/// One app report of a public skill the catalog lacks: a public GitHub
/// repository, a path to a `SKILL.md`, and a commit. Nothing else, and no
/// identity: the route takes no credential.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SkillSuggestion {
    pub repository: String,
    pub path: String,
    pub revision: String,
}

impl SkillSuggestion {
    /// Parse and validate a request body, or a fixed refusal label.
    pub fn parse(body: &[u8]) -> Result<Self, &'static str> {
        if body.len() > MAX_SUGGESTION_BYTES {
            return Err("skills-suggestion-too-large");
        }
        let suggestion: Self =
            serde_json::from_slice(body).map_err(|_| "skills-suggestion-malformed")?;
        suggestion.validate()?;
        Ok(suggestion)
    }

    /// The repository must already be in normalized form: the server never
    /// reinterprets a submitted address.
    pub fn validate(&self) -> Result<(), &'static str> {
        if normalize_repository(&self.repository).as_deref() != Some(self.repository.as_str()) {
            return Err("skills-suggestion-repository");
        }
        if !safe_skill_path(&self.path) {
            return Err("skills-suggestion-path");
        }
        if !is_lower_hex(&self.revision, 40) {
            return Err("skills-suggestion-revision");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;
