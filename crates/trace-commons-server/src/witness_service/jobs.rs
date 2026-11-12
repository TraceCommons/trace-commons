// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The job table and the sealed result store behind `POST /v1/witness/jobs`.
//!
//! See `docs/superpowers/specs/2026-11-05-witness-submit-then-poll.md`.
//!
//! - Raw sessions never reach this module. The submit handler holds the
//!   body in memory and hands it to the execution task; this module sees
//!   only finished answers.
//! - Finished answers are sealed to disk with AES-256-GCM under
//!   [`results_key`], which binds a dstack-derived secret to the boot
//!   measurement. A file that does not open is deleted, never served.
//! - Jobs are keyed by `sha256(job_id)`. Neither a job id nor a fetch
//!   secret is held raw, written, or logged.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use aes_gcm::aead::{Aead, Payload};
use aes_gcm::{Aes256Gcm, KeyInit, Nonce};
use rand::RngCore;
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use super::prefetch::ClassifyQueue;

/// A 32-byte digest: `sha256(job_id)` or `sha256(fetch_secret)`.
pub type Digest32 = [u8; 32];

const RESULTS_KEY_SALT: &[u8] = b"tcwitness-results-v1";
const SEALED_VERSION: u8 = 1;
const SEALED_AAD_TAG: &[u8] = b"tcwitness-result-v1";
const SEALED_SUFFIX: &str = "sealed";
const TEMP_SUFFIX: &str = "tmp";
const NONCE_LEN: usize = 12;

/// How long a finished, unfetched result is kept.
pub const RESULT_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// The sealed-results key: HKDF-SHA256 over a dstack `GetKey` secret, with
/// the boot measurement label as `info`.
///
/// `GetKey` derives from the app id alone, so every image of one app gets
/// the same secret. Binding the measurement is what makes an image with a
/// different measurement derive a different key and fail to open the
/// results an earlier image sealed.
pub fn results_key(secret: &[u8; 32], measurement: &str) -> Zeroizing<[u8; 32]> {
    let hkdf = hkdf::Hkdf::<Sha256>::new(Some(RESULTS_KEY_SALT), secret);
    let mut key = Zeroizing::new([0u8; 32]);
    hkdf.expand(measurement.as_bytes(), key.as_mut_slice())
        .expect("32 bytes is a valid HKDF-SHA256 output length");
    key
}

/// `sha256(bytes)`.
pub fn sha256(bytes: &[u8]) -> Digest32 {
    Sha256::digest(bytes).into()
}

/// Compare two digests without an early exit.
pub fn digests_equal(a: &Digest32, b: &Digest32) -> bool {
    let mut difference = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        difference |= x ^ y;
    }
    std::hint::black_box(difference) == 0
}

/// Why the store refused. Label-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum StoreError {
    #[error("the results directory is unusable")]
    DirectoryUnusable,
    #[error("a sealed result could not be written")]
    WriteFailed,
    #[error("a sealed result could not be read")]
    ReadFailed,
    #[error("a sealed result did not open")]
    OpenFailed,
}

/// A finished answer, as it is sealed and served.
#[derive(Clone, PartialEq, Eq)]
pub struct JobResult {
    pub status: u16,
    /// Only the headers a client reads: content type, certificate,
    /// signature, `Retry-After`.
    pub headers: Vec<(String, Vec<u8>)>,
    pub body: Vec<u8>,
}

impl std::fmt::Debug for JobResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JobResult")
            .field("status", &self.status)
            .field("headers", &self.headers.len())
            .field("body", &"<withheld>")
            .finish()
    }
}

/// What a sealed file holds once opened.
struct SealedRecord {
    secret_hash: Digest32,
    expires_unix: u64,
    result: JobResult,
}

