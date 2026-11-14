use super::tests::{doc, fixtures, now};
use super::*;

#[test]
fn environment_variable_wins_without_overwriting_local_or_remote_state() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open_with_environment(
        r.clone(),
        p.clone(),
        dir.path(),
        now(),
        Some(r#"{"live":true}"#),
        true,
    )
    .unwrap();
    rt.set_local_override("live", Some(false), now()).unwrap();
    rt.accept(&doc(1, false), now()).unwrap();
    assert!(rt.is_enabled("live"));
    assert!(!rt.snapshot().local_overrides["live"]);
    assert!(rt.snapshot().environment_variable_overrides["live"]);
    rt.set_local_override("live", None, now()).unwrap();
    assert!(
        rt.is_enabled("live"),
        "clearing the file does not clear process environment"
    );
    let restarted = Runtime::open(r, p, dir.path(), now()).unwrap();
    assert!(
        !restarted.is_enabled("live"),
        "environment choices are not persisted"
    );
    assert!(
        restarted
            .snapshot()
            .environment_variable_overrides
            .is_empty()
    );
}

#[test]
fn invalid_environment_variables_apply_no_partial_enables() {
    let dir = tempfile::tempdir().unwrap();
    let (mut r, p) = fixtures();
    let mut privacy = r.flags["live"].clone();
    privacy.class = FlagClass::PrivacySensitive;
    r.flags.insert("privacy".into(), privacy);
    for raw in [
        "",
        "not-json",
        "[]",
        r#"{"live":"true"}"#,
        r#"{"live":false,"live":true}"#,
        r#"{"live":true,"unknown":true}"#,
        r#"{"live":true,"blocked":true}"#,
        r#"{"live":true,"privacy":false}"#,
    ] {
        let rt = Runtime::open_with_environment(
            r.clone(),
            p.clone(),
            dir.path(),
            now(),
            Some(raw),
            true,
        )
        .unwrap();
        assert!(
            !rt.is_enabled("live"),
            "invalid environment value applied a partial enable"
        );
        assert!(rt.snapshot().environment_variable_overrides.is_empty());
        assert!(rt.snapshot().environment_variable_error.is_some());
    }
    let oversized = " ".repeat(65_537);
    let rt =
        Runtime::open_with_environment(r, p, dir.path(), now(), Some(&oversized), true).unwrap();
    assert!(rt.snapshot().environment_variable_error.is_some());
}

#[test]
fn environment_variable_choice_is_applied_before_next_start_latching() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open_with_environment(
        r,
        p,
        dir.path(),
        now(),
        Some(r#"{"restart":true,"live":false}"#),
        true,
    )
    .unwrap();
    assert!(rt.is_enabled("restart"));
    rt.set_local_override("restart", Some(false), now())
        .unwrap();
    assert!(rt.snapshot().flags["restart"].pending);
    assert!(rt.is_enabled("restart"));
    rt.accept(&doc(1, true), now()).unwrap();
    assert!(!rt.is_enabled("live"));
}

#[test]
fn manually_edited_config_is_loaded_on_restart() {
    let dir = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), dir.path(), now()).unwrap();
    rt.set_local_override("live", Some(true), now()).unwrap();
    let path = local::LocalOverrides::new(dir.path(), &p.digest().unwrap(), p.environment);
    let mut config: serde_json::Value =
        serde_json::from_slice(&std::fs::read(path.path()).unwrap()).unwrap();
    config["flags"]["live"] = serde_json::json!(false);
    std::fs::write(path.path(), serde_json::to_vec(&config).unwrap()).unwrap();
    assert!(
        rt.is_enabled("live"),
        "manual config edits are startup-only"
    );
    let restarted = Runtime::open(r, p, dir.path(), now()).unwrap();
    assert!(!restarted.is_enabled("live"));
    assert!(!restarted.snapshot().local_overrides["live"]);
}
