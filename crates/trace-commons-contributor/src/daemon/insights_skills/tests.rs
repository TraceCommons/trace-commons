use super::*;

/// Explicit, read-only qualification against the catalog's real public sources.
/// The ordinary suite never contacts a publisher.
#[tokio::test]
#[ignore = "fetches the fixed public publisher catalog"]
async fn live_public_catalog_has_parseable_skills_and_valid_cache() {
    let client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .user_agent("TraceCommons-Skills/1")
        .timeout(Duration::from_secs(15))
        .build()
        .unwrap();
    let mut state = PublicState::default();
    for publisher in publishers()
        .iter()
        .filter(|publisher| !publisher.repository.is_empty())
    {
        let skills = tokio::time::timeout(REFRESH_TOTAL, fetch_publisher(&client, publisher))
            .await
            .expect("public publisher fetch timed out")
            .expect("public publisher catalog could not be fetched and parsed");
        assert!(
            !skills.is_empty(),
            "publisher {} has no catalog entries",
            publisher.id
        );
        eprintln!(
            "{}: {} verified public skill documents",
            publisher.id,
            skills.len()
        );
        state.accept(publisher.id, skills, Utc::now());
    }
    validate_state(&state).expect("real public snapshot must survive cache validation");
    if let Some(path) = std::env::var_os("TC_SKILLS_SMOKE_OUTPUT") {
        let value = json!({
            "enabled": true,
            "catalog": {"publishers": publishers(), "skills": state.skills, "checked_at": state.checked_at},
            "copy": copy(),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }
}

#[test]
fn tree_rejects_truncation_symlinks_and_unsafe_paths() {
    let good = serde_json::json!({"truncated":false,"tree":[
        {"path":"skills/one/SKILL.md","type":"blob","mode":"100644"},
        {"path":"skills/link/SKILL.md","type":"blob","mode":"120000"},
        {"path":"skills/../bad/SKILL.md","type":"blob","mode":"100644"},
        {"path":"other/SKILL.md","type":"blob","mode":"100644"}
    ]});
    assert_eq!(
        skill_paths(&good, "skills").unwrap(),
        vec!["skills/one/SKILL.md"]
    );
    let mut truncated = good;
    truncated["truncated"] = true.into();
    assert!(skill_paths(&truncated, "skills").is_err());
}

#[test]
fn state_retains_old_catalog_when_one_publisher_fails() {
    let mut state = PublicState::default();
    let first = catalog_skill("obra", "skills/one/SKILL.md", "a");
    state.accept("obra", vec![first.clone()], Utc::now());
    state.failed("obra");
    assert_eq!(state.skills.len(), 1);
    assert_eq!(state.skills[0].sha256, first.sha256);
    assert_eq!(state.publisher_states["obra"].status, "unknown");
}

#[test]
fn feed_dates_are_observations_and_history_requires_a_prior_version() {
    let mut state = PublicState::default();
    let at = Utc::now();
    let mut skill = catalog_skill("obra", "skills/one/SKILL.md", "a");
    state.accept("obra", vec![skill.clone()], at);
    assert_eq!(state.feed[0].kind, "added");
    assert_eq!(state.feed[0].observed_at, at);
    assert!(!state.history.contains_key(&skill.id));
    skill.sha256 = "b".repeat(64);
    state.accept(
        "obra",
        vec![skill.clone()],
        at + chrono::Duration::seconds(1),
    );
    assert_eq!(state.feed[0].kind, "updated");
    assert_eq!(state.history[&skill.id], vec!["a".repeat(64)]);
    state.accept("obra", vec![skill], at + chrono::Duration::seconds(2));
    assert_eq!(state.feed.len(), 2);
}

#[test]
fn follows_validate_fixed_publisher_and_persist_no_inventory() {
    let dir = tempfile::tempdir().unwrap();
    let mut state = PublicState::default();
    state.follow("obra", true).unwrap();
    assert!(state.follow("https://evil.invalid", true).is_err());
    persist(dir.path(), &state).unwrap();
    let body = std::fs::read_to_string(dir.path().join(STATE_FILE)).unwrap();
    assert!(!body.contains("inventory"));
    assert!(load(dir.path()).unwrap().followed.contains("obra"));
}

#[cfg(unix)]
#[test]
fn symlink_cache_is_neither_loaded_nor_overwritten() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    std::fs::write(&target, b"secret").unwrap();
    std::os::unix::fs::symlink(&target, dir.path().join(STATE_FILE)).unwrap();
    assert!(load(dir.path()).is_err());
    assert!(persist(dir.path(), &PublicState::default()).is_err());
    assert_eq!(std::fs::read(target).unwrap(), b"secret");
}

fn catalog_skill(publisher: &str, path: &str, hash: &str) -> CatalogSkill {
    CatalogSkill {
        id: format!("{publisher}:{path}"),
        publisher_id: publisher.into(),
        name: "One".into(),
        description: "Test".into(),
        repository: "obra/superpowers".into(),
        path: path.into(),
        revision: "a".repeat(40),
        sha256: hash.repeat(64),
        source_url: "https://github.com/obra/superpowers/blob/main/skills/one/SKILL.md".into(),
        instructions_url: "https://github.com/obra/superpowers#installation".into(),
    }
}

#[test]
fn branch_ref_requires_an_immutable_commit_object() {
    let sha = "a".repeat(40);
    assert_eq!(
        resolve_revision(&json!({"object":{"type":"commit","sha":sha}})).unwrap(),
        sha
    );
    assert!(resolve_revision(&json!({"sha":sha})).is_err());
    assert!(resolve_revision(&json!({"object":{"type":"tree","sha":sha}})).is_err());
    assert!(resolve_revision(&json!({"object":{"type":"commit","sha":"main"}})).is_err());
}

#[test]
fn tree_file_count_is_bounded_without_silently_truncating() {
    let tree: Vec<_> = (0..MAX_SKILLS+1).map(|index| json!({"path":format!("skills/{index}/SKILL.md"),"type":"blob","mode":"100644"})).collect();
    assert!(skill_paths(&json!({"truncated":false,"tree":tree}), "skills").is_err());
}

#[test]
fn response_rows_observe_encoded_bytes_and_report_partial() {
    let (rows, partial) = bounded_rows([json!({"name":"one"}), json!({"name":"two"})], 20);
    assert!(partial);
    assert_eq!(rows, vec![json!({"name":"one"})]);
    let (rows, partial) = bounded_rows([json!({"name":"\t".repeat(1024)})], 100);
    assert!(partial);
    assert!(rows.is_empty());
}

#[test]
fn disabled_skills_never_opens_the_cache_or_scans_local_roots() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = false;
    let value = snapshot(&shared);
    assert_eq!(value["enabled"], false);
    assert!(!shared.insights_skills.runtime.lock().unwrap().loaded);
    assert!(!shared.store.dir().join(STATE_FILE).exists());
}

