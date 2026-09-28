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
//! 3. runs an image whose measurements match a pinned set
//!    (`TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`, the server's own
//!    variable and syntax: `mrconfigid=<96 hex>,...`, sets split by `;`), and
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
//! Nothing here logs a key, a nonce, a model, a report or a URL.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use ironwire_proxy::proof::{Attestation, SignerAttestor};
use trace_commons_attestation::measurements::{
    ExpectedMeasurements, MeasurementVerdict, check_measurements_opt,
};
use trace_commons_attestation::quote::{Collateral, VerifiedQuote, verify_quote};
use trace_commons_attestation::receipt::{AttestedKeyError, model_entry_quotes, quote_bound_keys};
use trace_commons_operator_client::host_allowlist::HostAllowlist;

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

/// Read the pins from the environment. Absent, empty or malformed all read as
/// no pins -- which answers `Unsupported`, never a pass.
#[must_use]
pub fn pins_from_env() -> Vec<ExpectedMeasurements> {
    let Ok(raw) = std::env::var(NEAR_AI_EXPECTED_MEASUREMENTS_ENV) else {
        return Vec::new();
    };
    parse_pins(&raw).unwrap_or_else(|| {
        tracing::warn!(
            control = CONTROL,
            "NEAR AI measurement pins did not parse; no answer will be proof"
        );
        Vec::new()
    })
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
    pins: Vec<ExpectedMeasurements>,
    now_unix: Clock,
    nonce: Nonces,
    verify: QuoteVerifier,
    /// model -> (keys, earned at).
    cache: Mutex<HashMap<String, (Vec<String>, u64)>>,
    /// One refresh at a time, so a burst of rows for one model costs one
    /// report fetch rather than one each.
    refresh: tokio::sync::Mutex<()>,
}

impl std::fmt::Debug for NearAiQuoteAttestor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearAiQuoteAttestor")
            .field("pinned_sets", &self.inner.pins.len())
            .finish_non_exhaustive()
    }
}

impl NearAiQuoteAttestor {
    /// An attestor over `source`, admitting quotes that match any of `pins`.
    #[must_use]
    pub fn new(source: Box<dyn AttestationSource>, pins: Vec<ExpectedMeasurements>) -> Self {
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
        pins: Vec<ExpectedMeasurements>,
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

    /// Whether any measurement set is pinned. With none, no key can be earned.
    #[must_use]
    pub fn is_pinned(&self) -> bool {
        !self.inner.pins.is_empty()
    }

    fn cached(&self, model: &str, now: u64) -> Option<Vec<String>> {
        let cache = self.inner.cache.lock().ok()?;
        let (keys, at) = cache.get(model)?;
        (now.saturating_sub(*at) < FRESH_TTL.as_secs()).then(|| keys.clone())
    }

    /// Earn the key set for `model` from scratch.
    async fn earn(&self, model: &str, now: u64) -> Attestation {
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
            let pinned = inner.pins.iter().any(|pins| {
                matches!(
                    check_measurements_opt(Some(pins), &checked, CONTROL),
                    MeasurementVerdict::Pinned { .. }
                )
            });
            if !pinned {
                return Attestation::NotAttested;
            }
            verified.push(checked);
        }
        let report_data: Vec<&[u8]> = verified.iter().map(|q| q.report_data.as_slice()).collect();
        match quote_bound_keys(&claimed, &report_data, &nonce) {
            Ok(keys) => {
                if let Ok(mut cache) = inner.cache.lock() {
                    // Replace, never merge: a rotation must drop the old key.
                    cache.insert(model.to_string(), (keys.clone(), now));
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
        if backend != NEAR_AI_BACKEND || !self.is_pinned() {
            return Attestation::Unsupported;
        }
        let now = (self.inner.now_unix)();
        if let Some(keys) = self.cached(model, now) {
            return Attestation::Keys(keys);
        }
        let _one_at_a_time = self.inner.refresh.lock().await;
        // Somebody else may have earned it while we waited.
        if let Some(keys) = self.cached(model, now) {
            return Attestation::Keys(keys);
        }
        self.earn(model, now).await
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
            pins,
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
            Attestation::Unsupported
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
            real_pins(),
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
}
