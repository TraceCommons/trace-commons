//! INTEGRATION: persists account-scoped and practice mission evaluation attempts
//! before inference so daemon restarts cannot erase partial outcomes.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use trace_commons_protocol::mission_evaluation::MISSION_EVALUATION_TOTAL_REQUESTS;
use uuid::Uuid;

use crate::skill_loop::{EvaluationArm, SkillTrialResult};

mod types;
pub use types::{
    MissionAttempt, MissionAttemptBegin, MissionAttemptScope, MissionAttemptStart,
    MissionAttemptStatus, MissionAttemptSummary, MissionTrialKey,
};

pub const MISSION_ATTEMPT_STORE_DIR: &str = "mission-attempts";

const JOURNAL_VERSION: u32 = 1;
const JOURNAL_FILE: &str = "mission-attempts.json";
const LOCK_FILE: &str = "mission-attempts.lock";
const MAX_JOURNAL_BYTES: u64 = 16 * 1024 * 1024;
const MAX_ATTEMPTS: usize = 128;
// Worst case per attempt: a running status grows to a terminal status, `null`
// grows to a quoted 96-byte failure label, and a short timestamp grows to u64.
// Content writes reserve this for every slot so lifecycle writes cannot be
// stranded by a journal that consumed the complete 16 MiB allowance.
const TERMINAL_METADATA_HEADROOM_PER_ATTEMPT: u64 = 128;
const MAX_CONTENT_JOURNAL_BYTES: u64 =
    MAX_JOURNAL_BYTES - TERMINAL_METADATA_HEADROOM_PER_ATTEMPT * MAX_ATTEMPTS as u64;
const MAX_TRIAL_BYTES: usize = 32 * 1024;
const EXPECTED_TRIALS: usize = MISSION_EVALUATION_TOTAL_REQUESTS as usize;
const MAX_MODEL_CHARS: usize = 256;
const MAX_TASK_ID_CHARS: usize = 128;
const COMPLETED_REASON: &str = "mission-attempt-completed";
const CANCELLED_REASON: &str = "mission-attempt-cancelled";
const INTERRUPTED_REASON: &str = "mission-attempt-interrupted";

/// Stable failures from the local mission-attempt journal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissionAttemptError {
    FieldInvalid,
    Conflict,
    StoreFull,
    NotFound,
    TrialInvalid,
    InvalidState,
    StoreUnavailable,
    StoreUnreadable,
    StoreInvalid,
    StoreBusy,
    StoreWriteFailed,
    StoreVersionUnsupported,
    ClockUnavailable,
    RequiresPrivateDirectory,
}

impl MissionAttemptError {
    /// Returns the stable content-free refusal label.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::FieldInvalid => "mission-attempt-field-invalid",
            Self::Conflict => "mission-attempt-conflict",
            Self::StoreFull => "mission-attempt-store-full",
            Self::NotFound => "mission-attempt-not-found",
            Self::TrialInvalid => "mission-attempt-trial-invalid",
            Self::InvalidState => "mission-attempt-invalid-state",
            Self::StoreUnavailable => "mission-attempt-store-unavailable",
            Self::StoreUnreadable => "mission-attempt-store-unreadable",
            Self::StoreInvalid => "mission-attempt-store-invalid",
            Self::StoreBusy => "mission-attempt-store-busy",
            Self::StoreWriteFailed => "mission-attempt-store-write-failed",
            Self::StoreVersionUnsupported => "mission-attempt-store-version-unsupported",
            Self::ClockUnavailable => "mission-attempt-clock-unavailable",
            Self::RequiresPrivateDirectory => "mission-attempt-store-requires-private-directory",
        }
    }
}

impl std::fmt::Display for MissionAttemptError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for MissionAttemptError {}

