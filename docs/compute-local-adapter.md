# Local compute adapter

The production `ComputeController::open` / `tc_compute_open` path remains
unavailable. Only the explicit `open_local` constructor enables a Unix debug-build
development worker. Release builds refuse that constructor. The native app does
not discover a worker through PATH, environment overrides, or persisted settings.

The local harness accepts an absolute state root, an absolute worker binary, its
expected SHA-256, a loopback WebSocket coordinator, a RAM scheduling allowance in
GiB, and an optional observation duration (default 45 seconds, maximum 300):

```sh
cargo run -p trace-commons-contributor --example compute-local -- \
  /absolute/test-state /absolute/holonear EXPECTED_SHA256 \
  ws://127.0.0.1:PORT 1 45
```

The harness explicitly grants compute consent, prints label-only JSON snapshots,
and requests shutdown. It exits nonzero if startup fails, no authenticated status
arrives, or worker termination remains unconfirmed. A synthetic public payout
identifier `compute-pilot.testnet` satisfies the local worker CLI preflight; this
does not configure a wallet, contact a production pool, or move funds.

## Contract and dependency decision

The IPC v0 executable contract is pinned to Holonear revision
`ef4e6e2479e8395f7d972d3342bad97851f2104e`, with committed cross-implementation
vectors in `crates/trace-commons-contributor/tests/fixtures/worker_ipc_v0.json`.
Trace Commons independently signs and verifies those exact body bytes/signatures
using its existing `ring` dependency. Golden cases cover Status and Drain;
tampering with direction, nonce, version, instance, or payload is refused.

A direct Holonear desktop-core dependency would pull protocol/crypto/store,
account, and AWS components, and couple the contributor to Holonear's newer
compiler. The native adapter instead uses existing ring, serde, sha2, hex and
Tokio. It enables Tokio's existing `process` feature; no package or lockfile
dependency is added. This preserves the contributor's Rust 1.92 boundary.

Frames are four-byte big-endian lengths plus JSON, capped at 65,536 bytes before
allocation. Every request uses a new random nonce. Status has a two-second
deadline, Drain ten seconds. There is no legacy transport fallback. A signed
response proves current endpoint liveness and the worker's reported state and
assignment; it does not prove training progress, measured memory, or earnings.

## Lifecycle and ownership

One background actor owns settings transactions and the child process. Ordinary
start commands allow one in flight, with visible busy rejection. Pause, Disable,
and Shutdown use durable coalesced stop intent outside that queue. Revocation
cannot be replaced by a weaker Pause. Stops interrupt startup readiness, and
discard pending starts. Settings remain independent of trace enrollment; all
consent restores paused and every later launch needs explicit Enable or Resume.

Shutdown is terminal for a controller: subsequent starts are refused. The C
shutdown call waits at most 30 seconds and returns `worker_stopped`, separately
from `drain_outcome` and `stop_outcome`. A timeout returns unconfirmed termination;
retain the handle and retry. Only after confirmed termination may a shell reopen
a controller (restored paused). Free is not a synchronous drain barrier; shells
must explicitly shut down before freeing a live controller.

The adapter holds `compute/worker/node/controller.lock` until its owned child is
reaped and probes `worker.lock` before launch. The worker independently owns its
lifetime lock; a competing process is refused, never adopted or killed by PID.
Lock files are never unlinked. Readiness requires the atomically published
`node/worker-endpoint.json` with matching version, freshly generated launch public
key and loopback/nonzero address, then a signed response. Endpoint reads are
bounded to 4096 bytes. Readiness defaults to 30 seconds in the harness.

Three consecutive failed status observations initiate stop. The snapshot also
marks telemetry older than six seconds stale. Cooperative Drain is followed by
three seconds for process exit, then owned-child kill and a two-second reap wait.
Forced termination never implies coordinator acknowledgement. Failed reaping
retains ownership. Duplicate stops preserve previously observed drain evidence.

Child environment is cleared. HOME/USERPROFILE, caches and temporary paths are
under the worker home; only fixed system PATH, required Windows SystemRoot,
HOLONEAR_HOME, the ephemeral IPC capability, and relay-only peer transport are
supplied. Standard streams are null. The worker cannot inherit contributor
tokens through the environment, but this is not an OS filesystem sandbox.

## Remaining release gates

Local hashing immediately before launch is a development integrity check, not
signed packaging or protection against a same-user executable replacement race.
The child-environment capability is not isolated from a compromised host.
Cross-instance machine-wide resource arbitration, enforced resource limits,
filesystem restrictions, platform signing/attestation acceptance, and production
account/pool configuration remain unimplemented. The production capability must
remain closed until those gates and real-device lifecycle validation pass.
