use super::*;
use chrono::{Duration, TimeZone};
pub(super) fn now() -> DateTime<Utc> {
    Utc.timestamp_opt(1_800_000_000, 0).unwrap()
}
pub(super) fn fixtures() -> (Registry, Profile) {
    let registry=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"flags":{"live":{"description":"Test","owner":"tests","default":false,"required_capabilities":["v1"],"timing":"live"},"restart":{"description":"Test","owner":"tests","default":false,"required_capabilities":[],"timing":"next_start"},"blocked":{"description":"Test","owner":"tests","default":false,"required_capabilities":["v2"],"timing":"live"}}})).unwrap();
    let profile=serde_json::from_value(serde_json::json!({"schema_version":1,"registry_version":1,"environment":"staging","capabilities":["v1"],"remote_enable":["live","restart","blocked"]})).unwrap();
    (registry, profile)
}
pub(super) fn doc(revision: u64, on: bool) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({"schema_version":1,"environment":"staging","revision":revision,"issued_at":now(),"expires_at":now()+Duration::hours(1),"flags":{"live":on,"restart":on,"blocked":on}})).unwrap()
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
