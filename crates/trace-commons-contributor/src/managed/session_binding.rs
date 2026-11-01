//! Per-launch Codex SessionStart receipts. Hooks approve only this declaration;
//! neither account credentials nor other hook trust decisions are changed.

use super::{ManagedError, profiles::private_directory};
use crate::config::ConfigStore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    process::Command,
};
use uuid::Uuid;

pub const DIRECTORY: &str = "managed-native-bindings";
const NONCE_FILE: &str = "nonce";
const MAX_DOCUMENT: usize = 64 * 1024;
const HOOK_TIMEOUT: u64 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundSession {
    pub session_id: Uuid,
    pub transcript_path: PathBuf,
}

pub struct CodexBinding {
    directory: PathBuf,
    nonce: String,
}

pub fn prepare(store: &ConfigStore, id: Uuid) -> Result<CodexBinding, ManagedError> {
    let root = std::fs::canonicalize(store.dir())
        .map_err(|_| ManagedError::StorageUnavailable)?
        .join(DIRECTORY);
    private_directory(&root)?;
    let directory = root.join(id.to_string());
    if directory.exists() {
        return Err(ManagedError::Conflict);
    }
    private_directory(&directory)?;
    let nonce = Uuid::new_v4().simple().to_string();
    let mut file = create_private(&directory.join(NONCE_FILE))?;
    file.write_all(nonce.as_bytes())
        .map_err(|_| ManagedError::StorageUnavailable)?;
    file.sync_all()
        .map_err(|_| ManagedError::StorageUnavailable)?;
    Ok(CodexBinding { directory, nonce })
}

/// The binding `prepare` made for launch `id`, opened again to read its
/// records, for collecting from a collection plan after the launcher is
/// gone. Reads its nonce from the directory, so `records` still checks it.
pub fn reopen(store: &ConfigStore, id: Uuid) -> Result<CodexBinding, ManagedError> {
    let directory = std::fs::canonicalize(store.dir())
        .map_err(|_| ManagedError::StorageUnavailable)?
        .join(DIRECTORY)
        .join(id.to_string());
    validate_directory(&directory)?;
    let nonce = String::from_utf8(read_regular(&directory.join(NONCE_FILE), 128)?)
        .map_err(|_| ManagedError::InvalidRequest)?;
    Ok(CodexBinding { directory, nonce })
}

impl CodexBinding {
    pub fn directory(&self) -> &Path {
        &self.directory
    }

    /// CLI-origin hook arrays cannot be silently replaced. File-based user and
    /// project hooks remain separate sources in Codex's hook discovery engine.
    /// The caller can continue launching without collection when this refuses.
    pub fn configure(&self, command: &mut Command) -> Result<(), ManagedError> {
        let args = command
            .get_args()
            .map(|arg| arg.to_str().ok_or(ManagedError::InvalidRequest))
            .collect::<Result<Vec<_>, _>>()?;
        if args.contains(&"--") || has_cli_hooks(&args)? {
            return Err(ManagedError::InvalidRequest);
        }
        let executable = std::env::current_exe().map_err(|_| ManagedError::StorageUnavailable)?;
        let line = hook_command(&executable, &self.directory, &self.nonce)?;
        let quoted = toml::Value::String(line.clone()).to_string();
        let declaration = format!(
            "hooks.SessionStart=[{{hooks=[{{type=\"command\",command={quoted},timeout={HOOK_TIMEOUT}}}]}}]"
        );
        // A TOML table's display is not an inline value. Serialize the tiny
        // state map explicitly so the CLI parses the source key as one key.
        let key = toml::Value::String(hook_key()?.to_string()).to_string();
        let hash = toml::Value::String(hook_hash(&line)?).to_string();
        let approval = format!("hooks.state={{{key}={{trusted_hash={hash}}}}}");
        command.args(["-c", &declaration, "-c", &approval]);
        // Codex 0.164's daemon_startup::config_exclusion forces an embedded
        // server for these hook overrides. No TUI-only flag is needed, so
        // subcommands such as `exec` retain their supported CLI arguments.
        Ok(())
    }

