//! The subscription logins IronWire knows about, and which client config file
//! belongs to each.
//!
//! A login is declared in `config.toml`, never found by scanning the disk, so
//! the list is the two defaults plus every `[[backends]]` entry that names a
//! `config_dir`. `connect`, `init` and `doctor` all walk this one list, which
//! is what keeps them agreeing on which settings file is whose — and that
//! decides which subscription pays for a request.

use std::path::{Path, PathBuf};

use anyhow::{Result, bail};
use ironwire_agents::{claude_settings, codex_config, endpoint};
use ironwire_core::config::{BackendImpl, Config, expand_config_dir};
use ironwire_creds::codex::CodexMode;
use ironwire_creds::{ClaudeLogin, CodexLogin, SubscriptionLogin, SubscriptionProduct};

/// One login, as config.toml declares it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Login {
    /// Its backend id, which is also what its consent is recorded under.
    pub(crate) id: String,
    pub(crate) product: SubscriptionProduct,
    /// The directory, exactly as config.toml has it. `None` for a default
    /// login in the place its client uses unprompted.
    pub(crate) config_dir: Option<String>,
    /// Whether the daemon will register it at all.
    pub(crate) enabled: bool,
}

impl Login {
    /// A product's default login, moved if `config.toml` moves it.
    pub(crate) fn default_for(product: SubscriptionProduct, config: &Config) -> Self {
        let id = product.default_backend_id();
        let entry = config.backends.iter().find(|b| b.id == id);
        Self {
            id: id.to_string(),
            product,
            config_dir: entry.and_then(|e| e.config_dir.clone()),
            enabled: entry.is_none_or(|e| e.enabled),
        }
    }

    /// Whether this is the product's default backend, `claude-sub` or
    /// `codex-sub` — moved with `config_dir` or not.
    pub(crate) fn is_default(&self) -> bool {
        self.id == self.product.default_backend_id()
    }

    /// Whether this is the default login left where its client keeps it
    /// unprompted: the one login the plain façade belongs to.
    pub(crate) fn owns_the_plain_url(&self) -> bool {
        self.is_default() && self.config_dir.is_none()
    }

    pub(crate) fn kind(&self) -> BackendImpl {
        kind_of(self.product)
    }

    /// The login, as every surface names it.
    pub(crate) fn describe(&self) -> SubscriptionLogin {
        match self.product {
            SubscriptionProduct::Claude => self.claude().describe(self.is_default()),
            SubscriptionProduct::ChatGpt => self.codex().describe(self.is_default()),
        }
    }

    pub(crate) fn claude(&self) -> ClaudeLogin {
        self.config_dir
            .clone()
            .map_or_else(ClaudeLogin::default_login, ClaudeLogin::in_dir)
    }

    pub(crate) fn codex(&self) -> CodexLogin {
        self.config_dir
            .clone()
            .map_or_else(CodexLogin::default_login, CodexLogin::in_home)
    }

    /// Where it lives, for people: `~/.claude`, `~/.codex-work`.
    pub(crate) fn location(&self) -> String {
        match self.product {
            SubscriptionProduct::Claude => self.claude().display_location(),
            SubscriptionProduct::ChatGpt => self.codex().display_location(),
        }
    }

    /// What to run to log it in: `CLAUDE_CONFIG_DIR=~/.claude-work claude`.
    pub(crate) fn login_command(&self) -> String {
        match self.product {
            SubscriptionProduct::Claude => self.claude().login_command(),
            SubscriptionProduct::ChatGpt => self.codex().login_command(),
        }
    }

    /// Whether this login holds a credential its backend can use. The
    /// error, when it does not, names the command that fixes it.
    pub(crate) fn check(&self) -> Result<(), String> {
        match self.product {
            SubscriptionProduct::Claude => {
                self.claude().read().map(drop).map_err(|e| e.to_string())
            }
            SubscriptionProduct::ChatGpt => match self.codex().read() {
                Ok(creds) if creds.mode == CodexMode::ChatGpt => Ok(()),
                Ok(creds) => Err(format!(
                    "{} holds an OpenAI API key rather than a ChatGPT login",
                    creds.source
                )),
                Err(e) => Err(e.to_string()),
            },
        }
    }

