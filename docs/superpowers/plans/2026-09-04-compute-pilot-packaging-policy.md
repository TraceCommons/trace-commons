# Compute pilot packaging and resource-policy implementation

Status: artifact validator, resource-policy actor enforcement, ABI ingress and
native observers implemented. Signed package assembly and installed qualification
remain open. Production construction remains unavailable.
Evidence and the MLX runtime asset-location gap are recorded in
[artifact inventory](../../compute-artifact-inventory.md). No shipping gate changed.
Design: [contract and policy](../specs/2026-09-04-compute-pilot-packaging-policy.md).
Build on Trace Commons `bd523c22` and Holonear `cef95b36` in isolated worktrees.

## 1. Artifact inventory and manifest validator

- Build the pinned arm64 MLX CLI locally; inventory libraries, Metal assets,
  minimum OS and backend readiness. No signing-key access or publishing.
- Add a strict shared manifest parser, fixed bundle resolution and typed artifact
  refusal reasons without new dependencies. No launch-capability change.
- Test missing/modified helper or asset, malformed/oversized manifest, traversal,
  duplicate paths, symlink escape, wrong architecture/backend/IPC, and pre-sign
  versus post-sign hash mismatch. Mock signature outcomes are not release proof.

Exit: exact artifact inventory and deterministic refusal tests; shipping gate closed.

## 2. Shared resource-policy reducer

Implemented foundation: `compute::policy` contains typed complete observations,
a six-second monotonic lease, epoch/sequence rejection, latched normal/urgent stop
requests, manual command precedence and explicit Resume after confirmed stop.
Tests cover the 240 resource combinations, exact expiry, recovery, event reorder,
critical escalation, clock errors, disabled/shutdown intent and wake invalidation.
The local-development actor now enforces this gate. Rust-issued single-use
capture tickets precede native reads, and a 250ms watchdog expires observations
independently of readiness/drain/UI callbacks. Safety updates bypass the bounded
start queue. The last eligibility check and actual spawn share the resource lock.
Urgent observations cap cooperative stopping at observation + 1 second, then
force-kill/reap with a 2-second budget; these are not OS real-time guarantees.
Failed reap retains child/lock, including when the host drops the controller.
Readiness/status futures are interruptible, and telemetry is resource-epoch bound.
The Swift bridge pins handles during blocking shutdown without blocking resource
ingress. One app-owned observer persists across controller replacement and replays
sleep state. Native evidence and remaining gates: [observer notes](../../compute-native-observers.md).

Previous reducer-only verification (2026-09-05): all 31 contributor compute tests passed with
warnings denied; all nine standalone policy tests also passed on Rust 1.92.
Contributor library Clippy passed with the repository allowances and warnings
denied; workspace formatting passed. Tests use injected monotonic times, not
sleeping or induced pressure on the host. No native observer validation is claimed.
All four unchanged license-boundary tests passed when compiled directly with
`rustc --test`, the cached `serde_json` dependency and this worktree's server
manifest directory. The Cargo test invocation was interrupted during its large
server rebuild; it is not recorded as a passing Cargo test run.

Enforcement invariants (preserve during future package integration):

- Serialize policy updates with actor lifecycle actions; do not queue safety
  updates behind the bounded start-command queue. Evaluate on timer ticks and
  immediately before spawn. A `Decision` is a snapshot, not a reusable launch token.
- Only acknowledge a stop after reaping or proving no owned child exists. A drain
  acknowledgment or failed reap cannot clear the latch. Healthy updates cannot
  downgrade an urgent request while draining.
- Provide complete genuinely refreshed readings; never relabel cached fields
  with a fresh timestamp. Use unknown when a current read is unavailable.
- On wake discard adapter caches, use the new epoch, invalidate worker telemetry
  and reconcile ownership. Epochs and intent are session-local, not persisted.
- Consent persistence, signed package authorization and worker ownership stay
  with their existing owners. The resource gate grants none of those permissions.
- Real hardware transitions and installed-package qualification remain separate
  from synthetic lifecycle fixtures; do not claim the entire pilot matrix passes.

- Add pure typed observations, monotonic freshness, policy reasons and stop
  urgency to the contributor core. Project copy/state through both ABI headers.
- Integrate durable policy-stop intent with the existing actor; preserve Disable
  and terminal Shutdown precedence. Recheck eligibility immediately before spawn.
- Test queued enable versus pressure, unplug during readiness, stale adapter,
  urgent escalation while normal drain waits, event reorder, wake invalidation,
  explicit Resume only, and failure retaining child ownership.

Exit: policy cannot be bypassed by queue saturation or stale eligibility.

## 3. Native adapter and inert package assembly

- Verify SDK power/thermal/memory/sleep APIs, especially memory-pressure initial
  state; add app-owned observation independent of window lifetime.
- Add Security-framework verification bridge and local staging option to bundle
  assembly. Preserve universal app/FFI checks and Sparkle signing semantics.
- Keep missing packaging an ordinary trace-only build; explicit compute-package
  requests with incomplete artifacts fail rather than silently omit the worker.
- Add exact nested signing order and post-sign manifest generation to the
  release script, but do not invoke credentialed signing in this slice.
- Test stale/missing adapter, Intel refusal, unchanged onboarding and trace
  operation, shared refusal copy, package tampering and duplicate stop behavior.

Exit: testable packaging and adapter exist; no production constructor enabled.

## 4. Installed-device qualification (separate gate)

- Obtain explicit authorization for credentialed signing/test distribution.
- Run notarized installed app on supported Apple Silicon and verify actual MLX,
  full runtime assets and clean-machine launch; retain Intel trace-only coverage.
- Execute the design's lifecycle matrix; record memory, responsiveness, thermal,
  network and disk measurements and determine supported workload/device bounds.
- Resolve model licensing/digest/cache limits, sandbox/trust boundary, test-pool
  account attribution and attestation compatibility. No real funds required.

Exit: reviewed pilot evidence, not automatic production approval.

## Verification discipline

Each code slice runs warnings-denied targeted Rust tests/checks, repository Clippy
allowances, formatting and license-boundary tests. ABI changes run header-parity
tests; native changes build the FFI dylib and run the relevant Swift suites.
Package scripts get syntax checks and temporary-fixture tests before real builds.
No new dependency without explicit approval. Do not rerun expensive production
or credentialed workflows to compensate for missing local test evidence.
