// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The limiter's sealed ledger on the `/data` volume (spec section 6).
//!
//! The ledger is the service's only state. It holds, for the current UTC
//! month, the operator's total spend and each keyed account digest's spend,
//! in integer nano-USD. Nothing else: no raw account, no digest, no label.
//!
//! # Sealing
//!
//! AES-256-GCM under the `ledger-seal` key (HKDF from the enclave-derived
//! `judgement-limiter` root, see [`super::limiter::LimiterKeys`]). The file
//! is `nonce(12) || ciphertext`, with a fresh random nonce per write. The
//! associated data is the file name, the format version and the fixed label
//! [`LEDGER_AD_LABEL`], so a ledger renamed or read by another format does
//! not open. It binds nothing about the boot: the sealing and signing keys
//! are per boot, and a ledger bound to them would be unreadable after every
//! restart. The binding to the deployment comes from the `ledger-seal` key
//! itself, which derives from the app-id-bound limiter root, so a ledger
//! copied to another app does not open. The service encrypts it itself, so
//! whether dstack also encrypts the volume does not affect confidentiality.
//!
//! # Writes
//!
//! Temp file, `fsync`, atomic rename, then `fsync` of the directory, after
//! every reserve and every settle and before the response leaves. A write
//! that fails is reported to the limiter, which refuses the call: a service
//! that cannot record spend does not spend.
//!
//! # Boot rules
//!
//! - the volume directory is absent: refuse;
//! - the ledger exists and does not decrypt: refuse
//!   (`judgement_limiter_unreadable`);
//! - the ledger is absent and `limiter.initialized` exists: refuse;
//! - both are absent: create both, log `limiter_initialized`;
//! - the ledger opens and the marker is absent (a crash between the first
//!   ledger write and the marker): write the marker, log
//!   `limiter_marker_restored`, so a later loss of the ledger still
//!   refuses.
//!
//! An operator can still delete the volume or roll the file back to an older
//! copy, and so reset spend. That only spends the operator's own money and
//! reveals nothing; the per-account cap defends the operator's budget
//! against contributors, not against the operator.

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use aes_gcm::aead::{Aead, KeyInit, OsRng, Payload, rand_core::RngCore};
use aes_gcm::{Aes256Gcm, Nonce};
use zeroize::Zeroizing;

/// The sealed ledger's file name.
pub const LEDGER_FILE: &str = "limiter.v1.sealed";
/// The marker written beside the ledger the first time it is created.
pub const MARKER_FILE: &str = "limiter.initialized";
/// The plaintext format version, also bound into the associated data.
pub const LEDGER_FORMAT_VERSION: u32 = 1;
/// The fixed label in the associated data.
pub const LEDGER_AD_LABEL: &[u8] = b"insights-judgement/ledger";

const NONCE_LEN: usize = 12;
const ACCOUNT_KEY_LEN: usize = 32;

/// The ledger's contents: one UTC month.
///
/// `month` is `year * 12 + (month - 1)`, see
/// [`super::limiter::month_index`]. Account entries are keyed by the keyed
/// account digest, never the account.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct LedgerState {
    pub month: u32,
    pub operator_spent_nanousd: u64,
    pub accounts: BTreeMap<[u8; ACCOUNT_KEY_LEN], u64>,
}

/// Counts only: the keys are account digests and stay out of logs.
impl std::fmt::Debug for LedgerState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LedgerState")
            .field("month", &self.month)
            .field("accounts", &self.accounts.len())
            .finish_non_exhaustive()
    }
}

/// Why the ledger refused to open. Each is a boot refusal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LedgerBootError {
    /// The `/data` volume is not mounted (the directory is absent).
    #[error("judgement_limiter_volume_absent")]
    VolumeAbsent,
    /// The ledger exists and does not decrypt or does not parse.
    #[error("judgement_limiter_unreadable")]
    Unreadable,
    /// The ledger is gone but the marker says one was created.
    #[error("judgement_limiter_missing")]
    MissingWithMarker,
    /// The volume could not be read or written.
    #[error("judgement_limiter_io")]
    Io,
}

/// Where the limiter persists its state. A trait so the limiter's
/// write-failure rule is testable.
pub trait LedgerStore: Send + Sync {
    /// Persist `state` durably before returning `Ok`.
    fn persist(&self, state: &LedgerState) -> std::io::Result<()>;
}

