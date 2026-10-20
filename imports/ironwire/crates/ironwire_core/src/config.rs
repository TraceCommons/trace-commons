//! On-disk configuration and the runtime home directory.
//!
//! Everything IronWire persists lives under `$IRONWIRE_HOME` (default
//! `~/.ironwire`, mode 0700). See `docs/PACKAGING.md` for the layout.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::DEFAULT_PORT;
use crate::error::{Error, Result};
use crate::protocol::{BackendKind, ModelTier};

/// Resolved filesystem locations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PathsConfig {
    /// `$IRONWIRE_HOME`.
    pub home: PathBuf,
}

impl PathsConfig {
    /// Resolve from the environment.
    ///
    /// # Errors
    ///
    /// Returns [`Error::NoHome`] when neither `$IRONWIRE_HOME` nor a home
    /// directory can be determined.
    pub fn resolve() -> Result<Self> {
        if let Some(home) = std::env::var_os("IRONWIRE_HOME") {
            return Ok(Self {
                home: PathBuf::from(home),
            });
        }
        let home = dirs::home_dir().ok_or(Error::NoHome)?.join(".ironwire");
        Ok(Self { home })
    }

    /// Root under an explicit directory. Used by tests.
    #[must_use]
    pub fn rooted_at(home: impl Into<PathBuf>) -> Self {
        Self { home: home.into() }
    }

    /// `config.toml`.
    #[must_use]
    pub fn config_file(&self) -> PathBuf {
        self.home.join("config.toml")
    }

    /// Recorded subscription consents (`docs/TRUST.md` §2).
    #[must_use]
    pub fn consent_file(&self) -> PathBuf {
        self.home.join("consent.json")
    }

    /// Control-API bearer token, mode 0600.
    #[must_use]
    pub fn control_token_file(&self) -> PathBuf {
        self.home.join("control.token")
    }

    /// Single-daemon lockfile.
    #[must_use]
    pub fn lock_file(&self) -> PathBuf {
        self.home.join("daemon.lock")
    }

    /// Local trace ledger.
    #[must_use]
    pub fn ledger_file(&self) -> PathBuf {
        self.home.join("ledger.sqlite")
    }

    /// Captured request and response bodies, when `capture.bodies = true`
    /// (`docs/PACKAGING.md`). Mode 0700 -- these are the user's source code.
    #[must_use]
    pub fn bodies_dir(&self) -> PathBuf {
        self.home.join("bodies")
    }

    /// Cached signed provider-catalog document.
    #[must_use]
    pub fn catalog_file(&self) -> PathBuf {
        self.home.join("catalog.json")
    }

    /// Observed quota from the previous run, mode 0600
    /// (`crate::quota_store`).
    #[must_use]
    pub fn quota_file(&self) -> PathBuf {
        self.home.join("quota.json")
    }

    /// Cached result of the last update check.
    #[must_use]
    pub fn update_cache_file(&self) -> PathBuf {
        self.home.join("update.json")
    }
}

/// Listener settings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ServerConfig {
    /// Loopback port. There is deliberately no `host` field — IronWire binds
    /// `127.0.0.1` and nothing else (`docs/TRUST.md` I1).
    pub port: u16,
    /// Idle timeout for upstream requests, in seconds. Long, because coding
    /// agents legitimately generate for many minutes.
    pub upstream_timeout_secs: u64,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            upstream_timeout_secs: 900,
        }
    }
}

/// Trace capture settings. Local capture is on; upload is a separate, explicit
/// decision made elsewhere (`docs/TRUST.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CaptureConfig {
    /// Record metadata for every exchange.
    pub enabled: bool,
    /// Also record request and response bodies. Off by default: these contain
    /// the user's source code.
    pub bodies: bool,
    /// How many days of exchanges to keep.
    ///
    /// A ledger with no retention grows for the life of the install, silently,
    /// on a machine where nobody is watching a SQLite file in a dotdir. Ninety
    /// days is long enough for "what did my agent do last quarter" and short
    /// enough that the file stays a few tens of megabytes.
    ///
    /// `0` disables pruning — an explicit choice someone can make, not the
    /// default, and not the accident it currently is.
    pub retain_days: u32,
    /// Ask a Chat Completions backend for the log-probability of each token it
    /// generates. `false` — the default — does not ask.
    ///
    /// A boolean rather than a count of alternatives. The only consumer is the
    /// confidence reduction in [`crate::confidence`], which is defined over
    /// p(chosen token) and never reads the alternatives, so a `top_logprobs: k`
    /// knob would be bandwidth paid for on every turn and read by nobody.
    ///
    /// Off by default for three separate reasons, any one of which would be
    /// enough. It changes what the provider is asked to produce, so a captured
    /// exchange is not comparable to an uncaptured one. It inflates every
    /// response materially, which on an agent loop is per-turn cost rather
    /// than a one-off. And the distributions are conditioned on the whole
    /// context, which makes them more sensitive than the text they describe —
    /// see `docs/PRIVACY.md`, and note this only holds together at all because
    /// substitution happens *before* the request goes out.
    ///
    /// Needs `enabled`: with no ledger there is nothing to write the aggregate
    /// to, and asking would be cost with no reader. Cross-family only — the
    /// native lane's byte-identity claim (`docs/PROTOCOL.md` §2) is untouched
    /// by this setting, and a request that is not translated is never modified
    /// to carry it.
    pub logprobs: bool,
    /// Detailed evidence capture. Separate consent from aggregate logprobs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token_capture: Option<TokenCaptureConfig>,
    /// Look up each NEAR AI exchange's receipt after it completes, and record
    /// whether it proves which model answered (`ironwire_proxy::proof`).
    /// `false` -- the default -- leaves those rows `pending`.
    ///
    /// Off by default because it makes a network call the exchange itself did
    /// not: a `GET` per NEAR AI exchange, to NEAR AI, with the user's NEAR AI
    /// key, naming the response id. The provider already served the call, so
    /// it learns that the call is being checked and nothing else. It runs off
    /// the response path and cannot delay or change an answer.
    ///
    /// Needs `enabled`. A receipt is checked against the digests of the bytes
    /// that crossed the wire; with this on they are taken for every NEAR AI
    /// exchange as the bytes stream past, whether or not `bodies` is on. With
    /// bodies off nothing is accumulated or written -- only hashed.
    pub receipts: bool,
}

impl Default for CaptureConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            bodies: false,
            retain_days: 90,
            logprobs: false,
            token_capture: None,
            receipts: false,
        }
    }
}

/// Explicitly selected backend/model pair; no routing fallback is added.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TokenCaptureTarget {
    /// Registered backend identity.
    pub backend: String,
    /// Exact selected model identity.
    pub model: String,
}
/// Local detailed evidence capture, with independent bounded retention.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TokenCaptureConfig {
    /// Models whose provider support the operator has qualified.
    pub targets: Vec<TokenCaptureTarget>,
    /// Requested alternatives; zero requests only chosen-token probabilities.
    pub top_k: u8,
    /// Global raw evidence byte budget.
    pub max_bytes: u64,
    /// Unleased evidence lifetime in seconds.
    pub retain_seconds: i64,
}
impl Default for TokenCaptureConfig {
    fn default() -> Self {
        Self {
            targets: Vec::new(),
            top_k: 20,
            max_bytes: 512 * 1024 * 1024,
            retain_seconds: 3 * 86400,
        }
    }
}
impl TokenCaptureConfig {
    /// True only for an explicitly selected pair. Never guesses capabilities.
    #[must_use]
    pub fn selects(&self, backend: &str, model: &str) -> bool {
        self.targets
            .iter()
            .any(|target| target.backend == backend && target.model == model)
    }
}

/// What `ironwire status` may say about how fast capacity is going.
///
/// Everything here describes *IronWire's own traffic*, measured from the local
/// ledger — never a provider's quota, which is reported or `unknown` and never
/// estimated (`AGENTS.md` rule 2, `docs/CRITIQUE.md` §4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UsageConfig {
    /// Show the session section on `ironwire status`. Needs `capture.enabled`
    /// — with no ledger there is nothing to measure.
    pub enabled: bool,
    /// Length of a session window, in hours. Five is Claude Code's, and the
    /// figure the whole comparison is calibrated against; change it only if
    /// your provider's window really is a different length.
    pub session_hours: u32,
    /// How far back to look for completed windows when calibrating against
    /// your own history. Eight days: long enough for a working week, short
    /// enough that a change in how you work shows up inside one.
    pub history_hours: u32,
    /// Your subscription plan: `pro`, `max5`, `max20`, or `team`.
    ///
    /// There is deliberately no default. Published per-window token limits do
    /// not exist — the figures in circulation are reverse-engineered — so
    /// IronWire will not assert one. Setting this makes the ceiling *your*
    /// claim about *your* plan, and the status screen says so. Left unset, the
    /// comparison is against your own past sessions, which needs no table.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan: Option<String>,
}

impl Default for UsageConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            session_hours: 5,
            history_hours: 192,
            plan: None,
        }
    }
}

/// Update-check settings. IronWire never applies an update itself; this only
/// governs whether it *looks* (`docs/UPDATES.md`, `docs/TRUST.md` §7).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UpdateConfig {
    /// Check for a newer release, at most once a day. The one request IronWire
    /// makes that is not the user's own work, and switchable off.
    pub check: bool,
}

