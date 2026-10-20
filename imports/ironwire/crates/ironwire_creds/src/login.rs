//! Which login a subscription backend reads, described once.
//!
//! A user may run Claude Code under several `CLAUDE_CONFIG_DIR`s and Codex
//! under several `CODEX_HOME`s, and each of those logins is its own backend.
//! The control API, the CLI and the consent prompt all have to name one of them
//! to a person — and three hand-written ways of doing that would be three
//! different names for the same login.

use std::borrow::Cow;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::claude::ANTHROPIC_HOST;
use crate::codex::CHATGPT_HOST;

/// A subscription product IronWire can read a login for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubscriptionProduct {
    /// A Claude plan, logged in through Claude Code.
    Claude,
    /// A ChatGPT plan, logged in through Codex.
    ChatGpt,
}

impl SubscriptionProduct {
    /// What the user calls the plan, e.g. `Claude`.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude",
            Self::ChatGpt => "ChatGPT",
        }
    }

    /// The official client that owns the login, e.g. `Claude Code`.
    #[must_use]
    pub fn client(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::ChatGpt => "Codex",
        }
    }

    /// The only host this product's token is ever sent to.
    #[must_use]
    pub fn host(self) -> &'static str {
        match self {
            Self::Claude => ANTHROPIC_HOST,
            Self::ChatGpt => CHATGPT_HOST,
        }
    }

    /// Who runs the service.
    #[must_use]
    pub fn vendor(self) -> &'static str {
        match self {
            Self::Claude => "Anthropic",
            Self::ChatGpt => "OpenAI",
        }
    }

    /// The backend id of this product's default login.
    ///
    /// Fixed forever: consent, quota, pins, trusted backends and ledger history
    /// all key on it.
    #[must_use]
    pub fn default_backend_id(self) -> &'static str {
        match self {
            Self::Claude => "claude-sub",
            Self::ChatGpt => "codex-sub",
        }
    }

    /// The product whose default login has this backend id.
    #[must_use]
    pub fn for_default_backend_id(backend_id: &str) -> Option<Self> {
        [Self::Claude, Self::ChatGpt]
            .into_iter()
            .find(|product| product.default_backend_id() == backend_id)
    }

    /// The `ironwire connect` target for this product's client.
    fn connect_target(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::ChatGpt => "codex",
        }
    }
}

/// One login, as every surface that names it should name it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionLogin {
    /// Which product the login is for.
    pub product: SubscriptionProduct,
    /// The login's directory, home-abbreviated (`~/.claude-work`). `None` for a
    /// default login that lives where the client puts it unprompted.
    pub location: Option<String>,
    /// The same directory exactly as the backend reads it: `~` expanded, no
    /// trailing slash trimmed — the string Claude Code hashes into its
    /// Keychain item name. `None` exactly when [`Self::location`] is.
    ///
    /// Kept beside `location` rather than derived from it, because
    /// abbreviating is not reversible: `/Users/alice/.claude-work` and
    /// `~/.claude-work` print the same, and only one of them is what the
    /// client saw.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_dir: Option<String>,
    /// Actual credential directory, including the default Codex home selected
    /// by the daemon's environment. Prompt grants bind it without changing
    /// legacy ledger matching for defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_dir: Option<String>,
    /// Whether this is the product's default login, the one behind
    /// [`SubscriptionProduct::default_backend_id`].
    pub default: bool,
}

impl SubscriptionLogin {
    /// The default login of `product`, in its usual place.
    #[must_use]
    pub fn default_for(product: SubscriptionProduct) -> Self {
        match product {
            SubscriptionProduct::Claude => crate::ClaudeLogin::default_login().describe(true),
            SubscriptionProduct::ChatGpt => crate::CodexLogin::default_login().describe(true),
        }
    }

    /// Which login a consent to this one answered, for
    /// [`crate::ConsentLedger::grant_for`] and
    /// [`crate::ConsentLedger::is_granted_for`].
    ///
    /// The directory, not the backend id: a relocated `claude-sub` is a
    /// different account behind the same id, and a yes given to the login in
    /// `~/.claude` is not a yes to whichever login that id reads next.
    #[must_use]
    pub fn consent_location(&self) -> Option<&str> {
        self.config_dir.as_deref()
    }

    /// What to run to connect this login, e.g.
    /// `ironwire connect claude --config-dir ~/.claude-work`.
    #[must_use]
    pub fn connect_command(&self) -> String {
        let target = self.product.connect_target();
        match &self.location {
            None => format!("ironwire connect {target}"),
            Some(location) => {
                format!(
                    "ironwire connect {target} --config-dir {}",
                    shell_word(location)
                )
            }
        }
    }

