# macOS `set_project_mode`: client wrapper, honest wiring, and a contract gap

Scope: add a `DaemonClient.setProjectMode` wrapper for the daemon's
`set_project_mode` (`docs/contributor-daemon-ipc-v1_1.md`), wire onboarding
screen 5 (`OnboardingProjectsView`) to call it instead of discarding the
contributor's "Ignore" choice as local-only state, handle the failure case
honestly, and check whether Settings can already change a project's mode.

## What was built

- `macos/Sources/TraceCommonsApp/DaemonClient.swift` — added
  `setProjectMode(projectKey:mode:)`, following the shape of every other
  `v1_1` wrapper in the file (`rawResult`, thrown `Failure`). It sends
  `project_key` and `mode` only; it never sends `label` — the contract says
  the daemon accepts a `label` parameter from old clients and always ignores
  it, deriving every label itself, and the contract's own rationale (this
  used to be a stored-verbatim injection path into `list_projects` and
  `daemon-audit.jsonl`) is reason enough not to reintroduce sending one from
  a new client either.
- `macos/Sources/TraceCommonsApp/AppModel.swift` — added
  `setProjectMode(_ project: ProjectRow, mode: ProjectMode)`, using the
  existing `perform(_:work:onSuccess:)` plumbing (same pattern as `approve`,
  `dismiss`, `pause`). On success it calls `refreshProjects()` so the UI
  reflects the daemon's own answer; on failure it lands in
  `lastActionError`, same as every other action. It deliberately does not
  flip any local state optimistically.
