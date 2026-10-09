//! Which projects may upload without asking.
//!
//! Autonomy is per-project and opt-in. An unknown project is `NotifyOnly`, so
//! a freshly installed daemon uploads nothing until the contributor has
//! deliberately said otherwise about a specific project.
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
pub const DAEMON_PROJECTS_SCHEMA: &str = "trace_commons.daemon_projects.v2";

/// The pre-normalization schema. Recognized only so `load` can migrate it.
pub const DAEMON_PROJECTS_SCHEMA_V1: &str = "trace_commons.daemon_projects.v1";

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
    #[serde(default)]
    pub arming_rewordings: Vec<ArmingRewordNotice>,
    /// The id the next [`ArmingRewordNotice`] gets. Never reused.
    #[serde(default)]
    pub next_arming_reword_id: u64,
    /// Recorded atomically with upgrade notices, never cleared by their ack.
    #[serde(default)]
    pub scrub_check_upgrade_recorded: bool,
    /// Projects the contributor armed **from now** (`set_project_mode`
    /// `auto_upload` with `from_now: true`), by project key. See
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
/// - `NotifyOnly` ("Ask me"): every folder resolves to `NotifyOnly`, except
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

/// K5: a project armed **from now**: "Share automatically" in Customize, where
/// the backlog goes only when the contributor picks it.
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

