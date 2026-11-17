//! Every tool IronWire knows about, and what is true of it right now.
//!
//! One list, from two sources that stay separate everywhere else: the agents
//! this binary ships knowing about, whose setup is more than one key and so
//! lives in code, and the ones a signed catalog introduced. A caller — the
//! control API, and through it the menu bar — should not have to know or care
//! which is which, only what is installed and what is pointed at us.
//!
//! Reading is all this does. Nothing here writes: `catalog::connect`,
//! `claude_settings::connect` and `codex_config::connect` are the only things
//! that touch a file, and they are called from the CLI where a change can be
//! printed before it happens.

use std::path::PathBuf;

use ironwire_catalog::schema::Catalog;
use sha2::{Digest, Sha256};

use crate::{catalog, claude_settings, codex_config};

/// A tool, as a screen needs to describe it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tool {
    /// Stable id — what `ironwire disconnect <id>` accepts.
    pub id: String,
    /// What the user calls it.
    pub name: String,
    /// The file IronWire would edit. Shown because a tool nobody expected to be
    /// configured is a question about *which file*, every time.
    pub config_path: Option<PathBuf>,
    /// Whether the tool looks present on this machine.
    pub installed: bool,
    /// Whether its config currently routes through IronWire.
    pub wired: bool,
    /// What to run to point it here.
    pub connect_command: String,
}

/// Whether an executable of this name is reachable on `PATH`.
///
/// Lives here rather than in the CLI because detection is part of describing a
/// tool, and the control API needs the same answer the CLI gets.
#[must_use]
pub fn on_path(name: &str) -> bool {
    let Ok(path) = std::env::var("PATH") else {
        return false;
    };
    std::env::split_paths(&path).any(|dir| {
        let candidate = dir.join(name);
        candidate.is_file() || candidate.with_extension("exe").is_file()
    })
}

/// Every tool, built-in and catalog-described.
///
/// `installed` and `wired` are read from the filesystem, so this is a snapshot
/// rather than a subscription — which is right for a menu that opens, asks, and
/// closes again.
#[must_use]
pub fn all(catalog_document: &Catalog) -> Vec<Tool> {
    let mut tools = vec![built_in_claude(), built_in_codex()];

    for agent in catalog_document.agents() {
        let path = dirs::home_dir().map(|home| agent.config.resolve(&home));
        let contents = path
            .as_ref()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .unwrap_or_default();
        let installed =
            dirs::home_dir().is_some_and(|home| catalog::detected(agent, &home, &on_path));
        tools.push(Tool {
            id: agent.id.clone(),
            name: agent.name.clone(),
            config_path: path,
            installed,
            wired: catalog::is_wired(agent, &contents),
            connect_command: format!("ironwire connect {}", agent.id),
        });
    }
    tools
}

/// The settings file of the Claude Code the default login belongs to:
/// `~/.claude/settings.json`, whatever this process's `CLAUDE_CONFIG_DIR` says.
///
/// The daemon builds `claude-sub` from the unsuffixed Keychain item and
/// `~/.claude` regardless of its own environment, and the plain URL written
/// here is served by that login. Following the variable would point another
/// login's Claude Code at it — one account's requests on another's token.
/// That Claude Code is connected from a terminal, as a login of its own.
fn default_claude_settings() -> Option<PathBuf> {
    Some(claude_settings::path_in(&claude_settings::default_dir()?))
}

fn built_in_claude() -> Tool {
    let path = default_claude_settings();
    let contents = read(path.as_ref());
    Tool {
        id: "claude".to_string(),
        name: "Claude Code".to_string(),
        // A config directory it created, or its name on `PATH`. Both, because a
        // tool installed but never run has no directory yet, and one installed
        // as an app bundle leaves no name on `PATH`.
        installed: path
            .as_ref()
            .and_then(|path| path.parent().map(std::path::Path::exists))
            .unwrap_or(false)
            || on_path("claude"),
        wired: claude_settings::is_wired(&contents),
        config_path: path,
        connect_command: "ironwire connect claude".to_string(),
    }
}

