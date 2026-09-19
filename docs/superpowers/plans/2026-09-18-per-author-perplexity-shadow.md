# Per-Author Perplexity (Shadow Mode) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Compute perplexity separately over the agent's prose and over tool results on every scored trace, persist both, and let the rescore route backfill them — without changing any gate decision.

**Architecture:** The chunker tags every char of the scored text with who authored it. Scorers report each token's char length. A pure enclave module walks tokens along each chunk and splits the NLL by author, exact-or-skip per chunk. The result travels as one `Option<AuthorPerplexity>` to the storage boundary, where it becomes five nullable columns on `trace_gate_decisions`.

**Tech Stack:** Rust workspace (`trace-commons-gate-api`, `trace-commons-gate-enclave`, `trace-commons-server`), PostgreSQL, refinery-style `migrations/V*.sql`.

**Spec:** `docs/superpowers/specs/2026-09-18-per-author-perplexity-shadow-design.md`

## Global Constraints

- Shadow mode: nothing may read the new values to decide anything. `perplexity_passed`, every floor, the credit function and `compute_gate_version_hash` inputs are untouched.
- `TraceChunk::text` stays byte-identical for every input. `CANONICAL_RENDER_VERSION` ("events.v1") and `CHUNK_SELECTION_ALGORITHM` do not change.
- The `PerplexityScorer` trait signature does not change.
- The shadow signal can never fail a score that succeeds today: every new failure mode degrades to "absent", never to an `Err`.
- Exact-or-skip: a chunk whose token lengths do not tile its chars exactly contributes nothing. No best-effort attribution.
- Hash-only logging: new log lines carry counts and fixed labels only, never text, token strings or values.
- PostgreSQL-only. All five new columns are nullable `BIGINT` with no `DEFAULT`; NULL means "not computed".
- Every new `.rs` file in these three AGPL crates starts with:
  ```rust
  // Copyright (C) 2026 K&Z Partners LLC
  // SPDX-License-Identifier: AGPL-3.0-or-later
  ```
- No new dependencies.
- Commit style: short imperative subject, no `feat:`/`fix:` prefix, no emojis. End every commit message with
  `Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>`.
- Verify with CI's flags, not plain cargo: `RUSTFLAGS="-D warnings"` on check and test, clippy with the repo allow-list (see Task 8).
- `trace-commons-ingest.rs` tests live in `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`. Do not inline them back.

## Review Focus

1. **Multi-byte text split across tokens** (emoji, CJK, accented chars in tool output): the decoded token strings contain replacement characters, so lengths do not tile. Expected: that chunk is skipped and `attributed_token_fraction_micros` drops; no mis-attribution, no error. Test in Task 4.
2. **A scorer that reports no token lengths** (local GPU scorer, reference, mocks, any proprietary backend): expected `author_perplexity == None`, columns NULL, every other decision field identical to today. Test in Task 5.
3. **A trace with no assistant prose at all** (tool-only or user-only sessions, the empty-after-redaction session): expected `agent_prose_tokens == 0` and `agent_prose_perplexity_micros == None`, not 0 and not a division by zero. Test in Task 4.
4. **An oversized single event split into fixed windows, and a capped trace**: spans must still tile every surviving chunk exactly, with the prefix `Other` only in the first window. Test in Task 3.
5. **A NEAR AI response whose `tokens` array is missing or a different length from `token_logprobs`**: expected empty lengths and a normal score, not `NearAiScorerResponseParseFailed`. Test in Task 2.

---

## File Structure

