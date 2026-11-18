//! Recorded sample data in place of a daemon. Debug builds only.
//!
//! The Linux half of the macOS app's `SampleDaemonClient`
//! (`macos/Sources/TCShellCore/DataContract/SampleDaemonClient.swift`): when
//! `TRACE_COMMONS_SAMPLE` names a set, the window reads that set's recorded
//! replies instead of reaching a daemon, so the two shells can be drawn from
//! the same data and their captures compared side by side (slice 1 of
//! `docs/superpowers/specs/2026-11-16-linux-mac-window-and-map-design.md`).
//!
//! The replies are the files the macOS app bundles,
//! `macos/Sources/TCShellCore/DataContract/RecordedSamples/<set>/<method>.json`:
//! real answers the real daemon sent against a throwaway store, recorded by
//! `crates/trace-commons-contributor/tests/k2_sample_recorder.rs`, whose drift
//! test keeps them matched to the daemon. They are read from this checkout at
//! run time, by a path fixed at compile time, so there is one copy of them
//! and no second recording to drift.
//!
//! What it never does: start or attach to a daemon, open the state
//! directory, take the lock, enroll, upload or write anything. A method is
//! answered only when its set, or the `shared/` set the macOS app uses for
//! the four writes whose reply never depends on the set (`approve`, `keep`,
//! `undo_keep`, `set_project_mode`), holds a recording of it. Every other
//! method -- every other write, and the network reads the macOS app answers
//! from hand-written Swift -- is refused with one fixed label. A write
//! answered from `shared/` changes nothing: the sets are fixtures, not a
//! simulator.
//!
//! This module does not exist in a release build (`cfg(debug_assertions)`
//! on its declaration in `lib.rs`), and neither does `Backend::Sample`, so a
//! release binary cannot be put on sample data by any environment.

use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow};

/// The variable the macOS app reads (`MonitorWindowView.sampleClient()`).
pub const SAMPLE_ENV: &str = "TRACE_COMMONS_SAMPLE";

/// Every set the macOS app names (`SampleDaemonClient.SampleSet`), in its
/// order. `coreDown` has no files: every call fails, as a daemon that is
/// not there does.
pub const SETS: [&str; 7] = [
    "empty",
    "normalDay",
    "busyQueue",
    "coreDown",
    "unknownCounts",
    "heldSessions",
    "armedFolder",
];

/// The set an empty or unrecognised name falls back to, as on macOS.
pub const DEFAULT_SET: &str = "normalDay";

/// The set whose every call fails.
pub const CORE_DOWN: &str = "coreDown";

/// The directory beside the sets that holds the set-independent writes.
pub const SHARED_DIR: &str = "shared";

/// The answer to a method no recording covers. Fixed and content-free, as
/// every daemon refusal label is.
pub const ERR_SAMPLE_UNRECORDED: &str = "sample-unrecorded";

/// The label `Backend::call` already uses for a daemon that went away, so a
/// `coreDown` window takes the same not-running path a real one does.
pub const ERR_DAEMON_NOT_RUNNING: &str = "daemon-not-running";

/// The recorded samples in this checkout.
pub fn default_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../macos/Sources/TCShellCore/DataContract/RecordedSamples")
}

/// Which set a `TRACE_COMMONS_SAMPLE` value names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Choice {
    pub set: &'static str,
    /// The value was set and non-empty but named no set. The set is then
    /// the default, and the launch says so.
    pub unknown: bool,
}

/// `MonitorWindowView.sampleChoice`, plus its caller's unset check: unset is
/// no sample at all (`None`), empty is the default, a set's name is that
/// set, and anything else is the default marked unknown.
pub fn choice(value: Option<&str>) -> Option<Choice> {
    let value = value?;
    if value.is_empty() {
        return Some(Choice {
            set: DEFAULT_SET,
            unknown: false,
        });
    }
    Some(match SETS.iter().find(|set| **set == value) {
        Some(set) => Choice {
            set,
            unknown: false,
        },
        None => Choice {
            set: DEFAULT_SET,
            unknown: true,
        },
    })
}

/// Whether this launch is on sample data.
pub fn requested() -> bool {
    std::env::var_os(SAMPLE_ENV).is_some()
}

/// A daemon stand-in answering from one recorded set.
#[derive(Debug, Clone)]
pub struct SampleBackend {
    set: &'static str,
    root: PathBuf,
}

impl SampleBackend {
    pub fn new(set: &'static str, root: PathBuf) -> Self {
        Self { set, root }
    }

    /// The set `value` names, over this checkout's recordings; `None` when
    /// the variable is unset. An unrecognised name is logged once, as a
    /// fixed label and the set used, never the value itself.
    pub fn from_value(value: Option<&str>) -> Option<Self> {
        let choice = choice(value)?;
        if choice.unknown {
            eprintln!(
                "trace-commons-shell:{}-unrecognised,fallback={}",
                SAMPLE_ENV, choice.set
            );
        }
        Some(Self::new(choice.set, default_root()))
    }

