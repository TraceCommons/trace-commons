//! What collecting a session's trace needs, kept privately while the tool
//! runs, so the daemon can collect it when the launcher is gone (terminal
//! window closed, force quit, logout).
//!
//! The launcher writes one plan per collected row right after the tool
//! starts. Whoever collects first, the launcher after the tool exits or the
//! daemon after the row's lease lapsed and the tool's process is gone,
//! takes the row's claim (`create_new` of `<row>.claim`); the other does
//! nothing. The plan and its claim are removed once collection has a final
//! result. A plan holds paths and ids only, never transcript content.

use super::{
    AccountId, ManagedError, ToolId,
    opencode::Version,
    profiles::private_directory,
    sessions::{CollectionState, SessionState},
};
use crate::config::ConfigStore;
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
};
use uuid::Uuid;

pub const DIRECTORY: &str = "managed-collection-plans";
const SCHEMA_VERSION: u32 = 1;
/// A Claude Code profile's snapshot lists every transcript already there,
/// so a plan can be large; it is still bounded.
const MAX_PLAN: u64 = 16 * 1024 * 1024;
const MAX_CLAIM: u64 = 1024;

/// How long after launch the daemon keeps waiting for a tool whose launcher
/// is gone. Past it, collection is recorded as failed and the local session
/// is left as it is.
pub const GIVE_UP_AFTER: Duration = Duration::hours(24);

/// A process, named by its pid and start time so a reused pid is never
/// mistaken for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessIdentity {
    pub pid: u32,
    /// Seconds since the epoch, as `sysinfo` reports it. None when the
    /// process could not be read at the time; such a process counts as
    /// running while any process holds its pid.
    pub start_time: Option<u64>,
}

impl ProcessIdentity {
    pub fn of(pid: u32) -> Self {
        let mut system = sysinfo::System::new();
        let key = sysinfo::Pid::from_u32(pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[key]), true);
        Self {
            pid,
            start_time: system.process(key).map(|process| process.start_time()),
        }
    }

    pub fn current() -> Self {
        Self::of(std::process::id())
    }

    /// The launcher's `process_identity` string (`<pid>:<start time>`).
    pub fn label(&self) -> Option<String> {
        self.start_time.map(|start| format!("{}:{start}", self.pid))
    }

    /// Whether this process still runs. A zombie has exited. Another
    /// process holding the pid with a different start time is not it.
    pub fn is_running(&self) -> bool {
        let mut system = sysinfo::System::new();
        let key = sysinfo::Pid::from_u32(self.pid);
        system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[key]), true);
        let Some(process) = system.process(key) else {
            return false;
        };
        if process.status() == sysinfo::ProcessStatus::Zombie {
            return false;
        }
        self.start_time
            .is_none_or(|start| process.start_time() == start)
    }
}

/// Claude Code and Codex: what `NativeCollection::snapshot` took before the
/// tool started. The profile directory is derived again from the account,
/// never read from here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativePlan {
    /// The profile's transcripts before launch, relative to it. None when
    /// the profile could not be read then (collection then fails).
    pub before: Option<BTreeSet<PathBuf>>,
    /// The launch prepared a Codex binding under its own row id
    /// (`managed-native-bindings/<row>`, derived again on use).
    pub codex_bound: bool,
}

/// OpenCode: enough to run its export again. The session's directory is
/// derived again from the account and `session`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodePlan {
    pub native: PathBuf,
    /// The OpenCode session's own id: the row's, or for a resumed row its
    /// root's.
    pub session: Uuid,
    pub version: Version,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CollectionPlan {
    pub schema_version: u32,
    pub session_id: Uuid,
    pub tool: ToolId,
    pub account_id: AccountId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resume_root: Option<Uuid>,
    pub cwd: PathBuf,
    pub collect_traces: bool,
    pub created_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub native: Option<NativePlan>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opencode: Option<OpenCodePlan>,
    /// The tool's process.
    pub process: ProcessIdentity,
}

