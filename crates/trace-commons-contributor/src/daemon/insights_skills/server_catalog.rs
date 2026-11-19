//! The server's signed Skills catalog, as the daemon fetches, verifies and
//! keeps it (spec 2026-11-15, "Daemon").
//!
//! One anonymous GET of a fixed URL: no credential, cookie, proxy or
//! redirect, and nothing derived from inventory, follows or views. Every
//! install downloads the same whole object. A document is used only after a
//! signature under a key this build pins verifies, for this build's
//! environment, while current; a revision below the highest ever accepted is
//! refused. With no pinned key every document is refused and the daemon keeps
//! fetching the four publishers from GitHub.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::skills_catalog::{
    self as catalog, CatalogDocument, Environment, MAX_CATALOG_BYTES, SourceKind,
};

use super::{CatalogSkill, Publisher, safe_skill_path, source_url};

pub(crate) const SERVER_CATALOG_FLAG: &str = "skills_server_catalog";
const MAX_SEED_SKILLS: usize = super::MAX_SKILLS;

fn cache_path(dir: &Path, environment: Environment) -> PathBuf {
    dir.join(format!(
        "insights-skills-catalog-{}-v1.json",
        environment.as_str()
    ))
}

fn watermark_path(dir: &Path, environment: Environment) -> PathBuf {
    dir.join(format!(
        "insights-skills-catalog-{}-watermark.json",
        environment.as_str()
    ))
}

/// The highest revision ever accepted, and its payload digest. Kept apart
/// from the cache so an expired or deleted cache still refuses a rollback.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Watermark {
    revision: u64,
    digest: String,
}

fn read_small(path: &Path, limit: usize) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= limit).then_some(bytes)
}

fn watermark(dir: &Path, environment: Environment) -> Option<Watermark> {
    serde_json::from_slice(&read_small(&watermark_path(dir, environment), 4096)?).ok()
}

/// The cached document, re-verified now. `None` when absent, unverifiable,
/// for another environment, or no longer current.
pub(crate) fn load_cached(
    dir: &Path,
    environment: Environment,
    keys: &[[u8; 32]],
    now: DateTime<Utc>,
) -> Option<CatalogDocument> {
    let bytes = read_small(&cache_path(dir, environment), MAX_CATALOG_BYTES)?;
    let (document, digest) = catalog::parse_signed_catalog(&bytes, keys, environment, now).ok()?;
    // A cache older than the watermark is a rollback, not a cache.
    let floor = watermark(dir, environment);
    if floor.is_some_and(|w| {
        w.revision > document.revision || (w.revision == document.revision && w.digest != digest)
    }) {
        return None;
    }
    Some(document)
}

/// Verify a downloaded document, refuse a rollback, then persist the signed
/// bytes and the watermark. Returns the document on success.
pub(crate) fn accept(
    dir: &Path,
    environment: Environment,
    keys: &[[u8; 32]],
    bytes: &[u8],
    now: DateTime<Utc>,
) -> Result<CatalogDocument, &'static str> {
    let (document, digest) =
        catalog::parse_signed_catalog(bytes, keys, environment, now).map_err(|error| error.0)?;
    if let Some(floor) = watermark(dir, environment) {
        if document.revision < floor.revision {
            return Err("skills-catalog-rollback");
        }
        if document.revision == floor.revision && digest != floor.digest {
            return Err("skills-catalog-revision-reused");
        }
    }
    crate::config::write_atomic_0600(dir, &cache_path(dir, environment), bytes)
        .map_err(|_| "skills-catalog-cache-write-failed")?;
    let mark = serde_json::to_vec(&Watermark {
        revision: document.revision,
        digest,
    })
    .map_err(|_| "skills-catalog-cache-write-failed")?;
    crate::config::write_atomic_0600(dir, &watermark_path(dir, environment), &mark)
        .map_err(|_| "skills-catalog-cache-write-failed")?;
    Ok(document)
}