fn built_in_codex() -> Tool {
    let path = codex_config::path();
    let contents = read(path.as_ref());
    Tool {
        id: "codex".to_string(),
        name: "Codex".to_string(),
        installed: path
            .as_ref()
            .and_then(|path| path.parent().map(std::path::Path::exists))
            .unwrap_or(false)
            || on_path("codex"),
        wired: codex_config::is_wired(&contents),
        config_path: path,
        connect_command: "ironwire connect codex".to_string(),
    }
}

/// A file that is not there reads as empty, which is the same answer as a file
/// with nothing of ours in it: not wired.
fn read(path: Option<&PathBuf>) -> String {
    path.and_then(|path| std::fs::read_to_string(path).ok())
        .unwrap_or_default()
}

/// Why a tool could not be wired.
///
/// The refusals are separate variants because they are separate answers to
/// "what do I do now". A file we cannot read is the user's file and needs a
/// person; a catalog entry that failed validation is *our* description being
/// wrong, and nothing done to the file will change it. Both used to arrive as
/// one formatted string, which left a caller — the control API, and through it
/// any GUI — matching on prose to tell them apart.
#[derive(Debug)]
pub enum Error {
    /// No tool by that id, built-in or catalog-described.
    UnknownTool(String),
    /// The tool's config could not be located.
    NoPath(String),
    /// The file on disk is not valid for its format, so it will not be
    /// rewritten. The tool description is fine; the file needs a person.
    Unparseable {
        /// The file that was read.
        path: PathBuf,
        /// What the parser said.
        detail: String,
    },
    /// The file is JSON with comments or trailing commas — legal for the tool
    /// that wrote it, and not something IronWire's JSON writer can read.
    ///
    /// The remediation differs by `comments`, because the reason to refuse
    /// does. A file with comments could not be rewritten without deleting
    /// them, so the way out is to set the one key by hand. A file whose only
    /// JSONC is a trailing comma loses nothing by being rewritten and is
    /// refused only because the parser stops at the comma, so deleting it is
    /// a fix the user can actually apply.
    Jsonc {
        /// The file that was read.
        path: PathBuf,
        /// Which construct was found, and on which line.
        detail: String,
        /// Whether the file has comments in it.
        comments: bool,
    },
    /// The catalog entry describing this tool did not survive validation.
    /// Nothing the user does to their config changes the answer.
    UnusableEntry {
        /// The tool the entry claims to describe.
        id: String,
        /// What is wrong with the entry.
        detail: String,
    },
}

impl Error {
    /// A stable slug for this refusal, for a caller that has to branch on it.
    ///
    /// Separate from [`Display`](std::fmt::Display) on purpose: the sentence is
    /// for a person and is expected to be rewritten, the slug is for a client
    /// and is not.
    #[must_use]
    pub const fn reason(&self) -> &'static str {
        match self {
            Self::UnknownTool(_) => "unknown-tool",
            Self::NoPath(_) => "no-path",
            Self::Unparseable { .. } => "unparseable",
            Self::Jsonc { .. } => "jsonc",
            Self::UnusableEntry { .. } => "unusable-entry",
        }
    }
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::UnknownTool(id) => write!(f, "no tool called `{id}`"),
            Self::NoPath(id) => write!(f, "could not work out where `{id}` keeps its config"),
            Self::Unparseable { path, detail } => write!(
                f,
                "{} is not valid for its format — IronWire will not rewrite a \
                 file it cannot read: {detail}. Fix the file, then run this again",
                path.display()
            ),
            Self::Jsonc {
                path,
                detail,
                comments: true,
            } => write!(
                f,
                "{} is JSONC — JSON with comments and trailing commas — which \
                 the tool that wrote it accepts and IronWire does not edit, \
                 because writing the file back would delete the comments: \
                 {detail}. Set the key by hand",
                path.display()
            ),
            Self::Jsonc { path, detail, .. } => write!(
                f,
                "{} is JSONC — JSON with trailing commas — which the tool that \
                 wrote it accepts and IronWire's JSON parser will not read: \
                 {detail}. Remove it and run this again, or set the key by hand",
                path.display()
            ),
            Self::UnusableEntry { id, detail } => write!(
                f,
                "the catalog entry for `{id}` is not usable: {detail}. Nothing \
                 done to the config file changes this — the description of the \
                 tool is wrong, not the file"
            ),
        }
    }
}

