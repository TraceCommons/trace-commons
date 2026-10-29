import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The glass parts the Insights, comparison and mission-draft screens are
/// drawn with: the type scale and inks for their text, their disclosures as
/// `GlassExpander`, their cards as `GlassCard`, and their buttons and
/// checkboxes in the glass styles. Every word stays the core's copy table's.
extension View {
    /// A screen's title.
    func insightsTitle() -> some View {
        glassType(GlassTokens.TypeScale.title).foregroundStyle(GlassColor.textPrimary)
    }

    /// A section heading.
    func insightsHeading() -> some View {
        glassType(GlassTokens.TypeScale.bodyStrong).foregroundStyle(GlassColor.textPrimary)
    }

    /// A notice beside the content it qualifies.
    func insightsNote() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassColor.textSecondary)
    }

    /// A caption.
    func insightsCaption() -> some View {
        glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
    }

    /// An identifier or a digest.
    func insightsMono() -> some View {
        glassType(GlassTokens.TypeScale.mono).foregroundStyle(GlassColor.textSecondary)
    }

    /// A failure, in the outside red's text ink.
    func insightsError() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassStatus.outside.textColor)
    }

    /// A success, in the on green's text ink.
    func insightsSuccess() -> some View {
        glassType(GlassTokens.TypeScale.label.weight(.regular)).foregroundStyle(GlassStatus.on.textColor)
    }

    /// A quiet glass card around this content. Inside a card's body it
    /// reaches out to the card's own edges (`InsightsInlineContainer`).
    func insightsCard() -> some View {
        GlassCard(quiet: true) { environment(\.insightsInCardBody, false) }
            .modifier(InsightsInlineContainer())
    }

    /// The label of a row that acts: a quiet card that lifts under the
    /// pointer. Its button takes `GlassPressStyle`. Inside a card's body it
    /// reaches out to the card's own edges (`InsightsInlineContainer`).
    func insightsRowCard() -> some View {
        GlassCard(quiet: true, interactive: true) { environment(\.insightsInCardBody, false) }
            .modifier(InsightsInlineContainer())
    }

    /// A screen's defaults: body type in the primary ink, glass buttons
    /// and glass checkboxes for every control that does not choose its own.
    func insightsSurface() -> some View {
        glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(GlassColor.textPrimary)
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .toggleStyle(GlassCheckboxStyle())
    }
}

/// The hairline between a screen's sections.
struct InsightsRule: View {
    var body: some View {
        GlassHairline(GlassColor.hairline)
    }
}

// MARK: - Direction B (2026-10-15)

/// Insights is a Home page, so it draws in the main pane: 440pt at the
/// window's default width, 408 inside the pane's insets. Every tab is one
/// column of cards there; a second column appears only when the pane is
/// wider (the map hidden in a zoomed window).
enum InsightsLayout {
    /// A column's narrowest width. Two columns need about 700pt.
    static let columnMinimum: CGFloat = 340

    static var columns: [GridItem] {
        [GridItem(.adaptive(minimum: columnMinimum), spacing: sectionGap, alignment: .top)]
    }

    /// The type and spacing scale every Insights tab keeps (owner,
    /// 2026-10-15: sections ran together, Analyze worst). A section is a
    /// card under a title at the card-title step; a group inside it opens
    /// with a heading at the body-strong step; lines in a group sit close,
    /// groups apart, sections further apart again.
    ///
    /// Between sections (cards).
    static let sectionGap: CGFloat = GlassTokens.Space.s8
    /// Between groups inside a section.
    static let groupGap: CGFloat = GlassTokens.Space.s8
    /// From a heading to what it heads.
    static let headingGap: CGFloat = GlassTokens.Space.s4
    /// Between the lines of one group.
    static let lineGap: CGFloat = GlassTokens.Space.s2
    /// How much further a card's body sits in from the card's edges than
    /// its title and its actions do (owner, 2026-10-19: the content between
    /// a card's title and its buttons ran too close to the card's sides).
    static let bodyInset: CGFloat = GlassTokens.Space.s5
}

extension View {
    /// A section: a card on the pane (cards inside it are quiet).
    func insightsSection() -> some View {
        GlassCard { self }
    }

    /// A card's body: everything between its title and its actions, inset
    /// `InsightsLayout.bodyInset` further on both sides. A container nested
    /// in it (a chart's well, a quiet card, a row card) is not inset that
    /// far: it reaches back out to the card's own edges, in line with the
    /// title (owner, 2026-10-19), and keeps its own padding inside.
    func insightsCardBody() -> some View {
        padding(.horizontal, InsightsLayout.bodyInset)
            .environment(\.insightsInCardBody, true)
    }
}

/// Explanatory lines behind an "i" beside what they explain (owner,
/// 2026-10-15: on every Insights tab the notices move into info tooltips).
/// The popover is the core's lines; its name is the core's "More about
/// {title}". Nothing is drawn when there are no lines. Status, failures and
/// what an action needs first stay on the page.
struct InsightsInfo: View {
    let title: String
    let lines: [String]

    init(_ title: String, _ lines: [String]) {
        self.title = title
        self.lines = lines
    }

