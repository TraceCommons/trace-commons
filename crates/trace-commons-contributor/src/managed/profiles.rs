//! Native tools own their credentials. We choose an isolated root per account.

use super::{AccountId, AccountView, AuthState, ConnectionKind, ManagedError, ToolId};
use crate::config::ConfigStore;
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub enum NativeAction {
    Version,
    Run,
    Login,
    Status,
    Logout,
}

pub struct NativeProfiles {
    root: PathBuf,
}

const AUTH_ENV: &[&str] = &[
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "ANTHROPIC_BASE_URL",
    "ANTHROPIC_PROFILE",
    "ANTHROPIC_FEDERATION_RULE_ID",
    "ANTHROPIC_ORGANIZATION_ID",
    "CLAUDE_CODE_OAUTH_TOKEN",
    "CLAUDE_CODE_USE_BEDROCK",
    "CLAUDE_CODE_USE_VERTEX",
    "CLAUDE_CODE_USE_FOUNDRY",
    "NEARAI_API_KEY",
    "NEAR_AI_SESSION_API_KEY",
    "OPENAI_API_KEY",
    "OPENAI_BASE_URL",
    "CODEX_API_KEY",
    "CODEX_ACCESS_TOKEN",
    "CODEX_HOME",
    "CLAUDE_CONFIG_DIR",
    "CLAUDE_SECURESTORAGE_CONFIG_DIR",
];

impl NativeProfiles {
    pub fn new(store: &ConfigStore) -> Self {
        Self {
            root: store.dir().join("managed-profiles"),
        }
    }

    /// The profile roots that exist now, one per managed account, for the
    /// Insights counter pass to read sessions from (`daemon::watcher`).
    /// Creates nothing, unlike `profile_root`: a missing folder is none,
    /// and an entry that is not a real directory named by an account ID (a
    /// symlink included) is passed over.
    pub fn existing_roots(&self) -> Vec<PathBuf> {
        let real_dir = |path: &Path| {
            std::fs::symlink_metadata(path)
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        };
        if !real_dir(&self.root) {
            return Vec::new();
        }
        let Ok(entries) = std::fs::read_dir(&self.root) else {
            return Vec::new();
        };
        let mut roots: Vec<PathBuf> = entries
            .flatten()
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|name| uuid::Uuid::parse_str(name).is_ok())
            })
            .map(|entry| entry.path())
            .filter(|path| real_dir(path))
            .collect();
        roots.sort();
        roots
    }

    /// The account's profile directory when it is already there, creating
    /// nothing: for checks that must not leave a directory behind.
    pub fn existing_profile_root(&self, id: AccountId) -> Option<PathBuf> {
        let real_dir = |path: &Path| {
            std::fs::symlink_metadata(path)
                .is_ok_and(|meta| meta.is_dir() && !meta.file_type().is_symlink())
        };
        let path = self.root.join(id.to_string());
        (real_dir(&self.root) && real_dir(&path)).then_some(path)
    }

    pub fn profile_root(&self, id: AccountId) -> Result<PathBuf, ManagedError> {
        private_directory(&self.root)?;
        let path = self.root.join(id.to_string());
        private_directory(&path)?;
        Ok(path)
    }

    pub fn command(
        &self,
        account: &AccountView,
        native: &Path,
        action: NativeAction,
        cwd: &Path,
    ) -> Result<Command, ManagedError> {
        if !native.is_absolute() || !native.is_file() || !cwd.is_dir() {
            return Err(ManagedError::InvalidRequest);
        }
        let root = self.profile_root(account.id)?;
        if matches!(action, NativeAction::Run) {
            check_project_config(account.tool, account.connection, cwd)?;
        }
        let mut command = Command::new(native);
        command.current_dir(cwd);
        for key in AUTH_ENV {
            command.env_remove(key);
        }
        match account.tool {
            ToolId::Claude => {
                command.env("CLAUDE_CONFIG_DIR", &root);
                match action {
                    NativeAction::Version => {
                        command.arg("--version");
                    }
                    NativeAction::Run => {}
                    NativeAction::Login => {
                        command.args(["auth", "login", "--claudeai"]);
                    }
                    NativeAction::Status => {
                        command.args(["auth", "status", "--json"]);
                    }
                    NativeAction::Logout => {
                        command.args(["auth", "logout"]);
                    }
                }
            }
            ToolId::Opencode => {
                super::opencode::isolate(&mut command, &root)?;
                match action {
                    NativeAction::Version => {
                        command.arg("--version");
                    }
                    NativeAction::Run => {
                        super::opencode::Version::from_native(
                            self.native_version(account, native)?,
                        )?
                        .interactive(&mut command);
                    }
                    _ => return Err(ManagedError::InvalidRequest),
                }
            }
            ToolId::Codex => {
                command.env("CODEX_HOME", &root);
                match action {
                    NativeAction::Version => {
                        command.arg("--version");
                    }
                    NativeAction::Run => {}
                    NativeAction::Login => {
                        command.arg("login");
                    }
                    NativeAction::Status => {
                        command.args(["login", "status"]);
                    }
                    NativeAction::Logout => {
                        command.arg("logout");
                    }
                }
            }
        }
        if matches!(action, NativeAction::Login)
            && account.connection != ConnectionKind::Subscription
        {
            return Err(ManagedError::InvalidRequest);
        }
        Ok(command)
    }

    pub fn remove(&self, account: &AccountView, native: &Path) -> Result<(), ManagedError> {
        self.verify_version(account, native)?;
        let root = self.profile_root(account.id)?;
        let result = bounded_output(self.command(account, native, NativeAction::Logout, &root)?)?;
        if !result.status.success() {
            return Err(ManagedError::StorageUnavailable);
        }
        self.remove_files(account.id)
    }

    pub fn remove_files(&self, id: AccountId) -> Result<(), ManagedError> {
        let root = self.profile_root(id)?;
        std::fs::remove_dir_all(root).map_err(|_| ManagedError::StorageUnavailable)
    }

    pub fn native_version(
        &self,
        account: &AccountView,
        native: &Path,
    ) -> Result<Option<(u32, u32, u32)>, ManagedError> {
        let root = self.profile_root(account.id)?;
        let version =
            bounded_output(self.command(account, native, NativeAction::Version, &root)?)?;
        if !version.status.success() {
            return Err(ManagedError::UnsupportedVersion);
        }
        let version = String::from_utf8_lossy(&version.stdout);
        let numeric = version.split_whitespace().find_map(|part| {
            let numbers: Vec<_> = part
                .strip_prefix('v')
                .unwrap_or(part)
                .split('.')
                .map(str::parse::<u32>)
                .collect::<Result<_, _>>()
                .ok()?;
            if numbers.len() == 3 {
                Some((numbers[0], numbers[1], numbers[2]))
            } else {
                None
            }
        });
        Ok(numeric)
    }

    pub fn verify_version(&self, account: &AccountView, native: &Path) -> Result<(), ManagedError> {
        let numeric = self.native_version(account, native)?;
        let supported = match account.tool {
            ToolId::Claude => numeric.is_some_and(|v| v.0 == 2 && v >= (2, 1, 289)),
            ToolId::Codex => numeric.is_some_and(|v| v.0 == 0 && v >= (0, 160, 0)),
            // Each major has its own CLI/configuration contract. Import/export
            // validation still checks the selected implementation before launch.
            ToolId::Opencode => numeric.is_some_and(|v| matches!(v.0, 1 | 2)),
        };
        if !supported {
            return Err(ManagedError::UnsupportedVersion);
        }
        Ok(())
    }

    pub fn inspect(&self, account: &AccountView, native: &Path) -> Result<AuthState, ManagedError> {
        self.verify_version(account, native)?;
        let root = self.profile_root(account.id)?;
        let output = bounded_output(self.command(account, native, NativeAction::Status, &root)?)?;
        if !output.status.success() {
            return Ok(AuthState::SignInRequired);
        }
        let ready = match account.tool {
            ToolId::Opencode => return Err(ManagedError::InvalidRequest),
            ToolId::Claude => {
                let value: serde_json::Value = serde_json::from_slice(&output.stdout)
                    .map_err(|_| ManagedError::UnsupportedVersion)?;
                value["loggedIn"] == true && value["authMethod"] == "claude.ai"
            }
            ToolId::Codex => output
                .stdout
                .windows(b"Logged in using ChatGPT".len())
                .chain(output.stderr.windows(b"Logged in using ChatGPT".len()))
                .any(|s| s == b"Logged in using ChatGPT"),
        };
        Ok(if ready {
            AuthState::Ready
        } else {
            AuthState::SignInRequired
        })
    }
}

