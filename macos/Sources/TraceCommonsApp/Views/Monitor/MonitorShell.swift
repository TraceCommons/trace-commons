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
    /// The graph footer's zoom, jump, range and bar words (#1146
    /// `TracesGraph`).
    static var graphWords: MonitorTracesGraphCopy? { MonitorWords.table?.tracesGraph }
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

/// Shared over kept, per period (#1146 `TracesGraph`): shared counts
/// sessions that left this machine (History, by submission time), kept
/// counts sessions still here (the queue, by discovery time). Counts, not
/// bytes. Pure, so the bucketing is tested.
enum TracesGraphModel {
    /// The window, in days, at #1146's default range.
    static let days = 11

    /// #1146's three ranges (`GraphRange`): 24 hours in 2-hour buckets, 11
    /// days, and 42 days. Zooming out widens the range.
    enum Range: Int, CaseIterable, Comparable {
        case hours = 0
        case days = 1
        case weeks = 2

        /// How many buckets, and how wide each is.
        var count: Int {
            switch self {
            case .hours: 12
            case .days: TracesGraphModel.days
            case .weeks: 42
            }
        }

        /// A bucket's width: 2 hours, or a day.
        var step: (component: Calendar.Component, value: Int) {
            self == .hours ? (.hour, 2) : (.day, 1)
        }

        /// The window's span as the range pill says it: 24 (hours), 11 or
        /// 42 (days).
        var span: Int { self == .hours ? count * 2 : count }

        static func < (a: Range, b: Range) -> Bool { a.rawValue < b.rawValue }

        /// One step wider, or nil at the widest.
        var zoomedOut: Range? { Range(rawValue: rawValue + 1) }
        /// One step narrower, or nil at the narrowest.
        var zoomedIn: Range? { Range(rawValue: rawValue - 1) }
    }

    struct Bucket: Equatable, Identifiable {
        let start: Date
        let shared: Int
        let kept: Int
        /// The label under its bar; empty where #1146 leaves one out.
        var label: String = ""
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

    /// The range's buckets ending in the current one (this hour's 2-hour
    /// bucket, or today), `offset` whole windows back (0 is now, -1 the
    /// window before it). #1146 `buildGraphBuckets`.
    static func buckets(
        shared: [Date], kept: [Date], offset: Int, now: Date, range: Range = .days,
        calendar: Calendar = .current
    ) -> [Bucket] {
        let anchor: Date
        if range == .hours {
            anchor = calendar.dateInterval(of: .hour, for: now)?.start ?? now
        } else {
            anchor = calendar.startOfDay(for: now)
        }
        let (component, value) = range.step
        return (0..<range.count).compactMap { index -> Bucket? in
            let back = range.count - 1 - index - offset * range.count
            guard let start = calendar.date(byAdding: component, value: -back * value, to: anchor),
                  let end = calendar.date(byAdding: component, value: value, to: start) else { return nil }
            let inside = { (date: Date) in date >= start && date < end }
            return Bucket(
                start: start, shared: shared.filter(inside).count, kept: kept.filter(inside).count,
                label: label(range, start: start, index: index, calendar: calendar))
        }
    }

    /// #1146 `bucketLabel`: every third 2-hour bucket's hour; every day's
    /// short weekday at 11 days; every sixth day's at 42.
    static func label(_ range: Range, start: Date, index: Int, calendar: Calendar = .current) -> String {
        switch range {
        case .hours:
            return index % 3 == 0 ? "\(calendar.component(.hour, from: start)):00" : ""
        case .days:
            return weekday(start, calendar: calendar)
        case .weeks:
            return index % 6 == 0 ? weekday(start, calendar: calendar) : ""
        }
    }

    /// "Sat": the short weekday.
    static func weekday(_ date: Date, calendar: Calendar = .current) -> String {
        var style = Date.FormatStyle.dateTime.weekday(.abbreviated)
        style.timeZone = calendar.timeZone
        return date.formatted(style)
    }

    /// "Monday, October 5": a hovered bar's day (#1146 `formatDay`).
    static func day(_ date: Date, calendar: Calendar = .current) -> String {
        var style = Date.FormatStyle.dateTime.weekday(.wide).month(.wide).day()
        style.timeZone = calendar.timeZone
        return date.formatted(style)
    }
}

/// The graph under the Traces tree, shown while the toolbar's Graph is on
/// (#1146 `GraphFooter` and `TracesGraph`): the legend, the bars, and six
/// pills around the range: previous, the binoculars that focus the map on
/// the selected session's tool, zoom out, the range, zoom in, next, and
/// jump to now.
struct TracesGraphFooter: View {
    let history: [DaemonData.HistoryRow]?
    let sessions: [DaemonData.QueueEntry]
    /// The selected session's tool; the graph counts only it, and the
    /// binoculars can focus the map on it.
    let tool: SourceKind?
    @Binding var focus: Bool
    let onFocus: () -> Void

    /// #1146's fixed footer height, rule included.
    static let height: CGFloat = 206

