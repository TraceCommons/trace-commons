# Signed, distributable builds for the contributor apps

Date: 2026-08-16

## Problem

Every artifact this project ships to a contributor is unsigned. The only
release workflow, `.github/workflows/release-contributor.yml`, publishes bare
CLI binaries whose own release notes say they are "not code-signed or
notarized" and that clearing Gatekeeper "requires an explicit user action."

For a background app that reads a contributor's coding transcripts, that is
the wrong thing to ask. The Gatekeeper warning is precisely the signal that
should stop someone installing a tampered build, and an install path whose
first step is teaching people to click past it trains them past the real
thing. `macos/scripts/make-release-dmg.sh` already argues this at length; the
gap is that the script has never run.

This design makes signed builds available on macOS, Windows, and Linux, and
installable on macOS via Homebrew.

## What exists today

| Platform | App | Signing state |
| --- | --- | --- |
| macOS | `macos/` SwiftPM app (`TraceCommons.app`, bundle id `ai.tracecommons.shell`) | `make-release-dmg.sh` is a complete sign/notarize/staple/DMG path with throwaway-keychain handling. Never executed. Not in CI. |
| Linux | `crates/trace-commons-contributor-gtk` (GTK4/libadwaita) plus a flatpak manifest | No signing. Built only by the local Docker script `scripts/linux-build.sh`. Manifest never built. |
| Windows | No GUI shell. CLI plus daemon over a restricted named pipe. | No cert. Windows is not in the release matrix at all. |

`release-contributor.yml` covers `aarch64-apple-darwin`,
`x86_64-apple-darwin`, and `x86_64-unknown-linux-gnu`, unsigned, with a
published SHA-256 that establishes integrity against the download but not
provenance.

## Credentials: already provisioned

Verified live rather than assumed:

- **Apple.** `asc` profile `iqlusion` authenticates. Certificate
  `3K939H4WUQ`, type `DEVELOPER_ID_APPLICATION_G2`, Iqlusion Inc, valid to
  2031-05-06.
- **Azure.** Trusted Signing account `argossigning` (resource group
  `argos-signing`, eastus). Certificate profile `argos`: `PublicTrust`,
  status `Active`, identity validation `e9d53dd3-9619-472c-926f-3571ede3f53a`
  completed for Iqlusion Inc. The `trustedsigning` Azure CLI extension is
  installed.

Nothing needs procuring and there is no identity-validation wait. One gap:
`security find-identity -v -p codesigning` lists only Apple Development and
iPhone Distribution identities, so **the private key for `3K939H4WUQ` is not
on the development machine.** Either it lives on another machine or we mint a
pair we control.

Decision: mint a fresh Developer ID pair via
`asc certificates create --certificate-type DEVELOPER_ID_APPLICATION_G2
--generate-csr`, so the key is one we hold and can export as a `.p12`. Revoke
the orphaned certificate only after the new one has produced a verified
stapled DMG — revoking first would leave no working identity if the new one
has a problem.

## Blockers in the existing macOS path

Each of these would break a release build today.

1. **`Package.swift` links against the wrong configuration.** Both
   executable targets hardcode `-L ../target/debug` in `linkerSettings`,
   while `make-release-dmg.sh` builds `release`. On a clean CI checkout that
   builds only the release dylib, the Swift link step fails — there is no
   `target/debug`. The library search path must come from the build script so
   `release` links `target/release`.

2. **The bundle version is hardcoded.** `make-app-bundle.sh` writes
   `CFBundleShortVersionString 0.1.0` and `CFBundleVersion 1` into a
   heredoc Info.plist. Tag `app-v0.2.0` would ship a DMG claiming `0.1.0`.
   Homebrew compares a cask's declared version against what is installed, so
   this also breaks `brew upgrade`. Version must be injected from the tag.