/// The Flow 1 grant: "contribute automatically from projects discovered from
/// now on".
///
/// **It arms nothing already on disk** (the spec's logout rule), recorded
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
/// counted as new.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomaticGrant {
    pub granted_at: DateTime<Utc>,
    /// The terms the grant was given under. A widening voids the grant
    /// itself as well as the projects it armed (R6), or it would go on arming
    /// new projects under terms nobody agreed to.
    pub granted_under: super::grant_terms::GrantTerms,
    /// Sources recorded so far, by name and root.
    #[serde(default)]
    pub recorded_sources: BTreeSet<String>,
    /// Projects with a session on disk in a recorded source.
    #[serde(default)]
    pub projects_on_disk: BTreeSet<String>,
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
    /// `discovered` (the queue's), by its own mode ([`Self::folder_mode`]),
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
        (project_key != UNKNOWN_PROJECT_KEY).then(|| self.folder_mode(project_key))
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

    /// Give the Flow 1 grant, replacing any earlier one. Arms nothing until
    /// a source has been recorded, and then nothing from that source's
    /// recording pass.
    pub fn grant_automatic(&mut self, now: DateTime<Utc>, terms: super::grant_terms::GrantTerms) {
        // Giving the grant again answers a notice that it had stopped, and
        // only that one: a project's, or the contribution override's, is
        // about something else.
        self.grant_voids
            .retain(|n| n.project_key.is_some() || n.contribution_override);
        self.automatic_grant = Some(AutomaticGrant {
            granted_at: now,
            granted_under: terms,
            recorded_sources: BTreeSet::new(),
            projects_on_disk: BTreeSet::new(),
        });
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
    /// disk when the grant recorded it.
    pub fn arms_by_default(&self, project_key: &str, session_path: &str, source_key: &str) -> bool {
        let Some(grant) = self.automatic_grant.as_ref() else {
            return false;
        };
        grant.recorded_sources.contains(source_key)
            && project_key != UNKNOWN_PROJECT_KEY
            && !self.projects.contains_key(project_key)
            && !grant.projects_on_disk.contains(project_key)
            && !self.sessions_on_disk_at_grant.contains(session_path)
    }

    /// Whether the session at `session_path` must not be approved unattended
    /// in `project_key`: the grant armed the project, and the session was on
    /// disk at a grant. It waits for the contributor instead.
    pub fn holds_back_unattended(&self, project_key: &str, session_path: &str) -> bool {
        self.armed_by_grant.contains(project_key)
            && self.sessions_on_disk_at_grant.contains(session_path)
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
    /// already), can be armed at all, and has not been declined inside
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
                    // Ask me folder nor offer to arm an armed one.
                    && self.folder_mode(key) == ProjectMode::NotifyOnly
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
        let renamed = |key: &str| -> Option<String> {
            if key == UNKNOWN_PROJECT_KEY {
                return Some(UNKNOWN_PROJECT_KEY.to_string());
            }
            crate::daemon::project_key::normalize_project_key(key)
        };

        let mut projects: BTreeMap<String, ProjectEntry> = BTreeMap::new();
        for (key, entry) in std::mem::take(&mut self.projects) {
            let Some(fresh) = renamed(&key) else { continue };
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

        self.schema_version = DAEMON_PROJECTS_SCHEMA.to_string();
    }

    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_PROJECTS_FILE)? else {
            return Ok(Self::new());
        };
        let mut policy: Self =
            serde_json::from_slice(&body).context("parsing daemon project policy")?;
        // Migrate in memory on every load rather than rewriting the file
        // here: `load` has no business writing, and the next `save` -- which
        // every mutation already performs -- persists the v2 form. A file
        // that is never mutated again is migrated identically on every read,
        // which costs one normalization pass and is always correct.
        if policy.schema_version != DAEMON_PROJECTS_SCHEMA {
            policy.rekey();
        }
        Ok(policy)
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
        let own = self.folder_mode(project_key);
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

    /// K5 for the "Automatic" contribution override: leave a notice
    /// when the words in force (`in_force`, what the arming offer would say
    /// now) no longer claim what the override was set under. It stays on;
    /// the recorded claim moves to the one in force in the same save, so a
    /// rewording is announced once. The notice is keyed by
    /// [`OVERRIDE_ARMING_KEY`]. Returns whether one was left.
    pub fn sweep_override_claim(
        &mut self,
        in_force: super::arming_wording::ArmingClaim,
        now: DateTime<Utc>,
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
        o.claim = Some(in_force);
        self.arming_rewordings
            .retain(|n| n.project_key != OVERRIDE_ARMING_KEY || n.scrub_check_defaulted);
        let id = self.next_arming_reword_id;
        self.next_arming_reword_id = id.saturating_add(1);
        self.arming_rewordings.push(ArmingRewordNotice {
            id,
            reworded_at: now,
            project_key: OVERRIDE_ARMING_KEY.to_string(),
            was,
            now: in_force,
            scrub_check_defaulted: false,
        });
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

    /// K5: leave a notice for every armed project whose words in force
    /// (`current`, per project key) no longer claim what it was armed under.
    ///
    /// The project stays armed. Its recorded claim moves to the one in force
    /// in the same save as the notice, so a rewording is announced exactly
    /// once and a later narrowing is announced again. Only a narrowing is
    /// announced (`ArmingClaim::narrowed_to`); a widening is not recorded,
    /// so what the contributor was told stays the baseline. Returns the
    /// labels of the projects reworded, for the audit.
    pub fn sweep_arming_claims(
        &mut self,
        current: impl Fn(&str) -> super::arming_wording::ArmingClaim,
        now: DateTime<Utc>,
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
        for (key, label, was, in_force) in reworded {
            self.arming_claims.insert(key.clone(), in_force);
            self.arming_rewordings
                .retain(|n| n.project_key != key || n.scrub_check_defaulted);
            let id = self.next_arming_reword_id;
            self.next_arming_reword_id = id.saturating_add(1);
            self.arming_rewordings.push(ArmingRewordNotice {
                id,
                reworded_at: now,
                project_key: key,
                was,
                now: in_force,
                scrub_check_defaulted: false,
            });
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

    /// Announce changed hold behavior only for folders armed at upgrade.
    /// The marker and notices are saved together by startup, before work.
    pub fn record_scrub_check_upgrade(&mut self, automatic: bool, now: DateTime<Utc>) -> bool {
        if self.scrub_check_upgrade_recorded {
            return false;
        }
        if automatic {
            for (key, entry) in &self.projects {
                if entry.mode != ProjectMode::AutoUpload || key == UNKNOWN_PROJECT_KEY {
                    continue;
                }
                let claim = self.arming_claim(key);
                let id = self.next_arming_reword_id;
                self.next_arming_reword_id = id.saturating_add(1);
                self.arming_rewordings.push(ArmingRewordNotice {
                    id,
                    reworded_at: now,
                    project_key: key.clone(),
                    was: claim,
                    now: claim,
                    scrub_check_defaulted: true,
                });
            }
        }
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
        // is itself from now. A plain `auto_upload` therefore releases the
        // backlog, which is what that call has always meant.
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
        self.projects.insert(
            project_key.to_string(),
            ProjectEntry {
                mode,
                added_at: now,
                label: project_label_for(shown.as_deref().unwrap_or(project_key)),
                display_path: shown,
                // A fresh entry on every mode change: leaving automatic
                // clears the terms, and re-arming records new ones.
                armed_under: None,
                // And the redaction record: it covers one arming only.
                automatic_redaction: AutomaticRedactionTally::default(),
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
    match std::fs::canonicalize(path) {
        Ok(resolved) => resolved.is_dir() && resolved.as_os_str() == path.as_os_str(),
        Err(_) => false,
    }
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
    let digest = Sha256::digest(project_key.as_bytes());
    format!(
        "{PROJECT_ID_PREFIX}{}",
        hex_prefix(&digest, PROJECT_ID_HEX_CHARS)
    )
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
    let label = project_label_for(project_path.unwrap_or(project_key));
    if project_key == UNKNOWN_PROJECT_KEY {
        return label;
    }

    let folded = project_label_for(project_key);
    let collides = known_keys
        .iter()
        .any(|other| other != project_key && project_label_for(other) == folded);
    if !collides {
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
    fn scrub_check_upgrade_is_once_only_and_keeps_other_notices() {
        let now = t("2026-09-30T00:00:00Z");
        let mut p = armed_by_hand(&["/w/api"]);
        p.scrub_check_upgrade_recorded = false;
        assert!(p.record_scrub_check_upgrade(true, now));
        assert_eq!(p.arming_rewordings.len(), 1);
        let upgrade_id = p.arming_rewordings[0].id;
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert_eq!(p.arming_rewordings.len(), 2);
        assert!(
            p.arming_rewordings
                .iter()
                .any(|n| n.id == upgrade_id && n.scrub_check_defaulted)
        );
        p.acknowledge_arming_rewordings(&[upgrade_id]);
        assert_eq!(p.arming_rewordings.len(), 1);
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
    /// wording in force is patterns-only, gets a notice and stays armed.
    #[test]
    fn a_folder_armed_under_the_scrub_wording_is_told_when_it_is_reworded() {
        let mut p = armed_by_hand(&["/w/api"]);
        let at = t("2026-09-27T01:00:00Z");

        let unchanged = p.sweep_arming_claims(|_| ArmingClaim::ModelScrubbed, at);
        assert!(unchanged.is_empty());
        assert!(
            p.arming_rewordings.is_empty(),
            "nothing reworded, nothing to say"
        );

        let reworded = p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at);
        assert_eq!(reworded, vec!["api".to_string()]);
        assert_eq!(p.arming_rewordings.len(), 1, "{:?}", p.arming_rewordings);
        let n = &p.arming_rewordings[0];
        assert_eq!(n.project_key, "/w/api");
        assert_eq!(n.was, ArmingClaim::ModelScrubbed);
        assert_eq!(n.now, ArmingClaim::PatternsOnly);
        assert_eq!(n.reworded_at, at);
        assert_eq!(p.resolve("/w/api"), ProjectMode::AutoUpload, "still armed");
    }

    /// Announced once: the next pass under the same wording adds nothing,
    /// and an acknowledged notice does not come back.
    #[test]
    fn a_rewording_is_announced_once() {
        let mut p = armed_by_hand(&["/w/api"]);
        let at = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at);
        assert_eq!(p.arming_rewordings.len(), 1);
        let id = p.arming_rewordings[0].id;
        assert!(
            p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, at)
                .is_empty()
        );
        assert_eq!(p.arming_rewordings.len(), 1);
        assert_eq!(p.acknowledge_arming_rewordings(&[id, 42]), 1);
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

    /// Setting the project's mode answers its notice, whether it is armed
    /// again (under words recorded afresh) or switched to ask-first.
    #[test]
    fn setting_the_mode_answers_a_rewording() {
        let mut p = armed_by_hand(&["/w/api", "/w/web"]);
        let now = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert_eq!(p.arming_rewordings.len(), 2);
        p.set_mode("/w/api", ProjectMode::NotifyOnly, now).unwrap();
        assert_eq!(p.arming_rewordings.len(), 1);
        assert_eq!(p.arming_rewordings[0].project_key, "/w/web");
        p.set_mode("/w/web", ProjectMode::AutoUpload, now).unwrap();
        assert!(p.arming_rewordings.is_empty());
    }

    /// A void returns the folder to ask-first and says so; a rewording
    /// notice saying it is still armed would then be false.
    #[test]
    fn a_void_takes_the_rewording_notice_with_it() {
        let mut p = armed_and_granted("https://ingest.invalid");
        let now = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert_eq!(p.arming_rewordings.len(), 1);
        p.sweep_grants(&grant_terms_with("https://elsewhere.invalid"), now);
        assert!(p.arming_rewordings.is_empty());
        assert!(!p.arming_claims.contains_key("/w/api"));
    }

    /// Ids are never reused across notices.
    #[test]
    fn a_later_rewording_never_reuses_an_acknowledged_id() {
        let mut p = armed_by_hand(&["/w/api"]);
        let now = t("2026-09-27T01:00:00Z");
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert_eq!(p.arming_rewordings.len(), 1);
        let first = p.arming_rewordings[0].id;
        p.acknowledge_arming_rewordings(&[first]);
        p.set_mode("/w/api", ProjectMode::AutoUpload, now).unwrap();
        p.record_arming_claim("/w/api", ArmingClaim::ModelScrubbed);
        p.sweep_arming_claims(|_| ArmingClaim::PatternsOnly, now);
        assert_eq!(p.arming_rewordings.len(), 1);
        assert_ne!(p.arming_rewordings[0].id, first);
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
            },
        );
        p.save(&store).unwrap();

        let loaded = ProjectPolicy::load(&store).unwrap();
        let key = project_key_for(Some(root.to_str().unwrap()));
        assert_eq!(loaded.resolve(&key), ProjectMode::Ignore);
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

    /// "Ask me" and "Never" leave nothing to go unattended, in any folder,
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
    /// scrubs, it is told once when the words in force narrow, and stays on.
    #[test]
    fn an_auto_override_is_told_when_its_words_narrow() {
        use super::super::arming_wording::ArmingClaim;
        let mut p = ProjectPolicy::new();
        let at = t("2026-10-02T00:00:00Z");
        override_to(&mut p, ProjectMode::AutoUpload, at);
        assert!(!p.sweep_override_claim(ArmingClaim::ModelScrubbed, at));
        assert!(p.arming_rewordings.is_empty());
        assert!(p.sweep_override_claim(ArmingClaim::PatternsOnly, at));
        assert_eq!(p.arming_rewordings.len(), 1);
        assert_eq!(p.arming_rewordings[0].project_key, OVERRIDE_ARMING_KEY);
        assert_eq!(p.arming_rewordings[0].was, ArmingClaim::ModelScrubbed);
        assert!(
            !p.sweep_override_claim(ArmingClaim::PatternsOnly, at),
            "told once"
        );
        assert_eq!(
            p.contribution_override.as_ref().map(|o| o.mode),
            Some(ProjectMode::AutoUpload),
            "a rewording leaves it on"
        );
        // Clearing it answers the notice.
        p.clear_contribution_override();
        assert!(p.arming_rewordings.is_empty());
    }
}
