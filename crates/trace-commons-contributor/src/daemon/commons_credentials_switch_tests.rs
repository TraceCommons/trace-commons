use super::*;
use crate::identity::DeviceIdentity;

fn legacy_config(device_key_id: &str) -> ContributorConfig {
    serde_json::from_value(serde_json::json!({
        "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
        "issuer_url": "https://issuer.example",
        "ingest_url": "https://ingest.example",
        "audience": "trace-commons-upload",
        "tenant_id": "tenant-legacy",
        "instance_id": "",
        "user_subject": "invitee",
        "device_key_id": device_key_id,
        "consent_scopes": ["debugging_evaluation"]
    }))
    .unwrap()
}

fn session(token: &str) -> Vec<u8> {
    serde_json::to_vec(&crate::account_auth::AccountSession {
        access_token: token.into(),
        account_id: "fixture-account".into(),
        expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
    })
    .unwrap()
}

/// An enrolled legacy invitee: config, device key and an account session.
fn enrolled() -> (
    tempfile::TempDir,
    ConfigStore,
    DeviceIdentity,
    ContributorConfig,
) {
    let (dir, store) = crate::config::tests_support::temp_store();
    let legacy = DeviceIdentity::load_or_generate(&store).unwrap();
    let cfg = legacy_config(&legacy.device_key_id);
    store.save_config(&cfg).unwrap();
    store
        .write_daemon_file(ACCOUNT_SESSION_FILE, &session("tcn1_legacy"))
        .unwrap();
    (dir, store, legacy, cfg)
}

fn switched_config(cfg: &ContributorConfig, staged: &DeviceIdentity) -> ContributorConfig {
    let mut next = cfg.clone();
    next.tenant_id = format!("nearai-{}", "3c".repeat(32));
    next.user_subject = staged.device_key_id.clone();
    next.device_key_id = staged.device_key_id.clone();
    next
}

fn live_device(store: &ConfigStore) -> String {
    DeviceIdentity::load(store).unwrap().unwrap().device_key_id
}

fn live_token(store: &ConfigStore) -> Option<String> {
    crate::account_auth::try_load_token(store).unwrap()
}

/// The legacy identity, exactly as it was before any switch.
fn assert_legacy_intact(store: &ConfigStore, legacy: &DeviceIdentity, cfg: &ContributorConfig) {
    let on_disk = store.load_config().unwrap().unwrap();
    assert_eq!(on_disk.tenant_id, cfg.tenant_id);
    assert_eq!(on_disk.device_key_id, legacy.device_key_id);
    assert_eq!(live_device(store), legacy.device_key_id);
    assert_eq!(live_token(store).as_deref(), Some("tcn1_legacy"));
    assert!(pending_switch(store).unwrap().is_none(), "no journal left");
}

#[test]
fn a_staged_key_sits_beside_the_live_one_and_never_replaces_it() {
    let (_dir, store, legacy, _cfg) = enrolled();
    let staged = stage_device_key(&store).unwrap();
    assert_ne!(staged.device_key_id, legacy.device_key_id);
    assert_eq!(
        live_device(&store),
        legacy.device_key_id,
        "live key unchanged"
    );
    assert_eq!(
        load_staged_device_key(&store)
            .unwrap()
            .unwrap()
            .device_key_id,
        staged.device_key_id
    );
    // Staging again replaces the staged key, still leaving the live one.
    let again = stage_device_key(&store).unwrap();
    assert_ne!(again.device_key_id, staged.device_key_id);
    assert_eq!(live_device(&store), legacy.device_key_id);
    // The staged entry is bound to its own domain: pointing the live slot at
    // it is refused rather than read as a device key.
    let staged_record = std::fs::read(Kind::StagedDevice.file(&store)).unwrap();
    std::fs::write(Kind::Device.file(&store), &staged_record).unwrap();
    assert!(DeviceIdentity::load(&store).is_err());
}

#[test]
fn discarding_the_staged_key_leaves_the_live_identity_and_its_session() {
    let (_dir, store, legacy, cfg) = enrolled();
    stage_device_key(&store).unwrap();
    discard_staged_device_key(&store).unwrap();
    assert!(load_staged_device_key(&store).unwrap().is_none());
    assert_legacy_intact(&store, &legacy, &cfg);
}

#[test]
fn logout_removes_the_staged_key_too() {
    let (_dir, store, _legacy, _cfg) = enrolled();
    stage_device_key(&store).unwrap();
    store.wipe().unwrap();
    assert!(load_staged_device_key(&store).unwrap().is_none());
    assert!(!Kind::StagedDevice.file(&store).exists());
}

