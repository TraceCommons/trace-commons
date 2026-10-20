//! `ironwire doctor` — verify every connection end to end.
//!
//! This command makes **real network calls**. A config that parses and a
//! credential file that exists prove nothing; the failures that actually bite
//! — an expired token, a beta flag the provider stopped honouring, an account
//! not entitled to a model — only appear on the wire.

use anyhow::Result;
use ironwire_agents::{claude_settings, codex_config, endpoint};
use ironwire_core::config::Config;
use ironwire_creds::SubscriptionProduct;

use super::control_client::ControlClient;
use super::logins::{self, Login};

/// Check the daemon, the clients pointed at it, and each backend.
pub(crate) async fn run(port: Option<u16>) -> Result<()> {
    let client = ControlClient::new(port)?;
    let status = client.status().await?;
    println!("daemon        ok — 127.0.0.1:{}", status.port);

    // What the daemon was started with, near enough: the logins it knows and
    // the privacy settings below. A config that will not load is the daemon's
    // problem to report, so this falls back rather than failing the check.
    let config = super::paths()
        .ok()
        .and_then(|paths| Config::load(&paths).ok())
        .unwrap_or_default();
    let declared = logins::configured(&config);

    // The clients *before* the backends, because this is the failure people
    // actually hit: IronWire running perfectly, and the agent still going
    // straight to the provider because nothing points it here. Every backend
    // below can be green while the user sees no effect whatsoever.
    check_clients(status.port, &declared);

    if status.backends.is_empty() {
        println!("backends      none configured");
        println!();
        println!("Run `ironwire connect claude --subscription`, or set ANTHROPIC_API_KEY");
        println!("and restart the daemon.");
        return Ok(());
    }

    // Wide enough for a login's id, which is longer than the defaults'.
    let width = status
        .backends
        .iter()
        .map(|b| b.id.chars().count() + 2)
        .max()
        .unwrap_or(0)
        .max(14);

    // Static checks first: a backend awaiting consent must not be probed, since
    // probing it would use the very credential the user has not authorised.
    let mut probeable = false;
    for backend in &status.backends {
        if !backend.authenticated {
            let why = backend.detail.as_deref().unwrap_or("no credential found");
            println!("{:<width$}not connected — {why}", backend.id);
        } else if !backend.consented {
            println!(
                "{:<width$}awaiting consent — `{}`",
                backend.id,
                crate::render::consent_command(backend)
            );
        } else {
            probeable = true;
        }
    }

    if !probeable {
        println!();
        println!("Nothing to probe: no backend is both authenticated and enabled.");
        return Ok(());
    }

    println!();
    // Said before any probe: under `full` a healthy backend that is not
    // trusted is not an answer, and a user staring at green probe lines
    // should not have to work that out.
    let privacy = config.privacy;
    if privacy.mode() == ironwire_core::config::PrivacyMode::Full {
        let registered: Vec<&str> = status
            .backends
            .iter()
            .filter(|b| b.authenticated && privacy.trusted_backends.contains(&b.id))
            .map(|b| b.id.as_str())
            .collect();
        if registered.is_empty() {
            println!(
                "privacy      mode is `full`, and none of the trusted backends ({}) \n\
                 \x20            is connected — every request will be refused",
                privacy.trusted_backends.join(", ")
            );
        } else {
            println!(
                "privacy      mode is `full` — routing restricted to {}",
                registered.join(", ")
            );
        }
        println!();
    }
    // The one line that makes a shortened model list falsifiable. The ChatGPT
    // backend gates newer models on the reported client version, and a stale
    // one returns fewer models rather than an error — so without this a user
    // seeing fewer models than Codex has nothing at all to look at. Any
    // ChatGPT login, not only the default one: they all report it.
    let codex_logins: Vec<&str> = declared
        .iter()
        .filter(|login| login.product == SubscriptionProduct::ChatGpt)
        .map(|login| login.id.as_str())
        .collect();
    if status
        .backends
        .iter()
        .any(|b| codex_logins.contains(&b.id.as_str()))
    {
        let (version, source) = ironwire_upstream::codex_version::detect_with_source().await;
        println!("codex version {version} — {}", source.describe());
        println!();
    }

    println!("Probing backends…");
    let mut failures = 0;
    for probe in client.probe().await? {
        // The daemon sends nothing for a subscription awaiting consent. That is
        // not a failure, and the lines above already said why — unless consent
        // was withdrawn between the two requests.
        if !probe.probed {
            let said = status
                .backends
                .iter()
                .any(|b| b.id == probe.id && !(b.authenticated && b.consented));
            if !said {
                let detail = probe.error.as_deref().unwrap_or("awaiting consent");
                println!("{:<width$}{detail}", probe.id);
            }
            continue;
        }
        if probe.ok {
            println!("{:<width$}ok — {} ms", probe.id, probe.latency_ms);
        } else {
            failures += 1;
            let detail = probe.error.as_deref().unwrap_or("unknown failure");
            println!("{:<width$}FAILED — {detail}", probe.id);
        }
    }

    println!();
    if failures == 0 {
        println!("All connected backends answered.");
    } else {
        println!("{failures} backend(s) failed. `ironwire status` has the details.");
    }
    Ok(())
}