pub(crate) fn client(https_only: bool) -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .https_only(https_only)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(20))
        .user_agent("TraceCommons-Skills/1")
        .build()
}

/// One anonymous, bounded GET of the catalog object.
pub(crate) async fn download(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, ()> {
    let mut response = client
        .get(url)
        .header(reqwest::header::ACCEPT, "application/json")
        .send()
        .await
        .map_err(|_| ())?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(());
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("")
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_owned();
    if mime != "application/json" {
        return Err(());
    }
    if response
        .content_length()
        .is_some_and(|n| n > MAX_CATALOG_BYTES as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) > MAX_CATALOG_BYTES {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

/// One seeded publisher's skills from the server catalog, in today's
/// `CatalogSkill` form (id `<publisher>:<path>`, the same source and
/// instruction URLs), so follows, the feed and hash history carry on.
/// `None` when the catalog does not list the publisher as that seed, which
/// a caller treats as a failed check, never as "no skills".
pub(crate) fn seed_skills(
    document: &CatalogDocument,
    publisher: &Publisher,
) -> Option<Vec<CatalogSkill>> {
    document.publishers.iter().find(|p| {
        p.id == publisher.id
            && p.repository == publisher.repository
            && p.source_kind == SourceKind::Seed
    })?;
    let mut skills: Vec<CatalogSkill> = document
        .skills
        .iter()
        .filter(|skill| {
            skill.publisher_id == publisher.id
                && skill.repository == publisher.repository
                && safe_skill_path(&skill.path, publisher.skills_root)
        })
        .map(|skill| CatalogSkill {
            id: format!("{}:{}", publisher.id, skill.path),
            publisher_id: publisher.id.into(),
            name: skill.name.clone(),
            description: skill.description.clone(),
            repository: publisher.repository.into(),
            path: skill.path.clone(),
            revision: skill.revision.clone(),
            sha256: skill.sha256.clone(),
            source_url: source_url(publisher, &skill.revision, &skill.path),
            instructions_url: publisher.instructions_url.into(),
        })
        .collect();
    skills.truncate(MAX_SEED_SKILLS);
    Some(skills)
}

/// A catalog skill as a snapshot row: today's fields plus the server's
/// facts. `instructions_url` is the publisher's.
pub(crate) fn skill_row(
    skill: &catalog::CatalogSkill,
    instructions_url: &str,
    checks_version: u32,
) -> serde_json::Value {
    let mut row = serde_json::json!({
        "id": skill.id,
        "publisher_id": skill.publisher_id,
        "name": skill.name,
        "description": skill.description,
        "repository": skill.repository,
        "path": skill.path,
        "revision": skill.revision,
        "sha256": skill.sha256,
        "source_url": skill.source_url,
        "instructions_url": instructions_url,
    });
    add_facts(&mut row, skill, checks_version);
    row
}

/// Merge the server's facts into an existing row.
pub(crate) fn add_facts(
    row: &mut serde_json::Value,
    skill: &catalog::CatalogSkill,
    checks_version: u32,
) {
    row["license"] = serde_json::to_value(&skill.license).unwrap_or_default();
    row["first_seen_at"] = serde_json::to_value(skill.first_seen_at).unwrap_or_default();
    row["last_changed_at"] = serde_json::to_value(skill.last_changed_at).unwrap_or_default();
    row["checks"] = serde_json::to_value(&skill.checks).unwrap_or_default();
    row["checks_version"] = checks_version.into();
}

/// Repository signals in the shape the publisher-state rows carry them.
pub(crate) fn signals_row(signals: &catalog::RepositorySignals) -> serde_json::Value {
    let mut row = serde_json::Map::new();
    if let Some(stars) = signals.stars {
        row.insert("stars".into(), stars.into());
    }
    if let Some(at) = signals.pushed_at {
        row.insert(
            "pushed_at".into(),
            serde_json::to_value(at).unwrap_or_default(),
        );
    }
    row.insert(
        "repository_checked_at".into(),
        serde_json::to_value(signals.checked_at).unwrap_or_default(),
    );
    if let (Some(week), Some(month)) = (signals.commits_7d, signals.commits_28d) {
        row.insert("commits_7d".into(), week.into());
        row.insert("commits_28d".into(), month.into());
        row.insert(
            "activity_checked_at".into(),
            serde_json::to_value(signals.checked_at).unwrap_or_default(),
        );
    }
    serde_json::Value::Object(row)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use chrono::Duration as ChronoDuration;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    use serde_json::json;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A throwaway key for this test run only. Never a real catalog key.
    pub(crate) fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    pub(crate) fn public(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }

    const REV: &str = "0123456789abcdef0123456789abcdef01234567";

    pub(crate) fn document(issued: DateTime<Utc>) -> serde_json::Value {
        let issued = DateTime::from_timestamp(issued.timestamp(), 0).unwrap();
        let skill = |publisher: &str, repository: &str, path: &str| {
            json!({
                "id": format!("{publisher}:{path}"), "publisher_id": publisher,
                "name": "plan", "description": "Plans.", "repository": repository,
                "path": path, "revision": REV, "sha256": "a".repeat(64),
                "previous_sha256": ["b".repeat(64)],
                "source_url": format!("https://github.com/{repository}/blob/{REV}/{path}"),
                "license": {"status": "declared", "spdx": "MIT"},
                "first_seen_at": issued - ChronoDuration::days(3),
                "last_changed_at": issued - ChronoDuration::days(1),
                "checks": {"frontmatter": "valid", "document_bytes": 10, "directory_bytes": 10,
                           "directory_files": 1, "bundled": {"scripts": 0, "executables": 0, "binaries": 0},
                           "patterns": [], "inactive": false}
            })
        };
        json!({
            "schema_version": 1, "checks_version": 2, "environment": "testing",
            "revision": issued.timestamp(), "issued_at": issued,
            "expires_at": issued + ChronoDuration::days(7),
            "publishers": [
                {"id": "gh.acme.skills", "name": "acme/skills", "repository": "acme/skills",
                 "url": "https://github.com/acme/skills", "instructions_url": "https://github.com/acme/skills",
                 "source_kind": "marketplace",
                 "signals": {"stars": 40, "commits_7d": 1, "commits_28d": 3, "checked_at": issued}},
                {"id": "obra", "name": "Obra", "repository": "obra/superpowers",
                 "url": "https://github.com/obra/superpowers",
                 "instructions_url": "https://github.com/obra/superpowers#installation",
                 "skills_root": "skills", "source_kind": "seed",
                 "signals": {"stars": 900, "checked_at": issued}}
            ],
            "skills": [
                skill("gh.acme.skills", "acme/skills", "lint/SKILL.md"),
                skill("obra", "obra/superpowers", "skills/plan/SKILL.md"),
            ]
        })
    }

    pub(crate) fn signed(key: &Ed25519KeyPair, value: &serde_json::Value) -> Vec<u8> {
        let payload = serde_json::to_string(value).unwrap();
        catalog::assemble_signed_catalog(
            payload.clone(),
            vec![catalog::CatalogSignature {
                key_id: catalog::key_id_for(&public(key)),
                signature: hex::encode(
                    key.sign(&catalog::signing_input(payload.as_bytes()))
                        .as_ref(),
                ),
            }],
        )
    }

    #[test]
    fn accept_requires_a_pinned_key_and_refuses_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        let now = Utc::now();
        let current = signed(&signer, &document(now));
        assert_eq!(
            accept(dir.path(), Environment::Testing, &[], &current, now).unwrap_err(),
            "skills-catalog-no-trusted-key"
        );
        assert_eq!(
            accept(
                dir.path(),
                Environment::Testing,
                &[public(&key())],
                &current,
                now
            )
            .unwrap_err(),
            "skills-catalog-signature"
        );
        assert_eq!(
            accept(
                dir.path(),
                Environment::Staging,
                &[public(&signer)],
                &current,
                now
            )
            .unwrap_err(),
            "skills-catalog-identity"
        );
        let accepted = accept(
            dir.path(),
            Environment::Testing,
            &[public(&signer)],
            &current,
            now,
        )
        .unwrap();
        assert_eq!(accepted.skills.len(), 2);
        assert!(load_cached(dir.path(), Environment::Testing, &[public(&signer)], now).is_some());
        assert!(load_cached(dir.path(), Environment::Testing, &[], now).is_none());
        // Expired: the cache no longer counts.
        assert!(
            load_cached(
                dir.path(),
                Environment::Testing,
                &[public(&signer)],
                now + ChronoDuration::days(8)
            )
            .is_none()
        );
        let older = signed(&signer, &document(now - ChronoDuration::hours(1)));
        assert_eq!(
            accept(
                dir.path(),
                Environment::Testing,
                &[public(&signer)],
                &older,
                now
            )
            .unwrap_err(),
            "skills-catalog-rollback"
        );
        let mut changed = document(now);
        changed["skills"][0]["name"] = json!("other");
        assert_eq!(
            accept(
                dir.path(),
                Environment::Testing,
                &[public(&signer)],
                &signed(&signer, &changed),
                now
            )
            .unwrap_err(),
            "skills-catalog-revision-reused"
        );
        // The same document again is fine.
        assert!(
            accept(
                dir.path(),
                Environment::Testing,
                &[public(&signer)],
                &current,
                now
            )
            .is_ok()
        );
    }

    #[test]
    fn seeds_keep_todays_ids_and_urls() {
        let document: CatalogDocument = serde_json::from_value(document(Utc::now())).unwrap();
        let obra = super::super::publishers()
            .into_iter()
            .find(|p| p.id == "obra")
            .unwrap();
        let skills = seed_skills(&document, &obra).unwrap();
        assert_eq!(skills.len(), 1);
        assert_eq!(skills[0].id, "obra:skills/plan/SKILL.md");
        assert_eq!(
            skills[0].source_url,
            source_url(&obra, REV, "skills/plan/SKILL.md")
        );
        assert_eq!(skills[0].instructions_url, obra.instructions_url);
        let slack = super::super::publishers()
            .into_iter()
            .find(|p| p.id == "slack")
            .unwrap();
        assert!(
            seed_skills(&document, &slack).is_none(),
            "absent is not empty"
        );
    }

    async fn serve(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!(
            "http://{}/skills-catalog/testing.json",
            listener.local_addr().unwrap()
        );
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(&response).await;
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (url, task)
    }

    #[tokio::test]
    async fn download_is_anonymous_bounded_and_refuses_redirects() {
        let body = "{}";
        let (url, task) = serve(
            format!("HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                .into_bytes(),
        )
        .await;
        assert_eq!(
            download(&client(false).unwrap(), &url).await.unwrap(),
            b"{}"
        );
        let request = task.await.unwrap().to_lowercase();
        assert!(request.starts_with("get /skills-catalog/testing.json "));
        for header in ["authorization:", "cookie:"] {
            assert!(!request.contains(header), "{request}");
        }
        let (url, task) = serve(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        )
        .await;
        assert!(download(&client(false).unwrap(), &url).await.is_err());
        task.await.unwrap();
        let mut huge =
            b"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nConnection: close\r\n\r\n"
                .to_vec();
        huge.extend(vec![b' '; MAX_CATALOG_BYTES + 1]);
        let (url, task) = serve(huge).await;
        assert!(download(&client(false).unwrap(), &url).await.is_err());
        task.await.unwrap();
        let (url, task) = serve(
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_vec(),
        )
        .await;
        assert!(download(&client(false).unwrap(), &url).await.is_err());
        task.await.unwrap();
        // The production client refuses plain HTTP outright.
        assert!(
            download(&client(true).unwrap(), "http://127.0.0.1:1/x")
                .await
                .is_err()
        );
    }
}
