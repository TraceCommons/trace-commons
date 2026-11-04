// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Embedding trait + deterministic mock.

use sha2::{Digest, Sha256};
pub use trace_commons_gate_api::{Embedder, MOCK_EMBEDDING_DIM};

/// Sub-window size for chunk embeddings, in tokens. Matches the fastembed
/// model family's ~512-token context; anything longer is silently truncated
/// by the model, which is exactly the bug this helper fixes.
pub const EMBED_SUB_WINDOW_TOKENS: usize = 512;

/// The embedder refused the input itself, not because a dependency failed:
/// the same input fails the same way on every attempt. Raised for text that
/// is empty or whitespace only, which the fastembed tokenizer cannot embed.
/// Carried as a typed error so [`classify_embedder_error`] finds it through
/// any `context` wrapping, without reading message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderInputRejected {
    EmptyInput,
}

impl std::fmt::Display for EmbedderInputRejected {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyInput => f.write_str("EmbedderEmptyInput: empty plaintext rejected"),
        }
    }
}

impl std::error::Error for EmbedderInputRejected {}

/// Refuses text an embedder cannot embed: empty or whitespace only. The
/// fastembed embedder calls this before the model, and
/// [`embed_chunk_mean_pooled`] uses the same test to skip sub-windows that
/// hold no text.
pub fn reject_unembeddable_input(text: &str) -> Result<(), EmbedderInputRejected> {
    if text.trim().is_empty() {
        return Err(EmbedderInputRejected::EmptyInput);
    }
    Ok(())
}

/// Whose fault an embedder failure is. Only `InputRejected` is
/// deterministic for the input; everything else is treated as the
/// dependency's, so an outage stays retryable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EmbedderFailureClass {
    InputRejected,
    Dependency,
}

/// A classified embedder failure. `reason_label` is a static label, safe
/// to log; it never carries input text or an error message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EmbedderFailure {
    pub class: EmbedderFailureClass,
    pub reason_label: &'static str,
}

pub const EMBEDDER_REASON_EMPTY_INPUT: &str = "empty_input";
pub const EMBEDDER_REASON_NON_FINITE_VECTOR: &str = "non_finite_vector";
pub const EMBEDDER_REASON_DEGENERATE_VECTOR: &str = "degenerate_vector";
pub const EMBEDDER_REASON_DIM_MISMATCH: &str = "dim_mismatch";
pub const EMBEDDER_REASON_INFERENCE_FAILED: &str = "inference_failed";
pub const EMBEDDER_REASON_UNAVAILABLE: &str = "unavailable";
pub const EMBEDDER_REASON_UNCLASSIFIED: &str = "unclassified";

/// The static label prefixes this crate's embedders raise, most specific
/// first, mapped to the reason they log under. Matched against the start of
/// each message in the error chain; the messages themselves are never
/// logged, since a few interpolate values.
const EMBEDDER_DEPENDENCY_REASONS: &[(&str, &str)] = &[
    ("EmbedderNonFiniteVector", EMBEDDER_REASON_NON_FINITE_VECTOR),
    (
        "EmbedderDegenerateVector",
        EMBEDDER_REASON_DEGENERATE_VECTOR,
    ),
    ("EmbedderDimMismatch", EMBEDDER_REASON_DIM_MISMATCH),
    ("EmbedderSubWindowDimMismatch", EMBEDDER_REASON_DIM_MISMATCH),
    (
        "EmbedderInferenceReturnedNoVectors",
        EMBEDDER_REASON_INFERENCE_FAILED,
    ),
    ("EmbedderInferenceFailed", EMBEDDER_REASON_INFERENCE_FAILED),
    ("EmbedderMutexPoisoned", EMBEDDER_REASON_UNAVAILABLE),
];

