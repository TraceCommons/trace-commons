# Trace Commons and Near AI Gateway

This repository holds two things that work together.

**Trace Commons** is a user-attributable collective of AI agent work. When an AI agent
works for someone, it leaves a record of what happened. Trace Commons keeps
that record under the contributor's control: traces are scrubbed on the
contributor's own machine, only the scrubbed version is shared, and only with
their consent. A server accepts records that are new and substantive, and
contributors earn Trace Credit for what is accepted. The server, the protocol
and the register's storage live here.

**Near AI Gateway** is the Mac app people use to take part. It watches the
coding agents you choose, scrubs and shares traces with Trace Commons on your
terms, and can route your tools' AI requests through NEAR AI's private
inference. The app, the contributor CLI and the background service they share
live here too.

## Near AI Gateway

**A Mac app for people who work with AI coding agents.** It keeps the record
of that work under your control, lets you earn credit for sharing it, and can
route your tools' AI requests through NEAR AI's private inference.

For macOS 14 or later, plus the contributor CLI for macOS, Linux and Windows.
The Windows and Linux desktop apps remain on their existing releases.

### What it does

- **Watches the folders you choose.** It finds the session transcripts that
  Claude Code, Codex, Gemini CLI, Cline and OpenCode leave on your Mac, and
  reads only the folders you allow.
- **Keeps raw traces on your Mac.** Each trace is scrubbed on your computer
  before anything leaves it. Sharing is off until you turn it on, and you
  choose which uses your traces allow, such as evaluation or training coding
  models.
- **Earns credit for accepted traces.** Shared traces are checked on the
  server for whether they are new and substantive. Accepted ones earn Trace
  Credit, and first run shows what your traces could earn before you share.
- **Private AI.** Your tools can send their AI requests through the app, to
  be answered by NEAR AI after you sign in. No tool switches over until you
  connect it, and requests are logged on this Mac.
- **Insights.** A private view of your own agent work, kept on this Mac.

### Install

#### The Mac app

Download the DMG from the [releases page][releases], or install the Homebrew
cask:

```bash
brew install --cask TraceCommons/tap/trace-commons
```

The DMG is universal (Apple silicon and Intel) and notarized. It installs
`Near.ai.app`, shown as Near AI Gateway.

This repository publishes two release streams: app releases are tagged
`app-v*` and CLI releases `contributor-v*`. GitHub's "latest release" is
whichever stream was cut most recently, and an app release carries no CLI
binary, so pick the newest release of the kind you want from the
[releases page][releases].

#### The CLI

