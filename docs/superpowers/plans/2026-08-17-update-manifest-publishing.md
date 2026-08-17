# Update Manifest Publishing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish a signed, rate-limit-free update manifest so installed contributor apps can discover new releases without calling the GitHub API.

**Architecture:** The release pipeline generates two files from one set of release metadata — `latest.json` (Ed25519-signed, for Linux and Windows) and `appcast.xml` (EdDSA-signed, for Sparkle on macOS) — and uploads both to the existing public GCS bucket under an `updates/` prefix. The Rust side of this plan is the manifest *format and verifier*, which every client depends on; the shell and CI side is generation and publication.

**Tech Stack:** Rust (`serde`, `serde_json`, `ring` — all existing direct dependencies of `trace-commons-contributor`), bash, GitHub Actions, `gcloud storage`, OpenSSL for Ed25519 signing.

## Global Constraints

- Verify the signature over the raw manifest bytes **before** parsing them. Never act on unparsed-but-unverified content.
- Fail closed: any verification failure aborts and keeps the current state. There is no unverified fallback path.
- Versions are three-part numeric (`X.Y.Z`), matching the tag validation already enforced in `release-apps.yml`. Do not add a semver dependency.
- Hash-only logging. Never log URLs, signatures, key material, or file bodies.
- No new Cargo dependencies. `serde`, `serde_json`, `ring`, `thiserror` are already direct dependencies of `trace-commons-contributor`.
- Rust edition 2024, `rust-version = 1.92`.
- Verify locally with `RUSTFLAGS="-D warnings" cargo check` and `cargo clippy` before claiming green; plain `cargo check` does not apply `-D warnings` but CI does.
- No emojis in code, commits, or PR bodies. Short imperative commit subjects, no `feat:`/`fix:` prefixes.

---

### Task 1: Manifest format and signature verification

