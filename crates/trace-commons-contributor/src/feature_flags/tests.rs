use super::*;
use chrono::{Duration, TimeZone};

#[test]
fn skills_is_live_default_off_and_remotely_supported_in_each_environment() {
    let registry = registry().unwrap();
    let definition = &registry.flags["insights_skills"];
    assert!(!definition.default);
    assert_eq!(definition.class, FlagClass::Product);
    assert_eq!(definition.timing, Timing::Live);
    for env in [
        Environment::Production,
        Environment::Staging,
        Environment::Testing,
    ] {
        // The shipped profile validates, which needs a pinned signing key.
        let shipped = profile_for_environment(env).unwrap();
        registry.validate(&shipped).unwrap();
        assert!(shipped.remote_enable.contains("insights_skills"));
        // Run it with the test key, which signs the fixture manifests.
        let profile = super::test_signing::pin(shipped);
        let dir = tempfile::tempdir().unwrap();
        let runtime = Runtime::open(registry.clone(), profile.clone(), dir.path(), now()).unwrap();
        assert!(!runtime.snapshot().is_enabled("insights_skills"));
        let manifest = |revision, enabled| {
            sign(
                &serde_json::to_vec(&serde_json::json!({
                    "schema_version":1,"environment":env,"revision":revision,
                    "issued_at":now(),"expires_at":now()+Duration::hours(1),
                    "flags":{"insights_skills":enabled}
                }))
                .unwrap(),
            )
        };
        runtime.accept(&manifest(1, true), now()).unwrap();
        assert!(runtime.snapshot().is_enabled("insights_skills"));
        runtime.accept(&manifest(2, false), now()).unwrap();
        assert!(!runtime.snapshot().is_enabled("insights_skills"));
        let mut unsupported = profile;
        unsupported.capabilities.clear();
        // Its own state directory: the rollback watermark is per environment,
        // so revision 1 here would be refused after revision 2 above.
        let blocked_dir = tempfile::tempdir().unwrap();
        let blocked =
            Runtime::open(registry.clone(), unsupported, blocked_dir.path(), now()).unwrap();
        blocked.accept(&manifest(1, true), now()).unwrap();
        assert!(!blocked.snapshot().is_enabled("insights_skills"));
        assert!(blocked.snapshot().flags["insights_skills"].capability_blocked);
    }
}
pub(super) fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}
pub(super) fn fixtures() -> (Registry, Profile) {
    let registry=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"flags":{"live":{"description":"Test","owner":"tests","class":"product","default":false,"required_capabilities":["v1"],"timing":"live"},"restart":{"description":"Test","owner":"tests","class":"product","default":false,"required_capabilities":[],"timing":"next_start"},"blocked":{"description":"Test","owner":"tests","class":"product","default":false,"required_capabilities":["v2"],"timing":"live"}}})).unwrap();
    let profile=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"environment":"staging","capabilities":["v1"],"remote_enable":["live","restart","blocked"],"signing_keys":[]})).unwrap();
    (registry, super::test_signing::pin(profile))
}
pub(super) fn sign(payload: &[u8]) -> Vec<u8> {
    super::test_signing::sign(payload)
}
pub(super) fn doc_until(revision: u64, on: bool, expires_at: DateTime<Utc>) -> Vec<u8> {
    sign(&serde_json::to_vec(&serde_json::json!({"schema_version":1,"environment":"staging","revision":revision,"issued_at":now(),"expires_at":expires_at,"flags":{"live":on,"restart":on,"blocked":on}})).unwrap())
}
pub(super) fn doc(revision: u64, on: bool) -> Vec<u8> {
    doc_until(revision, on, now() + Duration::hours(1))
}
#[test]
fn cache_preserves_revision_across_expiry_and_restart() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(10, true), now()).unwrap();
    assert!(rt.accept(&doc(9, false), now()).is_err());
    assert!(rt.accept(&doc(10, false), now()).is_err());
    assert!(rt.accept(&doc(10, true), now()).is_ok());
    let rt = Runtime::open(r, p, temp.path(), now() + Duration::hours(2)).unwrap();
    assert!(!rt.snapshot().flags["live"].enabled);
    assert!(rt.accept(&doc(9, false), now()).is_err());
    assert!(rt.accept(&doc(11, false), now()).is_ok());
}
#[test]
fn live_expiry_and_restart_latching() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(1, true), now()).unwrap();
    let s = rt.snapshot();
    assert!(s.flags["live"].enabled);
    assert!(!s.flags["restart"].enabled);
    assert!(s.flags["restart"].pending);
    assert!(!s.flags["blocked"].enabled);
    let rt = Runtime::open(r, p, temp.path(), now() + Duration::seconds(1)).unwrap();
    assert!(rt.snapshot().flags["restart"].enabled);
    rt.tick(now() + Duration::hours(2));
    assert!(!rt.snapshot().flags["live"].enabled);
    assert!(rt.snapshot().flags["restart"].enabled);
    assert!(!rt.snapshot().flags["restart"].pending);
    rt.tick(now());
    assert!(
        !rt.snapshot().flags["live"].enabled,
        "backwards clock must not resurrect expired enable"
    );
}
#[test]
fn invalid_response_preserves_cache_and_failed_write_preserves_state() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(1, true), now()).unwrap();
    assert!(rt.accept(b"{}", now()).is_err());
    assert!(rt.snapshot().flags["live"].enabled);
    let cached = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    assert!(cached.snapshot().flags["live"].enabled);
    let file = temp.path().join("file");
    std::fs::write(&file, b"x").unwrap();
    let rt = Runtime::open(r, p, &file, now()).unwrap();
    assert!(rt.accept(&doc(1, true), now()).is_err());
    assert!(!rt.snapshot().flags["live"].enabled);
}
#[test]
fn cache_is_profile_scoped_and_profile_selection_is_channel_bound() {
    assert!(select_environment(Some("testing"), "production").is_err());
    assert!(select_environment(Some("bogus"), "staging").is_err());
    assert_eq!(
        select_environment(Some("testing"), "staging").unwrap(),
        Environment::Testing
    );
    assert_eq!(
        select_environment(None, "production").unwrap(),
        Environment::Production
    );
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(1, true), now()).unwrap();
    let mut other = p;
    other.capabilities.insert("different-build".into());
    let rt = Runtime::open(r, other, temp.path(), now()).unwrap();
    assert!(!rt.snapshot().flags["live"].enabled);
}
/// The rollback watermark is environment-scoped: a release that changes the
/// build profile drops cached enables, but never the highest revision seen.
#[test]
fn rollback_watermark_survives_a_profile_change() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(10, true), now()).unwrap();
    let mut other = p;
    other.capabilities.insert("different-build".into());
    let rt = Runtime::open(r, other, temp.path(), now()).unwrap();
    assert!(
        !rt.snapshot().flags["live"].enabled,
        "enables stay profile-scoped"
    );
    assert!(
        rt.accept(&doc(9, true), now()).is_err(),
        "an older revision is refused"
    );
    assert!(
        rt.accept(&doc(10, false), now()).is_err(),
        "the same revision with other content is refused"
    );
    rt.accept(&doc(10, true), now()).unwrap();
    assert!(rt.snapshot().flags["live"].enabled);
    rt.accept(&doc(11, false), now()).unwrap();
}
#[test]
fn caches_for_other_profiles_are_removed() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r.clone(), p.clone(), temp.path(), now()).unwrap();
    rt.accept(&doc(1, true), now()).unwrap();
    let stale = temp
        .path()
        .join(format!("feature-flags-{}.json", p.digest().unwrap()));
    assert!(stale.exists());
    let unrelated = temp.path().join("feature-flags-notes.json");
    std::fs::write(&unrelated, b"keep").unwrap();
    let mut other = p;
    other.capabilities.insert("different-build".into());
    let _rt = Runtime::open(r, other, temp.path(), now()).unwrap();
    assert!(!stale.exists(), "the old profile's cache was left behind");
    assert!(unrelated.exists(), "only profile caches are removed");
}
#[test]
fn unsigned_or_foreign_signed_manifests_are_refused() {
    let temp = tempfile::tempdir().unwrap();
    let (r, p) = fixtures();
    let rt = Runtime::open(r, p, temp.path(), now()).unwrap();
    let payload = serde_json::to_vec(&serde_json::json!({"schema_version":1,"environment":"staging","revision":1,"issued_at":now(),"expires_at":now()+Duration::hours(1),"flags":{"live":true}})).unwrap();
    assert!(rt.accept(&payload, now()).is_err());
    let foreign = ring::signature::Ed25519KeyPair::from_seed_unchecked(&[8; 32]).unwrap();
    let forged = assemble_signed_manifest(
        String::from_utf8(payload.clone()).unwrap(),
        vec![ManifestSignature {
            key_id: {
                use ring::signature::KeyPair as _;
                key_id_for(&foreign.public_key().as_ref().try_into().unwrap())
            },
            signature: hex::encode(foreign.sign(&signing_input(&payload)).as_ref()),
        }],
    );
    assert!(rt.accept(&forged, now()).is_err());
    assert!(!rt.snapshot().flags["live"].enabled);
    rt.accept(&sign(&payload), now()).unwrap();
    assert!(rt.snapshot().flags["live"].enabled);
}
