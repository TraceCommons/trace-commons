# Fresh refactor task briefs

Parent: [fresh-session preparation](2026-09-12-fresh-refactor-sessions.md).

Pinned base: `60bfe5329aa166a5ec2cb5519d616f581d43fef3`

These four tasks propose separate behavior-preserving changes in the permissively licensed `trace-commons-contributor` crate. Each task owns one existing Rust file. None changes dependencies, public APIs, source discovery/root resolution, privacy policy, authentication, consent, routing, or error labels. They do not reuse the operator CLI plumbing from #606, source-root resolution from #616, or daemon IPC dispatch work from #632.

## Task 1: Make Cline content-block traversal allocation-free

**Owned path:** `crates/trace-commons-contributor/src/source/cline.rs`

**Current problem:** `map_message` at lines 291-400 normalizes a bare string into a synthetic `serde_json::Value` and clones an entire array of content blocks into `Vec<Value>` before traversing it. The following match then repeats full `SessionEvent` construction for text, thinking, and tool-use blocks. The temporary representation obscures the simple rule that a string is one text event while an array is visited in place, and it makes review of token-count attachment harder because `token_counts.take()` is separated from content normalization.

**Bounded desired change:** Introduce private, borrowed block traversal within this file (for example, a small private visitor/helper for one text value or one array block) and private constructors for the ordinary textual events. Keep `map_message` as the coordinator for role, timestamp, first observed model, and one-time assistant token-count attachment. Do not change block ordering, unknown-block `Opaque` emission, empty-text behavior, tool result verdict semantics, model selection, or any submitted fields. Do not move helpers into `source/mod.rs`.

**Existing verification:**

```bash
RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor --lib source::cline::tests
RUSTFLAGS='-D warnings' cargo clippy --locked -p trace-commons-contributor --lib -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

The existing Cline tests cover discovery/addressing, manifest-derived metadata, block order, string content, failed tools, unknown blocks, missing messages, fallback IDs, and file-size refusal.

**Main risks:** Accidentally attaching assistant token counts to reasoning or a later text block; changing an unknown role/block from one opaque event; treating an empty array or malformed content differently; retaining borrowed JSON past its owner. Compare complete `SessionTranscript` values in the existing fixture tests rather than testing only helper output.

## Task 2: Decompose Gemini turn emission by record kind

**Owned path:** `crates/trace-commons-contributor/src/source/gemini_cli.rs`

**Current problem:** `map_gemini_message` at lines 264-369 performs four distinct jobs in one function: first-model observation, thought projection, assistant answer/token projection, and paired tool-call/result projection. Each section constructs events directly and re-derives timestamp/content fields. The single function makes the required ordering contract—thoughts, answer, then call/result pairs—dependent on the physical layout of a long body and makes privacy review of `content` versus forbidden `displayContent` unnecessarily broad.

**Bounded desired change:** Extract private helpers in this file for thought emission, assistant-answer emission, and tool call/result-pair emission. Keep the top-level mapper responsible for the exact phase order, turn timestamp fallback, and first observed model. Pass only the data each helper needs, and keep `text_of` as the assistant-answer normalizer so `displayContent` remains unread. Preserve the separate existing thought-field joining and tool-result String/JSON serialization. Preserve current optional-token behavior and casts exactly; this task must not reinterpret malformed or oversized token fields.

**Existing verification:**

```bash
RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor --lib source::gemini_cli::tests
RUSTFLAGS='-D warnings' cargo clippy --locked -p trace-commons-contributor --lib -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

The existing Gemini tests cover all mapped record kinds, exact content joining without `displayContent`, timestamps and transcript metadata, subagent sessions, unknown message tolerance, malformed documents, symlink refusal, and size limits.

**Main risks:** Reordering thoughts/answer/tools; changing timestamp fallback for a thought or tool; separating a tool result from its call ID; consulting terminal-rendered `resultDisplay` or `displayContent`; changing absent explicit status from unknown to success. Assertions should remain at transcript/event level.

## Task 3: Encapsulate OpenCode export parser state

**Owned path:** `crates/trace-commons-contributor/src/source/opencode.rs`

**Current problem:** `parse_export_with_record_budget` at lines 242-447 owns session validation, three identity sets, chronological state, model aggregation, record-budget accounting, and all part-to-event conversion in one nested loop. Message invariants and part invariants mutate the same loose collection of local variables. This makes it easy for a future part handler to omit a bound/identity check or alter which stable error wins when more than one field is malformed.

