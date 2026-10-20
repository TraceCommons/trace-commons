//! `ironwire connect` — wire a client up, and gate subscription backends
//! behind recorded consent.
//!
//! Two rules, both from `docs/TRUST.md`:
//!
//! - Every command prints the exact file it is about to modify *before*
//!   modifying it, and `--dry-run` shows the change without making it.
//! - A subscription backend stays off until the user answers a specific
//!   question in plain language, and the answer is recorded against the version
//!   of the question they were asked.

use std::io::Write;
use std::path::Path;

use anyhow::{Context, Result, anyhow, bail};
use chrono::Utc;
use ironwire_core::DEFAULT_PORT;
use ironwire_core::config::{BackendImpl, Config};
use ironwire_creds::SubscriptionProduct;
use ironwire_creds::codex::CodexMode;
use ironwire_creds::consent::{ConsentLedger, ConsentPrompt};

use ironwire_agents::tools::on_path;
use ironwire_agents::{claude_settings, codex_config, endpoint};

use super::logins::{self, Login};
use super::paths;

/// Which login of a product a `connect` or `disconnect` is about, as asked.
#[derive(Debug)]
pub(crate) struct LoginArgs {
    /// `--config-dir`: the directory exported as `CLAUDE_CONFIG_DIR` or
    /// `CODEX_HOME` for that login.
    pub(crate) config_dir: Option<String>,
    /// `--id`: the backend id it has, or should have, in config.toml.
    pub(crate) id: Option<String>,
}

impl LoginArgs {
    fn names_one(&self) -> bool {
        self.config_dir.is_some() || self.id.is_some()
    }
}

/// Run `ironwire connect <target>`.
pub(crate) fn run(
    target: &str,
    subscription: bool,
    dry_run: bool,
    port: Option<u16>,
    login: LoginArgs,
) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    if let Some(product) = product_for(target) {
        return connect_login(product, &login, subscription, dry_run, port);
    }
    if login.names_one() {
        bail!(
            "`--config-dir` and `--id` choose a Claude or Codex login, and `{target}` has \
             none\n\ntry: {}",
            Verb::Connect.command(SubscriptionProduct::Claude, " --config-dir ~/.claude-work")
        );
    }
    match target {
        "near" => connect_near(port),
        "anthropic-api" => connect_api_key(
            "Anthropic",
            "ANTHROPIC_API_KEY",
            "https://console.anthropic.com/settings/keys",
        ),
        "openai-api" => connect_api_key(
            "OpenAI",
            "OPENAI_API_KEY",
            "https://platform.openai.com/api-keys",
        ),
        other => {
            bail!("unknown target `{other}`\n\ntry: claude, codex, near, anthropic-api, openai-api")
        }
    }
}

/// Run `ironwire disconnect <target>`.
pub(crate) fn disconnect(target: &str, subscription: bool, login: LoginArgs) -> Result<()> {
    if let Some(product) = product_for(target) {
        let config = load_config(&paths()?);
        let chosen = resolve(product, &login, &config, Verb::Disconnect { subscription })?;
        announce(&chosen);
        return if subscription {
            revoke(&chosen, &config)
        } else {
            unwire(&chosen.login)
        };
    }
    if login.names_one() {
        bail!(
            "`--config-dir` and `--id` choose a Claude or Codex login, and `{target}` has \
             none\n\ntry: {}",
            Verb::Disconnect { subscription }
                .command(SubscriptionProduct::Claude, " --config-dir ~/.claude-work")
        );
    }
    if subscription {
        bail!("unknown target `{target}`\n\ntry: claude, codex");
    }
    // Anything the catalog taught us about. Looked up by id so a tool that
    // arrived after this binary shipped can still be undone by it.
    match catalog_agent(target) {
        Some(agent) => unwire_catalog_agent(&agent, false),
        None => bail!("unknown target `{target}`"),
    }
}

/// The logins config.toml declares — or none, said out loud, when it cannot
/// be used. Undoing a connect must not depend on a file the user may be in
/// the middle of fixing; an edit to it is refused later on its own terms.
fn load_config(paths: &ironwire_core::config::PathsConfig) -> Config {
    Config::load(paths).unwrap_or_else(|error| {
        println!("{error}");
        println!();
        Config::default()
    })
}

fn product_for(target: &str) -> Option<SubscriptionProduct> {
    match target {
        "claude" => Some(SubscriptionProduct::Claude),
        "codex" => Some(SubscriptionProduct::ChatGpt),
        _ => None,
    }
}

/// A login, worked out from the flags and the environment.
struct Chosen {
    login: Login,
    /// Whether config.toml already says what `login` says. A default login in
    /// its usual place needs no entry, so it always is.
    declared: bool,
    /// `CLAUDE_CONFIG_DIR`, when that is what chose the login.
    from_environment: bool,
}

/// What a command is doing with the login it names, so a hint about a mistake
/// names a command that does the same thing. Told `ironwire connect`, someone
/// taking a login off IronWire would put it back on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Verb {
    Connect,
    Disconnect { subscription: bool },
}

impl Verb {
    /// `ironwire connect claude` and then `rest`, or the disconnect being run.
    fn command(self, product: SubscriptionProduct, rest: &str) -> String {
        let target = connect_target(product);
        match self {
            Self::Connect => format!("ironwire connect {target}{rest}"),
            Self::Disconnect { subscription } => format!(
                "ironwire disconnect {target}{rest}{}",
                if subscription { " --subscription" } else { "" }
            ),
        }
    }
}

