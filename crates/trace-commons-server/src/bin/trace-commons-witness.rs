// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-witness` — the redaction witness, served over HTTP.
//!
//! Runs inside a dstack TDX guest. It takes a raw transcript, redacts it,
//! reaches a residual-PII verdict with the same function ingest runs, and
//! signs a certificate over the redacted bytes with a key derived inside the
//! enclave. The server verifies that certificate without ever holding the raw
//! transcript.
//!
//! Two routes, and deliberately nothing else — see
//! [`trace_commons_server::witness_service::http`]. There is no health route
//! that reports state and no metrics route, because a witness that can be
//! asked what it has seen is not one that holds nothing.
//!
//! # This binary is thin on purpose
//!
//! Everything testable lives in the library: the router, the handlers, the
//! request bound, the nonce parser. What is left here is what cannot be
//! covered by a unit test — reading the environment, opening the dstack
//! socket, and binding a port. Anything that grows a branch worth asserting
//! on belongs behind `witness_service`, not here.
//!
//! # Boot is fail-closed
//!
//! Every dependency is resolved before the listener binds. A witness that
//! cannot reach the dstack agent, cannot derive its signing key, or cannot
//! read its own measurement exits non-zero at startup rather than accepting a
//! request it will refuse — the difference between an operator seeing the
//! failure and a contributor seeing it.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, bail};
use clap::Parser;
use sha2::Digest as _;
use tokio::net::TcpListener;
use trace_commons_server::witness_service::enclave::{DSTACK_SOCKET_PATH, DstackSocketAgent};
use trace_commons_server::witness_service::http::{WitnessLoadBound, witness_router};
use trace_commons_server::witness_service::inference::{
    DEFAULT_MAX_BODY_BYTES, InferenceAttestationPolicy, parse_gateway_key_pins,
    parse_model_key_pins,
};
use trace_commons_server::witness_service::surface::WitnessService;
use trace_commons_server::witness_service::{
    DeterministicRedaction, Enclave, FullPipelineRedaction, Signer, TranscriptRedactor,
};

/// The deterministic secret pass, and nothing else.
///
/// A witness wired this way redacts **less than ingest does**: the prose-PII
/// classifier never runs, and the certificate's `redaction_policy_version` is
/// the deterministic alias, so a server that requires the classifier can and
/// should refuse it. It stays available because it is the only pipeline with
/// no network dependency.
const DETERMINISTIC_ONLY: &str = "deterministic-only";

/// Both stages: the deterministic secret pass, then the prose-PII classifier
/// over its output -- the same two stages, in the same order, that ingest
/// applies to every event it receives.
///
/// The classifier backend is resolved from `TRACE_PRIVACY_FILTER_BACKEND` and
/// its adapter-specific variables **once, here, at startup**. A witness that
/// cannot resolve one does not start, and one that resolves an unset backend
/// does not start either: falling back to the deterministic pass would be a
/// certificate quietly claiming coverage the pass did not have.
const FULL_PIPELINE: &str = "full-pipeline";

/// 64 MiB, and the reasoning matters more than the number.
///
/// The redacted-envelope cap is 16 MiB; the measured raw-to-envelope ratio on
/// this pilot is about 3.4:1, and 7% of real sessions already exceed the cap
/// before that multiplier. 64 MiB clears 16 MiB × 3.4 with room, and is still
/// a bound rather than a gesture: the body is read through a limiter that
/// stops at it, so an oversized request costs this much and not what the
/// sender chose to send.
const DEFAULT_MAX_REQUEST_BYTES: usize = 64 * 1024 * 1024;

/// Four `POST /v1/witness` requests at once, and the number is argued rather
/// than round.
///
/// The binding evidence is #456: raising the privacy filter's in-flight
/// classify windows from 1 to 8 collapsed throughput on the pilot -- every
/// backstop tick returned `done=0 transient=3 breaker_tripped=true` -- and
/// `MAX_CONCURRENT_CLASSIFY_WINDOWS` has been 1 ever since. That constant
/// serialises windows *within* one request; N concurrent requests put N
/// windows in flight at the same endpoint and reintroduce exactly the
/// concurrency that collapsed. So the bound is set strictly under the 8 that
/// is known to fail, not at some number that merely sounds safe.
///
/// The other constraint is memory, and it is what rules out a much larger
/// value: a request at the 64 MiB body cap is buffered whole, parsed into an
/// owned transcript, redacted into another, and serialised again -- several
/// tens of megabytes each, so a couple of hundred megabytes per slot at the
/// cap. Four slots is on the order of a gigabyte in the worst case, which a
/// modestly provisioned CVM survives and sixteen would not.
///
/// In `full-pipeline` a request is IO-bound -- a serial chain of network calls
/// -- so four is not about cores. In `deterministic-only` it is CPU-bound and
/// four oversubscribes a small guest, but each pass is a linear scan that
/// finishes; oversubscription there costs latency, not stability.
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 4;

