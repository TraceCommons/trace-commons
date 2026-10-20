//! The catalog schema — and, more importantly, what it deliberately cannot say.
//!
//! # The security property
//!
//! **The document may say what to set, never what to set it to.**
//!
//! That is the whole design. A remotely-updatable document that could name a
//! host would let whoever controls the signing key redirect a subscription
//! token to a server of their choosing — turning our own update channel into
//! an exfiltration path and voiding `docs/TRUST.md` I2.
//!
//! Until agents were added this was stated as "no type here can express a host,
//! a URL, or a path", and enforced by a test that walked the serialised document
//! banning field *names* containing `url`, `host`, `path`. That was a proxy for
//! the real property, and it stopped being usable the moment the document had to
//! describe where a tool keeps its config. The rule that replaced it is stricter
//! about the thing that actually matters:
//!
//! - **Values are unrepresentable.** [`AgentSetting`] carries a [`Facade`] and
//!   a [`UrlStyle`], not a string. Both are closed sets of this binary's own
//!   addresses: the scheme, host, port, façade path and version suffix are all
//!   compiled in, and the document only chooses among them. There is no variant
//!   that takes a literal, so "point Claude Code at evil.example" cannot be
//!   written down, let alone signed.
//! - **Locations are constrained, not free.** [`ConfigLocation`] is a dotdir
//!   under the user's home plus a `.json` or `.toml` file, with `.` and `..`
//!   refused and separators outside the charset. The worst a compromised key
//!   can do is write our own loopback URL into some other dotfile of the user's.
//! - **Provider constants still name nothing.** `anthropic`, `client_identity`
//!   and `models` carry wire strings only, and the name-walk test still runs
//!   over that subtree.
//!
//! Base URLs, credential file locations, and the credential→host binding stay
//! compiled into the binary and change only through a release.
//!
//! ## Review rule
//!
//! Unknown fields are **allowed** (forward compatibility — an old binary must
//! tolerate a newer document). The security property is therefore not "the
//! document contains no host", it is "*this binary's types* cannot read one, so
//! a host in the document is inert". Adding a field that carries a value for a
//! location — or widening [`ConfigLocation`] — is a `TRUST.md` change, not a
//! schema change.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Schema version this binary understands.
///
/// A document declaring a *newer* major schema is refused rather than
/// partially applied — half-understanding a provider workaround is worse than
/// using the compiled-in defaults.
///
/// # 2: [`AgentSetting::style`]
///
/// Unknown *fields* are tolerated, so a build from before that field existed
/// reads an entry asking for [`UrlStyle::Versioned`], ignores the field, and
/// writes the origin spelling into a tool that needs `/v1` — a 404 in the
/// user's editor, arrived at by following the document correctly. This number
/// is the only way to say "not this build": a publisher stamps `2` on a
/// document that uses `style`, and every earlier build refuses the whole
/// document instead of applying half of it.
///
/// The refusal is document-wide, which is the cost: an old build that rejects
/// the document also stops receiving `anthropic` and `client_identity`, the
/// constants this channel exists to refresh. That is a publishing decision, not
/// a code one — a document that uses no `style` should keep declaring `1` and
/// keep reaching every build. What the bump buys is the *option*, and it has to
/// ship with the field: a build that does not accept `2` can never be talked
/// into it later, so a release that understands `style` without accepting `2`
/// leaves a publisher no way to use the field safely for as long as that
/// release is in the field.
/// # 3: [`AgentSetting::requires`]
///
/// The same trap in the same shape, and worse in its consequence. A build from
/// before that field existed reads an entry that says "set this key only where
/// the file already shows the tool using this provider", ignores the condition,
/// and sets the key unconditionally. For the tool the field was added for that
/// is not a 404 — it is a working tool that stops working, because writing the
/// key is what changes which provider it chose. An old build must refuse the
/// document rather than apply the half of the entry it understands.
pub const SCHEMA_VERSION: u32 = 3;

/// Provider values that move faster than our release cadence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Catalog {
    /// Schema this document targets.
    pub schema_version: u32,
    /// Monotonic counter. A document with a serial at or below the one already
    /// installed is refused — that is the rollback protection.
    pub serial: u64,
    /// When the document was issued. Display and diagnostics only; `serial` is
    /// what ordering decisions use, because a clock is not authenticated by a
    /// signature over it.
    pub issued_at: DateTime<Utc>,
    /// Anthropic protocol constants.
    #[serde(default)]
    pub anthropic: AnthropicCatalog,
    /// How a client's own identity is recognised (`docs/TRUST.md` §3).
    #[serde(default)]
    pub client_identity: ClientIdentityCatalog,
    /// Model catalogues, keyed by backend id.
    #[serde(default)]
    pub models: BTreeMap<String, Vec<ModelEntry>>,
    /// Tools this document knows how to point at IronWire.
    ///
    /// Empty by default and never populated at compile time: the two agents
    /// IronWire ships knowing about are wired by code (see [`AgentEntry`]).
    #[serde(default)]
    pub agents: Vec<AgentEntry>,
}

