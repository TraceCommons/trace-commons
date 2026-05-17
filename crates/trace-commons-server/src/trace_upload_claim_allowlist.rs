//! Pilot allowlist for the upload-claim issuer.
//!
//! The issuer mints upload claims for callers who can prove they hold an
//! operator-issued invite code. The allowlist file stores the SHA-256 hash
//! of each accepted invite code; raw codes never reach disk and never appear
//! in any log line, audit row, or admin response.
//!
//! Sources behind a trait so the open-pilot growth path (NEAR view-call
//! against an on-chain allowlist contract) lands without touching the
//! issuance handler. Only [`FileAllowlistSource`] is implemented today —
//! the NEAR variant is reserved in the spec/CLI surface so it can drop in
//! without breaking deployments.
//!
//! Hash-only convention: [`hash_invite_code`] is the single source of truth
//! that both the operator (via the `--hash-invite-code` CLI helper) and the
//! issuance handler call. They cannot drift because there is one function.

use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// Canonical invite-code hashing. The `"invite:"` prefix namespaces the
/// digest so a later subject type (NEAR account, EdDSA pubkey) cannot
/// accidentally hash to the same value as an invite code.
///
/// Used at two places only: the operator's `--hash-invite-code` helper and
/// the issuance handler's allowlist check. Keep these the only callers —
/// drift between them is the most likely way to break the pilot.
pub fn hash_invite_code(code: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"invite:");
    hasher.update(code.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}

/// Source dispatch parsed off `--allowlist-source` / env var. Only the
/// `File` variant is implemented; `Near` parses but is rejected at
/// construction so the CLI surface is stable for the later on-chain
/// allowlist slice.
#[derive(Debug, Clone)]
pub enum AllowlistSourceSpec {
    File(PathBuf),
    Near { account: String, view_method: String },
}

impl AllowlistSourceSpec {
    /// Parse a `file:<path>` or `near:<account>:<view_method>` spec. Returns
    /// `None` for `None` input. Anything else fails with a typed `anyhow`
    /// error.
    pub fn parse(raw: Option<&str>) -> anyhow::Result<Option<Self>> {
        let Some(raw) = raw else { return Ok(None) };
        let raw = raw.trim();
        if raw.is_empty() {
            return Ok(None);
        }
        if let Some(path) = raw.strip_prefix("file:") {
            let path = path.trim();
            if path.is_empty() {
                anyhow::bail!("PilotAllowlistSourceMalformed: file: source requires a path");
            }
            return Ok(Some(AllowlistSourceSpec::File(PathBuf::from(path))));
        }
        if let Some(rest) = raw.strip_prefix("near:") {
            let (account, view_method) = rest.split_once(':').ok_or_else(|| {
                anyhow::anyhow!(
                    "PilotAllowlistSourceMalformed: near: source requires near:<account>:<view_method>"
                )
            })?;
            // Reserved variant — parse but refuse to construct so the CLI
            // surface stays stable when the on-chain source ships in a
            // later slice.
            return Err(anyhow::anyhow!(
                "PilotAllowlistNearSourceNotImplemented: use file:<path> until the on-chain allowlist source lands (account={account}, view_method={view_method})"
            ));
        }
        anyhow::bail!(
            "PilotAllowlistSourceMalformed: source must be 'file:<path>' (got {raw:?})"
        )
    }
}

/// Single-entry shape inside an allowlist file.
#[derive(Debug, Deserialize)]
pub struct AllowlistEntry {
    pub subject_hash: String,
    /// Tenant the contributor will be attributed to once their claim is
    /// minted. The issuance handler does not currently force tenant equality
    /// against this field — the existing workload-claim flow already
    /// resolves the minted tenant. Stored for future cross-checks and for
    /// operator-side auditing of "who is allowed where".
    pub tenant_id: String,
    /// Operator-facing free text. Never returned to clients, never logged.
    #[serde(default)]
    pub note_label: Option<String>,
}

