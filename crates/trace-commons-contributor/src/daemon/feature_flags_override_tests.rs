use super::*;

fn override_params(shared: &DaemonShared, value: serde_json::Value) -> serde_json::Value {
    let snapshot = shared.feature_flags.snapshot();
    serde_json::json!({"id":"insights_skills", "value":value, "environment":snapshot.environment,"profile_digest":snapshot.profile_digest})
}

#[test]
fn feature_override_mutates_daemon_persists_and_publishes() {
    let (dir, store) = temp_store();
    let shared = DaemonShared::load(store.clone()).unwrap();
    let mut events = shared.events.subscribe();
    let reply = handle_request(&shared, &req("feature_flags_override_set", override_params(&shared, serde_json::json!(true))));
    assert!(reply.error.is_none(), "{:?}", reply.error);
    let result = reply.result.unwrap();
    assert_eq!(result["flags"]["insights_skills"]["enabled"], true);
    assert_eq!(result["local_overrides"]["insights_skills"], true);
    let event = events.try_recv().expect("change notification");
    assert_eq!(event.event, EVENT_FEATURE_FLAGS_CHANGED);
    assert_eq!(event.data["generation"], result["generation"]);
    assert!(DaemonShared::load(store).unwrap().feature_flags.is_enabled("insights_skills"));
    let reply = handle_request(&shared, &req("feature_flags_override_set", override_params(&shared, serde_json::Value::Null)));
    assert_eq!(reply.result.unwrap()["flags"]["insights_skills"]["enabled"], false);
    drop(dir);
}

#[test]
fn feature_override_rejects_missing_value_invalid_shape_and_wrong_identity_before_mutation() {
    let shared = shared();
    let before = shared.feature_flags.snapshot();
    let good = override_params(&shared, serde_json::json!(true));
    let mut cases = vec![];
    for field in ["id", "value", "environment", "profile_digest"] {
        let mut missing = good.clone();
        missing.as_object_mut().unwrap().remove(field);
        cases.push(missing);
    }
    for (field, value) in [
        ("value", serde_json::json!("on")),
        ("id", serde_json::json!("unknown")),
        ("environment", serde_json::json!("wrong-environment")),
        ("profile_digest", serde_json::json!("wrong-digest")),
        ("extra", serde_json::json!(true)),
    ] {
        let mut wrong = good.clone();
        wrong[field] = value;
        cases.push(wrong);
    }
    let mut events = shared.events.subscribe();
    for params in cases {
        let reply = handle_request(&shared, &req("feature_flags_override_set", params));
        assert!(reply.error.is_some());
        assert_eq!(shared.feature_flags.snapshot(), before);
        assert!(events.try_recv().is_err());
    }
}