    pub fn set(&self) -> &'static str {
        self.set
    }

    /// One request, one recorded result, or a fixed refusal.
    pub fn call(&self, method: &str) -> Result<serde_json::Value> {
        if self.set == CORE_DOWN {
            return Err(anyhow!(ERR_DAEMON_NOT_RUNNING));
        }
        // A method name is a file name here: anything but the daemon's
        // snake-case spelling could walk out of the directory.
        if method.is_empty()
            || !method
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        {
            return Err(anyhow!(ERR_SAMPLE_UNRECORDED));
        }
        let file = format!("{method}.json");
        let recorded = [
            self.root.join(self.set).join(&file),
            self.root.join(SHARED_DIR).join(&file),
        ];
        for path in recorded {
            match std::fs::read_to_string(&path) {
                Ok(text) => return Ok(serde_json::from_str(&text)?),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            }
        }
        Err(anyhow!(ERR_SAMPLE_UNRECORDED))
    }
}

/// The sample's event stream: it has no events, so it ends at once. The
/// window's subscription loop then stops, and the window reads only when it
/// reads anyway.
pub struct NoEvents;

impl crate::backend::EventStream for NoEvents {
    fn next(&mut self) -> Option<crate::backend::DaemonEvent> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(set: &str, method: &str) -> serde_json::Value {
        let text = std::fs::read_to_string(default_root().join(set).join(format!("{method}.json")))
            .expect("the recording is in this checkout");
        serde_json::from_str(&text).expect("the recording is JSON")
    }

    #[test]
    fn the_choice_follows_the_macos_app() {
        assert_eq!(choice(None), None, "unset is the daemon, not a sample");
        assert_eq!(
            choice(Some("")),
            Some(Choice {
                set: "normalDay",
                unknown: false
            }),
            "empty is the default set and is not unknown"
        );
        for set in SETS {
            assert_eq!(
                choice(Some(set)),
                Some(Choice {
                    set,
                    unknown: false
                })
            );
        }
        assert_eq!(
            choice(Some("NormalDay")),
            Some(Choice {
                set: "normalDay",
                unknown: true
            }),
            "names are matched exactly, as Swift's rawValue is"
        );
        assert_eq!(
            choice(Some("busy")),
            Some(Choice {
                set: "normalDay",
                unknown: true
            })
        );
    }

    #[test]
    fn every_named_set_but_core_down_has_recordings_in_this_checkout() {
        for set in SETS {
            let dir = default_root().join(set);
            assert_eq!(dir.is_dir(), set != CORE_DOWN, "{}", dir.display());
        }
        assert!(default_root().join(SHARED_DIR).is_dir());
    }

    #[test]
    fn a_read_answers_with_the_recording_for_its_set() {
        for set in ["normalDay", "heldSessions", "empty"] {
            let sample = SampleBackend::new(choice(Some(set)).unwrap().set, default_root());
            for method in [
                "status",
                "list_pending",
                "list_history",
                "get_settings",
                "list_projects",
            ] {
                assert_eq!(
                    sample.call(method).unwrap(),
                    read(set, method),
                    "{set}/{method}"
                );
            }
        }
    }

    #[test]
    fn a_set_independent_write_answers_from_shared() {
        let sample = SampleBackend::new("busyQueue", default_root());
        for method in ["approve", "keep", "undo_keep", "set_project_mode"] {
            assert_eq!(
                sample.call(method).unwrap(),
                read(SHARED_DIR, method),
                "{method}"
            );
        }
    }

    #[test]
    fn every_other_method_is_refused_with_one_fixed_label() {
        let sample = SampleBackend::new("normalDay", default_root());
        for method in [
            "set_settings",
            "enroll",
            "withdraw",
            "unenroll",
            "preview",
            "preview_body",
            "private_ai",
            "",
            "../normalDay/status",
            "status.json",
            "Status",
        ] {
            assert_eq!(
                sample.call(method).unwrap_err().to_string(),
                ERR_SAMPLE_UNRECORDED,
                "{method:?}"
            );
        }
    }

    #[test]
    fn core_down_fails_every_call_as_a_missing_daemon() {
        let sample = SampleBackend::new(CORE_DOWN, default_root());
        for method in ["status", "list_pending", "approve"] {
            assert_eq!(
                sample.call(method).unwrap_err().to_string(),
                ERR_DAEMON_NOT_RUNNING
            );
        }
    }

    #[test]
    fn the_sample_has_no_events() {
        use crate::backend::EventStream;
        assert!(NoEvents.next().is_none());
    }
}
