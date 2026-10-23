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
//! **A staging build takes them only from a signed set** (`near-ai-staging`;
//! design: `docs/superpowers/specs/2026-10-21-staging-environment-design.md`,
//! section 5). Neither ingest's document nor the override above is compiled
//! in. [`PinProvider`] fetches `crate::channel::STAGING_PIN_URL`, verifies it
//! against the compiled-in `crate::channel::STAGING_PIN_PUBLIC_KEY` and the
//! build's gateway host with `trace_commons_attestation::staging_pins`, and
//! refuses a sequence lower than the highest this state directory accepted
//! (`SEQUENCE_FILE`) or that sequence again with other bytes. Refresh
//! timing is the same as above; a failed fetch keeps the last good set until
//! [`PIN_TTL`] and never past the set's own expiry, and a document that is
//! fetched and refused pins nothing at once, never an older set. The status
//! reports the outcome as a label (`PinProvider::status_label`).
//!
//! **A staging build also replays RTMR3** (owner decision S16; spec 5.5,
//! steps 6 and 7). Its sets pin `mrtd`, `rtmr0-2` and `mrconfigid`, not a
//! static `rtmr3`, and the signed document carries an allowlist beside them.
//! For every model entry whose key it depends on, after the quote has
//! verified and matched a set, it replays that entry's own `event_log`
//! against the verified quote's RTMR3 and checks the events against the
//! allowlist (`staging_replay::replay_rtmr3`, `check_rtmr3_allowlist`), then
//! checks that entry's `compose_manager_attestation`: its quote verified the
//! same way, the same TD and boot as the model quote, our nonce, the
//! recomputed actions hash, and every action parsed by kind and allowed
//! (`staging_replay::check_compose_manager`; owner decision S17: the compose
//! manager's image by digest, a `compose_up`'s file by path, SHA-256 and
//! commit, a `compose_down`'s by path and commit). All from the one report this
//! answer fetched with its own nonce. A key is earned only if the set, the
//! replay, the RTMR3 allowlist and the compose-manager checks all pass. The
//! last outcome is reported as a label too
//! (`PinProvider::attestation_label`).
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
#[cfg(not(feature = "near-ai-staging"))]
use trace_commons_attestation::receipt::model_entry_quotes;
use trace_commons_attestation::receipt::{AttestedKeyError, quote_bound_keys};
#[cfg(feature = "near-ai-staging")]
use trace_commons_attestation::staging_replay::{
    Allowlist, ComposeManagerRecord, EntryEvidence, check_compose_manager, check_rtmr3_allowlist,
    model_entry_evidence, replay_rtmr3,
};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
#[cfg(not(feature = "near-ai-staging"))]
use trace_commons_protocol::near_ai_measurements::NearAiMeasurementPins;

/// The IronWire backend id whose receipts this attestor can speak for.
pub const NEAR_AI_BACKEND: &str = "nearai";

/// The NEAR AI gateway: the only host that publishes the per-model
/// attestation registry (a per-model completions host answers with its own
/// single enclave and no registry). Also IronWire's default NEAR AI base, so
/// the key sent here is the key IronWire already sends here.
pub const NEAR_AI_GATEWAY: &str = crate::channel::NEAR_AI_ATTESTATION_GATEWAY;

/// The device's measurement-pin override. The server's variable name, and
/// each set in the server's syntax; unlike the server, it accepts several
/// sets split by `;` (see [`parse_pins`]). Not compiled into a staging
/// build, which pins only the signed set.
#[cfg(not(feature = "near-ai-staging"))]
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
/// `GET /v1/contributors/me/near-ai-measurements`. Not compiled into a
/// staging build, whose only source is the signed pin set.
#[cfg(not(feature = "near-ai-staging"))]
#[async_trait::async_trait]
pub trait PinSource: Send + Sync {
    /// The published document.
    async fn fetch(&self) -> Result<NearAiMeasurementPins, SourceUnavailable>;
}

/// The production source: ingest, with this device's own credential.
#[cfg(not(feature = "near-ai-staging"))]
pub struct IngestPinSource {
    store: crate::config::ConfigStore,
}

#[cfg(not(feature = "near-ai-staging"))]
impl IngestPinSource {
    /// Read through `store`'s enrollment and device identity.
    #[must_use]
    pub fn new(store: crate::config::ConfigStore) -> Self {
        Self { store }
    }
}

#[cfg(not(feature = "near-ai-staging"))]
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

/// Where a staging build's signed pin set comes from (spec 5.3).
#[cfg(feature = "near-ai-staging")]
#[async_trait::async_trait]
pub trait SignedPinSource: Send + Sync {
    /// The signed document's exact bytes.
    async fn fetch(&self) -> Result<Vec<u8>, SourceUnavailable>;
}

/// The largest signed pin-set document read. A real one is a few hundred
/// bytes per set; this bounds what a misbehaving server can make the daemon
/// buffer.
#[cfg(feature = "near-ai-staging")]
pub const MAX_PIN_DOCUMENT_BYTES: usize = 64 * 1024;

/// The file in the state directory that holds the highest signed sequence
/// accepted and the SHA-256 of the payload accepted under it, as
/// `<sequence> <64 hex>\n`. A lower sequence is refused, and so is the same
/// sequence carrying other bytes (spec 5.5, step 4).
#[cfg(feature = "near-ai-staging")]
pub const SEQUENCE_FILE: &str = "near-ai-pins-sequence";

/// The staging source: the compiled-in pin URL, over the same client the
/// receipt and report fetches use (no redirects, a timeout).
#[cfg(feature = "near-ai-staging")]
pub struct StagingPinSource {
    http: reqwest::Client,
    url: url::Url,
    store: Option<crate::config::ConfigStore>,
}

#[cfg(feature = "near-ai-staging")]
impl StagingPinSource {
    fn new(
        url: &str,
        store: Option<crate::config::ConfigStore>,
    ) -> Result<Self, SourceUnavailable> {
        Ok(Self {
            http: crate::routing::receipt::receipt_http_client().map_err(|_| SourceUnavailable)?,
            url: url::Url::parse(url).map_err(|_| SourceUnavailable)?,
            store,
        })
    }
}

#[cfg(feature = "near-ai-staging")]
impl std::fmt::Debug for StagingPinSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StagingPinSource").finish_non_exhaustive()
    }
}

#[cfg(feature = "near-ai-staging")]
#[async_trait::async_trait]
impl SignedPinSource for StagingPinSource {
    async fn fetch(&self) -> Result<Vec<u8>, SourceUnavailable> {
        // An operator who narrowed egress keeps it narrowed: an enforcing
        // allowlist, from the config or else the environment, must admit the
        // pin URL too. A config that cannot be loaded fails closed rather
        // than skipping the check.
        let cfg = match self.store.as_ref() {
            Some(store) => store.load_config().map_err(|_| SourceUnavailable)?,
            None => None,
        };
        let configured =
            crate::config::allowlist_for(cfg.as_ref().and_then(|c| c.allowed_hosts.as_deref()));
        if configured.is_enforcing() {
            configured.check(&self.url).map_err(|_| SourceUnavailable)?;
        }
        let mut response = self
            .http
            .get(self.url.clone())
            .send()
            .await
            .map_err(|_| SourceUnavailable)?;
        if !response.status().is_success() {
            return Err(SourceUnavailable);
        }
        if response
            .content_length()
            .is_some_and(|declared| declared > MAX_PIN_DOCUMENT_BYTES as u64)
        {
            return Err(SourceUnavailable);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| SourceUnavailable)? {
            if body.len().saturating_add(chunk.len()) > MAX_PIN_DOCUMENT_BYTES {
                return Err(SourceUnavailable);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(body)
    }
}

/// The status label while pins are in force.
pub const PINNED: &str = "pinned";
/// The status label before the first fetch.
pub const PINS_NOT_FETCHED: &str = "pins_not_fetched";
/// The status label when the provider has no source at all.
pub const PINS_NONE: &str = "pins_none";
/// The status label when the document could not be fetched and no good set
/// is left to keep.
pub const PINS_UNAVAILABLE: &str = "pins_unavailable";
/// The status label when the set in force reached its own `expires_at`.
/// The same label the verifier gives an expired document.
pub const PINS_EXPIRED: &str = "pins_expired";
/// A signed sequence lower than one this state directory accepted.
#[cfg(feature = "near-ai-staging")]
pub const PINS_ROLLBACK: &str = "pins_rollback";
/// The accepted sequence again, carrying other bytes.
#[cfg(feature = "near-ai-staging")]
pub const PINS_SEQUENCE_CONFLICT: &str = "pins_sequence_conflict";
/// The sequence record exists but cannot be read or parsed. Fails closed:
/// without it a rollback cannot be told from an update.
#[cfg(feature = "near-ai-staging")]
pub const PINS_SEQUENCE_UNREADABLE: &str = "pins_sequence_unreadable";
/// A higher sequence that could not be recorded. Fails closed: a set
/// accepted without its record could be rolled back on the next start.
#[cfg(feature = "near-ai-staging")]
pub const PINS_SEQUENCE_UNWRITABLE: &str = "pins_sequence_unwritable";

/// The attestation label before any answer has been earned.
#[cfg(feature = "near-ai-staging")]
pub const ATTESTATION_NOT_CHECKED: &str = "not_checked";
/// The attestation label when the last answer earned keys.
#[cfg(feature = "near-ai-staging")]
pub const ATTESTED: &str = "attested";
/// Sets are in force but no allowlist came with them.
#[cfg(feature = "near-ai-staging")]
const PINS_NO_ALLOWLIST: &str = "pins_no_allowlist";
// The labels below name why an answer earned no key. Only a staging build
// reports them (`PinProvider::attestation_label`).

/// The report holds no model entry this answer can use.
const REPORT_NOT_ATTESTED: &str = "report_not_attested";
/// A model quote did not verify against Intel collateral.
const QUOTE_UNVERIFIED: &str = "quote_unverified";
/// A model quote verified on a platform behind on Intel's fixes.
const QUOTE_TCB_NOT_UP_TO_DATE: &str = "quote_tcb_not_up_to_date";
/// A model quote's registers match no pinned set.
const QUOTE_NOT_PINNED: &str = "quote_not_pinned";
/// The verified quotes do not bind the claimed keys under our nonce.
const QUOTE_KEYS_UNBOUND: &str = "quote_keys_unbound";
/// The pins in force right now, and a digest to tell when they changed.
#[derive(Debug, Clone, Default)]
pub struct CurrentPins {
    /// Any one matching admits an image. Empty admits none.
    pub sets: Vec<ExpectedMeasurements>,
    /// Changes whenever `sets` does. A change detector for the cache, not an
    /// integrity check: nothing signs it.
    pub digest: String,
    /// What a staging build checks each entry's RTMR3 log and
    /// compose-manager record against, signed with the sets. `None` earns no
    /// key: a set without its allowlist pins nothing.
    #[cfg(feature = "near-ai-staging")]
    pub allowlist: Option<Arc<Allowlist>>,
}

struct CachedPins {
    pins: CurrentPins,
    fetched_at: u64,
    /// The signed set's own `expires_at`: past it the set is not served,
    /// however recently it was fetched. `None` for ingest's document, which
    /// carries no expiry.
    valid_until: Option<u64>,
}

impl CachedPins {
    fn usable_at(&self, now_unix: u64) -> bool {
        self.valid_until.is_none_or(|until| now_unix < until)
    }
}

/// What one fetch produced.
enum Fetched {
    /// A document that grants these pins (possibly none).
    Pins {
        pins: CurrentPins,
        valid_until: Option<u64>,
    },
    /// A document that was fetched and refused, under this label. Pins
    /// nothing at once: it never falls back to an older set.
    #[cfg(feature = "near-ai-staging")]
    Refused(&'static str),
}

/// Where the provider's pins come from.
enum Mode {
    /// No source: nothing is pinned.
    Unpinned,
    /// A fixed list: the operator override in a default build, or a test's.
    #[cfg(any(test, not(feature = "near-ai-staging")))]
    Fixed(FixedPins),
    /// Ingest's unsigned document (default build only).
    #[cfg(not(feature = "near-ai-staging"))]
    Published(Box<dyn PinSource>),
    /// The signed staging set (staging build only).
    #[cfg(feature = "near-ai-staging")]
    Signed(SignedPins),
}

/// A fixed provider's pins.
#[cfg(any(test, not(feature = "near-ai-staging")))]
struct FixedPins {
    sets: Vec<ExpectedMeasurements>,
    /// A staging test's allowlist; `None` earns no key in a staging build.
    #[cfg(feature = "near-ai-staging")]
    allowlist: Option<Arc<Allowlist>>,
}

/// The signed source with what it is checked against, all compiled in.
#[cfg(feature = "near-ai-staging")]
struct SignedPins {
    source: Box<dyn SignedPinSource>,
    public_key: Vec<u8>,
    /// The build's NEAR AI gateway host. `None` only if the compiled-in
    /// gateway does not parse, which refuses every document.
    gateway_host: Option<String>,
    sequence_file: std::path::PathBuf,
}

/// The host of the build's NEAR AI attestation gateway, which a signed set
/// must name.
#[cfg(feature = "near-ai-staging")]
fn staging_gateway_host() -> Option<String> {
    url::Url::parse(NEAR_AI_GATEWAY)
        .ok()?
        .host_str()
        .map(str::to_string)
}

#[cfg(feature = "near-ai-staging")]
impl SignedPins {
    async fn fetch(&self, now_unix: u64) -> Result<Fetched, SourceUnavailable> {
        use trace_commons_attestation::staging_pins::{self, PinRefusal};
        let document = self.source.fetch().await?;
        let Some(gateway_host) = self.gateway_host.as_deref() else {
            return Ok(Fetched::Refused(PinRefusal::WrongGatewayHost.label()));
        };
        let verified =
            match staging_pins::verify(&document, &self.public_key, now_unix, gateway_host) {
                Ok(verified) => verified,
                Err(refusal) => return Ok(Fetched::Refused(refusal.label())),
            };
        let payload_sha256 = hex::encode(verified.payload_sha256);
        if let Err(label) = self.check_sequence(verified.sequence, &payload_sha256) {
            return Ok(Fetched::Refused(label));
        }
        Ok(Fetched::Pins {
            pins: CurrentPins {
                digest: format!("signed:{}:{payload_sha256}", verified.sequence),
                sets: verified.sets,
                allowlist: Some(Arc::new(verified.allowlist)),
            },
            valid_until: Some(verified.expires_at),
        })
    }

    /// Spec 5.5, step 4: refuse a lower sequence than the one recorded, and
    /// the recorded sequence with other payload bytes; record a higher one
    /// before it is used.
    fn check_sequence(&self, sequence: u64, payload_sha256: &str) -> Result<(), &'static str> {
        match std::fs::read_to_string(&self.sequence_file) {
            Ok(text) => {
                let (recorded, recorded_sha256) =
                    parse_sequence_record(&text).ok_or(PINS_SEQUENCE_UNREADABLE)?;
                if sequence < recorded {
                    return Err(PINS_ROLLBACK);
                }
                if sequence == recorded {
                    return if recorded_sha256 == payload_sha256 {
                        Ok(())
                    } else {
                        Err(PINS_SEQUENCE_CONFLICT)
                    };
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(PINS_SEQUENCE_UNREADABLE),
        }
        let dir = self
            .sequence_file
            .parent()
            .ok_or(PINS_SEQUENCE_UNWRITABLE)?;
        crate::config::write_atomic_0600(
            dir,
            &self.sequence_file,
            format!("{sequence} {payload_sha256}\n").as_bytes(),
        )
        .map_err(|_| PINS_SEQUENCE_UNWRITABLE)
    }
}

/// `<sequence> <64 lower-case hex>\n`, exactly.
#[cfg(feature = "near-ai-staging")]
fn parse_sequence_record(text: &str) -> Option<(u64, &str)> {
    let (sequence, sha256) = text.strip_suffix('\n')?.split_once(' ')?;
    if sequence.is_empty() || !sequence.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    if sha256.len() != 64
        || !sha256
            .bytes()
            .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        return None;
    }
    Some((sequence.parse().ok()?, sha256))
}

/// The attestor's pins. In a default build: the operator override when one
/// is set, else what ingest publishes. In a staging build: the signed
/// staging set and nothing else. Either is cached with a bounded TTL. See
/// the module docs for the precedence and the failure rule.
pub struct PinProvider {
    mode: Mode,
    cache: tokio::sync::Mutex<(Option<CachedPins>, Option<u64>)>,
    /// The last label reached, for `status`, which cannot await.
    status: Mutex<&'static str>,
    /// The last attestation outcome under these pins, as a label: the
    /// replay and compose-manager refusals among them (staging only).
    #[cfg(feature = "near-ai-staging")]
    attestation: Mutex<&'static str>,
}

impl std::fmt::Debug for PinProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PinProvider")
            .field("source", &self.kind())
            .finish_non_exhaustive()
    }
}

impl PinProvider {
    fn with_mode(mode: Mode) -> Self {
        let status = match &mode {
            Mode::Unpinned => PINS_NONE,
            #[cfg(any(test, not(feature = "near-ai-staging")))]
            Mode::Fixed(fixed) if fixed.sets.is_empty() => PINS_NONE,
            #[cfg(any(test, not(feature = "near-ai-staging")))]
            Mode::Fixed(_) => PINNED,
            #[allow(unreachable_patterns)]
            _ => PINS_NOT_FETCHED,
        };
        Self {
            mode,
            cache: tokio::sync::Mutex::new((None, None)),
            status: Mutex::new(status),
            #[cfg(feature = "near-ai-staging")]
            attestation: Mutex::new(ATTESTATION_NOT_CHECKED),
        }
    }