impl SealedRecord {
    fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(64 + self.result.body.len());
        out.extend_from_slice(&self.secret_hash);
        out.extend_from_slice(&self.expires_unix.to_be_bytes());
        out.extend_from_slice(&self.result.status.to_be_bytes());
        out.extend_from_slice(&(self.result.headers.len() as u16).to_be_bytes());
        for (name, value) in &self.result.headers {
            out.extend_from_slice(&(name.len() as u16).to_be_bytes());
            out.extend_from_slice(name.as_bytes());
            out.extend_from_slice(&(value.len() as u32).to_be_bytes());
            out.extend_from_slice(value);
        }
        out.extend_from_slice(&self.result.body);
        out
    }

    fn decode(bytes: &[u8]) -> Option<Self> {
        struct Reader<'a>(&'a [u8]);
        impl<'a> Reader<'a> {
            fn take(&mut self, n: usize) -> Option<&'a [u8]> {
                if self.0.len() < n {
                    return None;
                }
                let (head, rest) = self.0.split_at(n);
                self.0 = rest;
                Some(head)
            }
        }
        let mut reader = Reader(bytes);
        let secret_hash: Digest32 = reader.take(32)?.try_into().ok()?;
        let expires_unix = u64::from_be_bytes(reader.take(8)?.try_into().ok()?);
        let status = u16::from_be_bytes(reader.take(2)?.try_into().ok()?);
        let count = u16::from_be_bytes(reader.take(2)?.try_into().ok()?);
        let mut headers = Vec::with_capacity(usize::from(count));
        for _ in 0..count {
            let name_len = u16::from_be_bytes(reader.take(2)?.try_into().ok()?);
            let name = std::str::from_utf8(reader.take(usize::from(name_len))?)
                .ok()?
                .to_string();
            let value_len = u32::from_be_bytes(reader.take(4)?.try_into().ok()?);
            let value = reader.take(value_len as usize)?.to_vec();
            headers.push((name, value));
        }
        Some(Self {
            secret_hash,
            expires_unix,
            result: JobResult {
                status,
                headers,
                body: reader.0.to_vec(),
            },
        })
    }
}

/// Finished answers on disk, encrypted.
pub struct ResultSealer {
    dir: PathBuf,
    key: Zeroizing<[u8; 32]>,
}

impl ResultSealer {
    /// Use `dir`, creating it if needed, and prove it is writable before
    /// anything depends on it. A witness whose store does not work must not
    /// start.
    pub fn open(dir: impl Into<PathBuf>, key: Zeroizing<[u8; 32]>) -> Result<Self, StoreError> {
        let dir = dir.into();
        std::fs::create_dir_all(&dir).map_err(|_| StoreError::DirectoryUnusable)?;
        let probe = dir.join(format!(".probe.{TEMP_SUFFIX}"));
        std::fs::write(&probe, b"probe").map_err(|_| StoreError::DirectoryUnusable)?;
        std::fs::remove_file(&probe).map_err(|_| StoreError::DirectoryUnusable)?;
        Ok(Self { dir, key })
    }

    fn path(&self, id_hash: &Digest32) -> PathBuf {
        self.dir
            .join(format!("{}.{SEALED_SUFFIX}", hex::encode(id_hash)))
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.key.as_slice()).expect("the key is 32 bytes")
    }

    fn aad(id_hash: &Digest32) -> Vec<u8> {
        let mut aad = SEALED_AAD_TAG.to_vec();
        aad.extend_from_slice(id_hash);
        aad
    }

    fn seal(&self, id_hash: &Digest32, record: &SealedRecord) -> Result<(), StoreError> {
        let mut nonce = [0u8; NONCE_LEN];
        rand::rngs::OsRng.fill_bytes(&mut nonce);
        let plaintext = Zeroizing::new(record.encode());
        let ciphertext = self
            .cipher()
            .encrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &plaintext,
                    aad: &Self::aad(id_hash),
                },
            )
            .map_err(|_| StoreError::WriteFailed)?;
        let mut file = Vec::with_capacity(1 + NONCE_LEN + ciphertext.len());
        file.push(SEALED_VERSION);
        file.extend_from_slice(&nonce);
        file.extend_from_slice(&ciphertext);
        let target = self.path(id_hash);
        let temp = target.with_extension(TEMP_SUFFIX);
        let written = std::fs::write(&temp, &file).and_then(|()| std::fs::rename(&temp, &target));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
            return Err(StoreError::WriteFailed);
        }
        Ok(())
    }

    fn open_file(&self, id_hash: &Digest32, file: &[u8]) -> Result<SealedRecord, StoreError> {
        let (&version, rest) = file.split_first().ok_or(StoreError::OpenFailed)?;
        if version != SEALED_VERSION || rest.len() < NONCE_LEN {
            return Err(StoreError::OpenFailed);
        }
        let (nonce, ciphertext) = rest.split_at(NONCE_LEN);
        let plaintext = Zeroizing::new(
            self.cipher()
                .decrypt(
                    Nonce::from_slice(nonce),
                    Payload {
                        msg: ciphertext,
                        aad: &Self::aad(id_hash),
                    },
                )
                .map_err(|_| StoreError::OpenFailed)?,
        );
        SealedRecord::decode(&plaintext).ok_or(StoreError::OpenFailed)
    }

    fn read(&self, id_hash: &Digest32) -> Result<SealedRecord, StoreError> {
        let file = std::fs::read(self.path(id_hash)).map_err(|_| StoreError::ReadFailed)?;
        self.open_file(id_hash, &file)
    }

    fn remove(&self, id_hash: &Digest32) {
        let _ = std::fs::remove_file(self.path(id_hash));
    }

    /// Every sealed file that opens under this key and has not expired, as
    /// `(id hash, secret hash, expiry)`. Everything else in the directory
    /// that this store wrote -- files another measurement sealed, expired
    /// ones, half-written temporaries -- is deleted.
    fn recover(&self, now_unix: u64) -> Vec<(Digest32, Digest32, u64)> {
        let mut kept = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return kept;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if name.ends_with(&format!(".{TEMP_SUFFIX}")) {
                let _ = std::fs::remove_file(&path);
                continue;
            }
            let Some(stem) = name.strip_suffix(&format!(".{SEALED_SUFFIX}")) else {
                continue;
            };
            let id_hash: Option<Digest32> = hex::decode(stem)
                .ok()
                .and_then(|bytes| bytes.try_into().ok());
            let Some(id_hash) = id_hash else {
                let _ = std::fs::remove_file(&path);
                continue;
            };
            match std::fs::read(&path)
                .map_err(|_| StoreError::ReadFailed)
                .and_then(|file| self.open_file(&id_hash, &file))
            {
                Ok(record) if record.expires_unix > now_unix => {
                    kept.push((id_hash, record.secret_hash, record.expires_unix));
                }
                _ => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
        kept
    }
}

