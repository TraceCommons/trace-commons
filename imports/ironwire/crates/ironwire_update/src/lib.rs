//! Notify-only update checking.
//!
//! IronWire does **not** update itself. It is a daemon holding the user's
//! credentials, sitting in the critical path of an agent that may be halfway
//! through a ten-minute streamed response — and `docs/PROTOCOL.md` §5 says an
//! interrupted stream past the first byte is unrecoverable. A proxy that
//! restarts itself unprompted would be causing exactly the outage it exists to
//! prevent.
//!
//! So this crate does three things and nothing else:
//!
//! 1. Works out **who owns the install**, and defers to them. Self-updating a
//!    Homebrew- or apt-managed binary desyncs the package manager, which is the
//!    most common auto-updater bug there is.
//! 2. Compares versions and decides whether to say anything.
//! 3. Rate-limits the check, so the one thing that phones home does so rarely
//!    and can be switched off (`docs/TRUST.md` §7).
//!
//! Fetching the manifest is the caller's job — this crate holds no HTTP client,
//! which is what makes all of it testable without a network.
#![warn(missing_docs)]

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};

/// How this copy of IronWire got here, and therefore who should upgrade it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum InstallMethod {
    /// Homebrew.
    Homebrew,
    /// A `.deb` from the apt repository.
    Apt,
    /// The npm shim.
    Npm,
    /// A pip wheel.
    Pip,
    /// The shell installer — the only channel with no external manager, and so
    /// the only one where a self-update would be correct.
    ShellInstaller,
    /// A `cargo build` in a checkout, or anything else we cannot place.
    Unmanaged,
}

impl InstallMethod {
    /// The command that upgrades this install, if someone else owns it.
    #[must_use]
    pub fn upgrade_command(self) -> Option<&'static str> {
        match self {
            Self::Homebrew => Some("brew upgrade ironwire"),
            Self::Apt => Some("sudo apt update && sudo apt install --only-upgrade ironwire"),
            Self::Npm => Some("npm install -g ironwire@latest"),
            Self::Pip => Some("pip install --upgrade ironwire"),
            Self::ShellInstaller => Some("curl -fsSL https://ironwire.dev/install.sh | sh"),
            Self::Unmanaged => None,
        }
    }

    /// Infer the owner from where the binary sits.
    ///
    /// Path-based inference is a heuristic, and it is allowed to be: guessing
    /// wrong costs a slightly wrong suggestion in a notification. It never
    /// gates an action, because nothing here takes an action.
    #[must_use]
    pub fn detect(executable: &std::path::Path) -> Self {
        let path = executable.to_string_lossy();
        if path.contains("/Cellar/") || path.contains("/homebrew/") || path.contains("/linuxbrew/")
        {
            Self::Homebrew
        } else if path.starts_with("/usr/bin/") || path.starts_with("/usr/lib/ironwire") {
            Self::Apt
        } else if path.contains("/node_modules/") || path.contains("/.npm/") {
            Self::Npm
        } else if path.contains("/site-packages/") || path.contains("/.venv/") {
            Self::Pip
        } else if path.contains("/.ironwire/bin/") {
            Self::ShellInstaller
        } else {
            Self::Unmanaged
        }
    }
}

/// The release manifest, as published alongside the binaries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manifest {
    /// Latest released version.
    pub latest: String,
    /// Minimum version still expected to work against current provider APIs.
    ///
    /// This is what makes the notification worth reading: below it, IronWire is
    /// likely broken rather than merely old — a provider changed something a
    /// newer release accounts for.
    #[serde(default)]
    pub minimum_supported: Option<String>,
    /// One line on what changed, shown with the notification.
    #[serde(default)]
    pub summary: Option<String>,
}

/// What the user should be told, if anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdateStatus {
    /// Running the latest release, or a build newer than it.
    UpToDate,
    /// A newer release exists.
    Available {
        /// The newer version.
        latest: String,
        /// One line on what changed.
        summary: Option<String>,
        /// How to get it, when someone else owns the install.
        upgrade_command: Option<String>,
    },
    /// Older than `minimum_supported`: likely broken against current provider
    /// APIs, not merely out of date.
    Unsupported {
        /// The newer version.
        latest: String,
        /// The floor this build is below.
        minimum_supported: String,
        /// How to get it.
        upgrade_command: Option<String>,
    },
    /// No check has succeeded yet. Not an error worth showing.
    Unknown,
}

