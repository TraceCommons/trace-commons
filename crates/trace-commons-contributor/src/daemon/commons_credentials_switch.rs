//! A second device key, and switching this daemon's identity onto it.
//!
//! The legacy invite migration (`daemon::legacy_migration`) moves a daemon
//! from a `tenant-…` invite identity to a NEAR AI account. A device key is
//! registered to exactly one tenant, so the new identity needs a key of its
//! own, generated here and held in a **staging slot** (`Kind::StagedDevice`)
//! while the old key goes on signing everything, the link statement
//! included. Nothing reads a staged key as the live device key: its entry is
//! bound to its own domain.
//!
//! # The switch
//!
//! [`switch_identity`] replaces three things together -- the config, the
//! device key and the account session -- and does it so that a failure at
//! any point leaves the legacy identity exactly as it was:
//!
//! 1. New OS entries are prepared first: the staged key re-published as a
//!    `Device` entry, and the new account session bound to the new config.
//!    Nothing local points at them yet, and both are journaled for cleanup,
//!    so a failure here costs two orphaned entries and nothing else.
//! 2. Under the commit lock, the legacy snapshot is checked again. A logout
//!    since the migration began advances the generation and removes the
//!    device key, so the switch refuses rather than passing through it.
//! 3. A switch journal is written, holding the old config and the old
//!    credential records (references, never secrets), then the new config,
//!    device record and account record are written in that order.
//!
//! The legacy key's OS entry is **not** retired by the switch. It stays
//! until [`retire_legacy`], which the caller runs last, after it has
//! re-recorded the armed folders and marked the switch committed. Until
//! then [`roll_back_switch`] -- run in-process on a failure, and by
//! `legacy_migration::recover` at the next start after a crash -- puts every
//! file back, and the old key is still there to be pointed at.
use super::*;

const SWITCH_JOURNAL_VERSION: u8 = 1;
const MAX_SWITCH_JOURNAL_BYTES: usize = 65_536;

fn journal_path(store: &ConfigStore) -> std::path::PathBuf {
    store.daemon_path(crate::config::IDENTITY_SWITCH_JOURNAL_FILE)
}

/// Where a switch has got to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SwitchPhase {
    /// Files may be written, or half-written. Recovery rolls back.
    Switching,
    /// Every file is written and the caller's own step is done. Recovery
    /// finishes: it retires the legacy key.
    Committed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SwitchJournal {
    version: u8,
    phase: SwitchPhase,
    /// The config file's bytes before the switch.
    old_config: String,
    /// The device key's credential record before the switch.
    old_device: String,
    /// The account session's credential record before the switch, if any.
    old_account: Option<String>,
    /// What the caller needs to undo or finish its own part. Opaque here.
    context: serde_json::Value,
}

/// A pending switch, as recovery finds it.
pub(crate) struct PendingSwitch {
    pub(crate) phase: SwitchPhase,
    pub(crate) context: serde_json::Value,
}

fn utf8(bytes: &[u8]) -> Result<String> {
    String::from_utf8(bytes.to_vec()).map_err(|_| unavailable())
}

fn read_switch_journal(store: &ConfigStore) -> Result<Option<SwitchJournal>> {
    let bytes = match std::fs::read(journal_path(store)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(unavailable()),
    };
    if bytes.len() > MAX_SWITCH_JOURNAL_BYTES {
        return Err(anyhow!("identity_switch_journal_invalid"));
    }
    let journal: SwitchJournal =
        serde_json::from_slice(&bytes).map_err(|_| anyhow!("identity_switch_journal_invalid"))?;
    if journal.version != SWITCH_JOURNAL_VERSION {
        return Err(anyhow!("identity_switch_journal_invalid"));
    }
    Ok(Some(journal))
}

fn write_switch_journal(store: &ConfigStore, journal: &SwitchJournal) -> Result<()> {
    let bytes = serde_json::to_vec(journal).map_err(|_| unavailable())?;
    if bytes.len() > MAX_SWITCH_JOURNAL_BYTES {
        return Err(anyhow!("identity_switch_journal_invalid"));
    }
    write(store, &journal_path(store), &bytes)
}