/// Top-level file schema. Version 1 only; anything else is rejected.
#[derive(Debug, Deserialize)]
pub struct AllowlistFile {
    pub version: u32,
    pub generated_at: DateTime<Utc>,
    pub policy_label: String,
    pub entries: Vec<AllowlistEntry>,
}

/// Snapshot loaded from a source. Cheap to clone-by-reference; the
/// `subject_hashes` set is the hot path.
#[derive(Debug, Clone)]
pub struct AllowlistSnapshot {
    pub policy_label: String,
    pub generated_at: DateTime<Utc>,
    pub subject_hashes: HashSet<String>,
    pub loaded_at: Instant,
    pub source_label: String,
}

impl AllowlistSnapshot {
    /// Build a snapshot from a parsed file. Validates each entry's
    /// `subject_hash` is canonical `sha256:<64 lower-hex>`; rejects the
    /// whole snapshot if any entry is malformed. Dedupes hashes silently
    /// (operator typo recovery).
    pub fn from_file(
        file: AllowlistFile,
        source_label: String,
        loaded_at: Instant,
    ) -> Result<Self, AllowlistError> {
        if file.version != 1 {
            return Err(AllowlistError::Malformed(format!(
                "unsupported version {} (expected 1)",
                file.version
            )));
        }
        let mut subject_hashes = HashSet::with_capacity(file.entries.len());
        for entry in &file.entries {
            // Strict: the file's hashes must already be canonical
            // sha256:<64 lower-hex>. We don't lowercase on read because
            // silently fixing uppercase would mask the operator generating
            // hashes with the wrong tool. Trim leading/trailing whitespace
            // only.
            let trimmed = entry.subject_hash.trim();
            validate_subject_hash(trimmed)?;
            subject_hashes.insert(trimmed.to_string());
        }
        Ok(Self {
            policy_label: file.policy_label,
            generated_at: file.generated_at,
            subject_hashes,
            loaded_at,
            source_label,
        })
    }

    pub fn contains(&self, subject_hash: &str) -> bool {
        self.subject_hashes.contains(subject_hash)
    }
}

fn validate_subject_hash(s: &str) -> Result<(), AllowlistError> {
    let Some(hex) = s.strip_prefix("sha256:") else {
        return Err(AllowlistError::Malformed(format!(
            "subject_hash must start with sha256: (got {s:?})"
        )));
    };
    if hex.len() != 64 || !hex.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()) {
        return Err(AllowlistError::Malformed(format!(
            "subject_hash must be sha256:<64 lower-hex> (got {s:?})"
        )));
    }
    Ok(())
}

/// Internal allowlist error vocabulary. The issuer maps these to its own
/// `IssuerError` constructors so the public refusal labels live with the
/// rest of the issuer's error surface.
#[derive(Debug)]
pub enum AllowlistError {
    SourceMissing(String),
    Malformed(String),
    /// First-load failure with no cached snapshot to fall back to.
    NoSnapshotYet,
}

impl std::fmt::Display for AllowlistError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AllowlistError::SourceMissing(s) => write!(f, "source missing: {s}"),
            AllowlistError::Malformed(s) => write!(f, "malformed: {s}"),
            AllowlistError::NoSnapshotYet => write!(f, "no snapshot loaded yet"),
        }
    }
}

impl std::error::Error for AllowlistError {}

/// Trait so the issuance handler can hold an opaque source. One file impl
/// today; the `Near { ... }` variant of `AllowlistSourceSpec` lands behind
/// this trait in a later slice without touching the call site.
pub trait AllowlistSource: Send + Sync {
    /// Return the current snapshot. Implementations cache and only refetch
    /// once per refresh interval. Returns the cached snapshot on transient
    /// fetch error; returns `Err(NoSnapshotYet)` only if no snapshot has
    /// ever loaded successfully.
    fn snapshot(&self) -> Result<AllowlistSnapshot, AllowlistError>;
}

/// File-backed source. Re-reads the JSON file at most once per
/// `refresh_interval`. On parse / IO failure mid-run, keeps serving the
/// cached snapshot and logs hash-only warnings.
pub struct FileAllowlistSource {
    path: PathBuf,
    refresh_interval: Duration,
    cached: Mutex<Option<AllowlistSnapshot>>,
    source_label: String,
}

