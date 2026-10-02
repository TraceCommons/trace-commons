#if DEBUG
import SwiftUI
import TCDesign

/// The glass monitor window (R5 of #1173): the three-pane shell the native
/// screens are built into. The main pane holds the tabs and is always shown;
/// the map and the inspector hide independently (and the map below 1100pt),
/// and the tab and both preferences are restored per window.
///
/// Debug builds only, until the screens it frames (R6 onward) match the
/// design. The shipping window stays `MainWindowView` until then (R15).
/// Every label here is a single word or comes from the Rust core
/// (`ShellWordingTests`).
struct MonitorWindowView: View {
    enum Tab: String, CaseIterable, Identifiable {
        case home = "Home"
        case inference = "Inference"
        case traces = "Traces"

        var id: String { rawValue }
    }

    enum MapTab: String {
        case traces
        case privateAI
    }

    @EnvironmentObject private var model: AppModel
    @Environment(\.openSettings) private var openSettings

    @SceneStorage("monitor.tab") private var tab: Tab = .home
    @SceneStorage("monitor.mapTab") private var mapTab: MapTab = .traces
    /// The person's preferences. The map is also hidden below 1100pt
    /// without touching this, so widening the window brings it back.
    @SceneStorage("monitor.showsMap") private var showsMap = true
    @SceneStorage("monitor.showsInspector") private var showsInspector = true

    var body: some View {
        GlassThreePane(showsMap: showsMap, showsInspector: showsInspector) {
            MonitorMainPane(
                tab: $tab, inferenceDot: Self.inferenceDot(settings: model.daemonSettings),
                showsMap: $showsMap, showsInspector: $showsInspector,
                onSettings: { openSettings() })
        } map: {
            MonitorMapPane(mapTab: $mapTab, privateAILabel: model.privateInferenceCopy?.destination)
        } inspector: {
            GlassPane {
                Color.clear
            }
        }
        .glassWindow()
    }

    /// Inference's dot: Private AI on or off, and none while the daemon has
    /// not said. Unknown is never drawn as off.
    static func inferenceDot(settings: DaemonSettingsView?) -> GlassStatus? {
        guard let on = settings?.privateInference else { return nil }
        return on ? .on : .off
    }
}

/// The main pane, always shown: clearance for the traffic lights, the
/// toolbar capsule (map and inspector toggles) and the round Settings
/// button on the top row, then the tabs. The tabs' screens are R6 (Traces),
/// R8 (Inference) and R9 (Home).
private struct MonitorMainPane: View {
    @Binding var tab: MonitorWindowView.Tab
    let inferenceDot: GlassStatus?
    @Binding var showsMap: Bool
    @Binding var showsInspector: Bool
    let onSettings: () -> Void
    /// Half the unified title bar's 52pt height.
    static let lightsCentre: CGFloat = 26

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                HStack(spacing: GlassTokens.Space.s4) {
                    Spacer(minLength: 0)
                    GlassToolbarGroup {
                        GlassToolbarButton("Map", systemImage: "map", pressed: showsMap) {
                            showsMap.toggle()
                        }
                        GlassToolbarButton("Inspector", systemImage: "sidebar.right", pressed: showsInspector) {
                            showsInspector.toggle()
                        }
                    }
                    GlassRoundButton("Settings", systemImage: "gearshape", small: true, action: onSettings)
                }
                // Clearance for the real traffic lights, not an origin.
                .padding(.leading, GlassTokens.Space.windowControlsWidth - GlassTokens.Space.panePadding)
                .frame(height: GlassTokens.Size.controlLarge)
                // Centre the row on the traffic lights, which the unified
                // title bar centres 26pt below the window's top edge.
                .padding(.top, Self.lightsCentre - GlassTokens.Space.windowPadding - GlassTokens.Space.panePadding
                    - GlassTokens.Size.controlLarge / 2)
                GlassSegmentedTabs(
                    "Monitor",
                    selection: $tab,
                    segments: MonitorWindowView.Tab.allCases.map { item in
                        GlassSegment(item.rawValue, value: item, dot: item == .inference ? inferenceDot : nil)
                    })
                Spacer(minLength: 0)
            }
        }
    }
}

/// The map: the field, with its tabs floating on it. The map itself is R8.
private struct MonitorMapPane: View {
    @Binding var mapTab: MonitorWindowView.MapTab
    let privateAILabel: String?

    var body: some View {
        GlassPane(padding: 0) {
            ZStack(alignment: .topLeading) {
                RadialGradient(
                    colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                    center: .center, startRadius: 20, endRadius: 520)
                GlassFloatingGroup {
                    GlassSegmentedTabs("Map", selection: $mapTab, segments: segments, floating: true)
                        .padding(GlassTokens.Space.panePadding)
                }
            }
        }
    }

    /// Traces, and the Private AI tab once the Rust core has said what it is
    /// called (D11).
    private var segments: [GlassSegment<MonitorWindowView.MapTab>] {
        var segments = [GlassSegment(MonitorWindowView.Tab.traces.rawValue, value: MonitorWindowView.MapTab.traces)]
        if let privateAILabel {
            segments.append(GlassSegment(privateAILabel, value: .privateAI))
        }
        return segments
    }
}

/// The Settings window (D8): the existing settings, in a macOS Settings
/// window opened with ⌘,. Its theming may later move to the glass system to
/// match the monitor window (#1173).
struct MonitorSettingsWindow: View {
    let navigation: MainWindowNavigation

    var body: some View {
        SettingsView(navigation: navigation)
            .frame(minWidth: 620, minHeight: 520)
    }
}
#endif
