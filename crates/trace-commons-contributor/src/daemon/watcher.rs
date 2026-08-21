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
//! The same reasoning governs the expensive step, `TraceSource::load`, which
//! reads, parses and hashes a whole session group. A session already sitting
//! in the queue is `Eligible` on every poll -- eligibility is decided from
//! prior *uploads*, and a `Pending` entry has never uploaded -- so the pass
//! used to load all of them, every minute, and throw the result away when
//! `replace_live_at_path` found the entry already tracked. It asks
//! `Queue::unchanged_offer_at_path` first now, comparing the whole
//! observation (group size and group mtime) against what the queue entry was
//! built from, and only loads when something moved.
//!
//! Two smaller per-poll costs were measured on the same corpus (81
//! claude-code groups over 1,044 files, 3,069 codex sessions, 11.7 GB) with
//! a release build, and only one of them was worth changing:
//!
//! - `source.discover()` walks both trees every tick. That is 4.9 ms for
//!   claude-code and 9.0 ms for codex once the head-peek memos are warm --
//!   14 ms of a sixty-second tick, 0.02% duty -- and 154 ms on the first
//!   pass of a process, when those memos are cold. It stats rather than
//!   reads, and the memos (capped at 8192 entries each) have room for
//!   several times this corpus. Left as a full walk: incremental discovery
//!   would have to keep its own view of which directories moved, and the
//!   failure mode of getting that wrong -- a session never noticed, or a
//!   subagent that lands under an already-seen session directory never
//!   noticed -- costs far more than the 14 ms it would save.
//! - `state.save()` at the end of the tick re-serialized and rewrote the
//!   whole `DaemonState` unconditionally: 1.24 MB, ~0.85 ms to serialize
//!   and ~6-10 ms to write and `fsync`, every sixty seconds, around 1.8 GB
//!   of writes a day, for bytes identical to the ones already on disk. That
//!   one is now elided when nothing moved; see `DaemonState::save`.
//!
//! The trajectory source is not watched: trajectory files have no
//! conventional local store to poll, so they stay a deliberate `submit
//! --trajectory` action.

use anyhow::Result;
use chrono::{DateTime, Utc};

