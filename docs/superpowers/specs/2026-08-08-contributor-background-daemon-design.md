# Contributor background daemon — design

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 1 of 4 (daemon core). Native tray shells are separate specs.

## Problem

`trace-commons-contributor` is a one-shot CLI. A contributor only submits traces
when they remember to run `submit`, and they have no ambient sense of which
local sessions are even eligible. We want a background app that tells the user
which traces they could upload, lets them upload with one action, and can be
programmed to upload selected projects autonomously.

## Decomposition

The full product is four independently buildable pieces:

1. **Daemon core** (this spec) — discovery, eligibility, per-project policy,
   queue, upload, IPC contract, CLI control. Useful headless, with no GUI.
2. **macOS menu-bar shell** (SwiftUI)
3. **Windows tray shell** (WinUI)
4. **Linux tray shell** (GTK)

Shells 2–4 are pure clients of the IPC contract frozen at the end of this
sub-project, and can be built in parallel afterwards. They are out of scope
here.

### Surfaces each shell presents

Every shell presents **two** surfaces, which is what the contract must serve:

- **Menu bar / tray — glance and act.** Pending count badge, the few ready
  traces with one-click approve, pause/resume, open-window. Served by
  `status`, `list_pending`, `approve`, `pause`/`resume`, and the `subscribe`
  stream.
- **Main window — manage and review.** Full queue with `preview`, contribution
  history and credit rollup, per-project auto-upload settings, health and login
  state. Served by `list_history`, `history_rollup`, `list_projects`,
  `set_project_mode`, `get_settings`/`set_settings`.

The window uses strictly more of the contract than the tray; no method exists
solely for one surface. This split is recorded here so the v1 freeze is not
made against the tray alone.

## Decisions already fixed

- Three native shells over one shared Rust daemon.
- Autonomy is **per-project opt-in**; no global "upload everything" switch.
- Eligibility is **quiescence-based**, with bounded re-upload on growth.
- Notification is a **batched digest** over a **durable queue**.
- Unapproved queue entries **expire after 14 days**.

## Placement

The daemon lives in the existing `trace-commons-contributor` crate as a
`daemon` subcommand tree — not a new crate. New files under `src/daemon/`:
`watcher.rs`, `eligibility.rs`, `policy.rs`, `queue.rs`, `uploader.rs`,
`notify.rs`, `ipc.rs`, `install.rs`, `health.rs`.

### submit.rs seam

`submit_sessions` (`src/submit.rs:108`) is already non-interactive and takes
`Vec<(Box<dyn TraceSource>, SessionRef)>`. All prompting lives in
`commands.rs` (picker at `commands.rs:498`, consent at `commands.rs:111`).

It is **not** a single-session seam: claim minting (`submit.rs:225`) and the
canary self-test (`submit.rs:180`) are hoisted across the batch. Extracting
naively would re-mint a claim and re-run the canary per trace.

Extract a long-lived `SubmitContext` holding `device`, `issuer`,
`claim: Option<ClaimToken>`, `canary_checked_at`, and a receipts index, with
`submit_one(&mut self, source, session_ref) -> SubmitOutcome`. The existing
batch path is reimplemented as a loop over `submit_one` so CLI and daemon
share one pipeline. The three `println!` sites in `submit.rs` (~113, ~213,
~236) move behind a sink the daemon can route to IPC.

## Pipeline

### Watcher

Polls the claude-code and codex source roots every 60s. No filesystem-notify
crate: the quiescence window is 30 minutes, so poll resolution is free.

The **trajectory source is not watched** — trajectory files have no
conventional local store (`source/mod.rs:102`). It remains CLI-only via
`submit --trajectory`.

`SessionRef` (`source/mod.rs:19`) carries no modification time, so the watcher
`stat`s paths itself rather than changing the public struct.

`ClaudeCodeSource::discover` calls `peek_cwd` (`source/claude_code.rs:220`),
which reads each session file line-by-line every poll. The daemon caches
`(path, size, mtime) -> cwd` in `daemon-state.json` to avoid continuous disk
churn.