type Result<T> = std::result::Result<T, MissionAttemptError>;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MissionAttemptJournal {
    version: u32,
    attempts: Vec<ScopedAttempt>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ScopedAttempt {
    scope: MissionAttemptScope,
    attempt: MissionAttempt,
}

/// Private bounded filesystem journal for practice and account attempts.
pub struct MissionAttemptStore {
    dir: PathBuf,
}

struct JournalLock {
    file: File,
    dir: PathBuf,
}

impl Drop for JournalLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

impl MissionAttemptStore {
    /// Select a private dedicated subdirectory without creating local state.
    pub fn at(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// Begins or idempotently replays an attempt in exactly one scope.
    pub fn begin(
        &self,
        scope: &MissionAttemptScope,
        start: MissionAttemptStart,
    ) -> Result<MissionAttemptBegin> {
        validate_scope(scope)?;
        validate_start(&start)?;
        let (lock, mut journal) = self.locked(true)?;
        if let Some(stored) = journal.attempts.iter().find(|stored| {
            stored.scope == *scope && stored.attempt.start.request_id == start.request_id
        }) {
            if stored.attempt.start != start {
                return Err(MissionAttemptError::Conflict);
            }
            return Ok(MissionAttemptBegin {
                inserted: false,
                attempt: stored.attempt.clone(),
            });
        }
        evict_for_one_slot(&mut journal)?;
        let attempt_id = mint_attempt_id(&journal)?;
        let now = unix_seconds()?;
        let attempt = MissionAttempt {
            attempt_id,
            start,
            status: MissionAttemptStatus::Running,
            created_at_unix_seconds: now,
            updated_at_unix_seconds: now,
            trials: Vec::new(),
            trial_count: 0,
            terminal_reason: None,
            content_revoked: false,
        };
        journal.attempts.push(ScopedAttempt {
            scope: scope.clone(),
            attempt: attempt.clone(),
        });
        self.save_content(&lock, &journal)?;
        Ok(MissionAttemptBegin {
            inserted: true,
            attempt,
        })
    }

    /// Reads one attempt only from the supplied practice or account scope.
    pub fn get(&self, scope: &MissionAttemptScope, attempt_id: Uuid) -> Result<MissionAttempt> {
        validate_scope(scope)?;
        validate_uuid(attempt_id)?;
        let Some((_lock, journal)) = self.existing_locked(LockMode::Shared)? else {
            return Err(MissionAttemptError::NotFound);
        };
        find_attempt(&journal, scope, attempt_id)
            .map(|stored| stored.attempt.clone())
            .ok_or(MissionAttemptError::NotFound)
    }

    /// Lists content-free attempt summaries from one scope.
    pub fn list(&self, scope: &MissionAttemptScope) -> Result<Vec<MissionAttemptSummary>> {
        validate_scope(scope)?;
        let Some((_lock, journal)) = self.existing_locked(LockMode::Shared)? else {
            return Ok(Vec::new());
        };
        let mut summaries = journal
            .attempts
            .iter()
            .filter(|stored| stored.scope == *scope)
            .map(|stored| summary(&stored.attempt))
            .collect::<Vec<_>>();
        summaries.sort_by_key(|entry| (entry.created_at_unix_seconds, entry.attempt_id));
        Ok(summaries)
    }

    /// Persists one bounded expected trial before it enters a final report.
    pub fn record_trial(
        &self,
        scope: &MissionAttemptScope,
        attempt_id: Uuid,
        trial: SkillTrialResult,
    ) -> Result<MissionAttempt> {
        validate_scope(scope)?;
        validate_uuid(attempt_id)?;
        let trial_bytes =
            serde_json::to_vec(&trial).map_err(|_| MissionAttemptError::TrialInvalid)?;
        if trial_bytes.len() > MAX_TRIAL_BYTES {
            return Err(MissionAttemptError::TrialInvalid);
        }
        let (lock, mut journal) = self.locked_existing()?;
        let stored = find_attempt_mut(&mut journal, scope, attempt_id)
            .ok_or(MissionAttemptError::NotFound)?;
        if stored.attempt.status.is_terminal() || stored.attempt.content_revoked {
            return Err(MissionAttemptError::InvalidState);
        }
        if !matches!(
            stored.attempt.status,
            MissionAttemptStatus::Running | MissionAttemptStatus::CancelRequested
        ) || trial.served_model != stored.attempt.start.requested_model
        {
            return Err(MissionAttemptError::TrialInvalid);
        }
        let key = MissionTrialKey {
            task_id: trial.task_id.clone(),
            arm: trial.arm,
        };
        if !stored.attempt.start.expected_trials.contains(&key) {
            return Err(MissionAttemptError::TrialInvalid);
        }
        if let Some(existing) = stored
            .attempt
            .trials
            .iter()
            .find(|existing| existing.task_id == trial.task_id && existing.arm == trial.arm)
        {
            if existing == &trial {
                return Ok(stored.attempt.clone());
            }
            return Err(MissionAttemptError::Conflict);
        }
        if stored.attempt.trial_count >= EXPECTED_TRIALS {
            return Err(MissionAttemptError::TrialInvalid);
        }
        stored.attempt.trials.push(trial);
        stored.attempt.trial_count += 1;
        stored.attempt.updated_at_unix_seconds =
            next_timestamp(stored.attempt.updated_at_unix_seconds)?;
        let result = stored.attempt.clone();
        self.save_content(&lock, &journal)?;
        Ok(result)
    }

    /// Durably requests cancellation without discarding in-flight trial results.
    pub fn request_cancel(
        &self,
        scope: &MissionAttemptScope,
        attempt_id: Uuid,
    ) -> Result<MissionAttempt> {
        validate_scope(scope)?;
        validate_uuid(attempt_id)?;
        let (lock, mut journal) = self.locked_existing()?;
        let stored = find_attempt_mut(&mut journal, scope, attempt_id)
            .ok_or(MissionAttemptError::NotFound)?;
        if stored.attempt.status != MissionAttemptStatus::Running {
            return Ok(stored.attempt.clone());
        }
        stored.attempt.status = MissionAttemptStatus::CancelRequested;
        stored.attempt.updated_at_unix_seconds =
            next_timestamp(stored.attempt.updated_at_unix_seconds)?;
        let result = stored.attempt.clone();
        self.save_receipt(&lock, &journal)?;
        Ok(result)
    }

    /// Records one allowed terminal transition and its stable reason.
    pub fn finish(
        &self,
        scope: &MissionAttemptScope,
        attempt_id: Uuid,
        status: MissionAttemptStatus,
        reason: &str,
    ) -> Result<MissionAttempt> {
        validate_scope(scope)?;
        validate_uuid(attempt_id)?;
        validate_finish(status, reason)?;
        let (lock, mut journal) = self.locked_existing()?;
        let stored = find_attempt_mut(&mut journal, scope, attempt_id)
            .ok_or(MissionAttemptError::NotFound)?;
        if stored.attempt.status.is_terminal() {
            if stored.attempt.status == status
                && stored.attempt.terminal_reason.as_deref() == Some(reason)
            {
                return Ok(stored.attempt.clone());
            }
            return Err(MissionAttemptError::Conflict);
        }
        let transition_allowed = match status {
            MissionAttemptStatus::Completed => {
                stored.attempt.status == MissionAttemptStatus::Running
                    && stored.attempt.trial_count == EXPECTED_TRIALS
                    && keys_complete(&stored.attempt)
            }
            MissionAttemptStatus::Failed => matches!(
                stored.attempt.status,
                MissionAttemptStatus::Running | MissionAttemptStatus::CancelRequested
            ),
            MissionAttemptStatus::Cancelled => {
                stored.attempt.status == MissionAttemptStatus::CancelRequested
            }
            _ => false,
        };
        if !transition_allowed {
            return Err(MissionAttemptError::InvalidState);
        }
        stored.attempt.status = status;
        stored.attempt.terminal_reason = Some(reason.to_string());
        stored.attempt.updated_at_unix_seconds =
            next_timestamp(stored.attempt.updated_at_unix_seconds)?;
        let result = stored.attempt.clone();
        self.save_receipt(&lock, &journal)?;
        Ok(result)
    }

    /// Startup-only recovery; the caller must already own the daemon exclusively.
    pub fn recover_interrupted(&self) -> Result<usize> {
        let Some((lock, mut journal)) = self.existing_locked(LockMode::Exclusive)? else {
            return Ok(0);
        };
        let now = unix_seconds()?;
        let mut recovered = 0;
        for stored in &mut journal.attempts {
            if matches!(
                stored.attempt.status,
                MissionAttemptStatus::Running | MissionAttemptStatus::CancelRequested
            ) {
                stored.attempt.status = MissionAttemptStatus::Interrupted;
                stored.attempt.terminal_reason = Some(INTERRUPTED_REASON.to_string());
                stored.attempt.updated_at_unix_seconds =
                    now.max(stored.attempt.updated_at_unix_seconds);
                recovered += 1;
            }
        }
        if recovered != 0 {
            self.save_receipt(&lock, &journal)?;
        }
        Ok(recovered)
    }

    /// Revokes terminal trial content while retaining lifecycle counts and hashes.
    pub fn revoke_content(
        &self,
        scope: &MissionAttemptScope,
        attempt_id: Uuid,
    ) -> Result<MissionAttempt> {
        validate_scope(scope)?;
        validate_uuid(attempt_id)?;
        let (lock, mut journal) = self.locked_existing()?;
        let stored = find_attempt_mut(&mut journal, scope, attempt_id)
            .ok_or(MissionAttemptError::NotFound)?;
        if !stored.attempt.status.is_terminal() {
            return Err(MissionAttemptError::InvalidState);
        }
        if stored.attempt.content_revoked {
            return Ok(stored.attempt.clone());
        }
        stored.attempt.trials.clear();
        stored.attempt.content_revoked = true;
        stored.attempt.updated_at_unix_seconds =
            next_timestamp(stored.attempt.updated_at_unix_seconds)?;
        let result = stored.attempt.clone();
        self.save_receipt(&lock, &journal)?;
        Ok(result)
    }

    fn existing_locked(
        &self,
        mode: LockMode,
    ) -> Result<Option<(JournalLock, MissionAttemptJournal)>> {
        match fs::symlink_metadata(&self.dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(_) => return Err(MissionAttemptError::StoreUnavailable),
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                return Err(MissionAttemptError::StoreUnavailable);
            }
            Ok(_) => {}
        }
        let journal_path = self.dir.join(JOURNAL_FILE);
        match fs::symlink_metadata(&journal_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(MissionAttemptError::StoreUnreadable),
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                Err(MissionAttemptError::StoreInvalid)
            }
            Ok(_) => self.locked_as(false, mode).map(Some),
        }
    }

    fn locked_existing(&self) -> Result<(JournalLock, MissionAttemptJournal)> {
        self.existing_locked(LockMode::Exclusive)?
            .ok_or(MissionAttemptError::NotFound)
    }

    fn locked(&self, create: bool) -> Result<(JournalLock, MissionAttemptJournal)> {
        self.locked_as(create, LockMode::Exclusive)
    }

    fn locked_as(
        &self,
        create: bool,
        mode: LockMode,
    ) -> Result<(JournalLock, MissionAttemptJournal)> {
        reject_leaf_symlink(&self.dir)?;
        if create {
            create_private_dir(&self.dir)?;
        }
        reject_leaf_symlink(&self.dir)?;
        let dir = self
            .dir
            .canonicalize()
            .map_err(|_| MissionAttemptError::StoreUnavailable)?;
        reject_symlinks(&dir)?;
        require_private_dir(&dir)?;

        let lock_path = dir.join(LOCK_FILE);
        reject_leaf_symlink(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let lock_file = options
            .open(&lock_path)
            .map_err(|_| MissionAttemptError::StoreUnavailable)?;
        require_regular_private_file(&lock_file)?;
        acquire(&lock_file, mode)?;
        let lock = JournalLock {
            file: lock_file,
            dir: dir.clone(),
        };

        let journal_path = dir.join(JOURNAL_FILE);
        reject_leaf_symlink(&journal_path)?;
        let journal = match fs::symlink_metadata(&journal_path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => MissionAttemptJournal {
                version: JOURNAL_VERSION,
                attempts: Vec::new(),
            },
            Err(_) => return Err(MissionAttemptError::StoreUnreadable),
            Ok(metadata) if !metadata.is_file() || metadata.file_type().is_symlink() => {
                return Err(MissionAttemptError::StoreInvalid);
            }
            Ok(_) => {
                let file =
                    File::open(&journal_path).map_err(|_| MissionAttemptError::StoreUnreadable)?;
                require_regular_private_file(&file)?;
                serde_json::from_slice(&bounded_read(file)?)
                    .map_err(|_| MissionAttemptError::StoreInvalid)?
            }
        };
        validate_journal(&journal)?;
        Ok((lock, journal))
    }

    fn save_content(&self, lock: &JournalLock, journal: &MissionAttemptJournal) -> Result<()> {
        self.save(lock, journal, MAX_CONTENT_JOURNAL_BYTES)
    }

    fn save_receipt(&self, lock: &JournalLock, journal: &MissionAttemptJournal) -> Result<()> {
        self.save(lock, journal, MAX_JOURNAL_BYTES)
    }

    fn save(
        &self,
        lock: &JournalLock,
        journal: &MissionAttemptJournal,
        max_bytes: u64,
    ) -> Result<()> {
        let bytes = serde_json::to_vec(journal).map_err(|_| MissionAttemptError::StoreInvalid)?;
        if bytes.len() as u64 > max_bytes {
            return Err(MissionAttemptError::StoreFull);
        }
        reject_leaf_symlink(&self.dir).map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        let dir = self
            .dir
            .canonicalize()
            .map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        if dir != lock.dir {
            return Err(MissionAttemptError::StoreWriteFailed);
        }
        reject_symlinks(&lock.dir).map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        require_private_dir(&lock.dir).map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        let path = lock.dir.join(JOURNAL_FILE);
        reject_leaf_symlink(&path).map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        crate::config::write_atomic_0600(&lock.dir, &path, &bytes)
            .map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        #[cfg(unix)]
        File::open(&lock.dir)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| MissionAttemptError::StoreWriteFailed)?;
        Ok(())
    }
}

