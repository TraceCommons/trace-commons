//! Reporting public skills the catalog lacks (spec 2026-11-15, "Reporting
//! missing skills"). The only path by which anything about installed skills
//! leaves the machine, so every condition is explicit:
//!
//! - `insights_skills` and `skills_catalog_suggestions` are on, Local trace
//!   analysis is on, and the user opted in (stored here, default off);
//! - the skill's source came from a git checkout (the only provenance with a
//!   commit, and only when the local bytes equal the tracked blob), its remote
//!   is a github.com `owner/repo`, and the catalog does not list it;
//! - before anything is sent, an anonymous GET of
//!   `raw.githubusercontent.com/<repo>/<commit>/<path>` returns exactly the
//!   installed bytes, proving the repository is public and the commit pushed.
//!   Our server is not contacted for that;
//! - one suggestion per request, no credential, cookie or install id; at most
//!   four requests a day, each after its own random delay, and each skill at
//!   most once per 30 days.
//!
//! A skill that fails the public check is remembered by a salted local hash,
//! never by name, and never sent.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use trace_commons_protocol::skills_catalog::{
    CatalogDocument, SUGGESTIONS_PATH, SkillSuggestion, normalize_repository, safe_skill_path,
};

use super::super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::{Inventory, PublicState};

pub(crate) const SUGGESTIONS_FLAG: &str = "skills_catalog_suggestions";
const STATE_FILE: &str = "insights-skills-suggestions-v1.json";
const MAX_STATE_BYTES: usize = 512 * 1024;
const MAX_REMEMBERED: usize = 4096;
pub(crate) const MAX_PER_DAY: u32 = 4;
const REMEMBER_DAYS: i64 = 30;
const MIN_DELAY: Duration = Duration::from_secs(60);
const MAX_DELAY: Duration = Duration::from_secs(30 * 60);
const MAX_SKILL_BYTES: usize = 256 * 1024;
const RAW_BASE: &str = "https://raw.githubusercontent.com";

/// In-memory scheduling. A leaf lock.
#[derive(Default)]
pub(crate) struct Runtime {
    next_attempt: Option<Instant>,
    in_flight: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    schema_version: u32,
    opted_in: bool,
    /// Random, local, salts the remembered-skill keys.
    salt: String,
    /// Salted key of a skill to when it was last sent or refused.
    remembered: BTreeMap<String, DateTime<Utc>>,
    day: Option<String>,
    sent_today: u32,
    sent_total: u64,
    last_sent_at: Option<DateTime<Utc>>,
}

fn state_path(dir: &Path) -> PathBuf {
    dir.join(STATE_FILE)
}

fn random_bytes<const N: usize>() -> [u8; N] {
    use ring::rand::SecureRandom as _;
    let mut bytes = [0u8; N];
    let _ = ring::rand::SystemRandom::new().fill(&mut bytes);
    bytes
}

fn load(dir: &Path) -> State {
    let path = state_path(dir);
    let Ok(metadata) = std::fs::symlink_metadata(&path) else {
        return State::default();
    };
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.len() > MAX_STATE_BYTES as u64
    {
        return State::default();
    }
    std::fs::read(&path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<State>(&bytes).ok())
        .filter(|state| state.schema_version == 1)
        .unwrap_or_default()
}

fn persist(dir: &Path, state: &mut State) -> Result<(), ()> {
    state.schema_version = 1;
    if state.salt.len() != 64 {
        state.salt = hex::encode(random_bytes::<32>());
    }
    let cutoff = Utc::now() - chrono::Duration::days(REMEMBER_DAYS);
    state.remembered.retain(|_, at| *at > cutoff);
    while state.remembered.len() > MAX_REMEMBERED {
        let Some(oldest) = state
            .remembered
            .iter()
            .min_by_key(|(_, at)| **at)
            .map(|(key, _)| key.clone())
        else {
            break;
        };
        state.remembered.remove(&oldest);
    }
    let bytes = serde_json::to_vec(state).map_err(|_| ())?;
    crate::config::write_atomic_0600(dir, &state_path(dir), &bytes).map_err(|_| ())
}

