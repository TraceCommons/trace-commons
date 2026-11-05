use crate::{
    config::ConfigStore,
    feature_flags::{self, Environment, Snapshot},
};
use anyhow::{Context, Result};
use clap::{Args, Subcommand};
use std::path::PathBuf;
#[derive(Args)]
pub(super) struct FlagArgs {
    #[command(subcommand)]
    action: Option<Action>,
}
#[derive(Subcommand)]
enum Action {
    /// Show the immutable build profile (does not read local state).
    BuildProfile,
    /// Validate a local manifest without fetching or publishing it.
    Validate {
        #[arg(long)]
        file: PathBuf,
        #[arg(long,value_parser=["production","staging","testing"])]
        environment: String,
    },
}
pub(super) fn run(args: &FlagArgs, json: bool, directory: Option<PathBuf>) -> Result<()> {
    match &args.action {
        Some(Action::BuildProfile) => {
            println!(
                "{}",
                serde_json::to_string(&feature_flags::build_profile_identity()?)?
            );
        }
        Some(Action::Validate { file, environment }) => {
            use std::io::Read;
            let env = match environment.as_str() {
                "production" => Environment::Production,
                "staging" => Environment::Staging,
                _ => Environment::Testing,
            };
            let mut bytes = Vec::new();
            std::fs::File::open(file)?
                .take(feature_flags::MAX_MANIFEST_BYTES as u64 + 1)
                .read_to_end(&mut bytes)?;
            let manifest = feature_flags::parse_manifest(&bytes, env, chrono::Utc::now())?;
            let registry = feature_flags::registry()?;
            anyhow::ensure!(
                manifest
                    .flags
                    .keys()
                    .all(|id| registry.flags.contains_key(id)),
                "unknown-feature-flag"
            );
            let profile = feature_flags::profile_for_environment(env)?;
            let resolved =
                feature_flags::evaluate(&registry, &profile, Some(&manifest), chrono::Utc::now());
            anyhow::ensure!(
                manifest.flags.iter().all(|(id, enabled)| !*enabled
                    || resolved.get(id).is_some_and(|decision| decision.enabled)),
                "feature-unavailable-in-profile"
            );
            println!("{}", serde_json::to_string(&manifest)?);
        }
        None => {
            let store = ConfigStore::resolve_existing(directory)
                .context("feature flags state directory unavailable")?;
            let snapshot = match crate::daemon::client::try_call(
                &store,
                "feature_flags_snapshot",
                &serde_json::json!({}),
            )? {
                Some(reply) => {
                    anyhow::ensure!(
                        reply.error.is_none(),
                        "running daemon does not support feature flags"
                    );
                    let snapshot: Snapshot =
                        serde_json::from_value(reply.result.context("missing feature snapshot")?)?;
                    let identity = feature_flags::build_profile_identity()?;
                    anyhow::ensure!(
                        snapshot.schema_version == 1
                            && snapshot.profile_digest == identity.profile_digest
                            && snapshot.environment == identity.environment,
                        "running daemon feature profile mismatch"
                    );
                    snapshot
                }
                None => feature_flags::Runtime::for_store(&store)?.snapshot(),
            };
            if json {
                println!("{}", serde_json::to_string(&snapshot)?);
            } else {
                println!(
                    "Feature environment: {}\nBuild profile: {}\nConfiguration: {}\nRevision: {}\nSampled at: {}",
                    snapshot.environment.as_str(),
                    snapshot.profile_digest,
                    snapshot.source,
                    snapshot
                        .revision
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "none".into()),
                    snapshot.sampled_at
                );
                for (id, d) in snapshot.flags {
                    println!(
                        "{id}: {}{}{}",
                        if d.enabled { "on" } else { "off" },
                        if d.enabled != d.pending {
                            " (daemon restart required)"
                        } else {
                            ""
                        },
                        if d.capability_blocked {
                            " (unavailable in this build)"
                        } else {
                            ""
                        }
                    );
                }
            }
        }
    }
    Ok(())
}
