import SwiftUI
import TCDesign
import TCShellCore

/// Home's Traces hero (product owner, 2026-10-28, 2026-10-29; restored
/// 2026-11-05): the first and largest card on Home, saying what
/// contributing is and that it earns credit, with Start earning on its
/// pill. Under its line, the three steps across the whole card:
/// contribute, earn, use Private AI. It shows for everyone, new and
/// returning, until its X closes it for good on this Mac (owner,
/// 2026-11-07).
///
/// When the full card would push the stats under it out of view, it goes
/// compact (product owner, 2026-11-07): a smaller title, no line under
/// it, and each step on one short row, so the stats show without
/// scrolling. That is a narrow card, or a Home too short for the full
/// card and the first row of stats. Its X closes it, the
/// person's choice (product owner, 2026-11-07); the choice is kept under
/// its own key, so a card closed in an older version does not stay hidden.
///
/// This card is the product owner's design. Changes to this file need her
/// review (`.github/CODEOWNERS`), and `HomeTracesHeroTests` pins it.
struct HomeTracesHero: View {
    let words: MonitorHomeHistoryCopy
    let openTraces: () -> Void
    let openPrivateAI: () -> Void
    /// How tall Home's visible area is; zero until Home has said.
    var visibleHeight: CGFloat = 0
    /// The card's width, read as it lays out.
    @State private var width: CGFloat = 0
    /// The full card's height at the card's width, from a hidden full copy,
    /// so it is known while the card is compact too; zero until measured.
    @State private var fullHeight: CGFloat = 0
    /// The person closed the card with its X.
    @AppStorage(Self.closedKey) private var closed = false

    /// Not `home.tracesHeroClosed`, which older versions wrote.
    static let closedKey = "home.stepsCardClosed"

    /// Below this width the card is compact.
    static var compactWidth: CGFloat { 560 }
    /// What must fit under the full card for the stats to show: the
    /// no-credit line, the Traces heading and the first row of tiles.
    static var statsRoom: CGFloat { 200 }

    static func isCompact(width: CGFloat, visibleHeight: CGFloat, fullHeight: CGFloat) -> Bool {
        if width > 0 && width < compactWidth { return true }
        return visibleHeight > 0 && fullHeight > 0 && fullHeight + statsRoom > visibleHeight
    }

    var body: some View {
        if !closed { card }
    }

    private var card: some View {
        let compact = Self.isCompact(width: width, visibleHeight: visibleHeight, fullHeight: fullHeight)
        return hero(compact: compact)
            .onGeometryChange(for: CGFloat.self, of: \.size.width) { width = $0 }
            .background(alignment: .top) {
                // The full card at this width, never drawn or read, for its
                // height.
                hero(compact: false)
                    .fixedSize(horizontal: false, vertical: true)
                    .hidden()
                    .accessibilityHidden(true)
                    .onGeometryChange(for: CGFloat.self, of: \.size.height) { fullHeight = $0 }
            }
    }

    private func hero(compact: Bool) -> some View {
        GlassHeroCard(
            gradient: GlassTokens.Gradient.heroMissions,
            title: words.tracesHeroTitle, subtitle: words.tracesHeroBody,
            actionTitle: words.tracesHeroAction, action: openTraces,
            closeLabel: MonitorWords.close, onClose: { closed = true },
            contentSpansGraphic: true, actionOnPill: true, compact: compact
        ) {
            HomeHeroRing()
        } content: {
            HomeHeroSteps(compact: compact, steps: [
                .init(number: 1, systemImage: "apple.terminal",
                      title: words.tracesHeroStepContribute, detail: words.tracesHeroStepContributeDetail,
                      action: openTraces),
                .init(number: 2, systemImage: "cylinder.split.1x2",
                      title: words.tracesHeroStepEarn, detail: words.tracesHeroStepEarnDetail),
                .init(number: 3, systemImage: "sparkles",
                      title: words.tracesHeroStepUse, detail: words.tracesHeroStepUseDetail,
                      action: openPrivateAI),
            ])
        }
    }
}

/// The Traces hero's three steps (owner, 2026-10-29): side by side, an
/// arrow between each, each a translucent panel on the hero's ground with
/// a numbered disc and an icon on one row, then a title and one line
/// across the panel's width. All in heroInk, as every word on a hero is;
/// the discs and icons are heroAccent, the owner's mock-up's blue. The
/// panels are as tall as the tallest. Contribute opens Traces and Use
/// Private AI opens Private AI, each panel a button of its own; Earn
/// credit stays on Home, a plain panel (owner, 2026-10-29).
struct HomeHeroSteps: View {
    struct Step: Identifiable {
        let number: Int
        let systemImage: String
        let title: String
        let detail: String
        /// Opens the step's tab; nil draws a plain panel.
        var action: (() -> Void)? = nil
        var id: Int { number }
    }

    /// One short row per step, number and title only, for a narrow card.
    var compact = false
    let steps: [Step]

