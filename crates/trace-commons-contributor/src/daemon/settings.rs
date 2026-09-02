//! Daemon configuration: the knobs governing how patient, how chatty, and how
//! autonomous the background uploader is.
//!
//! These are persisted rather than read from the process environment because a
//! daemon started by a service manager inherits none of the user's shell
//! environment. Settings read from env would leave every upload refusing with
//! `pii-filter-unavailable` under systemd while working perfectly by hand.

use std::path::PathBuf;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::config::{ConfigStore, DAEMON_SETTINGS_FILE};
use crate::envelope::NearAiSettings;

pub const DAEMON_SETTINGS_SCHEMA: &str = "trace_commons.daemon_settings.v1";

/// How long a session must go unwritten before it counts as finished.
const DEFAULT_QUIESCENCE_SECS: u64 = 1800;
/// How often the watcher stats the session roots. Much finer than the
/// quiescence window, so the poll rate costs nothing in responsiveness.
const DEFAULT_POLL_INTERVAL_SECS: u64 = 60;
/// Minimum gap between digest notifications, so a busy day is one interruption
/// rather than a dozen.
const DEFAULT_DIGEST_INTERVAL_SECS: u64 = 14_400;
const DEFAULT_QUEUE_TTL_DAYS: i64 = 14;
/// A resumed session must grow by this factor to be worth re-uploading.
const DEFAULT_GROWTH_FACTOR: f64 = 2.0;
/// ...or by this many absolute bytes, which is what actually catches growth on
/// an already-large session.
const DEFAULT_GROWTH_MIN_NEW_BYTES: u64 = 65_536;
/// A session re-uploads at most this many times. Each re-upload re-sends the
/// whole file, so an unbounded count would pay the privacy-filter bill
/// repeatedly over the same text and dilute the contributor's own credit
/// through server-side duplicate clustering.
const DEFAULT_MAX_REUPLOADS: u32 = 3;
const DEFAULT_MAX_UPLOADS_PER_DAY: u32 = 50;
const DEFAULT_MAX_BYTES_PER_DAY: u64 = 209_715_200;
/// The upper bound `set_settings` (and the C ABI's pre-start override) will
/// accept for `max_uploads_per_day`. The cap exists to bound a runaway
/// client -- an app that decides to upload everything should not be able
/// to -- so it must stay a validated ceiling, not an open field. 20x the
/// default comfortably covers a contributor running several agents across
/// a very active day (a real machine needed 200/day, four times the
/// default) while still being a real bound: a client that hit this would
/// still be stopped well short of "everything, all day".
const MAX_UPLOADS_PER_DAY_CEILING: u32 = 1_000;
/// The upper bound for `max_bytes_per_day`, in bytes. Sized from real
/// corpus data, not a guess: a machine with 81 Claude sessions (0.9 GB
/// total, largest 93.6 MB) and 3,069 Codex sessions (10.8 GB total) still
/// only produces a few GB of *accepted* envelopes on its most active day,
/// since a single accepted envelope commonly runs several MB. 5 GiB is
/// comfortably above that (and above the 2 GiB a contributor had already
/// raised their own machine to by hand) while remaining a real ceiling on
/// a client that decided to send everything at once.
const MAX_BYTES_PER_DAY_CEILING: u64 = 5 * 1024 * 1024 * 1024;
const DEFAULT_MAX_QUEUE_ENTRIES: usize = 500;
const DEFAULT_HISTORY_POLL_SECS: u64 = 1800;
/// How often the public community roster is fetched. The server serves a
/// pre-rendered snapshot and refuses to serve one older than fifteen minutes,
/// so polling faster than that only re-fetches a body the server has not
/// recomputed. Fifteen minutes is the roster's own cadence, and this follows
/// it rather than inventing a second one.
const DEFAULT_COMMUNITY_POLL_SECS: u64 = 900;
/// A privacy-filter self-test from days ago proves nothing about the filter
/// now, so a long-lived process re-checks on this interval.
const DEFAULT_CANARY_INTERVAL_SECS: u64 = 3600;
/// How long an approval is held before the uploader will touch it, which is
/// how long a contributor's "Undo" really lasts.
///
/// The designed affordance is a five-second undo after approving. Five
/// seconds is therefore the floor, not the target: the client's countdown
/// starts when it renders the response, which is already after the approval
/// was stamped, and the cancel that ends it has to travel back over the
/// socket. Ten leaves room for both, plus the second or two of clock skew
/// between an application counting in its own process and a daemon deciding
/// in another, and costs nothing that matters -- uploads are unattended
/// background work on a 60-second poll, so an armed project's traces still
/// go out on the very next tick.
///
/// Zero disables the hold, restoring the old behaviour for anyone who wants
/// it; a client is expected to stop offering an undo when
/// `approve` reports no `hold_until`.
const DEFAULT_APPROVAL_HOLD_SECS: u64 = 10;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DaemonSettings {
    pub schema_version: String,
    pub poll_interval_secs: u64,
    pub quiescence_secs: u64,
    pub digest_interval_secs: u64,
    pub queue_ttl_days: i64,
    pub growth_factor: f64,
    pub growth_min_new_bytes: u64,
    pub max_reuploads: u32,
    pub max_uploads_per_day: u32,
    pub max_bytes_per_day: u64,
    pub max_queue_entries: usize,
    pub history_poll_secs: u64,
    /// How often the public community roster is fetched. `#[serde(default)]`
    /// so a settings file written before this field existed loads with the
    /// poll on its published cadence rather than failing to parse.
    #[serde(default = "default_community_poll_secs")]
    pub community_poll_secs: u64,
    pub canary_interval_secs: u64,
    /// How long after an approval the uploader must leave the entry alone,
    /// so the undo a client offers is real rather than a race against the
    /// next upload pass. See `DEFAULT_APPROVAL_HOLD_SECS` and
    /// `queue::QueueEntry::approved_at`.
    ///
    /// `#[serde(default = ...)]` so a settings file written before this
    /// field existed loads with the hold on rather than off: a missing key
    /// must not silently mean "no undo window".
    #[serde(default = "default_approval_hold_secs")]
    pub approval_hold_secs: u64,
    /// Whether the daemon itself renders OS notifications. Off by default:
    /// the native applications render their own, and the daemon's shell-out
    /// path needs a desktop session it may not have.
    pub local_notifications: bool,
    /// Privacy-filter credentials, persisted so a service-managed daemon can
    /// reach the filter without a shell environment.
    pub near_ai: Option<NearAiSettings>,
    /// What the contributor said about each agent's sessions.
    ///
    /// `None` is "never asked", and it is the ONLY state that still falls
    /// back to the conventional per-user location -- which is why the
    /// application shells refuse to start on it. `Some(Off)` is a real
    /// answer and is never a fallback; see [`SourceDeclaration`].
    #[serde(default)]
    pub claude_source: Option<SourceDeclaration>,
    #[serde(default)]
    pub codex_source: Option<SourceDeclaration>,
    /// Added after every desktop client had shipped, which is why an absent
    /// value here means "no gemini adapter" rather than "the conventional
    /// `~/.gemini`" -- see [`crate::source::SourceRoots`] and
    /// [`roots_declared`].
    #[serde(default)]
    pub gemini_source: Option<SourceDeclaration>,

    /// A local inference proxy, when the contributor declared one. Absent
    /// means off: see [`IronWireDeclaration`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ironwire: Option<IronWireDeclaration>,

    /// Legacy spellings, read on load and never written.
    ///
    /// Settings files written before source declarations existed carry
    /// `claude_root` / `codex_root` strings meaning "watch this path".
    /// [`DaemonSettings::load`] folds them into the fields above so an
    /// install that already declared its roots is not asked again. They are
    /// `skip_serializing` so a file rewritten by this version stops carrying
    /// two spellings of the same fact.
    /// Public only because `DaemonSettings { .. }` literals live in other
    /// crates; treat it as private. Read by `load` and never written.
    #[serde(default, rename = "claude_root", skip_serializing)]
    pub legacy_claude_root: Option<PathBuf>,
    /// See `legacy_claude_root`.
    #[serde(default, rename = "codex_root", skip_serializing)]
    pub legacy_codex_root: Option<PathBuf>,
}

