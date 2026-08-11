# macOS contributor shell — build report

Date: 2026-08-08
Branch: `macos-shell`
Builds on: `macos-ffi-bridge-report.md` (the proven C ABI bridge)

## What exists now

A SwiftUI menu-bar application that hosts the contributor daemon in-process
through the C ABI, driven entirely by `trace_commons.daemon.v1_1`.

Package layout (`macos/`):

| Target | Kind | What it is |
|---|---|---|
| `CTraceCommons` | systemLibrary | module map over `trace_commons.h` (unchanged) |
| `TCBridge` | library | the ONLY place raw pointers appear: `TCDaemon`, `TCPreview`, `TCSubscription` |
| `TraceCommonsApp` | executable | typed layer, app model, SwiftUI views |
| `tc-ffi-demo` | executable | the original bridge demo, now importing `TCBridge` |

Bundle: `macos/scripts/make-app-bundle.sh` assembles `TraceCommons.app` with
`LSUIElement = true`, `LSMinimumSystemVersion 14.0`, bundle id
`ai.tracecommons.shell`, the dylib relinked into `Contents/Frameworks`, and an
ad-hoc signature. Fixture launcher: `macos/scripts/run-demo.sh`.

### 1. Bridge additions

`TCDaemon` gained `openPreview`, `subscribe` and `unsubscribe` alongside the
existing `call`/`stop`/`close`. Three header rules are honoured explicitly and
commented at their call sites:

- **`tc_unsubscribe` is the only barrier.** `ctx` is a retained
  `Unmanaged<TCCallbackBox>` and is released only after a `tc_unsubscribe`
  that was *not* refused. Refusal is silent (the function returns void), so
  the wrapper snapshots `tc_last_error` before the call and compares after; a
  new error means the barrier did not hold, the ctx stays alive, and
  `unsubscribe` returns `false`. `AppModel.shutdown()` then retries once from
  a fresh plain `Thread` before freeing the handle.
- **Teardown order** is unsubscribe → `tc_daemon_stop` → `tc_handle_free`, all
  on the main thread, which is a plain thread with no tokio context.
- **Every returned `char*`** is copied into a Swift `String` and freed with
  `tc_string_free` in the same function; every borrowed `const char*` from a
  `tc_preview*` is copied before the preview can be closed.

Callbacks arrive on a Rust background thread and do exactly one thing: parse
the frame and `Task { @MainActor in ... }`. Nothing observable is touched off
the main actor.

Nothing under `crates/` was modified.

### 2. Typed layer

`Models.swift` + `DaemonClient.swift`, separate from the pointer file.
`status`, `list_pending`, `preview`, `approve`, `cancel`, `dismiss`, `pause`,
`resume`, plus `list_history`, `history_rollup`, `list_projects`,
`consent_options`, `get_settings`, `queue_outcome_counts`. Timestamps decode
with and without fractional seconds (chrono emits them; `.iso8601` alone
rejects them). No type in this layer has a field for a filesystem path.

Two contract details worth recording:

- **`tc_subscribe` never receives a `snapshot`.** The contract's
  "`subscribe` sends a full `snapshot` first" is a property of the socket
  connection loop, which sends it to the client that just connected. An FFI
  subscriber attaches to the event bus directly and gets no courtesy frame, so
  first paint comes from an explicit `list_pending` + `status`.
- **`queue_depth` lives on `status`, and `queue_changed` does not imply
  `status_changed`.** A status fetched at launch stays at 0 forever unless the
  client refetches it on a queue change. It does now.

### 3. Menu bar

Icon precedence attention → unhealthy → paused → idle; the badge counts
**decisions owed** (`state == pending`), not sessions found and not queue
total. The menu lists what is waiting per project with sizes as inert lines,
the armed-projects row (never collapsed), the week summary, pause
(1 hour / tomorrow morning / until turned back on, via `pause {until}`),
Review, Open, and Quit.

### 4. Queue and preview

Queue row: `project_label`, agent, when, the redacted opening prompt, the
would-send size, the redaction receipt, and the always-visible residual-risk
line. Its forward action is **Look inside**; the other is "Not this one" with
the spec's tooltip.

Preview sheet: Search first and focused, then What's in it, Exactly what would
be sent, Permissions. Search calls `tc_preview_search` and treats the results
as UTF-8 byte offsets, cutting context out of the byte array rather than from
Swift character indices. Recent searches persist locally. `Contribute` lives
here and nowhere else, and advances to the next entry in the sheet.

### 5. History and Settings

History renders the three groups, quarantine as held-not-rejected with no
turnaround number and `explanations` verbatim, and the credit paragraph with
pending and final separated and `last_refreshed_at: null` shown as "Not synced
yet". Settings renders connection, the `consent_options`-driven scope list with
nothing pre-checked and `public_attribution` visually separated, watching
parameters, and project modes.

## Verification

Everything below was run on this machine, in this worktree.

```
$ swift build                                   # macos/
Build complete!
$ ./scripts/make-app-bundle.sh
built .../macos/.build/TraceCommons.app
$ TRACE_COMMONS_SCREENSHOT_DIR=... TRACE_COMMONS_SELFTEST_OUT=... ./scripts/run-demo.sh
state dir: /tmp/tcapp-Rsb0pd
$ RUSTFLAGS='-D warnings' cargo check --workspace --bins
Finished `dev` profile ... in 1.34s
```