    /// `override_pins` wins when `Some`, even if empty. `source` is asked
    /// only without one.
    #[cfg(not(feature = "near-ai-staging"))]
    #[must_use]
    pub fn new(
        override_pins: Option<Vec<ExpectedMeasurements>>,
        source: Option<Box<dyn PinSource>>,
    ) -> Self {
        Self::with_mode(match (override_pins, source) {
            (Some(pins), _) => Mode::Fixed(FixedPins { sets: pins }),
            (None, Some(source)) => Mode::Published(source),
            (None, None) => Mode::Unpinned,
        })
    }

    /// The daemon's provider. `store` is the state directory, `None` for a
    /// daemon that must reach no network (tests); `env` reads an
    /// environment variable.
    ///
    /// A default build takes the operator override from `env` when it is
    /// set, else ingest's document through `store`. A staging build reads
    /// neither: it takes only the signed set from the compiled-in pin URL,
    /// checked against the compiled-in key, with its sequence recorded in
    /// `store` (spec 5.5).
    #[must_use]
    pub fn for_daemon(
        store: Option<crate::config::ConfigStore>,
        env: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        #[cfg(not(feature = "near-ai-staging"))]
        {
            Self::new(
                pins_from(env),
                store.map(|store| Box::new(IngestPinSource::new(store)) as Box<dyn PinSource>),
            )
        }
        #[cfg(feature = "near-ai-staging")]
        {
            // Deliberately unread: no variable overrides a signed set.
            let _ = env;
            Self::daemon_signed(store, crate::channel::STAGING_PIN_URL)
        }
    }

    /// The staging daemon's provider over `pin_url`, which is the
    /// compiled-in pin URL everywhere but in tests.
    #[cfg(feature = "near-ai-staging")]
    fn daemon_signed(store: Option<crate::config::ConfigStore>, pin_url: &str) -> Self {
        let Some(store) = store else {
            return Self::with_mode(Mode::Unpinned);
        };
        let sequence_file = store.dir().join(SEQUENCE_FILE);
        match StagingPinSource::new(pin_url, Some(store)) {
            Ok(source) => Self::signed_with(
                Box::new(source),
                crate::channel::STAGING_PIN_PUBLIC_KEY,
                sequence_file,
            ),
            Err(SourceUnavailable) => {
                let provider = Self::with_mode(Mode::Unpinned);
                provider.set_status(PINS_UNAVAILABLE);
                provider
            }
        }
    }

    #[cfg(feature = "near-ai-staging")]
    fn signed_with(
        source: Box<dyn SignedPinSource>,
        public_key: &[u8],
        sequence_file: std::path::PathBuf,
    ) -> Self {
        Self::with_mode(Mode::Signed(SignedPins {
            source,
            public_key: public_key.to_vec(),
            gateway_host: staging_gateway_host(),
            sequence_file,
        }))
    }

    /// A signed provider over a test's source and key, recording its
    /// sequence in `dir`.
    #[cfg(all(test, feature = "near-ai-staging"))]
    pub(crate) fn signed_for_test(
        source: Box<dyn SignedPinSource>,
        public_key: &[u8],
        dir: &std::path::Path,
    ) -> Self {
        Self::signed_with(source, public_key, dir.join(SEQUENCE_FILE))
    }

    /// A provider that always answers `sets`, with no allowlist: in a
    /// staging build it earns no key.
    #[cfg(test)]
    pub(crate) fn fixed(sets: Vec<ExpectedMeasurements>) -> Self {
        Self::with_mode(Mode::Fixed(FixedPins {
            sets,
            #[cfg(feature = "near-ai-staging")]
            allowlist: None,
        }))
    }

    /// A staging test's provider: `sets` and the allowlist signed with them.
    #[cfg(all(test, feature = "near-ai-staging"))]
    pub(crate) fn fixed_allowing(sets: Vec<ExpectedMeasurements>, allowlist: Allowlist) -> Self {
        Self::with_mode(Mode::Fixed(FixedPins {
            sets,
            allowlist: Some(Arc::new(allowlist)),
        }))
    }

    /// A provider that pins nothing.
    #[cfg(test)]
    pub(crate) fn unpinned() -> Self {
        Self::with_mode(Mode::Unpinned)
    }

    /// Which source this provider reads.
    #[cfg(test)]
    pub(crate) fn source_kind(&self) -> &'static str {
        self.kind()
    }

    fn kind(&self) -> &'static str {
        match &self.mode {
            Mode::Unpinned => "none",
            #[cfg(any(test, not(feature = "near-ai-staging")))]
            Mode::Fixed(_) => "fixed",
            #[cfg(not(feature = "near-ai-staging"))]
            Mode::Published(_) => "published",
            #[cfg(feature = "near-ai-staging")]
            Mode::Signed(_) => "signed",
        }
    }

    /// The last state reached, as a label: [`PINNED`], [`PINS_NOT_FETCHED`],
    /// [`PINS_UNAVAILABLE`], [`PINS_NONE`], or the refusal that pinned
    /// nothing. Never a key, host or measurement.
    pub fn status_label(&self) -> &'static str {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn set_status(&self, label: &'static str) {
        *self
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = label;
    }

    /// The last attestation outcome, as a label: [`ATTESTED`],
    /// [`ATTESTATION_NOT_CHECKED`], or the check that refused the last
    /// answer, such as `rtmr3_replay_mismatch` or
    /// `compose_action_not_allowed`. Never an event value, register, key or
    /// nonce. Staging builds only.
    #[cfg(feature = "near-ai-staging")]
    pub fn attestation_label(&self) -> &'static str {
        *self
            .attestation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(feature = "near-ai-staging")]
    fn set_attestation(&self, label: &'static str) {
        *self
            .attestation
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = label;
    }

    /// The pins in force at `now_unix`.
    pub async fn current(&self, now_unix: u64) -> CurrentPins {
        match &self.mode {
            Mode::Unpinned => CurrentPins::default(),
            #[cfg(any(test, not(feature = "near-ai-staging")))]
            Mode::Fixed(fixed) => CurrentPins {
                sets: fixed.sets.clone(),
                digest: format!("override:{}", digest_sets(&fixed.sets)),
                #[cfg(feature = "near-ai-staging")]
                allowlist: fixed.allowlist.clone(),
            },
            #[allow(unreachable_patterns)]
            _ => self.refreshed(now_unix).await,
        }
    }

    /// One fetch from the provider's source.
    async fn fetch(&self, now_unix: u64) -> Result<Fetched, SourceUnavailable> {
        let _ = now_unix;
        match &self.mode {
            #[cfg(not(feature = "near-ai-staging"))]
            Mode::Published(source) => source.fetch().await.map(|document| Fetched::Pins {
                pins: published_to_current(&document),
                valid_until: None,
            }),
            #[cfg(feature = "near-ai-staging")]
            Mode::Signed(signed) => signed.fetch(now_unix).await,
            #[allow(unreachable_patterns)]
            _ => Err(SourceUnavailable),
        }
    }

    /// The cached set, refreshed on its interval. A fetch that fails keeps
    /// the last good set until [`PIN_TTL`] after it was fetched and never
    /// past its own expiry; a document fetched and refused pins nothing at
    /// once, and stands as an empty set until the next refresh.
    async fn refreshed(&self, now_unix: u64) -> CurrentPins {
        let mut guard = self.cache.lock().await;
        let (cache, last_failure) = &mut *guard;
        if let Some(c) = cache.as_ref()
            && now_unix.saturating_sub(c.fetched_at) < PIN_REFRESH_AFTER.as_secs()
            && c.usable_at(now_unix)
        {
            return c.pins.clone();
        }
        let backing_off =
            last_failure.is_some_and(|at| now_unix.saturating_sub(at) < PIN_RETRY_FLOOR.as_secs());
        if !backing_off {
            match self.fetch(now_unix).await {
                Ok(Fetched::Pins { pins, valid_until }) => {
                    *last_failure = None;
                    *cache = Some(CachedPins {
                        pins: pins.clone(),
                        fetched_at: now_unix,
                        valid_until,
                    });
                    self.set_status(if pins.sets.is_empty() {
                        PINS_NONE
                    } else {
                        PINNED
                    });
                    return pins;
                }
                #[cfg(feature = "near-ai-staging")]
                Ok(Fetched::Refused(label)) => {
                    // The refusal is cached as an empty set: nothing older is
                    // ever served after it, and the document is asked for
                    // again on the usual refresh interval.
                    *last_failure = None;
                    *cache = Some(CachedPins {
                        pins: CurrentPins::default(),
                        fetched_at: now_unix,
                        valid_until: None,
                    });
                    tracing::warn!(
                        control = CONTROL,
                        label,
                        "NEAR AI pin set refused; no answer will be proof"
                    );
                    self.set_status(label);
                    return CurrentPins::default();
                }
                Err(SourceUnavailable) => *last_failure = Some(now_unix),
            }
        }
        // No fresh answer: the last good set stands only inside its TTL and
        // its own expiry.
        match cache {
            Some(c)
                if now_unix.saturating_sub(c.fetched_at) < PIN_TTL.as_secs()
                    && c.usable_at(now_unix) =>
            {
                c.pins.clone()
            }
            Some(c) => {
                let label = if c.usable_at(now_unix) {
                    PINS_UNAVAILABLE
                } else {
                    PINS_EXPIRED
                };
                *cache = None;
                self.set_status(label);
                CurrentPins::default()
            }
            None => {
                if !backing_off {
                    self.set_status(PINS_UNAVAILABLE);
                }
                CurrentPins::default()
            }
        }
    }
}

