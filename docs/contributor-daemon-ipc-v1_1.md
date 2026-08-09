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
- `list_projects` now reports **discovered** projects as well as configured
  ones, each with the mode actually in force and a `configured` boolean. An
  onboarding screen that asks a contributor to exclude a repository has to
  be able to list a repository nobody has ruled on yet.

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
> That job lands with the branch that adds it and has never run. Until it has
> run and passed, treat the ACL as unverified and do not describe it as
> working.

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
| `approve` | `entry_id` or `all: true` | `approved: <count>`, `hold_secs`, `hold_until` | `all: true` no longer requires a terminal; see "The approval hold" below |
| `dismiss` | `entry_id` | `ok: true` | |
| `cancel` | `entry_id` | `ok: true` | returns an `approved` entry to `pending`; guaranteed to succeed for the whole hold; error if not currently `approved` |
| `pause` | `until` (optional RFC 3339 timestamp) | `paused: true`, `paused_until` | see "Pause semantics" below |
| `resume` | — | `paused: false` | |
| `list_projects` | — | `projects[]` of `{project_id, project_label, mode, added_at, configured}` | configured **and** discovered projects; see "`list_projects`" below |
| `set_project_mode` | `project_id` **or** `project_key`, `mode` (`label` accepted and ignored) | `ok: true` | socket clients send `project_id`; `auto_upload` no longer requires a terminal; see "Naming a project" above |
| `list_history` | `limit` (optional, default 50, max 1000) | `history[]` | |
| `history_rollup` | — | see below | |
| `refresh_history` | — | `requested: true` | |
| `list_audit` | `limit` (optional, default 50, max 1000) | `entries[]`, newest first | see "Audit log" below |
| `queue_outcome_counts` | — | `reasons: {label: count}` | see "queue_outcome_counts" below; does **not** cover sessions never queued |
| `get_settings` | — | settings; credential and local paths reported as booleans only | |
| `set_settings` | any of `quiescence_secs`, `digest_interval_secs`, `approval_hold_secs`, `local_notifications`, `claude_root`, `codex_root` | updated settings | see "`set_settings`" below |
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
  "residual_risk": "pattern-based",
  "envelope_digest": "sha256:…",
  "input_fingerprint": "sha256:…",
  "enrolled": true
}
```

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

### `list_projects`

```json
{
  "projects": [
    {
      "project_id": "proj_9f2c1ab30d4e5f60",
      "project_label": "my-proj",
      "mode": "notify_only",
      "added_at": null,
      "configured": false
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

### The approval hold (the undo window)

`approve` returns:

```json
{ "approved": 1, "hold_secs": 10, "hold_until": "2026-08-08T12:00:10Z" }
```

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

`not_authorized` is retained in the error taxonomy for forward
compatibility but is no longer returned by any method in this version --
the `v1` terminal-only gate that used it was removed (see "Authorization"
above).
