# Holonear compute contribution in Trace Commons

Date: 2026-09-04. Status: implementation started in isolated worktrees.

### First implementation slice

Branches: `feat/holonear-compute` (Trace Commons, base `420288f9`) and
`feat/trace-commons-compute` (Holonear, base `e366e5c8`).

- Added independent compute preferences in `trace-commons-contributor::compute`.
  Missing settings are disabled; granted consent restores paused; malformed or
  unsupported settings are refused. Atomic writes use the existing private store.
- Reserved `<contributor-state>/compute/worker` for the worker's HOLONEAR_HOME.
  This module does not read enrollment, upload credentials, or local traces.
- Added instance-scoped launches and explicit process-stop outcomes in Holonear's
  desktop core, including retaining process ownership through stop cancellation.
- Worker IPC authentication/versioning, exclusive process ownership, acknowledged
  coordinator drain, the controller/FFI, and UI remain unfinished. No worker is
  launched by the new Trace Commons module. No pool has been selected/contacted.

Validation: five compute-settings tests, four license-boundary tests, contributor
Clippy with warnings denied and standard repository allowances, and formatting
passed. No dependencies were added. Paired Holonear validation is recorded in
`docs/trace-commons-compute-worker-contract.md`. The original phase acceptance
gates below remain open until their complete criteria are met.

### Controller/FFI foundation

The shared controller serializes strict enable/resume/pause/disable commands,
restores granted consent as paused, and provides state, capability flags, and
shared shell wording. Four independent C exports open/read/command/free the
controller without trace enrollment. Both C headers describe the same surface.

This build deliberately has no worker backend: enable/resume return a visible
unavailable result, and refused enable does not save consent. Disable persists
revocation atomically; write failure leaves previous consent visible. No worker
process, pool request, unauthenticated IPC, or Holonear dependency is introduced.
Commands perform synchronous settings I/O and belong on a background thread;
the handle must not be freed concurrently with other calls.

Live lifecycle states are reserved in the schema but not emitted yet. The
authenticated supervisor adapter, telemetry freshness, serialized async launch
and drain, cross-process ownership, bounded worker deadlines, and real worker
verification remain required before the pilot capability can become available.

### Explicit local adapter slice

The subsequent native adapter implements the pinned authenticated IPC contract,
exclusive process ownership, asynchronous readiness/status/drain, telemetry
freshness, durable priority stop intent, and terminal shutdown. The normal app
constructor stays unavailable. Only an explicit Unix debug development
constructor and local harness can launch a hash-checked worker against a
loopback coordinator. See `docs/compute-local-adapter.md` for the contract pin,
dependency decision, invocation, lifecycle guarantees, and remaining release
gates. A local adapter is not authorization to enable production compute.

## Outcome and scope

Trace Commons users can independently opt into contributing compute to Holonear,
see what their machine is doing, and stop participation reliably. Compute consent
does not enable trace contribution, authorize reading local traces, or authorize
training on the contributor's own data. Compute earnings remain distinct from
Trace Credits.

The first deliverable is an Apple Silicon macOS test-pool pilot. Model consumption,
training-data export, hosted scoring changes, automatic wallet migration, and
production pool deployment are outside this first milestone.

This plan spans the Holonear repository and TraceCommons/trace-commons. Both
implementations use dedicated worktrees; the original checkouts are preserved.

## Starting evidence

- `apps/holonear-desktop/core` is a UI-independent Rust supervisor, configuration,
  status, and account library. It is currently `publish = false` and uses workspace
  dependencies; consuming it externally needs an intentional release strategy.
- Its supervisor launches `holonear node run`, reads loopback NDJSON status, and
  sends SIGTERM on Unix before a bounded wait and forced termination. It currently
  cannot report an acknowledged drain separately from a forced stop. Windows'
  graceful termination branch currently does nothing.
- `NodeConfig.free_mem_gb` advertises capacity to the scheduler. It is not evidence
  of an enforced process memory ceiling.
- `holonear-home` supports `HOLONEAR_HOME`; current desktop configuration helpers
  otherwise resolve paths from process-global environment and share standalone
  Holonear state. Some artifact/cache paths live outside that root and need audit.
- Trace Commons has a shared contributor Rust core, a C ABI for macOS/Windows,
  and a GTK shell consuming Rust directly. Contributor-side code is permissively
  licensed and must not depend on the AGPL server/gate crates.

These are source observations from the planning pass, not release-readiness claims.

## Proposed product defaults

- Compute starts disabled, including after installation or update.
- Add a visible Compute destination alongside trace contribution. Its introduction
  explains resource use and independent consent before showing Enable.
- First-run flow: eligibility -> resource preferences -> test-pool explanation ->
  explicit enable -> live activity. No paid-earnings promise in the test pilot.