    /// The segment the daemon reads the login from. `None` only for a default
    /// login in its usual place.
    ///
    /// A default moved with `config_dir` is named like any other login: the
    /// plain URL is also what the client still using the default's usual
    /// directory is pointed at, and the daemon serves that URL from no
    /// subscription once the default has moved.
    pub(crate) fn account(&self) -> Option<&str> {
        (!self.owns_the_plain_url()).then_some(self.id.as_str())
    }

    /// The URL its client is pointed at.
    pub(crate) fn url(&self, port: u16) -> String {
        match self.product {
            SubscriptionProduct::Claude => endpoint::anthropic_url(port, self.account()),
            SubscriptionProduct::ChatGpt => endpoint::openai_url(port, self.account()),
        }
    }

    /// The config file of the client that uses this login: `settings.json` in
    /// the Claude config dir, `config.toml` in the Codex home.
    pub(crate) fn client_config(&self) -> Option<PathBuf> {
        match (self.product, &self.config_dir) {
            (SubscriptionProduct::Claude, Some(dir)) => {
                Some(claude_settings::path_in(&expand(dir)?))
            }
            (SubscriptionProduct::Claude, None) => {
                Some(claude_settings::path_in(&claude_settings::default_dir()?))
            }
            (SubscriptionProduct::ChatGpt, Some(dir)) => Some(codex_config::path_in(&expand(dir)?)),
            // The default Codex home follows `CODEX_HOME`, as the default
            // Codex login does — see `codex_config::path`.
            (SubscriptionProduct::ChatGpt, None) => codex_config::path(),
        }
    }

    /// For a default login moved with `config_dir`, the config file of the
    /// client it moved away from — the Claude Code or Codex still started
    /// without a directory of its own, which no subscription serves now.
    /// `None` for anything else, and for a move into that very directory.
    pub(crate) fn left_behind(&self) -> Option<PathBuf> {
        if !self.is_default() || self.config_dir.is_none() {
            return None;
        }
        let unmoved = Self {
            config_dir: None,
            ..self.clone()
        }
        .client_config()?;
        (self.client_config().as_ref() != Some(&unmoved)).then_some(unmoved)
    }

    /// The client, named so two of its logins read apart: `Claude Code`,
    /// `Claude Code (~/.claude-work)`.
    pub(crate) fn client_label(&self) -> String {
        let client = self.product.client();
        match self.describe().location {
            None => client.to_string(),
            Some(location) => format!("{client} ({location})"),
        }
    }
}

/// Every login config.toml knows about: the two defaults, then each
/// additional login in the order it is declared — which is the order the
/// daemon registers them in. Disabled ones included; callers decide.
pub(crate) fn configured(config: &Config) -> Vec<Login> {
    let mut logins = vec![
        Login::default_for(SubscriptionProduct::Claude, config),
        Login::default_for(SubscriptionProduct::ChatGpt, config),
    ];
    for entry in &config.backends {
        let Some(product) = BackendImpl::parse(&entry.kind).and_then(product_of) else {
            continue;
        };
        if entry.id == product.default_backend_id() {
            continue;
        }
        logins.push(Login {
            id: entry.id.clone(),
            product,
            config_dir: entry.config_dir.clone(),
            enabled: entry.enabled,
        });
    }
    logins
}

/// The configured login whose directory is `dir`, if any.
///
/// Matched as paths, so `~/.claude-work` and its absolute spelling are one
/// login. A default login only matches once config.toml has moved it.
pub(crate) fn by_dir<'a>(
    logins: &'a [Login],
    product: SubscriptionProduct,
    dir: &str,
) -> Option<&'a Login> {
    let wanted = expand(dir)?;
    logins.iter().find(|login| {
        login.product == product
            && login
                .config_dir
                .as_deref()
                .and_then(expand)
                .is_some_and(|theirs| {
                    theirs == wanted
                        && (product != SubscriptionProduct::Claude
                            || !KEYCHAIN_PER_DIRECTORY
                            || login.claude().keychain_services()
                                == ClaudeLogin::in_dir(dir).keychain_services())
                })
    })
}