**Files:**
- Create: `crates/trace-commons-contributor/src/update/mod.rs`
- Create: `crates/trace-commons-contributor/src/update/manifest.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod update;`)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub const UPDATE_MANIFEST_SCHEMA: &str`
  - `pub struct UpdateManifest { pub schema_version: String, pub version: String, pub published_at: String, pub platforms: BTreeMap<String, PlatformArtifact> }`
  - `pub struct PlatformArtifact { pub url: String, pub sha256: String, pub size: u64 }`
  - `pub enum ManifestError` (variants below)
  - `pub fn verify_manifest(bytes: &[u8], signature_b64: &str, public_key: &[u8]) -> Result<UpdateManifest, ManifestError>`

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/manifest.rs` with a test module at the bottom:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    fn test_keypair() -> Ed25519KeyPair {
        // A fixed seed keeps the test deterministic. This key is a test
        // fixture and signs nothing that is ever published.
        let seed = [7u8; 32];
        Ed25519KeyPair::from_seed_unchecked(&seed).expect("valid seed")
    }

    fn sign(kp: &Ed25519KeyPair, bytes: &[u8]) -> String {
        use base64::Engine as _;
        base64::engine::general_purpose::STANDARD.encode(kp.sign(bytes).as_ref())
    }

    fn good_manifest_bytes() -> Vec<u8> {
        br#"{
  "schema_version": "trace_commons.update_manifest.v1",
  "version": "0.2.0",
  "published_at": "2026-08-17T00:00:00Z",
  "platforms": {
    "windows-x86_64": {
      "url": "https://example.invalid/tc-0.2.0.zip",
      "sha256": "a3f1b2c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f80",
      "size": 4096
    }
  }
}"#
        .to_vec()
    }

    #[test]
    fn accepts_a_correctly_signed_manifest() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let sig = sign(&kp, &bytes);
        let manifest =
            verify_manifest(&bytes, &sig, kp.public_key().as_ref()).expect("should verify");
        assert_eq!(manifest.version, "0.2.0");
        assert_eq!(manifest.platforms["windows-x86_64"].size, 4096);
    }

    #[test]
    fn rejects_a_manifest_whose_bytes_changed_after_signing() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let sig = sign(&kp, &bytes);
        let tampered = String::from_utf8(bytes)
            .unwrap()
            .replace("0.2.0", "9.9.9")
            .into_bytes();
        let err = verify_manifest(&tampered, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::BadSignature));
    }

    #[test]
    fn rejects_a_signature_from_the_wrong_key() {
        let kp = test_keypair();
        let other = Ed25519KeyPair::from_seed_unchecked(&[9u8; 32]).unwrap();
        let bytes = good_manifest_bytes();
        let sig = sign(&other, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::BadSignature));
    }

    #[test]
    fn rejects_an_unknown_schema_version() {
        let kp = test_keypair();
        let bytes = String::from_utf8(good_manifest_bytes())
            .unwrap()
            .replace(
                "trace_commons.update_manifest.v1",
                "trace_commons.update_manifest.v2",
            )
            .into_bytes();
        let sig = sign(&kp, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::UnknownSchema));
    }

    #[test]
    fn rejects_malformed_base64_without_panicking() {
        let kp = test_keypair();
        let bytes = good_manifest_bytes();
        let err = verify_manifest(&bytes, "not base64!!", kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedSignature));
    }

    #[test]
    fn rejects_valid_signature_over_invalid_json() {
        let kp = test_keypair();
        let bytes = b"{ this is not json".to_vec();
        let sig = sign(&kp, &bytes);
        let err = verify_manifest(&bytes, &sig, kp.public_key().as_ref()).unwrap_err();
        assert!(matches!(err, ManifestError::MalformedJson));
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::manifest`
Expected: FAIL — the module does not compile because `verify_manifest`, `UpdateManifest`, and `ManifestError` do not exist.

- [ ] **Step 3: Write the minimal implementation**

Create `crates/trace-commons-contributor/src/update/mod.rs`:

```rust
//! Update discovery and installation.
//!
//! The manifest is the only thing a client trusts to learn that a new
//! version exists. It is signed because the transport is not: a public
//! bucket is a fine place to put bytes and a poor place to put authority.
pub mod manifest;
```

Add to `crates/trace-commons-contributor/src/lib.rs`, in the existing alphabetical run of `pub mod` declarations:

```rust
pub mod update;
```

Then the body of `crates/trace-commons-contributor/src/update/manifest.rs`, above the test module:

```rust
use std::collections::BTreeMap;

use base64::Engine as _;
use serde::{Deserialize, Serialize};

/// The only manifest schema this build understands. An unknown value is a
/// refusal, not a warning: a client that guesses at a newer schema's meaning
/// is a client that can be talked into installing the wrong thing.
pub const UPDATE_MANIFEST_SCHEMA: &str = "trace_commons.update_manifest.v1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformArtifact {
    pub url: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateManifest {
    pub schema_version: String,
    pub version: String,
    pub published_at: String,
    /// Keyed by platform slug: `windows-x86_64`, `macos-universal`,
    /// `linux-x86_64`. A platform absent from this map has no release --
    /// which is the normal state when one build job failed and the others
    /// did not.
    pub platforms: BTreeMap<String, PlatformArtifact>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The detached signature was not valid base64.
    #[error("update_manifest_malformed_signature")]
    MalformedSignature,
    /// The signature did not verify against the pinned public key.
    #[error("update_manifest_bad_signature")]
    BadSignature,
    /// Signature verified, but the payload was not valid JSON.
    #[error("update_manifest_malformed_json")]
    MalformedJson,
    /// Signature verified and parsed, but the schema is not one we know.
    #[error("update_manifest_unknown_schema")]
    UnknownSchema,
}

/// Verify a detached Ed25519 signature over `bytes`, then parse.
///
/// The order matters and is the whole point of this function: nothing is
/// parsed until the signature over the exact bytes has verified. Callers
/// must not deserialize the manifest themselves.
pub fn verify_manifest(
    bytes: &[u8],
    signature_b64: &str,
    public_key: &[u8],
) -> Result<UpdateManifest, ManifestError> {
    let signature = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| ManifestError::MalformedSignature)?;

    let key = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, public_key);
    key.verify(bytes, &signature)
        .map_err(|_| ManifestError::BadSignature)?;

    let manifest: UpdateManifest =
        serde_json::from_slice(bytes).map_err(|_| ManifestError::MalformedJson)?;

    if manifest.schema_version != UPDATE_MANIFEST_SCHEMA {
        return Err(ManifestError::UnknownSchema);
    }

    Ok(manifest)
}
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::manifest`
Expected: PASS, 6 tests.

Then run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor` and `cargo clippy -p trace-commons-contributor --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
Expected: both clean.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/ crates/trace-commons-contributor/src/lib.rs
git commit -m "Add the signed update manifest format and its verifier"
```

