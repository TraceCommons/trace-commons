//! Which projects may upload without asking.
//!
//! Autonomy is per-project and opt-in. An unknown project is `NotifyOnly`, so
//! a freshly installed daemon uploads nothing until the contributor has
//! deliberately opted in for a project or for all watched folders.
//!
//! The policy key is the session's true working directory. It is deliberately
//! *not* derived from the project basename: Claude Code encodes a cwd into a
//! directory name by replacing every `/` with `-`, which is ambiguous for any
//! hyphenated project name, and guessing wrong here would apply one project's
//! autonomy to a different project's traces.
//!
//! Sessions whose working directory cannot be resolved go to a single locked
//! bucket that can never be granted autonomy. Subagent transcripts and
//! normalized trajectory files land there. Since the daemon cannot tell which
//! project such a session belongs to, it cannot honour any opt-in for it.

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context, Result, bail};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::config::{ConfigStore, DAEMON_PROJECTS_FILE};

/// The current policy file schema.
///
/// Bumped to v2 when project keys became normalized
/// (`project_key::normalize_project_key`). A v1 file's keys are raw
/// recorded working directories, which no longer match anything
/// `project_key_for` produces, so `load` re-keys it. The version is what
/// makes that happen exactly once.
///
/// Bumped to v3 when `set_project_mode` began normalizing a caller's key
/// (#992). Until then a key `daemon project <path>` sent was stored in the
/// on-disk spelling, which the watcher -- folding case on macOS and Windows
/// -- never looked up, so a v2 file can hold keys that no session reaches.
/// `load` re-keys a v2 file once, with [`MovedArming::AsksAgain`].
pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v3";

/// The pre-normalization schema. Recognized only so `load` can migrate it.
pub const DAEMON_PROJECTS_SCHEMA_V1: &str = "trace_commons.daemon_projects.v1";

/// The schema whose keys the watcher wrote normalized and `set_project_mode`
/// did not. Recognized only so `load` can migrate it.
pub const DAEMON_PROJECTS_SCHEMA_V2: &str = "trace_commons.daemon_projects.v2";

/// What a re-key does with an `AutoUpload` entry whose key it changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MovedArming {
    /// Carry the mode across. Right for a v1 file, whose raw keys were the
    /// ones the watcher of the day looked up, so its arming was in effect.
    Carries,
    /// Return it to `NotifyOnly`. Right for a v2 file: a v2 key that
    /// normalization changes is one the watcher never matched, so the
    /// arming was never in effect, and its arm-from-now hold and claim are
    /// stored under the old spelling. Carrying the mode without them would
    /// send a backlog the contributor held back; asking again loses nothing.
    AsksAgain,
}

/// The bucket for sessions with no resolvable working directory. Permanently
/// notify-only.
pub const UNKNOWN_PROJECT_KEY: &str = "unknown-project";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectMode {
    /// Upload without asking.
    AutoUpload,
    /// Queue it and mention it in the next digest.
    NotifyOnly,
    /// Never offer sessions from this project at all.
    Ignore,
}

impl ProjectMode {
    /// The more restrictive of two modes.
    ///
    /// `Ignore` beats everything, then `NotifyOnly`, then `AutoUpload`.
    /// This is the merge rule when normalization collapses two policy
    /// entries into one key, and the direction is not arbitrary: merging
    /// toward the permissive mode would take a project the contributor had
    /// silenced and start offering it again, or take one they had left
    /// ask-first and upload from it unattended. A merge may only ever ask
    /// more permission than before, never less.
    pub fn more_restrictive(self, other: ProjectMode) -> ProjectMode {
        use ProjectMode::*;
        match (self, other) {
            (Ignore, _) | (_, Ignore) => Ignore,
            (NotifyOnly, _) | (_, NotifyOnly) => NotifyOnly,
            (AutoUpload, AutoUpload) => AutoUpload,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub mode: ProjectMode,
    /// When this mode was set. While a Flow 1 grant is in force it is
    /// never earlier than the grant's `granted_at`, though the clock be
    /// behind it (see [`ProjectPolicy::set_mode`]); and a rekey merge keeps
    /// the earlier of two. So it orders a mode after its grant, not on the
    /// wall clock alone.
    pub added_at: DateTime<Utc>,
    /// Display name for consumers. Shells render this; they never render the
    /// key, which is a full local path.
    pub label: String,
    /// The same directory as the map key, spelled the way the filesystem
    /// spells it.
    ///
    /// The key is case-folded on macOS and Windows so one project cannot
    /// mint two keys (see `project_key::NormalizedProject`). That fold is
    /// right for a lookup and wrong for a person: it renders `~/Code/Api`
    /// as `~/code/api`, a path that appears nowhere on their machine. This
    /// is the unfolded half, carried so rendering never has to guess.
    ///
    /// No new class of secret: this map is already keyed by the full local
    /// path, so the file this serializes into holds the same directory
    /// either way. `None` on a policy file written before the field
    /// existed, and on a key that no longer resolves -- both fall back to
    /// the folded key, which is honest about the directory even when it is
    /// not spelled the way the disk spells it.
    #[serde(default)]
    pub display_path: Option<String>,
    /// The terms this project was armed under, while it is `AutoUpload`.
    ///
    /// Compared on every watcher pass with the terms now in force; a change
    /// that widens who sees its sessions, or what leaves with them, voids the
    /// grant and returns the project to ask-first. See `grant_terms`.
    ///
    /// `None` for a project that is not armed, and for one armed before this
    /// field existed or by a route that could not read the config at the
    /// time. The watcher records the terms in force on its first pass over
    /// such a project -- a baseline, not a void, because there is no record
    /// of what was agreed to compare against. `#[serde(default)]` so an older
    /// policy file still loads.
    #[serde(default)]
    pub armed_under: Option<super::grant_terms::GrantTerms>,
    /// What redaction each session sent from this project on the
    /// contributor's behalf had, since it was last armed (R1, K6). Decides
    /// which arming disclosure the project gets: see
    /// `automatic_gate::project_disclosure`.
    ///
    /// Reset with the rest of the entry on every mode change, so it covers
    /// one arming only. Counts, never session identities: the policy file
    /// is not a history. `#[serde(default)]` so an older policy file loads,
    /// as "nothing recorded", which earns only the deterministic-only
    /// wording.
    #[serde(default)]
    pub automatic_redaction: AutomaticRedactionTally,
    /// The generation ([`AutomaticGrant::generation`]) of the Flow 1 grant
    /// in force when this mode was set, or `None` when none was. Written by
    /// [`ProjectPolicy::set_mode`] on every mode change and never by the
    /// clock, so a folder's Ask is an exception to the grant it was set
    /// under ([`ProjectPolicy::asks_despite_grant`]) however the wall clock
    /// moved in between, and to no later grant.
    ///
    /// `#[serde(default)]` so an older file loads; a file written before
    /// this existed has it derived once on load
    /// ([`ProjectPolicy::derive_grant_exceptions`]).
    #[serde(default)]
    pub set_under_grant: Option<u64>,
}

/// How many unattended sessions from one armed project had a certified full
/// redaction pipeline, and how many did not. See
/// [`ProjectEntry::automatic_redaction`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticRedactionTally {
    #[serde(default)]
    pub certified_full_pipeline: u32,
    #[serde(default)]
    pub not_certified: u32,
}

/// How many times a project must have contributed before the app offers to
/// arm it.
///
/// Five, because the offer has to be backed by evidence the contributor
/// actually has. Arming asks someone to stop reading previews from a
/// project; the only honest basis for that question is that they have read
/// several already and kept approving. One or two is a coincidence.
pub const ARMING_SUGGESTION_THRESHOLD: u32 = 5;

/// How long "Not now" silences the offer for one project.
///
/// Thirty days, which is the difference between an offer and nagging. It is
/// deliberately not permanent: "Not now" says not now, and a suppression
/// that never lifts would make those words a lie. Settings remains the way
/// to arm a project at any point in between, without being asked.
pub const ARMING_DECLINE_COOLDOWN_DAYS: i64 = 30;

/// What [`ProjectPolicy::sweep_grants`] did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GrantSweep {
    /// Projects returned to ask-first because their grant was widened.
    pub voided: Vec<VoidedGrant>,
    /// Why the Flow 1 grant itself was voided, if it was.
    pub automatic_grant_voided: Option<Vec<&'static str>>,
    /// Armed projects that had no recorded terms and now do.
    pub baselined: usize,
    /// Why an "Automatic" contribution override was voided, if it was.
    /// The override is cleared: every folder is back on its own mode.
    pub contribution_override_voided: Option<Vec<&'static str>>,
}

impl GrantSweep {
    /// Whether the policy changed and must be saved.
    pub fn changed(&self) -> bool {
        !self.voided.is_empty()
            || self.baselined > 0
            || self.automatic_grant_voided.is_some()
            || self.contribution_override_voided.is_some()
    }
}

/// The reason label an "Automatic" override is voided with when it
/// has no recorded terms to compare. Only a policy file written by an
/// unreleased build can hold one; it is voided rather than baselined (as a
/// legacy armed folder is) because there is no shipped state to protect.
pub const OVERRIDE_TERMS_UNRECORDED: &str = "terms-unrecorded";

/// One grant voided by a sweep, and why. Labels only.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VoidedGrant {
    pub project_label: String,
    pub reasons: Vec<&'static str>,
}

/// A void the contributor has not been shown yet.
///
/// R6 makes a void notice a ship condition: a grant that stops must say so
/// in every shell, not only in the audit. A sweep records one of these for
/// each project it returns to ask-first and one for the Flow 1 grant, and it
/// stays until a shell reports it shown (`acknowledge_grant_voids`) or the
/// contributor acts on what it is about -- sets that project's mode, or
/// gives the grant again -- which makes it stale.
///
/// Kept in the policy file, beside the grants it describes, so a void
/// during a pass no shell was watching is still shown at the next launch.
/// `project_key` is the policy's own key (a local path, as every key in this
/// file is); it never crosses the socket, which carries the `project_id` and
/// label derived from it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GrantVoidNotice {
    /// Unique within this policy file, so a shell acknowledges exactly the
    /// notices it showed and never one that arrived after it drew.
    pub id: u64,
    pub voided_at: DateTime<Utc>,
    /// The project voided, or `None` for the Flow 1 grant itself or for the
    /// contribution override (see `contribution_override`).
    pub project_key: Option<String>,
    /// Fixed reason labels, as `grant_terms::widening_from` gives them.
    pub reasons: Vec<String>,
    /// The "Automatic" contribution override was voided (#1173), not a
    /// project or the Flow 1 grant. `project_key` is `None`.
    /// `#[serde(default)]` so an older policy file still loads.
    #[serde(default)]
    pub contribution_override: bool,
}

/// K5: an armed folder whose arming words claimed more than the words it
/// would be shown now, not yet shown to the contributor.
///
/// The connect-and-forget design ("The prior decision") requires that a
/// folder armed under the old "will be scrubbed" copy, whose wording becomes
/// deterministic-only, is told what its arming now means: changing the words
/// under an armed folder must never happen silently. The folder stays armed;
/// this notice is the only thing that changes. See `arming_wording`.
///
/// Kept in the policy file, like [`GrantVoidNotice`], so a rewording during a
/// pass no shell was watching is shown at the next launch. It stays until a
/// shell reports it shown (`acknowledge_arming_rewordings`) or the
/// contributor sets that project's mode, which answers it. `project_key` is
/// the policy's own key and never crosses the socket.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmingRewordNotice {
    /// Unique within this policy file and never reused, for the reason
    /// [`GrantVoidNotice::id`] is.
    pub id: u64,
    pub reworded_at: DateTime<Utc>,
    pub project_key: String,
    /// What the words the folder was armed under claimed.
    pub was: super::arming_wording::ArmingClaim,
    /// What the words in force for it claim now.
    pub now: super::arming_wording::ArmingClaim,
    /// This notice announces the Automatic-default upgrade, not a claim
    /// narrowing. Existing shells pass the whole object to shared copy.
    #[serde(default)]
    pub scrub_check_defaulted: bool,
}

/// A project the app should offer to arm, and the evidence for offering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArmingSuggestion {
    pub project_id: String,
    pub project_label: String,
    pub contributed_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectPolicy {
    pub schema_version: String,
    pub projects: BTreeMap<String, ProjectEntry>,
    /// Successful uploads per project key, counted for the arming offer.
    ///
    /// Kept here rather than derived from the history cache because history
    /// is label-only by design -- it never carries a project key, so two
    /// projects sharing a final path segment are indistinguishable in it.
    /// Offering to arm the wrong repository on the strength of an ambiguous
    /// label is exactly the mistake this counter exists to avoid.
    ///
    /// `#[serde(default)]` so a policy file written before this existed
    /// still parses, and reads as "nothing counted yet" rather than failing
    /// the whole file.
    #[serde(default)]
    pub contributed: BTreeMap<String, u32>,
    /// When the contributor last said "Not now" to arming a project.
    #[serde(default)]
    pub arming_declined_at: BTreeMap<String, DateTime<Utc>>,
    /// The Flow 1 grant, if one is in force: arm projects discovered after
    /// it. See [`AutomaticGrant`].
    #[serde(default)]
    pub automatic_grant: Option<AutomaticGrant>,
    /// Every session path any automatic grant found on disk, per source, when
    /// it recorded that source. Kept after the grant is withdrawn or voided,
    /// because a project the grant armed stays armed and must still never
    /// send what predates the grant. Cleared only with the rest of this file,
    /// at logout.
    #[serde(default)]
    pub sessions_on_disk_at_grant: BTreeSet<String>,
    /// Projects an automatic grant armed, as opposed to the contributor.
    /// A session in `sessions_on_disk_at_grant` is never approved unattended
    /// in one of these, whichever project it reads as now. Leaves the set when
    /// the contributor sets the project's mode themselves.
    #[serde(default)]
    pub armed_by_grant: BTreeSet<String>,
    /// Voids not yet shown to the contributor. See [`GrantVoidNotice`].
    #[serde(default)]
    pub grant_voids: Vec<GrantVoidNotice>,
    /// The id the next [`GrantVoidNotice`] gets. Never reused, so an
    /// acknowledgement naming an old id cannot clear a newer notice.
    #[serde(default)]
    pub next_grant_void_id: u64,
    /// What the words each armed project was armed under claimed, recorded
    /// at arming. A project armed before this existed has no entry and is
    /// read through `arming_wording::legacy_claim`. Kept beside the entries
    /// rather than in [`ProjectEntry`] so an older file still loads, and
    /// cleared whenever the project's mode is set. See `arming_wording`.
    #[serde(default)]
    pub arming_claims: BTreeMap<String, super::arming_wording::ArmingClaim>,
    /// Rewordings not yet shown to the contributor. See
    /// [`ArmingRewordNotice`].
    ///
    /// Owner decision 2026-10-16: no switch-on notice is raised; the list
    /// stays on the wire and is always empty. A file written by an older
    /// build is cleared on load.
    #[serde(default)]
    pub arming_rewordings: Vec<ArmingRewordNotice>,
    /// The id the next [`ArmingRewordNotice`] gets. Never reused.
    #[serde(default)]
    pub next_arming_reword_id: u64,
    /// Recorded atomically with upgrade notices, never cleared by their ack.
    #[serde(default)]
    pub scrub_check_upgrade_recorded: bool,
    /// Projects the contributor armed **from now** (`set_project_mode`
    /// `auto_upload` with `include_backlog: false`, or a re-send that keeps
    /// an earlier hold), by project key. See
    /// [`ArmedFromNow`]. An entry leaves when the project's mode is set
    /// again by any route, since that is a fresh decision about the project.
    #[serde(default)]
    pub armed_from_now: BTreeMap<String, ArmedFromNow>,
    /// Every session path an arm-from-now record found on disk, with the
    /// sequence number of the **first** record that saw it. See
    /// [`ArmedFromNow`] for why this is ordered rather than a set like
    /// `sessions_on_disk_at_grant`.
    #[serde(default)]
    pub sessions_on_disk_at_arming: BTreeMap<String, u64>,
    /// The sequence number the next arm-from-now record takes. Never reused.
    #[serde(default)]
    pub next_arming_record: u64,
    /// The menu-bar pill's global override (R13, #1202), if one is in
    /// force. See [`ContributionOverride`]. Per-folder modes in `projects`
    /// are never written by it, so clearing it restores each folder exactly.
    /// `#[serde(default)]` so an older policy file loads with none.
    #[serde(default)]
    pub contribution_override: Option<ContributionOverride>,
    /// Whether this file was first written by a build that has the per-kind
    /// notification switches (`DaemonSettings::notify`). Set by
    /// [`ProjectPolicy::new`] and never changed, so a file an older build
    /// wrote keeps reading `false` -- that build has no such key -- however
    /// often this build saves it.
    ///
    /// `DaemonSettings::load` reads it when there is no settings file: a
    /// policy an older build wrote is an existing install, and gets the
    /// upgrade values and the one-time offers (constraint 12), even when
    /// that build already marked `scrub_check_upgrade_recorded`.
    #[serde(default)]
    pub notify_kinds_known: bool,
    /// The last [`AutomaticGrant::generation`] handed out. Only ever
    /// incremented, so no two fresh global choices share one. `0` on a
    /// file written before generations existed.
    #[serde(default)]
    pub grant_generation: u64,
    /// Whether every folder's [`ProjectEntry::set_under_grant`] was written
    /// by a build that records it. Set by [`ProjectPolicy::new`]; a file the
    /// #1747 build wrote reads `false`, and
    /// [`ProjectPolicy::derive_grant_exceptions`] derives the field once
    /// from that build's rule on load, then sets this.
    #[serde(default)]
    pub grant_exceptions_derived: bool,
}

/// The key the contribution override's arming record is listed under in
/// [`ProjectPolicy::armings_from_now`]. Not a path, so never a project key
/// (`project_key_for` mints absolute paths or [`UNKNOWN_PROJECT_KEY`]).
pub const OVERRIDE_ARMING_KEY: &str = "contribution-override";

/// A temporary global contribution mode over every folder (#1173, the
/// menu-bar Contribution mode pill). Set by `set_contribution_override`,
/// cleared by `clear_contribution_override`.
///
/// **Per-folder modes are never written by it.** [`ProjectPolicy::resolve`]
/// reads it over the folder's own mode, and clearing it is dropping this
/// value: every folder is back on exactly the mode it had, because nothing
/// else changed.
///
/// What each mode means, where a stricter rule wins:
///
/// - `Ignore` ("Never"): every folder resolves to `Ignore`. Nothing is
///   queued or sent from any folder. Waiting entries are left waiting rather
///   than refused, so clearing the override restores them. Entries the
///   contributor already approved are held, not sent, and `approve` is
///   refused ([`ProjectPolicy::holds_every_send`]); clearing the override
///   releases them as they were.
/// - `NotifyOnly` ("Ask"): every folder resolves to `NotifyOnly`, except
///   a folder set to `Ignore`, which stays `Ignore`. Nothing goes unattended.
/// - `AutoUpload` ("Automatic"): every folder resolves to
///   `AutoUpload`, **except a folder set to `Ignore`, which stays `Ignore`**
///   (a global override never reaches into a folder the contributor
///   excluded), and the unknown bucket, which stays `NotifyOnly`. It **arms
///   nothing already on disk**: for every folder not already armed by its own
///   mode, the override is an arming from now ([`ArmedFromNow`], recorded in
///   `arming`), so a session on disk when the override began -- queued or not
///   -- waits for a person. A folder already armed keeps its own holds (the
///   grant's, its own arming from now). Every other gate still applies: the
///   Scrub check (K4), review holds, `automatic_gate`.
///
/// **An `AutoUpload` override is a grant** (owner decision on #1208), held to
/// everything a per-folder arming is: it cannot be set without the
/// `GrantTerms` in force (`arming-terms-unavailable`), it records those
/// terms (`granted_under`) and the claim its words made (`claim`), and
/// [`ProjectPolicy::sweep_grants`] voids it when the terms widen (R6) --
/// clearing it, so every folder is back on its own mode, with a void notice.
/// K5 rewordings reach it through [`ProjectPolicy::sweep_override_claim`].
/// It creates no `automatic_grant` (the Flow 1 grant is a separate record).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContributionOverride {
    pub mode: ProjectMode,
    pub since: DateTime<Utc>,
    /// The arming-from-now record for an `AutoUpload` override; `None` for
    /// any other mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arming: Option<ArmedFromNow>,
    /// The terms an `AutoUpload` override was granted under, as
    /// [`ProjectEntry::armed_under`] is for a folder. `None` for any other
    /// mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub granted_under: Option<super::grant_terms::GrantTerms>,
    /// What the words an `AutoUpload` override was set under claimed, as
    /// [`ProjectPolicy::arming_claims`] records for a folder. `None` for any
    /// other mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claim: Option<super::arming_wording::ArmingClaim>,
}

/// K5: a project armed **from now** (`include_backlog: false`), where the
/// backlog goes only when the contributor picks it. Since 2026-10-19 this is
/// no longer the default: switching a folder to Automatic includes its
/// backlog unless the caller asks for from now.
///
/// The same rule as the Flow 1 grant ([`AutomaticGrant`]), applied to one
/// project the contributor armed themselves: **a session already on disk
/// when the project was armed -- queued or not -- is never approved
/// unattended in it**, whichever project it reads as now. It waits for the
/// contributor, who can still approve it by hand (the past-session picker is
/// per-entry `approve`). A session that first appears after the arming is
/// approved unattended like any other session in an armed project.
///
/// Recorded the way the grant records the disk, **per source**, by a full
/// watcher pass, before any session is visited, and only for an arming made
/// before that pass listed the disk. Until a source has been recorded for
/// this arming, nothing from that source is approved unattended in this
/// project: a harness connected after the arming, one re-rooted since, or one
/// whose discovery failed has its history recorded before it can send
/// anything. That is the fail-closed direction -- a session created between
/// the arming and the recording pass waits for a person too.
///
/// Why the paths are kept in an ordered map
/// (`sessions_on_disk_at_arming`) and not added to
/// `sessions_on_disk_at_grant`: that set means "on disk at *a grant*", and
/// every project the grant armed holds all of it back. Folding a later
/// project's arming into it would hold back sessions that are new relative
/// to the grant. Each path instead remembers the first record that saw it,
/// and each arming the record it took per source, so "on disk when *this*
/// project was armed" is one comparison and the paths are stored once, not
/// once per armed project.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArmedFromNow {
    /// When the contributor armed the project. A pass records only for the
    /// arming it read before listing the disk.
    pub armed_at: DateTime<Utc>,
    /// Sources recorded for this arming, by source key, with the sequence
    /// number of the record that covered it.
    #[serde(default)]
    pub recorded_sources: BTreeMap<String, u64>,
}