| File | Responsibility |
|---|---|
| `crates/trace-commons-gate-api/src/perplexity.rs` (modify) | `ChunkPerplexity::token_char_lens` |
| `crates/trace-commons-gate-api/src/decision.rs` (modify) | `AuthorPerplexity`; field on `GateDecision` and `PerplexityOnlyOutcome` |
| `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs` (modify) | read `logprobs.tokens`, report lengths |
| `crates/trace-commons-gate-enclave/src/chunker.rs` (modify) | `AuthorKind`, `AuthorSpan`, `RenderedEvent`, spans on `TraceChunk` |
| `crates/trace-commons-gate-enclave/src/author_attribution.rs` (create) | pure alignment + per-author aggregation |
| `crates/trace-commons-gate-enclave/src/orchestrator.rs` (modify) | call the aggregator, set the decision field |
| `migrations/V73__trace_gate_decision_author_perplexity.sql` (create) | five nullable columns |
| `crates/trace-commons-server/src/trace_gate_service.rs` (modify) | carry `Option<AuthorPerplexity>` |
| `crates/trace-commons-server/src/trace_corpus_storage.rs` (modify) | five `Option<i64>` on the decision row; PR 2 update method |
| `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (modify) | INSERT / SELECT / UPDATE |
| `crates/trace-commons-server/src/db/postgres.rs` (modify) | migration shape test |
| `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` (modify) | map to the row; PR 2 `author_only` |
| `docs/operator/perplexity-scoring-driver.md` (modify, PR 2) | backfill runbook note |

Tasks 1-8 are PR 1. Tasks 9-10 are PR 2, branched from PR 1's head.

---

### Task 1: `token_char_lens` on `ChunkPerplexity`

**Files:**
- Modify: `crates/trace-commons-gate-api/src/perplexity.rs` (struct at ~28, default `score_chunk` at ~56)
- Modify: every `ChunkPerplexity {` literal — `git grep -n 'ChunkPerplexity {' -- 'crates/*.rs'` (8 at plan time: 3 in `gate-api/src/perplexity.rs`, 3 in `gate-enclave/src/perplexity_near_ai.rs`, 2 in `gate-enclave/src/chunk_aggregate.rs`)

**Interfaces:**
- Produces: `ChunkPerplexity::token_char_lens: Vec<u32>`. When non-empty, `len() == logprobs.len() + 1` and `token_char_lens[i + 1]` is the char length of the token `logprobs[i]` scores; `token_char_lens[0]` is the dropped first token.

- [ ] **Step 1: Write the failing test** — append inside `mod tests` in `crates/trace-commons-gate-api/src/perplexity.rs`:

```rust
    #[test]
    fn default_score_chunk_reports_no_token_lengths() {
        // The default derives sums from collapsed micros and never sees
        // tokens, so it must say "cannot say" rather than invent lengths.
        let chunk = ZeroScorer.score_chunk(b"anything").unwrap();
        assert!(chunk.token_char_lens.is_empty());
    }
```

(`ZeroScorer` already exists in that test module; it is used by `default_score_chunk_zero_tokens_is_all_zero`.)

- [ ] **Step 2: Run it to see it fail**

Run: `cargo test -p trace-commons-gate-api default_score_chunk_reports_no_token_lengths`
Expected: compile error, `no field token_char_lens on type ChunkPerplexity`.

- [ ] **Step 3: Add the field** — in `ChunkPerplexity`, after `logprobs`:

```rust
    /// Char length of every returned token's decoded text, INCLUDING the
    /// first token that `logprobs` drops: when present,
    /// `token_char_lens.len() == logprobs.len() + 1`, and
    /// `token_char_lens[i + 1]` is the length of the token `logprobs[i]`
    /// scores. The dropped token still occupies chars in the chunk, so an
    /// aligner needs its length to know where token 1 starts. Empty means
    /// the scorer cannot say; per-author attribution is then unavailable.
    /// Shadow-mode input: nothing that decides a gate reads it.
    pub token_char_lens: Vec<u32>,
```

Add `token_char_lens: Vec::new(),` to both literals in the default `score_chunk` and to every other literal the compiler reports in `gate-api` and `gate-enclave`. Do not populate it anywhere yet.

- [ ] **Step 4: Run the tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-api -p trace-commons-gate-enclave`
Expected: PASS, including the new test.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-gate-api crates/trace-commons-gate-enclave
git commit -m "Add token char lengths to ChunkPerplexity"
```

---

### Task 2: NEAR AI scorer reports token lengths

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs` — `LogprobsBlock` (~545), `parse_logprobs_body` (~426), `fetch_logprobs` / `fetch_logprobs_once` (~318-420), `chunk_perplexity_from_logprobs` (~460), `score_chunk` (~492)

**Interfaces:**
- Consumes: `ChunkPerplexity::token_char_lens` (Task 1).
- Produces: `NearAiPerplexityScorer::score_chunk` returns lengths when the response carries a `tokens` array parallel to `token_logprobs`; otherwise empty.

This file is compiled only under `--features near-ai-scorer`; run its tests with that feature.

- [ ] **Step 1: Write the failing tests** — inside `mod tests`:

```rust
    const BODY_WITH_TOKENS: &str = r#"{"choices":[{"logprobs":{
        "tokens":["The"," capital"," of"," Fran","ce","."," It"],
        "token_logprobs":[null,-13.17,-0.5,-2.0,-0.1,-1.5,-3.0],
        "text_offset":[-1,-1,-1,-1,-1,-1,-1]}}]}"#;

    #[test]
    fn scored_body_reports_one_char_length_per_returned_token() {
        let scored = parse_scored_body(BODY_WITH_TOKENS).unwrap();
        assert_eq!(scored.logprobs.len(), 7);
        assert_eq!(scored.token_char_lens, vec![3, 8, 3, 5, 2, 1, 3]);
    }

    #[test]
    fn token_lengths_are_chars_not_bytes() {
        let body = r#"{"choices":[{"logprobs":{
            "tokens":["na","ï","ve"],"token_logprobs":[null,-1.0,-1.0]}}]}"#;
        assert_eq!(parse_scored_body(body).unwrap().token_char_lens, vec![2, 1, 2]);
    }

    #[test]
    fn a_missing_tokens_array_scores_normally_with_no_lengths() {
        let body = r#"{"choices":[{"logprobs":{"token_logprobs":[null,-1.0,-2.0]}}]}"#;
        let scored = parse_scored_body(body).unwrap();
        assert_eq!(scored.logprobs.len(), 3);
        assert!(scored.token_char_lens.is_empty());
    }

    #[test]
    fn a_tokens_array_of_the_wrong_length_is_ignored_not_an_error() {
        let body = r#"{"choices":[{"logprobs":{
            "tokens":["a","b"],"token_logprobs":[null,-1.0,-2.0]}}]}"#;
        assert!(parse_scored_body(body).unwrap().token_char_lens.is_empty());
    }

    #[test]
    fn chunk_lengths_stay_parallel_to_the_usable_logprobs() {
        let scored = parse_scored_body(BODY_WITH_TOKENS).unwrap();
        let chunk = chunk_perplexity_from_scored(&scored, -10.0);
        assert_eq!(chunk.logprobs.len(), 6);
        assert_eq!(chunk.token_char_lens.len(), chunk.logprobs.len() + 1);
    }

    #[test]
    fn a_degenerate_chunk_drops_its_lengths_with_its_logprobs() {
        let scored = ScoredTokens {
            logprobs: vec![0.0, f32::NAN],
            token_char_lens: vec![1, 1],
        };
        let chunk = chunk_perplexity_from_scored(&scored, -10.0);
        assert_eq!(chunk.tokens, 0);
        assert!(chunk.token_char_lens.is_empty());
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p trace-commons-gate-enclave --features near-ai-scorer perplexity_near_ai`
Expected: compile errors — `parse_scored_body`, `ScoredTokens`, `chunk_perplexity_from_scored` not found.

- [ ] **Step 3: Implement.** Add `tokens` to the wire struct:

```rust
    /// Decoded text of each token, parallel to `token_logprobs`. Optional:
    /// the score does not depend on it. `text_offset` is deliberately not
    /// read -- observed 2026-09-18, the endpoint returns -1 for every token.
    #[serde(default)]
    tokens: Option<Vec<String>>,
```

Add beside `parse_logprobs_body`:

```rust
/// One response's scoring material: the raw logprob slice (element 0 is the
/// BOS placeholder) and, when the response carried a parallel `tokens`
/// array, each token's char length.
#[derive(Debug, Clone, PartialEq)]
struct ScoredTokens {
    logprobs: Vec<f32>,
    /// Same length as `logprobs`, or empty.
    token_char_lens: Vec<u32>,
}
```

Rename `parse_logprobs_body` to `parse_scored_body`, returning `anyhow::Result<ScoredTokens>`. Keep its body; before the `token_logprobs` loop consumes `lp`, take the lengths:

```rust
    // Lengths feed a shadow signal only. A response without a usable
    // `tokens` array still scores exactly as before.
    let token_char_lens: Vec<u32> = match lp.tokens.take() {
        Some(tokens) if tokens.len() == lp.token_logprobs.len() => tokens
            .iter()
            .map(|t| u32::try_from(t.chars().count()).unwrap_or(u32::MAX))
            .collect(),
        _ => Vec::new(),
    };
```

(`lp` must be bound `let mut lp`.) Return `Ok(ScoredTokens { logprobs: out, token_char_lens })`.

Change `fetch_logprobs_once` and `fetch_logprobs` to return `ScoredTokens` instead of `Vec<f32>`. In `score` and `score_rarity`, use `.logprobs` where the slice was used; their behavior is unchanged.

Replace `chunk_perplexity_from_logprobs` with a wrapper pair so existing tests of the old name keep working:

```rust
fn chunk_perplexity_from_logprobs(logprobs: &[f32], tail_logprob_cutoff: f32) -> ChunkPerplexity {
    chunk_perplexity_from_scored(
        &ScoredTokens { logprobs: logprobs.to_vec(), token_char_lens: Vec::new() },
        tail_logprob_cutoff,
    )
}

/// As `chunk_perplexity_from_logprobs`, carrying token lengths through.
/// `logprobs` loses element 0; `token_char_lens` keeps it, because the
/// dropped token still occupies chars. A degenerate chunk drops both.
fn chunk_perplexity_from_scored(scored: &ScoredTokens, tail_logprob_cutoff: f32) -> ChunkPerplexity {
    let logprobs = &scored.logprobs;
    if logprobs.len() < 2 || logprobs[1..].iter().any(|lp| !lp.is_finite()) {
        return ChunkPerplexity {
            sum_nll: 0.0,
            tokens: 0,
            tail_tokens: 0,
            logprobs: Vec::new(),
            token_char_lens: Vec::new(),
        };
    }
    let usable = &logprobs[1..];
    let sum_nll: f64 = usable.iter().map(|lp| -(*lp as f64)).sum();
    let tail_tokens = usable.iter().filter(|&&lp| lp < tail_logprob_cutoff).count() as u64;
    ChunkPerplexity {
        sum_nll,
        tokens: usable.len() as u64,
        tail_tokens,
        logprobs: usable.to_vec(),
        token_char_lens: scored.token_char_lens.clone(),
    }
}
```

`score_chunk` becomes:

```rust
    fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
        let scored = self.fetch_logprobs(chunk)?;
        Ok(chunk_perplexity_from_scored(&scored, self.cfg.tail_logprob_cutoff))
    }
```

Update the existing tests that call `parse_logprobs_body` to call `parse_scored_body(...)?.logprobs`.

- [ ] **Step 4: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-enclave --features near-ai-scorer`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs
git commit -m "Report token char lengths from the NEAR AI scorer"
```

---

### Task 3: Author spans in the chunker

**Files:**
- Modify: `crates/trace-commons-gate-enclave/src/chunker.rs`
- Modify: any `TraceChunk {` literal outside it — `git grep -n 'TraceChunk {' -- 'crates/*.rs'`

**Interfaces:**
- Produces:
  ```rust
  pub enum AuthorKind { AgentProse, ToolResult, Other }   // Copy, Eq, Debug
  impl AuthorKind { pub const COUNT: usize = 3; pub fn index(self) -> usize; }
  pub struct AuthorSpan { pub start: u32, pub len: u32, pub kind: AuthorKind }
  pub struct RenderedEvent { pub text: String, pub spans: Vec<AuthorSpan> }
  pub fn parse_envelope_events(plaintext: &[u8]) -> Option<Vec<RenderedEvent>>;
  pub fn chunk_events(events: &[RenderedEvent], cfg: &ChunkerConfig) -> ChunkPlan;
  // TraceChunk gains: pub spans: Vec<AuthorSpan>
  ```
  Spans are in chars relative to `text`, sorted, non-overlapping, and tile `0..text.chars().count()` exactly. Adjacent spans of one kind are merged. An empty chunk has no spans.
- Unchanged signatures: `render_event_text`, `parse_envelope_rendered_events`, `chunk_rendered_events`, `chunk_envelope_plaintext`.

- [ ] **Step 1: Write the failing tests** — inside `mod tests`:

```rust
    fn assert_spans_tile(chunk: &TraceChunk) {
        let mut cursor = 0u32;
        for s in &chunk.spans {
            assert_eq!(s.start, cursor, "spans must be contiguous");
            assert!(s.len > 0, "no empty spans");
            cursor += s.len;
        }
        assert_eq!(cursor as usize, chunk.text.chars().count(), "spans must tile the text");
        for w in chunk.spans.windows(2) {
            assert_ne!(w[0].kind, w[1].kind, "adjacent same-kind spans must merge");
        }
    }

    fn kind_chars(chunk: &TraceChunk, kind: AuthorKind) -> String {
        let chars: Vec<char> = chunk.text.chars().collect();
        chunk
            .spans
            .iter()
            .filter(|s| s.kind == kind)
            .flat_map(|s| chars[s.start as usize..(s.start + s.len) as usize].iter())
            .collect()
    }

    fn typed_envelope(events: &[(&str, Option<&str>, &str)]) -> Vec<u8> {
        let events: Vec<serde_json::Value> = events
            .iter()
            .map(|(ty, tool, content)| {
                let mut e = serde_json::json!({"event_type": ty, "redacted_content": content});
                if let Some(t) = tool {
                    e["tool_name"] = serde_json::json!(t);
                }
                e
            })
            .collect();
        serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap()
    }

    #[test]
    fn spans_mark_only_the_content_of_prose_and_tool_results() {
        let env = typed_envelope(&[
            ("user_message", None, "fix it"),
            ("assistant_message", None, "on it"),
            ("tool_call", Some("Bash"), ""),
            ("tool_result", Some("Bash"), "ok\n"),
            ("reasoning", None, "hmm"),
        ]);
        let plan = chunk_envelope_plaintext(&env, &cfg(2048, 3072, 16));
        assert_eq!(plan.chunks.len(), 1);
        let c = &plan.chunks[0];
        assert_spans_tile(c);
        assert_eq!(kind_chars(c, AuthorKind::AgentProse), "on it");
        assert_eq!(kind_chars(c, AuthorKind::ToolResult), "ok\n");
        // Prefixes, newlines, user text, reasoning and the empty tool_call
        // scaffold are all Other.
        assert!(kind_chars(c, AuthorKind::Other).contains("reasoning: hmm"));
        assert!(kind_chars(c, AuthorKind::Other).contains("tool_call (Bash): "));
    }

    #[test]
    fn adding_spans_does_not_change_the_scored_text() {
        let env = typed_envelope(&[
            ("user_message", None, "héllo wörld"),
            ("assistant_message", None, "naïve café"),
            ("tool_result", Some("Read"), "x".repeat(40).as_str()),
        ]);
        let config = cfg(4, 6, 16);
        let legacy = chunk_rendered_events(&parse_envelope_rendered_events(&env).unwrap(), &config);
        let typed = chunk_envelope_plaintext(&env, &config);
        let texts = |p: &ChunkPlan| p.chunks.iter().map(|c| c.text.clone()).collect::<Vec<_>>();
        assert_eq!(texts(&legacy), texts(&typed));
        assert_eq!(legacy.chunks_capped, typed.chunks_capped);
    }

    #[test]
    fn an_oversized_event_keeps_exact_spans_across_its_windows() {
        let env = typed_envelope(&[("tool_result", Some("Read"), "y".repeat(100).as_str())]);
        let plan = chunk_envelope_plaintext(&env, &cfg(5, 6, 64));
        assert!(plan.chunks.len() > 1);
        for c in &plan.chunks {
            assert_spans_tile(c);
        }
        let all: String = plan.chunks.iter().map(|c| kind_chars(c, AuthorKind::ToolResult)).collect();
        assert_eq!(all, "y".repeat(100));
        // The prefix lives in the first window only.
        assert!(kind_chars(&plan.chunks[0], AuthorKind::Other).starts_with("tool_result (Read): "));
        assert_eq!(kind_chars(&plan.chunks[1], AuthorKind::Other), "");
    }

    #[test]
    fn spans_survive_the_strided_cap() {
        let contents: Vec<String> = (0..40).map(|i| format!("assistant text number {i} ").repeat(3)).collect();
        let events: Vec<(&str, Option<&str>, &str)> =
            contents.iter().map(|c| ("assistant_message", None, c.as_str())).collect();
        let plan = chunk_envelope_plaintext(&typed_envelope(&events), &cfg(8, 64, 4));
        assert!(plan.chunks_capped);
        assert_eq!(plan.chunks.len(), 4);
        for c in &plan.chunks {
            assert_spans_tile(c);
        }
    }

    #[test]
    fn unstructured_plaintext_is_all_other() {
        let plan = chunk_envelope_plaintext(b"not json at all", &cfg(2048, 3072, 16));
        let c = &plan.chunks[0];
        assert_spans_tile(c);
        assert!(c.spans.iter().all(|s| s.kind == AuthorKind::Other));
    }

    #[test]
    fn legacy_string_events_are_all_other() {
        let plan = chunk_rendered_events(&["assistant_message: hi\n".to_string()], &cfg(2048, 3072, 16));
        assert_spans_tile(&plan.chunks[0]);
        assert_eq!(kind_chars(&plan.chunks[0], AuthorKind::AgentProse), "");
    }
```

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p trace-commons-gate-enclave chunker`
Expected: compile errors — `AuthorKind`, `spans` not found.

- [ ] **Step 3: Implement.** Add after `TraceChunk`'s definition (and add `pub spans: Vec<AuthorSpan>,` to `TraceChunk`):

```rust
/// Who authored a run of scored chars. Drives per-author perplexity, a
/// shadow signal; it never changes what text is scored.
///
/// `reasoning` is deliberately `Other`: its presence follows the
/// contributor's `--no-reasoning` choice, and a score must not move with a
/// consent flag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthorKind {
    AgentProse,
    ToolResult,
    Other,
}