/// A published document as the pins it grants. Anything but a configured,
/// self-consistent document whose every set parses grants none.
#[cfg(not(feature = "near-ai-staging"))]
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

#[cfg(any(test, not(feature = "near-ai-staging")))]
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

/// The operator override, read through `env` (the process environment, in
/// the daemon). `None` when the variable is unset, which lets the published
/// pins apply. Set but empty or malformed is `Some(vec![])`: it wins and
/// pins nothing. Not compiled into a staging build.
#[cfg(not(feature = "near-ai-staging"))]
fn pins_from(env: &dyn Fn(&str) -> Option<String>) -> Option<Vec<ExpectedMeasurements>> {
    let raw = env(NEAR_AI_EXPECTED_MEASUREMENTS_ENV)?;
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
#[cfg(not(feature = "near-ai-staging"))]
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

    /// A refusal under `label`: no key. In a staging build the label is
    /// what the status reports; a default build reports none.
    fn refused(&self, label: &'static str) -> Attestation {
        #[cfg(feature = "near-ai-staging")]
        {
            self.inner.pins.set_attestation(label);
            tracing::debug!(control = CONTROL, label, "NEAR AI model key not earned");
        }
        let _ = label;
        Attestation::NotAttested
    }

    /// Earn the key set for `model` from scratch, under `pins`.
    async fn earn(&self, model: &str, now: u64, pins: &CurrentPins) -> Attestation {
        let inner = &self.inner;
        // A staging set without its allowlist pins nothing (S16): the sets
        // no longer pin RTMR3, and the allowlist is what stands in for it.
        #[cfg(feature = "near-ai-staging")]
        let Some(allowlist) = pins.allowlist.clone() else {
            return self.refused(PINS_NO_ALLOWLIST);
        };
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
                Err(AttestedKeyError::ModelNotAttested) => {
                    return self.refused(REPORT_NOT_ATTESTED);
                }
                Err(_) => return self.refused(REPORT_NOT_ATTESTED),
            };
        // Each entry's quote, and in a staging build its own log and
        // compose-manager record beside it, all from this one report.
        #[cfg(feature = "near-ai-staging")]
        let Ok(entries) = model_entry_evidence(&report, model) else {
            return self.refused(REPORT_NOT_ATTESTED);
        };
        #[cfg(not(feature = "near-ai-staging"))]
        let Ok(entries) = model_entry_quotes(&report, model) else {
            return self.refused(REPORT_NOT_ATTESTED);
        };
        let mut verified = Vec::with_capacity(entries.len());
        for entry in &entries {
            #[cfg(feature = "near-ai-staging")]
            let quote = &entry.quote;
            #[cfg(not(feature = "near-ai-staging"))]
            let quote = entry;
            let Ok(collateral) = inner.source.collateral(quote).await else {
                return Attestation::Unavailable;
            };
            let Some(checked) = (inner.verify)(quote, &collateral, now) else {
                return self.refused(QUOTE_UNVERIFIED);
            };
            // A genuine quote from a platform behind on Intel's fixes earns
            // nothing, exactly as the server's drill refuses it.
            if checked.tcb_status != REQUIRED_TCB_STATUS {
                return self.refused(QUOTE_TCB_NOT_UP_TO_DATE);
            }
            if !image_is_pinned(&pins.sets, &checked) {
                return self.refused(QUOTE_NOT_PINNED);
            }
            #[cfg(feature = "near-ai-staging")]
            match self
                .staging_checks(entry, &checked, &nonce, now, &allowlist)
                .await
            {
                Ok(()) => {}
                Err(StagingRefusal::Unavailable) => return Attestation::Unavailable,
                Err(StagingRefusal::Refused(label)) => return self.refused(label),
            }
            verified.push(checked);
        }
        let report_data: Vec<&[u8]> = verified.iter().map(|q| q.report_data.as_slice()).collect();
        match quote_bound_keys(&claimed, &report_data, &nonce) {
            Ok(keys) => {
                #[cfg(feature = "near-ai-staging")]
                inner.pins.set_attestation(ATTESTED);
                Attestation::Keys(keys)
            }
            Err(_) => self.refused(QUOTE_KEYS_UNBOUND),
        }
    }

    /// Spec 5.5, steps 6 and 7, for one model entry whose quote verified
    /// and matched a set: replay its own log against that quote and check
    /// the events against the allowlist, then check its own compose-manager
    /// record, whose quote goes through the same verifier with collateral
    /// fetched for it.
    #[cfg(feature = "near-ai-staging")]
    async fn staging_checks(
        &self,
        entry: &EntryEvidence,
        checked: &VerifiedQuote,
        nonce: &str,
        now: u64,
        allowlist: &Allowlist,
    ) -> Result<(), StagingRefusal> {
        let refused = |refusal: trace_commons_attestation::staging_replay::ReplayRefusal| {
            StagingRefusal::Refused(refusal.label())
        };
        let events = replay_rtmr3(entry.event_log.as_ref(), checked).map_err(refused)?;
        check_rtmr3_allowlist(&events, &allowlist.rtmr3).map_err(refused)?;
        let record = ComposeManagerRecord::from_entry(&entry.entry).map_err(refused)?;
        let collateral = self
            .inner
            .source
            .collateral(&record.quote)
            .await
            .map_err(|SourceUnavailable| StagingRefusal::Unavailable)?;
        check_compose_manager(
            &record,
            |quote| (self.inner.verify)(quote, &collateral, now),
            checked,
            nonce,
            Some(&allowlist.compose_actions),
        )
        .map_err(refused)?;
        Ok(())
    }
}

