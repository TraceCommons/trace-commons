#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// Missions (R10 of #1173): the commons' mission catalogue, reached from
/// Home (spec, "Screens"). Under #1174's rules:
///
/// - M1: one catalogue for the whole commons. Matching to this Mac's
///   activity is local and not built yet (K16), so nothing here claims a
///   mission was matched or chosen for this person.
/// - M2: a mission sends nothing by itself. The page has no action that
///   arms a folder, approves a session or widens a scope: it has no
///   actions at all.
/// - M3: mission credit is projected credit (owner ruling, 2026-10-02):
///   labelled Projected with the core's note that it is not yet earned,
///   never Pending, which is submitted credit still being scored. A
///   contribution mission is apart from the reward ledger (#1174). It is
///   shown only beside the commons' statement of what it waits on.
///
/// PROVISIONAL: `mission_catalogue`'s shape follows #1174's M1-M4, which
/// is still under review, and the live client answers `notAvailableYet`,
/// so only sample data reaches this page.
/// - M4: the disclosure is the core's copy. It does not exist yet, so none
///   is shown, and this page stays in the debug window until it does.
struct MissionsPage: View {
    let store: HomeStore
    let back: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            GlassBreadcrumb(
                [GlassCrumb(MonitorWindowView.Tab.home.title, action: back), GlassCrumb(MonitorWords.missions)],
                backLabel: MonitorWindowView.Tab.home.title, onBack: back)
            ScrollView {
                VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                    if let failure = store.failures["mission_catalogue"] {
                        GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                    }
                    if let catalogue = store.missions {
                        if let condition = MissionFormat.condition(catalogue) {
                            Text(condition)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if MissionFormat.showsProjected(catalogue) {
                            Text(MonitorWords.projectedNote)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        if catalogue.missions.isEmpty {
                            Text("0")
                                .glassType(GlassTokens.TypeScale.number)
                                .foregroundStyle(GlassColor.textTertiary)
                                .accessibilityLabel(FlowMapScene.pair(MonitorWords.missions, 0))
                        } else {
                            ForEach(catalogue.missions) { mission in
                                MissionCard(mission: mission, credit: MissionFormat.credit(mission, in: catalogue))
                            }
                        }
                    } else if store.failures["mission_catalogue"] == nil {
                        // Not loaded, or a daemon with no catalogue yet: a
                        // dash, never an empty list that reads as "none".
                        Text("—")
                            .glassType(GlassTokens.TypeScale.number)
                            .foregroundStyle(GlassColor.textTertiary)
                            .accessibilityLabel(MonitorWords.unknown)
                    }
                }
            }
            .scrollIndicators(.never)
        }
    }
}

/// One mission: the commons' title and summary, and its credit range as
/// projected credit (or a dash when its condition is unknown).
private struct MissionCard: View {
    let mission: DaemonData.Mission
    let credit: String

    var body: some View {
        GlassCard {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(mission.title)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                    if let summary = mission.summary {
                        Text(summary)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                Spacer(minLength: GlassTokens.Space.s4)
                VStack(alignment: .trailing, spacing: 1) {
                    Text(MonitorWords.projected)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    Text(credit)
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                        .foregroundStyle(GlassColor.textSecondary)
                }
                .fixedSize()
            }
        }
        .accessibilityElement(children: .combine)
    }
}

/// Formatting for Missions. Pure, so #1174's rules are tested.
enum MissionFormat {
    /// What mission credit waits on, in the commons' own words (M3).
    static func condition(_ catalogue: DaemonData.MissionCatalogue) -> String? {
        guard let explanation = catalogue.posture?.explanation, !explanation.isEmpty else { return nil }
        return explanation
    }

    /// Whether any mission shows a projected figure, and so the page needs
    /// the core's note that projected credit is not yet earned.
    static func showsProjected(_ catalogue: DaemonData.MissionCatalogue) -> Bool {
        catalogue.missions.contains { credit($0, in: catalogue) != "—" }
    }

    /// A mission's credit range, projected. A dash when the mission has no
    /// range, or when the commons has not said what the credit waits on: a
    /// bare range would read as owed (M3).
    static func credit(_ mission: DaemonData.Mission, in catalogue: DaemonData.MissionCatalogue) -> String {
        guard condition(catalogue) != nil, let range = mission.creditRange else { return "—" }
        let span = range.min == range.max ? "\(range.min)" : "\(range.min)–\(range.max)"
        return "\(span) \(range.unit)"
    }

    /// Home's count: the catalogue's size, or a dash when it was not read.
    static func count(_ catalogue: DaemonData.MissionCatalogue?) -> String {
        HomeFormat.count(catalogue?.missions.count)
    }
}

/// Missions' word, from the core's table (`MonitorWords.table`).
extension MonitorWords {
    static var missions: String { table?.missions ?? "" }
    static var projected: String { table?.projected ?? "" }
    static var projectedNote: String { table?.projectedNote ?? "" }
}
#endif
