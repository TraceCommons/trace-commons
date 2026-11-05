//! Passive local Skills inventory and a bounded, curated public metadata cache.
//! Catalog refreshes never send installed paths/content, authenticate, install,
//! execute publisher code, or follow redirects. Only public metadata is persisted.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::insights::skills::{
    CatalogSkill, Inventory, freshness, parse_skill_document, scan_inventory_with_policy,
};

const STATE_FILE: &str = "insights-skills-public-v1.json";
const MAX_STATE_BYTES: usize = 2 * 1024 * 1024;
const MAX_TREE_BYTES: usize = 4 * 1024 * 1024;
const MAX_SKILL_BYTES: usize = 128 * 1024;
const MAX_SKILLS: usize = 128;
const MAX_FEED: usize = 256;
const REFRESH_AFTER: Duration = Duration::from_secs(6 * 60 * 60);
const REFRESH_DEBOUNCE: Duration = Duration::from_secs(30);
const REFRESH_TOTAL: Duration = Duration::from_secs(90);
const ERROR_REFRESH: &str = "insights-skills-refresh-failed";
const ERROR_CACHE: &str = "insights-skills-cache-unavailable";

#[derive(Clone, Serialize)]
struct Publisher {
    id: &'static str,
    name: &'static str,
    repository: &'static str,
    skills_root: &'static str,
    instructions_url: &'static str,
}

fn publishers() -> [Publisher; 4] {
    [
        Publisher {
            id: "poteto",
            name: "Poteto",
            repository: "cursor/plugins",
            skills_root: "pstack/skills",
            instructions_url: "https://github.com/cursor/plugins/blob/main/pstack/README.md",
        },
        Publisher {
            id: "obra",
            name: "Obra",
            repository: "obra/superpowers",
            skills_root: "skills",
            instructions_url: "https://github.com/obra/superpowers#installation",
        },
        Publisher {
            id: "slack",
            name: "Slack",
            repository: "slackapi/slack-skills-plugin",
            skills_root: "skills",
            instructions_url: "https://github.com/slackapi/slack-skills-plugin#installation",
        },
        Publisher {
            id: "rex",
            name: "Rex",
            repository: "",
            skills_root: "",
            instructions_url: "https://www.superlogical.com/rex/docs/automate/agent-skill",
        },
    ]
}

#[derive(Clone, Serialize, Deserialize)]
struct PublisherState {
    status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    checked_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Serialize, Deserialize)]
struct FeedItem {
    id: String,
    publisher_id: String,
    skill_id: String,
    kind: String,
    observed_at: DateTime<Utc>,
}

#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicState {
    #[serde(default)]
    skills: Vec<CatalogSkill>,
    #[serde(default)]
    followed: BTreeSet<String>,
    #[serde(default)]
    publisher_states: BTreeMap<String, PublisherState>,
    #[serde(default)]
    history: BTreeMap<String, Vec<String>>,
    #[serde(default)]
    feed: Vec<FeedItem>,
    checked_at: Option<DateTime<Utc>>,
}