/// Which login the user means.
///
/// `--config-dir` names it outright. `--id` names one config.toml already
/// declares. With neither, a `CLAUDE_CONFIG_DIR` in this shell names the
/// Claude Code that would run here — a login of its own, because Claude Code
/// keys its credential to that directory — and otherwise it is the default.
///
/// `CODEX_HOME` is deliberately not read the same way. The default Codex login
/// already follows it, in the daemon as here (`ironwire_creds::codex::auth_path`),
/// and setups and hermetic tests plant it to mean exactly that; reading it as
/// a second login would make one directory two logins. [`connect_login`] says
/// so in a line instead.
fn resolve(
    product: SubscriptionProduct,
    args: &LoginArgs,
    config: &Config,
    verb: Verb,
) -> Result<Chosen> {
    let known = logins::configured(config);
    let variable = logins::kind_of(product)
        .login_dir_variable()
        .unwrap_or_default();

    let mut from_environment = false;
    let dir = match (&args.config_dir, &args.id) {
        (Some(dir), _) => Some(dir.clone()),
        (None, Some(_)) => None,
        (None, None) => {
            let exported = (product == SubscriptionProduct::Claude)
                .then(|| std::env::var("CLAUDE_CONFIG_DIR").ok())
                .flatten()
                .filter(|dir| !dir.is_empty());
            from_environment = exported.is_some();
            exported
        }
    };

    if let Some(dir) = dir {
        let stored = logins::stored_form(&dir, variable)?;
        let existing = logins::by_dir(&known, product, &stored);
        if product == SubscriptionProduct::Claude
            && let Some(other) = logins::claude_directory_alias(&known, &stored)
        {
            bail!(
                "`{stored}` shares a settings directory with `{}`, but names a different \
                macOS Keychain login. IronWire cannot bind both accounts through one settings file. \
                Export CLAUDE_CONFIG_DIR exactly as `{}`, then run this command again; \
                to keep another account, give it a separate directory.",
                other.id,
                other.describe().config_dir.unwrap_or_default()
            );
        }

        if existing.is_none() && logins::is_default_dir(product, &stored) {
            return in_the_usual_place(product, args, config, stored, from_environment, verb);
        }
        let id = match (&args.id, existing) {
            (Some(id), _) => id.clone(),
            // Whatever id config.toml already gave this directory — which is
            // how the command a status screen prints finds its way back to an
            // entry somebody named by hand.
            (None, Some(login)) => login.id.clone(),
            (None, None) => {
                let id = logins::derive_id(product, &stored);
                if let Some(theirs) = BackendImpl::discovered_as(&id) {
                    bail!(
                        "`{stored}` would be named `{id}`, which is the id IronWire gives \
                         {}. Name this login yourself with `--id NAME`{}",
                        theirs.as_str(),
                        if id == product.default_backend_id() && verb == Verb::Connect {
                            format!(", or pass `--id {id}` to move your default login here")
                        } else {
                            String::new()
                        }
                    );
                }
                id
            }
        };
        // Reuse the declared spelling only after establishing that the client
        // and daemon name the same credential identity.
        if let Some(login) = existing
            && login.id == id
        {
            return Ok(Chosen {
                login: login.clone(),
                declared: true,
                from_environment,
            });
        }
        return Ok(Chosen {
            login: Login {
                id,
                product,
                config_dir: Some(stored),
                enabled: true,
            },
            declared: false,
            from_environment,
        });
    }

    if let Some(id) = &args.id
        && id != product.default_backend_id()
    {
        let Some(login) = known.iter().find(|login| &login.id == id) else {
            if let Some(entry) = config.backends.iter().find(|b| &b.id == id) {
                bail!(
                    "`{id}` is a `{}` backend, not a {} login",
                    entry.kind,
                    product.client()
                );
            }
            let others: Vec<String> = known
                .iter()
                .filter(|login| login.product == product)
                .map(|login| format!("`{}` ({})", login.id, login.location()))
                .collect();
            let next = match verb {
                Verb::Connect => format!(
                    " To add one, run `{}`",
                    verb.command(product, &format!(" --config-dir DIR --id {id}"))
                ),
                Verb::Disconnect { .. } => " Name one of those with `--id`.".to_string(),
            };
            bail!(
                "config.toml declares no {} login called `{id}`; the ones IronWire knows \
                 are {}.{next}",
                product.client(),
                others.join(", ")
            );
        };
        if login.product != product {
            bail!(
                "`{id}` is a {} login, not a {} one: run `{}`",
                login.product.client(),
                product.client(),
                verb.command(login.product, &format!(" --id {id}"))
            );
        }
        return Ok(Chosen {
            login: login.clone(),
            declared: true,
            from_environment,
        });
    }

    Ok(Chosen {
        login: Login::default_for(product, config),
        declared: true,
        from_environment,
    })
}