/// Five minutes, and this one is measured.
///
/// The classifier's largest proven success is a 243 KiB window in 9.3 s
/// (`privacy_filter_near_ai`), so a request classifies on the order of
/// 25 KiB/s of prose, serially. Five minutes is therefore several megabytes of
/// transcript -- comfortably the whole of a typical session -- while bounding
/// the worst case: a fully occupied witness returns to service within five
/// minutes with no operator action.
///
/// The floor is set by the layer below. A single window may retry up to
/// `MAX_CLASSIFY_ATTEMPTS` times against a 30 s per-call timeout with backoff,
/// so a request whose one window merely retried can legitimately take over two
/// minutes. A request timeout under that would abandon healthy work and read
/// as flakiness.
///
/// A body at the 64 MiB cap will exceed this and be refused. That is the
/// intended answer: the alternative is one caller holding a slot for the
/// three-quarters of an hour that classifying 64 MiB serially would take.
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 300;

#[derive(Parser, Debug)]
#[command(
    name = "trace-commons-witness",
    about = "Redaction witness: redact, judge and certify inside a TDX enclave",
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
struct Args {
    /// Address to bind. Bind to the guest's own interface; the witness is
    /// reached through whatever terminates TLS in front of it.
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_BIND",
        default_value = "127.0.0.1:8088"
    )]
    bind: String,

    /// Path to the dstack guest-agent socket.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_DSTACK_SOCKET", default_value = DSTACK_SOCKET_PATH)]
    dstack_socket: String,

    /// Largest request body the witness will read, in bytes.
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_MAX_REQUEST_BYTES",
        default_value_t = DEFAULT_MAX_REQUEST_BYTES
    )]
    max_request_bytes: usize,

    /// Which redaction pipeline to wire: `deterministic-only` or
    /// `full-pipeline`. See [`DETERMINISTIC_ONLY`] and [`FULL_PIPELINE`].
    ///
    /// Required rather than defaulted, in either direction. An operator who
    /// has not read what the two mean cannot deploy either one by leaving a
    /// variable unset, and cannot get the narrower one by accident.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_REDACTION")]
    redaction: String,

    /// How many `POST /v1/witness` requests may be in flight at once.
    ///
    /// Defaulted rather than required, unlike `--redaction`. That variable
    /// refuses a default because its two values mean different coverage and
    /// neither is safe to pick on the operator's behalf. This one has an
    /// unambiguous safe answer: an operator who says nothing wants a bounded
    /// witness, and the alternative to a default here is the unbounded service
    /// this flag exists to end. See [`DEFAULT_MAX_CONCURRENT_REQUESTS`].
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_MAX_CONCURRENT_REQUESTS",
        default_value_t = DEFAULT_MAX_CONCURRENT_REQUESTS
    )]
    max_concurrent_requests: usize,

    /// Slots left available to ordinary calls when background calls fill
    /// their cooperative budget. Defaults to one, or zero for a one-slot
    /// witness. This does not increase the global concurrency limit.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_RESERVED_INTERACTIVE_SLOTS")]
    reserved_interactive_slots: Option<usize>,

    /// How long one `POST /v1/witness` request may take before it is
    /// abandoned and its slot released. See [`DEFAULT_REQUEST_TIMEOUT_SECS`].
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS",
        default_value_t = DEFAULT_REQUEST_TIMEOUT_SECS
    )]
    request_timeout_secs: u64,

    /// Comma-separated filesystem path prefixes the deterministic pass treats
    /// as known, so they are not reported as findings. Empty is a safe
    /// default: it reports more, never less.
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_KNOWN_PATH_PREFIXES",
        default_value = ""
    )]
    known_path_prefixes: String,

    /// Refuse any contribution whose last declared inference call does not
    /// carry a verified NEAR AI receipt.
    ///
    /// Off by default, and that default is the honest one rather than the
    /// lax one: turning it on refuses every trace from Claude Code, Codex,
    /// Gemini and Cline -- a receipt exists only for inference that went
    /// through NEAR AI -- and every trace that withheld tool payloads, since
    /// the bodies the receipt binds are carried under that consent flag. That
    /// is a product decision, and a witness must not make it by inheriting a
    /// security control's default.
    ///
    /// When it is on the witness is fail-closed: a submission it cannot
    /// verify is refused by a name of its own, and there is no configuration
    /// under which it certifies an unattested trace instead.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_REQUIRE_ATTESTED_INFERENCE")]
    require_attested_inference: bool,

    /// Largest attested request or response body the witness will hash.
    #[arg(
        long,
        env = "TRACE_COMMONS_WITNESS_MAX_INFERENCE_BODY_BYTES",
        default_value_t = DEFAULT_MAX_BODY_BYTES
    )]
    max_inference_body_bytes: usize,

    /// The ed25519 keys that may sign an inference receipt, **per model**:
    /// `model=key[,model=key...]`, each key 64 hex characters with no `0x`.
    /// A receipt whose bound model is not listed, or whose signer is not one
    /// of that model's keys, is refused. Repeating a model accumulates keys,
    /// which is how a model served by more than one enclave is pinned.
    ///
    /// Replaces `TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN`, which pinned the
    /// gateway's key. That key signs no receipt: NEAR AI signs each hosted
    /// model's receipts with a per-model `provider_tee` key, so the gateway
    /// pin refused every real receipt. **An operator upgrading past that
    /// release must unset the old variable and set this one** -- the old name
    /// is no longer read, and a witness left with only the old variable set
    /// runs unpinned.
    ///
    /// The values are public keys, obtained once and out of band from NEAR
    /// AI's attestation report: `GET /attestation/report?model=..&
    /// signing_algo=ed25519&nonce=<64 hex>`, taking each `model_attestations`
    /// entry whose `report_data` is `signing_address || nonce`.
    /// `signing_algo=ed25519` is not optional -- without it the endpoint
    /// answers with ECDSA attestations, whose keys sign nothing verified here.
    /// They are not fetched on the request path: a witness makes no outbound
    /// calls while serving, and a report fetched at request time would be
    /// trusted over a path an attacker able to substitute a signing key is
    /// also positioned to influence.
    ///
    /// Unset by default, and unset is exactly the behaviour that shipped
    /// before any of this existed -- a receipt still has to verify, but
    /// against the key it names rather than against ones this deployment
    /// trusts.
    ///
    /// Independent of `--require-attested-inference`. A witness that requires
    /// nothing still refuses a receipt from an unpinned key when one is
    /// offered, because certifying it would be a silent downgrade.
    ///
    /// Malformed is a startup failure, never an ignored value -- including
    /// the empty string, which is what an operator who exported the variable
    /// without a value would otherwise get a pin-less witness from.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_MODEL_KEY_PINS")]
    model_key_pins: Option<String>,

    /// The ed25519 keys that may sign a **gateway**-kind inference receipt:
    /// `key[,key...]`, each 64 hex characters with no `0x` and no `model=`
    /// prefix. A gateway receipt whose signer is not listed is refused.
    ///
    /// # Why this exists beside the model pins
    ///
    /// NEAR AI issues two legitimate kinds of receipt for the **same hosted
    /// model**, and the protocol decides which. A Chat Completions call
    /// (`/v1/chat/completions`) yields `signature_kind: "provider_tee"`,
    /// signed by that model's key -- pinned by
    /// `TRACE_COMMONS_WITNESS_MODEL_KEY_PINS`. A Responses API call
    /// (`/v1/responses`) yields `signature_kind: "gateway"`, signed by the
    /// single key in the report's `gateway_attestation` -- pinned here.
    ///
    /// **The Codex CLI speaks the Responses API exclusively.** It dropped
    /// `wire_api = "chat"`, so a deployment whose contributors use Codex
    /// produces only gateway-signed receipts, and a witness with model pins
    /// alone refuses every one of them. An operator whose contributors use
    /// Codex needs this variable; one whose traffic is chat-completions needs
    /// the model pins; a deployment seeing both needs both.
    ///
    /// # This is not the retired `TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN`
    ///
    /// Deliberately a different name, and the boot-time refusal on the old one
    /// stays. The old variable applied the gateway key to *every* receipt,
    /// which refused every `provider_tee` one; silently re-reading a value an
    /// operator set under those semantics would give it new meaning they never
    /// chose. Unset the old name and set this one.
    ///
    /// The value is a public key, obtained once and out of band from the same
    /// report the model pins come from -- `GET /attestation/report?model=..&
    /// signing_algo=ed25519&nonce=<64 hex>` -- taking `gateway_attestation`'s
    /// `signing_address`, whose `report_data` must be `signing_address ||
    /// nonce`. One fetch yields both pins.
    ///
    /// Unset means gateway receipts are unpinned. On a witness that pins the
    /// other kind, unpinned means **refused**: pinning model keys is not
    /// agreement to accept any gateway-signed receipt from any signer. On a
    /// witness that pins nothing at all, everything is dormant as before.
    ///
    /// Malformed is a startup failure, never an ignored value -- including the
    /// empty string.
    #[arg(long, env = "TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS")]
    gateway_receipt_key_pins: Option<String>,
}

