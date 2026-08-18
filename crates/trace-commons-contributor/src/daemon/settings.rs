//! Daemon configuration: the knobs governing how patient, how chatty, and how
//! autonomous the background uploader is.
//!
//! These are persisted rather than read from the process environment because a
//! daemon started by a service manager inherits none of the user's shell
//! environment. Settings read from env would leave every upload refusing with
//! `pii-filter-unavailable` under systemd while working perfectly by hand.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE};
use crate::envelope::NearAiSettings;

pub const DAEMON_SETTINGS_SCHEMA: &str = "trace_commons.daemon_settings.v1";

/// How long a session must go unwritten before it counts as finished.
const DEFAULT_QUIESCENCE_SECS: u64 = 1800;
/// How often the watcher stats the session roots. Much finer than the
/// quiescence window, so the poll rate costs nothing in responsiveness.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
/// Minimum gap between digest notifications, so a busy day is one interruption
/// rather than a dozen.
const DEFAULT_DIGEST_INTERVAL_SECS: u64 = 14_400;
const DEFAULT_QUEUE_TTL_DAYS: i64 = 14;
/// A resumed session must grow by this factor to be worth re-uploading.
const DEFAULT_GROWTH_FACTOR: f64 = 2.0;
/// ...or by this many absolute bytes, which is what actually catches growth on
/// an already-large session.
const DEFAULT_GROWTH_MIN_NEW_BYTES: u64 = 65_536;
/// A session re-uploads at most this many times. Each re-upload re-sends the
/// whole file, so an unbounded count would pay the privacy-filter bill
/// repeatedly over the same text and dilute the contributor's own credit
/// through server-side duplicate clustering.
const DEFAULT_MAX_REUPLOADS: u32 = 3;
const DEFAULT_MAX_UPLOADS_PER_DAY: u32 = 50;
const DEFAULT_MAX_BYTES_PER_DAY: u64 = 209_715_200;
const DEFAULT_MAX_QUEUE_ENTRIES: usize = 500;
const DEFAULT_HISTORY_POLL_SECS: u64 = 1800;
/// How often the public community roster is fetched. The server serves a
/// pre-rendered snapshot and refuses to serve one older than fifteen minutes,
/// so polling faster than that only re-fetches a body the server has not
/// recomputed. Fifteen minutes is the roster's own cadence, and this follows
/// it rather than inventing a second one.
const DEFAULT_COMMUNITY_POLL_SECS: u64 = 900;
/// A privacy-filter self-test from days ago proves nothing about the filter
/// now, so a long-lived process re-checks on this interval.
const DEFAULT_CANARY_INTERVAL_SECS: u64 = 3600;
/// How long an approval is held before the uploader will touch it, which is
/// how long a contributor's "Undo" really lasts.
///
/// The designed affordance is a five-second undo after approving. Five
/// seconds is therefore the floor, not the target: the client's countdown
/// starts when it renders the response, which is already after the approval
/// was stamped, and the cancel that ends it has to travel back over the
/// socket. Ten leaves room for both, plus the second or two of clock skew
/// between an application counting in its own process and a daemon deciding
/// in another, and costs nothing that matters -- uploads are unattended
/// background work on a 60-second poll, so an armed project's traces still
/// go out on the very next tick.
///
/// Zero disables the hold, restoring the old behaviour for anyone who wants
/// it; a client is expected to stop offering an undo when
/// `approve` reports no `hold_until`.
const DEFAULT_APPROVAL_HOLD_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonSettings {
    pub schema_version: String,
    pub poll_interval_secs: u64,
    pub quiescence_secs: u64,
    pub digest_interval_secs: u64,
    pub queue_ttl_days: i64,
    pub growth_factor: f64,
    pub growth_min_new_bytes: u64,
    pub max_reuploads: u32,
    pub max_uploads_per_day: u32,
    pub max_bytes_per_day: u64,
    pub max_queue_entries: usize,
    pub history_poll_secs: u64,
    /// How often the public community roster is fetched. `#[serde(default)]`
    /// so a settings file written before this field existed loads with the
    /// poll on its published cadence rather than failing to parse.
    #[serde(default = "default_community_poll_secs")]
    pub community_poll_secs: u64,
    pub canary_interval_secs: u64,
    /// How long after an approval the uploader must leave the entry alone,
    /// so the undo a client offers is real rather than a race against the
    /// next upload pass. See `DEFAULT_APPROVAL_HOLD_SECS` and
    /// `queue::QueueEntry::approved_at`.
    ///
    /// `#[serde(default = ...)]` so a settings file written before this
    /// field existed loads with the hold on rather than off: a missing key
    /// must not silently mean "no undo window".
    #[serde(default = "default_approval_hold_secs")]
    pub approval_hold_secs: u64,
    /// Whether the daemon itself renders OS notifications. Off by default:
    /// the native applications render their own, and the daemon's shell-out
    /// path needs a desktop session it may not have.
    pub local_notifications: bool,
    /// Privacy-filter credentials, persisted so a service-managed daemon can
    /// reach the filter without a shell environment.
    pub near_ai: Option<NearAiSettings>,
    /// Session roots to watch. `None` means the conventional per-user
    /// location for that agent. Overridable so the daemon can follow a
    /// relocated store, and so tests never touch the real one.
    #[serde(default)]
    pub claude_root: Option<PathBuf>,
    #[serde(default)]
    pub codex_root: Option<PathBuf>,
}

