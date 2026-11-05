// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-witness-list`: build, sign and check the signed witness
//! accepted-measurements list (spec 2026-11-05, section 7; runbook
//! `docs/operator/witness-accepted-list.md`).
//!
//! The list is signed by a Google Cloud KMS `ec-sign-ed25519` key (owner
//! decision Q11). There is no key file: `sign` asks `gcloud kms
//! asymmetric-sign` to sign the payload bytes, then checks the signature
//! against the key's exported public key before writing anything.
//!
//! - `payload`: an operator-written list (any formatting) to the canonical
//!   payload bytes that are signed. Every entry's measurement set and
//!   signing address is parsed with the same parsers the client uses.
//! - `sign`: sign a payload with the KMS key and write the served document,
//!   or add a signature to an existing one (key rotation).
//! - `verify`: check a served document against one or more public keys, as
//!   a client would.
//! - `public-key`: convert the KMS key's exported PEM into the 64-hex raw
//!   key compiled into clients, and print its key id.
//! - `configuration-payload`, `configuration-sign`, `configuration-verify`:
//!   the corresponding workflow for signed witness endpoint configuration,
//!   retaining enrollment anchors while publishing preferred/fallback endpoints.
//!
//! Output is labels and counts only; no path, URL or key material besides
//! the public key `public-key` is asked for.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, bail};
use base64::Engine as _;
use clap::{Parser, Subcommand};
use trace_commons_attestation::address::decode_address;
use trace_commons_attestation::measurements::ExpectedMeasurements;
use trace_commons_protocol::witness_accepted_list::{
    AcceptedList, ListSignature, assemble_signed_list, check_served_shape, key_id_for,
    parse_public_key_hex, verify_signed_list,
};
use trace_commons_protocol::witness_configuration::{
    WitnessConfiguration, check_configuration_served_shape, verify_signed_configuration,
};

/// The DER prefix of an Ed25519 SubjectPublicKeyInfo (RFC 8410): the raw
/// 32-byte key follows it.
const ED25519_SPKI_PREFIX: [u8; 12] = [
    0x30, 0x2a, 0x30, 0x05, 0x06, 0x03, 0x2b, 0x65, 0x70, 0x03, 0x21, 0x00,
];

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-witness-list",
    about = "Build, sign and verify the signed witness accepted-measurements list"
)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate an operator-written list and write the canonical payload.
    Payload {
        /// The list as JSON (`schema`, `version`, `issued_at`, `witness_url`,
        /// `entries`).
        #[arg(long)]
        input: PathBuf,
        /// Where to write the payload bytes that are signed.
        #[arg(long)]
        output: PathBuf,
    },
    /// Sign a payload with the KMS key and write the served document.
    Sign {
        #[command(flatten)]
        args: SignArgs,
    },
    /// Verify a served document as a client would.
    Verify {
        /// The served document.
        #[arg(long)]
        signed: PathBuf,
        /// A trusted public key as exported by KMS (PEM). Repeatable.
        #[arg(long = "public-key-pem")]
        public_key_pem: Vec<PathBuf>,
        /// A trusted public key as 64 hex characters. Repeatable.
        #[arg(long = "public-key-hex")]
        public_key_hex: Vec<String>,
        /// Also require the list to name this witness URL.
        #[arg(long)]
        witness_url: Option<String>,
    },
    /// Print the raw public key (64 hex) and key id of a KMS-exported PEM.
    PublicKey {
        #[arg(long)]
        pem: PathBuf,
    },
    /// Validate endpoint configuration and write the canonical payload.
    ConfigurationPayload {
        #[arg(long)]
        input: PathBuf,
        #[arg(long)]
        output: PathBuf,
    },
    /// Sign endpoint configuration with the same KMS list-signing key.
    ConfigurationSign {
        #[command(flatten)]
        args: SignArgs,
    },
    /// Verify signed endpoint configuration as a client would.
    ConfigurationVerify {
        #[arg(long)]
        signed: PathBuf,
        #[arg(long = "public-key-pem")]
        public_key_pem: Vec<PathBuf>,
        #[arg(long = "public-key-hex")]
        public_key_hex: Vec<String>,
        /// Also require this enrollment URL among the retained anchors.
        #[arg(long)]
        witness_url: Option<String>,
    },
}

#[derive(Debug, clap::Args)]
struct SignArgs {
    /// The payload written by `payload`.
    #[arg(long)]
    payload: PathBuf,
    /// The KMS key version's public key (`gcloud kms keys versions
    /// get-public-key ... --output-file`), used to check the signature
    /// before anything is written.
    #[arg(long = "public-key-pem")]
    public_key_pem: PathBuf,
    /// KMS location of the key ring.
    #[arg(long)]
    location: String,
    /// KMS key ring.
    #[arg(long)]
    keyring: String,
    /// KMS key.
    #[arg(long)]
    key: String,
    /// KMS key version.
    #[arg(long = "key-version")]
    key_version: String,
    /// GCP project, if not gcloud's configured one.
    #[arg(long)]
    project: Option<String>,
    /// Add the signature to this existing served document (same payload)
    /// instead of starting a new one. For a key rotation.
    #[arg(long = "append-to")]
    append_to: Option<PathBuf>,
    /// Where to write the served document.
    #[arg(long)]
    output: PathBuf,
    /// The gcloud executable.
    #[arg(long, default_value = "gcloud")]
    gcloud: PathBuf,
}

