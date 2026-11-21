use super::{MAX_METADATA_BYTES, SkillSource, hash_bytes, policy::ReadPolicy};
use std::collections::BTreeMap;
use std::path::{Component, Path};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Normalize only a GitHub remote or a bare GitHub owner/repository reference.
/// Credentials, query strings, additional URL paths and traversal are refused.
pub fn normalize_repository(value: &str) -> Option<String> {
    let value = value.trim();
    let repo = value
        .strip_prefix("https://github.com/")
        .or_else(|| value.strip_prefix("http://github.com/"))
        .or_else(|| value.strip_prefix("git@github.com:"))
        .or_else(|| value.strip_prefix("ssh://git@github.com/"))
        .unwrap_or(value);
    let repo = repo.strip_suffix(".git").unwrap_or(repo);
    let parts: Vec<_> = repo.split('/').collect();
    if parts.len() != 2
        || parts.iter().any(|p| {
            p.is_empty()
                || *p == "."
                || *p == ".."
                || !p
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        })
    {
        return None;
    }
    Some(repo.to_ascii_lowercase())
}

pub fn normalize_skill_path(value: &str) -> Option<String> {
    if value.contains(['\\', '\0', '?', '#'])
        || value.len() > 2048
        || value
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
        || value.chars().any(char::is_control)
    {
        return None;
    }
    let path = Path::new(value);
    if path
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
        || path.file_name()? != "SKILL.md"
    {
        return None;
    }
    Some(value.to_string())
}

/// Translate an already resolved native relative path into repository syntax.
/// Do not apply this conversion to untrusted manifest/lockfile strings: those
/// must pass the strict slash-only normalizer above without reinterpretation.
pub(super) fn native_skill_path(path: &Path) -> Option<String> {
    let parts: Option<Vec<_>> = path
        .components()
        .map(|component| match component {
            Component::Normal(name) => name.to_str(),
            _ => None,
        })
        .collect();
    normalize_skill_path(&parts?.join("/"))
}

pub(super) fn lock_sources(
    home: &Path,
    projects: &[std::path::PathBuf],
    deadline: Instant,
    policy: &ReadPolicy,
) -> (BTreeMap<std::path::PathBuf, SkillSource>, bool) {
    let mut sources = BTreeMap::new();
    let mut remaining = super::MAX_SKILLS;
    let mut partial = false;
    for base in std::iter::once(home).chain(projects.iter().map(|p| p.as_path())) {
        for relative in [".agents/.skill-lock.json", "skills-lock.json"] {
            if Instant::now() >= deadline {
                return (sources, true);
            }
            let bytes = match policy.read(&base.join(relative), MAX_METADATA_BYTES) {
                Ok(bytes) => bytes,
                Err(error) => {
                    partial |= error.kind() != std::io::ErrorKind::NotFound;
                    continue;
                }
            };
            let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let Some(skills) = value.get("skills").and_then(|s| s.as_object()) else {
                continue;
            };
            for (directory, entry) in skills {
                if remaining == 0 || Instant::now() >= deadline {
                    return (sources, true);
                }
                remaining -= 1;
                if Path::new(directory).components().count() != 1
                    || directory == "."
                    || directory == ".."
                {
                    continue;
                }
                if entry.get("sourceType").and_then(|s| s.as_str()) != Some("github") {
                    continue;
                }
                let Some(repository) = entry
                    .get("source")
                    .and_then(|s| s.as_str())
                    .and_then(normalize_repository)
                else {
                    continue;
                };
                if entry
                    .get("sourceUrl")
                    .and_then(|value| value.as_str())
                    .is_some_and(|url| normalize_repository(url).as_ref() != Some(&repository))
                {
                    continue;
                }
                let Some(path) = entry
                    .get("skillPath")
                    .and_then(|s| s.as_str())
                    .and_then(normalize_skill_path)
                else {
                    continue;
                };
                // This lock maps an installation slot, not the skill's frontmatter name.
                // A global alias can resolve to that exact same canonical slot.
                if let Ok(local) = policy.canonical_allowed(
                    &base.join(".agents/skills").join(directory).join("SKILL.md"),
                ) {
                    sources.insert(
                        local,
                        SkillSource {
                            repository,
                            path,
                            revision: None,
                            baseline_sha256: None,
                        },
                    );
                }
            }
        }
    }
    (sources, partial)
}

