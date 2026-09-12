//! Explicit local mission draft intake and inbox. No source URL is fetched and
//! no proposal is executed, published, funded, or treated as verified.

use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Result, anyhow, bail};
use serde::{Deserialize, Serialize};
use trace_commons_protocol::mission_draft::{
    DraftStatus, MAX_MISSION_DRAFT_BYTES, MissionDraft, MissionDraftReview,
};

const INBOX_VERSION: u32 = 1;
const MAX_INBOX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_DRAFTS: usize = 256;
const INDEX_FILE: &str = "mission-drafts.json";

#[derive(Debug, Serialize)]
pub struct StoredMissionDraft {
    pub id: String,
    pub proposal: MissionDraft,
    pub review: MissionDraftReview,
}

#[derive(Debug, Serialize)]
pub struct MissionDraftSummary {
    pub id: String,
    pub source_count: usize,
    pub status: DraftStatus,
}

#[derive(Debug, Serialize)]
pub struct MissionDraftImport {
    pub id: String,
    pub review: MissionDraftReview,
    pub inserted: bool,
}

#[derive(Debug, Serialize)]
pub struct MissionDraftDelete {
    pub id: String,
    pub deleted: bool,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct InboxIndex {
    version: u32,
    drafts: BTreeMap<String, MissionDraft>,
}

pub struct MissionDraftInbox {
    dir: PathBuf,
}

struct InboxLock(File);
impl Drop for InboxLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

impl MissionDraftInbox {
    /// Select an inbox location without creating any local state.
    pub fn at(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    pub fn resolve(dir: Option<&Path>) -> Result<Self> {
        let dir = match dir {
            Some(dir) => dir.to_path_buf(),
            None => dirs::data_local_dir()
                .ok_or_else(|| anyhow!("mission-draft-store-unavailable"))?
                .join("trace-commons/mission-drafts"),
        };
        Ok(Self::at(&dir))
    }

    pub fn import_file(&self, path: &Path) -> Result<MissionDraftImport> {
        let proposal = read_file(path)?;
        let id = proposal.review()?.proposal_sha256;
        let (_lock, mut index) = self.locked(true)?;
        let inserted = !index.drafts.contains_key(&id);
        if inserted {
            if index.drafts.len() >= MAX_DRAFTS {
                bail!("mission-draft-store-full");
            }
            index.drafts.insert(id.clone(), proposal);
            self.save(&index)?;
        }
        let proposal = index
            .drafts
            .remove(&id)
            .ok_or_else(|| anyhow!("mission-draft-store-invalid"))?;
        let review = proposal.review()?;
        Ok(MissionDraftImport {
            id,
            review,
            inserted,
        })
    }

    pub fn list(&self) -> Result<Vec<MissionDraftSummary>> {
        let Some((_lock, index)) = self.existing_locked()? else {
            return Ok(Vec::new());
        };
        index
            .drafts
            .into_iter()
            .map(|(id, proposal)| {
                let review = proposal.review()?;
                Ok(MissionDraftSummary {
                    id,
                    source_count: proposal.source_urls.len(),
                    status: review.status,
                })
            })
            .collect()
    }

    pub fn show(&self, id: &str) -> Result<StoredMissionDraft> {
        validate_id(id)?;
        let Some((_lock, mut index)) = self.existing_locked()? else {
            bail!("mission-draft-not-found");
        };
        let proposal = index
            .drafts
            .remove(id)
            .ok_or_else(|| anyhow!("mission-draft-not-found"))?;
        stored(id.into(), proposal)
    }

    pub fn delete(&self, id: &str) -> Result<MissionDraftDelete> {
        validate_id(id)?;
        let Some((_lock, mut index)) = self.existing_locked()? else {
            bail!("mission-draft-not-found");
        };
        if index.drafts.remove(id).is_none() {
            bail!("mission-draft-not-found");
        }
        self.save(&index)?;
        Ok(MissionDraftDelete {
            id: id.into(),
            deleted: true,
        })
    }

    fn existing_locked(&self) -> Result<Option<(InboxLock, InboxIndex)>> {
        match fs::symlink_metadata(&self.dir) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => bail!("mission-draft-store-unavailable"),
            Ok(metadata) if !metadata.is_dir() || metadata.file_type().is_symlink() => {
                bail!("mission-draft-store-unavailable")
            }
            Ok(_) => self.locked(false).map(Some),
        }
    }