#[test]
fn dry_run_disables_proactive_catalog_even_when_insights_is_on() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    shared.dev_dry_run = true;
    assert!(!enabled(&shared));
    assert_eq!(snapshot(&shared)["enabled"], false);
    assert!(!shared.insights_skills.runtime.lock().unwrap().loaded);
}

#[test]
fn changing_a_never_rule_invalidates_cached_global_inventory() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let policy_before = inventory_policy(&shared).unwrap();
    let inventory: Inventory = serde_json::from_value(json!({
        "skills":[{"id":"cached-global","name":"Cached","description":"Test","agent":"codex","scope":"global","location":"~/.codex/skills/cached/SKILL.md","sha256":"a".repeat(64),"source":null}],
        "partial":false,"coverage":[]
    })).unwrap();
    shared
        .insights_skills
        .state(shared.store.dir())
        .unwrap()
        .inventory = Some((Instant::now(), policy_before.clone(), inventory));
    assert_eq!(
        snapshot(&shared)["inventory"]["skills"][0]["id"],
        "cached-global"
    );
    let never = tempfile::tempdir().unwrap();
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            never.path().to_str().unwrap(),
            super::super::policy::ProjectMode::Ignore,
            Utc::now(),
        )
        .unwrap();
    let policy_after = inventory_policy(&shared).unwrap();
    assert_eq!(policy_before.projects, policy_after.projects);
    assert!(!policy_after.denied_roots.is_empty());
    assert_ne!(policy_before, policy_after);
    assert_eq!(snapshot(&shared)["inventory"]["skills"], json!([]));
}