    var body: some View {
        let shown = lines.filter { !$0.isEmpty }
        if !shown.isEmpty {
            GlassInfoButton(SessionReviewCard.moreAbout(title), text: shown.joined(separator: "\n\n"))
        }
    }
}

/// A line of text with its "i" right after the last word, wherever the
/// line wraps (owner, 2026-10-20: "move the tool tip to the end of the
/// string"). A wrapped `Text` fills its row, so an "i" beside it sat at the
/// row's far edge. The words flow as text does (`HistoryFilterFlow`, a
/// space's width apart), and the last word carries the "i", so the "i"
/// never starts a line alone. With nothing behind the "i" it is a plain
/// `Text`. Spoken as the whole line, with the "i" inside it.
struct InsightsTextWithInfo: View {
    let text: String
    let style: GlassTypeStyle
    let infoTitle: String
    let info: [String]

    init(_ text: String, style: GlassTypeStyle, infoTitle: String, info: [String]) {
        self.text = text
        self.style = style
        self.infoTitle = infoTitle
        self.info = info
    }

    /// The words in order; runs of spaces are one break.
    static func words(_ text: String) -> [String] {
        text.split(separator: " ", omittingEmptySubsequences: true).map(String.init)
    }

    /// A space's width at the style's size.
    static func wordGap(_ style: GlassTypeStyle) -> CGFloat { (style.size * 0.28).rounded() }

    var body: some View {
        if info.allSatisfy(\.isEmpty) {
            Text(text).glassType(style).fixedSize(horizontal: false, vertical: true)
        } else {
            let words = Self.words(text)
            HistoryFilterFlow(spacing: Self.wordGap(style), lineSpacing: style.lineSpacing) {
                ForEach(Array(words.dropLast().enumerated()), id: \.offset) { _, word in
                    Text(word).accessibilityHidden(true)
                }
                HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
                    Text(words.last ?? "").accessibilityHidden(true)
                    InsightsInfo(infoTitle, info)
                }
            }
            .glassType(style)
            .accessibilityElement(children: .contain)
            .accessibilityLabel(text)
        }
    }
}

/// A section's title at the card-title step, its info, and whatever sits
/// at its trailing edge (a refresh, a link).
struct InsightsSectionTitle<Trailing: View>: View {
    let title: String
    let info: [String]
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            InsightsTextWithInfo(title, style: GlassTokens.TypeScale.title, infoTitle: title, info: info)
                .foregroundStyle(GlassColor.textPrimary)
            Spacer(minLength: GlassTokens.Space.s4)
            trailing()
        }
        .accessibilityAddTraits(.isHeader)
    }
}

extension InsightsSectionTitle where Trailing == EmptyView {
    init(_ title: String, info: [String] = []) {
        self.init(title: title, info: info, trailing: { EmptyView() })
    }
}

extension View {
    /// A headline figure: the number step, tabular.
    func insightsFigure() -> some View {
        glassType(GlassTokens.TypeScale.number).foregroundStyle(GlassColor.textPrimary)
    }

    /// The name over a figure.
    func insightsEyebrow() -> some View {
        glassType(GlassTokens.TypeScale.eyebrow).foregroundStyle(GlassColor.textTertiary)
    }

    /// An axis or bucket label on a chart.
    func insightsTick(current: Bool = false) -> some View {
        glassType(GlassTokens.TypeScale.micro)
            .foregroundStyle(current ? GlassColor.textPrimary : GlassColor.textTertiary)
    }

    /// A chart's ground: the opaque chart surface, never glass. Its corner
    /// is the card's less the card's padding, held at the control radius.
    func insightsPlot() -> some View {
        padding(.horizontal, GlassTokens.Space.s5)
            .padding(.top, GlassTokens.Space.s5)
            .padding(.bottom, GlassTokens.Space.s4)
            .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(GlassTokens.Color.chartSurface.color))
            .modifier(InsightsInlineContainer())
    }
}

/// The four token counters, each with its chart series, in palette order.
/// Charts stack them from the axis as `stack` lists, so neighbours differ
/// in lightness as well as hue.
enum InsightsSeries: CaseIterable {
    case cacheRead, uncached, cacheWrite, output

    var key: String {
        switch self {
        case .cacheRead: "analytics_series_cache_read"
        case .uncached: "analytics_series_uncached"
        case .cacheWrite: "analytics_series_cache_write"
        case .output: "analytics_series_output"
        }
    }

    var color: GlassRGBA {
        switch self {
        case .cacheRead: GlassTokens.Color.chartSeries1
        case .uncached: GlassTokens.Color.chartSeries2
        case .cacheWrite: GlassTokens.Color.chartSeries3
        case .output: GlassTokens.Color.chartSeries4
        }
    }

    /// From the axis up: cache read, cache write, uncached, output.
    static let stack: [InsightsSeries] = [.cacheRead, .cacheWrite, .uncached, .output]
}

