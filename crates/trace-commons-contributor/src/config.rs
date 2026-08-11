//! Local contributor state: config file, device keystore path, receipts log.
//!
//! All files live under a single per-user directory (`ConfigStore::dir`).
//! Writes are atomic (temp file + rename) and permission-restricted on unix
//! (dir 0700, files 0600). Receipts are hash-only: never paths or content.

use std::fs::OpenOptions;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use uuid::Uuid;

pub const CONTRIBUTOR_CONFIG_SCHEMA_VERSION: &str = "trace_commons.contributor_config.v1";

const CONFIG_FILE: &str = "contributor.json";
const DEVICE_KEY_FILE: &str = "device.pk8";
const RECEIPTS_FILE: &str = "receipts.jsonl";
const NEAR_AI_NOTICE_MARKER_FILE: &str = "near-ai-notice-shown";

/// Background-daemon state files. All live in the same 0700 directory as the
/// device key and are removed by `wipe()`, so a logout cannot leave one
/// contributor's auto-upload opt-ins in place for whoever enrolls next.
pub const DAEMON_SETTINGS_FILE: &str = "daemon-settings.json";
pub const DAEMON_STATE_FILE: &str = "daemon-state.json";
pub const DAEMON_PROJECTS_FILE: &str = "daemon-projects.json";
pub const DAEMON_QUEUE_FILE: &str = "daemon-queue.jsonl";
pub const DAEMON_HISTORY_FILE: &str = "daemon-history.jsonl";
/// Local, label-only record of consequential autonomy changes (arming
/// auto-upload, bulk-approving). This is user-facing visibility, not a
/// security control -- see `daemon::audit`.
pub const DAEMON_AUDIT_FILE: &str = "daemon-audit.jsonl";
/// The account session token from the loopback browser sign-in
/// (`crate::account_auth`). A SECRET, written at 0600 inside the 0700 state
/// directory exactly like the device key, and swept by `wipe()` -- a logout
/// that left an account token behind would hand the next person to enroll on
/// this machine the ability to read and withdraw the previous contributor's
/// traces.
pub const ACCOUNT_SESSION_FILE: &str = "account-session.json";
/// Name prefix of the per-entry redacted envelope files
/// (`daemon::approved_envelope`). One file per previewed-and-approved queue
/// entry, so they cannot be listed by name; `wipe()` sweeps them by prefix.
pub const DAEMON_APPROVED_ENVELOPE_PREFIX: &str = "daemon-approved-envelope-";
/// Runtime files, not persistent state: removed on shutdown, not by `wipe()`.
pub const DAEMON_SOCK_FILE: &str = "daemon.sock";
pub const DAEMON_LOCK_FILE: &str = "daemon.lock";

/// Per-user contributor CLI configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContributorConfig {
    pub schema_version: String,
    pub issuer_url: String,
    pub ingest_url: String,
    pub audience: String,
    pub tenant_id: String,
    pub instance_id: String,
    pub user_subject: String,
    pub device_key_id: String,
    pub consent_scopes: Vec<String>,
    pub pii_filter: Option<String>,
    pub allowed_hosts: Option<String>,
}

/// Build the allowlist to enforce for issuer/ingest requests: the `allowed_hosts`
/// CSV when set (config's `allowed_hosts` field, or a pre-enrollment CLI
/// flag), otherwise the `TRACE_COMMONS_ALLOWED_HOSTS` env fallback. Shared by
/// every call site that builds an operator-client (`login`, issuer minting,
/// and ingest uploads/status) so none of them can drift into skipping env
/// enforcement.
pub fn allowlist_for(allowed_hosts: Option<&str>) -> HostAllowlist {
    match allowed_hosts {
        Some(csv) => HostAllowlist::from_csv(csv),
        None => HostAllowlist::from_env(),
    }
}

/// A hash-only record of a submitted trace. Never contains paths or content.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Receipt {
    pub submission_id: Uuid,
    pub session_hash: String,
    pub source: String,
    pub submitted_at: DateTime<Utc>,
    pub status: String,
}

/// Filesystem-backed store for contributor config, device key, and receipts.
pub struct ConfigStore {
    dir: PathBuf,
}