fn main() -> Result<()> {
    match Cli::parse().command {
        Commands::Payload { input, output } => {
            let list = build_payload(&input, &output)?;
            println!(
                "WitnessListPayloadOK: version={} entries={}",
                list.version,
                list.entries.len()
            );
        }
        Commands::Sign { args } => {
            let (list, signatures) = sign(&args)?;
            println!(
                "WitnessListSignOK: version={} entries={} signatures={signatures}",
                list.version,
                list.entries.len()
            );
        }
        Commands::Verify {
            signed,
            public_key_pem,
            public_key_hex,
            witness_url,
        } => {
            let mut keys = Vec::new();
            for path in &public_key_pem {
                keys.push(public_key_from_pem_file(path)?);
            }
            for hex_key in &public_key_hex {
                keys.push(parse_public_key_hex(hex_key).context("public_key_hex_invalid")?);
            }
            let list = verify(&signed, &keys, witness_url.as_deref())?;
            let now = chrono::Utc::now();
            println!(
                "WitnessListVerifyOK: version={} entries={} valid_now={}",
                list.version,
                list.entries.len(),
                list.entries_valid_at(now).count()
            );
        }
        Commands::PublicKey { pem } => {
            let key = public_key_from_pem_file(&pem)?;
            println!("public_key_hex={}", hex::encode(key));
            println!("key_id={}", key_id_for(&key));
        }
        Commands::ConfigurationPayload { input, output } => {
            let configuration = build_configuration_payload(&input, &output)?;
            println!(
                "WitnessConfigurationPayloadOK: version={} endpoints={}",
                configuration.version,
                configuration.endpoints.len()
            );
        }
        Commands::ConfigurationSign { args } => {
            let (configuration, signatures) = sign_configuration(&args)?;
            println!(
                "WitnessConfigurationSignOK: version={} endpoints={} signatures={signatures}",
                configuration.version,
                configuration.endpoints.len()
            );
        }
        Commands::ConfigurationVerify {
            signed,
            public_key_pem,
            public_key_hex,
            witness_url,
        } => {
            let mut keys = Vec::new();
            for path in &public_key_pem {
                keys.push(public_key_from_pem_file(path)?);
            }
            for hex_key in &public_key_hex {
                keys.push(parse_public_key_hex(hex_key).context("public_key_hex_invalid")?);
            }
            let configuration = verify_configuration(&signed, &keys, witness_url.as_deref())?;
            println!(
                "WitnessConfigurationVerifyOK: version={} endpoints={}",
                configuration.version,
                configuration.endpoints.len()
            );
        }
    }
    Ok(())
}

/// Read, validate and canonicalise an operator-written list.
fn build_payload(input: &Path, output: &Path) -> Result<AcceptedList> {
    let text = std::fs::read_to_string(input).context("list_input_unreadable")?;
    let list: AcceptedList = serde_json::from_str(&text).context("list_input_malformed")?;
    check_entries(&list)?;
    let payload = list
        .canonical_payload()
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    std::fs::write(output, payload.as_bytes()).context("payload_write_failed")?;
    Ok(list)
}

/// The client-side parsers, run before anything is signed, so a list no
/// client would accept never reaches the signing key.
fn check_entries(list: &AcceptedList) -> Result<()> {
    for (index, entry) in list.entries.iter().enumerate() {
        match ExpectedMeasurements::from_env_value(Some(&entry.measurements)) {
            Ok(Some(_)) => {}
            _ => bail!("entry {index}: measurements_malformed"),
        }
        if decode_address(&entry.signing_address).is_none() {
            bail!("entry {index}: signing_address_malformed");
        }
    }
    Ok(())
}

/// The arguments `gcloud kms asymmetric-sign` is run with. No
/// `--digest-algorithm`: an Ed25519 key signs the message itself.
fn gcloud_sign_args(args: &SignArgs, signature_file: &Path) -> Vec<String> {
    let mut out = vec![
        "kms".to_string(),
        "asymmetric-sign".to_string(),
        format!("--location={}", args.location),
        format!("--keyring={}", args.keyring),
        format!("--key={}", args.key),
        format!("--version={}", args.key_version),
        format!("--input-file={}", args.payload.display()),
        format!("--signature-file={}", signature_file.display()),
    ];
    if let Some(project) = &args.project {
        out.push(format!("--project={project}"));
    }
    out
}

