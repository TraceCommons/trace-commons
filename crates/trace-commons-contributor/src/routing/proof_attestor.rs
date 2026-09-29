//! The attestor IronWire asks before it calls an answer "proof ✓".
//!
//! IronWire checks a NEAR AI receipt itself -- signature, digests, model,
//! `provider_tee` kind -- but it carries no DCAP quote verifier, so on its own
//! the best a row reaches is `unattested`. It hands the last step to its host
//! through `ironwire_proxy::proof::SignerAttestor`, and this is ours.
//!
//! For a model, the answer is the set of per-model `provider_tee` ed25519 keys
//! that are bound by a TDX quote which
//!
//! 1. we asked for with a nonce we just generated
//!    (`GET {gateway}/attestation/report?model=..&signing_algo=ed25519&nonce=..`),
//! 2. passes Intel DCAP verification against collateral ingest fetched for it
//!    (`crate::witness::transport::fetch_collateral`), at the current time,
//! 3. runs an image whose measurements match a pinned set (see "Where the
//!    pins come from" below), and
//! 4. commits, in its verified `report_data`, to exactly the keys the report's
//!    JSON claims for the model, under our nonce
//!    (`trace_commons_attestation::receipt::quote_bound_keys`, the same rule
//!    the server's signer resolver applies).
//!
//! **Only per-model keys, never the gateway's.** The keys come from
//! `model_ed25519_keys`, which reads the model-attestation container and
//! refuses a gateway-only report. A gateway key signs `gateway` receipts,
//! which name the relay, not the model.
//!
//! What a `verified` row then means, and no more: *these bytes were signed
//! inside a NEAR AI model enclave whose image we pinned*. NEAR AI model
//! enclaves share measurements and the model name is a JSON field beside the
//! quote, so which model is still NEAR AI's statement -- the accepted limit
//! the rest of this repo documents.
//!
//! # Failure only ever subtracts
//!
//! No pins, a report we cannot fetch, a quote that does not verify, a
//! measurement outside the pins, a binding that does not reconcile: each
//! answers something other than keys, and IronWire records the row as not
//! proof. A key set is cached for [`FRESH_TTL`] and then must be earned again;
//! a failed refresh does not extend an old one.
//!
//! # Where the pins come from
//!
//! **Ingest publishes them.** `GET /v1/contributors/me/near-ai-measurements`
//! (device credential) returns the set ingest itself enforces, so the device
//! and ingest agree on which NEAR AI image counts. [`PinProvider`] caches the
//! answer: it refreshes after [`PIN_REFRESH_AFTER`], a failed refresh keeps
//! the last good set only until [`PIN_TTL`] after it was fetched, and past
//! that the device pins nothing. A published `unconfigured` or `invalid`
//! state, or a document whose digest does not match its sets, also pins
//! nothing.
//!
//! **`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` overrides it**, for an
//! operator or a developer: when the variable is set at daemon start it wins
//! outright and ingest is not asked. Set but empty or malformed, it pins
//! nothing -- it never falls back to the published set, because an operator
//! who set it meant it. The syntax is the server's own (`mrconfigid=<96
//! hex>,...`, sets split by `;`).
//!
//! No pins, whichever way, means no key can be earned: nothing verifies.
//!
//! Nothing here logs a key, a nonce, a model, a report or a URL.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironwire_proxy::proof::{Attestation, SignerAttestor};
use trace_commons_attestation::measurements::{
    ExpectedMeasurements, MeasurementVerdict, check_measurements_opt,
};
use trace_commons_attestation::quote::{
    Collateral, REQUIRED_TCB_STATUS, VerifiedQuote, verify_quote,
};
use trace_commons_attestation::receipt::{AttestedKeyError, model_entry_quotes, quote_bound_keys};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::near_ai_measurements::NearAiMeasurementPins;

/// The IronWire backend id whose receipts this attestor can speak for.
pub const NEAR_AI_BACKEND: &str = "nearai";

/// The NEAR AI gateway: the only host that publishes the per-model
/// attestation registry (a per-model completions host answers with its own
/// single enclave and no registry). Also IronWire's default NEAR AI base, so
/// the key sent here is the key IronWire already sends here.
pub const NEAR_AI_GATEWAY: &str = "https://cloud-api.near.ai/v1";

/// The measurement pins, in the server's variable and syntax.
pub const NEAR_AI_EXPECTED_MEASUREMENTS_ENV: &str = "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS";

/// The control name a missing pin is refused under.
const CONTROL: &str = "near_ai_expected_measurements";

/// How long an earned key set is served before it must be earned again.
pub const FRESH_TTL: Duration = Duration::from_secs(60 * 60);