impl Default for UpdateConfig {
    fn default() -> Self {
        Self { check: true }
    }
}

/// How hard to work at keeping a streamed response alive.
///
/// These exist because a coding agent's own patience is shorter than a model's
/// thinking time, and the gap is where "Response stalled mid-stream" lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResilienceConfig {
    /// Emit an SSE `ping` after this many seconds of upstream silence.
    pub keepalive_secs: u64,
    /// Give up on a silent upstream after this many seconds and end the stream
    /// with a stated error rather than pinging forever.
    pub stall_timeout_secs: u64,
    /// How many times to transparently restart a stream that died before
    /// producing any content.
    pub max_reconnects: usize,
    /// Stall timeout for a turn that looks like compaction
    /// (`docs/PROTOCOL.md` §8).
    ///
    /// A compaction turn sends the whole conversation and asks for a long
    /// summary, so it thinks for far longer than an ordinary turn before
    /// producing a token. Applying the normal timeout to it would abandon the
    /// one turn whose output becomes permanent — and it would do so most often
    /// in exactly the longest sessions, where compaction matters most.
    pub compaction_stall_timeout_secs: u64,
    /// Stall timeout for a turn served by local capacity.
    ///
    /// A 70B model on a laptop CPU can take minutes to its first token, which
    /// the ordinary timeout reads as a dead upstream. Defaults to the ordinary
    /// one and is floored at it, so this can lengthen IronWire's patience with
    /// a slow local model and never shorten it.
    pub local_stall_timeout_secs: u64,
}

impl Default for ResilienceConfig {
    fn default() -> Self {
        Self {
            keepalive_secs: 15,
            stall_timeout_secs: 180,
            max_reconnects: 2,
            compaction_stall_timeout_secs: 600,
            local_stall_timeout_secs: 180,
        }
    }
}

impl ResilienceConfig {
    /// The stall timeout to apply to this turn.
    #[must_use]
    pub fn stall_timeout_for(&self, likely_compaction: bool) -> u64 {
        if likely_compaction {
            // Never *shorter* than the ordinary timeout, whatever a user
            // configures: a config that made compaction more fragile than a
            // normal turn would be the opposite of the point.
            self.compaction_stall_timeout_secs
                .max(self.stall_timeout_secs)
        } else {
            self.stall_timeout_secs
        }
    }

    /// The stall timeout for a turn, given whether local capacity is serving it.
    ///
    /// Same floor rule, same reason: a local model is the slowest thing
    /// IronWire routes to, so a configured value below the ordinary timeout
    /// would make the one backend that needs the most patience the least
    /// patiently treated.
    #[must_use]
    pub fn stall_timeout_for_backend(&self, likely_compaction: bool, is_local: bool) -> u64 {
        let base = self.stall_timeout_for(likely_compaction);
        if is_local {
            base.max(self.local_stall_timeout_secs)
                .max(self.stall_timeout_secs)
        } else {
            base
        }
    }
}

/// The optional privacy filter.
///
/// **Off by default, and that is a trust commitment rather than a default**
/// (`docs/TRUST.md` I7). Everything else in IronWire rests on forwarding bytes
/// it did not change; this filter changes them by design, so it is never on
/// unless the user turned it on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PrivacyConfig {
    /// How much the user wants substituted. See [`PrivacyMode`].
    ///
    /// `None` means the file predates the ladder and still says
    /// `enabled`/`secrets`; [`PrivacyConfig::mode`] resolves the two.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<PrivacyMode>,
    /// Deprecated master switch, kept so an existing `config.toml` still loads
    /// — `deny_unknown_fields` would otherwise reject every upgraded install.
    #[serde(default, skip_serializing_if = "is_false")]
    pub enabled: bool,
    /// Deprecated. Superseded by `mode = "credentials"`.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub secrets: bool,
    /// Tier 2: exact strings to substitute, nominated by the user.
    ///
    /// These are the user's own sensitive values and they live in
    /// `$IRONWIRE_HOME/config.toml` (mode 0700). That is a real trade-off:
    /// listing them is what makes tier 2 work, and it also writes them down.
    /// Stated here rather than left for someone to discover.
    pub named_values: Vec<String>,
    /// Scan inside fenced code blocks. Off by default — a value in one is
    /// nearly always the code being edited, and substituting it makes the model
    /// rewrite a file into something that does not work.
    pub scan_code_blocks: bool,
    /// Backends that may receive requests under `mode = "full"`.
    ///
    /// Defaults to `["nearai"]`. This was deliberately empty before, on the
    /// argument that a shipped default is IronWire asserting some operator is
    /// trustworthy for someone else's data. That argument has not gone away —
    /// it is now answered rather than avoided: NEAR AI is this project's own
    /// destination, so the assertion is about ourselves and not about a third
    /// party, and `full` with nothing named was a mode that could not be
    /// selected without first editing a file most users never open.
    ///
    /// What this does **not** claim is that the destination is safe. `full`
    /// means "only send to backends named here", which is a routing
    /// constraint, not a property of the operator (`docs/TRUST.md` I7). The
    /// user can replace this list; naming their own destinations still
    /// overrides it entirely.
    ///
    /// Ignored below `full`, with a warning at load for a list the user wrote
    /// themselves. The shipped default is not warned about: it is on every
    /// machine, and `full` is on almost none.
    #[serde(
        default = "default_trusted_backends",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub trusted_backends: Vec<String>,
    /// Scan replayed tool results. Off by default — they are output from the
    /// user's own machine that the model needs verbatim to reason about.
    pub scan_tool_results: bool,
}

/// The destination `full` names when the user has named none.
///
/// A function rather than a constant because `serde(default = ...)` needs one,
/// and it is the single place the shipped answer to "trusted by whom" lives.
fn default_trusted_backends() -> Vec<String> {
    vec!["nearai".to_string()]
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn is_true(value: &bool) -> bool {
    *value
}

/// How much of a request the user wants substituted.
///
/// A ladder rather than a set of independent switches, and every level is a
/// strict superset of the one below — so `mode >= PrivacyMode::Pii` is the
/// natural way to ask a question about it. A matrix of booleans is more
/// expressive and strictly worse: it multiplies the states we have to test, and
/// it lets someone assemble a configuration that sounds protective and is not.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PrivacyMode {
    /// No substitution; requests are forwarded byte-identical.
    ///
    /// The default, and it stays the default. This is the one component that
    /// modifies a request, and everything else IronWire promises rests on
    /// forwarding bytes it did not change (`docs/TRUST.md` I7).
    #[default]
    Off,
    /// API keys, tokens, private keys, and any `named_values`.
    Credentials,
    /// Credentials, plus the deterministic PII classes: email addresses, IP
    /// addresses and phone numbers.
    ///
    /// Deliberately not human names: those need the tier-3 classifier, and a
    /// regex for them would be a false-negative machine that reads as
    /// protection (`docs/PRIVACY.md` §2).
    Pii,
    /// Everything in `pii`, and route only to backends the user marked
    /// trusted rather than falling back to any other provider.
    Full,
}

impl PrivacyMode {
    /// What this level substitutes, in one clause.
    #[must_use]
    pub fn describe(self) -> &'static str {
        match self {
            Self::Off => "off — requests are forwarded unchanged",
            Self::Credentials => "credentials: API keys, tokens, private keys, and named values",
            Self::Pii => {
                "credentials, plus deterministic PII: emails, IP addresses, phone numbers \
                 (not names — those need the tier-3 classifier)"
            }
            Self::Full => {
                "credentials and deterministic PII, and only trusted backends are routed to"
            }
        }
    }
}

impl Default for PrivacyConfig {
    fn default() -> Self {
        Self {
            mode: None,
            enabled: false,
            secrets: true,
            named_values: Vec::new(),
            trusted_backends: default_trusted_backends(),
            scan_code_blocks: false,
            scan_tool_results: false,
        }
    }
}

impl PrivacyConfig {
    /// The mode in force, resolving a file that predates the ladder.
    ///
    /// `mode` wins outright when it is present. Otherwise the old booleans are
    /// mapped: they could only ever express "off" or "credentials", so that is
    /// what they map to, and an upgraded install keeps filtering exactly what
    /// it filtered before.
    #[must_use]
    pub fn mode(&self) -> PrivacyMode {
        if let Some(mode) = self.mode {
            return mode;
        }
        if self.enabled && (self.secrets || !self.named_values.is_empty()) {
            PrivacyMode::Credentials
        } else {
            PrivacyMode::Off
        }
    }

    /// Whether this backend may be routed to at all.
    ///
    /// Only `full` restricts anything; every lower level returns true for
    /// everything, so this is safe to call unconditionally from the router.
    #[must_use]
    pub fn trusts(&self, backend: &str) -> bool {
        self.mode() != PrivacyMode::Full || self.trusted_backends.iter().any(|id| id == backend)
    }

    /// Whether the filter would actually do anything.
    ///
    /// On-but-configured-with-nothing is a real state and it should read as
    /// off, not as protection: `credentials` with `secrets` disabled and no
    /// named values matches nothing at all.
    #[must_use]
    pub fn is_active(&self) -> bool {
        match self.mode() {
            PrivacyMode::Off => false,
            // `pii` and `full` always have patterns of their own to run.
            PrivacyMode::Pii | PrivacyMode::Full => true,
            PrivacyMode::Credentials => self.secrets || !self.named_values.is_empty(),
        }
    }

