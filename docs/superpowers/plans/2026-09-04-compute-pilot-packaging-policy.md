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

## Continuation checkpoint, 2026-09-05

The isolated `resume/holonear-package-verification` branch starts from the committed
package-assembly snapshot `64063443`. That snapshot already integrates the resource
reducer, urgent stop actor and native observers. It does not yet include current
Trace Commons main `52b4fa7e`; reconcile those independently active native changes
before claiming merge readiness.

The next read-only signature gate now has a Developer ID verification harness and
negative OS-verifier evidence; see [artifact inventory](../../compute-artifact-inventory.md).
It supplies neither runtime launch authorization nor the still-required
Security-framework bridge. The remaining order is:

1. Integrate the compute snapshots with current Trace Commons main and reconcile
   native ownership/lifecycle changes; repeat contributor/FFI and native suites.
2. Review Orchard worker IPC `7d6f7051` and bundle relocation `0b348bc0` against
   current Orchard main, including the still-open Metal guard PR #2452. A declared
   source revision is not verified artifact provenance.
3. Implement compiled signature policy, the runtime Security-framework bridge,
   complete nested-code/hardened-runtime checks and release-script staging order.
   Preserve the disabled production constructor throughout preparation.
4. Obtain the existing separate authorization for signing/test distribution and
   perform installed Apple Silicon resource and MLX qualification. Synthetic
   fixtures and an inert signature harness do not close that gate.

Original worktrees are unchanged. The Orchard primary checkout was dirty during
inventory; its uncommitted work was not read into or copied into this continuation.

## Current-main integration checkpoint, 2026-09-05

`resume/holonear-main-integration` joins the committed compute chain through
`3c9731ee` onto Trace Commons main `52b4fa7e`. The merge needed no textual conflict
resolution. It preserves the native NEAR onboarding changes and independent
Compute navigation: choosing Compute does not enroll, discover trace sessions,
or grant trace consent. The separately developed onboarding continuation
`8119d971` also merges cleanly in a `git merge-tree` check; its single coordinator
identity remains inside the Traces destination. That check is source integration
evidence, not a test run of the combined future branches.

Verification on the current-main integration:

- Warnings-denied contributor compute tests: 44 passed; compute FFI tests: 2 passed.
  The first sandboxed attempt refused two local socket binds with `EPERM`; the
  approved local-socket rerun passed every case.
- Full FFI library + ABI tests: 106 passed; warnings-denied server binaries check passed.
- Both C/C++ header and ABI parity suites: 8 passed. Inert assembly: 5 passed.
  Read-only signature harness: 8 passed, including actual OS refusal checks.
- The final FFI dylib built and the full macOS suite passed: 520 XCTest cases
  plus 8 native-observer tests. Existing RecentSearches/ProjectRow concurrency
  warnings are unchanged; this is not a warnings-free Swift build.
- The unchanged Cargo license-boundary suite passed all 4 tests. License checks
  passed for default, near-ai-scorer, local-gpu-models, and all features.
- Contributor/FFI library and example Clippy passed with warnings denied and the
  repository allowances. Standalone contributor `--no-default-features` check passed.
- Workspace formatting, package-script syntax, and patch whitespace checks passed.

No new crate dependency or lockfile change is introduced; the original compute
implementation enables the existing Tokio dependency's `process` feature.
The production constructor remains unavailable. Developer signature checks do
not complete the runtime signature gate. Orchard integration, compiled runtime
trust requirements, complete packaging and separately authorized installed-device
qualification remain unfinished as listed above.

## PR 610 review follow-up

The first readying pass was stopped when the substantive review arrived. The
revision addresses its eleven required items as follows:

| Item | Change and evidence |
| --- | --- |
| 1 | Compute failures render Rust-owned recovery copy, coral refusal glyph and Retry; native tests repair invalid settings and reopen. |
| 2 | Quit refusal uses coral and the refused glyph. |
| 3 | All Swift pointer calls use an active-call pin and release the lifetime lock during Rust work; a blocked-command test exercises resource ingress and close refusal concurrently. |
| 4 | Compute resolves its directory independently of daemon startup/socket-path constraints and offers retry; native directory tests cover that separation. |
| 5 | Vector provenance is explicitly source-derived, not captured upstream interoperability evidence. |
| 6 | A real child fixture publishes the launch-instance endpoint, verifies signed requests, sends signed status and drain, and exits; tests reach Training and Serving and reject wrong/bounded endpoint data. |
| 7 | The compute-package Cargo example is explicitly registered with `test = true`; all five assembly tests run under default test discovery. |
| 8 | Independently supplied manifest SHA-256 is checked before parsing; coordinated manifest/worker rewrites fail without replacing that independent pin. |
| 9 | Helper and Compute resource inventories reject unlisted files/directories; executable header validation uses the same bytes that were hashed. |
| 10 | Existing macOS CI invokes Python signature tests, including actual csreq compilation and system-verifier refusals. |
| 11 | A held worker lock returns a distinct typed refusal and Rust-owned explanation; tests prove no child is adopted or spawned. |

The disconnected-peer test now accepts a real loopback connection before closing,
and asserts an I/O failure. The command drops its seed environment entry after
spawn, without claiming secure erasure of allocator or child-environment copies.
Canonical path requirements remain conservative; `/private/tmp` is supported
rather than weakening ancestor symlink checks for `/tmp`. Documentation distinguishes
signature verification from notarization, fresh revocation checks and Gatekeeper.

The existing development entrypoint is controlled by `debug_assertions`, not the
Cargo profile name. This is documented rather than removing exported C symbols
and silently changing ABI availability. Ordinary production construction remains
unavailable. Runtime launch authorization, actual upstream interoperability and
installed-device qualification remain separate unfinished gates.

Combined local revision validation: 53 contributor compute tests and 2 compute
FFI tests passed; the one ignored test is the child-process entrypoint invoked by
the successful lifecycle tests. All-target contributor/FFI Clippy and Windows-GNU
contributor test compilation passed with warnings denied. The package example's
5 tests, Python signature suite's 8 tests, and unchanged license boundary's 4 tests
passed. The final FFI dylib built; the full native suite passed 524 XCTest cases
plus 8 observer tests. Formatting and patch whitespace checks passed. CI still
must validate the pushed revision before it is marked ready for review.
