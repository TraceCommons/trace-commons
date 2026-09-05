//! Inert packaged-worker integrity checks. Success is NOT launch authorization:
//! OS signature validation, trusted release selection and resource policy are
//! separate gates. Paths are checked, not protected from same-user replacement.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
};

pub const MANIFEST_PATH: &str = "Contents/Resources/Compute/worker-manifest.json";
pub const WORKER_PATH: &str = "Contents/Helpers/holonear";
pub const METAL_ASSET: &str = "mlx.metallib";
const MAX_MANIFEST: u64 = 65_536;
const MAX_ENTRIES: usize = 128;
const MAX_FILE: u64 = 512 * 1024 * 1024;
const MAX_TOTAL: u64 = 1024 * 1024 * 1024;
const MAX_LOAD_COMMANDS: u64 = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ArtifactError {
    #[error("compute-artifact-manifest-invalid")]
    Manifest,
    #[error("compute-artifact-incompatible")]
    Incompatible,
    #[error("compute-artifact-path-invalid")]
    Path,
    #[error("compute-artifact-read-failed")]
    Read,
    #[error("compute-artifact-integrity-mismatch")]
    Integrity,
    #[error("compute-artifact-executable-invalid")]
    Executable,
}

/// Supplied from reviewed release policy, never derived from the manifest being
/// checked. No shipping release policy is supplied by this module.
#[derive(Clone)]
pub struct ArtifactExpectation<'a> {
    /// SHA-256 of the exact manifest bytes, obtained from independent reviewed policy.
    pub manifest_sha256: [u8; 32],
    pub source_revision: &'a str,
    pub compatibility_id: &'a str,
    pub signing_identifier: &'a str,
    pub signing_team: &'a str,
    pub host_target: &'a str,
    pub host_macos: [u16; 3],
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    schema_version: u32,
    source_revision: String,
    target: String,
    backend: String,
    minimum_macos: [u16; 3],
    ipc_version: u32,
    compatibility_id: String,
    signing_identifier: String,
    signing_team: String,
    worker: FilePin,
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct FilePin {
    size_bytes: u64,
    sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Asset {
    relative_path: String,
    size_bytes: u64,
    sha256: String,
}

/// Counts only; deliberately no executable path, launch token or signature claim.
#[derive(Debug, PartialEq, Eq)]
pub struct IntegrityChecked {
    pub asset_count: usize,
    pub checked_bytes: u64,
}

fn lowercase_hex(value: &str, len: usize) -> bool {
    value.len() == len
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
}
fn relative(value: &str) -> bool {
    value.len() <= 512
        && value
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != ".." && label(part))
}
fn valid_pin(size: u64, digest: &str) -> bool {
    size > 0 && size <= MAX_FILE && lowercase_hex(digest, 64)
}

impl Manifest {
    fn parse(bytes: &[u8], expected: &ArtifactExpectation<'_>) -> Result<Self, ArtifactError> {
        if bytes.len() as u64 > MAX_MANIFEST {
            return Err(ArtifactError::Manifest);
        }
        let m: Self = serde_json::from_slice(bytes).map_err(|_| ArtifactError::Manifest)?;
        if m.schema_version != 1
            || !lowercase_hex(&m.source_revision, 40)
            || !label(&m.compatibility_id)
            || !label(&m.signing_identifier)
            || m.signing_team.len() != 10
            || !m
                .signing_team
                .bytes()
                .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
            || !valid_pin(m.worker.size_bytes, &m.worker.sha256)
            || m.assets.len() > MAX_ENTRIES
            || m.minimum_macos[0] < 15
            || m.minimum_macos[1] > 255
            || m.minimum_macos[2] > 255
        {
            return Err(ArtifactError::Manifest);
        }
        if m.target != "aarch64-apple-darwin"
            || expected.host_target != m.target
            || m.backend != "mlx"
            || m.ipc_version != 0
            || m.minimum_macos > expected.host_macos
            || m.source_revision != expected.source_revision
            || m.compatibility_id != expected.compatibility_id
            || m.signing_identifier != expected.signing_identifier
            || m.signing_team != expected.signing_team
        {
            return Err(ArtifactError::Incompatible);
        }
        let mut paths = BTreeSet::new();
        let mut total = m.worker.size_bytes;
        for asset in &m.assets {
            if !relative(&asset.relative_path)
                || !valid_pin(asset.size_bytes, &asset.sha256)
                || !paths.insert(asset.relative_path.to_ascii_lowercase())
            {
                return Err(ArtifactError::Manifest);
            }
            total = total
                .checked_add(asset.size_bytes)
                .ok_or(ArtifactError::Manifest)?;
            if total > MAX_TOTAL {
                return Err(ArtifactError::Manifest);
            }
        }
        if !m
            .assets
            .iter()
            .any(|asset| asset.relative_path == METAL_ASSET)
        {
            return Err(ArtifactError::Manifest);
        }
        Ok(m)
    }
}

