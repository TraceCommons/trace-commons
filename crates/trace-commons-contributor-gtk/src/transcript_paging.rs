//! How the transcript tab shows a whole body without laying all of it out.
//!
//! The tab used to hand its whole body to one `TextBuffer` and tag every
//! redaction marker in it. On a real 17.5 MB Claude Code session the macOS
//! shell pinned its main thread inside CoreText at 197% CPU and 2.97 GB
//! resident until it had to be force-quit. The first fix, shared by all
//! three shells, was a 64 KB clamp with a notice saying the rest was not
//! displayed. That bounded the damage at the cost of making the tab's
//! promise -- "exactly what would be sent" -- something the tab could no
//! longer keep.
//!
//! This is the cap moved rather than removed. Every byte is reachable; what
//! is bounded is how much text is *laid out and retained at once*. The body
//! is cut into chunks, only the chunks near the viewport are put into a
//! buffer and tagged, and chunks that scroll away are evicted. Eviction is
//! the load-bearing half: a window that only ever adds chunks reaches the
//! same out-of-memory failure as the original, just further down the
//! scrollbar.
//!
//! The design is `docs/superpowers/specs/2026-08-20-chunked-transcript-design.md`
//! and the reference implementation is
//! `macos/Sources/TCShellCore/TranscriptPaging.swift`. The *reasoning* ports;
//! the numbers do not. GTK is a different toolkit and its constants below
//! are its own measurements.
//!
//! ## What GTK actually costs
//!
//! Measured on this crate through `gtk::init()` + Pango, release build,
//! 11 px monospace, 720 px wide, transcript-shaped ~78-byte lines with a
//! redaction marker every fifth line.
//!
//! GTK's `TextView` lays out one `PangoLayout` per *line*, so unlike
//! SwiftUI's single-run `Text` its layout is **linear in the body**:
//!
//! ```text
//!   body     per-line layout    us per KB
//!    4 KB       0.97 ms            236
//!   16 KB       3.39 ms            212
//!   64 KB      16.37 ms            256
//!  256 KB      66.88 ms            261
//!    1 MB     274.81 ms            268
//!    4 MB     923.93 ms            226
//! ```
//!
//! `TextBuffer::set_text` is linear too and nearly free: 3.4 us/KB, so all
//! of 17.5 MB lands in a buffer in 59 ms.
//!
//! What is *not* linear -- and is the actual reason this shell freezes -- is
//! applying redaction tags across a whole buffer:
//!
//! ```text
//!   body    tag pass (shipped)   tag pass (single-pass rewrite)
//!   16 KB        0.09 ms                0.05 ms
//!   64 KB        0.77 ms                0.32 ms
//!  256 KB       10.67 ms                3.71 ms
//!    1 MB      165.95 ms               49.99 ms
//!    4 MB     2923.85 ms              775.21 ms
//! ```
//!
//! Each quadrupling of the body costs 15-18x, in both. Extrapolated to
//! 17.5 MB that is a minute of frozen main loop. The shipped version is
//! quadratic twice over (`text[..start].chars().count()` per marker, then
//! `iter_at_offset` walking the buffer), and rewriting the scan to a single
//! pass only moves the constant: `GtkTextBuffer`'s offset addressing is not
//! O(1), so *any* whole-buffer tag pass is superlinear. Bounding what gets
//! tagged is the fix, and it is what this module does.
//!
//! Two shapes of body also punish a single run even on GTK. One `PangoLayout`
//! over N bytes of wrapped transcript costs 257 us/KB at 4 KB and 2,540
//! us/KB at 256 KB; over one unbroken minified line, 217 us/KB at 4 KB and
//! 446 us/KB at 128 KB. A 17.5 MB minified body is a single line, which is a
//! single layout, which is the CoreText failure in a different toolkit.
//! Chunking bounds that too, because a chunk boundary ends a line.

use std::collections::HashMap;
use std::ops::Range;

/// Target chunk size, in bytes of UTF-8.
///
/// Measured cost of materialising one chunk -- `TextBuffer::set_text`, the
/// bounded tag pass, and per-line Pango layout:
///
/// ```text
///  chunk    set_text   tag pass   layout    total
///   4 KB     0.031 ms   0.011 ms   0.965 ms   1.008 ms
///   8 KB     0.038 ms   0.023 ms   1.680 ms   1.741 ms
///  16 KB     0.060 ms   0.052 ms   3.385 ms   3.497 ms
///  32 KB     0.115 ms   0.123 ms   6.759 ms   6.997 ms
///  64 KB     0.225 ms   0.342 ms  13.601 ms  14.168 ms
/// 128 KB     0.515 ms   1.100 ms  26.723 ms  28.337 ms
/// ```
///
/// This is where GTK parts company with the macOS shell. There, layout is
/// quadratic in the run, so `(B/c) * k*c^2 = k*B*c` -- the total cost of a
/// body falls as the chunk shrinks, and 4 KB was chosen as small as a view
/// could usefully be. Here the total is `(B/c) * k*c = k*B`: **the same
/// whichever chunk size is picked**. Chunk size therefore buys nothing on
/// throughput and is chosen purely for granularity -- large enough to keep
/// the widget count down, small enough that materialising one during a
/// scroll fits inside a frame.
///
/// A 60 Hz frame is 16.67 ms. 16 KB costs 3.5 ms, about a fifth of a frame,
/// which leaves 4.7x of headroom for a machine slower than the one measured
/// and for GTK's own measure/allocate/snapshot on the same frame. 64 KB
/// costs a whole frame here with no headroom at all, and 4 KB -- the macOS
/// number -- would quadruple the widget count (4,480 slots for a 17.5 MB
/// body against 1,120) to buy nothing measurable.
pub const TARGET_CHUNK_BYTES: usize = 16 * 1024;