/// The coverage line behind a status dot: green when every count is
/// known, amber when any is partial or unknown, grey with no traces.
struct InsightsCoverageLine: View {
    let coverage: InsightsWeekCoverage
    let copy: [String: String]
    /// What qualifies the counts (which feed, undated and overlapping
    /// traces, a tab's own notes), behind the line's "i".
    var info: [String] = []

    static func status(_ coverage: InsightsWeekCoverage) -> GlassStatus {
        if coverage.sessions == 0 { return .off }
        return coverage.partial + coverage.unknown == 0 ? .on : .ask
    }

    /// The line's type step.
    static let style = GlassTokens.TypeScale.label.weight(.regular)

    var body: some View {
        // Top-aligned, the dot centred on the first line: the words flow in
        // a layout of their own (`InsightsTextWithInfo`), which has no
        // first baseline to align on.
        HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
            GlassStatusDot(Self.status(coverage), size: GlassTokens.Size.dotLarge, halo: true)
                .padding(.top, max(0, (Self.style.lineHeight - GlassTokens.Size.dotLarge) / 2))
            InsightsTextWithInfo(InsightsOverviewWords.coverageLine(coverage, copy: copy),
                                 style: Self.style,
                                 infoTitle: copy["analytics_drill_coverage"] ?? "", info: info)
                .foregroundStyle(GlassColor.textSecondary)
        }
    }
}

/// The week on screen, as a select of every week with a figure. Overview
/// and Patterns share it; the window routes a pick to the feed showing.
struct InsightsWeekSelect: View {
    let weekStart: String
    let weeks: [String]
    let copy: [String: String]
    let select: (String) -> Void

    /// Newest first, the week on screen included.
    static func options(weekStart: String, weeks: [String]) -> [GlassPickerOption<String>] {
        var weeks = weeks
        if !weeks.contains(weekStart) { weeks.insert(weekStart, at: 0) }
        return weeks.map { start in
            // UTC midnights, so six whole days land on the Sunday whatever
            // the local zone's daylight changes do.
            let day = Date.ISO8601FormatStyle().year().month().day()
            let end = (try? Date(start, strategy: day))
                .map { $0.addingTimeInterval(6 * 86_400).formatted(day) } ?? start
            return GlassPickerOption(InsightsOverviewWords.weekRange(start: start, end: end), value: start)
        }
    }

    var body: some View {
        GlassSelect(copy["analytics_this_week"] ?? "", selection: Binding(
            get: { weekStart },
            set: { select($0) }
        ), options: Self.options(weekStart: weekStart, weeks: weeks))
        .fixedSize()
    }
}

/// A tool's tile for a harness's wire name (`claude_code`, `codex`); none
/// for a harness this build does not know.
struct InsightsHarnessTile: View {
    let source: String

    var body: some View {
        if let kind = SourceKind(rawValue: source.replacingOccurrences(of: "_", with: "-")) {
            GlassToolTile(.tool(kind.glassTool))
        }
    }
}

/// What a tab's figures cover (owner, 2026-10-19): the harness's tile and
/// the core's line ("Claude Code traces only: 31 of 37."). Since
/// 2026-10-20 it leads the section under the week picker, over the
/// coverage line, rather than being a card of its own beside the picker.
struct InsightsScopeLine: View {
    let source: String
    let line: String

    var body: some View {
        HStack(spacing: GlassTokens.Space.s4) {
            InsightsHarnessTile(source: source)
            Text(line).insightsCaption()
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: 0)
        }
        .accessibilityElement(children: .combine)
    }
}

/// One row of a drill-down, sized for the main pane: the session and its
/// figure on the first line, its label under it, then its coverage state
/// and reasons. A four-column table does not fit 408pt.
struct InsightsDrillRow<Extra: View>: View {
    let title: String
    let figure: String
    let label: String?
    let state: String
    let reasons: String
    @ViewBuilder var extra: () -> Extra

    var body: some View {
        GlassTableRow {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                    Text(title).lineLimit(1).truncationMode(.middle)
                    Spacer(minLength: GlassTokens.Space.s4)
                    Text(figure).insightsMono().monospacedDigit()
                }
                if let label {
                    Text(label).insightsCaption().lineLimit(1).truncationMode(.tail)
                }
                Text(reasons.isEmpty ? state : state + " \u{b7} " + reasons).insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
                extra()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

extension InsightsDrillRow where Extra == EmptyView {
    init(title: String, figure: String, label: String?, state: String, reasons: String) {
        self.init(title: title, figure: figure, label: label, state: state, reasons: reasons, extra: { EmptyView() })
    }
}

/// Whether a view sits in a card's body (`insightsCardBody`), whose text is
/// inset further than the card's title.
extension EnvironmentValues {
    @Entry var insightsInCardBody = false
}

/// An inline container (a chart's well, a quiet card) inside a card's body
/// is not inset as far as the body's text (owner, 2026-10-19): it gives the
/// body's extra inset back on both sides, so its edges line up with the
/// card's title. Outside a card's body it is left as it is.
struct InsightsInlineContainer: ViewModifier {
    @Environment(\.insightsInCardBody) private var inBody

    func body(content: Content) -> some View {
        content.padding(.horizontal, inBody ? -InsightsLayout.bodyInset : 0)
    }
}