use super::eligibility::{Eligibility, Observation, evaluate};
use super::health;
use super::ipc::{DaemonShared, EVENT_QUEUE_CHANGED};
use super::policy::{ProjectMode, disambiguated_label, known_keys, project_key_for};
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
///
/// The pass itself (`tick_blocking`) is synchronous filesystem scanning and
/// hashing with no `.await` point of its own -- `source.discover()`,
/// `std::fs::metadata`, `source.load()` are all blocking calls. On a
/// multi-thread runtime with few worker threads (in the limit, one -- the
/// C ABI's `tc_daemon_start` builds a runtime sized by
/// `TOKIO_WORKER_THREADS`, which a 1-vCPU host or an explicit override can
/// set to 1), running that whole pass inline previously monopolized the
/// sole worker for the scan's entire duration: the socket server, every
/// `tc_subscribe` delivery, and even a reentrant `tc_daemon_stop`'s own
/// wait on the supervisor's `JoinHandle` all starved behind it. `tick`
/// moves the pass off whichever worker is currently running it via
/// `super::run_blocking` (see its doc) -- callers under a `current_thread`
/// runtime (the default `#[tokio::test]` flavor, which every test in this
/// module's suite uses) run the pass inline instead, since
/// `block_in_place` panics there.
pub async fn tick(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport> {
    // `is_paused` also auto-clears a timed pause that has lapsed, so an
    // elapsed `pause {until}` resumes ticking on its own rather than needing
    // an explicit `resume` from whichever app set the timer -- cheap enough
    // to check before deciding whether `block_in_place` is even worth it.
    if shared.is_paused(now) {
        return Ok(TickReport::default());
    }
    super::run_blocking(|| tick_blocking(shared, now))
}

/// The actual pass; see `tick`'s doc for why it is a plain synchronous
/// function rather than `async fn` (it never awaited anything -- every step
/// is a blocking filesystem or lock operation) and why `tick` runs it
/// through `run_blocking`.
fn tick_blocking(shared: &DaemonShared, now: DateTime<Utc>) -> Result<TickReport> {
    let (max_queue_entries, claude_source, codex_source) = {
        let s = shared.settings.lock().expect("settings lock");
        (
            s.max_queue_entries,
            s.claude_source.clone(),
            s.codex_source.clone(),
        )
    };
    tick_over(
        shared,
        now,
        all_sources(claude_source, codex_source, None),
        max_queue_entries,
    )
}

/// The pass itself, over an explicit source list.
///
/// Split out from `tick_blocking` only so tests can hand it a source that
/// counts its own `load` calls: "this poll did not re-read anything" is a
/// claim about how often `TraceSource::load` runs, and nothing observable
/// from the queue alone can prove it.
fn tick_over(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    sources: Vec<Box<dyn TraceSource>>,
    max_queue_entries: usize,
) -> Result<TickReport> {
    let mut report = TickReport::default();
    let mut changed = false;

    // The terms an auto-approval would be given under: the consent scopes,
    // and a fingerprint of everything else outside the session file that
    // determines the envelope. See `QueueEntry::approved_scopes` /
    // `approved_inputs`.
    //
    // Read once per pass rather than once per eligible candidate. It was
    // per-candidate, which meant re-reading and re-parsing the contributor
    // config file once for every session in the corpus on every poll -- and
    // a tick is a snapshot of the settings already (`max_queue_entries` and
    // the source declarations are taken once, above), so there is nothing
    // for a mid-pass re-read to be more correct about.
    let cfg = shared.store.load_config().ok().flatten();
    let consent_scopes = cfg
        .as_ref()
        .map(|c| c.consent_scopes.clone())
        .unwrap_or_default();
    let approval_inputs = cfg.as_ref().map(|c| {
        let near_ai = shared
            .settings
            .lock()
            .expect("settings lock")
            .near_ai
            .clone();
        crate::daemon::preview::input_fingerprint(c, near_ai.as_ref())
    });

    for source in sources {
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
            // Size and mtime come from the `SessionRef`, not from a re-stat
            // of `path`, because a ref can cover more than one file. A
            // claude-code session's delegated transcripts live beside it
            // under `<uuid>/subagents/`, and `path` deliberately stays the
            // parent file so the queue and the upload state keep one stable
            // address per conversation. Judging quiescence on the parent's
            // own mtime would therefore make a subagent that is still being
            // written completely invisible: the daemon would call the group
            // finished mid-delegation, and would never re-offer a
            // conversation that gained forty transcripts after the parent
            // went quiet. `group_modified_at` is `None` for every
            // single-file source, which is exactly the old behaviour.
            let obs = Observation {
                path: session_ref.path.clone(),
                size_bytes: session_ref.size_bytes,
                modified_at: session_ref
                    .group_modified_at
                    .unwrap_or_else(|| DateTime::<Utc>::from(modified)),
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

            // Eligibility cannot tell "already offered and unchanged" from
            // "never offered": it is decided from the observation, the
            // previous poll's size and any prior *upload*, and a session
            // sitting in the queue `Pending` has never uploaded, so it has
            // no prior and comes back `Eligible` forever. Ask the queue
            // before paying for a load. `unchanged_offer_at_path` compares
            // the whole observation -- group size and group mtime -- and
            // deliberately declines whenever a live offer here was built
            // from a different one, so a grown session (a new delegated
            // transcript included) still reaches `replace_live_at_path` and
            // still supersedes.
            //
            // The same lock answers the other half of "can this load
            // produce anything?": a session the queue holds no offer for at
            // all, because the queue is at `max_queue_entries`. See
            // `Queue::load_can_land` -- and note it answers `true`
            // whenever a live entry sits at this path, so a grown session
            // with a live card still reaches the load and still
            // supersedes, full queue or not.
            let (already_offered, can_land) = {
                let queue = shared.queue.lock().expect("queue lock");
                (
                    queue
                        .unchanged_offer_at_path(&obs.path, obs.size_bytes, obs.modified_at)
                        .map(|e| (e.entry_id, e.project_key.clone(), e.state)),
                    queue.load_can_land(&obs.path, max_queue_entries),
                )
            };
            if let Some((entry_id, project_key, state)) = already_offered {
                // The project key is taken from the entry rather than
                // re-derived, which also skips `resolve_cwd`: the entry's
                // key came from this same unchanged content, and resolving
                // it again is another per-poll lock and cache probe over
                // the whole corpus for an answer that cannot have moved.
                let mode = {
                    let policy = shared.policy.lock().expect("policy lock");
                    policy.resolve(&project_key)
                };
                if mode == ProjectMode::Ignore {
                    report.ignored += 1;
                    continue;
                }
                // The one thing the discarded dedup path did do: re-apply a
                // project's standing opt-in to an entry that has since been
                // put back to `Pending` (by a supersede, or by the
                // consent-scope guard). `approve` only moves `Pending`, so
                // it can never resurrect a dismissed, expired or uploaded
                // entry. Preserved here so skipping the load costs nothing
                // but the load.
                if mode == ProjectMode::AutoUpload && state == QueueState::Pending {
                    let mut queue = shared.queue.lock().expect("queue lock");
                    if queue.approve(entry_id, &consent_scopes, approval_inputs.as_deref(), None) {
                        changed = true;
                        report.auto_ready += 1;
                    }
                }
                continue;
            }

            // A full queue with no live entry at this path: whatever the
            // load produced, `replace_live_at_path` would refuse it
            // `queue-full` and the work would be discarded. Refuse it here
            // instead, before the read, the parse and the group hash, and
            // raise the same health label the refusal below raises -- the
            // contributor's queue is genuinely full and sessions are going
            // unoffered, which is the same condition either way.
            if !can_land {
                let mut health = shared.health.lock().expect("health lock");
                health.fail(health::LABEL_QUEUE_FULL, now);
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

            // Hashing reads the whole group, so it happens only here: for a
            // session the queue has no unchanged offer for and could still
            // hold the result of. Everything already offered at this exact
            // observation, and everything a full queue could not have
            // taken, was skipped above.
            let Ok(transcript) = source.load(&session_ref) else {
                continue;
            };

            // Collision detection must see every project the daemon knows
            // about -- both configured policy entries and projects already
            // sitting in the queue -- so a collision is visible as soon as
            // either colliding project has a queue entry. The end-of-tick
            // relabel pass below is what makes this symmetric across the
            // whole colliding set, since this per-entry snapshot alone can
            // still miss a project discovered later in the same pass.
            let known = {
                let policy = shared.policy.lock().expect("policy lock");
                let queue = shared.queue.lock().expect("queue lock");
                known_keys(&policy, queue.all().iter().map(|e| e.project_key.clone()))
            };

            let armed = mode == ProjectMode::AutoUpload;

            let entry = QueueEntry {
                entry_id: entry_id_for(&transcript.session_hash),
                session_hash: transcript.session_hash.clone(),
                source: session_ref.source.to_string(),
                project_key: project_key.clone(),
                project_label: disambiguated_label(&project_key, &known),
                path: obs.path.clone(),
                size_bytes: obs.size_bytes,
                discovered_at: now,
                state: if armed {
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
                approved_scopes: armed.then(|| consent_scopes.clone()),
                // `None` when the config could not be read, which the
                // uploader treats as "unknown, re-ask": fail-closed.
                approved_inputs: armed.then(|| approval_inputs.clone()).flatten(),
                // An armed project's sessions are never previewed, so there
                // is no shown artifact to pin to. The input fingerprint is
                // the guard that applies to them.
                previewed_envelope_digest: None,
                // No post-approval hold on a standing opt-in: it is a
                // decision taken in advance, separately audited, with no
                // click to take back and no client counting down for it.
                // See `Queue::approve`.
                approved_at: None,
                subagent_count: transcript.subagent_count,
                subagents_dropped: transcript.subagents_dropped,
                // The observation this entry is made of, so the next poll
                // can recognize it without reading the group again. See
                // `QueueEntry::observed_modified_at`.
                observed_modified_at: Some(obs.modified_at),
            };
            let entry_id = entry.entry_id;

            let mut queue = shared.queue.lock().expect("queue lock");
            // Add the new offer and retire any earlier one for this same
            // session in a single step. Without the retirement, a
            // conversation that gains a delegated transcript accumulates a
            // card per delegation -- `upsert` dedups on hash, and the hash
            // is precisely what moved. Without the atomicity, a `queue-full`
            // between the two would retire the old offer and never land the
            // replacement, leaving the conversation with no live card at
            // all. See `Queue::replace_live_at_path`.
            match queue.replace_live_at_path(entry, max_queue_entries) {
                Ok(outcome) => {
                    if outcome.superseded > 0 {
                        changed = true;
                    }
                    if outcome.inserted {
                        changed = true;
                        if armed {
                            report.auto_ready += 1;
                        } else {
                            report.queued += 1;
                        }
                        // A new entry passed the capacity check: there is
                        // space in the queue.
                        let mut health = shared.health.lock().expect("health lock");
                        health.resolve(health::LABEL_QUEUE_FULL);
                    } else {
                        // Dedup path: re-observing an already-queued session.
                        // The insert deliberately never rewrites an existing
                        // entry, which used to mean a standing opt-in simply
                        // stopped applying to an entry that had been put
                        // back to `Pending` since it was created -- by
                        // `supersede`, or by the consent-scope guard. The
                        // entry sat `Pending` until it aged out, in a
                        // project the contributor had explicitly armed.
                        // Re-apply the standing decision here, which is the
                        // one place that knows both the entry and the mode
                        // in force.
                        //
                        // `Queue::approve` only moves `Pending`, so this can
                        // never resurrect a dismissed-and-refused, expired,
                        // or already-uploaded entry.
                        // `approval_inputs` is passed through as `Option`,
                        // not flattened to `""`: the insert path above
                        // records `None` for "the config could not be read",
                        // and this path recording `Some("")` for the same
                        // condition made two spellings of "unknown". Both
                        // fail closed, but the uploader should only have one
                        // shape to recognize. `None` for `approved_at`,
                        // matching the fresh-entry path above: a standing
                        // opt-in is not held.
                        if armed
                            && queue.approve(
                                entry_id,
                                &consent_scopes,
                                approval_inputs.as_deref(),
                                None,
                            )
                        {
                            changed = true;
                            report.auto_ready += 1;
                        }
                        // This path returns Ok without checking capacity, so
                        // it does not prove space is available. Do not
                        // retract queue-full.
                    }
                }
                Err(_) => {
                    let mut health = shared.health.lock().expect("health lock");
                    health.fail(health::LABEL_QUEUE_FULL, now);
                }
            }
        }
    }

    // Relabel pass: `Queue::upsert` never rewrites an existing entry, so a
    // project that was unique when its entry was first queued would
    // otherwise keep a bare label forever even after a colliding project
    // later gets its own entry (or is configured in policy). Recomputing
    // every entry's label against the final known-key set for this tick
    // means a collision is always visible on *every* member of the
    // colliding set, not just whichever was processed second.
    if relabel_queue(shared) {
        changed = true;
    }

    if changed {
        {
            let queue = shared.queue.lock().expect("queue lock");
            queue.save(&shared.store)?;
            // An expired or superseded entry keeps no redacted trace
            // content on disk. Best-effort: a file that will not delete
            // must not fail a poll.
            let _ =
                crate::daemon::approved_envelope::sweep(&shared.store, &queue.pinned_entry_ids());
        }
        shared.publish(EVENT_QUEUE_CHANGED, serde_json::json!({}));
    }
    {
        let mut state = shared.state.lock().expect("state lock");
        state.save(&shared.store)?;
    }
    Ok(report)
}

/// End-of-tick pass: recompute every queue entry's `project_label` against
/// the tick's final known-key set and rewrite any that changed.
///
/// `Queue::upsert` deliberately never touches an existing entry, so without
/// this pass an entry queued while its basename was still unique would keep
/// a bare label forever, even after a colliding project shows up in a later
/// tick. The actual relabeling logic lives in `ipc::relabel_queue_entries`,
/// which `set_project_mode` also calls (immediately after a policy edit,
/// rather than waiting for the next poll) -- this wrapper only owns taking
/// the locks tick() needs anyway.
fn relabel_queue(shared: &DaemonShared) -> bool {
    let policy = shared.policy.lock().expect("policy lock");
    let mut queue = shared.queue.lock().expect("queue lock");
    super::ipc::relabel_queue_entries(&policy, &mut queue)
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
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// A real source with a counter around its `load`.
    ///
    /// "The poll did not re-read anything" is a claim about how many times
    /// `TraceSource::load` ran, and nothing observable from the queue can
    /// prove it -- a pass that loads every session and then discards the
    /// result leaves a queue identical to one that loaded nothing. Wrapping
    /// the genuine adapter rather than faking one keeps discovery, grouping
    /// and hashing exactly as they are in production.
    struct CountingSource {
        inner: Box<dyn TraceSource>,
        loads: Arc<AtomicUsize>,
    }

    impl TraceSource for CountingSource {
        fn name(&self) -> &'static str {
            self.inner.name()
        }
        fn discover(&self) -> Result<Vec<SessionRef>> {
            self.inner.discover()
        }
        fn load(&self, r: &SessionRef) -> Result<crate::source::SessionTranscript> {
            self.loads.fetch_add(1, Ordering::SeqCst);
            self.inner.load(r)
        }
    }

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
                s.claude_source = Some(crate::daemon::settings::SourceDeclaration::Watch {
                    path: claude_root.clone(),
                });
                s.codex_source =
                    Some(crate::daemon::settings::SourceDeclaration::Watch { path: codex_root });
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

        /// Like `write_session`, but the session's cwd is an explicit full
        /// path rather than derived from `project`. Used to simulate two
        /// distinct projects that happen to share a basename (e.g. two
        /// checkouts both named `api`), which `write_session` alone cannot
        /// produce since it always uses the same parent directory.
        fn write_session_with_cwd(&self, dir_name: &str, cwd: &str, name: &str) -> PathBuf {
            let project_dir = self.claude_root.join(dir_name);
            std::fs::create_dir_all(&project_dir).unwrap();
            let path = project_dir.join(format!("{name}.jsonl"));
            let body = format!(
                "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"hello\"}},\
                 \"cwd\":\"{cwd}\",\
                 \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                 \"sessionId\":\"{name}\",\"uuid\":\"a1\"}}\n"
            );
            std::fs::write(&path, body).unwrap();
            path
        }

        /// Write a delegated transcript under `<session>/subagents/`,
        /// stamped with the parent's `sessionId` so it verifies as a member.
        fn write_subagent(&self, project: &str, session: &str, agent: &str) -> PathBuf {
            let subagents = self
                .claude_root
                .join(format!("-Users-testuser-code-{project}"))
                .join(session)
                .join("subagents");
            std::fs::create_dir_all(&subagents).unwrap();
            let path = subagents.join(format!("{agent}.jsonl"));
            std::fs::write(
                &path,
                format!(
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"delegated\"}},\
                     \"cwd\":\"/Users/testuser/code/{project}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{session}\",\"uuid\":\"s1\"}}\n"
                ),
            )
            .unwrap();
            path
        }

        /// Shrink the queue cap, so a test can reach "the queue is full"
        /// with two sessions instead of five hundred.
        fn set_max_queue_entries(&self, max: usize) {
            self.shared.settings.lock().unwrap().max_queue_entries = max;
        }

        fn set_mode(&self, project: &str, mode: ProjectMode) {
            self.shared
                .policy
                .lock()
                .unwrap()
                .set_mode(
                    &format!("/Users/testuser/code/{project}"),
                    mode,
                    at("2026-08-08T12:00:00Z"),
                )
                .unwrap();
        }

        /// Like `set_mode`, but for an explicit project key rather than one
        /// derived from `/Users/testuser/code/{project}` -- needed for
        /// projects written via `write_session_with_cwd`.
        fn set_mode_for_key(&self, key: &str, mode: ProjectMode) {
            self.shared
                .policy
                .lock()
                .unwrap()
                .set_mode(key, mode, at("2026-08-08T12:00:00Z"))
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

        /// One pass -- the same `tick_over` `tick` runs -- over sources that
        /// count their `load` calls.
        fn tick_counted(&self, now: DateTime<Utc>, loads: &Arc<AtomicUsize>) -> TickReport {
            let (max_queue_entries, claude_source, codex_source) = {
                let s = self.shared.settings.lock().unwrap();
                (
                    s.max_queue_entries,
                    s.claude_source.clone(),
                    s.codex_source.clone(),
                )
            };
            let sources = all_sources(claude_source, codex_source, None)
                .into_iter()
                .map(|inner| {
                    Box::new(CountingSource {
                        inner,
                        loads: loads.clone(),
                    }) as Box<dyn TraceSource>
                })
                .collect();
            tick_over(&self.shared, now, sources, max_queue_entries).unwrap()
        }

        /// `settle`, counted.
        fn settle_counted(&self, now: DateTime<Utc>, loads: &Arc<AtomicUsize>) -> TickReport {
            self.tick_counted(now, loads);
            self.tick_counted(now, loads)
        }

        /// Append to an existing session file, i.e. the conversation
        /// continued after it was offered.
        fn append_to_session(&self, path: &std::path::Path, project: &str, name: &str) {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
            for i in 0..40 {
                writeln!(
                    f,
                    "{{\"type\":\"user\",\"message\":{{\"role\":\"user\",\"content\":\"later {i}\"}},\
                     \"cwd\":\"/Users/testuser/code/{project}\",\
                     \"timestamp\":\"2026-08-08T10:00:00Z\",\"version\":\"2.0.1\",\
                     \"sessionId\":\"{name}\",\"uuid\":\"c{i}\"}}"
                )
                .unwrap();
            }
        }
    }

    fn loads() -> Arc<AtomicUsize> {
        Arc::new(AtomicUsize::new(0))
    }

    fn count(c: &Arc<AtomicUsize>) -> usize {
        c.load(Ordering::SeqCst)
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
        // A real `pause` call always sets both together; `is_paused` trusts
        // `state.paused` once it has taken the state lock (needed so it can
        // return a non-stale answer to a reader that loses a race against a
        // lapsing timed pause -- see `DaemonShared::is_paused`), so the
        // fixture must keep both in sync too.
        f.shared.paused.store(true, Ordering::Relaxed);
        f.shared.state.lock().unwrap().paused = true;
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report, TickReport::default());
        assert_eq!(f.queue_len(), 0);
    }

    #[tokio::test]
    async fn a_lapsed_timed_pause_resumes_ticking_on_its_own() {
        // An app-side timer dies with the app; the daemon must notice the
        // pause has lapsed itself rather than waiting for an explicit
        // `resume` that might never come.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.shared.paused.store(true, Ordering::Relaxed);
        f.shared.state.lock().unwrap().paused_until = Some(at("2029-12-31T00:00:00Z"));
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert!(
            !f.shared.paused.load(Ordering::Relaxed),
            "the lapsed pause should have cleared itself"
        );
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

    #[tokio::test]
    async fn colliding_project_basenames_both_get_suffixed_in_the_same_tick() {
        // Two different repositories both called `api`; one might be the
        // client's. Both must render suffixed once the collision is known --
        // not just whichever one happened to be processed second -- or an
        // operator who has learned "unsuffixed = normal" will misread the
        // bare one as the safe default roughly half the time.
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            "/Users/testuser/work/api",
            "11111111-1111-1111-1111-111111111111",
        );
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            "/Users/testuser/client/api",
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let by_key: std::collections::BTreeMap<String, String> = queue
            .all()
            .iter()
            .map(|e| (e.project_key.clone(), e.project_label.clone()))
            .collect();
        let labels: Vec<&str> = by_key.values().map(String::as_str).collect();
        assert_ne!(
            labels[0], labels[1],
            "colliding projects must render distinct labels: {labels:?}"
        );
        for label in &labels {
            assert!(
                label.starts_with("api ("),
                "expected every colliding member suffixed, got {label}"
            );
            assert!(
                !label.contains("work") && !label.contains("client") && !label.contains('/'),
                "label must not leak a path segment: {label}"
            );
        }
    }

    #[tokio::test]
    async fn a_bare_label_is_relabelled_once_a_collision_appears_in_a_later_tick() {
        // Project queued first, alone, with a unique basename -- correctly
        // bare. A colliding project only shows up afterwards. Because
        // `Queue::upsert` never rewrites an existing entry, only the
        // end-of-tick relabel pass can fix the first entry's now-stale bare
        // label.
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            "/Users/testuser/work/api",
            "11111111-1111-1111-1111-111111111111",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;
        {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.all().len(), 1);
            assert_eq!(queue.all()[0].project_label, "api");
        }

        // A second, colliding project shows up in a later tick.
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            "/Users/testuser/client/api",
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:10:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let first = queue
            .all()
            .iter()
            .find(|e| e.project_key == "/Users/testuser/work/api")
            .unwrap();
        let second = queue
            .all()
            .iter()
            .find(|e| e.project_key == "/Users/testuser/client/api")
            .unwrap();
        assert!(
            first.project_label.starts_with("api ("),
            "the first-queued entry must be relabelled once it collides, got {}",
            first.project_label
        );
        assert!(second.project_label.starts_with("api ("));
        assert_ne!(first.project_label, second.project_label);
    }

    #[tokio::test]
    async fn a_unique_basename_stays_bare_and_is_untouched_by_the_relabel_pass() {
        let f = WatcherFixture::new();
        f.write_session("solo", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        tick(&f.shared, at("2030-01-01T00:03:00Z")).await.unwrap();
        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 1);
        assert_eq!(queue.all()[0].project_label, "solo");
    }

    #[tokio::test]
    async fn list_projects_and_the_queue_render_the_same_label_when_a_collision_exists() {
        let f = WatcherFixture::new();
        f.write_session_with_cwd(
            "-Users-testuser-work-api",
            "/Users/testuser/work/api",
            "11111111-1111-1111-1111-111111111111",
        );
        f.write_session_with_cwd(
            "-Users-testuser-client-api",
            "/Users/testuser/client/api",
            "22222222-2222-2222-2222-222222222222",
        );
        f.settle(at("2030-01-01T00:00:00Z")).await;

        // Configure both projects in policy too (with distinct modes so the
        // `list_projects` rows can be told apart, since that surface
        // deliberately never echoes the project key).
        f.set_mode_for_key("/Users/testuser/work/api", ProjectMode::NotifyOnly);
        f.set_mode_for_key("/Users/testuser/client/api", ProjectMode::Ignore);

        let resp = crate::daemon::ipc::handle_request(
            &f.shared,
            &crate::daemon::ipc::Request {
                id: 1,
                method: "list_projects".to_string(),
                params: serde_json::json!({}),
            },
        );
        let projects = resp.result.unwrap()["projects"].clone();
        let work_row = projects
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["mode"] == serde_json::json!("notify_only"))
            .expect("work/api row");
        let list_label = work_row["project_label"].as_str().unwrap();

        let queue = f.shared.queue.lock().unwrap();
        let queue_entry = queue
            .all()
            .iter()
            .find(|e| e.project_key == "/Users/testuser/work/api")
            .unwrap();
        assert_eq!(
            list_label, queue_entry.project_label,
            "the same project key must render identically on both surfaces"
        );
        assert!(
            list_label.starts_with("api ("),
            "expected a collision suffix, got {list_label}"
        );
    }

    #[tokio::test]
    async fn a_session_and_its_subagents_are_offered_as_one_card() {
        // The whole point: 911 files describing 69 conversations became 911
        // cards. One conversation is one decision.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.write_subagent("proj", session, "agent-a");
        f.write_subagent("proj", session, "agent-b");
        let report = f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(f.queue_len(), 1);
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        assert_eq!(e.subagent_count, 2, "the card must state its own extent");
        assert_eq!(e.subagents_dropped, 0);
        assert!(e.path.ends_with(format!("{session}.jsonl")), "{:?}", e.path);
    }

    #[tokio::test]
    async fn a_subagent_still_being_written_holds_the_whole_group_back() {
        // The trap this change exists to avoid. `Observation` used to come
        // from a re-stat of the parent file, so a subagent appearing or
        // growing was invisible: the daemon would call a conversation
        // finished while a delegate was mid-write. The parent here is
        // deliberately old; only the member is fresh.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.write_subagent("proj", session, "agent-a");
        let report = f.settle(Utc::now()).await;
        assert_eq!(
            report.queued, 0,
            "a group with a live delegate must not be offered: {report:?}"
        );
    }

    #[tokio::test]
    async fn a_new_subagent_supersedes_the_offer_it_invalidates() {
        // Membership is part of the description a contributor consents to.
        // When it moves, the old offer dies and a fresh one is made -- one
        // card, not one card per delegation.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.all().len(), 1);
            queue.all()[0].session_hash.clone()
        };

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(
            old.reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_CHANGED)
        );
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        let fresh = queue.pending()[0];
        assert_ne!(fresh.session_hash, first_hash, "the hash must have moved");
        assert_eq!(fresh.subagent_count, 1);
    }

    #[tokio::test]
    async fn a_new_subagent_releases_the_preview_the_old_offer_was_pinned_to() {
        // The end of the same story: the artifact a contributor was shown is
        // stored on disk and pinned to the entry that was offered. Once that
        // offer is superseded the stored bytes describe a conversation
        // nobody will now be asked about, so the sweep must delete them --
        // an entry the contributor never resolved must not leave redacted
        // trace content lying around.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;

        let entry_id = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].entry_id
        };
        // Exactly what `preview` does: build the envelope, store it, pin the
        // entry to it.
        let src = crate::source::claude_code::ClaudeCodeSource::new(f.claude_root.clone());
        let session_ref = src.discover().unwrap().remove(0);
        let (summary, _body, envelope) =
            crate::daemon::preview::build_preview(&f.shared.store, None, None, &src, &session_ref)
                .await
                .unwrap();
        crate::daemon::approved_envelope::save(&f.shared.store, entry_id, &envelope).unwrap();
        {
            let mut queue = f.shared.queue.lock().unwrap();
            assert!(queue.record_previewed_envelope(entry_id, &summary.envelope_digest));
        }
        assert!(
            crate::daemon::approved_envelope::load(&f.shared.store, entry_id)
                .unwrap()
                .is_some(),
            "the shown artifact should be on disk before the change"
        );

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        {
            let queue = f.shared.queue.lock().unwrap();
            assert_eq!(queue.get(entry_id).unwrap().state, QueueState::Superseded);
        }
        assert!(
            crate::daemon::approved_envelope::load(&f.shared.store, entry_id)
                .unwrap()
                .is_none(),
            "the superseded offer's stored preview must be swept"
        );
    }

    #[tokio::test]
    async fn an_approved_group_is_superseded_when_a_subagent_lands() {
        // The same rule, one state later: an approval covers a description,
        // and a delegate arriving after it is a different description. The
        // approval must not carry over.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        f.write_session("proj", session, 0);
        f.set_mode("proj", ProjectMode::AutoUpload);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Approved]);

        f.write_subagent("proj", session, "agent-a");
        f.settle(at("2030-01-02T00:00:00Z")).await;

        let queue = f.shared.queue.lock().unwrap();
        let superseded = queue
            .all()
            .iter()
            .filter(|e| e.state == QueueState::Superseded)
            .count();
        assert_eq!(superseded, 1, "{:?}", queue.all());
    }

    #[tokio::test]
    async fn an_unqueued_eligible_session_is_loaded_exactly_once() {
        // The baseline the skip is measured against: settling a fresh
        // session costs one load -- nothing on the first sighting (still
        // `Unstable`), one on the poll that actually offers it. Two would
        // mean `resolve_cwd` is reading the file as well as `load`.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        let c = loads();
        let report = f.settle_counted(at("2030-01-01T00:00:00Z"), &c);
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(
            count(&c),
            1,
            "settling one session must cost exactly one load"
        );
    }

    #[tokio::test]
    async fn a_queued_session_that_has_not_moved_is_never_loaded_again() {
        // The bug. `eligibility::evaluate` only knows about prior *uploads*,
        // so a session sitting `Pending` came back `Eligible` on every poll
        // and the pass read, parsed and hashed the whole group again --
        // 11 GB of transcripts a minute on the machine that reported this --
        // only for `replace_live_at_path` to find it already tracked.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        let c = loads();
        for minute in 1..=5 {
            f.tick_counted(at(&format!("2030-01-01T00:0{minute}:00Z")), &c);
        }
        assert_eq!(
            count(&c),
            0,
            "five polls over an unchanged queued session must load nothing"
        );
        assert_eq!(f.queue_len(), 1, "and must not disturb the queue");
        assert_eq!(f.states(), vec![QueueState::Pending]);
    }

    #[tokio::test]
    async fn a_corpus_of_queued_sessions_costs_nothing_on_a_later_poll() {
        // The property that actually matters at scale: the reported machine
        // had 498 entries in the queue and re-hashed the lot every sixty
        // seconds. N queued sessions, zero loads on the next poll.
        let f = WatcherFixture::new();
        for i in 0..30u32 {
            f.write_session(
                "proj",
                &format!("1111111{i:02}-1111-1111-1111-111111111111"),
                0,
            );
        }
        let first = loads();
        let report = f.settle_counted(at("2030-01-01T00:00:00Z"), &first);
        assert_eq!(report.queued, 30, "{report:?}");
        assert_eq!(
            count(&first),
            30,
            "each session is loaded once, when it is offered"
        );

        let later = loads();
        f.tick_counted(at("2030-01-01T00:01:00Z"), &later);
        assert_eq!(
            count(&later),
            0,
            "a poll over 30 unchanged queued sessions must load nothing"
        );
        assert_eq!(f.queue_len(), 30);
    }

    #[tokio::test]
    async fn a_queued_session_that_grew_is_loaded_again_and_supersedes() {
        // The skip must not swallow the supersede path: the offer describes
        // content, the content moved, so the old offer dies and a fresh one
        // is made.
        let f = WatcherFixture::new();
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };

        f.append_to_session(&path, "proj", name);
        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(count(&c), 1, "a grown session must be re-read exactly once");

        let queue = f.shared.queue.lock().unwrap();
        assert_eq!(queue.all().len(), 2, "{:?}", queue.all());
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(
            old.reason_label.as_deref(),
            Some(crate::daemon::queue::REASON_CHANGED)
        );
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        assert_ne!(queue.pending()[0].session_hash, first_hash);
    }

    #[tokio::test]
    async fn a_new_subagent_reloads_even_though_the_parent_files_own_stat_is_unchanged() {
        // The trap a naive pre-check falls into. A delegated transcript
        // lands beside the session under `<uuid>/subagents/`; the parent
        // file itself is untouched, so a check against the parent's own
        // stat would call this "unchanged" and never re-offer the
        // conversation. The observation compared is the group's, which is
        // why `SessionRef::size_bytes` and `group_modified_at` exist.
        let f = WatcherFixture::new();
        let session = "11111111-1111-1111-1111-111111111111";
        let parent = f.write_session("proj", session, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };
        let before = std::fs::metadata(&parent).unwrap();

        f.write_subagent("proj", session, "agent-a");
        let after = std::fs::metadata(&parent).unwrap();
        assert_eq!(
            before.len(),
            after.len(),
            "the parent file must be untouched"
        );
        assert_eq!(
            before.modified().unwrap(),
            after.modified().unwrap(),
            "the parent file's own mtime must be untouched -- that is the whole point"
        );

        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(count(&c), 1, "the group grew, so it must be re-read");

        let queue = f.shared.queue.lock().unwrap();
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .unwrap();
        assert_eq!(old.state, QueueState::Superseded);
        assert_eq!(queue.pending().len(), 1);
        assert_eq!(queue.pending()[0].subagent_count, 1);
    }

    #[tokio::test]
    async fn a_standing_opt_in_still_reaches_an_entry_the_skip_path_finds() {
        // The one thing the discarded per-poll work did do: re-apply a
        // project's standing `auto_upload` to an entry that has since been
        // put back to `Pending`. Skipping the load must not skip that, or
        // arming a project would stop applying to entries already offered.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.states(), vec![QueueState::Pending]);

        f.set_mode("proj", ProjectMode::AutoUpload);
        let c = loads();
        let report = f.tick_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(count(&c), 0, "re-approving must not cost a re-read");
        assert_eq!(report.auto_ready, 1, "{report:?}");
        assert_eq!(f.states(), vec![QueueState::Approved]);
    }

    #[tokio::test]
    async fn an_ignored_project_with_a_queued_entry_is_still_reported_ignored() {
        // The skip path reports the same way the load path did, from the
        // entry's own project key rather than a re-resolved cwd.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        f.set_mode("proj", ProjectMode::Ignore);
        let c = loads();
        let report = f.tick_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(report.ignored, 1, "{report:?}");
        assert_eq!(count(&c), 0);
    }

    #[tokio::test]
    async fn a_queue_entry_records_the_observation_it_was_built_from() {
        // The comparison data lives on the entry, so it cannot drift from
        // the queue it describes.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let queue = f.shared.queue.lock().unwrap();
        let e = &queue.all()[0];
        let observed = e
            .observed_modified_at
            .expect("the entry must record its mtime");
        assert!(
            queue
                .unchanged_offer_at_path(&e.path, e.size_bytes, observed)
                .is_some(),
            "the recorded observation must be the one the next poll compares against"
        );
    }

    /// Overwrite the daemon state file with bytes a save would never
    /// produce, so a later write is detectable by the sentinel being gone.
    /// The in-memory state stays authoritative -- the file is only read at
    /// startup -- so this observes writes without disturbing the tick.
    fn plant_state_sentinel(f: &WatcherFixture) {
        std::fs::write(
            f.shared.store.daemon_path(crate::config::DAEMON_STATE_FILE),
            b"SENTINEL",
        )
        .unwrap();
    }

    fn state_sentinel_survived(f: &WatcherFixture) -> bool {
        std::fs::read(f.shared.store.daemon_path(crate::config::DAEMON_STATE_FILE)).unwrap()
            == b"SENTINEL"
    }

    #[tokio::test]
    async fn a_tick_that_moved_nothing_does_not_rewrite_the_state_file() {
        // The second remaining per-poll cost. Every tick re-serialized the
        // whole `DaemonState` and wrote it with an fsync -- 1.24 MB a minute
        // on the reported machine -- even when the pass changed nothing.
        // `observe` runs for every path on every poll, which is why this is
        // asserted against writes rather than against a dirty flag: the
        // bookkeeping is touched either way, and what must not happen is
        // the write.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        plant_state_sentinel(&f);
        for minute in 1..=3 {
            tick(&f.shared, at(&format!("2030-01-01T00:0{minute}:00Z")))
                .await
                .unwrap();
        }
        assert!(
            state_sentinel_survived(&f),
            "three idle polls must not rewrite the state file"
        );
    }

    #[tokio::test]
    async fn a_tick_that_saw_something_new_does_rewrite_the_state_file() {
        // The other half: eliding the write must not lose an observation.
        // A session first sighted on this tick has to be on disk, or a
        // restart would treat it as never seen and the size-stability check
        // would start over.
        let f = WatcherFixture::new();
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        plant_state_sentinel(&f);

        let second = "22222222-2222-2222-2222-222222222222";
        let path = f.write_session("proj", second, 0);
        tick(&f.shared, at("2030-01-01T00:01:00Z")).await.unwrap();
        assert!(
            !state_sentinel_survived(&f),
            "a first sighting must be persisted"
        );
        let reloaded = crate::daemon::state::DaemonState::load(&f.shared.store).unwrap();
        assert!(
            reloaded.previous_size(&path).is_some(),
            "the reloaded state must carry the new session's observation"
        );
    }

    #[tokio::test]
    async fn a_restart_after_idle_polls_still_knows_what_was_offered() {
        // The risk the elision has to be measured against: state that was
        // never written is state a restart cannot see. After idle polls
        // skipped their writes, a daemon reloading from disk must still
        // find the observations and must not re-offer the session it
        // already has a card for.
        let f = WatcherFixture::new();
        let path = f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        for minute in 1..=3 {
            tick(&f.shared, at(&format!("2030-01-01T00:0{minute}:00Z")))
                .await
                .unwrap();
        }

        let reloaded = crate::daemon::state::DaemonState::load(&f.shared.store).unwrap();
        let live = f.shared.state.lock().unwrap();
        assert_eq!(
            reloaded.previous_size(&path),
            live.previous_size(&path),
            "the file must agree with memory about the last observation"
        );
        assert_eq!(
            reloaded.last_observation, live.last_observation,
            "no observation may be lost to a skipped write"
        );
        assert_eq!(reloaded.cwd_cache, live.cwd_cache);
        assert_eq!(reloaded.prior_uploads, live.prior_uploads);
    }

    #[tokio::test]
    async fn queue_full_is_retracted_when_a_new_entry_passes_capacity_check() {
        // When a genuinely new entry is inserted, it passed the capacity check,
        // so queue-full can be safely retracted: space is available.
        let f = WatcherFixture::new();
        // Set queue-full manually to simulate prior failure
        {
            let mut health = f.shared.health.lock().unwrap();
            health.fail(health::LABEL_QUEUE_FULL, at("2026-08-08T12:00:00Z"));
        }
        assert!(!{ f.shared.health.lock().unwrap().ok() });
        // Write a new session and settle it (two ticks to pass eligibility check)
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        // After settling, a new entry was inserted, which proves capacity check passed
        assert_eq!(f.queue_len(), 1);
        // queue-full should be retracted
        assert!({ f.shared.health.lock().unwrap().ok() });
    }

    #[tokio::test]
    async fn queue_full_survives_when_only_dedup_reobservation_occurs() {
        // When only dedup re-observation occurs (session is already queued),
        // Queue::upsert returns Ok(()) BEFORE checking capacity. Do not retract
        // queue-full, as there is no evidence of available space.
        let f = WatcherFixture::new();
        // Write a session and settle it so it is queued
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);
        // Set queue-full manually
        {
            let mut health = f.shared.health.lock().unwrap();
            health.fail(health::LABEL_QUEUE_FULL, at("2026-08-08T12:00:00Z"));
        }
        assert!(!{ f.shared.health.lock().unwrap().ok() });
        // Tick again: the same session is re-observed (dedup path)
        tick(&f.shared, at("2030-01-02T00:00:00Z")).await.unwrap();
        // Still exactly one entry (dedup did not insert a duplicate)
        assert_eq!(f.queue_len(), 1);
        // queue-full should SURVIVE because dedup does not check capacity
        assert!(
            !{ f.shared.health.lock().unwrap().ok() },
            "queue-full must persist on dedup re-observation"
        );
    }
    #[tokio::test]
    async fn a_session_a_full_queue_cannot_hold_is_never_loaded() {
        // The bug. With a corpus larger than `max_queue_entries` -- 3,152
        // sessions against a cap of 500 on the machine that reported this
        // -- every session the queue has no room for is eligible, has no
        // unchanged offer to be skipped by, and so was read, parsed and
        // hashed in full on every sixty-second poll, purely to be refused
        // `queue-full` afterwards. 74.6% of one core, on an idle app.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(1);
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1, "the queue is now at capacity");

        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let c = loads();
        for minute in 1..=3 {
            f.tick_counted(at(&format!("2030-01-01T00:0{minute}:00Z")), &c);
        }
        assert_eq!(
            count(&c),
            0,
            "a session the full queue cannot hold must not be read at all"
        );
        assert_eq!(f.queue_len(), 1, "and nothing lands, exactly as before");
        assert!(
            !{ f.shared.health.lock().unwrap().ok() },
            "the contributor must still be told the queue is full"
        );
    }

    #[tokio::test]
    async fn a_grown_session_with_a_live_card_still_loads_and_supersedes_at_capacity() {
        // The trap the capacity pre-check must not fall into. A naive "the
        // queue is full, skip" would mean a conversation that grew could
        // never supersede its own stale card once the queue filled up, and
        // the contributor would be left looking at an offer describing
        // content that has moved on. `replace_live_at_path` frees the slot
        // it is about to reuse, so this load can land and must happen.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(1);
        let name = "11111111-1111-1111-1111-111111111111";
        let path = f.write_session("proj", name, 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        let first_hash = {
            let queue = f.shared.queue.lock().unwrap();
            queue.all()[0].session_hash.clone()
        };
        assert_eq!(f.queue_len(), 1, "the queue is at capacity");

        f.append_to_session(&path, "proj", name);
        let c = loads();
        f.settle_counted(at("2030-01-02T00:00:00Z"), &c);
        assert_eq!(
            count(&c),
            1,
            "a grown session with a live card must still be re-read at capacity"
        );

        let queue = f.shared.queue.lock().unwrap();
        let old = queue
            .all()
            .iter()
            .find(|e| e.session_hash == first_hash)
            .expect("the first offer is still on record");
        assert_eq!(old.state, QueueState::Superseded, "{:?}", queue.all());
        assert_eq!(queue.pending().len(), 1, "exactly one live offer");
        assert_ne!(
            queue.pending()[0].session_hash,
            first_hash,
            "and it describes the session as it now stands"
        );
    }

    #[tokio::test]
    async fn a_queue_with_room_still_loads_and_offers_every_session() {
        // The other side of the check: below the cap nothing changes, and a
        // second session is read once and queued.
        let f = WatcherFixture::new();
        f.set_max_queue_entries(10);
        f.write_session("proj", "11111111-1111-1111-1111-111111111111", 0);
        f.settle(at("2030-01-01T00:00:00Z")).await;
        assert_eq!(f.queue_len(), 1);

        f.write_session("proj", "22222222-2222-2222-2222-222222222222", 0);
        let c = loads();
        let report = f.settle_counted(at("2030-01-01T00:01:00Z"), &c);
        assert_eq!(report.queued, 1, "{report:?}");
        assert_eq!(count(&c), 1, "the new session is read exactly once");
        assert_eq!(f.queue_len(), 2);
        assert!(
            { f.shared.health.lock().unwrap().ok() },
            "a queue with room must not report queue-full"
        );
    }
}
