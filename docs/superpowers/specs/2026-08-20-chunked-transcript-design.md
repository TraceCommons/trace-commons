# Chunked transcript — reading a 17.5 MB trace without freezing or filling memory

Date: 2026-08-20
Status: implemented on macOS (the reference shell) and on Windows. GTK unported.
Scope: issue #349. The preview sheet's "Exactly what would be sent" tab, its
redaction-marker chipping, and the search tab's context snippets.
Reads with: `2026-08-08-contributor-shell-macos-design.md` for the sheet, and
the doc comments in `macos/Sources/TCShellCore/TranscriptPaging.swift`, which
carry the same measurements at the point of use.

## The problem

The transcript tab handed its whole body to a single text run. On a real
17.5 MB Claude Code session that pinned the main thread inside CoreText at
197% CPU and 2.97 GB resident until the app had to be force-quit; every
main-thread sample landed in
`__NSCoreTypesetterCreateBaseLineFromAttributedString`.

The shipped mitigation is `TranscriptBudget`: clamp the tab to 64 KB, cut on
a line boundary, and say so in a notice. It bounded the damage. It also made
the tab's promise false — the tab is called "Exactly what would be sent", the
button beneath it approves every byte, and what it showed was the first
fraction of the body with a sentence admitting it.

This slice moves the cap instead of removing it. **Every byte is reachable.
What is bounded is how much text is laid out and retained at once.**

## Three decisions taken as given

These came from the project owner and are recorded here so a later reader
does not relitigate them.

1. **The read gate does not change.** It stays first-screen-plus-explicit-
   acknowledgement, and it keeps claiming only that the first screenful was
   displayed. Paging makes the whole body available; it must not become a new
   thing we pretend was read. There is no scroll-to-the-end gating, and the
   `onFirstScreenShown` callback and its wiring are untouched.
2. **The cap moves rather than disappears.** It used to bound what was
   reachable; it now bounds what is typeset and retained. Eviction is part of
   the requirement, not an optimisation: a window that only ever adds chunks
   reaches the same out-of-memory failure, further down the scrollbar.
3. **The notice is eliminated.** Once every byte is reachable, "The rest is
   not displayed here" is false. `TranscriptBudget.swift` and its tests are
   deleted, and the cross-shell copy contract they carried goes with them.

## Measurements

All on an M-series laptop, 13pt monospaced (`Font.system(.subheadline,
design: .monospaced)`, which is what the tab uses), 720pt wide, with
transcript-shaped ~78-byte lines. Harnesses used AppKit directly; the numbers
in the code comments are these numbers.

### Why single-run layout is the whole problem

SwiftUI's `Text` sizes an attributed string through
`NSAttributedString.boundingRect(with:options:.usesLineFragmentOrigin)`.
That call is quadratic in length:

| size | plain | with redaction chips |
|---|---|---|
| 4 KB | 0.005 s | 0.005 s |
| 8 KB | 0.007 s | 0.018 s |
| 16 KB | 0.020 s | 0.055 s |
| 32 KB | 0.088 s | 0.234 s |
| 64 KB | 0.379 s | 1.138 s |
| 128 KB | 1.727 s | 4.882 s |

Each doubling costs about four times as much, and the chip attributes that
mark where scrubbing fired roughly triple the constant. Extrapolated to
17.5 MB this is hours, which is exactly the observed "window never comes
back". Note that these are worse than the numbers recorded in
`TranscriptBudget` (64 KB at 0.142 s) — those were measured through a
different path; the shape is the same and it is the shape that decides the
design.

It is worth being explicit that the quadratic is in the *run*, not in
TextKit generally: `NSLayoutManager.ensureLayout` over the same bodies is
linear (256 KB in 0.031 s, 512 KB in 0.062 s). The tab is a SwiftUI `Text`,
so it is on the quadratic path, and a rewrite onto `NSTextView` was not in
scope for this slice.

### What the quadratic implies for chunking