    static var disc: CGFloat { 30 }
    static var icon: CGFloat { 28 }
    /// The compact rows' number disc.
    static var compactDisc: CGFloat { 22 }

    var body: some View {
        if compact {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                ForEach(steps) { HomeHeroStepRow(step: $0) }
            }
            .padding(.top, GlassTokens.Space.s1)
        } else {
            wide
        }
    }

    private var wide: some View {
        let ink = GlassTokens.Color.heroInk.color
        return HStack(alignment: .center, spacing: GlassTokens.Space.s2) {
            ForEach(steps) { step in
                if step.number > 1 {
                    Image(systemName: "arrow.right")
                        .glassGlyph(14, weight: .semibold)
                        .foregroundStyle(ink.opacity(0.6))
                        .accessibilityHidden(true)
                }
                HomeHeroStepPanel(step: step)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.top, GlassTokens.Space.s2)
    }
}

/// One of the Traces hero's steps: a translucent panel, brighter under the
/// pointer when it opens a tab, and then a button that reads its title and
/// line to VoiceOver as one.
struct HomeHeroStepPanel: View {
    let step: HomeHeroSteps.Step
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if let action = step.action {
            Button(action: action) { panel }
                .buttonStyle(GlassPressStyle())
                .onHover { inside in
                    withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }
                }
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
        } else {
            panel.accessibilityElement(children: .combine)
        }
    }

    private var panel: some View {
        let ink = GlassTokens.Color.heroInk.color
        let shape = RoundedRectangle(cornerRadius: GlassTier.cardQuiet.radius, style: .continuous)
        // The disc and icon share the top row, so the title and line take
        // the panel's whole width.
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s2) {
                Text("\(step.number)")
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(ink)
                    .frame(width: HomeHeroSteps.disc, height: HomeHeroSteps.disc)
                    .background(Circle().fill(GlassTokens.Color.heroAccent.color))
                    .accessibilityHidden(true)
                Image(systemName: step.systemImage)
                    .glassGlyph(HomeHeroSteps.icon, weight: .regular)
                    .foregroundStyle(GlassTokens.Color.heroAccent.color)
                    // One height for every icon, so the titles line up.
                    .frame(height: HomeHeroSteps.icon + 6, alignment: .leading)
                    .accessibilityHidden(true)
            }
            .padding(.bottom, GlassTokens.Space.s1)
            Text(step.title)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(ink)
                .fixedSize(horizontal: false, vertical: true)
            Text(step.detail)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(ink.opacity(0.8))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(GlassTokens.Space.s3)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(shape.fill(ink.opacity(hovering ? 0.16 : 0.08)))
        .contentShape(shape)
        .overlay(shape.strokeBorder(ink.opacity(0.18), lineWidth: 1))
    }
}

/// A compact step (owner, 2026-11-07): the numbered disc and the title on
/// one short row, a chevron on a step that opens a tab, brighter under the
/// pointer, and then a button that reads the title and line to VoiceOver.
struct HomeHeroStepRow: View {
    let step: HomeHeroSteps.Step
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if let action = step.action {
            Button(action: action) { row }
                .buttonStyle(GlassPressStyle())
                .onHover { inside in
                    withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }
                }
                .accessibilityElement(children: .combine)
                .accessibilityLabel("\(step.title), \(step.detail)")
                .accessibilityAddTraits(.isButton)
        } else {
            row.accessibilityElement(children: .combine)
                .accessibilityLabel("\(step.title), \(step.detail)")
        }
    }

    private var row: some View {
        let ink = GlassTokens.Color.heroInk.color
        let shape = RoundedRectangle(cornerRadius: GlassTier.cardQuiet.radius, style: .continuous)
        return HStack(spacing: GlassTokens.Space.s2) {
            Text("\(step.number)")
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .foregroundStyle(ink)
                .frame(width: HomeHeroSteps.compactDisc, height: HomeHeroSteps.compactDisc)
                .background(Circle().fill(GlassTokens.Color.heroAccent.color))
            Text(step.title)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(ink)
                .lineLimit(1)
            Spacer(minLength: 0)
            if step.action != nil {
                Image(systemName: "chevron.right")
                    .glassGlyph(11, weight: .semibold)
                    .foregroundStyle(ink.opacity(0.7))
            }
        }
        .padding(.horizontal, GlassTokens.Space.s2)
        .padding(.vertical, 6)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(shape.fill(ink.opacity(hovering ? 0.16 : 0.08)))
        .contentShape(shape)
    }
}

/// The Traces hero's graphic (owner, 2026-10-29, from the owner's
/// mock-up): one faint ring past the card's trailing edge, behind the
/// steps.
struct HomeHeroRing: View {
    static var side: CGFloat { 300 }
    static var stroke: CGFloat { 36 }

    var body: some View {
        Circle()
            .strokeBorder(GlassTokens.Color.heroInk.color.opacity(0.14), lineWidth: Self.stroke)
            .frame(width: Self.side, height: Self.side)
            .offset(x: 90, y: -30)
    }
}
