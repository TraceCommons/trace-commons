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
        let skills =
            tokio::time::timeout(REFRESH_TOTAL, fetch_publisher(&client, publisher, &|| true))
                .await
                .expect("public publisher fetch timed out")
                .expect("public publisher catalog could not be fetched and parsed");
        assert!(
            !skills.skills.is_empty(),
            "publisher {} has no catalog entries",
            publisher.id
        );
        eprintln!(
            "{}: {} verified public skill documents",
            publisher.id,
            skills.skills.len()
        );
        eprintln!("{}: complete={}", publisher.id, skills.complete);
        state.accept_fetched(publisher.id, skills, Utc::now());
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
    assert!(state.feed.is_empty(), "first catalog is a baseline");
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
    assert_eq!(state.feed.len(), 1);
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
        source_url: format!(
            "https://github.com/obra/superpowers/blob/{}/{path}",
            "a".repeat(40)
        ),
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
fn truncated_usage_rows_drop_the_per_agent_gaps() {
    let usage = || {
        json!({"partial":false,"partial_agents":[{"agent":"codex","days":[],"undated":1}],
            "skills":[{"name":"one"},{"name":"two"}]})
    };
    let mut kept = usage();
    bound_usage_rows(&mut kept, 1024);
    assert_eq!(kept["partial"], false);
    assert_eq!(kept["partial_agents"][0]["agent"], "codex");
    let mut cut = usage();
    bound_usage_rows(&mut cut, 20);
    assert_eq!(cut["partial"], true);
    assert!(
        cut.get("partial_agents").is_none(),
        "shell falls back to partial"
    );
}

/// 23 usage rows shaped as the daemon sends them (20 session references
/// and evidence entries each, the cap), with `days` daily buckets each.
fn owner_scale_usage(days: usize) -> Value {
    let row = |n: usize| {
        let windows: Vec<Value> = [7, 30, 90]
            .iter()
            .map(|d| json!({"days":d,"invocations":100,"loads":1,"sessions":10}))
            .collect();
        json!({"name":format!("plugin-name:skill-name-{n:02}"),"agent":"claude_code","population":"direct",
            "invocations":1234,"loads":12,"sessions":40,"last_seen":"2026-11-15T11:00:00.123456Z",
            "session_refs":vec!["t12345.1234567890123456789"; 20],"parent_session_refs":[],
            "evidence":vec![json!({"session_ref":"t12345.1234567890123456789","parent_session_ref":null}); 20],
            "windows":windows,
            "daily":(0..days).map(|d| json!({"day":format!("2026-{:02}-{:02}", 8 + d / 28, 1 + d % 28),"invocations":10,"loads":0,"sessions":2})).collect::<Vec<_>>(),
            "undated_events":0})
    };
    json!({"partial":true,"partial_agents":[{"agent":"codex","days":[],"undated":240}],
        "skills":(0..23).map(row).collect::<Vec<_>>()})
}

#[test]
fn owner_scale_usage_keeps_the_per_agent_gaps() {
    // The owner's snapshot (about 418 KB, 290 installed skills) carries 23
    // usage rows; only those rows count against their 96 KB bound.
    let mut usage = owner_scale_usage(14);
    bound_usage_rows(&mut usage, 96 * 1024);
    assert_eq!(usage["skills"].as_array().unwrap().len(), 23);
    assert_eq!(usage["partial_agents"][0]["agent"], "codex");
    // Measured limit of the fallback: 23 rows of this shape with 31 or
    // more days of daily history each no longer fit, and the shell then
    // reads the global flag alone.
    let fits = |days| {
        let mut usage = owner_scale_usage(days);
        bound_usage_rows(&mut usage, 96 * 1024);
        usage.get("partial_agents").is_some()
    };
    assert!(fits(30));
    assert!(!fits(31));
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
fn inventory_remains_available_when_usage_analysis_is_disabled() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = false;
    let value = snapshot(&shared);
    assert_eq!(value["enabled"], true);
    assert_eq!(value["usage"]["enabled"], false);
    assert!(!enabled(&shared));
    assert!(!shared.store.dir().join(STATE_FILE).exists());
}

#[test]
fn ignoring_unknown_sessions_keeps_known_skill_inventory_readable() {
    use super::super::policy::{ProjectMode, UNKNOWN_PROJECT_KEY};

    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let projects = tempfile::tempdir().unwrap();
    for (folder, name, mode) in [
        ("visible", "Visible", ProjectMode::NotifyOnly),
        ("private", "Private", ProjectMode::Ignore),
    ] {
        let project = projects.path().join(folder);
        let skill = project.join(".agents/skills/example/SKILL.md");
        std::fs::create_dir_all(skill.parent().unwrap()).unwrap();
        std::fs::write(
            skill,
            format!("---\nname: {name}\ndescription: Local fixture\n---\n"),
        )
        .unwrap();
        shared
            .policy
            .lock()
            .unwrap()
            .set_mode(project.to_str().unwrap(), mode, Utc::now())
            .unwrap();
    }
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(UNKNOWN_PROJECT_KEY, ProjectMode::Ignore, Utc::now())
        .unwrap();

    let private = std::fs::canonicalize(projects.path().join("private")).unwrap();
    assert!(
        inventory_policy(&shared)
            .unwrap()
            .denied_roots
            .iter()
            .any(|root| std::fs::canonicalize(root).unwrap() == private)
    );
    let value = snapshot(&shared);
    assert_eq!(value["readable"], true, "reason: {}", value["reason"]);
    let skills = value["inventory"]["skills"].as_array().unwrap();
    assert!(skills.iter().any(|skill| skill["name"] == "Visible"));
    assert!(!skills.iter().any(|skill| skill["name"] == "Private"));
    // The session policy still excludes unknown evidence; only filesystem
    // inventory has no path to exclude for this sentinel.
    let super::super::insights_week::NeverFolders::These(never) =
        super::super::insights_week::never_project_keys(&shared).unwrap()
    else {
        panic!("a per-project Never rule must not exclude every project");
    };
    assert!(never.iter().any(|key| key == UNKNOWN_PROJECT_KEY));
}

#[test]
fn unresolved_never_folder_still_makes_skills_unreadable() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    // A malformed stored folder rule is not the recognized unknown bucket.
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            "relative-folder",
            super::super::policy::ProjectMode::Ignore,
            Utc::now(),
        )
        .unwrap();
    let value = snapshot(&shared);
    assert_eq!(value["readable"], false);
    assert_eq!(value["reason"], "insights-skills-policy-unavailable");
    assert!(value.get("inventory").is_none());
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
    assert!(handle_follow(&shared, &follow).error.is_none());
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

