//! Signed operator routing, separate from the stable enrollment identity.
//!
//! A matching configuration is authoritative, including withdrawal. Endpoint
//! pins are paired and scoped to that endpoint; enrollment pins only establish
//! that the device opted into witnessing. Neither settings nor generic upload
//! allowlists are changed. Failover ends before the first raw POST.

use super::{
    AcceptedWitnessPin, WITNESS_EXPECTED_MEASUREMENT_CONTROL, WitnessTrust, WitnessTrustError,
    accepted_list::{self, InstallOutcome, RefreshOutcome},
    transport::{self, HttpWitnessTransport, WitnessTransport},
    verify::VerifiedWitness,
};
use crate::config::{ContributorConfig, WitnessSettings};
use chrono::{DateTime, Utc};
use std::{path::Path, sync::Arc};
use trace_commons_attestation::measurements::{ExpectedMeasurements, ExpectedMeasurementsError};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::witness_accepted_list::AcceptedEntry;
use trace_commons_protocol::witness_configuration::{
    WITNESS_CONFIGURATION_MAX_BYTES, WitnessConfiguration, verify_signed_configuration,
};

/// Exact signed bytes, verified on every read. Independent of the V1 list.
pub const CONFIGURATION_FILE: &str = "witness-configuration.json";
const ERR_URL_MISMATCH: &str = "witness-configuration-enrollment-mismatch";
const ERR_MEASUREMENT: &str = "witness-configuration-measurement-malformed";
const ERR_STORE: &str = "witness-configuration-store-failed";

fn parse_measurement(
    entry: &AcceptedEntry,
) -> Result<ExpectedMeasurements, ExpectedMeasurementsError> {
    ExpectedMeasurements::from_env_value(Some(&entry.measurements))?
        .ok_or(ExpectedMeasurementsError::NoPins)
}
fn measurements_valid(config: &WitnessConfiguration) -> bool {
    config
        .endpoints
        .iter()
        .flat_map(|endpoint| &endpoint.entries)
        .all(|entry| parse_measurement(entry).is_ok())
}
/// Read a bounded signed configuration and verify both signature and pin syntax.
pub fn load_verified(dir: &Path, keys: &[[u8; 32]]) -> Option<WitnessConfiguration> {
    use std::io::Read as _;
    if keys.is_empty() {
        return None;
    }
    let file = std::fs::File::open(dir.join(CONFIGURATION_FILE)).ok()?;
    let mut bytes = Vec::new();
    file.take(WITNESS_CONFIGURATION_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() > WITNESS_CONFIGURATION_MAX_BYTES {
        return None;
    }
    let config = verify_signed_configuration(&bytes, keys).ok()?;
    measurements_valid(&config).then_some(config)
}
/// Install only a relevant, fully parsed, strictly newer signed document.
pub fn install(
    dir: &Path,
    bytes: &[u8],
    keys: &[[u8; 32]],
    enrollment_url: &str,
) -> Result<InstallOutcome, &'static str> {
    let config = verify_signed_configuration(bytes, keys).map_err(|error| error.label())?;
    if !config.names_witness(enrollment_url) {
        return Err(ERR_URL_MISMATCH);
    }
    if !measurements_valid(&config) {
        return Err(ERR_MEASUREMENT);
    }
    let _lock = install_lock(dir)?;
    let held = match std::fs::symlink_metadata(dir.join(CONFIGURATION_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            Some(load_verified(dir, keys).ok_or(ERR_STORE)?)
        }
        _ => return Err(ERR_STORE),
    };
    if held.is_some_and(|held| held.version >= config.version) {
        return Ok(InstallOutcome::NotNewer);
    }
    crate::config::write_atomic_0600(dir, &dir.join(CONFIGURATION_FILE), bytes)
        .map_err(|_| ERR_STORE)?;
    Ok(InstallOutcome::Installed {
        version: config.version,
    })
}
// A stable inode, never the atomically replaced document. Lock across the
// high-water read and atomic write, including independent daemon/CLI processes.
fn install_lock(dir: &Path) -> Result<std::fs::File, &'static str> {
    let path = dir.join("witness-configuration.lock");
    if std::fs::symlink_metadata(&path)
        .is_ok_and(|meta| !meta.is_file() || meta.file_type().is_symlink())
    {
        return Err(ERR_STORE);
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path).map_err(|_| ERR_STORE)?;
    file.lock().map_err(|_| ERR_STORE)?;
    Ok(file)
}

fn read_cache(dir: &Path) -> Result<Option<WitnessConfiguration>, WitnessTrustError> {
    match std::fs::symlink_metadata(dir.join(CONFIGURATION_FILE)) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Ok(meta) if meta.is_file() && !meta.file_type().is_symlink() => {
            load_verified(dir, &accepted_list::trusted_keys(dir))
                .map(Some)
                .ok_or_else(unpinned)
        }
        _ => Err(unpinned()),
    }
}

/// Public-data digest used only to notice matching routing/validity changes.
/// Unrelated configurations cannot release this enrollment's measurement holds.
pub fn active_state(settings: &WitnessSettings, dir: &Path, now: DateTime<Utc>) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut config = read_cache(dir).ok()??;
    if !config.names_witness(&settings.url) {
        return None;
    }
    for endpoint in &mut config.endpoints {
        endpoint.entries.retain(|entry| entry.is_valid_at(now));
    }
    let bytes = serde_json::to_vec(&config).ok()?;
    Some(hex::encode(Sha256::digest(bytes)))
}

/// Anonymous global fixed-path fetch. Failure never changes held trust.
pub async fn refresh(cfg: &ContributorConfig, dir: &Path) -> RefreshOutcome {
    let Some(settings) = &cfg.witness else {
        return RefreshOutcome::Skipped;
    };
    let keys = accepted_list::trusted_keys(dir);
    if keys.is_empty() {
        return RefreshOutcome::Skipped;
    }
    let fetched = async {
        let origin = crate::config::ingest_origin_url(&cfg.ingest_url, "/").ok()?;
        let client = crate::credit_estimate_table::EstimateTableClient::new(
            origin.as_str(),
            cfg.allowed_hosts.as_deref(),
        )
        .ok()?;
        client.fetch_witness_configuration().await.ok()
    }
    .await;
    let outcome = match fetched {
        Some(bytes) => match install(dir, &bytes, &keys, &settings.url) {
            Ok(InstallOutcome::Installed { version }) => RefreshOutcome::Installed { version },
            Ok(InstallOutcome::NotNewer) => RefreshOutcome::NotNewer,
            Err(_) => RefreshOutcome::Failed,
        },
        None => RefreshOutcome::Failed,
    };
    match outcome {
        RefreshOutcome::Installed { version } => tracing::info!(
            event = "witness-configuration-installed",
            version,
            "witness configuration"
        ),
        RefreshOutcome::Failed => tracing::debug!(
            event = "witness-configuration-fetch-failed",
            "witness configuration"
        ),
        _ => {}
    }
    outcome
}