    /// One line describing what is running — never what the user is safe from
    /// (`docs/TRUST.md` I7).
    #[must_use]
    pub fn summary(&self) -> String {
        let mode = self.mode();
        if mode == PrivacyMode::Off {
            return "off".to_string();
        }
        if !self.is_active() {
            return "on, but nothing configured to match".to_string();
        }
        let mut parts = vec![format!("{mode:?}").to_lowercase()];
        if !self.named_values.is_empty() {
            parts.push(format!("{} named value(s)", self.named_values.len()));
        }
        let mut line = parts.join(" + ");
        if mode == PrivacyMode::Full {
            // What it is *doing*, never what the user is safe from: the trusted
            // backend still receives the data (`docs/TRUST.md` I7).
            line.push_str(&format!(
                " — routing restricted to: {}",
                self.trusted_backends.join(", ")
            ));
        }
        line
    }
}

/// A user-configured backend.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendConfig {
    /// Stable id, e.g. `claude-sub`.
    pub id: String,
    /// Backend implementation to construct: `claude-subscription`,
    /// `anthropic-api`, `codex-subscription`, `openai-api`, `nearai`,
    /// `openai-compatible`.
    pub kind: String,
    /// Whether the router may use it.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Base URL override, for `openai-compatible` and for testing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Environment variable holding the API key, where applicable. IronWire
    /// stores no secrets in `config.toml`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key_env: Option<String>,
    /// Where an official client keeps this login: the value you export as
    /// `CLAUDE_CONFIG_DIR` (`claude-subscription`) or `CODEX_HOME`
    /// (`codex-subscription`) when you use it. Absolute, or starting with `~/`.
    ///
    /// Kept as the text the user wrote rather than a resolved path. Claude
    /// Code keys its Keychain item on the exact directory string it was given,
    /// so a value that had been canonicalised — symlinks resolved, a trailing
    /// slash trimmed — would name a different Keychain item from the one the
    /// client wrote. See [`expand_config_dir`].
    ///
    /// Required on every subscription entry except the default login
    /// (`claude-sub`, `codex-sub`), where it relocates that login.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    /// Model slugs this backend should offer, best first. Configurable because
    /// third-party catalogues move faster than our release cadence.
    ///
    /// A bare string takes the tier IronWire infers from its name — except on
    /// a local backend, where it is [`ModelTier::Fast`] (see
    /// [`ModelEntry::tier_on`]). `{ name = "...", tier = "..." }` states one
    /// outright.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ModelEntry>>,
    /// Refuse a model name the provider would serve under an alias, rather
    /// than accept the substitution.
    ///
    /// NEAR AI only. Off by default: turning it on means an aliased name is
    /// answered 400 instead of served by the canonical model, which is the
    /// right trade for a caller binding payloads to a model TD's signing key
    /// and the wrong one for a caller who just wants an answer. Either way the
    /// substitution is recorded on the exchange -- this only decides whether it
    /// is also refused.
    #[serde(default)]
    pub refuse_model_aliases: bool,
}

/// A configured model: a slug, and optionally the tier it should count as.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ModelEntry {
    /// Just the slug.
    Name(String),
    /// The slug and the tier the user says it belongs to.
    Tiered {
        /// Model slug, as the provider spells it.
        name: String,
        /// `fast`, `balanced` or `frontier`.
        tier: ModelTier,
    },
}

impl ModelEntry {
    /// The slug.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::Name(name) | Self::Tiered { name, .. } => name,
        }
    }

    /// The tier this entry counts as on a backend of `kind`.
    ///
    /// A declared tier always wins. Otherwise the name decides — except on
    /// local capacity, where the name must not.
    ///
    /// `ModelTier::from_model_hint` resolves anything it does not recognise to
    /// `Frontier`, deliberately: for a hosted catalogue, guessing low silently
    /// downgrades a user's work. For a local catalogue the same default is
    /// catastrophic, because local capacity also sorts cheapest — `qwen3-coder:30b`
    /// would read as frontier-tier and take work meant for Opus. So a local
    /// model is `Fast` until the user says otherwise. They opt it up the
    /// ladder; IronWire never does it on their behalf.
    #[must_use]
    pub fn tier_on(&self, kind: BackendKind) -> ModelTier {
        match self {
            Self::Tiered { tier, .. } => *tier,
            Self::Name(name) => {
                if kind == BackendKind::Local {
                    ModelTier::Fast
                } else {
                    ModelTier::from_model_hint(name)
                }
            }
        }
    }
}

fn default_true() -> bool {
    true
}

/// Resolve a `config_dir` the way a shell resolves what the user exported:
/// a leading `~/` becomes `home`, and nothing else changes.
///
/// Textual on purpose, and free of I/O. Claude Code hashes the exact
/// `CLAUDE_CONFIG_DIR` string it sees to name its Keychain item, and a shell
/// expanding `~/.claude-work` produces exactly `<home>/.claude-work` — so this
/// reproduces the string the client hashed. Canonicalising (resolving
/// symlinks, trimming a trailing slash) would produce a tidier path and the
/// wrong Keychain item. It also keeps [`Config::validate`] free of the
/// filesystem: whether the directory exists is a question for registration,
/// where "not there" means "not logged in", not "invalid config".
///
/// Concatenated rather than joined, as the credential side does it:
/// `Path::join` treats the rest of `~//.claude-work` as an absolute path and
/// drops the home, so the client file would be looked for under `/` while the
/// credential is read from the home directory.
#[must_use]
pub fn expand_config_dir(raw: &str, home: &Path) -> PathBuf {
    match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => {
            let mut expanded = home.as_os_str().to_os_string();
            expanded.push(rest);
            PathBuf::from(expanded)
        }
        _ => PathBuf::from(raw),
    }
}

/// Top-level configuration.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    /// Listener settings.
    pub server: ServerConfig,
    /// Trace capture settings.
    pub capture: CaptureConfig,
    /// What the status screen may say about burn rate and projections.
    pub usage: UsageConfig,
    /// Update-check settings.
    pub updates: UpdateConfig,
    /// Stream-resilience settings.
    pub resilience: ResilienceConfig,
    /// Optional privacy filter. Off by default (`docs/PRIVACY.md`).
    pub privacy: PrivacyConfig,
    /// Configured backends, in preference order for ties.
    pub backends: Vec<BackendConfig>,
    /// Spend caps. No cap unless the user sets one.
    pub limits: LimitsConfig,
    /// How several subscriptions to one product relate to each other.
    pub subscriptions: SubscriptionsConfig,
}

/// How several subscriptions to one product relate to each other.
///
/// Each login is its own backend, and by default a client is served only by
/// its own: the Claude Code using `~/.claude-work` is served by that login's
/// subscription and, under sustained quota pressure or once it runs out,
/// descends the ordinary ladder — API key, NEAR AI, local — never onto the
/// user's other Claude subscription.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SubscriptionsConfig {
    /// Let a client be served by another of the user's subscriptions to the
    /// same product when its own is under quota pressure: at least 90% used
    /// as the provider reports it, or spent. A request choosing afresh moves
    /// at once; a conversation already on its own login moves once the
    /// pressure has lasted the 20-second descent debounce. That is the trigger
    /// every rung of the ladder uses, and pressure outranks the preference for
    /// the client's own login. It is not reserved for a subscription that has
    /// run out.
    ///
    /// Off by default, and the reasons are not ours to wave away. OpenAI's
    /// terms forbid circumventing rate limits and Anthropic's usage policy
    /// forbids bypassing restrictions established in its products; moving to
    /// another account after a 429 is the most direct reading of both. The
    /// provider can also see it: Claude Code's request body names the account
    /// it is logged in as, and would arrive on the other account's token. See
    /// `docs/TRUST.md`.
    ///
    /// Even when on, a Codex conversation that replays encrypted reasoning
    /// never changes ChatGPT account: OpenAI rejects reasoning minted under
    /// another organization, so the move would fail the turn rather than save
    /// it.
    pub cross_account_fallback: bool,
}

/// What to do when a spend cap is reached.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BreachAction {
    /// Keep working on whatever free capacity remains, following the ordinary
    /// ladder. The default, because the product's promise is that the agent
    /// does not die — a cap that killed the session by default would invert it.
    #[default]
    Descend,
    /// Stop, so the user finds out immediately. For someone who wants a hard
    /// stop and will set it deliberately.
    Refuse,
}

/// A cap on one backend.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BackendLimit {
    /// Backend id this applies to.
    pub id: String,
    /// Dollars per day. `0` means no cap.
    pub daily_spend_usd: f64,
}

/// Spend caps.
///
/// Money only, and metered money at that: a subscription is already paid for,
/// and capping it would cap capacity the user bought (see
/// `Summary::cost_by_backend`). Prepaid credits are excluded for the same
/// reason — `BackendKind::is_metered` draws the line.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LimitsConfig {
    /// Dollars per day across every metered backend. `None` or `0` is no cap.
    pub daily_spend_usd: Option<f64>,
    /// What happens when a cap is reached.
    pub on_breach: BreachAction,
    /// Per-backend caps, which apply on top of any global one.
    pub backends: Vec<BackendLimit>,
}

