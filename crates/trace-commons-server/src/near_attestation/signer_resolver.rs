// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Which ed25519 keys may sign a receipt for a model **right now**, answered
//! by the server rather than by the submitter.
//!
//! # The gap this closes
//!
//! The contributor client already fetches an attestation report with
//! `signing_algo=ed25519`, derives the per-model keys and checks a receipt's
//! signer against them. That is the sender inspecting its own evidence: a
//! submitter who checked nothing looks identical, on the wire, to one who
//! checked honestly. The server's own check has until now been a static pin
//! set (`InferenceAttestationPolicy::model_key_pins`), and the deployed
//! configuration sets none.
//!
//! This module is the server-side answer: a resolver that derives the
//! acceptable signer set from a report **the server fetched**, with a nonce
//! **the server generated**, over a quote **the server DCAP-verified** against
//! collateral **fetched per quote**, whose measurements the server pinned.
//!
//! # What may be claimed from a key this resolver returns
//!
//! Exactly this: *these bytes were produced inside an attested NEAR AI enclave
//! whose image we pinned*. Not which model served them. Every NEAR AI model
//! enclave presents the same `mrtd`/`mrconfigid`/`rtmr0-3`, and the model name
//! is a JSON field beside the quote rather than attested data, so no
//! measurement discriminates one hosted model from another. That is a known
//! and accepted limitation, and it is why the `receipt_bound_model()` /
//! `requested_model()` split in [`crate::admission_evidence`] stays: anything
//! asserting *which* model must keep treating the model as unattested.
//!
//! # The monotonicity rule
//!
//! **Every failure subtracts keys or leaves them alone. Nothing but a
//! verified, measurement-pinned report or an operator pin ever adds one.**
//!
//! A fetch failure, a malformed report, a quote that does not verify,
//! collateral that does not match, a measurement outside the pin set, an
//! absent model entry: each leaves the previously installed set exactly as it
//! was, and the hard ceiling below eventually empties it. There is no path in
//! this module by which a failure widens what is accepted.
//!
//! # Freshness
//!
//! - **1h TTL.** Past it [`ReportBackedResolver::refresh_if_stale`] refetches.
//! - **60s single-flight floor.** A receipt whose signer is in no accepted set
//!   may trigger a refetch through
//!   [`ReportBackedResolver::refresh_on_signer_miss`], but no more often than
//!   once a minute, and concurrent misses collapse into one fetch. Without
//!   both, an unauthenticated route becomes a way to make the server hammer
//!   its provider.
//! - **24h hard ceiling.** Past it the report-derived set reads **empty**, not
//!   stale. A resolver that cannot reach the endpoint for a day is a resolver
//!   that does not know who may sign, and the fail-closed reading of that is
//!   "nobody".
//!
//! Refreshing is off the request path: [`AttestedSignerResolver::keys_for`] is
//! synchronous and reads the installed set. Nothing here fetches while a
//! submission waits.
//!
//! # Operator pins remain, as an override
//!
//! [`OperatorKeyPins`] still lets an operator nail a key. Where a pin set is
//! configured for a kind, it **is** the answer for that kind and the
//! report-derived half is not consulted -- an override, not a union. An
//! operator who nailed a key did not thereby agree to also accept whatever the
//! endpoint attests tomorrow, and the override direction that narrows is the
//! one worth having.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Mutex;

use sha2::{Digest, Sha256};

use super::client::AttestationClientError;
use super::measurements::EXPECTED_MEASUREMENTS_CONTROL;
use super::quote::{Collateral, QuoteVerifyError, VerifiedQuote, verify_quote};
use super::receipt::{
    AttestedKeyError, ReceiptSignatureKind, gateway_ed25519_key, model_ed25519_keys,
};
use trace_commons_attestation::measurements::{ExpectedMeasurements, check_measurements_opt};

/// How long an installed set is served without refetching.
pub const FRESH_TTL_SECS: u64 = 60 * 60;

/// The floor between two signer-miss-triggered refetches.
pub const SIGNER_MISS_FLOOR_SECS: u64 = 60;

/// How stale an installed set may get before it reads empty.
pub const HARD_CEILING_SECS: u64 = 24 * 60 * 60;

