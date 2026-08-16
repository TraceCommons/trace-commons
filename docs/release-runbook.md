# Release Runbook

This document records the operational steps and invariants for cutting a
signed, distributable release of the Trace Commons contributor apps (the
macOS app bundle, the CLI, and the Linux flatpak). It is the companion to
`crates/trace-commons-contributor/tests/release_pipeline.rs`, which pins the
properties this runbook describes wherever they are visible in text (env
contracts, mandatory flags, deliberate exclusions). What the tests cannot
prove — that signing, notarization, or a flatpak build actually succeed
against real credentials — is covered by the manual gates below.

## Homebrew

The macOS app and CLI are distributed via a Homebrew tap:
`TraceCommons/homebrew-tap` (public, on GitHub), consumed as
`brew tap TraceCommons/tap`.

- `Casks/trace-commons.rb` installs the notarized `TraceCommons.app` from the
  DMG published as `TraceCommons-<version>.dmg` on releases of
  `TraceCommons/trace-commons-server` tagged `app-v<version>`.
- `Formula/trace-commons-contributor.rb` installs the signed CLI from the
  zips published as `trace-commons-contributor-<target>.zip` on releases
  tagged `contributor-v<version>`, for both `aarch64-apple-darwin` and
  `x86_64-apple-darwin`.
- Minimum macOS is Sonoma (`depends_on macos: ">= :sonoma"`), matching the
  app's `LSMinimumSystemVersion` of `14.0`.
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
with the `version` fields. After the first release, Task 12's version-bump
automation takes over updating these values on subsequent releases.