/// Reject every symlink below the supplied bundle root, including internal ones.
/// This conservative subset does not support framework symlink layouts yet.
fn regular_file(root: &Path, relative_path: &str) -> Result<File, ArtifactError> {
    let mut path = root.to_path_buf();
    let parts: Vec<_> = relative_path.split('/').collect();
    for (i, part) in parts.iter().enumerate() {
        path.push(part);
        let meta = std::fs::symlink_metadata(&path).map_err(|_| ArtifactError::Read)?;
        if meta.file_type().is_symlink()
            || (i + 1 < parts.len() && !meta.is_dir())
            || (i + 1 == parts.len() && !meta.is_file())
        {
            return Err(ArtifactError::Path);
        }
    }
    File::open(path).map_err(|_| ArtifactError::Read)
}

// Retain only the bounded executable header prefix from the SAME stream that
// is hashed, so path replacement or an in-place rewrite cannot substitute an
// unhashed header for structural inspection.
fn hash_file(
    mut file: File,
    size: u64,
    expected: &str,
    prefix_limit: usize,
) -> Result<Vec<u8>, ArtifactError> {
    if file.metadata().map_err(|_| ArtifactError::Read)?.len() != size {
        return Err(ArtifactError::Integrity);
    }
    let mut prefix = Vec::new();
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 32 * 1024];
    let mut reader = (&mut file).take(size + 1);
    let mut total = 0u64;
    loop {
        let n = reader.read(&mut buffer).map_err(|_| ArtifactError::Read)?;
        if n == 0 {
            break;
        }
        total += n as u64;
        digest.update(&buffer[..n]);
        let keep = n.min(prefix_limit.saturating_sub(prefix.len()));
        prefix.extend_from_slice(&buffer[..keep]);
    }
    if total != size || hex::encode(digest.finalize()) != expected {
        return Err(ArtifactError::Integrity);
    }
    Ok(prefix)
}

/// Bounded thin arm64 Mach-O header inspection, not an executable loader or
/// signature verifier. Fat binaries and legacy minimum-OS commands are refused.
fn macho(mut file: impl Read, minimum: [u16; 3]) -> Result<(), ArtifactError> {
    let mut header = [0u8; 32];
    file.read_exact(&mut header)
        .map_err(|_| ArtifactError::Executable)?;
    let word = |b: &[u8]| u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
    if word(&header[0..4]) != 0xfeedfacf
        || word(&header[4..8]) != 0x0100000c
        || word(&header[12..16]) != 2
    {
        return Err(ArtifactError::Executable);
    }
    let count = word(&header[16..20]) as usize;
    let size = word(&header[20..24]) as usize;
    if size as u64 > MAX_LOAD_COMMANDS || count == 0 || count > size / 8 {
        return Err(ArtifactError::Executable);
    }
    let mut commands = vec![0; size];
    file.read_exact(&mut commands)
        .map_err(|_| ArtifactError::Executable)?;
    let mut offset = 0;
    let mut found = false;
    for _ in 0..count {
        let remaining = commands.get(offset..).ok_or(ArtifactError::Executable)?;
        if remaining.len() < 8 {
            return Err(ArtifactError::Executable);
        }
        let command = word(&remaining[..4]);
        let len = word(&remaining[4..8]) as usize;
        if len < 8 || !len.is_multiple_of(8) || len > remaining.len() {
            return Err(ArtifactError::Executable);
        }
        if command == 0x24 {
            return Err(ArtifactError::Executable);
        }
        if command == 0x32 {
            if found || len < 24 || word(&remaining[8..12]) != 1 {
                return Err(ArtifactError::Executable);
            }
            let encoded =
                ((minimum[0] as u32) << 16) | ((minimum[1] as u32) << 8) | minimum[2] as u32;
            if word(&remaining[12..16]) != encoded
                || 24u64 + 8 * u64::from(word(&remaining[20..24])) != len as u64
            {
                return Err(ArtifactError::Executable);
            }
            found = true;
        }
        offset += len;
    }
    if !found || offset != size {
        return Err(ArtifactError::Executable);
    }
    Ok(())
}

