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
    time::{Duration, Instant},
};
use tokio::process::{Child, Command};

#[derive(Debug, thiserror::Error)]
#[error("worker-already-running")]
pub(super) struct WorkerAlreadyRunning;

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

    fn sleeping_worker_for(seconds: &str) -> (tempfile::TempDir, WorkerProcess) {
        let root = tempfile::tempdir().unwrap();
        let mut worker = WorkerProcess::new(
            root.path(),
            LocalWorkerConfig {
                binary: PathBuf::from("/usr/bin/true"),
                expected_sha256: "00".repeat(32),
                coordinator: "ws://127.0.0.1:9999".into(),
                startup_timeout_secs: 1,
            },
        )
        .unwrap();
        crate::config::ConfigStore::open(worker.home.join("node")).unwrap();
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(worker.home.join("node/controller.lock"))
            .unwrap();
        lock.try_lock().unwrap();
        worker.controller_lock = Some(lock);
        worker.child = Some(
            Command::new("/bin/sleep")
                .arg(seconds)
                .kill_on_drop(true)
                .spawn()
                .unwrap(),
        );
        (root, worker)
    }

    fn sleeping_worker() -> (tempfile::TempDir, WorkerProcess) {
        sleeping_worker_for("30")
    }

    #[tokio::test]
    async fn guarded_spawn_refusal_never_owns_child_and_releases_lock() {
        let (_root, mut worker) = sleeping_worker();
        worker
            .stop_with_urgency(
                tokio::sync::watch::channel(Some(Instant::now() - Duration::from_secs(2))).1,
            )
            .await;
        worker.config.expected_sha256 = hex::encode(Sha256::digest(
            std::fs::read(&worker.config.binary).unwrap(),
        ));
        let mut called = false;
        let result = worker
            .start_guarded(1, |_| {
                called = true;
                Err(std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "resource-policy-refused",
                ))
            })
            .await;
        assert!(called);
        assert!(result.is_err());
        assert!(worker.child.is_none());
        assert!(worker.controller_lock.is_none());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(worker.home.join("node/controller.lock"))
            .unwrap();
        lock.try_lock().unwrap();
    }

    #[tokio::test]
    async fn urgent_observation_before_stop_kills_and_releases_ownership() {
        let (_root, mut worker) = sleeping_worker();
        let (sender, receiver) =
            tokio::sync::watch::channel(Some(Instant::now() - Duration::from_secs(2)));
        drop(sender);
        let report =
            tokio::time::timeout(Duration::from_secs(2), worker.stop_with_urgency(receiver))
                .await
                .unwrap();
        assert_eq!(report.process, StopOutcome::Forced);
        assert!(report.stopped);
        assert!(report.drain.is_none());
        assert!(worker.child.is_none());
        assert!(worker.controller_lock.is_none());
    }

    #[tokio::test]
    async fn urgent_escalation_interrupts_an_unresponsive_drain() {
        let (_root, mut worker) = sleeping_worker();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        worker.address = Some(listener.local_addr().unwrap());
        worker.credential = Some(wire::Credential::from_seed(&[7; 32]).unwrap());
        let (sender, receiver) = tokio::sync::watch::channel(None);
        let stop = worker.stop_with_urgency(receiver);
        let escalation = async {
            let (_socket, _) = listener.accept().await.unwrap();
            sender.send(Some(Instant::now())).unwrap();
            tokio::time::sleep(Duration::from_secs(2)).await;
        };
        let started = Instant::now();
        let report = tokio::time::timeout(Duration::from_millis(1900), async {
            tokio::pin!(escalation);
            tokio::select! {
                report = stop => report,
                _ = &mut escalation => panic!("drain did not escalate"),
            }
        })
        .await
        .unwrap();
        assert!(started.elapsed() >= Duration::from_millis(900));
        assert_eq!(report.process, StopOutcome::Forced);
        assert!(report.stopped);
        assert!(report.drain.is_none());
    }

    #[tokio::test]
    async fn canceled_stop_retains_child_and_lock_then_escalates_during_exit_wait() {
        let (_root, mut worker) = sleeping_worker();
        // A concurrent fork/dup can retain the same open-file description.
        // Closing our descriptor alone must not leave its flock held after reap.
        let inherited = worker
            .controller_lock
            .as_ref()
            .unwrap()
            .try_clone()
            .unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(20), worker.stop())
                .await
                .is_err()
        );
        assert!(worker.child.is_some());
        assert!(worker.controller_lock.is_some());
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .open(worker.home.join("node/controller.lock"))
            .unwrap();
        assert!(lock.try_lock().is_err());
        let (sender, receiver) = tokio::sync::watch::channel(None);
        let escalation = async {
            tokio::time::sleep(Duration::from_millis(20)).await;
            sender
                .send(Some(Instant::now() - Duration::from_secs(2)))
                .unwrap();
        };
        let (report, ()) = tokio::time::timeout(Duration::from_secs(2), async {
            tokio::join!(worker.stop_with_urgency(receiver), escalation)
        })
        .await
        .unwrap();
        assert_eq!(report.process, StopOutcome::Forced);
        assert!(report.stopped);
        lock.try_lock().unwrap();
        drop(inherited);
    }

    #[tokio::test]
    async fn normal_stop_allows_cooperative_child_exit() {
        let (_root, mut worker) = sleeping_worker_for("0.1");
        let report = worker.stop().await;
        assert_eq!(report.process, StopOutcome::Exited);
        assert!(report.stopped);
    }

    #[tokio::test]
    async fn urgency_deadline_survives_sender_close_and_later_reset() {
        let (_sender, receiver) =
            tokio::sync::watch::channel(Some(Instant::now() - Duration::from_secs(2)));
        assert!(
            cooperative_until_urgent(std::future::pending::<()>(), receiver)
                .await
                .is_none()
        );
        let (sender, receiver) =
            tokio::sync::watch::channel(Some(Instant::now() - Duration::from_millis(900)));
        let reset = async {
            tokio::time::sleep(Duration::from_millis(10)).await;
            sender.send(None).unwrap();
            drop(sender);
        };
        let (result, ()) = tokio::time::timeout(Duration::from_secs(1), async {
            tokio::join!(
                cooperative_until_urgent(std::future::pending::<()>(), receiver),
                reset
            )
        })
        .await
        .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn signed_child_publishes_endpoint_and_completes_readiness_and_drain() {
        let root = tempfile::tempdir().unwrap();
        let mut worker =
            WorkerProcess::new(root.path(), super::super::test_worker::config(root.path()))
                .unwrap();
        crate::config::ConfigStore::open(worker.home.join("node")).unwrap();
        // An earlier launch's endpoint cannot satisfy the fresh launch instance.
        std::fs::write(
            worker.home.join("node/worker-endpoint.json"),
            serde_json::to_vec(
                &serde_json::json!({"version": 0, "instance": vec![0u8; 32], "address": "127.0.0.1:1"}),
            )
            .unwrap(),
        )
        .unwrap();
        let status = worker.start(1).await.unwrap();
        assert_eq!(status.state, wire::State::Training);
        assert_eq!(status.admission, wire::Admission::Assigned);
        assert!(worker.child.is_some());
        assert!(worker.read_endpoint().is_ok());
        let report = worker.stop().await;
        assert!(report.stopped);
        assert_eq!(report.drain, Some(wire::DrainOutcome::Acknowledged));
        assert_eq!(report.process, StopOutcome::Exited);
    }

    #[test]
    fn endpoint_requires_current_instance_loopback_port_and_exact_size_bound() {
        let root = tempfile::tempdir().unwrap();
        let mut worker =
            WorkerProcess::new(root.path(), super::super::test_worker::config(root.path()))
                .unwrap();
        crate::config::ConfigStore::open(worker.home.join("node")).unwrap();
        let credential = wire::Credential::from_seed(&[7; 32]).unwrap();
        let valid = serde_json::json!({"version": 0, "instance": credential.instance(), "address": "127.0.0.1:1234"});
        worker.credential = Some(credential);
        let endpoint = worker.home.join("node/worker-endpoint.json");
        for (field, value) in [
            ("version", serde_json::json!(1)),
            ("instance", serde_json::json!(vec![0u8; 32])),
            ("address", serde_json::json!("192.0.2.1:1234")),
            ("address", serde_json::json!("127.0.0.1:0")),
        ] {
            let mut invalid = valid.clone();
            invalid[field] = value;
            std::fs::write(&endpoint, serde_json::to_vec(&invalid).unwrap()).unwrap();
            assert_eq!(
                worker.read_endpoint().unwrap_err().to_string(),
                "worker-endpoint-mismatch"
            );
        }
        let mut bytes = serde_json::to_vec(&valid).unwrap();
        bytes.resize(4096, b' ');
        std::fs::write(&endpoint, &bytes).unwrap();
        assert_eq!(
            worker.read_endpoint().unwrap(),
            "127.0.0.1:1234".parse().unwrap()
        );
        bytes.push(b' ');
        std::fs::write(&endpoint, &bytes).unwrap();
        assert_eq!(
            worker.read_endpoint().unwrap_err().to_string(),
            "worker-endpoint-too-large"
        );
    }

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
        assert!(
            worker
                .start(1)
                .await
                .unwrap_err()
                .is::<WorkerAlreadyRunning>()
        );
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

    #[cfg(all(test, unix))]
    pub async fn start(&mut self, allowance: u64) -> anyhow::Result<wire::Status> {
        self.start_guarded(allowance, Command::spawn).await
    }

    /// The caller can serialize its final policy check with the actual spawn.
    /// No await or worker preparation occurs between this callback and ownership.
    pub async fn start_guarded(
        &mut self,
        allowance: u64,
        spawn: impl FnOnce(&mut Command) -> std::io::Result<Child>,
    ) -> anyhow::Result<wire::Status> {
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
            match file.try_lock() {
                Ok(()) => Ok(file),
                Err(std::fs::TryLockError::WouldBlock) if name == "worker.lock" => {
                    Err(WorkerAlreadyRunning.into())
                }
                Err(error) => Err(error.into()),
            }
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
        let spawned = spawn(&mut command);
        // Drop Command's copy promptly; this is not a secure wipe of String or
        // the child's process environment, which remain part of the local trust boundary.
        command.env_remove(wire::CREDENTIAL_ENV);
        seed.fill(0);
        let child = spawned?;
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

    #[cfg(all(test, unix))]
    pub async fn stop(&mut self) -> StopReport {
        let (_sender, receiver) = tokio::sync::watch::channel(None);
        self.stop_with_urgency(receiver).await
    }

    /// Escalation may arrive during drain or exit waiting. The first urgent
    /// observation caps all remaining cooperative work at observation + 1s.
    /// The child and ownership lock remain in self across cancellation/failure.
    pub async fn stop_with_urgency(
        &mut self,
        urgency: tokio::sync::watch::Receiver<Option<Instant>>,
    ) -> StopReport {
        let Some(child) = self.child.as_mut() else {
            return StopReport {
                drain: None,
                process: StopOutcome::NotRunning,
                stopped: true,
            };
        };
        let mut drain = None;
        let cooperative = async {
            if matches!(child.try_wait(), Ok(None)) {
                if let (Some(credential), Some(address)) = (&self.credential, self.address) {
                    drain = credential
                        .exchange(address, wire::Command::Drain)
                        .await
                        .ok()
                        .map(|s| s.drain);
                }
            }
            matches!(
                tokio::time::timeout(Duration::from_secs(3), child.wait()).await,
                Ok(Ok(_))
            )
        };
        let exited = cooperative_until_urgent(cooperative, urgency).await == Some(true);
        let mut process = StopOutcome::Exited;
        let mut stopped = if exited {
            true
        } else {
            process = StopOutcome::Forced;
            if child.start_kill().is_err() {
                false
            } else {
                matches!(
                    tokio::time::timeout(Duration::from_secs(2), child.wait()).await,
                    Ok(Ok(_))
                )
            }
        };
        if stopped
            && self
                .controller_lock
                .as_ref()
                .is_some_and(|lock| lock.unlock().is_err())
        {
            // Reaping preceded this attempt. Retain ownership evidence and retry
            // rather than claiming successful release after an unlock failure.
            stopped = false;
        }
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

async fn cooperative_until_urgent<T>(
    work: impl std::future::Future<Output = T>,
    mut urgency: tokio::sync::watch::Receiver<Option<Instant>>,
) -> Option<T> {
    tokio::pin!(work);
    let mut first_urgent = *urgency.borrow_and_update();
    let mut channel_open = true;
    loop {
        let deadline = async {
            if let Some(at) = first_urgent {
                tokio::time::sleep_until((at + Duration::from_secs(1)).into()).await;
            } else {
                std::future::pending::<()>().await;
            }
        };
        tokio::select! {
            biased;
            _ = deadline => return None,
            changed = urgency.changed(), if channel_open => {
                channel_open = changed.is_ok();
                if let Some(at) = *urgency.borrow_and_update() {
                    first_urgent = Some(first_urgent.map_or(at, |earlier| earlier.min(at)));
                }
            }
            result = &mut work => return Some(result),
        }
    }
}