fn directory(store: &ConfigStore) -> PathBuf {
    store.dir().join(DIRECTORY)
}

fn plan_path(store: &ConfigStore, id: Uuid) -> PathBuf {
    directory(store).join(format!("{id}.json"))
}

fn claim_path(store: &ConfigStore, id: Uuid) -> PathBuf {
    directory(store).join(format!("{id}.claim"))
}

fn create_private(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn read_regular(path: &Path, limit: u64) -> Option<Vec<u8>> {
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Write `plan` owner-only, all at once: a reader sees the whole plan or
/// none. Refuses a plan for a row that already has one.
pub fn write(store: &ConfigStore, plan: &CollectionPlan) -> Result<(), ManagedError> {
    if !plan.collect_traces {
        return Err(ManagedError::InvalidRequest);
    }
    let root = directory(store);
    private_directory(&root)?;
    let target = plan_path(store, plan.session_id);
    if std::fs::symlink_metadata(&target).is_ok() {
        return Err(ManagedError::Conflict);
    }
    let bytes = serde_json::to_vec(plan).map_err(|_| ManagedError::StorageUnavailable)?;
    // A plan no reader would accept is never written: its row's launcher
    // then collects for itself, with no fallback.
    if bytes.len() as u64 > MAX_PLAN {
        return Err(ManagedError::StorageUnavailable);
    }
    let partial = root.join(format!(
        "{}.{}.partial",
        plan.session_id,
        Uuid::new_v4().simple()
    ));
    let written = (|| {
        let mut file = create_private(&partial)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        std::fs::rename(&partial, &target)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&partial);
        return Err(ManagedError::StorageUnavailable);
    }
    Ok(())
}

/// The plan of row `id`, when one is there and reads as this build's.
pub fn read(store: &ConfigStore, id: Uuid) -> Option<CollectionPlan> {
    let plan: CollectionPlan =
        serde_json::from_slice(&read_regular(&plan_path(store, id), MAX_PLAN)?).ok()?;
    (plan.session_id == id && plan.schema_version == SCHEMA_VERSION).then_some(plan)
}

pub fn exists(store: &ConfigStore, id: Uuid) -> bool {
    std::fs::symlink_metadata(plan_path(store, id)).is_ok()
}

/// Every row with a plan.
pub fn list(store: &ConfigStore) -> Vec<Uuid> {
    let root = directory(store);
    if !std::fs::symlink_metadata(&root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return vec![];
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return vec![];
    };
    let mut ids = entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name();
            let id = name.to_str()?.strip_suffix(".json")?;
            Uuid::parse_str(id).ok()
        })
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

/// Take row `id`'s claim for `claimant`. True for the one caller that took
/// it; false when someone already holds it.
pub fn claim(
    store: &ConfigStore,
    id: Uuid,
    claimant: &ProcessIdentity,
) -> Result<bool, ManagedError> {
    let root = directory(store);
    private_directory(&root)?;
    let path = claim_path(store, id);
    let bytes = serde_json::to_vec(claimant).map_err(|_| ManagedError::StorageUnavailable)?;
    // The claimant is written whole first, then linked into place, which
    // fails when the claim exists: the claim is never seen without its
    // holder, so a reader never takes a claim being written for a dead one.
    let partial = root.join(format!("{id}.{}.claiming", Uuid::new_v4().simple()));
    let written = (|| {
        let mut file = create_private(&partial)?;
        file.write_all(&bytes)?;
        file.sync_all()
    })();
    let linked = written.and_then(|()| std::fs::hard_link(&partial, &path));
    let _ = std::fs::remove_file(&partial);
    match linked {
        Ok(()) => Ok(true),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => Ok(false),
        Err(_) => Err(ManagedError::StorageUnavailable),
    }
}

/// Who holds row `id`'s claim: None when nobody does, Some(None) when its
/// holder cannot be read.
pub fn claimant(store: &ConfigStore, id: Uuid) -> Option<Option<ProcessIdentity>> {
    let path = claim_path(store, id);
    std::fs::symlink_metadata(&path).ok()?;
    Some(read_regular(&path, MAX_CLAIM).and_then(|bytes| serde_json::from_slice(&bytes).ok()))
}

/// Collection of row `id` has a final result: remove its plan, then its
/// claim, so a plan is never seen without the claim that guards it.
pub fn finish(store: &ConfigStore, id: Uuid) {
    let _ = std::fs::remove_file(plan_path(store, id));
    let _ = std::fs::remove_file(claim_path(store, id));
}

/// Run the collection `plan` describes, with the same code and the same
/// confinement as a launcher's: only the bound or newly created
/// conversation, or a resumed row's root conversation, from the account's
/// own profile, staged under the row's id. The tool must have exited.
pub fn collect(store: &ConfigStore, plan: &CollectionPlan) -> Result<(), ManagedError> {
    if !plan.collect_traces {
        return Err(ManagedError::InvalidRequest);
    }
    match (plan.tool, &plan.native, &plan.opencode) {
        (ToolId::Claude | ToolId::Codex, Some(native), None) => {
            let collection = super::native_collection::NativeCollection::from_plan(
                store,
                plan.tool,
                plan.account_id,
                plan.session_id,
                native,
            )?;
            match plan.resume_root {
                Some(root) => collection.continuing(root),
                None => collection,
            }
            .collect(store)
        }
        (ToolId::Opencode, None, Some(opencode)) => {
            super::opencode::OpenCodeSession::reopen(store, plan.account_id, opencode, &plan.cwd)?
                .collect_as(store, plan.session_id)
        }
        _ => Err(ManagedError::InvalidRequest),
    }
}

/// Whether the trace of `plan`'s row is already staged: published where its
/// tool's collection publishes it, which happens only once it verified.
pub fn staged(store: &ConfigStore, plan: &CollectionPlan) -> bool {
    match plan.tool {
        ToolId::Claude | ToolId::Codex => {
            super::native_collection::published(store, plan.tool, plan.session_id)
        }
        ToolId::Opencode => super::opencode::exported(store, plan.session_id),
    }
}

/// What the registry says about a row with a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RowStatus {
    pub state: SessionState,
    pub collect_traces: bool,
    pub collection: CollectionState,
    pub tool: ToolId,
    pub account_id: AccountId,
    /// The root row whose conversation the row continues, if any.
    pub resume_of: Option<Uuid>,
}