/// Where the ed25519 signing key sits in a TDX quote's `report_data`, and
/// where the nonce sits beside it. Read out of a [`VerifiedQuote`], so these
/// are offsets into a structure DCAP verification has already accepted.
const REPORT_DATA_KEY: std::ops::Range<usize> = 0..32;
const REPORT_DATA_NONCE: std::ops::Range<usize> = 32..64;

/// A digest of a public value, for evidence. Keys are public, but they are
/// deployment state and this repository's operational surfaces are hash-only.
fn key_ref(value: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(&Sha256::digest(value.as_bytes())[..8])
    )
}

/// Which ed25519 keys may sign a receipt of a given kind for a given model.
///
/// A trait, and every holder must hold it as a trait object: the static-pin
/// answer and the report-derived answer are the same decision reached two
/// ways, and a concrete type at the point of use cannot participate in that
/// substitution.
pub trait AttestedSignerResolver: Send + Sync {
    /// The accepted signer set, or empty where nothing is accepted.
    ///
    /// An empty answer is a **refusal**, never "unchecked". A caller that
    /// receives one must refuse the receipt.
    ///
    /// `model` is the model the receipt itself binds, and is `None` for a
    /// receipt binding none. `now_unix` is wall-clock time; it is a parameter
    /// rather than a clock call so that staleness is testable.
    fn keys_for(
        &self,
        kind: ReceiptSignatureKind,
        model: Option<&str>,
        now_unix: u64,
    ) -> Vec<String>;

    /// Whether this resolver enforces at all.
    ///
    /// False only for the static resolver of a deployment that pins nothing,
    /// which is the pre-existing dormant default. A resolver that derives its
    /// own keys always enforces: there is no configuration of it that means
    /// "accept any signer".
    fn is_enforcing(&self) -> bool;
}

/// The pre-existing answer: whatever the operator configured, and nothing else.
///
/// Behaviourally identical to the lookup it replaces, including the
/// deliberately empty answer for a kind that is not pinned and for
/// [`ReceiptSignatureKind::Unrecognised`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StaticPinResolver {
    pins: OperatorKeyPins,
}

impl StaticPinResolver {
    /// Build from the pin sets an operator configured.
    pub fn new(pins: OperatorKeyPins) -> Self {
        Self { pins }
    }
}

impl AttestedSignerResolver for StaticPinResolver {
    fn keys_for(
        &self,
        kind: ReceiptSignatureKind,
        model: Option<&str>,
        _now_unix: u64,
    ) -> Vec<String> {
        self.pins.keys_for(kind, model).unwrap_or_default()
    }

    fn is_enforcing(&self) -> bool {
        self.pins.is_pinning()
    }
}

/// Signing keys an operator nailed by hand.
///
/// `None` for a kind means "not pinned", which is not "accept anything": see
/// [`OperatorKeyPins::keys_for`], whose `None` return is the caller's cue to
/// consult the report-derived half.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OperatorKeyPins {
    model: Option<BTreeMap<String, Vec<String>>>,
    gateway: Option<Vec<String>>,
}

impl OperatorKeyPins {
    /// Nothing pinned.
    pub fn none() -> Self {
        Self::default()
    }

    /// Pin the per-model keys.
    pub fn with_model_pins(mut self, pins: BTreeMap<String, Vec<String>>) -> Self {
        self.model = Some(pins);
        self
    }

    /// Pin the gateway keys.
    pub fn with_gateway_pins(mut self, pins: Vec<String>) -> Self {
        self.gateway = Some(pins);
        self
    }

    /// Whether either kind is pinned.
    pub fn is_pinning(&self) -> bool {
        self.model.is_some() || self.gateway.is_some()
    }

    /// The override for this kind, or `None` where this kind is not pinned.
    ///
    /// A pinned kind whose map has no entry for `model` answers
    /// `Some(vec![])` -- pinned, and this model is not among them, which is a
    /// refusal and not a fall-through to the report. An operator who listed
    /// the models they trust did not implicitly trust the rest.
    ///
    /// [`ReceiptSignatureKind::Unrecognised`] names no key source, so it
    /// resolves to a refusal under every configuration. Not a licence to try
    /// every pinned key.
    pub fn keys_for(&self, kind: ReceiptSignatureKind, model: Option<&str>) -> Option<Vec<String>> {
        match kind {
            ReceiptSignatureKind::ProviderTee => {
                let pins = self.model.as_ref()?;
                Some(
                    model
                        .and_then(|model| pins.get(model))
                        .cloned()
                        .unwrap_or_default(),
                )
            }
            ReceiptSignatureKind::Gateway => self.gateway.clone(),
            ReceiptSignatureKind::Unrecognised => Some(Vec::new()),
        }
    }
}

