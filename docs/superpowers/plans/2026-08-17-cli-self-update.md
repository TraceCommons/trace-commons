# CLI and Daemon Self-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an installed `trace-commons-contributor` binary on Windows, macOS, and Linux discover, verify, and replace itself from the signed update manifest — or, when winget owns the file, defer to winget and install nothing.

**Architecture:** A new `update` module inside `trace-commons-contributor` consumes the signed-manifest verifier and version comparator built by `docs/superpowers/plans/2026-08-17-update-manifest-publishing.md`. It detects who owns the installed bytes from the running executable's own path (no network), fetches and verifies `latest.json`, downloads and verifies the asset (sha256 everywhere, Authenticode on Windows), asks a running daemon to quiesce over a new IPC method, and then swaps the file. Every verification step fails closed, and the same conformance fixtures the Swift implementation uses are exercised here so a dropped check fails a test rather than shipping.

**Tech Stack:** Rust (`reqwest`, `sha2`, `serde`, `serde_json`, `ring`, `thiserror`, `tokio`, `hex`, `dirs`, `windows-sys` — all existing direct dependencies of `trace-commons-contributor`), bash + OpenSSL for fixture generation, PowerShell's `Get-AuthenticodeSignature` as the Windows signature verifier.

## Global Constraints

- **NO new Cargo dependencies.** `reqwest`, `sha2`, `serde`, `serde_json`, `ring`, `thiserror`, `tokio`, `dirs`, `hex`, `windows-sys` are already direct dependencies of `trace-commons-contributor`. Adding a dependency requires explicit user approval and is out of scope. Task 7 enables one additional *feature* of the already-present `windows-sys` (`Win32_Storage_FileSystem`); that is not a new dependency and is the only Cargo.toml change this plan makes.
- **Fail closed everywhere.** Any verification failure aborts, keeps the current binary, and backs off. There is no unverified fallback path and no flag that skips a check.
- **No downgrades.** The offered version must be strictly greater than the running version, checked with `is_newer` from the foundation plan.
- **Hash-only logging.** Never log URLs, tokens, signatures, certificate subjects, file bodies, or filesystem paths. Errors are fixed labels.
- Rust edition 2024, `rust-version = 1.92`.
- Verify with `RUSTFLAGS="-D warnings" cargo check` and `RUSTFLAGS="-D warnings" cargo test --no-run`; plain `cargo check` does not apply `-D warnings` but CI does.
- Clippy allow-list, exactly: `-A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen it.
- No emojis in code, commits, PRs, or docs. Short imperative commit subjects, no `feat:` / `fix:` prefixes.
- The Windows verification logic must not diverge from `scripts/install.ps1`: sha256 equality against the published digest, `Get-AuthenticodeSignature` status exactly `Valid`, and the signer subject containing `O=Iqlusion Inc`.

**Dependency on the foundation plan.** Tasks here consume `crates/trace-commons-contributor/src/update/manifest.rs` and `.../version.rs`, and the script `scripts/updates/generate-manifest.sh`, all created by `docs/superpowers/plans/2026-08-17-update-manifest-publishing.md`. That plan must be implemented first. The interfaces consumed, verbatim from it:

```rust
pub const UPDATE_MANIFEST_SCHEMA: &str;
pub struct UpdateManifest { pub schema_version: String, pub version: String, pub published_at: String, pub platforms: std::collections::BTreeMap<String, PlatformArtifact> }
pub struct PlatformArtifact { pub url: String, pub sha256: String, pub size: u64 }
pub enum ManifestError { MalformedSignature, BadSignature, MalformedJson, UnknownSchema }
pub fn verify_manifest(bytes: &[u8], signature_b64: &str, public_key: &[u8]) -> Result<UpdateManifest, ManifestError>;
pub enum VersionError { Malformed }
pub fn is_newer(current: &str, offered: &str) -> Result<bool, VersionError>;
```

---

### Task 1: Install-source detection

**Files:**
- Create: `crates/trace-commons-contributor/src/update/source.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod source;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum InstallSource { SelfManaged, WingetManaged }`
  - `pub const WINGET_PACKAGES_MARKER: &str`
  - `pub const WINGET_UPGRADE_COMMAND: &str`
  - `pub fn classify(exe: &std::path::Path) -> InstallSource`
  - `pub fn detect() -> Result<(InstallSource, std::path::PathBuf), SourceError>`
  - `pub enum SourceError { ExePathUnavailable }`

Winget is a hard defer: winget records portable-package versions in the registry, so a self-swap leaves that record stale and `winget upgrade --all` fights the binary forever. Detection is a pure path check with no network and no registry read, so it is testable on every platform.

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-contributor/src/update/source.rs` containing only this test module for now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_winget_package_path_is_winget_managed() {
        let p = Path::new(
            r"C:\Users\ada\AppData\Local\Microsoft\WinGet\Packages\TraceCommons.Contributor_Microsoft.Winget.Source_8wekyb3d8bbwe\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn the_marker_match_is_case_insensitive() {
        // Windows paths are case-insensitive and winget's own casing has
        // changed across releases ("Winget" vs "WinGet"). Matching on one
        // spelling would silently turn a defer into a self-swap, which is
        // the exact case that leaves winget offering a phantom upgrade
        // forever.
        let p = Path::new(
            r"c:\users\ada\appdata\local\microsoft\winget\packages\x\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn a_forward_slash_spelling_of_the_marker_still_matches() {
        let p = Path::new(
            "C:/Users/ada/AppData/Local/Microsoft/WinGet/Packages/x/trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::WingetManaged);
    }

    #[test]
    fn the_install_ps1_location_is_ours_to_replace() {
        let p = Path::new(
            r"C:\Users\ada\AppData\Local\Programs\TraceCommons\trace-commons-contributor.exe",
        );
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn the_install_sh_location_is_ours_to_replace() {
        let p = Path::new("/home/ada/.local/bin/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn a_project_directory_that_merely_mentions_winget_is_not_a_winget_install() {
        // The marker is the full segment run, not the word. A developer
        // running a debug build out of a checkout named "winget-work" owns
        // their own binary.
        let p = Path::new("/home/ada/src/winget-work/target/debug/trace-commons-contributor");
        assert_eq!(classify(p), InstallSource::SelfManaged);
    }

    #[test]
    fn detect_reports_a_real_path_for_the_running_test_binary() {
        let (_source, exe) = detect().expect("current_exe is available under cargo test");
        assert!(exe.is_absolute(), "exe path must be absolute");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::source`
Expected: FAIL — compilation error, `classify`, `InstallSource`, `detect` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/source.rs`:

```rust
//! Who owns the installed bytes.
//!
//! The governing rule of this subsystem is that whoever installed the binary
//! owns replacing it. This module answers that question from the running
//! executable's own path alone -- no network, no registry, no package-manager
//! invocation -- so the answer costs nothing and cannot be influenced by
//! anything remote.

use std::path::{Path, PathBuf};

/// The path segment run that means winget placed these bytes. Winget installs
/// portable packages under `%LOCALAPPDATA%\Microsoft\WinGet\Packages\<id>\`.
pub const WINGET_PACKAGES_MARKER: &str = r"\microsoft\winget\packages\";

/// What to tell a contributor whose copy winget owns. Printed verbatim; it is
/// a command, not a path, so it is safe to surface.
pub const WINGET_UPGRADE_COMMAND: &str = "winget upgrade TraceCommons.Contributor";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallSource {
    /// We placed these bytes (`install.ps1` into
    /// `%LOCALAPPDATA%\Programs\TraceCommons`, or `install.sh` into
    /// `~/.local/bin`), so we replace them.
    SelfManaged,
    /// Winget placed these bytes. Defer: a self-swap here leaves winget's
    /// registry record stale, so it would offer a phantom upgrade
    /// indefinitely and `winget upgrade --all` would fight us.
    WingetManaged,
}

#[derive(Debug, thiserror::Error)]
pub enum SourceError {
    /// `std::env::current_exe` failed. Refuse rather than guess: every
    /// downstream decision -- defer or swap, and which file to swap -- is
    /// derived from this path.
    #[error("update_source_exe_path_unavailable")]
    ExePathUnavailable,
}

/// Classify an executable path without touching the filesystem.
pub fn classify(exe: &Path) -> InstallSource {
    // Normalize separators and case before matching. Windows paths are
    // case-insensitive, winget's own casing has varied across releases, and a
    // path may arrive with either separator depending on how the process was
    // launched.
    let normalized = exe.to_string_lossy().replace('/', "\\").to_lowercase();
    if normalized.contains(WINGET_PACKAGES_MARKER) {
        InstallSource::WingetManaged
    } else {
        InstallSource::SelfManaged
    }
}

/// Classify the running executable, and return its path.
pub fn detect() -> Result<(InstallSource, PathBuf), SourceError> {
    let exe = std::env::current_exe().map_err(|_| SourceError::ExePathUnavailable)?;
    // Canonicalize when possible so a symlinked launcher is classified by
    // where the bytes actually live. A failure here is not fatal: the
    // uncanonicalized path is still the path we would swap.
    let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
    Ok((classify(&exe), exe))
}
```

Create `crates/trace-commons-contributor/src/update/mod.rs` if the foundation plan has not already, and ensure it contains (keeping the module list alphabetical):

```rust
//! Update discovery and installation.
//!
//! The manifest is the only thing a client trusts to learn that a new
//! version exists. It is signed because the transport is not: a public
//! bucket is a fine place to put bytes and a poor place to put authority.
pub mod manifest;
pub mod source;
pub mod version;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::source`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/
git commit -m "Detect whether winget or we own the installed binary"
```

---

### Task 2: Publish CLI binaries in the update manifest

**Files:**
- Modify: `scripts/updates/generate-manifest.sh` (the platform-slug allowlist)
- Modify: `scripts/updates/README.md` (a slug section)
- Modify: `.github/workflows/release-apps.yml` (the `publish-updates` job's manifest-building step)

**Interfaces:**
- Consumes: `scripts/updates/generate-manifest.sh` as created by the foundation plan's Task 3, and the `publish-updates` job created by its Task 5.
- Produces: four additional accepted platform slugs — `windows-x86_64-cli`, `linux-x86_64-cli`, `macos-aarch64-cli`, `macos-x86_64-cli` — carrying the bare, individually signed contributor binaries. Task 4 consumes these slug strings.

The foundation manifest advertises the desktop artifacts: a `.dmg` for macOS and a `.zip` for Windows. The CLI cannot consume either. It has no archive extractor and no new dependency may be added to give it one, and the Windows `.zip` is the winget payload — the very install source the CLI defers to. So the CLI needs its own slugs pointing at the bare binaries `install.sh` and `install.ps1` already download and verify. This is an additive change to the generator's allowlist; the manifest format and its verifier are untouched.

- [ ] **Step 1: Extend the generator's slug allowlist**

In `scripts/updates/generate-manifest.sh`, replace this case arm:

```bash
  case "$slug" in
    windows-x86_64|macos-universal|linux-x86_64) ;;
    *) die "unknown platform slug: $slug" ;;
  esac
```

with:

```bash
  # Two families, deliberately distinct. The bare `<os>-<arch>` slugs carry
  # the desktop artifacts (a .dmg, a .zip). The `-cli` slugs carry the single
  # signed contributor binary, which is what the CLI self-updater downloads:
  # it has no archive extractor and must not be pointed at the winget payload
  # it defers to.
  case "$slug" in
    windows-x86_64|macos-universal|linux-x86_64) ;;
    windows-x86_64-cli|linux-x86_64-cli|macos-aarch64-cli|macos-x86_64-cli) ;;
    *) die "unknown platform slug: $slug" ;;
  esac
```

Also update the usage text in the same file, replacing:

```
slugs: windows-x86_64 | macos-universal | linux-x86_64
```

with:

```
desktop slugs: windows-x86_64 | macos-universal | linux-x86_64
cli slugs:     windows-x86_64-cli | linux-x86_64-cli
               macos-aarch64-cli | macos-x86_64-cli
```

- [ ] **Step 2: Verify the generator accepts the new slugs and still refuses junk**

Run:

```bash
openssl genpkey -algorithm ed25519 -out /tmp/tc-slug-test.pem
./scripts/updates/generate-manifest.sh \
  --version 9.9.9 --key /tmp/tc-slug-test.pem --out /tmp/tc-slug-out \
  --platform "linux-x86_64-cli=https://example.invalid/tc=$(printf 'a%.0s' {1..64})=1024"
cat /tmp/tc-slug-out/latest.json
```
Expected: exit 0, and the manifest contains exactly the key `linux-x86_64-cli`.

Run:

```bash
./scripts/updates/generate-manifest.sh \
  --version 9.9.9 --key /tmp/tc-slug-test.pem --out /tmp/tc-slug-out \
  --platform "freebsd-x86_64-cli=https://example.invalid/tc=$(printf 'a%.0s' {1..64})=1024"
```
Expected: exit 1, `generate-manifest: unknown platform slug: freebsd-x86_64-cli`.

- [ ] **Step 3: Add the CLI assets to the release job**

In `.github/workflows/release-apps.yml`, inside the `publish-updates` job's `Build the manifest from the platforms that actually succeeded` step, insert the following immediately before the `./scripts/updates/generate-manifest.sh` invocation:

```bash
          # The CLI's own assets. These are published on the contributor-v*
          # tag stream, not app-v*, and are downloaded here so their digests
          # come from the bytes clients will actually receive rather than
          # from a value copied by hand.
          CLI_BASE="https://github.com/$REPO/releases/download/contributor-v$V"
          for pair in \
            "windows-x86_64-cli:trace-commons-contributor-x86_64-pc-windows-msvc.exe" \
            "linux-x86_64-cli:trace-commons-contributor-x86_64-unknown-linux-gnu" \
            "macos-aarch64-cli:trace-commons-contributor-aarch64-apple-darwin" \
            "macos-x86_64-cli:trace-commons-contributor-x86_64-apple-darwin"
          do
            SLUG="${pair%%:*}"
            NAME="${pair#*:}"
            if curl -fsSL --proto '=https' --tlsv1.2 "$CLI_BASE/$NAME" -o "dist/$NAME"; then
              SHA="$(shasum -a 256 "dist/$NAME" | awk '{print $1}')"
              SIZE="$(wc -c < "dist/$NAME" | tr -d ' ')"
              ARGS+=(--platform "$SLUG=$CLI_BASE/$NAME=$SHA=$SIZE")
            fi
            # A missing asset is skipped, not fatal: the CLI tag stream is
            # independent of the app tag stream, so a release that cut only
            # the desktop apps legitimately has none of these. An absent slug
            # means "no update for you"; a present-but-wrong one would point
            # every client of that platform at a 404.
          done
```

- [ ] **Step 4: Validate the workflow still parses**

Run: `python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-apps.yml'))"`
Expected: no output, exit 0.

- [ ] **Step 5: Document the slugs**

Append to `scripts/updates/README.md`:

```markdown
## Platform slugs

Two families:

| Slug | Artifact | Consumed by |
|---|---|---|
| `macos-universal` | `TraceCommons-<v>.dmg` | Sparkle (via the appcast, not this file) |
| `windows-x86_64` | `trace-commons-windows-x86_64-<v>.zip` | winget |
| `linux-x86_64` | flatpak repo | flatpak portal |
| `windows-x86_64-cli` | `trace-commons-contributor-x86_64-pc-windows-msvc.exe` | the CLI self-updater |
| `linux-x86_64-cli` | `trace-commons-contributor-x86_64-unknown-linux-gnu` | the CLI self-updater |
| `macos-aarch64-cli` | `trace-commons-contributor-aarch64-apple-darwin` | the CLI self-updater |
| `macos-x86_64-cli` | `trace-commons-contributor-x86_64-apple-darwin` | the CLI self-updater |

The CLI slugs carry a bare binary, never an archive: the CLI has no archive
extractor and no new dependency may be added to give it one. The desktop
`windows-x86_64` zip is the winget payload, and the CLI defers to winget
rather than consuming it.
```

- [ ] **Step 6: Commit**

```bash
git add scripts/updates/generate-manifest.sh scripts/updates/README.md .github/workflows/release-apps.yml
git commit -m "Advertise the bare CLI binaries in the update manifest"
```

---

### Task 3: Shared conformance fixtures

**Files:**
- Create: `tests/fixtures/update-conformance/regenerate.sh`
- Create: `tests/fixtures/update-conformance/README.md`
- Create (generated, then committed): `tests/fixtures/update-conformance/signing-key.pem`, `wrong-signing-key.pem`, `manifest-public-key.hex`, `good/latest.json`, `good/latest.json.sig`, `good/artifact.bin`, `bad-signature/latest.json`, `bad-signature/latest.json.sig`, `downgrade/latest.json`, `downgrade/latest.json.sig`, `tampered/artifact.bin`, `unsigned/artifact.exe`
- Create: `crates/trace-commons-contributor/tests/update_conformance.rs`

**Interfaces:**
- Consumes: `verify_manifest`, `ManifestError`, `UpdateManifest`, `is_newer` from the foundation plan.
- Produces: the fixture tree at `tests/fixtures/update-conformance/`, and the test-side helpers `fn fixture_dir() -> PathBuf`, `fn public_key() -> Vec<u8>`, `fn manifest_case(name: &str) -> (Vec<u8>, String)` inside `update_conformance.rs`. Later tasks add cases to this same file.

The verify-before-swap logic exists once here and once in Swift. These fixtures are the mitigation: both implementations consume the same bytes, so a check dropped in either fails a test rather than shipping. Repository root, not the crate, because the Swift tests are not in a Cargo crate.

- [ ] **Step 1: Write the fixture generator**

Create `tests/fixtures/update-conformance/regenerate.sh`:

```bash
#!/usr/bin/env bash
# Regenerate the shared update-conformance fixtures.
#
# Deterministic on purpose: the Ed25519 keys are built from fixed seeds and
# the manifests carry a fixed timestamp, so re-running this produces byte
# identical output and a regeneration shows up in a diff only when a fixture
# actually changed.
#
# These keys are test fixtures. They sign nothing that is ever published, and
# the private keys are committed deliberately so that both the Rust and the
# Swift test suites can re-derive the same signatures.
set -euo pipefail

cd "$(dirname "$0")"

die() { echo "regenerate: $*" >&2; exit 1; }
command -v openssl >/dev/null || die "openssl is required"
command -v xxd >/dev/null || die "xxd is required"
command -v jq >/dev/null || die "jq is required"

# An Ed25519 private key in PKCS#8 v1 is a fixed 16-byte DER prefix followed
# by the 32-byte seed, so a chosen seed yields a reproducible PEM.
write_key() {
  seed_hex="$1"
  out="$2"
  printf '302e020100300506032b657004220420%s' "$seed_hex" | xxd -r -p > "$out.der"
  {
    printf -- '-----BEGIN PRIVATE KEY-----\n'
    openssl base64 -in "$out.der"
    printf -- '-----END PRIVATE KEY-----\n'
  } > "$out"
  rm -f "$out.der"
}

write_key "$(printf '2a%.0s' $(seq 1 32))" signing-key.pem
write_key "$(printf '5b%.0s' $(seq 1 32))" wrong-signing-key.pem

# The raw 32-byte public key clients pin is the tail of the DER
# SubjectPublicKeyInfo.
openssl pkey -in signing-key.pem -pubout -outform DER \
  | tail -c 32 | xxd -p -c 32 > manifest-public-key.hex

mkdir -p good bad-signature downgrade tampered unsigned

printf 'trace-commons update conformance good artifact\n' > good/artifact.bin
# Same length, different bytes: the tampered case must fail on the digest and
# not incidentally on the size.
printf 'trace-commons update conformance EVIL artifact\n' > tampered/artifact.bin
printf 'not a signed windows binary\n' > unsigned/artifact.exe

SHA="$(openssl dgst -sha256 -hex good/artifact.bin | awk '{print $NF}')"
SIZE="$(wc -c < good/artifact.bin | tr -d ' ')"

write_manifest() {
  version="$1"
  out="$2"
  # Every CLI slug points at the same artifact so the fixtures exercise the
  # client on whatever host the test runs on.
  platforms=""
  for slug in windows-x86_64-cli linux-x86_64-cli macos-aarch64-cli macos-x86_64-cli; do
    entry="$(printf '"%s":{"url":"https://example.invalid/%s","sha256":"%s","size":%s}' \
               "$slug" "$slug" "$SHA" "$SIZE")"
    if [ -n "$platforms" ]; then platforms="$platforms,$entry"; else platforms="$entry"; fi
  done
  printf '{"schema_version":"trace_commons.update_manifest.v1","version":"%s","published_at":"2026-08-17T00:00:00Z","platforms":{%s}}' \
    "$version" "$platforms" | jq -S . > "$out"
}

sign() {
  key="$1"; in="$2"
  openssl pkeyutl -sign -rawin -inkey "$key" -in "$in" | openssl base64 -A > "$in.sig"
}

# The good case: a version no released build will ever reach, so it is always
# strictly newer than whatever is running.
write_manifest 9.9.9 good/latest.json
sign signing-key.pem good/latest.json

# A well-formed manifest signed by a key clients do not pin.
write_manifest 9.9.9 bad-signature/latest.json
sign wrong-signing-key.pem bad-signature/latest.json

# A correctly signed manifest for an old version. Replaying this at a client
# is a working attack unless the version comparison stops it, which is why
# this fixture exists separately from the bad-signature one.
write_manifest 0.0.1 downgrade/latest.json
sign signing-key.pem downgrade/latest.json

echo "regenerated fixtures in $(pwd)"
```

Make it executable and run it:

```bash
chmod +x tests/fixtures/update-conformance/regenerate.sh
./tests/fixtures/update-conformance/regenerate.sh
```
Expected: `regenerated fixtures in .../tests/fixtures/update-conformance`, and `ls` shows the four case directories plus the two keys and the hex file.

- [ ] **Step 2: Write the failing conformance test**

Create `crates/trace-commons-contributor/tests/update_conformance.rs`:

```rust
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
    let manifest = verify_manifest(&bytes, &sig, &public_key()).expect("downgrade fixture verifies");
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
```

- [ ] **Step 3: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test update_conformance`
Expected: FAIL — if the foundation plan's `update::manifest` and `update::version` are present the failure is at the `hex` import or the fixture path; if the fixtures were not generated in Step 1 it panics with "update-conformance fixtures are missing".

- [ ] **Step 4: Make it pass**

The fixtures generated in Step 1 and the foundation modules are the implementation. Confirm `hex` is listed under `[dependencies]` in `crates/trace-commons-contributor/Cargo.toml` (it is, as `hex = "0.4"`); nothing needs adding.

Run: `cargo test -p trace-commons-contributor --test update_conformance`
Expected: PASS, 5 tests.

- [ ] **Step 5: Document the fixtures**

Create `tests/fixtures/update-conformance/README.md`:

```markdown
# Update conformance fixtures

The verify-before-swap logic for automatic updates exists twice: once in Rust
(`crates/trace-commons-contributor/src/update/`) and once in Swift for the
macOS app. These fixtures are the mitigation for that duplication. Both suites
read these exact bytes, so a check dropped in either implementation fails a
test rather than shipping.

| Path | What it is | What must happen |
|---|---|---|
| `good/latest.json` + `.sig` | correctly signed, version `9.9.9` | verifies; is newer than any real build |
| `good/artifact.bin` | the artifact the good manifest publishes | sha256 and size match the manifest |
| `tampered/artifact.bin` | same length, different bytes | digest check refuses it |
| `bad-signature/latest.json` + `.sig` | signed by `wrong-signing-key.pem` | signature check refuses it |
| `downgrade/latest.json` + `.sig` | correctly signed, version `0.0.1` | verifies, then the version gate refuses it |
| `unsigned/artifact.exe` | a blob with no Authenticode signature | the Windows signature check refuses it |
| `manifest-public-key.hex` | raw 32-byte Ed25519 public key | what a client pins in these tests |

`signing-key.pem` and `wrong-signing-key.pem` are committed private keys. They
are test fixtures built from fixed seeds, they sign nothing that is ever
published, and they are committed so both suites can re-derive the same
signatures.

Regenerate with `./regenerate.sh`. It is deterministic: keys come from fixed
seeds and manifests carry a fixed `published_at`, so re-running changes
nothing unless a fixture genuinely changed.
```

- [ ] **Step 6: Commit**

```bash
git add tests/fixtures/update-conformance crates/trace-commons-contributor/tests/update_conformance.rs
git commit -m "Add shared update conformance fixtures and the Rust cases"
```

---

### Task 4: Pinned key, manifest endpoint, and this build's platform slug

**Files:**
- Create: `crates/trace-commons-contributor/src/update/endpoint.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod endpoint;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const MANIFEST_URL: &str`
  - `pub const MANIFEST_SIG_URL: &str`
  - `pub const MANIFEST_PUBLIC_KEY_HEX: &str`
  - `pub fn manifest_public_key() -> Result<Vec<u8>, EndpointError>`
  - `pub fn platform_slug() -> Result<&'static str, EndpointError>`
  - `pub enum EndpointError { NoPinnedKey, MalformedPinnedKey, UnsupportedPlatform }`

The public key is pinned at build time. An unset pin is a refusal, not a default: a build with no pinned key must not fall back to trusting whatever the bucket serves.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/endpoint.rs` with only this test module:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::endpoint`
Expected: FAIL — compilation error, `MANIFEST_URL`, `decode_pinned_key`, `platform_slug`, `EndpointError` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/endpoint.rs`:

```rust
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
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical:

```rust
pub mod endpoint;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::endpoint`
Expected: PASS, 5 tests.

Then run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor`
Expected: clean. (`#[allow(unreachable_code)]` is what keeps the always-returning `cfg` arms from tripping `-D warnings` on supported targets.)

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/
git commit -m "Pin the update manifest endpoint and signing key at build time"
```

---

### Task 5: Fetch and artifact verification

**Files:**
- Create: `crates/trace-commons-contributor/src/update/fetch.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod fetch;`)
- Modify: `crates/trace-commons-contributor/tests/update_conformance.rs` (add the tampered-artifact case)

**Interfaces:**
- Consumes: `PlatformArtifact` from the foundation plan.
- Produces:
  - `pub const MAX_MANIFEST_BYTES: usize`, `pub const MAX_SIGNATURE_BYTES: usize`, `pub const MAX_ASSET_BYTES: usize`
  - `pub enum FetchError { ClientBuild, Http, Status, TooLarge, SizeMismatch, DigestMismatch, Io }`
  - `pub fn http_client() -> Result<reqwest::Client, FetchError>`
  - `pub fn sha256_hex(bytes: &[u8]) -> String`
  - `pub fn verify_bytes(artifact: &PlatformArtifact, bytes: &[u8]) -> Result<(), FetchError>`
  - `pub async fn fetch_capped(client: &reqwest::Client, url: &str, max_bytes: usize) -> Result<Vec<u8>, FetchError>`
  - `pub async fn download_verified(client: &reqwest::Client, artifact: &PlatformArtifact, dest: &std::path::Path) -> Result<(), FetchError>`

`verify_bytes` is separated from the download so the digest logic is testable with no network at all, which is what the tampered fixture exercises.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/fetch.rs` with only this test module:

```rust
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
        assert!(MAX_SIGNATURE_BYTES < MAX_MANIFEST_BYTES);
        assert!(MAX_MANIFEST_BYTES < MAX_ASSET_BYTES);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::fetch`
Expected: FAIL — compilation error, `sha256_hex`, `verify_bytes`, `FetchError`, and the caps are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/fetch.rs`:

```rust
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

/// Download an artifact, verify it, and only then write it to `dest`.
///
/// The order is the point: nothing unverified is ever written to a path that
/// something else might later execute.
pub async fn download_verified(
    client: &reqwest::Client,
    artifact: &PlatformArtifact,
    dest: &Path,
) -> Result<(), FetchError> {
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
    Ok(())
}
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical:

```rust
pub mod fetch;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::fetch`
Expected: PASS, 7 tests.

- [ ] **Step 5: Add the tampered-artifact conformance case**

Append to `crates/trace-commons-contributor/tests/update_conformance.rs`:

```rust
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
```

- [ ] **Step 6: Run the conformance tests**

Run: `cargo test -p trace-commons-contributor --test update_conformance`
Expected: PASS, 6 tests.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/update/ crates/trace-commons-contributor/tests/update_conformance.rs
git commit -m "Fetch update artifacts under a cap and verify them before writing"
```

---

### Task 6: Authenticode verification on Windows

**Files:**
- Create: `crates/trace-commons-contributor/src/update/authenticode.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod authenticode;`)
- Modify: `crates/trace-commons-contributor/tests/update_conformance.rs` (add the unsigned-binary case)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const EXPECTED_SIGNER: &str` (`"O=Iqlusion Inc"`)
  - `pub enum AuthenticodeError { VerifierUnavailable, NotValid, UnexpectedSigner, MalformedVerifierOutput }`
  - `pub fn interpret(status: &str, subject: &str) -> Result<(), AuthenticodeError>`
  - `#[cfg(windows)] pub fn verify(path: &std::path::Path) -> Result<(), AuthenticodeError>`

**Choice of verifier, and why.** This shells out to the platform's own `Get-AuthenticodeSignature` at an absolute path rather than calling `WinVerifyTrust` through `windows-sys`. Both end at the same Win32 trust provider, but the checked property here is not "is a signature present" — `install.ps1` pins *who signed it*, and reproducing that through FFI means `WinVerifyTrust` plus `CryptQueryObject` plus `CertGetNameStringW`, several hundred lines of `unsafe` that cannot be compiled or exercised anywhere but a Windows runner. `install.ps1` is the stated reference for what "verified" means on Windows and the two must not diverge; invoking the same cmdlet it invokes makes divergence structurally hard rather than a thing to remember. The obvious risk of shelling out — a hijacked `powershell.exe` earlier on `PATH` — is closed by invoking the absolute path under `%SystemRoot%`, which is not user-writable.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/authenticode.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_signature_from_our_organisation_is_accepted() {
        assert!(
            interpret(
                "Valid",
                "CN=Iqlusion Inc, O=Iqlusion Inc, L=San Francisco, S=California, C=US"
            )
            .is_ok()
        );
    }

    #[test]
    fn an_unsigned_binary_is_refused() {
        assert!(matches!(
            interpret("NotSigned", "").unwrap_err(),
            AuthenticodeError::NotValid
        ));
    }

    #[test]
    fn every_non_valid_status_is_refused() {
        // Status-only checks that accept anything other than exactly "Valid"
        // are how a revoked or untrusted-root signature gets waved through.
        for status in ["HashMismatch", "UnknownError", "NotTrusted", "Incompatible"] {
            assert!(matches!(
                interpret(status, "O=Iqlusion Inc").unwrap_err(),
                AuthenticodeError::NotValid
            ));
        }
    }

    #[test]
    fn a_valid_signature_from_the_wrong_publisher_is_refused() {
        // This is the case a status-only check waves through, and it is the
        // one that matters: a validly signed binary from somebody else.
        assert!(matches!(
            interpret("Valid", "CN=Contoso Ltd, O=Contoso Ltd, C=US").unwrap_err(),
            AuthenticodeError::UnexpectedSigner
        ));
    }

    #[test]
    fn a_valid_status_with_no_subject_is_refused() {
        assert!(matches!(
            interpret("Valid", "").unwrap_err(),
            AuthenticodeError::UnexpectedSigner
        ));
    }

    #[test]
    fn the_status_comparison_ignores_surrounding_whitespace_only() {
        assert!(interpret(" Valid \r", " O=Iqlusion Inc ").is_ok());
        assert!(matches!(
            interpret("valid", "O=Iqlusion Inc").unwrap_err(),
            AuthenticodeError::NotValid
        ));
    }

    #[test]
    fn the_expected_signer_matches_install_ps1() {
        // scripts/install.ps1 pins $ExpectedSigner = 'O=Iqlusion Inc'. The
        // two must not drift.
        assert_eq!(EXPECTED_SIGNER, "O=Iqlusion Inc");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::authenticode`
Expected: FAIL — compilation error, `interpret`, `AuthenticodeError`, `EXPECTED_SIGNER` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/authenticode.rs`:

```rust
//! The Windows code-signature check, held to `scripts/install.ps1`'s standard.
//!
//! That script is the reference for what "verified" means on Windows and the
//! two must not diverge, so this asks the same question of the same platform
//! verifier: is the Authenticode status exactly `Valid`, and does the signing
//! subject name our organisation. Checking the signer, and not merely that a
//! signature is present, is the part that matters -- a validly signed binary
//! from an unexpected publisher is exactly the case a status-only check waves
//! through.
//!
//! Matching on the organisation rather than a full distinguished name is also
//! `install.ps1`'s choice: Azure Trusted Signing issues a fresh certificate
//! per signing job, so the leaf changes constantly while `O=` does not.

/// The substring the signing subject must contain. Kept identical to
/// `$ExpectedSigner` in `scripts/install.ps1`.
pub const EXPECTED_SIGNER: &str = "O=Iqlusion Inc";

#[derive(Debug, thiserror::Error)]
pub enum AuthenticodeError {
    /// The platform verifier could not be run at all. Fail closed: an
    /// unverifiable artifact is refused, never installed.
    #[error("update_authenticode_verifier_unavailable")]
    VerifierUnavailable,
    /// The verifier ran but its output was not the two lines expected.
    #[error("update_authenticode_malformed_verifier_output")]
    MalformedVerifierOutput,
    /// The signature is absent, or its status is anything other than `Valid`.
    #[error("update_authenticode_not_valid")]
    NotValid,
    /// A valid signature, from somebody who is not us.
    #[error("update_authenticode_unexpected_signer")]
    UnexpectedSigner,
}

/// Decide from a status and a subject. Separated from the platform call so
/// the decision is testable on every platform, including from the shared
/// conformance fixtures.
pub fn interpret(status: &str, subject: &str) -> Result<(), AuthenticodeError> {
    if status.trim() != "Valid" {
        return Err(AuthenticodeError::NotValid);
    }
    if !subject.contains(EXPECTED_SIGNER) {
        return Err(AuthenticodeError::UnexpectedSigner);
    }
    Ok(())
}

/// Verify the Authenticode signature on `path` using the platform verifier.
#[cfg(windows)]
pub fn verify(path: &std::path::Path) -> Result<(), AuthenticodeError> {
    use std::process::Command;

    // An absolute path under %SystemRoot%, never a bare `powershell.exe`: a
    // PATH lookup for the program that decides whether a binary is trusted
    // is a lookup an attacker who can write to PATH gets to answer.
    let system_root = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".to_string());
    let shell = std::path::Path::new(&system_root)
        .join(r"System32\WindowsPowerShell\v1.0\powershell.exe");

    // Single-quote the path for PowerShell and double any embedded quote.
    // `-LiteralPath` additionally stops PowerShell treating `[` and `]` in a
    // staging path as wildcards.
    let quoted = format!("'{}'", path.to_string_lossy().replace('\'', "''"));
    let script = format!(
        "$ErrorActionPreference='Stop'; \
         $s = Get-AuthenticodeSignature -LiteralPath {quoted}; \
         Write-Output ('STATUS=' + $s.Status); \
         if ($s.SignerCertificate) {{ Write-Output ('SUBJECT=' + $s.SignerCertificate.Subject) }} \
         else {{ Write-Output 'SUBJECT=' }}"
    );

    let output = Command::new(&shell)
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .map_err(|_| AuthenticodeError::VerifierUnavailable)?;
    if !output.status.success() {
        // stderr is deliberately not surfaced: it would carry the staging
        // path.
        return Err(AuthenticodeError::VerifierUnavailable);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut status = None;
    let mut subject = None;
    for line in stdout.lines() {
        if let Some(rest) = line.strip_prefix("STATUS=") {
            status = Some(rest.trim().to_string());
        } else if let Some(rest) = line.strip_prefix("SUBJECT=") {
            subject = Some(rest.trim().to_string());
        }
    }
    match (status, subject) {
        (Some(status), Some(subject)) => interpret(&status, &subject),
        _ => Err(AuthenticodeError::MalformedVerifierOutput),
    }
}
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical (this goes first):

```rust
pub mod authenticode;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::authenticode`
Expected: PASS, 7 tests.

- [ ] **Step 5: Add the unsigned-binary conformance case**

Append to `crates/trace-commons-contributor/tests/update_conformance.rs`:

```rust
#[test]
fn the_unsigned_fixture_is_refused_by_the_signature_decision() {
    use trace_commons_contributor::update::authenticode::{AuthenticodeError, interpret};
    // The fixture exists on every platform; what the platform verifier would
    // report for it is `NotSigned`, and that is what must be refused.
    let bytes = std::fs::read(fixture_dir().join("unsigned/artifact.exe")).expect("unsigned");
    assert!(!bytes.is_empty());
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
```

- [ ] **Step 6: Run the conformance tests**

Run: `cargo test -p trace-commons-contributor --test update_conformance`
Expected: PASS, 7 tests on unix (the `#[cfg(windows)]` case is not compiled), 8 on Windows.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-contributor/src/update/ crates/trace-commons-contributor/tests/update_conformance.rs
git commit -m "Check the Windows signature status and signer before installing"
```

---

### Task 7: Binary swap

**Files:**
- Create: `crates/trace-commons-contributor/src/update/swap.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod swap;`)
- Modify: `crates/trace-commons-contributor/Cargo.toml` (add the `Win32_Storage_FileSystem` feature to the existing `windows-sys` dependency)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub enum SwapError { Io }`
  - `pub fn swap_in_place(new_binary: &std::path::Path, target: &std::path::Path) -> Result<(), SwapError>`

Unix replaces the directory entry, which is why a running process keeps working: it holds the old inode. Windows cannot unlink a running image at all, so the running file is renamed aside first, and if it cannot then be deleted it is scheduled for deletion at the next boot with `MoveFileExW(.., NULL, MOVEFILE_DELAY_UNTIL_REBOOT)` — the same `PendingFileRenameOperations` mechanism installers have always used.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/swap.rs` with only this test module:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_new_bytes_replace_the_old_ones() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&target, b"old").unwrap();
        std::fs::write(&staged, b"new").unwrap();

        swap_in_place(&staged, &target).expect("swap");

        assert_eq!(std::fs::read(&target).unwrap(), b"new");
        assert!(!staged.exists(), "the staged file is consumed by the swap");
    }

    #[test]
    fn a_missing_target_is_still_installed_into() {
        // First install through the updater, or a target somebody removed
        // between staging and applying. Placing the verified binary is still
        // the right outcome.
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&staged, b"new").unwrap();

        swap_in_place(&staged, &target).expect("swap");
        assert_eq!(std::fs::read(&target).unwrap(), b"new");
    }

    #[test]
    fn a_missing_staged_binary_is_an_error_and_leaves_the_target_alone() {
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        std::fs::write(&target, b"old").unwrap();

        assert!(swap_in_place(&d.path().join("nope"), &target).is_err());
        assert_eq!(
            std::fs::read(&target).unwrap(),
            b"old",
            "a failed swap must not destroy the working binary"
        );
    }

    #[cfg(unix)]
    #[test]
    fn the_installed_binary_is_executable() {
        use std::os::unix::fs::PermissionsExt;
        let d = tempfile::tempdir().unwrap();
        let target = d.path().join("trace-commons-contributor");
        let staged = d.path().join("staged-binary");
        std::fs::write(&staged, b"new").unwrap();
        std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o600)).unwrap();

        swap_in_place(&staged, &target).expect("swap");
        let mode = std::fs::metadata(&target).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o755, "installed mode was {:o}", mode & 0o777);
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::swap`
Expected: FAIL — compilation error, `swap_in_place` and `SwapError` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/swap.rs`:

```rust
//! Replacing the installed binary.
//!
//! Unix replaces the directory entry with a rename, which is atomic within a
//! filesystem and is why a process already running the old image keeps
//! working: it holds the inode, not the name. Windows cannot unlink a running
//! image at all, so the running file is renamed aside first and then deleted
//! -- or, when it is still mapped, scheduled for deletion at the next boot.
//!
//! Both paths require `new_binary` and `target` to be on the same filesystem,
//! which is why the staging directory lives beside the target (see
//! `update::stage`).

use std::path::Path;

#[derive(Debug, thiserror::Error)]
pub enum SwapError {
    #[error("update_swap_io_failed")]
    Io,
}

/// Move `new_binary` into `target`, replacing whatever is there.
pub fn swap_in_place(new_binary: &Path, target: &Path) -> Result<(), SwapError> {
    if !new_binary.is_file() {
        return Err(SwapError::Io);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(new_binary, std::fs::Permissions::from_mode(0o755))
            .map_err(|_| SwapError::Io)?;
        // Unlink rather than overwrite: writing over a running binary is how
        // a half-written executable happens, and a stale file's mode would
        // otherwise survive. A missing target is fine.
        match std::fs::remove_file(target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(SwapError::Io),
        }
        std::fs::rename(new_binary, target).map_err(|_| SwapError::Io)?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        let aside = target.with_extension(format!(
            "old-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let moved_aside = match std::fs::rename(target, &aside) {
            Ok(()) => true,
            // Nothing to move aside is not a failure.
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => return Err(SwapError::Io),
        };
        if let Err(_e) = std::fs::rename(new_binary, target) {
            // Put the working binary back. Leaving the install with no
            // executable at all is a strictly worse outcome than not
            // updating.
            if moved_aside {
                let _ = std::fs::rename(&aside, target);
            }
            return Err(SwapError::Io);
        }
        if moved_aside {
            schedule_delete(&aside);
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Err(SwapError::Io)
}

/// Delete the displaced binary, or arrange for the OS to do it at the next
/// boot when it is still mapped by the running process.
///
/// Best effort by design: the swap has already succeeded at this point, and a
/// leftover `.old-<n>` file is a wart, not a failure to report to a
/// contributor.
#[cfg(windows)]
fn schedule_delete(path: &Path) {
    use std::os::windows::ffi::OsStrExt;

    if std::fs::remove_file(path).is_ok() {
        return;
    }
    let mut wide: Vec<u16> = path.as_os_str().encode_wide().collect();
    wide.push(0);
    // A NULL destination with MOVEFILE_DELAY_UNTIL_REBOOT registers the path
    // in PendingFileRenameOperations for deletion at the next boot. Requires
    // administrator rights on some systems; a failure is ignored for the same
    // reason the remove_file failure above is.
    unsafe {
        let _ = windows_sys::Win32::Storage::FileSystem::MoveFileExW(
            wide.as_ptr(),
            std::ptr::null(),
            windows_sys::Win32::Storage::FileSystem::MOVEFILE_DELAY_UNTIL_REBOOT,
        );
    }
}
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical:

```rust
pub mod swap;
```

In `crates/trace-commons-contributor/Cargo.toml`, add one feature to the existing `windows-sys` entry — this is a feature of a dependency already present, not a new dependency:

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.61", features = [
    "Win32_Foundation",
    "Win32_Security",
    "Win32_Security_Authorization",
    "Win32_Storage_FileSystem",
    "Win32_System_Threading",
] }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::swap`
Expected: PASS, 4 tests on unix (3 plus the unix-only mode check), 3 on Windows.

Then run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/ crates/trace-commons-contributor/Cargo.toml
git commit -m "Replace the installed binary in place on unix and Windows"
```

---

### Task 8: Staging a verified update

**Files:**
- Create: `crates/trace-commons-contributor/src/update/stage.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod stage;`)

**Interfaces:**
- Consumes: nothing.
- Produces:
  - `pub const STAGED_UPDATE_SCHEMA: &str`, `pub const STAGE_DIR_NAME: &str`, `pub const STAGE_RECORD_FILE: &str`, `pub const STAGED_BINARY_FILE: &str`
  - `pub struct StagedUpdate { pub schema_version: String, pub version: String, pub sha256: String, pub staged_at: String }`
  - `pub enum StageError { Io, Malformed, UnknownSchema }`
  - `pub fn stage_dir(target_exe: &std::path::Path) -> std::path::PathBuf`
  - `pub fn staged_binary_path(target_exe: &std::path::Path) -> std::path::PathBuf`
  - `pub fn prepare(target_exe: &std::path::Path) -> Result<std::path::PathBuf, StageError>`
  - `pub fn write_record(target_exe: &std::path::Path, staged: &StagedUpdate) -> Result<(), StageError>`
  - `pub fn read_record(target_exe: &std::path::Path) -> Result<Option<StagedUpdate>, StageError>`
  - `pub fn clear(target_exe: &std::path::Path) -> Result<(), StageError>`

The staging directory sits beside the target so the swap is a same-filesystem rename. The record carries the digest so the staged bytes are re-verified at apply time — a staged update may sit on disk across a reboot, and what is applied must be what was verified.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/stage.rs` with only this test module:

```rust
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::stage`
Expected: FAIL — compilation error, `StagedUpdate`, `stage_dir`, `prepare`, `write_record`, `read_record`, `clear`, `StageError` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/stage.rs`:

```rust
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
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical:

```rust
pub mod stage;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::stage`
Expected: PASS, 7 tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/
git commit -m "Stage a verified update beside the installed binary"
```

---

### Task 9: The quiesce IPC method

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/ipc.rs` (the `METHODS` array, `DaemonShared`, `handle_request`, `handle_request_async`, plus a new `handle_quiesce`)
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (`drain_approved`'s gate)
- Modify: `docs/contributor-daemon-ipc-v1_1.md` (the method table and a section)

**Interfaces:**
- Consumes: nothing from earlier tasks.
- Produces:
  - `pub const ERR_QUIESCE_TIMEOUT: &str` (`"quiesce-timeout"`)
  - `pub const DEFAULT_QUIESCE_TIMEOUT_SECS: u64` (`60`), `pub const MAX_QUIESCE_TIMEOUT_SECS: u64` (`300`)
  - `DaemonShared::quiesced: std::sync::atomic::AtomicBool`
  - the `"quiesce"` IPC method: params `{"timeout_secs": u64?}`, result `{"quiesced": true, "waited_ms": u64}`, refusal `busy` / `quiesce-timeout`
  - Task 10 calls this over `daemon::client::try_call`.

**Why a flag beside pause rather than pause itself.** Quiesce shares pause's gate — the single `is_paused` check at the top of `drain_approved`, which is where "nothing leaves this machine" is enforced — but it does not share pause's *state*. Pause is persisted in `daemon-state.json` and survives a restart, deliberately, because a contributor who paused meant it. An update that set that flag would rewrite a contributor's own setting, and a crash between quiesce and swap would leave the daemon paused forever with no record of who paused it. Quiesce is in-memory only and dies with the process, which is exactly right: after the swap there is a new process, and there is nothing left to un-quiesce.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod tests` in `crates/trace-commons-contributor/src/daemon/ipc.rs`:

```rust
    #[tokio::test]
    async fn quiesce_parks_the_queue_when_nothing_is_in_flight() {
        let s = shared();
        let r = handle_request_async(&s, &req("quiesce", serde_json::json!({}))).await;
        let v = r.result.expect("quiesce should succeed with an idle queue");
        assert_eq!(v["quiesced"], true);
        assert!(s.quiesced.load(Ordering::Relaxed), "the flag must be set");
    }

    #[tokio::test]
    async fn quiesce_times_out_rather_than_forcing_its_way_past_an_upload() {
        let s = shared();
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Uploading,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                    },
                    500,
                )
                .unwrap();
        }
        let r = handle_request_async(&s, &req("quiesce", serde_json::json!({"timeout_secs": 1})))
            .await;
        let err = r.error.expect("an in-flight upload must not be abandoned");
        assert_eq!(err.code, ERR_BUSY);
        assert_eq!(err.message, ERR_QUIESCE_TIMEOUT);
        // A failed quiesce must leave the daemon working: the update stays
        // staged and retries, rather than parking uploads indefinitely.
        assert!(!s.quiesced.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn quiesce_completes_once_the_in_flight_upload_finishes() {
        let s = std::sync::Arc::new(shared());
        let entry_id = uuid::Uuid::new_v4();
        {
            let mut queue = s.queue.lock().unwrap();
            queue
                .upsert(
                    super::super::queue::QueueEntry {
                        entry_id,
                        session_hash: "sha256:seed".to_string(),
                        source: "claude-code".to_string(),
                        project_key: "/tmp/p".to_string(),
                        project_label: "p".to_string(),
                        path: std::path::PathBuf::from("/tmp/seed.jsonl"),
                        size_bytes: 1,
                        discovered_at: Utc::now(),
                        state: QueueState::Uploading,
                        reason_label: None,
                        attempts: 0,
                        retry_after: None,
                        submission_id: None,
                        approved_scopes: None,
                        approved_inputs: None,
                        previewed_envelope_digest: None,
                        approved_at: None,
                    },
                    500,
                )
                .unwrap();
        }
        let finisher = std::sync::Arc::clone(&s);
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
            let mut queue = finisher.queue.lock().unwrap();
            queue.set_state(entry_id, QueueState::Uploaded, None);
        });
        let r = handle_request_async(&s, &req("quiesce", serde_json::json!({"timeout_secs": 10})))
            .await;
        assert_eq!(r.result.expect("drained")["quiesced"], true);
    }

    #[test]
    fn a_synchronous_quiesce_is_refused_rather_than_answered_wrongly() {
        let s = shared();
        let r = handle_request(&s, &req("quiesce", serde_json::json!({})));
        let err = r.error.unwrap();
        assert_eq!(err.code, ERR_UNAVAILABLE);
        assert_eq!(err.message, "quiesce-requires-async");
    }

    #[tokio::test]
    async fn an_absurd_quiesce_timeout_is_capped_rather_than_honoured() {
        let s = shared();
        let r = handle_request_async(
            &s,
            &req("quiesce", serde_json::json!({"timeout_secs": 999_999})),
        )
        .await;
        // The queue is idle, so this returns immediately; the point is that a
        // caller cannot ask the daemon to park uploads for a week.
        assert_eq!(r.result.expect("idle")["quiesced"], true);
        assert_eq!(clamp_quiesce_timeout(Some(999_999)), MAX_QUIESCE_TIMEOUT_SECS);
        assert_eq!(clamp_quiesce_timeout(None), DEFAULT_QUIESCE_TIMEOUT_SECS);
        assert_eq!(clamp_quiesce_timeout(Some(0)), DEFAULT_QUIESCE_TIMEOUT_SECS);
        assert_eq!(clamp_quiesce_timeout(Some(5)), 5);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor daemon::ipc::tests::quiesce`
Expected: FAIL — compilation error, `s.quiesced`, `ERR_QUIESCE_TIMEOUT`, `clamp_quiesce_timeout`, `MAX_QUIESCE_TIMEOUT_SECS`, `DEFAULT_QUIESCE_TIMEOUT_SECS` do not exist.

- [ ] **Step 3: Write the minimal implementation**

In `crates/trace-commons-contributor/src/daemon/ipc.rs`:

Add beside the other error-label constants (after `ERR_UNKNOWN_ENTRY_ID`):

```rust
/// `quiesce` gave up waiting for in-flight uploads to finish. The caller
/// leaves the update staged and tries again later; the swap never forces its
/// way past active work, because a half-uploaded trace is not an acceptable
/// cost for an update.
pub const ERR_QUIESCE_TIMEOUT: &str = "quiesce-timeout";

/// How long `quiesce` waits for in-flight uploads by default.
pub const DEFAULT_QUIESCE_TIMEOUT_SECS: u64 = 60;
/// The longest a caller may ask `quiesce` to park uploads for.
pub const MAX_QUIESCE_TIMEOUT_SECS: u64 = 300;
/// How often the drain is re-checked while waiting.
const QUIESCE_POLL_MS: u64 = 200;
```

Change the `METHODS` array to 28 entries, inserting `"quiesce"` in alphabetical order (after `"queue_outcome_counts"`, because `e` sorts before `i`):

```rust
pub const METHODS: [&str; 28] = [
    "acknowledge_near_ai_notice",
    "approve",
    "cancel",
    "consent_options",
    "dismiss",
    "enroll",
    "get_settings",
    "hello",
    "history_rollup",
    "list_audit",
    "list_history",
    "list_pending",
    "list_projects",
    "pause",
    "preview",
    "preview_body",
    "queue_outcome_counts",
    "quiesce",
    "refresh_history",
    "resume",
    "set_consent_scopes",
    "set_project_mode",
    "set_settings",
    "shutdown",
    "status",
    "subscribe",
    "withdraw",
    "withdraw_bulk",
];
```

Add the field to `DaemonShared`, after `paused`:

```rust
    /// Uploads are parked for an update swap.
    ///
    /// Deliberately *not* `paused`. Pause is the contributor's own setting
    /// and is persisted in `daemon-state.json`; an update that set it would
    /// be rewriting their preference, and a crash between quiescing and
    /// swapping would leave the daemon paused forever with nothing to say
    /// why. This flag is in-memory only and dies with the process, which is
    /// exactly the lifetime an update swap needs: after the swap there is a
    /// new process and nothing left to un-quiesce.
    pub quiesced: AtomicBool,
```

and in `DaemonShared::load`'s struct literal, after `paused: AtomicBool::new(paused),`:

```rust
            quiesced: AtomicBool::new(false),
```

Add the clamp helper and the handler near `handle_preview`:

```rust
/// The timeout `quiesce` will actually honour.
///
/// A caller cannot park uploads for a week, and a caller that asks for zero
/// gets the default rather than an instant refusal.
fn clamp_quiesce_timeout(requested: Option<u64>) -> u64 {
    match requested {
        Some(0) | None => DEFAULT_QUIESCE_TIMEOUT_SECS,
        Some(n) => n.min(MAX_QUIESCE_TIMEOUT_SECS),
    }
}

/// Park the upload queue and wait for anything already in flight to finish.
///
/// The flag is set first, so nothing new is claimed while the wait runs, and
/// then in-flight work is allowed to complete on its own terms. On timeout
/// the flag is cleared and the caller is refused: the update stays staged and
/// retries later. There is no forced path -- a half-uploaded trace is not an
/// acceptable cost for an update.
async fn handle_quiesce(shared: &DaemonShared, req: &Request) -> Response {
    let requested = match req.params.get("timeout_secs") {
        None => None,
        Some(v) => match v.as_u64() {
            Some(n) => Some(n),
            None => return Response::err(req.id, ERR_BAD_PARAMS, "timeout-secs-invalid"),
        },
    };
    let timeout = std::time::Duration::from_secs(clamp_quiesce_timeout(requested));

    shared.quiesced.store(true, Ordering::Relaxed);
    let started = std::time::Instant::now();
    loop {
        let in_flight = {
            let queue = shared.queue.lock().expect("queue lock");
            queue
                .all()
                .iter()
                .any(|e| e.state == QueueState::Uploading)
        };
        if !in_flight {
            return Response::ok(
                req.id,
                serde_json::json!({
                    "quiesced": true,
                    "waited_ms": started.elapsed().as_millis() as u64,
                }),
            );
        }
        if started.elapsed() >= timeout {
            shared.quiesced.store(false, Ordering::Relaxed);
            return Response::err(req.id, ERR_BUSY, ERR_QUIESCE_TIMEOUT);
        }
        tokio::time::sleep(std::time::Duration::from_millis(QUIESCE_POLL_MS)).await;
    }
}
```

Add the synchronous refusal arm in `handle_request`, beside `"preview_body"`'s:

```rust
        // Waiting for a drain is async by nature; the synchronous dispatcher
        // cannot do it and says so rather than claiming a quiesce it did not
        // perform. See the module doc's "Sync vs. async dispatch" section.
        "quiesce" => Response::err(req.id, ERR_UNAVAILABLE, "quiesce-requires-async"),
```

Add the real dispatch in `handle_request_async`, after the `"preview_body"` arm:

```rust
        "quiesce" => handle_quiesce(shared, req).await,
```

And update `handle_request_async`'s doc comment, which lists the async methods, to read:

```rust
/// The complete dispatcher: answers the async methods (`"preview"`,
/// `"preview_body"`, `"quiesce"`, `"enroll"`, `"withdraw"`,
/// `"withdraw_bulk"`) for real
```

In `crates/trace-commons-contributor/src/daemon/mod.rs`, change `drain_approved`'s gate from:

```rust
    if shared.is_paused(now) {
        return Ok(());
    }
```

to:

```rust
    if shared.is_paused(now) {
        return Ok(());
    }
    // Quiesced for an update swap. Same gate as pause and for the same
    // reason -- this is the one place "nothing leaves this machine" is
    // enforced -- but a separate, in-memory flag, so an update never rewrites
    // the contributor's own persisted pause setting. See `DaemonShared::
    // quiesced`.
    if shared.quiesced.load(Ordering::Relaxed) {
        return Ok(());
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor daemon::ipc::tests::quiesce daemon::ipc::tests::a_synchronous_quiesce daemon::ipc::tests::an_absurd_quiesce`
Expected: PASS, 5 tests.

Run the contract test, which asserts `hello` advertises exactly `METHODS`:
`cargo test -p trace-commons-contributor --test daemon_ipc_contract`
Expected: PASS (this is what catches a `METHODS` array whose declared length or ordering was not updated).

- [ ] **Step 5: Document the method**

In `docs/contributor-daemon-ipc-v1_1.md`, add this row to the method table immediately after the `queue_outcome_counts` row:

```markdown
| `quiesce` | `timeout_secs` (optional, default 60, max 300) | `quiesced: true`, `waited_ms` | parks uploads for an update swap; `busy` / `quiesce-timeout` if in-flight work does not finish in time |
```

And add this section immediately after the `### queue_outcome_counts` section:

```markdown
### `quiesce`

```json
{ "quiesced": true, "waited_ms": 412 }
```

Parks the upload queue and waits for anything already in flight to finish, so
an update can replace the binary without abandoning a half-uploaded trace.
Used by `trace-commons-contributor update`.

The park is in-memory and dies with the daemon process. It is deliberately not
`pause`: pause is the contributor's own persisted setting, and an update must
not rewrite it. There is no `unquiesce` verb for the same reason — the process
that was quiesced is the process the swap replaces.

On timeout the daemon answers `busy` / `quiesce-timeout` and un-parks itself.
The caller leaves the update staged and retries later. There is no forced
path.
```

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/daemon/ipc.rs crates/trace-commons-contributor/src/daemon/mod.rs docs/contributor-daemon-ipc-v1_1.md
git commit -m "Add a quiesce IPC method that drains uploads before an update"
```

---

### Task 10: The update flow

**Files:**
- Create: `crates/trace-commons-contributor/src/update/run.rs`
- Modify: `crates/trace-commons-contributor/src/update/mod.rs` (add `pub mod run;`)
- Modify: `crates/trace-commons-contributor/src/daemon/mod.rs` (apply a staged update in `start_embedded`)

**Interfaces:**
- Consumes: `source::{detect, classify, InstallSource, WINGET_UPGRADE_COMMAND}`, `endpoint::{MANIFEST_URL, MANIFEST_SIG_URL, manifest_public_key, platform_slug}`, `manifest::{verify_manifest, PlatformArtifact}`, `version::is_newer`, `fetch::{http_client, fetch_capped, download_verified, verify_bytes, sha256_hex, MAX_MANIFEST_BYTES, MAX_SIGNATURE_BYTES}`, `authenticode::verify` (Windows), `stage::{prepare, write_record, read_record, clear, staged_binary_path, StagedUpdate, STAGED_UPDATE_SCHEMA}`, `swap::swap_in_place`, `daemon::client::try_call`, `daemon::ipc::ERR_QUIESCE_TIMEOUT`.
- Produces:
  - `pub enum UpdateMode { Stage, Apply }`
  - `pub enum UpdateOutcome { DeferredToWinget, UpToDate { version: String }, NoArtifactForPlatform, Staged { version: String }, Applied { version: String }, QuiesceTimedOutStaged { version: String } }`
  - `pub enum UpdateError { Source, Endpoint, Manifest, Version, Fetch, Authenticode, Stage, Swap }`
  - `pub async fn check_and_install(store: &crate::config::ConfigStore, mode: UpdateMode) -> Result<UpdateOutcome, UpdateError>`
  - `pub fn apply_staged(target_exe: &std::path::Path) -> Result<Option<String>, UpdateError>`
  - Task 11 renders `UpdateOutcome`.

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor/src/update/run.rs` with only this test module:

```rust
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
        let d = tempfile::tempdir().unwrap();
        let target = write_staged(d.path(), b"new binary", "9.9.9");
        let applied = apply_staged(&target).unwrap().expect("applied");
        assert_eq!(applied, "9.9.9");
        assert_eq!(std::fs::read(&target).unwrap(), b"new binary");
        assert!(stage::read_record(&target).unwrap().is_none());
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
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor update::run`
Expected: FAIL — compilation error, `apply_staged`, `UpdateError`, `select_artifact` are not defined.

- [ ] **Step 3: Write the minimal implementation**

Prepend to `crates/trace-commons-contributor/src/update/run.rs`:

```rust
//! The update flow, end to end.
//!
//! Detect who owns the bytes, fetch and verify the manifest, compare
//! versions, download and verify the artifact, quiesce a running daemon, and
//! only then swap. Every step fails closed, and nothing unverified is ever
//! written where it could be executed.

use std::collections::BTreeMap;
use std::path::Path;

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
    /// Winget owns these bytes. Nothing was installed, by design.
    DeferredToWinget,
    /// The running version is already at or past what is published.
    UpToDate { version: String },
    /// The manifest advertises no artifact for this platform, which is the
    /// normal state when that platform's build job failed.
    NoArtifactForPlatform,
    Staged { version: String },
    Applied { version: String },
    /// Verified and staged, but a running daemon would not drain in time.
    /// Nothing was swapped; the next attempt picks the staged update up.
    QuiesceTimedOutStaged { version: String },
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
    if install_source == InstallSource::WingetManaged {
        // The rule is that whoever installed the binary owns replacing it.
        // No fetch happens here at all -- not even a check -- so a deferred
        // install makes no network request whatsoever.
        return Ok(UpdateOutcome::DeferredToWinget);
    }

    let public_key = endpoint::manifest_public_key().map_err(|_| UpdateError::Endpoint)?;
    let slug = endpoint::platform_slug().map_err(|_| UpdateError::Endpoint)?;
    let client = fetch::http_client().map_err(|_| UpdateError::Fetch)?;

    let manifest_bytes = fetch::fetch_capped(&client, endpoint::MANIFEST_URL, fetch::MAX_MANIFEST_BYTES)
        .await
        .map_err(|_| UpdateError::Fetch)?;
    let signature_bytes =
        fetch::fetch_capped(&client, endpoint::MANIFEST_SIG_URL, fetch::MAX_SIGNATURE_BYTES)
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
    fetch::download_verified(&client, artifact, &staged_path)
        .await
        .map_err(|_| UpdateError::Fetch)?;

    #[cfg(windows)]
    {
        // The same two questions install.ps1 asks: is the Authenticode
        // status Valid, and is the signer us.
        if let Err(_e) = authenticode::verify(&staged_path) {
            let _ = stage::clear(&exe);
            return Err(UpdateError::Authenticode);
        }
    }

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

/// Apply a staged update to `target_exe`, if one is staged and still valid.
///
/// Returns the version installed, or `None` when nothing was staged. Every
/// check the download performed is performed again here: a staged update may
/// have sat on disk across a reboot, and what is applied must be what was
/// verified.
pub fn apply_staged(target_exe: &Path) -> Result<Option<String>, UpdateError> {
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
    if !record.sha256.eq_ignore_ascii_case(&fetch::sha256_hex(&bytes)) {
        let _ = stage::clear(target_exe);
        return Err(UpdateError::Fetch);
    }

    let current = env!("CARGO_PKG_VERSION");
    match version::is_newer(current, &record.version) {
        Ok(true) => {}
        Ok(false) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Version);
        }
        Err(_) => {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Version);
        }
    }

    #[cfg(windows)]
    {
        if let Err(_e) = authenticode::verify(&staged_path) {
            let _ = stage::clear(target_exe);
            return Err(UpdateError::Authenticode);
        }
    }

    swap::swap_in_place(&staged_path, target_exe).map_err(|_| UpdateError::Swap)?;
    stage::clear(target_exe).map_err(|_| UpdateError::Stage)?;
    Ok(Some(record.version))
}
```

Add to `crates/trace-commons-contributor/src/update/mod.rs`, keeping the list alphabetical:

```rust
pub mod run;
```

In `crates/trace-commons-contributor/src/daemon/mod.rs`, apply any staged update at daemon start. Insert this immediately after the `try_lock` block in `start_embedded` and before `let shared = Arc::new(ipc::DaemonShared::load(store)?);`:

```rust
    // A verified update parked by an earlier check is applied here, at the
    // daemon's natural start, rather than swapped underneath a running
    // process. The binary this process is executing is unaffected -- on unix
    // it holds the old inode, and on Windows the old image is renamed aside
    // -- so the new code runs from the following start. `trace-commons-
    // contributor update` is the path for applying one immediately.
    //
    // Failures here are never fatal to starting the daemon: not updating is
    // always better than not running. The label is fixed, and no path is
    // logged.
    if let Ok(exe) = std::env::current_exe() {
        match crate::update::run::apply_staged(&exe) {
            Ok(Some(_version)) => {
                tracing::info!("applied a staged update; it takes effect at the next start");
            }
            Ok(None) => {}
            Err(e) => tracing::warn!(reason = %e, "staged update was refused"),
        }
    }
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor update::run`
Expected: PASS, 6 tests.

Then run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/update/ crates/trace-commons-contributor/src/daemon/mod.rs
git commit -m "Wire the update flow from manifest to swap"
```

---

### Task 11: The `update` subcommand

**Files:**
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (a new `Command::Update` variant and its dispatch)
- Modify: `crates/trace-commons-contributor/src/commands.rs` (a new `pub async fn update`)
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod update;` if the foundation plan has not)

**Interfaces:**
- Consumes: `update::run::{check_and_install, UpdateMode, UpdateOutcome}`, `update::source::WINGET_UPGRADE_COMMAND`.
- Produces: `pub async fn update(store: &ConfigStore, stage_only: bool, json: bool) -> anyhow::Result<()>` and `pub(crate) fn render_update_outcome(outcome: &UpdateOutcome, json: bool)`.

- [ ] **Step 1: Write the failing tests**

Add to the existing `#[cfg(test)] mod daemon_command_tests` in `crates/trace-commons-contributor/src/commands.rs`:

```rust
    #[test]
    fn a_winget_install_is_told_the_winget_command_and_nothing_else() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::DeferredToWinget);
        assert!(
            text.contains("winget upgrade TraceCommons.Contributor"),
            "{text}"
        );
        // Never suggest a manual replacement to somebody whose package
        // manager owns the file: doing it would leave winget offering a
        // phantom upgrade forever.
        assert!(!text.contains("install.ps1"), "{text}");
    }

    #[test]
    fn an_up_to_date_install_says_so_without_a_version_bump_claim() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::UpToDate {
            version: "0.1.0".to_string(),
        });
        assert!(text.contains("0.1.0"), "{text}");
        assert!(!text.contains("installed"), "{text}");
    }

    #[test]
    fn a_quiesce_timeout_is_reported_as_staged_not_as_installed() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::QuiesceTimedOutStaged {
            version: "0.2.0".to_string(),
        });
        assert!(text.contains("staged"), "{text}");
        assert!(!text.contains("installed"), "{text}");
    }

    #[test]
    fn an_applied_update_names_the_version_installed() {
        use crate::update::run::UpdateOutcome;
        let text = crate::commands::update_outcome_line(&UpdateOutcome::Applied {
            version: "0.2.0".to_string(),
        });
        assert!(text.contains("installed"), "{text}");
        assert!(text.contains("0.2.0"), "{text}");
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

Run: `cargo test -p trace-commons-contributor commands::daemon_command_tests`
Expected: FAIL — compilation error, `update_outcome_line` is not defined.

- [ ] **Step 3: Write the minimal implementation**

Append to `crates/trace-commons-contributor/src/commands.rs`:

```rust
/// The one human-readable line for each update outcome. A function rather
/// than inline `println!`s so it is testable without capturing stdout.
pub(crate) fn update_outcome_line(outcome: &crate::update::run::UpdateOutcome) -> String {
    use crate::update::run::UpdateOutcome;
    match outcome {
        UpdateOutcome::DeferredToWinget => format!(
            "winget installed this copy, so winget updates it:\n  {}",
            crate::update::source::WINGET_UPGRADE_COMMAND
        ),
        UpdateOutcome::UpToDate { version } => format!("already up to date ({version})"),
        UpdateOutcome::NoArtifactForPlatform => {
            "no update is published for this platform yet".to_string()
        }
        UpdateOutcome::Staged { version } => format!(
            "{version} verified and staged; it is applied at the daemon's next start, \
             or now with `trace-commons-contributor update`"
        ),
        UpdateOutcome::Applied { version } => format!("installed {version}"),
        UpdateOutcome::QuiesceTimedOutStaged { version } => format!(
            "{version} verified and staged, but an upload is still in flight; \
             nothing was replaced. Try again shortly."
        ),
    }
}

/// The machine-readable name for each outcome, for `--json` callers.
fn update_outcome_kind(outcome: &crate::update::run::UpdateOutcome) -> &'static str {
    use crate::update::run::UpdateOutcome;
    match outcome {
        UpdateOutcome::DeferredToWinget => "deferred_to_winget",
        UpdateOutcome::UpToDate { .. } => "up_to_date",
        UpdateOutcome::NoArtifactForPlatform => "no_artifact_for_platform",
        UpdateOutcome::Staged { .. } => "staged",
        UpdateOutcome::Applied { .. } => "applied",
        UpdateOutcome::QuiesceTimedOutStaged { .. } => "quiesce_timed_out_staged",
    }
}

/// Check for, verify, and install an update.
///
/// Verification is not optional and there is no flag that skips it: this tool
/// reads coding transcripts, so an updater that can be talked into installing
/// something unverified is worse than no updater.
pub async fn update(store: &ConfigStore, stage_only: bool, json: bool) -> Result<()> {
    use crate::update::run::{UpdateMode, check_and_install};

    let mode = if stage_only {
        UpdateMode::Stage
    } else {
        UpdateMode::Apply
    };
    let outcome = check_and_install(store, mode)
        .await
        // The error labels are fixed and carry no path, URL, or signature.
        .map_err(|e| anyhow::anyhow!("update refused: {e}"))?;

    if json {
        let version = match &outcome {
            crate::update::run::UpdateOutcome::UpToDate { version }
            | crate::update::run::UpdateOutcome::Staged { version }
            | crate::update::run::UpdateOutcome::Applied { version }
            | crate::update::run::UpdateOutcome::QuiesceTimedOutStaged { version } => {
                Some(version.clone())
            }
            _ => None,
        };
        let out = serde_json::json!({
            "schema_version": "trace_commons.cli_update.v1",
            "outcome": update_outcome_kind(&outcome),
            "version": version,
        });
        println!("{}", serde_json::to_string_pretty(&out)?);
    } else {
        println!("{}", update_outcome_line(&outcome));
    }
    Ok(())
}
```

In `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`, add a variant to `enum Command`, immediately after `Whoami`:

```rust
    /// Check for a newer release, verify it, and install it
    ///
    /// Refuses anything it cannot verify: the manifest signature, the
    /// sha256, and on Windows the Authenticode signer must all check out,
    /// and the offered version must be strictly newer. There is no flag
    /// that skips any of that. When winget installed this copy, this prints
    /// the winget command and installs nothing.
    Update {
        /// Verify and stage the update without replacing anything; the
        /// daemon applies it at its next start
        #[arg(long)]
        stage_only: bool,
    },
```

and the dispatch arm in `run`, immediately after the `Command::Whoami` arm:

```rust
        Command::Update { stage_only } => commands::update(&store, stage_only, cli.json).await,
```

In `crates/trace-commons-contributor/src/lib.rs`, add to the alphabetical `pub mod` run (after `pub mod submit;`) if it is not already there from the foundation plan:

```rust
pub mod update;
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor commands::daemon_command_tests`
Expected: PASS, including the 4 new cases.

Run: `cargo run -p trace-commons-contributor --bin trace-commons-contributor -- update --help`
Expected: the help text for `update` including `--stage-only`.

Run: `cargo run -p trace-commons-contributor --bin trace-commons-contributor -- update`
Expected: exit 1 with `Error: update refused: update_endpoint_no_pinned_key` — a developer build has no pinned key, and refusing is the correct behaviour.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs crates/trace-commons-contributor/src/commands.rs crates/trace-commons-contributor/src/lib.rs
git commit -m "Add an update subcommand that verifies before it replaces"
```

---

## Verification

Every one of these must pass before this plan is complete:

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor --no-run
cargo test -p trace-commons-contributor update::
cargo test -p trace-commons-contributor daemon::ipc::
cargo test -p trace-commons-contributor commands::
cargo test -p trace-commons-contributor --test update_conformance
cargo test -p trace-commons-contributor --test update_manifest_roundtrip
cargo test -p trace-commons-contributor --test daemon_ipc_contract
cargo clippy -p trace-commons-contributor --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
python3 -c "import yaml; yaml.safe_load(open('.github/workflows/release-apps.yml'))"
./tests/fixtures/update-conformance/regenerate.sh && git diff --exit-code tests/fixtures/update-conformance
```

The last line is the fixture determinism check: regenerating must produce no diff.

## Operator prerequisites

Not code, and required before the first release that ships this:

1. Set `TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX` in the release build environment to the raw 32-byte manifest public key, hex, from the foundation plan's operator step 5. Without it, released binaries refuse every update.
2. Confirm the `contributor-v*` release job publishes all four CLI assets named in Task 2, at the exact filenames listed there.
3. Add a Windows CI job that runs `cargo test -p trace-commons-contributor --test update_conformance` on `windows-latest`, in the spirit of the existing named-pipe ACL job. The Authenticode path cannot be exercised from a cross-compile, so this is the only thing standing behind the claim that it works.
