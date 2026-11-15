// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Build the unsigned catalog payload from stored crawl state. Deterministic:
//! the same rows, suppressions and issue time give the same bytes. The result
//! passes the same `decode_payload` a client runs before it is written.

use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use trace_commons_protocol::skills_catalog::{
    BLOCKING_PATTERN_LABELS, CHECKS_VERSION, CatalogDocument, CatalogPublisher, CatalogSkill,
    Environment, Frontmatter, MAX_CATALOG_BYTES, MAX_PREVIOUS_HASHES, MAX_SKILLS,
    MAX_VALIDITY_SECONDS, SCHEMA_VERSION, SkillChecks, SourceKind, decode_payload, repository_url,
    skill_source_url,
};

use super::store::{ExportRepository, ExportSkill};

/// The payload is carried as an escaped string inside the signed document,
/// which can grow it; keep a quarter of the bound free for that.
pub const PAYLOAD_BUDGET_BYTES: usize = MAX_CATALOG_BYTES / 4 * 3;
pub const DEFAULT_VALIDITY_DAYS: i64 = 14;

/// The operator's suppression list, reviewed in Git.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Suppressions {
    pub schema_version: u32,
    #[serde(default)]
    pub skills: BTreeSet<String>,
    #[serde(default)]
    pub publishers: BTreeSet<String>,
    #[serde(default)]
    pub repositories: BTreeSet<String>,
}

