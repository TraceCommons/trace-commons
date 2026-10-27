import SwiftUI
import TCDesign
import TCShellCore

/// General: the appearance, Light, Dark or the system's (design
/// exploration). Under the flat glass theme Light is its light tint and Dark
/// its dark tint; under classic they are the classic light and dark. The
/// choice applies at once and is kept for the next launch. Then the edge
/// rail's switch, and the Traces map's style.
struct GeneralSection: View {
    @State private var appearance = GlassAppearance.stored
    /// The edge rail's switch: a shell preference, read once and written
    /// through `EdgeRailController`, which shows or removes the rail at once.
    @State private var edgeRailOn = EdgeRailPreference.isEnabled()
    /// The Traces map's rocket and station (`TracesMapStyle`); the map
    /// reads the same key, so it changes at once.
    @AppStorage(TracesMapStyle.key) private var mapRocket = true
    /// Home's promotion card (owner, 2026-10-27): off, it never shows.
    /// Home reads the same key, so it changes at once.
    @AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true

    var body: some View {
        appearanceCard
        edgeRailCard
        mapStyleCard
        promotionsCard
    }

    /// Whether Home shows promotions, such as a new model on NEAR AI
    /// (owner, 2026-10-27). The Insights card is not one.
    @ViewBuilder
    private var promotionsCard: some View {
        if let words = MonitorWords.table?.homeHistory {
            GlassEyebrowCard(words.promotionsTitle) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Toggle(words.promotionsToggle, isOn: $promotionsOn)
                        .toggleStyle(GlassToggleStyle(.settings))
                    Text(words.promotionsCaption)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// The Traces map's style (owner, 2026-10-19): on, the rocket and space
    /// station; off, the laptop and the library orb.
    @ViewBuilder
    private var mapStyleCard: some View {
        if let words = MonitorWords.table?.flowMap {
            GlassEyebrowCard(words.styleTitle) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Toggle(words.styleToggle, isOn: $mapRocket)
                        .toggleStyle(GlassToggleStyle(.settings))
                    Text(words.styleCaption)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// The edge rail and its discovery tutorial.
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