/// The longest redaction marker the chunker will refuse to split.
///
/// Markers are short by construction (`<PRIVATE_SECRET_1>`,
/// `[REDACTED:aws_secret_key]`). This is the look-around window used when a
/// cut has to land in the middle of a line; a marker longer than this is not
/// protected, which is stated here rather than left to be discovered. Same
/// value as the other two shells, because it is a property of the markers
/// rather than of the toolkit.
pub const MAX_MARKER_BYTES: usize = 256;

/// The hard ceiling on a single chunk.
///
/// A chunk normally ends at a newline at or before the target. When a body
/// has no newline to cut on -- minified JSON on one line -- the cut is taken
/// at the target and then pushed off any redaction marker it landed inside,
/// which can carry it up to [`MAX_MARKER_BYTES`] further.
pub const MAX_CHUNK_BYTES: usize = TARGET_CHUNK_BYTES + MAX_MARKER_BYTES;

/// The ceiling on text laid out and retained at once, in bytes of UTF-8.
///
/// This is the number that replaces the old 64 KB clamp. It bounds buffer
/// and glyph storage rather than what a reader can reach, and it is constant
/// in the size of the body: a 17.5 MB trace retains exactly as much as a
/// 200 KB one.
///
/// Sized from the viewport, measured rather than guessed. At 11 px monospace
/// -- the size `.tc-transcript` sets -- Pango reports a 6.63 px advance and a
/// 13 px line box, and the tab adds 4 px of leading, so a screenful of
/// transcript is:
///
/// ```text
///   560 x  420 px pane    84 cols x 24 rows    2.0 KB
///   720 x  640 px pane   108 cols x 37 rows    3.9 KB
///  1400 x 1200 px pane   211 cols x 70 rows   14.4 KB
/// ```
///
/// 256 KB is 17 screenfuls even on a full-screen 1400x1200 pane and 65 on a
/// typical one: the visible page plus roughly eight screenfuls of overscan in
/// each direction, which is what keeps a flick-scroll from outrunning the
/// window and showing blank space.
///
/// Refilling all of it from cold was measured at 55.8 ms, and that only
/// happens on a jump, never on a drag (measured: 64 KB 14.1 ms, 128 KB
/// 27.8 ms, 256 KB 55.8 ms, 512 KB 113.6 ms, in 16 KB chunks). The macOS
/// shell caps at 128 KB and pays 221 ms for it, because its refill is
/// quadratic in the chunk; GTK's is linear, so this shell can afford twice
/// the window at a quarter of the cost.
pub const RETAINED_LIMIT_BYTES: usize = 256 * 1024;

/// One unit of layout: a byte range of the body that can be put into a
/// buffer on its own without splitting a character or a redaction marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Chunk {
    pub byte_offset: usize,
    pub byte_count: usize,
    /// Newlines in the chunk, used to place the chunk's stand-in while it is
    /// not resident. A chunk that ends at a newline counts that newline, so
    /// the count is the number of display rows when no line wraps.
    pub line_count: usize,
}

impl Chunk {
    pub fn byte_range(&self) -> Range<usize> {
        self.byte_offset..self.byte_offset + self.byte_count
    }

    /// Display rows this chunk occupies at `columns` characters per row.
    ///
    /// Estimated from bytes and newlines rather than measured, because
    /// measuring is the thing this whole design exists to avoid. In a
    /// monospaced font it is exact for any chunk whose lines all fit the
    /// width; it is high by at most one row per chunk when a line wraps and
    /// low by at most one row per line for a chunk mixing wrapped and short
    /// lines. Either way the error is bounded per chunk, so it shows up as
    /// the scrollbar settling slightly as chunks materialise rather than as
    /// a body of unknown length.
    pub fn rows(&self, columns: usize) -> usize {
        let columns = columns.max(1);
        let wrapped = self.byte_count.div_ceil(columns);
        self.line_count.max(wrapped).max(1)
    }
}

/// A body cut into chunks, holding its own copy so that slicing a chunk is a
/// slice rather than a walk.
#[derive(Debug, Clone)]
pub struct TranscriptDocument {
    text: String,
    chunks: Vec<Chunk>,
}

impl TranscriptDocument {
    /// Cuts the body once, when it arrives.
    pub fn new(text: String) -> Self {
        let chunks = cut(&text);
        Self { text, chunks }
    }

    pub fn chunks(&self) -> &[Chunk] {
        &self.chunks
    }

    pub fn chunk_count(&self) -> usize {
        self.chunks.len()
    }

    pub fn total_bytes(&self) -> usize {
        self.text.len()
    }

    /// The text of one chunk. Always valid UTF-8: the cut is scalar-aligned.
    pub fn text_of(&self, index: usize) -> &str {
        match self.chunks.get(index) {
            Some(chunk) => &self.text[chunk.byte_range()],
            None => "",
        }
    }

    /// The whole body. A string borrow, not a layout -- this is what "Copy
    /// everything" hands to the clipboard, and nothing lays it out.
    pub fn whole_text(&self) -> &str {
        &self.text
    }

    /// Display rows for every chunk at `columns`, in order.
    pub fn rows(&self, columns: usize) -> Vec<usize> {
        self.chunks.iter().map(|c| c.rows(columns)).collect()
    }
}

