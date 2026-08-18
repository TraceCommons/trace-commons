# Update manifest publishing

`generate-manifest.sh` writes `latest.json` and a detached Ed25519 signature
over its exact bytes. Clients verify the signature before parsing; see
`crates/trace-commons-contributor/src/update/manifest.rs`.

## Keys

The private key lives in GCP Secret Manager as `update-manifest-signing-key`
in project `tracecommons-pilot-2026`, alongside `flatpak-signing-key`. It is
never written to a runner's disk outside the release job's temporary
directory, and never printed.

Generate a new key with:

    openssl genpkey -algorithm ed25519 -out update-signing.pem

Export the raw 32-byte public key that clients pin (the last 32 bytes of the
DER SubjectPublicKeyInfo):

    openssl pkey -in update-signing.pem -pubout -outform DER | tail -c 32 | xxd -p -c 32

## Rotation

Clients pin the public key at build time, so rotating it means shipping a
release signed by the old key that carries the new key, then switching. Do
not rotate without that two-step, or every installed client stops seeing
updates.

## Sparkle appcast

`generate-appcast.sh` writes `appcast.xml` for the macOS app. Sparkle's EdDSA
key is separate from the manifest key above and lives in GCP Secret Manager as
`sparkle-signing-key`. Generate it with Sparkle's `generate_keys` tool and
store the public key in the app's Info.plist as `SUPublicEDKey`.

Sparkle compares `sparkle:version` (CFBundleVersion), not the short version,
so the appcast must carry the same monotonic build number the release
workflow stamps into the bundle.

## Winget

`WINGET_PKGS_TOKEN` is a fine-grained PAT with contents and pull-request write
on a fork of `microsoft/winget-pkgs`. `github.token` cannot reach another
repository, which is why this secret exists.

## Platform slugs

Two families:

| Slug | Artifact | Consumed by |
|---|---|---|
| `macos-universal` | `TraceCommons-<v>.dmg` | Sparkle (via the appcast, not this file) |
| `windows-x86_64` | `trace-commons-windows-x86_64-<v>.zip` | winget |
| `linux-x86_64` | flatpak repo | flatpak portal |
| `windows-x86_64-cli` | `trace-commons-contributor-x86_64-pc-windows-msvc.exe` | the CLI self-updater |
| `linux-x86_64-cli` | `trace-commons-contributor-x86_64-unknown-linux-gnu` | the CLI self-updater |
| `macos-aarch64-cli` | `trace-commons-contributor-aarch64-apple-darwin` | the CLI self-updater |
| `macos-x86_64-cli` | `trace-commons-contributor-x86_64-apple-darwin` | the CLI self-updater |

The CLI slugs carry a bare binary, never an archive: the CLI has no archive
extractor and no new dependency may be added to give it one. The desktop
`windows-x86_64` zip is the winget payload, and the CLI defers to winget
rather than consuming it.