#[derive(Clone)]
struct Candidate {
    url: String,
    // None is the legacy path only. A signed endpoint never inherits it.
    legacy: Option<WitnessTrust>,
    entries: Vec<AcceptedEntry>,
}
impl Candidate {
    fn trust_at(&self, now: DateTime<Utc>) -> Option<WitnessTrust> {
        if let Some(trust) = &self.legacy {
            return Some(trust.clone());
        }
        let mut pins = self
            .entries
            .iter()
            .filter(|entry| entry.is_valid_at(now))
            .map(|entry| {
                Some(AcceptedWitnessPin {
                    signing_address: entry.signing_address.clone(),
                    measurements: parse_measurement(entry).ok()?,
                })
            })
            .collect::<Option<Vec<_>>>()?
            .into_iter();
        let first = pins.next()?;
        Some(WitnessTrust {
            signing_address: first.signing_address,
            measurements: vec![first.measurements],
            accepted: pins.collect(),
        })
    }
}
fn unpinned() -> WitnessTrustError {
    WitnessTrustError::WitnessMeasurementUnpinned {
        control: WITNESS_EXPECTED_MEASUREMENT_CONTROL,
        reported: None,
    }
}
fn candidates(
    settings: &WitnessSettings,
    dir: &Path,
    now: DateTime<Utc>,
) -> Result<Vec<Candidate>, WitnessTrustError> {
    let signup = settings.trust().map_err(|_| unpinned())?;
    if !signup.is_pinned() {
        return Err(unpinned());
    }
    if let Some(config) = read_cache(dir)?.filter(|config| config.names_witness(&settings.url)) {
        return Ok(config
            .endpoints
            .into_iter()
            .map(|endpoint| Candidate {
                url: endpoint.url,
                legacy: None,
                entries: endpoint.entries,
            })
            .collect());
    }
    let legacy = accepted_list::trust_with_accepted(settings, dir, now).map_err(|_| unpinned())?;
    Ok(vec![Candidate {
        url: settings.url.clone(),
        legacy: Some(legacy),
        entries: Vec::new(),
    }])
}
/// This list belongs solely to this witness transport. Collateral still goes
/// to configured ingest. A configured/env operator policy remains absolute.
fn transport_allowlist(cfg: &ContributorConfig, endpoint: &str) -> HostAllowlist {
    let explicit = crate::config::allowlist_for(cfg.allowed_hosts.as_deref());
    if explicit.is_enforcing() {
        return explicit;
    }
    HostAllowlist::from_hosts(
        [endpoint, cfg.ingest_url.as_str()]
            .into_iter()
            .filter_map(|url| url::Url::parse(url).ok()?.host_str().map(str::to_string)),
    )
}

#[derive(Clone, Copy)]
pub(crate) struct TransportOptions {
    pub admission: bool,
    pub background: bool,
    pub envelope_limit: usize,
}
fn http_transport(
    cfg: &ContributorConfig,
    candidate: &Candidate,
    options: TransportOptions,
) -> Result<HttpWitnessTransport, WitnessTrustError> {
    HttpWitnessTransport::new(
        candidate.url.clone(),
        cfg.ingest_url.clone(),
        Arc::new(transport_allowlist(cfg, &candidate.url)),
        transport::WITNESS_REVIEW_TIMEOUT,
    )
    .map(|transport| {
        transport
            .with_admission_evidence(options.admission)
            .with_background_workload(options.background)
            .with_envelope_limit(options.envelope_limit)
    })
}

/// Rechecked by HTTP after serializing the raw body, directly before dispatch.
pub(crate) struct RoutingGuard {
    dir: std::path::PathBuf,
    snapshot: Option<WitnessConfiguration>,
    candidate: Candidate,
    trust: WitnessTrust,
}
impl RoutingGuard {
    pub(crate) fn validate(&self) -> Result<(), WitnessTrustError> {
        if read_cache(&self.dir)? != self.snapshot
            || self.candidate.trust_at(Utc::now()).as_ref() != Some(&self.trust)
        {
            // The supervisor may already have observed this transition before
            // this in-flight request is queued. Retry instead of creating a
            // measurement hold that needs another publication to release it.
            return Err(WitnessTrustError::WitnessUnreachable);
        }
        Ok(())
    }
}
trait RoutedTransport: WitnessTransport {
    fn with_guard(self, guard: RoutingGuard) -> Self;
}
impl RoutedTransport for HttpWitnessTransport {
    fn with_guard(self, guard: RoutingGuard) -> Self {
        self.with_routing_guard(guard)
    }
}

#[derive(Clone, Copy)]
struct RoutingContext<'a> {
    settings: &'a WitnessSettings,
    dir: &'a Path,
    refresh_cfg: Option<&'a ContributorConfig>,
}
impl<'a> RoutingContext<'a> {
    #[cfg(test)]
    fn test(settings: &'a WitnessSettings, dir: &'a Path) -> Self {
        Self {
            settings,
            dir,
            refresh_cfg: None,
        }
    }
}
async fn select_with<T: RoutedTransport>(
    context: RoutingContext<'_>,
    mut build: impl FnMut(&Candidate) -> Result<T, WitnessTrustError>,
) -> Result<(T, VerifiedWitness), WitnessTrustError> {
    let result = probe_candidates_with(context.settings, context.dir, &mut build).await;
    // Only quote/pin refusals, before raw bytes are sent, may refresh and
    // repeat selection. Raw transport failures never re-enter this helper.
    if matches!(
        &result,
        Err(WitnessTrustError::WitnessMeasurementUnpinned { .. }
            | WitnessTrustError::WitnessSignerUnexpected)
    ) {
        if let Some(cfg) = context.refresh_cfg {
            if accepted_list::refresh_on_refusal(cfg, context.dir, Utc::now()).await {
                return probe_candidates_with(context.settings, context.dir, build).await;
            }
        }
    }
    result
}