/// Cuts a body into chunks.
///
/// Rules, in order of preference, mirroring the reference implementation:
///
/// 1. End at the last newline at or before the target, provided that leaves
///    a chunk of at least half the target. A whole number of lines is what a
///    reader expects, and a newline can never be inside a redaction marker,
///    so this path is safe by construction. Essentially every real
///    transcript takes it.
/// 2. Otherwise cut at the target, then push the cut off any marker it
///    landed inside -- back to the marker's start if that leaves a non-empty
///    chunk, forward past its end if it did not.
/// 3. Then back the cut off any UTF-8 continuation byte, so the chunk ends
///    on a scalar boundary.
fn cut(text: &str) -> Vec<Chunk> {
    if text.is_empty() {
        return Vec::new();
    }
    let bytes = text.as_bytes();
    let minimum = TARGET_CHUNK_BYTES / 2;
    let mut chunks = Vec::with_capacity(bytes.len() / TARGET_CHUNK_BYTES + 1);

    let mut start = 0usize;
    while start < bytes.len() {
        let mut end = (start + TARGET_CHUNK_BYTES).min(bytes.len());
        if end < bytes.len() {
            match bytes[start..end].iter().rposition(|&b| b == b'\n') {
                Some(newline) if newline + 1 >= minimum => end = start + newline + 1,
                _ => {
                    end = push_off_marker(text, end, start);
                    while end > start + 1 && !text.is_char_boundary(end) {
                        end -= 1;
                    }
                }
            }
        }
        let line_count = bytes[start..end].iter().filter(|&&b| b == b'\n').count();
        chunks.push(Chunk {
            byte_offset: start,
            byte_count: end - start,
            line_count,
        });
        start = end;
    }
    chunks
}

/// Moves `cut` out of the middle of a redaction marker, if it is in one.
///
/// A marker rendered as two halves in two separately-tagged chunks --
/// `<PRIVATE_SEC` in one and `RET_1>` in the next -- is not a cosmetic
/// problem. The chips are how a contributor sees *where* scrubbing fired,
/// and half a marker in body type reads as content that was not scrubbed.
fn push_off_marker(text: &str, cut: usize, start: usize) -> usize {
    let mut look_back = cut.saturating_sub(MAX_MARKER_BYTES).max(start);
    while look_back > start && !text.is_char_boundary(look_back) {
        look_back -= 1;
    }
    if look_back >= cut {
        return cut;
    }
    let mut look_ahead = (cut + MAX_MARKER_BYTES).min(text.len());
    while look_ahead < text.len() && !text.is_char_boundary(look_ahead) {
        look_ahead += 1;
    }
    for span in marker_spans(&text[look_back..look_ahead]) {
        let marker_start = look_back + span.start;
        let marker_end = look_back + span.end;
        if marker_start >= cut || cut >= marker_end {
            continue;
        }
        return if marker_start > start {
            marker_start
        } else {
            marker_end.min(text.len())
        };
    }
    cut
}

/// Finds the redaction markers the scrubbing pipeline leaves behind, as byte
/// ranges into `text`, in order.
///
/// Shared by the chunker, which must not cut through one, and by the view,
/// which draws each one as a chip. One scanner, one place: a chunker that
/// protected a different set of markers than the view highlights would split
/// exactly the ones the view cares about.
///
/// This is the reference implementation's regex,
/// `<PRIVATE_[A-Za-z0-9_]+>|<REDACTED_[A-Za-z0-9_]+>|\[REDACTED[^\]\n]*\]`,
/// written out rather than compiled -- this crate carries no regex dependency
/// and the grammar is three literal prefixes and a terminator. The
/// `[REDACTED...]` arm excludes newlines as well as `]`: without that, one
/// unclosed bracket anywhere in a body would let a "marker" run to the end of
/// the file and the chunker would then refuse to cut there.
///
/// The `<REDACTED_...>` arm exists because the pipeline emits one
/// angle-bracketed FIXED token, `<REDACTED_PRIVATE_KEY>`
/// (`trace_contribution.rs`, `apply_pem_block_redaction`). It begins
/// `<REDACTED_`, not `<PRIVATE_`, and is not square-bracketed, so for as long
/// as this scanner had only two arms a PEM private key was removed from the
/// payload and left completely unmarked in the transcript -- the highest-stakes
/// redaction there is, and the one a contributor could not see had happened.
/// It was also the one marker the chunker would happily cut in half, since
/// this same scan is what protects them.
///
/// The arm is written as a general `<REDACTED_[A-Za-z0-9_]+>` rather than the
/// single literal, mirroring the `<PRIVATE_` arm, so a second angle-bracketed
/// fixed token cannot reopen the same hole.
/// The end of an angle-bracketed marker whose prefix ends at `after_prefix`,
/// or `None` if what follows is not one.
///
/// Shared by the `<PRIVATE_` and `<REDACTED_` arms, which have the identical
/// grammar: `[A-Za-z0-9_]+` -- at least one byte -- then `>`.
fn angle_marker_end(bytes: &[u8], after_prefix: usize) -> Option<usize> {
    let mut j = after_prefix;
    while j < bytes.len() && is_marker_word_byte(bytes[j]) {
        j += 1;
    }
    if j > after_prefix && bytes.get(j) == Some(&b'>') {
        Some(j + 1)
    } else {
        None
    }
}

pub fn marker_spans(text: &str) -> Vec<Range<usize>> {
    const PRIVATE: &[u8] = b"<PRIVATE_";
    const REDACTED_ANGLE: &[u8] = b"<REDACTED_";
    const REDACTED: &[u8] = b"[REDACTED";
    let bytes = text.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let end = match bytes[i] {
            b'<' if bytes[i..].starts_with(PRIVATE) => angle_marker_end(bytes, i + PRIVATE.len()),
            b'<' if bytes[i..].starts_with(REDACTED_ANGLE) => {
                angle_marker_end(bytes, i + REDACTED_ANGLE.len())
            }
            b'[' if bytes[i..].starts_with(REDACTED) => {
                let mut j = i + REDACTED.len();
                while j < bytes.len() && bytes[j] != b']' && bytes[j] != b'\n' {
                    j += 1;
                }
                if bytes.get(j) == Some(&b']') {
                    Some(j + 1)
                } else {
                    None
                }
            }
            _ => None,
        };
        match end {
            Some(end) => {
                spans.push(i..end);
                i = end;
            }
            None => i += 1,
        }
    }
    spans
}

