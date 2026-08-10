# macOS shell: freeing the daemon handle under in-flight FFI calls

Status: FIXED. Both reported defects were real. The handle is now freed only
after teardown can prove nothing is inside the C ABI with it, and is
deliberately leaked rather than freed when it cannot.

Swift-side only. No change to `crates/`, no change to the C ABI, no new
dependency.

## What the header actually requires

`crates/trace-commons-contributor-ffi/include/trace_commons.h` is the
authority, and it is narrower than "one call at a time":

- `tc_call` / `tc_preview_open` / `tc_subscribe` MAY run concurrently on
  other threads, including concurrently with `tc_daemon_stop`.
- `tc_handle_free` "must not be called concurrently with any other call still
  using the same pointer", must run on a plain thread outside any tokio
  runtime, and must not run concurrently with `tc_daemon_stop`.
- `tc_daemon_stop` is NOT a teardown barrier and NOT a synchronization point
  for subscriptions. `tc_unsubscribe` is the only function that guarantees no
  further callback, `ctx` must stay alive until it RETURNS, and it can refuse
  SILENTLY (it returns void) — so the caller must compare `tc_last_error`
  before and after to learn whether the barrier held.

So the requirement is shared/exclusive: many concurrent users of the pointer,
exactly one exclusive teardown, and no new user admitted once teardown starts.

## The defects

### (a) Untracked detached tasks vs. `tc_handle_free` — REAL

Every user action in `AppModel` runs its daemon call on a `Task.detached`
that reaches `tc_call` / `tc_preview_open` through `DaemonClient`. The call
sites were `enroll` (line ~249), `setConsentScopes` (~297),
`loadMissingSummaries` (~351), `undoApproval` (~404), `openPreview` (~440),
and the shared `perform` helper (~488) that backs status, queue, history,
projects, settings, consent options, outcome counts, approve, dismiss, pause,
resume and `set_project_mode`. None was tracked. `shutdown()` called
`daemon.close()` → `tc_handle_free` on the main thread with no regard for
them. Quitting during a preview (the longest call — it blocks for a whole
redaction pass), an enrollment, or a refresh freed the handle while a call
was using it.

The old `TCDaemon` also read and wrote `handle` from those threads with no
synchronization at all: `close()` assigned `handle = nil` on the main thread
while `call()` read it on a detached task. That is a data race on top of the
use-after-free, and `.swiftLanguageMode(.v5)` means the compiler said
nothing.

### (b) Fixed 200 ms sleep for the unsubscribe retry — REAL

`shutdown()` retried a refused `tc_unsubscribe` on a fresh `Thread`, slept
200 ms, and then freed the handle regardless of whether the retry had
finished or succeeded. Three separate problems: the retry thread could still
be inside `tc_unsubscribe` with a pointer the main thread then freed; the
callback `ctx` (`TCCallbackBox`) could outlive the handle; and if the retry
was refused again, nothing noticed.

Nothing in the reported defects turned out to be imaginary.

## The fix

### Mechanism: an `NSCondition`-guarded in-flight counter in `TCDaemon`

`TCDaemon` is the only file in the package that holds the raw pointer, so the
gate lives there. Every use of the pointer now goes through one private
`withHandle` that admits any number of concurrent callers while the handle is
live, increments an in-flight counter for the duration of the C call, and
refuses everyone once a `closing` flag is set.

Why not the alternatives (this reasoning is in the code, not only here):

- **An actor** serializes everything. `tc_preview_open` blocks for the whole
  redaction pass; parking it on an actor would stall every unrelated `status`
  behind it, and blocking work on Swift's cooperative pool starves that pool.
  It would also force every synchronous `DaemonClient` method to become
  `async`, rippling into `SelfTest.swift` and `DebugScreenshot.swift`, which
  this task must not touch.
- **A serial DispatchQueue** has the same serialization cost and cannot state
  the thread-affinity rule the header cares about (teardown on a plain
  thread).
- **A plain mutex** serializes just as much and still needs a condition
  variable to express "wait until the last in-flight call has left".

`NSCondition` is Foundation. No new dependency.

### Teardown: `TCDaemon.shutdown(unsubscribing:drainTimeout:unsubscribeTimeout:)`

Ordered, on the calling thread (the main thread, at `willTerminate`, which is
a plain thread with no tokio context):

1. Set `closing` — every subsequent `call` / `openPreview` / `subscribe` is
   refused with the ABI's own `unavailable` / `handle-freed` shape.
2. Wait on the condition until the in-flight count reaches zero (default 3 s).
3. Null the stored pointer under the lock, so no Swift path can reach it
   again whatever happens next.
4. `tc_unsubscribe` once on this thread, checking `tc_last_error` before and
   after. If refused, retry once on a genuinely fresh `Thread` — the header's
   refusal check is thread-local — and JOIN it with a bounded wait (2 s).
   Release `ctx` only on a confirmed barrier.
5. `tc_daemon_stop` then `tc_handle_free`, sequentially, on this one thread.

**If either wait fails, the handle is LEAKED, not freed**, and the outcome is
`.leaked(label)`. A handle still reachable by an in-flight C call, or a
subscription whose barrier the ABI would not confirm, is never passed to
`tc_handle_free`. The process is exiting: an unfreed handle costs nothing and
a use-after-free is a crash or worse. The leak also makes the timed-out retry
thread safe — it keeps operating on a pointer that now stays valid forever.

`deinit` follows the same rule: it frees only if the counters say the object
is idle and teardown never ran, and leaks otherwise.