/// How old a published pin set may get before it is fetched again.
pub const PIN_REFRESH_AFTER: Duration = Duration::from_secs(10 * 60);

/// How long a published pin set is trusted after it was fetched, however
/// refreshes go. Past this with no successful refresh, the device pins
/// nothing.
pub const PIN_TTL: Duration = Duration::from_secs(60 * 60);

/// The least time between two failed pin fetches, so a daemon whose ingest is
/// down does not ask on every pass.
pub const PIN_RETRY_FLOOR: Duration = Duration::from_secs(60);

/// Why the network half could not answer. Carries nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceUnavailable;

/// The network half: the report and the collateral. A trait so the checks can
/// be tested against real captured quotes without a network.
#[async_trait::async_trait]
pub trait AttestationSource: Send + Sync {
    /// `GET /attestation/report` for `model` under `nonce`, raw JSON.
    async fn report(&self, model: &str, nonce: &str) -> Result<String, SourceUnavailable>;
    /// Intel collateral for one quote.
    async fn collateral(&self, quote: &[u8]) -> Result<Collateral, SourceUnavailable>;
}

/// The production source: the NEAR AI gateway for the report, ingest for the
/// collateral, both behind the contributor's host allowlist.
pub struct HttpAttestationSource {
    http: reqwest::Client,
    allowlist: HostAllowlist,
    gateway: String,
    collateral: Option<url::Url>,
    /// The contributor's NEAR AI inference key. The gateway refuses the
    /// registry without one. Sent only to `gateway`.
    key: String,
}

impl HttpAttestationSource {
    /// Build one. `ingest_url` is the enrolled ingest base, whose origin
    /// serves `/v1/attestation-collateral`.
    ///
    /// The allowlist is the contributor's: an operator-enforced list is used
    /// as it stands, so an operator who narrowed egress keeps it narrowed. The
    /// derived default admits the gateway -- where IronWire already sends this
    /// key -- and ingest.
    ///
    /// # Errors
    ///
    /// [`SourceUnavailable`] when the HTTP client cannot be built.
    pub fn new(
        configured: &HostAllowlist,
        ingest_url: &str,
        key: String,
    ) -> Result<Self, SourceUnavailable> {
        let collateral = crate::config::ingest_origin_url(
            ingest_url,
            crate::witness::transport::COLLATERAL_PATH,
        )
        .ok();
        let allowlist = if configured.is_enforcing() {
            configured.clone()
        } else {
            let hosts = [Some(NEAR_AI_GATEWAY), Some(ingest_url)]
                .into_iter()
                .flatten()
                .filter_map(|url| url::Url::parse(url).ok()?.host_str().map(str::to_string));
            HostAllowlist::from_hosts(hosts)
        };
        Ok(Self {
            http: crate::routing::receipt::receipt_http_client().map_err(|_| SourceUnavailable)?,
            allowlist,
            gateway: NEAR_AI_GATEWAY.to_string(),
            collateral,
            key,
        })
    }
}

impl std::fmt::Debug for HttpAttestationSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpAttestationSource")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl AttestationSource for HttpAttestationSource {
    async fn report(&self, model: &str, nonce: &str) -> Result<String, SourceUnavailable> {
        crate::routing::receipt::fetch_attestation_report(
            &self.http,
            &self.allowlist,
            &self.gateway,
            model,
            nonce,
            Some(&self.key),
        )
        .await
        .map_err(|_| SourceUnavailable)
    }

    async fn collateral(&self, quote: &[u8]) -> Result<Collateral, SourceUnavailable> {
        let url = self.collateral.clone().ok_or(SourceUnavailable)?;
        self.allowlist.check(&url).map_err(|_| SourceUnavailable)?;
        crate::witness::transport::fetch_collateral(&self.http, url, quote)
            .await
            .map_err(|_| SourceUnavailable)
    }
}

/// Where published pins come from: ingest's
/// `GET /v1/contributors/me/near-ai-measurements`.
#[async_trait::async_trait]
pub trait PinSource: Send + Sync {
    /// The published document.
    async fn fetch(&self) -> Result<NearAiMeasurementPins, SourceUnavailable>;
}

/// The production source: ingest, with this device's own credential.
pub struct IngestPinSource {
    store: crate::config::ConfigStore,
}

impl IngestPinSource {
    /// Read through `store`'s enrollment and device identity.
    #[must_use]
    pub fn new(store: crate::config::ConfigStore) -> Self {
        Self { store }
    }
}