impl UpdateStatus {
    /// Whether this is worth putting in front of the user.
    #[must_use]
    pub fn is_actionable(&self) -> bool {
        matches!(self, Self::Available { .. } | Self::Unsupported { .. })
    }
}

/// Compare a running version against a manifest.
///
/// An unparseable version on either side yields [`UpdateStatus::Unknown`]: a
/// nag based on a version we could not read would be worse than silence.
#[must_use]
pub fn evaluate(current: &str, manifest: &Manifest, install: InstallMethod) -> UpdateStatus {
    let (Ok(current), Ok(latest)) = (
        semver::Version::parse(current.trim_start_matches('v')),
        semver::Version::parse(manifest.latest.trim_start_matches('v')),
    ) else {
        return UpdateStatus::Unknown;
    };
    let upgrade_command = install.upgrade_command().map(str::to_string);

    if let Some(floor) = manifest
        .minimum_supported
        .as_deref()
        .and_then(|v| semver::Version::parse(v.trim_start_matches('v')).ok())
        && current < floor
    {
        return UpdateStatus::Unsupported {
            latest: latest.to_string(),
            minimum_supported: floor.to_string(),
            upgrade_command,
        };
    }
    // `>=` so a local build ahead of the release does not nag.
    if current >= latest {
        return UpdateStatus::UpToDate;
    }
    UpdateStatus::Available {
        latest: latest.to_string(),
        summary: manifest.summary.clone(),
        upgrade_command,
    }
}

/// How often to check. Rare on purpose: this is the one request IronWire makes
/// that is not the user's work (`docs/TRUST.md` §7).
pub const CHECK_INTERVAL: Duration = Duration::hours(24);

/// The cached result of the last check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckedAt {
    /// When the last check completed.
    pub at: DateTime<Utc>,
    /// What it concluded.
    pub status: UpdateStatus,
}

/// Whether a check is due.
///
/// `enabled` is the user's kill switch and is honoured before anything else —
/// including the "never checked" case, so switching it off means no request is
/// ever made rather than one last one.
#[must_use]
pub fn should_check(enabled: bool, last: Option<&CheckedAt>, now: DateTime<Utc>) -> bool {
    if !enabled {
        return false;
    }
    last.is_none_or(|last| now - last.at >= CHECK_INTERVAL)
}

