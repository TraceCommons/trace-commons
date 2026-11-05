//! The signed witness accepted-measurements list on this machine: which keys
//! may sign it, where the verified copy is kept, when it is refreshed, and
//! how it widens the trust the sign-up pins give.
//!
//! Spec 2026-11-05 (witness at submission), section 7, and owner decisions
//! Q10 and Q11 (2026-11-05). The document format and its verification are in
//! `trace_commons_protocol::witness_accepted_list`.
//!
//! - **Trust.** A list is used only if a signature over it verifies under a
//!   key in [`TRUSTED_LIST_SIGNING_KEYS`]. A witness is then admitted if it
//!   matches the sign-up pins as before, **or** any list entry valid at the
//!   time of the check, measurement and signing address together
//!   ([`trust_with_accepted`]).
//! - **Never weaker.** Fetch failure, a bad or missing signature, a lower or
//!   equal version, a list for another witness URL, or a malformed
//!   measurement: the stored list stays as it was, and only a fixed label is
//!   logged. A list never makes an unpinned client pinned.
//! - **Outside consent (Q10).** The list lives in its own file, never in
//!   `WitnessSettings`, so it is not part of the witness identity digest, the
//!   approval input fingerprint or the grant-terms void rule. A refresh
//!   re-asks nothing; a change of witness URL still does, through
//!   `WitnessSettings` as before.
//! - **Refreshed** at daemon start, about daily after that
//!   ([`ListRefreshSchedule`]), and when a witness is refused for its
//!   measurement ([`refresh_on_refusal`]). A build with no trusted key never
//!   fetches.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use chrono::{DateTime, Duration, Utc};
use trace_commons_attestation::measurements::{ExpectedMeasurements, ExpectedMeasurementsError};
use trace_commons_protocol::witness_accepted_list::{
    AcceptedList, WITNESS_ACCEPTED_LIST_MAX_BYTES, accepts_version, parse_public_key_hex,
    verify_signed_list,
};

use super::{AcceptedWitnessPin, WitnessTrust};
use crate::config::{ContributorConfig, WitnessSettings};

/// The Ed25519 public keys whose signature on an accepted-measurements list
/// this build trusts, each as 64 lowercase hex characters (the raw 32-byte
/// key).
///
/// **Empty on purpose until the owner creates the list-signing key** (a
/// Google Cloud KMS `ec-sign-ed25519` key, owner decision Q11). With no key
/// nothing can verify, the client never fetches, and the feature is inert:
/// the sign-up pins are the whole of the trust, exactly as before.
///
/// To add a key, export the KMS key version's public key and convert it with
/// `trace-commons-witness-list public-key` (runbook:
/// `docs/operator/witness-accepted-list.md`). Rotation: add the new key here
/// and ship it to clients **before** the list is signed by it alone; remove
/// the old one only after that. Keep this set small.
pub const TRUSTED_LIST_SIGNING_KEYS: &[&str] = &[];

/// The file the verified served document is kept in, in the state directory.
/// It holds the exact bytes served and is verified again every time it is
/// read, so a copy edited on disk widens nothing.
pub const ACCEPTED_LIST_FILE: &str = "witness-accepted-list.json";

/// How often the list is fetched after the first fetch at start-up.
pub const ACCEPTED_LIST_REFRESH: Duration = Duration::hours(24);

/// The random offset added to every scheduled fetch after the first, so
/// fetches from many machines spread out.
pub const ACCEPTED_LIST_REFRESH_JITTER: Duration = Duration::hours(1);

/// The least time between two fetches asked for by a measurement refusal,
/// per state directory, so a witness that keeps refusing cannot turn every
/// upload attempt into a request.
pub const REFUSAL_REFRESH_SPACING: Duration = Duration::minutes(10);

/// Log labels. Fixed strings only; no URL, path or document content.
pub const LOG_INSTALLED: &str = "witness-accepted-list-installed";
pub const LOG_NOT_NEWER: &str = "witness-accepted-list-not-newer";
pub const LOG_FETCH_FAILED: &str = "witness-accepted-list-fetch-failed";
pub const LOG_STORE_FAILED: &str = "witness-accepted-list-store-failed";
/// The served list names a different witness URL from the configured one.
pub const ERR_URL_MISMATCH: &str = "witness-accepted-list-url-mismatch";
/// A signed entry's measurement set does not parse with the pin parser.
pub const ERR_MEASUREMENT_MALFORMED: &str = "witness-accepted-list-measurement-malformed";