/// Anthropic wire constants. Strings the API validates, not places we send to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AnthropicCatalog {
    /// `anthropic-version` header value.
    pub api_version: String,
    /// Beta flag that enables OAuth bearer auth. This is the value most likely
    /// to change under us, and the reason this channel exists.
    pub oauth_beta: String,
}

impl Default for AnthropicCatalog {
    fn default() -> Self {
        Self {
            api_version: "2023-06-01".to_string(),
            oauth_beta: "oauth-2025-04-20".to_string(),
        }
    }
}

/// Markers that identify which product a request came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ClientIdentityCatalog {
    /// Prefix of one of Claude Code's system blocks.
    pub claude_code_system_prefix: String,
    /// Prefix of the `user-agent` Claude Code sends. Refreshable for the same
    /// reason as the Codex originator: it is the half that survives the client
    /// rewriting its prompt, which Claude Code has already done.
    #[serde(default = "default_claude_code_user_agent_prefix")]
    pub claude_code_user_agent_prefix: String,
    /// Substring identifying Codex in a Responses `instructions` field.
    pub codex_instructions_marker: String,
    /// Prefix of the `originator` header Codex sends. Refreshable because it is
    /// the half that keeps working when Codex reshapes its request body — which
    /// it has already done once, dropping `instructions` entirely.
    #[serde(default = "default_codex_originator_prefix")]
    pub codex_originator_prefix: String,
    /// Phrases suggesting a request is a compaction turn
    /// (`docs/PROTOCOL.md` §8).
    ///
    /// Refreshable precisely because these are the least knowable strings in
    /// the product: every harness words its compaction prompt differently, none
    /// document it, and all of them change it. Advisory only — a wrong value
    /// costs a slightly worse routing decision, never a wrong answer.
    #[serde(default = "default_compaction_markers")]
    pub compaction_markers: Vec<String>,
}

fn default_claude_code_user_agent_prefix() -> String {
    ironwire_core::peek::CLAUDE_CODE_USER_AGENT_PREFIX.to_string()
}

fn default_codex_originator_prefix() -> String {
    ironwire_core::peek::CODEX_ORIGINATOR_PREFIX.to_string()
}

fn default_compaction_markers() -> Vec<String> {
    ironwire_core::peek::COMPACTION_MARKERS
        .iter()
        .map(|m| (*m).to_string())
        .collect()
}

impl Default for ClientIdentityCatalog {
    fn default() -> Self {
        Self {
            claude_code_system_prefix: "You are Claude Code".to_string(),
            claude_code_user_agent_prefix: default_claude_code_user_agent_prefix(),
            codex_instructions_marker: "Codex".to_string(),
            codex_originator_prefix: default_codex_originator_prefix(),
            compaction_markers: default_compaction_markers(),
        }
    }
}

/// One model a backend can serve.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelEntry {
    /// Provider's model slug.
    pub id: String,
    /// Quality tier: `fast`, `balanced`, or `frontier`. A value we do not
    /// recognise is treated as `frontier` by the caller — over-serving is
    /// recoverable, under-serving is silent.
    pub tier: String,
}

/// One of IronWire's own façades.
///
/// The document chooses *which* façade a tool is pointed at. It does not, and
/// cannot, say where that façade is: the scheme, the host and the port are this
/// binary's, and the path suffix is compiled in below.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Facade {
    /// The Anthropic-shaped façade.
    Anthropic,
    /// The OpenAI-shaped façade.
    OpenAi,
}

impl Facade {
    /// Every façade, so a caller can ask "is this one of ours" without
    /// hardcoding the list a second time.
    pub const ALL: [Self; 2] = [Self::Anthropic, Self::OpenAi];

    /// The compiled-in path this façade is served on. Never read from the
    /// document — this is the half of the address the catalog cannot reach.
    #[must_use]
    pub const fn path(self) -> &'static str {
        match self {
            Self::Anthropic => "/anthropic",
            Self::OpenAi => "/openai",
        }
    }

    /// Where this façade actually is, on this machine, right now.
    #[must_use]
    pub fn url(self, port: u16) -> String {
        format!("http://127.0.0.1:{port}{}", self.path())
    }

    /// The same address, in the form a particular tool expects to be given.
    ///
    /// `None` for a style this build does not implement: the address is only
    /// ever composed from parts compiled in here, so a style we do not
    /// recognise has no value to compose and the caller must refuse rather
    /// than guess at one.
    #[must_use]
    pub fn url_in(self, port: u16, style: UrlStyle) -> Option<String> {
        style
            .suffix()
            .map(|suffix| format!("{}{suffix}", self.url(port)))
    }
}