/// Limits on the table.
#[derive(Debug, Clone, Copy)]
pub struct JobLimits {
    /// Accepted, unfinished jobs.
    pub max_active: usize,
    /// Accepted, unfinished background jobs.
    pub max_active_background: usize,
    /// Finished, unfetched results on disk.
    pub max_stored: usize,
    /// The largest finished answer that is sealed.
    pub max_result_bytes: usize,
    /// How long a finished, unfetched result is kept.
    pub result_ttl: Duration,
}

enum Slot {
    Active {
        seq: u64,
        background: bool,
        secret_hash: Digest32,
        abort: Option<tokio::task::AbortHandle>,
    },
    Done {
        secret_hash: Digest32,
        expires_unix: u64,
    },
}

#[derive(Default)]
struct Table {
    slots: HashMap<Digest32, Slot>,
    active: usize,
    active_background: usize,
    done: usize,
}

impl Table {
    fn remove(&mut self, id_hash: &Digest32) -> Option<Slot> {
        let slot = self.slots.remove(id_hash)?;
        match &slot {
            Slot::Active { background, .. } => {
                self.active -= 1;
                if *background {
                    self.active_background -= 1;
                }
            }
            Slot::Done { .. } => self.done -= 1,
        }
        Some(slot)
    }
}

/// Why a submit was not accepted. Both are the existing busy answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdmitRefusal {
    /// The table, the background share or the store is full.
    Full,
    /// The jobs ahead would outlast the job's deadline at the recent pace.
    WouldExpire,
}

/// An accepted job, before its task is spawned.
pub struct Admitted {
    /// The job id, hex. Returned to the client once and never kept.
    pub job_id: String,
    pub id_hash: Digest32,
    pub queue_position: u32,
    pub poll_after_secs: u32,
}

/// What a poll finds.
#[derive(Debug)]
pub enum PollOutcome {
    Pending {
        queue_position: u32,
        poll_after_secs: u32,
    },
    Done(JobResult),
    Gone,
}

/// Label-only counts, for diagnostics. Never an id.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct JobCounts {
    /// Accepted and waiting for the classify turn.
    pub pending: usize,
    /// Accepted and past the wait.
    pub running: usize,
    /// Finished and sealed, not yet fetched.
    pub done_unfetched: usize,
    /// Results deleted unfetched at expiry since start.
    pub expired: u64,
}

/// A clock in Unix seconds, injectable for tests.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

fn system_clock() -> Clock {
    Arc::new(|| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    })
}

/// The table of jobs on this instance.
pub struct WitnessJobs {
    table: Mutex<Table>,
    sealer: ResultSealer,
    limits: JobLimits,
    deadline: Duration,
    queue: Option<Arc<ClassifyQueue>>,
    instance_id: Option<String>,
    next_seq: AtomicU64,
    expired: AtomicU64,
    clock: Clock,
}

