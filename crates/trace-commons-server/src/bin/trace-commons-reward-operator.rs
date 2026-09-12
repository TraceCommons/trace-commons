// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator CLI for the tenant-scoped reward pilot.
//!
//! INTEGRATION: Cargo auto-discovers this bin. The server's mission_rewards
//! adapter executes V69's protected PostgreSQL functions.

use std::fs::File;
use std::io::Read;
use std::path::PathBuf;

use clap::{Args, Parser, Subcommand};
use secrecy::SecretString;
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::postgres::PgBackend,
    mission_rewards::{RewardError, RewardProgramTerms},
};
use uuid::Uuid;

const DATABASE_URL_ENV: &str = "TRACE_COMMONS_REWARDS_DATABASE_URL";
const MAX_TERMS_BYTES: u64 = 16 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-reward-operator",
    about = "Operator-managed reward pilot CLI",
    long_about = "Tenant-scoped reward pilot for operator-asserted participant identities.\n\
Review is manual and program units are nonredeemable; this CLI performs no\n\
automatic mission verification. Database session authentication determines\n\
issuer and reviewer authority.",
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
struct Cli {
    /// Tenant identifier authorized for the current database login.
    #[arg(long)]
    tenant: String,

    /// Output is always JSON.
    #[arg(long, global = true)]
    json: bool,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Create an immutable v1 reward program from a terms JSON file.
    ProgramCreate(ProgramCreateArgs),
    /// Show an authorized program and its capacity projection.
    ProgramShow(ProgramShowArgs),
    /// Reserve fixed program units for operator-asserted participant work.
    Reserve(ReserveArgs),
    /// Submit retained evidence by digest for manual review.
    ClaimSubmit(ClaimSubmitArgs),
    /// Record the manual reviewer decision selected by database session authority.
    Review(ReviewArgs),
    /// Cancel an unsubmitted reservation.
    Cancel(CancelArgs),
    /// Invalidate an evidence digest before or after review.
    Invalidate(InvalidateArgs),
    /// Read bounded, authorized history for an operator-asserted participant.
    History(HistoryArgs),
}

#[derive(Debug, Args)]
struct ProgramCreateArgs {
    #[arg(long)]
    program: Uuid,
    #[arg(long)]
    terms: PathBuf,
}

#[derive(Debug, Args)]
struct ProgramShowArgs {
    #[arg(long)]
    program: Uuid,
}

#[derive(Debug, Args)]
struct ReserveArgs {
    #[arg(long)]
    program: Uuid,
    #[arg(long)]
    reservation: Uuid,
    #[arg(long)]
    participant_hash: String,
    #[arg(long)]
    work_hash: String,
    #[arg(long)]
    consent_hash: String,
}

#[derive(Debug, Args)]
struct ClaimSubmitArgs {
    #[arg(long)]
    reservation: Uuid,
    #[arg(long)]
    evidence_hash: String,
    #[arg(long)]
    evaluation_hash: String,
}

#[derive(Debug, Args)]
struct ReviewArgs {
    #[arg(long)]
    reservation: Uuid,
    #[arg(long)]
    decision: Uuid,
    #[arg(long, action = clap::ArgAction::Set)]
    accept: bool,
    #[arg(long)]
    reason: String,
}

#[derive(Debug, Args)]
struct CancelArgs {
    #[arg(long)]
    reservation: Uuid,
    #[arg(long)]
    decision: Uuid,
}

#[derive(Debug, Args)]
struct InvalidateArgs {
    #[arg(long)]
    evidence_hash: String,
    #[arg(long)]
    decision: Uuid,
}

#[derive(Debug, Args)]
struct HistoryArgs {
    #[arg(long)]
    participant_hash: String,
    #[arg(long, default_value_t = 50, value_parser = clap::value_parser!(i32).range(1..=100))]
    limit: i32,
}