impl PublicState {
    fn follow(&mut self, id: &str, followed: bool) -> Result<(), &'static str> {
        if !publishers().iter().any(|publisher| publisher.id == id) {
            return Err("insights-skills-publisher-invalid");
        }
        if followed {
            self.followed.insert(id.into());
        } else {
            self.followed.remove(id);
        }
        Ok(())
    }

    fn accept(&mut self, publisher: &str, skills: Vec<CatalogSkill>, now: DateTime<Utc>) {
        for skill in &skills {
            let previous = self.skills.iter().find(|old| old.id == skill.id);
            let kind = match previous {
                None => Some("added"),
                Some(old) if old.sha256 != skill.sha256 => {
                    let history = self.history.entry(skill.id.clone()).or_default();
                    if !history.contains(&old.sha256) {
                        history.push(old.sha256.clone());
                    }
                    if history.len() > 8 {
                        history.remove(0);
                    }
                    Some("updated")
                }
                _ => None,
            };
            if let Some(kind) = kind {
                self.feed.insert(
                    0,
                    FeedItem {
                        id: format!("{}:{}:{}", skill.id, now.timestamp_millis(), kind),
                        publisher_id: publisher.into(),
                        skill_id: skill.id.clone(),
                        kind: kind.into(),
                        observed_at: now,
                    },
                );
            }
        }
        self.feed.truncate(MAX_FEED);
        self.skills.retain(|skill| skill.publisher_id != publisher);
        self.skills.extend(skills);
        self.skills.sort_by(|left, right| left.id.cmp(&right.id));
        let ids: BTreeSet<_> = self.skills.iter().map(|skill| skill.id.clone()).collect();
        self.history.retain(|id, _| ids.contains(id));
        self.publisher_states.insert(
            publisher.into(),
            PublisherState {
                status: "current".into(),
                checked_at: Some(now),
            },
        );
        self.checked_at = Some(now);
    }

    fn failed(&mut self, publisher: &str) {
        self.publisher_states
            .entry(publisher.into())
            .and_modify(|state| state.status = "unknown".into())
            .or_insert(PublisherState {
                status: "unknown".into(),
                checked_at: None,
            });
    }
}

#[derive(Default)]
struct Runtime {
    loaded: bool,
    public: PublicState,
    refreshing: bool,
    last_attempt: Option<Instant>,
    error: Option<&'static str>,
    history_targets: Vec<HistoryTarget>,
    history_requested: BTreeSet<String>,
    inventory: Option<(Instant, InventoryPolicy, Inventory)>,
}

/// A leaf lock: never held while taking daemon settings, policy, or counter locks.
#[derive(Default)]
pub(crate) struct SkillsService {
    runtime: Mutex<Runtime>,
}

fn regular_destination(dir: &Path) -> Result<PathBuf, ()> {
    let metadata = std::fs::symlink_metadata(dir).map_err(|_| ())?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(());
    }
    let path = dir.join(STATE_FILE);
    match std::fs::symlink_metadata(&path) {
        Ok(meta) if meta.file_type().is_symlink() || !meta.is_file() => Err(()),
        Ok(_) => Ok(path),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(path),
        Err(_) => Err(()),
    }
}

fn load(dir: &Path) -> Result<PublicState, ()> {
    let path = regular_destination(dir)?;
    if !path.exists() {
        return Ok(PublicState::default());
    }
    let file = crate::evidence_import::open_import_file(&path).map_err(|_| ())?;
    if file.metadata().map_err(|_| ())?.len() > MAX_STATE_BYTES as u64 {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(MAX_STATE_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ())?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err(());
    }
    let state: PublicState = serde_json::from_slice(&bytes).map_err(|_| ())?;
    validate_state(&state)?;
    Ok(state)
}

fn validate_state(state: &PublicState) -> Result<(), ()> {
    if state.skills.len() > MAX_SKILLS * 3
        || state.feed.len() > MAX_FEED
        || state.history.len() > MAX_SKILLS * 3
    {
        return Err(());
    }
    let publishers = publishers();
    for skill in &state.skills {
        let publisher = publishers
            .iter()
            .find(|publisher| publisher.id == skill.publisher_id)
            .ok_or(())?;
        if publisher.repository.is_empty()
            || skill.repository != publisher.repository
            || !safe_skill_path(&skill.path, publisher.skills_root)
            || !is_hex(&skill.revision, 40)
            || !is_hex(&skill.sha256, 64)
            || skill.id != format!("{}:{}", publisher.id, skill.path)
            || skill.source_url != source_url(publisher, &skill.revision, &skill.path)
            || skill.instructions_url != publisher.instructions_url
            || skill.name.len() > 512
            || skill.description.len() > 16 * 1024
        {
            return Err(());
        }
    }
    for (id, hashes) in &state.history {
        if !state.skills.iter().any(|skill| &skill.id == id)
            || hashes.len() > 8
            || hashes.iter().any(|hash| !is_hex(hash, 64))
        {
            return Err(());
        }
    }
    if state
        .followed
        .iter()
        .any(|id| !publishers.iter().any(|publisher| publisher.id == id))
        || state
            .publisher_states
            .keys()
            .any(|id| !publishers.iter().any(|publisher| publisher.id == id))
        || state.feed.iter().any(|item| {
            item.id.len() > 1024
                || item.skill_id.len() > 512
                || !publishers
                    .iter()
                    .any(|publisher| publisher.id == item.publisher_id)
                || !matches!(item.kind.as_str(), "added" | "updated")
        })
    {
        return Err(());
    }
    Ok(())
}

