// The tests here are macOS-only; elsewhere their shared imports and helpers
// are unused, which -D warnings turns into a build failure (Linux and Windows CI).
#![cfg_attr(not(target_os = "macos"), allow(unused_imports, dead_code))]
use super::*;
use crate::config::ConfigStore;
use crate::daemon::ipc::Request;
use crate::daemon::managed::{handle, red_team_available};
use crate::managed::sessions::{LaunchPurpose, LaunchRequest};
use crate::managed::{AccountView, ConnectionKind, Generation, RED_TEAM_FLAG, ToolId, folder};
use serde_json::json;
use uuid::Uuid;

#[test]
fn sandbox_settings_on_a_claude_launch_are_refused() {
    let settings = SandboxSettings::default();
    let err = validate(ToolId::Claude, Some(&settings)).unwrap_err();
    assert_eq!(err, ManagedError::InvalidRequest);
    assert_eq!(validate(ToolId::Claude, None).unwrap(), None);
}

#[test]
fn a_missing_input_is_refused_by_name() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist-file.txt");
    let settings = SandboxSettings {
        inputs: vec![missing.clone()],
        ..Default::default()
    };
    let err = validate(ToolId::RedTeam, Some(&settings)).unwrap_err();
    match err {
        ManagedError::SandboxInputRefused(msg) => {
            assert!(
                msg.contains("does-not-exist-file.txt"),
                "error must name the offending path: {msg}"
            );
        }
        other => panic!("expected SandboxInputRefused, got {other:?}"),
    }
}

#[cfg(unix)]
#[test]
fn a_symlinked_input_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let real = dir.path().join("real.txt");
    std::fs::write(&real, b"hello").unwrap();
    let link = dir.path().join("link.txt");
    std::os::unix::fs::symlink(&real, &link).unwrap();

    let settings = SandboxSettings {
        inputs: vec![link],
        ..Default::default()
    };
    let err = validate(ToolId::RedTeam, Some(&settings)).unwrap_err();
    match err {
        ManagedError::SandboxInputRefused(msg) => {
            assert!(
                msg.contains("link.txt"),
                "error must name the symlink path: {msg}"
            );
        }
        other => panic!("expected SandboxInputRefused, got {other:?}"),
    }
}

