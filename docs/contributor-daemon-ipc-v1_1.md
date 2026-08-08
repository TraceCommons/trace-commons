# Contributor daemon IPC — `trace_commons.daemon.v1_1`

Status: **stable, additive**. This is the contract the native menu-bar and
window applications are built against, on three separate teams, from this
document alone. `v1_1` is additive over `v1`: every `v1` method keeps its
`v1` request and response shape unchanged, so a `v1` client that ignores
methods and fields it does not recognize keeps working against a `v1_1`
daemon without modification. New methods and fields are additions, not
replacements.

`crates/trace-commons-contributor/tests/daemon_ipc_contract.rs` is the
executable half of this document. `hello` reports its own method list and a
test asserts that list matches `METHODS` in `src/daemon/ipc.rs`, so this file
and the implementation cannot drift silently.

## Transport

A unix domain socket at `$TRACE_COMMONS_CONTRIBUTOR_DIR/daemon.sock`
(default `~/.config/trace-commons/daemon.sock`).

Windows is specified but **not implemented in v1_1**. A per-user-restricted
named pipe needs a `SECURITY_DESCRIPTOR`, which needs a `windows-sys`
dependency. That dependency is approved (2026-08-08), scoped to
`cfg(windows)` only; the implementation lands with the Windows application.
See `docs/superpowers/specs/2026-08-08-contributor-shell-windows-design.md`.

## Framing

JSON, one message per line, UTF-8.

- **Request**: `{"id": <u64>, "method": "<name>", "params": {...}}`. `params`
  may be omitted.
- **Response**: `{"id": <same u64>, "result": {...}}` or
  `{"id": <same u64>, "error": {"code": "...", "message": "..."}}`.
- **Event** (server-pushed): `{"event": "<name>", "data": {...}}`. Events
  never carry an `id`; that is how a client distinguishes them from responses
  on the shared connection.

Rules:

- Every response echoes its request's `id`. Clients may pipeline; the server
  may answer out of order.
- Maximum line length is 1 MiB. A longer or unparseable line gets one
  `bad_params` response and the connection closes.
- `error.message` is always a fixed label, never a server response body,
  a path, or a token.

## Authorization

The 0700 state directory is the access control. The daemon refuses to serve
from a directory that is not 0700; a 0700 directory belonging to someone else
is not writable by this process, so a socket cannot be created there.

**There is no longer a terminal-only carve-out.** Through `v1`, two
operations -- arming a project for `set_project_mode: "auto_upload"` and
bulk-approving the whole queue with `approve: {"all": true}` -- were refused
over the socket (`not_authorized` / `tty-required`) and a client was expected
to tell the contributor to run a CLI command instead. `v1_1` removes that
restriction. Both calls now work over the socket exactly like every other
method, with no TTY requirement. **Applications must stop special-casing
these two calls or telling users to open a terminal for them.**

This is an accepted-risk change, recorded deliberately, not an oversight.
The `v1` rationale was that the socket would let same-user code arm the
contributor's own daemon to exfiltrate a project continuously. That reasoning
does not survive scrutiny: malware with same-user code execution can already
read the contributor's session files (e.g. `~/.claude/projects`) directly and
send them anywhere, and can install its own persistent watcher process to do
so continuously -- the daemon confers it neither the read access nor the
persistence it would need. Routing exfiltration through the daemon instead
would in fact be strictly worse for such an attacker: it is rate-limited,
capped, redacted, PII-filtered, and delivered to a server the attacker
cannot read back from. Holding the device key already lets an attacker
upload once; these two calls do not meaningfully extend what that key
already grants.

What replaces the restriction is **visibility, not gatekeeping**:

- Every autonomy change (`set_project_mode: "auto_upload"`) and every bulk
  approval (`approve: {"all": true}`) appends a local, hash-only audit entry,
  readable via `list_audit`.
- Applications are expected to show armed (`auto_upload`) projects
  persistently in their UI, never collapsed away, so a contributor always
  knows what is armed.

**The audit log is a visibility feature for the contributor's own benefit.
It is not a security control, and this document does not claim it is one.**
It does not prevent anything; it only lets a contributor later see that
something happened. Do not build a security argument, a permission gate, or
any enforcement logic on top of `list_audit` -- it is a record, not a guard.

## Project keys and labels