/// The two calls a resolver makes against the attestation endpoint.
///
/// A trait so the resolver can be driven from captures. It is deliberately
/// narrow: this seam may fetch a nonce-parameterised ed25519 report and
/// collateral for a quote, and may do nothing else. In particular it cannot
/// spend a completion, which is what keeps a refresh from being metered.
#[async_trait::async_trait]
pub trait AttestedReportSource: Send + Sync {
    /// The model whose per-model attestation this source reads.
    fn model(&self) -> &str;

    /// `GET {base}/attestation/report?model=..&nonce=..&signing_algo=ed25519`,
    /// raw body.
    ///
    /// The raw body, not a parsed [`super::AttestationReport`]: that type
    /// models the single-enclave shape and carries no `model_attestations`
    /// field at all, and the key readers take the report JSON as a string.
    /// `signing_algo=ed25519` is load-bearing -- without it the endpoint
    /// answers with ECDSA attestations, which carry no receipt-signing key.
    async fn fetch_ed25519_report_json(
        &self,
        nonce: &str,
    ) -> Result<String, AttestationClientError>;

    /// Intel DCAP collateral for **this** quote.
    ///
    /// Per quote, not per endpoint. Collateral is platform-specific: the
    /// checked-in gateway collateral does not verify a model enclave's quote
    /// -- it fails with an invalid QE report signature, because the PCK chain
    /// belongs to a different machine. Measured, not assumed; see the tests.
    async fn fetch_collateral_for(
        &self,
        quote: &[u8],
    ) -> Result<Collateral, AttestationClientError>;
}

/// Where the challenge comes from.
///
/// A trait, not a `&str`, because the resolver's central claim is that **the
/// server** chose the nonce. A caller-supplied nonce would put the choice one
/// indirection away from the party being challenged.
pub trait ResolverNonceSource: Send + Sync {
    /// A fresh 64-hex-character nonce.
    fn nonce(&self) -> String;
}

/// The production nonce source: 32 fresh random bytes per refresh.
#[derive(Debug, Clone, Copy, Default)]
pub struct RandomResolverNonce;

impl ResolverNonceSource for RandomResolverNonce {
    fn nonce(&self) -> String {
        super::drill::generate_drill_nonce()
    }
}

/// The measurement pins a report-derived key must satisfy.
///
/// Two sets, because the gateway enclave and a model enclave are different
/// images and a single pin set would refuse one of them. Either being `None`
/// means that half accepts nothing: [`check_measurements_opt`] answers
/// `Refused` for an absent set, and this module treats that as the refusal it
/// is rather than as permission to skip the check.
#[derive(Debug, Clone, Default)]
pub struct ResolverMeasurementPins {
    /// Pins for a model enclave's quote.
    pub model: Option<ExpectedMeasurements>,
    /// Pins for the gateway enclave's quote.
    pub gateway: Option<ExpectedMeasurements>,
}

/// Why one half of a refresh installed nothing.
///
/// Labels, not messages: every one of these is reachable from an
/// unauthenticated submission's timing, and none of them may carry endpoint
/// text, a URL, or key material.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum RefreshRefusal {
    /// The nonce source produced something that is not 64 hex characters. No
    /// call is made.
    #[error("nonce_source_malformed")]
    NonceSourceMalformed,
    /// The endpoint could not be reached, or answered non-success.
    #[error("report_unavailable")]
    ReportUnavailable,
    /// The body is not the report shape this half needs.
    #[error("report_shape")]
    ReportShape,
    /// An attestation naming this half failed its key-and-nonce binding.
    #[error("key_binding_failed")]
    KeyBindingFailed,
    /// The report attests no key for the model asked about.
    #[error("model_not_attested")]
    ModelNotAttested,
    /// Collateral for one of the quotes could not be fetched or parsed.
    #[error("collateral_unavailable")]
    CollateralUnavailable,
    /// A quote did not verify against the collateral fetched for it.
    #[error("quote_unverified")]
    QuoteUnverified,
    /// A verified quote's measurements are outside the pinned set, or nothing
    /// is pinned.
    #[error("measurements_not_pinned")]
    MeasurementsNotPinned,
    /// A verified quote's `report_data` does not commit to the key the JSON
    /// named, or commits to a nonce we did not send.
    #[error("key_not_in_verified_quote")]
    KeyNotInVerifiedQuote,
}

