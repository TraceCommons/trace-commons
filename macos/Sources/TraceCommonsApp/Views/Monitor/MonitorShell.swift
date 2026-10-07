import SwiftUI
import TCDesign
import TCShellCore

/// What the monitor shell adds around the tabs, after #1146's
/// `monitor-shell.tsx` and `monitor-toolbar.tsx`: the View menu, the Traces
/// graph footer and the rule that opens the inspector on demand.
///
/// Every label here is a formatted number or date, or a word from the Rust
/// core's Monitor screens table (`ShellWordingTests`; no view under
/// Views/Monitor authors a label).
enum MonitorShellWords {
    static var view: String { MonitorWords.table?.view ?? "" }
    static var graph: String { MonitorWords.table?.graph ?? "" }
    static var showIgnoredFolders: String { MonitorWords.table?.showIgnoredFolders ?? "" }
    static var focus: String { MonitorWords.table?.focus ?? "" }
    static var previous: String { MonitorWords.table?.previous ?? "" }
    static var next: String { MonitorWords.table?.next ?? "" }
    /// Ron's #1146 toolbar toggles, each naming what pressing it does.
    static func graphToggle(shown: Bool) -> String { toggle(shown, \.hideGraph, \.showGraph) }
    static func mapToggle(shown: Bool) -> String { toggle(shown, \.hideMap, \.showMap) }
    static func inspectorToggle(shown: Bool) -> String { toggle(shown, \.hideInspector, \.showInspector) }
    /// The focus button's tip (#1146 `GraphFooter`).
    static func focusTip(tool: String?, focused: Bool) -> String {
        MonitorWords.table?.shell.focusTip(tool: tool, focused: focused) ?? focus
    }

    private static func toggle(_ shown: Bool, _ hide: KeyPath<MonitorShellCopy, String>,
                               _ show: KeyPath<MonitorShellCopy, String>) -> String {
        MonitorWords.table.map { $0.shell[keyPath: shown ? hide : show] } ?? ""
    }
}

// MARK: Inspector demand

/// The inspector starts closed in a narrow window, but the things a person
/// must not miss open it when they appear (#1146 `useInspectorDemand`): an
/// undo window, a folder's Submit all, a review just opened, and the core's
/// offers. Offers and undo still render above the tree (#1152); the
/// inspector opening beside them is in addition, never instead.
enum InspectorDemand {
    /// One key per thing asking for the inspector right now.
    static func keys(
        approvalUndo: [String]?, contributed: String?, kept: String?, contributedFolder: String?,
        submittingFolder: String?, privateAIOffer: Bool, armingOffer: String?
    ) -> Set<String> {
        var keys: Set<String> = []
        if let approvalUndo { keys.insert("undo:" + approvalUndo.joined(separator: ",")) }
        if let contributed { keys.insert("contributed:" + contributed) }
        if let kept { keys.insert("kept:" + kept) }
        if let contributedFolder { keys.insert("folder-undo:" + contributedFolder) }
        if let submittingFolder { keys.insert("folder-submit:" + submittingFolder) }
        if privateAIOffer { keys.insert("offer:private-ai") }
        if let armingOffer { keys.insert("offer:arming:" + armingOffer) }
        return keys
    }

    /// The inspector opens when something new asks for it; something going
    /// away, or the same thing still asking, leaves the person's choice.
    static func opens(previous: Set<String>, current: Set<String>) -> Bool {
        !current.subtracting(previous).isEmpty
    }
}

// MARK: Traces graph

/// Shared over kept, per day (#1146 `TracesGraph`): shared counts sessions
/// that left this machine (History, by submission time), kept counts
/// sessions still here (the queue, by discovery time). Counts, not bytes.
/// Pure, so the bucketing is tested.
enum TracesGraphModel {
    /// The window, in days: #1146's default range.
    static let days = 11

    struct Bucket: Equatable, Identifiable {
        let start: Date
        let shared: Int
        let kept: Int
        var id: Date { start }
    }

    /// History statuses that left this machine and still stand (#1146
    /// `CONTRIBUTED_STATUSES`): withdrawn is not shared any more.
    static let contributedStatuses: Set<String> = ["accepted", "submitted", "quarantined"]

    /// When each contribution left, for one tool or all of them.
    static func sharedTimes(_ rows: [DaemonData.HistoryRow], tool: SourceKind?) -> [Date] {
        rows.filter { contributedStatuses.contains($0.status ?? "") }
            .filter { tool == nil || $0.source.flatMap(SourceKind.init(rawValue:)) == tool }
            .compactMap(\.submittedAt)
    }

