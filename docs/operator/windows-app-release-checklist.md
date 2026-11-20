# Windows app release checklist

What the owner does to put the Windows app out again, and what the release
jobs need to be in place first. Written 2026-11-19 against
`.github/workflows/release-apps.yml` on `main`; reread the workflow before
using it, because it is the source of truth.

## Where things stand

- **A tag push publishes the Windows MSIX.** Since the commit that put
  `windows-app` back on tags (revert it alone to undo), an `app-v*` push
  runs `windows-app` beside `macos`: it signs, installs and checks the
  MSIX, uploads it and then `TraceCommons.appinstaller` to the bucket, and
  `publish` attaches both to the GitHub Release. The `windows` zip job
  stays dispatch-only (decision 3 below: the zips do not return).
- **A dispatch can publish the MSIX and feed, and nothing else.** Set
  `publish_windows_app` on a dispatch run on the `app-v<version>` tag
  (`version` equal to it, `platform` `windows` or `all`). `release-config`
  refuses it on any other ref and checks the publishing credentials first.
  It never cuts or edits the Release and never writes `latest.json`. Any
  other dispatch publishes nothing.
- **The live Windows artifacts are from app-v0.23.0** until the first tag
  after that change. The `.appinstaller` feed at
  `https://storage.googleapis.com/tracecommons-flatpak/windows/TraceCommons.appinstaller`
  and the last GitHub Release with Windows assets both date from it.
- **Turning tag releases back on is the owner's decision.** The change is
  prepared, in its own commit:
  1. `windows-app`'s `if:` admits `github.event_name == 'push'`;
  2. the six `AZURE_SIGNING_*` values are back in the `release-config`
     preflight, for a tag push and a publishing dispatch;
  3. only the MSIX job returns; the CLI/portable-zip job (`windows`) does
     not.
- **Release gates not yet met** (parity spec
  `docs/superpowers/specs/2026-11-18-windows-mac-parity-design.md`): the
  physical Windows 11 PC that signs off a slice as released does not exist
  yet; the W-slice pull requests are drafts; W10 (manifest names and the
  NEAR AI N tiles) is in #2209 and not on `main`.

## Inputs the Windows jobs read

| Input | Where | Used by | Status 2026-11-19 |
|---|---|---|---|
| `AZURE_SIGNING_CLIENT_ID`, `_TENANT_ID`, `_SUBSCRIPTION_ID` | variables, environment `release` | `azure/login` (OIDC, no key) | present (set 2026-08-16) |
| `AZURE_SIGNING_ENDPOINT`, `_ACCOUNT`, `_PROFILE` | variables, environment `release` | `scripts/windows/setup-trusted-signing.ps1` | present (set 2026-08-16) |
| Federated credential subject `repo:...:environment:release` | Azure app registration | OIDC token match | not readable from here; the jobs declare `environment: release` for it |
| `TRUSTED_SIGNING_CLIENT_SHA256` | literal in the workflow | dlib download check | pinned 2026-08-16 |
| `GCP_WIF_PROVIDER`, `GCP_FLATPAK_PUBLISHER_SA` | repository secrets | `windows-app` Publish (feed upload) | present |
| Package identity `Iqlusion.TraceCommons` | `windows/packaging/Package.appxmanifest` | install check, feed | kept (W10-D1) |
| Publisher | read back from a probe signature, stamped by `stamp-package-identity.ps1` | MSIX signature, feed | derived per run |
| Version | tag `app-vX.Y.Z` or dispatch input `version` | stamped as `X.Y.Z.0` | three-part numeric only |
| Rust toolchain | `1.96`, `x86_64-pc-windows-msvc` | both jobs | pinned in the workflow |
| .NET SDK | `8.0.x` (`windows/global.json`) | both jobs | pinned |
| Windows floor | `TargetPlatformMinVersion` 10.0.19041.0 and manifest `MinVersion` 10.0.19041.0 | app and MSIX | equal; `release_pipeline.rs` pins both |

The tag must also equal the GTK crate's version
(`crates/trace-commons-contributor-gtk/Cargo.toml`), checked in the `version`
job on a tag push; the release-prep commit bumps the contributor crates
together. #2136 (1.0.8) is the shape: `trace-commons-contributor`,
`-sources`, `-insights`, `-platform`, `-ffi` and `-gtk` `Cargo.toml`, the GTK
metainfo, and the three lockfiles (root, GTK, Tauri). No Windows file carries
the version; the MSIX version is stamped from the tag at build time.

## Before tagging

1. Every W-slice PR that the release note names is merged, and the release
   note (`docs/announcements/DRAFT-windows-app-parity.md`) is trimmed to it
   and approved.
2. #2209 (W10) has landed: the three manifest display names equal `APP_NAME`
   ("Near AI Gateway"), and the tiles carry the NEAR AI N. Its invite protocol
   name (`<uap:DisplayName>`) still reads "Near.ai invite" on #2209's head at
   6ce0202c; W10-D2 (a) says it becomes `APP_NAME` alone.