### Eligibility

A session is eligible when **both** hold:

- `now - mtime >= quiescence_window` (default 30 min, configurable), and
- `size` is unchanged across two consecutive polls.

`session_hash` is computed at eligibility, never at discovery, so the watcher
never reads whole files on a normal poll. A queue entry therefore always has
a hash.

**Growth re-queue is bounded.** `session_hash` covers whole file bytes
(`source/mod.rs:69`) and `submission_id_for` is a v5 UUID over it, so any
append re-uploads all prior content. Unbounded re-queue would pay the NEAR AI
filter bill repeatedly over the same text and create near-identical envelopes
that server-side simhash clustering collapses with `dup_pen = 1/size`,
diluting the contributor's own credit. A grown session re-queues only when:

- growth is material — `>= 2x` bytes **or** `>= 64 KiB` of new bytes (both
  configurable), **and**
- the session has been uploaded fewer than `max_reuploads` times (default 3).

`daemon-state.json` holds a `path -> {last_uploaded_hash, size, upload_count}`
index. Receipts are keyed by `session_hash` only, so this index cannot be
derived from `receipts.jsonl`.

### Policy

Per-project opt-in, keyed on the session's true `cwd`. Modes:
`auto_upload | notify_only | ignore`. Unknown projects default to
`notify_only` — nothing leaves the machine unreviewed until explicitly opted
in.

Sessions with **no resolvable cwd** — Claude Code subagent transcripts
(`source/claude_code.rs:94`), trajectory sessions (`source/trajectory.rs:254`)
— fall into a distinct `unknown-project` bucket that is permanently
`notify_only` and **cannot** be set to `auto_upload`. They never fall back to
the unreliable basename heuristic at `commands.rs:217`.

`auto_upload` goes straight to the uploader. Everything else lands in the
queue.

### Uploader

Reuses `SubmitContext::submit_one`, so redaction, PII filtering, claim
minting, and upload are byte-for-byte the CLI path. Appends to the existing
hash-only `receipts.jsonl`.

**Re-hash before upload.** The queue records `session_hash` and size at
eligibility; the user may approve hours later (digests batch at 4h). Between
notification and approval the session can grow, so the uploader re-loads and
compares. On mismatch it **refuses to upload**, marks the entry `superseded`,
creates a fresh `pending` entry at the new hash, and re-notifies. The user
must never approve a 42 KB description and have 900 KB shipped. This is the
design's central consent property.

**Volume caps** (all configurable, all enforced before the filter is called):
`max_uploads_per_day` (default 50), `max_bytes_per_day` (default 200 MB),
`max_queue_entries` (default 500). Hitting a cap sets a health state and
pauses uploads until the window rolls; it never drops entries.

### History poller

A contributor who lets the daemon upload while they are away needs to see what
went out and what it earned. The server already returns everything needed:
`status`, `consent_scopes`, `credit_points_pending`, `credit_points_final`,
`explanation`, and `delayed_credit_explanations` per submission
(`commands.rs:629`, via `submit::status`).

The daemon polls that endpoint on a timer (default 30 min, and once shortly
after each upload), joins the result with local receipts, and caches to
`daemon-history.jsonl`. One poller serves all surfaces rather than three shells
each polling the server, and history stays readable offline.

A history record holds: `submission_id`, `submitted_at`, `project_label`,
`source`, `session_hash`, `status`, `consent_scopes`,
`credit_points_pending`, `credit_points_final`, `explanations`,
`last_refreshed_at`. It carries **no** local `path` — history is the surface
most likely to be screenshotted or shared.

The rollup is computed from that cache: counts by status for this week, this
month, and all time; credit pending versus final; and a quarantined count
surfaced explicitly, since quarantine means "held for operator privacy review",
not "rejected" (`commands.rs:650`) and a contributor who sees only the word
reads it as failure.

Poll failures are non-fatal: the cache is served with its `last_refreshed_at`
so a shell can show staleness rather than an empty table.