    fn locked(&self, create: bool) -> Result<(InboxLock, InboxIndex)> {
        reject_leaf_symlink(&self.dir)?;
        if create {
            create_private_dir(&self.dir)?;
        }
        reject_leaf_symlink(&self.dir)?;
        let dir = self
            .dir
            .canonicalize()
            .map_err(|_| anyhow!("mission-draft-store-unavailable"))?;
        reject_symlinks(&dir)?;
        require_private_dir(&dir)?;
        let lock_path = dir.join("mission-drafts.lock");
        reject_symlinks(&lock_path)?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options
            .open(lock_path)
            .map_err(|_| anyhow!("mission-draft-store-unavailable"))?;
        file.try_lock()
            .map_err(|_| anyhow!("mission-draft-store-busy"))?;
        let lock = InboxLock(file);
        let path = dir.join(INDEX_FILE);
        reject_symlinks(&path)?;
        let index = match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => InboxIndex {
                version: INBOX_VERSION,
                drafts: BTreeMap::new(),
            },
            Err(_) => bail!("mission-draft-store-unreadable"),
            Ok(metadata) if !metadata.is_file() => bail!("mission-draft-store-invalid"),
            Ok(_) => serde_json::from_slice(&bounded_read(&path)?)
                .map_err(|_| anyhow!("mission-draft-store-invalid"))?,
        };
        validate_index(&index)?;
        Ok((lock, index))
    }

    fn save(&self, index: &InboxIndex) -> Result<()> {
        let bytes =
            serde_json::to_vec(index).map_err(|_| anyhow!("mission-draft-store-invalid"))?;
        if bytes.len() as u64 > MAX_INBOX_BYTES {
            bail!("mission-draft-store-full");
        }
        let dir = self
            .dir
            .canonicalize()
            .map_err(|_| anyhow!("mission-draft-store-write-failed"))?;
        reject_symlinks(&dir).map_err(|_| anyhow!("mission-draft-store-write-failed"))?;
        crate::config::write_atomic_0600(&dir, &dir.join(INDEX_FILE), &bytes)
            .map_err(|_| anyhow!("mission-draft-store-write-failed"))
    }
}

pub fn review_file(path: &Path) -> Result<MissionDraftReview> {
    Ok(read_file(path)?.review()?)
}

fn read_file(path: &Path) -> Result<MissionDraft> {
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
    Ok(MissionDraft::parse(&bytes)?)
}

fn stored(id: String, proposal: MissionDraft) -> Result<StoredMissionDraft> {
    let review = proposal.review()?;
    if review.proposal_sha256 != id {
        bail!("mission-draft-store-invalid");
    }
    Ok(StoredMissionDraft {
        id,
        proposal,
        review,
    })
}

fn validate_index(index: &InboxIndex) -> Result<()> {
    if index.version != INBOX_VERSION {
        bail!("mission-draft-store-version-unsupported");
    }
    if index.drafts.len() > MAX_DRAFTS {
        bail!("mission-draft-store-invalid");
    }
    for (id, proposal) in &index.drafts {
        validate_id(id).map_err(|_| anyhow!("mission-draft-store-invalid"))?;
        let review = proposal
            .review()
            .map_err(|_| anyhow!("mission-draft-store-invalid"))?;
        if review.proposal_sha256 != *id {
            bail!("mission-draft-store-invalid");
        }
    }
    Ok(())
}

fn validate_id(id: &str) -> Result<()> {
    if id.len() != 64
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        bail!("mission-draft-field-invalid");
    }
    Ok(())
}