impl ConfigStore {
    /// Resolve the contributor state directory using precedence:
    /// 1. `explicit` flag
    /// 2. `TRACE_COMMONS_CONTRIBUTOR_DIR` env var
    /// 3. `dirs::config_dir()/trace-commons`
    ///
    /// Creates the directory (mode 0700 on unix) if it does not exist.
    pub fn resolve(explicit: Option<PathBuf>) -> Result<Self> {
        let dir = if let Some(dir) = explicit {
            dir
        } else if let Ok(dir) = std::env::var("TRACE_COMMONS_CONTRIBUTOR_DIR") {
            PathBuf::from(dir)
        } else {
            dirs::config_dir()
                .context("could not determine a config directory for this platform")?
                .join("trace-commons")
        };
        Self::open(dir)
    }

    /// Open (creating if necessary) the given directory as a contributor
    /// state store. Does not consult the environment.
    pub fn open(dir: PathBuf) -> Result<Self> {
        if !dir.exists() {
            std::fs::create_dir_all(&dir)
                .with_context(|| format!("creating contributor state dir {}", dir.display()))?;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
                .with_context(|| format!("setting permissions on {}", dir.display()))?;
        }
        Ok(Self { dir })
    }

    /// The directory backing this store.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn config_path(&self) -> PathBuf {
        self.dir.join(CONFIG_FILE)
    }

    pub fn load_config(&self) -> Result<Option<ContributorConfig>> {
        let path = self.config_path();
        if !path.exists() {
            return Ok(None);
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cfg =
            serde_json::from_str(&raw).with_context(|| format!("parsing {}", path.display()))?;
        Ok(Some(cfg))
    }

    pub fn save_config(&self, cfg: &ContributorConfig) -> Result<()> {
        let path = self.config_path();
        let body = serde_json::to_vec_pretty(cfg).context("serializing contributor config")?;
        write_atomic_0600(&self.dir, &path, &body)
    }

    /// Path to the device key file. Does not imply the file exists.
    pub fn device_key_path(&self) -> PathBuf {
        self.dir.join(DEVICE_KEY_FILE)
    }

    pub fn save_device_key(&self, pkcs8_der: &[u8]) -> Result<()> {
        let path = self.device_key_path();
        write_atomic_0600(&self.dir, &path, pkcs8_der)
    }

    pub fn load_device_key(&self) -> Result<Option<Vec<u8>>> {
        let path = self.device_key_path();
        if !path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Ok(Some(bytes))
    }

    fn receipts_path(&self) -> PathBuf {
        self.dir.join(RECEIPTS_FILE)
    }

    pub fn append_receipt(&self, r: &Receipt) -> Result<()> {
        let path = self.receipts_path();
        let existed = path.exists();
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        if !existed {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.set_permissions(std::fs::Permissions::from_mode(0o600))
                    .with_context(|| format!("setting permissions on {}", path.display()))?;
            }
        }
        let mut line = serde_json::to_string(r).context("serializing receipt")?;
        line.push('\n');
        file.write_all(line.as_bytes())
            .with_context(|| format!("writing {}", path.display()))?;
        Ok(())
    }

    pub fn load_receipts(&self) -> Result<Vec<Receipt>> {
        let path = self.receipts_path();
        if !path.exists() {
            return Ok(Vec::new());
        }
        let raw = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let mut receipts = Vec::new();
        let mut skipped = 0usize;
        for line in raw.lines() {
            if line.trim().is_empty() {
                continue;
            }
            match serde_json::from_str::<Receipt>(line) {
                Ok(r) => receipts.push(r),
                Err(_) => skipped += 1,
            }
        }
        if skipped > 0 {
            tracing::warn!(skipped, "skipped unparseable receipt lines");
        }
        Ok(receipts)
    }

    /// Path to a daemon state file inside this store. Daemon state lives in
    /// the same 0700 directory as the device key, so the directory
    /// permissions are the single enforcing control for all of it.
    pub fn daemon_path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    /// Atomically write a daemon state file at 0600.
    pub fn write_daemon_file(&self, name: &str, body: &[u8]) -> Result<()> {
        let path = self.daemon_path(name);
        write_atomic_0600(&self.dir, &path, body)
    }

    /// Read a daemon state file, or `None` when it does not exist yet.
    pub fn read_daemon_file(&self, name: &str) -> Result<Option<Vec<u8>>> {
        let path = self.daemon_path(name);
        match std::fs::read(&path) {
            Ok(body) => Ok(Some(body)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e).with_context(|| format!("reading {}", path.display())),
        }
    }

    /// Remove a daemon runtime file (socket, lock). Missing is not an error.
    pub fn remove_daemon_file(&self, name: &str) -> Result<()> {
        let path = self.daemon_path(name);
        match std::fs::remove_file(&path) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e).with_context(|| format!("removing {}", path.display())),
        }
    }

    fn near_ai_notice_marker_path(&self) -> PathBuf {
        self.dir.join(NEAR_AI_NOTICE_MARKER_FILE)
    }

    /// Whether the first-use NEAR AI privacy-filter notice has already been
    /// recorded. This read-only check lets the renderer show the disclosure
    /// before message text leaves the machine without consuming it until the
    /// filter has actually succeeded.
    pub fn near_ai_notice_shown(&self) -> bool {
        self.near_ai_notice_marker_path().exists()
    }

    /// Record that the one-time first-use NEAR AI privacy-filter notice was
    /// shown and the filter then succeeded. Returns `true` when this call
    /// creates the marker and `false` when it was already present.
    pub fn ensure_near_ai_notice_shown(&self) -> Result<bool> {
        let path = self.near_ai_notice_marker_path();
        if path.exists() {
            return Ok(false);
        }
        std::fs::File::create(&path).with_context(|| format!("creating {}", path.display()))?;
        Ok(true)
    }

    /// Remove all local contributor state (logout).
    ///
    /// Also sweeps orphaned atomic-write temp files (`.{name}.tmp-{uuid}`)
    /// that can be left behind if the process crashes between creating the
    /// temp file and renaming it into place in `write_atomic_0600`.
    pub fn wipe(&self) -> Result<()> {
        for name in [
            CONFIG_FILE,
            DEVICE_KEY_FILE,
            RECEIPTS_FILE,
            NEAR_AI_NOTICE_MARKER_FILE,
            DAEMON_SETTINGS_FILE,
            DAEMON_STATE_FILE,
            DAEMON_PROJECTS_FILE,
            DAEMON_QUEUE_FILE,
            DAEMON_HISTORY_FILE,
            DAEMON_AUDIT_FILE,
            ACCOUNT_SESSION_FILE,
        ] {
            let path = self.dir.join(name);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        }

        let tmp_prefixes: Vec<String> = [
            CONFIG_FILE,
            DEVICE_KEY_FILE,
            RECEIPTS_FILE,
            DAEMON_SETTINGS_FILE,
            DAEMON_STATE_FILE,
            DAEMON_PROJECTS_FILE,
            DAEMON_QUEUE_FILE,
            DAEMON_HISTORY_FILE,
            DAEMON_AUDIT_FILE,
            ACCOUNT_SESSION_FILE,
        ]
        .into_iter()
        .map(|name| format!(".{name}.tmp-"))
        .collect();
        let entries = match std::fs::read_dir(&self.dir) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(e).with_context(|| format!("reading dir {}", self.dir.display()));
            }
        };
        for entry in entries {
            let entry = entry.with_context(|| format!("reading dir {}", self.dir.display()))?;
            let file_name = entry.file_name();
            let file_name = file_name.to_string_lossy();
            // Stored approved envelopes are one file per queue entry, so they
            // cannot be named in the fixed list above. They are redacted trace
            // content at rest and must not outlive the enrollment that
            // produced them -- see `daemon::approved_envelope`. Their temp
            // files share the same prefix and are swept by the same test.
            let is_approved_envelope = file_name.starts_with(DAEMON_APPROVED_ENVELOPE_PREFIX)
                || file_name.starts_with(&format!(".{DAEMON_APPROVED_ENVELOPE_PREFIX}"));
            if is_approved_envelope
                || tmp_prefixes
                    .iter()
                    .any(|prefix| file_name.starts_with(prefix))
            {
                let path = entry.path();
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing orphaned temp file {}", path.display()))?;
            }
        }
        Ok(())
    }
}