fn persist(dir: &Path, state: &PublicState) -> Result<(), ()> {
    let path = regular_destination(dir)?;
    let bytes = serde_json::to_vec(state).map_err(|_| ())?;
    if bytes.len() > MAX_STATE_BYTES {
        return Err(());
    }
    crate::config::write_atomic_0600(dir, &path, &bytes).map_err(|_| ())
}

fn is_hex(value: &str, count: usize) -> bool {
    value.len() == count && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn safe_skill_path(path: &str, root: &str) -> bool {
    path.len() <= 512
        && path.starts_with(&format!("{root}/"))
        && path.ends_with("/SKILL.md")
        && path.split('/').all(|part| {
            !part.is_empty()
                && part != "."
                && part != ".."
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
        })
}

fn skill_paths(tree: &Value, root: &str) -> Result<Vec<String>, ()> {
    if tree.get("truncated").and_then(Value::as_bool) != Some(false) {
        return Err(());
    }
    let nodes = tree.get("tree").and_then(Value::as_array).ok_or(())?;
    let mut paths = Vec::new();
    for node in nodes {
        if node.get("type").and_then(Value::as_str) != Some("blob")
            || !matches!(
                node.get("mode").and_then(Value::as_str),
                Some("100644" | "100755")
            )
        {
            continue;
        }
        if let Some(path) = node
            .get("path")
            .and_then(Value::as_str)
            .filter(|path| safe_skill_path(path, root))
        {
            paths.push(path.to_owned());
        }
    }
    paths.sort();
    paths.dedup();
    if paths.len() > MAX_SKILLS {
        return Err(());
    }
    Ok(paths)
}

fn source_url(publisher: &Publisher, revision: &str, path: &str) -> String {
    format!(
        "https://github.com/{}/blob/{revision}/{path}",
        publisher.repository
    )
}

async fn fetch_bytes(client: &reqwest::Client, url: String, limit: usize) -> Result<Vec<u8>, ()> {
    // URLs are constructed solely from fixed publishers, validated commits and
    // safe tree paths. No ambient API token/cookie or inherited proxy is used.
    let mut response = client.get(url).send().await.map_err(|_| ())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > limit as u64)
    {
        return Err(());
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(());
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

fn resolve_revision(commit: &Value) -> Result<&str, ()> {
    let object = commit.get("object").ok_or(())?;
    if object.get("type").and_then(Value::as_str) != Some("commit") {
        return Err(());
    }
    object
        .get("sha")
        .and_then(Value::as_str)
        .filter(|sha| is_hex(sha, 40))
        .ok_or(())
}

async fn fetch_publisher(
    client: &reqwest::Client,
    publisher: &Publisher,
) -> Result<Vec<CatalogSkill>, ()> {
    let commit: Value = serde_json::from_slice(
        &fetch_bytes(
            client,
            format!(
                "https://api.github.com/repos/{}/git/ref/heads/main",
                publisher.repository
            ),
            64 * 1024,
        )
        .await?,
    )
    .map_err(|_| ())?;
    let revision = resolve_revision(&commit)?;
    let tree: Value = serde_json::from_slice(
        &fetch_bytes(
            client,
            format!(
                "https://api.github.com/repos/{}/git/trees/{revision}?recursive=1",
                publisher.repository
            ),
            MAX_TREE_BYTES,
        )
        .await?,
    )
    .map_err(|_| ())?;
    let paths = skill_paths(&tree, publisher.skills_root)?;
    let mut skills = Vec::new();
    let mut total = 0usize;
    // Sequential requests bound concurrency to one. A publisher is committed
    // only after every listed entry has been fetched and parsed successfully.
    for path in paths {
        let bytes = fetch_bytes(
            client,
            format!(
                "https://raw.githubusercontent.com/{}/{revision}/{path}",
                publisher.repository
            ),
            MAX_SKILL_BYTES,
        )
        .await?;
        total = total.saturating_add(bytes.len());
        if total > MAX_STATE_BYTES {
            return Err(());
        }
        let (name, description) = parse_skill_document(&bytes).ok_or(())?;
        skills.push(CatalogSkill {
            id: format!("{}:{path}", publisher.id),
            publisher_id: publisher.id.into(),
            name,
            description,
            repository: publisher.repository.into(),
            source_url: source_url(publisher, revision, &path),
            path,
            revision: revision.into(),
            sha256: hex::encode(Sha256::digest(&bytes)),
            instructions_url: publisher.instructions_url.into(),
        });
    }
    Ok(skills)
}

impl SkillsService {
    fn state(&self, dir: &Path) -> Result<std::sync::MutexGuard<'_, Runtime>, ()> {
        let mut runtime = self.runtime.lock().map_err(|_| ())?;
        if !runtime.loaded {
            match load(dir) {
                Ok(public) => runtime.public = public,
                Err(()) => runtime.error = Some(ERROR_CACHE),
            }
            runtime.loaded = true;
        }
        Ok(runtime)
    }

    fn start_refresh(self: &Arc<Self>, shared: &DaemonShared, force: bool) {
        if !enabled(shared) {
            return;
        }
        let dir = shared.store.dir().to_owned();
        let Ok(mut runtime) = self.state(&dir) else {
            return;
        };
        if runtime.refreshing || runtime.error == Some(ERROR_CACHE) {
            return;
        }
        let pending_history = runtime
            .history_targets
            .iter()
            .any(|target| !runtime.history_requested.contains(&target.key()));
        let wait = if force || pending_history {
            REFRESH_DEBOUNCE
        } else {
            REFRESH_AFTER
        };
        if runtime
            .last_attempt
            .is_some_and(|last| last.elapsed() < wait)
        {
            return;
        }
        if !force
            && !pending_history
            && runtime.last_attempt.is_none()
            && runtime.public.checked_at.is_some_and(|at| {
                Utc::now().signed_duration_since(at).num_seconds() < REFRESH_AFTER.as_secs() as i64
            })
        {
            return;
        }
        let history_targets = runtime.history_targets.clone();
        for target in &history_targets {
            runtime.history_requested.insert(target.key());
        }
        runtime.last_attempt = Some(Instant::now());
        runtime.refreshing = true;
        drop(runtime);
        // Test builds exercise the actual admission/throttle path while never
        // contacting a publisher. Explicit live qualification calls fetch directly.
        if cfg!(any(test, feature = "test-credential-store")) {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.refreshing = false;
            }
            return;
        }
        let service = Arc::clone(self);
        let spawned = shared.spawn_detached(move |shared| async move {
            let client = reqwest::Client::builder()
                .https_only(true)
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(12))
                .user_agent("TraceCommons-Skills/1")
                .build();
            let mut results = Vec::new();
            if let Ok(client) = client {
                let work = async {
                    for publisher in publishers()
                        .iter()
                        .filter(|publisher| !publisher.repository.is_empty())
                    {
                        if !enabled(&shared) {
                            break;
                        }
                        let fetched = fetch_publisher(&client, publisher).await;
                        let mut histories = BTreeMap::new();
                        if let Ok(skills) = &fetched {
                            for target in history_targets
                                .iter()
                                .filter(|target| target.publisher_id == publisher.id)
                            {
                                if let Some(skill) = skills.iter().find(|skill| {
                                    skill.path == target.path && skill.sha256 != target.sha256
                                }) {
                                    if let Ok(hashes) =
                                        fetch_history(&client, publisher, skill).await
                                    {
                                        histories.insert(skill.id.clone(), hashes);
                                    }
                                }
                            }
                        }
                        results.push((publisher.id, fetched, histories));
                    }
                };
                let _ = tokio::time::timeout(REFRESH_TOTAL, work).await;
            }
            let publish = enabled(&shared);
            let Ok(mut runtime) = service.runtime.lock() else {
                return;
            };
            runtime.refreshing = false;
            // A refresh admitted while enabled may finish after a live disable.
            // Discard its feature results and leave the durable cache untouched.
            if !publish {
                runtime.last_attempt = None;
                for target in &history_targets {
                    runtime.history_requested.remove(&target.key());
                }
                return;
            }
            let now = Utc::now();
            let mut failed = false;
            for publisher in publishers()
                .iter()
                .filter(|publisher| !publisher.repository.is_empty())
            {
                match results
                    .iter_mut()
                    .find(|(id, _, _)| id == &publisher.id)
                    .map(|(_, result, histories)| (result, histories))
                {
                    Some((Ok(skills), histories)) => {
                        runtime
                            .public
                            .accept(publisher.id, std::mem::take(skills), now);
                        for (id, hashes) in histories {
                            let stored = runtime.public.history.entry(id.clone()).or_default();
                            for hash in hashes.iter() {
                                if !stored.contains(hash) {
                                    stored.push(hash.clone());
                                }
                            }
                            if stored.len() > 8 {
                                stored.drain(..stored.len() - 8);
                            }
                        }
                    }
                    _ => {
                        runtime.public.failed(publisher.id);
                        failed = true;
                    }
                }
            }
            runtime.refreshing = false;
            runtime.error = if persist(&dir, &runtime.public).is_err() {
                Some(ERROR_CACHE)
            } else if failed {
                Some(ERROR_REFRESH)
            } else {
                None
            };
        });
        if spawned.is_none() {
            if let Ok(mut runtime) = self.runtime.lock() {
                runtime.refreshing = false;
                runtime.last_attempt = None;
            }
        }
    }
}