    /// When each session still waiting was found, for one tool or all.
    static func keptTimes(_ sessions: [DaemonData.QueueEntry], tool: SourceKind?) -> [Date] {
        sessions.filter { tool == nil || TracesTree.majorityTool([$0]) == tool }.compactMap(\.discoveredAt)
    }

    /// `days` day buckets ending today, `offset` whole windows back (0 is
    /// now, -1 the window before it).
    static func buckets(shared: [Date], kept: [Date], offset: Int, now: Date, calendar: Calendar = .current) -> [Bucket] {
        let today = calendar.startOfDay(for: now)
        return (0..<days).compactMap { index -> Bucket? in
            let back = days - 1 - index - offset * days
            guard let start = calendar.date(byAdding: .day, value: -back, to: today),
                  let end = calendar.date(byAdding: .day, value: 1, to: start) else { return nil }
            let inside = { (date: Date) in date >= start && date < end }
            return Bucket(start: start, shared: shared.filter(inside).count, kept: kept.filter(inside).count)
        }
    }
}

/// The graph under the Traces tree, shown while the toolbar's Graph is on:
/// the legend, the bars, and the period controls with the binoculars that
/// focus the map on the selected session's tool.
struct TracesGraphFooter: View {
    let history: [DaemonData.HistoryRow]?
    let sessions: [DaemonData.QueueEntry]
    /// The selected session's tool; the graph counts only it, and the
    /// binoculars can focus the map on it.
    let tool: SourceKind?
    @Binding var focus: Bool
    let onFocus: () -> Void

    @State private var offset = 0
    @State private var hovered: String?

    var body: some View {
        let buckets = TracesGraphModel.buckets(
            shared: TracesGraphModel.sharedTimes(history ?? [], tool: tool),
            kept: TracesGraphModel.keptTimes(sessions, tool: tool),
            offset: offset, now: .now)
        let shown = buckets.first { $0.start.formatted(.iso8601) == hovered }
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MenuWords.shared, value: Self.figure(shown?.shared ?? buckets.map(\.shared).reduce(0, +),
                                                                        known: history != nil), status: .shared)
                GlassLegendCell(MenuWords.kept, value: String(shown?.kept ?? buckets.map(\.kept).reduce(0, +)), status: .kept)
            }
            GlassBarGraph(buckets.map(Self.bar), scaleFloor: 5, hovered: $hovered)
            HStack(spacing: GlassTokens.Space.s3) {
                Button { offset -= 1 } label: { Image(systemName: "chevron.left") }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .accessibilityLabel(MonitorShellWords.previous)
                    .help(MonitorShellWords.previous)
                Button(action: onFocus) { Image(systemName: "binoculars") }
                    .buttonStyle(GlassButtonStyle(.glass, small: true, selected: focus && tool != nil))
                    .disabled(tool == nil)
                    .accessibilityLabel(MonitorShellWords.focus)
                    .accessibilityAddTraits(focus && tool != nil ? .isSelected : [])
                    .help(MonitorShellWords.focusTip(tool: tool?.displayName, focused: focus && tool != nil))
                Text(Self.range(buckets))
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textSecondary)
                    .lineLimit(1)
                    .frame(maxWidth: .infinity)
                    .accessibilityAddTraits(.updatesFrequently)
                Button { offset = min(0, offset + 1) } label: { Image(systemName: "chevron.right") }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .disabled(offset == 0)
                    .accessibilityLabel(MonitorShellWords.next)
                    .help(MonitorShellWords.next)
            }
        }
        .padding(.top, GlassTokens.Space.s4)
        .overlay(alignment: .top) { Rectangle().fill(GlassColor.hairline).frame(height: 0.5) }
    }

    /// A count, or a dash while History has not been read: an unread record
    /// is never drawn as nothing shared.
    static func figure(_ count: Int, known: Bool) -> String {
        known ? String(count) : "—"
    }

    static func bar(_ bucket: TracesGraphModel.Bucket) -> GlassBarBucket {
        GlassBarBucket(
            id: bucket.start.formatted(.iso8601),
            label: bucket.start.formatted(.dateTime.weekday(.narrow)),
            up: Double(bucket.shared), down: Double(bucket.kept),
            description: "\(bucket.start.formatted(.dateTime.month(.abbreviated).day())): "
                + "\(MenuWords.shared) \(bucket.shared), \(MenuWords.kept) \(bucket.kept)")
    }

    /// The window's first and last day.
    static func range(_ buckets: [TracesGraphModel.Bucket]) -> String {
        guard let first = buckets.first?.start, let last = buckets.last?.start else { return "—" }
        let style = Date.FormatStyle.dateTime.month(.abbreviated).day()
        return "\(first.formatted(style)) – \(last.formatted(style))"
    }
}