fn is_marker_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// The chunks to keep laid out, given what is on screen.
///
/// The visible range comes first and is never dropped for overscan; if the
/// visible range alone somehow exceeded the ceiling it is trimmed from its
/// far end, so the returned window is under the ceiling unconditionally.
/// That trim is not expected to fire -- the ceiling is 256 KB and the
/// largest measured screenful is 14.4 KB -- but "not expected" is not a
/// bound.
///
/// Overscan is then added one chunk at a time, alternating below and above,
/// so a reader scrolling in either direction has the same amount of
/// already-laid-out text ahead of them. At the ends of the body the budget is
/// spent entirely on the side that exists.
pub fn residency_window(
    document: &TranscriptDocument,
    visible: Range<usize>,
    limit_bytes: usize,
) -> Range<usize> {
    let count = document.chunk_count();
    if count == 0 {
        return 0..0;
    }
    let chunks = document.chunks();

    let mut lower = visible.start.min(count - 1);
    let mut upper = visible.end.max(lower + 1).min(count);
    let mut bytes: usize = chunks[lower..upper].iter().map(|c| c.byte_count).sum();
    while upper - lower > 1 && bytes > limit_bytes {
        upper -= 1;
        bytes -= chunks[upper].byte_count;
    }

    let mut grow_down = true;
    loop {
        let can_down = lower > 0 && bytes + chunks[lower - 1].byte_count <= limit_bytes;
        let can_up = upper < count && bytes + chunks[upper].byte_count <= limit_bytes;
        if !can_down && !can_up {
            break;
        }
        if grow_down && can_down {
            lower -= 1;
            bytes += chunks[lower].byte_count;
        } else if can_up {
            bytes += chunks[upper].byte_count;
            upper += 1;
        } else {
            lower -= 1;
            bytes += chunks[lower].byte_count;
        }
        grow_down = !grow_down;
    }
    lower..upper
}

/// The chunks a viewport spanning `top..bottom` is over, given the height
/// each chunk's slot is currently standing at.
///
/// Separated from the view because it is the part of the scroll handler
/// that can be wrong quietly: a range that is off by one shows a strip of
/// blank where a chunk should be, and a range that collapses to nothing
/// stops the pane materialising anything at all. Heights are whatever the
/// slots are actually at -- measured for a chunk that has been laid out,
/// estimated for one that has not -- so this must not assume they are equal.
///
/// The returned range is never empty for a non-empty body: a viewport
/// scrolled past the end still has to be showing something.
pub fn visible_range(heights: &[f64], top: f64, bottom: f64) -> Range<usize> {
    if heights.is_empty() {
        return 0..0;
    }
    let mut y = 0f64;
    let mut first: Option<usize> = None;
    let mut last = 0usize;
    for (i, height) in heights.iter().enumerate() {
        // `y >= top` is the second half on purpose: a run of slots that
        // have not been allocated yet all stand at zero, so no slot's
        // bottom is past the top of the viewport and the first test alone
        // would name none of them.
        if first.is_none() && (y + height > top || y >= top) {
            first = Some(i);
        }
        if y >= bottom {
            break;
        }
        last = i;
        y += height;
    }
    let first = first.unwrap_or(heights.len() - 1);
    let last = last.max(first);
    first..(last + 1).min(heights.len())
}

/// The laid-out chunks the view is holding, and the eviction that keeps that
/// set bounded.
///
/// Generic over what a chunk is rendered into so the policy can be tested for
/// what it is -- an accounting rule over byte counts -- without a window, a
/// font or a display. The view instantiates it with `gtk::TextView`; the
/// tests instantiate it with `String` and assert the same `retained_bytes`
/// the view is subject to.
pub struct ResidentChunks<R> {
    window: Range<usize>,
    rendered: HashMap<usize, R>,
    retained_bytes: usize,
    evictions: usize,
}

impl<R> Default for ResidentChunks<R> {
    fn default() -> Self {
        Self {
            window: 0..0,
            rendered: HashMap::new(),
            retained_bytes: 0,
            evictions: 0,
        }
    }
}