/// The Flow 1 grant. Fresh onboarding can explicitly include existing
/// eligible history from every watched folder; older grants retain their
/// from-now scope. Under an include-existing grant a folder set to Never, or
/// set to Ask after the grant, is excluded from it
/// ([`ProjectPolicy::asks_despite_grant`]).
///
/// By default it arms nothing already on disk, recorded
/// **per source**. A source's first successful discovery under the grant
/// records its sessions and their projects and arms nothing from that pass;
/// until a source is recorded none of its sessions arms anything. So a
/// harness connected after the grant -- the normal onboarding order -- or
/// pointed at another root, or whose discovery failed on the first pass, is
/// recorded before it can arm, rather than having its history counted as
/// new. A project with a session on disk in any recorded source keeps asking,
/// and a session on disk at a grant is never approved unattended in a project
/// the grant armed (`sessions_on_disk_at_grant`, `armed_by_grant`).
///
/// That is also what makes a re-grant after logout safe. Logout wipes this
/// file, a Never folder's decision with it; the re-grant records the disk
/// again, so every folder that already had sessions asks rather than being
/// counted as new. Only a fresh explicit `include_existing` grant expands
/// that scope; loading an older grant never does.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticGrant {
    pub granted_at: DateTime<Utc>,
    /// The terms the grant was given under. A widening voids the grant
    /// itself as well as the projects it armed (R6), or it would go on arming
    /// new projects under terms nobody agreed to.
    pub granted_under: super::grant_terms::GrantTerms,
    /// A fresh onboarding choice to include existing eligible sessions in
    /// every watched folder, including sources connected later. Missing on
    /// older grants, which retain their from-now snapshot protection.
    #[serde(default)]
    pub include_existing: bool,
    /// Sources recorded so far, by name and root.
    #[serde(default)]
    pub recorded_sources: BTreeSet<String>,
    /// Projects with a session on disk in a recorded source.
    #[serde(default)]
    pub projects_on_disk: BTreeSet<String>,
    /// This grant's identity as a fresh global choice: a new value from
    /// [`ProjectPolicy::grant_generation`] on every
    /// [`ProjectPolicy::grant_automatic`] and every
    /// [`ProjectPolicy::include_existing_under_grant`]. A folder's Ask is an
    /// exception only to the generation it was set under
    /// ([`ProjectEntry::set_under_grant`]); a later fresh choice supersedes
    /// it. An identity, not an instant, so a clock that steps backwards
    /// cannot reorder the two. `0` only on a grant an older build wrote,
    /// which [`ProjectPolicy::derive_grant_exceptions`] replaces on load.
    #[serde(default)]
    pub generation: u64,
}

impl Default for ProjectPolicy {
    fn default() -> Self {
        Self::new()
    }
}

impl ProjectPolicy {
    pub fn new() -> Self {
        Self {
            schema_version: DAEMON_PROJECTS_SCHEMA.to_string(),
            projects: BTreeMap::new(),
            contributed: BTreeMap::new(),
            arming_declined_at: BTreeMap::new(),
            automatic_grant: None,
            sessions_on_disk_at_grant: BTreeSet::new(),
            armed_by_grant: BTreeSet::new(),
            grant_voids: Vec::new(),
            next_grant_void_id: 0,
            arming_claims: BTreeMap::new(),
            arming_rewordings: Vec::new(),
            next_arming_reword_id: 0,
            // Fresh installs start with Automatic; only old files (whose
            // serde default is false) need an upgrade announcement.
            scrub_check_upgrade_recorded: true,
            armed_from_now: BTreeMap::new(),
            sessions_on_disk_at_arming: BTreeMap::new(),
            next_arming_record: 0,
            contribution_override: None,
            notify_kinds_known: true,
            grant_generation: 0,
            grant_exceptions_derived: true,
        }
    }

    /// The folder's own mode, ignoring any contribution override: what
    /// clearing the override returns it to. The unknown bucket is never
    /// `AutoUpload`, as in [`Self::resolve`].
    pub fn folder_mode(&self, project_key: &str) -> ProjectMode {
        let stored = self
            .projects
            .get(project_key)
            .map(|e| e.mode)
            .unwrap_or(ProjectMode::NotifyOnly);
        if project_key == UNKNOWN_PROJECT_KEY && stored == ProjectMode::AutoUpload {
            return ProjectMode::NotifyOnly;
        }
        stored
    }

    /// Set the contribution override to `mode` at `now`. Setting the mode
    /// already in force changes nothing, so a shell re-sending it neither
    /// moves `since` nor releases an `AutoUpload` override's held backlog.
    /// Returns whether anything changed. See [`ContributionOverride`].
    ///
    /// `grant` is the terms in force and the claim the confirmation's words
    /// made. An `AutoUpload` override is a grant, so without them it is
    /// refused with `arming-terms-unavailable` and nothing changes; any
    /// other mode ignores them. Setting an override answers any outstanding
    /// notice that the override had been voided.
    pub fn set_contribution_override(
        &mut self,
        mode: ProjectMode,
        now: DateTime<Utc>,
        grant: Option<(
            super::grant_terms::GrantTerms,
            super::arming_wording::ArmingClaim,
        )>,
    ) -> Result<bool> {
        let grant = match (mode, grant) {
            (ProjectMode::AutoUpload, None) => bail!("arming-terms-unavailable"),
            (ProjectMode::AutoUpload, Some(grant)) => Some(grant),
            _ => None,
        };
        if self
            .contribution_override
            .as_ref()
            .is_some_and(|o| o.mode == mode)
        {
            return Ok(false);
        }
        let (granted_under, claim) = grant.unzip();
        self.contribution_override = Some(ContributionOverride {
            mode,
            since: now,
            arming: (mode == ProjectMode::AutoUpload).then(|| ArmedFromNow {
                armed_at: now,
                recorded_sources: BTreeMap::new(),
            }),
            granted_under,
            claim,
        });
        self.grant_voids.retain(|n| !n.contribution_override);
        self.arming_rewordings
            .retain(|n| n.project_key != OVERRIDE_ARMING_KEY);
        self.prune_arming_record();
        Ok(true)
    }

    /// Whether every send is held: a "Never" contribution override is in
    /// force. Nothing leaves while it is, not even an entry the contributor
    /// approved, and `approve` is refused, so "Nothing is queued or sent"
    /// is true. Clearing the override releases what was held as it was.
    pub fn holds_every_send(&self) -> bool {
        self.contribution_override
            .as_ref()
            .is_some_and(|o| o.mode == ProjectMode::Ignore)
    }

    /// Clear the contribution override. Every folder is back on its own
    /// mode, which the override never touched. Returns whether one was in
    /// force.
    pub fn clear_contribution_override(&mut self) -> bool {
        let cleared = self.contribution_override.take().is_some();
        if cleared {
            // No longer armed, so a notice about what its arming now means
            // no longer applies.
            self.arming_rewordings
                .retain(|n| n.project_key != OVERRIDE_ARMING_KEY);
            self.prune_arming_record();
        }
        cleared
    }

