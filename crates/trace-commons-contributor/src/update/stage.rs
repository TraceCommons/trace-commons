//! A verified update, parked on disk until something applies it.
//!
//! Staging exists because the headless daemon has no surface to prompt in.
//! A verified update waits here and is applied at the daemon's next start, or
//! immediately by `trace-commons-contributor update`. Nothing is ever swapped
//! silently underneath a running process.
//!
//! The directory sits beside the installed binary so that applying it is a
//! same-filesystem rename, and the record carries the digest so the staged
//! bytes can be re-verified at apply time: a staged update may sit across a
//! reboot, and what is applied must be what was verified.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

pub const STAGED_UPDATE_SCHEMA: &str = "trace_commons.staged_update.v1";
pub const STAGE_DIR_NAME: &str = ".trace-commons-update";
pub const STAGE_RECORD_FILE: &str = "staged.json";
pub const STAGED_BINARY_FILE: &str = "staged-binary";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StagedUpdate {
    pub schema_version: String,
    /// The version the staged binary reports, from the verified manifest.
    pub version: String,
    /// Lowercase hex sha256 of the staged binary, from the verified manifest.
    pub sha256: String,
    pub staged_at: String,
}

#[derive(Debug, thiserror::Error)]
pub enum StageError {
    #[error("update_stage_io_failed")]
    Io,
    #[error("update_stage_record_malformed")]
    Malformed,
    #[error("update_stage_unknown_schema")]
    UnknownSchema,
}

/// The staging directory for a given installed binary.
pub fn stage_dir(target_exe: &Path) -> PathBuf {
    target_exe
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(STAGE_DIR_NAME)
}

/// Where the downloaded, verified binary waits.
pub fn staged_binary_path(target_exe: &Path) -> PathBuf {
    stage_dir(target_exe).join(STAGED_BINARY_FILE)
}

/// Create the staging directory and return it.
pub fn prepare(target_exe: &Path) -> Result<PathBuf, StageError> {
    let dir = stage_dir(target_exe);
    std::fs::create_dir_all(&dir).map_err(|_| StageError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| StageError::Io)?;
    }
    Ok(dir)
}

/// Record what is staged. Written after the binary, so a record that exists
/// always describes bytes that are already on disk.
pub fn write_record(target_exe: &Path, staged: &StagedUpdate) -> Result<(), StageError> {
    let body = serde_json::to_vec_pretty(staged).map_err(|_| StageError::Malformed)?;
    std::fs::write(stage_dir(target_exe).join(STAGE_RECORD_FILE), body).map_err(|_| StageError::Io)
}

/// What is staged, if anything.
///
/// A record that cannot be parsed is an error, not a `None`: silently
/// ignoring it would leave a staged binary on disk that nothing ever applies
/// or clears.
pub fn read_record(target_exe: &Path) -> Result<Option<StagedUpdate>, StageError> {
    let path = stage_dir(target_exe).join(STAGE_RECORD_FILE);
    let body = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(StageError::Io),
    };
    let record: StagedUpdate = serde_json::from_slice(&body).map_err(|_| StageError::Malformed)?;
    if record.schema_version != STAGED_UPDATE_SCHEMA {
        return Err(StageError::UnknownSchema);
    }
    Ok(Some(record))
}

/// Forget any staged update. Idempotent.
pub fn clear(target_exe: &Path) -> Result<(), StageError> {
    for path in [
        stage_dir(target_exe).join(STAGE_RECORD_FILE),
        staged_binary_path(target_exe),
    ] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StageError::Io),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record() -> StagedUpdate {
        StagedUpdate {
            schema_version: STAGED_UPDATE_SCHEMA.to_string(),
            version: "0.2.0".to_string(),
            sha256: "a".repeat(64),
            staged_at: "2026-08-17T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn the_stage_directory_is_beside_the_target_so_a_rename_stays_on_one_filesystem() {
        let target = std::path::Path::new("/home/ada/.local/bin/trace-commons-contributor");
        assert_eq!(
            stage_dir(target),
            std::path::Path::new("/home/ada/.local/bin").join(STAGE_DIR_NAME)
        );
        assert_eq!(
            staged_binary_path(target),
            stage_dir(target).join(STAGED_BINARY_FILE)
        );
    }

    #[test]
    fn nothing_staged_reads_back_as_none() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        assert!(read_record(&target).unwrap().is_none());
    }

    #[test]
    fn a_written_record_reads_back_unchanged() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        prepare(&target).unwrap();
        write_record(&target, &record()).unwrap();
        let back = read_record(&target).unwrap().expect("a record");
        assert_eq!(back.version, "0.2.0");
        assert_eq!(back.sha256, "a".repeat(64));
    }

    #[test]
    fn clear_removes_the_record_and_the_staged_binary() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        prepare(&target).unwrap();
        std::fs::write(staged_binary_path(&target), b"new").unwrap();
        write_record(&target, &record()).unwrap();

        clear(&target).unwrap();

        assert!(read_record(&target).unwrap().is_none());
        assert!(!staged_binary_path(&target).exists());
    }

    #[test]
    fn clear_on_a_clean_install_is_not_an_error() {
        let d = tempfile::tempdir().unwrap();
        clear(&d.path().join("trace-commons-contributor")).expect("idempotent");
    }

    #[test]
    fn a_record_from_an_unknown_schema_is_refused_rather_than_guessed_at() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        prepare(&target).unwrap();
        let mut r = record();
        r.schema_version = "trace_commons.staged_update.v2".to_string();
        std::fs::write(
            stage_dir(&target).join(STAGE_RECORD_FILE),
            serde_json::to_vec(&r).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            read_record(&target).unwrap_err(),
            StageError::UnknownSchema
        ));
    }

    #[test]
    fn a_corrupt_record_is_refused_rather_than_ignored() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        prepare(&target).unwrap();
        std::fs::write(stage_dir(&target).join(STAGE_RECORD_FILE), b"{ not json").unwrap();
        assert!(matches!(
            read_record(&target).unwrap_err(),
            StageError::Malformed
        ));
    }
}