impl WitnessJobs {
    /// A table over `sealer`, with every result a previous run of this
    /// measurement left unfetched indexed back in.
    pub fn new(sealer: ResultSealer, limits: JobLimits, deadline: Duration) -> Self {
        Self::with_clock(sealer, limits, deadline, system_clock())
    }

    /// [`Self::new`] with a clock.
    pub fn with_clock(
        sealer: ResultSealer,
        limits: JobLimits,
        deadline: Duration,
        clock: Clock,
    ) -> Self {
        let mut table = Table::default();
        let mut recovered = sealer.recover(clock());
        // Oldest first past the cap is arbitrary; any choice is fine as long
        // as the store stays inside its bound.
        recovered.sort_by_key(|(_, _, expires)| *expires);
        let overflow = recovered.len().saturating_sub(limits.max_stored);
        for (id_hash, _, _) in recovered.drain(..overflow) {
            sealer.remove(&id_hash);
        }
        for (id_hash, secret_hash, expires_unix) in recovered {
            table.slots.insert(
                id_hash,
                Slot::Done {
                    secret_hash,
                    expires_unix,
                },
            );
            table.done += 1;
        }
        Self {
            table: Mutex::new(table),
            sealer,
            limits,
            deadline,
            queue: None,
            instance_id: None,
            next_seq: AtomicU64::new(0),
            expired: AtomicU64::new(0),
            clock,
        }
    }

    /// Estimate waits from, and refuse jobs the deadline cannot cover by,
    /// the classify queue's recent pace.
    pub fn with_classify_queue(mut self, queue: Arc<ClassifyQueue>) -> Self {
        self.queue = Some(queue);
        self
    }

    /// Name this instance in every `202`, for instance-addressed polling.
    pub fn with_instance_id(mut self, instance_id: Option<String>) -> Self {
        self.instance_id =
            instance_id.filter(|id| trace_commons_protocol::witness_jobs::is_instance_id(id));
        self
    }

    pub fn instance_id(&self) -> Option<&str> {
        self.instance_id.as_deref()
    }