fn resolved_reserved_interactive_slots(
    max_concurrent_requests: usize,
    configured: Option<usize>,
) -> Result<usize, &'static str> {
    if max_concurrent_requests == 0 {
        return Err("witness concurrency limit must be positive");
    }
    let reserved = configured.unwrap_or(usize::from(max_concurrent_requests > 1));
    if reserved >= max_concurrent_requests {
        return Err("reserved interactive slots must be below the concurrency limit");
    }
    Ok(reserved)
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let args = Args::parse();

    if args.redaction != DETERMINISTIC_ONLY && args.redaction != FULL_PIPELINE {
        bail!(
            "TRACE_COMMONS_WITNESS_REDACTION must be `{DETERMINISTIC_ONLY}` or \
             `{FULL_PIPELINE}`"
        );
    }
    if args.max_request_bytes == 0 {
        bail!("TRACE_COMMONS_WITNESS_MAX_REQUEST_BYTES must be greater than zero");
    }
    // Zero is refused rather than read as "unbounded" in one case and "serve
    // nothing" in the other. Both readings are guesses about what an operator
    // who typed 0 meant, and one of them silently removes the bound.
    if args.max_concurrent_requests == 0 {
        bail!("TRACE_COMMONS_WITNESS_MAX_CONCURRENT_REQUESTS must be greater than zero");
    }
    let reserved_interactive_slots = resolved_reserved_interactive_slots(
        args.max_concurrent_requests,
        args.reserved_interactive_slots,
    )
    .map_err(anyhow::Error::msg)?;
    if args.request_timeout_secs == 0 {
        bail!("TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS must be greater than zero");
    }

    // The retired gateway pin. It is no longer read, and a witness that
    // silently ignored it would start, pin nothing, and leave an operator
    // believing it pins -- which is the exact failure this release exists to
    // fix, in a new place. This repo's rule is that a configured gate whose
    // dependency is gone refuses, so refuse, and name the replacement.
    //
    // The bail stays even though gateway receipts are pinnable again, and
    // that is why the new variable took a new name. The old one applied its
    // key to *every* receipt, including `provider_tee` ones it could never
    // match; re-reading a value set under those semantics would give it a
    // meaning the operator never chose, on a control whose whole job is to
    // decide who may sign.
    //
    // Checked directly rather than through a clap argument: the point is to
    // notice a variable nothing is meant to parse. Presence alone is enough,
    // including when it is set to the empty string.
    if std::env::var_os("TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN").is_some() {
        bail!(
            "TRACE_COMMONS_WITNESS_GATEWAY_KEY_PIN is set but is no longer read. \
             It pinned the inference gateway's signing key, which signs no \
             receipt from a chat-completions call, so it refused every one of \
             those. Unset it. To pin receipt signers, set \
             TRACE_COMMONS_WITNESS_MODEL_KEY_PINS to `model=key[,model=key...]` \
             using each model's own attested ed25519 key, and, for \
             Responses-API traffic such as the Codex CLI's, \
             TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS to the gateway's \
             own attested key -- see deploy/witness/README.md for both \
             derivations. Leaving both unset is the dormant default."
        );
    }

    // Before the listener binds: the agent round trip that derives the signing
    // key and reads the boot measurement. A witness that cannot name itself
    // must not start.
    let agent = DstackSocketAgent::at(&args.dstack_socket);
    let enclave = Arc::new(
        trace_commons_server::witness_service::enclave::DstackEnclave::connect(Box::new(agent))
            .await
            .context("could not derive a signing identity from the dstack guest agent")?,
    );
    let measurement = enclave
        .measurement()
        .await
        .map_err(|_| anyhow::anyhow!("the enclave could not report its own measurement"))?;

    // Both are hash-only, and both are what an operator pins. Nothing else
    // about a request is ever logged by this process.
    tracing::info!(
        signing_address = %enclave.signing_address(),
        witness_measurement = %measurement,
        max_request_bytes = args.max_request_bytes,
        max_concurrent_requests = args.max_concurrent_requests,
        request_timeout_secs = args.request_timeout_secs,
        "witness ready"
    );

    let known_path_prefixes: Vec<String> = args
        .known_path_prefixes
        .split(',')
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
        .map(str::to_string)
        .collect();

    // Resolved before the listener binds, like the signing identity above: a
    // witness that cannot build the pipeline it was told to run must fail to
    // start, not fail per request.
    let (redactor, contribution_redactor): (
        Arc<dyn TranscriptRedactor>,
        Arc<dyn trace_commons_server::witness_service::ContributionRedactor>,
    ) = if args.redaction == FULL_PIPELINE {
        let (adapter, backend) =
            trace_commons_protocol::trace_contribution::privacy_filter_adapter_from_env()
                .context("could not build the configured privacy-filter backend")?
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "TRACE_COMMONS_WITNESS_REDACTION=`{FULL_PIPELINE}` requires a \
                     prose-PII classifier: set TRACE_PRIVACY_FILTER_BACKEND"
                    )
                })?;
        tracing::info!(
            redaction_pipeline = %trace_commons_protocol::trace_contribution::redaction_pipeline_version(backend),
            "witness redaction pipeline"
        );
        (Arc::new(FullPipelineRedaction::new(known_path_prefixes.clone(), adapter.clone(), backend)),
         Arc::new(trace_commons_server::witness_service::PipelineContributionRedaction::with_privacy_filter(known_path_prefixes, adapter, backend)))
    } else {
        (Arc::new(DeterministicRedaction::new(known_path_prefixes.clone())),
         Arc::new(trace_commons_server::witness_service::PipelineContributionRedaction::deterministic_only(known_path_prefixes)))
    };

    // Resolved before the listener binds, like everything else here: a
    // witness told to require attested inference under a policy that would
    // require nothing must fail to start rather than serve a requirement that
    // is not one.
    let inference_policy = if args.require_attested_inference {
        let policy =
            InferenceAttestationPolicy::required(args.max_inference_body_bytes).map_err(|_| {
                anyhow::anyhow!(
                    "TRACE_COMMONS_WITNESS_MAX_INFERENCE_BODY_BYTES must be greater \
                     than zero when attested inference is required"
                )
            })?;
        // Label-only, and it is what an operator needs to see to know which
        // policy this process is actually running: the certificate does not
        // carry it, and the measurement covers the image rather than the
        // environment.
        tracing::info!(
            require_attested_inference = true,
            max_inference_body_bytes = args.max_inference_body_bytes,
            "witness attested-inference requirement"
        );
        policy
    } else {
        InferenceAttestationPolicy::not_required()
    };

    // The per-model key pins, applied to whichever policy was just built: they
    // constrain which key a receipt may be signed by for the model the receipt
    // itself binds, and say nothing about whether a receipt is required.
    // Resolved here, before the listener binds, so pins that are not keys are
    // a startup failure rather than a control that silently matches nothing.
    let inference_policy = match args.model_key_pins.as_deref() {
        Some(spec) => {
            let malformed = || {
                // The value is not echoed. On a misconfiguration it is
                // whatever the operator pasted, and that is not something to
                // put in a log line.
                anyhow::anyhow!(
                    "TRACE_COMMONS_WITNESS_MODEL_KEY_PINS must be \
                     `model=key[,model=key...]`, each key a 32-byte ed25519 key: \
                     64 hex characters, no `0x` prefix"
                )
            };
            let pins = parse_model_key_pins(spec).map_err(|_| malformed())?;
            let policy = inference_policy
                .pinning_model_keys(pins)
                .map_err(|_| malformed())?;
            // Hash-only, like every other operational surface here. A short
            // prefix over the whole normalised pin set is enough for an
            // operator to confirm that the process holds the pins they meant
            // to configure, and carries neither a key nor a model name.
            let pins = policy
                .model_key_pins()
                .expect("the pins were just accepted");
            let mut hasher = sha2::Sha256::new();
            for (model, keys) in pins {
                hasher.update(model.as_bytes());
                for key in keys {
                    hasher.update(b"\0");
                    hasher.update(key.as_bytes());
                }
                hasher.update(b"\n");
            }
            let digest = hex::encode(hasher.finalize());
            tracing::info!(
                model_key_pins_sha256_prefix = %&digest[..8],
                pinned_models = pins.len(),
                "witness inference model key pins"
            );
            policy
        }
        None => inference_policy,
    };

    // The gateway pin, alongside the model pins rather than instead of them.
    // A receipt's own `signature_kind` decides which set applies: `gateway`
    // for a Responses-API receipt (every Codex-driven one), `provider_tee`
    // for a chat-completions one. Resolved here for the same reason the model
    // pins are -- a value that is not a key must fail the process rather than
    // become a control that silently matches nothing.
    let inference_policy = match args.gateway_receipt_key_pins.as_deref() {
        Some(spec) => {
            let malformed = || {
                // Not echoed, for the reason the model-pin message gives.
                anyhow::anyhow!(
                    "TRACE_COMMONS_WITNESS_GATEWAY_RECEIPT_KEY_PINS must be \
                     `key[,key...]`, each a 32-byte ed25519 key: 64 hex \
                     characters, no `0x` prefix and no `model=` -- that spelling \
                     belongs in TRACE_COMMONS_WITNESS_MODEL_KEY_PINS"
                )
            };
            let keys = parse_gateway_key_pins(spec).map_err(|_| malformed())?;
            let policy = inference_policy
                .pinning_gateway_keys(keys)
                .map_err(|_| malformed())?;
            let keys = policy
                .gateway_key_pins()
                .expect("the pins were just accepted");
            let mut hasher = sha2::Sha256::new();
            for key in keys {
                hasher.update(key.as_bytes());
                hasher.update(b"\n");
            }
            let digest = hex::encode(hasher.finalize());
            tracing::info!(
                gateway_key_pins_sha256_prefix = %&digest[..8],
                pinned_gateway_keys = keys.len(),
                "witness inference gateway key pins"
            );
            policy
        }
        None => inference_policy,
    };

    let mut service = WitnessService::new(
        redactor,
        enclave.clone() as Arc<dyn Signer>,
        enclave as Arc<dyn Enclave>,
        args.max_request_bytes,
    )
    .requiring_attested_inference(inference_policy)
    .with_contribution_redactor(contribution_redactor);
    if std::env::var_os("TRACE_COMMONS_WITNESS_ADMISSION_PROVIDER_SIGNERS").is_some() {
        let trust = trace_commons_server::admission_evidence::AdmissionProviderTrust::from_env(
            "TRACE_COMMONS_WITNESS_ADMISSION",
        )
        .map_err(|_| anyhow::anyhow!("admission_provider_policy_missing_or_invalid"))?;
        service = service.with_admission_provider_trust(trust);
    }
    let service = Arc::new(service);

    let load = WitnessLoadBound::with_reservation(
        args.max_concurrent_requests,
        reserved_interactive_slots,
        Duration::from_secs(args.request_timeout_secs),
    )
    .map_err(anyhow::Error::msg)?;
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("could not bind {}", args.bind))?;
    axum::serve(listener, witness_router(service, load))
        .await
        .context("the witness listener stopped")?;
    Ok(())
}

#[cfg(test)]
mod reserved_interactive_tests {
    use super::resolved_reserved_interactive_slots;

    #[test]
    fn default_and_explicit_reservations_respect_global_limit() {
        assert_eq!(resolved_reserved_interactive_slots(4, None), Ok(1));
        assert_eq!(resolved_reserved_interactive_slots(4, Some(0)), Ok(0));
        assert_eq!(resolved_reserved_interactive_slots(1, None), Ok(0));
        assert!(resolved_reserved_interactive_slots(1, Some(1)).is_err());
        assert!(resolved_reserved_interactive_slots(4, Some(4)).is_err());
        assert!(resolved_reserved_interactive_slots(0, None).is_err());
    }
}
