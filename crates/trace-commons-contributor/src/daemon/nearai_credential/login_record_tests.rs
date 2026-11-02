use super::*;

fn store() -> (tempfile::TempDir, ConfigStore) {
    crate::config::tests_support::temp_store()
}

fn login() -> &'static str {
    crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0]
}

#[test]
fn nothing_is_remembered_at_first() {
    let (_dir, store) = store();
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(commons_login(&store), None);
}

#[test]
fn the_stored_login_is_read_only_for_the_sign_in_it_was_written_for() {
    let (_dir, store) = store();
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
    let (_dir, store) = store();
    record_stored_login(&store, login(), "key-1");
    accepted_by_commons(&store, Some(login()));
    clear_stored_login(&store);
    assert_eq!(stored_login(&store, "key-1"), None);
    assert_eq!(commons_login(&store), Some(login()));
}

#[test]
fn an_unknown_login_changes_nothing_it_should_not() {
    let (_dir, store) = store();
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
    let (_dir, store) = store();
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
    let (_dir, store) = store();
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

#[test]
fn logging_out_and_unenrolling_remove_the_record() {
    let (_dir, store) = store();
    accepted_by_commons(&store, Some(login()));
    store.remove_enrollment_files().unwrap();
    assert_eq!(commons_login(&store), None);

    accepted_by_commons(&store, Some(login()));
    store.wipe().unwrap();
    assert_eq!(commons_login(&store), None);
}
