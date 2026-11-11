//! Claude sub-agent transcripts in the counter pass: rows of their own,
//! apart from the person's sessions. A child of `insights_week::tests`, for
//! its fixture.

use super::*;
use crate::insights::benchmarks::BenchPopulation;

/// The turn-series fixture with its message IDs made `tag`'s own, as a
/// sub-agent's are: identical IDs would read as one transcript resuming
/// another.
fn sub_agent_bytes(tag: &str) -> Vec<u8> {
    String::from_utf8(claude_bytes())
        .unwrap()
        .replace("msg_PRIVATE_", &format!("msg_PRIVATE_{tag}_"))
        .into_bytes()
}

/// The parent session, and its two spawned transcripts (a delegated
/// sub-agent and a workflow's), each the turn-series fixture's bytes.
struct Family {
    parent: CounterCandidate,
    spawned: Vec<SpawnedCandidate>,
}

fn family(f: &Fixture, prefix: &str) -> Family {
    let parent_path = f.write(&format!("{prefix}-PRIVATE-PARENT.jsonl"), &claude_bytes());
    let mut parent = candidate(SOURCE_CLAUDE_CODE, &parent_path);
    parent.group_member_count = 2;
    let spawned = [
        ("agent-a", SpawnedBy::Agent),
        ("agent-w", SpawnedBy::Workflow),
    ]
    .into_iter()
    .map(|(name, kind)| {
        let path = f.write(
            &format!("{prefix}-PRIVATE-{name}.jsonl"),
            &sub_agent_bytes(name),
        );
        SpawnedCandidate {
            parent: 0,
            kind,
            candidate: candidate(SOURCE_CLAUDE_CODE, &path),
        }
    })
    .collect();
    Family { parent, spawned }
}

fn run_family(
    f: &Fixture,
    candidates: &[CounterCandidate],
    spawned: &[SpawnedCandidate],
    never: bool,
) -> PassSummary {
    f.pass
        .run_pass_spawned(
            candidates,
            spawned,
            &if never {
                NeverFolders::These(vec![PROJECT.to_string()])
            } else {
                NeverFolders::NONE
            },
            now(),
            QUIET,
            &mut |_| (PROJECT.to_string(), never),
        )
        .unwrap()
}

fn store(f: &Fixture) -> Arc<CounterStore> {
    f.pass.load_cached().unwrap().unwrap()
}

fn bench(f: &Fixture, population: BenchPopulation) -> serde_json::Value {
    f.pass
        .benchmarks_value(true, &NeverFolders::NONE, false, 4, None, population, now())
}

#[test]
fn sub_agent_transcripts_are_rows_of_their_own_never_in_the_persons_figures() {
    // The person's figures with no sub-agent read at all.
    let alone = Fixture::new();
    let a = family(&alone, "x");
    run_family(&alone, std::slice::from_ref(&a.parent), &[], false);

    let f = Fixture::new();
    let fam = family(&f, "x");
    let summary = run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    assert_eq!(summary.read, 1, "the person's own reads");
    assert_eq!(summary.spawned_read, 2);

    let s = store(&f);
    assert_eq!(s.rows.len(), 1, "the parent only");
    assert_eq!(s.spawned.len(), 2);
    let parent_key = s.rows.keys().next().copied().unwrap();
    let parent = s.rows.values().next().unwrap();
    assert!(parent.facts.as_ref().unwrap().delegated_unread);
    let mut kinds: Vec<SpawnedBy> = s.spawned.values().map(|row| row.kind).collect();
    kinds.sort();
    assert_eq!(kinds, vec![SpawnedBy::Agent, SpawnedBy::Workflow]);
    for spawned in s.spawned.values() {
        assert_eq!(spawned.parent, parent_key);
        // Sub-agents carry their parent's session ID: never joined to its
        // route tally, never its goals, never placed by its start.
        assert_eq!(spawned.row.harness_session, None);
        assert!(spawned.row.goals.is_empty());
        assert_eq!(spawned.row.placed_at, None);
        assert_eq!(spawned.row.facts_version, SESSION_FACTS_VERSION);
        assert!(!spawned.row.facts.as_ref().unwrap().delegated_unread);
    }

    // The week, its tokens and spend, and the Sessions list are the
    // person's own: exactly as with no sub-agent read.
    let week = f.week(&[]);
    let base = alone.week(&[]);
    assert_eq!(claude_line(&week)["tokens"], claude_line(&base)["tokens"]);
    assert_eq!(claude_line(&week)["sessions"], 1);
    assert_eq!(week["sessions_stored"], base["sessions_stored"]);
    assert_eq!(week["rollup"], base["rollup"]);
    let listed = f.pass.sessions_value(true, &NeverFolders::NONE, &[], 100);
    let base_listed = alone
        .pass
        .sessions_value(true, &NeverFolders::NONE, &[], 100);
    assert_eq!(
        listed["sessions"].as_array().unwrap().len(),
        base_listed["sessions"].as_array().unwrap().len()
    );

    // Nothing private stored.
    let text = f.rows_text();
    for forbidden in ["PRIVATE", "agent-a", "agent-w", "/Users"] {
        assert!(!text.contains(forbidden), "{forbidden} stored");
    }
}