/// What one half of a refresh did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HalfOutcome {
    /// Keys were derived and installed, replacing whatever that half held.
    Installed {
        /// Digests of the installed keys, in the order derived.
        key_refs: Vec<String>,
    },
    /// Nothing was installed and the half was left exactly as it was.
    Refused(RefreshRefusal),
}

impl HalfOutcome {
    /// Whether this half installed a set.
    pub fn installed(&self) -> bool {
        matches!(self, HalfOutcome::Installed { .. })
    }

    /// The refusal, where there was one.
    pub fn refusal(&self) -> Option<RefreshRefusal> {
        match self {
            HalfOutcome::Refused(refusal) => Some(*refusal),
            HalfOutcome::Installed { .. } => None,
        }
    }
}

/// What a whole refresh did. Hash-only: counts, digests and labels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RefreshOutcome {
    /// The `provider_tee` half.
    pub model: HalfOutcome,
    /// The `gateway` half.
    pub gateway: HalfOutcome,
}

impl RefreshOutcome {
    fn both(refusal: RefreshRefusal) -> Self {
        Self {
            model: HalfOutcome::Refused(refusal),
            gateway: HalfOutcome::Refused(refusal),
        }
    }
}

/// The installed sets and the clocks that bound them.
#[derive(Debug, Default)]
struct ResolverState {
    model_keys: BTreeMap<String, Vec<String>>,
    model_installed_at: Option<u64>,
    gateway_keys: Vec<String>,
    gateway_installed_at: Option<u64>,
    /// When a refresh was last *attempted*, successful or not. The
    /// single-flight floor is measured from here, so a failing endpoint
    /// cannot be retried faster than a succeeding one.
    last_attempt_at: Option<u64>,
}

/// A resolver whose answers come from a report the server fetched and verified.
///
/// See the module documentation for the monotonicity rule, the freshness
/// windows, and what may be claimed from a key this returns.
pub struct ReportBackedResolver {
    source: Box<dyn AttestedReportSource>,
    nonces: Box<dyn ResolverNonceSource>,
    pins: ResolverMeasurementPins,
    overrides: OperatorKeyPins,
    state: Mutex<ResolverState>,
    /// Held across a refresh so concurrent signer misses collapse into one
    /// fetch rather than one fetch each.
    refresh_lock: tokio::sync::Mutex<()>,
}

/// Redacted: the installed sets are keys, and this type is held by code that
/// logs. Counts and staleness only.
impl fmt::Debug for ReportBackedResolver {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.state.lock().expect("resolver state lock");
        f.debug_struct("ReportBackedResolver")
            .field("model_key_models", &state.model_keys.len())
            .field("gateway_key_count", &state.gateway_keys.len())
            .field("model_installed_at", &state.model_installed_at)
            .field("gateway_installed_at", &state.gateway_installed_at)
            .finish()
    }
}

impl ReportBackedResolver {
    /// Build a resolver. Nothing is fetched here: a freshly constructed
    /// resolver has installed nothing and therefore accepts nothing.
    pub fn new(
        source: Box<dyn AttestedReportSource>,
        nonces: Box<dyn ResolverNonceSource>,
        pins: ResolverMeasurementPins,
        overrides: OperatorKeyPins,
    ) -> Self {
        Self {
            source,
            nonces,
            pins,
            overrides,
            state: Mutex::new(ResolverState::default()),
            refresh_lock: tokio::sync::Mutex::new(()),
        }
    }

    /// Whether the report-derived halves are past their 1h TTL.
    ///
    /// True for a resolver that has never installed anything, which is what
    /// makes the first refresh happen.
    pub fn needs_refresh(&self, now_unix: u64) -> bool {
        let state = self.state.lock().expect("resolver state lock");
        [state.model_installed_at, state.gateway_installed_at]
            .iter()
            .any(|installed| match installed {
                None => true,
                Some(at) => now_unix.saturating_sub(*at) >= FRESH_TTL_SECS,
            })
    }