impl AuthorKind {
    pub const COUNT: usize = 3;

    pub fn index(self) -> usize {
        match self {
            AuthorKind::AgentProse => 0,
            AuthorKind::ToolResult => 1,
            AuthorKind::Other => 2,
        }
    }

    fn of_event_type(event_type: &str) -> Self {
        match event_type {
            "assistant_message" => AuthorKind::AgentProse,
            "tool_result" => AuthorKind::ToolResult,
            _ => AuthorKind::Other,
        }
    }
}

/// A run of chars of one kind, in chars relative to the owning text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorSpan {
    pub start: u32,
    pub len: u32,
    pub kind: AuthorKind,
}

/// One canonically rendered event with its author spans. `text` is exactly
/// what [`render_event_text`] produces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedEvent {
    pub text: String,
    pub spans: Vec<AuthorSpan>,
}

/// Append a span, merging into the previous one when the kind matches.
/// Zero-length spans are dropped so spans always tile with no empties.
fn push_span(spans: &mut Vec<AuthorSpan>, len: usize, kind: AuthorKind) {
    let len = u32::try_from(len).unwrap_or(u32::MAX);
    if len == 0 {
        return;
    }
    if let Some(last) = spans.last_mut() {
        if last.kind == kind {
            last.len = last.len.saturating_add(len);
            return;
        }
    }
    let start = spans.last().map(|s| s.start.saturating_add(s.len)).unwrap_or(0);
    spans.push(AuthorSpan { start, len, kind });
}

/// The part of `spans` covering chars `[from, from + len)`, rebased to 0.
fn slice_spans(spans: &[AuthorSpan], from: usize, len: usize) -> Vec<AuthorSpan> {
    let (from, to) = (from as u64, (from + len) as u64);
    let mut out = Vec::new();
    for s in spans {
        let (s_from, s_to) = (s.start as u64, s.start as u64 + s.len as u64);
        let (lo, hi) = (s_from.max(from), s_to.min(to));
        if lo < hi {
            push_span(&mut out, (hi - lo) as usize, s.kind);
        }
    }
    out
}

fn render_event(event_type: &str, tool_name: Option<&str>, content: &str) -> RenderedEvent {
    let text = render_event_text(event_type, tool_name, content);
    let content_chars = content.chars().count();
    // `render_event_text` is `<prefix><content>\n`, so the prefix length is
    // what is left. Derived rather than re-formatted so the two can never
    // disagree about the prefix.
    let prefix_chars = text.chars().count() - content_chars - 1;
    let mut spans = Vec::new();
    push_span(&mut spans, prefix_chars, AuthorKind::Other);
    push_span(&mut spans, content_chars, AuthorKind::of_event_type(event_type));
    push_span(&mut spans, 1, AuthorKind::Other);
    RenderedEvent { text, spans }
}
```

Refactor `parse_envelope_rendered_events` so both parsers share one body:

```rust
/// As [`parse_envelope_rendered_events`], keeping each event's author spans.
pub fn parse_envelope_events(plaintext: &[u8]) -> Option<Vec<RenderedEvent>> {
    let v: serde_json::Value = serde_json::from_slice(plaintext).ok()?;
    let events = v.get("events")?.as_array()?;
    if events.is_empty() {
        return None;
    }
    Some(
        events
            .iter()
            .map(|e| {
                let event_type = e.get("event_type").and_then(|x| x.as_str()).unwrap_or("event");
                let tool_name = e.get("tool_name").and_then(|x| x.as_str());
                let content = e.get("redacted_content").and_then(|x| x.as_str()).unwrap_or("");
                render_event(event_type, tool_name, content)
            })
            .collect(),
    )
}

pub fn parse_envelope_rendered_events(plaintext: &[u8]) -> Option<Vec<String>> {
    Some(parse_envelope_events(plaintext)?.into_iter().map(|e| e.text).collect())
}
```

Keep the existing doc comment on `parse_envelope_rendered_events`.

Make `split_fixed_char_windows` unchanged. Rewrite packing around `RenderedEvent`, keeping the control flow identical to today's so text cannot drift:

```rust
pub fn chunk_rendered_events(events: &[String], cfg: &ChunkerConfig) -> ChunkPlan {
    let typed: Vec<RenderedEvent> = events
        .iter()
        .map(|text| {
            let mut spans = Vec::new();
            push_span(&mut spans, text.chars().count(), AuthorKind::Other);
            RenderedEvent { text: text.clone(), spans }
        })
        .collect();
    chunk_events(&typed, cfg)
}

/// Greedily pack consecutive rendered events into chunks of at most
/// `target_chars`, respecting event boundaries. A single event larger than
/// `max_chars` splits into `target_chars` fixed windows. Applies the cap via
/// coverage-preserving strided selection. Author spans follow the text
/// through every path.
pub fn chunk_events(events: &[RenderedEvent], cfg: &ChunkerConfig) -> ChunkPlan {
    let target = cfg.target_chars();
    let max = cfg.max_chars();
    let mut packed: Vec<(String, Vec<AuthorSpan>)> = Vec::new();
    let mut current = String::new();
    let mut current_spans: Vec<AuthorSpan> = Vec::new();
    let mut current_chars = 0usize;
    for event in events {
        let event_chars = event.text.chars().count();
        if event_chars > max {
            if !current.is_empty() {
                packed.push((std::mem::take(&mut current), std::mem::take(&mut current_spans)));
                current_chars = 0;
            }
            let mut from = 0usize;
            for window in split_fixed_char_windows(&event.text, target) {
                let len = window.chars().count();
                packed.push((window, slice_spans(&event.spans, from, len)));
                from += len;
            }
            continue;
        }
        if !current.is_empty() && current_chars + event_chars > target {
            packed.push((std::mem::take(&mut current), std::mem::take(&mut current_spans)));
            current_chars = 0;
        }
        current.push_str(&event.text);
        for s in &event.spans {
            push_span(&mut current_spans, s.len as usize, s.kind);
        }
        current_chars += event_chars;
    }
    if !current.is_empty() {
        packed.push((current, current_spans));
    }
    if packed.is_empty() {
        packed.push((String::new(), Vec::new()));
    }
    finalize_plan(packed, cfg)
}
```

Change `finalize_plan` to take `Vec<(String, Vec<AuthorSpan>)>`: replace `texts` with `packed`, make the take-once vector `Vec<Option<(String, Vec<AuthorSpan>)>>`, and build `TraceChunk { chunk_index: i as u32, text, spans }` from the taken pair. Keep every existing comment.

In `chunk_envelope_plaintext`, use the typed parser and give the fallback one `Other` span per window:

```rust
    if let Some(events) = parse_envelope_events(plaintext) {
        return chunk_events(&events, cfg);
    }
    let text = String::from_utf8_lossy(plaintext);
    let packed = split_fixed_char_windows(&text, cfg.target_chars())
        .into_iter()
        .map(|w| {
            let mut spans = Vec::new();
            push_span(&mut spans, w.chars().count(), AuthorKind::Other);
            (w, spans)
        })
        .collect();
    finalize_plan(packed, cfg)