impl std::error::Error for Error {}

/// Attach the file to a refusal the catalog module raised about it.
fn from_catalog(error: catalog::Error, path: &std::path::Path, id: &str) -> Error {
    match error {
        catalog::Error::Unparseable(detail) => Error::Unparseable {
            path: path.to_path_buf(),
            detail,
        },
        catalog::Error::Jsonc(jsonc) => Error::Jsonc {
            path: path.to_path_buf(),
            detail: jsonc.found,
            comments: jsonc.comments,
        },
        catalog::Error::Unusable(detail) => Error::UnusableEntry {
            id: id.to_string(),
            detail,
        },
    }
}

/// An edit that has been worked out but not made.
///
/// Split from [`commit`] because every path that writes to somebody's config in
/// this codebase shows the change first. The CLI prints it; the control API
/// hands it back so a GUI can. Neither gets to skip the step.
#[derive(Debug, Clone)]
pub struct Planned {
    /// The file that would change.
    pub path: PathBuf,
    /// What would change, in words.
    pub changes: Vec<String>,
    /// Slots left alone because the user is already using them.
    pub occupied: Vec<(String, String)>,
    /// Settings not written because this config does not meet their
    /// precondition, as `(key, the key whose absence is the reason)`.
    ///
    /// Separate from `occupied` because the two are different answers to "what
    /// do I do now". An occupied slot holds somebody's own value and the way
    /// forward is to decide whether to move it. A skipped one is a statement
    /// about this config not being the kind this setting is for, and the way
    /// forward is usually nothing at all.
    pub skipped: Vec<(String, String)>,
    existing: String,
    contents: String,
}

impl Planned {
    /// Whether this would change anything at all.
    #[must_use]
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    /// This plan, as a hash: the file it would edit, the bytes it found
    /// there, and the bytes it would leave.
    ///
    /// A plan cannot be handed to a caller and handed back — `contents` is the
    /// whole file, and sending somebody's config out over the control API to
    /// get it returned is a worse trade than the confirmation is worth. So a
    /// caller that has been shown a plan gets this instead, and passes it back
    /// when it asks for the edit.
    ///
    /// All three parts are in it because the caller picks the tool and the
    /// direction on both calls, and only the file being the same does not make
    /// the *edit* the same. A partly wired config has a non-empty connect and a
    /// non-empty disconnect worked out from identical bytes; hashing the file
    /// alone would let a client that was shown additions commit the removal
    /// instead. Two tools that both have no config yet hash identically for the
    /// same reason, which is why the path is in here too.
    ///
    /// `changes` is not: it is prose describing `contents`, and the same
    /// resulting file is the same edit whatever we called it.
    ///
    /// A file that is not there takes part as empty, which is what it reads as
    /// everywhere else in this module.
    #[must_use]
    pub fn digest(&self) -> String {
        let mut hasher = Sha256::new();
        // Length-prefixed so the parts cannot be slid into one another: a path
        // ending in bytes a file could begin with must not hash as some other
        // path and file.
        for part in [
            self.path.to_string_lossy().as_bytes(),
            self.existing.as_bytes(),
            self.contents.as_bytes(),
        ] {
            Digest::update(&mut hasher, (part.len() as u64).to_be_bytes());
            Digest::update(&mut hasher, part);
        }
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
}

/// Which status line, if any, a caller wants in the agent's status-line slot.
///
/// [`StatusLine::Offer`] writes the calling binary plus a `statusline`
/// subcommand. That is right for the `ironwire` CLI,
/// which implements it. It is not right for a host that embeds this crate:
/// Claude Code reads the command's stdout, so a binary that rejects the
/// argument shows the user a blank line rather than an error they can act on.
/// Such a host either names the command that does serve the line
/// ([`StatusLine::Command`]) or declines the slot ([`StatusLine::Decline`]).
///
/// Every variant follows the same slot rules: fill the slot only when it is
/// empty, follow a line IronWire installed when its command moves, and never
/// replace or remove a line the user wrote.
///
/// Callers must allow future values rather than exhaustively matching today's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum StatusLine {
    /// Offer IronWire's status line, in the slot, if the user is not using it.
    /// What every caller got before this existed, and what the CLI, the daemon
    /// and the control API still get.
    #[default]
    Offer,
    /// Install this command as the status line, if the user is not using the
    /// slot. For a host that serves the line from its own executable, usually
    /// a subcommand that calls `ironwire_proxy::statusline::line_for_home`.
    ///
    /// Written verbatim, so it must already be a complete command line for the
    /// agent's shell: quote any path that contains spaces. It is marked as
    /// IronWire's, so a later [`plan_disconnect`] removes it, and a status
    /// line IronWire installed earlier with a different command is replaced
    /// by this one. A blank command is treated as [`StatusLine::Decline`]: a
    /// command that prints nothing is the failure declining exists to avoid.
    Command(String),
    /// Install no status line, and take out one IronWire installed on an
    /// earlier connect — that command names an executable that cannot serve
    /// it, and leaving it would keep the tool invoking it. A status line the
    /// user wrote themselves is left alone, as everywhere else here. For a host
    /// that has no command able to serve one. Routing is unaffected: the tool
    /// is still pointed at IronWire.
    Decline,
}