/// Git is used only for fixed read-only builtins on one tracked file. No shell,
/// hooks, network, recursive file listing or configured fsmonitor is invoked.
/// A revision is attached only when the tracked blob exactly matches local bytes.
pub(super) fn git_source(path: &Path, bytes: &[u8], budget: &mut GitBudget) -> Option<SkillSource> {
    if budget.timed_out {
        return None;
    }
    let mut current = path.parent()?;
    for _ in 0..12 {
        if current.join(".git").exists() {
            let relative = native_skill_path(path.strip_prefix(current).ok()?)?;
            git(
                current,
                &["ls-files", "--error-unmatch", "--", &relative],
                budget,
            )?;
            let repository = normalize_repository(
                git(
                    current,
                    &["config", "--no-includes", "--get", "remote.origin.url"],
                    budget,
                )?
                .trim(),
            )?;
            let revision = git(current, &["rev-parse", "--verify", "HEAD"], budget)?;
            let revision = revision.trim();
            if !matches!(revision.len(), 40 | 64)
                || !revision.chars().all(|c| c.is_ascii_hexdigit())
            {
                return None;
            }
            let blob = git_bytes(current, &["show", &format!("HEAD:{relative}")], budget)?;
            return Some(SkillSource {
                repository,
                path: relative,
                revision: (hash_bytes(bytes) == hash_bytes(&blob)).then(|| revision.to_string()),
                baseline_sha256: Some(hash_bytes(&blob)),
            });
        }
        current = current.parent()?;
    }
    None
}

/// How long one read-only git command may take, as shipped, before the scan
/// gives up on that skill's provenance.
///
/// Measured 2026-11-20 with the four commands `git_source` runs, from a
/// fresh process each time: on a Mac, 10-19 ms. On a 4-core Windows VM with
/// Defender real-time scanning on, 60-250 ms idle (p50 about 150 ms) and
/// 95-566 ms with every core busy (p50 about 260 ms, p99 427 ms). The old
/// 250 ms bound failed about half the calls on that loaded machine and left
/// those skills without provenance, silently. Three seconds is about five
/// times the worst loaded call seen. A hung git still costs at most this
/// long, once per scan: `GitBudget` stops calling git after the first
/// timeout, and never lets a call outlive the scan's own deadline.
const SHIPPED_GIT_DEADLINE: Duration = Duration::from_secs(3);

/// The bound `git_source` uses. Tests wait longer: what they check is the
/// provenance found, not this bound, and a loaded CI runner can be slower
/// than any contributor's machine. The tests of the bound itself pass
/// `SHIPPED_GIT_DEADLINE` explicitly.
#[cfg(not(test))]
const GIT_DEADLINE: Duration = SHIPPED_GIT_DEADLINE;
#[cfg(test)]
const GIT_DEADLINE: Duration = Duration::from_secs(30);

/// The git work one skill scan may do. Each call gets `GIT_DEADLINE`, but
/// never past the scan's own deadline; after the first call that runs out of
/// time, the rest of the scan makes no git calls at all, so a hung or
/// pathologically slow git costs one per-call bound, not one per skill.
pub(super) struct GitBudget {
    scan_deadline: Instant,
    timed_out: bool,
}

impl GitBudget {
    pub(super) fn new(scan_deadline: Instant) -> Self {
        Self {
            scan_deadline,
            timed_out: false,
        }
    }
}

/// How one git command ended.
#[derive(Debug)]
enum GitRun {
    Output(Vec<u8>),
    /// Exited unsuccessfully, could not start, or wrote more than the cap.
    Failed,
    /// Still running at the deadline, and killed.
    TimedOut,
}

fn git(root: &Path, args: &[&str], budget: &mut GitBudget) -> Option<String> {
    String::from_utf8(git_bytes(root, args, budget)?).ok()
}

fn git_bytes(root: &Path, args: &[&str], budget: &mut GitBudget) -> Option<Vec<u8>> {
    let now = Instant::now();
    if budget.timed_out || now >= budget.scan_deadline {
        budget.timed_out = true;
        return None;
    }
    let deadline = (now + GIT_DEADLINE).min(budget.scan_deadline);
    match run_git(Command::new("git"), root, args, deadline) {
        GitRun::Output(bytes) => Some(bytes),
        GitRun::Failed => None,
        GitRun::TimedOut => {
            budget.timed_out = true;
            None
        }
    }
}