```

Add `spans: Vec::new()` — or the right spans — to any `TraceChunk {` literal the compiler reports elsewhere.

- [ ] **Step 4: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-enclave`
Expected: PASS, including every pre-existing chunker test unchanged.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-gate-enclave
git commit -m "Tag scored chars with their author in the chunker"
```

---

### Task 4: Attribution and per-author aggregation

**Files:**
- Create: `crates/trace-commons-gate-enclave/src/author_attribution.rs`
- Modify: `crates/trace-commons-gate-enclave/src/lib.rs` (add `pub mod author_attribution;` beside `pub mod chunk_aggregate;`)
- Modify: `crates/trace-commons-gate-api/src/decision.rs` (add `AuthorPerplexity`)

**Interfaces:**
- Consumes: `TraceChunk::spans`, `AuthorKind::{COUNT,index}` (Task 3); `ChunkPerplexity::{logprobs, tokens, token_char_lens}` (Task 1).
- Produces, in `trace_commons_gate_api::decision`:
  ```rust
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  pub struct AuthorPerplexity {
      pub agent_prose_perplexity_micros: Option<u64>,
      pub agent_prose_tokens: u64,
      pub tool_result_perplexity_micros: Option<u64>,
      pub tool_result_tokens: u64,
      pub attributed_token_fraction_micros: u64,
  }
  ```
  and in `trace_commons_gate_enclave::author_attribution`:
  ```rust
  pub struct AuthorSums { pub nll: [f64; AuthorKind::COUNT], pub tokens: [u64; AuthorKind::COUNT] }
  pub fn attribute_chunk(chunk: &TraceChunk, scored: &ChunkPerplexity) -> Option<AuthorSums>;
  pub fn aggregate_author_perplexity(chunks: &[TraceChunk], scored: &[ChunkPerplexity]) -> Option<AuthorPerplexity>;
  ```

- [ ] **Step 1: Add the result type** to `crates/trace-commons-gate-api/src/decision.rs`, above `PerplexityOnlyOutcome`:

```rust
/// Perplexity split by who authored the scored tokens. Shadow mode:
/// recorded, gates nothing. Absent as a whole when no chunk could be
/// attributed (the scorer reported no token lengths, or none tiled).
///
/// Whole-trace perplexity is token-weighted, so in an agent session it is
/// set by tool output and pasted input -- the most numerous and most
/// predictable tokens. See
/// `docs/superpowers/specs/2026-09-18-per-author-perplexity-shadow-design.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AuthorPerplexity {
    /// `exp(sum_nll / n)` over `assistant_message` content tokens, in
    /// micros. `None` when there were none -- never 0, which would read as
    /// a real and maximally unsurprising score.
    pub agent_prose_perplexity_micros: Option<u64>,
    pub agent_prose_tokens: u64,
    /// As above over `tool_result` content tokens.
    pub tool_result_perplexity_micros: Option<u64>,
    pub tool_result_tokens: u64,
    /// Tokens in attributed chunks over all scored tokens: how much of the
    /// trace the values above describe.
    pub attributed_token_fraction_micros: u64,
}
```

- [ ] **Step 2: Write the failing tests** — create `crates/trace-commons-gate-enclave/src/author_attribution.rs` with the header, a `todo`-free stub is not allowed, so write the tests first and let them fail to compile:

```rust
// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Split a chunk's per-token NLL by who authored each token.
//!
//! Pure: no I/O, no logging. Exact-or-skip: a chunk whose token lengths do
//! not tile its chars exactly yields `None` and contributes nothing, so a
//! mis-decoded token can shrink coverage but can never mis-attribute.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::{chunk_envelope_plaintext, ChunkerConfig};

    fn chunk_of(events: &[(&str, &str)]) -> TraceChunk {
        let events: Vec<serde_json::Value> = events
            .iter()
            .map(|(ty, c)| serde_json::json!({"event_type": ty, "redacted_content": c}))
            .collect();
        let env = serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap();
        let cfg = ChunkerConfig { target_tokens: 2048, max_tokens: 3072, chunk_cap: 16 };
        chunk_envelope_plaintext(&env, &cfg).chunks.remove(0)
    }

    /// Tokenize `text` into the given pieces (which must concatenate to it),
    /// append one generated token, and score every usable token at -1.0.
    fn scored(pieces: &[&str], generated: &str) -> ChunkPerplexity {
        let mut lens: Vec<u32> = pieces.iter().map(|p| p.chars().count() as u32).collect();
        lens.push(generated.chars().count() as u32);
        let usable = lens.len() - 1;
        ChunkPerplexity {
            sum_nll: usable as f64,
            tokens: usable as u64,
            tail_tokens: 0,
            logprobs: vec![-1.0; usable],
            token_char_lens: lens,
        }
    }

    // "assistant_message: hi there\n" then "tool_result: ok\n"
    fn two_event_chunk() -> TraceChunk {
        chunk_of(&[("assistant_message", "hi there"), ("tool_result", "ok")])
    }

    #[test]
    fn tokens_go_to_the_span_holding_their_first_char() {
        let chunk = two_event_chunk();
        let pieces = ["assistant", "_message", ":", " hi", " there", "\n", "tool", "_result", ":", " ok", "\n"];
        assert_eq!(pieces.concat(), chunk.text);
        let sums = attribute_chunk(&chunk, &scored(&pieces, " next")).unwrap();
        // " hi" starts on the prefix's trailing space, so it is Other; only
        // " there" starts inside the prose. Likewise " ok" is Other.
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 1);
        assert_eq!(sums.tokens[AuthorKind::ToolResult.index()], 0);
        // 11 prompt tokens, first dropped, generated one unattributed.
        assert_eq!(sums.tokens.iter().sum::<u64>(), 10);
    }

    #[test]
    fn the_generated_token_is_attributed_to_no_one() {
        let chunk = chunk_of(&[("assistant_message", "abc")]);
        let pieces = ["assistant_message: ", "a", "b", "c", "\n"];
        let sums = attribute_chunk(&chunk, &scored(&pieces, "zzz")).unwrap();
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 3);
        assert_eq!(sums.tokens[AuthorKind::Other.index()], 1);
    }

    #[test]
    fn lengths_that_do_not_tile_the_chunk_skip_it() {
        let chunk = chunk_of(&[("assistant_message", "naïve")]);
        // A multi-byte char split across two tokens decodes to two
        // replacement chars: one char too many.
        let pieces = ["assistant_message: ", "na", "\u{fffd}", "\u{fffd}", "ve", "\n"];
        assert!(attribute_chunk(&chunk, &scored(&pieces, "x")).is_none());
    }

    #[test]
    fn lengths_that_fall_short_skip_it() {
        let chunk = chunk_of(&[("assistant_message", "abcdef")]);
        assert!(attribute_chunk(&chunk, &scored(&["assistant_message: ", "abc"], "x")).is_none());
    }

    #[test]
    fn no_lengths_means_no_attribution() {
        let chunk = two_event_chunk();
        let mut s = scored(&["assistant_message: hi there\n", "tool_result: ok\n"], "x");
        s.token_char_lens.clear();
        assert!(attribute_chunk(&chunk, &s).is_none());
    }

    #[test]
    fn a_length_vector_not_one_longer_than_the_logprobs_is_rejected() {
        let chunk = two_event_chunk();
        let mut s = scored(&["assistant_message: hi there\n", "tool_result: ok\n"], "x");
        s.token_char_lens.push(1);
        assert!(attribute_chunk(&chunk, &s).is_none());
    }

    #[test]
    fn a_response_with_no_generated_token_still_attributes() {
        let chunk = chunk_of(&[("assistant_message", "abc")]);
        let mut s = scored(&["assistant_message: ", "a", "b", "c", "\n"], "");
        // Drop the generated token entirely: lens and logprobs shrink by one.
        s.token_char_lens.pop();
        s.logprobs.pop();
        s.tokens -= 1;
        let sums = attribute_chunk(&chunk, &s).unwrap();
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 3);
    }

    #[test]
    fn aggregate_reports_perplexity_per_author_and_coverage() {
        let a = chunk_of(&[("assistant_message", "abc")]);
        let sa = scored(&["assistant_message: ", "a", "b", "c", "\n"], "x"); // 5 usable
        let b = chunk_of(&[("tool_result", "naïve")]);
        let sb = scored(&["tool_result: ", "na", "\u{fffd}", "\u{fffd}", "ve", "\n"], "x"); // 6 usable, untiled
        let out = aggregate_author_perplexity(&[a, b], &[sa, sb]).unwrap();
        assert_eq!(out.agent_prose_tokens, 3);
        // Every usable token scored -1.0, so perplexity is e.
        assert_eq!(out.agent_prose_perplexity_micros, Some(2_718_281));
        assert_eq!(out.tool_result_tokens, 0);
        assert_eq!(out.tool_result_perplexity_micros, None);
        // 5 of 11 scored tokens sit in an attributed chunk.
        assert_eq!(out.attributed_token_fraction_micros, 454_545);
    }

    #[test]
    fn nothing_attributable_is_absent_not_zero() {
        let chunk = two_event_chunk();
        let mut s = scored(&["assistant_message: hi there\n", "tool_result: ok\n"], "x");
        s.token_char_lens.clear();
        assert_eq!(aggregate_author_perplexity(&[chunk], &[s]), None);
    }

    #[test]
    fn a_session_with_no_assistant_prose_reports_none_for_it() {
        let chunk = chunk_of(&[("user_message", "hello"), ("tool_result", "ok")]);
        let pieces = ["user_message: hello\n", "tool_result: ", "o", "k", "\n"];
        let out = aggregate_author_perplexity(&[chunk], &[scored(&pieces, "x")]).unwrap();
        assert_eq!(out.agent_prose_tokens, 0);
        assert_eq!(out.agent_prose_perplexity_micros, None);
        assert_eq!(out.tool_result_tokens, 2);
    }

    #[test]
    fn mismatched_slice_lengths_are_absent() {
        assert_eq!(aggregate_author_perplexity(&[two_event_chunk()], &[]), None);
    }
}
```

- [ ] **Step 3: Run to see them fail**

Run: `cargo test -p trace-commons-gate-enclave author_attribution`
Expected: compile errors — `attribute_chunk`, `aggregate_author_perplexity`, `AuthorSums` not found.

- [ ] **Step 4: Implement** — insert between the module doc and `mod tests`:

```rust
use trace_commons_gate_api::decision::AuthorPerplexity;