impl StatusLine {
    /// The command this choice writes into the slot, or `None` to keep the
    /// slot free of ours.
    fn command(&self) -> Option<String> {
        match self {
            Self::Offer => Some(statusline_command()),
            Self::Command(command) if !command.trim().is_empty() => Some(command.clone()),
            Self::Command(_) | Self::Decline => None,
        }
    }
}

/// What a caller chooses about an edit, beyond which tool and which port.
///
/// Construct from [`ConnectOptions::default`] and set what differs, so a future
/// choice does not break existing callers.
///
/// ```
/// use ironwire_agents::tools::{ConnectOptions, StatusLine};
/// let declined = ConnectOptions::default().with_status_line(StatusLine::Decline);
/// let hosted = ConnectOptions::default().with_status_line(StatusLine::Command(
///     "/opt/host/bin/host ironwire-statusline".to_string(),
/// ));
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ConnectOptions {
    /// Which status line to install. Only Claude Code has a slot for one
    /// today; the other tools ignore this.
    pub status_line: StatusLine,
}

impl ConnectOptions {
    /// Say which status line this caller can honour, if any.
    #[must_use]
    pub fn with_status_line(mut self, status_line: StatusLine) -> Self {
        self.status_line = status_line;
        self
    }
}

/// Work out how to point a tool at IronWire.
///
/// [`ConnectOptions::default`]: the status line is offered, which is what the
/// CLI and the control API want. A host that serves it from its own command,
/// or cannot serve one, uses [`plan_connect_with`].
///
/// # Errors
///
/// [`Error::UnknownTool`] for an id nothing knows, [`Error::NoPath`] when the
/// config cannot be located, [`Error::Unparseable`] or [`Error::Jsonc`] when the
/// file cannot be read, [`Error::UnusableEntry`] when the catalog entry
/// describing the tool did not survive validation.
pub fn plan_connect(id: &str, port: u16, catalog_document: &Catalog) -> Result<Planned, Error> {
    plan_connect_with(id, port, catalog_document, ConnectOptions::default())
}