The app launched, took the daemon lock, created `daemon.sock`, discovered both
fixture sessions, queued them, and answered every call the UI makes. The
fixtures live in a `mktemp` directory with `claude_root`/`codex_root` written
into `daemon-settings.json` before start; the real `~/.claude` and `~/.codex`
were never in scope, and the app **refuses to start at all** if those roots are
not declared (`DaemonHost.resolveConfigDirectory`).

`docs/images/macos-shell-selftest.txt` is the machine-written record of one run
driving the typed layer against that live daemon:

```
decisions owed (badge): 2
  waiting: payments-api count=1 bytes=918
  waiting: dotfiles count=1 bytes=426
entry: project=payments-api agent=Claude Code state=pending
socket preview: would_send=3049 raw=918 events=3
redaction receipt: scrubbed: 2 secret, 1 secret:aws access key, 1 secret:github token
ffi preview body bytes: 866
search Northwind -> 2 match(es) at byte offsets [173, 791]
search absent-string -> 0 match(es)
redacted body contains raw AWS key: false
redacted body contains raw GitHub token: false
after pause: paused=true
after resume: paused=false
```

The planted AWS key and GitHub token were both scrubbed out of the body the
sheet displays, and the planted client name was found twice by the same search
the contributor uses.

### Screenshots

- `docs/images/macos-shell-menu-bar.png` — the menu-bar item with its `2`
  badge and the menu: what is waiting per project with sizes, the week
  summary, Review, Open, Quit.
- `docs/images/macos-shell-window.png` — the queue with both fixture sessions,
  their redacted opening prompts, would-send sizes and real redaction receipts.
- `docs/images/macos-shell-preview-sheet.png` — the preview sheet showing
  `2 matches` for a real search over the redacted body, with Contribute in the
  footer.

**How they were taken, stated plainly.** The desktop session was locked for the
whole session, so `screencapture` photographs the login screen and
`cacheDisplay` returns blank — nothing is being composited. These are
`ImageRenderer` rasterizations of the shipping view hierarchy bound to the live
daemon (`DebugScreenshot.swift`, inert unless `TRACE_COMMONS_SCREENSHOT_DIR` is
set). They are the real views and real data, not mock-ups. `ImageRenderer`
cannot rasterize `ScrollView`, `List`, `NavigationSplitView`, `TextField` or a
segmented `Picker`, so those appear as SwiftUI's yellow placeholder or as blank
space in the images; the surrounding real content renders. A conventional
screenshot on an unlocked session would show all of it and is the one piece of
verification still owed.

## Findings worth acting on

1. **The five-second undo can lose its race.** `cancel` only works while the
   entry is `approved`, and the uploader picks approved entries up
   immediately. In the self-test the entry had already moved to `failed`
   (`claim-mint-failed`, offline) before the five seconds elapsed, so `cancel`
   returned `not-cancelable`. Online, an upload could *succeed* inside the
   window, which would make the Undo button a lie. **The daemon needs a short
   post-approval hold before dispatch for this affordance to be honest.** The
   shell now says "Too late to undo -- this one had already left the waiting
   list" instead of showing a raw label, but that is damage control, not a fix.
2. **The queue row's `Contribute` button from the shared spec is not built.**
   The brief's stricter rule — preview-then-approve only — was followed, so
   the only approve control is inside the sheet. The two documents disagree;
   this shell follows the stricter one.
3. **The shared spec's quit copy does not survive the macOS process model.**
   "The background watcher keeps running" is written for a shell with a
   separate daemon. On macOS the app *is* the daemon, so the confirmation says
   what is actually true: quitting stops the watcher, the queue stays on disk,
   nothing is sent while nobody is approving.
4. **`preview` requires an enrolled config** (`load_config` → `not-logged-in`)
   even though it does no network I/O. The fixture launcher writes a local
   `contributor.json` for that reason. An app-only contributor cannot see a
   single preview before enrolling, which is worth revisiting given onboarding
   ends with "Nothing has been sent."
5. **`tc_daemon_start` still cannot set the session roots.** The workaround
   (pre-seeded `daemon-settings.json`) is kept, and the app fail-closes rather
   than defaulting to the real home directories. Adopt
   `tc_daemon_start_with_settings` on the next rebase.

## Remaining work

- Onboarding (six screens), including `enroll`, `set_consent_scopes` and
  `acknowledge_near_ai_notice`. Settings currently renders consent read-only
  because changing it needs an enrollment.
- Login item (`SMAppService.mainApp.register()`), notarization, hardened
  runtime, DMG packaging. Out of scope by instruction.
- Arming a project (`set_project_mode: auto_upload`) with its confirmation
  flow and first-auto-upload notification.
- Withdraw from History — no method exists in v1_1; the button is present and
  disabled rather than hidden.
- The health banner's `Reconnect` / `Review and confirm` actions, which lead
  into onboarding surfaces that do not exist yet.
- Notification delivery was wired but not observed firing: `digest_due` needs
  a four-hour interval to elapse.
- A conventional desktop screenshot on an unlocked session.
