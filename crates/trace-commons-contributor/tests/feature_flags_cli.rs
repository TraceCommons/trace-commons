use std::process::Command;
use trace_commons_contributor::feature_flags;

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

#[test]
fn hermes_profiles_are_default_off_and_require_build_capability() {
    use feature_flags::{Environment, Runtime};
    for environment in [
        Environment::Production,
        Environment::Staging,
        Environment::Testing,
    ] {
        let home = tempfile::tempdir().unwrap();
        let registry = feature_flags::registry().unwrap();
        let profile = feature_flags::profile_for_environment(environment).unwrap();
        let now = chrono::Utc::now();
        let manifest = serde_json::to_vec(&serde_json::json!({"schema_version":1,
            "environment":environment,"revision":1,"issued_at":now,
            "expires_at":now+chrono::Duration::hours(1),"flags":{"managed_hermes":true}}))
        .unwrap();
        let runtime = Runtime::open(registry.clone(), profile.clone(), home.path(), now).unwrap();
        assert!(!runtime.snapshot().is_enabled("managed_hermes"));
        runtime.accept(&manifest, now).unwrap();
        assert!(runtime.snapshot().is_enabled("managed_hermes"));
        let mut incapable = profile;
        incapable.capabilities.remove("managed_hermes");
        let runtime = Runtime::open(registry, incapable, home.path(), now).unwrap();
        runtime.accept(&manifest, now).unwrap();
        assert!(!runtime.snapshot().is_enabled("managed_hermes"));
    }
}
