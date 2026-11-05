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
pub(super) struct Cache {
    dir: PathBuf,
    path: PathBuf,
    identity: String,
}
impl Cache {
    pub fn new(dir: &Path, identity: &str) -> Self {
        Self {
            dir: dir.to_owned(),
            path: dir.join(format!("feature-flags-{identity}.json")),
            identity: identity.into(),
        }
    }
    pub fn load(&self, environment: Environment) -> Option<(Manifest, Vec<u8>)> {
        let mut bytes = Vec::new();
        std::fs::File::open(&self.path)
            .ok()?
            .take(MAX_CACHE_BYTES + 1)
            .read_to_end(&mut bytes)
            .ok()?;
        if bytes.len() as u64 > MAX_CACHE_BYTES {
            return None;
        }
        let envelope: Envelope = serde_json::from_slice(&bytes).ok()?;
        if envelope.schema_version != 1
            || envelope.profile_digest != self.identity
            || digest(envelope.raw.as_bytes()) != envelope.content_digest
        {
            return None;
        }
        let raw = envelope.raw.into_bytes();
        Some((decode_manifest(&raw, environment).ok()?, raw))
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
}