Laying out a body of `B` bytes in chunks of `c` costs `(B/c) · k·c² = k·B·c`:
linear in the body and *proportional to the chunk size*. Smaller chunks are
strictly cheaper. The chunk size is therefore set by the smallest unit still
worth being a view, not by how much text looks like a reasonable page.

| chunk | layout, one chunk, with chips | refill a full 128 KB window |
|---|---|---|
| 2 KB | 0.0018 s | 0.114 s |
| 4 KB | 0.0064 s | 0.221 s |
| 8 KB | 0.0252 s | 0.383 s |
| 16 KB | 0.0690 s | 0.547 s |
| 32 KB | 0.2202 s | 0.980 s |

### How big a screenful actually is

13pt monospaced measures 8.036 pt of advance and a 16 pt line box:

| sheet | columns × rows | bytes on screen |
|---|---|---|
| 640 × 420 pt | 79 × 22 | 1.7 KB |
| 720 × 560 pt | 89 × 29 | 2.5 KB |
| 1000 × 1100 pt | 124 × 57 | 6.9 KB |

### Cost of the new work itself

- Chunking a 17.5 MB body: **0.0064 s** release, 0.663 s debug. It runs once,
  when the sheet's body arrives, and at 6 ms it does not need to leave the
  main actor in a shipping build. The debug figure is an unoptimised-build
  artifact and is why the test's bound is 2 s rather than 50 ms.
- 2,000 successive window moves over that body: **0.011 s**. Moving the
  window does not walk the body.
- Retained `NSAttributedString` storage measured about 2.4–3.1 bytes per
  source byte for the chipped text alone. The 170 bytes per source byte
  implied by the incident (2.97 GB for 17.5 MB) is the figure for a *live*
  single run with its glyph and layout caches; it is the one to size against,
  and it puts 128 KB resident at roughly 22 MB.

## The design

### Chunking model

`TranscriptDocument` (in `TCShellCore`, no SwiftUI) takes the body, keeps one
`[UInt8]` copy of it, and cuts it into chunks. Cut rules, in order:

1. End at the last newline at or before the target, provided that leaves at
   least half a target's worth. A whole number of lines is what a reader
   expects, and a newline can never be inside a redaction marker, so this
   path is safe by construction. This is the path essentially every real
   transcript takes.
2. Otherwise cut at the target and push the cut off any marker it landed
   inside — back to the marker's start if that leaves a non-empty chunk,
   forward past its end if it did not.
3. Then back the cut off any UTF-8 continuation byte, so a chunk always ends
   on a scalar boundary.

**Chunk size: 4 KB** (`TranscriptPaging.targetChunkBytes`). A 60 Hz frame is
0.0167 s; 4 KB is the largest chunk whose layout still fits inside one frame,
so materialising a chunk during a scroll costs at most a frame of jitter. 8 KB
drops a frame and a half every time a chunk comes into view. 2 KB is cheaper
still but doubles the view count — 8,960 rows for a 17.5 MB body against
4,480 — for a difference nobody can perceive.

The hard ceiling on one chunk is `targetChunkBytes + maxMarkerBytes` (4 KB +
256 B), which is what the tests assert against. In practice a 2 MB body of
transcript lines cuts into 518 chunks averaging 4,048 bytes: cutting back to
the last newline costs about 48 bytes a chunk.

### Retention ceiling

**128 KB** (`TranscriptPaging.retainedLimitBytes`) of body text typeset at
once, constant in the size of the trace. This is the number that replaces
`TranscriptBudget.limitBytes`, and it is asserted in tests the same way.

Sized from the viewport: 128 KB is at least 18 screenfuls even on a
full-height display — the visible page plus roughly nine screenfuls of
overscan in each direction, which is what keeps a flick-scroll from outrunning
the window and showing blank space. Refilling all of it from cold costs the
measured 0.221 s, and that only happens on a jump, never on a drag. At the
incident's 170 bytes of process memory per source byte it is about 22 MB, and
it stays there.

### Eviction policy

`TranscriptResidency.window(_:visible:limitBytes:)` returns the chunk range
to keep typeset:

- The visible range is included first and never dropped for overscan. If the
  visible range alone exceeded the ceiling it is trimmed from its far end, so
  the returned window is under the ceiling unconditionally. That trim is not
  expected to fire — 128 KB against a 6.9 KB largest measured screenful — but
  "not expected" is not a bound.
- Overscan is then added one chunk at a time, **alternating below and above**,
  so a reader scrolling either way has the same amount of already-typeset text
  ahead of them. At the ends of the body the budget is spent entirely on the
  side that exists.

`TranscriptResidentChunks<Rendered>` owns the typeset chunks and applies that
window: chunks that fall out are dropped and counted, chunks that come in are
rendered by a caller-supplied closure. It is generic over the rendered type so
the accounting can be tested with `String` and used in the view with
`AttributedString` — the same code path, not a parallel one. It exposes
`retainedBytes` (the number the ceiling is on) and `evictions` (so a test can
prove eviction happened rather than infer it from a count that stopped
growing).

Advancing one chunk renders exactly one chunk and evicts exactly one.

### Placing chunks that are not resident

A chunk that is not typeset still has to hold its place, or the scroll extent
would be the window's rather than the body's. `TranscriptRowIndex` estimates
rows per chunk from its byte count and newline count at the current column
width, and the view draws a `Color.clear` of that height.

The estimate is exact in a monospaced font for any chunk whose lines all fit
the width. It is high by at most one row per chunk when lines wrap (the
wrapped count rounds up at the chunk edge: an unbroken 8,900-byte line at 89
columns estimates 102 rows against a true 100) and low by at most one row per
line for a chunk mixing wrapped and short lines. Either way the error is
bounded per chunk — at most 16 pt of scroll extent per 4 KB — and shows up as
the scrollbar settling slightly as chunks materialise, not as a body of
unknown length.

### Redaction markers

The chip scan used to build one `AttributedString` over the entire body. It
now runs per chunk, over at most 4 KB, and the pattern lives in
`TranscriptMarkerScan` because the chunker needs the same one: a chunker that
protected a different set of markers than the view chips would split exactly
the ones the view cares about.

A marker split across two separately-typeset chunks — `<PRIVATE_SEC` in one
and `RET_1>` in the next — is not a cosmetic problem. Both halves would draw
in body type, and half a marker reads as content that was never scrubbed. Rule
1 above makes this impossible on newline-terminated bodies; rule 2 handles the
minified case; the tests walk a marker byte by byte across a boundary and
require it to come out whole in exactly one chunk.

Two changes to the pattern itself, both narrowing:

- The `[REDACTED…]` arm now excludes newlines as well as `]`. Without that,
  one unclosed bracket anywhere in a body would let a "marker" run to the end
  of the file and the chunker would refuse to cut there.
- Markers longer than `maxMarkerBytes` (256 B) are not protected from
  splitting. Real markers are tens of bytes; this is stated rather than left
  to be discovered.

### Text selection

`textSelection(.enabled)` stays on each chunk. Selection is therefore per
block — one chunk at a time — where it used to span the whole 64 KB slice. A
chunk that is not typeset has nothing to select, so this is intrinsic to
paging rather than a detail that could be fixed with more care.

The replacement is a **Copy everything** button in the tab header, which puts
the entire redacted body on the pasteboard. Copying is a string copy, not a
layout, so it is bounded work regardless of size. This is a deliberate trade:
whole-body *selection* is lost, whole-body *copying* is gained, and copying is
what the selection was for.

### Search

The search itself was never the problem and is left alone: it runs in the
daemon over the raw body and returns UTF-8 byte offsets, so no part of finding
a match is text layout. What it renders is already bounded — at most 20
snippets of at most a match plus 240 bytes.