`project_label` is **always derived by the daemon** from `project_key`. A
client cannot choose one. `set_project_mode` still accepts a `label`
parameter for compatibility with older clients, and ignores it: a
caller-supplied string used to be stored verbatim and then returned by
`list_projects` and written into `daemon-audit.jsonl`, which made both of
those -- the two surfaces the label-only rule exists to protect -- writable
by any socket client with an arbitrary path, token, or transcript fragment.

`project_key` itself is validated. It must be one of:

- the locked unknown-cwd sentinel (`unknown-project`), which can never be
  armed;
- a key the daemon already knows -- discovered on a queued session, or
  already present in the project policy;
- an absolute path that exists on this machine as a directory and
  canonicalizes to itself.

Anything else is refused with the fixed label `project-key-unrecognized`
and nothing is recorded. This keeps the label the daemon derives anchored to
something it can corroborate, rather than to a string a client invented.

## Privacy rules binding on clients

- Queue entries on the wire carry `project_label`, never `project_key` or
  `path`. Both of those are local filesystem paths and never cross the socket.
  Do not display or log a path.
- History records carry no path at all.
- `get_settings` reports three booleans -- `near_ai_configured`,
  `claude_root_configured`, `codex_root_configured` -- and never the
  underlying values. The first is a credential (the privacy-filter API key);
  the other two are local filesystem paths. All three are configured-or-not
  facts an app may render as a checkmark, never as text containing the
  actual value.