#[test]
fn global_never_keeps_public_catalog_but_suppresses_local_inventory() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.policy.lock().unwrap().contribution_override =
        Some(super::super::policy::ContributionOverride {
            mode: super::super::policy::ProjectMode::Ignore,
            since: Utc::now(),
            arming: None,
            granted_under: None,
            claim: None,
        });
    let policy = inventory_policy(&shared).unwrap();
    assert!(policy.all_local_excluded);
    let value = snapshot(&shared);
    assert_eq!(value["inventory"]["skills"], json!([]));
    assert_eq!(
        value["inventory"]["coverage"][0]["status"],
        "policy_excluded"
    );
    assert_eq!(value["catalog"]["publishers"].as_array().unwrap().len(), 4);
}

#[test]
fn only_successful_recent_catalog_can_claim_an_installed_update() {
    let now = Utc::now();
    let mut state = PublicState::default();
    let current = catalog_skill("obra", "skills/one/SKILL.md", "b");
    state.accept("obra", vec![current.clone()], now);
    state
        .history
        .insert(current.id.clone(), vec!["a".repeat(64)]);
    let inventory: Inventory = serde_json::from_value(json!({
        "skills":[{"id":"local-one","name":"One","description":"Test","agent":"codex","scope":"global","location":"~/.codex/skills/one/SKILL.md","sha256":"a".repeat(64),"source":{"repository":"obra/superpowers","path":"skills/one/SKILL.md","revision":null}}],
        "partial":false,"coverage":[]
    })).unwrap();
    assert_eq!(
        updates(&inventory, &state, now)[0]["status"],
        "update_available"
    );
    assert_eq!(
        updates(&inventory, &state, now + chrono::Duration::hours(7))[0]["status"],
        "unknown"
    );
    state.failed("obra");
    assert_eq!(updates(&inventory, &state, now)[0]["status"], "unknown");
}

#[test]
fn known_source_outside_catalog_is_unavailable_instead_of_unknown() {
    let now = Utc::now();
    let inventory: Inventory = serde_json::from_value(json!({
        "skills":[{"id":"local","name":"One","description":"Test","agent":"codex","scope":"global","location":"~/.codex/skills/one/SKILL.md","sha256":"a".repeat(64),"source":{"repository":"another/publisher","path":"skills/one/SKILL.md","revision":null}}],
        "partial":false,"coverage":[]
    })).unwrap();
    assert_eq!(
        updates(&inventory, &PublicState::default(), now)[0]["status"],
        "unavailable"
    );
}

fn with_feature_runtime(mut shared: DaemonShared) -> DaemonShared {
    let registry = serde_json::from_value(json!({
        "schema_version":1,"registry_version":2,
        "flags":{"insights_skills":{"description":"Skills","owner":"tests","class":"product","default":false,"required_capabilities":["insights_skills_v1"],"timing":"live"}}
    })).unwrap();
    let profile = serde_json::from_value(json!({
        "schema_version":1,"registry_version":2,"environment":"testing",
        "capabilities":["insights_skills_v1"],"remote_enable":["insights_skills"],"signing_keys":[]
    }))
    .unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        registry,
        crate::feature_flags::test_signing::pin(profile),
        shared.store.dir(),
        Utc::now(),
    )
    .unwrap();
    shared
}

fn set_skills_flag(shared: &DaemonShared, revision: u64, on: bool) {
    let now = Utc::now();
    let manifest = serde_json::to_vec(&json!({
        "schema_version":1,"environment":"testing","revision":revision,
        "issued_at":now,"expires_at":now+chrono::Duration::hours(1),
        "flags":{"insights_skills":on}
    }))
    .unwrap();
    shared
        .feature_flags
        .accept(&crate::feature_flags::test_signing::sign(&manifest), now)
        .unwrap();
}