Signed binaries are published per release, so contributors do not need this
repository or a Rust toolchain. [Contributor CLI](#contributor-cli) below
says what each installer checks and how to use the CLI once it is installed.

```bash
# macOS or Linux: installs to ~/.local/bin, no sudo
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.sh -o install.sh
sh install.sh

# macOS, from the Homebrew tap instead
brew tap TraceCommons/tap
brew trust tracecommons/tap          # Homebrew refuses untrusted third-party taps
brew install trace-commons-contributor
```

```powershell
# Windows, in PowerShell
irm https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1
```

[releases]: https://github.com/TraceCommons/trace-commons/releases

## Trace Commons: a user-attributable collective of AI agent work

When an AI agent does work for someone, it leaves a record of what actually
happened: the tools it called, the places it failed, the result it gave back.
That record is becoming valuable. The companies building the next generation
of agents need millions of those records to train against, and most of them
live inside private user sessions today — collected unilaterally by whoever
runs the model, on terms the user never specifically agreed to.

TraceCommons keeps the record under the contributor's control. Capture and
scrubbing both happen on the user's machine; only the scrubbed version moves
to a shared server, where two checks decide whether the record is worth
keeping. One asks whether the record is genuinely different from everything
already filed. The other asks whether it is substantive work rather than
template-shaped filler. Both must pass. Accepted records are signed, dated,
and filed into a register. Frontier labs, auditors, and regulators can query
the register under selective disclosure; they see what they need, and the
rest stays encrypted.

A **Trace Credit** is the signed, on-chain record that one of a contributor's
submissions was accepted into the register. Credits are how recognition flows
back when buyers later pay to query the evidence. They are non-transferable.

The contract is "local-first, opt-in, scrub before upload":

- Trace contribution is **off by default**. Raw traces stay on the user's
  device unless they explicitly opt in.
- Uploads carry only `ironclaw.trace_contribution.v1` envelopes — text and
  tool payloads are stripped or replaced with stable placeholders during
  local deterministic scrubbing.
- The server gates incoming envelopes on **two** axes — novelty against the
  existing register and substantive-work signal against a frontier model.
  In Phase A this runs on regular GPU hardware in NEAR AI's TEE-hosted vLLM;
  the Phase B milestone moves scoring inside attested hardware that even the
  operators of the server cannot read.
- Accepted, settled records mint **Trace Credits** through a hash-only
  utility-attestation pipeline. Credits are non-transferable, bound to
  reviewed evidence, and settle on-chain via NEAR; uploads alone don't pay.

The envelope contract and threat model are in
[`docs/trace-commons.md`](docs/trace-commons.md). Deployment status and the
phase plan are in [`docs/status.md`](docs/status.md) and
[`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md).

## The repository

Three things live here:

- **The hosted control plane**: ingest, review, retention, revocation,
  encrypted artifact storage, upload-claim issuing, audit chain, and credit
  settlement. It is PostgreSQL-only.
- **The contributor side**: the CLI and background daemon
  (`trace-commons-contributor`), which discover, locally redact, and submit
  traces, and the C ABI (`trace-commons-contributor-ffi`) that the macOS and
  Windows apps link.
- **The desktop apps**: the native SwiftUI macOS app in `macos/` is the lead
  client and the one new releases ship. The Windows app (`windows/`, C#),
  the Linux GTK app (`crates/trace-commons-contributor-gtk`) and the Tauri
  app (`tauri-desktop/`) are frozen.

Ironclaw is a separate, TEE-hosted trace source with its own client
integration; the shared protocol DTOs live in
`crates/trace-commons-protocol`.

The contributor's private-inference router is IronWire, a git dependency on
[nearai/ironwire](https://github.com/nearai/ironwire). It routes isolated
Claude Code and Codex subscription logins, each as itself; see
[multi-subscription routing](docs/multi-subscription-routing.md).

### Architecture

```
┌────────────────────┐    ┌────────────────────┐    ┌──────────────────┐
│  Ironclaw client   │    │  trace-commons-    │    │  NEAR AI Cloud   │
│  (separate repo)   │───▶│  ingest (this repo)│───▶│  (TEE-hosted vLLM│
│  local redaction   │    │                    │    │   scoring)       │
└────────────────────┘    │  ┌──────────────┐  │    └──────────────────┘
                          │  │ gate-enclave │  │
                          │  │  orchestrator│  │
                          │  └──────────────┘  │
                          │  ┌──────────────┐  │    ┌──────────────────┐
                          │  │  PostgreSQL  │◀─┼───▶│  Object storage  │
                          │  │  with RLS    │  │    │  (GCS, FS, local)│
                          │  └──────────────┘  │    └──────────────────┘
                          │  ┌──────────────┐  │    ┌──────────────────┐
                          │  │  credit      │──┼───▶│  NEAR receipt    │
                          │  │  settlement  │  │    │  outbox          │
                          │  └──────────────┘  │    └──────────────────┘
                          └────────────────────┘
```

The diagram shows Ironclaw as the client; the contributor CLI and the desktop
apps submit to the same ingest service.

Authoritative contracts to read before changing anything substantive:

- [`docs/trace-commons.md`](docs/trace-commons.md) — envelope contract and
  threat model
- [`docs/trace-commons-storage.md`](docs/trace-commons-storage.md) — storage
  contract
- [`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md) — phased
  open work and "Production Gap Queue"
- [`docs/contributor-daemon-ipc-v1_1.md`](docs/contributor-daemon-ipc-v1_1.md) —
  IPC contract between the contributor background daemon and the native
  menu-bar and window applications

### Repository Layout

```
crates/
├── trace-commons-protocol/      DTOs + redaction helpers shared with the client.
├── trace-commons-gate-api/      Public gate contracts plus the reference scorer.
├── trace-commons-gate-enclave/  Scoring orchestrator (perplexity, embedder, vector index).
│                                Two real perplexity backends: mistralrs (local CUDA,
│                                feature `local-gpu-models`) and NEAR AI Cloud HTTP
│                                (feature `near-ai-scorer`).
├── trace-commons-ingest/        Hosted ingest / review / admin / worker API
│                                (`trace-commons-ingest` binary; embed via
│                                `trace_commons_ingest::run`).
├── trace-commons-server/        Server library and the other hosted binaries.
│   └── src/bin/
│       ├── trace-commons-upload-claim-issuer    EdDSA/Ed25519 upload-claim issuer.
│       ├── trace-commons-gate-calibrate         Offline calibration + model bake-off.
│       ├── trace-commons-pilot-bootstrap        HF agent-traces load generator for pilot.
│       └── trace-commons-vector-replay          Vector-index replay tool.
├── trace-commons-contributor/   Contributor CLI and daemon: login, list, submit, status,
│                                whoami, logout, mint-grant. See its own README.
├── trace-commons-contributor-sources/  Session-store parsers and routing overlay,
│                                re-exported by the contributor at
│                                `trace_commons_contributor::{source, routing}`.
├── trace-commons-contributor-insights/  Private local Insights, re-exported at
│                                `trace_commons_contributor::insights`.
├── trace-commons-contributor-platform/  Shared OS credentials and local files.
├── trace-commons-contributor-ffi/  C ABI over the contributor, for the macOS and Windows apps.
├── trace-commons-contributor-gtk/  Linux GTK app (frozen; its own Cargo workspace).
├── ironwire_*/                  IronWire, the loopback inference proxy the daemon runs
│                                for Private AI (11 crates; MIT OR Apache-2.0, copyright
│                                NEAR AI; imported from nearai/ironwire). See docs/ironwire/.
└── trace-commons-client-server-tests/  Unpublished cross-contract test package;
                                 client-only tests do not build the server.

macos/                           Native SwiftUI macOS app, the lead client.
windows/                         Windows app, C# (frozen).
tauri-desktop/                   Tauri app (frozen; its own Cargo workspace).
migrations/                      PostgreSQL schema.
scripts/                         install.sh, install.ps1, contribute.sh; operator and CI scripts.
docs/
├── trace-commons.md, trace-commons-storage.md, trace-commons-roadmap.md  Authoritative contracts.
├── status.md                    Deployment status and reference notes.
├── release-runbook.md           Cutting a signed app or CLI release.
├── operator/                    Operator runbooks (per slice).
├── ironwire/                    IronWire design documents and working rules.
└── superpowers/                 Per-slice design specs + implementation plans.
.github/workflows/ci.yml         Server-side CI gates.
.github/workflows/clients.yml    Desktop app CI.
```

## Build and test

```bash
# Minimum: default-features build + tests (no GPU, no external scoring)
cargo check -p trace-commons-server --bins
cargo test  -p trace-commons-server
cargo check -p trace-commons-ingest --bins
cargo test  -p trace-commons-ingest

# With the NEAR AI scoring backend (pilot configuration)
cargo check -p trace-commons-server --bins --features near-ai-scorer
cargo check -p trace-commons-ingest --bins --features near-ai-scorer

# With local CUDA scoring (mistralrs; needs CUDA toolchain for the cuda subfeature)
cargo check -p trace-commons-server --bins --features local-gpu-models
```

CI applies `RUSTFLAGS=-D warnings` to every cargo invocation, so warnings
fail the build. To catch what CI catches before pushing:

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run
RUSTFLAGS='-D warnings' cargo check -p trace-commons-ingest --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-ingest --no-run
cargo clippy -p trace-commons-server -p trace-commons-ingest --all-targets -- \
  -D warnings \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

PostgreSQL integration tests require a live database; export
`TRACE_COMMONS_PG_TEST_DATABASE_URL` and run:

```bash
cargo test -p trace-commons-server --test trace_corpus_pg_store
```

[`CLAUDE.md`](CLAUDE.md) has the rest: how each crate's integration tests are
grouped into one `it` binary, the commands for client-only work, the shared
compile cache, the PostgreSQL setup for the ingest tests, and what each CI
job checks. The macOS app is a Swift package in `macos/`: build
`trace-commons-contributor-ffi` first, because the package links that dylib,
then run `swift test` there. The GTK and Tauri apps are separate Cargo
workspaces, so `-p` from the repository root does not reach them; `CLAUDE.md`
gives their manifest paths.

Releases are cut from tags.
[`docs/release-runbook.md`](docs/release-runbook.md) records the steps and
invariants.

### Run a Local Ingest Server

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2027-01-01T00:00:00Z' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run -p trace-commons-ingest --bin trace-commons-ingest
```

This runs the dev profile against the local-encrypted artifact store and
the in-process mock gate. Production-grade configuration (cloud KMS, real
gate scorer, central-issuer credit profile, RLS-pinned PostgreSQL role,
fresh rollout-smoke) is documented in
[`docs/operator/`](docs/operator/) — start there before touching any
deployment.

[`docs/operator/README.md`](docs/operator/README.md) is the index of the
operator runbooks.

## Contributor CLI

For AI agents, see [Configure the app with an AI agent](docs/agent-configuration.md)
and the [copyable sample prompt](docs/examples/agent-configuration-prompt.md).

[Install](#install) above has the install commands.
https://tracecommons.ai/install/ is the fuller guide, including how to verify
a download by hand.

On macOS or Linux, the installer script works out which build you need,
verifies it, and puts it in `~/.local/bin` — no `sudo`, nothing outside your
home directory. It installs the CLI under two names that run the same
program: `near-ai`, and `trace-commons-contributor`, which the examples below
use.

It refuses to install anything it cannot verify: the published checksum has to
match, and on macOS the signature has to be valid *and* name our Developer ID.
There is no flag to skip either check. `--dir <path>` (or `TC_INSTALL_DIR`)
picks a different destination, and `TC_VERSION=<x.y.z>` pins a release instead
of resolving the newest `contributor-v*` tag.

On Windows, `install.ps1` has the same verification policy, installs to
`%LOCALAPPDATA%\Programs\TraceCommons` and appends it to your user `PATH`, so
reopen the terminal afterwards.

Otherwise take a binary from the newest `contributor-v*` release on the
[releases page][releases]: macOS on both architectures and Windows on x86_64
are code-signed — Developer ID and notarized, Authenticode and
RFC3161-timestamped respectively — and the Linux x86_64 binary is not signed,
so use the published checksum beside it.

Confirm the install with:

```bash
trace-commons-contributor --version
```

Then contribute from the directory you want to cover. `submit` scopes itself
to the working directory's subtree: stand in one project to submit that
project, or in the parent of several repos to submit all of them. It refuses
to run from `$HOME` or a filesystem root, where the subtree would be every
session on the machine; `--all` says that deliberately, `--project <path>`
scopes somewhere you are not, and `--pick` brings back the per-session table.

```bash
cd ~/code/my-hackathon-project
trace-commons-contributor submit          # summarises the batch, asks y/N
```

For a single submission with nothing to install first, `scripts/contribute.sh`
fetches the verified binary into a cache directory, submits once, and exits —
no PATH entry, no daemon, nothing that autostarts. Reading it before running it
is encouraged, which is why the two-step form is first:

```bash
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons/main/scripts/contribute.sh -o contribute.sh
cd ~/code/my-hackathon-project
TRACE_COMMONS_INVITE='<your invite link>' sh ~/contribute.sh
```

It leaves one thing behind, deliberately: a **keep** — a `0700` state
directory holding the device key your account is minted from. That key is the
only way to sign in and withdraw the traces the run uploaded, so there is no
flag that suppresses it; the script prints where the keep is and how to delete
it on every run. Put the invite in `TRACE_COMMONS_INVITE` rather than on the
command line, where it would land in your shell history and in `ps`.

The desktop app ships as a universal notarized DMG
(`brew install --cask TraceCommons/tap/trace-commons`). New `app-v*` releases
are macOS only: the Windows and Linux apps are frozen on their existing
releases, and [`docs/release-runbook.md`](docs/release-runbook.md) names the
last release that carried them. The CLI above still ships for macOS, Linux
and Windows. https://docs.tracecommons.ai/cli/quickstart/ is the fuller
guide.

To build it from this checkout instead — necessary on any platform without a
published binary, Linux on arm64 for instance:

```bash
cargo build --release --bin trace-commons-contributor
./target/release/trace-commons-contributor login
```

`login` opens near.ai in your browser to sign in, then enrolls this device;
no invite is needed. With an invite, run `login --invite <link>` or set
`TRACE_COMMONS_INVITE`. `login --key-id` prints the device key id for an
instance-signed grant.

See [`crates/trace-commons-contributor/README.md`](crates/trace-commons-contributor/README.md)
for the full quickstart, consent model, and subcommand reference. Consent is
scope-based, not capped to a single default: the contributor requests
scopes at login (interactive prompt or `--scopes`), the server clamps the
request to the enrollment-stored instance-policy ceiling at claim-mint
time, and the resulting server-granted set — visible per-trace via
`status` — rides in the envelope. Retroactive updates to
consent on already-submitted traces are deferred to a future slice. See
[`docs/superpowers/specs/2026-07-08-consent-scope-broadening-design.md`](docs/superpowers/specs/2026-07-08-consent-scope-broadening-design.md)
for the full design.

### Uninstalling

Two things come off separately: the *local state* (device key, config,
receipts, daemon queue and history) and the *installed program*. Start with
the state, because `logout` also stops a running daemon before it wipes the
credentials that daemon uploads with:

```bash
trace-commons-contributor logout
```

Uninstalling is not withdrawal. Traces you already submitted stay on the
server; `daemon withdraw <submission-id>` — or `--all-quarantined` — is what
removes them, and it needs the account session, so do any withdrawing
*before* you log out.

`logout` empties the state directory but leaves the directory itself. Remove
it to finish the job:

| Platform | State directory |
| --- | --- |
| Linux | `~/.config/trace-commons` |
| macOS | `~/Library/Application Support/trace-commons` |
| Windows | `%LOCALAPPDATA%\trace-commons` (CLI and app share it) |
| Linux flatpak app | `~/.var/app/ai.tracecommons.Contributor/config/trace-commons` |

If `TRACE_COMMONS_CONTRIBUTOR_DIR` was set, that path wins over all of these.

Then remove the program itself, by however it was installed:

```bash
# CLI, scripts/install.sh (macOS, Linux)
rm ~/.local/bin/trace-commons-contributor    # or $TC_INSTALL_DIR, or --dir

# CLI, Homebrew
brew uninstall trace-commons-contributor

# CLI, winget
winget uninstall TraceCommons.Contributor

# Desktop app, Homebrew cask (macOS)
brew uninstall --cask trace-commons

# Desktop app, flatpak (Linux) -- --delete-data also removes ~/.var/app state
flatpak uninstall --delete-data ai.tracecommons.Contributor

# and, if you want the tap gone too
brew untap TraceCommons/tap
```

```powershell
# CLI, scripts/install.ps1
Remove-Item -Recurse "$env:LOCALAPPDATA\Programs\TraceCommons"
# install.ps1 appended that directory to your user PATH; take it back out
$p = [Environment]::GetEnvironmentVariable('Path','User') -split ';' |
  Where-Object { $_.TrimEnd('\') -ine "$env:LOCALAPPDATA\Programs\TraceCommons" }
[Environment]::SetEnvironmentVariable('Path', ($p -join ';'), 'User')

# Desktop app, MSIX / .appinstaller -- also ends the update subscription
Get-AppxPackage Iqlusion.TraceCommons | Remove-AppxPackage
```

Autostart registrations are the part an uninstall is easiest to leave
behind:

- **Linux, CLI daemon.** `systemctl --user disable --now
  trace-commons-contributor.service`, then `trace-commons-contributor daemon
  uninstall` to remove the unit file
  (`~/.config/systemd/user/trace-commons-contributor.service`).
- **macOS app.** Registered through `SMAppService`. Turn off "Run at login"
  in Settings before deleting the app, or clear the leftover entry in System
  Settings → General → Login Items.
- **Windows, MSIX app.** A packaged startup task; `Remove-AppxPackage` takes
  it with the package.
- **Windows, portable app.** The portable build uses the per-user Run key
  instead, which deleting the folder does not touch:
  `Remove-ItemProperty -Path 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run' -Name 'Trace Commons'`.

The macOS app itself is `Near.ai.app`, shown as Near AI Gateway (`TraceCommons.app` if it was installed
before the rename; bundle id `ai.tracecommons.shell`) in `/Applications` when installed from the DMG rather than the cask; quit it,
then move it to the Trash. The Windows portable build is just the unzipped
folder — delete it.

### Contributing traces from other harnesses

The CLI reads Claude Code, Codex, Gemini CLI, Cline and OpenCode sessions
natively, straight from their local stores. Nothing below is needed for those.

For any other harness [Letta Trajectory](https://github.com/letta-ai/trajectory)
covers -- `atif`, `copilot-cli`, `cursor`, `droid`, `hermes`, `letta-code`,
`omp`, `openclaw`, `openhands`, `pi` -- export the sessions first:

```bash
cd ~/code/my-project
npx @tracecommons/trajectory-export --all
trace-commons-contributor submit
```

The exporter writes `<source>-<id>.trajectory.json` into the working
directory, which is the name `submit` looks for, so there is no flag to pass
between the two commands.

Some of those harnesses cannot be listed from disk -- upstream can normalize
their transcripts but not enumerate their stores. The exporter names them and
takes a file instead:

```bash
npx @tracecommons/trajectory-export --source cursor --input session.json
```

**Do not run `npx @letta-ai/trajectory`.** Earlier versions of this file told
you to; it has never worked. That package is a library with no `bin`, so npx
exits with "could not determine executable to run". `@tracecommons/trajectory-export`
is the command that instruction assumed, and it wraps the same library.

#### What `submit` will and will not pick up

Trajectory files are discovered in exactly two places: the working directory,
where the name must end `.trajectory.json` or `.trajectory.jsonl`, and
`<state-dir>/trajectories/`, where any `*.json` or `*.jsonl` counts. Nothing
else is scanned -- never `$HOME`, never recursively.

The suffix is what keeps an unrelated `session.json` out of a submission.
Putting a file in the staging directory is itself the opt-in, so it needs no
suffix.

`--trajectory <path>` still names a file or directory explicitly, and still
treats a path that does not exist as an error rather than an empty result.

Model reasoning is captured by default and redacted client-side like any
other content. Pass `--no-reasoning` to exclude it from a submission.

## Contributing

**By opening a pull request you license your contribution under
`MIT OR Apache-2.0`**, including contributions to the AGPL-licensed server
crates. You keep your copyright. Read [`CONTRIBUTING.md`](CONTRIBUTING.md)
before your first PR — it explains why the inbound license differs from the
outbound one and what that means for you.

Branch protection on `main` requires:

- **Thirteen** required status checks green, and they must be green on a
  branch that is up to date with `main`:

  | | |
  |---|---|
  | `cargo fmt --check` | `cargo clippy` |
  | `cargo check (default features)` | `cargo test (default features)` |
  | `cargo check (near-ai-scorer)` | `pilot-bootstrap smoke` |
  | `cargo check (local-gpu-models, non-CUDA)` | `database suites against a real PostgreSQL` |
  | `cargo check (permissive crates, standalone)` | `builds at the declared MSRV floor` |
  | `trace-commons-ingest tests, whole bin, against PostgreSQL` | `staging channel tests` |
  | `wallet SDK and production CSP` | |

  `.github/workflows/ci.yml` holds more jobs than this (twenty-three as of
  2026-10-30); the others run on every PR but do not block the merge. The
  client shells -- the macOS app, the Windows app, installer, named-pipe ACL
  and update conformance, and the Linux GTK shell -- live in
  `.github/workflows/clients.yml` since 2026-09-22. They run on every push to
  `main`, on demand, and on a pull request only when it touches a client
  path, and none of them is required: a client regression shows up on
  `main` after the merge rather than blocking it.
- A pull request (no direct pushes), with one approving review from a code
  owner listed in [`.github/CODEOWNERS`](.github/CODEOWNERS).
- Linear history (squash or rebase, no merge commits).
- Any review conversations resolved before merge.

`main` is behind a **merge queue** (`main merge queue`, active). A PR merges
by entering the queue, not by a direct click, and the queue re-runs the
required checks against `main` as it is at that moment. A queued PR that
never receives its required checks times out of the queue rather than
merging.

### Conventions worth knowing

- **Hash-only audit and logging.** Audit rows, error logs, and operational
  surfaces are hash-only or label-only. Raw URLs, bearer tokens, ARNs,
  account references, transaction hashes, contributor identity, and trace
  bodies must never appear in stored rows or log strings.
- **Fail-closed by default.** When a required gate is configured but its
  dependency is missing, refuse the path with a safe missing-control name.
  Never silently fall back to plaintext or a less-restricted backend.
- **Tenant scoping.** Every read/write is driven by auth-derived tenant +
  actor context. Envelope tenant fields are attribution only.
- **PostgreSQL RLS is forced** on every Trace Commons table; tenant
  predicates go through `trace_current_tenant_id()`.
- **Commit style.** Short imperative subjects (no `feat:` / `fix:` prefixes
  — match the existing log). No emojis in commits, PRs, code, or reports.

A more complete style + workflow note for AI-assisted development is in
[`CLAUDE.md`](CLAUDE.md); the conventions there are the same ones humans
follow.

### Where to look for what

- Roadmap and pilot blockers: [`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md)
- Deployment status and reference notes: [`docs/status.md`](docs/status.md)
- Envelope + threat model: [`docs/trace-commons.md`](docs/trace-commons.md)
- Storage contract: [`docs/trace-commons-storage.md`](docs/trace-commons-storage.md)
- Per-slice design specs: [`docs/superpowers/specs/`](docs/superpowers/specs/)
- Per-slice implementation plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)
- Operator runbooks: [`docs/operator/README.md`](docs/operator/README.md)
- Cutting a release: [`docs/release-runbook.md`](docs/release-runbook.md)
- Contributor CLI: [`crates/trace-commons-contributor/README.md`](crates/trace-commons-contributor/README.md)

## License

TraceCommons is licensed in two parts, split along the client/server seam.

**Server components — AGPL-3.0-or-later** (`LICENSE-AGPL`):
`trace-commons-server`, `trace-commons-gate-api`, `trace-commons-gate-enclave`,
`trace-commons-ingest`, and the unpublished test package
`trace-commons-ingest-contributor-tests`.

These are normally operated as a network service, so section 13 applies: run a
modified version and let others reach it over a network, and you owe those
users the Corresponding Source. `trace-commons-ingest` answers `GET /v1/source`
with the license, the source URL, and the commit the running binary was built
from. If you deploy a modified build, point that constant at your own source.

**Client and protocol components — MIT OR Apache-2.0** (`LICENSE-MIT`,
`LICENSE-APACHE`): `trace-commons-protocol`, `trace-commons-contributor`,
`trace-commons-contributor-sources`, `trace-commons-contributor-insights`,
`trace-commons-contributor-platform`, `trace-commons-contributor-ffi`,
`trace-commons-contributor-gtk`, `trace-commons-operator-client`,
`trace-commons-mark`, `trace-commons-build-info`, and the unpublished
`trace-commons-client-server-tests` test package.

These stay permissive on purpose. The contributor CLI, the desktop apps, and
the envelope protocol are meant to be embedded in proprietary agent harnesses —
Ironclaw consumes `trace-commons-protocol` directly.

**Contributions are licensed inbound under MIT OR Apache-2.0**, including
contributions to the AGPL crates. Contributors keep their copyright; nothing is
assigned, and there is no CLA to sign. This deliberately departs from
"inbound = outbound" so the project retains the ability to relicense what it
distributes — what downstream recipients get is unchanged, since the server
crates ship AGPL either way. See [`CONTRIBUTING.md`](CONTRIBUTING.md).

Permissive code may flow into the AGPL crates; the reverse is a license
violation no compiler will report, so it is enforced by
`crates/trace-commons-server/tests/license_boundary.rs`. Adding an AGPL crate
to a client to reuse one trait will fail that test. See `LICENSE` for the full
statement and `deny.toml` for the dependency-license audit
(`cargo deny check licenses`).
