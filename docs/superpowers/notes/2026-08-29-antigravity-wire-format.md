> **SUPERSEDED on 2026-08-31** by the `import-antigravity` command.
>
> This note documents the on-disk SQLite/protobuf format read by the
> abandoned file-reading approach (see
> `docs/superpowers/specs/2026-08-29-antigravity-source-design.md` for why
> it was dropped). Nothing in the tree decodes this format now: the shipped
> command reads the running IDE's local API, whose JSON shape is captured by
> the fixtures under
> `crates/trace-commons-contributor/tests/fixtures/antigravity/`. The
> `conversation.db` fixture referenced below was deleted with the code that
> read it.
>
> Kept as the written record of the format, which remains the only one.

# Antigravity conversation wire format

Derived by decoding one real capture
(`~/.gemini/antigravity-ide/conversations/39f32a85-508b-430a-98fb-a67e89b4e689.db`,
committed in redacted form as
`crates/trace-commons-contributor/tests/fixtures/antigravity/conversation.db`)
with `protoc --decode_raw` against the `steps.step_payload` and
`trajectory_metadata_blob.data` columns.

**These field numbers come from one capture on one Antigravity build. This is
a vendor's unpublished, undocumented schema, not a spec. It will change
between builds. Any reader of this wire format must skip unknown fields
rather than reject them, precisely because these numbers are not stable.**

## 1. Step types

`steps.step_type` is also stored as a plain integer column, and duplicated as
field `1` of `steps.step_payload`. Every value observed in the capture
(`idx` = row in the `steps` table):

| step_type | Meaning | Observed at idx |
|---|---|---|
| `8` | Tool RESULT. Carries the tool-call submessage plus the tool's output. | 7, 11, 21 |
| `9` | Tool RESULT, same role as `8`. The 8/9 split tracks the tool family, not call-vs-result: `8` accompanies `view_file`, `9` accompanies `list_dir`. | 4, 13, 15, 17, 19 |
| `14` | Conversation start. Ids and a status timeline only. | 0 |
| `15` | Model turn. Carries the tool call on non-final turns; on the final turn carries the assistant text and reasoning instead. | 3, 6, 8, 10, 12, 14, 16, 18, 20, 22 |
| `23` | Title. | 5 |
| `98` | Small, ids only. | 1 |
| `99` | Small, ids only. | 2 |

## 2. Step payload fields

`steps.step_payload` is a protobuf message. Two fields are constant across
every step type observed:

- Field `1`: the step type (matches the `step_type` column).
- Field `4`: a small integer status code (`3` on every step observed in this
  capture; not otherwise decoded).

**The field number that holds the type-specific body differs by step type.**
This capture does not support a single "field 5 is always the body" rule —
that held for `step_type` 8 and 9 but not for 15 or 23:

- **`step_type` 8 and 9** (tool result / tool execution): the body is field
  `5`, a submessage. Within it:
  - `4`: the tool-call submessage — `1` = call id (e.g. `call_304828`),
    `2` = tool name (e.g. `list_dir`), `3` = arguments as a JSON string,
    `9` = tool name again, `7` = an opaque encrypted submessage.
  - `5`: repeated string, the argument key names (e.g. `DirectoryPath`,
    `toolAction`, `toolSummary`) — one entry per key in the field-`3` JSON.
  - `1`, `6`, `7`, `8`: timestamps (`{1: unix_seconds, 2: nanos}`).
  - `20`: an ids submessage (conversation id, per-step sequence number,
    trajectory id).
  - Observed at idx 4, 7, 9, 11, 13, 15, 17, 19, 21.