/// Work out how to point a tool at IronWire, with every caller-owned choice
/// stated.
///
/// # Errors
///
/// As [`plan_connect`].
pub fn plan_connect_with(
    id: &str,
    port: u16,
    catalog_document: &Catalog,
    options: ConnectOptions,
) -> Result<Planned, Error> {
    match id {
        "claude" => {
            let path = default_claude_settings().ok_or_else(|| Error::NoPath(id.to_string()))?;
            let existing = read(Some(&path));
            let url = crate::endpoint::anthropic_url(port, None);
            let command = options.status_line.command();
            let edit = claude_settings::connect(&existing, command.as_deref(), Some(&url))
                .map_err(|error| Error::Unparseable {
                    path: path.clone(),
                    detail: error.to_string(),
                })?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: edit
                    .occupied
                    .into_iter()
                    .map(|o| (o.slot.to_string(), o.current))
                    .collect(),
                // The hand-written two describe themselves in code and have no
                // preconditions to carry.
                skipped: Vec::new(),
                existing,
                contents: edit.contents,
            })
        }
        "codex" => {
            let path = codex_config::path().ok_or_else(|| Error::NoPath(id.to_string()))?;
            let existing = read(Some(&path));
            let edit =
                codex_config::connect(&existing, port).map_err(|error| Error::Unparseable {
                    path: path.clone(),
                    detail: error.to_string(),
                })?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: edit
                    .occupied
                    .into_iter()
                    .map(|o| (o.slot.to_string(), o.current))
                    .collect(),
                // The hand-written two describe themselves in code and have no
                // preconditions to carry.
                skipped: Vec::new(),
                existing,
                contents: edit.contents,
            })
        }
        other => {
            let agent = catalog_agent(catalog_document, other)?;
            let home = dirs::home_dir().ok_or_else(|| Error::NoPath(other.to_string()))?;
            let path = agent.config.resolve(&home);
            let existing = read(Some(&path));
            let edit = catalog::connect(agent, &existing, port)
                .map_err(|error| from_catalog(error, &path, other))?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: edit
                    .occupied
                    .into_iter()
                    .map(|o| (o.slot, o.current))
                    .collect(),
                skipped: edit
                    .skipped
                    .into_iter()
                    .map(|s| (s.slot, s.requires))
                    .collect(),
                existing,
                contents: edit.contents,
            })
        }
    }
}

/// Work out how to take a tool back off IronWire.
///
/// Every arm here reports no occupied slots, and that is a property of undoing
/// rather than a gap in the reporting: an undo fills nothing, so it never finds
/// a slot full and steps around it. Each module removes only what IronWire
/// wrote and leaves everything else exactly as it is — which is the same
/// promise `occupied` exists to make on the way in, kept without needing to say
/// anything.
///
/// # Errors
///
/// As [`plan_connect`].
pub fn plan_disconnect(id: &str, catalog_document: &Catalog) -> Result<Planned, Error> {
    match id {
        "claude" => {
            let path = default_claude_settings().ok_or_else(|| Error::NoPath(id.to_string()))?;
            let existing = read(Some(&path));
            let edit =
                claude_settings::disconnect(&existing).map_err(|error| Error::Unparseable {
                    path: path.clone(),
                    detail: error.to_string(),
                })?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: Vec::new(),
                skipped: Vec::new(),
                existing,
                contents: edit.contents,
            })
        }
        "codex" => {
            let path = codex_config::path().ok_or_else(|| Error::NoPath(id.to_string()))?;
            let existing = read(Some(&path));
            let edit = codex_config::disconnect(&existing).map_err(|error| Error::Unparseable {
                path: path.clone(),
                detail: error.to_string(),
            })?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: Vec::new(),
                skipped: Vec::new(),
                existing,
                contents: edit.contents,
            })
        }
        other => {
            let agent = catalog_agent(catalog_document, other)?;
            let home = dirs::home_dir().ok_or_else(|| Error::NoPath(other.to_string()))?;
            let path = agent.config.resolve(&home);
            let existing = read(Some(&path));
            let edit = catalog::disconnect(agent, &existing)
                .map_err(|error| from_catalog(error, &path, other))?;
            Ok(Planned {
                path,
                changes: edit.changes,
                occupied: Vec::new(),
                skipped: Vec::new(),
                existing,
                contents: edit.contents,
            })
        }
    }
}

fn catalog_agent<'a>(
    catalog_document: &'a Catalog,
    id: &str,
) -> Result<&'a ironwire_catalog::schema::AgentEntry, Error> {
    catalog_document
        .agents()
        .into_iter()
        .find(|agent| agent.id == id)
        .ok_or_else(|| Error::UnknownTool(id.to_string()))
}

