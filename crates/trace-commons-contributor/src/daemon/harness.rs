//! Listing the coding tools on this machine, and connecting them one at a
//! time.
//!
//! Almost nothing here is invented. `ironwire_agents::tools` already answers
//! "which tools does this machine have, and are they pointed at us", already
//! plans an edit before making it, and already carries the three rules that
//! make editing a file we do not own acceptable at all:
//!
//! - **Never rewrite a file we cannot parse.** A contributor's own syntax
//!   error must not come back looking like ours.
//! - **Fill an empty slot; leave a full one alone.** A value already in the
//!   key is another destination or a deliberate choice; it is reported, never
//!   overwritten.
//! - **Remove only what we put there.**
//!
//! This module carries those rules across the socket without letting a shell
//! route around them. In particular:
//!
//! # Plan and commit are two calls, and there is no third
//!
//! `harness_plan` works out the edit and writes nothing. `harness_commit`
//! takes a plan id and nothing else -- no tool id, no action, no port -- so a
//! shell cannot ask for a write it has not first been shown. There is
//! deliberately no method that does both. `ironwire_agents::Planned` keeps the
//! file contents it would write in private fields, so a plan cannot be
//! reconstructed from what crossed the socket either; it is held here, in this
//! process, between the two calls.
//!
//! # The file is re-read at commit
//!
//! A plan is worked out against the file as it was. `commit` writes the whole
//! file, so a tool that rewrote its own config in between would have that
//! rewrite silently reverted. The digest of the file as it was read at plan
//! time is kept beside the plan and checked again before the write; a file
//! that moved refuses with `harness-config-changed` and the shell plans
//! again.
//!
//! # Nothing from inside the contributor's file is logged or returned
//!
//! `changes` and `occupied` are IronWire's own words about slots and values,
//! and the occupied *values* are shown deliberately -- that is the whole
//! point of reporting rather than overwriting. Parse-failure detail is not:
//! serde and toml errors quote the offending line, so an unparseable file is
//! reported as a label and a path, never as its own contents.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use ironwire_agents::tools;
use ironwire_catalog::schema::{AgentEntry, Catalog, ConfigLocation};
use uuid::Uuid;

use crate::harness_state::{
    self, HarnessAction, HarnessEvidence, HarnessState, PlanOutcome, built_in_family,
};

/// How long the window looked at for "has a call actually arrived".
///
/// The ledger's own refresh window is 24 hours, so nothing longer can be
/// answered anyway. Kept as a named constant because it is a claim a shell
/// repeats: "answering" means a call arrived inside this window, not ever.
pub const ACTIVITY_WINDOW_HOURS: i64 = 24;

/// How long a worked-out plan stays committable.
///
/// A preview a contributor left on screen over lunch describes a file as it
/// was at lunchtime. Expiring it costs one extra call and removes a class of
/// stale write entirely.
const PLAN_TTL: Duration = Duration::from_secs(10 * 60);

/// How many plans are held at once.
///
/// One shell, one dialog, one plan -- but a shell that plans on every hover
/// must not grow this without bound, and the oldest is dropped rather than
/// the newest refused.
const MAX_HELD_PLANS: usize = 8;

/// `harness_commit` was handed a plan id this daemon does not hold.
///
/// Expired, already committed, or never minted. All three are the same
/// instruction to a shell: plan again and show the contributor the result.
pub const ERR_PLAN_UNKNOWN: &str = "harness-plan-unknown";

/// The file changed between the plan and the commit.
pub const ERR_CONFIG_CHANGED: &str = "harness-config-changed";

/// The write itself failed.
pub const ERR_COMMIT_FAILED: &str = "harness-commit-failed";

/// No tool by that id.
pub const ERR_UNKNOWN_HARNESS: &str = "harness-unknown";

/// A connect was asked for while nothing on this machine is answering model
/// calls, so there is no port to point the tool at.
///
/// Not a failure of the tool or the file: the destination has to be on
/// before a config can name it. A shell that gets this asks the exposure
/// question and turns the destination on, then plans again.
pub const ERR_NO_DESTINATION: &str = "harness-no-destination";

/// An edit that has been worked out and not made.
struct HeldPlan {
    id: Uuid,
    tool_id: String,
    action: HarnessAction,
    planned: tools::Planned,
    /// SHA-256 of the config file as it was when the plan was worked out, or
    /// `None` when the file did not exist. Compared again before the write.
    digest: Option<String>,
    minted: Instant,
}

/// The plans this daemon is holding between a preview and its commit.
#[derive(Default)]
pub struct PlanStore {
    held: Mutex<VecDeque<HeldPlan>>,
}

impl std::fmt::Debug for PlanStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A plan holds the contents it would write, which is the
        // contributor's own configuration file. It does not appear in a
        // debug line.
        f.debug_struct("PlanStore").finish_non_exhaustive()
    }
}

impl PlanStore {
    fn insert(&self, plan: HeldPlan) {
        let Ok(mut held) = self.held.lock() else {
            return;
        };
        held.retain(|p| p.minted.elapsed() < PLAN_TTL);
        while held.len() >= MAX_HELD_PLANS {
            held.pop_front();
        }
        held.push_back(plan);
    }

    /// Take a plan out, if it is still there and still fresh.
    ///
    /// Removed rather than read: a plan is a permission to write a file once.
    /// Leaving it in place would let a shell replay one commit against a file
    /// that has since been disconnected.
    fn take(&self, id: Uuid) -> Option<HeldPlan> {
        let mut held = self.held.lock().ok()?;
        held.retain(|p| p.minted.elapsed() < PLAN_TTL);
        let at = held.iter().position(|p| p.id == id)?;
        held.remove(at)
    }
}

/// SHA-256 of a file, or `None` when it is not there.
///
/// A file that exists and cannot be read yields `Some` of the digest of
/// nothing -- no: it yields `None` as well, which is deliberate. A plan whose
/// file could not be read at plan time is one `commit` will refuse anyway,
/// and the alternative is a digest that says "absent" for a file that is
/// present.
fn config_digest(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Some(format!("{:x}", hasher.finalize()))
}