#[async_trait::async_trait]
impl PinSource for IngestPinSource {
    async fn fetch(&self) -> Result<NearAiMeasurementPins, SourceUnavailable> {
        let cfg = self
            .store
            .load_config()
            .ok()
            .flatten()
            .ok_or(SourceUnavailable)?;
        crate::submit::near_ai_measurement_pins(&self.store, &cfg)
            .await
            .map_err(|_| SourceUnavailable)
    }
}

/// The pins in force right now, and a digest to tell when they changed.
#[derive(Debug, Clone, Default)]
pub struct CurrentPins {
    /// Any one matching admits an image. Empty admits none.
    pub sets: Vec<ExpectedMeasurements>,
    /// Changes whenever `sets` does.
    pub digest: String,
}

struct PublishedCache {
    pins: CurrentPins,
    fetched_at: u64,
}

/// The attestor's pins: the operator override when one is set, else what
/// ingest publishes, cached with a bounded TTL. See the module docs for the
/// precedence and the failure rule.
pub struct PinProvider {
    override_pins: Option<Vec<ExpectedMeasurements>>,
    source: Option<Box<dyn PinSource>>,
    cache: tokio::sync::Mutex<(Option<PublishedCache>, Option<u64>)>,
}

impl std::fmt::Debug for PinProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinProvider")
            .field("overridden", &self.override_pins.is_some())
            .finish_non_exhaustive()
    }
}

impl PinProvider {
    /// `override_pins` wins when `Some`, even if empty. `source` is asked
    /// only without one.
    #[must_use]
    pub fn new(
        override_pins: Option<Vec<ExpectedMeasurements>>,
        source: Option<Box<dyn PinSource>>,
    ) -> Self {
        Self {
            override_pins,
            source,
            cache: tokio::sync::Mutex::new((None, None)),
        }
    }

    /// The pins in force at `now_unix`.
    pub async fn current(&self, now_unix: u64) -> CurrentPins {
        if let Some(pins) = &self.override_pins {
            return CurrentPins {
                sets: pins.clone(),
                digest: format!("override:{}", digest_sets(pins)),
            };
        }
        let Some(source) = &self.source else {
            return CurrentPins::default();
        };
        let mut guard = self.cache.lock().await;
        let (cache, last_failure) = &mut *guard;
        let age = cache
            .as_ref()
            .map(|c| now_unix.saturating_sub(c.fetched_at));
        if age.is_some_and(|age| age < PIN_REFRESH_AFTER.as_secs()) {
            return cache.as_ref().map(|c| c.pins.clone()).unwrap_or_default();
        }
        let backing_off =
            last_failure.is_some_and(|at| now_unix.saturating_sub(at) < PIN_RETRY_FLOOR.as_secs());
        if !backing_off {
            match source.fetch().await {
                Ok(document) => {
                    *last_failure = None;
                    let pins = published_to_current(&document);
                    *cache = Some(PublishedCache {
                        pins: pins.clone(),
                        fetched_at: now_unix,
                    });
                    return pins;
                }
                Err(SourceUnavailable) => *last_failure = Some(now_unix),
            }
        }
        // No fresh answer: the last good set stands only inside its TTL.
        match cache {
            Some(c) if now_unix.saturating_sub(c.fetched_at) < PIN_TTL.as_secs() => c.pins.clone(),
            _ => {
                *cache = None;
                CurrentPins::default()
            }
        }
    }
}

/// A published document as the pins it grants. Anything but a configured,
/// self-consistent document whose every set parses grants none.
fn published_to_current(document: &NearAiMeasurementPins) -> CurrentPins {
    let parsed: Option<Vec<ExpectedMeasurements>> = document
        .usable_sets()
        .iter()
        .map(|set| {
            ExpectedMeasurements::from_env_value(Some(set))
                .ok()
                .flatten()
        })
        .collect();
    match parsed {
        Some(sets) => CurrentPins {
            sets,
            digest: document.digest.clone(),
        },
        None => CurrentPins {
            sets: Vec::new(),
            digest: document.digest.clone(),
        },
    }
}

fn digest_sets(sets: &[ExpectedMeasurements]) -> String {
    sets.iter()
        .map(ExpectedMeasurements::to_pin_string)
        .collect::<Vec<_>>()
        .join(";")
}

/// Whether a verified quote's image matches any pinned set. An empty list
/// matches nothing.
#[must_use]
pub fn image_is_pinned(pins: &[ExpectedMeasurements], quote: &VerifiedQuote) -> bool {
    pins.iter().any(|set| {
        matches!(
            check_measurements_opt(Some(set), quote, CONTROL),
            MeasurementVerdict::Pinned { .. }
        )
    })
}