3. The physical Windows 11 PC sign-off (parity spec, test matrix), and a
   launch on Windows 10 22H2, the new floor. Nothing has run on Windows 10
   yet; the trial below was Windows Server 2025.
4. A dispatch dry run on the release commit: run `release-apps` with
   `platform=windows`, `version=<next>`, `msix=false`. It signs with the real
   profile, installs the MSIX on the runner, checks write virtualization is
   off, and generates the feed, and publishes nothing. Read the
   `windows-msix` and `windows-zip` artifacts.
5. Bump the contributor crate versions (the #2136 file list) in a release
   PR and merge it.

## Tagging

1. The workflow change above (the MSIX back on tags), reviewed and merged.
2. `git tag app-vX.Y.Z <release commit>` and push the tag. That one push runs
   macOS and the Windows MSIX.
3. If only the Windows leg failed, fix the cause and re-run that job, or
   dispatch `release-apps` on the tag with `platform=windows`,
   `version=X.Y.Z` and `publish_windows_app` set. That republishes the
   package and feed; attach the MSIX to the Release by hand if `publish`
   ran without it.

## Where it lands

- GitHub Release `app-vX.Y.Z` (`publish` job): the MSIX with its `.sha256`
  and `TraceCommons.appinstaller`. (The CLI and portable app zips come from
  the dispatch-only `windows` job and are not on a tag's Release.)
- `gs://tracecommons-flatpak/windows/<package>.msix` (immutable cache), then
  `gs://tracecommons-flatpak/windows/TraceCommons.appinstaller` (no-cache),
  from the `windows-app` job, before the Release is cut.
- Build provenance attestations for every signed binary and the MSIX.
- `gs://tracecommons-flatpak/updates/latest.json` and `latest.json.sig`
  (`publish-updates` job, which runs when `macos` or `windows` succeeded):
  a tag push has no `windows` job, so no `windows-x86_64` entry, and
  `windows-x86_64-cli` points at the `contributor-vX.Y.Z` Release's exe. It signs with `update-manifest-signing-key` from Secret Manager in
  `tracecommons-pilot-2026`, through the same `GCP_WIF_PROVIDER` and
  `GCP_FLATPAK_PUBLISHER_SA`. The MSIX is not in this manifest; installed
  packages follow the `.appinstaller` feed.

## Also fix before the first Windows tag

- The `publish` job's release notes open with "Trace Commons contributor
  apps." That text is on the public Release page; the owner's rule is that
  "Trace Commons" is never user-visible.
- The `windows` job publishes the app without `-p:Version`, so the portable
  app's assembly version is the SDK default until #2216 (which reads the
  contributor crate's version) lands. The MSIX is unaffected: its version is
  stamped from the tag.
- `windows/scripts/make-msix.ps1` uses three-argument `Join-Path`, which
  Windows PowerShell 5.1 rejects ("A positional parameter cannot be found
  that accepts argument '..'"). The workflow runs it under `pwsh`, so only a
  hand run on a box without PowerShell 7 hits it.

## Trial on tc-win-az3, 2026-11-19

A trial of the `windows-app` job's steps by hand on the Azure Windows Server
2025 box, from `main` (46466b6b5) plus the floor change and #2225, with a
throwaway self-signed certificate (`CN=TC Release Trial Throwaway`, trusted
in LocalMachine TrustedPeople for the trial and deleted afterwards):

| Step | Result |
|---|---|
| FFI release build, app Release build, identity stamp (`1.0.11` -> `1.0.11.0`), `TcPackaged` package | one `.msix`, 87 MB |
| signtool sign and `verify /pa` | both succeeded |
| `make-appinstaller.ps1` (Windows PowerShell 5.1) | valid 2017/2 feed; MainPackage name, publisher and version equal the package's |
| `Add-AppxPackage` as a new local standard (non-admin) user | registered `Iqlusion.TraceCommons_1.0.11.0_x64`, `MinVersion` 10.0.19041.0 |
| Launch from `shell:AppsFolder` | window "Near AI Gateway" alive after 25 s |
| Run at login | startup task `TraceCommonsStartup` registered, state 0 (disabled) after launch; no HKCU Run value written |
| URL protocol | `tracecommons` registered by the deployment (HKCU stub with no `shell\open\command`, so not the unpackaged runtime writer); removed with the package |
| `Remove-AppxPackage` | clean: no package, no protocol key, no Run value |

Not covered: a sample-mode launch (a Release build has no sample client, so
the packaged launch ran the real app in an empty throwaway account and was
not touched); installing through the `.appinstaller` feed (the generator
requires an https base, and the trial had no trusted local https endpoint);
`scripts/install.ps1` (no dry-run mode; it always resolves the real
`contributor-v*` release, so it was parse-checked only); the Trusted Signing
profile and the virtualization probe, which only the dispatch run exercises.

## After

- Install from the feed on a clean standard user and confirm the version.
- Confirm an installed earlier build updates (App Installer checks on launch;
  the app's own update action needs the `packageManagement` capability, which
  the manifest declares).
