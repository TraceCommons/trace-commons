import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// When Insights has nothing of the person's own to show (product owner,
/// 2026-10-29): no trace in the week on screen or in any kept week, or, on
/// Submitted, no trace ever sent (`InsightsSubmittedWords.isEmpty`). Kept
/// pure for tests.
enum InsightsEmptyState {
    /// Whether the week on screen, or any week the picker or the daemon
    /// keeps, has a counted trace.
    static func weekHasData(_ overview: InsightsWeekOverview?, history: [InsightsWeekFigures]?) -> Bool {
        if let overview, overview.sessions > 0 || !overview.weeks.isEmpty { return true }
        return (history ?? []).contains { $0.sessions > 0 }
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
    /// The card's words: Overview and Breakdown's, or Submitted's.
    var words: Words = .week

    /// The copy keys of a card: its title, why, three steps (a title and
    /// a detail each), the step the one link sits under, and the sample
    /// banner's detail.
    struct Words {
        let title: String
        let why: String
        let steps: [(title: String, detail: String)]
        let linkStep: Int
        let bannerDetail: String

        static let week = Words(
            title: "analytics_start_title", why: "analytics_start_why",
            steps: [("analytics_start_step_use", "analytics_start_step_use_detail"),
                    ("analytics_start_step_share", "analytics_start_step_share_detail"),
                    ("analytics_start_step_fill", "analytics_start_step_fill_detail")],
            linkStep: 1, bannerDetail: "analytics_sample_banner_detail")

        /// Submitted (product owner, 2026-10-29): every step leads to the
        /// first trace sent, and the link sits under choosing a folder.
        static let submitted = Words(
            title: "analytics_sub_start_title", why: "analytics_sub_start_why",
            steps: [("analytics_sub_start_step_folder", "analytics_sub_start_step_folder_detail"),
                    ("analytics_sub_start_step_send", "analytics_sub_start_step_send_detail"),
                    ("analytics_sub_start_step_see", "analytics_sub_start_step_see_detail")],
            linkStep: 0, bannerDetail: "analytics_sub_sample_banner_detail")

        var keys: [String] { [title, why, bannerDetail] + steps.flatMap { [$0.title, $0.detail] } }
    }
}

/// The get-started card, then the sample banner. Every word is the core's.
struct InsightsGetStartedHeader: View {
    let start: InsightsGetStarted

    var body: some View {
        VStack(alignment: .leading, spacing: InsightsLayout.sectionGap) {
            InsightsGetStartedCard(start: start)
            InsightsSampleBanner(copy: start.copy, detailKey: start.words.bannerDetail)
        }
    }
}

/// "Your Insights grow with your traces": why it is worth it, then
/// three numbered steps, and one action: a text link under step 2 to the
/// folder list (product owner, 2026-10-29: one clear action, small step
/// numbers, room between the steps, not a setup wizard).
struct InsightsGetStartedCard: View {
    let start: InsightsGetStarted

    private func text(_ key: String) -> String { start.copy[key] ?? "" }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s9) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s5) {
                Image(systemName: "chart.bar.xaxis")
                    .glassGlyph(18, weight: .semibold)
                    .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                    .frame(width: 40, height: 40)
                    .background(Circle().fill(GlassTokens.Color.chartSeries1.color.opacity(0.14)))
                    .accessibilityHidden(true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(text(start.words.title)).insightsTitle()
                        .fixedSize(horizontal: false, vertical: true)
                        .accessibilityAddTraits(.isHeader)
                    Text(text(start.words.why)).insightsNote()
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s9) {
                ForEach(Array(start.words.steps.enumerated()), id: \.offset) { index, keys in
                    step(index + 1, title: text(keys.title), detail: text(keys.detail)) {
                        if index == start.words.linkStep { link }
                    }
                }
            }
            .insightsCardBody()
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .insightsSection()
    }

    /// The card's one action: a text link with an arrow to the folder list.
    @ViewBuilder private var link: some View {
        if let openTraces = start.openTraces {
            Button(action: openTraces) {
                HStack(spacing: GlassTokens.Space.s2) {
                    Text(text("analytics_start_choose"))
                    Image(systemName: "arrow.right").accessibilityHidden(true)
                }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .padding(.top, GlassTokens.Space.s3)
        }
    }

    /// One numbered step: its number in a disc, its title and its detail,
    /// and what the step can open under them.
    private func step<Action: View>(_ number: Int, title: String, detail: String,
                                    @ViewBuilder action: () -> Action) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s5) {
            Text(String(number))
                .glassType(GlassTokens.TypeScale.micro)
                .monospacedDigit()
                .foregroundStyle(GlassTokens.Color.chartSeries1.color)
                .frame(width: 18, height: 18)
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
    var detailKey = "analytics_sample_banner_detail"

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
                Text(copy[detailKey] ?? "").insightsCaption()
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
