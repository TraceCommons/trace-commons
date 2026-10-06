import SwiftUI
import TCDesign

/// A yes/no Settings fact. The state is a glyph for sighted readers and the
/// words "title: yes" / "title: no" for VoiceOver, as the legacy check row
/// did; colour alone never carries it.
struct SettingsStateRow: View {
    let title: String
    let isOn: Bool

    var body: some View {
        HStack(spacing: GlassTokens.Space.s3) {
            Image(systemName: isOn ? "checkmark.circle.fill" : "circle")
                .accessibilityHidden(true)
            Text(title)
        }
        .glassType(GlassTokens.TypeScale.label)
        .foregroundStyle(GlassColor.textSecondary)
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(SettingsLegacyWords.stateLabel(title, isOn))
    }
}

/// What a section draws where the daemon has not answered yet: the glass
/// spinner (R-20), which VoiceOver reads as a native progress indicator.
/// Never an empty card, never a value read from a default.
struct SettingsAwaiting: View {
    var body: some View {
        GlassSpinner(standalone: true)
    }
}

extension DaemonStatus {
    /// False until the daemon has answered `status` at all. `.unknown` is
    /// the placeholder held before the first answer, and a real status
    /// always carries a schema version, so the two cannot be confused.
    var answered: Bool { self != .unknown }
}
