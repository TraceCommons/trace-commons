# Automatic updates for the contributor apps

Date: 2026-08-17
Status: approved design, not yet implemented

## Problem

The three contributor shells — the macOS SwiftUI app, the Linux GTK app, and
the Windows CLI/daemon — ship signed, checksummed artifacts through
`release-apps.yml`, but nothing tells an installed copy that a newer release
exists. A contributor who installed at `app-v0.1.0` stays there until they
notice a release and act. That is the wrong default for a client that handles
trace data and whose scrubbing and consent logic changes between releases: the
population running the oldest redaction code is precisely the population that
most needs the update.

## Governing rule

**Whoever installed the binary owns replacing it.**

Each app detects its install source with a local path check — no network — and
takes one of three branches:

- *self-update*, when we placed the bytes and can replace them ourselves;
- *ask the installer to update us*, when the package manager exposes a
  sanctioned API for exactly that (the Flatpak portal);
- *defer with a nudge*, when the package manager owns the file and offers no
  such API.

The second is not an exception to the rule — it is the rule honored precisely.
Flatpak still performs the replacement; the app only asks. There is never a
case where the app and a package manager both believe they own the same file.

| Platform | Package-managed (defer) | Ours (self-update) |
|---|---|---|
| macOS | `Caskroom/trace-commons` exists under the Homebrew prefix | `.app` in `/Applications` with no Caskroom entry (DMG drag-install) |
| Linux | n/a — flatpak self-updates via the portal, see below | CLI in `~/.local/bin` from `install.sh` |
| Windows | running exe path contains `\Microsoft\WinGet\Packages\` | `%LOCALAPPDATA%\Programs\TraceCommons` from `install.ps1` |

Winget is a hard defer, not a preference. Winget records portable-package
versions in the registry; a self-swap leaves that record stale, so winget
offers a phantom upgrade indefinitely and `winget upgrade --all` fights the
app.

Linux flatpak is deliberately *not* a defer case. See the Linux section.

## Discovery: a signed manifest, not the GitHub API

The GitHub REST API is limited to 60 requests/hour keyed to the originating IP
for unauthenticated callers. Contributors behind a shared corporate NAT would
exhaust that between them and silently stop receiving updates, and the only fix
— authenticating — means shipping a token inside a client binary. So the
GitHub API is not the discovery channel.

Instead, the release pipeline publishes two files to the existing public GCS
bucket under an `updates/` prefix:

- `updates/appcast.xml` — Sparkle's native format, EdDSA-signed. macOS only.
- `updates/latest.json` — version, per-platform asset URL, and sha256, with an
  Ed25519 detached signature. Linux and Windows.

Both are generated in a single release step from the same metadata, so they
cannot disagree about what the current version is. The artifacts themselves
continue to live on GitHub Releases; the manifest only points at them.

Signing keys live in GCP Secret Manager alongside the existing
`flatpak-signing-key`. Clients ship the corresponding public keys.

**The manifest is written only for platforms whose build job actually
succeeded.** The three build jobs are independent so one platform's failure
does not withhold another's artifact; a manifest that unconditionally
advertised all three would point clients at a 404 — or, worse for Linux, at a
stale repo that installs an older build under a newer version number. This
mirrors the per-platform conditionals the existing `publish` job already
applies to release notes.

## Consent and timing

Check on launch, then daily. Download and verify in the background. Then
prompt. Nothing about the binary changes without a human saying yes, and
because the download already happened, the confirmation is near-instant.

The one exception is the headless daemon, which has no surface to prompt in:
there, a verified update is *staged* and applied on the daemon's next natural
start, with `trace-commons-contributor update` available to apply it
immediately. This preserves the rule that no swap happens silently underneath a
running process without inventing a notification surface the daemon does not
have.

## Quiesce

The daemon may be mid-upload when an update is ready. It gains an IPC verb that
drains in-flight uploads and parks the queue at a safe point, bounded by a
timeout. If the drain does not complete in time, the update stays staged and
retries later. The swap never forces its way past active work, and a
half-uploaded trace is never the cost of an update.

## Per-platform design

### macOS — Sparkle

Sparkle 2.x, configured with automatic checks on and automatic install off,
which is exactly the download-silently-prompt-before-install behavior above as
Sparkle's stock configuration.

Verification is Sparkle's own and is stronger than what we would hand-roll:
EdDSA over the appcast *plus* Apple Developer ID code signing, where the Apple
signature is what authorizes any future rotation of the EdDSA key. A compromised
web server therefore cannot push a malicious update.

When a Homebrew Caskroom entry is present, Sparkle is never started and
Settings shows "updates managed by Homebrew" with the `brew upgrade --cask
trace-commons` command.

Build work: embed and inside-out-sign `Sparkle.framework` into
`Contents/Frameworks` in `make-app-bundle.sh`, then notarize. The script
already embeds, rpaths, and signs the FFI dylib in that same directory, so this
follows a proven path in that file rather than a new one.

The app targets macOS 14; Sparkle 2.x requires macOS 12+.

### Linux — the Flatpak portal

The GTK app creates an `org.freedesktop.portal.Flatpak.UpdateMonitor`. The
portal signals when an update for *this app* becomes available, and installs it
on user confirmation. It is scoped so an app can only ever update itself,
nothing else. Flatpak added it specifically because "homegrown methods of doing
so are unreliable at best, and insecure at worst" — a sandboxed app must not
try to replace its own bytes directly, and this design does not.

Detection is `/.flatpak-info`. Running outside flatpak (built from source), the
app degrades to a check-and-notify banner and installs nothing.

The `install.sh` CLI on Linux takes the Rust self-update path below.

### Windows and the CLI — Rust, no new dependencies

`reqwest`, `sha2`, `serde_json`, and `ring` are already direct dependencies of
`trace-commons-contributor`, and `ring` is already the Ed25519 verifier used
for upload claims. The whole path is covered without adding anything.

Flow: fetch `latest.json` → verify the Ed25519 signature → compare versions →
download the asset → verify sha256 → verify the Authenticode signature via
`WinVerifyTrust` against the expected signing subject → quiesce → rename the
running exe aside, move the new one into place, and schedule the old one for
deletion via `MoveFileEx` with `PendingFileRenameOperations`.

Steps 3–6 are deliberately `install.ps1`'s existing verification logic
reimplemented in Rust; that script is the reference for what "verified" means
on Windows, and the two must not diverge.

The same code minus the Authenticode step serves the `install.sh` CLI on macOS
and Linux, where replacing a running binary is an ordinary unlink-and-rename.

## Failure behavior

Fail closed at every step, per the repo's convention:

- manifest signature does not verify
- sha256 does not match
- the artifact's code signature is absent or not the expected identity
- the offered version is not strictly greater than the running version

Each of these aborts, keeps the current binary, logs hash-only, and backs off.
There is no path that installs an unverified artifact, and no path that
downgrades. Downgrade protection is explicit and tested, not incidental: a
signed-but-old manifest replayed at a client is otherwise a working attack.

## Testing

Because the implementation is per-platform rather than a shared core, the
security-critical verify-before-swap logic exists three times. That is the
accepted cost of the architecture choice, and the mitigation is a shared
conformance fixture set at `tests/fixtures/update-conformance/`:

- a good artifact that must install
- an artifact whose bytes were tampered with after signing
- a manifest with an invalid signature
- an unsigned binary
- a downgrade attempt

Both the Rust tests and the Swift tests consume these fixtures, so a dropped
check in one implementation fails a test rather than shipping. A dedicated CI
job covers the Windows verify path, in the spirit of the existing named-pipe
ACL job — that path cannot be exercised from a cross-compile.

## Out of scope

Delta updates. Staged rollout percentages. Forced or mandatory updates. A
Windows GUI app.

## Recorded deviation: Sparkle adopted without a dependency workup

The repo's dependency policy calls for a written workup — adoption, transitive
dependency count, maintenance status, license, advisories — and explicit
approval before adding a direct dependency. Sparkle was approved on 2026-08-17
without that workup, as a deliberate call rather than an oversight.

Known facts: Sparkle 2.x, MIT licensed, macOS 12+, SwiftPM-supported, the
de-facto standard updater for non-App-Store macOS apps.

Known integration risk: SwiftPM delivers Sparkle as a binary XCFramework, while
this app's bundle is hand-assembled by `make-app-bundle.sh` rather than by
Xcode. Wiring the framework, its XPC services, and the inside-out signing order
into that script is the most likely place this slice hits friction.
