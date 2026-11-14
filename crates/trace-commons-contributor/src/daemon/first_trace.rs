//! First trace observed during this enrollment, independent of history paging.
//!
//! Queue, receipts and history deliberately survive unenrollment and older
//! records carry no enrollment identity. They cannot safely seed this date.
//! An existing installation starts a conservative observation boundary once;
//! subsequent restarts and upgrades retain it. New enrollments establish the
//! boundary before publishing their config. All writes share the credential
//! commit lock, including unenrollment's removal of this file.

use anyhow::Result;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, ContributorConfig};

use super::commons_credentials;

pub(crate) const FILE: &str = "daemon-first-trace.json";

#[derive(Serialize, Deserialize)]
struct Observation {
    authority: String,
    observed_since: DateTime<Utc>,
    first_trace_at: Option<DateTime<Utc>>,
}

fn load(store: &ConfigStore) -> Result<Option<Observation>> {
    store
        .read_daemon_file(FILE)?
        .map(|bytes| serde_json::from_slice(&bytes).map_err(Into::into))
        .transpose()
}

fn save(store: &ConfigStore, observation: &Observation) -> Result<()> {
    store.write_daemon_file(FILE, &serde_json::to_vec(observation)?)
}

/// Caller holds the credential commit lock. Reset before publishing a new
/// authority, so a crash cannot expose the previous enrollment's observation.
pub(super) fn reset_locked(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    now: DateTime<Utc>,
) -> Result<()> {
    let result = save(
        store,
        &Observation {
            authority: commons_credentials::authority(cfg),
            observed_since: now,
            first_trace_at: None,
        },
    );
    if result.is_err() {
        // Missing metadata is safely unknown. Let enrollment proceed if
        // the stale value can be removed, but never publish a new config
        // while an old value could revive on an A -> B -> A transition.
        match std::fs::remove_file(store.daemon_path(FILE)) {
            Ok(()) => return Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(_) => {}
        }
    }
    result
}

/// Observe all persisted queue rows, including resolved and uploaded rows.
/// Old rows retained across an account change predate its boundary and cannot
/// seed it, even when an old queue save finishes after the change.
pub(super) fn observe(store: &ConfigStore, dates: impl Iterator<Item = DateTime<Utc>>) {
    let Ok(Some(cfg)) = store.load_config() else {
        return;
    };
    observe_dates(store, &cfg, dates, false);
}

/// Submission callers supply the config that actually sent the trace, so a
/// response arriving after an account switch never dates the new enrollment.
pub(crate) fn observe_for_config(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    dates: impl Iterator<Item = DateTime<Utc>>,
) {
    observe_dates(store, cfg, dates, true);
}

fn observe_dates(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    dates: impl Iterator<Item = DateTime<Utc>>,
    attributed_submission: bool,
) {
    let result = commons_credentials::with_current_config(store, cfg, || {
        let now = Utc::now();
        let authority = commons_credentials::authority(cfg);
        let mut observation = match load(store)? {
            Some(observation) if observation.authority == authority => observation,
            _ => {
                // Retained queue dates have no trustworthy attribution on
                // first upgrade. A newly completed submission does: its
                // sending config was checked above, even for a CLI-only
                // store that has never started the daemon. Keep the queue
                // boundary at now without dropping that first receipt.
                let first_trace_at = if attributed_submission {
                    dates.filter(|at| *at <= now).min()
                } else {
                    None
                };
                return save(
                    store,
                    &Observation {
                        authority,
                        observed_since: now,
                        first_trace_at,
                    },
                );
            }
        };
        let earliest = dates
            .filter(|at| *at >= observation.observed_since && *at <= now)
            .chain(observation.first_trace_at)
            .min();
        if earliest != observation.first_trace_at {
            observation.first_trace_at = earliest;
            save(store, &observation)?;
        }
        Ok(())
    });
    if result.is_err() {
        // This display fact must never turn a persisted queue or receipt
        // into a reported upload failure. Unreadable state stays unknown.
        tracing::warn!("first_trace_observation_unavailable");
    }
}