- `macos/Sources/TraceCommonsApp/Views/OnboardingProjectsView.swift` —
  removed the local `@State private var ignored: Set<String>` and the
  `onContinue(Set<String>)` callback shape entirely. Each row now reads
  `project.mode` straight from `model.projects` (the daemon's own state) and
  the `Ignore`/`Ignored` button calls `model.setProjectMode(project, mode:)`
  for real. `model.lastActionError` is rendered inline on the screen so a
  refusal is visible where the decision was made, not just on some other
  tab. `onContinue` is now a plain `() -> Void`, since there is no longer a
  locally-held set to hand back — the daemon is the source of truth once
  `Ignore` is tapped, exactly as it should be.
- `macos/Sources/TraceCommonsApp/Views/SettingsView.swift` — Settings could
  only *display* `ProjectMode.mode` (`modeSentence`, read-only) before this
  change; there was no way to change one short of the CLI. Added an
  `Ignore` / `Ask again` button per project row (not `auto_upload` — arming
  automation is explicitly still gated behind "a deliberate confirmation
  flow" that the pre-existing note already says is not built), wired to the
  same `AppModel.setProjectMode`, with the same inline `lastActionError`
  surfacing as onboarding. So: **Settings could not change a project's mode
  before this change; it can now, for the `ask`/`ignore` half.**

Nothing under `crates/` was touched.

## The contract gap this surfaced (stop-and-report, not worked around)

`set_project_mode` requires a `project_key`, and the contract is explicit
about what one must be: the `unknown-project` sentinel, a key the daemon
already knows (from a queued session or its own policy file), or an
absolute path that exists on this machine and canonicalizes to itself.

Nothing on the wire ever gives a socket client one of those. Confirmed by
reading the implementation, not just the doc:

- `list_projects` (`crates/trace-commons-contributor/src/daemon/ipc.rs`)
  emits `project_label`, `mode`, `added_at` only — no key.
- `list_pending`'s queue entries emit `project_label` only; `entry_value()`
  in `ipc.rs` has a comment saying `path` and `project_key` are
  "deliberately absent."
- The C ABI (`macos/Sources/CTraceCommons/include/trace_commons.h`) exposes
  no separate project-key accessor either — `tc_daemon_call` is the same
  JSON-in/JSON-out surface the socket uses, so an in-process app gets
  nothing extra.
- The CLI's own `daemon project --mode <mode> <path>` command works
  *only* because a human types the real filesystem path on their own
  terminal; `resolve_project_key` in `crates/trace-commons-contributor/src/commands.rs`
  resolves that human-supplied path against the local filesystem before
  ever calling `set_project_mode`. A GUI app has no equivalent input.

Worse than the missing key: `list_projects` only returns
`policy.projects` — projects that already have a **stored mode** — and the
only code path that inserts into `policy.projects` is `ProjectPolicy::set_mode`,
called solely from the `set_project_mode` IPC handler. So a freshly
discovered project (queued sessions exist, `list_pending` shows them) never
appears in `list_projects` at all until `set_project_mode` has already
succeeded for it once. This is a closed loop for a socket-only client: you
cannot populate `list_projects` without calling `set_project_mode`, and you
cannot call `set_project_mode` without a `project_key` you can only get from
a source `list_projects` (or anything else on the wire) never provides.

Per this task's constraints, `crates/` is frozen and the daemon/contract is
not mine to change here, so I did not invent a workaround (e.g. sending
`project_label` in the `project_key` field and hoping, or reading
`~/.claude` from inside the app to reconstruct a path the contract
deliberately never sends). The code above is the honest version of what's
possible today: it makes the real call with the only identifier the app
has (`project_label`), and it surfaces — rather than swallows — the refusal
that call is guaranteed to get for any project a contributor could actually
see in this screen. That refusal is safer than the old behavior (silently
discarding the choice while showing "Ignored"), which is the actual bug
this task was about; it is not the same as the feature working.

**What the daemon/contract side would need**, if this is picked up as
follow-on work outside this task's scope: a stable, non-path project
identifier that crosses the socket (e.g. an opaque `project_id` alongside
`project_label` in both `list_projects` and queue entries), accepted as an
alternative to a raw path in `set_project_mode`'s admissibility check, and
`list_projects` reporting discovered-but-unconfigured projects (mode
`notify_only`, per `ProjectPolicy::resolve`'s default) rather than only
ones with a stored entry.

## Verification

- `swift build` (from `macos/`): `Build complete!` (only pre-existing,
  unrelated `Sendable` warnings in `AppModel.shutdown()`'s retry-thread
  closure, present before this change).
- `RUSTFLAGS='-D warnings' cargo check --workspace --bins`: `Finished` dev
  profile — Rust side untouched and green.
- `./macos/scripts/make-app-bundle.sh` run explicitly (not left to
  `run-demo.sh`'s missing-bundle shortcut), then
  `TRACE_COMMONS_SHOW_WINDOW=0 ./macos/scripts/run-demo.sh`, state dir
  `/tmp/tcapp-lFSUfQ`, fixture projects `payments-api` and `dotfiles`
  (`payments-api`'s fixture cwd is `/tmp/tcdemo/payments-api`, which does
  not exist as a real directory — only the fixture's `cwd` field claims it).

  **Before** (queue populated, `list_projects` empty — the closed-loop gap,
  confirmed live before touching anything):

  ```
  $ trace-commons-contributor daemon pending --json | jq '[.pending[].project_label]'
  ["payments-api", "dotfiles"]

  $ trace-commons-contributor daemon projects --json
  {"projects": []}
  ```

  **The exact call the new code path makes** (`DaemonClient.setProjectMode`
  wire shape reproduced against the live socket at
  `/tmp/tcapp-lFSUfQ/daemon.sock`, since screens 2/3/4 and the onboarding
  flow root that would chain to screen 5 are not wired into the shipped
  app's navigation yet — confirmed by grep, `OnboardingProjectsView` is
  currently only reachable from the `DebugScreenshot` static-render hook —
  so clicking through the running window cannot reach this screen; this
  reproduces the identical JSON `rawResult` sends, over the same socket,
  against the same daemon):

  ```
  $ python3 - <<'EOF'
  import socket, json
  s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
  s.connect("/tmp/tcapp-lFSUfQ/daemon.sock")
  req = {"id": 1, "method": "set_project_mode",
         "params": {"project_key": "payments-api", "mode": "ignore"}}
  s.sendall((json.dumps(req) + "\n").encode())
  print(s.recv(4096).decode())
  EOF
  {"id":1,"error":{"code":"bad_params","message":"project-key-unrecognized"}}
  ```

  **After** (unchanged — the refusal did not silently pretend to succeed):

  ```
  $ trace-commons-contributor daemon projects --json
  {"projects": []}

  $ trace-commons-contributor daemon pending --json | jq '[.pending[].project_label]'
  ["payments-api", "dotfiles"]
  ```

  This is exactly the honest behavior the new `AppModel.setProjectMode` /
  `OnboardingProjectsContent` / `SettingsView` code implements: the call is
  real, the failure is real, and nothing in the UI or the daemon's state
  moves as if it had succeeded.

  **Positive control**, to confirm the daemon mechanism itself is sound and
  the only blocker is the missing wire identifier: created the real
  directory `/tmp/tcdemo/payments-api` and used the CLI's path-resolving
  `daemon project` command against the same state dir/socket:

  ```
  $ mkdir -p /tmp/tcdemo/payments-api
  $ trace-commons-contributor daemon project --mode ignore /tmp/tcdemo/payments-api --json
  {"ok": true}

  $ trace-commons-contributor daemon projects --json
  {"projects": [{"project_label": "payments-api (9836)", "mode": "ignore",
                 "added_at": "2026-08-08T23:58:11.021022Z"}]}
  ```

  Confirms `set_project_mode` and `list_projects` work correctly end to end
  once a valid `project_key` is supplied — the gap is specifically that no
  socket/GUI client can ever produce one.

## Answer to "can Settings already change a project's mode"

No — before this change, `SettingsView.projects` only rendered
`modeSentence(project.mode)` and said outright "Arming a project ... is not
built yet," with no button of any kind, not even for `ignore`. This change
adds the `ignore`/`ask` toggle there, wired the same honest way as
onboarding. `auto_upload` remains unreachable from either screen, per this
task's constraint and the pre-existing note that a confirmation flow for it
does not exist yet.