/// How much of a façade's address a tool expects to be handed.
///
/// Both forms are the *same* façade on the *same* loopback port, differing only
/// by a suffix that is compiled in here exactly as [`Facade::path`] is. The
/// document picks between spellings this binary ships; it still cannot write
/// one.
///
/// The variant exists because both conventions are genuinely in use, and a
/// catalog that can only express one of them can only describe half the tools.
/// Claude Code reads `ANTHROPIC_BASE_URL` as an origin and appends
/// `/v1/messages`; most OpenAI-compatible clients take a base URL that already
/// ends in `/v1`. `codex_config.rs` writes that second form by hand for the
/// built-in Codex support, and until this field existed no catalog entry could
/// say the same thing.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UrlStyle {
    /// Origin plus the façade path: `http://127.0.0.1:8463/anthropic`. What
    /// Claude Code's `ANTHROPIC_BASE_URL` wants, and the default because it is
    /// what every entry written before this field existed already meant.
    #[default]
    Origin,
    /// The same, plus the version segment the façade is served under:
    /// `http://127.0.0.1:8463/openai/v1`. What an OpenAI-compatible client's
    /// `base_url` or `baseURL` wants.
    Versioned,
    /// A style a newer document named and this binary does not implement.
    ///
    /// Present so that one unknown spelling costs the entry that used it and
    /// not the whole document: without it serde would refuse the catalog
    /// outright and take the provider constants — the values that stop the
    /// proxy working at all — down with an agent row. An entry carrying it is
    /// dropped by [`AgentEntry::problem`], which is the failure this schema
    /// already has for a malformed row.
    #[serde(other)]
    Unrecognised,
}

impl UrlStyle {
    /// Every style this build can actually write.
    pub const ALL: [Self; 2] = [Self::Origin, Self::Versioned];

    /// What this style appends to the façade path, or `None` when it is not a
    /// style this build implements.
    #[must_use]
    pub const fn suffix(self) -> Option<&'static str> {
        match self {
            Self::Origin => Some(""),
            Self::Versioned => Some("/v1"),
            Self::Unrecognised => None,
        }
    }
}

/// One key in a tool's config file, and which façade it points at.
///
/// **There is deliberately no literal-value variant.** A document that could
/// supply the value as well as the key could pair `ANTHROPIC_BASE_URL` with a
/// host of its choosing, which is the exfiltration path `docs/TRUST.md` I2
/// exists to close. Naming a façade is the entire vocabulary: the worst a
/// compromised signing key can do is point a tool at the user's own proxy, or
/// at the wrong key of their own config.
///
/// [`UrlStyle`] does not weaken that. It chooses between two spellings of the
/// same loopback address, both compiled in here; no string in this type reaches
/// the value.
///
/// # Why a setting can carry a precondition
///
/// [`requires`](Self::requires) exists because a correct URL in a correct key
/// can still break the tool. opencode takes `provider.anthropic.options.baseURL`
/// — a base URL, no API key, no model name, exactly the shape this type is for
/// — and it does redirect. But the *presence* of a `provider.anthropic` block is
/// itself a statement: it promotes Anthropic into opencode's configured provider
/// set and reorders which model it picks by default. A user with an OpenAI key
/// and no Anthropic one goes from a working `gpt-5.3-chat-latest` to
/// `claude-sonnet-4-6` and "Anthropic API key is missing" — before any request
/// leaves the machine. The same edit is right for a user who already uses
/// Anthropic there and wrong for one who does not, and until this field existed
/// nothing in an entry could tell them apart. `detect` answers "is the tool
/// installed", which is true for both.
///
/// So this is one more step along the rule the writer already follows. "Fill an
/// empty slot; leave a full one alone" already lets the contents of the file
/// decide what happens. This adds: *fill this slot only where the file shows the
/// tool already uses this provider.*
///
/// # Why it is a presence check and not more
///
/// A predicate language in a signed document is a much larger surface than the
/// problem needs. Presence of a key answers the case above, is decidable by
/// reading, and has no evaluation order, no operators and no way to be subtly
/// wrong. Value matching was considered and left out: it would let a document
/// describe the *contents* of a user's config rather than only its shape, and
/// nothing here needs it.
///
/// It is on the setting rather than the entry because that is where the evidence
/// puts it. The condition is per-provider, and one tool has one config file with
/// several provider blocks in it: an opencode entry that set both an Anthropic
/// and an OpenAI base URL would want each key gated on its own block. "The whole
/// tool is inapplicable" is also already sayable — `detect` and `enabled` — so
/// an entry-level predicate would be the less useful of the two placements as
/// well as the less precise one.
///
/// # What it cannot do
///
/// It gains the document no reading power whatsoever. The path is walked in the
/// one file the entry already names — the file `connect` opens anyway — and is
/// held to the same character rule as the key being written, so it cannot
/// contain a separator, cannot spell `..`, and cannot become a path on disk.
/// Reading `provider.anthropic` in that file is strictly less than writing
/// `provider.anthropic.options.baseURL` in it, which the entry could already do.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSetting {
    /// Dotted key path inside the config file, e.g. `env.ANTHROPIC_BASE_URL`.
    pub key: String,
    /// Which façade the key is set to.
    pub facade: Facade,
    /// Which spelling of that façade's address the tool expects. Defaults to
    /// [`UrlStyle::Origin`], so a document written before this field existed
    /// keeps meaning what it meant.
    ///
    /// A document that sets this to anything else must declare
    /// `schema_version` 2: an older build tolerates the unknown field, ignores
    /// it, and writes the origin spelling. See [`SCHEMA_VERSION`].
    #[serde(default)]
    pub style: UrlStyle,
    /// A key that must already be present in this tool's config before this
    /// setting is written. `None` — the default, and what every entry written
    /// before this field existed means — writes it unconditionally.
    ///
    /// This is a *presence* check and nothing more: the key exists, whatever
    /// its value. See [`AgentEntry`] for why the check is here at all and why
    /// it is this small.
    ///
    /// A document that sets this must declare `schema_version` 3, and for a
    /// sharper reason than `style` had. An older build tolerates the unknown
    /// field, ignores it, and writes the key unconditionally — which is exactly
    /// the edit the field exists to prevent. See [`SCHEMA_VERSION`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requires: Option<String>,
}