/// What the daemon does next with a plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Next {
    /// Nothing yet: the launcher is still reporting, someone else is
    /// collecting, or the tool still runs.
    Wait,
    /// The tool is gone and nobody collected: claim and collect.
    Collect,
    /// Record collection as failed, then remove the plan: the tool ran past
    /// [`GIVE_UP_AFTER`], whoever claimed it died before staging its trace,
    /// or this build cannot read the plan ([`unreadable`]).
    Fail,
    /// Record collection as done, then remove the plan: whoever claimed it
    /// died after staging the row's trace, before recording the result.
    Collected,
    /// Remove the plan and change nothing: the row is gone, never
    /// collected, or already has its result.
    Discard,
}

/// What the daemon does with a row's plan that the row alone decides, so
/// the plan need not be read: Discard for a row that is gone, does not
/// collect or has its result; Wait for one whose launcher still reports,
/// and collects for itself whatever its claim looks like.
pub fn before_reading(row: Option<RowStatus>) -> Option<Next> {
    let Some(row) = row else {
        return Some(Next::Discard);
    };
    if !row.collect_traces || row.collection != CollectionState::Pending {
        return Some(Next::Discard);
    }
    if matches!(row.state, SessionState::Starting | SessionState::Running) {
        return Some(Next::Wait);
    }
    None
}

