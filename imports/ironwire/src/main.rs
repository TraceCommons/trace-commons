//! The `ironwire` command-line interface.

mod commands;
mod render;
mod style;

use anyhow::Result;
use clap::{CommandFactory, Parser, Subcommand};

/// IronWire — one local inference endpoint for all your AI capacity.
#[derive(Debug, Parser)]
#[command(name = "ironwire", version, about, long_about = None)]
struct Cli {
    /// Port the daemon listens on.
    #[arg(long, env = "IRONWIRE_PORT", global = true)]
    port: Option<u16>,

    /// When to colour the output.
    ///
    /// `auto` colours only a terminal, so a pipe stays greppable. `NO_COLOR`
    /// in the environment overrides `auto` and is honoured everywhere.
    #[arg(long, value_enum, default_value_t = style::ColorChoice::Auto, global = true)]
    color: style::ColorChoice,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Set IronWire up: find your capacity, wire up your agents, start it.
    ///
    /// Asks one question — whether IronWire may use the subscriptions it found
    /// — and does the rest. `--dry-run` shows every change without making one.
    Init {
        /// Also write a commented `config.toml`, if there is not one already.
        #[arg(long)]
        write: bool,
        /// Show what would change, and change nothing.
        #[arg(long)]
        dry_run: bool,
        /// Do not install the background service; run `ironwire serve` yourself.
        #[arg(long)]
        no_service: bool,
    },

    /// Run the loopback daemon in the foreground.
    Serve,

