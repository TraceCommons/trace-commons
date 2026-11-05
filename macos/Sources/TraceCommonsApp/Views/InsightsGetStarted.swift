import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// When Insights has nothing of the person's own to show (product owner,
/// 2026-10-29): no trace in the week on screen or in any kept week, or, on
/// Benchmarks, no kind of work with a trace. Kept pure for tests.
enum InsightsEmptyState {
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
/// where its one action goes. A tab given one draws the card, the sample
/// banner, and its own content dimmed and inert under them.
struct InsightsGetStarted {
    let copy: [String: String]
    /// Traces, on its folder list, where each folder is set to Automatic
    /// or Ask; `nil` draws no link.
    var openTraces: (() -> Void)?
    var localAnalysisOff = false
    var enablingLocalAnalysis = false
    var enableLocalAnalysis: (() -> Void)? = nil
}

/// The get-started card, then the sample banner. Every word is the core's.
struct InsightsGetStartedHeader: View {
    let start: InsightsGetStarted

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            if !start.localAnalysisOff { InsightsGetStartedCard(start: start) }
            InsightsSampleBanner(copy: start.copy, localAnalysisOff: start.localAnalysisOff,
                                 enablingLocalAnalysis: start.enablingLocalAnalysis,
                                 enableLocalAnalysis: start.enableLocalAnalysis)
        }
    }
}

/// "Your Insights will appear here": why it is worth it, then three
/// numbered steps, and one action: a text link under step 2 to the folder
/// list (product owner, 2026-10-29: one clear action, small step numbers,
/// room between the steps, not a setup wizard). No icon beside the title,
/// each step one short line, and a step's longer explanation behind the
/// "i" after that line (product owner, 2026-11-02).
struct InsightsGetStartedCard: View {
    let start: InsightsGetStarted

    private func text(_ key: String) -> String { start.copy[key] ?? "" }

    /// The shared getting-started card (`GetStartedCard`), which the
    /// Private AI tab's guide and Home's Traces walkthrough draw too.
    var body: some View {
        GetStartedCard(
            title: text("analytics_start_title"), lead: text("analytics_start_why"),
            steps: [
                .init(title: text("analytics_start_step_use"), detail: text("analytics_start_step_use_detail")),
                .init(title: text("analytics_start_step_share"), detail: text("analytics_start_step_share_detail"),
                      info: text("analytics_start_step_share_info"),
                      link: start.openTraces.map { GetStartedCard.Link(title: text("analytics_start_choose"), action: $0) }),
                .init(title: text("analytics_start_step_fill"), detail: text("analytics_start_step_fill_detail"),
                      info: text("analytics_start_step_fill_info")),
            ])
    }
}

/// "Sample data shown below", full width above the sample: a tinted card with
/// a test tube for "sample", and no border (product owner, 2026-11-02).
struct InsightsSampleBanner: View {
    let copy: [String: String]
    var localAnalysisOff = false
    var enablingLocalAnalysis = false
    var enableLocalAnalysis: (() -> Void)? = nil

    var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s4) {
            Image(systemName: "testtube.2")
                .glassGlyph(14, weight: .semibold)
                .foregroundStyle(GlassStatus.ask.textColor)
                .accessibilityHidden(true)
            VStack(alignment: .leading, spacing: 2) {
                Text(copy["analytics_sample_banner"] ?? "")
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                if localAnalysisOff {
                    Text(copy["analytics_local_analysis_off"] ?? "")
                        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    Text(copy["analytics_local_analysis_detail"] ?? "").insightsCaption()
                    if let enableLocalAnalysis {
                        Button(copy["analytics_local_analysis_enable"] ?? "", action: enableLocalAnalysis)
                            .disabled(enablingLocalAnalysis)
                    }
                } else {
                    Text(copy["analytics_sample_banner_detail"] ?? "").insightsCaption()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            Spacer(minLength: 0)
        }
        .padding(.horizontal, GlassTokens.Space.s6)
        .padding(.vertical, GlassTokens.Space.s4)
        .frame(maxWidth: .infinity, alignment: .leading)
        .background(RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
            .fill(GlassStatus.ask.color.opacity(0.12)))
        .accessibilityElement(children: .contain)
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