---

### Task 2: Version comparison with downgrade protection

**Files:**
- Create: `crates/trace-commons-contributor/src/update/version.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod version;`)

**Interfaces:**
- Consumes: nothing.
- Produces: `pub fn is_newer(current: &str, offered: &str) -> Result<bool, VersionError>` and `pub enum VersionError { Malformed }`.

Downgrade protection is a separate task from the manifest because it is a separate attack. A signed-but-old manifest replayed at a client verifies perfectly; only the version comparison stops it.

- [ ] **Step 1: Write the failing tests**

Append to `crates/trace-commons-contributor/src/update/version.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_higher_patch_is_newer() {
        assert!(is_newer("0.1.0", "0.1.1").unwrap());
    }

    #[test]
    fn a_higher_minor_is_newer() {
        assert!(is_newer("0.1.9", "0.2.0").unwrap());
    }

    #[test]
    fn a_higher_major_is_newer() {
        assert!(is_newer("0.9.9", "1.0.0").unwrap());
    }

    #[test]
    fn components_compare_numerically_not_lexically() {
        // The bug this guards: "10" < "9" as strings.
        assert!(is_newer("0.9.0", "0.10.0").unwrap());
        assert!(!is_newer("0.10.0", "0.9.0").unwrap());
    }

    #[test]
    fn the_same_version_is_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.3").unwrap());
    }

    #[test]
    fn an_older_version_is_not_newer() {
        assert!(!is_newer("1.2.3", "1.2.2").unwrap());
        assert!(!is_newer("2.0.0", "1.9.9").unwrap());
    }

    #[test]
    fn malformed_versions_are_refused_not_guessed() {
        assert!(is_newer("1.2", "1.2.3").is_err());
        assert!(is_newer("1.2.3", "1.2.3.4").is_err());
        assert!(is_newer("1.2.3", "v1.2.4").is_err());
        assert!(is_newer("1.2.3", "1.2.x").is_err());
        assert!(is_newer("", "1.2.3").is_err());
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::version`
Expected: FAIL — `is_newer` is not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/version.rs`:

```rust
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum VersionError {
    /// Not a three-part numeric version. `release-apps.yml` refuses to cut a
    /// tag that is not `X.Y.Z`, so anything else here is a malformed or
    /// hostile manifest rather than a version this build should reason about.
    #[error("update_version_malformed")]
    Malformed,
}

fn parse(v: &str) -> Result<[u64; 3], VersionError> {
    let mut parts = v.split('.');
    let mut out = [0u64; 3];
    for slot in out.iter_mut() {
        let raw = parts.next().ok_or(VersionError::Malformed)?;
        if raw.is_empty() || !raw.bytes().all(|b| b.is_ascii_digit()) {
            return Err(VersionError::Malformed);
        }
        *slot = raw.parse().map_err(|_| VersionError::Malformed)?;
    }
    if parts.next().is_some() {
        return Err(VersionError::Malformed);
    }
    Ok(out)
}

/// True when `offered` is strictly greater than `current`.
///
/// Strictly: equal is false, so a replayed manifest for the running version
/// installs nothing, and older is false, so a replayed manifest for an older
/// version cannot walk a client backwards onto a build with known problems.
pub fn is_newer(current: &str, offered: &str) -> Result<bool, VersionError> {
    Ok(parse(offered)? > parse(current)?)
}
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`:

```rust
pub mod version;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::version`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/
git commit -m "Compare update versions numerically and refuse downgrades"
```

---

### Task 3: Manifest generation script

**Files:**
- Create: `scripts/updates/generate-manifest.sh`
- Create: `scripts/updates/README.md`

**Interfaces:**
- Consumes: `UpdateManifest` JSON shape from Task 1.
- Produces: `latest.json` and `latest.json.sig` on disk; consumed by Task 5's workflow job.

The script writes only the platforms it was given, so a failed build job means an absent key rather than a URL that 404s. Follow the style of the existing `scripts/winget/generate-manifests.sh`.

- [ ] **Step 1: Write the script**

Create `scripts/updates/generate-manifest.sh`:

```bash
#!/usr/bin/env bash
# Generate and sign the update manifest that installed clients poll.
#
# Only platforms passed on the command line are written. That is the whole
# safety property: the three release build jobs are independent, so this runs
# routinely with a subset green, and a manifest that named a platform whose
# artifact does not exist would point every client of that platform at a 404.
set -euo pipefail

die() { echo "generate-manifest: $*" >&2; exit 1; }

VERSION=""
OUT_DIR="dist/updates"
KEY_FILE=""
declare -a PLATFORM_ARGS=()

usage() {
  cat >&2 <<'EOF'
usage: generate-manifest.sh --version X.Y.Z --key <ed25519.pem> \
         [--out <dir>] \
         --platform <slug>=<url>=<sha256>=<size> [--platform ...]

slugs: windows-x86_64 | macos-universal | linux-x86_64
EOF
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)  VERSION="${2:?--version needs a value}"; shift 2 ;;
    --key)      KEY_FILE="${2:?--key needs a path}"; shift 2 ;;
    --out)      OUT_DIR="${2:?--out needs a path}"; shift 2 ;;
    --platform) PLATFORM_ARGS+=("${2:?--platform needs a value}"); shift 2 ;;
    -h|--help)  usage ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || usage
[ -n "$KEY_FILE" ] || usage
[ -f "$KEY_FILE" ] || die "signing key not found: $KEY_FILE"
[ ${#PLATFORM_ARGS[@]} -gt 0 ] || die "refusing to publish a manifest with no platforms"

printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "version must be three-part numeric, got '$VERSION'"

PUBLISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

platforms_json=""
for spec in "${PLATFORM_ARGS[@]}"; do
  IFS='=' read -r slug url sha size <<<"$spec"
  case "$slug" in
    windows-x86_64|macos-universal|linux-x86_64) ;;
    *) die "unknown platform slug: $slug" ;;
  esac
  [ -n "$url" ]  || die "$slug: empty url"
  [ -n "$size" ] || die "$slug: empty size"
  printf '%s' "$sha" | grep -Eq '^[0-9a-f]{64}$' \
    || die "$slug: sha256 must be 64 lowercase hex characters, got '$sha'"
  printf '%s' "$size" | grep -Eq '^[0-9]+$' \
    || die "$slug: size must be numeric, got '$size'"

  entry="$(printf '"%s":{"url":"%s","sha256":"%s","size":%s}' \
             "$slug" "$url" "$sha" "$size")"
  if [ -n "$platforms_json" ]; then
    platforms_json="$platforms_json,$entry"
  else
    platforms_json="$entry"
  fi
done

mkdir -p "$OUT_DIR"
MANIFEST="$OUT_DIR/latest.json"

# Pretty-printed through jq so the published file is readable, and -S so key
# order is stable across runs. The signature covers these exact bytes, so
# nothing may touch the file after this point.
printf '{"schema_version":"trace_commons.update_manifest.v1","version":"%s","published_at":"%s","platforms":{%s}}' \
  "$VERSION" "$PUBLISHED_AT" "$platforms_json" \
  | jq -S . > "$MANIFEST"

openssl pkeyutl -sign -rawin -inkey "$KEY_FILE" -in "$MANIFEST" \
  | openssl base64 -A > "$MANIFEST.sig"

echo "wrote $MANIFEST"
echo "wrote $MANIFEST.sig"
```

Make it executable:

```bash
chmod +x scripts/updates/generate-manifest.sh
```

- [ ] **Step 2: Write a round-trip test that fails**

Create `crates/trace-commons-contributor/tests/update_manifest_roundtrip.rs`:

```rust
//! The generator and the verifier must agree on exact bytes. A test that
//! only exercised the Rust side would keep passing while a shell change
//! broke every installed client, which is the failure this file exists to
//! prevent.

use std::process::Command;

use trace_commons_contributor::update::manifest::{verify_manifest, ManifestError};

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
```

- [ ] **Step 3: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor --test update_manifest_roundtrip`
Expected: FAIL — the script does not exist yet if you have not created it, or the test binary fails to build because `tempfile` is not a dev-dependency of this crate.

If `tempfile` is missing, add it to `[dev-dependencies]` in `crates/trace-commons-contributor/Cargo.toml`:

```toml
tempfile = "3"
```

`tempfile` is already a dev-dependency of `trace-commons-server` in this workspace, so this adds no new third-party code to the dependency graph.

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --test update_manifest_roundtrip`
Expected: PASS, 3 tests.

- [ ] **Step 5: Document the key handling**

Create `scripts/updates/README.md`:

```markdown
# Update manifest publishing

`generate-manifest.sh` writes `latest.json` and a detached Ed25519 signature
over its exact bytes. Clients verify the signature before parsing; see
`crates/trace-commons-contributor/src/update/manifest.rs`.

## Keys

The private key lives in GCP Secret Manager as `update-manifest-signing-key`
in project `tracecommons-pilot-2026`, alongside `flatpak-signing-key`. It is
never written to a runner's disk outside the release job's temporary
directory, and never printed.

Generate a new key with:

    openssl genpkey -algorithm ed25519 -out update-signing.pem

Export the raw 32-byte public key that clients pin (the last 32 bytes of the
DER SubjectPublicKeyInfo):

    openssl pkey -in update-signing.pem -pubout -outform DER | tail -c 32 | xxd -p -c 32

## Rotation

Clients pin the public key at build time, so rotating it means shipping a
release signed by the old key that carries the new key, then switching. Do
not rotate without that two-step, or every installed client stops seeing
updates.
```

- [ ] **Step 6: Commit**

```bash
git add scripts/updates/ crates/trace-commons-contributor/tests/update_manifest_roundtrip.rs crates/trace-commons-contributor/Cargo.toml
git commit -m "Generate and sign the update manifest from the release pipeline"
```

---

### Task 4: Sparkle appcast generation

**Files:**
- Create: `scripts/updates/generate-appcast.sh`
- Modify: `scripts/updates/README.md` (add an appcast section)

**Interfaces:**
- Consumes: the same version and DMG metadata Task 3 consumes.
- Produces: `appcast.xml` on disk, consumed by Task 5's workflow job and by the macOS plan's Sparkle configuration.

Sparkle uses its own EdDSA key and its own `sign_update` tool, which ships inside the Sparkle distribution. It is a separate key from Task 3's because it is verified by a separate implementation; sharing one key across two verifiers means a bug in either compromises both.

- [ ] **Step 1: Write the script**

Create `scripts/updates/generate-appcast.sh`:

```bash
#!/usr/bin/env bash
# Generate the Sparkle appcast for the macOS app.
#
# Sparkle verifies this feed with its own EdDSA key AND the app's Developer ID
# code signature. Both must hold, so a compromised bucket alone cannot push an
# update.
set -euo pipefail

die() { echo "generate-appcast: $*" >&2; exit 1; }

SHORT_VERSION=""   # e.g. 0.2.0, shown to users
BUILD_VERSION=""   # CFBundleVersion, monotonic, what Sparkle compares
DMG_URL=""
DMG_PATH=""
SIGN_UPDATE=""     # path to Sparkle's sign_update binary
OUT="dist/updates/appcast.xml"

while [ $# -gt 0 ]; do
  case "$1" in
    --short-version) SHORT_VERSION="${2:?}"; shift 2 ;;
    --build-version) BUILD_VERSION="${2:?}"; shift 2 ;;
    --dmg-url)       DMG_URL="${2:?}"; shift 2 ;;
    --dmg-path)      DMG_PATH="${2:?}"; shift 2 ;;
    --sign-update)   SIGN_UPDATE="${2:?}"; shift 2 ;;
    --out)           OUT="${2:?}"; shift 2 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$SHORT_VERSION" ] || die "--short-version is required"
[ -n "$BUILD_VERSION" ] || die "--build-version is required"
[ -n "$DMG_URL" ] || die "--dmg-url is required"
[ -f "$DMG_PATH" ] || die "dmg not found: $DMG_PATH"
[ -x "$SIGN_UPDATE" ] || die "sign_update not found or not executable: $SIGN_UPDATE"

printf '%s' "$SHORT_VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "short version must be three-part numeric, got '$SHORT_VERSION'"

LENGTH="$(wc -c < "$DMG_PATH" | tr -d ' ')"

# sign_update prints an attribute fragment:
#   sparkle:edSignature="..." length="..."
# Take only the signature; the length is recomputed above from the file we
# are actually publishing.
SIGNATURE="$("$SIGN_UPDATE" "$DMG_PATH" | sed -E 's/.*sparkle:edSignature="([^"]+)".*/\1/')"
[ -n "$SIGNATURE" ] || die "sign_update produced no signature"

PUBDATE="$(date -u '+%a, %d %b %Y %H:%M:%S +0000')"

mkdir -p "$(dirname "$OUT")"
cat > "$OUT" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<rss version="2.0" xmlns:sparkle="http://www.andymatuschak.org/xml-namespaces/sparkle">
  <channel>
    <title>Trace Commons</title>
    <item>
      <title>$SHORT_VERSION</title>
      <pubDate>$PUBDATE</pubDate>
      <sparkle:minimumSystemVersion>14.0</sparkle:minimumSystemVersion>
      <enclosure
        url="$DMG_URL"
        sparkle:version="$BUILD_VERSION"
        sparkle:shortVersionString="$SHORT_VERSION"
        length="$LENGTH"
        type="application/octet-stream"
        sparkle:edSignature="$SIGNATURE" />
    </item>
  </channel>
</rss>
EOF

echo "wrote $OUT"
```