/// Whether the coding agents on this machine are actually pointed at us —
/// each login's client in its own file, and at the URL naming that login.
fn check_clients(port: u16, declared: &[Login]) {
    // A directory a login of its own claims is checked as that login's.
    let claimed: Vec<std::path::PathBuf> = declared
        .iter()
        .filter(|login| !login.is_default() && login.enabled)
        .filter_map(Login::client_config)
        .collect();
    let checked = declared.iter().filter(|login| {
        if login.is_default() {
            login
                .client_config()
                .is_none_or(|path| !claimed.contains(&path))
        } else {
            login.enabled
        }
    });

    let shell = logins::undeclared_claude_dir(declared);
    let mut shell_said = false;
    for login in checked
        .clone()
        .filter(|l| l.product == SubscriptionProduct::Claude)
    {
        if login.owns_the_plain_url() {
            shell_said |= check_default_claude(login, port, shell.as_deref());
        } else {
            check_login(login, port, declared);
        }
    }
    report_left_behind(declared, SubscriptionProduct::Claude, port);
    // Claude Code keys its login to this directory, so a Claude Code started
    // in this shell is a login IronWire has not been told about, and its
    // traffic would go out on another account.
    if let Some(dir) = shell.filter(|_| !shell_said) {
        println!("claude code   CLAUDE_CONFIG_DIR is {dir}, a login IronWire does not know");
        println!("              fix:  ironwire connect claude");
    }

    for login in checked.filter(|l| l.product == SubscriptionProduct::ChatGpt) {
        if login.owns_the_plain_url() {
            check_default_codex(login, port);
        } else {
            check_login(login, port, declared);
        }
    }
    report_left_behind(declared, SubscriptionProduct::ChatGpt, port);
    println!();
}

/// The client a moved default login left behind, when it is still pointed
/// here.
fn report_left_behind(declared: &[Login], product: SubscriptionProduct, port: u16) {
    for login in declared.iter().filter(|l| l.product == product) {
        if let Some(path) = login.left_behind()
            && let Some(lines) = left_behind_finding(login, &path, port)
        {
            for line in lines {
                println!("{line}");
            }
        }
    }
}

