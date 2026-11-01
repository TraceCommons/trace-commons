// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Observe the measurement sets NEAR AI staging's model enclaves run under,
//! for the staging pin re-sign tool (`trace-commons-staging-pins observe`).
//! Staging builds only.
//!
//! A set is printed only from a quote that went through every check a
//! staging app makes before it trusts a model key, in the same order
//! (`routing::proof_attestor` in the contributor):
//!
//! 1. the gateway's ed25519 report for the model, for a nonce this process
//!    chose ([`AttestedKeyReportClient::fetch_ed25519_report_json`]);
//! 2. the JSON-level binding of every entry for the model to that nonce
//!    (`model_ed25519_keys`);
//! 3. each entry's quote verified against Intel collateral fetched for that
//!    quote, at the caller's clock;
//! 4. Intel's TCB verdict exactly [`REQUIRED_TCB_STATUS`];
//! 5. each entry's own `event_log` replayed against its verified quote's
//!    RTMR3, with recomputed digests, and its RTMR3 events required to be
//!    the ten dstack events in order, with the `compose-hash` event equal to
//!    MRCONFIGID's (`staging_replay::replay_rtmr3`);
//! 6. each entry's `compose_manager_attestation`: its quote verified the
//!    same way, its registers equal to the model quote's, our nonce at
//!    `report_data[32..64]` and the recomputed actions hash at
//!    `report_data[0..32]` (`staging_replay::check_compose_manager`);
//! 7. the nonce and key re-checked against the **verified** quotes
//!    (`quote_bound_keys`).
//!
//! Only then are the registers read, out of the verified quote structure.
//! One refused model refuses the whole run: printing the sets of the models
//! that passed would let an operator sign a pin set that silently omits one.
//!
//! Steps 5 and 6 check the log and the record but not their values against
//! an allowlist: `observe` is where the allowlist comes from. It returns the
//! values instead, as a candidate allowlist ([`Observation::review`]) for
//! the operator to review and commit (owner decision S16). The
//! `instance-id` event's value is never part of it: it is the per-boot
//! random value the replay exists to absorb. Its compose half (owner
//! decision S17) lists the distinct compose-manager images and the
//! distinct (file, SHA-256, commit) triples of the `compose_up` actions
//! seen. A `compose_down` carries no file hash, so the run is refused
//! (`review_compose_down_unlisted`) when one names a file and commit no
//! `compose_up` in the run brought up: the allowlist could not list it, and
//! a signed allowlist that silently refuses the live record would say
//! nothing about why. A value the allowlist format cannot hold (an image not
//! named by digest, a hash or commit not lower-case hex of its length, two
//! hashes for one file and commit) refuses the run as
//! `review_compose_actions_unlistable`.
//!
//! Refusals are labels. No key, nonce, quote, measurement or URL reaches
//! one, so a refusal can be printed as it is. The tests are in
//! `tests/staging_pins_tool.rs`, against a mock gateway.

use trace_commons_attestation::measurements::{ExpectedMeasurements, MeasurementField};
use trace_commons_attestation::quote::{
    Collateral, QuoteVerifyError, REQUIRED_TCB_STATUS, VerifiedQuote,
};
use trace_commons_attestation::receipt::{AttestedKeyError, model_ed25519_keys, quote_bound_keys};
use trace_commons_attestation::staging_replay::{
    Allowlist, ComposeAction, ComposeAllowlist, ComposeFile, ComposeManagerRecord, MARKER_EVENTS,
    Marker, ReplayedEvent, Rtmr3Allowlist, check_compose_manager, model_entry_evidence,
    replay_rtmr3,
};

use super::client::{AttestationClientError, AttestedKeyReportClient};
use super::key_drift::DrillNonceSource;

/// The registers a set names unless the operator asks for others: owner
/// decision S16's `mrtd`, `rtmr0-2` and `mrconfigid`. RTMR3 is not pinned,
/// because a per-boot `instance-id` extends it; it is replayed from the
/// entry's event log and checked against the signed allowlist instead. This
/// set ships only together with that replay, in the tool and in the app.
pub const DEFAULT_REGISTERS: [MeasurementField; 5] = [
    MeasurementField::Mrtd,
    MeasurementField::Rtmr0,
    MeasurementField::Rtmr1,
    MeasurementField::Rtmr2,
    MeasurementField::MrConfigId,
];