#[test]
fn an_unchanged_sub_agent_is_not_read_again() {
    let f = Fixture::new();
    let fam = family(&f, "x");
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    let again = run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    assert_eq!(again.spawned_read, 0);
    assert_eq!(store(&f).spawned.len(), 2);
}

#[test]
fn a_never_folders_sub_agents_are_not_read_and_go() {
    let f = Fixture::new();
    let fam = family(&f, "x");
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    assert_eq!(store(&f).spawned.len(), 2);
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, true);
    assert!(store(&f).spawned.is_empty());

    let fresh = Fixture::new();
    let fam = family(&fresh, "y");
    let summary = run_family(
        &fresh,
        std::slice::from_ref(&fam.parent),
        &fam.spawned,
        true,
    );
    assert_eq!(summary.spawned_read, 0);
    let rows = fresh.pass.load_cached().unwrap();
    assert!(rows.is_none_or(|s| s.spawned.is_empty()));
}

#[test]
fn a_sub_agent_still_being_written_waits() {
    let f = Fixture::new();
    let mut fam = family(&f, "x");
    fam.spawned[0].candidate.modified_at = now() - Duration::seconds(10);
    let summary = run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    assert_eq!(summary.spawned_read, 1);
}

#[test]
fn the_sub_agent_allowance_evicts_the_oldest_and_does_not_read_them_again() {
    let f = Fixture::new();
    f.pass.set_spawned_caps_for_test(2, usize::MAX, usize::MAX);
    let parent_path = f.write("PRIVATE-PARENT.jsonl", &claude_bytes());
    let parent = candidate(SOURCE_CLAUDE_CODE, &parent_path);
    let spawned: Vec<SpawnedCandidate> = (0..3)
        .map(|n| {
            let path = f.write(&format!("agent-{n}.jsonl"), &claude_bytes());
            let mut c = candidate(SOURCE_CLAUDE_CODE, &path);
            c.modified_at = written() - Duration::minutes(10 * (3 - n));
            SpawnedCandidate {
                parent: 0,
                kind: SpawnedBy::Agent,
                candidate: c,
            }
        })
        .collect();
    // One read a poll would be the budget's business; read all three now.
    let first = run_family(&f, std::slice::from_ref(&parent), &spawned, false);
    assert_eq!(first.spawned_read, 3);
    let s = store(&f);
    assert_eq!(s.spawned.len(), 2, "the newest two kept");
    assert_eq!(s.rows.len(), 1, "the person's row is untouched");
    let oldest = spawned[0].candidate.modified_at;
    assert!(s.spawned.values().all(|row| row.row.modified_at > oldest));
    // The evicted one is not read on every poll.
    let again = run_family(&f, std::slice::from_ref(&parent), &spawned, false);
    assert_eq!(again.spawned_read, 0);
}