/// How a journal operation holds the lock file.
///
/// Reads take a shared lock so a UI listing attempts cannot exclude another
/// reader, and writes retry a bounded number of times instead of surfacing
/// `StoreBusy` on the first collision. Before this, `list()` from a UI could
/// land on the observer write of a running evaluation and end a paid run with
/// `PersistenceUnavailable`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LockMode {
    Shared,
    Exclusive,
}

const LOCK_ATTEMPTS: usize = 64;
const LOCK_RETRY_DELAY: Duration = Duration::from_millis(4);

fn acquire(file: &File, mode: LockMode) -> Result<()> {
    for attempt in 0..LOCK_ATTEMPTS {
        let taken = match mode {
            LockMode::Shared => file.try_lock_shared().is_ok(),
            LockMode::Exclusive => file.try_lock().is_ok(),
        };
        if taken {
            return Ok(());
        }
        if attempt + 1 < LOCK_ATTEMPTS {
            std::thread::sleep(LOCK_RETRY_DELAY);
        }
    }
    Err(MissionAttemptError::StoreBusy)
}

/// Free one journal slot by discarding the oldest terminal attempts.
///
/// In-progress attempts are never discarded: their trials are the only record
/// of inference the contributor has already paid for. A journal whose every
/// slot holds a live attempt therefore still refuses with `StoreFull`, and
/// leaves the stored bytes untouched because the caller returns before saving.
/// Without this the 128th lifetime attempt wedged the store permanently and
/// only deleting the file by hand restored it.
fn evict_for_one_slot(journal: &mut MissionAttemptJournal) -> Result<()> {
    while journal.attempts.len() >= MAX_ATTEMPTS {
        let oldest = journal
            .attempts
            .iter()
            .enumerate()
            .filter(|(_, stored)| stored.attempt.status.is_terminal())
            // `min_by_key` keeps the first minimum, so attempts created within
            // the same second are evicted in the order they were recorded.
            .min_by_key(|(_, stored)| stored.attempt.created_at_unix_seconds)
            .map(|(index, _)| index)
            .ok_or(MissionAttemptError::StoreFull)?;
        journal.attempts.remove(oldest);
    }
    Ok(())
}

