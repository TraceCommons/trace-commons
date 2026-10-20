//! `ironwire init` — set IronWire up, in one command.
//!
//! It looks at what is on the machine, asks the one question that has to be
//! asked, wires up every agent it finds, and leaves the daemon running. When it
//! returns, `claude` works.
//!
//! It used to print the commands instead of running them — five of them, plus
//! an `export` and a second terminal — on the reasoning that a setup step the
//! user runs themselves is a setup step they consented to. That reasoning is
//! right about exactly one thing, subscription credentials (`docs/TRUST.md`
//! §2), and wrong about the rest: writing a settings file we then name, or
//! starting a daemon the user just asked for, is not a decision they need to
//! make twice. So the consent gate stays, as one prompt, and everything else
//! happens.
//!
//! `--dry-run` shows every change without making one.

use std::io::Write as _;
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use ironwire_agents::{claude_settings, codex_config};
use ironwire_core::config::Config;
use ironwire_creds::codex::{CodexCredentials, CodexMode};
use ironwire_creds::consent::ConsentLedger;
use ironwire_creds::{SubscriptionLogin, SubscriptionProduct};

use super::logins::{self, Login};
use super::{connect, paths, service};

/// A key an agent on this machine is already configured to use.
///
/// Worth finding because it is capacity the user already has, and worth
/// reporting separately because IronWire reads keys from the *daemon's*
/// environment: a key that lives only in an agent's config file is one the
/// daemon cannot see, and that gap is invisible until a request fails.
struct AgentKey {
    /// Which agent's config it came out of.
    agent: &'static str,
    /// The variable it names. Never the value — `docs/TRUST.md` §5.
    var: String,
    /// Whether the daemon will actually be able to read it.
    in_environment: bool,
}

/// A subscription login IronWire would use, and whether it can.
struct FoundLogin {
    login: Login,
    /// Whether it holds a credential its backend can use right now.
    usable: bool,
}

/// What IronWire found on this machine.
struct Found {
    /// The default logins that are logged in, and every login config.toml
    /// declares whether it is or not — a declared login is one the user asked
    /// about, so "not logged in" is an answer worth a line.
    logins: Vec<FoundLogin>,
    codex_api_key: bool,
    anthropic_key: bool,
    openai_key: bool,
    nearai_key: bool,
    agent_keys: Vec<AgentKey>,
    /// A `CLAUDE_CONFIG_DIR` in this shell that no declared login uses.
    unknown_claude_dir: Option<String>,
}

impl Found {
    fn detect(config: &Config) -> Self {
        let declared = logins::configured(config);
        let logins = declared
            .iter()
            .filter(|login| login.enabled)
            .map(|login| FoundLogin {
                usable: login.check().is_ok(),
                login: login.clone(),
            })
            // A default login in its usual place is something we looked for,
            // not something the user named; its absence is not news.
            .filter(|found| found.usable || found.login.config_dir.is_some())
            .collect();
        let unknown_claude_dir = logins::undeclared_claude_dir(&declared);
        Self {
            logins,
            codex_api_key: CodexCredentials::discover().is_ok_and(|c| c.mode == CodexMode::ApiKey),
            anthropic_key: has_env("ANTHROPIC_API_KEY"),
            openai_key: has_env("OPENAI_API_KEY"),
            nearai_key: has_env("NEARAI_API_KEY"),
            agent_keys: detect_agent_keys(&declared),
            unknown_claude_dir,
        }
    }

    fn anything(&self) -> bool {
        self.logins.iter().any(|found| found.usable)
            || self.codex_api_key
            || self.anthropic_key
            || self.openai_key
            || self.nearai_key
    }

    /// Logins that are here but not yet consented to — including one whose
    /// consent was given while its id read a login somewhere else.
    fn ungranted_subscriptions(&self, consent: &ConsentLedger) -> Vec<Pending> {
        self.logins
            .iter()
            .filter(|found| found.usable)
            .map(|found| Pending {
                id: found.login.id.clone(),
                login: found.login.describe(),
                location: found.login.location(),
            })
            .filter(|p| !consent.is_granted_for(&p.id, p.login.consent_location()))
            .collect()
    }
}

/// One login the consent question is about.
struct Pending {
    /// The backend id consent is recorded under.
    id: String,
    login: SubscriptionLogin,
    /// Where it lives, for people — a default login's too, which
    /// `SubscriptionLogin` leaves unsaid.
    location: String,
}

/// The one question `init` asks, as text.
#[derive(Debug, PartialEq, Eq)]
struct Question {
    /// What IronWire would do, in a sentence or two.
    intro: String,
    /// What the user is taking on.
    points: Vec<String>,
    /// The question itself, without the `[Y/n]`.
    question: String,
}