#[test]
fn a_switch_moves_config_key_and_session_together_and_retires_the_legacy_key_last() {
    let (_dir, store, legacy, cfg) = enrolled();
    let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
    let staged = stage_device_key(&store).unwrap();
    let next = switched_config(&cfg, &staged);
    switch_identity(
        &store,
        &legacy_snapshot,
        &next,
        &session("tcn1_nearai"),
        serde_json::json!({"k": 1}),
    )
    .unwrap();

    assert_eq!(
        store.load_config().unwrap().unwrap().tenant_id,
        next.tenant_id
    );
    assert_eq!(live_device(&store), staged.device_key_id);
    assert_eq!(live_token(&store).as_deref(), Some("tcn1_nearai"));
    assert!(load_staged_device_key(&store).unwrap().is_none());
    let pending = pending_switch(&store).unwrap().expect("journal kept");
    assert_eq!(pending.phase, SwitchPhase::Switching);
    assert_eq!(pending.context, serde_json::json!({"k": 1}));

    // Not retirable before the caller's own step has committed.
    assert!(retire_legacy(&store).is_err());
    let old_device_record = {
        // The journal holds the legacy record; recover it for the check below.
        let journal = read_switch_journal(&store).unwrap().unwrap();
        journal.old_device
    };
    mark_switch_committed(&store).unwrap();
    retire_legacy(&store).unwrap();
    assert!(pending_switch(&store).unwrap().is_none());
    // The legacy entry is gone from the OS store: pointing the device slot
    // back at it no longer loads a key.
    std::fs::write(Kind::Device.file(&store), old_device_record).unwrap();
    assert!(DeviceIdentity::load(&store).is_err());
    let _ = legacy;
}

#[test]
fn a_failure_at_any_step_leaves_the_legacy_identity_fully_working() {
    for step in 1..=4u8 {
        let (_dir, store, legacy, cfg) = enrolled();
        let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
        let staged = stage_device_key(&store).unwrap();
        let next = switched_config(&cfg, &staged);
        fail_switch_after_for_test(&store, step);
        let error = switch_identity(
            &store,
            &legacy_snapshot,
            &next,
            &session("tcn1_nearai"),
            serde_json::Value::Null,
        )
        .unwrap_err();
        assert_eq!(
            error.to_string(),
            "identity_switch_injected_failure",
            "step {step}"
        );
        assert_legacy_intact(&store, &legacy, &cfg);
    }
}

#[test]
fn an_uncommitted_switch_rolls_back_to_the_legacy_identity() {
    let (_dir, store, legacy, cfg) = enrolled();
    let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
    let staged = stage_device_key(&store).unwrap();
    let next = switched_config(&cfg, &staged);
    switch_identity(
        &store,
        &legacy_snapshot,
        &next,
        &session("tcn1_nearai"),
        serde_json::json!("ctx"),
    )
    .unwrap();
    assert_eq!(
        roll_back_switch(&store).unwrap(),
        Some(serde_json::json!("ctx"))
    );
    assert_legacy_intact(&store, &legacy, &cfg);
    assert!(roll_back_switch(&store).unwrap().is_none(), "idempotent");
}

#[test]
fn a_committed_switch_is_not_rolled_back() {
    let (_dir, store, _legacy, cfg) = enrolled();
    let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
    let staged = stage_device_key(&store).unwrap();
    let next = switched_config(&cfg, &staged);
    switch_identity(
        &store,
        &legacy_snapshot,
        &next,
        &session("tcn1_nearai"),
        serde_json::Value::Null,
    )
    .unwrap();
    mark_switch_committed(&store).unwrap();
    assert!(roll_back_switch(&store).is_err());
    assert_eq!(live_device(&store), staged.device_key_id);
}

/// The logout guard: a sign-out after the migration began discards the
/// legacy key, and the switch refuses rather than passing through it.
#[test]
fn a_logout_after_the_migration_began_refuses_the_switch() {
    let (_dir, store, _legacy, cfg) = enrolled();
    let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
    let staged = stage_device_key(&store).unwrap();
    let next = switched_config(&cfg, &staged);
    store.wipe().unwrap();
    assert!(
        switch_identity(
            &store,
            &legacy_snapshot,
            &next,
            &session("tcn1_nearai"),
            serde_json::Value::Null,
        )
        .is_err()
    );
    assert!(store.load_config().unwrap().is_none(), "nothing enrolled");
    assert!(DeviceIdentity::load(&store).unwrap().is_none());
    assert!(pending_switch(&store).unwrap().is_none());
}

/// A logout and a fresh enrollment in between is not the same identity
/// either, even though a device key is present again.
#[test]
fn a_re_enrollment_after_the_migration_began_refuses_the_switch() {
    let (_dir, store, _legacy, cfg) = enrolled();
    let legacy_snapshot = snapshot(&store, Kind::Device).unwrap();
    let staged = stage_device_key(&store).unwrap();
    let next = switched_config(&cfg, &staged);
    store.wipe().unwrap();
    let other = DeviceIdentity::load_or_generate(&store).unwrap();
    store
        .save_config(&legacy_config(&other.device_key_id))
        .unwrap();
    let staged_again = stage_device_key(&store).unwrap();
    let mut next_again = next.clone();
    next_again.device_key_id = staged_again.device_key_id.clone();
    assert!(
        switch_identity(
            &store,
            &legacy_snapshot,
            &next_again,
            &session("tcn1_nearai"),
            serde_json::Value::Null,
        )
        .is_err()
    );
    assert_eq!(live_device(&store), other.device_key_id);
}