fn key(salt: &str, repository: &str, path: &str) -> String {
    hex::encode(Sha256::digest(format!("{salt}\n{repository}\n{path}")))
}

/// Whether the feature may be offered at all: both flags and local analysis.
pub(crate) fn available(shared: &DaemonShared) -> bool {
    super::enabled(shared) && shared.feature_flags.is_enabled(SUGGESTIONS_FLAG)
}

/// One installed skill that may be reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Candidate {
    pub(crate) suggestion: SkillSuggestion,
    /// The installed (and tracked) bytes' hash GitHub must serve.
    pub(crate) sha256: String,
}

/// Installed skills eligible for a report: a git-checkout source with a
/// commit whose tracked blob equals the local bytes, a github.com
/// repository, and no catalog entry at that repository and path.
pub(crate) fn candidates(
    inventory: &Inventory,
    public: &PublicState,
    server: Option<&CatalogDocument>,
) -> Vec<Candidate> {
    let listed = |repository: &str, path: &str| {
        public
            .skills
            .iter()
            .any(|s| s.repository == repository && s.path == path)
            || server.is_some_and(|document| {
                document
                    .skills
                    .iter()
                    .any(|s| s.repository == repository && s.path == path)
            })
    };
    let mut found: Vec<Candidate> = inventory
        .skills
        .iter()
        .filter_map(|skill| {
            let source = skill.source.as_ref()?;
            let revision = source.revision.as_deref()?;
            // Only git provenance carries a commit, and only for bytes that
            // equal the tracked blob: a local edit is never reported.
            if source.baseline_sha256.as_deref() != Some(skill.sha256.as_str()) {
                return None;
            }
            let suggestion = SkillSuggestion {
                repository: normalize_repository(&source.repository)?,
                path: source.path.clone(),
                revision: revision.to_ascii_lowercase(),
            };
            if suggestion.validate().is_err()
                || !safe_skill_path(&suggestion.path)
                || listed(&suggestion.repository, &suggestion.path)
            {
                return None;
            }
            Some(Candidate {
                suggestion,
                sha256: skill.sha256.clone(),
            })
        })
        .collect();
    found.sort_by(|a, b| {
        (&a.suggestion.repository, &a.suggestion.path)
            .cmp(&(&b.suggestion.repository, &b.suggestion.path))
    });
    found.dedup_by(|a, b| {
        a.suggestion.repository == b.suggestion.repository && a.suggestion.path == b.suggestion.path
    });
    found
}

fn today() -> String {
    Utc::now().format("%Y-%m-%d").to_string()
}

/// The next candidate to try under the stored state, if today's budget
/// allows one.
fn next_candidate(state: &State, candidates: &[Candidate]) -> Option<Candidate> {
    let used = if state.day.as_deref() == Some(today().as_str()) {
        state.sent_today
    } else {
        0
    };
    if used >= MAX_PER_DAY {
        return None;
    }
    candidates
        .iter()
        .find(|c| {
            !state.remembered.contains_key(&key(
                &state.salt,
                &c.suggestion.repository,
                &c.suggestion.path,
            ))
        })
        .cloned()
}

/// What the snapshot shows about reporting. Never lists a candidate.
/// `available` is sampled by the caller before it takes the Skills lock,
/// which must never be held while the settings lock is taken.
pub(crate) fn snapshot(
    available: bool,
    dir: &Path,
    inventory: &Inventory,
    public: &PublicState,
    server: Option<&CatalogDocument>,
) -> Value {
    if !available {
        return json!({"available": false, "opted_in": false});
    }
    let state = load(dir);
    let pending = if state.opted_in {
        candidates(inventory, public, server)
            .iter()
            .filter(|c| {
                !state.remembered.contains_key(&key(
                    &state.salt,
                    &c.suggestion.repository,
                    &c.suggestion.path,
                ))
            })
            .count()
    } else {
        0
    };
    json!({
        "available": true,
        "opted_in": state.opted_in,
        "pending": pending,
        "sent_total": state.sent_total,
        "last_sent_at": state.last_sent_at,
    })
}

