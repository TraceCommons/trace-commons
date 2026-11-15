// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The crawler's PostgreSQL state (V160). Every statement runs inside a
//! transaction that has taken `trace_skills_catalog_guard` with
//! `SET LOCAL ROLE`; a login that was not granted the guard cannot use this
//! store at all, which is the point.

use chrono::{DateTime, Utc};
use deadpool_postgres::Pool;
use serde::{Deserialize, Serialize};
use trace_commons_protocol::skills_catalog::{
    BundledFiles, License, RepositorySignals, SourceKind, generated_publisher_id, repository_url,
};

use super::crawl::RepositoryCrawl;
use crate::error::DatabaseError;

/// The facts stored per skill. Frontmatter validity is implied by the
/// absence of the `frontmatter_invalid` blocking label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredChecks {
    pub document_bytes: u64,
    pub directory_bytes: u64,
    pub directory_files: u32,
    pub bundled: BundledFiles,
    pub patterns: Vec<String>,
    pub inactive: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportRepository {
    pub repository: String,
    pub source_kind: SourceKind,
    pub publisher_id: String,
    pub publisher_name: String,
    pub instructions_url: String,
    pub skills_root: Option<String>,
    pub signals: RepositorySignals,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ExportSkill {
    pub skill_id: String,
    pub publisher_id: String,
    pub repository: String,
    pub path: String,
    pub revision: String,
    pub sha256: String,
    pub previous_sha256: Vec<String>,
    pub name: String,
    pub description: String,
    pub license: License,
    pub checks: StoredChecks,
    pub blocking: Vec<String>,
    pub present: bool,
    pub first_seen_at: DateTime<Utc>,
    pub last_changed_at: DateTime<Utc>,
}

pub struct SkillsCatalogStore {
    pool: Pool,
}

fn kind_label(kind: SourceKind) -> &'static str {
    match kind {
        SourceKind::Seed => "seed",
        SourceKind::List => "list",
        SourceKind::Marketplace => "marketplace",
        SourceKind::Suggested => "suggested",
    }
}

fn kind_from(label: &str) -> SourceKind {
    match label {
        "seed" => SourceKind::Seed,
        "list" => SourceKind::List,
        "marketplace" => SourceKind::Marketplace,
        _ => SourceKind::Suggested,
    }
}

fn to_i64(value: Option<u64>) -> Option<i64> {
    value.and_then(|v| i64::try_from(v).ok())
}

fn to_i32(value: Option<u32>) -> Option<i32> {
    value.and_then(|v| i32::try_from(v).ok())
}

impl SkillsCatalogStore {
    pub fn new(pool: Pool) -> Self {
        Self { pool }
    }

    pub fn from_backend(backend: &crate::db::postgres::PgBackend) -> Self {
        Self::new(backend.trace_pool())
    }

