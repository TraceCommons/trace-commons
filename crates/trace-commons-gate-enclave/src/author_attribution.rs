// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Split a chunk's per-token NLL by who authored each token.
//!
//! Pure: no I/O, no logging. Exact-or-skip: a chunk whose token lengths do
//! not tile its chars exactly yields `None` and contributes nothing, so a
//! mis-decoded token can shrink coverage but can never mis-attribute.

use trace_commons_gate_api::decision::AuthorPerplexity;

use crate::chunker::{AuthorKind, AuthorSpan, TraceChunk};
use crate::perplexity::ChunkPerplexity;

/// Per-author NLL and token counts for one chunk, indexed by
/// [`AuthorKind::index`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AuthorSums {
    pub nll: [f64; AuthorKind::COUNT],
    pub tokens: [u64; AuthorKind::COUNT],
}

/// Attribute each usable token to the author covering most of its chars;
/// a tie goes to the author rather than to `Other`.
///
/// Not "the span holding its first char": BPE tokens carry their leading
/// space and every rendered prefix ends in `": "`, so the first content
/// token of every event starts on a prefix char, and a first-char rule
/// would hand the first word of every message to `Other`.
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
    let mut sums = AuthorSums {
        nll: [0.0; AuthorKind::COUNT],
        tokens: [0; AuthorKind::COUNT],
    };
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
        let end = cursor + lens[i + 1] as u64;
        let kind = majority_kind(&chunk.spans[span_idx..], cursor, end)?;
        sums.nll[kind] -= *lp as f64;
        sums.tokens[kind] += 1;
        cursor = end;
    }
    // No generated token in the response: the prompt tokens must still end
    // exactly at the chunk's end.
    (cursor == chunk_chars).then_some(sums)
}

/// The author kind covering the most chars of `[from, to)`, as an
/// [`AuthorKind::index`]. `spans` must start at the span containing `from`.
/// An author beats `Other` on a tie; between the two authors, the earlier
/// one wins. A zero-length token takes the kind at `from`. `None` only when
/// `from` lies outside every span.
fn majority_kind(spans: &[AuthorSpan], from: u64, to: u64) -> Option<usize> {
    let first = spans.first()?.kind;
    let mut covered = [0u64; AuthorKind::COUNT];
    for s in spans {
        let (s_from, s_to) = (s.start as u64, s.start as u64 + s.len as u64);
        if s_from >= to {
            break;
        }
        covered[s.kind.index()] += s_to.min(to).saturating_sub(s_from.max(from));
    }
    let mut best = first;
    for kind in [
        AuthorKind::AgentProse,
        AuthorKind::ToolResult,
        AuthorKind::Other,
    ] {
        let (c, b) = (covered[kind.index()], covered[best.index()]);
        if c > b || (c == b && c > 0 && best == AuthorKind::Other) {
            best = kind;
        }
    }
    Some(best.index())
}

fn perplexity_micros(nll: f64, tokens: u64) -> Option<u64> {
    if tokens == 0 {
        return None;
    }
    let v = (nll / tokens as f64).exp() * 1_000_000.0;
    // Non-finite collapses to absent, never to a number that looks real.
    // `f64 as u64` saturates, so an enormous value pins at u64::MAX.
    (v.is_finite() && v >= 0.0).then_some(v as u64)
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
        let Some(sums) = attribute_chunk(chunk, s) else {
            continue;
        };
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
    let (prose, tool) = (
        AuthorKind::AgentProse.index(),
        AuthorKind::ToolResult.index(),
    );
    let fraction = (attributed_tokens as f64 / total_tokens as f64 * 1_000_000.0) as u64;
    Some(AuthorPerplexity {
        agent_prose_perplexity_micros: perplexity_micros(nll[prose], tokens[prose]),
        agent_prose_tokens: tokens[prose],
        tool_result_perplexity_micros: perplexity_micros(nll[tool], tokens[tool]),
        tool_result_tokens: tokens[tool],
        attributed_token_fraction_micros: fraction.min(1_000_000),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chunker::{ChunkerConfig, chunk_envelope_plaintext};

    fn chunk_of(events: &[(&str, &str)]) -> TraceChunk {
        let events: Vec<serde_json::Value> = events
            .iter()
            .map(|(ty, c)| serde_json::json!({"event_type": ty, "redacted_content": c}))
            .collect();
        let env = serde_json::to_vec(&serde_json::json!({ "events": events })).unwrap();
        let cfg = ChunkerConfig {
            target_tokens: 2048,
            max_tokens: 3072,
            chunk_cap: 16,
        };
        chunk_envelope_plaintext(&env, &cfg).chunks.remove(0)
    }

    /// Tokenize a chunk into the given pieces (which must concatenate to its
    /// text), append one generated token, and score every usable token at
    /// -1.0.
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
    fn tokens_go_to_the_author_covering_most_of_their_chars() {
        let chunk = two_event_chunk();
        let pieces = [
            "assistant",
            "_message",
            ":",
            " hi",
            " there",
            "\n",
            "tool",
            "_result",
            ":",
            " ok",
            "\n",
        ];
        assert_eq!(pieces.concat(), chunk.text);
        let sums = attribute_chunk(&chunk, &scored(&pieces, " next")).unwrap();
        // BPE tokens carry their leading space, and every prefix ends in
        // ": ", so the first content token of every event starts on a
        // prefix char. " hi" is one prefix char and two prose chars: prose.
        // A first-char rule would hand the first word of every message to
        // Other.
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 2);
        assert_eq!(sums.tokens[AuthorKind::ToolResult.index()], 1);
        // 11 prompt tokens, first dropped, generated one unattributed.
        assert_eq!(sums.tokens.iter().sum::<u64>(), 10);
    }

    #[test]
    fn a_token_split_evenly_with_other_goes_to_the_author() {
        let chunk = chunk_of(&[("assistant_message", "ok.")]);
        // ".\n" is one prose char and the event's trailing newline.
        let pieces = ["assistant_message:", " ok", ".\n"];
        assert_eq!(pieces.concat(), chunk.text);
        let sums = attribute_chunk(&chunk, &scored(&pieces, "x")).unwrap();
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 2);
        assert_eq!(sums.tokens[AuthorKind::Other.index()], 0);
    }

    #[test]
    fn a_token_mostly_in_the_scaffold_stays_other() {
        let chunk = chunk_of(&[("assistant_message", "a")]);
        // ": a" is two prefix chars and one prose char.
        let pieces = ["assistant_message", ": a", "\n"];
        assert_eq!(pieces.concat(), chunk.text);
        let sums = attribute_chunk(&chunk, &scored(&pieces, "x")).unwrap();
        assert_eq!(sums.tokens[AuthorKind::AgentProse.index()], 0);
        assert_eq!(sums.tokens[AuthorKind::Other.index()], 2);
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
        let pieces = [
            "assistant_message: ",
            "na",
            "\u{fffd}",
            "\u{fffd}",
            "ve",
            "\n",
        ];
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
        // 6 usable, untiled
        let sb = scored(
            &["tool_result: ", "na", "\u{fffd}", "\u{fffd}", "ve", "\n"],
            "x",
        );
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