- Closing the window leaves an already-enabled menu-bar app working. Explicit Quit
  drains and stops its worker. Launch-at-login and unattended restart are deferred.
- Manual Pause remains paused across app restart. After a crash or OS restart,
  recover stale ownership safely and require Resume for the pilot.
- Default to AC power. Battery transition, serious thermal pressure, or sustained
  memory pressure initiates drain and shows the pause reason. Resume is manual in
  the pilot to avoid oscillation. Do not keep the system awake implicitly.
- Show RAM as a scheduling allowance. Measure actual worker footprint separately.
  Do not expose an "idle only" option until its detection and behavior exist.
- Public release needs disk/cache bounds and a clear bandwidth policy; basic RAM
  controls alone are insufficient for an unattended contributor experience.

## Architecture and ownership

Native shells call a shared Trace Commons compute controller. That controller
owns one worker process and exposes typed settings, state, and commands through
the existing Rust/FFI pattern. Compute has a separate lifecycle from trace upload;
an upload pause, logout, or submission error cannot implicitly start/stop compute.

Preferred reuse: extract or harden a small provider-supervision library in Holonear,
then consume a pinned revision from Trace Commons. Keep account dependencies out
of the initial controller where possible. Confirm repository dependency approval
requirements at implementation time. Do not pull the ML engine into the shell.

Ship an approved, version-pinned `holonear` binary with the supported app package.
Production launches use an exact verified path, not PATH or an environment override.
Signing/repackaging must preserve the required worker attestation identity; verify
the final distributed artifact against pool policy, not only the build output.

Use an absolute Trace Commons-owned compute directory and pass `HOLONEAR_HOME`
only to the worker process. Make library paths explicit; never mutate the host
app's process-global environment to select an instance. Keep identity, config,
cache, IPC, and locks separate from `~/.holonear`. Detect another provider using
the machine and refuse a conflicting start; do not kill or adopt it automatically.

Separate processes and directories do not establish a filesystem security boundary.
Audit worker behavior and inherited environment, descriptors, working directory,
logs, and capabilities. Prototype OS restrictions compatible with Metal, networking,
and attestation. Public release must document and validate the actual boundary:
no Trace Commons trace directories, upload credentials, or wallet secrets are passed
to the worker, and no arbitrary workload/file-access path is enabled by this feature.

## Implementation sequence

### 1. Freeze the worker contract and pilot environment

- [ ] Pin a Holonear revision and inspect node startup, onboarding, actual RAM
  behavior, shutdown acknowledgements, IPC, cache locations, and update behavior.
- [ ] Select a controlled test pool and representative training/serving workload;
  record eligibility, trust mode, resource needs, and payout requirements. A local
  coordinator is sufficient initially. Use non-sensitive test workloads.
- [ ] Define a versioned contract for capabilities, start parameters, status,
  stop/drain results, stale telemetry, and incompatible versions. Prefer additive
  changes compatible with the standalone Holonear app.
- [ ] Specify states: disabled, unavailable, starting, waiting, training, serving,
  draining, paused, error. Keep user intent, worker state, telemetry freshness,
  and pool admission separate; a live process is not proof it joined the pool.
- [ ] Decide process ownership and authenticated local IPC. Loopback binding alone
  is not sufficient authentication. Bound message sizes and reconnect attempts.

Exit: a reviewed contract and a reproducible real-node test that joins, performs
work, and proves drain completion. Unknown trust/pool constraints are recorded.

### 2. Harden reusable Holonear supervision

- [ ] Add explicit instance paths, child-only environment configuration, exclusive
  ownership, and process identity checks stronger than a reused PID/name match.
- [ ] Add a version/capability handshake, bounded status parsing, stale-state
  handling, startup timeout, exit classification, and bounded retry policy.
- [ ] Return distinct outcomes for graceful drain, forced termination, and failure.
  Show a draining state until completion; never guarantee penalty-free shutdown
  after a crash, forced termination, network failure, or unacknowledged drain.
- [ ] Ensure crashes and repeated launches cannot create duplicate workers or
  leave an unbounded orphan; define cleanup for a crashed controller.
- [ ] Prevent independent worker self-update from bypassing the app's pinned
  compatibility and attestation policy. Inventory all caches and log destinations.

Exit: lifecycle tests cover concurrent starts, stale/reused PID, malformed and lost
status, worker crash, controller crash, drain timeout, and incompatible versions.
The standalone Holonear desktop app still works with the updated library.

### 3. Add the Trace Commons compute controller and FFI

- [ ] Add a focused `compute` module under `trace-commons-contributor`, with
  persisted consent/settings, typed state, and enable/pause/resume/status commands.