3. **A wasted ad-hoc signature in the release path.**
   `make-app-bundle.sh` ends with `codesign --force --sign -` on the dylib
   and the bundle; the release script then re-signs both with `--force`.
   Harmless, but it makes the release path read as if it might ship an
   ad-hoc signature. Make the ad-hoc step conditional on the development
   path.

## Credential handling

| Platform | Mechanism | Secrets in CI |
| --- | --- | --- |
| macOS | Freshly minted Developer ID App G2, exported `.p12` | `MACOS_CERTIFICATE_P12_BASE64`, `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY`, plus an App Store Connect API key (`.p8`, key id, issuer id) for notarytool |
| Windows | Azure Trusted Signing profile `argos` via GitHub OIDC federated credential, role `Trusted Signing Certificate Profile Signer` | None. Only the non-secret `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, `AZURE_SUBSCRIPTION_ID` |
| Linux | New GPG key for the OSTree repo, held in GCP Secret Manager, read through GitHub-to-GCP workload identity federation | None. Federated; no key material in GitHub |

macOS is the only platform where a long-lived signing key sits in GitHub
secrets. That is inherent to `codesign`, which has no keyless equivalent. The
throwaway-keychain handling already in `make-release-dmg.sh` — create, import,
restore the search list in a trap, delete — is the mitigation, and it stays.

**Notarization switches to the App Store Connect API key** rather than
`MACOS_NOTARY_APPLE_ID` plus an app-specific password. `notarytool` accepts
`--key/--key-id/--issuer`, `asc` already holds such a key, and this drops two
secrets. It also closes the residual exposure the script's own header
documents: `notarytool store-credentials` takes the password as an argument,
visible in `ps` for the duration of that call.

## Release pipeline

A new `.github/workflows/release-apps.yml`, driven by tag `app-v*`, with
`workflow_dispatch` taking a platform input so one leg can be re-run without
cutting a tag.

Three independent jobs rather than a matrix. The packaging steps share
essentially nothing — SwiftPM plus notarytool, cargo plus Trusted Signing,
flatpak-builder plus OSTree — so matrix legs would be a stack of `if:` guards.
Independent jobs also mean one platform failing does not block the others.

- **`macos`** — build the FFI dylib in release, run `make-app-bundle.sh
  release` with the version injected from the tag, then
  `make-release-dmg.sh`: sign nested dylib first and bundle second, notarize
  via the ASC API key, staple, and assert with `spctl --assess`. Upload the
  DMG.

- **`windows`** — add `x86_64-pc-windows-msvc`, build the CLI and daemon,
  sign with `azure/trusted-signing-action` **including RFC3161
  timestamping**, verify with `signtool verify /pa`, upload a zip.

  Timestamping is not optional here. Trusted Signing issues short-lived
  certificates (roughly three days). The signature outlives the certificate
  only by virtue of its countersignature, so an untimestamped binary starts
  failing validation within days of release — a failure that would not appear
  in any same-day test.

- **`linux-flatpak`** — a container with `flatpak-builder`, build from
  `crates/trace-commons-contributor-gtk/flatpak/ai.tracecommons.Contributor.yml`,
  then `flatpak build-sign` and `flatpak build-update-repo --gpg-sign
  --generate-static-deltas`, and sync the OSTree repo to a public GCS bucket
  in `tracecommons-pilot-2026` alongside the `.flatpakref` and the exported
  public key.

A final `publish` job gathers whatever succeeded into a single GitHub
Release. The flatpak repo publishes on its own path, since an OSTree repo is
not a release asset.

## Homebrew

A new repository, `TraceCommons/homebrew-tap`. Contributors run
`brew tap TraceCommons/tap` and then `brew install --cask trace-commons`.

Upstream `homebrew-cask` and `homebrew-core` both apply notability
expectations a pilot-stage project will not clear, and each version bump
would be a pull request against a third-party repository sitting in the
release path. A tap we own is the option that works now.

The cask is only viable because the DMG is notarized: a cask installs into
`/Applications` and Gatekeeper evaluates it exactly as it would a manual
download, so shipping an unsigned DMG through Homebrew only relocates the
failure.

- **Cask `trace-commons`** — the notarized DMG. Carries
  `quit: "ai.tracecommons.shell"`, because the app registers itself as a
  login item through `SMAppService`; deleting a running bundle strands an
  entry in System Settings > General > Login Items.

  Its `zap` stanza trashes caches and preferences but **deliberately excludes
  `~/Library/Application Support/trace-commons/contributor.json`.** That file
  holds the device identity key, and `/v1/onboard` is not idempotent, so
  deleting it burns an invite code that cannot be reissued — a contributor
  who ran `brew uninstall --zap` would be locked out of re-enrolling. The
  exclusion carries a comment stating this, because it reads like an
  oversight and would otherwise be "tidied up" later.

- **Formula `trace-commons-contributor`** — the signed CLI binary.

The two artifacts keep their separate tag streams, and each bumps only its own
file: `release-apps.yml` (tag `app-v*`) bumps the cask, and
`release-contributor.yml` (tag `contributor-v*`) bumps the formula. The app and
the CLI version independently and should not be forced into lockstep by the
packaging.

In both cases the release job opens a version-bump pull request against the
tap rather than committing to it directly, so a bad release does not
auto-publish.

### Signing the CLI binary

The CLI needs the same treatment on macOS, with one wrinkle: `notarytool`
accepts a disk image, a package, or a zip — never a bare Mach-O. So the macOS
CLI binaries are `codesign --timestamp --options runtime` signed, zipped, and
the **zip** submitted for notarization. There is nothing to staple to
(stapling requires a bundle, package, or image), which is fine: Gatekeeper
resolves the notarization ticket online for a binary invoked from a shell, and
a CLI is not subject to the quarantine-launch path a `.app` is. The published
SHA-256 stays — it is cheap and answers a different question than the
signature.

## Correcting the existing claims

Once the CLI is signed, `release-contributor.yml`'s release notes are wrong
in a way that matters: they tell contributors the binaries are unsigned and
that "signing needs an Apple Developer identity and is not set up yet."
Leaving that text in place would keep teaching people to click past
Gatekeeper after the reason to do so is gone. It is rewritten as part of this
work, not as a follow-up.

## Verification

The rule this project already applies to the Windows pipe ACL applies here:
a script that has never run is not evidence. `make-release-dmg.sh` has never
signed anything, so notarization is entirely unverified today. Each platform
needs a check on a machine that did not produce the artifact.

- **macOS** — open the stapled DMG on a Mac that did not build it, with the
  network off, and confirm it launches with no Gatekeeper prompt. Offline is
  the point: it is what distinguishes a stapled ticket from one that
  happens to resolve against Apple over the network.
- **Windows** — `signtool verify /pa` on a fresh machine, and confirm the
  signature still validates after the signing certificate's roughly
  three-day validity window has elapsed. That second check is what proves
  timestamping actually took effect.
- **Linux** — `flatpak install --from` the published `.flatpakref` in a clean
  container with GPG verification enabled.
- **Homebrew** — `brew install --cask` from the tap on a clean machine, then
  `brew uninstall --zap` and confirm `contributor.json` survives.

## Risk

`ai.tracecommons.Contributor.yml` has never been built. Linux carries real
discovery risk: the manifest may need work, and the sandbox has to reach both
the daemon socket and the transcript directories the app reads. The three
jobs are independent precisely so that macOS and Windows can ship if Linux
turns into a hole.

## Out of scope

- A Windows GUI shell. None exists; this work signs the CLI and daemon and
  puts the signing path in place for a shell to use later.
- An MSI or other Windows installer. A signed zip is the deliverable here.
- Submitting to upstream `homebrew-cask`, Flathub, or any distro repository.
- Linux package formats other than flatpak (`.deb`, `.rpm`, AppImage).
