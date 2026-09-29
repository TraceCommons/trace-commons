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
//! a failed refresh does not extend an old one. A "not attested" verdict is
//! cached for the much shorter [`NOT_ATTESTED_TTL`]. Either is cached against
//! the digest of the pins it was reached under, so a pin change drops it at
//! once. Refreshes are single-flight per model, and one model's slow fetch
//! does not hold up another's.
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
//! The document's digest is a change detector, not an integrity check. It
//! tells a device that the set changed (and drops verdicts cached under the
//! old one) and catches a document that is internally inconsistent. It is not
//! signed, so it adds nothing to what the TLS connection to ingest already
//! establishes about who sent the document.
//!
//! **`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` overrides it**, for an
//! operator or a developer: when the variable is set at daemon start it wins
//! outright and ingest is not asked. Set but empty or malformed, it pins
//! nothing -- it never falls back to the published set, because an operator
//! who set it meant it. Each set is written in the server's own syntax
//! (`mrconfigid=<96 hex>,...`), but the device variable can hold several
//! sets split by `;`, and the server's cannot: ingest reads exactly one set
//! from its variable and publishes it as a one-element list. The `;` form is
//! a device-side extension, for trying an image ingest does not pin yet.
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

/// The device's measurement-pin override. The server's variable name, and
/// each set in the server's syntax; unlike the server, it accepts several
/// sets split by `;` (see [`parse_pins`]).
pub const NEAR_AI_EXPECTED_MEASUREMENTS_ENV: &str = "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS";

/// The control name a missing pin is refused under.
const CONTROL: &str = "near_ai_expected_measurements";

/// How long an earned key set is served before it must be earned again.
pub const FRESH_TTL: Duration = Duration::from_secs(60 * 60);

/// How long a "not attested" verdict for a model is remembered.
///
/// Short on purpose. Two things can turn the verdict around: the pins
/// catching up with a new image, which changes the pins digest and so drops
/// the cached verdict at once whatever this says; and the report itself
/// changing under the same pins, which only this TTL covers. It exists so
/// that a burst of answers for a model that does not attest -- an image
/// rollout ingest's pins have not caught up with, say -- costs one report
/// and collateral fetch per model per interval rather than one per answer.
pub const NOT_ATTESTED_TTL: Duration = Duration::from_secs(60);

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
    /// Changes whenever `sets` does. A change detector for the cache, not an
    /// integrity check: nothing signs it.
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
/// syntax -- the server's syntax for one set. The `;` separator is this
/// device override's own: the server's variable holds a single set. `None` when any set is malformed: a partly parsed pin list is a
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
    /// model -> its last verdict, when, and under which pins.
    cache: Mutex<HashMap<String, Remembered>>,
    /// model -> its refresh lock. One refresh per model at a time, so a burst
    /// of rows for one model costs one report fetch rather than one each; per
    /// model, so a slow fetch for one does not hold up another.
    refresh: Mutex<HashMap<String, Arc<tokio::sync::Mutex<()>>>>,
}

/// A verdict worth remembering. `Unavailable` never is: it says nothing about
/// the model, only that we could not ask.
#[derive(Clone)]
enum Verdict {
    Keys(Vec<String>),
    NotAttested,
}

struct Remembered {
    verdict: Verdict,
    at: u64,
    pins_digest: String,
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
                refresh: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// A verdict reached under exactly these pins, still inside its TTL
    /// ([`FRESH_TTL`] for keys, [`NOT_ATTESTED_TTL`] for a refusal). One
    /// reached under pins that have since changed is not served.
    fn cached(&self, model: &str, now: u64, pins_digest: &str) -> Option<Attestation> {
        let cache = self.inner.cache.lock().ok()?;
        let remembered = cache.get(model)?;
        if remembered.pins_digest != pins_digest {
            return None;
        }
        let ttl = match remembered.verdict {
            Verdict::Keys(_) => FRESH_TTL,
            Verdict::NotAttested => NOT_ATTESTED_TTL,
        };
        if now.saturating_sub(remembered.at) >= ttl.as_secs() {
            return None;
        }
        Some(match &remembered.verdict {
            Verdict::Keys(keys) => Attestation::Keys(keys.clone()),
            Verdict::NotAttested => Attestation::NotAttested,
        })
    }