#[test]
fn the_home_folder_is_refused_as_an_input() {
    let fake_home = tempfile::tempdir().unwrap();
    let settings = SandboxSettings {
        inputs: vec![fake_home.path().to_path_buf()],
        ..Default::default()
    };
    let err =
        validate_with_home(ToolId::RedTeam, Some(&settings), Some(fake_home.path())).unwrap_err();
    match err {
        ManagedError::SandboxInputRefused(msg) => {
            assert!(
                msg.contains("home"),
                "error must explain home directory refusal: {msg}"
            );
        }
        other => panic!("expected SandboxInputRefused, got {other:?}"),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn a_loopback_host_is_refused() {
    let settings = SandboxSettings {
        allow_hosts: vec!["127.0.0.1:8080".to_owned()],
        ..Default::default()
    };
    let err = validate(ToolId::RedTeam, Some(&settings)).unwrap_err();
    match err {
        ManagedError::SandboxHostRefused(msg) => {
            assert!(
                msg.contains("127.0.0.1:8080"),
                "error must name the offending host entry: {msg}"
            );
        }
        other => panic!("expected SandboxHostRefused, got {other:?}"),
    }
}

#[cfg(target_os = "macos")]
#[test]
fn packages_adds_the_preset() {
    let settings = SandboxSettings {
        packages: true,
        ..Default::default()
    };
    let validated = validate(ToolId::RedTeam, Some(&settings))
        .unwrap()
        .expect("validated settings");
    assert!(
        validated.allow_hosts.iter().any(|h| h == "pypi.org:443"),
        "packages preset must add pypi.org:443: {:?}",
        validated.allow_hosts
    );
    assert!(
        validated
            .allow_hosts
            .iter()
            .any(|h| h == "registry.npmjs.org:443"),
        "packages preset must add registry.npmjs.org:443: {:?}",
        validated.allow_hosts
    );
    assert!(
        validated.allow_hosts.iter().any(|h| h == "crates.io:443"),
        "packages preset must add crates.io:443: {:?}",
        validated.allow_hosts
    );
}

#[cfg(target_os = "macos")]
#[test]
fn inputs_are_remembered_per_folder_and_hosts_are_not() {
    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let input_file = dir.path().join("sample.txt");
    std::fs::write(&input_file, b"data").unwrap();
    let canonical_input = std::fs::canonicalize(&input_file).unwrap();

    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let mut shared = crate::daemon::ipc::DaemonShared::load(store).unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        crate::feature_flags::registry().unwrap(),
        crate::feature_flags::test_signing::testing_profile(),
        shared.store.dir(),
        chrono::Utc::now(),
    )
    .unwrap();
    let now = chrono::Utc::now();
    let manifest = json!({
        "schema_version": 1,
        "environment": shared.feature_flags.snapshot().environment,
        "revision": 999,
        "issued_at": now,
        "expires_at": now + chrono::Duration::hours(1),
        "flags": { RED_TEAM_FLAG: true }
    });
    shared
        .feature_flags
        .accept(
            &crate::feature_flags::test_signing::sign(&serde_json::to_vec(&manifest).unwrap()),
            now,
        )
        .unwrap();
    assert!(red_team_available(&shared));

    let added = handle(
        &shared,
        &Request {
            id: 1,
            method: "managed_account_add".into(),
            params: json!({
                "tool": "red_team",
                "connection": "near_ai",
                "label": "NEAR AI"
            }),
        },
    );
    let account: AccountView = serde_json::from_value(added.result.unwrap()).unwrap();
    let _ = handle(
        &shared,
        &Request {
            id: 2,
            method: "managed_account_set_key".into(),
            params: json!({ "account_id": account.id, "key": "sk-near-remember-test" }),
        },
    );

    let prepared = handle(
        &shared,
        &Request {
            id: 3,
            method: "managed_launch_prepare".into(),
            params: json!(LaunchRequest {
                request_id: Uuid::new_v4(),
                tool: ToolId::RedTeam,
                connection: ConnectionKind::NearAi,
                account_id: account.id,
                cwd: project.clone(),
                expected_generation: Generation(0),
                save_default: false,
                collect_traces: true,
                purpose: LaunchPurpose::Coding,
                model: None,
                background_model: None,
                keep_saved_model: false,
                cli_options: None,
                resume_of: None,
                sandbox: Some(SandboxSettings {
                    inputs: vec![input_file],
                    output_dir: None,
                    allow_hosts: vec!["example.com:443".to_owned()],
                    packages: false,
                    cpus: None,
                    memory_mib: None,
                }),
                initial_prompt: None,
            }),
        },
    );
    assert!(prepared.error.is_none(), "{prepared:?}");

    let snapshot = folder::lookup(&shared.store, ToolId::RedTeam, &project).unwrap();
    assert!(snapshot.remembered);
    assert_eq!(snapshot.red_team_inputs, vec![canonical_input]);

    // Verify stored settings JSON does not contain the host entry anywhere.
    let settings_json =
        serde_json::to_string(&shared.settings.lock().unwrap().managed_folder_settings).unwrap();
    assert!(
        !settings_json.contains("example.com"),
        "hosts must never be remembered in folder settings: {settings_json}"
    );
}

#[test]
fn output_dir_must_be_an_existing_non_symlink_directory() {
    let dir = tempfile::tempdir().unwrap();
    let out = dir.path().join("results");
    std::fs::create_dir_all(&out).unwrap();
    let canonical = std::fs::canonicalize(&out).unwrap();

    let valid = SandboxSettings {
        output_dir: Some(out.clone()),
        ..Default::default()
    };
    let checked = validate_with_home(ToolId::RedTeam, Some(&valid), Some(dir.path()))
        .unwrap()
        .unwrap();
    assert_eq!(checked.output_dir, Some(canonical));

    // A regular file is refused.
    let file = dir.path().join("not-a-dir.txt");
    std::fs::write(&file, b"x").unwrap();
    let bad_file = SandboxSettings {
        output_dir: Some(file),
        ..Default::default()
    };
    assert!(matches!(
        validate_with_home(ToolId::RedTeam, Some(&bad_file), Some(dir.path())),
        Err(ManagedError::SandboxInputRefused(_))
    ));

    // Home directory itself is refused.
    let bad_home = SandboxSettings {
        output_dir: Some(dir.path().to_path_buf()),
        ..Default::default()
    };
    assert!(matches!(
        validate_with_home(ToolId::RedTeam, Some(&bad_home), Some(dir.path())),
        Err(ManagedError::SandboxInputRefused(_))
    ));

    // Filesystem root is refused.
    let bad_root = SandboxSettings {
        output_dir: Some(PathBuf::from("/")),
        ..Default::default()
    };
    assert!(matches!(
        validate_with_home(ToolId::RedTeam, Some(&bad_root), Some(dir.path())),
        Err(ManagedError::SandboxInputRefused(_))
    ));
}
