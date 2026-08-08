//! The poll loop: stat the session roots, decide what is finished, and route
//! it by project policy.
//!
//! Polling rather than filesystem notification is deliberate. The quiescence
//! window is half an hour, so a sixty-second poll costs nothing in
//! responsiveness, and it avoids a watch dependency plus the per-platform
//! behaviour differences that come with one.
//!
//! Resolving a session's working directory means reading into the file, so
//! results are cached against the file's size and mtime. Without that cache a
//! laptop would re-read every session file every minute.
//!
//! The trajectory source is not watched: trajectory files have no
//! conventional local store to poll, so they stay a deliberate `submit
//! --trajectory` action.

use std::sync::atomic::Ordering;

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::eligibility::{Eligibility, Observation, evaluate};
use super::ipc::{DaemonShared, EVENT_QUEUE_CHANGED};
use super::policy::{ProjectMode, project_key_for, project_label_for};
use super::queue::{QueueEntry, QueueState, entry_id_for};
use super::state::CwdCacheEntry;
use crate::source::{SessionRef, TraceSource, all_sources};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct TickReport {
    pub observed: usize,
    pub queued: usize,
    /// Entries handed straight to the uploader because their project is
    /// opted in.
    pub auto_ready: usize,
    pub ignored: usize,
}

/// One pass over the session roots.
///
/// Returns what it saw rather than acting on uploads itself: the caller owns
/// the submit pipeline, which needs an async context and mutable state this
/// function deliberately does not hold.
pub async fn tick(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport> {
    if shared.paused.load(Ordering::Relaxed) {
        return Ok(TickReport::default());
    }

    let mut report = TickReport::default();
    let mut changed = false;

    let (max_queue_entries, claude_root, codex_root) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.max_queue_entries,
            s.claude_root.clone(),
            s.codex_root.clone(),
        )
    };

    for source in all_sources(claude_root, codex_root, None) {
        let refs = match source.discover() {
            Ok(refs) => refs,
            Err(_) => continue,
        };
        for session_ref in refs {
            report.observed += 1;
            let Ok(meta) = std::fs::metadata(&session_ref.path) else {
                continue;
            };
            let Ok(modified) = meta.modified() else {
                continue;
            };
            let obs = Observation {
                path: session_ref.path.clone(),
                size_bytes: meta.len(),
                modified_at: DateTime::<Utc>::from(modified),
            };

            let (previous_size, prior) = {
                let state = shared.state.lock().expect("state lock");
                (
                    state.previous_size(&obs.path),
                    state.prior_upload(&obs.path).cloned(),
                )
            };

            let verdict = {
                let settings = shared.settings.lock().expect("settings lock");
                evaluate(&obs, previous_size, prior.as_ref(), now, &settings)
            };

            // Record the observation regardless, so the next poll can judge
            // size stability.
            {
                let mut state = shared.state.lock().expect("state lock");
                state.observe(&obs.path, obs.size_bytes);
            }

            if verdict != Eligibility::Eligible {
                continue;
            }

            let cwd = resolve_cwd(shared, source.as_ref(), &session_ref, &obs);
            let project_key = project_key_for(cwd.as_deref());
            let mode = {
                let policy = shared.policy.lock().expect("policy lock");
                policy.resolve(&project_key)
            };
            if mode == ProjectMode::Ignore {
                report.ignored += 1;
                continue;
            }

            // Hashing reads the whole file, so it happens here -- once a
            // session is actually eligible -- and never on a routine poll.
            let Ok(transcript) = source.load(&session_ref) else {
                continue;
            };

            let entry = QueueEntry {
                entry_id: entry_id_for(&transcript.session_hash),
                session_hash: transcript.session_hash.clone(),
                source: session_ref.source.to_string(),
                project_key: project_key.clone(),
                project_label: project_label_for(&project_key),
                path: obs.path.clone(),
                size_bytes: obs.size_bytes,
                discovered_at: now,
                state: if mode == ProjectMode::AutoUpload {
                    // Opted in, so it needs no decision; the uploader picks it
                    // up on its next pass.
                    QueueState::Approved
                } else {
                    QueueState::Pending
                },
                reason_label: None,
                attempts: 0,
                retry_after: None,
                submission_id: None,
            };

            let mut queue = shared.queue.lock().expect("queue lock");
            let already = queue.get(entry.entry_id).is_some();
            match queue.upsert(entry, max_queue_entries) {
                Ok(()) if !already => {
                    changed = true;
                    if mode == ProjectMode::AutoUpload {
                        report.auto_ready += 1;
                    } else {
                        report.queued += 1;
                    }
                }
                Ok(()) => {}
                Err(_) => {
                    let mut health = shared.health.lock().expect("health lock");
                    health.fail(super::health::LABEL_QUEUE_FULL, now);
                }
            }
        }
    }

    if changed {
        {
            let queue = shared.queue.lock().expect("queue lock");
            queue.save(&shared.store)?;
        }
        shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
    }
    {
        let state = shared.state.lock().expect("state lock");
        state.save(&shared.store)?;
    }
    Ok(report)
}

