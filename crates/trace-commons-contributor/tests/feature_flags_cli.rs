use std::process::Command;
use trace_commons_contributor::feature_flags;

#[test]
fn language_selection_rollout_is_live_and_limited_to_preview_profiles() {
    use feature_flags::{Environment, Runtime};
    let key = "localization_language_selection";
    let now = chrono::Utc::now();
    for (environment, eligible) in [
        (Environment::Production, false),
        (Environment::Staging, true),
        (Environment::Testing, true),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(
            feature_flags::registry().unwrap(),
            feature_flags::profile_for_environment(environment).unwrap(),
            directory.path(),
            now,
        )
        .unwrap();
        assert!(!runtime.snapshot().is_enabled(key));
        let manifest = |revision, enabled| {
            serde_json::to_vec(&serde_json::json!({
                "schema_version": 1, "environment": environment, "revision": revision,
                "issued_at": now, "expires_at": now + chrono::Duration::hours(1),
                "flags": {key: enabled}
            }))
            .unwrap()
        };
        runtime.accept(&manifest(1, true), now).unwrap();
        assert_eq!(runtime.snapshot().is_enabled(key), eligible);
        runtime.accept(&manifest(2, false), now).unwrap();
        assert!(!runtime.snapshot().is_enabled(key));
        runtime.accept(&manifest(3, true), now).unwrap();
        assert_eq!(runtime.snapshot().is_enabled(key), eligible);
        runtime.tick(now + chrono::Duration::hours(2));
        assert!(!runtime.snapshot().is_enabled(key));
    }
}

#[test]
fn build_profile_is_the_packaged_identity_not_a_runtime_override() {
    let result = Command::new(env!("CARGO_BIN_EXE_near-ai"))
        .args(["feature-flags", "build-profile", "--json"])
        .env("TC_FEATURE_PROFILE", "not-a-runtime-setting")
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let actual: feature_flags::BuildProfileIdentity =
        serde_json::from_slice(&result.stdout).unwrap();
    assert_eq!(actual, feature_flags::build_profile_identity().unwrap());
}

#[test]
fn local_validation_rejects_unknown_flags_without_a_daemon() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("manifest.json");
    let identity = feature_flags::build_profile_identity().unwrap();
    let now = chrono::Utc::now();
    let mut document = serde_json::json!({"schema_version":1,"environment":identity.environment,"revision":1,"issued_at":now,"expires_at":now+chrono::Duration::hours(1),"flags":{}});
    let check = || {
        Command::new(env!("CARGO_BIN_EXE_near-ai"))
            .args(["feature-flags", "validate", "--file"])
            .arg(&path)
            .args(["--environment", identity.environment.as_str()])
            .output()
            .unwrap()
    };
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(check().status.success());
    document["flags"] = serde_json::json!({"unregistered-feature-fixture":true});
    std::fs::write(&path, serde_json::to_vec(&document).unwrap()).unwrap();
    assert!(!check().status.success());
}