What was unbounded is now fixed. The context snippets were cut from
`Array(transcript.utf8)`, built **inside a computed property**, which SwiftUI
re-evaluates on every keystroke: a full 17.5 MB copy per character typed.
Snippets now come from `TranscriptDocument.snippet(around:matchBytes:window:)`,
which slices the bytes the document already holds. Both cut ends back off to a
scalar boundary, so a snippet never opens or closes with U+FFFD, and the
function reports whether it elided text on each side rather than the caller
guessing where the ellipses go.

The one cost is the document's byte array: a second copy of the body, 17.5 MB
for the trace that started this, held for the life of the sheet. That is the
trade for turning a per-keystroke copy into a per-sheet one.

## What changed on macOS

- `macos/Sources/TCShellCore/TranscriptPaging.swift` — new. The reference
  implementation: `TranscriptPaging` constants, `TranscriptDocument`,
  `TranscriptMarkerScan`, `TranscriptRowIndex`, `TranscriptResidency`,
  `TranscriptResidentChunks`.
- `macos/Sources/TCShellCore/TranscriptBudget.swift` — deleted, with
  `macos/Tests/TCShellCoreTests/TranscriptBudgetTests.swift`.
- `macos/Sources/TraceCommonsApp/Views/PreviewSheet.swift` — `TranscriptTab`
  rebuilt on a `LazyVStack` of chunk rows with placeholders and eviction; the
  clamped notice and its `transcript-clamped-notice` identifier removed;
  `SearchTab` takes the document instead of the raw string; the sheet builds
  one `TranscriptDocument` when the body arrives.
- `macos/Tests/TCShellCoreTests/TranscriptPagingTests.swift` — new, 27 tests.

The read gate, its copy, and `onFirstScreenShown` are untouched.

## What changed on Windows

Same design, same cut rules, same assertions; two differences forced by the
toolkit, and one honest gap.

- `windows/src/TraceCommons.Interop/TranscriptPaging.cs` — new.
  `TranscriptPaging` constants, `TranscriptDocument`, `TranscriptChunk`,
  `ChunkRange`, `TranscriptRowIndex`, `TranscriptResidency`,
  `TranscriptResidentChunks<T>`, and `TranscriptViewport`.
- `windows/src/TraceCommons.Interop/TranscriptBudget.cs` — deleted, with
  `windows/tests/TraceCommons.Interop.Tests/TranscriptBudgetTests.cs`.
- `windows/src/TraceCommons.Interop/TranscriptMarkers.cs` — the
  `[REDACTED…]` arm now excludes newlines, matching the other shells, and a
  `ByteSpans` entry point converts marker spans to UTF-8 offsets for the
  chunker. One pattern, two callers.
- `windows/src/TraceCommons.App/Controls/PreviewSheet.xaml` and its
  code-behind — the single `RichTextBlock` and the clamp notice are gone,
  replaced by a spacer/chunks/spacer panel and a **Copy everything** button.
- `windows/tests/TraceCommons.Interop.Tests/TranscriptPagingTests.cs` — new,
  32 tests.

**The chunk and retention numbers are inherited from macOS, not measured on
Windows.** The WinUI App project does not build on a Mac and no Windows box
was available, so `RichTextBlock`'s layout curve is unknown: it may be
linear in the length of a run, in which case 4 KB chunks are needlessly
small, or worse than CoreText, in which case they are too large. The four
constants and the two font-metric estimates are named `const`s in one class
with that caveat written on each of them. Someone with a Windows machine
should run the same size sweep and either confirm them or move them; nothing
else has to change when they do.

**Two structural differences from the macOS view:**

1. **Placeholders are two spacers, not one view per chunk.** macOS relies on
   `LazyVStack` to build only the rows near the viewport, so it can afford a
   view per chunk. WinUI's `StackPanel` does not virtualise, and 4,480
   elements for a 17.5 MB body is its own problem. The Windows panel holds a
   top spacer, the resident chunks, and a bottom spacer, so its element count
   is bounded by the retention ceiling regardless of trace size. The spacer
   heights come from the same `TranscriptRowIndex` estimate, and
   `TranscriptViewport.Spacers` is asserted to reproduce the whole body's
   scroll extent.