#[derive(Clone)]
struct HistoryTarget {
    publisher_id: &'static str,
    path: String,
    sha256: String,
}

impl HistoryTarget {
    fn key(&self) -> String {
        format!("{}:{}:{}", self.publisher_id, self.path, self.sha256)
    }
}

fn history_targets(inventory: &Inventory) -> Vec<HistoryTarget> {
    let mut targets = Vec::new();
    for installed in &inventory.skills {
        let Some(source) = &installed.source else {
            continue;
        };
        let Some(publisher) = publishers().into_iter().find(|publisher| {
            !publisher.repository.is_empty()
                && source.repository == publisher.repository
                && safe_skill_path(&source.path, publisher.skills_root)
        }) else {
            continue;
        };
        if !targets.iter().any(|target: &HistoryTarget| {
            target.publisher_id == publisher.id && target.path == source.path
        }) {
            targets.push(HistoryTarget {
                publisher_id: publisher.id,
                path: source.path.clone(),
                sha256: installed.sha256.clone(),
            });
        }
        if targets.len() >= 8 {
            break;
        }
    }
    targets
}

async fn fetch_history(
    client: &reqwest::Client,
    publisher: &Publisher,
    skill: &CatalogSkill,
) -> Result<Vec<String>, ()> {
    // The path has already matched a fixed public repository and curated root.
    // Only that public path crosses the network; installed bytes/hashes do not.
    let commits: Value = serde_json::from_slice(
        &fetch_bytes(
            client,
            format!(
                "https://api.github.com/repos/{}/commits?path={}&per_page=4",
                publisher.repository, skill.path
            ),
            256 * 1024,
        )
        .await?,
    )
    .map_err(|_| ())?;
    let commits = commits.as_array().ok_or(())?;
    let mut hashes = Vec::new();
    for commit in commits.iter().take(4) {
        let revision = commit
            .get("sha")
            .and_then(Value::as_str)
            .filter(|sha| is_hex(sha, 40))
            .ok_or(())?;
        if revision == skill.revision {
            continue;
        }
        if hashes.len() >= 3 {
            break;
        }
        let bytes = fetch_bytes(
            client,
            format!(
                "https://raw.githubusercontent.com/{}/{revision}/{}",
                publisher.repository, skill.path
            ),
            MAX_SKILL_BYTES,
        )
        .await?;
        hashes.push(hex::encode(Sha256::digest(&bytes)));
    }
    Ok(hashes)
}

