use crate::commands;
use crate::config::ConfigStore;
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[path = "cli/insights.rs"]
mod insights_cli;
#[path = "cli/mission_draft.rs"]
mod mission_draft_cli;

#[derive(Parser)]
#[command(
    name = "trace-commons-contributor",
    // The semver does not move when a deploy does, so --version carries the
    // commit the binary was built from as well.
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION")),
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
    /// Launch a coding tool in this terminal with an isolated saved account.
    Launch(crate::managed::cli::LaunchArgs),
    /// Manage saved native subscriptions and OS-protected API keys.
    Accounts {
        #[command(subcommand)]
        action: crate::managed::cli::AccountCommand,
    },
    /// Show managed sessions, including sessions started from the desktop app.
    Sessions {
        #[command(subcommand)]
        action: crate::managed::cli::SessionCommand,
    },
    #[command(hide = true)]
    RedeemLaunch {
        #[arg(long)]
        session_id: uuid::Uuid,
        #[arg(long)]
        ticket: String,
    },
    /// Check a local mission proposal; does not fetch, execute, or publish
    MissionDraft {
        #[arg(long)]
        file: PathBuf,
    },
    /// Manage explicitly imported local mission proposals; never fetches or publishes
    MissionDrafts(mission_draft_cli::MissionDraftsArgs),
    /// Analyze explicitly selected local sessions without enrollment or contribution
    Insights(insights_cli::InsightsArgs),
    /// Locally redact and preview a versioned explicit import; never uploads or grants admission
    ImportPreview {
        /// User-selected local evidence-import-v1 JSON file (no URL retrieval)
        #[arg(long)]
        file: PathBuf,
        /// Original working directory to redact, including a foreign-machine path; never opened
        #[arg(long)]
        cwd: String,
    },
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
        /// Skip the interactive consent menu and take its default answer (no)
        /// for every optional scope, enrolling with the always-on
        /// debugging_evaluation floor only. For agents and other scripted
        /// callers, which have a terminal but no one to answer the prompts.
        /// Use --scopes to grant anything beyond the floor.
        #[arg(
            long = "default",
            visible_alias = "defaults",
            conflicts_with = "scopes"
        )]
        default_consent: bool,
    },
    /// List discoverable local sessions
    List {
        /// Path to a trajectory-v1 file or directory of them (from `npx @letta-ai/trajectory`)
        #[arg(long)]
        trajectory: Option<PathBuf>,
    },
    /// Redact and submit sessions from the current directory's subtree
    ///
    /// With no flags this covers the working directory and everything under
    /// it, which is how you scope a run: stand in one project to submit that
    /// project, or in the parent of several repos to submit all of them. It
    /// refuses to run from `$HOME` or a filesystem root, where the subtree
    /// would be every session on the machine; `--all` says that deliberately.
    Submit {
        /// Every session on this machine, ignoring the working directory.
        /// Widens the scope only: the y/N summary still appears, and only
        /// --yes skips it. The existing --json --all mode remains non-interactive.
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
        /// Skip the confirmation and submit everything selected. This is the
        /// flag for suppressing the prompt in human-readable mode; --all does not.
        #[arg(long)]
        yes: bool,
        /// Choose sessions individually from a numbered table, instead of
        /// confirming the batch as a whole
        #[arg(long)]
        pick: bool,
        /// Run the full pipeline but upload nothing
        #[arg(long)]
        dry_run: bool,
        /// PII filter backend: near-ai (requires TRACE_NEAR_AI_PRIVACY_API_KEY)
        #[arg(long)]
        pii_filter: Option<String>,
        /// Write a JSON manifest of uploaded envelope ids (submission_id + status) to this path
        #[arg(long)]
        manifest: Option<PathBuf>,
        /// Write the signed score attestation for this run's submissions to this path.
        /// This is what a collector scores you on; an id list is not proof of authorship.
        #[arg(long, conflicts_with = "dry_run")]
        attest_out: Option<PathBuf>,
        /// POST the attestation to this collector endpoint instead of carrying the file
        /// yourself. Must be https, and the host must be on your allowlist.
        #[arg(long, conflicts_with = "dry_run")]
        attest_post: Option<String>,
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
        /// How these sessions went: worked | partly | failed. Recorded as the trace
        /// outcome, which nothing else in a transcript can answer.
        #[arg(long)]
        outcome: Option<String>,
        /// Invite link to enroll with if this machine is not enrolled yet.
        /// Prefer `TRACE_COMMONS_INVITE`: an invite passed here lands in your
        /// shell history and in `ps`.
        /// Reading it from the environment is done in `commands::submit`, so
        /// there is exactly one place the invite is sourced from.
        #[arg(long)]
        invite: Option<String>,
    },
    /// Import Antigravity conversations. Requires the Antigravity IDE to be
    /// running, since its conversations are only readable through the local
    /// API it serves.
    ///
    /// Imported conversations are staged, not submitted: run `submit`
    /// afterwards to redact and upload them.
    ImportAntigravity {
        /// Only import conversations for this project (default: the current directory)
        #[arg(long)]
        project: Option<PathBuf>,
        /// Import every conversation the running instance exposes, not just this project's
        #[arg(long, conflicts_with = "project")]
        all: bool,
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
    /// Dev-only: compare the local credit estimate with the credit this
    /// machine's scored sessions were given. Reads local files only and
    /// sends nothing; prints aggregate counts.
    #[command(hide = true)]
    EstimateCheck {
        /// A local estimate table file to check instead of the built-in one
        #[arg(long)]
        table: Option<PathBuf>,
    },
    /// Check for a newer release, verify it, and install it
    ///
    /// Refuses anything it cannot verify: the manifest signature, the
    /// sha256, and on Windows the Authenticode signer must all check out,
    /// and the offered version must be strictly newer. There is no flag
    /// that skips any of that. When a package manager installed this copy
    /// (winget, Homebrew, a distro package, etc.), this prints how to
    /// update it that way instead and installs nothing.
    Update {
        /// Verify and stage the update without replacing anything; the
        /// daemon applies it at its next start
        #[arg(long)]
        stage_only: bool,
    },
    /// Delete local keystore, config, receipts, history and audit log.
    /// Asks first, listing what goes; submitted traces stay on the server.
    Logout {
        /// Skip the confirmation. Nothing else suppresses it: a closed stdin
        /// counts as no.
        #[arg(long)]
        yes: bool,
    },
    /// Remove this Mac's enrollment: its config, device key and account
    /// session. Local only: the account and submitted traces stay on the
    /// server. Approved sessions not yet sent go back to waiting; history,
    /// receipts, settings and folder rules stay. Asks first.
    Unenroll {
        /// Skip the confirmation. Nothing else suppresses it: a closed stdin
        /// counts as no.
        #[arg(long)]
        yes: bool,
    },
    /// Choose how your traces may be used (consent scopes)
    Consent {
        #[command(subcommand)]
        action: ConsentAction,
    },
    /// Sign in to your account (needed to withdraw traces), or check/end that session
    Account {
        #[command(subcommand)]
        action: AccountAction,
    },
    /// Run and control the background upload daemon
    Daemon {
        #[command(subcommand)]
        action: DaemonAction,
    },
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

/// The daemon control surface.
///
/// Deliberately full parity with what the native menu-bar and window
/// applications can do, so the daemon stays completely usable over SSH and
/// on a machine with no desktop session. Parity runs in both directions:
/// there is no terminal-only operation here. Arming a project for automatic
/// upload and approving everything at once were once refused over the
/// socket; that gate was removed (it restricted nothing an attacker with
/// same-user code execution already had -- see the `daemon::ipc` module
/// doc), and a local audit log replaced it. `daemon audit` reads it.
///
/// Every mutating command here is delivered to the *running* daemon over
/// its socket when one is running, so it takes effect immediately rather
/// than being overwritten by the daemon's next pass. See `daemon::client`.
/// Account session, as distinct from device enrollment. The device key
/// uploads; the account withdraws. Keeping them separate is deliberate --
/// a stolen device key must not be worth the ability to delete someone's
/// contribution history.
#[derive(Subcommand)]
enum AccountAction {
    /// Sign in through your browser (opens it, or prints the URL)
    Login {
        /// Print the URL instead of opening a browser. The headless path.
        #[arg(long)]
        no_browser: bool,
    },
    /// Read current contribution readiness from your account
    Status,
    /// Redeem an invite code read from standard input (input is echoed; pipe
    /// the code in to keep it off the screen). Keep the same idempotency key
    /// when retrying
    Redeem {
        #[arg(long)]
        idempotency_key: uuid::Uuid,
    },
    /// Revoke the account session and forget it locally
    Logout,
}

/// Consent, after enrollment.
#[derive(Subcommand)]
enum ConsentAction {
    /// Choose the consent scopes, from a menu or `--scopes`. The always-on
    /// floor scope is always included. Recorded as your choice, which lets
    /// an enrollment made with `login --default` or without a terminal
    /// start contributing.
    Scopes {
        /// CSV of consent scopes (e.g. benchmark_only,model_training); omit
        /// to choose from the menu. Required when not run in a terminal.
        #[arg(long)]
        scopes: Option<String>,
    },
}

#[derive(Subcommand)]
enum DaemonAction {
    /// Inspect or remove locally owned token review copies; never agent files
    TokenStorage {
        #[arg(long, conflicts_with = "discard")]
        cleanup: bool,
        /// Discard unsubmitted token reviews; undo approvals first
        #[arg(long, requires = "confirm")]
        discard: bool,
        #[arg(long)]
        confirm: bool,
    },
    /// Run the daemon in the foreground; a service manager backgrounds it
    Run {
        /// Watch and queue as normal, but upload nothing
        #[arg(long)]
        dry_run: bool,
    },
    /// Show what the daemon is doing and whether anything is wrong
    Status,
    /// List sessions waiting for a decision
    Pending,
    /// Show what would be sent for one queued session
    Preview { entry_id: String },
    /// Approve one queued session, all of them, or a whole project's
    Approve {
        entry_id: Option<String>,
        #[arg(long)]
        all: bool,
        /// Approve every pending session in this project (project_id, not
        /// project_label)
        #[arg(long)]
        project: Option<String>,
    },
    /// Decline one queued session
    Dismiss { entry_id: String },
    /// Withdraw a submitted trace: content deleted, tier-dependent on how
    /// far it had already gone. See
    /// docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md.
    Withdraw {
        submission_id: Option<String>,
        /// Withdraw every trace currently held for privacy review, not one
        #[arg(long = "all-quarantined", conflicts_with = "submission_id")]
        all_quarantined: bool,
    },
    /// Stop queueing and uploading until resumed
    Pause,
    /// Resume after a pause
    Resume,
    /// List projects and their upload modes
    Projects,
    /// Set a project's upload mode
    Project {
        /// The project's working directory
        path: PathBuf,
        /// auto | notify | ignore
        #[arg(long)]
        mode: String,
        /// With `--mode auto`: also send the sessions already on disk.
        /// Without it, arming applies to new sessions only and the backlog
        /// waits for you.
        #[arg(long)]
        include_backlog: bool,
    },
    /// Show contribution history and the credit rollup
    History {
        #[arg(long, default_value_t = 20)]
        limit: usize,
        /// Ask the daemon to refresh from the server before showing
        #[arg(long)]
        refresh: bool,
    },
    /// Read the local audit log: autonomy armed, queue bulk-approved,
    /// consent scopes changed, NEAR AI notice acknowledged
    Audit {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Show or change daemon settings
    Settings {
        #[arg(long = "set", value_name = "KEY=VALUE")]
        set: Vec<String>,
    },
    /// Install the systemd user unit (Linux)
    Install,
    /// Remove the systemd user unit (Linux)
    Uninstall,
}

/// Entry point shared by the `trace-commons-contributor` and `near-ai`
/// binaries. Invoked as `near-ai`, the help names the canonical CLI.
pub fn run() -> std::process::ExitCode {
    // Windows gives the main thread 1 MiB, and a debug build's `daemon
    // preview` frames outgrow it even with the dispatch future boxed. Run
    // the whole CLI on a thread whose stack is ours to size.
    const CLI_STACK_BYTES: usize = 8 * 1024 * 1024;
    std::thread::Builder::new()
        .name("cli".into())
        .stack_size(CLI_STACK_BYTES)
        .spawn(run_on_this_thread)
        .expect("spawn the CLI thread")
        .join()
        .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
}

fn run_on_this_thread() -> std::process::ExitCode {
    use clap::CommandFactory;
    let canonical = std::env::args_os()
        .next()
        .and_then(|p| PathBuf::from(p).file_stem().map(|s| s == "near-ai"))
        .unwrap_or(false);
    let mut command = Cli::command();
    if canonical {
        command = command
            .name("near-ai")
            .about("NEAR AI — manage native coding sessions and contribute to Trace Commons");
    }
    let cli = <Cli as clap::FromArgMatches>::from_arg_matches(&command.get_matches())
        .unwrap_or_else(|e| e.exit());
    let json = cli.json;
    async_main(cli, json)
}

#[tokio::main]
async fn async_main(cli: Cli, json: bool) -> std::process::ExitCode {
    // `dispatch` handles every command, including the larger async submission
    // paths. Keep that combined future off the smaller Windows main-thread
    // stack even when the selected command itself is synchronous.
    let run_future = Box::pin(dispatch(cli));
    match run_future.await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if json
                && error
                    .downcast_ref::<commands::RenderedJsonFailure>()
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

async fn dispatch(cli: Cli) -> anyhow::Result<()> {
    if let Command::MissionDraft { file } = &cli.command {
        let review = crate::mission_draft::review_file(file)?;
        if cli.json {
            println!("{}", serde_json::to_string_pretty(&review)?);
        } else {
            println!("Draft structure checked. Curator review is required.");
            println!("Proposal SHA-256: {}", review.proposal_sha256);
            println!("Source claims, reproducibility, evaluator, and budget remain unverified.");
        }
        return Ok(());
    }
    if let Command::MissionDrafts(args) = &cli.command {
        return mission_draft_cli::run(args, cli.json);
    }
    if let Command::Insights(args) = &cli.command {
        return insights_cli::run(args, cli.json);
    }
    if let Command::ImportPreview { file, cwd } = &cli.command {
        let prepared = crate::evidence_import::read_import(file)?;
        let preview = prepared.local_preview(cwd).await?;
        println!("{}", serde_json::to_string_pretty(&preview)?);
        return Ok(());
    }
    let store = ConfigStore::resolve(cli.config_dir)?;
    match cli.command {
        Command::Launch(args) => {
            let code = crate::managed::cli::launch(&store, args).await?;
            std::process::exit(code);
        }
        Command::Accounts { action } => {
            crate::managed::cli::accounts(&store, action, cli.json).await
        }
        Command::Sessions { action } => crate::managed::cli::sessions(&store, action, cli.json),
        Command::RedeemLaunch { session_id, ticket } => {
            let code = crate::managed::supervisor::redeem_and_run(
                &store,
                crate::managed::sessions::PreparedLaunch {
                    session_id,
                    ticket: Some(ticket),
                },
            )
            .await?;
            std::process::exit(code);
        }
        Command::Login {
            grant,
            invite,
            allowed_hosts,
            scopes,
            default_consent,
        } => {
            commands::login(
                &store,
                grant.as_deref(),
                invite.as_deref(),
                allowed_hosts.as_deref(),
                scopes.as_deref(),
                default_consent,
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
            pick,
            dry_run,
            pii_filter,
            manifest,
            attest_out,
            attest_post,
            trajectory,
            no_reasoning,
            remediate_quarantined,
            outcome,
            invite,
        } => {
            let sel = commands::SubmitSelection {
                all,
                since: since.as_deref(),
                project: project.as_deref(),
                source: source.as_deref(),
                yes,
                pick,
                dry_run,
                pii_filter: pii_filter.as_deref(),
                manifest: manifest.as_deref(),
                attest_out: attest_out.as_deref(),
                attest_post: attest_post.as_deref(),
                trajectory: trajectory.as_deref(),
                json: cli.json,
                no_reasoning,
                remediate_quarantined,
                verdict: outcome.as_deref(),
                invite: invite.as_deref(),
            };
            commands::submit(&store, &sel).await
        }
        Command::ImportPreview { .. }
        | Command::Insights(_)
        | Command::MissionDraft { .. }
        | Command::MissionDrafts(_) => {
            anyhow::bail!("import-preview-dispatch-invalid")
        }
        Command::ImportAntigravity { project, all } => {
            commands::import_antigravity(&store, project.as_deref(), all, cli.json).await
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
        Command::EstimateCheck { table } => {
            let table = crate::credit_estimate_check::table_from(table.as_deref())?;
            let report = crate::credit_estimate_check::run(&store, &table)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&report)?);
            } else {
                for line in crate::credit_estimate_check::render(&report) {
                    println!("{line}");
                }
            }
            Ok(())
        }
        Command::Update { stage_only } => commands::update(&store, stage_only, cli.json).await,
        Command::Logout { yes } => commands::logout(&store, yes),
        Command::Unenroll { yes } => commands::unenroll(&store, yes, cli.json),
        Command::Consent { action } => match action {
            ConsentAction::Scopes { scopes } => {
                commands::consent_scopes(&store, scopes.as_deref(), cli.json)
            }
        },
        Command::Account { action } => match action {
            AccountAction::Login { no_browser } => {
                commands::account_login(&store, no_browser, cli.json).await
            }
            AccountAction::Status => commands::account_status(&store, cli.json).await,
            AccountAction::Redeem { idempotency_key } => {
                eprintln!("Invite code (read from standard input):");
                let mut code = String::new();
                std::io::stdin().read_line(&mut code)?;
                commands::account_redeem(&store, &code, idempotency_key, cli.json).await
            }
            AccountAction::Logout => commands::account_logout(&store).await,
        },
        Command::Daemon { action } => match action {
            DaemonAction::Run { dry_run } => crate::daemon::run(store, dry_run).await,
            DaemonAction::TokenStorage {
                cleanup,
                discard,
                confirm,
            } => commands::daemon_token_storage(&store, cleanup, discard, confirm, cli.json),
            DaemonAction::Status => commands::daemon_status(&store, cli.json),
            DaemonAction::Pending => commands::daemon_pending(&store, cli.json),
            DaemonAction::Preview { entry_id } => {
                commands::daemon_preview(&store, &entry_id, cli.json)
            }
            DaemonAction::Approve {
                entry_id,
                all,
                project,
            } => commands::daemon_approve(
                &store,
                entry_id.as_deref(),
                all,
                project.as_deref(),
                cli.json,
            ),
            DaemonAction::Dismiss { entry_id } => {
                commands::daemon_dismiss(&store, &entry_id, cli.json)
            }
            DaemonAction::Withdraw {
                submission_id,
                all_quarantined,
            } => commands::daemon_withdraw(
                &store,
                submission_id.as_deref(),
                all_quarantined,
                cli.json,
            ),
            DaemonAction::Pause => commands::daemon_pause(&store, true, cli.json),
            DaemonAction::Resume => commands::daemon_pause(&store, false, cli.json),
            DaemonAction::Projects => commands::daemon_projects(&store, cli.json),
            DaemonAction::Project {
                path,
                mode,
                include_backlog,
            } => commands::daemon_set_project(&store, &path, &mode, include_backlog, cli.json),
            DaemonAction::History { limit, refresh } => {
                commands::daemon_history(&store, limit, refresh, cli.json).await
            }
            DaemonAction::Audit { limit } => commands::daemon_audit(&store, limit, cli.json),
            DaemonAction::Settings { set } => commands::daemon_settings(&store, &set, cli.json),
            DaemonAction::Install => commands::daemon_install(&store),
            DaemonAction::Uninstall => commands::daemon_uninstall(),
        },
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

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Cli {
        Cli::try_parse_from(
            std::iter::once("trace-commons-contributor").chain(args.iter().copied()),
        )
        .unwrap_or_else(|e| panic!("{args:?}: {e}"))
    }

    #[test]
    fn unenroll_parses_with_and_without_yes() {
        assert!(matches!(
            parse(&["unenroll"]).command,
            Command::Unenroll { yes: false }
        ));
        assert!(matches!(
            parse(&["unenroll", "--yes"]).command,
            Command::Unenroll { yes: true }
        ));
    }

    #[test]
    fn consent_scopes_parses_with_and_without_a_list() {
        match parse(&["consent", "scopes"]).command {
            Command::Consent {
                action: ConsentAction::Scopes { scopes: None },
            } => {}
            _ => panic!("consent scopes"),
        }
        match parse(&["consent", "scopes", "--scopes", "model_training"]).command {
            Command::Consent {
                action: ConsentAction::Scopes { scopes: Some(s) },
            } => assert_eq!(s, "model_training"),
            _ => panic!("consent scopes --scopes"),
        }
    }
}
