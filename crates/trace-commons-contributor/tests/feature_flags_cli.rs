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
fn local_set_without_daemon_refuses_without_creating_state() {
    let directory = tempfile::tempdir().unwrap();
    let absent = directory.path().join("absent-state");
    let result = Command::new(env!("CARGO_BIN_EXE_near-ai"))
        .arg("--config-dir")
        .arg(&absent)
        .args(["--json", "feature-flags", "set", "insights_skills", "on"])
        .output()
        .unwrap();
    assert!(!result.status.success());
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert!(text.contains("daemon-unreachable"), "{text}");
    assert!(!absent.exists(), "a failed mutation must not create state");
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn local_set_cli_changes_real_daemon_and_survives_restart() {
    use trace_commons_contributor::{config::ConfigStore, daemon::start_embedded};
    async fn set(dir: &std::path::Path, value: &'static str) -> feature_flags::Snapshot {
        let dir = dir.to_owned();
        tokio::task::spawn_blocking(move || {
            let result = Command::new(env!("CARGO_BIN_EXE_near-ai"))
                .arg("--config-dir")
                .arg(dir)
                .args(["--json", "feature-flags", "set", "insights_skills", value])
                // A CLI caller's environment does not reconfigure the running daemon.
                .env(
                    feature_flags::ENVIRONMENT_OVERRIDE_VARIABLE,
                    r#"{"insights_skills":false}"#,
                )
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            serde_json::from_slice(&result.stdout).unwrap()
        })
        .await
        .unwrap()
    }
    let dir = tempfile::tempdir().unwrap();
    let daemon = start_embedded(ConfigStore::open(dir.path().to_owned()).unwrap())
        .await
        .unwrap();
    let on = set(dir.path(), "on").await;
    assert!(on.is_enabled("insights_skills"));
    assert!(
        daemon
            .shared
            .feature_flags
            .snapshot()
            .is_enabled("insights_skills")
    );
    daemon.close();
    let daemon = start_embedded(ConfigStore::open(dir.path().to_owned()).unwrap())
        .await
        .unwrap();
    assert!(
        daemon
            .shared
            .feature_flags
            .snapshot()
            .is_enabled("insights_skills")
    );
    let off = set(dir.path(), "off").await;
    assert!(!off.is_enabled("insights_skills"));
    assert!(!off.local_overrides["insights_skills"]);
    let clear = set(dir.path(), "default").await;
    assert!(!clear.is_enabled("insights_skills"));
    assert!(clear.local_overrides.is_empty());
    daemon.close();
    let daemon = start_embedded(ConfigStore::open(dir.path().to_owned()).unwrap())
        .await
        .unwrap();
    assert!(
        !daemon
            .shared
            .feature_flags
            .snapshot()
            .is_enabled("insights_skills")
    );
    assert!(
        daemon
            .shared
            .feature_flags
            .snapshot()
            .local_overrides
            .is_empty()
    );
    daemon.close();
}

#[test]
fn process_environment_is_read_by_runtime_and_reported_without_persistence() {
    let dir = tempfile::tempdir().unwrap();
    for (raw, enabled, invalid) in [
        (r#"{"insights_skills":true}"#, true, false),
        (r#"{"insights_skills":false}"#, false, false),
        (r#"{"insights_skills":true,"unknown":true}"#, false, true),
        ("{}", false, false),
    ] {
        let result = Command::new(env!("CARGO_BIN_EXE_near-ai"))
            .arg("--config-dir")
            .arg(dir.path())
            .args(["--json", "feature-flags"])
            .env(feature_flags::ENVIRONMENT_OVERRIDE_VARIABLE, raw)
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let snapshot: feature_flags::Snapshot = serde_json::from_slice(&result.stdout).unwrap();
        assert_eq!(snapshot.is_enabled("insights_skills"), enabled);
        assert_eq!(snapshot.environment_variable_error.is_some(), invalid);
        assert!(snapshot.local_overrides.is_empty());
        assert_eq!(
            std::fs::read_dir(dir.path()).unwrap().count(),
            0,
            "environment inspection must never persist its choices"
        );
    }
}