/// A shared settings directory whose macOS Keychain identity differs.
/// Registering an alias cannot help: both clients would read the same URL.
pub(crate) fn claude_directory_alias<'a>(logins: &'a [Login], dir: &str) -> Option<&'a Login> {
    if !KEYCHAIN_PER_DIRECTORY {
        return None;
    }
    let wanted = expand(dir)?;
    logins.iter().find(|login| {
        login.product == SubscriptionProduct::Claude
            && login
                .config_dir
                .as_deref()
                .and_then(expand)
                .is_some_and(|theirs| theirs == wanted)
            && login.claude().keychain_services() != ClaudeLogin::in_dir(dir).keychain_services()
    })
}

/// Whether `dir` is where a product's client keeps its default login:
/// `~/.claude`, or `~/.codex` whatever `CODEX_HOME` says — the directories
/// `Config::validate` refuses an additional login in.
pub(crate) fn is_default_dir(product: SubscriptionProduct, dir: &str) -> bool {
    let usual = match product {
        SubscriptionProduct::Claude => claude_settings::default_dir(),
        SubscriptionProduct::ChatGpt => dirs::home_dir().map(|home| home.join(".codex")),
    };
    usual.is_some_and(|usual| expand(dir).is_some_and(|dir| dir == usual))
}

/// The `CLAUDE_CONFIG_DIR` this shell exports, when the Claude Code it starts
/// uses a login config.toml does not declare.
pub(crate) fn undeclared_claude_dir(declared: &[Login]) -> Option<String> {
    let dir = std::env::var("CLAUDE_CONFIG_DIR")
        .ok()
        .filter(|dir| !dir.is_empty())?;
    is_undeclared_claude_dir(declared, &dir, KEYCHAIN_PER_DIRECTORY).then_some(dir)
}

/// Whether Claude Code keeps a login per exact `CLAUDE_CONFIG_DIR` string
/// even for `~/.claude`: true where it uses the Keychain, whose item is named
/// after that string, and not where the login is the directory's own
/// `.credentials.json`.
const KEYCHAIN_PER_DIRECTORY: bool = cfg!(target_os = "macos");

/// [`undeclared_claude_dir`], with the platform's answer passed in so both
/// can be tested anywhere.
///
/// `~/.claude` exported is the default login's own directory. Without a
/// Keychain that is the default login itself — one credentials file — as long
/// as config.toml has not moved the default away. With one it is a login of
/// its own that happens to read the default's `settings.json`.
fn is_undeclared_claude_dir(declared: &[Login], dir: &str, keychain: bool) -> bool {
    if by_dir(declared, SubscriptionProduct::Claude, dir).is_some() {
        return false;
    }
    let the_default_itself = !keychain
        && is_default_dir(SubscriptionProduct::Claude, dir)
        && declared
            .iter()
            .any(|l| l.product == SubscriptionProduct::Claude && l.owns_the_plain_url());
    !the_default_itself
}

/// Whether a Claude login in `dir` reads the `settings.json` that `claude`
/// started without `CLAUDE_CONFIG_DIR` reads, while keeping a credential of
/// its own: `~/.claude`, on macOS. Whatever URL that file holds then serves
/// both, and IronWire cannot tell their requests apart.
pub(crate) fn shares_plain_claudes_settings(dir: &str) -> bool {
    KEYCHAIN_PER_DIRECTORY && is_default_dir(SubscriptionProduct::Claude, dir)
}

/// What `ironwire connect claude` would do with a `CLAUDE_CONFIG_DIR` no
/// login declares, for the notes that send the user there: `~/.claude` itself
/// moves the default login, never adds one beside it.
pub(crate) fn connect_would(dir: &str) -> &'static str {
    if is_default_dir(SubscriptionProduct::Claude, dir) {
        "moves your default login (`claude-sub`) to it"
    } else {
        "adds it"
    }
}

pub(crate) fn kind_of(product: SubscriptionProduct) -> BackendImpl {
    match product {
        SubscriptionProduct::Claude => BackendImpl::ClaudeSubscription,
        SubscriptionProduct::ChatGpt => BackendImpl::CodexSubscription,
    }
}

fn product_of(kind: BackendImpl) -> Option<SubscriptionProduct> {
    match kind {
        BackendImpl::ClaudeSubscription => Some(SubscriptionProduct::Claude),
        BackendImpl::CodexSubscription => Some(SubscriptionProduct::ChatGpt),
        _ => None,
    }
}

