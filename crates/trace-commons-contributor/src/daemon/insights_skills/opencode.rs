//! Ephemeral skill-only OpenCode observations. Nothing enters token/spend rows.
//! Only configured source adapters read disk; each load retains their byte and
//! SQLite instruction limits. No transcript text or native ID crosses IPC.
use super::*;
use crate::insights::skill_usage::{SKILL_USAGE_VERSION, SkillUsageEvidence, opencode_skill_usage};
use crate::source::{SOURCE_OPENCODE, SessionRef, all_sources};

const MAX_READS: usize = 16;
const MAX_ROWS: usize = 512;
const MAX_EVIDENCE_BYTES: usize = 8 * 1024 * 1024;
const MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone)]
pub(crate) struct OpenCodeSkillRow {
    pub reference: String,
    pub evidence: SkillUsageEvidence,
    // The normalized adapter does not yet expose session-level parent IDs.
    pub population: &'static str,
    pub parent_reference: Option<String>,
}

#[derive(Clone)]
struct Cached {
    fingerprint: (u64, Option<DateTime<Utc>>),
    native_id: String,
    latest_event: Option<DateTime<Utc>>,
    row: OpenCodeSkillRow,
}

#[derive(Default)]
pub(super) struct State {
    running: bool,
    pending: bool,
    scope: Option<Arc<SnapshotScope>>,
    roots: BTreeMap<&'static str, String>,
    files: BTreeMap<PathBuf, Cached>,
    partial: bool,
    completed_at: Option<DateTime<Utc>>,
}

/// Coalesce watcher polls: one running pass and at most one follow-up.
pub(crate) fn schedule(shared: &Arc<DaemonShared>) {
    if !enabled(shared) {
        if let Ok(mut state) = shared.insights_skills.opencode.lock() {
            state.files.clear();
            state.scope = None;
            state.completed_at = None;
            state.pending = false;
        }
        return;
    }
    let Ok(mut state) = shared.insights_skills.opencode.lock() else {
        return;
    };
    state.pending = true;
    if state.running {
        return;
    }
    state.running = true;
    drop(state);
    let shared = Arc::clone(shared);
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        drop(handle.spawn_blocking(move || worker(&shared)));
    } else {
        worker(&shared);
    }
}

fn worker(shared: &DaemonShared) {
    struct Reset<'a>(&'a DaemonShared);
    impl Drop for Reset<'_> {
        fn drop(&mut self) {
            if std::thread::panicking() {
                if let Ok(mut state) = self.0.insights_skills.opencode.lock() {
                    state.running = false;
                    state.pending = false;
                }
            }
        }
    }
    let _reset = Reset(shared);
    loop {
        {
            let Ok(mut state) = shared.insights_skills.opencode.lock() else {
                return;
            };
            if !state.pending {
                state.running = false;
                return;
            }
            state.pending = false;
        }
        scan(shared);
    }
}

fn fingerprint(reference: &SessionRef) -> (u64, Option<DateTime<Utc>>) {
    (
        reference.size_bytes,
        reference.group_modified_at.or_else(|| {
            std::fs::metadata(&reference.path)
                .ok()?
                .modified()
                .ok()
                .map(DateTime::from)
        }),
    )
}

fn same_scope(left: &SnapshotScope, right: &SnapshotScope) -> bool {
    left.flags == right.flags
        && left.cleared == right.cleared
        && left.analysis == right.analysis
        && left.policy == right.policy
}

