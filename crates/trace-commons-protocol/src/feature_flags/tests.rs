use super::*;
use chrono::{Duration, TimeZone};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}
fn raw() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"environment":"staging","revision":1,"issued_at":now(),"expires_at":now()+Duration::hours(1),"flags":{"example":true}})
}
fn parse(value: serde_json::Value) -> Result<Manifest, FlagError> {
    parse_manifest(
        &serde_json::to_vec(&value).unwrap(),
        Environment::Staging,
        now(),
    )
}
#[test]
fn manifest_rejects_invalid_contract() {
    assert!(parse(raw()).is_ok());
    for (key, value) in [
        ("schema_version", serde_json::json!(2)),
        ("environment", serde_json::json!("production")),
        ("flags", serde_json::json!({"example":"true"})),
        ("issued_at", serde_json::json!(now() + Duration::seconds(1))),
        ("expires_at", serde_json::json!(now())),
        ("expires_at", serde_json::json!(now() + Duration::days(8))),
    ] {
        let mut doc = raw();
        doc[key] = value;
        assert!(parse(doc).is_err(), "{key}");
    }
    let bytes = serde_json::to_string(&raw()).unwrap();
    let duplicate = bytes.replacen("\"revision\":1", "\"revision\":1,\"revision\":2", 1);
    assert!(parse_manifest(duplicate.as_bytes(), Environment::Staging, now()).is_err());
    let duplicate = bytes.replace("\"example\":true", "\"example\":true,\"example\":false");
    assert!(parse_manifest(duplicate.as_bytes(), Environment::Staging, now()).is_err());
    assert!(
        parse_manifest(
            &vec![b' '; MAX_MANIFEST_BYTES + 1],
            Environment::Staging,
            now()
        )
        .is_err()
    );
}
fn fixture() -> (Registry, Profile) {
    let registry:Registry=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"flags":{"example":{"description":"Fixture","owner":"tests","default":false,"required_capabilities":["example-v1"],"timing":"live"},"blocked":{"description":"Fixture","owner":"tests","default":false,"required_capabilities":["missing"],"timing":"next_start"}}})).unwrap();
    let profile:Profile=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"environment":"staging","capabilities":["example-v1"],"remote_enable":["example","blocked"]})).unwrap();
    (registry, profile)
}
#[test]
fn resolution_is_a_complete_snapshot() {
    let (registry, profile) = fixture();
    registry.validate(&profile).unwrap();
    let mut doc = raw();
    doc["flags"] = serde_json::json!({"example":true,"blocked":true,"future":true});
    let manifest = parse(doc).unwrap();
    let result = evaluate(&registry, &profile, Some(&manifest), now());
    assert!(result["example"].enabled);
    assert!(!result["blocked"].enabled);
    assert!(result["blocked"].capability_blocked);
    assert!(!result.contains_key("future"));
    let mut doc = raw();
    doc["flags"] = serde_json::json!({});
    assert!(!evaluate(&registry, &profile, Some(&parse(doc).unwrap()), now())["example"].enabled);
    assert!(
        !evaluate(
            &registry,
            &profile,
            Some(&manifest),
            now() + Duration::hours(2)
        )["example"]
            .enabled
    );
    let mut profile = profile;
    profile.remote_enable.clear();
    assert!(!evaluate(&registry, &profile, Some(&manifest), now())["example"].enabled);
}
#[test]
fn profiles_are_validated_and_digest_changes_with_capabilities() {
    let (registry, mut profile) = fixture();
    let digest = profile.digest().unwrap();
    profile.capabilities.insert("another".into());
    assert_ne!(profile.digest().unwrap(), digest);
    profile.remote_enable.insert("unknown".into());
    assert!(registry.validate(&profile).is_err());
    profile.remote_enable.remove("unknown");
    profile.registry_version = 2;
    assert!(registry.validate(&profile).is_err());
}
