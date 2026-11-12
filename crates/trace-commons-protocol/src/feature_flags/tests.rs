use super::*;
use chrono::{Duration, TimeZone};
use ring::signature::{Ed25519KeyPair, KeyPair as _};

fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}
/// A throwaway key for this test run only. Never a real manifest key.
fn key() -> Ed25519KeyPair {
    let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
}
fn public_hex(key: &Ed25519KeyPair) -> String {
    hex::encode(key.public_key().as_ref())
}
fn sign(key: &Ed25519KeyPair, payload: &str) -> Vec<u8> {
    let public: [u8; 32] = key.public_key().as_ref().try_into().unwrap();
    assemble_signed_manifest(
        payload.into(),
        vec![ManifestSignature {
            key_id: key_id_for(&public),
            signature: hex::encode(key.sign(&signing_input(payload.as_bytes())).as_ref()),
        }],
    )
}
fn raw() -> serde_json::Value {
    serde_json::json!({"schema_version":1,"environment":"staging","revision":1,"issued_at":now(),"expires_at":now()+Duration::hours(1),"flags":{"example":true}})
}
fn profile_with(key: &Ed25519KeyPair) -> Profile {
    let mut profile = fixture().1;
    profile.signing_keys.insert(public_hex(key));
    profile
}
fn parse_with(key: &Ed25519KeyPair, value: serde_json::Value) -> Result<Manifest, FlagError> {
    parse_manifest(
        &sign(key, &serde_json::to_string(&value).unwrap()),
        &profile_with(key),
        now(),
    )
}
fn parse(value: serde_json::Value) -> Result<Manifest, FlagError> {
    parse_with(&key(), value)
}
#[test]
fn manifest_rejects_invalid_contract() {
    assert!(parse(raw()).is_ok());
    for (key, value) in [
        ("schema_version", serde_json::json!(2)),
        ("environment", serde_json::json!("production")),
        ("flags", serde_json::json!({"example":"true"})),
        (
            "issued_at",
            serde_json::json!(now() + Duration::seconds(MAX_CLOCK_SKEW_SECONDS + 1)),
        ),
        ("expires_at", serde_json::json!(now())),
        ("expires_at", serde_json::json!(now() + Duration::days(8))),
    ] {
        let mut doc = raw();
        doc[key] = value;
        assert!(parse(doc).is_err(), "{key}");
    }
    let bytes = serde_json::to_string(&raw()).unwrap();
    let k = key();
    let p = profile_with(&k);
    let duplicate = bytes.replacen("\"revision\":1", "\"revision\":1,\"revision\":2", 1);
    assert!(parse_manifest(&sign(&k, &duplicate), &p, now()).is_err());
    let duplicate = bytes.replace("\"example\":true", "\"example\":true,\"example\":false");
    assert!(parse_manifest(&sign(&k, &duplicate), &p, now()).is_err());
    assert!(parse_manifest(&vec![b' '; MAX_MANIFEST_BYTES + 1], &p, now()).is_err());
}
/// A manifest issued by a clock slightly ahead of the client's is current,
/// so a disable is not held back by skew.
#[test]
fn issued_at_tolerates_bounded_clock_skew() {
    let mut doc = raw();
    doc["issued_at"] = serde_json::json!(now() + Duration::seconds(MAX_CLOCK_SKEW_SECONDS));
    assert!(parse(doc).is_ok());
}
#[test]
fn only_a_signature_from_a_pinned_key_is_accepted() {
    let trusted = key();
    let other = key();
    let payload = serde_json::to_string(&raw()).unwrap();
    let profile = profile_with(&trusted);
    assert!(parse_manifest(&sign(&trusted, &payload), &profile, now()).is_ok());
    // Signed by a key the build does not pin.
    assert!(parse_manifest(&sign(&other, &payload), &profile, now()).is_err());
    // A bare, unsigned manifest.
    assert!(parse_manifest(payload.as_bytes(), &profile, now()).is_err());
    // The payload changed after signing.
    let signed = sign(&trusted, &payload);
    let tampered = String::from_utf8(signed)
        .unwrap()
        .replace("\\\"example\\\":true", "\\\"example\\\":false");
    assert!(parse_manifest(tampered.as_bytes(), &profile, now()).is_err());
    // A signature over the payload without the domain prefix.
    let public: [u8; 32] = trusted.public_key().as_ref().try_into().unwrap();
    let undomained = assemble_signed_manifest(
        payload.clone(),
        vec![ManifestSignature {
            key_id: key_id_for(&public),
            signature: hex::encode(trusted.sign(payload.as_bytes()).as_ref()),
        }],
    );
    assert!(parse_manifest(&undomained, &profile, now()).is_err());
    // No pinned key at all refuses everything.
    assert!(parse_manifest(&sign(&trusted, &payload), &fixture().1, now()).is_err());
    // Too many signatures.
    let mut envelope: SignedManifest = serde_json::from_slice(&sign(&trusted, &payload)).unwrap();
    let one = envelope.signatures[0].clone();
    envelope.signatures = vec![one; MAX_MANIFEST_SIGNATURES + 1];
    assert!(parse_manifest(&serde_json::to_vec(&envelope).unwrap(), &profile, now()).is_err());
}
fn fixture() -> (Registry, Profile) {
    let registry:Registry=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"flags":{"example":{"description":"Fixture","owner":"tests","class":"product","default":false,"required_capabilities":["example-v1"],"timing":"live"},"blocked":{"description":"Fixture","owner":"tests","class":"product","default":false,"required_capabilities":["missing"],"timing":"next_start"}}})).unwrap();
    let profile:Profile=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"environment":"staging","capabilities":["example-v1"],"remote_enable":["example","blocked"],"signing_keys":[]})).unwrap();
    (registry, profile)
}
#[test]
fn resolution_is_a_complete_snapshot() {
    let (registry, _) = fixture();
    let k = key();
    let profile = profile_with(&k);
    registry.validate(&profile).unwrap();
    let mut doc = raw();
    doc["flags"] = serde_json::json!({"example":true,"blocked":true,"future":true});
    let manifest = parse_with(&k, doc).unwrap();
    let result = evaluate(&registry, &profile, Some(&manifest), now());
    assert!(result["example"].enabled);
    assert!(!result["blocked"].enabled);
    assert!(result["blocked"].capability_blocked);
    assert!(!result.contains_key("future"));
    let mut doc = raw();
    doc["flags"] = serde_json::json!({});
    assert!(
        !evaluate(
            &registry,
            &profile,
            Some(&parse_with(&k, doc).unwrap()),
            now()
        )["example"]
            .enabled
    );
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
    let (registry, _) = fixture();
    let mut profile = profile_with(&key());
    let digest = profile.digest().unwrap();
    profile.capabilities.insert("another".into());
    assert_ne!(profile.digest().unwrap(), digest);
    profile.remote_enable.insert("unknown".into());
    assert!(registry.validate(&profile).is_err());
    profile.remote_enable.remove("unknown");
    profile.registry_version = 2;
    assert!(registry.validate(&profile).is_err());
}
/// The guards a reviewer used to enforce by hand.
#[test]
fn validate_refuses_unsafe_remote_enables() {
    let (registry, _) = fixture();
    let profile = profile_with(&key());
    registry.validate(&profile).unwrap();
    // Remote enables without a pinned signing key.
    assert!(registry.validate(&fixture().1).is_err());
    // A malformed pinned key.
    let mut bad = profile.clone();
    bad.signing_keys.insert("not-a-key".into());
    assert!(registry.validate(&bad).is_err());
    // Privacy-sensitive behavior can never be remotely enabled.
    let mut sensitive = registry.clone();
    sensitive.flags.get_mut("example").unwrap().class = FlagClass::PrivacySensitive;
    assert!(sensitive.validate(&profile).is_err());
    let mut kept_local = profile.clone();
    kept_local.remote_enable.remove("example");
    sensitive.validate(&kept_local).unwrap();
    // A remotely enabled flag defaults off, so a failed fetch fails closed.
    let mut on = registry.clone();
    on.flags.get_mut("example").unwrap().default = true;
    assert!(on.validate(&profile).is_err());
    // A flag without a class does not parse.
    assert!(serde_json::from_value::<Definition>(serde_json::json!({"description":"x","owner":"x","default":false,"required_capabilities":[],"timing":"live"})).is_err());
}