/// Whether a tool's config names the destination answering on THIS machine.
///
/// `ironwire_agents` answers a deliberately broader question. Its
/// `points_at_us` accepts any loopback port, and its own comment says why:
/// "A port change still counts as ours, which is what lets `connect` follow
/// the daemon to a new port instead of giving up." For a single product with
/// a single listener that is the right question and the right answer.
///
/// It is not our question. We hold a `destination_port` and want to know
/// whether this tool sends its calls to *that*, and borrowing the broader
/// answer makes the surface claim things that are false: a config left
/// pointing at a port the daemon no longer uses reads as "its settings send
/// its calls here", under a restart prompt that can never come true.
///
/// Checked as text rather than by re-parsing each tool's format: the file has
/// already satisfied upstream's own check, so the only remaining question is
/// which port it names. A file that cannot be read is not ours, which is the
/// safe direction.
fn points_at_our_port(path: Option<&PathBuf>, port: Option<u16>) -> bool {
    let (Some(path), Some(port)) = (path, port) else {
        return false;
    };
    std::fs::read_to_string(path)
        .map(|text| text.contains(&format!("127.0.0.1:{port}")))
        .unwrap_or(false)
}

/// One tool, as the wire describes it.
#[derive(Debug, Clone)]
pub struct HarnessRow {
    pub id: String,
    pub name: String,
    pub installed: bool,
    /// Whether this tool's config names OUR destination.
    ///
    /// Narrower than upstream's `wired`, and deliberately so -- see
    /// [`points_at_our_port`]. This is the one that drives the state and the
    /// sentence, because "its settings send its calls here" must not be said
    /// about a file naming somebody else's port.
    pub connected: bool,
    /// Whether the config names *a* local proxy, which is upstream's answer.
    ///
    /// Kept because it is the right input to the disconnect control: a line
    /// pointing at a stale or foreign port is still a line, and a contributor
    /// who cannot see a way to remove it is stuck.
    pub wired: bool,
    pub config_path: Option<PathBuf>,
    pub connect_command: String,
    pub family: Option<&'static str>,
    pub state: HarnessState,
}

/// What the ledger says arrived, by protocol family.
///
/// A family and a time, never a tool. The ledger records a facade, and this
/// type is the shape of that limitation: a caller can say "a call arrived"
/// and, only where the family belongs to exactly one connected tool, say
/// which tool it was.
#[derive(Debug, Clone, Default)]
pub struct FamilyActivity {
    /// Whether the ledger answered at all. False means no evidence about any
    /// tool, which a surface must not render as "no calls yet".
    pub readable: bool,
    /// `(family, last call, count)`, oldest-agnostic, one row per family
    /// seen in the window.
    pub families: Vec<(String, DateTime<Utc>, usize)>,
}

impl FamilyActivity {
    fn last_call_for(&self, family: &str) -> Option<DateTime<Utc>> {
        self.families
            .iter()
            .find(|(name, _, _)| name == family)
            .map(|(_, at, _)| *at)
    }

    /// The most recent call in any family, or `None`.
    ///
    /// The answer to "did a call arrive" for a surface that must not name a
    /// tool.
    #[must_use]
    pub fn last_call(&self) -> Option<DateTime<Utc>> {
        self.families.iter().map(|(_, at, _)| *at).max()
    }
}

/// Roll a window of ledger rows up into per-family activity.
///
/// Rows carry a facade and a start time and nothing this needs beyond them.
/// The facade is the family; no attempt is made to narrow it further, because
/// nothing in a row can.
#[must_use]
pub fn family_activity(rows: &[crate::routing::RoutedExchange], readable: bool) -> FamilyActivity {
    let mut families: Vec<(String, DateTime<Utc>, usize)> = Vec::new();
    for row in rows {
        match families.iter_mut().find(|(name, _, _)| *name == row.facade) {
            Some((_, at, count)) => {
                if row.started_at > *at {
                    *at = row.started_at;
                }
                *count += 1;
            }
            None => families.push((row.facade.clone(), row.started_at, 1)),
        }
    }
    families.sort_by(|a, b| a.0.cmp(&b.0));
    FamilyActivity { readable, families }
}

/// Every tool this machine knows about, with its state.
///
/// `catalog` is IronWire's signed catalog when one is loaded. With none, the
/// list is the two tools IronWire ships knowing about -- which is a fact
/// about this build, not about the machine, and is reported as
/// `catalog_present: false` so a surface can say so rather than implying the
/// machine has only two.
#[must_use]
pub fn list(
    catalog: &Catalog,
    activity: &FamilyActivity,
    destination_port: Option<u16>,
) -> Vec<HarnessRow> {
    let found = tools::all(catalog);

    // Which families more than one *connected* tool speaks. Counted over the
    // connected ones only: an installed-but-not-connected Codex cannot have
    // made the call, so it must not blur the attribution of one that is.
    let mut connected_per_family: Vec<(&str, usize)> = Vec::new();
    for tool in &found {
        // Ours, not merely wired: a tool pointed at somebody else's local
        // proxy did not make the call and must not blur the attribution of
        // one that did.
        if !(tool.wired && points_at_our_port(tool.config_path.as_ref(), destination_port)) {
            continue;
        }
        let Some(family) = built_in_family(&tool.id) else {
            continue;
        };
        match connected_per_family
            .iter_mut()
            .find(|(name, _)| *name == family)
        {
            Some((_, count)) => *count += 1,
            None => connected_per_family.push((family, 1)),
        }
    }

    found
        .into_iter()
        .map(|tool| {
            let family = built_in_family(&tool.id);
            let shared = family.is_some_and(|f| {
                connected_per_family
                    .iter()
                    .any(|(name, count)| *name == f && *count > 1)
            });
            let saw_call = family.is_some_and(|f| activity.last_call_for(f).is_some());
            let ours =
                tool.wired && points_at_our_port(tool.config_path.as_ref(), destination_port);
            let state = harness_state::harness_state(HarnessEvidence {
                connected: ours,
                activity_readable: activity.readable,
                family,
                family_saw_call: saw_call,
                family_shared: shared,
            });
            HarnessRow {
                id: tool.id,
                name: tool.name,
                installed: tool.installed,
                connected: ours,
                wired: tool.wired,
                config_path: tool.config_path,
                connect_command: tool.connect_command,
                family,
                state,
            }
        })
        .collect()
}