#[tokio::main]
async fn main() {
    let result = run(Cli::parse()).await;
    if let Err(error) = result {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<(), CliError> {
    let config = database_config_from_env()?;
    let backend = PgBackend::new(&config)
        .await
        .map_err(|_| CliError::DatabaseUnavailable)?;

    let value = match cli.command {
        Command::ProgramCreate(args) => {
            let terms = read_terms(&args.terms)?;
            backend
                .reward_program_create(&cli.tenant, args.program, &terms)
                .await
        }
        Command::ProgramShow(args) => backend.reward_program_show(&cli.tenant, args.program).await,
        Command::Reserve(args) => {
            backend
                .reward_reserve(
                    &cli.tenant,
                    args.program,
                    args.reservation,
                    &args.participant_hash,
                    &args.work_hash,
                    &args.consent_hash,
                )
                .await
        }
        Command::ClaimSubmit(args) => {
            backend
                .reward_claim_submit(
                    &cli.tenant,
                    args.reservation,
                    &args.evidence_hash,
                    &args.evaluation_hash,
                )
                .await
        }
        Command::Review(args) => {
            backend
                .reward_review(
                    &cli.tenant,
                    args.reservation,
                    args.decision,
                    args.accept,
                    &args.reason,
                )
                .await
        }
        Command::Cancel(args) => {
            backend
                .reward_cancel(&cli.tenant, args.reservation, args.decision)
                .await
        }
        Command::Invalidate(args) => {
            backend
                .reward_invalidate(&cli.tenant, &args.evidence_hash, args.decision)
                .await
        }
        Command::History(args) => {
            backend
                .reward_history(&cli.tenant, &args.participant_hash, args.limit)
                .await
        }
    }
    .map_err(CliError::Operation)?;

    let _ = cli.json;
    println!(
        "{}",
        serde_json::to_string(&value).map_err(|_| CliError::OutputEncoding)?
    );
    Ok(())
}

fn database_config_from_env() -> Result<DatabaseConfig, CliError> {
    let url = std::env::var(DATABASE_URL_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(CliError::MissingDatabaseUrl)?;
    validate_local_transport(&url)?;
    Ok(DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 1,
        ssl_mode: SslMode::from_env(),
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
}

// PgBackend currently uses NoTls. Remote administration must arrive through a
// protected local tunnel rather than sending operator credentials in plaintext.
fn validate_local_transport(url: &str) -> Result<(), CliError> {
    let config = url
        .parse::<tokio_postgres::Config>()
        .map_err(|_| CliError::UnsafeTransport)?;
    let hosts_are_local = config.get_hosts().iter().all(|host| match host {
        tokio_postgres::config::Host::Tcp(host) => host
            .parse::<std::net::IpAddr>()
            .is_ok_and(|address| address.is_loopback()),
        #[cfg(unix)]
        tokio_postgres::config::Host::Unix(_) => true,
    });
    if config.get_hosts().is_empty()
        || !hosts_are_local
        || config
            .get_hostaddrs()
            .iter()
            .any(|address| !address.is_loopback())
    {
        return Err(CliError::UnsafeTransport);
    }
    Ok(())
}

fn read_terms(path: &PathBuf) -> Result<RewardProgramTerms, CliError> {
    let file = File::open(path).map_err(|_| CliError::TermsUnreadable)?;
    let mut bytes = Vec::new();
    file.take(MAX_TERMS_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::TermsUnreadable)?;
    if bytes.len() as u64 > MAX_TERMS_BYTES {
        return Err(CliError::TermsTooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| CliError::TermsInvalid)
}

#[derive(Debug)]
enum CliError {
    MissingDatabaseUrl,
    DatabaseUnavailable,
    UnsafeTransport,
    Operation(RewardError),
    TermsUnreadable,
    TermsTooLarge,
    TermsInvalid,
    OutputEncoding,
}

impl std::fmt::Display for CliError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::MissingDatabaseUrl => {
                "reward_database_url_missing: set TRACE_COMMONS_REWARDS_DATABASE_URL and retry"
            }
            Self::DatabaseUnavailable => {
                "reward_database_unavailable: verify the configured reward database is reachable"
            }
            Self::UnsafeTransport => {
                "reward_transport_invalid: use a loopback address or Unix socket through a protected local connection"
            }
            Self::Operation(error) => {
                return write!(
                    formatter,
                    "{}: {}",
                    error.label(),
                    operation_next_action(*error)
                );
            }
            Self::TermsUnreadable => "reward_terms_unreadable: verify the terms file can be read",
            Self::TermsTooLarge => {
                "reward_terms_too_large: reduce the terms file to 16 KiB or less"
            }
            Self::TermsInvalid => {
                "reward_terms_invalid: correct the v1 reward terms JSON schema and retry"
            }
            Self::OutputEncoding => "reward_output_failed: retry the command",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CliError {}

fn operation_next_action(error: RewardError) -> &'static str {
    match error {
        RewardError::Unauthorized => "verify the database login has the required tenant grant",
        RewardError::RequestInvalid => "correct the command inputs and retry",
        RewardError::NotFound => "verify the tenant and resource identifier",
        RewardError::PayloadConflict => "retry with the original request payload",
        RewardError::ProgramClosed => "use an open program",
        RewardError::CapacityExhausted => "select a program with available capacity",
        RewardError::ParticipantCap => "inspect this participant's program history",
        RewardError::WorkDuplicate => "inspect the existing reservation for this work",
        RewardError::EvidenceDuplicate => "inspect the original claim for this evidence",
        RewardError::EvidenceInvalidated => "do not submit invalidated evidence",
        RewardError::ReservationExpired => {
            "review the expired reservation under the published terms"
        }
        RewardError::StateConflict => "read the reservation state before retrying",
        RewardError::SelfReview => "use a distinct authorized reviewer login",
        RewardError::StoreUnavailable => "retry after the reward database is reachable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transport_refuses_remote_hosts_and_hostaddr_overrides() {
        for url in [
            "dbname=rewards",
            "postgresql://operator@203.0.113.1/rewards",
            "host=127.0.0.1 hostaddr=203.0.113.1 dbname=rewards",
            "host=127.0.0.1,203.0.113.1 dbname=rewards",
        ] {
            assert!(validate_local_transport(url).is_err());
        }
        assert!(validate_local_transport("postgresql://operator@127.0.0.1/rewards").is_ok());
        assert!(validate_local_transport("host=::1 dbname=rewards").is_ok());
        #[cfg(unix)]
        assert!(validate_local_transport("host=/tmp dbname=rewards").is_ok());
    }

    #[test]
    fn history_limit_accepts_the_documented_bounds() {
        for limit in ["1", "100"] {
            let cli = Cli::try_parse_from([
                "reward",
                "--tenant",
                "tenant",
                "history",
                "--participant-hash",
                "sha256:abc",
                "--limit",
                limit,
            ]);
            assert!(cli.is_ok(), "limit {limit} should parse");
        }
    }

    #[test]
    fn history_limit_rejects_outside_the_documented_bounds() {
        for limit in ["0", "101"] {
            let cli = Cli::try_parse_from([
                "reward",
                "--tenant",
                "tenant",
                "history",
                "--participant-hash",
                "sha256:abc",
                "--limit",
                limit,
            ]);
            assert!(cli.is_err(), "limit {limit} should be rejected");
        }
    }

    #[test]
    fn review_requires_an_explicit_accept_value() {
        let result = Cli::try_parse_from([
            "reward",
            "--tenant",
            "tenant",
            "review",
            "--reservation",
            "00000000-0000-0000-0000-000000000001",
            "--decision",
            "00000000-0000-0000-0000-000000000002",
            "--reason",
            "completion_verified",
        ]);
        assert!(result.is_err());
    }

    #[test]
    fn review_accepts_explicit_true_and_false_values() {
        for accept in ["true", "false"] {
            let result = Cli::try_parse_from([
                "reward",
                "--tenant",
                "tenant",
                "review",
                "--reservation",
                "00000000-0000-0000-0000-000000000001",
                "--decision",
                "00000000-0000-0000-0000-000000000002",
                "--accept",
                accept,
                "--reason",
                "completion_verified",
            ]);
            assert!(result.is_ok(), "--accept {accept} should parse");
        }
    }

    #[test]
    fn terms_reader_rejects_a_file_larger_than_the_byte_limit() {
        let path = std::env::temp_dir().join(format!("reward-terms-{}.json", Uuid::new_v4()));
        std::fs::write(&path, vec![b' '; MAX_TERMS_BYTES as usize + 1]).unwrap();
        let result = read_terms(&path);
        let _ = std::fs::remove_file(&path);
        assert!(matches!(result, Err(CliError::TermsTooLarge)));
    }
}