/// `--config-dir` naming the directory a product's client keeps its default
/// login in: the default login, moved there, and never a second login.
///
/// A second one would keep its settings in the default's file, which can only
/// ever be pointed at one of them, and on Linux would read the default's
/// credential file: one pool counted twice. It is still a move, not a no-op —
/// Claude Code run with `CLAUDE_CONFIG_DIR` set, even to `~/.claude`, keeps its
/// login in a Keychain item of its own.
fn in_the_usual_place(
    product: SubscriptionProduct,
    args: &LoginArgs,
    config: &Config,
    stored: String,
    from_environment: bool,
    verb: Verb,
) -> Result<Chosen> {
    let default_id = product.default_backend_id();
    if let Some(id) = args.id.as_deref().filter(|id| *id != default_id) {
        bail!(
            "`{stored}` is where your default {} login lives, so it cannot be a second \
             login called `{id}`: both would keep their settings in one file. `{}` means \
             that directory's login, which is your default one",
            product.name(),
            verb.command(product, &format!(" --config-dir {stored}"))
        );
    }
    // An entry for the default already says where it is, and config.toml is
    // only ever appended to — so moving it is an edit to that line.
    if verb == Verb::Connect
        && let Some(entry) = config.backends.iter().find(|b| b.id == default_id)
    {
        let declared = entry.config_dir.as_deref().map_or_else(
            || "with no `config_dir`".to_string(),
            |dir| format!("in `{dir}`"),
        );
        bail!(
            "`{stored}` is where your default {} login lives, so naming it moves \
             `{default_id}` there — and config.toml already declares `{default_id}` \
             {declared}. To move it, set `config_dir = \"{stored}\"` on that entry, then \
             run this again",
            product.name()
        );
    }
    Ok(Chosen {
        login: Login {
            id: default_id.to_string(),
            product,
            config_dir: Some(stored),
            enabled: Login::default_for(product, config).enabled,
        },
        declared: false,
        from_environment,
    })
}

fn connect_target(product: SubscriptionProduct) -> &'static str {
    match product {
        SubscriptionProduct::Claude => "claude",
        SubscriptionProduct::ChatGpt => "codex",
    }
}

/// Say which login the environment chose, so nobody connects one while
/// believing they connected the other.
fn announce(chosen: &Chosen) {
    if chosen.from_environment {
        println!(
            "CLAUDE_CONFIG_DIR is set to {}, so this is about the Claude Code that uses \
             it — a login of its own. Unset it to mean your default login.",
            chosen.login.location()
        );
        println!();
    }
}

/// `connect claude` / `connect codex`, for one login.
fn connect_login(
    product: SubscriptionProduct,
    args: &LoginArgs,
    subscription: bool,
    dry_run: bool,
    port: u16,
) -> Result<()> {
    let paths = paths()?;
    let config = load_config(&paths);
    let chosen = resolve(product, args, &config, Verb::Connect)?;
    let login = &chosen.login;

    println!("{} → IronWire", login.client_label());
    println!();
    announce(&chosen);
    // A default login in its usual place is always declared, so this is a
    // default that this command moves.
    if !chosen.declared && login.is_default() {
        say_the_default_moves(login);
    }
    warn_shared_settings(login);
    if product == SubscriptionProduct::ChatGpt
        && !args.names_one()
        && let Ok(home) = std::env::var("CODEX_HOME")
        && !home.is_empty()
    {
        let home = logins::shown(Path::new(&home));
        println!(
            "CODEX_HOME is set to {home}; IronWire treats it as your default Codex login, \
             as it always has. If it is a second login, run `ironwire connect codex \
             --config-dir {home}` instead."
        );
        println!();
    }
    if !login.enabled {
        println!(
            "Note: `{}` is switched off in config.toml (`enabled = false`), so the daemon \
             will not use it until that line goes.",
            login.id
        );
        println!();
    }

    let registers = !chosen.declared && declare(&paths, login, dry_run)? && !dry_run;
    wire_login(login, port, dry_run)?;
    warn_left_behind(login);
    println!();

    if !found_login(login) {
        restart_note(login, registers);
        return Ok(());
    }

    let described = login.describe();
    let enable = format!("{} --subscription", described.connect_command());
    if !subscription {
        println!();
        println!("To let IronWire use that subscription, run:\n\n    {enable}\n");
        restart_note(login, registers);
        return Ok(());
    }

    if dry_run {
        println!();
        println!(
            "[dry run] would record consent for the `{}` backend.",
            login.id
        );
        return Ok(());
    }

    let path = paths.consent_file();
    let mut ledger = ConsentLedger::load(&path);
    if ledger.is_granted_for(&login.id, described.consent_location()) {
        println!();
        println!("The {} backend is already enabled.", described.label());
        restart_note(login, registers);
        return Ok(());
    }

    if !ask_subscription_consent(&ConsentPrompt::for_login(&described, &login.id))? {
        println!("Left disabled. IronWire will use API keys only.");
        return Ok(());
    }

    // Against the directory the login is read from: the id alone does not say
    // which account the user agreed to hand over.
    ledger.grant_for(&login.id, described.consent_location(), Utc::now());
    ledger.save(&path).context("writing the consent ledger")?;
    println!();
    println!("Enabled. Recorded in {}.", path.display());
    if registers {
        restart_note(login, registers);
    } else {
        println!("Start the daemon with `ironwire serve`.");
    }
    Ok(())
}

