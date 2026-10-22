// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `trace-commons-judgement` -- the Insights judgement service, served over
//! HTTP from a dstack TDX guest.
//!
//! Design: `docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`
//! (spec PR #1501). Two routes, `GET /v1/attestation` and
//! `POST /v1/judgement`; see
//! [`trace_commons_server::judgement_service::http`].
//!
//! # This binary is thin on purpose
//!
//! Everything testable lives in the library: the configuration checks and
//! the boot order ([`trace_commons_server::judgement_service::boot`]), the
//! router, the enclave seam. What is left here reads the environment, opens
//! the dstack socket and binds a port.
//!
//! # Boot is fail-closed
//!
//! Every dependency resolves before the listener binds. A refusal exits
//! non-zero and prints one line, `judgement_boot_refused: <label>`, with a
//! stable label and no value. The image workflow's pre-push checks match on
//! those labels.

use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result};
use clap::Parser;
use tokio::net::TcpListener;
use trace_commons_server::judgement_service::boot::{
    BootConfig, BootDeps, BootRefusal, SignerAttestation, boot, spawn_catalog_refresh,
    volume_is_mounted,
};
use trace_commons_server::judgement_service::http::{
    AttestationBound, JudgementHttpState, JudgementIdentity, JudgementLoadBound, judgement_router,
    system_clock,
};
use trace_commons_server::near_attestation::client::{
    ATTESTATION_GATEWAY_BASE_URL, AttestedKeyReportClient, HttpAttestationClient, INTEL_PCS_URL,
};
use trace_commons_server::witness_service::enclave::{DSTACK_SOCKET_PATH, DstackSocketAgent};

/// Eight requests at once. A request is a serial chain of at most six
/// short model calls with a 16 KiB body, so a slot costs little memory and
/// no CPU; the bound is on concurrent NEAR AI spend, which the limiter
/// also caps.
const DEFAULT_MAX_CONCURRENT_REQUESTS: usize = 8;

/// Four minutes: six calls at the 30-second per-call timeout is three, plus
/// the claim check and the ledger writes.
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 240;

/// The bound on each call to the NEAR AI gateway at boot.
const GATEWAY_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Parser, Debug)]
#[command(
    name = "trace-commons-judgement",
    about = "Insights judgement service: pinned judgements over a session digest inside a TDX enclave",
    version = trace_commons_build_info::version_line(env!("CARGO_PKG_VERSION"))
)]
struct Args {
    /// Address to bind. Inside a container, `0.0.0.0:<port>`; the compose
    /// file sets it.
    #[arg(
        long,
        env = "TRACE_COMMONS_JUDGEMENT_BIND",
        default_value = "127.0.0.1:8089"
    )]
    bind: String,

    /// Path to the dstack guest-agent socket.
    #[arg(long, env = "TRACE_COMMONS_JUDGEMENT_DSTACK_SOCKET", default_value = DSTACK_SOCKET_PATH)]
    dstack_socket: String,

    /// How many `POST /v1/judgement` requests may be in flight at once.
    #[arg(
        long,
        env = "TRACE_COMMONS_JUDGEMENT_MAX_CONCURRENT_REQUESTS",
        default_value_t = DEFAULT_MAX_CONCURRENT_REQUESTS
    )]
    max_concurrent_requests: usize,

    /// How long one `POST /v1/judgement` request may take.
    #[arg(
        long,
        env = "TRACE_COMMONS_JUDGEMENT_REQUEST_TIMEOUT_SECS",
        default_value_t = DEFAULT_REQUEST_TIMEOUT_SECS
    )]
    request_timeout_secs: u64,
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
    let load = JudgementLoadBound::new(
        args.max_concurrent_requests,
        Duration::from_secs(args.request_timeout_secs),
    )
    .map_err(anyhow::Error::msg)?;

    let booted = match boot_service(&args).await {
        Ok(booted) => booted,
        Err(refusal) => {
            eprintln!("judgement_boot_refused: {refusal}");
            std::process::exit(1);
        }
    };

    let clock = system_clock();
    spawn_catalog_refresh(booted.check.clone(), booted.catalog.clone(), clock.clone());
    let state = JudgementHttpState::new(
        booted.service.clone(),
        booted.enclave.clone() as Arc<dyn JudgementIdentity>,
        clock,
        load,
        AttestationBound::default(),
    );
    let listener = TcpListener::bind(&args.bind)
        .await
        .with_context(|| format!("could not bind {}", args.bind))?;
    axum::serve(listener, judgement_router(state))
        .await
        .context("the judgement listener stopped")?;
    Ok(())
}

async fn boot_service(
    args: &Args,
) -> Result<trace_commons_server::judgement_service::boot::Booted, BootRefusal> {
    let mut config = BootConfig::from_lookup(|name| std::env::var(name).ok())?;
    let pins_sha256_prefix = config.pins_sha256_prefix.clone();
    let reports: Option<Box<dyn AttestedKeyReportClient>> = match config.model.attestation() {
        SignerAttestation::NearAiGateway => {
            let api_key = config
                .near_ai_api_key
                .take()
                .ok_or(BootRefusal::NearAiApiKeyAbsent)?;
            Some(Box::new(
                HttpAttestationClient::new(
                    ATTESTATION_GATEWAY_BASE_URL,
                    config.model.model_id(),
                    api_key,
                    INTEL_PCS_URL,
                    GATEWAY_TIMEOUT,
                    None,
                )
                .map_err(|_| BootRefusal::GatewayReportUnavailable)?,
            ))
        }
        #[allow(unreachable_patterns)]
        _ => None,
    };
    let deps = BootDeps {
        agent: Box::new(DstackSocketAgent::at(&args.dstack_socket)),
        reports,
        volume_is_mounted,
        report_nonce: trace_commons_server::near_attestation::drill::generate_drill_nonce(),
        now: chrono::Utc::now().timestamp(),
    };
    let booted = boot(config, deps).await?;
    // The value an operator pins, and a digest of the pin set. The per-boot
    // sealing key and signing address are not logged: nothing pins them,
    // and each names one boot. Nothing about a request is logged here.
    tracing::info!(
        judgement_measurement = %booted.enclave.measurement(),
        model_id = %booted.model_id,
        model_key_pins_sha256_prefix = %pins_sha256_prefix,
        max_concurrent_requests = args.max_concurrent_requests,
        request_timeout_secs = args.request_timeout_secs,
        "judgement service ready"
    );
    Ok(booted)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn the_arguments_default_to_a_bounded_local_listener() {
        let args = Args::try_parse_from(["trace-commons-judgement"]).expect("defaults");
        assert_eq!(args.bind, "127.0.0.1:8089");
        assert_eq!(args.dstack_socket, DSTACK_SOCKET_PATH);
        assert_eq!(
            args.max_concurrent_requests,
            DEFAULT_MAX_CONCURRENT_REQUESTS
        );
        assert_eq!(args.request_timeout_secs, DEFAULT_REQUEST_TIMEOUT_SECS);
        let command = Args::command();
        assert!(
            command.get_version().is_some(),
            "--version is the image's first check"
        );
    }
}
