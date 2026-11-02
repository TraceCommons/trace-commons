use super::*;

fn store() -> (tempfile::TempDir, ConfigStore) {
    crate::config::tests_support::temp_store()
}

#[test]
fn nothing_is_remembered_at_first() {
    let (_dir, store) = store();
    assert_eq!(stored_login(&store), None);
    assert_eq!(commons_login(&store), None);
}

#[test]
fn the_stored_login_is_not_the_accounts_until_the_commons_accepts_it() {
    let (_dir, store) = store();
    let login = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0];
    record_stored_login(&store, login);
    assert_eq!(stored_login(&store), Some(login));
    assert_eq!(commons_login(&store), None);
    accepted_by_commons(&store, stored_login(&store));
    assert_eq!(commons_login(&store), Some(login));
}

#[test]
fn an_unknown_login_changes_nothing() {
    let (_dir, store) = store();
    let login = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0];
    accepted_by_commons(&store, Some(login));
    accepted_by_commons(&store, None);
    accepted_by_commons(&store, Some("bitbucket"));
    record_stored_login(&store, "bitbucket");
    assert_eq!(commons_login(&store), Some(login));
    assert_eq!(stored_login(&store), None);
}

#[test]
fn a_tag_this_build_does_not_offer_reads_as_none() {
    let (_dir, store) = store();
    store
        .write_daemon_file(
            LOGIN_RECORD_FILE,
            br#"{"stored_login":"bitbucket","commons_login":"<script>"}"#,
        )
        .unwrap();
    assert_eq!(stored_login(&store), None);
    assert_eq!(commons_login(&store), None);
    // And an unreadable file is an empty record, not an error.
    store
        .write_daemon_file(LOGIN_RECORD_FILE, b"not json")
        .unwrap();
    assert_eq!(commons_login(&store), None);
}

#[test]
fn unenrolling_forgets_the_accounts_login_and_keeps_the_stored_one() {
    let (_dir, store) = store();
    let login = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0];
    record_stored_login(&store, login);
    accepted_by_commons(&store, Some(login));
    forget_commons_login(&store);
    assert_eq!(commons_login(&store), None);
    assert_eq!(stored_login(&store), Some(login));
}

#[test]
fn the_record_holds_tags_only() {
    let (_dir, store) = store();
    let login = crate::channel::NEAR_AI_SIGN_IN_PROVIDERS[0];
    record_stored_login(&store, login);
    accepted_by_commons(&store, Some(login));
    let bytes = store.read_daemon_file(LOGIN_RECORD_FILE).unwrap().unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(
        value,
        serde_json::json!({ "stored_login": login, "commons_login": login })
    );
}