#[test]
fn snapshot_emits_bounded_details_without_canonical_paths_or_orphan_ids() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let policy = inventory_policy(&shared).unwrap();
    let mut inventory: Inventory = serde_json::from_value(json!({
        "skills":[{"id":"installed","name":"Review","description":"Test","agent":"codex","scope":"global","location":"~/.codex/skills/review/SKILL.md","sha256":"a".repeat(64),"source":null}],
        "details":{
            "installed":{"aliases":["~/.codex/skills/review/SKILL.md"],"availability":"unknown","definition_status":"valid"},
            "orphan":{"aliases":[],"availability":"unknown","definition_status":"valid"}
        },
        "partial":false,"coverage":[]
    })).unwrap();
    inventory
        .details
        .get_mut("installed")
        .unwrap()
        .canonical_path = PathBuf::from("/private/sensitive/canonical/SKILL.md");
    shared
        .insights_skills
        .state(shared.store.dir())
        .unwrap()
        .inventory = Some((Instant::now(), policy.clone(), inventory.clone()));
    let value = snapshot(&shared);
    assert_eq!(
        value["inventory"]["details"]["installed"]["availability"],
        "unknown"
    );
    assert_eq!(value["inventory"]["details"].as_object().unwrap().len(), 1);
    assert!(!value.to_string().contains("/private/sensitive"));
    assert!(
        value["inventory"]["details"]["installed"]
            .get("canonical_path")
            .is_none()
    );
    let installed = value["inventory"]["skills"].as_array().unwrap();
    let (bounded, partial) = bounded_inventory_details(&inventory, installed, 2);
    assert!(partial);
    assert!(bounded.is_empty());
    inventory.details.get_mut("installed").unwrap().aliases = vec!["x".repeat(65 * 1024)];
    shared
        .insights_skills
        .state(shared.store.dir())
        .unwrap()
        .inventory = Some((Instant::now(), policy, inventory));
    let truncated = snapshot(&shared);
    assert_eq!(truncated["inventory"]["partial"], true);
    assert_eq!(truncated["inventory"]["details"], json!({}));
    assert!(serde_json::to_vec(&truncated).unwrap().len() < super::super::ipc::MAX_LINE_BYTES);
}

#[test]
fn snapshot_scope_rejects_late_clear_feature_aba_and_policy_changes() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let scope = SnapshotScope::capture(&shared).unwrap();
    assert!(scope.is_current(&shared));
    shared.insights_counter.clear().unwrap();
    assert!(!scope.is_current(&shared));
    let scope = SnapshotScope::capture(&shared).unwrap();
    set_skills_flag(&shared, 2, false);
    set_skills_flag(&shared, 3, true);
    assert!(!scope.is_current(&shared));
    let scope = SnapshotScope::capture(&shared).unwrap();
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            "/private/test-project",
            super::super::policy::ProjectMode::Ignore,
            Utc::now(),
        )
        .unwrap();
    assert!(!scope.is_current(&shared));
}

#[test]
fn opencode_only_empty_completed_scan_is_checked_not_pending() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(with_feature_runtime(DaemonShared::load(store).unwrap()));
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    assert!(
        super::super::insights_week::skills_usage(&shared)["usage"]["last_analysis_at"].is_null()
    );
    let before = Utc::now();
    // No runtime: the real coalescing worker completes synchronously. No
    // ordinary counter pass runs, and the isolated source roots are empty.
    opencode::schedule(&shared);
    let value = super::super::insights_week::skills_usage(&shared);
    assert!(value["usage"]["skills"].as_array().unwrap().is_empty());
    let completed =
        DateTime::parse_from_rfc3339(value["usage"]["last_analysis_at"].as_str().unwrap())
            .unwrap()
            .with_timezone(&Utc);
    assert!(completed >= before && completed <= Utc::now());
    set_skills_flag(&shared, 2, false);
    set_skills_flag(&shared, 3, true);
    assert!(
        super::super::insights_week::skills_usage(&shared)["usage"]["last_analysis_at"].is_null()
    );
}

#[test]
fn configured_opencode_export_worker_counts_drills_and_invalidates_real_evidence() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = Arc::new(with_feature_runtime(DaemonShared::load(store).unwrap()));
    let exports = tempfile::tempdir().unwrap();
    let project = tempfile::tempdir().unwrap();
    let cwd = project.path().canonicalize().unwrap();
    let mut export: Value = serde_json::from_slice(include_bytes!(
        "../../../tests/fixtures/opencode/completed.json"
    ))
    .unwrap();
    export["info"]["directory"] = json!(cwd);
    export["messages"][1]["info"]["path"]["cwd"] = json!(cwd);
    export["messages"][1]["info"]["path"]["root"] = json!(cwd);
    let tool = &mut export["messages"][1]["parts"][1];
    tool["tool"] = json!("skill");
    tool["state"]["input"] = json!({"name":"review"});
    tool["state"]["output"] = json!("PRIVATE_DEFINITION_BYTES");
    let observed_at = Utc::now() - chrono::Duration::seconds(10);
    tool["state"]["time"]["start"] = json!(observed_at.timestamp_millis());
    tool["state"]["time"]["end"] = json!(observed_at.timestamp_millis());
    std::fs::write(
        exports.path().join("trace.json"),
        serde_json::to_vec(&export).unwrap(),
    )
    .unwrap();
    {
        let mut settings = shared.settings.lock().unwrap();
        settings.insights_counter_pass = true;
        settings.opencode_source = Some(crate::source::SourceDeclaration::Watch {
            path: exports.path().canonicalize().unwrap(),
        });
    }
    set_skills_flag(&shared, 1, true);
    let before = Utc::now();
    opencode::schedule(&shared);
    let (rows, partial, completed) = opencode::snapshot(&shared);
    assert!(!partial);
    assert!(completed.is_some_and(|at| at >= before));
    assert_eq!(
        rows.len(),
        1,
        "configured export must survive discover/load/session_at"
    );
    let row = &rows[0];
    assert_eq!(row.population, "unknown");
    assert_eq!(row.evidence.skills.len(), 1);
    assert_eq!(row.evidence.skills[0].name, "review");
    assert_eq!(row.evidence.skills[0].invocations, 1);
    assert_eq!(row.evidence.skills[0].loads, 0);
    assert_eq!(
        row.evidence.skills[0].last_seen.unwrap().timestamp_millis(),
        observed_at.timestamp_millis()
    );
    let request = Request {
        id: 1,
        method: "insights_skills".into(),
        params: json!({"session_ref":row.reference}),
    };
    let detail = opencode::detail(&shared, &request).result.unwrap();
    assert_eq!(detail["skill_usage"]["skills"][0]["invocations"], 1);
    assert!(!detail.to_string().contains("PRIVATE_DEFINITION_BYTES"));
    assert!(!detail.to_string().contains(cwd.to_str().unwrap()));
    // Clear rejects the issued capability before another worker poll.
    shared.insights_counter.clear().unwrap();
    assert!(opencode::rows(&shared).0.is_empty());
    assert_eq!(
        opencode::detail(&shared, &request).result.unwrap()["status"],
        "unavailable"
    );
    opencode::schedule(&shared);
    assert_eq!(opencode::rows(&shared).0.len(), 1);
    // A Never folder invalidates immediately and remains excluded on reread.
    shared
        .policy
        .lock()
        .unwrap()
        .set_mode(
            cwd.to_str().unwrap(),
            super::super::policy::ProjectMode::Ignore,
            Utc::now(),
        )
        .unwrap();
    assert!(opencode::rows(&shared).0.is_empty());
    opencode::schedule(&shared);
    assert!(opencode::rows(&shared).0.is_empty());
}

#[test]
fn initial_catalog_is_a_baseline_and_later_new_skills_are_added() {
    let mut state = PublicState::default();
    let at = Utc::now();
    state.failed("obra");
    state.accept(
        "obra",
        vec![catalog_skill("obra", "skills/one/SKILL.md", "a")],
        at,
    );
    assert!(state.feed.is_empty());
    state.accept(
        "obra",
        vec![
            catalog_skill("obra", "skills/one/SKILL.md", "a"),
            catalog_skill("obra", "skills/two/SKILL.md", "a"),
        ],
        at,
    );
    assert_eq!(state.feed.len(), 1);
    assert_eq!(state.feed[0].skill_id, "obra:skills/two/SKILL.md");
    assert_eq!(state.feed[0].kind, "added");
}