#[test]
fn the_turn_allowance_evicts_too() {
    let f = Fixture::new();
    let fam = family(&f, "x");
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    let turns = store(&f).spawned.values().next().unwrap().row.turns();
    assert!(turns > 0);
    let g = Fixture::new();
    g.pass
        .set_spawned_caps_for_test(usize::MAX, turns, usize::MAX);
    let fam = family(&g, "x");
    run_family(&g, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    assert_eq!(store(&g).spawned.len(), 1);
}

#[test]
fn a_superseded_parents_sub_agents_are_read_from_the_profile_copy_only() {
    const ID: &str = "66666666-6666-4666-8666-666666666666";
    let f = Fixture::new();
    let profile_dir = f.dir.path().join("profile");
    std::fs::create_dir_all(&profile_dir).unwrap();
    // The watched export and the managed profile's copy of one session.
    let watched_path = f.write(&format!("{ID}.jsonl"), &claude_bytes());
    let profile_path = profile_dir.join(format!("{ID}.jsonl"));
    std::fs::write(&profile_path, claude_bytes()).unwrap();
    let watched = candidate(SOURCE_CLAUDE_CODE, &watched_path);
    let profile = CounterCandidate {
        role: CandidateRole::ManagedProfile,
        ..candidate(SOURCE_CLAUDE_CODE, &profile_path)
    };
    let sub = |name: &str, parent: usize| {
        let path = f.write(name, &claude_bytes());
        SpawnedCandidate {
            parent,
            kind: SpawnedBy::Agent,
            candidate: candidate(SOURCE_CLAUDE_CODE, &path),
        }
    };
    let spawned = vec![sub("agent-watched.jsonl", 0), sub("agent-profile.jsonl", 1)];
    let summary = run_family(&f, &[watched, profile], &spawned, false);
    assert_eq!(summary.spawned_read, 1, "the profile copy's sub-agent only");
    assert_eq!(store(&f).spawned.len(), 1);
}

#[test]
fn benchmarks_answer_over_one_population_at_a_time() {
    let f = Fixture::new();
    let fam = family(&f, "x");
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    let own = bench(&f, BenchPopulation::Own);
    let spawned = bench(&f, BenchPopulation::Spawned);
    assert_eq!(own["population"], "own");
    assert_eq!(spawned["population"], "spawned");
    let in_window = |v: &serde_json::Value| v["sessions"].as_u64().unwrap();
    // The fixture's turns are within four weeks of `now`.
    assert_eq!(in_window(&own), 1, "{own}");
    assert_eq!(in_window(&spawned), 2, "{spawned}");
    assert_eq!(spawned["delegated"], 0);
}

#[test]
fn a_store_with_no_sub_agents_is_written_as_before() {
    let f = Fixture::new();
    let path = f.write("s.jsonl", &claude_bytes());
    f.run(&[candidate(SOURCE_CLAUDE_CODE, &path)]);
    let text = f.rows_text();
    assert!(!text.contains("spawned"), "{text}");
}

fn run_family_skills(f: &Fixture, family: &Family) -> PassSummary {
    f.pass
        .run_pass_from(
            std::slice::from_ref(&family.parent),
            &family.spawned,
            &NeverFolders::NONE,
            now(),
            QUIET,
            &mut |_| (PROJECT.to_owned(), false),
            f.pass.cleared.load(Ordering::SeqCst),
            true,
            &|| false,
        )
        .unwrap()
        .unwrap()
}

#[test]
fn skills_backfill_children_separately_and_resolve_only_permitted_family() {
    let f = Fixture::new();
    let mut fam = family(&f, "skills");
    let invocation = serde_json::json!({
        "type":"assistant", "timestamp": now(),
        "message":{"content":[{"type":"tool_use","id":"skill-1","name":"Skill","input":{"skill":"review"}}]}
    });
    for candidate in std::iter::once(&mut fam.parent)
        .chain(fam.spawned.iter_mut().map(|child| &mut child.candidate))
    {
        let mut bytes = std::fs::read(&candidate.path).unwrap();
        bytes.extend_from_slice(format!("\n{invocation}\n").as_bytes());
        std::fs::write(&candidate.path, bytes).unwrap();
        candidate.size_bytes = std::fs::metadata(&candidate.path).unwrap().len();
    }
    run_family(&f, std::slice::from_ref(&fam.parent), &fam.spawned, false);
    let unknown = f.pass.skill_usage_value(true, &NeverFolders::NONE);
    assert_eq!(unknown["usage"]["delegated_sessions_unknown"], 2);
    let before = f.week(&[]);
    let backfill = run_family_skills(&f, &fam);
    assert_eq!(backfill.read, 1);
    assert_eq!(backfill.spawned_read, 2);
    let unchanged = run_family_skills(&f, &fam);
    assert_eq!(unchanged.read, 0);
    assert_eq!(unchanged.spawned_read, 0);
    let after = f.week(&[]);
    // Backfill replaces rows and intentionally invalidates their old opaque
    // references. Every accounting field must remain identical.
    let mut before_rollup = before["rollup"].clone();
    let mut after_rollup = after["rollup"].clone();
    for rollup in [&mut before_rollup, &mut after_rollup] {
        for session in rollup["sessions"].as_array_mut().unwrap() {
            assert!(
                session
                    .as_object_mut()
                    .unwrap()
                    .remove("session_ref")
                    .is_some()
            );
        }
    }
    assert_eq!(before_rollup, after_rollup);
    let usage = f.pass.skill_usage_value(true, &NeverFolders::NONE);
    assert_eq!(usage["usage"]["sessions_observed"], 1);
    assert_eq!(usage["usage"]["delegated_sessions_observed"], 2);
    assert_eq!(usage["usage"]["delegated_sessions_unknown"], 0);
    let rows = usage["usage"]["skills"].as_array().unwrap();
    let direct = rows
        .iter()
        .find(|row| row["population"] == "direct")
        .unwrap();
    let delegated = rows
        .iter()
        .find(|row| row["population"] == "delegated")
        .unwrap();
    assert_eq!(direct["invocations"], 1);
    assert_eq!(delegated["invocations"], 2);
    assert_eq!(
        delegated["parent_session_refs"][0],
        direct["session_refs"][0]
    );
    let reference = delegated["session_refs"][0].as_str().unwrap();
    assert!(reference.starts_with('c'));
    assert!(
        f.pass
            .drill_value(true, reference, &NeverFolders::NONE, utc())
            .is_ok()
    );
    assert!(
        f.pass
            .drill_value(true, reference, &NeverFolders::Every, utc())
            .is_err()
    );
    let key = f.pass.keys.load().unwrap().unwrap();
    let retained = store(&f);
    assert!(
        find_listed(
            &key,
            &retained,
            &never_digests(&key, &NeverFolders::NONE),
            reference
        )
        .is_none()
    );
    let mut orphaned = (*retained).clone();
    orphaned.rows.clear();
    f.pass.save(orphaned).unwrap();
    assert!(
        f.pass
            .drill_value(true, reference, &NeverFolders::NONE, utc())
            .is_err()
    );
    assert!(
        f.pass.skill_usage_value(true, &NeverFolders::NONE)["usage"]["skills"]
            .as_array()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn skills_deferred_children_are_unknown_until_their_own_read() {
    let f = Fixture::new();
    let mut fam = family(&f, "pending");
    fam.spawned[0].candidate.modified_at = now() - Duration::seconds(1);
    let summary = run_family_skills(&f, &fam);
    assert_eq!(summary.spawned_read, 1);
    let usage = f.pass.skill_usage_value(true, &NeverFolders::NONE);
    assert_eq!(usage["usage"]["delegated_sessions_observed"], 1);
    assert_eq!(usage["usage"]["delegated_sessions_unknown"], 1);
    assert_eq!(usage["usage"]["partial"], true);
}

#[test]
fn child_drill_rpc_requires_skills_before_enable_and_after_disable() {
    use crate::daemon::insights_sessions::handle_session_drill;
    use crate::daemon::ipc::Request;

    let f = Fixture::new();
    let fam = family(&f, "rpc-scope");
    run_family_skills(&f, &fam);
    let tag = f.pass.store_tag();
    let retained = store(&f);
    let parent = retained.rows.values().next().unwrap().listed_reference(tag);
    let child = format!(
        "c{}.{tag}",
        retained.spawned.values().next().unwrap().row.seq
    );
    let (_dir, config) = crate::config::tests_support::temp_store();
    let mut shared = DaemonShared::load(config).unwrap();
    shared.insights_counter = f.pass;
    shared.settings.lock().unwrap().insights_counter_pass = true;
    let registry = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "registry_version": 1,
        "flags": {"insights_skills": {"description": "Test skills", "owner": "tests", "class": "product",
            "default": false, "required_capabilities": [], "timing": "live"}}
    })).unwrap();
    let profile = serde_json::from_value(serde_json::json!({
        "schema_version": 1, "registry_version": 1, "environment": "staging",
        "capabilities": [], "remote_enable": ["insights_skills"], "signing_keys": []
    }))
    .unwrap();
    let flags_dir = tempfile::tempdir().unwrap();
    shared.feature_flags = crate::feature_flags::Runtime::open(
        registry,
        crate::feature_flags::test_signing::pin(profile),
        flags_dir.path(),
        now(),
    )
    .unwrap();
    let drill = |reference: &str| {
        handle_session_drill(
            &shared,
            &Request {
                id: 1,
                method: "insights_session_drill".into(),
                params: serde_json::json!({"session_ref": reference, "tz": 0}),
            },
        )
    };
    let accept = |revision, on| {
        let manifest = serde_json::to_vec(&serde_json::json!({
            "schema_version": 1, "environment": "staging", "revision": revision,
            "issued_at": now(), "expires_at": now() + Duration::hours(1),
            "flags": {"insights_skills": on}
        }))
        .unwrap();
        shared
            .feature_flags
            .accept(&crate::feature_flags::test_signing::sign(&manifest), now())
            .unwrap();
    };
    // The child exists for other local analytics, but a guessed reference
    // must not bypass a flag that has never been enabled.
    assert_eq!(
        drill(&child).error.unwrap().message,
        crate::daemon::insights_sessions::ERR_SESSION_UNKNOWN
    );
    assert!(drill(&parent).result.unwrap()["session"].is_object());
    accept(1, true);
    assert!(drill(&child).result.unwrap()["session"].is_object());
    accept(2, false);
    assert_eq!(
        drill(&child).error.unwrap().message,
        crate::daemon::insights_sessions::ERR_SESSION_UNKNOWN
    );
    assert!(drill(&parent).result.unwrap()["session"].is_object());
}
