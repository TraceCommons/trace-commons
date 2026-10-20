//! Putting the real values back, on a stream that respects nothing.
//!
//! This is the most dangerous code in the privacy filter, for a specific
//! reason: a response arrives in chunks whose boundaries are chosen by the
//! network, so a placeholder can be split at *any* byte. A reverser that
//! handles the common case and drops one in ten thousand tokens would corrupt a
//! transcript rarely enough to be shipped and often enough to matter.
//!
//! Two rules (`docs/PRIVACY.md` §5):
//!
//! 1. **Never emit a partial placeholder.** Bytes that might be the start of
//!    one are held until we know.
//! 2. **Never emit an unreversed placeholder we minted.** If the stream ends
//!    with one still pending, that is a failure the caller must surface — not
//!    something to flush and hope about.

use crate::mint::{self, CLOSE, Map, OPEN};

/// Incremental reverser over a byte stream.
///
/// Feed it chunks; it returns the bytes that are safe to forward. The held
/// buffer is bounded by [`mint::max_placeholder_len`], so a hostile or broken
/// upstream cannot make it grow.
#[derive(Debug)]
pub struct Reverser {
    /// Bytes we have seen but cannot yet classify.
    pending: String,
    /// Placeholders successfully reversed, for the ledger.
    reversed: usize,
    /// Placeholder-shaped strings we did not mint and therefore passed through.
    passed_through: usize,
}

/// What went wrong at the end of a stream.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReverseError {
    /// The stream ended with a placeholder we minted still unreversed.
    ///
    /// Forwarding it would write a token into the client's permanent
    /// transcript, where — on a compaction turn — it would be resent every turn
    /// for the rest of the session and never reversed, because next turn's map
    /// is derived from plaintext and will not contain it.
    #[error("the response ended with {count} unreversed placeholder(s)")]
    Unreversed {
        /// How many.
        count: usize,
    },
}

impl Default for Reverser {
    fn default() -> Self {
        Self::new()
    }
}

impl Reverser {
    /// A reverser with nothing pending.
    #[must_use]
    pub fn new() -> Self {
        Self {
            pending: String::new(),
            reversed: 0,
            passed_through: 0,
        }
    }

    /// How many placeholders were put back.
    #[must_use]
    pub fn reversed(&self) -> usize {
        self.reversed
    }

    /// How many placeholder-shaped strings we did not mint and passed through
    /// untouched.
    #[must_use]
    pub fn passed_through(&self) -> usize {
        self.passed_through
    }

    /// Consume a chunk, returning what is safe to forward now.
    ///
    /// Chunks are `&str` rather than bytes: the caller is responsible for
    /// re-assembling UTF-8, because a placeholder's delimiters are multi-byte
    /// and splitting one mid-codepoint would produce nonsense either way.
    pub fn push(&mut self, map: &Map, chunk: &str) -> String {
        self.pending.push_str(chunk);
        self.drain(map, false)
    }

    /// Finish the stream.
    ///
    /// # Errors
    ///
    /// [`ReverseError::Unreversed`] when the stream ended mid-placeholder for
    /// a token we minted. The caller must fail the exchange rather than
    /// forwarding — see the module docs.
    pub fn finish(&mut self, map: &Map) -> Result<String, ReverseError> {
        let flushed = self.drain(map, true);
        // A trailing fragment that *starts* like one of our placeholders is the
        // dangerous case: it will be written into the transcript and can never
        // be reversed afterwards.
        if let Some(count) = self.dangling(map).or_else(|| self.mangled(map)) {
            return Err(ReverseError::Unreversed { count });
        }
        let rest = std::mem::take(&mut self.pending);
        Ok(flushed + &rest)
    }

    /// Whether the leftover looks like the beginning of a placeholder we minted.
    fn dangling(&self, map: &Map) -> Option<usize> {
        if !self.pending.contains(OPEN) {
            return None;
        }
        let fragment = &self.pending[self.pending.find(OPEN)?..];
        // A prefix of any token we minted is a token that was cut off.
        let matches = map
            .placeholders()
            .filter(|token| token.starts_with(fragment))
            .count();
        (matches > 0).then_some(matches)
    }

    /// Whether the pending text begins with a *mangled* placeholder: one of our
    /// tokens that was truncated or altered part-way through.
    ///
    /// Distinct from [`Self::dangling`], which catches a token cut off at the
    /// end of the stream. This catches one that was cut off *and then followed
    /// by more text* — which is what a model paraphrasing a summary produces,
    /// and what an over-long unterminated candidate would otherwise be flushed
    /// as ordinary text.
    ///
    /// Requires the digest to have started matching before calling it mangled:
    /// a bare `⟦` followed by coincidental text is not evidence of anything,
    /// and treating it as a failure would break streams that merely contain the
    /// delimiter.
    fn mangled(&self, map: &Map) -> Option<usize> {
        let start = self.pending.find(OPEN)?;
        let fragment = &self.pending[start..];
        let count = map
            .placeholders()
            .filter(|token| {
                let common = common_prefix_len(fragment, token);
                // The whole token is present: it will be reversed normally.
                if common == token.len() {
                    return false;
                }
                // Enough of the digest matched that this is our token, altered.
                token[..common]
                    .split_once('.')
                    .is_some_and(|(_, digest)| digest.len() >= MANGLE_EVIDENCE)
            })
            .count();
        (count > 0).then_some(count)
    }