#[test]
fn background_defaults_and_opening_tab_share_refresh_throttles() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    maybe_refresh_public(&shared);
    let attempt = shared.insights_skills.runtime.lock().unwrap().last_attempt;
    assert!(attempt.is_some());
    assert!(
        shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .public
            .followed
            .is_empty()
    );
    shared.insights_skills.start_refresh(&shared, false);
    assert_eq!(
        shared.insights_skills.runtime.lock().unwrap().last_attempt,
        attempt
    );
    shared.insights_skills.start_refresh(&shared, true);
    assert_eq!(
        shared.insights_skills.runtime.lock().unwrap().last_attempt,
        attempt
    );
}

#[test]
fn malformed_cache_recovers_and_follow_can_persist_again() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    std::fs::write(shared.store.dir().join(STATE_FILE), b"not json").unwrap();
    {
        let runtime = shared.insights_skills.state(shared.store.dir()).unwrap();
        assert!(runtime.public.skills.is_empty());
        assert!(runtime.public.followed.is_empty());
        assert_eq!(runtime.error, Some(ERROR_CACHE));
    }
    shared.insights_skills.start_refresh(&shared, true);
    assert!(
        shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .last_attempt
            .is_some()
    );
    let request = Request {
        id: 1,
        method: "insights_skills_follow".into(),
        params: json!({"publisher_id":"obra","followed":true}),
    };
    assert!(handle_follow(&shared, &request).error.is_none());
    assert!(load(shared.store.dir()).unwrap().followed.contains("obra"));
    assert!(
        shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .error
            .is_none()
    );
}

#[cfg(unix)]
#[test]
fn unsafe_cache_write_can_be_retried_after_destination_is_repaired() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let target = shared.store.dir().join("untouched");
    std::fs::write(&target, b"secret").unwrap();
    let cache = shared.store.dir().join(STATE_FILE);
    std::os::unix::fs::symlink(&target, &cache).unwrap();
    let request = Request {
        id: 1,
        method: "insights_skills_follow".into(),
        params: json!({"publisher_id":"obra","followed":true}),
    };
    assert!(handle_follow(&shared, &request).error.is_some());
    assert!(
        !shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .public
            .followed
            .contains("obra")
    );
    std::fs::remove_file(cache).unwrap();
    assert!(handle_follow(&shared, &request).error.is_none());
    assert!(load(shared.store.dir()).unwrap().followed.contains("obra"));
    assert_eq!(std::fs::read(target).unwrap(), b"secret");
}

#[tokio::test]
async fn malformed_document_keeps_other_catalog_entries_and_marks_partial() {
    let publisher = &publishers()[1];
    let fetched = fetch_publisher_with(publisher, &|| true, |url, _| async move {
        Ok(if url.contains("/git/ref/") {
            serde_json::to_vec(&json!({"object":{"type":"commit","sha":"a".repeat(40)}})).unwrap()
        } else if url.contains("/git/trees/") {
            serde_json::to_vec(&json!({"truncated":false,"tree":[
                {"path":"skills/one/SKILL.md","type":"blob","mode":"100644"},
                {"path":"skills/two/SKILL.md","type":"blob","mode":"100644"}
            ]}))
            .unwrap()
        } else if url.ends_with("skills/one/SKILL.md") {
            b"---\nname: One\ndescription: Valid fixture\n---\n".to_vec()
        } else {
            b"invalid frontmatter".to_vec()
        })
    })
    .await
    .unwrap();
    assert!(!fetched.complete);
    assert_eq!(fetched.skills.len(), 1);
    let mut state = PublicState::default();
    state.accept(
        "obra",
        vec![catalog_skill("obra", "skills/two/SKILL.md", "a")],
        Utc::now(),
    );
    state.accept_fetched("obra", fetched, Utc::now());
    assert_eq!(
        state.skills.len(),
        2,
        "partial fetch retains unrefreshed entries"
    );
    assert_eq!(state.publisher_states["obra"].status, "unknown");
}

#[tokio::test]
async fn live_disable_stops_fetch_before_next_document_request() {
    let on = std::cell::Cell::new(true);
    let calls = std::cell::Cell::new(0);
    let fetched = fetch_publisher_with(&publishers()[1], &|| on.get(), |url, _| {
        calls.set(calls.get() + 1);
        let response = if url.contains("/git/ref/") {
            json!({"object":{"type":"commit","sha":"a".repeat(40)}})
        } else {
            on.set(false);
            json!({"truncated":false,"tree":[{"path":"skills/one/SKILL.md","type":"blob","mode":"100644"}]})
        };
        std::future::ready(Ok(serde_json::to_vec(&response).unwrap()))
    }).await;
    assert!(fetched.is_err());
    assert_eq!(calls.get(), 2);
}

#[test]
fn observed_version_history_is_bounded_without_network_history() {
    let mut state = PublicState::default();
    for n in 0..12 {
        state.accept(
            "obra",
            vec![catalog_skill(
                "obra",
                "skills/one/SKILL.md",
                &format!("{n:x}"),
            )],
            Utc::now(),
        );
    }
    assert_eq!(state.history["obra:skills/one/SKILL.md"].len(), 8);
    assert_eq!(state.history["obra:skills/one/SKILL.md"][0], "3".repeat(64));
    assert_eq!(state.history["obra:skills/one/SKILL.md"][7], "a".repeat(64));
}

#[test]
fn disabled_refresh_discards_results_without_resetting_throttle() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    shared.settings.lock().unwrap().insights_counter_pass = true;
    set_skills_flag(&shared, 1, true);
    shared.insights_skills.start_refresh(&shared, true);
    let attempt = shared.insights_skills.runtime.lock().unwrap().last_attempt;
    set_skills_flag(&shared, 2, false);
    shared.insights_skills.finish_refresh(
        &shared,
        shared.store.dir(),
        vec![(
            "obra",
            Ok(FetchedCatalog {
                skills: vec![catalog_skill("obra", "skills/one/SKILL.md", "a")],
                complete: true,
            }),
        )],
    );
    {
        let runtime = shared.insights_skills.runtime.lock().unwrap();
        assert!(!runtime.refreshing);
        assert!(runtime.public.skills.is_empty());
        assert_eq!(runtime.last_attempt, attempt);
    }
    assert!(!shared.store.dir().join(STATE_FILE).exists());
    set_skills_flag(&shared, 3, true);
    shared.insights_skills.start_refresh(&shared, true);
    assert_eq!(
        shared.insights_skills.runtime.lock().unwrap().last_attempt,
        attempt
    );
}