fn remove_if_present(path: &std::path::Path) -> Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(unavailable()),
    }
}

/// Generate a fresh key into the staging slot, replacing any key staged
/// before. The live device key is untouched.
pub(crate) fn stage_device_key(store: &ConfigStore) -> Result<crate::identity::DeviceIdentity> {
    let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
        .map_err(|_| anyhow!("generating staged device keypair"))?;
    let expected = snapshot(store, Kind::StagedDevice)?;
    replace(store, &expected, doc.as_ref(), None)?;
    crate::identity::DeviceIdentity::from_staged_pkcs8(doc.as_ref())
}

/// The staged key, if there is one.
#[cfg(test)]
pub(crate) fn load_staged_device_key(
    store: &ConfigStore,
) -> Result<Option<crate::identity::DeviceIdentity>> {
    load(store, Kind::StagedDevice)?
        .map(|pkcs8| crate::identity::DeviceIdentity::from_staged_pkcs8(&pkcs8))
        .transpose()
}

/// Drop the staged key. Unlike a logout this advances no generation: the
/// live identity has not changed, so nothing captured against it should be
/// invalidated.
pub(crate) fn discard_staged_device_key(store: &ConfigStore) -> Result<()> {
    {
        let locks = coordination(store.dir())?;
        let _commit = locks.commit.lock()?;
        let path = Kind::StagedDevice.file(store);
        if let Some(bytes) = read(&path)? {
            if let Some(record) = record(&bytes)? {
                let mut journal = Journal::read_named(store, JOURNAL)?;
                journal.retain(record.reference)?;
                journal.save_named(store, JOURNAL)?;
            }
        }
        remove_if_present(&path)?;
        sync_directory(store).map_err(|_| unavailable())?;
    }
    let _ = cleanup(store);
    Ok(())
}

#[cfg(test)]
fn fail_points() -> &'static std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, u8>> {
    static POINTS: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, u8>>,
    > = std::sync::OnceLock::new();
    POINTS.get_or_init(Default::default)
}

/// Make the next switch on `store` fail after step `step`: 1 the journal,
/// 2 the config, 3 the device record, 4 the account record. Keyed by the
/// store rather than the thread, because on Linux the switch runs on a
/// scoped thread of its own.
#[cfg(test)]
pub(crate) fn fail_switch_after_for_test(store: &ConfigStore, step: u8) {
    fail_points()
        .lock()
        .unwrap()
        .insert(store.dir().to_path_buf(), step);
}

fn fail_point(_store: &ConfigStore, _step: u8) -> Result<()> {
    #[cfg(test)]
    {
        let mut points = fail_points().lock().unwrap();
        if points.get(_store.dir()) == Some(&_step) {
            points.remove(_store.dir());
            return Err(anyhow!("identity_switch_injected_failure"));
        }
    }
    Ok(())
}