/// A worked-out plan, as the wire describes it.
#[derive(Debug, Clone)]
pub struct PlanView {
    /// Present only when there is an edit to commit, and therefore only for
    /// [`PlanOutcome::Changes`].
    pub plan_id: Option<Uuid>,
    pub tool_id: String,
    pub action: HarnessAction,
    pub outcome: PlanOutcome,
    /// The file that would change, when one was located.
    pub path: Option<PathBuf>,
    /// What would change, in IronWire's words.
    pub changes: Vec<String>,
    /// Slots left alone because the contributor is already using them.
    /// Carried whatever the outcome -- see [`PlanOutcome`].
    pub occupied: Vec<(String, String)>,
}

/// Work out an edit without making it.
///
/// `port` is the port this machine answers model calls on, and is required
/// for a connect and ignored for a disconnect: taking a tool back off needs
/// no destination, which is why disconnecting keeps working after the
/// listener has stopped.
///
/// # Errors
///
/// [`ERR_UNKNOWN_HARNESS`] for an id nothing knows, and
/// [`ERR_NO_DESTINATION`] for a connect with no port. Every other refusal is
/// a [`PlanOutcome`], not an error: an unparseable file and a tool that is
/// not installed are facts about the machine a contributor needs shown, not
/// call failures.
pub fn plan(
    store: &PlanStore,
    catalog: &Catalog,
    tool_id: &str,
    action: HarnessAction,
    port: Option<u16>,
) -> Result<PlanView, &'static str> {
    let rows = tools::all(catalog);
    let Some(row) = rows.iter().find(|t| t.id == tool_id) else {
        return Err(ERR_UNKNOWN_HARNESS);
    };

    // Ahead of the tool's own state, deliberately.
    //
    // "There is nowhere to send calls" is a fact about this computer, not
    // about the tool, and it is true whatever the tool's state is. Deciding it
    // second made the refusal unreachable for any tool that is not installed:
    // the caller got `NotInstalled` and never learned there was no destination
    // either. It also made the outcome depend on what happens to be installed
    // on the machine running the code, which is not a property this function
    // should have -- and which made its own test pass only on a machine with
    // the tool installed.
    if matches!(action, HarnessAction::Connect) && port.is_none() {
        return Err(ERR_NO_DESTINATION);
    }

    let refuse = |outcome: PlanOutcome| PlanView {
        plan_id: None,
        tool_id: tool_id.to_string(),
        action,
        outcome,
        path: row.config_path.clone(),
        changes: Vec::new(),
        occupied: Vec::new(),
    };

    if !harness_state::action_available(action, row.installed, row.connected_hint()) {
        // Only one of the two rules can be reported as an outcome here: a
        // disconnect that is unavailable is unavailable because the tool is
        // not connected, and that is a no-op, not a refusal.
        return Ok(match action {
            HarnessAction::Connect if !row.installed => refuse(PlanOutcome::NotInstalled),
            _ => refuse(PlanOutcome::Noop),
        });
    }

    // A file we cannot read is refused HERE, before upstream is asked.
    //
    // "Never rewrite a file we cannot parse" is the first of the three rules
    // that make editing somebody else's config acceptable, and upstream cannot
    // enforce it on its own: `ironwire_agents::tools::read` is
    // `read_to_string(path).ok().unwrap_or_default()`, so a file that is not
    // valid UTF-8 reads as the empty string -- the same answer it gives for a
    // file that is not there.
    //
    // Everything downstream then agrees the file is absent. An empty string
    // parses as an empty settings map rather than an error, so the plan says
    // "(added)" instead of refusing; and `tools::commit` skips writing a
    // backup exactly when `existing` is empty. The result is a contributor's
    // settings replaced wholesale with no backup and no warning.
    //
    // It is not a contrived shape: UTF-16LE is what Windows PowerShell 5.1's
    // `Set-Content` and `>` write by default, so a hand-edited
    // `settings.json` on Windows can easily be one.
    //
    // Reading it ourselves is the whole fix: `fs::read_to_string` fails where
    // upstream's `.ok()` discards the error, and a path that exists but will
    // not decode is refused with the outcome the copy already describes.
    if let Some(path) = row.config_path.as_ref()
        && path.exists()
        && std::fs::read_to_string(path).is_err()
    {
        return Ok(refuse(PlanOutcome::Unparseable));
    }

    let planned = match action {
        HarnessAction::Connect => {
            let Some(port) = port else {
                return Err(ERR_NO_DESTINATION);
            };
            tools::plan_connect(tool_id, port, catalog)
        }
        HarnessAction::Disconnect => tools::plan_disconnect(tool_id, catalog),
    };

    let planned = match planned {
        Ok(planned) => planned,
        Err(tools::Error::UnknownTool(_)) => return Err(ERR_UNKNOWN_HARNESS),
        Err(tools::Error::NoPath(_)) => return Ok(refuse(PlanOutcome::NoConfigPath)),
        Err(tools::Error::Edit(detail)) => {
            // `Error::Edit` flattens two upstream cases into one string, and
            // the string quotes the file. Neither the string nor the file
            // crosses this line: the prefix is matched to separate a catalog
            // entry that did not validate from a file that would not parse,
            // and everything else is dropped. For the two built-in tools the
            // only possible cause is a parse failure, so the fallback is the
            // right answer there by construction.
            return Ok(refuse(
                if detail.starts_with("the catalog entry is not usable") {
                    PlanOutcome::EntryUnusable
                } else {
                    PlanOutcome::Unparseable
                },
            ));
        }
    };

    let occupied = planned.occupied.clone();
    let path = planned.path.clone();
    if planned.is_noop() {
        // A distinct outcome, not an empty preview. Occupied slots still
        // ride along: "nothing to change, and here is the slot somebody else
        // is using" is the exact state a contributor needs to see.
        return Ok(PlanView {
            plan_id: None,
            tool_id: tool_id.to_string(),
            action,
            outcome: PlanOutcome::Noop,
            path: Some(path),
            changes: Vec::new(),
            occupied,
        });
    }

    let id = Uuid::new_v4();
    let view = PlanView {
        plan_id: Some(id),
        tool_id: tool_id.to_string(),
        action,
        outcome: PlanOutcome::Changes,
        path: Some(path.clone()),
        changes: planned.changes.clone(),
        occupied,
    };
    store.insert(HeldPlan {
        id,
        tool_id: tool_id.to_string(),
        action,
        digest: config_digest(&path),
        planned,
        minted: Instant::now(),
    });
    Ok(view)
}