/// Word the consent question for every pending login at once.
///
/// One question, because `docs/TRUST.md` §2 does not ask for the same risk to
/// be asked about twice. Several logins of one product share a vendor, a host
/// and a client, so each of those is said once — "Anthropic and Anthropic do
/// not document…" is the sentence this exists never to produce — while the
/// logins themselves are named by where they live, because with two Claude
/// logins "the Claude subscription" does not say which accounts are being
/// handed over. Logins in their usual places keep the wording people have
/// already answered.
fn consent_question(pending: &[Pending]) -> Question {
    let mut products: Vec<SubscriptionProduct> = Vec::new();
    for p in pending {
        if !products.contains(&p.login.product) {
            products.push(p.login.product);
        }
    }
    let names = join(products.iter().map(|p| p.name()), "and");
    let clients = join(products.iter().map(|p| p.client()), "and");
    let vendors = join(products.iter().map(|p| p.vendor()), "and");
    let hosts = join(products.iter().map(|p| p.host()), "and");
    let alternatives = join(products.iter().map(|p| alternative(*p)), "or");

    let (subject, object) = if pending.iter().all(|p| p.login.location.is_none()) {
        if pending.len() == 1 {
            (
                format!("your {names} subscription"),
                format!("the {names} subscription"),
            )
        } else {
            (
                format!("the {names} subscriptions above"),
                format!("the {names} subscriptions"),
            )
        }
    } else {
        let groups: Vec<String> = products
            .iter()
            .map(|product| {
                let places: Vec<&str> = pending
                    .iter()
                    .filter(|p| p.login.product == *product)
                    .map(|p| p.location.as_str())
                    .collect();
                let plural = if places.len() == 1 { "" } else { "s" };
                format!(
                    "the {} subscription{plural} in {}",
                    product.name(),
                    join(places.into_iter(), "and")
                )
            })
            .collect();
        let separator = if groups.iter().any(|g| g.contains(" and ")) {
            ", and "
        } else {
            " and "
        };
        let named = groups.join(separator);
        (named.clone(), named)
    };

    let intro = match (pending.len(), products.len()) {
        (1, _) => format!(
            "IronWire can use {subject}, by replaying the OAuth token {clients} has \
             already stored on this machine. It goes only to {hosts}, from this computer."
        ),
        (_, 1) => format!(
            "IronWire can use {subject}, by replaying the OAuth tokens {clients} has \
             already stored on this machine. They go only to {hosts}, from this computer."
        ),
        _ => format!(
            "IronWire can use {subject}, by replaying the OAuth tokens {clients} have \
             already stored on this machine. Each token goes only to its own provider \
             ({hosts}), from this computer."
        ),
    };
    Question {
        intro,
        points: vec![
            format!(
                "{vendors} {} not document this authentication path and may change or \
                 block it at any time.",
                if products.len() > 1 { "do" } else { "does" }
            ),
            "Using it from a third-party proxy may fall outside your subscription's \
             intended use. If they object, it is your account that is affected."
                .to_string(),
            format!("You can use {alternatives} instead — fully supported, no ambiguity."),
        ],
        question: format!("Use {object}?"),
    }
}

/// The unambiguous alternative to a product's subscription.
fn alternative(product: SubscriptionProduct) -> &'static str {
    match product {
        SubscriptionProduct::Claude => "an Anthropic API key",
        SubscriptionProduct::ChatGpt => "an OpenAI API key",
    }
}

/// Join names the way a sentence would: "a", "a and b", "a, b and c".
///
/// The prompt reads out loud to someone deciding whether to hand over a
/// credential, so it has to be a sentence rather than a comma-separated list.
/// `conjunction` because a list of things we would use and a list of things
/// they could use instead are not the same kind of list.
fn join<'a>(items: impl Iterator<Item = &'a str>, conjunction: &str) -> String {
    let items: Vec<_> = items.collect();
    match items.split_last() {
        None => String::new(),
        Some((last, [])) => (*last).to_string(),
        Some((last, rest)) => format!("{} {conjunction} {last}", rest.join(", ")),
    }
}

fn has_env(name: &str) -> bool {
    std::env::var(name).is_ok_and(|value| !value.trim().is_empty())
}

/// Read the keys the installed agents are already configured with, in the
/// default places and in every declared login's.
///
/// Detection only. Copying a secret out of one config and into another — or
/// into a service unit, which is where this would naturally lead — would put
/// IronWire in the business of storing keys, which it is not (`docs/TRUST.md`
/// §5).
fn detect_agent_keys(logins: &[Login]) -> Vec<AgentKey> {
    let mut keys = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for login in logins {
        let Some(path) = login.client_config() else {
            continue;
        };
        if seen.contains(&path) {
            continue;
        }
        keys.extend(match login.product {
            SubscriptionProduct::Claude => claude_settings_keys(&path),
            SubscriptionProduct::ChatGpt => codex_config_keys(&path),
        });
        seen.push(path);
    }
    keys.sort_by(|a, b| a.var.cmp(&b.var));
    keys.dedup_by(|a, b| a.var == b.var);
    keys
}

