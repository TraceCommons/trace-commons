//! The generator and the verifier must agree on exact bytes. A test that
//! only exercised the Rust side would keep passing while a shell change
//! broke every installed client, which is the failure this file exists to
//! prevent.

use std::process::Command;

use trace_commons_contributor::update::manifest::{ManifestError, verify_manifest};

fn repo_root() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// Generates an Ed25519 key, runs the real script, and verifies the real
/// output with the real verifier.
fn generate(tmp: &std::path::Path, version: &str) -> (Vec<u8>, String, Vec<u8>) {
    let key = tmp.join("test-signing.pem");
    let status = Command::new("openssl")
        .args(["genpkey", "-algorithm", "ed25519", "-out"])
        .arg(&key)
        .status()
        .expect("openssl genpkey");
    assert!(status.success(), "key generation failed");

    let pub_pem = tmp.join("test-signing.pub.pem");
    let status = Command::new("openssl")
        .arg("pkey")
        .arg("-in")
        .arg(&key)
        .args(["-pubout", "-out"])
        .arg(&pub_pem)
        .status()
        .expect("openssl pkey");
    assert!(status.success(), "public key export failed");

    // Strip the SubjectPublicKeyInfo wrapper down to the raw 32-byte key
    // that ring's UnparsedPublicKey expects. For Ed25519 the raw key is
    // always the final 32 bytes of the DER encoding.
    let der = Command::new("openssl")
        .arg("pkey")
        .arg("-in")
        .arg(&key)
        .args(["-pubout", "-outform", "DER"])
        .output()
        .expect("openssl DER export");
    let raw_pub = der.stdout[der.stdout.len() - 32..].to_vec();

    let out_dir = tmp.join("out");
    let status = Command::new(repo_root().join("scripts/updates/generate-manifest.sh"))
        .args(["--version", version])
        .arg("--key")
        .arg(&key)
        .arg("--out")
        .arg(&out_dir)
        .args([
            "--platform",
            "windows-x86_64=https://example.invalid/tc.zip=\
             a3f1b2c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f80=4096",
        ])
        .status()
        .expect("generate-manifest.sh");
    assert!(status.success(), "generator failed");

    let bytes = std::fs::read(out_dir.join("latest.json")).expect("manifest");
    let sig = std::fs::read_to_string(out_dir.join("latest.json.sig")).expect("signature");
    (bytes, sig, raw_pub)
}

#[test]
fn the_generator_produces_a_manifest_the_verifier_accepts() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (bytes, sig, pubkey) = generate(tmp.path(), "1.4.2");
    let manifest = verify_manifest(&bytes, &sig, &pubkey).expect("should verify");
    assert_eq!(manifest.version, "1.4.2");
    assert_eq!(manifest.platforms.len(), 1);
    assert!(manifest.platforms.contains_key("windows-x86_64"));
}

#[test]
fn a_byte_changed_after_generation_fails_verification() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let (bytes, sig, pubkey) = generate(tmp.path(), "1.4.2");
    let tampered = String::from_utf8(bytes)
        .unwrap()
        .replace("1.4.2", "1.4.3")
        .into_bytes();
    assert!(matches!(
        verify_manifest(&tampered, &sig, &pubkey).unwrap_err(),
        ManifestError::BadSignature
    ));
}

#[test]
fn the_generator_refuses_a_manifest_with_no_platforms() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let key = tmp.path().join("k.pem");
    Command::new("openssl")
        .args(["genpkey", "-algorithm", "ed25519", "-out"])
        .arg(&key)
        .status()
        .expect("openssl");
    let status = Command::new(repo_root().join("scripts/updates/generate-manifest.sh"))
        .args(["--version", "1.0.0"])
        .arg("--key")
        .arg(&key)
        .status()
        .expect("script");
    assert!(!status.success(), "empty manifest must be refused");
}
