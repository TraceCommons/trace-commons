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
/// - `ea396d6a7f0e867f`: the commons' list-signing key, version 1. A Google
///   Cloud KMS `ec-sign-ed25519` key, not exportable, with signing permission
///   for the owner's account only (owner decision Q11, created 2026-11-05).
///
/// The key id is the first 16 hex characters of the key's SHA-256
/// (`trace-commons-witness-list public-key`). To add a key, export the KMS
/// key version's public key and convert it with that command (runbook:
/// `docs/operator/witness-accepted-list.md`). Rotation: add the new key here
/// and ship it to clients **before** the list is signed by it alone; remove
/// the old one only after that. Keep this set small.
pub const TRUSTED_LIST_SIGNING_KEYS: &[&str] =
    &["fed526376edb29b3328cb6c4a0a1f6a2d6c1267641b705cd276c5adf21d0a523"];

/// The key ids of [`TRUSTED_LIST_SIGNING_KEYS`], in the same order. Pinned
/// by a test, so a key cannot change without its id changing here too.
pub const TRUSTED_LIST_SIGNING_KEY_IDS: &[&str] = &["ea396d6a7f0e867f"];

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
/// higher than the stored one is [`InstallOutcome::NotNewer`]. The version
/// highwater remains global to this state directory, including across witness
/// URL changes: coordinated lists must keep increasing versions across URLs,
/// including when rolling back to an earlier URL.
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

