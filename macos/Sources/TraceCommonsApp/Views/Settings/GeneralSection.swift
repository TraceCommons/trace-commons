import SwiftUI
import TCDesign

/// General: the appearance, Light, Dark or the system's (design
/// exploration). Under the flat glass theme Light is its light tint and Dark
/// its dark tint; under classic they are the classic light and dark. The
/// choice applies at once and is kept for the next launch.
struct GeneralSection: View {
    @State private var appearance = GlassAppearance.stored

    var body: some View {
        GlassEyebrowCard(SettingsWords.appearance) {
            GlassSegmentedTabs(
                SettingsWords.appearance,
                selection: Binding(
                    get: { appearance },
                    set: { choice in
                        appearance = choice
                        choice.choose()
                    }
                ),
                segments: [
                    GlassSegment(SettingsWords.light, value: GlassAppearance.light),
                    GlassSegment(SettingsWords.dark, value: GlassAppearance.dark),
                    GlassSegment(SettingsWords.system, value: GlassAppearance.system),
                ]
            )
            .fixedSize()
        }
    }
}