/// The helper and Compute resource directories are closed inventories. Only
/// listed files and the directories necessary to contain them may exist.
fn complete_inventory(root: &Path, manifest: &Manifest) -> Result<(), ArtifactError> {
    let mut files = BTreeSet::from([MANIFEST_PATH.to_string(), WORKER_PATH.to_string()]);
    let mut directories = BTreeSet::from([
        "Contents/Helpers".to_string(),
        "Contents/Resources/Compute".to_string(),
        "Contents/Resources/Compute/assets".to_string(),
    ]);
    for asset in &manifest.assets {
        let path = format!("Contents/Resources/Compute/assets/{}", asset.relative_path);
        let mut parent = Path::new(&path).parent();
        while let Some(dir) = parent {
            let name = dir.to_str().ok_or(ArtifactError::Path)?;
            if name == "Contents/Resources/Compute" {
                break;
            }
            directories.insert(name.to_string());
            parent = dir.parent();
        }
        files.insert(path);
    }
    let mut pending = vec![
        "Contents/Helpers".to_string(),
        "Contents/Resources/Compute".to_string(),
    ];
    let mut seen = BTreeSet::new();
    while let Some(directory) = pending.pop() {
        let metadata =
            std::fs::symlink_metadata(root.join(&directory)).map_err(|_| ArtifactError::Read)?;
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err(ArtifactError::Path);
        }
        for entry in std::fs::read_dir(root.join(&directory)).map_err(|_| ArtifactError::Read)? {
            let entry = entry.map_err(|_| ArtifactError::Read)?;
            let name = entry.file_name();
            let name = name.to_str().ok_or(ArtifactError::Path)?;
            let relative = format!("{directory}/{name}");
            let kind = entry.file_type().map_err(|_| ArtifactError::Read)?;
            if kind.is_dir() && directories.contains(&relative) {
                pending.push(relative);
            } else if kind.is_file() && files.contains(&relative) {
                seen.insert(relative);
            } else {
                return Err(ArtifactError::Path);
            }
        }
    }
    if seen != files {
        return Err(ArtifactError::Read);
    }
    Ok(())
}

/// Read-only integrity inventory. Caller must not convert success into permission
/// to spawn: backend metadata can lie, and no OS signature is checked here.
pub fn check_integrity(
    bundle: &Path,
    expected: &ArtifactExpectation<'_>,
) -> Result<IntegrityChecked, ArtifactError> {
    if !bundle.is_absolute() {
        return Err(ArtifactError::Path);
    }
    let meta = std::fs::symlink_metadata(bundle).map_err(|_| ArtifactError::Read)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(ArtifactError::Path);
    }
    let root: PathBuf = bundle.canonicalize().map_err(|_| ArtifactError::Path)?;
    let mut bytes = Vec::new();
    regular_file(&root, MANIFEST_PATH)?
        .take(MAX_MANIFEST + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| ArtifactError::Read)?;
    if bytes.len() as u64 > MAX_MANIFEST {
        return Err(ArtifactError::Manifest);
    }
    if <[u8; 32]>::from(Sha256::digest(&bytes)) != expected.manifest_sha256 {
        return Err(ArtifactError::Integrity);
    }
    let m = Manifest::parse(&bytes, expected)?;
    complete_inventory(&root, &m)?;
    let worker = regular_file(&root, WORKER_PATH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if worker
            .metadata()
            .map_err(|_| ArtifactError::Read)?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(ArtifactError::Executable);
        }
    }
    let header = hash_file(
        worker,
        m.worker.size_bytes,
        &m.worker.sha256,
        (MAX_LOAD_COMMANDS + 32) as usize,
    )?;
    macho(header.as_slice(), m.minimum_macos)?;
    let mut total = m.worker.size_bytes;
    for asset in &m.assets {
        let path = format!("Contents/Resources/Compute/assets/{}", asset.relative_path);
        let file = regular_file(&root, &path)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if file
                .metadata()
                .map_err(|_| ArtifactError::Read)?
                .permissions()
                .mode()
                & 0o111
                != 0
            {
                return Err(ArtifactError::Path);
            }
        }
        hash_file(file, asset.size_bytes, &asset.sha256, 0)?;
        total += asset.size_bytes;
    }
    Ok(IntegrityChecked {
        asset_count: m.assets.len(),
        checked_bytes: total,
    })
}

#[cfg(test)]
#[path = "artifact_tests.rs"]
mod tests;
