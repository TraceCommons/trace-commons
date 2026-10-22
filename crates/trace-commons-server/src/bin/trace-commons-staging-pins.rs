// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-staging-pins` -- observe NEAR AI staging's enclave
//! measurements, and sign the pin set a staging app trusts.
//!
//! A staging build of the contributor app takes the measurement sets it
//! checks NEAR AI quotes against from one signed document, served as a
//! static file by the staging site, and from nowhere else (staging design,
//! section 5). This tool is the other half: operators run it on their own
//! machine; nothing ships it. Built only with `near-ai-staging`, which
//! points the compiled-in NEAR AI base at staging.
//!
//! ```text
//! observe --model <id>... [--registers mrtd,rtmr0,...]    API key on stdin
//! sign --sets <file> --sequence <n> --ttl-days <d>
//!      (--current <served copy> | --first) [--out current.json]
//!                                                         signing key on stdin
//! verify <file> --public-key <hex>
//! public-key                                              signing key on stdin
//! ```
//!
//! - `observe` fetches each model's ed25519 attestation report from the
//!   staging gateway for a fresh nonce, verifies every quote against Intel
//!   collateral, requires an up-to-date TCB and the nonce bound inside the
//!   verified quote, and prints the distinct measurement sets, one per line.
//!   A set is never printed from a quote that did not verify, and one
//!   refused model prints nothing at all
//!   (`trace_commons_server::near_attestation::pin_observe`). Fetching
//!   collateral needs the `near-attestation-collateral` feature as well;
//!   without it `observe` refuses with that control's name.
//! - `sign` reads the signing key from stdin only -- no argument takes one,
//!   and it never writes one -- builds the payload, signs it, checks the
//!   result with the same verifier the app runs, and writes the document
//!   (refusing to overwrite a file). It refuses a sequence not above the
//!   served document's and a lifetime outside 1 to 14 days. On stdout it
//!   prints exactly one line, the staging ingest's own pin for the first set
//!   (`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS=...`, section 5.9).
//! - `verify` runs the app's check over a document, at the current time,
//!   against a public key.
//! - `public-key` prints the public key and its key id for the signing key
//!   on stdin, so the key a staging build compiles in can be read off the
//!   secret without signing anything.
//!
//! # The signing key
//!
//! 32 random bytes (an Ed25519 seed) as 64 hex characters, held in Secret
//! Manager as `tc-staging-nearai-pin-signing-key`, readable by the owner
//! and never by the staging host's runtime service account: a compromised
//! staging host can then serve a stale set, never forge one.
//! `docs/operator/staging-pin-signing.md` has the procedure.
//!
//! # The payload's gateway host
//!
//! `gateway_host` is the bare host (no scheme, no path) of the compiled-in
//! staging NEAR AI API base, `near_ai_api::NEAR_AI_API_BASE_URL`. The app
//! compares it, as a string, with the host of its own attestation gateway.
//!
//! # Output
//!
//! stdout carries only what the command produces: sets from `observe`, the
//! ingest pin line from `sign`, a summary from `verify`, the public key from
//! `public-key`. stderr carries labels, counts, the key id and `sha256:`
//! hashes -- never a key, a seed, a measurement or a document byte.

use std::io::{Read, Write as _};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use clap::{ArgGroup, Parser, Subcommand};
use ring::signature::{Ed25519KeyPair, KeyPair as _};
use secrecy::SecretString;
use serde::Serialize;
use trace_commons_attestation::measurements::ExpectedMeasurements;
use trace_commons_attestation::staging_pins::{self, CHANNEL, FORMAT, MAX_LIFETIME_SECS};
use trace_commons_server::near_ai_api::NEAR_AI_API_BASE_URL;
use trace_commons_server::near_attestation::client::{
    AttestedKeyReportClient, HttpAttestationClient, INTEL_PCS_URL,
};
use trace_commons_server::near_attestation::key_drift::RandomNonce;
use trace_commons_server::near_attestation::pin_observe::{
    observe_measurement_sets, parse_registers,
};
use zeroize::Zeroizing;

const DAY_SECS: u64 = 24 * 60 * 60;

/// The most stdin is read for a key: a seed is 64 characters, an API key
/// well under this. Anything longer is refused, not truncated.
const STDIN_LIMIT: u64 = 4096;

/// How long each NEAR AI or Intel call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(60);

/// The variable the staging ingest pins its own receipts with (5.9).
const INGEST_PIN_VARIABLE: &str = "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS";