fn find_attempt<'a>(
    journal: &'a MissionAttemptJournal,
    scope: &MissionAttemptScope,
    attempt_id: Uuid,
) -> Option<&'a ScopedAttempt> {
    journal
        .attempts
        .iter()
        .find(|stored| stored.scope == *scope && stored.attempt.attempt_id == attempt_id)
}

fn find_attempt_mut<'a>(
    journal: &'a mut MissionAttemptJournal,
    scope: &MissionAttemptScope,
    attempt_id: Uuid,
) -> Option<&'a mut ScopedAttempt> {
    journal
        .attempts
        .iter_mut()
        .find(|stored| stored.scope == *scope && stored.attempt.attempt_id == attempt_id)
}

fn summary(attempt: &MissionAttempt) -> MissionAttemptSummary {
    MissionAttemptSummary {
        attempt_id: attempt.attempt_id,
        request_id: attempt.start.request_id,
        mission_id: attempt.start.mission_id,
        program_id: attempt.start.program_id,
        package_sha256: attempt.start.package_sha256.clone(),
        offer_version_hash: attempt.start.offer_version_hash.clone(),
        skill_sha256: attempt.start.skill_sha256.clone(),
        evaluation_contract_hash: attempt.start.evaluation_contract_hash.clone(),
        status: attempt.status,
        trial_count: attempt.trial_count,
        expected_trial_count: attempt.start.expected_trials.len(),
        created_at_unix_seconds: attempt.created_at_unix_seconds,
        updated_at_unix_seconds: attempt.updated_at_unix_seconds,
        content_revoked: attempt.content_revoked,
    }
}

