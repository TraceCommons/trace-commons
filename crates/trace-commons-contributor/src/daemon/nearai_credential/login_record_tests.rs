use super::*;

/// An enrolled device's store: the account's login is kept only on one.
fn enrolled() -> (tempfile::TempDir, ConfigStore) {
    let (dir, store) = crate::config::tests_support::temp_store();
    std::fs::write(store.dir().join(CONFIG_FILE), b"{}").unwrap();
    (dir, store)
}

fn login() -> &'static str {
    crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0]
}

#[test]
fn nothing_is_remembered_at_first() {
    let (_dir, store) = enrolled();
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(commons_login(&store), None);
}

#[test]
fn the_stored_login_is_read_only_for_the_sign_in_it_was_written_for() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    assert_eq!(stored_login(&store, "key-1"), Some(login()));
    // Another sign-in (another key) never reads this one's login: a tag that
    // was not rewritten for it is unknown, not wrong.
    assert_eq!(stored_login(&store, "key-2"), None);
    // And it is not the account's until the account accepts it.
    assert_eq!(commons_login(&store), None);
    accepted_by_commons(&store, stored_login(&store, "key-1"));
    assert_eq!(commons_login(&store), Some(login()));
}

#[test]
fn signing_out_forgets_the_stored_login_and_keeps_the_accounts() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    clear_stored_login(&store);
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(commons_login(&store), Some(login()));
}

#[test]
fn an_unknown_login_changes_nothing_it_should_not() {
    let (_dir, store) = enrolled();
    accepted_by_commons(&store, Some(login()));
    accepted_by_commons(&store, None);
    accepted_by_commons(&store, Some("bitbucket"));
    assert_eq!(commons_login(&store), Some(login()));
    // A stored login this build does not offer is recorded as none at all,
    // so it cannot leave an earlier sign-in's tag in place.
    record_stored_login(&store, login(), "key-1");
    record_stored_login(&store, "bitbucket", "key-2");
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(stored_login(&store, "key-2"), None);
}

#[test]
fn a_tag_this_build_does_not_offer_reads_as_none() {
    let (_dir, store) = enrolled();
    store
        .write_daemon_file(
            DAEMON_NEAR_AI_LOGIN_FILE,
            format!(
                r#"{{"stored":{{"provider":"bitbucket","sign_in":"{}"}},"commons_login":"<script>"}}"#,
                sign_in_digest("key-1")
            )
            .as_bytes(),
        )
        .unwrap();
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(commons_login(&store), None);
    // An unreadable file is an empty record, not an error.
    store
        .write_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE, b"not json")
        .unwrap();
    assert_eq!(commons_login(&store), None);
}

#[test]
fn the_record_holds_tags_and_a_digest_only() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    let bytes = store
        .read_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE)
        .unwrap()
        .unwrap();
    let text = String::from_utf8(bytes).unwrap();
    assert!(!text.contains("key-1"), "{text}");
    let value: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        value,
        serde_json::json!({
            "stored": { "provider": login(), "sign_in": sign_in_digest("key-1") },
            "commons_login": login(),
        })
    );
}

/// Forgetting the account's login (unenroll's step) leaves the stored
/// session's login alone: that half is cleared with the session itself
/// (`ceremony::forget_locked`, which unenroll's NEAR AI clear runs).
#[test]
fn unenrolling_forgets_the_accounts_login_and_keeps_the_sessions() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    store.remove_enrollment_files().unwrap();
    forget_commons_login(&store).unwrap();
    assert_eq!(commons_login(&store), None);
    assert_eq!(stored_login(&store, "key-1"), Some(login()));
}

/// Logging out removes the whole record, and any temp file a crashed write
/// left behind.
#[test]
fn logging_out_removes_the_record() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    let orphan = store
        .dir()
        .join(format!(".{DAEMON_NEAR_AI_LOGIN_FILE}.tmp-orphan"));
    std::fs::write(&orphan, b"{}").unwrap();
    store.wipe().unwrap();
    assert!(!store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());
    assert!(!orphan.exists());
    assert_eq!(commons_login(&store), None);
    assert_eq!(stored_login(&store, "key-1"), None);
}

/// An acceptance that lands after the device left its account -- an unenroll
/// or a logout that ran while it was in flight -- writes nothing, so the
/// record cannot come back with the old account's login.
#[test]
fn an_acceptance_after_leaving_the_account_writes_nothing() {
    let (_dir, store) = enrolled();
    store.wipe().unwrap();
    accepted_by_commons(&store, Some(login()));
    assert!(!store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());

    let (_dir, store) = enrolled();
    accepted_by_commons(&store, Some(login()));
    store.remove_enrollment_files().unwrap();
    forget_commons_login(&store).unwrap();
    accepted_by_commons(&store, Some(login()));
    assert_eq!(commons_login(&store), None);
}

/// Removal waits for a write already holding the record's lock, and that
/// write cannot recreate the file once removal has run.
#[test]
fn removal_is_ordered_against_a_write_in_flight() {
    let (_dir, store) = enrolled();
    accepted_by_commons(&store, Some(login()));
    let held = write_turn();
    let remover = std::thread::spawn({
        let store = store.clone();
        move || remove(&store).unwrap()
    });
    std::thread::sleep(std::time::Duration::from_millis(50));
    // Removal has not run while the lock is held.
    assert!(store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());
    drop(held);
    remover.join().unwrap();
    assert!(!store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());
}