/// Make the edit, keeping one copy of what was there before.
///
/// Returns the backup's path when one was written — a file that did not exist
/// has nothing worth preserving.
///
/// # Errors
///
/// Propagates the filesystem error.
pub fn commit(planned: &Planned) -> std::io::Result<Option<PathBuf>> {
    if let Some(parent) = planned.path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let backup = if planned.existing.is_empty() {
        None
    } else {
        let extension = planned
            .path
            .extension()
            .map_or_else(|| "bak".to_string(), |e| e.to_string_lossy().to_string());
        let backup = planned
            .path
            .with_extension(format!("{extension}.ironwire-backup"));
        // Never overwrite one. The first backup is the only one that holds the
        // file as it was before IronWire touched it; a second write — say a
        // disconnect after a connect — would replace the user's original with
        // our own edit and leave nothing to go back to. Found the hard way.
        if backup.exists() {
            None
        } else {
            std::fs::write(&backup, &planned.existing)?;
            Some(backup)
        }
    };
    std::fs::write(&planned.path, &planned.contents)?;
    Ok(backup)
}

/// Why a confirmed edit was not made.
#[derive(Debug)]
pub enum CommitError {
    /// The file is no longer the one the plan was worked out against.
    Changed,
    /// The write itself failed.
    Io(std::io::Error),
}

impl std::fmt::Display for CommitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Changed => write!(f, "the file changed after the plan was worked out"),
            Self::Io(error) => write!(f, "{error}"),
        }
    }
}

impl std::error::Error for CommitError {}

/// Make the edit, but only while the file is still what the plan was worked out
/// against.
///
/// [`commit`] writes what it is given. A caller confirming a plan somebody was
/// shown wants more than that: the file is read again here, immediately before
/// the write, and an edit that landed in between — another request, or the
/// agent itself — refuses rather than being overwritten by a plan that never
/// saw it.
///
/// This is a check and then a write, not one atomic operation. A write that
/// lands between the two is still lost, and closing that needs a conditional
/// write the filesystem does not offer portably. What it does close is the
/// whole span from working the plan out to committing it, which is where a
/// second request and a round trip to a user live.
///
/// # Errors
///
/// [`CommitError::Changed`] when the file moved since, [`CommitError::Io`] from
/// the write.
pub fn commit_if_unchanged(planned: &Planned) -> Result<Option<PathBuf>, CommitError> {
    if read(Some(&planned.path)) != planned.existing {
        return Err(CommitError::Changed);
    }
    commit(planned).map_err(CommitError::Io)
}

