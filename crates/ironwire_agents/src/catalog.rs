//! Pointing a catalog-described tool at IronWire.
//!
//! `claude_settings` and `codex_config` in the `ironwire` binary handle the two agents
//! this binary ships knowing about, and they do more than set a URL — a status
//! line, a provider table, a warning about what Codex cannot change afterwards.
//! This module is for the rest: tools the signed catalog introduces, where all
//! we know is a config file and a key to set in it
//! (`ironwire_catalog::schema::AgentEntry`).
//!
//! Every rule the hand-written two follow applies here, because they are what
//! makes editing someone else's config acceptable at all:
//!
//! - **Never rewrite a file we cannot parse.** A user's own syntax error must
//!   not come back looking like ours. A JSON file with comments in it is a
//!   case of this and not a syntax error — Zed and opencode both accept one —
//!   so it is refused under its own name rather than as a parse failure, and
//!   the refusal says which construct was found and on which line.
//! - **Fill an empty slot; leave a full one alone.** A value already in the key
//!   is another proxy or a deliberate choice, and taking it over would move
//!   someone's traffic without telling them. It is reported, not overwritten.
//! - **Remove only what we put there.**
//!
//! The value written is never the catalog's to choose: it comes from
//! [`Facade::url`], which is loopback and this daemon's port.

use std::path::Path;

use ironwire_catalog::schema::{AgentEntry, AgentSetting, ConfigFormat, Facade, UrlStyle};
use serde_json::{Map, Value};

/// A slot that already held something the user put there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupied {
    /// The key path, so the caller can name it back to them.
    pub slot: String,
    /// What is in it.
    pub current: String,
}

/// A setting that was not written because its precondition is not met.
///
/// Reported rather than silently dropped, for the reason every other refusal
/// here is: a connect that does nothing and says nothing is indistinguishable
/// from one that failed, and the person reading the output is the only one who
/// can tell whether the answer is right for them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// The key that was not written.
    pub slot: String,
    /// The key whose absence is the reason.
    pub requires: String,
}

/// What an edit would do, so the caller can show it before doing it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Edit {
    /// The full file contents after the edit.
    pub contents: String,
    /// Human-readable lines describing what changed.
    pub changes: Vec<String>,
    /// Slots left alone because the user was already using them.
    pub occupied: Vec<Occupied>,
    /// Settings not written because the config does not meet their
    /// precondition — see [`AgentSetting::requires`].
    pub skipped: Vec<Skipped>,
}

impl Edit {
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }
}

/// Why an edit could not be produced.
#[derive(Debug)]
pub enum Error {
    /// The existing file is not valid for its format.
    Unparseable(String),
    /// The existing file is JSON with comments or trailing commas — legal for
    /// the tool that wrote it, and not something this module will rewrite.
    Jsonc(Jsonc),
    /// The entry did not survive validation, so we will not act on it.
    Unusable(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unparseable(detail) => write!(
                f,
                "the file is not valid for its format — IronWire will not \
                 rewrite a file it cannot read: {detail}"
            ),
            Self::Jsonc(jsonc) if jsonc.comments => write!(
                f,
                "the file is JSONC — JSON with comments and trailing commas — \
                 which the tool that wrote it accepts and IronWire will not \
                 rewrite, because writing it back would delete the comments: \
                 {}",
                jsonc.found
            ),
            Self::Jsonc(jsonc) => write!(
                f,
                "the file is JSONC — JSON with trailing commas — which the \
                 tool that wrote it accepts and IronWire's JSON parser will \
                 not read: {}",
                jsonc.found
            ),
            Self::Unusable(detail) => write!(f, "the catalog entry is not usable: {detail}"),
        }
    }
}

impl std::error::Error for Error {}

/// A file that will not parse as JSON, but would if the constructs JSONC adds
/// on top of JSON were taken out of it.
///
/// The second half of that sentence is load-bearing and is checked, not
/// assumed. A document can contain a trailing comma *and* be broken for an
/// unrelated reason — `{"a": nope,}` is refused for the unquoted value, not
/// the comma — and reporting the comma there sends the user to fix the wrong
/// thing. So the constructs are removed and the result re-parsed; only a
/// document that is then a valid JSON object is reported as JSONC.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Jsonc {
    /// The first such construct, named with the line it is on.
    pub found: String,
    /// Whether the file has comments in it.
    ///
    /// The remediation turns on this and nothing else. A file with comments
    /// cannot be rewritten without losing them, so the way forward is to set
    /// the key by hand. A file whose only JSONC is a trailing comma would lose
    /// nothing by being rewritten — it is refused because the parser will not
    /// read it — so deleting that one comma is a real fix, and telling its
    /// owner to remove comments they do not have is not.
    pub comments: bool,
}

/// Point a tool at IronWire.
///
/// # Errors
///
/// [`Error::Unparseable`] when the file is not valid, [`Error::Jsonc`] when it
/// is valid JSONC rather than JSON, [`Error::Unusable`] when the catalog entry
/// fails validation. A shape this module will not edit by hand is reported as
/// an occupied slot rather than raised as an error.
pub fn connect(agent: &AgentEntry, existing: &str, port: u16) -> Result<Edit, Error> {
    match usable(agent)? {
        ConfigFormat::Json => json_connect(agent, existing, port),
        ConfigFormat::Toml => toml_connect(agent, existing, port),
    }
}

/// Remove what [`connect`] added, and only that.
///
/// # Errors
///
/// As [`connect`].
pub fn disconnect(agent: &AgentEntry, existing: &str) -> Result<Edit, Error> {
    match usable(agent)? {
        ConfigFormat::Json => json_disconnect(agent, existing),
        ConfigFormat::Toml => toml_disconnect(agent, existing),
    }
}

/// Whether this tool looks installed on the machine.
///
/// Mirrors `claude_installed` / `codex_installed`, and for the same reason both
/// halves are needed: a tool installed but never run has no config directory
/// yet, and one installed outside `PATH` — an app bundle, a version-manager
/// shim — still leaves its directory behind. Either is enough.
///
/// `on_path` is injected so this is decidable without a real `PATH`, and so the
/// rule can be tested rather than only observed.
pub fn detected(agent: &AgentEntry, home: &Path, on_path: &dyn Fn(&str) -> bool) -> bool {
    // An entry we would refuse to act on is not one to report as present:
    // offering to wire a tool and then failing at the file is worse than never
    // offering.
    if agent.problem().is_some() {
        return false;
    }
    if agent
        .config
        .resolve(home)
        .parent()
        .is_some_and(Path::exists)
    {
        return true;
    }
    agent.detect.iter().any(|name| on_path(name))
}

/// Whether this tool's config already points at IronWire, in the form the entry
/// currently asks for.
///
/// Every setting has to be ours. A tool with the base URL set but a second key
/// still the user's is half-wired, and reporting it as connected would explain
/// away the very symptom someone opened the menu to understand.
///
/// "Ours" is not sufficient on its own, because [`points_at_us`] deliberately
/// accepts every façade and both spellings — that width is what lets `connect`
/// recognise its own earlier value instead of mistaking it for somebody else's.
/// Recognising a value and being *done* are different questions. When a catalog
/// corrects an entry from [`UrlStyle::Origin`] to [`UrlStyle::Versioned`], the
/// value already in the file is still ours, but it names a path the façade does
/// not serve. Reporting that as wired is what would strand it: `tools::all`
/// hands this answer to the menu, whose onboarding banner offers only tools that
/// are *not* wired and whose per-tool toggle is already showing a checkmark, so
/// nothing would ever call `connect` to make the change the catalog asked for.
///
/// So recognition stays wide and this stays narrow: the value has to be the
/// address this setting asks for today — same façade, same spelling. The port
/// is still not part of the comparison, for the same reason `connect` follows
/// itself across one.
///
/// A setting whose precondition this file does not meet is not part of the
/// question. `connect` would not write it, so requiring it would make the tool
/// permanently unwired and put a checkmark permanently out of reach. An entry
/// *none* of whose settings apply is not wired either, rather than vacuously
/// wired over an empty set — nothing of ours is in the file, and saying
/// otherwise would report a connection that never happened.
#[must_use]
pub fn is_wired(agent: &AgentEntry, existing: &str) -> bool {
    let Ok(format) = usable(agent) else {
        return false;
    };
    if agent.settings.is_empty() {
        return false;
    }
    match format {
        ConfigFormat::Json => {
            // Reading, not writing: see `json_parse_tolerant`. The writers a
            // few hundred lines down deliberately keep the strict parser.
            let Ok(root) = json_parse_tolerant(existing) else {
                return false;
            };
            let present = |key: &str| json_present(&root, &key.split('.').collect::<Vec<_>>());
            let applicable: Vec<_> = agent
                .settings
                .iter()
                .filter(|setting| applies_here(setting, &present))
                .collect();
            !applicable.is_empty()
                && applicable.iter().all(|setting| {
                    let path: Vec<&str> = setting.key.split('.').collect();
                    matches!(json_slot(&root, &path), Slot::Ours(value) if points_where_asked(&value, setting))
                })
        }
        ConfigFormat::Toml => {
            let Ok(table) = toml_parse(existing) else {
                return false;
            };
            let present = |key: &str| toml_present(&table, key);
            let applicable: Vec<_> = agent
                .settings
                .iter()
                .filter(|setting| applies_here(setting, &present))
                .collect();
            !applicable.is_empty()
                && applicable.iter().all(|setting| {
                    matches!(toml_slot(&table, &setting.key), Slot::Ours(value) if points_where_asked(&value, setting))
                })
        }
    }
}