#[tokio::test]
async fn live_flag_off_on_off_gates_inventory_follow_and_proactive_refresh() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let project = tempfile::tempdir().unwrap();
    let skill = project.path().join(".agents/skills/local/SKILL.md");
    std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
    std::fs::write(
        &skill,
        b"---\nname: Before\ndescription: Local fixture\n---\n",
    )
    .unwrap();
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            project.path().to_str().unwrap(),
            super::super::policy::ProjectMode::NotifyOnly,
            Utc::now(),
        )
        .unwrap();
    let request = Request {
        id: 1,
        method: "insights_skills".into(),
        params: json!({"refresh":true}),
    };
    let follow = Request {
        id: 2,
        method: "insights_skills_follow".into(),
        params: json!({"publisher_id":"obra","followed":true}),
    };

    assert!(
        !shared
            .feature_flags
            .snapshot()
            .is_enabled("insights_skills")
    );
    maybe_refresh_public(&shared);
    assert_eq!(
        handle(&shared, &request).await.result.unwrap()["enabled"],
        false
    );
    assert_eq!(
        handle_follow(&shared, &follow).result.unwrap()["enabled"],
        false
    );
    {
        let runtime = shared.insights_skills.runtime.lock().unwrap();
        assert!(!runtime.loaded);
        assert!(runtime.inventory.is_none());
        assert!(runtime.last_attempt.is_none());
    }
    assert!(!shared.store.dir().join(STATE_FILE).exists());

    set_skills_flag(&shared, 1, true);
    maybe_refresh_public(&shared);
    assert!(
        shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .last_attempt
            .is_some()
    );
    let result = handle(&shared, &request).await.result.unwrap();
    assert_eq!(result["enabled"], true);
    assert!(
        result["inventory"]["skills"]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row["name"] == "Before")
    );
    assert!(handle_follow(&shared, &follow).error.is_none());
    let persisted = std::fs::read(shared.store.dir().join(STATE_FILE)).unwrap();
    let last_attempt = shared.insights_skills.runtime.lock().unwrap().last_attempt;

    set_skills_flag(&shared, 2, false);
    std::fs::write(
        skill,
        b"---\nname: After\ndescription: Changed after disable\n---\n",
    )
    .unwrap();
    maybe_refresh_public(&shared);
    assert_eq!(
        handle(&shared, &request).await.result.unwrap()["enabled"],
        false
    );
    let unfollow = Request {
        params: json!({"publisher_id":"obra","followed":false}),
        ..follow
    };
    assert_eq!(
        handle_follow(&shared, &unfollow).result.unwrap()["enabled"],
        false
    );
    assert_eq!(
        std::fs::read(shared.store.dir().join(STATE_FILE)).unwrap(),
        persisted
    );
    let runtime = shared.insights_skills.runtime.lock().unwrap();
    assert_eq!(runtime.last_attempt, last_attempt);
    assert!(runtime.public.followed.contains("obra"));
    assert!(
        runtime
            .inventory
            .as_ref()
            .unwrap()
            .2
            .skills
            .iter()
            .any(|row| row.name == "Before")
    );
    assert!(!runtime.refreshing);
}

#[test]
fn a_missing_registry_flag_cannot_enable_skills() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut shared = DaemonShared::load(store).unwrap();
    let registry =
        serde_json::from_value(json!({"schema_version":1,"registry_version":2,"flags":{}}))
            .unwrap();
    let profile = serde_json::from_value(json!({"schema_version":1,"registry_version":2,"environment":"testing","capabilities":["insights_skills_v1"],"remote_enable":[],"signing_keys":[]})).unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        registry,
        crate::feature_flags::test_signing::pin(profile),
        shared.store.dir(),
        Utc::now(),
    )
    .unwrap();
    shared.settings.lock().unwrap().insights_counter_pass = true;
    maybe_refresh_public(&shared);
    assert_eq!(snapshot(&shared)["enabled"], false);
    assert!(!shared.insights_skills.runtime.lock().unwrap().loaded);
}