impl FileAllowlistSource {
    pub fn new(path: PathBuf, refresh_interval: Duration) -> Self {
        let source_label = format!("file:{}", path.display());
        Self {
            path,
            refresh_interval,
            cached: Mutex::new(None),
            source_label,
        }
    }

    /// Force-load on construction so the issuer fails fast if the operator
    /// pointed at a missing or malformed file. Called by the issuer's
    /// `build_state` so a bad source aborts startup instead of waiting for
    /// the first request.
    pub fn warm(&self) -> Result<(), AllowlistError> {
        let snapshot = load_file(&self.path, &self.source_label, Instant::now())?;
        *self.cached.lock().expect("FileAllowlistSource mutex poisoned") = Some(snapshot);
        Ok(())
    }
}

impl AllowlistSource for FileAllowlistSource {
    fn snapshot(&self) -> Result<AllowlistSnapshot, AllowlistError> {
        let mut cached = self
            .cached
            .lock()
            .expect("FileAllowlistSource mutex poisoned");
        let needs_refresh = match cached.as_ref() {
            None => true,
            Some(snap) => snap.loaded_at.elapsed() >= self.refresh_interval,
        };
        if needs_refresh {
            match load_file(&self.path, &self.source_label, Instant::now()) {
                Ok(snapshot) => {
                    *cached = Some(snapshot);
                }
                Err(err) => {
                    // Cached snapshot survives transient errors; the
                    // max-stale check in the handler is what fails closed
                    // once the snapshot's `loaded_at` is too old.
                    let error_class = match &err {
                        AllowlistError::SourceMissing(_) => "PilotAllowlistSourceMissing",
                        AllowlistError::Malformed(_) => "PilotAllowlistMalformed",
                        AllowlistError::NoSnapshotYet => "PilotAllowlistNoSnapshotYet",
                    };
                    tracing::warn!(
                        error_class,
                        source_label = %self.source_label,
                        "allowlist reload failed; serving cached snapshot"
                    );
                    if cached.is_none() {
                        return Err(err);
                    }
                }
            }
        }
        cached
            .as_ref()
            .cloned()
            .ok_or(AllowlistError::NoSnapshotYet)
    }
}

fn load_file(
    path: &Path,
    source_label: &str,
    loaded_at: Instant,
) -> Result<AllowlistSnapshot, AllowlistError> {
    let bytes = std::fs::read(path).map_err(|e| {
        AllowlistError::SourceMissing(format!("read {}: {e}", path.display()))
    })?;
    let file: AllowlistFile = serde_json::from_slice(&bytes)
        .map_err(|e| AllowlistError::Malformed(format!("parse {}: {e}", path.display())))?;
    AllowlistSnapshot::from_file(file, source_label.to_string(), loaded_at)
}

/// Sliding-window denial counter for `/v1/admin/allowlist-status`. Process-
/// local, restart-resets. Bounded memory: trims to the active window on
/// every operation.
pub struct DenialCounter {
    window: Duration,
    samples: Mutex<VecDeque<Instant>>,
}

impl DenialCounter {
    pub fn new(window: Duration) -> Self {
        Self {
            window,
            samples: Mutex::new(VecDeque::new()),
        }
    }

    pub fn record(&self) {
        let mut samples = self.samples.lock().expect("DenialCounter mutex poisoned");
        let now = Instant::now();
        samples.push_back(now);
        evict_expired(&mut samples, now, self.window);
    }

    pub fn count_in_window(&self) -> usize {
        let mut samples = self.samples.lock().expect("DenialCounter mutex poisoned");
        let now = Instant::now();
        evict_expired(&mut samples, now, self.window);
        samples.len()
    }

    pub fn window_seconds(&self) -> u64 {
        self.window.as_secs()
    }
}

