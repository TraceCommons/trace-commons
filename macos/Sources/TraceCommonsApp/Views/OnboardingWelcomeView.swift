import SwiftUI

/// Onboarding screen 1, "What this is" -- the first thing a contributor ever
/// sees. Copy is verbatim from the shared design spec
/// (`docs/superpowers/specs/2026-08-08-contributor-shell-shared-design.md`,
/// "## Onboarding", "### 1. What this is") and from `design-import/DESIGN-SPEC.md`
/// section 5.8, not paraphrased.
///
/// The line "That scrubbing is good and it is not perfect -- which is why
/// you get to look first" is load-bearing and must not be softened: a
/// developer already knows automatic redaction is imperfect, and conceding
/// it before they ask is what makes every later claim in this app credible.
/// Do not reword it, and do not cut it for space.
struct OnboardingWelcomeView: View {
    var onGetStarted: () -> Void
    var onWhatGetsRemoved: () -> Void

    var body: some View {
        ScrollView {
            OnboardingWelcomeContent(onGetStarted: onGetStarted, onWhatGetsRemoved: onWhatGetsRemoved)
        }
        .background(TC.ground)
    }
}

/// The screen's content, split out of its `ScrollView` for the same reason
/// `ConsentScopesContent` is split out of `ConsentScopesView`: `ImageRenderer`
/// renders a `ScrollView` as blank.
///
/// ## Why this screen is built differently from every other one
///
/// It is drawn in the COMMUNITY brand rather than in the native token layer,
/// and that seam is deliberate. `design-import/DESIGN-SPEC.md` section 5.8
/// moves the site's hero language into the first-run window: a 2px ink frame
/// around a white page, Helvetica, display type at landing scale (uppercase,
/// tight tracking, .88 line height), the promise line carried on the mint
/// highlight the site uses for live nav, mono uppercase micro-labels, and the
/// wireframe globe. Everywhere past this flow the app is the quiet native
/// tool; this is the one frame that speaks in the commons' voice, because the
/// argument being made here is the commons' argument, not the tool's.
///
/// The globe is STILL. Section 4.5's one-motion rule says the only animation
/// in the desktop app is the mark's own assembly, which is what
/// `BrandMarkIntro` in the header bar is; the globe "only turns on the
/// website".
///
/// ## Copy
///
/// Every sentence is the spec's, unchanged. One is MOVED: "You decide what
/// gets contributed. Nothing is sent unless you say so." was set bold inside
/// the second paragraph, where it was the most important claim on the screen
/// and the least likely to be read. It is now the headline, which is what
/// bold inside a paragraph was trying and failing to do. The paragraph it
/// came from still reads as a complete sentence without it.
///
/// The scrubbing concession -- "That scrubbing is good and it is not perfect
/// -- which is why you get to look first" -- stays verbatim, stays on this
/// screen, and is not demoted into the small print. It is what makes every
/// later claim credible.
struct OnboardingWelcomeContent: View {
    var onGetStarted: () -> Void
    var onWhatGetsRemoved: () -> Void

    /// Landing scale, but not fixed: `@ScaledMetric` means the accessibility
    /// text sizes still move it, which a hardcoded 50 would not.
    @ScaledMetric(relativeTo: .largeTitle) private var displaySize: CGFloat
        = CommunityBrand.Font_.heroSize

    var body: some View {
        VStack(alignment: .leading, spacing: CommunityBrand.Metric.pageGap) {
            header
            hero
            supporting
            footer
        }
        .padding(CommunityBrand.Metric.pagePadding)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(CommunityBrand.paper)
        .overlay {
            // The 2px ink frame. It is the brand's only container: no radius,
            // no shadow, no inner hairline.
            Rectangle().strokeBorder(CommunityBrand.ink, lineWidth: CommunityBrand.Metric.frame)
        }
        .padding(.horizontal, TC.Space.lg)
        .padding(.bottom, TC.Space.lg)
        .frame(maxWidth: CommunityBrand.Metric.pageWidth, alignment: .leading)
        .frame(maxWidth: .infinity, alignment: .top)
        // The community brand is declared light-only (spec section 7.2 item
        // 4). Pinning the scheme keeps the mark's light drawing and the ink
        // hairlines correct even when the rest of the app is in Dark Mode.
        .environment(\.colorScheme, .light)
    }