/// What the contributor said about one agent's session store.
///
/// The tri-state this replaces was `Option<PathBuf>`, where `None` had to
/// carry both "never asked" and "I don't use this agent" -- and the daemon
/// resolved that ambiguity by watching the real `~/.claude` or `~/.codex`
/// (`crate::source::all_sources`). So the one answer a privacy-conscious
/// contributor is most likely to give was the one answer that silently
/// scanned their work.
///
/// Serialized tagged rather than as a bare string, so `off` can never be
/// mistaken for a path and a future third state has somewhere to go:
///
/// ```json
/// "claude_source": { "mode": "watch", "path": "/Users/x/.claude/projects" }
/// "codex_source":  { "mode": "off" }
/// ```
///
/// Deliberately NOT a sentinel path (an empty directory, a temp dir, "/dev/null").
/// A sentinel that is a real filesystem location is a lie every later reader
/// has to decode, and it stops being true the moment somebody creates that
/// directory.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum SourceDeclaration {
    /// Watch this directory. The contributor chose it.
    Watch { path: PathBuf },
    /// The contributor said they do not use this agent. Nothing is watched
    /// for it, and there is no fallback.
    Off,
}

impl SourceDeclaration {
    /// The directory to watch, or `None` when the source is off.
    pub fn path(&self) -> Option<&std::path::Path> {
        match self {
            SourceDeclaration::Watch { path } => Some(path.as_path()),
            SourceDeclaration::Off => None,
        }
    }
}

/// What the contributor said about a local inference proxy.
///
/// Deliberately NOT the same tri-state semantics as [`SourceDeclaration`].
/// There, `None` means "never asked" and falls back to the conventional
/// per-user location. Here `None` means **off**, with no fallback.
///
/// A session root has a conventional location to fall back to. A local service
/// does not: connecting to `127.0.0.1:8463` because nobody said otherwise is a
/// probe of a service the contributor never mentioned, which is exactly the
/// error the source tri-state was introduced to stop making about their files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum IronWireDeclaration {
    /// Read the proxy's ledger on this loopback port.
    Watch { port: u16 },
    /// The contributor said they do not use it. Nothing is read.
    Off,
}

impl IronWireDeclaration {
    /// The port to read, or `None` when the proxy is off.
    #[must_use]
    pub fn port(&self) -> Option<u16> {
        match self {
            IronWireDeclaration::Watch { port } => Some(*port),
            IronWireDeclaration::Off => None,
        }
    }
}