/// Why the staging checks earned nothing.
#[cfg(feature = "near-ai-staging")]
enum StagingRefusal {
    /// Collateral for the compose-manager quote could not be fetched: not a
    /// verdict, as for the model quote's.
    Unavailable,
    /// Refused under this label.
    Refused(&'static str),
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

    // -- the staged fixture ------------------------------------------------
    //
    // A staging build earns a key only when the model entry's RTMR3 log
    // replays to its verified quote and its compose-manager record checks
    // (S16). No log replays to the capture's real RTMR3, so every entry here
    // carries a synthetic log and record, and the verifier stages the
    // registers they describe on the real, verified quote: `verify_staged`
    // sets RTMR3 to the log's replay and MRCONFIGID to config-id v1 over the
    // synthetic compose hash, and `with_compose` answers the compose-manager
    // quote from the model quote it follows, as one TD's would. A default
    // build reads neither the log nor the record, and its tests run over the
    // same staged registers, so both builds test one fixture.

    pub(super) const APP_ID: &str = "a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1a1";
    pub(super) const COMPOSE_HASH: &str =
        "c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0c0";
    pub(super) const INSTANCE_ID: &str = "1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d1d";
    pub(super) const MR_KMS: &str = "3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b3b";
    pub(super) const OS_IMAGE_HASH: &str =
        "0505050505050505050505050505050505050505050505050505050505050505";
    pub(super) const KEY_PROVIDER: &str =
        "7b226e616d65223a226b6d73222c226964223a2273796e746865746963227d";
    /// Synthetic compose-action values, in the live record's formats.
    pub(super) const MANAGER_IMAGE: &str = "registry.example/compose-manager@sha256:abababababababababababababababababababababababababababababababab";
    pub(super) const COMPOSE_FILE: &str = "example/models.yaml";
    pub(super) const FILE_SHA256: &str =
        "f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1f1";
    pub(super) const COMMIT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
    const RTMR3_EVENT_TYPE: u32 = 0x0800_0001;
    const RTMR3_EVENTS: [&str; 10] = [
        "system-preparing",
        "app-id",
        "compose-hash",
        "instance-id",
        "boot-mr-done",
        "mr-kms",
        "os-image-hash",
        "key-provider",
        "storage-fs",
        "system-ready",
    ];

    fn marker_payload(name: &str) -> String {
        hex::encode(format!("synthetic {name}"))
    }

    /// The ten RTMR3 events, in order, with synthetic payloads.
    pub(super) fn ten_events() -> Vec<(String, String)> {
        RTMR3_EVENTS
            .iter()
            .map(|name| {
                let payload = match *name {
                    "app-id" => APP_ID.to_string(),
                    "compose-hash" => COMPOSE_HASH.to_string(),
                    "instance-id" => INSTANCE_ID.to_string(),
                    "mr-kms" => MR_KMS.to_string(),
                    "os-image-hash" => OS_IMAGE_HASH.to_string(),
                    "key-provider" => KEY_PROVIDER.to_string(),
                    marker => marker_payload(marker),
                };
                (name.to_string(), payload)
            })
            .collect()
    }

    pub(super) fn event_log(events: &[(String, String)]) -> serde_json::Value {
        let mut log = vec![serde_json::json!({
            "imr": 0, "event_type": 1, "digest": "ee".repeat(48), "event": "", "event_payload": ""
        })];
        for (name, payload) in events {
            log.push(serde_json::json!({
                "imr": 3, "event_type": RTMR3_EVENT_TYPE, "digest": "",
                "event": name, "event_payload": payload,
            }));
        }
        serde_json::Value::Array(log)
    }

    /// RTMR3 as the replay rule computes it: SHA-384 chained from zero over
    /// SHA-384(type LE32 || ":" || name || ":" || payload). Written out here
    /// so the default build's tests need no staging code.
    pub(super) fn replayed(events: &[(String, String)]) -> String {
        use sha2::{Digest, Sha384};
        let mut register = [0u8; 48];
        for (name, payload) in events {
            let mut digest = Sha384::new();
            digest.update(RTMR3_EVENT_TYPE.to_le_bytes());
            digest.update(b":");
            digest.update(name.as_bytes());
            digest.update(b":");
            digest.update(hex::decode(payload).unwrap());
            let digest = digest.finalize();
            let mut extend = Sha384::new();
            extend.update(register);
            extend.update(digest);
            register = extend.finalize().into();
        }
        hex::encode(register)
    }

    /// One action of each kind, structurally exact to the live record
    /// (string timestamps, a digest-pinned compose-manager image, a compose
    /// file named by path, commit and SHA-256); every value synthetic.
    pub(super) fn actions() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({"action": "compose_manager_started", "container": "compose-manager", "image": MANAGER_IMAGE, "timestamp": "2026-01-01T00:00:00Z"}),
            serde_json::json!({"action": "compose_up", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "services": ["model-a", "proxy"], "timestamp": "2026-01-01T00:00:01Z"}),
            serde_json::json!({"action": "compose_down", "tag": "v0.0.1", "commit": COMMIT, "file": COMPOSE_FILE, "services": ["proxy"], "timestamp": "2026-01-01T00:00:02Z"}),
        ]
    }

    /// SHA-256 of `actions` as sorted-key compact JSON. The fixture's
    /// actions are ASCII strings and lists, where `serde_json`'s compact
    /// form of a sorted copy is Python's.
    pub(super) fn actions_hash(actions: &[serde_json::Value]) -> String {
        use sha2::{Digest, Sha256};
        fn sorted(value: &serde_json::Value) -> serde_json::Value {
            match value {
                serde_json::Value::Object(map) => {
                    let ordered: std::collections::BTreeMap<_, _> =
                        map.iter().map(|(k, v)| (k.clone(), sorted(v))).collect();
                    let mut out = serde_json::Map::new();
                    for (k, v) in ordered {
                        out.insert(k, v);
                    }
                    serde_json::Value::Object(out)
                }
                serde_json::Value::Array(items) => {
                    serde_json::Value::Array(items.iter().map(sorted).collect())
                }
                other => other.clone(),
            }
        }
        let text =
            serde_json::to_string(&sorted(&serde_json::Value::Array(actions.to_vec()))).unwrap();
        hex::encode(Sha256::digest(text.as_bytes()))
    }

    pub(super) fn fixture_quote() -> Vec<u8> {
        hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap()
    }

    /// The compose-manager quote: the capture's with one byte appended, so
    /// `with_compose` can tell it from the model quote by its bytes.
    pub(super) fn compose_quote() -> Vec<u8> {
        let mut quote = fixture_quote();
        quote.push(0);
        quote
    }

    /// The registers the synthetic log and compose hash describe, on a
    /// verified quote.
    pub(super) fn stage(quote: &mut VerifiedQuote) {
        quote.rtmr[3] = replayed(&ten_events());
        quote.mr_config_id = format!("01{COMPOSE_HASH}{}", "00".repeat(15));
    }

    /// `verify_quote`, staged when the quote is the capture's.
    pub(super) fn verify_staged(
        quote: &[u8],
        collateral: &Collateral,
        now: u64,
    ) -> Option<VerifiedQuote> {
        let mut verified = verify_quote(quote, collateral, now).ok()?;
        if quote == fixture_quote().as_slice() {
            stage(&mut verified);
        }
        Some(verified)
    }

    /// `inner` for every quote but the compose-manager one, which is the
    /// capture verified for real, with the registers of the model quote
    /// `inner` verified last, our nonce at `[32..64]` and the fixture's
    /// actions hash at `[0..32]`.
    pub(super) fn with_compose(
        inner: impl Fn(&[u8], &Collateral, u64) -> Option<VerifiedQuote> + Send + Sync + 'static,
    ) -> QuoteVerifier {
        with_compose_over(actions(), |_| {}, inner)
    }

    /// [`with_compose`] for a record over `record_actions`, with `alter`
    /// applied to the compose-manager quote it answers.
    pub(super) fn with_compose_over(
        record_actions: Vec<serde_json::Value>,
        alter: impl Fn(&mut VerifiedQuote) + Send + Sync + 'static,
        inner: impl Fn(&[u8], &Collateral, u64) -> Option<VerifiedQuote> + Send + Sync + 'static,
    ) -> QuoteVerifier {
        let last: Mutex<Option<VerifiedQuote>> = Mutex::new(None);
        Box::new(move |quote, collateral, now| {
            if quote == compose_quote().as_slice() {
                let real = fixture_quote();
                let mut verified = verify_quote(&real, collateral, now).ok()?;
                let model = last.lock().unwrap().clone()?;
                verified.mrtd = model.mrtd;
                verified.mr_config_id = model.mr_config_id;
                verified.rtmr = model.rtmr;
                verified.report_data[0..32]
                    .copy_from_slice(&hex::decode(actions_hash(&record_actions)).unwrap());
                verified.report_data[32..64].copy_from_slice(&real[600..632]);
                alter(&mut verified);
                return Some(verified);
            }
            let verified = inner(quote, collateral, now)?;
            *last.lock().unwrap() = Some(verified.clone());
            Some(verified)
        })
    }

    /// The allowlist the staged fixture passes, as a signed payload carries
    /// it.
    #[cfg(feature = "near-ai-staging")]
    pub(super) fn fixture_allowlist_json() -> serde_json::Value {
        let markers: Vec<serde_json::Value> = [
            "system-preparing",
            "boot-mr-done",
            "storage-fs",
            "system-ready",
        ]
        .iter()
        .map(|name| serde_json::json!({"name": name, "payload": marker_payload(name)}))
        .collect();
        serde_json::json!({
            "rtmr3": {
                "app_id": APP_ID,
                "mr_kms": MR_KMS,
                "os_image_hash": OS_IMAGE_HASH,
                "key_provider": KEY_PROVIDER,
                "markers": markers,
            },
            "compose_actions": {
                "compose_manager_images": [MANAGER_IMAGE],
                "compose_files": [
                    {"file": COMPOSE_FILE, "file_sha256": FILE_SHA256, "commit": COMMIT},
                ],
            },
        })
    }

    /// A fixed provider over `sets`: in a staging build, with the fixture's
    /// allowlist, which a staging key needs as much as the sets.
    pub(super) fn fixed_pins(sets: Vec<ExpectedMeasurements>) -> PinProvider {
        #[cfg(feature = "near-ai-staging")]
        {
            PinProvider::fixed_allowing(
                sets,
                trace_commons_attestation::staging_replay::Allowlist::from_json(
                    &fixture_allowlist_json(),
                )
                .unwrap(),
            )
        }
        #[cfg(not(feature = "near-ai-staging"))]
        {
            PinProvider::fixed(sets)
        }
    }

    /// A model entry over `quote_hex`, carrying the synthetic log and a
    /// compose-manager record bound to the fixture's actions.
    pub(super) fn entry_json(
        quote_hex: &str,
        key: &str,
        nonce: &str,
        model: &str,
    ) -> serde_json::Value {
        serde_json::json!({
            "model_name": model,
            "signing_algo": "ed25519",
            "signing_address": key,
            "request_nonce": nonce,
            "intel_quote": quote_hex,
            "event_log": event_log(&ten_events()),
            "compose_manager_attestation": {
                "actions": actions(),
                "actions_hash": actions_hash(&actions()),
                // A JSON-encoded string, as the live record carries it.
                "event_log": event_log(&ten_events()).to_string(),
                "nonce": nonce,
                "nonce_source": "client",
                "quote": hex::encode(compose_quote()),
                "report_data": "00",
                "vm_config": "{}",
            },
        })
    }

    /// A real, Intel-verifiable quote (the ECDSA capture's) framed as an
    /// ed25519 model attestation. The key and nonce are read out of that
    /// quote's own `report_data`, so the report is consistent by
    /// construction rather than by a hand-written expectation -- the same
    /// construction the server's signer-resolver tests use. The entry
    /// carries the staged log and compose-manager record.
    fn synthetic_report() -> (String, String, String) {
        let quote_hex = json(ECDSA_REPORT)["intel_quote"]
            .as_str()
            .unwrap()
            .to_string();
        let quote = hex::decode(&quote_hex).unwrap();
        let report_data = &quote[568..632];
        let key = hex::encode(&report_data[0..32]);
        let nonce = hex::encode(&report_data[32..64]);
        let entry = entry_json(&quote_hex, &key, &nonce, MODEL);
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

    /// Pins taken from the quote's own verified (and staged) registers, by
    /// running the real verification path rather than pasting hex. They name
    /// the S16 registers and RTMR3 too, so one set serves both builds.
    fn real_pins() -> Vec<ExpectedMeasurements> {
        let verified = verify_staged(
            &fixture_quote(),
            &parse_collateral(COLLATERAL).unwrap(),
            FIXTURE_CAPTURED_AT,
        )
        .expect("the capture verifies against its collateral");
        let raw = format!(
            "mrtd={},mrconfigid={},rtmr0={},rtmr1={},rtmr2={},rtmr3={}",
            verified.mrtd,
            verified.mr_config_id,
            verified.rtmr[0],
            verified.rtmr[1],
            verified.rtmr[2],
            verified.rtmr[3]
        );
        vec![one_set(&raw)]
    }

    fn one_set(raw: &str) -> ExpectedMeasurements {
        ExpectedMeasurements::from_env_value(Some(raw))
            .expect("valid pins")
            .expect("a set")
    }

    fn other_pins() -> Vec<ExpectedMeasurements> {
        vec![one_set(&format!("mrtd={}", "00".repeat(48)))]
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
            Arc::new(fixed_pins(pins)),
            Box::new(move || clock.load(Ordering::SeqCst)),
            Box::new(move || Some(nonce.clone())),
            with_compose(verify_staged),
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
        let mut gateway = entry_json(&quote_hex, &key, &nonce, MODEL);
        gateway.as_object_mut().unwrap().remove("model_name");
        gateway["report_data"] = serde_json::json!(format!("{key}{nonce}"));
        let other_model = entry_json(&quote_hex, &key, &nonce, "Qwen/Qwen3.8-27B");
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
        let verified = verify_staged(
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
        let entry = |quote: &str| entry_json(quote, &key, &nonce, MODEL);
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
                Arc::new(fixed_pins(real_pins())),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                with_compose(move |quote: &[u8], collateral: &Collateral, now: u64| {
                    let mut verified = verify_staged(quote, collateral, now)?;
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
                Arc::new(fixed_pins(real_pins())),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                with_compose(move |quote: &[u8], collateral: &Collateral, now: u64| {
                    let mut verified = verify_staged(quote, collateral, now)?;
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
            Arc::new(fixed_pins(real_pins())),
            Box::new(|| FIXTURE_CAPTURED_AT),
            Box::new(move || Some(nonce.clone())),
            with_compose(verify_staged),
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
            Arc::new(fixed_pins(pins)),
            Box::new(|| FIXTURE_CAPTURED_AT),
            Box::new(move || Some(nonce.clone())),
            with_compose(verify_staged),
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

    /// A one-register set, for the published pins' tests (a staging build
    /// has its own, in `signed`).
    #[cfg(not(feature = "near-ai-staging"))]
    fn pin_string(byte: &str) -> String {
        format!("mrconfigid={}", byte.repeat(48))
    }

    const T0: u64 = 1_800_000_000;

    /// The unsigned pins ingest publishes and the operator's environment
    /// override: the production sources, which a staging build does not
    /// compile.
    #[cfg(not(feature = "near-ai-staging"))]
    mod published {
        use super::*;

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
                let provider =
                    Arc::new(PinProvider::new(None, Some(Box::new(Arc::clone(&source)))));
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
                    with_compose(verify_staged),
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

        use trace_commons_protocol::near_ai_measurements::{NearAiMeasurementPins, PinState};

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
                with_compose(verify_staged),
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
                with_compose(verify_staged),
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
    }

    /// The image check itself: an empty pin list admits no image. The
    /// mutation "an empty set allows everything" must turn this red.
    #[test]
    fn an_empty_pin_list_pins_no_image() {
        let quote = hex::decode(json(ECDSA_REPORT)["intel_quote"].as_str().unwrap()).unwrap();
        let verified = verify_staged(
            &quote,
            &parse_collateral(COLLATERAL).unwrap(),
            FIXTURE_CAPTURED_AT,
        )
        .expect("the capture verifies");
        assert!(!image_is_pinned(&[], &verified));
        assert!(image_is_pinned(&real_pins(), &verified));
        assert!(!image_is_pinned(&other_pins(), &verified));
    }

    /// The daemon's constructor in a default build: the operator override
    /// when the variable is set, else ingest's published document. Neither
    /// is replaced by anything signed.
    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn a_default_build_still_uses_the_ingest_document() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let unset = |_: &str| None;
        let provider = PinProvider::for_daemon(Some(store.clone()), &unset);
        assert_eq!(provider.source_kind(), "published");
        let set =
            |name: &str| (name == NEAR_AI_EXPECTED_MEASUREMENTS_ENV).then(|| pin_string("bb"));
        let overridden = PinProvider::for_daemon(Some(store), &set);
        assert_eq!(overridden.source_kind(), "fixed");
    }

    /// A default build carries nothing of the staging verifier: every
    /// `STAGING_PIN_*` value in the crate's source is defined only under the
    /// feature, so none of them is in this build at all.
    #[cfg(not(feature = "near-ai-staging"))]
    #[test]
    fn a_default_build_has_no_staging_pin_key() {
        fn rust_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    rust_files(&path, out);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    out.push(path);
                }
            }
        }
        let mut files = Vec::new();
        rust_files(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut definitions = 0;
        for file in files {
            let source = std::fs::read_to_string(&file).unwrap();
            let lines: Vec<&str> = source.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let trimmed = line.trim_start();
                let trimmed = trimmed
                    .strip_prefix("pub(crate) ")
                    .or_else(|| trimmed.strip_prefix("pub "))
                    .unwrap_or(trimmed);
                if !(trimmed.starts_with("const STAGING_PIN_")
                    || trimmed.starts_with("static STAGING_PIN_"))
                {
                    continue;
                }
                definitions += 1;
                // The nearest attribute above the definition, past its doc
                // comment, must be the feature gate.
                let gate = lines[..i]
                    .iter()
                    .rev()
                    .map(|l| l.trim())
                    .find(|l| !l.starts_with("///"))
                    .unwrap_or("");
                assert_eq!(
                    gate,
                    "#[cfg(feature = \"near-ai-staging\")]",
                    "{}:{}: a staging pin value outside the feature",
                    file.display(),
                    i + 1
                );
            }
        }
        assert_eq!(definitions, 2, "the staging pin key and URL are defined");
    }

    /// Signed pins, the only source a staging build has.
    #[cfg(feature = "near-ai-staging")]
    mod signed {
        use super::*;
        use base64::Engine as _;
        use ring::signature::{Ed25519KeyPair, KeyPair};
        use trace_commons_attestation::staging_pins::{self, PinRefusal};

        fn utc(t: u64) -> String {
            chrono::DateTime::from_timestamp(t as i64, 0)
                .unwrap()
                .format("%Y-%m-%dT%H:%M:%SZ")
                .to_string()
        }

        fn b64(bytes: &[u8]) -> String {
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
        }

        /// A v2 set: every S16 register, all `byte`. Shadows the outer
        /// helper, whose one-register set a v2 document refuses.
        fn pin_string(byte: &str) -> String {
            let v = byte.repeat(48);
            format!("mrtd={v},mrconfigid={v},rtmr0={v},rtmr1={v},rtmr2={v}")
        }

        /// A key pair made for the test, and documents it signs.
        struct Signer(Ed25519KeyPair);

        impl Signer {
            fn new() -> Self {
                let rng = ring::rand::SystemRandom::new();
                let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).unwrap();
                Self(Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap())
            }

            fn public(&self) -> Vec<u8> {
                self.0.public_key().as_ref().to_vec()
            }

            /// A v2 payload for this build, carrying the fixture's
            /// allowlist, with `changes` laid over it.
            fn payload(
                sequence: u64,
                issued_at: u64,
                expires_at: u64,
                sets: &[String],
                changes: serde_json::Value,
            ) -> Vec<u8> {
                let mut payload = serde_json::json!({
                    "format": staging_pins::FORMAT,
                    "channel": staging_pins::CHANNEL,
                    "gateway_host": staging_gateway_host().unwrap(),
                    "sequence": sequence,
                    "issued_at": utc(issued_at),
                    "expires_at": utc(expires_at),
                    "sets": sets,
                    "allowlist": fixture_allowlist_json(),
                });
                for (k, v) in changes.as_object().unwrap() {
                    payload[k] = v.clone();
                }
                serde_json::to_vec(&payload).unwrap()
            }

            /// `payload` signed by this key, under `key_id`.
            fn wrap(&self, payload: &[u8], key_id: &str) -> Vec<u8> {
                let signature = self.0.sign(&staging_pins::signing_input(payload));
                serde_json::to_vec(&serde_json::json!({
                    "payload": b64(payload),
                    "signature": b64(signature.as_ref()),
                    "key_id": key_id,
                }))
                .unwrap()
            }

            fn sign(&self, payload: &[u8]) -> Vec<u8> {
                self.wrap(payload, &staging_pins::key_id(&self.public()))
            }

            /// `payload` signed under the v1 prefix, as a v1 tool did.
            fn sign_v1(&self, payload: &[u8]) -> Vec<u8> {
                let mut input = staging_pins::SIGNING_PREFIX_V1.to_vec();
                input.extend_from_slice(payload);
                let signature = self.0.sign(&input);
                serde_json::to_vec(&serde_json::json!({
                    "payload": b64(payload),
                    "signature": b64(signature.as_ref()),
                    "key_id": staging_pins::key_id(&self.public()),
                }))
                .unwrap()
            }

            /// A well-formed current document: `sequence`, issued at `at`,
            /// living a week.
            fn document(&self, sequence: u64, at: u64, sets: &[String]) -> Vec<u8> {
                self.sign(&Self::payload(
                    sequence,
                    at,
                    at + 7 * 86_400,
                    sets,
                    serde_json::json!({}),
                ))
            }
        }

        /// A signed-document source whose answer the test changes.
        struct Served {
            answer: Mutex<Result<Vec<u8>, SourceUnavailable>>,
            calls: AtomicUsize,
        }

        impl Served {
            fn serving(document: Vec<u8>) -> Arc<Self> {
                Arc::new(Self {
                    answer: Mutex::new(Ok(document)),
                    calls: AtomicUsize::new(0),
                })
            }
            fn set(&self, answer: Result<Vec<u8>, SourceUnavailable>) {
                *self.answer.lock().unwrap() = answer;
            }
            fn calls(&self) -> usize {
                self.calls.load(Ordering::SeqCst)
            }
        }

        #[async_trait::async_trait]
        impl SignedPinSource for Arc<Served> {
            async fn fetch(&self) -> Result<Vec<u8>, SourceUnavailable> {
                self.calls.fetch_add(1, Ordering::SeqCst);
                self.answer.lock().unwrap().clone()
            }
        }

        fn provider(source: &Arc<Served>, signer: &Signer, dir: &std::path::Path) -> PinProvider {
            PinProvider::signed_with(
                Box::new(Arc::clone(source)),
                &signer.public(),
                dir.join(SEQUENCE_FILE),
            )
        }

        fn sets(bytes: &[&str]) -> Vec<String> {
            bytes.iter().map(|b| pin_string(b)).collect()
        }

        fn real_sets() -> Vec<String> {
            real_pins()
                .iter()
                .map(ExpectedMeasurements::to_pin_string)
                .collect()
        }

        /// An attestor over the fixture report, asking `provider`.
        fn attestor_over(provider: Arc<PinProvider>, at: u64) -> (NearAiQuoteAttestor, String) {
            let (report, nonce, key) = synthetic_report();
            let attestor = NearAiQuoteAttestor::with_parts(
                Box::new(Fixture {
                    report,
                    reports: Arc::new(AtomicUsize::new(0)),
                    collateral_ok: true,
                }),
                provider,
                Box::new(move || at),
                Box::new(move || Some(nonce.clone())),
                with_compose(verify_staged),
            );
            (attestor, key)
        }

        /// A signed, current set is pinned, earns the fixture's key, is
        /// recorded as the highest sequence, and the identical document
        /// fetched again on the next refresh is still pinned.
        #[tokio::test]
        async fn a_staging_build_pins_a_signed_set() {
            let dir = tempfile::tempdir().unwrap();
            let signer = Signer::new();
            let at = FIXTURE_CAPTURED_AT;
            let source = Served::serving(signer.document(7, at - 60, &real_sets()));
            let provider = Arc::new(provider(&source, &signer, dir.path()));
            assert_eq!(provider.status_label(), "pins_not_fetched");

            let current = provider.current(at).await;
            assert_eq!(current.sets, real_pins());
            assert_eq!(provider.status_label(), "pinned");
            let recorded = std::fs::read_to_string(dir.path().join(SEQUENCE_FILE)).unwrap();
            assert!(recorded.starts_with("7 "), "{recorded}");

            let (attestor, key) = attestor_over(Arc::clone(&provider), at);
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Keys(vec![key])
            );

            // The same bytes again: equal sequence, equal payload, accepted.
            let again = provider.current(at + PIN_REFRESH_AFTER.as_secs()).await;
            assert_eq!(source.calls(), 2, "refreshed");
            assert_eq!(again.sets, real_pins());
            assert_eq!(provider.status_label(), "pinned");
        }

        /// A local stand-in for both the staging pin URL and ingest, which
        /// records every path it is asked for. It serves a valid unsigned
        /// ingest document and answers 404 for the signed one.
        async fn recording_server() -> (
            String,
            Arc<Mutex<Vec<String>>>,
            tokio::sync::oneshot::Sender<()>,
        ) {
            let seen: Arc<Mutex<Vec<String>>> = Arc::default();
            let unsigned = serde_json::json!({
                "state": "configured",
                "sets": [pin_string("aa")],
            })
            .to_string();
            let app = axum::Router::new().fallback({
                let seen = Arc::clone(&seen);
                move |uri: axum::http::Uri| {
                    let seen = Arc::clone(&seen);
                    let unsigned = unsigned.clone();
                    async move {
                        let path = uri.path().to_string();
                        seen.lock().unwrap().push(path.clone());
                        if path.ends_with("/near-ai-measurements") {
                            (axum::http::StatusCode::OK, unsigned)
                        } else {
                            (axum::http::StatusCode::NOT_FOUND, String::new())
                        }
                    }
                }
            });
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
            (base, seen, tx)
        }

        /// A pin server answering past the document bound is a failed
        /// fetch, not a buffered body; a document inside it is read whole.
        #[tokio::test]
        async fn an_oversized_pin_document_is_refused() {
            let app = axum::Router::new()
                .route(
                    "/big",
                    axum::routing::get(|| async { " ".repeat(MAX_PIN_DOCUMENT_BYTES + 1) }),
                )
                .route("/small", axum::routing::get(|| async { "{}" }));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let _ = axum::serve(listener, app).await;
            });
            let big = StagingPinSource::new(&format!("{base}/big"), None).unwrap();
            assert_eq!(big.fetch().await, Err(SourceUnavailable));
            let small = StagingPinSource::new(&format!("{base}/small"), None).unwrap();
            assert_eq!(small.fetch().await, Ok(b"{}".to_vec()));
        }

        /// An enforcing allowlist that leaves out the pin host refuses the
        /// fetch, and so does a config that cannot be loaded: an operator's
        /// narrowed egress is never skipped because the config is unreadable.
        /// Neither reaches the server.
        #[tokio::test]
        async fn the_pin_fetch_keeps_to_the_allowlist() {
            let (base, seen, _stop) = recording_server().await;
            let url = format!("{base}/near-ai-pins/current.json");

            let narrowed = tempfile::tempdir().unwrap();
            let store = crate::config::ConfigStore::open(narrowed.path().to_path_buf()).unwrap();
            let config = serde_json::json!({
                "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
                "issuer_url": "https://issuer.example",
                "ingest_url": "https://ingest.example",
                "audience": "trace-commons-upload",
                "tenant_id": "tenant-abc",
                "instance_id": "instance-1",
                "user_subject": "user-1",
                "device_key_id": "sha256:00",
                "consent_scopes": ["debugging_evaluation"],
                "allowed_hosts": "issuer.example,ingest.example",
            });
            std::fs::write(
                narrowed.path().join(crate::config::CONFIG_FILE),
                serde_json::to_vec(&config).unwrap(),
            )
            .unwrap();
            let loaded = store.load_config().unwrap().expect("the config loads");
            assert!(crate::config::allowlist_for(loaded.allowed_hosts.as_deref()).is_enforcing());
            let source = StagingPinSource::new(&url, Some(store)).unwrap();
            assert_eq!(source.fetch().await, Err(SourceUnavailable));

            let broken = tempfile::tempdir().unwrap();
            let store = crate::config::ConfigStore::open(broken.path().to_path_buf()).unwrap();
            std::fs::write(broken.path().join(crate::config::CONFIG_FILE), b"not json").unwrap();
            let source = StagingPinSource::new(&url, Some(store)).unwrap();
            assert_eq!(source.fetch().await, Err(SourceUnavailable));

            assert!(
                seen.lock().unwrap().is_empty(),
                "{:?}",
                seen.lock().unwrap()
            );
        }

        /// The daemon's provider in a staging build asks the signed pin URL
        /// and nothing else: ingest's unsigned document is never fetched,
        /// and with the signed one missing nothing is pinned.
        #[tokio::test]
        async fn a_staging_build_ignores_the_ingest_document() {
            let dir = tempfile::tempdir().unwrap();
            let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
            let unset = |_: &str| None;
            assert_eq!(
                PinProvider::for_daemon(Some(store.clone()), &unset).source_kind(),
                "signed"
            );
            let (base, seen, _stop) = recording_server().await;
            let provider = PinProvider::daemon_signed(
                Some(store),
                &format!("{base}/near-ai-pins/current.json"),
            );
            assert!(provider.current(T0).await.sets.is_empty());
            assert_eq!(provider.status_label(), "pins_unavailable");
            assert_eq!(
                *seen.lock().unwrap(),
                vec!["/near-ai-pins/current.json".to_string()]
            );
        }

        /// A staging build never reads the measurement override, whatever
        /// the environment says.
        #[tokio::test]
        async fn a_staging_build_ignores_the_measurement_env_override() {
            let dir = tempfile::tempdir().unwrap();
            let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
            let asked = AtomicUsize::new(0);
            let everything = |_: &str| {
                asked.fetch_add(1, Ordering::SeqCst);
                Some(pin_string("aa"))
            };
            let provider = PinProvider::for_daemon(Some(store), &everything);
            assert_eq!(provider.source_kind(), "signed");
            assert_eq!(asked.load(Ordering::SeqCst), 0, "the environment was read");
            let unpinned = PinProvider::for_daemon(None, &everything);
            assert!(unpinned.current(T0).await.sets.is_empty());
            assert_eq!(asked.load(Ordering::SeqCst), 0, "the environment was read");
        }

        /// Once sequence 5 is accepted, sequence 4 is refused -- by this
        /// daemon and by the next one on the same state directory.
        #[tokio::test]
        async fn a_lower_sequence_is_refused_after_a_higher_one() {
            let dir = tempfile::tempdir().unwrap();
            let signer = Signer::new();
            let source = Served::serving(signer.document(5, T0, &sets(&["aa"])));
            let first = provider(&source, &signer, dir.path());
            assert_eq!(first.current(T0).await.sets.len(), 1);

            source.set(Ok(signer.document(4, T0, &sets(&["bb"]))));
            let later = T0 + PIN_REFRESH_AFTER.as_secs();
            assert!(
                first.current(later).await.sets.is_empty(),
                "no fallback to 5"
            );
            assert_eq!(first.status_label(), "pins_rollback");
            assert_no_fallback_after_refusal(&first, &source, later, "pins_rollback").await;

            let restarted = provider(&source, &signer, dir.path());
            assert!(restarted.current(later).await.sets.is_empty());
            assert_eq!(restarted.status_label(), "pins_rollback");

            // And a higher one is taken.
            source.set(Ok(signer.document(6, T0, &sets(&["cc"]))));
            let next = later + PIN_REFRESH_AFTER.as_secs();
            assert_eq!(
                restarted.current(next).await.sets[0].to_pin_string(),
                pin_string("cc")
            );
        }

        /// The same sequence carrying other bytes is refused, and the bytes
        /// first accepted under it are not served in its place.
        #[tokio::test]
        async fn the_same_sequence_with_other_bytes_is_refused() {
            let dir = tempfile::tempdir().unwrap();
            let signer = Signer::new();
            let source = Served::serving(signer.document(5, T0, &sets(&["aa"])));
            let provider = provider(&source, &signer, dir.path());
            assert_eq!(provider.current(T0).await.sets.len(), 1);

            source.set(Ok(signer.document(5, T0, &sets(&["bb"]))));
            let later = T0 + PIN_REFRESH_AFTER.as_secs();
            assert!(provider.current(later).await.sets.is_empty());
            assert_eq!(provider.status_label(), "pins_sequence_conflict");
            assert_no_fallback_after_refusal(&provider, &source, later, "pins_sequence_conflict")
                .await;
        }

        /// After a document refused at `refused_at`, while a good set from
        /// before is still inside `PIN_TTL`: the good set is never served
        /// again, inside the retry floor or past it, and the refused
        /// document is asked for again only on the refresh interval, not
        /// every retry floor.
        async fn assert_no_fallback_after_refusal(
            provider: &PinProvider,
            source: &Arc<Served>,
            refused_at: u64,
            label: &str,
        ) {
            let calls = source.calls();
            for after in [30, PIN_RETRY_FLOOR.as_secs() + 30] {
                assert!(
                    provider.current(refused_at + after).await.sets.is_empty(),
                    "+{after}s: an older set served after a refusal"
                );
                assert_eq!(provider.status_label(), label, "+{after}s");
            }
            assert_eq!(
                source.calls(),
                calls,
                "a refused document is asked for again before the refresh interval"
            );
            assert!(
                provider
                    .current(refused_at + PIN_REFRESH_AFTER.as_secs())
                    .await
                    .sets
                    .is_empty()
            );
            assert_eq!(source.calls(), calls + 1, "refreshed on its interval");
            assert_eq!(provider.status_label(), label);
        }

        /// A failed fetch keeps the last good set until `PIN_TTL` after it
        /// was fetched, and never past the set's own `expires_at`, whichever
        /// comes first.
        #[tokio::test]
        async fn a_failed_fetch_keeps_the_last_set_only_until_pin_ttl_and_expiry() {
            let refresh = PIN_REFRESH_AFTER.as_secs();
            let ttl = PIN_TTL.as_secs();

            // A week-long set: PIN_TTL is what ends it.
            let dir = tempfile::tempdir().unwrap();
            let signer = Signer::new();
            let source = Served::serving(signer.document(1, T0, &sets(&["aa"])));
            let long = provider(&source, &signer, dir.path());
            assert_eq!(long.current(T0).await.sets.len(), 1);
            source.set(Err(SourceUnavailable));
            assert_eq!(long.current(T0 + refresh + 1).await.sets.len(), 1);
            assert_eq!(long.status_label(), "pinned");
            assert!(long.current(T0 + ttl + 1).await.sets.is_empty());
            assert_eq!(long.status_label(), "pins_unavailable");

            // A set expiring 20 minutes after issue: its expiry ends it,
            // inside PIN_TTL, while the fetch fails.
            let expiring = 20 * 60;
            let dir = tempfile::tempdir().unwrap();
            let short_doc = signer.sign(&Signer::payload(
                1,
                T0,
                T0 + expiring,
                &sets(&["aa"]),
                serde_json::json!({}),
            ));
            let source = Served::serving(short_doc);
            let short = provider(&source, &signer, dir.path());
            assert_eq!(short.current(T0).await.sets.len(), 1);
            source.set(Err(SourceUnavailable));
            assert_eq!(short.current(T0 + refresh + 1).await.sets.len(), 1);
            assert!(short.current(T0 + expiring).await.sets.is_empty());
            assert_eq!(short.status_label(), "pins_expired");

            // A set expiring inside one refresh interval is not served past
            // its expiry from the cache, before any refresh is due.
            let dir = tempfile::tempdir().unwrap();
            let brief = signer.sign(&Signer::payload(
                1,
                T0,
                T0 + 120,
                &sets(&["aa"]),
                serde_json::json!({}),
            ));
            let source = Served::serving(brief);
            let brief = provider(&source, &signer, dir.path());
            assert_eq!(brief.current(T0).await.sets.len(), 1);
            source.set(Err(SourceUnavailable));
            assert!(brief.current(T0 + 120).await.sets.is_empty());
            assert_eq!(brief.status_label(), "pins_expired");
        }

        /// Every refusal pins nothing, so no key is earned, and the status
        /// names the refusal.
        #[tokio::test]
        async fn every_refusal_earns_no_key() {
            let signer = Signer::new();
            let other = Signer::new();
            let at = FIXTURE_CAPTURED_AT;
            let good = real_sets();
            let payload = |changes: serde_json::Value| {
                Signer::payload(9, at - 60, at + 86_400, &good, changes)
            };
            let own_id = staging_pins::key_id(&signer.public());
            let label = |refusal: PinRefusal| refusal.label();
            let cases: Vec<(&str, Vec<u8>, Option<&str>, &str)> = vec![
                (
                    "not json",
                    b"not json".to_vec(),
                    None,
                    label(PinRefusal::Malformed),
                ),
                (
                    "another key",
                    other.sign(&payload(serde_json::json!({}))),
                    None,
                    label(PinRefusal::UnknownKey),
                ),
                (
                    "a bad signature under our key id",
                    other.wrap(&payload(serde_json::json!({})), &own_id),
                    None,
                    label(PinRefusal::BadSignature),
                ),
                (
                    "the wrong channel",
                    signer.sign(&payload(serde_json::json!({"channel": "production"}))),
                    None,
                    label(PinRefusal::WrongChannel),
                ),
                (
                    "the wrong gateway host",
                    signer.sign(&payload(
                        serde_json::json!({"gateway_host": "gateway.invalid"}),
                    )),
                    None,
                    label(PinRefusal::WrongGatewayHost),
                ),
                (
                    "expired",
                    signer.sign(&Signer::payload(
                        9,
                        at - 7_200,
                        at - 60,
                        &good,
                        serde_json::json!({}),
                    )),
                    None,
                    label(PinRefusal::Expired),
                ),
                (
                    "issued in the future",
                    signer.sign(&Signer::payload(
                        9,
                        at + 3_600,
                        at + 7_200,
                        &good,
                        serde_json::json!({}),
                    )),
                    None,
                    label(PinRefusal::IssuedInFuture),
                ),
                (
                    "an unparseable set",
                    signer.sign(&payload(
                        serde_json::json!({"sets": [good[0].clone(), "mrtd:zz"]}),
                    )),
                    None,
                    label(PinRefusal::SetUnparseable),
                ),
                (
                    "a v1 document",
                    signer.sign_v1(&payload(serde_json::json!({
                        "format": staging_pins::FORMAT_V1,
                    }))),
                    None,
                    label(PinRefusal::V1Document),
                ),
                (
                    "a set without the S16 registers",
                    signer.sign(&payload(serde_json::json!({
                        "sets": [format!("mrtd={}", "aa".repeat(48))],
                    }))),
                    None,
                    label(PinRefusal::SetMissingRegister),
                ),
                (
                    "a malformed allowlist",
                    signer.sign(&payload(serde_json::json!({"allowlist": {}}))),
                    None,
                    label(PinRefusal::Malformed),
                ),
                (
                    "a rollback",
                    signer.sign(&payload(serde_json::json!({}))),
                    Some("10 0000000000000000000000000000000000000000000000000000000000000000\n"),
                    "pins_rollback",
                ),
                (
                    "the same sequence with other bytes",
                    signer.sign(&payload(serde_json::json!({}))),
                    Some("9 0000000000000000000000000000000000000000000000000000000000000000\n"),
                    "pins_sequence_conflict",
                ),
                (
                    "an unreadable sequence record",
                    signer.sign(&payload(serde_json::json!({}))),
                    Some("not a sequence"),
                    "pins_sequence_unreadable",
                ),
            ];
            for (name, document, recorded, expected) in cases {
                let dir = tempfile::tempdir().unwrap();
                if let Some(recorded) = recorded {
                    std::fs::write(dir.path().join(SEQUENCE_FILE), recorded).unwrap();
                }
                let source = Served::serving(document);
                let provider = Arc::new(provider(&source, &signer, dir.path()));
                let (attestor, _) = attestor_over(Arc::clone(&provider), at);
                let answer = attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
                assert!(
                    !matches!(answer, Attestation::Keys(_)),
                    "{name}: {answer:?}"
                );
                assert_eq!(provider.status_label(), expected, "{name}");
            }

            // The sequence record cannot be written (its directory is
            // missing), or cannot be read (a directory stands in its place).
            let unrecordable: Vec<(&str, fn(&std::path::Path) -> std::path::PathBuf, &str)> = vec![
                (
                    "an unwritable sequence record",
                    |dir| dir.join("missing").join(SEQUENCE_FILE),
                    "pins_sequence_unwritable",
                ),
                (
                    "a sequence record that is a directory",
                    |dir| {
                        let path = dir.join(SEQUENCE_FILE);
                        std::fs::create_dir(&path).unwrap();
                        path
                    },
                    "pins_sequence_unreadable",
                ),
            ];
            for (name, sequence_file, expected) in unrecordable {
                let dir = tempfile::tempdir().unwrap();
                let source = Served::serving(signer.sign(&payload(serde_json::json!({}))));
                let provider = Arc::new(PinProvider::signed_with(
                    Box::new(Arc::clone(&source)),
                    &signer.public(),
                    sequence_file(dir.path()),
                ));
                let (attestor, _) = attestor_over(Arc::clone(&provider), at);
                let answer = attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
                assert!(
                    !matches!(answer, Attestation::Keys(_)),
                    "{name}: {answer:?}"
                );
                assert_eq!(provider.status_label(), expected, "{name}");
            }

            // Positive control: the same signer, the same sets and time,
            // with nothing wrong, does earn the key.
            let dir = tempfile::tempdir().unwrap();
            let source = Served::serving(signer.sign(&payload(serde_json::json!({}))));
            let provider = Arc::new(provider(&source, &signer, dir.path()));
            let (attestor, key) = attestor_over(provider, at);
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Keys(vec![key])
            );
        }

        /// The compiled-in key is the one the provider the daemon builds
        /// checks against, and the placeholder that stands in for it until
        /// the real key exists refuses every document.
        #[tokio::test]
        async fn the_compiled_in_key_refuses_a_document_it_did_not_sign() {
            let signer = Signer::new();
            let dir = tempfile::tempdir().unwrap();
            let source = Served::serving(signer.document(1, T0, &sets(&["aa"])));
            let provider = PinProvider::signed_with(
                Box::new(Arc::clone(&source)),
                crate::channel::STAGING_PIN_PUBLIC_KEY,
                dir.path().join(SEQUENCE_FILE),
            );
            assert!(provider.current(T0).await.sets.is_empty());
            assert_eq!(provider.status_label(), label_unknown_key());

            // A document that names the compiled-in key's id reaches the
            // signature check, which refuses it.
            let dir = tempfile::tempdir().unwrap();
            let claimed = signer.wrap(
                &Signer::payload(1, T0, T0 + 86_400, &sets(&["aa"]), serde_json::json!({})),
                &staging_pins::key_id(crate::channel::STAGING_PIN_PUBLIC_KEY),
            );
            let source = Served::serving(claimed);
            let provider = PinProvider::signed_with(
                Box::new(Arc::clone(&source)),
                crate::channel::STAGING_PIN_PUBLIC_KEY,
                dir.path().join(SEQUENCE_FILE),
            );
            assert!(provider.current(T0).await.sets.is_empty());
            assert_eq!(provider.status_label(), PinRefusal::BadSignature.label());
        }

        /// The compiled-in key is a real Ed25519 public key, and the
        /// production release scan lists it, so a release binary that carried
        /// it would be refused (staging design 5.7). The two cannot drift
        /// apart.
        #[test]
        fn the_compiled_in_key_is_real_and_listed_for_the_release_scan() {
            let key = crate::channel::STAGING_PIN_PUBLIC_KEY;
            assert_eq!(key.len(), 32);
            let listed = std::fs::read_to_string(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../scripts/ci/staging-pin-public-keys.txt"
            ))
            .unwrap();
            let hex: String = key.iter().map(|b| format!("{b:02x}")).collect();
            assert!(
                listed
                    .lines()
                    .map(str::trim)
                    .any(|line| !line.starts_with('#') && line == hex),
                "the compiled-in staging pin key is not listed for the release scan"
            );
        }

        fn label_unknown_key() -> &'static str {
            PinRefusal::UnknownKey.label()
        }

        /// The signed document is fetched from the staging site, and its
        /// gateway host is the build's own NEAR AI gateway.
        #[test]
        fn the_pin_url_is_on_the_staging_site() {
            let url = url::Url::parse(crate::channel::STAGING_PIN_URL).unwrap();
            assert_eq!(url.scheme(), "https");
            assert_eq!(url.host_str(), Some(crate::channel::STAGING_HOST));
            assert_eq!(url.path(), "/near-ai-pins/current.json");
            assert_eq!(
                staging_gateway_host().as_deref(),
                url::Url::parse(crate::channel::NEAR_AI_ATTESTATION_GATEWAY)
                    .unwrap()
                    .host_str()
            );
        }
    }

    /// The RTMR3 replay and the compose-manager check, as a staging build
    /// makes them before it earns a key (S16, spec 5.5 steps 6 and 7).
    #[cfg(feature = "near-ai-staging")]
    mod replay {
        use super::*;
        use trace_commons_attestation::staging_replay::Allowlist;

        /// The S16 set of the staged quote: `mrtd`, `rtmr0-2` and
        /// `mrconfigid`, never a static RTMR3, which is what the replay
        /// stands in for.
        fn s16_pins() -> Vec<ExpectedMeasurements> {
            let verified = verify_staged(
                &fixture_quote(),
                &parse_collateral(COLLATERAL).unwrap(),
                FIXTURE_CAPTURED_AT,
            )
            .unwrap();
            vec![one_set(&format!(
                "mrtd={},mrconfigid={},rtmr0={},rtmr1={},rtmr2={}",
                verified.mrtd,
                verified.mr_config_id,
                verified.rtmr[0],
                verified.rtmr[1],
                verified.rtmr[2]
            ))]
        }

        fn quote_hex() -> String {
            hex::encode(fixture_quote())
        }

        fn key_and_nonce() -> (String, String) {
            let quote = fixture_quote();
            (hex::encode(&quote[568..600]), hex::encode(&quote[600..632]))
        }

        /// A report whose one model entry carries `events` as its log and a
        /// record over `record_actions`.
        fn report_over(
            events: &[(String, String)],
            record_actions: &[serde_json::Value],
        ) -> String {
            let (key, nonce) = key_and_nonce();
            let mut entry = entry_json(&quote_hex(), &key, &nonce, MODEL);
            entry["event_log"] = event_log(events);
            entry["compose_manager_attestation"]["actions"] = serde_json::json!(record_actions);
            entry["compose_manager_attestation"]["actions_hash"] =
                serde_json::json!(actions_hash(record_actions));
            serde_json::json!({ "model_attestations": [entry] }).to_string()
        }

        /// A verifier whose model quote measured `events` and whose
        /// compose-manager quote binds `record_actions`, altered by
        /// `alter_compose`.
        fn verifier(
            events: Vec<(String, String)>,
            record_actions: Vec<serde_json::Value>,
            alter_compose: impl Fn(&mut VerifiedQuote) + Send + Sync + 'static,
        ) -> QuoteVerifier {
            with_compose_over(
                record_actions,
                alter_compose,
                move |quote, collateral, now| {
                    let mut verified = verify_staged(quote, collateral, now)?;
                    if quote == fixture_quote().as_slice() {
                        verified.rtmr[3] = replayed(&events);
                    }
                    Some(verified)
                },
            )
        }

        struct Run {
            attestor: NearAiQuoteAttestor,
            pins: Arc<PinProvider>,
            reports: Arc<AtomicUsize>,
        }

        fn run(report: String, pins: PinProvider, verify: QuoteVerifier) -> Run {
            let (_, nonce) = key_and_nonce();
            let reports = Arc::new(AtomicUsize::new(0));
            let pins = Arc::new(pins);
            let attestor = NearAiQuoteAttestor::with_parts(
                Box::new(Fixture {
                    report,
                    reports: Arc::clone(&reports),
                    collateral_ok: true,
                }),
                Arc::clone(&pins),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                verify,
            );
            Run {
                attestor,
                pins,
                reports,
            }
        }

        /// Earn under the fixture's pins and allowlist: the answer and the
        /// status label it left.
        async fn earn(report: String, verify: QuoteVerifier) -> (Attestation, &'static str) {
            let run = run(report, fixed_pins(s16_pins()), verify);
            let answer = run.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
            (answer, run.pins.attestation_label())
        }

        fn good() -> (String, QuoteVerifier) {
            (
                report_over(&ten_events(), &actions()),
                verifier(ten_events(), actions(), |_| {}),
            )
        }

        #[tokio::test]
        async fn a_staging_build_earns_a_key_only_when_set_replay_and_allowlist_pass() {
            let (key, _) = key_and_nonce();
            // All of them pass: the key.
            let (report, verify) = good();
            assert_eq!(
                earn(report, verify).await,
                (Attestation::Keys(vec![key]), ATTESTED)
            );

            // The set does not match: no key, whatever the replay says.
            let (report, verify) = good();
            let off_pins = run(report, fixed_pins(other_pins()), verify);
            assert_eq!(
                off_pins.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested
            );
            assert_eq!(off_pins.pins.attestation_label(), "quote_not_pinned");

            // The set matches but there is no allowlist to check against:
            // no key.
            let (report, verify) = good();
            let bare = run(report, PinProvider::fixed(s16_pins()), verify);
            assert_eq!(
                bare.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested
            );
            assert_eq!(bare.pins.attestation_label(), "pins_no_allowlist");

            // The set matches and the replay holds, against the
            // refuse-everything placeholder allowlist: no key.
            let placeholder = Allowlist::from_json(&serde_json::json!({
                "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
                "compose_actions": {"compose_manager_images": [], "compose_files": []},
            }))
            .unwrap();
            let (report, verify) = good();
            let refusing = run(
                report,
                PinProvider::fixed_allowing(s16_pins(), placeholder),
                verify,
            );
            assert_eq!(
                refusing.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::NotAttested
            );
            assert_eq!(
                refusing.pins.attestation_label(),
                "rtmr3_app_id_not_allowed"
            );

            // The log is missing: no key.
            let (key, nonce) = key_and_nonce();
            let mut entry = entry_json(&quote_hex(), &key, &nonce, MODEL);
            entry.as_object_mut().unwrap().remove("event_log");
            let report = serde_json::json!({ "model_attestations": [entry] }).to_string();
            assert_eq!(
                earn(report, verifier(ten_events(), actions(), |_| {})).await,
                (Attestation::NotAttested, "rtmr3_log_missing")
            );
        }

        #[tokio::test]
        async fn a_staging_build_refuses_an_unknown_event() {
            // The quote really measured this log, an eleventh event and all.
            let mut events = ten_events();
            events.insert(8, ("add-container".to_string(), "00".to_string()));
            assert_eq!(
                earn(
                    report_over(&events, &actions()),
                    verifier(events, actions(), |_| {})
                )
                .await,
                (Attestation::NotAttested, "rtmr3_event_unknown")
            );
            // Another key provider, which the quote also really measured.
            let events: Vec<(String, String)> = ten_events()
                .into_iter()
                .map(|(n, p)| {
                    if n == "key-provider" {
                        (n, hex::encode(r#"{"name":"kms","id":"another"}"#))
                    } else {
                        (n, p)
                    }
                })
                .collect();
            assert_eq!(
                earn(
                    report_over(&events, &actions()),
                    verifier(events, actions(), |_| {})
                )
                .await,
                (Attestation::NotAttested, "rtmr3_key_provider_not_allowed")
            );
        }

        #[tokio::test]
        async fn a_staging_build_refuses_a_replay_mismatch() {
            // The report's log is altered after the quote measured the
            // original: it no longer replays to the quote's RTMR3.
            let mut altered = ten_events();
            altered[8].1 = "00".to_string();
            assert_eq!(
                earn(
                    report_over(&altered, &actions()),
                    verifier(ten_events(), actions(), |_| {})
                )
                .await,
                (Attestation::NotAttested, "rtmr3_replay_mismatch")
            );
        }

        #[tokio::test]
        async fn a_staging_build_accepts_an_allowlisted_compose_action() {
            let (key, _) = key_and_nonce();
            // The listed actions, at other times: the timestamp is not
            // compared.
            let mut later = actions();
            later[1]["timestamp"] = serde_json::json!("2027-01-01T00:00:00Z");
            // Which services a listed file starts or stops is free, and the
            // tag is not matched.
            later[1]["services"] = serde_json::json!(["model-b"]);
            later[2]["services"] = serde_json::json!([]);
            later[1]["tag"] = serde_json::json!("v9.9.9");
            assert_eq!(
                earn(
                    report_over(&ten_events(), &later),
                    verifier(ten_events(), later.clone(), |_| {})
                )
                .await,
                (Attestation::Keys(vec![key]), ATTESTED)
            );
        }

        #[tokio::test]
        async fn a_staging_build_refuses_a_disallowed_compose_image() {
            // A compose file swapped after boot for one that logs prompts:
            // the record honestly binds it, and its hash is not listed.
            let mut swapped = actions();
            swapped.push(serde_json::json!({
                "action": "compose_up", "tag": "v0.0.1", "commit": COMMIT,
                "file": COMPOSE_FILE, "file_sha256": "f2".repeat(32),
                "services": ["model-a"], "timestamp": "2026-01-01T00:00:03Z",
            }));
            assert_eq!(
                earn(
                    report_over(&ten_events(), &swapped),
                    verifier(ten_events(), swapped.clone(), |_| {})
                )
                .await,
                (Attestation::NotAttested, "compose_action_not_allowed")
            );
            // And a record whose list was edited after its quote was made.
            assert_eq!(
                earn(
                    report_over(&ten_events(), &swapped),
                    verifier(ten_events(), actions(), |_| {})
                )
                .await,
                (Attestation::NotAttested, "compose_actions_hash_mismatch")
            );
        }

        #[tokio::test]
        async fn a_staging_build_refuses_each_unlisted_or_unknown_compose_action() {
            let mut cases: Vec<(Vec<serde_json::Value>, &str)> = Vec::new();
            // Another compose-manager image, or the listed one in another
            // container.
            let mut image = actions();
            image[0]["image"] = serde_json::json!(format!(
                "registry.example/compose-manager@sha256:{}",
                "cd".repeat(32)
            ));
            cases.push((image, "compose_action_not_allowed"));
            let mut container = actions();
            container[0]["container"] = serde_json::json!("sidecar");
            cases.push((container, "compose_action_not_allowed"));
            // A compose_up of the listed file at another commit.
            let mut commit = actions();
            commit[1]["commit"] = serde_json::json!("c2".repeat(20));
            cases.push((commit, "compose_action_not_allowed"));
            // A compose_down of a file not listed.
            let mut down = actions();
            down[2]["file"] = serde_json::json!("example/other.yaml");
            cases.push((down, "compose_action_not_allowed"));
            // A kind the format does not define.
            let mut unknown = actions();
            unknown[1]["action"] = serde_json::json!("compose_restart");
            cases.push((unknown, "compose_action_unknown"));
            // A key the kind does not carry, and a numeric timestamp.
            let mut extra = actions();
            extra[2]["file_sha256"] = serde_json::json!(FILE_SHA256);
            cases.push((extra, "compose_manager_malformed"));
            let mut numeric = actions();
            numeric[0]["timestamp"] = serde_json::json!(1);
            cases.push((numeric, "compose_manager_malformed"));
            for (record_actions, label) in cases {
                assert_eq!(
                    earn(
                        report_over(&ten_events(), &record_actions),
                        verifier(ten_events(), record_actions.clone(), |_| {})
                    )
                    .await,
                    (Attestation::NotAttested, label),
                    "{label}"
                );
            }
        }

        #[tokio::test]
        async fn a_staging_build_refuses_a_compose_manager_quote_from_another_td() {
            for alter in [
                (|q: &mut VerifiedQuote| q.mrtd = "12".repeat(48)) as fn(&mut VerifiedQuote),
                |q| q.rtmr[3] = "13".repeat(48),
                |q| q.mr_config_id = "14".repeat(48),
            ] {
                assert_eq!(
                    earn(
                        report_over(&ten_events(), &actions()),
                        verifier(ten_events(), actions(), alter)
                    )
                    .await,
                    (Attestation::NotAttested, "compose_manager_other_td")
                );
            }
            // A record quote that does not verify at all.
            let (key, nonce) = key_and_nonce();
            let mut entry = entry_json(&quote_hex(), &key, &nonce, MODEL);
            entry["compose_manager_attestation"]["quote"] = serde_json::json!("00");
            let report = serde_json::json!({ "model_attestations": [entry] }).to_string();
            assert_eq!(
                earn(report, verifier(ten_events(), actions(), |_| {})).await,
                (Attestation::NotAttested, "compose_manager_quote_unverified")
            );
        }

        #[tokio::test]
        async fn a_staging_build_takes_the_log_and_compose_record_from_the_quotes_report() {
            // One report per answer: the quote, its log and its record all
            // come from the fetch made with this answer's nonce.
            let (report, verify) = good();
            let one = run(report, fixed_pins(s16_pins()), verify);
            assert!(matches!(
                one.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Keys(_)
            ));
            assert_eq!(one.reports.load(Ordering::SeqCst), 1);

            // A record quoted for another fetch's nonce is never combined
            // with this one's quote.
            assert_eq!(
                earn(
                    report_over(&ten_events(), &actions()),
                    verifier(ten_events(), actions(), |q| q.report_data[32..64]
                        .fill(0x5a))
                )
                .await,
                (Attestation::NotAttested, "compose_manager_nonce_mismatch")
            );

            // The log and record are the ones beside the quote: a good log
            // on another model's entry does not stand in for a bad one on
            // ours.
            let (key, nonce) = key_and_nonce();
            let other = entry_json(&quote_hex(), &key, &nonce, "Qwen/Qwen3.8-27B");
            let mut ours = entry_json(&quote_hex(), &key, &nonce, MODEL);
            let mut bad = ten_events();
            bad[8].1 = "00".to_string();
            ours["event_log"] = event_log(&bad);
            let report = serde_json::json!({ "model_attestations": [other, ours] }).to_string();
            assert_eq!(
                earn(report, verifier(ten_events(), actions(), |_| {})).await,
                (Attestation::NotAttested, "rtmr3_replay_mismatch")
            );
        }

        #[tokio::test]
        async fn a_staging_build_checks_every_entry_for_the_model() {
            // Two entries for our model, the first of which passes. Each
            // entry's own log and record are checked, so a bad second entry
            // refuses the key rather than riding on the first.
            let (key, nonce) = key_and_nonce();
            let good_entry = || entry_json(&quote_hex(), &key, &nonce, MODEL);
            let two = |second: serde_json::Value| {
                serde_json::json!({ "model_attestations": [good_entry(), second] }).to_string()
            };

            // The control: two good entries earn the key.
            let (answer, label) =
                earn(two(good_entry()), verifier(ten_events(), actions(), |_| {})).await;
            assert!(matches!(answer, Attestation::Keys(_)), "{answer:?}");
            assert_eq!(label, ATTESTED);

            // The second entry's log does not replay to its quote.
            let mut bad_log = good_entry();
            let mut altered = ten_events();
            altered[8].1 = "00".to_string();
            bad_log["event_log"] = event_log(&altered);
            // The second entry has no compose-manager record.
            let mut no_record = good_entry();
            no_record
                .as_object_mut()
                .unwrap()
                .remove("compose_manager_attestation");
            // The second entry's record quote does not verify.
            let mut bad_record = good_entry();
            bad_record["compose_manager_attestation"]["quote"] = serde_json::json!("00");
            for (second, expected) in [
                (bad_log, "rtmr3_replay_mismatch"),
                (no_record, "compose_manager_missing"),
                (bad_record, "compose_manager_quote_unverified"),
            ] {
                assert_eq!(
                    earn(two(second), verifier(ten_events(), actions(), |_| {})).await,
                    (Attestation::NotAttested, expected),
                    "{expected}"
                );
            }
        }

        /// The fixture, with collateral for the compose-manager quote alone
        /// unavailable while `down` is set.
        struct ComposeCollateralDown {
            inner: Fixture,
            down: Arc<std::sync::atomic::AtomicBool>,
        }

        #[async_trait::async_trait]
        impl AttestationSource for ComposeCollateralDown {
            async fn report(&self, model: &str, nonce: &str) -> Result<String, SourceUnavailable> {
                self.inner.report(model, nonce).await
            }
            async fn collateral(&self, quote: &[u8]) -> Result<Collateral, SourceUnavailable> {
                if self.down.load(Ordering::SeqCst) && quote == compose_quote().as_slice() {
                    return Err(SourceUnavailable);
                }
                self.inner.collateral(quote).await
            }
        }

        #[tokio::test]
        async fn compose_manager_collateral_unavailable_is_no_answer() {
            // The model quote's collateral is there and the record's is not:
            // the record cannot be checked, so there is no key and no
            // verdict either, and the next call asks again.
            let (report, verify) = good();
            let (key, nonce) = key_and_nonce();
            let reports = Arc::new(AtomicUsize::new(0));
            let down = Arc::new(std::sync::atomic::AtomicBool::new(true));
            let pins = Arc::new(fixed_pins(s16_pins()));
            let attestor = NearAiQuoteAttestor::with_parts(
                Box::new(ComposeCollateralDown {
                    inner: Fixture {
                        report,
                        reports: Arc::clone(&reports),
                        collateral_ok: true,
                    },
                    down: Arc::clone(&down),
                }),
                Arc::clone(&pins),
                Box::new(|| FIXTURE_CAPTURED_AT),
                Box::new(move || Some(nonce.clone())),
                verify,
            );
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Unavailable
            );
            assert_eq!(pins.attestation_label(), ATTESTATION_NOT_CHECKED);
            assert_eq!(reports.load(Ordering::SeqCst), 1);

            // Nothing was remembered: with the collateral back, the next
            // call fetches again and earns the key.
            down.store(false, Ordering::SeqCst);
            assert_eq!(
                attestor.model_keys(NEAR_AI_BACKEND, MODEL).await,
                Attestation::Keys(vec![key])
            );
            assert_eq!(reports.load(Ordering::SeqCst), 2);
        }

        #[tokio::test]
        async fn a_staging_build_never_takes_a_gateway_quote_for_the_model() {
            // A gateway attestation carrying a good quote, log and record,
            // and no model entry: no key.
            let (key, nonce) = key_and_nonce();
            let mut gateway = entry_json(&quote_hex(), &key, &nonce, MODEL);
            gateway.as_object_mut().unwrap().remove("model_name");
            gateway["report_data"] = serde_json::json!(format!("{key}{nonce}"));
            let gateway_only = serde_json::json!({ "gateway_attestation": gateway }).to_string();
            assert_eq!(
                earn(gateway_only, verifier(ten_events(), actions(), |_| {}))
                    .await
                    .0,
                Attestation::NotAttested
            );
            // A model entry without a log, beside a gateway attestation
            // with one: the gateway's log is not read for the model.
            let mut entry = entry_json(&quote_hex(), &key, &nonce, MODEL);
            entry.as_object_mut().unwrap().remove("event_log");
            let report = serde_json::json!({
                "gateway_attestation": gateway,
                "model_attestations": [entry],
            })
            .to_string();
            assert_eq!(
                earn(report, verifier(ten_events(), actions(), |_| {})).await,
                (Attestation::NotAttested, "rtmr3_log_missing")
            );
        }

        #[tokio::test]
        async fn status_names_the_replay_and_allowlist_refusals() {
            // Before any answer, nothing has been checked.
            let (report, verify) = good();
            let fresh = run(report, fixed_pins(s16_pins()), verify);
            assert_eq!(fresh.pins.attestation_label(), ATTESTATION_NOT_CHECKED);

            let mut mr_kms = ten_events();
            mr_kms[5].1 = "99".repeat(48);
            let mut marker = ten_events();
            marker[9].1 = "00".to_string();
            let cases: Vec<(Vec<(String, String)>, Vec<serde_json::Value>, &str)> = vec![
                (mr_kms, actions(), "rtmr3_mr_kms_not_allowed"),
                (marker, actions(), "rtmr3_marker_not_allowed"),
            ];
            for (events, record_actions, label) in cases {
                let report = report_over(&events, &record_actions);
                let (answer, status) = earn(report, verifier(events, record_actions, |_| {})).await;
                assert_eq!(answer, Attestation::NotAttested, "{label}");
                assert_eq!(status, label);
            }

            // A refusal is replaced by the next outcome, never kept: the
            // label is the last answer's.
            let (report, verify) = good();
            let run = run(report, fixed_pins(s16_pins()), verify);
            let _ = run.attestor.model_keys(NEAR_AI_BACKEND, MODEL).await;
            assert_eq!(run.pins.attestation_label(), ATTESTED);
            // Every label is a label: no event value, register or nonce.
            for label in [ATTESTED, ATTESTATION_NOT_CHECKED, "quote_not_pinned"] {
                assert!(label.bytes().all(|b| b.is_ascii_lowercase() || b == b'_'));
            }
        }
    }
}
