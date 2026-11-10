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
Cloud-managed Developer ID certificate. A subsequent access audit found that the signed-in Admin account has Cloud
Managed Distribution access but lacks the separate Cloud Managed Developer
ID permission. The Account Holder should grant that permission before a
retry. This is a candidate repair, not a confirmed explanation of HTTP 409.
If the conflict persists with the required access, escalate the managed
certificate state to Apple. Signed/notarized success remains unproven.
Other developers report the same contradiction on the
[Apple Xcode Cloud forum](https://developer.apple.com/forums/topics/developer-tools-and-services/developer-tools-and-services-xcode-cloud).

Retain build 2's Developer ID `IDEDistributionProvisioning.log` and workflow
ID above for Apple Support. No certificate was revoked and no app was
published or installed. Production release publishing remains on GitHub.

Build 4 (`b96f5d43-35dd-4737-a786-7b4e5eb2224c`, 2026-11-10) ran after the
Account Holder granted the Cloud Managed Developer ID permission. It failed
the same way: HTTP 409 / resultCode 7460 on creating the managed certificate.
The App Store Connect API lists no managed Developer ID certificate; the
team's only Developer ID certificate is the ordinary `DEVELOPER_ID_APPLICATION_G2`
one that release-apps.yml signs with. The missing permission was therefore
not the cause. The stuck certificate state is Apple's to clear, through a
Developer Support request from the Account Holder.

The archive itself is sound. Build 4's archive, exported locally with the
ordinary Developer ID certificate and the committed profile
(`macos/TraceCommons-DeveloperID.provisionprofile`), notarized on the first
submission, stapled, and passed `spctl` as "Notarized Developer ID". It also
passed `verify-macos-entitlements.sh`'s static checks. Its embedded metadata
named the production channel, a universal release build and the production
Sparkle key.

## Sign and notarize from GitHub

Until Apple clears the managed certificate, `.github/workflows/xcode-cloud-pilot.yml`
signs Cloud archives the way releases are signed. A push to
`xcode-cloud-release-pilot` starts a clean Cloud build of that commit; a manual
run can name a branch, or an existing build run to sign without starting one.
The job then:

1. waits for the run and downloads the Archive action's archive. While the
   managed export fails, every run ends FAILED, so the archive is the gate;
2. refuses to sign unless the embedded `pilot-build.json` names the commit
   Cloud built (and, for a push, the commit pushed), the production channel,
   a universal release build and the production Sparkle key;
3. exports with `method developer-id`, the release Developer ID certificate
   (in a throwaway keychain) and the committed profile;
4. verifies Developer ID authority and hardened runtime on the app, CLI, FFI
   and Sparkle, the staging-marker check and `verify-macos-entitlements.sh`,
   which launches the app;
5. notarizes, staples, assesses with `spctl`, and uploads the stapled app as
   a 14-day workflow artifact.

It publishes nothing. It uses the `release` environment and the secrets
release-apps.yml already has (`MACOS_CERTIFICATE_*`, `MACOS_NOTARY_ASC_*`,
`SPARKLE_PUBLIC_ED_KEY`). Starting Cloud builds needs an App Store Connect
key whose role reaches Xcode Cloud (App Manager or higher):
`XCODE_CLOUD_ASC_KEY_P8_BASE64`, `XCODE_CLOUD_ASC_KEY_ID` and
`XCODE_CLOUD_ASC_ISSUER_ID`, or, when those are unset, the notary key.
`macos/ci_scripts/xcode-cloud-api.py` makes the App Store Connect calls with
the standard library and `openssl`; no third-party client runs in the job
that holds the Developer ID key.

## Trigger from automation

To start or inspect builds by hand, with the `asc` CLI authenticated:

```sh
asc xcode-cloud run --workflow-id 59025125-A8A9-4F65-9D19-8E504576C729 \
  --branch xcode-cloud-release-pilot --clean
asc xcode-cloud status --run-id BUILD_RUN_ID
asc xcode-cloud doctor --run-id BUILD_RUN_ID
```

Verify the returned source commit. Publishing must wait for successful
notarization and independent artifact verification. The pilot does not
replace the release workflow.

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
