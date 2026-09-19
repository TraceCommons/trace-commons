# Per-Author Perplexity (Shadow Mode) — Design

Date: 2026-09-18
Status: draft for review
Scope: `trace-commons-gate-api`, `trace-commons-gate-enclave`,
`trace-commons-server`, one migration. Two PRs.

## Problem

The gate's headline perplexity is
`exp(sum_c sum_nll_c / sum_c n_c)` — token-weighted over everything the
renderer emits (`chunk_aggregate.rs`). The renderer
(`chunker::parse_envelope_rendered_events`) emits every event's
`redacted_content`: user messages, assistant messages, reasoning, and
tool results. In an agent session most tokens are tool output and pasted
user input, which are also the most predictable text, so they set the
score.

Measured 2026-09-18 on 40 public tool-using sessions from
`jedisct1/agent-traces-swival` @ `6d527ff0`, scored on
`Qwen/Qwen3.8-27B` through NEAR AI with the production request shape
(experiment artifacts outside the repo; not the production packer, see
Limitations):

| tokens scored | token share | median | q3/q1 | rho vs length | rho vs rater difficulty |
|---|---|---|---|---|---|
| whole trace (today) | 100% | 2.77 | 1.30 | -0.54 | 0.05 |
| agent prose only | 1.9% | 4.54 | 2.01 | 0.37 | 0.45 |
| tool results only | 53% | 2.04 | 1.23 | -0.14 | 0.37 |
| user prompts only | 34% | 3.99 | 2.62 | -0.60 | -0.11 |
| tool-call arguments only | 7.2% | 3.00 | 1.87 | -0.18 | -0.47 |

Whole-trace perplexity is anti-correlated with length and carries no
difficulty signal. Perplexity restricted to the agent's own prose spreads
wider, loses the length artifact, and is the only variant that tracks
difficulty. Tool-result perplexity carries a weaker, separate signal.
Tool-call arguments are anti-correlated with difficulty; production
already excludes them because they live in `structured_payload`, which
the renderer never reads. What the renderer does emit for such an event
is its scaffold: an event with no `redacted_content` renders as
`"tool_call (Read): \n"` or `"tool_result: \n"`. In the 21 locally
redacted sessions, 8,516 of 10,533 `tool_result` events and all 2,017
`tool_call` events had no content, so each contributes one scaffold line
of highly predictable tokens to today's score. Those tokens are `Other`
below.

On 20 of the operator's own redacted Claude Code sessions, 12 fell below
the 6.0 floor, including the three Jev (an external rubric judge) rated
hardest, while short low-substance sessions scored highest
(rho vs length -0.49).

This is the same dilution `qualifying_token_fraction_micros` describes
("a quarter-substantive trace ... crushed in with pure boilerplate"),
attacked at a different granularity: that statistic asks which *chunks*
clear a floor; this one asks which *author* wrote each token.

### Limitations of the evidence

40 sessions, one security-audit harness, labels from a model rater (30
labeled, most at one difficulty level), correlations near 0.45 at n=30,
and a fixed-window chunker rather than the production event packer. This
is enough to justify computing and persisting the signal. It is not
enough to gate on it. Hence shadow mode.

## Goal

Compute per-author perplexity on every scored trace, persist it, and
backfill stored pilot traces, **without changing any gate decision**.
Calibration and any change to the floor are a later, separate decision.

## Non-goals

- No change to `perplexity_passed`, any floor, the credit function, or
  the scored text.
- No external judge (Jev), no substance check, no new junk filter.
- No change to which text the renderer emits. `CANONICAL_RENDER_VERSION`
  does not move.

## Design

### 1. Author spans in the chunker (`trace-commons-gate-enclave`)

```rust
pub enum AuthorKind { AgentProse, ToolResult, Other }
```

- `AgentProse`: `event_type == "assistant_message"`.
- `ToolResult`: `event_type == "tool_result"`.
- `Other`: everything else — `user_message`, `reasoning`, unrecognized
  types, and the `"{event_type} ({tool}): "` prefix and trailing newline
  that `render_event_text` adds to every event, whatever its kind.

`reasoning` is `Other` on purpose: its presence follows the contributor's
`--no-reasoning` choice, and a score must not move with a consent flag.

