use super::tests::{doc, fixtures, now};
use super::*;
use chrono::Duration;

#[test]
fn local_choices_override_refresh_and_clear_to_current_environment() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    rt.set_local_override("live", Some(true), now()).unwrap();
    rt.accept(&doc(1, false), now()).unwrap();
    assert!(rt.is_enabled("live"));
    rt.set_local_override("live", None, now()).unwrap();
    assert!(!rt.is_enabled("live"));
    rt.accept(&doc(2, true), now()).unwrap();
    rt.set_local_override("live", Some(false), now()).unwrap();
    assert!(!rt.is_enabled("live"));
    rt.set_local_override("live", None, now()).unwrap();
    assert!(rt.is_enabled("live"));
    assert!(rt.snapshot().local_overrides.is_empty());
    rt.set_local_override("live", Some(true), now()).unwrap();
    rt.tick(now() + Duration::hours(2));
    assert!(
        rt.is_enabled("live"),
        "local enable does not expire with remote"
    );
    rt.set_local_override("live", None, now() + Duration::hours(2))
        .unwrap();
    assert!(!rt.is_enabled("live"), "clear uses expired remote defaults");
    assert!(
        rt.accept(&doc(1, false), now()).is_err(),
        "local edits preserve watermark"
    );
}

#[test]
fn local_choices_persist_in_all_environments_and_do_not_cross_profiles() {
    for env in [
        Environment::Production,
        Environment::Staging,
        Environment::Testing,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let r = registry().unwrap();
        let p = profile_for_environment(env).unwrap();
        let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
        rt.set_local_override("insights_skills", Some(true), now())
            .unwrap();
        let restarted = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
        assert!(restarted.is_enabled("insights_skills"));
        assert!(restarted.snapshot().local_overrides["insights_skills"]);
        assert_eq!(
            restarted.snapshot().revision,
            None,
            "local changes never mint remote revisions"
        );
        let mut changed = p;
        changed.capabilities.insert("new-profile".into());
        let changed = Runtime::open(r, changed, dir.path(), now()).unwrap();
        assert!(!changed.is_enabled("insights_skills"));
        assert!(changed.snapshot().local_overrides.is_empty());
    }
}

#[test]
fn local_choice_changes_are_observable_even_when_effective_value_is_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    let start = rt.generation();
    let selected = rt.set_local_override("live", Some(false), now()).unwrap();
    assert!(!selected.is_enabled("live"));
    assert!(selected.generation > start);
    assert!(!selected.local_overrides["live"]);
    assert_eq!(
        rt.set_local_override("live", Some(false), now())
            .unwrap()
            .generation,
        selected.generation
    );
    let cleared = rt.set_local_override("live", None, now()).unwrap();
    assert!(cleared.generation > selected.generation);
    assert!(cleared.local_overrides.is_empty());
}

#[test]
fn local_choices_cannot_enable_unsupported_or_privacy_sensitive_flags() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, p) = fixtures();
    let mut privacy = r.flags["live"].clone();
    privacy.class = FlagClass::PrivacySensitive;
    r.flags.insert("privacy".into(), privacy);
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    let before = rt.snapshot();
    for id in ["unknown", "blocked", "privacy"] {
        for value in [Some(true), Some(false), None] {
            assert!(rt.set_local_override(id, value, now()).is_err());
            assert_eq!(rt.snapshot(), before);
        }
    }
    assert!(before.local_override_allowed["live"]);
    assert!(!before.local_override_allowed["blocked"]);
    assert!(!before.local_override_allowed["privacy"]);
}

#[test]
fn local_next_start_changes_latch_only_when_daemon_restarts() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
    let pending = rt.set_local_override("restart", Some(true), now()).unwrap();
    assert!(!pending.flags["restart"].enabled);
    assert!(pending.flags["restart"].pending);
    let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
    assert!(rt.is_enabled("restart"));
    let pending = rt.set_local_override("restart", None, now()).unwrap();
    assert!(pending.flags["restart"].enabled);
    assert!(!pending.flags["restart"].pending);
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    assert!(!rt.is_enabled("restart"));
}

#[test]
fn failed_local_persistence_keeps_the_entire_snapshot_unchanged() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, b"x").unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r, p, &file, now()).unwrap();
    let before = rt.snapshot();
    assert!(rt.set_local_override("live", Some(true), now()).is_err());
    assert_eq!(rt.snapshot(), before);
}

