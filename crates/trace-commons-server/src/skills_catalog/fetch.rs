// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Every request the crawler makes goes through [`SkillsSourceFetcher`]. The
//! HTTP implementation talks to GitHub's REST API and raw content host with
//! no redirects and byte-bounded reads; [`FixtureFetcher`] serves tests, so
//! no test reaches the network.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde_json::Value;

/// Bound on one tree listing.
pub const MAX_TREE_BYTES: usize = 8 * 1024 * 1024;
/// Bound on one metadata response.
pub const MAX_METADATA_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FetchError {
    NotFound,
    RateLimited,
    Http,
    TooLarge,
    Malformed,
}

impl FetchError {
    pub fn label(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::RateLimited => "rate_limited",
            Self::Http => "http_error",
            Self::TooLarge => "too_large",
            Self::Malformed => "malformed",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RepositoryMeta {
    pub default_branch: String,
    pub private: bool,
    pub archived: bool,
    pub stars: Option<u64>,
    pub pushed_at: Option<DateTime<Utc>>,
    pub created_at: Option<DateTime<Utc>>,
    /// `None`: unknown. `Some(None)`: GitHub reports no licence.
    pub license_spdx: Option<Option<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeEntry {
    pub path: String,
    pub mode: String,
    pub blob: bool,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
    pub truncated: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Participation {
    Counts {
        commits_7d: u32,
        commits_28d: u32,
    },
    /// GitHub answered 202: still computing. Unknown, never zero.
    Pending,
}

#[async_trait]
pub trait SkillsSourceFetcher: Send + Sync {
    async fn repository(&self, repository: &str) -> Result<RepositoryMeta, FetchError>;
    /// The commit a branch points at.
    async fn head(&self, repository: &str, branch: &str) -> Result<String, FetchError>;
    async fn tree(&self, repository: &str, revision: &str) -> Result<Tree, FetchError>;
    async fn participation(&self, repository: &str) -> Result<Participation, FetchError>;
    /// A file's bytes at a commit, at most `limit` of them.
    async fn raw(
        &self,
        repository: &str,
        revision: &str,
        path: &str,
        limit: usize,
    ) -> Result<Vec<u8>, FetchError>;
}

/// GitHub over HTTPS. The token, when set, is sent only to the API base.
pub struct HttpFetcher {
    client: reqwest::Client,
    api_base: String,
    raw_base: String,
    token: Option<String>,
}

impl std::fmt::Debug for HttpFetcher {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpFetcher")
            .field("api_base", &self.api_base)
            .field("raw_base", &self.raw_base)
            .field("token", &self.token.as_ref().map(|_| "<set>"))
            .finish()
    }
}

impl HttpFetcher {
    pub const GITHUB_API: &'static str = "https://api.github.com";
    pub const GITHUB_RAW: &'static str = "https://raw.githubusercontent.com";

    pub fn new(api_base: &str, raw_base: &str, token: Option<String>) -> Result<Self, FetchError> {
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(20))
            .user_agent("TraceCommons-SkillsCatalog/1")
            .build()
            .map_err(|_| FetchError::Http)?;
        Ok(Self {
            client,
            api_base: api_base.trim_end_matches('/').to_owned(),
            raw_base: raw_base.trim_end_matches('/').to_owned(),
            token: token.filter(|t| !t.trim().is_empty()),
        })
    }

    pub fn github(token: Option<String>) -> Result<Self, FetchError> {
        Self::new(Self::GITHUB_API, Self::GITHUB_RAW, token)
    }

    async fn api(&self, path: &str, limit: usize) -> Result<(u16, Vec<u8>), FetchError> {
        let mut request = self
            .client
            .get(format!("{}{path}", self.api_base))
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28");
        if let Some(token) = &self.token {
            request = request.bearer_auth(token);
        }
        read_bounded(request.send().await.map_err(|_| FetchError::Http)?, limit).await
    }

    async fn api_json(&self, path: &str, limit: usize) -> Result<Value, FetchError> {
        let (status, body) = self.api(path, limit).await?;
        if status != 200 {
            return Err(FetchError::Http);
        }
        serde_json::from_slice(&body).map_err(|_| FetchError::Malformed)
    }
}

async fn read_bounded(
    mut response: reqwest::Response,
    limit: usize,
) -> Result<(u16, Vec<u8>), FetchError> {
    let status = response.status().as_u16();
    match status {
        200 | 202 => {}
        404 | 451 => return Err(FetchError::NotFound),
        403 | 429 => return Err(FetchError::RateLimited),
        _ => return Err(FetchError::Http),
    }
    if response.content_length().is_some_and(|n| n > limit as u64) {
        return Err(FetchError::TooLarge);
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| FetchError::Http)? {
        if body.len().saturating_add(chunk.len()) > limit {
            return Err(FetchError::TooLarge);
        }
        body.extend_from_slice(&chunk);
    }
    Ok((status, body))
}

fn time(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

fn hex40(value: &str) -> bool {
    value.len() == 40
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The fields the crawler reads from `GET /repos/{owner}/{repo}`.
pub fn parse_repository(value: &Value) -> Result<RepositoryMeta, FetchError> {
    let default_branch = value
        .get("default_branch")
        .and_then(Value::as_str)
        .filter(|b| !b.is_empty() && b.len() <= 255 && !b.contains(".."))
        .ok_or(FetchError::Malformed)?
        .to_owned();
    let license_spdx = match value.get("license") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(license) => Some(
            license
                .get("spdx_id")
                .and_then(Value::as_str)
                .map(str::to_owned),
        ),
    };
    Ok(RepositoryMeta {
        default_branch,
        private: value
            .get("private")
            .and_then(Value::as_bool)
            .unwrap_or(true),
        archived: value
            .get("archived")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        stars: value.get("stargazers_count").and_then(Value::as_u64),
        pushed_at: value.get("pushed_at").and_then(time),
        created_at: value.get("created_at").and_then(time),
        license_spdx,
    })
}

pub fn parse_ref(value: &Value) -> Result<String, FetchError> {
    let object = value.get("object").ok_or(FetchError::Malformed)?;
    if object.get("type").and_then(Value::as_str) != Some("commit") {
        return Err(FetchError::Malformed);
    }
    object
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| hex40(sha))
        .map(str::to_owned)
        .ok_or(FetchError::Malformed)
}

pub fn parse_tree(value: &Value) -> Result<Tree, FetchError> {
    let truncated = value
        .get("truncated")
        .and_then(Value::as_bool)
        .ok_or(FetchError::Malformed)?;
    let nodes = value
        .get("tree")
        .and_then(Value::as_array)
        .ok_or(FetchError::Malformed)?;
    let entries = nodes
        .iter()
        .filter_map(|node| {
            Some(TreeEntry {
                path: node.get("path")?.as_str()?.to_owned(),
                mode: node.get("mode")?.as_str()?.to_owned(),
                blob: node.get("type")?.as_str()? == "blob",
                size: node.get("size").and_then(Value::as_u64).unwrap_or(0),
            })
        })
        .collect();
    Ok(Tree { entries, truncated })
}

pub fn parse_participation(value: &Value) -> Result<Participation, FetchError> {
    let all = value
        .get("all")
        .and_then(Value::as_array)
        .filter(|weeks| weeks.len() >= 4)
        .ok_or(FetchError::Malformed)?;
    let weeks: Option<Vec<u32>> = all
        .iter()
        .rev()
        .take(4)
        .map(|w| w.as_u64().and_then(|n| u32::try_from(n).ok()))
        .collect();
    let weeks = weeks.ok_or(FetchError::Malformed)?;
    Ok(Participation::Counts {
        commits_7d: weeks[0],
        commits_28d: weeks.iter().fold(0u32, |sum, n| sum.saturating_add(*n)),
    })
}

#[async_trait]
impl SkillsSourceFetcher for HttpFetcher {
    async fn repository(&self, repository: &str) -> Result<RepositoryMeta, FetchError> {
        parse_repository(
            &self
                .api_json(&format!("/repos/{repository}"), MAX_METADATA_BYTES)
                .await?,
        )
    }

    async fn head(&self, repository: &str, branch: &str) -> Result<String, FetchError> {
        let encoded: String = branch
            .split('/')
            .map(url_path_segment)
            .collect::<Vec<_>>()
            .join("/");
        parse_ref(
            &self
                .api_json(
                    &format!("/repos/{repository}/git/ref/heads/{encoded}"),
                    MAX_METADATA_BYTES,
                )
                .await?,
        )
    }

    async fn tree(&self, repository: &str, revision: &str) -> Result<Tree, FetchError> {
        if !hex40(revision) {
            return Err(FetchError::Malformed);
        }
        parse_tree(
            &self
                .api_json(
                    &format!("/repos/{repository}/git/trees/{revision}?recursive=1"),
                    MAX_TREE_BYTES,
                )
                .await?,
        )
    }

    async fn participation(&self, repository: &str) -> Result<Participation, FetchError> {
        let (status, body) = self
            .api(
                &format!("/repos/{repository}/stats/participation"),
                MAX_METADATA_BYTES,
            )
            .await?;
        if status == 202 {
            return Ok(Participation::Pending);
        }
        parse_participation(&serde_json::from_slice(&body).map_err(|_| FetchError::Malformed)?)
    }

    async fn raw(
        &self,
        repository: &str,
        revision: &str,
        path: &str,
        limit: usize,
    ) -> Result<Vec<u8>, FetchError> {
        if !hex40(revision) || !trace_commons_protocol::skills_catalog::safe_directory_path(path) {
            return Err(FetchError::Malformed);
        }
        let response = self
            .client
            .get(format!("{}/{repository}/{revision}/{path}", self.raw_base))
            .send()
            .await
            .map_err(|_| FetchError::Http)?;
        let (status, body) = read_bounded(response, limit).await?;
        if status != 200 {
            return Err(FetchError::Http);
        }
        Ok(body)
    }
}

fn url_path_segment(part: &str) -> String {
    part.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

/// One repository as a fixture serves it.
#[derive(Debug, Clone)]
pub struct FixtureRepository {
    pub meta: RepositoryMeta,
    pub head: String,
    pub truncated: bool,
    /// Path to (mode, bytes).
    pub files: BTreeMap<String, (String, Vec<u8>)>,
    pub participation: Option<Participation>,
}

impl FixtureRepository {
    pub fn new(head: &str) -> Self {
        Self {
            meta: RepositoryMeta {
                default_branch: "main".into(),
                private: false,
                archived: false,
                stars: Some(50),
                pushed_at: Some(Utc::now() - chrono::Duration::days(2)),
                created_at: Some(Utc::now() - chrono::Duration::days(400)),
                license_spdx: Some(Some("MIT".into())),
            },
            head: head.into(),
            truncated: false,
            files: BTreeMap::new(),
            participation: Some(Participation::Counts {
                commits_7d: 2,
                commits_28d: 9,
            }),
        }
    }

    pub fn file(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.files
            .insert(path.into(), ("100644".into(), bytes.into()));
        self
    }

    pub fn executable(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.files
            .insert(path.into(), ("100755".into(), bytes.into()));
        self
    }
}

/// Serves fixed repositories and records each request, so a test can assert
/// what was and was not fetched. Never touches the network.
#[derive(Debug, Default)]
pub struct FixtureFetcher {
    pub repositories: BTreeMap<String, FixtureRepository>,
    /// Paths whose raw read fails, to exercise incomplete crawls.
    pub failing_raw: Vec<String>,
    pub rate_limited: bool,
    pub calls: Mutex<Vec<String>>,
}

impl FixtureFetcher {
    pub fn with(mut self, repository: &str, fixture: FixtureRepository) -> Self {
        self.repositories.insert(repository.into(), fixture);
        self
    }

    fn get(&self, call: String, repository: &str) -> Result<&FixtureRepository, FetchError> {
        self.calls.lock().expect("calls lock").push(call);
        if self.rate_limited {
            return Err(FetchError::RateLimited);
        }
        self.repositories
            .get(repository)
            .ok_or(FetchError::NotFound)
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait]
impl SkillsSourceFetcher for FixtureFetcher {
    async fn repository(&self, repository: &str) -> Result<RepositoryMeta, FetchError> {
        Ok(self
            .get(format!("repo {repository}"), repository)?
            .meta
            .clone())
    }

    async fn head(&self, repository: &str, branch: &str) -> Result<String, FetchError> {
        let fixture = self.get(format!("head {repository} {branch}"), repository)?;
        Ok(fixture.head.clone())
    }

    async fn tree(&self, repository: &str, revision: &str) -> Result<Tree, FetchError> {
        let fixture = self.get(format!("tree {repository} {revision}"), repository)?;
        if fixture.head != revision {
            return Err(FetchError::NotFound);
        }
        Ok(Tree {
            truncated: fixture.truncated,
            entries: fixture
                .files
                .iter()
                .map(|(path, (mode, bytes))| TreeEntry {
                    path: path.clone(),
                    mode: mode.clone(),
                    blob: true,
                    size: bytes.len() as u64,
                })
                .collect(),
        })
    }

    async fn participation(&self, repository: &str) -> Result<Participation, FetchError> {
        self.get(format!("participation {repository}"), repository)?
            .participation
            .ok_or(FetchError::Http)
    }

    async fn raw(
        &self,
        repository: &str,
        revision: &str,
        path: &str,
        limit: usize,
    ) -> Result<Vec<u8>, FetchError> {
        let fixture = self.get(format!("raw {repository} {revision} {path}"), repository)?;
        if fixture.head != revision || self.failing_raw.iter().any(|p| p == path) {
            return Err(FetchError::NotFound);
        }
        let (_, bytes) = fixture.files.get(path).ok_or(FetchError::NotFound)?;
        if bytes.len() > limit {
            return Err(FetchError::TooLarge);
        }
        Ok(bytes.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    /// A one-shot local server that answers with `response` and hands back
    /// the request it saw.
    async fn serve(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = vec![0; 8192];
            let n = socket.read(&mut request).await.unwrap();
            let _ = socket.write_all(&response).await;
            String::from_utf8_lossy(&request[..n]).into_owned()
        });
        (base, task)
    }

    fn reply(status: &str, body: &str) -> Vec<u8> {
        format!(
            "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        )
        .into_bytes()
    }

    #[tokio::test]
    async fn the_http_fetcher_refuses_a_redirect_and_sends_the_token_only_to_the_api() {
        let (base, task) = serve(
            b"HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/x\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                .to_vec(),
        )
        .await;
        let fetcher = HttpFetcher::new(&base, &base, Some("secret-token".into())).unwrap();
        assert_eq!(
            fetcher
                .raw("acme/skills", &"a".repeat(40), "SKILL.md", 1024)
                .await
                .unwrap_err(),
            FetchError::Http
        );
        let request = task.await.unwrap().to_lowercase();
        assert!(!request.contains("authorization:"), "{request}");
        assert!(!request.contains("cookie:"));

        let (base, task) = serve(reply(
            "200 OK",
            r#"{"default_branch":"main","private":false,"license":null}"#,
        ))
        .await;
        let fetcher = HttpFetcher::new(&base, &base, Some("secret-token".into())).unwrap();
        let meta = fetcher.repository("acme/skills").await.unwrap();
        assert_eq!(meta.license_spdx, Some(None));
        assert!(
            task.await
                .unwrap()
                .to_lowercase()
                .contains("authorization: bearer secret-token")
        );
    }

    #[tokio::test]
    async fn the_http_fetcher_bounds_a_body_and_maps_status() {
        let (base, task) = serve(reply("200 OK", &"x".repeat(2048))).await;
        let fetcher = HttpFetcher::new(&base, &base, None).unwrap();
        assert_eq!(
            fetcher
                .raw("acme/skills", &"a".repeat(40), "SKILL.md", 1024)
                .await
                .unwrap_err(),
            FetchError::TooLarge
        );
        task.await.unwrap();
        let (base, task) = serve(reply("202 Accepted", "{}")).await;
        let fetcher = HttpFetcher::new(&base, &base, None).unwrap();
        assert_eq!(
            fetcher.participation("acme/skills").await.unwrap(),
            Participation::Pending
        );
        task.await.unwrap();
        let (base, task) = serve(reply("403 Forbidden", "{}")).await;
        let fetcher = HttpFetcher::new(&base, &base, None).unwrap();
        assert_eq!(
            fetcher.repository("acme/skills").await.unwrap_err(),
            FetchError::RateLimited
        );
        task.await.unwrap();
    }

    #[test]
    fn github_responses_parse_and_refuse_bad_shapes() {
        let meta = parse_repository(&serde_json::json!({
            "default_branch": "main", "private": false, "archived": true,
            "stargazers_count": 7, "pushed_at": "2026-11-01T00:00:00Z",
            "license": {"spdx_id": "Apache-2.0"}
        }))
        .unwrap();
        assert!(meta.archived);
        assert_eq!(meta.stars, Some(7));
        assert_eq!(meta.license_spdx, Some(Some("Apache-2.0".into())));
        // A response without `private` is treated as private.
        assert!(
            parse_repository(&serde_json::json!({"default_branch": "main"}))
                .unwrap()
                .private
        );
        assert!(parse_repository(&serde_json::json!({})).is_err());
        assert_eq!(
            parse_ref(&serde_json::json!({"object": {"type": "commit", "sha": "a".repeat(40)}}))
                .unwrap(),
            "a".repeat(40)
        );
        assert!(
            parse_ref(&serde_json::json!({"object": {"type": "tag", "sha": "a".repeat(40)}}))
                .is_err()
        );
        assert!(parse_tree(&serde_json::json!({"tree": []})).is_err());
        let weeks: Vec<u32> = (1..=52).collect();
        assert_eq!(
            parse_participation(&serde_json::json!({"all": weeks})).unwrap(),
            Participation::Counts {
                commits_7d: 52,
                commits_28d: 52 + 51 + 50 + 49
            }
        );
    }
}
