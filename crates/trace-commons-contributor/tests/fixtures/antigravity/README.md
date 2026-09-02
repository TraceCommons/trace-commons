# Antigravity API fixtures

These are real HTTP responses captured from a live Antigravity instance on
macOS on 2026-08-31, via the IDE's local `GetCascadeTrajectorySteps` and
`GetAllCascadeTrajectories` API calls. They are not hand-authored.

`GetUserTrajectoryDescriptions` was tried first and abandoned: it lists a
different concept ("user trajectories"), and neither its `trajectoryId` nor
any value derived from it can fetch steps under any field name --
`GetCascadeTrajectorySteps` always answers "trajectory not found" for it,
indistinguishable from an empty request. `GetAllCascadeTrajectories` is the
correct listing call; it is keyed by cascade id, the identifier
`GetCascadeTrajectorySteps` actually takes.

## Files

- `steps-single-turn.json` — `GetCascadeTrajectorySteps` response for a
  conversation with one user turn. 23 steps, 1
  `CORTEX_STEP_TYPE_USER_INPUT` step.
- `steps-multi-turn.json` — `GetCascadeTrajectorySteps` response for the
  **same conversation** as `steps-single-turn.json`, captured again after a
  second user turn was added. 48 steps, 2 `CORTEX_STEP_TYPE_USER_INPUT`
  steps. The two files are paired deliberately: tests that need to prove a
  conversation which gains a turn produces different staged output rely on
  this exact relationship. Do not replace either file with an unrelated
  capture.
- `listing.json` — `GetAllCascadeTrajectories` response, a map keyed by
  cascade id. Its one entry, key `39f32a85-508b-430a-98fb-a67e89b4e689`, is
  the **same conversation** as both step fixtures above (its `trajectoryId`
  is `f1422752-2ec0-45ad-a5cc-0068a6b2ffd7`, and its `stepCount` of 48
  matches the multi-turn capture's post-second-turn state). Unlike the
  retired `descriptions.json` fixture this replaces, the listing and the
  step fixtures deliberately DO cross-reference: a test resolves a
  conversation through the listing, takes its cascade id, and fetches that
  cascade's steps, exactly as the real list-then-fetch flow does.

## Modifications from the raw capture

Two kinds of change were made to the raw API responses before they were
committed:

1. The operator's local username (`zakimanian`) was replaced with
   `anonymized` via a raw-text `str.replace` over the whole file, before
   `json.loads`. This is broader than a JSON-field-scoped substitution (it
   also catches the username anywhere it appears outside a value a JSON
   walk would otherwise reach, e.g. inside an already-escaped nested JSON
   string), so it could not under-redact; it can only over-match literal
   occurrences of the substring `zakimanian`, which does not collide with
   anything else in these files.
2. Every vendor identifier is redacted by a **rule**, not a list of field
   names, enforced by
   `crates/trace-commons-contributor/tests/antigravity_fixture_hygiene.rs`:
   walk the parsed JSON at every depth, and for every key matching
   `*[Ii]d` / `*ID` whose value is not on a small, deliberate allowlist,
   replace the value with `REDACTED-<UPPERCASE-KEY>`. `thinkingSignature`
   values (an opaque encrypted blob of model internals; Antigravity never
   produces the string `REDACTED-THINKING-SIGNATURE`, so its presence is
   always a marker that the real value was stripped) get the same
   treatment under a fixed name. The walk also recurses into any string
   value that itself successfully parses as JSON — `argumentsJson` holds a
   serialized JSON document as a string, and a vendor id hidden inside one
   is exactly as much of a leak as one sitting in a normal field.

   The allowlist — the only identifiers left as real values — is:
   `trajectoryId`, `cascadeId`, `conversationId` (conversation identity;
   the deliberate cross-reference between `listing.json` and the step
   fixtures, above, depends on these being real), `TaskId` / `task_id`
   (shaped `<cascadeId>/task-N`, derived from the above), and a bare `id`
   in exactly two recognized *shapes*, never by the key name alone:

   - a tool call's `id`, when the value is `call_<digits>` or the
     containing object also carries `name` and `argumentsJson`;
   - a `taskDetails.id` (object also carries `logUri` and `description`),
     when the value itself has the `<cascadeId>/task-N` shape. This one
     is always byte-identical to the same task's `TaskId`/`task_id` value
     elsewhere in the same file; redacting only this occurrence would
     create a mismatch the real capture never has, since
     `taskDetails.id` is the schema's obvious join key to
     `TaskId`/`task_id` — a future reader correlating task-status events
     would join on it, get nothing, and hunt a bug that exists only in
     the fixture. Leaving it real costs no privacy, because the value is
     already public in the same file under an allowlisted key.

   Both cases check the object shape *and* the value shape — an object
   that merely looks like a tool call or a task-details record does not
   get a blanket pass for whatever id sits on it; a `bot-<uuid>` on a
   `logUri`+`description` object is still redacted. A bare `id`
   elsewhere (e.g. a vendor message object's `agentMessage.id`) is
   redacted like any other correlation id. See `ALLOWED_ID_KEYS` and
   `is_scoped_real_id` in the hygiene test for the exact rule, and
   update it — not this README — as the source of truth.

   This rule has already caught, across successive review passes,
   `executionId`, `sessionID`, `responseId`, `messageId`, two
   `agentMessage.id` values that an earlier, unscoped `id` allowlist
   entry had let through, and (in the other direction) an over-eager
   redaction of `taskDetails.id` that created a fixture-only mismatch
   with the real `TaskId`/`task_id` values nobody asked for. The hygiene
   test runs as part of the normal suite and fails on any future capture
   that reintroduces an unredacted vendor identifier — do not rely on
   manual review to catch this category again.

Everything else in these files — including some incidental references to
the operator's public GitHub handle and to a pilot tenant slug — is
unmodified from the live capture and already appears elsewhere in this
repository's committed history.