/// Test-only helpers shared with the daemon modules, which all need a
/// throwaway store rooted in a tempdir.
#[cfg(test)]
pub(crate) mod tests_support {
    use super::ConfigStore;

    pub(crate) fn temp_store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }
}

/// Write `body` to `path` atomically (temp file in the same dir, then
/// rename), setting 0600 permissions on unix.
fn write_atomic_0600(dir: &Path, path: &Path, body: &[u8]) -> Result<()> {
    let file_name = path
        .file_name()
        .context("destination path has no file name")?
        .to_string_lossy();
    let tmp_path = dir.join(format!(".{file_name}.tmp-{}", Uuid::new_v4()));
    {
        #[cfg(unix)]
        let mut tmp = {
            use std::os::unix::fs::OpenOptionsExt;
            std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp_path)
                .with_context(|| format!("creating temp file {}", tmp_path.display()))?
        };
        #[cfg(not(unix))]
        let mut tmp = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating temp file {}", tmp_path.display()))?;
        tmp.write_all(body)
            .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
        tmp.sync_all()
            .with_context(|| format!("syncing temp file {}", tmp_path.display()))?;
    }
    if let Err(e) = std::fs::rename(&tmp_path, path) {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(e)
            .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn sample_config() -> ContributorConfig {
        ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "user-1".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[test]
    fn config_round_trip_and_permissions() {
        let (_d, store) = store();
        assert!(store.load_config().unwrap().is_none());
        store.save_config(&sample_config()).unwrap();
        let loaded = store.load_config().unwrap().unwrap();
        assert_eq!(loaded.tenant_id, "tenant-abc");
        let mode = std::fs::metadata(store_path(&store, "contributor.json"))
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn device_key_round_trip_and_permissions() {
        let (_d, store) = store();
        assert!(store.load_device_key().unwrap().is_none());
        store.save_device_key(b"fake-der-bytes").unwrap();
        assert_eq!(store.load_device_key().unwrap().unwrap(), b"fake-der-bytes");
        let mode = std::fs::metadata(store.device_key_path())
            .unwrap()
            .permissions()
            .mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn receipts_append_load_and_skip_garbage() {
        let (_d, store) = store();
        let r = Receipt {
            submission_id: uuid::Uuid::new_v4(),
            session_hash: "sha256:aa".into(),
            source: "claude-code".into(),
            submitted_at: chrono::Utc::now(),
            status: "accepted".into(),
        };
        store.append_receipt(&r).unwrap();
        // Simulate a corrupt line.
        std::fs::write(
            store_path(&store, "receipts.jsonl"),
            format!("{}\nnot-json\n", serde_json::to_string(&r).unwrap()),
        )
        .unwrap();
        let loaded = store.load_receipts().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].session_hash, "sha256:aa");
    }

    #[test]
    fn near_ai_notice_shown_once_then_silent() {
        let (_d, store) = store();
        // Marker absent: first call reports "shown" and creates the marker.
        assert!(store.ensure_near_ai_notice_shown().unwrap());
        assert!(store_path(&store, "near-ai-notice-shown").exists());
        // Marker present: subsequent calls stay silent.
        assert!(!store.ensure_near_ai_notice_shown().unwrap());
        assert!(!store.ensure_near_ai_notice_shown().unwrap());
    }

    #[test]
    fn wipe_removes_state() {
        let (_d, store) = store();
        store.save_config(&sample_config()).unwrap();
        store.save_device_key(b"k").unwrap();
        assert!(store.ensure_near_ai_notice_shown().unwrap());
        store.wipe().unwrap();
        assert!(store.load_config().unwrap().is_none());
        assert!(store.load_device_key().unwrap().is_none());
        // Logout must also clear the first-use notice marker so a
        // re-enrolled user sees the notice again.
        assert!(store.ensure_near_ai_notice_shown().unwrap());
    }

    #[test]
    fn wipe_removes_daemon_state() {
        // Daemon state outliving a logout would hand the next person to
        // enroll on this machine the previous contributor's auto-upload
        // opt-ins and their contribution history.
        let (_d, store) = store();
        let names = [
            DAEMON_SETTINGS_FILE,
            DAEMON_STATE_FILE,
            DAEMON_PROJECTS_FILE,
            DAEMON_QUEUE_FILE,
            DAEMON_HISTORY_FILE,
            DAEMON_AUDIT_FILE,
            ACCOUNT_SESSION_FILE,
        ];
        for name in names {
            store.write_daemon_file(name, b"{}").unwrap();
        }
        store.wipe().unwrap();
        for name in names {
            assert!(
                store.read_daemon_file(name).unwrap().is_none(),
                "{name} survived logout"
            );
        }
    }

    #[test]
    fn wipe_removes_orphaned_daemon_temp_files() {
        let (_d, store) = store();
        let orphan = store.dir().join(".daemon-queue.jsonl.tmp-deadbeef");
        std::fs::write(&orphan, b"leftover").unwrap();
        store.wipe().unwrap();
        assert!(!orphan.exists());
    }

    #[test]
    fn wipe_removes_orphaned_temp_files() {
        let (_d, store) = store();
        let orphan = store.dir().join(".device.pk8.tmp-deadbeef");
        std::fs::write(&orphan, b"leftover-key-material").unwrap();
        assert!(orphan.exists());
        store.wipe().unwrap();
        assert!(!orphan.exists());
    }

    // Test helper: expose the file path for assertions.
    fn store_path(store: &ConfigStore, name: &str) -> std::path::PathBuf {
        store.dir().join(name)
    }
}
