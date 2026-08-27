# TraceCommons

**A user-owned register of AI agent work.**

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

This repository — `trace-commons-server` — is the hosted control plane:
ingest, review, retention, revocation, encrypted artifact storage,
upload-claim issuing, audit chain, and credit settlement. It also ships
`trace-commons-contributor`, a standalone CLI contributors run on their own
machine to discover, locally redact, and submit Claude Code / Codex
session traces. Ironclaw is a separate, TEE-hosted trace source with its
own client integration; the shared protocol DTOs live in
`crates/trace-commons-protocol`.

## Status: Pilot Deployment

TraceCommons is **in pilot deployment as of May 2026**. What that means
concretely:

| Component | State |
|---|---|
| Hosted server (this repo) | Phase A code-complete, smoke-validated, deployable. |
| Scoring backend | **NEAR AI Cloud** (TEE-hosted vLLM, Intel TDX + NVIDIA GPU TEE) — chosen so a pilot host needs no local CUDA stack. Smoke-validated against `Qwen3.6-35B-A3B-FP8`. |
| Gate floors | Recalibration against the hosted model is required before first contributor traffic — see [`docs/operator/a27-perplexity-floor-calibration.md`](docs/operator/a27-perplexity-floor-calibration.md). |
| Contributor gate | Invite-code allowlist on the upload-claim issuer; off by default, enabled for the pilot — see [`docs/operator/pilot-allowlist.md`](docs/operator/pilot-allowlist.md). |
| KMS / KEK | Cloud KMS (GCP first) with envelope-encrypted per-object DEKs. Phase A trust boundary. |
| TEE trust upgrade | Phase B — move the gate service into an attested dstack enclave once dstack-GPU primitives stabilize. The current KEK boundary is honestly weaker than the Phase B target; this is documented, not papered over. |
| Contributor client | `trace-commons-contributor` (this repo) is available for direct human contributors: `login`, `submit`, `status`, `whoami`, `logout`, `mint-grant`. Ironclaw integration (a TEE-hosted trace source) is separate, ongoing work. The `trace-commons-pilot-bootstrap` binary stands in as a load-generation harness against real HF agent-traces sessions so calibration and end-to-end validation can proceed without either. |
| Credits | Settlement, hash-only attestation pipeline, central-issuer ABAC, NEAR receipt outbox — all in. Credit-bearing routes are gated by a central-issuer principal allowlist. |