/// The operator override, read from the environment. `None` when the
/// variable is unset, which lets the published pins apply. Set but empty or
/// malformed is `Some(vec![])`: it wins and pins nothing.
#[must_use]
pub fn pins_from_env() -> Option<Vec<ExpectedMeasurements>> {
    let raw = std::env::var(NEAR_AI_EXPECTED_MEASUREMENTS_ENV).ok()?;
    Some(parse_pins(&raw).unwrap_or_else(|| {
        tracing::warn!(
            control = CONTROL,
            "NEAR AI measurement override did not parse; no answer will be proof"
        );
        Vec::new()
    }))
}

/// `;`-separated sets, each in `ExpectedMeasurements`' own `key=value,`
/// syntax. `None` when any set is malformed: a partly parsed pin list is a
/// list that believes it pins something it does not.
#[must_use]
pub fn parse_pins(raw: &str) -> Option<Vec<ExpectedMeasurements>> {
    let mut sets = Vec::new();
    for entry in raw.split(';').map(str::trim).filter(|e| !e.is_empty()) {
        sets.push(ExpectedMeasurements::from_env_value(Some(entry)).ok()??);
    }
    Some(sets)
}

type Clock = Box<dyn Fn() -> u64 + Send + Sync>;
type Nonces = Box<dyn Fn() -> Option<String> + Send + Sync>;
type QuoteVerifier = Box<dyn Fn(&[u8], &Collateral, u64) -> Option<VerifiedQuote> + Send + Sync>;

/// The attestor. Cheap to clone; clones share one cache.
#[derive(Clone)]
pub struct NearAiQuoteAttestor {
    inner: Arc<Inner>,
}

struct Inner {
    source: Box<dyn AttestationSource>,
    pins: Arc<PinProvider>,
    now_unix: Clock,
    nonce: Nonces,
    verify: QuoteVerifier,
    /// model -> (keys, earned at, digest of the pins they were earned under).
    cache: Mutex<HashMap<String, (Vec<String>, u64, String)>>,
    /// One refresh at a time, so a burst of rows for one model costs one
    /// report fetch rather than one each.
    refresh: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for NearAiQuoteAttestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearAiQuoteAttestor")
            .finish_non_exhaustive()
    }
}

impl NearAiQuoteAttestor {
    /// An attestor over `source`, admitting quotes whose image matches the
    /// pins `pins` has in force when it asks.
    #[must_use]
    pub fn new(source: Box<dyn AttestationSource>, pins: Arc<PinProvider>) -> Self {
        Self::with_parts(
            source,
            pins,
            Box::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_secs())
            }),
            Box::new(|| crate::routing::receipt::fresh_nonce_hex().ok()),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        )
    }

    fn with_parts(
        source: Box<dyn AttestationSource>,
        pins: Arc<PinProvider>,
        now_unix: Clock,
        nonce: Nonces,
        verify: QuoteVerifier,
    ) -> Self {
        Self {
            inner: Arc::new(Inner {
                source,
                pins,
                now_unix,
                nonce,
                verify,
                cache: Mutex::new(HashMap::new()),
                refresh: tokio::sync::Mutex::new(()),
            }),
        }
    }

    /// Keys earned under exactly these pins, still inside their TTL. A set
    /// earned under pins that have since changed is not served.
    fn cached(&self, model: &str, now: u64, pins_digest: &str) -> Option<Vec<String>> {
        let cache = self.inner.cache.lock().ok()?;
        let (keys, at, digest) = cache.get(model)?;
        (now.saturating_sub(*at) < FRESH_TTL.as_secs() && digest == pins_digest)
            .then(|| keys.clone())
    }

    /// Earn the key set for `model` from scratch, under `pins`.
    async fn earn(&self, model: &str, now: u64, pins: &CurrentPins) -> Attestation {
        let inner = &self.inner;
        let Some(nonce) = (inner.nonce)() else {
            return Attestation::Unavailable;
        };
        let Ok(report) = inner.source.report(model, &nonce).await else {
            return Attestation::Unavailable;
        };
        // The JSON-level binding first: it is what enforces ed25519 and reads
        // only the model container, never the gateway's.
        let claimed =
            match trace_commons_attestation::receipt::model_ed25519_keys(&report, &nonce, model) {
                Ok(keys) => keys,
                Err(AttestedKeyError::ModelNotAttested) => return Attestation::NotAttested,
                Err(_) => return Attestation::NotAttested,
            };
        let Ok(quotes) = model_entry_quotes(&report, model) else {
            return Attestation::NotAttested;
        };
        let mut verified = Vec::with_capacity(quotes.len());
        for quote in &quotes {
            let Ok(collateral) = inner.source.collateral(quote).await else {
                return Attestation::Unavailable;
            };
            let Some(checked) = (inner.verify)(quote, &collateral, now) else {
                return Attestation::NotAttested;
            };
            // A genuine quote from a platform behind on Intel's fixes earns
            // nothing, exactly as the server's drill refuses it.
            if checked.tcb_status != REQUIRED_TCB_STATUS {
                return Attestation::NotAttested;
            }
            if !image_is_pinned(&pins.sets, &checked) {
                return Attestation::NotAttested;
            }
            verified.push(checked);
        }
        let report_data: Vec<&[u8]> = verified.iter().map(|q| q.report_data.as_slice()).collect();
        match quote_bound_keys(&claimed, &report_data, &nonce) {
            Ok(keys) => {
                if let Ok(mut cache) = inner.cache.lock() {
                    // Replace, never merge: a rotation must drop the old key.
                    cache.insert(model.to_string(), (keys.clone(), now, pins.digest.clone()));
                }
                Attestation::Keys(keys)
            }
            Err(_) => Attestation::NotAttested,
        }
    }
}