/// A login directory as it is stored: what the user typed, with their home
/// written as `~/` when it is under it.
///
/// A shell has usually expanded `~` before we see it. Writing it back as `~/`
/// keeps config.toml portable across a renamed home, and expands to exactly
/// the string it was — which is what Claude Code hashes to name its Keychain
/// item, so nothing about the login changes. A trailing slash is kept for the
/// same reason.
///
/// Never `~//`. A home spelled with a trailing slash turns `~/.claude-work`
/// into `/Users/alice//.claude-work`, and that exact string is the login; as
/// `~//.claude-work` it reads like a typo, and tidying it by hand to `~/` names
/// a different Keychain item. So it is kept absolute, as typed.
///
/// # Errors
///
/// For a relative directory. The client hashed the relative string it was
/// given, and the daemon, whose working directory is not this shell's, has no
/// way to reproduce it.
pub(crate) fn stored_form(dir: &str, variable: &str) -> Result<String> {
    stored_form_under(dir, variable, dirs::home_dir().as_deref())
}

/// [`stored_form`] against a given home, so a home spelled with a trailing
/// slash can be tested without changing this process's `HOME`.
fn stored_form_under(dir: &str, variable: &str, home: Option<&Path>) -> Result<String> {
    if dir.starts_with("~//") {
        return Ok(home.map_or_else(
            || dir.to_string(),
            |home| expand_config_dir(dir, home).display().to_string(),
        ));
    }
    if dir.starts_with("~/") {
        return Ok(dir.to_string());
    }
    if !Path::new(dir).is_absolute() {
        bail!(
            "`{dir}` is a relative path. Use the full directory you export as {variable} \
             for that login, for example `--config-dir ~/{}`",
            dir.trim_start_matches("./")
        );
    }
    Ok(abbreviate(dir, home))
}

/// A path for people: under the home directory, written from `~`.
pub(crate) fn shown(path: &Path) -> String {
    abbreviate(&path.display().to_string(), dirs::home_dir().as_deref())
}

/// `/Users/alice/.claude-work` → `~/.claude-work`; anything else unchanged.
///
/// The credential side's rule, so the `~/` form always expands back to the
/// exact string — see [`ironwire_creds::login::abbreviate_home`] for why that
/// matters. The home itself stays absolute: `config_dir = "~"` is not a form
/// config.toml accepts.
fn abbreviate(dir: &str, home: Option<&Path>) -> String {
    let short = ironwire_creds::login::abbreviate_home(dir, home);
    if short
        .strip_prefix("~/")
        .is_some_and(|rest| !rest.is_empty())
    {
        short
    } else {
        dir.to_string()
    }
}

fn expand(dir: &str) -> Option<PathBuf> {
    Some(expand_config_dir(dir, &dirs::home_dir()?))
}