- [ ] Serialize state-changing commands and keep process/network work off UI
  threads. Keep worker ownership independent of individual view lifetimes.
- [ ] Persist settings atomically and reject invalid configuration visibly.
  Compute must work without enabling trace upload or discovering trace files.
- [ ] Add shared user-facing status/reason copy following existing shell rules.
  Extend the C ABI and both identical headers, plus Rust/Swift bridge coverage.
- [ ] Preserve existing license boundaries and keep heavy backend dependencies
  inside the worker artifact.

Exit: controller and FFI tests prove independent consent, persistent pause,
idempotent commands, safe errors, and continued trace operation when compute fails.

### 4. Deliver the macOS test-pool experience

- [ ] Add the Compute destination and first-run journey; display eligibility,
  scheduling allowance, activity, measured resource use, and pause/error reasons.
- [ ] Wire AC/battery, thermal, memory-pressure, sleep/wake, window close, and Quit
  behavior. Unexpected sleep/disconnect is recovery, not a claimed graceful drain.
- [ ] Package the MLX-capable worker and required assets using the existing app
  signing/notarization workflow; verify it launches from the installed app.
- [ ] Run real training/serving jobs with trace contribution enabled and disabled;
  measure memory, responsiveness, thermals, bandwidth, and disk growth.

Exit: a signed installable pilot starts explicitly, joins the pool, performs real
work, shows fresh state, pauses with an observed drain result, and recovers without
duplicate workers. Battery, pressure, sleep, restart, and uninstall cases have
recorded evidence. No production rollout is implied by passing this gate.

### 5. Add account linkage, earnings, and release controls

- [ ] Select the production account journey: wallet connection or separately
  scoped Holonear account. Reuse public account identifiers only with explicit
  linkage; never reuse Trace Commons authentication tokens or transfer full-access
  wallet secrets to the worker.
- [ ] Provide OS-backed key custody if local signing is needed. Handle unfunded
  implicit accounts, expired sessions, and unavailable earnings honestly.
- [ ] Display accrued, pending, and settled earnings separately, with freshness
  and provenance. Do not display genesis points or Trace Credits as cash earnings.
- [ ] Validate resource bounds and cache cleanup, secure IPC, trust-mode wording,
  signed updates/rollback, and attestation acceptance of the shipped binary.
- [ ] Add a staged release flag that cannot enable compute without user consent.
  Document disabling new participation and safely draining existing workers.

Exit: verified account attribution and real settlement evidence in the appropriate
test environment, plus a reviewed release checklist. Mainnet transactions and
production deployment remain separate explicitly authorized operations.

### 6. Extend platform support

- [ ] GTK: reuse the controller, add native power/lifecycle integration, package
  supported worker builds, and test on supported real hardware.
- [ ] Windows: implement and verify cooperative drain IPC before enabling compute;
  a timeout followed by process kill is not equivalent. Cover process ownership,
  named-pipe ACLs where used, signing, install/update, sleep, and logoff.
- [ ] Build a capability matrix from actual shipped backends and pool acceptance.
  Unsupported hardware gets an explanation; no silent CPU fallback presented as
  GPU contribution. Shared UI does not establish runtime platform support.

Exit: each enabled platform passes the same lifecycle/privacy/consent suite plus
its native IPC, packaging, and real-workload checks.

## PR boundaries and verification

Suggested order: Holonear worker contract -> Holonear supervisor hardening ->
Trace Commons controller/FFI -> macOS UX/package -> earnings/release controls ->
GTK and Windows. Keep runnable intermediate states; do not enable a shell feature
before its required worker capability ships. Steps 1-4 constitute the pilot.

Use focused process/contract tests and real worker integration tests for the
lifecycle guarantees. Run Trace Commons' warnings-as-errors checks, relevant
contributor/FFI suites, header parity, Swift tests, and license-boundary checks;
run dependency/license checks when dependencies change. Follow each repository's
current required CI rather than treating this list as a substitute.

Record real-device evidence for graceful drain, resource pressure, sleep/wake,
network loss, upgrade/rollback, and coexistence with standalone Holonear. Logs and
test artifacts must not contain trace bodies, wallet secrets, or upload tokens.

## Decisions to close during step 1

1. Which pinned worker and test pool support the minimum useful workload?
2. Can existing worker IPC prove drain completion and enforce instance ownership,
   or are additive protocol changes required?
3. Which OS resource/access restrictions preserve MLX and attestation behavior?
4. What measured RAM/disk/bandwidth defaults fit a background desktop experience?
5. Can the reusable supervisor be released as a small independent dependency,
   and how will worker and controller compatibility be maintained?

These questions do not block writing or reviewing the plan. Resolve them with
source inspection and local experiments before committing to the pilot release.
