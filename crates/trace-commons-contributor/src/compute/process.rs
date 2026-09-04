//! Owned local-development worker; production artifact policy is deliberately absent.

use super::{ComputeSettingsStore, worker_protocol as wire};
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::{File, OpenOptions},
    io::Read,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};
use tokio::process::{Child, Command};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalWorkerConfig {
    pub binary: PathBuf,
    pub expected_sha256: String,
    pub coordinator: String,
    pub startup_timeout_secs: u64,
}
impl LocalWorkerConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(cfg!(debug_assertions), "local-worker-disabled-in-release");
        anyhow::ensure!(cfg!(unix), "local-worker-platform-unavailable");
        anyhow::ensure!(
            self.binary.is_absolute() && self.binary.is_file(),
            "worker-binary-invalid"
        );
        anyhow::ensure!(
            (1..=60).contains(&self.startup_timeout_secs),
            "worker-deadline-invalid"
        );
        anyhow::ensure!(
            self.expected_sha256.len() == 64 && hex::decode(&self.expected_sha256).is_ok(),
            "worker-hash-invalid"
        );
        let url = url::Url::parse(&self.coordinator)?;
        let ip: std::net::IpAddr = url
            .host_str()
            .unwrap_or("")
            .trim_matches(['[', ']'])
            .parse()?;
        anyhow::ensure!(
            ip.is_loopback()
                && matches!(url.scheme(), "ws" | "wss")
                && url.username().is_empty()
                && url.password().is_none(),
            "local-coordinator-required"
        );
        Ok(())
    }
    fn verify_binary(&self) -> anyhow::Result<()> {
        self.validate()?;
        let mut file = File::open(&self.binary)?;
        let mut digest = Sha256::new();
        std::io::copy(&mut file, &mut DigestWriter(&mut digest))?;
        anyhow::ensure!(
            hex::encode(digest.finalize()).eq_ignore_ascii_case(&self.expected_sha256),
            "worker-binary-hash-mismatch"
        );
        Ok(())
    }
}
struct DigestWriter<'a>(&'a mut Sha256);
impl std::io::Write for DigestWriter<'_> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.update(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StopOutcome {
    NotRunning,
    Exited,
    Forced,
    Failed,
}
#[derive(Debug, Clone, Copy)]
pub struct StopReport {
    pub drain: Option<wire::DrainOutcome>,
    pub process: StopOutcome,
    pub stopped: bool,
}

pub struct WorkerProcess {
    // Drop order: kill_on_drop child before releasing controller ownership.
    child: Option<Child>,
    controller_lock: Option<File>,
    credential: Option<wire::Credential>,
    address: Option<SocketAddr>,
    config: LocalWorkerConfig,
    home: PathBuf,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Endpoint {
    version: u32,
    instance: [u8; 32],
    address: SocketAddr,
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    #[tokio::test]
    async fn wrong_artifact_and_existing_worker_fail_before_launch_and_release_parent_lock() {
        let root = tempfile::tempdir().unwrap();
        let binary = PathBuf::from("/usr/bin/true");
        let config = LocalWorkerConfig {
            expected_sha256: "00".repeat(32),
            binary: binary.clone(),
            coordinator: "ws://127.0.0.1:9999".into(),
            startup_timeout_secs: 1,
        };
        let mut worker = WorkerProcess::new(root.path(), config).unwrap();
        assert!(worker.start(1).await.is_err());
        assert!(worker.child.is_none());
        worker.config.expected_sha256 = hex::encode(Sha256::digest(std::fs::read(binary).unwrap()));
        crate::config::ConfigStore::open(worker.home.join("node")).unwrap();
        let held = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(worker.home.join("node/worker.lock"))
            .unwrap();
        held.try_lock().unwrap();
        assert!(worker.start(1).await.is_err());
        assert!(worker.child.is_none());
        let parent = OpenOptions::new()
            .read(true)
            .write(true)
            .open(worker.home.join("node/controller.lock"))
            .unwrap();
        parent.try_lock().unwrap();
        assert!(worker.stop().await.stopped);
    }
}

impl WorkerProcess {
    pub fn new(root: &Path, config: LocalWorkerConfig) -> anyhow::Result<Self> {
        config.validate()?;
        Ok(Self {
            child: None,
            controller_lock: None,
            credential: None,
            address: None,
            home: ComputeSettingsStore::open(root)?.worker_home(),
            config,
        })
    }