    /// The menu-bar pill's roll-up (#1173): the override's mode while one is
    /// in force; otherwise the one mode every folder shares, or `None` when
    /// they differ ("Mixed").
    ///
    /// "Every folder" is every configured folder and every key in
    /// `discovered` (the queue's), by its own mode as shown
    /// ([`Self::shown_folder_mode`], so a folder the include-existing grant
    /// will arm counts as Automatic, as its row reads),
    /// excluding the unknown bucket, which can never be armed and so would
    /// make any all-automatic set read as mixed. With no folders it is
    /// `NotifyOnly`, the default every unruled folder resolves to.
    pub fn contribution_mode<'a>(
        &self,
        discovered: impl IntoIterator<Item = &'a str>,
    ) -> Option<ProjectMode> {
        if let Some(o) = &self.contribution_override {
            return Some(o.mode);
        }
        // A folder counted twice changes nothing: only equality is asked.
        let configured = self
            .projects
            .keys()
            .map(|k| self.folder_mode_unless_unknown(k));
        let queued = discovered
            .into_iter()
            .map(|k| self.folder_mode_unless_unknown(k));
        let mut modes = configured.chain(queued).flatten();
        let Some(first) = modes.next() else {
            return Some(ProjectMode::NotifyOnly);
        };
        modes.all(|m| m == first).then_some(first)
    }

    /// Whether an `AutoUpload` roll-up ([`Self::contribution_mode`]) leaves
    /// some folders out (#1208): a folder whose own mode is Never, or the
    /// unknown bucket, among the configured folders and `discovered`. Under
    /// an "Automatic" override neither uploads, so the pill says
    /// "Automatic" with a sub-line rather than overstating it. False
    /// for any other roll-up.
    pub fn contribution_mode_partial<'a>(
        &self,
        discovered: impl IntoIterator<Item = &'a str> + Clone,
    ) -> bool {
        if self.contribution_mode(discovered.clone()) != Some(ProjectMode::AutoUpload) {
            return false;
        }
        let left_out =
            |k: &str| k == UNKNOWN_PROJECT_KEY || self.folder_mode(k) == ProjectMode::Ignore;
        self.projects.keys().any(|k| left_out(k)) || discovered.into_iter().any(left_out)
    }

    fn folder_mode_unless_unknown(&self, project_key: &str) -> Option<ProjectMode> {
        (project_key != UNKNOWN_PROJECT_KEY).then(|| self.shown_folder_mode(project_key))
    }

    /// Whether the include-existing grant in force will arm `project_key`
    /// on its next pass: a real folder whose own mode is Ask and that is not
    /// an exception to the grant ([`Self::asks_despite_grant`]) -- one set
    /// to Ask before the grant, or one with no entry yet. The same test as
    /// [`Self::arms_by_default`]'s include-existing branch, less the
    /// recorded source, which a listing has no session to name. Reading
    /// Automatic before the source is recorded only ever says more is sent
    /// than is, never less.
    pub fn grant_will_arm(&self, project_key: &str) -> bool {
        self.includes_existing_automatically()
            && project_key != UNKNOWN_PROJECT_KEY
            && self.folder_mode(project_key) == ProjectMode::NotifyOnly
            && !self.asks_despite_grant(project_key)
    }

    /// [`Self::folder_mode`] as a shell shows it: Automatic for a folder the
    /// include-existing grant will arm ([`Self::grant_will_arm`]), because
    /// that is what the daemon will do with its sessions. Showing it as Ask
    /// would say a person decides while the grant sends, and a shell could
    /// not offer Ask on it as a change. Never, a post-grant Ask, and the
    /// unknown bucket read as their own mode; with no include-existing grant
    /// this is `folder_mode`.
    pub fn shown_folder_mode(&self, project_key: &str) -> ProjectMode {
        if self.grant_will_arm(project_key) {
            ProjectMode::AutoUpload
        } else {
            self.folder_mode(project_key)
        }
    }

    /// [`Self::resolve`] as a shell shows it: the contribution override read
    /// over [`Self::shown_folder_mode`] by the same rules.
    pub fn shown_mode(&self, project_key: &str) -> ProjectMode {
        self.resolve_over(self.shown_folder_mode(project_key), project_key)
    }

    /// The arming-from-now record that holds `project_key`'s backlog, if
    /// any: its own, or an `AutoUpload` override's when the folder is not
    /// armed by its own mode. At most one applies, because a folder with an
    /// own record is armed by its own mode.
    fn arming_for(&self, project_key: &str) -> Option<&ArmedFromNow> {
        if let Some(own) = self.armed_from_now.get(project_key) {
            return Some(own);
        }
        let o = self.contribution_override.as_ref()?;
        if o.mode != ProjectMode::AutoUpload
            || self.folder_mode(project_key) == ProjectMode::AutoUpload
        {
            return None;
        }
        o.arming.as_ref()
    }

    /// An arming record by the key [`Self::armings_from_now`] lists it
    /// under.
    fn arming_record(&self, key: &str) -> Option<&ArmedFromNow> {
        if key == OVERRIDE_ARMING_KEY {
            return self
                .contribution_override
                .as_ref()
                .and_then(|o| o.arming.as_ref());
        }
        self.armed_from_now.get(key)
    }

    fn arming_record_mut(&mut self, key: &str) -> Option<&mut ArmedFromNow> {
        if key == OVERRIDE_ARMING_KEY {
            return self
                .contribution_override
                .as_mut()
                .and_then(|o| o.arming.as_mut());
        }
        self.armed_from_now.get_mut(key)
    }

    /// Arm `project_key` from now (K5). The caller has already set the mode
    /// to `AutoUpload` through [`Self::set_mode`], which cleared any earlier
    /// arming record, so this starts a fresh one: nothing is recorded yet,
    /// and until a full pass records a source nothing from it is approved
    /// unattended here. See [`ArmedFromNow`].
    pub fn arm_from_now(&mut self, project_key: &str, now: DateTime<Utc>) {
        self.armed_from_now.insert(
            project_key.to_string(),
            ArmedFromNow {
                armed_at: now,
                recorded_sources: BTreeMap::new(),
            },
        );
    }

    /// Put back an arming-from-now record that [`Self::set_mode`] just
    /// cleared: a re-arm from now over an arming from now keeps the hold it
    /// had, rather than starting a new one.
    pub fn restore_arming_from_now(&mut self, project_key: &str, prior: ArmedFromNow) {
        self.armed_from_now.insert(project_key.to_string(), prior);
    }

    /// When `project_key` was armed from now, if it was.
    /// Also answers for a folder an `AutoUpload` contribution override arms.
    pub fn armed_from_now_at(&self, project_key: &str) -> Option<DateTime<Utc>> {
        self.arming_for(project_key).map(|a| a.armed_at)
    }

    /// Defence in depth for "on disk at the arming", which the record reads
    /// by path: content older than the arming that turns up at a **new**
    /// path -- a resumed conversation written to a fresh file, a restore, a
    /// sync -- was never listed by the record. The watcher calls this when a
    /// session's first event, or its file's birth time, predates the arming;
    /// the path is then recorded as on disk for that arming (the earliest
    /// record wins), so it is held like the rest of the backlog. Returns
    /// whether anything changed. A source not yet recorded holds everything
    /// already, and its record will list this path.
    pub fn hold_for_arming(
        &mut self,
        project_key: &str,
        session_path: &str,
        source_key: &str,
    ) -> bool {
        let Some(seq) = self
            .arming_for(project_key)
            .and_then(|a| a.recorded_sources.get(source_key).copied())
        else {
            return false;
        };
        match self.sessions_on_disk_at_arming.get_mut(session_path) {
            Some(first) if *first <= seq => false,
            Some(first) => {
                *first = seq;
                true
            }
            None => {
                self.sessions_on_disk_at_arming
                    .insert(session_path.to_string(), seq);
                true
            }
        }
    }

    /// Drop recorded paths no arming in force can still hold: one first seen
    /// by a record later than every arming's own records holds nothing, and
    /// with no arming from now left the record is empty. Returns whether
    /// anything was dropped.
    pub fn prune_arming_record(&mut self) -> bool {
        let before = self.sessions_on_disk_at_arming.len();
        let override_arming = self
            .contribution_override
            .as_ref()
            .and_then(|o| o.arming.as_ref());
        match self
            .armed_from_now
            .values()
            .chain(override_arming)
            .flat_map(|a| a.recorded_sources.values().copied())
            .max()
        {
            Some(latest) => self
                .sessions_on_disk_at_arming
                .retain(|_, first| *first <= latest),
            None if self.armed_from_now.is_empty() && override_arming.is_none() => {
                self.sessions_on_disk_at_arming.clear()
            }
            // Armed from now, nothing recorded yet: everything is held
            // anyway, and the coming record decides what stays.
            None => {}
        }
        self.sessions_on_disk_at_arming.len() != before
    }

    /// Whether `project_key` is armed from now rather than with its backlog.
    pub fn is_armed_from_now(&self, project_key: &str) -> bool {
        self.armed_from_now.contains_key(project_key)
    }

    /// The arm-from-now armings in force, by project key and instant. A pass
    /// reads this before it lists anything and records only for these, so
    /// an arming made while discovery walks the disk is recorded from a
    /// later listing -- the rule [`Self::grant_id`] follows for the grant.
    ///
    /// An `AutoUpload` contribution override's record is listed too, under
    /// [`OVERRIDE_ARMING_KEY`], so the same pass records it the same way.
    pub fn armings_from_now(&self) -> Vec<(String, DateTime<Utc>)> {
        let override_arming = self
            .contribution_override
            .as_ref()
            .and_then(|o| o.arming.as_ref())
            .map(|a| (OVERRIDE_ARMING_KEY.to_string(), a.armed_at));
        self.armed_from_now
            .iter()
            .map(|(key, a)| (key.clone(), a.armed_at))
            .chain(override_arming)
            .collect()
    }

    /// Whether any of `armings` still needs `source_key` recorded.
    pub fn needs_arming_record(
        &self,
        armings: &[(String, DateTime<Utc>)],
        source_key: &str,
    ) -> bool {
        armings.iter().any(|(key, at)| {
            self.arming_record(key)
                .is_some_and(|a| a.armed_at == *at && !a.recorded_sources.contains_key(source_key))
        })
    }

    /// Record what one source had on disk for each of `armings` still in
    /// force unchanged and not yet recorded for it. Returns whether anything
    /// was recorded. A path keeps the first record that saw it.
    pub fn record_source_for_armings(
        &mut self,
        armings: &[(String, DateTime<Utc>)],
        source_key: &str,
        sessions: BTreeSet<String>,
    ) -> bool {
        if !self.needs_arming_record(armings, source_key) {
            return false;
        }
        let seq = self.next_arming_record;
        self.next_arming_record = seq.saturating_add(1);
        for path in sessions {
            self.sessions_on_disk_at_arming.entry(path).or_insert(seq);
        }
        for (key, at) in armings {
            if let Some(a) = self.arming_record_mut(key) {
                if a.armed_at == *at {
                    a.recorded_sources
                        .entry(source_key.to_string())
                        .or_insert(seq);
                }
            }
        }
        true
    }

    /// Whether the session at `session_path`, from `source_key`, must not be
    /// approved unattended in `project_key` because the project is armed
    /// from now and the session was on disk at the arming -- or its source
    /// has not been recorded for the arming yet, which fails closed.
    pub fn holds_back_from_arming(
        &self,
        project_key: &str,
        session_path: &str,
        source_key: &str,
    ) -> bool {
        let Some(arming) = self.arming_for(project_key) else {
            return false;
        };
        match arming.recorded_sources.get(source_key) {
            None => true,
            Some(recorded) => self
                .sessions_on_disk_at_arming
                .get(session_path)
                .is_some_and(|first| first <= recorded),
        }
    }

    /// The send-time form of [`Self::holds_back_from_arming`], for a caller
    /// that holds a queue entry and not the source key it was discovered
    /// under. Stricter in the only direction it can be: no source recorded
    /// yet holds everything, and a path is held if it was on disk at the
    /// record of any source for this arming.
    pub fn holds_back_from_arming_at_send(&self, project_key: &str, session_path: &str) -> bool {
        let Some(arming) = self.arming_for(project_key) else {
            return false;
        };
        if arming.recorded_sources.is_empty() {
            return true;
        }
        self.sessions_on_disk_at_arming
            .get(session_path)
            .is_some_and(|first| arming.recorded_sources.values().any(|r| first <= r))
    }

    /// The send-time form of [`Self::waits_for_a_person`], for the upload
    /// pass, which holds a queue entry and not its source key: the grant's
    /// hold, and [`Self::holds_back_from_arming_at_send`].
    pub fn waits_for_a_person_at_send(&self, project_key: &str, session_path: &str) -> bool {
        self.holds_back_unattended(project_key, session_path)
            || self.holds_back_from_arming_at_send(project_key, session_path)
    }

    /// The one question both unattended approval sites ask: must this
    /// session wait for the contributor although its project is armed?
    /// Either the grant armed the project and the session predates a grant
    /// ([`Self::holds_back_unattended`]), or the contributor armed it from
    /// now and the session predates that ([`Self::holds_back_from_arming`]).
    pub fn waits_for_a_person(
        &self,
        project_key: &str,
        session_path: &str,
        source_key: &str,
    ) -> bool {
        self.holds_back_unattended(project_key, session_path)
            || self.holds_back_from_arming(project_key, session_path, source_key)
    }

    /// Give the legacy from-now Flow 1 grant, replacing any earlier one.
    /// Fresh onboarding can explicitly widen it to existing sessions before
    /// persisting. Every grant waits until its source has been recorded.
    pub fn grant_automatic(&mut self, now: DateTime<Utc>, terms: super::grant_terms::GrantTerms) {
        // Giving the grant again answers a notice that it had stopped, and
        // only that one: a project's, or the contribution override's, is
        // about something else.
        self.grant_voids
            .retain(|n| n.project_key.is_some() || n.contribution_override);
        let generation = self.next_grant_generation();
        self.automatic_grant = Some(AutomaticGrant {
            granted_at: now,
            granted_under: terms,
            include_existing: false,
            recorded_sources: BTreeSet::new(),
            projects_on_disk: BTreeSet::new(),
            generation,
        });
    }

    /// A [`AutomaticGrant::generation`] no grant has had: one past the last
    /// handed out, and recorded as handed out. Never `0`, which marks a
    /// grant an older build wrote.
    fn next_grant_generation(&mut self) -> u64 {
        self.grant_generation = self.grant_generation.saturating_add(1);
        self.grant_generation
    }

    /// The identity of the Flow 1 grant in force, if any. See
    /// [`AutomaticGrant::generation`].
    pub fn grant_generation_in_force(&self) -> Option<u64> {
        self.automatic_grant.as_ref().map(|g| g.generation)
    }

    /// Whether the confirmed onboarding grant covers existing history.
    pub fn includes_existing_automatically(&self) -> bool {
        self.automatic_grant
            .as_ref()
            .is_some_and(|grant| grant.include_existing)
    }

    /// Expand the fresh grant to all watched history. A fresh global choice
    /// supersedes old global modes, old per-folder Ask modes and per-folder
    /// Automatic-from-now limits, while folder Never and every per-session
    /// decision remain in force. A folder set to Ask after this is an
    /// exception to it, as Never is ([`Self::asks_despite_grant`]).
    pub fn include_existing_under_grant(&mut self) {
        if self.automatic_grant.is_none() {
            return;
        }
        // A fresh global choice, so a fresh identity: every folder Ask set
        // under the grant before this is superseded by it.
        let generation = self.next_grant_generation();
        let Some(grant) = self.automatic_grant.as_mut() else {
            return;
        };
        grant.include_existing = true;
        grant.generation = generation;
        self.clear_contribution_override();
        let projects = &self.projects;
        self.armed_from_now.retain(|key, _| {
            projects
                .get(key)
                .is_none_or(|entry| entry.mode != ProjectMode::AutoUpload)
        });
        self.prune_arming_record();
    }

    /// Withdraw the Flow 1 grant. Projects it already armed keep their own
    /// entries, and still never send what was on disk at it; withdrawing a
    /// project is `set_mode`. Returns whether one was in force.
    pub fn withdraw_automatic_grant(&mut self) -> bool {
        self.automatic_grant.take().is_some()
    }

    /// Which grant is in force, by the instant it was given. A pass reads
    /// this before it lists anything, and records only for the grant it
    /// read, so a grant given while discovery was walking the disk is
    /// recorded from a later listing, not that one.
    pub fn grant_id(&self) -> Option<DateTime<Utc>> {
        self.automatic_grant.as_ref().map(|g| g.granted_at)
    }

    /// Whether `source_key` still has to be recorded for the grant `grant`.
    pub fn needs_source_record(&self, grant: DateTime<Utc>, source_key: &str) -> bool {
        self.automatic_grant
            .as_ref()
            .is_some_and(|g| g.granted_at == grant && !g.recorded_sources.contains(source_key))
    }

    /// Record what one source had on disk, for the grant `grant` only, and
    /// only once per source. Returns whether anything was recorded.
    pub fn record_source(
        &mut self,
        grant: DateTime<Utc>,
        source_key: &str,
        sessions: BTreeSet<String>,
        projects: BTreeSet<String>,
    ) -> bool {
        match self.automatic_grant.as_mut() {
            Some(g) if g.granted_at == grant && !g.recorded_sources.contains(source_key) => {
                g.recorded_sources.insert(source_key.to_string());
                g.projects_on_disk.extend(projects);
                self.sessions_on_disk_at_grant.extend(sessions);
                true
            }
            _ => false,
        }
    }

    /// Whether the grant arms `project_key` on seeing the session at
    /// `session_path` from `source_key`: a recorded source, a real project
    /// with no entry of its own, and neither the project nor the session on
    /// disk when the grant recorded it. An explicit include-existing grant
    /// also arms Ask folders set before it, while Never and an Ask set after
    /// it ([`Self::asks_despite_grant`]) remain excluded.
    pub fn arms_by_default(&self, project_key: &str, session_path: &str, source_key: &str) -> bool {
        let Some(grant) = self.automatic_grant.as_ref() else {
            return false;
        };
        grant.recorded_sources.contains(source_key)
            && project_key != UNKNOWN_PROJECT_KEY
            && if grant.include_existing {
                // This fresh global choice includes Ask folders from before
                // it; Never is still an explicit exclusion and must always
                // win, and so is an Ask the contributor chose after it.
                self.folder_mode(project_key) == ProjectMode::NotifyOnly
                    && !self.asks_despite_grant(project_key)
            } else {
                !self.projects.contains_key(project_key)
                    && !grant.projects_on_disk.contains(project_key)
                    && !self.sessions_on_disk_at_grant.contains(session_path)
            }
    }

    /// Whether `project_key` is a folder the contributor set to Ask
    /// (`NotifyOnly`) after the include-existing grant in force was given:
    /// an exception to that grant, as a folder set to Never is (owner
    /// decision 2026-10-29, after Ask went missing under the grant in
    /// staging). The grant does not arm it ([`Self::arms_by_default`]), so it
    /// resolves to Ask and its sessions wait for a person.
    ///
    /// "After" is by identity, never by clock: the folder's
    /// [`ProjectEntry::set_under_grant`], which [`Self::set_mode`] writes on
    /// every mode change, is the generation of the grant in force
    /// ([`AutomaticGrant::generation`]). A clock stepped backwards, or a
    /// tie, cannot reorder the two. An Ask from before the grant was set
    /// under no grant or an earlier generation, so it is superseded -- the
    /// fresh global choice replaces old folder modes at the moment it is
    /// made, as [`Self::include_existing_under_grant`] says -- and a later
    /// fresh choice supersedes this one's exceptions the same way.
    /// A folder the grant armed and a sweep then voided back to Ask
    /// ([`Self::sweep_grants`]) counts too: it was armed under this grant,
    /// and the void keeps that record, so it keeps asking rather than being
    /// re-armed on its next session.
    pub fn asks_despite_grant(&self, project_key: &str) -> bool {
        let Some(grant) = self
            .automatic_grant
            .as_ref()
            .filter(|grant| grant.include_existing)
        else {
            return false;
        };
        self.projects.get(project_key).is_some_and(|entry| {
            entry.mode == ProjectMode::NotifyOnly && entry.set_under_grant == Some(grant.generation)
        })
    }

    /// Whether the session at `session_path` must not be approved unattended
    /// in `project_key`: the grant armed the project, and the session was on
    /// disk at a grant. It waits for the contributor instead, unless a fresh
    /// include-existing grant explicitly covers that history.
    pub fn holds_back_unattended(&self, project_key: &str, session_path: &str) -> bool {
        self.armed_by_grant.contains(project_key)
            && self.sessions_on_disk_at_grant.contains(session_path)
            && !self
                .automatic_grant
                .as_ref()
                .is_some_and(|grant| grant.include_existing)
    }

    /// Arm `project_key` on the grant's behalf.
    pub fn arm_by_grant(
        &mut self,
        project_key: &str,
        now: DateTime<Utc>,
        terms: super::grant_terms::GrantTerms,
    ) -> Result<()> {
        self.set_mode(project_key, ProjectMode::AutoUpload, now)?;
        self.record_grant_terms(project_key, terms);
        self.armed_by_grant.insert(project_key.to_string());
        Ok(())
    }

    /// Count one successful upload against its project.
    ///
    /// The unresolvable bucket is never counted. It can never be armed --
    /// `set_mode` and `resolve` both refuse it -- so a count for it could
    /// only ever feed an offer that cannot be delivered.
    pub fn record_contribution(&mut self, project_key: &str) {
        if project_key == UNKNOWN_PROJECT_KEY {
            return;
        }
        *self.contributed.entry(project_key.to_string()).or_insert(0) += 1;
    }

    /// Record what one session sent from `project_key` on the contributor's
    /// behalf had for redaction (K6).
    ///
    /// Only while the project is armed: a session approved unattended in a
    /// project that has since left automatic says nothing about the next
    /// arming, which starts from nothing.
    pub fn record_automatic_redaction(
        &mut self,
        project_key: &str,
        redaction: super::automatic_gate::SessionRedaction,
    ) {
        use super::automatic_gate::SessionRedaction;
        let Some(entry) = self.projects.get_mut(project_key) else {
            return;
        };
        if entry.mode != ProjectMode::AutoUpload {
            return;
        }
        let tally = &mut entry.automatic_redaction;
        match redaction {
            SessionRedaction::CertifiedFullPipeline => {
                tally.certified_full_pipeline = tally.certified_full_pipeline.saturating_add(1);
            }
            SessionRedaction::NotCertified => {
                tally.not_certified = tally.not_certified.saturating_add(1);
            }
        }
    }

    /// Record a "Not now" against one project.
    pub fn decline_arming(&mut self, project_key: &str, now: DateTime<Utc>) {
        self.arming_declined_at.insert(project_key.to_string(), now);
    }

    /// The one project worth offering to arm right now, if any.
    ///
    /// At most one, deliberately. A queue that sprouts an offer per project
    /// is the ongoing administration the contributor asked to be rid of; the
    /// strongest single candidate is the whole of what this ever asks.
    ///
    /// A project qualifies when it has contributed at least
    /// [`ARMING_SUGGESTION_THRESHOLD`] times, is still ask-first (an armed
    /// project has nothing to offer and an ignored one has been answered
    /// already) and not about to be armed by the include-existing grant
    /// ([`Self::grant_will_arm`]), can be armed at all, and has not been declined inside
    /// [`ARMING_DECLINE_COOLDOWN_DAYS`].
    pub fn arming_suggestion(&self, now: DateTime<Utc>) -> Option<ArmingSuggestion> {
        let cooldown = Duration::days(ARMING_DECLINE_COOLDOWN_DAYS);
        self.contributed
            .iter()
            .filter(|(key, count)| {
                **count >= ARMING_SUGGESTION_THRESHOLD
                    && key.as_str() != UNKNOWN_PROJECT_KEY
                    // The folder's own mode, not an override's: the offer
                    // is to set that, and an override must neither hide an
                    // Ask folder nor offer to arm an armed one.
                    && self.folder_mode(key) == ProjectMode::NotifyOnly
                    // Nor one the include-existing grant will arm: it
                    // already reads Automatic (`shown_folder_mode`), so the
                    // offer would contradict the row it is about.
                    && !self.grant_will_arm(key)
                    && match self.arming_declined_at.get(*key) {
                        // A clock that went backwards lands here as "still
                        // inside the cooldown", which can only ever suppress
                        // an offer, never add one.
                        Some(declined) => now.signed_duration_since(*declined) >= cooldown,
                        None => true,
                    }
            })
            // Most contributions first; the key breaks a tie so the answer is
            // stable across runs rather than depending on map iteration.
            .max_by(|a, b| a.1.cmp(b.1).then_with(|| b.0.cmp(a.0)))
            .map(|(key, count)| ArmingSuggestion {
                project_id: project_id_for(key),
                project_label: project_label_for(key),
                contributed_count: *count,
            })
    }

    /// Re-key every map through `normalize_project_key`, merging entries
    /// that collapse onto one key.
    ///
    /// Idempotent: a key that is already normalized normalizes to itself,
    /// so running this on a v2 file changes nothing. The unknown bucket is
    /// not a path and is carried across untouched.
    ///
    /// A key that no longer normalizes at all -- a relative path from a
    /// hand-edited file, say -- is dropped rather than kept under its old
    /// spelling, because nothing will ever look it up again.
    pub fn rekey(&mut self) {
        self.rekey_with(MovedArming::Carries);
    }

    /// [`Self::rekey`], choosing what happens to an armed entry whose key
    /// moves. See [`MovedArming`].
    pub fn rekey_with(&mut self, moved_arming: MovedArming) {
        let renamed = normalize_policy_key;
        // The grant a merged Ask may be an exception to. Read before the
        // loop: only `projects` is taken out of `self`.
        let in_force = self.grant_generation_in_force();

        let mut projects: BTreeMap<String, ProjectEntry> = BTreeMap::new();
        for (key, mut entry) in std::mem::take(&mut self.projects) {
            let Some(fresh) = renamed(&key) else { continue };
            if moved_arming == MovedArming::AsksAgain
                && fresh != key
                && entry.mode == ProjectMode::AutoUpload
            {
                entry.mode = ProjectMode::NotifyOnly;
                entry.armed_under = None;
                entry.automatic_redaction = AutomaticRedactionTally::default();
            }
            // Re-derived rather than carried across from `entry`: a v2 file
            // has no display path at all, and one written on another host
            // may name a directory this one spells differently.
            let shown = display_path_for_key(&fresh);
            let label = project_label_for(shown.as_deref().unwrap_or(&fresh));
            projects
                .entry(fresh)
                .and_modify(|existing| {
                    existing.mode = existing.mode.more_restrictive(entry.mode);
                    // The earlier of the two: the contributor's decision
                    // about this project is as old as its oldest half.
                    existing.added_at = existing.added_at.min(entry.added_at);
                    // An exception to the grant in force survives the merge
                    // when either half was one (fail closed): the merged
                    // folder keeps asking rather than being armed by it.
                    if in_force.is_some()
                        && (existing.set_under_grant == in_force
                            || entry.set_under_grant == in_force)
                    {
                        existing.set_under_grant = in_force;
                    }
                    existing.label = label.clone();
                    existing.display_path = shown.clone();
                    // Terms only mean anything while the merged project is
                    // still armed. When it is, the surviving entry's terms
                    // stand; when the merge made it ask-first, they go.
                    if existing.mode != ProjectMode::AutoUpload {
                        existing.armed_under = None;
                    }
                    // Both halves' sessions are now this project's, so both
                    // records count: one uncertified session in either keeps
                    // the merged project off the model-scrub wording. A merge
                    // that left it ask-first starts the next arming clean.
                    existing.automatic_redaction = if existing.mode == ProjectMode::AutoUpload {
                        AutomaticRedactionTally {
                            certified_full_pipeline: existing
                                .automatic_redaction
                                .certified_full_pipeline
                                .saturating_add(entry.automatic_redaction.certified_full_pipeline),
                            not_certified: existing
                                .automatic_redaction
                                .not_certified
                                .saturating_add(entry.automatic_redaction.not_certified),
                        }
                    } else {
                        AutomaticRedactionTally::default()
                    };
                })
                .or_insert(ProjectEntry {
                    mode: entry.mode,
                    added_at: entry.added_at,
                    label,
                    display_path: shown,
                    armed_under: entry.armed_under,
                    automatic_redaction: entry.automatic_redaction,
                    set_under_grant: entry.set_under_grant,
                });
        }
        self.projects = projects;

        let mut contributed: BTreeMap<String, u32> = BTreeMap::new();
        for (key, count) in std::mem::take(&mut self.contributed) {
            let Some(fresh) = renamed(&key) else { continue };
            *contributed.entry(fresh).or_insert(0) += count;
        }
        self.contributed = contributed;

        let mut declined: BTreeMap<String, DateTime<Utc>> = BTreeMap::new();
        for (key, at) in std::mem::take(&mut self.arming_declined_at) {
            let Some(fresh) = renamed(&key) else { continue };
            // The most recent decline wins: a merged project was declined
            // as recently as its most recent half, which is what the
            // cooldown should measure from.
            declined
                .entry(fresh)
                .and_modify(|existing| *existing = (*existing).max(at))
                .or_insert(at);
        }
        self.arming_declined_at = declined;

        if moved_arming == MovedArming::AsksAgain {
            // What only an armed folder carries, left under a spelling no
            // folder is armed under any more: the old key of a moved arming,
            // or a key a merge made stricter. Nothing would look it up again,
            // and a re-arm starts each of these fresh.
            let armed: BTreeSet<String> = self
                .projects
                .iter()
                .filter(|(_, e)| e.mode == ProjectMode::AutoUpload)
                .map(|(k, _)| k.clone())
                .collect();
            self.armed_from_now.retain(|k, _| armed.contains(k));
            self.arming_claims.retain(|k, _| armed.contains(k));
            self.armed_by_grant.retain(|k| armed.contains(k));
        }

        self.schema_version = DAEMON_PROJECTS_SCHEMA.to_string();
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_PROJECTS_FILE)? else {
            return Ok(Self::new());
        };
        let mut policy: Self =
            serde_json::from_slice(&body).context("parsing daemon project policy")?;
        // Owner decision 2026-10-16: no switch-on notice is raised, so one
        // an older build persisted is never reported. The next save writes
        // the empty list.
        policy.arming_rewordings.clear();
        // Before the rekey below, which merges `added_at`: the derivation
        // reads each folder's own.
        policy.derive_grant_exceptions();
        // Migrate in memory on every load rather than rewriting the file
        // here: `load` has no business writing, and the next `save` -- which
        // every mutation already performs -- persists the v2 form. A file
        // that is never mutated again is migrated identically on every read,
        // which costs one normalization pass and is always correct.
        if policy.schema_version == DAEMON_PROJECTS_SCHEMA_V2 {
            policy.rekey_with(MovedArming::AsksAgain);
        } else if policy.schema_version != DAEMON_PROJECTS_SCHEMA {
            policy.rekey();
        }
        Ok(policy)
    }

    /// Give a policy file written before grant generations existed (the
    /// #1747 build) the identities this build reads, once, from the rule
    /// that build applied: a folder whose mode was set after the grant's
    /// `granted_at` was set under that grant. So every Ask that build kept
    /// as an exception to an include-existing grant stays one, and every
    /// Ask it let the grant supersede stays superseded -- an upgrade never
    /// arms a folder that was asking. A folder the grant armed is recorded
    /// too, so a void back to Ask keeps asking as it did.
    ///
    /// Runs only while `grant_exceptions_derived` is false, which is only a
    /// file an older build wrote: on a file this build wrote, a folder with
    /// no recorded grant was set under none, and the clock must not decide
    /// otherwise. In memory, like the rekey; the next save persists it.
    pub fn derive_grant_exceptions(&mut self) {
        if self.grant_exceptions_derived {
            return;
        }
        self.grant_exceptions_derived = true;
        if self
            .automatic_grant
            .as_ref()
            .is_none_or(|g| g.generation != 0)
        {
            return;
        }
        let generation = self.next_grant_generation();
        let Some(grant) = self.automatic_grant.as_mut() else {
            return;
        };
        grant.generation = generation;
        let granted_at = grant.granted_at;
        for entry in self.projects.values_mut() {
            if entry.set_under_grant.is_none() && entry.added_at > granted_at {
                entry.set_under_grant = Some(generation);
            }
        }
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon project policy")?;
        store.write_daemon_file(DAEMON_PROJECTS_FILE, &body)
    }

    /// The mode in force for a project key.
    ///
    /// Autonomy for the unknown-cwd bucket is denied here, after the stored
    /// map is consulted rather than before it, so a hand-edited or tampered
    /// policy file cannot grant autonomy to sessions the daemon cannot
    /// attribute. `Ignore` is still honoured for that bucket: refusing to
    /// upload it unattended is not a reason to refuse to silence it.
    ///
    /// A contribution override ([`ContributionOverride`]) is read over the
    /// folder's own mode ([`Self::folder_mode`]), with the stricter rule
    /// winning where one applies: a folder set to `Ignore` stays `Ignore`
    /// under every override, and the unknown bucket is never `AutoUpload`.
    /// An `AutoUpload` override's hold on what was already on disk is not
    /// here but in [`Self::waits_for_a_person`], beside the other holds.
    pub fn resolve(&self, project_key: &str) -> ProjectMode {
        self.resolve_over(self.folder_mode(project_key), project_key)
    }

    /// The override read over `own`, the folder's mode, as [`Self::resolve`]
    /// describes.
    fn resolve_over(&self, own: ProjectMode, project_key: &str) -> ProjectMode {
        let Some(o) = self.contribution_override.as_ref() else {
            return own;
        };
        match o.mode {
            ProjectMode::AutoUpload => match own {
                ProjectMode::Ignore => ProjectMode::Ignore,
                _ if project_key == UNKNOWN_PROJECT_KEY => ProjectMode::NotifyOnly,
                _ => ProjectMode::AutoUpload,
            },
            mode => own.more_restrictive(mode),
        }
    }

    /// Record the terms an armed project was granted under.
    ///
    /// Called where the arming happens, with the terms in force at that
    /// moment. A project that is not `AutoUpload` has no grant to record.
    pub fn record_grant_terms(
        &mut self,
        project_key: &str,
        terms: super::grant_terms::GrantTerms,
    ) -> bool {
        match self.projects.get_mut(project_key) {
            Some(entry) if entry.mode == ProjectMode::AutoUpload => {
                entry.armed_under = Some(terms);
                true
            }
            _ => false,
        }
    }

    /// The contributor chose these data uses themselves: every standing
    /// grant (each armed folder, the Flow 1 grant and an "Automatic"
    /// override) now covers them, so the next sweep does not void it as
    /// `scopes-widened` (product owner, 2026-10-29). Before this, allowing
    /// Train coding models, the use credit needs, turned Automatic off
    /// everywhere.
    ///
    /// Only the scopes term moves, and only by the contributor's own
    /// choice: a new destination, identity, filter or witness still voids.
    /// The scopes are added to what each grant held, never replaced, so a
    /// sweep that reads the config before or after the caller writes it
    /// finds the grant covering it either way. Returns whether any grant
    /// changed.
    pub fn adopt_chosen_scopes(&mut self, scopes: &BTreeSet<String>) -> bool {
        fn widen(terms: &mut super::grant_terms::GrantTerms, scopes: &BTreeSet<String>) -> bool {
            let before = terms.consent_scopes.len();
            terms.consent_scopes.extend(scopes.iter().cloned());
            terms.consent_scopes.len() != before
        }
        let mut changed = false;
        for entry in self.projects.values_mut() {
            if entry.mode == ProjectMode::AutoUpload
                && let Some(terms) = entry.armed_under.as_mut()
            {
                changed |= widen(terms, scopes);
            }
        }
        if let Some(grant) = self.automatic_grant.as_mut() {
            changed |= widen(&mut grant.granted_under, scopes);
        }
        if let Some(o) = self.contribution_override.as_mut()
            && o.mode == ProjectMode::AutoUpload
            && let Some(terms) = o.granted_under.as_mut()
        {
            changed |= widen(terms, scopes);
        }
        changed
    }

    /// Void every armed project whose grant no longer covers the terms in
    /// force, and record terms for any armed project that has none.
    ///
    /// A voided project returns to `NotifyOnly`: it asks again rather than
    /// carrying on under terms nobody agreed to, and arming it again records
    /// the new terms. A project with no recorded terms is baselined rather
    /// than voided, because there is nothing to compare against -- see
    /// [`ProjectEntry::armed_under`].
    pub fn sweep_grants(
        &mut self,
        current: &super::grant_terms::GrantTerms,
        now: DateTime<Utc>,
    ) -> GrantSweep {
        let mut sweep = GrantSweep::default();
        let mut voided_keys: Vec<(String, Vec<&'static str>)> = Vec::new();
        for (key, entry) in self.projects.iter_mut() {
            if entry.mode != ProjectMode::AutoUpload {
                continue;
            }
            match &entry.armed_under {
                None => {
                    entry.armed_under = Some(current.clone());
                    sweep.baselined += 1;
                }
                Some(granted) => {
                    let reasons = current.widening_from(granted);
                    if !reasons.is_empty() {
                        entry.mode = ProjectMode::NotifyOnly;
                        entry.armed_under = None;
                        entry.automatic_redaction = AutomaticRedactionTally::default();
                        voided_keys.push((key.clone(), reasons.clone()));
                        sweep.voided.push(VoidedGrant {
                            project_label: entry.label.clone(),
                            reasons,
                        });
                    }
                }
            }
        }
        if let Some(grant) = &self.automatic_grant {
            let reasons = current.widening_from(&grant.granted_under);
            if !reasons.is_empty() {
                self.automatic_grant = None;
                sweep.automatic_grant_voided = Some(reasons);
            }
        }
        // An "Automatic" override is a grant too (#1208): a widening
        // clears it, so no folder resolves to `AutoUpload` because of it,
        // and the pill falls back to the folders' own modes. One with no
        // recorded terms is voided rather than baselined -- see
        // `OVERRIDE_TERMS_UNRECORDED`.
        if let Some(o) = &self.contribution_override
            && o.mode == ProjectMode::AutoUpload
        {
            let reasons = match &o.granted_under {
                Some(granted) => current.widening_from(granted),
                None => vec![OVERRIDE_TERMS_UNRECORDED],
            };
            if !reasons.is_empty() {
                self.contribution_override = None;
                self.arming_rewordings
                    .retain(|n| n.project_key != OVERRIDE_ARMING_KEY);
                self.prune_arming_record();
                sweep.contribution_override_voided = Some(reasons);
            }
        }
        // Recorded with the void, in the same save, so a void is never
        // written without the notice that tells the contributor about it.
        for (key, reasons) in voided_keys {
            // No longer armed, so neither what it was armed under nor a
            // notice about what that now means applies; the void says what
            // happened instead.
            self.arming_claims.remove(&key);
            self.arming_rewordings.retain(|n| n.project_key != key);
            self.push_grant_void(Some(key), &reasons, now);
        }
        if let Some(reasons) = sweep.automatic_grant_voided.clone() {
            self.push_grant_void(None, &reasons, now);
        }
        if let Some(reasons) = sweep.contribution_override_voided.clone() {
            self.push_void(None, true, &reasons, now);
        }
        sweep
    }

    /// K5 for the "Automatic" contribution override: move its recorded claim
    /// when the words in force (`in_force`, what the arming offer would say
    /// now) no longer claim what the override was set under. It stays on and
    /// no notice is raised. Returns whether the claim moved.
    pub fn sweep_override_claim(
        &mut self,
        in_force: super::arming_wording::ArmingClaim,
        _now: DateTime<Utc>,
    ) -> bool {
        let Some(o) = self.contribution_override.as_mut() else {
            return false;
        };
        if o.mode != ProjectMode::AutoUpload {
            return false;
        }
        // Its confirmation says sessions "will be scrubbed", as the arming
        // offer does, so an override set before claims were recorded claimed
        // that.
        let was = o
            .claim
            .unwrap_or(super::arming_wording::ArmingClaim::ModelScrubbed);
        if !was.narrowed_to(in_force) {
            return false;
        }
        // Owner decision 2026-10-16: no switch-on notice is raised; the list
        // stays on the wire and is always empty. Only the claim moves.
        o.claim = Some(in_force);
        true
    }

    /// Record what the words `project_key` was just armed under claim. Only
    /// for an armed project; returns whether it was recorded.
    pub fn record_arming_claim(
        &mut self,
        project_key: &str,
        claim: super::arming_wording::ArmingClaim,
    ) -> bool {
        match self.projects.get(project_key) {
            Some(entry) if entry.mode == ProjectMode::AutoUpload => {
                self.arming_claims.insert(project_key.to_string(), claim);
                true
            }
            _ => false,
        }
    }

    /// What the words `project_key` was armed under claimed: as recorded, or
    /// for a project armed before claims were recorded, what its arming
    /// screen said then.
    pub fn arming_claim(&self, project_key: &str) -> super::arming_wording::ArmingClaim {
        self.arming_claims
            .get(project_key)
            .copied()
            .unwrap_or_else(|| {
                super::arming_wording::legacy_claim(self.armed_by_grant.contains(project_key))
            })
    }

    /// K5: move the recorded claim of every armed project whose words in
    /// force (`current`, per project key) no longer claim what it was armed
    /// under. No notice is raised.
    ///
    /// The project stays armed. Only a narrowing is recorded
    /// (`ArmingClaim::narrowed_to`); a widening is not, so what the
    /// contributor was told stays the baseline. Returns the labels of the
    /// projects reworded, for the audit.
    pub fn sweep_arming_claims(
        &mut self,
        current: impl Fn(&str) -> super::arming_wording::ArmingClaim,
        _now: DateTime<Utc>,
    ) -> Vec<String> {
        let mut reworded: Vec<(
            String,
            String,
            super::arming_wording::ArmingClaim,
            super::arming_wording::ArmingClaim,
        )> = Vec::new();
        for (key, entry) in &self.projects {
            if entry.mode != ProjectMode::AutoUpload || key == UNKNOWN_PROJECT_KEY {
                continue;
            }
            let was = self.arming_claim(key);
            let in_force = current(key);
            if was.narrowed_to(in_force) {
                reworded.push((key.clone(), entry.label.clone(), was, in_force));
            }
        }
        let mut labels = Vec::with_capacity(reworded.len());
        for (key, label, _was, in_force) in reworded {
            // Owner decision 2026-10-16: no switch-on notice is raised; the
            // list stays on the wire and is always empty. Only the claim
            // moves.
            self.arming_claims.insert(key, in_force);
            labels.push(label);
        }
        labels
    }

    /// Drop the rewording notices a shell reports it has shown. Returns how
    /// many went; an id that is not outstanding is ignored.
    pub fn acknowledge_arming_rewordings(&mut self, ids: &[u64]) -> usize {
        let before = self.arming_rewordings.len();
        self.arming_rewordings.retain(|n| !ids.contains(&n.id));
        before - self.arming_rewordings.len()
    }

    /// Record that the Scrub-check upgrade has been seen. No notice is
    /// raised. The marker is saved by startup, before work.
    pub fn record_scrub_check_upgrade(&mut self, _automatic: bool, _now: DateTime<Utc>) -> bool {
        if self.scrub_check_upgrade_recorded {
            return false;
        }
        // Owner decision 2026-10-16: no switch-on notice is raised; the list
        // stays on the wire and is always empty. Only the marker is kept.
        self.scrub_check_upgrade_recorded = true;
        true
    }

    /// Record a notice, replacing any still outstanding for the same grant:
    /// the newer one says everything the contributor now needs to know.
    fn push_grant_void(
        &mut self,
        project_key: Option<String>,
        reasons: &[&'static str],
        now: DateTime<Utc>,
    ) {
        self.push_void(project_key, false, reasons, now);
    }

    /// [`Self::push_grant_void`], for a project, the Flow 1 grant, or (with
    /// `contribution_override`) the override. Replaces only a notice about
    /// the same grant.
    fn push_void(
        &mut self,
        project_key: Option<String>,
        contribution_override: bool,
        reasons: &[&'static str],
        now: DateTime<Utc>,
    ) {
        self.grant_voids.retain(|n| {
            n.project_key != project_key || n.contribution_override != contribution_override
        });
        let id = self.next_grant_void_id;
        self.next_grant_void_id = id.saturating_add(1);
        self.grant_voids.push(GrantVoidNotice {
            id,
            voided_at: now,
            project_key,
            reasons: reasons.iter().map(|r| (*r).to_string()).collect(),
            contribution_override,
        });
    }

    /// Drop the notices a shell reports it has shown. Returns how many went;
    /// an id that is not outstanding is ignored rather than refused, since a
    /// second shell may have acknowledged it first.
    pub fn acknowledge_grant_voids(&mut self, ids: &[u64]) -> usize {
        let before = self.grant_voids.len();
        self.grant_voids.retain(|n| !ids.contains(&n.id));
        before - self.grant_voids.len()
    }

    /// Whether `mode` may be set on `project_key` at all, before anything is
    /// recorded: the unknown bucket can never be armed. `set_mode` applies
    /// it too; a caller that records an arming first asks here first, so a
    /// refusal leaves no record of an arming that never happened.
    pub fn check_mode(project_key: &str, mode: ProjectMode) -> Result<()> {
        if project_key == UNKNOWN_PROJECT_KEY && mode == ProjectMode::AutoUpload {
            bail!(
                "unknown-project sessions cannot be set to auto_upload: \
                 their working directory could not be resolved, so no \
                 per-project opt-in can apply to them"
            );
        }
        Ok(())
    }

    /// Record a mode for `project_key`.
    ///
    /// The label is **derived here**, from the key, and is never a caller
    /// argument. It used to be one, and `set_project_mode` passed straight
    /// through whatever a socket client sent -- so any client could write
    /// an arbitrary string (a full filesystem path, a token, a fragment of
    /// somebody's transcript) into `list_projects` output and into
    /// `daemon-audit.jsonl`, the two sinks this crate's label-only rule
    /// exists to protect. Deriving it removes the injection path by
    /// construction rather than by validation.
    ///
    /// The stored label is the bare basename (`project_label_for`);
    /// disambiguation against colliding basenames happens at render time,
    /// so a stored label never goes stale when a colliding project appears
    /// later.
    pub fn set_mode(
        &mut self,
        project_key: &str,
        mode: ProjectMode,
        now: DateTime<Utc>,
    ) -> Result<()> {
        Self::check_mode(project_key, mode)?;
        // A mode set here is the contributor's own decision about the project,
        // not the grant's; `arm_by_grant` re-adds it after calling this.
        self.armed_by_grant.remove(project_key);
        // Likewise an earlier arm-from-now: any mode set here replaces it,
        // and `set_project_mode` re-adds it after this when the new setting
        // is itself from now (asked for, or a re-send keeping an earlier
        // hold). An arming with the backlog therefore releases it.
        self.armed_from_now.remove(project_key);
        // And it answers any notice that this project's grant had stopped.
        self.grant_voids
            .retain(|n| n.project_key.as_deref() != Some(project_key));
        // A mode set here is also a fresh decision about the words: what
        // was claimed before is no longer what it was armed under, and a
        // notice that it was reworded is answered. An arming records its
        // claim afterwards (`record_arming_claim`).
        self.arming_claims.remove(project_key);
        self.arming_rewordings
            .retain(|n| n.project_key != project_key);
        let shown = display_path_for_key(project_key);
        let set_under_grant = self.grant_generation_in_force();
        // Under a grant, never earlier than it, even when the clock has
        // stepped back since: a #1747 daemon, on a downgrade, reads an Ask
        // as an exception to the grant by `added_at > granted_at` alone, and
        // must agree with `set_under_grant` here or it arms a folder that
        // was asking. A later `derive_grant_exceptions` agrees with it too.
        let added_at = match &self.automatic_grant {
            Some(grant) => now.max(grant.granted_at + Duration::microseconds(1)),
            None => now,
        };
        self.projects.insert(
            project_key.to_string(),
            ProjectEntry {
                mode,
                added_at,
                label: project_label_for(shown.as_deref().unwrap_or(project_key)),
                display_path: shown,
                // A fresh entry on every mode change: leaving automatic
                // clears the terms, and re-arming records new ones.
                armed_under: None,
                // And the redaction record: it covers one arming only.
                automatic_redaction: AutomaticRedactionTally::default(),
                // The grant this decision was made under, if any: what makes
                // an Ask an exception to it (`asks_despite_grant`).
                set_under_grant,
            },
        );
        Ok(())
    }
}

/// The fixed label `set_project_mode` refuses an unrecognized key with.
pub const ERR_PROJECT_KEY_UNRECOGNIZED: &str = "project-key-unrecognized";

/// Whether the daemon will accept `project_key` from a socket client.
///
/// A socket client used to be able to name any key at all. The key is not
/// itself echoed anywhere, but its *basename* becomes the project label --
/// which crosses the socket in `list_projects` and lands in
/// `daemon-audit.jsonl` -- so `"/x/ghp_realtokenvalue"` put a token into
/// both sinks with one call.
///
/// A key is admissible when it is one of:
///
/// * the locked unknown-cwd sentinel (which `set_mode` still refuses to
///   arm, and whose label is the sentinel name, not a path segment);
/// * a key the daemon already knows -- one it has discovered on a queued
///   session, or one already in the policy file, so its label is one the
///   daemon itself derived;
/// * an absolute path that exists on this machine as a directory and
///   canonicalizes to itself. This is the only admissible *new* key, and it
///   is exactly what both producers of keys emit: the watcher takes the
///   cwd an agent recorded, and `daemon project <path>` canonicalizes an
///   existing directory. Keeping it means a project can still be set to
///   `ignore` (or armed) before its first session is ever seen, which is
///   the whole point of that CLI flow.
///
/// Everything else is refused with `ERR_PROJECT_KEY_UNRECOGNIZED`.
///
/// What this does and does not buy, stated precisely, because an earlier
/// version of this comment claimed more than the code delivers. It does not
/// make an arbitrary label impossible: same-user code can `mkdir
/// /tmp/<any-string>` and then name that directory, and the basename
/// becomes a label in `list_projects` and `daemon-audit.jsonl`. What it
/// buys is that the string must first exist as a real directory on this
/// machine, which bounds it to what a filesystem will accept -- no `/`, no
/// NUL, at most 255 bytes -- and leaves it visible on disk. That is a
/// narrowing, not a seal, and the same surface is already reachable by
/// writing a session file with an arbitrary `cwd`. `MAX_PROJECT_LABEL_CHARS`
/// bounds the length independently at the render sinks, since a filesystem
/// limit is not a promise this crate makes.
///
/// "Canonicalizes to itself" is now read as: the key's normalized form
/// ([`normalize_policy_key`]) names an existing directory. The key itself
/// need not already be in that form -- a mixed-case spelling, a path through
/// a symlink, a Windows 8.3 short name or `\\?\` prefix all name a real
/// directory -- and `set_project_mode` stores the normalized form, which is
/// the spelling sessions are looked up by (#992). A path naming no
/// directory is refused, however it is spelled.
pub fn project_key_is_admissible(project_key: &str, known_keys: &[String]) -> bool {
    if project_key == UNKNOWN_PROJECT_KEY {
        return true;
    }
    if known_keys.iter().any(|k| k == project_key) {
        return true;
    }
    let path = std::path::Path::new(project_key);
    if !path.is_absolute() {
        return false;
    }
    // Both: the key as given must be a real directory, or a missing path
    // inside a repository would pass on the strength of its root, which is
    // what normalization walks it to.
    path.is_dir()
        && normalize_policy_key(project_key)
            .is_some_and(|normalized| std::path::Path::new(&normalized).is_dir())
}

/// The policy key for a caller-supplied project key: the unknown bucket's
/// sentinel as itself, anything else through
/// `project_key::normalize_project_key` -- the one normalization the watcher
/// keys sessions by and [`ProjectPolicy::rekey`] keys stored entries by.
///
/// That includes folding case where the filesystem is case-insensitive
/// (macOS, Windows; never Linux) and walking up to the repository root, so
/// `daemon project <repo>/sub` names `<repo>`, which is the key every
/// session in `sub` already resolves to.
///
/// `None` for anything that cannot be a key (relative, empty).
pub fn normalize_policy_key(project_key: &str) -> Option<String> {
    if project_key == UNKNOWN_PROJECT_KEY {
        return Some(UNKNOWN_PROJECT_KEY.to_string());
    }
    crate::daemon::project_key::normalize_project_key(project_key)
}

/// The prefix every opaque project id carries.
///
/// It exists so the two identifier spaces `set_project_mode` accepts can
/// never be confused for one another: an id always starts with this and a
/// project key never does (a key is either an absolute path or the
/// `unknown-project` sentinel).
pub const PROJECT_ID_PREFIX: &str = "proj_";

/// Hex characters of SHA-256 carried in an opaque project id. 16 nibbles is
/// 64 bits, which is far more than enough to keep the handful of projects on
/// one contributor's machine distinct, and short enough to be pasted into a
/// bug report.
const PROJECT_ID_HEX_CHARS: usize = 16;

/// The fixed label `set_project_mode` refuses an unrecognized id with.
pub const ERR_PROJECT_ID_UNRECOGNIZED: &str = "project-id-unrecognized";

/// The opaque, daemon-issued identifier for a project.
///
/// This exists because a socket client could not previously name a project
/// at all. The privacy rule is that a project key -- a local filesystem path
/// -- never crosses the socket, so queue entries and `list_projects` rows
/// carry only `project_label`. But a label is not an admissible key, and
/// `project_key_is_admissible` (rightly) refuses anything that is not a real
/// path the daemon can corroborate. A GUI therefore held nothing it could
/// pass to `set_project_mode`, which made arming and ignoring a project
/// unreachable from every application this contract exists to serve.
///
/// The id is a hash of the key, not an encoding of it: it is one-way, so it
/// leaks no path component, and it is deterministic, so it is the same
/// across a daemon restart and across a policy file rebuilt from scratch --
/// nothing is stored to make it stable, because nothing needs to be.
///
/// It is *not* a capability. Knowing an id confers nothing that naming the
/// directory did not already confer; it is an identifier a client can hold,
/// and the daemon still resolves it only against projects it already knows.
pub fn project_id_for(project_key: &str) -> String {
    #[cfg(test)]
    project_id_derivations::record();
    let digest = Sha256::digest(project_key.as_bytes());
    format!(
        "{PROJECT_ID_PREFIX}{}",
        hex_prefix(&digest, PROJECT_ID_HEX_CHARS)
    )
}

/// How many ids `project_id_for` has derived on this thread, so a test can
/// hold a handler to a count of sha256 derivations rather than a timing.
#[cfg(test)]
pub(crate) mod project_id_derivations {
    use std::cell::Cell;

    thread_local! {
        static COUNT: Cell<usize> = const { Cell::new(0) };
    }

    pub(crate) fn record() {
        COUNT.with(|c| c.set(c.get() + 1));
    }

    pub(crate) fn reset() {
        COUNT.with(|c| c.set(0));
    }

    pub(crate) fn count() -> usize {
        COUNT.with(Cell::get)
    }
}

/// Resolve an opaque project id back to the key it was minted from, or
/// `None` if no project the daemon knows about has that id.
///
/// Resolution is by re-deriving ids over the known-key set rather than by
/// any stored mapping, which is what keeps ids stable with nothing to
/// migrate. The unknown-cwd sentinel is always resolvable -- it is a
/// permanent bucket rather than a discovered project, and a client that sees
/// it in a queue entry must be able to silence it -- while `set_mode` still
/// refuses to arm it.
///
/// An id can only ever name a project the daemon already knows. That is the
/// deliberate asymmetry with `project_key`: a path can name a project that
/// has never been seen (the CLI's `daemon project <path> --mode ignore`
/// before that project's first session), and an id cannot, because the
/// daemon cannot mint an id for something it has never discovered.
pub fn project_key_for_id(project_id: &str, known_keys: &[String]) -> Option<String> {
    if !project_id.starts_with(PROJECT_ID_PREFIX) {
        return None;
    }
    std::iter::once(UNKNOWN_PROJECT_KEY.to_string())
        .chain(known_keys.iter().cloned())
        .find(|key| project_id_for(key) == project_id)
}

/// Whether a working directory yields a usable display label -- i.e. has a
/// final path segment at all.
///
/// `Path::file_name` returns `None` for `/`, for anything ending in `..`,
/// and for the empty string. Every one of those is a real cwd a coding
/// agent can record.
fn has_usable_basename(cwd: &str) -> bool {
    std::path::Path::new(cwd)
        .file_name()
        .is_some_and(|n| !n.is_empty())
}

/// The policy key for a session: its normalized working directory, or the
/// locked unknown bucket. Never falls back to a basename heuristic.
///
/// Normalization (`project_key::normalize_project_key`) is what makes one
/// directory one project regardless of how the recording spelled it. A cwd
/// with no usable final path segment -- `/`, anything ending in `..`, the
/// empty string, a relative path -- yields no key and goes to the unknown
/// bucket rather than becoming a key of its own. Such a key has no label
/// but itself, and `project_label` crosses the socket, lands in
/// `daemon-audit.jsonl`, in OS notification text, and in `HistoryRecord` --
/// so the fallback turned a full local path into every one of those, in
/// direct violation of the invariant `audit`'s own
/// `an_audit_entry_never_carries_a_path` test asserts.
pub fn project_key_for(cwd: Option<&str>) -> String {
    project_for(cwd).0
}

/// [`project_key_for`] and, beside it, the unfolded path a person is shown.
///
/// Both from one normalization, because they are two spellings of one
/// directory and re-deriving the second would let them drift. `None` for
/// the unknown bucket, which is not a directory and has nothing to show.
pub fn project_for(cwd: Option<&str>) -> (String, Option<String>) {
    match cwd.and_then(crate::daemon::project_key::normalize_project) {
        Some(p) => (p.key, Some(p.display_path)),
        None => (UNKNOWN_PROJECT_KEY.to_string(), None),
    }
}

/// Recover the unfolded spelling of a directory from its folded key.
///
/// The key is itself a real path, so re-normalizing it recovers the case
/// the filesystem holds -- `std::fs::canonicalize` reports the on-disk
/// spelling on both macOS and Windows. `None` for the unknown bucket and
/// for a key that no longer resolves to anything; callers fall back to the
/// folded key, which names the right directory in the wrong case rather
/// than naming nothing.
pub fn display_path_for_key(project_key: &str) -> Option<String> {
    if project_key == UNKNOWN_PROJECT_KEY {
        return None;
    }
    crate::daemon::project_key::normalize_project(project_key).map(|p| p.display_path)
}

/// A display label for a project key: the final path segment, or the bucket
/// name. Consumers render this instead of the key, which is a local path.
///
/// A key with no final path segment reports the bucket name rather than
/// echoing the key. `project_key_for` already prevents such a key from
/// being created, so this is the second line of defence -- it covers a
/// hand-edited or older `daemon-projects.json`, whose keys reach here
/// having never gone through `project_key_for` at all. Under no
/// circumstances does a raw path leave this function.
pub fn project_label_for(project_key: &str) -> String {
    if project_key == UNKNOWN_PROJECT_KEY || !has_usable_basename(project_key) {
        return UNKNOWN_PROJECT_KEY.to_string();
    }
    let label = std::path::Path::new(project_key)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| UNKNOWN_PROJECT_KEY.to_string());
    truncate_label(&label)
}

/// The ceiling on a rendered project label, in characters.
///
/// The label crosses the socket, lands in `daemon-audit.jsonl`, and goes
/// into OS notification text. Nothing bounded its length: a directory name
/// can be up to 255 bytes on every filesystem this runs on, and
/// `project_key_is_admissible` accepts any real directory, so a caller
/// willing to `mkdir` could put 255 bytes of chosen text into all three
/// sinks. That is not a leak of anything the caller did not already know,
/// but a label is a display name and it should be bounded here rather than
/// by whatever the filesystem happened to allow.
pub const MAX_PROJECT_LABEL_CHARS: usize = 64;

/// Truncate on a character boundary, never mid-codepoint.
fn truncate_label(label: &str) -> String {
    if label.chars().count() <= MAX_PROJECT_LABEL_CHARS {
        return label.to_string();
    }
    label.chars().take(MAX_PROJECT_LABEL_CHARS).collect()
}

/// The known-key set every disambiguation call site must agree on: every
/// configured project plus every project already sitting in the queue.
/// `queue_project_keys` takes an iterator rather than a `Queue` so this
/// module does not need to depend on `queue`; call sites pass
/// `queue.all().iter().map(|e| e.project_key.clone())`.
pub fn known_keys(
    policy: &ProjectPolicy,
    queue_project_keys: impl Iterator<Item = String>,
) -> Vec<String> {
    policy
        .projects
        .keys()
        .cloned()
        // A project the daemon has uploaded from is one it knows, whether or
        // not the contributor has ever set a mode for it and whether or not
        // it still has anything queued. Without this, a project could be
        // offered for arming -- the offer is built from exactly this counter
        // -- and then the answer refused as an unrecognized id, because
        // nothing else on the machine still remembered the key.
        .chain(policy.contributed.keys().cloned())
        .chain(queue_project_keys)
        .collect()
}

/// A display label unique within `known_keys`. Adds a short stable hash
/// suffix only when the basename collides.
///
/// The suffix is derived from `sha256(project_key)`, never from any path
/// segment: labels cross the IPC socket and must never leak which directory
/// a colliding project lives in.
pub fn disambiguated_label(
    project_key: &str,
    project_path: Option<&str>,
    known_keys: &[String],
) -> String {
    // Two labels, deliberately. The rendered one comes from the unfolded
    // path so `IronWire` stays `IronWire`; the collision test runs on the
    // folded key, so two projects whose basenames differ only in case are
    // still treated as colliding and still both get a suffix. Testing on
    // the rendered label instead would let `Api` and `api` sit side by side
    // looking like two spellings of one project.
    label_with_suffix_if(project_key, project_path, |folded| {
        known_keys
            .iter()
            .any(|other| other != project_key && project_label_for(other) == folded)
    })
}

/// [`disambiguated_label`] for many keys against one known-key set, with
/// the folded basenames counted once instead of once per label.
///
/// `disambiguated_label` scans the whole set for each call, so labelling
/// every row of `list_projects` was quadratic in the number of folders.
/// This answers the same label: a key collides when some *other* distinct
/// key folds to the same basename. Keys are deduplicated, because the known
/// set repeats a key once per queue entry, and a key repeated is not a
/// collision with itself.
pub struct LabelCollisions {
    /// Folded basename to the first distinct key seen with it, and whether
    /// a second distinct key shares it.
    by_label: std::collections::HashMap<String, (String, bool)>,
}

impl LabelCollisions {
    pub fn new(known_keys: &[String]) -> Self {
        let mut by_label: std::collections::HashMap<String, (String, bool)> =
            std::collections::HashMap::new();
        for key in known_keys {
            match by_label.entry(project_label_for(key)) {
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert((key.clone(), false));
                }
                std::collections::hash_map::Entry::Occupied(mut slot) => {
                    let (first, shared) = slot.get_mut();
                    if first != key {
                        *shared = true;
                    }
                }
            }
        }
        Self { by_label }
    }

    /// The label [`disambiguated_label`] gives `project_key` against the
    /// known-key set this was built from.
    pub fn label(&self, project_key: &str, project_path: Option<&str>) -> String {
        label_with_suffix_if(project_key, project_path, |folded| {
            self.by_label
                .get(folded)
                .is_some_and(|(first, shared)| *shared || first != project_key)
        })
    }
}