impl Suppressions {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let parsed: Self =
            serde_json::from_slice(bytes).map_err(|_| "suppressions-file-malformed")?;
        if parsed.schema_version != 1 {
            return Err("suppressions-file-version");
        }
        Ok(parsed)
    }

    fn suppresses(&self, skill: &ExportSkill) -> bool {
        self.skills.contains(&skill.skill_id)
            || self.publishers.contains(&skill.publisher_id)
            || self.repositories.contains(&skill.repository)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExportSummary {
    pub publishers: usize,
    pub skills: usize,
    pub blocked: usize,
    pub suppressed: usize,
    pub absent: usize,
    pub over_budget: usize,
}

fn rank(kind: SourceKind) -> u8 {
    match kind {
        SourceKind::Seed => 0,
        SourceKind::List => 1,
        SourceKind::Marketplace => 2,
        SourceKind::Suggested => 3,
    }
}

fn publisher(repository: &ExportRepository) -> CatalogPublisher {
    CatalogPublisher {
        id: repository.publisher_id.clone(),
        name: repository.publisher_name.clone(),
        repository: repository.repository.clone(),
        url: repository_url(&repository.repository),
        instructions_url: repository.instructions_url.clone(),
        skills_root: match repository.source_kind {
            SourceKind::Seed => repository.skills_root.clone(),
            _ => None,
        },
        source_kind: repository.source_kind,
        signals: repository.signals.clone(),
    }
}

fn skill(row: &ExportSkill) -> CatalogSkill {
    let mut previous = row.previous_sha256.clone();
    previous.retain(|hash| hash != &row.sha256);
    previous.truncate(MAX_PREVIOUS_HASHES);
    CatalogSkill {
        id: row.skill_id.clone(),
        publisher_id: row.publisher_id.clone(),
        name: row.name.clone(),
        description: row.description.clone(),
        repository: row.repository.clone(),
        path: row.path.clone(),
        revision: row.revision.clone(),
        sha256: row.sha256.clone(),
        previous_sha256: previous,
        source_url: skill_source_url(&row.repository, &row.revision, &row.path),
        license: row.license.clone(),
        first_seen_at: row.first_seen_at,
        last_changed_at: row.last_changed_at,
        checks: SkillChecks {
            frontmatter: Frontmatter::Valid,
            document_bytes: row.checks.document_bytes,
            directory_bytes: row.checks.directory_bytes,
            directory_files: row.checks.directory_files,
            bundled: row.checks.bundled,
            patterns: row.checks.patterns.clone(),
            inactive: row.checks.inactive,
        },
    }
}

/// Build and validate one environment's payload.
pub fn build_document(
    repositories: &[ExportRepository],
    skills: &[ExportSkill],
    suppressions: &Suppressions,
    environment: Environment,
    now: DateTime<Utc>,
    validity_days: i64,
) -> Result<(CatalogDocument, Vec<u8>, ExportSummary), &'static str> {
    if validity_days <= 0 || validity_days * 86_400 > MAX_VALIDITY_SECONDS {
        return Err("export-validity");
    }
    let issued_at = DateTime::from_timestamp(now.timestamp(), 0).ok_or("export-time")?;
    let by_repository: BTreeMap<&str, &ExportRepository> = repositories
        .iter()
        .map(|r| (r.repository.as_str(), r))
        .collect();
    let mut summary = ExportSummary {
        publishers: 0,
        skills: 0,
        blocked: 0,
        suppressed: 0,
        absent: 0,
        over_budget: 0,
    };
    let mut candidates: Vec<(&ExportSkill, &ExportRepository)> = Vec::new();
    for row in skills {
        let Some(repository) = by_repository.get(row.repository.as_str()) else {
            continue;
        };
        if !row.present {
            summary.absent += 1;
        } else if !row.blocking.is_empty()
            // A row crawled under an older checks version may carry a pattern
            // that blocks now; it is not published either.
            || row
                .checks
                .patterns
                .iter()
                .any(|label| BLOCKING_PATTERN_LABELS.contains(&label.as_str()))
        {
            summary.blocked += 1;
        } else if suppressions.suppresses(row) || row.publisher_id != repository.publisher_id {
            summary.suppressed += 1;
        } else {
            candidates.push((row, repository));
        }
    }
    // Most trusted sources first, then the most starred repositories; the
    // bound drops from the end.
    candidates.sort_by(|(a, ra), (b, rb)| {
        rank(ra.source_kind)
            .cmp(&rank(rb.source_kind))
            .then(
                rb.signals
                    .stars
                    .unwrap_or(0)
                    .cmp(&ra.signals.stars.unwrap_or(0)),
            )
            .then(a.skill_id.cmp(&b.skill_id))
    });
    let mut kept: Vec<CatalogSkill> = Vec::new();
    let mut publishers: BTreeMap<String, CatalogPublisher> = BTreeMap::new();
    let mut used = 1024usize;
    for (row, repository) in candidates {
        let entry = skill(row);
        let mut cost = serde_json::to_vec(&entry)
            .map_err(|_| "export-encode")?
            .len()
            + 1;
        if !publishers.contains_key(&repository.publisher_id) {
            cost += serde_json::to_vec(&publisher(repository))
                .map_err(|_| "export-encode")?
                .len()
                + 1;
        }
        if kept.len() == MAX_SKILLS || used + cost > PAYLOAD_BUDGET_BYTES {
            summary.over_budget += 1;
            continue;
        }
        used += cost;
        publishers
            .entry(repository.publisher_id.clone())
            .or_insert_with(|| publisher(repository));
        kept.push(entry);
    }
    kept.sort_by(|a, b| a.id.cmp(&b.id));
    let document = CatalogDocument {
        schema_version: SCHEMA_VERSION,
        checks_version: CHECKS_VERSION,
        environment,
        revision: u64::try_from(issued_at.timestamp()).map_err(|_| "export-time")?,
        issued_at,
        expires_at: issued_at + Duration::days(validity_days),
        publishers: publishers.into_values().collect(),
        skills: kept,
    };
    let bytes = serde_json::to_vec(&document).map_err(|_| "export-encode")?;
    decode_payload(&bytes, environment).map_err(|error| error.0)?;
    summary.publishers = document.publishers.len();
    summary.skills = document.skills.len();
    if summary.skills == 0 {
        return Err("export-empty");
    }
    Ok((document, bytes, summary))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills_catalog::store::StoredChecks;
    use chrono::TimeZone;
    use trace_commons_protocol::skills_catalog::{
        BundledFiles, License, LicenseStatus, RepositorySignals,
    };

    fn now() -> DateTime<Utc> {
        Utc.timestamp_opt(1_800_000_000, 0).unwrap() + Duration::milliseconds(250)
    }

    fn repository(
        name: &str,
        kind: SourceKind,
        publisher_id: &str,
        stars: u64,
    ) -> ExportRepository {
        ExportRepository {
            repository: name.into(),
            source_kind: kind,
            publisher_id: publisher_id.into(),
            publisher_name: name.into(),
            instructions_url: repository_url(name),
            skills_root: (kind == SourceKind::Seed).then(|| "skills".into()),
            signals: RepositorySignals {
                stars: Some(stars),
                pushed_at: None,
                created_at: None,
                archived: Some(false),
                commits_7d: None,
                commits_28d: None,
                checked_at: now() - Duration::hours(1),
            },
        }
    }

    fn row(repository: &ExportRepository, path: &str) -> ExportSkill {
        ExportSkill {
            skill_id: format!("{}:{path}", repository.publisher_id),
            publisher_id: repository.publisher_id.clone(),
            repository: repository.repository.clone(),
            path: path.into(),
            revision: "1".repeat(40),
            sha256: "a".repeat(64),
            previous_sha256: vec!["b".repeat(64), "a".repeat(64)],
            name: "x".into(),
            description: "Does x.".into(),
            license: License {
                status: LicenseStatus::Declared,
                spdx: Some("MIT".into()),
            },
            checks: StoredChecks {
                document_bytes: 10,
                directory_bytes: 20,
                directory_files: 2,
                bundled: BundledFiles::default(),
                patterns: vec![],
                inactive: false,
            },
            blocking: vec![],
            present: true,
            first_seen_at: now() - Duration::days(2),
            last_changed_at: now() - Duration::days(1),
        }
    }

    fn fixture() -> (Vec<ExportRepository>, Vec<ExportSkill>) {
        let seed = repository("obra/superpowers", SourceKind::Seed, "obra", 5);
        let market = repository(
            "acme/skills",
            SourceKind::Marketplace,
            "gh.acme.skills",
            100,
        );
        let mut blocked = row(&market, "b/SKILL.md");
        blocked.blocking = vec!["hidden_text".into()];
        let mut absent = row(&market, "c/SKILL.md");
        absent.present = false;
        let skills = vec![
            row(&seed, "skills/plan/SKILL.md"),
            row(&market, "a/SKILL.md"),
            blocked,
            absent,
            row(&market, "d/SKILL.md"),
        ];
        (vec![seed, market], skills)
    }

    #[test]
    fn export_is_deterministic_and_valid() {
        let (repositories, skills) = fixture();
        let none = Suppressions::default();
        let (document, bytes, summary) = build_document(
            &repositories,
            &skills,
            &none,
            Environment::Staging,
            now(),
            14,
        )
        .unwrap();
        let (_, again, _) = build_document(
            &repositories,
            &skills,
            &none,
            Environment::Staging,
            now(),
            14,
        )
        .unwrap();
        assert_eq!(bytes, again);
        assert_eq!(document.revision, 1_800_000_000);
        assert_eq!(document.issued_at.timestamp_subsec_nanos(), 0);
        assert_eq!(
            document
                .skills
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            [
                "gh.acme.skills:a/SKILL.md",
                "gh.acme.skills:d/SKILL.md",
                "obra:skills/plan/SKILL.md"
            ]
        );
        assert_eq!(document.skills[0].previous_sha256, ["b".repeat(64)]);
        assert_eq!((summary.blocked, summary.absent, summary.skills), (1, 1, 3));
        assert_eq!(
            document.publishers[1].skills_root.as_deref(),
            Some("skills")
        );
        assert_eq!(document.publishers[0].skills_root, None);
    }

    #[test]
    fn suppression_by_skill_publisher_or_repository() {
        let (repositories, skills) = fixture();
        let by_skill = Suppressions {
            schema_version: 1,
            skills: ["gh.acme.skills:a/SKILL.md".to_owned()].into(),
            ..Default::default()
        };
        let (document, _, summary) = build_document(
            &repositories,
            &skills,
            &by_skill,
            Environment::Staging,
            now(),
            14,
        )
        .unwrap();
        assert_eq!(document.skills.len(), 2);
        assert_eq!(summary.suppressed, 1);
        let by_publisher = Suppressions {
            schema_version: 1,
            publishers: ["gh.acme.skills".to_owned()].into(),
            ..Default::default()
        };
        let (document, _, _) = build_document(
            &repositories,
            &skills,
            &by_publisher,
            Environment::Staging,
            now(),
            14,
        )
        .unwrap();
        assert_eq!(
            document.publishers.len(),
            1,
            "a publisher with no skills is not listed"
        );
        let by_repository = Suppressions {
            schema_version: 1,
            repositories: ["obra/superpowers".to_owned(), "acme/skills".to_owned()].into(),
            ..Default::default()
        };
        assert_eq!(
            build_document(
                &repositories,
                &skills,
                &by_repository,
                Environment::Staging,
                now(),
                14
            )
            .unwrap_err(),
            "export-empty"
        );
    }

    #[test]
    fn a_row_with_a_now_blocking_pattern_is_not_published() {
        let (repositories, mut skills) = fixture();
        // Crawled before download-and-execute blocked publication.
        skills[0].checks.patterns = vec!["pipe_to_shell".into()];
        let (document, _, summary) = build_document(
            &repositories,
            &skills,
            &Suppressions::default(),
            Environment::Staging,
            now(),
            14,
        )
        .unwrap();
        assert_eq!(summary.blocked, 2);
        assert!(document.skills.iter().all(|s| s.id != skills[0].skill_id));
    }

    #[test]
    fn the_shipped_suppressions_file_parses() {
        let parsed =
            Suppressions::parse(include_bytes!("../../skills-catalog/suppressions.json")).unwrap();
        assert!(parsed.skills.is_empty() && parsed.publishers.is_empty());
        assert!(Suppressions::parse(br#"{"schema_version":1,"skills":[],"why":"x"}"#).is_err());
    }

    #[test]
    fn validity_is_bounded() {
        let (repositories, skills) = fixture();
        let none = Suppressions::default();
        assert_eq!(
            build_document(
                &repositories,
                &skills,
                &none,
                Environment::Staging,
                now(),
                31
            )
            .unwrap_err(),
            "export-validity"
        );
        assert_eq!(
            build_document(
                &repositories,
                &skills,
                &none,
                Environment::Staging,
                now(),
                0
            )
            .unwrap_err(),
            "export-validity"
        );
    }
}
