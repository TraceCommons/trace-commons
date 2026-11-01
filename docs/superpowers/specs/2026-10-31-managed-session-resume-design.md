# Resume managed sessions (Claude Code, Codex, OpenCode)

Status: design, agreed 2026-10-31. Scope of the first PR: daemon, CLI, FFI
method list, and the macOS app. GTK, Windows and Tauri follow; every new
field is additive.

## Problem

A session started from Manage sessions runs in an isolated per-account
profile (`NativeProfiles::profile_root`). Its conversation lives there, not
in `~/.claude`, `~/.codex` or `~/.local/share/opencode`, so the person cannot
run `claude --resume` / `codex resume` / `opencode --session` from their own
terminal and find it. Inside the app, an ended row offers only **Start
again**, which replays the launch with a fresh conversation. The launcher
also refuses resume flags in saved CLI options (`managed/options.rs`), on
purpose: session identity belongs to the launcher.

## What already exists

| Tool | Native conversation id | Where history lives | Resume argv |
|---|---|---|---|
| Claude | the row id: launch passes `--session-id <row.id>` (`supervisor.rs`) | account profile `projects/`; collection copies, never moves (`native_collection.rs`) | `--resume <root id>` |
| OpenCode | `ses_<row.id.simple()>` (`opencode.rs`) | `<account root>/<row.id>/`, kept after exit | same command, `--session ses_<root>`, **skip `prepare()`** (it imports an empty seed) |
| Codex | Codex's own thread id, recorded by the SessionStart hook into `managed-native-bindings/<row.id>/` (`session_binding.rs`) **only when collection is on** | account profile `sessions/` | `codex resume <thread id>` plus the launch's `-c` overrides |

## Design

### Identity

A resumed launch is a **new row** (new `SessionView.id`, new ticket, new
proxy for a NEAR AI sign-in session) that continues an earlier conversation.

- `LaunchRequest.resume_of: Option<Uuid>` (`#[serde(default)]`,
  skipped when None): the **root** row whose native conversation this
  continues. Resuming a resumed row resolves to that row's `resume_of`, so a
  chain always points at the root and the native id never drifts.
- The supervisor derives the native id from the root:
  - Claude: `--resume <root>`; do **not** pass `--session-id`.
  - OpenCode: `OpenCodeSession::new(..., root, ...)` so the same per-session
    root and `ses_<root>` id are used; skip `prepare()`.
  - Codex: read the root's `BoundSession.session_id`; build
    `codex resume <thread id>` with the same profile isolation and `-c`
    overrides as a fresh launch. Verify subcommand/flag ordering against the
    installed Codex.

### Codex always records its thread id

Install the SessionStart binding hook on **every** Codex coding launch, not
only collected ones. It records `{session_id, transcript_path}` and changes
nothing else. Collection still decides separately whether anything is
staged. When `configure()` refuses (the person passed their own CLI hooks),
the launch proceeds and that row is simply not resumable. The binding
directory must survive exit and collection. A resume must read the root's
existing binding, never `prepare()` it again (that returns `Conflict`).

### Daemon

- `SessionView.can_resume: bool` (`#[serde(default)]`), computed in
  `managed_snapshot` next to `can_start_again`: all of `can_start_again`'s
  conditions, plus the native id is known (Claude and OpenCode: always;
  Codex: root binding file exists and parses).
- New IPC method `managed_session_resume {session_id, request_id}`,
  handled exactly like `managed_session_start_again` (same replay, account,
  folder and generation checks, same NEAR AI sign-in gating), through a new
  `SessionRegistry::resume_request` that sets `resume_of` to the root.
  Add it to the method allowlist in `daemon/ipc.rs` and the FFI ABI test.
