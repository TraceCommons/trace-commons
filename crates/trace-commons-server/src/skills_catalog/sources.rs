// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Where the crawler looks: the reviewed source file, Claude Code plugin
//! marketplaces one hop deep, awesome-lists, and app suggestions. No
//! repository is found by searching GitHub.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::Value;
use trace_commons_protocol::skills_catalog::{
    SourceKind, normalize_repository, safe_directory_path,
};

/// Plugin entries read from one marketplace.
pub const MAX_MARKETPLACE_PLUGINS: usize = 200;
/// Repository links read from one list.
pub const MAX_LIST_LINKS: usize = 500;
/// Default marketplace file (Claude Code).
pub const CLAUDE_MARKETPLACE_PATH: &str = ".claude-plugin/marketplace.json";

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourcesFile {
    pub schema_version: u32,
    pub repositories: Vec<RepositorySource>,
    #[serde(default)]
    pub marketplaces: Vec<MarketplaceSource>,
    #[serde(default)]
    pub lists: Vec<ListSource>,
}

/// A repository named in the file. With a `publisher_id` it is a seed: a
/// publisher clients already know by that fixed id.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepositorySource {
    pub repository: String,
    #[serde(default)]
    pub publisher_id: Option<String>,
    #[serde(default)]
    pub publisher_name: Option<String>,
    #[serde(default)]
    pub skills_root: Option<String>,
    #[serde(default)]
    pub instructions_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarketplaceSource {
    pub repository: String,
    #[serde(default)]
    pub path: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListSource {
    pub repository: String,
    pub path: String,
}

/// One repository to crawl, after every source has been merged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrawlTarget {
    pub repository: String,
    pub kind: SourceKind,
    /// Fixed id and name for a seed; `None` for every other kind.
    pub seed: Option<Seed>,
    /// Directories whose `SKILL.md` files count; empty means the whole
    /// repository.
    pub roots: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    pub publisher_id: String,
    pub publisher_name: String,
    pub instructions_url: String,
}

impl SourcesFile {
    pub fn parse(bytes: &[u8]) -> Result<Self, &'static str> {
        let file: Self = serde_json::from_slice(bytes).map_err(|_| "sources-file-malformed")?;
        if file.schema_version != 1 {
            return Err("sources-file-version");
        }
        for source in &file.repositories {
            if normalize_repository(&source.repository).as_deref()
                != Some(source.repository.as_str())
            {
                return Err("sources-file-repository");
            }
            if source
                .skills_root
                .as_deref()
                .is_some_and(|root| !safe_directory_path(root))
            {
                return Err("sources-file-root");
            }
            if source.publisher_id.is_some()
                && (source.publisher_name.is_none() || source.instructions_url.is_none())
            {
                return Err("sources-file-seed");
            }
        }
        for repository in file
            .marketplaces
            .iter()
            .map(|m| &m.repository)
            .chain(file.lists.iter().map(|l| &l.repository))
        {
            if normalize_repository(repository).as_deref() != Some(repository.as_str()) {
                return Err("sources-file-repository");
            }
        }
        Ok(file)
    }

    /// The targets this file names directly (seeds and plain repositories).
    pub fn direct_targets(&self) -> Vec<CrawlTarget> {
        self.repositories
            .iter()
            .map(|source| CrawlTarget {
                repository: source.repository.clone(),
                kind: if source.publisher_id.is_some() {
                    SourceKind::Seed
                } else {
                    SourceKind::List
                },
                seed: source.publisher_id.as_ref().map(|id| Seed {
                    publisher_id: id.clone(),
                    publisher_name: source.publisher_name.clone().unwrap_or_default(),
                    instructions_url: source.instructions_url.clone().unwrap_or_default(),
                }),
                roots: source.skills_root.iter().cloned().collect(),
            })
            .collect()
    }
}

/// Merge targets naming the same repository. The most trusted kind wins; a
/// seed keeps only its own root, so a marketplace cannot widen a seeded
/// publisher; otherwise roots are unioned, and any whole-repository target
/// makes the whole repository count.
pub fn merge_targets(targets: impl IntoIterator<Item = CrawlTarget>) -> Vec<CrawlTarget> {
    let mut merged: BTreeMap<String, CrawlTarget> = BTreeMap::new();
    for target in targets {
        match merged.get_mut(&target.repository) {
            None => {
                merged.insert(target.repository.clone(), target);
            }
            Some(existing) => {
                if existing.seed.is_some() {
                    continue;
                }
                if target.seed.is_some() {
                    *existing = target;
                    continue;
                }
                existing.kind = existing.kind.min(target.kind);
                if existing.roots.is_empty() || target.roots.is_empty() {
                    existing.roots.clear();
                } else {
                    existing.roots.extend(target.roots);
                    existing.roots.sort();
                    existing.roots.dedup();
                }
            }
        }
    }
    merged.into_values().collect()
}

