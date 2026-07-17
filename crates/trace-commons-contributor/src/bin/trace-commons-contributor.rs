use clap::{Parser, Subcommand};
use std::path::PathBuf;
use trace_commons_contributor::commands;
use trace_commons_contributor::config::ConfigStore;

#[derive(Parser)]
#[command(
    name = "trace-commons-contributor",
    version,
    about = "Submit local coding-agent traces to Trace Commons"
)]
struct Cli {
    /// Override the config directory (default: $TRACE_COMMONS_CONTRIBUTOR_DIR, then OS config dir)
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enroll this device with an instance-signed enrollment grant
    Login {
        /// Base64 enrollment grant minted by your instance; omit to print this device's key id
        #[arg(long)]
        grant: Option<String>,
        /// CSV of allowed issuer hosts (default: $TRACE_COMMONS_ALLOWED_HOSTS); persisted for later commands
        #[arg(long)]
        allowed_hosts: Option<String>,
        /// CSV of consent scopes to request (e.g. debugging_evaluation,model_training);
        /// omit to be prompted interactively (or default to the debugging_evaluation
        /// floor when not running in a terminal)
        #[arg(long)]
        scopes: Option<String>,
    },
    /// List discoverable local sessions
    List,
    /// Redact and submit selected sessions
    Submit {
        #[arg(long)]
        all: bool,
        /// Only sessions started within this duration (e.g. 2d, 12h)
        #[arg(long)]
        since: Option<String>,
        /// Only sessions whose project directory matches this path
        #[arg(long)]
        project: Option<PathBuf>,
        /// Restrict to one source: claude-code | codex
        #[arg(long)]
        source: Option<String>,
        /// Skip the interactive picker confirmation
        #[arg(long)]
        yes: bool,
        /// Run the full pipeline but upload nothing
        #[arg(long)]
        dry_run: bool,
        /// PII filter backend: near-ai (requires TRACE_NEAR_AI_PRIVACY_API_KEY)
        #[arg(long)]
        pii_filter: Option<String>,
        /// Devfolio submission id to stamp on every uploaded envelope
        /// (self-asserted attribution; overrides the config value)
        #[arg(long = "devfolio-submission")]
        devfolio_submission: Option<String>,
    },
    /// Show server-side status of previously submitted sessions
    Status,
    /// Print local identity (no network)
    Whoami,
    /// Delete local keystore, config, and receipts
    Logout,
    /// Operator/dogfood tool: mint an enrollment grant with an instance private key
    MintGrant {
        #[arg(long)]
        instance_key_pem: PathBuf,
        #[arg(long)]
        instance_id: String,
        #[arg(long)]
        user_subject: String,
        #[arg(long)]
        audience: String,
        #[arg(long)]
        issuer_url: String,
        /// Device key id to bind; defaults to this machine's local device key
        #[arg(long)]
        device_key_id: Option<String>,
        #[arg(long, default_value_t = 300)]
        ttl_seconds: i64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let store = ConfigStore::resolve(cli.config_dir)?;
    match cli.command {
        Command::Login {
            grant,
            allowed_hosts,
            scopes,
        } => {
            commands::login(
                &store,
                grant.as_deref(),
                allowed_hosts.as_deref(),
                scopes.as_deref(),
            )
            .await
        }
        Command::List => commands::list(),
        Command::Submit {
            all,
            since,
            project,
            source,
            yes,
            dry_run,
            pii_filter,
            devfolio_submission,
        } => {
            let sel = commands::SubmitSelection {
                all,
                since: since.as_deref(),
                project: project.as_deref(),
                source: source.as_deref(),
                yes,
                dry_run,
                pii_filter: pii_filter.as_deref(),
                devfolio_submission: devfolio_submission.as_deref(),
            };
            commands::submit(&store, &sel).await
        }
        Command::Status => commands::status(&store).await,
        Command::Whoami => commands::whoami(&store),
        Command::Logout => commands::logout(&store),
        Command::MintGrant {
            instance_key_pem,
            instance_id,
            user_subject,
            audience,
            issuer_url,
            device_key_id,
            ttl_seconds,
        } => commands::mint_grant_cmd(
            &store,
            &instance_key_pem,
            &instance_id,
            &user_subject,
            &audience,
            &issuer_url,
            device_key_id.as_deref(),
            ttl_seconds,
        ),
    }
}
