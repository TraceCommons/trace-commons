# Private Xcode Cloud macOS pilot

The pilot builds the existing production-channel native app as a private,
universal archive and uses Apple's managed signing and notarization. It does
not publish a DMG, appcast, CLI manifest, release, or package, and does not
install the app. The existing GitHub release publisher and its secret and
provenance paths remain authoritative.

## Current setup

The Near.ai macOS app record exists in App Store Connect: app ID `6832109427`,
bundle ID `ai.tracecommons.shell`, Iqlusion team `KXSWJN7WY8`. The repository
is connected to Cloud product `3eff5e74-a518-4a99-927d-481130210e75`.

Workflow `59025125-A8A9-4F65-9D19-8E504576C729` (NearAI Cloud pilot) uses
manual branch starts, a clean macOS archive, restricted editing, and a
Notarize macOS post-action. Both production public verification keys are
configured. The preparation script defaults `TC_CHANNEL` to `production`.
No App Store Connect or TestFlight distribution action is enabled.

## Qualification results

Build 1 (`0b18e328-d3e5-4286-9286-bb10703b4cc1`) exposed Rust 1.96's
release debug-strip alignment defect under Xcode 27. The scoped strip override
and alignment regression below resolve that failure.

Build 2 (`441d132b-aafe-4dba-ae8d-2cbea6d4611b`) at source commit
`20f7e18b6198d59f9c8908c0bda3bb485f98944c` compiled and archived the app and
passed the post-build archive validator. Independent inspection of the
downloaded archive confirmed source/version/channel, the production Sparkle
key, both architectures for app/CLI/FFI, aligned LINKEDIT string pools, and
hardened runtime for both Rust binaries.

The overall run failed at Developer ID export, before notarization. Apple's
managed certificate query returned no certificates, but creation returned
HTTP 409 / resultCode 7460: an existing Developer ID Application Managed
certificate or pending request prevents creation. The browser certificate
inventory showed the existing ordinary Developer ID certificate but no
Cloud-managed Developer ID certificate. This requires resolving the team's
managed signing state with Apple before claiming signed/notarized success.
Other developers report the same contradiction on the
[Apple Xcode Cloud forum](https://developer.apple.com/forums/topics/developer-tools-and-services/developer-tools-and-services-xcode-cloud).

Retain build 2's Developer ID `IDEDistributionProvisioning.log` and workflow
ID above for Apple Support. No certificate was revoked and no app was
published or installed. Production release publishing remains on GitHub.

## Trigger from automation

GitHub Actions can use an App Store Connect API key stored in GitHub secrets
to trigger builds, poll results, and retrieve artifacts. Native Xcode is only
needed for initial onboarding. With the `asc` CLI authenticated:

```sh
asc xcode-cloud run --workflow-id 59025125-A8A9-4F65-9D19-8E504576C729 \
  --branch xcode-cloud-release-pilot --clean
asc xcode-cloud status --run-id BUILD_RUN_ID
asc xcode-cloud doctor --run-id BUILD_RUN_ID
```

Verify the returned source commit. Publishing must wait for successful
notarization and independent artifact verification. This pilot does not yet
add a GitHub Actions trigger or replace the release workflow.

## Reproduce workflow setup

1. Open `macos/NearAI.xcodeproj` in native Xcode and select the shared
   `NearAI-Cloud` scheme. Start Xcode Cloud onboarding with the Iqlusion team
   and the existing Near.ai app record.
2. Connect the intended GitHub repository. Select this checked-in project
   and scheme, a reviewed pilot branch, macOS, Release, and an Archive action.
   Use an Xcode 26 or newer image that supports Icon Composer `.icon` assets.
   Do not install XcodeGen on Cloud or regenerate the project there.
3. Add the **Notarize (macOS Only)** post-action to the Archive action.
   Require restricted editing and a clean build. These are workflow settings, not shell
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
are needed. Rust commands use `RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0` and
`CARGO_PROFILE_RELEASE_STRIP=none`. Rust 1.96's default release debug-strip
pass can misalign the Mach-O string pool, which Xcode 27's linker rejects
([Rust #157750](https://github.com/rust-lang/rust/issues/157750)). The pilot
preserves the native linker's layout and checks every staged and archived
binary slice's string-pool alignment after staging and signing.

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
# Requires the pinned Rust toolchain and both macOS targets from ci_post_clone.sh.
/usr/bin/python3 macos/ci_scripts/test-linkedit.py
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