    pub async fn start(&mut self, allowance: u64) -> anyhow::Result<wire::Status> {
        anyhow::ensure!(self.child.is_none(), "worker-already-owned");
        self.config.verify_binary()?;
        for name in ["", "node", "host-home", "cache", "tmp"] {
            crate::config::ConfigStore::open(self.home.join(name))?;
        }
        let lock = |name: &str| -> anyhow::Result<File> {
            let file = OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(self.home.join("node").join(name))?;
            file.try_lock()?;
            Ok(file)
        };
        let parent_lock = lock("controller.lock")?;
        drop(lock("worker.lock")?);
        let mut seed = [0; 32];
        SystemRandom::new()
            .fill(&mut seed)
            .map_err(|_| anyhow::anyhow!("worker-random-failed"))?;
        let credential = wire::Credential::from_seed(&seed)?;
        let mut command = Command::new(&self.config.binary);
        command
            .args([
                "node",
                "run",
                "--coordinator",
                &self.config.coordinator,
                "--free-mem-gb",
                &allowance.to_string(),
                "--status-socket",
                "127.0.0.1:0",
                "--skip-input",
                "--payout",
                "compute-pilot.testnet",
            ])
            .env_clear()
            .env("HOLONEAR_HOME", &self.home)
            .env(wire::CREDENTIAL_ENV, hex::encode(seed))
            .env("HOME", self.home.join("host-home"))
            .env("USERPROFILE", self.home.join("host-home"))
            .env("XDG_CACHE_HOME", self.home.join("cache"))
            .env("TMPDIR", self.home.join("tmp"))
            .env("TMP", self.home.join("tmp"))
            .env("TEMP", self.home.join("tmp"))
            .env("HOLONEAR_PEER_TRANSPORT", "coordinator")
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .current_dir(&self.home)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(true);
        #[cfg(windows)]
        if let Some(root) = std::env::var_os("SystemRoot") {
            command.env("SystemRoot", root);
        }
        let child = command.spawn()?;
        seed.fill(0);
        self.child = Some(child);
        self.controller_lock = Some(parent_lock);
        self.credential = Some(credential);
        self.address = None;
        let deadline = Duration::from_secs(self.config.startup_timeout_secs);
        tokio::time::timeout(deadline, async {
            loop {
                anyhow::ensure!(!self.exited()?, "worker-exited-before-ready");
                if let Ok(address) = self.read_endpoint() {
                    self.address = Some(address);
                    if let Ok(status) = self.status().await {
                        return Ok(status);
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        })
        .await
        .map_err(|_| anyhow::anyhow!("worker-readiness-timeout"))?
    }

    fn read_endpoint(&self) -> anyhow::Result<SocketAddr> {
        let file = File::open(self.home.join("node/worker-endpoint.json"))?;
        let mut bytes = Vec::new();
        file.take(4097).read_to_end(&mut bytes)?;
        anyhow::ensure!(bytes.len() <= 4096, "worker-endpoint-too-large");
        let endpoint: Endpoint = serde_json::from_slice(&bytes)?;
        let credential = self
            .credential
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("worker-not-started"))?;
        anyhow::ensure!(
            endpoint.version == wire::VERSION
                && endpoint.instance == credential.instance()
                && endpoint.address.ip().is_loopback()
                && endpoint.address.port() != 0,
            "worker-endpoint-mismatch"
        );
        Ok(endpoint.address)
    }

    fn exited(&mut self) -> anyhow::Result<bool> {
        match self.child.as_mut() {
            Some(child) => Ok(child.try_wait()?.is_some()),
            None => Ok(true),
        }
    }

    pub async fn status(&mut self) -> anyhow::Result<wire::Status> {
        anyhow::ensure!(!self.exited()?, "worker-exited");
        let address = self
            .address
            .ok_or_else(|| anyhow::anyhow!("worker-not-ready"))?;
        self.credential
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("worker-not-ready"))?
            .exchange(address, wire::Command::Status)
            .await
    }

    pub async fn stop(&mut self) -> StopReport {
        let Some(child) = self.child.as_mut() else {
            return StopReport {
                drain: None,
                process: StopOutcome::NotRunning,
                stopped: true,
            };
        };
        let mut drain = None;
        if matches!(child.try_wait(), Ok(None)) {
            if let (Some(credential), Some(address)) = (&self.credential, self.address) {
                drain = credential
                    .exchange(address, wire::Command::Drain)
                    .await
                    .ok()
                    .map(|s| s.drain);
            }
        }
        let mut process = StopOutcome::Exited;
        let stopped = match tokio::time::timeout(Duration::from_secs(3), child.wait()).await {
            Ok(Ok(_)) => true,
            _ => {
                process = StopOutcome::Forced;
                if child.start_kill().is_err() {
                    false
                } else {
                    matches!(
                        tokio::time::timeout(Duration::from_secs(2), child.wait()).await,
                        Ok(Ok(_))
                    )
                }
            }
        };
        if stopped {
            self.child = None;
            self.controller_lock = None;
            self.credential = None;
            self.address = None;
        } else {
            process = StopOutcome::Failed;
        }
        StopReport {
            drain,
            process,
            stopped,
        }
    }
}
