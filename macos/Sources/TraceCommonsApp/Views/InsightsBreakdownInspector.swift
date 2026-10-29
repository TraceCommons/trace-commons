import SwiftUI
import TCBridge
import TCDesign

/// Where Breakdown hands the Monitor window's inspector its details (owner,
/// 2026-10-29: "details for breakdown should appear in the right details
/// panel"): a day picked on Tokens by day shows that day's sessions there,
/// and a pattern card's "See sessions" the sessions behind the card.
/// Breakdown keeps its charts and tables and marks what is picked.
///
/// Filled while Breakdown is on screen, empty otherwise. As for a Traces
/// session, the inspector opens when the person picks something
/// (`demandKey`), never when a pick is cleared or the tab is shown again.
@Observable @MainActor
final class InsightsBreakdownInspector {
    /// Builds the details. Called in the inspector's body, so what it reads
    /// (the picked day, the card's sessions) keeps them current.
    private(set) var detail: (@MainActor () -> InsightsBreakdownDetail)?
    /// The day picked on Tokens by day, by its date; `nil` is none.
    private(set) var pickedDay: String?
    /// How many times the person has picked something on Breakdown in this
    /// window. Kept through `clear()` and `show(_:)`, so the demand is the
    /// same key whichever tab is shown and changes only on a pick.
    private(set) var opens = 0

    func show(_ detail: @escaping @MainActor () -> InsightsBreakdownDetail) { self.detail = detail }

    func clear() { detail = nil }

    /// A day picked on Tokens by day, or `nil` for the week again. A day
    /// replaces a card's sessions in the inspector; the week asks nothing.
    func pick(day: String?, patterns: InsightsPatternsModel?) {
        pickedDay = day
        guard day != nil else { return }
        patterns?.hideSessions()
        opens += 1
    }

    /// A card's "See sessions": shows the card's sessions, replacing a
    /// picked day; on the card already shown, hides them.
    func toggleSessions(_ kind: String, patterns: InsightsPatternsModel) {
        if patterns.sessions?.pattern == kind {
            patterns.hideSessions()
            return
        }
        pickedDay = nil
        patterns.showSessions(kind)
        opens += 1
    }

    /// The inspector's demand (`InspectorDemand`): a new key for each pick,
    /// and none before the first.
    var demandKey: String? { opens > 0 ? "insights-breakdown-open:\(opens)" : nil }
}

/// Breakdown's details in the inspector: a card's sessions, else the picked
/// day's sessions, else Breakdown's title over the dash.
struct InsightsBreakdownDetail: View {
    let inspector: InsightsBreakdownInspector
    let patterns: InsightsPatternsModel
    let overview: InsightsOverviewModel?
    let copy: [String: String]
    /// Saved snapshots, so a picked day's rows read as their labels.
    var snapshots: [LocalInsight] = []

    private func text(_ key: String) -> String { copy[key] ?? "" }

    /// What the inspector shows.
    enum Shown: Equatable {
        case pattern
        case day
        case empty
    }

    /// A card's sessions first, then a picked day of the week on screen
    /// with sessions to show; otherwise the empty state. Kept pure for tests.
    static func shown(patternSessions: Bool, day: InsightsDayPick?, daySessions: Bool) -> Shown {
        if patternSessions { return .pattern }
        if day != nil, daySessions { return .day }
        return .empty
    }

    /// The picked day among the week on screen; `nil` for none, or for a
    /// day of another week.
    private var day: InsightsDayPick? {
        InsightsDayPick.pick(inspector.pickedDay, days: overview?.shown?.by_day)
    }

    private var hasDaySessions: Bool {
        guard let overview else { return false }
        if let sessions = overview.counterSessions { return !sessions.rows.isEmpty }
        return !(overview.sessionsInputs?.sessions.isEmpty ?? true)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            switch Self.shown(patternSessions: patterns.sessions != nil, day: day, daySessions: hasDaySessions) {
            case .pattern:
                if let found = patterns.sessions { InsightsPatternSessionsView(found: found, copy: copy) }
            case .day:
                daySessions
            case .empty:
                // Nothing picked: Breakdown's title over the dash, as an
                // empty table on Breakdown draws it.
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    InsightsBreakdownTitle(day: nil, copy: copy, showWeek: {})
                    Text(text("analytics_unavailable")).insightsNote()
                }
            }
        }
        .environment(\.insightsInPanel, true)
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSurface()
    }

    /// The picked day's sessions: the daemon's rows under feed T, the
    /// core's under feed S. "Show week" puts the week back.
    @ViewBuilder
    private var daySessions: some View {
        if let overview, let day {
            let showWeek = { inspector.pick(day: nil, patterns: patterns) }
            if let sessions = overview.counterSessions {
                InsightsCounterSessionsView(sessions: sessions, copy: copy, day: day, showWeek: showWeek)
            } else if let inputs = overview.sessionsInputs {
                InsightsCardInputsView(inputs: inputs, snapshots: snapshots, copy: copy, day: day, showWeek: showWeek)
            }
        }
    }
}

private struct InsightsInPanelKey: EnvironmentKey {
    static let defaultValue = false
}

extension EnvironmentValues {
    /// Drawn in the Monitor window's inspector, whose pane is the card:
    /// a section there is drawn without a card of its own.
    var insightsInPanel: Bool {
        get { self[InsightsInPanelKey.self] }
        set { self[InsightsInPanelKey.self] = newValue }
    }
}

/// A section's card on a tab, and no card in the inspector
/// (`insightsInPanel`), as the other tabs' inspector content has none.
struct InsightsDetailSection: ViewModifier {
    @Environment(\.insightsInPanel) private var inPanel

    func body(content: Content) -> some View {
        if inPanel { content } else { content.insightsSection() }
    }
}

extension View {
    func insightsDetailSection() -> some View { modifier(InsightsDetailSection()) }
}
