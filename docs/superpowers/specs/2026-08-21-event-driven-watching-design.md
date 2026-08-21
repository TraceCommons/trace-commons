# Event-driven session watching

**Status:** approved for implementation
**Issue:** #388
**Follows:** #374, #381 (both removed per-poll *load* cost; this removes the per-poll *scan* cost)

## Goal

Stop walking the whole corpus every 60 seconds. Learn about session changes
from the operating system instead, and keep a slow reconciliation sweep so a
lost event costs minutes, not forever.

## Why

Measured on a real machine (0.4.6, 3,069 Codex + 81 Claude Code sessions,
11.7 GB, 979 subagent transcripts under 36 `subagents/` directories):

| | |
|---|---|
| idle CPU | 15.6% of a core, continuously |
| per tick | ~4.7 s in `discover`, ~2.6 s loading/parsing/hashing |

`discover` is dominated by `lstat`ing every subagent transcript to recompute
`group_modified_at`. That cost is paid whether or not anything changed, and it
grows with the corpus, forever.

#374 and #381 removed the *load* for sessions nothing could be done with.
Neither can touch the scan, because the scan is what decides whether anything
moved: the group mtime is the very thing "unchanged" is judged on. Making it
cheap requires a different source of truth for "something happened", which is
what the OS already has.

## Design

### The loop today

`daemon::supervise` is a `tokio::select!` over a shutdown signal and one
ticker at `poll_interval_secs`. Each tick runs, in order:

1. `watcher::tick` — the expensive corpus scan
2. `expire_and_digest` — queue TTL and digest scheduling
3. `drain_approved` — the upload pass

Only (1) is driven by the filesystem. (2) and (3) are time-driven and must
keep their current cadence; slowing them would delay uploads.

### The loop after

Three arms instead of two:

- **Fast ticker, unchanged interval.** Runs `expire_and_digest` and
  `drain_approved` only. These are cheap: no corpus walk.
- **Filesystem events.** A debounced stream of changed paths. Each batch runs
  the watcher pipeline **scoped to the affected sessions**, not the corpus.
- **Reconciliation ticker, 15 minutes.** Runs the existing full
  `watcher::tick` unchanged, as it does today.

The reconciliation arm is what makes the change safe to ship: the current
behaviour remains reachable and correct on its own, and the event path is an
optimisation layered over it. If the event path degrades to delivering
nothing, the daemon behaves exactly like today with a 15-minute poll.

### Scoped scanning

`tick_over` currently iterates `source.discover()`. It gains a sibling that
takes an explicit set of session paths and runs the same per-session body:
observe, evaluate eligibility, check the queue, load and hash if warranted.
Everything downstream of the per-session loop (`relabel_queue`, the sweep,
`queue.save`, the `changed` publish) is shared and must not be duplicated.

Mapping an event path to a session is source-specific and belongs on
`TraceSource`, beside `discover`: a Codex event maps to its own rollout file;
a Claude Code event under `<uuid>/subagents/` maps to the parent session,
which is the existing group-address rule and must not be re-derived
independently.

### Quiescence

Today: quiet for `quiescence_secs` **and** size steady across two consecutive
polls. The second condition exists because "mtime granularity and clock skew
both lie, while a changing byte count does not" (`eligibility.rs`).

Events give a better signal than either: the daemon is told when writes
happen. "No event for this session for `quiescence_secs`" is a stronger
statement than two mtime samples agreeing.

`eligibility::evaluate` stays a pure function and keeps its current contract.
The event path supplies `previous_size` from the last observation exactly as
the poll path does, so a session still needs a stable byte count. What
changes is only *when* the daemon looks, not what it decides. This keeps one
eligibility rule for both paths and keeps the existing tests meaningful.

### Debounce

Editors and agents produce many events per logical write. Coalesce in the
daemon: hold a `HashMap<PathBuf, Instant>` of dirty sessions, refresh the
instant on each event, and process a session once it has been quiet for a
short debounce window (proposed: 2 s, distinct from and much shorter than
`quiescence_secs`). Roughly 30 lines; no second dependency.

### Roots and fail-closed behaviour

Watches are registered **only** on declared session roots. This is the same
rule the macOS shell already enforces and that Linux and Windows are being
brought to (see the fail-closed roots work). Event-driven watching makes it
structural rather than advisory: a root that was never declared is a root no
watch is registered for, so it cannot produce events.

A root that disappears (unmounted, deleted, permissions revoked) must
deregister its watch and raise a health label, not silently stop.

### Platform reality

`notify` selects FSEvents on macOS, inotify on Linux, and
ReadDirectoryChangesW on Windows.

**Linux watch exhaustion is the failure mode to design for.** Recursive
inotify consumes one watch per *directory*; `fs.inotify.max_user_watches`
commonly defaults to 8192. A large corpus can exhaust it. When registration
fails for this reason the daemon must:

1. raise a distinct health label naming the condition,
2. fall back to the current polling behaviour at the **fast** interval rather
   than the 15-minute one, and
3. keep working.

Silently watching less than the declared roots is the one outcome that is not
acceptable, because it looks identical to "nothing is happening".

Network and virtual filesystems deliver events unreliably or not at all. The
reconciliation sweep is the answer for all of them; no per-filesystem
special-casing.

## Dependency

`notify` 8.2.0 only, approved 2026-08-21.

- 141.5M all-time downloads, 35.4M in the last 90 days
- CC0-1.0
- Of its ten required dependencies, `mio`, `walkdir`, `bitflags`, `libc`,
  `log` and `windows-sys` are already in our lockfile. New: `notify`,
  `notify-types`, plus `inotify` (Linux only) and `fsevent-sys` (macOS only).
  `kqueue` is BSD/iOS-gated and never builds for our targets.

`notify-debouncer-full` is deliberately **not** taken; the debounce above is
small enough to own.

## Testing

The event source must be injectable. Define a trait for "something that
reports changed paths", implement it over `notify`, and use a hand-driven
double in tests. Tests then cover the pipeline without depending on real
filesystem event timing, which is flaky in CI and differs per platform.

Cover at least:

- an event for a session maps to the right session, including a
  `subagents/` member mapping to its parent
- a session that goes quiet for `quiescence_secs` becomes eligible via the
  event path, and produces the same decision the poll path would
- a burst of events for one session coalesces into one pass
- reconciliation still finds a session whose events were never delivered
- watch registration failure falls back to fast polling and raises the health
  label, rather than going quiet
- an undeclared root produces no watch

CI runs on GitHub runners where filesystem event behaviour varies; no test may
depend on real event delivery timing.

## Out of scope

- Changing what `eligibility::evaluate` decides
- Changing the queue, supersede, or upload paths
- The unexplained continuous ~7% baseline in #388, which does not appear with
  the window occluded and is a separate question
- Any change to the GTK or Windows shells; this is daemon-side and benefits
  all three

## Risks

- **A missed event delays a session by up to 15 minutes.** Accepted: queue
  entries routinely sit for hours awaiting a decision.
- **Watch registration is a new startup failure mode.** Mitigated by the
  fallback above; it must be exercised by a test, not assumed.
- **Event storms during heavy agent activity.** The debounce bounds work per
  session, but a corpus-wide storm could still queue many sessions. The
  existing `max_queue_entries` cap and #381's `load_can_land` already bound
  what happens downstream.