use crate::chunker::{AuthorKind, TraceChunk};
use crate::perplexity::ChunkPerplexity;

/// Per-author NLL and token counts for one chunk, indexed by
/// [`AuthorKind::index`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthorSums {
    pub nll: [f64; AuthorKind::COUNT],
    pub tokens: [u64; AuthorKind::COUNT],
}

/// Attribute each usable token to the span holding its first char.
///
/// `scored.token_char_lens[0]` is the dropped first token, so the cursor
/// starts after it. A token whose start cursor equals the chunk's char
/// count is the single generated token the request asks for
/// (`max_tokens: 1`): it must be last, and it belongs to no author.
///
/// `None` when lengths are absent, are not `logprobs.len() + 1` long, or do
/// not tile the chunk exactly.
pub fn attribute_chunk(chunk: &TraceChunk, scored: &ChunkPerplexity) -> Option<AuthorSums> {
    let lens = &scored.token_char_lens;
    let logprobs = &scored.logprobs;
    if logprobs.is_empty() || lens.len() != logprobs.len() + 1 {
        return None;
    }
    let chunk_chars: u64 = chunk.spans.iter().map(|s| s.len as u64).sum();
    let mut sums = AuthorSums { nll: [0.0; AuthorKind::COUNT], tokens: [0; AuthorKind::COUNT] };
    let mut cursor = lens[0] as u64;
    let mut span_idx = 0usize;
    for (i, lp) in logprobs.iter().enumerate() {
        if cursor > chunk_chars {
            return None;
        }
        if cursor == chunk_chars {
            // Only the final element may be the generated token.
            return (i == logprobs.len() - 1).then_some(sums);
        }
        while let Some(s) = chunk.spans.get(span_idx) {
            if cursor < s.start as u64 + s.len as u64 {
                break;
            }
            span_idx += 1;
        }
        let kind = chunk.spans.get(span_idx)?.kind.index();
        sums.nll[kind] -= *lp as f64;
        sums.tokens[kind] += 1;
        cursor += lens[i + 1] as u64;
    }
    // No generated token in the response: the prompt tokens must still end
    // exactly at the chunk's end.
    (cursor == chunk_chars).then_some(sums)
}

fn perplexity_micros(nll: f64, tokens: u64) -> Option<u64> {
    if tokens == 0 {
        return None;
    }
    let v = (nll / tokens as f64).exp() * 1_000_000.0;
    // Non-finite collapses to absent, never to a number that looks real.
    (v.is_finite() && v >= 0.0).then(|| if v >= u64::MAX as f64 { u64::MAX } else { v as u64 })
}

/// Whole-trace per-author perplexity over every attributable chunk.
/// `chunks` and `scored` are parallel, as the orchestrator produces them.
/// `None` when they are not, or when no chunk was attributable.
pub fn aggregate_author_perplexity(
    chunks: &[TraceChunk],
    scored: &[ChunkPerplexity],
) -> Option<AuthorPerplexity> {
    if chunks.len() != scored.len() {
        return None;
    }
    let total_tokens: u64 = scored.iter().map(|s| s.tokens).sum();
    let mut nll = [0.0f64; AuthorKind::COUNT];
    let mut tokens = [0u64; AuthorKind::COUNT];
    let mut attributed_tokens = 0u64;
    let mut any = false;
    for (chunk, s) in chunks.iter().zip(scored) {
        let Some(sums) = attribute_chunk(chunk, s) else { continue };
        any = true;
        attributed_tokens += s.tokens;
        for k in 0..AuthorKind::COUNT {
            nll[k] += sums.nll[k];
            tokens[k] += sums.tokens[k];
        }
    }
    if !any || total_tokens == 0 {
        return None;
    }
    let (prose, tool) = (AuthorKind::AgentProse.index(), AuthorKind::ToolResult.index());
    let fraction = (attributed_tokens as f64 / total_tokens as f64 * 1_000_000.0) as u64;
    Some(AuthorPerplexity {
        agent_prose_perplexity_micros: perplexity_micros(nll[prose], tokens[prose]),
        agent_prose_tokens: tokens[prose],
        tool_result_perplexity_micros: perplexity_micros(nll[tool], tokens[tool]),
        tool_result_tokens: tokens[tool],
        attributed_token_fraction_micros: fraction.min(1_000_000),
    })
}
```

Check how `crate::perplexity` re-exports `ChunkPerplexity` (`chunk_aggregate.rs` already does `use crate::perplexity::ChunkPerplexity;`) and match it.

- [ ] **Step 5: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-api -p trace-commons-gate-enclave`
Expected: PASS. If `aggregate_reports_perplexity_per_author_and_coverage` is off by one micro on `2_718_281`, the f32->f64 widening of `-1.0` is exact, so treat any mismatch as a real bug, not a rounding tolerance to loosen.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-gate-api crates/trace-commons-gate-enclave
git commit -m "Split chunk NLL by author, exact or not at all"
```

---

### Task 5: Carry it on the decision, and prove it decides nothing

**Files:**
- Modify: `crates/trace-commons-gate-api/src/decision.rs` — `GateDecision` (field after `qualifying_token_fraction_micros`, ~97) and `PerplexityOnlyOutcome` (~131)
- Modify: `crates/trace-commons-gate-enclave/src/orchestrator.rs` — `evaluate_perplexity_only` (~127) and the `GateDecision` literal (~264)
- Modify: every other literal of those two structs the compiler reports

**Interfaces:**
- Consumes: `aggregate_author_perplexity` (Task 4).
- Produces: `GateDecision::author_perplexity: Option<AuthorPerplexity>` and `PerplexityOnlyOutcome::author_perplexity: Option<AuthorPerplexity>`.

- [ ] **Step 1: Write the failing test** — in `orchestrator.rs`'s `mod tests`. First read the existing tests there to find how they build an orchestrator with a stub scorer (for example the one used by `chunk_scorer_error_fails_the_whole_evaluation`) and reuse that constructor. Add a stub scorer that can report lengths:

```rust
    /// Scores every char as one token at -1.0 and, when `with_lengths`,
    /// reports char-per-token lengths plus one generated token.
    struct CharScorer {
        with_lengths: bool,
    }

    impl PerplexityScorer for CharScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            let n = std::str::from_utf8(plaintext)?.chars().count() as u64;
            Ok(PerplexityResult {
                aggregate_perplexity_micros: 2_718_281,
                tail_fraction_micros: 0,
                tokens_scored: n,
            })
        }

        fn score_chunk(&self, chunk: &[u8]) -> anyhow::Result<ChunkPerplexity> {
            let n = std::str::from_utf8(chunk)?.chars().count();
            // n prompt tokens + 1 generated; the first prompt token is dropped.
            Ok(ChunkPerplexity {
                sum_nll: n as f64,
                tokens: n as u64,
                tail_tokens: 0,
                logprobs: vec![-1.0; n],
                token_char_lens: if self.with_lengths { vec![1; n + 1] } else { Vec::new() },
            })
        }
    }

    #[test]
    fn author_perplexity_is_recorded_and_decides_nothing() {
        let envelope = serde_json::to_vec(&serde_json::json!({"events": [
            {"event_type": "user_message", "redacted_content": "please fix the build"},
            {"event_type": "assistant_message", "redacted_content": "the linker flag was wrong"},
            {"event_type": "tool_result", "tool_name": "Bash", "redacted_content": "ok"},
        ]}))
        .unwrap();

        let with = orchestrator_with_scorer(CharScorer { with_lengths: true })
            .evaluate_perplexity_only(&envelope)
            .unwrap();
        let without = orchestrator_with_scorer(CharScorer { with_lengths: false })
            .evaluate_perplexity_only(&envelope)
            .unwrap();

        let ap = with.author_perplexity.expect("lengths were reported");
        assert_eq!(ap.agent_prose_tokens, "the linker flag was wrong".chars().count() as u64);
        assert_eq!(ap.tool_result_tokens, 2);
        assert_eq!(ap.attributed_token_fraction_micros, 1_000_000);
        assert_eq!(without.author_perplexity, None);

        // Shadow: every pre-existing field is identical either way.
        let strip = |mut o: PerplexityOnlyOutcome| {
            o.author_perplexity = None;
            o
        };
        assert_eq!(strip(with), strip(without));
    }
```

Add this helper beside `orch_with_floors` in the same test module; it is the construction `chunk_scorer_error_fails_the_whole_evaluation` already uses, with the scorer substituted:

```rust
    fn orchestrator_with_scorer(
        scorer: CharScorer,
    ) -> EnclaveGateOrchestrator<CharScorer, MockEmbedder, MockVectorIndex> {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.chunk_min_tokens = 1;
        EnclaveGateOrchestrator::new(scorer, MockEmbedder::new(), MockVectorIndex::new(), cfg)
    }