/// Whether a dotted key path is one this crate is willing to walk.
///
/// One rule for the key a setting writes and for the key a precondition reads,
/// because they are the same kind of thing and a second rule would eventually
/// disagree with the first. A segment is alphanumerics, `_` and `-`; nothing
/// else. That is what makes a key path unable to become a path on disk — no
/// separator of either slash, no `.` inside a segment, and `..` cannot be
/// spelled because an empty segment is refused.
fn key_path_is_safe(key: &str) -> bool {
    !key.is_empty()
        && key.split('.').all(|segment| {
            !segment.is_empty()
                && segment
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        })
}

impl AgentSetting {
    /// The value to write for this setting, or `None` when the style is one
    /// this build does not implement.
    #[must_use]
    pub fn url(&self, port: u16) -> Option<String> {
        self.facade.url_in(port, self.style)
    }

    /// Whether the key path is one we are willing to write.
    fn key_is_safe(&self) -> bool {
        key_path_is_safe(&self.key)
    }

    /// Whether the precondition, if there is one, is a key path we are willing
    /// to *read*.
    ///
    /// Held to the identical rule as the key we write, because it is the
    /// identical kind of thing: a dotted path into the one file this entry
    /// already names. `key_path_is_safe` admits only alphanumerics, `_` and `-`
    /// within a segment, so a separator, a drive letter and `..` are all
    /// unrepresentable — this cannot become a path, and cannot leave the file.
    fn requires_is_safe(&self) -> bool {
        self.requires.as_deref().is_none_or(key_path_is_safe)
    }
}

/// Which format a config file is written in. Derived from the file name rather
/// than declared, so the two can never disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigFormat {
    /// JSON object.
    Json,
    /// TOML table.
    Toml,
}

/// Where a tool keeps its config, as far as the document is allowed to say.
///
/// Always relative to the user's home directory, which is never named here. The
/// constraints below are the whole security argument for letting a signed
/// document introduce a tool at all:
///
/// - the first segment is a dotdir, so writes land in a hidden config directory
///   and not in `Documents` or a source tree;
/// - at most two directory segments, so this cannot walk anywhere interesting;
/// - `.` and `..` are refused outright, so it cannot escape upward;
/// - the file must end in `.json` or `.toml`, which is what rules out
///   `~/.ssh/config`, `~/.aws/credentials`, and every other extensionless
///   secret file that lives in a dotdir.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfigLocation {
    /// Directory segments under the user's home.
    pub dir: Vec<String>,
    /// File name within that directory.
    pub file: String,
}

impl ConfigLocation {
    /// The format implied by the file name, or `None` when it is not one we
    /// are prepared to write.
    #[must_use]
    pub fn format(&self) -> Option<ConfigFormat> {
        if self.file.ends_with(".json") {
            Some(ConfigFormat::Json)
        } else if self.file.ends_with(".toml") {
            Some(ConfigFormat::Toml)
        } else {
            None
        }
    }

    /// The absolute path, given a home directory.
    #[must_use]
    pub fn resolve(&self, home: &std::path::Path) -> std::path::PathBuf {
        let mut path = home.to_path_buf();
        for segment in &self.dir {
            path.push(segment);
        }
        path.push(&self.file);
        path
    }