**Bounded desired change:** Add a private parser-state struct in this file that owns `seen_messages`, `seen_parts`, `seen_calls`, prior message time, models, events, and the record budget. Move the existing message and part processing into private methods with explicit inputs (`session_id`, message metadata, part). Leave document/header validation and final `SessionTranscript` construction in `parse_export_with_record_budget`. Preserve validation order and every existing content-free error label, including duplicate/cross-session IDs, parent ordering, record limits, tool state/time checks, and unknown part refusal. Preserve the existing identity marker in completed/error `ToolResult.structured`; keep `ToolCall.arguments` unchanged and free of the export marker. Do not remove existing structured provenance.

**Existing verification:**

```bash
RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor --lib source::opencode::tests
RUSTFLAGS='-D warnings' cargo clippy --locked -p trace-commons-contributor --lib -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

The existing OpenCode tests cover observed identity/roles/tool outcomes, malformed and unknown versions, duplicate/reordered/cross-session records, parent links, tool status semantics, explicit-root security, unsupported opaque payloads, record/discovery limits, oversize refusal, and symlink refusal.

**Main risks:** Changing validation/error precedence; allowing a part to mutate state before all its identity fields validate; counting message and part budgets differently; accepting a forward parent or duplicate call; moving export identity/flags into tool arguments or removing existing structured result markers. Keep methods private and retain the current loop order.

## Task 4: Encapsulate trajectory record-to-event state

**Owned path:** `crates/trace-commons-contributor/src/source/trajectory.rs`

**Current problem:** `parse_trajectory` at lines 98-310 combines envelope/meta parsing with a large role match that directly mutates `events` and `seen_call_ids`. Assistant tool calls and later tool-result orphan checks share state implicitly across distant match arms. Repeated full event literals for opaque, textual, assistant, and tool records make the schema-specific keep/drop rules harder to audit.

**Bounded desired change:** Introduce a private trajectory parser-state struct in this file that owns events and the existing seen tool-call IDs (retain set membership after a result; do not introduce outstanding-call semantics; retain acceptance of repeated results and uncompleted calls), with a `consume_record` method and focused private methods for assistant calls and tool results. Keep `records_from`, leading-meta validation, source/model/cwd extraction, and final `ParsedTrajectory` assembly outside the state object. Preserve JSON-array and JSONL behavior, record order, duplicate/orphan rejection, strict timestamp and optional-field handling, the system-content drop rule, observation-content retention, malformed-argument length-only fallback, and every current error label. Do not alter discovery or path containment code later in the file.

**Existing verification:**

```bash
RUSTFLAGS='-D warnings' cargo test --locked -p trace-commons-contributor --lib source::trajectory::tests
RUSTFLAGS='-D warnings' cargo clippy --locked -p trace-commons-contributor --lib -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
cargo fmt --all -- --check
```

The existing trajectory tests cover every role in order, JSON and JSONL, missing/duplicate meta, unknown roles, timestamps, source-name validation, duplicate/orphan tool IDs, optional-field types, assistant ambiguity, malformed argument privacy, discovery containment, whole-session refusal, the Letta conformance corpus, and tool-call ID propagation.

**Main risks:** Checking a tool result against the wrong state set; changing whether duplicate calls are detected before event emission; retaining system content; leaking malformed argument text instead of its length; changing which malformed record aborts first. The state object should consume records sequentially and produce the same all-or-nothing result.

## Balanced assignment pairs

**Pair A — message mappers:** Task 1 (Cline) and Task 2 (Gemini). Both refactor one tolerant provider message mapper of roughly 100 lines, preserve ordered event expansion and optional timestamps/tokens, and rely on mature adapter fixture tests. Assign the two models one task each. Do not replay these same work items as new independent observations. Any later pilot requires distinct new work items.

**Pair B — fail-closed parsers:** Task 3 (OpenCode) and Task 4 (trajectory). Both convert a roughly 200-line nested parser into private state plus focused handlers while preserving identity/order/tool-link invariants, stable errors, record bounds, and content-handling policy. They are larger than Pair A but closely matched to each other in reasoning burden and regression risk.

For all four tasks, acceptance is a small same-file diff, unchanged public API and dependencies, unchanged existing test expectations, the adapter-focused test command passing with warnings denied, Clippy with the repository allowlist, and formatting clean. New tests are appropriate only when extraction exposes an existing behavior that the current transcript-level suite does not assert; helper-shape tests alone should not be added.

For comparison protocol setup, bind `comparison_configuration.prompt_template_digest` to the one common instruction-template byte sequence shared by all four assignments. Record each complete task brief's distinct digest as task provenance only. Using the full task-specific brief as the configuration digest would incorrectly place the four assignments in four different strata.
