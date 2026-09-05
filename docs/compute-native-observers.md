# macOS compute resource observations

`macos/Sources/TCBridge/TCComputeResourceObserver.swift` is an app-owned adapter,
not a launch policy. ComputeModel owns one continuously registered observer and
rebinds its safe TCCompute host after reopening. Closing a confirmed-stopped host
stops observation; the bridge rejects callbacks against closed handles. Opening/closing Compute windows
must not start/stop this observer. Rust owns consent, eligibility, freshness,
latched stops and explicit Resume.

The adapter requests an opaque controller ticket immediately before each full
synchronous OS read. It submits that ticket unchanged with the reading. It does
not generate timestamps or epochs. A two-second timer and native notifications
trigger full reads; previously sampled fields are never restamped as current.

Power uses `IOPSCopyPowerSourcesInfo` and `IOPSGetProvidingPowerSourceType`.
AC, battery and UPS are distinct; missing/new values remain unknown.
Low Power Mode and thermal state use `ProcessInfo`. Memory bootstrap and every
refresh query the current `kern.memorystatus_vm_pressure_level` using a read-only
`sysctlbyname` call. Failure or unexpected size/value is unknown, not normal.
This masked sysctl may be denied by a sandbox or unavailable on another OS;
such a machine must fail closed until a supported observation path exists.

The sysctl returns dispatch flags (normal 1, warning 2, critical 4), **not** the
internal kernel pressure enum. This conversion is explicit in
[Apple XNU's sysctl handler](https://github.com/apple-oss-distributions/xnu/blob/f6217f891ac0bb64f3d375211650a4c1ff8ca1ea/bsd/kern/kern_memorystatus_notify.c#L1885).
Power return values follow
[Apple's power-source API](https://developer.apple.com/documentation/iokit/iopowersources_h/1810316-iopsgetprovidingpowersourcetype)
and the installed macOS SDK's `IOPowerSources.h`.

Memory events trigger a fresh full read; a newly delivered critical/warning
event can conservatively raise that reading's severity if the query already
recovered. An event indicating normal can never turn an unknown query into
normal. Only Rust decides when recovery permits an explicit Resume.

All callbacks execute synchronously on the main actor. Sleep suppresses further
reads and reports sleep immediately. Wake reports invalidation before reading
again, without issuing Resume. A registration generation ignores queued events
from an earlier stop/start cycle. Shutdown must leave the Rust resource ingress
callable during a blocking worker drain; serializing ingress behind that drain
would prevent urgent escalation. The Rust watchdog must remain independent of
the UI thread so stalled native delivery expires rather than extending a lease.
There is no sleep-prevention assertion or guarantee the worker drains before
the OS sleeps.

## Local evidence, 2026-09-05

- Direct `swiftc -swift-version 6 -warnings-as-errors -typecheck` passed.
- Eight deterministic tests in `ComputeResourceObserverTests.swift` passed in
  a standalone Swift package compiling the exact adapter and test files. This
  isolates the new adapter from the Rust dylib and Sparkle; it is not evidence
  that the entire app package passed.
- A read-only native probe started the actual observer, ran the main run loop
  for 2.2 seconds, stopped it, and ran for another 2.2 seconds. It observed two
  complete readings and no post-stop samples.
- Outside the tool sandbox: AC power, Low Power Mode off, thermal **fair**,
  memory normal. Fair thermal state correctly forbids a real worker launch.
- Inside the tool sandbox: memory unknown because the sysctl was denied. This
  demonstrates the fail-closed path, not support for compute in that sandbox.
- No sleep, power transition, memory pressure or thermal load was induced.

Tests cover ticket-before-read ordering, fresh read failure, idempotent start
and stop, old-generation callbacks, sleep/wake ordering, failed ticket issuance,
transient critical memory events, strict power/memory mappings and explicit
JSON null for unknown Low Power Mode. Native event delivery under actual
sleep/wake and hardware transitions still needs a separately controlled pilot.

## Controller integration

The debug local-worker actor now requires observations; no adapter means no
launch. The legacy `compute-local` Rust CLI has no native observer and now refuses
immediately instead of inventing healthy readings. Prior MLX smoke evidence is
historical; repeat future positive smoke through an observing native host only
when actual readings are eligible. No production constructor or signing gate is
enabled by this change.

`tc_compute_resource_begin_json` issues one pending `{epoch,sequence}` ticket and
records its monotonic capture time. `tc_compute_resource_event_json` consumes the
ticket with a complete reading, or accepts sleep/wake. Old, replaced and reused
tickets cannot refresh the lease. Wake invalidates pending tickets and telemetry.
The ABI accepts trusted in-process platform facts, not cryptographic OS evidence.
Callbacks must never reuse cached fields under a new ticket.

The host releases its Swift pointer lock during bounded shutdown, retaining an
active-call pin that prevents free. This permits critical observations to shorten
a normal stop already in progress. After failed reap Rust retains ownership and
requires explicit Resume after recovery; neither healthy samples nor reopening a
window can restart a worker.

## Integrated verification, 2026-09-05

- Warnings-denied combined contributor/FFI compute tests: 44 + 2 passed. Run
  `RUSTFLAGS='-D warnings' cargo test -p trace-commons-contributor -p trace-commons-contributor-ffi --lib compute:: --locked`.
  This combined build also checks the FFI unwind boundary with unified Tokio
  features; urgency state stays behind the controller mutex.
- Both header suites: eight tests passed, including exact ABI parity and C/C++
  compilation. Four unchanged license-boundary tests passed via the isolated
  `rustc --test` procedure recorded in the implementation plan.
- Contributor/FFI library Clippy passed with warnings denied and repository
  allowances. Standalone contributor `--no-default-features` check and workspace
  formatting passed. No dependencies changed.
- Final FFI dylib built; `swift test --filter 'Compute|QuitCoordinator'` passed
  21 XCTest cases plus eight observer tests. The full app compiled; existing
  unrelated RecentSearches/ProjectRow test concurrency warnings remain.
- The real-reading-through-ABI test reported `power=ac thermal=fair memory=normal
  eligible=false`. It requested no Enable/Resume and launched no worker.
- Native bridge tests used a pinned `/bin/sleep` wrapper, not MLX or a pool.
  They verified critical observation ingress during pinned-handle shutdown,
  forced child termination without fabricated drain acknowledgement, consumed
  ticket rejection, and wake invalidation.
- Regression coverage includes stale expiry without any host callbacks, pressure
  during readiness, Resume after a resource stop, and lock release after reap
  while a cloned descriptor remains alive. The clone models shared descriptor
  ownership; the exact cause of an earlier intermittent lock assertion was not
  proven. Failed reap/unlock keeps ownership and cannot permit a new launch.

These results do not qualify a signed installed app, sandboxed memory-pressure
support, real sleep/wake or unplug transitions, forced OS termination, or an MLX
workload under pressure. Artifact assembly/signature verification and controlled
device qualification remain gates. Production compute stays unavailable.