impl<R> ResidentChunks<R> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn window(&self) -> Range<usize> {
        self.window.clone()
    }

    pub fn resident_count(&self) -> usize {
        self.rendered.len()
    }

    pub fn contains(&self, index: usize) -> bool {
        self.rendered.contains_key(&index)
    }

    /// UTF-8 bytes of body currently laid out. The number the ceiling is on.
    pub fn retained_bytes(&self) -> usize {
        self.retained_bytes
    }

    /// How many chunks have been evicted since this set was created. Exists
    /// so a test can prove eviction happened rather than infer it from a
    /// count that merely stopped growing.
    pub fn evictions(&self) -> usize {
        self.evictions
    }

    /// Drops everything, for a new body or a body that went away.
    pub fn clear(&mut self, mut evict: impl FnMut(usize, R)) {
        for (index, rendered) in self.rendered.drain() {
            evict(index, rendered);
        }
        self.window = 0..0;
        self.retained_bytes = 0;
    }

    /// Moves the window to cover `visible`, laying out what came into it and
    /// dropping what fell out.
    ///
    /// `make` is called only for chunks that are not already rendered, so a
    /// scroll of one chunk costs one chunk of layout, not a window's worth.
    pub fn update(
        &mut self,
        document: &TranscriptDocument,
        visible: Range<usize>,
        limit_bytes: usize,
        mut make: impl FnMut(usize) -> R,
        mut evict: impl FnMut(usize, R),
    ) {
        let next = residency_window(document, visible, limit_bytes);
        if next == self.window && self.rendered.len() == next.len() {
            return;
        }
        let dropped: Vec<usize> = self
            .rendered
            .keys()
            .copied()
            .filter(|i| !next.contains(i))
            .collect();
        for index in dropped {
            if let Some(rendered) = self.rendered.remove(&index) {
                self.retained_bytes -= document.chunks()[index].byte_count;
                self.evictions += 1;
                evict(index, rendered);
            }
        }
        for index in next.clone() {
            if let std::collections::hash_map::Entry::Vacant(slot) = self.rendered.entry(index) {
                slot.insert(make(index));
                self.retained_bytes += document.chunks()[index].byte_count;
            }
        }
        self.window = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A transcript-shaped body: ~78-byte lines, a redaction marker every
    /// fifth line. The same shape the measurements above were taken on.
    fn transcript(bytes: usize) -> String {
        let mut s = String::with_capacity(bytes + 128);
        let mut i = 0usize;
        while s.len() < bytes {
            if i % 5 == 4 {
                s.push_str("  \"key\": \"<PRIVATE_SECRET_1>\", and some more text on this line\n");
            } else {
                s.push_str("  \"role\": \"assistant\", \"text\": \"transcript content here\"\n");
            }
            i += 1;
        }
        s
    }

    /// The chunks are the body: concatenating them reproduces it exactly,
    /// byte for byte. This is the whole promise of the slice -- the tab is
    /// called "exactly what would be sent" and every byte has to be in it.
    #[test]
    fn chunks_concatenate_to_the_whole_body() {
        for body in [
            transcript(2 * 1024 * 1024),
            "\u{1F642}".repeat(200_000),
            "no newline at all ".repeat(9_000),
            "short".to_string(),
            String::new(),
        ] {
            let doc = TranscriptDocument::new(body.clone());
            let mut rebuilt = String::with_capacity(body.len());
            let mut expected_offset = 0usize;
            for (i, chunk) in doc.chunks().iter().enumerate() {
                assert_eq!(
                    chunk.byte_offset, expected_offset,
                    "chunk {i} is not adjacent"
                );
                assert!(chunk.byte_count > 0, "chunk {i} is empty");
                rebuilt.push_str(doc.text_of(i));
                expected_offset += chunk.byte_count;
            }
            assert_eq!(expected_offset, body.len());
            assert_eq!(rebuilt, body);
            assert_eq!(doc.total_bytes(), body.len());
        }
    }

    /// No chunk is over the hard ceiling. The ceiling is what the residency
    /// arithmetic is built on: a chunk that could be arbitrarily large would
    /// let the retained window be arbitrarily large with it.
    #[test]
    fn no_chunk_exceeds_the_ceiling() {
        for body in [
            transcript(2 * 1024 * 1024),
            "x".repeat(500_000),
            format!(
                "{}<PRIVATE_SECRET_1>{}",
                "y".repeat(16_380),
                "z".repeat(40_000)
            ),
        ] {
            let doc = TranscriptDocument::new(body);
            for (i, chunk) in doc.chunks().iter().enumerate() {
                assert!(
                    chunk.byte_count <= MAX_CHUNK_BYTES,
                    "chunk {i} is {} bytes, over the {MAX_CHUNK_BYTES} ceiling",
                    chunk.byte_count
                );
            }
        }
    }

    /// A body of lines cuts on line boundaries: every chunk but the last
    /// ends at a newline, so no chunk opens or closes mid-line.
    #[test]
    fn a_line_shaped_body_cuts_on_line_boundaries() {
        let doc = TranscriptDocument::new(transcript(1024 * 1024));
        assert!(doc.chunk_count() > 50);
        for i in 0..doc.chunk_count() - 1 {
            assert!(
                doc.text_of(i).ends_with('\n'),
                "chunk {i} does not end on a line boundary"
            );
        }
        // Cutting back to the last newline costs a little of the target --
        // half a line on average. The last chunk is whatever is left, so it
        // is excluded rather than allowed to drag the average down.
        let full = doc.chunk_count() - 1;
        let average: usize = doc.chunks()[..full]
            .iter()
            .map(|c| c.byte_count)
            .sum::<usize>()
            / full;
        assert!(
            average > TARGET_CHUNK_BYTES - 100 && average <= TARGET_CHUNK_BYTES,
            "average chunk {average} is not near the {TARGET_CHUNK_BYTES} target"
        );
    }

    /// The minified case: no newline to cut on, and four-byte scalars, so a
    /// naive byte cut lands mid-character with high probability. Every chunk
    /// must still be valid UTF-8 that round-trips its own bytes.
    #[test]
    fn a_body_without_newlines_never_splits_a_character() {
        let body = "\u{1F642}\u{00E9}a".repeat(60_000);
        let doc = TranscriptDocument::new(body.clone());
        assert!(doc.chunk_count() > 20);
        for i in 0..doc.chunk_count() {
            let text = doc.text_of(i);
            assert!(
                !text.contains('\u{FFFD}'),
                "chunk {i} has a replacement character"
            );
            let chunk = doc.chunks()[i];
            assert_eq!(text.as_bytes(), &body.as_bytes()[chunk.byte_range()]);
        }
    }

    /// A marker walked byte by byte across a chunk boundary comes out whole
    /// in exactly one chunk, never as two halves in two.
    ///
    /// Half a marker is not a cosmetic problem: `<PRIVATE_SEC` in one chunk
    /// and `RET_1>` in the next both draw in body type, and a marker in body
    /// type reads as content that was never scrubbed.
    #[test]
    fn a_marker_is_never_split_across_a_boundary() {
        for marker in [
            "<PRIVATE_SECRET_1>",
            "[REDACTED:aws_secret_key]",
            "[REDACTED]",
            // Until the scanner grew its `<REDACTED_` arm this one matched
            // nothing, so it was also the one marker the chunker would cut
            // in half -- a private key rendering as two fragments across a
            // boundary.
            "<REDACTED_PRIVATE_KEY>",
        ] {
            for offset in (TARGET_CHUNK_BYTES - marker.len() - 4)..(TARGET_CHUNK_BYTES + 4) {
                // No newlines anywhere, so rule 1 cannot apply and the cut
                // has to be pushed off the marker by rule 2.
                let mut body = "a".repeat(offset);
                body.push_str(marker);
                body.push_str(&"b".repeat(TARGET_CHUNK_BYTES));
                let doc = TranscriptDocument::new(body);
                let whole: Vec<&str> = (0..doc.chunk_count())
                    .flat_map(|i| {
                        let text = doc.text_of(i);
                        marker_spans(text)
                            .into_iter()
                            .map(move |s| &text[s])
                            .collect::<Vec<_>>()
                    })
                    .collect();
                assert_eq!(
                    whole,
                    vec![marker],
                    "marker {marker:?} at offset {offset} did not survive chunking whole"
                );
            }
        }
    }

    /// Scanning each chunk finds exactly the markers a scan of the whole
    /// body finds, at the same absolute offsets. This is what makes the
    /// bounded highlight pass equivalent to the unbounded one it replaces.
    #[test]
    fn per_chunk_marker_scan_equals_a_whole_body_scan() {
        for body in [
            transcript(512 * 1024),
            format!(
                "{}<PRIVATE_SECRET_1>{}",
                "q".repeat(16_370),
                "r".repeat(40_000)
            ),
            "[REDACTED:aws_secret_key]data".repeat(2_000),
        ] {
            let doc = TranscriptDocument::new(body.clone());
            let expected: Vec<Range<usize>> = marker_spans(&body);
            assert!(!expected.is_empty(), "fixture has no markers");
            let mut found: Vec<Range<usize>> = Vec::new();
            for i in 0..doc.chunk_count() {
                let base = doc.chunks()[i].byte_offset;
                for span in marker_spans(doc.text_of(i)) {
                    found.push(base + span.start..base + span.end);
                }
            }
            assert_eq!(found, expected);
        }
    }

    /// The marker grammar is the reference implementation's regex,
    /// `<PRIVATE_[A-Za-z0-9_]+>|\[REDACTED[^\]\n]*\]`, and the three shells
    /// have to agree on it or they protect different things.
    #[test]
    fn marker_grammar_matches_the_reference() {
        let cases: &[(&str, &[&str])] = &[
            ("a <PRIVATE_SECRET_1> b", &["<PRIVATE_SECRET_1>"]),
            ("a [REDACTED] b", &["[REDACTED]"]),
            (
                "a [REDACTED:aws_secret_key] b",
                &["[REDACTED:aws_secret_key]"],
            ),
            (
                "two <PRIVATE_A> and [REDACTED]",
                &["<PRIVATE_A>", "[REDACTED]"],
            ),
            // `+` needs at least one word byte after the underscore.
            ("<PRIVATE_>", &[]),
            // A space is not `[A-Za-z0-9_]`.
            ("<PRIVATE_A B>", &[]),
            // The angle-bracketed FIXED token the PEM path emits. For a long
            // time this matched neither arm, so a private key was removed
            // from the payload and left unmarked in the transcript.
            (
                "key <REDACTED_PRIVATE_KEY> here",
                &["<REDACTED_PRIVATE_KEY>"],
            ),
            // The general arm, not the single literal.
            ("<REDACTED_ANYTHING_ELSE>", &["<REDACTED_ANYTHING_ELSE>"]),
            // `+` needs at least one word byte after the underscore, same as
            // the `<PRIVATE_` arm.
            ("<REDACTED_>", &[]),
            // No underscore, so not the angle-bracketed family at all.
            ("<REDACTED>", &[]),
            // All three families in one body, in document order.
            (
                "<PRIVATE_LOCAL_PATH_1> [REDACTED:person_name] <REDACTED_PRIVATE_KEY> [REDACTED]",
                &[
                    "<PRIVATE_LOCAL_PATH_1>",
                    "[REDACTED:person_name]",
                    "<REDACTED_PRIVATE_KEY>",
                    "[REDACTED]",
                ],
            ),
            // Not a marker family we own.
            ("<html> [note] <PRIVATE", &[]),
            // The newline exclusion: an unclosed bracket must not swallow
            // the rest of the body.
            ("[REDACTED:oops\nnext line]", &[]),
            (
                "[REDACTED:ok]\n[REDACTED]",
                &["[REDACTED:ok]", "[REDACTED]"],
            ),
            // A marker abutting a multi-byte scalar keeps its own bounds.
            ("\u{1F642}<PRIVATE_X1>\u{1F642}", &["<PRIVATE_X1>"]),
        ];
        for (text, expected) in cases {
            let found: Vec<&str> = marker_spans(text).into_iter().map(|s| &text[s]).collect();
            assert_eq!(&found, expected, "text {text:?}");
        }
    }

    /// The ceiling holds while scrolling the whole way down a large body,
    /// and eviction is what makes it hold: the retained set is bounded, the
    /// eviction counter climbs, and every chunk was reachable on the way.
    ///
    /// This is the assertion the old 64 KB clamp bought by making the rest of
    /// the body unreachable. It is the same bound, without that price.
    #[test]
    fn retained_bytes_stay_under_the_ceiling_while_scrolling_the_whole_body() {
        let doc = TranscriptDocument::new(transcript(4 * 1024 * 1024));
        assert!(doc.chunk_count() > 200, "fixture is too small to evict");

        let mut resident: ResidentChunks<String> = ResidentChunks::new();
        let mut seen = vec![false; doc.chunk_count()];
        let mut peak = 0usize;
        // Three chunks visible at a time, a chunk at a time, top to bottom.
        for top in 0..doc.chunk_count() {
            let visible = top..(top + 3).min(doc.chunk_count());
            resident.update(
                &doc,
                visible,
                RETAINED_LIMIT_BYTES,
                |i| {
                    seen[i] = true;
                    doc.text_of(i).to_string()
                },
                |_, _| {},
            );
            assert!(
                resident.retained_bytes() <= RETAINED_LIMIT_BYTES,
                "retained {} bytes at chunk {top}, over the {RETAINED_LIMIT_BYTES} ceiling",
                resident.retained_bytes()
            );
            // The accounting is the truth of the rendered set, not a
            // parallel counter that could drift from it.
            let actual: usize = resident.window().map(|i| doc.chunks()[i].byte_count).sum();
            assert_eq!(resident.retained_bytes(), actual);
            assert_eq!(resident.resident_count(), resident.window().len());
            peak = peak.max(resident.retained_bytes());
        }

        assert!(
            peak > RETAINED_LIMIT_BYTES / 2,
            "the window never filled up"
        );
        assert!(
            resident.evictions() > doc.chunk_count() / 2,
            "only {} evictions over {} chunks -- the window grew instead of moving",
            resident.evictions(),
            doc.chunk_count()
        );
        assert!(seen.iter().all(|s| *s), "some chunk was never reachable");
        // And what is retained at the bottom is a window, not the body.
        assert!(resident.resident_count() < doc.chunk_count() / 4);
    }

    /// Advancing the viewport by one chunk lays out one chunk and drops one.
    /// A scroll that re-rendered the window would put a visible stall on
    /// every drag.
    #[test]
    fn advancing_one_chunk_renders_one_and_evicts_one() {
        let doc = TranscriptDocument::new(transcript(2 * 1024 * 1024));
        let mut resident: ResidentChunks<String> = ResidentChunks::new();
        let mut made = 0usize;
        let mut dropped = 0usize;
        resident.update(
            &doc,
            40..41,
            RETAINED_LIMIT_BYTES,
            |i| doc.text_of(i).to_string(),
            |_, _| {},
        );
        let before = resident.window();
        resident.update(
            &doc,
            41..42,
            RETAINED_LIMIT_BYTES,
            |i| {
                made += 1;
                doc.text_of(i).to_string()
            },
            |_, _| dropped += 1,
        );
        assert_eq!(made, 1, "one step of scroll laid out {made} chunks");
        assert_eq!(dropped, 1, "one step of scroll evicted {dropped} chunks");
        assert_eq!(resident.window().len(), before.len());
        assert!(resident.retained_bytes() <= RETAINED_LIMIT_BYTES);
    }

    /// Scrolling back up re-lays out what was evicted -- the body is
    /// reachable in both directions, not only forwards.
    #[test]
    fn scrolling_back_up_restores_an_evicted_chunk() {
        let doc = TranscriptDocument::new(transcript(2 * 1024 * 1024));
        let mut resident: ResidentChunks<String> = ResidentChunks::new();
        resident.update(
            &doc,
            0..2,
            RETAINED_LIMIT_BYTES,
            |i| doc.text_of(i).to_string(),
            |_, _| {},
        );
        assert!(resident.contains(0));
        resident.update(
            &doc,
            90..92,
            RETAINED_LIMIT_BYTES,
            |i| doc.text_of(i).to_string(),
            |_, _| {},
        );
        assert!(!resident.contains(0), "chunk 0 was not evicted on a jump");
        assert!(resident.evictions() > 0);
        let mut remade: Option<String> = None;
        resident.update(
            &doc,
            0..2,
            RETAINED_LIMIT_BYTES,
            |i| {
                let text = doc.text_of(i).to_string();
                if i == 0 {
                    remade = Some(text.clone());
                }
                text
            },
            |_, _| {},
        );
        assert!(resident.contains(0));
        // What came back is the body's own bytes, not a stale or empty
        // stand-in left over from the eviction.
        assert_eq!(remade.as_deref(), Some(doc.text_of(0)));
        assert!(resident.retained_bytes() <= RETAINED_LIMIT_BYTES);
    }

    /// A visible range larger than the ceiling is trimmed rather than
    /// honoured. Not expected to fire -- 256 KB against a 14.4 KB screenful
    /// -- but "not expected" is not a bound.
    #[test]
    fn a_visible_range_over_the_ceiling_is_trimmed() {
        let doc = TranscriptDocument::new(transcript(4 * 1024 * 1024));
        let window = residency_window(&doc, 0..doc.chunk_count(), RETAINED_LIMIT_BYTES);
        let bytes: usize = window.clone().map(|i| doc.chunks()[i].byte_count).sum();
        assert!(bytes <= RETAINED_LIMIT_BYTES);
        assert_eq!(
            window.start, 0,
            "the visible range is trimmed from its far end"
        );
    }

    /// Overscan is spent on both sides in the middle of a body and entirely
    /// on the side that exists at its ends.
    #[test]
    fn overscan_is_balanced_in_the_middle_and_one_sided_at_the_ends() {
        let doc = TranscriptDocument::new(transcript(4 * 1024 * 1024));
        let middle = residency_window(&doc, 100..101, RETAINED_LIMIT_BYTES);
        assert!(middle.start < 100 && middle.end > 101);
        let below = 100 - middle.start;
        let above = middle.end - 101;
        assert!(
            below.abs_diff(above) <= 1,
            "overscan {below} below, {above} above"
        );

        let top = residency_window(&doc, 0..1, RETAINED_LIMIT_BYTES);
        assert_eq!(top.start, 0);
        assert_eq!(top.len(), middle.len(), "the budget is spent either way");

        let last = doc.chunk_count() - 1;
        let bottom = residency_window(&doc, last..last + 1, RETAINED_LIMIT_BYTES);
        assert_eq!(bottom.end, doc.chunk_count());
        assert_eq!(bottom.len(), middle.len());
    }

    /// A body that fits inside the ceiling is entirely resident: the common
    /// case must not pay for machinery it does not need.
    #[test]
    fn a_small_body_is_entirely_resident() {
        let doc = TranscriptDocument::new(transcript(64 * 1024));
        let window = residency_window(&doc, 0..1, RETAINED_LIMIT_BYTES);
        assert_eq!(window, 0..doc.chunk_count());
    }

    /// An empty body has no chunks and no window, rather than one empty
    /// chunk the view would have to special-case.
    #[test]
    fn an_empty_body_has_no_chunks() {
        let doc = TranscriptDocument::new(String::new());
        assert_eq!(doc.chunk_count(), 0);
        assert_eq!(residency_window(&doc, 0..1, RETAINED_LIMIT_BYTES), 0..0);
        assert_eq!(doc.text_of(0), "");
    }

    /// The viewport walk lands on the chunks the viewport is actually over,
    /// for heights that are not all the same -- which is the normal state,
    /// since a chunk that has been laid out stands at its measured height
    /// and one that has not stands at an estimate.
    #[test]
    fn the_viewport_walk_covers_exactly_the_chunks_on_screen() {
        // Chunk 0 spans 0..100, 1 spans 100..350, 2 spans 350..400,
        // 3 spans 400..1000, 4 spans 1000..1010.
        let heights = [100.0, 250.0, 50.0, 600.0, 10.0];
        assert_eq!(visible_range(&heights, 0.0, 100.0), 0..1);
        assert_eq!(visible_range(&heights, 0.0, 101.0), 0..2);
        // A viewport starting exactly on a boundary starts at that chunk.
        assert_eq!(visible_range(&heights, 100.0, 350.0), 1..2);
        assert_eq!(visible_range(&heights, 99.0, 351.0), 0..3);
        assert_eq!(visible_range(&heights, 360.0, 410.0), 2..4);
        // Scrolled to the very bottom.
        assert_eq!(visible_range(&heights, 1000.0, 1010.0), 4..5);
        // Scrolled past the end -- a stale adjustment during a resize. The
        // range must still name a chunk rather than collapse to nothing.
        let past = visible_range(&heights, 5000.0, 5400.0);
        assert_eq!(past, 4..5);
        assert!(!past.is_empty());
    }

    /// Degenerate inputs a resize can produce: no chunks, and slots that
    /// have not been allocated yet and stand at nothing.
    #[test]
    fn the_viewport_walk_survives_zero_heights_and_an_empty_body() {
        assert_eq!(visible_range(&[], 0.0, 400.0), 0..0);
        let zeros = [0.0, 0.0, 0.0];
        let range = visible_range(&zeros, 0.0, 400.0);
        assert!(!range.is_empty(), "a zero-height column named no chunk");
        assert_eq!(range, 0..3);
        assert_eq!(visible_range(&[500.0], 0.0, 400.0), 0..1);
    }

    /// The walk and the residency window compose into what the pane does:
    /// the chunks on screen are always resident, and the total stays under
    /// the ceiling.
    #[test]
    fn what_is_on_screen_is_always_resident() {
        let doc = TranscriptDocument::new(transcript(2 * 1024 * 1024));
        // A 108-column pane: the shape the sheet opens at.
        let heights: Vec<f64> = doc
            .chunks()
            .iter()
            .map(|c| (c.rows(108) * 17) as f64)
            .collect();
        let total: f64 = heights.iter().sum();
        let page = 640.0;
        let mut resident: ResidentChunks<String> = ResidentChunks::new();
        let mut top = 0.0;
        while top < total {
            let visible = visible_range(&heights, top, top + page);
            resident.update(
                &doc,
                visible.clone(),
                RETAINED_LIMIT_BYTES,
                |i| doc.text_of(i).to_string(),
                |_, _| {},
            );
            for i in visible {
                assert!(
                    resident.contains(i),
                    "chunk {i} is on screen but not laid out"
                );
            }
            assert!(resident.retained_bytes() <= RETAINED_LIMIT_BYTES);
            // Half a screen at a time, the way a drag moves.
            top += page / 2.0;
        }
        assert!(
            resident.evictions() > 0,
            "a whole-body scroll evicted nothing"
        );
    }

    /// The row estimate is exact for lines that fit the width, and never
    /// zero -- a chunk with no height would collapse the scroll extent.
    #[test]
    fn row_estimate_is_exact_for_unwrapped_lines_and_never_zero() {
        let body = "0123456789\n".repeat(3_000);
        let doc = TranscriptDocument::new(body);
        for chunk in doc.chunks() {
            assert_eq!(
                chunk.rows(80),
                chunk.line_count,
                "80 columns fits a 10-byte line"
            );
            assert!(
                chunk.rows(1) >= chunk.byte_count,
                "a 1-column pane wraps every byte"
            );
            assert!(
                chunk.rows(0) >= 1,
                "a zero-width pane must not divide by zero"
            );
        }
        let one_line = TranscriptDocument::new("x".repeat(8_000));
        assert_eq!(one_line.chunks()[0].rows(80), 100);
    }

    /// Cutting the body that started this is a scan, not a reflow. The bound
    /// is generous because this runs in a debug build under test; the point
    /// is that it is not quadratic.
    #[test]
    fn the_seventeen_megabyte_body_chunks_promptly() {
        let body = transcript(17_500_000);
        let started = std::time::Instant::now();
        let doc = TranscriptDocument::new(body.clone());
        let elapsed = started.elapsed();
        assert_eq!(doc.total_bytes(), body.len());
        assert!(doc.chunk_count() > 1_000);
        assert!(
            elapsed.as_secs_f64() < 2.0,
            "chunking took {elapsed:?}; it should be a scan, not a reflow"
        );
        // And the resident window over it is the same size as over a small
        // body: constant in the size of the trace.
        let window = residency_window(&doc, 500..503, RETAINED_LIMIT_BYTES);
        let bytes: usize = window.clone().map(|i| doc.chunks()[i].byte_count).sum();
        assert!(bytes <= RETAINED_LIMIT_BYTES);
        assert!(bytes > RETAINED_LIMIT_BYTES - MAX_CHUNK_BYTES);
    }
}