fn default_approval_hold_secs() -> u64 {
    DEFAULT_APPROVAL_HOLD_SECS
}

fn default_community_poll_secs() -> u64 {
    DEFAULT_COMMUNITY_POLL_SECS
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            schema_version: DAEMON_SETTINGS_SCHEMA.to_string(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            quiescence_secs: DEFAULT_QUIESCENCE_SECS,
            digest_interval_secs: DEFAULT_DIGEST_INTERVAL_SECS,
            queue_ttl_days: DEFAULT_QUEUE_TTL_DAYS,
            growth_factor: DEFAULT_GROWTH_FACTOR,
            growth_min_new_bytes: DEFAULT_GROWTH_MIN_NEW_BYTES,
            max_reuploads: DEFAULT_MAX_REUPLOADS,
            max_uploads_per_day: DEFAULT_MAX_UPLOADS_PER_DAY,
            max_bytes_per_day: DEFAULT_MAX_BYTES_PER_DAY,
            max_queue_entries: DEFAULT_MAX_QUEUE_ENTRIES,
            history_poll_secs: DEFAULT_HISTORY_POLL_SECS,
            community_poll_secs: DEFAULT_COMMUNITY_POLL_SECS,
            canary_interval_secs: DEFAULT_CANARY_INTERVAL_SECS,
            approval_hold_secs: DEFAULT_APPROVAL_HOLD_SECS,
            local_notifications: false,
            near_ai: None,
            claude_root: None,
            codex_root: None,
        }
    }
}

impl DaemonSettings {
    /// Load persisted settings, falling back to defaults when the daemon has
    /// never been configured on this machine.
    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_SETTINGS_FILE)? else {
            return Ok(Self::default());
        };
        serde_json::from_slice(&body).context("parsing daemon settings")
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon settings")?;
        store.write_daemon_file(DAEMON_SETTINGS_FILE, &body)
    }
}

/// A partial-settings object held nothing this function recognizes.
pub const ERR_SETTINGS_NOT_OBJECT: &str = "settings-not-object";
/// A partial-settings object had a top-level key this function does not
/// recognize.
pub const ERR_SETTINGS_UNKNOWN_FIELD: &str = "settings-unknown-field";
/// A recognized key held a value of the wrong JSON type (or, for
/// `claude_root`/`codex_root`, a JSON type other than string/null).
pub const ERR_SETTINGS_INVALID_VALUE: &str = "settings-invalid-value";