/// Called by the existing daemon poll; public refresh does not require opening
/// the Skills tab, and has the same Insights gate and six-hour throttle.
pub(crate) fn maybe_refresh_public(shared: &DaemonShared) {
    if enabled(shared) {
        shared.insights_skills.start_refresh(shared, false);
    }
}

fn enabled(shared: &DaemonShared) -> bool {
    !shared.dev_dry_run
        && shared
            .feature_flags
            .snapshot()
            .is_enabled("insights_skills")
        && shared
            .settings
            .lock()
            .map(|settings| settings.insights_counter_pass)
            .unwrap_or(false)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct InventoryPolicy {
    projects: Vec<PathBuf>,
    denied_roots: Vec<PathBuf>,
    all_local_excluded: bool,
}

fn inventory_policy(shared: &DaemonShared) -> Result<InventoryPolicy, ()> {
    let policy = shared.policy.lock().map_err(|_| ())?;
    if policy.holds_every_send() {
        return Ok(InventoryPolicy {
            all_local_excluded: true,
            ..InventoryPolicy::default()
        });
    }
    let denied_roots = match super::insights_week::never_folders(&policy) {
        super::insights_week::NeverFolders::Every => return Err(()),
        super::insights_week::NeverFolders::These(keys) => {
            // Negative rules must never be silently truncated.
            if keys.len() > 4096 {
                return Err(());
            }
            keys.iter()
                .map(|key| {
                    super::policy::display_path_for_key(key)
                        .map(PathBuf::from)
                        .filter(|path| path.is_absolute())
                        .ok_or(())
                })
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    let queue = shared.queue.lock().map_err(|_| ())?;
    let mut keys = super::policy::known_keys(
        &policy,
        queue.all().iter().map(|entry| entry.project_key.clone()),
    );
    if let Some(grant) = &policy.automatic_grant {
        keys.extend(grant.projects_on_disk.iter().cloned());
    }
    keys.sort();
    keys.dedup();
    let projects = keys
        .into_iter()
        .filter(|key| {
            key != super::policy::UNKNOWN_PROJECT_KEY
                && policy.resolve(key) != super::policy::ProjectMode::Ignore
        })
        .filter_map(|key| super::policy::display_path_for_key(&key).map(PathBuf::from))
        .take(256)
        .collect();
    Ok(InventoryPolicy {
        projects,
        denied_roots,
        all_local_excluded: false,
    })
}

fn snapshot(shared: &DaemonShared) -> Value {
    if !enabled(shared) {
        return json!({"enabled":false,"copy":copy()});
    }
    let policy = match inventory_policy(shared) {
        Ok(policy) => policy,
        Err(()) => {
            return json!({"enabled":true,"readable":false,"reason":"insights-skills-policy-unavailable","copy":copy()});
        }
    };
    let cached = shared
        .insights_skills
        .state(shared.store.dir())
        .ok()
        .and_then(|runtime| {
            runtime
                .inventory
                .as_ref()
                .filter(|(at, roots, _)| {
                    at.elapsed() < Duration::from_secs(300) && roots == &policy
                })
                .map(|(_, _, inventory)| inventory.clone())
        });
    let inventory = cached.unwrap_or_else(|| {
        let mut extra = Vec::new();
        // Explicit environment roots only; tests use the same isolated home as
        // session discovery and never inherit a developer's configured roots.
        if !cfg!(any(test, feature = "test-credential-store")) {
            if let Some(path) = std::env::var_os("SKILLS_HOME").filter(|path| !path.is_empty()) {
                extra.push(("shared".into(), PathBuf::from(path)));
            }
            if let Some(path) = std::env::var_os("CODEX_HOME").filter(|path| !path.is_empty()) {
                extra.push(("codex".into(), PathBuf::from(path).join("skills")));
            }
        }
        let inventory = if policy.all_local_excluded {
            Inventory {
                skills: Vec::new(),
                partial: true,
                coverage: vec![crate::insights::skills::ScanCoverage {
                    location: "local".into(),
                    status: crate::insights::skills::ScanStatus::PolicyExcluded,
                }],
            }
        } else {
            scan_inventory_with_policy(
                &crate::source::home_dir(),
                &policy.projects,
                &extra,
                &policy.denied_roots,
            )
        };
        if let Ok(mut runtime) = shared.insights_skills.state(shared.store.dir()) {
            runtime.inventory = Some((Instant::now(), policy, inventory.clone()));
        }
        inventory
    });
    if let Ok(mut runtime) = shared.insights_skills.state(shared.store.dir()) {
        runtime.history_targets = history_targets(&inventory);
    }
    let usage_response = super::insights_week::skills_usage(shared);
    if usage_response.get("readable").and_then(Value::as_bool) == Some(false) {
        return json!({"enabled":true,"readable":false,"reason":usage_response.get("reason"),"copy":copy()});
    }
    let mut usage = usage_response
        .get("usage")
        .cloned()
        .unwrap_or_else(|| json!({"partial":true,"skills":[]}));
    usage["coverage"] = Value::Array(
        usage_response
            .get("coverage")
            .and_then(Value::as_object)
            .map(|coverage| {
                coverage
                    .iter()
                    .map(|(agent, status)| json!({"agent":agent,"status":status}))
                    .collect()
            })
            .unwrap_or_default(),
    );
    let Ok(runtime) = shared.insights_skills.state(shared.store.dir()) else {
        return json!({"enabled":true,"readable":false,"reason":ERROR_CACHE,"copy":copy()});
    };
    let now = Utc::now();
    let publisher_states: Vec<_> = publishers().iter().map(|publisher| {
        let state = runtime.public.publisher_states.get(publisher.id);
        let status = if publisher.repository.is_empty() { "unavailable" } else { state.map_or("not_checked", |state| {
            if state.status == "current" && state.checked_at.is_some_and(|at| now.signed_duration_since(at).num_seconds() >= REFRESH_AFTER.as_secs() as i64) { "unknown" } else { &state.status }
        }) };
        json!({"publisher_id":publisher.id,"status":status,"checked_at":state.and_then(|state| state.checked_at)})
    }).collect();
    let (installed, installed_partial) = bounded_rows(inventory.skills.iter(), 240 * 1024);
    let (coverage, coverage_partial) = bounded_rows(inventory.coverage.iter(), 40 * 1024);
    let (catalog, catalog_partial) = bounded_rows(runtime.public.skills.iter(), 180 * 1024);
    let (updates, updates_partial) =
        bounded_rows(updates(&inventory, &runtime.public, now), 64 * 1024);
    let (feed, feed_partial) = bounded_rows(
        runtime
            .public
            .feed
            .iter()
            .filter(|item| runtime.public.followed.contains(&item.publisher_id)),
        48 * 1024,
    );
    if let Some(rows) = usage.get("skills").and_then(Value::as_array) {
        let (rows, partial) = bounded_rows(rows, 96 * 1024);
        usage["skills"] = rows.into();
        if partial {
            usage["partial"] = true.into();
        }
    }
    json!({
        "enabled":true,"readable":true,"inventory":{"skills":installed,"coverage":coverage,"partial":inventory.partial || installed_partial || coverage_partial || updates_partial},
        "catalog":{"publishers":publishers(),"skills":catalog,"partial":catalog_partial,"checked_at":runtime.public.checked_at,"publisher_states":publisher_states},
        "followed_publishers":runtime.public.followed,"updates":updates,"feed":feed,"feed_partial":feed_partial,
        "usage":usage,
        "refreshing":runtime.refreshing,"error":runtime.error,"copy":copy(),
    })
}

fn bounded_rows<T: Serialize>(
    rows: impl IntoIterator<Item = T>,
    limit: usize,
) -> (Vec<Value>, bool) {
    let mut result = Vec::new();
    let mut bytes = 2usize;
    for row in rows {
        let Ok(body) = serde_json::to_vec(&row) else {
            return (result, true);
        };
        bytes = bytes.saturating_add(body.len()).saturating_add(1);
        if bytes > limit {
            return (result, true);
        }
        let Ok(value) = serde_json::from_slice(&body) else {
            return (result, true);
        };
        result.push(value);
    }
    (result, false)
}

fn updates(inventory: &Inventory, state: &PublicState, now: DateTime<Utc>) -> Vec<Value> {
    inventory.skills.iter().map(|installed| {
        let catalog = installed.source.as_ref().and_then(|source| state.skills.iter().find(|skill| source.repository == skill.repository && source.path == skill.path));
        let status = catalog.map(|skill| {
            let checked = state.publisher_states.get(&skill.publisher_id);
            if checked.is_some_and(|state| state.status == "current" && state.checked_at.is_some_and(|at| now.signed_duration_since(at).num_seconds() < REFRESH_AFTER.as_secs() as i64)) {
                serde_json::to_value(freshness(installed, skill, state.history.get(&skill.id).map(Vec::as_slice).unwrap_or(&[]))).unwrap_or(json!("unknown"))
            } else { json!("unknown") }
        }).unwrap_or_else(|| {
            match installed.source.as_ref() {
                None => json!("unknown_source"),
                Some(source) if publishers().iter().any(|publisher| !publisher.repository.is_empty() && source.repository == publisher.repository && safe_skill_path(&source.path, publisher.skills_root)) => json!("not_checked"),
                Some(_) => json!("unavailable"),
            }
        });
        json!({"installed_id":installed.id,"status":status,"catalog_skill_id":catalog.map(|skill| &skill.id)})
    }).collect()
}

pub(crate) async fn handle(shared: &DaemonShared, req: &Request) -> Response {
    let force = match req.params.get("refresh") {
        None => false,
        Some(Value::Bool(value)) => *value,
        _ => return Response::err(req.id, ERR_BAD_PARAMS, "insights-skills-refresh-invalid"),
    };
    if force && enabled(shared) {
        if let Ok(mut runtime) = shared.insights_skills.runtime.lock() {
            runtime.inventory = None;
        }
    }
    let mut value = super::run_blocking(|| snapshot(shared));
    if enabled(shared) {
        shared.insights_skills.start_refresh(shared, force);
        if let Ok(runtime) = shared.insights_skills.runtime.lock() {
            value["refreshing"] = runtime.refreshing.into();
        }
    }
    if !enabled(shared) {
        value = json!({"enabled":false,"copy":copy()});
    }
    Response::ok(req.id, value)
}

pub(crate) fn handle_follow(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req.params.get("publisher_id").and_then(Value::as_str) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "insights-skills-publisher-invalid");
    };
    let Some(followed) = req.params.get("followed").and_then(Value::as_bool) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "insights-skills-follow-invalid");
    };
    if !enabled(shared) {
        return Response::ok(req.id, json!({"enabled":false,"copy":copy()}));
    }
    let service = &shared.insights_skills;
    let Ok(mut runtime) = service.state(shared.store.dir()) else {
        return Response::err(req.id, ERR_UNAVAILABLE, ERROR_CACHE);
    };
    if runtime.error == Some(ERROR_CACHE) {
        return Response::err(req.id, ERR_UNAVAILABLE, ERROR_CACHE);
    }
    let mut next = runtime.public.clone();
    if let Err(label) = next.follow(id, followed) {
        return Response::err(req.id, ERR_BAD_PARAMS, label);
    }
    if persist(shared.store.dir(), &next).is_err() {
        return Response::err(
            req.id,
            ERR_UNAVAILABLE,
            "insights-skills-follow-write-failed",
        );
    }
    runtime.public = next;
    drop(runtime);
    Response::ok(req.id, super::run_blocking(|| snapshot(shared)))
}

fn copy() -> Value {
    serde_json::to_value(
        crate::insights::service::ui_copy()
            .into_iter()
            .filter(|(key, _)| key.starts_with("skills_"))
            .collect::<BTreeMap<_, _>>(),
    )
    .unwrap_or_else(|_| json!({}))
}

#[cfg(test)]
mod tests;