    /// Remember a verdict for `model`, replacing whatever was there -- never
    /// merging, so a rotation drops the old key. `Unavailable` is not
    /// remembered, and leaves an older entry to lapse on its own TTL.
    fn remember(&self, model: &str, now: u64, pins_digest: &str, answer: &Attestation) {
        let verdict = match answer {
            Attestation::Keys(keys) => Verdict::Keys(keys.clone()),
            Attestation::NotAttested => Verdict::NotAttested,
            _ => return,
        };
        if let Ok(mut cache) = self.inner.cache.lock() {
            cache.insert(
                model.to_string(),
                Remembered {
                    verdict,
                    at: now,
                    pins_digest: pins_digest.to_string(),
                },
            );
        }
    }

    /// The refresh lock for one model, created on first use.
    fn refresh_lock(&self, model: &str) -> Arc<tokio::sync::Mutex<()>> {
        let mut locks = self
            .inner
            .refresh
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        Arc::clone(locks.entry(model.to_string()).or_default())
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
            Ok(keys) => Attestation::Keys(keys),
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
        if let Some(answer) = self.cached(model, now, &pins.digest) {
            return answer;
        }
        let lock = self.refresh_lock(model);
        let _one_at_a_time = lock.lock().await;
        // Somebody else may have answered it while we waited -- keys or a
        // refusal alike, or a burst for an unattested model would still
        // fetch once per caller, one after another.
        if let Some(answer) = self.cached(model, now, &pins.digest) {
            return answer;
        }
        let answer = self.earn(model, now, &pins).await;
        self.remember(model, now, &pins.digest, &answer);
        answer
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

    /// A gateway key that a genuine, pinned quote really does bind, under our
    /// nonce, is still never returned for a model. The report's only
    /// quote-bound key sits in `gateway_attestation`; the model container has
    /// an entry for another model only, or is absent altogether.
    #[tokio::test]
    async fn the_gateway_key_is_never_returned() {
        let (_, nonce, key) = synthetic_report();
        let quote_hex = json(ECDSA_REPORT)["intel_quote"]
            .as_str()
            .unwrap()
            .to_string();
        let gateway = serde_json::json!({
            "signing_algo": "ed25519",
            "signing_address": key,
            "request_nonce": nonce,
            "intel_quote": quote_hex,
            "report_data": format!("{key}{nonce}"),
        });
        let other_model = serde_json::json!({
            "model_name": "Qwen/Qwen3.8-27B",
            "signing_algo": "ed25519",
            "signing_address": key,
            "request_nonce": nonce,
            "intel_quote": quote_hex,
        });
        let with_other_model = serde_json::json!({
            "gateway_attestation": gateway,
            "model_attestations": [other_model],
        })
        .to_string();
        let gateway_only = serde_json::json!({ "gateway_attestation": gateway }).to_string();

        // The premise, so the refusal below cannot be for some other reason:
        // the gateway attestation binds the key under our nonce, and its
        // quote verifies and is pinned.
        assert_eq!(
            trace_commons_attestation::receipt::gateway_ed25519_key(&with_other_model, &nonce),
            Ok(key.clone())
        );
        let verified = verify_quote(
            &hex::decode(&quote_hex).unwrap(),
            &parse_collateral(COLLATERAL).unwrap(),
            FIXTURE_CAPTURED_AT,
        )
        .expect("the gateway's quote verifies");
        assert!(image_is_pinned(&real_pins(), &verified));
        assert_eq!(verified.tcb_status, REQUIRED_TCB_STATUS);

        for report in [with_other_model, gateway_only] {
            let (attestor, _) = attestor(
                report,
                nonce.clone(),
                real_pins(),
                clock_at(FIXTURE_CAPTURED_AT),
            );
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested
            );
        }
    }