- **`step_type` 15** (model turn): field `5` is present but here carries
  *only* timestamps and status/ids (subfields `1`, `3`, `6`, `7`, `8`, `9`,
  `11`, `12`, `13`, `20`, `21`, `26`, `32`) — no tool call or text. The actual
  body is a separate top-level field **`20`**:
  - On a tool-call turn (idx 3): `20.6` = bot id, `20.7` = the tool-call
    submessage, same shape as above (`1` = call id, `2` = tool name,
    `3` = arguments JSON, `9` = tool name again), `20.12` = `2`.
  - On the final turn (idx 22): `20.1` = assistant text (markdown),
    `20.3` = reasoning/thinking text, `20.6` = bot id, `20.8` = rendered
    text (observed byte-identical to `20.1` in this capture), `20.14` = an
    opaque encrypted submessage.

- **`step_type` 23** (title): the body is top-level field `30`. `30.4` is
  the title string (e.g. `"Repository Overview Request"`); `30.19` echoes the
  original user request text; `30.15` is a `file://` URI to a transcript log
  on disk — do not read or ingest that path, it points outside the capture.

- **`step_type` 14, 98, 99**: field `5` (or, for 98/99, an unstructured tail
  field) carries only timestamps and the ids submessage (`12` = trajectory
  id string, `20` = `{1, 4}` = conversation id / trajectory id). No body
  text or tool call.

Takeaway for implementers: dispatch on `step_type` first, then read the
tool-call / text fields from the field number that type actually uses (`5`
for 8/9, `20` for 15, `30` for 23) — do not assume a fixed "body field"
across step types.

**And do not read the tool-call submessage as evidence of a CALL.** It
appears in both halves: `step_type` 15 carries the call (arguments, no
output), and 8/9 carry the result (the same call id and arguments, plus the
tool's output). Verified across the whole fixture — every one of the nine
calls is a 15 immediately followed by an 8 or a 9 sharing its `call_id`,
and only the 8/9 half ever carries output:

```
idx=3  type=15 call_304828 list_dir   output=no    idx=4  type=9 call_304828 output=yes
idx=6  type=15 call_428501 view_file  output=no    idx=7  type=8 call_428501 output=yes
idx=8  type=15 call_307594 view_file  output=no    idx=9  type=8 call_307594 output=yes
```

An earlier draft of this note described `9` as "tool execution", which read
as though it were the call. Mapping 9 to a `ToolCall` leaves five of the
nine calls in this capture unanswered and breaks call/result pairing.

## 3. Metadata blob fields (`trajectory_metadata_blob.data`)

Single row, `id = "main"`. Observed submessage:

- `1`: workspace submessage —
  - `1`, `2`: workspace URI as a `file://` string (both observed
    byte-identical in this capture).
  - `3`: submessage — `1` = `owner/repo` (e.g. `TraceCommons/trace-commons`),
    `2` = remote URL (e.g. `https://github.com/TraceCommons/trace-commons.git`).
  - `4`: branch name.
- `2`: a timestamp (`{1: unix_seconds, 2: nanos}`).
- `3`, `6`: uuid-shaped strings (trajectory id, conversation id).
- `7`: the workspace `file://` URI again.
- `15`: an opaque binary blob, not decoded further here.

## Where the user turn actually lives

User turns are **not** a `steps` row. The user's literal request text is
wrapped as `<USER_REQUEST>...</USER_REQUEST>` inside the serialized model
input stored in `gen_metadata.data` (the same table whose row 9 is redacted
in the committed fixture). A given `gen_metadata` blob can contain more than
one occurrence of the literal substring `<USER_REQUEST>` — the system prompt
in this capture describes the tag in prose before the wrapped turn ever
appears, and the same description recurs later in the blob. A naive
`bytes.index(b"<USER_REQUEST>")` finds that *description*, not the wrapped
turn, and a start/end pair taken from mismatched occurrences pulls in
everything between them — in this capture, the entire vendor system prompt
and tool schema listing. The wrapped turn is the pair where the very next
`</USER_REQUEST>` after a `<USER_REQUEST>` closes almost immediately after
it (tens of bytes later, not tens of thousands): find the closing tag first,
then the last `<USER_REQUEST>` at or before it.
