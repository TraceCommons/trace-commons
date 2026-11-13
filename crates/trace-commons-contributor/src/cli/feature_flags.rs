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
    /// Validate a manifest, then write the exact bytes a key signs for it.
    SigningInput {
        #[arg(long)]
        file: PathBuf,
        #[arg(long,value_parser=["production","staging","testing"])]
        environment: String,
        #[arg(long)]
        output: PathBuf,
    },
    /// Check a signature against a key this build pins for the environment,
    /// then write the served document.
    Assemble {
        #[arg(long)]
        file: PathBuf,
        #[arg(long,value_parser=["production","staging","testing"])]
        environment: String,
        /// The signature as `gcloud kms asymmetric-sign` wrote it.
        #[arg(long)]
        signature: PathBuf,
        /// The signing key's public key, `PUBLIC KEY` PEM.
        #[arg(long)]
        public_key_pem: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Verify a served document exactly as a client of this build would,
    /// and print its manifest.
    Verify {
        #[arg(long)]
        file: PathBuf,
        #[arg(long,value_parser=["production","staging","testing"])]
        environment: String,
    },
    /// Print the raw hex and key id of a `PUBLIC KEY` PEM, the form a
    /// profile's `signing_keys` pins.
    PublicKey {
        #[arg(long)]
        pem: PathBuf,
    },
}
fn environment_of(name: &str) -> Environment {
    match name {
        "production" => Environment::Production,
        "staging" => Environment::Staging,
        _ => Environment::Testing,
    }
}
fn read_bounded(file: &std::path::Path) -> Result<Vec<u8>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(file)?
        .take(feature_flags::MAX_MANIFEST_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    anyhow::ensure!(
        bytes.len() <= feature_flags::MAX_MANIFEST_BYTES,
        "manifest too large"
    );
    Ok(bytes)
}
/// Everything about a manifest except its signature: shape, time, known
/// flags, and that every enable can take effect in this build's profile.
fn check_manifest(payload: &[u8], env: Environment) -> Result<feature_flags::Manifest> {
    let now = chrono::Utc::now();
    let manifest = feature_flags::decode_payload(payload, env)?;
    anyhow::ensure!(manifest.valid_at(now), "manifest not current");
    let registry = feature_flags::registry()?;
    anyhow::ensure!(
        manifest
            .flags
            .keys()
            .all(|id| registry.flags.contains_key(id)),
        "unknown-feature-flag"
    );
    let profile = feature_flags::profile_for_environment(env)?;
    let resolved = feature_flags::evaluate(&registry, &profile, Some(&manifest), now);
    anyhow::ensure!(
        manifest
            .flags
            .iter()
            .all(|(id, enabled)| !*enabled
                || resolved.get(id).is_some_and(|decision| decision.enabled)),
        "feature-unavailable-in-profile"
    );
    Ok(manifest)
}
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];
/// The raw Ed25519 key in a `PUBLIC KEY` PEM, as `gcloud kms keys versions
/// get-public-key --public-key-format pem` writes it.
fn public_key_from_pem(pem: &str) -> Result<[u8; 32]> {
    use base64::Engine as _;
    let body: String = pem
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "-----BEGIN PUBLIC KEY-----")
        .skip(1)
        .take_while(|line| *line != "-----END PUBLIC KEY-----")
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("public-key-pem-malformed")?;
    let raw = der
        .strip_prefix(&ED25519_SPKI_PREFIX)
        .context("public-key-not-ed25519")?;
    <[u8; 32]>::try_from(raw).map_err(|_| anyhow::anyhow!("public-key-not-ed25519"))
}
/// A signature file as gcloud wrote it: 64 raw bytes, or their base64.
fn decode_signature_file(bytes: &[u8]) -> Result<[u8; 64]> {
    use base64::Engine as _;
    if let Ok(raw) = <[u8; 64]>::try_from(bytes) {
        return Ok(raw);
    }
    let text = std::str::from_utf8(bytes).context("signature-file-malformed")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .context("signature-file-malformed")?;
    <[u8; 64]>::try_from(decoded.as_slice())
        .map_err(|_| anyhow::anyhow!("signature-file-malformed"))
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
            let manifest = check_manifest(&read_bounded(file)?, environment_of(environment))?;
            println!("{}", serde_json::to_string(&manifest)?);
        }
        Some(Action::SigningInput {
            file,
            environment,
            output,
        }) => {
            let payload = read_bounded(file)?;
            check_manifest(&payload, environment_of(environment))?;
            std::fs::write(output, feature_flags::signing_input(&payload))?;
        }
        Some(Action::Assemble {
            file,
            environment,
            signature,
            public_key_pem,
            output,
        }) => {
            let env = environment_of(environment);
            let payload = read_bounded(file)?;
            check_manifest(&payload, env)?;
            let public_key = public_key_from_pem(&std::fs::read_to_string(public_key_pem)?)?;
            let profile = feature_flags::profile_for_environment(env)?;
            anyhow::ensure!(
                profile.signing_keys.contains(&hex::encode(public_key)),
                "signing-key-not-pinned-for-environment"
            );
            let signature = decode_signature_file(&std::fs::read(signature)?)?;
            ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
                .verify(&feature_flags::signing_input(&payload), &signature)
                .map_err(|_| anyhow::anyhow!("signature-does-not-match-public-key"))?;
            let document = feature_flags::assemble_signed_manifest(
                String::from_utf8(payload).context("manifest is not UTF-8")?,
                vec![feature_flags::ManifestSignature {
                    key_id: feature_flags::key_id_for(&public_key),
                    signature: hex::encode(signature),
                }],
            );
            // Exactly the check a client makes, before anything is written.
            feature_flags::parse_manifest(&document, &profile, chrono::Utc::now())?;
            std::fs::write(output, &document)?;
        }
        Some(Action::Verify { file, environment }) => {
            let env = environment_of(environment);
            let profile = feature_flags::profile_for_environment(env)?;
            let payload = feature_flags::open_signed_manifest(&read_bounded(file)?, &profile)?;
            let manifest = check_manifest(payload.as_bytes(), env)?;
            println!("{}", serde_json::to_string(&manifest)?);
        }
        Some(Action::PublicKey { pem }) => {
            let key = public_key_from_pem(&std::fs::read_to_string(pem)?)?;
            println!(
                "public_key_hex={}\nkey_id={}",
                hex::encode(key),
                feature_flags::key_id_for(&key)
            );
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
