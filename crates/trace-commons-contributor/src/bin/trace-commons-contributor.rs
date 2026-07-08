use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "trace-commons-contributor", version, about = "Submit local coding-agent traces to Trace Commons")]
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
    match cli.command {
        Command::Login { .. } => anyhow::bail!("not implemented"),
        Command::List => anyhow::bail!("not implemented"),
        Command::Submit { .. } => anyhow::bail!("not implemented"),
        Command::Status => anyhow::bail!("not implemented"),
        Command::Whoami => anyhow::bail!("not implemented"),
        Command::Logout => anyhow::bail!("not implemented"),
        Command::MintGrant { .. } => anyhow::bail!("not implemented"),
    }
}