/// The id a login gets when nobody chose one: its directory's name, without
/// the leading dot, prefixed with the product unless it already starts with
/// it. `~/.claude-work` → `claude-work`, `~/.codex2` → `codex2`,
/// `~/work` → `claude-work`.
pub(crate) fn derive_id(product: SubscriptionProduct, dir: &str) -> String {
    let prefix = match product {
        SubscriptionProduct::Claude => "claude",
        SubscriptionProduct::ChatGpt => "codex",
    };
    let name = dir
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .trim_start_matches('.');
    let name: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = name.trim_matches('-');
    if name.is_empty() {
        format!("{prefix}-login")
    } else if name.starts_with(prefix) {
        name.to_string()
    } else {
        format!("{prefix}-{name}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_login_is_named_after_its_directory() {
        use SubscriptionProduct::{ChatGpt, Claude};
        assert_eq!(derive_id(Claude, "~/.claude-work"), "claude-work");
        assert_eq!(
            derive_id(Claude, "/Users/alice/.claude-work/"),
            "claude-work"
        );
        assert_eq!(derive_id(ChatGpt, "~/.codex2"), "codex2");
        assert_eq!(derive_id(Claude, "~/work"), "claude-work");
        assert_eq!(derive_id(ChatGpt, "~/work"), "codex-work");
        // Only what an id may hold: it ends up in ledger rows and headers.
        assert_eq!(derive_id(Claude, "/Volumes/My Work/"), "claude-My-Work");
        assert_eq!(derive_id(Claude, "/"), "claude-login");
    }

    #[test]
    fn a_relative_directory_is_refused_with_the_form_that_would_work() {
        let error = stored_form(".claude-work", "CLAUDE_CONFIG_DIR")
            .expect_err("relative")
            .to_string();
        assert!(error.contains("--config-dir ~/.claude-work"), "{error}");
        assert_eq!(
            stored_form("~/.claude-work/", "CLAUDE_CONFIG_DIR").expect("kept"),
            "~/.claude-work/"
        );
        assert_eq!(
            stored_form("/srv/claude", "CLAUDE_CONFIG_DIR").expect("kept"),
            "/srv/claude"
        );
    }

    fn config(text: &str) -> Config {
        toml::from_str(text).expect("parses")
    }

    #[test]
    fn every_declared_login_is_listed_after_the_defaults_in_declaration_order() {
        let config = config(
            "[[backends]]\nid = \"codex-work\"\nkind = \"codex-subscription\"\n\
             config_dir = \"~/.codex-work\"\n\
             [[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-work\"\nenabled = false\n\
             [[backends]]\nid = \"anthropic-key\"\nkind = \"anthropic-api\"\n",
        );
        let logins = configured(&config);
        let ids: Vec<&str> = logins.iter().map(|l| l.id.as_str()).collect();
        assert_eq!(
            ids,
            ["claude-sub", "codex-sub", "codex-work", "claude-work"]
        );
        assert!(!logins[3].enabled);
        assert_eq!(logins[2].account(), Some("codex-work"));
        assert_eq!(logins[0].account(), None);
        assert_eq!(
            logins[3].url(8463),
            "http://127.0.0.1:8463/anthropic/account/claude-work"
        );
        assert_eq!(
            logins[2].url(8463),
            "http://127.0.0.1:8463/openai/account/codex-work/v1"
        );
        assert_eq!(logins[3].client_label(), "Claude Code (~/.claude-work)");
    }

    /// The plain façade belongs to the default login in its usual place, and
    /// the daemon serves it from no subscription once that login moves. So a
    /// moved default is wired with the URL naming it, like any other login —
    /// and the client it moved away from is the one left on the plain URL.
    #[test]
    fn a_moved_default_login_is_wired_with_the_url_that_names_it() {
        let moved = config(
            "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-personal\"\n\
             [[backends]]\nid = \"codex-sub\"\nkind = \"codex-subscription\"\n\
             config_dir = \"~/.codex-personal\"\n",
        );
        let logins = configured(&moved);
        assert_eq!(logins[0].config_dir.as_deref(), Some("~/.claude-personal"));
        assert_eq!(
            logins[0].url(8463),
            "http://127.0.0.1:8463/anthropic/account/claude-sub"
        );
        assert_eq!(
            logins[1].url(8463),
            "http://127.0.0.1:8463/openai/account/codex-sub/v1"
        );
        assert_eq!(
            by_dir(&logins, SubscriptionProduct::Claude, "~/.claude-personal/")
                .map(|l| l.id.as_str()),
            if KEYCHAIN_PER_DIRECTORY {
                None
            } else {
                Some("claude-sub")
            }
        );

        let home = dirs::home_dir().expect("a home directory");
        assert_eq!(
            logins[0].left_behind(),
            Some(home.join(".claude").join("settings.json"))
        );

        // Left in place, the default keeps the plain URL and leaves nothing
        // behind; an additional login never has anything to leave.
        let unmoved = configured(&Config::default());
        assert_eq!(unmoved[0].url(8463), "http://127.0.0.1:8463/anthropic");
        assert_eq!(unmoved[0].left_behind(), None);
        let work = config(
            "[[backends]]\nid = \"claude-work\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-work\"\n",
        );
        assert_eq!(configured(&work)[2].left_behind(), None);
    }

    /// Moved into its own usual directory, the default leaves no other
    /// client behind: the file it was in is the file it is in.
    #[test]
    fn a_default_moved_into_its_usual_directory_leaves_nothing_behind() {
        let logins = configured(&config(
            "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude/\"\n",
        ));
        assert_eq!(logins[0].left_behind(), None);
        assert!(is_default_dir(SubscriptionProduct::Claude, "~/.claude/"));
        assert!(is_default_dir(SubscriptionProduct::ChatGpt, "~/.codex"));
        assert!(!is_default_dir(SubscriptionProduct::Claude, "~/.codex"));
        assert!(!is_default_dir(
            SubscriptionProduct::Claude,
            "~/.claude-work"
        ));
    }

    /// `CLAUDE_CONFIG_DIR=~/.claude` is the default login itself where the
    /// login is that directory's credentials file. Where Claude Code uses the
    /// Keychain it is an item of its own, so a login nothing declares — and a
    /// default moved elsewhere leaves `~/.claude` undeclared everywhere.
    #[test]
    fn an_exported_default_directory_is_a_login_of_its_own_only_with_a_keychain() {
        let home = dirs::home_dir().expect("a home directory");
        let usual = home.join(".claude").display().to_string();
        let unmoved = configured(&Config::default());
        assert!(!is_undeclared_claude_dir(&unmoved, &usual, false));
        assert!(!is_undeclared_claude_dir(&unmoved, "~/.claude/", false));
        assert!(is_undeclared_claude_dir(&unmoved, &usual, true));

        let moved = configured(&config(
            "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-main\"\n",
        ));
        assert!(is_undeclared_claude_dir(&moved, &usual, false));
        assert!(!is_undeclared_claude_dir(&moved, "~/.claude-main", true));

        for keychain in [false, true] {
            assert!(is_undeclared_claude_dir(
                &unmoved,
                "~/.claude-work",
                keychain
            ));
        }
    }

    /// `~//` round-trips now, but reads as a typo, and the tidied `~/` is a
    /// different Keychain item: a doubled slash after the home stays absolute.
    #[test]
    fn a_login_directory_is_never_stored_as_tilde_double_slash() {
        let home = dirs::home_dir().expect("a home directory");
        let home = home.to_str().expect("UTF-8 home").trim_end_matches('/');
        let doubled = format!("{home}//.claude-work");
        assert_eq!(
            stored_form(&doubled, "CLAUDE_CONFIG_DIR").expect("absolute"),
            doubled
        );
        assert_eq!(
            stored_form("~//.claude-work", "CLAUDE_CONFIG_DIR").expect("absolute"),
            doubled
        );
        assert_eq!(
            stored_form(&format!("{home}/.claude-work"), "CLAUDE_CONFIG_DIR").expect("kept"),
            "~/.claude-work"
        );
    }

    /// Under `HOME=/h/` the daemon expands `~/.claude-work` to
    /// `/h//.claude-work` — a different Keychain item from the
    /// `/h/.claude-work` the user named. Whatever is stored has to expand back
    /// to exactly what was typed.
    #[test]
    fn a_stored_login_directory_expands_back_to_what_was_typed_under_a_slashed_home() {
        let slashed = Path::new("/h/");
        let stored = |dir: &str| {
            stored_form_under(dir, "CLAUDE_CONFIG_DIR", Some(slashed)).expect("absolute")
        };

        assert_eq!(stored("/h/.claude-work"), "/h/.claude-work");
        assert_eq!(stored("/h//.claude-work"), "~/.claude-work");
        assert_eq!(stored("/h/"), "/h/");

        for (home, typed) in [
            (slashed, "/h/.claude-work"),
            (slashed, "/h//.claude-work"),
            (slashed, "/h/work/"),
            (Path::new("/h"), "/h/.claude-work"),
            (Path::new("/h"), "/h//.claude-work"),
            (Path::new("/h"), "/h/"),
        ] {
            let kept = stored_form_under(typed, "CLAUDE_CONFIG_DIR", Some(home)).expect("absolute");
            assert_eq!(
                expand_config_dir(&kept, home),
                Path::new(typed),
                "{typed} under {home:?} was stored as {kept}"
            );
            assert!(
                kept.starts_with("~/") || Path::new(&kept).is_absolute(),
                "{kept} is not a form config.toml accepts"
            );
        }
    }
}