/// What to say about the file a moved default login was wired in before it
/// moved, if anything.
///
/// The client still started without a directory of its own reads it. On the
/// plain URL that client gets no subscription, because the plain façade
/// belongs only to a default login in its usual place; on the URL naming the
/// moved login it would be served by an account that is not its own. Either
/// way the fix is to take it off IronWire.
fn left_behind_finding(login: &Login, path: &std::path::Path, port: u16) -> Option<[String; 2]> {
    let (label, target, state) = match login.product {
        SubscriptionProduct::Claude => ("claude code", "claude", claude_state_at(path, port)),
        SubscriptionProduct::ChatGpt => ("codex", "codex", codex_state_at(path, port)),
    };
    let shown = logins::shown(path);
    let id = login.id.as_str();
    let location = login.location();
    let problem = match state {
        ClientState::PointedHere { account: None } => format!(
            "{label:<14}{shown} points here, and no subscription serves the plain URL \
             since `{id}` moved to {location}"
        ),
        ClientState::PointedHere {
            account: Some(account),
        } if account == id => format!(
            "{label:<14}{shown} points here as `{id}`, which now reads {location} — \
             another login's subscription"
        ),
        _ => return None,
    };
    let dir = path.parent().map(logins::shown).unwrap_or_default();
    Some([
        problem,
        format!("              fix:  ironwire disconnect {target} --config-dir {dir}"),
    ])
}

/// The default Claude login's client, which can also be pointed here from the
/// shell.
///
/// `shell` is this shell's `CLAUDE_CONFIG_DIR` when it names a login nobody
/// declared. Returns whether that was said here, so it is not said twice.
fn check_default_claude(login: &Login, port: u16, shell: Option<&str>) -> bool {
    let here = |value: &str| endpoint::parse_anthropic(value).filter(|e| e.port == port);

    // Two places can point Claude Code here, and `doctor` has to look at both.
    // The settings file is where `ironwire init` writes it, and it is *not* in
    // this process's environment — checking only the variable would report a
    // correctly configured machine as broken, which is the exact failure this
    // command exists to catch.
    let path = login.client_config();
    let shown = path.as_deref().map_or_else(String::new, logins::shown);
    let settings = path
        .as_deref()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|text| claude_settings::base_url(&text));
    let variable = std::env::var("ANTHROPIC_BASE_URL").ok();

    match (settings.as_deref(), variable.as_deref()) {
        (Some(s), _) if let Some(ours) = here(s) => {
            match ours.account {
                Some(account) => println!("claude code   pointed here as `{account}` ({shown})"),
                None => println!("claude code   pointed here ({shown})"),
            }
            // A shell variable disagreeing with the file is worth saying out
            // loud: which one wins is Claude Code's business, not ours, and a
            // machine where the two disagree will behave differently depending
            // on how the agent was started.
            if let Some(v) = variable.as_deref().filter(|v| here(v).is_none()) {
                println!("              note: ANTHROPIC_BASE_URL is also set, to {v}");
                println!(
                    "              unset it so there is one answer:  unset ANTHROPIC_BASE_URL"
                );
            }
        }
        (_, Some(v)) if here(v).is_some() => {
            println!("claude code   pointed here (ANTHROPIC_BASE_URL)")
        }
        (Some(s), _) => {
            println!("claude code   {shown} points at {s} — bypassing IronWire");
            println!("              fix:  {}", login.describe().connect_command());
        }
        // The commonest near-miss: an old port, or a `--port` override that a
        // shell profile never learned about.
        (None, Some(v)) => {
            println!("claude code   pointed at {v} — bypassing IronWire");
            println!("              fix:  unset ANTHROPIC_BASE_URL && ironwire connect claude");
        }
        // With no ~/.claude, the default login's Claude Code has not run
        // here, and `ironwire init` leaves its file alone while this shell
        // starts another one — so init is no fix. Connecting the login this
        // shell's Claude Code uses is.
        (None, None)
            if let Some(dir) = shell
                && !claude_settings::default_dir().is_some_and(|dir| dir.exists()) =>
        {
            println!("claude code   {shown} does not point here, and the Claude Code this shell");
            println!(
                "              starts reads {} instead (CLAUDE_CONFIG_DIR), a login IronWire \
                 does not know",
                logins::shown(std::path::Path::new(dir))
            );
            println!("              fix:  ironwire connect claude");
            return true;
        }
        (None, None) => {
            println!("claude code   {shown} does not point here");
            println!("              fix:  ironwire init");
        }
    }
    false
}

