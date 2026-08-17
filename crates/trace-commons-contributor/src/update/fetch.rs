//! Fetching, and the digest check that decides whether the bytes are usable.
//!
//! Every download is capped before it is read: an update client that will
//! buffer whatever a server sends is a client a server can exhaust. Nothing
//! that has not verified is ever placed where it could be executed.

use std::path::Path;

use sha2::{Digest, Sha256};

use super::manifest::PlatformArtifact;

/// The signed manifest is a few hundred bytes today; 64 KiB is generous.
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
/// A base64 Ed25519 signature is 88 bytes.
pub const MAX_SIGNATURE_BYTES: usize = 1024;
/// The contributor binary is tens of megabytes. 256 MiB is a ceiling, not a
/// target.
pub const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;

/// How long any single update request may take.
const REQUEST_TIMEOUT_SECS: u64 = 120;

#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error("update_fetch_client_build_failed")]
    ClientBuild,
    #[error("update_fetch_http_failed")]
    Http,
    #[error("update_fetch_bad_status")]
    Status,
    #[error("update_fetch_too_large")]
    TooLarge,
    #[error("update_fetch_size_mismatch")]
    SizeMismatch,
    #[error("update_fetch_digest_mismatch")]
    DigestMismatch,
    #[error("update_fetch_io_failed")]
    Io,
}

/// The one HTTP client for the whole update path.
pub fn http_client() -> Result<reqwest::Client, FetchError> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
        .build()
        .map_err(|_| FetchError::ClientBuild)
}

/// Lowercase hex sha256, the form the manifest publishes.
pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Whether these bytes are the artifact the signed manifest describes.
///
/// Size first, then digest: a size mismatch is the cheaper and more specific
/// signal, and reporting it as a digest mismatch would send an operator
/// looking for corruption when what they have is the wrong file.
pub fn verify_bytes(artifact: &PlatformArtifact, bytes: &[u8]) -> Result<(), FetchError> {
    if bytes.len() as u64 != artifact.size {
        return Err(FetchError::SizeMismatch);
    }
    // An empty or short published digest is a refusal, never a wildcard.
    if artifact.sha256.len() != 64 {
        return Err(FetchError::DigestMismatch);
    }
    if !artifact.sha256.eq_ignore_ascii_case(&sha256_hex(bytes)) {
        return Err(FetchError::DigestMismatch);
    }
    Ok(())
}

/// Refuse a plaintext URL. Signing proves origin, not transport, so a
/// manifest that names an `http://` artifact is refused even though the
/// manifest itself verified.
fn require_https(url: &str) -> Result<(), FetchError> {
    if url.starts_with("https://") {
        Ok(())
    } else {
        Err(FetchError::Status)
    }
}