    /// Why this location is not one we will write, or `None` when it is fine.
    fn problem(&self) -> Option<String> {
        if self.dir.is_empty() || self.dir.len() > 2 {
            return Some("needs one or two directory segments".to_string());
        }
        for segment in &self.dir {
            if !segment_is_safe(segment) {
                return Some(format!("`{segment}` is not a plain directory name"));
            }
        }
        if !self.dir[0].starts_with('.') {
            return Some(format!(
                "`{}` is not a dotdir; a tool's config lives in a hidden directory",
                self.dir[0]
            ));
        }
        if !segment_is_safe(&self.file) || self.file.starts_with('.') {
            return Some(format!("`{}` is not a plain file name", self.file));
        }
        if self.format().is_none() {
            return Some(format!(
                "`{}` is not a format IronWire can write (.json or .toml)",
                self.file
            ));
        }
        None
    }
}

/// A single path segment we are willing to join onto a home directory.
///
/// Rejects separators, `.` and `..`, and anything outside a conservative
/// charset. This is the function that makes traversal unrepresentable rather
/// than merely checked-for.
fn segment_is_safe(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.starts_with('-')
        && segment
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-')
}

/// A tool IronWire can offer to point at itself.
///
/// This is for tools that arrive *after* a release. The two IronWire ships
/// knowing about — Claude Code and Codex — are wired by compiled-in code
/// instead, because their setup is more than one key: Claude Code also gets a
/// statusline command, and Codex needs a provider table and a warning about
/// what it cannot change afterwards. Describing them here as well would be two
/// sources of truth for the same file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentEntry {
    /// Stable id, used for enable/disable and display.
    pub id: String,
    /// What the user calls it.
    pub name: String,
    /// Whether to offer it. Present so a tool whose config format changed can
    /// be switched off the same day, which is the case this channel exists for.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Executable names that mean the tool is installed. **Names, not paths** —
    /// anything with a separator is refused, so this cannot become a lookup
    /// outside `PATH`.
    #[serde(default)]
    pub detect: Vec<String>,
    /// Where its config lives.
    pub config: ConfigLocation,
    /// What to set in it.
    ///
    /// A setting may carry a precondition — [`AgentSetting::requires`] — that
    /// decides whether it is written at all. See the type docs for why that
    /// lives on the setting and not here.
    #[serde(default)]
    pub settings: Vec<AgentSetting>,
}

const fn default_true() -> bool {
    true
}

impl AgentEntry {
    /// Why this entry will not be used, or `None` when it is usable.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !segment_is_safe(&self.id) || self.id.starts_with('.') {
            return Some(format!("`{}` is not a usable id", self.id));
        }
        if self.name.trim().is_empty() {
            return Some("has no display name".to_string());
        }
        if let Some(problem) = self.config.problem() {
            return Some(problem);
        }
        for name in &self.detect {
            if !segment_is_safe(name) || name.starts_with('.') {
                return Some(format!("`{name}` is not a plain executable name"));
            }
        }
        if self.settings.is_empty() {
            return Some("sets nothing, so wiring it would do nothing".to_string());
        }
        for setting in &self.settings {
            if !setting.key_is_safe() {
                return Some(format!("`{}` is not a usable key path", setting.key));
            }
            if setting.style.suffix().is_none() {
                return Some(format!(
                    "`{}` asks for a URL style this build does not implement",
                    setting.key
                ));
            }
            if !setting.requires_is_safe() {
                return Some(format!(
                    "`{}` is not a usable key path",
                    setting.requires.as_deref().unwrap_or_default()
                ));
            }
        }
        None
    }
}

impl Default for Catalog {
    /// The compiled-in baseline: what this binary knows without a network.
    ///
    /// A fresh install with no connectivity must work, so the defaults are the
    /// values that were correct when the binary shipped — never empty.
    fn default() -> Self {
        Self {
            schema_version: SCHEMA_VERSION,
            serial: 0,
            issued_at: DateTime::UNIX_EPOCH,
            anthropic: AnthropicCatalog::default(),
            client_identity: ClientIdentityCatalog::default(),
            models: BTreeMap::new(),
            agents: Vec::new(),
        }
    }
}

impl Catalog {
    /// Models for a backend, or an empty slice when the document says nothing —
    /// in which case the backend keeps its compiled-in catalogue.
    #[must_use]
    pub fn models_for(&self, backend_id: &str) -> &[ModelEntry] {
        self.models.get(backend_id).map_or(&[], Vec::as_slice)
    }

    /// Agents that are enabled and passed validation.
    ///
    /// A malformed entry is **dropped, not fatal**. Refusing the whole document
    /// over one bad row would take the provider constants down with it — and
    /// those are the values that stop the proxy working at all. Rule 3 in
    /// `docs/UPDATES.md` is about failing closed onto the compiled-in
    /// defaults, which is exactly what dropping the row does.
    #[must_use]
    pub fn agents(&self) -> Vec<&AgentEntry> {
        self.agents
            .iter()
            .filter(|agent| agent.enabled && agent.problem().is_none())
            .collect()
    }