/// The ledger on disk.
pub struct SealedLedger {
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl std::fmt::Debug for SealedLedger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SealedLedger(..)")
    }
}

impl SealedLedger {
    /// Open the ledger in `dir` under the boot rules, returning the store
    /// and the state read back (empty when it was just created).
    pub fn open(
        dir: &Path,
        seal_key: Zeroizing<[u8; 32]>,
    ) -> Result<(Self, LedgerState), LedgerBootError> {
        if !dir.is_dir() {
            return Err(LedgerBootError::VolumeAbsent);
        }
        let ledger = Self {
            dir: dir.to_path_buf(),
            key: seal_key,
        };
        let marker = dir.join(MARKER_FILE);
        match std::fs::read(ledger.ledger_path()) {
            Ok(sealed) => {
                let state = ledger.unseal(&sealed).ok_or(LedgerBootError::Unreadable)?;
                if !marker.exists() {
                    // The first boot crashed between the ledger and the
                    // marker. Without the marker, a later loss of the
                    // ledger would boot fresh rather than refuse.
                    write_durably(dir, &marker, b"1\n").map_err(|_| LedgerBootError::Io)?;
                    tracing::info!(
                        target: "trace_commons_judgement",
                        event = "limiter_marker_restored"
                    );
                }
                Ok((ledger, state))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if marker.exists() {
                    return Err(LedgerBootError::MissingWithMarker);
                }
                let state = LedgerState::default();
                // The ledger first, then the marker: a crash between the two
                // leaves a ledger without a marker, which opens; the reverse
                // order would leave a marker without a ledger, which refuses.
                ledger.persist(&state).map_err(|_| LedgerBootError::Io)?;
                write_durably(dir, &marker, b"1\n").map_err(|_| LedgerBootError::Io)?;
                tracing::info!(target: "trace_commons_judgement", event = "limiter_initialized");
                Ok((ledger, state))
            }
            Err(_) => Err(LedgerBootError::Io),
        }
    }

    fn ledger_path(&self) -> PathBuf {
        self.dir.join(LEDGER_FILE)
    }

    fn associated_data(&self) -> Vec<u8> {
        let mut ad = Vec::with_capacity(64);
        ad.extend_from_slice(&(LEDGER_FILE.len() as u64).to_le_bytes());
        ad.extend_from_slice(LEDGER_FILE.as_bytes());
        ad.extend_from_slice(&LEDGER_FORMAT_VERSION.to_le_bytes());
        ad.extend_from_slice(&(LEDGER_AD_LABEL.len() as u64).to_le_bytes());
        ad.extend_from_slice(LEDGER_AD_LABEL);
        ad
    }

    fn seal(&self, state: &LedgerState) -> Vec<u8> {
        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice())
            .expect("a 32-byte key is an AES-256 key");
        let mut nonce = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce);
        let plaintext = Zeroizing::new(encode_state(state));
        let ciphertext = cipher
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &self.associated_data(),
                },
            )
            .expect("AES-GCM encryption of a bounded buffer does not fail");
        let mut sealed = Vec::with_capacity(NONCE_LEN + ciphertext.len());
        sealed.extend_from_slice(&nonce);
        sealed.extend_from_slice(&ciphertext);
        sealed
    }

    fn unseal(&self, sealed: &[u8]) -> Option<LedgerState> {
        if sealed.len() < NONCE_LEN {
            return None;
        }
        let (nonce, ciphertext) = sealed.split_at(NONCE_LEN);
        let cipher = Aes256Gcm::new_from_slice(self.key.as_slice()).ok()?;
        let plaintext = Zeroizing::new(
            cipher
                .decrypt(
                    Nonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &self.associated_data(),
                    },
                )
                .ok()?,
        );
        decode_state(&plaintext)
    }
}

impl LedgerStore for SealedLedger {
    fn persist(&self, state: &LedgerState) -> std::io::Result<()> {
        write_durably(&self.dir, &self.ledger_path(), &self.seal(state))
    }
}

/// Temp file in the same directory, `fsync`, atomic rename, `fsync` of the
/// directory so the rename itself is durable.
fn write_durably(dir: &Path, path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut temp = tempfile::NamedTempFile::new_in(dir)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    temp.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(dir)?.sync_all()
}

