# MSIX packaging for the Windows contributor app

Everything here is **additive and opt-in**. The shipping Windows app artifact
is still `trace-commons-app-windows-x86_64-<version>.zip`: a self-contained,
unpackaged (`WindowsPackageType=None`), Authenticode-signed publish tree that
unzips and runs. Nothing in this directory changes that build. An `app-v*`
tag push builds, signs and publishes the MSIX and its `.appinstaller` feed
(the `windows-app` job); the zips (the `windows` job) are built only on a
`workflow_dispatch`, which never publishes them. From `app-v0.23.0` until
the MSIX went back on tags, a tag push built no Windows artifact at all
(see `docs/release-runbook.md`).

The MSIX path is entered only by passing `TcPackaged=true` to MSBuild, which is
done in one place: `windows/scripts/make-msix.ps1`.

## The publisher string, and why it is what it is

MSIX refuses to install when the `<Identity Publisher="...">` in the manifest is
not byte-identical to the subject of the certificate the package was signed
with. It is the one value in the manifest that cannot be guessed.

It was not guessed. It was read out of a published, signed release artifact --
`trace-commons-contributor.exe` from `app-v0.2.1` -- by pulling the Authenticode
PKCS#7 blob out of the PE certificate table and printing the leaf subject:

```
$ openssl x509 -in leaf.pem -noout -subject -nameopt RFC2253
subject=CN=Iqlusion Inc,O=Iqlusion Inc,L=Santa Clara,ST=California,C=US

$ openssl x509 -in leaf.pem -noout -subject -nameopt oneline
subject=C = US, ST = California, L = Santa Clara, O = Iqlusion Inc, CN = Iqlusion Inc
```

Issued by `CN=Microsoft ID Verified CS AOC CA 03`, which is Azure Trusted
Signing -- the same identity the Authenticode pass already uses. MSIX signing
must use that identity too; a second signing scheme is not introduced.

Windows renders `stateOrProvinceName` as `S=` rather than OpenSSL's `ST=`, and
renders the RDNs most-specific-first, so the manifest carries:

```
CN=Iqlusion Inc, O=Iqlusion Inc, L=Santa Clara, S=California, C=US
```

`S=` and `ST=` both parse to OID 2.5.4.8, so either spelling encodes to the same
DER; the ordering is the part that matters. **This has not been confirmed
against `signtool` on Windows** -- see "What is verified, and what is not" below.

`Identity Name` (`Iqlusion.TraceCommons`) is a choice, not a constraint, for
sideloaded and directly-downloaded MSIX. If this app is ever submitted to the
Microsoft Store, the Store assigns both the package name and the publisher
(`CN=<GUID>`), and this manifest would need those values instead. That is an
owner decision, not a packaging one, and it is not made here.

## What MSIX changes at runtime

Two per-user paths change under a package, and neither change is acceptable
silently, so both are handled explicitly.

1. **The URL scheme.** `UrlSchemeRegistration.cs` writes
   `HKCU\Software\Classes\tracecommons` at startup in the unpackaged build.
   Under MSIX the OS owns protocol registration through the
   `windows.protocol` extension in `Package.appxmanifest`, and the runtime
   write is the wrong mechanism. `EnsureRegistered()` now returns early when
   the process has package identity, so exactly one of the two paths is live
   at a time and they cannot disagree.

2. **The state directory and the onboarding file.** A packaged desktop app's
   `%LOCALAPPDATA%` writes are redirected into the package's private
   `LocalCache`. `DaemonHost.DefaultConfigDir()` is
   `%LOCALAPPDATA%\trace-commons` and is deliberately the same directory the
   unpackaged contributor CLI uses; redirected, the packaged app would watch an
   empty private copy and show an empty queue -- a silent, plausible-looking
   failure. `OnboardingState` writes
   `%LOCALAPPDATA%\TraceCommons\onboarded.json` and would re-onboard a
   contributor who had already used the zip build. The manifest therefore
   disables both file-system and registry write virtualization, which needs the
   `unvirtualizedResources` restricted capability and Windows 10 2004.

   The packaged flavour therefore requires 10.0.19041. Since 2026-11-19 the
   unpackaged build has the same floor (`TargetPlatformMinVersion` in
   `TraceCommons.App.csproj`, raised from 10.0.17763 when the owner chose to
   support Windows 10 22H2 and Windows 11 only), so the zip and the MSIX reach
   the same machines. `unvirtualizedResources` is a restricted capability that
   a Store submission would have to justify.

## The logo assets are generated, not edited

`Assets/*.png` carry the mark. They are rendered by
`crates/trace-commons-mark` and written by
`cargo run -p trace-commons-mark --bin mark-export -- assets/mark --repo-root .`;
`scripts/mark/check-drift.sh` regenerates them in CI and fails if the committed
bytes differ, so editing one by hand is a build failure rather than a surprise
in the Start menu.

They were flat `#315FBA` squares -- one unique pixel each -- for as long as they
were, because they had the right filenames at the right dimensions in the right
brand colour, and nothing that looked at them looked at the pixels. If you are
adding an asset here, add it to `windows_tiles()` in that crate rather than
committing a file; a file nothing can regenerate is a file nothing can verify.

The renderer is deliberately dependency-free and platform-independent so the
drift check can run on `ubuntu-latest`. An earlier fix put the mark on these
tiles using CoreGraphics, which meant only a Mac could regenerate them and the
check had to skip them entirely.

## What is verified, and what is not

Built, signed with a throwaway certificate, installed through its feed and
updated in place on Windows Server 2025 (tc-win-az3, 2026-11-19 and
2026-11-20; see `docs/operator/windows-app-release-checklist.md`):

- `TcPackaged=true` produces one package, and `make-msix.ps1` builds it under
  Windows PowerShell 5.1 and PowerShell 7;
- the virtualization opt-out works: the packaged app writes the CLI's real
  `%LOCALAPPDATA%\trace-commons`;
- the manifest-declared protocol handler is registered by the deployment;
- the `.appinstaller` feed installs, registers the intended update settings,
  and the in-app update installs the next version with the state intact. It
  does not start the app again afterwards.

Still unproven: that `signtool` accepts the publisher string against a real
Trusted Signing certificate (the failure mode is an outright signing
refusal, which is loud), which only the release job's dispatch run
exercises, and any of it on a Windows 10 or 11 client or as a standard user.