    pub fn records(&self) -> Result<Vec<BoundSession>, ManagedError> {
        validate_directory(&self.directory)?;
        if read_regular(&self.directory.join(NONCE_FILE), 128)? != self.nonce.as_bytes() {
            return Err(ManagedError::InvalidRequest);
        }
        read_records(&self.directory)
    }
}

/// The Codex conversation an earlier launch `id` recorded, for resuming
/// it (and for collecting a resumed row's continuation of it): its binding directory holds exactly one valid record, whose
/// transcript is still there. None otherwise: no binding (the hook was
/// refused, or the launch predates it), no record (the tool never
/// started), or more than one conversation, where a guess could continue
/// the wrong one. Reads only; the binding is never prepared again.
pub fn read(store: &ConfigStore, id: Uuid) -> Option<BoundSession> {
    let directory = std::fs::canonicalize(store.dir())
        .ok()?
        .join(DIRECTORY)
        .join(id.to_string());
    validate_directory(&directory).ok()?;
    // The launch's nonce is not known here; its file marks a directory
    // `prepare` made.
    read_regular(&directory.join(NONCE_FILE), 128).ok()?;
    let mut records = read_records(&directory).ok()?;
    if records.len() != 1 {
        return None;
    }
    records.pop()
}

/// Every record in an already validated binding directory, each named by
/// its session id and naming a transcript that is a regular file.
fn read_records(directory: &Path) -> Result<Vec<BoundSession>, ManagedError> {
    let mut records = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|_| ManagedError::StorageUnavailable)? {
        let path = entry.map_err(|_| ManagedError::StorageUnavailable)?.path();
        if path.extension().is_none_or(|extension| extension != "json") {
            continue;
        }
        let id = path
            .file_stem()
            .and_then(|value| value.to_str())
            .and_then(|value| Uuid::parse_str(value).ok())
            .ok_or(ManagedError::InvalidRequest)?;
        let record: BoundSession = serde_json::from_slice(&read_regular(&path, MAX_DOCUMENT)?)
            .map_err(|_| ManagedError::InvalidRequest)?;
        if record.session_id != id {
            return Err(ManagedError::InvalidRequest);
        }
        validate_transcript(&record.transcript_path)?;
        records.push(record);
    }
    records.sort_by_key(|record| record.session_id);
    Ok(records)
}

fn hook_key() -> Result<&'static str, ManagedError> {
    // Only Unix's synthetic SessionFlags origin has been verified against the
    // native hook engine. Unknown origins must disable collection, not trust a
    // guessed handler or prevent the native session from launching.
    #[cfg(unix)]
    {
        Ok("/<session-flags>/config.toml:session_start:0:0")
    }
    #[cfg(not(unix))]
    {
        Err(ManagedError::InvalidRequest)
    }
}

fn hook_hash(command: &str) -> Result<String, ManagedError> {
    // Codex 0.164 normalizes optional fields away and includes the effective
    // timeout and async flag, then hashes sorted compact JSON derived from TOML.
    let mut identity = serde_json::json!({"event_name":"session_start","hooks":[{"type":"command","command":command,"timeout":HOOK_TIMEOUT,"async":false}]});
    identity.sort_all_objects();
    let bytes = serde_json::to_vec(&identity).map_err(|_| ManagedError::InvalidRequest)?;
    Ok(format!("sha256:{:x}", Sha256::digest(bytes)))
}