    /// Show every connected backend and its observed capacity.
    Status {
        /// Emit JSON instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },

    /// Print one line of IronWire state, for an agent's status line.
    ///
    /// IronWire will not write into a response stream — the only text channel
    /// there is the model's own. A status line is the harness's own furniture,
    /// so this is where IronWire gets to say something without putting words in
    /// the model's mouth. Wired up by `ironwire connect claude`.
    Statusline,

    /// Connect a coding agent or a capacity source.
    ///
    /// For Claude Code and Codex, one login at a time: the default one, or the
    /// one in `--config-dir`. With `CLAUDE_CONFIG_DIR` set, `connect claude`
    /// means the login in that directory.
    Connect {
        /// What to connect: `claude`, `codex`, `anthropic-api`, `openai-api`, `near`.
        target: String,
        /// Enable the subscription backend for this target, after consent.
        #[arg(long)]
        subscription: bool,
        /// A login of its own: the directory you export as `CLAUDE_CONFIG_DIR`
        /// (Claude Code) or `CODEX_HOME` (Codex) when you use it. Declared in
        /// config.toml, and that client is pointed at a URL naming it. The
        /// client's usual directory (`~/.claude`, `~/.codex`) moves your
        /// default login there instead.
        #[arg(long, value_name = "DIR")]
        config_dir: Option<String>,
        /// The backend id for that login. Defaults to one taken from the
        /// directory's name, e.g. `claude-work` for `~/.claude-work`.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
        /// Print what would change without writing anything.
        #[arg(long)]
        dry_run: bool,
    },

    /// Undo a `connect`.
    Disconnect {
        /// What to disconnect.
        target: String,
        /// Revoke the subscription consent instead, leaving the client wired.
        #[arg(long)]
        subscription: bool,
        /// The login, by the directory its client uses.
        #[arg(long, value_name = "DIR", conflicts_with = "id")]
        config_dir: Option<String>,
        /// The login, by its backend id.
        #[arg(long, value_name = "ID")]
        id: Option<String>,
    },

    /// Check every connection end to end, with a real request per backend.
    Doctor,

    /// Show recent exchanges from the local trace ledger.
    Log {
        /// How many exchanges to show, newest first.
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Emit JSON instead of the human-readable table.
        #[arg(long)]
        json: bool,
    },

    /// Report whether a newer IronWire exists. Never applies it.
    Update,

    /// Print the environment a client needs, for `eval "$(ironwire env)"`.
    Env {
        /// Shell syntax to emit. Defaults to `$SHELL`, so piping this into
        /// `eval` works without anyone having to think about it.
        #[arg(long)]
        shell: Option<String>,
    },

    /// Inspect the optional privacy filter.
    ///
    /// `check <file>` shows what the configured filter would and would not
    /// catch, so its false-negative rate is something you can see rather than
    /// take on trust.
    Privacy {
        /// `check` or `status`.
        action: String,
        /// File to scan, for `check`.
        path: Option<std::path::PathBuf>,
        /// Check against a mode other than the configured one, to see what it
        /// would catch before switching to it.
        #[arg(long)]
        mode: Option<String>,
    },

    /// Watch routing decisions as they happen.
    ///
    /// IronWire will not write into your agent's transcript — the only channel
    /// there is the response stream, and putting a line in it would put words
    /// in the model's mouth. The channels that are not the transcript: this,
    /// and `ironwire statusline` in the agent's own status bar.
    Watch {
        /// Only show family changes and failures — the events that mean
        /// something. On a healthy system this prints nothing.
        #[arg(long)]
        only_changes: bool,
    },

    /// Run the daemon in the background as a user agent.
    ///
    /// Always a *user* agent, never a system service: IronWire holds your
    /// credentials and must not run with more privilege than you have.
    Service {
        /// `install`, `uninstall`, or `status`.
        action: String,
    },

    /// Emit a shell completion script.
    ///
    /// Packaged installs wire this up for you; `eval "$(ironwire completions
    /// bash)"` does it by hand.
    Completions {
        /// bash, zsh, fish, powershell, or elvish.
        shell: clap_complete::Shell,
    },

    /// Force all traffic onto one backend, or clear the force.
    Pin {
        /// Backend id. Omit to clear.
        backend: Option<String>,
        /// Model to force.
        #[arg(long)]
        model: Option<String>,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    // Parsed before the subscriber exists, because `--color` decides whether
    // the logs are coloured too. `tracing_subscriber` paints unconditionally
    // otherwise, and its stderr is in the same pipe as everything else the
    // moment anyone writes `ironwire status 2>&1 | grep`.
    let cli = Cli::parse();
    let style = style::Style::resolve(cli.color);

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("IRONWIRE_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("ironwire=info,warn")),
        )
        .with_target(false)
        .with_ansi(style::logs_coloured(cli.color))
        .init();
    match cli.command {
        Command::Init {
            write,
            dry_run,
            no_service,
        } => commands::init::run(cli.port, write, dry_run, no_service).await,
        Command::Serve => commands::serve::run(cli.port).await,
        Command::Status { json } => commands::status::run(cli.port, json, style).await,
        Command::Statusline => commands::statusline::run(cli.port).await,
        Command::Connect {
            target,
            subscription,
            config_dir,
            id,
            dry_run,
        } => commands::connect::run(
            &target,
            subscription,
            dry_run,
            cli.port,
            commands::connect::LoginArgs { config_dir, id },
        ),
        Command::Disconnect {
            target,
            subscription,
            config_dir,
            id,
        } => commands::connect::disconnect(
            &target,
            subscription,
            commands::connect::LoginArgs { config_dir, id },
        ),
        Command::Doctor => commands::doctor::run(cli.port).await,
        Command::Log { limit, json } => commands::log::run(cli.port, limit, json, style).await,
        Command::Update => commands::update::run(cli.port).await,
        Command::Env { shell } => commands::connect::print_env(cli.port, shell),
        Command::Privacy { action, path, mode } => commands::privacy::run(&action, path, mode),
        Command::Watch { only_changes } => commands::watch::run(cli.port, only_changes).await,
        Command::Service { action } => commands::service::run(&action, cli.port),
        Command::Completions { shell } => {
            clap_complete::generate(
                shell,
                &mut Cli::command(),
                "ironwire",
                &mut std::io::stdout(),
            );
            Ok(())
        }
        Command::Pin { backend, model } => commands::pin::run(cli.port, backend, model).await,
    }
}
