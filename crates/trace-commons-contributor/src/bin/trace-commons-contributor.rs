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
    /// Emit machine-readable JSON instead of human-readable lines. For
    /// callers driving this CLI programmatically (MCP servers, CI).
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enroll this device, with an instance-signed grant or an invite link
    Login {
        /// Base64 enrollment grant minted by your instance; omit to print this device's key id
        #[arg(long)]
        grant: Option<String>,
        /// Invite link you were handed, e.g. https://issuer.example.ai/onboard#CODE.
        /// Registers this device and writes the config in one step. Spends one
        /// use of the invite, so run it once.
        #[arg(long, conflicts_with = "grant")]
        invite: Option<String>,
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
    List {
        /// Path to a trajectory-v1 file or directory of them (from `npx @letta-ai/trajectory`)
        #[arg(long)]
        trajectory: Option<PathBuf>,
    },
    /// Redact and submit selected sessions
    Submit {
        #[arg(long)]
        all: bool,
        /// Only sessions started within this duration (e.g. 2d, 12h)
        #[arg(long)]
        since: Option<String>,
        /// Only sessions whose working directory is at or under this path
        #[arg(long)]
        project: Option<PathBuf>,
        /// Restrict to one source: claude-code | codex | trajectory
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
        /// Write a JSON manifest of uploaded envelope ids (submission_id + status) to this path
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Path to a trajectory-v1 file or directory of them (from `npx @letta-ai/trajectory`)
        #[arg(long)]
        trajectory: Option<PathBuf>,
        /// Exclude model reasoning from this submission (reasoning is included by default)
        #[arg(long)]
        no_reasoning: bool,
        /// Re-submit sessions whose local receipt is quarantined, keeping the
        /// same submission_id so the server can supersede the stored envelope
        #[arg(long)]
        remediate_quarantined: bool,
    },
    /// Show server-side status of previously submitted sessions
    Status,
    /// Fetch a server-signed attestation of your own scores, for handing to
    /// a collector. Unlike a list of submission ids, it cannot be forged by
    /// someone who merely learns the ids.
    Attest {
        /// Write the attestation here instead of printing it
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Claim, update, or withdraw your public handle. Requires the
    /// public_attribution scope, which is chosen at `login`.
    ///
    /// Setting a handle REPLACES the whole public profile, so one of `--bio`
    /// or `--no-bio` is required with `--handle`. The API has no way to say
    /// "leave the bio alone", and defaulting to empty would silently discard
    /// a bio you had already published.
    Profile {
        /// Handle to publish. ASCII letters, digits, `-` and `_`; no
        /// separator at either end.
        #[arg(long, conflicts_with = "withdraw")]
        handle: Option<String>,
        /// Short bio to publish alongside the handle
        #[arg(long, conflicts_with_all = ["withdraw", "no_bio"], requires = "handle")]
        bio: Option<String>,
        /// Publish no bio, clearing one if previously set
        #[arg(long, conflicts_with = "withdraw", requires = "handle")]
        no_bio: bool,
        /// Withdraw public attribution. The row goes at the next snapshot.
        #[arg(long)]
        withdraw: bool,
    },
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
async fn main() -> std::process::ExitCode {
    let cli = Cli::parse();
    let json = cli.json;
    match run(cli).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if json
                && error
                    .downcast_ref::<commands::RenderedSubmitFailure>()
                    .is_some()
            {
                return std::process::ExitCode::FAILURE;
            }
            // In --json mode a caller parses stdout. Emitting a bare
            // "Error: ..." line there would force it to special-case
            // failure, which is exactly when it most needs structure.
            if json {
                let out = serde_json::json!({
                    "schema_version": "trace_commons.cli_error.v1",
                    "error": format!("{error:#}"),
                });
                println!(
                    "{}",
                    serde_json::to_string_pretty(&out)
                        .unwrap_or_else(|_| r#"{"error":"serialization failed"}"#.to_string())
                );
            } else {
                eprintln!("Error: {error:#}");
            }
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> anyhow::Result<()> {
    let store = ConfigStore::resolve(cli.config_dir)?;
    match cli.command {
        Command::Login {
            grant,
            invite,
            allowed_hosts,
            scopes,
        } => {
            commands::login(
                &store,
                grant.as_deref(),
                invite.as_deref(),
                allowed_hosts.as_deref(),
                scopes.as_deref(),
            )
            .await
        }
        Command::List { trajectory } => commands::list(trajectory.as_deref(), cli.json),
        Command::Submit {
            all,
            since,
            project,
            source,
            yes,
            dry_run,
            pii_filter,
            manifest,
            trajectory,
            no_reasoning,
            remediate_quarantined,
        } => {
            let sel = commands::SubmitSelection {
                all,
                since: since.as_deref(),
                project: project.as_deref(),
                source: source.as_deref(),
                yes,
                dry_run,
                pii_filter: pii_filter.as_deref(),
                manifest: manifest.as_deref(),
                trajectory: trajectory.as_deref(),
                json: cli.json,
                no_reasoning,
                remediate_quarantined,
            };
            commands::submit(&store, &sel).await
        }
        Command::Status => commands::status(&store).await,
        Command::Attest { out } => commands::attest(&store, out.as_deref(), cli.json).await,
        Command::Profile {
            handle,
            bio,
            no_bio,
            withdraw,
        } => {
            commands::profile(
                &store,
                handle.as_deref(),
                bio.as_deref(),
                no_bio,
                withdraw,
                cli.json,
            )
            .await
        }
        Command::Whoami => commands::whoami(&store, cli.json),
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