/// Runs `command` (git, or a test's stand-in for it) with the fixed
/// read-only flags, `-C root` and `args`, killing it at `deadline`.
fn run_git(mut command: Command, root: &Path, args: &[&str], deadline: Instant) -> GitRun {
    use std::io::Read;
    let spawned = command
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .env_remove("GIT_CONFIG_COUNT")
        .env_remove("GIT_CONFIG_PARAMETERS")
        .env_remove("GIT_COMMON_DIR")
        .env_remove("GIT_ALTERNATE_OBJECT_DIRECTORIES")
        .env_remove("GIT_OBJECT_DIRECTORY")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .env("GIT_OPTIONAL_LOCKS", "0")
        .args([
            "--literal-pathspecs",
            "--no-replace-objects",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.hooksPath=",
            "-C",
        ])
        .arg(root)
        .args(args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .stdout(Stdio::piped())
        .spawn();
    let Ok(mut child) = spawned else {
        return GitRun::Failed;
    };
    // Reading concurrently prevents a pipe-sized valid SKILL.md from blocking
    // the child before it exits. The reader itself keeps at most the byte cap.
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return GitRun::Failed;
    };
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        stdout
            .take(super::MAX_SKILL_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        Some(bytes)
    });
    let finished = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break Some(false);
            }
        }
    };
    let bytes = reader.join().ok().flatten();
    match (finished, bytes) {
        (None, _) => GitRun::TimedOut,
        (Some(true), Some(bytes)) if bytes.len() as u64 <= super::MAX_SKILL_BYTES => {
            GitRun::Output(bytes)
        }
        _ => GitRun::Failed,
    }
}

/// A package repository is not proof that the package is rooted at the repo
/// root. Only explicit directory metadata or a verified root package mapping
/// establishes the repository-relative skill path.
pub(super) fn plugin_source(path: &Path, policy: &ReadPolicy) -> (Option<SkillSource>, bool) {
    let Some(skill_root) = path.parent().and_then(Path::parent) else {
        return (None, false);
    };
    if skill_root.file_name().is_none_or(|name| name != "skills") {
        return (None, false);
    }
    let Some(package) = skill_root.parent() else {
        return (None, false);
    };
    let Ok(canonical_package) = policy.canonical_allowed(package) else {
        return (None, true);
    };
    let Ok(canonical_document) = policy.canonical_allowed(path) else {
        return (None, true);
    };
    // An external linked skill cannot borrow the enclosing plugin's identity.
    let Ok(relative) = canonical_document.strip_prefix(&canonical_package) else {
        return (None, false);
    };
    let Some(relative) = native_skill_path(relative) else {
        return (None, false);
    };
    let mut partial = false;
    for manifest in [".codex-plugin/plugin.json", ".claude-plugin/plugin.json"] {
        let bytes = match policy.read(&package.join(manifest), MAX_METADATA_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                partial |= error.kind() != std::io::ErrorKind::NotFound;
                continue;
            }
        };
        let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
            continue;
        };
        let Some((repository, directory)) = manifest_repository(&value) else {
            continue;
        };
        let path = if directory.is_empty() {
            relative.clone()
        } else {
            format!("{directory}/{relative}")
        };
        let Some(path) = normalize_skill_path(&path) else {
            continue;
        };
        return (
            Some(SkillSource {
                repository,
                path,
                revision: None,
                baseline_sha256: None,
            }),
            partial,
        );
    }
    (None, partial)
}

fn manifest_repository(value: &serde_json::Value) -> Option<(String, String)> {
    let metadata = value.get("repository")?;
    if let Some(url) = metadata.as_str() {
        let repository = normalize_repository(url)?;
        // Verified public package root. Marketplace and display names alone
        // never establish a path for an arbitrary monorepo package.
        return (repository == "obra/superpowers" && value.get("name")?.as_str()? == "superpowers")
            .then_some((repository, String::new()));
    }
    let repository = normalize_repository(metadata.get("url")?.as_str()?)?;
    let directory = metadata.get("directory")?.as_str()?;
    let directory = directory
        .strip_prefix("./")
        .unwrap_or(directory)
        .trim_end_matches('/');
    if directory.is_empty() || directory == "." {
        return Some((repository, String::new()));
    }
    // Reuse the strict file-path normalizer to validate a directory prefix.
    normalize_skill_path(&format!("{directory}/SKILL.md"))?;
    Some((repository, directory.to_string()))
}

