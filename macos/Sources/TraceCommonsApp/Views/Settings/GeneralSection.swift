import SwiftUI
import TCDesign
import TCShellCore

/// General: the appearance, Light, Dark or the system's (design
/// exploration). Under the flat glass theme Light is its light tint and Dark
/// its dark tint; under classic they are the classic light and dark. The
/// choice applies at once and is kept for the next launch. Then the edge
/// rail's switch.
struct GeneralSection: View {
    @State private var appearance = GlassAppearance.stored
    /// The edge rail's switch: a shell preference, read once and written
    /// through `EdgeRailController`, which shows or removes the rail at once.
    @State private var edgeRailOn = EdgeRailPreference.isEnabled()

    var body: some View {
        appearanceCard
        edgeRailCard
    }

    /// The edge rail, under the Desktop eyebrow the Startup section's system
    /// integrations card uses.
    @ViewBuilder
    private var edgeRailCard: some View {
        if let rail = MonitorWords.table?.edgeRail {
            GlassEyebrowCard(SettingsLegacyWords.desktopEyebrow, title: rail.railLabel) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Toggle(rail.settingToggle, isOn: Binding(
                        get: { edgeRailOn },
                        set: { on in
                            edgeRailOn = on
                            EdgeRailController.shared.setEnabled(on)
                        }))
                        .toggleStyle(GlassToggleStyle(.settings))
                    Text(rail.settingCaption)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    private var appearanceCard: some View {
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