`AppModel.shutdown()` drops `subscription` / `daemon` / `client` first (so
nothing new starts from the Swift side — `perform` and friends all guard on
`client`), then calls `daemon.shutdown(unsubscribing:)` and records a fixed
label if the handle was leaked. It deliberately does not try to track the
detached Tasks itself: a Task's suspension points have nothing to do with
when the C call actually returns, and only the wrapper making the call knows
that.

### Swift 6 language mode

`Package.swift` compiled every target at `.swiftLanguageMode(.v5)`.

- **TCBridge is now `.v6`** and compiles clean — no errors, no warnings. That
  is the target that owns the pointers, so it is the one worth having checked.
  Two `@unchecked Sendable` boxes were needed and are commented: the retry
  state (guarded by its own `NSCondition`) and the handle+subscription pair
  deliberately carried to the retry thread.
- **The app targets cannot move yet.** `TraceCommonsApp` at `.v6` produced 27
  errors before the compiler stopped at emit-module: `AppModel.PreviewOutcome`
  is not `Sendable` (it carries a `TCPreview`), `Task<...>.value` cannot cross
  the main actor with it, `Notifier.shared` is a non-`Sendable` global, and
  more behind those. Fixing them means changing `PreviewOutcome`'s shape and
  `Notifier`, which ripples into `SelfTest.swift` and `DebugScreenshot.swift`
  — off-limits for this task. Left as follow-up work.

## What was verified

Swift 6.3.3 / Xcode toolchain, macOS 26.5 SDK, arm64.

**Build.** `cd macos && swift build` from a cleaned `.build`: `Build
complete!`, no errors and no warnings, all four targets.

**Teardown stress, at the ABI level.** `TC_DEMO_TEARDOWN_STRESS=1
./.build/debug/tc-ffi-demo` — a new env-gated mode in the existing FFI demo.
It starts the daemon, registers a subscription, runs 8 threads hammering
`tc_call("status")`, and tears down 500 ms in, with calls genuinely mid-flight.
Five consecutive runs, all clean:

```
stress: subscription registered
stress: calls inside the ABI at teardown: 8
stress: shutdown -> freed (served=36582 refused=1004)
stress: after workers finished (served=36582 refused=4741100)
```

`calls inside the ABI at teardown: 7`/`8` is the exact condition the old code
freed under. `served` does not move after `shutdown` returns — every later
call was refused, ~4.7M of them — and the process exits 0 with no crash.

**The leak branch.** `TC_DEMO_TEARDOWN_DRAIN_TIMEOUT=0` forces the drain to
fail:

```
stress: shutdown -> leaked("in-flight-calls-did-not-drain") (served=36746 refused=0)
stress: after workers finished (served=36753 refused=4029307)
```

The handle is not freed, and the 7 calls that completed after `shutdown`
returned (36746 → 36753) are precisely the ones that would have been reading
freed memory under the old code.

**The app itself.** `TRACE_COMMONS_SCREENSHOT_DIR=... TRACE_COMMONS_QUIT_AFTER_SHOT=1
TRACE_COMMONS_SELFTEST_OUT=... TRACE_COMMONS_DEMO_PREVIEW=1 ./scripts/run-demo.sh`
against the fixture state dir. The self-test ran to completion against the
live in-process daemon (2 queued fixture sessions, real preview through the
FFI — `ffi preview body bytes: 866`, `search Northwind -> 2 match(es)`,
secrets confirmed scrubbed, approve/undo/pause/resume all exercised), all 11
screenshots rendered including the preview sheet, then the app called
`model.shutdown()` and terminated. It exited on its own after ~35 s with no
crash and no entry in `~/Library/Logs/DiagnosticReports`.

## Honest limits of that evidence

- **The app-level quit was not proven to be mid-call.** The self-test had
  finished by the time `QUIT_AFTER_SHOT` fired, so that run tore down an idle
  daemon. The mid-call case is covered by the stress harness above, which
  measures 7–8 calls inside the ABI at the instant teardown begins. Forcing a
  genuinely mid-call quit in the GUI would mean adding another hook to
  `DebugScreenshot.swift`, which this task must not touch.
- **A negative control did not crash.** Patching `shutdown` to skip the drain
  and free anyway (8 threads, 25–35k calls, three runs, also under
  `MallocScribble`/`MallocGuardEdges`) produced no crash. That is what
  undefined behaviour looks like from the outside: the freed allocation is not
  unmapped, so reading it is silent. It means the stress harness is a
  regression detector for the drain semantics (which it measures directly),
  not a reproducer for the crash. The argument that the old code was unsound
  rests on the header, not on a crash.

## Left undone

- `TraceCommonsApp` / `tc-ffi-demo` remain at `.swiftLanguageMode(.v5)`; see
  above for what blocks `.v6`.
- **A live `TCPreview` can outlive `tc_handle_free`.** The header says a
  `const char*` is valid "until the handle that owns it (`tc_handle*` or
  `tc_preview*`) is freed" and does not say whether freeing the daemon handle
  invalidates an open preview. Opening a preview is now inside the gate, so
  the open call itself is safe, but a `TCPreview` held by `PreviewSheet`
  across a quit is not tracked. Closing outstanding previews before
  `tc_handle_free` would settle it; it is a different object's lifetime, so it
  was left for a follow-up rather than folded into this fix.
- `AppModel` surfaces a leaked teardown as `lastActionError`, which nothing
  will render at `willTerminate`. It is a label for a future diagnostics
  surface, not a user-visible message.