/// The registers of the staging ingest's own pin (spec 5.9): production's,
/// `mrtd` and `rtmr0-3`. The ingest does not replay RTMR3, so its pin keeps
/// a static `rtmr3` and never the S16 set; it goes stale when the model
/// instance restarts.
pub const INGEST_REGISTERS: [MeasurementField; 5] = [
    MeasurementField::Mrtd,
    MeasurementField::Rtmr0,
    MeasurementField::Rtmr1,
    MeasurementField::Rtmr2,
    MeasurementField::Rtmr3,
];

/// What one `observe` run produced.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    /// The distinct sets, in the order first seen.
    pub sets: Vec<String>,
    /// The staging ingest's own pin, in [`INGEST_REGISTERS`], read from the
    /// first model's first quote.
    pub ingest_pin: String,
    /// The candidate allowlist: the values every replayed log carried (one
    /// value per event, the same across every entry and model), the
    /// distinct compose-manager images and the distinct `compose_up` files
    /// seen, first-seen order. Never the `instance-id`.
    pub review: Allowlist,
}

/// One model entry that passed every check.
struct ObservedEntry {
    quote: VerifiedQuote,
    events: Vec<ReplayedEvent>,
    actions: Vec<ComposeAction>,
}

/// Why a run printed nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveRefusal {
    /// The model's position on the command line, from 1 (0 when the run
    /// itself was refused). A position rather than the model id keeps the
    /// refusal a label.
    pub model_position: usize,
    /// A stable label; never a key, nonce, quote, measurement or URL.
    pub label: String,
}

/// Parse a comma-separated register list (`mrtd,rtmr0,...`), refusing an
/// unknown name, a repeat or an empty list.
pub fn parse_registers(text: &str) -> Result<Vec<MeasurementField>, String> {
    let mut fields = Vec::new();
    for name in text.split(',').map(str::trim) {
        let lowered = name.to_ascii_lowercase();
        let field = MeasurementField::ALL
            .into_iter()
            .find(|field| field.as_str() == lowered)
            .ok_or_else(|| "registers_unknown".to_string())?;
        if fields.contains(&field) {
            return Err("registers_repeated".to_string());
        }
        fields.push(field);
    }
    Ok(fields)
}

/// Observe every model's measurement sets: the distinct sets, canonical,
/// across every enclave serving every model. Each model is asked with a
/// fresh nonce from `nonce_source`.
///
/// The sets come back in the order first seen: model order as given, then
/// quote order within a model. Not sorted, because `sign` pins the staging
/// ingest to the first set, which must be a set of the first model's
/// rather than whichever happens to sort lowest.
///
/// `now_unix` is the clock the collateral is evaluated against; the tool
/// passes wall-clock time, and only a test pins it.
///
/// `verify` is the quote verifier: the tool passes
/// [`trace_commons_attestation::quote::verify_quote`]. It is a parameter, as
/// in the contributor's `proof_attestor`, so a test can reach the checks
/// that follow verification (the TCB verdict, the nonce in the verified
/// quote), which no checked-in quote fails on its own.
pub async fn observe_measurement_sets(
    clients: &[&dyn AttestedKeyReportClient],
    nonce_source: &dyn DrillNonceSource,
    now_unix: u64,
    registers: &[MeasurementField],
    verify: &(dyn Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync),
) -> Result<Observation, ObserveRefusal> {
    let refused = |model_position: usize, label: String| ObserveRefusal {
        model_position,
        label,
    };
    if registers.is_empty() {
        return Err(refused(0, "registers_empty".to_string()));
    }
    if clients.is_empty() {
        return Err(refused(0, "no_models".to_string()));
    }
    let mut sets: Vec<String> = Vec::new();
    let mut ingest_pin: Option<String> = None;
    let mut review = Review::default();
    for (index, client) in clients.iter().enumerate() {
        let entries = observe_model(*client, &nonce_source.nonce(), now_unix, verify)
            .await
            .map_err(|label| refused(index + 1, label))?;
        for entry in &entries {
            let set = set_of(&entry.quote, registers).map_err(|label| refused(index + 1, label))?;
            if !sets.contains(&set) {
                sets.push(set);
            }
            if ingest_pin.is_none() {
                ingest_pin = Some(
                    set_of(&entry.quote, &INGEST_REGISTERS)
                        .map_err(|label| refused(index + 1, label))?,
                );
            }
            review
                .add(entry)
                .map_err(|label| refused(index + 1, label.to_string()))?;
        }
    }
    Ok(Observation {
        sets,
        ingest_pin: ingest_pin.ok_or_else(|| refused(0, "no_models".to_string()))?,
        review: review
            .finish()
            .map_err(|label| refused(0, label.to_string()))?,
    })
}