/// The trusted list-signing keys in force.
///
/// [`TRUSTED_LIST_SIGNING_KEYS`] in a shipped build. Tests may register keys
/// for one state directory (`set_test_keys`); nothing else can add one.
pub fn trusted_keys(dir: &Path) -> Vec<[u8; 32]> {
    #[cfg(test)]
    if let Some(keys) = test_keys().lock().expect("test keys").get(dir) {
        return keys.clone();
    }
    let _ = dir;
    TRUSTED_LIST_SIGNING_KEYS
        .iter()
        .filter_map(|key| parse_public_key_hex(key))
        .collect()
}

#[cfg(test)]
fn test_keys() -> &'static Mutex<HashMap<PathBuf, Vec<[u8; 32]>>> {
    static KEYS: OnceLock<Mutex<HashMap<PathBuf, Vec<[u8; 32]>>>> = OnceLock::new();
    KEYS.get_or_init(Default::default)
}

/// Trust `keys` for lists in `dir`, in tests only.
#[cfg(test)]
pub(crate) fn set_test_keys(dir: &Path, keys: Vec<[u8; 32]>) {
    test_keys()
        .lock()
        .expect("test keys")
        .insert(dir.to_path_buf(), keys);
}

/// What an install did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallOutcome {
    /// The document was verified and stored; it carries this version.
    Installed { version: u64 },
    /// Verified, but its version is not higher than the stored one, so the
    /// stored list stays.
    NotNewer,
}

/// The stored list, verified again under `keys`. `None` when there is no
/// file, it cannot be read, or it no longer verifies (a key was removed
/// from this build, or the file was edited).
pub fn load_verified(dir: &Path, keys: &[[u8; 32]]) -> Option<AcceptedList> {
    if keys.is_empty() {
        return None;
    }
    let bytes = read_bounded(&dir.join(ACCEPTED_LIST_FILE))?;
    verify_signed_list(&bytes, keys).ok()
}

fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    use std::io::Read as _;
    let file = std::fs::File::open(path).ok()?;
    let mut bytes = Vec::new();
    file.take(WITNESS_ACCEPTED_LIST_MAX_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= WITNESS_ACCEPTED_LIST_MAX_BYTES).then_some(bytes)
}

/// Verify a served document and, if it is newer than the stored one, store
/// it.
///
/// Refused, leaving the stored list as it was, when: no signature verifies
/// under `keys`; it names a witness URL other than `configured_url`; any
/// entry's measurement set does not parse (the whole list, never a skipped
/// line, as for the sign-up pins). A verified list whose version is not
/// higher than the stored one is [`InstallOutcome::NotNewer`].
pub fn install(
    dir: &Path,
    document: &[u8],
    keys: &[[u8; 32]],
    configured_url: &str,
) -> Result<InstallOutcome, &'static str> {
    let list = verify_signed_list(document, keys).map_err(|error| error.label())?;
    if !list.names_witness(configured_url) {
        return Err(ERR_URL_MISMATCH);
    }
    parse_entries(&list).map_err(|_| ERR_MEASUREMENT_MALFORMED)?;
    let held = load_verified(dir, keys).map(|held| held.version);
    if !accepts_version(held, list.version) {
        return Ok(InstallOutcome::NotNewer);
    }
    crate::config::write_atomic_0600(dir, &dir.join(ACCEPTED_LIST_FILE), document)
        .map_err(|_| LOG_STORE_FAILED)?;
    Ok(InstallOutcome::Installed {
        version: list.version,
    })
}

fn parse_entries(
    list: &AcceptedList,
) -> Result<Vec<ExpectedMeasurements>, ExpectedMeasurementsError> {
    list.entries
        .iter()
        .map(|entry| {
            ExpectedMeasurements::from_env_value(Some(&entry.measurements))?
                .ok_or(ExpectedMeasurementsError::NoPins)
        })
        .collect()
}