fn bounded_read(path: &Path) -> Result<Vec<u8>> {
    let file = File::open(path).map_err(|_| anyhow!("mission-draft-store-unreadable"))?;
    if file
        .metadata()
        .map_err(|_| anyhow!("mission-draft-store-unreadable"))?
        .len()
        > MAX_INBOX_BYTES
    {
        bail!("mission-draft-store-invalid");
    }
    let mut bytes = Vec::new();
    file.take(MAX_INBOX_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| anyhow!("mission-draft-store-unreadable"))?;
    if bytes.len() as u64 > MAX_INBOX_BYTES {
        bail!("mission-draft-store-invalid");
    }
    Ok(bytes)
}

fn reject_symlinks(path: &Path) -> Result<()> {
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                bail!("mission-draft-store-unavailable")
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => bail!("mission-draft-store-unavailable"),
        }
    }
    Ok(())
}

fn reject_leaf_symlink(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            bail!("mission-draft-store-unavailable")
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => bail!("mission-draft-store-unavailable"),
    }
}

fn create_private_dir(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true).mode(0o700);
        builder
            .create(path)
            .map_err(|_| anyhow!("mission-draft-store-unavailable"))?;
    }
    #[cfg(not(unix))]
    fs::create_dir_all(path).map_err(|_| anyhow!("mission-draft-store-unavailable"))?;
    Ok(())
}

fn require_private_dir(_path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if fs::metadata(_path)
            .map_err(|_| anyhow!("mission-draft-store-unavailable"))?
            .permissions()
            .mode()
            & 0o077
            != 0
        {
            bail!("mission-draft-store-requires-private-directory");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../trace-commons-protocol/tests/fixtures/mission-draft.json")
    }

    #[test]
    fn empty_reads_do_not_create_state_and_missing_ids_are_fixed_errors() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("absent");
        let inbox = MissionDraftInbox::at(&path);
        assert!(inbox.list().unwrap().is_empty());
        assert!(!path.exists());
        let id = "0".repeat(64);
        assert_eq!(
            inbox.show(&id).unwrap_err().to_string(),
            "mission-draft-not-found"
        );
        assert_eq!(
            inbox.delete(&id).unwrap_err().to_string(),
            "mission-draft-not-found"
        );
        assert!(!path.exists());
    }

    #[test]
    fn import_deduplicates_by_digest_and_delete_leaves_source_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("proposal.json");
        let original = fs::read(fixture()).unwrap();
        fs::write(&source, &original).unwrap();
        let inbox = MissionDraftInbox::at(&root.path().join("inbox"));
        let first = inbox.import_file(&source).unwrap();
        let second = inbox.import_file(&source).unwrap();
        assert!(first.inserted);
        assert!(!second.inserted);
        assert_eq!(first.id, second.id);
        assert_eq!(inbox.list().unwrap().len(), 1);
        assert_eq!(
            inbox.show(&first.id).unwrap().review.proposal_sha256,
            first.id
        );
        assert!(inbox.delete(&first.id).unwrap().deleted);
        assert!(inbox.list().unwrap().is_empty());
        assert_eq!(fs::read(source).unwrap(), original);
    }

    #[test]
    fn corrupted_future_and_digest_mismatched_stores_are_refused() {
        let root = tempfile::tempdir().unwrap();
        let dir = root.path().join("inbox");
        create_private_dir(&dir).unwrap();
        fs::write(dir.join(INDEX_FILE), b"not json").unwrap();
        let inbox = MissionDraftInbox::at(&dir);
        assert_eq!(
            inbox.list().unwrap_err().to_string(),
            "mission-draft-store-invalid"
        );
        fs::write(dir.join(INDEX_FILE), br#"{"version":2,"drafts":{}}"#).unwrap();
        assert_eq!(
            inbox.list().unwrap_err().to_string(),
            "mission-draft-store-version-unsupported"
        );
        let bad = InboxIndex {
            version: INBOX_VERSION,
            drafts: BTreeMap::from([("0".repeat(64), read_file(&fixture()).unwrap())]),
        };
        fs::write(dir.join(INDEX_FILE), serde_json::to_vec(&bad).unwrap()).unwrap();
        assert_eq!(
            inbox.list().unwrap_err().to_string(),
            "mission-draft-store-invalid"
        );
    }
}
