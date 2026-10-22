// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement service's boot sequence (spec sections 2, 5, 6 and 8).
//!
//! Boot is fail-closed: every dependency resolves before the listener
//! binds, and each refusal is a stable label ([`BootRefusal`]) the image
//! workflow greps for, so a pre-push check proves *which* rule refused
//! rather than that something did.
//!
//! # Order
//!
//! Cheap, local checks first, so each one is reachable without the
//! infrastructure the later ones need:
//!
//! 1. configuration ([`BootConfig::from_lookup`]):
//!    1. a model is configured, and a production build has a non-empty
//!       allowlist;
//!    2. this build has an adapter for it and it is allowed in this build
//!       (a production build refuses every development model);
//!    3. the per-model receipt signer pins parse;
//!    4. the issuer public keys parse;
//!    5. both caps are set;
//! 2. the limiter volume is mounted at the data directory;
//! 3. the dstack guest agent derives the limiter root, and a `tcjudge1`
//!    boot quote binds the two per-boot keys generated in memory;
//! 4. the sealed ledger opens under the boot rules;
//! 5. the catalog is fetched, and the model passes the production check
//!    (allowlisted, both flags, prices, a pin);
//! 6. for a NEAR AI model, the gateway report's DCAP-verified quotes bind
//!    every pinned key.
//!
//! A NEAR AI model also needs the gateway check of spec section 5, check 5
//! (NEAR AI's gateway quote verified at boot and its TLS key pinned on every
//! connection). This build does not have it, so a NEAR AI model refuses
//! before step 2 with `missing_control:judgement_near_gateway_pin`, and step
//! 6 is reached only by the tests of [`attest_model_pins`] until it lands.
//!
//! # The production build has no model yet
//!
//! Owner decision 1: the production allowlist is empty until the TEE-hosted
//! decision model passes section 5, so a production image refuses at step
//! 1.2 whatever it is configured with. Steps 5 and 6 are built and tested
//! here against fakes, and are what slice 9 reaches by adding the model and
//! its adapter. A development build (`judgement-development-models`) adds
//! the mock model, whose catalog entry is synthetic and whose receipt
//! signer is generated at boot; it skips step 6, since there is no gateway
//! report for it, and serves synthetic data only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use sha2::Digest as _;
use trace_commons_attestation::quote::{
    Collateral, QuoteVerifyError, REQUIRED_TCB_STATUS, VerifiedQuote, verify_quote,
};
use trace_commons_attestation::receipt::{
    model_ed25519_keys, model_entry_quotes, quote_bound_keys,
};

use super::catalog::{CATALOG_REFRESH_INTERVAL_SECS, CatalogEntry, parse_catalog_page};
use super::claim::{ISSUER_PUBLIC_KEYS_ENV, IssuerKeys, parse_issuer_public_keys};
use super::enclave::{JudgementAgent, JudgementEnclave};
use super::ledger::{LedgerBootError, SealedLedger};
use super::limiter::{ACCOUNT_MONTHLY_CAP_ENV, Limiter, LimiterCaps, OPERATOR_MONTHLY_BUDGET_ENV};
use super::model::JudgementModel;
use super::production::{
    MODEL_KEY_PINS_ENV, PRODUCTION_MODEL_ALLOWLIST, ProductionBootRefusal, ProductionCheck,
    parse_judgement_model_key_pins, require_pins_attested,
};
use super::receipt_check::JudgementCaller;
use super::service::JudgementService;
use crate::near_attestation::client::AttestedKeyReportClient;

/// The model the service calls, by catalog id.
pub const MODEL_ENV: &str = "TRACE_COMMONS_JUDGEMENT_MODEL";
/// Where the limiter volume is mounted.
pub const DATA_DIR_ENV: &str = "TRACE_COMMONS_JUDGEMENT_DATA_DIR";
/// The data directory when [`DATA_DIR_ENV`] is unset.
pub const DEFAULT_DATA_DIR: &str = "/data";
/// The NEAR AI API key, the one secret, injected encrypted at deploy. Read
/// only for a NEAR AI model.
pub const NEAR_AI_API_KEY_ENV: &str = "TRACE_COMMONS_JUDGEMENT_NEAR_AI_API_KEY";
/// NEAR AI's public model catalog.
pub const NEAR_AI_CATALOG_URL: &str = "https://cloud-api.near.ai/v1/model/list";
/// The bound on one catalog fetch.
pub const CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// Why boot refused. Each is a stable label: the binary prints it, and the
/// image workflow's pre-push checks match on it. None carries a value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum BootRefusal {
    #[error("judgement_model_unconfigured")]
    ModelUnconfigured,
    #[error("judgement_model_not_allowlisted")]
    ModelNotAllowlisted,
    /// Allowlisted, but this build has no adapter that can call it.
    #[error("judgement_model_adapter_absent")]
    ModelAdapterAbsent,
    #[error("judgement_model_key_pins_malformed")]
    ModelKeyPinsMalformed,
    #[error("judgement_issuer_keys_absent")]
    IssuerKeysAbsent,
    #[error("judgement_issuer_keys_malformed")]
    IssuerKeysMalformed,
    #[error("judgement_account_cap_absent")]
    AccountCapAbsent,
    #[error("judgement_operator_budget_absent")]
    OperatorBudgetAbsent,
    #[error("judgement_caps_malformed")]
    CapsMalformed,
    #[error("judgement_near_ai_api_key_absent")]
    NearAiApiKeyAbsent,
    #[error("judgement_limiter_volume_absent")]
    LimiterVolumeAbsent,
    #[error("judgement_enclave_unavailable")]
    EnclaveUnavailable,
    #[error("judgement_limiter_unreadable")]
    LimiterUnreadable,
    #[error("judgement_limiter_missing")]
    LimiterMissing,
    #[error("judgement_limiter_io")]
    LimiterIo,
    #[error("judgement_catalog_unavailable")]
    CatalogUnavailable,
    #[error("judgement_model_catalog_refused")]
    CatalogRefused,
    #[error("judgement_model_key_pin_absent")]
    KeyPinAbsent,
    #[error("judgement_gateway_report_unavailable")]
    GatewayReportUnavailable,
    #[error("judgement_model_key_pin_unattested")]
    KeyPinUnattested,
    /// A model that calls NEAR AI, in a build without the gateway check
    /// (spec section 5, check 5): NEAR AI's gateway quote verified at boot
    /// and its TLS key pinned on every connection. Named as a missing
    /// control, because nothing the operator configures can supply it.
    #[error("missing_control:judgement_near_gateway_pin")]
    NearGatewayPinMissing,
}

