import SwiftUI

/// A tab's own heading row, at the top of its content (owner, 2026-10-19:
/// one style on every tab): the title in the card-title face, primary ink,
/// on one line at the content's leading edge, then whatever the tab puts
/// beside it. Its height is the round glass controls', so a row with
/// buttons and a row with a status line sit the same distance under the
/// tab bar. The Traces list's "Watched Directories" and the Private AI
/// status use it.
public struct GlassTabHeading<Accessory: View>: View {
    private let title: String
    private let accessory: Accessory

    public init(_ title: String, @ViewBuilder accessory: () -> Accessory) {
        self.title = title
        self.accessory = accessory()
    }

    /// The title's face: the cards' title step (owner, 2026-10-11: no
    /// larger than the app's other titles).
    public static var titleType: GlassTypeStyle { GlassTokens.TypeScale.bodyStrong }

    public var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            Text(title)
                .glassType(Self.titleType)
                .foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
                .accessibilityAddTraits(.isHeader)
            accessory
        }
        .frame(minHeight: GlassTokens.Size.controlLarge)
    }
}