    /// Refresh if the TTL has elapsed. `None` where it has not.
    pub async fn refresh_if_stale(&self, now_unix: u64) -> Option<RefreshOutcome> {
        if !self.needs_refresh(now_unix) {
            return None;
        }
        Some(self.refresh(now_unix).await)
    }

    /// Refresh because a receipt's signer was in no accepted set.
    ///
    /// `None` where the 60s floor has not elapsed since the last attempt.
    /// Concurrent callers collapse: the floor is re-read under the refresh
    /// lock, so the second caller through the door sees the first's attempt
    /// and declines.
    pub async fn refresh_on_signer_miss(&self, now_unix: u64) -> Option<RefreshOutcome> {
        if !self.miss_floor_elapsed(now_unix) {
            return None;
        }
        let guard = self.refresh_lock.lock().await;
        if !self.miss_floor_elapsed(now_unix) {
            return None;
        }
        let outcome = self.refresh_locked(now_unix).await;
        drop(guard);
        Some(outcome)
    }

    fn miss_floor_elapsed(&self, now_unix: u64) -> bool {
        let state = self.state.lock().expect("resolver state lock");
        match state.last_attempt_at {
            None => true,
            Some(at) => now_unix.saturating_sub(at) >= SIGNER_MISS_FLOOR_SECS,
        }
    }

    /// Fetch, verify and install. Every failure leaves the affected half
    /// exactly as it was; see the module's monotonicity rule.
    pub async fn refresh(&self, now_unix: u64) -> RefreshOutcome {
        let guard = self.refresh_lock.lock().await;
        let outcome = self.refresh_locked(now_unix).await;
        drop(guard);
        outcome
    }

    async fn refresh_locked(&self, now_unix: u64) -> RefreshOutcome {
        self.state
            .lock()
            .expect("resolver state lock")
            .last_attempt_at = Some(now_unix);

        let nonce = self.nonces.nonce();
        // Checked before anything goes on the network: a challenge we cannot
        // recognise in the answer is not a challenge, and sending it would
        // spend a call to learn nothing.
        if nonce.len() != 64 || hex::decode(&nonce).is_err() {
            return RefreshOutcome::both(RefreshRefusal::NonceSourceMalformed);
        }
        let nonce = nonce.to_ascii_lowercase();

        let report_json = match self.source.fetch_ed25519_report_json(&nonce).await {
            Ok(body) => body,
            Err(_) => return RefreshOutcome::both(RefreshRefusal::ReportUnavailable),
        };

        let model = self.source.model().to_string();
        let model_half = self
            .derive_model_half(&report_json, &nonce, &model, now_unix)
            .await;
        let gateway_half = self
            .derive_gateway_half(&report_json, &nonce, now_unix)
            .await;

        let mut state = self.state.lock().expect("resolver state lock");
        if let Ok(keys) = &model_half {
            // Replace, never merge: a rotation must remove the old key, and a
            // union would keep every key the endpoint has ever attested.
            state.model_keys.insert(model.clone(), keys.clone());
            state.model_installed_at = Some(now_unix);
        }
        if let Ok(keys) = &gateway_half {
            state.gateway_keys = keys.clone();
            state.gateway_installed_at = Some(now_unix);
        }
        drop(state);

        RefreshOutcome {
            model: half_outcome(model_half),
            gateway: half_outcome(gateway_half),
        }
    }

    /// The `provider_tee` half: the per-model keys.
    async fn derive_model_half(
        &self,
        report_json: &str,
        nonce: &str,
        model: &str,
        now_unix: u64,
    ) -> Result<Vec<String>, RefreshRefusal> {
        // The JSON-level binding first. It is not the thing trusted -- the
        // verified quote below is -- but it is what enforces
        // `signing_algo == ed25519`, and that is load-bearing: `report_data`
        // of an ECDSA attestation is a 20-byte address and 12 zero bytes, so
        // reading bytes 0..32 out of one would manufacture a 32-byte "key"
        // that no ed25519 signer holds.
        let claimed = model_ed25519_keys(report_json, nonce, model).map_err(key_error_refusal)?;
        let quotes = model_entry_quotes(report_json, model)?;
        let verified = self
            .verify_each(&quotes, self.pins.model.as_ref(), now_unix)
            .await?;
        reconcile(&claimed, &verified, nonce)
    }

