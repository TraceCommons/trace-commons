# Contributor daemon IPC: `trace_commons.daemon.v1_1`

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

- `set_settings` now accepts `max_uploads_per_day` and `max_bytes_per_day`,
  each validated against a fixed ceiling rather than left open (see
  [`set_settings`](#set_settings)). Before this, the only way to raise either
  cap was to stop the daemon and hand-edit `daemon-settings.json`.
- `project_id` is an opaque, daemon-issued handle for a project. It appears
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
- `status.routing` reports whether the IronWire proxy overlay is declared and
  whether it is producing anything, in three states rather than two. Before
  this, a contributor who declared a proxy that never produced a row saw the
  same nothing as one who never declared it, and a declaration change did
  not take effect until the daemon was restarted. It now applies on the
  `set_settings` call. See ["`routing`"](#routing).
- `list_projects` now reports **discovered** projects as well as configured
  ones, each with the mode actually in force and a `configured` boolean. An
  onboarding screen that asks a contributor to exclude a repository has to
  be able to list a repository nobody has ruled on yet.
- `preview_turns` provides an index of turn boundaries **into the body
  `preview_body` already returns**. The transcript surface wants
  separators that identify `user` and `turn 1`, plus a `144 more turns` footer. It had
  nothing to place them from. This is strictly an overlay: `preview_body`'s
  request and response shapes are untouched, its bytes are unchanged, and
  every offset indexes that same string. The daemon deliberately does not
  re-render the events as turns -- that would drop `structured_payload`,
  `token_counts`, `latency_ms`, `cost_usd` and `failure_modes`, showing a
  contributor less than the artifact under a tab titled "exactly what would
  be sent". See ["`preview_turns`"](#preview_turns).
- `preview_request`, `preview_visible`, `preview_cancel`, and the
  `preview_ready` event form the **bounded** preview path. `preview` builds on
  the connection's time, which meant a shell drawing a list of N cards
  started N full read-parse-redact-serialize passes at once; on one
  contributor's machine (about 500 queued sessions, 11.7 GB across 4,097
  files, the largest 367.5 MB) that reached 1.7 cores sustained, 1.34 GB
  resident, and a load average of 649 inside three minutes. The daemon now
  owns a two-worker preview pool with deduplication, visible-first
  priority, cancellation, a size admission cap, and a result cache.
  `preview` itself is **unchanged** in request shape, response shape, and
  behaviour -- the CLI and the C ABI still use it. See
  ["Scheduled previews"](#scheduled-previews).
- `history_rollup` now carries an optional `community` object: this
  contributor's own line on the public roster, polled from the server's
  public snapshot on the daemon's own interval. Both desktop clients already
  draw a History community section and neither could populate it, because
  nothing on this contract carried the standing. Additive in the strict
  sense -- no existing `history_rollup` field changed shape, and a client
  that ignores the object is unaffected. See
  ["`history_rollup`"](#history_rollup).
- `history_detail`, reviewed public pages, and the seven `skill_*` methods
  add the account-owned session-to-skill flow. The daemon derives a candidate
  from one accepted correction, freezes the owner-reviewed Agent Skill,
  compares it with two controls through a NEAR-owned model, and exposes a
  signed, preview-before-write Codex installation with rollback. See
  ["Session detail and publication"](#session-detail-and-publication) and
  ["Tested skill workflow"](#tested-skill-workflow).
- `certificate_detail` provides a bounded read-only projection of a held
  witness certificate for native review surfaces. It returns signed claims,
  recovered signer, receipt state, and explicit expiry absence without
  exposing the stored envelope, signature, or certificate JSON.
- **K7 (history credit).** Three additive changes and one new method, all in
  service of the design's History screen:
  - `list_history` rows now carry `approved_unattended` and
    `approved_verdict`, recorded once at upload time rather than derived
    later, so a client can draw "you approved · Worked" versus "armed ·
    went without asking" without a second request. See
    ["History provenance (K7)"](#history-provenance-k7).
  - `history_rollup` now carries `taken_back`, and each of `week`,
    `month` and `all_time` a `withdrawn` bucket, for the design's "2 taken
    back" tile. A row withdrawn here or `revoked` by a withdrawal on the web
    counts there, once, and never in `other`.
  - `list_projects` now carries a top-level `unpurposed_traces`: scrubbed
    (previewed), undecided, Ask-me sessions, for the design's upsell
    sentence. See ["`list_projects`"](#list_projects).
  - `commons_credit_summary` is new: the commons' own settlement posture,
    read from the device route `GET /v1/contributors/me/settlement-posture`,
    and this contributor's points on its ledger, read from
    `GET /v1/account/credit-summary` with the account session. The daemon
    never read either before this. See
    ["`commons_credit_summary`"](#commons_credit_summary).
- **K9 (titles on the queue).** `title` -- previously served only on a
  `preview` summary -- is now also on every queue entry (`list_pending`, the
  `snapshot` event). It is never written to a receipt or a history row.
  The watcher computes it synchronously when it queues the session, from
  the same opening prompt and the same cut/truncate rule `preview`'s own
  `title` uses, but through the deterministic redaction pass alone rather
  than a full preview build. See ["`title`"](#title) and
  ["The preview content boundary"](#the-preview-content-boundary).
- **K10 (sizes in history, and the would-send size).** Two additive fields,
  so the History graph can weigh contributions by bytes and not only by
  count. `list_history` rows now carry `uploaded_bytes`: the serialized size
  of the redacted envelope a submission actually sent, recorded once at
  upload time. Every queue entry (`list_pending`, the `snapshot` event) now
  also carries `would_send_bytes`: the serialized size of the redacted
  envelope a preview pinned for that entry, present only once a preview has
  run. Neither is the raw session size on disk (`size_bytes`), which was the
  only figure either surface could report before this. See
  ["Sizes in history, and the would-send size (K10)"](#sizes-in-history-and-the-would-send-size-k10).
- **K12 (withdrawal dates on revoked rows).** `list_history` rows now carry
  `revoked_at`: when a server `revoked` read-back -- a withdrawal made on the
  web, which this daemon never drove -- was first observed, so a revoked row
  can show a date the way a locally `withdrawn` one already does from
  `withdrawn_at`. The server's read-back itself carries no timestamp, so this
  is the moment of first discovery rather than the moment of the web
  withdrawal; it does not move on a later poll that only re-confirms the same
  status. See
  ["Withdrawal dates on revoked rows (K12)"](#withdrawal-dates-on-revoked-rows-k12).
- **K15 (the native View menu's Group by and Sort by).** No field is added
  for this. Every Group by and Sort by option the native Traces tab needs
  is already answerable from `list_pending`'s entry fields, `list_projects`,
  and `list_history` -- once K11's `list_projects[].tools` (#1192) lands.
  K9's `title` (#1191) and K10's `would_send_bytes` and `uploaded_bytes`
  (#1196) have already landed. See ["View menu: Group by and Sort by
  (K15)"](#view-menu-group-by-and-sort-by-k15).

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
the hex is a SHA-256 prefix over the state directory path. The path itself
itself, which under the default layout carries the OS username, and which a
pipe name would expose to every process on the machine.

The Windows access control is entirely different in kind, and the difference
matters. A named pipe does not live in the state directory; it lives in the
machine-wide pipe namespace, where any local process may attempt to open it
by name. **Its DACL is the only thing protecting it.** The daemon builds one
granting the creating user's SID alone (`D:P(A;;GA;;;<sid>)`, protected so
no inherited entry can widen it) and refuses to serve if it cannot. There is
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
> excludes another user, which is a runtime property no cross-compile can observe.
> The observation is the `windows-pipe-acl` CI job
> (`scripts/windows/verify-pipe-acl.ps1`, driving
> `src/bin/win-pipe-acl-probe.rs`): on `windows-latest` it creates a second,
> non-administrator local account, has it attempt to open the pipe, and
> requires ERROR_ACCESS_DENIED, with a control confirming the owning user is
> still admitted. The account is deliberately not an administrator because one can
> take ownership of any object and would reach the pipe regardless, so that
> test would look like evidence while proving nothing.
>
> **That job has run and passed** (PR #247, run 31307072159): `DENIED 5` from
> the second user: ERROR_ACCESS_DENIED, refused by the access check
> specifically rather than by pipe-busy or file-not-found, which the script
> rejects so a coincidental refusal cannot pass as evidence. It also recorded `CONNECTED`
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
  approval -- `approve: {"all": true}` and `approve: {"project_id": ...}`
  alike -- appends a local, hash-only audit entry, readable via
  `list_audit`. A project-wide approval that matched nothing appends no
  entry, because nothing was approved. So do `set_consent_scopes` and
  `acknowledge_near_ai_notice`, and so does `cancel: {"project_id": ...}`
  -- undoing a batch approval is the same class of act as making one.
- The action and its audit entry are **one fail-closed unit**. If the entry
  cannot be persisted -- disk full, permissions, a corrupt log -- the action
  is rolled back and the call returns `audit-write-failed`. It does not
  succeed with a warning: an unrecorded change is exactly what removing the
  terminal-only restriction was not supposed to make possible.
- Arming records the terms it is granted under (see `auto-upload-voided`
  below). With no config to read them from -- not yet enrolled, or a config
  that cannot be read -- `set_project_mode: "auto_upload"` is refused with
  `arming-terms-unavailable` (`ERR_UNAVAILABLE`) before anything is recorded.
  Arming the unknown-project bucket is refused for its own reason first,
  also before anything is recorded.
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

## Project identifiers

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

An id resolves only against projects the daemon already knows: one already
in the policy, one sitting in the queue, or the `unknown-project` sentinel.
An id that resolves to none of those is refused with the fixed label
`project-id-unrecognized`, and nothing is recorded.

Knowing an id confers nothing. It is an identifier, not a capability: the
same call was always available to anyone who could name the directory, and
resolution is still limited to projects the daemon discovered on its own.

### Accepted `project_key` use

`project_key` is an absolute local path. It does not cross the socket in any
response, and no GUI should ever hold one. It remains an accepted
*parameter* for exactly one caller: a human in a terminal running
`daemon project <path> --mode ignore` **before that project's first
session**. This flow excludes a repository pre-emptively. The daemon
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
- `get_settings` reports five booleans -- `near_ai_configured`,
  `near_ai_inference_configured`, `near_ai_session_retained`,
  `claude_root_configured`, `codex_root_configured` -- and never the
  underlying values. The first three are credentials and are **different
  credentials for different services**, which their near-identical names do
  not convey: `near_ai_configured` is the privacy-filter API key,
  `near_ai_inference_configured` is the NEAR AI inference key this daemon
  mints for a contributor, and `near_ai_session_retained` is the NEAR AI
  session kept alongside that key so the account balance can be read. The
  session is the **widest** of the three -- a refresh token can mint further
  API keys and read organization and workspace state, where the inference key
  buys inference and nothing else -- and it is the one that most obviously
  must never cross this socket. The last two are local filesystem paths. All
  five are configured-or-not facts an app may render as a checkmark, never as
  text containing the actual value.
- `preview` returns a **summary** over the socket -- counts, labels, and
  sizes. The full redacted event body is a separate call, `preview_body`,
  because it does not fit one frame and has to be paged. Both carry trace
  content under the preview boundary below. Account-authenticated session
  detail and tested-skill extraction have their own bounded content boundary.
  The one other session-derived text is a queue entry's `title` (K9), a
  bounded exception described under the boundary below; no other method
  returns session-derived text.
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

### The preview content boundary

The preview interfaces deliberately carry trace content. The socket's
`opening_prompt`, the socket's `preview_body` chunks, and the C ABI's
`tc_preview_body` are all trace content. A contributor cannot consent to
sending something they cannot see; an approval given against a byte count
and a project name is not an informed one. This content is bounded:

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

**The one exception: a queue entry's `title` (K9).** Every queue entry, in
`list_pending` and the `snapshot` event, carries `title`: the first non-empty
line of the opening prompt, cut and truncated as `preview`'s `title` is. It
departs from the first two rules above, deliberately, and only these ways:

- **Deterministic redaction only.** The watcher builds it when it queues the
  session, through the deterministic pass alone (secret-leak patterns, local
  paths, private emails, PEM blocks), never a configured prose filter. So it
  is never less redacted than an unenrolled `preview.title`, but can be less
  redacted than an enrolled contributor's.
- **Served in bulk.** It is on every row of the queue a client already
  lists, without naming an entry.

It keeps the third rule in full. It lives on the queue entry and goes when
the entry goes. It is never written to a receipt, a history record, a log, an
audit entry or a notification, and nothing copies it onward. A history list
shows the project label, not a title.

`preview_unsure_spans` is outside the exemption for the same reason as
`preview_turns` below: it carries fixed labels and byte offsets into the body
the caller already holds, never the text at those offsets, and is served
under the same `unknown-entry-id` rule.

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
the rest of the exemption, plus three of their own:

- **Bounded in bytes.** One file per pinned entry, each at most the 16 MB
  envelope ceiling, and live entries are capped by `max_queue_entries` --
  but that pair alone would allow 7.8 GB of redacted trace content on a
  contributor's disk, which is not a bound anyone would choose. The store
  is held under a ceiling of its own (256 MB) by releasing the oldest
  pending previews.
- **Kept only while somebody is waiting on it.** The exemption is for an
  entry the contributor asked about, so a `pending` entry's stored envelope
  is released once it is three days old and the contributor has not acted
  on it; opening the entry again rebuilds and re-pins it. An `approved` or
  `uploading` entry is never released this way: its bytes are the bytes the
  upload will send.
- **Deleted when the entry resolves.** Uploading, refusing, failing,
  expiring, superseding, or revoking an approval all drop the file, and
  `logout` removes any that remain. If the bytes are missing or unreadable
  when the upload comes to read them, the approval is revoked and the entry
  re-offered -- never silently rebuilt.

They never cross the socket or the C ABI. `preview` reports the
`envelope_digest` that identifies them; it does not serve them.

Outside the preview, owned-session, and tested-skill response fields named in
this contract, the rule remains absolute: no unrestricted local path, token,
invite code, claim, device private key, or trace content appears in a log line,
error string, receipt, audit entry, notification text, or IPC response.

## Native onboarding state and bootstrap trust

`get_settings.admission_evidence_required` is an additive boolean derived from
configured witness admission policy. It is not proof of eligibility or a grant.
The separate `ironwire_attested_bodies` consent remains authoritative. If that
consent is off while admission is required, explicit witness review refuses with
`admission_receipt_unavailable` before transmitting a session. Clients must reread
`get_settings` after a failed settings write rather than display an assumed value.

The native wallet view carries `flow_id`, `state` (`Unsupported`, `Idle`,
`Checking`, `Ready`, `WaitingForWallet`, `Refused`, `Complete`), `busy`,
`can_check`, `can_start`, `can_edit`, `can_cancel`, `wait`, `message`, `tone`,
`glyph`, and optional `browser_url`. Shells render core copy and state, open the
browser handoff once, and request `wait` while requested by the view. Core owns
the two-second cadence and discards late replies after cancellation. Closing a
sheet sends `cancel`, including while start is pending. `Refused` uses the
returned refusal tone and glyph. Unsupported methods retain the invite flow.

`prepare_admission_session.view` adds `{ready, state, message, tone, glyph}` to
its existing result. Core requires both the expected status and a future expiry;
shells use `view.ready` rather than repeat readiness or clock rules.

Capability discovery bootstraps trust from allowlisted HTTPS at the ingest origin
chosen by the user (trust on first use). The advertised witness/issuer pins are
not independently authenticated merely because TLS succeeds. Operators must
provide an authenticated service origin; subsequent exchanges enforce the stored
pins. No account token, device key or PKCE verifier is returned to native views.

## Methods

| Method | Params | Result | Notes |
|---|---|---|---|
| `hello` | — | `schema_version`, `supported_versions[]`, `methods[]`, `events[]`, `max_line_bytes` | |
| `status` | — | see below | |
| `list_pending` | `project_id` (optional) | `pending[]` of queue entries | `project_id` narrows the list to that project's `pending` entries, for Customize's past-session picker; refused with `project-id-unrecognized` if the daemon does not know that project, or `project_id-invalid` if it is not a string. Absent is every project, as before. Each entry carries `scrub`, `marks` / `content_marks` / `unsure_spans` (only once its pinned bytes are counted) and `second_look[]`; see "The scrub state and `second_look`" below. Each entry also carries `would_send_bytes`, the pinned preview's measured size (K10); see ["Sizes in history, and the would-send size (K10)"](#sizes-in-history-and-the-would-send-size-k10) |
| `certificate_detail` | `entry_id` | held certificate claims and verification metadata | read-only; refuses entries without a witness pin and never returns raw artifact bytes |
| `route_disclosure` | — | `route`, `witness`, `local_filter`, `receipts`, `attested_bodies` | read-only, no network; what leaves this machine, to whom, and what this client checked; see "`route_disclosure`" below |
| `preview` | `entry_id` | see below | summary only; the body is `preview_body` |
| `preview_body` | `entry_id`, `offset` (optional), `limit` (optional), `body_digest` (required when `offset > 0`) | `chunk`, `next_offset`, `total_bytes`, `body_digest`, `envelope_digest`, `enrolled`, `max_chunk_bytes` | the redacted body, paged; see "`preview_body`" below |
| `preview_turns` | `entry_id`, `body_digest` (**required**) | `entry_id`, `body_digest`, `envelope_digest`, `turn_count`, `turns[]`, `leaves_this_mac` | an index of turn boundaries **into the body `preview_body` returns**; the body itself is unchanged. See "`preview_turns`" and "`leaves_this_mac`" below |
| `preview_unsure_spans` | `entry_id`, `body_digest` (**required**) | `entry_id`, `body_digest`, `envelope_digest`, `span_count`, `spans[]`, `spans_truncated` | byte ranges **into the body `preview_body` returns** that look like personal data the scrubber did not mark, each with a fixed label; offsets and labels only, never the text. See "`preview_unsure_spans`" below |
| `prepare_admission_session` | `entry_id`, `backend`, `confirmed: true` | `status: "ready_for_next_inference"`, `expires_at`, `view` | consent-gated challenge registration for the next inference; no funding or routing changes |
| `native_wallet_flow` | `action: open/check/start/wait/cancel`; `flow_id` after open; `ingest_url` for check/start; `account_id` for start | shared wallet view (see below) | owns capability checks, origin validation, polling cadence and cancellation; no new C ABI |
| `near_account_capabilities` | `ingest_url` | validated `ready`, issuer, audience and witness settings; on `ready: false` a `reason` of `address_refused`, `unreachable` or `unsupported` | checks allowlisted HTTPS service; no signup or funding. With `TRACE_COMMONS_ALLOWED_HOSTS` unset the allowlist is derived from `ingest_url` plus the issuer and witness hosts that origin publishes |
| `near_account_start` | `ingest_url`, `account_id` | `attempt_id`, `browser_url`, `status` | explicit wallet ceremony; keys and PKCE stay in daemon |
| `near_account_status` | `attempt_id` | `attempt_id`, `status` | no account token or signing material |
| `near_account_cancel` | `attempt_id` | cancellation state | cancels the matching local attempt |
| `near_ai_credential_start` | `provider` (optional, default `github`) | `attempt_id`, `browser_url`, `status` | asynchronous only; opens a sign-in whose result is an inference key kept on this machine. The browser URL is returned **once** and no poll re-serves it |
| `near_ai_credential_status` | `attempt_id` (optional) | `state`, plus `attempt_id` and `attempt_status` for a caller that named the current attempt | never errors on a missing or stale `attempt_id`: `state` is a fact about the machine. See "The credential state" below |
| `near_ai_credential_cancel` | `attempt_id` (**required**) | `attempt_id`, `status` | stops this machine waiting on the browser; `near_ai_credential_unknown` if the id is not the current attempt |
| `near_ai_credential_forget` | — | `removed`, `revoked: false` | removes the key from **this machine only**; `revoked` is always false and is not a placeholder. See "The credential state" below |
| `near_ai_credential_migrate` | — | `migrated`, `state` | asynchronous only; macOS. Copies the sign-in an earlier build kept in the login keychain into the data-protection store under the same reference, and never deletes the legacy entry. The one read of the login keychain, so it may raise a password prompt, which is why only a contributor's action sends it. `migrated: false` means neither store held it, and the dangling reference was dropped (`state` becomes `absent`). A refused legacy read is an error and changes nothing. See "The credential state" below |
| `witness_preview_request` | `entry_id`, `raw_session_confirmed: true`; optional `outcome`, `correction` | `status: "ready"`, `summary` | awaits explicit remote review; saves and pins certified bytes without approval or upload |
| `preview_request` | `entry_id` | `entry_id`, `state`, and the fields that state carries | enqueues and returns immediately; the result arrives as a `preview_ready` event. See "Scheduled previews" below |
| `preview_visible` | `entry_ids[]` | `visible: <count>` | replaces the on-screen set wholesale; decides preview **order**, never membership |
| `preview_cancel` | `entry_id` | `entry_id`, `dropped` | drops a queued preview, or discards a running one's result; `dropped: false` is a no-op, not an error |
| `approve` | `entry_id`, `all: true`, or `project_id`; `outcome` (optional); `correction` (optional, `entry_id` + `partly`/`failed` only) | `approved: <count>`, `hold_secs`, `hold_until`, `flagged`, `redactions`, `skipped[]` | `all: true` no longer requires a terminal; `project_id` approves that project's `Pending` entries and no others, matched by the id `entry_value` publishes (never `project_label`, which is display text and unstable), and is refused with `project-id-unrecognized` if the daemon does not know that project; the three are mutually exclusive and `all` wins over `project_id` wins over `entry_id` when more than one is sent; refused with `contribution-override-never` while a Never contribution override is in force (#1208); see "The approval hold", "What `approve` reports" and "The `outcome` verdict" below |
| `dismiss` | `entry_id` | `ok: true` | declines the **session**, not just this entry: the daemon never offers that session file again, however much it grows afterwards. See "`dismiss` is permanent" below |
| `keep` | `entry_id` | `kept: true` | "Keep on this Mac": the **reversible** decline. The entry must be `pending`, or `approved` unattended, which the keep revokes (`not-pending` otherwise, `unknown-entry-id` if there is none). See "`keep`: Keep on this Mac" below |
| `undo_keep` | `entry_id` | `kept: false` | returns a kept entry to `pending`, waiting for a person; `not-kept` for anything that is not kept, a dismissed entry included; `queue-full` at the queue cap; `project-ignored` if its folder is now Never. See "`keep`: Keep on this Mac" below |
| `list_kept` | — | `kept[]` of queue entries | every kept entry, in the `list_pending` shape, so a shell can show them and offer the undo |
| `list_past_sessions` | `project_id` (required) | `sessions[]` of past-session rows, `total`, `project_mode` | the first-run past-session picker: one folder's past sessions, queued or never offered, each named by an opaque `session_id`, never a path; nothing is read before the person chooses; `project_id-invalid` / `project-id-unrecognized`, never an empty list for an unknown folder. See "`list_past_sessions`" below |
| `include_past_sessions` | `project_id`, `session_ids[]` (1 to 500 distinct) | `approved`, `skipped[]` of `{session_id, label}` | the picker's Continue: a person's `approve` of exactly the sessions named, never `include_backlog`; the whole call is refused on one foreign id (`session-id-unrecognized`), a Never folder (`project-mode-never`) or the Never override; writes `past-sessions-included` first. Async entry point only, as `approve`. See "`include_past_sessions`" below |
| `cancel` | `entry_id` **or** `project_id` | `ok: true` (`entry_id`) or `canceled: <count>` (`project_id`) | returns matching `approved` entries to `pending` and clears their pin, so the next `approve` rebuilds; guaranteed to succeed for the whole hold; `project_id` undoes that project's `approved` entries and no others -- `pending` entries are left alone, matched by the id `entry_value` publishes (never `project_label`) -- and is refused with `project-id-unrecognized` if the daemon does not know that project; the two selectors are mutually exclusive and `project_id` wins if both are sent; a known project with nothing `approved` succeeds with `canceled: 0`; the single-`entry_id` form errors if that entry is not currently `approved`; see "The approval hold" below |
| `pause` | `until` (optional RFC 3339 timestamp) | `paused: true`, `paused_until` | see "Pause semantics" below |
| `resume` | — | `paused: false` | |
| `list_projects` | — | `projects[]` of `{project_id, project_label, mode, folder_mode, added_at, configured, is_unresolved_bucket}`, plus a top-level `unpurposed_traces` | configured **and** discovered projects; see "`list_projects`" below |
| `set_project_mode` | `project_id` **or** `project_key`, `mode` (`label` accepted and ignored), `include_backlog` (optional boolean, `auto_upload` only) | `ok: true`, `purged: <count>`, `retracted: <count>`, `from_now`, `overridden_by` (`null` or the override's mode) | socket clients send `project_id`; `auto_upload` no longer requires a terminal; see "Naming a project" above, "`set_project_mode` and the ignore purge" and "Arming from now" below |
| `set_contribution_override` | `mode` (`notify_only`, `auto_upload` or `ignore`), `confirm` (boolean; `true` required for `auto_upload`) | `changed`, `contribution_override: {mode, since}`, `returned: <count>` | the menu-bar pill's global override (#1173); per-folder modes are never written; `auto_upload` is a grant, refused with `arming-terms-unavailable` without terms (#1208); see "The contribution override" below |
| `clear_contribution_override` | — | `cleared`, `returned: <count>` | every folder back on its own mode; see "The contribution override" below |
| `mission_matches` | `catalogue` (a contribution mission catalogue; PROVISIONAL, shape owned by Z7/Z8) | `matches[]` of mission ids, `read: {tools, folders}` (counts) | K16 (#1173): which contribution missions this Mac's work fits, worked out on this Mac only; read-only; refused with `catalogue-required`, `catalogue-invalid` or `catalogue-schema-unsupported`; see "`mission_matches`" below |
| `list_history` | `limit` (optional, default 50, max 1000) | `history[]`, each row now also carrying `approved_unattended`, `approved_verdict`, `uploaded_bytes`, and `revoked_at` | see "History provenance (K7)", "Sizes in history, and the would-send size (K10)" and "Withdrawal dates on revoked rows (K12)" below |
| `history_rollup` | — | see below | now also carries `taken_back`, and a `withdrawn` bucket in each window |
| `commons_credit_summary` | — | `posture_state`, `commons_settlement`, `commons_settlement_explanation`, `commons_graded` (device route); `points_state`, `commons_points_earned_this_period`, `commons_points_lifetime_earned`, `commons_pending_review`, `commons_currency_code`, `commons_currency_earned_this_period`, `commons_period_start`, `commons_period_end` (account route); `observed_at` | always succeeds; see "`commons_credit_summary`" below |
| `history_detail` | `submission_id` | owned redacted outcome, correction, up to 24 evidence candidates, contributed versions, publication state, and an opaque local owner-scope digest | one account-authenticated request; each excerpt capped at 700 characters; native clients use the digest only to evict account-owned caches after sign-in changes |
| `skill_candidate` | `submission_id` | candidate, correction, up to six evidence excerpts, default draft, manual control, evaluation contract, and optional replacement review id | reads the same account-owned redacted record as `history_detail`; no model call |
| `skill_review` | `candidate_id`, `draft: {name, description, procedure}`, `replaces_review_id` (optional) | reviewed draft, complete `SKILL.md`, SHA-256, and source lineage | validates and freezes the exact skill locally; a changed prior review requires its id |
| `skill_evaluate` | `review_id`, `skill_sha256` | bounded three-arm evaluation report, source-exclusion record, regressions, and install verdict | uses one eligible NEAR-owned model; sends public fixtures and approved generic skill content only |
| `skill_install_plan` | `evaluation_id` | exact `SKILL.md`, signed marker JSON, separate preview digests, symbolic locations, occupancy, and install permission | performs blocking local inspection without writing the package |
| `skill_install_commit` | `plan_id`, `as_previewed_sha256`, `as_previewed_marker_sha256` | installed package identity, source lineage, digests, time, and symbolic target location | one-use commit of both previewed files; never accepts paths from the client |
| `skill_install_status` | `source_submission_id` | `installed`, plus verified `skill` or `null` | verifies exact signed on-disk state after restart |
| `skill_install_rollback` | `install_id`, `source_submission_id` | `removed`, `retained_directory` | makes the skill unavailable with one quarantine rename and retains a non-loadable backup |
| `publish_public_run` | `submission_id`, `draft`, `task_success`, `contributed_version`, `expected_publication_version` | reviewed public page | computes the approval digest over every public field and owner-state version |
| `unpublish_public_run` | `submission_id` | `unpublished`, `expected_publication_version` | idempotent account-authenticated request |
| `refresh_history` | — | `requested: true` | |
| `list_audit` | `limit` (optional, default 50, max 1000) | `entries[]`, newest first | see "Audit log" below |
| `queue_outcome_counts` | — | `reasons: {label: count}` | see "queue_outcome_counts" below; does **not** cover sessions never queued |
| `probe_routing` | `port` (required), `token_dir` (optional absolute directory) | `outcome`, plus `token_path` or `port` | asks a declared IronWire proxy whether it is there; performs real loopback I/O and **never returns the token**; see "`probe_routing`" below |
| `discover_routing` | — | `found`, plus `port` and optionally `token_path` when found | reads the pointer a running IronWire published, so the declaring flow can pre-fill instead of asking; no network I/O and **never returns the token**; see "`discover_routing`" below |
| `harness_list` | — | `catalog_present`, `harnesses[]`, `activity`, `spend`, `destination_port` | which coding tools this machine has and whether each sends its calls here; reads the filesystem on every call, never a cache; see "The harness list" below |
| `harness_plan` | `id` (required), `action` (`connect` \| `disconnect`) | `id`, `action`, `outcome`, `plan_id`, `path`, `changes[]`, `occupied[]` | works out an edit and **writes nothing**; see "The harness list" below |
| `harness_commit` | `plan_id` (required) | `id`, `action`, `committed: true`, `path`, `backup_path` | makes an edit that was already shown; takes a plan id and **nothing else**, so a shell cannot ask for a write it did not preview |
| `quiesce` | `timeout_secs` (optional, default 60, max 300) | `quiesced: true`, `waited_ms` | parks uploads for an update swap; `busy` / `quiesce-timeout` if in-flight work does not finish in time |
| `get_settings` | — | settings; credential presence as booleans, source declarations as `*_source_mode` (`unset`/`off`/`watch`), never local paths | |
| `set_settings` | any of `quiescence_secs`, `digest_interval_secs`, `digest_schedule`, `approval_hold_secs`, `local_notifications`, `claude_root`, `codex_root`, `claude_source`, `codex_source`, `gemini_source`, `cline_source`, `opencode_source`, `trajectory_source`, `ironwire`, `ironwire_attested_bodies`, `token_distributions_contribution`, `token_capture_enabled`, `private_inference`, `private_inference_offer_seen`, `scrub_check`, `max_uploads_per_day`, `max_bytes_per_day` | updated settings | see "`set_settings`" below |
| `consent_options` | — | `scopes[]` of `{name, title, description, always_on, grants_data_use}` | |
| `set_consent_scopes` | `scopes[]` (wire-name strings; omitted means floor scope only) | `consent_scopes[]` | requires an existing enrollment |
| `enroll` | `grant` xor `invite`, `scopes[]` (optional) | `enrolled: bool`, and on success `tenant_id`, `device_key_id`, `consent_scopes[]` | performs real network I/O |
| `acknowledge_grant_voids` | `ids[]` (**required**) | `acknowledged: <count>` | records that the void notices with these ids were shown; see "Void notices" below |
| `legacy_invite_migrate` | `invite` (optional: the invite link or code, sent only after `legacy_migration_invite_needed`) | `migrated: true`, `folders_kept`, `automatic_grant_kept`, `legacy_session_revoked` | async only; performs real network I/O; moves a legacy invite identity to the contributor's NEAR AI account at their request; refusals are `legacy_migration_*` labels; see "Moving a legacy invite identity" below |
| `acknowledge_legacy_invite_migration` | — | `acknowledged: bool` | records that the move notice was shown |
| `acknowledge_arming_rewordings` | `ids[]` (**required**) | `acknowledged: <count>` | records that the rewording notices with these ids were shown; see `arming_rewordings` below |
| `acknowledge_near_ai_notice` | — | `acknowledged: true`, `reoffered: <count>` | clears the `near-ai-notice-not-acknowledged` health label and re-offers the sessions it had refused; see below |
| `near_ai_balance` | — | `state`, `currency`, `scale`, `remaining_nanos`, `spend_limit_nanos`, `total_spent_nanos`, `total_requests`, `total_tokens`, `observed_at` | performs real network I/O; **always succeeds** and reports every way of not knowing as a named `state`; see "`near_ai_balance`" below |
| `set_public_profile` | `handle` (required), `bio` (required, string **or** `null`) | the profile, plus `handle_persisted` | performs real network I/O; replaces the whole profile; see "The public profile" below |
| `clear_public_profile` | — | the profile (now empty), plus `withdrawn: true` and `handle_persisted` | performs real network I/O; see "The public profile" below |
| `get_public_profile` | — | `on_roster`, `handle`, `bio`, `public_since`, `public_url` | a LOCAL cache, not a server read-back; `public_url` is always `null` |
| `subscribe` | — | `subscribed: true`, then a `snapshot` event | |
| `shutdown` | — | `stopping: true` | |
| `withdraw` | `submission_id` | `withdrawn: true`, `distribution_reach` | performs real network I/O; see "Withdrawal" below |
| `withdraw_bulk` | `status` (`submitted` \| `quarantined` \| `accepted`) | `withdrawn: <count>`, `failed: <count>` | performs real network I/O; see "Withdrawal" below |
| `inference_connection_offers` | — | `offers[]` of `{offer_id, revision, provider_id, disclosure_version, config_digest}` | account session; a read that selects nothing; see "Connecting inference" below |
| `inference_connection_current` | — | `selection` (or `null`), `installed_on_this_device`, `pending_install`, `revocation_applied` | account session; applies an observed revocation on this device; see "Connecting inference" below |
| `inference_connection_select` | `offer_id`, `provider_id`, `revision`, `config_digest`, `disclosure_version`, `confirmed: true` (all **required**, exactly as shown); `expected_current_version` (optional); `idempotency_key` (optional UUID) | `selected: true`, `connection_id`, `state_version`, `offer_id`, `revision`, `config_digest`, `disclosure_version`, `receipt_endpoint_offered`, `install_required: true`, `previous_witness_removed` | account session; installs nothing; see "Connecting inference" below |
| `inference_connection_install` | `connection_id`, `config_digest`, `confirmed: true` (all **required**) | `installed: true`, `connection_id`, `state_version`, `receipt_endpoint_installed` | account session; the separate explicit step that writes the witness on this device; see "Connecting inference" below |
| `inference_connection_disconnect` | `connection_id` (**required**) | `disconnected`, `connection_id`, `state_version` (or `null`), `local_witness_removed`, `server_disconnect` (`revoked` / `not-found` / `pending`), `server_refusal` (label or `null`) | removes the local witness first, with or without an account session; see "Connecting inference" below |

### `status`

```json
{
  "schema_version": "trace_commons.daemon.v1_1",
  "logged_in": false,
  "tenant_id": null,
  "consent_scopes": [],
  "paused": false,
  "queue_depth": 0,
  "decisions_owed": 0,
  "next_digest_at": null,
  "health": { "last_error_label": null, "since": null },
  "daily_budget": {
    "bytes_today": 204659969,
    "max_bytes_per_day": 209715200,
    "bytes_remaining": 5055231,
    "uploads_today": 12,
    "max_uploads_per_day": 50,
    "uploads_remaining": 38,
    "resets_at": "2026-08-22T00:00:00Z",
    "blocked": true,
    "blocked_entries": 14,
    "blocked_bytes": 137283584
  },
  "routing": { "state": "not_declared", "last_refresh_at": null },
  "grant_voids": [],
  "witness_capacity": { "waiting_sessions": 0, "next_retry_at": null },
  "arming_rewordings": [],
  "automatic_contribution_held": { "held_sessions": 0, "reasons": [], "projects": [] },
  "contribution_override": null,
  "contribution_mode": "notify_only",
  "contribution_mode_partial": false
}
```

`contribution_override`, `contribution_mode` and `contribution_mode_partial`
are additive; see "The
contribution override" below.

`grant_voids` is additive; see "Void notices" below. `witness_capacity`,
`arming_rewordings`, `automatic_contribution_held` and `decisions_owed` are
additive; see their sections below.

`legacy_invite_migration` is additive: `{"offered": bool, "notice": null |
{"folders_kept": n, "automatic_grant_kept": bool}}`. See "Moving a legacy
invite identity" below.

`health.last_error_label` is the field a tray renders when something is
wrong. It is one of the labels in "Health precedence" below, or `null` when
healthy.

The banner's words -- a title, a sentence stating what is held and the data
consequence, and an action label where there is a real recovery step -- are
`health_copy::health_copy_for_label`, across the C ABI as `tc_health_copy_json`
(R6/R7, #1173). Pass `reachable: 0` instead of a label when the daemon
cannot be reached at all; that returns the separate core-down sentence
(`health_copy::core_down_copy`, approved 2026-10-06 -- no shell had
shown a contributor-facing sentence for a fully unreachable daemon before).
`reachable` is never derived from this call: it is the caller's own
liveness fact, from whatever probe or IPC failure told it the daemon is
down. A reachable daemon with `last_error_label: null` has nothing to show
and should not call this at all; passed anyway with an empty label it
answers `NULL`, not a banner. A non-empty label that is not UTF-8 gets the
on-hold banner, never `NULL`.

The C signature is `tc_health_copy_json(reachable, label,
max_queue_entries)`: pass `get_settings.max_queue_entries` so `queue-full`
says how many are waiting, or `0` when unknown and the sentence names no
number. The JSON is `{title, detail, action, action_kind, severity}`:

- `severity` is `actionable` (the contributor can act on it) or `waiting`
  (it clears on its own) -- Swift `HealthCopy.Severity`'s two banner kinds.
  It cannot be derived from `action`: an unsupported OpenCode export is
  actionable with no button.
- `action_kind` is a stable kind for the button, present exactly when
  `action` is: `reconnect`, `privacy_scan_notice` or `review_queue` (Swift's
  `reviewsQueue`). Switch on it, never on the button's words.
- `opencode-export-version-unsupported` is answered with the source settings'
  own version sentence (`source_copy`), not the generic banner.

The core-down detail (draft) says the queue is safe and that sessions from
while the daemon was down are picked up when it is running again: the queue
is persisted, and the daemon's first poll after it starts is a full pass over
every watched source.

Moved into the core from
`macos/Sources/TraceCommonsApp/HealthCopy.swift`, which has shipped this
table since before this export existed; Windows
(`windows/src/TraceCommons.Interop/HealthCopy.cs`) independently wrote the
same sentences by hand. Neither shell has been switched over to the export
yet -- that is follow-up work, not part of this change -- so the two Swift
and C# tables still carry their own copies for now.

`next_digest_at` depends on `digest_schedule` (see `set_settings`). Under
`interval` it is `null` until a first digest has fired, then that digest's
time plus `digest_interval_secs` -- unchanged from before `digest_schedule`
existed. Under `evening` it is always a timestamp, whether or not a digest
has ever fired, and it honours the last digest: when an evening target has
passed that no digest answered (the daemon slept through it, or it had
nothing to say), it is that target, at or before now -- already due, the
same reading `interval` gives when last-plus-interval is in the past.
Otherwise it is the next occurrence of the configured local hour.

#### `daily_budget`

Added in `trace_commons.daemon.v1_1` as an additive field. It reports the
daily volume caps and, crucially, how much already-approved work they are
holding back.

It is deliberately **not** routed through `health`. The daemon does set
`daily-cap-reached` when a cap refuses an upload, but that label sits at the
bottom of the precedence order, so any other condition, including the full
queue that prompted this field, occupies the single
`health.last_error_label` slot and the cap becomes invisible. A client that
only reads `health` therefore cannot tell a spent budget from a broken
daemon. Read `daily_budget` independently of `health`.

| field | meaning |
| --- | --- |
| `bytes_today` / `uploads_today` | consumed so far in the current UTC day |
| `max_bytes_per_day` / `max_uploads_per_day` | the caps in force |
| `bytes_remaining` / `uploads_remaining` | what is left, saturating at zero |
| `resets_at` | the next UTC midnight, when the counters zero. Derived from the daemon's own day bucket, so a client may state it |
| `blocked` | whether at least one approved entry cannot be uploaded before `resets_at` |
| `blocked_entries` / `blocked_bytes` | how many, and their combined on-disk size |

`blocked_entries` counts every approved entry that will not go out today,
not only the ones that individually overflow the budget: the upload pass
stops at the first entry that does not fit rather than skipping past it, so
a small entry queued behind a large one waits as well.

`blocked` is false when the budget is spent but nothing is approved and
waiting. No contributor action is pending in that case.

#### `witness_capacity`

Additive. How many approved sessions are held because the privacy witness
answered `503 witness_saturated` (the pacing contract in
`trace_commons_protocol::witness_pacing`), and when the first of them is
tried again. Always present; `waiting_sessions` is `0` and `next_retry_at`
`null` when nothing is waiting.

| field | meaning |
| --- | --- |
| `waiting_sessions` | approved entries whose `reason_label` is `witness-saturated` |
| `next_retry_at` | the earliest `retry_after` among them (RFC 3339), or `null` |

The upload pass marks its witness requests
`x-trace-witness-workload: background`; a review a contributor asked for
sends no header. On a saturation refusal the entry keeps its approval, is
labelled `witness-saturated`, and is not claimed again before the witness's
own `Retry-After` (bounded to an hour; an unreadable value is the contract's
30 seconds), doubling while the witness stays busy, capped at an hour, plus
up to half the witness's delay of per-session jitter. Every later entry in
the same pass that would need the witness is held until the same time
without being sent, so a busy witness is asked once per pass, not once per
session. There is no attempt limit: a busy witness never costs a session.

It is beside `health` for the reason `daily_budget` is: the daemon also
sets `witness-saturated` in the health slot, set and retracted from the
queue on every upload pass, but a higher-precedence label can mask it.
Render the condition from this object. The words are
`consent_copy::witness_capacity_notice_for_wire` (`tc_witness_capacity_notice`
across the ABI); a shell renders `next_retry_at` itself, in local time,
beside the notice's `next_check` label.

Counts and timestamps only. No entry id, hash, or path appears here.

Approved entries are not listed by `list_pending`, which returns `pending`
entries only, so this is the only place the condition is reported; there is
no per-entry equivalent.

#### `arming_rewordings`

Additive. K5 of the connect-and-forget design: armed folders whose arming
words claimed a model scrubs their sessions (the "will be scrubbed" arming
offer, or the grant's model-scrub wording), where the words in force for them
now claim only the fixed patterns. The folder stays armed; this notice is how
the contributor is told what its arming now means. Always present; `[]` when
there is nothing to show.

```json
[{ "id": 0, "reworded_at": "2026-09-27T01:00:00Z",
   "project_id": "3f2a...", "project_label": "api",
   "was": "model_scrubbed", "now": "patterns_only" }]
```

Each element is named the way `list_projects` names the project, never by its
key. A notice is recorded once per narrowing, in the policy file with the
claim it records, so it survives a restart; the watcher checks on every pass.
It stays until a shell calls `acknowledge_arming_rewordings` with its `id`
(audited as `arming-rewordings-acknowledged`, no "all") or the contributor
sets the project's mode, which answers it. A void takes it with it. The
rewording itself is audited as `arming-reworded` with the project label.

An "Auto contribute" contribution override (#1208) is armed by its own
words and gets the same notice when they narrow: an element with
`"kind": "contribution_override"`, `project_id` and `project_label` both
`null` (no folder; the copy words it as unplaced, with no "Ask me first"
button), audited as `arming-reworded` with no project label. The override
stays on. Setting or clearing the override, or its void, answers it.
Elements for folders carry no `kind`.

For claim narrowings, every shell's arming offer currently says "will be
scrubbed" whatever R1's disclosure is, so the words in force have not
changed. That notice fires when the arming offer becomes disclosure-dependent
(`arming_wording::project_arming_claim`), or when a folder's disclosure drops
to patterns-only. The words are
`consent_copy::arming_reworded_notice_for_wire` (`tc_arming_reworded_notice`);
its `ask_first_action` button is `set_project_mode` with the element's
`project_id` and `notify_only`.

The Automatic-default upgrade also uses this persisted notice channel.
An element with `scrub_check_defaulted: true` announces the new review holds;
its `was` and `now` claims are unchanged. The shared formatter selects the
upgrade wording, including the existing Ask me first action. An upgrade and
a claim-narrowing notice can coexist and are acknowledged by their own ids.

#### `automatic_contribution_held`

Additive. What the automatic-contribution gate held at the last **full**
watcher pass: sessions in armed folders it is holding instead of approving
(for example, R3 before this commons admits the account), the unmet
requirements' reason labels, and each folder, named as `list_projects` names
it. Always present; zero, `[]` and `[]` when nothing is held, which is always
the case while `automatic_gate::ENFORCED` is off.

| field | meaning |
| --- | --- |
| `held_sessions` | sessions held, as `TickReport::gate_blocked` counts them |
| `reasons` | `automatic_gate::REASON_*` labels; empty when nothing is held |
| `projects[]` | `project_id`, `project_label`, `held_sessions` per armed folder |

Set and cleared only by a full pass; a scoped pass sees only changed paths and
leaves it as it was. In memory only: a restarted daemon's first full pass
measures it again. The daemon also sets `automatic-contribution-held` in the
health slot from the same pass, but a higher label can mask it, so render the
condition from this object. It is never acknowledged: it releases on its own
on the first full pass that finds the gate met. The words are
`consent_copy::gate_held_notice_for_wire` (`tc_gate_held_notice`).

#### `decisions_owed`

Added in `trace_commons.daemon.v1_1` as an additive field (K6). The menu-bar
badge's exact count: `Pending` entries that need a decision from this person.
It is **not** `queue_depth`, and the two are expected to diverge -- the design
rule is "the menu-bar badge counts decisions owed, never queue depth or
credit," and "armed folders never move it."

`queue_depth` is `list_pending`'s length, unchanged for compatibility: every
`Pending` entry, whatever folder it is in. `decisions_owed` excludes a
`Pending` entry sitting in an armed (`auto_upload`) folder with Automatic
Scrub check, because that
entry is going to be sent unattended once it settles (see
`eligibility::ARMED_SETTLE_SECS`) or once the automatic-contribution gate
clears (`automatic_contribution_held` above) -- the design never asks about
either, so armed folders must not move the badge whether they are merely
unsettled or gate-held.

These entries count even inside an armed folder, because they do need a
person:

- every pending entry when Scrub check is **Manual**, including newly
  discovered entries with no hold label;
- an entry `held_for_review` -- revoked for a reason in
  `REASONS_NEEDING_A_PERSON` that no unattended re-approval can satisfy;
- an entry returned by undoing **Keep on this Mac**: undo restores a
  personal decision, never unattended approval;
- a session held back by the automatic grant or **arm from now**
  (`waits_for_a_person_at_send`): backlog waits for a person rather than
  being swept into the arming. A kept entry itself is not pending and does
  not count.

A `Pending` entry in an `ignore` folder does not count (setting `ignore`
refuses what was waiting, so one there is only a transient, and the
contributor has already said "never offer these"), unless it is
`held_for_review` or returned from Keep. Manual Scrub check does not override
Ignore. `notify_only` ("Ask me") counts every `Pending` entry.

Computed fresh on every `status` (and therefore on `snapshot`, since
`subscribe` sends `status` inside it) from the queue, policy and Scrub check
setting, the
same way `witness_capacity` is: never a second stored counter that could
disagree with the rows it counts. `decisions_owed` and `queue_depth` are read
under the same queue lock, so one `status` never pairs two different queues.
The single function behind it, `queue::decisions_owed(queue, policy, scrub_check)`, is
written around one small predicate for exactly this reason: a later state
that must also count, or must not, extends that predicate rather than
growing a second badge-counting path elsewhere. Desktop shells consume this
field directly, never `queue_depth` or `list_pending` length. When connecting
to an older daemon that omits it, they show an unavailable count rather than
inventing a zero or reconstructing the daemon's policy locally.

A policy change can move this count without changing the queue: arming a
folder with entries waiting (`set_project_mode`), or the watcher arming a
newly discovered project under the automatic grant. Switching Scrub check
can also move it without changing an already-pending entry. When one does, the daemon
publishes `status_changed`, so a shell that refreshes status only on
`queue_changed` does not keep the old badge.

#### `routing`

Additive, and the protocol version is unchanged: `trace_commons.daemon.v1_1`
stays as it is. Every shell ignores keys it does not know. The Swift models
decode declared keys only, the GTK models carry no `deny_unknown_fields`, and
the Windows deserializer is left at its default, so an older shell against a
newer daemon behaves exactly as it did before, which is the rule this
document's "additive" status already states.

Whether the effective IronWire metadata overlay is enabled and whether it is
producing anything. Four ordinary states distinguish configuration from data;
`unknown` reports unavailable internal state without blaming a credential.

An explicit `ironwire: {"mode":"off"}` always disables this overlay. An
explicit Watch declaration keeps its exact port and token directory, including
while private-inference hosting starts or stops. With no explicit declaration,
a successfully persisted `private_inference: true` may derive metadata routing
from the proxy this daemon actually owns: its bound port and canonical home.
Discovery, an existing foreign instance, an unpersisted request, and a failed or
stopping proxy cannot supply that endpoint. Turning hosting off withdraws the
derived reader in the accepted settings transaction; cleanup need not finish
first. Terminal daemon cleanup also withdraws only the derived reader.

This derived declaration is not written to settings. `ironwire: null` removes
an explicit declaration, so automatic metadata can resume while owned hosting
is enabled; use explicit Off to refuse it. Derivation never grants body
collection: `ironwire_attested_bodies` still requires its separate opt-in and an
explicit Watch declaration. No agent configuration or tool endpoint is changed.
Unrelated settings writes retain a warm metadata reader when its effective
endpoint is unchanged. Derived refresh first reconciles owned lifecycle state, so
an already-observed proxy exit withdraws its reader before the next request.
This does not authenticate a later listener or cancel an already in-flight request.

| `state` | meaning |
| --- | --- |
| `not_declared` | no effective metadata declaration; the daemon holds no ledger and reads nothing |
| `awaiting_rows` | declared, and the daemon holds a ledger, but no row has arrived yet |
| `rows_seen` | declared, and the last refresh window had rows |
| `token_unreadable` | declared, and no ledger could be built: `control.token` could not be read |
| `unknown` | internal routing state is unavailable; no configuration or token diagnosis is implied |

Every routing snapshot includes `derived: true|false`, with these meanings:

- `true` identifies automatic metadata routing from this daemon's owned proxy.
- `false` identifies an explicit or absent declaration.

With `state: "unknown"`, false is a conservative default that provides no
evidence about metadata status; shells should present the effective status,
its origin, and the independent explicit declaration controls together. The
shared routing copy includes `derived_origin` for that explanation and
`state_unknown` for unavailable nonempty labels. `derived_origin` is descriptive
and cannot enable routing or body reading; older snapshots omit the explanation.

`token_unreadable` is what a declared proxy that is not running looks like,
and it is the one state a contributor has to act on. It was reported as
`not_declared` before it existed, which made a shell print "off" under a
switch the contributor could see was on. It is not `awaiting_rows` either:
that state says a reader exists and has seen nothing, and this one says no
reader exists. `last_refresh_at` is always `null` under it because nothing was
built, so nothing was ever checked.

`awaiting_rows` is **not an error** and a client must not render it as one.
A machine whose proxy was installed this morning reports it, and so does one
whose declaration changed a second ago: `set_settings` rebuilds the ledger in
place without a restart, and a rebuilt ledger starts cold by construction. Say
"nothing seen yet", not "broken".

The distinction `not_declared` carries cannot be recovered from row counts:
a daemon holding no ledger and a daemon holding a ledger that has read
nothing both have zero rows, and reporting them the same way tells a
contributor whose declaration never took that everything is fine.

`last_refresh_at` is when a refresh last **reached** the proxy and came back
readable (RFC 3339), or `null` when none ever has. It is not stamped on a
failed attempt, which is what makes it useful: rows say data exists, not that
the proxy answers now, and a proxy that died an hour ago still has rows. With
`awaiting_rows`, a non-null `last_refresh_at` means the proxy answered and
this window genuinely had nothing in it; a null one means nothing has
answered yet.

No port, token, or row content appears here.

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
(`list_pending`, the `snapshot` event), as do `eligibility` and
`eligibility_reason` -- see "Contribution eligibility" below -- and
`attestation` and `attestation_reason`, see "The attestation mark" below,
and `holds_certificate`, see "The certificate a queue entry holds" below.
`attestation` and `holds_certificate` are on EVERY entry; `eligibility` is
not. All are
additive; the schema version
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

`opening_prompt` is redacted trace content -- see "The preview content boundary"
above for why this one field is allowed to be, and what that permission does
not extend to.

#### `title`

`title` is a short name for the session, for a row or a sheet header: the
first non-empty line of the redacted `opening_prompt`, cut at a word boundary
to 60 characters with an ellipsis when cut, and `null` when there is no task
description.

**K9**: `title` is no longer confined to a `preview` summary. It is also on
every queue entry (`list_pending`, the `snapshot` event), as the one
exception to the preview content boundary -- see
["The preview content boundary"](#the-preview-content-boundary). It is never
carried onto a receipt or a `list_history` row.
The watcher builds it once, synchronously, at the moment it queues the
session, from the same opening prompt and the same cut/truncate rule this
`preview` field uses -- but through the deterministic redaction pass alone
(secret-leak patterns, known and generic local paths, private emails, PEM
blocks), never a configured prose privacy filter. The watcher's poll loop is
synchronous and runs on every discovered session; it cannot pay for that
filter's network call there the way a `preview` build can. That deterministic
pass is the floor every `preview`'s own redaction starts from and an
unenrolled contributor's `preview.title` never goes past, so the queued
`title` is never LESS redacted than an equivalent `preview.title` -- an
enrolled contributor's `preview` may additionally scrub prose PII that only a
configured filter catches, which the queued `title` cannot. `preview.title`
itself is unchanged: same field, same rule, independently computed on that
path.

`title` on a queue entry is `null` on an entry queued before this existed --
it is not backfilled, exactly like `shape` (see below): the entry gets a
title the next time its session is loaded (it grows, or is re-offered) --
and whenever the task named no description. A client that wants the
preview's fully-filtered title regardless of either still schedules previews
(`preview_request`, `preview_visible`) and fills each row in as its
`preview_ready` arrives.

### A session's shape on every queue entry

Every entry `list_pending` and `snapshot` return also carries when its session
ran and how many prompts it had. These are metadata, not content: timestamps
and a count, never a word of what was said, so they are on every row.

| Field | Meaning |
|---|---|
| `started_at` | the earlier of the source's own session start and the earliest event timestamp; `null` when neither is known |
| `ended_at` | the latest event timestamp, or `null` |
| `duration_secs` | `ended_at - started_at` in whole seconds, or `null` when either end is unknown or they are out of order |
| `user_turns` | how many prompts the person typed: user messages in the parent transcript that name a task (the same test the preview's `opening_prompt` uses), so injected wrappers -- system reminders, command metadata, AGENTS.md and environment preambles -- and a delegated subagent's own prompts are not counted |

`user_turns` is deliberately not `preview_turns`' `turn_count`, which indexes
every event of the redacted envelope. `ended_at` spans the delegated work
too, since it is part of the session. An entry queued before these fields
existed has them all `null`; it is not backfilled, and gains a shape when its
session is next loaded (it grows, or is re-offered).

Every entry also carries `title` (K9) -- unlike the fields above, redacted
*content*, not metadata, so it is documented on its own: see
["`title`"](#title). It also carries `would_send_bytes` (K10), the pinned
preview's measured size, documented in
["Sizes in history, and the would-send size (K10)"](#sizes-in-history-and-the-would-send-size-k10).

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

`redactions_distinct` (R7, #1173) is distinct values removed per label,
beside the occurrence counts in `redactions`: a client renders "185 local
path (12 distinct)" rather than just the total. It was previously only on
the full summary (a certificate entry's `preview`, and `tc_preview_summary_json`
across the C ABI); it is now on the card shape too -- `preview` for every
other entry, `preview_request`'s cache hit, and the `preview_ready` event --
since the review screen needs it on a card exactly as much as on the full
sheet. A count only, like every other field here: no value removed is ever
named.

**`preview` does not require an enrollment.** It performs no network I/O and
needs neither the daemon's file lock nor its running loop, so an app can
show a contributor what would be sent *before* they decide to enrol -- which
is when the question matters most. Through `v1_1`'s first releases this
refused with `unavailable` / `not-logged-in` unless a `contributor.json`
existed, which forced app harnesses to fabricate an enrollment purely to
preview a local file; that requirement was incidental and is gone.

`enrolled` says which kind of preview you got:

- `true` is the ordinary case. It is built from the real enrolled identity through
  the configured privacy filter, and (as described above) **pinned**: the
  envelope is stored and a later `approve` covers exactly those bytes.
- `false` means this device has no enrollment. The envelope is built from the
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

### The scrub state and `second_look`

The Flow 2 design shows each pending session with its scrub state
("Scrubbed · 7 marks") and marks some "worth a second look": sessions the
scrubber was unsure about, which wait for a person. The daemon decides that
once, in `daemon::second_look::second_look_reasons`, and publishes the answer
in additive fields on every queue entry (`list_pending`, `snapshot`, the
`entry` inside a `preview` response) and on every preview summary
(`preview`, `preview_request`'s `ready` summary, the `preview_ready` event):

| Field | Presence | Meaning |
|---|---|---|
| `scrub` | always | `scrubbed` or `not-yet-scrubbed` (see below). A summary is always `scrubbed`: it describes its own build. |
| `marks` | **only when `scrubbed`** | how many values the scrubber took out: the sum of `redactions` over labels that removed something. A `residual_secret_at:*` survivor is not a mark. |
| `content_marks` | **only when `scrubbed`** | `marks` without the path family (`local_path`). |
| `unsure_spans` | **only when `scrubbed`** | how many spans `preview_unsure_spans` would report for that build's body. |
| `second_look` | always, possibly empty | fixed reasons, in this order: `nothing-matched`, `looks-unsure`, `trimmed-to-fit` |
| `second_look_lines` | always, possibly empty (R6/R7, #1173; approved 2026-10-06) | `second_look`'s reasons, in the same order, each already turned into the sentence a person reads for it (`preview_copy::second_look_line`) |

`second_look_lines` exists so a card or the review sheet can render the
explanation without separately asking `tc_second_look_line_text` for each
reason; it is the same table, inlined. It is exactly as long as
`second_look` and lines up with it index for index -- never reordered,
never deduplicated, and never shorter: a reason this build has no sentence
for gets a generic fallback line rather than being dropped. The sentences it
quotes (`preview_copy::second_look_line`) were approved 2026-10-06; a
client that renders it should expect the words, not the presence or
absence of the field, to still change.

The reasons:

- **`nothing-matched`**: `content_marks` is `0`. The scrubber took out no
  personal detail -- only file paths, or nothing at all. Path removals are
  deliberately left out of this test: nearly every real session carries
  absolute paths, so counting them would make the reason almost never fire,
  and a session whose only marks were paths has had nothing personal found
  in it.
- **`looks-unsure`**: the unsure-span detector found at least one span (an
  email, phone number or key shape the scrubber did not mark), or could not
  index the body at all, which counts as unsure.
- **`trimmed-to-fit`**: `subagents_dropped > 0`; part of the conversation is
  not in what would be sent.

**Not yet scrubbed is not zero marks.** On a queue entry, `scrub` is
`scrubbed` only while the entry is pinned to an envelope **and** the counts
were taken from that exact envelope: they are stored beside its digest, and
only the pinning build (`preview_body`, `preview_turns`,
`preview_unsure_spans`, `approve`, a witnessed review) writes them. A card
(`preview`, `preview_request`) pins nothing and so records nothing, and an
unenrolled build is never pinned. When the pin is released, replaced or
revoked -- a re-enrolment, a privacy-filter change, an approval revoked and
re-offered, a stale pin released after three days -- the entry reads as
`not-yet-scrubbed` again, with nothing to clear by hand. Before then `marks`,
`content_marks` and `unsure_spans` are **absent** -- never `0`, never `null`
-- and `second_look` never contains `nothing-matched` or `looks-unsure`. A
shell must not render an unscrubbed session as "0 marks" or as clean; test
for the key. `trimmed-to-fit` can appear before any preview, because the
trim is decided at discovery.

**An empty `second_look` is an all-clear only when `scrub` is `scrubbed`.**
For a `not-yet-scrubbed` entry it means no reason is known yet. A caller that
would move a session on its own (the Scrub check's Automatic mode) must pin
and count it first.

Recording the counts publishes no event: the next `list_pending` or
`snapshot` carries them.

The Automatic Scrub check's hold (see `scrub_check` under `set_settings`)
counts the envelope it is about to send in exactly this way, and pins that
envelope with its counts when it holds the session, so a held entry reads
`scrubbed` with the reasons that held it.

Neither state is a colour: a shell renders the reason.

### Explicit witnessed review

The daemon exposes `witness_preview_request` through authenticated local IPC.
Clients must not offer it unless the daemon advertises that method. Existing `preview`, `preview_request`, card summaries, and background
refresh never invoke this builder. A configured witness still refuses ordinary
preview with `witness_claim_unavailable`, rather than building a local substitute.

The handler enforces these conditions:

1. Require an authenticated local IPC caller, an enrolled device, a pending entry
   the caller already holds, and explicit confirmation to send this session to
   the configured remote witness before redaction. Read the separate body-export
   consent, `ironwire_attested_bodies`, from current daemon settings; a missing
   confirmation ends the request before work begins.
2. Snapshot the selected entry's source hash and current configuration, including
   consent and witness pins, then call `preview::build_witnessed_preview` with
   `WitnessPreviewOptions { raw_session_confirmed, expected_session_hash,
   include_inference_bodies, verdict, correction }`. A missing/stale device,
   changed source, unpinned witness, failed claim, or unverified certificate
   refuses without a local-redaction fallback.
3. Use an ordinary authenticated upload-claim request, separate from admission
   and credit. Its explicitly echoed grant must be fresh and no wider than the
   requested permissions; the witness certifies bytes that include the granted
   scopes and uses. The helper uploads no contribution and persists no token.
4. On completion, take the queue lock and recheck that the entry is still pending
   and its source/configuration/consent match the snapshot; persist via
   `approved_envelope::save_witnessed`, then pin `summary.envelope_digest` and save
   the queue. A failed save blocks approval, and the result never passes through
   the local-envelope `save` function.
5. `witness-sha256:` pins identify the full versioned record: exact wire bytes,
   certificate/signature, source hash, configuration fingerprint, and approval
   answers; read through `load_witnessed`, check its digest and `validate` against
   current context, then use `envelope()` for existing body/turn/summary rendering.
   Missing, corrupt, partial, unknown-version, and legacy local records are refused.
6. The witnessed artifact is immutable. Corrections/verdicts must be supplied
   before that explicit review, and approval must match those answers exactly.
   A changed answer requires another explicit review; neither ordinary `approve`
   nor background upload may re-witness on the contributor's behalf. Keep local
   envelope approval semantics unchanged.
7. Upload obtains fresh authorization. All certified scopes/uses must match; a
   compatible 401/403 re-mint may resend the **same bytes and certificate**, while
   a changed grant re-offers the entry for review. No restamping, second redaction,
   or new raw witness request occurs. Existing source/fingerprint/residual-secret
   checks remain enforced. Record cleanup uses the existing pin lifetime/sweep.

The record stores only redacted artifact content and hash-only bindings; attached
inference bodies, bearer claims, and plaintext correction inputs are excluded.
Exact envelope bytes are base64-encoded only inside the atomic local record;
ingest receives the original bytes. The envelope remains bounded by the existing
size limit; the record including encoding/certificate overhead is limited to twice
that size, under the existing aggregate approved-artifact storage ceiling.

This enables a local implementation seam, not a deployed capability claim.
Live acceptance still requires configured trusted witness deployment, usable
provider receipt retrieval, and an invited end-to-end artifact check.

### Scheduled previews

`preview` is synchronous: the connection is held for the whole
read-parse-redact-serialize pass. That is correct for a caller that wants
one preview and is willing to wait for it, and it is what the CLI and the C
ABI still use. It is wrong for a shell drawing a list, because the natural
implementation -- one request per card -- starts one full pipeline pass per
queued entry, and nothing bounded how many ran at once. See the "New in this
revision" note for what that measured out to on a real machine.

`preview_request` is the same preview, scheduled. The daemon runs at most
**two** at a time, deduplicates, serves repeats from a cache, builds
on-screen entries first, and refuses sessions too large to be worth parsing
for a card.

**`preview_request`** takes `{entry_id}`. It always returns immediately,
with an object whose `state` is one of:

| `state` | Also carries | Meaning |
|---|---|---|
| `queued` | — | accepted; a `preview_ready` event will follow |
| `running` | — | already building from an earlier request; a `preview_ready` event will follow |
| `ready` | `summary` | answered from cache; **no event follows**, because no work was done |
| `too_large` | `raw_session_bytes`, `limit_bytes` | refused by admission control; nothing was parsed |
| `failed` | `code`, `label` | the pipeline refused; same fixed labels `preview` uses |

`summary` also carries `scrub`, `marks`, `content_marks`, `unsure_spans`
and `second_look`, describing that build (see "The scrub state and
`second_look`" above). Beyond those, it carries exactly the fields
`preview` returns -- `would_send_bytes`,
`raw_session_bytes`, `event_count`, `opening_prompt`, `redactions`,
`pii_labels_present`, `consent_scopes`, `residual_risk`, `envelope_digest`,
`input_fingerprint`, `enrolled` -- with one deliberate omission: it does
**not** carry `entry`. The summary is cached and a queue entry's state
changes underneath it, so embedding one would let a cached preview assert a
stale state. Clients already hold entries from `list_pending` and
`snapshot`.

Calling this repeatedly for the same entry is free and is the intended
usage: a cached result comes straight back, and an entry already queued or
building is not enqueued twice.

**`preview_visible`** takes `{entry_ids: [...]}` and replaces the daemon's
idea of what is on screen, wholesale. Send it after each scroll settles; it
takes one lock and moves no work. Visibility decides the **order** previews
are built in, never whether they are built: an entry that scrolls away keeps
its place in the queue. An unparseable id is `bad_params` /
`entry-ids-invalid`; an empty array is valid and means nothing is on screen.

**`preview_cancel`** takes `{entry_id}` and returns `{entry_id, dropped}`.
`dropped: false` means there was nothing to drop -- already finished, never
requested, already cancelled -- and is not an error, because a client that
cancels on every card leaving the list will hit it constantly.

> **What cancel cannot do.** A preview that has already started cannot be
> interrupted. The pipeline has no cancellation point in it, and its
> expensive stretch is a single parse the daemon does not get control back
> from. Cancelling a running entry costs exactly the CPU that job had left
> to spend; what it buys is that no `preview_ready` event is published and
> nothing is cached. The bound on that waste is two jobs, which is why the
> two-worker bound rather than cancellation is what protects the machine. Do
> not build a client on the assumption that cancelling is immediate.

`dismiss` cancels an entry's scheduled preview implicitly. `approve` does
not -- it needs the envelope it would be cancelling.

### `dismiss` is permanent

A queue entry is identified by the hash of its content, so a dismissal
recorded on the entry alone was a decision about a byte range: the moment
the contributor typed the next message into the same conversation it hashed
to something else, and the watcher offered it again as a brand-new
`pending` card. In a project set to `auto_upload` the re-offer arrived
already `approved`, so a declined conversation uploaded unattended.

`dismiss` therefore declines the **session**, addressed the way the daemon
addresses every session -- by its path. Once dismissed, the watcher skips
that session for good: no new `pending` card, no `approved` one, and no
re-read (the skip sits in front of the load, so a declined conversation
someone keeps working in costs nothing per poll); arming a project cannot
override it because `auto_upload` applies only to sessions without a ruling.

There is no un-dismiss. The record is the dismissed entry itself, which
lives in the queue file with reason `dismissed-by-contributor` and is never
compacted, so dismissals made by older daemons are honoured with no
migration. A `refused` entry carrying any *other* reason is the daemon's
verdict on some bytes rather than the contributor's on the conversation,
and does not suppress anything.

**Too large.** A session over the admission cap is refused rather than
previewed, and the refusal carries **no size estimate of any kind**:
`raw_session_bytes` is a `stat` of the file, `limit_bytes` is the cap, and
there is no `would_send_bytes` and no `summary`. A would-send number is a
claim about an envelope that was never built, and the preview card is a
consent surface. Render "too large to preview" and the raw size; do not
synthesize a would-send figure from the raw bytes. This is a *preview*
policy only -- it does not decide whether such a session can be contributed,
and `approve` still builds and pins one.

**Caching and staleness.** A result is cached against the session file's
path, size, and mtime, the entry's whole-group size, and a fingerprint of
the local configuration. Any of those changing rebuilds. The cache lives in
the daemon process and does not survive a daemon restart.

### `keep`: Keep on this Mac

Open decision #4 in #1118 asked whether the review sheet's "Keep on this Mac"
is a permanent dismissal or leaves the session pending. `keep` is the
reversible answer, built beside `dismiss`, which is unchanged.

A kept session, exactly:

- **Stays on this Mac, and is not sent.** The entry moves `pending` to
  `refused` with `reason_label: "kept-on-this-mac"`. An entry the watcher
  approved **unattended** can be kept too: the keep revokes that approval in
  the same step, so a keep pressed while the folder's rule re-approves the
  card does not lose the race. A person's own approval is `cancel`led first.
  Nothing uploads a
  `refused` entry, and `approve` -- by `entry_id`, `project_id` or `all` --
  acts only on `pending`, so an `approve` naming a kept entry sends nothing
  and answers `not-pending` until the keep is undone. Any preview pin goes with the keep; an
  undo previews afresh.
- **Is not owed.** It is out of `list_pending`, so out of the Ask-me list and
  out of `status.queue_depth` (the badge), and out of every group `approve`.
- **Stays kept across passes.** Like a dismissal it is a decision about the
  **conversation**, answered from the session's path: while it is kept the
  watcher skips that session -- no new `pending` card, no re-read -- however
  much it grows.
- **Is never sent unattended**, including after its folder is armed, plainly
  or from now: arming applies only to sessions without a ruling, and the path
  skip sits ahead of it.
- **Never expires.** Expiry applies to `pending` entries only, and nothing
  compacts `refused` (only `superseded` is compacted), so a kept session is
  not silently lost to the 14-day clock or to compaction.
- **Is undoable**, with `undo_keep`. The entry returns to `pending` with
  `reason_label: "returned-from-keep"`, dated now, so the expiry clock
  restarts from the undo rather than from when it was first offered. It then
  **waits for a person**: the watcher does not approve it on anyone's behalf,
  in an armed folder included. A person's `approve` -- single or group --
  sends it and clears the label; `keep` or `dismiss` rules on it again. If
  the session grew while it was kept, the next pass re-offers the grown
  content as a fresh `pending` entry that carries the same label, and waits
  in the same way.
- **Cannot clear a hold.** An entry held for a person when it was kept --
  a witness risk review, a token-distribution review, any reason that keeps
  a session out of unattended and group approval -- comes back from the undo
  with that same label, not `returned-from-keep`. The label is remembered on
  the entry while it is kept. Its preview pin is not: the undone entry is
  reviewed afresh.
- **Comes back only where it can be offered.** `undo_keep` is refused with
  `queue-full` when the queue is at its cap, like any new offer, and with
  `project-ignored` when the session's folder is now Never; either way the
  session stays kept, and the undo can be asked for again.

**Expiry after an undo, and of the backlog.** A `returned-from-keep` entry,
and a backlog entry an arming from now leaves waiting, are ordinary waiting
cards: they expire on the normal 14-day clock, which for the first starts at
the undo and for the second is the clock the card already had (an unattended
approval returned by an arming is re-dated to the arming). They are not
exempt, on purpose. The cap counts waiting cards, so exempting a whole disk's
history would fill the queue for good and stop new sessions being offered;
and a card the contributor has not decided in 14 days is the case expiry
exists for, in an Ask-me folder or not. An expired entry is not re-offered
while its file is unchanged, as for every expired card. A contributor who
wants a session kept past that uses `keep`, which never expires.

What it is not:

- **Not a dismissal.** `keep` never writes `dismissed-by-contributor`, and
  `undo_keep` refuses a dismissed entry with `not-kept`. There is still no
  un-dismiss.
- **Not a withdrawal.** It acts on sessions not yet sent. A session already
  uploaded is taken back with `withdraw`.
- **Not kept across logout.** Logout clears the queue, keeps included, as it
  clears dismissals.

`queue_outcome_counts` counts kept sessions under `kept-on-this-mac`.
`list_kept` returns them in the entry shape `list_pending` uses. `keep`
cancels any scheduled preview for the entry, as `dismiss` does. Neither
method writes an audit row, as `dismiss` does not: they are decisions about
one session, not autonomy changes.

### `set_project_mode` and the ignore purge

Setting a project to `ignore` also clears whatever it has waiting. Two
kinds of entry move to `refused` with `reason_label = "project-ignored"`,
and the response counts them separately:

- `purged`: `pending` entries for that project -- the cards the contributor
  was shown as waiting.
- `retracted`: `approved` entries the daemon approved **unattended**, under
  the project's standing `auto_upload`. Nobody decided about those bytes, so
  excluding the project is the contributor's first decision about them.

A client comparing the result with the number of waiting cards it showed
MUST compare against `purged` only; `retracted` entries were never on screen
as waiting. Both fields are always present and may be `0`.

**Turning automatic off retracts too.** Setting a project to `notify_only`
moves the same unattended `approved` entries back to `pending`, with their
old approval's terms cleared, and counts them in `retracted`; `purged` is
`0`, since no waiting card is removed. They become ordinary waiting cards
for the contributor to decide, rather than refusals: turning automatic off
means "ask me", not "never offer this". Before this, only `ignore`
retracted, so turning automatic off left every session it had already
approved uploading.

An `approved` entry the **contributor** approved is deliberately untouched,
and so is an `uploading` entry. A contributor's approval is a decision
already made about specific bytes under specific consent scopes, and a
project-level preference set afterwards does not retract it -- so a project
with three waiting and one approved by hand loses three and still uploads
one. A client MUST say so rather than let a contributor discover it.

An unattended approval that was `uploading` at the moment of the change and
did not complete -- the daily cap, a fail-closed precondition, a restart --
returns to `approved`. It is not sent: the upload pass re-checks the
project's mode before sending any unattended approval, refuses it as
`"project-ignored"` if the project is now `ignore`, returns it to `pending`
if the project is now `notify_only`, and emits `queue_changed`. That arrives
on a later pass, not in this response. A project key the policy cannot
resolve falls back to `notify_only`, so a lookup miss now waits for the
contributor rather than sending.

`"project-ignored"` is not `"dismissed-by-contributor"`. A dismissal is
permanent and suppresses that conversation at its path forever; this is a
verdict on whatever was queued at the moment the mode changed, so setting
the project back to `notify_only` or `auto_upload` lets those sessions be
offered again -- including sessions that are finished and will never be
written to again. Leaving `ignore` drops that project's `"project-ignored"`
entries outright -- purged, retracted and refused-at-send alike -- which is
what lets the watcher re-offer them; `dismissed` and pipeline refusals in the
same project are untouched. The re-offer
arrives on a later poll, not in this response, and is still subject to the
queue cap.

A client MUST NOT present the purge as irreversible, and MUST NOT rely on
the entries reappearing within any particular time.

### Arming from now

**Arming is from now by default.** `set_project_mode` with
`mode: "auto_upload"` arms a folder for **new** sessions; what is already on
disk waits for the contributor, who sends it by picking it in the
past-session picker. That applies the Flow 1 grant's rule ("it arms nothing
already on disk", `grant_automatic` below) to every folder a contributor arms,
whatever shell arms it. Only `include_backlog: true` sends the backlog too:

- **A session already on disk when the folder was armed -- queued as
  `pending` or not yet seen -- is never approved unattended in it**, whichever
  project it reads as later. It stays `pending`, waiting for the contributor.
- A session that first appears **after** the arming is approved unattended,
  as in any armed folder, through the same settle window and gate.
- What is on disk is recorded **per source** by the next full watcher pass,
  before any session is visited, and only for an arming made before that pass
  listed the disk. Until a source is recorded for the arming, nothing from it
  is approved unattended in the folder, so a harness connected or re-rooted
  after the arming has its history recorded before it can send anything. A
  session created between the arming and that pass waits too: fail closed.
  A source whose listing cannot be read -- its root, or any directory under
  it, unreadable for a reason other than not existing -- reports a failed
  discovery rather than an empty one, so it stays unrecorded for that pass
  and holds everything. The automatic grant's recording follows the same
  rule.
- **Defence in depth for content at a new path.** The record is by path, so a
  conversation older than the arming that turns up in a new file -- a resume
  into a fresh file, a restore, a sync -- is also held when its first event,
  or its file's birth time, is earlier than the arming. Where the filesystem
  reports no birth time and the session no first-event time, the path record
  alone applies.
- **A re-arm keeps the hold.** `auto_upload` without `include_backlog` over a
  folder already armed from now keeps the record of the first arming: the
  backlog still waits, and a session that arrived in between is still new.
- Arming from now over an arming **with** the backlog returns what that
  arming approved unattended and has not sent yet to `pending`, counted in
  `retracted`. It was on disk, so it is backlog. An `approved` entry a
  **person** approved is untouched, as for every mode change.
- The send path re-checks too: an unattended approval that comes back from
  `uploading` to `approved`, whose session must wait for a person -- on disk
  when its folder was armed from now, or on disk at the automatic grant in a
  folder the grant armed -- returns to `pending` rather than being sent.
- The arming writes the `armed-auto-upload` audit row with
  `detail: "from-now"`; an arming with the backlog writes it with no detail.
  Labels only, as every audit row. The result's `from_now` says which it was.
- `list_projects` reports `from_now: true` on an armed row armed from now and
  `from_now: false` on any other armed row; the key is absent on rows that
  are not armed, like `automatic_disclosure`.
- Setting the folder to ask-first or Never ends the from-now record, and
  `include_backlog: true` replaces it: the backlog is then approved
  unattended. Recorded paths no arming can still hold are pruned.

**Folders armed before this default are left as they are.** A folder armed
with the old meaning has no from-now record, so it keeps sending what it
did; nothing is migrated. Setting its mode again -- `auto_upload` included --
applies the rule in force at that call.

`include_backlog` is a boolean. Any other type is refused with
`include-backlog-invalid`, and `include_backlog: true` with a mode other than
`auto_upload` with `include-backlog-requires-auto-upload`: a backlog means
nothing for ask-first or Never, and ignoring it would let a shell believe it
had set a rule it had not. Both are `bad_params`, and a refusal records and
changes nothing. The CLI's `daemon project --mode auto` arms from now and
says the backlog waits; `--include-backlog` sends it.

#### The past-session picker

"Past sessions, by folder" needs no new consent. It lists `pending` entries
grouped by `project_id` -- `list_pending` with a `project_id` returns one
folder's -- and each session the contributor ticks is an ordinary per-entry
`approve`, which is a person's decision and goes through the approval hold
like any other. A folder armed from now leaves exactly those entries waiting,
so the picker and the rule compose: the rule covers what comes next, the
picker what is already there. Unticked sessions stay `pending`; "Keep on this
Mac" (`keep`) takes one out of the list without deciding it for good.

The first run's picker (#1030's Rules step) needs more than the queue holds:
on a first run nothing has been offered yet, and a session the watcher never
visited is not in the queue at all. It uses `list_past_sessions` and
`include_past_sessions`, below, which walk the declared sources themselves.

### `list_past_sessions`

```json
{ "method": "list_past_sessions", "params": { "project_id": "proj_..." } }
```

```json
{
  "sessions": [
    {
      "session_id": "sess_0123456789abcdef0123456789abcdef",
      "entry_id": "5f0c...-uuid",
      "state": "pending",
      "selectable": true,
      "started_at": "2026-10-01T09:12:00Z",
      "duration_secs": 1840,
      "title": "Fix the flaky upload test",
      "size_bytes": 482113,
      "source": "claude-code"
    },
    {
      "session_id": "sess_fedcba9876543210fedcba9876543210",
      "entry_id": null,
      "state": "not_queued",
      "selectable": true,
      "started_at": "2026-09-14T16:40:00Z",
      "duration_secs": null,
      "title": null,
      "size_bytes": 90211,
      "source": "codex"
    }
  ],
  "total": 2,
  "project_mode": "notify_only"
}
```

One folder's past sessions, newest first, whatever the queue knows of them.
The listing walks every declared source's discovery itself rather than the
watcher's cwd cache, so it is complete before the first discovery pass has
run (a Rules step shown seconds after Folders started the daemon) and it
includes sessions no pass has ever visited. It never loads a session: a row
the queue has not offered is described by its date and size alone.

`project_id` is required. A missing or non-string one is `bad_params` /
`project_id-invalid`; an id that resolves to no project the daemon knows --
the usual known set plus every project a declared source lists now -- is
`bad_params` / `project-id-unrecognized`. An unknown folder is refused, never
answered with an empty list. `project_mode` is the folder's resolved rule
(`notify_only`, `auto_upload` or `ignore`).

Each row's `state`:

| `state` | `selectable` | Meaning |
|---|---|---|
| `pending` | true | queued and waiting on a person |
| `approved` | false | already approved |
| `expired` | true | aged out of the queue without a decision; including it revives it |
| `not_queued` | true | on disk, never offered |
| `never` | false | the folder's rule is Never; every row it would list is `never` |
| `still_active` | false | still being written: modified within `quiescence_secs` |

A queued row (`pending`, `approved`, `expired`) carries its queue entry's
`entry_id`, `started_at`, `duration_secs`, `title`, `size_bytes` and `source`
(the declared source where the entry has one). A `not_queued` row carries
`entry_id: null`, `title: null`, `duration_secs: null`, the discovery's
`started_at` (else the file's modification time) and the file's
`size_bytes`: nothing is read before the person chooses. `title` is `null`
on a queued entry that has none, as on `list_pending`.

Kept and dismissed sessions are not listed. Outside a Never folder, neither
is a session whose latest offer was decided some other way (uploading,
uploaded, refused, failed): the picker is for sessions still open to a
choice. A Never folder lists those too, as `never`. `total` is the number of rows
returned.

**No path crosses the socket.** `session_id` is `sess_` and the first 32 hex
characters of sha256 over the session path's bytes: one-way, and
deterministic, so the id a listing gave is the id `include_past_sessions`
re-derives from its own walk. No row carries a path, a cwd or a project
path; the folder is named only by `project_id`.

### `include_past_sessions`

```json
{
  "method": "include_past_sessions",
  "params": {
    "project_id": "proj_...",
    "session_ids": ["sess_0123456789abcdef0123456789abcdef"]
  }
}
```

```json
{
  "approved": 1,
  "skipped": [
    { "session_id": "sess_fedcba9876543210fedcba9876543210", "label": "session-still-active" }
  ]
}
```

The picker's Continue: approve exactly the sessions named, as a person's
approval. Each one is pinned to a preview and held for the undo window
exactly as a click on a card is (`approve` and this method share one
implementation), and it is recorded as the person's own, so a later change
of the folder's rule -- back to Ask me, say -- does not take it back.
"Include every past session in {folder}" is a selection of every id the
listing returned, never `include_backlog`.

Dispatched on the async entry point only, as `approve` is; the synchronous
entry point answers `unknown_method`.

**The whole call is validated before anything changes.** Each of these
refuses every session and records nothing:

| Code / label | When |
|---|---|
| `bad_params` / `project_id-invalid` | `project_id` missing or not a string |
| `bad_params` / `session_ids-invalid` | `session_ids` missing, not an array, holding a non-string, or empty once duplicates are dropped |
| `bad_params` / `too-many-sessions` | more than 500 ids, checked before any id is read |
| `bad_params` / `project-id-unrecognized` | the project resolves as for `list_past_sessions` and does not |
| `bad_params` / `contribution-override-never` | the global Never contribution override is on, as `approve` refuses it |
| `bad_params` / `project-mode-never` | the folder's rule is Never |
| `bad_params` / `session-id-unrecognized` | any one id is not one of this folder's sessions in the daemon's own walk -- another folder's id, a path, an empty string, a made-up id |
| `unavailable` / `audit-write-failed` | the `past-sessions-included` audit row could not be written |

Duplicate ids are counted once, in the order first named. A session the
walk can no longer find is `session-id-unrecognized`, not a skip: the call
names what the person saw, and a vanished session refuses the selection.

**The audit row comes first.** Once the call validates, a
`past-sessions-included` row is appended to the audit log (`list_audit`)
before anything is revived, queued or approved: `project_label` is the
folder's derived label (from the key the daemon holds, never the caller's
string) and `detail` the number of sessions chosen. No session id, path or
title is in it. A log that refuses the write refuses the call, under the
same rollback-cannot-record guarantee as `bulk-approved`.

**One refusal comes after the audit row.** If the queue cannot be saved once
the approvals are made, the call is `unavailable` / `queue-write-failed`:
every approval it made is cancelled again (back to `pending`, its pin
dropped, as `cancel` leaves an entry), so nothing it named is sent. The
`past-sessions-included` row stands, as `bulk-approved`'s does after a
failed save. Sessions it revived from `expired` or queued for the first time
are left `pending`, waiting for a person, and a retry with the same ids
approves them.

**Per session**, every distinct id is counted in `approved` or listed in
`skipped` exactly once, with a fixed label:

| `label` | Meaning |
|---|---|
| `session-dismissed` | the session was dismissed; it is never revived |
| `session-kept` | the session is kept on this Mac; `undo_keep` first |
| `session-still-active` | still being written (judged at the walk, and again at the read); never queued half-written |
| `held-for-review` | its offer is held for a person's review, or was when it aged out, as a group `approve` leaves it |
| `not-pending` | already decided (approved, uploading, uploaded) by the time the include reached it |
| `session-project-changed` | read now, the session resolves to another folder than the one named |
| `project-mode-never` | the folder turned Never between the validation and the read |
| `session-unreadable` | the session file could not be read or parsed |
| `session-file-vanished` | the session file was gone by the read |
| `envelope-too-large` | over the size limit, at the read or at the pin |
| `queue-full` | the queue refused the offer outright |
| `not-enrolled`, and `approve`'s other per-entry labels | the approval itself was refused, as `approve` reports it |

An `expired` session is revived to `pending` (its `discovered_at` set to
now, as `undo_keep` dates a return) and approved. A `not_queued` session is
read and queued now, then approved; an explicit selection lands past the
queue's entry cap, because it is a person's choice rather than the watcher's
offer, but never past the quiescence check. A session queued by a discovery
pass that ran between the listing and the include is approved through that
offer if it still waits.

A session read here for a folder armed from now, with content older than
that arming, is recorded on the arming as the watcher records it, so if the
approval does not land the offer it leaves still waits for a person.

### The contribution override

The menu-bar Contribution mode pill (R13, #1202; requested on #1173) offers
three global overrides: **Ask me** (`notify_only`), **Auto contribute**
(`auto_upload`) and **Never** (`ignore`). `set_contribution_override` sets
one; `clear_contribution_override` clears it.

**Per-folder modes are never written.** The override is one value in the
policy file beside the folders' own entries, and the mode in force is read
over the folder's own mode. Clearing it drops that value, so every folder is
back on exactly the mode it had. `set_project_mode` still works while an
override is in force: it sets the folder's own mode (`list_projects`
`folder_mode`), and the override still decides `mode` until it clears. Its
reply then carries `overridden_by`: the override's mode when the override
still decides that folder (for example a folder set to Ask me under Auto
contribute, whose new sessions are still approved unattended), and `null`
when the folder now resolves to what was set -- always so with no override,
and always so for Never, which every override leaves alone. A shell shows
the change as saved but not yet in effect when `overridden_by` is present.

What each override does, with the stricter rule winning:

| Override | Folder set to Automatic | Folder set to Ask me, or never ruled on | Folder set to Never | Unresolved bucket |
|---|---|---|---|---|
| `ignore` | Never | Never | Never | Never |
| `notify_only` | Ask me | Ask me | **Never** | Ask me (or Never if set) |
| `auto_upload` | Automatic, under its own holds | **Automatic from now** | **Never** | Ask me (or Never if set) |

- **Never** queues and sends nothing from any folder (#1208). Nothing
  waiting is refused: entries already `pending` stay `pending` (they leave
  `decisions_owed` while it is in force), and an unattended approval not yet
  sent goes back to `pending` rather than being refused, so clearing the
  override restores both. An entry the **contributor** approved is held,
  not sent: it stays `approved`, with its pin and hold, and the send path
  skips it (re-checked when it claims each entry, so a Never set mid-pass
  stops the next one); clearing the override releases it to send as it
  was. `approve` is refused while Never is in force (`bad_params` /
  `contribution-override-never`), before anything is approved or audited.
  An upload already in flight when Never is set is not recalled. Sessions
  finished while it is in force are not queued; once it clears, the next
  pass offers them under each folder's own mode -- an Automatic folder
  sends them unattended.
- **Ask me** leaves nothing to go unattended. Every unattended approval not
  yet sent in a folder that no longer resolves to Automatic goes back to
  `pending` (counted in `returned`), and the send path returns any that
  comes back from `uploading`. A folder set to Never stays Never.
- **Auto contribute** is the consent-sensitive one, and is held to the
  arming paths' rules:
  - **It arms nothing already on disk.** For every folder not already armed
    by its own mode it is an arming from now (see "Arming from now"): what is
    on disk when it begins, queued or not, is recorded per source by the next
    full pass and waits for a person; until a source is recorded, nothing
    from it goes. Only a session that first appears after it is approved
    unattended. A folder already armed keeps its own holds (an arming from
    now, the automatic grant's) and nothing more.
  - **A folder set to Never stays Never.** A global override does not reach
    into a folder the contributor excluded.
  - Every other gate still applies: the Scrub check (Manual holds
    everything), review holds, the settle window, and the
    automatic-contribution gate.
  - It is refused without `confirm: true` (`bad_params` /
    `confirm-required`; a non-boolean `confirm` is `confirm-invalid`). A
    shell sends it only from the confirmation the core words
    (`tc_contribution_override_confirm_json`), which carries the arming
    disclosure.
  - **It is a grant** (owner decision on #1208), held to everything a
    per-folder arming is. Without grant terms in force (no config, or one
    that cannot be read) it is refused, `unavailable` /
    `arming-terms-unavailable`, before anything is recorded -- the label
    `set_project_mode` uses. It records the terms in force and the claim its
    words made. On every pass the grant sweep compares those terms with the
    terms in force, as it does a folder's (see `auto-upload-voided`): a
    widening **clears the override**, so every folder is back on its own
    mode and none resolves to Automatic because of it, and the pill shows the
    folders' roll-up again. What it approved unattended and has not sent, in
    folders that then ask first, goes back to `pending`. The void is told
    like any other: a `grant_voids` element of kind `contribution_override`,
    the audit row `contribution-override-voided`, and `status_changed`
    (`queue_changed` too if entries went back). Folders armed by their own
    mode void exactly as before, with their own notices. An override saved
    with no recorded terms (only a pre-release build could write one) is
    voided with the reason `terms-unrecorded` rather than baselined. A K5
    rewording reaches it too (see `arming_rewordings`). It still creates no
    `automatic_grant` and no per-folder `armed-auto-upload` row. Folders it
    arms report the `patterns_only` disclosure.
- Re-sending the override in force changes nothing (`changed: false`): no
  audit row, and an Auto contribute override keeps the hold it had.
- Clearing an Auto contribute override returns what it approved unattended
  and has not sent, in folders that then ask first, to `pending`.

**Audit.** Label-only, never a folder: `contribution-override-set` with
`detail` the mode, `contribution-override-cleared` with `detail` the mode
that was in force, and `contribution-override-voided` with `detail` the
comma-separated reason labels of `auto-upload-voided` (or
`terms-unrecorded`). For `auto_upload` the row is written **before** the
override takes effect, and an audit failure refuses the call
(`unavailable` / `audit-write-failed`). A stopping override and a clear are
never refused for want of an audit row, and record after. The "same mode
again" check and the row are made under the one policy lock that applies
the change, so two identical concurrent sets write one row.

**Writes and events.** A policy file that cannot be written rolls the
override back (`unavailable` / `policy-write-failed`) and nothing changes. On
every change `status_changed` is published, and `queue_changed` too when
entries were returned to `pending`. A queue file that cannot be written after
that keeps the in-memory truth, publishes both events exactly as success
does, and answers `unavailable` / `queue-write-failed`.

**Status.** `status.contribution_override` is `null`, or `{mode, since}`
while one is in force. `status.contribution_mode` is the pill's roll-up, so
no shell computes it: the override's mode while one is in force; otherwise
the one mode every folder shares (`notify_only`, `auto_upload`, `ignore`), or
`mixed`. "Every folder" is every configured folder and every folder with a
session in the queue, by its own mode, excluding the unresolved bucket, which
can never be armed. With no folders it is `notify_only`, the default.
`status.contribution_mode_partial` (#1208) is `true` when the roll-up is
`auto_upload` but some of those folders do not upload: one set to Never, or
the unresolved bucket (whose sessions ask first). It is `false` for every
other roll-up. A shell shows the copy's `auto_partial` line under the
label exactly when it is `true`.

**Copy.** `tc_contribution_mode_copy_json` is the pill (title, `Mixed`, the
three choices with their sub-list lines, the override line, `clear`, and
`auto_partial`). `clear` describes the Mixed row, whose choice clears an
override in force; it is a sub-line, not a button label. A shell's list
checks what the pill shows: the override's own mode while one is in force,
otherwise the roll-up (the Mixed row only for `mixed`), and nothing while the
status is unread or the daemon is down. `tc_contribution_override_confirm_json(mode, config_dir)` is each
confirmation; for `auto_upload` it carries `arming`, the Flow 1 grant
screens' disclosure table for the configuration in `config_dir`. For
`mode` `"clear"` it returns the confirmation a shell shows before
`clear_contribution_override` when any folder's own setting is Automatic, or
when the folder list or a folder's own setting is unknown: its `mode` is
`"clear"` (never a `ProjectMode`, never sent to `set_contribution_override`)
and it carries no `arming`. Every sentence was approved 2026-10-06
(`project_copy.rs`).

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
1.5 MB envelope ceiling while a socket line is capped at `max_line_bytes`
(1 MiB), so the method promises no single frame. Read from `offset: 0`, append
each `chunk`, and follow `next_offset` until it is `null`; `total_bytes` gives
the whole length, and receiving less than `[0, total_bytes)` means the trace
remains incomplete.

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
can draw separators that identify `user` and `turn 1`, plus a `144 more turns` footer over a
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
`tool: bash, turn 3` once rather than putting a boundary between a
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

#### `leaves_this_mac`

`preview_turns` also returns the review sheet's "Leaves this Mac" line, so no
shell writes its own claim about what the envelope carries:

```json
"leaves_this_mac": {
  "fields": ["conversation", "tool", "tool-version", "timing", "outcome",
             "uses", "redaction-summary", "session-id", "trace-ids",
             "contributor-id", "tenant", "revocation-handle",
             "folder-fingerprint", "replay", "scores", "format-version"],
  "would_send_bytes": 19456,
  "turn_count": 12,
  "folder_named_in_conversation": false,
  "folder_named_in_metadata": false,
  "line": "19 KB · 12 turns · tool, tool version, timing, outcome, …. The metadata never carries the path or the folder name."
}
```

`fields` is derived by **walking every key of the serialized envelope** that
would be sent (`consent_copy::leaves_this_mac_fields`) and naming it, so it
is complete by construction. The set is closed and listed in this order:
`conversation`, `tool`, `tool-version`, `model`, `timing`,
`usage-and-cost` (per-event token counts and cost), `routing` (routing
rows), `outcome`, `correction`, `uses`, `redaction-summary`
(`privacy.redaction_counts` and the rest of the privacy block),
`session-id` (`conversation_id`, the tool's own session id, and
`source_session`), `trace-ids`, `contributor-id`, `tenant`
(`contributor.tenant_scope_ref`), `credit-account`, `revocation-handle`,
`folder-fingerprint` (`cwd_hash`), `replay`, `scores`, `format-version`, and
`other`. A label appears only when the envelope carries a non-null key for
it. A key this build has no name for is reported as `other` -- over-described,
never dropped -- and a test fails the build on any such key. Labels, never
values.

`would_send_bytes` is the envelope's size, `preview`'s figure; `turn_count`
equals the method's own. `line` is the finished sentence (approved
2026-10-06), absent only if the envelope could not be measured.

**What the line promises about the folder is only what is true.** Only
absolute paths are scrubbed out of the conversation, so a relative path
(`../myproj/src/main.rs`) or a sentence can still name the folder. The daemon
looks for the folder's own names (the basenames of the project root, the
unfolded path and the session's working directory) as standalone,
case-insensitive words, separately in the conversation (`events`) and in
everything else, and reports both as booleans:

- the line always says "The metadata never carries the path or the folder
  name." -- unless `folder_named_in_metadata` is true, when it says "The
  folder's name appears in what would be sent." instead;
- when `folder_named_in_conversation` is true it adds "The conversation
  itself names the folder."

The list does **not** say "project label", which the design's mock does: the
envelope has carried no project name, in the clear or hashed, since #207.

### `preview_unsure_spans`

Where the redacted body holds something that looks like personal data the
scrubber did not mark, so the review sheet can put "looks like an email. Not
matched. Your call." under that line. **Offsets and labels, never the
text**: the shell already holds the body from `preview_body` and renders the
span from it.

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
  "span_count": 1,
  "spans": [
    { "label": "looks-like-email", "byte_offset": 1408, "byte_len": 11 }
  ],
  "spans_truncated": false
}
```

`label` is one of `looks-like-email` (an address, or a bracket-obfuscated one
such as `name [at] example [dot] com`), `looks-like-phone` (international
`+44 20 7946 0958`, or North American `(415) 555-0100` / `415-555-0100`) and
`looks-like-key` (a well-known credential prefix such as `sk-`, `ghp_`,
`AKIA`). The set is closed. `byte_offset` and `byte_len` are a half-open
range of **UTF-8 byte** offsets into the body, on character boundaries,
sorted and never overlapping.

**Conservative.** Only the contents of JSON strings are scanned, and a span
never crosses a JSON escape (`\n`, `\"`, `é`), so an offset can never
point into the middle of one. A redaction placeholder
(`<PRIVATE_EMAIL_1>`, `[REDACTED]`) never matches, so an email the scrubber
took yields no hint. A bare run of digits is never a phone. A hint is a
pointer for a person, not a second scrubber: nothing is redacted and nothing
waits because of one.

**`body_digest` is required, on every call,** on exactly `preview_turns`'
rule: omitted is `bad_params` / `body-digest-required`, a non-string is
`bad_params` / `body-digest-invalid`, and one that does not match the body
the daemon resolved is `unavailable` / `preview-body-changed` -- re-read the
body from `offset: 0` and ask again. The body is resolved through the same
path as `preview_body` and `preview_turns`, with the same refusals
(`unknown-entry-id`, `approved-envelope-unavailable`).

**Fail-closed.** A body the detector cannot index exactly, or any span that
does not re-verify against the bytes it points at, refuses the whole
response with `unavailable` / `preview-unsure-index-failed`. `spans` holds at
most 2000 entries; `span_count` is always the total, and `spans_truncated`
is `true` when the list was cut. A shell must not present a cut list as the
whole one.

`preview_unsure_spans` is answered on the async dispatcher only; the
synchronous entry point refuses it with `preview-unsure-spans-requires-async`.

The C ABI's `tc_preview_unsure_spans_json(handle, entry_id, body_digest,
err)` returns the same object, from the same function, under the same
anchoring and refusals, and like `tc_preview_turns_json` refuses an attached
handle with `preview-requires-embedded`.

### `list_projects`

```json
{
  "projects": [
    {
      "project_id": "proj_9f2c1ab30d4e5f60",
      "project_label": "my-proj",
      "mode": "notify_only",
      "folder_mode": "notify_only",
      "added_at": null,
      "configured": false,
      "is_unresolved_bucket": false,
      "session_count": 22,
      "last_session_at": "2026-09-12T11:18:00Z",
      "pending_count": 7,
      "contributable_count": 3,
      "tools": [
        { "source": "antigravity", "session_count": 2, "answers_at": "Google" },
        { "source": "claude-code", "session_count": 20, "answers_at": "Anthropic" }
      ]
    }
  ]
}
```

`session_count` is how many sessions the watcher has observed in this
project that were still on this machine at the last full pass, whatever their
queue state: a session whose file is deleted stops being counted once a full
pass in which every source listed cleanly has seen it gone. `last_session_at`
is the latest of those sessions' file modification times -- when it was last
written, not when it started -- or `null` when none has been observed. Both
come from the daemon's per-session record of each session's project, recorded
when the watcher resolves it, so answering reads and canonicalizes nothing. A tool's own totals are in `tc_discover_sources` (`session_count`,
`most_recent`).

#### `tools` (K11)

Which tool (or tools) this project's sessions came from, each with its own
`session_count` within this project -- the design's first-run screen says
"Repos found in Claude Code sessions"; this is what lets a later screen say
the same thing about any project, for any tool, once more than one has
contributed to it.

Each row names a `source`: an adapter name (`claude-code`, `codex`,
`gemini-cli`, `cline`, `opencode`) for a session the daemon watched directly,
or the declared source a trajectory file names for one that was staged
(`trajectory`'s own imports carry their vendor's name, e.g. `antigravity` for
an imported Antigravity conversation -- see "Antigravity" under `harness_list`
below). This is the same preference a queue entry's own display already
applies: what a session declares about itself over the adapter that happens
to store it, so an imported conversation is never attributed to `trajectory`.

For a staged import the `source` is **self-declared**: it is the
`meta.source` field of a file anyone can drop into the staging directory,
checked for shape (`validate_source_name`) but not verified. A staged
trajectory that claims `claude-code` is counted here as Claude Code and
reads `answers_at: "Anthropic"`. The effect is local display only; nothing
decides routing, consent or upload on this field.

`answers_at` is the same fixed vendor word `tc_discover_sources` and
`harness_list` read `answers_at` from, so a project's tool breakdown cannot
name a vendor differently than either surface does for the same tool.
`null` for a tool this build has no fixed default for (Cline, OpenCode) or
does not recognise.

Always an array, empty rather than absent when nothing is known yet -- a
client tests its length rather than testing for the key, the way `tools`
being empty and `session_count` being `0` already agree with each other.
Rows are ordered by `source`, alphabetically, not by count.

**This can undercount `session_count`.** The per-tool breakdown is read from
the same per-session record `session_count` already comes from, but the tool
a session came from was not recorded before this field existed, and a cache
entry whose file has not changed since is never rewritten just to backfill
it. A project can therefore show `"session_count": 22` with its `tools[]`
entries summing to fewer than 22 on a daemon upgraded from an older state
file, until those sessions' files change again. Never read `tools[]` as a
second, more detailed `session_count` -- read `session_count` for the total
and `tools[]` for what it can say about the sessions it has re-seen since.

`pending_count` is how many `Pending` entries this project holds.
`contributable_count` is how many of those a group-level `approve` would act
on -- what a shell needs to draw "send the 3 of 7 that can be sent" without
enumerating rows and classifying them itself.

**`contributable_count` is ABSENT when eligibility does not apply**, on the
same rule as an entry's `eligibility` field: an invited contributor has no
"3 of 7" to be told about, and `pending_count` alone is their answer. Test
for the key; it is never null.

Both numbers, never one. "Submit all (7)" quietly becoming "Submit all (3)"
is worse than either: the contributor sees a smaller number with no
explanation and cannot tell whether four sessions vanished or were never
counted. A control that counts the sendable ones needs the total beside it,
and `pending_count - contributable_count` is what
`tc_contribution_withheld_line` turns into the sentence that closes the gap.

**Absent, never zero.** A client that read a zero where the question does not
apply would draw "Submit all (0)" and offer nothing to an invited contributor
whose sessions are all perfectly sendable. When the key is missing, count the
button off `pending_count` and render no withheld line.

**`contributable_count: 0` means the group's submit control is not offered.**
A header offering "Submit all" where nothing is eligible is a press with no
visible consequence -- the row-level rule ("shown, not offered") one level up.
The group still renders, with its rows and their sentences; only the control
goes. Decide it through `tc_contribution_group_control(pending,
contributable)`, passing **any negative value** for an absent
`contributable_count`, so the absent-versus-zero distinction is made once
rather than in three shells.

It is a count and not a promise. Entries can move between this call and the
approve, and the expensive checks still run at submit -- see "Contribution
eligibility" below.

Every project the daemon knows about, in two kinds:

- **configured** (`configured: true`, `added_at` set) means the contributor has
  ruled on it with `set_project_mode`.
- **discovered** (`configured: false`, `added_at: null`) means the daemon has
  seen a session for it and nobody has ruled on it. `mode` is the effective
  mode, which for an unruled project is the `notify_only` default.

`mode` is the mode **in force**, which a contribution override can change;
`folder_mode` is the folder's own mode, the one `set_project_mode` set and
clearing the override returns it to. They differ only while an override is
in force. See "The contribution override" below.

An armed row also carries `from_now`: `true` when it was armed from now,
the `set_project_mode` default, so its backlog waits for the contributor,
and `false` when it was armed with `include_backlog: true` or before that
default existed. A row armed only by an "Auto contribute" override reports
`from_now: true` too, since the override holds its backlog the same way.
Absent on rows that are not armed. See
"Arming from now" above.

#### `is_unresolved_bucket`

True for exactly one row: the bucket holding sessions whose working directory
had no usable final segment. `Policy` prevents those sessions from being armed
for automatic upload, independently of any client, so a shell showing the row
with a permanent note is **reporting** enforcement. `Ignore` still applies and
can silence the bucket.

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
configured-only list contains decisions already made, so it can never
contain the repository the contributor is being asked to exclude. Nothing
new crosses the socket: a discovered row carries the same two
daemon-derived fields (`project_id`, `project_label`) that the queue entry
for that project already carries.

`mode` is always the mode in force, not the stored value: the
`unknown-project` bucket reports `notify_only` even if a hand-edited policy
file says `auto_upload`, because the daemon refuses to act on that.

#### `unpurposed_traces` (K7)

A top-level count beside `projects[]`, for the design's upsell: "27 scrubbed
sessions are sitting on this Mac under folders set to Ask me. None has been
decided." It is the number of queue entries that are, all three:

- **`Pending`** -- undecided. An entry already `Approved`, `Uploaded`,
  `Refused`, `Expired`, or `Superseded` has already been decided, one way or
  another, and is not part of a backlog waiting on a person.
- **In a project whose mode resolves to `notify_only`** ("Ask me"). An
  **armed** (`auto_upload`) project is excluded on its resolved mode, not on
  the entry's own approval state -- a gate-held armed session is also
  `Pending` with nothing decided about it (see the design's note that
  "gate-held armed sessions stay Pending and would inflate the badge"), and
  counting those here would send a contributor to go decide about a folder
  they already armed.
- **Previewed at least once** (`previewed_envelope_digest` is set on the
  entry) -- "scrubbed", in the design's word. An entry nobody has opened a
  preview for has not been through the redaction pass this count is about.

This differs intentionally from K6's `status.decisions_owed`: the upsell
promises previewed Ask-me sessions only, while the badge includes unpreviewed
sessions and armed sessions that require a person. Neither changes the
other's number.

Like `pending_count` and `contributable_count`, this is a plain count with no
side effects, computed from the same queue and policy state a client already
fetches `list_projects` to draw. It is always present (never absent, unlike
`contributable_count`), because whether a session is undecided-and-Ask-me is
a question every contributor's queue can answer, invited or not.

### `mission_matches`

K16 (#1173). Which **contribution missions** fit this contributor's work,
matched on this Mac, under the consent design's "Missions" rules (M1, M2).
These are not the published mission packages (`GET /v1/missions`).

```json
{"method": "mission_matches", "params": {"catalogue": {
  "schema_version": 1,
  "missions": [
    {"mission_id": "m-rust", "title": "Rust refactors",
     "criteria": {"tools": ["claude-code"], "tool_families": ["anthropic"],
                  "languages": ["rust"], "min_sessions": 2}}
  ]}}}
```

```json
{"matches": ["m-rust"], "read": {"tools": 1, "folders": 2}}
```

**The catalogue.** A parameter, for now: the daemon fetches nothing for
this call. The server catalogue is Z7/Z8's, and when it exists the daemon
will download it with a request that is the same for every contributor;
nothing per-contributor is sent for it either way. The shape is
**PROVISIONAL, owned by Z7/Z8** (`contribution_missions::ContributionMissionCatalogue`):
`schema_version` (1) and `missions[]` of `{mission_id, title, criteria}`.
`criteria` lists are each "any of", and an empty or absent list does not
restrict: `tools` (session sources, as `list_projects`' `tools[].source`
names them), `tool_families` (`anthropic`, `openai`, `google`), `languages`
(from marker files at a folder's root, such as `Cargo.toml` for `rust`).
A session fits when it satisfies every non-empty list; a mission matches when
at least `min_sessions` (default 1, never less) readable sessions fit.
Unknown fields are ignored, so a catalogue with later additions still
loads; a newer `schema_version` is refused with
`catalogue-schema-unsupported`; a malformed one, a duplicate `mission_id`,
or one over a bound (256 missions, 200 characters, 32 values a list) with
`catalogue-invalid`; no catalogue with `catalogue-required`.

**What matching reads (M1).** Only the sessions the daemon has already seen
(the cache `list_projects` counts tools from), and of those only sessions
from an **adapter** that is on -- one whose session folder is watched; an
imported Antigravity conversation is read by the trajectory adapter, so it
counts as that, not as `antigravity` -- in a folder whose mode in force is
not Never. The gate is the adapter that actually discovered a session
(`SessionRef::source`, recorded as `CwdCacheEntry::adapter`), never the
self-declared, contributor-facing name a staged import can claim
(`SessionRef::displayed_source`): a trajectory file staged with
`meta.source: "claude-code"` is read only once trajectory itself is on, not
because claude-code is. A cache entry written before the `adapter` field
existed carries no adapter and is not read -- fails closed, the same as one
with no recorded tool at all. An unset claude-code or codex declaration
still counts as on, matching what the watcher reads from by default (its
conventional per-user store); Gemini, Cline and OpenCode count as off until
declared. A Never contribution override makes every folder Never, so it
reads nothing and matches nothing. A folder that is not armed and not
shared may be read, including the unidentified-folder bucket -- it resolves
to `NotifyOnly`, not Never, so it is read and counted in `read.folders`,
though it is never asked about languages (it has no folder root to look
at). When a folder has no display path recorded in the policy (never
armed, or known only through the cache), the language probe falls back to
the project key itself, which is case-folded on macOS and Windows; on a
case-sensitive volume that can miss a marker file under its real-cased
name, so a language goes unreported rather than over-reported. No session
body is read. When a mission asks about `languages`, the roots of the
readable folders are checked for marker files, by existence only. `read`
counts the distinct tools and folders matching read, and nothing else.

**What matching excludes (M2).** A session counts toward a mission only
when it is contributed through one of the three consent paths ("The three
paths," above). A **kept** session (the contributor explicitly holding it on
this Mac) and a **withdrawn** one (an upload later taken back) are excluded
from the candidates matching reads -- neither was contributed in a way that
should count, the first because it was declined, the second because it was
reversed. An **already-uploaded** session that is neither kept nor
withdrawn, and one still being written and **not yet quiescent**, are
excluded from neither set and stay candidates: quiescence is a fact about
when a session is ready to read, not about whether it was ever offered, and
an ordinary accepted upload did nothing M2 forbids.

**What it never does (M1, M2).** It sends nothing: no activity profile,
match, folder or tool list leaves the Mac. It writes no audit row and logs
one label, `mission-matches-answered`. It changes no policy, queue or daemon
state: it never arms a folder, approves a session or widens a scope, and
emits no event -- the queue and the local history cache are read here (to
exclude kept and withdrawn sessions), never written. A match is a
suggestion for this screen; a session counts toward a mission only when it
is contributed through one of the existing paths.

**The disclosure (M4).** `tc_missions_disclosure_copy_json` returns the
words shown the first time Missions is opened and in Settings
(`consent_copy::missions_disclosure_copy`): `{title, matching, nothing_sent,
credit}`. Approved 2026-10-06.

### View menu: Group by and Sort by (K15)

The native app's View menu (#1146, #1152) offers Group by and Sort by for
the Traces tab, alongside the already-built "Show ignored folders". Neither
needs a new field: every option is answerable from `list_pending`'s entry
fields (`entry_value`), `list_projects`, and `list_history`, below. One of
those fields is on an open, not-yet-merged PR -- `list_projects[].tools`
(K11, #1192); the table names it, so nothing here is duplicated when it
lands. K9's `title` (#1191) and K10's `would_send_bytes` and
`uploaded_bytes` (#1196) have landed and are listed as existing.

**Group by**

| Option | What it groups on | Source |
|---|---|---|
| Tool (default) | an entry's `source` / `declared_source` | `list_pending`, existing |
| Tool, for a folder with nothing waiting | the per-tool session counts the project has seen | `list_projects[].tools[].source` (K11, #1192) -- until it lands, such a folder cannot be placed and is listed on its own, exactly as #1183 (R6) documents |
| Folder / project | `project_id` (grouping key) and `project_label` (display) | both existing, on every entry and every `list_projects` row |
| None (flat list) | no grouping field; every entry already carries enough to render a row on its own | — |

**Sort by**

| Option | What it sorts on | Source |
|---|---|---|
| Name | a session's `title`, falling back to its formatted `started_at` when `title` is `null` (the existing fallback, unchanged) | `title` on every `list_pending` entry (K9, existing; `null` on an entry queued before K9, see ["`title`"](#title)); `started_at` existing |
| Name, for a folder | `project_label` | existing, on every `list_projects` row (already disambiguated when two projects share a basename) |
| Date | a session's `started_at`, or `discovered_at` when `started_at` is unknown | both existing, on every `list_pending` entry |
| Date, for a folder | `last_session_at`, falling back to `added_at` for a configured folder the watcher has not yet observed a session in | both existing, on every `list_projects` row |
| Size | a session's `would_send_bytes` (what an upload would actually send), falling back to `size_bytes` (the raw file) when no envelope is pinned | `would_send_bytes` on `list_pending` (K10, #1196); `size_bytes` existing |
| Size, in history | `uploaded_bytes` | `list_history` rows (K10, #1196) |
| Status | `state` and `reason_label` for a queued session, plus `eligibility` where it is present; `status` for a history row | all existing; `eligibility` is ABSENT whenever the signup flag is off (see ["Contribution eligibility"](#contribution-eligibility)), so a client must not require it -- sort an entry without it as `eligible`, since an invited contributor's whole queue is contributable |

No row carries a folder-level size or count total. A client that wants one
-- to sort folders themselves by total size, say -- already holds every
session in that folder from `list_pending` and can sum `size_bytes` or
`would_send_bytes` itself; `list_projects` already carries the plain count
(`session_count`, `pending_count`). Nothing here asks the daemon to
pre-aggregate what the client already has the parts for.

**A stable sort key needs no new field.** Every row already carries a
daemon-issued, stable, unique id to break ties deterministically: `entry_id`
on a queue entry, `submission_id` on a history row, `project_id` on a
project. The underlying lists are themselves returned in a deterministic
order, so two reads with nothing changed produce the same order even before
a client's own Group by / Sort by choice is applied: `list_pending` is the
queue's insertion order, and `list_projects` is every configured project
(sorted by its policy key) followed by every discovered-but-unconfigured
one (sorted by its key) -- so a configured `/z/repo` precedes a discovered
`/a/repo`. Neither order is alphabetical by anything a contributor sees; a
client that offers a sort sorts for itself.

A project's label on an entry needs no new field either: `project_label` is
a plain (non-optional) `String` on every `QueueEntry`, never absent.

### The `outcome` verdict

`approve` accepts an optional `outcome` parameter for the contributor's verdict
on the session. Accepted values are `worked`, `partly`, and `failed`; absence
maps to `TaskSuccess::Unknown` and allows approval to proceed. Any other value
returns `bad_params` (`outcome-invalid`) without approving an entry, while a
valid value sent with `all` or `project_id` applies across that approval.

### The `correction` parameter

`approve` also accepts an optional `correction`: what the contributor wrote
about what the run got wrong, stored **as they typed it**.

- **String, optional.** Blank or whitespace-only is treated as absent, and
  the approval proceeds exactly as an uncorrected one. Anything but a string
  is refused with `bad_params` (`correction-invalid`).
- **Only with `partly` or `failed`.** A correction sent with `worked`, or
  with no `outcome` at all, is refused with `correction-needs-outcome` and
  approves nothing. You cannot correct a run you have just called
  successful; the gate is a guard as much as it is semantics.
- **Only with `entry_id`.** Sent with `all` or `project_id` it is refused
  with `correction-needs-entry-id`: a correction is written about one
  session, and applying one string to a batch would attach an explanation to
  sessions it does not describe.
- **Capped** at 2000 characters (`envelope::MAX_CORRECTION_CHARS`); longer is
  refused with `correction-too-long`. Clients should cap at the keyboard so
  the refusal lands where the person can shorten what they wrote.

A correction is folded into the envelope **before redaction runs**, not
stamped on afterwards like `outcome`. That is what puts it in front of
credential detection and what makes `consent.correction_included` describe
the envelope. Consequently an entry carrying a correction is **rebuilt and
re-pinned** by this call even if it was already previewed: the pinned
artifact was built before the contributor had written anything.

A correction is **not scrubbed** -- neither the deterministic semantic passes
nor the prose-PII filter runs over it, because redaction would destroy the
explanation it exists to give. Credential detection still runs and still
blocks: a High or Critical match refuses that entry with the skip
`reason_label` `correction-credential-detected`, approves nothing for it, and
leaves it `Pending`. Clients must surface that refusal distinctly from other
skips -- the contributor needs to remove the credential *and* rotate it --
and must never echo the correction text or the detected value.

### What `approve` reports

`approve` returns:

```json
{
  "approved": 1,
  "hold_secs": 10,
  "hold_until": "2026-08-08T12:00:10Z",
  "flagged": 1,
  "redactions": { "private_email": 2, "secret:openai_api_key": 1 },
  "skipped": [ { "entry_id": "…", "reason_label": "not-enrolled" } ],
  "excluded_ineligible": 4,
  "excluded_held": 1
}
```

**A group-level `approve` means "all eligible", never "all".** Both group
selectors -- `project_id` and `all` -- act only on entries whose
`eligibility` is `eligible`. The alternative either fails partway or succeeds
at sending exactly what the surface just finished saying could not be sent,
and no shell can close it: a per-project approve has no row to check, so the
per-row gate cannot reach it.

`excluded_ineligible` is how many pending entries the selector left out for
this reason, so a client can say what became of the rest rather than infer it
from a count smaller than the one it drew a button for. Excluded entries do
**not** appear in `skipped`: they were never selected, and `skipped` is the
account of what this call was asked to act on.

`excluded_ineligible` is **absent** when no filter ran -- an invited
contributor, or a single `entry_id`. Absent, never zero: zero would read as
"nothing was left out", which is a claim about a filter that did not run.

**A group `approve` also leaves out sessions held for a person's review**, and
`excluded_held` is how many. An entry is held when it was revoked for a
reason no unattended approval can satisfy:

- `token-distribution-review-required`, which clears through a witness review
  of that one session;
- `witness-risk-review-required`: a session approved on the contributor's
  behalf that the witness certified with a residual-risk verdict other than
  `low`. The witness has already seen it, and nothing went to the commons.
  Its certified review is kept pinned to the entry, so `preview` shows the
  witness's bytes without running the witness again, and a person's
  `approve` uploads exactly those bytes. Like any pinned `Pending` preview,
  the pin is released after the preview age limit, and opening it then runs
  the witness again.
- `second-look-review-required`: a session approved on the contributor's
  behalf that the Automatic Scrub check held because the envelope built for
  it is worth a second look -- nothing matched, it looks unsure, or it was
  trimmed to fit. See `scrub_check` under `set_settings`.

A group control is by definition not a review of one
session, so held entries are left out for every contributor, invited or not,
and are not in `skipped`. The watcher does not re-approve them under an
`auto_upload` opt-in either; before it stopped, it re-approved one on the
next poll, the uploader revoked it again, and the session never uploaded.

`excluded_held` is present on every group call, because this filter always
runs on one, and absent on a single `entry_id`, where it does not. It is kept
apart from `excluded_ineligible` because the two say different things: an
ineligible session cannot be sent, and a held one can once someone looks.

Render it through `tc_contribution_withheld_line`, which turns the count into
a sentence and answers the **empty string** for zero. A button reading
"Submit all (2)" above a folder showing five rows, with nothing explaining the
gap, is the same small dishonesty the rest of this surface removes. The
sentence says how many and **not why**: the reason a particular session cannot
be sent is that row's own sentence, one level in, and a summary here would
stand for up to fourteen different reasons and say nothing true about any of
them.

**One reply, one deadline.** A group `approve` does not fan out. It takes one
approval instant for the whole call, so every entry it approves shares one
hold and the single `hold_until` it reports is true of all of them. A client
must not fan a group submit out into per-entry calls and keep the first
reply's hold: an undo bar has to outlast every entry it offers to undo, and
the first reply's deadline retires Undo while something it covers is still
recoverable. Ask for the group and use the group's deadline.

An entry with no recorded eligibility renders `unknown`. `unknown` IS offered
the control and IS included by a group selector: it is a session whose
attestation could not be decided at discovery -- every Responses-API call,
which is every Codex session, because a hosted and a brokered call come back
under the same identifier shape -- and the receipt fetch at submission is the
only thing that can decide it. Nothing is claimed; the state sentence still
says it has not been worked out. The two `ineligible_*` states are the ones
that offer nothing and are excluded.

**A single `entry_id` is never filtered.** Naming one entry is an explicit act
about a session the contributor is looking at, the shell's per-row gate
already covers it, and the server decides admission either way. The daemon
reports eligibility on a row; it enforces it only where a shell cannot.

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
  | `envelope-too-large` | The built envelope exceeds the size the daemon will store, even though the build succeeded. The entry is moved to `refused` with this same string as its `reason_label`, so it stops being offered | **Never** succeeds for this entry -- do not offer retry |
  | `not-pinned` | The pin did not stick even though the build succeeded, and the entry is still `pending` (a concurrent write, or the entry vanished from the queue mid-call) | Transient -- retry is expected to work |
  | `correction-credential-detected` | The `correction` sent with this call contains something credential-shaped. Nothing was built, pinned or sent; the entry stays `pending` | Retry succeeds once the credential is out of the text. Surface this distinctly and tell the contributor to rotate it -- never echo the correction or the match |
  | `not-pending` | The entry was not `pending` when this call reached it -- already `approved` by an earlier `approve`, or dismissed, expired or superseded meanwhile | Refresh queue state rather than retry blindly; a retry alone can never succeed |

  Only `envelope-too-large` changes the entry's state; every other label
  above leaves the entry exactly where it stood. A refusal that no retry
  can ever get past is not a state a queue should keep calling `pending`,
  and leaving it there re-offered the same unapprovable card on every poll.
  The refusal binds to the entry, not to the session's path: a dismissal
  (`dismissed-by-contributor`) silences a conversation permanently, but
  this is a verdict on one envelope built under one set of consent scopes,
  so a session that grows or is rebuilt under narrower scopes is offered
  again. Note that `list_pending` returns `pending` entries only, so a
  client's own toast is the only place the contributor sees this: render
  the skip rather than dropping it.

  Nothing here is free text, a path, or trace content. **`approved` plus
  the length of `skipped` always equals the number of entries `approve` was
  asked to act on** -- for `entry_id` that is 1, for `all`/`project_id` it
  is however many matched at selection time. An id absent from both would
  be a silent loss of an approval decision; the response is built so that
  cannot happen. An `entry_id` naming no entry at all is refused before any
  of this runs -- see below.
- **An unrecognized `entry_id`** is refused the same way `preview` refuses
  the same input: `bad_params` / `unknown-entry-id`, before a `skipped` entry
  can be produced; this applies only to the single-`entry_id` form because
  `all` and `project_id` select existing ids from the queue.
- **An unrecognized `project_id`** is refused the same way
  `set_project_mode` refuses it: `bad_params` / `project-id-unrecognized`.
  A handle the daemon cannot resolve is a client bug, and answering it
  `approved: 0` would be indistinguishable from "that project had nothing
  pending" -- a client holding a typo'd or stale id would render "Sent 0
  sessions" and never learn otherwise. A project the daemon **does** know
  with nothing pending -- everything already approved, dismissed or swept
  -- is not an error: it succeeds with `approved: 0` and an empty
  `skipped`. Recognition is by the project appearing in the daemon's policy
  or on any queue entry in any state, so a project whose entries were all
  just approved stays recognized.
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
  approval outright: the scopes, the envelope-determining fingerprint, the
  hold itself, and the pin binding the approval to the exact bytes it
  covered. A subsequent `approve` therefore rebuilds the envelope from the
  session as it then stands -- reporting its own `redactions` and `flagged`
  counts, like any other approval of an entry with no preview behind it --
  and starts a fresh window. An undone approval leaves nothing on disk: the
  stored envelope is swept once the pin naming it is gone. `cancel:
  {"project_id": ...}` applies exactly this to every `approved` entry in
  that project in one call, so an Undo offered on a batch `approve` is one
  call with the same `project_id` argument rather than a shell deriving
  "the ids I saw pending minus the ones reported skipped" and racing the
  queue to cancel them individually.
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

A `bulk-approved` entry carries the number of entries selected in `detail`,
and, for the `project_id` form, that project's derived label in
`project_label` -- `null` for `approve: {"all": true}`, which names no one
project. The label is derived from the key the daemon holds, never from the
caller's string.

A `bulk-canceled` entry is `cancel: {"project_id": ...}`'s counterpart: same
shape, `detail` carries the number of `approved` entries in that project
that were undone, and `project_label` is that project's derived label. It is
written before anything is canceled, under the same rollback-cannot-record
guarantee as `bulk-approved`, and only when the selector matched at least
one entry -- a `cancel` against a project with nothing `approved` appends
nothing. The single-`entry_id` form of `cancel` stays unaudited, the same
as the single-`entry_id` form of `approve`.

A `past-sessions-included` entry is `include_past_sessions`'s: `detail`
carries the number of sessions chosen and `project_label` the folder's
derived label. It is written once the call validates and before anything is
revived, queued or approved; no session id, path or title is in it.

An `auto-upload-voided` entry records a standing `auto_upload` grant that the
daemon voided because the terms in force widened past what it was armed under
-- a new recipient (destination, identity, witness, classifier host or model,
receipt endpoint) or more leaving the machine (scopes gaining an entry, a
filter added or removed, attested bodies turning on, a witness measurement
admitted). `project_label` names the project and `detail` is a comma-separated
list of fixed reason labels: `destination-changed`, `identity-changed`,
`scopes-widened`, `privacy-filter-changed`, `receipt-endpoint-changed`,
`witness-changed`, `witness-measurement-admitted`, `attested-bodies-on`. The
project is `notify_only` from that pass on; arming it again records the new
terms. Narrowing voids nothing. Unlike arming, the void is written **after**
the mode change and a failed write does not undo it, because voiding is the
safe direction.

Four entries belong to the automatic grant (`grant_automatic`, below):
`automatic-granted` and `automatic-grant-withdrawn` record it being given and
withdrawn; `armed-by-default` records a project it armed, with that project's
`project_label`; `automatic-grant-voided` records the grant itself voided by
widened terms, with the same `detail` labels as `auto-upload-voided`.

A `contribution-override-voided` entry records an "Auto contribute"
contribution override cleared by widened terms (#1208), with no
`project_label` and the same `detail` labels as `auto-upload-voided` (or
`terms-unrecorded`); see "The contribution override".

`limit` is optional, defaults to 50, and is capped at 1000 even if a larger
value is requested. Entries are returned newest first, matching
`list_history`'s convention. `action` and `detail` are always fixed labels --
never free text, a path, or a token. See "Authorization" above for what this
log is (and is not) for.

A `grant-voids-acknowledged` entry records `acknowledge_grant_voids`;
`detail` is how many notices it cleared.

### Void notices

R6 of the connect-and-forget design makes a void notice a ship condition:
when widened terms void a grant, every shell tells the contributor, not only
the audit. `status.grant_voids` lists every void no shell has acknowledged
yet, oldest first:

```json
{
  "grant_voids": [
    { "id": 4, "kind": "project", "voided_at": "2026-09-26T12:00:00Z",
      "project_id": "3f1c...", "project_label": "api",
      "reasons": ["witness-measurement-admitted"] },
    { "id": 5, "kind": "automatic_grant", "voided_at": "2026-09-26T12:00:00Z",
      "project_id": null, "project_label": null,
      "reasons": ["witness-measurement-admitted"] }
  ]
}
```

- `kind` is `project` for an `auto-upload-voided` void, `automatic_grant`
  for an `automatic-grant-voided` one, and `contribution_override` for a
  `contribution-override-voided` one (#1208; see "The contribution
  override"). `project_id` and `project_label` are the ones `list_projects`
  gives that project, and are `null` for the grant and the override. No
  path crosses. The override's notice has no re-arm button: it is turned
  back on from the pill's own confirmation, and setting any override
  answers the notice. Giving the Flow 1 grant again answers only the
  grant's notice.
- `reasons` are the same fixed labels as the audit's `detail`.
- The list is always present, and `[]` when there is nothing to show, so a
  shell can tell that from a daemon too old to report voids.
- A shell does not write the notice. It passes one element to
  `consent_copy::void_notice_for_wire` (across the C ABI,
  `tc_grant_void_notice`), which returns the title, body, one sentence per
  reason, the line after them, and the buttons' labels. An element it
  cannot place (an unknown `kind`, or a project without a label) still gets
  a notice, saying automatic contributing stopped without saying for what,
  so no shell writes a fallback of its own.
- The list is in the policy file, so a void during a pass no shell saw is
  shown at the next launch. A sweep that voids publishes `status_changed`.

`acknowledge_grant_voids` takes `ids`, the ids of the notices actually
shown, and removes them. It is required, and there is no "all": a void
raised between a shell drawing and the contributor pressing the button would
otherwise be cleared unseen. An id that is not outstanding is ignored, since
another shell may have acknowledged it first. It is audited
(`grant-voids-acknowledged`) before anything is cleared, and refused with
`audit-write-failed` when that entry cannot be written; it publishes
`status_changed` when it clears anything. Acknowledging only records that
the notice was seen: it re-arms nothing.

A project's notice carries a second button, `rearm_action` ("Turn back on"),
present only when the element has a `project_id`. It is `set_project_mode`
with that `project_id` and `auto_upload`, nothing more: the same refusals
(`arming-terms-unavailable`, the unknown bucket, an unrecognised id), the
same `armed-auto-upload` audit row, and arming under the terms now in
force. Pressing it is the fresh consent R6 asks for, which the sentence
beside it says. A refusal changes nothing and leaves the notice; a shell
shows `rearm_failed`. The automatic grant's notice has no such button: it
says what happened to projects instead. A shell that can give the grant
(today only Tauri, through its Flow 1 screens) asks for the notice with
`consent_copy::void_notice_for_wire_with_regrant`, which adds `regrant` and
`regrant_action` on the grant's notice only. That button opens the grant
screens again -- scope, path, both disclosures, the grant -- and gives
nothing by itself; `grant_automatic` given there clears the notice. Shells
that cannot give the grant keep `void_notice_for_wire`, which promises no
re-grant.

A notice also goes when the contributor acts on what it is about: setting
that project's mode (`set_project_mode`, including re-arming it) clears the
project's notice, and `grant_automatic` clears the grant's. Ids are never
reused. Logout clears the policy file and every notice with it.

### `grant_automatic`, `withdraw_automatic_grant`, `automatic_grant`

The Flow 1 grant (the connect-and-forget design, K3 and K4): contribute
automatically from projects discovered from now on.

```json
{ "granted": true, "granted_at": "2026-09-25T12:00:00Z", "on_disk_recorded": false }
```

`grant_automatic` takes two required params:

- `confirmed`: JSON `true`, sent only from the grant screen's button, after
  the contributor has gone through the scope, path and disclosure screens.
- `witness_signing_address`: the signing address of the witness the
  contributor was shown on the disclosure screen, or `null` when that screen
  showed none.

It returns the grant as `automatic_grant` reports it. Nothing is granted when
it is refused:

- `arming-terms-unavailable` (`ERR_UNAVAILABLE`): there is no config to record
  terms from.
- `automatic-grant-confirmation-required` (`bad_params`): `confirmed` is
  absent, `false`, or not a boolean (`"true"` and `1` are refused). The same
  label `flow1::grant_precondition` gives a shell, so the daemon holds the
  confirmation for every IPC caller rather than trusting a shell to have
  asked.
- `automatic-grant-scopes-not-chosen` (`bad_params`): the config's
  `consent_scopes_chosen` is false, so nobody chose the saved scopes (R7), or
  the saved `consent_scopes` list is empty, so there is nothing to grant
  under. A saved scope list is not a choice: every enrollment saves at least
  the floor scope, which `validate_scopes` adds, and an invite enrollment
  saves it with nobody having picked it. Only `set_consent_scopes` records a
  choice.
- `automatic-grant-witness-required` (`bad_params`): `witness_signing_address`
  is absent, or neither a string nor `null`.
- `automatic-grant-witness-changed` (`bad_params`): the witness configured now
  is not the one named, including one configured where none was shown or none
  where one was. A witness written between the disclosure screen and the grant
  would otherwise be bound into the grant's terms unseen, and the void rule
  would not catch it, since it compares against the terms captured at the
  grant. The terms are captured from the same config read that was checked.
- `audit-write-failed`: its `automatic-granted` entry cannot be written.

A second call replaces the first, and records what is on disk again.

**Every shell must call `set_consent_scopes` from its scope picker before
`grant_automatic`**, even when the contributor keeps the scopes enrollment
saved. As of this writing only the Tauri app gives the grant; the macOS,
Windows and GTK shells do not call `grant_automatic`.

**It arms nothing already on disk**, recorded per source. Each source's first
successful discovery in a full watcher pass under the grant records every
session in it and the project each belongs to, and arms nothing from them. So
a harness connected after the grant, one pointed at another root, and one whose
discovery failed on an earlier pass are all recorded before they can arm
anything; until a source is recorded, none of its sessions arms a project.
`on_disk_recorded` is true once any source has been recorded. A grant given
while a pass is listing the disk is recorded by a later pass.

After a source is recorded, a session from it in a project that no recorded
source had on disk, with no policy entry of its own, and not the
unknown-project bucket, arms its project: an explicit `auto_upload` entry, the
terms it is granted under, and an `armed-by-default` audit row written first.
A project with any session on disk at the grant keeps asking, for its new
sessions too. And a session that was on disk at the grant is **never approved
unattended in a project the grant armed**, whichever project it reads as now;
it waits for the contributor. That holds after the grant is withdrawn or
voided, and stops holding for a project once the contributor sets its mode
themselves.

That is what makes a re-grant after logout safe. Logout wipes the policy,
including a project set to `ignore`; a re-grant records the disk again, so that
project asks rather than being armed as new.

Widened terms void the grant itself as well as the projects it armed, with an
`automatic-grant-voided` entry. `withdraw_automatic_grant` returns
`{"withdrawn": bool}` and leaves the projects the grant armed as they are;
each is withdrawn with `set_project_mode`. `automatic_grant` returns
`{"granted": false}` when none is in force. None of the three returns a path
or a count of them.

### `queue_outcome_counts`

```json
{ "reasons": { "dismissed-by-contributor": 2, "expired-without-decision": 1 } }
```

A count, by `reason_label`, across resolved entries currently on the queue.
Pending, approved, and uploading entries are excluded: sessions held for
review are still waiting, not terminal outcomes. This method is **not**
named `eligibility_reasons`, and does not
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

### `probe_routing`

```json
{ "outcome": "reachable" }
{ "outcome": "token_unreadable", "token_path": "/Users/x/.ironwire/control.token" }
{ "outcome": "unreachable", "port": 8463 }
```

Asks the IronWire proxy a contributor is about to declare whether it is
actually there, so that declaring can answer instead of failing silently.

The routing *reader* deliberately treats absence and failure as the same
state: a proxy that vanished must never cost anyone a trace, so nothing on
the submission path reports an error. That is right for reading and wrong
for declaring. Without this method a contributor can name a wrong port or an
unreachable token, see no error and no indicator, and have every trace carry
no routing data. This method runs only when a human asks; it touches no
daemon state and cannot affect a submission.

`token_dir` is resolved exactly as the reader resolves it -- the declared
directory, then the `token_path` in IronWire's discovery pointer, then
`IRONWIRE_HOME`, then `~/.ironwire` -- through the same function, so the
path reported is the file that would actually be read.

The three outcomes:

- **`reachable`** -- the proxy answered and the token was accepted.
- **`token_unreadable`** -- carries `token_path`, the absolute path that was
  tried. Covers no readable `control.token` there and a proxy that answered
  and refused the one that was. A GUI-launched daemon never sees
  `IRONWIRE_HOME`, so it reads `~/.ironwire/control.token` whatever the
  contributor set in a shell; that produces a missing file on one machine
  and a stale token on another, and naming the directory fixes both.
  `token_path` is absent only when nothing resolves at all -- no declared
  directory, no `IRONWIRE_HOME`, no discoverable home.
- **`unreachable`** -- carries `port`, the port that was tried. Also the
  answer when something answered but did not serve the ledger (a 404, a
  500): the usual cause is a number naming some other local service, so the
  port is still the actionable fact.

**No outcome ever carries the token.** It is a credential for an API that
can rewrite the contributor's agent configuration, and this answer crosses a
socket to a shell. The token *directory* is not the token, and the path is
the whole point.

Refusals are `bad_params` / `port-invalid` (missing, non-integer, `0`, or
above 65535) and `bad_params` / `token-dir-invalid` (present but not a
non-empty string -- refused rather than treated as absent, which would
answer about a path the caller did not ask about).

### `discover_routing`

```json
{ "found": true, "port": 8463, "token_path": "/Users/x/.ironwire/control.token" }
{ "found": false }
```

Reads `~/.ironwire/endpoint.json`, the pointer IronWire writes when its
daemon binds and removes on a clean stop, and reports what it says. The
point is that a contributor should not be asked for two things the machine
already knows.

Takes no parameters and performs no network I/O -- it reads one small local
file. `probe_routing` is the other half: this method reports a proxy that
published itself, the probe checks a proxy the contributor named.

`found` is a boolean and not a vocabulary of outcome names on purpose. There
is one distinction to draw -- a pointer was read, or it was not -- and every
reason it was not (IronWire not installed, not running, a version that does
not publish a pointer, a file this reader will not act on) is the same fact
to the caller and the same next step for the contributor: type the port. A
set of outcome strings would invite matching on one, and a name that is a
prefix of another is how a shell comes to treat `unreachable` as
`reachable`.

`token_path` is absent when the pointer named none, or named a relative one.
Its presence is informational -- something to show beside the port. A caller
does **not** need to pass it back: the daemon resolves the same pointer
itself whenever it opens a token, so a declaration that names no `token_dir`
finds it anyway.

**The answer never carries the token**, for the same reason `probe_routing`
does not: it is a credential for an API that can rewrite the contributor's
agent configuration, and this answer crosses a socket to a shell. The
pointer itself never contains a token either -- it names a path.

The pointer's **port is advisory**. It is offered here, to a flow where a
human confirms it, and it is deliberately not used to override a port
already declared in settings: IronWire removes the pointer on a clean stop,
so a crash leaves one behind, and a stale port silently overriding a correct
declaration would make every trace carry either nothing or another local
service's data while the settings file and the probe both still agreed on
the declared port. A missing or stale pointer costs at most one refused
connection, which is what a daemon that never ran would cost.

### The harness list

Three methods, and there is deliberately **no fourth** that plans and commits
in one call.

These edit configuration files this application does not own -- Claude Code's
`settings.json`, Codex's `config.toml`. That is acceptable only because of
three rules, which come from `ironwire_agents` and which this surface carries
end to end rather than reimplementing:

- **Never rewrite a file we cannot parse.** A contributor's own syntax error
  must not come back looking like ours.
- **Fill an empty slot; leave a full one alone.** A value already in the key is
  another destination or a deliberate choice. It is **reported, never
  overwritten**.
- **Remove only what we put there.** A disconnect leaves neighbouring keys
  alone and needs no saved original.

#### `harness_list`

```json
{
  "catalog_present": false,
  "destination_port": 8463,
  "harnesses": [
    {
      "id": "claude",
      "name": "Claude Code",
      "installed": true,
      "connected": true,
      "config_path": "/Users/x/.claude/settings.json",
      "connect_command": "ironwire connect claude",
      "family": "anthropic",
      "answers_at": "Anthropic",
      "state": "answering",
      "last_call_at": "2026-09-07T18:04:11+00:00",
      "can_connect": false,
      "can_disconnect": true
    }
  ],
  "activity": {
    "readable": true,
    "window_hours": 24,
    "last_call_at": "2026-09-07T18:04:11+00:00",
    "families": [{ "family": "anthropic", "last_call_at": "...", "calls": 12 }]
  },
  "spend": {
    "known": true,
    "micros": 1230000
  }
}
```

`installed` and `connected` are read from the filesystem on **every** call,
never cached: a tool that rewrites its own config underneath us is a real
thing, and a list read fresh corrects itself where a cached one lies.

`config_path` is present always, not on demand. A tool nobody expected to be
configured is a question about *which file*, every time. It is `null` only
where this build could not work out where the tool keeps its config.

`connect_command` is a command string, not prose. Show it verbatim, as the
fallback for a contributor who would rather do it themselves; do not parse it
and do not paraphrase it.

`catalog_present` is a fact about **this build**, not about the machine. When
it is `false` the list is the two tools IronWire ships knowing about, and a
surface must say that the list is what this machine knows about rather than
implying only two coding tools exist. A harness that is not installed is
listed with `installed: false`, not omitted: hiding it makes the absence of a
tool indistinguishable from the app never having heard of it.

`answers_at` names the vendor this row's tool answers at by default --
`"Anthropic"` for `claude`, `"OpenAI"` for `codex` -- drawn from the SAME
fixed table `tc_discover_sources` reads its own `answers_at` field from, so
the two surfaces cannot name a vendor differently for the same tool. `null`
for a catalog-described tool, which has no `family` and therefore no claimed
default either. Like `family`, this is a fixed fact about the tool's own
released default, never something this daemon checked against the copy
actually installed.

#### Gemini CLI and Antigravity are not rows here (K13)

`harness_list` is deliberately narrower than "every coding tool this daemon
knows about". `found` above is `ironwire_agents::tools::all(catalog)`, and
with no catalog loaded (`catalog_present: false`, the only state this build
ships in today) that is **exactly** `claude` and `codex` -- the two tools
IronWire's embedded proxy can redirect, because `Facade::url` only speaks
the Anthropic and OpenAI wire shapes. Gemini CLI speaks neither, so it has
never had a row here, and Antigravity -- which is not even a watched
source; see below -- cannot either. Adding either would mean writing our
loopback URL into a config key that does not actually redirect that tool's
calls, which is the exact hazard `owned_agents`'s doc comment refuses to
guess at. Nothing in K13 changes this list; "Gemini CLI keeps its row"
refers to its row in `tc_discover_sources` (a watched session store, not a
redirectable harness), not to this one.

Antigravity's own `answers_at` is `"Google"`, from the same
`source::source_default_family` table as `gemini-cli`'s -- see `tools` under
`list_projects` above, and "Antigravity" under `tc_discover_sources`-shaped
discovery, below.

#### Antigravity has no `tc_discover_sources` row either, and that is also deliberate

Antigravity is not a `TraceSource`: there is no conventional, watchable
per-user store for it to probe blind, the way `claude-code`, `codex`,
`gemini-cli` and `cline` are probed. It ships as a one-shot
`import-antigravity` command that reads the running IDE's local API and
stages what it finds as `trajectory` files -- see
`crates/trace-commons-contributor/src/antigravity/mod.rs`'s own doc comment.
Nothing it stages shares a folder with Gemini CLI's `~/.gemini/tmp` adapter,
and the two were never actually merged in the shipped code: an earlier,
abandoned design (`docs/superpowers/specs/2026-08-29-antigravity-source-design.md`)
would have read Antigravity's own SQLite files from under `~/.gemini/`, but
it was superseded before it shipped by the API-import design actually in
place (`docs/superpowers/specs/2026-08-31-antigravity-import-command-design.md`).

What an imported conversation DOES carry, and has carried since that design
landed, is its own declared source: `meta.source: "antigravity"` on the
staged file, read back as `SessionRef::declared_source` and shown as
`"Antigravity"` by every shell's agent label -- never as `"trajectory"`,
and never merged with a real Gemini CLI session. K11's `tools` field and
K13's `source_default_family` entry read that same string, so a project
mixing an Antigravity import with a real Gemini CLI session reports two
distinct tool rows, not one.

`spend` is what the calls answered **on this computer** have cost since the
most recent local midnight, in millionths of a dollar. It comes from the
proxy's own status object, which is why it is metered-only: the ledger rows
this daemon already reads price *every* exchange, including work a monthly
plan has already paid for, and summing those would produce a figure for a day
on which nothing was billed.

`known` is `false` -- and `micros` `null` -- for **every way of not knowing**:
no declared, readable proxy; a refresh that has not landed; an answer that did
not come back or would not parse; and a figure the proxy itself declined to
measure.

**`known: false` IS NOT ZERO, and a surface must not render it as one.** A day
on which nothing was spent is `known: true, micros: 0`, and it says so in
words. A day nobody could measure draws no line at all. The amount is
assembled by `tc_harness_spend_line`, whose absence convention is an
out-of-range integer -- pass a negative value for `known: false` -- so no
shell formats money, rounds it, or decides for itself what an unmeasured
figure looks like. The sentence saying what the figure leaves out is the copy
payload's `harnesses_spend_scope`, drawn beside the amount and only when the
amount is drawn.

A daemon older than the release that reports this sends no `spend` block at
all, which is one of the ways of not knowing -- not a zero.

`destination_port` is the port a connect would write into a config file -- the
hosted listener's port when it is up, else the port declared for a proxy the
contributor runs themselves. It is `null` when nothing on this machine is
answering model calls, and in that state `harness_plan` refuses a connect with
`harness-no-destination` rather than writing a guess.

#### `state`, and why there are five values and not two

`connected` proves a config file has the right value in it. It does **not**
prove a single call was ever answered. `state` is the three-valued answer the
design asks for, plus two honest unknowns:

| `state` | meaning |
|---|---|
| `not_connected` | its config does not send calls here |
| `connected_no_calls` | config is right; no call has arrived yet |
| `answering` | a call arrived, and only this tool could have made it. `last_call_at` says when |
| `activity_shared` | a call arrived in this tool's protocol family, and another connected tool speaks that family too, so it cannot be attributed to either |
| `unknown` | connected, and nothing here can say whether a call arrived -- no readable ledger, or a tool whose family this build does not know |

**Attribution is approximate and must be described as such.** The proxy's
ledger records a *facade* -- `anthropic`, `openai` -- not a tool id. Claude
Code speaks Anthropic and Codex speaks OpenAI, so today a call separates them;
two connected tools of the same family do not separate at all. `family` on a
row is what makes attribution possible, and it is `null` for a
catalog-described tool, whose facade nothing here knows.

`last_call_at` on a **row** is present only for `answering` -- the case where
the tool is nameable. The `activity` block is the same evidence with **no tool
named**: `last_call_at` there answers "did a call arrive at all", and
`families[]` answers it per protocol family. A surface that needs to say a
call arrived without naming a tool reads that block. `readable: false` means
no ledger answered, which is not evidence of no calls and must never be
rendered as "nothing has arrived yet".

Making this exact needs an upstream change -- a per-tool path chosen at connect
time, which `plan_connect(id, port, catalog)` does not expose.

#### `harness_plan`

```json
{
  "id": "claude",
  "action": "connect",
  "outcome": "changes",
  "plan_id": "6f1c...",
  "path": "/Users/x/.claude/settings.json",
  "changes": ["set env.ANTHROPIC_BASE_URL"],
  "occupied": [{ "slot": "env.ANTHROPIC_BASE_URL", "current": "https://their-proxy.example" }]
}
```

Writes nothing. `outcome` is one of:

| `outcome` | meaning |
|---|---|
| `changes` | there is an edit to make; `changes[]` describes it and `plan_id` is minted |
| `noop` | nothing to do. Say so rather than showing an empty confirmation |
| `unparseable` | the file could not be parsed, so it was **refused rather than rewritten**. Distinct from `noop`: nothing was decided and the file needs a human. Name the file -- `path` carries it |
| `not_installed` | the tool is not on this machine, so there is nothing to connect |
| `entry_unusable` | the catalog entry describing this tool did not survive validation |
| `no_config_path` | this build could not work out where the tool keeps its config |

`plan_id` is minted for `changes` and for nothing else, so "there is a plan id"
and "the outcome is committable" are one question answered once.

**`occupied` is not an outcome.** A plan can carry changes *and* occupied slots
at once -- the empty slots are filled and the full one is reported in the same
pass -- so it rides alongside whatever `outcome` says, and must be rendered
whatever `outcome` says. Each entry names the slot and shows the value already
in it. Show the value, say it was left alone, and **do not offer to take it
over**.

Refusals that are call errors rather than facts about the machine come back as
errors, not outcomes: `harness-unknown` for an id nothing knows,
`harness-no-destination` for a connect while nothing is answering model calls,
`action-invalid`, `id-required`.

Parse-failure *detail* never crosses the socket. The underlying error quotes
the offending line of the contributor's own file; only the label and the path
are reported.

#### `harness_commit`

```json
{
  "id": "claude",
  "action": "connect",
  "committed": true,
  "path": "/Users/x/.claude/settings.json",
  "backup_path": "/Users/x/.claude/settings.json.ironwire-backup"
}
```

Takes `plan_id` and **nothing else**. There is no argument here that could name
a different tool, a different action or a different file from the one
previewed, which is what makes the preview binding rather than advisory. The
plan itself never crosses the socket -- the daemon holds it between the two
calls -- so a shell cannot reconstruct one it was not given.

A plan id is **single use** and expires after ten minutes. Refusals:

| label | meaning |
|---|---|
| `harness-plan-unknown` | expired, already committed, or never minted. Plan again and show the contributor the result |
| `harness-config-changed` | the file moved between the plan and the commit -- the tool rewrote its own config while the preview was on screen. The write is refused rather than reverting whatever the tool just wrote |
| `harness-commit-failed` | the write itself failed |
| `plan-id-invalid` | not a plan id at all |

`backup_path` is where the file as it was before this edit was kept, when one
was written. Only the **first** backup of a file is kept -- it is the only copy
holding the file as it was before any of our edits -- so this is `null` on a
second edit to the same file, and on a file that did not exist before.

### The map and the Inference tab

K8 of #1118, the client half. `tool_destinations` feeds the map and
`inference_calls` feeds the Inference tab's routing table. Both are
synchronous and local. They read state this daemon already holds -- the
harness list, the private-inference state, the route `route_disclosure`
reports, the source settings, the project policy and the local routing
ledger -- and make no network call and read no body.

Every value on the wire is a fixed label, a count, a time or a ledger row id,
with one exception: `model` is free text (see below). No prompt, response,
body reference, body digest, provider exchange identifier, session id, token,
endpoint or URL crosses the socket, and nothing here is logged. The same holds
for the `inference_call_added` event (K14), which carries a subset of an
`inference_calls` row.

#### `tool_destinations`

```json
{
  "private_ai": "running",
  "sessions_route": "witness",
  "window_hours": 24,
  "unattributed_calls": 2,
  "folders": { "armed": 1, "ask_first": 3, "ignored": 0 },
  "tools": [
    {
      "tool": "claude-code",
      "name": "Claude Code",
      "sessions": { "watch": "watched", "to": ["commons", "witness"] },
      "model_calls": { "to": "near_ai", "basis": "observed" },
      "counts": { "sessions": 4, "inference_calls": 37 }
    }
  ]
}
```

No parameters. One row per session source this build has (`claude-code`,
`codex`, `gemini-cli`, `cline`, `opencode`).

`sessions.watch` is `watched` when an adapter is actually built for the tool
(an undeclared Claude Code or Codex still falls back to its conventional
folder, and reads `watched`), `off` when the contributor said they do not use
it, and `not_watched` otherwise. `sessions.to` is empty unless the tool is
watched, and then it is what `sessions_route` sends: `["commons", "witness"]`
on `witness`, `["commons"]` on `local`, and nothing on `witness_refusing`,
`not_enrolled` or `settings_unreadable`. `sessions_route` is the same value
`route_disclosure.route` carries, so the map cannot draw a witness disclosure
does not name; the witness a connected inference offer installs
(`inference_connection_install`) reaches the map through it. `folders` counts
the per-repository rules (`auto_upload`, `notify_only`, `ignore`). They are
per repository across tools, so they say *when* a session goes, not *where*.

`counts` (K14) is the map's per-tool node data over `window_hours`, the same
24-hour window `inference_calls` uses. Local reads only; nothing new is
fetched.

- `sessions` counts the sessions this daemon saw in the window, once per
  session hash: queue entries whose session was last seen written (or, for an
  entry from before that was recorded, discovered) inside it, and history
  records submitted inside it. An entry still in the queue and its history
  record are one session. `null` when the history cache cannot be read; a
  count from the queue alone would undercount.

  A session counts under the tool it reads as: its declared source when
  discovery knew one, else the adapter that read it -- the rule
  `list_projects.tools` and the CLI's session table use. So an imported
  Antigravity conversation (read by the `trajectory` adapter, declaring
  `antigravity`) counts as `antigravity`, never as `trajectory`, and never
  under both. A history record names only the adapter, so it takes its label
  from the queue entry with the same session hash; one with no queue twin (a
  CLI `submit`, say) falls back to the adapter. There is no `antigravity`
  row on this map -- it has no adapter, watch declaration or connection of
  its own -- so those sessions are not counted on any tool row today.
- `inference_calls` counts exactly the rows `inference_calls` lists (rows
  with a ledger id) by the `tool` it names for them, so the map and the
  Inference tab cannot disagree. `null` when no ledger has answered
  (`inference_calls.readable: false`), which is not zero.
- `unattributed_calls` is the window's listed calls whose `tool` is
  `unknown`. They are counted there and never folded into a tool; `null`
  under the same condition as `inference_calls`.

A shell that pulses on `inference_call_added` should re-read these counts
rather than add to them: the event is capped per poll and is a pulse, not a
ledger.

`model_calls.to` and `basis`:

| `to` | `basis` | when |
|---|---|---|
| `near_ai` | `observed` | connected to this daemon's port, `private_ai` is `running`, and every labelled call in the window in the tool's protocol family was routed |
| `near_ai` | `configured` | as above, but the window has no labelled call in the family to confirm it. **A shell must not draw this as confirmed** |
| vendor | `answered_elsewhere` | connected, and the proxy answers with the tool's own credentials (`running_answered_elsewhere`) |
| vendor | `tool_default` | the tool's config names no local proxy |
| `unknown` | `unknown` | connected while any labelled call in the family went `outside`, or while the proxy is in any other state; or wired to a local proxy that is not this daemon's |

`running` says NEAR AI reports a credential, not that a given family is
answered there -- the proxy can hold other backends -- which is why it is
confirmed against the ledger's own labels rather than trusted on its own.

The vendor is a fixed label: `anthropic` (Claude Code), `openai` (Codex),
`google` (Gemini CLI), and `unknown` for Cline and OpenCode, which talk to
many providers. It is the tool's **default**, not a reading of its config: a
tool whose own config names another gateway is still drawn at its default.
Which tool has a fixed default is now read from K2's `source_default_family`
and confirmed against `source::vendor_label` (#1130), rather than kept in a
separate local table. The word on the wire stays the family's own lowercase
spelling above, not `vendor_label`'s returned word (`Anthropic`, `OpenAI`,
`Google`), which is capitalized for the "answers at" display and is a
different surface from this one. `private_ai` is the
`private_inference_state.state` label `status` carries.

#### `inference_calls`

```json
{
  "readable": true,
  "window_hours": 24,
  "calls": [
    {
      "id": 412,
      "at": "2026-09-28T18:04:11+00:00",
      "tool": "codex",
      "family": "openai",
      "model": "Qwen/Qwen3.6-27B-FP8",
      "route": "routed",
      "cost": { "known": true, "priced_micros": 12300 },
      "proof": "gateway_only"
    }
  ],
  "next_cursor": "1790000000000:411"
}
```

Newest first. `limit` is 1 to 200 (default 50); out of range or not an
integer is `bad_params` / `limit-invalid`, refused rather than capped so a
caller paging by the size it asked for never skips rows. `cursor` is the
previous page's `next_cursor`, passed back unread; anything else is
`bad_params` / `cursor-invalid`. `next_cursor` is `null` on the last page.
The order is `(started_at, id)`, which is total; rows from a proxy too old to
expose an `id` are not listed.

`readable: false` means no ledger has answered, which is not evidence of no
calls and must not be drawn as an empty table. The window is the ledger's
own 24 hours.

- `tool` (K14). IronWire's log rows carry no harness, only the facade that
  took the call and the endpoint inside it the client called. Each tool this
  daemon connects is pointed at its own facade and speaks its own API there,
  so the endpoint names the tool, connected now or not:

  | facade | endpoint | `tool` |
  |---|---|---|
  | `anthropic` | `/v1/messages`, `/v1/messages/count_tokens` | `claude-code` |
  | `openai` | `/v1/responses` (Codex is connected with `wire_api = "responses"`) | `codex` |
  | any | any other endpoint, e.g. `/v1/chat/completions` | `unknown` |

  An endpoint no connection writes is `unknown` even while a tool of that
  family is connected: it is not that tool's own wire. A named endpoint is
  also `unknown` while a *different* connected tool speaks the same family.
  On a proxy too old to record the endpoint, the fallback is the harness
  that set the proxy: the one tool connected **now** that speaks the call's
  family (the rule `harness_list` applies to `answering`), else `unknown`;
  that fallback is approximate, since a row from before a connection changed
  can carry the wrong name. A tool pointed at the proxy by hand, speaking the
  same API at the same facade as a connected one, cannot be told apart in the
  row and reads as that tool. The endpoint is read for this and never passed
  through. `family` is `anthropic`, `openai` or `unknown`.

  **Limit: endpoint attribution does not check who set up the proxy.** Any
  `/anthropic` + `/v1/messages` call reads `claude-code`, and any `/openai`
  + `/v1/responses` call reads `codex`, whether or not this daemon connected
  that tool, and whether or not this daemon owns the proxy at all. An agent
  routed through IronWire's own catalog, or by another program that manages
  the same proxy, is counted as Claude Code or Codex, and the "different
  connected tool" check above cannot see it, because only tools this daemon
  connects are speakers. The label means "called the endpoint Claude Code's
  connection writes", not "came from Claude Code".

  `/v1/messages/count_tokens` calls are included: they are listed, counted
  in `tool_destinations.counts.inference_calls` and announced by
  `inference_call_added` like any other call.

  The "different connected tool speaks the same family" case cannot arise
  today, since each family has exactly one tool this daemon connects; it is
  there so a second one in a family turns that family's endpoints
  `unknown` rather than crediting the first.
- `model` is **free text, not a fixed label**: the served model as recorded,
  else the requested one, passed through when it is at most 128 characters of
  `[A-Za-z0-9._:/@-]` and names no URL; else `unknown`. It comes from a proxy
  the contributor can patch.
- `cost.priced_micros` is the ledger's price in millionths of a dollar. It is
  **priced, not billed** -- work a plan already paid for is priced at the
  meter -- and must not be drawn as money spent. `known: false` is not zero.

`proof` is IronWire's own label for the row (`RoutedExchange.proof`),
passed through unchanged -- the stored verdict of IronWire's receipt check,
never re-derived here -- or `unrecorded`:

| `proof` | Means |
|---|---|
| `verified` | proof: a model receipt over this call's digests, signed by a key a verified, measurement-pinned TDX quote binds |
| `gateway_only` | a valid receipt, but the gateway's; it names the relay, not the model. Not proof |
| `unattested` | a valid model receipt whose key nothing tied to a verified quote. Not proof |
| `pending` | a NEAR AI call not checked yet, or receipt checks are off |
| `unavailable` | no receipt to be had or checked |
| `failed` | a receipt that does not check out -- possibly tampering |
| `outside` | not a NEAR AI backend: an outside call, never checked |
| `unrecorded` | IronWire recorded no label: an older proxy, a row that predates proof tracking, or a label this build does not know. Not `outside`, not `pending` |

Only `verified` is proof. The labels are deliberately not collapsed: a shell
that wants fewer buckets maps them itself, and must keep `failed` distinct.

`route` is taken from that label and nothing else: `outside` for `outside`,
`routed` (through NEAR AI) for any other label, and `unknown` for
`unrecorded`. It is never taken from the backend id, which is whatever the
contributor named it.

#### What the ledger does not record (Z4)

The table the design draws is work x model x calls x cost with a proof per
routed answer. These are not in the local ledger, and nothing here invents
them:

- **kind of work** -- no field records it; there is no classifier;
- **tool** -- the ledger records a facade and an endpoint, not a harness,
  so a call is attributed by the endpoint its tool's connection writes (see
  `inference_calls.tool`), and is `unknown` otherwise;
- **billed cost** -- the ledger prices every call; only the day's metered
  total (`harness_list.spend`) is billed spend;
- **outside calls that bypass the proxy** -- a tool not connected to Private
  AI never reaches the ledger, so it is not in this list at all;
  interception (Stop / Send anyway) needs the proxy in the path;
- **the smart router's choice** -- nothing records which model a router
  picked or why.

### `set_settings`

Takes a JSON object whose top-level keys must come from
`quiescence_secs`, `digest_interval_secs`, `digest_schedule`,
`approval_hold_secs`,
`local_notifications`, `claude_root`, `codex_root`, `claude_source`,
`codex_source`, `gemini_source`, `cline_source`, `opencode_source`,
`trajectory_source`, `ironwire`,
`ironwire_attested_bodies`, `private_inference`,
`private_inference_offer_seen`, `scrub_check`, `max_uploads_per_day`,
`max_bytes_per_day` --
a key this method does
not recognize is
refused outright (`bad_params` / `settings-unknown-field`), not silently
ignored, so a caller that mistypes a key gets a definite signal rather than
a daemon that quietly kept the old value. A recognized key holding the
wrong JSON type returns `bad_params` / `settings-invalid-value`; an empty
object returns `bad_params` / `no-known-setting-supplied`.

`digest_schedule` picks when the digest fires: `{"mode":"interval"}` (the
default, unchanged from before this key existed -- one digest every
`digest_interval_secs`) or `{"mode":"evening","hour":H}` for one digest a
day at local hour `H` (0-23; omitted, it defaults to 18, i.e. 18:00 local).
"Local" is computed in the contributor's own timezone (`chrono::Local` on
the daemon's host), handling DST transitions and a missed window: an
evening the daemon was asleep or unreachable through still fires once, at
the next opportunity (a laptop that sleeps from 17:30 and wakes at 07:00
fires at 07:00), never once per missed day. The first digest after
switching to `evening` waits for that day's hour. A last-digest time in the
future (the clock was moved backwards) is treated as stale on either
schedule rather than holding digests off. `hour` outside `0..=23` is
`bad_params` / `settings-invalid-value` from `set_settings`; read from a
settings file, it falls back to `interval` with a label-only log line. Either mode keeps "only with
something to say" unchanged (see the `digest_due` event, under "Events"
below): a digest with nothing pending and nothing contributed since the
last one never fires, on either schedule. This is open decision #5 on issue
#1118; both schedules ship rather than picking one, so the choice is a
setting rather than a release cliff.

`opencode_source` takes `{"mode":"watch","path":"/chosen/export-directory"}`,
`{"mode":"off"}`, or `null`; absent, null, and Off construct no adapter, including
when older settings load. Watch reads direct `.json` children exported with
`opencode export SESSION_ID` without scanning OpenCode's database; the
[qualification report](superpowers/reports/2026-09-07-opencode-export-qualification.md)
records version support and routing limits, and the declaration grants neither
body capture nor remote submission.

`trajectory_source` takes the same three values for a folder of exported
trajectory files (a Letta Trajectory export, for example). Absent, null and
Off add no folder; Watch reads its direct `.json`/`.jsonl` children with the
strict trajectory reader alongside the staging folder. It is not part of the
claude/codex start gate, `get_settings` reports only
`trajectory_source_mode`, never the path, and a session found there is never
armed for automatic upload: it always waits for a person.

See "The `trajectory_source` declaration", below.

`approval_hold_secs` takes a non-negative integer no greater than 300
(five minutes): how long an approval is
held before the uploader will touch it, which sets the duration of the
contributor's undo; the default is 10, while `0` disables the hold and makes `approve` report
`hold_until: null` so a client knows to offer no undo. It is read at each
upload pass, so a change applies to approvals already sitting in the queue,
and a shortened hold can release an entry a client is still counting down
for -- treat the `hold_until` from `approve` as authoritative for the
approval it accompanied, and do not change this setting mid-countdown. A
value outside `0..=300` is `bad_params` / `settings-invalid-value`.

`quiescence_secs` and `digest_interval_secs` are likewise bounded, not open
`u64` fields: `quiescence_secs` to `0..=14_400` (zero is meaningful -- a
session counts as finished the instant it stops growing -- and 14,400
seconds is four hours, past which "done" never realistically arrives for a
session still being written) and `digest_interval_secs` to `3_600..=86_400`
(one hour to one day). Each used to accept any value, with only the Tauri
shell's own command layer clamping before the call ever reached
`set_settings`; a raw caller had no such floor. Both are now validated in
`apply_settings_object` itself, so every caller gets the same bound with the
same label-only `settings-invalid-value` the other numeric fields already
use.

Every numeric field's exact bounds -- `quiescence_secs`, `approval_hold_secs`,
`digest_interval_secs`, `max_uploads_per_day` and `max_bytes_per_day`, all in
the unit `set_settings` itself stores and validates (seconds or bytes, never
a shell's own minute/hour/megabyte control granularity) -- are available
without guessing or hard-coding a second copy: the C ABI's
`tc_settings_ranges_json` returns them as one JSON object, so a shell can
draw its controls' bounds from the same numbers this method enforces.

`ironwire`'s `{"mode":"watch","port":P,"token_dir":D}` now validates `P` and
`D` with the same floor `probe_routing` holds: `port` must be non-zero (`0` is
the ask-the-kernel sentinel, never a port a proxy actually listens on,
`bad_params` / `settings-invalid-value` -- more precisely
`routing-port-invalid`), and `token_dir`, when present and non-empty, must be
an absolute path (`routing-token-dir-must-be-absolute`); an empty `token_dir`
is treated as absent. The two differ in two labelled ways a shell should
know: `probe_routing` refuses port `0` as `port-invalid` (not
`routing-port-invalid`), and refuses an empty `token_dir` as
`token-dir-invalid` rather than treating it as absent, because a probe that
fell through to the environment would answer about a path the caller did
not ask about. A relative `token_dir` is refused by both, with
`routing-token-dir-must-be-absolute`. Before this, only the Tauri shell's own command layer
refused these two shapes -- a raw `set_settings` caller (another shell, or a
future one) had no such floor and could persist a declaration nothing would
ever actually route through.

`claude_root` and `codex_root` each take a JSON string (a filesystem path)
or `null` (clear the override, falling back to the conventional per-user
location); setting either here only takes effect from the daemon's *next*
supervisor tick onward -- the tick already scheduled or in flight when this
call returns has already read the old value.

A caller that needs the
watcher to scan a non-default location from the very first tick -- most
importantly a native host embedding the daemon via the C ABI, or a test
harness that must never scan the real `~/.claude`/`~/.codex` -- cannot get
that through `set_settings`, since it only works on an already-running
daemon and the first tick fires immediately on start. That is what the C
ABI's `tc_daemon_start_with_settings` is for: it applies the same object
this method validates, but before starting the daemon, so the first tick
already observes the override; see `include/trace_commons.h`.

`ironwire_attested_bodies` takes a boolean, and it is a **second, separate
answer from `ironwire`** rather than a detail of it; `ironwire` declares a
local inference proxy whose ledger the daemon may read: metadata about model
calls -- how many, what they cost, which backend served them. This key says
that the final call's verbatim request and response bodies -- the
contributor's own prompt, in the clear -- may additionally be carried to a
configured redaction witness, which verifies an inference receipt against
them inside its enclave, strips them, and certifies what is left; declaring
the proxy therefore never switches this on: cost attribution is not consent
to send a prompt, and a client that sets `ironwire` alone gets routing
telemetry and no bodies.

Default `false`, and a settings file written before this key existed loads
with it off; turning it on moves the approval fingerprint, so approvals
already given are re-asked rather than honoured under the new terms. The
directory read is not configurable and is not this key: it is
`bodies/` inside whichever proxy home the control token resolved in, so the
body store and the ledger always belong to the same proxy; with no proxy
declared, with the proxy declared off, or with no witness configured,
setting this changes nothing that leaves the machine.

`private_inference` takes a boolean and asks this daemon to host the proxy.
An explicit `ironwire` declaration still controls metadata precedence; without
one, accepted hosting opt-in supplies metadata from the owned proxy, as described
under `routing`. Body collection stays separate. With hosting enabled, the daemon starts IronWire in its own process,
using `$IRONWIRE_HOME`, else `~/.ironwire`, as its home, so the `ironwire`
CLI and the existing ledger reader see the same ledger, token and pointer
they always did.

Default `false`, and a settings file written before this key existed loads
with it off. Nothing turns it on by discovery: a pointer on disk means
someone else is running IronWire, which is a different fact from the
contributor asking this daemon to. Turning it on repoints no agent -- which
tools route through IronWire stays a per-tool declaration -- and turning it
off stops only the instance this daemon started.

**What turning it on exposes.** Until this key, a running daemon bound one
thing: its own IPC socket or named pipe, guarded by the 0700 state directory
on unix and by the pipe DACL on Windows. With the switch on it also binds
IronWire's loopback listener, and that listener is not the same shape of
thing. IronWire's *control* API is token-gated -- a caller needs the token
file out of the proxy home -- but the *inference* path is not: any process
running as any user who can reach `127.0.0.1` on that port can send
inference requests through it, and they are billed and authenticated with
whatever upstream credentials the proxy home is configured with. On a
single-user desktop that is the contributor's own software; on a shared or
multi-user machine it is anyone with a local shell. Enabling it also starts
IronWire's own background work, including catalogue discovery, which makes
network calls the daemon did not previously make.

This is why the default is off and why nothing turns it on by discovery. A
client offering the switch should say plainly what it turns on, rather than
presenting it as a performance or privacy toggle alone.

What actually happened is reported separately, as `private_inference_state`
on `get_settings`, `set_settings` and `status`. It is an object,
`{"state": <label>, "port": <number or null>}`, with these labels:

| `state` | meaning | `port` |
|---|---|---|
| `off` | the switch is off, this daemon owns no proxy and nothing is bound | `null` |
| `stopping` | retained shutdown is awaiting outstanding startup, calls or cleanup; the requested switch may already be off | last owned port, or `null` before binding |
| `running` | this daemon's proxy is serving and has a backend registered | the bound port |
| `running_no_backends` | this daemon's proxy is serving but no backend is configured, so nothing will route through it | the bound port |
| `running_elsewhere` | a loopback discovery pointer responds, or the exclusive home lock is held; nothing was bound or stopped, and readiness is not established | the published port, or requested port when the lock owner has not published one |
| `port_in_use` | something that is not this daemon's proxy holds the port | `null` |
| `start_failed` | the proxy refused to start for any other reason | `null` |
| `crashed` | startup, serving, or cleanup failed; release of owned resources may be unconfirmed | `null` |

Turning hosting off persists the request before stopping the owned proxy. The
reply may report `stopping` while requests drain; it does not mean the listener,
pointer, or home lock has been released. A new on request waits for that cleanup
before another listener can start. Daemon termination prevents any later restart
and waits up to five seconds for cleanup, including waiting for a startup already
in progress. On expiry the retained cleanup continues while the runtime lives;
process exit or runtime destruction can interrupt remaining streams. Closing or
dropping the embedded daemon requests cleanup without blocking synchronously.
Drop-based cleanup requires unwinding; a `panic=abort` build terminates the
process instead and cannot finish in-flight requests.

Existing-instance discovery has two fixed bounds:

- Read at most 64 KiB from an opened regular pointer file.
- Probe only the fixed IPv4 loopback health path.

Accepted hosts are `127.0.0.1` and `localhost`; an IPv6-only pointer is not
discovered through an unrelated IPv4 port, while URL-shape restrictions add
defense in depth because the request URL is constructed from the validated port.

Platform checks differ:

- On supported Unix targets, the opened object must match the checked
  device/inode and effective-user owner, remain unwritable by others, and use
  no-follow/nonblocking flags so a replacement symlink or FIFO cannot bypass
  the check or block the open.
- Windows checks regular-file/reparse shape on the opened handle. This check
  supplies neither a DACL nor a Unix ownership guarantee.

Advisory discovery fails closed on Unix targets other than shipped macOS and
Linux x86_64/aarch64. The probe sends no token,
ignores environment proxies, and never follows redirects. A successful health
response is advisory: it conservatively avoids takeover but does not authenticate
the endpoint. The exclusive home lock remains authoritative when starting.

`running_no_backends` is deliberately not `running`: the proxy answers its
health endpoint and no inference can pass through it, so a client that
rendered it as running would show a green light over a proxy that cannot
work. `crashed` is sticky until the switch is turned off and on again,
rather than restarting every poll tick, so a proxy that cannot stay up is
visible instead of flickering. If startup or shutdown lost its completion
signal, Off does not claim cleanup succeeded. A newly accepted off/on cycle may
retry through IronWire's exclusive home lock; only a successful start clears
that uncertainty. If the prior owner still holds the lock, the failure remains
visible and no second proxy is started.

The shells distinguish an absent or empty status from a nonempty state label
this build does not recognize. The former says the daemon does not report
model-call status (including older daemons); the latter says the state is
unavailable. Neither proves `off`. `stopping` uses the Held tone until the
retained-shutdown producer confirms cleanup; a port alone is metadata, not
proof that calls can be answered.

The companion C ABI copy payload (`tc_private_inference_copy`, not a daemon
settings key) supplies the fixed string fields for the destination, the proxy
state, the harness rows and the credential row:

- `destination`, `subtitle`;
- `offer_title`, `offer_what`, `offer_exposure`, `offer_no_repoint`,
  `offer_accept`, `offer_decline`, `offer_asked_once`;
- `settings_title`, `settings_toggle`, `settings_applies_at_once`;
- `state_off`, `state_unreported`, `state_unknown`, `state_stopping`,
  `state_running`, `state_running_no_backends`,
  `state_running_answered_elsewhere`, `state_running_destination_unknown`,
  `state_running_elsewhere`,
  `state_port_in_use`, `state_start_failed`, `state_crashed`;
- `quit_also_stops`, `write_unconfirmed`;
- `settings_moved`, `tray_turn_off`, `tray_open_to_turn_on`;
- `harnesses_title`, `harnesses_what`, `harnesses_spend_scope`,
  `harnesses_none_found`;
- `harness_not_connected`, `harness_connected_nothing_seen`,
  `harness_answering`;
- `harness_connect`, `harness_disconnect`;
- `harness_preview_title`, `harness_preview_confirm`,
  `harness_preview_cancel`;
- `harness_slot_taken`, `harness_needs_restart`,
  `harness_unreadable_config`;
- `harness_not_installed`;
- `harness_plan_nothing_to_change`, `harness_plan_entry_unusable`,
  `harness_plan_no_config_path`;
- `credential_title`, `credential_what`, `credential_cost`;
- `credential_obtain`, `credential_cancel`, `credential_forget`,
  `credential_forget_explains`, `credential_migrate`,
  `credential_migrate_explains`;
- `credential_absent`, `credential_obtaining`, `credential_failed`,
  `credential_cancelled`, `credential_present`, `credential_unknown`,
  `credential_unreported`.

**Those are the fields for those rows, and not the whole payload.** The
payload also carries a sentence for every state of every per-state family --
the queue entry's `eligibility_*` and `attestation_*` sentences among them --
and each family is written down in the section for the field it describes
rather than repeated here. It grows a set whenever a family is added, and
the list above is not extended when that happens. So do not read that list as
an inventory of the payload: a key's absence from it is not evidence the key
does not exist. The place that settles the question is `private_inference_copy`
in `trace-commons-contributor`, whose `every_sentence_arrives_finished` pins
the payload's field count and is what a shell's decoder is checked against.
This document deliberately does not repeat that number: a count copied into
prose goes stale in silence, and the test does not.

### The `trajectory_source` declaration

```json
{ "method": "set_settings", "params": { "trajectory_source": { "mode": "watch", "path": "/chosen/exports" } } }
```

"Add your tool" (#1030's Tools step) lets a person point at a folder of
exported traces -- a Letta Trajectory export, for example -- that no parsed
tool's layout matches. `trajectory_source` declares it:

- `{"mode":"watch","path":"..."}` reads the folder's direct `.json` and
  `.jsonl` children with the strict trajectory reader, the one that reads the
  staging folder, as one `trajectory` source covering both. No new parser is
  involved.
- `{"mode":"off"}`, `null`, or the key absent add no folder. Settings
  written before this key existed load as absent, so an upgrade reads no new
  folder.
- Any other value -- a bare path string, an unknown mode -- is `bad_params`
  / `settings-invalid-value`, as for the other `*_source` keys.

`get_settings` reports `trajectory_source_mode` (`unset`, `off` or `watch`)
and never the path; no response, log line or audit row carries it.

The declaration is not part of the claude/codex start gate: a daemon with
only a trajectory folder declared has not declared its roots.

**A session found there is never armed for automatic upload.** It always
waits for a person, in a folder on Automatic too, under the same rule as a
staged trajectory: a trajectory there is an export somebody added, not a
watched agent store, and sending it on first sight could send something the
person does not remember adding. The rule is on the adapter, so it holds for
a file dropped in by hand as well as for one that names its own source. It
can be approved one by one, or chosen in `include_past_sessions`.

### Contribution eligibility

Queue entries carry two additive fields. They appear on **every** surface that
hands a client an entry object -- `list_pending`, the `snapshot` event, and
the `entry` object both `preview` responses carry (the async-dispatch one and
the built card) -- and on no other, because `entry_value` is the only thing
that serialises a queue entry and those are all of its callers.

`approve` and
the `preview_ready` event do not carry an entry object at all; they carry an
`entry_id`. A client MUST NOT reconstruct eligibility from an entry it cached
from some other path.

| Field | Meaning |
|---|---|
| `eligibility` | `eligible` \| `ineligible_permanent` \| `ineligible_configuration` \| `unknown` |
| `eligibility_reason` | a stable label naming why, or absent |

**`eligibility` is ABSENT -- not `unknown`, not null -- whenever the
contributor is admitted on an invite** rather than on evidence, i.e. whenever
`admission_evidence_required` is false or could not be read. An invited
contributor has no eligibility question: everything in their queue is
contributable, which is why this whole surface stayed invisible for so long.
A field answering a question they do not have would put three shells to work
rendering a caveat on work that carries none. A client MUST test for the key,
never read a missing key as a state.

Show every session. Offer only the eligible ones. Hiding a contributor's own
work is its own dishonesty and makes the app look as though it had not
noticed files the contributor knows it can see. An ineligible row is present,
not offered, and carries its reason.

| `eligibility` | Means | Sentence | Control a shell may offer |
|---|---|---|---|
| `eligible` | the cheap checks pass and the marked call is here | `eligibility_eligible` | contribute |
| `ineligible_permanent` | nothing the contributor changes will alter this | `eligibility_ineligible_permanent` | **none** |
| `ineligible_configuration` | this session stays ineligible; a setting decides future ones | `eligibility_ineligible_configuration` | **none** |
| `unknown` | not evaluated | `eligibility_unknown` | **none** |
| anything else | the state could not be read | `eligibility_unknown` | **none** |

The sentence, the tone and the control come from the shared Rust tables --
`tc_contribution_eligibility_line`, `tc_contribution_eligibility_tone` and
`tc_contribution_eligibility_control` -- never from shell-authored branching
on a variant name, and a shell must not recover any of the three by reading
another. `TC_CONTRIBUTION_CONTROL_NONE` (50) and
`TC_CONTRIBUTION_CONTROL_CONTRIBUTE` (51) are the control values.

**An unrecognised state must not borrow an ineligibility sentence.** A state
this build cannot read is not evidence about a contributor's session, and
saying it is would stop them offering work that is fine. Every shell carries
a test for this.

A shell does not have to apply this rule to a group control itself. Ask for
the project and the daemon returns the honest subset; `list_projects` carries
`contributable_count` so the button can name it before the press. See "What
`approve` reports".

`eligible` is a well-founded expectation and not a guarantee. The cheap
checks -- the ones a list may run -- are answered here; the expensive ones,
which need every captured body read back and hashed, still run at submit. The
server decides admission either way, and if this answer and the server's
decision disagree, **the server is right**. When a submission is refused for
an admission reason the daemon writes the refusal back into the row, so a row
that was `eligible` stops saying so. A list that changes while it is being
read is the cost of that, and it is also just what happened.

`eligibility_reason` is absent on an `eligible` entry -- there is nothing to
explain -- and is one of these otherwise. Render each through
`tc_contribution_eligibility_reason_line`, which answers the **empty string**
for a label this build does not know; render nothing for an empty string
rather than guessing.

The two unknown-handling rules are deliberately different, and the difference
is not an inconsistency. An unrecognised **state** degrades to a sentence
because the row still has to say something -- it is on screen, a contributor
is reading it, and silence there would leave them to infer a state from an
empty space. An unrecognised **reason** has nothing honest to say: the state
sentence beside it has already carried the fact, and a sentence invented for a
label this build does not know would add a detail nobody established. Silence
beats a guess exactly where something true has already been said.

| `eligibility_reason` | Sentence | Usually seen with |
|---|---|---|
| `no_inference_call` | `eligibility_reason_no_call` | `ineligible_permanent` |
| `capture_off` | `eligibility_reason_capture_off` | `ineligible_configuration` |
| `digest_absent` | `eligibility_reason_digest_absent` | `ineligible_permanent` |
| `upstream_id_absent` | `eligibility_reason_upstream_id_absent` | `ineligible_permanent` |
| `digest_mismatch` | `eligibility_reason_digest_mismatch` | `ineligible_permanent` |
| `reference_malformed` | `eligibility_reason_reference_malformed` | `ineligible_permanent` |
| `bodies_unreadable` | `eligibility_reason_bodies_unreadable` | `ineligible_permanent` |
| `body_not_utf8` | `eligibility_reason_body_not_utf8` | `ineligible_permanent` |
| `body_too_large` | `eligibility_reason_body_too_large` | `ineligible_permanent` |
| `evidence_capture_off` | `eligibility_reason_evidence_capture_off` | `ineligible_configuration` |
| `marker_absent` | `eligibility_reason_marker_absent` | `ineligible_permanent` |
| `request_malformed` | `eligibility_reason_request_malformed` | `ineligible_permanent` |
| `receipt_unavailable` | `eligibility_reason_receipt_unavailable` | `unknown` |
| `receipt_not_issued` | `eligibility_reason_receipt_not_issued` | `ineligible_permanent` |

Only `ineligible_configuration` names a setting, and it is the only state
painted `TC_PRIVATE_INFERENCE_TONE_ATTENTION`. `no_inference_call` has no
actionable answer for the session the row is about, and advice about the
*next* session is guidance rather than status -- rows stay about their own
session. A permanent ineligibility is `_NEUTRAL` and never `_REFUSED`:
nothing was refused and nothing went wrong.

Both fields are additive; the schema version stays
`trace_commons.daemon.v1_1`, and a client that ignores them behaves exactly
as before.

The seventeen sentences named above are fields of the `private_inference_copy`
payload (`tc_private_inference_copy`), which now carries **97** fields.

### The attestation mark

`eligibility` above answers *may I send this?* -- a permission question, and
one an invited contributor does not have. Underneath it is a second question
every contributor has, all of the time:

> Does this session carry a checkable copy of its last model call?

That is a fact about the trace rather than about anyone's permission, and it
is about to stop being provenance metadata: the credit scoring function is
expected to weight attestations. A contributor who cannot see which of their
sessions carry one cannot act on it, and an app that showed the same
classification to one class of user under a different name would be
withholding a fact that affects what their work is worth.

| Field | Values |
|---|---|
| `attestation` | `attested` \| `unattested_permanent` \| `unattested_configuration` \| `unknown` |
| `attestation_reason` | a stable label naming why not, or absent |

**`attestation` is ALWAYS PRESENT, on every entry, for every contributor.**
The exact opposite of the `eligibility` rule above, and deliberately: there
is no contributor for whom the question does not apply, so the only thing an
absent field could mean is a build too old to answer it. An entry written
before the field existed reports `unknown`, never an unattested mark.

| `attestation` | Means | Sentence | Tone |
|---|---|---|---|
| `attested` | the session carries a checkable copy of its last model call | `attestation_attested` | `_CLEAR` |
| `unattested_permanent` | it does not, and nothing the contributor changes will add one | `attestation_unattested_permanent` | `_NEUTRAL` |
| `unattested_configuration` | it does not; a setting decides whether future ones will | `attestation_unattested_configuration` | `_ATTENTION` |
| `unknown` | not worked out | `attestation_unknown` | `_NEUTRAL` |
| anything else | the mark could not be read | `attestation_unknown` | `_NEUTRAL` |

Sentence and tone come from `tc_contribution_attestation_line` and
`tc_contribution_attestation_tone` -- never from shell-authored branching on
the label.

**There is no control accessor, and that is the contract.** The mark
describes the trace; it offers nothing to press. A shell that drew a button
from it would be inventing an action out of a description. Whether a row may
be sent is `eligibility`'s question and `tc_contribution_eligibility_control`
answers it.

**The positive case is the one that matters here**, unlike `eligibility`,
whose surface only ever spoke up to explain a refusal. A session that IS
attested says so. A surface that only names what is missing teaches a
contributor that the mark means bad news.

**An unrecognised mark must not read as "no copy".** A mark this build
cannot read is not evidence about a contributor's session, and telling them
an attested session carries nothing is the one wrong answer this table can
give.

**A permanently unattested session is `_NEUTRAL` and never `_REFUSED`.**
Nothing was refused and nothing went wrong: most of a contributor's history
was recorded before anything was keeping copies.

`attestation_reason` takes the **same fourteen labels** as
`eligibility_reason` -- a reason names a fact about the session, not an answer
to either question -- and is rendered through
`tc_contribution_attestation_reason_line`, which answers the **empty string**
for an unfamiliar or absent label. It is absent on an `attested` entry.

**`unknown` may arrive with or without a reason, and a shell MUST branch on
the key rather than on the mark.** Two different things produce that mark. A
row the daemon never evaluated has no reason -- nobody worked anything out.
A row whose send was turned away because the receipt could not be fetched is
`unknown` **with** `receipt_unavailable`, and that reason is the only thing
telling the contributor it may work later. Every other mark is
shape-predictable: `attested` never carries a reason, and the two unattested
marks always do.

**The sentences are NOT the same, and a shell must not substitute one call
for the other.** Five of the eligibility sentences say the session cannot be
sent, which is true for an evidence-admitted contributor and false for an
invited one, whose session sends perfectly well and merely arrives without a
copy of its call.

| `attestation_reason` | Sentence | Usually seen with |
|---|---|---|
| `no_inference_call` | `attestation_reason_no_call` | `unattested_permanent` |
| `capture_off` | `attestation_reason_capture_off` | `unattested_configuration` |
| `digest_absent` | `attestation_reason_digest_absent` | `unattested_permanent` |
| `upstream_id_absent` | `attestation_reason_upstream_id_absent` | `unattested_permanent` |
| `digest_mismatch` | `attestation_reason_digest_mismatch` | `unattested_permanent` |
| `reference_malformed` | `attestation_reason_reference_malformed` | `unattested_permanent` |
| `bodies_unreadable` | `attestation_reason_bodies_unreadable` | `unattested_permanent` |
| `body_not_utf8` | `attestation_reason_body_not_utf8` | `unattested_permanent` |
| `body_too_large` | `attestation_reason_body_too_large` | `unattested_permanent` |
| `evidence_capture_off` | `attestation_reason_evidence_capture_off` | `unattested_configuration` |
| `marker_absent` | `attestation_reason_marker_absent` | `unattested_permanent` |
| `request_malformed` | `attestation_reason_request_malformed` | `unattested_permanent` |
| `receipt_unavailable` | `attestation_reason_receipt_unavailable` | `unknown` |
| `receipt_not_issued` | `attestation_reason_receipt_not_issued` | `unattested_permanent` |

A submission turned away for an admission reason writes back here exactly as
it does to `eligibility`: a row still claiming its session carries proof,
after a send of that session was refused for want of that proof, is the
defect this surface removes reproduced one layer up.

Both fields are additive; the schema version stays
`trace_commons.daemon.v1_1`, and a client that ignores them behaves exactly
as before.

### The certificate a queue entry holds

| Field | Values |
|---|---|
| `holds_certificate` | `true` \| `false` |

True when a witness certificate is held for the bytes this entry was pinned
to. Both witness routes produce one: `POST /v1/witness` returns a
certificate, and `POST /v1/witness/admission` returns a certificate AND
admission evidence. The daemon stores either as a single
`trace_commons.witness_review.v1` artifact under a single pin, so "did the
contributor complete step 1 or step 2" is not two questions.

**`holds_certificate` is ALWAYS PRESENT, on every entry, for every
contributor** -- the `attestation` rule, not the `eligibility` one, and for
a reason of its own. This is one fact with two readings:

| Contributor | Reads `true` as |
|---|---|
| without an invite | this session is a candidate for submission |
| with an invite | this session is cryptographically attested |

The fact does not differ between them; only the wording does. The shell
picks the wording from the invite status it already holds for the
eligibility surface, and MUST NOT ask the daemon a second question to get
it. Emitting the field only under the signup flag would put the second
reading out of reach of exactly the contributors it is written for.

**This is not `attestation`, and the two must not be conflated.**
`attestation` answers whether the session carries a checkable copy of its
last model call. `holds_certificate` answers whether a witness
certificate is held over the reviewed bytes. A session can have either
without the other, and *holds a certificate*, *is attestable* and *was
attested* are three different facts. A shell deciding what to put in a
certificate-held list MUST read `holds_certificate` and never the mark.

### `certificate_detail`

`certificate_detail` takes an `entry_id` for a pending entry whose
`holds_certificate` is true. It verifies the stored artifact still matches its
queue pin, then returns only:

```json
{
  "state": "held",
  "verification": "verified_at_review",
  "redacted_sha256": "…",
  "residual_risk_verdict": "low|medium|high",
  "redaction_policy_version": "…",
  "witness_measurement": "…",
  "issued_at": 0,
  "expires_at": null,
  "expiry_state": "not_issued",
  "signer": "0x…",
  "signature_present": true,
  "admission_evidence_present": false,
  "inference_receipt": {"state": "certified|uncertified", "reason": "…"}
}
```

`inference_receipt` is `null` for reviews written before the client recorded
that fact. `expires_at` is always `null` under current certificate schema; the
shell must not invent an expiry. Missing, malformed, stale, or unavailable
artifacts return fixed labels and no partial certificate. The daemon never
returns `envelope_bytes`, `signature_hex`, or `certificate_json`.

Derived from the pin rather than stored, so it cannot drift from it: the
artifact is written and the pin recorded under one queue lock. An entry
re-offered because its bytes moved loses the pin and therefore the claim --
the certificate covered the old bytes.

The Tauri client calls it from the redacted-view inspector, for an entry
whose `holds_certificate` is true, to show the measurement and signer the
witness was checked against at review (K11). The other shells do not call it
yet.

### `route_disclosure`

K11 of the connect-and-forget consent design ("The disclosure"): the facts
behind the disclosure screens. Read-only and local -- it reads the config,
the daemon's settings and the daemon process's environment, and makes no
network call. Answered by the daemon rather than read by a shell from the
config file, because the daemon is the process that sends: the privacy filter
`TRACE_PRIVACY_FILTER_BACKEND` attaches is the daemon's environment, and
`ironwire_attested_bodies` is the daemon's setting.

```json
{
  "route": "witness",
  "witness": {
    "state": "pinned",
    "url": "https://witness.example",
    "signing_address": "0x…",
    "pinned_measurements": ["mrtd=…,rtmr0=…"],
    "origin": "published_at_join"
  },
  "local_filter": null,
  "receipts": { "endpoint_configured": true, "check_attestation": false },
  "attested_bodies": false
}
```

| `route` | Means |
|---|---|
| `witness` | a pinned witness: each session is sent unredacted to it, and it redacts inside its enclave |
| `witness_refusing` | a witness is configured and refusing (nothing pinned, or a pin that does not parse): nothing is sent |
| `local` | no witness: redaction runs on this machine and the unredacted session does not leave it |
| `not_enrolled` | no enrollment: nothing is sent |
| `settings_unreadable` | the config could not be read: nothing is sent |

`witness` is present whenever a witness is configured, pinned or refusing, and
`null` otherwise. `state` is the same `WitnessTrustState` the witness settings
card shows; `pinned_measurements` are verbatim, in stored order.

`witness.origin` says how the witness came to be configured:

| `origin` | Written by |
|---|---|
| `published_at_join` | a NEAR AI or wallet join, from what the commons publishes, without asking |
| `connected_inference` | `inference_connection_install`, on the contributor's confirmation |
| `settings` | a shell's witness settings (Tauri, the C ABI's `tc_witness_configure`, GTK) |
| `environment` | `TRACE_COMMONS_WITNESS_*` read at enrollment |
| `not_recorded` | no record, or a record written for a different witness |

The config records the origin beside the witness, keyed by a digest of the
witness's URL, signing address and pins (`ContributorConfig::witness_origin`),
so a witness changed later by something that does not record an origin reads
as `not_recorded` rather than inheriting a stale answer. Configs written before
the record existed read the same way. The record is not an input to
`input_fingerprint`.

`local_filter` is present only when `route` is `local`: `none`, `near_ai`,
`self_hosted`, `sidecar`, or `invalid` (a setting the redactor refuses to
build, so nothing is sent). It is the config's `pii_filter` when set, and
otherwise the environment's backend.

**What this does not report, because this client does not know it:** which
privacy filter the witness itself calls (it is set in the configuration the
witness's measurement covers, which this client never reads), that filter's
attestation, and whether a receipt signer is one the witness pins. A shell
says so rather than implying either way; `consent_copy::route_disclosure_copy`
has the sentences.

A shell renders the words `consent_copy::route_disclosure_copy` gives for these
facts and branches on neither. A shape it cannot read -- an unknown `route` or
`origin` from a newer daemon -- is refused, not rendered as the nearest value.

The native shells get the words through the C ABI rather than linking the
crate: `tc_route_disclosure_copy(facts_json)` takes this method's result as
sent and returns `{"facts": .., "copy": ..}`, or NULL for anything it cannot
read; `tc_route_disclosure_unreadable_copy()` returns the sentences a surface
shows in that case, under the section title (`title`, `panel`, `session`); and `tc_certificate_detail_copy()`
returns the labels for `certificate_detail`. GTK calls
`consent_copy::route_disclosure_for_wire` directly. No shell writes a
disclosure sentence of its own.

### The attested-inference record

The attestation mark above is computed at discovery, before any receipt is
fetched and before any witness is contacted. What happened *after* that --
whether the witness that certified this entry's review was actually handed a
receipt with the bodies -- was recorded nowhere: the certificate carries no
such field, the receipt is discarded after the witness call, and ingest
stores nothing about it. This field is the daemon's own record of that fact.

| Field | Shape |
|---|---|
| `attested_inference` | `{"state": "certified"}` or `{"state": "uncertified", "reason": <label>}` |

| `state` | Means |
|---|---|
| `certified` | a receipt was offered with the bodies and the witness issued a certificate over them |
| `uncertified` | the review was certified without attested inference; `reason` says why |

| `reason` | Means |
|---|---|
| `no_attested_call` | the session joined no inference hop, or the hop recorded no verbatim bodies |
| `bodies_withheld` | the session carried an attested call and the review was requested without inference bodies |
| `receipt_unavailable` | the session carried an attested call and no receipt could be obtained for it; the trace was certified without it |

**`attested_inference` is PRESENT only while a witnessed review is pinned to
the entry**, and then it is exactly what the stored review records. It is
ABSENT -- not `unknown`, not null -- for an entry with no witnessed review, an
entry whose review predates the record, and an entry whose pin is a local
preview. Absent means *not known*, and a shell MUST render it as nothing: an
older review may have carried a verified receipt, and nothing says either
way. Flattening absence into `uncertified` tells a contributor their work is
worth less than it may be; flattening it into `certified` is the lie this
field exists to stop.

**Written from the fact, not the intention.** `certified` is set only when a
receipt was among what the witness was handed *and* the witness answered
with a certificate. A session whose queue row says `attestation: attested`
and whose receipt fetch then fails is `uncertified` with
`receipt_unavailable` -- the mark describes what the session carries, this
record describes what the witness was given. The two disagreeing is the
case that matters, not a defect.

What it does not say: whether the receipt's *signer* was one the witness
pins. That is the witness's configuration, invisible to the client; a
certificate from a pinning witness and one from a dormant witness read the
same here.

The record lives and dies with the pin: an undone approval, a released
preview or a re-preview without a witness clears it, so a row can never
inherit an answer from a review that is no longer the one it would send.

### The credential state

`near_ai_credential_status` answers ONE label, and it is the only thing a
shell branches on:

| `state` | Means | Sentence | Action a shell may offer |
|---|---|---|---|
| `absent` | no key is kept on this machine | `credential_absent` | obtain |
| `obtaining` | a ceremony is in flight here | `credential_obtaining` | cancel |
| `failed` | the last attempt ended without a key | `credential_failed` | obtain |
| `cancelled` | the last attempt was stopped by the contributor | `credential_cancelled` | obtain |
| `present` | a key is kept on this machine | `credential_present` | forget |
| `storage_unavailable` | the store did not answer (locked, denied, platform error) | unlock and restart | forget |
| `cleanup_required` | sign-in removed locally, but its store entries could not be deleted | unlock, then Forget again | forget |
| `storage_unentitled` | macOS: this build is not signed to reach the store at all | restarting will not help; use a released build | **none** |
| `migration_available` | macOS: the sign-in an earlier build kept in the login keychain has not been moved | the sign-in is still in the login keychain; move it | migrate (`near_ai_credential_migrate`), with `credential_migrate_explains` beside it |
| absent/empty field | this daemon does not answer the question | `credential_unreported` | **none** |
| anything else | the state could not be read | `credential_unknown` | **none** |

The sentence, the tone and the action are chosen by the shared Rust tables --
`tc_near_ai_credential_state_line`, `tc_near_ai_credential_state_tone` and
`tc_near_ai_credential_action` -- never by shell-authored branching, and a
shell must not recover any of the three by reading another.

**Neither unread case may render as `absent`.** "No key is kept here" in
front of somebody who has one invites a second sign-in, at a third party,
that their own account will list and nothing on this screen will ever mention
again. That is why the unread states also answer *no action*: the action in
question mints the second key.

The precedence is the daemon's, not a shell's. A ceremony in flight outranks
a key already stored, because somebody with a browser tab open is waiting on
that and not on what they had before; a stored key outranks the ending of an
older attempt; `absent` is the answer only when there is nothing else to say.

`attempt_id` and `attempt_status` are echoed only to a caller that already
named the current attempt. A caller that names none, or names a stale one,
still gets `state`, since the resting state may be disclosed. The response reveals nothing
that would let it cancel somebody else's ceremony, and the browser URL is
never re-served.

`credential_cost` is the sentence that has to sit in front of the obtain
action, and it is the counterpart of `offer_exposure`: getting a key opens a
browser, signs the contributor in to a company that is not this app, and
mints a key this app then keeps. `credential_forget_explains` is its
counterpart at the other end -- forgetting is LOCAL, the key stays valid at
the service until the contributor removes it there, and this app cannot do it
for them, which is why `near_ai_credential_forget` answers `revoked: false`
rather than leaving "removed" to be read as "revoked".

No sentence on this surface renders a key, a key prefix, an id or an account
name, and none of them carries a hole a shell could fill with one.

The three per-harness states are not two. `harness_connected_nothing_seen`
says a tool's own settings send its calls here; `harness_answering` says a
call actually arrived, and it is the only one of the three that means the
tool works. `harness_slot_taken` reports a slot left exactly as the
contributor had it and must never be rendered as a fault or paired with an
action that takes it over.

`harness_not_installed` is what a row whose tool is not on this computer
shows INSTEAD of any state sentence. Such a tool is listed and disabled
rather than hidden -- a tool left out cannot be told apart from one the app
was never taught about -- and it must not also carry
`harness_not_connected`, which claims a tool's own settings still send its
calls wherever they went before.

`harnesses_spend_scope` is the sentence that has to accompany the amount
`tc_harness_spend_line` assembles: only calls answered on **this** computer
are in the figure. Work already covered by a monthly plan is excluded. A
contributor signed in to the same account on a second machine, or in a
browser, has spent more than the figure says, and the number without this
sentence is a lie of omission. It is drawn beside the amount and only when
the amount is drawn -- a scope line on its own qualifies a figure that is not
on screen. Neither this sentence nor the amount may say anything about a
balance: the amount is what went out, and nothing here is an account total.

The four `harness_plan_*` sentences plus `harness_unreadable_config` are the
answers of `tc_harness_outcome_line`, one per non-committable `harness_plan`
outcome. `changes` answers the empty string, because a plan with changes in
it shows them. Every other outcome writes nothing, and without a sentence its
preview is a title, a path and a way out.

State sentences, outcome sentences and tones are chosen by the shared Rust
table rather than by shell-authored branching. Copy-field inventory parity is checked separately
from successful decoding of required fields.

The shared `tc_private_inference_write_confirmed` decision accepts optional
requested-on, echoed-offer-seen, and echoed-on booleans (`-1` absent, `0` false,
`1` true). Decline requests no switch and requires only a true marker echo;
a settings switch requires both the marker and a present matching switch.
Invalid inputs refuse confirmation. Shells keep their previous confirmed
state and display `write_unconfirmed` on failures, including ambiguous
transport failures that may have followed persistence. A daemon predating
`private_inference_offer_seen` cannot acknowledge an answer: the offer remains
until a supporting daemon confirms it. Missing values are not explicit off.
The GTK switch starts disabled with the shared unavailable explanation until
settings arrive. macOS serializes offer and settings writes through one
pending guard; its toggle stays bound to the last confirmed settings.

`private_inference_offer_seen` takes a boolean, and it is **not a fourth
answer**: it records that the question was put, never what the answer was.
A client that offers the switch on first start writes `true` here on
*either* button, so declining is remembered exactly as accepting is and the
contributor is not asked again on the next launch. Nothing in the daemon
reads it except a client deciding whether to ask; setting it starts nothing,
stops nothing, and changes no other value.

It lives here rather than in each application's own state file because the
fact belongs to the machine and not to a window. On Linux the daemon
outlives the GTK shell and a second shell would otherwise re-ask; on macOS
and Windows the app is the daemon, and this file is the thing that survives
a reinstall of neither more nor less than the settings do.

Default `false`, and a settings file written before this key existed loads
with it false. That default is what makes an offer appear on the first start
after an upgrade as well as on a fresh install: an installed build's
settings file has no such key, so the first build that knows the key reads
it as unanswered and asks once.

#### `scrub_check`

The Settings row "Scrub check" (K4 of #1118). `set_settings` takes a string,
`"automatic"` or `"manual"`; anything else, including another case, a boolean
or `null`, is `bad_params` / `settings-invalid-value` and changes nothing.
`get_settings` always reports the key as one of the two strings, never
`null`: `"automatic"` until the contributor chooses otherwise. It is persisted
with the other settings and read at
each watcher pass and each upload, so a change reaches a running daemon
without a restart.

It decides what happens to a session in a folder set to share automatically
(`auto_upload`), and only there. A person's own `approve` is never held by it:
they are the second look.

An existing settings file with an absent/null choice records
`scrub_check_defaulted_on_upgrade: true`, retained through unrelated saves.
An older armed policy with no settings file also records that provenance;
a new-format policy distinguishes fresh installs from those upgrades.
At startup, already-armed folders receive a one-time persisted notice in
`arming_rewordings` with `scrub_check_defaulted: true`. All shells render it
through the shared notice formatter and acknowledge its exact id. Fresh
installs and explicit Manual choices receive no upgrade notice. A saved
policy migration marker prevents replay after acknowledgement or later arming.

| Value | Armed folders |
|---|---|
| `automatic` (**the default**) | send on their own, **except** a session that is worth a second look (`second_look` would be `nothing-matched`, `looks-unsure` or `trimmed-to-fit` for the envelope about to be sent, see "The scrub state and `second_look`"). That one is held for a person under `second-look-review-required` and never moves on its own. |
| `manual` | nothing is sent without a person. The watcher approves nothing on anyone's behalf, and an approval made before the switch is held under `scrub-check-manual` when the uploader reaches it. |

**Where the Automatic hold happens.** The watcher approves an armed session
without building anything, so at that point nobody has counted its marks: it
is `not-yet-scrubbed`, and treating that as fine would be wrong. So the hold
is decided by the uploader, on the envelope it has just built for the send,
after every refusal and before anything reaches the commons -- the same point
as the witness's residual-risk hold. It is counted exactly as a pinning
preview counts (`ScrubCounts::of` over the redaction map and
`preview::body_of`), so the count is exact and never "not yet scrubbed"; a
body the unsure-span detector cannot read counts as `looks-unsure`. On the
local-redaction path nothing has left the machine
when a session is held. With a witness configured, the witness has already
seen the session (as with `witness-risk-review-required`): the hold stops the
upload to the commons, not the send to the enclave.

A session newly held for a person receives a full review window from its
first hold (`review_started_at`); its original discovery time is preserved.
Repeated holds and restarts do not extend that window. On witness routes,
the first hold has already incurred witness/claim work; approving it may
repeat that work before upload.

**What a hold leaves on the entry.** The entry goes back to `Pending` with
`reason_label` `second-look-review-required`, and the envelope the hold was
decided on is **pinned** to it: saved through the same
`approved_envelope::save` a preview pins with, with its digest as the entry's
pin and the counts recorded beside that digest. So `list_pending` and
`snapshot` show `scrub: "scrubbed"`, `marks`, `content_marks`,
`unsure_spans` and the `second_look` reasons that held it, and the person's
review is of exactly those bytes; their `approve` sends them. The pin is kept
and released like any pinned `Pending` preview: swept when the entry
resolves, released after the preview age limit (the entry then reads
`not-yet-scrubbed` and the next review builds again). Two cases pin nothing
and leave the entry reading `not-yet-scrubbed` while still holding it: a
witnessed envelope, whose certified bytes are pinned only through a witnessed
review, and a save that fails. The daemon logs the second-look labels the
hold was decided on (labels only).
`second-look-review-required` is one of the reasons a session is held for a
person (see the group `approve` section): the watcher does not approve it
again, and a group `approve` leaves it out and counts it in `excluded_held`.
A person's `approve` of that one entry sends it.

`scrub-check-manual` is deliberately **not** such a reason. While Manual is
set the watcher approves nothing anyway, and once Automatic is set again, an
armed folder's waiting sessions going through the Automatic check -- its hold
included -- is what the contributor asked for.

**Switching to Manual.** A `set_settings` call that changes `scrub_check` to
`"manual"` returns every unsent approval made on the contributor's behalf
(`Approved`, approved unattended) to `Pending` under `scrub-check-manual` at
once, instead of leaving it reading approved until the uploader reaches it.
A person's own approvals, and anything already uploading, are untouched. The
reply then carries `scrub_check_returned_to_waiting`, the count moved
(possibly `0`), and a `queue_changed` event follows when it is non-zero. The
key is **absent** on any call that did not switch to Manual, including one
that sets Manual when it was already Manual.

**The default is `automatic`** (decided on #1139). A fresh install holds a
session worth a second look in an armed folder without anyone choosing
anything. An upgraded install does too: a settings file written before this
key existed, or one holding the `null` an earlier build wrote for "never
chosen", loads as `automatic`, so on upgrade an armed folder starts holding
the sessions where nothing matched, something looks unsure, or it was trimmed
to fit. An explicit `"manual"` is kept across restarts and upgrades; only an
unset value becomes `automatic`. `null` cannot be set: it is not a mode, and
a caller that means the default sends `"automatic"`. A shell renders the choice with
`consent_copy::SCRUB_CHECK_*` (approved 2026-10-06), whose Automatic
sentence says the check only counts what was removed and does not check that
the scrubbing was right: it is not a model or quality check (the
connect-and-forget design's R1).

`max_uploads_per_day` and `max_bytes_per_day` each take a positive integer,
validated against a fixed ceiling (1,000 uploads; 5 GiB) rather than
accepted as an open field. The cap exists to bound a runaway client -- an
app that decided to upload everything should not be able to -- so making it
freely settable to any value would give up exactly the protection it was
added for. A value below the built-in default (50 uploads; 200 MiB) is
accepted with no floor beyond non-zero: a contributor throttling their own
uploads is legitimate and is not what the ceiling guards against. Zero is
refused (`bad_params` / `settings-invalid-value`) rather than treated as
"stop uploading" -- that state already exists and is spelled `pause`, which
is visibly temporary in a way a cap of zero would not be. A value above
either ceiling is refused the same way. Like the other numeric knobs, the
new setting is read at each upload pass, so a change reaches an
already-running daemon -- including one that reached its old cap with
approved traces still waiting -- without a restart, and it is written to
the persisted settings file in the same call, so the raised value survives
one.

### History provenance (K7)

`list_history` rows (`HistoryRecord`) carry two more fields, for the
design's per-trace credit log: "just now · you approved · Worked" versus
"Wed · armed · went without asking".

```json
{
  "submission_id": "…",
  "...": "…existing fields unchanged…",
  "approved_unattended": false,
  "approved_verdict": "worked"
}
```

- **`approved_unattended`** -- whether this trace reached upload without the
  contributor deciding: `true` for an armed (`auto_upload`) folder's send,
  `false` for a person's own approval, and **`null` when it was not
  recorded**. Carried from the queue entry's own
  `approved_unattended` at the moment of upload (`daemon::uploader`, via
  `SubmitContext::set_upload_provenance`), not derived after the fact --
  the entry itself is gone by the time a client asks, so this is the only
  place the fact can still be answered from.
- **`approved_verdict`** -- the contributor's verdict at approval time
  (`worked` / `partly` / `failed`), or `null` when none was given. Carried
  from the same queue entry, the same way.

Both are fixed labels, both `#[serde(default)]` on the wire: a row whose
receipt predates this reads `approved_unattended: null` and
`approved_verdict: null`. **A client must render `null` as "not recorded",
never as "you approved"** -- an older row may well have been sent by an
armed folder, and a consent label must not guess in the reassuring
direction. A row written by the CLI's own `submit` command reads
`approved_unattended: false` (a person ran the command) and carries the
`--verdict` it was given, if any.

Render both as fixed labels, never as prose composed from anything else on
the row -- "you approved" and "armed" are not synonyms for any existing
status, and mean nothing about whether the submission was later accepted,
quarantined, or withdrawn.

### Sizes in history, and the would-send size (K10)

Before this, a history row carried no bytes at all -- only the server's
status, credit and explanation prose -- so the History graph could count
sessions but not weigh them, and nothing on a queue entry said how large an
upload would actually be once redaction ran. `size_bytes` on a queue entry
is the raw session file on disk, which redaction shrinks or reshapes; it was
never a stand-in for either figure.

`list_history` rows (`HistoryRecord`) carry one more field:

```json
{
  "submission_id": "…",
  "...": "…existing fields unchanged…",
  "uploaded_bytes": 15320
}
```

- **`uploaded_bytes`** -- the serialized size, in bytes, of the redacted
  envelope this submission actually sent, or `null`. Recorded once, at
  upload time, in `submit_loaded`: a witnessed submission reports the
  witness's own certified `envelope_bytes.len()` -- the exact wire bytes
  `/v1/traces` received -- and an ordinary submission reports
  `envelope::envelope_size` on the final, grant-stamped envelope, which is
  what the same call serializes onto the wire. Written onto the receipt
  by `submit_loaded`, and carried from there onto the history row.

Every queue entry (`list_pending`, the `snapshot` event) carries a sibling
field:

```json
{
  "entry_id": "…",
  "...": "…existing fields unchanged…",
  "size_bytes": 48210,
  "would_send_bytes": 15320
}
```

- **`would_send_bytes`** -- the serialized size of the redacted envelope a
  preview pinned for this entry, or `null` when nothing is pinned: an entry
  never previewed (an armed auto-upload, an approve-all), or one written
  before this field existed. The same figure [`preview`](#preview)'s own
  `would_send_bytes` reports, mirrored onto the entry at the moment a
  preview pins it (`QueueEntry::previewed_envelope_digest`'s own sibling
  field) so a queue list can say it without opening the stored envelope for
  every row -- the same reason `attested_inference` is mirrored there. It
  is cleared wherever the pin is (keep, Undo, a released preview, a revoked
  approval). For a local preview it is measured before the upload stamps
  the grant's scopes onto the envelope, so it can be a few bytes short of
  `uploaded_bytes`; treat it as the size to expect, not a promise.

Both `uploaded_bytes` and `would_send_bytes` are `Option<u64>`,
`#[serde(default)]` on the wire, so a cached row or a queue line written
before either field existed still loads, reading `null`.

### Withdrawal dates on revoked rows (K12)

A locally driven withdrawal (`withdraw`, `withdraw_bulk`) stamps
`withdrawn_at` the moment it runs, so a `withdrawn` row has always been able
to show a date. A withdrawal made on the web instead, which this daemon only
learns about the next time it polls submission status and gets back
`revoked`, had no equivalent: the server's status read-back
(`TraceSubmissionStatusUpdate`) carries a status and credit figures, never a
timestamp, so a `revoked` row had no date to show at all.

`list_history` rows (`HistoryRecord`) carry one more field:

```json
{
  "submission_id": "…",
  "...": "…existing fields unchanged…",
  "status": "revoked",
  "revoked_at": "2026-08-09T12:00:00Z"
}
```

- **`revoked_at`** -- when this row was first seen carrying status
  `revoked`, or `null` for a row that is not (or not yet) revoked, or one
  written before this field existed. **Not** the moment of the web
  withdrawal itself, which this daemon has no way to learn -- the moment
  this device first discovered it, the same honest compromise
  `observed_modified_at` and `review_started_at` already make elsewhere on
  this contract for a fact only discoverable by polling. Stamped once, by
  the history join that first sees `revoked`, from that poll's own
  `last_refreshed_at`, and carried forward on every later refresh exactly
  the way a local `withdrawn_at` already is -- a later poll that merely
  re-confirms the same `revoked` status must not push the date forward.
  Two cases stay `null` on purpose: a row the cache already held as
  `revoked` before this field existed (it was withdrawn on a day this
  device cannot know, and the upgrade's first poll is not that day), and a
  withdrawal this device drove itself, which already carries `withdrawn_at`
  and whose `revoked` read-back is not a web withdrawal.

`#[serde(default)]` on the wire, like every other field on this row, so a
cached row written before this field existed still loads, reading `null`
until the next poll re-observes the revocation and dates it.

### `history_rollup`

```json
{
  "week":     {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "month":    {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "all_time": {"submitted": 0, "accepted": 0, "quarantined": 0, "other": 0},
  "credit_pending": 0.0,
  "credit_final": 0.0,
  "quarantined": 0,
  "taken_back": 0,
  "last_refreshed_at": null
}
```

`quarantined` is reported separately and must be rendered separately.
Quarantine means **held for operator privacy review**, not rejected. A
contributor who sees it grouped with failures reads it as rejection.

`taken_back` (K7) is how many history records have been taken back -- the
design's third tile, "2 taken back": withdrawn from this daemon (a local
`withdrawn_at`, status `withdrawn`), or reported `revoked` by the server
after a withdrawal on the web (#1112). Each of `all_time`/`month`/`week`
counts the same rows in a `withdrawn` bucket, so a withdrawn row is counted
exactly once and never in `other`. A local withdrawal survives the next
history refresh: the refresh carries `withdrawn_at` over from the cache it
replaces. Both fields are `#[serde(default)]` on the wire, so an old client
that ignores them is unaffected.

`last_refreshed_at` is `null` when history has never been refreshed from the
server; show staleness rather than presenting a stale cache as current.

The `submitted` bucket means **uploaded, no verdict back yet**. It is a real
and ordinary state, and it now appears promptly: an upload pass writes its
receipts into the history cache immediately, without calling the server, so
a trace that has just gone out is counted as `submitted` within seconds
rather than at the next `history_poll_secs` boundary. Those rows carry
`last_refreshed_at: null`, because nothing has been read back for them yet.

The bucket also counts `processing`, the status the versioned pipeline's
receipt carries: uploaded, no verdict **reported** yet. That is not the same
as no verdict. Admission runs inside the upload request, so a trace it
quarantined or rejected gets the same `processing` receipt, and receipts are
never rewritten. Render and offer a `processing` row exactly as `submitted`:
same words, same withdrawal stage.

A `list_history` row keeps the raw `processing` until a status read-back
reports something for that submission. A server with the pipeline's product
layer (#1143) reads a pipeline submission back in `main`'s vocabulary:
`quarantined` or `rejected` when Admission decided so, and `accepted` for an
admitted trace, before Review has decided it. On such a server the first
read-back after the upload replaces `processing`. An older server leaves a
submission whose stored status is still `received` out of the read-back
altogether, so there an admitted trace's row stays `processing` until Review
promotes it, and only a quarantined or rejected one changes sooner.

The CLI's submit short-circuit and its picker's SUBMITTED marker read a
`processing` receipt through that read-back: once the history cache holds
`rejected` for the submission, the session is no longer already submitted,
and `quarantined` is reported as `quarantined`.

The daemon then asks the server for verdicts about ninety seconds after an
upload pass, rather than waiting out the full `history_poll_secs` interval
(1800 by default). A burst of uploads produces one such read-back, not one
per upload, and no two read-backs are ever less than two minutes apart. A
verdict, a quarantine, or a withdrawal moves the row out of `submitted` on
the next refresh; the count can go down as well as up.

Render `submitted` in the contributor's words: sent, waiting to hear back;
and never as an error.

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
    { "name": "debugging_evaluation", "title": "…", "description": "…", "always_on": true, "grants_data_use": true },
    { "name": "public_attribution", "title": "…", "description": "…", "always_on": false, "grants_data_use": false }
  ]
}
```

`always_on` is `true` for exactly one scope, the floor scope every
contributor implicitly grants. `grants_data_use` is `false` for scopes (such
as `public_attribution`) that carry no data-use grant of their own -- do not
present those beside real data-use scopes with equal visual weight.

`title` is the scope's short bold label (owner ruling, 2026-10-06). Every
shell draws it as given and keeps no table of its own; it is never the wire
name with its underscores replaced. A shell that receives a scope without a
`title` fails closed: macOS, GTK and Windows refuse the list (or the row),
so nothing is offered under words the core did not supply.

### `set_consent_scopes`

Takes an optional `scopes` array of wire-name strings (omitted means the
floor scope only) and replaces the enrolled config's consent scopes.
Requires an existing enrollment (`unavailable` / `not-logged-in` otherwise).
Appends a `consent-scopes-changed` audit entry.

It also records that the contributor chose the scopes: the config field
`consent_scopes_chosen` (default `false`) becomes `true` when `scopes` names at
least one scope, and `false` when it is omitted or empty, since that saves the
floor scope without naming anything. `grant_automatic` requires it. No
enrollment path sets it, and a new enrollment writes a config without it.

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

Acknowledging also re-offers every session the gate had refused, and
`reoffered` is how many. A session sent while the notice was outstanding is
refused with `near-ai-notice-not-acknowledged`, which is a refusal about
timing rather than about the session -- but a refused entry is never moved
again, and the watcher does not re-offer a session whose file is unchanged,
so without this those sessions were lost for good. They return as `Pending`
with their old approval cleared, because that approval was given before the
disclosure it depended on: an `auto_upload` folder re-approves them on the
next poll, and any other folder asks again. A `queue_changed` event is
published when any move.

A refused entry whose file already has a live entry (the session grew while
it sat refused, and the watcher offered the new content) is not revived: it
is marked `superseded` with `session-changed-after-offer`, and is not counted
in `reoffered`. The same step -- clear the label, re-offer, supersede -- also
runs on every daemon tick once the notice marker exists, so a notice
acknowledged through the CLI, which writes the marker without calling this
method, gets the same result. It runs whether or not the daemon is paused,
quiesced for an update, or in dry-run, since it sends nothing.

### `near_ai_balance`

What the contributor's NEAR AI account has left, read with the session the
credential ceremony retained. A `state` and a set of numbers:

```json
{
  "state": "known",
  "currency": "USD",
  "scale": 9,
  "remaining_nanos": 8500000000,
  "spend_limit_nanos": 10000000000,
  "total_spent_nanos": 1500000000,
  "total_requests": 12,
  "total_tokens": 3456,
  "observed_at": "2026-09-08T09:17:15Z"
}
```

`state` is one of:

| `state` | What it means | What a client should say |
|---|---|---|
| `known` | The service answered. | The figures, with their age. |
| `no_session` | Nothing is stored. The account was never connected, or was forgotten. | Offer the credential ceremony. |
| `session_expired` | The stored session was refused. | Offer the ceremony again. It does **not** require forgetting first: the ceremony overwrites both records. |
| `no_organization` | The account has no active organization, which is a real state -- the service creates one on signup **best effort**. | Point at the contributor's NEAR AI account page. |
| `unavailable` | The service did not answer, or answered something this daemon cannot read. | Say nothing about the balance. |

Three rules bind a client here, and they are the reason the shape is this
shape rather than a bare number:

- **Every numeric key is present and `null` in every state but `known`.** An
  absent key would be ambiguous between "this daemon is too old to know" and
  "this daemon knows it does not know"; a `null` is only ever the second.
  **A `null` must never be rendered as `0`.** Zero is a real balance and
  means the contributor is out of credit.
- **`remaining_nanos` and `spend_limit_nanos` can be `null` even when `state`
  is `known`.** They are nullable in the service's own schema: an
  organization with no credit ceiling configured genuinely has no remaining
  balance to report. Same rule -- not zero.
- **`observed_at` is when this daemon asked**, not the service's own
  `updated_at`. It is there so a client can say how old the figure is.
  A stale balance presented as current is worse than no balance; the daemon
  will not serve a cached figure older than a minute, and returns a failure
  state rather than the last good number when a read fails.

The units are the service's own: fixed scale 9 (nano-dollars) and USD,
carried on the wire rather than assumed, and not rounded by the daemon.

The call performs network I/O -- it may exchange the stored refresh token
for an access token before making two management reads -- so it is on the
async dispatch path and a client should not call it on a paint loop.

#### A shell does not write these sentences, or this arithmetic

Every word and every figure on this row is assembled in
`trace-commons-contributor`'s `private_inference_copy` and reachable across
the C ABI. A shell reads `state` and the numbers off this wire and passes
them straight back; it does not branch on the state string to pick a
sentence, and it does not divide by `10^scale` itself.

| Call | What it takes | What it gives |
|---|---|---|
| `tc_near_ai_balance_state_line` | `state` | The sentence. `known` gives `""`; a state this build cannot read gets its own sentence and borrows nobody's. |
| `tc_near_ai_balance_state_tone` | `state` | `TC_PRIVATE_INFERENCE_TONE_*`. `known` is the only `_CLEAR`, and it means the READ succeeded -- nothing judges the amount. |
| `tc_near_ai_balance_action` | `state` | `TC_CREDENTIAL_ACTION_*`. `OBTAIN` for `no_session` and `session_expired`, `NONE` for everything else. |
| `tc_near_ai_balance_amount` | `present`, `nanos`, `scale` | The bare amount, e.g. `$8.50`. |
| `tc_near_ai_balance_remaining_line` | `present`, `nanos`, `scale` | `Left to spend: $8.50.`, or the no-limit sentence when `present == 0`. |
| `tc_near_ai_balance_limit_line` | `present`, `nanos`, `scale` | The ceiling, or `""`. |
| `tc_near_ai_balance_spent_line` | `present`, `nanos`, `scale` | What the account has spent, or `""`. |
| `tc_near_ai_balance_observed_line` | `seconds_ago` | How long ago **this daemon** asked. Negative gives `""`. |

`present` is `0` for a `null` on the wire and non-zero otherwise. It is a
separate argument rather than the out-of-range-integer convention the other
money exports use, because these amounts are **signed**: an overdrawn account
is a negative figure, and folding `null` onto `negative` would render a real
debt as no figure at all.

The rounding is **down**, toward minus infinity, so a printed figure is never
larger than the one that arrived; a nonzero amount under a cent is
`less than $0.01` and not `$0.00`.

#### Three neighbouring fields, three different meanings for a missing key

Read this before writing a client that touches more than one of them. They
share a wire and nothing else: each is an answer to its own question, and
knowing one tells you nothing about the next.

- **`attestation_reason`: branch on the key's presence, not on the mark.**
  This is the one that catches people, because the mark does not determine
  the shape. `attested` never carries a reason and both unattested marks
  always do -- but `unknown` has two sources, and they differ: a row written
  before the field existed has **no** reason, while a send refused with
  `admission_receipt_unavailable` carries `receipt_unavailable`. The second
  is a retraction rather than a refusal -- the receipt comes from a service
  that can be down -- and that reason is the only thing telling the
  contributor it may work later. Do not "simplify" it to always-absent.
- **`eligibility`: an absent key means the question does not apply.** A
  present `unknown` means the question applies and was not answered.
- **`near_ai_balance`: every numeric key is present and `null`**, and the
  `null` is load-bearing -- it means "this daemon knows it does not know". An
  **absent** key here means only "this daemon is too old to answer", and a
  client that treats it as a `null` is asserting a state the daemon never
  reported. **A `null` must never be rendered as `0`.**

Each of these answers its own question and they happen to share a wire.
There is no convention here to generalise from: read the contract for the
field you are touching, and do not carry a habit across from another one.
A field added later is a fourth answer to a fourth question, not a fourth
instance of a pattern.

### `commons_credit_summary`

K7's credit relay: what the commons itself says, in two halves, each read on
the credential that can read it. The daemon never read either before K7; the
design's "Settlement off" line and per-trace credit log needed them.

- **Posture** (`posture_state` and the three `commons_settlement*` /
  `commons_graded` fields) comes from the DEVICE route
  `GET /v1/contributors/me/settlement-posture` (#1119), with a
  device-signed status claim like `status`'s read-back. So a contributor
  with no account session still learns "Settlement off".
- **Points** (`points_state` and the rest) come from the ACCOUNT route
  `GET /v1/account/credit-summary`, with the account session
  (`crate::account_auth`, the one `withdraw` presents), only when one is
  stored.

```json
{
  "posture_state": "known",
  "commons_settlement": "disabled",
  "commons_settlement_explanation": "Credit is recorded but not settled: on-chain settlement is not enabled on this deployment, so this figure stays pending.",
  "commons_graded": false,
  "points_state": "known",
  "commons_points_earned_this_period": 12,
  "commons_points_lifetime_earned": 45,
  "commons_pending_review": 2,
  "commons_currency_code": null,
  "commons_currency_earned_this_period": null,
  "commons_period_start": "2026-09-01T00:00:00Z",
  "commons_period_end": "2026-10-01T00:00:00Z",
  "observed_at": "2026-09-28T09:17:15Z"
}
```

Each state is `known` or `unknown`. Every failure of a half collapses that
half to `unknown`: for points, no account session; for either, a transport
failure, or **any HTTP status other than exactly 200** (a 2xx that is not
200 included, the rule `account_admission::fetch_status` applies to its
sibling account route). This method never returns an IPC error for any of
them: like `near_ai_balance`, it **always succeeds**, because missing
figures are a fact about the machine or the network, not a caller mistake.

**Every field of an `unknown` half is present and `null`.** The same rule
`near_ai_balance` documents: an absent key would be ambiguous between "this
build is too old to know" and "this daemon knows it does not know," and a
`null` must never be rendered as a zero point balance or as "settlement is
off" -- it means the daemon could not ask.

`commons_currency_code` and `commons_currency_earned_this_period` are relayed
**only when the account route's own posture says `graded` and settlement is
`http`**; otherwise both are `null`, together. The app never shows a dollar
figure it cannot verify: a currency amount beside ungraded points, or under
a settlement mode that moves no money, would be one. `null` makes no claim
about what a point is worth.

`commons_period_start` / `commons_period_end` are the window
`commons_points_earned_this_period` covers.

The answer is cached per daemon for 60 seconds when both halves are known
and 15 seconds otherwise (the account route allows 30 calls per window, and
a shell may ask on every screen open). An answer carrying a
`credential_warning` is not cached.

When the account route rotated the session and the daemon could not store
the new one, the answer carries `credential_warning:
"commons_credential_storage_unavailable"`, as `withdraw` and the public-run
methods do.

`observed_at` is when THIS daemon asked (the cached answer keeps the time it
was fetched), not any server timestamp, so a client can say how old the
figure is, exactly as `near_ai_balance` does.

#### Naming: these are the commons' figures, never near.ai's

#1118's open decision #1 is which ledger is of record for credit: the
commons (this method -- points, settlement posture) or near.ai
(`near_ai_balance` -- a cloud balance in dollars). **This contract does not
answer that question, and neither does the daemon.** Every field here
carries a `commons_` prefix for exactly that reason: so a client cannot
present one ledger's numbers as the other's, whether by a careless rename or
by merging the two into one balance line. A History screen may show both --
the design's mock does, "45.5 credits" beside a near.ai dollar balance -- but
it must label each by its source, not pick a winner. Do not fold this
method's numbers into `near_ai_balance`'s response, or vice versa, for the
same reason.

### Session detail and publication

These methods use the account session held in the operating system credential
store. `history_detail` returns a bounded server projection of the owned,
permanently redacted record.

It includes the creator outcome, contributed
correction, version labels, owner publication version, and up to 24 redacted evidence candidates, with each excerpt capped at 700 characters. The response
excludes raw trace content, credentials, tenant or account identifiers, and
server error bodies.

`publish_public_run` accepts the exact reviewed public fields, one to four
evidence selections, reuse permission, and an optional source slug. The daemon
binds the approval digest to the displayed outcome, contributed version, and
owner publication version; before writing, the server verifies ownership,
acceptance, metadata, evidence, correction, and public-text privacy.
Withdrawing a page advances the version, so an earlier approval cannot restore
it; publication consent remains separate from contribution and profile consent.

Every authenticated account response may rotate the native session token,
including an error response. The daemon persists that header before handling
the route result. A committed publication or withdrawal remains successful if
the credential store fails afterward; the response carries
`commons_credential_storage_unavailable` so the app can show the required
sign-in action.

Fixed route labels include `account-session-required`,
`public-run-trace-not-found`, `public-run-source-not-found`,
`public-run-conflict`, `public-run-invalid`, `public-run-publish-failed`, and
`public-run-unpublish-failed`. They contain no contributed text, wallet
material, identifier, URL, or response body.

### Tested skill workflow

The seven `skill_*` methods form one account-bound state machine. Every call
requires the current account session from the operating system credential
store. The daemon derives an opaque owner-scope SHA-256 from the ingest origin,
tenant, instance, user subject, device key id, and NEAR account id; the source
values stay inside the daemon. A changed scope clears held candidates, reviews,
evaluations, plans, and in-memory installation receipts before the new owner can
name them. Evaluation snapshots this scope and checks it again after all Cloud
calls, returning `skill-owner-changed` rather than retaining a result for a
different owner.

Install methods also load the existing `DeviceIdentity` and require its key id
to match the enrolled configuration. The public identity verifies markers
after a restart. Its Ed25519 private key remains inside `DeviceIdentity` and is
never serialized, logged, placed in a marker, or returned over IPC.

#### Candidate and review

`skill_candidate {submission_id}` re-reads the submission through the same
account-authenticated owner route as `history_detail`. It accepts only an
`accepted` record whose feedback is `correction`, whose correction matches the
generated-source-repair family, and whose task can produce a bounded text-free
fingerprint. This call performs no model inference.

The candidate may return session-derived text from that owned record under
these limits:

- `source_correction` is the exact contributed correction, bounded by the
  2,000-character approval limit and admitted only after credential-shape
  screening. The correction is deliberately preserved as the owner's decisive
  instruction; it never enters an evaluation prompt.
- `source_evidence` contains no more than six items selected from the
  permanently redacted envelope. Each excerpt remains within the session-detail
  limit of 700 characters and retains its event id and fixed label.
- The source task text is absent. `source_task_fingerprint` contains a SHA-256,
  a 128-bit similarity hash, and a bounded token count so local fixture
  selection can reject exact and lexical near-overlap.

The response also carries the default draft, the disclosed simple control,
the evaluation contract, and `replaces_review_id` when this source already has
a review. At most 12 workflows remain in memory. When capacity is full, the
daemon may evict an idle candidate, review, or completed evaluation; it refuses
with `skill-workflow-capacity` when every slot has active or unexpired work.

`skill_review {candidate_id, draft, replaces_review_id?}` validates and freezes
the owner-edited draft locally. Names contain at most 64 lowercase ASCII
letters, digits, or single internal hyphens. Descriptions contain 1 to 1,024
characters, procedures contain 1 to 12,000 characters, and both pass the public
text privacy validator and control-character checks. The response contains the
rendered `SKILL.md`, its SHA-256, and source submission and evidence lineage.
Changing an existing review requires its current `review_id`; an identical
draft returns the frozen review.

#### Evaluation

`skill_evaluate {review_id, skill_sha256}` checks the supplied digest against
the rendered review before the first catalog or completion request. A completed
report for that review is idempotently returned. One comparison may run per
daemon; another request receives `skill-evaluation-in-progress` for the same
review or `skill-evaluation-busy` for a different review.

The held-out contract is fixed and inspectable:

- A client-shipped public fixture pool contains seven repository incidents.
  Local SHA-256 and 128-bit SimHash comparison excludes exact or lexical
  near-overlap with the source task. The first six eligible fixtures are scored,
  remaining eligible ids are disclosed as reserves, and evaluation refuses
  unless six remain.
- Two metadata-only probes test whether the skill applies to one relevant task
  and stays inactive for one unrelated task. Together with the six repository
  plans, they produce eight tasks.
- Every task runs through baseline, the disclosed simple manual instruction,
  and the candidate skill, for 24 Cloud completion requests. Plan scheduling
  starts independently at position zero and puts each arm twice in every
  position.
- Every request uses temperature zero, structured JSON, disabled reasoning, a
  900-output-token ceiling, and a 90-second timeout. At most two requests run
  concurrently.

The daemon selects one model from the live NEAR AI catalog. An eligible record
must report `ownedBy: nearai`, `isReady: true`, JSON-mode or structured-output
support, and text output. `/v1/models` is consulted only when the current
`/v1/model/list` route is unavailable. A deterministic compatibility list
prefers models already exercised against the fixture contract, then falls back
to the lexically first eligible model. Updating the list requires rerunning the
transport and scoring suites. The selected id is pinned for the whole comparison;
each completion response must report that same model or the daemon aborts with
`skill-evaluation-model-changed`.

Prompts contain public fixture evidence and the arm-specific control. The
baseline receives neither instruction, the manual arm receives the fixed
one-line instruction, and the candidate arm receives the frozen generic skill.
They contain no session task, correction, evidence excerpt, account id, wallet
material, host rule, host skill, or local repository file. Fixture selection is
explicit through `selected_plan_task_ids`, `excluded_source_overlap_task_ids`,
and `reserve_plan_task_ids`. Each trial carries its model output and scoring
failures; the response also records model identity, usage, aggregate scores,
and regressions. The catalog is capped at 2 MiB. A completion response is capped
at 128 KiB, retained raw model output at 8 KiB, and the serialized report at
768 KiB.

Installation is allowed only when the candidate passes more of the six plan
tasks than baseline and manual control, passes both applicability probes, and
has no failure on a task either control passed. Replacing or forgetting the
inference credential increments its local revision, races the active operation,
and discards the result with `skill-evaluation-credential-changed`.

#### Installation and recovery

`skill_install_plan {evaluation_id}` requires a passing report whose digest
still matches its review; the daemon resolves the absolute Codex skills root
internally and never accepts a client path. Its response uses only
`$CODEX_HOME/skills/<name>` symbolic locations and contains the exact
`skill_md`, canonical signed `marker_json`, `skill_sha256`, and separate
`marker_file_sha256`.

`occupied: true` makes `can_install` false without
changing the destination. An installable plan is held for ten minutes.

The schema-v2 marker binds the install id, evaluation id, source submission and
evidence ids, tool, name, skill digest, installation time, owner-scope SHA-256,
and device key id. It contains a digest over the domain-separated canonical
signing bytes and an Ed25519 device signature; unknown fields or non-canonical
JSON fail verification.

`skill_install_commit` accepts only `plan_id`, `as_previewed_sha256`, and
`as_previewed_marker_sha256`. Both digests must match the exact bytes returned
by the plan, and the signed marker must still match the identity, owner scope,
review, and evaluation. A plan is single-use, including after a refused commit;
the client must request another plan before retrying.

Commit assembles and syncs the complete marker and `SKILL.md` package in a
private sibling directory, then publishes it with one exclusive atomic
directory rename. Platform-specific publication refuses to replace an existing
target, and the daemon verifies the staged directory identity after the rename.
Failures before publication clean only the unchanged staging directory created
by that transaction. `skill_install_status` repeats exact signed-package
verification after restart and refuses multiple matching lineage records. A
root scan stops after 512 entries.

`skill_install_rollback {install_id, source_submission_id}` first resolves the
signed status and validates the exact two-file package. It then renames the
whole target directory to an ignored quarantine name, which is the logical
rollback commit. Validation or namespace failure before that rename leaves the
loadable target in place. After the rename, the daemon retains the package in
quarantine and renames `SKILL.md` and the marker to names Codex and status do
not recognize. Success returns `removed: true, retained_directory: true`; a
post-rename cleanup race never deletes the retained package.

#### Errors, dispatch, and interruption

Errors use the ordinary response code plus a fixed `error.message`; contributed
text, paths, model bodies, credentials, and private keys never enter a label.
The method-specific labels are:

| Phase | Fixed labels |
|---|---|
| Account and ownership | `account-session-required`, `commons_credential_storage_unavailable`, `session-detail-not-found`, `session-detail-unavailable`, `skill-owner-changed`, `skill-install-identity-unavailable` |
| Candidate | `submission_id-invalid`, `skill-session-not-accepted`, `skill-correction-required`, `skill-family-not-supported`, `skill-source-task-fingerprint-unavailable`, `skill-workflow-capacity`, `skill-candidate-unavailable` |
| Review | `skill-review-invalid`, `skill-candidate-unknown`, `skill-name-invalid`, `skill-description-required`, `skill-description-too-long`, `skill-procedure-required`, `skill-procedure-too-long`, `skill-control-character`, `skill-sensitive-text`, `skill-review-unavailable` |
| Evaluation | `skill-evaluation-invalid`, `skill-review-unknown`, `skill-review-changed`, `skill-evaluation-in-progress`, `skill-evaluation-busy`, `skill-evaluation-credential-changed`, `skill-evaluation-credential-required`, `skill-evaluation-client-unavailable`, `skill-evaluation-catalog-unavailable`, `skill-evaluation-private-model-unavailable`, `skill-evaluation-request-unavailable`, `skill-evaluation-provider-rejected`, `skill-evaluation-funding-required`, `skill-evaluation-response-too-large`, `skill-evaluation-model-changed`, `skill-evaluation-held-out-set-unavailable`, `skill-evaluation-internal`, `skill-evaluation-unavailable` |
| Install plan and commit | `evaluation_id-invalid`, `skill-evaluation-unknown`, `skill-evaluation-did-not-pass`, `skill-install-plan-unknown`, `skill-install-commit-invalid`, `skill-home-unavailable`, `skill-install-root-invalid`, `skill-install-occupied`, `skill-install-plan-changed`, `skill-install-write-failed`, `skill-install-plan-unavailable`, `skill-install-commit-unavailable`, `skill-install-rollback-incomplete` |
| Status and rollback | `skill-install-status-invalid`, `install_id-invalid`, `skill-install-unknown`, `skill-install-not-owned`, `skill-install-modified`, `skill-install-extra-files`, `skill-install-multiple-matches`, `skill-install-scan-limit`, `skill-install-status-unavailable`, `skill-rollback-unavailable` |

Candidate, evaluation, and all four install methods require async dispatch; a
synchronous call returns the matching `skill-*-requires-async` label. Review is
local and remains available through the synchronous dispatcher.

There is no separate skill cancellation method. A credential revision cancels
evaluation as described above, an owner change drops the previous owner's held
state, and operation guards restore `Reviewed` or `Evaluated` when evaluation,
planning, or installation exits before its success transition. Expired and
consumed plans return `skill-install-plan-unknown` and require a fresh preview.

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
the roster; a dialog that only sets the scope leaves the contributor
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
  the action. `processing` is `submitted` (see `history_rollup`) and maps the
  same way.
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
which covers statuses this client has no stable name for). A `submitted`
selector also takes in `processing` rows, which `history_rollup` counts as
`submitted`; `processing` is not a selector of its own. It reports
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

### Connecting inference

K12 of the connect-and-forget consent design
(`docs/superpowers/specs/2026-09-23-connect-and-forget-consent-design.md`):
an invited contributor gets a redaction witness by deliberately connecting
inference, never as a side effect of joining. The five
`inference_connection_*` methods call the server routes described in
`docs/operator/inference-connection.md`. All five are asynchronous only (the
synchronous entry point answers `inference-connection-requires-async`) and all
five present the **account session**, never the device key; without a live
one they answer `unavailable` / `account-session-required` before any network
call, exactly as withdrawal does, so a shell routes the contributor to account
sign-in. A session the server rejects (expired, or a device bearer) is the same
label. A rotated session the server hands back is kept whatever the outcome,
and a result carries `credential_warning` when it could not be stored.

The flow a shell drives:

1. `inference_connection_offers` lists the operator's offers. Each offer is
   identifiers and digests only; the daemon refuses the whole list
   (`inference-connection-response-invalid`) if any offer is malformed, rather
   than showing a subset. The disclosure copy is the shell's own; the server's
   description text is not passed through. Shells take it from
   `consent_copy::inference_connection_disclosure(disclosure_version)`, which
   answers `None` for a version the build cannot describe; such an offer is
   not offered. The Tauri onboarding's optional connect-inference step
   (`consent_copy::inference_connection_copy`) is the first caller.
2. The shell shows one offer and the disclosure its `disclosure_version`
   names, and on the contributor's choice calls `inference_connection_select`
   with **exactly** that offer's `offer_id`, `provider_id`, `revision`,
   `config_digest` and `disclosure_version`, plus `confirmed: true` --
   refused (`bad_params` / `inference-connection-confirmation-required`)
   without it, even with an otherwise well-formed offer. (Before this
   existed, the only place this was ever checked was the Tauri shell's own
   command layer, which refused locally and never forwarded the choice; a
   raw caller had no such floor.) The daemon sends those values
   unchanged (`provider_id` is not sent; it is bound into the digest check
   below) and fills in nothing. Pass `expected_current_version` as the `state_version` from
   `inference_connection_current` when replacing an existing selection
   (including selecting again from a new device); omit it when there is none.
   Pass `idempotency_key` to make a retry of the same choice safe; the daemon
   generates one when it is absent. A response naming any other revision,
   digest or disclosure is refused (`inference-connection-response-invalid`).
   The daemon then recomputes `config_digest` from the witness URL, signing
   address, pins and receipt endpoint the response carries, and `revision`
   from that digest with the offer's `offer_id` and `provider_id`, using the
   protocol crate's `encode_config` / `encode_revision` -- the same functions
   the server publishes digests with. Material that does not hash to the
   chosen digest is refused (`inference-connection-digest-mismatch`) and
   nothing is held. A successful select on this device also removes this
   device's previously installed witness at once (`previous_witness_removed:
   true`), because the server has just revoked that connection.
3. **Selecting installs nothing.** The witness material the server returns is
   held on this device only. The shell then asks the contributor, separately,
   whether to use this witness on this device, and on confirmation calls
   `inference_connection_install` with the `connection_id`, `config_digest`
   from the select result, and `confirmed: true` -- refused
   (`inference-connection-confirmation-required`) without it, same as
   `inference_connection_select` above. The daemon re-checks the held
   material against its digest (`inference-connection-digest-mismatch`), then
   re-reads the account's selection; if it was disconnected or replaced since,
   the answer is `inference-connection-not-current`, and if its revision was
   retired, or the account reports it with another revision, digest or
   disclosure, it is `connection_reselection_required`. Either way nothing is
   written and the held selection is discarded. If this device selected again
   while the install was asking the server, the install answers
   `inference-connection-selection-replaced`, writes nothing, and the newer
   selection stays held. The daemon re-reads the config and its state file
   after the network call and changes only the witness fields, so config
   written meanwhile is kept.
4. Install writes `witness` (`url`, `signing_address`,
   `expected_measurements`, exactly as returned) and, when the selection
   carries one, `inference_receipt_endpoint`. Nothing else in the config
   changes; `admission_evidence` keeps an existing witness's value and is off
   on a first installation. An absent receipt endpoint leaves the configured
   one alone and carries no inference provenance claim. The pins must parse
   and an operator allowlist, when set, must admit the witness host
   (`inference-connection-witness-refused`); a receipt endpoint must pass the
   same checks a published one does at signup
   (`inference-connection-receipt-endpoint-refused`).

**Installing changes the witness, so it voids armed grants.** A new witness is
a new recipient under R6: the watcher's next grant sweep voids every armed
`auto_upload` project and the automatic grant made under the old terms,
writes `auto-upload-voided` / `automatic-grant-voided`, and raises void
notices (see "Void notices"). That is intended. A shell should say so before
the contributor confirms installation, and must not re-arm on their behalf.

**One installed device per account.** A selection is an account record, not
permission for a device to install anything, and the server returns witness
material only in the select response -- `inference_connection_current`
carries identifiers and digests, never a URL or pin. So a device can install
only what it selected itself: on a device that did not select,
`inference_connection_current` reports the selection with
`installed_on_this_device: false` and `pending_install: false`, and
`inference_connection_install` answers `inference-connection-select-required`.
Because every select revokes the account's live connection and mints a new
`connection_id`, **at most one device per account holds an installed
witness, and installing on another device replaces it.** When device B
selects, device A's connection is revoked; A's next
`inference_connection_current` reports `revocation_applied: true`, A's
witness is removed, and -- being a witness change -- A's armed grants are
voided (see above). If A selects again, B loses its witness the same way. A
shell should say so before a contributor selects on a second device. Login,
invite redemption and the capability reads never select.

**A selection grants no other consent.** No folder, trace, raw-session,
consent-scope or standing contribution consent follows from selecting or
installing, and none of these methods changes a project mode.

**A retired revision.** When the operator replaces the selected entry,
`inference_connection_current` reports `selection.reselection_required: true`
and changes nothing on this device; select and install answer
`connection_reselection_required` and install nothing. The current operator
configuration is never substituted for what the contributor chose: the shell
shows the current offers and the contributor chooses again. A witness already
installed from a retired revision stays installed until the contributor
reselects or disconnects, because the connection is still the account's live,
unrevoked selection -- retirement only means the operator now offers
something else -- and removing it unprompted would void the contributor's
grants for a change they did not make; an operator who must stop use of a
witness revokes the connection, which every device applies.

**Disconnect.** `inference_connection_disconnect` first removes from this
device exactly what the installation wrote -- the witness and receipt
endpoint, where the config still holds those values -- and only then asks the
server to revoke the named connection. The local step never waits on the
server: with no account session, an unreachable commons, or a refusal, the
witness is still removed and the call succeeds with `disconnected: false`,
`server_disconnect: "pending"` and `server_refusal` set to the label
(`account-session-required`, `inference-connection-unavailable`, ...); the
shell resolves that (for example by signing in) and calls disconnect again to
finish the server side. `server_disconnect: "revoked"` (with `disconnected:
true`) is a completed revocation, and `"not-found"` means the server holds no
such live connection. A witness configured some other way is left alone
(`local_witness_removed: false`). When the connection this device installed is
found no longer live -- disconnected or replaced from another device, or
reported with a different revision, digest or disclosure --
`inference_connection_current` removes what it wrote the same way and reports
`revocation_applied: true`. New use stops once the revocation is observed; an
offline device keeps its settings until it next asks. Nothing already
disclosed is recalled, and no provider credential is revoked.

Refusal labels, all `unavailable` unless noted: `account-session-required`,
`not-logged-in`, `connection_reselection_required`,
`inference-connection-version-conflict` (the account changed; re-read
`inference_connection_current`), `inference-connection-idempotency-conflict`,
`inference-connection-account-ineligible`,
`inference-connection-offer-invalid` (no such offer, or not its current
revision), `inference-connection-not-found`,
`inference-connection-response-invalid`,
`inference-connection-digest-mismatch`, `inference-connection-unavailable`,
`inference-connection-select-required`,
`inference-connection-selection-replaced`,
`inference-connection-confirmation-mismatch`,
`inference-connection-not-current`, `inference-connection-witness-refused`,
`inference-connection-receipt-endpoint-refused`,
`inference-connection-state-unreadable`,
`inference-connection-state-write-failed`, `config-write-failed`,
`audit-write-failed`. A missing or malformed param is `bad_params` with the
param's name (or the protocol's label, such as `digest_invalid`) as message.

Audit rows, label-only: `inference-connection-selected` (`detail`: the
`offer_id`), `inference-connection-installed` (`detail`: `witness` or
`witness,receipt-endpoint`; written before the config), and
`inference-connection-disconnected` (`detail`: `local-witness-removed` or
`nothing-installed-here`, then `,server-revoked`, `,server-not-found` or
`,server-pending`), plus `inference-connection-revocation-applied` when an
installed witness is removed other than by disconnect (`detail`: `not-live`,
`configuration-changed`, or `replaced-by-selection` for a select on this
device). No URL, signing address, pin, token or
connection id is written. The per-device state (a selection awaiting install,
and what was installed) is in `daemon-inference-connection.json` and is
removed at logout.

## Native network additions: C3 (#1173)

**Contract-only baseline (2026-10-01).** This section specifies additive
interfaces before behavioral implementation. It does not assert that these
methods are registered or that a pilot recording, signed app or deployment
exists. Check `hello.methods` before calling a new method; absence means
unsupported (`unknown_method`), never an empty result. Existing methods and
fields retain their contracts. The tables distinguish a reusable backend from
an implemented daemon method. Implementations may add response fields; callers
must ignore fields they do not recognize.

All JSON blocks in this section are **SAMPLE**, synthetic contract examples,
not pilot observations. C1 may use them only in debug and previews, clearly
marked `// SAMPLE`; release builds never substitute them for unavailable data.
The normal IPC envelope carries the objects below as `params` and `result`.
Dates use RFC3339, UUIDs are strings, counters are nonnegative integers and
unknown amounts are `null`, never zero. Errors use the existing taxonomy with
fixed labels; never pass a remote response body, URL, invite, token or platform
error string into `error.message`.

| Method | Parameters | Reply | Support at contract baseline |
|---|---|---|---|
| `inference_summary` | `since?`: RFC3339; default previous 24 hours | `readable`, `window_hours`, `observed_at`, `summary` | IronWire summary exists; daemon wrapper new |
| `inference_call_proof` | `call_id`: positive integer | `call_id`, `proof`, `checked_at`, `checks`, `readable`, `found` | Stored proof labels exist; method and registration new |
| `model_spend` | none | `known`, `scope`, `source`, `since`, `models`, `reason_label` | Organization-wide provider billing; see the Z4 source contract below |
| `private_ai` | none | `on`, `state`, `port`, `disclosure` | Hosting state and Rust disclosure exist; wrapper new |
| `set_private_ai` | `on`: boolean; `confirmed: true` when enabling | same as `private_ai` | Reuses existing async hosting lifecycle; wrapper new |
| `mission_catalogue` | `limit?`: 1–50, default 20; `before?`: UUID | `kind`, `catalogue`, `disclosure` | Public skill-evaluation catalogue/client exist; IPC new |
| `invite_lookup` | `code`: full invite URL | protocol `InviteLookupResponse` | Issuer lookup/client exist; IPC new |
| Native passkey/account methods | see below | token-free replies below | Server routes exist; daemon and native adapter new |
| `history_rollup` | none | existing optional `community` | Already implemented; authoritative standing surface |

### `inference_summary`

SAMPLE request: `{"since":"2026-09-30T00:00:00Z"}`. SAMPLE result:

```json
{
  "readable": true,
  "window_hours": null,
  "observed_at": "2026-10-01T00:00:00Z",
  "summary": {
    "enabled": true,
    "receipts": true,
    "since": "2026-09-30T00:00:00Z",
    "groups": [{
      "model": "example-model", "backend": "nearai", "route": "routed",
      "work_kind": null, "calls": 3, "priced_calls": 2, "cost_usd": 0.02,
      "proof": {"verified":1,"gateway_only":1,"unattested":0,"pending":1,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}
    }],
    "routed": {"calls":3,"priced_calls":2,"cost_usd":0.02,"proof":{"verified":1,"gateway_only":1,"unattested":0,"pending":1,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}},
    "outside": {"calls":0,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}},
    "unknown": {"calls":0,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}}
  }
}
```

`summary` preserves the Cargo-pinned IronWire `/_ironwire/summary` shape,
including its `(model, backend, route, work_kind)` groups and route totals;
it does not rewrite the upstream DTO into one row per model. The contributor
validates bounds, counter consistency, finite nonnegative prices and safe
labels before exposing it. Arbitrary backend text is not a verified provider
identity. `route` and proof counts come from stored proof statuses; no inference
from the backend's spelling. A future bounded upstream `work_kind` string
contributes to the opaque group identity, but is not displayed: this client
returns `work_kind: null` until classification has an approved display contract. The summary contains no protocol family; do not invent `family`.

`cost_usd` is **registry-priced cost, not billed spend**, including for an
`api_key` backend. `priced_calls < calls` means incomplete pricing. Only
`verified` counts are model proof; `failed` stays distinct. `enabled: false`
means capture is disabled, not an observed empty window. `receipts` reports
the upstream receipts setting, not that every call was verified.

SAMPLE unavailable result:

```json
{"readable":false,"window_hours":24,"observed_at":null,"summary":null}
```

A refused, malformed, stale or unreachable read must not manufacture empty
measured groups. `window_hours` is 24 for the default window and `null` for an
explicit `since`; `summary.since` is the authoritative lower bound. Polling is
bounded, authenticated to the existing loopback control API, content-free and
read-only. It never fetches prompts, responses or receipt bodies.

**C1 migration:** `InferenceSummary.models` / `ModelSummary` in #1175 were
explicitly provisional. Decode this wrapper and the actual upstream DTO;
identify group rows by `group_id`, with all grouping fields as the legacy
fallback, never by `model` alone. A separate
view projection may aggregate per model but must retain incomplete pricing,
route/proof distinctions and the distinction between unknown and empty.
This contract does not authorize editing the teammate's open branch.

### Proof detail and billed spend

SAMPLE `inference_call_proof` request: `{"call_id":412}`; SAMPLE result:

```json
{"call_id":412,"proof":"gateway_only","checked_at":null,"checks":null,"readable":true,"found":true}
```

This is a lookup of the stored `inference_calls` label, not a fresh attestation
check. The ledger records neither a check timestamp nor detailed checks, so
`checked_at` and `checks` are always `null` for this source. For an absent row
return `found:false, proof:"unrecorded"`; for an unreadable ledger also return
`readable:false`. Neither case asserts `outside` or verification. A zero,
negative or noninteger `call_id` is `bad_params` / `call-id-invalid`.

SAMPLE `model_spend` request: `{}`; SAMPLE result:

```json
{"known":false,"since":null,"models":[],"reason_label":"billed-model-spend-unavailable"}
```

This unknown result is returned when authoritative provider billing is
unavailable; it is never a zero balance. The Z4 source contract below defines
known organization-wide provider rows, their window, exact nano-USD amounts
and rounded display amounts. IronWire summary prices, backend kinds,
account-wide NEAR AI balance and the existing harness daily price estimate
cannot be converted into billed-per-model facts.

### Private AI switch

SAMPLE `private_ai` request: `{}`; SAMPLE result:

```json
{
  "on": false, "state": "off", "port": null,
  "disclosure": "While it is on, anything else running on this computer can send calls through it as well, charged to the accounts you have set up here. On a computer only you use that is your own software; on a shared one it is anyone who can log in."
}
```

SAMPLE `set_private_ai` request: `{"on":true,"confirmed":true}`. Both replies
separate the requested setting (`on`, nullable when unreadable) from the actual
`private_inference_state.state` and port documented under `set_settings`.
`stopping`, `running_elsewhere`, `start_failed` and `crashed` are not running.
An unreadable read uses `on:null, state:null, port:null`, not `off`.

The disclosure is the existing Rust `private_inference_copy::OFFER_EXPOSURE`,
returned from the core, not authored by Swift. Enabling requires the person's
explicit answer after the core disclosure; a missing/false `confirmed` is
`bad_params` / `confirmation-required`. Disabling requires `on:false` and
may omit `confirmed`. The wrapper uses the existing async `set_settings`
validation/persistence/reconciliation for `private_inference`. Only confirmed
enabling marks `private_inference_offer_seen`; disabling preserves its prior
value, so writing off cannot suppress an offer that was never shown. The
wrapper does not create another proxy lifecycle.
Enabling does not repoint tools, select a provider, grant body/contribution
consent, or enroll an account. Turning off stops only the owned instance.

Z5 extends existing `tool_destinations` additively; its existing fields retain
their shape. Observed `outside` calls may traverse the owned proxy while their
provider remains unknown. A tool default is still `basis:"tool_default"`, not
observed traffic. An owned-loopback hub uses actual hosting state/port; no
SSH/dev-server discovery or unobserved bypass traffic is claimed. K14 retains
ownership of counts, tool attribution and new-call events.

### Catalogue, standing and invite lookup

SAMPLE `mission_catalogue` request: `{"limit":20}`; SAMPLE result:

```json
{
  "kind": "skill_evaluation",
  "disclosure": "SAMPLE: catalogue disclosure from the Rust core",
  "catalogue": {
    "schema_version": 1,
    "entries": [{
      "mission_id": "00000000-0000-4000-8000-000000000001",
      "program_id": "00000000-0000-4000-8000-000000000002",
      "package_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
      "offer_version_hash": "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
      "task_preview": "SAMPLE: evaluate a reviewed skill package against its controls.",
      "published_at": "2026-10-01T00:00:00Z"
    }],
    "next_cursor": null
  }
}
```

`disclosure` carries the Rust-owned catalogue consent explanation; the SAMPLE
placeholder is not release copy and grants no copy approval. `catalogue` is the unchanged protocol `MissionCatalogPage` from anonymous
`GET /v1/missions`, read through the existing bounded, no-redirect
`MissionCatalogClient`. `before` is the previous page's `next_cursor`;
`package_sha256` is bare lowercase hex and `offer_version_hash` is `sha256:`
plus lowercase hex. `task_preview` is plaintext, at most 240 Unicode scalar
values. This is a catalogue of **skill-evaluation packages**, not daily tasks.
An unavailable fetch is `unavailable` / `mission-catalogue-unavailable`, never
an empty successful catalogue. No profile, local signals, match result or
person-specific ranking goes into this request. Matching stays on the Mac;
matching must not trigger a person-specific fetch. An entry grants no execution,
model spend, capture, sending or contribution permission.

The catalogue currently supplies no daily assignments, completion rewards,
levels, streaks, badges or pending-credit range. The commons record is the
settled ledger of record for pending credit (#1118); dollars remain a near.ai
link-out. Daily mission economics and the credit-to-inference bridge remain
unspecified in #1174/#1118. Build the configurable missions server with rewards
off; do not invent reward arithmetic or a `missions[]` DTO with fabricated
credits. Future pending credit must carry its condition and never be described
as earned. Standing
uses the existing `history_rollup.community` unchanged: absent object means no
standing, nullable rank means unknown, and no percentile is implied by rank.

SAMPLE `invite_lookup` request:
`{"code":"https://issuer.example/onboard#SAMPLE-CODE"}`; SAMPLE result:

```json
{"valid":true,"issuer_display_name":"SAMPLE Pilot","credit_range":{"min":1,"max":5,"unit":"points_per_accepted_trace"}}
```

Despite C1's argument name `code`, it carries a **full invite URL**, parsed by
the existing `commands::parse_invite`; a bare code has no issuer and is refused.
Use the configured host allowlist (`allowlist_for(None)` when unenrolled),
never a caller-supplied host override. HTTPS is required except for literal
loopback IP addresses; a hostname such as `localhost` does not grant that
exception. The issuer request is the existing
`POST /v1/invite/lookup` with
`{schema_version:"trace_commons.invite_lookup_request.v1",invite_code}` in its
body, never its URL. This read neither consumes an invite nor enrolls/writes
identity. On invalid invite return the protocol's fixed
`reason_label: malformed|not_found|expired|exhausted|revoked`. Success metadata
may be absent. Transport failure is unavailable, not an invalid invite.

Credit is an operator-set **estimated credit per accepted trace, not yet
settled**, never a payment promise. C1's provisional `CreditRange.unit` comment
`points` must change to the real `points_per_accepted_trace`; lookup credit is
not daily mission credit. Perform lookup for an explicit action, not on every
keystroke. Display-name/range bounds must be validated before presentation.

### Native passkey and account identity

The C1 names in the approved [native passkey design](superpowers/specs/2026-09-28-native-passkey-identity-design.md)
are authoritative. That design remains on the #1120 branch at this contract
baseline; its proposed screen copy is **not approved** by this contract.
The native app uses only Apple's platform passkey provider, while one Rust
encoder creates the WebAuthn JSON. All raw byte fields below use canonical,
unpadded base64url. The app receives no native bearer, NEAR AI token, device
key, tenant identity or secret-store reference.

| Method | Parameters | Token-free result |
|---|---|---|
| `passkey_create_begin` | `label?`, `ingest_url?` | registration options below |
| `passkey_create_complete` | `ceremony`, `credential_id`, `raw_client_data_json`, `raw_attestation_object` | `{binding_state}` |
| `passkey_login_begin` | `ingest_url?` | assertion options below |
| `passkey_login_complete` | `ceremony`, `credential_id`, `raw_client_data_json`, `raw_authenticator_data`, `signature`, `user_handle` | `{binding_state}` |
| `passkey_add_begin` | `label?` | registration options below |
| `passkey_add_complete` | same fields as create complete | `{binding_state}` |
| `passkey_cancel` | `ceremony` | `{cancelled:true}` |
| `account_bind` | none | `{outcome,binding_state}` |
| `account_binding` | none | `{binding_state}` |
| `passkey_state` | none | `{state,passkey_count,near_ai_connected}` |
| `account_sign_in` | `ingest_url?` | `{signed_in:true,expires_at}` |
| `account_session_status` | none | `{state,signed_in,expires_at}` |
| `account_sign_out` | none | `{signed_out:true}` |

`label` is optional, at most 64 characters. A no-config create/login may name
an `ingest_url` only when its canonical origin equals the compiled production
API origin `https://ingest.tracecommons.ai`; omitting it selects that origin.
A retained unconfigured session must satisfy the same trust root. Explicit
operator configuration retains its host policy. Once a config or pre-enrollment
session pins an origin, a caller cannot substitute another one. The native
Swift adapter exposes no caller-supplied ingest URL. Begin stores the server ceremony privately and returns
an opaque local `ceremony` handle pinned to action, origin, expiry and current
credential/config lifecycle. Adding a passkey reloads the current token and
permits ordinary rotation only within the same lifecycle and account. Complete
consumes the ceremony once; cancel invalidates it.
Sign-out, wipe or identity/config replacement invalidates stale finishes.
Complete accepts no origin, account or tenant parameters.

SAMPLE create/add begin result:

```json
{
  "ceremony": "SAMPLE-local-handle", "rp_id": "tracecommons.ai",
  "challenge": "AQID", "user_id": "BAUG", "user_name": "SAMPLE passkey",
  "user_display_name": "SAMPLE passkey", "attestation": "none",
  "expires_in_secs": 180, "exclude_credentials": [],
  "user_verification": "preferred", "authenticator_attachment": "platform",
  "resident_key": "preferred", "algorithms": [-7]
}
```

SAMPLE login begin result:

```json
{"ceremony":"SAMPLE-local-handle","rp_id":"tracecommons.ai","challenge":"AQID","expires_in_secs":180,"user_verification":"preferred","allowed_credentials":[]}
```

Verification/resident-key options are the validated server values
`required|preferred|discouraged`, not permission for the app to downgrade them.
`exclude_credentials` and login's `allowed_credentials` contain canonical
credential IDs, with at most 128 entries per list. An empty login list selects
a discoverable credential. `user_display_name` is the validated server display
name, falling back to `user_name`; `attestation` preserves the validated server
preference `none|direct|indirect`, defaulting to `none`. These three fields are
additive to the original C3 begin options. Creation uses the platform provider
and algorithms the adapter actually supports (ES256 `-7`).
Refuse unsupported server options rather than silently weakening them.

SAMPLE create/add complete request:

```json
{"ceremony":"SAMPLE-local-handle","credential_id":"AQID","raw_client_data_json":"BAUG","raw_attestation_object":"BwgJ"}
```

SAMPLE login complete request:

```json
{"ceremony":"SAMPLE-local-handle","credential_id":"AQID","raw_client_data_json":"BAUG","raw_authenticator_data":"BwgJ","signature":"CgsM","user_handle":"DQ4P"}
```

These bytes illustrate encoding only and are not valid credentials. SAMPLE
complete result: `{"binding_state":"unbound"}`. Binding read values are
`unbound|bound|closed|legacy`; `legacy` means no passkey-origin binding row,
not proof of a NEAR AI connection. A signed-out binding read is unavailable,
not `unbound`. `passkey_state.state` projects those labels, or `none` when
signed out and `unknown` when unreadable. `passkey_count` is nullable until
an authenticated source answers; `near_ai_connected` is nullable unless an
authenticated identity fact establishes it. Never infer a count of zero or
connected status from a missing binding row.

Native login is discoverable authentication: `user_handle` must contain the
nonempty raw Apple user ID encoded as canonical base64url. A missing or empty
handle is an incomplete credential and must not reach login completion.

SAMPLE account status:

```json
{"state":"known","signed_in":true,"expires_at":"2026-10-02T00:00:00Z"}
```

Signed-out status has `state:"known", signed_in:false` and null expiry;
unreadable OS storage has `state:"unknown", signed_in:null` and null
expiry. Raw account identifiers do not cross this status surface. It must not become signed out merely because Keychain refused
a read. Status and sign-out support a pre-enrollment account without creating a
fake config. `account_sign_in` reuses existing browser/PKCE machinery where
its enrollment prerequisites hold; without real enrollment configuration it
refuses with `unavailable` / `account-enrollment-required`. An unenrolled
account uses the native passkey create/login path. Browser URL exposure is bounded to the live sign-in attempt and
uses the existing URL validation/opening controls.

`account_bind` reuses retained NEAR AI provisioning against the authenticated
bind start/finish routes and the **bind-specific** proof preimage. SAMPLE
result: `{"outcome":"bound","binding_state":"bound"}`. An
`existing_account` outcome switches to the existing account and closes the
unbound account; its passkey is not transferred (S6 fold is deferred).

For a signed-in account whose `binding_state` is `bound` (another Mac bound
it, and this Mac signed in with the same passkey), `account_bind` **enrols
this Mac into that account** through the same routes: SAMPLE result
`{"outcome":"enrolled","binding_state":"bound"}`. The commons compares the
near.ai login this Mac proves with the account the passkey session is signed
in to -- the session's own account, never a request field -- inside the one
transaction that would write the device key, and refuses before writing
anything when they differ. That refusal is `unavailable` /
`account-enrol-mismatch` (server: `409 {"error":"near_ai_account_mismatch"}`),
one fixed label whether the login owns another account or none; the passkey
session is left as it was, and the shell signs it out. A `bound` or
`enrolled` result is persisted only if its tenant and account are the passkey
session's (`account-enrollment-mismatch` otherwise). `legacy` (or any other
state) is refused with `account-bind-refused` before the refresh token is
spent. Tokens,
rotations and atomic device/config persistence stay in Rust/Keychain. Binding
creates no folder/trace/body consent. Native login remains weak and native add
keeps the existing first-strong-authenticator gate; adding another requires
browser passkey step-up.

The Associated Domains entitlement, approved profile bytes, deployed AASA and
signed create/login origin check are **release gates**. Local/mock tests do
not establish Apple origin behavior or satisfy the spec's S3 deployment gate.
This contract approves no new screen copy, deployment, profile fabrication or
relaxation of origin/CORS/CSP controls.

## Events

| Event | When | Data |
|---|---|---|
| `snapshot` | immediately after `subscribe` | `{pending[], status}` |
| `queue_changed` | queue contents changed | `{}` |
| `status_changed` | pause/resume, a lapsed timed pause, or health changed | `{}` |
| `digest_due` | batching interval elapsed with pending work | `{pending, text}` |
| `resync_required` | this client fell behind the event buffer | `{}` |
| `preview_ready` | a scheduled preview finished and was delivered | the same object `preview_request` returns for a cache hit -- see "Scheduled previews" |
| `inference_call_added` | the poll tick read a call from IronWire's log that no earlier tick had (K14) | `{id, tool, model, proof}` -- see below |
| `managed_changed` | a saved model account or managed session changed (see `docs/managed-sessions.md`) | `{revision}` |

`inference_call_added` is published where the daemon already reads IronWire's
`/log`: the poll tick's routing refresh. Nothing is fetched for it and it adds
no poll of its own, so a call is announced on the first tick after it lands
(the daemon's poll interval). Its data is label-only and is the same labels
`inference_calls` puts on that row: `id` (the ledger row id), `tool`,
`model` (the same free-text rule) and `proof` (IronWire's label, or
`unrecorded`). No body, URL, endpoint, session id, digest or token.

- Once per call. New is tracked per ledger by the highest row id announced.
- The first window a ledger reads is the backlog and is not announced; it
  sets the baseline. A ledger rebuilt for a new endpoint starts over the
  same way, and so does one whose ids went backwards (a proxy whose own
  ledger started over). A ledger that started over and climbed past the old
  highest id within one tick cannot be told from one that only grew: its
  rows above the old id are announced, and the rest are never announced.
  Counts on `tool_destinations` are unaffected.
- Rows without an id (a proxy too old to expose one) are never announced.
- At most 64 per tick, the newest kept, so a burst cannot push a subscriber
  into `resync_required`. Re-read `tool_destinations` for exact counts.

`subscribe` sends a full `snapshot` before any delta, so a client never has to
race `list_pending` against the stream at startup. On `resync_required`, call
`list_pending` and `status` again.

## Queue states

`pending`, `approved`, `uploading`, `uploaded`, `refused`, `failed`,
`expired`, `superseded`.

- `expired` means the entry aged out after the TTL (14 days) without a decision. The clock
  is **suspended** while the daemon is unhealthy, so an outage does not
  silently discard traces.
- A session kept on this Mac is `refused` with `reason_label:
  "kept-on-this-mac"`. It never expires, and `undo_keep` returns it to
  `pending`; see "`keep`: Keep on this Mac" above. It is the only `refused`
  entry a contributor can undo.
- `superseded` means the session changed after it was offered. The daemon re-hashes
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
- `approved` entries can be returned to `pending` with `cancel`, which
  clears the pin along with the rest of the approval, so a re-offered entry
  is previewed or rebuilt afresh rather than carrying the withdrawn
  approval's artifact. An entry approved through `approve` stays untouched
  for its hold window first (see "The approval hold" above), so `cancel` is
  guaranteed to succeed for that whole window. After it, `cancel` still works right up until the upload
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
| `witness-saturated` | approved sessions held because the privacy witness is at capacity; also the `reason_label` on each held entry | yes |
| `session-too-large` | at least one session on disk is past the byte budget its source will read, so it is not being offered | no |
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
8. `witness-saturated`
9. `automatic-contribution-held`
10. `admission-limit-reached`
11. `daily-cap-reached`
12. `session-too-large`

`session-too-large` sits last on purpose: every label above it describes the
daemon, and this one describes a single file on disk. It must never mask an
outage. It is raised by the poll pass when a source declines to read a
session at all, and retracted by the next full pass that reads everything --
a scoped pass over a handful of changed paths may raise it and never clears
it, since one readable session says nothing about the rest of the corpus.
A read that merely *failed* -- a file deleted mid-pass, a momentary
permission or IO error -- does not raise it: that is expected to succeed on
the next poll, and a flag a blip pins on a healthy daemon is one nobody will
trust.

Because `daily-cap-reached` is last, it is the label most often masked by
another condition. That is why the daily caps are also reported in full on
`status.daily_budget`, outside the precedence order described above. A client
should render the budget condition from `daily_budget`, not by waiting for
`daily-cap-reached` to reach the health slot.

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

The harness methods add `id-required`, `action-invalid`, `harness-unknown`,
`harness-no-destination` and `plan-id-invalid` (all `bad_params`), and
`harness-plan-unknown`, `harness-config-changed` and `harness-commit-failed`
(all `unavailable`). A refusal that is a fact about the machine rather than
about the call -- an unparseable config, a tool that is not installed -- is
**not** an error: it is a `harness_plan` `outcome`. See "The harness list".

`not_authorized` is retained in the error taxonomy for forward
compatibility but is no longer returned by any method in this version --
the `v1` terminal-only gate that used it was removed (see "Authorization"
above).

### Token distribution contribution

`token_distributions_contribution` is a boolean, default `false`. It grants
permission to include token probabilities and alternatives in explicit witness
reviews. It does not enable Ironwire capture. Capture must be configured for an
exact supported backend/model pair, and `ironwire_attested_bodies` must also be
allowed before raw evidence goes to the configured, pinned witness.

When enabled, review requires an exact capture digest match, a bounded snapshot
lease, a verified provider receipt, and an ingest endpoint advertising the
restricted bundle policy. Missing support is a refusal, never a transcript-only
fallback. Changing this setting invalidates incompatible queued approvals.

The certified manifest and payload are pinned locally before approval. Upload
reuses those exact bytes and resumes missing objects. Only a destination-bound
durable receipt authorizes successful-submission cleanup. A separate three-day unapproved (seven-day approved)
review expiry abandons old local payloads and releases their own leases; it does
not imply successful submission. Logout deletes local token review state. Agent
session files are never deleted by this mechanism.

### Local token storage

`token_capture_enabled` is an optional boolean override for the hosted Ironwire
proxy. `false` stops future token captures; `true` requires explicitly configured
backend/model targets. An absent override honors the proxy configuration. This
setting does not authorize contribution or sending raw bodies to the witness.
Changing it cycles the hosted proxy. External proxies keep their own settings.

| Method | Parameters | Result |
| --- | --- | --- |
| `token_storage_status` | none | Content-free local byte counts, pending releases, review counts and lease renewal status |
| `remove_token_local_copies` | none | Updated status after removing submitted local payloads |
| `discard_token_reviews` | `confirmed: true` | Updated status after discarding unsubmitted token reviews; approved/uploading reviews require undoing approval first |

The CLI exposes these through `daemon token-storage`, `--cleanup`, and
`--discard --confirm`. Capture intent can be set with
`daemon settings --set token_capture_enabled=true` after configuring qualified
targets. Lease expiry is reported only when confirmed by the capture store.
Failed renewals and releases remain durable and retry with bounded backoff.
Server withdrawal is a separate action; local cleanup never deletes original
agent session files or another destination's capture lease.

Single-submission `withdraw` also returns an optional `token_deletion_note` from
shared Rust copy. It distinguishes pending deletion, a retention hold, completed
server-managed deletion and an unknown result. This does not claim deletion from
backups or previously distributed copies. Older servers omit the field.

## Moving a legacy invite identity

A daemon enrolled with a legacy `tenant-…` invite can move to the
contributor's NEAR AI account, only when the contributor asks
(`legacy_invite_migrate`). `status.legacy_invite_migration.offered` is true
while the config is a legacy invite identity enrolled without an instance
and no move has happened. Nothing moves otherwise: coexistence is the
default, and a pooled (shared event) invite is refused by the server with
`legacy_migration_tenant_pooled` and keeps working.

The daemon, in order: stages a second device key; provisions it into the
NEAR AI account with the retained NEAR AI login; requires an affirmative,
`ready` `/v1/account/contribution-status` for that account; asks ingest for a
challenge, signs the #1066 statement with the legacy device key, and
verifies the countersigned record against its own legacy key and the account
it signed into; then, under the watcher's pass lock, writes the new config,
promotes the staged key and stores the new session together, re-records
every armed folder's grant with the new identity term and nothing else, and
retires the legacy key last. Once committed, it revokes the legacy account
session on the server (`POST /v1/account/logout` with that session;
best effort, audited as `legacy-account-session-revoked` or
`-revocation-failed`, and reported as `legacy_session_revoked`). A failure
at any step leaves the legacy
identity as it was; a daemon that dies mid-switch is rolled back, or
finished once committed, at its next start. There is no IPC method for the
re-baseline itself.

The invite is named by its subject hash: from `invite` when given, else the
hash saved at invite enrollment, else the issuer
(`POST /v1/device/invite-subject`, signed by the legacy key). When none can
say, the answer is `legacy_migration_invite_needed` and a shell asks for the
invite link.

Queued sessions already approved return to pending after the move, because
their approval was bound to the old identity; armed folders re-approve them
on the next pass. The move is audited label-only (`auto-upload-rebaselined`
per folder, `legacy-invite-migrated`), and
`status.legacy_invite_migration.notice` stays until
`acknowledge_legacy_invite_migration`. The words are
`consent_copy::legacy_migration_*`. Every shell shows the notice: Tauri
through its copy commands, macOS and Windows through
`tc_legacy_migration_notice`, GTK directly; each passes the `notice` object
through unread and acknowledges with `acknowledge_legacy_invite_migration`.

## Zaki network addendum: provider model spend and summary identity (#1173)

This addendum supersedes the initial `model_spend` unavailable-only contract.
`model_spend {}` is asynchronous and uses the existing connected NEAR AI session,
selected inference-key organization (or the existing first-active-organization
fallback for a legacy session), and the fixed trusted management origin
`https://cloud-api.near.ai`. It requests
`GET /v1/organizations/{organization}/usage/by-model?period=day`. The deployed
[provider OpenAPI](https://cloud-api.near.ai/api-docs/openapi.json) defines this
session-authenticated rolling 24-hour report. No registry price estimate is
converted into an actual cost, and no additional reporting token is minted.

A known reply has the following fields; all metadata shown is mandatory:

```json
{"known":true,"scope":"near_ai_organization","source":"near_ai_usage_by_model","currency":"USD","scale":9,"window_hours":24,"since":"2026-10-01T12:00:00Z","observed_at":"2026-10-02T12:00:00Z","models":[{"model":"example-model","billed_nanos":1501,"billed_micros":2,"rounding":"nearest_micro_half_up","calls":3}]}
```

This is a SAMPLE, not a release fallback. `since` is the provider's `start_date`,
validated as an RFC3339 UTC timestamp no later than the daemon observation time;
`observed_at` is the daemon's completion time. `billed_nanos` retains the exact
provider usage cost in nanoUSD (the official [usage source](https://github.com/nearai/cloud-api/blob/88989203c8172fd19c46c2b0082c71b024925f49/crates/api/src/routes/usage.rs#L1624) specifies scale 9). `billed_micros` is a compatibility amount rounded
to the nearest microUSD, half upward; `rounding` makes that conversion explicit.
Counts and native costs must be nonnegative signed 64-bit integers. Model labels
use the existing content-free label sanitizer. Display model labels are not
guaranteed unique after sanitization: consumers must retain every row and use
row position as identity when needed, rather than grouping by the display label. Provider display prose, organization
identifiers, access tokens, inference credentials, and refresh tokens are omitted.

The scope is the connected NEAR AI organization's usage across its callers. It is
not a sum attributed to this machine, its local proxy, a tool, or its local ledger;
it is not a payment, invoice, settlement, or cryptographic receipt. Empty provider
`data` is a readable known report with `models: []`, rather than an unavailable
report. Readable zero-cost rows retain their zero.

No session, expired session, no organization, unavailable transport, or malformed
source data returns `known:false`, `since:null`, `observed_at:null`, `models:[]`,
`reason_label:"billed-model-spend-unavailable"`, and `state` equal to respectively
`no_session`, `session_expired`, `no_organization`, or `unavailable`. Scope, source,
currency, scale, and window remain present with the values above. Unknown never
means known zero and never serves the last successful amount after a failed read.
The balance and model-spend reads share the existing session rotation lock,
conservative access-token reuse, single 401 retry, and stored-connection completion
check. A forgotten or replaced account cannot receive an in-flight old account's
cost report. Successful readings have the same 60-second cache policy as balance.
The synchronous dispatcher returns `model-spend-requires-async`; socket and local
CLI callers use the asynchronous dispatcher.

Each newly returned `inference_summary.summary.groups` row also carries mandatory
`group_id`, formatted `sha256:<64 lowercase hexadecimal digits>`. This is SHA-256
of the canonical JSON array `[original_model, original_backend, original_route,
original_work_kind]` before content-free label normalization. It is an opaque row
identity, not a verified provider identity or an anonymization guarantee.
These unsalted digests can be guessed from a dictionary of likely inputs. Backend labels continue to retain only
the vetted `nearai` label or an opaque digest. Original model labels that normalize
to the same display value therefore remain separate rows with separate stable IDs;
no group counts, costs, or proof totals are merged or lost. Older daemons may omit
`group_id`; clients may retain their existing tuple fallback for those versions.
The summary wrapper preserves the validated upstream groups and totals, with these
explicit identity and privacy normalizations.

## Configured activity missions (Z7)

`activity_missions_catalogue` and `activity_missions_status` each accept only
an empty params object `{}`. Both are async methods advertised in `hello`.
Unknown fields, including profile, matching results, session/account identifiers,
URLs and pagination, are `bad_params` / `activity-missions-params-invalid` before
credentials or HTTP are read. This is separate from the `mission_catalogue`
skill-evaluation domain.

`activity_missions_catalogue` returns
`{"catalogue":<ActivityCatalogue>,"disclosure":<Rust-owned string>}`. Its inner
schema-v1 protocol DTO comes from anonymous `GET /v1/activity-missions` at the
stored ingest origin; an upload endpoint such as `/v1/traces` contributes only
its origin. The complete bounded common policy is fetched independently of local
matching. `kind` is `trace_activity`; `state: unconfigured` with null policy and
digest is a real fetched state, not a fallback for a failed request. A configured
policy and its canonical SHA-256 digest must agree. No account/device credential,
profile or matching result goes to the public request.

`activity_missions_status` returns
`{"status":<ActivityProgress>,"disclosure":<Rust-owned string>}` from authenticated
`GET /v1/account/activity-missions/status`, using the stored native account
session. It accepts no caller-supplied principal or completion assertion. The
unchanged DTO includes authoritative contribution qualification/source, UTC
coverage/month dates, observation time, nullable daily/streak/level/badge fields
and the policy digest. Unconfigured daily rules, levels and badges retain their
null semantics; they do not become zero progress or invented defaults. Progress
may decrease after withdrawal/revocation and is a current projection, not an
irrevocable award. Account-token rotation is retained on success and refusal;
rotation persistence failure refuses the result. Account/config snapshot checks
before sending and after HTTP also refuse late progress from an account that
signed out or changed while the request was in flight.

Response envelopes and progress rows tolerate additive unknown fields and
omit them from IPC. Known schema versions, policy digests, reward flags and
credit conditions remain enforced. Policy and nested rule objects remain strict
and digest-covered; changing their shape requires a supported schema-version
change rather than an unversioned extension.

Both replies carry `consent_copy::ACTIVITY_MISSIONS_DISCLOSURE` assembled in Rust.
Neither read changes capture, contribution consent, project modes, scopes,
approvals or uploads. Status can update only a rotated account credential.
Rewards are hard-disabled: `rewards_enabled` is false,
`credit_points_pending` is null, and `credit_condition` is
`mission_credit_ledger_unavailable`. A response claiming activated rewards or
mission credit is refused. Corpus credit and skill-evaluation awards remain
separate.

Responses are limited to 128 KiB, HTTP is allowed only on literal loopback IPs,
and configured host allowlists apply. Proxies and redirects are disabled. Safe
errors never include URLs or remote bodies: `unavailable` /
`activity-missions-unavailable` for config, host, transport, status, bounds or
response-validation failure; `unavailable` / `account-session-required` for no
live native account; `unavailable` / `commons_credential_storage_unavailable`
for credential read or rotation persistence failure. Unavailable is never an
empty catalogue or a zero progress response.

See `docs/superpowers/specs/2026-10-02-configured-activity-missions-design.md`
for the operator policy and qualification rules. The policy is optional; this
adapter does not choose thresholds or activate economics.

## Explicit account contribution controls

`status` includes nullable `account_scope`, an opaque local account lifecycle and
configuration identifier. It changes on sign-in/replacement (including a new
sign-in to the same account), logout, or configuration changes, and stays stable
through token rotation. It contains no account name, token, or raw credential.
Existing opaque credential records remain usable. The daemon publishes
`status_changed` when this scope changes, including without armed folders; this
local notification does not request contribution status from the server.

`account_contribution_status` accepts optional `account_scope` and fetches fresh
authenticated contribution readiness even when no automatic folder is armed.
When supplied, the scope must still match the local session/configuration before
the request. Its result contains `status` (`authority`, `policy_version`, `ready`,
`refusal_label`, and `retry_after_seconds`), the shared user-facing `line`, and
`account_scope`. It does not report a numeric remaining allowance. Shells clear
cached readiness and invite attempts when the observed scope changes and discard
responses whose scope does not match the current status.

`account_invite_redeem` takes `invite_code`, a UUID `idempotency_key`, and optional
`account_scope`. Keep the same key when retrying the same code in the same scope
after an uncertain transport outcome. On success the daemon refreshes contribution
status and returns the same status shape. Redemption and its refresh share one
operation scope: token rotation is accepted, while account lifecycle or
configuration changes between POST and GET refuse the combined result. Both
methods require an account session and discard responses when the local account
or ingest configuration changes. Neither returns an invite code or session token.
Acceptance can earn pending credit; this is not credit redemption.