fn permitted(scope: &SnapshotScope, cwd: Option<&str>) -> bool {
    if scope.policy.holds_every_send() {
        return false;
    }
    // Unknown folders cannot safely be checked against Never ancestors.
    let Some(cwd) = cwd.filter(|cwd| Path::new(cwd).is_absolute()) else {
        return false;
    };
    let path = Path::new(cwd);
    let canonical = path.canonicalize().ok();
    match super::super::insights_week::never_folders(&scope.policy) {
        super::super::insights_week::NeverFolders::Every => return false,
        super::super::insights_week::NeverFolders::These(keys) => {
            for key in keys {
                let Some(root) =
                    super::super::policy::display_path_for_key(&key).map(PathBuf::from)
                else {
                    return false;
                };
                if !root.is_absolute() {
                    return false;
                }
                if path.starts_with(&root) {
                    return false;
                }
                let Ok(root) = root.canonicalize() else {
                    return false;
                };
                let Some(canonical) = &canonical else {
                    return false;
                };
                if canonical.starts_with(root) {
                    return false;
                }
            }
        }
    }
    let key = super::super::policy::project_for(Some(cwd)).0;
    scope.policy.resolve(&key) != super::super::policy::ProjectMode::Ignore
}

fn scan(shared: &DaemonShared) {
    scan_with_sources(shared, all_sources);
}

fn scan_with_sources(
    shared: &DaemonShared,
    sources: impl FnOnce(&crate::source::SourceRoots) -> Vec<Box<dyn crate::source::TraceSource>>,
) {
    if !enabled(shared) {
        return;
    }
    let Some(scope) = SnapshotScope::capture(shared).map(Arc::new) else {
        return;
    };
    let roots = shared.source_roots_with_routing();
    let identities = roots.source_identities();
    let mut files = {
        let Ok(state) = shared.insights_skills.opencode.lock() else {
            return;
        };
        if state
            .scope
            .as_ref()
            .is_some_and(|old| same_scope(old, &scope))
            && state.roots == identities
        {
            state.files.clone()
        } else {
            BTreeMap::new()
        }
    };
    let cutoff = Utc::now()
        - chrono::Duration::weeks(crate::insights::analytics_constants::COUNTER_STORE_KEEP_WEEKS);
    let mut seen = BTreeSet::new();
    let mut reads = 0;
    let mut bytes = 0u64;
    let mut partial = match super::super::insights_week::never_folders(&scope.policy) {
        super::super::insights_week::NeverFolders::Every => true,
        super::super::insights_week::NeverFolders::These(keys) => keys.iter().any(|key| {
            super::super::policy::display_path_for_key(key)
                .is_none_or(|root| Path::new(&root).canonicalize().is_err())
        }),
    };
    for source in sources(&roots)
        .into_iter()
        .filter(|source| source.name() == SOURCE_OPENCODE)
    {
        let Ok(mut refs) = source.discover() else {
            partial = true;
            continue;
        };
        refs.sort_by_key(|reference| std::cmp::Reverse(fingerprint(reference).1));
        for reference in refs {
            let stamp = fingerprint(&reference);
            if stamp.1.is_none_or(|at| at < cutoff) {
                continue;
            }
            if reference
                .cwd
                .as_deref()
                .is_none_or(|cwd| !Path::new(cwd).is_absolute())
            {
                partial = true;
                continue;
            }
            partial |= reference
                .cwd
                .as_deref()
                .is_some_and(|cwd| Path::new(cwd).canonicalize().is_err());
            if !permitted(&scope, reference.cwd.as_deref()) {
                continue;
            }
            if seen.len() >= MAX_ROWS {
                partial = true;
                break;
            }
            seen.insert(reference.path.clone());
            if files.get(&reference.path).is_some_and(|old| {
                old.fingerprint == stamp && old.row.evidence.schema_version == SKILL_USAGE_VERSION
            }) {
                continue;
            }
            if reads >= MAX_READS || reference.size_bytes > MAX_BYTES.saturating_sub(bytes) {
                files.remove(&reference.path);
                partial = true;
                continue;
            }
            if !scope.is_current(shared) || !enabled(shared) {
                return;
            }
            reads += 1;
            bytes = bytes.saturating_add(reference.size_bytes);
            let Ok(transcript) = source.load(&reference) else {
                files.remove(&reference.path);
                partial = true;
                continue;
            };
            // Loading can block while the flag, local analysis, clear
            // generation or folder policy changes. Do not inspect or extract
            // the loaded transcript under a scope that is no longer current.
            if !scope.is_current(shared) || !enabled(shared) {
                return;
            }
            if !permitted(&scope, transcript.cwd.as_deref()) {
                files.remove(&reference.path);
                continue;
            }
            let Some(identity) = transcript
                .source_session
                .filter(|id| id.adapter == SOURCE_OPENCODE)
            else {
                files.remove(&reference.path);
                partial = true;
                continue;
            };
            // A read changing under us is retried next poll, never cached as settled.
            if source
                .session_at(&reference.path)
                .ok()
                .flatten()
                .as_ref()
                .map(fingerprint)
                != Some(stamp)
            {
                files.remove(&reference.path);
                partial = true;
                continue;
            }
            let evidence = opencode_skill_usage(&transcript.events);
            let latest_event = transcript
                .events
                .iter()
                .filter_map(|event| event.timestamp)
                .max();
            let opaque = files
                .values()
                .find(|old| old.native_id == identity.native_id)
                .map(|old| old.row.reference.clone())
                .unwrap_or_else(|| format!("os.{}", uuid::Uuid::new_v4()));
            files.insert(
                reference.path.clone(),
                Cached {
                    fingerprint: stamp,
                    native_id: identity.native_id,
                    latest_event,
                    row: OpenCodeSkillRow {
                        reference: opaque,
                        evidence,
                        population: "unknown",
                        parent_reference: None,
                    },
                },
            );
        }
    }
    // Discovery failures cannot preserve old observations as if current.
    files.retain(|path, _| seen.contains(path));
    let mut evidence_bytes = 0usize;
    files.retain(|_, cached| {
        evidence_bytes = evidence_bytes.saturating_add(
            serde_json::to_vec(&cached.row.evidence)
                .map_or(MAX_EVIDENCE_BYTES + 1, |bytes| bytes.len()),
        );
        let keep = evidence_bytes <= MAX_EVIDENCE_BYTES;
        partial |= !keep;
        keep
    });
    if !scope.is_current(shared)
        || !enabled(shared)
        || identities != shared.source_roots_with_routing().source_identities()
    {
        return;
    }
    if let Ok(mut state) = shared.insights_skills.opencode.lock() {
        state.files = files;
        state.scope = Some(scope);
        state.roots = identities;
        state.partial = partial;
        state.completed_at = Some(Utc::now());
    }
}