/// What a commit did.
#[derive(Debug, Clone)]
pub struct CommitView {
    pub tool_id: String,
    pub action: HarnessAction,
    pub path: PathBuf,
    /// Where the file as it was before this edit was kept, when one was
    /// written. IronWire keeps only the first, so this is absent on a second
    /// edit to the same file.
    pub backup_path: Option<PathBuf>,
}

/// Make an edit that was already shown.
///
/// Takes a plan id and nothing else. There is no argument here that could
/// name a different tool, a different action or a different file from the one
/// previewed, which is what makes the preview binding rather than advisory.
///
/// # Errors
///
/// [`ERR_PLAN_UNKNOWN`] when the id is expired, already used or never minted;
/// [`ERR_CONFIG_CHANGED`] when the file moved under the plan;
/// [`ERR_COMMIT_FAILED`] when the write itself failed.
pub fn commit(store: &PlanStore, plan_id: Uuid) -> Result<CommitView, &'static str> {
    let held = store.take(plan_id).ok_or(ERR_PLAN_UNKNOWN)?;
    if config_digest(&held.planned.path) != held.digest {
        return Err(ERR_CONFIG_CHANGED);
    }
    let backup_path = tools::commit(&held.planned).map_err(|_| ERR_COMMIT_FAILED)?;
    Ok(CommitView {
        tool_id: held.tool_id,
        action: held.action,
        path: held.planned.path.clone(),
        backup_path,
    })
}

/// `Tool` names the field `wired`; this crate says `connected` everywhere a
/// contributor can see. One helper so the translation happens once.
trait ConnectedHint {
    fn connected_hint(&self) -> bool;
}

impl ConnectedHint for tools::Tool {
    fn connected_hint(&self) -> bool {
        self.wired
    }
}

// ---------------------------------------------------------------------------
// The socket surface
// ---------------------------------------------------------------------------

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};

/// The tools we compile in ourselves, beyond the two IronWire ships knowing
/// about.
///
/// **This is deliberately ours, and deliberately not the fetched catalog.**
/// IronWire can also take this list from a signed document it downloads, and
/// that channel is inert: the URL it names does not resolve, and the key it
/// verifies against is thirty-two zero bytes, documented upstream as a
/// placeholder that cannot verify anything. So adopting it would buy nothing
/// today -- and the day it worked, it would mean a third party's release
/// signing key decided which files on a contributor's machine we offer to
/// edit. The bound on the damage is real (a location must be a `.json` or
/// `.toml` file one or two segments under `$HOME`, the first a dotdir, and
/// the value written is always one of our own loopback URLs) but it is not
/// nothing, and it is not a trust we need to take on. A table compiled into
/// our own release asks for the trust we already ask for.
///
/// Nothing about the signature guards this path: `tools::all`,
/// `plan_connect` and `plan_disconnect` take a `&Catalog` as a plain
/// argument with public fields. Verification guards the network refresh, not
/// the value.
///
/// **The list is short on purpose.** A tool goes in only when all three hold,
/// each with evidence:
///
/// 1. its config file is representable under the location rules above;
/// 2. it has a key that takes a base URL *only* and genuinely redirects that
///    tool's model calls -- not an API key, not a model name, not a
///    per-provider block that needs more than a URL;
/// 3. it speaks the Anthropic or OpenAI wire shape, because the value written
///    is always `Facade::url(port)` and exactly those two facades exist.
///
/// Rule 3 is narrower than it looks. `Facade::url` yields
/// `http://127.0.0.1:{port}/anthropic` or `.../openai` -- an origin-style
/// base with no `/v1`. That matches the `ANTHROPIC_BASE_URL` convention
/// Claude Code follows, and it matches nothing else so far examined: the SDK
/// convention is a base URL that already ends in `/v1`, and Codex is wired by
/// hand precisely so its base can be spelled `.../openai/v1`. A catalog entry
/// cannot spell that, so a tool that wants it cannot be an entry.
///
/// A wrong entry writes our URL into a real contributor's config file and
/// breaks their tool. An empty table is the honest answer until an entry can
/// be shown to be right.
fn owned_agents() -> Vec<AgentEntry> {
    Vec::new()
}

/// Whether a config location is one we are willing to edit.
///
/// A restatement, in our own code, of the rules `ConfigLocation` enforces
/// upstream. Duplicated on purpose: these rules are the whole security
/// argument for a table that names files on someone else's machine, and
/// neither the test over our own entries nor the filter in [`catalog`] must
/// be able to stop meaning anything because the pinned IronWire revision
/// moved.
fn config_location_is_allowed(location: &ConfigLocation) -> bool {
    fn segment_is_plain(segment: &str) -> bool {
        !segment.is_empty()
            && segment != "."
            && segment != ".."
            && !segment.starts_with('-')
            && segment
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
    }

    if location.dir.is_empty() || location.dir.len() > 2 {
        return false;
    }
    if !location.dir.iter().all(|s| segment_is_plain(s)) {
        return false;
    }
    if !location.dir[0].starts_with('.') {
        return false;
    }
    if !segment_is_plain(&location.file) || location.file.starts_with('.') {
        return false;
    }
    location.file.ends_with(".json") || location.file.ends_with(".toml")
}

/// The catalog this daemon lists tools from.
///
/// The provider constants stay IronWire's compiled-in defaults; the tool
/// table is [`owned_agents`], passed through
/// [`config_location_is_allowed`] so that an entry naming a file outside the
/// allowed shape is dropped here and not merely caught by a test. It is a
/// function rather than an inlined value at each call site so that adding a
/// tool is one table edit -- and so that `catalog_present` on the wire is
/// derived from the same value the list is, rather than being a second,
/// separately-maintained claim.
fn catalog() -> Catalog {
    Catalog {
        agents: owned_agents()
            .into_iter()
            .filter(|entry| config_location_is_allowed(&entry.config))
            .collect(),
        ..Catalog::default()
    }
}

/// What the ledger can say about calls that arrived, rolled up by family.
fn activity_for(shared: &DaemonShared) -> FamilyActivity {
    let Some(ledger) = shared.routing_ledger() else {
        // No declared, readable proxy: no evidence about any tool. Reported
        // as unreadable rather than as an empty window, because "nothing has
        // answered" and "the proxy answered and this window is empty" are
        // the two states a contributor most needs told apart.
        return FamilyActivity::default();
    };
    if ledger.last_refresh_at().is_none() {
        return FamilyActivity::default();
    }
    let since = Utc::now() - chrono::Duration::hours(ACTIVITY_WINDOW_HOURS);
    let rows = crate::routing::RoutingLedger::exchanges_since(ledger.as_ref(), since);
    family_activity(&rows, true)
}