/// Plaintext: format version, month, operator spend, entry count, then each
/// entry as `account_key(32) || spent(u64 LE)` in key order.
fn encode_state(state: &LedgerState) -> Vec<u8> {
    let mut out = Vec::with_capacity(24 + state.accounts.len() * (ACCOUNT_KEY_LEN + 8));
    out.extend_from_slice(&LEDGER_FORMAT_VERSION.to_le_bytes());
    out.extend_from_slice(&state.month.to_le_bytes());
    out.extend_from_slice(&state.operator_spent_nanousd.to_le_bytes());
    out.extend_from_slice(&(state.accounts.len() as u64).to_le_bytes());
    for (key, spent) in &state.accounts {
        out.extend_from_slice(key);
        out.extend_from_slice(&spent.to_le_bytes());
    }
    out
}

/// The inverse of [`encode_state`], refusing a wrong version, a short read
/// or trailing bytes.
fn decode_state(bytes: &[u8]) -> Option<LedgerState> {
    fn take<'a>(bytes: &mut &'a [u8], n: usize) -> Option<&'a [u8]> {
        if bytes.len() < n {
            return None;
        }
        let (head, tail) = bytes.split_at(n);
        *bytes = tail;
        Some(head)
    }
    let mut rest = bytes;
    let version = u32::from_le_bytes(take(&mut rest, 4)?.try_into().ok()?);
    if version != LEDGER_FORMAT_VERSION {
        return None;
    }
    let month = u32::from_le_bytes(take(&mut rest, 4)?.try_into().ok()?);
    let operator_spent_nanousd = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
    let count = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
    let entry_len = ACCOUNT_KEY_LEN + 8;
    if (rest.len() as u64) != count.checked_mul(entry_len as u64)? {
        return None;
    }
    let mut accounts = BTreeMap::new();
    for _ in 0..count {
        let key: [u8; ACCOUNT_KEY_LEN] = take(&mut rest, ACCOUNT_KEY_LEN)?.try_into().ok()?;
        let spent = u64::from_le_bytes(take(&mut rest, 8)?.try_into().ok()?);
        if accounts.insert(key, spent).is_some() {
            return None;
        }
    }
    Some(LedgerState {
        month,
        operator_spent_nanousd,
        accounts,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new([byte; 32])
    }

    fn sample_state() -> LedgerState {
        let mut accounts = BTreeMap::new();
        accounts.insert([1u8; 32], 1_200_000);
        accounts.insert([2u8; 32], 5_000_000_000);
        LedgerState {
            month: 24_309,
            operator_spent_nanousd: 5_001_200_000,
            accounts,
        }
    }

    #[test]
    fn a_fresh_volume_is_initialized_with_the_marker() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (_, state) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        assert_eq!(state, LedgerState::default());
        assert!(dir.path().join(LEDGER_FILE).exists());
        assert!(dir.path().join(MARKER_FILE).exists());
    }

    #[test]
    fn the_seal_round_trips_through_the_volume() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        ledger.persist(&sample_state()).expect("persists");
        let (_, state) = SealedLedger::open(dir.path(), key(1)).expect("reopens");
        assert_eq!(state, sample_state());
    }

    #[test]
    fn the_file_holds_no_plaintext_account_key_or_amount() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        ledger.persist(&sample_state()).expect("persists");
        let bytes = std::fs::read(dir.path().join(LEDGER_FILE)).expect("read");
        assert!(!bytes.windows(32).any(|w| w == [1u8; 32] || w == [2u8; 32]));
        assert!(
            !bytes
                .windows(8)
                .any(|w| w == 5_000_000_000u64.to_le_bytes())
        );
        // Two writes of one state use two nonces.
        ledger.persist(&sample_state()).expect("persists");
        assert_ne!(
            std::fs::read(dir.path().join(LEDGER_FILE)).expect("read"),
            bytes
        );
    }

    #[test]
    fn a_wrong_key_refuses_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        ledger.persist(&sample_state()).expect("persists");
        assert_eq!(
            SealedLedger::open(dir.path(), key(2)).unwrap_err(),
            LedgerBootError::Unreadable
        );
    }

    #[test]
    fn a_ledger_reads_back_whatever_the_per_boot_keys_are() {
        // The per-boot sealing and signing keys change at every boot, so
        // nothing about them may be bound into the ledger: a restart must
        // read back the spend it wrote.
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("first boot");
        ledger.persist(&sample_state()).expect("persists");
        drop(ledger);
        let (_, state) = SealedLedger::open(dir.path(), key(1)).expect("second boot");
        assert_eq!(state, sample_state());
    }

    #[test]
    fn the_associated_data_is_the_file_name_the_format_and_a_fixed_label() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        let mut expected = Vec::new();
        expected.extend_from_slice(&17u64.to_le_bytes());
        expected.extend_from_slice(b"limiter.v1.sealed");
        expected.extend_from_slice(&1u32.to_le_bytes());
        expected.extend_from_slice(&25u64.to_le_bytes());
        expected.extend_from_slice(b"insights-judgement/ledger");
        assert_eq!(ledger.associated_data(), expected);
    }

    #[test]
    fn a_ledger_found_without_its_marker_gets_the_marker_back() {
        // A crash after the first ledger write and before the marker
        // leaves a ledger without a marker. It opens, and the marker is
        // written then, so a later loss of the ledger is still refused.
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        ledger.persist(&sample_state()).expect("persists");
        std::fs::remove_file(dir.path().join(MARKER_FILE)).expect("remove marker");
        let (_, state) = SealedLedger::open(dir.path(), key(1)).expect("reopens");
        assert_eq!(state, sample_state());
        assert!(dir.path().join(MARKER_FILE).exists());
        std::fs::remove_file(dir.path().join(LEDGER_FILE)).expect("remove ledger");
        assert_eq!(
            SealedLedger::open(dir.path(), key(1)).unwrap_err(),
            LedgerBootError::MissingWithMarker
        );
    }

    #[test]
    fn a_corrupt_ledger_refuses_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        SealedLedger::open(dir.path(), key(1)).expect("opens");
        let path = dir.path().join(LEDGER_FILE);
        let mut bytes = std::fs::read(&path).expect("read");
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, &bytes).expect("write");
        assert_eq!(
            SealedLedger::open(dir.path(), key(1)).unwrap_err(),
            LedgerBootError::Unreadable
        );
        std::fs::write(&path, [0u8; 4]).expect("write");
        assert_eq!(
            SealedLedger::open(dir.path(), key(1)).unwrap_err(),
            LedgerBootError::Unreadable
        );
    }

    #[test]
    fn a_missing_ledger_with_the_marker_refuses_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        SealedLedger::open(dir.path(), key(1)).expect("opens");
        std::fs::remove_file(dir.path().join(LEDGER_FILE)).expect("remove");
        assert_eq!(
            SealedLedger::open(dir.path(), key(1)).unwrap_err(),
            LedgerBootError::MissingWithMarker
        );
    }

    #[test]
    fn an_absent_volume_refuses_the_boot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("data");
        assert_eq!(
            SealedLedger::open(&missing, key(1)).unwrap_err(),
            LedgerBootError::VolumeAbsent
        );
        // A file where the volume should be is not a volume.
        std::fs::write(&missing, b"").expect("write");
        assert_eq!(
            SealedLedger::open(&missing, key(1)).unwrap_err(),
            LedgerBootError::VolumeAbsent
        );
    }

    #[test]
    fn the_plaintext_encoding_refuses_trailing_or_short_bytes() {
        let encoded = encode_state(&sample_state());
        assert_eq!(decode_state(&encoded), Some(sample_state()));
        let mut long = encoded.clone();
        long.push(0);
        assert_eq!(decode_state(&long), None);
        assert_eq!(decode_state(&encoded[..encoded.len() - 1]), None);
        let mut wrong_version = encoded;
        wrong_version[0] = 9;
        assert_eq!(decode_state(&wrong_version), None);
    }

    #[test]
    fn no_temp_file_is_left_behind() {
        let dir = tempfile::tempdir().expect("tempdir");
        let (ledger, _) = SealedLedger::open(dir.path(), key(1)).expect("opens");
        ledger.persist(&sample_state()).expect("persists");
        let mut names: Vec<String> = std::fs::read_dir(dir.path())
            .expect("list")
            .map(|e| e.expect("entry").file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        assert_eq!(
            names,
            vec![MARKER_FILE.to_string(), LEDGER_FILE.to_string()]
        );
    }
}