impl LimitsConfig {
    /// Whether any cap is actually set.
    ///
    /// A configured `[limits]` block with every value zero is not a cap, and
    /// must not switch on the machinery — including the startup check that
    /// refuses to run a cap without the ledger behind it.
    #[must_use]
    pub fn any_cap(&self) -> bool {
        self.daily_spend_usd.is_some_and(|cap| cap > 0.0)
            || self.backends.iter().any(|b| b.daily_spend_usd > 0.0)
    }

    /// The cap for one backend: its own, or the global one, whichever binds
    /// first. `None` when neither is set.
    #[must_use]
    pub fn cap_for(&self, id: &str) -> Option<f64> {
        let specific = self
            .backends
            .iter()
            .find(|b| b.id == id)
            .map(|b| b.daily_spend_usd)
            .filter(|cap| *cap > 0.0);
        let global = self.daily_spend_usd.filter(|cap| *cap > 0.0);
        match (specific, global) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (only, None) | (None, only) => only,
        }
    }
}

impl Config {
    /// Load from `paths.config_file()`, returning defaults when absent.
    ///
    /// # Errors
    ///
    /// Returns [`Error::ConfigRead`] or [`Error::ConfigParse`] when the file
    /// exists but cannot be used. A missing file is not an error — a fresh
    /// install must start.
    pub fn load(paths: &PathsConfig) -> Result<Self> {
        Self::load_from(&paths.config_file())
    }

