//! Carrying observed quota across a restart.
//!
//! What a provider told us about its own capacity is a fact with an expiry, and
//! until now it lived only in process memory. A daemon restart — a config
//! change, a `brew upgrade`, a laptop sleep, the `systemctl --user restart`
//! that `ironwire service install` sets up — threw all of it away, so the next
//! request walked straight back into a rate limit IronWire had been told about
//! five seconds earlier and burned a turn re-learning it.
//!
//! The whole design of this module is in the *load* rules, not the write. A
//! restored number must never be shown as though it were current: `ironwire
//! status` renders the observation's own age, and the one thing that would make
//! this feature worse than not having it is restamping `observed_at` to load
//! time, which turns "we knew this fourteen minutes ago" into a fabricated live
//! reading (`docs/CRITIQUE.md` §4).
//!
//! What is deliberately **not** here:
//!
//! - **Spend.** It is derived from the ledger, which is already durable. Two
//!   sources of truth for a number about money will disagree, and the ledger is
//!   the one that can be audited.
//! - **Circuit-breaker state.** An open circuit is a health inference *we* made,
//!   not something a provider stated — and the commonest reason to restart the
//!   daemon is that something was broken. Coming back up with our own pessimism
//!   intact would make a fixed problem look unfixed. A `retry-after` is
//!   categorically different: it came from the provider, with an expiry on it.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

use crate::quota::{Headroom, QuotaSnapshot};

/// How old an observation may be and still be shown after a restart.
///
/// Anthropic's unified window and Codex's primary window are both measured in
/// hours, so a fifteen-minute-old percentage is still roughly true. Beyond
/// that, `unknown` is the honest answer, and the first request refreshes it
/// anyway.
pub const QUOTA_MAX_AGE: Duration = Duration::minutes(15);

/// Format version, so a future change can be recognised rather than guessed at.
const VERSION: u32 = 1;

/// The on-disk document.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaFile {
    /// Format version.
    pub version: u32,
    /// When this was written.
    pub written_at: DateTime<Utc>,
    /// Per-backend snapshots, keyed by backend id.
    ///
    /// A `BTreeMap` so the file is byte-stable between writes that changed
    /// nothing — which is what lets the writer skip a rewrite by comparing
    /// rendered output.
    pub backends: BTreeMap<String, PersistedQuota>,
}

/// One backend's windows. Deliberately not `QuotaSnapshot`: that type carries
/// `spend_today_usd`, and this file must not.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PersistedQuota {
    /// Primary window.
    pub primary: Headroom,
    /// Secondary window, where the provider exposes one.
    pub secondary: Option<Headroom>,
}

impl From<&QuotaSnapshot> for PersistedQuota {
    fn from(snapshot: &QuotaSnapshot) -> Self {
        Self {
            primary: snapshot.primary.clone(),
            secondary: snapshot.secondary.clone(),
        }
    }
}