#[async_trait::async_trait]
impl SignerAttestor for NearAiQuoteAttestor {
    async fn model_keys(&self, backend: &str, model: &str) -> Attestation {
        if backend != NEAR_AI_BACKEND {
            return Attestation::Unsupported;
        }
        let now = (self.inner.now_unix)();
        let pins = self.inner.pins.current(now).await;
        if pins.sets.is_empty() {
            // The pins lapsed or were withdrawn. `Unavailable`, not a
            // verdict: IronWire keeps the row `pending` and retries within
            // its budget, and it can never become `verified` from here.
            return Attestation::Unavailable;
        }
        if let Some(keys) = self.cached(model, now, &pins.digest) {
            return Attestation::Keys(keys);
        }
        let _one_at_a_time = self.inner.refresh.lock().await;
        // Somebody else may have earned it while we waited.
        if let Some(keys) = self.cached(model, now, &pins.digest) {
            return Attestation::Keys(keys);
        }
        self.earn(model, now, &pins).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use trace_commons_attestation::quote::parse_collateral;

    const COLLATERAL: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
    );
    const ECDSA_REPORT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );
    /// When the fixtures were captured. `verify_quote` consults no clock of
    /// its own, so this is what keeps a checked-in collateral from turning
    /// into a test that fails on a date.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;
    const MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";

    fn json(report: &str) -> serde_json::Value {
        serde_json::from_str(report).expect("fixture parses")
    }

    /// A real, Intel-verifiable quote (the ECDSA capture's) framed as an
    /// ed25519 model attestation. The key and nonce are read out of that
    /// quote's own `report_data`, so the report is consistent by
    /// construction rather than by a hand-written expectation -- the same
    /// construction the server's signer-resolver tests use.
    fn synthetic_report() -> (String, String, String) {
        let quote_hex = json(ECDSA_REPORT)["intel_quote"]
            .as_str()
            .unwrap()
            .to_string();
        let quote = hex::decode(&quote_hex).unwrap();
        let report_data = &quote[568..632];
        let key = hex::encode(&report_data[0..32]);
        let nonce = hex::encode(&report_data[32..64]);
        let entry = serde_json::json!({
            "model_name": MODEL,
            "signing_algo": "ed25519",
            "signing_address": key,
            "request_nonce": nonce,
            "intel_quote": quote_hex,
        });
        let gateway = serde_json::json!({
            "signing_algo": "ed25519",
            "signing_address": "cd".repeat(32),
            "request_nonce": nonce,
            "report_data": format!("{}{}", "cd".repeat(32), nonce),
        });
        let document = serde_json::json!({
            "gateway_attestation": gateway,
            "model_attestations": [entry],
        });
        (document.to_string(), nonce, key)
    }

    /// Pins taken from the quote's own verified registers, by running the
    /// real verification path rather than pasting hex.
    fn real_pins() -> Vec<ExpectedMeasurements> {
        let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
        let verified = verify_quote(
            &quote,
            &parse_collateral(COLLATERAL).unwrap(),
            FIXTURE_CAPTURED_AT,
        )
        .expect("the capture verifies against its collateral");
        let raw = format!(
            "mrtd={},rtmr0={},rtmr1={},rtmr2={},rtmr3={}",
            verified.mrtd, verified.rtmr[0], verified.rtmr[1], verified.rtmr[2], verified.rtmr[3]
        );
        parse_pins(&raw).expect("valid pins")
    }

    fn other_pins() -> Vec<ExpectedMeasurements> {
        parse_pins(&format!("mrtd={}", "00".repeat(48))).expect("valid pins")
    }

    struct Fixture {
        report: String,
        reports: Arc<AtomicUsize>,
        collateral_ok: bool,
    }

    #[async_trait::async_trait]
    impl AttestationSource for Fixture {
        async fn report(&self, _model: &str, _nonce: &str) -> Result<String, SourceUnavailable> {
            self.reports.fetch_add(1, Ordering::SeqCst);
            Ok(self.report.clone())
        }
        async fn collateral(&self, _quote: &[u8]) -> Result<Collateral, SourceUnavailable> {
            if self.collateral_ok {
                parse_collateral(COLLATERAL).map_err(|_| SourceUnavailable)
            } else {
                Err(SourceUnavailable)
            }
        }
    }

    /// An attestor over the fixture with real DCAP verification, the
    /// fixture's nonce, and a clock the test moves.
    fn attestor(
        report: String,
        nonce: String,
        pins: Vec<ExpectedMeasurements>,
        clock: Arc<std::sync::atomic::AtomicU64>,
    ) -> (NearAiQuoteAttestor, Arc<AtomicUsize>) {
        let reports = Arc::new(AtomicUsize::new(0));
        let source = Fixture {
            report,
            reports: Arc::clone(&reports),
            collateral_ok: true,
        };
        let attestor = NearAiQuoteAttestor::with_parts(
            Box::new(source),
            Arc::new(PinProvider::new(Some(pins), None)),
            Box::new(move || clock.load(Ordering::SeqCst)),
            Box::new(move || Some(nonce.clone())),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        );
        (attestor, reports)
    }

    fn clock_at(t: u64) -> Arc<std::sync::atomic::AtomicU64> {
        Arc::new(std::sync::atomic::AtomicU64::new(t))
    }

    #[tokio::test]
    async fn a_dcap_verified_pinned_quote_yields_its_bound_key() {
        let (report, nonce, key) = synthetic_report();
        let (attestor, _) = attestor(report, nonce, real_pins(), clock_at(FIXTURE_CAPTURED_AT));
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Keys(vec![key])
        );
    }

    /// The gateway key sits in the same report, bound to the same nonce. It
    /// must never come back.
    #[tokio::test]
    async fn the_gateway_key_is_never_returned() {
        let (report, nonce, _) = synthetic_report();
        let (attestor, _) = attestor(report, nonce, real_pins(), clock_at(FIXTURE_CAPTURED_AT));
        let Attestation::Keys(keys) = attestor.model_keys(NEAR_AI_BACKEND, MODEL).await else {
            panic!("expected keys");
        };
        assert!(!keys.contains(&"cd".repeat(32)));
    }

    #[tokio::test]
    async fn without_pins_nothing_is_attested() {
        let (report, nonce, _) = synthetic_report();
        let (attestor, reports) =
            attestor(report, nonce, Vec::new(), clock_at(FIXTURE_CAPTURED_AT));
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Unavailable,
            "no pins is never a verdict, and never keys"
        );
        assert_eq!(reports.load(Ordering::SeqCst), 0, "nothing fetched");
    }

    #[tokio::test]
    async fn a_quote_outside_the_pins_is_not_attested() {
        let (report, nonce, _) = synthetic_report();
        let (attestor, _) = attestor(report, nonce, other_pins(), clock_at(FIXTURE_CAPTURED_AT));
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::NotAttested
        );
    }

    /// A report bound to somebody else's nonce -- a replay -- earns nothing,
    /// even though its quote is genuine and pinned.
    #[tokio::test]
    async fn a_report_for_another_nonce_is_not_attested() {
        let (report, _, _) = synthetic_report();
        let (attestor, _) = attestor(
            report,
            "ab".repeat(32),
            real_pins(),
            clock_at(FIXTURE_CAPTURED_AT),
        );
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::NotAttested
        );
    }

    /// Collateral windows are real: a year later the same quote and
    /// collateral no longer verify, and nothing is attested.
    #[tokio::test]
    async fn stale_collateral_does_not_verify() {
        let (report, nonce, _) = synthetic_report();
        let (attestor, _) = attestor(
            report,
            nonce,
            real_pins(),
            clock_at(FIXTURE_CAPTURED_AT + 365 * 24 * 60 * 60),
        );
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::NotAttested
        );
    }

    /// Intel's TCB verdict must be `UpToDate`, as the server's drill requires.
    /// The fixture is `UpToDate`, so every other verdict is injected on top
    /// of a real verification: the quote, pins, nonce and binding all still
    /// pass, and only the platform's patch level is wrong.
    #[tokio::test]
    async fn a_platform_that_is_not_up_to_date_earns_no_key() {
        for status in [
            "OutOfDate",
            "OutOfDateConfigurationNeeded",
            "ConfigurationNeeded",
            "SWHardeningNeeded",
            "ConfigurationAndSWHardeningNeeded",
            "Revoked",
            "",
            "uptodate",
        ] {
            let (report, nonce, _) = synthetic_report();
            let reports = Arc::new(AtomicUsize::new(0));
            let attestor = NearAiQuoteAttestor::with_parts(
                Box::new(Fixture {
                    report,
                    reports,
                    collateral_ok: true,
                }),
                Arc::new(PinProvider::new(Some(real_pins()), None)),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                Box::new(move |quote, collateral, now| {
                    let mut verified = verify_quote(quote, collateral, now).ok()?;
                    verified.tcb_status = status.to_string();
                    Some(verified)
                }),
            );
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested,
                "tcb_status {status:?}"
            );
        }
    }

    #[tokio::test]
    async fn another_model_or_backend_is_not_attested() {
        let (report, nonce, _) = synthetic_report();
        let (attestor, _) = attestor(report, nonce, real_pins(), clock_at(FIXTURE_CAPTURED_AT));
        assert_eq!(
            attestor
                .model_keys(NEAR_AI_BACKEND, "Qwen/Qwen3.8-27B")
                .await,
            Attestation::NotAttested
        );
        assert_eq!(
            attestor.model_keys("vendor", MODEL).await,
            Attestation::Unsupported
        );
    }

    #[tokio::test]
    async fn no_collateral_is_unavailable_not_a_verdict() {
        let (report, nonce, _) = synthetic_report();
        let source = Fixture {
            report,
            reports: Arc::new(AtomicUsize::new(0)),
            collateral_ok: false,
        };
        let attestor = NearAiQuoteAttestor::with_parts(
            Box::new(source),
            Arc::new(PinProvider::new(Some(real_pins()), None)),
            Box::new(|| FIXTURE_CAPTURED_AT),
            Box::new(move || Some(nonce.clone())),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        );
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Unavailable
        );
    }

    /// An earned set is served for the TTL, then must be earned again.
    #[tokio::test]
    async fn the_key_set_is_cached_for_the_ttl_only() {
        let (report, nonce, key) = synthetic_report();
        let clock = clock_at(FIXTURE_CAPTURED_AT);
        let (attestor, reports) = attestor(report, nonce, real_pins(), Arc::clone(&clock));
        for _ in 0..3 {
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Keys(vec![key.clone()])
            );
        }
        assert_eq!(reports.load(Ordering::SeqCst), 1, "one fetch for a burst");
        clock.fetch_add(FRESH_TTL.as_secs(), Ordering::SeqCst);
        let _ = attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
        assert_eq!(reports.load(Ordering::SeqCst), 2, "refetched past the TTL");
    }

    #[test]
    fn pins_parse_in_the_servers_syntax_and_refuse_a_partial_list() {
        let good = format!("mrtd={0},rtmr0={0};mrconfigid={0}", "aa".repeat(48));
        assert_eq!(parse_pins(&good).map(|p| p.len()), Some(2));
        let bad = format!("mrtd={};mrtd:deadbeef", "aa".repeat(48));
        assert!(
            parse_pins(&bad).is_none(),
            "one malformed set voids the list"
        );
        assert_eq!(parse_pins("").map(|p| p.len()), Some(0));
    }

    // ---- where the pins come from ----

    use trace_commons_protocol::near_ai_measurements::{NearAiMeasurementPins, PinState};

    fn pin_string(byte: &str) -> String {
        format!("mrconfigid={}", byte.repeat(48))
    }

    /// A published-pins source whose answer the test changes, counting calls.
    struct Published {
        answer: Mutex<Result<NearAiMeasurementPins, SourceUnavailable>>,
        calls: AtomicUsize,
    }

    impl Published {
        fn serving(document: NearAiMeasurementPins) -> Arc<Self> {
            Arc::new(Self {
                answer: Mutex::new(Ok(document)),
                calls: AtomicUsize::new(0),
            })
        }
        fn set(&self, answer: Result<NearAiMeasurementPins, SourceUnavailable>) {
            *self.answer.lock().unwrap() = answer;
        }
        fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait::async_trait]
    impl PinSource for Arc<Published> {
        async fn fetch(&self) -> Result<NearAiMeasurementPins, SourceUnavailable> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            self.answer.lock().unwrap().clone()
        }
    }

    fn configured(bytes: &[&str]) -> NearAiMeasurementPins {
        NearAiMeasurementPins::new(
            PinState::Configured,
            bytes.iter().map(|b| pin_string(b)).collect(),
        )
    }

    const T0: u64 = 1_800_000_000;

    #[tokio::test]
    async fn published_pins_are_fetched_and_used() {
        let source = Published::serving(configured(&["aa"]));
        let provider = PinProvider::new(None, Some(Box::new(Arc::clone(&source))));
        let current = provider.current(T0).await;
        assert_eq!(current.sets.len(), 1);
        assert_eq!(current.sets[0].to_pin_string(), pin_string("aa"));
        // Within the refresh interval, no second fetch.
        provider.current(T0 + 1).await;
        assert_eq!(source.calls(), 1);
    }

    /// A failed refresh keeps the last good set, but only until its TTL runs
    /// out; after that the device pins nothing, so nothing can verify.
    #[tokio::test]
    async fn a_failing_fetch_keeps_the_last_good_set_only_until_its_ttl() {
        let source = Published::serving(configured(&["aa"]));
        let provider = PinProvider::new(None, Some(Box::new(Arc::clone(&source))));
        assert_eq!(provider.current(T0).await.sets.len(), 1);

        source.set(Err(SourceUnavailable));
        let refresh = PIN_REFRESH_AFTER.as_secs();
        let within = provider.current(T0 + refresh + 1).await;
        assert_eq!(within.sets.len(), 1, "still inside the TTL");
        assert!(source.calls() >= 2, "a refresh was attempted");

        let past = provider.current(T0 + PIN_TTL.as_secs() + 1).await;
        assert!(past.sets.is_empty(), "past the TTL the pins are gone");
        // And they stay gone while the fetch keeps failing.
        let later = provider
            .current(T0 + PIN_TTL.as_secs() + PIN_RETRY_FLOOR.as_secs() + 2)
            .await;
        assert!(later.sets.is_empty());
    }

    /// Ingest publishing "nothing configured" means the device pins nothing
    /// -- and an attestor over it attests nothing.
    #[tokio::test]
    async fn a_published_empty_set_means_no_verification() {
        let source = Published::serving(NearAiMeasurementPins::new(
            PinState::Unconfigured,
            Vec::new(),
        ));
        let provider = Arc::new(PinProvider::new(None, Some(Box::new(Arc::clone(&source)))));
        assert!(provider.current(T0).await.sets.is_empty());

        let (report, nonce, _) = synthetic_report();
        let attestor = NearAiQuoteAttestor::with_parts(
            Box::new(Fixture {
                report,
                reports: Arc::new(AtomicUsize::new(0)),
                collateral_ok: true,
            }),
            Arc::clone(&provider),
            Box::new(|| FIXTURE_CAPTURED_AT),
            Box::new(move || Some(nonce.clone())),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        );
        assert!(!matches!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Keys(_)
        ));
    }

    /// A document whose digest does not match its sets is not trusted.
    #[tokio::test]
    async fn a_published_document_with_a_wrong_digest_pins_nothing() {
        let mut document = configured(&["aa"]);
        document.digest = "00".repeat(32);
        let source = Published::serving(document);
        let provider = PinProvider::new(None, Some(Box::new(Arc::clone(&source))));
        assert!(provider.current(T0).await.sets.is_empty());
    }

    /// The operator override wins over what ingest publishes, and when it is
    /// set ingest is not asked at all.
    #[tokio::test]
    async fn the_environment_override_wins() {
        let source = Published::serving(configured(&["aa"]));
        let overridden = parse_pins(&pin_string("bb")).unwrap();
        let provider = PinProvider::new(Some(overridden), Some(Box::new(Arc::clone(&source))));
        let current = provider.current(T0).await;
        assert_eq!(current.sets.len(), 1);
        assert_eq!(current.sets[0].to_pin_string(), pin_string("bb"));
        assert_eq!(source.calls(), 0);
        // An override that pins nothing (set but empty or malformed) still
        // wins, and means nothing verifies -- it does not fall back.
        let empty = PinProvider::new(Some(Vec::new()), Some(Box::new(Arc::clone(&source))));
        assert!(empty.current(T0).await.sets.is_empty());
        assert_eq!(source.calls(), 0);
    }

    /// The image check itself: an empty pin list admits no image. The
    /// mutation "an empty set allows everything" must turn this red.
    #[test]
    fn an_empty_pin_list_pins_no_image() {
        let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
        let verified = verify_quote(
            &quote,
            &parse_collateral(COLLATERAL).unwrap(),
            FIXTURE_CAPTURED_AT,
        )
        .expect("the capture verifies");
        assert!(!image_is_pinned(&[], &verified));
        assert!(image_is_pinned(&real_pins(), &verified));
        assert!(!image_is_pinned(&other_pins(), &verified));
    }
}