    /// Load from an explicit path.
    ///
    /// # Errors
    ///
    /// As [`Config::load`].
    pub fn load_from(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let config: Self = toml::from_str(&text).map_err(|detail| Error::ConfigParse {
                    path: path.to_path_buf(),
                    detail,
                })?;
                // Only for a file that exists and parsed. A missing config must
                // still start a fresh install, which is the commonest case of
                // all.
                config.validate(path)?;
                Ok(config)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(source) => Err(Error::ConfigRead {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    /// Check what `deny_unknown_fields` cannot.
    ///
    /// Serde catches a misspelled *key*, which makes the file feel checked —
    /// so a value that is accepted reads as a value that works. These are the
    /// ways an entry can be well-formed and still describe a backend that
    /// cannot exist, and every one of them is better said now than discovered
    /// as a backend that silently never appears.
    ///
    /// # Errors
    ///
    /// [`Error::ConfigInvalid`], naming the entry at fault.
    pub fn validate(&self, path: &Path) -> Result<()> {
        // The home directory is read only to compare `~/` paths with absolute
        // ones; nothing here touches the filesystem.
        self.validate_with_home(path, &dirs::home_dir().unwrap_or_default())
    }

    pub(crate) fn validate_with_home(&self, path: &Path, home: &Path) -> Result<()> {
        let invalid = |id: &str, detail: String| Error::ConfigInvalid {
            path: path.to_path_buf(),
            id: id.to_string(),
            detail,
        };

        if let Some(capture) = &self.capture.token_capture
            && (!self.capture.enabled
                || capture.top_k > 20
                || capture.max_bytes == 0
                || capture.max_bytes > 512 * 1024 * 1024
                || !(1..=7 * 86400).contains(&capture.retain_seconds)
                || capture.targets.len() > 32
                || capture.targets.iter().any(|t| {
                    t.backend.is_empty()
                        || t.model.is_empty()
                        || t.backend.len() > 256
                        || t.model.len() > 256
                }))
        {
            return Err(invalid("capture.token_capture", "Enable capture and select top_k 0..20, at most 512 MiB, and retention 1..604800 seconds".into()));
        }

        if self.privacy.mode() == PrivacyMode::Full && self.privacy.trusted_backends.is_empty() {
            return Err(Error::ConfigInvalid {
                path: path.to_path_buf(),
                id: "privacy".to_string(),
                detail: format!(
                    "`mode = \"full\"` restricts routing to backends you name, and \
                     `trusted_backends` was set to an empty list — so nothing \
                     could be routed anywhere. Name the destinations you accept, \
                     for example: trusted_backends = [{}]. Removing the key \
                     entirely restores the default, [\"nearai\"]; an empty list \
                     is read as a deliberate choice and refused rather than \
                     quietly replaced.",
                    if self.backends.is_empty() {
                        "\"nearai\"".to_string()
                    } else {
                        self.backends
                            .iter()
                            .map(|b| format!("\"{}\"", b.id))
                            .collect::<Vec<_>>()
                            .join(", ")
                    }
                ),
            });
        }
        // Only for a list the user actually wrote. `trusted_backends` arrives
        // non-empty for everybody now (`default_trusted_backends`) and `mode`
        // is below `full` for almost everybody, so warning on the value alone
        // put a line on every command about a key nobody had touched.
        if self.privacy.mode() < PrivacyMode::Full
            && !self.privacy.trusted_backends.is_empty()
            && self.privacy.trusted_backends != default_trusted_backends()
        {
            tracing::warn!(
                "`privacy.trusted_backends` is set but `privacy.mode` is below `full`; \
                 it restricts nothing at this level"
            );
        }

        let mut seen: HashSet<&str> = HashSet::new();
        // Every login directory declared so far: (kind, where it resolves, id).
        let mut logins: HashMap<(BackendImpl, PathBuf), &str> = HashMap::new();
        for backend in &self.backends {
            // Two entries with one id means `BackendRegistry::get` answers with
            // whichever was pushed first, and the other silently does nothing.
            if !seen.insert(backend.id.as_str()) {
                return Err(invalid(
                    &backend.id,
                    "declared twice. Ids must be unique: the router looks a \
                     backend up by id, so a duplicate is silently ignored."
                        .to_string(),
                ));
            }

            let Some(kind) = BackendImpl::parse(&backend.kind) else {
                return Err(invalid(
                    &backend.id,
                    format!(
                        "`kind = \"{}\"` is not a backend IronWire can build. \
                         Valid kinds: {}.",
                        backend.kind,
                        BackendImpl::ALL.join(", ")
                    ),
                ));
            };

            // The id travels into ledger rows, event payloads and
            // `X-IronWire-Route`, so it is not free-form text.
            if !backend
                .id
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
                || backend.id.is_empty()
            {
                return Err(invalid(
                    &backend.id,
                    "ids may contain only letters, digits, `-` and `_`: this one \
                     ends up in ledger rows, event payloads and the \
                     `X-IronWire-Route` header."
                        .to_string(),
                ));
            }

            // An id discovery uses names the backend discovery builds, and an
            // entry under it only configures that backend. A different kind
            // here used to be accepted and silently ignored, which reads as
            // configuration and does nothing.
            if let Some(expected) = BackendImpl::discovered_as(&backend.id)
                && expected != kind
            {
                return Err(invalid(
                    &backend.id,
                    format!(
                        "`{id}` is the id IronWire gives {what}, which is always \
                         `kind = \"{expected}\"`; `kind = \"{actual}\"` would be \
                         ignored. Set `kind = \"{expected}\"`, or give this entry \
                         an id of its own.",
                        id = backend.id,
                        what = expected.discovered_noun(),
                        expected = expected.as_str(),
                        actual = backend.kind,
                    ),
                ));
            }

            if let Some(raw) = &backend.config_dir {
                let Some(variable) = kind.login_dir_variable() else {
                    return Err(invalid(
                        &backend.id,
                        format!(
                            "`config_dir` says where an official client keeps a \
                             login, so it applies only to `kind = \
                             \"claude-subscription\"` and `kind = \
                             \"codex-subscription\"`. A `{}` backend has no login \
                             directory; remove the line.",
                            backend.kind
                        ),
                    ));
                };
                // The daemon runs under launchd or systemd, whose working
                // directory is not the user's shell's.
                if !(raw.starts_with("~/") || Path::new(raw).is_absolute()) {
                    return Err(invalid(
                        &backend.id,
                        format!(
                            "`config_dir = \"{raw}\"` must be an absolute path or \
                             start with `~/`: IronWire runs as a background service \
                             whose working directory is not your shell's, so a \
                             relative path would point somewhere else. Use the \
                             value you export as `{variable}` for that login, for \
                             example `config_dir = \"{example}\"`.",
                            example = kind.example_config_dir(),
                        ),
                    ));
                }
                // Compared as paths, so `~/.claude-work`, its absolute
                // spelling and a trailing slash all count as one directory.
                let resolved = expand_config_dir(raw, home);
                if let Some(other) = logins.get(&(kind, resolved.clone())) {
                    return Err(invalid(
                        &backend.id,
                        format!(
                            "`config_dir = \"{raw}\"` is the same directory as the \
                             `{other}` entry's, so both would read one login: one \
                             pool shown as two, and a fallback between them that \
                             moves nothing. Remove one of the two entries."
                        ),
                    ));
                }
                // The same mistake against the default login, which has no
                // entry to compare with. On Linux both would read one
                // credentials file. On macOS they are two Keychain items, but
                // both clients keep their settings in one directory, so it
                // could only ever be wired for one of them.
                if let (Some(default_id), Some(default_dir)) =
                    (kind.discovered_id(), kind.default_config_dir())
                    && backend.id != default_id
                    && resolved == expand_config_dir(default_dir, home)
                {
                    return Err(invalid(
                        &backend.id,
                        format!(
                            "`config_dir = \"{raw}\"` is where your default \
                             {product} login lives, which IronWire already reads \
                             as `{default_id}`; a second entry for it is one login \
                             counted twice. If that login is the one you meant, \
                             remove this entry. To move the default login, set \
                             `config_dir` on it instead: `id = \"{default_id}\"`, \
                             `kind = \"{kind}\"`, `config_dir = \"{raw}\"`.",
                            product = kind.product_name(),
                            kind = kind.as_str(),
                        ),
                    ));
                }
                logins.insert((kind, resolved), &backend.id);
            } else if let Some(variable) = kind.login_dir_variable()
                && kind.discovered_id() != Some(backend.id.as_str())
            {
                // Without a directory there is nothing to tell this login
                // from the default one, and it used to be built as exactly
                // that: a second copy of the default login under the default's
                // id.
                return Err(invalid(
                    &backend.id,
                    format!(
                        "An additional {product} login needs `config_dir`: the \
                         directory you export as `{variable}` when you use it, \
                         for example `config_dir = \"{example}\"`. Without it this \
                         entry would be your default login again under another \
                         name. `ironwire connect {command} --config-dir DIR` \
                         writes the entry for you; only `{default}` means the \
                         default login.",
                        product = kind.product_name(),
                        example = kind.example_config_dir(),
                        command = kind.connect_noun(),
                        default = kind.discovered_id().unwrap_or_default(),
                    ),
                ));
            }

            // Neither kind has a host to fall back on: there is no canonical
            // address for "some OpenAI-compatible server", and building one
            // against nothing yields a backend that fails every request.
            if matches!(kind, BackendImpl::OpenAiCompatible | BackendImpl::Local) {
                if backend.base_url.is_none() {
                    return Err(invalid(
                        &backend.id,
                        format!(
                            "`kind = \"{}\"` needs a `base_url`; there is no \
                             default host to assume. For a local server it must \
                             include the OpenAI-compatible prefix, usually \
                             `/v1` — Ollama's native `/api/*` is a different \
                             protocol and is not supported.",
                            backend.kind
                        ),
                    ));
                }
                // A key is required for a hosted endpoint and optional for a
                // local one: most local servers take no auth at all, and
                // demanding a variable that holds nothing would be ceremony.
                if kind == BackendImpl::OpenAiCompatible && backend.api_key_env.is_none() {
                    return Err(invalid(
                        &backend.id,
                        "`kind = \"openai-compatible\"` needs an `api_key_env` \
                         naming the environment variable that holds its key. \
                         IronWire keeps no secrets in config.toml."
                            .to_string(),
                    ));
                }
            }
        }
        Ok(())
    }
}

/// The backend implementations a `[[backends]]` entry can name.
///
/// Distinct from [`crate::protocol::BackendKind`], which is the *capacity*
/// class a backend draws on — subscription, key, credits, local. Several impls
/// here map to one capacity class and vice versa.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BackendImpl {
    /// Claude Code's stored subscription credential: the default login, or
    /// the one in the entry's `config_dir`. One entry per login.
    ClaudeSubscription,
    /// The metered Anthropic API.
    AnthropicApi,
    /// Codex's stored ChatGPT credential: the default login, or the one in
    /// the entry's `config_dir`. One entry per login.
    CodexSubscription,
    /// The metered OpenAI API.
    OpenAiApi,
    /// NEAR AI credits.
    NearAi,
    /// Any other endpoint speaking OpenAI Chat Completions.
    OpenAiCompatible,
    /// A model server on this machine or LAN. Free at the margin, and usually
    /// unauthenticated, which is what separates it from `openai-compatible`.
    Local,
}

impl BackendImpl {
    /// Every accepted spelling, for the error message that lists them.
    pub const ALL: &'static [&'static str] = &[
        "claude-subscription",
        "anthropic-api",
        "codex-subscription",
        "openai-api",
        "nearai",
        "openai-compatible",
        "local",
    ];

    /// Parse the `kind` field. `None` for anything not in [`Self::ALL`].
    #[must_use]
    pub fn parse(kind: &str) -> Option<Self> {
        match kind {
            "claude-subscription" => Some(Self::ClaudeSubscription),
            "anthropic-api" => Some(Self::AnthropicApi),
            "codex-subscription" => Some(Self::CodexSubscription),
            "openai-api" => Some(Self::OpenAiApi),
            "nearai" => Some(Self::NearAi),
            "openai-compatible" => Some(Self::OpenAiCompatible),
            "local" => Some(Self::Local),
            _ => None,
        }
    }

    /// The `kind` spelling, as [`Self::parse`] accepts it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudeSubscription => "claude-subscription",
            Self::AnthropicApi => "anthropic-api",
            Self::CodexSubscription => "codex-subscription",
            Self::OpenAiApi => "openai-api",
            Self::NearAi => "nearai",
            Self::OpenAiCompatible => "openai-compatible",
            Self::Local => "local",
        }
    }

    /// Whether this reads a login an official client stored.
    #[must_use]
    pub fn is_subscription(self) -> bool {
        matches!(self, Self::ClaudeSubscription | Self::CodexSubscription)
    }

    /// The id IronWire's own discovery registers this implementation under,
    /// if it discovers one at all.
    ///
    /// For a subscription that is the *default* login's id, which never
    /// changes: consent, persisted quota, pins, `trusted_backends`, spend caps
    /// and ledger history are all keyed on it.
    #[must_use]
    pub fn discovered_id(self) -> Option<&'static str> {
        match self {
            Self::ClaudeSubscription => Some("claude-sub"),
            Self::AnthropicApi => Some("anthropic-key"),
            Self::CodexSubscription => Some("codex-sub"),
            Self::OpenAiApi => Some("openai-key"),
            Self::NearAi => Some("nearai"),
            Self::OpenAiCompatible | Self::Local => None,
        }
    }

    /// The implementation discovery builds under `id`, if `id` is one of
    /// discovery's.
    #[must_use]
    pub fn discovered_as(id: &str) -> Option<Self> {
        [
            Self::ClaudeSubscription,
            Self::AnthropicApi,
            Self::CodexSubscription,
            Self::OpenAiApi,
            Self::NearAi,
        ]
        .into_iter()
        .find(|kind| kind.discovered_id() == Some(id))
    }

    /// The environment variable a user sets to put this client's login
    /// somewhere other than its default. `None` for anything that is not a
    /// subscription, which is also what says `config_dir` does not apply.
    #[must_use]
    pub fn login_dir_variable(self) -> Option<&'static str> {
        match self {
            Self::ClaudeSubscription => Some("CLAUDE_CONFIG_DIR"),
            Self::CodexSubscription => Some("CODEX_HOME"),
            _ => None,
        }
    }

    /// What discovery's backend of this kind is, for a sentence.
    fn discovered_noun(self) -> &'static str {
        match self {
            Self::ClaudeSubscription => "your default Claude login",
            Self::CodexSubscription => "your default ChatGPT login",
            Self::AnthropicApi => "the Anthropic API key it finds",
            Self::OpenAiApi => "the OpenAI API key it finds",
            Self::NearAi => "your NEAR AI account",
            Self::OpenAiCompatible | Self::Local => "no backend",
        }
    }

    fn product_name(self) -> &'static str {
        match self {
            Self::CodexSubscription => "ChatGPT",
            _ => "Claude",
        }
    }

    fn connect_noun(self) -> &'static str {
        match self {
            Self::CodexSubscription => "codex",
            _ => "claude",
        }
    }

    fn example_config_dir(self) -> &'static str {
        match self {
            Self::CodexSubscription => "~/.codex-work",
            _ => "~/.claude-work",
        }
    }

    /// Where the official client keeps its login when nothing relocates it.
    ///
    /// For Codex the daemon honours its own `CODEX_HOME`, which validation
    /// does not read; `~/.codex` is the default every user without one has.
    fn default_config_dir(self) -> Option<&'static str> {
        match self {
            Self::ClaudeSubscription => Some("~/.claude"),
            Self::CodexSubscription => Some("~/.codex"),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load(toml: &str) -> Result<Config> {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, toml).expect("write");
        Config::load_from(&path)
    }

    /// The error has to name the entry. "invalid config" against a file with
    /// five backends in it is a scavenger hunt.
    fn rejection(toml: &str) -> String {
        match load(toml) {
            Err(error) => error.to_string(),
            Ok(_) => panic!("accepted a config that cannot describe a backend:\n{toml}"),
        }
    }

    #[test]
    fn an_unknown_kind_is_refused_with_the_valid_ones_listed() {
        let message = rejection(
            r#"
            [[backends]]
            id = "typo"
            kind = "anthropic"
            "#,
        );
        assert!(message.contains("typo"), "{message}");
        assert!(message.contains("anthropic-api"), "{message}");
    }

    /// `BackendRegistry::get` answers with the first match, so the second entry
    /// would do nothing at all while looking like configuration.
    #[test]
    fn a_duplicate_id_is_refused() {
        let message = rejection(
            r#"
            [[backends]]
            id = "nearai"
            kind = "nearai"

            [[backends]]
            id = "nearai"
            kind = "nearai"
            "#,
        );
        assert!(message.contains("nearai"), "{message}");
    }

    #[test]
    fn an_openai_compatible_backend_needs_somewhere_to_send_requests() {
        let message = rejection(
            r#"
            [[backends]]
            id = "local"
            kind = "openai-compatible"
            api_key_env = "LOCAL_KEY"
            "#,
        );
        assert!(message.contains("base_url"), "{message}");
    }

    #[test]
    fn an_openai_compatible_backend_needs_a_named_key_variable() {
        let message = rejection(
            r#"
            [[backends]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://127.0.0.1:11434/v1"
            "#,
        );
        assert!(message.contains("api_key_env"), "{message}");
    }

    #[test]
    fn a_well_formed_backend_entry_is_accepted() {
        let config = load(
            r#"
            [[backends]]
            id = "local"
            kind = "openai-compatible"
            base_url = "http://127.0.0.1:11434/v1"
            api_key_env = "LOCAL_KEY"
            models = ["qwen3-coder"]

            [[backends]]
            id = "anthropic-key"
            kind = "anthropic-api"
            enabled = false
            "#,
        )
        .expect("valid");
        assert_eq!(config.backends.len(), 2);
        assert!(!config.backends[1].enabled);
    }

    /// Validation runs on a file that exists. A fresh install has none, and
    /// must still start.
    #[test]
    fn a_config_with_no_backends_block_is_valid() {
        assert!(load("[server]\nport = 8463\n").is_ok());
    }

    /// The rule the whole local-backend feature rests on. `from_model_hint`
    /// resolves an unrecognised slug to `Frontier`, and local capacity sorts
    /// cheapest, so a bare name on a local backend must not inherit that.
    #[test]
    fn a_bare_slug_on_local_capacity_is_fast_not_frontier() {
        let entry = ModelEntry::Name("qwen3-coder:30b".to_string());
        assert_eq!(entry.tier_on(BackendKind::Local), ModelTier::Fast);
        // The same slug on hosted capacity keeps the cautious default.
        assert_eq!(entry.tier_on(BackendKind::Credits), ModelTier::Frontier);
    }

    /// The user opts a local model up the ladder; IronWire never does.
    #[test]
    fn a_declared_tier_wins_everywhere() {
        let entry = ModelEntry::Tiered {
            name: "llama3.3:70b".to_string(),
            tier: ModelTier::Balanced,
        };
        assert_eq!(entry.tier_on(BackendKind::Local), ModelTier::Balanced);
        assert_eq!(entry.tier_on(BackendKind::ApiKey), ModelTier::Balanced);
    }

    #[test]
    fn a_models_list_accepts_both_spellings() {
        let config = load(
            r#"
            [[backends]]
            id = "ollama"
            kind = "local"
            base_url = "http://127.0.0.1:11434/v1"
            models = ["qwen3-coder:30b", { name = "llama3.3:70b", tier = "balanced" }]
            "#,
        )
        .expect("valid");
        let models = config.backends[0].models.as_ref().expect("declared");
        assert_eq!(models[0].name(), "qwen3-coder:30b");
        assert_eq!(models[1].tier_on(BackendKind::Local), ModelTier::Balanced);
    }

    /// Local servers usually take no auth, so requiring a key variable would be
    /// ceremony — but a host is still required, and the message says why.
    #[test]
    fn a_local_backend_needs_a_base_url_but_not_a_key() {
        assert!(
            load(
                r#"
                [[backends]]
                id = "ollama"
                kind = "local"
                base_url = "http://127.0.0.1:11434/v1"
                "#,
            )
            .is_ok()
        );
        let message = rejection(
            r#"
            [[backends]]
            id = "ollama"
            kind = "local"
            "#,
        );
        assert!(message.contains("base_url"), "{message}");
        assert!(
            message.contains("/v1"),
            "the fix must be spelled out: {message}"
        );
    }

    /// The id reaches ledger rows, event payloads and `X-IronWire-Route`.
    #[test]
    fn an_id_is_restricted_to_a_conservative_character_set() {
        for bad in ["has space", "quote\"d", "semi;colon", ""] {
            let toml = format!("[[backends]]\nid = \"{bad}\"\nkind = \"nearai\"\n");
            assert!(
                Config::load_from(&{
                    let dir = tempfile::tempdir().expect("tempdir");
                    let path = dir.path().join("config.toml");
                    std::fs::write(&path, &toml).expect("write");
                    std::mem::forget(dir);
                    path
                })
                .is_err(),
                "accepted id {bad:?}"
            );
        }
    }

    /// A `[limits]` block with nothing in it is not a cap, and must not switch
    /// on the machinery — including the startup refusal that a real cap
    /// triggers when the ledger is off.
    #[test]
    fn an_empty_limits_block_is_not_a_cap() {
        assert!(!LimitsConfig::default().any_cap());
        assert!(
            !LimitsConfig {
                daily_spend_usd: Some(0.0),
                ..LimitsConfig::default()
            }
            .any_cap()
        );
        assert!(
            LimitsConfig {
                daily_spend_usd: Some(10.0),
                ..LimitsConfig::default()
            }
            .any_cap()
        );
    }

    /// A per-backend cap and a global cap both apply; whichever binds first
    /// wins, because the user meant both.
    #[test]
    fn the_tighter_of_two_caps_binds() {
        let limits = LimitsConfig {
            daily_spend_usd: Some(10.0),
            on_breach: BreachAction::Descend,
            backends: vec![BackendLimit {
                id: "anthropic-key".into(),
                daily_spend_usd: 5.0,
            }],
        };
        assert_eq!(limits.cap_for("anthropic-key"), Some(5.0));
        assert_eq!(
            limits.cap_for("openai-key"),
            Some(10.0),
            "a backend with no cap of its own still counts against the global one"
        );
        assert_eq!(
            LimitsConfig::default().cap_for("anthropic-key"),
            None,
            "no cap configured is not a cap of zero"
        );
    }

    #[test]
    fn descend_is_the_default_breach_action() {
        // The product's promise is that the agent does not die. A cap that
        // killed the session by default would invert it.
        assert_eq!(LimitsConfig::default().on_breach, BreachAction::Descend);
    }

    /// Every row of the upgrade table. `deny_unknown_fields` means an existing
    /// `config.toml` that still says `enabled`/`secrets` has to keep loading,
    /// and has to keep filtering exactly what it filtered before.
    #[test]
    fn a_config_predating_the_ladder_keeps_its_behaviour() {
        let old_style = |toml: &str| -> PrivacyConfig {
            load(toml)
                .expect("an existing config must still load")
                .privacy
        };

        assert_eq!(
            old_style("[privacy]\nenabled = true\nsecrets = true\n").mode(),
            PrivacyMode::Credentials
        );
        assert_eq!(
            old_style("[privacy]\nenabled = true\nsecrets = false\nnamed_values = [\"acme\"]\n")
                .mode(),
            PrivacyMode::Credentials,
            "named values alone were a working configuration and must stay one"
        );
        assert_eq!(
            old_style("[privacy]\nenabled = false\n").mode(),
            PrivacyMode::Off
        );
        assert_eq!(
            old_style("[privacy]\nenabled = true\nsecrets = false\n").mode(),
            PrivacyMode::Off,
            "on with nothing to match reads as off, not as protection"
        );
        assert_eq!(
            PrivacyConfig::default().mode(),
            PrivacyMode::Off,
            "the default is off, and that is a commitment rather than a default"
        );
    }

    /// `mode` wins outright, so a user who adopts the ladder is not silently
    /// still governed by a boolean they left behind.
    #[test]
    fn the_mode_overrides_the_deprecated_booleans() {
        let config = load("[privacy]\nmode = \"pii\"\nenabled = false\nsecrets = false\n")
            .expect("loads")
            .privacy;
        assert_eq!(config.mode(), PrivacyMode::Pii);
        assert!(config.is_active());
    }

    #[test]
    fn the_modes_are_ordered_as_a_ladder() {
        assert!(PrivacyMode::Off < PrivacyMode::Credentials);
        assert!(PrivacyMode::Credentials < PrivacyMode::Pii);
        assert!(PrivacyMode::Pii < PrivacyMode::Full);
    }

    /// A round trip must not reintroduce the deprecated keys, or writing a
    /// config back would undo the upgrade.
    #[test]
    fn a_round_trip_does_not_resurrect_the_old_switches() {
        let config = Config {
            privacy: PrivacyConfig {
                mode: Some(PrivacyMode::Pii),
                ..PrivacyConfig::default()
            },
            ..Config::default()
        };
        let text = toml::to_string(&config).expect("serializes");
        let privacy = text
            .split("[privacy]")
            .nth(1)
            .expect("a privacy section")
            .to_string();
        assert!(privacy.contains("mode = \"pii\""), "{privacy}");
        assert!(
            !privacy.contains("enabled") && !privacy.contains("secrets"),
            "the deprecated switches came back on a round trip:\n{privacy}"
        );
    }

    #[test]
    fn every_advertised_kind_parses() {
        for kind in BackendImpl::ALL {
            assert!(
                BackendImpl::parse(kind).is_some(),
                "`{kind}` is advertised but not accepted"
            );
        }
        assert!(BackendImpl::parse("not-a-kind").is_none());
    }

    /// Every spelling a message names must be one `parse` accepts.
    #[test]
    fn every_kind_names_itself_the_way_it_is_parsed() {
        for kind in BackendImpl::ALL {
            let parsed = BackendImpl::parse(kind).expect("advertised");
            assert_eq!(parsed.as_str(), *kind);
        }
    }

    /// The default logins keep the ids everything persisted is keyed on.
    #[test]
    fn discovery_ids_and_kinds_map_both_ways() {
        for (id, kind) in [
            ("claude-sub", BackendImpl::ClaudeSubscription),
            ("codex-sub", BackendImpl::CodexSubscription),
            ("anthropic-key", BackendImpl::AnthropicApi),
            ("openai-key", BackendImpl::OpenAiApi),
            ("nearai", BackendImpl::NearAi),
        ] {
            assert_eq!(BackendImpl::discovered_as(id), Some(kind));
            assert_eq!(kind.discovered_id(), Some(id));
        }
        assert_eq!(BackendImpl::discovered_as("claude-work"), None);
        assert_eq!(BackendImpl::Local.discovered_id(), None);
    }

    /// Cross-account fallback is an opt-in, with the terms risk stated where
    /// it is turned on. A config that never mentions it must not have it.
    #[test]
    fn cross_account_fallback_is_off_unless_the_user_turns_it_on() {
        assert!(!Config::default().subscriptions.cross_account_fallback);
        assert!(
            !load("[server]\nport = 8463\n")
                .expect("valid")
                .subscriptions
                .cross_account_fallback
        );
        assert!(
            load("[subscriptions]\ncross_account_fallback = true\n")
                .expect("valid")
                .subscriptions
                .cross_account_fallback
        );
        assert!(
            toml::from_str::<Config>("[subscriptions]\ncross_acount_fallback = true\n").is_err(),
            "a misspelled opt-in must not silently stay off"
        );
    }

    #[test]
    fn an_additional_login_entry_is_accepted() {
        let config = load(
            r#"
            [[backends]]
            id = "claude-work"
            kind = "claude-subscription"
            config_dir = "~/.claude-work"

            [[backends]]
            id = "codex-work"
            kind = "codex-subscription"
            config_dir = "/Users/alice/.codex-work"
            "#,
        )
        .expect("an additional login with its directory is valid");
        assert_eq!(
            config.backends[0].config_dir.as_deref(),
            Some("~/.claude-work"),
            "kept as written: the Keychain item is keyed on the exact string"
        );
    }

    /// Rule 1: only a subscription has a login directory.
    #[test]
    fn a_config_dir_on_a_backend_without_a_login_is_refused() {
        let message = rejection(
            r#"
            [[backends]]
            id = "anthropic-key"
            kind = "anthropic-api"
            config_dir = "~/.claude-work"
            "#,
        );
        assert!(message.contains("anthropic-key"), "{message}");
        assert!(message.contains("config_dir"), "{message}");
        assert!(message.contains("remove the line"), "{message}");
    }

    /// Rule 2: the daemon's working directory is not the user's.
    #[test]
    fn a_relative_config_dir_is_refused() {
        for relative in [".claude-work", "work/.claude", "~", "~alice/.claude", ""] {
            let message = rejection(&format!(
                "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
                 config_dir = \"{relative}\"\n"
            ));
            assert!(message.contains("claude-work"), "{message}");
            assert!(message.contains("~/"), "{message}");
            assert!(message.contains("CLAUDE_CONFIG_DIR"), "{message}");
        }
    }

    /// Rule 3, and the bug it closes: such an entry used to build a second
    /// copy of the *default* login under the default's id.
    #[test]
    fn an_additional_login_without_its_directory_is_refused() {
        let claude = rejection(
            r#"
            [[backends]]
            id = "claude-work"
            kind = "claude-subscription"
            "#,
        );
        assert!(claude.contains("claude-work"), "{claude}");
        assert!(claude.contains("CLAUDE_CONFIG_DIR"), "{claude}");
        assert!(
            claude.contains("ironwire connect claude --config-dir"),
            "{claude}"
        );

        let codex = rejection(
            r#"
            [[backends]]
            id = "codex-work"
            kind = "codex-subscription"
            enabled = false
            "#,
        );
        assert!(codex.contains("codex-work"), "{codex}");
        assert!(codex.contains("CODEX_HOME"), "{codex}");
        assert!(
            codex.contains("ironwire connect codex --config-dir"),
            "{codex}"
        );
    }

    /// Rule 4: an id discovery uses names discovery's backend, so a different
    /// kind under it would be silently ignored.
    #[test]
    fn a_discovered_id_with_another_kind_is_refused() {
        for (id, kind, expected) in [
            ("claude-sub", "anthropic-api", "claude-subscription"),
            ("codex-sub", "claude-subscription", "codex-subscription"),
            ("anthropic-key", "claude-subscription", "anthropic-api"),
            ("openai-key", "nearai", "openai-api"),
            ("nearai", "openai-api", "nearai"),
        ] {
            let message = rejection(&format!(
                "[[backends]]\nid = \"{id}\"\nkind = \"{kind}\"\nconfig_dir = \"~/.x\"\n"
            ));
            assert!(message.contains(&format!("`{id}`")), "{message}");
            assert!(
                message.contains(&format!("kind = \"{expected}\"")),
                "the fix must be spelled out: {message}"
            );
        }
    }

    /// Rule 5: the default login may be relocated, and needs nothing else.
    #[test]
    fn the_default_login_may_be_relocated_or_left_alone() {
        let config = load(
            r#"
            [[backends]]
            id = "claude-sub"
            kind = "claude-subscription"
            config_dir = "~/.claude-personal"

            [[backends]]
            id = "codex-sub"
            kind = "codex-subscription"
            "#,
        )
        .expect("valid");
        assert_eq!(
            config.backends[0].config_dir.as_deref(),
            Some("~/.claude-personal")
        );
        assert_eq!(config.backends[1].config_dir, None);
    }

    /// Rule 6: one login counted twice is one pool shown as two.
    #[test]
    fn two_entries_for_one_login_directory_are_refused() {
        let home = Path::new("/Users/alice");
        let config_with = |first: &str, second: &str| -> Config {
            toml::from_str(&format!(
                "[[backends]]\nid = \"claude-a\"\nkind = \"claude-subscription\"\n\
                 config_dir = \"{first}\"\n\
                 [[backends]]\nid = \"claude-b\"\nkind = \"claude-subscription\"\n\
                 config_dir = \"{second}\"\n"
            ))
            .expect("parses")
        };
        for (first, second) in [
            ("~/.claude-work", "~/.claude-work"),
            ("~/.claude-work", "/Users/alice/.claude-work"),
            ("/Users/alice/.claude-work/", "~/.claude-work"),
        ] {
            let message = config_with(first, second)
                .validate_with_home(Path::new("config.toml"), home)
                .expect_err("one login, two entries")
                .to_string();
            assert!(message.contains("claude-b"), "{message}");
            assert!(message.contains("claude-a"), "{message}");
        }
        // Different directories are different logins.
        config_with("~/.claude-work", "~/.claude-personal")
            .validate_with_home(Path::new("config.toml"), home)
            .expect("two logins");

        // The same directory under two *products* is two logins: Claude Code
        // and Codex never read each other's files.
        let mixed: Config = toml::from_str(
            "[[backends]]\nid = \"claude-a\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.work\"\n\
             [[backends]]\nid = \"codex-a\"\nkind = \"codex-subscription\"\n\
             config_dir = \"~/.work\"\n",
        )
        .expect("parses");
        mixed
            .validate_with_home(Path::new("config.toml"), home)
            .expect("one directory, two products");
    }

    /// The expansion reproduces what a shell hands the client, character for
    /// character, because Claude Code hashes that string to name its Keychain
    /// item.
    #[test]
    fn a_config_dir_expands_exactly_as_a_shell_would() {
        let home = Path::new("/Users/alice");
        assert_eq!(
            expand_config_dir("~/.claude-work", home),
            Path::new("/Users/alice/.claude-work")
        );
        assert_eq!(
            expand_config_dir("~/.claude-work/", home)
                .to_string_lossy()
                .as_ref(),
            "/Users/alice/.claude-work/",
            "a trailing slash is a different Keychain item and must survive"
        );
        assert_eq!(
            expand_config_dir("/opt/claude", home),
            Path::new("/opt/claude")
        );
        assert_eq!(expand_config_dir("~", home), home);
        assert_eq!(
            expand_config_dir("~alice/x", home),
            Path::new("~alice/x"),
            "only the current user's home is expanded"
        );
    }

    /// `~//x` is what a shell makes of `~/x` under a `HOME` ending in `/`, and
    /// what the CLI stores for `$HOME//x`. It is still under the home; joining
    /// instead of concatenating read the rest as absolute and dropped it.
    #[test]
    fn a_doubled_slash_after_the_tilde_keeps_the_home() {
        let home = Path::new("/Users/alice");
        assert_eq!(
            expand_config_dir("~//.claude-work", home)
                .to_string_lossy()
                .as_ref(),
            "/Users/alice//.claude-work",
            "the same string the credential side hashes"
        );
        assert_eq!(
            expand_config_dir("~/.claude-work", Path::new("/Users/alice/"))
                .to_string_lossy()
                .as_ref(),
            "/Users/alice//.claude-work"
        );
    }

    /// Rule 7: an additional entry for the default login's own directory is
    /// that login counted twice; moving the default is done on its own entry.
    #[test]
    fn an_additional_login_in_the_default_directory_is_refused_in_favour_of_relocating_the_default()
    {
        let home = Path::new("/Users/alice");
        let validate = |text: String| {
            toml::from_str::<Config>(&text)
                .expect("parses")
                .validate_with_home(Path::new("config.toml"), home)
        };
        for (id, kind, dir, default_id) in [
            (
                "claude-main",
                "claude-subscription",
                "~/.claude",
                "claude-sub",
            ),
            (
                "claude-main",
                "claude-subscription",
                "~/.claude/",
                "claude-sub",
            ),
            (
                "claude-main",
                "claude-subscription",
                "/Users/alice/.claude",
                "claude-sub",
            ),
            ("codex-main", "codex-subscription", "~/.codex", "codex-sub"),
        ] {
            let message = validate(format!(
                "[[backends]]\nid = \"{id}\"\nkind = \"{kind}\"\nconfig_dir = \"{dir}\"\n"
            ))
            .expect_err("the default login, declared again")
            .to_string();
            assert!(message.contains(id), "{message}");
            assert!(message.contains(&format!("`{default_id}`")), "{message}");
            assert!(
                message.contains(&format!("`id = \"{default_id}\"`")),
                "the fix must be spelled out: {message}"
            );
        }

        // Relocating the default login onto it is not a second login.
        validate(
            "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude\"\n"
                .to_string(),
        )
        .expect("the default login, in its own directory");
        // Nor is a directory that merely starts the same way.
        validate(
            "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-work\"\n"
                .to_string(),
        )
        .expect("an additional login");
        // And the other product's default directory is not this one's.
        validate(
            "[[backends]]\nid = \"codex-claude\"\nkind = \"codex-subscription\"\n\
             config_dir = \"~/.claude\"\n"
                .to_string(),
        )
        .expect("Codex never reads Claude Code's directory");
    }

    #[test]
    fn a_missing_config_yields_defaults_rather_than_failing() {
        let cfg = Config::load_from(Path::new("/nonexistent/ironwire/config.toml"))
            .expect("a fresh install must start");
        assert_eq!(cfg.server.port, DEFAULT_PORT);
        assert!(cfg.capture.enabled);
        assert!(!cfg.capture.bodies, "bodies contain user source code");
    }

    /// Checking receipts is a network call the exchange itself did not make,
    /// so it is something a user turns on, not something they find on.
    #[test]
    fn receipt_checks_are_off_by_default() {
        assert!(!Config::default().capture.receipts);
        let cfg = load("[capture]\nreceipts = true\n").expect("parses");
        assert!(cfg.capture.receipts);
    }

    #[test]
    fn config_round_trips() {
        let cfg = Config {
            server: ServerConfig {
                port: 9000,
                upstream_timeout_secs: 60,
            },
            capture: CaptureConfig {
                enabled: true,
                bodies: true,
                retain_days: 30,
                logprobs: true,
                token_capture: None,
                receipts: true,
            },
            usage: UsageConfig {
                enabled: true,
                session_hours: 5,
                history_hours: 192,
                plan: Some("max5".into()),
            },
            updates: UpdateConfig { check: false },
            resilience: ResilienceConfig::default(),
            privacy: PrivacyConfig::default(),
            backends: vec![
                BackendConfig {
                    id: "claude-sub".into(),
                    kind: "claude-subscription".into(),
                    enabled: true,
                    base_url: None,
                    api_key_env: None,
                    config_dir: None,
                    refuse_model_aliases: false,
                    models: None,
                },
                BackendConfig {
                    id: "claude-work".into(),
                    kind: "claude-subscription".into(),
                    enabled: true,
                    base_url: None,
                    api_key_env: None,
                    config_dir: Some("~/.claude-work".into()),
                    refuse_model_aliases: false,
                    models: None,
                },
            ],
            limits: LimitsConfig {
                daily_spend_usd: Some(10.0),
                on_breach: BreachAction::Refuse,
                backends: vec![BackendLimit {
                    id: "anthropic-key".into(),
                    daily_spend_usd: 5.0,
                }],
            },
            subscriptions: SubscriptionsConfig {
                cross_account_fallback: true,
            },
        };
        let text = toml::to_string(&cfg).expect("serializes");
        let back: Config = toml::from_str(&text).expect("deserializes");
        assert_eq!(cfg, back);
        // And what was written is something `load` accepts, not merely
        // something serde can read back.
        back.validate(Path::new("config.toml"))
            .expect("a round-tripped config still validates");
    }

    /// `capture.logprobs` was added after the first release. A config written
    /// before it must still load, and must not quietly start asking a provider
    /// for something the user never opted into.
    #[test]
    fn a_config_written_before_logprobs_existed_still_loads_with_it_off() {
        let cfg: Config = toml::from_str("[capture]\nenabled = true\nbodies = false\n")
            .expect("an older config still deserializes");
        assert!(!cfg.capture.logprobs);
    }

    #[test]
    fn unknown_config_keys_are_rejected_not_ignored() {
        // A typo in a routing knob must not silently do nothing.
        let err = toml::from_str::<Config>("[server]\nprot = 1234\n");
        assert!(err.is_err());
    }

    #[test]
    fn paths_hang_off_one_root() {
        let paths = PathsConfig::rooted_at("/tmp/iw-test");
        assert_eq!(paths.config_file(), Path::new("/tmp/iw-test/config.toml"));
        assert_eq!(paths.lock_file(), Path::new("/tmp/iw-test/daemon.lock"));
        assert_eq!(
            paths.control_token_file(),
            Path::new("/tmp/iw-test/control.token")
        );
    }
}

