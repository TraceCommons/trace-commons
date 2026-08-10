# macOS FFI bridge milestone: Rust C ABI callable from Swift

Status: PROVEN. `trace-commons-contributor-ffi`'s C ABI is callable end to
end from a Swift executable on this machine, built with Xcode 26.6 / Swift
6.3.3, without touching anything under `crates/`.

## What was built

A new top-level `macos/` directory containing a minimal SwiftPM package,
`TraceCommonsFFIDemo`:

```
macos/
  Package.swift
  Sources/
    CTraceCommons/                      # systemLibrary target: exposes the header
      module.modulemap
      include/trace_commons.h           # copy of crates/.../include/trace_commons.h
    tc-ffi-demo/                        # executable target
      TCDaemon.swift                    # the ONLY file with raw pointers (~90 lines)
      main.swift                        # temp-dir setup + hello/status calls
```

Name chosen: `TraceCommonsFFIDemo` (package) / `tc-ffi-demo` (executable
target), matching the crate's `tc_` prefix convention.

`CTraceCommons` is a SwiftPM `.systemLibrary` target whose `module.modulemap`
umbrella-imports a **copy** of `trace_commons.h` (copied, not modified, and
not symlinked into `crates/`, per the constraint not to touch anything
there). The executable target links the Rust build output directly via
linker flags:

```swift
.unsafeFlags(["-L", "../target/debug", "-ltrace_commons_contributor_ffi"])
```

This links `target/debug/libtrace_commons_contributor_ffi.dylib`. `otool -L`
on that dylib shows its own install name is the absolute path under this
worktree's `target/debug/deps/`, so no rpath juggling was needed for the
demo to find it at runtime.

## Header review (step 1)

Read `crates/trace-commons-contributor-ffi/include/trace_commons.h` in full.
Key rules obeyed by `TCDaemon.swift`:

- Every `char*` returned (`tc_call`, `tc_daemon_start`'s `err` out-param) is
  owned by the caller and freed with `tc_string_free` — done via `defer`-like
  discipline (explicit free right after `String(cString:)` conversion, or in
  the `err` branch of the start failure path).
- `tc_daemon_stop` does not free the handle; `tc_handle_free` is the only
  function that does, and must not be called concurrently with
  `tc_daemon_stop`. The demo calls them sequentially on the same thread.
- `tc_handle_free`/`tc_unsubscribe` refuse to run from inside a tokio runtime
  context (e.g. a `tc_subscribe` callback). The demo never subscribes, so
  this doesn't apply here, but is called out in `TCDaemon.swift`'s doc
  comments for future extension.
- `tc_call` never returns NULL — a bad method/params produces a JSON error
  frame. The wrapper treats a NULL defensively anyway but does not throw.

## Session-root safety (step 4)

The C ABI has **no call to set `claude_root`/`codex_root` before
`tc_daemon_start`** — `tc_call`'s `set_settings` (only usable after the
daemon is already running) covers `quiescence_secs` / `digest_interval_secs`
/ `local_notifications` only (see
`crates/trace-commons-contributor/src/daemon/ipc.rs:786-813`). Left at their
default of `None`, the daemon would watch the developer's real `~/.claude`
and `~/.codex` trees.

The Rust integration test (`crates/trace-commons-contributor-ffi/tests/abi.rs`,
`write_tempdir_session_roots`) solves this by calling
`trace_commons_contributor::daemon::settings::DaemonSettings::save` directly
before `tc_daemon_start` — not available to a pure-Swift/C-ABI consumer.

Since this constraint forbids editing `crates/` (and adding a new C export
is exactly the kind of ABI change that needs to go through the frozen
contract, not be added unilaterally here), `main.swift`'s `seedSettings`
instead writes `<config_dir>/daemon-settings.json` directly, in the same
location and shape `DaemonSettings::save` would produce (confirmed by
reading `crates/trace-commons-contributor/src/config.rs`:
`ConfigStore::daemon_path` joins `DAEMON_SETTINGS_FILE = "daemon-settings.json"`
onto `config_dir`, and `crates/trace-commons-contributor/src/daemon/settings.rs`
for the exact field list / defaults), with `claude_root` and `codex_root`
pointed at two empty subdirectories of the same temp dir
(`<config_dir>/claude-root`, `<config_dir>/codex-root`), created before the
daemon starts. This is a file the daemon already treats as untrusted input
(`#[serde(default)]` on both fields) — not a hidden reach into Rust
internals via a private API, just pre-seeding a config file on disk in its
documented format.