/// Variables named in Claude Code's `env` block.
fn claude_settings_keys(path: &std::path::Path) -> Vec<AgentKey> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(root) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    root.get("env")
        .and_then(serde_json::Value::as_object)
        .map(|env| {
            env.keys()
                .filter(|name| looks_like_a_key(name))
                .map(|name| AgentKey {
                    agent: "Claude Code",
                    var: name.clone(),
                    in_environment: has_env(name),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Variables named by `env_key` under Codex's `[model_providers]`.
fn codex_config_keys(path: &std::path::Path) -> Vec<AgentKey> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(table) = text.parse::<toml::Table>() else {
        return Vec::new();
    };
    table
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .map(|providers| {
            providers
                .values()
                .filter_map(|provider| provider.get("env_key")?.as_str())
                .map(|name| AgentKey {
                    agent: "Codex",
                    var: name.to_string(),
                    in_environment: has_env(name),
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Whether a variable name is plausibly a credential rather than a setting.
///
/// A conservative shape match, because the alternative is listing every
/// variable in someone's `env` block back at them as though it were capacity.
fn looks_like_a_key(name: &str) -> bool {
    let name = name.to_ascii_uppercase();
    name.ends_with("_API_KEY") || name.ends_with("_AUTH_TOKEN") || name.ends_with("_TOKEN")
}

/// Say what was found, and what each thing is worth.
fn report(found: &Found, local: &[(u16, &'static str)]) {
    let line = |present: bool, what: &str, note: &str| {
        if present {
            println!("  found    {what:<28}{note}");
        }
    };
    // One line per login, naming where it lives: with two Claude logins,
    // "Claude Code login" twice says nothing about which is which.
    for FoundLogin { login, usable } in &found.logins {
        let what = match login.product {
            SubscriptionProduct::Claude => "Claude Code login",
            SubscriptionProduct::ChatGpt => "ChatGPT login (Codex)",
        };
        if *usable {
            println!(
                "  found    {what:<28}{} · capacity you have already paid for",
                login.location()
            );
        } else {
            println!(
                "  missing  {what:<28}{} · not logged in — run `{}`",
                login.location(),
                login.login_command()
            );
        }
    }
    line(found.codex_api_key, "OpenAI key (via Codex)", "metered");
    line(found.anthropic_key, "ANTHROPIC_API_KEY", "metered");
    line(found.openai_key, "OPENAI_API_KEY", "metered");
    line(
        found.nearai_key,
        "NEARAI_API_KEY",
        "credits; cross-family fallback",
    );

    for (port, name) in local {
        println!(
            "  found    {:<28}free, private, and yours",
            format!("{name} on :{port}")
        );
    }

    if let Some(dir) = &found.unknown_claude_dir {
        // Claude Code keys its login to this directory, so it is a login of
        // its own — and one IronWire has not been told about. Wiring it as
        // the default would send that Claude Code's traffic out on the
        // default login's account.
        println!("  note     CLAUDE_CONFIG_DIR is {dir}, a login IronWire does not know yet;");
        println!(
            "           `ironwire connect claude` {}",
            logins::connect_would(dir)
        );
    }

    for key in &found.agent_keys {
        // The unreadable case is the one worth the words: it looks like
        // capacity, and it is not, until the variable is where the daemon runs.
        let note = if key.in_environment {
            format!("used by {}", key.agent)
        } else {
            format!("used by {} — not in this environment", key.agent)
        };
        println!("  found    {:<28}{note}", key.var);
    }

    if !found.anything() && local.is_empty() {
        println!("  nothing yet");
    }
}

/// Local model servers worth looking for, on the ports they ship with.
///
/// Reported, never written: `init` without `--write` changes nothing, and a
/// discovered endpoint is the user's decision to make (`docs/DESIGN.md`).
const LOCAL_SERVERS: &[(u16, &str)] = &[(11434, "Ollama"), (1234, "LM Studio"), (8000, "vLLM")];

/// Ask each loopback port whether an OpenAI-compatible server is there.
///
/// Short timeout and total silence when nothing answers: this runs on every
/// `init`, including on machines that will never run a local model, and a
/// second of latency or a line of "not found" noise would be a tax on everyone
/// for a feature most people do not use.
async fn find_local_servers() -> Vec<(u16, &'static str)> {
    let client = match reqwest::Client::builder()
        .timeout(std::time::Duration::from_millis(500))
        .build()
    {
        Ok(client) => client,
        Err(_) => return Vec::new(),
    };
    let probes = LOCAL_SERVERS.iter().map(|(port, name)| {
        let client = client.clone();
        async move {
            let url = format!("http://127.0.0.1:{port}/v1/models");
            match client.get(url).send().await {
                Ok(response) if response.status().is_success() => Some((*port, *name)),
                _ => None,
            }
        }
    });
    futures_util::future::join_all(probes)
        .await
        .into_iter()
        .flatten()
        .collect()
}

/// Run `ironwire init`.
pub(crate) async fn run(
    port: Option<u16>,
    write: bool,
    dry_run: bool,
    no_service: bool,
) -> Result<()> {
    let paths = paths()?;
    // A config the daemon would refuse still leaves everything else here worth
    // doing, so it is said and set aside rather than ending the run.
    let config = Config::load(&paths).unwrap_or_else(|error| {
        println!("{error}");
        println!();
        Config::default()
    });
    let port = port.unwrap_or(config.server.port);

    println!("IronWire — one local endpoint for the AI capacity you already have");
    println!();
    if dry_run {
        println!("[dry run] nothing on this machine will be changed.");
        println!();
    }
    println!("Looking at this machine…");
    println!();

    let found = Found::detect(&config);
    let local = find_local_servers().await;
    report(&found, &local);

    if !found.anything() && local.is_empty() {
        println!();
        println!("Nothing found yet. Any one of these is enough to start:");
        println!();
        println!("    claude                      log in to Claude Code");
        println!("    codex login                 log in to Codex");
        println!("    export ANTHROPIC_API_KEY=…  a metered key");
        println!("    export NEARAI_API_KEY=…     NEAR AI credits");
        println!();
        println!("Then run `ironwire init` again.");
        return Ok(());
    }

    if write {
        println!();
        if dry_run {
            println!("[dry run] would write {}", paths.config_file().display());
        } else {
            write_config(&paths, port)?;
        }
    }

    let enabled = ask_for_subscriptions(&paths, &found, dry_run)?;
    let wired = wire_agents(&config, &found, port, dry_run)?;
    let daemon = start_daemon(port, no_service, dry_run).await?;

    summarise(port, &found, &enabled, &wired, &daemon);
    Ok(())
}

/// Ask once, for everything found.
///
/// `docs/TRUST.md` §2 requires that the risk be stated in plain language and
/// the answer recorded against the wording it answered. It does not require
/// that the question be asked twice when two subscriptions carry the same risk
/// — and asking twice makes the second one feel like a formality, which is the
/// opposite of what a consent gate is for.
///
/// Answering no is a complete answer: the metered keys and local models found
/// above still work, which is what makes saying no cheap enough to mean
/// something.
fn ask_for_subscriptions(
    paths: &ironwire_core::config::PathsConfig,
    found: &Found,
    dry_run: bool,
) -> Result<Vec<String>> {
    let path = paths.consent_file();
    let mut ledger = ConsentLedger::load(&path);
    let pending = found.ungranted_subscriptions(&ledger);
    if pending.is_empty() {
        return Ok(Vec::new());
    }

    let question = consent_question(&pending);
    println!();
    for line in connect::wrap(&question.intro, 70) {
        println!("  {line}");
    }
    println!();
    for point in &question.points {
        let mut lines = connect::wrap(point, 68).into_iter();
        if let Some(first) = lines.next() {
            println!("  · {first}");
        }
        for rest in lines {
            println!("    {rest}");
        }
    }
    println!();

    if !ask(&format!("  {} [Y/n] ", question.question))? {
        println!();
        println!("  Left disabled. IronWire will use API keys and local models only.");
        let again: Vec<String> = pending
            .iter()
            .map(|p| format!("{} --subscription", p.login.connect_command()))
            .collect();
        match again.as_slice() {
            [one] => println!("  `{one}` asks again."),
            many => {
                println!("  Each of these asks again, one login at a time:");
                for command in many {
                    println!("      {command}");
                }
            }
        }
        return Ok(Vec::new());
    }

    let enabled: Vec<String> = pending.iter().map(|p| p.id.clone()).collect();
    if dry_run {
        println!(
            "  [dry run] would record consent for: {}",
            enabled.join(", ")
        );
        return Ok(enabled);
    }
    grant_all(&mut ledger, &pending, Utc::now());
    ledger.save(&path).context("writing the consent ledger")?;
    println!("  Recorded in {}.", path.display());
    Ok(enabled)
}

/// Record the one yes against every login it was given for, each by the
/// directory it is read from: the id alone does not say which account was
/// handed over.
fn grant_all(ledger: &mut ConsentLedger, pending: &[Pending], now: DateTime<Utc>) {
    for p in pending {
        ledger.grant_for(&p.id, p.login.consent_location(), now);
    }
}

/// Read a yes/no answer, defaulting to yes.
///
/// Not a terminal: no answer is coming, and blocking forever inside a script
/// would be worse than declining. The safe default when nobody is there to ask
/// is the one that grants nothing.
fn ask(question: &str) -> Result<bool> {
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        println!("{question}");
        println!("  (not a terminal — leaving subscriptions disabled)");
        return Ok(false);
    }
    print!("{question}");
    std::io::stdout().flush().ok();
    let mut answer = String::new();
    std::io::stdin()
        .read_line(&mut answer)
        .context("reading your answer")?;
    Ok(!matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "n" | "no"
    ))
}

/// What [`wire_agents`] pointed at IronWire.
struct Wired {
    /// Every client, named for the summary.
    labels: Vec<String>,
    /// The config files the built-in clients were wired in, which is how the
    /// summary tells whether the `claude` or `codex` this shell starts is one
    /// of them.
    files: Vec<PathBuf>,
}

/// Point every agent that is installed at IronWire — and each declared login's
/// client at the URL that names that login.
///
/// Only the ones that are installed: writing a settings file nothing will read
/// is how a setup step turns into litter in someone's home directory. A login
/// config.toml declares counts as installed, because declaring it is the user
/// saying they use it; a disabled one does not, because the URL that names it
/// would be refused.
fn wire_agents(config: &Config, found: &Found, port: u16, dry_run: bool) -> Result<Wired> {
    let declared = logins::configured(config);
    // A directory a login of its own claims is that login's, even when it is
    // also where the default login would be — the more specific answer wins.
    let claimed: Vec<PathBuf> = declared
        .iter()
        .filter(|login| !login.is_default() && login.enabled)
        .filter_map(Login::client_config)
        .collect();

    let mut wired = Wired {
        labels: Vec::new(),
        files: Vec::new(),
    };
    println!();
    for login in &declared {
        let wire = if !login.is_default() || login.config_dir.is_some() {
            login.enabled
        } else {
            let installed = match login.product {
                // `claude` on PATH is the Claude Code this shell starts, and
                // with an undeclared CLAUDE_CONFIG_DIR that one does not read
                // ~/.claude. Only a directory it has already made says the
                // default login's Claude Code runs here too.
                SubscriptionProduct::Claude if found.unknown_claude_dir.is_some() => {
                    claude_settings::default_dir().is_some_and(|dir| dir.exists())
                }
                SubscriptionProduct::Claude => connect::claude_installed(),
                SubscriptionProduct::ChatGpt => connect::codex_installed(),
            };
            installed
                && login
                    .client_config()
                    .is_none_or(|path| !claimed.contains(&path))
        };
        if wire {
            connect::warn_shared_settings(login);
            connect::wire_login(login, port, dry_run)?;
            connect::warn_left_behind(login);
            wired.labels.push(login.client_label());
            wired.files.extend(login.client_config());
        }
    }
    wired.labels.extend(wire_catalog_agents(port, dry_run));
    // With an undeclared CLAUDE_CONFIG_DIR there is an agent here, and the
    // summary names the command that wires it.
    if wired.labels.is_empty() && found.unknown_claude_dir.is_none() {
        println!("No coding agent found here yet. Once one is installed, run");
        println!("`ironwire init` again and it will be pointed at IronWire.");
    }
    Ok(wired)
}

/// Tools the signed catalog taught us about since the last release.
///
/// One tool failing is not the run failing. These arrive from a document rather
/// than from this binary, so a config shape we did not anticipate is a thing to
/// report and move past — the two built-in agents above, and the daemon itself,
/// have nothing to do with it.
fn wire_catalog_agents(port: u16, dry_run: bool) -> Vec<String> {
    let Ok(paths) = ironwire_core::config::PathsConfig::resolve() else {
        return Vec::new();
    };
    let store = ironwire_catalog::CatalogStore::load(
        ironwire_catalog::CATALOG_PUBLIC_KEY,
        &paths.catalog_file(),
    );
    let catalog = store.current();

    for (id, problem) in catalog.rejected_agents() {
        // Said out loud rather than dropped silently: a tool the document meant
        // to ship and we would not touch is exactly the thing worth knowing.
        tracing::warn!(agent = id, %problem, "catalog entry ignored");
    }

    let mut wired = Vec::new();
    for agent in catalog.agents() {
        if !connect::catalog_agent_installed(agent) {
            continue;
        }
        match connect::wire_catalog_agent(agent, port, dry_run) {
            Ok(()) => wired.push(agent.name.clone()),
            Err(error) => println!("  {} could not be wired: {error:#}", agent.name),
        }
    }
    wired
}

/// Where the daemon ended up.
enum Daemon {
    /// Running in the background, and it survives a reboot.
    Service,
    /// Already listening before we got here.
    AlreadyUp,
    /// Nothing to install to, or the user asked us not to.
    Foreground,
}

/// Leave the daemon running.
async fn start_daemon(port: u16, no_service: bool, dry_run: bool) -> Result<Daemon> {
    println!();
    if listening(port).await {
        println!("IronWire is already listening on 127.0.0.1:{port}.");
        return Ok(Daemon::AlreadyUp);
    }
    if dry_run {
        println!("[dry run] would install and start the background service.");
        return Ok(Daemon::Service);
    }
    if no_service {
        return Ok(Daemon::Foreground);
    }

    match service::install_and_start(Some(port))? {
        service::Outcome::Running(_) => {
            // Started is not the same as listening, and the difference is a
            // port already taken or a unit that dies on start. Waiting a moment
            // and looking is the only honest way to say "it is running".
            if wait_until_listening(port).await {
                Ok(Daemon::Service)
            } else {
                println!("The service started but nothing is listening on {port} yet.");
                println!("Check it with: ironwire service status");
                Ok(Daemon::Service)
            }
        }
        service::Outcome::Installed { retry } => {
            println!();
            println!("The unit is installed but did not start. Finish with:");
            println!("    {retry}");
            Ok(Daemon::Foreground)
        }
        service::Outcome::Unsupported => {
            println!("No user-scoped service manager here — common in containers and");
            println!("over a bare SSH session. Run it in the foreground instead.");
            Ok(Daemon::Foreground)
        }
    }
}

/// Whether anything is accepting connections on the loopback port.
async fn listening(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

/// Give a just-started daemon a moment to bind.
async fn wait_until_listening(port: u16) -> bool {
    for _ in 0..20 {
        if listening(port).await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    }
    false
}

/// Say what is true now, and the one command that comes next.
fn summarise(port: u16, found: &Found, enabled: &[String], wired: &Wired, daemon: &Daemon) {
    println!();
    match daemon {
        Daemon::Service | Daemon::AlreadyUp => {
            println!("Ready. IronWire is on http://127.0.0.1:{port}.");
        }
        Daemon::Foreground => {
            println!("Almost there. Start the daemon and leave it running:");
            println!();
            println!("    ironwire serve");
        }
    }
    println!();

    if !wired.labels.is_empty() {
        println!("  wired    {}", wired.labels.join(", "));
    }
    if !enabled.is_empty() {
        println!("  enabled  {}", enabled.join(", "));
    }

    // A key an agent uses that the daemon cannot see is the one failure this
    // command can predict, so it says so here rather than letting `doctor`
    // discover it later.
    for key in found.agent_keys.iter().filter(|k| !k.in_environment) {
        println!(
            "  note     {} is set for {}, but not where IronWire runs",
            key.var, key.agent
        );
    }

    println!();
    // Only once there is something to talk to. "Run `claude`" while the daemon
    // is down sends the user at a connection error, which is a worse first
    // impression than the extra step they still have to take.
    //
    // And only about the client this shell starts, as the login it is: with
    // a CLAUDE_CONFIG_DIR no login here uses, `claude` reads a settings file
    // nothing above touched — or, for `~/.claude` on macOS, the default's
    // file while logged in as itself. Either way connecting it comes next,
    // and "it goes through IronWire now" would say it is done.
    let starts_here = |path: Option<PathBuf>| path.is_some_and(|path| wired.files.contains(&path));
    if matches!(daemon, Daemon::Service | Daemon::AlreadyUp) {
        if found.unknown_claude_dir.is_none() && starts_here(claude_settings::path()) {
            println!("Start a new terminal and run `claude` — it goes through IronWire now.");
        } else if starts_here(codex_config::path()) {
            println!("Start a new `codex` session — it goes through IronWire now.");
        }
    }
    if let Some(dir) = &found.unknown_claude_dir {
        let shown = logins::shown(std::path::Path::new(dir));
        if logins::shares_plain_claudes_settings(dir) {
            println!(
                "Next: `ironwire connect claude`. The Claude Code this shell starts uses \
                 {shown} through CLAUDE_CONFIG_DIR, which on macOS is a login of its own — \
                 not the Keychain item plain `claude` uses — though both read \
                 {shown}/settings.json{}.",
                if starts_here(claude_settings::path()) {
                    "; until then IronWire serves it from plain `claude`'s login"
                } else {
                    ""
                }
            );
        } else {
            println!(
                "Next: `ironwire connect claude`. The Claude Code this shell starts uses \
                 {shown} (CLAUDE_CONFIG_DIR), a login IronWire does not know yet."
            );
        }
    }
    println!("    ironwire doctor    confirm it end to end, with a real request");
    println!("    ironwire status    what capacity you have, and what is left");
    println!("    ironwire watch     live routing, quiet unless something changes");
}

/// Write a commented `config.toml`, if there is not one already.
fn write_config(paths: &ironwire_core::config::PathsConfig, port: u16) -> Result<()> {
    let path = paths.config_file();
    if path.exists() {
        println!("{} already exists — leaving it alone.", path.display());
        return Ok(());
    }

    let template = config_template(port);
    std::fs::write(&path, template).with_context(|| format!("writing {}", path.display()))?;
    println!("Wrote {}", path.display());
    Ok(())
}

/// The commented default config.
///
/// Every value in it *is* a default. The file exists to make the settings
/// discoverable, not to change behaviour — a generated config that silently
/// differs from the built-in defaults is a trap, and the test below is what
/// keeps the two from drifting apart.
fn config_template(port: u16) -> String {
    format!(
        r#"# IronWire configuration. Every value below is the default; the file
# exists so the settings are discoverable, not to change anything.

[server]
port = {port}
# Idle timeout for a single upstream request. Long, because coding agents
# legitimately generate for many minutes.
upstream_timeout_secs = 900

[capture]
# Local trace ledger: what `ironwire log` reads. Metadata only.
enabled = true
# Request and response bodies, exactly as they crossed the wire, under
# $IRONWIRE_HOME/bodies. Off by default — these contain your source.
#
# Only the *current* exchange of each session is held: the next turn releases
# the previous one's bodies, so a finished session leaves exactly one — its
# final call, the only one anything downstream attests. The SHA-256 of both
# bodies stays on every row, because that is what a provider's own per-request
# receipt is a signature over, and a hash is not the content.
bodies = false
# Days of history to keep. Pruned daily by the daemon; 0 keeps everything,
# which is a real choice but not a good default for a file nobody watches.
retain_days = 90

[usage]
# The session section on `ironwire status`: burn rate, and where this window
# ends up at that rate. Measured from the ledger above — your own traffic,
# never a provider's quota, which is reported or `unknown` and never guessed.
enabled = true
# Length of a session window, in hours. Five is Claude Code's.
session_hours = 5
# How far back to look when calibrating against your own past windows.
history_hours = 192
# Your plan: pro, max5, max20, or team. Deliberately unset by default —
# per-window token limits are not published, and IronWire will not assert one
# on your behalf. Set it and the ceiling becomes your claim about your own
# subscription, labelled as such. Left unset, the comparison is against your
# own completed sessions, which needs no table at all.
# plan = "max5"

[resilience]
# Emit an SSE ping after this many seconds of upstream silence, so a client
# whose patience is shorter than the model's thinking time does not give up.
keepalive_secs = 15
# Give up on a silent upstream after this long and end the stream with a
# stated error rather than pinging into the void.
stall_timeout_secs = 180
# Compaction turns send the whole conversation and think for far longer.
compaction_stall_timeout_secs = 600
max_reconnects = 2

[updates]
# Check for a newer release, at most once a day. IronWire never applies one.
check = true

[privacy]
# Optional. Substitutes values on the way out and restores them on the way
# back, so a provider never sees them. Off by default: it is the one thing
# here that modifies your requests.
#
# See `ironwire privacy check <file>` to find out what it would actually
# catch before you rely on it, and docs/PRIVACY.md for what it cannot.
# off         — no substitution; requests are forwarded byte-identical.
# credentials — API keys, tokens, private keys, and any named_values below.
# pii         — credentials, plus emails, IP addresses and phone numbers.
#               Deterministic classes only: names need a classifier that has
#               to publish its precision and recall before it ships.
# full        — pii, and requests are routed only to the backends named in
#               trusted_backends below. Nothing else is tried: when none of
#               them can serve a request, IronWire refuses it rather than
#               falling back. There is no default set — which operators you
#               trust with your data is not IronWire's call to make.
mode = "off"
# Required when mode = "full"; ignored otherwise.
# trusted_backends = ["nearai", "ollama"]
# API keys, tokens and private keys, by shape.
secrets = true
# Exact strings to substitute — your employer, a customer's domain. Note that
# listing them here writes them to this file.
named_values = []
# Values inside fenced code blocks and tool results are left alone by default:
# in a coding session they are almost always the code being edited, and
# substituting them makes the model write something that does not work.
scan_code_blocks = false
scan_tool_results = false

# [limits]
# Spend caps, in dollars per day, measured over your local calendar day and
# only against *metered* backends — a subscription is already paid for, and
# capping it would cap capacity you bought. No cap unless you set one.
#
# daily_spend_usd = 10.0
# on_breach = "descend"   # keep working on free capacity (default), or
#                         # "refuse" to stop so you find out immediately
#
# [[limits.backends]]
# id = "anthropic-key"
# daily_spend_usd = 5.0
#
# A cap needs the trace ledger, which is where spend is measured; `ironwire
# serve` refuses to start with a cap set and capture off, rather than leaving
# you believing in a limit that can never fire.

# [subscriptions]
# Each login is its own backend, and a client is served by its own: the
# Claude Code on ~/.claude-work by that login's subscription and, under quota
# pressure or once it runs out, by the ordinary ladder below it — API key,
# NEAR AI, local — never by your other Claude subscription.
#
# Turning this on lets a client be served by another of your subscriptions
# for the same product as soon as its own is under quota pressure — the
# trigger every rung of the ladder uses, which outranks the preference for
# your own login and does not wait for it to run out. Pressure means
# at least 90% used as the provider reports it, or spent: a request choosing
# afresh moves at once, and a conversation already on your own login once
# that has lasted the 20-second descent debounce. Read docs/TRUST.md first.
# Moving to another account after a rate limit is the most direct reading of
# what OpenAI's terms ("circumvent any rate limits") and Anthropic's usage
# policy ("bypass ... restrictions") forbid; the provider can see it, because
# Claude Code's request names the account it is logged in as and would arrive
# on the other account's token; and a Codex conversation carrying encrypted
# reasoning never moves account even with this on, because OpenAI refuses
# reasoning minted by another one.
#
# cross_account_fallback = false

# Backends are discovered from your logins and environment, so this section is
# usually unnecessary. Declare one to override how a discovered backend is
# built, to switch one off, or to add an endpoint IronWire cannot discover.
# Commented out because an empty `[[backends]]` block is not the default —
# declaring nothing and declaring an entry are different states.
#
# [[backends]]
# id = "anthropic-key"          # matches a discovered backend: configures it
# kind = "anthropic-api"        # claude-subscription, anthropic-api,
#                               # codex-subscription, openai-api, nearai,
#                               # openai-compatible
# enabled = false               # a real kill switch: it is never registered
# api_key_env = "ANTHROPIC_API_KEY_WORK"   # which variable holds the key
#
# [[backends]]
# id = "claude-work"            # a second Claude login, as its own backend:
# kind = "claude-subscription"  # its own consent, quota and ledger rows
# config_dir = "~/.claude-work" # what you export as CLAUDE_CONFIG_DIR when you
#                               # use it (CODEX_HOME, for codex-subscription).
#                               # `ironwire connect claude --config-dir DIR`
#                               # writes this entry and points that Claude Code
#                               # at a URL naming it, so it is served by its own
#                               # subscription and nobody else's.
#
# [[backends]]
# id = "local"
# kind = "openai-compatible"    # a hosted endpoint speaking OpenAI chat
# base_url = "https://example.internal/v1" # required for this kind
# api_key_env = "LOCAL_API_KEY"            # required; no secrets in this file
#
# [[backends]]
# id = "ollama"                 # an id discovery does not produce: adds one
# kind = "local"                # a model on this machine: free, private, and
#                               # the cheapest capacity there is
# base_url = "http://127.0.0.1:11434/v1"   # required, and must include /v1 —
#                               # Ollama's native /api/* is a different protocol
# api_key_env = "LOCAL_API_KEY" # optional here; most local servers take no auth
# # A bare slug on a local backend counts as `fast`, whatever its name suggests,
# # because local capacity sorts cheapest and a model that reads as frontier-tier
# # would take work you asked a frontier model for. Say so explicitly to opt one
# # up the ladder:
# models = ["qwen3-coder:30b", {{ name = "llama3.3:70b", tier = "balanced" }}]
"#
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ironwire_core::DEFAULT_PORT;

    #[test]
    fn the_generated_config_parses_and_equals_the_built_in_defaults() {
        // The trap this guards against: a template that drifts from the
        // defaults turns "here is a file documenting what IronWire does" into
        // "here is a file that quietly changes what IronWire does".
        let dir = tempfile::tempdir().expect("tempdir");
        let paths = ironwire_core::config::PathsConfig::rooted_at(dir.path());
        std::fs::write(paths.config_file(), config_template(DEFAULT_PORT)).expect("write");

        let mut loaded = Config::load(&paths).expect("the generated config parses");
        // The template states `mode = "off"` where the struct default leaves it
        // unset. That is the one intentional difference: the file exists to
        // *document* the ladder, and a commented-out mode teaches nothing. It
        // must still mean off, which is what this checks before comparing the
        // rest field by field.
        assert_eq!(
            loaded.privacy.mode,
            Some(ironwire_core::config::PrivacyMode::Off)
        );
        assert_eq!(loaded.privacy.mode(), Config::default().privacy.mode());
        loaded.privacy.mode = None;
        assert_eq!(loaded, Config::default());
    }

    #[test]
    fn the_generated_config_carries_the_port_it_was_given() {
        let template = config_template(9999);
        assert!(template.contains("port = 9999"));
    }

    /// A block of the template, uncommented, from `first` to the next line
    /// that is only `#`.
    fn uncommented(template: &str, first: &str) -> String {
        template
            .lines()
            .skip_while(|line| *line != first)
            .take_while(|line| line.trim() != "#" && !line.trim().is_empty())
            .map(|line| line.strip_prefix("# ").unwrap_or(line))
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// The two settings people with a second login need are discoverable —
    /// and still only documentation: neither changes anything until it is
    /// uncommented, and the risky one is described as such.
    #[test]
    fn the_generated_config_documents_several_logins_without_turning_any_on() {
        let template = config_template(DEFAULT_PORT);
        assert!(template.contains("# cross_account_fallback = false"));
        assert!(template.contains("docs/TRUST.md"));
        // The move happens on the trigger every rung uses, and the comment
        // has to say so rather than promise it waits for a spent login.
        assert!(
            template.contains("# at least 90% used as the provider reports it"),
            "{template}"
        );
        assert!(template.contains("# afresh moves at once"), "{template}");
        assert!(!template.contains("whose own subscription is spent"));
        let parsed: Config = toml::from_str(&template).expect("parses");
        assert!(!parsed.subscriptions.cross_account_fallback);
        assert!(parsed.backends.is_empty());

        // The example is one the daemon would accept, as written.
        let start = template
            .find("# id = \"claude-work\"")
            .expect("the example is there");
        let header = template[..start]
            .rfind("# [[backends]]")
            .expect("under a header");
        let example = uncommented(&template[header..], "# [[backends]]");
        let declared: Config = toml::from_str(&example).expect("the example parses");
        assert_eq!(declared.backends.len(), 1, "{example}");
        assert_eq!(
            declared.backends[0].config_dir.as_deref(),
            Some("~/.claude-work")
        );
        declared
            .validate(std::path::Path::new("config.toml"))
            .expect("the example is a valid login");
    }

    fn pending(id: &str, product: SubscriptionProduct, location: Option<&str>) -> Pending {
        let usual = match product {
            SubscriptionProduct::Claude => "~/.claude",
            SubscriptionProduct::ChatGpt => "~/.codex",
        };
        Pending {
            id: id.to_string(),
            login: SubscriptionLogin {
                product,
                location: location.map(str::to_string),
                config_dir: location.map(str::to_string),
                source_dir: None,
                default: location.is_none(),
            },
            location: location.unwrap_or(usual).to_string(),
        }
    }

    /// Nobody who already answered the one-login question should find it
    /// reworded under them.
    #[test]
    fn one_default_login_is_asked_about_in_the_words_people_have_answered() {
        let question =
            consent_question(&[pending("claude-sub", SubscriptionProduct::Claude, None)]);
        assert_eq!(
            question.intro,
            "IronWire can use your Claude subscription, by replaying the OAuth token \
             Claude Code has already stored on this machine. It goes only to \
             api.anthropic.com, from this computer."
        );
        assert_eq!(
            question.points[0],
            "Anthropic does not document this authentication path and may change or \
             block it at any time."
        );
        assert_eq!(
            question.points[2],
            "You can use an Anthropic API key instead — fully supported, no ambiguity."
        );
        assert_eq!(question.question, "Use the Claude subscription?");

        let both = consent_question(&[
            pending("claude-sub", SubscriptionProduct::Claude, None),
            pending("codex-sub", SubscriptionProduct::ChatGpt, None),
        ]);
        assert_eq!(
            both.intro,
            "IronWire can use the Claude and ChatGPT subscriptions above, by replaying the \
             OAuth tokens Claude Code and Codex have already stored on this machine. Each \
             token goes only to its own provider (api.anthropic.com and chatgpt.com), from \
             this computer."
        );
        assert!(both.points[0].starts_with("Anthropic and OpenAI do not"));
        assert_eq!(both.question, "Use the Claude and ChatGPT subscriptions?");
    }

    /// The case this was rewritten for. Two Claude logins share a vendor, a
    /// host and a client, so each is said once — and the logins themselves are
    /// named by where they live, because "the Claude subscription" no longer
    /// says which accounts are being handed over.
    #[test]
    fn the_question_for_two_claude_logins_names_each_and_repeats_nothing() {
        let question = consent_question(&[
            pending("claude-sub", SubscriptionProduct::Claude, None),
            pending(
                "claude-work",
                SubscriptionProduct::Claude,
                Some("~/.claude-work"),
            ),
        ]);
        assert_eq!(
            question.intro,
            "IronWire can use the Claude subscriptions in ~/.claude and ~/.claude-work, by \
             replaying the OAuth tokens Claude Code has already stored on this machine. \
             They go only to api.anthropic.com, from this computer."
        );
        assert_eq!(
            question.question,
            "Use the Claude subscriptions in ~/.claude and ~/.claude-work?"
        );
        let all = format!("{} {}", question.intro, question.points.join(" "));
        for repeated in [
            "Anthropic and Anthropic",
            "Claude Code and Claude Code",
            "api.anthropic.com and api.anthropic.com",
            "API key or an Anthropic",
        ] {
            assert!(!all.contains(repeated), "{repeated:?} in: {all}");
        }
        assert!(question.points[0].starts_with("Anthropic does not document"));
    }

    #[test]
    fn logins_of_both_products_are_named_product_by_product() {
        let question = consent_question(&[
            pending("claude-sub", SubscriptionProduct::Claude, None),
            pending(
                "claude-work",
                SubscriptionProduct::Claude,
                Some("~/.claude-work"),
            ),
            pending(
                "codex-work",
                SubscriptionProduct::ChatGpt,
                Some("~/.codex-work"),
            ),
        ]);
        assert_eq!(
            question.question,
            "Use the Claude subscriptions in ~/.claude and ~/.claude-work, and the ChatGPT \
             subscription in ~/.codex-work?"
        );
        assert!(question.intro.contains("Claude Code and Codex have"));
        assert!(
            question
                .intro
                .contains("(api.anthropic.com and chatgpt.com)")
        );
        assert_eq!(
            question.points[2],
            "You can use an Anthropic API key or an OpenAI API key instead — fully \
             supported, no ambiguity."
        );

        // One additional login alone is still named, even with nothing to tell
        // it apart from on this screen: the prompt has to say which account.
        let alone = consent_question(&[pending(
            "codex-work",
            SubscriptionProduct::ChatGpt,
            Some("~/.codex-work"),
        )]);
        assert_eq!(
            alone.question,
            "Use the ChatGPT subscription in ~/.codex-work?"
        );
        assert!(alone.intro.contains("the OAuth token Codex has"));
    }

    /// Consent answers for the login it was given to. A `claude-sub` moved
    /// since it was consented to is asked about again, and the yes is
    /// recorded against the directory it reads now.
    #[test]
    fn a_moved_login_is_asked_about_again_and_recorded_by_its_directory() {
        let now = DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp");
        let config: Config = toml::from_str(
            "[[backends]]\nid = \"claude-sub\"\nkind = \"claude-subscription\"\n\
             config_dir = \"~/.claude-main\"\n",
        )
        .expect("parses");
        let moved = Login::default_for(SubscriptionProduct::Claude, &config);
        let found = Found {
            logins: vec![FoundLogin {
                login: moved.clone(),
                usable: true,
            }],
            codex_api_key: false,
            anthropic_key: false,
            openai_key: false,
            nearai_key: false,
            agent_keys: Vec::new(),
            unknown_claude_dir: None,
        };

        // Given while `claude-sub` read the login in ~/.claude.
        let mut ledger = ConsentLedger::default();
        ledger.grant("claude-sub", now);
        let pending = found.ungranted_subscriptions(&ledger);
        let ids: Vec<&str> = pending.iter().map(|p| p.id.as_str()).collect();
        assert_eq!(ids, ["claude-sub"]);

        grant_all(&mut ledger, &pending, now);
        let described = moved.describe();
        let location = described.consent_location();
        assert!(
            location.is_some_and(|l| l.ends_with("/.claude-main")),
            "{location:?}"
        );
        assert!(ledger.is_granted_for("claude-sub", location));
        assert!(!ledger.is_granted("claude-sub"));
        assert!(found.ungranted_subscriptions(&ledger).is_empty());
    }

    #[test]
    fn the_privacy_filter_is_off_in_the_generated_config() {
        // `docs/TRUST.md` I7: it modifies requests, so it is never on unless
        // the user turned it on — including in a file we wrote for them.
        let template = config_template(DEFAULT_PORT);
        let start = template.find("[privacy]").expect("has a privacy section");
        assert!(
            template[start..].contains("enabled = false"),
            "the generated config would have enabled the filter"
        );
    }
}