/// Report what the login holds. `false` when there is nothing to enable, after
/// saying what would fix that.
fn found_login(login: &Login) -> bool {
    let next = |what: &str| {
        if login.owns_the_plain_url() {
            println!("IronWire can still route {what}.");
        } else {
            println!(
                "Log it in with `{}`, then run `{} --subscription`.",
                login.login_command(),
                login.describe().connect_command()
            );
        }
    };
    match login.product {
        SubscriptionProduct::Claude => match login.claude().read() {
            Ok(creds) => {
                let plan = creds.subscription_type.as_deref().unwrap_or("unknown plan");
                println!("Found a Claude Code login ({plan}) at {}.", creds.source);
                true
            }
            Err(e) => {
                println!("No Claude Code login found: {e}");
                next("through ANTHROPIC_API_KEY");
                false
            }
        },
        SubscriptionProduct::ChatGpt => match login.codex().read() {
            Ok(creds) if creds.mode == CodexMode::ChatGpt => {
                println!("Found a ChatGPT login (Codex) at {}.", creds.source);
                true
            }
            // There is no subscription in it to consent to. Offering the
            // question anyway would record consent for a backend that can
            // never exist.
            Ok(creds) => {
                println!(
                    "{} holds an OpenAI API key rather than a ChatGPT login.",
                    creds.source
                );
                if login.owns_the_plain_url() {
                    println!(
                        "IronWire uses it as a metered key; there is no subscription to enable."
                    );
                } else {
                    println!(
                        "To use a ChatGPT subscription from there, log in with `{}`.",
                        login.login_command()
                    );
                }
                false
            }
            Err(e) => {
                println!("No Codex login found: {e}");
                next("Codex through OPENAI_API_KEY");
                false
            }
        },
    }
}

/// Declare a login with a directory of its own — an additional one, or the
/// default moved — in config.toml, naming the file first.
///
/// Returns whether anything changed — and so whether the daemon has a login
/// to pick up when it next starts.
fn declare(
    paths: &ironwire_core::config::PathsConfig,
    login: &Login,
    dry_run: bool,
) -> Result<bool> {
    let Some(dir) = login.config_dir.as_deref() else {
        return Ok(false);
    };
    let path = paths.config_file();
    let existing = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
    };
    let edited = ironwire_core::config_edit::ensure_subscription_entry(
        &existing,
        &login.id,
        login.kind(),
        dir,
    )
    .map_err(|e| anyhow!("{}: {e}", path.display()))?;
    if edited == existing {
        return Ok(false);
    }
    println!("Writing {}:", path.display());
    println!(
        "  · [[backends]] `{}`: {} in {dir} (added)",
        login.id,
        login.kind().as_str()
    );
    if dry_run {
        println!("  [dry run] nothing was written.");
    } else {
        ironwire_core::atomic::write(&path, &edited)
            .with_context(|| format!("writing {}", path.display()))?;
    }
    println!();
    Ok(true)
}

/// The daemon builds its backends once, at start.
fn restart_note(login: &Login, registers: bool) {
    if registers {
        println!(
            "The daemon reads its logins when it starts: restart it (or start it with \
             `ironwire serve`) to register `{}`.",
            login.id
        );
    }
}

/// Say that this connect moves the default login, before it does: the id
/// stays, and the account behind it changes.
fn say_the_default_moves(login: &Login) {
    let Some(dir) = login.config_dir.as_deref() else {
        return;
    };
    let location = login.location();
    let name = login.product.name();
    if logins::is_default_dir(login.product, dir) {
        println!(
            "{location} is where your default {name} login lives, so this moves `{id}` \
             there rather than adding a second login: from now on it reads the login {client} \
             keeps for {variable}={location}, and only the URL naming it reaches it.",
            id = login.id,
            client = login.product.client(),
            variable = login.kind().login_dir_variable().unwrap_or_default(),
        );
    } else {
        println!(
            "This moves your default {name} login: from now on `{}` reads the login in \
             {location}, and only the URL naming it reaches it.",
            login.id
        );
    }
    println!();
}

/// Say so, before anything is written, when a Claude login's client shares
/// its settings file with a `claude` logged in as someone else.
///
/// On macOS, Claude Code run with `CLAUDE_CONFIG_DIR=~/.claude` keeps its
/// login in a Keychain item named after that string, apart from the one plain
/// `claude` uses, while both read `~/.claude/settings.json`. The URL in that
/// file is all IronWire knows a client's login by, so it serves both from
/// whichever login the file names. Elsewhere both read one credentials file:
/// one login, nothing to say.
pub(crate) fn warn_shared_settings(login: &Login) {
    if login.product != SubscriptionProduct::Claude
        || !login
            .config_dir
            .as_deref()
            .is_some_and(logins::shares_plain_claudes_settings)
    {
        return;
    }
    let location = login.location();
    let file = login.client_config().map_or_else(
        || format!("{location}/settings.json"),
        |p| logins::shown(&p),
    );
    println!(
        "Note: on macOS, Claude Code run with CLAUDE_CONFIG_DIR={location} keeps its login \
         in a Keychain item of its own, and plain `claude` keeps another — but both read \
         {file}. Any `claude` started without CLAUDE_CONFIG_DIR reads the same settings.json \
         and will be served by this login, `{id}`. If those are two accounts, take \
         `config_dir` off `{id}` in config.toml and run `ironwire connect claude` from a \
         shell without CLAUDE_CONFIG_DIR.",
        id = login.id,
    );
    println!();
}