```

If `EnclaveGateOrchestrator` is not generic over its three parts in that shape, copy the return type from `orch_with_floors`'s signature instead. `PerplexityOnlyOutcome` derives `Debug, Clone, Copy, PartialEq, Eq` today, which is why `AuthorPerplexity` derives the same set: adding a non-`Copy`/non-`Eq` field would break that derive.

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p trace-commons-gate-enclave author_perplexity_is_recorded_and_decides_nothing`
Expected: compile error — no field `author_perplexity`.

- [ ] **Step 3: Implement.** In `decision.rs`, add to both structs, directly after `qualifying_token_fraction_micros`:

```rust
    /// Perplexity split by token author. Shadow mode: recorded, gates
    /// nothing. `None` when no chunk could be attributed.
    pub author_perplexity: Option<AuthorPerplexity>,
```

In `orchestrator.rs`, `evaluate_perplexity_only`: bind the chunk scores instead of discarding them and set the field.

```rust
        let (plan, chunk_scores, perp_agg) = self.chunk_and_score_perplexity(plaintext)?;
```
```rust
            author_perplexity: crate::author_attribution::aggregate_author_perplexity(
                &plan.chunks,
                &chunk_scores,
            ),
```

In `evaluate`, the `(plan, chunk_scores, perp_agg)` triple is already bound; add the same field to the `GateDecision` literal after `qualifying_token_fraction_micros`. `plan.chunks` and `chunk_scores` are parallel by construction in `chunk_and_score_perplexity` (one push per chunk, error aborts).

Add `author_perplexity: None,` to every other literal of either struct the compiler reports (mocks, reference gate, server tests).

- [ ] **Step 4: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-api -p trace-commons-gate-enclave && RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
Expected: PASS / clean. The server does not read the field yet.

- [ ] **Step 5: Commit**

```bash
git add crates
git commit -m "Record per-author perplexity on gate decisions in shadow mode"
```

---

### Task 6: Migration and storage row

**Files:**
- Create: `migrations/V73__trace_gate_decision_author_perplexity.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (migration shape test, beside `v54_adds_a_nullable_qualifying_mass_column` ~5800)
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` (`TraceGateDecisionRow`, after `qualifying_token_fraction_micros` ~1938; the ingest binary imports it as `StorageTraceGateDecisionRow`)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` — `stream_trace_gate_decisions_for_replay` (~5974), `insert_trace_gate_decision` (~6089), `insert_trace_gate_decision_with_chunk_entries` (~6142), `find_gate_decision_by_canonical_hash` (~6608)
- Test: `crates/trace-commons-server/tests/trace_corpus_pg_store.rs`

**Interfaces:**
- Produces on `TraceGateDecisionRow`: `agent_prose_perplexity_micros`, `agent_prose_tokens`, `tool_result_perplexity_micros`, `tool_result_tokens`, `attributed_token_fraction_micros`, each `Option<i64>`.

- [ ] **Step 0: Confirm the number.** Run `ls migrations | sort -V | tail -3`. If `V73` is taken, use the next free number here, in the test's `include_str!`, and in the commit. Numbering collisions have happened in this repo; do not guess.

- [ ] **Step 1: Write the failing migration test** in `postgres.rs`, after the V54 test:

```rust
    /// Shadow-mode columns: nullable and default-free, so a row written
    /// before the migration, or scored by a backend that reports no token
    /// lengths, reads as "not computed" rather than as a real zero.
    #[test]
    fn v73_adds_nullable_author_perplexity_columns() {
        const V73: &str = include_str!(
            "../../../../migrations/V73__trace_gate_decision_author_perplexity.sql"
        );
        for col in [
            "agent_prose_perplexity_micros",
            "agent_prose_tokens",
            "tool_result_perplexity_micros",
            "tool_result_tokens",
            "attributed_token_fraction_micros",
        ] {
            assert!(
                V73.contains(&format!("ADD COLUMN IF NOT EXISTS {col} BIGINT")),
                "V73 must add {col}"
            );
        }
        assert!(!V73.contains("NOT NULL") && !V73.contains("DEFAULT"));
    }
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p trace-commons-server v73_adds_nullable_author_perplexity_columns`
Expected: compile error, file not found.

- [ ] **Step 3: Write the migration**

```sql
-- Per-author perplexity, shadow mode.
--
-- Whole-trace perplexity is token-weighted, so in an agent session it is set
-- by tool output and pasted input. These columns record the same logprobs
-- split by who authored each token. Nothing reads them to decide anything.
-- See docs/superpowers/specs/2026-09-18-per-author-perplexity-shadow-design.md.
--
-- Nullable with no default on purpose: NULL is "not computed" -- a row
-- written before this migration, or scored by a backend that reports no
-- token lengths. A perplexity column is also NULL when that author has no
-- attributed tokens; its token column is then 0. Readers MUST NOT default
-- these, or calibration reads every unmeasured row as a real observation.
--
-- No RLS change: same table, same forced policies.

ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS agent_prose_perplexity_micros BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS agent_prose_tokens BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS tool_result_perplexity_micros BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS tool_result_tokens BIGINT;
ALTER TABLE trace_gate_decisions
    ADD COLUMN IF NOT EXISTS attributed_token_fraction_micros BIGINT;
```

- [ ] **Step 4: Add the row fields** in `trace_corpus_storage.rs`, after `qualifying_token_fraction_micros`:

```rust
    /// Per-author perplexity (migration V73). Shadow mode. All five are
    /// `None` when nothing was attributed -- pre-V73 rows and backends that
    /// report no token lengths. A `*_perplexity_micros` is also `None` when
    /// that author had no attributed tokens. Readers MUST NOT default any of
    /// them.
    pub agent_prose_perplexity_micros: Option<i64>,
    pub agent_prose_tokens: Option<i64>,
    pub tool_result_perplexity_micros: Option<i64>,
    pub tool_result_tokens: Option<i64>,
    pub attributed_token_fraction_micros: Option<i64>,