/// A signature file as gcloud wrote it: 64 raw bytes, or their base64.
fn decode_signature_file(bytes: &[u8]) -> Result<[u8; 64]> {
    if let Ok(raw) = <[u8; 64]>::try_from(bytes) {
        return Ok(raw);
    }
    let text = std::str::from_utf8(bytes).context("signature_file_malformed")?;
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(text.trim())
        .context("signature_file_malformed")?;
    <[u8; 64]>::try_from(decoded.as_slice())
        .map_err(|_| anyhow::anyhow!("signature_file_malformed"))
}

/// Sign `args.payload` with the KMS key and write the served document.
/// Returns the list and the number of signatures the document now carries.
fn sign(args: &SignArgs) -> Result<(AcceptedList, usize)> {
    let payload = std::fs::read_to_string(&args.payload).context("payload_unreadable")?;
    // The payload must already be what `payload` writes: canonical and
    // valid. Signing anything else would publish bytes nobody reviewed.
    let parsed: AcceptedList = serde_json::from_str(&payload).context("payload_malformed")?;
    check_entries(&parsed)?;
    if parsed
        .canonical_payload()
        .map_err(|error| anyhow::anyhow!(error.label()))?
        != payload
    {
        bail!("payload_not_canonical");
    }
    let public_key = public_key_from_pem_file(&args.public_key_pem)?;

    let mut signatures = match &args.append_to {
        Some(existing) => {
            let bytes = std::fs::read(existing).context("existing_list_unreadable")?;
            check_served_shape(&bytes).map_err(|error| anyhow::anyhow!(error.label()))?;
            let value: trace_commons_protocol::witness_accepted_list::SignedAcceptedList =
                serde_json::from_slice(&bytes).context("existing_list_malformed")?;
            if value.payload != payload {
                bail!("existing_list_payload_differs");
            }
            value.signatures
        }
        None => Vec::new(),
    };
    let key_id = key_id_for(&public_key);
    if signatures
        .iter()
        .any(|signature| signature.key_id == key_id)
    {
        bail!("already_signed_by_this_key");
    }

    let scratch = tempfile::tempdir().context("scratch_unavailable")?;
    let signature_file = scratch.path().join("signature");
    let status = Command::new(&args.gcloud)
        .args(gcloud_sign_args(args, &signature_file))
        .status()
        .context("gcloud_unavailable")?;
    if !status.success() {
        bail!("gcloud_sign_failed");
    }
    let signature =
        decode_signature_file(&std::fs::read(&signature_file).context("signature_file_missing")?)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(payload.as_bytes(), &signature)
        .map_err(|_| anyhow::anyhow!("signature_does_not_match_public_key"))?;

    signatures.push(ListSignature {
        key_id,
        signature: hex::encode(signature),
    });
    let count = signatures.len();
    let document = assemble_signed_list(payload, signatures);
    let list = verify_signed_list(&document, &[public_key])
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    std::fs::write(&args.output, &document).context("signed_list_write_failed")?;
    Ok((list, count))
}

/// Verify a served document as a client would, under `keys`.
fn verify(path: &Path, keys: &[[u8; 32]], witness_url: Option<&str>) -> Result<AcceptedList> {
    if keys.is_empty() {
        bail!("no_public_key_given");
    }
    let bytes = std::fs::read(path).context("signed_list_unreadable")?;
    let list = verify_signed_list(&bytes, keys).map_err(|error| anyhow::anyhow!(error.label()))?;
    check_entries(&list)?;
    if let Some(url) = witness_url
        && !list.names_witness(url)
    {
        bail!("witness_url_mismatch");
    }
    Ok(list)
}

/// Validate with the same measurement/address parsers used by contributors.
fn check_configuration_entries(configuration: &WitnessConfiguration) -> Result<()> {
    for (endpoint_index, endpoint) in configuration.endpoints.iter().enumerate() {
        for (entry_index, entry) in endpoint.entries.iter().enumerate() {
            match ExpectedMeasurements::from_env_value(Some(&entry.measurements)) {
                Ok(Some(_)) => {}
                _ => bail!("endpoint {endpoint_index} entry {entry_index}: measurements_malformed"),
            }
            if decode_address(&entry.signing_address).is_none() {
                bail!("endpoint {endpoint_index} entry {entry_index}: signing_address_malformed");
            }
        }
    }
    Ok(())
}

fn build_configuration_payload(input: &Path, output: &Path) -> Result<WitnessConfiguration> {
    let text = std::fs::read_to_string(input).context("configuration_input_unreadable")?;
    let configuration: WitnessConfiguration =
        serde_json::from_str(&text).context("configuration_input_malformed")?;
    check_configuration_entries(&configuration)?;
    let payload = configuration
        .canonical_payload()
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    check_configuration_publication_size(&payload, &[])?;
    std::fs::write(output, payload.as_bytes()).context("payload_write_failed")?;
    Ok(configuration)
}