    /// A name that tells two logins of one product apart, e.g.
    /// `Claude subscription (~/.claude-work)`.
    #[must_use]
    pub fn label(&self) -> String {
        let name = self.product.name();
        match &self.location {
            None => format!("{name} subscription"),
            Some(location) => format!("{name} subscription ({location})"),
        }
    }
}

/// Expand a leading `~` the way a shell does before the client ever sees the
/// value.
///
/// Textual, not `Path::join`: Claude Code hashes the exact string the shell
/// handed it, and the shell's rule is "replace `~` with `$HOME`", nothing more.
pub(crate) fn expand_home(raw: &str, home: Option<&Path>) -> String {
    let Some(home) = home.and_then(Path::to_str) else {
        return raw.to_string();
    };
    match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with('/') => format!("{home}{rest}"),
        _ => raw.to_string(),
    }
}

/// Show a path under the home directory as `~/…`, so a command we print reads
/// the way the user would type it.
///
/// Only when expanding the `~` again, as a shell does, gives the exact string
/// back. The commands we print set `CLAUDE_CONFIG_DIR`, and Claude Code names
/// the Keychain item after the string the shell hands it: under `HOME=/h/`,
/// `~/.claude-work` becomes `/h//.claude-work`, a different item from
/// `/h/.claude-work`. A path no `~/…` reproduces is printed as it is. So is one
/// that would need `~//`, which reproduces it but reads as a typo someone will
/// fix.
///
/// Public because the CLI writes login directories into `config.toml` in this
/// form, and the daemon expands them back with the same rule.
pub fn abbreviate_home(path: &str, home: Option<&Path>) -> String {
    let Some(home) = home.and_then(Path::to_str).filter(|h| !h.is_empty()) else {
        return path.to_string();
    };
    match path.strip_prefix(home) {
        Some("") => "~".to_string(),
        Some(rest) if rest.starts_with('/') && !rest.starts_with("//") => format!("~{rest}"),
        _ => path.to_string(),
    }
}

/// A path as one shell word, keeping a leading `~/` outside the quotes so the
/// shell still expands it.
///
/// These strings end up in commands we tell the user to run. A config dir with
/// a space in it, printed bare, would be a command that does something else.
pub(crate) fn shell_word(path: &str) -> Cow<'_, str> {
    let plain = |s: &str| {
        s.chars().all(|c| {
            c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | '+' | '@' | ',' | ':')
        })
    };
    if let Some(rest) = path.strip_prefix("~/") {
        if plain(rest) {
            return Cow::Borrowed(path);
        }
        return Cow::Owned(format!("~/{}", single_quoted(rest)));
    }
    if path == "~" || plain(path) {
        return Cow::Borrowed(path);
    }
    Cow::Owned(single_quoted(path))
}