    /// A report with two entries for the model is attested only when both
    /// hold up. One genuine, pinned quote beside one that fails DCAP, or
    /// beside one that verifies but runs an image outside the pins, earns
    /// nothing -- not the key of the good one.
    #[tokio::test]
    async fn a_report_mixing_a_good_quote_with_a_bad_one_is_not_attested() {
        let (_, nonce, key) = synthetic_report();
        let quote_hex = json(ECDSA_REPORT)["intel_quote"]
            .as_str()
            .unwrap()
            .to_string();
        let mut tampered = hex::decode(&quote_hex).unwrap();
        // Inside the quote's signature data, past report_data: the JSON
        // binding still reads the same key and nonce, and only DCAP can tell.
        tampered[700] ^= 0x01;
        let entry = |quote: &str| {
            serde_json::json!({
                "model_name": MODEL,
                "signing_algo": "ed25519",
                "signing_address": key,
                "request_nonce": nonce,
                "intel_quote": quote,
            })
        };
        let report = |second: &str| {
            serde_json::json!({ "model_attestations": [entry(&quote_hex), entry(second)] })
                .to_string()
        };
        let build = |report: String, second_off_pin: bool| {
            let calls = Arc::new(AtomicUsize::new(0));
            let nonce = nonce.clone();
            NearAiQuoteAttestor::with_parts(
                Box::new(Fixture {
                    report,
                    reports: Arc::new(AtomicUsize::new(0)),
                    collateral_ok: true,
                }),
                Arc::new(PinProvider::new(Some(real_pins()), None)),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                Box::new(move |quote, collateral, now| {
                    let mut verified = verify_quote(quote, collateral, now).ok()?;
                    if second_off_pin && calls.fetch_add(1, Ordering::SeqCst) == 1 {
                        verified.mrtd = "00".repeat(48);
                    }
                    Some(verified)
                }),
            )
        };

        // Control: two good entries earn the key, so the report's shape is
        // not what the refusals below turn on.
        match build(report(&quote_hex), false)
            .model_keys(NEAR_AI_BACKEND, MODEL)
            .await
        {
            Attestation::Keys(keys) => {
                assert!(!keys.is_empty() && keys.iter().all(|k| k == &key));
            }
            other => panic!("two good entries: {other:?}"),
        }
        assert_eq!(
            build(report(&hex::encode(&tampered)), false)
                .model_keys(NEAR_AI_BACKEND, MODEL)
                .await,
            Attestation::NotAttested,
            "good + fails DCAP"
        );
        assert_eq!(
            build(report(&quote_hex), true)
                .model_keys(NEAR_AI_BACKEND, MODEL)
                .await,
            Attestation::NotAttested,
            "good + verifies but off the pins"
        );
    }

    /// A stand-in for ingest's collateral route that records every request
    /// it receives, headers and body, and answers with the fixture or with
    /// an oversized body.
    struct CollateralServer {
        base: String,
        seen: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
        _shutdown: tokio::sync::oneshot::Sender<()>,
    }