    /// Confirm this login may take the guard. Run once before a crawl so a
    /// misconfigured login fails before any request leaves.
    pub async fn check_access(&self) -> Result<(), DatabaseError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_skills_catalog_guard")
            .await?;
        tx.query_one(
            "SELECT count(*) FROM trace_skills_catalog_repositories",
            &[],
        )
        .await?;
        tx.rollback().await?;
        Ok(())
    }

    /// Distinct repositories with pending suggestions, most reported first.
    pub async fn pending_suggestions(&self, limit: i64) -> Result<Vec<String>, DatabaseError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_skills_catalog_guard")
            .await?;
        let rows = tx
            .query(
                "SELECT repository FROM trace_skills_catalog_suggestions
                  WHERE status = 'pending'
                  GROUP BY repository
                  ORDER BY sum(report_count) DESC, max(last_received_day) DESC, repository
                  LIMIT $1",
                &[&limit],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.into_iter().map(|row| row.get(0)).collect())
    }

    /// Record one repository's crawl. Skills seen are upserted keeping
    /// `first_seen_at`; `last_changed_at` and the previous hashes move only
    /// when the content hash changes. Only a complete crawl marks unseen
    /// skills absent. Pending suggestions for the repository are settled
    /// unless the crawl was rate limited.
    pub async fn record_crawl(
        &self,
        crawl: &RepositoryCrawl,
        now: DateTime<Utc>,
    ) -> Result<(), DatabaseError> {
        let target = &crawl.target;
        let (publisher_id, publisher_name, instructions_url, skills_root) = match &target.seed {
            Some(seed) => (
                seed.publisher_id.clone(),
                seed.publisher_name.clone(),
                seed.instructions_url.clone(),
                target.roots.first().cloned(),
            ),
            None => (
                generated_publisher_id(&target.repository),
                target.repository.clone(),
                repository_url(&target.repository),
                None,
            ),
        };
        let signals = crawl.signals.as_ref();
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_skills_catalog_guard")
            .await?;
        tx.execute(
            "INSERT INTO trace_skills_catalog_repositories AS r (
                 repository, source_kind, publisher_id, publisher_name, instructions_url,
                 skills_root, head_revision, stars, pushed_at, created_at_github, archived,
                 commits_7d, commits_28d, signals_checked_at, crawl_outcome, crawled_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
             ON CONFLICT (repository) DO UPDATE SET
                 source_kind = EXCLUDED.source_kind,
                 publisher_id = EXCLUDED.publisher_id,
                 publisher_name = EXCLUDED.publisher_name,
                 instructions_url = EXCLUDED.instructions_url,
                 skills_root = EXCLUDED.skills_root,
                 head_revision = COALESCE(EXCLUDED.head_revision, r.head_revision),
                 stars = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.stars ELSE EXCLUDED.stars END,
                 pushed_at = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.pushed_at ELSE EXCLUDED.pushed_at END,
                 created_at_github = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.created_at_github ELSE EXCLUDED.created_at_github END,
                 archived = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.archived ELSE EXCLUDED.archived END,
                 commits_7d = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.commits_7d ELSE EXCLUDED.commits_7d END,
                 commits_28d = CASE WHEN EXCLUDED.signals_checked_at IS NULL THEN r.commits_28d ELSE EXCLUDED.commits_28d END,
                 signals_checked_at = COALESCE(EXCLUDED.signals_checked_at, r.signals_checked_at),
                 crawl_outcome = EXCLUDED.crawl_outcome,
                 crawled_at = EXCLUDED.crawled_at",
            &[
                &target.repository,
                &kind_label(target.kind),
                &publisher_id,
                &publisher_name,
                &instructions_url,
                &skills_root,
                &crawl.revision,
                &to_i64(signals.and_then(|s| s.stars)),
                &signals.and_then(|s| s.pushed_at),
                &signals.and_then(|s| s.created_at),
                &signals.and_then(|s| s.archived),
                &to_i32(signals.and_then(|s| s.commits_7d)),
                &to_i32(signals.and_then(|s| s.commits_28d)),
                &signals.map(|s| s.checked_at),
                &crawl.outcome,
                &now,
            ],
        )
        .await?;
        if let Some(revision) = &crawl.revision {
            for skill in &crawl.skills {
                let checks = serde_json::to_value(StoredChecks {
                    document_bytes: skill.document_bytes,
                    directory_bytes: skill.directory_bytes,
                    directory_files: skill.directory_files,
                    bundled: skill.bundled,
                    patterns: skill.patterns.clone(),
                    inactive: skill.inactive,
                })
                .map_err(|_| DatabaseError::Pool("skills_catalog_checks_encode".into()))?;
                let license = serde_json::to_value(&skill.license)
                    .map_err(|_| DatabaseError::Pool("skills_catalog_license_encode".into()))?;
                tx.execute(
                    "INSERT INTO trace_skills_catalog_skills AS s (
                         skill_id, repository, path, publisher_id, revision, sha256, name,
                         description, license, checks, blocking, present,
                         first_seen_at, last_changed_at, last_seen_at)
                     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, TRUE, $12, $12, $12)
                     ON CONFLICT (repository, path) DO UPDATE SET
                         skill_id = EXCLUDED.skill_id,
                         publisher_id = EXCLUDED.publisher_id,
                         previous_sha256 = CASE WHEN s.sha256 = EXCLUDED.sha256
                             THEN s.previous_sha256
                             ELSE (array_remove(ARRAY[s.sha256] || s.previous_sha256, EXCLUDED.sha256))[1:8]
                         END,
                         last_changed_at = CASE WHEN s.sha256 = EXCLUDED.sha256
                             THEN s.last_changed_at ELSE EXCLUDED.last_changed_at END,
                         sha256 = EXCLUDED.sha256,
                         revision = EXCLUDED.revision,
                         name = EXCLUDED.name,
                         description = EXCLUDED.description,
                         license = EXCLUDED.license,
                         checks = EXCLUDED.checks,
                         blocking = EXCLUDED.blocking,
                         present = TRUE,
                         last_seen_at = EXCLUDED.last_seen_at",
                    &[
                        &format!("{publisher_id}:{}", skill.path),
                        &target.repository,
                        &skill.path,
                        &publisher_id,
                        revision,
                        &skill.sha256,
                        &skill.name,
                        &skill.description,
                        &license,
                        &checks,
                        &skill.blocking,
                        &now,
                    ],
                )
                .await?;
            }
            if crawl.complete {
                let seen: Vec<&str> = crawl.skills.iter().map(|s| s.path.as_str()).collect();
                tx.execute(
                    "UPDATE trace_skills_catalog_skills SET present = FALSE
                      WHERE repository = $1 AND NOT (path = ANY($2))",
                    &[&target.repository, &seen],
                )
                .await?;
            }
        }
        if !crawl.rate_limited() {
            let status = if crawl.revision.is_some() {
                "crawled"
            } else {
                "refused"
            };
            tx.execute(
                "UPDATE trace_skills_catalog_suggestions SET status = $2
                  WHERE repository = $1 AND status = 'pending'",
                &[&target.repository, &status],
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    /// Every repository with signals, and every skill, for the exporter.
    pub async fn load_for_export(
        &self,
    ) -> Result<(Vec<ExportRepository>, Vec<ExportSkill>), DatabaseError> {
        let mut client = self.pool.get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_skills_catalog_guard")
            .await?;
        let repositories = tx
            .query(
                "SELECT repository, source_kind, publisher_id, publisher_name, instructions_url,
                        skills_root, stars, pushed_at, created_at_github, archived, commits_7d,
                        commits_28d, COALESCE(signals_checked_at, crawled_at)
                   FROM trace_skills_catalog_repositories ORDER BY repository",
                &[],
            )
            .await?
            .into_iter()
            .map(|row| ExportRepository {
                repository: row.get(0),
                source_kind: kind_from(row.get::<_, &str>(1)),
                publisher_id: row.get(2),
                publisher_name: row.get(3),
                instructions_url: row.get(4),
                skills_root: row.get(5),
                signals: RepositorySignals {
                    stars: row
                        .get::<_, Option<i64>>(6)
                        .and_then(|v| u64::try_from(v).ok()),
                    pushed_at: row.get(7),
                    created_at: row.get(8),
                    archived: row.get(9),
                    commits_7d: row
                        .get::<_, Option<i32>>(10)
                        .and_then(|v| u32::try_from(v).ok()),
                    commits_28d: row
                        .get::<_, Option<i32>>(11)
                        .and_then(|v| u32::try_from(v).ok()),
                    checked_at: row.get(12),
                },
            })
            .collect();
        let rows = tx
            .query(
                "SELECT skill_id, publisher_id, repository, path, revision, sha256,
                        previous_sha256, name, description, license, checks, blocking, present,
                        first_seen_at, last_changed_at
                   FROM trace_skills_catalog_skills ORDER BY skill_id",
                &[],
            )
            .await?;
        tx.commit().await?;
        let mut skills = Vec::with_capacity(rows.len());
        for row in rows {
            let license: License = serde_json::from_value(row.get(9))
                .map_err(|_| DatabaseError::Pool("skills_catalog_license_decode".into()))?;
            let checks: StoredChecks = serde_json::from_value(row.get(10))
                .map_err(|_| DatabaseError::Pool("skills_catalog_checks_decode".into()))?;
            skills.push(ExportSkill {
                skill_id: row.get(0),
                publisher_id: row.get(1),
                repository: row.get(2),
                path: row.get(3),
                revision: row.get(4),
                sha256: row.get(5),
                previous_sha256: row.get(6),
                name: row.get(7),
                description: row.get(8),
                license,
                checks,
                blocking: row.get(11),
                present: row.get(12),
                first_seen_at: row.get(13),
                last_changed_at: row.get(14),
            });
        }
        Ok((repositories, skills))
    }
}