/// Load the cached check, if any. A corrupt cache is simply no cache.
#[must_use]
pub fn load_cache(path: &std::path::Path) -> Option<CheckedAt> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// Persist a check result.
///
/// # Errors
///
/// Propagates I/O and serialisation failures; a cache we could not write only
/// means the next check happens sooner.
pub fn save_cache(path: &std::path::Path, checked: &CheckedAt) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_string_pretty(checked)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn manifest(latest: &str, minimum: Option<&str>) -> Manifest {
        Manifest {
            latest: latest.to_string(),
            minimum_supported: minimum.map(str::to_string),
            summary: Some("fixes the Anthropic OAuth beta flag".to_string()),
        }
    }

    #[test]
    fn a_newer_release_is_reported_with_the_right_upgrade_command() {
        // The point of detecting the install method: telling a Homebrew user to
        // re-run the shell installer would leave brew's world view broken.
        let status = evaluate("0.1.0", &manifest("0.2.0", None), InstallMethod::Homebrew);
        match status {
            UpdateStatus::Available {
                latest,
                upgrade_command,
                ..
            } => {
                assert_eq!(latest, "0.2.0");
                assert_eq!(upgrade_command.as_deref(), Some("brew upgrade ironwire"));
            }
            other => panic!("expected an available update, got {other:?}"),
        }
    }

    #[test]
    fn being_below_the_supported_floor_is_reported_differently() {
        // "You are probably broken" and "you are a bit old" deserve different
        // words — the first is why this channel is worth having at all.
        let status = evaluate(
            "0.1.0",
            &manifest("0.4.0", Some("0.3.0")),
            InstallMethod::ShellInstaller,
        );
        assert!(matches!(status, UpdateStatus::Unsupported { .. }));
        assert!(status.is_actionable());
    }

    #[test]
    fn a_local_build_ahead_of_the_release_does_not_nag() {
        let status = evaluate("0.3.0", &manifest("0.2.0", None), InstallMethod::Unmanaged);
        assert_eq!(status, UpdateStatus::UpToDate);
        assert!(!status.is_actionable());
    }

    #[test]
    fn matching_the_latest_release_is_up_to_date() {
        assert_eq!(
            evaluate("0.2.0", &manifest("0.2.0", None), InstallMethod::Apt),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn a_v_prefix_on_either_side_is_tolerated() {
        assert_eq!(
            evaluate("v0.2.0", &manifest("v0.2.0", None), InstallMethod::Apt),
            UpdateStatus::UpToDate
        );
    }

    #[test]
    fn an_unreadable_version_produces_silence_rather_than_a_wrong_nag() {
        assert_eq!(
            evaluate(
                "not-a-version",
                &manifest("0.2.0", None),
                InstallMethod::Apt
            ),
            UpdateStatus::Unknown
        );
        assert_eq!(
            evaluate("0.1.0", &manifest("garbage", None), InstallMethod::Apt),
            UpdateStatus::Unknown
        );
    }

    #[test]
    fn an_unmanaged_build_gets_no_upgrade_command_invented_for_it() {
        let status = evaluate("0.1.0", &manifest("0.2.0", None), InstallMethod::Unmanaged);
        match status {
            UpdateStatus::Available {
                upgrade_command, ..
            } => assert!(upgrade_command.is_none()),
            other => panic!("expected an available update, got {other:?}"),
        }
    }

    #[test]
    fn the_install_owner_is_inferred_from_where_the_binary_sits() {
        let cases = [
            (
                "/opt/homebrew/Cellar/ironwire/0.1.0/bin/ironwire",
                InstallMethod::Homebrew,
            ),
            ("/usr/bin/ironwire", InstallMethod::Apt),
            (
                "/home/u/proj/node_modules/@ironwire/cli-linux-x64/ironwire",
                InstallMethod::Npm,
            ),
            (
                "/home/u/.venv/lib/python3.12/site-packages/ironwire/ironwire",
                InstallMethod::Pip,
            ),
            (
                "/home/u/.ironwire/bin/ironwire",
                InstallMethod::ShellInstaller,
            ),
            (
                "/home/u/src/ironwire/target/release/ironwire",
                InstallMethod::Unmanaged,
            ),
        ];
        for (path, expected) in cases {
            assert_eq!(InstallMethod::detect(Path::new(path)), expected, "{path}");
        }
    }

    #[test]
    fn the_kill_switch_stops_every_check_including_the_first() {
        // Switching it off must mean "no request is ever made", not "one more".
        assert!(!should_check(false, None, Utc::now()));
        let recent = CheckedAt {
            at: Utc::now() - Duration::days(30),
            status: UpdateStatus::Unknown,
        };
        assert!(!should_check(false, Some(&recent), Utc::now()));
    }

    #[test]
    fn checks_are_rate_limited_to_once_a_day() {
        let now = Utc::now();
        assert!(should_check(true, None, now), "first run should check");
        let just_now = CheckedAt {
            at: now - Duration::hours(1),
            status: UpdateStatus::UpToDate,
        };
        assert!(!should_check(true, Some(&just_now), now));
        let yesterday = CheckedAt {
            at: now - Duration::hours(25),
            status: UpdateStatus::UpToDate,
        };
        assert!(should_check(true, Some(&yesterday), now));
    }

    #[test]
    fn the_cache_round_trips_and_a_corrupt_one_is_simply_absent() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("update.json");
        assert!(load_cache(&path).is_none());

        let checked = CheckedAt {
            at: DateTime::from_timestamp(1_700_000_000, 0).expect("timestamp"),
            status: UpdateStatus::Available {
                latest: "0.2.0".into(),
                summary: None,
                upgrade_command: None,
            },
        };
        save_cache(&path, &checked).expect("saves");
        assert_eq!(load_cache(&path), Some(checked));

        std::fs::write(&path, "{ not json").expect("writes");
        assert!(load_cache(&path).is_none());
    }
}