/// Say so when moving the default login leaves another client on IronWire.
///
/// The client still started without a directory of its own keeps reading the
/// file the default used to be wired in. On the plain URL it gets no
/// subscription once the default has moved; on the URL naming the moved login
/// it would get another account's.
pub(crate) fn warn_left_behind(login: &Login) {
    let Some(path) = login.left_behind() else {
        return;
    };
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let ours = match login.product {
        SubscriptionProduct::Claude => {
            claude_settings::base_url(&text).and_then(|url| endpoint::parse_anthropic(&url))
        }
        SubscriptionProduct::ChatGpt => codex_config::is_wired(&text)
            .then(|| codex_config::base_url(&text))
            .flatten()
            .and_then(|url| endpoint::parse_openai(&url)),
    };
    let Some(ours) = ours else {
        return;
    };
    let file = logins::shown(&path);
    let dir = path.parent().map(logins::shown).unwrap_or_default();
    let client = login.product.client();
    println!();
    let undo = Verb::Disconnect {
        subscription: false,
    }
    .command(login.product, &format!(" --config-dir {dir}"));
    match ours.account {
        None => println!(
            "Note: {file} still points {client} at IronWire's plain URL, and with `{id}` \
             reading {location} no subscription serves that: the {client} that uses {dir} \
             gets an API key or whatever else IronWire has, or is refused. `{undo}` sends it \
             straight to {vendor} again; taking `config_dir` off `{id}` in config.toml moves \
             your default login back.",
            id = login.id,
            location = login.location(),
            vendor = login.product.vendor(),
        ),
        Some(bound) if bound == login.id => println!(
            "Note: {file} still points {client} at the URL naming `{id}`, which now reads \
             {location}: the {client} that uses {dir} would be served by another login's \
             subscription. `{undo}` takes it off IronWire.",
            id = login.id,
            location = login.location(),
        ),
        Some(_) => {}
    }
}

/// Withdraw one login's consent.
fn revoke(chosen: &Chosen, config: &Config) -> Result<()> {
    let login = &chosen.login;
    let paths = paths()?;
    let path = paths.consent_file();
    let mut ledger = ConsentLedger::load(&path);
    let described = login.describe();
    let label = described.label();
    let answered = ledger.is_granted_for(&login.id, described.consent_location());
    let before = ledger.clone();
    // By id, whichever directory or prompt the record answered: withdrawing
    // must never leave one in place on a technicality. Saved either way, so
    // an answer to an older prompt goes too.
    ledger.revoke(&login.id);
    ledger.save(&path).context("writing the consent ledger")?;
    if answered {
        println!("Revoked consent for the {label} (`{}`).", login.id);
    } else if ledger != before {
        println!("Revoked the consent recorded for `{}`.", login.id);
    } else {
        println!("No consent is recorded for the {label} (`{}`).", login.id);
    }
    if ledger != before {
        println!("Restart the daemon for this to take effect: `ironwire serve`");
    }
    default_still_consented(chosen, config, &ledger);
    Ok(())
}

/// A `CLAUDE_CONFIG_DIR` chose the login — while the default one may still
/// hold consent: a yes given from this same shell before that variable named
/// a login of its own was recorded for the default, and withdrawing the
/// shell's login, whether it held a record or not, leaves that one in force.
fn default_still_consented(chosen: &Chosen, config: &Config, ledger: &ConsentLedger) {
    let default = Login::default_for(chosen.login.product, config);
    if !chosen.from_environment || chosen.login.id == default.id {
        return;
    }
    if ledger.is_granted_for(&default.id, default.describe().consent_location()) {
        println!(
            "Your default login (`{id}`, {location}) still holds consent. To withdraw it, \
             run `{command}`, or unset CLAUDE_CONFIG_DIR and run this again.",
            id = default.id,
            location = default.location(),
            command = Verb::Disconnect { subscription: true }
                .command(default.product, &format!(" --id {}", default.id)),
        );
    }
}

/// Take one login's client back off IronWire, in the file that client reads.
fn unwire(login: &Login) -> Result<()> {
    let path = login
        .client_config()
        .context("could not locate your home directory")?;
    match login.product {
        SubscriptionProduct::Claude => unwire_claude(&path),
        // Codex is pointed here by a file we wrote, so we can undo it.
        SubscriptionProduct::ChatGpt => disconnect_codex(&path),
    }
}

/// A catalog-described tool by id, if the installed document has one.
pub(crate) fn catalog_agent(id: &str) -> Option<ironwire_catalog::schema::AgentEntry> {
    let paths = ironwire_core::config::PathsConfig::resolve().ok()?;
    let store = ironwire_catalog::CatalogStore::load(
        ironwire_catalog::CATALOG_PUBLIC_KEY,
        &paths.catalog_file(),
    );
    store
        .current()
        .agents()
        .into_iter()
        .find(|agent| agent.id == id)
        .cloned()
}

/// Print the environment a client needs.
pub(crate) fn print_env(port: Option<u16>, shell: Option<String>) -> Result<()> {
    let port = port.unwrap_or(DEFAULT_PORT);
    // The Claude Code this shell starts is the login in `CLAUDE_CONFIG_DIR`
    // when that is set, and the plain URL would have the default login pay
    // for it. Read quietly: whatever reaches stdout here goes into `eval`.
    let url = match std::env::var("CLAUDE_CONFIG_DIR") {
        Ok(dir) if !dir.is_empty() => {
            let config = ironwire_core::config::PathsConfig::resolve()
                .ok()
                .and_then(|paths| Config::load(&paths).ok())
                .unwrap_or_default();
            let known = logins::configured(&config);
            if let Some(login) = logins::by_dir(&known, SubscriptionProduct::Claude, &dir) {
                login.url(port)
            } else {
                if logins::undeclared_claude_dir(&known).is_some() {
                    eprintln!(
                        "note: CLAUDE_CONFIG_DIR is {dir}, a login IronWire does not know \
                         yet; `ironwire connect claude` {}",
                        logins::connect_would(&dir)
                    );
                }
                endpoint::anthropic_url(port, None)
            }
        }
        _ => endpoint::anthropic_url(port, None),
    };

    // Default to the shell the user is actually running, not to bash. Someone
    // in fish who pipes `export FOO=bar` into `eval` gets a syntax error and no
    // idea why, which is a miserable first five minutes.
    let shell = shell.unwrap_or_else(|| {
        std::env::var("SHELL")
            .ok()
            .and_then(|path| {
                std::path::Path::new(&path)
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "sh".to_string())
    });

    match shell.as_str() {
        "fish" => println!("set -gx ANTHROPIC_BASE_URL {url}"),
        "powershell" | "pwsh" => println!("$env:ANTHROPIC_BASE_URL = \"{url}\""),
        "cmd" => println!("set ANTHROPIC_BASE_URL={url}"),
        "nu" | "nushell" => println!("$env.ANTHROPIC_BASE_URL = \"{url}\""),
        // bash, zsh, ksh, dash, sh, and anything else POSIX-ish.
        _ => println!("export ANTHROPIC_BASE_URL={url}"),
    }
    Ok(())
}

fn disconnect_codex(path: &Path) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    if existing.is_empty() {
        println!("{} does not exist — nothing to undo.", path.display());
        return Ok(());
    }
    let edit = codex_config::disconnect(&existing)
        .with_context(|| format!("{} is not valid TOML", path.display()))?;
    if edit.is_noop() {
        println!("{} does not point at IronWire.", path.display());
        return Ok(());
    }
    println!("This will change {}:", path.display());
    for change in &edit.changes {
        println!("  · {change}");
    }
    write_with_backup(path, &existing, &edit.contents, "toml")?;
    println!();
    println!("Written. Restart Codex to pick it up.");
    Ok(())
}