/// Build a routing ledger for a declaration, or nothing.
///
/// The token is read from `$IRONWIRE_HOME/control.token` at build time and
/// never copied into our settings file. An unreadable token yields no reader:
/// absence and failure are the same state.
#[must_use]
pub fn ironwire_ledger_for(
    declaration: Option<&IronWireDeclaration>,
) -> Option<std::sync::Arc<crate::routing::ironwire::IronWireLedger>> {
    let port = declaration?.port()?;
    let home = std::env::var_os("IRONWIRE_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|h| h.join(".ironwire")))?;
    let token = std::fs::read_to_string(home.join("control.token")).ok()?;
    Some(std::sync::Arc::new(
        crate::routing::ironwire::IronWireLedger::new(port, token.trim().to_string()),
    ))
}

fn default_approval_hold_secs() -> u64 {
    DEFAULT_APPROVAL_HOLD_SECS
}

fn default_community_poll_secs() -> u64 {
    DEFAULT_COMMUNITY_POLL_SECS
}

impl Default for DaemonSettings {
    fn default() -> Self {
        Self {
            schema_version: DAEMON_SETTINGS_SCHEMA.to_string(),
            poll_interval_secs: DEFAULT_POLL_INTERVAL_SECS,
            quiescence_secs: DEFAULT_QUIESCENCE_SECS,
            digest_interval_secs: DEFAULT_DIGEST_INTERVAL_SECS,
            queue_ttl_days: DEFAULT_QUEUE_TTL_DAYS,
            growth_factor: DEFAULT_GROWTH_FACTOR,
            growth_min_new_bytes: DEFAULT_GROWTH_MIN_NEW_BYTES,
            max_reuploads: DEFAULT_MAX_REUPLOADS,
            max_uploads_per_day: DEFAULT_MAX_UPLOADS_PER_DAY,
            max_bytes_per_day: DEFAULT_MAX_BYTES_PER_DAY,
            max_queue_entries: DEFAULT_MAX_QUEUE_ENTRIES,
            history_poll_secs: DEFAULT_HISTORY_POLL_SECS,
            community_poll_secs: DEFAULT_COMMUNITY_POLL_SECS,
            canary_interval_secs: DEFAULT_CANARY_INTERVAL_SECS,
            approval_hold_secs: DEFAULT_APPROVAL_HOLD_SECS,
            local_notifications: false,
            near_ai: None,
            claude_source: None,
            codex_source: None,
            gemini_source: None,
            ironwire: None,
            legacy_claude_root: None,
            legacy_codex_root: None,
        }
    }
}

impl DaemonSettings {
    /// Load persisted settings, falling back to defaults when the daemon has
    /// never been configured on this machine.
    pub fn load(store: &ConfigStore) -> Result<Self> {
        let Some(body) = store.read_daemon_file(DAEMON_SETTINGS_FILE)? else {
            return Ok(Self::default());
        };
        // The serde context stays for local stderr and journals, where the
        // parser's own "missing field `schema_version` at line 1 column 65"
        // is the whole diagnosis. `StartFailure` rides alongside it so a
        // caller across the C ABI -- which must not receive that text, since
        // the file it names is in the contributor's home directory -- can
        // still tell this apart from every other start failure.
        let mut settings: Self = serde_json::from_slice(&body)
            .context("parsing daemon settings")
            .context(crate::daemon::StartFailure::SettingsUnreadable)?;
        settings.absorb_legacy_roots();
        Ok(settings)
    }

    /// Fold `claude_root` / `codex_root` from an older file into the source
    /// declarations. A legacy path means "watch this"; it never means off,
    /// because off could not be expressed before this existed.
    ///
    /// An explicit declaration always wins, so a file carrying both (written
    /// by a version in between, or edited by hand) is not downgraded.
    fn absorb_legacy_roots(&mut self) {
        if self.claude_source.is_none()
            && let Some(path) = self.legacy_claude_root.take()
        {
            self.claude_source = Some(SourceDeclaration::Watch { path });
        }
        if self.codex_source.is_none()
            && let Some(path) = self.legacy_codex_root.take()
        {
            self.codex_source = Some(SourceDeclaration::Watch { path });
        }
        self.legacy_claude_root = None;
        self.legacy_codex_root = None;
    }

    /// What the contributor declared, in the shape `crate::source::all_sources`
    /// takes.
    ///
    /// The named fields stay the serialised shape -- a `daemon-settings.json`
    /// written by any previous version parses unchanged -- and the map is
    /// built from them here, in one place, so adding an adapter does not
    /// touch the daemon, the watcher, the preview scheduler or the CLI.
    ///
    /// No WORKING-DIRECTORY trajectory scope: a daemon's working directory
    /// is whatever a service manager handed it, so auto-discovery would
    /// mean nothing there.
    ///
    /// The STAGING directory is a different thing and is included. It is a
    /// fixed path under the contributor's own state directory, resolved
    /// through `ConfigStore`, created 0700 and cleared by `logout`, holding
    /// only what `import-antigravity` put there on an explicit command.
    ///
    /// That distinction was previously collapsed: this method took neither,
    /// under one reason that covers only the first. The cost was that every
    /// imported conversation was invisible to all three desktop apps -- no
    /// entry, no error, no empty state naming it -- while the CLI, which
    /// builds its own roots, could see them the whole time.
    ///
    /// No routing overlay either, and deliberately not yet: settings
    /// describe the IronWire *declaration*, not the ledger *instance*.
    /// [`ironwire_ledger_for`] builds a fresh, cold `IronWireLedger` on every
    /// call, so wiring it in here would hand every caller its own
    /// never-refreshed snapshot -- the overlay would compile but never
    /// produce a row. The instance needs a single long-lived owner that
    /// refreshes it on a schedule, which is a separate, reviewed piece of
    /// work; see [`crate::source::SourceRoots::with_routing`].
    pub fn source_roots(&self, store: &ConfigStore) -> crate::source::SourceRoots {
        crate::source::SourceRoots::new()
            .declare(
                crate::source::SOURCE_CLAUDE_CODE,
                self.claude_source.clone(),
            )
            .declare(crate::source::SOURCE_CODEX, self.codex_source.clone())
            .declare(crate::source::SOURCE_GEMINI_CLI, self.gemini_source.clone())
            .with_trajectory(crate::source::TrajectorySelection::Auto {
                working_dir: None,
                staging_dir: Some(store.dir().join(crate::source::TRAJECTORY_STAGING_SUBDIR)),
            })
    }

    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let body = serde_json::to_vec_pretty(self).context("serializing daemon settings")?;
        store.write_daemon_file(DAEMON_SETTINGS_FILE, &body)
    }
}

/// A partial-settings object held nothing this function recognizes.
pub const ERR_SETTINGS_NOT_OBJECT: &str = "settings-not-object";
/// A partial-settings object had a top-level key this function does not
/// recognize.
pub const ERR_SETTINGS_UNKNOWN_FIELD: &str = "settings-unknown-field";
/// A recognized key held a value of the wrong JSON type (or, for
/// `claude_root`/`codex_root`, a JSON type other than string/null), or --
/// for `max_uploads_per_day`/`max_bytes_per_day` -- a value of the right
/// type but out of the accepted range (zero, or above the validated
/// ceiling; see `MAX_UPLOADS_PER_DAY_CEILING` and
/// `MAX_BYTES_PER_DAY_CEILING`). One fixed label covers both failure
/// shapes deliberately: the point of the label is that it never carries
/// the caller's value, and "wrong type" vs. "right type, wrong range" is
/// not a distinction a fail-closed caller needs to branch on.
pub const ERR_SETTINGS_INVALID_VALUE: &str = "settings-invalid-value";