#[cfg(unix)]
#[test]
fn failed_refresh_persistence_does_not_latch_out_future_refreshes() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    shared.settings.lock().unwrap().insights_counter_pass = true;
    set_skills_flag(&shared, 1, true);
    shared.insights_skills.start_refresh(&shared, true);
    let target = shared.store.dir().join("untouched");
    std::fs::write(&target, b"secret").unwrap();
    let cache = shared.store.dir().join(STATE_FILE);
    std::os::unix::fs::symlink(&target, &cache).unwrap();
    shared
        .insights_skills
        .finish_refresh(&shared, shared.store.dir(), vec![]);
    assert_eq!(
        shared.insights_skills.runtime.lock().unwrap().error,
        Some(ERROR_CACHE)
    );
    std::fs::remove_file(cache).unwrap();
    let old = Instant::now() - REFRESH_DEBOUNCE;
    shared.insights_skills.runtime.lock().unwrap().last_attempt = Some(old);
    shared.insights_skills.start_refresh(&shared, true);
    assert_ne!(
        shared.insights_skills.runtime.lock().unwrap().last_attempt,
        Some(old)
    );
    shared
        .insights_skills
        .finish_refresh(&shared, shared.store.dir(), vec![]);
    assert!(load(shared.store.dir()).is_ok());
    assert_eq!(
        shared.insights_skills.runtime.lock().unwrap().error,
        Some(ERROR_REFRESH)
    );
    assert_eq!(std::fs::read(target).unwrap(), b"secret");
}

#[tokio::test]
async fn local_override_drives_skills_gate_and_invalidates_in_flight_scope() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    shared.settings.lock().unwrap().insights_counter_pass = false;
    let request = Request {
        id: 1,
        method: "insights_skills".into(),
        params: json!({}),
    };
    assert!(!feature_enabled(&shared));
    shared
        .feature_flags
        .set_local_override("insights_skills", Some(true), Utc::now())
        .unwrap();
    assert!(feature_enabled(&shared));
    let in_flight = SnapshotScope::capture(&shared).unwrap();
    assert!(in_flight.is_current(&shared));
    shared
        .feature_flags
        .set_local_override("insights_skills", Some(false), Utc::now())
        .unwrap();
    assert!(!in_flight.is_current(&shared));
    assert_eq!(
        handle(&shared, &request).await.result.unwrap()["enabled"],
        false
    );
    set_skills_flag(&shared, 1, true);
    assert!(
        !feature_enabled(&shared),
        "remote refresh must not undo local off"
    );
    shared
        .feature_flags
        .set_local_override("insights_skills", None, Utc::now())
        .unwrap();
    assert!(
        feature_enabled(&shared),
        "clear returns to current environment decision"
    );
    assert!(
        !in_flight.is_current(&shared),
        "re-enable cannot publish an old in-flight result"
    );
    assert!(
        !shared.settings.lock().unwrap().insights_counter_pass,
        "turning Skills on must not opt into Local trace analysis"
    );
}