fn join(base: &str, rest: &str) -> String {
    match (base.is_empty(), rest.is_empty()) {
        (true, _) => rest.to_owned(),
        (_, true) => base.to_owned(),
        _ => format!("{base}/{rest}"),
    }
}

fn relative_dir(value: &str) -> Option<String> {
    let trimmed = value.trim_end_matches('/');
    if trimmed == "." || trimmed == "./" || trimmed.is_empty() {
        return Some(String::new());
    }
    let inner = trimmed.strip_prefix("./")?;
    safe_directory_path(inner).then(|| inner.to_owned())
}

/// The `(repository, skills root)` pairs a Claude Code marketplace file
/// lists. Only content on GitHub at a commit is followed: relative paths in
/// the marketplace repository, `github`, and `git-subdir`/`url` sources on
/// github.com. `npm`, `archive` and `command` sources are skipped.
pub fn parse_marketplace(bytes: &[u8], marketplace_repository: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_slice::<Value>(bytes) else {
        return Vec::new();
    };
    let Some(plugins) = value.get("plugins").and_then(Value::as_array) else {
        return Vec::new();
    };
    let plugin_root = value
        .pointer("/metadata/pluginRoot")
        .and_then(Value::as_str)
        .and_then(relative_dir);
    let mut found = Vec::new();
    for plugin in plugins.iter().take(MAX_MARKETPLACE_PLUGINS) {
        let Some(source) = plugin.get("source") else {
            continue;
        };
        let target = match source {
            Value::String(path) => {
                let directory = if path.starts_with('.') {
                    relative_dir(path)
                } else {
                    // A bare name resolves under metadata.pluginRoot.
                    plugin_root.as_ref().and_then(|root| {
                        (!path.contains('/') && safe_directory_path(path)).then(|| join(root, path))
                    })
                };
                directory.map(|dir| (marketplace_repository.to_owned(), join(&dir, "skills")))
            }
            Value::Object(object) => match object.get("source").and_then(Value::as_str) {
                Some("github") => object
                    .get("repo")
                    .and_then(Value::as_str)
                    .and_then(github_only)
                    .map(|repo| (repo, "skills".to_owned())),
                Some("url") => object
                    .get("url")
                    .and_then(Value::as_str)
                    .and_then(github_url)
                    .map(|repo| (repo, "skills".to_owned())),
                Some("git-subdir") => {
                    let repo = object
                        .get("url")
                        .and_then(Value::as_str)
                        .and_then(|url| github_url(url).or_else(|| github_only(url)));
                    let path = object
                        .get("path")
                        .and_then(Value::as_str)
                        .map(|p| p.trim_matches('/'))
                        .filter(|p| safe_directory_path(p));
                    repo.zip(path)
                        .map(|(repo, path)| (repo, join(path, "skills")))
                }
                _ => None,
            },
            _ => None,
        };
        if let Some(target) = target {
            found.push(target);
        }
    }
    found.sort();
    found.dedup();
    found
}

/// `owner/repo` shorthand only.
fn github_only(value: &str) -> Option<String> {
    if value.contains(':') {
        return None;
    }
    normalize_repository(value)
}

/// A github.com URL only, never another host.
fn github_url(value: &str) -> Option<String> {
    let rest = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("git@github.com:"))?;
    normalize_repository(rest)
}