/// `insights_skills_suggestions {"opted_in": bool}`: the user's choice.
pub(crate) fn handle_set(shared: &DaemonShared, req: &Request) -> Response {
    let Some(opted_in) = req.params.get("opted_in").and_then(Value::as_bool) else {
        return Response::err(
            req.id,
            ERR_BAD_PARAMS,
            "insights-skills-suggestions-invalid",
        );
    };
    if !available(shared) {
        return Response::ok(req.id, json!({"available": false, "opted_in": false}));
    }
    let dir = shared.store.dir();
    let mut state = load(dir);
    state.opted_in = opted_in;
    if persist(dir, &mut state).is_err() {
        return Response::err(
            req.id,
            ERR_UNAVAILABLE,
            "insights-skills-suggestions-write-failed",
        );
    }
    if let Ok(mut runtime) = shared.insights_skills.suggestions.lock() {
        // A fresh choice waits a full random delay before its first report.
        runtime.next_attempt = Some(Instant::now() + random_delay());
    }
    Response::ok(
        req.id,
        json!({"available": true, "opted_in": state.opted_in}),
    )
}

fn random_delay() -> Duration {
    let raw = u64::from_le_bytes(random_bytes::<8>());
    let span = (MAX_DELAY - MIN_DELAY).as_secs().max(1);
    MIN_DELAY + Duration::from_secs(raw % span)
}

/// Record one attempt's outcome: `sent` counts toward the day's budget;
/// either way the skill is remembered for 30 days.
fn record(dir: &Path, candidate: &Candidate, sent: bool) {
    let mut state = load(dir);
    if state.salt.len() != 64 {
        let _ = persist(dir, &mut state);
    }
    let now = Utc::now();
    state.remembered.insert(
        key(
            &state.salt,
            &candidate.suggestion.repository,
            &candidate.suggestion.path,
        ),
        now,
    );
    if sent {
        let day = today();
        if state.day.as_deref() != Some(day.as_str()) {
            state.day = Some(day);
            state.sent_today = 0;
        }
        state.sent_today += 1;
        state.sent_total += 1;
        state.last_sent_at = Some(now);
    }
    let _ = persist(dir, &mut state);
}

pub(crate) fn raw_client() -> reqwest::Result<reqwest::Client> {
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(15))
        .user_agent("TraceCommons-Skills/1")
        .build()
}

/// Whether GitHub serves exactly the installed bytes at that commit to an
/// anonymous request.
pub(crate) async fn is_public(
    client: &reqwest::Client,
    raw_base: &str,
    candidate: &Candidate,
) -> bool {
    let s = &candidate.suggestion;
    let url = format!("{raw_base}/{}/{}/{}", s.repository, s.revision, s.path);
    let Ok(mut response) = client.get(url).send().await else {
        return false;
    };
    if response.status() != reqwest::StatusCode::OK
        || response
            .content_length()
            .is_some_and(|n| n > MAX_SKILL_BYTES as u64)
    {
        return false;
    }
    let mut bytes = Vec::new();
    while let Ok(Some(chunk)) = response.chunk().await {
        if bytes.len() + chunk.len() > MAX_SKILL_BYTES {
            return false;
        }
        bytes.extend_from_slice(&chunk);
    }
    hex::encode(Sha256::digest(&bytes)) == candidate.sha256
}

/// One anonymous suggestion. True only when the server took it.
pub(crate) async fn send(
    client: &reqwest::Client,
    origin: &str,
    suggestion: &SkillSuggestion,
) -> bool {
    client
        .post(format!("{origin}{SUGGESTIONS_PATH}"))
        .json(suggestion)
        .send()
        .await
        .is_ok_and(|response| response.status() == reqwest::StatusCode::ACCEPTED)
}

