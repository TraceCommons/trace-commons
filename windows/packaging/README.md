# MSIX packaging for the Windows contributor app

Everything here is **additive and opt-in**. The shipping Windows app artifact
is still `trace-commons-app-windows-x86_64-<version>.zip`: a self-contained,
unpackaged (`WindowsPackageType=None`), Authenticode-signed publish tree that
unzips and runs. Nothing in this directory changes that build, and the release
job produces that zip on a tag push exactly as it did before.

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
against `signtool` on Windows** -- see "What is unverified" below.

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

   Consequence worth stating plainly: the packaged flavour requires 10.0.19041
   where the unpackaged build supports 10.0.17763, and `unvirtualizedResources`
   is a restricted capability that a Store submission would have to justify.
   The zip remains the artifact with the wider reach.

## The logo assets are placeholders

`Assets/*.png` are flat `#315FBA` squares generated to satisfy the manifest's
required image references. They are the app's accent colour and nothing more.
The repository has no Windows app icon of any kind today -- the unpackaged
build ships without one either -- so this is not a regression, but an MSIX
carrying a solid blue square as its Start-menu tile should not be published to
contributors. Replace these before the MSIX becomes a shipping artifact.

## What is unverified

None of this has been built. It was authored on macOS, which cannot build WinUI
or MSIX at all. Specifically unproven:

- that `TcPackaged=true` produces a package at all;
- that `signtool` accepts the publisher string against a real Trusted Signing
  certificate (the failure mode is an outright signing refusal, which is loud);
- that the virtualization opt-out behaves as documented, i.e. that the packaged
  app sees the CLI's real state directory;
- that the manifest-declared protocol handler resolves to the packaged app.

The first three need a Windows runner; the second additionally needs the
signing identity, which pull-request CI does not have. The MSIX steps in
`release-apps.yml` are therefore gated behind an explicit `workflow_dispatch`
input and never run on a tag push, so the first attempt cannot damage a real
release.