fn check_default_codex(login: &Login, port: u16) {
    let Some(path) = login.client_config() else {
        println!("codex         could not read its config — no home directory");
        return;
    };
    match codex_state_at(&path, port) {
        ClientState::PointedHere { account: None } => {
            println!("codex         pointed here ({})", logins::shown(&path));
        }
        ClientState::PointedHere {
            account: Some(account),
        } => println!(
            "codex         pointed here as `{account}` ({})",
            logins::shown(&path)
        ),
        ClientState::PointedElsewhere(provider) => {
            println!("codex         using provider `{provider}` — bypassing IronWire");
            println!("              fix:  ironwire connect codex");
        }
        ClientState::NoConfig => {
            // Not a problem: plenty of people do not use Codex. Said once,
            // quietly, rather than as a warning that needs acting on.
            println!("codex         no config found (not using Codex?)");
        }
        ClientState::Unreadable(why) => {
            println!("codex         could not read its config — {why}");
        }
    }
}

/// The client of an additional login, or of a moved default: pointed here is
/// not enough, it has to be pointed here *as that login*, or another login's
/// subscription pays — or, once the default has moved, none does.
fn check_login(login: &Login, port: u16, declared: &[Login]) {
    let label = match login.product {
        SubscriptionProduct::Claude => "claude code",
        SubscriptionProduct::ChatGpt => "codex",
    };
    let fix = login.describe().connect_command();
    let Some(path) = login.client_config() else {
        println!(
            "{label:<14}could not locate {} — no home directory",
            login.location()
        );
        return;
    };
    let shown = logins::shown(&path);
    let id = login.id.as_str();

    let state = match login.product {
        SubscriptionProduct::Claude => claude_state_at(&path, port),
        SubscriptionProduct::ChatGpt => codex_state_at(&path, port),
    };
    match state {
        ClientState::PointedHere { account } if account.as_deref() == Some(id) => {
            println!("{label:<14}pointed here as `{id}` ({shown})");
        }
        ClientState::PointedHere { account: None } => {
            let default_in_place = declared
                .iter()
                .any(|l| l.product == login.product && l.owns_the_plain_url());
            if default_in_place {
                println!(
                    "{label:<14}{shown} points here without naming `{id}` — the default \
                     login's subscription pays for it"
                );
            } else {
                println!(
                    "{label:<14}{shown} points here without naming `{id}` — no \
                     subscription serves the plain URL while your default login is moved"
                );
            }
            println!("              fix:  {fix}");
        }
        ClientState::PointedHere {
            account: Some(other),
        } => {
            println!("{label:<14}{shown} points here as `{other}`, not as `{id}`");
            println!("              fix:  {fix}");
        }
        ClientState::PointedElsewhere(elsewhere) => {
            println!("{label:<14}{shown} points at {elsewhere} — bypassing IronWire");
            println!("              fix:  {fix}");
        }
        ClientState::NoConfig => {
            println!("{label:<14}{shown} does not point here");
            println!("              fix:  {fix}");
        }
        ClientState::Unreadable(why) => {
            println!("{label:<14}could not read {shown} — {why}");
        }
    }
}

/// Where a client's config sends it. Both clients reduce to the same
/// question: which of our URLs, if any.
enum ClientState {
    /// Pointed at this daemon, as the login the URL names (`None`: the plain
    /// façade, the default login).
    PointedHere {
        account: Option<String>,
    },
    PointedElsewhere(String),
    NoConfig,
    Unreadable(String),
}

/// What a Claude Code settings file says, read the same way as
/// [`codex_state_at`].
fn claude_state_at(path: &std::path::Path, port: u16) -> ClientState {
    let Ok(text) = std::fs::read_to_string(path) else {
        return ClientState::NoConfig;
    };
    match claude_settings::base_url(&text) {
        None => ClientState::NoConfig,
        Some(url) => match endpoint::parse_anthropic(&url) {
            Some(ours) if ours.port == port => ClientState::PointedHere {
                account: ours.account,
            },
            _ => ClientState::PointedElsewhere(url),
        },
    }
}