    /// Entries that were dropped, with the reason, so a caller can log them.
    /// Silently ignoring a tool the document meant to ship is how a channel
    /// like this stops being trusted.
    #[must_use]
    pub fn rejected_agents(&self) -> Vec<(&str, String)> {
        self.agents
            .iter()
            .filter_map(|agent| agent.problem().map(|problem| (agent.id.as_str(), problem)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn location(dir: &[&str], file: &str) -> ConfigLocation {
        ConfigLocation {
            dir: dir.iter().map(|s| (*s).to_string()).collect(),
            file: file.to_string(),
        }
    }

    fn agent(id: &str, config: ConfigLocation) -> AgentEntry {
        AgentEntry {
            id: id.to_string(),
            name: "A Tool".to_string(),
            enabled: true,
            detect: vec![id.to_string()],
            config,
            settings: vec![AgentSetting {
                key: "env.ANTHROPIC_BASE_URL".to_string(),
                facade: Facade::Anthropic,
                style: UrlStyle::Origin,
                requires: None,
            }],
        }
    }

    /// The provider constants still name nothing. This is the original walk,
    /// kept over the subtree it still applies to — `agents` describes locations
    /// on purpose and is guarded by the constraints below instead.
    #[test]
    fn the_provider_sections_cannot_express_a_host_a_url_or_a_path() {
        let catalog = Catalog::default();
        let document = serde_json::json!({
            "anthropic": catalog.anthropic,
            "client_identity": catalog.client_identity,
            "models": catalog.models,
        });

        fn walk(value: &serde_json::Value, path: &str) {
            match value {
                serde_json::Value::Object(map) => {
                    for (key, child) in map {
                        let lower = key.to_ascii_lowercase();
                        for banned in ["url", "host", "endpoint", "path", "base", "addr", "uri"] {
                            assert!(
                                !lower.contains(banned),
                                "`{path}.{key}` names a network location or filesystem path. \
                                 A signed remote document must not be able to redirect where a \
                                 credential goes — see the module docs and docs/TRUST.md I2."
                            );
                        }
                        walk(child, &format!("{path}.{key}"));
                    }
                }
                serde_json::Value::Array(items) => {
                    for item in items {
                        walk(item, path);
                    }
                }
                _ => {}
            }
        }
        walk(&document, "catalog");
    }

    /// `requires` is the only new string a document can carry, so it gets the
    /// same treatment the key it sits beside gets: a dotted path into the one
    /// file this entry already names, and nothing that could become a path on
    /// disk. Every rejected form here is a way someone would try to widen it.
    #[test]
    fn a_precondition_cannot_be_spelled_as_a_path() {
        let mut entry = agent(
            "tool",
            ConfigLocation {
                dir: vec![".tool".to_string()],
                file: "config.json".to_string(),
            },
        );
        for bad in [
            "../../.ssh/config",
            "/etc/passwd",
            "..",
            ".",
            "a/b",
            "a\\b",
            "",
            "a..b",
            "a.",
            "~",
            "$HOME",
            "http://example.com",
        ] {
            entry.settings[0].requires = Some(bad.to_string());
            assert!(
                entry.problem().is_some(),
                "`{bad}` must not be a usable precondition"
            );
        }
    }

    /// And the shape it is actually for is accepted.
    #[test]
    fn a_precondition_naming_a_key_in_the_same_file_is_usable() {
        let mut entry = agent(
            "tool",
            ConfigLocation {
                dir: vec![".tool".to_string()],
                file: "config.json".to_string(),
            },
        );
        entry.settings[0].requires = Some("provider.anthropic".to_string());
        assert_eq!(entry.problem(), None);
    }

    /// The default is what every entry written before this field existed
    /// meant: write the key, no questions asked.
    #[test]
    fn an_entry_without_a_precondition_still_deserialises_and_is_unconditional() {
        let entry: AgentEntry = serde_json::from_value(serde_json::json!({
            "id": "tool",
            "name": "A Tool",
            "config": {"dir": [".tool"], "file": "config.json"},
            "settings": [{"key": "env.BASE", "facade": "anthropic"}],
        }))
        .expect("deserialises");
        assert_eq!(entry.problem(), None);
        assert_eq!(entry.settings[0].requires, None);
    }

    /// The property that replaced it. A signed document picks a façade; the
    /// address is this binary's, so there is no string anywhere in the schema
    /// that could carry someone else's host.
    #[test]
    fn a_setting_can_name_a_facade_but_never_an_address() {
        let setting = AgentSetting {
            key: "env.ANTHROPIC_BASE_URL".to_string(),
            facade: Facade::Anthropic,
            style: UrlStyle::Origin,
            requires: None,
        };
        let serialised = serde_json::to_value(&setting).expect("serialises");
        let object = serialised.as_object().expect("an object");
        // Exactly three fields; only `key` is a string, and it names a slot in
        // the user's own file rather than anywhere to send to.
        assert_eq!(object.len(), 3, "{serialised}");
        assert_eq!(object["facade"], "anthropic");
        assert_eq!(object["style"], "origin");

        // And a value that does exist is loopback, on our own port — in either
        // spelling, because both halves of the address are ours.
        assert_eq!(
            Facade::Anthropic.url(8463),
            "http://127.0.0.1:8463/anthropic"
        );
        assert!(Facade::OpenAi.url(1).starts_with("http://127.0.0.1:1/"));
        for facade in Facade::ALL {
            for style in UrlStyle::ALL {
                let url = facade.url_in(8463, style).expect("a style we implement");
                assert!(url.starts_with("http://127.0.0.1:8463/"), "{url}");
            }
        }
    }

    /// A document that could name a literal value would reopen exactly the hole
    /// the façade indirection closes, so the deserialiser must not accept one.
    #[test]
    fn a_setting_carrying_its_own_value_does_not_deserialise() {
        let attempt = serde_json::json!({
            "key": "env.ANTHROPIC_BASE_URL",
            "value": "https://evil.example",
        });
        assert!(
            serde_json::from_value::<AgentSetting>(attempt).is_err(),
            "a setting without a façade must not parse — a `value` field would \
             be the redirect this schema exists to prevent"
        );

        // And the adversarial case: a well-formed setting with a rogue field
        // smuggled alongside. Unknown fields are tolerated for forward
        // compatibility, so this *parses* — the property is that nothing reads
        // it, and the address still comes from us.
        let smuggled = serde_json::json!({
            "key": "env.ANTHROPIC_BASE_URL",
            "facade": "anthropic",
            "value": "https://evil.example",
            "base_url": "https://evil.example",
        });
        let parsed: AgentSetting = serde_json::from_value(smuggled).expect("tolerates extras");
        assert_eq!(parsed.facade.url(8463), "http://127.0.0.1:8463/anthropic");
        assert_eq!(parsed.style, UrlStyle::Origin);
        assert_eq!(
            parsed.url(8463).as_deref(),
            Some("http://127.0.0.1:8463/anthropic")
        );
    }

    /// The gap this field closes. Most OpenAI-compatible clients want a base
    /// URL that already ends in `/v1` — which is what `codex_config.rs` writes
    /// by hand for the built-in Codex support, and what no catalog entry could
    /// previously ask for. Both spellings are still our own loopback address.
    #[test]
    fn a_setting_can_ask_for_the_versioned_spelling_of_the_same_facade() {
        let setting = AgentSetting {
            key: "model_providers.ironwire.base_url".to_string(),
            facade: Facade::OpenAi,
            style: UrlStyle::Versioned,
            requires: None,
        };
        assert_eq!(
            setting.url(8463).as_deref(),
            Some("http://127.0.0.1:8463/openai/v1")
        );

        // The suffix is not the document's either: it is picked from a closed
        // set, so widening a style to a host would take a release.
        let round_tripped: AgentSetting = serde_json::from_value(serde_json::json!({
            "key": "model_providers.ironwire.base_url",
            "facade": "open_ai",
            "style": "versioned",
        }))
        .expect("parses");
        assert_eq!(round_tripped, setting);
    }

    /// A style a newer document names must cost the row that used it, not the
    /// document: the provider constants beside it are what keep the proxy
    /// working at all.
    #[test]
    fn an_unrecognised_url_style_drops_the_entry_and_not_the_document() {
        let document = serde_json::json!({
            "schema_version": 1, "serial": 3, "issued_at": "2026-09-06T00:00:00Z",
            "anthropic": {"api_version": "2023-06-01", "oauth_beta": "oauth-2026-01-01"},
            "agents": [{
                "id": "tool",
                "name": "A Tool",
                "detect": ["tool"],
                "config": {"dir": [".tool"], "file": "config.json"},
                "settings": [{
                    "key": "base_url",
                    "facade": "open_ai",
                    "style": "some_future_spelling",
                }],
            }],
        });
        let catalog: Catalog = serde_json::from_value(document).expect("still parses");
        assert_eq!(catalog.anthropic.oauth_beta, "oauth-2026-01-01");
        assert!(catalog.agents().is_empty());
        assert_eq!(catalog.rejected_agents().len(), 1);

        // And nothing composes an address out of a style we do not implement.
        assert_eq!(Facade::OpenAi.url_in(8463, UrlStyle::Unrecognised), None);
    }

    #[test]
    fn a_config_location_cannot_escape_the_home_directory() {
        for dir in [
            vec![".."],
            vec![".claude", ".."],
            vec!["."],
            vec![".claude/../.ssh"],
            vec![""],
        ] {
            let entry = agent("tool", location(&dir, "settings.json"));
            assert!(
                entry.problem().is_some(),
                "{dir:?} was accepted as a directory"
            );
        }
    }

    /// The rule that keeps this out of `~/.ssh/config`, `~/.aws/credentials`,
    /// and every other extensionless secret that lives in a dotdir.
    #[test]
    fn a_config_file_must_be_a_format_we_can_write() {
        assert!(
            agent("tool", location(&[".ssh"], "config"))
                .problem()
                .is_some()
        );
        assert!(
            agent("tool", location(&[".aws"], "credentials"))
                .problem()
                .is_some()
        );
        assert!(
            agent("tool", location(&[".config", "zed"], "settings.json"))
                .problem()
                .is_none()
        );
        assert!(
            agent("tool", location(&[".zcode"], "config.toml"))
                .problem()
                .is_none()
        );
    }

    #[test]
    fn a_config_must_live_in_a_dotdir_and_not_walk_far() {
        // Not hidden: this would put writes in an ordinary directory.
        assert!(
            agent("tool", location(&["Documents"], "settings.json"))
                .problem()
                .is_some()
        );
        // Deeper than we are willing to go.
        assert!(
            agent("tool", location(&[".config", "a", "b"], "settings.json"))
                .problem()
                .is_some()
        );
    }

    #[test]
    fn a_detection_name_cannot_become_a_path() {
        let mut entry = agent("tool", location(&[".tool"], "config.json"));
        entry.detect = vec!["/usr/bin/evil".to_string()];
        assert!(entry.problem().is_some());

        entry.detect = vec!["../evil".to_string()];
        assert!(entry.problem().is_some());

        entry.detect = vec!["claude".to_string()];
        assert!(entry.problem().is_none());
    }

    #[test]
    fn a_key_path_is_restricted_to_plain_segments() {
        let mut entry = agent("tool", location(&[".tool"], "config.json"));
        for key in ["", "env..X", "env./etc/passwd", "a b"] {
            entry.settings = vec![AgentSetting {
                key: key.to_string(),
                facade: Facade::Anthropic,
                style: UrlStyle::Origin,
                requires: None,
            }];
            assert!(entry.problem().is_some(), "`{key}` was accepted");
        }
    }

    /// One bad row must not take the provider constants down with it — those
    /// are what stop the proxy working at all.
    #[test]
    fn a_malformed_agent_is_dropped_rather_than_refusing_the_document() {
        let catalog = Catalog {
            agents: vec![
                agent("good", location(&[".good"], "config.json")),
                agent("bad", location(&[".."], "config.json")),
            ],
            ..Catalog::default()
        };
        let usable: Vec<&str> = catalog.agents().iter().map(|a| a.id.as_str()).collect();
        assert_eq!(usable, vec!["good"]);

        let rejected = catalog.rejected_agents();
        assert_eq!(rejected.len(), 1);
        assert_eq!(rejected[0].0, "bad");
    }

    /// The case this channel exists for: a tool whose config format changed
    /// under us, switched off the same day without a release.
    #[test]
    fn a_disabled_agent_is_not_offered() {
        let mut entry = agent("tool", location(&[".tool"], "config.json"));
        entry.enabled = false;
        let catalog = Catalog {
            agents: vec![entry],
            ..Catalog::default()
        };
        assert!(catalog.agents().is_empty());
        // Disabled is not malformed, so it is not reported as rejected.
        assert!(catalog.rejected_agents().is_empty());
    }

    #[test]
    fn the_compiled_in_default_ships_no_agents() {
        // Claude Code and Codex are wired by code, not described here. Two
        // sources of truth for the same file is how they drift.
        assert!(Catalog::default().agents.is_empty());
    }

    #[test]
    fn the_compiled_in_default_is_usable_offline() {
        // A fresh install with no network must still talk to Anthropic.
        let catalog = Catalog::default();
        assert!(!catalog.anthropic.api_version.is_empty());
        assert!(!catalog.anthropic.oauth_beta.is_empty());
        assert!(!catalog.client_identity.claude_code_system_prefix.is_empty());
    }

    #[test]
    fn unknown_fields_are_tolerated_for_forward_compatibility() {
        // An older binary must survive a newer document rather than refusing
        // to start — that is the point of a data channel.
        let document = serde_json::json!({
            "schema_version": 1,
            "serial": 7,
            "issued_at": "2026-08-08T00:00:00Z",
            "anthropic": {"api_version": "2023-06-01", "oauth_beta": "oauth-2026-01-01",
                          "some_future_knob": true},
            "a_whole_new_section": {"x": 1},
        });
        let catalog: Catalog = serde_json::from_value(document).expect("tolerates unknown fields");
        assert_eq!(catalog.anthropic.oauth_beta, "oauth-2026-01-01");
    }

    #[test]
    fn omitted_sections_fall_back_to_the_compiled_in_values() {
        let document = serde_json::json!({
            "schema_version": 1, "serial": 1, "issued_at": "2026-08-08T00:00:00Z",
        });
        let catalog: Catalog = serde_json::from_value(document).expect("parses");
        assert_eq!(catalog.anthropic, AnthropicCatalog::default());
        assert_eq!(catalog.client_identity, ClientIdentityCatalog::default());
    }

    #[test]
    fn a_backend_with_no_catalogue_keeps_its_own() {
        assert!(Catalog::default().models_for("nearai").is_empty());
    }
}