### Notifier

The daemon owns the **batching policy** — at most one digest per
`digest_interval` (default 4h) — because that policy is shared by all three
shells. Delivery is a `digest_due` push event over `subscribe`.

A minimal `osascript` / `notify-send` shell-out exists behind an
**off-by-default** `local_notifications` setting, so the daemon is usable
before any shell ships. It is best-effort: unavailable notifier means a
logged label, never a failed upload.

## State files

All in the existing contributor config dir (0700, `config.rs:95`), written
with the existing `write_atomic_0600` (`config.rs:265`) — not a new atomic
writer.

| File | Contents |
|---|---|
| `daemon-projects.json` | project key -> mode, added_at |
| `daemon-queue.jsonl` | queue entries (below) |
| `daemon-history.jsonl` | cached contribution history joined from receipts + server status |
| `daemon-state.json` | watcher cursors, cwd cache, path->last-upload index, last digest time, daily counters |
| `daemon-settings.json` | quiescence window, digest interval, TTL, caps, PII filter settings, local_notifications |
| `daemon.sock` | unix socket (Windows: named pipe) |
| `daemon.lock` | single-instance advisory lock |

Queue entry: `entry_id` (v5 UUID over `session_hash`, stable across restarts),
`session_hash`, `source`, `project_key`, `project_label`, `path`,
`size_bytes`, `discovered_at`, `state`, `reason_label`, `attempts`,
`retry_after`, `submission_id`.

Queue states: `pending | approved | uploading | uploaded | refused | failed |
expired | superseded`.

`path` is **local-only**. It never enters `receipts.jsonl`, never crosses into
any telemetry or server-bound payload. Shells render `project_label`, and the
spec states this explicitly so no shell is tempted to display or log `path`.

### Three ways to say "no", and why each exists

- `ignore` (project mode) is a **standing** decision: this project never
  produces queue entries at all.
- `dismiss` (queue action) is a **per-entry** decision: this one session is not
  worth uploading, but the project keeps being offered.
- `expired` is **inaction**, recorded so the entry is not re-offered.

They are distinct because they answer different questions, and a shell renders
each differently: a settings toggle, a per-row action, and a passive state.

### Expiry

`pending` entries expire to `expired` after 14 days and are not re-offered.
The expiry clock is **suspended** while an entry is blocked on a daemon-level
health failure (PII filter down, not logged in, cap reached) — otherwise an
outage silently drops two weeks of traces.

## Lifecycle and revocation

`ConfigStore::wipe()` (`config.rs:225`) currently deletes four files and knows
nothing about the daemon. As-is, `logout` leaves a daemon holding a cached
claim — valid until 60s before expiry (`issuer_client.rs:35`) against ~300s
tokens (`issuer_client.rs:202`) — still uploading into a receipts file that no
longer exists. Worse, `daemon-projects.json` would survive, so a different
user re-enrolling on the same machine inherits the previous user's
`auto_upload` opt-ins.

Required:

- `wipe()` deletes all seven daemon state files, `daemon-history.jsonl`
  included — contribution history is per-identity and must not survive a
  logout into someone else's session.
- `wipe()` signals the daemon (connect to the socket, send `shutdown`; fall
  back to a revocation marker) and blocks until the lock is released.
- The daemon re-checks `contributor.json` and `device_key_path().exists()`
  immediately before **every** upload, drops any cached claim on change, and
  treats absence as "stop, drain nothing".
- The daemon reloads `ContributorConfig` and invalidates its cached claim
  whenever `contributor.json` mtime changes. `stamp_granted_scopes`
  (`submit.rs:456`) already prefers issuer-granted scopes over local config,
  but only at mint time; without reload, narrowing consent mid-run is ignored
  for a full claim TTL.

### NEAR AI notice