impl BootRefusal {
    /// The label, as printed.
    pub fn label(self) -> String {
        self.to_string()
    }
}

impl From<ProductionBootRefusal> for BootRefusal {
    fn from(refusal: ProductionBootRefusal) -> Self {
        match refusal {
            ProductionBootRefusal::NotAllowlisted => Self::ModelNotAllowlisted,
            ProductionBootRefusal::CatalogRefused => Self::CatalogRefused,
            ProductionBootRefusal::KeyPinAbsent => Self::KeyPinAbsent,
            ProductionBootRefusal::KeyPinUnattested => Self::KeyPinUnattested,
        }
    }
}

impl From<LedgerBootError> for BootRefusal {
    fn from(error: LedgerBootError) -> Self {
        match error {
            LedgerBootError::VolumeAbsent => Self::LimiterVolumeAbsent,
            LedgerBootError::Unreadable => Self::LimiterUnreadable,
            LedgerBootError::MissingWithMarker => Self::LimiterMissing,
            LedgerBootError::Io => Self::LimiterIo,
        }
    }
}

/// The catalog fetch failed. Carries nothing from the response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("judgement_catalog_fetch_failed")]
pub struct CatalogFetchFailed;

/// Where the catalog comes from: NEAR AI's for a NEAR AI model, a fixed
/// synthetic entry for a development model.
#[async_trait]
pub trait CatalogSource: Send + Sync {
    async fn fetch(&self) -> Result<BTreeMap<String, CatalogEntry>, CatalogFetchFailed>;
}

/// `GET https://cloud-api.near.ai/v1/model/list`, public and
/// unauthenticated. One page: the live catalog's 59 models fit in the
/// default page of 100, and a model that a page leaves out is absent, which
/// refuses it rather than admitting it.
pub struct HttpCatalog {
    http: reqwest::Client,
    url: String,
}

impl HttpCatalog {
    pub fn new(url: impl Into<String>) -> Result<Self, CatalogFetchFailed> {
        let http = reqwest::Client::builder()
            .timeout(CATALOG_FETCH_TIMEOUT)
            .build()
            .map_err(|_| CatalogFetchFailed)?;
        Ok(Self {
            http,
            url: url.into(),
        })
    }
}

#[async_trait]
impl CatalogSource for HttpCatalog {
    async fn fetch(&self) -> Result<BTreeMap<String, CatalogEntry>, CatalogFetchFailed> {
        let response = self
            .http
            .get(&self.url)
            .send()
            .await
            .map_err(|_| CatalogFetchFailed)?;
        if !response.status().is_success() {
            return Err(CatalogFetchFailed);
        }
        let body = response.bytes().await.map_err(|_| CatalogFetchFailed)?;
        parse_catalog_page(&body).map_err(|_| CatalogFetchFailed)
    }
}

/// A fixed catalog. Development models only: their entries are not NEAR
/// AI's, and a production build has no development model to hold one for.
#[cfg(any(test, feature = "judgement-development-models"))]
pub struct FixedCatalog(pub BTreeMap<String, CatalogEntry>);

#[cfg(any(test, feature = "judgement-development-models"))]
#[async_trait]
impl CatalogSource for FixedCatalog {
    async fn fetch(&self) -> Result<BTreeMap<String, CatalogEntry>, CatalogFetchFailed> {
        Ok(self.0.clone())
    }
}

/// How a model's receipt signers are vouched for at boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignerAttestation {
    /// The gateway report's DCAP-verified quotes must bind every pin
    /// (spec section 5, check 3).
    NearAiGateway,
    /// A development model signing with a key it generated at boot. No
    /// report exists for it.
    #[cfg(any(test, feature = "judgement-development-models"))]
    DevelopmentSelfSigned,
}

/// A configured model, with the check and the catalog that go with it.
pub struct ResolvedModel {
    model: Arc<dyn JudgementModel>,
    check: ProductionCheck,
    catalog: Arc<dyn CatalogSource>,
    attestation: SignerAttestation,
}

impl std::fmt::Debug for ResolvedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedModel")
            .field("model", &self.model.model_id())
            .field("attestation", &self.attestation)
            .finish_non_exhaustive()
    }
}

impl ResolvedModel {
    /// A NEAR AI model behind the production check, with NEAR AI's catalog
    /// and gateway attestation. What slice 9 builds for the decision model.
    pub fn near_ai(
        model: Arc<dyn JudgementModel>,
        pins: BTreeMap<String, Vec<String>>,
        catalog: Arc<dyn CatalogSource>,
    ) -> Self {
        Self {
            model,
            check: ProductionCheck::production(pins),
            catalog,
            attestation: SignerAttestation::NearAiGateway,
        }
    }

    pub fn model_id(&self) -> &str {
        self.model.model_id()
    }

    pub fn attestation(&self) -> SignerAttestation {
        self.attestation
    }

    /// A model resolved over a test check, so the production boot path can
    /// be driven with the mock.
    #[cfg(test)]
    pub(crate) fn for_tests(
        model: Arc<dyn JudgementModel>,
        check: ProductionCheck,
        catalog: Arc<dyn CatalogSource>,
        attestation: SignerAttestation,
    ) -> Self {
        Self {
            model,
            check,
            catalog,
            attestation,
        }
    }

    /// The mock model, signing with a key generated now, its synthetic
    /// catalog entry, and a development check that admits it alone.
    #[cfg(feature = "judgement-development-models")]
    fn development_mock() -> Self {
        use super::catalog::TokenPrices;
        use super::mock::{MOCK_MODEL_ID, MockJudgementModel, MockReceiptSigner};

        let model = Arc::new(MockJudgementModel::new(MockReceiptSigner::generate()));
        let mut pins = BTreeMap::new();
        pins.insert(MOCK_MODEL_ID.to_string(), vec![model.signer_public_hex()]);
        let mut entries = BTreeMap::new();
        entries.insert(
            MOCK_MODEL_ID.to_string(),
            CatalogEntry {
                model_id: MOCK_MODEL_ID.to_string(),
                verifiable: true,
                attestation_supported: true,
                // The interim chat model's prices (spec "What already
                // exists"), so development metering is realistic.
                prices: Some(TokenPrices {
                    input_nanousd: 40,
                    output_nanousd: 1350,
                }),
            },
        );
        Self {
            model,
            check: ProductionCheck::development(&[MOCK_MODEL_ID], pins),
            catalog: Arc::new(FixedCatalog(entries)),
            attestation: SignerAttestation::DevelopmentSelfSigned,
        }
    }
}