/// Status helpers may wait for native OS authentication. Bound the wait and
/// keep their identity-bearing output off the terminal and out of logs.
pub(crate) fn bounded_output(command: Command) -> Result<std::process::Output, ManagedError> {
    bounded_output_limit(command, 65536)
}

/// A scratch file that holds a helper's output until it is read, gone once
/// its last handle closes (see `local_files::create_capture_file`). A
/// failure is logged by its error kind and OS code only, never a path.
fn capture_file() -> Result<std::fs::File, ManagedError> {
    trace_commons_contributor_platform::local_files::create_capture_file(&std::env::temp_dir())
        .map_err(|error| {
            tracing::warn!(
                kind = ?error.kind(),
                os_error = ?error.raw_os_error(),
                "managed output capture file could not be created"
            );
            ManagedError::StorageUnavailable
        })
}

pub(crate) fn bounded_output_limit(
    mut command: Command,
    limit: u64,
) -> Result<std::process::Output, ManagedError> {
    use std::io::{Read, Seek};
    use std::process::Stdio;
    let mut stdout = capture_file()?;
    let mut stderr = capture_file()?;
    command
        .stdin(Stdio::null())
        .stdout(
            stdout
                .try_clone()
                .map_err(|_| ManagedError::StorageUnavailable)?,
        )
        .stderr(
            stderr
                .try_clone()
                .map_err(|_| ManagedError::StorageUnavailable)?,
        );
    let mut child = command
        .spawn()
        .map_err(|_| ManagedError::UnsupportedVersion)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let status = loop {
        if stdout.metadata().map(|m| m.len() > limit).unwrap_or(true)
            || stderr.metadata().map(|m| m.len() > 65536).unwrap_or(true)
        {
            let _ = child.kill();
            let _ = child.wait();
            return Err(ManagedError::UnsupportedVersion);
        }
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(25))
            }
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ManagedError::LaunchUnknown);
            }
        }
    };
    let read = |file: &mut std::fs::File, limit: u64| -> Result<Vec<u8>, ManagedError> {
        file.rewind()
            .map_err(|_| ManagedError::StorageUnavailable)?;
        let mut bytes = vec![];
        file.take(limit + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        if bytes.len() as u64 > limit {
            return Err(ManagedError::UnsupportedVersion);
        }
        Ok(bytes)
    };
    Ok(std::process::Output {
        status,
        stdout: read(&mut stdout, limit)?,
        stderr: read(&mut stderr, 65536)?,
    })
}

pub(crate) fn private_directory(path: &Path) -> Result<(), ManagedError> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(ManagedError::StorageUnavailable);
        }
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => create_private_directory(path)?,
        Err(_) => return Err(ManagedError::StorageUnavailable),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ManagedError::StorageUnavailable)?;
    }
    Ok(())
}

/// Create the file `path` for writing, owner-only, refusing one that is
/// already there (a link included).
pub(crate) fn create_private_file(path: &Path) -> std::io::Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// The whole of `path` when it is a regular file, never a link, of at most
/// `limit` bytes, however it grows while being read.
pub(crate) fn read_regular_file(path: &Path, limit: u64) -> Option<Vec<u8>> {
    use std::io::Read;
    let metadata = std::fs::symlink_metadata(path).ok()?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > limit {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(limit + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= limit).then_some(bytes)
}

/// Create `path` owner-only. Two launches can race here; the loser's
/// create fails with AlreadyExists, and the directory the winner made is
/// accepted only as a real directory, never a symlink or a file.
fn create_private_directory(path: &Path) -> Result<(), ManagedError> {
    let builder = std::fs::DirBuilder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = builder;
        builder.mode(0o700);
        builder
    };
    match builder.create(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            match std::fs::symlink_metadata(path) {
                Ok(metadata) if metadata.is_dir() && !metadata.file_type().is_symlink() => Ok(()),
                _ => Err(ManagedError::StorageUnavailable),
            }
        }
        Err(_) => Err(ManagedError::StorageUnavailable),
    }
}

/// Whether `name`, a Claude Code environment variable, chooses a model:
/// the main model, the small, fast model, a family alias (any
/// `ANTHROPIC_DEFAULT_*_MODEL`), the custom picker entry, or one of the
/// `CLAUDE_CODE_*_MODEL` variables that set a subagent's, a workflow's or a
/// classifier's model. A display name (`*_NAME`) chooses nothing.
fn names_a_claude_model(name: &str) -> bool {
    matches!(
        name,
        "ANTHROPIC_MODEL" | "ANTHROPIC_SMALL_FAST_MODEL" | "ANTHROPIC_CUSTOM_MODEL_OPTION"
    ) || (name.starts_with("ANTHROPIC_DEFAULT_") && name.ends_with("_MODEL"))
        || (name.starts_with("CLAUDE_CODE_") && name.ends_with("_MODEL"))
}

/// Claude Code's model aliases, which it maps onto the session's own
/// family variables, so a subagent naming one stays on the session's
/// models. Matched before any `[...]` suffix (`opus[1m]`).
const CLAUDE_MODEL_ALIASES: &[&str] = &[
    "inherit", "default", "sonnet", "opus", "haiku", "fable", "opusplan", "best",
];

/// Whether `value`, a model setting, is one of [`CLAUDE_MODEL_ALIASES`],
/// optionally with one `[...]` suffix (`opus[1m]`).
fn is_claude_model_alias(value: &str) -> bool {
    let (alias, rest) = value.split_at(value.find('[').unwrap_or(value.len()));
    let suffix_ok =
        rest.is_empty() || (rest.ends_with(']') && !rest[1..rest.len() - 1].contains(['[', ']']));
    suffix_ok && CLAUDE_MODEL_ALIASES.contains(&alias.to_ascii_lowercase().as_str())
}

/// Whether a project markdown file's frontmatter may give Claude Code a
/// `model` other than an alias.
///
/// Claude Code parses the block between the opening `---` and the next one
/// as YAML, so a line-prefix match is not enough: a value on the next line,
/// a space before the colon, a quoted or complex key, a flow mapping, an
/// anchor or a tag can all set `model`. Without a YAML parser here, this
/// reads the block conservatively: the only accepted form is a top-level
/// line `model: <alias>` (optionally quoted, optionally with a trailing
/// comment) that no indented line continues. Any other top-level line that
/// is not a plain `key: value` counts as naming a model, so a form this
/// cannot read is refused rather than guessed at. Indented lines belong to
/// the key above them and cannot set a top-level `model`.
fn frontmatter_names_a_model(text: &str) -> bool {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut lines = text.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return false;
    }
    let block: Vec<&str> = lines.take_while(|line| line.trim_end() != "---").collect();
    let indented = |line: &str| line.starts_with([' ', '\t']);
    for (index, line) in block.iter().enumerate() {
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') || indented(line) {
            continue;
        }
        let Some((key, value)) = line.split_once(':') else {
            return true;
        };
        if key.is_empty()
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
        {
            return true;
        }
        if !key.eq_ignore_ascii_case("model") {
            continue;
        }
        let value = match value.find(" #").or_else(|| value.find("\t#")) {
            Some(comment) => &value[..comment],
            None => value,
        }
        .trim();
        let unquoted = [('"', '"'), ('\'', '\'')]
            .iter()
            .find_map(|(open, close)| {
                value
                    .strip_prefix(*open)
                    .and_then(|v| v.strip_suffix(*close))
            })
            .unwrap_or(value);
        let continued = block
            .get(index + 1)
            .is_some_and(|next| indented(next) && !next.trim().is_empty());
        if continued || !is_claude_model_alias(unquoted) {
            return true;
        }
    }
    false
}

