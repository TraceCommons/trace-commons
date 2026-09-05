# Compute pilot: packaged worker and resource policy

Status: implementation design, not a release approval. 2026-09-04.
Base: Trace Commons `bd523c22`, paired Holonear `cef95b36`.
Companion: [implementation sequence](../plans/2026-09-04-compute-pilot-packaging-policy.md).

## Scope and source findings

Deliver a verifiable Apple Silicon worker package and conservative resource
policy without opening the shipping constructor. Keep trace contribution and
its universal arm64/x86_64 app unchanged. No production pool, signing credentials,
wallet linkage, paid earnings, or automatic participation is authorized here.

- `macos/scripts/make-app-bundle.sh` builds a universal app and FFI library.
- `macos/scripts/make-release-dmg.sh` signs nested code before the app, preserves
  Sparkle entitlements, then notarizes. Its source comments are not evidence of
  a successful release run.
- Holonear `apps/holonear-desktop/scripts/prepare-sidecar.sh` builds the actual
  `holonear-cli --bin holonear` with `mlx` on arm64; it defaults the deployment
  target to macOS 15.0. Do not substitute the separately published `holonear-node`
  binary without proving this controller's CLI and IPC compatibility.
- `compute/process.rs` only supports a debug local binary/hash/loopback config.
  `compute/live.rs` owns the child and durable stop intent. Neither currently
  implements packaged artifact authorization or resource policy.

## Artifact contract

Keep the worker out of the contributor dependency graph. Build in Holonear's
pinned toolchain and locked dependency tree; consume an explicit local staging
artifact. No build-time download of latest, PATH lookup, runtime executable
download, or worker self-update. The app's existing updater owns replacement.

Proposed bundle locations:

- `Contents/Helpers/holonear`: the arm64 MLX CLI helper.
- `Contents/Resources/Compute/worker-manifest.json`: sealed contract metadata.
- `Contents/Resources/Compute/assets/`: non-executable runtime assets only.
- `Contents/Frameworks/`: any required non-system native libraries, explicitly
  inventoried and individually signed; no loose executable code in Resources.

Manifest v1 is strict, size-bounded JSON with no arbitrary executable path. It
contains schema version, full source revision, build target, backend `mlx`,
minimum macOS, IPC version `0`, app/worker compatibility identifier, signing
identifier/team requirement, post-sign worker SHA-256, and asset entries
(`relative_path`, `size_bytes`, `sha256`). Reject unknown schema/fields, duplicate
paths, traversal, symlink escapes, missing assets, wrong types and oversized lists.
The runtime's trusted signing requirement is compiled policy, not authority
chosen by an unverified manifest. Reject CPU-only or wrong-architecture workers.

Inventory MLX/Metal resources and Mach-O load commands from an actual release
build before freezing the asset list. Reject build-machine/Homebrew paths and
unresolved libraries. Do not guess that a standalone binary is self-contained.
Model weights are separate from executable assets: the pilot must choose a
licensed, digest-pinned workload and a bounded download/cache policy before
network execution is enabled. No arbitrary coordinator-supplied executable assets.

The universal app may include the arm64-only helper, but Compute must remain
unavailable on Intel and under an unsupported process architecture. Do not weaken
the existing universal checks for the app or FFI. Verify the final worker's actual
minimum OS load command, not merely an environment variable.

## Signing and launch verification

1. Stage the helper, libraries and assets; finish all load-path rewriting.
2. Sign nested libraries and helper with the approved Developer ID requirement
   and hardened runtime. Preserve the existing Sparkle signing sequence.
3. Hash the final signed helper and assets, then write the manifest.
4. Sign the outer app, verify it, package/notarize/staple through the existing
   release path. Do not mutate sealed contents afterward.

The order follows Apple's [nested-code signing guidance](https://developer.apple.com/library/archive/documentation/Security/Conceptual/CodeSigningGuide/Procedures/Procedures.html).
Signing changes executable bytes; a pre-sign hash cannot be the runtime pin.
Use explicit signing targets rather than recursive re-signing with `--deep`.

Before each launch, resolve only within the current application bundle; validate
the app and helper signatures against compiled requirements, bounded manifest,
architecture/OS/backend compatibility and hashes. Missing control means refusal.
Perform verification off the UI thread. The shared controller consumes typed
verification evidence through a dedicated packaged constructor, never by
relaxing `LocalWorkerConfig`'s debug restrictions. IPC readiness remains mandatory
after launch. A signed executable alone does not prove backend readiness.