/// Classifies an error from [`Embedder::embed`] or
/// [`embed_chunk_mean_pooled`]. An [`EmbedderInputRejected`] anywhere in the
/// chain is the input's fault; anything else is the dependency's, with the
/// most specific known label as its reason, or `unclassified`.
///
/// Non-finite and degenerate vectors are deliberately the dependency's:
/// a broken model produces them for every input, and failing every run for
/// good on a model fault would be far worse than retrying.
pub fn classify_embedder_error(error: &anyhow::Error) -> EmbedderFailure {
    if error
        .chain()
        .any(|cause| cause.downcast_ref::<EmbedderInputRejected>().is_some())
    {
        return EmbedderFailure {
            class: EmbedderFailureClass::InputRejected,
            reason_label: EMBEDDER_REASON_EMPTY_INPUT,
        };
    }
    let messages: Vec<String> = error.chain().map(|cause| cause.to_string()).collect();
    let reason_label = EMBEDDER_DEPENDENCY_REASONS
        .iter()
        .find(|(prefix, _)| messages.iter().any(|m| m.starts_with(prefix)))
        .map(|(_, reason)| *reason)
        .unwrap_or(EMBEDDER_REASON_UNCLASSIFIED);
    EmbedderFailure {
        class: EmbedderFailureClass::Dependency,
        reason_label,
    }
}

/// Embed a chunk (~2K tokens) with no truncation: split it into ≤512-token
/// (char-proxy) sub-windows, embed each, mean-pool, and L2-renormalize so
/// cosine similarity stays a dot product downstream. A chunk that fits in
/// one sub-window embeds identically to a direct `embed` call.
///
/// The windows are cut by char count, blind to content, so one can hold
/// only whitespace -- a chunk ending in `\n` one char past a window
/// boundary leaves a final window of just `\n`. Such a window carries no
/// text and the fastembed embedder rejects it, so it is skipped and the mean
/// is over the windows that were embedded. A long chunk with no text at all
/// is refused as [`EmbedderInputRejected::EmptyInput`]. No embedding that
/// succeeded before this rule changes: every input it affects used to fail.
pub fn embed_chunk_mean_pooled<E: Embedder + ?Sized>(
    embedder: &E,
    chunk_text: &str,
) -> anyhow::Result<Vec<f32>> {
    let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
    let char_count = chunk_text.chars().count();
    if char_count <= window_chars {
        return embedder.embed(chunk_text.as_bytes());
    }

    // Char-boundary-safe fixed windows.
    let mut windows: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut count = 0usize;
    for ch in chunk_text.chars() {
        current.push(ch);
        count += 1;
        if count == window_chars {
            windows.push(std::mem::take(&mut current));
            count = 0;
        }
    }
    if !current.is_empty() {
        windows.push(current);
    }
    windows.retain(|w| reject_unembeddable_input(w).is_ok());
    if windows.is_empty() {
        return Err(EmbedderInputRejected::EmptyInput.into());
    }

    let mut sum: Vec<f32> = Vec::new();
    for w in &windows {
        let v = embedder.embed(w.as_bytes())?;
        if sum.is_empty() {
            sum = vec![0.0; v.len()];
        }
        anyhow::ensure!(v.len() == sum.len(), "EmbedderSubWindowDimMismatch");
        for (s, x) in sum.iter_mut().zip(v.iter()) {
            *s += x;
        }
    }
    let n = windows.len() as f32;
    for s in &mut sum {
        *s /= n;
    }
    let norm: f32 = sum.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        for s in &mut sum {
            *s /= norm;
        }
    }
    Ok(sum)
}

/// Deterministic mock embedder.
///
/// The output is a 256-dimensional L2-normalized `Vec<f32>` whose entries come
/// from SHAKE-style expansion of `sha256(plaintext)`. It is **not** a real
/// embedding — it just satisfies the contract of "stable test vectors".
#[derive(Debug, Default, Clone)]
pub struct MockEmbedder;

impl MockEmbedder {
    pub fn new() -> Self {
        Self
    }
}

impl Embedder for MockEmbedder {
    fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        // Build deterministic bytes by hashing (counter || plaintext) until we
        // have enough material for `MOCK_EMBEDDING_DIM * 4` bytes (each f32
        // is 4 bytes). Each f32 is then mapped from `u32` into `[-1, 1]`.
        let needed_bytes = MOCK_EMBEDDING_DIM * 4;
        let mut bytes = Vec::with_capacity(needed_bytes);
        let mut counter: u32 = 0;
        while bytes.len() < needed_bytes {
            let mut h = Sha256::new();
            h.update(b"trace_gate_enclave.mock_embedder.v1\n");
            h.update(counter.to_be_bytes());
            h.update(b"\n");
            h.update(plaintext);
            bytes.extend_from_slice(&h.finalize());
            counter += 1;
        }
        bytes.truncate(needed_bytes);