async fn probe_candidates_with<T: RoutedTransport>(
    settings: &WitnessSettings,
    dir: &Path,
    mut build: impl FnMut(&Candidate) -> Result<T, WitnessTrustError>,
) -> Result<(T, VerifiedWitness), WitnessTrustError> {
    let snapshot = read_cache(dir)?;
    let endpoints = candidates(settings, dir, Utc::now())?;
    let mut failure = unpinned();
    for candidate in endpoints {
        // Read the real clock per probe, not the submission's earlier clock.
        let Some(trust) = candidate.trust_at(Utc::now()) else {
            continue;
        };
        let transport = match build(&candidate) {
            Ok(transport) => transport,
            Err(error) => {
                failure = error;
                continue;
            }
        };
        match super::probe_witness(
            &transport,
            &candidate.url,
            &trust,
            Utc::now().timestamp().max(0) as u64,
        )
        .await
        {
            Ok(verified) => {
                // A slow successful probe can cross a validity/publication
                // boundary already observed by the supervisor. Fail closed
                // transiently so the next invocation resolves current policy.
                if candidate.trust_at(Utc::now()).as_ref() != Some(&trust) {
                    return Err(WitnessTrustError::WitnessUnreachable);
                }
                // Invalid cache still refuses as unpinned; a verified policy
                // change is retryable rather than a permanent measurement hold.
                if read_cache(dir)? != snapshot {
                    return Err(WitnessTrustError::WitnessUnreachable);
                }
                let guard = RoutingGuard {
                    dir: dir.to_path_buf(),
                    snapshot,
                    candidate,
                    trust,
                };
                return Ok((transport.with_guard(guard), verified));
            }
            Err(error) => failure = error,
        }
    }
    Err(failure)
}

/// Both production raw routes share preconditions and endpoint selection.
/// The returned capability is consumed for one send; sends are outside the
/// selection loop and cannot trigger a cross-endpoint retry.
pub(crate) async fn witness_session(
    cfg: &ContributorConfig,
    settings: &WitnessSettings,
    dir: &Path,
    options: TransportOptions,
    raw: trace_commons_protocol::trace_contribution::RawTraceContribution,
    attested: Option<transport::AttestedInference<'_>>,
    granted: &transport::GrantedConsent,
) -> Result<transport::WitnessedEnvelope, WitnessTrustError> {
    session_with(
        RoutingContext {
            settings,
            dir,
            refresh_cfg: Some(cfg),
        },
        options.envelope_limit,
        raw,
        attested,
        granted,
        |candidate| http_transport(cfg, candidate, options),
    )
    .await
}
async fn session_with<T: RoutedTransport>(
    context: RoutingContext<'_>,
    limit: usize,
    raw: trace_commons_protocol::trace_contribution::RawTraceContribution,
    attested: Option<transport::AttestedInference<'_>>,
    granted: &transport::GrantedConsent,
    build: impl FnMut(&Candidate) -> Result<T, WitnessTrustError>,
) -> Result<transport::WitnessedEnvelope, WitnessTrustError> {
    if !context
        .settings
        .trust()
        .map_err(|_| unpinned())?
        .is_pinned()
    {
        return Err(unpinned());
    }
    crate::envelope::raw_contribution_size_ok(&raw, limit)
        .map_err(|_| WitnessTrustError::WitnessPayloadTooLarge)?;
    let (transport, verified) = select_with(context, build).await?;
    transport::witness_contribution(&transport, &verified, raw, attested, granted).await
}

