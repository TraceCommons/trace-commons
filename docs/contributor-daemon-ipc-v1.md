# Contributor daemon IPC — `trace_commons.daemon.v1`

Status: **frozen**. This is the contract the native menu-bar and window
applications are built against. Additive changes get a new version; the
methods, framing, and semantics below do not change under `v1`.

`crates/trace-commons-contributor/tests/daemon_ipc_contract.rs` is the
executable half of this document. `hello` reports its own method list and a
test asserts that list matches `METHODS` in `src/daemon/ipc.rs`, so this file
and the implementation cannot drift silently.

## Transport

A unix domain socket at `$TRACE_COMMONS_CONTRIBUTOR_DIR/daemon.sock`
(default `~/.config/trace-commons/daemon.sock`).

Windows is specified but **not implemented in v1**. A per-user-restricted
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

**Two operations are refused over the socket** and return
`not_authorized` / `tty-required`:

- `set_project_mode` with `mode: "auto_upload"`
- `approve` with `all: true`

This is not redundant with holding the device key. The key lets an attacker
upload once; these two calls would let them arm the contributor's own trusted
daemon to exfiltrate every future session in a project, continuously, under
the contributor's real grant, producing receipts that look normal. Both
require a terminal:

```
trace-commons-contributor daemon project <path> --mode auto
trace-commons-contributor daemon approve --all
```

Applications should surface that command rather than trying to work around
the refusal.

## Privacy rules binding on clients

- Queue entries on the wire carry `project_label`, never `project_key` or
  `path`. Both of those are local filesystem paths and never cross the socket.
  Do not display or log a path.
- History records carry no path at all.
- `get_settings` reports `near_ai_configured: bool`. The privacy-filter
  credential is never echoed.

## Methods

| Method | Params | Result | TTY only |
|---|---|---|---|
| `hello` | — | `schema_version`, `methods[]`, `events[]`, `max_line_bytes` | |
| `status` | — | see below | |
| `list_pending` | — | `pending[]` of queue entries | |
| `preview` | `entry_id` | `entry`, `would_send_bytes` | |
| `approve` | `entry_id` or `all: true` | `approved: <count>` | `all` only |
| `dismiss` | `entry_id` | `ok: true` | |
| `pause` | — | `paused: true` | |
| `resume` | — | `paused: false` | |
| `list_projects` | — | `projects[]` of `{project_label, mode, added_at}` | |
| `set_project_mode` | `project_key`, `label`, `mode` | `ok: true` | `auto_upload` only |
| `list_history` | `limit` (default 50, max 1000) | `history[]` | |
| `history_rollup` | — | see below | |
| `refresh_history` | — | `requested: true` | |
| `get_settings` | — | settings, credential redacted | |
| `set_settings` | any of `quiescence_secs`, `digest_interval_secs`, `local_notifications` | updated settings | |
| `subscribe` | — | `subscribed: true`, then a `snapshot` event | |
| `shutdown` | — | `stopping: true` | |

### `status`

```json
{
  "schema_version": "trace_commons.daemon.v1",
  "logged_in": false,
  "tenant_id": null,
  "consent_scopes": [],
  "paused": false,
  "queue_depth": 0,
  "next_digest_at": null,
  "health": { "last_error_label": null, "since": null }
}
```

`health.last_error_label` is the field a tray renders when something is wrong.
It is one of the labels below, or `null` when healthy.

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

## Events

| Event | When | Data |
|---|---|---|
| `snapshot` | immediately after `subscribe` | `{pending[], status}` |
| `queue_changed` | queue contents changed | `{}` |
| `status_changed` | pause/resume or health changed | `{}` |
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

## Error codes

`unknown_method`, `bad_params`, `not_authorized`, `busy`, `unavailable`.
