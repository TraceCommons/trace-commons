//! The shared update-conformance fixtures, from the Rust side.
//!
//! The Swift implementation of the same verify-before-swap logic consumes
//! these exact bytes. That is the whole point: the logic exists twice, so a
//! check dropped in either implementation has to fail a test here or there
//! rather than ship.

use std::path::PathBuf;

use trace_commons_contributor::update::manifest::{ManifestError, UpdateManifest, verify_manifest};
use trace_commons_contributor::update::version::is_newer;

pub fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/update-conformance")
        .canonicalize()
        .expect("update-conformance fixtures are missing; run tests/fixtures/update-conformance/regenerate.sh")
}

/// The raw 32-byte Ed25519 public key the fixtures are signed under.
pub fn public_key() -> Vec<u8> {
    let hex_text = std::fs::read_to_string(fixture_dir().join("manifest-public-key.hex"))
        .expect("fixture public key");
    hex::decode(hex_text.trim()).expect("fixture public key is hex")
}

/// The manifest bytes and detached signature for one fixture case.
pub fn manifest_case(name: &str) -> (Vec<u8>, String) {
    let dir = fixture_dir().join(name);
    let bytes = std::fs::read(dir.join("latest.json")).expect("fixture manifest");
    let sig = std::fs::read_to_string(dir.join("latest.json.sig")).expect("fixture signature");
    (bytes, sig)
}

fn good_manifest() -> UpdateManifest {
    let (bytes, sig) = manifest_case("good");
    verify_manifest(&bytes, &sig, &public_key()).expect("the good fixture must verify")
}

#[test]
fn the_good_manifest_verifies_and_names_every_cli_platform() {
    let manifest = good_manifest();
    assert_eq!(manifest.version, "9.9.9");
    for slug in [
        "windows-x86_64-cli",
        "linux-x86_64-cli",
        "macos-aarch64-cli",
        "macos-x86_64-cli",
    ] {
        assert!(
            manifest.platforms.contains_key(slug),
            "good fixture is missing {slug}"
        );
    }
}

#[test]
fn a_manifest_signed_by_an_unpinned_key_is_refused() {
    let (bytes, sig) = manifest_case("bad-signature");
    let err = verify_manifest(&bytes, &sig, &public_key()).unwrap_err();
    assert!(matches!(err, ManifestError::BadSignature));
}

#[test]
fn a_correctly_signed_older_manifest_is_refused_by_the_version_gate() {
    // It verifies -- that is the point. Only the version comparison stops a
    // replayed manifest from walking a client backwards.
    let (bytes, sig) = manifest_case("downgrade");
    let manifest =
        verify_manifest(&bytes, &sig, &public_key()).expect("downgrade fixture verifies");
    assert_eq!(manifest.version, "0.0.1");
    assert!(!is_newer("0.1.0", &manifest.version).unwrap());
    assert!(!is_newer("0.0.1", &manifest.version).unwrap());
}

#[test]
fn the_good_manifest_is_newer_than_this_build() {
    let manifest = good_manifest();
    assert!(is_newer(env!("CARGO_PKG_VERSION"), &manifest.version).unwrap());
}

#[test]
fn the_fixture_artifact_matches_the_digest_the_manifest_publishes() {
    use sha2::{Digest, Sha256};
    let manifest = good_manifest();
    let bytes = std::fs::read(fixture_dir().join("good/artifact.bin")).expect("good artifact");
    let actual = format!("{:x}", Sha256::digest(&bytes));
    let published = &manifest.platforms["linux-x86_64-cli"];
    assert_eq!(actual, published.sha256);
    assert_eq!(bytes.len() as u64, published.size);
}

#[test]
fn an_artifact_tampered_with_after_signing_is_refused() {
    use trace_commons_contributor::update::fetch::{FetchError, verify_bytes};
    let manifest = good_manifest();
    let bytes = std::fs::read(fixture_dir().join("tampered/artifact.bin")).expect("tampered");
    // Same length as the good artifact, so this fails on the digest and not
    // incidentally on the size -- otherwise the test would pass even if the
    // digest check were removed.
    let published = &manifest.platforms["linux-x86_64-cli"];
    assert_eq!(bytes.len() as u64, published.size);
    assert!(matches!(
        verify_bytes(published, &bytes).unwrap_err(),
        FetchError::DigestMismatch
    ));
}

#[test]
fn the_unsigned_fixture_is_refused_by_the_signature_decision() {
    use trace_commons_contributor::update::authenticode::{AuthenticodeError, interpret};
    // The fixture exists on every platform; what the platform verifier would
    // report for it is `NotSigned`, and that is what must be refused.
    let bytes = std::fs::read(fixture_dir().join("unsigned/artifact.exe")).expect("unsigned");
    assert!(!bytes.is_empty());
    // A PE with an Authenticode signature always starts with the `MZ`
    // magic. This fixture does not, so it cannot carry a certificate table
    // regardless of what any verifier reports -- the fixture is genuinely
    // unsigned, not merely a PE this test forgot to sign.
    assert_ne!(&bytes[..2.min(bytes.len())], b"MZ");
    assert!(matches!(
        interpret("NotSigned", "").unwrap_err(),
        AuthenticodeError::NotValid
    ));
}

#[cfg(windows)]
#[test]
fn the_platform_verifier_refuses_the_unsigned_fixture() {
    use trace_commons_contributor::update::authenticode::{AuthenticodeError, verify};
    let path = fixture_dir().join("unsigned/artifact.exe");
    assert!(matches!(
        verify(&path).unwrap_err(),
        AuthenticodeError::NotValid | AuthenticodeError::UnexpectedSigner
    ));
}