```

Add the five `None`s to every `TraceGateDecisionRow {` / `StorageTraceGateDecisionRow {` literal the compiler reports (the ingest binary, its tests file, and `tests/`, including `sample_gate_decision` in `tests/trace_corpus_pg_store.rs`). The ingest mapping becomes real in Task 7.

- [ ] **Step 5: Thread the pg store.** In both INSERT functions, add the five column names to the column list directly after `qualifying_token_fraction_micros`, add five placeholders continuing the existing `$n` numbering in the same position, and add `&decision.agent_prose_perplexity_micros, ... &decision.attributed_token_fraction_micros` to the params slice directly after `&decision.qualifying_token_fraction_micros`. Column order, placeholder order and param order must match position for position.

In `stream_trace_gate_decisions_for_replay`, add the five names to BOTH SELECT lists (~5994 and ~6014) after `qualifying_token_fraction_micros` and five `row.get("<name>")` lines in the row mapping (~6049); those reads are by name.

`find_gate_decision_by_canonical_hash` reads POSITIONALLY and carries a comment saying a mid-list insert silently re-points the indices above it. Append the five columns at the END of its SELECT list, after `d.qualifying_token_fraction_micros`, and read them as `row.get(24)` through `row.get(28)`. First confirm `qualifying_token_fraction_micros` is still `row.get(23)`; if it has moved, continue from its index + 1.

- [ ] **Step 6: Write the store roundtrip test** in `tests/trace_corpus_pg_store.rs`, directly after `pg_store_round_trips_prospective_gate_instrumentation` (~3507), which it mirrors:

```rust
#[tokio::test]
async fn pg_store_round_trips_author_perplexity_including_null() {
    let Some(backend) = postgres_backend().await else {
        return;
    };
    backend.run_migrations().await.expect("run migrations");

    let tenant_id = format!("pg-author-ppl-{}", Uuid::new_v4());
    let submission_id = Uuid::new_v4();
    backend
        .upsert_trace_submission(sample_submission(&tenant_id, submission_id))
        .await
        .expect("insert submission");

    let mut measured = sample_gate_decision(submission_id);
    measured.agent_prose_perplexity_micros = Some(4_540_000);
    measured.agent_prose_tokens = Some(397);
    measured.tool_result_perplexity_micros = None; // no tool-result tokens
    measured.tool_result_tokens = Some(0);
    measured.attributed_token_fraction_micros = Some(850_000);
    let measured_id = measured.decision_id;
    backend
        .insert_trace_gate_decision(&tenant_id, measured)
        .await
        .expect("insert measured gate decision");

    let mut unmeasured = sample_gate_decision(submission_id);
    let unmeasured_id = unmeasured.decision_id;
    unmeasured.decided_at = Utc::now() + chrono::Duration::seconds(1);
    backend
        .insert_trace_gate_decision(&tenant_id, unmeasured)
        .await
        .expect("insert unmeasured gate decision");

    let rows = backend
        .stream_trace_gate_decisions_for_replay(&tenant_id, 50, None)
        .await
        .expect("read back gate decisions");
    let got = rows.iter().find(|r| r.decision_id == measured_id).expect("measured row");
    assert_eq!(got.agent_prose_perplexity_micros, Some(4_540_000));
    assert_eq!(got.agent_prose_tokens, Some(397));
    assert_eq!(got.tool_result_perplexity_micros, None);
    assert_eq!(got.tool_result_tokens, Some(0), "a real zero must not read as NULL");
    assert_eq!(got.attributed_token_fraction_micros, Some(850_000));

    let got = rows.iter().find(|r| r.decision_id == unmeasured_id).expect("unmeasured row");
    assert_eq!(got.agent_prose_tokens, None, "unmeasured must stay NULL, not 0");
    assert_eq!(got.tool_result_tokens, None);
    assert_eq!(got.attributed_token_fraction_micros, None);

    cleanup_tenant(&backend, &tenant_id).await;
}
```

This test returns early, and prints `ok`, when `postgres_backend()` finds no database. A green line is therefore not evidence it ran. Confirm it executed against PostgreSQL by checking that the run is not instantaneous and that `TRACE_COMMONS_TEST_DATABASE_URL` (or whatever `postgres_test_config` at the top of that file reads) was set; if it was not, report the store as UNVERIFIED.

`find_gate_decision_by_canonical_hash` reads positionally and this test does not reach it. After adding `row.get(24)`..`row.get(28)`, run the existing test near line 3886 of the same file that exercises that lookup; a wrong index there fails as a type or column-count error at runtime, not at compile time.

- [ ] **Step 7: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server v73_adds_nullable_author_perplexity_columns` — expected PASS.
Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --test trace_corpus_storage_contract` — expected PASS.
Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store` (add `-- --ignored` if that is how the neighbor runs) — requires PostgreSQL. Expected PASS. If no PostgreSQL is reachable, say so in the task report; do not report the store as verified.

- [ ] **Step 8: Commit**

```bash
git add migrations crates/trace-commons-server
git commit -m "Persist per-author perplexity on trace_gate_decisions"
```

---

### Task 7: Wire ingest

**Files:**
- Modify: `crates/trace-commons-server/src/trace_gate_service.rs` — outcome struct (~121), deterministic service literal (~410), enclave mapping (~752), and `PerplexityOnlyGateOutcome` (~209) with its mapping
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — row mapping (~51175), short-circuit literal (~52730), re-hydration literal (~53001)
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`

**Interfaces:**
- Consumes: `GateDecision::author_perplexity`, `PerplexityOnlyOutcome::author_perplexity` (Task 5); the five row fields (Task 6).
- Produces: `author_perplexity: Option<AuthorPerplexity>` on the gate-service decision outcome and on `PerplexityOnlyGateOutcome`; a free function in `trace_gate_service.rs`:
  ```rust
  pub fn author_perplexity_columns(ap: Option<&AuthorPerplexity>) -> [Option<i64>; 5]
  ```
  ordered `[agent_prose_perplexity_micros, agent_prose_tokens, tool_result_perplexity_micros, tool_result_tokens, attributed_token_fraction_micros]`.

- [ ] **Step 1: Write the failing test** in `trace_gate_service.rs`'s `mod tests`:

```rust
    #[test]
    fn author_perplexity_columns_keep_absent_and_zero_apart() {
        assert_eq!(author_perplexity_columns(None), [None; 5]);
        let ap = AuthorPerplexity {
            agent_prose_perplexity_micros: Some(4_540_000),
            agent_prose_tokens: 397,
            tool_result_perplexity_micros: None,
            tool_result_tokens: 0,
            attributed_token_fraction_micros: 850_000,
        };
        assert_eq!(
            author_perplexity_columns(Some(&ap)),
            [Some(4_540_000), Some(397), None, Some(0), Some(850_000)]
        );
    }

    #[test]
    fn author_perplexity_columns_saturate_instead_of_wrapping() {
        let ap = AuthorPerplexity {
            agent_prose_perplexity_micros: Some(u64::MAX),
            agent_prose_tokens: u64::MAX,
            tool_result_perplexity_micros: None,
            tool_result_tokens: 0,
            attributed_token_fraction_micros: 0,
        };
        let cols = author_perplexity_columns(Some(&ap));
        assert_eq!(cols[0], Some(i64::MAX));
        assert_eq!(cols[1], Some(i64::MAX));
    }
```

- [ ] **Step 2: Run to see it fail**

Run: `cargo test -p trace-commons-server author_perplexity_columns`
Expected: compile error — function not found.

- [ ] **Step 3: Implement.** In `trace_gate_service.rs`:

```rust
use trace_commons_gate_api::decision::AuthorPerplexity;

/// Fan `AuthorPerplexity` out into its five nullable columns, in migration
/// order. Absent stays `None` throughout; a real zero token count stays
/// `Some(0)`. Saturates rather than wraps, as the other micros mappings do.
pub fn author_perplexity_columns(ap: Option<&AuthorPerplexity>) -> [Option<i64>; 5] {
    let Some(ap) = ap else { return [None; 5] };
    let sat = |v: u64| i64::try_from(v).unwrap_or(i64::MAX);
    [
        ap.agent_prose_perplexity_micros.map(sat),
        Some(sat(ap.agent_prose_tokens)),
        ap.tool_result_perplexity_micros.map(sat),
        Some(sat(ap.tool_result_tokens)),
        Some(sat(ap.attributed_token_fraction_micros)),
    ]
}
```

(If the import already exists in another form, extend it.) Add to the decision outcome struct, after `qualifying_token_fraction_micros`:

```rust
    /// Perplexity split by token author. Shadow mode. `None` from a
    /// deterministic service and from any backend that reports no token
    /// lengths -- unknown, not zero.
    pub author_perplexity: Option<AuthorPerplexity>,
```

Set it to `None` in the deterministic-service literal (~410) and to `decision.author_perplexity` in the enclave mapping (~752). Add the same field to `PerplexityOnlyGateOutcome` and copy it across wherever that struct is built from `PerplexityOnlyOutcome`.

In `trace-commons-ingest.rs` at the row mapping (~51175), after the `qualifying_token_fraction_micros` entry:

```rust
        // Shadow mode, as above: recorded, read by nothing that decides.
        agent_prose_perplexity_micros: author_cols[0],
        agent_prose_tokens: author_cols[1],
        tool_result_perplexity_micros: author_cols[2],
        tool_result_tokens: author_cols[3],
        attributed_token_fraction_micros: author_cols[4],
```

with, before the struct literal:

```rust
    let author_cols = trace_commons_server::trace_gate_service::author_perplexity_columns(
        decision.author_perplexity.as_ref(),
    );
```

(Match the path style the file already uses for `trace_gate_service` items.) At the short-circuit literal (~52730) and the re-hydration literal (~53001), the five fields stay `None` as Task 6 left them; extend the existing comment there to say the per-author values are absent for the same reason as the qualifying mass.

- [ ] **Step 4: Add the ingest-level test** in `trace_commons_ingest_internal/tests.rs`. Find the test that asserts `qualifying_token_fraction_micros` reaches the persisted row (`git grep -n 'qualifying_token_fraction_micros' crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs`) and add, in that same test or a sibling built the same way, that a decision carrying `author_perplexity: Some(..)` persists the five values and one carrying `None` persists five `None`s. Use the values from Step 1 so the expected array is the same literal.

- [ ] **Step 5: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server author_perplexity`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-server
git commit -m "Wire per-author perplexity from the gate service to the decision row"
```

---

### Task 8: PR 1 verification

- [ ] **Step 1: Format.** `cargo fmt --all`, then `git diff --stat`. The repo is not rustfmt-clean, so a formatter can rewrite whole files: revert any hunk outside the lines this plan touched (`git checkout -p`), keeping only your own.

- [ ] **Step 2: Every configuration CI builds**

```bash
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins --features near-ai-scorer
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins --features local-gpu-models
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy -p trace-commons-server --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-api -p trace-commons-gate-enclave
RUSTFLAGS="-D warnings" cargo test -p trace-commons-gate-enclave --features near-ai-scorer
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server
cargo test -p trace-commons-server --test license_boundary
```

Expected: all clean. `local-gpu-models` may fail to LINK without CUDA; `cargo check` is what CI runs and must pass. Compare the `cargo test -p trace-commons-server` failure count against a baseline taken on `origin/main` before Task 1 — report both numbers; do not report "tests pass" from a filtered run.

- [ ] **Step 3: The gate version hash did not move.** `git diff origin/main -- crates/trace-commons-server/src/bin/trace-commons-ingest.rs | grep -n 'compute_gate_version_hash\|CANONICAL_RENDER_VERSION\|CHUNK_SELECTION_ALGORITHM'` must print nothing, and `git diff origin/main -- crates/trace-commons-gate-enclave/src/chunker.rs | grep -E '^[-+].*(events\.v1|stride_endpoint_inclusive)'` must print nothing.

- [ ] **Step 4: Headers.** `head -2 crates/trace-commons-gate-enclave/src/author_attribution.rs` shows the copyright and SPDX lines.

- [ ] **Step 5: Open PR 1** against `main` on `TraceCommons/trace-commons` using `.github/pull_request_template.md`. The body states: shadow mode, no decision changes, hash unchanged, which checks ran and their output, and whether the pg store test ran against a real PostgreSQL.

---

### Task 9 (PR 2): Author-only backfill through the rescore route

**Files:**
- Modify: `crates/trace-commons-server/src/trace_corpus_storage.rs` — new trait method beside `update_trace_gate_decision_perplexity` (~3039)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` — impl beside the existing one (~6260)
- Modify: `crates/trace-commons-server/src/bin/trace-commons-ingest.rs` — `rescore_perplexity_one` (~51650), `run_rescore_perplexity_pass` (~51692), `rescore_perplexity_handler` (~51728) and its request type
- Test: `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs` (the in-memory store there implements the storage trait at ~67369)

**Interfaces:**
- Consumes: `PerplexityOnlyGateOutcome::author_perplexity`, `author_perplexity_columns` (Task 7).
- Produces:
  ```rust
  async fn update_trace_gate_decision_author_perplexity(
      &self, tenant_id: &str, submission_id: Uuid, columns: [Option<i64>; 5],
  ) -> Result<(), DatabaseError>;
  ```
  and a request field `author_only: bool` (`#[serde(default)]`) on the rescore route.

Why a second method: `update_trace_gate_decision_perplexity` rewrites `perplexity_micros`, `peak_perplexity_micros` and `perplexity_passed`. The pilot's scorer model has changed since stored rows were written, so a backfill through it would re-derive `perplexity_passed` under a different model and silently rewrite gating history.

- [ ] **Step 1: Write the failing tests** in `tests.rs`, directly after `rescore_perplexity_pass_updates_only_perplexity_leaves_novelty_untouched` (~70640).

That test's first half builds everything needed: two tempdirs, `fixture_gate_worker_artifact_store`, `seed_perplexity_driver_test_db(&artifact_store, "tenant-a", 3)`, the `test_state_with_configured_artifact_store_policies_and_export_guardrails(...)` state, one `run_perplexity_score_driver_tick` to create decisions, then `list_submissions_with_gate_decision(100)` and a `snapshot` of `gate_decision_for` rows. Move exactly that span, verbatim, into a helper and make the existing test call it, so all three tests share one setup:

```rust
struct RescoreFixture {
    // Held so the directories outlive the state.
    _temp: tempfile::TempDir,
    _artifact_temp: tempfile::TempDir,
    state: Arc<AppState>,
    db: Arc<PerplexityDriverTestDb>,
    work_items: Vec<GateWorkItem>,
    snapshot: Vec<StorageTraceGateDecisionRow>,
}

async fn rescore_fixture() -> RescoreFixture { /* the moved span */ }
```

Use the concrete types the moved code already has for `state`, `db` and `work_items` (`seed_perplexity_driver_test_db`'s return type, and the element type of `list_submissions_with_gate_decision`); the names above are what to look for, not new types.

The state's gate service is the deterministic one, which reports no token lengths, so its outcome carries `author_perplexity: None`. The tests therefore plant sentinels and check what the pass does to them:

```rust
const AUTHOR_SENTINEL: [Option<i64>; 5] = [Some(1), Some(1), Some(1), Some(1), Some(1)];

fn author_columns_of(row: &StorageTraceGateDecisionRow) -> [Option<i64>; 5] {
    [
        row.agent_prose_perplexity_micros,
        row.agent_prose_tokens,
        row.tool_result_perplexity_micros,
        row.tool_result_tokens,
        row.attributed_token_fraction_micros,
    ]
}

#[tokio::test]
async fn author_only_rescore_leaves_whole_trace_perplexity_untouched() {
    let fx = rescore_fixture().await;
    for item in &fx.work_items {
        // Corrupt the whole-trace columns so "untouched" is observable.
        fx.db
            .update_trace_gate_decision_perplexity(&item.tenant_id, item.submission_id, 0, Some(0), false)
            .await
            .expect("corrupt update succeeds");
        fx.db
            .update_trace_gate_decision_author_perplexity(&item.tenant_id, item.submission_id, AUTHOR_SENTINEL)
            .await
            .expect("sentinel update succeeds");
    }

    let summary = run_rescore_perplexity_pass(fx.state.clone(), None, true)
        .await
        .expect("author-only pass succeeds");
    assert_eq!(summary.rescored, 3, "{summary:?}");
    assert_eq!(summary.failed, 0, "{summary:?}");

    for item in &fx.work_items {
        let after = fx.db.gate_decision_for(&item.tenant_id, item.submission_id).expect("row");
        assert_eq!(after.perplexity_micros, 0, "author_only must not rewrite perplexity");
        assert_eq!(after.peak_perplexity_micros, Some(0));
        assert!(!after.perplexity_passed, "author_only must not re-derive the pass flag");
        // The pass wrote its own outcome over the sentinel: this service
        // reports no lengths, so that outcome is absent.
        assert_eq!(author_columns_of(&after), [None; 5]);
    }
}

#[tokio::test]
async fn the_default_rescore_still_restores_whole_trace_perplexity() {
    let fx = rescore_fixture().await;
    for item in &fx.work_items {
        fx.db
            .update_trace_gate_decision_perplexity(&item.tenant_id, item.submission_id, 0, Some(0), false)
            .await
            .expect("corrupt update succeeds");
        fx.db
            .update_trace_gate_decision_author_perplexity(&item.tenant_id, item.submission_id, AUTHOR_SENTINEL)
            .await
            .expect("sentinel update succeeds");
    }

    run_rescore_perplexity_pass(fx.state.clone(), None, false)
        .await
        .expect("full pass succeeds");

    for original in &fx.snapshot {
        let after = fx
            .db
            .gate_decision_for_decision_id(original)
            .expect("row");
        assert_eq!(after.perplexity_micros, original.perplexity_micros);
        assert_eq!(after.perplexity_passed, original.perplexity_passed);
        // A full rescore is a superset: it writes the author columns too.
        assert_eq!(author_columns_of(&after), [None; 5]);
    }
}
```

In the second test, look the row up the same way the existing test's final loop does (it iterates `snapshot` and re-reads each row; copy that lookup in place of `gate_decision_for_decision_id`, which is a stand-in for it). Update the existing test's call to `run_rescore_perplexity_pass(state.clone(), None, false)`.

- [ ] **Step 2: Run to see them fail**

Run: `cargo test -p trace-commons-server author_only_rescore`
Expected: compile error — wrong argument count / method not found.

- [ ] **Step 3: Storage trait method** in `trace_corpus_storage.rs`, mirroring the existing default (log-once warning, no-op) with its own `static WARNED`:

```rust
    /// Write ONLY the five per-author perplexity columns on the latest
    /// decision row for `submission_id`. Every other column -- including
    /// `perplexity_micros`, `peak_perplexity_micros` and
    /// `perplexity_passed` -- is left untouched, so a backfill scored by a
    /// different model than the row was gated under cannot rewrite gating
    /// history. Implementations MUST scope the update by `tenant_id`.
    async fn update_trace_gate_decision_author_perplexity(
        &self,
        _tenant_id: &str,
        _submission_id: Uuid,
        _columns: [Option<i64>; 5],
    ) -> Result<(), DatabaseError> {
        static WARNED: std::sync::Once = std::sync::Once::new();
        WARNED.call_once(|| {
            tracing::warn!(
                error_class = "AuthorPerplexityUpdateUnsupported",
                "storage backend does not persist per-author perplexity updates"
            );
        });
        Ok(())
    }
```

- [ ] **Step 4: pg impl** in `trace_corpus_pg.rs`, after `update_trace_gate_decision_perplexity`, same transaction and latest-row selection:

```rust
    async fn update_trace_gate_decision_author_perplexity(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
        columns: [Option<i64>; 5],
    ) -> Result<(), DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Latest decision row only, for the same reason as
        // `update_trace_gate_decision_perplexity`: a submission can own
        // several rows and older ones carry an older gate version stamp.
        tx.execute(
            "UPDATE trace_gate_decisions
                SET agent_prose_perplexity_micros = $3,
                    agent_prose_tokens = $4,
                    tool_result_perplexity_micros = $5,
                    tool_result_tokens = $6,
                    attributed_token_fraction_micros = $7
             WHERE tenant_id = $1 AND decision_id = (
                 SELECT decision_id FROM trace_gate_decisions
                  WHERE tenant_id = $1 AND submission_id = $2
                  ORDER BY decided_at DESC LIMIT 1)",
            &[&tenant_id, &submission_id, &columns[0], &columns[1], &columns[2], &columns[3], &columns[4]],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }
```

Implement it in the test file's in-memory store too, setting the five fields on the latest row for the submission, the way its `update_trace_gate_decision_perplexity` picks its row.

- [ ] **Step 5: Route.** Add `#[serde(default)] author_only: bool` to the rescore request type with the doc comment "Backfill the per-author perplexity columns only; never touch whole-trace perplexity or `perplexity_passed`." Thread `author_only` through `run_rescore_perplexity_pass(state: Arc<AppState>, limit: Option<i64>, author_only: bool)` into `rescore_perplexity_one(state, item, author_only)`, whose tail becomes:

```rust
    let author_cols = trace_commons_server::trace_gate_service::author_perplexity_columns(
        outcome.author_perplexity.as_ref(),
    );
    if !author_only {
        db.update_trace_gate_decision_perplexity(
            &item.tenant_id,
            item.submission_id,
            perplexity_micros,
            peak_perplexity_micros,
            outcome.perplexity_passed,
        )
        .await?;
    }
    db.update_trace_gate_decision_author_perplexity(&item.tenant_id, item.submission_id, author_cols)
        .await?;
```

Keep the existing hash-only log line; add `author_only` to it as a plain bool field.

- [ ] **Step 6: Run**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server rescore`
Expected: PASS, including the two pre-existing rescore tests unchanged.

- [ ] **Step 7: Commit**

```bash
git add crates/trace-commons-server
git commit -m "Backfill per-author perplexity without rewriting gating history"
```

---

### Task 10 (PR 2): Runbook and verification

**Files:**
- Modify: `docs/operator/perplexity-scoring-driver.md`

- [ ] **Step 1: Add a section** "Backfilling per-author perplexity" stating: what the five columns are and that they gate nothing; that the backfill MUST be run with `"author_only": true`; why (the scorer model has changed since stored rows were gated, and a full rescore re-derives `perplexity_passed`); that before running it the operator confirms which model and base URL the running ingest process actually uses (pilot config lives in the process, not in env files) and that the endpoint answers; that backfilled values are self-consistent per row but must not be compared against whole-trace values scored under an older model; and the calibration questions the backfill exists to answer — spread of `agent_prose_perplexity_micros`, the share of traces with under ~200 `agent_prose_tokens`, the distribution of `attributed_token_fraction_micros`, agreement with human labels. Read the file's existing sections first and match their heading style and request examples; take the route's exact request shape from `rescore_perplexity_handler`, not from memory.

- [ ] **Step 2: Repeat Task 8 Steps 1-2** on this branch, plus `cargo test -p trace-commons-server --test trace_corpus_pg_store` for the new UPDATE if PostgreSQL is reachable.

- [ ] **Step 3: Commit and open PR 2** against `main`, stacked on PR 1. Do not merge PR 2's base out from under it: merge top-down or retarget before deleting PR 1's branch.

```bash
git add docs/operator/perplexity-scoring-driver.md
git commit -m "Document the author-only perplexity backfill"
```