    /// Emit everything we can classify; hold the rest.
    fn drain(&mut self, map: &Map, final_chunk: bool) -> String {
        let bound = mint::max_placeholder_len();
        let mut out = String::with_capacity(self.pending.len());

        loop {
            let Some(open) = self.pending.find(OPEN) else {
                // Nothing that could start a placeholder. Everything before the
                // last `bound` bytes is definitely safe; hold a tail in case a
                // delimiter is split across the boundary.
                let safe = if final_chunk {
                    self.pending.len()
                } else {
                    floor_char_boundary(
                        &self.pending,
                        self.pending.len().saturating_sub(OPEN.len()),
                    )
                };
                out.push_str(&self.pending[..safe]);
                self.pending.drain(..safe);
                return out;
            };

            // Text before a candidate is always safe.
            out.push_str(&self.pending[..open]);
            self.pending.drain(..open);

            match self.pending.find(CLOSE) {
                Some(close) => {
                    let end = close + CLOSE.len();
                    let candidate = self.pending[..end].to_string();
                    match map.plaintext(&candidate) {
                        Some(plaintext) => {
                            out.push_str(plaintext);
                            self.reversed += 1;
                        }
                        None => {
                            // Not ours. The model may have invented it, or it
                            // may be a stale token from a previous salt.
                            // Either way we did not mint it and must not map it.
                            out.push_str(&candidate);
                            self.passed_through += 1;
                        }
                    }
                    self.pending.drain(..end);
                }
                None => {
                    // An unterminated candidate. Hold it — unless it is already
                    // longer than any placeholder can be, in which case the
                    // opening delimiter was ordinary text and holding more
                    // would grow the buffer without bound.
                    //
                    // Unless it is one of *our* tokens, altered. Flushing that
                    // as ordinary text is exactly the silent corruption this
                    // module exists to prevent, so it is held for `finish` to
                    // report. The buffer stays bounded because `mangled` only
                    // fires on a fragment that matches a token we minted, and
                    // those are bounded in number and length.
                    if self.pending.len() > bound {
                        if self.mangled(map).is_some() {
                            return out;
                        }
                        let safe = floor_char_boundary(&self.pending, bound);
                        out.push_str(&self.pending[..safe]);
                        self.pending.drain(..safe);
                        continue;
                    }
                    return out;
                }
            }
        }
    }
}

/// How many digest characters must match before a fragment is treated as one
/// of our tokens rather than as coincidental text.
const MANGLE_EVIDENCE: usize = 4;

/// Length of the longest common prefix, on a character boundary.
fn common_prefix_len(a: &str, b: &str) -> usize {
    let mut len = 0;
    for (x, y) in a.chars().zip(b.chars()) {
        if x != y {
            break;
        }
        len += x.len_utf8();
    }
    len
}