/// Switch this daemon's identity onto the staged key, `new_config` and the
/// account session `account_session`.
///
/// `legacy` is a `Kind::Device` snapshot taken when the migration began,
/// before the staged key existed. It is checked under the commit lock, so a
/// logout, a wipe, a re-enrollment or a config change since then refuses the
/// switch: the migration never passes through a sign-out.
///
/// On success the journal stays, in `Switching`: the caller does its own
/// step and then calls [`mark_switch_committed`] and [`retire_legacy`]. On
/// failure every file is put back before this returns.
pub(crate) fn switch_identity(
    store: &ConfigStore,
    legacy: &Snapshot,
    new_config: &ContributorConfig,
    account_session: &[u8],
    context: serde_json::Value,
) -> Result<()> {
    if needs_native_thread() {
        return std::thread::scope(|scope| {
            scope
                .spawn(|| switch_identity(store, legacy, new_config, account_session, context))
                .join()
                .map_err(|_| unavailable())?
        });
    }
    if legacy.kind != Kind::Device {
        return Err(changed());
    }
    let Some((staged, staged_snapshot)) = load_with_snapshot(store, Kind::StagedDevice)? else {
        return Err(anyhow!("identity_switch_no_staged_key"));
    };
    let staged_identity = crate::identity::DeviceIdentity::from_staged_pkcs8(&staged)?;
    if staged_identity.device_key_id != new_config.device_key_id {
        return Err(changed());
    }
    let config_bytes = serde_json::to_vec_pretty(new_config).map_err(|_| unavailable())?;

    let locks = coordination(store.dir())?;
    let _storage = locks.storage.lock()?;
    let credentials = native(store)?;
    cleanup_locked(store, &credentials)?;
    if read_switch_journal(store)?.is_some() {
        return Err(anyhow!("identity_switch_pending"));
    }

    let device_record = Record {
        commons_credential_version: 1,
        kind: Kind::Device,
        reference: CredentialReference::allocate(),
        authority: None,
    };
    let account_record = Record {
        commons_credential_version: 1,
        kind: Kind::Account,
        reference: CredentialReference::allocate(),
        authority: Some(authority(new_config)),
    };
    let bundle = |record: &Record, payload: &[u8]| -> Result<Vec<u8>> {
        serde_json::to_vec(&Bundle {
            version: 1,
            binding: binding(store, record)?,
            payload: STANDARD.encode(payload),
        })
        .map_err(|_| unavailable())
    };
    let device_bundle = bundle(&device_record, &staged)?;
    let account_bundle = bundle(&account_record, account_session)?;
    {
        let _commit = locks.commit.lock()?;
        ensure_current(store, legacy)?;
        let mut journal = Journal::read_named(store, JOURNAL)?;
        journal.retain(device_record.reference)?;
        journal.retain(account_record.reference)?;
        journal.save_named(store, JOURNAL)?;
    }
    credentials
        .prepare_bytes_at(&device_record.reference, &device_bundle)
        .map_err(|_| unavailable())?;
    credentials
        .prepare_bytes_at(&account_record.reference, &account_bundle)
        .map_err(|_| unavailable())?;

    let _commit = locks.commit.lock()?;
    ensure_current(store, legacy)?;
    if snapshot_locked(store, Kind::StagedDevice)? != staged_snapshot {
        return Err(changed());
    }
    let (Some(old_config), Some(old_device)) = (&legacy.config, &legacy.previous) else {
        return Err(anyhow!("identity_switch_not_enrolled"));
    };
    let old_account = read(&Kind::Account.file(store))?;
    let journal = SwitchJournal {
        version: SWITCH_JOURNAL_VERSION,
        phase: SwitchPhase::Switching,
        old_config: utf8(old_config)?,
        old_device: utf8(old_device)?,
        old_account: old_account.as_deref().map(utf8).transpose()?,
        context,
    };
    write_switch_journal(store, &journal)?;
    let steps = || -> Result<()> {
        fail_point(store, 1)?;
        write(store, &store.daemon_path("contributor.json"), &config_bytes)?;
        fail_point(store, 2)?;
        write(
            store,
            &Kind::Device.file(store),
            &serde_json::to_vec(&device_record).map_err(|_| unavailable())?,
        )?;
        fail_point(store, 3)?;
        write(
            store,
            &Kind::Account.file(store),
            &serde_json::to_vec(&account_record).map_err(|_| unavailable())?,
        )?;
        fail_point(store, 4)?;
        // The staged entry's reference was journaled when it was staged; the
        // file pointing at it goes, and cleanup retires the entry.
        remove_if_present(&Kind::StagedDevice.file(store))?;
        // Anything captured against the old identity is now stale.
        write(
            store,
            &store.daemon_path(EPOCH),
            uuid::Uuid::new_v4().to_string().as_bytes(),
        )?;
        sync_directory(store).map_err(|_| unavailable())
    };
    if let Err(error) = steps() {
        // Put everything back while still holding the lock. If even that
        // fails the journal stays, and the next start rolls back instead.
        if restore_locked(store, &journal).is_ok() {
            let _ = remove_if_present(&journal_path(store));
            let _ = sync_directory(store);
        }
        return Err(error);
    }
    Ok(())
}

