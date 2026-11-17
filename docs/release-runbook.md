# Release Runbook

This document records the operational steps and invariants for cutting a
signed, distributable release of the Trace Commons contributor apps (the
macOS app bundle and the CLI). It is the companion to
`crates/trace-commons-contributor/tests/release_pipeline.rs`, which pins the
properties this runbook describes wherever they are visible in text (env
contracts, mandatory flags, deliberate exclusions). What the tests cannot
prove — that signing, notarization, or a flatpak build actually succeed
against real credentials — is covered by the manual gates below.

## What an `app-v*` tag releases: macOS only

Native macOS is the lead client. The Windows and Tauri shells are frozen,
and the Linux GTK app, back in development since 2026-11-11, is not
released on tags until it catches up. Since `app-v0.23.0`, an `app-v*`
tag push in
`release-apps.yml` builds and publishes the macOS DMG alone: the GitHub
Release, `updates/latest.json` and the Sparkle appcast. The Homebrew cask
is bumped by hand (see "Publishing, and the manual tap bump").

- The `windows` (signed zips), `windows-app` (MSIX) and `linux-flatpak`
  jobs run only on a `workflow_dispatch` with `platform` set to `all`,
  `windows` or `linux`. A dispatch builds and signs but never publishes, so
  there is no path from these jobs to a release, a bucket or a feed.
- `app-v0.23.0` is the last release with Windows and Linux app assets. The
  MSIX `.appinstaller` feed and the flatpak OSTree repo in the
  `tracecommons-flatpak` bucket are written only by those jobs, so they
  stay on that build: installed MSIX and flatpak apps see no update and
  are not pointed at anything missing.
- `updates/latest.json` carries no `windows-x86_64` desktop entry from a
  macOS-only release. Nothing installed reads that slug (the CLI
  self-updater reads only the `-cli` slugs), and an absent slug means
  "no update for this platform".
- The tag pre-flight (`release-config`) no longer requires the
  `AZURE_SIGNING_*` values. The `version` job still checks the tag against
  `crates/trace-commons-contributor-gtk/Cargo.toml`, so keep bumping that
  crate in the release-prep commit.
- The Tauri app is not built for release by any workflow.
- The CLI is unaffected: `release-contributor.yml` still builds and
  publishes it for macOS, Linux and Windows on `contributor-v*` tags. The
  Homebrew formula and the winget manifest are updated by hand.
  `scripts/install.ps1`
  resolves `contributor-v*` releases, never `app-v*` ones.

To put a platform back on tags, restore `github.event_name == 'push' ||`
in its job's `if:` (and its credentials in `release-config`). `publish`
and `publish-updates` already branch on each platform's result.

## Homebrew

The macOS app and CLI are distributed via a Homebrew tap:
`TraceCommons/homebrew-tap` (public, on GitHub), consumed as
`brew tap TraceCommons/tap`.

- `Casks/trace-commons.rb` installs the notarized `Near.ai.app` from the
  DMG published as `Gateway-<version>.dmg` (volume `Gateway`) on releases
  of `TraceCommons/trace-commons` tagged `app-v<version>`. Releases up to
  1.0.2 published `Near.ai-<version>.dmg`, and earlier ones
  `TraceCommons-<version>.dmg` holding `TraceCommons.app`; the cask's `url`
  and `app` stanzas name the current files.
- `Formula/trace-commons-contributor.rb` installs the signed CLI from the
  zips published as `trace-commons-contributor-<target>.zip` on releases
  tagged `contributor-v<version>`, for both `aarch64-apple-darwin` and
  `x86_64-apple-darwin`.
- Minimum macOS is Sonoma (`depends_on macos: ">= :sonoma"`), matching the
  app's `LSMinimumSystemVersion` of `14.0`.

### The app DMG is a universal binary

`macos/scripts/make-app-bundle.sh` builds the FFI dylib for both
`aarch64-apple-darwin` and `x86_64-apple-darwin`, `lipo`s them into one
dylib, and passes `swift build --arch arm64 --arch x86_64` so the app
executable is universal too; it verifies both with `lipo -archs` and fails
loudly if either is thin. `Gateway-<version>.dmg` therefore runs on both
Apple silicon and Intel Macs, and the cask carries no `depends_on arch:`.

The CLI (`Formula/trace-commons-contributor.rb`) ships separate
`aarch64-apple-darwin` and `x86_64-apple-darwin` builds instead of a
universal one; that is unrelated to the app DMG and unaffected by this.
- The cask's `uninstall quit:` stanza targets bundle identifier
  `ai.tracecommons.shell`, because the app registers itself as a login item
  via `SMAppService`; removing a running bundle without quitting it first
  strands an entry in System Settings > General > Login Items.

### The deliberate zap exclusion

The cask's `zap` stanza does **not** trash
`~/Library/Application Support/trace-commons/contributor.json`.