fn validate_journal(journal: &MissionAttemptJournal) -> Result<()> {
    if journal.version != JOURNAL_VERSION {
        return Err(MissionAttemptError::StoreVersionUnsupported);
    }
    if journal.attempts.len() > MAX_ATTEMPTS {
        return Err(MissionAttemptError::StoreInvalid);
    }
    let mut attempt_ids = BTreeSet::new();
    let mut requests = BTreeSet::new();
    for stored in &journal.attempts {
        validate_scope(&stored.scope).map_err(|_| MissionAttemptError::StoreInvalid)?;
        validate_attempt(&stored.attempt)?;
        if !attempt_ids.insert(stored.attempt.attempt_id)
            || !requests.insert((stored.scope.clone(), stored.attempt.start.request_id))
        {
            return Err(MissionAttemptError::StoreInvalid);
        }
    }
    Ok(())
}

fn validate_attempt(attempt: &MissionAttempt) -> Result<()> {
    validate_uuid(attempt.attempt_id).map_err(|_| MissionAttemptError::StoreInvalid)?;
    validate_start(&attempt.start).map_err(|_| MissionAttemptError::StoreInvalid)?;
    if attempt.updated_at_unix_seconds < attempt.created_at_unix_seconds
        || attempt.trial_count > EXPECTED_TRIALS
    {
        return Err(MissionAttemptError::StoreInvalid);
    }
    let expected = attempt
        .start
        .expected_trials
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    if attempt.content_revoked {
        if !attempt.trials.is_empty() || !attempt.status.is_terminal() {
            return Err(MissionAttemptError::StoreInvalid);
        }
    } else if attempt.trials.len() != attempt.trial_count {
        return Err(MissionAttemptError::StoreInvalid);
    }
    if !attempt.content_revoked {
        let mut trial_keys = BTreeSet::new();
        for trial in &attempt.trials {
            if trial.served_model != attempt.start.requested_model
                || serde_json::to_vec(trial)
                    .map_err(|_| MissionAttemptError::StoreInvalid)?
                    .len()
                    > MAX_TRIAL_BYTES
            {
                return Err(MissionAttemptError::StoreInvalid);
            }
            if !trial_keys.insert(MissionTrialKey {
                task_id: trial.task_id.clone(),
                arm: trial.arm,
            }) {
                return Err(MissionAttemptError::StoreInvalid);
            }
        }
        if !trial_keys.is_subset(&expected) {
            return Err(MissionAttemptError::StoreInvalid);
        }
    }
    match attempt.status {
        MissionAttemptStatus::Running | MissionAttemptStatus::CancelRequested => {
            if attempt.terminal_reason.is_some() {
                return Err(MissionAttemptError::StoreInvalid);
            }
        }
        MissionAttemptStatus::Completed => {
            if attempt.terminal_reason.as_deref() != Some(COMPLETED_REASON)
                || !keys_complete(attempt)
            {
                return Err(MissionAttemptError::StoreInvalid);
            }
        }
        MissionAttemptStatus::Failed => {
            let Some(reason) = attempt.terminal_reason.as_deref() else {
                return Err(MissionAttemptError::StoreInvalid);
            };
            validate_reason(reason).map_err(|_| MissionAttemptError::StoreInvalid)?;
        }
        MissionAttemptStatus::Cancelled => {
            if attempt.terminal_reason.as_deref() != Some(CANCELLED_REASON) {
                return Err(MissionAttemptError::StoreInvalid);
            }
        }
        MissionAttemptStatus::Interrupted => {
            if attempt.terminal_reason.as_deref() != Some(INTERRUPTED_REASON) {
                return Err(MissionAttemptError::StoreInvalid);
            }
        }
    }
    Ok(())
}