/// The entry's format, once it has passed the schema's own validation.
///
/// Checked here as well as at load, because an entry reaching a file writer is
/// the last place it is cheap to refuse.
fn usable(agent: &AgentEntry) -> Result<ConfigFormat, Error> {
    if let Some(problem) = agent.problem() {
        return Err(Error::Unusable(problem));
    }
    agent
        .config
        .format()
        .ok_or_else(|| Error::Unusable("no format".to_string()))
}

/// Whether this setting's precondition is met by the file as it stands.
///
/// A setting with no `requires` always applies, which is what every entry
/// written before that field existed means.
///
/// The path is walked in the file already parsed for the edit. Nothing else is
/// opened, and nothing outside this file is reachable: the schema holds
/// `requires` to the same character rule as the key being written, so it cannot
/// carry a separator or spell `..`. This is a *presence* test — the key is
/// there, whatever its value — because that is what the case it exists for
/// needs, and because a value test would let a document describe the contents
/// of somebody's config rather than only its shape.
fn applies_here(setting: &AgentSetting, present: &dyn Fn(&str) -> bool) -> bool {
    setting.requires.as_deref().is_none_or(present)
}

/// The value this setting asks us to write.
///
/// `usable` has already refused an entry naming a style this build cannot
/// write, so this only fires if the two ever disagree — and refusing is the
/// half of that disagreement we can stand behind.
fn value_for(setting: &AgentSetting, port: u16) -> Result<String, Error> {
    setting.url(port).ok_or_else(|| {
        Error::Unusable(format!(
            "`{}` asks for a URL style this build does not implement",
            setting.key
        ))
    })
}

// ---------------------------------------------------------------------- JSON

fn json_connect(agent: &AgentEntry, existing: &str, port: u16) -> Result<Edit, Error> {
    let mut root = json_parse(existing)?;
    let mut changes = Vec::new();
    let mut occupied = Vec::new();
    let mut skipped = Vec::new();

    // Every precondition is answered against the file as we found it, not
    // against the document as we are building it. Otherwise a setting could
    // satisfy the condition of one written after it — an entry whose meaning
    // depended on the order of its own settings, which is not a property worth
    // having.
    let before = root.clone();

    for setting in &agent.settings {
        if !applies_here(setting, &|key| {
            json_present(&before, &key.split('.').collect::<Vec<_>>())
        }) {
            skipped.push(Skipped {
                slot: setting.key.clone(),
                requires: setting.requires.clone().unwrap_or_default(),
            });
            continue;
        }
        let url = value_for(setting, port)?;
        let path: Vec<&str> = setting.key.split('.').collect();
        match json_slot(&root, &path) {
            Slot::Blocked(current) => occupied.push(Occupied {
                slot: setting.key.clone(),
                current,
            }),
            Slot::Ours(current) => {
                if current != url {
                    json_insert(&mut root, &path, &url);
                    changes.push(format!("{}: updated to `{url}`", setting.key));
                }
            }
            Slot::Empty => {
                json_insert(&mut root, &path, &url);
                changes.push(format!("{}: `{url}` (added)", setting.key));
            }
        }
    }

    Ok(Edit {
        contents: json_render(&root),
        changes,
        occupied,
        skipped,
    })
}

/// Disconnect is deliberately *not* gated on preconditions.
///
/// A condition that held when we wrote is not guaranteed to hold when we come
/// to remove — the user may have taken the provider block out from under our
/// key. Filtering here would leave our value behind in exactly that case, which
/// is the one where leaving it behind does the most damage. "Remove only what we
/// put there" is already the rule that bounds this, and it does not need help
/// from a precondition.
fn json_disconnect(agent: &AgentEntry, existing: &str) -> Result<Edit, Error> {
    let mut root = json_parse(existing)?;
    let mut changes = Vec::new();

    for setting in &agent.settings {
        let path: Vec<&str> = setting.key.split('.').collect();
        if let Slot::Ours(_) = json_slot(&root, &path) {
            json_remove(&mut root, &path);
            changes.push(format!("{}: removed", setting.key));
        }
    }

    Ok(Edit {
        contents: json_render(&root),
        changes,
        occupied: Vec::new(),
        skipped: Vec::new(),
    })
}

/// What is in a key path, from this module's point of view.
enum Slot {
    /// Nothing there, or a parent that does not exist yet.
    Empty,
    /// A value we wrote — loopback, our own façade path.
    Ours(String),
    /// Something else, with a description of it. Never overwritten.
    Blocked(String),
}

fn json_slot(root: &Map<String, Value>, path: &[&str]) -> Slot {
    let Some((leaf, parents)) = path.split_last() else {
        return Slot::Empty;
    };

    let mut current = root;
    for segment in parents {
        match current.get(*segment) {
            Some(Value::Object(next)) => current = next,
            // A parent that exists but is not an object is not ours to fix, and
            // saying so is more honest than silently replacing it.
            Some(_) => return Slot::Blocked(format!("`{segment}` is not an object")),
            None => return Slot::Empty,
        }
    }

    match current.get(*leaf) {
        None => Slot::Empty,
        Some(Value::String(value)) if points_at_us(value) => Slot::Ours(value.clone()),
        Some(Value::String(value)) => Slot::Blocked(value.clone()),
        Some(other) => Slot::Blocked(other.to_string()),
    }
}

/// Whether a key path resolves to anything at all in this document.
///
/// Presence, not value: `Slot` is about what we may write into a key and would
/// call a non-object parent `Blocked`, which is the wrong answer to "is the
/// provider block there". A block whose value is a string, a number or an empty
/// object is still a block the user put there.
fn json_present(root: &Map<String, Value>, path: &[&str]) -> bool {
    let Some((leaf, parents)) = path.split_last() else {
        return false;
    };
    let mut current = root;
    for segment in parents {
        match current.get(*segment) {
            Some(Value::Object(next)) => current = next,
            _ => return false,
        }
    }
    current.contains_key(*leaf)
}

/// Whether a key path resolves to anything at all in this table. As
/// [`json_present`].
fn toml_present(table: &toml::Table, key: &str) -> bool {
    let mut path = key.split('.').peekable();
    let mut current = table;
    while let Some(segment) = path.next() {
        if path.peek().is_none() {
            return current.contains_key(segment);
        }
        match current.get(segment) {
            Some(toml::Value::Table(next)) => current = next,
            _ => return false,
        }
    }
    false
}

/// Whether a URL is one of ours.
///
/// Loopback and one of our own façade paths, in any spelling we write. A port
/// change still counts as ours, which is what lets `connect` follow the daemon
/// to a new port instead of treating its own previous value as somebody else's
/// — and so does a change of [`UrlStyle`], so an entry corrected from the
/// origin form to the versioned one is rewritten rather than reported as
/// occupied.
fn points_at_us(url: &str) -> bool {
    our_facade_path(url).is_some()
}

/// Which of our façade paths a URL names, or `None` when it is not one of ours.
///
/// The port is not part of the answer: a daemon that moved to a new port still
/// wrote the value, and treating it as somebody else's is what would stop
/// `connect` following itself.
fn our_facade_path(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("http://127.0.0.1:")
        .or_else(|| url.strip_prefix("http://localhost:"))?;
    let (port, path) = rest.trim_end_matches('/').split_once('/')?;
    if port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let path = format!("/{path}");
    Facade::ALL
        .iter()
        .any(|facade| {
            UrlStyle::ALL
                .iter()
                .filter_map(|style| style.suffix())
                .any(|suffix| format!("{}{suffix}", facade.path()) == path)
        })
        .then_some(path)
}

/// Whether a value already in the file is the address this setting asks for.
///
/// Narrower than [`points_at_us`] on purpose — see [`is_wired`]. A style this
/// build cannot write has no address to compare against, so it is never
/// satisfied; `usable` has already refused such an entry, and answering "not
/// wired" is the safe half of the disagreement if the two ever part.
fn points_where_asked(url: &str, setting: &AgentSetting) -> bool {
    let Some(suffix) = setting.style.suffix() else {
        return false;
    };
    our_facade_path(url).is_some_and(|path| path == format!("{}{suffix}", setting.facade.path()))
}