#[cfg(test)]
mod git_deadline_tests {
    use super::*;
    use std::fs;

    /// A stand-in for git: `/bin/sh` running `script`, then the real git.
    /// Run through `sh` rather than exec'd, so a write-then-exec race with
    /// another test's fork (ETXTBSY) cannot touch it.
    #[cfg(unix)]
    fn slow_git(dir: &Path, script: &str) -> Command {
        let path = dir.join("slow-git.sh");
        fs::write(&path, script).unwrap();
        let mut command = Command::new("/bin/sh");
        command.arg(path);
        command
    }

    /// A git that takes longer than the old 250 ms bound but well inside the
    /// shipped one, as on a loaded Windows machine with antivirus scanning
    /// git, still answers.
    #[cfg(unix)]
    #[test]
    fn slow_git_inside_the_shipped_deadline_answers() {
        let dir = tempfile::tempdir().unwrap();
        let git = slow_git(dir.path(), "sleep 0.6\nexec git \"$@\"\n");
        let run = run_git(
            git,
            dir.path(),
            &["--version"],
            Instant::now() + SHIPPED_GIT_DEADLINE,
        );
        let GitRun::Output(bytes) = run else {
            panic!("a 0.6 s git should finish inside the shipped deadline, got {run:?}");
        };
        assert!(bytes.starts_with(b"git version"), "{bytes:?}");
    }

    /// A git that would answer, but only after the shipped deadline, is
    /// killed at the deadline rather than waited for.
    #[cfg(unix)]
    #[test]
    fn git_past_the_shipped_deadline_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let late = SHIPPED_GIT_DEADLINE + Duration::from_secs(2);
        // `sleep` writes nothing, so it gets no handle on the stdout pipe and
        // killing `sh` closes it.
        let git = slow_git(
            dir.path(),
            &format!(
                "sleep {}.{:03} >/dev/null\nexec git \"$@\"\n",
                late.as_secs(),
                late.subsec_millis()
            ),
        );
        let started = Instant::now();
        let run = run_git(
            git,
            dir.path(),
            &["--version"],
            started + SHIPPED_GIT_DEADLINE,
        );
        let elapsed = started.elapsed();
        assert!(matches!(run, GitRun::TimedOut), "{run:?}");
        assert!(
            elapsed >= SHIPPED_GIT_DEADLINE && elapsed < late,
            "killed after {elapsed:?}; deadline {SHIPPED_GIT_DEADLINE:?}, git done at {late:?}"
        );
    }

    fn tracked_skill() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let repo = dir.path().canonicalize().unwrap().join("skill");
        fs::create_dir_all(&repo).unwrap();
        let git = |args: &[&str]| {
            assert!(
                Command::new("git")
                    .arg("-C")
                    .arg(&repo)
                    .args(args)
                    .output()
                    .unwrap()
                    .status
                    .success(),
                "git {args:?}"
            )
        };
        git(&["init", "-q"]);
        git(&[
            "remote",
            "add",
            "origin",
            "https://github.com/publisher/repo.git",
        ]);
        fs::write(repo.join("SKILL.md"), b"# skill\n").unwrap();
        git(&["add", "SKILL.md"]);
        git(&[
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=Test",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "-qm",
            "Initial",
        ]);
        let path = repo.join("SKILL.md");
        (dir, path)
    }

    /// After one git call runs out of time, the rest of the scan makes none:
    /// a hung git costs one per-call bound per scan, not one per skill.
    #[test]
    fn a_timeout_ends_git_for_the_rest_of_the_scan() {
        let (_dir, path) = tracked_skill();
        let bytes = fs::read(&path).unwrap();
        let mut fresh = GitBudget::new(Instant::now() + Duration::from_secs(60));
        assert!(git_source(&path, &bytes, &mut fresh).is_some());
        assert!(!fresh.timed_out);

        let mut spent = GitBudget::new(Instant::now() + Duration::from_secs(60));
        spent.timed_out = true;
        assert!(git_source(&path, &bytes, &mut spent).is_none());
    }

    /// No git call outlives the scan's own deadline.
    #[test]
    fn no_git_call_past_the_scan_deadline() {
        let (_dir, path) = tracked_skill();
        let bytes = fs::read(&path).unwrap();
        let mut budget = GitBudget::new(Instant::now());
        assert!(git_source(&path, &bytes, &mut budget).is_none());
        assert!(budget.timed_out);
    }
}