    @State private var range: TracesGraphModel.Range = .days
    @State private var offset = 0
    @State private var hovered: String?
    /// Which way the bars slide in on a period step (#1146 `tc-slide-l/r`);
    /// nil for a zoom, which swaps them in place.
    @State private var slide: Edge?
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        let buckets = TracesGraphModel.buckets(
            shared: TracesGraphModel.sharedTimes(history ?? [], tool: tool),
            kept: TracesGraphModel.keptTimes(sessions, tool: tool),
            offset: offset, now: .now, range: range)
        let shown = buckets.first { $0.start.formatted(.iso8601) == hovered }
        let words = MonitorShellWords.graphWords
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MenuWords.shared, value: Self.figure(shown?.shared ?? buckets.map(\.shared).reduce(0, +),
                                                                        known: history != nil), status: .shared)
                GlassLegendCell(MenuWords.kept, value: String(shown?.kept ?? buckets.map(\.kept).reduce(0, +)), status: .kept)
            }
            GlassBarGraph(buckets.map { Self.bar($0, words: words) }, scaleFloor: 5, hovered: $hovered)
                .id("\(range.rawValue):\(offset)")
                .transition(slide.map { .move(edge: $0).combined(with: .opacity) } ?? .opacity)
                .frame(maxWidth: .infinity)
                .clipped()
            HStack(spacing: 6) {
                GlassPillIconButton(MonitorShellWords.previous, glyph: "\u{2039}") { step(-1, from: .trailing) }
                GlassPillIconButton(
                    MonitorShellWords.focusTip(tool: tool?.displayName, focused: focus && tool != nil),
                    systemImage: "binoculars", ink: Self.focusInk(focused: focus, canFocus: tool != nil),
                    action: onFocus)
                    .disabled(tool == nil)
                    .accessibilityAddTraits(focus && tool != nil ? .isSelected : [])
                GlassPillIconButton(words?.zoomOut ?? "", glyph: "\u{2212}") { zoom(range.zoomedOut) }
                    .disabled(range.zoomedOut == nil)
                Text(Self.rangeLabel(range: range, offset: offset, hovered: shown?.start, words: words))
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textPrimary)
                    .lineLimit(1)
                    .truncationMode(.tail)
                    .padding(.horizontal, GlassTokens.Space.s4)
                    .frame(maxWidth: .infinity, minHeight: GlassTokens.Size.control, maxHeight: GlassTokens.Size.control)
                    .background(Capsule().fill(GlassColor.ink(0.07)))
                    .glassEdge(Self.rangeEdge, in: Capsule())
                    .accessibilityAddTraits(.updatesFrequently)
                GlassPillIconButton(words?.zoomIn ?? "", glyph: "+") { zoom(range.zoomedIn) }
                    .disabled(range.zoomedIn == nil)
                GlassPillIconButton(MonitorShellWords.next, glyph: "\u{203A}") { step(1, from: .leading) }
                    .disabled(offset == 0)
                GlassPillIconButton(words?.jumpToNow ?? "", glyph: "\u{203A}|") {
                    slide = .leading
                    withAnimation(Self.slideAnimation(reduceMotion)) { offset = 0 }
                }
                .disabled(offset == 0)
            }
        }
        // Its insets, its fixed height and its rule are the shell's
        // (`MonitorWindowView`).
    }

    /// A period step: the bars slide in from the side the new period lies on.
    private func step(_ by: Int, from edge: Edge) {
        slide = edge
        withAnimation(Self.slideAnimation(reduceMotion)) { offset = min(0, offset + by) }
    }

    /// A zoom resets to now and swaps the bars in place.
    private func zoom(_ to: TracesGraphModel.Range?) {
        guard let to else { return }
        slide = nil
        hovered = nil
        withAnimation(GlassMotion.fast(reduceMotion)) {
            range = to
            offset = 0
        }
    }

    /// #1146's slide: 0.45s on `cubic-bezier(.4,0,.2,1)`; none under Reduce
    /// Motion.
    static func slideAnimation(_ reduceMotion: Bool) -> Animation? {
        reduceMotion ? nil : .timingCurve(0.4, 0, 0.2, 1, duration: 0.45)
    }

    /// The range pill's inner edge (#1146: lit above, shaded below).
    static let rangeEdge: [GlassShadow] = [
        GlassShadow(x: 0, y: 1, blur: 0, color: GlassTokens.Color.ink.opacity(0.18), inset: true),
        GlassShadow(x: 0, y: -1, blur: 0, color: GlassTokens.Color.textOnStatus.opacity(0.15), inset: true),
    ]

    /// The binoculars' glyph (#1146): blue while the map is focused, light
    /// while a tool can be focused, dim with nothing to focus on.
    static func focusInk(focused: Bool, canFocus: Bool) -> GlassRGBA {
        guard canFocus else { return GlassTokens.Color.graphFocusOff }
        return focused ? GlassTokens.Color.graphFocusOn : GlassTokens.Color.graphFocusIdle
    }

    /// A count, or a dash while History has not been read: an unread record
    /// is never drawn as nothing shared.
    static func figure(_ count: Int, known: Bool) -> String {
        known ? String(count) : "—"
    }

    static func bar(_ bucket: TracesGraphModel.Bucket, words: MonitorTracesGraphCopy?) -> GlassBarBucket {
        let day = TracesGraphModel.day(bucket.start)
        return GlassBarBucket(
            id: bucket.start.formatted(.iso8601),
            label: bucket.label,
            up: Double(bucket.shared), down: Double(bucket.kept),
            description: words?.bar(day: day, shared: bucket.shared, kept: bucket.kept) ?? day)
    }

    /// The range pill: the hovered bar's day, else the window in the core's
    /// words ("Last 11 days", "11 days, 2 windows back").
    static func rangeLabel(
        range: TracesGraphModel.Range, offset: Int, hovered: Date?, words: MonitorTracesGraphCopy?
    ) -> String {
        if let hovered { return TracesGraphModel.day(hovered) }
        return words?.range(span: range.span, hours: range == .hours, back: -offset) ?? "—"
    }
}