/// Apply a partial settings object -- the shape `tc_call(handle,
/// "set_settings", ...)` takes over the socket, and the shape
/// `tc_daemon_start_with_settings` takes over the C ABI before the daemon's
/// first supervisor tick -- onto `settings` in place. One function, so
/// both callers share one definition of "a valid settings object" rather
/// than two that can drift.
///
/// Every top-level key must be one this function recognizes; an
/// unrecognized key is rejected outright (`Err(ERR_SETTINGS_UNKNOWN_FIELD)`)
/// rather than silently ignored. Silently ignoring a misspelled
/// `claude_root` is exactly the bug this exists to prevent: a caller that
/// meant to redirect the watcher and typo'd the key would otherwise get no
/// signal at all, and the daemon would quietly go on scanning wherever it
/// was already pointed.
///
/// Every error is a fixed, content-free `&'static str` label. In
/// particular, a bad `claude_root`/`codex_root` value never appears in the
/// label -- only the recognized field name distinguishes one failure from
/// another, and the field *names* are a small, fixed, known set, never
/// caller-supplied text. The values themselves (which is where a
/// filesystem path lives) never cross into an error string.
///
/// Returns whether anything was applied. An empty object (or one holding
/// only keys whose values happen to match the current setting) still
/// reports `true` for any key present and accepted -- this always applies
/// every key it accepts, so `Ok(false)` only ever means "the object had no
/// keys at all". Callers that require at least one recognized field (as
/// `set_settings` does, to catch an empty or accidental call) check that
/// themselves; `tc_daemon_start_with_settings` does not, since "nothing to
/// override" is its documented no-op case.
pub fn apply_settings_object(
    settings: &mut DaemonSettings,
    params: &serde_json::Value,
) -> std::result::Result<bool, &'static str> {
    let obj = params.as_object().ok_or(ERR_SETTINGS_NOT_OBJECT)?;
    let mut changed = false;
    for (key, value) in obj {
        match key.as_str() {
            "quiescence_secs" => {
                settings.quiescence_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "digest_interval_secs" => {
                settings.digest_interval_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "approval_hold_secs" => {
                settings.approval_hold_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "local_notifications" => {
                settings.local_notifications = value.as_bool().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "claude_root" => {
                settings.claude_root = parse_optional_root(value)?;
            }
            "codex_root" => {
                settings.codex_root = parse_optional_root(value)?;
            }
            _ => return Err(ERR_SETTINGS_UNKNOWN_FIELD),
        }
        changed = true;
    }
    Ok(changed)
}

/// `null` clears the override (falls back to the conventional per-user
/// location); a string sets it; anything else is a type error. Never
/// formats `value` into the error -- see `apply_settings_object`'s doc.
fn parse_optional_root(
    value: &serde_json::Value,
) -> std::result::Result<Option<PathBuf>, &'static str> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(PathBuf::from(s))),
        _ => Err(ERR_SETTINGS_INVALID_VALUE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    #[test]
    fn settings_round_trip_through_the_store() {
        let (_d, store) = temp_store();
        let s = DaemonSettings {
            quiescence_secs: 60,
            ..Default::default()
        };
        s.save(&store).unwrap();
        assert_eq!(DaemonSettings::load(&store).unwrap().quiescence_secs, 60);
    }

    #[test]
    fn settings_default_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        let s = DaemonSettings::load(&store).unwrap();
        assert_eq!(s.quiescence_secs, DEFAULT_QUIESCENCE_SECS);
        assert_eq!(s.max_reuploads, DEFAULT_MAX_REUPLOADS);
        assert!(!s.local_notifications, "notifications must be opt-in");
        assert!(s.near_ai.is_none());
    }

    #[test]
    fn the_approval_hold_defaults_to_more_than_the_five_second_undo() {
        // The client-side undo is five seconds. A hold shorter than that
        // would leave the same race the hold exists to remove, so the
        // default is a floor with margin rather than an exact match.
        let s = DaemonSettings::default();
        assert_eq!(s.approval_hold_secs, DEFAULT_APPROVAL_HOLD_SECS);
        assert!(s.approval_hold_secs >= 5);
    }

    #[test]
    fn a_settings_file_written_before_the_hold_existed_loads_with_it_on() {
        // A missing key must not silently mean "no undo window".
        let (_d, store) = temp_store();
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v.as_object_mut().unwrap().remove("approval_hold_secs");
        store
            .write_daemon_file(DAEMON_SETTINGS_FILE, v.to_string().as_bytes())
            .unwrap();
        assert_eq!(
            DaemonSettings::load(&store).unwrap().approval_hold_secs,
            DEFAULT_APPROVAL_HOLD_SECS
        );
    }

    #[test]
    fn the_approval_hold_is_settable_and_type_checked() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"approval_hold_secs": 30})),
            Ok(true)
        );
        assert_eq!(s.approval_hold_secs, 30);
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"approval_hold_secs": "30"})),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(s.approval_hold_secs, 30, "a rejected value changes nothing");
    }

    #[test]
    fn settings_are_written_readable_only_by_the_owner() {
        // near_ai carries an API key.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (_d, store) = temp_store();
            DaemonSettings::default().save(&store).unwrap();
            let meta = std::fs::metadata(store.daemon_path(DAEMON_SETTINGS_FILE)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }
}