/// The `daemon-settings.json` key carrying one adapter's declaration.
///
/// The mapping exists because the adapter names are wire names
/// (`gemini-cli`) and the settings keys are field names (`gemini_source`);
/// a shell that renders one candidate per discovered source needs to turn
/// the first into the second without transcribing a table of its own. An
/// unrecognised source answers `None` rather than inventing a key, so a
/// caller that has fallen behind this crate refuses rather than writing a
/// field `apply_settings_object` will reject.
pub fn source_settings_key(source: &str) -> Option<&'static str> {
    match source {
        crate::source::SOURCE_CLAUDE_CODE => Some("claude_source"),
        crate::source::SOURCE_CODEX => Some("codex_source"),
        crate::source::SOURCE_GEMINI_CLI => Some("gemini_source"),
        _ => None,
    }
}

/// Whether the contributor has said which session folders to watch.
///
/// BOTH, not either. `claude_root: None` does not mean "no Claude source" --
/// `DaemonSettings` documents it as meaning the conventional per-user
/// location, so an undeclared root is the real `~/.claude` or `~/.codex`.
/// Half a declaration therefore buys none of the protection while reading as
/// though it had, which is why an `||` here would be a fail-open.
///
/// This is the ONLY place the rule is written. The application shells consult
/// it -- macOS and Windows through the C ABI's start functions, the GTK shell
/// by calling it directly -- rather than each transcribing the predicate into
/// its own language, because three copies of a rule that decides whether a
/// developer's source tree gets scanned is three chances for one of them to
/// drift. Compare `tc_daemon_start_with_settings`, which shares
/// `set_settings`' validator for exactly this reason.
///
/// The daemon core does not consult it: `trace-commons-contributor daemon` is
/// someone typing a command on purpose, and the CLI keeps its defaults.
pub fn roots_declared(settings: &DaemonSettings) -> bool {
    settings.claude_source.is_some() && settings.codex_source.is_some()
}

/// Whether the contributor has been asked about their Gemini CLI sessions.
///
/// Deliberately NOT a third conjunct in `roots_declared`. That predicate
/// decides whether the daemon may start, and every desktop client already
/// installed declares claude and codex and has no gemini field: a third
/// conjunct would stop the daemon starting on every one of them. An absent
/// gemini declaration is not disqualifying because it is not dangerous --
/// it constructs no adapter and scans nothing (`crate::source::Undeclared`)
/// -- which is exactly the property the fail-closed-roots rule turns on.
///
/// So this is a question about what a shell should OFFER to ask, not a gate
/// on starting.
pub fn gemini_declared(settings: &DaemonSettings) -> bool {
    settings.gemini_source.is_some()
}

/// Apply a partial settings object -- the shape `tc_call(handle,
/// "set_settings", ...)` takes over the socket, and the shape
/// `tc_daemon_start_with_settings` takes over the C ABI before the daemon's
/// first supervisor tick -- onto `settings` in place. One function, so
/// both callers share one definition of "a valid settings object" rather
/// than two that can drift.
///
/// Every top-level key must be one this function recognizes; an
/// unrecognized key is rejected outright (`Err(ERR_SETTINGS_UNKNOWN_FIELD)`)
/// rather than silently ignored. Silently ignoring a misspelled
/// `claude_root` is exactly the bug this exists to prevent: a caller that
/// meant to redirect the watcher and typo'd the key would otherwise get no
/// signal at all, and the daemon would quietly go on scanning wherever it
/// was already pointed.
///
/// Every error is a fixed, content-free `&'static str` label. In
/// particular, a bad `claude_root`/`codex_root` value never appears in the
/// label -- only the recognized field name distinguishes one failure from
/// another, and the field *names* are a small, fixed, known set, never
/// caller-supplied text. The values themselves (which is where a
/// filesystem path lives) never cross into an error string.
///
/// Returns whether anything was applied. An empty object (or one holding
/// only keys whose values happen to match the current setting) still
/// reports `true` for any key present and accepted -- this always applies
/// every key it accepts, so `Ok(false)` only ever means "the object had no
/// keys at all". Callers that require at least one recognized field (as
/// `set_settings` does, to catch an empty or accidental call) check that
/// themselves; `tc_daemon_start_with_settings` does not, since "nothing to
/// override" is its documented no-op case.
///
/// `max_uploads_per_day` and `max_bytes_per_day` are validated against a
/// fixed ceiling (`MAX_UPLOADS_PER_DAY_CEILING`, `MAX_BYTES_PER_DAY_CEILING`)
/// rather than accepted as an open field: the cap exists to bound a runaway
/// client, and an unbounded setter would give that protection up entirely.
/// A value below the current default is accepted with no floor beyond
/// non-zero -- throttling one's own uploads is not a safety concern -- but
/// zero and anything above the ceiling are refused with the same
/// `ERR_SETTINGS_INVALID_VALUE` label the other typed fields use.
pub fn apply_settings_object(
    settings: &mut DaemonSettings,
    params: &serde_json::Value,
) -> std::result::Result<bool, &'static str> {
    let obj = params.as_object().ok_or(ERR_SETTINGS_NOT_OBJECT)?;
    let mut changed = false;
    for (key, value) in obj {
        match key.as_str() {
            "quiescence_secs" => {
                settings.quiescence_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "digest_interval_secs" => {
                settings.digest_interval_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "approval_hold_secs" => {
                settings.approval_hold_secs = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            "local_notifications" => {
                settings.local_notifications = value.as_bool().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
            }
            // Both caps take a validated ceiling, not an open field: the
            // cap exists to bound a runaway client, and a freely settable
            // value would give up exactly the protection it was added for.
            // A value below the default IS allowed -- a contributor
            // throttling their own uploads is a legitimate thing to want
            // and is not a safety concern -- but zero is refused rather
            // than accepted as "no uploads": that state already exists and
            // is spelled `pause`, which (unlike a cap of zero) is visibly
            // temporary and does not fight the health-label machinery that
            // treats a reached cap as a `CapReached`/health-label
            // condition on every single upload attempt.
            "max_uploads_per_day" => {
                settings.max_uploads_per_day = parse_max_uploads_per_day(value)?;
            }
            "max_bytes_per_day" => {
                settings.max_bytes_per_day = parse_max_bytes_per_day(value)?;
            }
            // The path spellings. A string declares "watch this"; null
            // clears the declaration back to never-asked, which is what the
            // application shells refuse to start on. Kept because the C ABI
            // documents these exact keys and both native shells send them.
            "claude_root" => {
                settings.claude_source = parse_optional_root(value)?;
            }
            "codex_root" => {
                settings.codex_source = parse_optional_root(value)?;
            }
            // The full declaration, including the one thing a path cannot
            // say: off.
            "claude_source" => {
                settings.claude_source = parse_source_declaration(value)?;
            }
            "codex_source" => {
                settings.codex_source = parse_source_declaration(value)?;
            }
            "gemini_source" => {
                settings.gemini_source = parse_source_declaration(value)?;
            }
            // Unlike the source roots above, `null` here means **off**, not
            // "never asked" -- see `IronWireDeclaration`'s doc comment for
            // why the tri-state does not apply to a local service with no
            // conventional fallback location.
            "ironwire" => {
                settings.ironwire = parse_ironwire_declaration(value)?;
            }
            _ => return Err(ERR_SETTINGS_UNKNOWN_FIELD),
        }
        changed = true;
    }
    Ok(changed)
}

/// `max_uploads_per_day`: a non-zero `u32` at most `MAX_UPLOADS_PER_DAY_CEILING`.
/// Zero is refused -- see the call site's doc for why a cap of zero is not
/// this method's way to stop uploads.
fn parse_max_uploads_per_day(value: &serde_json::Value) -> std::result::Result<u32, &'static str> {
    let n = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
    if n == 0 || n > MAX_UPLOADS_PER_DAY_CEILING as u64 {
        return Err(ERR_SETTINGS_INVALID_VALUE);
    }
    Ok(n as u32)
}

/// `max_bytes_per_day`: a non-zero `u64` at most `MAX_BYTES_PER_DAY_CEILING`.
fn parse_max_bytes_per_day(value: &serde_json::Value) -> std::result::Result<u64, &'static str> {
    let n = value.as_u64().ok_or(ERR_SETTINGS_INVALID_VALUE)?;
    if n == 0 || n > MAX_BYTES_PER_DAY_CEILING {
        return Err(ERR_SETTINGS_INVALID_VALUE);
    }
    Ok(n)
}