/// The session's working directory, from cache when the file has not changed.
fn resolve_cwd(
    shared: &DaemonShared,
    source: &dyn TraceSource,
    session_ref: &SessionRef,
    obs: &Observation,
) -> Option<String> {
    let key = obs.path.to_string_lossy().to_string();
    {
        let state = shared.state.lock().expect("state lock");
        if let Some(hit) = state.cwd_cache.get(&key) {
            if hit.size_bytes == obs.size_bytes && hit.modified_at == obs.modified_at {
                return hit.cwd.clone();
            }
        }
    }
    // Discovery may already know it; otherwise this reads the file.
    let cwd = session_ref
        .cwd
        .clone()
        .or_else(|| source.load(session_ref).ok().and_then(|t| t.cwd));
    let mut state = shared.state.lock().expect("state lock");
    state.cwd_cache.insert(
        key,
        CwdCacheEntry {
            size_bytes: obs.size_bytes,
            modified_at: obs.modified_at,
            cwd: cwd.clone(),
        },
    );
    cwd
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::ConfigStore;
    use crate::daemon::policy::ProjectMode;
    use std::path::PathBuf;

    fn at(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    /// A daemon whose session roots are a tempdir, so a test never reads the
    /// developer's real transcripts.
    struct WatcherFixture {
        _dir: tempfile::TempDir,
        shared: DaemonShared,
        claude_root: PathBuf,
    }

    impl WatcherFixture {
        fn new() -> Self {
            let dir = tempfile::tempdir().unwrap();
            let store = ConfigStore::open(dir.path().join("state")).unwrap();
            let claude_root = dir.path().join("projects");
            let codex_root = dir.path().join("codex-sessions");
            std::fs::create_dir_all(&claude_root).unwrap();
            std::fs::create_dir_all(&codex_root).unwrap();
            let shared = DaemonShared::load(store).unwrap();
            {
                let mut s = shared.settings.lock().unwrap();
                s.claude_root = Some(claude_root.clone());
                s.codex_root = Some(codex_root);
            }
            Self {
                _dir: dir,
                shared,
                claude_root,
            }
        }

        /// Write a session and backdate it so it reads as quiescent.
        fn write_session(&self, project: &str, name: &str, extra_events: usize) -> PathBuf {
            let project_dir = self
                .claude_root
                .join(format!("-Users-testuser-code-{project}"));
            std::fs::create_dir_all(&project_dir).unwrap();
            let path = project_dir.join(format!("{name}.jsonl"));
            let mut body = format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\
                 \"cwd\":\"/Users/testuser/code/{project}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{name}\",\"uuid\":\"a1\"}}\n"
            );
            for i in 0..extra_events {
                body.push_str(&format!(
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"more {i}\"}},\
                     \"cwd\":\"/Users/testuser/code/{project}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{name}\",\"uuid\":\"b{i}\"}}\n"
                ));
            }
            std::fs::write(&path, body).unwrap();
            path
        }

        fn set_mode(&self, project: &str, mode: ProjectMode) {
            self.shared
                .policy
                .lock()
                .unwrap()
                .set_mode(
                    &format!("/Users/testuser/code/{project}"),
                    project,
                    mode,
                    at("2026-08-08T12:00:00Z"),
                )
                .unwrap();
        }

        fn queue_len(&self) -> usize {
            self.shared.queue.lock().unwrap().all().len()
        }

        fn states(&self) -> Vec<QueueState> {
            self.shared
                .queue
                .lock()
                .unwrap()
                .all()
                .iter()
                .map(|e| e.state)
                .collect()
        }

        /// Two ticks: the first records a size, the second can confirm it is
        /// stable. Eligibility deliberately never fires on a first sighting.
        async fn settle(&self, now: DateTime<Utc>) -> TickReport {
            tick(&self.shared, now).await.unwrap();
            tick(&self.shared, now).await.unwrap()
        }
    }

    #[tokio::test]
    async fn a_quiesced_session_is_queued_for_a_notify_only_project() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(f.queue_len(), 1);
        assert_eq!(f.states(), vec![QueueState::Pending]);
    }

    #[tokio::test]
    async fn a_session_still_being_written_is_not_queued() {
        // Only one tick, so size stability was never confirmed.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = tick(&f.shared, at("2030-01-01T00:00:00Z")).await.unwrap();
        assert_eq!(report.queued, 0);
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn a_recently_written_session_is_not_queued() {
        // The fixture file's mtime is genuinely now, so judging it against
        // the present clock is exactly the live-session case.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let report = f.settle(Utc::now()).await;
        assert_eq!(report.queued, 0, "a live session must not be offered");
    }

    #[tokio::test]
    async fn an_ignored_project_is_never_queued() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::Ignore);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 0);
        assert_eq!(report.ignored, 1);
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn an_opted_in_project_is_queued_already_approved() {
        // Opting the project in is the decision; the entry needs no second one.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(report.queued, 0);
        assert_eq!(f.states(), vec![QueueState::Approved]);
    }

    #[tokio::test]
    async fn repeated_ticks_do_not_duplicate_an_entry() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        tick(&f.shared, at("2030-01-01T00:01:00Z")).await.unwrap();
        tick(&f.shared, at("2030-01-01T00:02:00Z")).await.unwrap();
        assert_eq!(f.queue_len(), 1);
    }

    #[tokio::test]
    async fn a_paused_daemon_does_no_work_at_all() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.shared.paused.store(true, Ordering::Relaxed);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report, TickReport::default());
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn sessions_from_several_projects_are_all_offered() {
        let f = WatcherFixture::new();
        f.write_session("alpha", "11111111-1111-1111-1111-111111111111", 0);
        f.write_session("beta", "22222222-2222-2222-2222-222222222222", 0);
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 2, "{report:?}");
    }

    #[tokio::test]
    async fn the_queue_and_state_are_persisted_after_a_tick() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert!(
            f.shared
                .store
                .read_daemon_file(crate::config::DAEMON_STATE_FILE)
                .unwrap()
                .is_some()
        );
        let reloaded = crate::daemon::queue::Queue::load(&f.shared.store).unwrap();
        assert_eq!(reloaded.all().len(), 1);
    }

    #[tokio::test]
    async fn a_queued_entry_records_a_label_and_a_hash() {
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        assert_eq!(e.project_label, "proj");
        assert!(e.session_hash.starts_with("sha256:"));
        assert_eq!(e.source, "claude-code");
        assert!(e.size_bytes > 0);
    }
}