#[derive(Debug, Parser)]
#[command(
    name = "trace-commons-staging-pins",
    about = "Observe NEAR AI staging's enclave measurements and sign the staging pin set"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Print the measurement sets NEAR AI staging serves the given models
    /// under, from verified quotes only. Reads a staging inference API key
    /// from stdin.
    Observe {
        /// A model id, as NEAR AI names it. Repeat for each model.
        #[arg(long = "model", required = true)]
        models: Vec<String>,
        /// The registers each set names, comma-separated.
        #[arg(long, default_value = "mrtd,rtmr0,rtmr1,rtmr2,rtmr3")]
        registers: String,
    },
    /// Sign a pin set. Reads the signing key from stdin.
    #[command(group(ArgGroup::new("served").required(true).args(["current", "first"])))]
    Sign {
        /// The sets to sign, one per line (what `observe` prints).
        #[arg(long)]
        sets: PathBuf,
        /// The new sequence; must be above the served document's.
        #[arg(long)]
        sequence: u64,
        /// How long the set is valid, in days (1 to 14).
        #[arg(long = "ttl-days")]
        ttl_days: u64,
        /// A copy of the document the staging site serves now.
        #[arg(long)]
        current: Option<PathBuf>,
        /// Nothing has been served yet.
        #[arg(long)]
        first: bool,
        /// Where to write the signed document. Never overwritten.
        #[arg(long, default_value = "current.json")]
        out: PathBuf,
    },
    /// Check a signed document the way a staging app does, now.
    Verify {
        /// The signed document.
        file: PathBuf,
        /// The Ed25519 public key, 64 hex characters.
        #[arg(long = "public-key")]
        public_key: String,
    },
    /// Print the public key and key id of the signing key on stdin.
    PublicKey,
}

/// What `sign` produces.
struct Signed {
    /// The signed document, exactly the bytes to serve.
    document: Vec<u8>,
    /// The staging ingest's own pin, for the first set.
    expected_measurements_line: String,
    /// A hash-only description for stderr.
    summary: String,
}

impl std::fmt::Debug for Signed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Signed")
            .field("summary", &self.summary)
            .finish()
    }
}

struct SignRequest<'a> {
    /// The sets file's contents.
    sets_text: &'a str,
    sequence: u64,
    ttl_days: u64,
    /// The served document, or `None` when nothing has been served yet.
    served: Option<&'a [u8]>,
    /// Seconds since the Unix epoch.
    now: u64,
}

/// The payload, field for field the v1 format. Serialized from this struct
/// so no field is ever repeated or added.
#[derive(Serialize)]
struct Payload<'a> {
    format: &'a str,
    channel: &'a str,
    gateway_host: &'a str,
    sequence: u64,
    issued_at: String,
    expires_at: String,
    sets: &'a [String],
}

/// The outer document.
#[derive(Serialize)]
struct Document {
    payload: String,
    signature: String,
    key_id: String,
}

/// Build, sign and self-check a pin-set document.
fn sign(seed_text: &[u8], request: &SignRequest<'_>) -> Result<Signed, String> {
    let lifetime = request
        .ttl_days
        .checked_mul(DAY_SECS)
        .filter(|secs| (1..=MAX_LIFETIME_SECS).contains(secs))
        .ok_or("ttl_out_of_range")?;
    let sets = parse_sets(request.sets_text)?;
    if let Some(served) = request.served {
        if request.sequence <= served_sequence(served)? {
            return Err("sequence_not_above_served".to_string());
        }
    }
    let pair = key_pair(seed_text)?;
    let public_key = pair.public_key().as_ref();
    let host = staging_gateway_host();
    let expires_at = request
        .now
        .checked_add(lifetime)
        .ok_or("ttl_out_of_range")?;

    let payload = serde_json::to_vec(&Payload {
        format: FORMAT,
        channel: CHANNEL,
        gateway_host: &host,
        sequence: request.sequence,
        issued_at: utc(request.now)?,
        expires_at: utc(expires_at)?,
        sets: &sets,
    })
    .map_err(|_| "payload_unserializable")?;
    let signature = pair.sign(&staging_pins::signing_input(&payload));
    let mut document = serde_json::to_vec_pretty(&Document {
        payload: URL_SAFE_NO_PAD.encode(&payload),
        signature: URL_SAFE_NO_PAD.encode(signature.as_ref()),
        key_id: staging_pins::key_id(public_key),
    })
    .map_err(|_| "document_unserializable")?;
    document.push(b'\n');

    // Never emit a document the app would refuse: check it with the app's
    // own verifier, at the time it was issued.
    let verified = staging_pins::verify(&document, public_key, request.now, &host)
        .map_err(|refusal| format!("self_verify_failed:{}", refusal.label()))?;

    Ok(Signed {
        summary: format!(
            "signed sequence={} sets={} issued_at={} expires_at={} key_id={} payload_sha256=sha256:{}",
            verified.sequence,
            verified.sets.len(),
            utc(verified.issued_at)?,
            utc(verified.expires_at)?,
            staging_pins::key_id(public_key),
            hex::encode(verified.payload_sha256),
        ),
        expected_measurements_line: format!("{INGEST_PIN_VARIABLE}={}", sets[0]),
        document,
    })
}