/// Write back the files a switch replaced. The staged file is not restored:
/// a migration that failed starts again with a fresh key.
fn restore_locked(store: &ConfigStore, journal: &SwitchJournal) -> Result<()> {
    write(
        store,
        &store.daemon_path("contributor.json"),
        journal.old_config.as_bytes(),
    )?;
    write(
        store,
        &Kind::Device.file(store),
        journal.old_device.as_bytes(),
    )?;
    match &journal.old_account {
        Some(account) => write(store, &Kind::Account.file(store), account.as_bytes())?,
        None => remove_if_present(&Kind::Account.file(store))?,
    }
    if let Some(bytes) = read(&Kind::StagedDevice.file(store))? {
        if let Some(record) = record(&bytes)? {
            let mut cleanup = Journal::read_named(store, JOURNAL)?;
            cleanup.retain(record.reference)?;
            cleanup.save_named(store, JOURNAL)?;
        }
    }
    remove_if_present(&Kind::StagedDevice.file(store))?;
    sync_directory(store).map_err(|_| unavailable())
}

/// The switch in progress, if any.
pub(crate) fn pending_switch(store: &ConfigStore) -> Result<Option<PendingSwitch>> {
    Ok(read_switch_journal(store)?.map(|journal| PendingSwitch {
        phase: journal.phase,
        context: journal.context,
    }))
}

/// Undo a switch that has not been committed: the old config, device key
/// record and account record go back. Returns the caller's context, so it
/// can undo its own part. `None` when there was nothing to undo.
pub(crate) fn roll_back_switch(store: &ConfigStore) -> Result<Option<serde_json::Value>> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    let Some(journal) = read_switch_journal(store)? else {
        return Ok(None);
    };
    if journal.phase != SwitchPhase::Switching {
        return Err(anyhow!("identity_switch_committed"));
    }
    restore_locked(store, &journal)?;
    write(
        store,
        &store.daemon_path(EPOCH),
        uuid::Uuid::new_v4().to_string().as_bytes(),
    )?;
    remove_if_present(&journal_path(store))?;
    sync_directory(store).map_err(|_| unavailable())?;
    Ok(Some(journal.context))
}

/// The caller's own step is done: from here recovery finishes the switch
/// rather than undoing it.
pub(crate) fn mark_switch_committed(store: &ConfigStore) -> Result<()> {
    let locks = coordination(store.dir())?;
    let _commit = locks.commit.lock()?;
    let Some(mut journal) = read_switch_journal(store)? else {
        return Err(anyhow!("identity_switch_missing"));
    };
    journal.phase = SwitchPhase::Committed;
    write_switch_journal(store, &journal)
}

/// Retire the legacy key and account session: last, and only for a
/// committed switch. Their OS entries go to the cleanup journal and the
/// switch journal is removed.
pub(crate) fn retire_legacy(store: &ConfigStore) -> Result<()> {
    {
        let locks = coordination(store.dir())?;
        let _commit = locks.commit.lock()?;
        let Some(journal) = read_switch_journal(store)? else {
            return Ok(());
        };
        if journal.phase != SwitchPhase::Committed {
            return Err(anyhow!("identity_switch_not_committed"));
        }
        let mut cleanup = Journal::read_named(store, JOURNAL)?;
        for old in std::iter::once(&journal.old_device).chain(journal.old_account.as_ref()) {
            if let Some(record) = record(old.as_bytes())? {
                cleanup.retain(record.reference)?;
            }
        }
        cleanup.save_named(store, JOURNAL)?;
        remove_if_present(&journal_path(store))?;
        sync_directory(store).map_err(|_| unavailable())?;
    }
    let _ = cleanup(store);
    Ok(())
}

#[cfg(test)]
#[path = "commons_credentials_switch_tests.rs"]
mod tests;