/// The daemon's decision for a plan this build cannot read: another
/// schema, too large, corrupt, or a link. Nobody can collect from it, so
/// its row's collection fails, unless a live holder of its claim is still
/// collecting under it.
pub fn unreadable(row: Option<RowStatus>, claim: Option<bool>) -> Next {
    if let Some(decided) = before_reading(row) {
        return decided;
    }
    if claim == Some(true) {
        Next::Wait
    } else {
        Next::Fail
    }
}

/// The daemon's decision for `plan`. `row` is the registry's view of its
/// row; `tool_running`, `claim` (who holds the claim and whether that
/// process runs) and `staged` (whether the row's trace is already staged,
/// [`staged`]) are read from the OS by the caller.
pub fn next(
    plan: &CollectionPlan,
    row: Option<RowStatus>,
    tool_running: bool,
    claim: Option<bool>,
    staged: bool,
    now: DateTime<Utc>,
) -> Next {
    if let Some(decided) = before_reading(row) {
        return decided;
    }
    let Some(row) = row else {
        return Next::Discard;
    };
    if !plan.collect_traces {
        return Next::Discard;
    }
    // A plan collects only the conversation its row launched on: the row's
    // own tool and account, and its own root.
    let session = row.resume_of.unwrap_or(plan.session_id);
    if plan.tool != row.tool
        || plan.account_id != row.account_id
        || plan.resume_root != row.resume_of
        || plan
            .opencode
            .as_ref()
            .is_some_and(|opencode| opencode.session != session)
    {
        return Next::Discard;
    }
    match claim {
        Some(true) => return Next::Wait,
        Some(false) if staged => return Next::Collected,
        Some(false) => return Next::Fail,
        None => {}
    }
    if tool_running {
        if now.signed_duration_since(plan.created_at) > GIVE_UP_AFTER {
            return Next::Fail;
        }
        return Next::Wait;
    }
    Next::Collect
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT: AccountId = AccountId(Uuid::from_u128(7));

    fn plan(id: Uuid) -> CollectionPlan {
        CollectionPlan {
            schema_version: SCHEMA_VERSION,
            session_id: id,
            tool: ToolId::Claude,
            account_id: ACCOUNT,
            resume_root: None,
            cwd: PathBuf::from("/tmp"),
            collect_traces: true,
            created_at: Utc::now(),
            native: Some(NativePlan {
                before: Some(BTreeSet::from([PathBuf::from("p/old.jsonl")])),
                codex_bound: false,
            }),
            opencode: None,
            process: ProcessIdentity {
                pid: 1,
                start_time: Some(2),
            },
        }
    }

    fn row(state: SessionState) -> Option<RowStatus> {
        Some(RowStatus {
            state,
            collect_traces: true,
            collection: CollectionState::Pending,
            tool: ToolId::Claude,
            account_id: ACCOUNT,
            resume_of: None,
        })
    }

    #[test]
    fn a_plan_is_written_owner_only_and_reads_back() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        let written = plan(id);
        write(&store, &written).unwrap();
        assert_eq!(read(&store, id), Some(written.clone()));
        assert_eq!(list(&store), vec![id]);
        assert!(exists(&store, id));
        assert_eq!(
            write(&store, &written),
            Err(ManagedError::Conflict),
            "one plan per row"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = |path: &Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode(&plan_path(&store, id)), 0o600);
            assert_eq!(mode(&directory(&store)), 0o700);
        }
        // Nothing but the plan is left behind.
        assert_eq!(std::fs::read_dir(directory(&store)).unwrap().count(), 1);
        finish(&store, id);
        assert_eq!(read(&store, id), None);
        assert!(list(&store).is_empty());
    }

    #[test]
    fn no_plan_is_written_for_a_row_that_does_not_collect() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        let off = CollectionPlan {
            collect_traces: false,
            ..plan(id)
        };
        assert_eq!(write(&store, &off), Err(ManagedError::InvalidRequest));
        assert!(!exists(&store, id));
        assert_eq!(collect(&store, &off), Err(ManagedError::InvalidRequest));
    }

    #[cfg(unix)]
    #[test]
    fn a_substituted_plan_directory_or_file_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().join("state")).unwrap();
        let elsewhere = home.path().join("elsewhere");
        std::fs::create_dir(&elsewhere).unwrap();
        std::os::unix::fs::symlink(&elsewhere, directory(&store)).unwrap();
        assert_eq!(
            write(&store, &plan(Uuid::new_v4())),
            Err(ManagedError::StorageUnavailable)
        );
        assert!(list(&store).is_empty());
        std::fs::remove_file(directory(&store)).unwrap();

        let id = Uuid::new_v4();
        std::fs::create_dir(directory(&store)).unwrap();
        let real = elsewhere.join("plan.json");
        std::fs::write(&real, serde_json::to_vec(&plan(id)).unwrap()).unwrap();
        std::os::unix::fs::symlink(&real, plan_path(&store, id)).unwrap();
        assert_eq!(read(&store, id), None, "a linked plan is never read");
    }

    #[test]
    fn a_plan_naming_another_row_is_not_read() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        write(&store, &plan(id)).unwrap();
        let other = Uuid::new_v4();
        std::fs::rename(plan_path(&store, id), plan_path(&store, other)).unwrap();
        assert_eq!(read(&store, other), None);
    }

    #[test]
    fn only_one_of_launcher_and_daemon_takes_the_claim() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        write(&store, &plan(id)).unwrap();
        let launcher = ProcessIdentity {
            pid: 10,
            start_time: Some(1),
        };
        let daemon = ProcessIdentity::current();
        assert_eq!(claim(&store, id, &launcher), Ok(true));
        assert_eq!(
            claim(&store, id, &daemon),
            Ok(false),
            "the loser does nothing"
        );
        assert_eq!(claimant(&store, id), Some(Some(launcher)));
        finish(&store, id);
        assert_eq!(claimant(&store, id), None);
        assert!(!exists(&store, id));

        // Racing threads: exactly one wins.
        let id = Uuid::new_v4();
        write(&store, &plan(id)).unwrap();
        let wins = std::thread::scope(|scope| {
            let handles = (0..8)
                .map(|_| scope.spawn(|| claim(&store, id, &daemon).unwrap()))
                .collect::<Vec<_>>();
            handles
                .into_iter()
                .map(|h| h.join().unwrap())
                .filter(|won| *won)
                .count()
        });
        assert_eq!(wins, 1);
    }

    #[cfg(unix)]
    #[test]
    fn a_reused_pid_is_not_the_tool() {
        let me = ProcessIdentity::current();
        assert!(me.is_running());
        assert!(me.start_time.is_some());
        let reused = ProcessIdentity {
            start_time: me.start_time.map(|start| start + 1),
            ..me
        };
        assert!(!reused.is_running());
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert!(
            !ProcessIdentity {
                pid,
                start_time: Some(0)
            }
            .is_running()
        );
    }

    #[test]
    fn the_daemon_collects_only_a_lapsed_rows_exited_tool_with_no_claim() {
        let now = Utc::now();
        let p = plan(Uuid::new_v4());
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, None, false, now),
            Next::Collect
        );
        assert_eq!(
            next(&p, row(SessionState::Failed), false, None, false, now),
            Next::Collect
        );
        assert_eq!(
            next(&p, row(SessionState::Exited), false, None, false, now),
            Next::Collect
        );
        // Never while the tool runs, nor while its launcher still reports.
        assert_eq!(
            next(&p, row(SessionState::Unknown), true, None, false, now),
            Next::Wait
        );
        assert_eq!(
            next(&p, row(SessionState::Running), false, None, false, now),
            Next::Wait
        );
        assert_eq!(
            next(&p, row(SessionState::Starting), false, None, false, now),
            Next::Wait
        );
        // A held claim waits; a dead holder's is a failure.
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, Some(true), false, now),
            Next::Wait
        );
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, Some(false), false, now),
            Next::Fail
        );
        // Bounded.
        let later = now + GIVE_UP_AFTER + Duration::minutes(1);
        assert_eq!(
            next(&p, row(SessionState::Unknown), true, None, false, later),
            Next::Fail
        );
        // Never for a row that does not collect, is gone, or has its result.
        assert_eq!(next(&p, None, false, None, false, now), Next::Discard);
        let off = Some(RowStatus {
            collect_traces: false,
            collection: CollectionState::Off,
            ..row(SessionState::Unknown).unwrap()
        });
        assert_eq!(next(&p, off, false, None, false, now), Next::Discard);
        let done = Some(RowStatus {
            collection: CollectionState::Collected,
            ..row(SessionState::Exited).unwrap()
        });
        assert_eq!(next(&p, done, false, None, false, now), Next::Discard);
        let off_plan = CollectionPlan {
            collect_traces: false,
            ..p.clone()
        };
        assert_eq!(
            next(&off_plan, row(SessionState::Unknown), false, None, false, now),
            Next::Discard
        );
        // A launcher that still reports is waited for, even when its claim
        // cannot be told from a dead holder's.
        assert_eq!(
            next(&p, row(SessionState::Running), false, Some(false), false, now),
            Next::Wait
        );
        assert_eq!(
            next(&p, row(SessionState::Starting), false, Some(false), false, now),
            Next::Wait
        );
    }

    /// A holder that died after staging the row's trace, before it could
    /// record the result: the row is collected, not failed.
    #[test]
    fn a_dead_holder_that_staged_the_trace_is_recorded_as_collected() {
        let now = Utc::now();
        let p = plan(Uuid::new_v4());
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, Some(false), true, now),
            Next::Collected
        );
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, Some(false), false, now),
            Next::Fail
        );
        assert_eq!(
            next(&p, row(SessionState::Unknown), false, Some(true), true, now),
            Next::Wait,
            "a live holder is still finishing"
        );
    }

    /// The row's staged output is found where each tool's collection
    /// publishes it, and only there.
    #[test]
    fn staged_output_is_found_where_each_tool_publishes_it() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        let claude = plan(id);
        assert!(!staged(&store, &claude));
        let pending = store
            .dir()
            .join(super::super::native_collection::STAGING)
            .join("claude");
        std::fs::create_dir_all(pending.join(format!("{id}.pending"))).unwrap();
        assert!(!staged(&store, &claude), "a collection still being verified");
        std::fs::create_dir_all(pending.join(id.to_string())).unwrap();
        assert!(staged(&store, &claude));
        let codex = CollectionPlan {
            tool: ToolId::Codex,
            ..plan(id)
        };
        assert!(!staged(&store, &codex), "another tool's output");

        let opencode = CollectionPlan {
            tool: ToolId::Opencode,
            native: None,
            opencode: Some(OpenCodePlan {
                native: PathBuf::from("/usr/bin/opencode"),
                session: id,
                version: Version::V1,
            }),
            ..plan(id)
        };
        assert!(!staged(&store, &opencode));
        let exports = store.dir().join(super::super::opencode::STAGING);
        std::fs::create_dir_all(&exports).unwrap();
        std::fs::write(exports.join(format!("{id}.json")), b"{}").unwrap();
        assert!(staged(&store, &opencode));
    }

    /// A plan this build cannot read fails its row's collection, so the
    /// row does not wait for a result forever, unless a live holder is
    /// still collecting under it. A row that needs no result drops it.
    #[test]
    fn an_unreadable_plan_fails_its_rows_collection() {
        assert_eq!(unreadable(row(SessionState::Unknown), None), Next::Fail);
        assert_eq!(unreadable(row(SessionState::Failed), Some(false)), Next::Fail);
        assert_eq!(unreadable(row(SessionState::Unknown), Some(true)), Next::Wait);
        assert_eq!(unreadable(row(SessionState::Running), None), Next::Wait);
        assert_eq!(unreadable(None, None), Next::Discard);
        let done = Some(RowStatus {
            collection: CollectionState::Collected,
            ..row(SessionState::Exited).unwrap()
        });
        assert_eq!(unreadable(done, None), Next::Discard);
    }

    /// A plan larger than a reader accepts is never written, so the
    /// launcher collects for itself instead.
    #[test]
    fn a_plan_too_large_to_read_back_is_not_written() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        let name = "p/".to_owned() + &"x".repeat(1000);
        let before = (0..(MAX_PLAN as usize / 1000 + 1))
            .map(|n| PathBuf::from(format!("{name}{n}.jsonl")))
            .collect();
        let large = CollectionPlan {
            native: Some(NativePlan {
                before: Some(before),
                codex_bound: false,
            }),
            ..plan(id)
        };
        assert_eq!(write(&store, &large), Err(ManagedError::StorageUnavailable));
        assert!(!exists(&store, id));
    }

    /// A plan whose tool, account or conversation is not its row's is
    /// discarded, never collected under that row.
    #[test]
    fn a_plan_that_disagrees_with_its_row_is_discarded() {
        let now = Utc::now();
        let id = Uuid::new_v4();
        let root = Uuid::new_v4();
        let p = plan(id);
        let lapsed = row(SessionState::Unknown).unwrap();
        for disagreeing in [
            RowStatus {
                tool: ToolId::Codex,
                ..lapsed
            },
            RowStatus {
                account_id: AccountId(Uuid::new_v4()),
                ..lapsed
            },
            RowStatus {
                resume_of: Some(root),
                ..lapsed
            },
        ] {
            assert_eq!(
                next(&p, Some(disagreeing), false, None, false, now),
                Next::Discard,
                "{disagreeing:?}"
            );
        }
        let resumed = CollectionPlan {
            resume_root: Some(root),
            ..plan(id)
        };
        assert_eq!(
            next(&resumed, Some(lapsed), false, None, false, now),
            Next::Discard
        );
        let resumed_row = RowStatus {
            resume_of: Some(root),
            ..lapsed
        };
        assert_eq!(
            next(&resumed, Some(resumed_row), false, None, false, now),
            Next::Collect
        );

        let opencode = |session| CollectionPlan {
            tool: ToolId::Opencode,
            native: None,
            opencode: Some(OpenCodePlan {
                native: PathBuf::from("/usr/bin/opencode"),
                session,
                version: Version::V1,
            }),
            ..plan(id)
        };
        let opencode_row = RowStatus {
            tool: ToolId::Opencode,
            ..lapsed
        };
        assert_eq!(
            next(&opencode(id), Some(opencode_row), false, None, false, now),
            Next::Collect
        );
        assert_eq!(
            next(&opencode(root), Some(opencode_row), false, None, false, now),
            Next::Discard,
            "another conversation"
        );
        let resumed_opencode = CollectionPlan {
            resume_root: Some(root),
            ..opencode(root)
        };
        assert_eq!(
            next(
                &resumed_opencode,
                Some(RowStatus {
                    resume_of: Some(root),
                    ..opencode_row
                }),
                false,
                None,
                false, now
            ),
            Next::Collect
        );
    }

    /// A claim is never seen without the claimant that holds it, and taking
    /// it leaves nothing else behind.
    #[test]
    fn a_claim_names_its_holder_from_the_start() {
        let home = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(home.path().to_path_buf()).unwrap();
        let id = Uuid::new_v4();
        write(&store, &plan(id)).unwrap();
        let holder = ProcessIdentity {
            pid: 10,
            start_time: Some(1),
        };
        assert_eq!(claim(&store, id, &holder), Ok(true));
        assert_eq!(claimant(&store, id), Some(Some(holder)));
        let mut names = std::fs::read_dir(directory(&store))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().into_string().unwrap())
            .collect::<Vec<_>>();
        names.sort();
        assert_eq!(names, [format!("{id}.claim"), format!("{id}.json")]);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(claim_path(&store, id))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
