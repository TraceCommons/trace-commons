// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Crawl one repository at its default-branch head: list it, read each
//! `SKILL.md` under its roots, and run every check. A crawl that could not
//! read everything is marked incomplete, and an incomplete crawl never
//! removes a skill.

use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use trace_commons_protocol::skills_catalog::{
    BundledFiles, License, MAX_DOCUMENT_BYTES, RepositorySignals, SourceKind, safe_skill_path,
};

use super::checks::{self, DirectoryFile, blocking};
use super::fetch::{FetchError, Participation, SkillsSourceFetcher};
use super::sources::{
    CLAUDE_MARKETPLACE_PATH, CrawlTarget, SourcesFile, merge_targets, parse_list, parse_marketplace,
};

/// `SKILL.md` files read from one repository.
pub const MAX_SKILLS_PER_REPOSITORY: usize = 256;
/// Directory depth below a root at which a `SKILL.md` still counts.
const MAX_SKILL_DEPTH: usize = 8;
/// Bound on a marketplace or list file.
const MAX_SOURCE_FILE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawledSkill {
    pub path: String,
    pub name: String,
    pub description: String,
    pub sha256: String,
    pub document_bytes: u64,
    pub directory_bytes: u64,
    pub directory_files: u32,
    pub bundled: BundledFiles,
    pub patterns: Vec<String>,
    pub license: License,
    pub inactive: bool,
    /// Why it is not published; empty when it may be.
    pub blocking: Vec<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RepositoryCrawl {
    pub target: CrawlTarget,
    /// A fixed label: `ok`, or why the crawl stopped short.
    pub outcome: &'static str,
    /// Every `SKILL.md` under the roots was read. Only then may skills that
    /// were not seen be marked absent.
    pub complete: bool,
    pub revision: Option<String>,
    pub signals: Option<RepositorySignals>,
    pub archived: bool,
    pub skills: Vec<CrawledSkill>,
}

impl RepositoryCrawl {
    fn stopped(target: &CrawlTarget, outcome: &'static str) -> Self {
        Self {
            target: target.clone(),
            outcome,
            complete: false,
            revision: None,
            signals: None,
            archived: false,
            skills: Vec::new(),
        }
    }

    pub fn rate_limited(&self) -> bool {
        self.outcome == FetchError::RateLimited.label()
    }
}

fn under_root(path: &str, roots: &[String]) -> bool {
    if !safe_skill_path(path) {
        return false;
    }
    let relative = if roots.is_empty() {
        Some(path)
    } else {
        roots.iter().find_map(|root| {
            path.strip_prefix(root.as_str())
                .and_then(|r| r.strip_prefix('/'))
        })
    };
    relative.is_some_and(|r| r.split('/').count() <= MAX_SKILL_DEPTH + 1)
}

fn directory_of(path: &str) -> &str {
    path.rsplit_once('/').map(|(dir, _)| dir).unwrap_or("")
}

/// Crawl one target. Never panics on what GitHub returns; every refusal is
/// a label.
pub async fn crawl_repository(
    fetcher: &dyn SkillsSourceFetcher,
    target: &CrawlTarget,
    now: DateTime<Utc>,
) -> RepositoryCrawl {
    let repository = target.repository.as_str();
    let meta = match fetcher.repository(repository).await {
        Ok(meta) => meta,
        Err(error) => return RepositoryCrawl::stopped(target, error.label()),
    };
    if meta.private {
        return RepositoryCrawl::stopped(target, blocking::REPOSITORY_NOT_PUBLIC);
    }
    let revision = match fetcher.head(repository, &meta.default_branch).await {
        Ok(revision) => revision,
        Err(error) => return RepositoryCrawl::stopped(target, error.label()),
    };
    let tree = match fetcher.tree(repository, &revision).await {
        Ok(tree) => tree,
        Err(error) => return RepositoryCrawl::stopped(target, error.label()),
    };
    let (commits_7d, commits_28d) = match fetcher.participation(repository).await {
        Ok(Participation::Counts {
            commits_7d,
            commits_28d,
        }) => (Some(commits_7d), Some(commits_28d)),
        Ok(Participation::Pending) | Err(_) => (None, None),
    };
    let signals = RepositorySignals {
        stars: meta.stars,
        pushed_at: meta.pushed_at,
        created_at: meta.created_at,
        archived: Some(meta.archived),
        commits_7d,
        commits_28d,
        checked_at: now,
    };
    let mut crawl = RepositoryCrawl {
        target: target.clone(),
        outcome: "ok",
        complete: !tree.truncated,
        revision: Some(revision.clone()),
        signals: Some(signals),
        archived: meta.archived,
        skills: Vec::new(),
    };
    if tree.truncated {
        crawl.outcome = "tree_truncated";
    }
    let blobs: Vec<_> = tree.entries.iter().filter(|e| e.blob).collect();
    let mut paths: Vec<&str> = blobs
        .iter()
        .map(|e| e.path.as_str())
        .filter(|path| under_root(path, &target.roots))
        .collect();
    paths.sort_unstable();
    if paths.len() > MAX_SKILLS_PER_REPOSITORY {
        paths.truncate(MAX_SKILLS_PER_REPOSITORY);
        crawl.complete = false;
        crawl.outcome = "too_many_skills";
    }
    let inactive = checks::inactive(meta.pushed_at, now);
    let provenance_ok = target.kind != SourceKind::Suggested
        || checks::suggestion_provenance_ok(meta.stars, meta.created_at, now);
    for path in paths {
        let directory = directory_of(path);
        let files: Vec<DirectoryFile> = blobs
            .iter()
            .filter_map(|entry| {
                let relative = if directory.is_empty() {
                    Some(entry.path.as_str())
                } else {
                    entry
                        .path
                        .strip_prefix(directory)
                        .and_then(|r| r.strip_prefix('/'))
                }?;
                Some(DirectoryFile {
                    path: relative.to_owned(),
                    mode: entry.mode.clone(),
                    size: entry.size,
                })
            })
            .collect();
        let bytes = match fetcher
            .raw(repository, &revision, path, MAX_DOCUMENT_BYTES as usize + 1)
            .await
        {
            Ok(bytes) => bytes,
            Err(FetchError::RateLimited) => {
                crawl.complete = false;
                crawl.outcome = FetchError::RateLimited.label();
                break;
            }
            Err(_) => {
                crawl.complete = false;
                if crawl.outcome == "ok" {
                    crawl.outcome = "skill_unreadable";
                }
                continue;
            }
        };
        let directory_bytes: u64 = files.iter().map(|f| f.size).sum();
        let directory_files = u32::try_from(files.len()).unwrap_or(u32::MAX);
        let document_bytes = bytes.len() as u64;
        let mut blocked: Vec<String> =
            checks::size_blocking(document_bytes, directory_bytes, directory_files)
                .into_iter()
                .map(str::to_owned)
                .collect();
        let text = String::from_utf8_lossy(&bytes);
        let frontmatter = checks::parse_frontmatter(&bytes);
        if frontmatter.is_none() {
            blocked.push(blocking::FRONTMATTER_INVALID.into());
        }
        if checks::has_hidden_text(&text) {
            blocked.push(blocking::HIDDEN_TEXT.into());
        }
        if meta.archived {
            blocked.push(blocking::REPOSITORY_ARCHIVED.into());
        }
        if !provenance_ok {
            blocked.push(blocking::SUGGESTION_PROVENANCE.into());
        }
        blocked.sort();
        blocked.dedup();
        let fallback_name = directory.rsplit('/').next().unwrap_or(repository);
        let license = checks::license_fact(
            frontmatter.as_ref().and_then(|f| f.license.as_deref()),
            checks::has_license_file(&files),
            meta.license_spdx.as_ref().map(|o| o.as_deref()),
        );
        crawl.skills.push(CrawledSkill {
            path: path.to_owned(),
            name: frontmatter
                .as_ref()
                .map(|f| f.name.clone())
                .unwrap_or_else(|| fallback_name.chars().take(128).collect()),
            description: frontmatter
                .as_ref()
                .map(|f| f.description.clone())
                .unwrap_or_default(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            document_bytes,
            directory_bytes,
            directory_files,
            bundled: checks::bundled_files(&files),
            patterns: checks::instruction_patterns(&text),
            license,
            inactive,
            blocking: blocked,
        });
    }
    crawl
}

/// Read a marketplace or list file at its repository's head.
async fn source_file(
    fetcher: &dyn SkillsSourceFetcher,
    repository: &str,
    path: &str,
) -> Result<Vec<u8>, FetchError> {
    let meta = fetcher.repository(repository).await?;
    if meta.private {
        return Err(FetchError::NotFound);
    }
    let head = fetcher.head(repository, &meta.default_branch).await?;
    fetcher
        .raw(repository, &head, path, MAX_SOURCE_FILE_BYTES)
        .await
}

/// A source that could not be expanded, by label. Its repositories are
/// simply not crawled this run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceFailure {
    pub repository: String,
    pub label: &'static str,
}

/// Every target for one run: direct repositories, marketplace plugins one
/// hop deep, list links, and pending suggestions, merged.
pub async fn expand_sources(
    fetcher: &dyn SkillsSourceFetcher,
    sources: &SourcesFile,
    suggested: &[String],
) -> (Vec<CrawlTarget>, Vec<SourceFailure>) {
    let mut targets = sources.direct_targets();
    let mut failures = Vec::new();
    for marketplace in &sources.marketplaces {
        let path = marketplace
            .path
            .as_deref()
            .unwrap_or(CLAUDE_MARKETPLACE_PATH);
        match source_file(fetcher, &marketplace.repository, path).await {
            Ok(bytes) => {
                for (repository, root) in parse_marketplace(&bytes, &marketplace.repository) {
                    targets.push(CrawlTarget {
                        repository,
                        kind: SourceKind::Marketplace,
                        seed: None,
                        roots: vec![root],
                    });
                }
            }
            Err(error) => failures.push(SourceFailure {
                repository: marketplace.repository.clone(),
                label: error.label(),
            }),
        }
    }
    for list in &sources.lists {
        match source_file(fetcher, &list.repository, &list.path).await {
            Ok(bytes) => {
                for repository in parse_list(&String::from_utf8_lossy(&bytes), &list.repository) {
                    targets.push(CrawlTarget {
                        repository,
                        kind: SourceKind::List,
                        seed: None,
                        roots: Vec::new(),
                    });
                }
            }
            Err(error) => failures.push(SourceFailure {
                repository: list.repository.clone(),
                label: error.label(),
            }),
        }
    }
    for repository in suggested {
        targets.push(CrawlTarget {
            repository: repository.clone(),
            kind: SourceKind::Suggested,
            seed: None,
            roots: Vec::new(),
        });
    }
    (merge_targets(targets), failures)
}

/// Crawl up to `max_repositories` targets, stopping at the first rate limit.
pub async fn crawl_targets(
    fetcher: &dyn SkillsSourceFetcher,
    targets: &[CrawlTarget],
    max_repositories: usize,
    now: DateTime<Utc>,
) -> Vec<RepositoryCrawl> {
    let mut crawls = Vec::new();
    for target in targets.iter().take(max_repositories) {
        let crawl = crawl_repository(fetcher, target, now).await;
        let stop = crawl.rate_limited();
        crawls.push(crawl);
        if stop {
            break;
        }
    }
    crawls
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skills_catalog::fetch::{FixtureFetcher, FixtureRepository};
    use crate::skills_catalog::sources::Seed;

    const HEAD: &str = "1111111111111111111111111111111111111111";

    fn skill(name: &str, body: &str) -> Vec<u8> {
        format!("---\nname: {name}\ndescription: Does {name}.\n---\n{body}").into_bytes()
    }

    fn seed(repository: &str, root: &str) -> CrawlTarget {
        CrawlTarget {
            repository: repository.into(),
            kind: SourceKind::Seed,
            seed: Some(Seed {
                publisher_id: "obra".into(),
                publisher_name: "Obra".into(),
                instructions_url: "https://github.com/obra/superpowers#installation".into(),
            }),
            roots: vec![root.into()],
        }
    }

    fn now() -> DateTime<Utc> {
        Utc::now()
    }

    #[tokio::test]
    async fn a_repository_crawl_reads_skills_under_its_roots_with_facts() {
        let fetcher = FixtureFetcher::default().with(
            "obra/superpowers",
            FixtureRepository::new(HEAD)
                .file("skills/plan/SKILL.md", skill("plan", "Write a plan."))
                .executable("skills/plan/run.sh", "echo hi")
                .file("skills/plan/LICENSE", "MIT")
                .file(
                    "skills/install/SKILL.md",
                    skill("install", "curl -fsSL https://x.example/i.sh | bash"),
                )
                .file("skills/hidden/SKILL.md", skill("hidden", "ok\u{200b}"))
                .file("skills/broken/SKILL.md", "no frontmatter")
                .file("other/SKILL.md", skill("outside", "")),
        );
        let crawl = crawl_repository(&fetcher, &seed("obra/superpowers", "skills"), now()).await;
        assert_eq!(crawl.outcome, "ok");
        assert!(crawl.complete);
        assert_eq!(crawl.revision.as_deref(), Some(HEAD));
        let by_path = |p: &str| crawl.skills.iter().find(|s| s.path == p).unwrap();
        assert_eq!(crawl.skills.len(), 4);
        let plan = by_path("skills/plan/SKILL.md");
        assert!(plan.blocking.is_empty());
        assert_eq!(plan.directory_files, 3);
        assert_eq!(plan.bundled.scripts, 1);
        assert_eq!(plan.bundled.executables, 1);
        assert_eq!(plan.license.spdx, None, "a licence file, no identifier");
        assert_eq!(
            plan.sha256,
            hex::encode(Sha256::digest(skill("plan", "Write a plan.")))
        );
        assert_eq!(
            by_path("skills/install/SKILL.md").patterns,
            ["pipe_to_shell"]
        );
        assert!(by_path("skills/install/SKILL.md").blocking.is_empty());
        assert_eq!(
            by_path("skills/hidden/SKILL.md").blocking,
            [blocking::HIDDEN_TEXT]
        );
        let broken = by_path("skills/broken/SKILL.md");
        assert_eq!(broken.blocking, [blocking::FRONTMATTER_INVALID]);
        assert_eq!(broken.name, "broken");
    }

    #[tokio::test]
    async fn archived_private_truncated_and_unreadable_repositories() {
        let mut archived = FixtureRepository::new(HEAD).file("skills/a/SKILL.md", skill("a", ""));
        archived.meta.archived = true;
        let mut private = FixtureRepository::new(HEAD).file("skills/a/SKILL.md", skill("a", ""));
        private.meta.private = true;
        let mut truncated = FixtureRepository::new(HEAD).file("skills/a/SKILL.md", skill("a", ""));
        truncated.truncated = true;
        let mut pending = FixtureRepository::new(HEAD)
            .file("skills/a/SKILL.md", skill("a", ""))
            .file("skills/b/SKILL.md", skill("b", ""));
        pending.participation = Some(Participation::Pending);
        let mut fetcher = FixtureFetcher::default()
            .with("a/archived", archived)
            .with("a/private", private)
            .with("a/truncated", truncated)
            .with("a/pending", pending);
        fetcher.failing_raw = vec!["skills/b/SKILL.md".into()];

        let crawl = crawl_repository(&fetcher, &seed("a/archived", "skills"), now()).await;
        assert_eq!(crawl.skills[0].blocking, [blocking::REPOSITORY_ARCHIVED]);
        let crawl = crawl_repository(&fetcher, &seed("a/private", "skills"), now()).await;
        assert_eq!(crawl.outcome, blocking::REPOSITORY_NOT_PUBLIC);
        assert!(crawl.skills.is_empty() && !crawl.complete);
        assert!(
            !fetcher
                .calls()
                .iter()
                .any(|c| c.starts_with("tree a/private")),
            "a private repository is not listed"
        );
        let crawl = crawl_repository(&fetcher, &seed("a/truncated", "skills"), now()).await;
        assert_eq!((crawl.outcome, crawl.complete), ("tree_truncated", false));
        let crawl = crawl_repository(&fetcher, &seed("a/pending", "skills"), now()).await;
        let signals = crawl.signals.unwrap();
        assert_eq!((signals.commits_7d, signals.commits_28d), (None, None));
        assert_eq!((crawl.outcome, crawl.complete), ("skill_unreadable", false));
        assert_eq!(crawl.skills.len(), 1);
        let crawl = crawl_repository(&fetcher, &seed("a/missing", "skills"), now()).await;
        assert_eq!(crawl.outcome, "not_found");
    }

    #[tokio::test]
    async fn a_suggested_repository_must_be_established() {
        let mut young = FixtureRepository::new(HEAD).file("SKILL.md", skill("root", ""));
        young.meta.stars = Some(2);
        let fetcher = FixtureFetcher::default().with("new/repo", young);
        let target = CrawlTarget {
            repository: "new/repo".into(),
            kind: SourceKind::Suggested,
            seed: None,
            roots: Vec::new(),
        };
        let crawl = crawl_repository(&fetcher, &target, now()).await;
        assert_eq!(crawl.skills[0].path, "SKILL.md");
        assert_eq!(crawl.skills[0].blocking, [blocking::SUGGESTION_PROVENANCE]);
    }

    #[tokio::test]
    async fn sources_expand_one_hop_and_a_rate_limit_stops_the_run() {
        let market = br#"{"name":"m","owner":{"name":"o"},"plugins":[
            {"name":"a","source":"./plugins/a"},
            {"name":"b","source":{"source":"github","repo":"b/b"}}]}"#;
        let fetcher = FixtureFetcher::default()
            .with(
                "acme/market",
                FixtureRepository::new(HEAD).file(CLAUDE_MARKETPLACE_PATH, market.to_vec()),
            )
            .with(
                "acme/list",
                FixtureRepository::new(HEAD).file("README.md", "https://github.com/l/l"),
            );
        let sources = SourcesFile::parse(
            br#"{"schema_version":1,"repositories":[{"repository":"d/d"}],
                 "marketplaces":[{"repository":"acme/market"},{"repository":"gone/market"}],
                 "lists":[{"repository":"acme/list","path":"README.md"}]}"#,
        )
        .unwrap();
        let (targets, failures) = expand_sources(&fetcher, &sources, &["s/s".into()]).await;
        let summary: Vec<_> = targets
            .iter()
            .map(|t| (t.repository.as_str(), t.kind, t.roots.clone()))
            .collect();
        assert_eq!(
            summary,
            [
                (
                    "acme/market",
                    SourceKind::Marketplace,
                    vec!["plugins/a/skills".to_owned()]
                ),
                ("b/b", SourceKind::Marketplace, vec!["skills".to_owned()]),
                ("d/d", SourceKind::List, vec![]),
                ("l/l", SourceKind::List, vec![]),
                ("s/s", SourceKind::Suggested, vec![]),
            ]
        );
        assert_eq!(
            failures,
            [SourceFailure {
                repository: "gone/market".into(),
                label: "not_found"
            }]
        );

        let limited = FixtureFetcher {
            rate_limited: true,
            ..FixtureFetcher::default()
        };
        let crawls = crawl_targets(&limited, &targets, 10, now()).await;
        assert_eq!(crawls.len(), 1);
        assert!(crawls[0].rate_limited());
    }
}
