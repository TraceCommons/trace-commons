use super::*;
use std::io::Read;
use std::path::{Path, PathBuf};
const MAX_CACHE_BYTES: u64 = 512 * 1024;
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Envelope {
    schema_version: u32,
    profile_digest: String,
    raw: String,
    content_digest: String,
}
/// The highest revision this environment has accepted, and the digest of
/// its signed payload. Scoped to the environment, not the build profile, so
/// a release that changes the profile cannot reopen a rollback window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Watermark {
    pub revision: u64,
    pub payload_digest: String,
}
pub(super) struct Cache {
    dir: PathBuf,
    path: PathBuf,
    watermark_path: PathBuf,
    identity: String,
}
fn read_bounded(path: &Path) -> Option<Vec<u8>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(MAX_CACHE_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() as u64 <= MAX_CACHE_BYTES).then_some(bytes)
}
impl Cache {
    pub fn new(dir: &Path, identity: &str, environment: Environment) -> Self {
        Self {
            dir: dir.to_owned(),
            path: dir.join(format!("feature-flags-{identity}.json")),
            watermark_path: dir.join(format!(
                "feature-flags-{}-watermark.json",
                environment.as_str()
            )),
            identity: identity.into(),
        }
    }
    /// The cached document, its manifest and its payload digest.
    pub fn load(&self, profile: &Profile) -> Option<(Manifest, Vec<u8>, String)> {
        let envelope: Envelope = serde_json::from_slice(&read_bounded(&self.path)?).ok()?;
        if envelope.schema_version != 1
            || envelope.profile_digest != self.identity
            || digest(envelope.raw.as_bytes()) != envelope.content_digest
        {
            return None;
        }
        let raw = envelope.raw.into_bytes();
        let (manifest, payload_digest) = decode_manifest(&raw, profile).ok()?;
        Some((manifest, raw, payload_digest))
    }
    pub fn save(&self, raw: &[u8]) -> anyhow::Result<()> {
        let data = serde_json::to_vec(&Envelope {
            schema_version: 1,
            profile_digest: self.identity.clone(),
            raw: String::from_utf8(raw.to_vec())?,
            content_digest: digest(raw),
        })?;
        crate::config::write_atomic_0600(&self.dir, &self.path, &data)
    }
    pub fn load_watermark(&self) -> Option<Watermark> {
        serde_json::from_slice(&read_bounded(&self.watermark_path)?).ok()
    }
    pub fn save_watermark(&self, watermark: &Watermark) -> anyhow::Result<()> {
        crate::config::write_atomic_0600(
            &self.dir,
            &self.watermark_path,
            &serde_json::to_vec(watermark)?,
        )
    }
    /// Remove the cached documents of other build profiles. Best effort:
    /// they can no longer be loaded, so leaving one behind costs disk only.
    pub fn remove_other_profiles(&self) {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(identity) = name
                .strip_prefix("feature-flags-")
                .and_then(|rest| rest.strip_suffix(".json"))
            else {
                continue;
            };
            if identity != self.identity
                && identity.len() == 64
                && identity.bytes().all(|b| b.is_ascii_hexdigit())
            {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
}
