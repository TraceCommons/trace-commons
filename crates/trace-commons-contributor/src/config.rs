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
use uuid::Uuid;

pub const CONTRIBUTOR_CONFIG_SCHEMA_VERSION: &str = "trace_commons.contributor_config.v1";

const CONFIG_FILE: &str = "contributor.json";
const DEVICE_KEY_FILE: &str = "device.pk8";
const RECEIPTS_FILE: &str = "receipts.jsonl";

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
        let cfg = serde_json::from_str(&raw)
            .with_context(|| format!("parsing {}", path.display()))?;
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
        let bytes =
            std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
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

    /// Remove all local contributor state (logout).
    pub fn wipe(&self) -> Result<()> {
        for name in [CONFIG_FILE, DEVICE_KEY_FILE, RECEIPTS_FILE] {
            let path = self.dir.join(name);
            if path.exists() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("removing {}", path.display()))?;
            }
        }
        Ok(())
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
        let mut tmp = std::fs::File::create(&tmp_path)
            .with_context(|| format!("creating temp file {}", tmp_path.display()))?;
        tmp.write_all(body)
            .with_context(|| format!("writing temp file {}", tmp_path.display()))?;
        tmp.sync_all().ok();
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp_path, std::fs::Permissions::from_mode(0o600))
            .with_context(|| format!("setting permissions on {}", tmp_path.display()))?;
    }
    std::fs::rename(&tmp_path, path)
        .with_context(|| format!("renaming {} to {}", tmp_path.display(), path.display()))?;
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
        let mode = std::fs::metadata(store_path(&store, "contributor.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn device_key_round_trip_and_permissions() {
        let (_d, store) = store();
        assert!(store.load_device_key().unwrap().is_none());
        store.save_device_key(b"fake-der-bytes").unwrap();
        assert_eq!(store.load_device_key().unwrap().unwrap(), b"fake-der-bytes");
        let mode = std::fs::metadata(store.device_key_path()).unwrap().permissions().mode();
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
    fn wipe_removes_state() {
        let (_d, store) = store();
        store.save_config(&sample_config()).unwrap();
        store.save_device_key(b"k").unwrap();
        store.wipe().unwrap();
        assert!(store.load_config().unwrap().is_none());
        assert!(store.load_device_key().unwrap().is_none());
    }

    // Test helper: expose the file path for assertions.
    fn store_path(store: &ConfigStore, name: &str) -> std::path::PathBuf {
        store.dir().join(name)
    }
}