fn evict_expired(samples: &mut VecDeque<Instant>, now: Instant, window: Duration) {
    while let Some(front) = samples.front() {
        if now.duration_since(*front) > window {
            samples.pop_front();
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn hash_invite_code_is_stable_and_namespaced() {
        let a = hash_invite_code("INV-PILOT-001");
        let b = hash_invite_code("INV-PILOT-001");
        assert_eq!(a, b);
        assert!(a.starts_with("sha256:"));
        assert_eq!(a.len(), "sha256:".len() + 64);
        // Different inputs → different digests.
        assert_ne!(a, hash_invite_code("INV-PILOT-002"));
        // Prefix sharing does not collide.
        assert_ne!(hash_invite_code("INV-1"), hash_invite_code("INV-12"));
        // The "invite:" prefix means raw hex of just the code's sha256 is
        // never accepted — a hash of the code without the prefix produces
        // a different digest.
        let mut bare = Sha256::new();
        bare.update("INV-PILOT-001".as_bytes());
        let bare_hex = format!("sha256:{}", hex::encode(bare.finalize()));
        assert_ne!(a, bare_hex);
    }

    #[test]
    fn source_spec_parser_accepts_file_rejects_others() {
        assert!(matches!(
            AllowlistSourceSpec::parse(Some("file:/etc/tracecommons/allowlist.json")).unwrap(),
            Some(AllowlistSourceSpec::File(_))
        ));
        assert!(AllowlistSourceSpec::parse(Some("file:")).is_err());
        assert!(AllowlistSourceSpec::parse(Some("garbage")).is_err());
        // Near parses syntactically but is rejected at construction so the
        // CLI surface is stable for the later on-chain slice.
        let err = AllowlistSourceSpec::parse(Some("near:allowlist.tracecommons.near:is_allowed"))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("PilotAllowlistNearSourceNotImplemented"),
            "expected NearSourceNotImplemented error label, got {err}"
        );
        assert!(AllowlistSourceSpec::parse(None).unwrap().is_none());
        assert!(AllowlistSourceSpec::parse(Some("   ")).unwrap().is_none());
    }

    #[test]
    fn snapshot_rejects_wrong_version_and_malformed_hashes() {
        let bad_version = AllowlistFile {
            version: 2,
            generated_at: Utc::now(),
            policy_label: "pilot".into(),
            entries: vec![],
        };
        assert!(matches!(
            AllowlistSnapshot::from_file(bad_version, "test".into(), Instant::now()),
            Err(AllowlistError::Malformed(_))
        ));

        let bad_hash = AllowlistFile {
            version: 1,
            generated_at: Utc::now(),
            policy_label: "pilot".into(),
            entries: vec![AllowlistEntry {
                subject_hash: "not-a-sha256".into(),
                tenant_id: "t".into(),
                note_label: None,
            }],
        };
        assert!(matches!(
            AllowlistSnapshot::from_file(bad_hash, "test".into(), Instant::now()),
            Err(AllowlistError::Malformed(_))
        ));

        let uppercase_hex = AllowlistFile {
            version: 1,
            generated_at: Utc::now(),
            policy_label: "pilot".into(),
            entries: vec![AllowlistEntry {
                subject_hash: format!("sha256:{}", "A".repeat(64)),
                tenant_id: "t".into(),
                note_label: None,
            }],
        };
        assert!(matches!(
            AllowlistSnapshot::from_file(uppercase_hex, "test".into(), Instant::now()),
            Err(AllowlistError::Malformed(_))
        ));
    }

    #[test]
    fn snapshot_dedupes_duplicate_hashes() {
        let h = hash_invite_code("X");
        let file = AllowlistFile {
            version: 1,
            generated_at: Utc::now(),
            policy_label: "pilot".into(),
            entries: vec![
                AllowlistEntry {
                    subject_hash: h.clone(),
                    tenant_id: "t".into(),
                    note_label: None,
                },
                AllowlistEntry {
                    subject_hash: h.clone(),
                    tenant_id: "t2".into(),
                    note_label: Some("dup".into()),
                },
            ],
        };
        let snap =
            AllowlistSnapshot::from_file(file, "test".into(), Instant::now()).expect("ok");
        assert_eq!(snap.subject_hashes.len(), 1);
        assert!(snap.contains(&h));
    }

    #[test]
    fn file_source_caches_and_reloads() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        let h = hash_invite_code("INV-1");
        write_file(
            &path,
            &format!(
                r#"{{
                    "version": 1,
                    "generated_at": "2026-05-17T00:00:00Z",
                    "policy_label": "p1",
                    "entries": [{{"subject_hash": "{h}", "tenant_id": "t"}}]
                }}"#
            ),
        );
        let source = FileAllowlistSource::new(path.clone(), Duration::from_millis(50));
        source.warm().expect("first warm load succeeds");

        let snap1 = source.snapshot().expect("snapshot");
        assert_eq!(snap1.policy_label, "p1");
        assert!(snap1.contains(&h));

        // Inside the refresh interval: filesystem changes are not picked up.
        let h2 = hash_invite_code("INV-2");
        write_file(
            &path,
            &format!(
                r#"{{
                    "version": 1,
                    "generated_at": "2026-05-17T00:00:01Z",
                    "policy_label": "p2",
                    "entries": [{{"subject_hash": "{h2}", "tenant_id": "t"}}]
                }}"#
            ),
        );
        let snap_cached = source.snapshot().expect("snapshot");
        assert_eq!(snap_cached.policy_label, "p1");

        // After the interval: reload picks up the new content.
        std::thread::sleep(Duration::from_millis(80));
        let snap_fresh = source.snapshot().expect("snapshot");
        assert_eq!(snap_fresh.policy_label, "p2");
        assert!(snap_fresh.contains(&h2));
        assert!(!snap_fresh.contains(&h));
    }

    #[test]
    fn file_source_serves_cached_when_file_deleted_or_corrupt() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("allowlist.json");
        let h = hash_invite_code("INV-1");
        write_file(
            &path,
            &format!(
                r#"{{
                    "version": 1,
                    "generated_at": "2026-05-17T00:00:00Z",
                    "policy_label": "p1",
                    "entries": [{{"subject_hash": "{h}", "tenant_id": "t"}}]
                }}"#
            ),
        );
        let source = FileAllowlistSource::new(path.clone(), Duration::from_millis(50));
        source.warm().expect("warm load");
        std::thread::sleep(Duration::from_millis(80));

        std::fs::remove_file(&path).expect("rm");
        let after_delete = source.snapshot().expect("cached survives delete");
        assert_eq!(after_delete.policy_label, "p1");
        assert!(after_delete.contains(&h));

        // Write a corrupt file; cache still wins.
        std::thread::sleep(Duration::from_millis(80));
        write_file(&path, "this is not json");
        let after_corrupt = source.snapshot().expect("cached survives corrupt");
        assert_eq!(after_corrupt.policy_label, "p1");
    }

    #[test]
    fn first_load_failure_returns_no_snapshot_yet() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("does-not-exist.json");
        let source = FileAllowlistSource::new(missing, Duration::from_millis(50));
        // No warm() call; snapshot() must fail with NoSnapshotYet (or
        // SourceMissing, which is also acceptable as long as it's surfaced).
        let err = source.snapshot().unwrap_err();
        let label = err.to_string();
        assert!(label.contains("source missing") || label.contains("no snapshot loaded yet"));
    }

    #[test]
    fn denial_counter_records_and_evicts() {
        let counter = DenialCounter::new(Duration::from_millis(100));
        counter.record();
        counter.record();
        counter.record();
        assert_eq!(counter.count_in_window(), 3);

        std::thread::sleep(Duration::from_millis(150));
        assert_eq!(counter.count_in_window(), 0);

        counter.record();
        assert_eq!(counter.count_in_window(), 1);
        assert_eq!(counter.window_seconds(), 0); // 100ms rounds down
    }

    fn write_file(path: &Path, content: &str) {
        let mut f = std::fs::File::create(path).expect("create");
        f.write_all(content.as_bytes()).expect("write");
    }
}