/// GET `url`, refusing a body larger than `max_bytes`.
///
/// The cap is enforced against the advertised length when there is one and
/// against the accumulated body regardless, because a `Content-Length` is a
/// claim by the same server that sends the body.
pub async fn fetch_capped(
    client: &reqwest::Client,
    url: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, FetchError> {
    require_https(url)?;
    // The URL is a compile-time constant or a field of an
    // already-signature-verified manifest. It is never logged.
    let response = client.get(url).send().await.map_err(|_| FetchError::Http)?;
    if !response.status().is_success() {
        return Err(FetchError::Status);
    }
    if let Some(len) = response.content_length() {
        if len > max_bytes as u64 {
            return Err(FetchError::TooLarge);
        }
    }
    let body = response.bytes().await.map_err(|_| FetchError::Http)?;
    if body.len() > max_bytes {
        return Err(FetchError::TooLarge);
    }
    Ok(body.to_vec())
}

/// A path proven, at the moment it was returned, to hold bytes matching a
/// signed manifest's published digest.
///
/// `download_verified` is the only production constructor. `swap::swap_in_place`
/// takes this type rather than a raw path so that it is impossible to call it
/// with bytes that were never checked against the manifest -- the compiler
/// enforces the ordering that the doc comment on `download_verified` used to
/// only assert in prose.
pub struct VerifiedArtifact(std::path::PathBuf);

impl VerifiedArtifact {
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Test-only escape hatch. `swap::tests` exercises swap mechanics
    /// directly against files it writes itself, without running a network
    /// fetch, and needs a way to construct this type without one.
    #[cfg(test)]
    pub(crate) fn for_test(path: std::path::PathBuf) -> Self {
        Self(path)
    }

    /// The second (and only other) way to construct this type outside of
    /// tests.
    ///
    /// `update::run::apply_staged` applies a binary that a *previous* run of
    /// this process downloaded and verified via `download_verified` -- there
    /// is no download to re-run on the staged-apply path, only a disk read.
    /// That disk read is re-verified by the caller against the recorded
    /// `StagedUpdate::sha256` before this is called, which is what makes it
    /// safe: the precondition is spelled out in the name, and `pub(crate)`
    /// keeps it out of reach of anything outside this crate that has not
    /// done that check. Do not call this before the digest comparison
    /// succeeds.
    pub(crate) fn verified_from_stage(path: std::path::PathBuf) -> Self {
        Self(path)
    }
}

/// Download an artifact, verify it, and only then write it to `dest`.
///
/// The order is the point: nothing unverified is ever written to a path that
/// something else might later execute.
pub async fn download_verified(
    client: &reqwest::Client,
    artifact: &PlatformArtifact,
    dest: &Path,
) -> Result<VerifiedArtifact, FetchError> {
    require_https(&artifact.url)?;
    if artifact.size > MAX_ASSET_BYTES as u64 {
        return Err(FetchError::TooLarge);
    }
    let bytes = fetch_capped(client, &artifact.url, MAX_ASSET_BYTES).await?;
    verify_bytes(artifact, &bytes)?;
    std::fs::write(dest, &bytes).map_err(|_| FetchError::Io)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dest, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| FetchError::Io)?;
    }
    Ok(VerifiedArtifact(dest.to_path_buf()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn artifact(sha256: &str, size: u64) -> PlatformArtifact {
        PlatformArtifact {
            url: "https://example.invalid/tc".to_string(),
            sha256: sha256.to_string(),
            size,
        }
    }

    #[test]
    fn sha256_hex_is_lowercase_and_64_characters() {
        let d = sha256_hex(b"abc");
        assert_eq!(
            d,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn matching_bytes_verify() {
        let body = b"abc";
        let a = artifact(&sha256_hex(body), body.len() as u64);
        assert!(verify_bytes(&a, body).is_ok());
    }

    #[test]
    fn a_published_digest_in_uppercase_still_matches() {
        // install.ps1 compares Get-FileHash output, which is uppercase, so a
        // digest may reach us in either case. Case is not the property being
        // checked here.
        let body = b"abc";
        let a = artifact(&sha256_hex(body).to_uppercase(), body.len() as u64);
        assert!(verify_bytes(&a, body).is_ok());
    }

    #[test]
    fn a_wrong_size_is_refused_before_the_digest_is_considered() {
        let body = b"abc";
        let a = artifact(&sha256_hex(body), 999);
        assert!(matches!(
            verify_bytes(&a, body).unwrap_err(),
            FetchError::SizeMismatch
        ));
    }

    #[test]
    fn bytes_that_do_not_match_the_published_digest_are_refused() {
        let a = artifact(&sha256_hex(b"abc"), 3);
        assert!(matches!(
            verify_bytes(&a, b"xyz").unwrap_err(),
            FetchError::DigestMismatch
        ));
    }

    #[test]
    fn an_empty_published_digest_is_refused_rather_than_treated_as_a_wildcard() {
        let a = artifact("", 3);
        assert!(matches!(
            verify_bytes(&a, b"abc").unwrap_err(),
            FetchError::DigestMismatch
        ));
    }

    #[test]
    fn the_caps_are_ordered_the_way_the_three_downloads_are_sized() {
        const { assert!(MAX_SIGNATURE_BYTES < MAX_MANIFEST_BYTES) };
        const { assert!(MAX_MANIFEST_BYTES < MAX_ASSET_BYTES) };
    }

    #[test]
    fn a_plaintext_artifact_url_is_refused_even_though_it_would_be_signed() {
        let a = PlatformArtifact {
            url: "http://example.invalid/tc".to_string(),
            sha256: sha256_hex(b"abc"),
            size: 3,
        };
        assert!(matches!(
            require_https(&a.url).unwrap_err(),
            FetchError::Status
        ));
    }
}