/// Reserve the exact serialized space of one Ed25519 signature, including
/// payload escaping, before writing a payload or invoking KMS. These zero
/// bytes only measure publication shape and are never treated as authentic.
fn check_configuration_publication_size(payload: &str, existing: &[ListSignature]) -> Result<()> {
    let mut signatures = existing.to_vec();
    signatures.push(ListSignature {
        key_id: "0".repeat(16),
        signature: "0".repeat(128),
    });
    check_configuration_served_shape(&assemble_signed_list(payload.into(), signatures))
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    Ok(())
}

/// Sign the exact reviewed canonical payload and locally verify before writing.
fn sign_configuration(args: &SignArgs) -> Result<(WitnessConfiguration, usize)> {
    let payload = std::fs::read_to_string(&args.payload).context("payload_unreadable")?;
    let configuration: WitnessConfiguration =
        serde_json::from_str(&payload).context("payload_malformed")?;
    check_configuration_entries(&configuration)?;
    if configuration
        .canonical_payload()
        .map_err(|error| anyhow::anyhow!(error.label()))?
        != payload
    {
        bail!("payload_not_canonical");
    }
    let public_key = public_key_from_pem_file(&args.public_key_pem)?;
    let mut signatures = match &args.append_to {
        Some(existing) => {
            let bytes = std::fs::read(existing).context("existing_configuration_unreadable")?;
            check_configuration_served_shape(&bytes)
                .map_err(|error| anyhow::anyhow!(error.label()))?;
            let signed: trace_commons_protocol::witness_accepted_list::SignedAcceptedList =
                serde_json::from_slice(&bytes).context("existing_configuration_malformed")?;
            if signed.payload != payload {
                bail!("existing_configuration_payload_differs");
            }
            signed.signatures
        }
        None => Vec::new(),
    };
    let key_id = key_id_for(&public_key);
    if signatures
        .iter()
        .any(|signature| signature.key_id == key_id)
    {
        bail!("already_signed_by_this_key");
    }
    check_configuration_publication_size(&payload, &signatures)?;
    let scratch = tempfile::tempdir().context("scratch_unavailable")?;
    let signature_file = scratch.path().join("signature");
    let status = Command::new(&args.gcloud)
        .args(gcloud_sign_args(args, &signature_file))
        .status()
        .context("gcloud_unavailable")?;
    if !status.success() {
        bail!("gcloud_sign_failed");
    }
    let signature =
        decode_signature_file(&std::fs::read(&signature_file).context("signature_file_missing")?)?;
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key)
        .verify(payload.as_bytes(), &signature)
        .map_err(|_| anyhow::anyhow!("signature_does_not_match_public_key"))?;
    signatures.push(ListSignature {
        key_id,
        signature: hex::encode(signature),
    });
    let count = signatures.len();
    let document = assemble_signed_list(payload, signatures);
    let configuration = verify_signed_configuration(&document, &[public_key])
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    std::fs::write(&args.output, &document).context("signed_configuration_write_failed")?;
    Ok((configuration, count))
}

fn verify_configuration(
    path: &Path,
    keys: &[[u8; 32]],
    witness_url: Option<&str>,
) -> Result<WitnessConfiguration> {
    if keys.is_empty() {
        bail!("no_public_key_given");
    }
    let bytes = std::fs::read(path).context("signed_configuration_unreadable")?;
    let configuration = verify_signed_configuration(&bytes, keys)
        .map_err(|error| anyhow::anyhow!(error.label()))?;
    check_configuration_entries(&configuration)?;
    if let Some(url) = witness_url
        && !configuration.names_witness(url)
    {
        bail!("witness_url_mismatch");
    }
    Ok(configuration)
}

fn public_key_from_pem_file(path: &Path) -> Result<[u8; 32]> {
    public_key_from_pem(&std::fs::read_to_string(path).context("public_key_unreadable")?)
}

