# Daemon core v1.1 — design

Date: 2026-08-08
Status: approved for planning
Scope: sub-project 1.5. Client-only. Buildable and verifiable on any platform.
Predecessor: `2026-08-08-contributor-background-daemon-design.md`

## Why there is a v1.1

Designing the native shells surfaced four things the frozen `v1` contract
cannot support, one of which is a defect in what shipped.

1. **`preview` does not do what it claims.** The v1 contract documents
   `would_send_bytes`, and `ipc.rs` returns `entry.size_bytes` — the raw
   session file size taken off the queue entry. No dry run is executed. There
   is no event count, no redaction count, and no body.

   The number is not merely incomplete, it is wrong, and wrong in the
   dangerous direction. Measured on the repository's own fixture, a session
   file of 1615 bytes produces a 4160-byte envelope: envelope metadata
   dominates, so the redacted payload is roughly **2.6x larger** than the file
   it came from. The old preview therefore *understated* what leaves the
   machine. An earlier draft of this spec asserted the opposite on the
   assumption that redaction shrinks the payload; it does not.

   Preview is the entire consent surface of the product, and it currently has
   nothing truthful to render.

2. **A GUI-only contributor can get permanently stuck.** Choosing the NEAR AI
   scan sets `near-ai-notice-not-acknowledged`, which the daemon clears only
   when an interactive `submit` prints the notice. There is no method that
   clears it, so an app-only user is paralysed with no path out.

3. **Enrollment and consent are terminal-only.** The app must be able to
   onboard, and the consent prompt already tells people they can change scopes
   later.

4. **The process model changed.** The shells host the watch loop in-process
   rather than shipping a second signed binary, so the core has to be callable
   as a library through a C ABI, not only reachable over a socket.

## Process model

The library is the product; the process model varies by platform.

- **macOS and Windows**: one application bundle. The menu-bar/tray app links
  the C-ABI library, takes the existing exclusive lock, and runs the watch,
  upload, digest, and history loops in-process. It also serves the control
  socket, so the CLI keeps working while the app is running.
- **Linux**: the same loop runs headless under the existing systemd user unit.
  The GTK window is an optional client over the socket.
- **Anywhere**: `trace-commons-contributor daemon run` is unchanged.

Exactly one process runs the loop at a time; `daemon.lock` already enforces
that and needs no change. A shell that fails to take the lock connects to
whoever holds it instead.

## The C ABI

A new `crates/trace-commons-contributor-ffi` crate, `crate-type = ["cdylib",
"staticlib"]`, exporting a small C surface with a generated header. It wraps
the existing Rust; it contains no logic of its own.

```c
// Lifecycle. Runs the daemon loop on its own thread with its own runtime.
tc_handle*  tc_daemon_start(const char* config_dir, char** err);
void        tc_daemon_stop(tc_handle*);

// Control. Same request handlers the socket serves, called in-process.
// Returns a NUL-terminated JSON response; free with tc_string_free.
char*       tc_call(tc_handle*, const char* method, const char* params_json);

// Events. cb is invoked on a background thread with a JSON event frame.
void        tc_subscribe(tc_handle*, void (*cb)(const char* event_json, void* ctx), void* ctx);

// Preview. Reads the session file and runs the real redaction pipeline.
tc_preview* tc_preview_open(tc_handle*, const char* entry_id, char** err);
const char* tc_preview_body(tc_preview*);          // redacted transcript, UTF-8
const char* tc_preview_summary_json(tc_preview*);  // counts, sizes, opening prompt
int32_t     tc_preview_search(tc_preview*, const char* needle, char** matches_json);
void        tc_preview_free(tc_preview*);

void        tc_string_free(char*);
const char* tc_last_error(void);
```

Ownership rule, stated once and enforced everywhere: **every `char*` returned
by this library is owned by the caller and freed with `tc_string_free`.**
Every `const char*` is borrowed and valid only until the owning handle is
freed. No other lifetime rules exist.

Panics are caught at the boundary (`catch_unwind`) and converted to an error
string. A Rust panic must never unwind into Swift, C#, or C.

### Why preview is in-process rather than on the socket

A 900 KB redacted transcript does not fit the socket's 1 MiB line cap once
metadata is added, and paging it would be a workaround for a transport the
hosting shell is not using anyway. In-process it is a pointer. Search becomes
a local scan over an in-memory string rather than a protocol.

Critically, preview calls the **same** redaction path the uploader calls, so
what preview shows cannot disagree with what gets sent. Any design where the
shell computes its own preview is rejected for that reason.

## Real preview

`preview` (both socket and FFI) runs the existing dry-run pipeline for one
entry and returns:

```json
{
  "entry": { ...as today... },
  "would_send_bytes": 84213,
  "raw_session_bytes": 148902,
  "event_count": 148,
  "opening_prompt": "add a withdrawal path that evicts from snapshots",
  "redactions": { "secret": 12, "token": 4, "path": 31, "email": 0 },
  "consent_scopes": ["debugging_evaluation", "model_training"],
  "residual_risk": "pattern-based"
}
```

- `would_send_bytes` is the **redacted envelope** size — normally larger than
  `raw_session_bytes`, not smaller. Both are reported so a contributor can see
  the relationship rather than having to assume one.
- `opening_prompt` is the first user message, redacted and truncated to 200
  characters. It is what identifies a session to its author; a timestamp does
  not.
- `redactions` proves scrubbing ran and calibrates the reader. A session that
  obviously touched a `.env` and reports `secret: 0` is a signal.