`ensure_near_ai_notice_shown` (`config.rs:211`) fires once and is printed by
`submit_sessions` (`submit.rs:113`). Under a service manager that `println!`
goes to a log nobody reads, and the interactive CLI never shows it again — the
user's traces reach a third party with no notice ever delivered. The daemon
**refuses** to use the `near-ai` filter until the marker already exists, i.e.
until the notice was delivered interactively.

### PII filter configuration

The CLI reads filter settings from process env via `near_ai_settings_from_env`
(`envelope.rs:55`). A systemd user unit inherits none of the user's shell env,
so every entry would fail `pii-filter-unavailable` and silently expire.

The seam already exists: `build_redactor_with` (`envelope.rs:87`) takes
`near_ai: Option<NearAiSettings>` explicitly rather than reading the
environment, precisely so callers can supply settings. The daemon resolves
them from `daemon-settings.json` (0600) and passes them in; no refactor of the
redactor path is required. `daemon install` fails loudly when `pii_filter == Some("near-ai")`
and no persisted key exists.

`canary_self_test_async` (`submit.rs:184`) currently fails the whole batch
call. In the daemon a canary failure is a **daemon-level health state**, not N
per-entry failures, and the canary re-runs on a timer (default 1h) and on
filter-settings change — not once per process lifetime.

## IPC contract — `trace_commons.daemon.v1`

The artifact the three shells depend on. Frozen and documented at the end of
this sub-project.

**Transport.** JSON-lines over a unix domain socket at `$CONFIG_DIR/daemon.sock`.
Windows uses a per-user named pipe; a per-user-restricted pipe ACL requires a
`SECURITY_DESCRIPTOR` and therefore a `windows-sys` dependency, so Windows is
**specified but not implemented** in this sub-project (no Windows shell exists
yet). Implementing it requires dependency approval first.

**Framing.** Every request carries an `id`; every response echoes it. Push
frames carry `event` and no `id`. Max line length 1 MiB; oversize or malformed
lines get an error frame and the connection closes. The server may answer
pipelined requests out of order.

**Methods.** `hello` (capabilities + schema version), `status`, `list_pending`,
`preview`, `approve`, `dismiss`, `pause`, `resume`, `list_projects`,
`set_project_mode`, `list_history`, `history_rollup`, `refresh_history`,
`get_settings`, `set_settings`, `subscribe`, `shutdown`.

- `status` returns `{logged_in, tenant_id, consent_scopes, paused, queue_depth,
  next_digest_at, health: {last_error_label, since}}`. "Not logged in", "claim
  minting failing", "PII filter unreachable", and "ingest unreachable" are the
  four states every tray needs and must be expressible.
- `preview <entry_id>` returns the redacted envelope summary — event count,
  byte size, redaction counts — for an entry. `SubmitOptions.dry_run`
  (`submit.rs:61`) already produces this. For an app that autonomously uploads
  real coding sessions, "show me what would be sent" is the core consent
  affordance.
- `list_history` is paginated (`before`, `limit`) and returns records with
  `last_refreshed_at` so a shell can render staleness. `history_rollup` returns
  the week/month/all-time counts, pending-vs-final credit, and quarantined
  count. `refresh_history` forces a poll, for a window's pull-to-refresh; it is
  rate-limited and returns `busy` rather than queueing.
- `subscribe` sends a full snapshot first (so a tray never races
  `list_pending` against the stream), then deltas, plus `resync` after daemon
  restart. Slow clients get drop-oldest and a `resync_required` event.
- Error taxonomy: `unknown_method`, `bad_params`, `not_authorized`, `busy`,
  `unavailable`.

**Authorization.** Filesystem ownership, with one carve-out. The socket is
*not* merely a peer of the device key: stealing `device.pk8` lets an attacker
upload once, whereas the socket lets them call `set_project_mode(auto_upload)`
and have the legitimate, user-blessed daemon exfiltrate every future session
in that project continuously, under the user's real grant, with normal-looking
receipts. The CLI cannot do this because it requires a TTY
(`commands.rs:498`).

