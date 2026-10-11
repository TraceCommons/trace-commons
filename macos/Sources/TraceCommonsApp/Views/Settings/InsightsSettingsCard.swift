import Observation
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The "Higher-than-usual card" switch (`analytics_setting_lever`): the
/// core's `lever_card` operation, not a daemon setting, because the lever's
/// "Not useful" answers live in the core's goals file. Read from
/// `comparisons` with no weekly figures, whose `lever.card_enabled` answers
/// whatever the week; moved only by a `lever_card` reply whose
/// `lever_card_off` is the opposite of what was asked.
@Observable @MainActor
final class InsightsLeverCardSwitch {
    private let service: InsightsModel.Service
    /// `nil` until the core answered, and from a core that does not send
    /// the switch: no switch is drawn, never an "off".
    private(set) var enabled: Bool?
    private(set) var busy = false
    /// The core's request-failed words, until the next confirmed write.
    private(set) var refusal: String?

    init(service: @escaping InsightsModel.Service) { self.service = service }

    /// Reads the switch. A failed read leaves no value, so no switch.
    func load() async {
        let request = InsightsRequest(operation: .init("comparisons", tz: InsightsOverviewModel.offset))
        enabled = (try? await service(request))?.comparisons?.lever.card_enabled
    }

    func set(_ on: Bool) async {
        guard !busy else { return }
        busy = true
        defer { busy = false }
        let reply = try? await service(InsightsRequest(operation: .init("lever_card", enabled: on)))
        if reply?.type == "lever_card", let off = reply?.state?.lever_card_off, off == !on {
            enabled = on
            refusal = nil
        } else {
            await load()
            refusal = MonitorWords.table?.requestFailed
        }
    }
}

/// The Insights switches in Settings (Ron's #1287 review, adopted by the
/// owner): count tokens in the watched folders (off by default, D4), the
/// weekly summary card and the higher-than-usual card. Drawn under Tools
/// (`GlassSettingsContent`), which waits for first run (R-43). Each switch reads the value its owner
/// confirmed and is drawn only once that owner reported one; every word is
/// the core's analytics copy.
struct InsightsSettingsCard: View {
    @EnvironmentObject private var model: AppModel
    /// Routed to the same store as the Insights window
    /// (`--insights-store`).
    @State private var lever = InsightsLeverCardSwitch(service: { request in
        try await InsightsServiceRouter(selection: .parse(arguments: CommandLine.arguments)).call(request)
    })

    private static let copy = TCInsights.copy() ?? [:]
    private func text(_ key: String) -> String { InsightsOverviewWords.text(key, Self.copy) }

    var body: some View {
        VStack(alignment: .leading, spacing: 0) {
            // No headed card without a switch in it.
            if model.daemonSettings?.insightsCounterPass != nil
                || model.daemonSettings?.insightsRecapCardEnabled != nil
                || lever.enabled != nil {
                GlassEyebrowCard(text("title")) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        if let settings = model.daemonSettings {
                            if let on = settings.insightsCounterPass {
                                Toggle(text("analytics_setting_counter_pass"), isOn: Binding(
                                    get: { on },
                                    set: { on in Task { await model.setInsightsSwitch(.counterPass, on) } }
                                ))
                                .toggleStyle(GlassToggleStyle(.settings))
                                .disabled(model.insightsSwitchBusy.contains(.counterPass))
                                refusal(model.insightsSwitchRefusal[.counterPass])
                            }
                            if let on = settings.insightsRecapCardEnabled {
                                Toggle(text("analytics_setting_recap_card"), isOn: Binding(
                                    get: { on },
                                    set: { on in Task { await model.setInsightsSwitch(.recapCard, on) } }
                                ))
                                .toggleStyle(GlassToggleStyle(.settings))
                                .disabled(model.insightsSwitchBusy.contains(.recapCard))
                                refusal(model.insightsSwitchRefusal[.recapCard])
                            }
                        }
                        if let on = lever.enabled {
                            Toggle(text("analytics_setting_lever"), isOn: Binding(
                                get: { on },
                                set: { on in Task { await lever.set(on) } }
                            ))
                            .toggleStyle(GlassToggleStyle(.settings))
                            .disabled(lever.busy)
                            refusal(lever.refusal)
                        }
                    }
                }
            }
        }
        // "Hide" on the summary card writes the daemon directly, and "Not
        // useful" the core, so both are read again whenever this appears.
        .task {
            model.refreshSettings()
            await lever.load()
        }
    }

    @ViewBuilder
    private func refusal(_ words: String?) -> some View {
        if let words { GlassNotice(tone: .outside) { Text(words) } }
    }
}
