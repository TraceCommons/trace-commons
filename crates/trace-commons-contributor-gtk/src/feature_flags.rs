//! The daemon's feature-flag snapshot, read the way the Mac reads it
//! (`macos/Sources/TCShellCore/FeatureFlags.swift`,
//! `TraceCommonsApp/DaemonClient+FeatureFlags.swift` and
//! `AppModel+FeatureFlags.swift`).
//!
//! A daemon snapshot, never a second evaluator: the daemon resolves every
//! decision (`trace_commons_contributor::feature_flags`), and this shell
//! decodes its `feature_flags_snapshot` answer, checks that the answer is
//! about this build's profile, keeps the newest, and reads `enabled` off
//! it. A flag the snapshot does not name, and every flag while there is no
//! snapshot, is off.
//!
//! The container is decoded leniently, as the Mac's `Decodable` is: the
//! contributor's `Snapshot` and the protocol's `Decision` both deny unknown
//! fields, so a newer daemon -- a Flatpak window attached to a host daemon
//! installed separately -- that added one would read here as every flag
//! off while the Mac kept reading it. The protocol's `Environment` and
//! `Timing` are reused as they are.

use std::cell::{Cell, RefCell};
use std::collections::BTreeMap;

use serde::Deserialize;
use trace_commons_contributor::feature_flags::{BuildProfileIdentity, Environment, Timing};

/// The daemon method this module reads.
pub const METHOD_SNAPSHOT: &str = "feature_flags_snapshot";

/// The daemon's event for a changed snapshot. Opt-in on a socket
/// subscription: see `backend::SocketEvents`.
pub use trace_commons_contributor::daemon::ipc::EVENT_FEATURE_FLAGS_CHANGED;

/// One flag's decision, as the daemon resolved it.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Decision {
    pub enabled: bool,
    pub pending: bool,
    pub capability_blocked: bool,
    pub timing: Timing,
}

/// Where the daemon's decisions came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    Default,
    Remote,
    Cache,
}

/// The fields of `feature_flags_snapshot` the Mac decodes, and no others.
#[derive(Deserialize)]
struct Wire {
    schema_version: i64,
    environment: Environment,
    profile_digest: String,
    instance_id: String,
    generation: u64,
    sampled_at: String,
    source: Source,
    revision: Option<u64>,
    expires_at: Option<String>,
    fresh: bool,
    flags: BTreeMap<String, Decision>,
}

/// A validated daemon snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub environment: Environment,
    pub profile_digest: String,
    pub instance_id: String,
    pub generation: u64,
    pub sampled_at: String,
    pub source: Source,
    pub revision: Option<u64>,
    pub expires_at: Option<String>,
    pub fresh: bool,
    pub flags: BTreeMap<String, Decision>,
}

impl Snapshot {
    /// Decode a `feature_flags_snapshot` answer. `None` for anything the
    /// Mac refuses: another schema, generation zero, an instance id that is
    /// not a UUID, or a digest that is not 64 lowercase hex characters.
    pub fn decode(value: serde_json::Value) -> Option<Self> {
        let wire: Wire = serde_json::from_value(value).ok()?;
        let valid = wire.schema_version == 1
            && wire.generation > 0
            && instance_id_valid(&wire.instance_id)
            && digest_valid(&wire.profile_digest);
        valid.then_some(Self {
            environment: wire.environment,
            profile_digest: wire.profile_digest,
            instance_id: wire.instance_id,
            generation: wire.generation,
            sampled_at: wire.sampled_at,
            source: wire.source,
            revision: wire.revision,
            expires_at: wire.expires_at,
            fresh: wire.fresh,
            flags: wire.flags,
        })
    }

    /// Whether the daemon resolved `id` on. Off when it names no such flag.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.flags.get(id).is_some_and(|d| d.enabled)
    }

    /// Whether `newer` may replace this snapshot: the same profile, and
    /// either another daemon instance or a later generation of this one.
    pub fn accepts_successor(&self, newer: &Snapshot) -> bool {
        self.environment == newer.environment
            && self.profile_digest == newer.profile_digest
            && (self.instance_id != newer.instance_id || newer.generation > self.generation)
    }
}

/// What Foundation's `UUID(uuidString:)` takes: the hyphenated form.
fn instance_id_valid(id: &str) -> bool {
    id.len() == 36 && uuid::Uuid::try_parse(id).is_ok()
}