/// Every model id this build can construct an adapter for.
///
/// A production build: the production adapters, of which there are none
/// until slice 9. A development build adds its development models. The
/// production-allowlist test (spec section 2) is over this list.
pub fn registry_model_ids() -> Vec<&'static str> {
    /// Production adapters. None until slice 9.
    const PRODUCTION_ADAPTERS: &[&str] = &[];
    #[cfg(feature = "judgement-development-models")]
    const DEVELOPMENT_ADAPTERS: &[&str] = &[super::mock::MOCK_MODEL_ID];
    #[cfg(not(feature = "judgement-development-models"))]
    const DEVELOPMENT_ADAPTERS: &[&str] = &[];
    PRODUCTION_ADAPTERS
        .iter()
        .chain(DEVELOPMENT_ADAPTERS)
        .copied()
        .collect()
}

/// A production build refuses to boot while its allowlist is empty, so
/// nothing but the allowlist can admit a production model: not a later
/// adapter registered ahead of its allowlist entry, and not configuration.
/// A development build boots its development models without one.
fn production_allowlist_check(
    development_build: bool,
    allowlist: &[&str],
) -> Result<(), BootRefusal> {
    if !development_build && allowlist.is_empty() {
        return Err(BootRefusal::ModelNotAllowlisted);
    }
    Ok(())
}

/// The configured model, if this build may call it.
pub fn resolve_model(
    model_id: &str,
    pins: &BTreeMap<String, Vec<String>>,
) -> Result<ResolvedModel, BootRefusal> {
    let _ = pins;
    production_allowlist_check(
        cfg!(feature = "judgement-development-models"),
        PRODUCTION_MODEL_ALLOWLIST,
    )?;
    if PRODUCTION_MODEL_ALLOWLIST.contains(&model_id) {
        // Allowlisted, but no production adapter exists in this slice:
        // `ChatStructuredAdapter` and `DecisionModelAdapter` arrive with the
        // model (slice 9). Refused by name rather than by allowlist.
        return Err(BootRefusal::ModelAdapterAbsent);
    }
    #[cfg(feature = "judgement-development-models")]
    if model_id == super::mock::MOCK_MODEL_ID {
        return Ok(ResolvedModel::development_mock());
    }
    Err(BootRefusal::ModelNotAllowlisted)
}

/// Everything boot reads from the environment, checked.
pub struct BootConfig {
    pub model: ResolvedModel,
    pub issuer: IssuerKeys,
    pub caps: LimiterCaps,
    pub data_dir: PathBuf,
    /// Present exactly when the model is a NEAR AI model.
    pub near_ai_api_key: Option<secrecy::SecretString>,
    /// The first 8 hex of a SHA-256 over the normalised pin set, for the
    /// boot log line. Carries neither a key nor a model name.
    pub pins_sha256_prefix: String,
}

impl std::fmt::Debug for BootConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BootConfig")
            .field("model", &self.model)
            .field("data_dir", &self.data_dir)
            .field("pins_sha256_prefix", &self.pins_sha256_prefix)
            .finish_non_exhaustive()
    }
}

fn pins_sha256_prefix(pins: &BTreeMap<String, Vec<String>>) -> String {
    let mut hasher = sha2::Sha256::new();
    for (model, keys) in pins {
        hasher.update(model.as_bytes());
        for key in keys {
            hasher.update(b"\0");
            hasher.update(key.as_bytes());
        }
        hasher.update(b"\n");
    }
    hex::encode(hasher.finalize())[..8].to_string()
}

fn whole_usd(value: Option<String>, absent: BootRefusal) -> Result<u64, BootRefusal> {
    let value = value.ok_or(absent)?;
    value
        .trim()
        .parse::<u64>()
        .map_err(|_| BootRefusal::CapsMalformed)
}

impl BootConfig {
    /// Read and check the configuration in the order the module docs give.
    /// `lookup` is `std::env::var(..).ok()` in the binary.
    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, BootRefusal> {
        let model_id = lookup(MODEL_ENV)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
            .ok_or(BootRefusal::ModelUnconfigured)?;
        let pins = match lookup(MODEL_KEY_PINS_ENV) {
            // Malformed, including empty, refuses: an operator who exported
            // the variable meant to pin something.
            Some(spec) => parse_judgement_model_key_pins(&spec)
                .map_err(|_| BootRefusal::ModelKeyPinsMalformed)?,
            None => BTreeMap::new(),
        };
        let model = resolve_model(&model_id, &pins)?;
        let issuer = parse_issuer_public_keys(
            &lookup(ISSUER_PUBLIC_KEYS_ENV).ok_or(BootRefusal::IssuerKeysAbsent)?,
        )
        .map_err(|_| BootRefusal::IssuerKeysMalformed)?;
        let account = whole_usd(
            lookup(ACCOUNT_MONTHLY_CAP_ENV),
            BootRefusal::AccountCapAbsent,
        )?;
        let operator = whole_usd(
            lookup(OPERATOR_MONTHLY_BUDGET_ENV),
            BootRefusal::OperatorBudgetAbsent,
        )?;
        let caps = LimiterCaps::from_usd(account, operator).ok_or(BootRefusal::CapsMalformed)?;
        let near_ai_api_key = match model.attestation {
            SignerAttestation::NearAiGateway => Some(secrecy::SecretString::from(
                lookup(NEAR_AI_API_KEY_ENV)
                    .filter(|key| !key.is_empty())
                    .ok_or(BootRefusal::NearAiApiKeyAbsent)?,
            )),
            #[cfg(any(test, feature = "judgement-development-models"))]
            SignerAttestation::DevelopmentSelfSigned => None,
        };
        let data_dir = lookup(DATA_DIR_ENV)
            .filter(|value| !value.is_empty())
            .map_or_else(|| PathBuf::from(DEFAULT_DATA_DIR), PathBuf::from);
        Ok(Self {
            pins_sha256_prefix: pins_sha256_prefix(&pins),
            model,
            issuer,
            caps,
            data_dir,
            near_ai_api_key,
        })
    }
}

/// Whether `dir` is a directory with a filesystem mounted on it: a
/// different device from its parent's. A directory baked into the image
/// sits on the read-only root and is not the limiter volume, so it is
/// refused rather than written to (and lost at the next upgrade).
#[cfg(unix)]
pub fn volume_is_mounted(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let Ok(own) = std::fs::metadata(dir) else {
        return false;
    };
    if !own.is_dir() {
        return false;
    }
    let parent = dir.parent().unwrap_or(Path::new("/"));
    std::fs::metadata(parent).is_ok_and(|parent| parent.dev() != own.dev())
}