    pub fn limits(&self) -> JobLimits {
        self.limits
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Table> {
        // A panic while holding the lock leaves counters that are still
        // consistent (every mutation is a single insert/remove pair), so
        // the table stays usable.
        self.table
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
    }

    fn poll_after(&self, position: u32) -> u32 {
        match self.queue.as_ref().and_then(|q| q.mean_turn()) {
            Some(turn) => {
                let estimate = turn.saturating_mul(position + 1).as_secs_f64() / 2.0;
                (estimate.ceil() as u32).clamp(2, 30)
            }
            None => 5,
        }
    }

    fn position_of(table: &Table, seq: u64) -> u32 {
        table
            .slots
            .values()
            .filter(|slot| matches!(slot, Slot::Active { seq: other, .. } if *other < seq))
            .count() as u32
    }

    /// Whether a submit would be refused for room alone. A cheap check made
    /// before the body is read; [`Self::admit`] decides.
    pub fn is_full(&self, background: bool) -> bool {
        let table = self.lock();
        table.active >= self.limits.max_active
            || table.done + table.active >= self.limits.max_stored
            || (background && table.active_background >= self.limits.max_active_background)
    }

    /// Accept a job, or refuse it as busy.
    pub fn admit(&self, secret_hash: Digest32, background: bool) -> Result<Admitted, AdmitRefusal> {
        let mut table = self.lock();
        if table.active >= self.limits.max_active
            || table.done + table.active >= self.limits.max_stored
            || (background && table.active_background >= self.limits.max_active_background)
        {
            return Err(AdmitRefusal::Full);
        }
        if self
            .queue
            .as_ref()
            .is_some_and(|queue| queue.would_exceed(table.active, self.deadline))
        {
            return Err(AdmitRefusal::WouldExpire);
        }
        let mut id = [0u8; trace_commons_protocol::witness_jobs::WITNESS_JOB_ID_LEN];
        rand::rngs::OsRng.fill_bytes(&mut id);
        let job_id = hex::encode(id);
        let id_hash = sha256(job_id.as_bytes());
        let seq = self.next_seq.fetch_add(1, Ordering::Relaxed);
        let queue_position = Self::position_of(&table, seq);
        table.slots.insert(
            id_hash,
            Slot::Active {
                seq,
                background,
                secret_hash,
                abort: None,
            },
        );
        table.active += 1;
        if background {
            table.active_background += 1;
        }
        drop(table);
        Ok(Admitted {
            job_id,
            id_hash,
            queue_position,
            poll_after_secs: self.poll_after(queue_position),
        })
    }

    /// Record the task running a job, so a cancel can abort it. A job that
    /// already finished or was cancelled is left alone.
    pub fn set_abort(&self, id_hash: &Digest32, handle: tokio::task::AbortHandle) {
        let mut table = self.lock();
        match table.slots.get_mut(id_hash) {
            Some(Slot::Active { abort, .. }) => *abort = Some(handle),
            _ => handle.abort(),
        }
    }

    /// Seal a finished answer and mark the job done. A job cancelled while
    /// it ran keeps nothing. A seal failure drops the result, so the job
    /// reads as gone and the client resubmits: nothing is kept unsealed.
    pub fn finish(&self, id_hash: &Digest32, result: JobResult) -> Result<(), StoreError> {
        let secret_hash = {
            let table = self.lock();
            match table.slots.get(id_hash) {
                Some(Slot::Active { secret_hash, .. }) => *secret_hash,
                _ => return Ok(()),
            }
        };
        let result = if result.body.len() > self.limits.max_result_bytes {
            oversized_result()
        } else {
            result
        };
        let expires_unix = (self.clock)().saturating_add(self.limits.result_ttl.as_secs());
        let sealed = self.sealer.seal(
            id_hash,
            &SealedRecord {
                secret_hash,
                expires_unix,
                result,
            },
        );
        let mut table = self.lock();
        let still_active = matches!(table.slots.get(id_hash), Some(Slot::Active { .. }));
        if !still_active {
            // Cancelled between the check above and the seal.
            drop(table);
            self.sealer.remove(id_hash);
            return Ok(());
        }
        table.remove(id_hash);
        match sealed {
            Ok(()) => {
                table.slots.insert(
                    *id_hash,
                    Slot::Done {
                        secret_hash,
                        expires_unix,
                    },
                );
                table.done += 1;
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    /// Answer a poll. A finished result is taken out of the table before it
    /// is read, so it is served at most once, and its file is deleted.
    pub fn poll(&self, id_hash: &Digest32, presented_secret_hash: &Digest32) -> PollOutcome {
        let mut table = self.lock();
        let (secret_hash, active_seq) = match table.slots.get(id_hash) {
            Some(Slot::Active {
                seq, secret_hash, ..
            }) => (*secret_hash, Some(*seq)),
            Some(Slot::Done { secret_hash, .. }) => (*secret_hash, None),
            None => return PollOutcome::Gone,
        };
        if !digests_equal(&secret_hash, presented_secret_hash) {
            return PollOutcome::Gone;
        }
        if let Some(seq) = active_seq {
            let queue_position = Self::position_of(&table, seq);
            drop(table);
            return PollOutcome::Pending {
                queue_position,
                poll_after_secs: self.poll_after(queue_position),
            };
        }
        table.remove(id_hash);
        drop(table);
        let record = self.sealer.read(id_hash);
        self.sealer.remove(id_hash);
        match record {
            Ok(record)
                if record.expires_unix > (self.clock)()
                    && digests_equal(&record.secret_hash, presented_secret_hash) =>
            {
                PollOutcome::Done(record.result)
            }
            Ok(_) => PollOutcome::Gone,
            Err(_) => {
                tracing::warn!("witness job result did not open; deleted");
                PollOutcome::Gone
            }
        }
    }

    /// Cancel a job: abort it if it is running, delete its result if it
    /// finished. `false` for every miss, as for a poll.
    pub fn cancel(&self, id_hash: &Digest32, presented_secret_hash: &Digest32) -> bool {
        let mut table = self.lock();
        let matches = match table.slots.get(id_hash) {
            Some(Slot::Active { secret_hash, .. }) | Some(Slot::Done { secret_hash, .. }) => {
                digests_equal(secret_hash, presented_secret_hash)
            }
            None => false,
        };
        if !matches {
            return false;
        }
        match table.remove(id_hash) {
            Some(Slot::Active {
                abort: Some(abort), ..
            }) => abort.abort(),
            Some(Slot::Active { abort: None, .. }) => {}
            Some(Slot::Done { .. }) => {
                drop(table);
                self.sealer.remove(id_hash);
            }
            None => {}
        }
        true
    }

    /// Delete every finished result past its expiry. Returns how many.
    pub fn sweep_expired(&self) -> usize {
        let now = (self.clock)();
        let mut table = self.lock();
        let expired: Vec<Digest32> = table
            .slots
            .iter()
            .filter_map(|(id, slot)| match slot {
                Slot::Done { expires_unix, .. } if *expires_unix <= now => Some(*id),
                _ => None,
            })
            .collect();
        for id in &expired {
            table.remove(id);
        }
        drop(table);
        for id in &expired {
            self.sealer.remove(id);
        }
        self.expired
            .fetch_add(expired.len() as u64, Ordering::Relaxed);
        expired.len()
    }

    /// Label-only counts.
    pub fn counts(&self) -> JobCounts {
        let table = self.lock();
        let waiting = self.queue.as_ref().map_or(0, |q| q.waiting());
        let pending = waiting.min(table.active);
        JobCounts {
            pending,
            running: table.active - pending,
            done_unfetched: table.done,
            expired: self.expired.load(Ordering::Relaxed),
        }
    }

    /// Run [`Self::sweep_expired`] every minute for as long as the table
    /// lives.
    pub fn spawn_sweeper(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let jobs = Arc::downgrade(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(60));
            loop {
                interval.tick().await;
                let Some(jobs) = jobs.upgrade() else {
                    return;
                };
                let swept = jobs.sweep_expired();
                if swept > 0 {
                    tracing::info!(expired = swept, "witness job results expired unfetched");
                }
            }
        })
    }
}

/// The answer kept in place of one too large to seal.
fn oversized_result() -> JobResult {
    JobResult {
        status: 503,
        headers: vec![("content-type".to_string(), b"application/json".to_vec())],
        body: format!(
            r#"{{"error":"{}"}}"#,
            trace_commons_protocol::witness_refusal::WITNESS_REDACTION_FAILED
        )
        .into_bytes(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU64;

    fn key(byte: u8) -> Zeroizing<[u8; 32]> {
        Zeroizing::new([byte; 32])
    }

    fn limits() -> JobLimits {
        JobLimits {
            max_active: 4,
            max_active_background: 3,
            max_stored: 8,
            max_result_bytes: 1024,
            result_ttl: Duration::from_secs(100),
        }
    }

    struct Fixture {
        dir: tempfile::TempDir,
        now: Arc<AtomicU64>,
    }

    impl Fixture {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
                now: Arc::new(AtomicU64::new(1_000)),
            }
        }

        fn clock(&self) -> Clock {
            let now = self.now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        }

        fn jobs_with(&self, key: Zeroizing<[u8; 32]>, limits: JobLimits) -> WitnessJobs {
            WitnessJobs::with_clock(
                ResultSealer::open(self.dir.path(), key).unwrap(),
                limits,
                Duration::from_secs(540),
                self.clock(),
            )
        }

        fn jobs(&self) -> WitnessJobs {
            self.jobs_with(key(7), limits())
        }

        fn files(&self) -> Vec<String> {
            let mut names: Vec<String> = std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                .collect();
            names.sort();
            names
        }

        fn raw_files(&self) -> Vec<Vec<u8>> {
            std::fs::read_dir(self.dir.path())
                .unwrap()
                .map(|e| std::fs::read(e.unwrap().path()).unwrap())
                .collect()
        }
    }

    const MARKER: &[u8] = b"redacted-envelope-marker-zq";

    fn result() -> JobResult {
        JobResult {
            status: 200,
            headers: vec![
                ("content-type".into(), b"application/json".to_vec()),
                ("x-trace-witness-signature".into(), b"0xabc".to_vec()),
            ],
            body: MARKER.to_vec(),
        }
    }

    fn secret() -> Digest32 {
        sha256(b"fetch-secret")
    }

    #[test]
    fn the_results_key_depends_on_the_measurement_and_the_secret() {
        let a = results_key(&[1; 32], "mrtd:aa+mrconfigid:bb");
        let b = results_key(&[1; 32], "mrtd:cc+mrconfigid:bb");
        let c = results_key(&[2; 32], "mrtd:aa+mrconfigid:bb");
        assert_ne!(*a, *b);
        assert_ne!(*a, *c);
        assert_eq!(*a, *results_key(&[1; 32], "mrtd:aa+mrconfigid:bb"));
    }

    #[test]
    fn digests_compare_by_value() {
        assert!(digests_equal(&[3; 32], &[3; 32]));
        let mut other = [3; 32];
        other[31] = 4;
        assert!(!digests_equal(&[3; 32], &other));
    }

    #[test]
    fn a_finished_result_is_sealed_served_once_then_deleted() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        assert_eq!(admitted.job_id.len(), 32);
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Pending { .. }
        ));
        jobs.finish(&admitted.id_hash, result()).unwrap();