fn deduplicated(files: &BTreeMap<PathBuf, Cached>) -> (Vec<OpenCodeSkillRow>, bool) {
    let mut selected: BTreeMap<&str, &Cached> = BTreeMap::new();
    let mut partial = false;
    for entry in files.values() {
        match selected.get(entry.native_id.as_str()) {
            Some(old) => {
                // Native event timestamps, never export filesystem copy time.
                // Divergent snapshots are not added together: complementary
                // missing history remains visibly partial rather than inflated.
                partial |= old.row.evidence != entry.row.evidence;
                if entry.latest_event > old.latest_event {
                    selected.insert(&entry.native_id, entry);
                }
            }
            None => {
                selected.insert(&entry.native_id, entry);
            }
        }
    }
    (
        selected
            .into_values()
            .map(|entry| entry.row.clone())
            .collect(),
        partial,
    )
}

pub(crate) fn rows(shared: &DaemonShared) -> (Vec<OpenCodeSkillRow>, bool) {
    let (rows, partial, _) = snapshot(shared);
    (rows, partial)
}

/// Completion time is captured with the rows under the same scope; event time
/// cannot stand in for the time local analysis finished.
pub(crate) fn snapshot(
    shared: &DaemonShared,
) -> (Vec<OpenCodeSkillRow>, bool, Option<DateTime<Utc>>) {
    if !enabled(shared) {
        return (Vec::new(), true, None);
    }
    let Some(scope) = SnapshotScope::capture(shared) else {
        return (Vec::new(), true, None);
    };
    let roots = shared.source_roots_with_routing().source_identities();
    let result = {
        let Ok(state) = shared.insights_skills.opencode.lock() else {
            return (Vec::new(), true, None);
        };
        if !state
            .scope
            .as_ref()
            .is_some_and(|old| same_scope(old, &scope))
            || state.roots != roots
        {
            return (Vec::new(), true, None);
        }
        let (rows, duplicates_partial) = deduplicated(&state.files);
        (
            rows,
            state.partial || duplicates_partial,
            state.completed_at,
        )
    };
    if !scope.is_current(shared) {
        return (Vec::new(), true, None);
    }
    result
}