fn single_quoted(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alice() -> Option<&'static Path> {
        Some(Path::new("/Users/alice"))
    }

    #[test]
    fn a_logins_connect_command_names_its_config_dir() {
        let work = SubscriptionLogin {
            product: SubscriptionProduct::Claude,
            location: Some("~/.claude-work".into()),
            config_dir: Some("/Users/alice/.claude-work".into()),
            source_dir: None,
            default: false,
        };
        assert_eq!(
            work.connect_command(),
            "ironwire connect claude --config-dir ~/.claude-work"
        );

        let codex = SubscriptionLogin {
            product: SubscriptionProduct::ChatGpt,
            location: Some("~/.codex-work".into()),
            config_dir: Some("/Users/alice/.codex-work".into()),
            source_dir: None,
            default: false,
        };
        assert_eq!(
            codex.connect_command(),
            "ironwire connect codex --config-dir ~/.codex-work"
        );
    }

    #[test]
    fn a_default_logins_connect_command_is_the_one_users_already_know() {
        let claude = SubscriptionLogin::default_for(SubscriptionProduct::Claude);
        assert_eq!(claude.connect_command(), "ironwire connect claude");
        let codex = SubscriptionLogin::default_for(SubscriptionProduct::ChatGpt);
        assert_eq!(codex.connect_command(), "ironwire connect codex");
    }

    #[test]
    fn a_label_tells_two_logins_of_one_product_apart() {
        let personal = SubscriptionLogin::default_for(SubscriptionProduct::Claude);
        let work = SubscriptionLogin {
            location: Some("~/.claude-work".into()),
            config_dir: Some("/Users/alice/.claude-work".into()),
            source_dir: None,
            default: false,
            ..personal.clone()
        };
        assert_eq!(personal.label(), "Claude subscription");
        assert_eq!(work.label(), "Claude subscription (~/.claude-work)");

        let codex = SubscriptionLogin {
            product: SubscriptionProduct::ChatGpt,
            location: Some("~/.codex-work".into()),
            config_dir: Some("/Users/alice/.codex-work".into()),
            source_dir: None,
            default: false,
        };
        assert_eq!(codex.label(), "ChatGPT subscription (~/.codex-work)");
    }

    #[test]
    fn only_the_two_default_ids_name_a_default_login() {
        assert_eq!(
            SubscriptionProduct::for_default_backend_id("claude-sub"),
            Some(SubscriptionProduct::Claude)
        );
        assert_eq!(
            SubscriptionProduct::for_default_backend_id("codex-sub"),
            Some(SubscriptionProduct::ChatGpt)
        );
        assert_eq!(
            SubscriptionProduct::for_default_backend_id("claude-work"),
            None
        );
    }

    #[test]
    fn a_tilde_expands_exactly_as_the_shell_would_hand_it_to_the_client() {
        assert_eq!(
            expand_home("~/.claude-work", alice()),
            "/Users/alice/.claude-work"
        );
        // The trailing slash survives: Claude Code hashes it.
        assert_eq!(
            expand_home("~/.claude-work/", alice()),
            "/Users/alice/.claude-work/"
        );
        assert_eq!(expand_home("~", alice()), "/Users/alice");
        // `~bob` is another user's home, which we have no business resolving.
        assert_eq!(expand_home("~bob/.claude", alice()), "~bob/.claude");
        assert_eq!(expand_home("/srv/claude", alice()), "/srv/claude");
        assert_eq!(expand_home("~/.claude-work", None), "~/.claude-work");
    }

    #[test]
    fn only_paths_inside_the_home_are_abbreviated() {
        assert_eq!(
            abbreviate_home("/Users/alice/.claude-work", alice()),
            "~/.claude-work"
        );
        assert_eq!(abbreviate_home("/Users/alice", alice()), "~");
        // A sibling whose name merely starts with the home's is not inside it.
        assert_eq!(
            abbreviate_home("/Users/alicex/.claude", alice()),
            "/Users/alicex/.claude"
        );
        assert_eq!(abbreviate_home("/srv/claude", alice()), "/srv/claude");
        assert_eq!(
            abbreviate_home("/Users/alice/.claude", None),
            "/Users/alice/.claude"
        );
    }

    /// Every abbreviation has to expand back to the string it came from,
    /// because that string is what the Keychain item is named after.
    #[test]
    fn an_abbreviated_dir_expands_back_to_the_exact_string_under_any_home() {
        let slashed = Some(Path::new("/h/"));
        assert_eq!(
            abbreviate_home("/h//.claude-work", slashed),
            "~/.claude-work"
        );
        assert_eq!(abbreviate_home("/h/", slashed), "~");
        // `~/.claude-work` would come back as `/h//.claude-work`.
        assert_eq!(
            abbreviate_home("/h/.claude-work", slashed),
            "/h/.claude-work"
        );

        let plain = Some(Path::new("/h"));
        assert_eq!(abbreviate_home("/h/.claude-work", plain), "~/.claude-work");
        // Exact as `~//.claude-work`, but never printed that way.
        assert_eq!(
            abbreviate_home("/h//.claude-work", plain),
            "/h//.claude-work"
        );

        for home in [slashed, plain] {
            for path in [
                "/h//.claude-work",
                "/h/.claude-work",
                "/h/",
                "/h",
                "/h/a/b/",
                "/hx/.claude",
            ] {
                let shown = abbreviate_home(path, home);
                assert!(!shown.starts_with("~//"), "{path} under {home:?}: {shown}");
                assert_eq!(
                    expand_home(&shown, home),
                    path,
                    "{path} under {home:?} was shown as {shown}"
                );
            }
        }
    }

    #[test]
    fn a_dir_the_shell_would_split_is_quoted_without_losing_the_tilde() {
        assert_eq!(shell_word("~/.claude-work"), "~/.claude-work");
        assert_eq!(shell_word("~/my claude"), "~/'my claude'");
        assert_eq!(shell_word("/srv/it's here"), r"'/srv/it'\''s here'");
        assert_eq!(shell_word("~/zoë"), "~/zoë");
    }
}