fn validate_scope(scope: &MissionAttemptScope) -> Result<()> {
    if let MissionAttemptScope::Account { owner_sha256 } = scope {
        validate_digest(owner_sha256)?;
    }
    Ok(())
}

fn validate_start(start: &MissionAttemptStart) -> Result<()> {
    for id in [
        start.request_id,
        start.approval_id,
        start.mission_id,
        start.program_id,
    ] {
        validate_uuid(id)?;
    }
    for digest in [
        &start.package_sha256,
        &start.skill_sha256,
        &start.evaluation_contract_hash,
    ] {
        validate_digest(digest)?;
    }
    validate_offer_version_hash(&start.offer_version_hash)?;
    validate_text(&start.requested_model, MAX_MODEL_CHARS)?;
    if start.expected_trials.len() != EXPECTED_TRIALS {
        return Err(MissionAttemptError::FieldInvalid);
    }
    let mut by_task = BTreeMap::<&str, BTreeSet<EvaluationArm>>::new();
    let mut unique = BTreeSet::new();
    for key in &start.expected_trials {
        validate_text(&key.task_id, MAX_TASK_ID_CHARS)?;
        if !unique.insert(key.clone()) {
            return Err(MissionAttemptError::FieldInvalid);
        }
        by_task.entry(&key.task_id).or_default().insert(key.arm);
    }
    let all_arms = BTreeSet::from(EvaluationArm::ALL);
    if by_task.len() != EXPECTED_TRIALS / all_arms.len()
        || by_task.values().any(|arms| arms != &all_arms)
    {
        return Err(MissionAttemptError::FieldInvalid);
    }
    Ok(())
}

fn validate_finish(status: MissionAttemptStatus, reason: &str) -> Result<()> {
    match status {
        MissionAttemptStatus::Completed if reason == COMPLETED_REASON => Ok(()),
        MissionAttemptStatus::Cancelled if reason == CANCELLED_REASON => Ok(()),
        MissionAttemptStatus::Failed => validate_reason(reason),
        _ => Err(MissionAttemptError::FieldInvalid),
    }
}

fn validate_reason(reason: &str) -> Result<()> {
    if reason.is_empty()
        || reason.len() > 96
        || !reason
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    {
        return Err(MissionAttemptError::FieldInvalid);
    }
    Ok(())
}

fn validate_uuid(id: Uuid) -> Result<()> {
    if id.is_nil() {
        return Err(MissionAttemptError::FieldInvalid);
    }
    Ok(())
}

fn validate_digest(value: &str) -> Result<()> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(MissionAttemptError::FieldInvalid);
    }
    Ok(())
}

fn validate_offer_version_hash(value: &str) -> Result<()> {
    let digest = value
        .strip_prefix("sha256:")
        .ok_or(MissionAttemptError::FieldInvalid)?;
    validate_digest(digest)
}

fn validate_text(value: &str, max_chars: usize) -> Result<()> {
    let count = value.chars().count();
    if count == 0
        || count > max_chars
        || value.trim() != value
        || value.chars().any(char::is_control)
    {
        return Err(MissionAttemptError::FieldInvalid);
    }
    Ok(())
}