/// What boot needs besides the configuration.
pub struct BootDeps {
    pub agent: Box<dyn JudgementAgent>,
    /// The gateway report source, for a NEAR AI model.
    pub reports: Option<Box<dyn AttestedKeyReportClient>>,
    /// [`volume_is_mounted`] in the binary.
    pub volume_is_mounted: fn(&Path) -> bool,
    /// A fresh 64-hex nonce for the gateway report.
    pub report_nonce: String,
    pub now: i64,
}

/// A booted service, ready to serve.
pub struct Booted {
    pub service: Arc<JudgementService>,
    pub enclave: Arc<JudgementEnclave>,
    pub check: Arc<ProductionCheck>,
    pub catalog: Arc<dyn CatalogSource>,
    pub model_id: String,
}

impl std::fmt::Debug for Booted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Booted")
            .field("enclave", &self.enclave)
            .field("model_id", &self.model_id)
            .finish_non_exhaustive()
    }
}

/// Spec section 5, check 5, which this build does not have: NEAR AI's
/// gateway quote, fetched in fingerprint mode and DCAP-verified against a
/// pinned gateway measurement, binding the TLS key every connection to the
/// gateway is then pinned to. It waits on a live fingerprint-mode fixture
/// and a direct `rustls` dependency (design open decision S2). Until it
/// lands, a model that calls NEAR AI does not boot: without it, a non-TEE
/// proxy or a mis-issued certificate for the gateway would see every digest
/// while the per-request receipts still verified.
fn require_near_gateway_pin(attestation: SignerAttestation) -> Result<(), BootRefusal> {
    match attestation {
        SignerAttestation::NearAiGateway => Err(BootRefusal::NearGatewayPinMissing),
        #[cfg(any(test, feature = "judgement-development-models"))]
        SignerAttestation::DevelopmentSelfSigned => Ok(()),
    }
}

/// Steps 2 to 6. See the module docs.
pub async fn boot(config: BootConfig, deps: BootDeps) -> Result<Booted, BootRefusal> {
    // Before step 2: the NEAR AI gateway check, so a model that needs it
    // asks nothing of the enclave and writes nothing to the volume.
    require_near_gateway_pin(config.model.attestation)?;
    // 2. The volume, before anything derives a key for it.
    if !(deps.volume_is_mounted)(&config.data_dir) {
        return Err(BootRefusal::LimiterVolumeAbsent);
    }
    // 3. The enclave.
    let enclave = JudgementEnclave::connect(deps.agent)
        .await
        .map_err(|_| BootRefusal::EnclaveUnavailable)?;
    // 4. The ledger.
    let limiter_keys = enclave.limiter_keys();
    let (ledger, state) = SealedLedger::open(&config.data_dir, limiter_keys.ledger_seal_key())?;
    // 5. The catalog and the production check.
    let model_id = config.model.model_id().to_string();
    let entries = config
        .model
        .catalog
        .fetch()
        .await
        .map_err(|_| BootRefusal::CatalogUnavailable)?;
    config
        .model
        .check
        .catalog()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .record_refresh(deps.now, entries);
    config.model.check.boot_check(&model_id, deps.now)?;
    // 6. The gateway attestation of the pins.
    match config.model.attestation {
        SignerAttestation::NearAiGateway => {
            let reports = deps
                .reports
                .as_deref()
                .ok_or(BootRefusal::GatewayReportUnavailable)?;
            let pinned = config
                .model
                .check
                .model_pins(&model_id)
                .ok_or(BootRefusal::KeyPinAbsent)?;
            let now = u64::try_from(deps.now).map_err(|_| BootRefusal::KeyPinUnattested)?;
            attest_model_pins(reports, &model_id, pinned, &deps.report_nonce, now).await?;
        }
        #[cfg(any(test, feature = "judgement-development-models"))]
        SignerAttestation::DevelopmentSelfSigned => {}
    }

    let check = Arc::new(config.model.check);
    let limiter = Arc::new(Limiter::new(config.caps, state, Arc::new(ledger)));
    let caller = JudgementCaller::new(config.model.model, check.clone(), limiter);
    let service = Arc::new(JudgementService::new(config.issuer, limiter_keys, caller));
    Ok(Booted {
        service,
        enclave: Arc::new(enclave),
        check,
        catalog: config.model.catalog,
        model_id,
    })
}

/// Spec section 5, check 3: fetch the gateway report with `nonce`, DCAP
/// verify each of the model's entry quotes against collateral fetched for
/// that quote, require Intel's `UpToDate` verdict, and require every pinned
/// key among the keys those verified quotes bind.
///
/// NEAR AI's own enclave measurements are not pinned here: that is the
/// attestation drill's job (spec section 5, "What it does not prove").
pub async fn attest_model_pins(
    reports: &dyn AttestedKeyReportClient,
    model_id: &str,
    pinned: &[String],
    nonce: &str,
    now_unix: u64,
) -> Result<(), BootRefusal> {
    attest_model_pins_with(reports, model_id, pinned, nonce, now_unix, &verify_quote).await
}

/// The DCAP verification [`attest_model_pins`] applies to each quote. A seam
/// so the accepting path is testable: no checked-in collateral verifies a
/// model enclave's quote.
type QuoteVerifier =
    dyn Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync;

async fn attest_model_pins_with(
    reports: &dyn AttestedKeyReportClient,
    model_id: &str,
    pinned: &[String],
    nonce: &str,
    now_unix: u64,
    verify: &QuoteVerifier,
) -> Result<(), BootRefusal> {
    let report = reports
        .fetch_ed25519_report_json(nonce)
        .await
        .map_err(|_| BootRefusal::GatewayReportUnavailable)?;
    let claimed =
        model_ed25519_keys(&report, nonce, model_id).map_err(|_| BootRefusal::KeyPinUnattested)?;
    let quotes =
        model_entry_quotes(&report, model_id).map_err(|_| BootRefusal::KeyPinUnattested)?;
    let mut verified = Vec::with_capacity(quotes.len());
    for quote in &quotes {
        let collateral = reports
            .fetch_collateral_for(quote)
            .await
            .map_err(|_| BootRefusal::GatewayReportUnavailable)?;
        let quote =
            verify(quote, &collateral, now_unix).map_err(|_| BootRefusal::KeyPinUnattested)?;
        if quote.tcb_status != REQUIRED_TCB_STATUS {
            return Err(BootRefusal::KeyPinUnattested);
        }
        verified.push(quote.report_data);
    }
    let report_data: Vec<&[u8]> = verified.iter().map(Vec::as_slice).collect();
    let bound = quote_bound_keys(&claimed, &report_data, nonce)
        .map_err(|_| BootRefusal::KeyPinUnattested)?;
    require_pins_attested(pinned, &bound)?;
    Ok(())
}