    /// Header bar: the mark and the wordmark on the left, the one link on the
    /// right, ruled off underneath in 2px ink.
    ///
    /// The mark is The Turn, not the circuit/solder-dot mark that section
    /// 5.8's frame still draws. That frame predates the decision recorded in
    /// section 1.1; the circuit mark is now the community *website* mark only,
    /// and section 7.2 item 5 says to treat The Turn as correct everywhere.
    private var header: some View {
        VStack(alignment: .leading, spacing: CommunityBrand.Metric.headerRule) {
            HStack(alignment: .center, spacing: TC.Space.sm) {
                BrandMarkIntro(size: 26, variant: .light)
                Text("Trace Commons — Contributor")
                    .font(CommunityBrand.Font_.chromeMono)
                    .tracking(CommunityBrand.Font_.monoTracking)
                    .foregroundStyle(CommunityBrand.ink)
                Spacer(minLength: TC.Space.m)
            }
            Rectangle().fill(CommunityBrand.ink).frame(height: CommunityBrand.Metric.frame)
        }
    }

    /// The argument on the left at display size, the globe on the right --
    /// at the largest of a measured ladder of sizes that actually fits.
    ///
    /// ## Why this is a ladder and not one layout
    ///
    /// The spec states the headline at 50pt. Measured (`NSAttributedString`
    /// with Helvetica Neue Bold at `heroTrackingEm`), its longest line, "YOU
    /// DECIDE WHAT GETS CONTRIBUTED.", is **948pt** wide at 50pt. The brand
    /// page is 860pt on its widest canvas and the globe takes 230 of it, so
    /// that line does not fit beside the globe at 50pt in any window this app
    /// can open -- not at the spec's own canvas size. It is not a matter of
    /// the window being too small today; the number does not work.
    ///
    /// So the size is chosen rather than asserted. `ViewThatFits` walks the
    /// ladder below and takes the first rung whose stated width fits, and
    /// every rung's column width is the measured width of that longest line
    /// at that size (plus the 4pt highlight padding on each side). Because
    /// each rung declares an explicit width, its ideal size is exact and the
    /// choice is made in one layout pass -- which also means it works under
    /// `ImageRenderer`, where a `GeometryReader`-plus-state approach would
    /// render at the default and never update.
    ///
    /// ## What the hero can actually be
    ///
    /// The page is capped at the spec's 860pt canvas, and section 5.8 spends
    /// that on `margin:0 18px 18px` (36), the 2px frame (4) and `padding:12px`
    /// (24), leaving 796 inside the frame and 780 across the hero after its
    /// own 8pt inset. So 780 -- not 860, and nowhere near 948 -- is the widest
    /// the headline can ever be, on any display.
    ///
    /// That is what forces the one deviation from section 5.8's arrangement.
    /// The spec puts headline, lede and button together in a left column with
    /// the globe in a right column, which means the headline pays for the
    /// globe's 230pt out of its own measure. The line breaks are fixed by the
    /// copy -- "You decide what gets contributed." is one line whatever the
    /// column does -- so a narrower column cannot make that line need less
    /// width; it can only make the type smaller. Sharing the column costs
    /// roughly a third of the headline's size, and at the narrower canvas it
    /// costs the globe entirely.
    ///
    /// So the headline spans the full measure and the split starts underneath
    /// it: the lede and the button on the left, the globe on the right, still
    /// a two-column hero with the illustration on the right. That is the only
    /// arrangement in which both survive at these widths, and it is why the
    /// globe is back at every size.
    ///
    /// The headline ladder (measured widths, `+8` for the highlight padding):
    ///
    ///     size   width   fits
    ///     42     797     --
    ///     36     691     the 860 canvas (780)
    ///     30     577     the 660 capture (580)
    ///     26     501
    ///     22     426
    ///     18     350
    ///
    /// And the row below it, lede column plus 30pt gap plus globe:
    ///
    ///     lede   globe   total   fits
    ///     560    170      760    the 860 canvas (780)
    ///     400    145      575    the 660 capture (580)
    ///     330    130      490
    ///     300     --      300    the last resort, and the only rung with
    ///                            no globe at all
    private var hero: some View {
        VStack(alignment: .leading, spacing: TC.Space.xl) {
            ViewThatFits(in: .horizontal) {
                headline(42 * displayScale)
                headline(36 * displayScale)
                headline(30 * displayScale)
                headline(26 * displayScale)
                headline(22 * displayScale)
                headline(18 * displayScale)
            }

            ViewThatFits(in: .horizontal) {
                heroBody(column: 560, globe: 170)
                heroBody(column: 400, globe: 145)
                heroBody(column: 330, globe: 130)
                heroBody(column: 300, globe: nil)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, TC.Space.s)
    }

    /// The lede and the button on the left, the globe on the right. Both
    /// widths are stated explicitly so the rung's ideal size is exact and
    /// `ViewThatFits` can choose in a single layout pass -- a `Text` left to
    /// itself reports its whole unwrapped string as its ideal width, which
    /// would make every rung look far too wide to fit.
    private func heroBody(column: CGFloat, globe: CGFloat?) -> some View {
        HStack(alignment: .center, spacing: CommunityBrand.Metric.heroGap) {
            VStack(alignment: .leading, spacing: TC.Space.xl) {
                Text("""
                Coding agents get better when there are real transcripts to learn \
                from. Almost all of that data is locked inside companies. Trace \
                Commons is a shared pool that isn't.
                """)
                .font(CommunityBrand.Font_.lede)
                .lineSpacing(TC.Font_.LineHeight.spacing(for: 18, 1.3))
                .tracking(-0.18)
                .foregroundStyle(CommunityBrand.ink)
                .fixedSize(horizontal: false, vertical: true)

                Button("Get started", action: onGetStarted)
                    .buttonStyle(
                        CommunityBrandButtonStyle(
                            fill: CommunityBrand.accent,
                            size: .onboarding
                        )
                    )
                    .keyboardShortcut(.defaultAction)
            }
            .frame(width: column, alignment: .leading)

            if let globe {
                WireframeGlobe(size: globe)
            }
        }
    }

    /// How far the accessibility text sizes have moved the display type from
    /// the spec's 50pt base.
    private var displayScale: CGFloat { displaySize / CommunityBrand.Font_.heroSize }

    /// Three lines, the last two carried on the mint highlight the site uses
    /// for live nav.
    ///
    /// The lines are laid out one per `Text` rather than as one string with
    /// newlines because the highlight is a per-line block on the site
    /// (`background:#00d4aa; padding:0 4px`), and a single `Text` cannot draw
    /// a background that stops at the end of each line.
    ///
    /// ## The arithmetic behind the line height
    ///
    /// Helvetica Neue Bold, measured: line box **1.221em** (ascender .975,
    /// descender -.246), cap height **.714em**. The target line height is
    /// .88em. Reaching it by subtracting from the stack spacing needs
    /// `.88 - 1.221 = -.341em`, and at that spacing each line's background --
    /// which is the size of its 1.221em line box, not of its .88em line --
    /// overlaps the line above it by .341em, which is more than the .246em of
    /// empty descender space beneath the caps. The mint block of line 2 then
    /// paints over the bottom .095em of line 1's capitals. That is the
    /// overlap: not the glyphs colliding, the background covering them.
    ///
    /// So the line box is set instead of the gap. Each line gets an explicit
    /// `.88em` frame, and the stack spacing is zero: the boxes butt exactly at
    /// the .88 pitch, no background reaches its neighbour, and the caps
    /// (.714em) sit centred with .083em of clearance top and bottom. The type
    /// overflows its frame by .17em at each end, but only into empty space --
    /// uppercase has no descenders -- so nothing is clipped and nothing is
    /// covered. This is also what a browser does with `line-height:.88`: the
    /// inline background box is the font's content area, and the lines advance
    /// by the line height.
    private func headline(_ size: CGFloat) -> some View {
        VStack(alignment: .leading, spacing: 0) {
            headlineLine("You decide what gets contributed.", size: size, highlighted: false)
            headlineLine("Nothing is sent", size: size, highlighted: true)
            headlineLine("unless you say so.", size: size, highlighted: true)
        }
        .accessibilityElement(children: .combine)
        .accessibilityLabel("You decide what gets contributed. Nothing is sent unless you say so.")
    }

    private func headlineLine(_ text: String, size: CGFloat, highlighted: Bool) -> some View {
        Text(text.uppercased())
            .font(CommunityBrand.Font_.display(size))
            .tracking(size * CommunityBrand.Font_.heroTrackingEm)
            .foregroundStyle(CommunityBrand.ink)
            .lineLimit(1)
            // The rung this line belongs to was chosen because the line fits
            // at this size, so there is nothing to scale down to and nothing
            // to truncate. `.fixedSize` says so to the layout rather than
            // leaving an ellipsis as the fallback.
            .fixedSize()
            .frame(height: size * CommunityBrand.Font_.heroLineHeightEm, alignment: .center)
            .padding(.horizontal, TC.Space.xxs)
            .background(highlighted ? CommunityBrand.accent : Color.clear)
    }

    /// The two sentences that say what the app actually does, in brand body
    /// type. Section 5.8 does not draw them -- it draws the hero and stops --
    /// but the scrubbing concession is not droppable, so they stay, set at
    /// `body.brand` under the hero rather than competing with it.
    private var supporting: some View {
        HStack(alignment: .top, spacing: CommunityBrand.Metric.heroGap) {
            Text("""
            This app watches for finished Claude Code and Codex sessions on this \
            machine and shows them to you.
            """)
            .frame(maxWidth: .infinity, alignment: .leading)

            // The link sits directly under the sentence that raises the
            // question, not in the header where it started. Beside the
            // wordmark it was chrome, a page-width away from the paragraph
            // that makes anyone want it -- and on this screen the question
            // "removed how, exactly?" arises here or nowhere.
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                Text("""
                Before anything leaves this machine it is scrubbed locally for secrets, \
                keys, and tokens. That scrubbing is good and it is not perfect — which \
                is why you get to look first.
                """)
                Button("What gets removed?", action: onWhatGetsRemoved)
                    .buttonStyle(.plain)
                    .font(CommunityBrand.Font_.linkMono)
                    .underline()
                    .foregroundStyle(CommunityBrand.ink)
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .font(CommunityBrand.Font_.body)
        .lineSpacing(TC.Font_.LineHeight.spacing(for: 13, 1.45))
        .foregroundStyle(CommunityBrand.ink)
        .fixedSize(horizontal: false, vertical: true)
        .padding(.horizontal, TC.Space.s)
    }

    /// The promise, restated small, and the step counter. Both in the mono
    /// micro-label; a 1px ink rule above, not the header's 2px.
    private var footer: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Rectangle().fill(CommunityBrand.ink).frame(height: TC.Space.hairline)
            HStack(alignment: .firstTextBaseline) {
                Text("Scrubbed locally · shown to you · sent only on your word")
                Spacer(minLength: TC.Space.m)
                // Screen 1 of the six-step flow owned by
                // `OnboardingCoordinatorView`. This screen is only ever
                // step 1, so the counter is stated rather than plumbed.
                Text("01 — 06")
            }
            .font(CommunityBrand.Font_.labelMono)
            .tracking(CommunityBrand.Font_.monoTracking)
            .foregroundStyle(CommunityBrand.muted)
        }
    }
}