/// Repositories linked from a Markdown list, in order of first appearance.
pub fn parse_list(text: &str, list_repository: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("https://github.com/") {
        let after = &rest[start + "https://github.com/".len()..];
        let end = after
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/')))
            .unwrap_or(after.len());
        let mut parts = after[..end].split('/');
        if let (Some(owner), Some(repo)) = (parts.next(), parts.next()) {
            if let Some(repository) = normalize_repository(&format!("{owner}/{repo}")) {
                if repository != list_repository && !found.contains(&repository) {
                    found.push(repository);
                    if found.len() == MAX_LIST_LINKS {
                        break;
                    }
                }
            }
        }
        rest = &after[end..];
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_sources_file_is_strict() {
        let good = br#"{"schema_version":1,"repositories":[
            {"repository":"obra/superpowers","publisher_id":"obra","publisher_name":"Obra",
             "skills_root":"skills","instructions_url":"https://github.com/obra/superpowers#installation"},
            {"repository":"acme/skills"}],
            "marketplaces":[{"repository":"acme/market"}]}"#;
        let file = SourcesFile::parse(good).unwrap();
        let targets = file.direct_targets();
        assert_eq!(targets[0].kind, SourceKind::Seed);
        assert_eq!(targets[0].roots, ["skills"]);
        assert_eq!(targets[1].kind, SourceKind::List);
        for bad in [
            &br#"{"schema_version":2,"repositories":[]}"#[..],
            br#"{"schema_version":1,"repositories":[{"repository":"Acme/X"}]}"#,
            br#"{"schema_version":1,"repositories":[{"repository":"acme/x","skills_root":"../x"}]}"#,
            br#"{"schema_version":1,"repositories":[{"repository":"acme/x","publisher_id":"x"}]}"#,
            br#"{"schema_version":1,"repositories":[],"extra":1}"#,
        ] {
            assert!(SourcesFile::parse(bad).is_err(), "{}", String::from_utf8_lossy(bad));
        }
    }

    #[test]
    fn the_shipped_sources_file_parses_and_seeds_todays_publishers() {
        let file = SourcesFile::parse(include_bytes!("../../skills-catalog/sources.json")).unwrap();
        let seeds: Vec<_> = file
            .direct_targets()
            .into_iter()
            .filter_map(|t| t.seed.map(|s| (s.publisher_id, t.repository, t.roots)))
            .collect();
        assert_eq!(
            seeds,
            [
                (
                    "poteto".into(),
                    "cursor/plugins".into(),
                    vec!["pstack/skills".to_owned()]
                ),
                (
                    "obra".into(),
                    "obra/superpowers".into(),
                    vec!["skills".to_owned()]
                ),
                (
                    "slack".into(),
                    "slackapi/slack-skills-plugin".into(),
                    vec!["skills".to_owned()]
                ),
            ]
        );
    }

    #[test]
    fn marketplace_sources_are_followed_only_to_github_content() {
        let market = br#"{
          "name": "m", "owner": {"name": "o"},
          "metadata": {"pluginRoot": "./plugins"},
          "plugins": [
            {"name": "a", "source": "./plugins/a"},
            {"name": "b", "source": "b"},
            {"name": "root", "source": "."},
            {"name": "c", "source": {"source": "github", "repo": "Other/C"}},
            {"name": "d", "source": {"source": "git-subdir", "url": "https://github.com/mono/repo.git", "path": "tools/d"}},
            {"name": "e", "source": {"source": "url", "url": "https://github.com/e/e.git"}},
            {"name": "f", "source": {"source": "url", "url": "https://gitlab.com/f/f.git"}},
            {"name": "g", "source": {"source": "npm", "package": "@g/g"}},
            {"name": "h", "source": {"source": "command", "command": "rm -rf ~"}},
            {"name": "i", "source": "./../escape"},
            {"name": "j", "source": {"source": "git-subdir", "url": "mono/repo", "path": "../x"}}
          ]}"#;
        assert_eq!(
            parse_marketplace(market, "acme/market"),
            [
                ("acme/market".to_owned(), "plugins/a/skills".to_owned()),
                ("acme/market".to_owned(), "plugins/b/skills".to_owned()),
                ("acme/market".to_owned(), "skills".to_owned()),
                ("e/e".to_owned(), "skills".to_owned()),
                ("mono/repo".to_owned(), "tools/d/skills".to_owned()),
                ("other/c".to_owned(), "skills".to_owned()),
            ]
        );
        assert!(parse_marketplace(b"not json", "acme/market").is_empty());
        let many = format!(
            r#"{{"plugins":[{}]}}"#,
            (0..300)
                .map(|n| format!(r#"{{"name":"p{n}","source":"./p{n}"}}"#))
                .collect::<Vec<_>>()
                .join(",")
        );
        assert_eq!(
            parse_marketplace(many.as_bytes(), "acme/market").len(),
            MAX_MARKETPLACE_PLUGINS
        );
    }

    #[test]
    fn list_links_are_repositories() {
        let text = "- [A](https://github.com/Acme/A) and https://github.com/acme/a/tree/main\n\
                    - https://github.com/b/b.git, https://github.com/self/list, https://github.com/only";
        assert_eq!(parse_list(text, "self/list"), ["acme/a", "b/b"]);
    }

    #[test]
    fn merging_keeps_the_most_trusted_kind_and_never_widens_a_seed() {
        let target = |repository: &str, kind, roots: &[&str], seed: bool| CrawlTarget {
            repository: repository.into(),
            kind,
            seed: seed.then(|| Seed {
                publisher_id: "s".into(),
                publisher_name: "S".into(),
                instructions_url: "https://x.example".into(),
            }),
            roots: roots.iter().map(|r| (*r).to_owned()).collect(),
        };
        let merged = merge_targets([
            target("a/a", SourceKind::Suggested, &[], false),
            target("a/a", SourceKind::Marketplace, &["x/skills"], false),
            target("s/s", SourceKind::Marketplace, &["other/skills"], false),
            target("s/s", SourceKind::Seed, &["skills"], true),
            target("s/s", SourceKind::Marketplace, &["more/skills"], false),
            target("m/m", SourceKind::Marketplace, &["b/skills"], false),
            target("m/m", SourceKind::Marketplace, &["a/skills"], false),
        ]);
        assert_eq!(
            merged[0],
            target("a/a", SourceKind::Marketplace, &[], false)
        );
        assert_eq!(
            merged[1],
            target(
                "m/m",
                SourceKind::Marketplace,
                &["a/skills", "b/skills"],
                false
            )
        );
        assert_eq!(
            merged[2],
            target("s/s", SourceKind::Seed, &["skills"], true)
        );
    }
}