/// `null` clears the override (falls back to the conventional per-user
/// location); a string sets it; anything else is a type error. Never
/// formats `value` into the error -- see `apply_settings_object`'s doc.
fn parse_optional_root(
    value: &serde_json::Value,
) -> std::result::Result<Option<SourceDeclaration>, &'static str> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::String(s) => Ok(Some(SourceDeclaration::Watch {
            path: PathBuf::from(s),
        })),
        _ => Err(ERR_SETTINGS_INVALID_VALUE),
    }
}

/// `{"mode":"watch","port":8463}` or null to turn it off. `{"mode":"off"}`
/// is also accepted since it round-trips `IronWireDeclaration::Off`, but null
/// is the documented way to reach the same state over IPC. Never formats
/// `value` into the error -- see `apply_settings_object`'s doc.
fn parse_ironwire_declaration(
    value: &serde_json::Value,
) -> std::result::Result<Option<IronWireDeclaration>, &'static str> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Object(_) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| ERR_SETTINGS_INVALID_VALUE),
        _ => Err(ERR_SETTINGS_INVALID_VALUE),
    }
}

/// `{"mode":"watch","path":"..."}`, `{"mode":"off"}`, or null to clear the
/// declaration back to never-asked. Never formats `value` into the error --
/// see `apply_settings_object`'s doc; a declaration carries a path.
fn parse_source_declaration(
    value: &serde_json::Value,
) -> std::result::Result<Option<SourceDeclaration>, &'static str> {
    match value {
        serde_json::Value::Null => Ok(None),
        serde_json::Value::Object(_) => serde_json::from_value(value.clone())
            .map(Some)
            .map_err(|_| ERR_SETTINGS_INVALID_VALUE),
        _ => Err(ERR_SETTINGS_INVALID_VALUE),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;

    /// The daemon reads the staging directory `import-antigravity` writes to.
    ///
    /// It did not, and the reason given covered only half of what it
    /// excluded: a service manager's working directory means nothing to a
    /// daemon, which says nothing about a fixed path under the
    /// contributor's own 0700 state directory. A contributor who imported
    /// and then opened a desktop app saw nothing at all -- no entry, no
    /// error, no empty state naming Antigravity.
    #[test]
    fn the_daemon_reads_the_trajectory_staging_directory() {
        let (_d, store) = temp_store();
        let s = DaemonSettings::default();

        let names: Vec<&str> = crate::source::all_sources(&s.source_roots(&store))
            .iter()
            .map(|s| s.name())
            .collect();
        assert!(
            names.contains(&crate::source::SOURCE_TRAJECTORY),
            "the daemon must construct a trajectory source; got {names:?}"
        );
    }

    /// And ONLY the staging directory. The working-directory half of
    /// `TrajectorySelection::Auto` stays off, which is what the original
    /// exclusion was actually about: a daemon's working directory is
    /// whatever a service manager handed it.
    #[test]
    fn the_daemon_does_not_read_its_own_working_directory() {
        let (_d, store) = temp_store();
        let s = DaemonSettings::default();
        let roots = s.source_roots(&store);

        match roots.trajectory_selection() {
            crate::source::TrajectorySelection::Auto {
                working_dir,
                staging_dir,
            } => {
                assert!(
                    working_dir.is_none(),
                    "the daemon must not scan its own working directory"
                );
                assert_eq!(
                    staging_dir.as_deref(),
                    Some(
                        store
                            .dir()
                            .join(crate::source::TRAJECTORY_STAGING_SUBDIR)
                            .as_path()
                    )
                );
            }
            other => panic!("expected an Auto staging selection, got {other:?}"),
        }
    }

    #[test]
    fn settings_round_trip_through_the_store() {
        let (_d, store) = temp_store();
        let s = DaemonSettings {
            quiescence_secs: 60,
            ..Default::default()
        };
        s.save(&store).unwrap();
        assert_eq!(DaemonSettings::load(&store).unwrap().quiescence_secs, 60);
    }

    // `DaemonSettings::schema_version` has no `#[serde(default)]`, so a bare
    // `{}` does not exercise the field under test -- it fails to parse at
    // all, for an unrelated reason. Every case below starts from a full
    // `DaemonSettings::default()` value and edits just the `ironwire` key,
    // matching the pattern `a_settings_file_written_before_gemini_existed_
    // loads_with_it_absent` already uses for the same reason.

    #[test]
    fn a_contributor_who_never_mentioned_the_proxy_is_not_probed() {
        // The divergence from SourceDeclaration, and the reason for it. For a
        // session root, `None` falls back to the conventional location. There is
        // no conventional location for a local service: connecting to 127.0.0.1
        // unasked is a probe of something the contributor never mentioned, which
        // is the same mistake the source tri-state exists to have fixed.
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v.as_object_mut().unwrap().remove("ironwire");
        let settings: DaemonSettings = serde_json::from_value(v).expect("settings load");
        assert!(settings.ironwire.is_none());
        assert!(
            ironwire_ledger_for(settings.ironwire.as_ref()).is_none(),
            "no declaration means no reader is built at all"
        );
    }

    #[test]
    fn a_proxy_declared_off_builds_no_reader() {
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v["ironwire"] = serde_json::json!({"mode": "off"});
        let settings: DaemonSettings = serde_json::from_value(v).expect("loads");
        assert!(ironwire_ledger_for(settings.ironwire.as_ref()).is_none());
    }

    #[test]
    fn a_watched_proxy_round_trips_its_port() {
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v["ironwire"] = serde_json::json!({"mode": "watch", "port": 8463});
        let settings: DaemonSettings = serde_json::from_value(v).expect("loads");
        assert_eq!(
            settings.ironwire,
            Some(IronWireDeclaration::Watch { port: 8463 })
        );
    }

    /// `ironwire_ledger_for` and `SourceRoots::with_routing` are correct and
    /// are what a future task wires up. Neither is called from
    /// `source_roots` yet: `ironwire_ledger_for` builds a fresh, cold
    /// `IronWireLedger` on every call, so attaching one here would hand
    /// every caller its own never-refreshed snapshot -- it would compile and
    /// silently enrich nothing. Pinned so that regression does not sneak
    /// back in before the ledger has a single long-lived owner.
    #[test]
    fn source_roots_does_not_yet_attach_a_routing_overlay() {
        let (_d, store) = temp_store();
        let s = DaemonSettings {
            ironwire: Some(IronWireDeclaration::Watch { port: 8463 }),
            ..Default::default()
        };
        assert!(!s.source_roots(&store).is_routed());
    }

    /// The A2 rule, at the gate it must not join.
    ///
    /// Every desktop client already installed declares claude and codex and
    /// has no gemini field. A third conjunct here would stop the daemon
    /// starting on every one of them.
    #[test]
    fn roots_declared_does_not_require_a_gemini_declaration() {
        let declared = |path: &str| {
            Some(SourceDeclaration::Watch {
                path: PathBuf::from(path),
            })
        };
        let s = DaemonSettings {
            claude_source: declared("/declared/claude"),
            codex_source: declared("/declared/codex"),
            ..Default::default()
        };
        assert!(
            roots_declared(&s),
            "an absent gemini declaration is not disqualifying"
        );
        assert!(
            !gemini_declared(&s),
            "but a shell can still ask whether to offer the question"
        );

        let asked = DaemonSettings {
            gemini_source: Some(SourceDeclaration::Off),
            ..s
        };
        assert!(roots_declared(&asked));
        assert!(
            gemini_declared(&asked),
            "off is an answer; it is not the absence of one"
        );
    }

    /// A settings file written before this source existed must load, and
    /// must construct no gemini adapter.
    #[test]
    fn a_settings_file_written_before_gemini_existed_loads_with_it_absent() {
        let (_d, store) = temp_store();
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v.as_object_mut().unwrap().remove("gemini_source");
        store
            .write_daemon_file(DAEMON_SETTINGS_FILE, v.to_string().as_bytes())
            .unwrap();
        let loaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(loaded.gemini_source, None);
        assert!(
            !crate::source::all_sources(&loaded.source_roots(&store))
                .iter()
                .any(|s| s.name() == crate::source::SOURCE_GEMINI_CLI)
        );
    }

    /// The declarations reach `all_sources` as a map, so an adapter added
    /// later needs no call-site change anywhere in the daemon.
    #[test]
    fn source_roots_carries_every_declaration() {
        let s = DaemonSettings {
            claude_source: Some(SourceDeclaration::Off),
            codex_source: Some(SourceDeclaration::Off),
            gemini_source: Some(SourceDeclaration::Watch {
                path: PathBuf::from("/declared/gemini"),
            }),
            ..Default::default()
        };
        let (_d, store) = temp_store();
        let names: Vec<&str> = crate::source::all_sources(&s.source_roots(&store))
            .iter()
            .map(|s| s.name())
            .collect();
        // The trajectory source is always constructed now: the daemon reads
        // the staging directory `import-antigravity` writes to. It comes
        // last because `all_sources` appends it after the native adapters.
        assert_eq!(
            names,
            vec![
                crate::source::SOURCE_GEMINI_CLI,
                crate::source::SOURCE_TRAJECTORY
            ]
        );
    }

    #[test]
    fn the_gemini_declaration_is_settable_and_type_checked() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"gemini_source": {"mode": "off"}})
            ),
            Ok(true)
        );
        assert_eq!(s.gemini_source, Some(SourceDeclaration::Off));
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"gemini_source": "/a/path"})),
            Err(ERR_SETTINGS_INVALID_VALUE),
            "a bare string is the legacy *_root spelling, which this key \
             never had"
        );
    }

    /// Every source a shell can discover must have a settings key, and
    /// every one of those keys must be one `apply_settings_object` accepts.
    /// A source added to the table without both is a roots screen that
    /// silently discards the contributor's answer.
    #[test]
    fn every_discoverable_source_has_a_settings_key_that_round_trips() {
        let (_d, store) = temp_store();
        let home = std::env::temp_dir();
        for candidate in crate::source::discovery::probe(&home, |_| None) {
            let key = source_settings_key(&candidate.source)
                .unwrap_or_else(|| panic!("no settings key for {}", candidate.source));
            let mut s = DaemonSettings::default();
            assert_eq!(
                apply_settings_object(&mut s, &serde_json::json!({ key: {"mode": "off"} })),
                Ok(true),
                "{key} is not a key apply_settings_object accepts"
            );
            assert!(
                s.source_roots(&store)
                    .is_declared(candidate.source.as_str()),
                "{key} did not reach the declaration map"
            );
        }
    }

    #[test]
    fn settings_default_when_the_file_is_absent() {
        let (_d, store) = temp_store();
        let s = DaemonSettings::load(&store).unwrap();
        assert_eq!(s.quiescence_secs, DEFAULT_QUIESCENCE_SECS);
        assert_eq!(s.max_reuploads, DEFAULT_MAX_REUPLOADS);
        assert!(!s.local_notifications, "notifications must be opt-in");
        assert!(s.near_ai.is_none());
    }

    #[test]
    fn the_approval_hold_defaults_to_more_than_the_five_second_undo() {
        // The client-side undo is five seconds. A hold shorter than that
        // would leave the same race the hold exists to remove, so the
        // default is a floor with margin rather than an exact match.
        let s = DaemonSettings::default();
        assert_eq!(s.approval_hold_secs, DEFAULT_APPROVAL_HOLD_SECS);
        assert!(s.approval_hold_secs >= 5);
    }

    #[test]
    fn a_settings_file_written_before_the_hold_existed_loads_with_it_on() {
        // A missing key must not silently mean "no undo window".
        let (_d, store) = temp_store();
        let mut v = serde_json::to_value(DaemonSettings::default()).unwrap();
        v.as_object_mut().unwrap().remove("approval_hold_secs");
        store
            .write_daemon_file(DAEMON_SETTINGS_FILE, v.to_string().as_bytes())
            .unwrap();
        assert_eq!(
            DaemonSettings::load(&store).unwrap().approval_hold_secs,
            DEFAULT_APPROVAL_HOLD_SECS
        );
    }

    #[test]
    fn the_approval_hold_is_settable_and_type_checked() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"approval_hold_secs": 30})),
            Ok(true)
        );
        assert_eq!(s.approval_hold_secs, 30);
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"approval_hold_secs": "30"})),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(s.approval_hold_secs, 30, "a rejected value changes nothing");
    }

    #[test]
    fn a_source_can_be_declared_off_and_that_survives_a_round_trip() {
        // "I don't use Codex" has to be a durable, readable declaration --
        // distinguishable from "watching a path" AND from "never asked".
        // Before this existed the only way to express it was to leave the
        // field unset, which the daemon reads as the real ~/.codex: the
        // exact fail-open the refusal exists to prevent.
        let (_d, store) = temp_store();
        let s = DaemonSettings {
            claude_source: Some(SourceDeclaration::Watch {
                path: PathBuf::from("/somewhere/claude"),
            }),
            codex_source: Some(SourceDeclaration::Off),
            ..Default::default()
        };
        s.save(&store).unwrap();

        let loaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(loaded.codex_source, Some(SourceDeclaration::Off));
        assert_eq!(
            loaded.claude_source,
            Some(SourceDeclaration::Watch {
                path: PathBuf::from("/somewhere/claude")
            })
        );
    }

    #[test]
    fn off_is_not_the_same_as_never_asked() {
        let never = DaemonSettings::default();
        assert_eq!(
            never.codex_source, None,
            "a fresh install has been asked nothing"
        );

        let off = DaemonSettings {
            codex_source: Some(SourceDeclaration::Off),
            ..Default::default()
        };
        assert_ne!(
            off.codex_source, never.codex_source,
            "an answered 'I don't use this' must not collapse into 'not answered'"
        );
    }

    #[test]
    fn a_legacy_settings_file_using_claude_root_still_loads() {
        // Built from a real serialized default so this fixture cannot drift
        // out of sync with the struct, then downgraded to the old spelling:
        // source declarations removed, claude_root / codex_root added back.
        let (_d, store) = temp_store();
        let mut legacy = serde_json::to_value(DaemonSettings::default()).unwrap();
        let obj = legacy.as_object_mut().unwrap();
        obj.remove("claude_source");
        obj.remove("codex_source");
        obj.insert(
            "claude_root".to_string(),
            serde_json::Value::String("/legacy/claude".to_string()),
        );
        obj.insert(
            "codex_root".to_string(),
            serde_json::Value::String("/legacy/codex".to_string()),
        );
        store
            .write_daemon_file(DAEMON_SETTINGS_FILE, &serde_json::to_vec(&legacy).unwrap())
            .unwrap();

        let loaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(
            loaded.claude_source,
            Some(SourceDeclaration::Watch {
                path: PathBuf::from("/legacy/claude")
            })
        );
        assert!(
            roots_declared(&loaded),
            "an install that already declared both roots must not be re-asked"
        );

        // And the rewrite drops the old spelling rather than carrying two.
        loaded.save(&store).unwrap();
        let raw = String::from_utf8(
            store
                .read_daemon_file(DAEMON_SETTINGS_FILE)
                .unwrap()
                .unwrap(),
        )
        .unwrap();
        assert!(
            !raw.contains("claude_root"),
            "legacy key must not be rewritten: {raw}"
        );
        assert!(raw.contains("claude_source"));
    }

    #[test]
    fn roots_are_declared_only_when_both_are_set() {
        let mut s = DaemonSettings::default();
        assert!(
            !roots_declared(&s),
            "a fresh settings object declares neither source"
        );

        s.claude_source = Some(SourceDeclaration::Watch {
            path: PathBuf::from("/somewhere/claude"),
        });
        assert!(
            !roots_declared(&s),
            "half a declaration is the fail-open case: an undeclared codex \
             source means the daemon watches the real ~/.codex"
        );

        s.codex_source = Some(SourceDeclaration::Off);
        assert!(
            roots_declared(&s),
            "'I don't use Codex' is an answer. Declared-off is declared -- \
             that is the entire reason it has to be representable"
        );

        s.claude_source = None;
        assert!(!roots_declared(&s), "the rule is symmetric");
    }

    #[test]
    fn settings_are_written_readable_only_by_the_owner() {
        // near_ai carries an API key.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let (_d, store) = temp_store();
            DaemonSettings::default().save(&store).unwrap();
            let meta = std::fs::metadata(store.daemon_path(DAEMON_SETTINGS_FILE)).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        }
    }

    // --- Runtime-settable daily caps (issue #371) --------------------------

    #[test]
    fn a_valid_cap_change_is_accepted_persisted_and_observed_by_the_next_cap_check() {
        use crate::daemon::state::DaemonState;
        use crate::daemon::uploader::cap_check;

        let (_d, store) = temp_store();
        let mut s = DaemonSettings::default();
        assert_eq!(s.max_uploads_per_day, DEFAULT_MAX_UPLOADS_PER_DAY);
        assert_eq!(s.max_bytes_per_day, DEFAULT_MAX_BYTES_PER_DAY);

        // A state that has already exhausted the *default* budget: the
        // real-world trigger for this feature was exactly this shape --
        // approved traces waiting with nothing left in the old budget.
        let mut st = DaemonState::new();
        st.uploads_today = DEFAULT_MAX_UPLOADS_PER_DAY;
        st.bytes_today = DEFAULT_MAX_BYTES_PER_DAY;
        assert!(
            !cap_check(&st, 1, &s),
            "sanity: the default budget really is exhausted"
        );

        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({
                    "max_uploads_per_day": 200,
                    "max_bytes_per_day": 2_147_483_648u64,
                }),
            ),
            Ok(true)
        );
        assert_eq!(s.max_uploads_per_day, 200);
        assert_eq!(s.max_bytes_per_day, 2_147_483_648);

        // Persisted: a restart must not revert what was just raised.
        s.save(&store).unwrap();
        let reloaded = DaemonSettings::load(&store).unwrap();
        assert_eq!(reloaded.max_uploads_per_day, 200);
        assert_eq!(reloaded.max_bytes_per_day, 2_147_483_648);

        // Observed: the same state, held against the *reloaded* settings
        // (standing in for the live `Mutex<DaemonSettings>` a running
        // uploader reads each tick), now has room again.
        assert!(
            cap_check(&st, 1, &reloaded),
            "raising the cap must be visible to the very next cap check, \
             with no restart required"
        );
    }

    #[test]
    fn max_uploads_per_day_above_the_ceiling_is_rejected() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_uploads_per_day": MAX_UPLOADS_PER_DAY_CEILING + 1}),
            ),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(
            s.max_uploads_per_day, DEFAULT_MAX_UPLOADS_PER_DAY,
            "a rejected value changes nothing"
        );
        // The ceiling itself is accepted -- it is a ceiling, not an
        // exclusive bound.
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_uploads_per_day": MAX_UPLOADS_PER_DAY_CEILING}),
            ),
            Ok(true)
        );
        assert_eq!(s.max_uploads_per_day, MAX_UPLOADS_PER_DAY_CEILING);
    }

    #[test]
    fn max_bytes_per_day_above_the_ceiling_is_rejected() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_bytes_per_day": MAX_BYTES_PER_DAY_CEILING + 1}),
            ),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(s.max_bytes_per_day, DEFAULT_MAX_BYTES_PER_DAY);
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_bytes_per_day": MAX_BYTES_PER_DAY_CEILING}),
            ),
            Ok(true)
        );
        assert_eq!(s.max_bytes_per_day, MAX_BYTES_PER_DAY_CEILING);
    }

    #[test]
    fn a_daily_cap_of_zero_is_rejected_not_treated_as_pause() {
        // Zero would silently overlap with `pause`, which is a different,
        // visibly-temporary state. It is refused like any other
        // out-of-range value.
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"max_uploads_per_day": 0})),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"max_bytes_per_day": 0})),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
    }

    #[test]
    fn a_cap_below_the_default_is_allowed_as_self_throttling() {
        // A contributor throttling their own uploads is legitimate and is
        // not the safety concern the ceiling exists for.
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_uploads_per_day": 1, "max_bytes_per_day": 1}),
            ),
            Ok(true)
        );
        assert_eq!(s.max_uploads_per_day, 1);
        assert_eq!(s.max_bytes_per_day, 1);
    }

    #[test]
    fn an_unknown_key_alongside_a_valid_cap_is_still_rejected_outright() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(
                &mut s,
                &serde_json::json!({"max_uploads_per_day": 100, "nonsense": 1}),
            ),
            Err(ERR_SETTINGS_UNKNOWN_FIELD)
        );
    }

    #[test]
    fn wrong_type_for_a_cap_is_rejected_and_changes_nothing() {
        let mut s = DaemonSettings::default();
        assert_eq!(
            apply_settings_object(&mut s, &serde_json::json!({"max_bytes_per_day": "lots"})),
            Err(ERR_SETTINGS_INVALID_VALUE)
        );
        assert_eq!(s.max_bytes_per_day, DEFAULT_MAX_BYTES_PER_DAY);
    }
}