        let mut v = Vec::with_capacity(MOCK_EMBEDDING_DIM);
        for &chunk in bytes.as_chunks::<4>().0 {
            let raw = u32::from_be_bytes(chunk);
            // Map u32 into [-1, 1]: ratio in [0, 1] then shift to [-1, 1].
            let ratio = raw as f64 / u32::MAX as f64;
            v.push(((ratio * 2.0) - 1.0) as f32);
        }

        // L2-normalize so cosine similarity reduces to a plain dot product.
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm > 0.0 {
            for x in &mut v {
                *x /= norm;
            }
        }
        Ok(v)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_embedder_is_deterministic() {
        let e = MockEmbedder::new();
        let a = e.embed(b"hello world").unwrap();
        let b = e.embed(b"hello world").unwrap();
        assert_eq!(a, b);
        assert_eq!(a.len(), MOCK_EMBEDDING_DIM);
    }

    #[test]
    fn mock_embedder_is_unit_norm() {
        let e = MockEmbedder::new();
        let v = e.embed(b"hello world").unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    #[test]
    fn mock_embedder_differs_per_input() {
        let e = MockEmbedder::new();
        let a = e.embed(b"hello world").unwrap();
        let b = e.embed(b"goodbye world").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn short_chunk_mean_pool_equals_direct_embed() {
        // A chunk within one sub-window must embed identically to a direct
        // call — proves the helper is a no-op for small inputs.
        let e = MockEmbedder::new();
        let direct = e.embed(b"hello world").unwrap();
        let pooled = embed_chunk_mean_pooled(&e, "hello world").unwrap();
        assert_eq!(direct, pooled);
    }

    #[test]
    fn long_chunk_covers_all_sub_windows_not_just_the_first() {
        // Two chunks that share the same first sub-window but differ in the
        // second MUST produce different embeddings — the old truncation bug
        // would make them identical.
        let e = MockEmbedder::new();
        let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
        let shared_head = "a".repeat(window_chars);
        let text_1 = format!("{shared_head}{}", "b".repeat(window_chars));
        let text_2 = format!("{shared_head}{}", "c".repeat(window_chars));
        let v1 = embed_chunk_mean_pooled(&e, &text_1).unwrap();
        let v2 = embed_chunk_mean_pooled(&e, &text_2).unwrap();
        assert_ne!(v1, v2, "tail content must influence the chunk embedding");
    }

    #[test]
    fn mean_pooled_embedding_is_unit_norm() {
        let e = MockEmbedder::new();
        let window_chars = EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN;
        let text = "x".repeat(window_chars * 3 + 17);
        let v = embed_chunk_mean_pooled(&e, &text).unwrap();
        let norm: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!((norm - 1.0).abs() < 1e-3, "expected unit norm, got {norm}");
    }

    /// Rejects whitespace-only input exactly as the fastembed embedder does
    /// (`reject_unembeddable_input`), and counts its calls. No model.
    struct WhitespaceRejectingEmbedder {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl WhitespaceRejectingEmbedder {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn calls(&self) -> usize {
            self.calls.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Embedder for WhitespaceRejectingEmbedder {
        fn embed(&self, plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            reject_unembeddable_input(&String::from_utf8_lossy(plaintext))?;
            MockEmbedder::new().embed(plaintext)
        }
    }

    fn window_chars() -> usize {
        EMBED_SUB_WINDOW_TOKENS * crate::chunker::APPROX_CHARS_PER_TOKEN
    }

    fn assert_close(a: &[f32], b: &[f32]) {
        assert_eq!(a.len(), b.len());
        for (x, y) in a.iter().zip(b) {
            assert!((x - y).abs() < 1e-5, "{x} != {y}");
        }
    }

    #[test]
    fn whitespace_only_input_is_rejected_as_input() {
        for text in ["", " ", "\n", " \t\r\n  "] {
            let error = reject_unembeddable_input(text).expect_err("whitespace is rejected");
            let failure = classify_embedder_error(&anyhow::Error::from(error));
            assert_eq!(failure.class, EmbedderFailureClass::InputRejected);
            assert_eq!(failure.reason_label, EMBEDDER_REASON_EMPTY_INPUT);
        }
        reject_unembeddable_input(" a ").expect("text with content is embeddable");
    }

    #[test]
    fn a_chunk_ending_in_a_newline_past_a_window_boundary_embeds() {
        // Every rendered event ends in `\n`, so a chunk one char longer than
        // a window has a final sub-window of just `\n`. The embedder rejects
        // that window; it carries no text, so the helper must not send it.
        let e = WhitespaceRejectingEmbedder::new();
        let head = "a".repeat(window_chars());
        let pooled = embed_chunk_mean_pooled(&e, &format!("{head}\n"))
            .expect("a trailing whitespace window does not fail the chunk");
        assert_eq!(e.calls(), 1, "only the window with text is embedded");
        assert_close(
            &pooled,
            &MockEmbedder::new().embed(head.as_bytes()).unwrap(),
        );
    }

    #[test]
    fn whitespace_only_sub_windows_inside_a_chunk_are_skipped() {
        let e = WhitespaceRejectingEmbedder::new();
        let w = window_chars();
        let text = format!("{}{}{}", "a".repeat(w), " ".repeat(w), "b".repeat(w));
        let pooled = embed_chunk_mean_pooled(&e, &text).expect("the chunk embeds");
        assert_eq!(e.calls(), 2);
        let m = MockEmbedder::new();
        let a = m.embed("a".repeat(w).as_bytes()).unwrap();
        let b = m.embed("b".repeat(w).as_bytes()).unwrap();
        let mut mean: Vec<f32> = a.iter().zip(&b).map(|(x, y)| (x + y) / 2.0).collect();
        let norm: f32 = mean.iter().map(|x| x * x).sum::<f32>().sqrt();
        for x in &mut mean {
            *x /= norm;
        }
        assert_close(&pooled, &mean);
    }

    #[test]
    fn a_long_chunk_of_only_whitespace_is_rejected_as_input_without_a_call() {
        let e = WhitespaceRejectingEmbedder::new();
        let error = embed_chunk_mean_pooled(&e, &" \n".repeat(window_chars() * 2))
            .expect_err("a chunk with no text cannot be embedded");
        assert_eq!(e.calls(), 0);
        let failure = classify_embedder_error(&error);
        assert_eq!(failure.class, EmbedderFailureClass::InputRejected);
        assert_eq!(failure.reason_label, EMBEDDER_REASON_EMPTY_INPUT);
    }

    #[test]
    fn a_short_whitespace_chunk_reaching_the_embedder_classifies_as_input() {
        // The short path passes straight through; the embedder's own
        // rejection must still classify as the input's fault.
        let e = WhitespaceRejectingEmbedder::new();
        let error = embed_chunk_mean_pooled(&e, "\n").expect_err("rejected");
        let failure = classify_embedder_error(&error.context("EmbedderInferenceFailed"));
        assert_eq!(failure.class, EmbedderFailureClass::InputRejected);
        assert_eq!(failure.reason_label, EMBEDDER_REASON_EMPTY_INPUT);
    }

    #[test]
    fn other_embedder_failures_classify_as_the_dependency() {
        let cases: [(anyhow::Error, &str); 5] = [
            (
                anyhow::anyhow!("onnx runtime exploded").context("EmbedderInferenceFailed"),
                EMBEDDER_REASON_INFERENCE_FAILED,
            ),
            (
                crate::embedder_fastembed::l2_normalize_in_place(&mut [f32::NAN])
                    .unwrap_err()
                    .context("EmbedderNormalizationFailed"),
                EMBEDDER_REASON_NON_FINITE_VECTOR,
            ),
            (
                crate::embedder_fastembed::l2_normalize_in_place(&mut [0.0_f32; 4]).unwrap_err(),
                EMBEDDER_REASON_DEGENERATE_VECTOR,
            ),
            (
                anyhow::anyhow!("EmbedderMutexPoisoned"),
                EMBEDDER_REASON_UNAVAILABLE,
            ),
            (
                anyhow::anyhow!("some message no embedder here writes"),
                EMBEDDER_REASON_UNCLASSIFIED,
            ),
        ];
        for (error, reason) in cases {
            let failure = classify_embedder_error(&error);
            assert_eq!(failure.class, EmbedderFailureClass::Dependency, "{reason}");
            assert_eq!(failure.reason_label, reason);
        }
    }

    #[test]
    fn empty_chunk_embeds_the_empty_string() {
        let e = MockEmbedder::new();
        let direct = e.embed(b"").unwrap();
        let pooled = embed_chunk_mean_pooled(&e, "").unwrap();
        assert_eq!(direct, pooled);
    }
}
