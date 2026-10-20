//! Editing `~/.codex/config.toml` without destroying it.
//!
//! Codex's config is a file the user owns and edits by hand. Round-tripping it
//! through a TOML serializer would silently delete their comments, their
//! ordering, and any key our struct does not model — which is a rude thing to do
//! to someone's config in exchange for setting two lines.
//!
//! So this module edits *text*, and parses only to check the result is still
//! valid TOML before anything is written.

use std::path::{Path, PathBuf};

use crate::endpoint;

/// The table IronWire owns. Outside it, the only lines this module writes are
/// the `model_provider` selection — and that one only when it is empty or
/// already ours — and the note above it recording what was selected before.
const BLOCK_HEADER: &str = "[model_providers.ironwire]";

/// The one key outside our block that decides where Codex sends calls.
const MODEL_PROVIDER: &str = "model_provider";

/// Marker recording the `model_provider` that was there before IronWire, so
/// `ironwire disconnect codex` can put that value back.
///
/// Two kinds of config carry one, and they mean the same thing. Older versions
/// wrote it when they replaced the provider outright. This one writes it when
/// it leaves a provider alone and asks the user to select IronWire by hand:
/// the file then ends up in the same state by their edit rather than ours, and
/// owes the same undo.
const PREVIOUS_MARKER: &str = "# ironwire: previous model_provider =";

/// A slot that already held something the user put there.
///
/// The same shape, and the same meaning, as [`crate::claude_settings::Occupied`]
/// and the catalog's: their value stays, and the caller says what they could do
/// by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Occupied {
    /// Which setting — `model_provider` is the only one this module can leave.
    pub slot: &'static str,
    /// What is in it, so the caller can name it back to them.
    pub current: String,
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
}

impl Edit {
    /// Whether this edit would change anything at all.
    pub fn is_noop(&self) -> bool {
        self.changes.is_empty()
    }

    /// What is in a slot we left alone, if we left that one alone.
    pub fn occupied_slot(&self, slot: &str) -> Option<&str> {
        self.occupied
            .iter()
            .find(|o| o.slot == slot)
            .map(|o| o.current.as_str())
    }
}

/// Produce the `[model_providers.ironwire]` block for a base URL.
fn block(base_url: &str) -> String {
    // `wire_api = "responses"` matters: Codex's Responses path is the one that
    // carries reasoning state, and downgrading it to chat completions would
    // quietly lose that. `env_key` is deliberately absent — IronWire supplies
    // the credential itself and strips whatever the client sends
    // (`docs/PROTOCOL.md` §2).
    format!(
        "{BLOCK_HEADER}\n\
         name = \"IronWire\"\n\
         base_url = {}\n\
         wire_api = \"responses\"\n",
        quoted(base_url)
    )
}

/// Compute the edit that points Codex at IronWire.
///
/// Two slots, and only one of them is ever ours: `[model_providers.ironwire]`,
/// which nothing reads until it is selected, and the top-level
/// `model_provider`, which is what actually routes Codex. A `model_provider`
/// the user already chose is another proxy or a deliberate choice, so it is
/// reported and left — the same rule [`crate::claude_settings`] follows for
/// `ANTHROPIC_BASE_URL` and the catalog follows for every key it describes.
/// The block is still written either way, so selecting IronWire afterwards is
/// one word rather than four lines -- and a note above the provider records
/// what it says today, so that manual switch keeps the undo the automatic one
/// used to have.
///
/// # Errors
///
/// Returns the parse error when the existing file is not valid TOML — we will
/// not append to a file we cannot read, because the user's own syntax error
/// would then look like ours.
pub fn connect(existing: &str, port: u16) -> Result<Edit, toml::de::Error> {
    connect_as(existing, port, None)
}