/// Render the current state as the document that would be written.
///
/// Separate from writing so the caller can compare it against what it wrote
/// last and skip an unchanged rewrite — this runs on a timer for the life of
/// the daemon.
#[must_use]
pub fn render(quotas: &[(String, QuotaSnapshot)], now: DateTime<Utc>) -> String {
    let file = QuotaFile {
        version: VERSION,
        written_at: now,
        backends: quotas
            .iter()
            .map(|(id, snapshot)| (id.clone(), PersistedQuota::from(snapshot)))
            .collect(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_else(|_| String::new())
}

/// Write the rendered document, atomically and owner-only.
///
/// # Errors
///
/// Propagates the I/O failure. Callers log it and carry on: bookkeeping must
/// never stop the proxy doing its actual job.
pub fn write(path: &Path, contents: &str) -> std::io::Result<()> {
    crate::atomic::write(path, contents)
}

/// Load what a previous run observed, keeping only what is still true.
///
/// Never errors: a missing, empty, truncated or unparseable file means we know
/// nothing, which is exactly the state the daemon starts in anyway. Startup
/// must not fail over a cache.
#[must_use]
pub fn load(path: &Path, now: DateTime<Utc>) -> BTreeMap<String, PersistedQuota> {
    let Ok(raw) = std::fs::read_to_string(path) else {
        return BTreeMap::new();
    };
    let Ok(file) = serde_json::from_str::<QuotaFile>(&raw) else {
        tracing::warn!(path = %path.display(), "stored quota is unreadable; starting without it");
        return BTreeMap::new();
    };
    if file.version != VERSION {
        return BTreeMap::new();
    }
    // A file written in the future means the clock moved backwards — a resync,
    // a timezone fix, a VM restored from a snapshot. Every age and expiry in it
    // is computed against a clock we no longer have, so none of it can be
    // trusted and the honest move is to discard the lot.
    if file.written_at > now {
        tracing::warn!(
            path = %path.display(),
            "stored quota was written in the future; discarding it"
        );
        return BTreeMap::new();
    }

    file.backends
        .into_iter()
        .map(|(id, quota)| {
            (
                id,
                PersistedQuota {
                    primary: still_true(quota.primary, now),
                    secondary: quota.secondary.map(|h| still_true(h, now)),
                },
            )
        })
        .collect()
}

/// Pick out the quota for one backend, if the file has it.
///
/// Lookup is by the *live* registry's ids rather than by iterating the file:
/// an id that is in the file but no longer configured — a backend the user
/// disconnected — must not be resurrected, because quota for something we will
/// never route to is not a fact about anything.
#[must_use]
pub fn for_backend(stored: &BTreeMap<String, PersistedQuota>, id: &str) -> Option<QuotaSnapshot> {
    stored.get(id).map(|quota| QuotaSnapshot {
        primary: quota.primary.clone(),
        secondary: quota.secondary.clone(),
        // Never restored: spend comes from the ledger, which is durable
        // already, and a second source for a number about money is a second
        // number about money.
        spend_today_usd: None,
    })
}

/// Apply the staleness rules to one window.
///
/// `Observed` keeps its **original** `observed_at`. That field is the whole
/// line between "we know this" and "we knew this once", and `ironwire status`
/// renders it directly as "observed 8m ago".
fn still_true(headroom: Headroom, now: DateTime<Utc>) -> Headroom {
    match headroom {
        // The point of the exercise: a stated retry-after outlives us.
        Headroom::Exhausted { until } if until > now => Headroom::Exhausted { until },
        // The window passed while we were down.
        Headroom::Exhausted { .. } => Headroom::Unknown,
        Headroom::Observed { observed_at, .. } if observed_at > now => Headroom::Unknown,
        Headroom::Observed {
            used_pct,
            resets_at,
            observed_at,
        } if now - observed_at <= QUOTA_MAX_AGE => Headroom::Observed {
            used_pct,
            resets_at,
            observed_at,
        },
        // A spend cap is the user's own state, recomputed from the ledger at
        // every start — restoring a stale one would be asserting a budget
        // decision from a previous run against today's spending.
        Headroom::CapReached { .. } | Headroom::Observed { .. } | Headroom::Unknown => {
            Headroom::Unknown
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(offset_secs: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_800_000_000 + offset_secs, 0).expect("valid timestamp")
    }

    fn write_file(dir: &tempfile::TempDir, contents: &str) -> std::path::PathBuf {
        let path = dir.path().join("quota.json");
        std::fs::write(&path, contents).expect("write fixture");
        path
    }

    fn snapshot(primary: Headroom) -> QuotaSnapshot {
        QuotaSnapshot {
            primary,
            secondary: None,
            spend_today_usd: Some(12.34),
        }
    }

    fn round_trip(primary: Headroom, elapsed_secs: i64) -> Headroom {
        let dir = tempfile::tempdir().expect("tempdir");
        let rendered = render(&[("claude-sub".to_string(), snapshot(primary))], t(0));
        let path = write_file(&dir, &rendered);
        load(&path, t(elapsed_secs))
            .remove("claude-sub")
            .expect("backend present")
            .primary
    }

    /// The failure this exists to stop: a stated `retry-after` survives the
    /// restart, so the next request does not walk back into a wall we were
    /// told about.
    #[test]
    fn an_unexpired_retry_after_survives_a_restart() {
        let until = t(3600);
        let restored = round_trip(Headroom::Exhausted { until }, 60);
        assert_eq!(restored, Headroom::Exhausted { until });
        assert!(!restored.is_available(t(60)));
    }

    #[test]
    fn an_expired_retry_after_loads_as_unknown() {
        let restored = round_trip(Headroom::Exhausted { until: t(60) }, 120);
        assert_eq!(restored, Headroom::Unknown);
        assert!(restored.is_available(t(120)));
    }

    /// The line between "we know this" and "we knew this once". Restamping
    /// would render a fourteen-minute-old reading as live.
    #[test]
    fn a_restored_observation_keeps_its_original_timestamp() {
        let observed_at = t(0);
        let restored = round_trip(
            Headroom::Observed {
                used_pct: 82.0,
                resets_at: Some(t(7200)),
                observed_at,
            },
            600,
        );
        match restored {
            Headroom::Observed {
                used_pct,
                observed_at: restored_at,
                ..
            } => {
                assert!((used_pct - 82.0).abs() < f32::EPSILON);
                assert_eq!(restored_at, observed_at, "observed_at was restamped");
            }
            other => panic!("expected an observation, got {other:?}"),
        }
    }

    #[test]
    fn an_observation_older_than_the_max_age_loads_as_unknown() {
        let restored = round_trip(
            Headroom::Observed {
                used_pct: 82.0,
                resets_at: None,
                observed_at: t(0),
            },
            QUOTA_MAX_AGE.num_seconds() + 1,
        );
        assert_eq!(restored, Headroom::Unknown);
    }

    /// A clock that moved backwards invalidates every age and expiry in the
    /// file at once, so the file goes rather than any part of it being trusted.
    #[test]
    fn a_file_from_the_future_is_discarded_whole() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rendered = render(
            &[
                (
                    "claude-sub".to_string(),
                    snapshot(Headroom::Exhausted { until: t(9999) }),
                ),
                (
                    "codex-sub".to_string(),
                    snapshot(Headroom::Exhausted { until: t(9999) }),
                ),
            ],
            t(3600),
        );
        let path = write_file(&dir, &rendered);
        assert!(load(&path, t(0)).is_empty());
    }

    #[test]
    fn a_missing_or_broken_file_is_simply_no_knowledge() {
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(load(&dir.path().join("absent.json"), t(0)).is_empty());
        assert!(load(&write_file(&dir, ""), t(0)).is_empty());
        assert!(load(&write_file(&dir, "{\"version\":1,\"writ"), t(0)).is_empty());
        assert!(load(&write_file(&dir, "null"), t(0)).is_empty());
        assert!(load(&write_file(&dir, "{\"version\":99}"), t(0)).is_empty());
    }

    /// This file records what a provider said about capacity. Anything else in
    /// it would be a leak, and spend in particular has a durable home already.
    #[test]
    fn the_file_carries_capacity_and_nothing_else() {
        let rendered = render(
            &[(
                "claude-sub".to_string(),
                snapshot(Headroom::Observed {
                    used_pct: 82.0,
                    resets_at: None,
                    observed_at: t(0),
                }),
            )],
            t(0),
        );
        assert!(!rendered.contains("spend"), "spend must stay in the ledger");
        assert!(!rendered.contains("12.34"));
        for forbidden in ["token", "Bearer", "sk-", "conversation", "message"] {
            assert!(
                !rendered.contains(forbidden),
                "`{forbidden}` has no business in quota.json:\n{rendered}"
            );
        }
    }

    /// A backend the user has since disconnected is not brought back to life by
    /// a file that still mentions it.
    #[test]
    fn quota_for_a_backend_that_is_gone_is_not_resurrected() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rendered = render(
            &[(
                "openai-key".to_string(),
                snapshot(Headroom::Exhausted { until: t(3600) }),
            )],
            t(0),
        );
        let stored = load(&write_file(&dir, &rendered), t(60));
        assert!(for_backend(&stored, "claude-sub").is_none());
        assert!(for_backend(&stored, "openai-key").is_some());
    }

    /// Spend is the ledger's to report, and the ledger is durable already.
    #[test]
    fn a_restored_snapshot_carries_no_spend() {
        let dir = tempfile::tempdir().expect("tempdir");
        let rendered = render(
            &[("claude-sub".to_string(), snapshot(Headroom::Unknown))],
            t(0),
        );
        let stored = load(&write_file(&dir, &rendered), t(60));
        let restored = for_backend(&stored, "claude-sub").expect("present");
        assert!(restored.spend_today_usd.is_none());
    }

    #[test]
    fn an_unchanged_state_renders_identical_bytes() {
        // What lets the writer skip a rewrite rather than churn the disk every
        // thirty seconds for the life of the daemon.
        let quotas = vec![("claude-sub".to_string(), snapshot(Headroom::Unknown))];
        assert_eq!(render(&quotas, t(0)), render(&quotas, t(0)));
    }

    #[cfg(unix)]
    #[test]
    fn the_file_is_written_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("quota.json");
        write(&path, &render(&[], t(0))).expect("written");
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600, "mode was {:o}", mode & 0o777);
    }
}