Hashing a path immediately before spawning is not a same-user replacement-race
defense. Packaging is not a filesystem sandbox or workload attestation. Review
bundle replacement, hardened-runtime behavior and library validation on an
installed build before opening the gate. No speculative JIT/library-validation
exceptions: add an entitlement only after a reproducible MLX need and review.
Release keys are not required for the initial fixture/staging implementation.

## Resource policy v1

The following are conservative pilot design defaults, not measured performance
limits. Keep decisions and user-facing reasons in shared Rust; the macOS adapter
only supplies typed observations. Preserve independent consent and owned-child
cleanup. Never put a platform stop behind a full ordinary-command queue.

| Observation | Launch decision / running-worker action |
| --- | --- |
| Explicit AC power; Low Power Mode off | Eligible if all other controls pass |
| Battery, UPS, unknown power, or Low Power Mode | Refuse / request bounded stop |
| Thermal nominal | Eligible if all other controls pass |
| Thermal fair or serious | Refuse / request bounded stop |
| Thermal critical or critical memory pressure | Refuse / urgent owned-child stop |
| Memory pressure warning | Refuse / request bounded stop |
| Missing, stale or unsupported required observation | Refuse / request bounded stop |
| Sleep notification | Cancel starts, request stop; no guaranteed drain before sleep |
| Wake | Invalidate old observations and worker telemetry; reconcile ownership; stay paused |

Resource stops preserve consent but require an explicit Resume after eligibility
recovers. This avoids restart oscillation and automatic work on wake/AC return.
Manual Pause, Disable and terminal Shutdown dominate policy updates. Disable
revocation cannot be undone by a delayed platform event. No sleep-prevention
assertion and no work triggered by merely opening Compute.

Proposed observation lease: refresh every two seconds; expire after six seconds
using monotonic time, plus immediate change notifications. These values require
device validation. A renewed lease must contain fresh queried observations;
re-sending an old event is not freshness. Memory-pressure bootstrap needs a
supported current-state query plus notifications, not silence interpreted as
normal. Missing bootstrap capability keeps eligibility unavailable.

Normal policy stop uses the current bounded cooperative drain path. Proposed
urgent budget: at most one second for cooperative handling, then kill only the
owned child and allow two seconds to reap. If termination remains unconfirmed,
retain ownership and report failure; do not launch another worker. These are
application deadlines, not OS real-time guarantees. Never infer acknowledged
drain from exit, sleep, a timeout, or force-kill.

Use IOKit [power source snapshots and notifications](https://developer.apple.com/documentation/iokit/iopowersources_h?changes=_4),
Foundation [thermal notifications](https://developer.apple.com/documentation/foundation/processinfo/thermalstatedidchangenotification),
Dispatch memory-pressure observation, and NSWorkspace sleep/wake observation.
Do not treat Low Power Mode as a proxy for AC/battery state.
Exact SDK APIs and memory bootstrap must be verified in the adapter spike.

RAM allowance remains scheduler capacity, not a hard resident-memory ceiling.
Measure worker memory, GPU allocation where supported, CPU, responsiveness,
bandwidth and cache growth on real hardware. Before pilot activation, define
measured device/workload eligibility and download/disk budgets. Policy stopping
alone does not establish a hard resource or filesystem boundary.

## Acceptance and activation boundary

Fixture tests must prove artifact refusal, policy precedence and stale-event
behavior without launching production work. Real-device evidence must include
an installed MLX worker with trace contribution both disabled and enabled,
unplug/replug, thermal/pressure injection, actual sleep/wake, Pause/Resume,
Quit timeout, crash recovery, app replacement and Intel trace-only regression.
Use injected critical-pressure/thermal observations; do not deliberately stress
the user's machine into a hazardous state. Record synthetic versus real events.

Every run separates backend/work evidence, signed status, observed SafeToExit,
process termination and lock release. Do not promise spare migration or earnings.
Activation requires reviewed resource/cache bounds, sandbox/trust-boundary
decision, signed installed-build evidence, compatible test pool/account policy,
and attestation acceptance where required. Until then `tc_compute_open` remains
closed. Signing, external distribution and production activation require their
own explicit authorization.