/// 64 lowercase hexadecimal characters.
fn digest_valid(digest: &str) -> bool {
    digest.len() == 64
        && digest
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The Mac's `BuildFeatureProfile.agrees(environment:digest:)`: whether
/// this build's identity is valid and names `environment` and `digest`.
pub fn identity_agrees(
    identity: &BuildProfileIdentity,
    environment: Option<&str>,
    digest: Option<&str>,
) -> bool {
    identity.schema_version == 1
        && digest_valid(&identity.profile_digest)
        && environment == Some(identity.environment.as_str())
        && digest == Some(identity.profile_digest.as_str())
}

/// This build's feature profile identity, the one `tc_feature_profile_json`
/// gives the Mac. `None` when it cannot be read.
pub fn build_identity() -> Option<BuildProfileIdentity> {
    trace_commons_contributor::feature_flags::build_profile_identity().ok()
}

/// A `feature_flags_snapshot` answer, kept only when it decodes and is
/// about `identity`'s profile (the Mac's `featureFlagsSnapshot()`).
pub fn read_answer(
    answer: Result<serde_json::Value, String>,
    identity: Option<&BuildProfileIdentity>,
) -> Option<Snapshot> {
    let snapshot = Snapshot::decode(answer.ok()?)?;
    identity_agrees(
        identity?,
        Some(snapshot.environment.as_str()),
        Some(&snapshot.profile_digest),
    )
    .then_some(snapshot)
}

/// The window's current snapshot (the Mac's `AppModel.featureFlags` and
/// `featureFlagReadGeneration`).
#[derive(Default)]
pub struct FeatureFlags {
    current: RefCell<Option<Snapshot>>,
    reads: Cell<u64>,
}

impl FeatureFlags {
    /// Start a read. Only the answer to the latest read is applied.
    pub fn begin_read(&self) -> u64 {
        let read = self.reads.get().wrapping_add(1);
        self.reads.set(read);
        read
    }

    /// Apply the answer to read `read`: a stale read changes nothing, a
    /// failed or mismatched one clears the snapshot, and a newer one
    /// replaces it unless the current one refuses it as a successor.
    /// Whether anything changed.
    pub fn finish_read(&self, read: u64, answer: Option<Snapshot>) -> bool {
        if read != self.reads.get() {
            return false;
        }
        let mut current = self.current.borrow_mut();
        match answer {
            None => current.take().is_some(),
            Some(newer) => {
                if current
                    .as_ref()
                    .is_some_and(|old| !old.accepts_successor(&newer))
                {
                    return false;
                }
                let changed = current.as_ref() != Some(&newer);
                *current = Some(newer);
                changed
            }
        }
    }

    /// Whether the daemon resolved `id` on. Off with no snapshot.
    pub fn is_enabled(&self, id: &str) -> bool {
        self.current
            .borrow()
            .as_ref()
            .is_some_and(|snapshot| snapshot.is_enabled(id))
    }

    /// The daemon was replaced: hold nothing, and drop the answer to any
    /// read made before (the Mac clears `featureFlags` on a new daemon).
    pub fn forget(&self) {
        self.begin_read();
        self.current.borrow_mut().take();
    }

    /// The snapshot held now.
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.current.borrow().clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A real daemon's `feature_flags_snapshot` answer, recorded from a
    /// daemon this shell hosted (`backend::feature_flag_tests`), with its
    /// instance id, timestamps and digest as the daemon wrote them.
    const RECORDED: &str = include_str!("../tests/fixtures/feature_flags_snapshot.json");

    fn recorded() -> serde_json::Value {
        serde_json::from_str(RECORDED).unwrap()
    }

    fn identity_of(snapshot: &Snapshot) -> BuildProfileIdentity {
        BuildProfileIdentity {
            schema_version: 1,
            environment: snapshot.environment,
            profile_digest: snapshot.profile_digest.clone(),
        }
    }

    fn with(value: &serde_json::Value, key: &str, field: serde_json::Value) -> serde_json::Value {
        let mut value = value.clone();
        value[key] = field;
        value
    }

    #[test]
    fn a_recorded_daemon_snapshot_decodes() {
        let snapshot = Snapshot::decode(recorded()).expect("the daemon's own answer decodes");
        assert_eq!(
            snapshot.generation,
            recorded()["generation"].as_u64().unwrap()
        );
        assert_eq!(
            snapshot.instance_id,
            recorded()["instance_id"].as_str().unwrap()
        );
        assert!(
            snapshot.flags.contains_key("insights_skills")
                && snapshot.flags.contains_key("claude_desktop_private_ai"),
            "the registry's flags are all named"
        );
        for (id, decision) in &snapshot.flags {
            assert_eq!(
                snapshot.is_enabled(id),
                recorded()["flags"][id]["enabled"].as_bool().unwrap(),
                "{id} reads its own decision"
            );
            assert_eq!(
                decision.timing == Timing::Live,
                recorded()["flags"][id]["timing"] == "live"
            );
        }
    }

    #[test]
    fn a_field_this_shell_does_not_know_is_ignored_as_on_the_mac() {
        let mut value = with(&recorded(), "a_later_field", serde_json::json!(1));
        value["flags"]["insights_skills"]["a_later_field"] = serde_json::json!(true);
        assert!(Snapshot::decode(value).is_some());
    }

    #[test]
    fn what_the_mac_refuses_does_not_decode() {
        let base = recorded();
        for (key, bad) in [
            ("schema_version", serde_json::json!(2)),
            ("generation", serde_json::json!(0)),
            ("instance_id", serde_json::json!("not-a-uuid")),
            ("environment", serde_json::json!("development")),
            ("profile_digest", serde_json::json!("ABC")),
            ("profile_digest", serde_json::json!("A".repeat(64))),
            ("source", serde_json::json!("guess")),
            ("fresh", serde_json::Value::Null),
        ] {
            assert!(
                Snapshot::decode(with(&base, key, bad.clone())).is_none(),
                "{key} = {bad} must not decode"
            );
        }
        let mut no_flags = base.clone();
        no_flags.as_object_mut().unwrap().remove("flags");
        assert!(Snapshot::decode(no_flags).is_none());
    }

    #[test]
    fn an_unknown_flag_is_off() {
        let mut value = recorded();
        value["flags"]["insights_skills"]["enabled"] = serde_json::json!(true);
        let snapshot = Snapshot::decode(value).unwrap();
        assert!(snapshot.is_enabled("insights_skills"));
        assert!(!snapshot.is_enabled("no_such_flag"));
        let flags = FeatureFlags::default();
        let read = flags.begin_read();
        flags.finish_read(read, Some(snapshot));
        assert!(flags.is_enabled("insights_skills"));
        assert!(!flags.is_enabled("no_such_flag"));
    }

    #[test]
    fn with_no_snapshot_every_flag_is_off() {
        let flags = FeatureFlags::default();
        assert!(flags.snapshot().is_none());
        assert!(!flags.is_enabled("insights_skills"));
        assert!(!flags.is_enabled("claude_desktop_private_ai"));

        // A failed read clears one that was held.
        let mut value = recorded();
        value["flags"]["insights_skills"]["enabled"] = serde_json::json!(true);
        let read = flags.begin_read();
        flags.finish_read(read, Snapshot::decode(value));
        assert!(flags.is_enabled("insights_skills"));
        let read = flags.begin_read();
        assert!(flags.finish_read(read, None));
        assert!(flags.snapshot().is_none());
        assert!(!flags.is_enabled("insights_skills"));
    }

    /// A replaced daemon leaves nothing held, as the Mac's `featureFlags =
    /// nil` on a new daemon, and an answer to a read made before it is
    /// dropped (the Mac's `featureFlagClient === client` guard).
    #[test]
    fn a_replaced_daemon_holds_nothing_and_drops_the_old_read() {
        let flags = FeatureFlags::default();
        let mut on = Snapshot::decode(recorded()).unwrap();
        on.flags.insert(
            "insights_skills".into(),
            serde_json::from_value(serde_json::json!({"enabled":true,"pending":false,"capability_blocked":false,"timing":"live"})).unwrap(),
        );
        let first = flags.begin_read();
        assert!(flags.finish_read(first, Some(on.clone())));
        assert!(flags.is_enabled("insights_skills"));
        let in_flight = flags.begin_read();
        flags.forget();
        assert!(
            !flags.is_enabled("insights_skills"),
            "the old daemon's snapshot is still held"
        );
        assert!(
            !flags.finish_read(in_flight, Some(on)),
            "a read to the old daemon was applied"
        );
        assert!(flags.snapshot().is_none());
    }

    #[test]
    fn successors_follow_the_macs_order() {
        let old = Snapshot::decode(recorded()).unwrap();
        let later = Snapshot {
            generation: old.generation + 1,
            ..old.clone()
        };
        let same = old.clone();
        let earlier = Snapshot {
            generation: old.generation - 1,
            ..old.clone()
        };
        let restarted = Snapshot {
            instance_id: uuid::Uuid::new_v4().to_string(),
            generation: 1,
            ..old.clone()
        };
        let other_digest = Snapshot {
            profile_digest: "0".repeat(64),
            generation: old.generation + 1,
            ..old.clone()
        };
        let other_environment = Snapshot {
            environment: if old.environment == Environment::Testing {
                Environment::Staging
            } else {
                Environment::Testing
            },
            generation: old.generation + 1,
            ..old.clone()
        };
        assert!(old.accepts_successor(&later));
        assert!(!old.accepts_successor(&same));
        assert!(!old.accepts_successor(&earlier));
        assert!(
            old.accepts_successor(&restarted),
            "a restarted daemon starts over"
        );
        assert!(!old.accepts_successor(&other_digest));
        assert!(!old.accepts_successor(&other_environment));

        // The store keeps the newer one and refuses the older.
        let flags = FeatureFlags::default();
        let read = flags.begin_read();
        assert!(flags.finish_read(read, Some(later.clone())));
        let read = flags.begin_read();
        assert!(!flags.finish_read(read, Some(earlier)));
        assert_eq!(flags.snapshot(), Some(later));
    }

    #[test]
    fn a_stale_read_changes_nothing() {
        let flags = FeatureFlags::default();
        let first = flags.begin_read();
        let second = flags.begin_read();
        assert!(!flags.finish_read(first, Snapshot::decode(recorded())));
        assert!(flags.snapshot().is_none());
        assert!(flags.finish_read(second, Snapshot::decode(recorded())));
        assert!(flags.snapshot().is_some());
    }

    #[test]
    fn a_snapshot_about_another_profile_is_not_read() {
        let snapshot = Snapshot::decode(recorded()).unwrap();
        let identity = identity_of(&snapshot);
        assert_eq!(
            read_answer(Ok(recorded()), Some(&identity)),
            Some(snapshot.clone())
        );
        let other_digest = BuildProfileIdentity {
            profile_digest: "0".repeat(64),
            ..identity.clone()
        };
        assert_eq!(read_answer(Ok(recorded()), Some(&other_digest)), None);
        let other_environment = BuildProfileIdentity {
            environment: if snapshot.environment == Environment::Production {
                Environment::Staging
            } else {
                Environment::Production
            },
            ..identity.clone()
        };
        assert_eq!(read_answer(Ok(recorded()), Some(&other_environment)), None);
        let invalid = BuildProfileIdentity {
            schema_version: 2,
            ..identity.clone()
        };
        assert_eq!(read_answer(Ok(recorded()), Some(&invalid)), None);
        assert_eq!(
            read_answer(Ok(recorded()), None),
            None,
            "an unreadable identity"
        );
        assert_eq!(
            read_answer(Err("daemon-not-running".into()), Some(&identity)),
            None
        );

        // A mismatch clears what was held, as on the Mac.
        let flags = FeatureFlags::default();
        let read = flags.begin_read();
        flags.finish_read(read, read_answer(Ok(recorded()), Some(&identity)));
        assert!(flags.snapshot().is_some());
        let read = flags.begin_read();
        flags.finish_read(read, read_answer(Ok(recorded()), Some(&other_digest)));
        assert!(flags.snapshot().is_none());
    }

    #[test]
    fn identity_agrees_only_on_a_valid_identity_naming_both() {
        let identity = BuildProfileIdentity {
            schema_version: 1,
            environment: Environment::Production,
            profile_digest: "a".repeat(64),
        };
        let digest = "a".repeat(64);
        assert!(identity_agrees(
            &identity,
            Some("production"),
            Some(&digest)
        ));
        assert!(!identity_agrees(&identity, Some("staging"), Some(&digest)));
        assert!(!identity_agrees(
            &identity,
            Some("production"),
            Some(&"b".repeat(64))
        ));
        assert!(!identity_agrees(&identity, None, Some(&digest)));
        assert!(!identity_agrees(&identity, Some("production"), None));
        let short = BuildProfileIdentity {
            profile_digest: "a".into(),
            ..identity.clone()
        };
        assert!(!identity_agrees(&short, Some("production"), Some("a")));
    }

    #[test]
    fn this_builds_identity_is_readable_and_valid() {
        let identity = build_identity().expect("the embedded profile is readable");
        assert!(identity_agrees(
            &identity,
            Some(identity.environment.as_str()),
            Some(&identity.profile_digest)
        ));
    }
}