/// The candidate allowlist, built up entry by entry.
#[derive(Default)]
struct Review {
    rtmr3: Option<Rtmr3Allowlist>,
    compose_actions: ComposeAllowlist,
    /// The (file, commit) of every `compose_down` seen: each must be a
    /// listed file by the end of the run.
    downs: Vec<(String, String)>,
}

impl Review {
    /// Add one entry's values. Every entry, of every model, must carry the
    /// same value for each checked event: the allowlist holds one, so a run
    /// whose entries differ could only be signed into a set that refuses
    /// some of them.
    fn add(&mut self, entry: &ObservedEntry) -> Result<(), &'static str> {
        let payload = |name: &str| {
            entry
                .events
                .iter()
                .find(|event| event.name == name)
                .map(|event| event.payload.clone())
                .unwrap_or_default()
        };
        let values = Rtmr3Allowlist {
            app_id: payload("app-id"),
            mr_kms: payload("mr-kms"),
            os_image_hash: payload("os-image-hash"),
            key_provider: payload("key-provider"),
            markers: MARKER_EVENTS
                .iter()
                .map(|name| Marker {
                    name: (*name).to_string(),
                    payload: payload(name),
                })
                .collect(),
        };
        match &self.rtmr3 {
            None => self.rtmr3 = Some(values),
            Some(seen) if *seen == values => {}
            Some(_) => return Err("review_entries_disagree"),
        }
        for action in &entry.actions {
            match action {
                ComposeAction::ManagerStarted { image, .. } => {
                    let images = &mut self.compose_actions.compose_manager_images;
                    if !images.contains(image) {
                        images.push(image.clone());
                    }
                }
                ComposeAction::Up {
                    commit,
                    file,
                    file_sha256,
                    ..
                } => {
                    let listed = ComposeFile {
                        file: file.clone(),
                        file_sha256: file_sha256.clone(),
                        commit: commit.clone(),
                    };
                    let files = &mut self.compose_actions.compose_files;
                    if !files.contains(&listed) {
                        files.push(listed);
                    }
                }
                ComposeAction::Down { commit, file, .. } => {
                    let pair = (file.clone(), commit.clone());
                    if !self.downs.contains(&pair) {
                        self.downs.push(pair);
                    }
                }
            }
        }
        Ok(())
    }

    /// The candidate allowlist, once every entry is in. Refused when it
    /// would not allow every action seen, or could not be read back.
    fn finish(self) -> Result<Allowlist, &'static str> {
        let compose_actions = self.compose_actions;
        if self.downs.iter().any(|(file, commit)| {
            !compose_actions
                .compose_files
                .iter()
                .any(|listed| listed.file == *file && listed.commit == *commit)
        }) {
            return Err("review_compose_down_unlisted");
        }
        // The review must be an allowlist `sign` accepts as it is.
        let compose_actions = ComposeAllowlist::from_json(&compose_actions.to_json())
            .map_err(|_| "review_compose_actions_unlistable")?;
        Ok(Allowlist {
            rtmr3: self.rtmr3.unwrap_or(Rtmr3Allowlist {
                app_id: Vec::new(),
                mr_kms: Vec::new(),
                os_image_hash: Vec::new(),
                key_provider: Vec::new(),
                markers: Vec::new(),
            }),
            compose_actions,
        })
    }
}

