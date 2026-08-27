# Letta trajectory conformance corpus

Each `.jsonl` file here is one trajectory file and one expectation. The
filename carries the expectation:

```
<expected>__<case>.jsonl
```

`<expected>` is either `ok` or one of the parser's documented rejection
reasons. `letta_conformance_corpus_matches_expected_outcomes` in
`src/source/trajectory.rs` parses every file and asserts the outcome matches,
and separately asserts that every documented rejection reason has at least one
fixture. Adding a case is dropping in a file; no Rust changes.

## Why this exists

The trajectory format is the versioned cross-harness standard, so producers
other than this repository need a way to check an emitter against it offline.
The parser is already fail-closed, but its contract was only partly exercised:
four of the ten rejection reasons had no test at all before this corpus.

## Contract details a producer is likely to get wrong

These are the ones that cost time when discovered against a live ingest rather
than a fixture:

- **`timestamp` is required on every non-meta record**, RFC 3339. Absent is
  `invalid_timestamp`, not a default.
- **`tool_calls[].args` is a *stringified* JSON object**, not an object. An
  object is `malformed_record`. A string that does not parse is accepted, but
  only its length is recorded, never the raw text.
- **An assistant record carries content or tool calls, never both.** Both is
  `malformed_record`; content must also be non-empty when there are no calls.
- **A `tool` record must follow the `assistant` record that issued its
  `tool_call_id`.** Otherwise `orphaned_tool_result`. Repeating an id is
  `duplicate_tool_call_id`, because it makes the orphan check meaningless.
- **`meta` is the first record and appears once.** Optional fields may be
  absent or null, but a present-and-wrong-typed one is `malformed_record`
  rather than being coerced away.
- **Both framings parse**: JSON Lines, or a single top-level JSON array.
- A record that is valid JSON but not an object is `unknown_record`, not
  `malformed_json`; `malformed_json` is reserved for text that does not parse.
- **`system` and `observation` are accepted, and are not equivalent.** An
  observation keeps its content; a system record keeps only its position, on
  the grounds that a system prompt is near-identical across a harness's
  sessions and carries whatever project context that harness injected. Both
  still require a well-formed `content` and `timestamp`. They were added to
  trajectory-v1 upstream after this reader shipped, and until that was fixed
  either one rejected the entire file as `unknown_record` -- which is the
  reason to add a fixture here the next time upstream grows a role rather
  than finding out from a contributor.