/// The raw Ed25519 key in a `PUBLIC KEY` PEM (SubjectPublicKeyInfo), as
/// `gcloud kms keys versions get-public-key` writes it.
fn public_key_from_pem(pem: &str) -> Result<[u8; 32]> {
    let body: String = pem
        .lines()
        .map(str::trim)
        .skip_while(|line| *line != "-----BEGIN PUBLIC KEY-----")
        .skip(1)
        .take_while(|line| *line != "-----END PUBLIC KEY-----")
        .collect();
    let der = base64::engine::general_purpose::STANDARD
        .decode(body)
        .context("public_key_pem_malformed")?;
    let raw = der
        .strip_prefix(&ED25519_SPKI_PREFIX)
        .context("public_key_not_ed25519")?;
    <[u8; 32]>::try_from(raw).map_err(|_| anyhow::anyhow!("public_key_not_ed25519"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    use trace_commons_protocol::witness_accepted_list::WITNESS_ACCEPTED_LIST_SCHEMA;

    /// A throwaway key for this test run only. Never a real list key.
    fn test_key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn pem_of(key: &Ed25519KeyPair) -> String {
        let mut der = ED25519_SPKI_PREFIX.to_vec();
        der.extend_from_slice(key.public_key().as_ref());
        format!(
            "-----BEGIN PUBLIC KEY-----\n{}\n-----END PUBLIC KEY-----\n",
            base64::engine::general_purpose::STANDARD.encode(der)
        )
    }

    fn operator_list() -> serde_json::Value {
        // Written the way an operator would: pretty, keys in any order.
        serde_json::json!({
            "version": 3,
            "schema": WITNESS_ACCEPTED_LIST_SCHEMA,
            "witness_url": "https://witness.example",
            "issued_at": "2026-11-05T00:00:00Z",
            "entries": [{
                "signing_address": format!("0x{}", "11".repeat(20)),
                "measurements": format!("mrtd={},mrconfigid={}", "aa".repeat(48), "bb".repeat(48)),
                "not_before": "2026-11-05T00:00:00Z",
                "not_after": "2027-11-05T00:00:00Z",
                "commit": "85173de19"
            }]
        })
    }

    fn operator_configuration() -> serde_json::Value {
        let list = operator_list();
        serde_json::json!({
            "schema": trace_commons_protocol::witness_configuration::WITNESS_CONFIGURATION_SCHEMA,
            "version": 7,
            "issued_at": list["issued_at"],
            "enrollment_urls": ["https://old.example", "https://witness.example"],
            "endpoints": [{"url": "https://witness.example", "entries": list["entries"]}]
        })
    }

    fn sign_args(dir: &Path, gcloud: PathBuf) -> SignArgs {
        SignArgs {
            payload: dir.join("payload.json"),
            public_key_pem: dir.join("public.pem"),
            location: "LOCATION".into(),
            keyring: "KEYRING".into(),
            key: "KEY".into(),
            key_version: "1".into(),
            project: Some("PROJECT".into()),
            append_to: None,
            output: dir.join("signed.json"),
            gcloud,
        }
    }

    #[test]
    fn a_kms_pem_converts_to_the_raw_key_clients_compile_in() {
        let key = test_key();
        let raw = public_key_from_pem(&pem_of(&key)).unwrap();
        assert_eq!(raw.as_slice(), key.public_key().as_ref());
        assert!(
            public_key_from_pem("-----BEGIN PUBLIC KEY-----\nAAAA\n-----END PUBLIC KEY-----")
                .is_err()
        );
    }

    #[test]
    fn a_signature_file_is_read_raw_or_base64() {
        let raw = [7u8; 64];
        assert_eq!(decode_signature_file(&raw).unwrap(), raw);
        let encoded = base64::engine::general_purpose::STANDARD.encode(raw);
        assert_eq!(decode_signature_file(encoded.as_bytes()).unwrap(), raw);
        assert_eq!(
            decode_signature_file(format!("{encoded}\n").as_bytes()).unwrap(),
            raw
        );
        assert!(decode_signature_file(b"short").is_err());
    }

    #[test]
    fn gcloud_signs_the_raw_payload_with_no_digest_step() {
        let dir = tempfile::tempdir().unwrap();
        let args = sign_args(dir.path(), "gcloud".into());
        let argv = gcloud_sign_args(&args, Path::new("/scratch/signature"));
        assert_eq!(&argv[..2], ["kms", "asymmetric-sign"]);
        assert!(argv.contains(&"--key=KEY".to_string()));
        assert!(argv.contains(&"--version=1".to_string()));
        assert!(argv.contains(&"--project=PROJECT".to_string()));
        assert!(argv.contains(&"--signature-file=/scratch/signature".to_string()));
        assert!(!argv.iter().any(|arg| arg.starts_with("--digest-algorithm")));
    }

    #[test]
    fn a_list_no_client_would_accept_is_never_built() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("list.json");
        let output = dir.path().join("payload.json");
        for mutate in [
            |v: &mut serde_json::Value| v["entries"][0]["measurements"] = "mrtd=zz".into(),
            |v: &mut serde_json::Value| v["entries"][0]["measurements"] = "compose_hash=00".into(),
            |v: &mut serde_json::Value| {
                v["entries"][0]["signing_address"] = format!("0x{}", "zz".repeat(20)).into()
            },
            |v: &mut serde_json::Value| v["schema"] = "other".into(),
        ] {
            let mut list = operator_list();
            mutate(&mut list);
            std::fs::write(&input, serde_json::to_vec_pretty(&list).unwrap()).unwrap();
            assert!(build_payload(&input, &output).is_err());
        }
    }

    /// The whole operator path against a stand-in for gcloud that signs
    /// with a throwaway key: payload, sign, verify, and a second signature
    /// for a rotation.
    #[cfg(unix)]
    #[test]
    fn build_sign_and_verify_round_trip_through_a_stand_in_gcloud() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("list.json");
        std::fs::write(&input, serde_json::to_vec_pretty(&operator_list()).unwrap()).unwrap();
        let payload_path = dir.path().join("payload.json");
        build_payload(&input, &payload_path).unwrap();
        let payload = std::fs::read(&payload_path).unwrap();

        // The stand-in checks it was asked to sign exactly the payload file,
        // with no digest step, then writes the signature as base64 the way
        // gcloud's help describes.
        let stand_in = |key: &Ed25519KeyPair, name: &str| -> PathBuf {
            let signature =
                base64::engine::general_purpose::STANDARD.encode(key.sign(&payload).as_ref());
            let expected = dir.path().join(format!("{name}.expected"));
            std::fs::write(&expected, &payload).unwrap();
            let script = dir.path().join(name);
            std::fs::write(
                &script,
                format!(
                    "#!/bin/sh\nset -e\nin=''\nout=''\nfor a in \"$@\"; do\n  case \"$a\" in\n    --digest-algorithm*) exit 9;;\n    --input-file=*) in=\"${{a#--input-file=}}\";;\n    --signature-file=*) out=\"${{a#--signature-file=}}\";;\n  esac\ndone\ncmp -s \"$in\" '{}'\nprintf '%s' '{}' > \"$out\"\n",
                    expected.display(),
                    signature
                ),
            )
            .unwrap();
            std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
            script
        };

        let first = test_key();
        std::fs::write(dir.path().join("public.pem"), pem_of(&first)).unwrap();
        let args = sign_args(dir.path(), stand_in(&first, "gcloud-first"));
        let (list, count) = sign(&args).unwrap();
        assert_eq!((list.version, count), (3, 1));
        let first_raw = public_key_from_pem(&pem_of(&first)).unwrap();
        let verified = verify(&args.output, &[first_raw], Some("https://witness.example")).unwrap();
        assert_eq!(verified.entries.len(), 1);
        assert!(verify(&args.output, &[first_raw], Some("https://other.example")).is_err());
        assert!(verify(&args.output, &[], None).is_err());
        let stranger = public_key_from_pem(&pem_of(&test_key())).unwrap();
        assert!(verify(&args.output, &[stranger], None).is_err());

        // A rotation: the incoming key adds its signature to the same list.
        let second = test_key();
        let second_pem = dir.path().join("second.pem");
        std::fs::write(&second_pem, pem_of(&second)).unwrap();
        let mut rotate = sign_args(dir.path(), stand_in(&second, "gcloud-second"));
        rotate.public_key_pem = second_pem;
        rotate.append_to = Some(args.output.clone());
        rotate.output = dir.path().join("signed-both.json");
        let (_, count) = sign(&rotate).unwrap();
        assert_eq!(count, 2);
        let second_raw = public_key_from_pem(&pem_of(&second)).unwrap();
        assert!(verify(&rotate.output, &[first_raw], None).is_ok());
        assert!(verify(&rotate.output, &[second_raw], None).is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn a_signature_that_does_not_match_the_public_key_writes_nothing() {
        use std::os::unix::fs::PermissionsExt as _;
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("list.json");
        std::fs::write(&input, serde_json::to_vec(&operator_list()).unwrap()).unwrap();
        build_payload(&input, &dir.path().join("payload.json")).unwrap();
        std::fs::write(dir.path().join("public.pem"), pem_of(&test_key())).unwrap();
        // A stand-in that signs garbage.
        let script = dir.path().join("gcloud-bad");
        std::fs::write(
            &script,
            "#!/bin/sh\nfor a in \"$@\"; do case \"$a\" in --signature-file=*) head -c 64 /dev/zero > \"${a#--signature-file=}\";; esac; done\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let args = sign_args(dir.path(), script);
        let error = sign(&args).unwrap_err().to_string();
        assert_eq!(error, "signature_does_not_match_public_key");
        assert!(!args.output.exists());
    }

    #[test]
    fn a_hand_edited_payload_is_not_signed() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("list.json");
        std::fs::write(&input, serde_json::to_vec(&operator_list()).unwrap()).unwrap();
        let payload_path = dir.path().join("payload.json");
        build_payload(&input, &payload_path).unwrap();
        let pretty = serde_json::to_vec_pretty(
            &serde_json::from_slice::<serde_json::Value>(&std::fs::read(&payload_path).unwrap())
                .unwrap(),
        )
        .unwrap();
        std::fs::write(&payload_path, pretty).unwrap();
        std::fs::write(dir.path().join("public.pem"), pem_of(&test_key())).unwrap();
        let args = sign_args(dir.path(), "/nonexistent/gcloud".into());
        assert_eq!(
            sign(&args).unwrap_err().to_string(),
            "payload_not_canonical"
        );
    }

    #[test]
    fn configuration_commands_are_available_alongside_legacy_commands() {
        assert!(matches!(
            Cli::try_parse_from([
                "witness-list",
                "configuration-payload",
                "--input",
                "in",
                "--output",
                "out"
            ])
            .unwrap()
            .command,
            Commands::ConfigurationPayload { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "witness-list",
                "configuration-verify",
                "--signed",
                "in",
                "--witness-url",
                "https://old.example"
            ])
            .unwrap()
            .command,
            Commands::ConfigurationVerify { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "witness-list",
                "configuration-sign",
                "--payload",
                "in",
                "--public-key-pem",
                "pem",
                "--location",
                "loc",
                "--keyring",
                "ring",
                "--key",
                "key",
                "--key-version",
                "1",
                "--output",
                "out"
            ])
            .unwrap()
            .command,
            Commands::ConfigurationSign { .. }
        ));
        assert!(matches!(
            Cli::try_parse_from([
                "witness-list",
                "payload",
                "--input",
                "in",
                "--output",
                "out"
            ])
            .unwrap()
            .command,
            Commands::Payload { .. }
        ));
    }

    #[test]
    fn invalid_configuration_is_never_built_and_valid_withdrawal_is_canonical() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("configuration.json");
        let output = dir.path().join("payload.json");
        for mutate in [
            |v: &mut serde_json::Value| {
                v["endpoints"][0]["entries"][0]["measurements"] = "mrtd=zz".into()
            },
            |v: &mut serde_json::Value| {
                v["endpoints"][0]["entries"][0]["measurements"] = "compose_hash=00".into()
            },
            |v: &mut serde_json::Value| {
                v["endpoints"][0]["entries"][0]["signing_address"] = "0x00".into()
            },
            |v: &mut serde_json::Value| v["endpoints"][0]["url"] = "http://witness.example".into(),
            |v: &mut serde_json::Value| v["enrollment_urls"] = serde_json::json!([]),
            |v: &mut serde_json::Value| v["future"] = true.into(),
        ] {
            let mut configuration = operator_configuration();
            mutate(&mut configuration);
            std::fs::write(&input, serde_json::to_vec(&configuration).unwrap()).unwrap();
            assert!(build_configuration_payload(&input, &output).is_err());
            assert!(!output.exists());
        }
        let mut withdrawn = operator_configuration();
        withdrawn["endpoints"] = serde_json::json!([]);
        std::fs::write(&input, serde_json::to_vec_pretty(&withdrawn).unwrap()).unwrap();
        let configuration = build_configuration_payload(&input, &output).unwrap();
        assert!(configuration.endpoints.is_empty());
        assert_eq!(
            std::fs::read_to_string(&output).unwrap(),
            configuration.canonical_payload().unwrap()
        );
    }

    #[test]
    fn oversized_configuration_is_refused_before_writing_or_calling_kms() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("configuration.json");
        let output = dir.path().join("payload.json");
        let mut value = operator_configuration();
        let entry = value["endpoints"][0]["entries"][0].clone();
        value["enrollment_urls"] = serde_json::Value::Array(
            (0..8)
                .map(|i| format!("https://w{i}.example").into())
                .collect(),
        );
        value["endpoints"] = serde_json::Value::Array((0..8).map(|i| serde_json::json!({"url": format!("https://w{i}.example"), "entries": vec![entry.clone(); 32]})).collect());
        std::fs::write(&input, serde_json::to_vec(&value).unwrap()).unwrap();
        assert_eq!(
            build_configuration_payload(&input, &output)
                .unwrap_err()
                .to_string(),
            "witness-configuration-too-large"
        );
        assert!(!output.exists());
        let configuration: WitnessConfiguration = serde_json::from_value(value).unwrap();
        std::fs::write(&output, configuration.canonical_payload().unwrap()).unwrap();
        std::fs::write(dir.path().join("public.pem"), pem_of(&test_key())).unwrap();
        let args = sign_args(dir.path(), "/nonexistent/gcloud".into());
        assert_eq!(
            sign_configuration(&args).unwrap_err().to_string(),
            "witness-configuration-too-large"
        );
        assert!(!args.output.exists());
    }

    #[cfg(unix)]
    fn configuration_gcloud(
        dir: &Path,
        payload: &[u8],
        key: &Ed25519KeyPair,
        name: &str,
    ) -> PathBuf {
        use std::os::unix::fs::PermissionsExt as _;
        let signature =
            base64::engine::general_purpose::STANDARD.encode(key.sign(payload).as_ref());
        let expected = dir.join(format!("{name}.expected"));
        std::fs::write(&expected, payload).unwrap();
        let script = dir.join(name);
        std::fs::write(&script, format!(
            "#!/bin/sh\nset -e\nin=''\nout=''\nfor a in \"$@\"; do\n case \"$a\" in\n --digest-algorithm*) exit 9;;\n --input-file=*) in=\"${{a#--input-file=}}\";;\n --signature-file=*) out=\"${{a#--signature-file=}}\";;\n esac\ndone\ncmp -s \"$in\" '{}'\nprintf '%s' '{}' > \"$out\"\n", expected.display(), signature
        )).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        script
    }

    #[cfg(unix)]
    #[test]
    fn configuration_build_sign_verify_and_key_rotation_use_existing_kms_workflow() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("configuration.json");
        std::fs::write(
            &input,
            serde_json::to_vec_pretty(&operator_configuration()).unwrap(),
        )
        .unwrap();
        let payload_path = dir.path().join("payload.json");
        build_configuration_payload(&input, &payload_path).unwrap();
        let payload = std::fs::read(&payload_path).unwrap();
        let first = test_key();
        let first_raw = public_key_from_pem(&pem_of(&first)).unwrap();
        std::fs::write(dir.path().join("public.pem"), pem_of(&first)).unwrap();
        let args = sign_args(
            dir.path(),
            configuration_gcloud(dir.path(), &payload, &first, "first"),
        );
        let (configuration, count) = sign_configuration(&args).unwrap();
        assert_eq!((configuration.version, count), (7, 1));
        assert!(
            verify_configuration(&args.output, &[first_raw], Some("https://old.example/")).is_ok()
        );
        assert!(
            verify_configuration(
                &args.output,
                &[first_raw],
                Some("https://unrelated.example")
            )
            .is_err()
        );
        assert!(verify_configuration(&args.output, &[], None).is_err());
        assert!(verify(&args.output, &[first_raw], None).is_err());
        let stranger = public_key_from_pem(&pem_of(&test_key())).unwrap();
        assert!(verify_configuration(&args.output, &[stranger], None).is_err());

        let second = test_key();
        let second_raw = public_key_from_pem(&pem_of(&second)).unwrap();
        let second_pem = dir.path().join("second.pem");
        std::fs::write(&second_pem, pem_of(&second)).unwrap();
        let mut rotate = sign_args(
            dir.path(),
            configuration_gcloud(dir.path(), &payload, &second, "second"),
        );
        rotate.public_key_pem = second_pem;
        rotate.append_to = Some(args.output.clone());
        rotate.output = dir.path().join("signed-both.json");
        assert_eq!(sign_configuration(&rotate).unwrap().1, 2);
        assert!(verify_configuration(&rotate.output, &[first_raw], None).is_ok());
        assert!(verify_configuration(&rotate.output, &[second_raw], None).is_ok());
        rotate.append_to = Some(rotate.output.clone());
        rotate.output = dir.path().join("duplicate.json");
        assert_eq!(
            sign_configuration(&rotate).unwrap_err().to_string(),
            "already_signed_by_this_key"
        );
        assert!(!rotate.output.exists());
    }

    #[cfg(unix)]
    #[test]
    fn configuration_signing_checks_signature_payload_and_appended_document_before_writing() {
        let dir = tempfile::tempdir().unwrap();
        let input = dir.path().join("configuration.json");
        std::fs::write(
            &input,
            serde_json::to_vec(&operator_configuration()).unwrap(),
        )
        .unwrap();
        let payload_path = dir.path().join("payload.json");
        build_configuration_payload(&input, &payload_path).unwrap();
        let payload = std::fs::read(&payload_path).unwrap();
        let key = test_key();
        std::fs::write(dir.path().join("public.pem"), pem_of(&key)).unwrap();
        let mut args = sign_args(
            dir.path(),
            configuration_gcloud(dir.path(), &payload, &test_key(), "wrong-key"),
        );
        assert_eq!(
            sign_configuration(&args).unwrap_err().to_string(),
            "signature_does_not_match_public_key"
        );
        assert!(!args.output.exists());

        args.gcloud = configuration_gcloud(dir.path(), &payload, &key, "correct-key");
        let legacy_payload = serde_json::from_value::<AcceptedList>(operator_list())
            .unwrap()
            .canonical_payload()
            .unwrap();
        let legacy = assemble_signed_list(
            legacy_payload.clone(),
            vec![ListSignature {
                key_id: key_id_for(&public_key_from_pem(&pem_of(&key)).unwrap()),
                signature: hex::encode(key.sign(legacy_payload.as_bytes()).as_ref()),
            }],
        );
        let append_path = dir.path().join("legacy.json");
        std::fs::write(&append_path, legacy).unwrap();
        args.append_to = Some(append_path);
        assert!(sign_configuration(&args).is_err());
        assert!(!args.output.exists());

        args.append_to = None;
        let pretty = serde_json::to_vec_pretty(
            &serde_json::from_slice::<serde_json::Value>(&payload).unwrap(),
        )
        .unwrap();
        std::fs::write(&payload_path, pretty).unwrap();
        assert_eq!(
            sign_configuration(&args).unwrap_err().to_string(),
            "payload_not_canonical"
        );
        assert!(!args.output.exists());
    }
}