/// The status line command, which is this binary plus a subcommand.
fn statusline_command() -> String {
    std::env::current_exe().map_or_else(
        |_| "ironwire statusline".to_string(),
        |exe| format!("{} statusline", exe.display()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_two_built_in_agents_are_always_listed() {
        // Listed whether or not they are installed: "Claude Code — not found"
        // is an answer, and omitting it looks like IronWire never heard of it.
        let tools = all(&Catalog::default());
        let ids: Vec<&str> = tools.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["claude", "codex"]);
    }

    #[test]
    fn every_tool_names_the_command_that_would_wire_it() {
        for tool in all(&Catalog::default()) {
            assert!(
                tool.connect_command.starts_with("ironwire connect "),
                "{tool:?}"
            );
        }
    }

    /// The CLI, the daemon and the control API all reach `plan_connect`, and
    /// none of them passes options. The status line has to stay on for them.
    #[test]
    fn the_default_still_offers_the_status_line() {
        assert_eq!(ConnectOptions::default().status_line, StatusLine::Offer);
    }

    /// A host names the command that serves its line, and that is what goes
    /// in the slot, byte for byte.
    #[test]
    fn a_host_command_is_written_as_given() {
        let command = "/opt/host/bin/host ironwire-statusline --home '/a b'";
        assert_eq!(
            StatusLine::Command(command.to_string())
                .command()
                .as_deref(),
            Some(command)
        );
        assert!(StatusLine::Decline.command().is_none());
        assert!(
            StatusLine::Offer
                .command()
                .is_some_and(|c| c.ends_with(" statusline"))
        );
    }

    /// A blank command would print nothing on every render, which is exactly
    /// what declining exists to prevent, so it declines.
    #[test]
    fn a_blank_host_command_declines_rather_than_installing_nothing() {
        assert!(StatusLine::Command(String::new()).command().is_none());
        assert!(StatusLine::Command("  ".to_string()).command().is_none());
    }

    #[test]
    fn a_missing_file_is_not_wired_rather_than_an_error() {
        assert!(!claude_settings::is_wired(""));
        assert!(!codex_config::is_wired(""));
    }

    #[test]
    fn a_config_pointing_somewhere_else_is_not_wired() {
        assert!(!claude_settings::is_wired(
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://their-proxy.example"}}"#
        ));
        assert!(!codex_config::is_wired(
            "model_provider = \"someone-else\"\n"
        ));
    }

    #[test]
    fn a_config_pointing_at_us_is_wired() {
        assert!(claude_settings::is_wired(
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic"}}"#
        ));
        assert!(codex_config::is_wired("model_provider = \"ironwire\"\n"));
    }

    /// Codex's provider block can sit in the file unselected. What decides
    /// where traffic goes is `model_provider`, so that is what is reported.
    #[test]
    fn a_codex_provider_block_alone_is_not_wired() {
        let existing = "[model_providers.ironwire]\nbase_url = \"http://127.0.0.1:8463/openai\"\n";
        assert!(!codex_config::is_wired(existing));
    }

    /// A caller shown a plan sends its digest back to commit the edit it was
    /// shown, and it picks the tool and the direction again when it does. So
    /// the digest has to name the edit, not the file: a partly wired config has
    /// a real connect *and* a real disconnect worked out from identical bytes,
    /// and a digest over the file alone would let a client that was shown the
    /// additions commit the removal instead.
    #[test]
    fn a_digest_names_the_edit_and_not_only_the_file() {
        let planned = |path: &str, existing: &str, contents: &str| Planned {
            path: PathBuf::from(path),
            changes: vec!["something".to_string()],
            occupied: Vec::new(),
            skipped: Vec::new(),
            existing: existing.to_string(),
            contents: contents.to_string(),
        };

        // The same edit, worked out twice.
        assert_eq!(
            planned("config.toml", "ORIGINAL", "EDITED").digest(),
            planned("config.toml", "ORIGINAL", "EDITED").digest(),
        );
        // Two edits to the same file: the connect and the disconnect.
        assert_ne!(
            planned("config.toml", "ORIGINAL", "WIRED").digest(),
            planned("config.toml", "ORIGINAL", "UNWIRED").digest(),
        );
        // The file moved between the preview and the commit.
        assert_ne!(
            planned("config.toml", "ORIGINAL", "EDITED").digest(),
            planned("config.toml", "ORIGINAL, THEN CHANGED", "EDITED").digest(),
        );
        // Two tools that both have no config yet: same empty bytes, and the
        // preview for one must not authorise the edit to the other.
        assert_ne!(
            planned("a/config.toml", "", "EDITED").digest(),
            planned("b/config.toml", "", "EDITED").digest(),
        );
    }

    /// The plan holds the file as it was read. Confirming one has to look
    /// again, or an edit that landed in between is overwritten by a plan that
    /// never saw it.
    #[test]
    fn a_confirmed_edit_refuses_a_file_that_moved_since() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, "ORIGINAL").expect("write");

        let planned = Planned {
            path: path.clone(),
            changes: vec!["something".to_string()],
            occupied: Vec::new(),
            skipped: Vec::new(),
            existing: "ORIGINAL".to_string(),
            contents: "EDITED".to_string(),
        };
        assert!(commit_if_unchanged(&planned).is_ok());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), "EDITED");

        // Somebody else got there first, so this plan is stale.
        std::fs::write(&path, "SOMEBODY ELSE").expect("write");
        assert!(matches!(
            commit_if_unchanged(&planned),
            Err(CommitError::Changed)
        ));
        assert_eq!(
            std::fs::read_to_string(&path).expect("read"),
            "SOMEBODY ELSE",
            "a stale plan overwrote the edit it never saw"
        );
    }

    /// The whole point of splitting the variants: a caller deciding what to
    /// tell somebody must not have to look at the sentence to know which of
    /// these two happened. They ask for different things — one for a person to
    /// fix a file, one for us to fix a catalog entry.
    #[test]
    fn a_broken_file_and_a_broken_catalog_entry_are_different_refusals() {
        assert_eq!(
            Error::Unparseable {
                path: PathBuf::from("/home/u/.tool/config.json"),
                detail: "expected `,`".to_string(),
            }
            .reason(),
            "unparseable"
        );
        assert_eq!(
            Error::UnusableEntry {
                id: "tool".to_string(),
                detail: "no format".to_string(),
            }
            .reason(),
            "unusable-entry"
        );
        assert_eq!(
            Error::Jsonc {
                path: PathBuf::from("/home/u/.config/zed/settings.json"),
                detail: "a `//` comment on line 1".to_string(),
                comments: true,
            }
            .reason(),
            "jsonc"
        );
    }

    /// Every refusal names the file or the tool it is about, because "could not
    /// be edited" with nothing after it is the message this change exists to
    /// stop producing.
    #[test]
    fn every_refusal_names_what_it_is_about() {
        let unparseable = Error::Unparseable {
            path: PathBuf::from("/home/u/.tool/config.json"),
            detail: "expected `,`".to_string(),
        }
        .to_string();
        assert!(
            unparseable.contains("/home/u/.tool/config.json"),
            "{unparseable}"
        );

        let unusable = Error::UnusableEntry {
            id: "tool".to_string(),
            detail: "no format".to_string(),
        }
        .to_string();
        assert!(unusable.contains("`tool`"), "{unusable}");
        assert!(unusable.contains("not the file"), "{unusable}");

        let jsonc = Error::Jsonc {
            path: PathBuf::from("/home/u/.config/zed/settings.json"),
            detail: "a `//` comment on line 1".to_string(),
            comments: true,
        }
        .to_string();
        assert!(
            jsonc.contains("/home/u/.config/zed/settings.json"),
            "{jsonc}"
        );
        assert!(jsonc.contains("Set the key by hand"), "{jsonc}");
    }

    /// The advice, not just the label. A file with no comments in it must not
    /// be told to remove its comments: a user who follows that finds nothing
    /// to delete and the request still failing.
    #[test]
    fn a_file_with_no_comments_is_not_told_to_remove_comments() {
        let jsonc = Error::Jsonc {
            path: PathBuf::from("/home/u/.config/opencode/opencode.json"),
            detail: "a trailing comma on line 3".to_string(),
            comments: false,
        }
        .to_string();
        assert!(!jsonc.contains("comment"), "{jsonc}");
        assert!(jsonc.contains("Remove it and run this again"), "{jsonc}");
        assert!(
            jsonc.contains("/home/u/.config/opencode/opencode.json"),
            "{jsonc}"
        );
    }

    #[test]
    fn an_id_nothing_knows_is_refused_before_any_file_is_read() {
        let error =
            plan_connect("nothing-by-that-name", 8463, &Catalog::default()).expect_err("refuses");
        assert_eq!(error.reason(), "unknown-tool");
    }

    /// The first backup is the only one holding the file as it was before
    /// IronWire touched it. A connect followed by a disconnect used to replace
    /// it with our own edit, leaving nothing to go back to.
    #[test]
    fn a_second_write_does_not_replace_the_original_backup() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, "ORIGINAL").expect("write");
        let backup = path.with_extension("json.ironwire-backup");

        let first = Planned {
            path: path.clone(),
            changes: vec!["something".to_string()],
            occupied: Vec::new(),
            skipped: Vec::new(),
            existing: "ORIGINAL".to_string(),
            contents: "EDITED".to_string(),
        };
        assert_eq!(commit(&first).expect("commit"), Some(backup.clone()));
        assert_eq!(std::fs::read_to_string(&backup).expect("read"), "ORIGINAL");

        let second = Planned {
            path,
            changes: vec!["something else".to_string()],
            occupied: Vec::new(),
            skipped: Vec::new(),
            existing: "EDITED".to_string(),
            contents: "EDITED AGAIN".to_string(),
        };
        assert_eq!(commit(&second).expect("commit"), None);
        assert_eq!(
            std::fs::read_to_string(&backup).expect("read"),
            "ORIGINAL",
            "the original backup was overwritten by a later edit"
        );
    }
}