/// The hourly catalog refresh. A failed refresh keeps the last good catalog
/// (up to the 24-hour staleness rule in [`super::catalog`]). Logs a label
/// only.
pub fn spawn_catalog_refresh(
    check: Arc<ProductionCheck>,
    catalog: Arc<dyn CatalogSource>,
    clock: super::http::Clock,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let period = Duration::from_secs(CATALOG_REFRESH_INTERVAL_SECS.unsigned_abs());
        let mut ticks = tokio::time::interval_at(tokio::time::Instant::now() + period, period);
        loop {
            ticks.tick().await;
            refresh_once(&check, catalog.as_ref(), clock()).await;
        }
    })
}

/// One tick of [`spawn_catalog_refresh`]: fetch, then record the catalog or
/// the failure at `now`. Logs a label only.
pub async fn refresh_once(check: &ProductionCheck, catalog: &dyn CatalogSource, now: i64) {
    let outcome = catalog.fetch().await;
    let mut state = check
        .catalog()
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match outcome {
        Ok(entries) => {
            state.record_refresh(now, entries);
            tracing::info!(target: super::service::LOG_TARGET, outcome = "ok", "catalog_refresh");
        }
        Err(error) => {
            state.record_refresh_failure(now);
            tracing::warn!(target: super::service::LOG_TARGET, outcome = %error, "catalog_refresh");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::judgement_service::catalog::{
        CATALOG_STALE_AFTER_SECS, CatalogRefusal, TokenPrices,
    };
    use crate::judgement_service::claim::tests::{NOW, TestIssuer};
    use crate::judgement_service::enclave::tests::StubAgent;
    use crate::judgement_service::ledger::{LEDGER_FILE, MARKER_FILE};
    use crate::judgement_service::mock::{MOCK_MODEL_ID, MockJudgementModel, MockReceiptSigner};
    use crate::judgement_service::production::DEVELOPMENT_ONLY_MODEL_IDS;
    use crate::near_attestation::client::{AttestationClientError, AttestationStep};
    use crate::near_attestation::quote::{Collateral, parse_collateral};

    const PIN: &str = "0011223344556677889900112233445566778899001122334455667788990011";

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: BTreeMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |name| map.get(name).cloned()
    }

    fn refused(lookup: impl Fn(&str) -> Option<String>) -> BootRefusal {
        BootConfig::from_lookup(lookup).expect_err("refused")
    }

    #[test]
    fn no_model_is_the_first_refusal() {
        assert_eq!(refused(env(&[])), BootRefusal::ModelUnconfigured);
        assert_eq!(
            refused(env(&[(MODEL_ENV, "  ")])),
            BootRefusal::ModelUnconfigured
        );
        assert_eq!(
            BootRefusal::ModelUnconfigured.label(),
            "judgement_model_unconfigured"
        );
    }

    #[test]
    fn jev_is_refused_in_every_build_of_this_slice() {
        // Jev is a development model, and its adapter is slice 6: no build
        // of this slice may call it.
        assert_eq!(
            refused(env(&[(MODEL_ENV, "typesafe/jev-1.13")])),
            BootRefusal::ModelNotAllowlisted
        );
        assert_eq!(
            BootRefusal::ModelNotAllowlisted.label(),
            "judgement_model_not_allowlisted"
        );
    }

    #[test]
    fn an_unknown_model_is_refused() {
        assert_eq!(
            refused(env(&[(MODEL_ENV, "Qwen/Qwen3.8-27B")])),
            BootRefusal::ModelNotAllowlisted
        );
    }

    /// Spec section 2: with the development feature off, the registry
    /// holds no model outside the production allowlist, and every
    /// development model is refused.
    #[cfg(not(feature = "judgement-development-models"))]
    #[test]
    fn a_production_build_registers_nothing_outside_the_allowlist() {
        for id in registry_model_ids() {
            assert!(PRODUCTION_MODEL_ALLOWLIST.contains(&id), "{id} registered");
        }
        assert!(!DEVELOPMENT_ONLY_MODEL_IDS.is_empty());
        for id in DEVELOPMENT_ONLY_MODEL_IDS {
            assert!(!registry_model_ids().contains(id));
            assert_eq!(
                resolve_model(id, &BTreeMap::new()).unwrap_err(),
                BootRefusal::ModelNotAllowlisted,
                "{id} resolved in a production build"
            );
        }
    }

    #[cfg(feature = "judgement-development-models")]
    #[test]
    fn a_development_build_registers_the_mock_and_never_on_the_production_allowlist() {
        assert_eq!(registry_model_ids(), vec![MOCK_MODEL_ID]);
        let resolved = resolve_model(MOCK_MODEL_ID, &BTreeMap::new()).expect("mock");
        assert_eq!(
            resolved.attestation(),
            SignerAttestation::DevelopmentSelfSigned
        );
        for id in DEVELOPMENT_ONLY_MODEL_IDS {
            assert!(!PRODUCTION_MODEL_ALLOWLIST.contains(id));
        }
    }

    #[cfg(feature = "judgement-development-models")]
    fn mock_env<'a>(issuer: &'a str, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, &'a str)> {
        let mut pairs = vec![
            (MODEL_ENV, MOCK_MODEL_ID),
            (ISSUER_PUBLIC_KEYS_ENV, issuer),
            (ACCOUNT_MONTHLY_CAP_ENV, "5"),
            (OPERATOR_MONTHLY_BUDGET_ENV, "100"),
        ];
        pairs.extend_from_slice(extra);
        pairs
    }

    #[cfg(feature = "judgement-development-models")]
    #[test]
    fn the_remaining_configuration_refuses_in_order() {
        let issuer = TestIssuer::new();
        let key = issuer.public_hex().to_string();
        let without = |name: &str| {
            let pairs: Vec<_> = mock_env(&key, &[])
                .into_iter()
                .filter(|(n, _)| *n != name)
                .collect();
            BootConfig::from_lookup(env(&pairs)).expect_err("refused")
        };
        assert_eq!(
            without(ISSUER_PUBLIC_KEYS_ENV),
            BootRefusal::IssuerKeysAbsent
        );
        assert_eq!(
            without(ACCOUNT_MONTHLY_CAP_ENV),
            BootRefusal::AccountCapAbsent
        );
        assert_eq!(
            without(OPERATOR_MONTHLY_BUDGET_ENV),
            BootRefusal::OperatorBudgetAbsent
        );
        let with = |extra: &[(&str, &str)]| {
            let mut pairs = mock_env(&key, &[]);
            for (name, value) in extra {
                pairs.retain(|(n, _)| n != name);
                pairs.push((name, value));
            }
            BootConfig::from_lookup(env(&pairs))
        };
        assert_eq!(
            with(&[(ISSUER_PUBLIC_KEYS_ENV, "not-a-key")]).unwrap_err(),
            BootRefusal::IssuerKeysMalformed
        );
        assert_eq!(
            with(&[(ACCOUNT_MONTHLY_CAP_ENV, "five")]).unwrap_err(),
            BootRefusal::CapsMalformed
        );
        assert_eq!(
            with(&[(MODEL_KEY_PINS_ENV, "")]).unwrap_err(),
            BootRefusal::ModelKeyPinsMalformed
        );
        let config = with(&[]).expect("configured");
        assert_eq!(config.data_dir, PathBuf::from(DEFAULT_DATA_DIR));
        assert!(config.near_ai_api_key.is_none());
        assert_eq!(config.caps, LimiterCaps::from_usd(5, 100).expect("caps"));
    }

    #[cfg(unix)]
    #[test]
    fn only_a_mounted_directory_is_the_limiter_volume() {
        let dir = tempfile::tempdir().expect("tempdir");
        // On the same filesystem as its parent: baked into an image, not
        // a volume.
        assert!(!volume_is_mounted(dir.path()));
        assert!(!volume_is_mounted(&dir.path().join("absent")));
        // `/dev` is its own filesystem on Linux and macOS alike.
        assert!(volume_is_mounted(Path::new("/dev")));
    }

    fn mock_entries(verifiable: bool) -> BTreeMap<String, CatalogEntry> {
        let mut entries = BTreeMap::new();
        entries.insert(
            MOCK_MODEL_ID.to_string(),
            CatalogEntry {
                model_id: MOCK_MODEL_ID.to_string(),
                verifiable,
                attestation_supported: true,
                prices: Some(TokenPrices {
                    input_nanousd: 40,
                    output_nanousd: 1350,
                }),
            },
        );
        entries
    }

    struct NoCatalog;
    #[async_trait]
    impl CatalogSource for NoCatalog {
        async fn fetch(&self) -> Result<BTreeMap<String, CatalogEntry>, CatalogFetchFailed> {
            Err(CatalogFetchFailed)
        }
    }

    /// A configuration over the mock, resolved through a test check, so
    /// every later step runs in a build without the development feature.
    fn config(
        data_dir: &Path,
        catalog: Arc<dyn CatalogSource>,
        attestation: SignerAttestation,
        pinned: bool,
    ) -> BootConfig {
        let model = Arc::new(MockJudgementModel::new(MockReceiptSigner::generate()));
        let mut pins = BTreeMap::new();
        if pinned {
            pins.insert(MOCK_MODEL_ID.to_string(), vec![model.signer_public_hex()]);
        }
        let check = ProductionCheck::for_tests(&[MOCK_MODEL_ID], pins.clone());
        let issuer = TestIssuer::new();
        BootConfig {
            model: ResolvedModel::for_tests(model, check, catalog, attestation),
            issuer: issuer.keys(),
            caps: LimiterCaps::from_usd(5, 100).expect("caps"),
            data_dir: data_dir.to_path_buf(),
            near_ai_api_key: None,
            pins_sha256_prefix: pins_sha256_prefix(&pins),
        }
    }

    fn deps(agent: StubAgent) -> (BootDeps, Arc<StubAgent>) {
        let agent = Arc::new(agent);
        (
            BootDeps {
                agent: Box::new(agent.clone()),
                reports: None,
                volume_is_mounted: |dir| dir.is_dir(),
                report_nonce: "ab".repeat(32),
                now: NOW,
            },
            agent,
        )
    }

    fn dev_config(dir: &Path) -> BootConfig {
        config(
            dir,
            Arc::new(FixedCatalog(mock_entries(true))),
            SignerAttestation::DevelopmentSelfSigned,
            true,
        )
    }

    #[tokio::test]
    async fn a_development_model_boots_end_to_end() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (deps, _) = deps(StubAgent::new());
        let booted = boot(dev_config(dir.path()), deps).await.expect("booted");
        assert_eq!(booted.model_id, MOCK_MODEL_ID);
        assert!(dir.path().join(LEDGER_FILE).exists());
        assert!(dir.path().join(MARKER_FILE).exists());
    }

    #[tokio::test]
    async fn no_volume_refuses_before_the_enclave_is_asked_anything() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut deps, agent) = deps(StubAgent::new());
        deps.volume_is_mounted = |_| false;
        assert_eq!(
            boot(dev_config(dir.path()), deps).await.unwrap_err(),
            BootRefusal::LimiterVolumeAbsent
        );
        assert!(agent.keys_asked.lock().expect("lock").is_empty());
        assert_eq!(
            BootRefusal::LimiterVolumeAbsent.label(),
            "judgement_limiter_volume_absent"
        );
    }

    #[tokio::test]
    async fn no_guest_agent_refuses_the_boot() {
        struct Absent;
        #[async_trait]
        impl JudgementAgent for Absent {
            async fn derive_key(
                &self,
                _: &str,
                _: &str,
            ) -> Result<zeroize::Zeroizing<[u8; 32]>, crate::witness_service::enclave::EnclaveError>
            {
                Err(crate::witness_service::enclave::EnclaveError::Transport)
            }
            async fn quote(
                &self,
                _: &[u8; 64],
            ) -> Result<Vec<u8>, crate::witness_service::enclave::EnclaveError> {
                Err(crate::witness_service::enclave::EnclaveError::Transport)
            }
        }
        let dir = tempfile::tempdir().expect("tempdir");
        let (mut deps, _) = deps(StubAgent::new());
        deps.agent = Box::new(Absent);
        assert_eq!(
            boot(dev_config(dir.path()), deps).await.unwrap_err(),
            BootRefusal::EnclaveUnavailable
        );
        // Nothing was written to the volume.
        assert!(!dir.path().join(MARKER_FILE).exists());
    }

    #[tokio::test]
    async fn an_unreadable_ledger_or_a_lost_one_refuses_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(LEDGER_FILE), b"not sealed").expect("write");
        let (d, _) = deps(StubAgent::new());
        assert_eq!(
            boot(dev_config(dir.path()), d).await.unwrap_err(),
            BootRefusal::LimiterUnreadable
        );
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(dir.path().join(MARKER_FILE), b"1\n").expect("write");
        let (d, _) = deps(StubAgent::new());
        assert_eq!(
            boot(dev_config(dir.path()), d).await.unwrap_err(),
            BootRefusal::LimiterMissing
        );
    }

    #[tokio::test]
    async fn a_restart_reads_the_ledger_back() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (d, _) = deps(StubAgent::new());
        boot(dev_config(dir.path()), d).await.expect("first boot");
        let (d, _) = deps(StubAgent::new());
        boot(dev_config(dir.path()), d).await.expect("second boot");
    }

    #[tokio::test]
    async fn the_catalog_and_the_production_check_refuse_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (d, _) = deps(StubAgent::new());
        let unavailable = config(
            dir.path(),
            Arc::new(NoCatalog),
            SignerAttestation::DevelopmentSelfSigned,
            true,
        );
        assert_eq!(
            boot(unavailable, d).await.unwrap_err(),
            BootRefusal::CatalogUnavailable
        );
        let (d, _) = deps(StubAgent::new());
        let unflagged = config(
            dir.path(),
            Arc::new(FixedCatalog(mock_entries(false))),
            SignerAttestation::DevelopmentSelfSigned,
            true,
        );
        assert_eq!(
            boot(unflagged, d).await.unwrap_err(),
            BootRefusal::CatalogRefused
        );
        let (d, _) = deps(StubAgent::new());
        let unpinned = config(
            dir.path(),
            Arc::new(FixedCatalog(mock_entries(true))),
            SignerAttestation::DevelopmentSelfSigned,
            false,
        );
        assert_eq!(
            boot(unpinned, d).await.unwrap_err(),
            BootRefusal::KeyPinAbsent
        );
    }

    const ED25519_REPORT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_model_attestation_report_ed25519.json"
    );
    const COLLATERAL: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
    );
    const FIXTURE_MODEL: &str = "Qwen/Qwen3.6-35B-A3B-FP8";
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    fn fixture_nonce() -> String {
        let value: serde_json::Value = serde_json::from_str(ED25519_REPORT).expect("json");
        value["gateway_attestation"]["request_nonce"]
            .as_str()
            .expect("nonce")
            .to_string()
    }

    /// Serves the captured report, and collateral that (measured, see
    /// `key_drift_tests`) does not verify a model enclave's quote.
    struct CapturedReports {
        report: Option<&'static str>,
        collateral: bool,
    }

    #[async_trait]
    impl AttestedKeyReportClient for CapturedReports {
        fn model(&self) -> &str {
            FIXTURE_MODEL
        }
        async fn fetch_ed25519_report_json(
            &self,
            _nonce: &str,
        ) -> Result<String, AttestationClientError> {
            self.report
                .map(str::to_string)
                .ok_or(AttestationClientError::HttpStatus {
                    step: AttestationStep::Report,
                    status: 503,
                })
        }
        async fn fetch_collateral_for(
            &self,
            _quote: &[u8],
        ) -> Result<Collateral, AttestationClientError> {
            if self.collateral {
                Ok(parse_collateral(COLLATERAL).expect("collateral"))
            } else {
                Err(AttestationClientError::HttpStatus {
                    step: AttestationStep::Collateral,
                    status: 503,
                })
            }
        }
    }

    #[tokio::test]
    async fn an_unreachable_gateway_report_refuses_the_pins() {
        let reports = CapturedReports {
            report: None,
            collateral: true,
        };
        assert_eq!(
            attest_model_pins(
                &reports,
                FIXTURE_MODEL,
                &[PIN.to_string()],
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT
            )
            .await
            .unwrap_err(),
            BootRefusal::GatewayReportUnavailable
        );
    }

    #[tokio::test]
    async fn a_report_for_another_nonce_or_model_refuses_the_pins() {
        let reports = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: true,
        };
        assert_eq!(
            attest_model_pins(
                &reports,
                FIXTURE_MODEL,
                &[PIN.to_string()],
                &"cd".repeat(32),
                FIXTURE_CAPTURED_AT
            )
            .await
            .unwrap_err(),
            BootRefusal::KeyPinUnattested
        );
        assert_eq!(
            attest_model_pins(
                &reports,
                "Qwen/Qwen3.8-27B",
                &[PIN.to_string()],
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT
            )
            .await
            .unwrap_err(),
            BootRefusal::KeyPinUnattested
        );
    }

    #[tokio::test]
    async fn a_quote_that_does_not_dcap_verify_refuses_the_pins() {
        // The JSON binds the key and nonce; only DCAP stands between that
        // and acceptance, and the collateral here is another machine's.
        let reports = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: true,
        };
        let claimed =
            model_ed25519_keys(ED25519_REPORT, &fixture_nonce(), FIXTURE_MODEL).expect("claimed");
        assert_eq!(
            attest_model_pins(
                &reports,
                FIXTURE_MODEL,
                &claimed,
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT
            )
            .await
            .unwrap_err(),
            BootRefusal::KeyPinUnattested
        );
        let no_collateral = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: false,
        };
        assert_eq!(
            attest_model_pins(
                &no_collateral,
                FIXTURE_MODEL,
                &claimed,
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT
            )
            .await
            .unwrap_err(),
            BootRefusal::GatewayReportUnavailable
        );
    }

    /// Stands in for DCAP only: reads `report_data` straight out of the
    /// quote (TDX v4: a 48-byte header, then the TD report body with
    /// `report_data` at its offset 520) and reports `tcb_status`. Every other
    /// step of [`attest_model_pins`] runs as in production.
    /// The offset is pinned by `pins_every_verified_quote_binds_are_accepted`:
    /// read from anywhere else, the nonce would not match and it would fail.
    fn trusting_verifier(
        tcb_status: &'static str,
    ) -> impl Fn(&[u8], &Collateral, u64) -> Result<VerifiedQuote, QuoteVerifyError> + Sync {
        move |quote, _collateral, _now| {
            Ok(VerifiedQuote {
                report_data: quote[568..632].to_vec(),
                mrtd: String::new(),
                mr_config_id: String::new(),
                rtmr: Default::default(),
                tcb_status: tcb_status.to_string(),
                advisory_ids: Vec::new(),
            })
        }
    }

    #[tokio::test]
    async fn pins_every_verified_quote_binds_are_accepted() {
        let reports = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: true,
        };
        let claimed =
            model_ed25519_keys(ED25519_REPORT, &fixture_nonce(), FIXTURE_MODEL).expect("claimed");
        attest_model_pins_with(
            &reports,
            FIXTURE_MODEL,
            &claimed,
            &fixture_nonce(),
            FIXTURE_CAPTURED_AT,
            &trusting_verifier(REQUIRED_TCB_STATUS),
        )
        .await
        .expect("every pin is bound by a verified quote");
    }

    #[tokio::test]
    async fn one_unbound_pin_among_two_refuses() {
        let reports = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: true,
        };
        let claimed =
            model_ed25519_keys(ED25519_REPORT, &fixture_nonce(), FIXTURE_MODEL).expect("claimed");
        assert!(!claimed.contains(&PIN.to_string()));
        assert_eq!(
            attest_model_pins_with(
                &reports,
                FIXTURE_MODEL,
                &[claimed[0].clone(), PIN.to_string()],
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT,
                &trusting_verifier(REQUIRED_TCB_STATUS),
            )
            .await
            .unwrap_err(),
            BootRefusal::KeyPinUnattested
        );
    }

    #[tokio::test]
    async fn a_platform_behind_on_its_tcb_refuses_the_pins() {
        let reports = CapturedReports {
            report: Some(ED25519_REPORT),
            collateral: true,
        };
        let claimed =
            model_ed25519_keys(ED25519_REPORT, &fixture_nonce(), FIXTURE_MODEL).expect("claimed");
        assert_eq!(
            attest_model_pins_with(
                &reports,
                FIXTURE_MODEL,
                &claimed,
                &fixture_nonce(),
                FIXTURE_CAPTURED_AT,
                &trusting_verifier("OutOfDate"),
            )
            .await
            .unwrap_err(),
            BootRefusal::KeyPinUnattested
        );
    }

    #[tokio::test]
    async fn a_refresh_replaces_the_catalog_and_a_failed_one_keeps_it() {
        let check = ProductionCheck::for_tests(&[MOCK_MODEL_ID], BTreeMap::new());
        let state = |now: i64| {
            check
                .catalog()
                .read()
                .expect("catalog")
                .check(MOCK_MODEL_ID, now)
        };
        assert_eq!(state(NOW), Err(CatalogRefusal::Absent));
        refresh_once(&check, &FixedCatalog(mock_entries(true)), NOW).await;
        assert!(state(NOW).is_ok());
        // A failed refresh keeps the last good catalog, and does not move
        // the time the staleness rule measures from.
        refresh_once(&check, &NoCatalog, NOW + 3_600).await;
        assert!(state(NOW + 3_600).is_ok());
        assert_eq!(
            state(NOW + CATALOG_STALE_AFTER_SECS + 1),
            Err(CatalogRefusal::Stale)
        );
        // A later good refresh replaces what was there.
        refresh_once(
            &check,
            &FixedCatalog(mock_entries(false)),
            NOW + CATALOG_STALE_AFTER_SECS,
        )
        .await;
        assert_eq!(
            state(NOW + CATALOG_STALE_AFTER_SECS + 1),
            Err(CatalogRefusal::ModelUnattested)
        );
    }

    /// #1501 review point 2, #1531 review point 2: until the gateway check
    /// (spec section 5, check 5) is compiled in and configured, a NEAR AI
    /// model refuses to boot by that control's name, whatever else is in
    /// order, and before the guest agent is asked for anything.
    #[tokio::test]
    async fn a_near_ai_model_refuses_until_the_gateway_pin_check_exists() {
        assert_eq!(
            BootRefusal::NearGatewayPinMissing.label(),
            "missing_control:judgement_near_gateway_pin"
        );
        for reports in [false, true] {
            let dir = tempfile::tempdir().expect("tempdir");
            let (mut d, agent) = deps(StubAgent::new());
            if reports {
                d.reports = Some(Box::new(CapturedReports {
                    report: Some(ED25519_REPORT),
                    collateral: true,
                }));
            }
            let near_ai = config(
                dir.path(),
                Arc::new(FixedCatalog(mock_entries(true))),
                SignerAttestation::NearAiGateway,
                true,
            );
            assert_eq!(
                boot(near_ai, d).await.unwrap_err(),
                BootRefusal::NearGatewayPinMissing
            );
            assert!(agent.keys_asked.lock().expect("lock").is_empty());
            assert!(
                std::fs::read_dir(dir.path()).expect("dir").next().is_none(),
                "nothing written to the volume"
            );
        }
    }

    /// A model that does not call NEAR AI is not held to the gateway check.
    #[tokio::test]
    async fn a_development_model_is_not_held_to_the_gateway_check() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (d, _) = deps(StubAgent::new());
        boot(dev_config(dir.path()), d)
            .await
            .expect("the mock calls nobody");
    }

    #[test]
    fn a_production_build_with_an_empty_allowlist_refuses() {
        assert_eq!(
            production_allowlist_check(false, &[]),
            Err(BootRefusal::ModelNotAllowlisted)
        );
        assert_eq!(production_allowlist_check(false, &["vendor/model"]), Ok(()));
        // A development build boots its development models without one.
        assert_eq!(production_allowlist_check(true, &[]), Ok(()));
        // The state of this slice: nothing is allowlisted, so a production
        // build refuses every model it could be told to call.
        assert!(PRODUCTION_MODEL_ALLOWLIST.is_empty());
        #[cfg(not(feature = "judgement-development-models"))]
        for id in ["vendor/model", "typesafe/jev-1.13"] {
            assert_eq!(
                resolve_model(id, &BTreeMap::new()).unwrap_err(),
                BootRefusal::ModelNotAllowlisted
            );
        }
    }

    #[test]
    fn the_pin_digest_carries_no_key_or_model() {
        let mut pins = BTreeMap::new();
        pins.insert("vendor/model".to_string(), vec![PIN.to_string()]);
        let prefix = pins_sha256_prefix(&pins);
        assert_eq!(prefix.len(), 8);
        assert!(!prefix.contains("vendor"));
        assert_ne!(prefix, pins_sha256_prefix(&BTreeMap::new()));
    }

    #[test]
    fn every_label_is_a_judgement_label() {
        for refusal in [
            BootRefusal::ModelUnconfigured,
            BootRefusal::ModelNotAllowlisted,
            BootRefusal::ModelAdapterAbsent,
            BootRefusal::LimiterVolumeAbsent,
            BootRefusal::EnclaveUnavailable,
            BootRefusal::KeyPinUnattested,
        ] {
            assert!(refusal.label().starts_with("judgement_"));
        }
        // A missing control is named as one, the shape the drills use.
        assert!(
            BootRefusal::NearGatewayPinMissing
                .label()
                .starts_with("missing_control:judgement_")
        );
    }
}