/// The trust the sign-up pins give ([`WitnessSettings::trust`]), plus every
/// stored list entry valid at `now` for this witness URL.
///
/// The sign-up half is unchanged, errors included. A stored list that does
/// not verify, names another witness, or holds a measurement that does not
/// parse adds nothing.
pub fn trust_with_accepted(
    settings: &WitnessSettings,
    dir: &Path,
    now: DateTime<Utc>,
) -> Result<WitnessTrust, ExpectedMeasurementsError> {
    let mut trust = settings.trust()?;
    trust.accepted = accepted_pins(settings, dir, &trusted_keys(dir), now);
    Ok(trust)
}

fn accepted_pins(
    settings: &WitnessSettings,
    dir: &Path,
    keys: &[[u8; 32]],
    now: DateTime<Utc>,
) -> Vec<AcceptedWitnessPin> {
    let Some(list) = load_verified(dir, keys) else {
        return Vec::new();
    };
    if !list.names_witness(&settings.url) {
        return Vec::new();
    }
    let Ok(parsed) = parse_entries(&list) else {
        return Vec::new();
    };
    list.entries
        .iter()
        .zip(parsed)
        .filter(|(entry, _)| entry.is_valid_at(now))
        .map(|(entry, measurements)| AcceptedWitnessPin {
            signing_address: entry.signing_address.clone(),
            measurements,
        })
        .collect()
}

/// When the next scheduled fetch is due: at daemon start, then every
/// [`ACCEPTED_LIST_REFRESH`] plus a random offset. Kept as a local of the
/// daemon's supervisor loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListRefreshSchedule {
    next_at: DateTime<Utc>,
    /// The stored list's version when last looked at, so a change is seen
    /// whichever path installed it.
    seen_version: Option<u64>,
}

impl ListRefreshSchedule {
    #[must_use]
    pub fn starting(started_at: DateTime<Utc>) -> Self {
        Self {
            next_at: started_at,
            seen_version: None,
        }
    }

    /// Record the stored list's `version` and say whether it is a list this
    /// schedule has not seen: a newer one, or the first one seen since the
    /// daemon started (a list installed while it was stopped, by the `submit`
    /// command, counts). `None`, no verified list, is never a change.
    pub fn list_version_changed(&mut self, version: Option<u64>) -> bool {
        let changed = version.is_some() && version != self.seen_version;
        if version.is_some() {
            self.seen_version = version;
        }
        changed
    }

    #[must_use]
    pub fn due(&self, now: DateTime<Utc>) -> bool {
        now >= self.next_at
    }

    /// Record an attempt, whatever its outcome. `draw` is in `[0, 1)`.
    pub fn attempted(&mut self, now: DateTime<Utc>, draw: f64) {
        let draw = if draw.is_finite() {
            draw.clamp(0.0, 1.0)
        } else {
            0.0
        };
        let jitter = ACCEPTED_LIST_REFRESH_JITTER.num_seconds() as f64 * draw;
        self.next_at = now + ACCEPTED_LIST_REFRESH + Duration::seconds(jitter as i64);
    }
}

/// What one refresh did, for tests and the caller's decision to retry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefreshOutcome {
    /// No witness configured, or this build trusts no key: nothing fetched.
    Skipped,
    /// A newer list was stored.
    Installed { version: u64 },
    /// Fetched and verified, not newer than the stored one.
    NotNewer,
    /// Fetch failed or the document was refused; the stored list stays.
    Failed,
}

/// Fetch the list from the configured ingest origin and install it.
///
/// Anonymous, no redirects, the host allowlist and a bounded read, through
/// the same client as the other public documents. Logs one fixed label.
pub async fn refresh(cfg: &ContributorConfig, dir: &Path) -> RefreshOutcome {
    let Some(witness) = cfg.witness.as_ref() else {
        return RefreshOutcome::Skipped;
    };
    let keys = trusted_keys(dir);
    if keys.is_empty() {
        return RefreshOutcome::Skipped;
    }
    let fetched = match crate::config::ingest_origin_url(&cfg.ingest_url, "/") {
        Ok(origin) => {
            match crate::credit_estimate_table::EstimateTableClient::new(
                origin.as_str(),
                cfg.allowed_hosts.as_deref(),
            ) {
                Ok(client) => client.fetch_witness_accepted_list().await.ok(),
                Err(_) => None,
            }
        }
        Err(_) => None,
    };
    let Some(document) = fetched else {
        tracing::info!(event = LOG_FETCH_FAILED, "witness accepted list");
        return RefreshOutcome::Failed;
    };
    match install(dir, &document, &keys, &witness.url) {
        Ok(InstallOutcome::Installed { version }) => {
            tracing::info!(event = LOG_INSTALLED, version, "witness accepted list");
            RefreshOutcome::Installed { version }
        }
        Ok(InstallOutcome::NotNewer) => {
            tracing::debug!(event = LOG_NOT_NEWER, "witness accepted list");
            RefreshOutcome::NotNewer
        }
        Err(label) => {
            tracing::warn!(event = label, "witness accepted list refused");
            RefreshOutcome::Failed
        }
    }
}

