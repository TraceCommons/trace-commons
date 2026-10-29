import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// When Insights has nothing of the person's own to show (product owner,
/// 2026-10-29): no trace in the week on screen or in any kept week, or, on
/// Benchmarks, no kind of work with a trace. Kept pure for tests.
enum InsightsEmptyState {
    /// The two folders the app watches for Claude and Codex traces.
    static let watchedFolders = (claude: "~/.claude/projects", codex: "~/.codex/sessions")

    /// Whether the week on screen, or any week the picker or the daemon
    /// keeps, has a counted trace.
    static func weekHasData(_ overview: InsightsWeekOverview?, history: [InsightsWeekFigures]?) -> Bool {
        if let overview, overview.sessions > 0 || !overview.weeks.isEmpty { return true }
        return (history ?? []).contains { $0.sessions > 0 }
    }

    /// Whether Benchmarks has a kind of work with a trace in it.
    static func benchmarksHaveData(_ answer: DaemonData.InsightsBenchmarks?) -> Bool {
        guard let answer, answer.enabled, answer.readable != false else { return false }
        return (answer.categories ?? []).contains { $0.sessions > 0 }
    }
}

/// What a tab draws above its sample: the get-started card's words and
/// where its buttons go. A tab given one draws the card, the sample
/// banner, and its own content dimmed and inert under them.
struct InsightsGetStarted {
    let copy: [String: String]
    /// Traces, on its folder list; `nil` draws no button.
    var openTraces: (() -> Void)?
    /// Settings, at Projects; `nil` draws no button.
    var openSettings: (() -> Void)?
}

/// The get-started card, then the sample banner. Every word is the core's.
struct InsightsGetStartedHeader: View {
    let start: InsightsGetStarted

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            InsightsGetStartedCard(start: start)
            InsightsSampleBanner(copy: start.copy)
        }
    }
}

/// "Your Insights fill in from your own traces": why it is worth it, then
/// three numbered steps naming the app's own screens, with a button where
/// the app can open the screen.
struct InsightsGetStartedCard: View {
    let start: InsightsGetStarted

    private func text(_ key: String) -> String { start.copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                Image(systemName: "chart.bar.xaxis")
                    .glassGlyph(18, weight: .semibold)
                    .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                    .frame(width: 40, height: 40)
                    .background(Circle().fill(GlassTokens.Color.chartSeries1.color.opacity(0.14)))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(text("analytics_start_title")).insightsTitle()
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    Text(text("analytics_start_why")).insightsNote()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                step(1, title: text("analytics_start_step_use"),
                     detail: InsightsOverviewWords.fill(text("analytics_start_step_use_detail"), [
                         "first": InsightsEmptyState.watchedFolders.claude,
                         "second": InsightsEmptyState.watchedFolders.codex,
                     ])) { EmptyView() }
                step(2, title: text("analytics_start_step_share"),
                     detail: text("analytics_start_step_share_detail")) {
                    if start.openTraces != nil || start.openSettings != nil {
                        HStack(spacing: GlassTokens.Space.s4) {
                            if let openTraces = start.openTraces {
                                Button(text("analytics_start_open_traces")) { openTraces() }
                                    .buttonStyle(GlassButtonStyle(.primary, small: true))
                            }
                            if let showSettings = start.openSettings {
                                Button(text("analytics_start_open_settings")) { showSettings() }
                                    .buttonStyle(GlassButtonStyle(.link))
                            }
                        }
                        .padding(.top, GlassTokens.Space.s2)
                    }
                }
                step(3, title: text("analytics_start_step_fill"),
                     detail: text("analytics_start_step_fill_detail")) { EmptyView() }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// One numbered step: its number in a disc, its title and its detail,
    /// and what the step can open under them.
    private func step<Action: View>(_ number: Int, title: String, detail: String,
                                    @ViewBuilder action: () -> Action) -> some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
            Text(String(number))
                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                .monospacedDigit()
                .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                .frame(width: 24, height: 24)
                .background(Circle().fill(GlassTokens.Color.chartSeries1.color.opacity(0.14)))
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                Text(title).glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(detail).insightsNote()
                    .fixedSize(horizontal: false, vertical: true)
                action()
            }
            .frame(maxWidth: .infinity, alignment: .leading)
        }
        .accessibilityElement(children: .contain)
    }
}

/// "Sample data — not yours", full width above the sample.
struct InsightsSampleBanner: View {
    let copy: [String: String]

    var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Image(systemName: "eye")
                .glassGlyph(14, weight: .semibold)
                .foregroundStyle(GlassStatus.ask.textColor)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(copy["analytics_sample_banner"] ?? "")
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                Text(copy["analytics_sample_banner_detail"] ?? "").insightsCaption()
                    .fixedSize(horizontal: false, vertical: true)
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, GlassTokens.Space.s6)
        .padding(.vertical, GlassTokens.Space.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
            .fill(GlassStatus.ask.color.opacity(0.12)))
        .overlay(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
            .strokeBorder(GlassStatus.ask.color.opacity(0.55), style: StrokeStyle(lineWidth: 1, dash: [5, 4])))
        .accessibilityElement(children: .combine)
    }
}

extension View {
    /// A tab's content: as is, or, under the get-started card, the sample:
    /// dimmed, inert and left out of VoiceOver, so it never reads as the
    /// person's own.
    @ViewBuilder
    func insightsSample(_ isSample: Bool) -> some View {
        if isSample {
            self.opacity(0.55)
                .allowsHitTesting(false)
                .accessibilityHidden(true)
        } else {
            self
        }
    }
}

/// A tab's scrolling page: the get-started header when there is one, then
/// the tab's content, the sample's when there is a header.
struct InsightsTabPage<Content: View>: View {
    let getStarted: InsightsGetStarted?
    @ViewBuilder var content: () -> Content

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
                if let getStarted { InsightsGetStartedHeader(start: getStarted) }
                VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) { content() }
                    .insightsSample(getStarted != nil)
            }
            .padding(.vertical, GlassTokens.Space.s7)
            .frame(maxWidth: .infinity, alignment: .leading)
            .insightsSurface()
        }
    }
}
