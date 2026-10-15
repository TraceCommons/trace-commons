use trace_commons_contributor::managed::secrets::ApiKey;

#[test]
fn api_key_debug_is_redacted_and_control_bytes_are_refused() {
    let key = ApiKey::new("fixture-key-never-log".into()).unwrap();
    assert_eq!(format!("{key:?}"), "ApiKey([redacted])");
    for invalid in ["", "secret\nsecond-command", "secret\0tail", "secret\r"] {
        assert!(ApiKey::new(invalid.into()).is_err());
    }
}
