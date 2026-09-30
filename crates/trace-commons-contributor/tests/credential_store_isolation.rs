//! Guard: a test run must never write the user's real OS credential store.
//!
//! Integration tests link this crate WITHOUT `cfg(test)`, so before this guard
//! every test that generated a device identity wrote a real login-keychain
//! item under the production service `trace-commons.account.credentials`
//! (account `cloud-v1-<uuid>`) and never deleted it: about 76 per run of this
//! crate's integration tests, 5,579 on one developer machine by 2026-09-27.
//!
//! Test builds now compile the `test-credential-store` feature (through this
//! crate's dev-dependency on itself), which routes both credential stores to
//! a file store inside the test's own config directory. This file fails if
//! that wiring is lost, and on macOS it asks the login keychain directly --
//! read-only, one exact account name -- whether the item a test just created
//! reached it.

use std::path::Path;

use trace_commons_contributor::config::ConfigStore;
use trace_commons_contributor::identity::DeviceIdentity;

/// Production service names. A test build must never create an item under
/// either.
const PRODUCTION_SERVICES: [&str; 2] = [
    "trace-commons.account.credentials",
    "trace-commons.near-ai.credentials",
];

/// The storage key the device-key record points at (`cloud-v1-<simple uuid>`),
/// read from the record file the credential store leaves in the config dir.
fn device_key_storage_key(store: &ConfigStore) -> String {
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.device_key_path()).unwrap()).unwrap();
    let reference = &record["reference"];
    let version = reference["version"].as_u64().expect("reference version");
    let id: uuid::Uuid = reference["id"]
        .as_str()
        .expect("reference id")
        .parse()
        .unwrap();
    format!("cloud-v{version}-{}", id.simple())
}

/// Read-only: `security find-generic-password` with an exact service and
/// account, no `-g`/`-w`, so it never reads or prints a secret and never
/// prompts. Exit 0 means the item exists.
#[cfg(target_os = "macos")]
fn in_login_keychain(service: &str, account: &str) -> bool {
    std::process::Command::new("/usr/bin/security")
        .args(["find-generic-password", "-s", service, "-a", account])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .expect("running /usr/bin/security")
        .success()
}

fn assert_not_in_os_store(_account: &str) {
    #[cfg(target_os = "macos")]
    for service in PRODUCTION_SERVICES {
        assert!(
            !in_login_keychain(service, _account),
            "a test wrote the real login keychain: service {service}, account {_account}"
        );
    }
    #[cfg(not(target_os = "macos"))]
    let _ = PRODUCTION_SERVICES;
}

fn assert_in_test_store(dir: &Path, account: &str) {
    assert!(
        dir.join("test-credential-store").join(account).is_file(),
        "the credential did not land in the test store under {}",
        dir.display()
    );
}

#[test]
fn a_device_key_generated_by_a_test_stays_out_of_the_os_credential_store() {
    let dir = tempfile::tempdir().unwrap();
    let store = ConfigStore::open(dir.path().join("state")).unwrap();
    let device = DeviceIdentity::load_or_generate(&store).unwrap();

    let account = device_key_storage_key(&store);
    assert_not_in_os_store(&account);
    assert_in_test_store(store.dir(), &account);

    // And it round-trips through the same store, so the fake is not simply
    // dropping writes on the floor.
    let reloaded = DeviceIdentity::load_or_generate(&store).unwrap();
    assert_eq!(reloaded.device_key_id, device.device_key_id);
}