/// [`connect`], for the Codex that runs with a `CODEX_HOME` of its own: the
/// block's `base_url` names `account`, the login that Codex is using, so the
/// daemon serves it from that login's subscription.
///
/// `None` is the plain façade — the default login. It does not unbind a block
/// that already names a login, for the reason
/// [`crate::claude_settings::connect`] gives: a connect that names no login
/// moving that Codex onto the default login's subscription would be a change
/// nobody asked for. It keeps the login and follows the port.
///
/// # Errors
///
/// As [`connect`].
pub fn connect_as(
    existing: &str,
    port: u16,
    account: Option<&str>,
) -> Result<Edit, toml::de::Error> {
    existing.parse::<toml::Table>()?;

    let kept = account.is_none().then(|| self::account(existing)).flatten();
    let url = endpoint::openai_url(port, account.or(kept.as_deref()));

    let mut changes = Vec::new();
    let mut occupied = Vec::new();
    let mut out = replace_our_block(existing, &block(&url), &mut changes);

    match top_level_model_provider(&out) {
        Some(value) if value == "ironwire" => {}
        Some(value) => {
            // Their provider stays. The note is what makes leaving it safe: we
            // are about to tell them to select IronWire by hand, and once they
            // do, this line is the only record of what they had. Without it a
            // later `disconnect` would find our selection, no value to put
            // back, and delete the line -- which is the loss this whole change
            // exists to prevent, arriving one step later.
            out = note_previous_provider(&out, &value, &mut changes);
            occupied.push(Occupied {
                slot: MODEL_PROVIDER,
                current: value,
            });
        }
        None => {
            out = format!("{MODEL_PROVIDER} = \"ironwire\"\n{out}");
            changes.push(format!("{MODEL_PROVIDER} = \"ironwire\" (added)"));
        }
    }

    Ok(Edit {
        contents: out,
        changes,
        occupied,
    })
}

/// Compute the edit that undoes [`connect`].
///
/// `occupied` is always empty here, and that is a property rather than an
/// omission: an undo fills no slot, so there is no slot it can find full and
/// step around. What it does instead is remove only what IronWire put there —
/// a `model_provider` naming anyone else is left exactly as it is.
///
/// # Errors
///
/// Returns the parse error when the existing file is not valid TOML.
pub fn disconnect(existing: &str) -> Result<Edit, toml::de::Error> {
    existing.parse::<toml::Table>()?;

    // Whether the selection is ours to undo. A `model_provider` naming someone
    // else is one `connect` left alone, or one the user has chosen since;
    // either way, removing it would be this module deleting a line it never
    // wrote — and it would look to Codex exactly like a config with no provider
    // at all.
    let ours = top_level_model_provider(existing).is_some_and(|value| value == "ironwire");

    let mut changes = Vec::new();
    let mut lines: Vec<String> = Vec::new();
    let mut removing_block = false;
    let mut restore: Option<String> = None;

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_HEADER {
            removing_block = true;
            changes.push(format!("{BLOCK_HEADER}: removed"));
            continue;
        }
        if removing_block {
            // The block ends at the next table header, and only there — a blank
            // line inside a table is still inside the table.
            if trimmed.starts_with('[') {
                removing_block = false;
            } else {
                continue;
            }
        }
        // The note goes whether or not we can act on it. It is a line IronWire
        // wrote, about a state that ends here, and leaving it behind is how it
        // comes true later against the wrong provider: a user who moves Codex
        // elsewhere and disconnects would keep a note naming what they moved
        // away from, and the next connect-and-select would hand it back to
        // them as if it were still their choice.
        if let Some(previous) = trimmed.strip_prefix(PREVIOUS_MARKER) {
            if ours {
                restore = Some(previous.trim().trim_matches('"').to_string());
            }
            continue;
        }
        lines.push(line.to_string());
    }

    // Put back whatever `model_provider` said before, or drop the line we added.
    // A marker is only ever honoured alongside our own selection, so a config
    // left by an older version — which did replace the provider — still gets it
    // back.
    let provider_line = lines
        .iter()
        .position(|l| is_top_level_assignment(l, MODEL_PROVIDER));
    if ours && let Some(index) = provider_line {
        match &restore {
            Some(previous) => {
                lines[index] = format!("{MODEL_PROVIDER} = {}", quoted(previous));
                changes.push(format!("{MODEL_PROVIDER}: restored to \"{previous}\""));
            }
            None => {
                lines.remove(index);
                changes.push(format!("{MODEL_PROVIDER}: removed"));
            }
        }
    }

    Ok(Edit {
        contents: joined(&lines),
        changes,
        occupied: Vec::new(),
    })
}