- `capabilities.resume` (or the existing capabilities object's equivalent)
  so a shell offers Resume only to a daemon that has it.

### Trace collection on a resumed session

A resumed row inherits the root's `collect_traces`. It never upgrades from
off to on: that would upload earlier turns the person did not agree to
share. If the root collected, re-collecting the continued transcript is
covered by the original consent. If the existing exact-new-file staging
rules cannot stage an appended transcript, the resumed row reports
`collection: off` honestly rather than failing; improving that is a
follow-up.

### Collect traces even when the session is killed

Today collection runs only inside the launcher (`supervisor.rs`) after the
tool exits normally. If the launcher itself dies (terminal window closed,
force quit, logout), the daemon lapses the row after `LEASE` (45 s) and
nothing is collected. Two layers fix this, for fresh and resumed sessions
alike, and only when the row has `collect_traces`:

1. **Launcher survives hangup.** The launcher handles SIGHUP, SIGTERM and
   SIGINT: it forwards the signal to the tool's process group, keeps
   waiting for the tool to exit, then collects and reports as usual (best
   effort; stderr may be gone). A tool killed by a signal already reaches
   the normal collect path (exit code 128+signal).
2. **Daemon fallback.** At launch the launcher persists a private
   collection plan in the config dir (row id, tool, account, root for a
   resume, cwd, Claude's before-snapshot, Codex binding directory, OpenCode
   session root and version, the tool's pid and process start time) and
   reports the pid in its Running heartbeats. When the sweep lapses a row
   whose plan exists, or the daemon starts and finds such rows, the daemon
   waits until that pid is gone (checking start time to rule out pid reuse),
   then runs the same collection code from the plan and records `Collected`
   or `Failed`. A plan is removed once collection has a final result. The
   launcher and the daemon must never both stage the same row: whoever takes
   an exclusive claim on the plan first collects.

The same verification rules apply as for a normal exit: only the bound or
newly created conversation (or the resume root's file), confined to the
profile.

### CLI and "Copy resume command"

- `trace-commons-contributor [--config-dir <dir>] sessions resume <session-id>` calls
  `managed_session_resume` and opens the session the way `managed launch`
  does.
- The macOS row's **Copy resume command** copies that exact line (using the
  installed binary path and `--config-dir` the app uses), so the person can
  resume from any terminal with the right isolated profile.

### macOS

- `ManagedSession.canResume` decoded with `decodeIfPresent ?? false`.
- Ended-row actions: **Resume** (primary) when `canResume && terminalLaunch`,
  then Start again, then Dismiss; **Copy resume command** in the row's menu
  when `canResume`.
- Copy strings come from `managed/copy.rs` (`resume`, `copy_resume_command`),
  like `start_again`.
- Update the `RecordedSamples/*/tool_destinations.json` fixtures and
  `tests/fixtures/managed/snapshot.json`.

## Tests (write first)

- `sessions.rs`: `resume_request` sets `resume_of` to the root; resuming a
  resumed row keeps the root; refused while the row holds its profile; refused
  for a login row; inherits `collect_traces` and cannot turn it on.
- Supervisor argv per tool: Claude has `--resume <root>` and no
  `--session-id`; OpenCode uses `--session ses_<root>` and does not run
  import; Codex builds `resume <thread id>` with the isolation overrides.
- Codex binding is configured when `collect_traces` is false.
- `daemon/managed.rs`: `can_resume` true/false matrix (folder gone, account
  gone, Codex without binding); `managed_session_resume` replays on retry.
- Options validation still refuses resume flags in saved CLI options.
- Swift: decode `can_resume` present/absent; action list ordering.

## Out of scope

GTK, Windows, Tauri shells; resuming sessions the person started outside the
app; changing collection staging rules.

## As built (deviations from the design above)

- CLI binary is `near-ai`: `near-ai [--config-dir <dir>] sessions resume <id>`.
- `resume_of` lives on the registry entry next to the stored request (like
  `keep_saved_model`), not inside `LaunchRequest`, so a downgraded daemon can
  still open the registry. `managed_launch_prepare` refuses a request naming
  `resume_of`. The launcher learns the root via `LaunchContext.resume_of`.
- The always-on Codex hook changes one thing: hook overrides make Codex
  0.164+ start its embedded server. Codex rows are never resumable on
  Windows (the hook is Unix-only, pre-existing).
- `can_resume` also requires: Claude, a regular `projects/<dir>/<root>.jsonl`
  in the profile; OpenCode, the `<profile>/<root>/` directory; Codex,
  exactly one recorded binding (none after `/new`). Refused when the tool
  never started, and while any session of the same conversation is still
  running (one live session per conversation).
- Start again on a resumed row starts a fresh conversation (`resume_of`
  cleared).
- Resumed rows collect when the root collected. They stage the whole
  current transcript under the new row id with the root's identity. No
  dedupe by conversation identity exists (receipts key on content hash), so
  earlier turns appear in each copy; withdrawal by identity covers all.
- Saved Codex options refuse `resume`/`fork`/`--last` only in subcommand
  position; `options::snapshot` no longer fails the panel on a saved value
  this build refuses (launch still refuses it).
- Killed-session collection is implemented as specified
  (`managed/collection_plan.rs`, plans in `managed-collection-plans/`, 0600,
  claim by hard link). The daemon fallback trusts the plan's OpenCode
  executable path (re-resolving deferred: the daemon's PATH may not see the
  terminal's tool); other plan fields are checked against the registry row.
- UI as built: Resume primary; Start again, Copy resume command and
  Dismiss are items in the row menu (Dismiss stays a link only when it is
  the row's sole action); after Copy resume command the row's status reads
  "Copied" with a neutral dot for 2 s;
  resumed rows show "Resumed · Continues <time>" (`SessionView.resumed_from`,
  stored at prepare time).
- New OpenCode sessions on NEAR AI are titled "NEAR AI private session"
  (shown as `OC | NEAR AI private session`); others keep "Managed session".
