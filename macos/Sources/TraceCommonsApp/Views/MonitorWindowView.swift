#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The glass monitor window (R5 of #1173): the three-pane shell the native
/// screens are built into. The leading pane holds the tabs, the center the
/// map, the trailing pane the inspector; the leading pane and the inspector
/// hide independently, and the tab and both columns are restored per window.
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
    @SceneStorage("monitor.showsLeading") private var showsLeading = true
    @SceneStorage("monitor.showsInspector") private var showsInspector = true
    /// The selected session's entry id; empty for none.
    @SceneStorage("monitor.selectedSession") private var selectedSession = ""

    /// The screens' data (C1). Sample data in this debug window until K1
    /// moves the screens to the live client: `TRACE_COMMONS_SAMPLE` names
    /// the set (`normalDay` by default).
    @State private var traces = TracesStore(client: MonitorWindowView.dataClient())

    static func dataClient() -> any DaemonDataClient {
        let name = ProcessInfo.processInfo.environment["TRACE_COMMONS_SAMPLE"] ?? ""
        return DaemonDataWiring.sample(SampleDaemonClient.SampleSet(rawValue: name) ?? .normalDay)
    }

    var body: some View {
        GlassThreePane(showsLeading: showsLeading, showsTrailing: showsInspector) {
            MonitorLeadingPane(tab: $tab, inferenceDot: inferenceDot, onSettings: { openSettings() }) {
                switch tab {
                case .traces: TracesTreeView(store: traces, selection: $selectedSession)
                case .home, .inference: Spacer(minLength: 0)
                }
            }
        } center: {
            MonitorMapPane(
                mapTab: $mapTab,
                privateAILabel: model.privateInferenceCopy?.destination,
                showsLeading: $showsLeading,
                showsInspector: $showsInspector)
        } trailing: {
            GlassPane {
                if tab == .traces {
                    SessionInspectorView(client: traces.client, entry: selectedEntry)
                }
            }
        }
        .frame(minWidth: GlassTokens.Size.mapWidth, minHeight: GlassTokens.Size.windowHeight * 0.7)
        .glassWindow()
        .task { traces.start() }
    }

    /// The selected session, while it is still in the tree.
    private var selectedEntry: DaemonData.QueueEntry? {
        traces.tree.allSessions.first { $0.entryId == selectedSession }
    }

    /// Inference's dot: Private AI on or off, and none while the daemon has
    /// not said. Unknown is never drawn as off.
    private var inferenceDot: GlassStatus? {
        guard let settings = model.daemonSettings else { return nil }
        return settings.privateInferenceOn ? .on : .off
    }
}

/// The leading pane: the window's controls and Settings on the top row,
/// then the tabs. The tabs' screens are R6 (Traces), R8 (Inference) and R9
/// (Home).
private struct MonitorLeadingPane<Content: View>: View {
    @Binding var tab: MonitorWindowView.Tab
    let inferenceDot: GlassStatus?
    let onSettings: () -> Void
    @ViewBuilder let content: () -> Content
    @Environment(\.glassWindowControlsInset) private var controlsInset

    var body: some View {
        GlassPane {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                HStack {
                    Spacer()
                    GlassRoundButton("Settings", systemImage: "gearshape", small: true, action: onSettings)
                }
                .frame(height: max(controlsInset - GlassTokens.Space.panePadding / 2, GlassTokens.Size.control))
                GlassSegmentedTabs(
                    "Monitor",
                    selection: $tab,
                    segments: MonitorWindowView.Tab.allCases.map { item in
                        GlassSegment(item.rawValue, value: item, dot: item == .inference ? inferenceDot : nil)
                    })
                content()
                    .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
            }
        }
    }
}

/// The center: the map field, with its tabs and the column toggles floating
/// on it. The map itself is R8.
private struct MonitorMapPane: View {
    @Binding var mapTab: MonitorWindowView.MapTab
    let privateAILabel: String?
    @Binding var showsLeading: Bool
    @Binding var showsInspector: Bool
    @Environment(\.glassWindowControlsInset) private var controlsInset

    var body: some View {
        GlassPane(padding: 0) {
            ZStack(alignment: .top) {
                RadialGradient(
                    colors: [GlassTokens.Color.mapFieldInner.color, GlassTokens.Color.mapFieldOuter.color],
                    center: .center, startRadius: 20, endRadius: 520)
                GlassFloatingGroup {
                    HStack(spacing: GlassTokens.Space.s4) {
                        GlassSegmentedTabs("Map", selection: $mapTab, segments: segments, floating: true)
                        Spacer()
                        GlassToolbarGroup {
                            GlassToolbarButton("Sidebar", systemImage: "sidebar.left", pressed: showsLeading) {
                                showsLeading.toggle()
                            }
                            GlassToolbarButton("Inspector", systemImage: "sidebar.right", pressed: showsInspector) {
                                showsInspector.toggle()
                            }
                        }
                    }
                    .padding(.leading, controlsInset > 0 ? GlassTokens.Space.windowControlsWidth : GlassTokens.Space.panePadding)
                    .padding([.top, .trailing], GlassTokens.Space.panePadding)
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