    /// The `gateway` half: the one key the gateway attestation carries.
    async fn derive_gateway_half(
        &self,
        report_json: &str,
        nonce: &str,
        now_unix: u64,
    ) -> Result<Vec<String>, RefreshRefusal> {
        let claimed = vec![gateway_ed25519_key(report_json, nonce).map_err(key_error_refusal)?];
        let quotes = gateway_quotes(report_json)?;
        let verified = self
            .verify_each(&quotes, self.pins.gateway.as_ref(), now_unix)
            .await?;
        reconcile(&claimed, &verified, nonce)
    }

    /// DCAP-verify every quote and pin its measurements. Collateral is fetched
    /// **per quote**.
    async fn verify_each(
        &self,
        quotes: &[Vec<u8>],
        expected: Option<&ExpectedMeasurements>,
        now_unix: u64,
    ) -> Result<Vec<VerifiedQuote>, RefreshRefusal> {
        let mut verified = Vec::with_capacity(quotes.len());
        for quote in quotes {
            let collateral = self
                .source
                .fetch_collateral_for(quote)
                .await
                .map_err(|_| RefreshRefusal::CollateralUnavailable)?;
            let quote = verify_quote(quote, &collateral, now_unix).map_err(quote_refusal)?;
            // Measurements are the durable pin; keys rotate. An absent pin set
            // answers `Refused`, which is a refusal here and not a skip.
            if !check_measurements_opt(expected, &quote, EXPECTED_MEASUREMENTS_CONTROL).is_pinned()
            {
                return Err(RefreshRefusal::MeasurementsNotPinned);
            }
            verified.push(quote);
        }
        Ok(verified)
    }
}

impl AttestedSignerResolver for ReportBackedResolver {
    fn keys_for(
        &self,
        kind: ReceiptSignatureKind,
        model: Option<&str>,
        now_unix: u64,
    ) -> Vec<String> {
        // An operator pin for this kind is the answer for this kind. See the
        // module note on why this is an override and not a union.
        if let Some(pinned) = self.overrides.keys_for(kind, model) {
            return pinned;
        }
        let state = self.state.lock().expect("resolver state lock");
        let (installed_at, keys) = match kind {
            ReceiptSignatureKind::ProviderTee => (
                state.model_installed_at,
                model
                    .and_then(|model| state.model_keys.get(model))
                    .cloned()
                    .unwrap_or_default(),
            ),
            ReceiptSignatureKind::Gateway => {
                (state.gateway_installed_at, state.gateway_keys.clone())
            }
            // Names no key source, so there is nothing to answer with.
            ReceiptSignatureKind::Unrecognised => return Vec::new(),
        };
        match installed_at {
            // Never installed: not "unchecked", refused.
            None => Vec::new(),
            // Past the ceiling the set reads empty rather than stale.
            Some(at) if now_unix.saturating_sub(at) > HARD_CEILING_SECS => Vec::new(),
            Some(_) => keys,
        }
    }

    fn is_enforcing(&self) -> bool {
        // Always. There is no configuration of this resolver that means
        // "accept any signer": with nothing installed and nothing pinned it
        // answers empty, which refuses.
        true
    }
}

fn half_outcome(half: Result<Vec<String>, RefreshRefusal>) -> HalfOutcome {
    match half {
        Ok(keys) => HalfOutcome::Installed {
            key_refs: keys.iter().map(|key| key_ref(key)).collect(),
        },
        Err(refusal) => HalfOutcome::Refused(refusal),
    }
}