#[test]
fn corrupt_wrong_profile_unknown_and_duplicate_local_data_cannot_enable() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let local = local::LocalOverrides::new(dir.path(), &p.digest().unwrap(), p.environment);
    let good = serde_json::json!({"schema_version":1,"environment":"staging", "profile_digest":p.digest().unwrap(),"flags":{"live":true}});
    let mut invalids = vec![b"not-json".to_vec(), vec![b' '; 65_537]];
    for field in ["schema_version", "environment", "profile_digest", "flags"] {
        let mut bad = good.clone();
        bad[field] = match field {
            "schema_version" => serde_json::json!(2),
            "environment" => serde_json::json!("production"),
            "profile_digest" => serde_json::json!("different"),
            _ => serde_json::json!({"unknown":true,"live":true}),
        };
        invalids.push(serde_json::to_vec(&bad).unwrap());
    }
    invalids.push(
        serde_json::to_string(&good)
            .unwrap()
            .replace("\"live\":true", "\"live\":false,\"live\":true")
            .into_bytes(),
    );
    for bad in invalids {
        std::fs::write(local.path(), bad).unwrap();
        let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
        assert!(!rt.is_enabled("live"));
        assert!(rt.snapshot().local_overrides.is_empty());
    }
}

#[cfg(unix)]
#[test]
fn local_override_file_is_private_and_separate_from_signed_cache() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let local = local::LocalOverrides::new(dir.path(), &p.digest().unwrap(), p.environment);
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    rt.set_local_override("live", Some(true), now()).unwrap();
    assert_eq!(
        std::fs::metadata(local.path())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
}

#[cfg(unix)]
#[test]
fn non_regular_local_files_are_refused_without_blocking_startup() {
    use std::os::unix::fs::symlink;
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let local = local::LocalOverrides::new(dir.path(), &p.digest().unwrap(), p.environment);
    let target = dir.path().join("target.json");
    let good = serde_json::json!({"schema_version":1,"environment":"staging", "profile_digest":p.digest().unwrap(),"flags":{"live":true}});
    std::fs::write(&target, serde_json::to_vec(&good).unwrap()).unwrap();
    symlink(&target, local.path()).unwrap();
    assert!(
        !Runtime::open(r.clone(), p.clone(), dir.path(), now())
            .unwrap()
            .is_enabled("live")
    );
    std::fs::remove_file(local.path()).unwrap();
    assert!(
        std::process::Command::new("mkfifo")
            .arg(local.path())
            .status()
            .unwrap()
            .success()
    );
    assert!(
        !Runtime::open(r.clone(), p.clone(), dir.path(), now())
            .unwrap()
            .is_enabled("live")
    );
    std::fs::remove_file(local.path()).unwrap();
    std::fs::create_dir(local.path()).unwrap();
    assert!(
        !Runtime::open(r, p, dir.path(), now())
            .unwrap()
            .is_enabled("live")
    );
}

/// A hand-edited file with one bad entry enables nothing, and a later `set`
/// must not replace it: that would drop every other choice in the file.
#[test]
fn an_invalid_local_file_is_kept_and_set_refuses_until_it_is_fixed() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let local = local::LocalOverrides::new(dir.path(), &p.digest().unwrap(), p.environment);
    let bad = serde_json::to_vec(
        &serde_json::json!({"schema_version":1,"environment":"staging",
        "profile_digest":p.digest().unwrap(),"flags":{"live":true,"mistyped":true}}),
    )
    .unwrap();
    std::fs::write(local.path(), &bad).unwrap();
    let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
    assert!(!rt.is_enabled("live"));
    assert!(rt.local_overrides_unreadable());
    let error = rt
        .set_local_override("live", Some(true), now())
        .unwrap_err();
    assert_eq!(error.to_string(), LOCAL_OVERRIDES_INVALID);
    assert_eq!(std::fs::read(local.path()).unwrap(), bad);
    // Fixed by hand, the next start reads it and `set` works again.
    std::fs::remove_file(local.path()).unwrap();
    let rt = Runtime::open(r, p, dir.path(), now()).unwrap();
    assert!(!rt.local_overrides_unreadable());
    rt.set_local_override("live", Some(true), now()).unwrap();
    assert!(rt.is_enabled("live"));
}

/// A status view with no daemon running, from a build whose profile
/// differs, must not delete the running build's persisted choices.
#[test]
fn a_read_only_open_does_not_prune_other_profiles_choices() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let other = dir
        .path()
        .join(format!("feature-flags-local-{}.json", "a".repeat(64)));
    std::fs::write(&other, b"{}").unwrap();
    let _view =
        Runtime::open_with_environment(r.clone(), p.clone(), dir.path(), now(), None, false)
            .unwrap();
    assert!(other.exists(), "a read-only open deletes nothing");
    let _daemon = Runtime::open(r, p, dir.path(), now()).unwrap();
    assert!(!other.exists(), "the daemon still prunes other profiles");
}
