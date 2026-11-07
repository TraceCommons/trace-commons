# Private Xcode Cloud macOS pilot

The pilot builds the existing production-channel native app as a private,
universal archive and uses Apple's managed signing and notarization. It does
not publish a DMG, appcast, CLI manifest, release, or package, and does not
install the app. The existing GitHub release publisher and its secret and
provenance paths remain authoritative.

## Current setup

The Near.ai macOS app record exists in App Store Connect: app ID `6832109427`,
bundle ID `ai.tracecommons.shell`, Iqlusion team `KXSWJN7WY8`. An app record is
not a completed Cloud setup. At preparation time, no GitHub source provider
was connected and the first Cloud workflow still required onboarding from
native Xcode. No real Cloud archive, managed signing, or notarization result
has been demonstrated by these local checks. Updated Apple program terms,
if required, need the account holder's review and acceptance.

## Finish onboarding

1. Open `macos/NearAI.xcodeproj` in native Xcode and select the shared
   `NearAI-Cloud` scheme. Start Xcode Cloud onboarding with the Iqlusion team
   and the existing Near.ai app record.
2. Connect the intended GitHub repository. Select this checked-in project
   and scheme, a reviewed pilot branch, macOS, Release, and an Archive action.
   Use an Xcode 26 or newer image that supports Icon Composer `.icon` assets.
   Do not install XcodeGen on Cloud or regenerate the project there.
3. Configure the Archive action for **Developer ID** distribution and the
   Cloud notarization post-action. These are workflow settings, not shell
   signing commands. Keep external distribution and App Store/TestFlight
   upload actions disabled. Use manual starts while qualifying the pilot.
4. Configure these workflow environment variables:

   | Variable | Value |
   | --- | --- |
   | `TC_CHANNEL` | `production` |
   | `TC_SPARKLE_PUBLIC_ED_KEY` | Existing production 32-byte Ed25519 public key, Base64 |
   | `TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX` | Existing production 32-byte CLI update public key, hex |

   Both keys are public verification material. Obtain them from the reviewed
   production configuration; no private signing keys enter these scripts.
   Cloud supplies `CI_BUILD_NUMBER` and `CI_COMMIT`.
5. Start the pilot and retain the exact source commit, workflow settings,
   archive and distribution/notarization results. Download the artifact for
   independent signature, stapling, Gatekeeper and launch checks before
   considering any change to the publisher.

`ci_post_clone.sh` installs Rust `1.96.0` with both Apple targets. If rustup is
absent, it downloads official rustup-init `1.29.3` and verifies its pinned
host-specific SHA-256 before execution. No sudo or Python/npm dependencies
are needed. Rust commands use `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0`.

`ci_pre_xcodebuild.sh` builds the FFI and `near-ai` with `--locked --release`
for arm64 and x86_64, merges and checks both architectures, and sets the
FFI install ID to `@rpath/libtrace_commons_contributor_ffi.dylib`. It renders
the existing production Info.plist and localized display name, and embeds a
private pilot metadata JSON. The short version comes from the contributor
Cargo.toml; the private build version comes from `CI_BUILD_NUMBER`.

The native project embeds the FFI in Frameworks and the daemon in MacOS with
CodeSignOnCopy. Preparation first ad-hoc signs both Rust binaries with
`--options runtime`; both staged binaries
and every archived Rust binary slice must retain the hardened-runtime flag.
It uses the existing entitlements, hardened runtime, automatic
team signing, the existing Icon Composer asset, and exactly Sparkle `2.9.6`.
It reuses the SwiftPM modules and TCShellCore resource bundle. No script
recursively re-signs Sparkle or replaces its helper entitlements.

`ci_post_xcodebuild.sh` runs before Cloud distribution/notarization. For an
Archive action it checks the source/build metadata, production plist and
runtime FFI/CLI channels, CLI source/version, universal architectures, dylib
linkage, hardened-runtime flags, icons, localization, sample bundle and Sparkle layout. Passing this
check does not assert a Developer ID distribution signature or notarization;
those must be checked after the corresponding Cloud actions complete.

## Local checks

From the repository root, supply the two public-key variables above, then:

```sh
CI_BUILD_NUMBER=1 macos/scripts/prepare-xcode-cloud.sh --metadata-only
xcodebuild -list -project macos/NearAI.xcodeproj
xcodebuild -project macos/NearAI.xcodeproj -scheme NearAI-Cloud \
  -configuration Release -destination 'generic/platform=macOS' -showBuildSettings
/usr/bin/python3 macos/ci_scripts/test-pilot.py
```

Metadata-only mode builds no Rust binaries. For a local diagnostic build,
install the exact Rust toolchain and use `--debug-native` instead; it builds
fresh native debug FFI and CLI inputs. Build the native Xcode app locally
with `ARCHS=arm64 ONLY_ACTIVE_ARCH=YES CODE_SIGNING_ALLOWED=NO` on Apple silicon.
Cloud hooks always select release-universal mode. A debug-native or
metadata-only input set is refused by archive verification.

To update project structure, first generate metadata, then run the locally
installed XcodeGen `2.46.0`:

```sh
xcodegen generate --spec macos/xcode-cloud-project.yml --project macos
mkdir -p macos/NearAI.xcodeproj/project.xcworkspace/xcshareddata/swiftpm
cp macos/Package.resolved \
  macos/NearAI.xcodeproj/project.xcworkspace/xcshareddata/swiftpm/Package.resolved
```

Review and check in the generated project, shared scheme and dependency lock.
The `.xcode-cloud` build inputs remain ignored. The integration tests use
synthetic public keys and real temporary universal Mach-O fixtures; they do
not exercise Apple accounts, Cloud-managed credentials or notarization.