`parse_envelope_rendered_events` returns, per event, the rendered string
plus the char range of its content within that string and its kind.
`TraceChunk` gains `spans: Vec<AuthorSpan { start: u32, len: u32, kind }>`
in chars, relative to `TraceChunk::text`, covering the chunk exactly once
(gaps are not allowed; `Other` fills them). Spans survive both packing
paths: greedy event packing and the fixed-window split of an oversized
event. The fallback path for plaintext with no event structure yields one
`Other` span per chunk.

`TraceChunk::text` is byte-identical to today for every input. The
existing guard `only_redacted_content_reaches_the_scored_text` stays and
a new test asserts text identity against the pre-change renderer output
on the existing fixtures.

### 2. Token lengths at the scorer seam (`trace-commons-gate-api`)

`ChunkPerplexity` gains:

```rust
/// Char length of every returned token's decoded text, INCLUDING the
/// first token that `logprobs` drops: when present,
/// `token_char_lens.len() == logprobs.len() + 1`, and
/// `token_char_lens[i + 1]` is the length of the token `logprobs[i]`
/// scores. The dropped token still occupies chars in the chunk, so the
/// aligner needs its length to know where token 1 starts. Empty means
/// the scorer cannot say; attribution is then unavailable.
pub token_char_lens: Vec<u32>,
```

The `PerplexityScorer` trait signature does not change. Scorers report
lengths; they do not learn about spans. Alignment lives once, in the
enclave, which keeps the proprietary-backend seam a plain data contract.

`NearAiPerplexityScorer` fills it from the response's `logprobs.tokens`.
A response with no `tokens` array, or one whose length differs from
`token_logprobs`, yields an empty vector, never an error: the shadow
signal must not be able to fail a score that succeeds today.
`text_offset` is not used: observed 2026-09-18, the NEAR AI endpoint
returns `-1` for every token. Every other implementation
(`LocalPerplexityScorer`, reference, mocks, the default
`score_chunk` derived from `score`) returns an empty vector.

Trailing prediction position: the request sends `max_tokens: 1`, so the
response carries one generated token after the prompt tokens.
`chunk_perplexity_from_logprobs` sums `logprobs[1..]`, so that token's
NLL is part of today's whole-trace value; this design leaves that alone.
The aligner recognizes it as the token whose start cursor equals the
chunk's char count, requires it to be last, and attributes it to no
author. Per-author sums therefore exclude one token per chunk that the
whole-trace value includes.

### 3. Alignment and aggregation (`trace-commons-gate-enclave`)

New pure module `author_attribution.rs`:

```rust
pub fn attribute_chunk(chunk: &TraceChunk, scored: &ChunkPerplexity)
    -> Option<AuthorSums>        // sum_nll + tokens per AuthorKind
```

Walk tokens left to right, advancing a char cursor by each length; each
token is attributed to the author kind covering **most of its chars**, a
tie going to the author rather than to `Other`. (An earlier draft said
"first char". BPE tokens carry their leading space and every rendered
prefix ends in `": "`, so the first content token of every event starts on
a prefix char; a first-char rule hands the first word of every message to
`Other`. Found while writing the attribution tests.) Returns
`None` (chunk unattributed) when `token_char_lens` is empty, its length
is not `logprobs.len() + 1`, or the prompt tokens do not tile the chunk's
char count exactly. There is no partial or best-effort attribution: a chunk is
exact or it contributes nothing. (In the experiment a naive walk drifted
on 6 of 40 sessions where multi-byte characters split across tokens
decoded to replacement characters; those chunks must drop out rather
than mis-attribute.)

The result type lives in `trace-commons-gate-api` beside the other
decision types and travels as one value, `Option<AuthorPerplexity>`, from
the orchestrator to the storage boundary, where it fans out into five
columns:

```rust
pub struct AuthorPerplexity {
    pub agent_prose_perplexity_micros: Option<u64>,
    pub agent_prose_tokens: u64,
    pub tool_result_perplexity_micros: Option<u64>,
    pub tool_result_tokens: u64,
    /// Tokens in attributed chunks over all scored tokens.
    pub attributed_token_fraction_micros: u64,
}
```