/// Nothing to keep is never written: forgetting on a device with no record
/// creates none.
#[test]
fn an_empty_change_creates_no_file() {
    let (_dir, store) = enrolled();
    forget_commons_login(&store).unwrap();
    clear_stored_login(&store);
    assert!(!store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());
}

/// When the record cannot be rewritten, clearing the account's login falls
/// back to removing the record: the old login is gone either way.
#[test]
fn a_record_that_cannot_be_rewritten_is_removed_instead() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    forget_or_remove_with(&store, |_| Err(anyhow::anyhow!("unwritable"))).unwrap();
    assert!(!store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).exists());
    assert_eq!(commons_login(&store), None);
}

/// A change that changes nothing writes nothing: a record holding only the
/// stored session's login is left byte for byte as it was.
#[test]
fn a_no_op_change_does_not_rewrite_the_record() {
    let (_dir, store) = enrolled();
    let compact = format!(
        r#"{{"stored":{{"provider":"{}","sign_in":"{}"}}}}"#,
        login(),
        sign_in_digest("key-1")
    );
    store
        .write_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE, compact.as_bytes())
        .unwrap();
    forget_commons_login(&store).unwrap();
    let bytes = store
        .read_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE)
        .unwrap()
        .unwrap();
    assert_eq!(bytes, compact.as_bytes());
}

/// A panic while the record's lock was held does not make the record
/// unwritable for the rest of the daemon's life.
#[test]
fn a_poisoned_lock_still_lets_the_record_be_written() {
    let _ = std::thread::spawn(|| {
        let _held = WRITE.lock();
        panic!("poison the record's lock");
    })
    .join();
    let (_dir, store) = enrolled();
    accepted_by_commons(&store, Some(login()));
    assert_eq!(commons_login(&store), Some(login()));
    forget_commons_login(&store).unwrap();
    remove(&store).unwrap();
}

/// An unreadable record is never taken as already holding what a clear
/// would write: clearing it tries the write and reports the failure, so the
/// caller can fall back to removing it.
#[test]
fn clearing_an_unreadable_record_reports_it_rather_than_skipping() {
    let (_dir, store) = enrolled();
    // A directory where the record goes: it can be neither read nor written.
    std::fs::create_dir_all(store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE).join("blocked")).unwrap();
    assert!(forget_commons_login(&store).is_err());
    assert!(forget_commons_login_or_remove(&store).is_err());

    // One that exists but does not parse is rewritten, not skipped.
    let (_dir, store) = enrolled();
    store
        .write_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE, b"not json")
        .unwrap();
    forget_commons_login(&store).unwrap();
    let text = String::from_utf8(
        store
            .read_daemon_file(DAEMON_NEAR_AI_LOGIN_FILE)
            .unwrap()
            .unwrap(),
    )
    .unwrap();
    assert_ne!(text, "not json");
}

/// The account's login is shown only while the device is enrolled: a
/// leftover on an unenrolled device is never read back.
#[test]
fn the_accounts_login_reads_as_unknown_unless_enrolled() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    store
        .write_daemon_file(
            DAEMON_NEAR_AI_LOGIN_FILE,
            format!(r#"{{"commons_login":"{}"}}"#, login()).as_bytes(),
        )
        .unwrap();
    assert_eq!(commons_login(&store), None);
    std::fs::write(store.dir().join(CONFIG_FILE), b"{}").unwrap();
    assert_eq!(commons_login(&store), Some(login()));
}

/// A new enrollment's login replaces the record's account login, and an
/// unknown one clears it: an earlier account's login is never shown as the
/// new one's. The stored session's login is kept.
#[test]
fn a_new_enrollment_replaces_or_clears_the_accounts_login() {
    let (_dir, store) = enrolled();
    record_stored_login(&store, login(), "key-1");
    store
        .write_daemon_file(
            DAEMON_NEAR_AI_LOGIN_FILE,
            format!(
                r#"{{"stored":{{"provider":"{}","sign_in":"{}"}},"commons_login":"{}"}}"#,
                login(),
                sign_in_digest("key-1"),
                login()
            )
            .as_bytes(),
        )
        .unwrap();
    enrolled_with(&store, None);
    assert_eq!(commons_login(&store), None);
    assert_eq!(stored_login(&store, "key-1"), Some(login()));
    enrolled_with(&store, Some(login()));
    assert_eq!(commons_login(&store), Some(login()));
}

/// A read that failed says nothing about what is there -- it may be a valid
/// record -- so no change writes over it: the change fails instead, and only
/// a caller that must clear the record falls back to removing it.
#[test]
fn a_failed_read_is_never_written_over() {
    let (_dir, store) = enrolled();
    let path = store.daemon_path(DAEMON_NEAR_AI_LOGIN_FILE);
    // A directory where the record goes: reading it fails.
    std::fs::create_dir_all(path.join("kept")).unwrap();
    record_stored_login(&store, login(), "key-1");
    clear_stored_login(&store);
    accepted_by_commons(&store, Some(login()));
    assert!(path.join("kept").exists(), "nothing wrote over it");
    assert!(forget_commons_login(&store).is_err());
}