This is a genuine finding: **the frozen C ABI cannot itself configure
session roots before start.** Every native host that embeds this daemon
(not just this demo) either has to (a) know and replicate the
`daemon-settings.json` on-disk format the way this demo does, which is
undocumented/unstable surface outside the header, or (b) rely on
`set_settings` after start, which cannot reach `claude_root`/`codex_root`
at all. If real applications need this, the C ABI likely needs either a
`tc_daemon_start_with_settings` variant or an extension to `set_settings`'s
field coverage — but per the task constraint, no such change was made here;
this is reported, not fixed.

## Socket path length (constraint reminder)

`crates/trace-commons-contributor/src/daemon/ipc.rs` enforces
`MAX_SOCKET_PATH_BYTES = 104` on `<config_dir>/daemon.sock`.
`NSTemporaryDirectory()` on this machine resolves to a long
`/var/folders/.../T/` path that would blow this limit once
`/daemon.sock` is appended. `main.swift`'s `makeShortTempDir()` calls
`mkdtemp("/tmp/tccfg-XXXXXX")` instead, keeping the full socket path well
under 30 bytes.

## Commands run and real output

```
$ cargo build -p trace-commons-contributor-ffi
   Finished `dev` profile [unoptimized + debuginfo] target(s) in 37.20s

$ ls target/debug/ | grep trace_commons_contributor_ffi
libtrace_commons_contributor_ffi.a
libtrace_commons_contributor_ffi.d
libtrace_commons_contributor_ffi.dylib
libtrace_commons_contributor_ffi.rlib

$ RUSTFLAGS='-D warnings' cargo check --workspace --bins
    Finished `dev` profile [unoptimized + debuginfo] target(s) in 46.12s

$ cd macos && swift build
Building for debugging...
[7/8] Applying tc-ffi-demo
Build complete! (0.60s)

$ ./.build/debug/tc-ffi-demo
config dir: /tmp/tccfg-vTOZYM
daemon started
hello -> {"id":0,"result":{"events":["snapshot","queue_changed","status_changed","digest_due","resync_required"],"max_line_bytes":1048576,"methods":["acknowledge_near_ai_notice","approve","cancel","consent_options","dismiss","enroll","get_settings","hello","history_rollup","list_audit","list_history","list_pending","list_projects","pause","preview","queue_outcome_counts","refresh_history","resume","set_consent_scopes","set_project_mode","set_settings","shutdown","status","subscribe"],"schema_version":"trace_commons.daemon.v1_1","supported_versions":["trace_commons.daemon.v1","trace_commons.daemon.v1_1"]}}
status -> {"id":0,"result":{"consent_scopes":[],"health":{"last_error_label":null,"since":null},"logged_in":false,"next_digest_at":null,"paused":false,"queue_depth":0,"schema_version":"trace_commons.daemon.v1_1","tenant_id":null}}
daemon stopped and handle freed
```

Re-run 3 times consecutively with identical shape of output and exit code 0
each time (only `config dir` and queue-depth timestamps vary run to run).

Verified afterwards that no real config directory was touched
(`~/.config/trace-commons`: does not exist / unaffected) and every
`/tmp/tccfg-*` temp dir was removed by the demo's own cleanup at exit.

`git status --porcelain` after the run shows only the new `macos/`
directory; `git diff --stat -- crates/` is empty — nothing under `crates/`
was modified.

## Files

- `macos/Package.swift`
- `macos/Sources/CTraceCommons/module.modulemap`
- `macos/Sources/CTraceCommons/include/trace_commons.h` (copy, unmodified content)
- `macos/Sources/tc-ffi-demo/TCDaemon.swift`
- `macos/Sources/tc-ffi-demo/main.swift`

## Not done (out of scope per task)

No menu bar app, no window, no UI. No changes under `crates/`. Stopped after
step 6 as instructed.