The whole value is `None` when no chunk was attributable. When some were,
the fraction says how much of the trace the per-author values describe. A
per-author perplexity is `None` when that author has zero attributed
tokens (its token count is then 0). Math is f64, saturating micros,
non-finite collapses to `None` — never to a number that looks real.

### 4. Persistence (`trace-commons-server`, migration)

One migration (next free number at implementation time; V72 is the
newest on `origin/main` as of 2026-09-18) adds five nullable `BIGINT`
columns to `trace_gate_decisions`, named as the fields above, with
`ADD COLUMN IF NOT EXISTS` as in
`V54__trace_gate_decision_qualifying_mass.sql`. NULL reads
as "not computed" — pre-migration rows, and rows scored by a backend that
reports no token lengths. The columns follow the path
`qualifying_token_fraction_micros` already takes: `gate-api` decision
type, `trace_gate_service.rs`, `trace_corpus_storage.rs`,
`db/trace_corpus_pg.rs`, `db/postgres.rs`. RLS is unchanged: same table,
same policies.

Hash-only logging holds: the new code logs counts (chunks attributed,
chunks skipped) and fixed labels only.

### 5. Shadow guarantee

Nothing reads the new fields to decide anything. `compute_gate_version_hash`
covers "every dimension that influences the gate decision"; these fields
influence none and the renderer output is unchanged, so the hash inputs
and `CANONICAL_RENDER_VERSION` are untouched. A test scores one trace
twice, with and without token lengths, and asserts every pre-existing
decision field is identical.

### 6. Backfill (PR 2)

`/v1/admin/rescore-perplexity` runs `evaluate_perplexity_only`, which
shares `chunk_and_score_perplexity` with ingest. `PerplexityOnlyOutcome`
gains `author_perplexity`. No new route, no new credential; the existing
admin gate applies.

The route today overwrites `perplexity_micros`, `peak_perplexity_micros`
and `perplexity_passed` via `update_trace_gate_decision_perplexity`. A
backfill must not do that: the pilot's scorer model has changed since
those rows were written, and re-deriving `perplexity_passed` under a new
model would silently change gating history. PR 2 adds a query parameter
`author_only` (default `false`, preserving today's behavior; the route
already takes `limit` as a query parameter). The query struct refuses
unknown parameters, so a mistyped mode is a 400 rather than a silent full
re-score, and the acknowledgement echoes the mode it accepted and a second
storage method, `update_trace_gate_decision_author_perplexity`, that
writes only the five new columns on the latest decision row. With
`author_only: true` the three whole-trace columns are never touched.

Operational prerequisite, not a code change: before any backfill,
confirm which model and host the pilot scorer is actually using. The
recorded base URL (`qwen3-6-27b.completions.near.ai`) no longer completes
a TLS handshake and `Qwen/Qwen3.6-27B-FP8` has left the model list. A
backfill scored by a different model than the stored `perplexity_micros`
is still valid for the new columns (they are self-consistent per row)
but must not be compared against old whole-trace values.

## PR split

1. Spans, token lengths, attribution, aggregate fields, migration,
   persistence, ingest wiring, tests.
2. Rescore route writes the new columns; operator runbook note.

## Testing

TDD throughout. Unit: span coverage and exactness across greedy packing,
oversized-event windows, the strided cap, and the no-structure fallback;
scored-text identity; aligner on exact ASCII, on a multi-byte mismatch
(must return `None`), on empty lengths, on length/`logprobs` mismatch;
aggregate arithmetic including zero-token authors and partial
attribution; NEAR AI wire parsing from a recorded fixture with `tokens`.
Store: pg roundtrip of NULL and non-NULL values
(`trace_corpus_pg_store`, requires PostgreSQL). Shadow: decision
identity with and without lengths.

Verification before claiming done: `RUSTFLAGS="-D warnings"` check and
test, clippy with the repo allow-list, the `near-ai-scorer` and
`local-gpu-models` checks, `cargo fmt --all`, AGPL headers on new `.rs`
files, `license_boundary` untouched.

## What happens after

With backfilled values over real pilot traces: look at spread, at how
many traces have too few agent-prose tokens to trust, and at agreement
with human labels. Only then decide whether the floor moves to the new
signal, whether short traces need a fallback, and whether a minimum
agent-token count belongs in admission. None of that is decided here.