Make it executable:

```bash
chmod +x scripts/updates/generate-appcast.sh
```

- [ ] **Step 2: Verify the script rejects bad input**

Run each of these and confirm each exits non-zero with the stated message:

```bash
# A REAL regular file is required here: the `-f` DMG check runs before the
# version regex, so passing /dev/null would trip "dmg not found" instead and
# prove nothing about the version validation.
head -c 16 /dev/zero > /tmp/tc-fake.dmg
./scripts/updates/generate-appcast.sh --short-version 1.2 --build-version 100 \
  --dmg-url https://example.invalid/a.dmg --dmg-path /tmp/tc-fake.dmg --sign-update /bin/echo
```
Expected: exit 1, "short version must be three-part numeric, got '1.2'"

```bash
./scripts/updates/generate-appcast.sh --short-version 1.2.0 --build-version 100 \
  --dmg-url https://example.invalid/a.dmg --dmg-path /nonexistent --sign-update /bin/echo
```
Expected: exit 1, "dmg not found: /nonexistent"

- [ ] **Step 3: Append the appcast section to the README**

Add to `scripts/updates/README.md`:

```markdown
## Sparkle appcast

`generate-appcast.sh` writes `appcast.xml` for the macOS app. Sparkle's EdDSA
key is separate from the manifest key above and lives in GCP Secret Manager as
`sparkle-signing-key`. Generate it with Sparkle's `generate_keys` tool and
store the public key in the app's Info.plist as `SUPublicEDKey`.

Sparkle compares `sparkle:version` (CFBundleVersion), not the short version,
so the appcast must carry the same monotonic build number the release
workflow stamps into the bundle.
```

- [ ] **Step 4: Commit**

```bash
git add scripts/updates/
git commit -m "Generate the Sparkle appcast alongside the update manifest"
```

---

### Task 5: Publish both manifests from the release workflow

**Files:**
- Modify: `.github/workflows/release-apps.yml` (add a `publish-updates` job after the existing `publish` job)

**Interfaces:**
- Consumes: `scripts/updates/generate-manifest.sh` (Task 3), `scripts/updates/generate-appcast.sh` (Task 4), and the `macos-dmg` / `windows-zip` artifacts the existing build jobs upload.
- Produces: `updates/latest.json`, `updates/latest.json.sig`, and `updates/appcast.xml` in the `tracecommons-flatpak` bucket.

- [ ] **Step 1: Add the job**

Append to `.github/workflows/release-apps.yml`:

```yaml
  publish-updates:
    name: publish update manifests
    needs: [version, macos, windows, linux-flatpak, publish]
    # Runs only when the release itself was cut, and only if at least one
    # platform is genuinely publishable. A manifest is what installed clients
    # poll, so publishing one that names a platform whose build failed would
    # point every client of that platform at a URL that does not exist.
    if: >-
      ${{ always() && github.event_name == 'push' &&
          needs.publish.result == 'success' &&
          (needs.macos.result == 'success' || needs.windows.result == 'success') }}
    runs-on: macos-26
    # macos-26, not ubuntu: Sparkle's sign_update is a macOS binary, and this
    # job signs the appcast.
    environment: release
    timeout-minutes: 20
    steps:
      - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803 # v6

      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8
        with:
          path: dist
          pattern: "{macos-dmg,windows-zip}"

      - name: Authenticate to GCP (workload identity federation; no key on the runner)
        uses: google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093 # v3
        with:
          workload_identity_provider: ${{ secrets.GCP_WIF_PROVIDER }}
          service_account: ${{ secrets.GCP_FLATPAK_PUBLISHER_SA }}

      - uses: google-github-actions/setup-gcloud@aa5489c8933f4cc7a4f7d45035b3b1440c9c10db # v3.0.1

      # Both keys are fetched into RUNNER_TEMP and deleted by the runner when
      # the job ends. Neither is ever echoed: a `set -x` in this step would
      # print key material into a public log.
      - name: Fetch the signing keys from Secret Manager
        env:
          GCP_PROJECT: tracecommons-pilot-2026
        run: |
          set -euo pipefail
          gcloud secrets versions access latest \
            --secret=update-manifest-signing-key --project "$GCP_PROJECT" \
            > "$RUNNER_TEMP/update-signing.pem"
          chmod 600 "$RUNNER_TEMP/update-signing.pem"

      - name: Build the manifest from the platforms that actually succeeded
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
          MACOS_RESULT: ${{ needs.macos.result }}
          WINDOWS_RESULT: ${{ needs.windows.result }}
          REPO: ${{ github.repository }}
        run: |
          set -euo pipefail
          V="$SHORT_VERSION"
          BASE="https://github.com/$REPO/releases/download/app-v$V"
          ARGS=()

          if [ "$MACOS_RESULT" = success ]; then
            F="dist/macos-dmg/TraceCommons-$V.dmg"
            SHA="$(shasum -a 256 "$F" | awk '{print $1}')"
            SIZE="$(wc -c < "$F" | tr -d ' ')"
            ARGS+=(--platform "macos-universal=$BASE/TraceCommons-$V.dmg=$SHA=$SIZE")
          fi

          if [ "$WINDOWS_RESULT" = success ]; then
            F="dist/windows-zip/trace-commons-windows-x86_64-$V.zip"
            SHA="$(shasum -a 256 "$F" | awk '{print $1}')"
            SIZE="$(wc -c < "$F" | tr -d ' ')"
            ARGS+=(--platform "windows-x86_64=$BASE/trace-commons-windows-x86_64-$V.zip=$SHA=$SIZE")
          fi

          ./scripts/updates/generate-manifest.sh \
            --version "$V" \
            --key "$RUNNER_TEMP/update-signing.pem" \
            --out dist/updates \
            "${ARGS[@]}"

      - name: Verify the manifest we are about to publish
        run: |
          set -euo pipefail
          # Publishing an unverifiable manifest breaks every installed client
          # at once, and the failure would only surface on contributors'
          # machines. Verify here, against the same public key clients pin.
          openssl pkey -in "$RUNNER_TEMP/update-signing.pem" -pubout \
            -out "$RUNNER_TEMP/update-signing.pub.pem"
          openssl base64 -d -A -in dist/updates/latest.json.sig \
            -out "$RUNNER_TEMP/latest.sig.bin"
          openssl pkeyutl -verify -rawin \
            -pubin -inkey "$RUNNER_TEMP/update-signing.pub.pem" \
            -sigfile "$RUNNER_TEMP/latest.sig.bin" \
            -in dist/updates/latest.json
          cat dist/updates/latest.json

      - name: Publish to the bucket
        env:
          BUCKET: tracecommons-flatpak
        run: |
          set -euo pipefail
          # Cache-Control is short: this is the file that decides whether a
          # contributor ever learns a security fix shipped. A long edge cache
          # would silently extend that delay by hours.
          gcloud storage cp --cache-control="public, max-age=300" \
            dist/updates/latest.json dist/updates/latest.json.sig \
            "gs://$BUCKET/updates/"
```

- [ ] **Step 2: Validate the workflow parses**

Run: `gh workflow view release-apps.yml --repo zmanian/trace-commons-server` after pushing the branch, or validate locally with `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release-apps.yml'))"`
Expected: no parse error.

- [ ] **Step 3: Exercise the manifest path without publishing**

Run locally, standing in for the workflow:

```bash
openssl genpkey -algorithm ed25519 -out /tmp/tc-test-signing.pem
./scripts/updates/generate-manifest.sh \
  --version 9.9.9 --key /tmp/tc-test-signing.pem --out /tmp/tc-updates \
  --platform "macos-universal=https://example.invalid/a.dmg=$(printf 'a%.0s' {1..64})=1024"
cat /tmp/tc-updates/latest.json
```
Expected: a manifest containing exactly one platform key, `macos-universal`.

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-apps.yml
git commit -m "Publish signed update manifests when a release is cut"
```

---

### Task 6: Auto-bump the winget manifest on release

**Files:**
- Modify: `.github/workflows/release-apps.yml` (add a `winget-bump` job)

**Interfaces:**
- Consumes: `scripts/winget/generate-manifests.sh`, the `windows-zip` artifact.
- Produces: a pull request against `microsoft/winget-pkgs`.

Deferring to winget only helps contributors if winget actually learns about new versions. Without this, the Windows defer branch points at a package manager that never has the update, which is worse than no update path at all.

- [ ] **Step 1: Add the job**

Append to `.github/workflows/release-apps.yml`:

```yaml
  winget-bump:
    name: bump the winget manifest
    needs: [version, windows, publish]
    if: >-
      ${{ always() && github.event_name == 'push' &&
          needs.windows.result == 'success' && needs.publish.result == 'success' }}
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/checkout@d23441a48e516b6c34aea4fa41551a30e30af803 # v6

      - uses: actions/download-artifact@3e5f45b2cfb9172054b4087a40e8e0b5a5461e7c # v8
        with:
          name: windows-zip
          path: dist

      # Opens a pull request rather than pushing, matching the homebrew cask
      # bump above: winget-pkgs is a third-party repository with its own
      # review, and a bad manifest reaches everyone who runs `winget upgrade`.
      - name: Generate and submit
        env:
          GH_TOKEN: ${{ secrets.WINGET_PKGS_TOKEN }}
          SHORT_VERSION: ${{ needs.version.outputs.short }}
          REPO: ${{ github.repository }}
        run: |
          set -euo pipefail
          V="$SHORT_VERSION"
          ZIP="dist/trace-commons-windows-x86_64-$V.zip"
          SHA="$(sha256sum "$ZIP" | awk '{print $1}' | tr 'a-f' 'A-F')"
          URL="https://github.com/$REPO/releases/download/app-v$V/trace-commons-windows-x86_64-$V.zip"
          ./scripts/winget/generate-manifests.sh "$V" "$URL" "$SHA" > /dev/null
          gh repo clone microsoft/winget-pkgs winget-pkgs -- --depth 1
          DEST="winget-pkgs/manifests/t/TraceCommons/Contributor/$V"
          mkdir -p "$DEST"
          cp manifests/*.yaml "$DEST"/
          cd winget-pkgs
          git switch -c "TraceCommons.Contributor-$V"
          git config user.name "trace-commons-release"
          git config user.email "ops@tracecommons.ai"
          git add "manifests/t/TraceCommons/Contributor/$V"
          git commit -m "New version: TraceCommons.Contributor version $V"
          git push -u origin "TraceCommons.Contributor-$V"
          gh pr create --fill --repo microsoft/winget-pkgs
```

- [ ] **Step 2: Confirm the generator's argument shape matches**

Run: `head -40 scripts/winget/generate-manifests.sh`
Confirm the script takes version, URL, and SHA in that order and writes to `manifests/`. If it differs, adjust the invocation above to match the script rather than changing the script.

- [ ] **Step 3: Document the required secret**

Add to `scripts/updates/README.md`:

```markdown
## Winget

`WINGET_PKGS_TOKEN` is a fine-grained PAT with contents and pull-request write
on a fork of `microsoft/winget-pkgs`. `github.token` cannot reach another
repository, which is why this secret exists.
```

- [ ] **Step 4: Commit**

```bash
git add .github/workflows/release-apps.yml scripts/updates/README.md
git commit -m "Open a winget manifest bump when a Windows release is cut"
```

---

## Verification

After all tasks:

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
cargo test -p trace-commons-contributor update::
cargo test -p trace-commons-contributor --test update_manifest_roundtrip
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

All must pass before this plan is considered complete.

## Operator prerequisites

These are not code and must be done before the first release that uses this plan:

1. Create the Ed25519 manifest signing key and store it in GCP Secret Manager as `update-manifest-signing-key` in `tracecommons-pilot-2026`.
2. Create the Sparkle EdDSA key with Sparkle's `generate_keys` and store it as `sparkle-signing-key`.
3. Grant the existing `GCP_FLATPAK_PUBLISHER_SA` service account `secretmanager.versions.access` on both secrets and write access to `gs://tracecommons-flatpak/updates/`.
4. Create the `WINGET_PKGS_TOKEN` PAT against a fork of `microsoft/winget-pkgs`.
5. Record the raw 32-byte manifest public key; the CLI plan pins it at build time.
