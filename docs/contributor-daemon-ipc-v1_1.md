# Contributor daemon IPC — `trace_commons.daemon.v1_1`

Status: **stable, additive, with one deliberate exception**. This is the
contract the native menu-bar and window applications are built against, on
three separate teams, from this document alone. `v1_1` is additive over
`v1`: every `v1` method keeps its `v1` request and response shape, so a
`v1` client that ignores methods and fields it does not recognize keeps
working against a `v1_1` daemon without modification. New methods and
fields are additions, not replacements.

**The exception, stated rather than buried:** `set_settings` now *refuses* a
key it does not recognize, where `v1` silently ignored it. A `v1` caller
that sent an unrecognized key alongside a recognized one used to get a
partial success and now gets `bad_params`. This is a deliberate break of the
compatibility rule above, made because the old behaviour meant a mistyped
key left the daemon quietly running on the old value with the caller
believing otherwise -- and one of those keys decides which directories get
scanned for the contributor's transcripts. No shipped client relies on the
old behaviour, because no application has shipped against `v1` yet. See
[`set_settings`](#set_settings) for the full rules.

**New in this revision (additive, no existing field or shape changed):**

- `project_id` — an opaque, daemon-issued handle for a project. It appears
  on every queue entry (`list_pending`, the `snapshot` event) and on every
  `list_projects` row, and `set_project_mode` accepts it in place of
  `project_key`. Before this, a socket client could not call
  `set_project_mode` at all: paths never cross this socket, so a client held
  only `project_label`, and a label is not an admissible `project_key`.
  Arming and ignoring a project were unreachable from every GUI. See
  ["Naming a project"](#naming-a-project-ids-keys-and-labels).
- `set_public_profile`, `clear_public_profile`, and `get_public_profile` --
  the public roster handle. A shell could show the `public_attribution`
  consent scope but had no way to claim the handle that scope is about, so
  the "Go public" flow and the settings profile panel were unreachable from
  every application. See ["The public profile"](#the-public-profile).
- `list_projects` now reports **discovered** projects as well as configured
  ones, each with the mode actually in force and a `configured` boolean. An
  onboarding screen that asks a contributor to exclude a repository has to
  be able to list a repository nobody has ruled on yet.
- `preview_turns` — an index of turn boundaries **into the body
  `preview_body` already returns**. The transcript surface wants
  `— user — turn 1 —` separators and a `144 more turns` footer, and had
  nothing to place them from. This is strictly an overlay: `preview_body`'s
  request and response shapes are untouched, its bytes are unchanged, and
  every offset indexes that same string. The daemon deliberately does not
  re-render the events as turns -- that would drop `structured_payload`,
  `token_counts`, `latency_ms`, `cost_usd` and `failure_modes`, showing a
  contributor less than the artifact under a tab titled "exactly what would
  be sent". See ["`preview_turns`"](#preview_turns).
- `history_rollup` now carries an optional `community` object: this
  contributor's own line on the public roster, polled from the server's
  public snapshot on the daemon's own interval. Both desktop clients already
  draw a History community section and neither could populate it, because
  nothing on this contract carried the standing. Additive in the strict
  sense -- no existing `history_rollup` field changed shape, and a client
  that ignores the object is unaffected. See
  ["`history_rollup`"](#history_rollup).

`crates/trace-commons-contributor/tests/daemon_ipc_contract.rs` is the
executable half of this document. `hello` reports its own method list and a
test asserts that list matches `METHODS` in `src/daemon/ipc.rs`, so this file
and the implementation cannot drift silently.

## Transport

Two transports carrying an identical protocol. The framing, the method set,
and the error taxonomy do not vary by platform; only the listening and
connecting ends do. `ipc::serve_connection` is generic over the stream, so
both share one implementation of everything above the transport.

**Unix**: a domain socket at `$TRACE_COMMONS_CONTRIBUTOR_DIR/daemon.sock`
(default `~/.config/trace-commons/daemon.sock`). Access control is the
containing directory: the daemon refuses to serve unless it is 0700, and a
0700 directory belonging to another user is not writable, so no one else can
place a socket there either.

**Windows**: a named pipe at `\\.\pipe\trace-commons-daemon-<16 hex>`, where
the hex is a SHA-256 prefix over the state directory path — never the path
itself, which under the default layout carries the OS username, and which a
pipe name would expose to every process on the machine.

The Windows access control is entirely different in kind, and the difference
matters. A named pipe does not live in the state directory; it lives in the
machine-wide pipe namespace, where any local process may attempt to open it
by name. **Its DACL is the only thing protecting it.** The daemon builds one
granting the creating user's SID alone (`D:P(A;;GA;;;<sid>)`, protected so
no inherited entry can widen it) and refuses to serve if it cannot — there is
no fallback to a default-ACL pipe, because that fallback is the
vulnerability. The first instance is created with `first_pipe_instance` so a
squatter cannot pre-create the name under a weaker descriptor and be served
on.

One behavioural difference on the client side: the Windows one-shot client
opens the pipe as a file handle, which has no equivalent of
`set_read_timeout`, so the 60-second request timeout is not applied there.

> **The Windows DACL is verified by CI, and that job has not yet run.**
> Type-checking for `x86_64-pc-windows-gnu` establishes the FFI signatures
> and the control flow and nothing about whether the descriptor actually
> excludes another user — a runtime property no cross-compile can observe.
> The observation is the `windows-pipe-acl` CI job
> (`scripts/windows/verify-pipe-acl.ps1`, driving
> `src/bin/win-pipe-acl-probe.rs`): on `windows-latest` it creates a second,
> non-administrator local account, has it attempt to open the pipe, and
> requires ERROR_ACCESS_DENIED, with a control confirming the owning user is
> still admitted. The account is deliberately not an administrator — one can
> take ownership of any object and would reach the pipe regardless, so that
> test would look like evidence while proving nothing.
>
> **That job has run and passed** (PR #247, run 31307072159): `DENIED 5` from
> the second user — ERROR_ACCESS_DENIED, refused by the access check
> specifically rather than by pipe-busy or file-not-found, which the script
> rejects so a coincidental refusal cannot pass as evidence — and `CONNECTED`
> from the owner. The claim holds only while that job keeps running; weaken
> or remove it and the verification lapses with it.

`windows-sys` is approved (2026-08-08) for exactly this and scoped to
`[target.'cfg(windows)'.dependencies]`; macOS and Linux dependency trees do
not contain it.
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
  readable via `list_audit`. So do `set_consent_scopes` and
  `acknowledge_near_ai_notice`.
- The action and its audit entry are **one fail-closed unit**. If the entry
  cannot be persisted -- disk full, permissions, a corrupt log -- the action
  is rolled back and the call returns `audit-write-failed`. It does not
  succeed with a warning: an unrecorded change is exactly what removing the
  terminal-only restriction was not supposed to make possible.
- The durable log is capped and rotates oldest-first, so it cannot grow
  until appending to it starts failing. Capping `list_audit`'s output alone
  would not have bounded the file.
- Applications are expected to show armed (`auto_upload`) projects
  persistently in their UI, never collapsed away, so a contributor always
  knows what is armed.

**The audit log is a visibility feature for the contributor's own benefit.
It is not a security control, and this document does not claim it is one.**
It does not prevent anything; it only lets a contributor later see that
something happened. Do not build a security argument, a permission gate, or
any enforcement logic on top of `list_audit` -- it is a record, not a guard.

## Naming a project: ids, keys, and labels

A project has three names on this contract, and they are not
interchangeable.

| Name | Who mints it | Crosses the socket | What it is for |
|---|---|---|---|
| `project_id` | the daemon | yes | naming a project back to the daemon |
| `project_label` | the daemon | yes | showing a project to a human |
| `project_key` | the caller | **no** | naming a project from a terminal |

### `project_id`

`project_id` is an opaque handle the daemon derives from the project key:
`"proj_"` followed by 16 hex characters of `sha256(project_key)`. It is a
hash, not an encoding, so it carries no path component and cannot be turned
back into one. It is derived rather than stored, so it is the same across a
daemon restart and across a policy file rebuilt from scratch, and there is
nothing to migrate.

It appears on every queue entry and every `list_projects` row: a client that
can see a project can name it. `set_project_mode` accepts it in place of
`project_key`, and it is the identifier **every socket client should use**.
`project_id` wins if both are sent.

An id resolves only against projects the daemon already knows — one already
in the policy, one sitting in the queue, or the `unknown-project` sentinel.
An id that resolves to none of those is refused with the fixed label
`project-id-unrecognized`, and nothing is recorded.

Knowing an id confers nothing. It is an identifier, not a capability: the
same call was always available to anyone who could name the directory, and
resolution is still limited to projects the daemon discovered on its own.

### `project_key`, and why it is still accepted

`project_key` is an absolute local path. It does not cross the socket in any
response, and no GUI should ever hold one. It remains an accepted
*parameter* for exactly one caller: a human in a terminal running
`daemon project <path> --mode ignore` **before that project's first
session** — the flow that excludes a repository pre-emptively. The daemon
cannot mint an id for a project it has never discovered, so an id cannot
serve that flow, and the two coexist deliberately rather than one replacing
the other.

Sending neither is `bad_params` with `project_id-or-project_key-required`.

### `project_label`

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
and nothing is recorded. An unrecognized `project_id` is refused the same
way, with `project-id-unrecognized`. This keeps the label the daemon derives anchored to
something it can corroborate, rather than to a string a client invented.

## Privacy rules binding on clients

- Queue entries on the wire carry `project_label` and `project_id`, never
  `project_key` or `path`. Both of those are local filesystem paths and never
  cross the socket. Do not display or log a path. Render the label; send the
  id back.
- History records carry no path at all.
- `get_settings` reports three booleans -- `near_ai_configured`,
  `claude_root_configured`, `codex_root_configured` -- and never the
  underlying values. The first is a credential (the privacy-filter API key);
  the other two are local filesystem paths. All three are configured-or-not
  facts an app may render as a checkmark, never as text containing the
  actual value.
- `preview` returns a **summary** over the socket -- counts, labels, and
  sizes. The full redacted event body is a separate call, `preview_body`,
  because it does not fit one frame and has to be paged. Both carry trace
  content under the one carve-out below; nothing else on this surface does.
  An earlier revision of this document said the body was deliberately
  in-process only, reachable through the crate's C ABI, on the reasoning
  that any process could compute a preview for itself. That reasoning does
  not survive the deployment we recommend: the C ABI's entry point needs the
  daemon's shared state, which only the process holding the daemon lock has,
  and under a systemd-managed daemon with the window as a socket client that
  is never the window. Loading a second copy of the daemon's state is not a
  substitute -- it rewrites the queue file and sweeps the pinned envelopes
  the running daemon is still holding. So the body is served over the
  socket, paged.

### The preview exemption

`preview` is the **one** interface that deliberately carries trace content,
and this is a decision, not a contradiction left lying around. The socket's
`opening_prompt`, the socket's `preview_body` chunks, and the C ABI's
`tc_preview_body` are all trace content. A contributor cannot consent to
sending something they cannot see; an approval given against a byte count
and a project name is not an informed one. So the rule has exactly one
carve-out, and it is bounded:

- **Post-redaction only.** What preview carries is what the real redaction
  pipeline produced. Raw session text never crosses either boundary.
- **Only for an entry the caller already holds.** Content is reachable only
  by naming an `entry_id` already in the queue. There is no bulk read, no
  ambient read, and no way to ask for a session the daemon has not offered.
  An id that is not in the queue -- unknown, already swept, or never
  offered -- is refused by both `preview` and `preview_body` with the same
  fixed label, `bad_params` / `unknown-entry-id`, so the two cases are not
  distinguishable from outside.
- **Never onward.** It never appears in a log line, an audit entry, a
  history record, notification text, or a receipt. Not truncated, not
  summarized, not hashed-with-a-sample. Nothing copies it into any of those.

`preview_turns` is **not** part of the exemption and does not need to be: it
carries event-type labels, tool names the envelope already records as
metadata, and byte offsets, never redacted text. It is still served only for
an entry the caller already holds, under the same `unknown-entry-id` rule,
because the shape of a contributor's transcript is itself something they
have not offered anyone.

The exemption also covers **the previewed envelope at rest**. A successful
`preview` writes the redacted envelope it built to the contributor's own
0700 state directory (`daemon-approved-envelope-{entry_id}.json`, 0600,
atomic), and the upload sends exactly those bytes rather than building a
second envelope. That is what makes preview and upload agree under a
privacy filter that does not reproduce its own output: with
`pii_filter = "near-ai"` an LLM-backed filter returns different spans for
identical text, and any design that rebuilt-and-compared refused every
previewed entry forever. The stored bytes are held to the same bounds as
the rest of the exemption, plus two of their own:

- **Bounded.** One file per previewed-and-approved entry, each at most the
  1.5 MB envelope ceiling, and live entries are capped by
  `max_queue_entries`. Only an approved-but-unsent backlog accumulates.
- **Deleted when the entry resolves.** Uploading, refusing, failing,
  expiring, superseding, or revoking an approval all drop the file, and
  `logout` removes any that remain. If the bytes are missing or unreadable
  when the upload comes to read them, the approval is revoked and the entry
  re-offered -- never silently rebuilt.

They never cross the socket or the C ABI. `preview` reports the
`envelope_digest` that identifies them; it does not serve them.

Everywhere else the rule remains absolute: no path, token, invite code,
claim, device key, or trace content in any log line, error string, receipt,
history record, audit entry, notification text, or IPC response.

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `hello` | — | `schema_version`, `supported_versions[]`, `methods[]`, `events[]`, `max_line_bytes` | |
| `status` | — | see below | |
| `list_pending` | — | `pending[]` of queue entries | |
| `preview` | `entry_id` | see below | summary only; the body is `preview_body` |
| `preview_body` | `entry_id`, `offset` (optional), `limit` (optional), `body_digest` (required when `offset > 0`) | `chunk`, `next_offset`, `total_bytes`, `body_digest`, `envelope_digest`, `enrolled`, `max_chunk_bytes` | the redacted body, paged; see "`preview_body`" below |
| `preview_turns` | `entry_id`, `body_digest` (**required**) | `entry_id`, `body_digest`, `envelope_digest`, `turn_count`, `turns[]` | an index of turn boundaries **into the body `preview_body` returns**; the body itself is unchanged. See "`preview_turns`" below |
| `approve` | `entry_id`, `all: true`, or `project_id` | `approved: <count>`, `hold_secs`, `hold_until`, `flagged`, `redactions`, `skipped[]` | `all: true` no longer requires a terminal; `project_id` approves that project's `Pending` entries and no others, matched by the id `entry_value` publishes (never `project_label`, which is display text and unstable); the three are mutually exclusive and `all` wins over `project_id` wins over `entry_id` when more than one is sent; see "The approval hold" and "What `approve` reports" below |
| `dismiss` | `entry_id` | `ok: true` | |
| `cancel` | `entry_id` | `ok: true` | returns an `approved` entry to `pending`; guaranteed to succeed for the whole hold; error if not currently `approved` |
| `pause` | `until` (optional RFC 3339 timestamp) | `paused: true`, `paused_until` | see "Pause semantics" below |
| `resume` | — | `paused: false` | |
| `list_projects` | — | `projects[]` of `{project_id, project_label, mode, added_at, configured, is_unresolved_bucket}` | configured **and** discovered projects; see "`list_projects`" below |
| `set_project_mode` | `project_id` **or** `project_key`, `mode` (`label` accepted and ignored) | `ok: true` | socket clients send `project_id`; `auto_upload` no longer requires a terminal; see "Naming a project" above |
| `list_history` | `limit` (optional, default 50, max 1000) | `history[]` | |
| `history_rollup` | — | see below | |
| `refresh_history` | — | `requested: true` | |
| `list_audit` | `limit` (optional, default 50, max 1000) | `entries[]`, newest first | see "Audit log" below |
| `queue_outcome_counts` | — | `reasons: {label: count}` | see "queue_outcome_counts" below; does **not** cover sessions never queued |
| `quiesce` | `timeout_secs` (optional, default 60, max 300) | `quiesced: true`, `waited_ms` | parks uploads for an update swap; `busy` / `quiesce-timeout` if in-flight work does not finish in time |
| `get_settings` | — | settings; credential and local paths reported as booleans only | |
| `set_settings` | any of `quiescence_secs`, `digest_interval_secs`, `approval_hold_secs`, `local_notifications`, `claude_root`, `codex_root` | updated settings | see "`set_settings`" below |
| `consent_options` | — | `scopes[]` of `{name, description, always_on, grants_data_use}` | |
| `set_consent_scopes` | `scopes[]` (wire-name strings; omitted means floor scope only) | `consent_scopes[]` | requires an existing enrollment |
| `enroll` | `grant` xor `invite`, `scopes[]` (optional) | `enrolled: bool`, and on success `tenant_id`, `device_key_id`, `consent_scopes[]` | performs real network I/O |
| `acknowledge_near_ai_notice` | — | `acknowledged: true` | clears the `near-ai-notice-not-acknowledged` health label |
| `set_public_profile` | `handle` (required), `bio` (required, string **or** `null`) | the profile, plus `handle_persisted` | performs real network I/O; replaces the whole profile; see "The public profile" below |
| `clear_public_profile` | — | the profile (now empty), plus `withdrawn: true` and `handle_persisted` | performs real network I/O; see "The public profile" below |
| `get_public_profile` | — | `on_roster`, `handle`, `bio`, `public_since`, `public_url` | a LOCAL cache, not a server read-back; `public_url` is always `null` |
| `subscribe` | — | `subscribed: true`, then a `snapshot` event | |
| `shutdown` | — | `stopping: true` | |
| `withdraw` | `submission_id` | `withdrawn: true`, `distribution_reach` | performs real network I/O; see "Withdrawal" below |
| `withdraw_bulk` | `status` (`submitted` \| `quarantined` \| `accepted`) | `withdrawn: <count>`, `failed: <count>` | performs real network I/O; see "Withdrawal" below |

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
  "residual_risk": "pattern-based",
  "envelope_digest": "sha256:…",
  "input_fingerprint": "sha256:…",
  "enrolled": true,
  "subagent_count": 3,
  "subagents_dropped": 0
}
```

`subagent_count` and `subagents_dropped` also appear on every queue entry
(`list_pending`, the `snapshot` event). Both are additive; the schema version
stays `trace_commons.daemon.v1_1`, and a client that ignores them behaves
exactly as before.

A Claude Code conversation is not one file: each delegated subagent's turns
are written beside the session under `<session-uuid>/subagents/`, and one
conversation on a probed machine had 114 of them. The daemon offers the whole
conversation as a single entry, so `subagent_count` is how many delegated
transcripts that entry covers. A client should say so on the card -- what is
being consented to is the whole conversation, and its extent is part of the
description rather than decoration.

`subagents_dropped` is non-zero only when the conversation exceeded the
source's raw byte budget and the largest delegated transcripts were left out
to keep the envelope under its cap. A client **must** surface a non-zero
value: the difference between a trace the contributor knows was trimmed and
one that silently arrives partial is the whole point of showing it. The drop
is decided when the transcript is loaded, so the preview and the upload
describe the same bytes.

No ordinal is exposed -- there is no "1 of 3" -- because nothing in the
transcript format supplies one. Ordering delegated transcripts against each
other would be a claim this daemon cannot verify.

`opening_prompt` is redacted trace content -- see "The preview exemption"
above for why this one field is allowed to be, and what that permission does
not extend to.

`envelope_digest` identifies the redacted envelope this summary describes;
`input_fingerprint` identifies the configuration that produced it. Both are
hashes, never content. Issuing `preview` **pins the entry** to that envelope
and stores the envelope itself: an `approve` that follows is an approval of
exactly those bytes, and the upload sends them verbatim rather than
redacting a second time. The upload still refuses and re-offers the entry if
an envelope-determining input has moved since (`approval-inputs-changed`),
if the session file has changed, or if the stored bytes are gone
(`approved-envelope-unavailable`). An app can hold these two values to
confirm that the entry it later approves is the one it actually displayed.

Previewing the same entry twice re-pins it to the second preview, which is
the one the upload will send. Previewing an entry that is no longer
`pending` reports the summary but changes no pin: an entry already approved
stays bound to the artifact it was approved as.

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

**`preview` does not require an enrollment.** It performs no network I/O and
needs neither the daemon's file lock nor its running loop, so an app can
show a contributor what would be sent *before* they decide to enrol -- which
is when the question matters most. Through `v1_1`'s first releases this
refused with `unavailable` / `not-logged-in` unless a `contributor.json`
existed, which forced app harnesses to fabricate an enrollment purely to
preview a local file; that requirement was incidental and is gone.

`enrolled` says which kind of preview you got:

- `true` — the ordinary case. Built from the real enrolled identity through
  the configured privacy filter, and (as described above) **pinned**: the
  envelope is stored and a later `approve` covers exactly those bytes.
- `false` — no enrollment on this device. The envelope is built from the
  same placeholder identity the CLI's unenrolled `--dry-run` uses, with a
  preview submission id disjoint from any real one, and through the
  **deterministic-only** redactor: any configured external privacy filter is
  ignored, so pre-enrollment trace text is never sent anywhere to be
  classified. Nothing is pinned, and `envelope_digest` /
  `input_fingerprint` describe that placeholder build -- neither is bindable
  to a later approval, since enrolling changes the identity the envelope
  carries. Render such a preview as an illustration, and re-preview after
  enrolling before asking for an approval.

`would_send_bytes`, `redactions`, `pii_labels_present` and `opening_prompt`
are real in both cases; an unenrolled preview understates nothing about
redaction except what an external filter would additionally have removed.

### `preview_body`

The redacted body `preview` describes: the envelope's redacted events,
pretty-printed JSON, exactly the bytes the upload will send. This is what a
"Search" tab searches and what an "Exactly what would be sent" tab renders.

Request:

```json
{ "entry_id": "…", "offset": 0, "limit": 131072, "body_digest": "sha256:…" }
```

Response:

```json
{
  "entry_id": "…",
  "total_bytes": 432118,
  "offset": 0,
  "chunk": "[\n  {\n    \"event_type\": \"user_message\", …",
  "next_offset": 131072,
  "body_digest": "sha256:…",
  "envelope_digest": "sha256:…",
  "enrolled": true,
  "max_chunk_bytes": 131072
}
```

**It is paged, and you must page it.** A redacted envelope can approach the
1.5 MB envelope ceiling; a socket line is capped at `max_line_bytes` (1 MiB).
A single frame therefore cannot be promised, so there is none: read from
`offset: 0`, append `chunk`, and follow `next_offset` until it is `null`.
Nothing is ever silently truncated -- `total_bytes` is the length of the
whole body, and a client that has not received `[0, total_bytes)` has not
read the trace.

**Continuation pages must be anchored.** Every response carries
`body_digest`, a SHA-256 over the complete body. Send it back on every
request with `offset > 0`. Omitting it is `bad_params` /
`body-digest-required`; sending one that does not match the body the daemon
resolved is `unavailable` / `preview-body-changed`, and the correct
response to that is to restart from `offset: 0`, not to splice. This is not
ceremony: a rebuilt envelope is a different artifact (event ids are minted
per build, and an LLM-backed privacy filter does not reproduce its own
spans), so two pages of two builds concatenated would be a transcript that
never existed.

`offset` is a byte offset into a UTF-8 string and is only ever a value the
daemon handed you: pages break on character boundaries, so `next_offset` is
not always `offset + limit`. An `offset` that is out of range or not on a
character boundary is `bad_params` / `offset-invalid`. `limit` above
`max_chunk_bytes` is capped rather than refused; `limit: 0` is
`bad_params` / `limit-invalid`.

**Search happens in the client.** The daemon ships the body and does not
match against it. A daemon-side search would have to reproduce whatever the
client means by a match -- case folding, word boundaries, how a hit that
spans two events is presented -- and would still have to ship surrounding
text for the client to render, leaving the client holding the body anyway
plus a second matcher to keep in step. One body, one text, one search: what
the contributor searched is the text in front of them.

The property that outranks both of those decisions: **never report a trace
clean that you could not actually read.** A "0 matches" is only honest after
`[0, total_bytes)` has been received and searched. If any page errors, if
`preview-body-changed` interrupts you, or if you stopped early, say so --
"could not read the whole trace" -- and do not render an all-clear.

Where the body comes from, and why it is stable:

- An entry already previewed (and so pinned) has its envelope stored on
  disk, and that is what is read -- byte-identical across pages, across
  calls, and identical to the C ABI's `tc_preview_body` for the same entry.
- An entry with no stored envelope runs the redaction pipeline, exactly as
  `preview` does, and is pinned by the same rules (an unenrolled build is
  never pinned, and `enrolled: false` says so). A first call may therefore
  take as long as `preview`.
- An entry that is pinned but whose stored bytes are missing or unusable is
  refused with `unavailable` / `approved-envelope-unavailable`. It is not
  rebuilt: a rebuild is not the artifact the contributor approved, and
  presenting it as "what would be sent" would be false.

`envelope_digest` is the same value `preview` reports, so an app can confirm
the body it is showing belongs to the summary it displayed.

### `preview_turns`

Where the turns begin inside the body `preview_body` returns, so a client
can draw `— user — turn 1 —` separators and a `144 more turns` footer over a
transcript it is rendering verbatim.

**This adds nothing to the body and changes nothing about it.** It is an
overlay: `preview_body`'s bytes are still the whole artifact, still
pretty-printed JSON, still exactly what the upload sends, and every offset
here indexes that same string. The daemon does **not** re-render the events
as chat turns, and a client must not either. A prose re-render drops every
field that has no prose form -- `structured_payload`, `token_counts`,
`latency_ms`, `cost_usd`, `failure_modes` -- and would therefore show a
contributor *less* than what would be sent, under a tab titled "exactly what
would be sent". Flat monospace with separators drawn at these offsets is the
design, and this method is what makes it possible without a second
rendering path.

Request:

```json
{ "entry_id": "…", "body_digest": "sha256:…" }
```

Response:

```json
{
  "entry_id": "…",
  "body_digest": "sha256:…",
  "envelope_digest": "sha256:…",
  "turn_count": 3,
  "turns": [
    { "index": 0, "role": "user_message", "byte_offset": 2, "byte_len": 412 },
    { "index": 1, "role": "assistant_message", "byte_offset": 416, "byte_len": 690 },
    { "index": 2, "role": "tool_call", "tool_name": "bash", "byte_offset": 1108, "byte_len": 1244 }
  ]
}
```

`index` is 0-based and dense, so it indexes the array directly; a client
showing "turn 1" for the first separator renders `index + 1`. `role` is the
`event_type` wire name of the event that opens the turn -- the same string
that appears in the bytes at `byte_offset`, so there are not two
vocabularies to reconcile. `tool_name` is present only when the opening
event names a tool. `byte_offset` and `byte_len` are a half-open range of
**UTF-8 byte** offsets into the body, on character and element boundaries.

**Grouping: a tool call and its result are one turn.** A `tool_call`
followed immediately by the `tool_result` carrying the same `tool_call_id`
is indexed as a single turn spanning both events, so the separator reads
`— tool: bash — turn 3 —` once rather than putting a boundary between a
command and its output. The pairing must be explicit and adjacent:
an unmatched call, a result whose call is missing, a pair reordered by the
source, and a pair with no `tool_call_id` to correlate on are all one turn
per event. Guessing a pair would mean labelling a span that covers two
unrelated events, which is the one error this index must not make.

**`body_digest` is required, on every call.** This is the anchoring rule
`preview_body`'s continuation pages use, applied from the first request,
because an index is a set of offsets into one specific string: against any
other string it is not stale, it is wrong, and wrong invisibly -- a
separator drawn over the wrong text still looks like a transcript. Omitting
it is `bad_params` / `body-digest-required`; a non-string is `bad_params` /
`body-digest-invalid`; a digest that does not match the body the daemon
resolved is `unavailable` / `preview-body-changed`. The correct response to
that last one is the same as for a page: re-read the body from `offset: 0`
and index the body you actually hold.

Where the body comes from, when an unpinned entry is built and pinned, and
which failures are refused rather than rebuilt are all exactly as described
for `preview_body` -- the two methods resolve the same envelope through the
same path, so the index and the body can never come from two different
builds. An entry that resolves but whose body cannot be indexed is
`unavailable` / `preview-turn-index-failed`: fail-closed, because an index
that is not certainly exact is worse than none.

The result is not paged, and does not need to be: a turn serializes to well
under 100 bytes while one pretty-printed event costs upwards of 170, and an
envelope is capped at 1.5 MB, so the index stays a fraction of the 1 MiB
line cap even for an envelope at the ceiling. It is never truncated -- a
truncated index is a transcript with turns silently missing from the end --
so `turn_count` always equals `turns.length`. A client that wants a
`144 more turns` footer computes it from `turn_count` and what it chose to
render, not from anything the daemon left out.

### `list_projects`

```json
{
  "projects": [
    {
      "project_id": "proj_9f2c1ab30d4e5f60",
      "project_label": "my-proj",
      "mode": "notify_only",
      "added_at": null,
      "configured": false,
      "is_unresolved_bucket": false
    }
  ]
}
```

Every project the daemon knows about, in two kinds:

- **configured** (`configured: true`, `added_at` set) — the contributor has
  ruled on it with `set_project_mode`.
- **discovered** (`configured: false`, `added_at: null`) — the daemon has
  seen a session for it and nobody has ruled on it. `mode` is the effective
  mode, which for an unruled project is the `notify_only` default.

#### `is_unresolved_bucket`

True for exactly one row: the bucket holding sessions whose working directory
had no usable final segment. Sessions in it can never be armed for automatic
upload — `Policy` refuses `auto_upload` for that key independently of any
client — so a shell showing the row with a permanent note is **reporting**
enforcement, not performing it. `Ignore` still applies: the bucket can be
silenced even though it cannot be armed.

The flag is sent rather than left for clients to derive, because the daemon is
the only side that knows it for free. Deriving it means re-implementing the
`project_id_for` hash to compare ids, which is a copy of the rule per client
with nothing keeping the copies in step.

**Clients MUST NOT recognise this row by `project_label`.** The raw label is a
slug no contributor should read, so every shell replaces it with its own
wording; a client keyed on the displayed string loses the row's explanation
the moment that wording improves, and does it silently.

Discovered rows are reported because the onboarding "which of these should
never be uploaded" screen has to list precisely the projects nobody has
decided about yet. A project becomes configured only by being ruled on, so a
configured-only list is a list of decisions already made — it can never
contain the repository the contributor is being asked to exclude. Nothing
new crosses the socket: a discovered row carries the same two
daemon-derived fields (`project_id`, `project_label`) that the queue entry
for that project already carries.

`mode` is always the mode in force, not the stored value: the
`unknown-project` bucket reports `notify_only` even if a hand-edited policy
file says `auto_upload`, because the daemon refuses to act on that.

### What `approve` reports

`approve` returns:

```json
{
  "approved": 1,
  "hold_secs": 10,
  "hold_until": "2026-08-08T12:00:10Z",
  "flagged": 1,
  "redactions": { "private_email": 2, "secret:openai_api_key": 1 },
  "skipped": [ { "entry_id": "…", "reason_label": "not-enrolled" } ]
}
```

This is the whole signal a one-click submit needs: a client that never calls
`preview` can still show "Sent -- scrubbing removed 3 things, 1 flagged.
[Undo]" off this response alone, with `hold_until` driving the undo window
(see below).

- **`redactions`** sums the redaction category counts from
  `preview`'s `redactions` (same shape: category name to count) across every
  entry this call built a preview for. An entry that was already previewed
  before this call -- and so was already pinned -- contributes nothing here;
  its own `preview` response already reported those counts once, and this
  call does not rebuild it. **This means an already-previewed entry and a
  freshly-built one with nothing to redact look identical in this
  response**: both report `redactions: {}`, `flagged: 0`. A client that
  calls `preview` before `approve` should render its toast from the
  `preview` response's own counts, not assume `approve`'s zero means
  nothing was found. Counts and category names only, exactly as in
  `preview`: never the redacted text itself. Keys are real category names
  the deterministic redactor and the (optional) remote privacy filter emit
  -- e.g. `private_email`, `local_path`, `secret:openai_api_key`,
  `secret:github_token`, or a `privacy_filter:<label>` from the remote pass
  -- not the two placeholder names shown above; see
  `PreviewSummary::redactions` for the full set.
- **`flagged`** counts how many of the entries this call built a preview for
  came back with a non-empty `pii_labels_present` (the same field `preview`
  reports). It is a count of entries, not a count of labels. Same
  already-previewed caveat as `redactions` above.
- **`skipped`** lists, for every id `approve` was asked to act on that it
  did not approve, an `entry_id` plus a fixed `reason_label`:

  | `reason_label` | Meaning | Retry |
  |---|---|---|
  | `not-enrolled` | No config was readable when the build ran | Retry after enrolling |
  | `session-file-vanished` | The session file behind the entry is gone | Will not succeed for this entry |
  | `preview-failed` | The redaction pipeline itself failed | May be transient |
  | `envelope-too-large` | The built envelope exceeds the size the daemon will store, even though the build succeeded | **Never** succeeds for this entry -- do not offer retry |
  | `not-pinned` | The pin did not stick even though the build succeeded, and the entry is still `pending` (a concurrent write, or the entry vanished from the queue mid-call) | Transient -- retry is expected to work |
  | `not-pending` | The entry was not `pending` when this call reached it -- already `approved` by an earlier `approve`, or dismissed, expired or superseded meanwhile | Refresh queue state rather than retry blindly; a retry alone can never succeed |

  Nothing here is free text, a path, or trace content. **`approved` plus
  the length of `skipped` always equals the number of entries `approve` was
  asked to act on** -- for `entry_id` that is 1, for `all`/`project_id` it
  is however many matched at selection time. An id absent from both would
  be a silent loss of an approval decision; the response is built so that
  cannot happen. An `entry_id` naming no entry at all is refused before any
  of this runs -- see below.
- **An unrecognized `entry_id`** is refused the same way `preview` refuses
  the same input: `bad_params` / `unknown-entry-id`, not a `skipped` entry.
  This applies only to the single-`entry_id` form. `all` and `project_id`
  cannot produce this case: their ids are read from the queue itself at
  selection time, so every id they act on already names a real entry.
- `approve` builds and pins an envelope for any entry that was not already
  previewed -- the same build `preview` runs, just triggered by `approve`
  instead. This is what makes `redactions` and `flagged` available even when
  a client skips `preview` entirely: a client that never calls `preview`
  still produces a pinned envelope the uploader accepts, and still gets the
  counts to render its toast from. An entry that was already pinned by an
  earlier `preview` is approved without rebuilding, so it is counted in
  `approved` but does not contribute to `redactions` or `flagged` (see the
  caveat above).

### The approval hold (the undo window)

`hold_until` is the instant the daemon will first consider the entry for
upload. Until then the uploader skips it, so an "Undo" offered during that
window is real: `cancel` cannot answer `not-cancelable` while the hold runs,
because nothing can have claimed the entry.

Rules an application can rely on:

- **Count down against `hold_until`, never against your own duration.** A
  client running its own five-second timer while the daemon holds for some
  other interval is the same bug as having no hold at all -- the countdown
  and the daemon disagree about when the decision stops being reversible.
  `hold_until` is read from the entry the daemon just wrote and is the exact
  value its upload pass compares against.
- **The entry uploads at `hold_until`, not after some later poll.** The
  comparison is `now < hold_until`, so waiting out exactly the reported
  instant is waiting out exactly the hold. (The upload itself still happens
  on the daemon's ordinary poll, so the send occurs at or after that
  instant, never before it.)
- **`approve: {"all": true}` holds every entry it approved**, all to the one
  reported deadline. The response's `hold_until` is true of the whole batch.
- **`hold_until` is `null`** when nothing was approved, or when
  `approval_hold_secs` is `0`. A client must then offer no undo rather than
  invent one.
- **`cancel` during the hold returns the entry to `pending`** and clears the
  approval outright: the scopes, the envelope-determining fingerprint, and
  the hold itself. A subsequent `approve` starts a fresh window.
- **A standing `auto_upload` opt-in is not held.** Those entries are
  approved in advance, are separately audited, and no client is counting
  down for them; they upload on the next pass exactly as before. Only an
  `approve` call creates a hold.

`hold_secs` is the configured window (`approval_hold_secs`, default **10
seconds**). Ten rather than five: the designed undo is five seconds, and
five is therefore the floor, not the target -- the client's countdown starts
after the approval was stamped, and the `cancel` that ends it still has to
travel back over the socket. The extra margin also absorbs clock skew
between an application counting in its own process and a daemon deciding in
another. It costs nothing that matters: uploading is unattended background
work on a 60-second poll.

The hold is a property of the entry (the approval instant it carries plus
the configured window), not of the daemon's poll timing. Tuning
`poll_interval_secs`, or the uploader getting faster, cannot shorten it.

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

### `quiesce`

```json
{ "quiesced": true, "waited_ms": 412 }
```

Parks the upload queue and waits for anything already in flight to finish, so
an update can replace the binary without abandoning a half-uploaded trace.
Used by `trace-commons-contributor update`.

The park is in-memory and dies with the daemon process. It is deliberately not
`pause`: pause is the contributor's own persisted setting, and an update must
not rewrite it. There is no `unquiesce` verb for the same reason -- the process
that was quiesced is the process the swap replaces.

On timeout the daemon answers `busy` / `quiesce-timeout` and un-parks itself.
The caller leaves the update staged and retries later. There is no forced
path.

### `set_settings`

Takes a JSON object of settings to change. Every top-level key must be one
of `quiescence_secs`, `digest_interval_secs`, `approval_hold_secs`,
`local_notifications`, `claude_root`, `codex_root` -- a key this method does
not recognize is
refused outright (`bad_params` / `settings-unknown-field`), not silently
ignored, so a caller that mistypes a key gets a definite signal rather than
a daemon that quietly kept the old value. A recognized key holding the
wrong JSON type is refused the same way (`bad_params` /
`settings-invalid-value`). An object with no keys at all is refused
(`bad_params` / `no-known-setting-supplied`).

`approval_hold_secs` takes a non-negative integer: how long an approval is
held before the uploader will touch it, i.e. how long the contributor's undo
really lasts. Default 10; `0` disables the hold, and `approve` then reports
`hold_until: null` so a client knows to offer no undo. It is read at each
upload pass, so a change applies to approvals already sitting in the queue,
and a shortened hold can release an entry a client is still counting down
for -- treat the `hold_until` from `approve` as authoritative for the
approval it accompanied, and do not change this setting mid-countdown.

`claude_root` and `codex_root` each take a JSON string (a filesystem path)
or `null` (clear the override, falling back to the conventional per-user
location). Setting either here only takes effect from the daemon's *next*
supervisor tick onward -- the tick already scheduled or in flight when this
call returns has already read the old value. A caller that needs the
watcher to scan a non-default location from the very first tick -- most
importantly a native host embedding the daemon via the C ABI, or a test
harness that must never scan the real `~/.claude`/`~/.codex` -- cannot get
that through `set_settings`, since it only works on an already-running
daemon and the first tick fires immediately on start. That is what the C
ABI's `tc_daemon_start_with_settings` is for: it applies the same object
this method validates, but before starting the daemon, so the first tick
already observes the override. See `include/trace_commons.h`.

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

The answer additionally carries a `community` object when, and only when,
this contributor has a standing on the public roster:

```json
{
  "community": {
    "rank": 14,
    "novelty_credit": 1240.0,
    "accepted_in_window": 12,
    "accept_rate": 0.75,
    "window_label": "7d",
    "public_since": "2026-07-09T10:30:00Z",
    "snapshot_at": "2026-08-17T12:00:00Z",
    "analytics_withheld": true
  }
}
```

This is additive: every field above is new, no existing field of
`history_rollup` changed shape, and a client that ignores `community`
behaves exactly as it did before. The protocol version is unchanged.

The figures are this contributor's own row on the roster the server serves
publicly at `GET /v1/community/leaderboard`, reduced to what a client draws.
The names differ from the server's, deliberately: `novelty_credit` is the
server's `score` (named for the snapshot's metric, which is
`novelty_credit`); `accepted_in_window` is its `accepted_count`, whose window
is `window_label`; `snapshot_at` is its `computed_at`. `accept_rate` is a
decimal in `0..=1`, not a percentage. `analytics_withheld` says whether the
corpus-wide aggregates were withheld -- when it is `true`, say so in words
rather than drawing an empty chart.

**Absent means no standing, and absent is not `null`.** The object is omitted
entirely -- rather than sent with null or zeroed fields -- in every one of
these cases, which a client renders identically by drawing no community
section at all:

- The contributor has published no handle.
- No snapshot is being served. The server answers `503` when the snapshot is
  withheld or has not been computed yet; that is a normal state of the
  community surface, not an error, and it must not surface to a contributor
  as one.
- The handle is not on the roster.
- `accepted_in_window` cannot be represented -- the snapshot reported a
  negative or out-of-range count. It is a bare number on the wire with no
  absent form, so rather than being rounded into a definite "0 accepted" it
  withholds the whole object.

`rank` and `accept_rate` may themselves be `null` inside an otherwise present
object; render a dash rather than `#0` or `0%`.

The daemon fetches the roster on its own interval (15 minutes, matching the
server's snapshot cadence and its published 15-minute withdrawal bound) and
`history_rollup` serves that cache. **The handler makes no network call**, and
there is no method to force a fetch. A cached standing older than twice the
withdrawal bound is dropped rather than served, so a daemon whose poll has
been failing goes quiet instead of publishing a stale public figure.

No `profile_url` is sent: nothing on the contributor's machine is configured
with the community site's address. A client draws no link rather than a
guessed one.

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

### The public profile

Three methods, one shape. `set_public_profile` and `clear_public_profile`
call the server's `PUT` and `DELETE` on `/v1/community/profile`;
`get_public_profile` reads a local cache and makes no network call at all.

```json
{
  "on_roster": true,
  "handle": "manian",
  "bio": "Ships billing systems by day.",
  "public_since": "2026-05-12T09:31:00Z",
  "public_url": null
}
```

`set_public_profile` adds `handle_persisted`; `clear_public_profile` adds
`withdrawn: true` and `handle_persisted`. Both are otherwise this shape, so
a client parses one profile whichever call it made.

**"Go public" is TWO calls, not one.** `set_consent_scopes` with
`public_attribution` added to the scope list, and then
`set_public_profile`. Neither implies the other: the scope records what the
contributor agreed to, and the second call is what actually puts a row on
the roster. A dialog that only sets the scope leaves the contributor
believing they are listed when nothing was published.

**The server authorizes against the claim's grant ceiling, not the local
scope list**, and the daemon deliberately does not pre-check
`consent_scopes` before calling. These calls mint an empty-scope claim,
which the issuer resolves to the caller's full grant ceiling, so the local
set can be *narrower* than what the credential actually carries -- refusing
locally would refuse contributors the server would have allowed. If the
server refuses, the contributor's remedy is to enrol again with
`public_attribution` in `scopes`, not to change anything locally.

**`bio` is required, and `null` is how you publish none.** The server
upserts with `bio = excluded.bio`, so the `PUT` replaces the *whole*
profile: there is no "leave the bio alone", and a call that omitted `bio`
would silently erase a published one on a handle rename. Omitting the key
is `bad_params` / `bio-required-or-null`. Send `"bio": null` to publish no
bio and `"bio": "…"` to publish one. (The CLI refuses the same ambiguity
with its `--bio` / `--no-bio` pair.)

**`get_public_profile` is a local cache, and a client must present it as
one.** There is no `GET /v1/community/profile` -- the server derives the
principal from the authenticated request and offers a contributor no
read-back of their own row -- so this reports what *this device* last
published: the handle, bio, and roster date the server returned on the last
successful `set_public_profile`, cleared by a successful
`clear_public_profile`. It is not a live read, and a profile claimed from
another device does not appear here. `on_roster` is simply whether a handle
is cached.

**`public_url` is always `null`.** The daemon knows the ingest origin it
uploads to, not the origin the community website serves profiles from.
Inventing one would give a "View public profile" link that does not
resolve, so the field is reported as `null` and a client that wants that
link must get the origin elsewhere.

**`handle_persisted` is not whether the call worked.** The response is a
success either way: the server already accepted the change, and the profile
is public (or withdrawn) regardless of what happened on this disk.
`handle_persisted: false` means only that the local cache write failed, so
`get_public_profile` will not report this profile until the next successful
`set_public_profile`. Do not render it as a failed save.

Errors, all fixed labels under the taxonomy at the bottom of this document:

| Code | Label | When |
|---|---|---|
| `bad_params` | `handle-required` | no `handle` |
| `bad_params` | `handle-too-short`, `handle-too-long`, `handle-invalid-character`, `handle-invalid-boundary`, `handle-consecutive-separators`, `handle-reserved` | the shared handle rules refused it |
| `bad_params` | `bio-required-or-null` | `bio` omitted (see above) |
| `bad_params` | `bio-invalid` | `bio` present but neither a string nor `null` |
| `bad_params` | `bio-too-long`, `bio-invalid-character` | the shared bio rules refused it |
| `unavailable` | `not-logged-in` | no enrollment on this device |
| `unavailable` | `profile-update-failed` / `profile-withdraw-failed` | the server call failed; the underlying error is never echoed, since it can carry a response body or a URL |

The handle and bio rules are `trace_commons_protocol::community_handle`'s,
the same code the server validates with, rather than a second copy in the
daemon: 3--32 characters of ASCII `[a-zA-Z0-9_-]`, alphanumeric at both
ends, no doubled separator, not a reserved name, and a bio of at most 280
UTF-8 bytes with no control characters except newline. Surrounding
whitespace on the handle is trimmed before validating, and the trimmed form
is what gets published. A client may pre-validate to give live feedback,
but the daemon's refusal is the authority.

The handle and the bio are the one thing on this surface that may appear in
a response, because publishing them is the entire point of the call. They
are still never written to a log line or an audit entry.

### Withdrawal

`withdraw` and `withdraw_bulk` call the server's
`POST /v1/account/traces/{submission_id}/withdraw` (see
`docs/superpowers/specs/2026-08-08-trace-withdrawal-design.md` for the full
design and the three response tiers). A successful `withdraw` reports
`distribution_reach`, one of:

- `not_distributed` -- the trace was `submitted` or `quarantined` and never
  entered the commons. Its content is simply deleted.
- `commons_not_distributed` -- the trace was `accepted` but not yet used in
  any published export or benchmark. Its content is deleted and it is
  excluded going forward.
- `commons_distributed` -- the trace was `accepted` and already included in a
  published export or benchmark. Its content is deleted and it is excluded
  going forward, but copies already distributed cannot be recalled.

**These are the server's names, and two of them were wrong in this document
until 2026-08-10.** It previously said `in_commons` and `distributed`. A client
built from the old text deserialises `not_distributed` correctly and fails on
the other two -- which are exactly the tiers whose message a contributor most
needs to be true. `wire_names_match_the_server` in
`crates/trace-commons-contributor/src/withdraw.rs` pins them; if that test
fails, this table is what to fix.

#### Canonical confirmation copy

Three applications are built from this document, and withdrawal is the one
place where a plausible-sounding phrase becomes a false promise about erasure.
Do not paraphrase per platform. Use these, adapted only for sentence case and
platform punctuation conventions.

**The tier is not knowable before the call, and that shapes everything below.**
The server computes `distribution_reach` during the withdrawal, from live
export membership. A client holds only the local `status`. So:

- local status `submitted` or `quarantined` maps to `not_distributed`
  reliably -- that is the server's own rule, and its copy can be shown before
  the action.
- local status `accepted` may resolve to EITHER `commons_not_distributed` or
  `commons_distributed`, and the client cannot tell which. It must show
  **both** bodies before the action, with the `commons_distributed` one given
  the greater weight, and must say plainly that the outcome is decided on the
  server. Showing only the gentler one would be a promise the client is not in
  a position to make.
- an unrecognised status shows the `commons_distributed` body alone, on the
  grounds that the furthest reach cannot be ruled out.

After the call, report the tier the server actually applied, using that tier's
body. Never a generic "withdrawn".

A contributor deciding whether to withdraw needs to know what it will achieve
while they can still change their mind -- which here means knowing the range
of what it might achieve, honestly, rather than a single confident sentence
the client cannot support.

| tier | confirmation body |
|---|---|
| `not_distributed` | "This trace never entered the commons. Withdrawing deletes it. Nothing was distributed and nothing needs recalling." |
| `commons_not_distributed` | "This trace is in the commons but has not been included in any published export or benchmark yet. Withdrawing deletes it and excludes it from everything published from here on." |
| `commons_distributed` | "This trace has already been included in a published export or benchmark. Withdrawing deletes our copy and excludes it from everything published from here on, **but copies that have already been distributed cannot be recalled.** Withdrawing does not undo that." |

Rules that bind every application:

1. **Never a generic "withdrawn".** The tier determines what actually
   happened, and collapsing three outcomes into one word is the specific
   failure this table exists to prevent.
2. **Never claim more erasure than the tier achieved.** In particular
   `commons_distributed` must not be phrased so a contributor could come away
   believing distributed copies were retrieved.
3. **Withdrawal does not reverse settled credit.** Do not state or imply that
   it does. (Revocation used to claw credit back; that is being removed.)
4. **`not_found` must not disclose which.** The server deliberately answers
   the same way whether a submission belongs to someone else or does not exist
   at all, so that account enumeration is impossible. An application must
   therefore say something like "no trace with that id under your account",
   and must NOT say "that trace belongs to someone else" or "that trace does
   not exist" -- either phrasing leaks precisely what the server refuses to.
5. **`confirmation_prompt` in the Rust client takes a `reach` the caller does
   not have pre-action.** It is usable for the after-the-fact message, or with
   a deliberately chosen worst case; it is not a pre-action lookup. Do not
   build a flow that assumes the tier is known before the request.
6. **Bulk withdrawal spans tiers.** `withdraw_bulk` reports only counts, so a
   bulk confirmation cannot promise a per-tier outcome. It must say that the
   selected traces may fall into different tiers and that some may already
   have been distributed. If an application cannot say that clearly, it should
   not offer bulk withdrawal.

`withdraw_bulk` withdraws every submission currently at `status` in the
local history cache (one of `submitted`, `quarantined`, or `accepted`; not
`withdrawn` itself, and not the `other` bucket `history_rollup` reports,
which covers statuses this client has no stable name for). It reports
`withdrawn` and `failed` counts rather than per-submission detail -- a
partial failure does not fail the whole call, and a contributor can retry
individual traces with `withdraw` if some did not go through.

Both update the local history cache immediately (the record's `status`
becomes `withdrawn`) so a contributor sees the effect without needing a
`refresh_history` round trip first.

**Both methods answer `unavailable` / `account-session-required` today,
always**, before ever attempting the call. Withdrawal is authenticated by an
account session -- deliberately not the device key that authenticates every
other call in this contract, so withdrawal survives losing the device that
submitted the trace -- and this daemon does not yet acquire or store one; it
only ever holds a device key. `account-session-required` is a distinct,
documented error rather than a generic failure so a calling shell can route
the contributor to account sign-in instead of showing a dead end, once
account sign-in exists to route to. Acquiring an account session is separate
work, tracked outside this document; nothing in this contract should be
read as that flow already existing.

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
- An approval also covers the **terms** it was given under: the consent
  scopes, the configuration that determines the envelope, and -- if the
  entry was previewed -- the exact redacted envelope that was shown. If any
  of those move before the upload, the approval is revoked and the entry
  returns to `pending` with `consent-scopes-changed-after-approval`,
  `approval-inputs-changed`, or `envelope-changed-after-approval`. Nothing
  is sent. An app should treat these the same as a superseded entry: offer
  it again, previewing afresh.
- `approved` entries can be returned to `pending` with `cancel`. An entry
  approved through `approve` stays untouched for its hold window first (see
  "The approval hold" above), so `cancel` is guaranteed to succeed for that
  whole window. After it, `cancel` still works right up until the upload
  pass claims the entry (`uploading`), at which point it is refused.

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
| `consent-scopes-changed-after-approval` | approval revoked, entry re-offered | n/a |
| `approval-inputs-changed` | an envelope-determining input moved after approval; entry re-offered | n/a |
| `envelope-changed-after-approval` | the envelope that would be sent is not the one previewed; entry re-offered | n/a |

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

`error.message` is always a fixed label. The ones `preview_body` adds are
`unknown-entry-id`, `offset-invalid`, `limit-invalid`,
`body-digest-invalid`, `body-digest-required` (all `bad_params`), and
`preview-body-changed`, `preview-failed`, `approved-envelope-unavailable`
(all `unavailable`).

`preview_turns` reuses that set -- `unknown-entry-id`, `body-digest-invalid`
and `body-digest-required` (`bad_params`), `preview-body-changed`,
`preview-failed` and `approved-envelope-unavailable` (`unavailable`) -- and
adds one of its own, `preview-turn-index-failed` (`unavailable`), for a body
that resolved but could not be indexed exactly.

`not_authorized` is retained in the error taxonomy for forward
compatibility but is no longer returned by any method in this version --
the `v1` terminal-only gate that used it was removed (see "Authorization"
above).
