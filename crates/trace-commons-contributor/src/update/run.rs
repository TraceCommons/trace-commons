//! The update flow, end to end.
//!
//! Detect who owns the bytes, fetch and verify the manifest, compare
//! versions, download and verify the artifact, quiesce a running daemon, and
//! only then swap. Every step fails closed, and nothing unverified is ever
//! written where it could be executed.

use std::collections::BTreeMap;
use std::path::Path;

use super::fetch::VerifiedArtifact;
use super::manifest::PlatformArtifact;
use super::source::InstallSource;
use super::{endpoint, fetch, manifest, source, stage, swap, version};
// Only the Windows path verifies a code signature, and an unconditional
// import would be an unused-import warning everywhere else -- which CI turns
// into an error.
#[cfg(windows)]
use super::authenticode;
use crate::config::ConfigStore;

/// How long the updater is prepared to wait for a running daemon to drain.
const QUIESCE_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdateMode {
    /// Verify and park. The daemon applies it at its next start.
    Stage,
    /// Verify and swap now.
    Apply,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateOutcome {
    /// Winget owns these bytes. Nothing was installed, by design; the
    /// contributor is told to run `winget upgrade` instead.
    DeferredToWinget,
    /// The bytes are somewhere we do not recognize as ours (Homebrew, a
    /// distro package, a read-only Nix store path, `install.sh --dir`).
    /// Nothing was installed: we do not know we placed these bytes, so we
    /// do not touch them. See `source::InstallSource::Unrecognized`.
    DeferredUnrecognized,
    /// The running version is already at or past what is published.
    UpToDate {
        version: String,
    },
    /// The manifest advertises no artifact for this platform, which is the
    /// normal state when that platform's build job failed.
    NoArtifactForPlatform,
    Staged {
        version: String,
    },
    Applied {
        version: String,
    },
    /// Verified and staged, but a running daemon would not drain in time.
    /// Nothing was swapped; the next attempt picks the staged update up.
    QuiesceTimedOutStaged {
        version: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum UpdateError {
    #[error("update_source_failed")]
    Source,
    #[error("update_endpoint_failed")]
    Endpoint,
    #[error("update_manifest_failed")]
    Manifest,
    #[error("update_version_failed")]
    Version,
    #[error("update_fetch_failed")]
    Fetch,
    #[error("update_authenticode_failed")]
    Authenticode,
    #[error("update_stage_failed")]
    Stage,
    #[error("update_swap_failed")]
    Swap,
}

/// The artifact for one platform slug, if the manifest carries one.
fn select_artifact<'a>(
    platforms: &'a BTreeMap<String, PlatformArtifact>,
    slug: &str,
) -> Option<&'a PlatformArtifact> {
    platforms.get(slug)
}

/// Run the whole flow.
///
/// `store` is used only to reach a running daemon's socket; no configuration
/// influences whether or what this updates.
pub async fn check_and_install(
    store: &ConfigStore,
    mode: UpdateMode,
) -> Result<UpdateOutcome, UpdateError> {
    let (install_source, exe) = source::detect().map_err(|_| UpdateError::Source)?;
    // The rule is that whoever installed the binary owns replacing it. Both
    // defer arms return before anything is fetched -- not even a manifest
    // check -- so a deferred install makes no network request whatsoever and
    // cannot reach the swap below. `SelfManaged` is the only source that
    // continues past this match.
    match install_source {
        InstallSource::WingetManaged => return Ok(UpdateOutcome::DeferredToWinget),
        InstallSource::Unrecognized => return Ok(UpdateOutcome::DeferredUnrecognized),
        InstallSource::SelfManaged => {}
    }

    let public_key = endpoint::manifest_public_key().map_err(|_| UpdateError::Endpoint)?;
    let slug = endpoint::platform_slug().map_err(|_| UpdateError::Endpoint)?;
    let client = fetch::http_client().map_err(|_| UpdateError::Fetch)?;

    let manifest_bytes =
        fetch::fetch_capped(&client, endpoint::MANIFEST_URL, fetch::MAX_MANIFEST_BYTES)
            .await
            .map_err(|_| UpdateError::Fetch)?;
    let signature_bytes = fetch::fetch_capped(
        &client,
        endpoint::MANIFEST_SIG_URL,
        fetch::MAX_SIGNATURE_BYTES,
    )
    .await
    .map_err(|_| UpdateError::Fetch)?;
    let signature = String::from_utf8(signature_bytes).map_err(|_| UpdateError::Manifest)?;

    // Signature over the exact bytes first; nothing is parsed before it
    // verifies.
    let published = manifest::verify_manifest(&manifest_bytes, &signature, &public_key)
        .map_err(|_| UpdateError::Manifest)?;

    let current = env!("CARGO_PKG_VERSION");
    if !version::is_newer(current, &published.version).map_err(|_| UpdateError::Version)? {
        return Ok(UpdateOutcome::UpToDate {
            version: current.to_string(),
        });
    }

    let artifact = match select_artifact(&published.platforms, slug) {
        Some(a) => a,
        None => return Ok(UpdateOutcome::NoArtifactForPlatform),
    };

    stage::prepare(&exe).map_err(|_| UpdateError::Stage)?;
    let staged_path = stage::staged_binary_path(&exe);
    let verified = fetch::download_verified(&client, artifact, &staged_path)
        .await
        .map_err(|_| UpdateError::Fetch)?;

    #[cfg(windows)]
    {
        // The same two questions install.ps1 asks: is the Authenticode
        // status Valid, and is the signer us.
        if authenticode::verify(verified.path()).is_err() {
            let _ = stage::clear(&exe);
            return Err(UpdateError::Authenticode);
        }
    }
    // `verified` proved the bytes at `staged_path` matched the signed
    // manifest at the moment of download. Nothing below reads it again --
    // the record below carries the same digest so a later `apply_staged`
    // (possibly a different process, possibly after a reboot) re-checks it
    // against the bytes actually on disk rather than trusting this value.
    drop(verified);

    stage::write_record(
        &exe,
        &stage::StagedUpdate {
            schema_version: stage::STAGED_UPDATE_SCHEMA.to_string(),
            version: published.version.clone(),
            sha256: artifact.sha256.to_lowercase(),
            staged_at: chrono::Utc::now().to_rfc3339(),
        },
    )
    .map_err(|_| UpdateError::Stage)?;

    if mode == UpdateMode::Stage {
        return Ok(UpdateOutcome::Staged {
            version: published.version,
        });
    }

    // A running daemon may be mid-upload. Ask it to drain and park; if it
    // will not, the update stays staged and this returns without swapping.
    if !quiesce_running_daemon(store) {
        return Ok(UpdateOutcome::QuiesceTimedOutStaged {
            version: published.version,
        });
    }

    match apply_staged(&exe)? {
        Some(applied) => {
            // Ask the daemon to stop, so a service manager restarts it into
            // the binary that was just installed. Without this the swapped
            // file would not take effect until the next natural restart.
            let _ = crate::daemon::run_blocking(|| {
                crate::daemon::client::try_call(store, "shutdown", &serde_json::json!({}))
            });
            Ok(UpdateOutcome::Applied { version: applied })
        }
        // The record vanished between staging and applying: treat it as
        // staged rather than claiming an install that did not happen.
        None => Ok(UpdateOutcome::Staged {
            version: published.version,
        }),
    }
}

/// Ask a running daemon to park its upload queue. `true` when there is
/// nothing running, or when it parked; `false` when it refused or timed out.
fn quiesce_running_daemon(store: &ConfigStore) -> bool {
    let response = crate::daemon::run_blocking(|| {
        crate::daemon::client::try_call(
            store,
            "quiesce",
            &serde_json::json!({ "timeout_secs": QUIESCE_TIMEOUT_SECS }),
        )
    });
    match response {
        // No daemon is running: there is nothing to drain.
        Ok(None) => true,
        Ok(Some(r)) => r.error.is_none(),
        // A daemon that accepted the connection and then failed is not a
        // daemon we may assume has parked.
        Err(_) => false,
    }
}

/// The platform's pre-swap signature check on a staged binary.
///
/// On Windows this is a real Authenticode verification, held to the same
/// standard as `install.ps1` -- see `authenticode`'s module doc for why there
/// is nothing to stand in its place off Windows. Kept as its own function,
/// separate from `apply_staged`'s control flow, for the same reason
/// `authenticode::interpret` is kept separate from the platform call it
/// backs: so the decision of whether a staged binary may be applied is
/// swappable in a test without touching the swap logic around it.
#[cfg(windows)]
fn verify_staged_signature(path: &Path) -> Result<(), UpdateError> {
    authenticode::verify(path).map_err(|_| UpdateError::Authenticode)
}

#[cfg(not(windows))]
fn verify_staged_signature(_path: &Path) -> Result<(), UpdateError> {
    Ok(())
}

/// Apply a staged update to `target_exe`, if one is staged and still valid.
///
/// Returns the version installed, or `None` when nothing was staged. Every
/// check the download performed is performed again here: a staged update may
/// have sat on disk across a reboot, and what is applied must be what was
/// verified. In particular, `VerifiedArtifact` is never rebuilt from the
/// staged path until the digest of the bytes actually on disk has been
/// re-checked against the recorded digest in this function -- see
/// `fetch::VerifiedArtifact::verified_from_stage`.
pub fn apply_staged(target_exe: &Path) -> Result<Option<String>, UpdateError> {
    apply_staged_with_verifier(target_exe, verify_staged_signature)
}

/// `apply_staged`, with the platform signature check passed in rather than
/// called directly, so a test can substitute a stub verifier and exercise
/// the refusal path (staged bytes rejected, nothing swapped, staging
/// cleared) on any platform -- not only on a Windows box where the real
/// Authenticode call can run. Production always calls this through
/// `apply_staged`, which supplies the real `verify_staged_signature`; this
/// function is not part of the crate's public API.
fn apply_staged_with_verifier(
    target_exe: &Path,
    verify: impl Fn(&Path) -> Result<(), UpdateError>,
) -> Result<Option<String>, UpdateError> {
    let record = match stage::read_record(target_exe) {
        Ok(Some(r)) => r,
        Ok(None) => return Ok(None),
        // An unreadable record leaves bytes on disk nothing would ever apply
        // or clear. Clear them.
        Err(_) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Stage);
        }
    };

    let staged_path = stage::staged_binary_path(target_exe);
    let bytes = match std::fs::read(&staged_path) {
        Ok(b) => b,
        Err(_) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Stage);
        }
    };
    // Re-verify before anything else: these bytes were not verified by this
    // process, and a staging directory is writable by anything that can
    // write where the installed binary lives.
    if !record
        .sha256
        .eq_ignore_ascii_case(&fetch::sha256_hex(&bytes))
    {
        let _ = stage::clear(target_exe);
        return Err(UpdateError::Fetch);
    }

    let current = env!("CARGO_PKG_VERSION");
    match version::is_newer(current, &record.version) {
        Ok(true) => {}
        // Someone may have updated by another route since this was staged.
        // Applying a stale or downgrading record would walk the running
        // binary backwards, or reinstall what is already running.
        Ok(false) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Version);
        }
        Err(_) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Version);
        }
    }

    if let Err(e) = verify(&staged_path) {
        let _ = stage::clear(target_exe);
        return Err(e);
    }

    // Only reachable after the digest comparison above succeeded: this is
    // the sole place outside of tests, besides `fetch::download_verified`
    // itself, that can construct a `VerifiedArtifact`.
    let verified = VerifiedArtifact::verified_from_stage(staged_path);
    swap::swap_in_place(&verified, target_exe).map_err(|_| UpdateError::Swap)?;
    stage::clear(target_exe).map_err(|_| UpdateError::Stage)?;
    Ok(Some(record.version))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::update::manifest::PlatformArtifact;
    use crate::update::stage;

    fn write_staged(dir: &std::path::Path, body: &[u8], version: &str) -> std::path::PathBuf {
        let target = dir.join("trace-commons-contributor");
        std::fs::write(&target, b"old").unwrap();
        stage::prepare(&target).unwrap();
        std::fs::write(stage::staged_binary_path(&target), body).unwrap();
        stage::write_record(
            &target,
            &stage::StagedUpdate {
                schema_version: stage::STAGED_UPDATE_SCHEMA.to_string(),
                version: version.to_string(),
                sha256: crate::update::fetch::sha256_hex(body),
                staged_at: "2026-08-17T00:00:00Z".to_string(),
            },
        )
        .unwrap();
        target
    }

    #[test]
    fn nothing_staged_applies_nothing() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        std::fs::write(&target, b"old").unwrap();
        assert!(apply_staged(&target).unwrap().is_none());
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
    }

    #[test]
    fn a_staged_update_is_applied_and_then_forgotten() {
        // The staged bytes here are not signed by anyone -- this test is
        // about the staging/swap/cleanup plumbing, not the signature check,
        // so it supplies a verifier that always accepts, the same way the
        // real one does on every platform except Windows.
        let d = tempfile::tempdir().unwrap();
        let target = write_staged(d.path(), b"new binary", "9.9.9");
        let applied = apply_staged_with_verifier(&target, |_| Ok(()))
            .unwrap()
            .expect("applied");
        assert_eq!(applied, "9.9.9");
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");
        assert!(stage::read_record(&target).unwrap().is_none());
    }

    #[test]
    fn an_unsigned_staged_binary_is_refused_and_cleared() {
        // This is the gate itself, not a bypass of it: a verifier that
        // refuses (standing in for a real Authenticode check on an unsigned
        // binary) must stop the swap, leave the running binary untouched,
        // and clear the poisoned staging area -- on every platform, not only
        // where the real Windows verifier can run.
        let d = tempfile::tempdir().unwrap();
        let target = write_staged(d.path(), b"new binary", "9.9.9");
        let result = apply_staged_with_verifier(&target, |_| Err(UpdateError::Authenticode));
        assert!(matches!(result, Err(UpdateError::Authenticode)));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(
            stage::read_record(&target).unwrap().is_none(),
            "a refused staged binary must not be retried forever"
        );
    }

    #[test]
    fn staged_bytes_that_changed_after_verification_are_refused_and_cleared() {
        // A staged update may sit on disk across a reboot. What is applied
        // must be what was verified, so the digest is re-checked here rather
        // than trusted from the earlier download.
        let d = tempfile::tempdir().unwrap();
        let target = write_staged(d.path(), b"new binary", "9.9.9");
        std::fs::write(stage::staged_binary_path(&target), b"swapped out!").unwrap();

        assert!(matches!(apply_staged(&target), Err(UpdateError::Fetch)));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(
            stage::read_record(&target).unwrap().is_none(),
            "a poisoned staging area must be cleared, not left to retry forever"
        );
    }

    #[test]
    fn a_staged_downgrade_is_refused_and_cleared() {
        let d = tempfile::tempdir().unwrap();
        let target = write_staged(d.path(), b"old binary", "0.0.1");
        assert!(matches!(apply_staged(&target), Err(UpdateError::Version)));
        assert_eq!(std::fs::read(&target).unwrap(), b"old");
        assert!(stage::read_record(&target).unwrap().is_none());
    }

    #[test]
    fn a_winget_owned_install_defers_without_touching_the_network() {
        // classify is the whole decision, and it is a pure path check -- so
        // this asserts the branch that must never reach a fetch.
        let p = std::path::Path::new(
            r"C:\Users\ada\AppData\Local\Microsoft\WinGet\Packages\x\trace-commons-contributor.exe",
        );
        assert_eq!(
            crate::update::source::classify(p),
            crate::update::source::InstallSource::WingetManaged
        );
    }

    #[test]
    fn an_unrecognized_install_defers_without_touching_the_network() {
        // We do not know we placed these bytes (Homebrew, a distro package,
        // a read-only Nix path, install.sh --dir), so nothing installs.
        // classify is a pure path check that decides this branch too, and it
        // must never fall through to the swap: doing so would clobber a
        // package manager's file.
        let p = std::path::Path::new("/opt/homebrew/bin/trace-commons-contributor");
        assert_eq!(
            crate::update::source::classify(p),
            crate::update::source::InstallSource::Unrecognized
        );
    }

    #[test]
    fn the_artifact_for_this_platform_is_looked_up_by_slug() {
        let mut platforms = std::collections::BTreeMap::new();
        platforms.insert(
            "linux-x86_64-cli".to_string(),
            PlatformArtifact {
                url: "https://example.invalid/tc".to_string(),
                sha256: "a".repeat(64),
                size: 7,
            },
        );
        assert!(select_artifact(&platforms, "linux-x86_64-cli").is_some());
        // A platform absent from the manifest is the normal state when that
        // build job failed. It is "nothing to do", not an error.
        assert!(select_artifact(&platforms, "windows-x86_64-cli").is_none());
    }
}