/// Point one login's client at IronWire, in the file that client reads, with
/// the URL that tells the daemon which login it is.
pub(crate) fn wire_login(login: &Login, port: u16, dry_run: bool) -> Result<()> {
    let path = login
        .client_config()
        .context("could not locate your home directory")?;
    match login.product {
        SubscriptionProduct::Claude => wire_claude(&path, &login.url(port), dry_run),
        SubscriptionProduct::ChatGpt => {
            let undo = login.describe().connect_command().replacen(
                "ironwire connect",
                "ironwire disconnect",
                1,
            );
            wire_codex(&path, port, login.account(), &undo, dry_run)
        }
    }
}

/// Point Claude Code at IronWire, in the file Claude Code already reads.
///
/// Two slots, one write:
///
/// - `env.ANTHROPIC_BASE_URL`, which is what actually routes it here. This used
///   to be an `export` line we printed for the user to run, which meant the
///   setup did not survive a new terminal and `doctor` spent its life
///   explaining that. A setting in the file is the same decision, made once.
///   For a login with a directory of its own the URL names that login, which
///   is how the daemon knows whose subscription pays.
/// - `statusLine`, IronWire's one line of screen space. IronWire will not write
///   into a response stream, so without this the only place it can say "your
///   traffic just moved" is a second terminal nobody is looking at.
///
/// Neither is taken from a user already using it — see [`claude_settings`].
fn wire_claude(path: &Path, url: &str, dry_run: bool) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let command = format!("{} statusline", our_binary()?);
    // Always `Some` here: this is the `ironwire` binary, and it implements
    // `ironwire statusline`. A host embedding the library names its own
    // command, or passes `None` — see `ironwire_agents::tools::StatusLine`.
    let edit =
        claude_settings::connect(&existing, Some(&command), Some(url)).with_context(|| {
            format!(
                "{} is not valid JSON — IronWire will not rewrite a file it cannot read",
                path.display()
            )
        })?;

    if !edit.is_noop() {
        // TRUST.md: name the file before touching it, every time.
        println!("Writing {}:", path.display());
        for change in &edit.changes {
            println!("  · {change}");
        }
        if dry_run {
            println!("  [dry run] nothing was written.");
        } else {
            write_with_backup(path, &existing, &edit.contents, "json")?;
            println!("  Claude Code picks this up on its next start.");
        }
    }

    // A slot of their own is not a failure, but it does mean the thing we
    // promised did not happen — so say what would make it happen by hand.
    if let Some(theirs) = edit.occupied_slot("ANTHROPIC_BASE_URL") {
        println!("  ANTHROPIC_BASE_URL is already set to `{theirs}`, so IronWire left it.");
        println!("  To route Claude Code here instead, set it to: {url}");
    }
    if let Some(theirs) = edit.occupied_slot("statusLine") {
        println!("  You already have a status line (`{theirs}`), so IronWire left it alone.");
        println!("  To include IronWire in it, add the output of: {command}");
    }
    kept_binding(path, &edit.contents, url, claude_settings::account);
    Ok(())
}

/// Say so when a file stays bound to a login this connect did not name.
///
/// That is the edit working as intended — a connect for the default login does
/// not move a client off the login it was bound to — but a person who expected
/// otherwise should not have to diff the file to find out.
fn kept_binding(path: &Path, contents: &str, url: &str, account: fn(&str) -> Option<String>) {
    if let Some(bound) = account(contents)
        && !url.contains(&format!("/account/{bound}"))
    {
        println!(
            "  {} keeps routing as the login `{bound}`; `ironwire connect {} --id {bound}` \
             is the command that names it.",
            path.display(),
            if url.contains("/anthropic") {
                "claude"
            } else {
                "codex"
            }
        );
    }
}

/// Whether a catalog-described tool looks installed here.
pub(crate) fn catalog_agent_installed(agent: &ironwire_catalog::schema::AgentEntry) -> bool {
    let Some(home) = dirs::home_dir() else {
        return false;
    };
    ironwire_agents::catalog::detected(agent, &home, &on_path)
}