fn with_catalog_flags(mut shared: DaemonShared) -> DaemonShared {
    let flag = |capability: &str| {
        json!({"description":"Skills","owner":"tests","class":"product","default":false,
               "required_capabilities":[capability],"timing":"live"})
    };
    let registry = serde_json::from_value(json!({
        "schema_version":1,"registry_version":3,
        "flags":{
            "insights_skills": flag("insights_skills_v1"),
            "skills_server_catalog": flag("skills_server_catalog_v1"),
            "skills_catalog_suggestions": flag("skills_catalog_suggestions_v1"),
        }
    }))
    .unwrap();
    let profile = serde_json::from_value(json!({
        "schema_version":1,"registry_version":3,"environment":"testing",
        "capabilities":["insights_skills_v1","skills_server_catalog_v1","skills_catalog_suggestions_v1"],
        "remote_enable":["insights_skills","skills_server_catalog","skills_catalog_suggestions"],
        "signing_keys":[]
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

fn set_catalog_flags(shared: &DaemonShared, revision: u64, server: bool, suggestions: bool) {
    let now = Utc::now();
    let manifest = serde_json::to_vec(&json!({
        "schema_version":1,"environment":"testing","revision":revision,
        "issued_at":now,"expires_at":now+chrono::Duration::hours(1),
        "flags":{"insights_skills":true,"skills_server_catalog":server,
                 "skills_catalog_suggestions":suggestions}
    }))
    .unwrap();
    shared
        .feature_flags
        .accept(&crate::feature_flags::test_signing::sign(&manifest), now)
        .unwrap();
}

/// A shared daemon holding a verified server catalog signed by a throwaway
/// key that only this test pins, with an installed copy of the catalog's
/// non-seeded skill.
fn with_server_catalog() -> (tempfile::TempDir, DaemonShared) {
    with_catalog(|_| {})
}

/// The same, with the fixture document changed by `edit` before signing.
fn with_catalog(edit: impl FnOnce(&mut Value)) -> (tempfile::TempDir, DaemonShared) {
    use super::server_catalog::tests::{document, key, public, signed};
    let (dir, store) = crate::config::tests_support::temp_store();
    let shared = with_catalog_flags(DaemonShared::load(store).unwrap());
    set_catalog_flags(&shared, 1, true, false);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let signer = key();
    *shared.insights_skills.catalog_keys.lock().unwrap() = Some(vec![public(&signer)]);
    let now = Utc::now();
    let mut value = document(now);
    edit(&mut value);
    // A document lists its publishers and its skills in id order.
    for list in ["publishers", "skills"] {
        value[list]
            .as_array_mut()
            .unwrap()
            .sort_by(|a, b| a["id"].as_str().cmp(&b["id"].as_str()));
    }
    let document = server_catalog::accept(
        shared.store.dir(),
        trace_commons_protocol::skills_catalog::Environment::Testing,
        &[public(&signer)],
        &signed(&signer, &value),
        now,
    )
    .unwrap();
    shared
        .insights_skills
        .finish_refresh(&shared, shared.store.dir(), seed_results(&document));
    let policy = inventory_policy(&shared).unwrap();
    let inventory: Inventory = serde_json::from_value(json!({
        "skills":[{"id":"lint","name":"lint","description":"Test","agent":"codex","scope":"global",
                   "location":"~/.codex/skills/lint/SKILL.md","sha256":"b".repeat(64),
                   "source":{"repository":"acme/skills","path":"lint/SKILL.md","revision":null}}],
        "partial":false,"coverage":[]
    }))
    .unwrap();
    shared
        .insights_skills
        .state(shared.store.dir())
        .unwrap()
        .inventory = Some((Instant::now(), policy, inventory));
    (dir, shared)
}

#[test]
fn a_server_catalog_keeps_seed_ids_and_adds_its_own_skills_with_facts() {
    let (_dir, shared) = with_server_catalog();
    {
        let runtime = shared.insights_skills.runtime.lock().unwrap();
        // Seeds flow through the existing path with today's ids.
        assert_eq!(
            runtime
                .public
                .skills
                .iter()
                .map(|s| s.id.as_str())
                .collect::<Vec<_>>(),
            ["obra:skills/plan/SKILL.md"]
        );
        // A seed the catalog does not list keeps its state and is unknown.
        assert_eq!(runtime.public.publisher_states["slack"].status, "unknown");
        assert_eq!(runtime.public.publisher_states["obra"].status, "current");
        // Only public metadata reaches the persisted cache.
        assert!(
            runtime
                .public
                .skills
                .iter()
                .all(|s| s.publisher_id == "obra")
        );
    }
    let value = snapshot(&shared);
    let catalog = &value["catalog"];
    assert_eq!(catalog["source"], "server");
    assert_eq!(catalog["total_skills"], 2);
    let rows = catalog["skills"].as_array().unwrap();
    // The installed skill's catalog row comes first.
    assert_eq!(rows[0]["id"], "gh.acme.skills:lint/SKILL.md");
    assert_eq!(rows[0]["license"]["spdx"], "MIT");
    assert_eq!(rows[0]["checks"]["frontmatter"], "valid");
    assert_eq!(rows[0]["checks_version"], 2);
    assert_eq!(rows[1]["id"], "obra:skills/plan/SKILL.md");
    assert_eq!(
        rows[1]["checks"]["document_bytes"], 10,
        "seed rows gain the server's facts"
    );
    let acme = catalog["publishers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["id"] == "gh.acme.skills")
        .unwrap();
    assert_eq!(acme["url"], "https://github.com/acme/skills");
    assert_eq!(acme["source_kind"], "marketplace");
    let states = catalog["publisher_states"].as_array().unwrap();
    let state = |id: &str| states.iter().find(|s| s["publisher_id"] == id).unwrap();
    assert_eq!(state("obra")["signals"]["stars"], 900);
    assert_eq!(state("gh.acme.skills")["status"], "current");
    assert_eq!(state("gh.acme.skills")["signals"]["commits_28d"], 3);
    // The installed copy's hash is the server's previous version.
    let update = &value["updates"][0];
    assert_eq!(update["catalog_skill_id"], "gh.acme.skills:lint/SKILL.md");
    assert_eq!(update["status"], "update_available");
    assert_eq!(
        value["suggestions"],
        json!({"available": false, "opted_in": false})
    );
}

#[test]
fn recently_active_publishers_come_from_the_verified_catalog_alone() {
    let (_dir, shared) = with_server_catalog();
    let first = snapshot(&shared);
    // acme has stars and a commit in the measured week; obra's activity is
    // unknown, which is not zero and ranks nowhere.
    assert_eq!(
        first["catalog"]["active_publishers"],
        json!([{"publisher_id": "gh.acme.skills", "skill_ids": ["gh.acme.skills:lint/SKILL.md"]}])
    );
    // Every id names a row and a publisher this same snapshot carries.
    let has = |list: &str, key: &str, id: &str| {
        first["catalog"][list]
            .as_array()
            .unwrap()
            .iter()
            .any(|row| row[key] == id)
    };
    assert!(has("skills", "id", "gh.acme.skills:lint/SKILL.md"));
    assert!(has("publishers", "id", "gh.acme.skills"));
    assert!(has("publisher_states", "publisher_id", "gh.acme.skills"));

    // Nothing local takes part: with no installed skill and local analysis
    // off, the same document names the same publishers and skills.
    set_inventory(&shared, json!([]));
    shared.settings.lock().unwrap().insights_counter_pass = false;
    let second = snapshot(&shared);
    assert!(second["updates"].as_array().unwrap().is_empty());
    assert_eq!(
        second["catalog"]["active_publishers"],
        first["catalog"]["active_publishers"]
    );
}

/// Replace the installed skills the snapshot reads.
fn set_inventory(shared: &DaemonShared, skills: Value) {
    let mut runtime = shared.insights_skills.runtime.lock().unwrap();
    let (at, policy, _) = runtime.inventory.take().unwrap();
    let inventory: Inventory =
        serde_json::from_value(json!({"skills": skills, "partial": false, "coverage": []}))
            .unwrap();
    runtime.inventory = Some((at, policy, inventory));
}

#[test]
fn the_active_publishers_do_not_depend_on_which_rows_an_install_is_sent() {
    // A catalog far past the snapshot's 180 KiB of rows. The most active
    // publisher has few stars, so by stars alone its row is cut; before the
    // rows of the active publishers were sent first, only an install holding
    // its skill was sent the row, and so only that install was shown it.
    let (_dir, shared) = with_catalog(|document| {
        let issued = document["issued_at"].clone();
        let template = document["skills"][0].clone();
        let mut skill = |publisher: &str, repository: &str, path: &str, description: String| {
            let mut row = template.clone();
            row["id"] = format!("{publisher}:{path}").into();
            row["publisher_id"] = publisher.into();
            row["repository"] = repository.into();
            row["path"] = path.into();
            row["description"] = description.into();
            row["source_url"] = format!(
                "https://github.com/{repository}/blob/{}/{path}",
                row["revision"].as_str().unwrap()
            )
            .into();
            document["skills"].as_array_mut().unwrap().push(row);
        };
        skill(
            "gh.small.skills",
            "small/skills",
            "tidy/SKILL.md",
            "Tidies.".into(),
        );
        for publisher in 0..60 {
            for index in 0..6 {
                skill(
                    &format!("gh.big{publisher}.skills"),
                    &format!("big{publisher}/skills"),
                    &format!("s{index}/SKILL.md"),
                    "x".repeat(900),
                );
            }
        }
        let publishers = document["publishers"].as_array_mut().unwrap();
        publishers.push(json!({"id": "gh.small.skills", "name": "small/skills", "repository": "small/skills",
            "url": "https://github.com/small/skills", "instructions_url": "https://github.com/small/skills",
            "source_kind": "list",
            "signals": {"stars": 10, "commits_7d": 15, "commits_28d": 40, "checked_at": issued}}));
        for publisher in 0..60 {
            publishers.push(json!({"id": format!("gh.big{publisher}.skills"), "name": format!("big{publisher}/skills"),
                "repository": format!("big{publisher}/skills"),
                "url": format!("https://github.com/big{publisher}/skills"),
                "instructions_url": format!("https://github.com/big{publisher}/skills"),
                "source_kind": "list",
                "signals": {"stars": 10_000 + publisher, "checked_at": issued}}));
        }
    });
    let expected = json!([
        {"publisher_id": "gh.small.skills", "skill_ids": ["gh.small.skills:tidy/SKILL.md"]},
        {"publisher_id": "gh.acme.skills", "skill_ids": ["gh.acme.skills:lint/SKILL.md"]},
    ]);
    let installed = |repository: &str, path: &str| {
        json!([{"id":"held","name":"held","description":"Test","agent":"codex","scope":"global",
                "location":"~/.codex/skills/held/SKILL.md","sha256":"b".repeat(64),
                "source":{"repository":repository,"path":path,"revision":null}}])
    };
    for inventory in [
        json!([]),
        installed("small/skills", "tidy/SKILL.md"),
        installed("big7/skills", "s0/SKILL.md"),
    ] {
        set_inventory(&shared, inventory.clone());
        let value = snapshot(&shared);
        let catalog = &value["catalog"];
        assert_eq!(catalog["total_skills"], 363);
        assert_eq!(
            catalog["partial"], true,
            "the fixture must overflow the cap"
        );
        let rows = catalog["skills"].as_array().unwrap();
        assert!(rows.len() < 363);
        assert_eq!(catalog["active_publishers"], expected, "{inventory}");
        // Each named skill and publisher is in this same snapshot.
        for id in [
            "gh.small.skills:tidy/SKILL.md",
            "gh.acme.skills:lint/SKILL.md",
        ] {
            assert!(rows.iter().any(|row| row["id"] == id), "{id} {inventory}");
        }
        for id in ["gh.small.skills", "gh.acme.skills"] {
            assert!(
                catalog["publisher_states"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|row| row["publisher_id"] == id && row["signals"]["stars"].is_u64()),
                "{id} {inventory}"
            );
        }
    }
}

#[test]
fn an_active_fixed_publisher_is_named_by_its_own_row_ids() {
    let (_dir, shared) = with_catalog(|document| {
        let issued = document["issued_at"].clone();
        document["publishers"][1]["signals"] =
            json!({"stars": 900, "commits_7d": 6, "commits_28d": 20, "checked_at": issued});
    });
    let value = snapshot(&shared);
    let catalog = &value["catalog"];
    assert_eq!(
        catalog["active_publishers"],
        json!([
            {"publisher_id": "obra", "skill_ids": ["obra:skills/plan/SKILL.md"]},
            {"publisher_id": "gh.acme.skills", "skill_ids": ["gh.acme.skills:lint/SKILL.md"]},
        ])
    );
    // The id is the one the fixed publisher's row carries in this snapshot,
    // the row follows and updates already use.
    let row = catalog["skills"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "obra:skills/plan/SKILL.md")
        .unwrap();
    assert_eq!(row["publisher_id"], "obra");
    assert_eq!(row["repository"], "obra/superpowers");
    let state = catalog["publisher_states"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["publisher_id"] == "obra")
        .unwrap();
    assert_eq!(state["signals"]["commits_7d"], 6);
}

#[test]
fn no_publisher_is_named_without_a_current_verified_catalog() {
    // The flag off: the held document is not read.
    let (_dir, shared) = with_server_catalog();
    set_catalog_flags(&shared, 2, false, false);
    let off = snapshot(&shared);
    assert_eq!(off["catalog"]["source"], "github");
    assert!(off["catalog"]["active_publishers"].is_null());

    // No pinned key (the shipped state): the cached document does not load.
    let (_dir, shared) = with_server_catalog();
    *shared.insights_skills.catalog_keys.lock().unwrap() = Some(vec![]);
    {
        let mut runtime = shared.insights_skills.runtime.lock().unwrap();
        runtime.server = None;
        runtime.server_loaded = false;
    }
    let unpinned = snapshot(&shared);
    assert_eq!(unpinned["catalog"]["source"], "github");
    assert!(unpinned["catalog"]["active_publishers"].is_null());

    // A document held past its expiry is no longer the catalog.
    let (_dir, shared) = with_server_catalog();
    assert!(snapshot(&shared)["catalog"]["active_publishers"].is_array());
    {
        let mut runtime = shared.insights_skills.runtime.lock().unwrap();
        let document = runtime.server.as_mut().unwrap();
        document.expires_at = Utc::now() - chrono::Duration::seconds(1);
    }
    let expired = snapshot(&shared);
    assert_eq!(expired["catalog"]["source"], "github");
    assert!(expired["catalog"]["active_publishers"].is_null());
}

#[test]
fn turning_the_server_catalog_flag_off_returns_to_github_rows() {
    let (_dir, shared) = with_server_catalog();
    set_catalog_flags(&shared, 2, false, false);
    let value = snapshot(&shared);
    assert_eq!(value["catalog"]["source"], "github");
    assert!(value["catalog"]["revision"].is_null());
    let rows = value["catalog"]["skills"].as_array().unwrap();
    assert!(rows.iter().all(|row| row["publisher_id"] == "obra"));
    assert!(rows.iter().all(|row| row.get("checks").is_none()));
}

#[test]
fn a_cached_server_catalog_needs_a_pinned_key_to_load() {
    let (_dir, shared) = with_server_catalog();
    let dir = shared.store.dir();
    let environment = trace_commons_protocol::skills_catalog::Environment::Testing;
    assert!(server_catalog::load_cached(dir, environment, &[], Utc::now()).is_none());
    // A daemon restarted with no pinned key (the shipped state) ignores it.
    *shared.insights_skills.catalog_keys.lock().unwrap() = Some(vec![]);
    {
        let mut runtime = shared.insights_skills.runtime.lock().unwrap();
        runtime.server = None;
        runtime.server_loaded = false;
    }
    assert_eq!(snapshot(&shared)["catalog"]["source"], "github");
}

#[test]
fn reporting_is_offered_only_under_its_flag_and_the_users_choice() {
    let (_dir, shared) = with_server_catalog();
    let request = |opted_in: bool| Request {
        id: 1,
        method: "insights_skills_suggestions".into(),
        params: json!({"opted_in": opted_in}),
    };
    let reply = handle_suggestions(&shared, &request(true));
    assert_eq!(
        reply.result.unwrap(),
        json!({"available": false, "opted_in": false})
    );
    assert!(
        !shared
            .store
            .dir()
            .join("insights-skills-suggestions-v1.json")
            .exists()
    );
    set_catalog_flags(&shared, 2, true, true);
    assert_eq!(
        snapshot(&shared)["suggestions"],
        json!({"available": true, "opted_in": false, "pending": 0, "sent_total": 0, "last_sent_at": null})
    );
    let reply = handle_suggestions(&shared, &request(true));
    assert_eq!(
        reply.result.unwrap(),
        json!({"available": true, "opted_in": true})
    );
    assert_eq!(snapshot(&shared)["suggestions"]["opted_in"], true);
    let bad = Request {
        id: 2,
        method: "insights_skills_suggestions".into(),
        params: json!({"opted_in": "yes"}),
    };
    assert!(handle_suggestions(&shared, &bad).error.is_some());
    // Local analysis off withdraws the offer without forgetting the choice.
    shared.settings.lock().unwrap().insights_counter_pass = false;
    assert_eq!(snapshot(&shared)["suggestions"]["available"], false);
}

/// Which fetchers one `choose_catalog` call used, and what it chose.
async fn choice(
    dir: &Path,
    server_on: bool,
    keys: &[[u8; 32]],
    served: Result<Vec<u8>, ()>,
) -> (bool, bool, Option<u64>, usize) {
    use std::sync::atomic::{AtomicBool, Ordering};
    let asked_server = AtomicBool::new(false);
    let asked_github = AtomicBool::new(false);
    let (server_flag, github_flag) = (&asked_server, &asked_github);
    let (document, results) = choose_catalog(
        dir,
        trace_commons_protocol::skills_catalog::Environment::Testing,
        server_on,
        keys,
        move || async move {
            server_flag.store(true, Ordering::SeqCst);
            served
        },
        move || async move {
            github_flag.store(true, Ordering::SeqCst);
            vec![("obra", Err(()))]
        },
    )
    .await;
    (
        asked_server.load(Ordering::SeqCst),
        asked_github.load(Ordering::SeqCst),
        document.map(|d| d.revision),
        results.len(),
    )
}

#[tokio::test]
async fn the_server_catalog_replaces_github_only_when_enabled_pinned_and_valid() {
    use super::server_catalog::tests::{document, key, public, signed};
    let dir = tempfile::tempdir().unwrap();
    let signer = key();
    let pinned = [public(&signer)];
    let now = Utc::now();
    let good = signed(&signer, &document(now));
    // Flag off: no catalog request, GitHub as before.
    assert_eq!(
        choice(dir.path(), false, &pinned, Ok(good.clone())).await,
        (false, true, None, 1)
    );
    // No pinned key (the shipped state): no catalog request either.
    assert_eq!(
        choice(dir.path(), true, &[], Ok(good.clone())).await,
        (false, true, None, 1)
    );
    // A refused document with nothing cached: GitHub.
    assert_eq!(
        choice(
            dir.path(),
            true,
            &pinned,
            Ok(signed(&key(), &document(now)))
        )
        .await,
        (true, true, None, 1)
    );
    // A valid document: GitHub is not contacted, and every seed has a result.
    let revision = Some(now.timestamp() as u64);
    assert_eq!(
        choice(dir.path(), true, &pinned, Ok(good)).await,
        (true, false, revision, 3)
    );
    // Unreachable now, but the verified cache still answers.
    assert_eq!(
        choice(dir.path(), true, &pinned, Err(())).await,
        (true, false, revision, 3)
    );
    // An expired cache does not: GitHub again.
    let stale = tempfile::tempdir().unwrap();
    let old = now - chrono::Duration::days(8);
    server_catalog::accept(
        stale.path(),
        trace_commons_protocol::skills_catalog::Environment::Testing,
        &pinned,
        &signed(&signer, &document(old)),
        old,
    )
    .unwrap();
    assert_eq!(
        choice(stale.path(), true, &pinned, Err(())).await,
        (true, true, None, 1)
    );
}

#[test]
fn home_defaults_show_real_changes_without_following_creators() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut state = PublicState::default();
    let at = Utc::now();
    for publisher in publishers()
        .iter()
        .filter(|publisher| !publisher.repository.is_empty())
    {
        let path = format!("{}/one/SKILL.md", publisher.skills_root);
        let mut skill = catalog_skill(publisher.id, &path, "a");
        skill.repository = publisher.repository.into();
        skill.source_url = source_url(publisher, &skill.revision, &path);
        skill.instructions_url = publisher.instructions_url.into();
        state.accept(publisher.id, vec![skill.clone()], at);
        skill.sha256 = "b".repeat(64);
        state.accept(publisher.id, vec![skill], at + chrono::Duration::seconds(1));
    }
    validate_state(&state).unwrap();
    persist(store.dir(), &state).unwrap();
    let before = std::fs::read(store.dir().join(STATE_FILE)).unwrap();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let value = snapshot(&shared);
    assert_eq!(
        value["default_publishers"],
        json!(["obra", "poteto", "slack"])
    );
    assert_eq!(value["followed_publishers"], json!([]));
    assert_eq!(value["feed"], json!([]), "Following remains explicit");
    assert_eq!(value["home_feed"].as_array().unwrap().len(), 3);
    assert!(
        value["home_feed"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| item["kind"] == "updated")
    );
    assert_eq!(
        std::fs::read(shared.store.dir().join(STATE_FILE)).unwrap(),
        before
    );
}

#[test]
fn home_defaults_do_not_invent_first_visit_updates_and_remain_after_following() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut state = PublicState::default();
    let at = Utc::now();
    state.accept(
        "obra",
        vec![catalog_skill("obra", "skills/one/SKILL.md", "a")],
        at,
    );
    persist(store.dir(), &state).unwrap();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let first = snapshot(&shared);
    assert_eq!(
        first["default_publishers"],
        json!(["obra", "poteto", "slack"])
    );
    assert_eq!(first["home_feed"], json!([]));
    {
        let mut runtime = shared.insights_skills.state(shared.store.dir()).unwrap();
        runtime.public.follow("obra", true).unwrap();
        runtime.public.accept(
            "obra",
            vec![catalog_skill("obra", "skills/one/SKILL.md", "b")],
            at + chrono::Duration::seconds(1),
        );
    }
    let chosen = snapshot(&shared);
    assert_eq!(chosen["followed_publishers"], json!(["obra"]));
    assert_eq!(
        chosen["default_publishers"],
        json!(["obra", "poteto", "slack"])
    );
    assert_eq!(chosen["home_feed"], json!([]));
    assert_eq!(chosen["feed"].as_array().unwrap().len(), 1);
    set_skills_flag(&shared, 2, false);
    assert!(snapshot(&shared).get("home_feed").is_none());
    assert!(snapshot(&shared).get("default_publishers").is_none());
}

#[tokio::test]
async fn home_defaults_refresh_public_sources_without_explicit_follows() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = false;
    maybe_refresh_public(&shared);
    assert!(
        shared
            .insights_skills
            .runtime
            .lock()
            .unwrap()
            .last_attempt
            .is_none()
    );
    shared.settings.lock().unwrap().insights_counter_pass = true;
    maybe_refresh_public(&shared);
    let runtime = shared.insights_skills.runtime.lock().unwrap();
    assert!(runtime.public.followed.is_empty());
    assert!(runtime.last_attempt.is_some());
}

#[test]
fn home_defaults_keep_each_creator_when_the_catalog_is_bounded() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut state = PublicState::default();
    for publisher in publishers()
        .iter()
        .filter(|publisher| !publisher.repository.is_empty())
    {
        let count = if publisher.id == "obra" { 16 } else { 1 };
        let skills = (0..count)
            .map(|index| {
                let path = format!("{}/example-{index:02}/SKILL.md", publisher.skills_root);
                let mut skill = catalog_skill(publisher.id, &path, "a");
                skill.repository = publisher.repository.into();
                skill.source_url = source_url(publisher, &skill.revision, &path);
                skill.instructions_url = publisher.instructions_url.into();
                skill.description = "x".repeat(16 * 1024);
                skill
            })
            .collect();
        state.accept(publisher.id, skills, Utc::now());
    }
    validate_state(&state).unwrap();
    persist(store.dir(), &state).unwrap();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    let value = snapshot(&shared);
    let rows = value["catalog"]["skills"].as_array().unwrap();
    for publisher in DEFAULT_HOME_PUBLISHERS {
        assert!(
            rows.iter().any(|row| row["publisher_id"] == publisher),
            "missing default creator {publisher}"
        );
    }
    assert_eq!(value["catalog"]["partial"], true);
    assert!(serde_json::to_vec(rows).unwrap().len() <= 180 * 1024);
    assert_eq!(value["followed_publishers"], json!([]));
    assert_eq!(value["home_feed"], json!([]));
}

fn home_news_item(publisher: &str, index: usize, at: DateTime<Utc>, padded: bool) -> FeedItem {
    FeedItem {
        id: format!(
            "{publisher}:{index:03}:{}",
            if padded {
                "\t".repeat(400)
            } else {
                String::new()
            }
        ),
        publisher_id: publisher.into(),
        skill_id: format!("{publisher}:skills/example-{index:03}/SKILL.md"),
        kind: "updated".into(),
        observed_at: at,
    }
}

fn home_news_snapshot(feed: Vec<FeedItem>, followed: &[&str]) -> Value {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    {
        let mut runtime = shared.insights_skills.state(shared.store.dir()).unwrap();
        runtime.public.feed = feed;
        runtime.public.followed = followed.iter().map(|id| (*id).to_owned()).collect();
    }
    snapshot(&shared)
}

#[test]
fn home_news_mixes_defaults_with_follows_without_duplicates_or_implicit_follows() {
    let at = Utc::now();
    let mut items: Vec<_> = ["obra", "poteto", "slack", "rex"]
        .into_iter()
        .map(|publisher| home_news_item(publisher, 0, at, false))
        .collect();
    items.push(items[0].clone());
    let value = home_news_snapshot(items, &["obra"]);
    assert_eq!(
        value["default_publishers"],
        json!(["obra", "poteto", "slack"])
    );
    assert_eq!(value["followed_publishers"], json!(["obra"]));
    let following = value["feed"].as_array().unwrap();
    let discovery = value["home_feed"].as_array().unwrap();
    assert_eq!(following.len(), 1);
    assert_eq!(following[0]["publisher_id"], "obra");
    assert_eq!(
        discovery
            .iter()
            .map(|row| row["publisher_id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["poteto", "slack"]
    );
    let ids: BTreeSet<_> = following
        .iter()
        .chain(discovery)
        .map(|row| row["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.len(), following.len() + discovery.len());
    assert_eq!(value["feed_partial"], false);
    assert_eq!(value["home_feed_partial"], false);
}

#[test]
fn home_news_shares_one_encoded_budget_and_gives_every_publisher_an_opportunity() {
    let at = Utc::now();
    let mut items = Vec::new();
    for publisher in ["obra", "poteto", "slack"] {
        for index in 0..80 {
            items.push(home_news_item(
                publisher,
                index,
                at + chrono::Duration::seconds(index as i64),
                true,
            ));
        }
    }
    let value = home_news_snapshot(items, &["obra"]);
    let following = value["feed"].as_array().unwrap();
    let discovery = value["home_feed"].as_array().unwrap();
    assert!(!following.is_empty());
    assert!(!discovery.is_empty());
    assert!(
        serde_json::to_vec(following).unwrap().len() + serde_json::to_vec(discovery).unwrap().len()
            <= 48 * 1024
    );
    for publisher in ["obra", "poteto", "slack"] {
        assert!(
            following
                .iter()
                .chain(discovery)
                .any(|row| row["publisher_id"] == publisher),
            "missing creator {publisher}"
        );
    }
    let counts: Vec<_> = ["obra", "poteto", "slack"]
        .into_iter()
        .map(|publisher| {
            following
                .iter()
                .chain(discovery)
                .filter(|row| row["publisher_id"] == publisher)
                .count()
        })
        .collect();
    assert!(
        counts.iter().max().unwrap() - counts.iter().min().unwrap() <= 1,
        "a burst must not consume another creator's turns: {counts:?}"
    );
    assert!(
        following[0]["id"]
            .as_str()
            .unwrap()
            .starts_with("obra:079:")
    );
    assert!(
        discovery[0]["id"]
            .as_str()
            .unwrap()
            .starts_with("poteto:079:")
    );
    assert_eq!(value["feed_partial"], true);
    assert_eq!(value["home_feed_partial"], true);
}

#[test]
fn home_news_partial_flags_describe_only_the_partition_that_was_cut() {
    let at = Utc::now();
    for followed_is_large in [false, true] {
        let mut items = vec![home_news_item(
            if followed_is_large { "poteto" } else { "obra" },
            0,
            at,
            false,
        )];
        items.extend((0..100).map(|index| {
            home_news_item(
                if followed_is_large { "obra" } else { "poteto" },
                index,
                at,
                true,
            )
        }));
        let value = home_news_snapshot(items, &["obra"]);
        assert_eq!(value["feed_partial"], followed_is_large);
        assert_eq!(value["home_feed_partial"], !followed_is_large);
        assert!(!value["feed"].as_array().unwrap().is_empty());
        assert!(!value["home_feed"].as_array().unwrap().is_empty());
    }
}

#[test]
fn home_news_returns_each_partition_newest_first_with_stable_id_ties() {
    let at = Utc::now();
    let old = at - chrono::Duration::days(1);
    let mut items = vec![
        home_news_item("poteto", 2, old, false),
        home_news_item("obra", 2, old, false),
        home_news_item("slack", 0, at, false),
        home_news_item("poteto", 1, at, false),
        home_news_item("obra", 1, at, false),
        home_news_item("obra", 0, at, false),
    ];
    let first = home_news_snapshot(items.clone(), &["obra"]);
    items.reverse();
    let second = home_news_snapshot(items, &["obra"]);
    for key in ["feed", "home_feed"] {
        assert_eq!(
            first[key], second[key],
            "input order must not select a different feed"
        );
    }
    assert_eq!(
        first["feed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["obra:000:", "obra:001:", "obra:002:"]
    );
    assert_eq!(
        first["home_feed"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["id"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["poteto:001:", "slack:000:", "poteto:002:"]
    );
}

/// The Windows `skillsHome` sample extra holds this snapshot: three creators'
/// catalogs refreshed twice (a baseline, then one skill updated and one
/// added), Obra followed. Everything but `usage` is the daemon's own reply;
/// `usage` is hand-written there (the counter pass reads real sessions, and
/// its windows are drawn against the daemon's clock), as is `_sample`.
#[test]
fn the_windows_skills_home_sample_is_this_snapshot() {
    let at = |s: &str| DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc);
    let (_dir, store) = crate::config::tests_support::temp_store();
    let mut state = PublicState::default();
    let named: &[(&str, &[(&str, &str)])] = &[
        (
            "obra",
            &[
                (
                    "brainstorming",
                    "Turns a rough idea into a design before any code is written.",
                ),
                (
                    "test-driven-development",
                    "Write the failing test first, then the code that passes it.",
                ),
                (
                    "systematic-debugging",
                    "Find the root cause before proposing a fix.",
                ),
                (
                    "writing-plans",
                    "Break an approved design into small, checked steps.",
                ),
            ],
        ),
        (
            "poteto",
            &[
                (
                    "code-review",
                    "Review a change for correctness and simplicity.",
                ),
                (
                    "refactor",
                    "Restructure code without changing what it does.",
                ),
            ],
        ),
        (
            "slack",
            &[
                ("slack-search", "Search channels and threads for context."),
                ("slack-summarize", "Summarise a long thread."),
            ],
        ),
    ];
    let make = |publisher: &Publisher, name: &str, desc: &str, hash: &str| {
        let path = format!("{}/{name}/SKILL.md", publisher.skills_root);
        let mut skill = catalog_skill(publisher.id, &path, hash);
        skill.name = name.into();
        skill.description = desc.into();
        skill.repository = publisher.repository.into();
        skill.source_url = source_url(publisher, &skill.revision, &path);
        skill.instructions_url = publisher.instructions_url.into();
        skill
    };
    for (id, skills) in named {
        let publisher = publishers().into_iter().find(|p| p.id == *id).unwrap();
        // Baseline three days before, all but the last skill.
        let base: Vec<_> = skills[..skills.len() - 1]
            .iter()
            .map(|(n, d)| make(&publisher, n, d, "a"))
            .collect();
        state.accept(id, base, at("2026-11-15T09:00:00Z"));
        // Then the last one added and the first one updated.
        let mut now: Vec<_> = skills
            .iter()
            .map(|(n, d)| make(&publisher, n, d, "a"))
            .collect();
        now[0] = make(&publisher, skills[0].0, skills[0].1, "b");
        state.accept(id, now, at("2026-11-18T14:00:00Z"));
    }
    state.follow("obra", true).unwrap();
    validate_state(&state).unwrap();
    persist(store.dir(), &state).unwrap();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let mut value = snapshot(&shared);
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../windows/src/TraceCommons.Interop/SampleExtras/skillsHome/insights_skills.json",
    );
    let mut recorded: Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    for key in ["copy", "usage"] {
        value.as_object_mut().unwrap().remove(key);
    }
    // The scan is stamped with the daemon's clock; the sample's is pinned.
    let scanned = value["inventory"]["scanned_at"].as_str().unwrap();
    assert!(DateTime::parse_from_rfc3339(scanned).is_ok());
    value["inventory"]["scanned_at"] = recorded["inventory"]["scanned_at"].clone();
    for key in ["_sample", "usage"] {
        recorded.as_object_mut().unwrap().remove(key);
    }
    assert_eq!(
        recorded,
        value,
        "update {} (keeping its usage and _sample) to:\n{}",
        path.display(),
        serde_json::to_string_pretty(&value).unwrap()
    );
}