        // On disk, sealed: neither the body nor the job id appears.
        let files = fixture.raw_files();
        assert_eq!(files.len(), 1);
        assert!(!files[0].windows(MARKER.len()).any(|w| w == MARKER));
        assert!(
            fixture
                .files()
                .iter()
                .all(|name| !name.contains(&admitted.job_id))
        );

        match jobs.poll(&admitted.id_hash, &secret()) {
            PollOutcome::Done(served) => assert_eq!(served, result()),
            other => panic!("expected the result, got {other:?}"),
        }
        assert!(fixture.files().is_empty(), "the result outlived its fetch");
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Gone
        ));
    }

    #[test]
    fn a_wrong_secret_is_gone_and_does_not_consume_the_result() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        let wrong = sha256(b"another secret");
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &wrong),
            PollOutcome::Gone
        ));
        jobs.finish(&admitted.id_hash, result()).unwrap();
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &wrong),
            PollOutcome::Gone
        ));
        assert!(!jobs.cancel(&admitted.id_hash, &wrong));
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Done(_)
        ));
    }

    #[test]
    fn results_survive_a_restart_under_the_same_key() {
        let fixture = Fixture::new();
        let admitted = {
            let jobs = fixture.jobs();
            let admitted = jobs.admit(secret(), false).unwrap();
            jobs.finish(&admitted.id_hash, result()).unwrap();
            admitted
        };
        let restarted = fixture.jobs();
        assert_eq!(restarted.counts().done_unfetched, 1);
        assert!(matches!(
            restarted.poll(&admitted.id_hash, &secret()),
            PollOutcome::Done(_)
        ));
    }

    #[test]
    fn another_measurement_cannot_open_old_results_and_deletes_them() {
        let fixture = Fixture::new();
        let admitted = {
            let jobs = fixture.jobs_with(results_key(&[9; 32], "mrtd:old"), limits());
            let admitted = jobs.admit(secret(), false).unwrap();
            jobs.finish(&admitted.id_hash, result()).unwrap();
            admitted
        };
        let redeployed = fixture.jobs_with(results_key(&[9; 32], "mrtd:new"), limits());
        assert_eq!(redeployed.counts().done_unfetched, 0);
        assert!(fixture.files().is_empty(), "an unreadable result was kept");
        assert!(matches!(
            redeployed.poll(&admitted.id_hash, &secret()),
            PollOutcome::Gone
        ));
    }

    #[test]
    fn a_tampered_file_is_gone_and_deleted() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        jobs.finish(&admitted.id_hash, result()).unwrap();
        let path = std::fs::read_dir(fixture.dir.path())
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, bytes).unwrap();
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Gone
        ));
        assert!(fixture.files().is_empty());
    }

    #[test]
    fn a_file_renamed_onto_another_id_does_not_open() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let first = jobs.admit(secret(), false).unwrap();
        let second = jobs.admit(secret(), false).unwrap();
        jobs.finish(&first.id_hash, result()).unwrap();
        jobs.finish(&second.id_hash, result()).unwrap();
        let first_path = jobs.sealer.path(&first.id_hash);
        let second_path = jobs.sealer.path(&second.id_hash);
        std::fs::rename(&first_path, &second_path).unwrap();
        assert!(matches!(
            jobs.poll(&second.id_hash, &secret()),
            PollOutcome::Gone
        ));
    }

    #[test]
    fn results_expire_after_their_lifetime_and_are_counted() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        jobs.finish(&admitted.id_hash, result()).unwrap();
        fixture.now.fetch_add(99, Ordering::SeqCst);
        assert_eq!(jobs.sweep_expired(), 0);
        fixture.now.fetch_add(1, Ordering::SeqCst);
        assert_eq!(jobs.sweep_expired(), 1);
        assert_eq!(jobs.counts().expired, 1);
        assert!(fixture.files().is_empty());
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Gone
        ));
    }

    #[test]
    fn expired_results_are_not_recovered_at_startup() {
        let fixture = Fixture::new();
        {
            let jobs = fixture.jobs();
            let admitted = jobs.admit(secret(), false).unwrap();
            jobs.finish(&admitted.id_hash, result()).unwrap();
        }
        fixture.now.fetch_add(100, Ordering::SeqCst);
        let restarted = fixture.jobs();
        assert_eq!(restarted.counts().done_unfetched, 0);
        assert!(fixture.files().is_empty());
    }

    #[test]
    fn the_table_refuses_past_its_bounds() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        for _ in 0..3 {
            jobs.admit(secret(), true).unwrap();
        }
        assert_eq!(
            jobs.admit(secret(), true).err(),
            Some(AdmitRefusal::Full),
            "the background share is three"
        );
        jobs.admit(secret(), false).unwrap();
        assert_eq!(jobs.admit(secret(), false).err(), Some(AdmitRefusal::Full));
    }

    #[test]
    fn the_store_bound_counts_unfetched_results() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs_with(
            key(7),
            JobLimits {
                max_stored: 2,
                ..limits()
            },
        );
        let a = jobs.admit(secret(), false).unwrap();
        jobs.finish(&a.id_hash, result()).unwrap();
        let b = jobs.admit(secret(), false).unwrap();
        jobs.finish(&b.id_hash, result()).unwrap();
        assert_eq!(jobs.admit(secret(), false).err(), Some(AdmitRefusal::Full));
        assert!(matches!(
            jobs.poll(&a.id_hash, &secret()),
            PollOutcome::Done(_)
        ));
        assert!(jobs.admit(secret(), false).is_ok());
    }

    #[test]
    fn a_job_the_queue_would_make_expire_is_refused() {
        let fixture = Fixture::new();
        let queue = Arc::new(ClassifyQueue::new());
        queue.record_turn(Duration::from_secs(300));
        let jobs = fixture.jobs().with_classify_queue(queue);
        jobs.admit(secret(), false).unwrap();
        assert_eq!(
            jobs.admit(secret(), false).err(),
            Some(AdmitRefusal::WouldExpire)
        );
    }

    #[test]
    fn queue_position_counts_earlier_active_jobs() {
        let fixture = Fixture::new();
        let queue = Arc::new(ClassifyQueue::new());
        queue.record_turn(Duration::from_secs(10));
        let jobs = fixture.jobs().with_classify_queue(queue);
        let first = jobs.admit(secret(), false).unwrap();
        let second = jobs.admit(secret(), false).unwrap();
        assert_eq!(first.queue_position, 0);
        assert_eq!(second.queue_position, 1);
        assert_eq!(first.poll_after_secs, 5);
        assert_eq!(second.poll_after_secs, 10);
        jobs.finish(&first.id_hash, result()).unwrap();
        match jobs.poll(&second.id_hash, &secret()) {
            PollOutcome::Pending { queue_position, .. } => assert_eq!(queue_position, 0),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn an_oversized_result_is_kept_as_a_refusal_not_sealed_whole() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        let mut big = result();
        big.body = vec![b'x'; 2048];
        jobs.finish(&admitted.id_hash, big).unwrap();
        match jobs.poll(&admitted.id_hash, &secret()) {
            PollOutcome::Done(served) => {
                assert_eq!(served.status, 503);
                assert_eq!(served.body, br#"{"error":"witness_redaction_failed"}"#);
            }
            other => panic!("{other:?}"),
        }
    }

    #[tokio::test]
    async fn cancelling_an_active_job_aborts_it_and_keeps_nothing() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        let task = tokio::spawn(std::future::pending::<()>());
        jobs.set_abort(&admitted.id_hash, task.abort_handle());
        assert!(jobs.cancel(&admitted.id_hash, &secret()));
        assert!(task.await.unwrap_err().is_cancelled());
        // A finish that races the cancel stores nothing.
        jobs.finish(&admitted.id_hash, result()).unwrap();
        assert!(fixture.files().is_empty());
        assert_eq!(jobs.counts(), JobCounts::default());
    }

    #[test]
    fn a_seal_failure_drops_the_result_and_reads_as_gone() {
        let fixture = Fixture::new();
        let jobs = fixture.jobs();
        let admitted = jobs.admit(secret(), false).unwrap();
        std::fs::remove_dir_all(fixture.dir.path()).unwrap();
        assert_eq!(
            jobs.finish(&admitted.id_hash, result()),
            Err(StoreError::WriteFailed)
        );
        assert!(matches!(
            jobs.poll(&admitted.id_hash, &secret()),
            PollOutcome::Gone
        ));
        assert_eq!(jobs.counts(), JobCounts::default());
    }

    #[test]
    fn an_unwritable_directory_refuses_to_open() {
        let file = tempfile::NamedTempFile::new().unwrap();
        assert_eq!(
            ResultSealer::open(file.path().join("results"), key(1)).err(),
            Some(StoreError::DirectoryUnusable)
        );
    }

    #[test]
    fn a_debug_render_withholds_the_body() {
        let rendered = format!("{:?}", result());
        assert!(!rendered.contains("redacted-envelope-marker"));
    }
}