/// Check a document as the app does, against `public_key_hex`, at `now`.
/// The `Ok` is a hash-only summary.
fn verify_document(document: &[u8], public_key_hex: &str, now: u64) -> Result<String, String> {
    let public_key = hex::decode(public_key_hex.trim())
        .ok()
        .filter(|key| key.len() == 32)
        .ok_or("public_key_malformed")?;
    let verified = staging_pins::verify(document, &public_key, now, &staging_gateway_host())
        .map_err(|refusal| refusal.label().to_string())?;
    Ok(format!(
        "verified sequence={} sets={} issued_at={} expires_at={} key_id={} payload_sha256=sha256:{}",
        verified.sequence,
        verified.sets.len(),
        utc(verified.issued_at)?,
        utc(verified.expires_at)?,
        staging_pins::key_id(&public_key),
        hex::encode(verified.payload_sha256),
    ))
}

/// The bare host of the compiled-in staging NEAR AI base: what a staging
/// app compares a payload's `gateway_host` with.
fn staging_gateway_host() -> String {
    reqwest::Url::parse(NEAR_AI_API_BASE_URL)
        .ok()
        .and_then(|url| url.host_str().map(str::to_ascii_lowercase))
        .expect("the compiled-in NEAR AI base is a URL with a host")
}

/// One set per non-blank line, each canonicalised, repeats dropped, order
/// kept (the first is the ingest's pin).
fn parse_sets(text: &str) -> Result<Vec<String>, String> {
    let mut sets: Vec<String> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let set = match ExpectedMeasurements::from_env_value(Some(line)) {
            Ok(Some(set)) => set.to_pin_string(),
            _ => return Err(format!("set_unparseable:line_{}", index + 1)),
        };
        if !sets.contains(&set) {
            sets.push(set);
        }
    }
    if sets.is_empty() {
        return Err("no_sets".to_string());
    }
    Ok(sets)
}

/// The sequence the served document states.
///
/// Read, not verified. This guards the operator against signing a sequence
/// devices would refuse as a rollback; it is not a security boundary, which
/// is each device's own record of the highest sequence it accepted. Not
/// verifying also means an expired served document (whose sequence devices
/// still remember) or one signed by a retired key is still read.
fn served_sequence(served: &[u8]) -> Result<u64, String> {
    let unreadable = || "served_document_unreadable".to_string();
    let outer: serde_json::Value = serde_json::from_slice(served).map_err(|_| unreadable())?;
    let payload = outer
        .get("payload")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(unreadable)?;
    let payload = URL_SAFE_NO_PAD.decode(payload).map_err(|_| unreadable())?;
    let payload: serde_json::Value = serde_json::from_slice(&payload).map_err(|_| unreadable())?;
    payload
        .get("sequence")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(unreadable)
}

/// The signing key, from 64 hex characters of seed.
fn key_pair(seed_text: &[u8]) -> Result<Ed25519KeyPair, String> {
    let text = Zeroizing::new(seed_text.to_vec());
    let trimmed = text.trim_ascii();
    if trimmed.is_empty() {
        return Err("signing_key_missing".to_string());
    }
    if trimmed.len() != 64 || !trimmed.iter().all(u8::is_ascii_hexdigit) {
        return Err("signing_key_malformed".to_string());
    }
    let seed = Zeroizing::new(hex::decode(trimmed).map_err(|_| "signing_key_malformed")?);
    Ed25519KeyPair::from_seed_unchecked(&seed).map_err(|_| "signing_key_malformed".to_string())
}