fn refusal_attempts() -> &'static Mutex<HashMap<PathBuf, DateTime<Utc>>> {
    static ATTEMPTS: OnceLock<Mutex<HashMap<PathBuf, DateTime<Utc>>>> = OnceLock::new();
    ATTEMPTS.get_or_init(Default::default)
}

/// Whether a refusal-driven fetch may run at `now` for `dir`, recording it
/// if so: at most one per [`REFUSAL_REFRESH_SPACING`].
fn take_refusal_slot(dir: &Path, now: DateTime<Utc>) -> bool {
    let mut attempts = refusal_attempts().lock().expect("refusal attempts");
    match attempts.get(dir) {
        Some(last) if now < *last + REFUSAL_REFRESH_SPACING => false,
        _ => {
            attempts.insert(dir.to_path_buf(), now);
            true
        }
    }
}

/// Called when a witness is refused for its measurement, before the refusal
/// is reported: fetch the list once, within [`REFUSAL_REFRESH_SPACING`] of
/// the last such fetch for this state directory. True only when a newer list
/// was stored, which is the only case in which trying the witness again can
/// reach a different answer.
pub async fn refresh_on_refusal(cfg: &ContributorConfig, dir: &Path, now: DateTime<Utc>) -> bool {
    if trusted_keys(dir).is_empty() || cfg.witness.is_none() || !take_refusal_slot(dir, now) {
        return false;
    }
    matches!(refresh(cfg, dir).await, RefreshOutcome::Installed { .. })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone as _;
    use ring::signature::{Ed25519KeyPair, KeyPair as _};
    use trace_commons_protocol::witness_accepted_list::{
        AcceptedEntry, ListSignature, WITNESS_ACCEPTED_LIST_SCHEMA, assemble_signed_list,
        key_id_for,
    };

    pub(crate) const WITNESS_URL: &str = "https://witness.example";
    const SIGNUP_ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    const NEW_ADDRESS: &str = "0x2222222222222222222222222222222222222222";

    fn at(day: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, day, 0, 0, 0).unwrap()
    }

    fn key() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).unwrap()
    }

    fn public(key: &Ed25519KeyPair) -> [u8; 32] {
        key.public_key().as_ref().try_into().unwrap()
    }

    fn pin(mrconfigid: &str) -> String {
        format!(
            "mrtd={},mrconfigid={}",
            "aa".repeat(48),
            mrconfigid.repeat(48)
        )
    }

    fn settings() -> WitnessSettings {
        WitnessSettings {
            admission_evidence: false,
            url: WITNESS_URL.into(),
            signing_address: SIGNUP_ADDRESS.into(),
            expected_measurements: vec![pin("bb")],
        }
    }

    fn list(version: u64, entries: Vec<AcceptedEntry>) -> AcceptedList {
        AcceptedList {
            schema: WITNESS_ACCEPTED_LIST_SCHEMA.into(),
            version,
            issued_at: at(2),
            witness_url: WITNESS_URL.into(),
            entries,
        }
    }

    fn entry(mrconfigid: &str, from: u32, until: u32) -> AcceptedEntry {
        AcceptedEntry {
            measurements: pin(mrconfigid),
            signing_address: NEW_ADDRESS.into(),
            not_before: at(from),
            not_after: at(until),
            commit: None,
        }
    }

    fn signed(key: &Ed25519KeyPair, list: &AcceptedList) -> Vec<u8> {
        // Signed by hand so a test can sign what the builder would refuse.
        let value = serde_json::to_value(list).unwrap();
        let payload = trace_commons_protocol::canonical_json::to_canonical_string(&value).unwrap();
        let signature = ListSignature {
            key_id: key_id_for(&public(key)),
            signature: hex::encode(key.sign(payload.as_bytes()).as_ref()),
        };
        assemble_signed_list(payload, vec![signature])
    }

    #[test]
    fn this_build_ships_no_list_signing_key_so_the_feature_is_inert() {
        // Remove this assertion in the change that adds the owner's key.
        assert!(TRUSTED_LIST_SIGNING_KEYS.is_empty());
        let dir = tempfile::tempdir().unwrap();
        assert!(trusted_keys(dir.path()).is_empty());
        // A correctly signed list is refused without a trusted key.
        let signer = key();
        let document = signed(&signer, &list(1, vec![entry("cc", 1, 20)]));
        assert_eq!(
            install(
                dir.path(),
                &document,
                &trusted_keys(dir.path()),
                WITNESS_URL
            ),
            Err("witness-accepted-list-signature-invalid")
        );
    }

    #[test]
    fn every_compiled_in_key_parses() {
        for key in TRUSTED_LIST_SIGNING_KEYS {
            assert!(parse_public_key_hex(key).is_some(), "a malformed key");
        }
    }

    #[test]
    fn a_verified_list_widens_trust_with_its_valid_entries_only() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let document = signed(
            &signer,
            &list(
                1,
                vec![
                    entry("cc", 1, 20),
                    // Not yet valid on the 10th; stored, and live later.
                    entry("dd", 15, 25),
                    // Already expired on the 10th.
                    entry("ee", 1, 5),
                ],
            ),
        );
        assert_eq!(
            install(dir.path(), &document, &[public(&signer)], WITNESS_URL),
            Ok(InstallOutcome::Installed { version: 1 })
        );

        let trust = trust_with_accepted(&settings(), dir.path(), at(10)).unwrap();
        // The sign-up pin is untouched.
        assert_eq!(trust.signing_address, SIGNUP_ADDRESS);
        assert_eq!(trust.measurements, settings().trust().unwrap().measurements);
        let admitted: Vec<String> = trust
            .accepted
            .iter()
            .map(|pin| pin.measurements.to_pin_string())
            .collect();
        assert_eq!(admitted.len(), 1);
        assert!(admitted[0].contains(&"cc".repeat(48)));
        assert_eq!(trust.accepted[0].signing_address, NEW_ADDRESS);

        let later = trust_with_accepted(&settings(), dir.path(), at(16)).unwrap();
        assert_eq!(later.accepted.len(), 2, "the second entry is now live");
        let after = trust_with_accepted(&settings(), dir.path(), at(26)).unwrap();
        assert!(after.accepted.is_empty(), "every entry has expired");
    }

    #[test]
    fn a_list_never_makes_an_unpinned_client_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        install(
            dir.path(),
            &signed(&signer, &list(1, vec![entry("cc", 1, 20)])),
            &[public(&signer)],
            WITNESS_URL,
        )
        .unwrap();
        let mut unpinned = settings();
        unpinned.expected_measurements.clear();
        let trust = trust_with_accepted(&unpinned, dir.path(), at(10)).unwrap();
        assert_eq!(trust.accepted.len(), 1);
        assert!(!trust.is_pinned());
    }

    #[test]
    fn versions_only_move_up_and_a_refused_list_keeps_the_stored_one() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        let keys = [public(&signer)];
        let v2 = signed(&signer, &list(2, vec![entry("cc", 1, 20)]));
        assert_eq!(
            install(dir.path(), &v2, &keys, WITNESS_URL),
            Ok(InstallOutcome::Installed { version: 2 })
        );
        // Older and equal versions are not applied.
        let v1 = signed(&signer, &list(1, vec![entry("dd", 1, 20)]));
        assert_eq!(
            install(dir.path(), &v1, &keys, WITNESS_URL),
            Ok(InstallOutcome::NotNewer)
        );
        let v2_again = signed(&signer, &list(2, vec![entry("dd", 1, 20)]));
        assert_eq!(
            install(dir.path(), &v2_again, &keys, WITNESS_URL),
            Ok(InstallOutcome::NotNewer)
        );
        // Wrong key, other witness, malformed measurement: refused.
        let stranger = key();
        let v3_forged = signed(&stranger, &list(3, vec![entry("dd", 1, 20)]));
        assert!(install(dir.path(), &v3_forged, &keys, WITNESS_URL).is_err());
        let mut other = list(3, vec![entry("dd", 1, 20)]);
        other.witness_url = "https://other.example".into();
        assert_eq!(
            install(dir.path(), &signed(&signer, &other), &keys, WITNESS_URL),
            Err(ERR_URL_MISMATCH)
        );
        let mut malformed = entry("dd", 1, 20);
        malformed.measurements = "mrtd=not-hex".into();
        assert_eq!(
            install(
                dir.path(),
                &signed(&signer, &list(3, vec![entry("dd", 1, 20), malformed])),
                &keys,
                WITNESS_URL
            ),
            Err(ERR_MEASUREMENT_MALFORMED)
        );
        // The stored list is still v2, with its own entry.
        let held = load_verified(dir.path(), &keys).unwrap();
        assert_eq!(held.version, 2);
        assert_eq!(held.entries[0].measurements, pin("cc"));
    }

    #[test]
    fn a_newer_list_with_no_valid_entries_replaces_the_stored_one() {
        // That is how an operator withdraws an entry.
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let keys = [public(&signer)];
        install(
            dir.path(),
            &signed(&signer, &list(1, vec![entry("cc", 1, 20)])),
            &keys,
            WITNESS_URL,
        )
        .unwrap();
        install(
            dir.path(),
            &signed(&signer, &list(2, vec![])),
            &keys,
            WITNESS_URL,
        )
        .unwrap();
        let trust = trust_with_accepted(&settings(), dir.path(), at(10)).unwrap();
        assert!(trust.accepted.is_empty());
    }

    #[test]
    fn a_stored_list_adds_nothing_once_it_no_longer_verifies_or_names_this_witness() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let document = signed(&signer, &list(1, vec![entry("cc", 1, 20)]));
        install(dir.path(), &document, &[public(&signer)], WITNESS_URL).unwrap();

        // The witness URL changed: the list is for the old one.
        let mut moved = settings();
        moved.url = "https://other.example".into();
        assert!(
            trust_with_accepted(&moved, dir.path(), at(10))
                .unwrap()
                .accepted
                .is_empty()
        );

        // The file was edited on disk to add an entry.
        let path = dir.path().join(ACCEPTED_LIST_FILE);
        let edited = std::fs::read_to_string(&path)
            .unwrap()
            .replace(&"cc".repeat(48), &"ff".repeat(48));
        std::fs::write(&path, edited).unwrap();
        assert!(
            trust_with_accepted(&settings(), dir.path(), at(10))
                .unwrap()
                .accepted
                .is_empty()
        );
    }

    #[test]
    fn the_list_is_outside_the_witness_identity_and_approval_inputs() {
        // Q10: a signed refresh re-asks nothing. The witness identity digest
        // and the config are byte-identical before and after an install.
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.witness = Some(settings());
        store.save_config(&cfg).unwrap();
        let identity_before = settings().identity_sha256();
        let config_path = dir.path().join(crate::config::CONFIG_FILE);
        let config_before = std::fs::read(&config_path).unwrap();

        let signer = key();
        install(
            dir.path(),
            &signed(&signer, &list(1, vec![entry("cc", 1, 20)])),
            &[public(&signer)],
            WITNESS_URL,
        )
        .unwrap();

        let reloaded = store.load_config().unwrap().unwrap();
        assert_eq!(
            reloaded.witness.as_ref().unwrap().identity_sha256(),
            identity_before
        );
        assert_eq!(
            serde_json::to_value(&reloaded).unwrap(),
            serde_json::to_value(&cfg).unwrap()
        );
        assert_eq!(std::fs::read(&config_path).unwrap(), config_before);
    }

    #[test]
    fn the_schedule_fetches_at_start_then_daily_with_bounded_jitter() {
        let mut schedule = ListRefreshSchedule::starting(at(1));
        assert!(schedule.due(at(1)));
        schedule.attempted(at(1), 0.0);
        assert!(!schedule.due(at(1) + Duration::hours(23)));
        assert!(schedule.due(at(2)));
        schedule.attempted(at(2), 0.999);
        assert!(!schedule.due(at(3)));
        assert!(schedule.due(at(3) + ACCEPTED_LIST_REFRESH_JITTER));
        schedule.attempted(at(3), f64::NAN);
        assert!(schedule.due(at(4)));
    }

    /// The schedule remembers the stored list's version so the daemon can
    /// tell a list changed, whichever path installed it (the daily fetch, or
    /// a fetch on a refusal inside a submit). The first sight of a stored
    /// list counts as a change; seeing the same version again does not.
    #[test]
    fn the_schedule_reports_a_changed_list_version_once() {
        let mut schedule = ListRefreshSchedule::starting(at(1));
        assert!(!schedule.list_version_changed(None), "no list, nothing new");
        assert!(schedule.list_version_changed(Some(3)));
        assert!(!schedule.list_version_changed(Some(3)));
        assert!(schedule.list_version_changed(Some(4)));
        assert!(!schedule.list_version_changed(Some(4)));
    }

    #[test]
    fn refusal_fetches_are_spaced_per_state_directory() {
        let dir = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        assert!(take_refusal_slot(dir.path(), at(1)));
        assert!(!take_refusal_slot(dir.path(), at(1) + Duration::minutes(9)));
        assert!(take_refusal_slot(other.path(), at(1)));
        assert!(take_refusal_slot(
            dir.path(),
            at(1) + REFUSAL_REFRESH_SPACING
        ));
    }

    async fn serve(document: Option<Vec<u8>>) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, http::StatusCode, routing::get};
        let route = trace_commons_protocol::witness_accepted_list::WITNESS_ACCEPTED_LIST_PATH;
        let app = Router::new().route(
            route,
            get(move || {
                let document = document.clone();
                async move {
                    match document {
                        Some(bytes) => (StatusCode::OK, bytes),
                        None => (StatusCode::NOT_FOUND, Vec::new()),
                    }
                }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{address}"), handle)
    }

    fn cfg_for(origin: &str) -> ContributorConfig {
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = origin.to_string();
        cfg.witness = Some(settings());
        cfg
    }

    #[tokio::test]
    async fn a_refresh_fetches_verifies_and_stores_and_a_failure_keeps_the_pins() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let document = signed(&signer, &list(4, vec![entry("cc", 1, 20)]));
        let (origin, server) = serve(Some(document)).await;
        assert_eq!(
            refresh(&cfg_for(&origin), dir.path()).await,
            RefreshOutcome::Installed { version: 4 }
        );
        assert_eq!(
            refresh(&cfg_for(&origin), dir.path()).await,
            RefreshOutcome::NotNewer
        );
        server.abort();

        // The server is gone: the stored list stays in force.
        assert_eq!(
            refresh(&cfg_for(&origin), dir.path()).await,
            RefreshOutcome::Failed
        );
        let (missing, server) = serve(None).await;
        assert_eq!(
            refresh(&cfg_for(&missing), dir.path()).await,
            RefreshOutcome::Failed
        );
        server.abort();
        // An unsigned document is refused the same way.
        let (unsigned, server) =
            serve(Some(b"{\"payload\":\"{}\",\"signatures\":[]}".to_vec())).await;
        assert_eq!(
            refresh(&cfg_for(&unsigned), dir.path()).await,
            RefreshOutcome::Failed
        );
        server.abort();

        let trust = trust_with_accepted(&settings(), dir.path(), at(10)).unwrap();
        assert_eq!(trust.accepted.len(), 1);
        assert!(trust.is_pinned());
    }

    #[tokio::test]
    async fn a_build_without_a_key_never_fetches() {
        let dir = tempfile::tempdir().unwrap();
        // An origin nothing listens on: a fetch would fail, not skip.
        let cfg = cfg_for("http://127.0.0.1:9");
        assert_eq!(refresh(&cfg, dir.path()).await, RefreshOutcome::Skipped);
        assert!(!refresh_on_refusal(&cfg, dir.path(), at(1)).await);
    }

    #[tokio::test]
    async fn a_refusal_refresh_reports_only_a_newer_install() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let document = signed(&signer, &list(1, vec![entry("cc", 1, 20)]));
        let (origin, server) = serve(Some(document)).await;
        let cfg = cfg_for(&origin);
        assert!(refresh_on_refusal(&cfg, dir.path(), at(1)).await);
        // Within the spacing: no fetch at all.
        assert!(!refresh_on_refusal(&cfg, dir.path(), at(1) + Duration::minutes(1)).await);
        // After it: fetched, but not newer, so nothing to retry for.
        assert!(!refresh_on_refusal(&cfg, dir.path(), at(1) + Duration::minutes(11)).await);
        server.abort();
    }
}