fn json_insert(root: &mut Map<String, Value>, path: &[&str], value: &str) {
    let Some((leaf, parents)) = path.split_last() else {
        return;
    };
    let mut current = root;
    for segment in parents {
        current = current
            .entry((*segment).to_string())
            .or_insert_with(|| Value::Object(Map::new()))
            .as_object_mut()
            // `json_slot` reported a non-object parent as blocked, so this
            // branch is unreachable for any path we chose to write.
            .expect("parent is an object");
    }
    current.insert((*leaf).to_string(), Value::String(value.to_string()));
}

fn json_remove(root: &mut Map<String, Value>, path: &[&str]) {
    let Some((leaf, parents)) = path.split_last() else {
        return;
    };
    // Reborrowed, not moved: `prune_empty` needs the root again below.
    let mut current = &mut *root;
    for segment in parents {
        match current.get_mut(*segment).and_then(Value::as_object_mut) {
            Some(next) => current = next,
            None => return,
        }
    }
    current.remove(*leaf);

    // A container we emptied is one we created. Anything the user put beside
    // our key keeps it alive, which is the behaviour `claude_settings` has for
    // `env` and the reason it is worth doing here too.
    prune_empty(root, parents);
}

/// Drop containers that our own removal just emptied, deepest first.
///
/// Iterative, re-descending from the root each round: a recursive version
/// cannot hand out a `&mut` to a nested map and then mutate an ancestor.
fn prune_empty(root: &mut Map<String, Value>, parents: &[&str]) {
    for depth in (1..=parents.len()).rev() {
        let Some((last, above)) = parents[..depth].split_last() else {
            return;
        };
        let mut current = &mut *root;
        for segment in above {
            match current.get_mut(*segment).and_then(Value::as_object_mut) {
                Some(next) => current = next,
                None => return,
            }
        }
        let empty = current
            .get(*last)
            .and_then(Value::as_object)
            .is_some_and(Map::is_empty);
        if !empty {
            return;
        }
        current.remove(*last);
    }
}

fn json_parse(existing: &str) -> Result<Map<String, Value>, Error> {
    if existing.trim().is_empty() {
        return Ok(Map::new());
    }
    match serde_json::from_str::<Value>(existing) {
        Ok(Value::Object(map)) => Ok(map),
        Ok(_) => Err(Error::Unparseable(
            "the file is valid JSON but not an object".to_string(),
        )),
        Err(error) => {
            Err(jsonc(existing).map_or_else(|| Error::Unparseable(error.to_string()), Error::Jsonc))
        }
    }
}

