// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Repair proven native-client upload ownership without rewriting credit history.

use std::process::ExitCode;

use clap::{Parser, error::ErrorKind};
use serde::Serialize;
use sha2::{Digest, Sha256};
use trace_commons_server::account_trust::is_device_key_id;
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::{Database, postgres::PgBackend};

#[derive(Parser)]
#[command(
    name = "trace-commons-repair-upload-ownership",
    about = "Associate proven native upload identities with their registered device.",
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
struct Cli {
    /// Tenant containing the registered device and historical submissions.
    #[arg(long, env = "TC_REPAIR_TENANT_ID", hide_env_values = true)]
    tenant_id: String,
    /// Registered device key id (sha256:<64 lowercase hex digits>).
    #[arg(long, env = "TC_REPAIR_DEVICE_KEY_ID", hide_env_values = true)]
    device_key_id: String,
    /// Persist proven associations; existing settled credit may become payable.
    /// Default is a read-only dry run.
    #[arg(long)]
    apply: bool,
}

impl Cli {
    fn dry_run(&self) -> bool {
        !self.apply
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.tenant_id.trim().is_empty() || self.tenant_id != self.tenant_id.trim() {
            return Err("upload_ownership_repair_tenant_invalid");
        }
        if !is_device_key_id(&self.device_key_id) {
            return Err("upload_ownership_repair_device_invalid");
        }
        Ok(())
    }
}

#[derive(Serialize)]
struct RepairReport {
    dry_run: bool,
    matched_submissions: u64,
    tenant_hash: String,
    device_hash: String,
}

fn identity_hash(value: &str) -> String {
    format!("sha256:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

fn repair_report(args: &Cli, matched_submissions: u64) -> RepairReport {
    RepairReport {
        dry_run: args.dry_run(),
        matched_submissions,
        tenant_hash: identity_hash(&args.tenant_id),
        device_hash: identity_hash(&args.device_key_id),
    }
}

async fn run(args: &Cli) -> Result<RepairReport, &'static str> {
    args.validate()?;
    // Keep connection credentials out of command arguments, reports and errors.
    let database_url = std::env::var("DATABASE_URL")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or("upload_ownership_repair_database_unconfigured")?;
    let config = DatabaseConfig::from_postgres_url(&database_url, 1);
    let db = PgBackend::new(&config)
        .await
        .map_err(|_| "upload_ownership_repair_database_unavailable")?;
    // Schema changes are a deployment step, never an implicit part of a repair.
    let matched = db
        .repair_native_device_upload_principal(&args.tenant_id, &args.device_key_id, args.dry_run())
        .await
        .map_err(|_| "upload_ownership_repair_refused")?;
    Ok(repair_report(args, matched))
}

#[tokio::main]
async fn main() -> ExitCode {
    let args = match Cli::try_parse() {
        Ok(args) => args,
        Err(error)
            if matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            ) =>
        {
            let _ = error.print();
            return ExitCode::SUCCESS;
        }
        Err(_) => {
            eprintln!("upload_ownership_repair_arguments_invalid; use --help");
            return ExitCode::FAILURE;
        }
    };
    match run(&args).await {
        Ok(report) => match serde_json::to_string(&report) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(_) => {
                eprintln!("upload_ownership_repair_report_failed");
                ExitCode::FAILURE
            }
        },
        Err(label) => {
            eprintln!("{label}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(extra: &[&str]) -> Cli {
        let mut values = vec![
            "repair",
            "--tenant-id",
            "tenant-private",
            "--device-key-id",
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ];
        values.extend_from_slice(extra);
        Cli::try_parse_from(values).unwrap()
    }

    #[test]
    fn report_contains_counts_and_hashes_without_raw_identity() {
        let args = args(&[]);
        let report = serde_json::to_value(repair_report(&args, 13)).unwrap();
        assert_eq!(report["matched_submissions"], 13);
        assert_eq!(report["dry_run"], true);
        let encoded = report.to_string();
        assert!(!encoded.contains(&args.tenant_id));
        assert!(!encoded.contains(&args.device_key_id));
    }

    #[test]
    fn malformed_device_refuses_before_database_access() {
        let mut args = args(&[]);
        args.device_key_id = "untrusted-private-input".into();
        assert_eq!(
            args.validate(),
            Err("upload_ownership_repair_device_invalid")
        );
    }

    #[test]
    fn historical_repair_is_read_only_unless_apply_is_explicit() {
        assert!(args(&[]).dry_run());
        assert!(!args(&["--apply"]).dry_run());
    }
}