#[async_trait::async_trait]
trait TokenTransport: RoutedTransport {
    async fn send_tokens(
        &self,
        verified: &VerifiedWitness,
        request: super::TokenBundleSession<'_>,
    ) -> Result<crate::token_bundle::CertifiedBundleUpload, WitnessTrustError>;
}
#[async_trait::async_trait]
impl TokenTransport for HttpWitnessTransport {
    async fn send_tokens(
        &self,
        verified: &VerifiedWitness,
        request: super::TokenBundleSession<'_>,
    ) -> Result<crate::token_bundle::CertifiedBundleUpload, WitnessTrustError> {
        self.witness_token_contribution(
            verified,
            request.raw,
            request.attested,
            request.granted,
            request.options,
        )
        .await
    }
}
pub(crate) async fn witness_token_session(
    cfg: &ContributorConfig,
    settings: &WitnessSettings,
    dir: &Path,
    options: TransportOptions,
    request: super::TokenBundleSession<'_>,
) -> Result<crate::token_bundle::CertifiedBundleUpload, WitnessTrustError> {
    token_session_with(
        RoutingContext {
            settings,
            dir,
            refresh_cfg: Some(cfg),
        },
        options.envelope_limit,
        request,
        |candidate| http_transport(cfg, candidate, options),
    )
    .await
}
async fn token_session_with<T: TokenTransport>(
    context: RoutingContext<'_>,
    limit: usize,
    request: super::TokenBundleSession<'_>,
    build: impl FnMut(&Candidate) -> Result<T, WitnessTrustError>,
) -> Result<crate::token_bundle::CertifiedBundleUpload, WitnessTrustError> {
    if !context
        .settings
        .trust()
        .map_err(|_| unpinned())?
        .is_pinned()
    {
        return Err(unpinned());
    }
    if !request.options.restricted_token_consent || request.attested.receipt.is_none() {
        return Err(WitnessTrustError::WitnessAdmissionEvidenceRefused);
    }
    crate::envelope::raw_contribution_size_ok(&request.raw, limit)
        .map_err(|_| WitnessTrustError::WitnessPayloadTooLarge)?;
    let (transport, verified) = select_with(context, build).await?;
    transport.send_tokens(&verified, request).await
}
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};
    use trace_commons_protocol::witness_accepted_list::{AcceptedEntry, ListSignature, key_id_for};
    use trace_commons_protocol::witness_configuration::{
        WITNESS_CONFIGURATION_SCHEMA, WitnessEndpoint,
    };
    const ANCHOR: &str = "https://enrollment.example";
    const PRIMARY: &str = "https://new-witness.example";
    const FALLBACK: &str = "https://fallback-witness.example";
    const OLD_SIGNER: &str = "0x1111111111111111111111111111111111111111";
    const NEW_SIGNER: &str = "0x2222222222222222222222222222222222222222";
    fn signer() -> Ed25519KeyPair {
        let bytes = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(bytes.as_ref()).unwrap()
    }
    fn public(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }
    fn measurement(byte: &str) -> String {
        format!("mrtd={}", byte.repeat(48))
    }
    fn settings() -> WitnessSettings {
        WitnessSettings {
            admission_evidence: false,
            url: ANCHOR.into(),
            signing_address: OLD_SIGNER.into(),
            expected_measurements: vec![measurement("aa")],
        }
    }
    fn entry(address: &str, byte: &str) -> AcceptedEntry {
        AcceptedEntry {
            signing_address: address.into(),
            measurements: measurement(byte),
            not_before: Utc::now() - chrono::Duration::hours(1),
            not_after: Utc::now() + chrono::Duration::hours(1),
            commit: None,
        }
    }
    fn document(version: u64) -> WitnessConfiguration {
        WitnessConfiguration {
            schema: WITNESS_CONFIGURATION_SCHEMA.into(),
            version,
            issued_at: Utc::now(),
            enrollment_urls: vec![ANCHOR.into(), PRIMARY.into(), FALLBACK.into()],
            endpoints: vec![
                WitnessEndpoint {
                    url: PRIMARY.into(),
                    entries: vec![entry(NEW_SIGNER, "bb")],
                },
                WitnessEndpoint {
                    url: FALLBACK.into(),
                    entries: vec![entry(OLD_SIGNER, "cc")],
                },
            ],
        }
    }
    fn signed(key: &Ed25519KeyPair, config: &WitnessConfiguration) -> Vec<u8> {
        let payload = trace_commons_protocol::canonical_json::to_canonical_string(
            &serde_json::to_value(config).unwrap(),
        )
        .unwrap();
        let signature = ListSignature {
            key_id: key_id_for(&public(key)),
            signature: hex::encode(key.sign(payload.as_bytes()).as_ref()),
        };
        serde_json::to_vec(&serde_json::json!({"payload": payload, "signatures": [signature]}))
            .unwrap()
    }
    fn setup() -> (tempfile::TempDir, Ed25519KeyPair) {
        let dir = tempfile::tempdir().unwrap();
        let key = signer();
        accepted_list::set_test_keys(dir.path(), vec![public(&key)]);
        (dir, key)
    }
    #[test]
    fn verifies_before_install_and_independently_rejects_rollback() {
        let (dir, key) = setup();
        let keys = [public(&key)];
        let bytes = signed(&key, &document(8));
        assert_eq!(
            install(dir.path(), &bytes, &keys, ANCHOR),
            Ok(InstallOutcome::Installed { version: 8 })
        );
        assert_eq!(
            install(dir.path(), &signed(&key, &document(7)), &keys, ANCHOR),
            Ok(InstallOutcome::NotNewer)
        );
        let mut malformed = document(9);
        malformed.endpoints[0].entries[0].measurements = "mrtd=not-hex".into();
        assert!(install(dir.path(), &signed(&key, &malformed), &keys, ANCHOR).is_err());
        assert!(install(dir.path(), &signed(&signer(), &document(9)), &keys, ANCHOR).is_err());
        let tampered = String::from_utf8(signed(&key, &document(9)))
            .unwrap()
            .replace("new-witness", "bad-witness");
        assert!(install(dir.path(), tampered.as_bytes(), &keys, ANCHOR).is_err());
        let mut unrelated = document(9);
        unrelated.enrollment_urls = vec![
            "https://other.example".into(),
            PRIMARY.into(),
            FALLBACK.into(),
        ];
        assert!(install(dir.path(), &signed(&key, &unrelated), &keys, ANCHOR).is_err());
        assert_eq!(
            std::fs::read(dir.path().join(CONFIGURATION_FILE)).unwrap(),
            bytes
        );
        assert_eq!(load_verified(dir.path(), &keys).unwrap().version, 8);
    }
    #[test]
    fn endpoints_own_their_pairs_and_never_inherit_the_signup_pin() {
        let (dir, key) = setup();
        install(
            dir.path(),
            &signed(&key, &document(1)),
            &[public(&key)],
            ANCHOR,
        )
        .unwrap();
        let endpoints = candidates(&settings(), dir.path(), Utc::now()).unwrap();
        assert_eq!(endpoints.len(), 2);
        assert_eq!(endpoints[0].url, PRIMARY);
        let primary = endpoints[0].trust_at(Utc::now()).unwrap();
        assert_eq!(primary.signing_address, NEW_SIGNER);
        assert_eq!(
            primary.measurements,
            vec![
                ExpectedMeasurements::from_env_value(Some(&measurement("bb")))
                    .unwrap()
                    .unwrap()
            ]
        );
        assert!(primary.accepted.is_empty());
        assert_ne!(primary, settings().trust().unwrap());
        let mut unpinned = settings();
        unpinned.expected_measurements.clear();
        assert!(matches!(
            candidates(&unpinned, dir.path(), Utc::now()),
            Err(WitnessTrustError::WitnessMeasurementUnpinned { .. })
        ));
    }
    #[test]
    fn matching_withdrawals_and_expiry_fail_closed_unrelated_and_missing_keep_legacy() {
        let (dir, key) = setup();
        assert_eq!(
            candidates(&settings(), dir.path(), Utc::now()).unwrap()[0].url,
            ANCHOR
        );
        let mut config = document(1);
        config.endpoints.clear();
        install(dir.path(), &signed(&key, &config), &[public(&key)], ANCHOR).unwrap();
        assert!(
            candidates(&settings(), dir.path(), Utc::now())
                .unwrap()
                .is_empty()
        );
        let mut other = settings();
        other.url = "https://unrelated.example".into();
        assert_eq!(
            candidates(&other, dir.path(), Utc::now()).unwrap()[0].url,
            other.url
        );
        config = document(2);
        config.endpoints[0].entries[0].not_after = Utc::now() - chrono::Duration::minutes(1);
        config.endpoints[1].entries[0].not_before = Utc::now() + chrono::Duration::minutes(1);
        install(dir.path(), &signed(&key, &config), &[public(&key)], ANCHOR).unwrap();
        assert!(
            candidates(&settings(), dir.path(), Utc::now())
                .unwrap()
                .iter()
                .all(|candidate| candidate.trust_at(Utc::now()).is_none())
        );
    }
    #[test]
    fn configuration_does_not_edit_settings_or_generic_allowlists() {
        let (dir, key) = setup();
        let store = crate::config::ConfigStore::open(dir.path().into()).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.witness = Some(settings());
        store.save_config(&cfg).unwrap();
        let original = std::fs::read(dir.path().join(crate::config::CONFIG_FILE)).unwrap();
        let digest = settings().identity_sha256();
        install(
            dir.path(),
            &signed(&key, &document(1)),
            &[public(&key)],
            ANCHOR,
        )
        .unwrap();
        assert_eq!(
            std::fs::read(dir.path().join(crate::config::CONFIG_FILE)).unwrap(),
            original
        );
        assert_eq!(
            store
                .load_config()
                .unwrap()
                .unwrap()
                .witness
                .unwrap()
                .identity_sha256(),
            digest
        );
        let endpoint = url::Url::parse(PRIMARY).unwrap();
        assert!(
            crate::config::config_allowlist(&cfg)
                .check(&endpoint)
                .is_err()
        );
        assert!(transport_allowlist(&cfg, PRIMARY).check(&endpoint).is_ok());
        assert!(
            transport_allowlist(&cfg, PRIMARY)
                .check(&url::Url::parse(FALLBACK).unwrap())
                .is_err()
        );
        cfg.allowed_hosts = Some("ingest.example,enrollment.example".into());
        assert!(transport_allowlist(&cfg, PRIMARY).check(&endpoint).is_err());
    }
    #[test]
    fn concurrent_installers_cannot_lower_the_configuration_high_water() {
        let (dir, key) = setup();
        let keys = [public(&key)];
        let older = signed(&key, &document(3));
        let newer = signed(&key, &document(4));
        let lock = install_lock(dir.path()).unwrap();
        let path = dir.path().to_path_buf();
        let (tx, rx) = std::sync::mpsc::channel();
        let task = std::thread::spawn(move || {
            tx.send(install(&path, &newer, &keys, ANCHOR)).unwrap();
        });
        assert!(
            rx.recv_timeout(std::time::Duration::from_millis(30))
                .is_err()
        );
        assert!(!dir.path().join(CONFIGURATION_FILE).exists());
        drop(lock);
        assert_eq!(
            rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(),
            Ok(InstallOutcome::Installed { version: 4 })
        );
        task.join().unwrap();
        assert_eq!(
            install(dir.path(), &older, &keys, ANCHOR),
            Ok(InstallOutcome::NotNewer)
        );
        assert_eq!(load_verified(dir.path(), &keys).unwrap().version, 4);
    }
    #[test]
    fn validity_transitions_change_the_schedule_without_a_new_publication() {
        let (dir, key) = setup();
        let now = Utc::now();
        let mut config = document(3);
        config.endpoints.truncate(1);
        config.endpoints[0].entries[0].not_before = now + chrono::Duration::minutes(1);
        config.endpoints[0].entries[0].not_after = now + chrono::Duration::minutes(2);
        install(dir.path(), &signed(&key, &config), &[public(&key)], ANCHOR).unwrap();
        let mut schedule = accepted_list::ListRefreshSchedule::starting(now);
        assert!(schedule.configuration_changed(active_state(&settings(), dir.path(), now)));
        assert!(!schedule.configuration_changed(active_state(&settings(), dir.path(), now)));
        assert!(schedule.configuration_changed(active_state(
            &settings(),
            dir.path(),
            now + chrono::Duration::minutes(1)
        )));
        assert!(!schedule.configuration_changed(active_state(
            &settings(),
            dir.path(),
            now + chrono::Duration::seconds(90)
        )));
        assert!(schedule.configuration_changed(active_state(
            &settings(),
            dir.path(),
            now + chrono::Duration::minutes(2)
        )));
        let mut other = settings();
        other.url = "https://unrelated.example".into();
        assert!(!schedule.configuration_changed(active_state(&other, dir.path(), now)));
    }

    use super::super::transport::{
        AttestationEvidence, AttestedInference, GrantedConsent, WitnessNonce,
    };
    use std::sync::Mutex;
    use trace_commons_attestation::quote::{Collateral, VerifiedQuote, parse_collateral};
    use trace_commons_protocol::trace_contribution::{
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
    };
    type Events = Arc<Mutex<Vec<String>>>;
    struct Recording {
        url: String,
        address: String,
        measurement: String,
        seen: Events,
        invalid_nonce: bool,
        delay: bool,
        // Simulate a concurrent signed withdrawal while a nonce probe is in flight.
        replace: Option<(std::path::PathBuf, Vec<u8>)>,
        fixtures: Mutex<Vec<super::super::verify::QuoteFixture>>,
        guard: Option<RoutingGuard>,
        success: Option<transport::WitnessedEnvelope>,
    }
    impl Recording {
        fn new(candidate: &Candidate, seen: &Events) -> Self {
            let trust = candidate.trust_at(Utc::now()).unwrap();
            let is_primary = candidate.url == PRIMARY;
            Self {
                url: candidate.url.clone(),
                address: trust.signing_address,
                measurement: if is_primary { "bb" } else { "cc" }.repeat(48),
                seen: seen.clone(),
                invalid_nonce: false,
                delay: false,
                replace: None,
                fixtures: Mutex::new(Vec::new()),
                guard: None,
                success: None,
            }
        }
    }
    impl RoutedTransport for Recording {
        fn with_guard(mut self, guard: RoutingGuard) -> Self {
            self.guard = Some(guard);
            self
        }
    }
    #[async_trait::async_trait]
    impl WitnessTransport for Recording {
        async fn attestation(
            &self,
            nonce: &WitnessNonce,
        ) -> Result<AttestationEvidence, WitnessTrustError> {
            self.seen
                .lock()
                .unwrap()
                .push(format!("probe:{}", self.url));
            if let Some((dir, bytes)) = &self.replace {
                install(dir, bytes, &accepted_list::trusted_keys(dir), ANCHOR).unwrap();
            }
            let mut report_data = vec![0; 64];
            report_data[..8].copy_from_slice(super::super::WITNESS_QUOTE_DOMAIN);
            let address =
                trace_commons_attestation::address::decode_address(&self.address).unwrap();
            report_data[8..28].copy_from_slice(&address);
            report_data[28..60].copy_from_slice(nonce.as_bytes());
            if self.invalid_nonce {
                report_data[28] ^= 1;
            }
            let fixture = super::super::verify::register_quote_fixture(VerifiedQuote {
                report_data,
                mrtd: self.measurement.clone(),
                mr_config_id: "00".repeat(48),
                rtmr: std::array::from_fn(|_| "00".repeat(48)),
                tcb_status: "UpToDate".into(),
                advisory_ids: Vec::new(),
            });
            let evidence = AttestationEvidence {
                quote_hex: hex::encode(&fixture.0),
                signing_address: self.address.clone(),
            };
            self.fixtures.lock().unwrap().push(fixture);
            Ok(evidence)
        }
        async fn collateral(&self, _: &[u8]) -> Result<Collateral, WitnessTrustError> {
            if self.delay {
                tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            }
            Ok(parse_collateral(include_str!("../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json")).unwrap())
        }
        async fn witness(
            &self,
            verified: &VerifiedWitness,
            body: &[u8],
        ) -> Result<transport::WitnessedEnvelope, WitnessTrustError> {
            self.guard
                .as_ref()
                .expect("selected route guard")
                .validate()?;
            assert_eq!(verified.url(), self.url);
            assert!(body.windows(11).any(|part| part == b"private-raw"));
            self.seen.lock().unwrap().push(format!("post:{}", self.url));
            self.success
                .clone()
                .ok_or(WitnessTrustError::WitnessUnreachable)
        }
        fn envelope_limit_bytes(&self) -> usize {
            crate::envelope::MAX_ENVELOPE_BYTES
        }
    }
    #[async_trait::async_trait]
    impl TokenTransport for Recording {
        async fn send_tokens(
            &self,
            verified: &VerifiedWitness,
            request: super::super::TokenBundleSession<'_>,
        ) -> Result<crate::token_bundle::CertifiedBundleUpload, WitnessTrustError> {
            self.guard
                .as_ref()
                .expect("selected route guard")
                .validate()?;
            assert_eq!(verified.url(), self.url);
            assert!(request.options.restricted_token_consent);
            assert!(request.attested.receipt.is_some());
            self.seen
                .lock()
                .unwrap()
                .push(format!("token-post:{}", self.url));
            Err(WitnessTrustError::WitnessSaturated {
                retry_after_secs: 10,
            })
        }
    }
    fn raw() -> RawTraceContribution {
        RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "private-raw".into(),
                response: None,
                tool_calls: Vec::new(),
                started_at: Utc::now(),
                completed_at: None,
                state: None,
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                ..Default::default()
            },
        )
    }
    fn consent() -> GrantedConsent {
        GrantedConsent {
            scopes: Vec::new(),
            uses: Vec::new(),
        }
    }
    fn recording_events() -> Events {
        Arc::new(Mutex::new(Vec::new()))
    }
    fn install_document(dir: &Path, key: &Ed25519KeyPair, config: &WitnessConfiguration) {
        install(dir, &signed(key, config), &[public(key)], ANCHOR).unwrap();
    }

    #[tokio::test]
    async fn invalid_primary_uses_freshly_verified_fallback_before_one_raw_post() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let events = recording_events();
        let result = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| {
                let mut transport = Recording::new(candidate, &events);
                transport.invalid_nonce = candidate.url == PRIMARY;
                Ok(transport)
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), WitnessTrustError::WitnessUnreachable);
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                format!("probe:{PRIMARY}"),
                format!("probe:{FALLBACK}"),
                format!("post:{FALLBACK}")
            ]
        );
    }
    #[tokio::test]
    async fn raw_post_failure_never_retries_another_endpoint_in_the_invocation() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let events = recording_events();
        let result = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| Ok(Recording::new(candidate, &events)),
        )
        .await;
        assert_eq!(result.unwrap_err(), WitnessTrustError::WitnessUnreachable);
        assert_eq!(
            *events.lock().unwrap(),
            vec![format!("probe:{PRIMARY}"), format!("post:{PRIMARY}")]
        );
    }
    #[tokio::test]
    async fn pins_are_never_crossed_between_endpoints_or_inherited_from_signup() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        for (address, measurement) in [(NEW_SIGNER, "cc"), (OLD_SIGNER, "bb"), (OLD_SIGNER, "aa")] {
            let events = recording_events();
            let result = session_with(
                RoutingContext::test(&settings(), dir.path()),
                crate::envelope::MAX_ENVELOPE_BYTES,
                raw(),
                None,
                &consent(),
                |candidate| {
                    let mut transport = Recording::new(candidate, &events);
                    transport.address = address.into();
                    transport.measurement = measurement.repeat(48);
                    Ok(transport)
                },
            )
            .await;
            assert!(result.is_err());
            assert!(
                events
                    .lock()
                    .unwrap()
                    .iter()
                    .all(|event| event.starts_with("probe:"))
            );
        }
    }
    #[tokio::test]
    async fn cache_tamper_and_withdrawal_during_probe_prevent_raw_transmission() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let events = recording_events();
        let mut withdrawal = document(2);
        withdrawal.endpoints.clear();
        let bytes = signed(&key, &withdrawal);
        let result = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| {
                let mut transport = Recording::new(candidate, &events);
                transport.replace = Some((dir.path().into(), bytes.clone()));
                Ok(transport)
            },
        )
        .await;
        assert_eq!(result.unwrap_err(), WitnessTrustError::WitnessUnreachable);
        assert_eq!(*events.lock().unwrap(), vec![format!("probe:{PRIMARY}")]);
        let withdrawn = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |_| -> Result<Recording, _> { panic!("authoritative withdrawal cannot probe") },
        )
        .await;
        assert_eq!(withdrawn.unwrap_err(), unpinned());
        std::fs::write(dir.path().join(CONFIGURATION_FILE), b"tampered").unwrap();
        let result = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |_| -> Result<Recording, _> { panic!("invalid cache must never use legacy pins") },
        )
        .await;
        assert_eq!(result.unwrap_err(), unpinned());
    }
    #[tokio::test]
    async fn a_pin_expiring_during_attestation_cannot_authorize_raw_bytes() {
        let (dir, key) = setup();
        let mut config = document(1);
        config.endpoints.truncate(1);
        config.endpoints[0].entries[0].not_after = Utc::now() + chrono::Duration::milliseconds(250);
        install_document(dir.path(), &key, &config);
        let events = recording_events();
        let result = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| {
                let mut t = Recording::new(candidate, &events);
                t.delay = true;
                Ok(t)
            },
        )
        .await;
        let had_probe = !events.lock().unwrap().is_empty();
        assert_eq!(
            result.unwrap_err(),
            if had_probe {
                WitnessTrustError::WitnessUnreachable
            } else {
                unpinned()
            }
        );
        let expired = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |_| -> Result<Recording, _> { panic!("already expired authority cannot probe") },
        )
        .await;
        assert_eq!(expired.unwrap_err(), unpinned());
        assert!(
            events
                .lock()
                .unwrap()
                .iter()
                .all(|event| event.starts_with("probe:"))
        );
    }
    #[tokio::test]
    async fn unpinned_and_oversized_submissions_never_even_build_a_probe() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let mut unpinned_settings = settings();
        unpinned_settings.expected_measurements.clear();
        for (settings, limit, expected) in [
            (
                unpinned_settings,
                crate::envelope::MAX_ENVELOPE_BYTES,
                unpinned(),
            ),
            (settings(), 1, WitnessTrustError::WitnessPayloadTooLarge),
        ] {
            let result = session_with(
                RoutingContext::test(&settings, dir.path()),
                limit,
                raw(),
                None,
                &consent(),
                |_| -> Result<Recording, _> { panic!("preconditions before network") },
            )
            .await;
            assert_eq!(result.unwrap_err(), expected);
        }
    }
    fn token_call() -> crate::routing::attested::AttestedCall {
        use sha2::{Digest, Sha256};
        use trace_commons_protocol::evidence_import::*;
        crate::routing::attested::AttestedCall::from_import(&ImportedInference {
            exchange_id: "exchange".into(),
            coverage: InferenceCoverage::FinalCallOnly,
            request_body: "{}".into(),
            response_body: "{}".into(),
            request_sha256: hex::encode(Sha256::digest(b"{}")),
            response_sha256: hex::encode(Sha256::digest(b"{}")),
            upstream_id: "upstream".into(),
            served_model: None,
            status: 200,
            timestamp: Utc::now(),
            receipt: ImportedReceipt {
                text: "receipt".into(),
                signature: "00".into(),
                signing_address: OLD_SIGNER.into(),
                signing_algo: "ecdsa".into(),
                signature_kind: "unrecognised".into(),
            },
        })
        .unwrap()
    }
    fn token_receipt() -> trace_commons_attestation::receipt::ReceiptPayload {
        use trace_commons_attestation::receipt::*;
        ReceiptPayload {
            text: "receipt".into(),
            signature: "00".into(),
            signing_address: OLD_SIGNER.into(),
            signing_algo: ReceiptAlgo::Ecdsa,
            signature_kind: ReceiptSignatureKind::Unrecognised,
        }
    }
    #[tokio::test]
    async fn token_bundles_use_signed_routing_and_do_not_failover_after_raw_post() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let events = recording_events();
        let settings = settings();
        let trust = settings.trust().unwrap();
        let call = token_call();
        let receipt = token_receipt();
        let granted = consent();
        let request = super::super::TokenBundleSession {
            url: ANCHOR,
            trust: &trust,
            now_unix: 0,
            raw: raw(),
            attested: AttestedInference {
                call: &call,
                receipt: Some(&receipt),
            },
            granted: &granted,
            options: transport::TokenBundleRequest {
                capture_store_id: "store".into(),
                capture_id: "capture".into(),
                bundle_revision: "revision".into(),
                restricted_token_consent: true,
            },
        };
        let result = token_session_with(
            RoutingContext::test(&settings, dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            request,
            |candidate| {
                let mut t = Recording::new(candidate, &events);
                t.invalid_nonce = candidate.url == PRIMARY;
                Ok(t)
            },
        )
        .await;
        assert!(matches!(
            result,
            Err(WitnessTrustError::WitnessSaturated { .. })
        ));
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                format!("probe:{PRIMARY}"),
                format!("probe:{FALLBACK}"),
                format!("token-post:{FALLBACK}")
            ]
        );
    }
    #[tokio::test]
    async fn token_bundles_require_restricted_consent_before_any_probe() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let settings = settings();
        let trust = settings.trust().unwrap();
        let call = token_call();
        let receipt = token_receipt();
        let granted = consent();
        for (restricted, receipt) in [(false, Some(&receipt)), (true, None)] {
            let request = super::super::TokenBundleSession {
                url: ANCHOR,
                trust: &trust,
                now_unix: 0,
                raw: raw(),
                attested: AttestedInference {
                    call: &call,
                    receipt,
                },
                granted: &granted,
                options: transport::TokenBundleRequest {
                    capture_store_id: "store".into(),
                    capture_id: "capture".into(),
                    bundle_revision: "revision".into(),
                    restricted_token_consent: restricted,
                },
            };
            let result = token_session_with(
                RoutingContext::test(&settings, dir.path()),
                crate::envelope::MAX_ENVELOPE_BYTES,
                request,
                |_| -> Result<Recording, _> { panic!("token consent gate before probe") },
            )
            .await;
            assert!(matches!(
                result,
                Err(WitnessTrustError::WitnessAdmissionEvidenceRefused)
            ));
        }
    }
    #[test]
    fn corrupt_cache_cannot_be_repaired_by_replaying_an_older_signed_document() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(8));
        std::fs::write(dir.path().join(CONFIGURATION_FILE), b"corrupt-cache").unwrap();
        assert_eq!(
            install(
                dir.path(),
                &signed(&key, &document(1)),
                &[public(&key)],
                ANCHOR
            ),
            Err(ERR_STORE)
        );
        assert_eq!(
            std::fs::read(dir.path().join(CONFIGURATION_FILE)).unwrap(),
            b"corrupt-cache"
        );
    }
    async fn serve_config(
        bytes: Option<Vec<u8>>,
        seen: Arc<Mutex<Vec<axum::http::HeaderMap>>>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{
            Router,
            http::{HeaderMap, StatusCode, Uri},
            routing::get,
        };
        let app = Router::new().route(
            trace_commons_protocol::witness_configuration::WITNESS_CONFIGURATION_PATH,
            get(move |uri: Uri, headers: HeaderMap| {
                assert!(
                    uri.query().is_none(),
                    "global fixed path has no enrollment selector"
                );
                seen.lock().unwrap().push(headers);
                let bytes = bytes.clone();
                async move {
                    match bytes {
                        Some(bytes) => (StatusCode::OK, bytes),
                        None => (StatusCode::NOT_FOUND, Vec::new()),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        let task = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (origin, task)
    }
    #[tokio::test]
    async fn anonymous_fixed_path_refresh_installs_config_independently_of_legacy_and_keeps_it_on_404()
     {
        let (dir, key) = setup();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let bytes = signed(&key, &document(19));
        let (origin, task) = serve_config(Some(bytes.clone()), seen.clone()).await;
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = format!("{origin}/v1/traces");
        cfg.allowed_hosts = Some("127.0.0.1".into());
        cfg.witness = Some(settings());
        let settings_before = cfg.witness.clone();
        let now = Utc::now();
        let mut schedule = accepted_list::ListRefreshSchedule::starting(now);
        // The old list route is 404, but a newer signed routing document releases holds.
        assert!(
            accepted_list::refresh_for_tick(&cfg, dir.path(), &mut schedule, now, 0.5, true).await
        );
        assert!(!schedule.due(now + chrono::Duration::hours(23)));
        assert_eq!(cfg.witness, settings_before);
        assert_eq!(
            load_verified(dir.path(), &[public(&key)]).unwrap().version,
            19
        );
        assert!(accepted_list::load_verified(dir.path(), &[public(&key)]).is_none());
        {
            let headers = seen.lock().unwrap();
            assert_eq!(headers.len(), 1);
            assert!(!headers[0].contains_key("authorization"));
            assert!(!headers[0].contains_key("cookie"));
        }
        task.abort();
        let (origin, task) = serve_config(None, seen).await;
        cfg.ingest_url = origin;
        assert_eq!(
            accepted_list::refresh(&cfg, dir.path()).await,
            RefreshOutcome::Failed
        );
        assert_eq!(
            std::fs::read(dir.path().join(CONFIGURATION_FILE)).unwrap(),
            bytes
        );
        assert_eq!(
            candidates(&settings(), dir.path(), Utc::now()).unwrap()[0].url,
            PRIMARY
        );
        task.abort();
    }
    #[tokio::test]
    async fn enforcing_operator_policy_rejects_signed_hosts_before_raw_or_attestation() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.witness = Some(settings());
        cfg.allowed_hosts = Some("127.0.0.1,enrollment.example".into());
        let options = TransportOptions {
            admission: false,
            background: false,
            envelope_limit: crate::envelope::MAX_ENVELOPE_BYTES,
        };
        let error = witness_session(
            &cfg,
            &settings(),
            dir.path(),
            options,
            raw(),
            None,
            &consent(),
        )
        .await
        .unwrap_err();
        assert_eq!(error, WitnessTrustError::WitnessHostNotAllowed);
    }
    #[tokio::test]
    async fn a_signed_withdrawal_while_polling_blocks_a_second_raw_job_post() {
        use axum::{
            Router,
            http::StatusCode,
            routing::{get, post},
        };
        use std::sync::atomic::{AtomicUsize, Ordering};
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let snapshot = read_cache(dir.path()).unwrap();
        let candidate = candidates(&settings(), dir.path(), Utc::now())
            .unwrap()
            .remove(0);
        let trust = candidate.trust_at(Utc::now()).unwrap();
        let guard = RoutingGuard {
            dir: dir.path().into(),
            snapshot,
            candidate,
            trust,
        };
        let mut withdrawal = document(2);
        withdrawal.endpoints.clear();
        let withdrawn_bytes = signed(&key, &withdrawal);
        let keys = [public(&key)];
        let posts = Arc::new(AtomicUsize::new(0));
        let polls = Arc::new(AtomicUsize::new(0));
        let app = Router::new()
            .route("/v1/witness/jobs", post({
                let posts = posts.clone();
                move |body: axum::body::Bytes| {
                    assert_eq!(body.as_ref(), b"private-raw"); posts.fetch_add(1, Ordering::SeqCst);
                    async { (StatusCode::ACCEPTED, axum::Json(serde_json::json!({"job_id":"11111111111111111111111111111111", "poll_after_secs":1, "queue_position":0}))) }
                }
            }))
            .route("/v1/witness/jobs/{job_id}", get({
                let dir = dir.path().to_path_buf(); let polls = polls.clone();
                move || {
                    polls.fetch_add(1, Ordering::SeqCst);
                    install(&dir, &withdrawn_bytes, &keys, ANCHOR).unwrap();
                    async { (StatusCode::NOT_FOUND, axum::Json(serde_json::json!({"error":"witness_job_gone"}))) }
                }
            }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        // Only the DCAP capability is a test fixture; job POST/poll traffic
        // and the concurrent signed withdrawal use their real implementations.
        let transport = HttpWitnessTransport::new(
            url.clone(),
            url.clone(),
            Arc::new(HostAllowlist::permissive()),
            std::time::Duration::from_secs(5),
        )
        .unwrap()
        .with_routing_guard(guard);
        let verified = super::super::verify::verified_witness_for_test(&url, NEW_SIGNER);
        let result = transport.witness(&verified, b"private-raw").await;
        server.abort();
        assert_eq!(
            result.unwrap_err(),
            WitnessTrustError::WitnessUnreachable,
            "a post-send refusal must not look like a pre-send refresh opportunity"
        );
        assert_eq!(
            posts.load(Ordering::SeqCst),
            1,
            "withdrawal forbids raw resubmission"
        );
        assert_eq!(polls.load(Ordering::SeqCst), 1);
    }
    #[tokio::test]
    async fn a_publication_during_successful_probe_retries_then_uses_current_policy() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let (answer, new_signer) = transport::signed_fixture(b"{}".to_vec());
        let mut next = document(2);
        next.endpoints.truncate(1);
        next.endpoints[0].entries[0].signing_address = new_signer;
        let next_bytes = signed(&key, &next);
        let events = recording_events();
        let first = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| {
                let mut transport = Recording::new(candidate, &events);
                transport.replace = Some((dir.path().into(), next_bytes.clone()));
                Ok(transport)
            },
        )
        .await;
        assert_eq!(first.unwrap_err(), WitnessTrustError::WitnessUnreachable);
        assert_eq!(
            *events.lock().unwrap(),
            vec![format!("probe:{PRIMARY}")],
            "no raw POST under the superseded policy"
        );
        // The supervisor has already observed V2. Another identical refresh
        // will not release a newly created measurement hold, so this outcome
        // must be a transient retry rather than a measurement hold.
        let mut schedule = accepted_list::ListRefreshSchedule::starting(Utc::now());
        assert!(schedule.configuration_changed(active_state(&settings(), dir.path(), Utc::now())));
        assert!(!schedule.configuration_changed(active_state(&settings(), dir.path(), Utc::now())));
        let second = session_with(
            RoutingContext::test(&settings(), dir.path()),
            crate::envelope::MAX_ENVELOPE_BYTES,
            raw(),
            None,
            &consent(),
            |candidate| {
                let mut transport = Recording::new(candidate, &events);
                transport.success = Some(answer.clone());
                Ok(transport)
            },
        )
        .await
        .expect("a new invocation freshly attests and sends under V2");
        assert_eq!(second.envelope_bytes, b"{}");
        assert_eq!(
            *events.lock().unwrap(),
            vec![
                format!("probe:{PRIMARY}"),
                format!("probe:{PRIMARY}"),
                format!("post:{PRIMARY}")
            ]
        );
    }

    #[test]
    fn routing_guard_policy_transitions_are_transient_but_invalid_cache_is_unpinned() {
        let (dir, key) = setup();
        install_document(dir.path(), &key, &document(1));
        let candidate = candidates(&settings(), dir.path(), Utc::now())
            .unwrap()
            .remove(0);
        let trust = candidate.trust_at(Utc::now()).unwrap();
        let guard = RoutingGuard {
            dir: dir.path().into(),
            snapshot: read_cache(dir.path()).unwrap(),
            candidate: candidate.clone(),
            trust: trust.clone(),
        };
        let mut expired_candidate = candidate;
        expired_candidate.entries[0].not_after = Utc::now() - chrono::Duration::seconds(1);
        let expired = RoutingGuard {
            dir: dir.path().into(),
            snapshot: read_cache(dir.path()).unwrap(),
            candidate: expired_candidate,
            trust,
        };
        assert_eq!(
            expired.validate(),
            Err(WitnessTrustError::WitnessUnreachable)
        );
        let mut withdrawal = document(2);
        withdrawal.endpoints.clear();
        install_document(dir.path(), &key, &withdrawal);
        assert_eq!(guard.validate(), Err(WitnessTrustError::WitnessUnreachable));
        std::fs::write(dir.path().join(CONFIGURATION_FILE), b"tampered").unwrap();
        assert_eq!(guard.validate(), Err(unpinned()));
    }
}