/// Read only. Missing, unreadable, or differently scoped state is unknown;
/// the shell must never fall back to an unscoped history-page minimum.
pub(super) fn read(store: &ConfigStore, cfg: Option<&ContributorConfig>) -> Option<DateTime<Utc>> {
    let cfg = cfg?;
    commons_credentials::with_current_config(store, cfg, || {
        Ok(load(store)?
            .filter(|observation| observation.authority == commons_credentials::authority(cfg))
            .and_then(|observation| observation.first_trace_at))
    })
    .ok()
    .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::history::{HistoryCache, HistoryRecord};
    use crate::daemon::ipc::{DaemonShared, Request, handle_request};
    use crate::daemon::queue::{Queue, QueueEntry, QueueState};
    use chrono::Duration;
    use serde_json::json;
    use uuid::Uuid;

    fn enrolled() -> (
        tempfile::TempDir,
        ConfigStore,
        ContributorConfig,
        DateTime<Utc>,
    ) {
        let (dir, store) = crate::config::tests_support::temp_store();
        let cfg = crate::commands::unenrolled_preview_config();
        store.save_config(&cfg).unwrap();
        let boundary = Utc::now() - Duration::days(200);
        commons_credentials::with_current_config(&store, &cfg, || {
            reset_locked(&store, &cfg, boundary)
        })
        .unwrap();
        (dir, store, cfg, boundary)
    }

    fn entry(at: DateTime<Utc>) -> QueueEntry {
        let id = Uuid::new_v4();
        QueueEntry {
            entry_id: id,
            session_hash: format!("sha256:{id}"),
            discovered_at: at,
            ..Default::default()
        }
    }

    #[test]
    fn boundary_is_inclusive_and_rejects_retained_and_future_records() {
        let (_dir, store, cfg, boundary) = enrolled();
        observe(
            &store,
            [
                boundary - Duration::seconds(1),
                Utc::now() + Duration::days(1),
            ]
            .into_iter(),
        );
        assert_eq!(read(&store, Some(&cfg)), None);
        observe(&store, [boundary, boundary + Duration::days(1)].into_iter());
        assert_eq!(read(&store, Some(&cfg)), Some(boundary));
    }

    #[test]
    fn pending_upload_and_history_page_limit_do_not_move_first_trace() {
        let (_dir, store, cfg, boundary) = enrolled();
        let mut queue = Queue::new();
        let first = entry(boundary);
        let first_id = first.entry_id;
        queue.push_for_test(first);
        queue.save(&store).unwrap();
        let mut history = Vec::new();
        for day in 0..150 {
            let at = boundary + Duration::days(day);
            let row: HistoryRecord = serde_json::from_value(json!({
                "submission_id": Uuid::new_v4(), "submitted_at": at,
                "project_label": "project", "source": "claude-code",
                "session_hash": format!("sha256:{day}"), "status": "accepted",
                "consent_scopes": [], "credit_points_pending": 0,
                "explanations": []
            }))
            .unwrap();
            history.push(row);
        }
        history.reverse();
        HistoryCache::save(&store, &history).unwrap();
        queue.set_state(first_id, QueueState::Uploaded, None);
        queue.save(&store).unwrap();
        let shared = DaemonShared::load(store.clone()).unwrap();
        let page = handle_request(
            &shared,
            &Request {
                id: 1,
                method: "list_history".into(),
                params: json!({"limit": 100}),
            },
        )
        .result
        .unwrap();
        assert_eq!(page["history"].as_array().unwrap().len(), 100);
        assert!(
            page["history"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| { row["submitted_at"] != serde_json::to_value(boundary).unwrap() })
        );
        assert_eq!(shared.status_value()["first_trace_at"], json!(boundary));
        assert_eq!(read(&store, Some(&cfg)), Some(boundary));
    }

    #[test]
    fn first_trace_survives_compaction_and_restart() {
        let (_dir, store, cfg, boundary) = enrolled();
        let mut queue = Queue::new();
        for day in 0..60 {
            let mut row = entry(boundary + Duration::days(day));
            row.state = QueueState::Superseded;
            queue.push_for_test(row);
        }
        queue.save(&store).unwrap();
        assert_eq!(queue.compact_superseded(), 10);
        queue.save(&store).unwrap();
        assert!(
            Queue::load(&store)
                .unwrap()
                .all()
                .iter()
                .all(|row| row.discovered_at > boundary)
        );
        let reopened = ConfigStore::open(store.dir().to_path_buf()).unwrap();
        let restarted = DaemonShared::load(reopened).unwrap();
        assert_eq!(read(&store, Some(&cfg)), Some(boundary));
        assert_eq!(restarted.status_value()["first_trace_at"], json!(boundary));
    }

    #[test]
    fn identity_changes_reset_and_delayed_old_observations_cannot_repopulate() {
        let (_dir, store, cfg, boundary) = enrolled();
        let mut old_queue = Queue::new();
        old_queue.push_for_test(entry(boundary));
        old_queue.save(&store).unwrap();
        let mut other = cfg.clone();
        other.tenant_id = "another-tenant".into();
        store.save_config(&other).unwrap();
        old_queue.save(&store).unwrap();
        observe_for_config(&store, &cfg, std::iter::once(Utc::now()));
        assert_eq!(read(&store, Some(&other)), None);
        assert_eq!(read(&store, Some(&cfg)), None);
        // A -> B -> A is a new enrollment observation, not the old A value.
        store.save_config(&cfg).unwrap();
        old_queue.save(&store).unwrap();
        assert_eq!(read(&store, Some(&cfg)), None);
    }

    #[test]
    fn preferences_and_account_session_rotation_preserve_enrollment_observation() {
        let (_dir, store, mut cfg, boundary) = enrolled();
        observe(&store, std::iter::once(boundary));
        cfg.consent_scopes_chosen = Some(true);
        store.save_config(&cfg).unwrap();
        for token in ["first-session", "rotated-session"] {
            store
                .write_daemon_file(
                    crate::config::ACCOUNT_SESSION_FILE,
                    &serde_json::to_vec(&json!({
                        "access_token": token, "account_id": "same-account",
                        "expires_at": Utc::now() + Duration::hours(1)
                    }))
                    .unwrap(),
                )
                .unwrap();
        }
        assert_eq!(read(&store, Some(&cfg)), Some(boundary));
    }

    #[test]
    fn unenrollment_removes_the_date_even_when_the_same_config_returns() {
        let (_dir, store, cfg, boundary) = enrolled();
        observe(&store, std::iter::once(boundary));
        commons_credentials::clear_with(&store, &commons_credentials::Kind::ALL, || {
            store.remove_enrollment_files()
        })
        .unwrap();
        assert!(!store.daemon_path(FILE).exists());
        assert_eq!(read(&store, None), None);
        store.save_config(&cfg).unwrap();
        observe(&store, std::iter::once(boundary));
        assert_eq!(read(&store, Some(&cfg)), None);
    }

    #[test]
    fn first_cli_submission_seeds_missing_legacy_state_without_importing_old_queue() {
        let (_dir, store, cfg, boundary) = enrolled();
        std::fs::remove_file(store.daemon_path(FILE)).unwrap();
        let submitted = Utc::now() - Duration::seconds(1);
        observe_for_config(&store, &cfg, std::iter::once(submitted));
        assert_eq!(read(&store, Some(&cfg)), Some(submitted));
        observe(&store, std::iter::once(boundary));
        assert_eq!(read(&store, Some(&cfg)), Some(submitted));
        DaemonShared::load(store.clone()).unwrap();
        assert_eq!(read(&store, Some(&cfg)), Some(submitted));
    }

    #[test]
    fn observation_write_failure_does_not_reject_a_persisted_queue() {
        let (_dir, store, cfg, boundary) = enrolled();
        std::fs::remove_file(store.daemon_path(FILE)).unwrap();
        std::fs::create_dir(store.daemon_path(FILE)).unwrap();
        let mut queue = Queue::new();
        queue.push_for_test(entry(boundary));
        queue.save(&store).unwrap();
        assert_eq!(Queue::load(&store).unwrap(), queue);
        assert_eq!(read(&store, Some(&cfg)), None);
    }

    #[test]
    fn missing_legacy_state_migrates_once_and_unreadable_state_stays_unknown() {
        let (_dir, store, cfg, boundary) = enrolled();
        std::fs::remove_file(store.daemon_path(FILE)).unwrap();
        assert_eq!(read(&store, Some(&cfg)), None);
        observe(&store, std::iter::once(boundary));
        assert_eq!(read(&store, Some(&cfg)), None);
        let discovered = Utc::now();
        observe(&store, std::iter::once(discovered));
        let original = store.read_daemon_file(FILE).unwrap();
        DaemonShared::load(store.clone()).unwrap();
        assert_eq!(store.read_daemon_file(FILE).unwrap(), original);
        assert_eq!(read(&store, Some(&cfg)), Some(discovered));
        store.write_daemon_file(FILE, b"unreadable").unwrap();
        observe(&store, std::iter::once(discovered));
        assert_eq!(read(&store, Some(&cfg)), None);
        assert_eq!(
            store.read_daemon_file(FILE).unwrap().unwrap(),
            b"unreadable"
        );
    }
}