fn keys_complete(attempt: &MissionAttempt) -> bool {
    attempt.trial_count == EXPECTED_TRIALS
        && (attempt.content_revoked
            || attempt
                .trials
                .iter()
                .map(|trial| MissionTrialKey {
                    task_id: trial.task_id.clone(),
                    arm: trial.arm,
                })
                .collect::<BTreeSet<_>>()
                == attempt
                    .start
                    .expected_trials
                    .iter()
                    .cloned()
                    .collect::<BTreeSet<_>>())
}

fn mint_attempt_id(journal: &MissionAttemptJournal) -> Result<Uuid> {
    for _ in 0..8 {
        let id = Uuid::new_v4();
        if !id.is_nil()
            && journal
                .attempts
                .iter()
                .all(|stored| stored.attempt.attempt_id != id)
        {
            return Ok(id);
        }
    }
    Err(MissionAttemptError::StoreUnavailable)
}

fn unix_seconds() -> Result<u64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| MissionAttemptError::ClockUnavailable)
}

fn next_timestamp(previous: u64) -> Result<u64> {
    Ok(unix_seconds()?.max(previous))
}

fn bounded_read(file: File) -> Result<Vec<u8>> {
    if file
        .metadata()
        .map_err(|_| MissionAttemptError::StoreUnreadable)?
        .len()
        > MAX_JOURNAL_BYTES
    {
        return Err(MissionAttemptError::StoreInvalid);
    }
    let mut bytes = Vec::new();
    file.take(MAX_JOURNAL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| MissionAttemptError::StoreUnreadable)?;
    if bytes.len() as u64 > MAX_JOURNAL_BYTES {
        return Err(MissionAttemptError::StoreInvalid);
    }
    Ok(bytes)
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(MissionAttemptError::StoreUnavailable);
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(MissionAttemptError::StoreUnavailable),
        }
    }
    Ok(())
}

fn reject_leaf_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(MissionAttemptError::StoreUnavailable)
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(MissionAttemptError::StoreUnavailable),
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|_| MissionAttemptError::StoreUnavailable)?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path).map_err(|_| MissionAttemptError::StoreUnavailable)?;
    #[cfg(windows)]
    windows_private_dir::apply_owner_only_dacl(path)?;
    Ok(())
}

/// Refuse a journal directory any other local account can read.
///
/// On Unix that is the 0700 mode bit. Windows has no equivalent bit, so the
/// directory carries a protected DACL naming exactly one trustee, built the
/// same way the daemon named pipe's descriptor is. Before this, the control
/// simply did not exist on Windows while the journal held raw model
/// completions, and every other platform's `RequiresPrivateDirectory` refusal
/// was unreachable there.
fn require_private_dir(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(_path)
            .map_err(|_| MissionAttemptError::StoreUnavailable)?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(MissionAttemptError::RequiresPrivateDirectory);
        }
    }
    #[cfg(windows)]
    windows_private_dir::require_owner_only_dacl(_path)?;
    Ok(())
}

#[cfg(windows)]
mod windows_private_dir {
    //! The Windows half of the journal's private-directory control.
    //!
    //! The descriptor is `D:P(A;OICI;FA;;;<user sid>)`, the file-object twin of
    //! the daemon pipe's `D:P(A;;GA;;;<sid>)`: `P` protects it so no
    //! inheritable entry from an ancestor can widen access afterwards, and
    //! `OICI` makes the single allow entry inherit to the journal and lock
    //! files created inside. There is deliberately no Administrators entry --
    //! an administrator can already take ownership, so one would grant nothing
    //! while making the intent harder to read.
    //!
    //! Verification compares the directory's DACL bytes against the bytes of a
    //! freshly built expected DACL rather than re-rendering SDDL, so a widened
    //! or reordered ACL cannot round-trip to the same string and pass.

    use std::ffi::c_void;
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{
        ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW,
        SDDL_REVISION_1, SE_FILE_OBJECT, SetNamedSecurityInfoW,
    };
    use windows_sys::Win32::Security::{
        ACE_HEADER, ACL, DACL_SECURITY_INFORMATION, GetAce, GetSecurityDescriptorControl,
        GetSecurityDescriptorDacl, PROTECTED_DACL_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR,
        SE_DACL_PROTECTED,
    };

    use super::{MissionAttemptError, Result};

    /// Releases a descriptor allocated by Win32 on drop.
    struct OwnedDescriptor(PSECURITY_DESCRIPTOR);