impl ListRefreshSchedule {
    /// Record a fetch that failed: try again after [`REFUSAL_REFRESH_SPACING`]
    /// rather than waiting out a full [`ACCEPTED_LIST_REFRESH`], so a daemon
    /// that started before a list was published (or while offline) recovers
    /// within minutes.
    pub fn failed(&mut self, now: DateTime<Utc>) {
        self.next_at = now + REFUSAL_REFRESH_SPACING;
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
/// the same client as the other public documents. The query selects the
/// configured witness URL; install still requires the signed document to name
/// that URL. Logs one fixed label.
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
                Ok(client) => client
                    .fetch_witness_accepted_list_for(&witness.url)
                    .await
                    .ok(),
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

/// One supervisor tick of the list refresh.
///
/// When the schedule is due, fetch; a failed fetch is retried after
/// [`REFUSAL_REFRESH_SPACING`], any other outcome waits the usual day. When it
/// is not due but a session is held for a measurement outside the accepted
/// set (`measurement_holds`), fetch early through [`refresh_on_refusal`]: the
/// same 10-minute spacing, so a backlog of held sessions cannot hammer the
/// ingest. Returns true when a newer list was stored. The caller releases the
/// holds when it sees the stored version change.
pub async fn refresh_for_tick(
    cfg: &ContributorConfig,
    dir: &Path,
    schedule: &mut ListRefreshSchedule,
    now: DateTime<Utc>,
    draw: f64,
    measurement_holds: bool,
) -> bool {
    if schedule.due(now) {
        let outcome = refresh(cfg, dir).await;
        if outcome == RefreshOutcome::Failed {
            schedule.failed(now);
        } else {
            schedule.attempted(now, draw);
        }
        matches!(outcome, RefreshOutcome::Installed { .. })
    } else if measurement_holds {
        refresh_on_refusal(cfg, dir, now).await
    } else {
        false
    }
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
    fn this_build_trusts_exactly_the_commons_list_signing_key() {
        let shipped: Vec<[u8; 32]> = TRUSTED_LIST_SIGNING_KEYS
            .iter()
            .map(|key| parse_public_key_hex(key).expect("a shipped key parses"))
            .collect();
        let ids: Vec<String> = shipped.iter().map(key_id_for).collect();
        assert_eq!(ids, TRUSTED_LIST_SIGNING_KEY_IDS);
        assert_eq!(ids, ["ea396d6a7f0e867f"]);
        // A state directory with no test keys gets the shipped set.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(trusted_keys(dir.path()), shipped);
    }

    #[test]
    fn the_shipped_key_is_the_one_kms_exported() {
        // The PEM `gcloud kms keys versions get-public-key` wrote for the
        // list-signing key, version 1. Public, not secret.
        const PEM_BODY: &str = "MCowBQYDK2VwAyEA/tUmN27bKbMyjLbEoKH2otbBJnZBtwXNJ2xa3yHQpSM=";
        use base64::Engine as _;
        let der = base64::engine::general_purpose::STANDARD
            .decode(PEM_BODY)
            .unwrap();
        // The Ed25519 SubjectPublicKeyInfo prefix, then the raw key.
        assert_eq!(hex::encode(&der[..12]), "302a300506032b6570032100");
        assert_eq!(hex::encode(&der[12..]), TRUSTED_LIST_SIGNING_KEYS[0]);
    }

    #[test]
    fn a_list_signed_by_any_other_key_is_refused_under_the_shipped_set() {
        // No signature from the real key can be made here (it never leaves
        // KMS), so the shipped set is exercised against a stranger's list:
        // it must be refused, and nothing stored.
        let dir = tempfile::tempdir().unwrap();
        let stranger = key();
        let document = signed(&stranger, &list(1, vec![entry("cc", 1, 20)]));
        assert_eq!(
            install(
                dir.path(),
                &document,
                &trusted_keys(dir.path()),
                WITNESS_URL
            ),
            Err("witness-accepted-list-signature-invalid")
        );
        assert!(!dir.path().join(ACCEPTED_LIST_FILE).exists());
    }

    #[test]
    fn a_list_signed_by_a_trusted_key_verifies_through_the_same_path() {
        // The wiring the shipped key uses, with a test key in its place.
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let document = signed(&signer, &list(1, vec![entry("cc", 1, 20)]));
        assert_eq!(
            install(
                dir.path(),
                &document,
                &trusted_keys(dir.path()),
                WITNESS_URL
            ),
            Ok(InstallOutcome::Installed { version: 1 })
        );
        assert_eq!(
            install(
                dir.path(),
                &document,
                &trusted_keys(dir.path()),
                WITNESS_URL
            ),
            Ok(InstallOutcome::NotNewer)
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
    fn the_version_highwater_stays_global_across_url_changes_and_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        let keys = [public(&signer)];
        let legacy = signed(&signer, &list(4, vec![entry("cc", 1, 20)]));
        install(dir.path(), &legacy, &keys, WITNESS_URL).unwrap();

        let custom_url = "https://custom-witness.example/api/%2Fproof";
        let mut custom = list(4, vec![entry("dd", 1, 20)]);
        custom.witness_url = custom_url.into();
        assert_eq!(
            install(dir.path(), &signed(&signer, &custom), &keys, custom_url),
            Ok(InstallOutcome::NotNewer)
        );
        assert_eq!(
            std::fs::read(dir.path().join(ACCEPTED_LIST_FILE)).unwrap(),
            legacy
        );
        custom.version = 5;
        assert_eq!(
            install(dir.path(), &signed(&signer, &custom), &keys, custom_url),
            Ok(InstallOutcome::Installed { version: 5 })
        );
        assert_eq!(
            install(dir.path(), &legacy, &keys, WITNESS_URL),
            Ok(InstallOutcome::NotNewer)
        );
        assert_eq!(
            install(
                dir.path(),
                &signed(&signer, &list(6, vec![entry("cc", 1, 20)])),
                &keys,
                WITNESS_URL
            ),
            Ok(InstallOutcome::Installed { version: 6 })
        );
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
        serve_app(app).await
    }

    async fn serve_app(app: axum::Router) -> (String, tokio::task::JoinHandle<()>) {
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
    async fn a_legacy_server_ignoring_the_query_still_refreshes_the_configured_legacy_url() {
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
    async fn a_refresh_requests_and_installs_the_list_for_the_configured_url() {
        use axum::{Router, extract::Query, http::StatusCode, routing::get};
        const CUSTOM_URL: &str = "https://custom-witness.example/api/%2Fproof/";
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let mut custom = list(4, vec![entry("cc", 1, 20)]);
        custom.witness_url = CUSTOM_URL.into();
        let document = signed(&signer, &custom);
        let served = document.clone();
        let app = Router::new().route(
            trace_commons_protocol::witness_accepted_list::WITNESS_ACCEPTED_LIST_PATH,
            get(move |Query(query): Query<HashMap<String, String>>| {
                let served = served.clone();
                async move {
                    if query.len() == 1
                        && query.get("witness_url").map(String::as_str) == Some(CUSTOM_URL)
                    {
                        (StatusCode::OK, served)
                    } else {
                        (StatusCode::NOT_FOUND, Vec::new())
                    }
                }
            }),
        );
        let (origin, server) = serve_app(app).await;
        let mut cfg = cfg_for(&origin);
        cfg.witness.as_mut().unwrap().url = CUSTOM_URL.into();
        let settings_before = cfg.witness.clone();
        assert_eq!(
            refresh(&cfg, dir.path()).await,
            RefreshOutcome::Installed { version: 4 }
        );
        assert_eq!(
            std::fs::read(dir.path().join(ACCEPTED_LIST_FILE)).unwrap(),
            document
        );
        assert_eq!(
            cfg.witness, settings_before,
            "refresh never migrates the configured witness"
        );
        let trust = trust_with_accepted(cfg.witness.as_ref().unwrap(), dir.path(), at(10)).unwrap();
        assert_eq!(trust.accepted.len(), 1);
        server.abort();
    }

    #[tokio::test]
    async fn an_old_server_ignoring_the_query_cannot_install_a_list_for_another_url() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let custom_url = "https://custom-witness.example/api/%2Fproof";
        let mut custom = list(4, vec![entry("cc", 1, 20)]);
        custom.witness_url = custom_url.into();
        let stored = signed(&signer, &custom);
        install(dir.path(), &stored, &[public(&signer)], custom_url).unwrap();

        // This legacy server ignores the selector and returns its default list,
        // even with a higher version and a trusted signature.
        let (origin, server) =
            serve(Some(signed(&signer, &list(5, vec![entry("dd", 1, 20)])))).await;
        let mut cfg = cfg_for(&origin);
        cfg.witness.as_mut().unwrap().url = custom_url.into();
        assert_eq!(refresh(&cfg, dir.path()).await, RefreshOutcome::Failed);
        assert_eq!(
            std::fs::read(dir.path().join(ACCEPTED_LIST_FILE)).unwrap(),
            stored
        );
        assert_eq!(cfg.witness.as_ref().unwrap().url, custom_url);
        server.abort();
    }

    #[tokio::test]
    async fn malformed_oversized_and_redirected_refreshes_preserve_the_stored_list() {
        use axum::{Router, response::Redirect, routing::get};
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let stored = signed(&signer, &list(4, vec![entry("cc", 1, 20)]));
        install(dir.path(), &stored, &[public(&signer)], WITNESS_URL).unwrap();
        for document in [
            b"not json".to_vec(),
            vec![b' '; WITNESS_ACCEPTED_LIST_MAX_BYTES + 1],
        ] {
            let (origin, server) = serve(Some(document)).await;
            assert_eq!(
                refresh(&cfg_for(&origin), dir.path()).await,
                RefreshOutcome::Failed
            );
            assert_eq!(
                std::fs::read(dir.path().join(ACCEPTED_LIST_FILE)).unwrap(),
                stored
            );
            server.abort();
        }

        let newer = signed(&signer, &list(5, vec![entry("dd", 1, 20)]));
        let app = Router::new()
            .route(
                trace_commons_protocol::witness_accepted_list::WITNESS_ACCEPTED_LIST_PATH,
                get(|| async { Redirect::temporary("/redirected-list") }),
            )
            .route(
                "/redirected-list",
                get(move || {
                    let newer = newer.clone();
                    async move { newer }
                }),
            );
        let (origin, server) = serve_app(app).await;
        assert_eq!(
            refresh(&cfg_for(&origin), dir.path()).await,
            RefreshOutcome::Failed
        );
        assert_eq!(
            std::fs::read(dir.path().join(ACCEPTED_LIST_FILE)).unwrap(),
            stored
        );
        server.abort();
    }

    #[tokio::test]
    async fn a_build_without_a_key_never_fetches() {
        let dir = tempfile::tempdir().unwrap();
        // This directory trusts no key, as a build with an empty set.
        set_test_keys(dir.path(), Vec::new());
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

    #[tokio::test]
    async fn a_failed_scheduled_fetch_retries_within_the_spacing_not_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        // Nothing published yet: 404.
        let (origin, server) = serve(None).await;
        let cfg = cfg_for(&origin);
        let mut schedule = ListRefreshSchedule::starting(at(1));
        assert!(!refresh_for_tick(&cfg, dir.path(), &mut schedule, at(1), 0.5, false).await);
        assert!(!schedule.due(at(1) + Duration::minutes(9)));
        assert!(schedule.due(at(1) + REFUSAL_REFRESH_SPACING));
        server.abort();

        // A success waits the day.
        let (origin, server) =
            serve(Some(signed(&signer, &list(1, vec![entry("cc", 1, 20)])))).await;
        let cfg = cfg_for(&origin);
        let later = at(1) + REFUSAL_REFRESH_SPACING;
        assert!(refresh_for_tick(&cfg, dir.path(), &mut schedule, later, 0.5, false).await);
        assert!(!schedule.due(later + Duration::hours(23)));
        server.abort();
    }

    #[tokio::test]
    async fn held_sessions_trigger_one_early_fetch_per_spacing() {
        let dir = tempfile::tempdir().unwrap();
        let signer = key();
        set_test_keys(dir.path(), vec![public(&signer)]);
        let (origin, server) =
            serve(Some(signed(&signer, &list(2, vec![entry("cc", 1, 20)])))).await;
        let cfg = cfg_for(&origin);
        // Startup fetch done long ago: not due for a day.
        let mut schedule = ListRefreshSchedule::starting(at(1));
        schedule.attempted(at(1), 0.0);
        let now = at(1) + Duration::hours(1);
        // No holds: no early fetch.
        assert!(!refresh_for_tick(&cfg, dir.path(), &mut schedule, now, 0.5, false).await);
        assert!(load_verified(dir.path(), &trusted_keys(dir.path())).is_none());
        // Holds: fetched and installed.
        assert!(refresh_for_tick(&cfg, dir.path(), &mut schedule, now, 0.5, true).await);
        assert_eq!(
            load_verified(dir.path(), &trusted_keys(dir.path()))
                .unwrap()
                .version,
            2
        );
        server.abort();

        // A newer list now exists, but a second refusal inside the spacing
        // does not fetch.
        let (origin, server) =
            serve(Some(signed(&signer, &list(3, vec![entry("dd", 1, 20)])))).await;
        let cfg = cfg_for(&origin);
        let soon = now + Duration::minutes(5);
        assert!(!refresh_for_tick(&cfg, dir.path(), &mut schedule, soon, 0.5, true).await);
        assert_eq!(
            load_verified(dir.path(), &trusted_keys(dir.path()))
                .unwrap()
                .version,
            2
        );
        let after = now + REFUSAL_REFRESH_SPACING;
        assert!(refresh_for_tick(&cfg, dir.path(), &mut schedule, after, 0.5, true).await);
        server.abort();
    }
}
