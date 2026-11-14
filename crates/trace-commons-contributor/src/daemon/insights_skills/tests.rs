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
fn background_refresh_requires_follow_but_opening_tab_can_fetch() {
    let (_dir, store) = crate::config::tests_support::temp_store();
    let shared = with_feature_runtime(DaemonShared::load(store).unwrap());
    set_skills_flag(&shared, 1, true);
    shared.settings.lock().unwrap().insights_counter_pass = true;
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
    shared.insights_skills.start_refresh(&shared, false);
    let attempt = shared.insights_skills.runtime.lock().unwrap().last_attempt;
    assert!(attempt.is_some());
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