    impl Drop for OwnedDescriptor {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: both producers below document `LocalFree` as the
                // release function, and this is the only place it is called.
                unsafe { LocalFree(self.0.cast::<c_void>()) };
            }
        }
    }

    fn refusal() -> MissionAttemptError {
        MissionAttemptError::RequiresPrivateDirectory
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn owner_only_descriptor() -> Result<OwnedDescriptor> {
        let sid = crate::daemon::win_pipe::current_user_sid_string().map_err(|_| refusal())?;
        let sddl = format!("D:P(A;OICI;FA;;;{sid})");
        let text: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `text` is a NUL-terminated wide string that outlives the
        // call, `descriptor` is a valid out-pointer, and a null size
        // out-pointer is permitted.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                text.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                std::ptr::null_mut(),
            )
        };
        if converted == 0 || descriptor.is_null() {
            return Err(refusal());
        }
        Ok(OwnedDescriptor(descriptor))
    }

    fn dacl_of(descriptor: PSECURITY_DESCRIPTOR) -> Result<*mut ACL> {
        let mut present = 0;
        let mut defaulted = 0;
        let mut dacl: *mut ACL = std::ptr::null_mut();
        // SAFETY: `descriptor` is a live descriptor and all three out-pointers
        // address writable storage of the required types.
        let read = unsafe {
            GetSecurityDescriptorDacl(descriptor, &mut present, &mut dacl, &mut defaulted)
        };
        if read == 0 || present == 0 || dacl.is_null() {
            // A descriptor with no DACL grants everyone everything.
            return Err(refusal());
        }
        Ok(dacl)
    }

    /// The bytes of the single entry a compliant DACL carries.
    ///
    /// Compared entry by entry rather than over the whole ACL: `AclSize` is the
    /// allocated size, so a padded copy of the identical ACL would not match
    /// byte for byte and the control would refuse a directory that is in fact
    /// private.
    fn sole_ace_bytes(acl: *const ACL) -> Result<Vec<u8>> {
        // SAFETY: `acl` points at a well-formed ACL header.
        if unsafe { (*acl).AceCount } != 1 {
            // More than one trustee, or none at all.
            return Err(refusal());
        }
        let mut entry: *mut c_void = std::ptr::null_mut();
        // SAFETY: index 0 exists because the count above is one, and `entry` is
        // a valid out-pointer.
        let read = unsafe { GetAce(acl, 0, &mut entry) };
        if read == 0 || entry.is_null() {
            return Err(refusal());
        }
        // SAFETY: every ACE begins with an ACE_HEADER recording its own size.
        let size = usize::from(unsafe { (*entry.cast::<ACE_HEADER>()).AceSize });
        // SAFETY: the entry occupies `size` contiguous bytes, as its own header
        // records.
        Ok(unsafe { std::slice::from_raw_parts(entry.cast::<u8>(), size) }.to_vec())
    }

    pub(super) fn apply_owner_only_dacl(path: &Path) -> Result<()> {
        let descriptor = owner_only_descriptor()?;
        let dacl = dacl_of(descriptor.0)?;
        let path = wide(path);
        // SAFETY: `path` is a NUL-terminated wide string and `dacl` points into
        // `descriptor`, which outlives this synchronous call.
        let status = unsafe {
            SetNamedSecurityInfoW(
                path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                dacl,
                std::ptr::null(),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(refusal());
        }
        Ok(())
    }

    pub(super) fn require_owner_only_dacl(path: &Path) -> Result<()> {
        let expected = owner_only_descriptor()?;
        let expected_entry = sole_ace_bytes(dacl_of(expected.0)?)?;

        let wide_path = wide(path);
        let mut dacl: *mut ACL = std::ptr::null_mut();
        let mut descriptor: PSECURITY_DESCRIPTOR = std::ptr::null_mut();
        // SAFETY: `wide_path` is a NUL-terminated wide string that outlives the
        // call and both out-pointers address writable storage.
        let status = unsafe {
            GetNamedSecurityInfoW(
                wide_path.as_ptr(),
                SE_FILE_OBJECT,
                DACL_SECURITY_INFORMATION,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut dacl,
                std::ptr::null_mut(),
                &mut descriptor,
            )
        };
        let owned = OwnedDescriptor(descriptor);
        if status != ERROR_SUCCESS || owned.0.is_null() || dacl.is_null() {
            return Err(refusal());
        }

        let mut control = 0u16;
        let mut revision = 0u32;
        // SAFETY: `owned.0` is a live descriptor and both out-pointers address
        // writable storage of the required types.
        let read = unsafe { GetSecurityDescriptorControl(owned.0, &mut control, &mut revision) };
        if read == 0 || control & SE_DACL_PROTECTED == 0 {
            // An unprotected DACL can be widened by an inherited entry.
            return Err(refusal());
        }
        if sole_ace_bytes(dacl)? != expected_entry {
            return Err(refusal());
        }
        Ok(())
    }
}

fn require_regular_private_file(file: &File) -> Result<()> {
    let metadata = file
        .metadata()
        .map_err(|_| MissionAttemptError::StoreUnreadable)?;
    if !metadata.is_file() {
        return Err(MissionAttemptError::StoreInvalid);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(MissionAttemptError::StoreInvalid);
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "mission_attempt_tests.rs"]
mod tests;