#[cfg(test)]
mod bulk_config_tests {
    use super::*;

    #[test]
    fn ten_thousand_distinct_logins_validate_and_a_late_duplicate_is_rejected() {
        let mut config = Config {
            backends: (0..10_000)
                .map(|index| BackendConfig {
                    id: format!("claude-{index}"),
                    kind: "claude-subscription".into(),
                    enabled: true,
                    base_url: None,
                    api_key_env: None,
                    config_dir: Some(format!("/fixtures/login-{index}")),
                    refuse_model_aliases: false,
                    models: None,
                })
                .collect(),
            ..Config::default()
        };
        let path = Path::new("/fixtures/config.toml");
        let home = Path::new("/fixtures/home");
        config
            .validate_with_home(path, home)
            .expect("all logins distinct");
        let mut duplicate = config.backends[0].clone();
        duplicate.id = "claude-extra".into();
        config.backends.push(duplicate);
        let error = config
            .validate_with_home(path, home)
            .expect_err("duplicate directory");
        assert!(error.to_string().contains("same directory"), "{error}");
        config.backends.pop();
        config.backends.push(config.backends[0].clone());
        let error = config
            .validate_with_home(path, home)
            .expect_err("duplicate id");
        assert!(error.to_string().contains("declared twice"), "{error}");
    }
}