/// Largest index `<= at` that lies on a UTF-8 character boundary.
///
/// `str::floor_char_boundary` is unstable, and slicing mid-codepoint panics.
fn floor_char_boundary(text: &str, at: usize) -> usize {
    let mut index = at.min(text.len());
    while index > 0 && !text.is_char_boundary(index) {
        index -= 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mint::{Class, Salt};

    fn fixture() -> (Map, String) {
        let salt = Salt::fixed(11);
        let mut map = Map::new();
        let token = map.insert(&salt, Class::Email, "alice@corp.com");
        (map, token)
    }

    fn reverse_whole(map: &Map, text: &str) -> Result<String, ReverseError> {
        let mut reverser = Reverser::new();
        let mut out = reverser.push(map, text);
        out.push_str(&reverser.finish(map)?);
        Ok(out)
    }

    #[test]
    fn a_placeholder_in_one_piece_comes_back() {
        let (map, token) = fixture();
        let text = format!("Mail {token} about it.");
        assert_eq!(
            reverse_whole(&map, &text).expect("reverses"),
            "Mail alice@corp.com about it."
        );
    }

    #[test]
    fn a_placeholder_split_at_every_byte_offset_comes_back() {
        // The assertion this module exists for. A response arrives in chunks
        // whose boundaries the network chooses, and a reverser that handles
        // most splits would corrupt transcripts rarely enough to ship.
        let (map, token) = fixture();
        let text = format!("before {token} after");

        for split in 1..text.len() {
            if !text.is_char_boundary(split) {
                continue;
            }
            let mut reverser = Reverser::new();
            let mut out = reverser.push(&map, &text[..split]);
            out.push_str(&reverser.push(&map, &text[split..]));
            out.push_str(&reverser.finish(&map).expect("reverses"));
            assert_eq!(
                out, "before alice@corp.com after",
                "split at byte {split} lost the placeholder"
            );
        }
    }

    #[test]
    fn a_placeholder_split_into_single_bytes_comes_back() {
        // The pathological case: one chunk per character.
        let (map, token) = fixture();
        let text = format!("x{token}y");
        let mut reverser = Reverser::new();
        let mut out = String::new();
        for ch in text.chars() {
            out.push_str(&reverser.push(&map, &ch.to_string()));
        }
        out.push_str(&reverser.finish(&map).expect("reverses"));
        assert_eq!(out, "xalice@corp.comy");
    }

    #[test]
    fn a_stream_that_ends_mid_placeholder_fails_loudly() {
        // Forwarding the fragment would write a token into the client's
        // permanent transcript, where it can never be reversed again.
        let (map, token) = fixture();
        let truncated = &token[..token.len() - CLOSE.len() - 2];

        let mut reverser = Reverser::new();
        let _ = reverser.push(&map, &format!("text {truncated}"));
        assert_eq!(
            reverser.finish(&map),
            Err(ReverseError::Unreversed { count: 1 })
        );
    }

    #[test]
    fn a_placeholder_we_did_not_mint_is_passed_through_untouched() {
        // The model can invent placeholder-shaped strings, and a stale token
        // from a previous salt can arrive in replayed history. Mapping either
        // to a real value would be the worst bug here.
        let (map, _token) = fixture();
        let text = "see ⟦email.ffffffffffff⟧ please";
        let mut reverser = Reverser::new();
        let mut out = reverser.push(&map, text);
        out.push_str(&reverser.finish(&map).expect("no dangling"));
        assert_eq!(out, text);
        assert_eq!(reverser.passed_through(), 1);
        assert_eq!(reverser.reversed(), 0);
    }

    #[test]
    fn ordinary_text_containing_the_opening_delimiter_does_not_stall_the_stream() {
        // A lone `⟦` in someone's source would otherwise hold the buffer open
        // forever, which reads to the user as a hung agent.
        let (map, _token) = fixture();
        let long_tail = "x".repeat(mint::max_placeholder_len() * 3);
        let text = format!("math: ⟦{long_tail}");

        let mut reverser = Reverser::new();
        let emitted = reverser.push(&map, &text);
        assert!(
            !emitted.is_empty(),
            "the stream stalled on a lone delimiter"
        );
        let rest = reverser.finish(&map).expect("no dangling");
        assert_eq!(emitted + &rest, text);
    }

    #[test]
    fn the_held_buffer_never_grows_without_bound() {
        // A hostile or broken upstream must not be able to make us allocate.
        let (map, _token) = fixture();
        let mut reverser = Reverser::new();
        for _ in 0..1000 {
            let _ = reverser.push(&map, "⟦not-a-real-placeholder-at-all");
        }
        assert!(
            reverser.pending.len() <= mint::max_placeholder_len() * 2,
            "buffer grew to {}",
            reverser.pending.len()
        );
    }

    #[test]
    fn several_placeholders_in_one_chunk_all_come_back() {
        let salt = Salt::fixed(12);
        let mut map = Map::new();
        let a = map.insert(&salt, Class::Email, "a@x.com");
        let b = map.insert(&salt, Class::IpAddress, "10.0.0.1");
        let text = format!("{a} at {b} and {a} again");
        assert_eq!(
            reverse_whole(&map, &text).expect("reverses"),
            "a@x.com at 10.0.0.1 and a@x.com again"
        );
    }

    #[test]
    fn a_stream_with_no_placeholders_is_returned_unchanged() {
        // The overwhelmingly common case, and it must not be perturbed.
        let (map, _token) = fixture();
        let text = "a perfectly ordinary response with no substitutions at all";
        assert_eq!(reverse_whole(&map, text).expect("reverses"), text);
    }

    #[test]
    fn multibyte_text_around_a_placeholder_survives() {
        let (map, token) = fixture();
        let text = format!("日本語 {token} — émoji 🎉");
        assert_eq!(
            reverse_whole(&map, &text).expect("reverses"),
            "日本語 alice@corp.com — émoji 🎉"
        );
    }

    #[test]
    fn an_empty_stream_is_fine() {
        let (map, _token) = fixture();
        let mut reverser = Reverser::new();
        assert_eq!(reverser.finish(&map).expect("reverses"), "");
    }
}