/// Every model entry for one model, each verified, replayed and its
/// compose-manager record checked, or the label of the first check that
/// refused.
async fn observe_model(
    client: &dyn AttestedKeyReportClient,
    nonce: &str,
    now_unix: u64,
    verify: &(dyn Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync),
) -> Result<Vec<ObservedEntry>, String> {
    // A nonce that is not 32 bytes of hex binds nothing; refuse before the
    // network call.
    if nonce.len() != 64 || !nonce.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("nonce_malformed".to_string());
    }
    let report = client
        .fetch_ed25519_report_json(nonce)
        .await
        .map_err(|error| client_label(&error))?;
    let model = client.model();
    let claimed = model_ed25519_keys(&report, nonce, model)
        .map_err(|error| format!("report_{}", key_label(error)))?;
    // Each entry's quote, log and compose-manager record, read from the one
    // entry, so the log and record checked are those beside the quote.
    let evidence = model_entry_evidence(&report, model)
        .map_err(|error| format!("report_{}", key_label(error)))?;

    let mut observed = Vec::with_capacity(evidence.len());
    for entry in &evidence {
        let collateral = client
            .fetch_collateral_for(&entry.quote)
            .await
            .map_err(|error| client_label(&error))?;
        let checked = verify(&entry.quote, &collateral, now_unix).map_err(|error| {
            match error {
                QuoteVerifyError::CollateralMalformed { .. } => "collateral_malformed",
                QuoteVerifyError::VerificationFailed { .. } => "quote_verification_failed",
                QuoteVerifyError::NotTdx => "quote_not_tdx",
            }
            .to_string()
        })?;
        if checked.tcb_status != REQUIRED_TCB_STATUS {
            return Err("quote_tcb_not_up_to_date".to_string());
        }
        let events = replay_rtmr3(entry.event_log.as_ref(), &checked)
            .map_err(|refusal| refusal.label().to_string())?;
        let record = ComposeManagerRecord::from_entry(&entry.entry)
            .map_err(|refusal| refusal.label().to_string())?;
        let record_collateral = client
            .fetch_collateral_for(&record.quote)
            .await
            .map_err(|error| client_label(&error))?;
        let actions = check_compose_manager(
            &record,
            |quote| verify(quote, &record_collateral, now_unix).ok(),
            &checked,
            nonce,
            None,
        )
        .map_err(|refusal| refusal.label().to_string())?;
        observed.push(ObservedEntry {
            quote: checked,
            events,
            actions,
        });
    }

    let report_data: Vec<&[u8]> = observed
        .iter()
        .map(|entry| entry.quote.report_data.as_slice())
        .collect();
    quote_bound_keys(&claimed, &report_data, nonce)
        .map_err(|error| format!("quote_{}", key_label(error)))?;
    Ok(observed)
}

/// The named registers of a verified quote, in the canonical pin syntax.
fn set_of(quote: &VerifiedQuote, registers: &[MeasurementField]) -> Result<String, String> {
    let raw = registers
        .iter()
        .map(|field| format!("{}={}", field.as_str(), field.read(quote)))
        .collect::<Vec<_>>()
        .join(",");
    match ExpectedMeasurements::from_env_value(Some(&raw)) {
        Ok(Some(set)) => Ok(set.to_pin_string()),
        _ => Err("quote_registers_unreadable".to_string()),
    }
}

/// A transport failure's label: the step and the condition, plus the
/// missing control's name when that is the cause. Never the message.
fn client_label(error: &AttestationClientError) -> String {
    match error.missing_control() {
        Some(control) => format!("{}_missing_control:{control}", error.step().as_str()),
        None => format!("{}_{}", error.step().as_str(), error.code()),
    }
}

fn key_label(error: AttestedKeyError) -> &'static str {
    match error {
        AttestedKeyError::Malformed => "shape",
        AttestedKeyError::NotEd25519 => "not_ed25519",
        AttestedKeyError::NonceMismatch => "nonce_mismatch",
        AttestedKeyError::ReportDataMismatch => "report_data_mismatch",
        AttestedKeyError::KeyMalformed => "key_malformed",
        AttestedKeyError::ModelNotAttested => "model_not_attested",
        AttestedKeyError::SignatureKindUnrecognised => "signature_kind_unrecognised",
    }
}