/// Evidence drill accepts only an issued opaque reference and rechecks scope.
pub(super) fn detail(shared: &DaemonShared, req: &Request) -> Response {
    let Some(reference) = req
        .params
        .get("session_ref")
        .and_then(Value::as_str)
        .filter(|reference| reference.starts_with("os.") && reference.len() <= 64)
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "skills-trace-ref-invalid");
    };
    let (rows, partial) = rows(shared);
    let Some(row) = rows.into_iter().find(|row| row.reference == reference) else {
        return Response::ok(
            req.id,
            json!({"session_ref":reference,"status":"unavailable"}),
        );
    };
    Response::ok(
        req.id,
        json!({"enabled":true,"readable":true,"session_ref":reference,"agent":"opencode","population":row.population,"parent_session_ref":row.parent_reference,"skill_usage":row.evidence,"partial":partial}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    fn cached(id: &str, at: i64, invocation: u32) -> Cached {
        let mut evidence = opencode_skill_usage(&[]);
        evidence
            .skills
            .push(crate::insights::skill_usage::SkillUsageCount {
                name: "review".into(),
                invocations: invocation,
                loads: 0,
                last_seen: None,
                events: vec![],
            });
        Cached {
            fingerprint: (0, None),
            native_id: id.into(),
            latest_event: DateTime::from_timestamp(at, 0),
            row: OpenCodeSkillRow {
                reference: format!("os.{id}"),
                evidence,
                population: "unknown",
                parent_reference: None,
            },
        }
    }
    // Delegate real parsing, then change the daemon scope at the return from
    // a potentially slow load. Follow-up I/O must not run under that old scope.
    struct ChangingSource {
        inner: Box<dyn crate::source::TraceSource>,
        after_load: Arc<dyn Fn() + Send + Sync>,
        loads: Arc<std::sync::atomic::AtomicUsize>,
        followups: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl crate::source::TraceSource for ChangingSource {
        fn name(&self) -> &'static str {
            self.inner.name()
        }

        fn discover(&self) -> anyhow::Result<Vec<SessionRef>> {
            self.inner.discover()
        }

        fn load(&self, reference: &SessionRef) -> anyhow::Result<crate::source::SessionTranscript> {
            let transcript = self.inner.load(reference)?;
            self.loads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (self.after_load)();
            Ok(transcript)
        }

        fn session_at(&self, path: &Path) -> anyhow::Result<Option<SessionRef>> {
            self.followups
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.inner.session_at(path)
        }
    }

    #[test]
    fn scope_change_during_load_stops_before_extraction_and_followup_io() {
        fn accept(shared: &DaemonShared, revision: u64, on: bool) {
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

        for change in [
            "unchanged",
            "disabled",
            "reenabled",
            "cleared",
            "analysis",
            "policy",
        ] {
            let (_dir, store) = crate::config::tests_support::temp_store();
            let mut shared = DaemonShared::load(store).unwrap();
            let registry = serde_json::from_value(json!({
                "schema_version":1,"registry_version":1,
                "flags":{"insights_skills":{"description":"Skills","owner":"tests","class":"product",
                    "default":false,"required_capabilities":[],"timing":"live"}}
            })).unwrap();
            let profile = serde_json::from_value(json!({
                "schema_version":1,"registry_version":1,"environment":"testing",
                "capabilities":[],"remote_enable":["insights_skills"],"signing_keys":[]
            }))
            .unwrap();
            shared.feature_flags = crate::feature_flags::Runtime::open(
                registry,
                crate::feature_flags::test_signing::pin(profile),
                shared.store.dir(),
                Utc::now(),
            )
            .unwrap();
            accept(&shared, 1, true);
            let shared = Arc::new(shared);
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
            let changing = Arc::clone(&shared);
            let after_load: Arc<dyn Fn() + Send + Sync> = Arc::new(move || match change {
                "disabled" => accept(&changing, 2, false),
                "reenabled" => {
                    accept(&changing, 2, false);
                    accept(&changing, 3, true);
                }
                "cleared" => changing.insights_counter.clear().unwrap(),
                "analysis" => changing.settings.lock().unwrap().insights_counter_pass = false,
                "policy" => changing
                    .policy
                    .lock()
                    .unwrap()
                    .set_mode(
                        cwd.to_str().unwrap(),
                        super::super::super::policy::ProjectMode::Ignore,
                        Utc::now(),
                    )
                    .unwrap(),
                _ => {}
            });
            let loads = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let followups = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            scan_with_sources(&shared, |roots| {
                all_sources(roots)
                    .into_iter()
                    .map(|inner| {
                        Box::new(ChangingSource {
                            inner,
                            after_load: Arc::clone(&after_load),
                            loads: Arc::clone(&loads),
                            followups: Arc::clone(&followups),
                        }) as Box<dyn crate::source::TraceSource>
                    })
                    .collect()
            });
            assert_eq!(
                loads.load(std::sync::atomic::Ordering::SeqCst),
                1,
                "{change}: real export loaded"
            );
            assert_eq!(
                followups.load(std::sync::atomic::Ordering::SeqCst),
                usize::from(change == "unchanged"),
                "{change}: stale scope must abort immediately after load, before metadata I/O or extraction"
            );
            let state = shared.insights_skills.opencode.lock().unwrap();
            assert_eq!(
                state.completed_at.is_some(),
                change == "unchanged",
                "{change}"
            );
            assert_eq!(
                state.files.len(),
                usize::from(change == "unchanged"),
                "{change}"
            );
            if change == "unchanged" {
                assert_eq!(
                    state.files.values().next().unwrap().row.evidence.skills[0].name,
                    "review"
                );
            }
        }
    }

    #[test]
    fn copies_deduplicate_by_native_identity_and_event_freshness() {
        let mut files = BTreeMap::new();
        let mut stale_export = cached("same", 10, 1);
        stale_export.fingerprint.1 = DateTime::from_timestamp(999, 0);
        files.insert(PathBuf::from("export.json"), stale_export);
        files.insert(PathBuf::from("opencode.db/same"), cached("same", 20, 2));
        let (rows, partial) = deduplicated(&files);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].evidence.skills[0].invocations, 2);
        assert!(partial);
        files.insert(PathBuf::from("export.json"), cached("same", 30, 3));
        assert_eq!(deduplicated(&files).0[0].evidence.skills[0].invocations, 3);
    }
    #[test]
    fn identical_copies_are_complete_and_distinct_native_traces_remain_distinct() {
        let files = BTreeMap::from([
            (PathBuf::from("export.json"), cached("same", 10, 1)),
            (PathBuf::from("database"), cached("same", 10, 1)),
            (PathBuf::from("other.json"), cached("other", 10, 1)),
        ]);
        let (rows, partial) = deduplicated(&files);
        assert_eq!(rows.len(), 2);
        assert!(!partial);
    }
    #[test]
    fn missing_folder_and_changed_generation_fail_closed() {
        let mut scope = SnapshotScope {
            flags: 1,
            cleared: 0,
            analysis: true,
            policy: super::super::super::policy::ProjectPolicy::default(),
        };
        assert!(!permitted(&scope, None));
        assert!(!permitted(&scope, Some("relative")));
        let original = SnapshotScope {
            flags: scope.flags,
            cleared: scope.cleared,
            analysis: scope.analysis,
            policy: scope.policy.clone(),
        };
        assert!(same_scope(&scope, &original));
        scope.cleared += 1;
        assert!(!same_scope(&scope, &original));
        scope.cleared = 0;
        scope.flags += 1;
        assert!(!same_scope(&scope, &original));
        scope.flags = 1;
        scope.analysis = false;
        assert!(!same_scope(&scope, &original));
    }
}