/// The rendering both [`disambiguated_label`] and [`LabelCollisions`] share:
/// the label from the unfolded path, with a `sha256(key)` suffix when
/// `collides` says the folded basename is shared.
fn label_with_suffix_if(
    project_key: &str,
    project_path: Option<&str>,
    collides: impl FnOnce(&str) -> bool,
) -> String {
    let label = project_label_for(project_path.unwrap_or(project_key));
    if project_key == UNKNOWN_PROJECT_KEY {
        return label;
    }

    let folded = project_label_for(project_key);
    if !collides(&folded) {
        return label;
    }

    let digest = Sha256::digest(project_key.as_bytes());
    let suffix = hex_prefix(&digest, 4);
    format!("{label} ({suffix})")
}

fn hex_prefix(bytes: &[u8], chars: usize) -> String {
    bytes
        .iter()
        .flat_map(|b| [b >> 4, b & 0x0f])
        .take(chars)
        .map(|nibble| char::from_digit(nibble as u32, 16).expect("nibble is < 16"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use crate::daemon::test_paths::{abs, abs_key};

    fn now() -> DateTime<Utc> {
        "2026-08-08T12:00:00Z".parse().unwrap()
    }

    /// `LabelCollisions` gives every key the label `disambiguated_label`
    /// gives it: colliding basenames get the hash suffix, a key repeated
    /// in the known set does not collide with itself, the unknown bucket
    /// never gets a suffix, and a key outside the set collides with any
    /// key that shares its basename.
    #[test]
    fn label_collisions_label_as_disambiguated_label_does() {
        let known: Vec<String> = [
            "/tmp/lc-a/app",
            "/tmp/lc-b/app",
            "/tmp/lc-a/solo",
            "/tmp/lc-a/solo",
            "/tmp/lc-a/solo",
            "/tmp/lc-c/twice",
            "/tmp/lc-c/twice",
            "/tmp/lc-d/twice",
            UNKNOWN_PROJECT_KEY,
            UNKNOWN_PROJECT_KEY,
            "/",
        ]
        .iter()
        .map(|k| k.to_string())
        .collect();
        let labels = LabelCollisions::new(&known);
        let outside = ["/tmp/lc-e/solo", "/tmp/lc-e/fresh", "/tmp/lc-e/app"];
        for key in known.iter().map(String::as_str).chain(outside) {
            for shown in [None, Some("/tmp/Shown/App")] {
                assert_eq!(
                    labels.label(key, shown),
                    disambiguated_label(key, shown, &known),
                    "{key} {shown:?}"
                );
            }
        }
        assert_eq!(labels.label("/tmp/lc-a/solo", None), "solo");
        assert!(labels.label("/tmp/lc-c/twice", None).starts_with("twice ("));
        assert!(labels.label("/tmp/lc-e/solo", None).starts_with("solo ("));
    }

    #[test]
    fn an_unknown_project_defaults_to_notify_only() {
        let p = ProjectPolicy::new();
        assert_eq!(
            p.resolve(&abs("Users/z/code/never-seen")),
            ProjectMode::NotifyOnly
        );
    }

    #[test]
    fn sessions_without_a_cwd_land_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("   ")), UNKNOWN_PROJECT_KEY);
        // Spelled for the host platform: `/Users/z/code/proj` has a root
        // but no prefix on Windows, so it is not absolute there and would
        // fall into the unknown bucket for a reason that has nothing to do
        // with what this test is about. `abs_key` applies the case-folding
        // rule directly rather than by calling normalization again.
        assert_eq!(
            project_key_for(Some(&abs("Users/z/code/proj"))),
            abs_key("Users/z/code/proj")
        );
    }

    #[test]
    fn the_unknown_bucket_cannot_be_set_to_auto_upload() {
        let mut p = ProjectPolicy::new();
        let err = p
            .set_mode(UNKNOWN_PROJECT_KEY, ProjectMode::AutoUpload, now())
            .unwrap_err();
        assert!(err.to_string().contains("unknown-project"));
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_stays_notify_only_even_if_the_file_says_auto() {
        // Defence against a hand-edited or tampered policy file.
        let mut p = ProjectPolicy::new();
        p.projects.insert(
            UNKNOWN_PROJECT_KEY.to_string(),
            ProjectEntry {
                mode: ProjectMode::AutoUpload,
                added_at: now(),
                label: "unknown".into(),
                display_path: None,
                armed_under: None,
                automatic_redaction: Default::default(),
                set_under_grant: None,
            },
        );
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn the_unknown_bucket_may_still_be_ignored() {
        // Refusing autonomy is not the same as refusing to be silenced.
        let mut p = ProjectPolicy::new();
        p.set_mode(UNKNOWN_PROJECT_KEY, ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::Ignore);
    }

    #[test]
    fn set_and_resolve_round_trip_for_a_real_project() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", ProjectMode::AutoUpload, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::AutoUpload);
        p.set_mode("/Users/z/code/proj", ProjectMode::Ignore, now())
            .unwrap();
        assert_eq!(p.resolve("/Users/z/code/proj"), ProjectMode::Ignore);
    }

    #[test]
    fn labels_are_basenames_so_consumers_never_need_the_path() {
        assert_eq!(project_label_for("/Users/z/code/my-proj"), "my-proj");
        assert_eq!(project_label_for(UNKNOWN_PROJECT_KEY), UNKNOWN_PROJECT_KEY);
    }

    #[test]
    fn a_cwd_with_no_usable_basename_goes_to_the_unknown_bucket() {
        // `Path::file_name` is None for every one of these, and each is a
        // real cwd an agent can record. Before this, the key became the raw
        // path and so did the label -- which then crossed the socket,
        // landed in daemon-audit.jsonl, in OS notification text, and in
        // HistoryRecord.
        for cwd in ["/", "/Users/z/code/..", "..", ""] {
            assert_eq!(
                project_key_for(Some(cwd)),
                UNKNOWN_PROJECT_KEY,
                "cwd {cwd:?} must not become a policy key of its own"
            );
        }
    }

    #[test]
    fn a_degenerate_key_never_renders_as_a_raw_path() {
        // Second line of defence: a hand-edited or older
        // daemon-projects.json can hold such a key without it ever having
        // gone through `project_key_for`.
        for key in ["/", "/Users/z/secret-client/..", ".."] {
            let label = project_label_for(key);
            assert_eq!(label, UNKNOWN_PROJECT_KEY, "key {key:?} leaked as {label}");
            assert!(!label.contains('/'), "key {key:?} leaked as {label}");
        }
    }

    #[test]
    fn a_label_is_length_bounded_at_the_sink() {
        // `project_key_is_admissible` accepts any real directory, and a
        // directory name can be 255 bytes. The label crosses the socket,
        // lands in daemon-audit.jsonl, and goes into notification text, so
        // it is bounded here rather than by whatever the filesystem allowed.
        let long = "n".repeat(255);
        let label = project_label_for(&format!("/Users/z/code/{long}"));
        assert_eq!(label.chars().count(), MAX_PROJECT_LABEL_CHARS);
    }

    #[test]
    fn a_multibyte_label_is_truncated_on_a_character_boundary() {
        let long = "\u{e9}".repeat(200);
        let label = project_label_for(&format!("/Users/z/code/{long}"));
        assert_eq!(label.chars().count(), MAX_PROJECT_LABEL_CHARS);
        assert!(label.chars().all(|c| c == '\u{e9}'));
    }

    #[test]
    fn a_short_label_is_left_exactly_as_it_is() {
        assert_eq!(project_label_for("/Users/z/code/my-proj"), "my-proj");
    }

    #[test]
    fn a_degenerate_key_is_never_suffixed_into_a_path_either() {
        // `disambiguated_label` only suffixes a hash, but it starts from
        // `project_label_for`, so a leak there would leak through here too.
        let keys = vec!["/".to_string(), "/Users/z/client/..".to_string()];
        for key in &keys {
            let label = disambiguated_label(key, None, &keys);
            assert!(!label.contains('/'), "{key} leaked as {label}");
        }
    }

    #[test]
    fn a_degenerate_key_cannot_be_armed() {
        // It resolves to the locked bucket, which `resolve` refuses to
        // report as AutoUpload however the file was written.
        let mut p = ProjectPolicy::new();
        assert!(
            p.set_mode("/", ProjectMode::AutoUpload, now()).is_ok(),
            "the key itself is not the sentinel, so set_mode does not refuse it"
        );
        // But no session can ever resolve to it: every cwd with no usable
        // basename is bucketed before policy is consulted.
        assert_eq!(project_key_for(Some("/")), UNKNOWN_PROJECT_KEY);
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::NotifyOnly);
    }

    #[test]
    fn policy_round_trips_through_the_store() {
        let (_d, store) = temp_store();
        let mut p = ProjectPolicy::new();
        p.set_mode("/Users/z/code/proj", ProjectMode::AutoUpload, now())
            .unwrap();
        p.save(&store).unwrap();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), p);
    }

    #[test]
    fn policy_defaults_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), ProjectPolicy::new());
    }

    #[test]
    fn a_unique_basename_is_left_alone() {
        let keys = vec![
            "/Users/z/code/alpha".to_string(),
            "/Users/z/code/beta".to_string(),
        ];
        assert_eq!(
            disambiguated_label("/Users/z/code/alpha", None, &keys),
            "alpha"
        );
    }

    #[test]
    fn colliding_basenames_get_distinct_stable_suffixes() {
        // The dangerous case: one of these is the client's repo.
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        let a = disambiguated_label("/Users/z/work/api", None, &keys);
        let b = disambiguated_label("/Users/z/client/api", None, &keys);
        assert_ne!(a, b, "colliding projects must be distinguishable");
        assert!(a.starts_with("api ("), "got {a}");
        assert_eq!(
            a,
            disambiguated_label("/Users/z/work/api", None, &keys),
            "must be stable"
        );
    }

    #[test]
    fn a_suffix_never_contains_a_path_segment() {
        // The suffix is a hash, not a directory name: paths never cross the wire.
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        let a = disambiguated_label("/Users/z/work/api", None, &keys);
        assert!(!a.contains("work") && !a.contains('/'), "got {a}");
    }

    #[test]
    fn a_project_id_leaks_no_path_component() {
        // The whole reason the label-only rule exists: this key names a
        // client the contributor may not disclose. The id crosses the same
        // socket the key is forbidden from crossing.
        let key = "/Users/z/clients/acme-secret-merger/api";
        let id = project_id_for(key);
        for fragment in ["acme", "secret", "merger", "clients", "api", "Users", "/"] {
            assert!(!id.contains(fragment), "{id} leaked {fragment}");
        }
        assert!(id.starts_with(PROJECT_ID_PREFIX), "got {id}");
        assert_eq!(id.len(), PROJECT_ID_PREFIX.len() + PROJECT_ID_HEX_CHARS);
    }

    #[test]
    fn a_project_id_is_deterministic_and_distinct_per_project() {
        // Deterministic: nothing is stored to make it stable, so it is the
        // same after a restart and after a policy file rebuilt from scratch.
        assert_eq!(
            project_id_for("/Users/z/code/proj"),
            project_id_for("/Users/z/code/proj")
        );
        assert_ne!(
            project_id_for("/Users/z/work/api"),
            project_id_for("/Users/z/client/api"),
            "colliding basenames must still get distinct ids"
        );
    }

    #[test]
    fn a_project_id_resolves_back_to_the_key_that_minted_it() {
        let keys = vec![
            "/Users/z/work/api".to_string(),
            "/Users/z/client/api".to_string(),
        ];
        for key in &keys {
            assert_eq!(
                project_key_for_id(&project_id_for(key), &keys).as_deref(),
                Some(key.as_str())
            );
        }
    }

    #[test]
    fn an_id_for_a_project_the_daemon_does_not_know_resolves_to_nothing() {
        let keys = vec!["/Users/z/work/api".to_string()];
        assert_eq!(
            project_key_for_id(&project_id_for("/Users/z/never/seen"), &keys),
            None
        );
        // A label is not an id, and neither is a path or a bare string.
        for bogus in ["api", "/Users/z/work/api", "proj_deadbeefdeadbeef", ""] {
            assert_eq!(project_key_for_id(bogus, &keys), None, "accepted {bogus}");
        }
    }

    #[test]
    fn the_unknown_bucket_has_a_resolvable_id_even_when_nothing_is_known() {
        // It is a permanent bucket rather than a discovered project, and a
        // client seeing it on a queue entry must be able to silence it.
        assert_eq!(
            project_key_for_id(&project_id_for(UNKNOWN_PROJECT_KEY), &[]).as_deref(),
            Some(UNKNOWN_PROJECT_KEY)
        );
    }

    #[test]
    fn an_id_survives_a_policy_file_rebuilt_from_scratch() {
        let (_d, store) = temp_store();
        let key = "/Users/z/code/proj";
        let mut p = ProjectPolicy::new();
        p.set_mode(key, ProjectMode::Ignore, now()).unwrap();
        p.save(&store).unwrap();
        let before = project_id_for(key);

        let reloaded = ProjectPolicy::load(&store).unwrap();
        let reloaded_key = reloaded.projects.keys().next().unwrap();
        assert_eq!(project_id_for(reloaded_key), before);
    }

    #[test]
    fn the_unknown_bucket_is_never_suffixed() {
        let keys = vec![UNKNOWN_PROJECT_KEY.to_string()];
        assert_eq!(
            disambiguated_label(UNKNOWN_PROJECT_KEY, None, &keys),
            UNKNOWN_PROJECT_KEY
        );
    }

    fn grant_terms_with(ingest_url: &str) -> super::super::grant_terms::GrantTerms {
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest_url.to_string();
        super::super::grant_terms::GrantTerms::current(&cfg, None, false, "none")
    }

    const SRC: &str = "claude-code /root";

    fn set_of(items: &[&str]) -> BTreeSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn granted_with_disk(projects: &[&str], sessions: &[&str]) -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        let at = t("2026-09-25T00:00:00Z");
        p.grant_automatic(at, grant_terms_with("https://ingest.invalid"));
        assert!(p.record_source(at, SRC, set_of(sessions), set_of(projects)));
        p
    }

    /// The grant arms only a real project, new since the grant, with no
    /// entry of its own, from a source it has recorded.
    #[test]
    fn the_grant_arms_only_a_project_new_since_it() {
        let mut p = ProjectPolicy::new();
        p.grant_automatic(
            t("2026-09-25T00:00:00Z"),
            grant_terms_with("https://ingest.invalid"),
        );
        assert!(
            !p.arms_by_default("/w/new", "/s/new.jsonl", SRC),
            "not before a record"
        );

        let mut p = granted_with_disk(&["/w/old"], &["/s/live.jsonl"]);
        assert!(p.arms_by_default("/w/new", "/s/new.jsonl", SRC));
        assert!(
            !p.arms_by_default("/w/old", "/s/other.jsonl", SRC),
            "on disk at the grant"
        );
        assert!(
            !p.arms_by_default("/w/moved", "/s/live.jsonl", SRC),
            "a session on disk at the grant, whatever project it reads as now"
        );
        assert!(!p.arms_by_default(UNKNOWN_PROJECT_KEY, "/s/u.jsonl", SRC));
        p.set_mode("/w/new", ProjectMode::Ignore, t("2026-09-25T01:00:00Z"))
            .unwrap();
        assert!(
            !p.arms_by_default("/w/new", "/s/new2.jsonl", SRC),
            "its own entry wins"
        );
    }

    /// Per source: a source the grant has not recorded -- connected after
    /// it, re-rooted, or whose discovery failed -- arms nothing, and
    /// recording it later puts its history on disk rather than counting it
    /// as new.
    #[test]
    fn a_source_arms_nothing_until_it_is_recorded() {
        let mut p = granted_with_disk(&[], &[]);
        let at = p.grant_id().unwrap();
        let late = "codex /other-root";
        assert!(!p.arms_by_default("/w/codex-proj", "/c/old.jsonl", late));
        assert!(p.needs_source_record(at, late));
        assert!(p.record_source(
            at,
            late,
            set_of(&["/c/old.jsonl"]),
            set_of(&["/w/codex-proj"])
        ));
        assert!(!p.needs_source_record(at, late));
        assert!(
            !p.arms_by_default("/w/codex-proj", "/c/new.jsonl", late),
            "its project was on disk"
        );
        assert!(
            !p.record_source(at, late, BTreeSet::new(), BTreeSet::new()),
            "once per source"
        );
    }

    /// A pass records only for the grant it read before listing anything, so
    /// a grant given during discovery is not recorded from that listing.
    #[test]
    fn a_record_is_only_for_the_grant_read_before_the_listing() {
        let mut p = granted_with_disk(&[], &[]);
        let earlier = p.grant_id().unwrap();
        p.grant_automatic(
            t("2026-09-25T02:00:00Z"),
            grant_terms_with("https://ingest.invalid"),
        );
        assert!(!p.needs_source_record(earlier, SRC));
        assert!(!p.record_source(earlier, SRC, BTreeSet::new(), BTreeSet::new()));
        assert!(
            !p.arms_by_default("/w/new", "/s/new.jsonl", SRC),
            "the new grant is unrecorded"
        );
    }

    /// A session on disk at the grant is never approved unattended in a
    /// project the grant armed, even after the grant is withdrawn; a project
    /// the contributor armed themselves is theirs to send.
    #[test]
    fn a_pre_grant_session_is_held_back_in_a_project_the_grant_armed() {
        let mut p = granted_with_disk(&["/w/old"], &["/s/pre.jsonl"]);
        let now = t("2026-09-25T03:00:00Z");
        p.arm_by_grant("/w/new", now, grant_terms_with("https://ingest.invalid"))
            .unwrap();
        assert!(p.holds_back_unattended("/w/new", "/s/pre.jsonl"));
        assert!(!p.holds_back_unattended("/w/new", "/s/fresh.jsonl"));
        assert!(p.withdraw_automatic_grant());
        assert!(
            p.holds_back_unattended("/w/new", "/s/pre.jsonl"),
            "outlives the grant"
        );

        p.set_mode("/w/new", ProjectMode::AutoUpload, now).unwrap();
        assert!(
            !p.holds_back_unattended("/w/new", "/s/pre.jsonl"),
            "armed by the contributor"
        );
    }

    /// K5: a project armed from now holds back every session its sources had
    /// on disk at the arming -- and everything from a source not yet recorded
    /// for it -- while a session first seen after the record is its to send.
    /// Setting the mode again by any route ends the hold.
    #[test]
    fn an_arming_from_now_holds_back_only_what_was_on_disk_at_it() {
        let mut p = ProjectPolicy::new();
        let now = t("2026-09-28T00:00:00Z");
        p.set_mode("/w/a", ProjectMode::AutoUpload, now).unwrap();
        p.arm_from_now("/w/a", now);
        assert!(p.is_armed_from_now("/w/a"));
        assert!(
            p.holds_back_from_arming("/w/a", "/s/new.jsonl", SRC),
            "nothing is sent before a record"
        );
        assert!(p.holds_back_from_arming_at_send("/w/a", "/s/new.jsonl"));

        let armings = p.armings_from_now();
        assert!(p.needs_arming_record(&armings, SRC));
        assert!(p.record_source_for_armings(&armings, SRC, set_of(&["/s/pre.jsonl"])));
        assert!(!p.needs_arming_record(&armings, SRC), "once per source");
        assert!(p.holds_back_from_arming("/w/a", "/s/pre.jsonl", SRC));
        assert!(p.holds_back_from_arming_at_send("/w/a", "/s/pre.jsonl"));
        assert!(!p.holds_back_from_arming("/w/a", "/s/new.jsonl", SRC));
        assert!(!p.holds_back_from_arming_at_send("/w/a", "/s/new.jsonl"));
        assert!(
            p.holds_back_from_arming("/w/a", "/s/other.jsonl", "codex /late"),
            "a source not recorded yet"
        );
        assert!(
            !p.holds_back_from_arming("/w/other", "/s/pre.jsonl", SRC),
            "only in the project armed from now"
        );
        assert!(p.waits_for_a_person("/w/a", "/s/pre.jsonl", SRC));

        // A later arming of another project records a session that is new
        // to the first: it is held there, not here.
        let later = t("2026-09-28T01:00:00Z");
        p.set_mode("/w/b", ProjectMode::AutoUpload, later).unwrap();
        p.arm_from_now("/w/b", later);
        let armings = p.armings_from_now();
        assert!(p.record_source_for_armings(
            &armings,
            SRC,
            set_of(&["/s/pre.jsonl", "/s/new.jsonl"])
        ));
        assert!(p.holds_back_from_arming("/w/b", "/s/new.jsonl", SRC));
        assert!(!p.holds_back_from_arming("/w/a", "/s/new.jsonl", SRC));
        assert!(p.holds_back_from_arming("/w/a", "/s/pre.jsonl", SRC));

        // A plain arming by the contributor replaces it: the backlog is
        // theirs to send, as `auto_upload` has always meant.
        p.set_mode("/w/a", ProjectMode::AutoUpload, later).unwrap();
        assert!(!p.is_armed_from_now("/w/a"));
        assert!(!p.holds_back_from_arming("/w/a", "/s/pre.jsonl", SRC));
    }

    /// A pass records only for the armings it read before listing the disk,
    /// so an arming made while discovery walked it stays unrecorded -- and
    /// so holds everything -- until a later pass.
    #[test]
    fn an_arming_record_is_only_for_the_arming_read_before_the_listing() {
        let mut p = ProjectPolicy::new();
        let first = t("2026-09-28T00:00:00Z");
        p.set_mode("/w/a", ProjectMode::AutoUpload, first).unwrap();
        p.arm_from_now("/w/a", first);
        let stale = p.armings_from_now();
        let again = t("2026-09-28T00:05:00Z");
        p.set_mode("/w/a", ProjectMode::AutoUpload, again).unwrap();
        p.arm_from_now("/w/a", again);
        assert!(!p.record_source_for_armings(&stale, SRC, set_of(&["/s/x.jsonl"])));
        assert!(p.holds_back_from_arming("/w/a", "/s/y.jsonl", SRC));
    }

    /// The record is pruned to what an arming in force can still hold, and
    /// emptied with the last arming from now.
    #[test]
    fn the_arming_record_is_pruned_to_what_can_still_hold() {
        let mut p = ProjectPolicy::new();
        let a = t("2026-09-28T00:00:00Z");
        p.set_mode("/w/a", ProjectMode::AutoUpload, a).unwrap();
        p.arm_from_now("/w/a", a);
        p.record_source_for_armings(&p.armings_from_now(), SRC, set_of(&["/s/one.jsonl"]));
        let b = t("2026-09-28T01:00:00Z");
        p.set_mode("/w/b", ProjectMode::AutoUpload, b).unwrap();
        p.arm_from_now("/w/b", b);
        p.record_source_for_armings(&p.armings_from_now(), SRC, set_of(&["/s/two.jsonl"]));
        assert!(!p.prune_arming_record(), "both armings can hold");

        p.set_mode("/w/b", ProjectMode::NotifyOnly, b).unwrap();
        assert!(p.prune_arming_record());
        assert!(p.holds_back_from_arming("/w/a", "/s/one.jsonl", SRC));
        assert!(!p.sessions_on_disk_at_arming.contains_key("/s/two.jsonl"));

        p.set_mode("/w/a", ProjectMode::NotifyOnly, b).unwrap();
        assert!(p.prune_arming_record());
        assert!(p.sessions_on_disk_at_arming.is_empty());
    }

    /// The fields are additive: a policy file written before them loads, with
    /// nothing armed from now.
    #[test]
    fn a_policy_file_without_armings_from_now_loads_with_none() {
        let mut value = serde_json::to_value(ProjectPolicy::new()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("armed_from_now");
        object.remove("sessions_on_disk_at_arming");
        object.remove("next_arming_record");
        let p: ProjectPolicy = serde_json::from_value(value).unwrap();
        assert!(p.armed_from_now.is_empty());
        assert!(!p.holds_back_from_arming("/w/a", "/s/pre.jsonl", SRC));
    }

    /// R6 reaches the grant itself: widened terms void it, so it cannot go
    /// on arming new projects under terms nobody agreed to. Unchanged terms
    /// keep it.
    #[test]
    fn widening_the_terms_voids_the_automatic_grant() {
        let mut p = granted_with_disk(&[], &[]);
        let same = p.sweep_grants(
            &grant_terms_with("https://ingest.invalid"),
            t("2026-09-25T04:00:00Z"),
        );
        assert!(same.automatic_grant_voided.is_none());
        assert!(p.automatic_grant.is_some());

        let moved = p.sweep_grants(
            &grant_terms_with("https://elsewhere.invalid"),
            t("2026-09-25T04:00:00Z"),
        );
        assert_eq!(
            moved.automatic_grant_voided,
            Some(vec![super::super::grant_terms::VOID_DESTINATION])
        );
        assert!(moved.changed());
        assert!(p.automatic_grant.is_none());
        assert!(!p.arms_by_default("/w/new", "/s/new.jsonl", SRC));
    }

    /// K6: a void (R6) returns the project to ask-first and clears what its
    /// sessions had for redaction, so a later arming starts from nothing and
    /// cannot inherit the model-scrub wording.
    #[test]
    fn a_voided_grant_clears_the_projects_redaction_record() {
        use super::super::automatic_gate::SessionRedaction;
        let mut p = armed_and_granted("https://ingest.invalid");
        p.record_automatic_redaction("/w/api", SessionRedaction::CertifiedFullPipeline);
        assert_eq!(
            p.projects["/w/api"]
                .automatic_redaction
                .certified_full_pipeline,
            1
        );
        let sweep = p.sweep_grants(
            &grant_terms_with("https://elsewhere.invalid"),
            t("2026-09-25T04:00:00Z"),
        );
        assert_eq!(sweep.voided.len(), 1);
        assert_eq!(
            p.projects["/w/api"].automatic_redaction,
            AutomaticRedactionTally::default()
        );
    }

    /// K6: normalization that merges two armed entries keeps both records,
    /// so an uncertified session in either half is not forgotten.
    #[test]
    fn a_merge_keeps_both_halves_redaction_records() {
        use super::super::automatic_gate::SessionRedaction;
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        for key in [&root, &sub] {
            p.projects.insert(
                key.to_string_lossy().to_string(),
                ProjectEntry {
                    mode: ProjectMode::AutoUpload,
                    added_at: now(),
                    label: "repo".into(),
                    display_path: None,
                    armed_under: None,
                    automatic_redaction: AutomaticRedactionTally::default(),
                    set_under_grant: None,
                },
            );
        }
        p.record_automatic_redaction(&root.to_string_lossy(), SessionRedaction::NotCertified);
        p.record_automatic_redaction(
            &sub.to_string_lossy(),
            SessionRedaction::CertifiedFullPipeline,
        );
        p.rekey();
        assert_eq!(p.projects.len(), 1, "the two entries merge");
        let merged = p.projects.values().next().unwrap().automatic_redaction;
        assert_eq!(
            merged,
            AutomaticRedactionTally {
                certified_full_pipeline: 1,
                not_certified: 1,
            },
            "the uncertified session survives the merge"
        );
    }

    /// A policy with `/w/api` armed under `ingest`'s terms and the Flow 1
    /// grant given under the same terms.
    fn armed_and_granted(ingest: &str) -> ProjectPolicy {
        let mut p = granted_with_disk(&[], &[]);
        let now = t("2026-09-25T03:00:00Z");
        p.set_mode("/w/api", ProjectMode::AutoUpload, now).unwrap();
        assert!(p.record_grant_terms("/w/api", grant_terms_with(ingest)));
        p
    }

    /// The contributor's own choice of data uses carries every grant with
    /// it: allowing Train coding models no longer turns Automatic off
    /// (product owner, 2026-10-29). Without the adoption the same sweep
    /// voids both grants as `scopes-widened`.
    #[test]
    fn chosen_scopes_keep_automatic_on() {
        let ingest = "https://ingest.invalid";
        let mut widened = grant_terms_with(ingest);
        widened.consent_scopes.insert("model_training".to_string());
        let at = t("2026-09-25T04:00:00Z");

        let mut p = armed_and_granted(ingest);
        let sweep = p.sweep_grants(&widened, at);
        assert_eq!(
            sweep.voided.len(),
            1,
            "without adoption, the folder is voided"
        );
        assert!(sweep.automatic_grant_voided.is_some());

        let mut p = armed_and_granted(ingest);
        assert!(p.adopt_chosen_scopes(&widened.consent_scopes));
        assert!(
            !p.adopt_chosen_scopes(&widened.consent_scopes),
            "already covered, nothing changes"
        );
        let sweep = p.sweep_grants(&widened, at);
        assert!(sweep.voided.is_empty(), "{:?}", sweep.voided);
        assert!(sweep.automatic_grant_voided.is_none());
        assert_eq!(p.resolve("/w/api"), ProjectMode::AutoUpload);
        assert!(p.automatic_grant.is_some());
        assert!(p.grant_voids.is_empty());

        // Only the scopes move: a new destination still voids.
        let mut moved = grant_terms_with("https://elsewhere.invalid");
        moved.consent_scopes = widened.consent_scopes.clone();
        let sweep = p.sweep_grants(&moved, at);
        assert_eq!(sweep.voided.len(), 1);
        assert_eq!(sweep.voided[0].reasons, vec!["destination-changed"]);
    }

    /// Adopted scopes are added, never replaced: a sweep that still reads
    /// the wider config written before a narrowing finds the grant covering
    /// it, so the race between the two writes cannot void.
    #[test]
    fn adopting_narrower_scopes_keeps_what_the_grant_held() {
        let ingest = "https://ingest.invalid";
        let mut wide = grant_terms_with(ingest);
        wide.consent_scopes.insert("model_training".to_string());
        let mut p = armed_and_granted(ingest);
        p.adopt_chosen_scopes(&wide.consent_scopes);

        assert!(!p.adopt_chosen_scopes(&BTreeSet::new()));
        let sweep = p.sweep_grants(&wide, t("2026-09-25T04:00:00Z"));
        assert!(sweep.voided.is_empty());
        assert!(sweep.automatic_grant_voided.is_none());
    }

    /// A policy whose Flow 1 grant includes existing history, given at
    /// `2026-10-29T12:00:00Z` with its one source recorded, and a folder set
    /// to Ask an hour before it.
    fn granted_including_existing() -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        p.set_mode("/w/pre", ProjectMode::NotifyOnly, t("2026-10-29T11:00:00Z"))
            .unwrap();
        let at = t("2026-10-29T12:00:00Z");
        p.grant_automatic(at, grant_terms_with("https://ingest.invalid"));
        p.include_existing_under_grant();
        assert!(p.record_source(at, SRC, BTreeSet::new(), BTreeSet::new()));
        p
    }

    /// Owner decision 2026-10-29: Ask is always offered on a folder, and a
    /// folder set to Ask after the include-existing grant is an exception
    /// to it, as a folder set to Never is. A folder's Ask from before the
    /// grant is still superseded by it: the fresh global choice replaces
    /// old folder modes at the moment it is made.
    #[test]
    fn a_folder_set_to_ask_after_the_grant_is_an_exception_to_it() {
        let mut p = granted_including_existing();
        p.set_mode(
            "/w/post",
            ProjectMode::NotifyOnly,
            t("2026-10-29T13:00:00Z"),
        )
        .unwrap();
        p.set_mode("/w/never", ProjectMode::Ignore, t("2026-10-29T13:00:00Z"))
            .unwrap();

        assert!(p.arms_by_default("/w/none", "/s/a.jsonl", SRC), "no entry");
        assert!(
            p.arms_by_default("/w/pre", "/s/b.jsonl", SRC),
            "an Ask from before the grant is superseded by it"
        );
        assert!(!p.asks_despite_grant("/w/pre"));
        assert!(
            !p.arms_by_default("/w/post", "/s/c.jsonl", SRC),
            "an Ask chosen after the grant is the folder's exception"
        );
        assert!(p.asks_despite_grant("/w/post"));
        assert_eq!(p.resolve("/w/post"), ProjectMode::NotifyOnly);
        assert!(!p.arms_by_default("/w/never", "/s/d.jsonl", SRC));
        assert!(!p.arms_by_default(UNKNOWN_PROJECT_KEY, "/s/e.jsonl", SRC));

        // Back to Automatic: the exception is gone, and Ask again restores it.
        p.set_mode(
            "/w/post",
            ProjectMode::AutoUpload,
            t("2026-10-29T14:00:00Z"),
        )
        .unwrap();
        assert!(!p.asks_despite_grant("/w/post"));
        assert_eq!(p.resolve("/w/post"), ProjectMode::AutoUpload);
        p.set_mode(
            "/w/post",
            ProjectMode::NotifyOnly,
            t("2026-10-29T15:00:00Z"),
        )
        .unwrap();
        assert!(!p.arms_by_default("/w/post", "/s/c.jsonl", SRC));

        // A fresh include-existing grant is a fresh global choice: it
        // supersedes the folder's Ask as it did the pre-grant one.
        let again = t("2026-10-29T16:00:00Z");
        p.grant_automatic(again, grant_terms_with("https://ingest.invalid"));
        p.include_existing_under_grant();
        assert!(p.record_source(again, SRC, BTreeSet::new(), BTreeSet::new()));
        assert!(!p.asks_despite_grant("/w/post"));
        assert!(p.arms_by_default("/w/post", "/s/c.jsonl", SRC));
    }

    /// Finding 1 on #1747: an Ask set after the grant is an exception to it
    /// however the clock moved in between. A clock stepped back (an NTP
    /// correction, a dual-boot RTC, a restored VM snapshot) stamps the Ask
    /// earlier than the grant, and an exact tie stamps it at the same
    /// instant; neither may let the grant arm the folder.
    #[test]
    fn a_folder_set_to_ask_after_the_grant_is_an_exception_whatever_the_clock_says() {
        let mut p = granted_including_existing();
        p.set_mode(
            "/w/stepped-back",
            ProjectMode::NotifyOnly,
            t("2026-10-29T11:59:59Z"),
        )
        .unwrap();
        p.set_mode("/w/tie", ProjectMode::NotifyOnly, t("2026-10-29T12:00:00Z"))
            .unwrap();
        for key in ["/w/stepped-back", "/w/tie"] {
            assert!(p.asks_despite_grant(key), "{key}");
            assert!(!p.arms_by_default(key, "/s/a.jsonl", SRC), "{key}");
            assert_eq!(p.resolve(key), ProjectMode::NotifyOnly, "{key}");
        }
    }

    /// A policy file the #1747 daemon wrote records no grant generation:
    /// that build ordered a folder's Ask against the grant by wall clock.
    /// Loading it derives the generation once from that build's rule, so a
    /// folder that was an exception keeps asking -- an upgrade never arms a
    /// folder that was asking -- and one that was not stays superseded.
    #[test]
    fn an_ask_the_1747_daemon_kept_as_an_exception_survives_the_upgrade() {
        let (_d, store) = temp_store();
        let narrow = grant_terms_with("https://ingest.invalid");
        let mut p = granted_including_existing();
        p.set_mode(
            "/w/post",
            ProjectMode::NotifyOnly,
            t("2026-10-29T13:00:00Z"),
        )
        .unwrap();
        p.arm_by_grant("/w/api", t("2026-10-29T13:00:00Z"), narrow.clone())
            .unwrap();
        // As the #1747 build wrote it: none of the generation fields.
        let mut old = serde_json::to_value(&p).unwrap();
        let top = old.as_object_mut().unwrap();
        top.remove("grant_generation");
        top.remove("grant_exceptions_derived");
        top["automatic_grant"]
            .as_object_mut()
            .unwrap()
            .remove("generation");
        for entry in top["projects"].as_object_mut().unwrap().values_mut() {
            entry.as_object_mut().unwrap().remove("set_under_grant");
        }
        store
            .write_daemon_file(DAEMON_PROJECTS_FILE, &serde_json::to_vec(&old).unwrap())
            .unwrap();

        let mut loaded = ProjectPolicy::load(&store).unwrap();
        let generation = loaded.automatic_grant.as_ref().unwrap().generation;
        assert_ne!(generation, 0, "the loaded grant has an identity");
        assert_eq!(loaded.projects["/w/post"].set_under_grant, Some(generation));
        assert!(loaded.asks_despite_grant("/w/post"));
        assert!(!loaded.arms_by_default("/w/post", "/s/a.jsonl", SRC));
        assert!(
            loaded.arms_by_default("/w/pre", "/s/b.jsonl", SRC),
            "an Ask from before the grant is still superseded by it"
        );

        // A folder the grant armed, voided after the upgrade while the grant
        // survives, keeps asking as it would have before it.
        let mut wide = narrow;
        wide.consent_scopes.insert("model_training".to_string());
        loaded.automatic_grant.as_mut().unwrap().granted_under = wide.clone();
        let sweep = loaded.sweep_grants(&wide, t("2026-10-29T14:00:00Z"));
        assert_eq!(sweep.voided.len(), 1);
        assert!(!loaded.arms_by_default("/w/api", "/s/c.jsonl", SRC));

        // Derived once: saving and loading again changes nothing.
        loaded.save(&store).unwrap();
        assert_eq!(ProjectPolicy::load(&store).unwrap(), loaded);

        // A fresh include-existing grant after the upgrade supersedes it.
        let again = t("2026-10-29T16:00:00Z");
        loaded.grant_automatic(again, grant_terms_with("https://ingest.invalid"));
        loaded.include_existing_under_grant();
        assert!(loaded.record_source(again, SRC, BTreeSet::new(), BTreeSet::new()));
        assert!(!loaded.asks_despite_grant("/w/post"));
        assert!(loaded.arms_by_default("/w/post", "/s/a.jsonl", SRC));
    }

    /// A rekey that merges two spellings of one folder keeps an Ask that is
    /// an exception to the grant in force if either half was one: merging
    /// may only ever ask more permission than before, never less.
    #[test]
    fn a_rekey_merge_keeps_an_ask_made_under_the_grant() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();
        let root_key = root.to_string_lossy().to_string();
        let sub_key = sub.to_string_lossy().to_string();

        // The root's Ask is from before the grant, the subdirectory's after.
        let mut p = ProjectPolicy::new();
        p.set_mode(
            &root_key,
            ProjectMode::NotifyOnly,
            t("2026-10-29T11:00:00Z"),
        )
        .unwrap();
        let at = t("2026-10-29T12:00:00Z");
        p.grant_automatic(at, grant_terms_with("https://ingest.invalid"));
        p.include_existing_under_grant();
        assert!(p.record_source(at, SRC, BTreeSet::new(), BTreeSet::new()));
        p.set_mode(&sub_key, ProjectMode::NotifyOnly, t("2026-10-29T13:00:00Z"))
            .unwrap();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();

        p.rekey();
        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(p.projects.len(), 1, "the two spellings merged");
        assert!(p.asks_despite_grant(&key));
        assert!(!p.arms_by_default(&key, "/s/a.jsonl", SRC));
    }

    /// A downgrade to the #1747 daemon reads an Ask exception by
    /// `added_at > granted_at`. An Ask this build recorded under the grant
    /// with a stepped-back clock must still read as post-grant to that
    /// build, or the downgrade arms a folder that was asking.
    #[test]
    fn a_downgrade_to_the_1747_daemon_keeps_a_stepped_clock_ask_asking() {
        let (_d, store) = temp_store();
        let mut p = granted_including_existing();
        p.set_mode(
            "/w/stepped-back",
            ProjectMode::NotifyOnly,
            t("2026-10-29T11:59:59Z"),
        )
        .unwrap();
        assert!(p.asks_despite_grant("/w/stepped-back"));
        p.save(&store).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(
            &store
                .read_daemon_file(DAEMON_PROJECTS_FILE)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        let granted_at: DateTime<Utc> =
            serde_json::from_value(raw["automatic_grant"]["granted_at"].clone()).unwrap();
        let added_at: DateTime<Utc> =
            serde_json::from_value(raw["projects"]["/w/stepped-back"]["added_at"].clone()).unwrap();
        assert!(
            added_at > granted_at,
            "the #1747 daemon would read this Ask as pre-grant and arm the folder"
        );
    }

    /// The arming offer (`arming_suggestion`) never offers to turn on
    /// Automatic for a folder whose `list_projects` row already reads
    /// Automatic because the include-existing grant will arm it.
    #[test]
    fn no_arming_offer_for_a_folder_the_broad_grant_will_arm() {
        let mut p = granted_including_existing();
        p.contributed
            .insert("/w/pre".to_string(), ARMING_SUGGESTION_THRESHOLD);
        assert_eq!(p.shown_folder_mode("/w/pre"), ProjectMode::AutoUpload);
        assert!(
            p.arming_suggestion(t("2026-10-29T13:00:00Z")).is_none(),
            "offered to arm a folder already shown as Automatic"
        );
    }

    /// What a shell shows for a folder is what the daemon will do with it:
    /// under the include-existing grant a folder the grant will arm reads
    /// Automatic, and nothing else moves. With no grant, or a legacy
    /// from-now grant, the shown mode is the resolved one.
    #[test]
    fn a_folder_the_grant_will_arm_shows_as_automatic_and_nothing_else_moves() {
        let mut p = granted_including_existing();
        p.set_mode(
            "/w/post",
            ProjectMode::NotifyOnly,
            t("2026-10-29T13:00:00Z"),
        )
        .unwrap();
        p.set_mode("/w/never", ProjectMode::Ignore, t("2026-10-29T13:00:00Z"))
            .unwrap();
        for (key, shown) in [
            ("/w/pre", ProjectMode::AutoUpload),
            ("/w/none", ProjectMode::AutoUpload),
            ("/w/post", ProjectMode::NotifyOnly),
            ("/w/never", ProjectMode::Ignore),
            (UNKNOWN_PROJECT_KEY, ProjectMode::NotifyOnly),
        ] {
            assert_eq!(p.shown_folder_mode(key), shown, "{key}");
            assert_eq!(p.shown_mode(key), shown, "{key}");
            assert_eq!(
                p.grant_will_arm(key),
                shown == ProjectMode::AutoUpload,
                "{key}"
            );
        }
        assert_eq!(
            p.resolve("/w/pre"),
            ProjectMode::NotifyOnly,
            "stored mode untouched"
        );
        // An Ask override still reads Ask over a folder the grant will arm.
        p.set_contribution_override(ProjectMode::NotifyOnly, t("2026-10-29T14:00:00Z"), None)
            .unwrap();
        assert_eq!(p.shown_mode("/w/pre"), ProjectMode::NotifyOnly);
        assert_eq!(p.shown_folder_mode("/w/pre"), ProjectMode::AutoUpload);

        for p in [ProjectPolicy::new(), granted_with_disk(&[], &[])] {
            let mut p = p;
            p.set_mode("/w/ask", ProjectMode::NotifyOnly, t("2026-10-29T13:00:00Z"))
                .unwrap();
            for key in ["/w/ask", "/w/none"] {
                assert!(!p.grant_will_arm(key), "{key}");
                assert_eq!(p.shown_mode(key), p.resolve(key), "{key}");
                assert_eq!(p.shown_folder_mode(key), p.folder_mode(key), "{key}");
            }
        }
    }

    /// The exception belongs to the include-existing grant: under a legacy
    /// from-now grant every folder with an entry already keeps its own mode,
    /// and with no grant there is nothing to be an exception to.
    #[test]
    fn asking_despite_the_grant_needs_an_include_existing_grant() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, t("2026-10-29T13:00:00Z"))
            .unwrap();
        assert!(!p.asks_despite_grant("/w/ask"));
        let mut p = granted_with_disk(&[], &[]);
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, t("2026-10-29T13:00:00Z"))
            .unwrap();
        assert!(!p.asks_despite_grant("/w/ask"));
        assert!(!p.arms_by_default("/w/ask", "/s/a.jsonl", SRC));
    }

    /// #1739's keep-alive: adopting the contributor's chosen data uses
    /// keeps the grant, and keeps a folder's Ask chosen under it.
    #[test]
    fn a_folder_ask_under_the_grant_survives_adopting_chosen_scopes() {
        let mut p = granted_including_existing();
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, t("2026-10-29T13:00:00Z"))
            .unwrap();
        let mut widened = grant_terms_with("https://ingest.invalid");
        widened.consent_scopes.insert("model_training".to_string());
        assert!(p.adopt_chosen_scopes(&widened.consent_scopes));
        let sweep = p.sweep_grants(&widened, t("2026-10-29T14:00:00Z"));
        assert!(sweep.automatic_grant_voided.is_none());
        assert!(p.includes_existing_automatically());
        assert!(p.asks_despite_grant("/w/ask"));
        assert!(!p.arms_by_default("/w/ask", "/s/a.jsonl", SRC));
        assert_eq!(p.resolve("/w/ask"), ProjectMode::NotifyOnly);
    }

    /// A folder armed after the grant and then voided while the grant
    /// survives returns to Ask and stays there: the void's "asks again" is
    /// not undone by the grant re-arming it on the next session. The
    /// fail-closed reading, and the one `sweep_grants` documents.
    #[test]
    fn a_folder_voided_after_the_grant_keeps_asking() {
        let mut p = granted_including_existing();
        let narrow = grant_terms_with("https://ingest.invalid");
        p.arm_by_grant("/w/api", t("2026-10-29T13:00:00Z"), narrow.clone())
            .unwrap();
        // The folder's terms widen while the grant's own still cover them.
        let mut wide = narrow;
        wide.consent_scopes.insert("model_training".to_string());
        p.automatic_grant.as_mut().unwrap().granted_under = wide.clone();
        let sweep = p.sweep_grants(&wide, t("2026-10-29T14:00:00Z"));
        assert_eq!(sweep.voided.len(), 1);
        assert!(sweep.automatic_grant_voided.is_none());
        assert_eq!(p.resolve("/w/api"), ProjectMode::NotifyOnly);
        assert!(!p.arms_by_default("/w/api", "/s/a.jsonl", SRC));
    }

    /// R6's ship condition, in the policy: every void leaves a notice for
    /// the shells, one per project returned to ask-first and one for the
    /// Flow 1 grant, carrying the same reason labels as the audit.
    #[test]
    fn a_void_leaves_a_notice_for_each_grant_it_stopped() {
        let mut p = armed_and_granted("https://ingest.invalid");
        let at = t("2026-09-25T04:00:00Z");

        p.sweep_grants(&grant_terms_with("https://ingest.invalid"), at);
        assert!(p.grant_voids.is_empty(), "nothing voided, nothing to say");

        p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), at);
        assert_eq!(p.grant_voids.len(), 2, "{:?}", p.grant_voids);
        let project = p
            .grant_voids
            .iter()
            .find(|n| n.project_key.is_some())
            .expect("a notice for the project");
        assert_eq!(project.project_key.as_deref(), Some("/w/api"));
        assert_eq!(project.reasons, vec!["destination-changed".to_string()]);
        assert_eq!(project.voided_at, at);
        let grant = p
            .grant_voids
            .iter()
            .find(|n| n.project_key.is_none())
            .expect("a notice for the automatic grant");
        assert_eq!(grant.reasons, vec!["destination-changed".to_string()]);
        assert_ne!(
            project.id, grant.id,
            "each notice is its own to acknowledge"
        );
    }

    /// What the grant's notice says about projects (`VOID_GRANT_PROJECTS`)
    /// is what the sweep does: a project armed under terms that still cover
    /// what is in force stays armed and gets no notice, while the grant
    /// armed under older terms is voided.
    #[test]
    fn voiding_the_grant_leaves_a_project_whose_terms_still_cover_it() {
        let mut p = granted_with_disk(&[], &[]);
        let now = t("2026-09-25T03:00:00Z");
        p.set_mode("/w/new", ProjectMode::AutoUpload, now).unwrap();
        p.record_grant_terms("/w/new", grant_terms_with("https://elsewhere.invalid"));

        let sweep = p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), now);
        assert!(sweep.automatic_grant_voided.is_some());
        assert!(sweep.voided.is_empty());
        assert_eq!(p.resolve("/w/new"), ProjectMode::AutoUpload);
        assert_eq!(p.grant_voids.len(), 1);
        assert!(p.grant_voids[0].project_key.is_none());
    }

    /// A shell acknowledges exactly the notices it showed. An id it did not
    /// name stays, and a stale or unknown id is not an error.
    #[test]
    fn acknowledging_clears_only_the_notices_named() {
        let mut p = armed_and_granted("https://ingest.invalid");
        p.sweep_grants(
            &grant_terms_with("https://elsewhere.invalid"),
            t("2026-09-25T04:00:00Z"),
        );
        let ids: Vec<u64> = p.grant_voids.iter().map(|n| n.id).collect();
        assert_eq!(ids.len(), 2);

        assert_eq!(p.acknowledge_grant_voids(&[ids[0], 9_999]), 1);
        assert_eq!(p.grant_voids.len(), 1);
        assert_eq!(p.grant_voids[0].id, ids[1]);
        assert_eq!(p.acknowledge_grant_voids(&[ids[0]]), 0, "already gone");
        assert_eq!(p.acknowledge_grant_voids(&[ids[1]]), 1);
        assert!(p.grant_voids.is_empty());
    }

    /// Ids are never reused, so an acknowledgement for an old notice cannot
    /// clear one raised after the shell drew.
    #[test]
    fn a_later_void_never_reuses_an_acknowledged_id() {
        let mut p = armed_and_granted("https://ingest.invalid");
        p.sweep_grants(
            &grant_terms_with("https://elsewhere.invalid"),
            t("2026-09-25T04:00:00Z"),
        );
        let first: Vec<u64> = p.grant_voids.iter().map(|n| n.id).collect();
        assert_eq!(p.acknowledge_grant_voids(&first), 2);

        let now = t("2026-09-25T05:00:00Z");
        p.set_mode("/w/api", ProjectMode::AutoUpload, now).unwrap();
        p.record_grant_terms("/w/api", grant_terms_with("https://elsewhere.invalid"));
        p.sweep_grants(&grant_terms_with("https://third.invalid"), now);
        assert_eq!(p.grant_voids.len(), 1);
        assert!(!first.contains(&p.grant_voids[0].id));
        assert_eq!(p.acknowledge_grant_voids(&first), 0);
        assert_eq!(p.grant_voids.len(), 1);
    }

    /// Acting on what a notice is about makes it stale: setting that
    /// project's mode clears its notice, and giving the grant again clears
    /// the grant's. Neither touches the other.
    #[test]
    fn acting_on_a_voided_grant_clears_its_notice() {
        let mut p = armed_and_granted("https://ingest.invalid");
        let now = t("2026-09-25T04:00:00Z");
        p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), now);
        assert_eq!(p.grant_voids.len(), 2);

        p.set_mode("/w/api", ProjectMode::AutoUpload, now).unwrap();
        assert_eq!(p.grant_voids.len(), 1);
        assert!(p.grant_voids[0].project_key.is_none());

        p.grant_automatic(now, grant_terms_with("https://elsewhere.invalid"));
        assert!(p.grant_voids.is_empty());
    }

    /// A policy file written before notices existed still loads, with none.
    #[test]
    fn a_policy_file_without_notices_loads_with_none() {
        let mut value = serde_json::to_value(ProjectPolicy::new()).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("grant_voids");
        object.remove("next_grant_void_id");
        object.remove("scrub_check_upgrade_recorded");
        let p: ProjectPolicy = serde_json::from_value(value).unwrap();
        assert!(p.grant_voids.is_empty());
        assert_eq!(p.next_grant_void_id, 0);
        assert!(!p.scrub_check_upgrade_recorded);
    }

    use super::super::arming_wording::ArmingClaim;

    fn armed_by_hand(keys: &[&str]) -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        for key in keys {
            p.set_mode(key, ProjectMode::AutoUpload, t("2026-09-27T00:00:00Z"))
                .unwrap();
        }
        p
    }

    #[test]
    fn scrub_check_upgrade_is_once_only_and_raises_no_notice() {
        let now = t("2026-09-30T00:00:00Z");
        let mut p = armed_by_hand(&["/w/api"]);
        p.scrub_check_upgrade_recorded = false;
        assert!(p.record_scrub_check_upgrade(true, now));
        assert!(p.arming_rewordings.is_empty());
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert!(p.arming_rewordings.is_empty());
        assert!(!p.record_scrub_check_upgrade(true, now));
        p.set_mode("/w/api", ProjectMode::NotifyOnly, now).unwrap();
        assert!(p.arming_rewordings.is_empty());
    }

    #[test]
    fn scrub_check_upgrade_does_not_notify_manual_or_later_armed_folders() {
        let now = t("2026-09-30T00:00:00Z");
        let mut manual = armed_by_hand(&["/w/api"]);
        manual.scrub_check_upgrade_recorded = false;
        assert!(manual.record_scrub_check_upgrade(false, now));
        assert!(manual.arming_rewordings.is_empty());
        let mut no_folders = ProjectPolicy::new();
        no_folders.scrub_check_upgrade_recorded = false;
        assert!(no_folders.record_scrub_check_upgrade(true, now));
        no_folders
            .set_mode("/w/api", ProjectMode::AutoUpload, now)
            .unwrap();
        assert!(!no_folders.record_scrub_check_upgrade(true, now));
        assert!(no_folders.arming_rewordings.is_empty());
    }

    /// K5: a folder armed under the old "will be scrubbed" wording, whose
    /// wording in force is patterns-only, has its claim moved and stays
    /// armed. No notice is raised.
    #[test]
    fn a_folder_armed_under_the_scrub_wording_has_its_claim_moved_when_reworded() {
        let mut p = armed_by_hand(&["/w/api"]);
        let at = t("2026-09-27T01:00:00Z");

        let unchanged = p.sweep_arming_claims(|_| ArmingClaim::ModelScrubbed, at);
        assert!(unchanged.is_empty());
        assert!(p.arming_rewordings.is_empty());

        let reworded = p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at);
        assert_eq!(reworded, vec!["api".to_string()]);
        assert!(p.arming_rewordings.is_empty(), "{:?}", p.arming_rewordings);
        assert_eq!(p.arming_claim("/w/api"), ArmingClaim::PatternsOnly);
        assert_eq!(p.resolve("/w/api"), ProjectMode::AutoUpload, "still armed");
    }

    /// Reworded once: the next pass under the same wording finds nothing.
    #[test]
    fn a_rewording_is_recorded_once() {
        let mut p = armed_by_hand(&["/w/api"]);
        let at = t("2026-09-27T01:00:00Z");
        assert_eq!(
            p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at)
                .len(),
            1
        );
        assert!(
            p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at)
                .is_empty()
        );
        assert!(p.arming_rewordings.is_empty());
    }

    /// Only a narrowing is owed a notice. A folder the grant armed was shown
    /// the patterns-only wording, so it has nothing to be told; one armed
    /// by hand and recorded as patterns-only neither.
    #[test]
    fn a_folder_told_patterns_only_is_not_reworded() {
        let mut p = granted_with_disk(&[], &[]);
        let now = t("2026-09-27T00:00:00Z");
        p.arm_by_grant("/w/new", now, grant_terms_with("https://ingest.invalid"))
            .unwrap();
        p.set_mode("/w/hand", ProjectMode::AutoUpload, now).unwrap();
        assert!(p.record_arming_claim("/w/hand", ArmingClaim::PatternsOnly));
        assert!(
            p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now)
                .is_empty()
        );
        assert!(p.arming_rewordings.is_empty());
        // And a widening is not a notice either.
        assert!(
            p.sweep_arming_claims(|_| ArmingClaim::ModelScrubbed, now)
                .is_empty()
        );
    }

    /// Ask-first and ignored folders were never told their sessions would
    /// be sent, so there is nothing to reword.
    #[test]
    fn only_armed_folders_are_reworded() {
        let mut p = ProjectPolicy::new();
        let now = t("2026-09-27T00:00:00Z");
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, now).unwrap();
        p.set_mode("/w/never", ProjectMode::Ignore, now).unwrap();
        assert!(
            p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now)
                .is_empty()
        );
        assert!(!p.record_arming_claim("/w/ask", ArmingClaim::ModelScrubbed));
    }

    /// Setting the project's mode drops its recorded claim, whether it is
    /// armed again (under words recorded afresh) or switched to ask-first.
    #[test]
    fn setting_the_mode_clears_the_recorded_claim() {
        let mut p = armed_by_hand(&["/w/api", "/w/web"]);
        let now = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert!(p.arming_rewordings.is_empty());
        assert!(p.arming_claims.contains_key("/w/api"));
        p.set_mode("/w/api", ProjectMode::NotifyOnly, now).unwrap();
        assert!(!p.arming_claims.contains_key("/w/api"));
        assert!(p.arming_claims.contains_key("/w/web"));
        p.set_mode("/w/web", ProjectMode::AutoUpload, now).unwrap();
        assert!(p.arming_rewordings.is_empty());
    }

    /// A void returns the folder to ask-first and takes its recorded claim.
    #[test]
    fn a_void_takes_the_recorded_claim_with_it() {
        let mut p = armed_and_granted("https://ingest.invalid");
        let now = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert!(p.arming_rewordings.is_empty());
        assert!(p.arming_claims.contains_key("/w/api"));
        p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), now);
        assert!(p.arming_rewordings.is_empty());
        assert!(!p.arming_claims.contains_key("/w/api"));
    }

    /// A policy file written before rewordings existed loads with none,
    /// and its armed folders read as armed under the scrub wording.
    #[test]
    fn a_policy_file_without_arming_claims_loads_as_the_old_wording() {
        let p = armed_by_hand(&["/w/api"]);
        let mut value = serde_json::to_value(&p).unwrap();
        let object = value.as_object_mut().unwrap();
        object.remove("arming_claims");
        object.remove("arming_rewordings");
        object.remove("next_arming_reword_id");
        let p: ProjectPolicy = serde_json::from_value(value).unwrap();
        assert!(p.arming_rewordings.is_empty());
        assert_eq!(p.arming_claim("/w/api"), ArmingClaim::ModelScrubbed);
    }

    fn armed_policy() -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
        }
        p
    }

    fn t(s: &str) -> DateTime<Utc> {
        s.parse().unwrap()
    }

    #[test]
    fn a_project_is_offered_once_it_has_contributed_enough() {
        let p = armed_policy();
        let s = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        assert_eq!(s.project_label, "api");
        assert_eq!(s.contributed_count, ARMING_SUGGESTION_THRESHOLD);
        assert_eq!(s.project_id, project_id_for("/Users/z/code/api"));
    }

    /// The offer has to be backed by evidence the contributor actually has.
    #[test]
    fn a_project_below_the_threshold_is_not_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD - 1) {
            p.record_contribution("/Users/z/code/api");
        }
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

    /// An armed project has nothing to offer and an ignored one has been
    /// answered already. Neither is a question worth asking again.
    #[test]
    fn an_armed_or_ignored_project_is_never_offered() {
        for mode in [ProjectMode::AutoUpload, ProjectMode::Ignore] {
            let mut p = armed_policy();
            p.set_mode("/Users/z/code/api", mode, t("2026-08-31T11:00:00Z"))
                .unwrap();
            assert!(
                p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none(),
                "{mode:?} must not be offered"
            );
        }
    }

    /// The bucket can never be armed, so counting it could only ever feed an
    /// offer the daemon would refuse.
    #[test]
    fn the_unresolvable_bucket_is_never_counted_or_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD * 3) {
            p.record_contribution(UNKNOWN_PROJECT_KEY);
        }
        assert!(!p.contributed.contains_key(UNKNOWN_PROJECT_KEY));
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

    #[test]
    fn declining_silences_the_offer_for_the_cooldown() {
        let mut p = armed_policy();
        p.decline_arming("/Users/z/code/api", t("2026-08-01T12:00:00Z"));
        assert!(p.arming_suggestion(t("2026-08-15T12:00:00Z")).is_none());
    }

    /// "Not now" says not now. A suppression that never lifted would make
    /// those words a lie.
    #[test]
    fn the_offer_returns_after_the_cooldown() {
        let mut p = armed_policy();
        p.decline_arming("/Users/z/code/api", t("2026-08-01T12:00:00Z"));
        assert!(p.arming_suggestion(t("2026-09-01T12:00:00Z")).is_some());
    }

    /// At most one offer, ever. A queue that sprouts one per project is the
    /// ongoing administration this is supposed to remove.
    #[test]
    fn only_the_strongest_candidate_is_offered() {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
        }
        for _ in 0..(ARMING_SUGGESTION_THRESHOLD + 4) {
            p.record_contribution("/Users/z/code/web");
        }
        let s = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        assert_eq!(s.project_label, "web");
    }

    /// Two projects on the same count must not shuffle between runs.
    #[test]
    fn a_tie_is_broken_stably() {
        let mut p = ProjectPolicy::new();
        for _ in 0..ARMING_SUGGESTION_THRESHOLD {
            p.record_contribution("/Users/z/code/api");
            p.record_contribution("/Users/z/code/web");
        }
        let first = p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap();
        for _ in 0..20 {
            assert_eq!(
                p.arming_suggestion(t("2026-08-31T12:00:00Z")).unwrap(),
                first
            );
        }
    }

    /// A policy file written before these fields existed must still parse,
    /// and read as "nothing counted yet" rather than failing the whole file
    /// and losing every mode the contributor had set.
    #[test]
    fn a_policy_file_without_the_new_fields_still_parses() {
        let json = format!(r#"{{"schema_version":"{DAEMON_PROJECTS_SCHEMA}","projects":{{}}}}"#);
        let p: ProjectPolicy = serde_json::from_str(&json).unwrap();
        assert!(p.contributed.is_empty());
        assert!(p.arming_declined_at.is_empty());
        assert!(p.arming_suggestion(t("2026-08-31T12:00:00Z")).is_none());
    }

    #[test]
    fn two_recordings_of_one_directory_share_a_key() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("crates").join("inner");
        std::fs::create_dir_all(&sub).unwrap();

        // What Claude Code records, and what Codex records, for one repo.
        let from_root = project_key_for(Some(root.to_str().unwrap()));
        let from_sub = project_key_for(Some(sub.to_str().unwrap()));

        assert_eq!(from_root, from_sub);
        assert_eq!(project_id_for(&from_root), project_id_for(&from_sub));
    }

    #[test]
    fn an_unusable_cwd_still_lands_in_the_unknown_bucket() {
        assert_eq!(project_key_for(None), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("")), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("/")), UNKNOWN_PROJECT_KEY);
        assert_eq!(project_key_for(Some("relative")), UNKNOWN_PROJECT_KEY);
    }

    #[test]
    fn ignore_survives_a_rekey_that_merges_two_entries() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("crates").join("inner");
        std::fs::create_dir_all(&sub).unwrap();

        // A v1 file: two entries that Task 1 collapses into one key, with
        // the more permissive mode listed second so a naive last-write-wins
        // would lose the Ignore.
        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            root.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: "repo".to_string(),
                display_path: None,
                armed_under: None,
                automatic_redaction: Default::default(),
                set_under_grant: None,
            },
        );
        p.projects.insert(
            sub.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::AutoUpload,
                added_at: now(),
                label: "inner".to_string(),
                display_path: None,
                armed_under: None,
                automatic_redaction: Default::default(),
                set_under_grant: None,
            },
        );

        p.rekey();

        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(p.projects.len(), 1);
        assert_eq!(p.resolve(&key), ProjectMode::Ignore);
        assert_eq!(p.schema_version, DAEMON_PROJECTS_SCHEMA);
    }

    #[test]
    fn a_rekey_carries_contribution_counts_and_declines_across() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.contributed.insert(root.to_string_lossy().to_string(), 3);
        p.contributed.insert(sub.to_string_lossy().to_string(), 4);
        p.arming_declined_at
            .insert(sub.to_string_lossy().to_string(), now());

        p.rekey();

        let key = project_key_for(Some(root.to_str().unwrap()));
        // Counts for two spellings of one project are one project's count.
        assert_eq!(p.contributed.get(&key), Some(&7));
        assert!(p.arming_declined_at.contains_key(&key));
    }

    #[test]
    fn a_rekey_is_idempotent() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.set_mode(root.to_str().unwrap(), ProjectMode::Ignore, now())
            .unwrap();

        p.rekey();
        let once = p.clone();
        p.rekey();
        assert_eq!(p, once);
    }

    #[test]
    fn loading_a_v1_file_rekeys_it() {
        let (_d, store) = temp_store();
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("repo");
        std::fs::create_dir(&root).unwrap();
        std::fs::create_dir(root.join(".git")).unwrap();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).unwrap();

        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            sub.to_string_lossy().to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: "sub".to_string(),
                display_path: None,
                armed_under: None,
                automatic_redaction: Default::default(),
                set_under_grant: None,
            },
        );
        p.save(&store).unwrap();

        let loaded = ProjectPolicy::load(&store).unwrap();
        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(loaded.resolve(&key), ProjectMode::Ignore);
    }

    /// #992: a v2 file can hold a key `daemon project <path>` stored in the
    /// on-disk spelling, which the watcher (folding case on macOS and
    /// Windows) never looked up. Loading it must put an `Ignore` where the
    /// watcher looks.
    ///
    /// An `AutoUpload` under such a key was never in effect, and its
    /// arm-from-now hold sits under the old spelling too. Carrying the mode
    /// across without the hold would start sending the backlog the
    /// contributor held back, so it asks again instead.
    #[test]
    fn loading_a_v2_file_rekeys_keys_the_cli_stored_unfolded() {
        let (_d, store) = temp_store();
        let dir = tempfile::tempdir().unwrap();
        let never = dir.path().join("NeverRepo");
        let armed = dir.path().join("ArmedRepo");
        std::fs::create_dir(&never).unwrap();
        std::fs::create_dir(&armed).unwrap();
        // The CLI's spelling: canonical, with the case on disk.
        let never_cli = std::fs::canonicalize(&never)
            .unwrap()
            .to_string_lossy()
            .into_owned();
        let armed_cli = std::fs::canonicalize(&armed)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let mut p = ProjectPolicy::new();
        p.set_mode(&never_cli, ProjectMode::Ignore, now()).unwrap();
        p.set_mode(&armed_cli, ProjectMode::AutoUpload, now())
            .unwrap();
        p.arm_from_now(&armed_cli, now());
        p.schema_version = "trace_commons.daemon_projects.v2".to_string();
        p.save(&store).unwrap();

        let loaded = ProjectPolicy::load(&store).unwrap();
        let never_key = project_key_for(Some(&never_cli));
        let armed_key = project_key_for(Some(&armed_cli));
        assert_eq!(loaded.resolve(&never_key), ProjectMode::Ignore);
        assert_eq!(loaded.schema_version, DAEMON_PROJECTS_SCHEMA);
        if armed_key == armed_cli {
            // Linux: the CLI's key already was the watcher's.
            assert_eq!(loaded.resolve(&armed_key), ProjectMode::AutoUpload);
            assert!(loaded.armed_from_now.contains_key(&armed_key));
        } else {
            assert_eq!(loaded.resolve(&armed_key), ProjectMode::NotifyOnly);
            assert!(
                loaded.armed_from_now.is_empty(),
                "no hold left behind under a key nothing looks up"
            );
        }
        for key in loaded.projects.keys() {
            assert_eq!(
                crate::daemon::project_key::normalize_project_key(key).as_deref(),
                Some(key.as_str()),
                "every stored key is the watcher's"
            );
        }
    }

    /// The same file under an "Automatic" override: a miss there reads as
    /// `AutoUpload`, so the migration is what stands between a Never set
    /// from the CLI and an unattended upload.
    #[test]
    fn a_v2_never_set_from_the_cli_holds_under_an_automatic_override() {
        let (_d, store) = temp_store();
        let dir = tempfile::tempdir().unwrap();
        let never = dir.path().join("NeverRepo");
        std::fs::create_dir(&never).unwrap();
        let never_cli = std::fs::canonicalize(&never)
            .unwrap()
            .to_string_lossy()
            .into_owned();

        let mut p = ProjectPolicy::new();
        p.set_mode(&never_cli, ProjectMode::Ignore, now()).unwrap();
        p.contribution_override = Some(ContributionOverride {
            mode: ProjectMode::AutoUpload,
            since: now(),
            arming: None,
            granted_under: None,
            claim: None,
        });
        p.schema_version = "trace_commons.daemon_projects.v2".to_string();
        p.save(&store).unwrap();

        let loaded = ProjectPolicy::load(&store).unwrap();
        assert_eq!(
            loaded.resolve(&project_key_for(Some(&never_cli))),
            ProjectMode::Ignore
        );
    }

    #[test]
    fn the_unknown_bucket_is_never_rekeyed() {
        let mut p = ProjectPolicy::new();
        p.schema_version = DAEMON_PROJECTS_SCHEMA_V1.to_string();
        p.projects.insert(
            UNKNOWN_PROJECT_KEY.to_string(),
            ProjectEntry {
                mode: ProjectMode::Ignore,
                added_at: now(),
                label: UNKNOWN_PROJECT_KEY.to_string(),
                display_path: None,
                armed_under: None,
                automatic_redaction: Default::default(),
                set_under_grant: None,
            },
        );
        p.rekey();
        assert_eq!(p.resolve(UNKNOWN_PROJECT_KEY), ProjectMode::Ignore);
    }

    #[test]
    fn ignore_is_the_most_restrictive_mode() {
        use ProjectMode::*;
        assert_eq!(Ignore.more_restrictive(AutoUpload), Ignore);
        assert_eq!(AutoUpload.more_restrictive(Ignore), Ignore);
        assert_eq!(NotifyOnly.more_restrictive(AutoUpload), NotifyOnly);
        assert_eq!(AutoUpload.more_restrictive(AutoUpload), AutoUpload);
    }

    // -- The contribution override (#1173) ---------------------------------

    /// Three folders, one per mode, set by the contributor.
    fn one_folder_per_mode() -> ProjectPolicy {
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-01T00:00:00Z");
        p.set_mode("/w/auto", ProjectMode::AutoUpload, at).unwrap();
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, at).unwrap();
        p.set_mode("/w/never", ProjectMode::Ignore, at).unwrap();
        p
    }

    fn modes(p: &ProjectPolicy) -> Vec<ProjectMode> {
        [
            "/w/auto",
            "/w/ask",
            "/w/never",
            "/w/unseen",
            UNKNOWN_PROJECT_KEY,
        ]
        .iter()
        .map(|k| p.resolve(k))
        .collect()
    }

    /// The override never writes a folder's own mode, so clearing it puts
    /// every folder back exactly, whichever override was in force.
    #[test]
    fn clearing_the_override_restores_each_folders_own_mode() {
        let mut p = one_folder_per_mode();
        let before = modes(&p);
        let entries = p.projects.clone();
        for mode in [
            ProjectMode::AutoUpload,
            ProjectMode::NotifyOnly,
            ProjectMode::Ignore,
        ] {
            assert!(override_to(&mut p, mode, t("2026-10-02T00:00:00Z")));
            assert_eq!(p.projects, entries, "{mode:?} wrote a folder's entry");
            assert!(p.clear_contribution_override());
            assert_eq!(modes(&p), before, "{mode:?}");
        }
        assert!(!p.clear_contribution_override(), "nothing left to clear");
    }

    /// "Ask" and "Never" leave nothing to go unattended, in any folder,
    /// including one armed by its own mode.
    #[test]
    fn ask_and_never_overrides_stop_every_unattended_send() {
        use ProjectMode::*;
        let mut p = one_folder_per_mode();
        override_to(&mut p, NotifyOnly, t("2026-10-02T00:00:00Z"));
        assert_eq!(
            modes(&p),
            vec![NotifyOnly, NotifyOnly, Ignore, NotifyOnly, NotifyOnly]
        );
        override_to(&mut p, Ignore, t("2026-10-02T00:00:00Z"));
        assert_eq!(modes(&p), vec![Ignore; 5]);
        assert_eq!(p.folder_mode("/w/auto"), AutoUpload);
    }

    /// "Automatic" reaches every folder but one set to Never, which a
    /// global override must not reach into, and the unknown bucket, which can
    /// never be armed.
    #[test]
    fn an_auto_override_leaves_never_folders_and_the_unknown_bucket_alone() {
        use ProjectMode::*;
        let mut p = one_folder_per_mode();
        override_to(&mut p, AutoUpload, t("2026-10-02T00:00:00Z"));
        assert_eq!(
            modes(&p),
            vec![AutoUpload, AutoUpload, Ignore, AutoUpload, NotifyOnly]
        );
    }

    /// "Automatic" arms nothing already on disk: in every folder not
    /// armed by its own mode it is an arming from now, so the backlog -- and
    /// everything, before a source is recorded -- waits for a person. What
    /// appears afterwards goes. A folder already armed keeps its own rules.
    #[test]
    fn an_auto_override_never_sends_an_existing_backlog() {
        let mut p = one_folder_per_mode();
        let at = t("2026-10-02T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        for folder in ["/w/ask", "/w/unseen"] {
            assert!(
                p.waits_for_a_person(folder, "/s/pre.jsonl", SRC),
                "{folder}"
            );
            assert!(
                p.waits_for_a_person(folder, "/s/new.jsonl", SRC),
                "{folder}"
            );
            assert!(p.waits_for_a_person_at_send(folder, "/s/pre.jsonl"));
            assert_eq!(p.armed_from_now_at(folder), Some(at));
        }
        let armings = p.armings_from_now();
        assert!(armings.contains(&(OVERRIDE_ARMING_KEY.to_string(), at)));
        assert!(p.record_source_for_armings(&armings, SRC, set_of(&["/s/pre.jsonl"])));
        for folder in ["/w/ask", "/w/unseen"] {
            assert!(
                p.waits_for_a_person(folder, "/s/pre.jsonl", SRC),
                "{folder}"
            );
            assert!(p.waits_for_a_person_at_send(folder, "/s/pre.jsonl"));
            assert!(
                !p.waits_for_a_person(folder, "/s/new.jsonl", SRC),
                "{folder}"
            );
            assert!(!p.waits_for_a_person_at_send(folder, "/s/new.jsonl"));
            assert!(p.waits_for_a_person(folder, "/s/x.jsonl", "codex /late"));
        }
        // Armed by its own plain arming: its backlog was already the
        // contributor's to send, and the override changes nothing about it.
        assert!(!p.waits_for_a_person("/w/auto", "/s/pre.jsonl", SRC));
        assert_eq!(p.armed_from_now_at("/w/auto"), None);

        // A newer session that turns up at a path the record never saw, but
        // whose content predates the override, is held like the backlog.
        assert!(p.hold_for_arming("/w/ask", "/s/resumed.jsonl", SRC));
        assert!(p.waits_for_a_person("/w/ask", "/s/resumed.jsonl", SRC));

        // Re-sending the same override keeps the hold it had.
        assert!(!override_to(
            &mut p,
            ProjectMode::AutoUpload,
            t("2026-10-03T00:00:00Z")
        ));
        assert!(p.waits_for_a_person("/w/ask", "/s/pre.jsonl", SRC));
        assert!(!p.waits_for_a_person("/w/ask", "/s/new.jsonl", SRC));

        // Cleared, the record goes with it.
        p.clear_contribution_override();
        assert!(p.sessions_on_disk_at_arming.is_empty());
        assert!(p.armings_from_now().is_empty());
    }

    /// The arming offer reads the folder's own mode, so an override neither
    /// offers to arm a folder that is armed nor hides one that asks.
    #[test]
    fn the_arming_offer_ignores_the_override() {
        let mut p = ProjectPolicy::new();
        p.set_mode("/w/ask", ProjectMode::NotifyOnly, t("2026-10-01T00:00:00Z"))
            .unwrap();
        p.contributed
            .insert("/w/ask".to_string(), ARMING_SUGGESTION_THRESHOLD);
        override_to(&mut p, ProjectMode::AutoUpload, t("2026-10-02T00:00:00Z"));
        assert!(p.arming_suggestion(t("2026-10-02T00:00:00Z")).is_some());
    }

    /// A policy file written before the override existed loads with none,
    /// and every folder resolves to its own mode.
    #[test]
    fn a_policy_file_without_an_override_loads_with_none() {
        let mut value = serde_json::to_value(one_folder_per_mode()).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .remove("contribution_override");
        let p: ProjectPolicy = serde_json::from_value(value).unwrap();
        assert!(p.contribution_override.is_none());
        assert_eq!(modes(&p), modes(&one_folder_per_mode()));

        let mut armed = one_folder_per_mode();
        override_to(
            &mut armed,
            ProjectMode::AutoUpload,
            t("2026-10-02T00:00:00Z"),
        );
        let back: ProjectPolicy =
            serde_json::from_value(serde_json::to_value(&armed).unwrap()).unwrap();
        assert_eq!(back, armed, "the override round-trips");
    }

    /// The pill's roll-up: one shared mode, `None` ("Mixed") when folders
    /// differ, the override's mode while one is in force, and the default
    /// with no folders. The unknown bucket never makes it mixed.
    #[test]
    fn the_roll_up_reports_mixed_and_yields_to_the_override() {
        use ProjectMode::*;
        let p = ProjectPolicy::new();
        assert_eq!(p.contribution_mode([]), Some(NotifyOnly));
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-01T00:00:00Z");
        p.set_mode("/w/a", AutoUpload, at).unwrap();
        p.set_mode("/w/b", AutoUpload, at).unwrap();
        assert_eq!(p.contribution_mode([UNKNOWN_PROJECT_KEY]), Some(AutoUpload));
        assert_eq!(
            p.contribution_mode(["/w/queued"]),
            None,
            "a discovered folder asks first"
        );
        p.set_mode("/w/b", Ignore, at).unwrap();
        assert_eq!(p.contribution_mode([]), None);
        override_to(&mut p, NotifyOnly, at);
        assert_eq!(p.contribution_mode([]), Some(NotifyOnly));
        p.clear_contribution_override();
        assert_eq!(p.contribution_mode([]), None);
    }

    /// Set the override as `set_contribution_override` does: an `AutoUpload`
    /// override under the terms `grant_terms_with` gives for the default
    /// destination, and the arming claim every shell's arming offer makes.
    fn override_to(p: &mut ProjectPolicy, mode: ProjectMode, at: DateTime<Utc>) -> bool {
        let grant = (mode == ProjectMode::AutoUpload).then(|| {
            (
                grant_terms_with("https://ingest.invalid"),
                super::super::arming_wording::ArmingClaim::ModelScrubbed,
            )
        });
        p.set_contribution_override(mode, at, grant).unwrap()
    }

    /// Owner decision on #1208: an "Automatic" override is a grant,
    /// so it fails closed exactly as per-folder arming does. Without terms
    /// to be a grant of, it is refused and nothing changes.
    #[test]
    fn an_auto_override_without_terms_is_refused() {
        let mut p = one_folder_per_mode();
        let before = p.clone();
        let err = p
            .set_contribution_override(ProjectMode::AutoUpload, t("2026-10-02T00:00:00Z"), None)
            .expect_err("no terms, no grant");
        assert_eq!(err.to_string(), "arming-terms-unavailable");
        assert_eq!(p, before, "a refusal changes nothing");
        // A stopping override needs no terms: it grants nothing.
        assert!(
            p.set_contribution_override(ProjectMode::NotifyOnly, t("2026-10-02T00:00:00Z"), None)
                .unwrap()
        );
    }

    /// R6 reaches the override: widened terms clear it, every folder is
    /// back on its own mode -- so none resolves to `AutoUpload` because of
    /// the override -- and the contributor is told, as for a folder.
    #[test]
    fn widening_the_terms_voids_an_auto_override() {
        let mut p = one_folder_per_mode();
        let at = t("2026-10-02T00:00:00Z");
        assert!(p.record_grant_terms("/w/auto", grant_terms_with("https://ingest.invalid")));
        override_to(&mut p, ProjectMode::AutoUpload, at);
        assert_eq!(p.resolve("/w/ask"), ProjectMode::AutoUpload);

        let sweep = p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), at);
        assert!(sweep.changed(), "the void is saved");
        assert_eq!(
            sweep.contribution_override_voided,
            Some(vec!["destination-changed"])
        );
        assert!(
            p.contribution_override.is_none(),
            "the pill is back to the roll-up"
        );
        for folder in ["/w/ask", "/w/unseen"] {
            assert_ne!(p.resolve(folder), ProjectMode::AutoUpload, "{folder}");
            assert_eq!(p.armed_from_now_at(folder), None, "{folder}");
        }
        assert!(p.armings_from_now().is_empty());
        // The folder armed by its own mode voids as it always did.
        assert_eq!(p.resolve("/w/auto"), ProjectMode::NotifyOnly);
        let override_voids: Vec<&GrantVoidNotice> = p
            .grant_voids
            .iter()
            .filter(|n| n.contribution_override)
            .collect();
        assert_eq!(override_voids.len(), 1, "{:?}", p.grant_voids);
        assert_eq!(override_voids[0].project_key, None);
        assert_eq!(override_voids[0].reasons, vec!["destination-changed"]);
        // And the folder's own void notice is a separate one.
        assert!(
            p.grant_voids
                .iter()
                .any(|n| n.project_key.as_deref() == Some("/w/auto"))
        );
    }

    /// Unchanged terms leave the override exactly as it was.
    #[test]
    fn an_auto_override_survives_a_sweep_under_unchanged_terms() {
        let mut p = one_folder_per_mode();
        let at = t("2026-10-02T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        let before = p.contribution_override.clone();
        let sweep = p.sweep_grants(&grant_terms_with("https://ingest.invalid"), at);
        assert_eq!(sweep.contribution_override_voided, None);
        assert_eq!(p.contribution_override, before);
        assert_eq!(p.resolve("/w/ask"), ProjectMode::AutoUpload);
        assert!(p.grant_voids.iter().all(|n| !n.contribution_override));
    }

    /// An override recorded with no terms (a policy file written before
    /// they were recorded) is voided rather than baselined: no such file
    /// ever shipped, so there is no legacy to protect, and failing closed
    /// asks the contributor again.
    #[test]
    fn an_auto_override_with_no_recorded_terms_is_voided() {
        let mut p = one_folder_per_mode();
        let at = t("2026-10-02T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        p.contribution_override.as_mut().unwrap().granted_under = None;
        let sweep = p.sweep_grants(&grant_terms_with("https://ingest.invalid"), at);
        assert_eq!(
            sweep.contribution_override_voided,
            Some(vec![OVERRIDE_TERMS_UNRECORDED])
        );
        assert!(p.contribution_override.is_none());
    }

    /// The override's void notice and the Flow 1 grant's are told apart:
    /// neither evicts the other, giving the grant again answers only its
    /// own, and setting the override again answers only the override's.
    #[test]
    fn the_override_void_notice_is_its_own() {
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-02T00:00:00Z");
        p.grant_automatic(at, grant_terms_with("https://ingest.invalid"));
        override_to(&mut p, ProjectMode::AutoUpload, at);
        p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), at);
        assert_eq!(p.grant_voids.len(), 2, "{:?}", p.grant_voids);
        p.grant_automatic(at, grant_terms_with("https://elsewhere.invalid"));
        assert_eq!(p.grant_voids.len(), 1);
        assert!(p.grant_voids[0].contribution_override);
        p.set_contribution_override(
            ProjectMode::AutoUpload,
            at,
            Some((
                grant_terms_with("https://elsewhere.invalid"),
                super::super::arming_wording::ArmingClaim::ModelScrubbed,
            )),
        )
        .unwrap();
        assert!(p.grant_voids.is_empty(), "setting it again answers it");
    }

    /// #1208: an `auto_upload` roll-up does not overstate itself. Under an
    /// "Automatic" override, a Never folder or the unknown bucket does
    /// not upload, and `contribution_mode_partial` says so; with neither,
    /// or with any other roll-up, it is false.
    #[test]
    fn an_auto_roll_up_says_when_some_folders_do_not_upload() {
        use ProjectMode::*;
        let at = t("2026-10-02T00:00:00Z");
        let mut p = ProjectPolicy::new();
        p.set_mode("/w/a", NotifyOnly, at).unwrap();
        override_to(&mut p, AutoUpload, at);
        assert_eq!(p.contribution_mode(["/w/b"]), Some(AutoUpload));
        assert!(!p.contribution_mode_partial(["/w/b"]), "every folder goes");
        assert!(p.contribution_mode_partial([UNKNOWN_PROJECT_KEY]));
        p.set_mode("/w/never", Ignore, at).unwrap();
        assert_eq!(p.contribution_mode([]), Some(AutoUpload), "still auto");
        assert!(p.contribution_mode_partial([]), "except Never");
        override_to(&mut p, NotifyOnly, at);
        assert!(!p.contribution_mode_partial([UNKNOWN_PROJECT_KEY]));
    }

    /// K5 reaches the override too: armed under words that claimed a model
    /// scrubs, its recorded claim moves once when the words in force narrow,
    /// and it stays on. No notice is raised.
    #[test]
    fn an_auto_override_has_its_claim_moved_when_its_words_narrow() {
        use super::super::arming_wording::ArmingClaim;
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-02T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        assert!(!p.sweep_override_claim(ArmingClaim::ModelScrubbed, at));
        assert!(p.sweep_override_claim(ArmingClaim::PatternsOnly, at));
        assert!(p.arming_rewordings.is_empty());
        assert!(
            !p.sweep_override_claim(ArmingClaim::PatternsOnly, at),
            "moved once"
        );
        assert_eq!(
            p.contribution_override.as_ref().map(|o| o.mode),
            Some(ProjectMode::AutoUpload),
            "a rewording leaves it on"
        );
        p.clear_contribution_override();
        assert!(p.arming_rewordings.is_empty());
    }

    /// Owner decision 2026-10-16: narrowing a folder's claim raises no
    /// notice, but the recorded claim still moves to the one in force.
    #[test]
    fn narrowing_a_folder_claim_raises_no_notice_but_records_it() {
        let mut p = armed_by_hand(&["/w/api"]);
        let at = t("2026-10-16T00:00:00Z");
        assert_eq!(p.arming_claim("/w/api"), ArmingClaim::ModelScrubbed);
        let reworded = p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at);
        assert_eq!(reworded, vec!["api".to_string()]);
        assert!(p.arming_rewordings.is_empty(), "{:?}", p.arming_rewordings);
        assert_eq!(p.arming_claim("/w/api"), ArmingClaim::PatternsOnly);
        assert_eq!(p.resolve("/w/api"), ProjectMode::AutoUpload, "still armed");
    }

    /// The same for the contribution override.
    #[test]
    fn narrowing_the_override_claim_raises_no_notice_but_records_it() {
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-16T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        assert!(p.sweep_override_claim(ArmingClaim::PatternsOnly, at));
        assert!(p.arming_rewordings.is_empty(), "{:?}", p.arming_rewordings);
        assert_eq!(
            p.contribution_override.as_ref().and_then(|o| o.claim),
            Some(ArmingClaim::PatternsOnly)
        );
        assert!(!p.sweep_override_claim(ArmingClaim::PatternsOnly, at));
    }

    /// The Scrub-check upgrade is still recorded once, with no notice.
    #[test]
    fn the_scrub_check_upgrade_raises_no_notice_but_is_recorded() {
        let mut p = armed_by_hand(&["/w/api"]);
        p.scrub_check_upgrade_recorded = false;
        assert!(p.record_scrub_check_upgrade(true, t("2026-10-16T00:00:00Z")));
        assert!(p.arming_rewordings.is_empty(), "{:?}", p.arming_rewordings);
        assert!(p.scrub_check_upgrade_recorded);
        assert!(!p.record_scrub_check_upgrade(true, t("2026-10-16T00:00:00Z")));
    }

    /// A notice persisted by an older build is dropped on load, so a stale
    /// one is never reported.
    #[test]
    fn a_persisted_rewording_notice_is_cleared_on_load() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let mut p = armed_by_hand(&["/w/api"]);
        p.arming_rewordings.push(ArmingRewordNotice {
            id: 7,
            reworded_at: t("2026-10-01T00:00:00Z"),
            project_key: "/w/api".to_string(),
            was: ArmingClaim::ModelScrubbed,
            now: ArmingClaim::PatternsOnly,
            scrub_check_defaulted: false,
        });
        p.save(&store).unwrap();
        let loaded = ProjectPolicy::load(&store).unwrap();
        assert!(loaded.arming_rewordings.is_empty());
        loaded.save(&store).unwrap();
        let body = store
            .read_daemon_file(DAEMON_PROJECTS_FILE)
            .unwrap()
            .unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["arming_rewordings"], serde_json::json!([]));
    }
}