/// Seconds since the epoch as `YYYY-MM-DDTHH:MM:SSZ`, the one timestamp
/// form the format accepts.
fn utc(secs: u64) -> Result<String, String> {
    i64::try_from(secs)
        .ok()
        .and_then(|secs| chrono::DateTime::from_timestamp(secs, 0))
        .map(|time| time.format("%Y-%m-%dT%H:%M:%SZ").to_string())
        .ok_or_else(|| "timestamp_out_of_range".to_string())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

/// Read a secret from stdin, refusing one longer than [`STDIN_LIMIT`].
fn read_stdin_secret() -> Result<Zeroizing<Vec<u8>>, String> {
    let mut buffer = Zeroizing::new(Vec::new());
    std::io::stdin()
        .take(STDIN_LIMIT + 1)
        .read_to_end(&mut buffer)
        .map_err(|_| "stdin_unreadable")?;
    if buffer.len() as u64 > STDIN_LIMIT {
        return Err("stdin_too_long".to_string());
    }
    Ok(buffer)
}

fn run_observe(models: &[String], registers: &str) -> Result<Vec<String>, String> {
    let registers = parse_registers(registers)?;
    let key = read_stdin_secret()?;
    let key = std::str::from_utf8(&key)
        .map(str::trim)
        .map_err(|_| "api_key_malformed")?;
    if key.is_empty() {
        return Err("api_key_missing".to_string());
    }
    let mut clients = Vec::with_capacity(models.len());
    for model in models {
        clients.push(
            HttpAttestationClient::new(
                NEAR_AI_API_BASE_URL,
                model.as_str(),
                SecretString::from(key.to_string()),
                INTEL_PCS_URL,
                CALL_TIMEOUT,
                Some(NEAR_AI_API_BASE_URL.to_string()),
            )
            .map_err(|_| "client_unbuildable")?,
        );
    }
    let clients: Vec<&dyn AttestedKeyReportClient> = clients
        .iter()
        .map(|client| client as &dyn AttestedKeyReportClient)
        .collect();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| "runtime_unbuildable")?;
    runtime
        .block_on(observe_measurement_sets(
            &clients,
            &RandomNonce,
            now_unix(),
            &registers,
        ))
        .map_err(|refusal| format!("model_{}:{}", refusal.model_position, refusal.label))
}

fn run_sign(
    sets: &Path,
    sequence: u64,
    ttl_days: u64,
    current: Option<&Path>,
    out: &Path,
) -> Result<Signed, String> {
    let sets_text = std::fs::read_to_string(sets).map_err(|_| "sets_unreadable")?;
    let served = match current {
        Some(path) => Some(std::fs::read(path).map_err(|_| "served_document_unreadable")?),
        None => None,
    };
    let seed = read_stdin_secret()?;
    let signed = sign(
        &seed,
        &SignRequest {
            sets_text: &sets_text,
            sequence,
            ttl_days,
            served: served.as_deref(),
            now: now_unix(),
        },
    )?;
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(out)
        .map_err(|error| match error.kind() {
            std::io::ErrorKind::AlreadyExists => "output_exists",
            _ => "output_unwritable",
        })?;
    file.write_all(&signed.document)
        .and_then(|()| file.sync_all())
        .map_err(|_| "output_unwritable")?;
    Ok(signed)
}

fn run_public_key() -> Result<String, String> {
    let seed = read_stdin_secret()?;
    let pair = key_pair(&seed)?;
    let public_key = pair.public_key().as_ref();
    Ok(format!(
        "public_key={}\nkey_id={}",
        hex::encode(public_key),
        staging_pins::key_id(public_key)
    ))
}

fn refuse(label: &str) -> ExitCode {
    eprintln!("refused: {label}");
    ExitCode::from(1)
}

fn main() -> ExitCode {
    match Cli::parse().command {
        Command::Observe { models, registers } => match run_observe(&models, &registers) {
            Ok(sets) => {
                for set in &sets {
                    println!("{set}");
                }
                eprintln!("observed models={} sets={}", models.len(), sets.len());
                ExitCode::SUCCESS
            }
            Err(label) => refuse(&label),
        },
        Command::Sign {
            sets,
            sequence,
            ttl_days,
            current,
            first: _,
            out,
        } => match run_sign(&sets, sequence, ttl_days, current.as_deref(), &out) {
            Ok(signed) => {
                println!("{}", signed.expected_measurements_line);
                eprintln!("{}", signed.summary);
                ExitCode::SUCCESS
            }
            Err(label) => refuse(&label),
        },
        Command::Verify { file, public_key } => {
            let result = std::fs::read(&file)
                .map_err(|_| "document_unreadable".to_string())
                .and_then(|document| verify_document(&document, &public_key, now_unix()));
            match result {
                Ok(summary) => {
                    println!("{summary}");
                    ExitCode::SUCCESS
                }
                Err(label) => refuse(&label),
            }
        }
        Command::PublicKey => match run_public_key() {
            Ok(text) => {
                println!("{text}");
                ExitCode::SUCCESS
            }
            Err(label) => refuse(&label),
        },
    }
}

#[cfg(test)]
#[path = "staging_pins_internal/tests.rs"]
mod tests;