That file is the device identity key, and the server's `/v1/onboard` is
**not idempotent** — an invite code can be redeemed exactly once. If
`brew uninstall --zap` deleted `contributor.json`, a contributor who
reinstalls would have no way to re-enroll: their original invite code is
already spent and cannot be reissued. Omitting this path from `zap` is
intentional, and the cask carries a comment saying so, precisely because an
incomplete-looking `zap` stanza invites someone to "finish" it later. Don't.

### Checksum placeholders

No release has been published yet as of this writing, so the real
`sha256` values for the DMG and CLI zips are unknown. The cask and formula
currently carry obviously-invalid placeholders (e.g.
`sha256 "REPLACE_ON_FIRST_RELEASE_see_docs_release-runbook.md"`) rather than
a plausible-looking but wrong hex string — a wrong checksum makes Homebrew
report what looks like tampering, whereas an obviously-missing one just
looks unfinished.

**Action required on the first release:** compute the real `sha256` for each
published artifact (`shasum -a 256 <file>`) and replace the placeholders in
`Casks/trace-commons.rb` and `Formula/trace-commons-contributor.rb`, along
with the `version` fields. Every later release bumps them by hand the same
way; see the next section.

## Publishing, and the manual tap bump

Both `release-apps.yml` and `release-contributor.yml` end in a `publish`
job (gated to `github.event_name == 'push'` -- a `workflow_dispatch` run
never publishes) that downloads the platform artifacts and creates a GitHub
Release with `gh release create`, tagged from the pushed tag. In
`release-apps.yml`, `publish-updates` then writes `updates/latest.json` and
the Sparkle appcast.

No workflow touches `TraceCommons/homebrew-tap`. The cask and formula bumps
used to run inside `publish` on a cross-repository token,
`HOMEBREW_TAP_TOKEN`; when it stopped authenticating, the 1.0.0 and 1.0.1
publish jobs failed after their assets were out, and that skipped the update
feed behind them. The tap is bumped by hand now, with your own `gh` login,
after both tag runs have succeeded:

1. Download the release assets: `Gateway-<version>.dmg` from
   `app-v<version>`, and `trace-commons-contributor-aarch64-apple-darwin.zip`
   and `trace-commons-contributor-x86_64-apple-darwin.zip` from
   `contributor-v<version>`, each with its `.sha256`.
2. Hash each file (`shasum -a 256 <file>`) and check the hash matches the
   published `.sha256`. Use the hash of the bytes you downloaded, never one
   copied from a log or an earlier branch.
3. In a clone of the tap, on a branch per file:
   - `Casks/trace-commons.rb`: set `version` and the DMG's `sha256`. The
     first bump of a release named `Gateway-<version>.dmg` must also change
     the `url` from `Near.ai-#{version}.dmg` to `Gateway-#{version}.dmg`;
     the `app` stanza stays `Near.ai.app`.
   - `Formula/trace-commons-contributor.rb`: set `version`, the `on_arm`
     `sha256` to the `aarch64-apple-darwin` zip's hash, and the `on_intel`
     `sha256` to the `x86_64-apple-darwin` zip's hash. Getting these
     backwards produces a checksum mismatch that reads to users as
     tampering.
4. Open a pull request for each and squash-merge it. Homebrew serves the
   previous version until you do.

## winget (by hand)

No workflow submits to winget: `WINGET_PKGS_TOKEN` stopped authenticating
(contributor-v1.0.2), and the step only ever pushed a branch to a fork,
because no token can open a pull request on `microsoft/winget-pkgs`. After
`release-contributor` succeeds:

1. Run `scripts/winget/generate-manifests.sh <version>`. It downloads the
   published `trace-commons-contributor-x86_64-pc-windows-msvc.zip` and
   hashes the bytes it got, then writes the three manifests under
   `manifests/t/TraceCommons/Contributor/<version>/`.
2. Copy that directory into your fork of `microsoft/winget-pkgs` at the same
   path, validate it on Windows (`winget validate --manifest <path>`), and
   open a pull request against `microsoft/winget-pkgs`.

### The publish job's guard

In `release-apps.yml`, the `publish` job carries `if: always() && ...` so
a failed or skipped platform does not withhold a verified macOS DMG (on a
tag push the Windows and Linux jobs are always skipped; see above) -- but
`always()` alone would also let the job run when *every* platform failed,
with nothing to publish. The condition additionally requires
`needs.macos.result == 'success' || needs.windows.result == 'success' ||
needs.linux-flatpak.result == 'success'` to close that gap.

### What to expect to fail on the first real tagged release

This automation has never run against real credentials or a real tag.
Expect friction in roughly this order:

- **Empty `dist/*/*` glob.** If every platform legitimately failed (not
  possible on the first real run without the `always()` guard already
  refusing to publish, but worth confirming): the guard added in this task
  should prevent `gh release create` from running with nothing to attach.
  Confirm this in practice, not just by reading the condition.
- **Task 11's Step 8 install gate** (`brew tap` / `brew install --cask` /
  `brew install` against the real, merged tap) has not been run against a
  real release yet and still needs to happen after a manual tap bump.
