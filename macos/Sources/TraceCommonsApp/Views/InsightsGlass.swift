import SwiftUI
import TCDesign

/// The glass parts the Insights, comparison and mission-draft screens are
/// drawn with: the type scale and inks for their text, their disclosures as
/// `GlassExpander`, their cards as `GlassCard`, and their buttons and
/// checkboxes in the glass styles. Every word stays the core's copy table's.
extension View {
    /// A screen's title.
    func insightsTitle() -> some View {
        glassType(GlassTokens.TypeScale.title).foregroundStyle(GlassColor.textPrimary)
    }

    /// A section heading.
    func insightsHeading() -> some View {
        glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
    }

    /// A notice beside the content it qualifies.
    func insightsNote() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassColor.textSecondary)
    }

    /// A caption.
    func insightsCaption() -> some View {
        glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
    }

    /// An identifier or a digest.
    func insightsMono() -> some View {
        glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textSecondary)
    }

    /// A failure, in the outside red's text ink.
    func insightsError() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassStatus.outside.textColor)
    }

    /// A success, in the on green's text ink.
    func insightsSuccess() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassStatus.on.textColor)
    }

    /// A quiet glass card around this content.
    func insightsCard() -> some View {
        GlassCard(quiet: true) { self }
    }

    /// The label of a row that acts: a quiet card that lifts under the
    /// pointer. Its button takes `GlassPressStyle`.
    func insightsRowCard() -> some View {
        GlassCard(quiet: true, interactive: true) { self }
    }

    /// A screen's defaults: body type in the primary ink, glass buttons
    /// and glass checkboxes for every control that does not choose its own.
    func insightsSurface() -> some View {
        glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.textPrimary)
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .toggleStyle(GlassCheckboxStyle())
    }
}

/// A disclosure: `GlassExpander` and, while open, its content indented
/// under it. The replacement for a stock `DisclosureGroup`.
struct InsightsDisclosure<Content: View>: View {
    let title: String
    @ViewBuilder let content: () -> Content
    @State private var isOpen = false

    init(_ title: String, @ViewBuilder content: @escaping () -> Content) {
        self.title = title
        self.content = content
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            GlassExpander(title, isOpen: $isOpen)
            if isOpen {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    content()
                }
                .padding(.leading, GlassTokens.Space.s8)
            }
        }
    }
}

/// The hairline between a screen's sections.
struct InsightsRule: View {
    var body: some View {
        GlassHairline(GlassColor.hairline)
    }
}