Therefore `set_project_mode -> auto_upload` and `approve --all` are **TTY/CLI
only** in v1 and return `not_authorized` over the socket. Shells surface the
CLI command to run. Additionally, `UnixListener::bind` does not portably set
socket mode: the 0700 config dir is the enforcing control, and the daemon
**refuses to start** if that dir is not 0700 and owned by the euid.

## CLI control surface

```
daemon run [--dry-run]
daemon status
daemon pending
daemon preview <entry_id>
daemon approve <entry_id> | --all
daemon dismiss <entry_id>
daemon pause | resume
daemon projects
daemon project <path> --mode auto|notify|ignore
daemon history [--limit N] [--refresh]
daemon settings [--set key=value]
daemon install | uninstall
```

`daemon install` writes a **systemd user unit only** — headless Linux has no
tray and genuinely needs it. macOS and Windows autostart is deferred to the
shells that will own login-item registration; documented plist and Startup
templates ship in the meantime.

The CLI surface is full parity with the future GUIs, so the daemon is
completely usable over SSH.

## Dependencies

**Zero new crates.**

- `tokio` `net` moves from dev-dependencies (`Cargo.toml:34`) to dependencies;
  `sync` (broadcast for `subscribe`) and `signal` (SIGTERM handling) are added.
  Feature additions, not new crates.
- Single-instance locking uses `std::fs::File::try_lock_exclusive`, stable
  since 1.89; the workspace is on 1.92 (`Cargo.toml:13`). No `fs2`/`fd-lock`.
- Notifications shell out; no notifier crate.
- Polling instead of `notify`.

The only dependency that would be required is `windows-sys`, for the named-pipe
ACL. It was deferred here and has since been approved (2026-08-08) for the
Windows shell, scoped to `cfg(windows)` so this sub-project's zero-new-crate
property holds on macOS and Linux.

## Error handling

Fail-closed throughout. A configured-but-unreachable PII filter leaves entries
`pending` with a `reason_label` and a suspended expiry clock; it never uploads
raw. Network failures retry with exponential backoff and an attempt cap, after
which the entry goes `failed` with a label and is retryable by hand. The daemon
never modifies or deletes session files. All state writes are atomic.

## Known concurrency limits

The daemon and a concurrently run one-shot CLI share the config dir. Both can
pass the receipts check (`submit.rs:150`) and upload the same session.
`submission_id_for` is deterministic, so the server should dedupe; the plan
verifies this against the ingest path and, if it does not hold, has the CLI
take the same advisory lock. `daemon.lock` as specified guards only
daemon-vs-daemon.

`load_receipts` is called inside the per-session loop (`submit.rs:150`),
re-parsing the whole log per session. That is fine for a one-shot CLI and
quadratic for a long-lived daemon on an unbounded append-only file.
`SubmitContext` holds an in-memory receipts index instead.

## Testing

TDD per repo convention.

Unit:
- quiescence: mtime window + size-stability, resumed-after-days, held-open-idle
- growth re-queue: below threshold, above threshold, re-upload cap reached
- policy: mode resolution, unknown-project default, `unknown-project` bucket
  cannot be set to auto
- queue state machine: every transition, expiry, expiry suspension under health
  failure, supersede-on-hash-mismatch
- digest batching interval
- volume caps
- history join: receipts joined with server status updates, rollup counts and
  pending-vs-final credit arithmetic, quarantined surfaced separately, stale
  cache served with `last_refreshed_at` when a poll fails, no `path` in any
  history record

Integration:
- IPC round-trip over a tempdir socket: framing, id correlation, subscribe
  snapshot-then-delta, `not_authorized` on socket-side auto-enrollment
- logout-while-running: daemon exits, state files gone, no post-logout upload
- end-to-end auto-upload against a stub ingest, following
  `tests/e2e_enroll_and_submit.rs`

## Out of scope

- The three native shells (sub-projects 2–4).
- Windows named-pipe implementation.
- Delta/continuation uploads — considered and rejected here because the
  envelope and server have no continuation concept; revisit if the re-upload
  cap proves too coarse.
- Any server-side change. This sub-project is client-only.
