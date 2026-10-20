# Packaging and distribution

Target install commands:

```bash
brew install nearai/tap/ironwire
npx ironwire@latest connect claude
sudo apt install ironwire
pip install ironwire
curl -fsSL https://ironwire.dev/install.sh | sh
```

All five deliver **the same prebuilt Rust binary**. Nothing is built on the
user's machine and nothing is reimplemented per ecosystem — the npm/pip packages
are thin shims that select and exec the right platform binary.

---

## Build matrix

| Target | Notes |
|---|---|
| `aarch64-apple-darwin` | primary |
| `x86_64-apple-darwin` | |
| `x86_64-unknown-linux-gnu` | |
| `aarch64-unknown-linux-gnu` | |
| `x86_64-unknown-linux-musl` | static; what the shell installer prefers on unknown distros |
| `x86_64-pc-windows-msvc` | |

Binary size is worth keeping down — five ecosystems ship this one file and
users re-download it on every update — which is part of why the ironclaw reuse
is feature-gated (DESIGN §7): the default build does not pull `rig-core` or the
AWS SDK.

It is **not a hard budget**. CI reports the size on every build so a jump shows
up in review, and nothing fails on a threshold. A capability worth its bytes
should not be blocked by a number picked in advance; the point is that growth is
a decision someone makes rather than something that happens quietly.

Current default build: **~7 MB stripped** on linux-x64. Most of the growth from
the initial ~5 MB is bundled SQLite for the trace ledger — a deliberate trade:
vendoring it keeps the install a single self-contained file on every platform,
which matters more here than two megabytes.

---

## Mechanism

`cargo-dist` produces the shell installer, the Homebrew formula, the npm
package and the MSI from one config. IronClaw already uses
`[package.metadata.dist]`, so the tooling is familiar in this org.

### Homebrew

`cargo-dist` generates a formula pushed to `nearai/homebrew-tap`. It ships the
binary plus a `brew services` plist so `ironwire serve` can run as a launchd
agent.

### npm

`ironwire` is a thin package whose `postinstall` (or lazy `bin` shim) picks the
matching `@ironwire/cli-<platform>-<arch>` optional dependency and execs its
binary. `npx ironwire@latest` therefore works with no build step. Optional
dependencies with `os`/`cpu` fields mean a user only downloads their own
platform.

### apt

`nfpm` (driven from the same release job) builds a `.deb` carrying the binary, a
systemd **user** unit (`ironwire.service`, `WantedBy=default.target`) and the
shell completions. Hosted at `apt.ironwire.dev` with a signed `Release` file.
Lowest priority of the five: Linux developers reach for the shell installer, and
the apt repo is the only option here that needs ongoing infrastructure.

### pip

A wheel per platform tag (`macosx_11_0_arm64`, `manylinux_2_28_x86_64`, …)
containing the binary and a console-script entry point that execs it. No
`maturin`, no compilation — the wheel is a delivery vehicle. Weakest fit of the
five, but it is the natural channel for Aider users, who install their agent
with pip.

### Shell installer

Detects platform, downloads the matching archive, verifies a checksum, installs
to `~/.ironwire/bin`, and prints the `PATH` line. This is the one to get right
first: it is the fallback for every platform the others miss.

---

## Runtime layout

```
$IRONWIRE_HOME              default ~/.ironwire   (0700)
├── config.toml             user configuration
├── consent.json            recorded subscription consents (TRUST §2)
├── control.token           control-API token (0600)
├── daemon.lock             single-daemon lockfile
├── ledger.sqlite           local trace ledger
├── catalog.json             signed provider catalog (docs/UPDATES.md)
├── quota.json              observed capacity, carried across restarts (0600)
├── update.json             cached update check, at most one a day
└── bodies/                 captured bodies, only if capture.bodies = true
```

Nothing here is world-readable. Nothing here is synced.

## Service management

| Platform | Mechanism |
|---|---|
| macOS | launchd agent (`brew services start ironwire`, or `ironwire service install`) |
| Linux | systemd **user** unit — the daemon holds user credentials and must not run as root |
| Windows | scheduled task at logon |

`ironwire serve` in the foreground stays first-class: it is what the shell
installer tells you to run, and it is what `doctor` assumes.

---

## Release process

1. Tag `vX.Y.Z`.
2. CI builds the matrix, runs the conformance harness (PROTOCOL §7) on
   macos-arm64 and linux-x64, and reports the binary size.
3. `cargo-dist` publishes the GitHub release, the tap commit and the npm
   packages; `nfpm` pushes the `.deb`; the wheel job pushes to PyPI.
4. Sign the release and publish `manifest.json` (`latest`, `minimum_supported`,
   `summary`) at the pinned URL. `install.sh` and `ironwire update` both read
   it, so there is one source of truth for "what is latest".
5. `ironwire update` **notifies only** — it never downloads or applies anything,
   and it prints the command belonging to the user's install. See
   [`UPDATES.md`](./UPDATES.md).

---

## The macOS menu bar app

Built, in [`macos/`](../macos/README.md): a SwiftUI `MenuBarExtra` that is a pure
client of the control API. It renders `/_ironwire/status`, holds
`/_ironwire/events` open, and posts to `/_ironwire/pin` — and decides nothing, because
the daemon is the only brain (`docs/DESIGN.md` §6).

It is a Swift package rather than an `.xcodeproj`, so the same `make -C macos
dist` produces the bundle locally and in the release job. `macos/README.md` has
the reasoning and the other decisions worth knowing about (the App Sandbox is
off, so the app can read `control.token`).

### How it ships

**As its own artifact, not inside the binary tarballs.** The archive layout above
is the installer's contract — a single `ironwire` at the root — and
`scripts/install.sh` finds the binary by searching the unpacked tree. A second
executable in there would be a hazard for no gain.

| Where | What |
|---|---|
| Release | `IronWire-macos.zip`, universal, built on the `aarch64-apple-darwin` runner |
| Homebrew | a `resource` in the formula, staged into `prefix` inside `on_macos` |
| npm / pip / apt | not carried — see below |

The formula omits the app entirely when the artifact is missing, so a release
that skipped the macOS runner still installs the binary. Formulae do not link
`.app` bundles into `/Applications` the way casks do, so the caveats say where
it landed.

npm and pip *could* carry it — `@ironwire/cli-darwin-arm64` and the macOS wheels
are the only packages a Mac pulls, so it would cost Linux and Windows users
nothing — but neither has any business copying an app into `/Applications`, and
`build_npm.mjs` refuses install scripts on purpose. That would need an
`ironwire menubar install` command, which does not exist.

### What is still missing: signing and notarisation

The bundle is **ad-hoc signed**. That is enough to execute on Apple Silicon and
enough to run a locally built copy, and it is enough for the Homebrew path today
because `brew` unpacks the resource itself rather than handing the user a
download. It is **not** enough for a bundle a user downloads directly:
un-notarised apps are blocked by Gatekeeper on first launch, and clearing the
quarantine attribute by hand is not a thing to ask of anyone.

Proper signing, notarisation and a `.dmg` need an Apple Developer certificate,
which this project does not have (`docs/ROADMAP.md`). Everything else is in
place: the remaining step is a credential, not a redesign.

### CI

`make -C macos test app` runs on the **macOS leg of `ci.yml` only**. Linux
runners cannot build Swift, and a red matrix on every push would be worse than
no coverage — but the alternative, finding a Swift compile break during a tag
build after the tag is pushed, is the failure the packaging job exists to
prevent. The guard is `if: matrix.os == 'macos-latest'`.