- `residual_risk` is always present and never optimistic.
### Preview is a local operation, not daemon state

Preview reads a session file from disk and runs redaction. It needs the config
directory and the session path; it does not need the lock, the queue, or the
running loop. So **any process with access to the state directory can produce
a preview**, whether or not it is the one hosting the daemon.

That is what makes the Linux arrangement work: a GTK window connected over the
socket to a systemd-hosted daemon still gets full body and search, because it
computes the preview itself through the same library rather than asking the
daemon for it.

The socket's `preview` therefore returns the summary only, and the body is
obtained locally in every shell. This is not a limitation being worked around
— shipping a redacted transcript through a socket to a process that could
simply read it is pointless work.

The one invariant this must not break: preview and upload call the same
redaction path, so what preview shows cannot disagree with what gets sent. A
test asserts the preview size equals the uploaded envelope size for the same
session.

## New methods

| Method | Purpose |
|---|---|
| `enroll` | Invite link or grant plus chosen scopes; performs what `login` does. Returns the resulting identity. |
| `consent_options` | The available scopes with human descriptions, so three shells do not each hardcode a list that drifts from the protocol. |
| `set_consent_scopes` | Change scopes after enrollment. The consent prompt already promises this. |
| `acknowledge_near_ai_notice` | Records that the third-party scan disclosure was shown in a UI. The only way an app-only user can become unstuck. |
| `cancel` | Returns an `approved` entry to `pending`. Backs the 5-second undo. Fails if the upload already started. |
| `pause {until}` | Timed pause, persisted. App-side timers die with the app and would silently un-pause. |
| `list_audit` | Local hash-only record of autonomy changes and bulk approvals. |
| `eligibility_reasons` | Why discovered sessions were not offered. The reasons are already computed and currently discarded. |

`hello` reports `schema_version: "trace_commons.daemon.v1_1"` and
`supported_versions: ["trace_commons.daemon.v1", "trace_commons.daemon.v1_1"]`.
Every v1 method keeps its v1 shape.

## Removing the terminal-only gate

`set_project_mode: auto_upload` and `approve --all` become available over the
socket. `Origin` stops gating them and the concept is removed rather than left
dormant.

**Accepted risk, recorded deliberately.** The v1 rationale was that the socket
would let same-user code arm the contributor's own daemon to exfiltrate a
project continuously. That reasoning does not survive scrutiny: malware with
same-user code execution can already read `~/.claude/projects` directly and
send it anywhere, and can install its own persistent watcher, so the daemon
confers neither the read nor the persistence. Routing exfiltration through the
daemon would in fact be strictly worse for an attacker — rate-limited, capped,
redacted, PII-filtered, and delivered to a server they cannot read from.

The replacement is visibility, not restriction:

- Every autonomy change and bulk approval appends a local audit entry.
- The first automatic upload from a newly armed project raises a notification.
- Armed projects are shown persistently, never collapsed away.

These are user-visibility features. They are not security controls and the
spec does not claim otherwise.

## Project label disambiguation

Labels are basenames, so two repositories both called `api` are
indistinguishable in the queue — and one of them might be the client's. This
is a correctness problem, not cosmetics: it can cause someone to approve the
wrong repository.

The daemon appends a short stable suffix when, and only when, a label collides
with another known project: `api (a3f1)`, derived from a hash of the project
key. Unique labels are unchanged. The path itself still never crosses the
socket.

## Eligibility reasons

`eligibility::evaluate` already returns `NotQuiescent`, `Unstable`,
`AlreadyUploaded`, `GrowthBelowThreshold`, and `ReuploadCapReached`, and the
watcher throws them away. They are retained per session and exposed as counts
plus per-session labels. "It can see my files and I don't know what it is
doing with them" is the thought that gets software uninstalled, and the answer
already exists in memory.

## Health precedence

`status.health` carries a single label, so two simultaneous problems are
indistinguishable. v1.1 fixes the precedence explicitly, highest first:

`not-logged-in` → `near-ai-notice-not-acknowledged` → `privacy-filter-canary-failed`
→ `pii-filter-unavailable` → `claim-mint-failed` → `ingest-unreachable`
→ `queue-full` → `daily-cap-reached`

Rationale: states the contributor can act on outrank states they can only wait
out.

## Error handling

Unchanged in character. Fail-closed everywhere, fixed labels only, atomic
state writes, no path or credential over any boundary — including the FFI
boundary, which is subject to exactly the same rules as the socket.

## Testing

- Preview: redacted size differs from raw size on a fixture with secrets;
  redaction counts are non-zero where the fixture plants secrets; opening
  prompt is redacted; preview size equals what an actual upload sends.
- FFI: round-trip through the C ABI from a Rust integration test that loads
  the cdylib; string ownership under repeated alloc/free; a deliberate panic
  inside a call surfaces as an error rather than unwinding; `tc_daemon_start`
  twice against one directory fails the second time on the lock.
- Each new method: happy path, bad params, and the state it is supposed to
  change actually persisted.
- Label disambiguation: two projects with the same basename get distinct,
  stable labels; a unique basename is untouched.
- Health precedence: asserted as a table test over every pair.

## Out of scope

- Withdraw (`2026-08-08-trace-withdrawal-design.md`).
- The shells themselves.
- The Windows named pipe. `windows-sys` is approved (2026-08-08) but scoped
  to `cfg(windows)`; the implementation lands with the Windows shell, and this
  sub-project must not add it to the macOS or Linux dependency tree.