/// The JSONC constructs taken out, and which ones were found.
///
/// Shared by the two questions that need it, so there is one lexer rather than
/// two that could come to disagree about what a comment is: [`jsonc`] asks in
/// order to name the construct in a refusal, and [`json_parse_tolerant`] asks
/// in order to read a file it will not write.
///
/// The returned text exists only to be parsed and dropped. Nothing in this
/// module writes it anywhere, and the module header's first rule depends on
/// that staying true.
struct Stripped {
    /// The text with the JSONC constructs removed. Parsed and dropped, never
    /// written -- see the module header's first rule.
    text: String,
    /// The first comment, and the line it is on.
    comment: Option<(usize, &'static str)>,
    /// The first trailing comma, and the line it is on.
    comma: Option<(usize, ())>,
}

fn strip_jsonc(existing: &str) -> Option<Stripped> {
    let (without_comments, comment) = strip_comments(existing.as_bytes());
    let (stripped, comma) = strip_trailing_commas(&without_comments);
    // Deletions only ever span whole ASCII tokens or whole comments, so this
    // holds; if it somehow did not, the original parse error stands.
    let text = String::from_utf8(stripped).ok()?;
    Some(Stripped {
        text,
        comment,
        comma,
    })
}

/// Parse for reading, tolerating the JSONC a strict parse refuses.
///
/// [`json_parse`] is the writer's parser and stays strict, because rewriting a
/// file we only half-understand is how a user loses their comments. Reading is
/// not that: there is no round trip, nothing is written back, and the worst a
/// wrong answer does here is describe the file inaccurately.
///
/// The asymmetry is the whole point, so it is worth stating in one place: this
/// function may be called from anything that only *asks* about a file, and
/// must not be called from anything that edits one.
///
/// Without this, a config a user wired by hand reads back as not wired for the
/// entire class of tools whose own format is JSONC — Zed ships a settings file
/// with eight lines of `//` in it, and opencode's published schema sets
/// `allowComments`. `is_wired` feeding the menu means the banner then offers
/// to connect a tool that is already connected, and the user's own correct
/// configuration is what triggers it.
///
/// Strict first, so a plain JSON file takes exactly the path it always did and
/// pays nothing for this.
fn json_parse_tolerant(existing: &str) -> Result<Map<String, Value>, Error> {
    match json_parse(existing) {
        Ok(map) => Ok(map),
        Err(strict) => {
            let Some(stripped) = strip_jsonc(existing) else {
                return Err(strict);
            };
            // Only an object is an answer. A file that strips down to a valid
            // JSON array or scalar is not a config this module could read the
            // keys of, and the strict error is the honest thing to return.
            match serde_json::from_str::<Value>(&stripped.text) {
                Ok(Value::Object(map)) => Ok(map),
                _ => Err(strict),
            }
        }
    }
}

/// Whether this file is JSONC rather than JSON, and what makes it so.
///
/// Only ever asked after `serde_json` has already refused the file. Two things
/// have to be true before we tell somebody their file is JSONC: it has to
/// contain a construct JSONC allows, *and* taking those constructs out has to
/// leave a valid JSON object. Without the second check a document that merely
/// contains a trailing comma gets classified by it — `{"a": nope,}` is refused
/// for the unquoted value, and naming the comma sends the user to fix
/// something that is not the blocker. So the constructs are stripped and the
/// result handed back to `serde_json`; if it still will not parse, this is not
/// a JSONC file and the original error stands.
///
/// The stripped text exists only to answer that question. It is never written
/// anywhere — see the module header. Refusing is still the answer; this only
/// decides what the refusal says, which is the difference between a user
/// editing one key by hand and a user thinking they have corrupted their
/// config. Zed's shipped `initial_user_settings.json` opens with eight lines
/// of `//` and closes two objects on a trailing comma; opencode's published
/// schema sets `allowComments` and `allowTrailingCommas` at the root. Both are
/// ordinary files their own tools read without complaint, and "expected value
/// at line 1 column 1" is the wrong thing to say about either.
///
/// A file that strips down to a valid JSON *value* but not an object is left
/// to the ordinary parse error: it is not a config file this module could have
/// edited even spelled as strict JSON.
fn jsonc(existing: &str) -> Option<Jsonc> {
    let Stripped {
        text: stripped,
        comment,
        comma,
    } = strip_jsonc(existing)?;

    // In file order, so the construct named is the first one a reader meets.
    let found = match (&comment, &comma) {
        (Some((line, text)), Some((comma_line, _))) if line <= comma_line => {
            format!("{text} on line {line}")
        }
        (_, Some((line, _))) => format!("a trailing comma on line {line}"),
        (Some((line, text)), None) => format!("{text} on line {line}"),
        (None, None) => return None,
    };

    matches!(
        serde_json::from_str::<Value>(&stripped),
        Ok(Value::Object(_))
    )
    .then(|| Jsonc {
        found,
        comments: comment.is_some(),
    })
}

/// Remove `//` and `/* */` comments, reporting the first one found.
///
/// Newlines inside a block comment are kept so that every later line keeps the
/// number it has in the file the user is looking at. String literals are
/// tracked because `"https://x"` is not a comment.
fn strip_comments(bytes: &[u8]) -> (Vec<u8>, Option<(usize, &'static str)>) {
    let mut out = Vec::with_capacity(bytes.len());
    let mut first = None;
    let mut line = 1usize;
    let mut i = 0usize;
    let mut in_string = false;
    let mut escaped = false;

    while i < bytes.len() {
        let byte = bytes[i];
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else if byte == b'"' {
            in_string = true;
        } else if byte == b'/' && bytes.get(i + 1) == Some(&b'/') {
            first.get_or_insert((line, "a `//` comment"));
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        } else if byte == b'/' && bytes.get(i + 1) == Some(&b'*') {
            first.get_or_insert((line, "a `/* */` comment"));
            i += 2;
            while i < bytes.len() && !(bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/')) {
                if bytes[i] == b'\n' {
                    line += 1;
                    out.push(b'\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }

        if byte == b'\n' {
            line += 1;
        }
        // A byte buffer, not a string: `byte as char` would re-encode
        // anything above ASCII. No byte we branch on above can appear inside
        // a multi-byte character, so copying byte by byte is safe.
        out.push(byte);
        i += 1;
    }

    (out, first)
}

/// Remove commas that sit immediately before a closing bracket, reporting the
/// first one found.
///
/// Run on comment-free text, so "immediately before" only has to look past
/// whitespace. String literals are tracked because `{"a": "b,"}` has no
/// trailing comma in it.
fn strip_trailing_commas(bytes: &[u8]) -> (Vec<u8>, Option<(usize, ())>) {
    let mut out = Vec::with_capacity(bytes.len());
    let mut first = None;
    let mut line = 1usize;
    let mut in_string = false;
    let mut escaped = false;
    // Where the last comma landed in `out`, and on which line of the file,
    // for as long as nothing but whitespace has followed it.
    let mut pending: Option<(usize, usize)> = None;

    for &byte in bytes {
        if in_string {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == b'"' {
                in_string = false;
            }
        } else {
            match byte {
                b'"' => {
                    in_string = true;
                    pending = None;
                }
                b',' => pending = Some((out.len(), line)),
                b'}' | b']' => {
                    if let Some((at, comma_line)) = pending.take() {
                        first.get_or_insert((comma_line, ()));
                        out.remove(at);
                    }
                }
                _ if byte.is_ascii_whitespace() => {}
                _ => pending = None,
            }
        }

        if byte == b'\n' {
            line += 1;
        }
        out.push(byte);
    }

    (out, first)
}

fn json_render(root: &Map<String, Value>) -> String {
    let mut out = serde_json::to_string_pretty(root).unwrap_or_else(|_| "{}".to_string());
    out.push('\n');
    out
}

// ---------------------------------------------------------------------- TOML
//
// Edited as *text*, for the reason `codex_config` gives: round-tripping a TOML
// file through a serializer deletes the user's comments, their ordering, and
// anything our types do not model. Parsing happens twice — once to refuse a
// file we cannot read, once on the result to refuse an edit we cannot stand
// behind — and the value is read back out of that second parse to prove it
// landed where we meant it to.
//
// The shapes this handles are the ones a tool's config actually uses: a
// top-level key, and a key inside a `[table]` or `[table.sub]` header. Anything
// else — a table written inline, a key already spelled as a dotted path, an
// array of tables — is reported rather than guessed at. Refusing is a worse
// user experience than succeeding and a much better one than corrupting a file.

fn toml_connect(agent: &AgentEntry, existing: &str, port: u16) -> Result<Edit, Error> {
    let before = toml_parse(existing)?;

    let mut out = existing.to_string();
    let mut changes = Vec::new();
    let mut occupied = Vec::new();
    let mut skipped = Vec::new();
    let mut written: Vec<(String, String)> = Vec::new();

    for setting in &agent.settings {
        // Against the file as found, not as being built — see `json_connect`.
        if !applies_here(setting, &|key| toml_present(&before, key)) {
            skipped.push(Skipped {
                slot: setting.key.clone(),
                requires: setting.requires.clone().unwrap_or_default(),
            });
            continue;
        }
        let url = value_for(setting, port)?;
        let table = toml_parse(&out)?;
        match toml_slot(&table, &setting.key) {
            Slot::Blocked(current) => {
                occupied.push(Occupied {
                    slot: setting.key.clone(),
                    current,
                });
                continue;
            }
            Slot::Ours(current) if current == url => continue,
            Slot::Ours(_) => match toml_replace(&out, &setting.key, &url) {
                Some(next) => {
                    out = next;
                    changes.push(format!("{}: updated to `{url}`", setting.key));
                    written.push((setting.key.clone(), url));
                }
                None => occupied.push(Occupied {
                    slot: setting.key.clone(),
                    current: "a shape IronWire will not edit by hand".to_string(),
                }),
            },
            Slot::Empty => match toml_insert(&out, &setting.key, &url) {
                Some(next) => {
                    out = next;
                    changes.push(format!("{}: `{url}` (added)", setting.key));
                    written.push((setting.key.clone(), url));
                }
                None => occupied.push(Occupied {
                    slot: setting.key.clone(),
                    current: "a table IronWire cannot safely extend".to_string(),
                }),
            },
        }
    }

    verify_written(&out, &written)?;
    Ok(Edit {
        contents: out,
        changes,
        occupied,
        skipped,
    })
}

fn toml_disconnect(agent: &AgentEntry, existing: &str) -> Result<Edit, Error> {
    toml_parse(existing)?;

    let mut out = existing.to_string();
    let mut changes = Vec::new();

    for setting in &agent.settings {
        let table = toml_parse(&out)?;
        if let Slot::Ours(_) = toml_slot(&table, &setting.key)
            && let Some(next) = toml_remove_line(&out, &setting.key)
        {
            out = next;
            changes.push(format!("{}: removed", setting.key));
        }
    }

    toml_parse(&out)?;
    Ok(Edit {
        contents: out,
        changes,
        occupied: Vec::new(),
        skipped: Vec::new(),
    })
}

/// The result must still parse, and every key we *wrote* must read back as the
/// value we wrote.
///
/// The read-back is the half that matters: a textual edit which produced valid
/// TOML with the value in the wrong table would otherwise look like a success.
/// Only written keys are checked — a slot we deliberately left alone is
/// supposed to be unchanged, and asserting on it would fail the very refusals
/// this module exists to make.
fn verify_written(contents: &str, written: &[(String, String)]) -> Result<(), Error> {
    let table = toml_parse(contents)?;
    for (key, url) in written {
        match toml_slot(&table, key) {
            Slot::Ours(current) if &current == url => {}
            _ => {
                return Err(Error::Unparseable(format!(
                    "after editing, `{key}` is not the value IronWire wrote — refusing to save"
                )));
            }
        }
    }
    Ok(())
}

fn toml_parse(contents: &str) -> Result<toml::Table, Error> {
    contents
        .parse::<toml::Table>()
        .map_err(|error| Error::Unparseable(error.to_string()))
}

fn toml_slot(table: &toml::Table, key: &str) -> Slot {
    let mut path = key.split('.').peekable();
    let mut current = table;
    while let Some(segment) = path.next() {
        if path.peek().is_none() {
            return match current.get(segment) {
                None => Slot::Empty,
                Some(toml::Value::String(value)) if points_at_us(value) => {
                    Slot::Ours(value.clone())
                }
                Some(toml::Value::String(value)) => Slot::Blocked(value.clone()),
                Some(other) => Slot::Blocked(other.to_string()),
            };
        }
        match current.get(segment) {
            Some(toml::Value::Table(next)) => current = next,
            Some(_) => return Slot::Blocked(format!("`{segment}` is not a table")),
            None => return Slot::Empty,
        }
    }
    Slot::Empty
}

/// The `[header]` line for a table path, if the file spells it that way.
///
/// `None` covers both "not there" and "there, but written inline or as a dotted
/// key" — neither of which this module will edit.
fn toml_header_line(contents: &str, table_path: &[&str]) -> Option<usize> {
    let wanted = format!("[{}]", table_path.join("."));
    contents.lines().position(|line| line.trim() == wanted)
}

/// Where a table's block ends: the next header, or end of file.
fn toml_block_end(lines: &[&str], header: usize) -> usize {
    lines
        .iter()
        .enumerate()
        .skip(header + 1)
        .find(|(_, line)| line.trim_start().starts_with('['))
        .map_or(lines.len(), |(index, _)| index)
}

/// The line index of `key = ...` within a range, ignoring comments.
fn toml_key_line(lines: &[&str], range: std::ops::Range<usize>, key: &str) -> Option<usize> {
    lines[range.clone()]
        .iter()
        .position(|line| {
            let trimmed = line.trim_start();
            !trimmed.starts_with('#')
                && trimmed
                    .strip_prefix(key)
                    .is_some_and(|rest| rest.trim_start().starts_with('='))
        })
        .map(|offset| range.start + offset)
}

fn toml_insert(contents: &str, key: &str, value: &str) -> Option<String> {
    let mut segments: Vec<&str> = key.split('.').collect();
    let leaf = segments.pop()?;
    let assignment = format!("{leaf} = \"{value}\"");
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();

    if segments.is_empty() {
        // A top-level key has to come before the first table header, or TOML
        // reads it as belonging to that table.
        let at = lines
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .unwrap_or(lines.len());
        lines.insert(at, assignment);
        return Some(joined(&lines));
    }

    match toml_header_line(contents, &segments) {
        Some(header) => {
            lines.insert(header + 1, assignment);
            Some(joined(&lines))
        }
        None => {
            // No header for it. Only safe to add one when nothing of that path
            // exists yet — otherwise it is spelled inline or dotted somewhere,
            // and a second definition is a parse error at best.
            let table = contents.parse::<toml::Table>().ok()?;
            if !matches!(toml_slot(&table, key), Slot::Empty) {
                return None;
            }
            let mut walked = &table;
            for segment in &segments {
                match walked.get(*segment) {
                    Some(toml::Value::Table(next)) => walked = next,
                    Some(_) => return None,
                    None => {
                        // Nothing of this path exists: a fresh header is safe.
                        let mut out = joined(&lines);
                        if !out.is_empty() && !out.ends_with('\n') {
                            out.push('\n');
                        }
                        out.push_str(&format!("\n[{}]\n{assignment}\n", segments.join(".")));
                        return Some(out);
                    }
                }
            }
            // The table exists but has no header line, so it is inline.
            None
        }
    }
}

fn toml_replace(contents: &str, key: &str, value: &str) -> Option<String> {
    let mut segments: Vec<&str> = key.split('.').collect();
    let leaf = segments.pop()?;
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();

    let range = if segments.is_empty() {
        0..borrowed
            .iter()
            .position(|line| line.trim_start().starts_with('['))
            .unwrap_or(borrowed.len())
    } else {
        let header = toml_header_line(contents, &segments)?;
        (header + 1)..toml_block_end(&borrowed, header)
    };

    let at = toml_key_line(&borrowed, range, leaf)?;
    let indent: String = lines[at]
        .chars()
        .take_while(|c| c.is_whitespace())
        .collect();
    lines[at] = format!("{indent}{leaf} = \"{value}\"");
    Some(joined(&lines))
}

fn toml_remove_line(contents: &str, key: &str) -> Option<String> {
    let mut segments: Vec<&str> = key.split('.').collect();
    let leaf = segments.pop()?;
    let mut lines: Vec<String> = contents.lines().map(str::to_string).collect();
    let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();

    let (range, header) = if segments.is_empty() {
        (
            0..borrowed
                .iter()
                .position(|line| line.trim_start().starts_with('['))
                .unwrap_or(borrowed.len()),
            None,
        )
    } else {
        let header = toml_header_line(contents, &segments)?;
        (
            (header + 1)..toml_block_end(&borrowed, header),
            Some(header),
        )
    };

    let at = toml_key_line(&borrowed, range.clone(), leaf)?;
    lines.remove(at);

    // A header whose block we just emptied is one we added. Anything the user
    // put in it keeps it, same as the JSON side.
    if let Some(header) = header {
        let borrowed: Vec<&str> = lines.iter().map(String::as_str).collect();
        let end = toml_block_end(&borrowed, header);
        let empty = borrowed[(header + 1)..end]
            .iter()
            .all(|line| line.trim().is_empty());
        if empty {
            lines.drain(header..end);
        }
    }
    Some(joined(&lines))
}

fn joined(lines: &[String]) -> String {
    let mut out = lines.join("\n");
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironwire_catalog::schema::ConfigLocation;

    fn agent(key: &str) -> AgentEntry {
        AgentEntry {
            id: "tool".to_string(),
            name: "A Tool".to_string(),
            enabled: true,
            detect: vec!["tool".to_string()],
            config: ConfigLocation {
                dir: vec![".tool".to_string()],
                file: "config.json".to_string(),
            },
            settings: vec![AgentSetting {
                key: key.to_string(),
                facade: Facade::Anthropic,
                style: UrlStyle::Origin,
                requires: None,
            }],
        }
    }

    /// The opencode shape: one key, gated on the provider block that shows the
    /// user already uses that provider.
    fn gated(key: &str, requires: &str) -> AgentEntry {
        let mut entry = agent(key);
        entry.settings[0].requires = Some(requires.to_string());
        entry
    }

    /// The whole point. A config with no `provider.anthropic` in it belongs to
    /// someone who does not use Anthropic in this tool, and writing the key
    /// would not merely be useless — it would promote the provider and change
    /// which model the tool picks.
    #[test]
    fn a_setting_whose_precondition_is_unmet_is_not_written() {
        let existing = r#"{"provider":{"openai":{"options":{"apiKey":"theirs"}}}}"#;
        let edit = connect(
            &gated("provider.anthropic.options.baseURL", "provider.anthropic"),
            existing,
            8463,
        )
        .expect("edits");

        assert!(edit.is_noop(), "{:?}", edit.changes);
        assert!(
            !edit.contents.contains("127.0.0.1"),
            "nothing of ours may reach the file: {}",
            edit.contents
        );
        assert!(
            edit.contents.contains("theirs"),
            "and their config is untouched: {}",
            edit.contents
        );
        assert_eq!(edit.skipped.len(), 1);
        assert_eq!(edit.skipped[0].slot, "provider.anthropic.options.baseURL");
        assert_eq!(edit.skipped[0].requires, "provider.anthropic");
        assert!(edit.occupied.is_empty(), "not occupied — inapplicable");
    }

    /// The other half, and the one that would go unnoticed if it broke: where
    /// the precondition holds, the edit is exactly what it was before this
    /// field existed.
    #[test]
    fn a_setting_whose_precondition_is_met_behaves_exactly_as_before() {
        let existing = r#"{"provider":{"anthropic":{"options":{"apiKey":"theirs"}}}}"#;
        let gated_edit = connect(
            &gated("provider.anthropic.options.baseURL", "provider.anthropic"),
            existing,
            8463,
        )
        .expect("edits");
        let ungated_edit =
            connect(&agent("provider.anthropic.options.baseURL"), existing, 8463).expect("edits");

        assert_eq!(gated_edit.contents, ungated_edit.contents);
        assert_eq!(gated_edit.changes, ungated_edit.changes);
        assert!(gated_edit.skipped.is_empty());
        assert!(
            gated_edit
                .contents
                .contains("http://127.0.0.1:8463/anthropic")
        );
    }

    /// An unmet precondition is not "wired". It is also not the vacuous truth
    /// an `all()` over an empty set would give: nothing of ours is in the file.
    #[test]
    fn an_entry_with_no_applicable_setting_is_not_wired() {
        let entry = gated("provider.anthropic.options.baseURL", "provider.anthropic");
        assert!(!is_wired(&entry, "{}"));
        assert!(!is_wired(&entry, r#"{"provider":{"openai":{}}}"#));
    }

    /// A precondition met and the value written reads back as wired, so the
    /// menu can show a checkmark for a tool that really is connected.
    #[test]
    fn a_met_precondition_with_our_value_reads_as_wired() {
        let entry = gated("provider.anthropic.options.baseURL", "provider.anthropic");
        let existing = r#"{"provider":{"anthropic":{"options":{"baseURL":"http://127.0.0.1:8463/anthropic"}}}}"#;
        assert!(is_wired(&entry, existing));
    }

    /// Presence, not value. A block the user left empty still says they use the
    /// provider, and a value test would be a bigger thing than this needs to be.
    #[test]
    fn presence_is_the_test_whatever_the_value_is() {
        let entry = gated("provider.anthropic.options.baseURL", "provider.anthropic");
        for existing in [
            r#"{"provider":{"anthropic":{}}}"#,
            r#"{"provider":{"anthropic":null}}"#,
            r#"{"provider":{"anthropic":"whatever they like"}}"#,
        ] {
            let edit = connect(&entry, existing, 8463).expect("edits");
            assert!(
                edit.skipped.is_empty(),
                "`provider.anthropic` is present in {existing}"
            );
        }
    }

    /// A condition that held when we wrote may not hold when we come to remove.
    /// Gating disconnect too would strand our value in the one case where
    /// leaving it does the most harm.
    #[test]
    fn disconnect_removes_our_value_even_once_the_precondition_stops_holding() {
        let entry = gated("provider.anthropic.options.baseURL", "provider.anthropic");
        // The user took the block out from under our key — bar the key itself.
        let existing = r#"{"other":{"anthropic":{}},"provider":{"anthropic":{"options":{"baseURL":"http://127.0.0.1:8463/anthropic"}}}}"#;
        let edit = disconnect(&entry, existing).expect("edits");
        assert_eq!(edit.changes.len(), 1);
        assert!(
            !edit.contents.contains("127.0.0.1"),
            "ours must be gone: {}",
            edit.contents
        );
    }

    /// Preconditions are answered against the file as found. Otherwise a
    /// setting could satisfy the condition of one listed after it, and the
    /// entry would mean different things depending on the order of its own
    /// settings.
    #[test]
    fn one_setting_cannot_satisfy_the_precondition_of_another() {
        let mut entry = agent("provider.anthropic.options.baseURL");
        entry.settings[0].requires = None;
        entry.settings.push(AgentSetting {
            key: "provider.openai.options.baseURL".to_string(),
            facade: Facade::OpenAi,
            style: UrlStyle::Versioned,
            // Satisfied only by what the *first* setting is about to write.
            requires: Some("provider.anthropic".to_string()),
        });

        let edit = connect(&entry, "{}", 8463).expect("edits");
        assert_eq!(
            edit.changes.len(),
            1,
            "only the ungated one: {:?}",
            edit.changes
        );
        assert_eq!(edit.skipped.len(), 1);
        assert_eq!(edit.skipped[0].slot, "provider.openai.options.baseURL");
    }

    /// TOML gets the same rule, not a second one that drifts from it.
    #[test]
    fn the_precondition_applies_to_toml_too() {
        let mut entry = gated("providers.anthropic.base_url", "providers.anthropic");
        entry.config.file = "config.toml".to_string();

        let absent = connect(&entry, "", 8463).expect("edits");
        assert!(absent.is_noop(), "{:?}", absent.changes);
        assert_eq!(absent.skipped.len(), 1);

        let present =
            connect(&entry, "[providers.anthropic]\nname = \"theirs\"\n", 8463).expect("edits");
        assert_eq!(present.changes.len(), 1);
        assert!(present.skipped.is_empty());
        assert!(present.contents.contains("127.0.0.1:8463/anthropic"));
    }

    #[test]
    fn an_empty_slot_is_filled_with_our_own_loopback_url() {
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), "{}", 8463).expect("edits");
        assert!(edit.contents.contains("http://127.0.0.1:8463/anthropic"));
        assert!(edit.occupied.is_empty());
        assert_eq!(edit.changes.len(), 1);
    }

    /// The rule that makes editing someone else's config acceptable: a value
    /// they put there stays, and they are told rather than surprised.
    #[test]
    fn a_slot_someone_else_is_using_is_reported_and_left_alone() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"https://my-own-proxy.example"}}"#;
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), existing, 8463).expect("edits");
        assert!(edit.is_noop(), "{:?}", edit.changes);
        assert_eq!(edit.occupied.len(), 1);
        assert_eq!(edit.occupied[0].current, "https://my-own-proxy.example");
        assert!(edit.contents.contains("my-own-proxy.example"));
    }

    /// A port change is still ours to follow — otherwise the daemon moving
    /// would look like somebody else's value and never be corrected.
    #[test]
    fn our_own_value_is_followed_to_a_new_port() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9999/anthropic"}}"#;
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), existing, 8463).expect("edits");
        assert_eq!(edit.changes.len(), 1);
        assert!(edit.contents.contains("127.0.0.1:8463/anthropic"));
        assert!(edit.occupied.is_empty());
    }

    #[test]
    fn everything_the_user_wrote_survives_the_edit() {
        let existing = r#"{"theme":"dark","env":{"MY_KEY":"mine"}}"#;
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), existing, 8463).expect("edits");
        assert!(edit.contents.contains("\"theme\": \"dark\""));
        assert!(edit.contents.contains("\"MY_KEY\": \"mine\""));
    }

    #[test]
    fn a_file_we_cannot_read_is_never_rewritten() {
        let error = connect(&agent("env.X"), "{ not json", 8463).expect_err("refuses");
        assert!(matches!(error, Error::Unparseable(_)));
    }

    /// Zed's own `initial_user_settings.json` — the file it writes the first
    /// time it starts, verbatim. Eight lines of `//`, and both objects closed
    /// on a trailing comma.
    const ZED_INITIAL_USER_SETTINGS: &str = r#"// Zed settings