2. **The anchor is the scroll offset, not the last row to appear.** WinUI has
   no `onAppear`; the residency window is driven from
   `ScrollViewer.VerticalOffset` through the row index. That is arguably more
   robust than the macOS anchor, and it is also the piece most exposed to the
   row estimate being wrong, since a bad estimate moves the window rather
   than just the scrollbar.

**Unverified on Windows:** everything above the model layer. The interop
tests run and pass on a Mac; the assembled sheet has never been built or
scrolled. In particular, whether placeholder-to-text height changes cause
visible scroll jump on a fast flick, whether the re-entrancy guard around
child and spacer updates is sufficient, and whether `RichTextBlock`
per-chunk layout really costs a frame, are all open until someone runs it.

## What the other two shells did

Both are ported. The notice is retired everywhere, so there is no longer a
cross-shell copy contract for it to be part of, and all three budget modules
are deleted in favour of paging ones.

**GTK re-measured and chose different numbers, and the measurement changed
the reasoning.** `GtkTextView` lays out one `PangoLayout` per line, so its
layout is LINEAR -- roughly 250 microseconds per KB, flat from 4 KB to 4 MB.
What freezes that shell is the redaction tag pass, which is quadratic:
0.77 ms at 64 KB, 10.67 ms at 256 KB, 165.95 ms at 1 MB, 2923.85 ms at 4 MB,
extrapolating to about a minute of frozen main loop at 17.5 MB. Rewriting
the scan to a single pass only moves the constant, because `GtkTextBuffer`
offset addressing is not O(1); bounding what gets tagged is the fix. Single-
run layout is superlinear there too, which still matters for a minified body
where one 17.5 MB line is one layout.

So GTK chunks at 16 KB rather than 4 KB -- on that toolkit chunk size does
not change total cost at all, making it pure granularity, and 16 KB
materialises in 3.5 ms while 4 KB would quadruple the widget count to buy
nothing measurable -- and retains 256 KB rather than 128 KB, refilling cold
in 55.8 ms. See `crates/trace-commons-contributor-gtk/src/transcript_paging.rs`,
which carries the tables at the point of use.

**Windows inherited the numbers and says so.** `RichTextBlock` could not be
measured: the WinUI project does not build off Windows. All four constants
sit in one named class whose doc states in bold that they are inherited and
not measured, each constant repeats it, and the two font metrics are marked
derived rather than measured with their derivation shown. If `RichTextBlock`
turns out to be linear in the run, its chunk size is free to be much larger.
See `windows/src/TraceCommons.Interop/TranscriptPaging.cs`.

The rules that did port unchanged, and must stay that way in any future
shell: the cut rules (line boundary preferred, marker-aware fallback,
scalar-aligned) and the marker grammar including the newline exclusion in
the `[REDACTED...]` arm -- an omission that made a single unclosed bracket
stop the chunker from cutting for the rest of the body, found independently
on macOS and on Windows; the assertions (every byte reachable, no chunk over
the ceiling, no split characters, no split markers, retained bytes under the
ceiling *while scrolling*, and a chunk that scrolled away actually gone);
and the read gate, which is the same gate making the same claim on all three.

## What is not verified

- Everything above is measured at the model layer and through AppKit
  harnesses. **The scroll behaviour of the assembled SwiftUI view was not
  observed on a 17.5 MB trace in the running app** — no such trace and no
  interactive session were available here. In particular: whether
  `LazyVStack` re-measures resident chunks on scroll (it should not, since
  the proposed width is constant), and whether placeholder-to-text height
  changes cause visible scroll jump on a fast flick, are unconfirmed.
- The residency anchor is the most recently appeared chunk row. That is
  robust to `LazyVStack` keeping more rows materialised than are visible, but
  it assumes `onAppear` fires for rows as they approach the viewport, which is
  observed behaviour rather than documented contract.
- Actual process memory for the assembled tab under a fast scroll was not
  measured; the ceiling asserted in tests is source bytes handed to layout,
  and the memory figure is derived from the incident's ratio.