fn has_cli_hooks(args: &[&str]) -> Result<bool, ManagedError> {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let config = if matches!(*arg, "-c" | "--config") {
            Some(*args.next().ok_or(ManagedError::InvalidRequest)?)
        } else if let Some(value) = arg.strip_prefix("--config=") {
            Some(value)
        } else if arg.starts_with("-c") && !arg.starts_with("--") {
            Some(&arg[2..])
        } else {
            None
        };
        if let Some(config) = config {
            let (key, _) = config.split_once('=').ok_or(ManagedError::InvalidRequest)?;
            let key: toml::Table = format!("{key}=0")
                .parse()
                .map_err(|_| ManagedError::InvalidRequest)?;
            if key.contains_key("hooks") {
                return Ok(true);
            }
        }
    }
    Ok(false)
}

fn hook_command(executable: &Path, directory: &Path, nonce: &str) -> Result<String, ManagedError> {
    let exe = executable.to_str().ok_or(ManagedError::InvalidRequest)?;
    let directory = directory.to_str().ok_or(ManagedError::InvalidRequest)?;
    #[cfg(not(windows))]
    {
        let quote = |word: &str| format!("'{}'", word.replace('\'', r"'\''"));
        Ok(format!(
            "{} managed-session-bind --directory {} --nonce {}",
            quote(exe),
            quote(directory),
            quote(nonce)
        ))
    }
    #[cfg(windows)]
    {
        use base64::Engine;
        // An encoded PowerShell command keeps spaces, %, quotes and Unicode
        // out of the outer shell's grammar (cmd, PowerShell or Git Bash).
        let quote = |word: &str| format!("'{}'", word.replace('\'', "''"));
        let script = format!(
            "$OutputEncoding=[System.Text.UTF8Encoding]::new($false); [Console]::InputEncoding=[System.Text.Encoding]::UTF8; [Console]::In.ReadToEnd() | & {} managed-session-bind --directory {} --nonce {}",
            quote(exe),
            quote(directory),
            quote(nonce)
        );
        let bytes = script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>();
        Ok(format!(
            "powershell.exe -NoLogo -NoProfile -NonInteractive -EncodedCommand {}",
            base64::engine::general_purpose::STANDARD.encode(bytes)
        ))
    }
}

fn no_symlink_components(path: &Path) -> Result<(), ManagedError> {
    if !path.is_absolute()
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(ManagedError::InvalidRequest);
    }
    for ancestor in path.ancestors().filter(|path| path.is_absolute()) {
        let metadata =
            std::fs::symlink_metadata(ancestor).map_err(|_| ManagedError::InvalidRequest)?;
        if metadata.file_type().is_symlink() {
            return Err(ManagedError::InvalidRequest);
        }
    }
    Ok(())
}

fn validate_directory(directory: &Path) -> Result<(), ManagedError> {
    no_symlink_components(directory)?;
    if directory
        .file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| Uuid::parse_str(name).ok())
        .is_none()
        || directory
            .parent()
            .and_then(Path::file_name)
            .is_none_or(|name| name != DIRECTORY)
        || !std::fs::metadata(directory).is_ok_and(|metadata| metadata.is_dir())
    {
        return Err(ManagedError::InvalidRequest);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if std::fs::metadata(directory)
            .map_err(|_| ManagedError::InvalidRequest)?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            return Err(ManagedError::InvalidRequest);
        }
    }
    Ok(())
}

fn validate_transcript(path: &Path) -> Result<(), ManagedError> {
    no_symlink_components(path)?;
    if !std::fs::metadata(path).is_ok_and(|metadata| metadata.is_file()) {
        return Err(ManagedError::InvalidRequest);
    }
    Ok(())
}

fn read_regular(path: &Path, limit: usize) -> Result<Vec<u8>, ManagedError> {
    super::profiles::read_regular_file(path, limit as u64).ok_or(ManagedError::InvalidRequest)
}

fn create_private(path: &Path) -> Result<std::fs::File, ManagedError> {
    super::profiles::create_private_file(path).map_err(|_| ManagedError::StorageUnavailable)
}

#[derive(Deserialize)]
struct HookInput {
    session_id: Uuid,
    transcript_path: Option<PathBuf>,
    hook_event_name: String,
}