- `preview` returns a **summary** over the socket -- counts, labels, and
  sizes -- never the redacted trace body. The full redacted event body is
  intentionally available only in-process, through the crate's C ABI, not
  over this IPC surface. This is not an oversight: unlike `enroll`, `preview`
  needs neither the daemon's file lock nor its running event loop, so a
  native app that wants the actual body should call the C ABI's local
  preview entry point directly rather than asking the daemon for it. The
  socket path exists for uses (a tray or window summarizing what's pending)
  that never need the trace content itself.

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `hello` | — | `schema_version`, `supported_versions[]`, `methods[]`, `events[]`, `max_line_bytes` | |
| `status` | — | see below | |
| `list_pending` | — | `pending[]` of queue entries | |
| `preview` | `entry_id` | see below | summary only, no trace body |
| `approve` | `entry_id` or `all: true` | `approved: <count>` | `all: true` no longer requires a terminal |
| `dismiss` | `entry_id` | `ok: true` | |
| `cancel` | `entry_id` | `ok: true` | returns an `approved` entry to `pending`; error if not currently `approved` |
| `pause` | `until` (optional RFC 3339 timestamp) | `paused: true`, `paused_until` | see "Pause semantics" below |
| `resume` | — | `paused: false` | |
| `list_projects` | — | `projects[]` of `{project_label, mode, added_at}` | |
| `set_project_mode` | `project_key`, `mode` (`label` accepted and ignored) | `ok: true` | `auto_upload` no longer requires a terminal; see "Project keys and labels" below |
| `list_history` | `limit` (optional, default 50, max 1000) | `history[]` | |
| `history_rollup` | — | see below | |
| `refresh_history` | — | `requested: true` | |
| `list_audit` | `limit` (optional, default 50, max 1000) | `entries[]`, newest first | see "Audit log" below |
| `queue_outcome_counts` | — | `reasons: {label: count}` | see "queue_outcome_counts" below; does **not** cover sessions never queued |
| `get_settings` | — | settings; credential and local paths reported as booleans only | |
| `set_settings` | any of `quiescence_secs`, `digest_interval_secs`, `local_notifications` | updated settings | |
| `consent_options` | — | `scopes[]` of `{name, description, always_on, grants_data_use}` | |
| `set_consent_scopes` | `scopes[]` (wire-name strings; omitted means floor scope only) | `consent_scopes[]` | requires an existing enrollment |
| `enroll` | `grant` xor `invite`, `scopes[]` (optional) | `enrolled: bool`, and on success `tenant_id`, `device_key_id`, `consent_scopes[]` | performs real network I/O |
| `acknowledge_near_ai_notice` | — | `acknowledged: true` | clears the `near-ai-notice-not-acknowledged` health label |
| `subscribe` | — | `subscribed: true`, then a `snapshot` event | |
| `shutdown` | — | `stopping: true` | |

### `status`

```json
{
  "schema_version": "trace_commons.daemon.v1_1",
  "logged_in": false,
  "tenant_id": null,
  "consent_scopes": [],
  "paused": false,
  "queue_depth": 0,
  "next_digest_at": null,
  "health": { "last_error_label": null, "since": null }
}
```

`health.last_error_label` is the field a tray renders when something is
wrong. It is one of the labels in "Health precedence" below, or `null` when
healthy.

### `preview`

```json
{
  "entry": { "...": "queue entry, see list_pending" },
  "would_send_bytes": 4160,
  "raw_session_bytes": 1615,
  "event_count": 3,
  "opening_prompt": "…",
  "redactions": { "aws_secret_key": 1 },
  "pii_labels_present": ["email"],
  "consent_scopes": ["debugging_evaluation"],
  "residual_risk": "pattern-based"
}
```

`would_send_bytes` is the size of the **redacted envelope**, not the raw
session file -- the same envelope `submit` would actually send, computed by
running the real redaction pipeline. It is normally **larger** than
`raw_session_bytes`, not smaller. Intuition runs the other way, because
redaction removes content, but a redacted envelope also carries schema,
consent, and privacy metadata that the raw session file does not, and that
overhead usually outweighs whatever redaction shortened. Measured example: a
1615-byte raw session produced a 4160-byte envelope. Do not build a UI that
assumes `would_send_bytes < raw_session_bytes`; assert nothing about the
direction, only that `would_send_bytes` is the number that governs consent.

`preview` requires an entry currently in the queue (`bad_params` /
`unknown-entry-id` otherwise) and a working privacy filter (`unavailable` /
`preview-failed` on failure). Neither `redactions` nor `pii_labels_present`
in the response ever contains the actual matched text, only counts and
category labels.

### `pause` semantics

`until` is optional. Passed, it is parsed as an RFC 3339 timestamp:

- A timestamp in the past is rejected with `bad_params` /
  `until-in-the-past` rather than accepted and immediately treated as
  resumed -- a pause that is already a lie the instant it is acknowledged is
  worse than an explicit error.
- A malformed timestamp is rejected with `bad_params` / `until-invalid`.
- A valid future timestamp is persisted (it survives a daemon or app
  restart) and, once it lapses, the daemon clears the pause on its own and
  publishes a `status_changed` event. A client should treat `status_changed`
  as the authoritative signal that a timed pause ended, rather than running
  its own timer against the `paused_until` it was given.
- Omitting `until` pauses indefinitely, exactly as in `v1`.

### `list_audit`

```json
{
  "entries": [
    { "at": "2026-08-08T12:00:00Z", "action": "armed-auto-upload", "project_label": "myproj", "detail": null }
  ]
}
```

`limit` is optional, defaults to 50, and is capped at 1000 even if a larger
value is requested. Entries are returned newest first, matching
`list_history`'s convention. `action` and `detail` are always fixed labels --
never free text, a path, or a token. See "Authorization" above for what this
log is (and is not) for.

### `queue_outcome_counts`

```json
{ "reasons": { "dismissed-by-contributor": 2, "expired-without-decision": 1 } }
```

A count, by `reason_label`, across every entry currently on the queue in any
state. This method is **not** named `eligibility_reasons`, and does not
explain sessions that were never offered at all. Every `reason_label` this
method can report belongs to an entry that already exists in the queue (in
practice: dismissed, refused, expired, and superseded entries). It cannot
answer "I finished a session, why is nothing pending?" for a session the
watcher discarded *before* a queue entry was ever created -- for example a
non-eligible verdict or an `Ignore`-mode project. Do not present this
method's output as covering that case; a future method may be added for it,
and this name was deliberately chosen to leave room for that without another
contract break.

### `history_rollup`

```json
{
  "week":     {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "month":    {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "all_time": {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "credit_pending": 0.0,
  "credit_final": 0.0,
  "quarantined": 0,
  "last_refreshed_at": null
}
```

`quarantined` is reported separately and must be rendered separately.
Quarantine means **held for operator privacy review**, not rejected. A
contributor who sees it grouped with failures reads it as rejection.

`last_refreshed_at` is `null` when history has never been refreshed from the
server; show staleness rather than presenting a stale cache as current.

### `consent_options`

```json
{
  "scopes": [
    { "name": "debugging_evaluation", "description": "…", "always_on": true, "grants_data_use": true },
    { "name": "public_attribution", "description": "…", "always_on": false, "grants_data_use": false }
  ]
}
```

`always_on` is `true` for exactly one scope, the floor scope every
contributor implicitly grants. `grants_data_use` is `false` for scopes (such
as `public_attribution`) that carry no data-use grant of their own -- do not
present those beside real data-use scopes with equal visual weight.

### `set_consent_scopes`

Takes an optional `scopes` array of wire-name strings (omitted means the
floor scope only) and replaces the enrolled config's consent scopes.
Requires an existing enrollment (`unavailable` / `not-logged-in` otherwise).
Appends a `consent-scopes-changed` audit entry.

### `enroll`

Performs real network I/O against the issuer -- registering this device,
exactly as the CLI's `login` does for a terminal caller. Takes `grant` xor
`invite` (both present is `bad_params` / `grant-and-invite-mutually-exclusive`)
plus the optional `scopes` array described above. Does **not** accept an
`allowed_hosts` override from the caller (unlike the CLI's `--allowed-hosts`
flag); a caller-supplied allowlist can degrade to permissive, and a socket
caller is not trusted with that. On success, the underlying error is never
echoed back over the socket -- it can carry an issuer response body or a URL
-- so failures are reported only as `unavailable` / `enroll-failed`.

### `acknowledge_near_ai_notice`

Records that the NEAR AI first-use privacy disclosure was shown to the
contributor in a UI, and clears the `near-ai-notice-not-acknowledged` health
label so the daemon will start using that filter. This is the only way an
app-only contributor (one who never touches the CLI, which shows the same
notice on stdout) can get past that gate. Because this asserts, on the
caller's unverified word, that a disclosure was actually shown to someone,
it is audited (`near-ai-notice-acknowledged`) -- an application must not
call it without actually having shown the notice text first.

## Events

| Event | When | Data |
|---|---|---|
| `snapshot` | immediately after `subscribe` | `{pending[], status}` |
| `queue_changed` | queue contents changed | `{}` |
| `status_changed` | pause/resume, a lapsed timed pause, or health changed | `{}` |
| `digest_due` | batching interval elapsed with pending work | `{pending, text}` |
| `resync_required` | this client fell behind the event buffer | `{}` |

`subscribe` sends a full `snapshot` before any delta, so a client never has to
race `list_pending` against the stream at startup. On `resync_required`, call
`list_pending` and `status` again.

## Queue states

`pending`, `approved`, `uploading`, `uploaded`, `refused`, `failed`,
`expired`, `superseded`.

- `expired` — aged out after the TTL (14 days) without a decision. The clock
  is **suspended** while the daemon is unhealthy, so an outage does not
  silently discard traces.
- `superseded` — the session changed after it was offered. The daemon re-hashes
  before uploading; on a mismatch it sends nothing and creates a fresh
  `pending` entry for the new content. An approval covers content, not a
  filename.
- `approved` entries can be returned to `pending` with `cancel`.

## `reason_label` and health taxonomy

| Label | Meaning | Suspends expiry |
|---|---|---|
| `not-logged-in` | no config or no device key | yes |
| `pii-filter-unavailable` | filter configured but unreachable | yes |
| `claim-mint-failed` | issuer refused or failed | yes |
| `ingest-unreachable` | upload failed after retries | yes |
| `daily-cap-reached` | volume cap in force until the day rolls | yes |
| `near-ai-notice-not-acknowledged` | first-use notice not delivered interactively | yes |
| `privacy-filter-canary-failed` | canary self-test failed | yes |
| `queue-full` | queue at its configured maximum | no |
| `dismissed-by-contributor` | declined by hand | n/a |
| `expired-without-decision` | aged out | n/a |
| `session-changed-after-offer` | superseded | n/a |

### Health precedence

`status.health.last_error_label` carries a single label at a time, even
though several conditions can be true simultaneously. When more than one
applies, the daemon shows the one with the **highest precedence** below,
listed highest first:

1. `not-logged-in`
2. `near-ai-notice-not-acknowledged`
3. `privacy-filter-canary-failed`
4. `pii-filter-unavailable`
5. `claim-mint-failed`
6. `ingest-unreachable`
7. `queue-full`
8. `daily-cap-reached`

Rationale: states the contributor can act on outrank states they can only
wait out. An application should not attempt to infer or reconstruct this
order itself; treat `status.health.last_error_label` as the one label to
render, and use `queue_outcome_counts` / `list_audit` for supplementary
detail, not for picking a different label to show instead.

## Error codes

`unknown_method`, `bad_params`, `not_authorized`, `busy`, `unavailable`.

`not_authorized` is retained in the error taxonomy for forward
compatibility but is no longer returned by any method in this version --
the `v1` terminal-only gate that used it was removed (see "Authorization"
above).