// MARK: - The globe

/// The wireframe globe from section 5.8, transcribed from its `0 0 200 200`
/// viewBox so it stays exact at any size.
///
/// Two things about it are load-bearing. It is STILL -- the one-motion rule
/// gives the app exactly one animation, the mark's assembly, and the globe
/// turns only on the website. And its mint is a MINORITY: exactly one of the
/// five ellipses, one of the two signal arcs and one of the four nodes carry
/// it, everything else is ink. A globe drawn entirely in the accent reads as
/// a logo; drawn like this it reads as a diagram with something happening in
/// one corner of it.
private struct WireframeGlobe: View {
    var size: CGFloat = 230

    var body: some View {
        ZStack {
            ellipse(rx: 86, ry: 86, ink: CommunityBrand.ink, width: 1.5)
            ellipse(rx: 86, ry: 30, ink: CommunityBrand.ink, width: 1.2)
            ellipse(rx: 86, ry: 62, ink: CommunityBrand.accent, width: 1.5)
            ellipse(rx: 30, ry: 86, ink: CommunityBrand.ink, width: 1.2)
            ellipse(rx: 62, ry: 86, ink: CommunityBrand.ink, width: 1.2)

            arc(
                from: CGPoint(x: 40, y: 68),
                control: CGPoint(x: 100, y: 8),
                to: CGPoint(x: 162, y: 82),
                ink: CommunityBrand.ink
            )
            arc(
                from: CGPoint(x: 52, y: 148),
                control: CGPoint(x: 120, y: 190),
                to: CGPoint(x: 168, y: 118),
                ink: CommunityBrand.rim
            )

            node(x: 36, y: 64, filled: false)
            node(x: 158, y: 78, filled: false)
            node(x: 48, y: 144, filled: true)
            node(x: 164, y: 114, filled: false)
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }

    /// One unit of the 200-unit viewBox, in points.
    private var unit: CGFloat { size / 200 }

    private func ellipse(rx: CGFloat, ry: CGFloat, ink: Color, width: CGFloat) -> some View {
        Ellipse()
            .strokeBorder(ink, lineWidth: width * unit)
            .frame(width: rx * 2 * unit, height: ry * 2 * unit)
    }

    private func arc(from: CGPoint, control: CGPoint, to: CGPoint, ink: Color) -> some View {
        SignalArc(start: from, control: control, end: to)
            .stroke(
                ink,
                style: StrokeStyle(lineWidth: 1.5 * unit, dash: [4 * unit, 3 * unit])
            )
    }

    /// A 7x7 node on the globe's surface, positioned by its top-left corner
    /// the way the source SVG states it.
    private func node(x: CGFloat, y: CGFloat, filled: Bool) -> some View {
        Group {
            if filled {
                Rectangle().fill(CommunityBrand.rim)
            } else {
                Rectangle().strokeBorder(CommunityBrand.ink, lineWidth: 1.5 * unit)
            }
        }
        .frame(width: 7 * unit, height: 7 * unit)
        .position(x: (x + 3.5) * unit, y: (y + 3.5) * unit)
    }
}

/// One dashed signal arc, in the globe's 200-unit space.
private struct SignalArc: Shape {
    let start: CGPoint
    let control: CGPoint
    let end: CGPoint

    func path(in rect: CGRect) -> Path {
        let unit = min(rect.width, rect.height) / 200
        func place(_ point: CGPoint) -> CGPoint {
            CGPoint(x: rect.minX + point.x * unit, y: rect.minY + point.y * unit)
        }
        var path = Path()
        path.move(to: place(start))
        path.addQuadCurve(to: place(end), control: place(control))
        return path
    }
}

#Preview("Onboarding welcome") {
    OnboardingWelcomeContent(onGetStarted: {}, onWhatGetsRemoved: {})
        .frame(width: 860)
        .background(TC.ground)
}