/// Only the bounded identity projection is retained; no model/tool/body fields
/// from the hook document reach disk or output.
pub fn record(directory: &Path, nonce: &str, reader: impl Read) -> Result<(), ManagedError> {
    validate_directory(directory)?;
    if nonce.len() != 32 || !nonce.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(ManagedError::InvalidRequest);
    }
    let expected = read_regular(&directory.join(NONCE_FILE), 128)?;
    if expected.len() != nonce.len()
        || expected
            .iter()
            .zip(nonce.as_bytes())
            .fold(0u8, |difference, (a, b)| difference | (a ^ b))
            != 0
    {
        return Err(ManagedError::InvalidRequest);
    }
    let mut bytes = Vec::new();
    reader
        .take(MAX_DOCUMENT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ManagedError::InvalidRequest)?;
    if bytes.len() > MAX_DOCUMENT {
        return Err(ManagedError::InvalidRequest);
    }
    let input: HookInput =
        serde_json::from_slice(&bytes).map_err(|_| ManagedError::InvalidRequest)?;
    if input.hook_event_name != "SessionStart" {
        return Err(ManagedError::InvalidRequest);
    }
    let transcript_path = input.transcript_path.ok_or(ManagedError::InvalidRequest)?;
    validate_transcript(&transcript_path)?;
    let record = BoundSession {
        session_id: input.session_id,
        transcript_path,
    };
    let bytes = serde_json::to_vec(&record).map_err(|_| ManagedError::InvalidRequest)?;
    let target = directory.join(format!("{}.json", record.session_id));
    if target.exists() || std::fs::symlink_metadata(&target).is_ok() {
        return if read_regular(&target, MAX_DOCUMENT)? == bytes {
            Ok(())
        } else {
            Err(ManagedError::Conflict)
        };
    }
    let temporary = directory.join(format!("{}.pending", Uuid::new_v4()));
    let result = (|| {
        let mut file = create_private(&temporary)?;
        file.write_all(&bytes)
            .map_err(|_| ManagedError::StorageUnavailable)?;
        file.sync_all()
            .map_err(|_| ManagedError::StorageUnavailable)?;
        match std::fs::hard_link(&temporary, &target) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                if read_regular(&target, MAX_DOCUMENT)? == bytes {
                    Ok(())
                } else {
                    Err(ManagedError::Conflict)
                }
            }
            Err(_) => Err(ManagedError::StorageUnavailable),
        }
    })();
    let _ = std::fs::remove_file(&temporary);
    result
}

/// The hidden hook command is deliberately silent and cannot open a daemon or
/// config store. A blocked stdin writer cannot keep its process alive forever.
pub fn run_record(directory: &Path, nonce: &str) {
    use std::io::IsTerminal;
    if std::io::stdin().is_terminal() {
        return;
    }
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = std::io::stdin()
            .take(MAX_DOCUMENT as u64 + 1)
            .read_to_end(&mut bytes);
        let _ = sender.send(result.map(|_| bytes));
    });
    if let Ok(Ok(bytes)) = receiver.recv_timeout(std::time::Duration::from_secs(1)) {
        let _ = record(directory, nonce, bytes.as_slice());
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn trust_hash_matches_the_installed_codex_normalization() {
        // Verified through actual Codex 0.164 hooks/list for this declaration.
        let mut identity = serde_json::json!({"event_name":"session_start","hooks":[{"type":"command","command":"/usr/bin/true","timeout":600,"async":false}]});
        identity.sort_all_objects();
        use sha2::Digest;
        assert_eq!(
            format!(
                "sha256:{:x}",
                sha2::Sha256::digest(serde_json::to_vec(&identity).unwrap())
            ),
            "sha256:5bcf5439a4b245c57c9e7f3910368077fa8b85b8231cd151f801f58f1414d57e"
        );
    }
}