/// Replace an existing IronWire block in place, or append one.
///
/// In place rather than remove-and-append so a user who moved our block
/// somewhere deliberate keeps it there.
fn replace_our_block(existing: &str, replacement: &str, changes: &mut Vec<String>) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut inside = false;
    let mut found = false;
    let mut original = Vec::new();

    for line in existing.lines() {
        let trimmed = line.trim();
        if trimmed == BLOCK_HEADER {
            inside = true;
            found = true;
            original.push(line.to_string());
            for replaced in replacement.lines() {
                lines.push(replaced.to_string());
            }
            continue;
        }
        if inside {
            if trimmed.starts_with('[') {
                inside = false;
            } else {
                original.push(line.to_string());
                continue;
            }
        }
        lines.push(line.to_string());
    }

    if found {
        let before = joined(&original);
        if before.trim() != replacement.trim() {
            changes.push(format!("{BLOCK_HEADER}: updated"));
        }
        return joined(&lines);
    }

    changes.push(format!("{BLOCK_HEADER}: added"));
    let mut out = joined(&lines);
    if !out.is_empty() && !out.ends_with("\n\n") {
        out.push('\n');
    }
    out.push_str(replacement);
    out
}

/// Record the provider we are leaving in place, above the line that holds it.
///
/// Only ever called when that provider is someone else's. The note says what
/// `disconnect` should put back if the user takes our advice and selects
/// IronWire by hand -- the manual half of the same edit `connect` used to make
/// for them, and it deserves the same undo.
///
/// Writing it is idempotent: a note we already left is rewritten only when the
/// provider under it has changed since, which is the user choosing again and
/// the newer choice being the one owed back.
fn note_previous_provider(contents: &str, current: &str, changes: &mut Vec<String>) -> String {
    let note = format!("{PREVIOUS_MARKER} {}", quoted(current));

    let mut lines: Vec<String> = Vec::new();
    let mut existing_note = None;
    for line in contents.lines() {
        let trimmed = line.trim();
        // Only the note above the top-level key is ours to rewrite; past the
        // first table header we are inside someone else's table.
        if existing_note.is_none()
            && !trimmed.starts_with('[')
            && trimmed.starts_with(PREVIOUS_MARKER)
        {
            existing_note = Some(trimmed.to_string());
            continue;
        }
        lines.push(line.to_string());
    }

    if existing_note.as_deref() == Some(note.as_str()) {
        return contents.to_string();
    }

    let Some(index) = lines
        .iter()
        .position(|l| is_top_level_assignment(l, MODEL_PROVIDER))
    else {
        // No line to sit above. Nothing sensible to record, and nothing that
        // needs recording: without a selection there is nothing to restore.
        return contents.to_string();
    };
    lines.insert(index, note);
    changes.push(format!(
        "{MODEL_PROVIDER} = \"{current}\": left alone, noted so a later disconnect can restore it"
    ));
    joined(&lines)
}

/// A string as TOML writes it, quotes and escapes included.
fn quoted(value: &str) -> String {
    toml::Value::String(value.to_string()).to_string()
}

/// Read the top-level `model_provider`, as Codex reads it.
///
/// Top level means before the first table header: `model_provider` inside
/// `[profiles.foo]` is a different setting entirely, and rewriting it would
/// change a profile the user did not ask us to touch. TOML says the same
/// thing -- a bare key belongs to whatever table it follows -- so asking the
/// parser for the root key is asking exactly the question the line scan was
/// approximating.
///
/// The parser is what answers, rather than trimming quotes off the raw
/// right-hand side, because the right-hand side is TOML and only TOML can
/// read it. `model_provider = "ironwire" # selected` is a valid line a user
/// may well have written, and taking it apart by hand yields
/// `ironwire" # selected` -- a value equal to nothing, so our own selection
/// stops looking like ours. Escapes and literal strings fail the same way.
///
/// The caller has already parsed `contents` before reaching here; failing to
/// parse returns `None`, which reads as "no selection we can act on" and
/// leaves the file alone.
fn top_level_model_provider(contents: &str) -> Option<String> {
    let table = contents.parse::<toml::Table>().ok()?;
    Some(table.get(MODEL_PROVIDER)?.as_str()?.to_string())
}