/// A path, or absent.
///
/// Null rather than `""`: a surface renders nothing for absent, and an empty
/// string would send a contributor to look at a file called "".
fn path_value(path: Option<&Path>) -> serde_json::Value {
    path.map_or(serde_json::Value::Null, |p| {
        serde_json::json!(p.to_string_lossy())
    })
}

/// `harness_list`: every tool this machine knows about, and its state.
pub fn handle_list(shared: &DaemonShared, req: &Request) -> Response {
    let catalog = catalog();
    let activity = activity_for(shared);
    let destination_port = shared.destination_port();
    let rows = list(&catalog, &activity, destination_port);

    let harnesses: Vec<serde_json::Value> = rows
        .iter()
        .map(|row| {
            serde_json::json!({
                "id": row.id,
                "name": row.name,
                "installed": row.installed,
                "connected": row.connected,
                "config_path": path_value(row.config_path.as_deref()),
                "connect_command": row.connect_command,
                "family": row.family,
                "state": row.state.label(),
                // The time that makes "answering" mean something. Present
                // only where the family belongs to this tool alone; a shared
                // family's time is on the block below, where it names no
                // tool.
                "last_call_at": match row.state {
                    HarnessState::Answering => row
                        .family
                        .and_then(|f| activity.last_call_for(f))
                        .map(|at| at.to_rfc3339()),
                    _ => None,
                },
                "can_connect": harness_state::action_available(
                    HarnessAction::Connect, row.installed, row.connected,
                ),
                // Deliberately `wired`, not `connected`: a line naming a
                // stale or foreign port is still a line this app can remove,
                // and hiding the control would strand it.
                "can_disconnect": harness_state::action_available(
                    HarnessAction::Disconnect, row.installed, row.wired,
                ),
            })
        })
        .collect();

    Response::ok(
        req.id,
        serde_json::json!({
            // A fact about this build, not about the machine. False means
            // the list is the two built-in tools, and says nothing about
            // every other tool that exists.
            "catalog_present": !catalog.agents().is_empty(),
            "harnesses": harnesses,
            // A call arrived, with no tool named. This is the honest shape
            // of the ledger's evidence: it records a facade, so a surface
            // can always say a call arrived and can only sometimes say by
            // whom.
            "activity": {
                "readable": activity.readable,
                "window_hours": ACTIVITY_WINDOW_HOURS,
                "last_call_at": activity.last_call().map(|at| at.to_rfc3339()),
                "families": activity
                    .families
                    .iter()
                    .map(|(family, at, count)| serde_json::json!({
                        "family": family,
                        "last_call_at": at.to_rfc3339(),
                        "calls": count,
                    }))
                    .collect::<Vec<_>>(),
            },
            // The port a connect would write into a config file. Null when
            // nothing on this machine is answering model calls, which is
            // what `harness_plan` refuses a connect with.
            "destination_port": shared.destination_port(),
        }),
    )
}

/// `harness_plan`: work out one tool's edit and write nothing.
pub fn handle_plan(shared: &DaemonShared, req: &Request) -> Response {
    let Some(id) = req.params.get("id").and_then(serde_json::Value::as_str) else {
        return Response::err(req.id, ERR_BAD_PARAMS, "id-required");
    };
    let Some(action) = req
        .params
        .get("action")
        .and_then(serde_json::Value::as_str)
        .and_then(HarnessAction::from_label)
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "action-invalid");
    };

    match plan(
        &shared.harness_plans,
        &catalog(),
        id,
        action,
        shared.destination_port(),
    ) {
        Ok(view) => Response::ok(
            req.id,
            serde_json::json!({
                "id": view.tool_id,
                "action": view.action.label(),
                "outcome": view.outcome.label(),
                // Minted for a committable plan and for nothing else, so
                // "there is a plan id" and "the outcome is committable" are
                // one question answered once.
                "plan_id": view.plan_id.map(|id| id.to_string()),
                "path": path_value(view.path.as_deref()),
                "changes": view.changes,
                // Reported, never overwritten -- the rule that makes editing
                // a file we do not own acceptable at all. Carried alongside
                // the outcome rather than folded into it, because a plan can
                // have changes AND an occupied slot.
                "occupied": view
                    .occupied
                    .iter()
                    .map(|(slot, current)| serde_json::json!({
                        "slot": slot,
                        "current": current,
                    }))
                    .collect::<Vec<_>>(),
            }),
        ),
        Err(label) => Response::err(req.id, ERR_BAD_PARAMS, label),
    }
}