/// The keys a set of verified quotes commits to, cross-checked against what
/// the report's JSON claimed.
///
/// Three conditions, all of which must hold:
///
/// 1. every verified quote's `report_data[32..64]` is the nonce we sent;
/// 2. every verified quote's `report_data[0..32]` is a key the JSON claimed;
/// 3. every key the JSON claimed appears in some verified quote.
///
/// (3) is what stops a report from mixing one entry over a verifiable quote
/// with a second entry whose key nothing verified: without it the sound
/// entry's pass would carry the forged one. The comparison is over **sets**,
/// not a positional zip: two separate walks of the report produced the two
/// lists, and a zip would quietly truncate on exactly the divergence worth
/// catching.
///
/// What is returned is the verified-quote set, not the claimed set. They are
/// equal by the time this returns; returning the verified one is the habit
/// that survives an edit to the conditions above.
fn reconcile(
    claimed: &[String],
    verified: &[VerifiedQuote],
    nonce: &str,
) -> Result<Vec<String>, RefreshRefusal> {
    let mut derived = Vec::with_capacity(verified.len());
    for quote in verified {
        let key = quote
            .report_data
            .get(REPORT_DATA_KEY)
            .ok_or(RefreshRefusal::KeyNotInVerifiedQuote)?;
        let bound_nonce = quote
            .report_data
            .get(REPORT_DATA_NONCE)
            .ok_or(RefreshRefusal::KeyNotInVerifiedQuote)?;
        if hex::encode(bound_nonce) != nonce {
            return Err(RefreshRefusal::KeyNotInVerifiedQuote);
        }
        derived.push(hex::encode(key));
    }
    let claimed_lower: Vec<String> = claimed.iter().map(|k| k.to_ascii_lowercase()).collect();
    if derived.iter().any(|key| !claimed_lower.contains(key)) {
        return Err(RefreshRefusal::KeyNotInVerifiedQuote);
    }
    if claimed_lower.iter().any(|key| !derived.contains(key)) {
        return Err(RefreshRefusal::KeyNotInVerifiedQuote);
    }
    Ok(derived)
}

/// The `intel_quote` bytes of every `model_attestations` entry naming `model`.
///
/// An entry for this model carrying no readable quote is a refusal, not a
/// skip: skipping it would leave the resolver verifying some other entry's
/// quote and installing a key it never checked.
fn model_entry_quotes(report_json: &str, model: &str) -> Result<Vec<Vec<u8>>, RefreshRefusal> {
    let document: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| RefreshRefusal::ReportShape)?;
    let entries = document
        .get("model_attestations")
        .and_then(serde_json::Value::as_array)
        .ok_or(RefreshRefusal::ReportShape)?;
    let mut quotes = Vec::new();
    for entry in entries {
        if entry.get("model_name").and_then(serde_json::Value::as_str) != Some(model) {
            continue;
        }
        let hex_quote = entry
            .get("intel_quote")
            .and_then(serde_json::Value::as_str)
            .ok_or(RefreshRefusal::ReportShape)?;
        quotes.push(hex::decode(hex_quote).map_err(|_| RefreshRefusal::ReportShape)?);
    }
    if quotes.is_empty() {
        return Err(RefreshRefusal::ModelNotAttested);
    }
    Ok(quotes)
}

/// The gateway attestation's own quote. Exactly one, or a refusal.
fn gateway_quotes(report_json: &str) -> Result<Vec<Vec<u8>>, RefreshRefusal> {
    let document: serde_json::Value =
        serde_json::from_str(report_json).map_err(|_| RefreshRefusal::ReportShape)?;
    let hex_quote = document
        .get("gateway_attestation")
        .and_then(|gateway| gateway.get("intel_quote"))
        .and_then(serde_json::Value::as_str)
        .ok_or(RefreshRefusal::ReportShape)?;
    Ok(vec![
        hex::decode(hex_quote).map_err(|_| RefreshRefusal::ReportShape)?,
    ])
}

fn key_error_refusal(error: AttestedKeyError) -> RefreshRefusal {
    match error {
        AttestedKeyError::Malformed => RefreshRefusal::ReportShape,
        AttestedKeyError::ModelNotAttested => RefreshRefusal::ModelNotAttested,
        AttestedKeyError::NotEd25519
        | AttestedKeyError::NonceMismatch
        | AttestedKeyError::ReportDataMismatch
        | AttestedKeyError::KeyMalformed
        | AttestedKeyError::SignatureKindUnrecognised => RefreshRefusal::KeyBindingFailed,
    }
}

fn quote_refusal(error: QuoteVerifyError) -> RefreshRefusal {
    match error {
        QuoteVerifyError::CollateralMalformed { .. } => RefreshRefusal::CollateralUnavailable,
        QuoteVerifyError::VerificationFailed { .. } | QuoteVerifyError::NotTdx => {
            RefreshRefusal::QuoteUnverified
        }
    }
}

#[cfg(test)]
#[path = "signer_resolver_tests.rs"]
mod tests;