/// Point a catalog-described tool at IronWire.
///
/// The same shape as [`wire_claude`] and [`wire_codex`] on purpose: name the
/// file before touching it, show every change, and when a slot turns out to be
/// the user's, say what would make it ours by hand instead of doing it for
/// them. The only difference is that nothing here is compiled in — the file and
/// the keys come from the signed catalog, and the value never does.
pub(crate) fn wire_catalog_agent(
    agent: &ironwire_catalog::schema::AgentEntry,
    port: u16,
    dry_run: bool,
) -> Result<()> {
    let home = dirs::home_dir().context("no home directory")?;
    let path = agent.config.resolve(&home);
    let existing = std::fs::read_to_string(&path).unwrap_or_default();

    let edit = ironwire_agents::catalog::connect(agent, &existing, port)
        .with_context(|| format!("{} could not be edited", path.display()))?;

    if !edit.is_noop() {
        println!("Writing {}:", path.display());
        for change in &edit.changes {
            println!("  · {change}");
        }
        if dry_run {
            println!("  [dry run] nothing was written.");
        } else {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("creating {}", dir.display()))?;
            }
            let extension = if agent.config.file.ends_with(".toml") {
                "toml"
            } else {
                "json"
            };
            write_with_backup(&path, &existing, &edit.contents, extension)?;
            println!("  {} picks this up on its next start.", agent.name);
        }
    }

    for occupied in &edit.occupied {
        println!(
            "  {} is already set to `{}`, so IronWire left it.",
            occupied.slot, occupied.current
        );
    }
    // Said out loud, because the alternative is a connect that changes nothing
    // and explains nothing — and the person reading is the only one who can
    // tell whether that is the right answer for their setup.
    for skipped in &edit.skipped {
        println!(
            "  {} was left unset: this config has no `{}`, so {} is not \
             configured to use that provider here.",
            skipped.slot, skipped.requires, agent.name
        );
    }
    Ok(())
}

/// Undo [`wire_catalog_agent`], removing only what it added.
pub(crate) fn unwire_catalog_agent(
    agent: &ironwire_catalog::schema::AgentEntry,
    dry_run: bool,
) -> Result<()> {
    let home = dirs::home_dir().context("no home directory")?;
    let path = agent.config.resolve(&home);
    let Ok(existing) = std::fs::read_to_string(&path) else {
        return Ok(());
    };

    let edit = ironwire_agents::catalog::disconnect(agent, &existing)
        .with_context(|| format!("{} could not be edited", path.display()))?;
    if edit.is_noop() {
        return Ok(());
    }

    println!("Writing {}:", path.display());
    for change in &edit.changes {
        println!("  · {change}");
    }
    if dry_run {
        println!("  [dry run] nothing was written.");
    } else {
        let extension = if agent.config.file.ends_with(".toml") {
            "toml"
        } else {
            "json"
        };
        write_with_backup(&path, &existing, &edit.contents, extension)?;
    }
    Ok(())
}

/// Point Codex at IronWire, in the config file Codex already reads.
///
/// The `model_provider` line is not taken from a user already using it — see
/// [`ironwire_agents::codex_config`] — which is why the occupied slot is said
/// even when there was nothing to write.
fn wire_codex(
    path: &Path,
    port: u16,
    account: Option<&str>,
    undo: &str,
    dry_run: bool,
) -> Result<()> {
    let existing = std::fs::read_to_string(path).unwrap_or_default();
    let edit = codex_config::connect_as(&existing, port, account).with_context(|| {
        format!(
            "{} is not valid TOML — IronWire will not append to a file it cannot read",
            path.display()
        )
    })?;

    if !edit.is_noop() {
        println!("Writing {}:", path.display());
        for change in &edit.changes {
            println!("  · {change}");
        }
        // Said before the edit, like every other change in this file. This one
        // is not IronWire's limitation, but IronWire is what makes the user meet
        // it, and finding out afterwards — with a thread stuck on a model and no
        // UI to change it — is the worst way to learn.
        println!();
        println!("  One consequence worth knowing: Codex has no UI for changing the");
        println!("  model on a custom provider (openai/codex#15364). A desktop thread");
        println!("  keeps whatever model it was created with, and the CLI needs `-m`");
        println!("  or `model =` in config.toml. `{undo}`");
        println!("  puts it all back.");
        println!();
        if dry_run {
            println!("  [dry run] nothing was written.");
        } else {
            write_with_backup(path, &existing, &edit.contents, "toml")?;
            // One file drives both clients, and "restart" means different things
            // to each.
            println!("  CLI: start a new `codex` session.");
            println!("  Desktop: quit the app and relaunch it — reopening a window is not");
            println!("  enough, it keeps the old config.");
        }
    }

    // A provider of their own is not a failure, but it does mean the thing we
    // promised did not happen — so say what would make it happen by hand.
    if let Some(theirs) = edit.occupied_slot("model_provider") {
        println!("  model_provider is already set to `{theirs}`, so IronWire left it.");
        println!("  To route Codex here instead, set it to: model_provider = \"ironwire\"");
    }
    kept_binding(
        path,
        &edit.contents,
        &endpoint::openai_url(port, account),
        codex_config::account,
    );
    Ok(())
}

/// Write a file we did not create, keeping a copy of what was there.
///
/// The backup is what makes an automatic edit defensible: the user can always
/// get back exactly what they had, without us having to be right about what
/// mattered in it.
fn write_with_backup(
    path: &std::path::Path,
    existing: &str,
    contents: &str,
    extension: &str,
) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    if !existing.is_empty() {
        let backup = path.with_extension(format!("{extension}.ironwire-backup"));
        std::fs::write(&backup, existing)
            .with_context(|| format!("writing {}", backup.display()))?;
        println!("  (previous contents saved to {})", backup.display());
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}