#[cfg(test)]
mod resilience_tests {
    use super::ResilienceConfig;

    #[test]
    fn a_compaction_turn_gets_the_longer_patience() {
        let config = ResilienceConfig::default();
        assert!(config.stall_timeout_for(true) > config.stall_timeout_for(false));
    }

    #[test]
    fn an_ordinary_turn_is_unaffected() {
        let config = ResilienceConfig::default();
        assert_eq!(config.stall_timeout_for(false), config.stall_timeout_secs);
    }

    #[test]
    fn a_misconfigured_compaction_timeout_never_makes_compaction_more_fragile() {
        // Someone lowering `compaction_stall_timeout_secs` below the ordinary
        // one would get the exact opposite of what the setting is for, and the
        // symptom — abandoned compaction turns in long sessions — is very hard
        // to trace back to a config value.
        let config = ResilienceConfig {
            stall_timeout_secs: 180,
            compaction_stall_timeout_secs: 10,
            ..ResilienceConfig::default()
        };
        assert_eq!(config.stall_timeout_for(true), 180);
    }

    #[test]
    fn a_local_turn_can_be_given_more_patience() {
        let config = ResilienceConfig {
            stall_timeout_secs: 180,
            local_stall_timeout_secs: 900,
            ..ResilienceConfig::default()
        };
        assert_eq!(config.stall_timeout_for_backend(false, true), 900);
        assert_eq!(
            config.stall_timeout_for_backend(false, false),
            180,
            "a hosted turn keeps the ordinary timeout"
        );
    }

    /// The same trap as the compaction floor: a local model is the slowest
    /// thing IronWire routes to, so a value below the ordinary timeout would
    /// make the backend that needs the most patience the least patiently
    /// treated.
    #[test]
    fn a_misconfigured_local_timeout_never_makes_a_local_turn_more_fragile() {
        let config = ResilienceConfig {
            stall_timeout_secs: 180,
            local_stall_timeout_secs: 10,
            ..ResilienceConfig::default()
        };
        assert_eq!(config.stall_timeout_for_backend(false, true), 180);
    }

    /// A compaction turn on local capacity gets the longer of the two, not
    /// whichever rule was consulted last.
    #[test]
    fn a_local_compaction_turn_gets_the_longest_patience() {
        let config = ResilienceConfig {
            stall_timeout_secs: 180,
            compaction_stall_timeout_secs: 600,
            local_stall_timeout_secs: 300,
            ..ResilienceConfig::default()
        };
        assert_eq!(config.stall_timeout_for_backend(true, true), 600);
    }
}
