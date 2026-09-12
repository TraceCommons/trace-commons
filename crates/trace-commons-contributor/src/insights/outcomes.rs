//! Explicitly linked observations, never inferred acceptance or task success.
//! Git inspection is local and read-only. Imported test reports are assertions
//! from their producer, not tests executed or independently verified here.
use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const MAX_TEST_REPORT_BYTES: usize = 64 * 1024;
const MAX_COMMIT_BYTES: usize = 64 * 1024;
const MAX_PARENTS: usize = 1024;
const GIT_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum GitEvidenceProvenance {
    InspectedLocalObject,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum TestEvidenceProvenance {
    ImportedReport,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct GitCommitEvidence {
    pub repository_path_digest: String,
    pub object_id: String,
    pub tree_id: String,
    pub parent_ids: Vec<String>,
    pub inspected_at: DateTime<Utc>,
    pub provenance: GitEvidenceProvenance,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct TestReportEvidence {
    pub schema_version: u32,
    pub runner: String,
    pub passed: u64,
    pub failed: u64,
    pub skipped: u64,
    pub observed_at: DateTime<Utc>,
    /// An imported association only; not a commit-existence check.
    pub commit_id: Option<String>,
    pub artifact_digest: String,
    pub imported_at: DateTime<Utc>,
    pub provenance: TestEvidenceProvenance,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "type",
    content = "evidence",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum OutcomeEvidence {
    GitCommit(GitCommitEvidence),
    TestReport(TestReportEvidence),
}
impl OutcomeEvidence {
    pub fn validate(&self) -> Result<()> {
        match self {
            Self::GitCommit(evidence) => evidence.validate(),
            Self::TestReport(evidence) => evidence.validate(),
        }
    }
    pub fn identity_digest(&self) -> Result<String> {
        match self {
            Self::GitCommit(evidence) => evidence.identity_digest(),
            Self::TestReport(evidence) => evidence.identity_digest(),
        }
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hex_id(value: &str, length: usize) -> bool {
    value.len() == length
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
fn object_id(value: &str) -> bool {
    hex_id(value, 40) || hex_id(value, 64)
}
fn timestamp(value: DateTime<Utc>) -> Result<()> {
    // Permit five minutes of clock skew; reject pre-epoch and implausible future records.
    if value.timestamp() < 0 || value > Utc::now() + chrono::Duration::minutes(5) {
        bail!("insights-outcome-timestamp-invalid");
    }
    Ok(())
}
impl GitCommitEvidence {
    pub fn validate(&self) -> Result<()> {
        if !hex_id(&self.repository_path_digest, 64)
            || !object_id(&self.object_id)
            || !hex_id(&self.tree_id, self.object_id.len())
            || self.parent_ids.len() > MAX_PARENTS
            || self
                .parent_ids
                .iter()
                .any(|parent| !hex_id(parent, self.object_id.len()))
        {
            bail!("insights-git-evidence-invalid");
        }
        timestamp(self.inspected_at)
    }
    pub fn identity_digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(
            format!(
                "git-commit:{}:{}",
                self.repository_path_digest, self.object_id
            )
            .as_bytes(),
        ))
    }
}
impl TestReportEvidence {
    pub fn validate(&self) -> Result<()> {
        if self.schema_version != 1
            || self.runner.is_empty()
            || self.runner.len() > 96
            || !self
                .runner
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
            || !hex_id(&self.artifact_digest, 64)
            || self.commit_id.as_ref().is_some_and(|id| !object_id(id))
            || self
                .passed
                .checked_add(self.failed)
                .and_then(|sum| sum.checked_add(self.skipped))
                .is_none()
        {
            bail!("insights-test-report-invalid");
        }
        timestamp(self.observed_at)?;
        timestamp(self.imported_at)?;
        if self.observed_at > self.imported_at + chrono::Duration::minutes(5) {
            bail!("insights-outcome-timestamp-invalid");
        }
        Ok(())
    }
    pub fn identity_digest(&self) -> Result<String> {
        self.validate()?;
        Ok(digest(
            format!("test-report:{}", self.artifact_digest).as_bytes(),
        ))
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TestReportInput {
    schema_version: u32,
    runner: String,
    passed: u64,
    failed: u64,
    skipped: u64,
    observed_at: DateTime<Utc>,
    commit_id: Option<String>,
}
/// Parse strict schema-v1 JSON, binding provenance to the exact imported bytes.
pub fn parse_test_report(bytes: &[u8]) -> Result<TestReportEvidence> {
    if bytes.len() > MAX_TEST_REPORT_BYTES {
        bail!("insights-test-report-too-large");
    }
    let input: TestReportInput =
        serde_json::from_slice(bytes).map_err(|_| anyhow!("insights-test-report-invalid"))?;
    let evidence = TestReportEvidence {
        schema_version: input.schema_version,
        runner: input.runner,
        passed: input.passed,
        failed: input.failed,
        skipped: input.skipped,
        observed_at: input.observed_at,
        commit_id: input.commit_id,
        artifact_digest: digest(bytes),
        imported_at: Utc::now(),
        provenance: TestEvidenceProvenance::ImportedReport,
    };
    evidence.validate()?;
    Ok(evidence)
}
pub fn import_test_report(path: &Path) -> Result<TestReportEvidence> {
    let file = crate::evidence_import::open_import_file(path)
        .map_err(|_| anyhow!("insights-test-report-unreadable"))?;
    let mut bytes = Vec::new();
    file.take(MAX_TEST_REPORT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("insights-test-report-unreadable"))?;
    parse_test_report(&bytes)
}

fn isolated_git(repo: &Path) -> Command {
    configure_git(Command::new("git"), repo)
}
fn configure_git(mut command: Command, repo: &Path) -> Command {
    // Preserve executable lookup and Windows runtime directories only. In particular,
    // no inherited GIT_DIR, worktree, alternate objects, config injection, SSH or pager.
    command.env_clear();
    for key in ["PATH", "SystemRoot", "WINDIR"] {
        if let Some(value) = std::env::var_os(key) {
            command.env(key, value);
        }
    }
    command
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_CEILING_DIRECTORIES", repo)
        .args([
            "--no-pager",
            "--no-replace-objects",
            "--no-lazy-fetch",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=",
            "-c",
            "protocol.allow=never",
            "-C",
        ])
        .arg(repo)
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    command
}
fn read_git(repo: &Path, arguments: &[&str], max_bytes: usize) -> Result<Vec<u8>> {
    let dot_git = repo.join(".git");
    let git_dir = if dot_git.is_file() || dot_git.is_dir() {
        dot_git
    } else if repo.join("HEAD").is_file() && repo.join("objects").is_dir() {
        repo.to_path_buf()
    } else {
        bail!("insights-git-repository-invalid");
    };
    // Explicit Git directory prevents discovery in an enclosing repository.
    // Git resolves .git indirection files itself, retaining linked-worktree support.
    let mut command = isolated_git(repo);
    command.arg("--git-dir").arg(git_dir).args(arguments);
    read_command(command, max_bytes, GIT_TIMEOUT)
}
fn read_command(mut command: Command, max_bytes: usize, timeout: Duration) -> Result<Vec<u8>> {
    let deadline = Instant::now() + timeout;
    let mut child = command
        .stdout(Stdio::piped())
        .spawn()
        .map_err(|_| anyhow!("insights-git-inspection-failed"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow!("insights-git-inspection-failed"))?;
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    if std::thread::Builder::new()
        .spawn(move || {
            let mut bytes = Vec::new();
            let result = stdout
                .take(max_bytes as u64 + 1)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send(result);
        })
        .is_err()
    {
        let _ = child.kill();
        let _ = child.wait();
        bail!("insights-git-inspection-failed");
    }
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    // Never join the reader: an unexpected descendant could retain stdout after
    // the Git child exits. The same deadline bounds reception as process waiting.
    let bytes = receiver
        .recv_timeout(deadline.saturating_duration_since(Instant::now()))
        .map_err(|_| anyhow!("insights-git-inspection-failed"))?
        .map_err(|_| anyhow!("insights-git-inspection-failed"))?;
    if !status.is_some_and(|status| status.success()) || bytes.len() > max_bytes {
        bail!("insights-git-inspection-failed");
    }
    Ok(bytes)
}
/// Inspect an exact local commit object. This does not inspect branches, refs,
/// merge status, acceptance, reverts, authors, messages, or changed paths.
pub fn inspect_git_commit(repo: &Path, requested_id: &str) -> Result<GitCommitEvidence> {
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        if let Some(Component::Prefix(prefix)) = repo.components().next()
            && !matches!(prefix.kind(), Prefix::Disk(_) | Prefix::VerbatimDisk(_))
        {
            bail!("insights-git-repository-invalid");
        }
    }
    if !object_id(requested_id) {
        bail!("insights-git-object-id-invalid");
    }
    let repo = repo
        .canonicalize()
        .map_err(|_| anyhow!("insights-git-repository-invalid"))?;
    if !repo.is_dir() {
        bail!("insights-git-repository-invalid");
    }
    if read_git(&repo, &["cat-file", "-t", requested_id], 16)? != b"commit\n" {
        bail!("insights-git-object-not-commit");
    }
    let size = read_git(&repo, &["cat-file", "-s", requested_id], 32)?;
    let size = std::str::from_utf8(&size)
        .ok()
        .and_then(|value| value.trim().parse::<usize>().ok())
        .ok_or_else(|| anyhow!("insights-git-inspection-failed"))?;
    if size > MAX_COMMIT_BYTES {
        bail!("insights-git-commit-too-large");
    }
    let commit = read_git(
        &repo,
        &["cat-file", "commit", requested_id],
        MAX_COMMIT_BYTES,
    )?;
    let mut tree_id = None;
    let mut parent_ids = Vec::new();
    for line in commit
        .split(|byte| *byte == b'\n')
        .take_while(|line| !line.is_empty())
    {
        if let Some(value) = line.strip_prefix(b"tree ") {
            if tree_id.is_some() {
                bail!("insights-git-commit-invalid");
            }
            tree_id = Some(
                std::str::from_utf8(value)
                    .map_err(|_| anyhow!("insights-git-commit-invalid"))?
                    .to_owned(),
            );
        } else if let Some(value) = line.strip_prefix(b"parent ") {
            if parent_ids.len() >= MAX_PARENTS {
                bail!("insights-git-commit-invalid");
            }
            parent_ids.push(
                std::str::from_utf8(value)
                    .map_err(|_| anyhow!("insights-git-commit-invalid"))?
                    .to_owned(),
            );
        }
    }
    let evidence = GitCommitEvidence {
        repository_path_digest: digest(repo.as_os_str().as_encoded_bytes()),
        object_id: requested_id.to_owned(),
        tree_id: tree_id.ok_or_else(|| anyhow!("insights-git-commit-invalid"))?,
        parent_ids,
        inspected_at: Utc::now(),
        provenance: GitEvidenceProvenance::InspectedLocalObject,
    };
    evidence.validate()?;
    Ok(evidence)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn git(repo: &Path, args: &[&str], input: Option<&[u8]>) -> String {
        let mut command = isolated_git(repo);
        command.args(args).stdout(Stdio::piped());
        if input.is_some() {
            command.stdin(Stdio::piped());
        }
        let mut child = command.spawn().unwrap();
        if let Some(input) = input {
            child.stdin.take().unwrap().write_all(input).unwrap();
        }
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "synthetic Git operation failed");
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }
    fn repo(format: &str) -> (tempfile::TempDir, String, String) {
        let root = tempfile::tempdir().unwrap();
        git(
            root.path(),
            &["init", "--quiet", &format!("--object-format={format}")],
            None,
        );
        let tree = git(root.path(), &["mktree"], Some(b""));
        let commit = commit(root.path(), &tree, &[], "PRIVATE_COMMIT_MESSAGE");
        (root, tree, commit)
    }
    fn commit(repo: &Path, tree: &str, parents: &[&str], message: &str) -> String {
        let mut body = format!("tree {tree}\n");
        for parent in parents {
            body.push_str(&format!("parent {parent}\n"));
        }
        body.push_str("author Private Author <private@example.test> 1700000000 +0000\ncommitter Private Author <private@example.test> 1700000000 +0000\n\n");
        body.push_str(message);
        git(
            repo,
            &["hash-object", "-t", "commit", "-w", "--stdin"],
            Some(body.as_bytes()),
        )
    }
    fn report() -> serde_json::Value {
        serde_json::json!({"schema_version":1,"runner":"cargo-test","passed":3,"failed":1,"skipped":2,"observed_at":"2026-01-01T00:00:00Z","commit_id":null})
    }
    #[test]
    fn git_inspection_retains_only_bounded_object_metadata_and_stable_identity() {
        for format in ["sha1", "sha256"] {
            let (root, tree, first) = repo(format);
            let second = commit(root.path(), &tree, &[&first], "ANOTHER_PRIVATE_MESSAGE");
            let evidence = inspect_git_commit(root.path(), &second).unwrap();
            assert_eq!(evidence.tree_id, tree);
            assert_eq!(evidence.parent_ids, [first]);
            assert_eq!(
                evidence.provenance,
                GitEvidenceProvenance::InspectedLocalObject
            );
            assert_eq!(
                evidence.identity_digest().unwrap(),
                inspect_git_commit(root.path(), &second)
                    .unwrap()
                    .identity_digest()
                    .unwrap()
            );
            let encoded = serde_json::to_string(&evidence).unwrap();
            for secret in [
                "Private Author",
                "private@example",
                "PRIVATE_MESSAGE",
                root.path().to_str().unwrap(),
            ] {
                assert!(!encoded.contains(secret));
            }
            assert!(inspect_git_commit(root.path(), &tree).is_err());
            assert!(inspect_git_commit(&root.path().join(".git"), &second).is_ok());
            let linked = root.path().join("linked");
            git(
                root.path(),
                &[
                    "worktree",
                    "add",
                    "--detach",
                    linked.to_str().unwrap(),
                    &second,
                ],
                None,
            );
            assert_eq!(inspect_git_commit(&linked, &second).unwrap().tree_id, tree);
            for id in [
                "HEAD",
                "HEAD~1",
                "--help",
                "--batch-command",
                "aabb",
                "refs/heads/main",
                "$(touch marker)",
            ] {
                assert!(inspect_git_commit(root.path(), id).is_err());
            }
            assert!(inspect_git_commit(root.path(), &"f".repeat(second.len())).is_err());
        }
    }
    #[test]
    fn git_ignores_replacement_objects_and_inherited_configuration_overrides() {
        let (root, tree, first) = repo("sha1");
        let second = commit(root.path(), &tree, &[&first], "replacement");
        git(root.path(), &["replace", &first, &second], None);
        assert!(
            inspect_git_commit(root.path(), &first)
                .unwrap()
                .parent_ids
                .is_empty()
        );
        let mut poisoned = Command::new("git");
        poisoned
            .env("GIT_DIR", "/nonexistent-private-repo")
            .env("GIT_WORK_TREE", "/nonexistent-private-tree")
            .env("GIT_CONFIG_COUNT", "1")
            .env("GIT_CONFIG_KEY_0", "core.repositoryformatversion")
            .env("GIT_CONFIG_VALUE_0", "9999")
            .env("GIT_OBJECT_DIRECTORY", "/nonexistent-private-objects");
        let output = configure_git(poisoned, root.path())
            .args(["cat-file", "-t", &first])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"commit\n");
        let subdirectory = root.path().join("not-a-selected-repository");
        std::fs::create_dir(&subdirectory).unwrap();
        assert!(
            inspect_git_commit(&subdirectory, &first).is_err(),
            "must not discover a parent repository"
        );
    }
    #[test]
    fn oversized_commit_and_bad_evidence_are_refused() {
        let (root, tree, first) = repo("sha1");
        let big = commit(root.path(), &tree, &[], &"x".repeat(MAX_COMMIT_BYTES));
        assert_eq!(
            inspect_git_commit(root.path(), &big)
                .unwrap_err()
                .to_string(),
            "insights-git-commit-too-large"
        );
        let mut evidence = inspect_git_commit(root.path(), &first).unwrap();
        evidence.parent_ids.push("bad".into());
        assert!(evidence.validate().is_err());
        assert!(evidence.identity_digest().is_err());
    }
    #[test]
    fn report_import_is_strict_hash_bound_and_not_an_execution_claim() {
        let bytes = serde_json::to_vec(&report()).unwrap();
        let evidence = parse_test_report(&bytes).unwrap();
        assert_eq!(evidence.provenance, TestEvidenceProvenance::ImportedReport);
        assert_eq!(
            (evidence.passed, evidence.failed, evidence.skipped),
            (3, 1, 2)
        );
        assert_eq!(evidence.artifact_digest, digest(&bytes));
        assert!(evidence.commit_id.is_none());
        assert_eq!(
            evidence.identity_digest().unwrap(),
            parse_test_report(&bytes)
                .unwrap()
                .identity_digest()
                .unwrap()
        );
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("report.json");
        std::fs::write(&file, &bytes).unwrap();
        assert_eq!(
            import_test_report(&file).unwrap().artifact_digest,
            evidence.artifact_digest
        );
        let wrapped = OutcomeEvidence::TestReport(evidence);
        let restored: OutcomeEvidence =
            serde_json::from_slice(&serde_json::to_vec(&wrapped).unwrap()).unwrap();
        assert_eq!(
            wrapped.identity_digest().unwrap(),
            restored.identity_digest().unwrap()
        );
        assert!(parse_test_report(&vec![b' '; MAX_TEST_REPORT_BYTES + 1]).is_err());
        assert!(parse_test_report(&[0xff]).is_err());
        for (key, value) in [
            ("runner", serde_json::json!("cargo test --token=private")),
            ("schema_version", serde_json::json!(2)),
            ("passed", serde_json::json!(-1)),
            ("passed", serde_json::json!(u64::MAX)),
            ("commit_id", serde_json::json!("HEAD")),
            ("observed_at", serde_json::json!("2099-01-01T00:00:00Z")),
            ("observed_at", serde_json::json!("1960-01-01T00:00:00Z")),
            ("command", serde_json::json!("touch private-marker")),
            ("provenance", serde_json::json!("independently_verified")),
        ] {
            let mut invalid = report();
            invalid[key] = value;
            let error = parse_test_report(&serde_json::to_vec(&invalid).unwrap())
                .unwrap_err()
                .to_string();
            assert!(!error.contains("private"));
        }
        let mut missing = report();
        missing.as_object_mut().unwrap().remove("passed");
        assert!(parse_test_report(&serde_json::to_vec(&missing).unwrap()).is_err());
        let mut known_zero = report();
        known_zero["passed"] = serde_json::json!(0);
        known_zero["failed"] = serde_json::json!(0);
        known_zero["skipped"] = serde_json::json!(0);
        known_zero["commit_id"] = serde_json::json!("a".repeat(64));
        assert_eq!(
            parse_test_report(&serde_json::to_vec(&known_zero).unwrap())
                .unwrap()
                .passed,
            0
        );
    }
    #[cfg(unix)]
    #[test]
    fn report_file_reader_refuses_symlinks_and_special_files() {
        use std::os::unix::fs::symlink;
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("report.json");
        std::fs::write(&file, serde_json::to_vec(&report()).unwrap()).unwrap();
        let link = root.path().join("link.json");
        symlink(&file, &link).unwrap();
        assert!(import_test_report(&link).is_err());
        assert!(import_test_report(root.path()).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn git_reader_deadline_covers_stdout_held_by_a_descendant() {
        let mut command = Command::new("/bin/sh");
        // Fixed synthetic helper only. Production always invokes Git directly.
        command
            .args(["-c", "sleep 2 & exit 0"])
            .stdin(Stdio::null())
            .stderr(Stdio::null());
        let start = Instant::now();
        assert!(read_command(command, 32, Duration::from_millis(100)).is_err());
        assert!(start.elapsed() < Duration::from_secs(1));
    }
}
