import SwiftUI
import TCDesign

/// A getting-started card, as the Insights empty state draws it (product
/// owner, 2026-11-02: the Private AI tab's guide and Home's Traces
/// walkthrough take the Insights card's design, formatting and layout).
///
/// A title, then an optional line under it, then numbered steps: each a
/// small number in a disc, a title and one short line, with an "i" after
/// the line holding a step's longer explanation, and the step's action, if
/// it has one, as a text link with an arrow under it. Given `close`, a
/// small X in the top trailing corner closes the card. No gradient. Every
/// word is the caller's.
///
/// `pixelField` adds a dithered grid of blues that grows from the card's
/// bottom trailing corner and fades out before its text (owner, 2026-11-02,
/// tried on the Private AI tab's card first, `GlassPixelField`), between
/// the card's fill and its words.
struct GetStartedCard: View {
    struct Step {
        let title: String
        let detail: String
        /// The "i" after the line; empty draws none.
        var info: String = ""
        var link: Link?
    }

    /// A step's one action: a text link with an arrow.
    struct Link {
        let title: String
        let action: () -> Void
    }

    /// The X that closes the card.
    struct Close {
        let label: String
        let action: () -> Void
    }

    let title: String
    /// The line under the title; empty draws none.
    var lead: String = ""
    let steps: [Step]
    var close: Close?
    var pixelField = false
    /// The field's shape, so one card can be told from another.
    var fieldPattern: GlassPixelField.Pattern = .corner

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s9) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    // The title's type is a hero card's (Ron, 2026-11-05: as
                    // Home's promo cards), and so is the line's under it.
                    Text(title)
                        .glassType(GlassTokens.TypeScale.display.weight(.heavy))
                        .foregroundStyle(GlassColor.textPrimary)
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    if !lead.isEmpty {
                        Text(lead).insightsNote()
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                if let close { GetStartedCardClose(close: close) }
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s9) {
                ForEach(Array(steps.enumerated()), id: \.offset) { index, step in
                    GetStartedCardStep(number: index + 1, step: step)
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .background { if pixelField { GetStartedCardField(pattern: fieldPattern) } }
        .insightsSection()
    }
}

/// The pixel field, out to the card's own edges and corners: the card's
/// content sits inside its padding, so the field reaches back out by that
/// padding and is cut to the card's corner radius.
struct GetStartedCardField: View {
    /// Three nearby blues: the kept blue, the chart blue and first run's
    /// pale beam core, which the cells drift between.
    static let palette = [GlassTokens.Color.dataKept, GlassTokens.Color.chartSeries1,
                          GlassTokens.Color.firstRunBeamCore]
    var pattern: GlassPixelField.Pattern = .corner

    var body: some View {
        GlassPixelField(palette: Self.palette, pattern: pattern)
            .clipShape(RoundedRectangle(cornerRadius: GlassTokens.Radius.card, style: .continuous))
            .padding(.horizontal, -GlassTokens.Space.cardPaddingHorizontal)
            .padding(.vertical, -GlassTokens.Space.cardPaddingVertical)
    }
}

/// One numbered step: its number in a disc, its title and its line, with
/// the "i" holding its longer explanation after the line's last word, and
/// its link under them.
private struct GetStartedCardStep: View {
    let number: Int
    let step: GetStartedCard.Step

    var body: some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s5) {
            Text(String(number))
                .glassType(GlassTokens.TypeScale.micro)
                .monospacedDigit()
                .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                .frame(width: 18, height: 18)
                .background(Circle().fill(GlassTokens.Color.chartSeries1.color.opacity(0.14)))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(step.title).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                InsightsTextWithInfo(step.detail, style: GlassTokens.TypeScale.label.weight(.regular),
                                     infoTitle: step.title, info: [step.info])
                    .foregroundStyle(GlassColor.textSecondary)
                if let link = step.link {
                    Button(action: link.action) {
                        HStack(spacing: GlassTokens.Space.s2) {
                            Text(link.title)
                            Image(systemName: "arrow.right").accessibilityHidden(true)
                        }
                    }
                    .buttonStyle(GlassButtonStyle(.link))
                    .padding(.top, GlassTokens.Space.s3)
                }
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
    }
}

/// The card's X: a bare glyph in the secondary ink, primary under the
/// pointer, as a hero card's X is drawn on its own ground.
struct GetStartedCardClose: View {
    let close: GetStartedCard.Close
    @State private var hovering = false

    var body: some View {
        Button(action: close.action) {
            Image(systemName: "xmark")
                .glassGlyph(12, weight: .semibold)
                .foregroundStyle(hovering ? GlassColor.textPrimary : GlassColor.textSecondary)
                .frame(width: 20, height: 20)
                .contentShape(Rectangle())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityLabel(close.label)
        .help(close.label)
    }
}
