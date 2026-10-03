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