    async fn collateral_server(oversized: bool) -> CollateralServer {
        use axum::routing::post;
        let seen: Arc<Mutex<Vec<(String, Vec<u8>)>>> = Arc::default();
        let app = axum::Router::new().route(
            crate::witness::transport::COLLATERAL_PATH,
            post({
                let seen = Arc::clone(&seen);
                move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
                    let seen = Arc::clone(&seen);
                    async move {
                        let rendered = headers
                            .iter()
                            .map(|(name, value)| {
                                format!("{}: {}", name, String::from_utf8_lossy(value.as_bytes()))
                            })
                            .collect::<Vec<_>>()
                            .join("\n");
                        seen.lock().unwrap().push((rendered, body.to_vec()));
                        if oversized {
                            // Valid collateral, padded past the bound with
                            // whitespace JSON allows: only the bound refuses it.
                            format!(
                                "{COLLATERAL}{}",
                                " ".repeat(crate::witness::transport::MAX_COLLATERAL_BYTES)
                            )
                        } else {
                            COLLATERAL.to_string()
                        }
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        CollateralServer {
            base,
            seen,
            _shutdown: tx,
        }
    }

    /// The contributor's NEAR AI key goes to the gateway only. The collateral
    /// request to ingest carries it in no header and nowhere in its body.
    #[tokio::test]
    async fn the_near_ai_key_is_never_sent_with_the_collateral_request() {
        const KEY: &str = "sk-near-ai-key-that-must-stay-on-the-gateway-path";
        let server = collateral_server(false).await;
        let source =
            HttpAttestationSource::new(&HostAllowlist::permissive(), &server.base, KEY.into())
                .expect("the source builds");
        let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
        assert!(source.collateral(&quote).await.is_ok(), "collateral served");

        // Positive control: the recorder does capture an Authorization
        // header when one is sent, so its absence below is a real absence.
        reqwest::Client::new()
            .post(format!(
                "{}{}",
                server.base,
                crate::witness::transport::COLLATERAL_PATH
            ))
            .bearer_auth("control")
            .body("{}")
            .send()
            .await
            .expect("the control request is served");

        let seen = server.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        let (headers, body) = &seen[0];
        assert!(
            headers.contains("application/json"),
            "the attestor's request: {headers}"
        );
        assert!(!headers.to_ascii_lowercase().contains("authorization"));
        assert!(!headers.contains(KEY));
        assert!(!String::from_utf8_lossy(body).contains(KEY));
        assert!(seen[1].0.contains("authorization: Bearer control"));
    }

    /// An ingest that answers with a body past the collateral bound is
    /// refused, not buffered.
    #[tokio::test]
    async fn an_oversized_collateral_body_is_refused() {
        let server = collateral_server(true).await;
        let source =
            HttpAttestationSource::new(&HostAllowlist::permissive(), &server.base, "sk".into())
                .expect("the source builds");
        let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
        assert_eq!(
            source.collateral(&quote).await.err(),
            Some(SourceUnavailable)
        );
        assert_eq!(server.seen.lock().unwrap().len(), 1, "the request was made");
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

    /// A "not attested" verdict is remembered too, briefly: during an image
    /// rollout that ingest's pins have not caught up with, a burst of answers
    /// for the model costs one report fetch, not one each. Past its TTL it is
    /// asked again.
    #[tokio::test]
    async fn a_not_attested_verdict_is_cached_for_its_short_ttl() {
        let (report, nonce, _) = synthetic_report();
        let clock = clock_at(FIXTURE_CAPTURED_AT);
        let (attestor, reports) = attestor(report, nonce, other_pins(), Arc::clone(&clock));
        for _ in 0..3 {
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested
            );
        }
        assert_eq!(reports.load(Ordering::SeqCst), 1, "one fetch for a burst");
        clock.fetch_add(NOT_ATTESTED_TTL.as_secs(), Ordering::SeqCst);
        let _ = attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
        assert_eq!(reports.load(Ordering::SeqCst), 2, "asked again past it");
        assert!(NOT_ATTESTED_TTL < PIN_REFRESH_AFTER && NOT_ATTESTED_TTL < FRESH_TTL);
    }

    /// A report source that holds the report for one model until released,
    /// and says when a fetch for it has begun.
    struct Gated {
        report: String,
        slow_model: &'static str,
        entered: Arc<tokio::sync::Notify>,
        release: Arc<tokio::sync::Semaphore>,
        reports: Arc<AtomicUsize>,
    }

    #[async_trait::async_trait]
    impl AttestationSource for Gated {
        async fn report(&self, model: &str, _nonce: &str) -> Result<String, SourceUnavailable> {
            self.reports.fetch_add(1, Ordering::SeqCst);
            if model == self.slow_model {
                self.entered.notify_one();
                let _permit = self
                    .release
                    .acquire()
                    .await
                    .map_err(|_| SourceUnavailable)?;
            }
            Ok(self.report.clone())
        }
        async fn collateral(&self, _quote: &[u8]) -> Result<Collateral, SourceUnavailable> {
            parse_collateral(COLLATERAL).map_err(|_| SourceUnavailable)
        }
    }

    fn gated_attestor(
        pins: Vec<ExpectedMeasurements>,
    ) -> (
        NearAiQuoteAttestor,
        Arc<tokio::sync::Notify>,
        Arc<tokio::sync::Semaphore>,
        Arc<AtomicUsize>,
        String,
    ) {
        let (report, nonce, key) = synthetic_report();
        let entered = Arc::new(tokio::sync::Notify::new());
        let release = Arc::new(tokio::sync::Semaphore::new(0));
        let reports = Arc::new(AtomicUsize::new(0));
        let attestor = NearAiQuoteAttestor::with_parts(
            Box::new(Gated {
                report,
                slow_model: MODEL,
                entered: Arc::clone(&entered),
                release: Arc::clone(&release),
                reports: Arc::clone(&reports),
            }),
            Arc::new(PinProvider::new(Some(pins), None)),
            Box::new(|| FIXTURE_CAPTURED_AT),
            Box::new(move || Some(nonce.clone())),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        );
        (attestor, entered, release, reports, key)
    }

    /// One model whose report is slow to come back does not hold up the
    /// answer for another: the refresh lock is per model.
    #[tokio::test]
    async fn one_slow_model_does_not_stall_another() {
        let (attestor, entered, release, _, key) = gated_attestor(real_pins());
        let slow = tokio::spawn({
            let attestor = attestor.clone();
            async move { attestor.model_keys(NEAR_AI_BACKEND, MODEL).await }
        });
        entered.notified().await;
        let other = tokio::time::timeout(
            Duration::from_secs(5),
            attestor.model_keys(NEAR_AI_BACKEND, "Qwen/Qwen3.8-27B"),
        )
        .await
        .expect("the other model is answered while the slow one is still fetching");
        assert_eq!(other, Attestation::NotAttested);
        release.add_permits(64);
        assert_eq!(slow.await.unwrap(), Attestation::Keys(vec![key]));
    }

    /// Concurrent questions about one model share one refresh: one report
    /// fetch, and every caller gets the answer it earned. Holds for a key set
    /// and for a "not attested" verdict alike.
    #[tokio::test]
    async fn refreshes_are_single_flight_per_model() {
        for (pins, attested) in [(real_pins(), true), (other_pins(), false)] {
            let (attestor, entered, release, reports, key) = gated_attestor(pins);
            let callers: Vec<_> = (0..8)
                .map(|_| {
                    let attestor = attestor.clone();
                    tokio::spawn(async move { attestor.model_keys(NEAR_AI_BACKEND, MODEL).await })
                })
                .collect();
            entered.notified().await;
            // Let every other caller reach the lock before the fetch returns.
            for _ in 0..32 {
                tokio::task::yield_now().await;
            }
            release.add_permits(64);
            let expected = if attested {
                Attestation::Keys(vec![key.clone()])
            } else {
                Attestation::NotAttested
            };
            for caller in callers {
                assert_eq!(caller.await.unwrap(), expected);
            }
            assert_eq!(reports.load(Ordering::SeqCst), 1, "attested={attested}");
        }
    }

    /// A cached answer belongs to the pins it was earned under. When the
    /// published pins change, the next question is answered afresh -- in
    /// both directions, and well inside either cache's TTL.
    #[tokio::test]
    async fn a_pin_digest_flip_invalidates_the_cache() {
        let doc = |pins: Vec<ExpectedMeasurements>| {
            NearAiMeasurementPins::new(
                PinState::Configured,
                pins.iter()
                    .map(ExpectedMeasurements::to_pin_string)
                    .collect(),
            )
        };
        for (before, after, first, then) in [
            (other_pins(), real_pins(), false, true),
            (real_pins(), other_pins(), true, false),
        ] {
            let (report, nonce, key) = synthetic_report();
            let source = Published::serving(doc(before));
            let provider = Arc::new(PinProvider::new(None, Some(Box::new(Arc::clone(&source)))));
            // Fetched just under one refresh interval ago, so a two-second
            // step in the attestor's clock is enough to make it fetch again.
            let t = FIXTURE_CAPTURED_AT;
            provider.current(t - PIN_REFRESH_AFTER.as_secs() + 1).await;
            let clock = clock_at(t);
            let reports = Arc::new(AtomicUsize::new(0));
            let attestor = NearAiQuoteAttestor::with_parts(
                Box::new(Fixture {
                    report,
                    reports: Arc::clone(&reports),
                    collateral_ok: true,
                }),
                provider,
                Box::new({
                    let clock = Arc::clone(&clock);
                    move || clock.load(Ordering::SeqCst)
                }),
                Box::new(move || Some(nonce.clone())),
                Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
            );
            let expect = |attested: bool| {
                if attested {
                    Attestation::Keys(vec![key.clone()])
                } else {
                    Attestation::NotAttested
                }
            };
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                expect(first)
            );
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                expect(first)
            );
            assert_eq!(
                reports.load(Ordering::SeqCst),
                1,
                "cached under the first pins"
            );

            source.set(Ok(doc(after)));
            clock.fetch_add(2, Ordering::SeqCst);
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                expect(then),
                "first={first}"
            );
            assert_eq!(
                reports.load(Ordering::SeqCst),
                2,
                "re-earned under the new pins"
            );
        }
    }

    #[test]
    fn pins_parse_as_server_sets_split_by_semicolon_and_refuse_a_partial_list() {
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

    /// Pins that ingest starts publishing after the attestor was built reach
    /// that same instance: it answers `Unavailable` while there are none and
    /// earns keys once they arrive, with no new attestor and so no proxy
    /// restart. This is what lets the daemon install the attestor always.
    #[tokio::test]
    async fn pins_published_after_start_reach_the_same_attestor() {
        let source = Published::serving(NearAiMeasurementPins::new(
            PinState::Unconfigured,
            Vec::new(),
        ));
        let provider = Arc::new(PinProvider::new(None, Some(Box::new(Arc::clone(&source)))));
        let (report, nonce, key) = synthetic_report();
        let clock = clock_at(FIXTURE_CAPTURED_AT);
        let reports = Arc::new(AtomicUsize::new(0));
        let attestor = NearAiQuoteAttestor::with_parts(
            Box::new(Fixture {
                report,
                reports: Arc::clone(&reports),
                collateral_ok: true,
            }),
            provider,
            Box::new({
                let clock = Arc::clone(&clock);
                move || clock.load(Ordering::SeqCst)
            }),
            Box::new(move || Some(nonce.clone())),
            Box::new(|quote, collateral, now| verify_quote(quote, collateral, now).ok()),
        );
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Unavailable
        );
        assert_eq!(
            reports.load(Ordering::SeqCst),
            0,
            "nothing fetched unpinned"
        );

        source.set(Ok(NearAiMeasurementPins::new(
            PinState::Configured,
            real_pins()
                .iter()
                .map(ExpectedMeasurements::to_pin_string)
                .collect(),
        )));
        clock.fetch_add(PIN_REFRESH_AFTER.as_secs(), Ordering::SeqCst);
        assert_eq!(
            attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
            Attestation::Keys(vec![key])
        );
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