Pilot intentionally **scopes down** from the original design: regular GPU
hardware (in NEAR AI's TEE) instead of an attested local enclave; cloud
KMS as the KEK; a single calibrated model rather than per-tenant selection.
Phase B narrows the trust gap; Phase A proves the path with operators.

The full phasing and open work queue lives in
[`docs/trace-commons-roadmap.md`](docs/trace-commons-roadmap.md). Per-slice
design specs and implementation plans live under
[`docs/superpowers/`](docs/superpowers/).

## Architecture

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

## Repository Layout

```
crates/
├── trace-commons-protocol/      DTOs + redaction helpers shared with the client.
├── trace-commons-gate-api/      Public gate contracts plus the reference scorer.
├── trace-commons-gate-enclave/  Scoring orchestrator (perplexity, embedder, vector index).
│                                Two real perplexity backends: mistralrs (local CUDA,
│                                feature `local-gpu-models`) and NEAR AI Cloud HTTP
│                                (feature `near-ai-scorer`).
├── trace-commons-contributor/   Contributor-facing CLI: login, list, submit, status,
│                                whoami, logout, mint-grant. See its own README.
└── trace-commons-server/        All hosted binaries.
    └── src/bin/
        ├── trace-commons-ingest                 Hosted ingest / review / admin / worker API.
        ├── trace-commons-upload-claim-issuer    EdDSA/Ed25519 upload-claim issuer.
        ├── trace-commons-gate-calibrate         Offline calibration + model bake-off.
        ├── trace-commons-pilot-bootstrap        HF agent-traces load generator for pilot.
        └── trace-commons-vector-replay          Vector-index replay tool.

migrations/                      PostgreSQL schema.
docs/
├── trace-commons.md, trace-commons-storage.md, trace-commons-roadmap.md  Authoritative contracts.
├── operator/                    Operator runbooks (per slice).
└── superpowers/                 Per-slice design specs + implementation plans.
.github/workflows/ci.yml         CI gates.
```

## Getting Started

### Build + Test

```bash
# Minimum: default-features build + tests (no GPU, no external scoring)
cargo check -p trace-commons-server --bins
cargo test  -p trace-commons-server

# With the NEAR AI scoring backend (pilot configuration)
cargo check -p trace-commons-server --bins --features near-ai-scorer

# With local CUDA scoring (mistralrs; needs CUDA toolchain for the cuda subfeature)
cargo check -p trace-commons-server --bins --features local-gpu-models
```

CI applies `RUSTFLAGS=-D warnings` to every cargo invocation, so warnings
fail the build. To catch what CI catches before pushing:

```bash
RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins
RUSTFLAGS='-D warnings' cargo test  -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
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

### Contributor CLI

Signed binaries are published per release, so contributors do not need this
repository or a Rust toolchain. https://tracecommons.ai/install/ is the fuller
guide, including how to verify a download by hand; the short form follows.

On macOS or Linux, the installer script works out which build you need,
verifies it, and puts it in `~/.local/bin` — no `sudo`, nothing outside your
home directory:

```bash
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/install.sh -o install.sh
sh install.sh
```

It refuses to install anything it cannot verify: the published checksum has to
match, and on macOS the signature has to be valid *and* name our Developer ID.
There is no flag to skip either check. `--dir <path>` (or `TC_INSTALL_DIR`)
picks a different destination, and `TC_VERSION=<x.y.z>` pins a release instead
of resolving the newest `contributor-v*` tag.

On macOS, from the Homebrew tap instead:

```bash
brew tap TraceCommons/tap
brew trust tracecommons/tap          # Homebrew refuses untrusted third-party taps
brew install trace-commons-contributor
```

On Windows, in PowerShell — same verification policy, installs to
`%LOCALAPPDATA%\Programs\TraceCommons` and appends it to your user `PATH`, so
reopen the terminal afterwards:

```powershell
irm https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/install.ps1 -OutFile install.ps1
.\install.ps1
```

Otherwise take a binary from the [current CLI release][releases]: macOS on both
architectures and Windows on x86_64 are code-signed — Developer ID and notarized,
Authenticode and RFC3161-timestamped respectively — and the Linux x86_64 binary
is not signed, so use the published checksum beside it. Follow that link rather
than GitHub's "latest release": CLI releases are tagged `contributor-v*` and
desktop-app releases `app-v*`, so "latest" is whichever stream was cut most
recently — today `contributor-v0.4.6` and `app-v0.4.6` were cut minutes apart, and
an app tag carries no CLI binary.

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
curl -fsSL https://raw.githubusercontent.com/TraceCommons/trace-commons-server/main/scripts/contribute.sh -o contribute.sh
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
(`brew install --cask trace-commons`), as a GPG-signed flatpak, and on Windows
as a self-contained Authenticode-signed zip — unpack it and run
`TraceCommons.exe`; the .NET runtime and Windows App SDK are inside, so there is
nothing to install first. https://docs.tracecommons.ai/cli/quickstart/ is the
fuller guide.

To build it from this checkout instead — necessary on any platform without a
published binary, Linux on arm64 for instance:

```bash
cargo build --release --bin trace-commons-contributor
./target/release/trace-commons-contributor login
```

[releases]: https://github.com/TraceCommons/trace-commons-server/releases/tag/contributor-v0.4.6

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

#### Uninstalling

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

The macOS app itself is `TraceCommons.app` (bundle id `ai.tracecommons.shell`)
in `/Applications` when installed from the DMG rather than the cask; quit it,
then move it to the Trash. The Windows portable build is just the unzipped
folder — delete it.

#### Contributing sessions from other harnesses

The CLI reads Claude Code, Codex and Gemini CLI sessions natively, straight
from their local stores. Nothing below is needed for those.

For any other harness [Letta Trajectory](https://github.com/letta-ai/trajectory)
covers -- `atif`, `copilot-cli`, `cursor`, `droid`, `hermes`, `letta-code`,
`omp`, `openclaw`, `opencode`, `openhands`, `pi` -- export the sessions first:

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

##### What `submit` will and will not pick up

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

### Run a Local Ingest Server

```bash
TRACE_COMMONS_TENANT_TOKENS='tenant-a:dev-token-a;expires_at=2027-01-01T00:00:00Z' \
TRACE_COMMONS_BIND='127.0.0.1:3907' \
cargo run --bin trace-commons-ingest
```

This runs the dev profile against the local-encrypted artifact store and
the in-process mock gate. Production-grade configuration (cloud KMS, real
gate scorer, central-issuer credit profile, RLS-pinned PostgreSQL role,
fresh rollout-smoke) is documented in
[`docs/operator/`](docs/operator/) — start there before touching any
deployment.

## Contributing

Branch protection on `main` requires:

- All CI checks green (`cargo fmt --check`, three `cargo check` variants,
  `cargo clippy`, `cargo test`, `pilot-bootstrap-smoke`).
- A pull request (no direct pushes).
- Linear history (squash or rebase, no merge commits).
- Any review conversations resolved before merge.

Self-merge is permitted; reviewer approval is not currently required (the
project is still small). When this changes, the requirement will land here
and in the GitHub branch protection settings simultaneously.

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
- Envelope + threat model: [`docs/trace-commons.md`](docs/trace-commons.md)
- Storage contract: [`docs/trace-commons-storage.md`](docs/trace-commons-storage.md)
- Per-slice design specs: [`docs/superpowers/specs/`](docs/superpowers/specs/)
- Per-slice implementation plans: [`docs/superpowers/plans/`](docs/superpowers/plans/)
- Operator runbooks: [`docs/operator/`](docs/operator/)
- Contributor CLI: [`crates/trace-commons-contributor/README.md`](crates/trace-commons-contributor/README.md)

## Public Reference Notes

The Trace Credits, ranking-evidence, and external-adapter surface areas are
each large enough to warrant their own documents. Until they're broken out,
they live inline in `docs/`:

- **Trace Credits** — settlement model, central-issuer profile, NEAR outbox.
  See `docs/operator/calibration.md` and the credit-settlement specs under
  `docs/superpowers/specs/`.
- **Ranking evidence** — calibration registry, label-source authority, model
  promotion. See ranking-related specs under `docs/superpowers/specs/`.
- **External adapters** — benchmark/process evaluators, NEAR credit
  submit/confirm. All adapters are operator-owned; the server only records
  `configured` / `not_configured` readiness fields in
  `/v1/admin/config-status`.