//
// For information on how to configure Zed, see the Zed
// documentation: https://zed.dev/docs/configuring-zed
//
// To see all of Zed's default settings without changing your
// custom settings, run `zed: open default settings` from the
// command palette (cmd-shift-p / ctrl-shift-p)
{
  "ui_font_size": 16,
  "buffer_font_size": 15,
  "theme": {
    "mode": "system",
    "light": "One Light",
    "dark": "One Dark",
  },
}
"#;

    /// The refusal is right — the file cannot be written back without losing
    /// those comments. Calling it a syntax error is not: the tool that wrote
    /// the file reads it every time it starts.
    #[test]
    fn a_json_file_with_comments_is_refused_as_jsonc_and_not_as_a_syntax_error() {
        let error = connect(&agent("env.X"), ZED_INITIAL_USER_SETTINGS, 8463).expect_err("refuses");
        let Error::Jsonc(jsonc) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(jsonc.found, "a `//` comment on line 1");
        assert!(jsonc.comments);
        assert!(error.to_string().contains("delete the comments"));
    }

    /// A file whose own tool calls it valid must not read as unwired just
    /// because the writer would refuse to edit it.
    ///
    /// The refusal is right and stays. The answer to "is this already wired"
    /// is a different question with no round trip in it, and getting it wrong
    /// costs the user a banner offering to connect a tool they connected
    /// themselves -- triggered by their own correct configuration.
    #[test]
    fn a_hand_wired_jsonc_file_reads_as_wired() {
        let entry = agent("env.X");
        let strict = r#"{"env":{"X":"http://127.0.0.1:8463/anthropic"}}"#;
        assert!(
            is_wired(&entry, strict),
            "the strict spelling is the control"
        );

        for (name, existing) in [
            (
                "line comment",
                "// mine\n{\"env\":{\"X\":\"http://127.0.0.1:8463/anthropic\"}}\n",
            ),
            (
                "block comment",
                "/* mine */\n{\"env\":{\"X\":\"http://127.0.0.1:8463/anthropic\"}}\n",
            ),
            (
                "trailing comma",
                "{\"env\":{\"X\":\"http://127.0.0.1:8463/anthropic\",},}\n",
            ),
            (
                "all three, as Zed and opencode actually ship them",
                "// Settings\n/* and a block */\n{\n  \"env\": {\n    \"X\": \"http://127.0.0.1:8463/anthropic\",\n  },\n}\n",
            ),
        ] {
            assert!(is_wired(&entry, existing), "{name} should read as wired");
        }
    }

    /// Tolerance is not a blanket yes.
    ///
    /// A reader that answered "wired" for anything it could not parse would
    /// hide the tools that need connecting, which is the same failure as this
    /// change fixes, pointing the other way.
    #[test]
    fn tolerance_does_not_make_every_file_read_as_wired() {
        let entry = agent("env.X");
        for (name, existing) in [
            ("JSONC, but ours is absent", "// mine\n{\"env\":{}}\n"),
            (
                "JSONC, but the value is somebody else's",
                "// mine\n{\"env\":{\"X\":\"https://api.anthropic.com\"},}\n",
            ),
            (
                "not JSON at all once the comments come out",
                "// mine\n{\"env\": nope,}\n",
            ),
            ("truncated", "// mine\n{\"env\":{\"X\":\n"),
            ("an array, not a config object", "// mine\n[1, 2,]\n"),
            ("empty once stripped", "// only a comment\n"),
        ] {
            assert!(!is_wired(&entry, existing), "{name} must not read as wired");
        }
    }

    /// The writers keep the strict parser, which is the whole asymmetry:
    /// reading a JSONC file loses nothing, rewriting one deletes the comments.
    ///
    /// Tying the two answers together in one test means a later change that
    /// makes `connect` tolerant has to delete this to land, rather than
    /// quietly widening what gets rewritten.
    #[test]
    fn reading_a_jsonc_file_is_tolerant_and_writing_it_is_still_refused() {
        let entry = agent("env.X");
        let wired = "// mine\n{\"env\":{\"X\":\"http://127.0.0.1:8463/anthropic\"},}\n";
        assert!(is_wired(&entry, wired));
        let error = connect(&entry, wired, 8463).expect_err("still refuses to rewrite it");
        assert!(matches!(error, Error::Jsonc(_)), "{error:?}");
    }

    /// opencode's published schema sets `allowTrailingCommas` on its own, so a
    /// file with one and no comments is still a file its tool reads.
    ///
    /// The advice has to match the file. Comment loss is not why this one is
    /// refused — there is nothing in it to lose — so the message must not say
    /// so, or the user follows it and nothing changes.
    #[test]
    fn a_trailing_comma_alone_is_reported_without_blaming_comments() {
        let existing = "{\n  \"a\": 1,\n  \"b\": 2,\n}\n";
        let error = connect(&agent("env.X"), existing, 8463).expect_err("refuses");
        let Error::Jsonc(jsonc) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(jsonc.found, "a trailing comma on line 3");
        assert!(!jsonc.comments);
        assert!(!error.to_string().contains("comment"), "{error}");
    }

    /// A trailing comma is not what stops this file parsing — `nope` is. Being
    /// told to remove the comma sends the user to fix something that is not
    /// the blocker, and leaves them no better off when they do.
    #[test]
    fn a_broken_file_that_happens_to_have_a_trailing_comma_is_not_jsonc() {
        let error = connect(&agent("env.X"), r#"{"a": nope,}"#, 8463).expect_err("refuses");
        let Error::Unparseable(detail) = &error else {
            panic!("{error:?}");
        };
        assert!(detail.contains("line 1 column 8"), "{detail}");
    }

    /// The same, with the comment arm: a file with a real comment in it is
    /// still not JSONC if it would not parse once the comment is gone.
    #[test]
    fn a_broken_file_that_happens_to_have_a_comment_is_not_jsonc() {
        let existing = "// a note\n{\"a\": 1,, \"b\": 2}\n";
        let error = connect(&agent("env.X"), existing, 8463).expect_err("refuses");
        assert!(matches!(error, Error::Unparseable(_)), "{error:?}");
    }

    /// Stripping happens on a copy, to answer a question. A comment inside a
    /// string is part of a value and must survive it, or a legal JSONC file
    /// gets called broken for a `//` its tool never treated as syntax.
    #[test]
    fn a_comment_marker_inside_a_value_does_not_break_the_check() {
        let existing = "// note\n{\"env\": {\"X\": \"https://example.com\"},}\n";
        let error = connect(&agent("env.X"), existing, 8463).expect_err("refuses");
        let Error::Jsonc(jsonc) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(jsonc.found, "a `//` comment on line 1");
    }

    /// Line numbers have to point into the file the user is looking at, so a
    /// block comment's newlines survive the strip that removes its text.
    #[test]
    fn a_line_number_after_a_block_comment_still_matches_the_file() {
        let existing = "{\n  /* one\n     two */\n  \"a\": 1,\n}\n";
        let error = connect(&agent("env.X"), existing, 8463).expect_err("refuses");
        let Error::Jsonc(jsonc) = &error else {
            panic!("{error:?}");
        };
        assert_eq!(jsonc.found, "a `/* */` comment on line 2");
    }

    /// The scanner must not read a value as syntax. A URL has `//` in it and a
    /// string can end in a comma; neither makes the file JSONC, and a file that
    /// is merely broken has to keep saying so.
    #[test]
    fn a_url_or_a_comma_inside_a_string_is_not_a_comment() {
        let existing = r#"{"env":{"X":"https://example.com/a","Y":"b,"} "#;
        let error = connect(&agent("env.X"), existing, 8463).expect_err("refuses");
        assert!(matches!(error, Error::Unparseable(_)), "{error:?}");
    }

    /// Valid JSON is never routed through the JSONC arm — commas inside an
    /// object and slashes inside values are ordinary.
    #[test]
    fn valid_json_is_still_edited() {
        let existing = r#"{"a":1,"url":"https://example.com//x"}"#;
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), existing, 8463).expect("edits");
        assert!(edit.contents.contains("http://127.0.0.1:8463/anthropic"));
    }

    #[test]
    fn a_parent_that_is_not_an_object_is_reported_rather_than_replaced() {
        let existing = r#"{"env":"a string the user put here"}"#;
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), existing, 8463).expect("edits");
        assert!(edit.is_noop());
        assert_eq!(edit.occupied.len(), 1);
        assert!(edit.contents.contains("a string the user put here"));
    }

    #[test]
    fn disconnect_removes_ours_and_nothing_else() {
        let existing =
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic","MY_KEY":"mine"}}"#;
        let edit = disconnect(&agent("env.ANTHROPIC_BASE_URL"), existing).expect("edits");
        assert_eq!(edit.changes.len(), 1);
        assert!(!edit.contents.contains("ANTHROPIC_BASE_URL"));
        assert!(edit.contents.contains("MY_KEY"));
    }

    #[test]
    fn disconnect_leaves_a_value_that_was_never_ours() {
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"https://my-own-proxy.example"}}"#;
        let edit = disconnect(&agent("env.ANTHROPIC_BASE_URL"), existing).expect("edits");
        assert!(edit.is_noop());
        assert!(edit.contents.contains("my-own-proxy.example"));
    }

    /// A container that only ever held our key goes with it; one the user was
    /// also using stays.
    #[test]
    fn a_container_we_emptied_is_removed_and_a_shared_one_is_kept() {
        let ours_only = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic"}}"#;
        let edit = disconnect(&agent("env.ANTHROPIC_BASE_URL"), ours_only).expect("edits");
        assert!(!edit.contents.contains("env"), "{}", edit.contents);

        let shared =
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic","MY_KEY":"x"}}"#;
        let edit = disconnect(&agent("env.ANTHROPIC_BASE_URL"), shared).expect("edits");
        assert!(edit.contents.contains("env"));
    }

    #[test]
    fn a_toml_table_is_created_when_nothing_of_it_exists() {
        let edit = connect(&toml_agent("providers.ironwire.base_url"), "", 8463).expect("edits");
        assert!(edit.contents.contains("[providers.ironwire]"));
        assert!(
            edit.contents
                .contains("base_url = \"http://127.0.0.1:8463/anthropic\"")
        );
        assert!(edit.contents.parse::<toml::Table>().is_ok());
    }

    fn toml_agent(key: &str) -> AgentEntry {
        let mut entry = agent(key);
        entry.config.file = "config.toml".to_string();
        entry
    }

    #[test]
    fn a_key_goes_into_a_table_that_already_has_a_header() {
        let existing = "# mine\n[providers.ironwire]\nwire_api = \"responses\"\n";
        let edit = connect(&toml_agent("providers.ironwire.base_url"), existing, 8463).expect("e");
        assert!(edit.contents.contains("# mine"), "{}", edit.contents);
        assert!(edit.contents.contains("wire_api = \"responses\""));
        let parsed: toml::Table = edit.contents.parse().expect("valid");
        assert_eq!(
            parsed["providers"]["ironwire"]["base_url"].as_str(),
            Some("http://127.0.0.1:8463/anthropic")
        );
    }

    /// The whole reason this is a text edit and not a round trip.
    #[test]
    fn comments_and_ordering_survive() {
        let existing = "# a comment the user wrote\nmodel = \"x\"\n\n[other]\nkeep = 1\n";
        let edit = connect(&toml_agent("providers.p.base_url"), existing, 8463).expect("edits");
        assert!(edit.contents.contains("# a comment the user wrote"));
        assert!(edit.contents.contains("model = \"x\""));
        assert!(edit.contents.contains("[other]"));
        assert!(edit.contents.contains("keep = 1"));
    }

    #[test]
    fn a_toml_slot_someone_else_is_using_is_left_alone() {
        let existing = "[providers.ironwire]\nbase_url = \"https://their-proxy.example\"\n";
        let edit = connect(&toml_agent("providers.ironwire.base_url"), existing, 8463).expect("e");
        assert!(edit.is_noop());
        assert_eq!(edit.occupied.len(), 1);
        assert!(edit.contents.contains("their-proxy.example"));
    }

    #[test]
    fn our_own_toml_value_is_followed_to_a_new_port() {
        let existing = "[providers.ironwire]\nbase_url = \"http://127.0.0.1:1111/anthropic\"\n";
        let edit = connect(&toml_agent("providers.ironwire.base_url"), existing, 8463).expect("e");
        assert_eq!(edit.changes.len(), 1);
        let parsed: toml::Table = edit.contents.parse().expect("valid");
        assert_eq!(
            parsed["providers"]["ironwire"]["base_url"].as_str(),
            Some("http://127.0.0.1:8463/anthropic")
        );
    }

    /// A table written inline cannot be extended by adding a header — that
    /// would be a second definition and a parse error. Reported, not attempted.
    #[test]
    fn an_inline_table_is_reported_rather_than_given_a_second_definition() {
        let existing = "providers = { ironwire = { wire_api = \"responses\" } }\n";
        let edit = connect(&toml_agent("providers.ironwire.base_url"), existing, 8463).expect("e");
        assert!(edit.is_noop(), "{:?}", edit.changes);
        assert_eq!(edit.occupied.len(), 1);
        assert_eq!(edit.contents, existing);
    }

    #[test]
    fn a_top_level_key_lands_before_the_first_table_header() {
        let existing = "[other]\nkeep = 1\n";
        let edit = connect(&toml_agent("base_url"), existing, 8463).expect("edits");
        let parsed: toml::Table = edit.contents.parse().expect("valid");
        // If it landed after the header it would belong to `other`.
        assert!(parsed.get("base_url").is_some(), "{}", edit.contents);
        assert_eq!(parsed["other"]["keep"].as_integer(), Some(1));
    }

    #[test]
    fn a_toml_file_we_cannot_read_is_never_rewritten() {
        let error = connect(&toml_agent("a.b"), "not = = toml", 8463).expect_err("refuses");
        assert!(matches!(error, Error::Unparseable(_)));
    }

    #[test]
    fn toml_disconnect_removes_ours_and_the_header_it_emptied() {
        let existing = "[providers.ironwire]\nbase_url = \"http://127.0.0.1:8463/anthropic\"\n";
        let edit = disconnect(&toml_agent("providers.ironwire.base_url"), existing).expect("e");
        assert_eq!(edit.changes.len(), 1);
        assert!(!edit.contents.contains("base_url"));
        assert!(
            !edit.contents.contains("[providers.ironwire]"),
            "{}",
            edit.contents
        );
    }

    #[test]
    fn toml_disconnect_keeps_a_table_the_user_is_also_using() {
        let existing = "[providers.ironwire]\nbase_url = \"http://127.0.0.1:8463/anthropic\"\nwire_api = \"responses\"\n";
        let edit = disconnect(&toml_agent("providers.ironwire.base_url"), existing).expect("e");
        assert!(edit.contents.contains("[providers.ironwire]"));
        assert!(edit.contents.contains("wire_api"));
        assert!(!edit.contents.contains("base_url"));
    }

    #[test]
    fn toml_disconnect_leaves_a_value_that_was_never_ours() {
        let existing = "[providers.ironwire]\nbase_url = \"https://their-proxy.example\"\n";
        let edit = disconnect(&toml_agent("providers.ironwire.base_url"), existing).expect("e");
        assert!(edit.is_noop());
        assert!(edit.contents.contains("their-proxy.example"));
    }

    // ------------------------------------------------------------- detection

    #[test]
    fn a_tool_with_a_config_directory_is_detected_even_when_it_is_not_on_path() {
        let home = tempfile::tempdir().expect("tempdir");
        std::fs::create_dir_all(home.path().join(".tool")).expect("mkdir");
        // An app bundle or a version-manager shim leaves no name on PATH.
        assert!(detected(
            &agent("env.ANTHROPIC_BASE_URL"),
            home.path(),
            &|_| false
        ));
    }

    #[test]
    fn a_tool_on_path_is_detected_before_it_has_ever_been_run() {
        let home = tempfile::tempdir().expect("tempdir");
        // No config directory yet — it has not been started.
        assert!(detected(
            &agent("env.ANTHROPIC_BASE_URL"),
            home.path(),
            &|name| name == "tool"
        ));
    }

    #[test]
    fn a_tool_that_is_neither_is_not_detected() {
        let home = tempfile::tempdir().expect("tempdir");
        assert!(!detected(
            &agent("env.ANTHROPIC_BASE_URL"),
            home.path(),
            &|_| false
        ));
    }

    /// Offering to wire a tool and then refusing at the file is worse than
    /// never offering, so an entry we would not act on is not "present".
    #[test]
    fn an_entry_we_would_refuse_to_write_is_never_reported_as_installed() {
        let home = tempfile::tempdir().expect("tempdir");
        let mut escaping = agent("env.ANTHROPIC_BASE_URL");
        escaping.config.dir = vec!["..".to_string()];
        assert!(!detected(&escaping, home.path(), &|_| true));
    }

    /// A catalog entry that failed validation must not reach a file at all.
    #[test]
    fn an_unusable_entry_never_reaches_the_filesystem() {
        let mut escaping = agent("env.X");
        escaping.config.dir = vec!["..".to_string()];
        let error = connect(&escaping, "{}", 8463).expect_err("refuses");
        assert!(matches!(error, Error::Unusable(_)));
    }

    #[test]
    fn an_absent_file_starts_from_an_empty_object() {
        let edit = connect(&agent("env.ANTHROPIC_BASE_URL"), "", 8463).expect("edits");
        assert!(edit.contents.contains("127.0.0.1:8463/anthropic"));
    }

    // ------------------------------------------------------------ URL styles

    /// The proof that the gap is closed, stated as the thing that measures it:
    /// the base URL `codex_config` writes by hand is now something a catalog
    /// entry can ask for. Compared against that module's own output rather than
    /// a copy of the string, so the two cannot drift apart silently.
    #[test]
    fn the_built_in_codex_base_url_is_expressible_through_the_catalog() {
        let built_in = crate::codex_config::connect("", 8463).expect("edits");
        let built_in: toml::Table = built_in.contents.parse().expect("valid TOML");
        let expected = built_in["model_providers"]["ironwire"]["base_url"]
            .as_str()
            .expect("a base_url")
            .to_string();

        let mut entry = toml_agent("model_providers.ironwire.base_url");
        entry.settings[0].facade = Facade::OpenAi;
        entry.settings[0].style = UrlStyle::Versioned;

        let edit = connect(&entry, "", 8463).expect("edits");
        let parsed: toml::Table = edit.contents.parse().expect("valid TOML");
        assert_eq!(
            parsed["model_providers"]["ironwire"]["base_url"].as_str(),
            Some(expected.as_str()),
            "{}",
            edit.contents
        );
        assert_eq!(expected, "http://127.0.0.1:8463/openai/v1");
    }

    /// A tool that wants the versioned form and is holding the origin form is
    /// holding a value we wrote, not somebody else's — so it is corrected
    /// rather than reported as occupied. Otherwise a catalog fixing a tool's
    /// entry would leave every existing install pointed at a 404.
    #[test]
    fn our_own_value_is_corrected_to_the_spelling_the_tool_wants() {
        let mut entry = agent("provider.openai.options.baseURL");
        entry.settings[0].facade = Facade::OpenAi;
        entry.settings[0].style = UrlStyle::Versioned;

        let existing = r#"{"provider":{"openai":{"options":
                          {"baseURL":"http://127.0.0.1:8463/openai"}}}}"#;
        let edit = connect(&entry, existing, 8463).expect("edits");
        assert!(edit.occupied.is_empty(), "{:?}", edit.occupied);
        assert!(
            edit.contents.contains("http://127.0.0.1:8463/openai/v1"),
            "{}",
            edit.contents
        );
    }

    /// And it is still only ours that we remove.
    #[test]
    fn disconnect_removes_a_versioned_value_we_wrote() {
        let mut entry = agent("provider.openai.options.baseURL");
        entry.settings[0].facade = Facade::OpenAi;
        entry.settings[0].style = UrlStyle::Versioned;

        let existing = r#"{"provider":{"openai":{"options":
                          {"baseURL":"http://127.0.0.1:8463/openai/v1"}}}}"#;
        let edit = disconnect(&entry, existing).expect("edits");
        assert_eq!(edit.changes.len(), 1);
        assert!(!edit.contents.contains("baseURL"), "{}", edit.contents);
    }

    /// Someone else's `/v1` base URL is still someone else's.
    #[test]
    fn a_versioned_url_that_is_not_loopback_is_not_ours() {
        assert!(!points_at_us("https://api.openai.com/v1"));
        assert!(!points_at_us("http://127.0.0.1:8463/openai/v2"));
        assert!(points_at_us("http://127.0.0.1:8463/openai/v1"));
        assert!(points_at_us("http://127.0.0.1:8463/anthropic/v1"));
    }

    /// The state the whole `style` field exists to reach: a catalog corrects an
    /// entry to the spelling the tool actually needs, and the machine it lands
    /// on already has our older value in that key.
    ///
    /// If this reports wired, nothing ever calls `connect`: the menu's
    /// onboarding banner offers only unwired tools and its toggle is already
    /// checked, so the correction can never take effect and the tool keeps
    /// requesting a path the façade does not serve.
    #[test]
    fn a_value_in_the_old_spelling_is_not_wired_once_the_entry_asks_for_the_new_one() {
        let mut entry = agent("provider.openai.options.baseURL");
        entry.settings[0].facade = Facade::OpenAi;
        entry.settings[0].style = UrlStyle::Versioned;

        let existing = r#"{"provider":{"openai":{"options":
                          {"baseURL":"http://127.0.0.1:8463/openai"}}}}"#;
        assert!(
            !is_wired(&entry, existing),
            "the origin spelling satisfied an entry asking for the versioned one"
        );

        // And the value it asks for does satisfy it, so the correction settles
        // rather than being offered again every time the menu opens.
        let corrected = connect(&entry, existing, 8463).expect("edits").contents;
        assert!(is_wired(&entry, &corrected), "{corrected}");
    }

    /// The same for TOML, which is the format the tools that want `/v1`
    /// actually use — Codex's `config.toml` is the case in hand.
    #[test]
    fn the_toml_reader_asks_the_same_question() {
        let mut entry = toml_agent("model_providers.ironwire.base_url");
        entry.settings[0].facade = Facade::OpenAi;
        entry.settings[0].style = UrlStyle::Versioned;

        let origin = "[model_providers.ironwire]\nbase_url = \"http://127.0.0.1:8463/openai\"\n";
        assert!(!is_wired(&entry, origin));

        let corrected = connect(&entry, origin, 8463).expect("edits").contents;
        assert!(is_wired(&entry, &corrected), "{corrected}");
    }

    /// The narrowing is about the spelling, not about the port. A daemon that
    /// moved is still wired — `connect` follows itself across a port, and a
    /// menu that called that "not routed" would be answering a question nobody
    /// asked.
    #[test]
    fn a_port_we_have_since_moved_off_is_still_wired() {
        let entry = agent("env.ANTHROPIC_BASE_URL");
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:9999/anthropic"}}"#;
        assert!(is_wired(&entry, existing));
    }

    /// Our own value for the *other* façade is ours, and still not what this
    /// entry asked for. Same defect as the style mismatch, and the same answer:
    /// recognised, so `connect` will rewrite it; not wired, so something will.
    #[test]
    fn our_own_value_for_another_facade_does_not_satisfy_this_one() {
        let entry = agent("env.ANTHROPIC_BASE_URL");
        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/openai"}}"#;
        assert!(points_at_us("http://127.0.0.1:8463/openai"));
        assert!(!is_wired(&entry, existing));
    }

    /// Two settings, one already correct and one still in the old spelling.
    /// Half-wired is not wired — the rule this function already had for a key
    /// the user was using, applied to a key that is ours but stale.
    #[test]
    fn one_stale_setting_is_enough_to_be_unwired() {
        let mut entry = agent("env.ANTHROPIC_BASE_URL");
        entry.settings.push(AgentSetting {
            key: "env.OPENAI_BASE_URL".to_string(),
            facade: Facade::OpenAi,
            style: UrlStyle::Versioned,
            requires: None,
        });

        let existing = r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic",
                                 "OPENAI_BASE_URL":"http://127.0.0.1:8463/openai"}}"#;
        assert!(!is_wired(&entry, existing));
    }
}
