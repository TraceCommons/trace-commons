//! Explicit local draft intake. No source URL is fetched and no code executes.
use std::io::Read;
use std::path::Path;

use anyhow::{Result, anyhow, bail};
use trace_commons_protocol::mission_draft::{
    MAX_MISSION_DRAFT_BYTES, MissionDraft, MissionDraftReview,
};

pub fn review_file(path: &Path) -> Result<MissionDraftReview> {
    let file = crate::evidence_import::open_import_file(path)
        .map_err(|_| anyhow!("mission-draft-file-unreadable"))?;
    let metadata = file
        .metadata()
        .map_err(|_| anyhow!("mission-draft-file-unreadable"))?;
    if !metadata.is_file() {
        bail!("mission-draft-file-not-regular");
    }
    if metadata.len() > MAX_MISSION_DRAFT_BYTES as u64 {
        bail!("mission-draft-too-large");
    }
    let mut bytes = Vec::new();
    file.take(MAX_MISSION_DRAFT_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("mission-draft-file-unreadable"))?;
    Ok(MissionDraft::parse(&bytes)?.review()?)
}