/// Which project markdown Claude Code reads a `model` from, under a
/// folder's `.claude`: subagents, slash commands and routines at any depth,
/// and each skill's `SKILL.md`.
#[derive(Clone, Copy)]
enum ProjectMarkdown {
    Agents,
    Commands,
    Routines,
    Skills,
}

impl ProjectMarkdown {
    const ALL: [ProjectMarkdown; 4] = [Self::Agents, Self::Commands, Self::Routines, Self::Skills];

    fn folder(self) -> &'static str {
        match self {
            Self::Agents => "agents",
            Self::Commands => "commands",
            Self::Routines => "routines",
            Self::Skills => "skills",
        }
    }
}

/// Whether the project markdown of `kind` under `dir` names a model by id
/// rather than by alias ([`frontmatter_names_a_model`]). A file that cannot
/// be read whole, or a tree past its limits, counts as one: nothing is
/// guessed.
fn markdown_names_a_model(dir: &Path, kind: ProjectMarkdown) -> Result<bool, ()> {
    const MAX_FILES: usize = 1000;
    const MAX_DEPTH: usize = 8;
    let mut pending = vec![(dir.to_path_buf(), 0usize)];
    let mut seen = 0usize;
    while let Some((folder, depth)) = pending.pop() {
        let entries = match std::fs::read_dir(&folder) {
            Ok(entries) => entries,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound && depth == 0 => return Ok(false),
            Err(_) => return Err(()),
        };
        for entry in entries {
            let path = entry.map_err(|_| ())?.path();
            let metadata = std::fs::metadata(&path).map_err(|_| ())?;
            if metadata.is_dir() {
                // A skill is one folder deep; nothing below it is a skill.
                let descend = match kind {
                    ProjectMarkdown::Skills => depth == 0,
                    ProjectMarkdown::Agents
                    | ProjectMarkdown::Commands
                    | ProjectMarkdown::Routines => true,
                };
                if descend {
                    if depth + 1 > MAX_DEPTH {
                        return Err(());
                    }
                    pending.push((path, depth + 1));
                }
                continue;
            }
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let read = match kind {
                ProjectMarkdown::Skills => depth == 1 && name.eq_ignore_ascii_case("SKILL.md"),
                ProjectMarkdown::Agents | ProjectMarkdown::Commands | ProjectMarkdown::Routines => {
                    path.extension().and_then(|e| e.to_str()) == Some("md")
                }
            };
            if !read {
                continue;
            }
            seen += 1;
            if seen > MAX_FILES || metadata.len() > 1024 * 1024 {
                return Err(());
            }
            let text = std::fs::read_to_string(&path).map_err(|_| ())?;
            if frontmatter_names_a_model(&text) {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

/// The main checkout of the linked git worktree whose `.git` is the file
/// at `dot_git`: `gitdir:` names the worktree's admin folder, whose
/// `commondir` names the shared `.git`. `Ok(None)` for a `.git` file with
/// no `commondir` (a submodule), or a shared `.git` that is not a
/// checkout's. A `.git` file that cannot be read is an error.
fn linked_worktree_main(dot_git: &Path) -> Result<Option<PathBuf>, ()> {
    let base = dot_git.parent().ok_or(())?;
    let text = read_regular_file(dot_git, 64 * 1024).ok_or(())?;
    let text = std::str::from_utf8(&text).map_err(|_| ())?;
    let gitdir = text
        .lines()
        .find_map(|line| line.strip_prefix("gitdir:"))
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .ok_or(())?;
    let gitdir = base.join(gitdir);
    let commondir = match std::fs::read_to_string(gitdir.join("commondir")) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    let common = normalize_lexically(&gitdir.join(commondir.trim()));
    Ok(match common.file_name() {
        Some(name) if name == ".git" => common.parent().map(Path::to_path_buf),
        _ => None,
    })
}

/// `path` with `.` and `..` resolved without touching the file system.
fn normalize_lexically(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

/// The folders whose `.claude` Claude Code reads project agents, commands
/// and skills from, as 2.1.316 finds them: from `cwd` upward, stopping
/// before `home` and at the git root. A linked worktree nested inside its
/// main checkout walks on up to the main checkout's root, and a linked
/// worktree also gets its main checkout's folder. With no git root the
/// walk goes up to `home`, or to the file system root when `cwd` is not
/// under it. This may list more than Claude Code would read; never less.
fn claude_project_dirs(cwd: &Path, home: Option<&Path>) -> Result<Vec<PathBuf>, ()> {
    let git_root = cwd
        .ancestors()
        .find(|p| std::fs::symlink_metadata(p.join(".git")).is_ok());
    let mut stop = git_root.map(Path::to_path_buf);
    let mut main_checkout = None;
    if let Some(root) = git_root {
        let dot_git = root.join(".git");
        // Followed: a `.git` that links to a folder is an ordinary checkout.
        let metadata = std::fs::metadata(&dot_git).map_err(|_| ())?;
        if !metadata.is_dir() {
            main_checkout = linked_worktree_main(&dot_git)?;
            if let Some(main) = &main_checkout {
                if root.starts_with(main) {
                    stop = Some(main.clone());
                }
            }
        }
    }
    let mut dirs = vec![];
    for ancestor in cwd.ancestors() {
        if home == Some(ancestor) {
            break;
        }
        dirs.push(ancestor.to_path_buf());
        if stop.as_deref() == Some(ancestor) {
            break;
        }
    }
    if let Some(main) = main_checkout {
        if !dirs.contains(&main) {
            dirs.push(main);
        }
    }
    Ok(dirs)
}

/// Whether a project settings key outside `env` chooses a model other than
/// by alias: `fallbackModel` (a name or a list of them, tried when the
/// session's model is unavailable), `advisorModel` (the advisor tool's
/// model), and any non-empty `modelOverrides` (a remapping of model ids).
fn settings_choose_a_model(value: &serde_json::Value) -> bool {
    let not_alias = |v: &serde_json::Value| !v.as_str().is_some_and(is_claude_model_alias);
    let fallback = match value.get("fallbackModel") {
        None => false,
        Some(serde_json::Value::Array(models)) => models.iter().any(not_alias),
        Some(model) => not_alias(model),
    };
    let advisor = value.get("advisorModel").is_some_and(not_alias);
    let overrides = value
        .get("modelOverrides")
        .is_some_and(|o| o.as_object().is_none_or(|map| !map.is_empty()));
    fallback || advisor || overrides
}

/// Project configuration can outrank the isolated profile. Never claim a
/// selected subscription while a project requests a different credential.
///
/// On a NEAR AI session a project also may not choose Claude Code's models
/// (owner, 2026-11-01): its settings' `env` may not set a model variable
/// ([`names_a_claude_model`]), its settings may not choose one another way
/// ([`settings_choose_a_model`]), and none of its agents, commands or
/// skills may name a model by id ([`markdown_names_a_model`], wherever
/// Claude Code looks for them: [`claude_project_dirs`]). Each would outrank
/// the launcher's own model variables and could send work to a less
/// private model. The launch is refused rather than run without them. A
/// subscription session has no tiers to leave and is unchanged.
fn check_project_config(
    tool: ToolId,
    connection: ConnectionKind,
    cwd: &Path,
) -> Result<(), ManagedError> {
    check_project_config_in(tool, connection, cwd, dirs::home_dir().as_deref())
}

/// [`check_project_config`] with the home folder given.
fn check_project_config_in(
    tool: ToolId,
    connection: ConnectionKind,
    cwd: &Path,
    home: Option<&Path>,
) -> Result<(), ManagedError> {
    let near_ai = matches!(
        connection,
        ConnectionKind::NearAi | ConnectionKind::NearAiSignIn
    );
    // OpenCode runs in pure mode with project config disabled.
    if tool == ToolId::Opencode {
        return Ok(());
    }
    let project_root = cwd
        .ancestors()
        .find(|p| p.join(".git").exists())
        .unwrap_or(cwd);
    for ancestor in cwd.ancestors() {
        let names: &[&str] = match tool {
            ToolId::Claude => &[".claude/settings.json", ".claude/settings.local.json"],
            ToolId::Codex => &[".codex/config.toml"],
            ToolId::Opencode => &[],
        };
        for name in names {
            let path = ancestor.join(name);
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(_) => return Err(ManagedError::Conflict),
            };
            if bytes.len() > 1024 * 1024 {
                return Err(ManagedError::Conflict);
            }
            match tool {
                ToolId::Opencode => return Err(ManagedError::InvalidRequest),
                ToolId::Claude => {
                    let value: serde_json::Value =
                        serde_json::from_slice(&bytes).map_err(|_| ManagedError::Conflict)?;
                    if !value.is_object()
                        || value.get("apiKeyHelper").is_some()
                        || value.get("forceLoginMethod").is_some()
                        || value.get("forceLoginOrgUUID").is_some()
                        || value.get("forceLoginGatewayUrl").is_some()
                    {
                        return Err(ManagedError::Conflict);
                    }
                    if near_ai && settings_choose_a_model(&value) {
                        return Err(ManagedError::ProjectChoosesModel);
                    }
                    if let Some(env) = value.get("env") {
                        let env = env.as_object().ok_or(ManagedError::Conflict)?;
                        if env.keys().any(|k| AUTH_ENV.contains(&k.as_str())) {
                            return Err(ManagedError::Conflict);
                        }
                        if near_ai && env.keys().any(|k| names_a_claude_model(k)) {
                            return Err(ManagedError::ProjectChoosesModel);
                        }
                    }
                }
                ToolId::Codex => {
                    let text = std::str::from_utf8(&bytes).map_err(|_| ManagedError::Conflict)?;
                    let value: toml::Value =
                        toml::from_str(text).map_err(|_| ManagedError::Conflict)?;
                    if codex_auth_override(&value) {
                        return Err(ManagedError::Conflict);
                    }
                }
            }
        }
        if ancestor == project_root {
            break;
        }
    }
    project_model_check_in(tool, connection, cwd, &[], home)?;
    Ok(())
}

/// What a NEAR AI launch tells the person about its project before the
/// tool starts. The session starts either way.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct ProjectNotice {
    /// The project has Claude Code workflows, whose steps can choose a
    /// model this check cannot read (`project_workflows`).
    pub workflows: bool,
}

/// On a NEAR AI Claude Code launch, whether any agent, command, routine
/// or skill Claude Code would read names a model by id
/// ([`markdown_names_a_model`]), in the project's folders
/// ([`claude_project_dirs`]) or in a folder `cli_options` adds with
/// `--add-dir` ([`added_dirs`]): refused with
/// [`ManagedError::ProjectChoosesModel`]. Otherwise, what the launch should
/// say ([`ProjectNotice`]). Anything else, and any other connection, has
/// nothing to say.
pub fn project_model_check(
    tool: ToolId,
    connection: ConnectionKind,
    cwd: &Path,
    cli_options: &[String],
) -> Result<ProjectNotice, ManagedError> {
    project_model_check_in(
        tool,
        connection,
        cwd,
        cli_options,
        dirs::home_dir().as_deref(),
    )
}

/// [`project_model_check`] with the home folder given.
fn project_model_check_in(
    tool: ToolId,
    connection: ConnectionKind,
    cwd: &Path,
    cli_options: &[String],
    home: Option<&Path>,
) -> Result<ProjectNotice, ManagedError> {
    let near_ai = matches!(
        connection,
        ConnectionKind::NearAi | ConnectionKind::NearAiSignIn
    );
    if tool != ToolId::Claude || !near_ai {
        return Ok(ProjectNotice::default());
    }
    let project = claude_project_dirs(cwd, home).map_err(|_| ManagedError::Conflict)?;
    let added = added_dirs(cli_options, cwd);
    for dir in project.iter().chain(&added) {
        for kind in ProjectMarkdown::ALL {
            let folder = dir.join(".claude").join(kind.folder());
            if markdown_names_a_model(&folder, kind).unwrap_or(true) {
                return Err(ManagedError::ProjectChoosesModel);
            }
        }
    }
    let workflows = project.iter().chain(&added).any(|dir| holds_workflows(dir));
    Ok(ProjectNotice { workflows })
}

/// Whether Claude Code would find workflows for a launch of `tool` in
/// `cwd`, in any folder it reads the project from ([`claude_project_dirs`]),
/// whatever the connection. The macOS sheet reads it from
/// `managed_folder_settings_get` and shows the `project_workflows` line on
/// a NEAR AI session. A folder Claude Code's walk cannot read has none.
pub fn project_has_workflows(tool: ToolId, cwd: &Path) -> bool {
    project_has_workflows_in(tool, cwd, dirs::home_dir().as_deref())
}

/// [`project_has_workflows`] with the home folder given.
fn project_has_workflows_in(tool: ToolId, cwd: &Path, home: Option<&Path>) -> bool {
    tool == ToolId::Claude
        && claude_project_dirs(cwd, home)
            .is_ok_and(|dirs| dirs.iter().any(|dir| holds_workflows(dir)))
}

/// Whether `dir/.claude/workflows` holds a workflow ([`holds_an_entry`]).
fn holds_workflows(dir: &Path) -> bool {
    holds_an_entry(&dir.join(".claude/workflows"))
}

/// The folders Claude Code's `--add-dir` names in `cli_options`, relative
/// ones taken from `cwd`. The flag takes every value up to the next flag,
/// or one with `=`; Claude Code reads agents and commands from each
/// folder's `.claude` (2.1.316, its additional directories). Project
/// settings' `permissions.additionalDirectories` only widen what the tools
/// may touch and are not read for agents, so they are not listed.
fn added_dirs(cli_options: &[String], cwd: &Path) -> Vec<PathBuf> {
    let mut dirs = vec![];
    let mut taking = false;
    for arg in cli_options {
        if let Some(value) = arg.strip_prefix("--add-dir=") {
            dirs.push(cwd.join(value));
            taking = false;
        } else if arg == "--add-dir" {
            taking = true;
        } else if arg.starts_with('-') {
            taking = false;
        } else if taking {
            dirs.push(cwd.join(arg));
        }
    }
    dirs
}

/// Whether the folder `dir` holds anything but hidden files (a
/// `.gitkeep`, a `.DS_Store`). A folder that cannot be read counts as
/// holding something, so the notice is shown rather than left out.
fn holds_an_entry(dir: &Path) -> bool {
    match std::fs::read_dir(dir) {
        Ok(entries) => entries.into_iter().any(|entry| {
            entry.map_or(true, |entry| {
                !entry.file_name().to_string_lossy().starts_with('.')
            })
        }),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

fn codex_auth_override(value: &toml::Value) -> bool {
    match value {
        toml::Value::Table(table) => table.iter().any(|(key, value)| {
            matches!(
                key.as_str(),
                "model_provider"
                    | "model_providers"
                    | "base_url"
                    | "env_key"
                    | "http_headers"
                    | "env_http_headers"
                    | "experimental_bearer_token"
                    | "forced_login_method"
                    | "forced_chatgpt_workspace_id"
                    | "chatgpt_base_url"
                    | "cli_auth_credentials_store"
                    | "CODEX_HOME"
            ) || codex_auth_override(value)
        }),
        toml::Value::Array(values) => values.iter().any(codex_auth_override),
        _ => false,
    }
}

pub fn find_native(tool: ToolId) -> Result<PathBuf, ManagedError> {
    find_native_named(match tool {
        ToolId::Claude => "claude",
        ToolId::Codex => "codex",
        ToolId::Opencode => "opencode",
    })
}

fn find_native_named(name: &str) -> Result<PathBuf, ManagedError> {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    let folders = discovery_folders(&paths, dirs::home_dir().as_deref(), name);
    find_native_in(name, folders)
}

fn installed(name: &str) -> bool {
    let paths = std::env::var_os("PATH").unwrap_or_default();
    let folders = discovery_folders(&paths, dirs::home_dir().as_deref(), name);
    installed_in(name, folders, cfg!(windows))
}

/// Windows package managers may install command wrappers. These count as an
/// installation, but are never handed to the managed native executable launcher.
fn installed_in(name: &str, folders: Vec<PathBuf>, windows: bool) -> bool {
    if !windows {
        return find_native_in(name, folders).is_ok();
    }
    let current = std::env::current_exe()
        .ok()
        .and_then(|path| std::fs::canonicalize(path).ok());
    folders
        .iter()
        .filter(|folder| folder.is_absolute())
        .any(|folder| {
            ["exe", "cmd", "bat"].iter().any(|extension| {
                let candidate = folder.join(format!("{name}.{extension}"));
                candidate.is_file()
                    && std::fs::canonicalize(candidate)
                        .ok()
                        .is_some_and(|resolved| Some(&resolved) != current.as_ref())
            })
        })
}

fn discovery_folders(paths: &std::ffi::OsStr, home: Option<&Path>, name: &str) -> Vec<PathBuf> {
    let mut folders: Vec<PathBuf> = std::env::split_paths(paths).collect();
    if let Some(home) = home {
        folders.push(home.join(".local/bin"));
        folders.push(home.join(".cargo/bin"));
        if name == "opencode" {
            folders.push(home.join(".opencode/bin"));
        }
    }
    #[cfg(target_os = "macos")]
    folders.extend([
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    folders
}

fn find_native_in(name: &str, folders: Vec<PathBuf>) -> Result<PathBuf, ManagedError> {
    for folder in folders {
        // Relative PATH entries depend on the launch project and are not a
        // stable executable identity for the daemon/helper handoff.
        if !folder.is_absolute() {
            continue;
        }
        let file = folder.join(if cfg!(windows) {
            format!("{name}.exe")
        } else {
            name.into()
        });
        if executable_file(&file) {
            let Ok(resolved) = std::fs::canonicalize(file) else {
                continue;
            };
            if std::env::current_exe()
                .ok()
                .and_then(|p| std::fs::canonicalize(p).ok())
                .as_ref()
                != Some(&resolved)
            {
                return Ok(resolved);
            }
        }
    }
    Err(ManagedError::ToolUnavailable)
}

fn executable_file(path: &Path) -> bool {
    let Ok(metadata) = path.metadata() else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

/// Installed binaries, independently of trace sources or managed launch support.
/// Discovery reads filesystem metadata only; it never runs a tool or scans traces.
/// `install_url` is the tool's setup guide from `crate::install_guides`.
pub fn tools() -> serde_json::Value {
    use crate::install_guides::install_guide;
    serde_json::json!([
        {"tool": "claude", "installed": installed("claude"), "install_url": install_guide("claude-code")},
        {"tool": "codex", "installed": installed("codex"), "install_url": install_guide("codex")},
        {"tool": "opencode", "installed": installed("opencode"), "install_url": install_guide("opencode")}
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A project folder inside a git root, with `files` written under it.
    fn project(files: &[(&str, &str)]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join(".git")).unwrap();
        for (name, body) in files {
            let path = dir.path().join(name);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, body).unwrap();
        }
        dir
    }

    const NEAR_AI: [ConnectionKind; 2] = [ConnectionKind::NearAi, ConnectionKind::NearAiSignIn];

    /// A project's Claude Code settings may not name a model through their
    /// `env` block on a NEAR AI session: it would outrank the launcher's
    /// own model variables and could send work to a less private model
    /// (owner, 2026-11-01). The launch is refused, as an auth override is.
    #[test]
    fn a_project_cannot_set_claude_model_variables_on_a_near_ai_session() {
        for var in [
            "ANTHROPIC_MODEL",
            "ANTHROPIC_SMALL_FAST_MODEL",
            "ANTHROPIC_DEFAULT_OPUS_MODEL",
            "ANTHROPIC_DEFAULT_SONNET_MODEL",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL",
            "ANTHROPIC_DEFAULT_FABLE_MODEL",
            "ANTHROPIC_DEFAULT_NEXT_MODEL",
            "ANTHROPIC_CUSTOM_MODEL_OPTION",
            "CLAUDE_CODE_SUBAGENT_MODEL",
            "CLAUDE_CODE_WORKFLOW_SUBAGENT_MODEL",
            "CLAUDE_CODE_AUTO_MODE_MODEL",
            "CLAUDE_CODE_BG_CLASSIFIER_MODEL",
        ] {
            for file in [".claude/settings.json", ".claude/settings.local.json"] {
                let body = format!(r#"{{"env":{{"{var}":"anthropic/claude-opus-5-5"}}}}"#);
                let dir = project(&[(file, &body)]);
                for connection in NEAR_AI {
                    assert_eq!(
                        check_project_config(ToolId::Claude, connection, dir.path()),
                        Err(ManagedError::ProjectChoosesModel),
                        "{var} in {file} on {connection:?}"
                    );
                }
                // A subscription has no tiers to leave: unchanged.
                assert_eq!(
                    check_project_config(ToolId::Claude, ConnectionKind::Subscription, dir.path()),
                    Ok(()),
                    "{var}"
                );
            }
        }
        // A display name selects nothing, and other variables stay the
        // project's.
        let dir = project(&[(
            ".claude/settings.json",
            r#"{"env":{"ANTHROPIC_DEFAULT_SONNET_MODEL_NAME":"Sonnet","BASH_DEFAULT_TIMEOUT_MS":"1000"}}"#,
        )]);
        assert_eq!(
            check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
            Ok(())
        );
        // A parent folder up to the git root counts too.
        let dir = project(&[(
            ".claude/settings.json",
            r#"{"env":{"ANTHROPIC_MODEL":"x/y"}}"#,
        )]);
        let inner = dir.path().join("src/deep");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(
            check_project_config(ToolId::Claude, ConnectionKind::NearAiSignIn, &inner),
            Err(ManagedError::ProjectChoosesModel)
        );
    }

    /// A project's subagent that names a model of its own by id, in its
    /// `.claude/agents` frontmatter, would run there whatever the session's
    /// tier, so a NEAR AI session refuses to start rather than skip it
    /// silently (owner, 2026-11-01). An alias Claude Code maps onto the
    /// session's own model variables is fine, as is no model at all.
    #[test]
    fn a_project_agent_naming_a_model_refuses_a_near_ai_session() {
        let agent = |model: &str| {
            format!("---\nname: helper\ndescription: d\nmodel: {model}\n---\nBody.\n")
        };
        for model in [
            "anthropic/claude-opus-5-5",
            "\"anthropic/claude-haiku-4-5\"",
            "claude-opus-4-1",
            "Qwen/Qwen3.8-27B",
        ] {
            for path in [".claude/agents/helper.md", ".claude/agents/team/helper.md"] {
                let dir = project(&[(path, &agent(model))]);
                for connection in NEAR_AI {
                    assert_eq!(
                        check_project_config(ToolId::Claude, connection, dir.path()),
                        Err(ManagedError::ProjectChoosesModel),
                        "{model} at {path}"
                    );
                }
                assert_eq!(
                    check_project_config(ToolId::Claude, ConnectionKind::Subscription, dir.path()),
                    Ok(())
                );
            }
        }
        for model in [
            "inherit", "haiku", "sonnet", "opus", "fable", "'sonnet'", "opus[1m]",
        ] {
            let dir = project(&[(".claude/agents/helper.md", &agent(model))]);
            assert_eq!(
                check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
                Ok(()),
                "{model}"
            );
        }
        let dir = project(&[
            (
                ".claude/agents/plain.md",
                "---\nname: plain\ndescription: d\n---\nmodel: x/y is prose here\n",
            ),
            (
                ".claude/agents/notes.txt",
                "model: anthropic/claude-opus-5-5\n",
            ),
        ]);
        assert_eq!(
            check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
            Ok(())
        );
        // An agent file that cannot be read whole is not guessed at.
        let dir = project(&[(".claude/agents/big.md", &"x".repeat(1024 * 1024 + 1))]);
        assert_eq!(
            check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
            Err(ManagedError::ProjectChoosesModel)
        );
    }

    const ID: &str = "anthropic/claude-opus-5-5";

    fn markdown(model_block: &str) -> String {
        format!("---\nname: helper\ndescription: d\n{model_block}\n---\nBody.\n")
    }

    /// Claude Code reads a `model:` from project slash commands and skills
    /// as well as subagents (#1803 review). Each is held to the same rule.
    #[test]
    fn a_project_command_or_skill_naming_a_model_refuses_a_near_ai_session() {
        for path in [
            ".claude/commands/review.md",
            ".claude/commands/team/review.md",
            ".claude/skills/review/SKILL.md",
            ".claude/routines/nightly.md",
        ] {
            let dir = project(&[(path, &markdown(&format!("model: {ID}")))]);
            for connection in NEAR_AI {
                assert_eq!(
                    check_project_config(ToolId::Claude, connection, dir.path()),
                    Err(ManagedError::ProjectChoosesModel),
                    "{path} on {connection:?}"
                );
            }
            assert_eq!(
                check_project_config(ToolId::Claude, ConnectionKind::Subscription, dir.path()),
                Ok(()),
                "{path}"
            );
            let dir = project(&[(path, &markdown("model: opus"))]);
            assert_eq!(
                check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
                Ok(()),
                "{path} with an alias"
            );
        }
    }

    /// Settings keys that choose a model outside the `env` block: the
    /// fallback list, the advisor's model, and a remapping of model ids
    /// (#1803 review). Aliases still follow the session's tier.
    #[test]
    fn project_settings_choosing_a_model_outside_env_refuse_a_near_ai_session() {
        for body in [
            format!(r#"{{"fallbackModel":["{ID}"]}}"#),
            format!(r#"{{"fallbackModel":["sonnet","{ID}"]}}"#),
            format!(r#"{{"fallbackModel":"{ID}"}}"#),
            r#"{"fallbackModel":[1]}"#.to_string(),
            format!(r#"{{"advisorModel":"{ID}"}}"#),
            r#"{"advisorModel":7}"#.to_string(),
            format!(r#"{{"modelOverrides":{{"claude-opus-5-5":"{ID}"}}}}"#),
        ] {
            for file in [".claude/settings.json", ".claude/settings.local.json"] {
                let dir = project(&[(file, &body)]);
                for connection in NEAR_AI {
                    assert_eq!(
                        check_project_config(ToolId::Claude, connection, dir.path()),
                        Err(ManagedError::ProjectChoosesModel),
                        "{body} in {file} on {connection:?}"
                    );
                }
                assert_eq!(
                    check_project_config(ToolId::Claude, ConnectionKind::Subscription, dir.path()),
                    Ok(()),
                    "{body}"
                );
            }
        }
        for body in [
            r#"{"fallbackModel":["default","sonnet","opus[1m]"]}"#,
            r#"{"fallbackModel":"haiku"}"#,
            r#"{"fallbackModel":[]}"#,
            r#"{"advisorModel":"opus"}"#,
            r#"{"modelOverrides":{}}"#,
        ] {
            let dir = project(&[(".claude/settings.json", body)]);
            assert_eq!(
                check_project_config(ToolId::Claude, ConnectionKind::NearAi, dir.path()),
                Ok(()),
                "{body}"
            );
        }
    }

    /// YAML forms Claude Code reads as a `model` that a line-prefix match
    /// missed, and forms it cannot be sure of, all count as naming a model
    /// (#1803 review). Aliases written plainly still pass, and a `model:`
    /// that is only text inside another field stays allowed.
    #[test]
    fn frontmatter_naming_a_model_in_any_yaml_form_is_caught() {
        for block in [
            format!("model:\n  {ID}"),
            format!("model : {ID}"),
            format!("model:\t{ID}"),
            "model: opus\n  -4-1".to_string(),
            format!("\"model\": {ID}"),
            format!("'model': {ID}"),
            format!("? model\n: {ID}"),
            format!("{{model: {ID}}}"),
            format!("model: !!str {ID}"),
            format!("model: |\n  {ID}"),
            format!("base: &b {ID}\nmodel: *b"),
            format!("<<: {{model: {ID}}}"),
            "model:".to_string(),
            "Model: x/y".to_string(),
            "model: opus\nmodel: x/y".to_string(),
        ] {
            assert!(
                frontmatter_names_a_model(&markdown(&block)),
                "{block:?} should count as naming a model"
            );
        }
        for block in [
            "model: opus",
            "model: 'sonnet'",
            "model: \"opus[1m]\"",
            "model: inherit # the session's",
            "description2: |\n  model: x/y is prose",
            "tools: Read, Grep",
            "",
        ] {
            assert!(
                !frontmatter_names_a_model(&markdown(block)),
                "{block:?} should not count"
            );
        }
        assert!(frontmatter_names_a_model(&format!(
            "\u{feff}---\r\nmodel: {ID}\r\n---\r\n"
        )));
        assert!(!frontmatter_names_a_model(&format!(
            "no frontmatter\nmodel: {ID}\n"
        )));
    }

    /// Claude Code looks for project agents, commands and skills in more
    /// folders than the git root's ancestors (#1803 review): with no git
    /// repository it walks up to (not including) the home folder; a linked
    /// worktree also gets the main checkout's; and a worktree nested inside
    /// the main checkout walks up to the main checkout's root.
    #[test]
    fn project_markdown_is_found_where_claude_code_looks_for_it() {
        let agent = markdown(&format!("model: {ID}"));
        let refused = |cwd: &Path, home: &Path| {
            check_project_config_in(ToolId::Claude, ConnectionKind::NearAi, cwd, Some(home))
        };

        // No git repository: every folder up to home counts, home does not.
        let home = tempfile::tempdir().unwrap();
        let scratch = home.path().join("work/scratch");
        std::fs::create_dir_all(&scratch).unwrap();
        let parent_agents = home.path().join("work/.claude/agents");
        std::fs::create_dir_all(&parent_agents).unwrap();
        std::fs::write(parent_agents.join("x.md"), &agent).unwrap();
        assert_eq!(
            refused(&scratch, home.path()),
            Err(ManagedError::ProjectChoosesModel)
        );
        std::fs::remove_dir_all(home.path().join("work/.claude")).unwrap();
        let home_agents = home.path().join(".claude/agents");
        std::fs::create_dir_all(&home_agents).unwrap();
        std::fs::write(home_agents.join("x.md"), &agent).unwrap();
        assert_eq!(
            refused(&scratch, home.path()),
            Ok(()),
            "home's own .claude is the person's, not a project's"
        );

        // A linked worktree outside the main checkout.
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        let wt_git = main.join(".git/worktrees/wt");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let worktree = root.path().join("wt");
        std::fs::create_dir_all(worktree.join("src")).unwrap();
        std::fs::write(
            worktree.join(".git"),
            format!("gitdir: {}\n", wt_git.display()),
        )
        .unwrap();
        assert_eq!(refused(&worktree.join("src"), root.path()), Ok(()));
        let main_commands = main.join(".claude/commands");
        std::fs::create_dir_all(&main_commands).unwrap();
        std::fs::write(main_commands.join("x.md"), &agent).unwrap();
        assert_eq!(
            refused(&worktree.join("src"), root.path()),
            Err(ManagedError::ProjectChoosesModel),
            "the main checkout's commands"
        );

        // A worktree nested inside the main checkout walks up to its root.
        let root = tempfile::tempdir().unwrap();
        let main = root.path().join("main");
        let wt_git = main.join(".git/worktrees/x");
        std::fs::create_dir_all(&wt_git).unwrap();
        std::fs::write(wt_git.join("commondir"), "../..\n").unwrap();
        let nested = main.join("nest/x");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(".git"), "gitdir: ../../.git/worktrees/x\n").unwrap();
        assert_eq!(refused(&nested, root.path()), Ok(()));
        let between = main.join("nest/.claude/skills/s");
        std::fs::create_dir_all(&between).unwrap();
        std::fs::write(between.join("SKILL.md"), &agent).unwrap();
        assert_eq!(
            refused(&nested, root.path()),
            Err(ManagedError::ProjectChoosesModel)
        );

        // A `.git` that links to a folder is an ordinary checkout.
        #[cfg(unix)]
        {
            let root = tempfile::tempdir().unwrap();
            std::fs::create_dir(root.path().join("real-git")).unwrap();
            let repo = root.path().join("repo");
            std::fs::create_dir(&repo).unwrap();
            std::os::unix::fs::symlink(root.path().join("real-git"), repo.join(".git")).unwrap();
            assert_eq!(refused(&repo, root.path()), Ok(()));
        }

        // A `.git` file that cannot be understood is not guessed at.
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join(".git"), "not a gitdir line\n").unwrap();
        assert_eq!(
            check_project_config_in(
                ToolId::Claude,
                ConnectionKind::NearAi,
                root.path(),
                Some(root.path())
            ),
            Err(ManagedError::Conflict)
        );
    }

    /// The refusal names what happened, in the owner's words (2026-11-01),
    /// and is the shared copy line the shells show for its label.
    #[test]
    fn the_project_model_refusal_says_why() {
        let text = "This project's Claude Code settings, agents, commands or skills choose a model. NEAR AI sessions use the session's models.";
        assert_eq!(ManagedError::ProjectChoosesModel.to_string(), text);
        assert_eq!(crate::managed::copy::copy()["project_chooses_model"], text);
        assert_eq!(
            crate::managed::PROJECT_CHOOSES_MODEL_LABEL,
            "managed-project-chooses-model"
        );
    }

    /// Folders given to Claude Code with `--add-dir` are read for agents
    /// and commands too, so they are held to the same rule (owner,
    /// 2026-11-01). Each spelling of the flag counts, and a relative folder
    /// is the launch folder's.
    #[test]
    fn folders_added_with_add_dir_are_held_to_the_same_rule() {
        let check = |cwd: &Path, args: &[&str]| {
            let args: Vec<String> = args.iter().map(|a| a.to_string()).collect();
            project_model_check_in(
                ToolId::Claude,
                ConnectionKind::NearAi,
                cwd,
                &args,
                Some(cwd),
            )
        };
        let launch = project(&[]);
        let extra = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        for kind in ["agents", "commands", "routines"] {
            let dir = extra.path().join(".claude").join(kind);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("x.md"), markdown(&format!("model: {ID}"))).unwrap();
            let extra_s = extra.path().to_str().unwrap();
            let other_s = other.path().to_str().unwrap();
            for args in [
                vec!["--add-dir", extra_s],
                vec![&format!("--add-dir={extra_s}")],
                vec!["--add-dir", other_s, extra_s, "--verbose"],
            ] {
                let args: Vec<&str> = args.iter().map(|a| a.as_ref()).collect();
                assert_eq!(
                    check(launch.path(), &args),
                    Err(ManagedError::ProjectChoosesModel),
                    "{kind} via {args:?}"
                );
            }
            std::fs::remove_dir_all(extra.path().join(".claude")).unwrap();
        }
        let skill = extra.path().join(".claude/skills/s");
        std::fs::create_dir_all(&skill).unwrap();
        std::fs::write(skill.join("SKILL.md"), markdown(&format!("model: {ID}"))).unwrap();
        assert_eq!(
            check(
                launch.path(),
                &["--add-dir", extra.path().to_str().unwrap()]
            ),
            Err(ManagedError::ProjectChoosesModel)
        );
        // Relative to the launch folder.
        let nested = launch.path().join("sub/.claude/agents");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join("x.md"), markdown(&format!("model: {ID}"))).unwrap();
        let inner = launch.path().join("inner");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(
            check(&inner, &["--add-dir", "../sub"]),
            Err(ManagedError::ProjectChoosesModel)
        );
        // An alias is fine there too, and a folder not added is not read.
        assert_eq!(
            check(launch.path(), &["--verbose"]),
            Ok(ProjectNotice::default())
        );
        std::fs::write(nested.join("x.md"), markdown("model: opus")).unwrap();
        assert_eq!(
            check(&inner, &["--add-dir", "../sub"]),
            Ok(ProjectNotice::default())
        );
        // A subscription session is unchanged.
        let args = vec!["--add-dir".to_string(), extra.path().display().to_string()];
        assert_eq!(
            project_model_check_in(
                ToolId::Claude,
                ConnectionKind::Subscription,
                launch.path(),
                &args,
                None
            ),
            Ok(ProjectNotice::default())
        );
    }

    /// A project's Claude Code workflows do not refuse a NEAR AI session,
    /// since a step's model cannot be read from the script; the launch
    /// says so instead (owner, 2026-11-01). Only on NEAR AI.
    #[test]
    fn project_workflows_start_with_a_notice_on_near_ai() {
        let dir = project(&[(".claude/workflows/release.js", "export default {}\n")]);
        for connection in NEAR_AI {
            assert_eq!(
                project_model_check_in(ToolId::Claude, connection, dir.path(), &[], None),
                Ok(ProjectNotice { workflows: true }),
                "{connection:?}"
            );
            // The launch itself is not refused.
            assert_eq!(
                check_project_config(ToolId::Claude, connection, dir.path()),
                Ok(())
            );
        }
        assert_eq!(
            project_model_check_in(
                ToolId::Claude,
                ConnectionKind::Subscription,
                dir.path(),
                &[],
                None
            ),
            Ok(ProjectNotice::default())
        );
        let empty = project(&[(".claude/workflows/.keep", "")]);
        std::fs::remove_file(empty.path().join(".claude/workflows/.keep")).unwrap();
        assert_eq!(
            project_model_check_in(
                ToolId::Claude,
                ConnectionKind::NearAi,
                empty.path(),
                &[],
                None
            ),
            Ok(ProjectNotice::default()),
            "an empty folder holds no workflow"
        );
        let hidden = project(&[
            (".claude/workflows/.gitkeep", ""),
            (".claude/workflows/.DS_Store", ""),
        ]);
        assert_eq!(
            project_model_check_in(
                ToolId::Claude,
                ConnectionKind::NearAi,
                hidden.path(),
                &[],
                None
            ),
            Ok(ProjectNotice::default()),
            "hidden files are not workflows"
        );
        // A parent folder's workflows count, as Claude Code reads them.
        let inner = dir.path().join("src");
        std::fs::create_dir_all(&inner).unwrap();
        assert_eq!(
            project_model_check_in(ToolId::Claude, ConnectionKind::NearAi, &inner, &[], None),
            Ok(ProjectNotice { workflows: true })
        );
        assert_eq!(
            crate::managed::copy::copy()["project_workflows"],
            "This project has Claude Code workflows. A workflow step can choose its own model, so it may not stay in this session's privacy tier."
        );
    }

    /// Whether a folder has Claude Code workflows, whatever the connection:
    /// the shells decide when that matters. Read where the launch reads
    /// them, parent folders up to the git root included.
    #[test]
    fn project_workflows_are_found_for_claude_code_only() {
        let dir = project(&[(".claude/workflows/release.js", "")]);
        let inner = dir.path().join("src");
        std::fs::create_dir_all(&inner).unwrap();
        assert!(project_has_workflows_in(ToolId::Claude, &inner, None));
        assert!(!project_has_workflows_in(ToolId::Codex, &inner, None));
        assert!(!project_has_workflows_in(ToolId::Opencode, &inner, None));
        let plain = project(&[(".claude/workflows/.gitkeep", "")]);
        assert!(!project_has_workflows_in(
            ToolId::Claude,
            plain.path(),
            None
        ));
    }

    #[test]
    fn existing_roots_lists_account_folders_and_creates_none() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("config")).unwrap();
        let profiles = NativeProfiles::new(&store);
        assert!(profiles.existing_roots().is_empty());
        assert!(
            !store.dir().join("managed-profiles").exists(),
            "created nothing"
        );

        let id = AccountId(uuid::Uuid::new_v4());
        let made = profiles.profile_root(id).unwrap();
        let base = store.dir().join("managed-profiles");
        std::fs::create_dir(base.join("not-an-account")).unwrap();
        std::fs::write(base.join(uuid::Uuid::new_v4().to_string()), b"file").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(dir.path(), base.join(uuid::Uuid::new_v4().to_string()))
            .unwrap();
        assert_eq!(profiles.existing_roots(), vec![made]);
    }

    #[test]
    fn managed_claude_commands_remove_inherited_secure_storage_override() {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().join("config")).unwrap();
        let profiles = NativeProfiles::new(&store);
        let executable = dir.path().join("fixture-claude");
        std::fs::write(&executable, b"never executed").unwrap();
        let account = AccountView {
            id: AccountId(uuid::Uuid::new_v4()),
            tool: ToolId::Claude,
            connection: ConnectionKind::Subscription,
            label: "fixture".into(),
            auth_state: AuthState::Ready,
            verified_at: None,
        };
        let command = profiles
            .command(&account, &executable, NativeAction::Status, dir.path())
            .unwrap();
        let removed = command
            .get_envs()
            .find(|(key, _)| *key == "CLAUDE_SECURESTORAGE_CONFIG_DIR");
        assert_eq!(removed.map(|(_, value)| value), Some(None));
        assert_eq!(
            command
                .get_envs()
                .find(|(key, _)| *key == "CLAUDE_CONFIG_DIR")
                .and_then(|(_, value)| value),
            Some(profiles.profile_root(account.id).unwrap().as_os_str())
        );
    }

    #[test]
    fn windows_installation_detects_wrappers_without_expanding_native_launch() {
        let home = tempfile::tempdir().unwrap();
        let folders = vec![home.path().to_path_buf()];
        for extension in ["cmd", "bat", "exe"] {
            let file = home.path().join(format!("codex.{extension}"));
            std::fs::write(&file, b"must not execute during discovery").unwrap();
            assert!(installed_in("codex", folders.clone(), true));
            assert!(!installed_in("claude", folders.clone(), true));
            if extension != "exe" {
                assert!(find_native_in("codex", folders.clone()).is_err());
            }
            std::fs::remove_file(file).unwrap();
        }
        std::fs::create_dir(home.path().join("codex.cmd")).unwrap();
        assert!(!installed_in("codex", folders, true));
        assert!(!installed_in("codex", vec![PathBuf::from(".")], true));
    }

    #[test]
    fn availability_rows_cover_every_managed_tool() {
        let rows = tools();
        let rows = rows.as_array().unwrap();
        assert_eq!(rows.len(), 3);
        for (row, name) in rows.iter().zip(["claude", "codex", "opencode"]) {
            assert_eq!(row["tool"], name);
            assert!(row["installed"].is_boolean());
            assert!(row["install_url"].as_str().unwrap().starts_with("https://"));
            assert_eq!(row.as_object().unwrap().len(), 3);
            // Every row is a managed tool (#1417 made OpenCode one).
            assert!(serde_json::from_value::<ToolId>(row["tool"].clone()).is_ok());
        }
        for name in ["claude", "codex", "opencode"] {
            assert!(serde_json::from_str::<ToolId>(&format!("\"{name}\"")).is_ok());
        }
    }

    #[cfg(unix)]
    #[test]
    fn discovery_requires_an_executable_and_keeps_searching() {
        use std::os::unix::fs::PermissionsExt;
        let home = tempfile::tempdir().unwrap();
        let first = home.path().join("first");
        let second = home.path().join(".opencode/bin");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        std::fs::write(first.join("opencode"), b"not executable").unwrap();
        std::fs::set_permissions(
            first.join("opencode"),
            std::fs::Permissions::from_mode(0o600),
        )
        .unwrap();
        assert_eq!(
            find_native_in("opencode", vec![first.clone()]),
            Err(ManagedError::ToolUnavailable)
        );
        std::fs::write(second.join("opencode"), b"not run during discovery").unwrap();
        std::fs::set_permissions(
            second.join("opencode"),
            std::fs::Permissions::from_mode(0o700),
        )
        .unwrap();
        assert_eq!(
            find_native_in("opencode", vec![first.clone(), second.clone()]).unwrap(),
            std::fs::canonicalize(second.join("opencode")).unwrap()
        );
        assert!(find_native_in("claude", vec![second.clone()]).is_err());
        assert!(find_native_in("codex", vec![second.clone()]).is_err());
        std::fs::remove_file(second.join("opencode")).unwrap();
        assert_eq!(
            find_native_in("opencode", vec![first, second]),
            Err(ManagedError::ToolUnavailable)
        );
    }

    #[test]
    fn opencode_installer_directory_is_discovered_without_path() {
        let home = tempfile::tempdir().unwrap();
        let folders = discovery_folders(std::ffi::OsStr::new(""), Some(home.path()), "opencode");
        assert!(folders.contains(&home.path().join(".opencode/bin")));
        let claude = discovery_folders(std::ffi::OsStr::new(""), Some(home.path()), "claude");
        assert!(!claude.contains(&home.path().join(".opencode/bin")));
    }

    #[test]
    fn discovery_ignores_relative_paths_and_directories() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join("claude")).unwrap();
        assert!(
            find_native_in(
                "claude",
                vec![home.path().to_path_buf(), PathBuf::from(".")]
            )
            .is_err()
        );
    }

    #[test]
    fn a_directory_another_launch_just_made_is_accepted() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("profile");
        std::fs::create_dir(&path).unwrap();
        assert!(create_private_directory(&path).is_ok());
    }

    #[test]
    fn a_file_in_the_directory_s_place_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("profile");
        std::fs::write(&path, b"").unwrap();
        assert!(matches!(
            create_private_directory(&path),
            Err(ManagedError::StorageUnavailable)
        ));
    }

    #[cfg(unix)]
    #[test]
    fn a_symlink_in_the_directory_s_place_is_refused() {
        let home = tempfile::tempdir().unwrap();
        let target = home.path().join("elsewhere");
        std::fs::create_dir(&target).unwrap();
        let path = home.path().join("profile");
        std::os::unix::fs::symlink(&target, &path).unwrap();
        assert!(matches!(
            create_private_directory(&path),
            Err(ManagedError::StorageUnavailable)
        ));
    }

    #[test]
    fn concurrent_creates_all_succeed() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("profile");
        let results: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || private_directory(&path))
            })
            .collect::<Vec<_>>()
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert!(results.iter().all(Result::is_ok), "{results:?}");
    }
}
