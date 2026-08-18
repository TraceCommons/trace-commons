//! Where the manifest lives and whose signature over it we accept.
//!
//! Both are compile-time constants. The URL is fixed because a configurable
//! update source is an update source an attacker can configure; the key is
//! pinned because the bucket is public and a public bucket carries bytes, not
//! authority.

/// The signed manifest, in the same public bucket the flatpak repo uses.
pub const MANIFEST_URL: &str =
    "https://storage.googleapis.com/tracecommons-flatpak/updates/latest.json";

/// The detached signature over `MANIFEST_URL`'s exact bytes.
pub const MANIFEST_SIG_URL: &str =
    "https://storage.googleapis.com/tracecommons-flatpak/updates/latest.json.sig";

/// The raw 32-byte Ed25519 public key, hex, pinned at build time.
///
/// Supplied by the release build as `TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX`.
/// An unset pin leaves this empty, and an empty pin refuses every update --
/// see `decode_pinned_key`. A developer build therefore never self-updates,
/// which is the correct default for a binary built from a working tree.
pub const MANIFEST_PUBLIC_KEY_HEX: &str = match option_env!("TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX") {
    Some(v) => v,
    None => "",
};

#[derive(Debug, thiserror::Error)]
pub enum EndpointError {
    /// This build has no pinned update key, so there is nothing to verify a
    /// manifest against and no update can be trusted.
    #[error("update_endpoint_no_pinned_key")]
    NoPinnedKey,
    /// The pinned key is present but is not 32 bytes of hex.
    #[error("update_endpoint_malformed_pinned_key")]
    MalformedPinnedKey,
    /// No CLI artifact is published for this target.
    #[error("update_endpoint_unsupported_platform")]
    UnsupportedPlatform,
}

/// Decode a hex-encoded raw Ed25519 public key, refusing anything that is not
/// exactly 32 bytes.
pub(crate) fn decode_pinned_key(hex_text: &str) -> Result<Vec<u8>, EndpointError> {
    let trimmed = hex_text.trim();
    if trimmed.is_empty() {
        return Err(EndpointError::NoPinnedKey);
    }
    let bytes = hex::decode(trimmed).map_err(|_| EndpointError::MalformedPinnedKey)?;
    if bytes.len() != 32 {
        return Err(EndpointError::MalformedPinnedKey);
    }
    Ok(bytes)
}

/// The public key this build accepts manifest signatures under.
pub fn manifest_public_key() -> Result<Vec<u8>, EndpointError> {
    decode_pinned_key(MANIFEST_PUBLIC_KEY_HEX)
}

/// The manifest platform slug for the target this binary was built for.
///
/// Derived from `cfg`, not from a runtime probe: the artifact that can
/// replace this binary is decided by how this binary was compiled.
pub fn platform_slug() -> Result<&'static str, EndpointError> {
    #[cfg(all(target_os = "windows", target_arch = "x86_64"))]
    {
        return Ok("windows-x86_64-cli");
    }
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        return Ok("linux-x86_64-cli");
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        return Ok("macos-aarch64-cli");
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        return Ok("macos-x86_64-cli");
    }
    #[allow(unreachable_code)]
    Err(EndpointError::UnsupportedPlatform)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_manifest_urls_are_https_and_agree_on_their_object() {
        assert!(MANIFEST_URL.starts_with("https://"));
        assert_eq!(MANIFEST_SIG_URL, format!("{MANIFEST_URL}.sig"));
    }

    #[test]
    fn a_32_byte_hex_pin_decodes() {
        let hex_text = "2a".repeat(32);
        let key = decode_pinned_key(&hex_text).expect("32 bytes of hex is a key");
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn an_absent_pin_is_a_refusal_not_an_empty_key() {
        // A build with no pinned key must refuse to update at all. Treating
        // "" as "trust whatever the bucket serves" is the one failure mode
        // this whole subsystem exists to prevent.
        assert!(matches!(
            decode_pinned_key(""),
            Err(EndpointError::NoPinnedKey)
        ));
        assert!(matches!(
            decode_pinned_key("   "),
            Err(EndpointError::NoPinnedKey)
        ));
    }

    #[test]
    fn a_pin_that_is_not_32_bytes_of_hex_is_refused() {
        assert!(matches!(
            decode_pinned_key("2a2a2a"),
            Err(EndpointError::MalformedPinnedKey)
        ));
        assert!(matches!(
            decode_pinned_key(&"zz".repeat(32)),
            Err(EndpointError::MalformedPinnedKey)
        ));
    }

    #[test]
    fn this_build_maps_to_exactly_one_slug_or_refuses() {
        match platform_slug() {
            Ok(slug) => assert!(
                [
                    "windows-x86_64-cli",
                    "linux-x86_64-cli",
                    "macos-aarch64-cli",
                    "macos-x86_64-cli",
                ]
                .contains(&slug),
                "unexpected slug {slug}"
            ),
            Err(e) => assert!(matches!(e, EndpointError::UnsupportedPlatform)),
        }
    }
}