fn is_top_level_assignment(line: &str, key: &str) -> bool {
    let trimmed = line.trim();
    trimmed
        .strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

fn joined(lines: &[String]) -> String {
    if lines.is_empty() {
        return String::new();
    }
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

/// Where Codex keeps its config. `CODEX_HOME` wins, as it does for Codex.
///
/// Unlike Claude Code's, the default here does follow this process's
/// `CODEX_HOME`, and so does the default Codex login IronWire reads
/// (`ironwire_creds::codex::auth_path`): setups and hermetic tests plant it
/// precisely to move the default. A `CODEX_HOME` that is a login of its own is
/// declared with `config_dir` and wired through [`path_in`].
#[must_use]
pub fn path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME")
        && !home.is_empty()
    {
        return Some(path_in(Path::new(&home)));
    }
    Some(path_in(&dirs::home_dir()?.join(".codex")))
}

/// The config file of the Codex that runs with `CODEX_HOME=home`.
#[must_use]
pub fn path_in(home: &Path) -> PathBuf {
    home.join("config.toml")
}

/// The `base_url` of IronWire's provider block, if there is one.
#[must_use]
pub fn base_url(existing: &str) -> Option<String> {
    let table = existing.parse::<toml::Table>().ok()?;
    Some(
        table
            .get("model_providers")?
            .get("ironwire")?
            .get("base_url")?
            .as_str()?
            .to_string(),
    )
}

/// The login IronWire's provider block binds Codex to, when its `base_url` is
/// a login-bound URL of ours.
#[must_use]
pub fn account(existing: &str) -> Option<String> {
    endpoint::parse_openai(&base_url(existing)?)?.account
}

/// Whether this config currently routes Codex through IronWire.
///
/// The provider block alone is not enough: it can sit there unselected. What
/// decides where traffic goes is the top-level `model_provider`.
#[must_use]
pub fn is_wired(existing: &str) -> bool {
    top_level_model_provider(existing).is_some_and(|value| value == "ironwire")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(contents: &str) -> toml::Table {
        contents.parse().expect("still valid TOML")
    }

    /// The user editing the selection by hand, the way `wire_codex` advises.
    ///
    /// Line-wise rather than a string replace, because the note carries the
    /// same text and a blind replace would rewrite the record along with the
    /// choice -- which is not an edit any user makes.
    fn select(contents: &str, provider: &str) -> String {
        let lines: Vec<String> = contents
            .lines()
            .map(|line| {
                if is_top_level_assignment(line, MODEL_PROVIDER) {
                    format!("{MODEL_PROVIDER} = \"{provider}\"")
                } else {
                    line.to_string()
                }
            })
            .collect();
        joined(&lines)
    }

    #[test]
    fn an_empty_config_gets_both_the_pointer_and_the_block() {
        let edit = connect("", 8463).expect("edits");
        let parsed = table(&edit.contents);
        assert_eq!(parsed["model_provider"].as_str(), Some("ironwire"));
        assert_eq!(
            parsed["model_providers"]["ironwire"]["base_url"].as_str(),
            Some("http://127.0.0.1:8463/openai/v1")
        );
        assert_eq!(
            parsed["model_providers"]["ironwire"]["wire_api"].as_str(),
            Some("responses")
        );
    }

    #[test]
    fn everything_the_user_wrote_survives() {
        // The reason this module edits text instead of round-tripping through a
        // serializer. A comment lost here is a comment lost forever.
        let existing = "\
# my notes about this file
model = \"gpt-5.6\"

[tui]
theme = \"dark\"  # trailing comment
";
        let edit = connect(existing, 8463).expect("edits");
        assert!(edit.contents.contains("# my notes about this file"));
        assert!(
            edit.contents
                .contains("theme = \"dark\"  # trailing comment")
        );
        assert_eq!(table(&edit.contents)["model"].as_str(), Some("gpt-5.6"));
    }

    #[test]
    fn a_provider_the_user_already_chose_is_reported_and_left() {
        // The rule `claude_settings` follows for `ANTHROPIC_BASE_URL`: a value
        // already there is another proxy or a deliberate choice, and taking it
        // over would move someone's traffic. The caller gets told instead.
        let existing = "model_provider = \"my-own-proxy\"\n";
        let edit = connect(existing, 8463).expect("edits");

        assert_eq!(
            table(&edit.contents)["model_provider"].as_str(),
            Some("my-own-proxy"),
            "IronWire took over a provider the user had chosen"
        );
        assert_eq!(edit.occupied_slot(MODEL_PROVIDER), Some("my-own-proxy"));

        // Left alone, and written down: the caller is about to advise a manual
        // switch, and this note is what lets a later disconnect undo it.
        assert!(
            edit.contents
                .contains(&format!("{PREVIOUS_MARKER} \"my-own-proxy\"")),
            "nothing recorded the provider we are asking them to replace"
        );

        // The block is still written: unselected it routes nothing, and having
        // it there is what makes the manual fix one word.
        assert_eq!(
            table(&edit.contents)["model_providers"]["ironwire"]["base_url"].as_str(),
            Some("http://127.0.0.1:8463/openai/v1")
        );
        assert!(!is_wired(&edit.contents));
    }

    #[test]
    fn a_marker_left_by_an_older_version_still_restores_that_provider() {
        // Versions before the occupied-slot rule replaced `model_provider` and
        // remembered the old value in a comment. Those configs are on disk, and
        // disconnect owes them their provider back.
        let existing = format!(
            "{PREVIOUS_MARKER} \"openai\"\nmodel_provider = \"ironwire\"\n\n{}",
            block(&endpoint::openai_url(8463, None))
        );
        let undone = disconnect(&existing).expect("edits");
        assert_eq!(
            table(&undone.contents)["model_provider"].as_str(),
            Some("openai"),
            "disconnect must put back what the user had"
        );
        assert!(!undone.contents.contains(BLOCK_HEADER));
        assert!(!undone.contents.contains(PREVIOUS_MARKER));
    }

    #[test]
    fn disconnect_leaves_a_provider_it_never_selected() {
        // Rule three: remove only what we put there. Dropping this line would
        // look to Codex like a config with no provider at all.
        let existing = "model_provider = \"my-own-proxy\"\n";
        let connected = connect(existing, 8463).expect("edits").contents;
        let undone = disconnect(&connected).expect("edits");
        assert_eq!(
            table(&undone.contents)["model_provider"].as_str(),
            Some("my-own-proxy")
        );
        assert_eq!(table(&undone.contents), table(existing));
        assert!(
            !undone.contents.contains(PREVIOUS_MARKER),
            "the note we wrote outlived the connection it described"
        );
        assert_eq!(undone.contents.trim_end(), existing.trim_end());
    }

    #[test]
    fn a_manual_switch_to_ironwire_still_restores_the_previous_provider() {
        // The advice `wire_codex` prints, carried out. Connect leaves their
        // provider; the user replaces it by hand; disconnect owes them the old
        // value back, exactly as it would have if connect had done the edit.
        let existing = "model_provider = \"my-own-proxy\"\n";
        let connected = connect(existing, 8463).expect("edits").contents;
        let selected = select(&connected, "ironwire");
        assert!(is_wired(&selected), "the manual switch must actually route");

        let undone = disconnect(&selected).expect("edits");
        assert_eq!(
            table(&undone.contents)["model_provider"].as_str(),
            Some("my-own-proxy"),
            "the provider they had before IronWire was lost"
        );
        assert!(!undone.contents.contains(PREVIOUS_MARKER));
        assert!(!undone.contents.contains(BLOCK_HEADER));
    }

    #[test]
    fn our_own_selection_is_recognised_with_a_comment_after_it() {
        // `model_provider = "ironwire" # selected` is valid TOML and a line a
        // user may well write. Reading the value by trimming quotes off the
        // right-hand side yields `ironwire" # selected`, which matches nothing:
        // connect would report our own selection as someone else's, and
        // disconnect would take the block away and leave the pointer behind,
        // naming a provider that no longer exists.
        let existing = format!(
            "model_provider = \"ironwire\" # selected\n\n{}",
            block(&endpoint::openai_url(8463, None))
        );
        assert!(is_wired(&existing));

        let edit = connect(&existing, 8463).expect("edits");
        assert_eq!(edit.occupied_slot(MODEL_PROVIDER), None);
        assert!(!edit.contents.contains(PREVIOUS_MARKER));

        let undone = disconnect(&existing).expect("edits");
        assert!(!undone.contents.contains(BLOCK_HEADER));
        assert!(
            table(&undone.contents).get(MODEL_PROVIDER).is_none(),
            "Codex was left pointing at a provider we had just removed"
        );
    }

    #[test]
    fn a_note_is_not_left_behind_for_a_provider_the_user_has_moved_on_from() {
        // An upgraded config: an older version took the provider and noted it.
        // The user has since chosen someone else, so `ours` is false and there
        // is nothing to restore -- but leaving the note would let a later
        // connect-and-select hand back `openai` instead of their newer choice.
        let existing = format!(
            "{PREVIOUS_MARKER} \"openai\"\nmodel_provider = \"someone-else\"\n\n{}",
            block(&endpoint::openai_url(8463, None))
        );
        let undone = disconnect(&existing).expect("edits");
        assert_eq!(
            table(&undone.contents)["model_provider"].as_str(),
            Some("someone-else"),
            "disconnect moved a provider it never selected"
        );
        assert!(
            !undone.contents.contains(PREVIOUS_MARKER),
            "a stale note survived to mislead the next disconnect"
        );
        assert!(!undone.contents.contains(BLOCK_HEADER));
    }

    #[test]
    fn a_second_connect_renotes_only_when_their_choice_has_changed() {
        let first = connect("model_provider = \"my-own-proxy\"\n", 8463).expect("edits");
        let again = connect(&first.contents, 8463).expect("edits");
        assert_eq!(again.contents, first.contents);
        assert!(again.is_noop());

        // Chosen again since: the newer choice is the one owed back.
        let moved = select(&first.contents, "vllm");
        let renoted = connect(&moved, 8463).expect("edits");
        assert!(
            renoted
                .contents
                .contains(&format!("{PREVIOUS_MARKER} \"vllm\"")),
            "the note still names a provider they left"
        );
        assert_eq!(renoted.contents.matches(PREVIOUS_MARKER).count(), 1);
    }

    #[test]
    fn an_undo_reports_no_occupied_slots_because_it_fills_none() {
        let connected = connect("", 8463).expect("edits").contents;
        assert!(disconnect(&connected).expect("edits").occupied.is_empty());
        assert!(
            disconnect("model_provider = \"my-own-proxy\"\n")
                .expect("edits")
                .occupied
                .is_empty()
        );
    }

    #[test]
    fn disconnecting_a_config_we_added_to_leaves_no_trace() {
        let original = "model = \"gpt-5.6\"\n\n[tui]\ntheme = \"dark\"\n";
        let connected = connect(original, 8463).expect("edits").contents;
        let restored = disconnect(&connected).expect("edits").contents;
        assert_eq!(table(&restored), table(original));
    }

    #[test]
    fn connecting_twice_changes_nothing_the_second_time() {
        let once = connect("", 8463).expect("edits").contents;
        let twice = connect(&once, 8463).expect("edits");
        assert_eq!(twice.contents, once);
        assert!(twice.is_noop(), "reported {:?}", twice.changes);
    }

    #[test]
    fn a_port_change_rewrites_the_block_in_place_rather_than_appending_a_second_one() {
        let once = connect("", 8463).expect("edits").contents;
        let moved = connect(&once, 9000).expect("edits");
        assert_eq!(
            moved.contents.matches(BLOCK_HEADER).count(),
            1,
            "a second block would leave Codex reading a stale one"
        );
        assert!(moved.contents.contains("127.0.0.1:9000"));
        assert!(!moved.is_noop());
    }

    #[test]
    fn a_block_the_user_moved_stays_where_they_put_it() {
        let existing = format!(
            "model = \"gpt-5.6\"\n\n{}\n\n[tui]\ntheme = \"dark\"\n",
            block(&endpoint::openai_url(8463, None)).trim_end()
        );
        let edit = connect(&existing, 9000).expect("edits");
        let block_at = edit.contents.find(BLOCK_HEADER).expect("block present");
        let tui_at = edit.contents.find("[tui]").expect("tui present");
        assert!(block_at < tui_at, "our block jumped past the user's table");
    }

    #[test]
    fn a_model_provider_inside_a_profile_is_left_alone() {
        // `[profiles.work] model_provider` is a different setting; rewriting it
        // would change a profile the user did not ask us to touch.
        let existing = "[profiles.work]\nmodel_provider = \"openai\"\n";
        let edit = connect(existing, 8463).expect("edits");
        let parsed = table(&edit.contents);
        assert_eq!(
            parsed["profiles"]["work"]["model_provider"].as_str(),
            Some("openai")
        );
        assert_eq!(parsed["model_provider"].as_str(), Some("ironwire"));
    }

    #[test]
    fn a_config_we_cannot_parse_is_refused_rather_than_appended_to() {
        // Appending to a file with a syntax error makes the user's bug look
        // like ours, and they will delete our block trying to fix it.
        assert!(connect("this is not = = toml\n", 8463).is_err());
        assert!(disconnect("this is not = = toml\n").is_err());
    }

    const BOUND: &str = "http://127.0.0.1:8463/openai/account/codex-work/v1";

    #[test]
    fn wiring_a_codex_home_writes_its_bound_url() {
        let edit = connect_as("", 8463, Some("codex-work")).expect("edits");
        let parsed = table(&edit.contents);
        assert_eq!(parsed["model_provider"].as_str(), Some("ironwire"));
        assert_eq!(
            parsed["model_providers"]["ironwire"]["base_url"].as_str(),
            Some(BOUND)
        );
        assert!(is_wired(&edit.contents));
        assert_eq!(account(&edit.contents).as_deref(), Some("codex-work"));

        // The plain block in that file is ours, so binding replaces it in place.
        let plain = connect("", 8463).expect("edits").contents;
        assert_eq!(account(&plain), None);
        let bound = connect_as(&plain, 8463, Some("codex-work")).expect("edits");
        assert_eq!(bound.contents.matches(BLOCK_HEADER).count(), 1);
        assert_eq!(account(&bound.contents).as_deref(), Some("codex-work"));
        assert_eq!(bound.changes, vec![format!("{BLOCK_HEADER}: updated")]);
    }

    /// The same rule as Claude Code's settings: a connect that names no login
    /// must not move a Codex bound to one onto the default login.
    #[test]
    fn a_connect_that_names_no_login_does_not_unbind_one() {
        let bound = connect_as("", 8463, Some("codex-work"))
            .expect("edits")
            .contents;
        let again = connect(&bound, 8463).expect("edits");
        assert!(again.is_noop(), "changed: {:?}", again.changes);
        let moved = connect(&bound, 9000).expect("edits");
        assert_eq!(
            base_url(&moved.contents).as_deref(),
            Some("http://127.0.0.1:9000/openai/account/codex-work/v1")
        );
    }

    /// A provider of the user's own is left alone whichever login is asking.
    #[test]
    fn a_login_never_takes_a_provider_that_is_not_ours() {
        let edit = connect_as(
            "model_provider = \"my-own-proxy\"\n",
            8463,
            Some("codex-work"),
        )
        .expect("edits");
        assert_eq!(edit.occupied_slot(MODEL_PROVIDER), Some("my-own-proxy"));
        assert!(!is_wired(&edit.contents));
    }

    #[test]
    fn a_bound_codex_disconnects_like_any_other() {
        let original = "model = \"gpt-5.6\"\n";
        let connected = connect_as(original, 8463, Some("codex-work"))
            .expect("edits")
            .contents;
        let restored = disconnect(&connected).expect("edits").contents;
        assert_eq!(table(&restored), table(original));
    }
}