/// `harness_commit`: make an edit that was already shown.
pub fn handle_commit(shared: &DaemonShared, req: &Request) -> Response {
    let Some(plan_id) = req
        .params
        .get("plan_id")
        .and_then(serde_json::Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
    else {
        return Response::err(req.id, ERR_BAD_PARAMS, "plan-id-invalid");
    };
    match commit(&shared.harness_plans, plan_id) {
        Ok(view) => Response::ok(
            req.id,
            serde_json::json!({
                "id": view.tool_id,
                "action": view.action.label(),
                "committed": true,
                "path": path_value(Some(&view.path)),
                "backup_path": path_value(view.backup_path.as_deref()),
            }),
        ),
        Err(label) => Response::err(req.id, ERR_UNAVAILABLE, label),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use std::sync::{Mutex as StdMutex, MutexGuard, PoisonError};

    /// Serializes the tests that point Claude Code's config somewhere.
    ///
    /// `CLAUDE_CONFIG_DIR` is process-wide, and these tests both read and
    /// write the file it names. The pattern is `ironwire_pointer`'s: one
    /// lock, restored on drop, so a panicking test cannot leave the variable
    /// set for whatever runs next.
    static CLAUDE_CONFIG_LOCK: StdMutex<()> = StdMutex::new(());

    struct ClaudeConfigAt {
        _lock: MutexGuard<'static, ()>,
        previous: Option<std::ffi::OsString>,
    }

    impl ClaudeConfigAt {
        fn at(dir: &Path) -> Self {
            let lock = CLAUDE_CONFIG_LOCK
                .lock()
                .unwrap_or_else(PoisonError::into_inner);
            let previous = std::env::var_os("CLAUDE_CONFIG_DIR");
            unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", dir) };
            Self {
                _lock: lock,
                previous,
            }
        }
    }

    impl Drop for ClaudeConfigAt {
        fn drop(&mut self) {
            match self.previous.take() {
                Some(v) => unsafe { std::env::set_var("CLAUDE_CONFIG_DIR", v) },
                None => unsafe { std::env::remove_var("CLAUDE_CONFIG_DIR") },
            }
        }
    }

    /// A temp directory standing in for Claude Code's config directory,
    /// holding `body`.
    fn claude_config(body: &str) -> (tempfile::TempDir, ClaudeConfigAt, PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(&path, body).expect("write");
        let guard = ClaudeConfigAt::at(dir.path());
        (dir, guard, path)
    }

    fn plan_claude(store: &PlanStore, action: HarnessAction) -> PlanView {
        plan(store, &Catalog::default(), "claude", action, Some(8463)).expect("claude is known")
    }

    /// A settings file that is not valid UTF-8 is refused, not replaced.
    ///
    /// Upstream cannot catch this: `tools::read` is
    /// `read_to_string(path).ok().unwrap_or_default()`, so an undecodable file
    /// reads as the empty string, which is also what an absent file reads as.
    /// The plan then reports "(added)" rather than refusing, and
    /// `tools::commit` skips the backup precisely because `existing` is empty
    /// -- so the contributor's file would be replaced wholesale with nothing
    /// kept.
    ///
    /// The bytes below are real UTF-16LE with a BOM, which is what Windows
    /// PowerShell 5.1 writes by default, so a hand-edited settings file can
    /// genuinely be this.
    #[test]
    fn a_settings_file_that_is_not_utf8_is_refused_rather_than_replaced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");

        let source = r#"{"env":{"MY_KEY":"keep me"}}"#;
        let mut bytes = vec![0xFF, 0xFE]; // UTF-16LE BOM
        for unit in source.encode_utf16() {
            bytes.extend_from_slice(&unit.to_le_bytes());
        }
        std::fs::write(&path, &bytes).expect("write");
        let _guard = ClaudeConfigAt::at(dir.path());

        assert!(
            std::fs::read_to_string(&path).is_err(),
            "the fixture must actually be undecodable, or this test proves nothing"
        );

        let store = PlanStore::default();
        let view = plan_claude(&store, HarnessAction::Connect);

        assert_eq!(
            view.outcome,
            PlanOutcome::Unparseable,
            "a file we cannot read must be refused, never rewritten"
        );
        assert!(
            view.plan_id.is_none(),
            "a refusal mints no permission to write"
        );
        assert!(view.changes.is_empty());

        // The contributor's bytes are untouched, which is the point.
        assert_eq!(
            std::fs::read(&path).expect("read back"),
            bytes,
            "the file must be exactly as it was"
        );
    }

    /// The rule most likely to be broken by a well-meaning simplification: a
    /// slot the contributor is already using is REPORTED, and the value they
    /// put there is still in the file afterwards.
    #[test]
    fn an_occupied_slot_is_reported_and_never_overwritten() {
        let (_dir, _guard, path) =
            claude_config(r#"{"env":{"ANTHROPIC_BASE_URL":"https://their-own-choice.example"}}"#);
        let store = PlanStore::default();

        let view = plan_claude(&store, HarnessAction::Connect);
        assert!(
            view.occupied
                .iter()
                .any(|(_, current)| current == "https://their-own-choice.example"),
            "the occupied slot was not reported: {view:?}"
        );

        if let Some(id) = view.plan_id {
            commit(&store, id).expect("the rest of the edit still applies");
        }
        let after = std::fs::read_to_string(&path).expect("read");
        assert!(
            after.contains("https://their-own-choice.example"),
            "the contributor's own value was overwritten: {after}"
        );
    }

    /// Refused deliberately, and distinguishable from "nothing to change".
    #[test]
    fn an_unparseable_config_is_refused_and_named() {
        let (_dir, _guard, path) = claude_config("{ this is not settings");
        let store = PlanStore::default();

        let view = plan_claude(&store, HarnessAction::Connect);
        assert_eq!(view.outcome, PlanOutcome::Unparseable);
        assert_ne!(view.outcome, PlanOutcome::Noop);
        assert_eq!(
            view.plan_id, None,
            "an unparseable file has nothing to commit"
        );
        assert_eq!(view.path.as_deref(), Some(path.as_path()), "name the file");
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "{ this is not settings",
            "a file we cannot parse was rewritten"
        );
    }

    /// A no-op says so rather than showing an empty confirmation.
    #[test]
    fn a_second_connect_is_a_noop_with_no_plan_id() {
        let (_dir, _guard, _path) = claude_config("{}");
        let store = PlanStore::default();

        let first = plan_claude(&store, HarnessAction::Connect);
        assert_eq!(first.outcome, PlanOutcome::Changes);
        commit(&store, first.plan_id.expect("a committable plan")).expect("commit");

        let second = plan_claude(&store, HarnessAction::Connect);
        assert_eq!(second.outcome, PlanOutcome::Noop);
        assert_eq!(second.plan_id, None);
        assert!(second.changes.is_empty());
    }

    /// Remove only what we put there.
    #[test]
    fn a_disconnect_leaves_neighbouring_keys_alone() {
        let (_dir, _guard, path) = claude_config(r#"{"env":{"THEIR_OWN_KEY":"keep me"}}"#);
        let store = PlanStore::default();

        let connect = plan_claude(&store, HarnessAction::Connect);
        commit(&store, connect.plan_id.expect("a committable plan")).expect("commit");

        let disconnect = plan_claude(&store, HarnessAction::Disconnect);
        assert_eq!(disconnect.outcome, PlanOutcome::Changes);
        commit(&store, disconnect.plan_id.expect("a committable plan")).expect("commit");

        let after = std::fs::read_to_string(&path).expect("read");
        assert!(
            after.contains("keep me"),
            "a neighbouring key was removed: {after}"
        );
    }

    /// The tool rewrote its own config while the preview was on screen. The
    /// preview no longer describes the file, so the write is refused rather
    /// than reverting whatever the tool just wrote.
    #[test]
    fn a_config_that_moved_under_the_plan_refuses_the_commit() {
        let (_dir, _guard, path) = claude_config("{}");
        let store = PlanStore::default();

        let view = plan_claude(&store, HarnessAction::Connect);
        let id = view.plan_id.expect("a committable plan");
        std::fs::write(&path, r#"{"env":{"SOMETHING":"the tool wrote this"}}"#).expect("write");

        assert_eq!(
            commit(&store, id).expect_err("the file moved"),
            ERR_CONFIG_CHANGED
        );
        assert!(
            std::fs::read_to_string(&path)
                .expect("read")
                .contains("the tool wrote this"),
            "a stale plan overwrote a newer file"
        );
    }

    /// A plan is a permission to write once.
    #[test]
    fn a_committed_plan_cannot_be_replayed() {
        let (_dir, _guard, _path) = claude_config("{}");
        let store = PlanStore::default();

        let view = plan_claude(&store, HarnessAction::Connect);
        let id = view.plan_id.expect("a committable plan");
        commit(&store, id).expect("commit");
        assert_eq!(
            commit(&store, id).expect_err("already used"),
            ERR_PLAN_UNKNOWN
        );
    }

    fn row(facade: &str, offset: i64) -> crate::routing::RoutedExchange {
        crate::routing::RoutedExchange {
            id: None,
            started_at: Utc.timestamp_opt(1_700_000_000 + offset, 0).unwrap(),
            client_session_id: None,
            total_ms: None,
            facade: facade.to_string(),
            backend: "somewhere".to_string(),
            requested_model: None,
            served_model: None,
            upstream_id: None,
            request_sha256: None,
            response_sha256: None,
            body_ref: None,
            rung: "same_model".to_string(),
            attempts: 1,
            input_tokens: None,
            cache_read_tokens: None,
            cache_write_tokens: None,
            output_tokens: None,
            cost_usd: None,
            status: 200,
        }
    }

    #[test]
    fn activity_is_grouped_by_family_and_keeps_the_latest_call() {
        let activity = family_activity(
            &[row("anthropic", 0), row("anthropic", 60), row("openai", 10)],
            true,
        );
        assert_eq!(activity.families.len(), 2);
        assert_eq!(
            activity.last_call_for("anthropic"),
            Some(Utc.timestamp_opt(1_700_000_060, 0).unwrap())
        );
        assert_eq!(
            activity.last_call(),
            Some(Utc.timestamp_opt(1_700_000_060, 0).unwrap())
        );
        assert_eq!(activity.last_call_for("gemini"), None);
    }

    /// The point of the type: a surface can say a call arrived without any
    /// tool being nameable.
    #[test]
    fn a_call_is_reportable_with_no_tool_named() {
        let activity = family_activity(&[row("anthropic", 0)], true);
        assert!(activity.last_call().is_some());
        // Nothing on this value names a tool.
        assert!(activity.families.iter().all(|(f, _, _)| f == "anthropic"));
    }

    #[test]
    fn an_unreadable_ledger_reports_no_families_and_says_so() {
        let activity = family_activity(&[], false);
        assert!(!activity.readable);
        assert_eq!(activity.last_call(), None);
    }

    /// A config naming a DIFFERENT local port is not "sends its calls here".
    ///
    /// Upstream's `wired` accepts any loopback port on purpose, so `connect`
    /// can follow the daemon to a new one. Borrowing that answer made the row
    /// say "Connected, and nothing has arrived from it yet. Its settings send
    /// its calls here" about a file that names somebody else's port -- and
    /// under it the shells draw "Quit this tool and open it again", advice
    /// that can never come true and that is shown until a call arrives.
    ///
    /// The disconnect control must survive the narrowing: the line is still a
    /// line this app can remove, and hiding the only control would strand it.
    #[test]
    fn a_config_pointing_at_another_port_is_not_connected_to_us() {
        let (_dir, _guard, _path) =
            claude_config(r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9999/anthropic"}}"#);

        let ours = list(&Catalog::default(), &FamilyActivity::default(), Some(8463));
        let claude = ours.iter().find(|r| r.id == "claude").expect("claude row");

        assert!(
            !claude.connected,
            "a file naming port 9999 does not send its calls to 8463"
        );
        assert_ne!(
            claude.state,
            HarnessState::ConnectedNoCalls,
            "the row must not claim its settings send calls here"
        );
        assert!(
            claude.wired,
            "upstream still recognises the line, which is what keeps disconnect offered"
        );
        assert!(
            harness_state::action_available(
                HarnessAction::Disconnect,
                claude.installed,
                claude.wired
            ),
            "the contributor must still have a way to remove the stale line"
        );

        // The same file IS ours once the port matches.
        let theirs = list(&Catalog::default(), &FamilyActivity::default(), Some(9999));
        let claude = theirs
            .iter()
            .find(|r| r.id == "claude")
            .expect("claude row");
        assert!(claude.connected, "the very same file names 9999");
    }

    /// With no catalog the list is the two built-in tools, and every row is
    /// still a full row -- installed or not.
    #[test]
    fn the_list_degrades_to_the_two_built_in_tools() {
        let rows = list(&Catalog::default(), &FamilyActivity::default(), Some(8463));
        let ids: Vec<&str> = rows.iter().map(|r| r.id.as_str()).collect();
        assert_eq!(ids, vec!["claude", "codex"]);
        assert_eq!(rows[0].family, Some("anthropic"));
        assert_eq!(rows[1].family, Some("openai"));
    }

    /// Every row carries a state, and with no readable ledger a connected
    /// tool is `unknown` rather than "no calls yet".
    #[test]
    fn no_ledger_never_reports_no_calls_yet() {
        let rows = list(&Catalog::default(), &FamilyActivity::default(), Some(8463));
        for row in rows {
            assert!(
                matches!(
                    row.state,
                    HarnessState::NotConnected | HarnessState::Unknown
                ),
                "{row:?}"
            );
        }
    }

    #[test]
    fn an_unknown_tool_id_is_refused_rather_than_planned() {
        let store = PlanStore::default();
        let err = plan(
            &store,
            &Catalog::default(),
            "not-a-tool",
            HarnessAction::Connect,
            Some(8463),
        )
        .expect_err("an unknown id has no plan");
        assert_eq!(err, ERR_UNKNOWN_HARNESS);
    }

    /// A connect with nowhere to send calls is refused by name, for EVERY
    /// tool, whatever is installed on the machine running this test.
    ///
    /// The earlier version asked only about `claude` and passed only on a
    /// machine that had Claude Code installed: `plan` consulted the tool's
    /// state first, so a missing tool answered `NotInstalled` and the refusal
    /// under test never happened. It passed on a developer laptop and failed
    /// on a clean checkout -- and would have started failing on that laptop
    /// too the moment the contributor connected Claude Code, which is the one
    /// thing this whole feature exists to let them do.
    ///
    /// Asserting it across the whole catalog rather than one id is what makes
    /// it independent of the machine: the answer must not depend on which of
    /// these happens to be installed.
    #[test]
    fn a_connect_with_no_destination_is_refused_by_name() {
        // Pin the config directory, and take the lock with it. Without this
        // the test reads whatever `CLAUDE_CONFIG_DIR` a concurrent test has
        // set: point it at an already-wired file and the connect is
        // unavailable, so the refusal never gets as far as the port.
        let (_dir, _guard, _path) = claude_config("{}");
        let store = PlanStore::default();
        let catalog = Catalog::default();
        let tools = tools::all(&catalog);
        assert!(!tools.is_empty(), "the built-ins are always present");

        for tool in &tools {
            let err = plan(&store, &catalog, &tool.id, HarnessAction::Connect, None)
                .expect_err("nothing is answering, so there is no port to name");
            assert_eq!(
                err, ERR_NO_DESTINATION,
                "{} must be refused for having nowhere to send calls, \
                 whatever its own state is",
                tool.id
            );
        }
    }

    #[test]
    fn a_plan_id_is_single_use() {
        let store = PlanStore::default();
        let id = Uuid::new_v4();
        assert!(commit(&store, id).is_err());
    }

    #[test]
    fn an_unheld_plan_id_is_named_rather_than_written() {
        let store = PlanStore::default();
        assert_eq!(
            commit(&store, Uuid::new_v4()).expect_err("nothing was minted"),
            ERR_PLAN_UNKNOWN
        );
    }

    // -----------------------------------------------------------------------
    // The catalog we own
    // -----------------------------------------------------------------------

    /// Every entry we compile in has to survive IronWire's own validation,
    /// because an entry that does not is silently dropped from
    /// `Catalog::agents` -- a tool that never appears, rather than an error.
    #[test]
    fn every_owned_entry_survives_validation() {
        for entry in owned_agents() {
            assert_eq!(entry.problem(), None, "{}", entry.id);
        }
    }

    /// The safety property. Checked here, over our own table, rather than
    /// only through `AgentEntry::problem`: the rules that bound which file we
    /// will edit are the reason a compiled-in table is acceptable at all, and
    /// they must not be able to widen underneath us when the pinned IronWire
    /// revision moves.
    #[test]
    fn no_owned_entry_names_a_config_path_outside_the_allowed_shape() {
        for entry in owned_agents() {
            assert!(
                config_location_is_allowed(&entry.config),
                "{} names a config path we will not edit",
                entry.id
            );
        }
    }

    /// The check itself has teeth. Written against the shapes that must stay
    /// refused, so the test above is a guard rather than a tautology while
    /// the table is short.
    #[test]
    fn the_config_shape_check_refuses_what_it_must() {
        let allowed = |dir: &[&str], file: &str| {
            config_location_is_allowed(&ConfigLocation {
                dir: dir.iter().map(|s| (*s).to_string()).collect(),
                file: file.to_string(),
            })
        };
        assert!(allowed(&[".claude"], "settings.json"));
        assert!(allowed(&[".config", "atool"], "atool.toml"));

        // No directory at all: a write straight into the home directory.
        assert!(!allowed(&[], "settings.json"));
        // Three segments: deep enough to reach somewhere unexpected.
        assert!(!allowed(&[".config", "a", "b"], "settings.json"));
        // Not a hidden directory: a source tree, or Documents.
        assert!(!allowed(&["config"], "settings.json"));
        // Traversal, in a segment of its own or smuggled inside one.
        assert!(!allowed(&[".config", ".."], "settings.json"));
        assert!(!allowed(&[".."], "settings.json"));
        assert!(!allowed(&[".config/../.ssh"], "settings.json"));
        // Extensionless secrets in a dotdir -- the case the extension rule
        // exists for.
        assert!(!allowed(&[".ssh"], "config"));
        assert!(!allowed(&[".aws"], "credentials"));
        // A format nothing here can write.
        assert!(!allowed(&[".continue"], "config.yaml"));
    }

    /// Every setting we compile in names a key we are willing to write.
    /// `Facade` has exactly two variants and the value is never ours, so
    /// this is a statement about the key alone.
    #[test]
    fn every_owned_setting_names_a_writable_key() {
        for entry in owned_agents() {
            assert!(!entry.settings.is_empty(), "{}", entry.id);
            for setting in &entry.settings {
                let usable = !setting.key.is_empty()
                    && setting.key.split('.').all(|segment| {
                        !segment.is_empty()
                            && segment
                                .chars()
                                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    });
                assert!(usable, "{} sets `{}`", entry.id, setting.key);
            }
        }
    }

    /// The list the daemon serves is built from our catalog, so every entry
    /// we add shows up beside the two built-in tools.
    #[test]
    fn every_owned_entry_appears_in_the_list() {
        let catalog = catalog();
        let ids: Vec<String> = tools::all(&catalog).into_iter().map(|t| t.id).collect();
        assert_eq!(ids[..2], ["claude".to_string(), "codex".to_string()]);
        assert_eq!(ids.len(), 2 + owned_agents().len());
        for entry in owned_agents() {
            assert!(ids.contains(&entry.id), "{} is missing", entry.id);
        }
    }

    /// `catalog_present` is a fact about our table, not a constant. Derived
    /// from `Catalog::default()` -- which ships no agents and, by its own
    /// test upstream, never will -- the claim could only ever be false.
    #[test]
    fn catalog_present_follows_our_own_table() {
        let catalog = catalog();
        assert_eq!(
            !catalog.agents().is_empty(),
            !owned_agents().is_empty(),
            "the wire field and the table must not be able to disagree"
        );
    }

    /// Nothing we compile in is dropped for a reason we never see.
    #[test]
    fn our_catalog_rejects_none_of_its_own_entries() {
        assert!(catalog().rejected_agents().is_empty());
    }
}