/// Called on the daemon's periodic tick. Sends at most one suggestion, when
/// every gate holds and its random delay has passed. Uses only the inventory
/// a Skills view already scanned; it never starts a scan of its own.
pub(crate) fn maybe_send(shared: &DaemonShared) {
    if !available(shared) {
        return;
    }
    let dir = shared.store.dir().to_owned();
    let state = load(&dir);
    if !state.opted_in {
        return;
    }
    {
        let Ok(mut runtime) = shared.insights_skills.suggestions.lock() else {
            return;
        };
        let now = Instant::now();
        match runtime.next_attempt {
            _ if runtime.in_flight => return,
            None => {
                runtime.next_attempt = Some(now + random_delay());
                return;
            }
            Some(at) if at > now => return,
            Some(_) => {}
        }
        runtime.next_attempt = Some(now + random_delay());
    }
    let candidate = {
        let Ok(mut runtime) = shared.insights_skills.runtime.lock() else {
            return;
        };
        shared.insights_skills.load_server(shared, &mut runtime);
        let server = super::current_server(shared, &runtime, Utc::now());
        let Some((_, _, inventory)) = runtime.inventory.as_ref() else {
            return;
        };
        let found = candidates(inventory, &runtime.public, server);
        next_candidate(&state, &found)
    };
    let Some(candidate) = candidate else {
        return;
    };
    if cfg!(any(test, feature = "test-credential-store")) {
        return;
    }
    if let Ok(mut runtime) = shared.insights_skills.suggestions.lock() {
        runtime.in_flight = true;
    }
    let spawned = shared.spawn_detached(move |shared| async move {
        let sent = match raw_client() {
            Ok(client) if available(&shared) && is_public(&client, RAW_BASE, &candidate).await => {
                // Re-check the gates: the user may have opted out meanwhile.
                available(&shared)
                    && load(&dir).opted_in
                    && send(
                        &client,
                        crate::channel::NATIVE_ACCOUNT_ORIGIN,
                        &candidate.suggestion,
                    )
                    .await
            }
            _ => false,
        };
        record(&dir, &candidate, sent);
        if let Ok(mut runtime) = shared.insights_skills.suggestions.lock() {
            runtime.in_flight = false;
        }
    });
    if spawned.is_none() {
        if let Ok(mut runtime) = shared.insights_skills.suggestions.lock() {
            runtime.in_flight = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    const REV: &str = "0123456789abcdef0123456789abcdef01234567";

    fn inventory(skills: Value) -> Inventory {
        serde_json::from_value(json!({"skills": skills, "partial": false, "coverage": []})).unwrap()
    }

    fn installed(
        id: &str,
        repository: &str,
        path: &str,
        revision: Option<&str>,
        baseline: Option<&str>,
    ) -> Value {
        json!({"id": id, "name": id, "description": "x", "agent": "codex", "scope": "global",
               "location": "~/.codex/skills/x/SKILL.md", "sha256": "a".repeat(64),
               "source": {"repository": repository, "path": path, "revision": revision,
                          "baseline_sha256": baseline}})
    }

    #[test]
    fn only_unmodified_git_checkouts_missing_from_the_catalog_are_candidates() {
        let a = "a".repeat(64);
        let inventory = inventory(json!([
            installed(
                "git",
                "acme/skills",
                "skills/x/SKILL.md",
                Some(REV),
                Some(&a)
            ),
            installed(
                "dup",
                "acme/skills",
                "skills/x/SKILL.md",
                Some(REV),
                Some(&a)
            ),
            installed("lock", "acme/skills", "skills/y/SKILL.md", None, None),
            installed(
                "edited",
                "acme/skills",
                "skills/z/SKILL.md",
                Some(REV),
                Some(&"b".repeat(64))
            ),
            installed(
                "listed",
                "obra/superpowers",
                "skills/plan/SKILL.md",
                Some(REV),
                Some(&a)
            ),
            installed(
                "short",
                "acme/skills",
                "skills/w/SKILL.md",
                Some("HEAD"),
                Some(&a)
            ),
        ]));
        let mut public = PublicState::default();
        public.skills.push(crate::insights::skills::CatalogSkill {
            id: "obra:skills/plan/SKILL.md".into(),
            publisher_id: "obra".into(),
            name: "plan".into(),
            description: "x".into(),
            repository: "obra/superpowers".into(),
            path: "skills/plan/SKILL.md".into(),
            revision: REV.into(),
            sha256: a.clone(),
            source_url: String::new(),
            instructions_url: String::new(),
        });
        let found = candidates(&inventory, &public, None);
        assert_eq!(found.len(), 1);
        assert_eq!(
            found[0].suggestion,
            SkillSuggestion {
                repository: "acme/skills".into(),
                path: "skills/x/SKILL.md".into(),
                revision: REV.into()
            }
        );
    }

    #[test]
    fn the_daily_budget_and_memory_bound_what_is_tried() {
        let candidate = Candidate {
            suggestion: SkillSuggestion {
                repository: "acme/skills".into(),
                path: "skills/x/SKILL.md".into(),
                revision: REV.into(),
            },
            sha256: "a".repeat(64),
        };
        let mut state = State {
            salt: "s".repeat(64),
            ..State::default()
        };
        assert_eq!(
            next_candidate(&state, std::slice::from_ref(&candidate)),
            Some(candidate.clone())
        );
        state.remembered.insert(
            key(&state.salt, "acme/skills", "skills/x/SKILL.md"),
            Utc::now(),
        );
        assert_eq!(
            next_candidate(&state, std::slice::from_ref(&candidate)),
            None
        );
        state.remembered.clear();
        state.day = Some(today());
        state.sent_today = MAX_PER_DAY;
        assert_eq!(
            next_candidate(&state, std::slice::from_ref(&candidate)),
            None
        );
        state.day = Some("2000-01-01".into());
        assert!(
            next_candidate(&state, &[candidate]).is_some(),
            "a new day resets the budget"
        );
    }

    #[test]
    fn recorded_attempts_keep_no_names() {
        let dir = tempfile::tempdir().unwrap();
        let candidate = Candidate {
            suggestion: SkillSuggestion {
                repository: "secret-org/private-repo".into(),
                path: "skills/x/SKILL.md".into(),
                revision: REV.into(),
            },
            sha256: "a".repeat(64),
        };
        record(dir.path(), &candidate, false);
        let text = std::fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
        assert!(!text.contains("secret-org") && !text.contains("private-repo"));
        let state = load(dir.path());
        assert_eq!((state.sent_total, state.remembered.len()), (0, 1));
        record(dir.path(), &candidate, true);
        assert_eq!(load(dir.path()).sent_today, 1);
    }

    /// A one-shot local server: reads one whole request (headers and any
    /// body), answers with `response`, and hands back the request.
    async fn serve(response: Vec<u8>) -> (String, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = socket.read(&mut buffer).await.unwrap();
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request).to_lowercase();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end]
                        .lines()
                        .find_map(|l| l.strip_prefix("content-length:"))
                        .and_then(|v| v.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if request.len() >= end + 4 + length {
                        break;
                    }
                }
                if n == 0 {
                    break;
                }
            }
            let _ = socket.write_all(&response).await;
            String::from_utf8_lossy(&request).into_owned()
        });
        (base, task)
    }

    #[tokio::test]
    async fn the_public_check_and_the_report_are_anonymous() {
        let body = b"---\nname: x\ndescription: y\n---\n";
        let candidate = Candidate {
            suggestion: SkillSuggestion {
                repository: "acme/skills".into(),
                path: "skills/x/SKILL.md".into(),
                revision: REV.into(),
            },
            sha256: hex::encode(Sha256::digest(body)),
        };
        let mut response = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        let (base, task) = serve(response).await;
        assert!(is_public(&raw_client().unwrap(), &base, &candidate).await);
        let request = task.await.unwrap();
        assert!(request.starts_with(&format!("GET /acme/skills/{REV}/skills/x/SKILL.md ")));
        // Different bytes (a private repository's 404 page, an edit) fail.
        let (base, task) = serve(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        )
        .await;
        assert!(!is_public(&raw_client().unwrap(), &base, &candidate).await);
        task.await.unwrap();

        let (origin, task) = serve(
            b"HTTP/1.1 202 Accepted\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        )
        .await;
        assert!(send(&raw_client().unwrap(), &origin, &candidate.suggestion).await);
        let request = task.await.unwrap();
        let lower = request.to_lowercase();
        assert!(request.starts_with(&format!("POST {SUGGESTIONS_PATH} ")));
        for header in ["authorization:", "cookie:", "x-install"] {
            assert!(!lower.contains(header), "{request}");
        }
        let body = &request[request.find("\r\n\r\n").unwrap() + 4..];
        let sent: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            sent,
            json!({"repository": "acme/skills", "path": "skills/x/SKILL.md", "revision": REV})
        );
    }
}
