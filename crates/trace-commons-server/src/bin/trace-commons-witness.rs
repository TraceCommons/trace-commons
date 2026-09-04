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
use tokio::net::TcpListener;
use trace_commons_server::witness_service::enclave::{DSTACK_SOCKET_PATH, DstackSocketAgent};
use trace_commons_server::witness_service::http::{WitnessLoadBound, witness_router};
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
    if args.request_timeout_secs == 0 {
        bail!("TRACE_COMMONS_WITNESS_REQUEST_TIMEOUT_SECS must be greater than zero");
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
    let redactor: Arc<dyn TranscriptRedactor> = if args.redaction == FULL_PIPELINE {
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
        Arc::new(FullPipelineRedaction::new(
            known_path_prefixes,
            adapter,
            backend,
        ))
    } else {
        Arc::new(DeterministicRedaction::new(known_path_prefixes))
    };

    let service = Arc::new(WitnessService::new(
        redactor,
        enclave.clone() as Arc<dyn Signer>,
        enclave as Arc<dyn Enclave>,
        args.max_request_bytes,
    ));

    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("could not bind {}", args.bind))?;
    let load = WitnessLoadBound::new(
        args.max_concurrent_requests,
        Duration::from_secs(args.request_timeout_secs),
    );
    axum::serve(listener, witness_router(service, load))
        .await
        .context("the witness listener stopped")?;
    Ok(())
}