/// The path is a parameter rather than a global read, so this is testable
/// without mutating process environment — which `unsafe_code = "forbid"` rules
/// out anyway, and which would make these tests order-dependent.
fn codex_state_at(path: &std::path::Path, port: u16) -> ClientState {
    let Ok(text) = std::fs::read_to_string(path) else {
        return ClientState::NoConfig;
    };
    let parsed: toml::Table = match text.parse() {
        Ok(parsed) => parsed,
        Err(error) => return ClientState::Unreadable(error.to_string()),
    };

    let provider = parsed
        .get("model_provider")
        .and_then(toml::Value::as_str)
        .unwrap_or("openai");
    if provider != "ironwire" {
        return ClientState::PointedElsewhere(provider.to_string());
    }

    // Pointed at *an* IronWire — check it is this one. A stale port here is a
    // silent bypass, and the symptom is identical to the daemon being down.
    let base = codex_config::base_url(&text).unwrap_or_default();
    match endpoint::parse_openai(&base) {
        Some(ours) if ours.port == port => ClientState::PointedHere {
            account: ours.account,
        },
        _ => ClientState::PointedElsewhere(format!("ironwire at {base}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn codex_config(contents: &str) -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let mut file = std::fs::File::create(&path).expect("create");
        file.write_all(contents.as_bytes()).expect("write");
        (dir, path)
    }

    #[test]
    fn a_codex_pointed_at_this_daemon_is_recognised() {
        let (_dir, path) = codex_config(
            "model_provider = \"ironwire\"\n\n\
             [model_providers.ironwire]\n\
             base_url = \"http://127.0.0.1:8463/openai/v1\"\n",
        );
        assert!(matches!(
            codex_state_at(&path, 8463),
            ClientState::PointedHere { account: None }
        ));
    }

    #[test]
    fn a_codex_pointed_at_a_different_port_is_a_silent_bypass() {
        // The symptom is identical to the daemon being down, which is why this
        // is worth naming rather than reporting as "pointed here".
        let (_dir, path) = codex_config(
            "model_provider = \"ironwire\"\n\n\
             [model_providers.ironwire]\n\
             base_url = \"http://127.0.0.1:9999/openai/v1\"\n",
        );
        assert!(matches!(
            codex_state_at(&path, 8463),
            ClientState::PointedElsewhere(_)
        ));
    }

    #[test]
    fn a_codex_using_its_own_provider_is_reported_as_bypassing_us() {
        let (_dir, path) = codex_config("model = \"gpt-5.6\"\n");
        match codex_state_at(&path, 8463) {
            ClientState::PointedElsewhere(provider) => assert_eq!(provider, "openai"),
            _ => panic!("a Codex not configured for IronWire must be reported as bypassing"),
        }
    }

    /// A login's client is pointed here only when it is pointed here *as
    /// that login*; doctor has to be able to tell the plain URL, the login's
    /// URL and another login's apart.
    #[test]
    fn a_codex_bound_to_a_login_is_recognised_as_that_login() {
        let (_dir, path) = codex_config(
            "model_provider = \"ironwire\"\n\n\
             [model_providers.ironwire]\n\
             base_url = \"http://127.0.0.1:8463/openai/account/codex-work/v1\"\n",
        );
        match codex_state_at(&path, 8463) {
            ClientState::PointedHere { account } => {
                assert_eq!(account.as_deref(), Some("codex-work"));
            }
            _ => panic!("a login-bound Codex is pointed here"),
        }
        assert!(matches!(
            codex_state_at(&path, 9999),
            ClientState::PointedElsewhere(_)
        ));
    }

    #[test]
    fn a_claude_code_bound_to_a_login_is_recognised_as_that_login() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("settings.json");
        std::fs::write(
            &path,
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic/account/claude-work"}}"#,
        )
        .expect("write");
        match claude_state_at(&path, 8463) {
            ClientState::PointedHere { account } => {
                assert_eq!(account.as_deref(), Some("claude-work"));
            }
            _ => panic!("a login-bound Claude Code is pointed here"),
        }

        std::fs::write(
            &path,
            r#"{"env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8463/anthropic"}}"#,
        )
        .expect("write");
        assert!(matches!(
            claude_state_at(&path, 8463),
            ClientState::PointedHere { account: None }
        ));

        std::fs::write(
            &path,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://proxy.corp.internal"}}"#,
        )
        .expect("write");
        assert!(matches!(
            claude_state_at(&path, 8463),
            ClientState::PointedElsewhere(_)
        ));
        assert!(matches!(
            claude_state_at(&dir.path().join("absent.json"), 8463),
            ClientState::NoConfig
        ));
    }

    /// A default login moved with `config_dir` leaves the client it moved away
    /// from wired as it was. On the plain URL that client gets no
    /// subscription; on the URL naming the moved login it gets another
    /// account's. Doctor names the file and the command that takes it off.
    #[test]
    fn a_client_left_on_ironwire_by_a_moved_default_login_is_flagged() {
        let home = tempfile::tempdir().expect("tempdir");
        let claude_dir = home.path().join(".claude");
        std::fs::create_dir_all(&claude_dir).expect("mkdir");
        let path = claude_dir.join("settings.json");
        let moved = Login {
            id: "claude-sub".to_string(),
            product: SubscriptionProduct::Claude,
            config_dir: Some("~/.claude-main".to_string()),
            enabled: true,
        };
        let pointed_at = |url: &str| {
            std::fs::write(
                &path,
                format!(r#"{{"env":{{"ANTHROPIC_BASE_URL":"{url}"}}}}"#),
            )
            .expect("write");
            left_behind_finding(&moved, &path, 8463)
        };
        let undo = format!(
            "fix:  ironwire disconnect claude --config-dir {}",
            logins::shown(&claude_dir)
        );

        let [problem, fix] = pointed_at("http://127.0.0.1:8463/anthropic").expect("plain URL");
        assert!(
            problem.contains("no subscription serves the plain URL"),
            "{problem}"
        );
        assert!(problem.contains("~/.claude-main"), "{problem}");
        assert!(fix.ends_with(&undo), "{fix}");

        let [problem, fix] = pointed_at("http://127.0.0.1:8463/anthropic/account/claude-sub")
            .expect("bound to the moved login");
        assert!(
            problem.contains("another login's subscription"),
            "{problem}"
        );
        assert!(fix.ends_with(&undo), "{fix}");

        // Anything else is not this mistake: bound to a login of its own, or
        // not on IronWire at all.
        assert!(pointed_at("http://127.0.0.1:8463/anthropic/account/claude-work").is_none());
        assert!(pointed_at("https://proxy.corp.internal").is_none());

        let codex_home = home.path().join(".codex");
        std::fs::create_dir_all(&codex_home).expect("mkdir");
        let codex = codex_home.join("config.toml");
        std::fs::write(
            &codex,
            "model_provider = \"ironwire\"\n\n[model_providers.ironwire]\n\
             base_url = \"http://127.0.0.1:8463/openai/v1\"\n",
        )
        .expect("write");
        let moved_codex = Login {
            id: "codex-sub".to_string(),
            product: SubscriptionProduct::ChatGpt,
            config_dir: Some("~/.codex-main".to_string()),
            enabled: true,
        };
        let [_, fix] = left_behind_finding(&moved_codex, &codex, 8463).expect("plain URL");
        assert!(
            fix.contains("ironwire disconnect codex --config-dir"),
            "{fix}"
        );
    }

    #[test]
    fn a_missing_codex_config_is_not_an_error() {
        // Plenty of people do not use Codex, and telling them something is
        // wrong would be worse than saying nothing.
        let dir = tempfile::tempdir().expect("tempdir");
        assert!(matches!(
            codex_state_at(&dir.path().join("absent.toml"), 8463),
            ClientState::NoConfig
        ));
    }

    #[test]
    fn an_unparseable_codex_config_says_so_rather_than_guessing() {
        let (_dir, path) = codex_config("this is = = not toml\n");
        assert!(matches!(
            codex_state_at(&path, 8463),
            ClientState::Unreadable(_)
        ));
    }
}