/// Whether Claude Code is on this machine at all.
///
/// Either signal is enough: the config directory means it has run here, and the
/// binary means it can. Wiring an agent that is not installed writes settings
/// nothing will read, which is how a setup step turns into litter.
pub(crate) fn claude_installed() -> bool {
    claude_settings::default_dir().is_some_and(|dir| dir.exists()) || on_path("claude")
}

/// Whether Codex is on this machine at all.
pub(crate) fn codex_installed() -> bool {
    codex_config::path().is_some_and(|path| path.parent().is_some_and(Path::exists))
        || on_path("codex")
}

/// Undo [`wire_claude`], removing only what it added.
fn unwire_claude(path: &Path) -> Result<()> {
    let Ok(existing) = std::fs::read_to_string(path) else {
        println!("{} does not exist — nothing to undo.", path.display());
        return Ok(());
    };
    let edit = claude_settings::disconnect(&existing).with_context(|| {
        format!(
            "{} is not valid JSON — IronWire will not rewrite a file it cannot read",
            path.display()
        )
    })?;
    if edit.is_noop() {
        println!("{} has nothing of IronWire's in it.", path.display());
        return Ok(());
    }
    println!("Writing {}:", path.display());
    for change in &edit.changes {
        println!("  · {change}");
    }
    write_with_backup(path, &existing, &edit.contents, "json")?;
    println!("  Claude Code goes back to Anthropic on its next start.");
    Ok(())
}

/// This binary's own path, so the status line keeps working for someone who
/// installed IronWire somewhere that is not on `PATH` — which is most people
/// running it from a build.
fn our_binary() -> Result<String> {
    let path = std::env::current_exe().context("locating the ironwire binary")?;
    Ok(path.display().to_string())
}

/// NEAR AI credits — the cross-family fallback lane.
fn connect_near(port: u16) -> Result<()> {
    println!("NEAR AI → IronWire");
    println!();

    match std::env::var("NEARAI_API_KEY") {
        Ok(key) if !key.is_empty() => {
            // Never the key itself, here or anywhere (`docs/TRUST.md` §5).
            println!(
                "Found NEARAI_API_KEY in your environment ({} chars).",
                key.len()
            );
            println!();
            println!("Restart the daemon to pick it up, then check it end to end:");
            println!();
            println!("    ironwire serve --port {port}");
            println!("    ironwire doctor");
        }
        _ => {
            println!("No NEARAI_API_KEY found.");
            println!();
            println!("Get a key at https://app.near.ai, then:");
            println!();
            println!("    export NEARAI_API_KEY=...");
            println!("    ironwire serve");
        }
    }

    println!();
    println!("NEAR AI is a different API family, so IronWire reaches it through");
    println!("the translated lane — and only at a turn boundary, never mid tool");
    println!("loop (docs/PROTOCOL.md §6). When a conversation moves there, your");
    println!("agent is talking to a different model family; `ironwire watch`");
    println!("tells you the moment it happens.");
    println!();
    println!("Device-key enrolment and trace-contribution credits land with M6");
    println!("(docs/ROADMAP.md). Today the key is all that is needed.");
    Ok(())
}

/// A metered API key. Nothing to write — IronWire keeps no secrets in
/// `config.toml` (`docs/TRUST.md` §5) — so this explains and verifies.
fn connect_api_key(vendor: &str, env: &str, url: &str) -> Result<()> {
    println!("{vendor} API key → IronWire");
    println!();
    match std::env::var(env) {
        Ok(key) if !key.is_empty() => {
            println!("Found {env} in your environment ({} chars).", key.len());
            println!();
            println!("Restart the daemon to pick it up:");
            println!();
            println!("    ironwire serve");
            println!("    ironwire doctor");
        }
        _ => {
            println!("No {env} found.");
            println!();
            println!("Get a key at {url}, then:");
            println!();
            println!("    export {env}=...");
            println!("    ironwire serve");
        }
    }
    println!();
    println!("IronWire reads this from the environment and never writes it to");
    println!("disk. There is no `ironwire` config field that holds a secret.");
    Ok(())
}

/// Ask the consent question and read the answer.
///
/// The wording is [`ConsentPrompt`], not this function: the menu bar app has to
/// ask the same question, and two hand-maintained copies of a consent prompt are
/// two prompts — while the recorded version goes on claiming both users answered
/// the same one. This renders it for a terminal and does nothing else.
fn ask_subscription_consent(prompt: &ConsentPrompt) -> Result<bool> {
    println!();
    for line in wrap(&prompt.summary, 72) {
        println!("  {line}");
    }
    println!();
    for point in &prompt.points {
        let mut lines = wrap(point, 70).into_iter();
        if let Some(first) = lines.next() {
            println!("  · {first}");
        }
        for rest in lines {
            println!("    {rest}");
        }
    }
    println!();
    print!("  {} [y/N] ", prompt.question);
    std::io::stdout().flush().ok();

    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("reading your answer")?;
    Ok(matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes"
    ))
}

/// Break a sentence onto lines of at most `width`, on word boundaries.
///
/// The prompt is stored as sentences rather than pre-broken lines so that a GUI
/// can lay it out for its own width; a terminal has to do the breaking itself.
pub(crate) fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for word in text.split_whitespace() {
        if !current.is_empty() && current.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut current));
        }
        if !current.is_empty() {
            current.push(' ');
        }
        current.push_str(word);
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}
