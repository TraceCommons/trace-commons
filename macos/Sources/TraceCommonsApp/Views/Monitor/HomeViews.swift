import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Home tab (R9 of #1173, in Ron's #1146 shape): the status card with
/// its way into Traces, the counts, Missions, and the way into History
/// (spec, "Screens": History is Home, then History).
///
/// Counts come from the core: waiting is `decisions_owed` (a dash when the
/// core did not say), contributed is the rollup's accepted count. Pending
/// credit is shown only beside the commons' own statement of what it waits
/// on (D6), and never as earned.
///
/// Missions is #1146's: the local mission drafts, and its card opens them.
/// The two screens #1146 has no place for stay reachable from quiet cards
/// under History: Insights, and the commons mission catalogue once the
/// daemon answers it.
struct HomeTabView: View {
    /// Declared top-level (`MonitorNavigation.swift`) so a destination can
    /// name a page in a release build.
    typealias Page = HomePage

    let store: HomeStore
    let traces: TracesStore
    /// The core's label for a history status, when its copy has loaded.
    let statusLabel: (String?) -> String?
    @Binding var page: Page
    /// The opened History row's submission id; empty for none. Opening a
    /// row shows its details in the inspector (Ron's inspector auto-open).
    @Binding var selection: String
    /// The status card's "Open Traces" link: switches the window's tab.
    var openTraces: () -> Void = {}
    /// The pending-credit tile: switches the window to the Private AI tab.
    var openPrivateAI: () -> Void = {}
    /// The Contributed tile: History, which lives on the Traces tab (owner,
    /// 2026-10-12).
    var openHistory: () -> Void = {}
    /// The Insights card: switches the window to the Insights tab.
    var openInsights: () -> Void = {}
    /// The hosted Insights and Mission drafts screens' inputs, from
    /// `TraceCommonsAppMain`.
    let insightsStoreSelection: InsightsStoreSelection
    let missionDrafts: MissionDraftsModel
    /// Whether missions are shown (the daemon's `missions_enabled`). Off:
    /// no Missions card, no catalogue card, no mission page, and the
    /// drafts inbox is not read.
    var missionsEnabled: Bool = false
    /// The Private AI tab's store, which the window runs for as long as it
    /// is open: Home's Private AI section reads its state and counts from
    /// it. Nil draws the counts as dashes.
    var inference: InferenceStore? = nil

    /// The core's Insights copy, read once: it is fixed for the life of the
    /// process, and reading it is a C ABI call and a JSON decode, which
    /// `body` would otherwise repeat on every evaluation.
    private nonisolated static let insightsCopy = TCInsights.copy()

    /// Insights' heading, the core's: the Insights tab's name and its card
    /// on Home. Nil until the core's copy has arrived.
    nonisolated static var insightsHeading: String? { HomeFormat.cardHeading(Self.insightsCopy?["title"]) }

    /// The breadcrumb word for a hosted page: the core's heading, or nil
    /// before its copy has arrived. Mission drafts is #1146's Missions, and
    /// its crumb says so; the drafts screen's own title is the core's
    /// (`missionDrafts.copy["title"]`), read by the screen.
    static func hostedHeading(_ page: Page, missionDrafts: MissionDraftsModel) -> String? {
        switch page {
        case .missionDrafts: HomeFormat.cardHeading(MonitorWords.missions)
        case .overview, .history, .missions: nil
        }
    }

    var body: some View {
        // History and Missions are reached from here and lead back here; the
        // breadcrumb that says so is the shell's, under the tabs
        // (`MonitorWindowView.breadcrumb`), as #1146 draws it.
        switch page.shown(missions: missionsEnabled) {
        case .overview:
            HomeOverview(
                store: store, traces: traces, statusLabel: statusLabel, openTraces: openTraces,
                openPrivateAI: openPrivateAI,
                insightsHeading: Self.insightsHeading,
                missionDrafts: missionDrafts, missionsEnabled: missionsEnabled,
                openHistory: openHistory, openMissionDrafts: { page = .missionDrafts },
                openInsights: openInsights, openMissions: { page = .missions }, inference: inference)
        case .history:
            HistoryPage(store: store, statusLabel: statusLabel, selection: $selection, back: { page = .overview })
        case .missions:
            MissionsPage(store: store, back: { page = .overview })
        // The screens built outside the Monitor (R15: the legacy window
        // that held them is gone), hosted as Home pages under the shell's
        // breadcrumb.
        case .missionDrafts:
            MissionDraftsView(model: missionDrafts)
        }
    }
}

/// History on the Traces tab (owner, 2026-10-12): the same page Home
/// hosted, with its breadcrumb back to the tree.
struct TracesHistoryPage: View {
    let store: HomeStore
    let statusLabel: (String?) -> String?
    @Binding var selection: String
    let back: () -> Void

    var body: some View {
        HistoryPage(store: store, statusLabel: statusLabel, selection: $selection, back: back)
    }
}

/// Home's hero cards that the person closed with their X (owner,
/// 2026-10-27: Kimi and Insights were in the person's face), and Settings'
/// switch for the campaign card. Kept in this Mac's defaults; nothing
/// leaves the machine.
///
/// A closed campaign comes back when what the feed says for it changes;
/// the Insights card comes back the next day; promotions off hides the
/// campaign card altogether (owner, 2026-10-27).
enum HomeClosedCards {
    /// The fingerprint of the campaign as it was when closed.
    static let campaignKey = "home.closedCampaignFingerprint"
    /// The day (`day(_:)`) the Insights card was closed on.
    static let insightsKey = "home.insightsCardClosedOn"
    /// Settings' "Show promotions on Home", on by default.
    static let promotionsKey = "home.showsPromotions"

    /// Everything the campaign card shows, so any change in the feed --
    /// under the same id or a new one -- is a different fingerprint. New
    /// art behind the same image URL is not: the feed changes the URL with
    /// the art.
    static func fingerprint(_ campaign: PrivateAICampaign) -> String {
        [campaign.id, campaign.title, campaign.body, campaign.actionTitle, campaign.actionURL.absoluteString,
         campaign.symbol, campaign.imageURL?.absoluteString ?? ""].joined(separator: "\u{1F}")
    }

    /// A campaign shows while promotions are on and it is not the one the
    /// person closed, as it was then.
    static func showsCampaign(_ fingerprint: String, closed: String, promotionsOn: Bool) -> Bool {
        promotionsOn && fingerprint != closed
    }

    /// The day `date` falls on in the person's calendar, as "2026-10-27".
    static func day(_ date: Date, calendar: Calendar = .current) -> String {
        let parts = calendar.dateComponents([.year, .month, .day], from: date)
        return String(format: "%04d-%02d-%02d", parts.year ?? 0, parts.month ?? 0, parts.day ?? 0)
    }

    /// The Insights card shows unless it was closed today: once a day.
    static func showsInsights(closedOn: String, now: Date = Date(), calendar: Calendar = .current) -> Bool {
        closedOn != day(now, calendar: calendar)
    }
}

private struct HomeOverview: View {
    let store: HomeStore
    let traces: TracesStore
    let statusLabel: (String?) -> String?
    let openTraces: () -> Void
    let openPrivateAI: () -> Void
    /// The core's Insights heading; nil until the core's copy has arrived,
    /// and then its card is not drawn.
    let insightsHeading: String?
    let missionDrafts: MissionDraftsModel
    let missionsEnabled: Bool
    let openHistory: () -> Void
    let openMissionDrafts: () -> Void
    let openInsights: () -> Void
    let openMissions: () -> Void
    /// The Private AI section's state and counts (`HomeTabView.inference`).
    let inference: InferenceStore?
    /// The NEAR AI balance the Balance tile reads.
    @EnvironmentObject private var model: AppModel
    /// Where the NEAR AI sign-in's browser page opens.
    @Environment(\.openURL) private var openURL
    @State private var campaignStore = PrivateAICampaignStore()
    /// The campaign closed with its X, as it was then (`fingerprint`): it
    /// comes back when the feed changes it (owner, 2026-10-27).
    @AppStorage(HomeClosedCards.campaignKey) private var closedCampaign = ""
    /// The day the Insights card was closed with its X: it is back the
    /// next day; the Insights tab stays where it is (owner, 2026-10-27).
    @AppStorage(HomeClosedCards.insightsKey) private var insightsClosedOn = ""
    /// Settings' "Show promotions on Home": off, no campaign card.
    @AppStorage(HomeClosedCards.promotionsKey) private var promotionsOn = true

    /// The rollup, or nil after a failed `history_rollup` read
    /// (`SummaryFacts.fresh`): a stale one is not shown as current.
    private var freshRollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
    }

    /// The credit summary, or nil after a failed `commons_credit_summary`
    /// read: the earlier settlement sentence is not shown as current.
    private var freshCredit: DaemonData.CommonsCreditSummary? {
        SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
    }

    private var words: MonitorHomeHistoryCopy? { MonitorWords.table?.homeHistory }

    /// The glance, unless its last read failed: a stale day is not drawn
    /// as today's.
    private var freshGlance: DaemonData.InsightsGlance? {
        SummaryFacts.fresh(store.glance, unless: store.failures["insights_glance"])
    }

    /// A native promotion, available from compiled copy before its remote refresh.
    @ViewBuilder
    private var campaign: some View {
        if let campaign = campaignStore.campaign,
           HomeClosedCards.showsCampaign(
               HomeClosedCards.fingerprint(campaign), closed: closedCampaign, promotionsOn: promotionsOn) {
            GlassHeroCard(
                gradient: GlassTokens.Gradient.heroCampaign,
                title: campaign.title, subtitle: campaign.body,
                actionTitle: campaign.actionTitle, action: { openURL(campaign.actionURL) },
                closeLabel: MonitorWords.close, onClose: { closedCampaign = HomeClosedCards.fingerprint(campaign) }
            ) {
                if let image = campaignStore.image {
                    Image(nsImage: image)
                        .resizable()
                        .scaledToFit()
                } else {
                    GlassHeroSymbol(campaign.symbol)
                }
            } content: {
                EmptyView()
            }
        }
    }

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // The core's health, the undos, the offers and the
                // first-contribution note are the Traces tab's popover
                // notices (owner, 2026-10-19, `MonitorNotices`), not Home's.
                // No Watching card (owner, 2026-10-12): the tiles lead.
                // Over the cards below, so the Balance tile's coins fall
                // over them too.
                stats
                    .zIndex(1)
                // The NEAR AI campaign, under the tiles (owner, 2026-10-12).
                campaign
                // Ron's Missions card, as a hero promotion (owner,
                // 2026-10-12): the drafts' line under the subtitle, and
                // Review drafts; no pill.
                if missionsEnabled {
                    GlassHeroCard(
                        gradient: GlassTokens.Gradient.heroMissions,
                        title: MonitorWords.missions, subtitle: words?.missionsSubtitle,
                        actionTitle: words?.missionsAction, action: openMissionDrafts
                    ) {
                        GlassHeroSymbol(Self.missionsHeroSymbol)
                    } content: {
                        drafts
                    }
                }
                // What #1146 has no place for, after its cards: Insights,
                // and the commons catalogue once the daemon answers it.
                // Insights as a hero promotion, today's figures in its words
                // and its graphic (owner, 2026-10-12: an upsell, not a bare
                // way in).
                if let insightsHeading, HomeClosedCards.showsInsights(closedOn: insightsClosedOn) {
                    GlassHeroCard(
                        gradient: GlassTokens.Gradient.heroInsights, title: insightsHeading,
                        subtitle: words?.insightsSubtitle, actionTitle: words?.insightsAction, action: openInsights,
                        closeLabel: MonitorWords.close, onClose: { insightsClosedOn = HomeClosedCards.day(Date()) }
                    ) {
                        HomeInsightsGraphic(glance: freshGlance)
                    } content: {
                        HomeInsightsVisual(glance: freshGlance, words: words)
                    }
                }
                // Sent traces without Train coding models, and Review, under
                // the Insights card while there are any. Not closed with the
                // card: it is the way back to a choice, not a promotion.
                SentTracesTrainingLine(model: model.sentTracesTraining, history: store.history)
                if missionsEnabled, store.missions != nil, let catalogue = words?.missionCatalogue {
                    hostedCard(
                        systemImage: Self.catalogueIcon, title: catalogue,
                        subtitle: words?.catalogueSubtitle, action: openMissions)
                }
            }
        }
        .scrollIndicators(.never)
        .task { await campaignStore.refresh() }
        .task(id: missionsEnabled) {
            // Hidden missions read nothing: the drafts inbox stays on disk.
            if missionsEnabled { missionDrafts.preview() }
        }
    }

    /// The Home cards' icons, in the Private AI cards' place for them.
    static let missionsIcon = HomeOverviewIcons.missions
    static let historyIcon = HomeOverviewIcons.history
    static let insightsIcon = HomeOverviewIcons.insights
    static let catalogueIcon = HomeOverviewIcons.catalogue
    /// The hero promotions' graphics.
    static let missionsHeroSymbol = "scope"

    /// A Home card that is a way into a screen, laid out as the Private AI
    /// cards are (owner, 2026-10-11): the icon, the title over the core's
    /// subtitle, the card's accessory on the right, the card's rows under
    /// the head, and a chevron centred on the whole card. The whole card
    /// opens the screen.
    private func homeCard<Accessory: View, Content: View>(
        systemImage: String, title: String, subtitle: String?, promo: GlassPromoTreatment? = nil,
        action: @escaping () -> Void,
        @ViewBuilder accessory: () -> Accessory, @ViewBuilder content: () -> Content
    ) -> some View {
        Button(action: action) {
            GlassIconCard(
                systemImage: systemImage, title: title, subtitle: subtitle.map { [$0] } ?? [],
                interactive: true, promo: promo, chevron: true,
                accessory: accessory,
                content: content)
        }
        .buttonStyle(GlassPressStyle())
    }

    /// A way into a screen #1146 has no place for: its icon, heading,
    /// subtitle and a chevron, and nothing else (the screen holds its own
    /// words).
    private func hostedCard(
        systemImage: String, title: String, subtitle: String?, action: @escaping () -> Void
    ) -> some View {
        homeCard(systemImage: systemImage, title: title, subtitle: subtitle, action: action) {
            EmptyView()
        } content: {
            EmptyView()
        }
    }

    /// Waiting, contributed and pending credit (#1146 `home-view.tsx`), each
    /// from the core: waiting is decisions owed, contributed the rollup's
    /// accepted count. Pending credit is a figure only beside the commons'
    /// statement of what it waits on (D6), drawn under the tiles; without
    /// that statement it is a dash, never a bare number that reads as owed.
    ///
    /// Waiting is the Traces store's count, the one the Traces badge reads:
    /// one waiting number. A failed rollup or credit read is a dash, never
    /// the store's earlier value (`freshRollup`, `freshCredit`).
    ///
    /// Each tile is a way to where its number lives (owner, 2026-10-12):
    /// waiting opens Traces, contributed opens History, and pending credit
    /// opens Private AI.
    private var stats: some View {
        // A failed status read draws no dollar line rather than an old one.
        let settlement = SummaryFacts.fresh(store.status, unless: store.failures["status"])?.settlementEstimate
        let uncontributed = HomeUncontributed(traces.tree.allSessions, settlement: settlement)
        // The Traces hero leads (owner, 2026-10-28: the macOS MVP is
        // "contribute traces, earn credit"), above the Traces heading
        // (owner, 2026-10-29), so the heading sits over Waiting and
        // Contributed. Then three groups, each under its title: Traces
        // (the tiles and what is not contributed across both columns),
        // Account (Credit and the NEAR AI balance), then Private AI, below
        // and smaller than Traces.
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            tracesHero
            HomeSection(words?.sectionTraces) {
                Grid(horizontalSpacing: GlassTokens.Space.s3, verticalSpacing: GlassTokens.Space.s3) {
                    GridRow {
                        HomeStatTile(label: MonitorWords.waiting, value: HomeFormat.count(traces.decisionsOwed),
                                     systemImage: HomeStatIcons.waiting, action: openTraces)
                        HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(freshRollup?.allTime?.accepted),
                                     systemImage: HomeStatIcons.contributed, action: openHistory)
                    }
                    if uncontributed.count > 0, let title = HomeFormat.cardHeading(words?.uncontributed) {
                        GridRow {
                            notContributed(uncontributed, title: title)
                                .gridCellColumns(2)
                        }
                    }
                }
                .fixedSize(horizontal: false, vertical: true)
            }
            HomeSection(words?.sectionAccount) {
                HomeAccountTiles(store: store, openPrivateAI: openPrivateAI)
            }
            // Over Private AI, so the coins that burst from the Balance
            // tile fall over it, not under it.
            .zIndex(1)
            privateAI
        }
    }

    /// Home's Traces hero (owner, 2026-10-28): the first and largest card,
    /// saying what contributing is and that it earns credit, with the way
    /// into Traces, where the person shares. Under its line, three steps
    /// across the whole card, contribute, earn, use Private AI (owner,
    /// 2026-10-29, from the owner's mock-up). Drawn once the core's words
    /// have loaded.
    @ViewBuilder
    private var tracesHero: some View {
        if let words {
            GlassHeroCard(
                gradient: GlassTokens.Gradient.heroMissions,
                title: words.tracesHeroTitle, subtitle: words.tracesHeroBody,
                actionTitle: words.tracesHeroAction, action: openTraces,
                contentSpansGraphic: true, actionOnPill: true
            ) {
                HomeHeroRing()
            } content: {
                HomeHeroSteps(steps: [
                    .init(number: 1, systemImage: "apple.terminal",
                          title: words.tracesHeroStepContribute, detail: words.tracesHeroStepContributeDetail,
                          action: openTraces),
                    .init(number: 2, systemImage: "cylinder.split.1x2",
                          title: words.tracesHeroStepEarn, detail: words.tracesHeroStepEarnDetail),
                    .init(number: 3, systemImage: "sparkles",
                          title: words.tracesHeroStepUse, detail: words.tracesHeroStepUseDetail,
                          action: openPrivateAI),
                ])
            }
        }
    }

    /// Home's Private AI section (owner, 2026-10-20), drawn once the core's
    /// words have loaded, the daemon runs and the NEAR AI sign-in has been
    /// read. It sits under Traces and Account and is smaller than Traces
    /// (owner, 2026-10-28). Signed in, one slim row: the calls answered
    /// here and how many are verified, over the Private AI tab's window.
    /// Signed out, one slim card: "Sign in to NEAR AI to get started." and
    /// Sign in, which begins the NEAR AI sign-in as the Private AI tab's
    /// notice does.
    @ViewBuilder
    private var privateAI: some View {
        if let words, model.privateInferenceCopy != nil, case .running = model.startup,
           let snapshot = model.managedSnapshot {
            HomeSection(words.sectionPrivateAI) {
                if snapshot.nearAiSignedIn {
                    let totals = HomePrivateAI.totals(inference)
                    HomePrivateAIRow(
                        figures: [
                            (MonitorWords.calls, HomeFormat.count(totals.calls)),
                            (InferenceWords.proof(.verified), HomeFormat.count(totals.verified)),
                        ],
                        action: openPrivateAI)
                } else {
                    HomePrivateAISignInCard(
                        line: words.privateAISignIn, button: model.managedText("sign_in_near_ai"),
                        buttonName: model.managedText("sign_in_near_ai_accessibility"), busy: model.credentialBusy) {
                        NearAiSignIn.begin(model, openURL: openURL)
                    }
                }
            }
        }
    }

    /// What waits on this Mac, not yet contributed, across both columns
    /// (owner, 2026-10-19): a slim card with a solid accent border, a large
    /// icon on the left, how many traces and their size, and the core's
    /// call to share. The whole card opens Traces. Drawn only while
    /// something waits.
    private func notContributed(_ uncontributed: HomeUncontributed, title: String) -> some View {
        HomeNotContributedCard(
            title: title, count: uncontributed.count, size: uncontributed.size(words),
            detail: uncontributed.detail(words), action: openTraces)
    }


    /// The Missions hero's line: the newest draft and how many there are,
    /// or Ron's empty line; a dash before the inbox has answered, never an
    /// empty inbox. In the hero's ink.
    @ViewBuilder
    private var drafts: some View {
        Group {
            if let summary = missionDrafts.summary, let words {
                if let first = summary.first {
                    Text(summary.count > 1 ? "\(first.id) · +\(summary.count - 1)" : first.id)
                        .lineLimit(1)
                        .truncationMode(.middle)
                } else {
                    Text(words.noMissionDrafts)
                }
            } else {
                Text("—").accessibilityLabel(MonitorWords.unknown)
            }
        }
        .glassType(GlassTokens.TypeScale.label.weight(.semibold))
        .foregroundStyle(GlassTokens.Color.heroInk.color.opacity(0.92))
    }

}

/// The Not contributed card (owner, 2026-10-19): half a hero's height; the
/// label and the count and size on the left, and the core's call to share
/// (owner, 2026-10-20) on the right in a translucent card of its own
/// (owner, 2026-10-27), over its icon as a watermark on the right, as the
/// tiles'. It is on the standard card fill with no border, and now and
/// then a soft band of light crosses it (`GlassCardShimmer`, owner,
/// 2026-10-27; before that a soft light turned round its edge, before that
/// a duller band crossed it, before that it had a solid accent border, and
/// before that a turning glow edge). The whole card is the button.
private struct HomeNotContributedCard: View {
    let title: String
    let count: Int
    let size: String?
    let detail: String?
    let action: () -> Void

    var body: some View {
        let shape = RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous)
        Button(action: action) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    Text(title)
                        .glassType(GlassTokens.TypeScale.eyebrow)
                        .foregroundStyle(GlassColor.textTertiary)
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s3) {
                        Text(String(count))
                            .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                            .monospacedDigit()
                            .foregroundStyle(GlassColor.textPrimary)
                        if let size {
                            Text(size)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                        }
                    }
                }
                .fixedSize()
                if let detail {
                    // A card on the card: the nested card's translucent
                    // fill (`GlassGround`), so the watermark shows through.
                    Text(detail)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                        .padding(.horizontal, GlassTokens.Space.s6)
                        .padding(.vertical, GlassTokens.Space.s4)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .glassTier(.cardQuiet)
                } else {
                    Spacer(minLength: 0)
                }
            }
            // The tiles' inset, so its words start where theirs do.
            .padding(.horizontal, 12)
            .padding(.vertical, GlassTokens.Space.s5)
            .frame(maxWidth: .infinity, alignment: .leading)
            // Its icon is the card's watermark, as the tiles' is, behind
            // the words (owner, 2026-10-20).
            .glassCardWatermark(HomeStatIcons.uncontributed)
            // The stat tiles' fill and hover.
            .glassTier(.card, hover: GlassTokens.Color.cardHover)
            // A soft band of light crossing it, over the fill and the words
            // (owner, 2026-10-27).
            .glassCardShimmer()
            .contentShape(shape)
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}

/// Home's stat tile icons (owner, 2026-10-12).
enum HomeStatIcons {
    static let waiting = "hourglass"
    static let contributed = "checkmark.seal"
    static let credit = "dollarsign.circle"
    /// Held on this Mac, not yet shared: local and private.
    static let uncontributed = "lock.laptopcomputer"
    /// The NEAR AI balance (owner, 2026-10-19).
    static let balance = "creditcard"
    /// Home's Private AI tiles (owner, 2026-10-20).
    static let calls = "arrow.left.arrow.right"
    static let verified = "checkmark.shield"
}

/// Home's Private AI card while NEAR AI is signed out (owner,
/// 2026-10-20): the core's line and Sign in, centred on the card's height,
/// which the button sets. No icon. Sign in waits while a sign-in is under
/// way.
private struct HomePrivateAISignInCard: View {
    let line: String
    let button: String
    /// The button's name for VoiceOver: "Sign in" alone does not say to
    /// what.
    let buttonName: String
    let busy: Bool
    let action: () -> Void

    var body: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s3) {
            Text(line)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Spacer(minLength: GlassTokens.Space.s4)
            Button(button, action: action)
                .buttonStyle(GlassButtonStyle(.primary, small: true))
                .fixedSize()
                .accessibilityLabel(buttonName)
                .disabled(busy)
        }
        .padding(.vertical, 10)
        .padding(.horizontal, 12)
        .frame(maxWidth: .infinity, alignment: .leading)
        .glassTier(.card)
        .accessibilityElement(children: .contain)
    }
}

/// Home's Private AI row while NEAR AI is signed in (owner, 2026-10-28:
/// below and smaller than Traces): one slim card the sign-in card's
/// height, each figure's word beside its number, and a chevron. The whole
/// row opens Private AI.
private struct HomePrivateAIRow: View {
    let figures: [(label: String, value: String)]
    let action: () -> Void

    var body: some View {
        Button(action: action) {
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s6) {
                ForEach(figures.indices, id: \.self) { index in
                    HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                        Text(figures[index].label)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                        Text(figures[index].value)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .monospacedDigit()
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                }
                Spacer(minLength: GlassTokens.Space.s4)
                Image(systemName: "chevron.right")
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .accessibilityHidden(true)
            }
            .padding(.vertical, 10)
            .padding(.horizontal, 12)
            .frame(maxWidth: .infinity, alignment: .leading)
            .glassTier(.card, hover: GlassTokens.Color.cardHover)
            .contentShape(RoundedRectangle(cornerRadius: GlassTier.card.radius, style: .continuous))
        }
        .buttonStyle(GlassPressStyle())
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
    }
}

/// A group of Home's cards under its title (owner, 2026-10-20). With no
/// title (the core's words not loaded yet) the cards are drawn alone.
struct HomeSection<Content: View>: View {
    let title: String?
    let content: Content

    init(_ title: String?, @ViewBuilder content: () -> Content) {
        self.title = title
        self.content = content()
    }

    /// The title's face: the tabs' heading face (`GlassTabHeading`), so
    /// every heading on a tab reads alike.
    static var titleType: GlassTypeStyle { GlassTabHeading<EmptyView>.titleType }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            if let title, !title.isEmpty {
                Text(title)
                    .glassType(Self.titleType)
                    .foregroundStyle(GlassColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
            }
            content
        }
    }
}

/// Home's Private AI section's reading of the Private AI tab's store.
@MainActor
enum HomePrivateAI {
    /// Full-window daemon totals, independent of the shared proxy's calls page.
    static func totals(_ store: InferenceStore?) -> (calls: Int?, verified: Int?) {
        networkTotals(store?.networkSummary)
    }

    static func networkTotals(_ network: DaemonData.NetworkInferenceSummary?) -> (calls: Int?, verified: Int?) {
        guard let network, network.readable, let summary = network.summary, summary.enabled else {
            return (nil, nil)
        }
        let routes = [summary.routed, summary.outside, summary.unknown]
        return (exactTotal(routes.map(\.calls)), exactTotal(routes.map { $0.proof.verified }))
    }

    private static func exactTotal(_ parts: [UInt64]) -> Int? {
        var total: UInt64 = 0
        for part in parts {
            let sum = total.addingReportingOverflow(part)
            guard !sum.overflow else { return nil }
            total = sum.partialValue
        }
        return Int(exactly: total)
    }
}

/// The Home cards' icons, shared with the edge rail's Home peek.
enum HomeOverviewIcons {
    static let missions = "sparkles"
    static let history = "clock.arrow.circlepath"
    static let insights = "chart.xyaxis.line"
    static let catalogue = "list.bullet.rectangle"
}

/// Home's status line (Ron's status card): the dot and the watching line,
/// in the stack-wide states. Shared with the edge rail's Home peek, so the
/// two never say different things.
struct HomeStatusLine: View {
    let store: HomeStore

    var body: some View {
        let state = HomeFormat.watchingState(store)
        return HStack(spacing: GlassTokens.Space.s6) {
                switch state {
                case .ready:
                    switch HomeFormat.watching(store.status, destinations: store.destinations) {
                    case .signedOut:
                        GlassStatusDot(.ask, ring: true)
                        Text(MonitorWords.signedOut)
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    case .unhealthy(let label):
                        GlassStatusDot(.outside, ring: true)
                        Text(HealthCopy.core(label: label, maxQueueEntries: nil).title)
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    case .watching(let tools):
                        GlassStatusDot(.on, ring: true)
                        Text(MonitorWords.table?.shell.watching(tools: tools) ?? FlowMapScene.pair(MonitorWords.watching, tools))
                            .alignmentGuide(.homeStatusText) { $0[.leading] }
                    }
                case .paused:
                    GlassStatusDot(.ask, ring: true)
                    Text(MonitorWords.paused)
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                case .coreDown:
                    GlassStatusDot(.outside, ring: true)
                    Text(store.failures["status"].flatMap { MonitorWords.table?.line(for: $0) } ?? "—")
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                case .loading, .unknown:
                    // No dot: unknown is never drawn as on, or as off.
                    Text("—").accessibilityLabel(MonitorWords.unknown)
                        .alignmentGuide(.homeStatusText) { $0[.leading] }
                }
        }
    }
}

/// Rows inside a Home card (Ron's `home-view.tsx`): text on the left, a tag
/// on the right, and a hairline with 6pt above every row after the first.
private struct HomeCardRows<Item: Identifiable, Words: View, Tag: View>: View {
    let items: [Item]
    let words: (Item) -> Words
    let tag: (Item) -> Tag

    init(_ items: [Item], @ViewBuilder text: @escaping (Item) -> Words, @ViewBuilder tag: @escaping (Item) -> Tag) {
        self.items = items
        self.words = text
        self.tag = tag
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            ForEach(Array(items.enumerated()), id: \.element.id) { index, item in
                HStack(spacing: GlassTokens.Space.s5) {
                    words(item)
                    Spacer(minLength: 0)
                    tag(item)
                }
                .padding(.top, index == 0 ? 0 : GlassTokens.Space.s3)
                .overlay(alignment: .top) {
                    if index > 0 { GlassHairline(GlassColor.hairline) }
                }
                .accessibilityElement(children: .combine)
            }
        }
    }
}

/// History, in Ron's #1146 order, in the left pane: the three stat cards,
/// community standing, every contribution from this
/// machine under SUBMISSIONS, grouped by PROJECT, with Open and Withdraw on
/// its row, the credit record, and what is held for privacy review (apart,
/// never as rejected). The record by period, which #1146 has no place for,
/// closes the page. Opening a row shows its details in the inspector
/// (`HistoryDetailInspector`, Ron's inspector auto-open).
private struct HistoryPage: View {
    let store: HomeStore
    let statusLabel: (String?) -> String?
    @Binding var selection: String
    let back: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var filter: HistoryList.Filter = .all

    private var words: MonitorHomeHistoryCopy? { MonitorWords.table?.homeHistory }

    // The app's own records, which each row's Withdraw and the opened
    // row's detail resolve against, are read again on every visit: the
    // daemon's view, not the one it had at launch (the legacy screen's rule).
    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // A failed rollup read says so; the cells it feeds
                // read a dash, never the store's earlier counts.
                if let failure = store.failures["history_rollup"] {
                    GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                }
                // Verdict news and the one-time offer to turn verdict
                // notifications on are popover notices over this page
                // (owner, 2026-10-19, `MonitorNotices`).
                HistoryStats(store: store)
                HistoryCommunityCard(store: store)
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        submissionsHeader
                        refreshOutcome
                        list
                    }
                }
                HistoryCreditCard(store: store)
                held
                HistoryPeriodCard(store: store)
            }
        }
        .scrollIndicators(.never)
        .onAppear {
            model.clearHistoryRefresh()
            model.refreshHistory()
            model.refreshAccountSession()
        }
        // Escape goes back to Home, as the shell's breadcrumb does.
        .onExitCommand(perform: back)
    }

    /// The rollup, or nil after a failed `history_rollup` read: a stale one
    /// is not shown as current (`SummaryFacts.fresh`).
    private var rollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
    }

    /// One section heading with its refresh control beside it.
    private var submissionsHeader: some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s6) {
            Text(words?.submissions ?? MonitorWords.history)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
                .accessibilityAddTraits(.isHeader)
            Spacer(minLength: 0)
            refresh
        }
    }

    /// The refresh control (Ron's #1146 HistoryRefreshControl): asks the daemon to check the
    /// server sooner (`refresh_history`), then reads History again, the app's
    /// records and this screen's. Only with the core's words.
    @ViewBuilder
    private var refresh: some View {
        if let words = MonitorWords.table?.historyActions {
            Button(model.historyRefresh == .requesting ? words.requesting : words.requestRefresh) {
                Task { if await model.requestHistoryRefresh() { await store.load() } }
            }
            .buttonStyle(GlassButtonStyle(.link))
            .fixedSize()
            .disabled(model.historyRefresh == .requesting)
        }
    }

    /// What the last refresh request did, in the core's words: asked, or
    /// refused. Nothing before one is made.
    @ViewBuilder
    private var refreshOutcome: some View {
        if let words = MonitorWords.table?.historyActions {
            switch model.historyRefresh {
            case .requested: Text(words.refreshRequested)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            case .failed: GlassAlert(words.refreshFailed)
            case .idle, .requesting: EmptyView()
            }
        }
    }

    /// The core's filter labels, or none at all: a filter with an empty
    /// segment is never drawn, and then every row is shown.
    private var labels: [HistoryList.Filter: String]? {
        HistoryList.labels(shell: MonitorWords.table?.shell)
    }

    private var shownFilter: HistoryList.Filter {
        labels == nil ? .all : filter
    }

    @ViewBuilder
    private var list: some View {
        if let failure = store.failures["list_history"] {
            GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
        }
        if let rows = store.history {
            if let cap = HomeFormat.cap(rows.count, rollup: rollup) {
                Text(cap)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
            }
            if rows.isEmpty {
                Text(MonitorWords.table?.shell.historyEmpty ?? "0")
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                filterBar(rows)
                let groups = HistoryFolders.folders(
                    HistoryList.rows(rows, filter: shownFilter),
                    projectID: { $0.projectId ?? "" }, projectLabel: { $0.projectLabel ?? "—" })
                if groups.isEmpty {
                    Text(MonitorWords.table?.shell.historyFilterEmpty ?? "0")
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textTertiary)
                } else {
                    // Ron's groups: 24 apart, each after the first under a
                    // hairline, a PROJECT eyebrow over the folder, its
                    // record count, then its rows.
                    LazyVStack(alignment: .leading, spacing: GlassTokens.Space.s10) {
                        ForEach(Array(groups.enumerated()), id: \.element.id) { index, group in
                            #if DEBUG
                            let _ = OtherCollectionRowBodies.record("supplementalHistoryGroups", group.id)
                            #endif
                            projectGroup(group, first: index == 0)
                        }
                    }
                    .padding(.top, GlassTokens.Space.s8)
                }
            }
        } else if store.failures["list_history"] == nil {
            // Ron's reading line while History is first read.
            if let words {
                Text(words.readingHistory)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                GlassSpinner(MonitorWords.history).frame(maxWidth: .infinity)
            }
        }
    }

    private func projectGroup(_ group: QueueGroup<DaemonData.HistoryRow>, first: Bool) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            HStack(alignment: .bottom, spacing: GlassTokens.Space.s9) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    if let words {
                        Text(words.project)
                            .glassType(GlassTokens.TypeScale.eyebrow)
                            .foregroundStyle(GlassColor.textTertiary)
                    }
                    Text(group.label)
                        .glassType(GlassTokens.TypeScale.bodyStrong)
                        .foregroundStyle(GlassColor.textPrimary)
                        .lineLimit(1)
                }
                Spacer(minLength: 0)
                Text(words?.records(group.count) ?? "\(group.count)")
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
                    .monospacedDigit()
                    .fixedSize()
            }
            .accessibilityElement(children: .combine)
            LazyVStack(alignment: .leading, spacing: 1) {
                ForEach(group.entries) { row in
                    #if DEBUG
                    let _ = OtherCollectionRowBodies.record("supplementalHistoryRows", row.submissionId)
                    #endif
                    HistoryListRow(
                        row: row, statusLabel: statusLabel,
                        selected: selection == row.submissionId,
                        open: { selection = row.submissionId })
                }
            }
        }
        .padding(.top, first ? 0 : GlassTokens.Space.s9 - 2)
        .overlay(alignment: .top) {
            if !first { GlassHairline(GlassColor.hairline) }
        }
    }

    /// Ron's filter: All, then each status, with its count, as pills that
    /// wrap onto a second line rather than scroll out of sight.
    @ViewBuilder
    private func filterBar(_ rows: [DaemonData.HistoryRow]) -> some View {
        if let labels {
            let counts = HistoryList.counts(rows)
            HistoryFilterFlow(spacing: GlassTokens.Space.s2) {
                ForEach(HistoryList.Filter.allCases, id: \.self) { item in
                    HistoryFilterPill(
                        label: labels[item] ?? "", count: counts[item] ?? 0,
                        selected: filter == item, select: { filter = item })
                }
            }
            .accessibilityElement(children: .contain)
            .accessibilityLabel(words?.filterLabel ?? MonitorWords.history)
        }
    }

    /// Held for review, never as rejected (Ron's PRIVACY REVIEW card): the
    /// count as its heading, the core's sentence, no promised wait, the
    /// server's distinct reasons without digests, and why there is no bulk
    /// action. Only when the rollup counts something held and the core's
    /// words are there (never an empty title). The reasons span every
    /// record the app holds, not the list's capped page.
    @ViewBuilder
    private var held: some View {
        if let quarantined = rollup?.quarantined, quarantined > 0, let words = MonitorWords.table {
            GlassEyebrowCard(words.shell.filterQuarantined) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                    Text(words.homeHistory.held(quarantined))
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    Text(words.heldExplanation)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                    Group {
                        Text(HistoryLegacyWords.typicalWait)
                        ForEach(HeldExplanations.lines(in: model.history.filter { $0.status == "quarantined" }.map(\.explanations)),
                                id: \.self) { line in
                            Text(line)
                        }
                        Text(WithdrawalCopy.noBulkAction)
                    }
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                }
                .fixedSize(horizontal: false, vertical: true)
            }
        }
    }
}

/// One History filter (Ron's `history-filter.tsx`): an 11pt pill, its label
/// and count; the chosen one on a lit fill in primary ink, the others
/// tertiary, lifting to primary under the pointer.
private struct HistoryFilterPill: View {
    let label: String
    let count: Int
    let selected: Bool
    let select: () -> Void
    @State private var hovering = false

    var body: some View {
        Button(action: select) {
            HStack(spacing: GlassTokens.Space.s2) {
                Text(label)
                Text("\(count)").monospacedDigit().opacity(0.7)
            }
            .glassType(GlassTokens.TypeScale.caption.weight(.semibold))
            .foregroundStyle(
                selected ? GlassTokens.Color.textOnSelected.color
                    : hovering ? GlassColor.textPrimary : GlassColor.textTertiary)
            .lineLimit(1)
            .fixedSize()
            .padding(.horizontal, GlassTokens.Space.s5)
            .padding(.vertical, 5)
            .background(Capsule().fill(selected ? GlassTokens.Color.controlSelected.color : Color.clear))
            .contentShape(Capsule())
        }
        .buttonStyle(GlassPressStyle())
        .onHover { hovering = $0 }
        .accessibilityAddTraits(selected ? .isSelected : [])
    }
}

/// Ron's `flex-wrap` for the filter: items left to right, onto the next
/// line when the next one would not fit.
struct HistoryFilterFlow: Layout {
    let spacing: CGFloat
    /// The gap between lines; `spacing` when not given.
    var lineSpacing: CGFloat? = nil

    private var lineGap: CGFloat { lineSpacing ?? spacing }

    func sizeThatFits(proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) -> CGSize {
        let lines = Self.lines(width: proposal.width ?? .infinity, sizes: subviews.map { $0.sizeThatFits(.unspecified) },
                               spacing: spacing)
        let width = lines.map(\.width).max() ?? 0
        let height = lines.map(\.height).reduce(0, +) + lineGap * CGFloat(max(lines.count - 1, 0))
        return CGSize(width: proposal.width.map { min($0, width) } ?? width, height: height)
    }

    func placeSubviews(in bounds: CGRect, proposal: ProposedViewSize, subviews: Subviews, cache: inout ()) {
        let sizes = subviews.map { $0.sizeThatFits(.unspecified) }
        var y = bounds.minY
        for line in Self.lines(width: bounds.width, sizes: sizes, spacing: spacing) {
            var x = bounds.minX
            for index in line.items {
                subviews[index].place(at: CGPoint(x: x, y: y), proposal: ProposedViewSize(sizes[index]))
                x += sizes[index].width + spacing
            }
            y += line.height + lineGap
        }
    }

    /// The items on each line, and each line's width and height. Pure.
    static func lines(width: CGFloat, sizes: [CGSize], spacing: CGFloat) -> [(items: [Int], width: CGFloat, height: CGFloat)] {
        var lines: [(items: [Int], width: CGFloat, height: CGFloat)] = []
        var current: (items: [Int], width: CGFloat, height: CGFloat) = ([], 0, 0)
        for (index, size) in sizes.enumerated() {
            let needed = current.items.isEmpty ? size.width : current.width + spacing + size.width
            if !current.items.isEmpty, needed > width {
                lines.append(current)
                current = ([index], size.width, size.height)
            } else {
                current = (current.items + [index], needed, max(current.height, size.height))
            }
        }
        if !current.items.isEmpty { lines.append(current) }
        return lines
    }
}

/// One contribution in History's list (Ron's `HistoryRow`): a folder tile,
/// then what it is -- the folder, its tool and day, its status line, the
/// server's reasons and its settled credit -- with Open and Withdraw on the
/// right of the row, the one Withdraw on History's page. Withdraw asks
/// first, with the existing confirmation, and stands on the app's own record
/// of the contribution, so a row that record does not hold yet offers none
/// (fail closed). It decides from the status the opened detail decides
/// from, offers Retry after a failed withdrawal, and gives way to the core's
/// sign-in while no account session is active. Without the core's
/// public-run copy the row offers nothing.
private struct HistoryListRow: View {
    let row: DaemonData.HistoryRow
    let statusLabel: (String?) -> String?
    let selected: Bool
    let open: () -> Void
    @EnvironmentObject private var model: AppModel
    @State private var confirming = false

    private var record: HistoryRecord? {
        HistorySelection.record(for: row.submissionId, in: model.history)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // The actions beside the text while the text keeps a readable
            // column; stacked at the row's right, then under the text, as
            // the pane narrows. Never a label broken mid-word.
            ViewThatFits(in: .horizontal) {
                line(stacked: false)
                line(stacked: true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s7) {
                        tile
                        description
                    }
                    if let copy = model.publicRunCopy {
                        HStack(spacing: GlassTokens.Space.s4) { actions(copy) }
                    }
                }
            }
            if let copy = model.publicRunCopy {
                below(copy)
            }
        }
        // #1146's confirmation: a glass modal over the window, not a well
        // under the row (owner: glass modal for confirmations).
        .glassModal(isPresented: Binding(
            get: {
                confirming && control == .withdraw && record.flatMap { model.withdrawals[$0.submissionID] } == nil
            },
            set: { if !$0 { confirming = false } }
        )) {
            if let record, let copy = model.publicRunCopy {
                WithdrawalConfirmationModal(
                    status: SessionDetailView.withdrawalStatus(record, detail: model.sessionDetails[record.submissionID]),
                    keepLabel: copy.keepContribution,
                    inFlight: model.withdrawing.contains(record.submissionID),
                    onKeep: { confirming = false },
                    onConfirm: { model.withdraw(record) }
                )
            }
        }
        .padding(.vertical, GlassTokens.Space.s7)
        .padding(.horizontal, GlassTokens.Space.s2)
        .background(
            RoundedRectangle(cornerRadius: GlassTokens.Radius.control, style: .continuous)
                .fill(selected ? GlassColor.ink(0.12) : Color.clear)
        )
        .overlay(alignment: .bottom) {
            GlassHairline(GlassColor.hairline)
        }
        .accessibilityElement(children: .contain)
        .accessibilityAddTraits(selected ? .isSelected : [])
    }

    /// #1146's tile: the folder's first letter.
    private var tile: some View {
        GlassToolTile(.folderInitial(row.projectLabel ?? ""), large: true)
            .frame(width: 38, alignment: .leading)
    }

    private func line(stacked: Bool) -> some View {
        HStack(alignment: .center, spacing: GlassTokens.Space.s7) {
            tile
            description
                .frame(minWidth: 120, idealWidth: 120, maxWidth: .infinity, alignment: .leading)
            if let copy = model.publicRunCopy {
                let layout = stacked
                    ? AnyLayout(VStackLayout(alignment: .trailing, spacing: GlassTokens.Space.s3))
                    : AnyLayout(HStackLayout(spacing: GlassTokens.Space.s4))
                layout { actions(copy) }
                    .fixedSize()
            }
        }
    }

    /// The folder in 600, its tool and day, Ron's "Status:" line, the
    /// server's reasons, and the settled credit.
    private var description: some View {
        let words = MonitorWords.table?.homeHistory
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(row.projectLabel ?? "—")
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .lineLimit(1)
            Text(HomeFormat.rowMeta(row))
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            if let word = HomeFormat.statusWord(row.status, label: statusLabel) {
                if let words {
                    Text(words.status(word))
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                } else {
                    GlassTag(word, tone: HomeFormat.tone(row.status)).fixedSize()
                }
            }
            explanations
            if let words, let reason = HomeFormat.noCreditReason(row, copy: model.publicRunCopy) {
                Text(words.noCredit(reason))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            } else if let figure = HomeFormat.credit(row), let words {
                Text(words.credit(figure))
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
        }
        .accessibilityElement(children: .combine)
    }

    /// The server's own reasons for this row, without opaque digests; a held
    /// row the server said nothing about reads the core's held sentence
    /// (`MonitorScreensCopy.heldExplanation`); without the core's words it
    /// reads nothing, never a blank line.
    @ViewBuilder
    private var explanations: some View {
        let lines = HeldExplanations.lines(in: [row.explanations ?? []])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            ForEach(lines, id: \.self) { Text($0).fixedSize(horizontal: false, vertical: true) }
            if lines.isEmpty, row.status == "quarantined", !MonitorWords.heldExplanation.isEmpty {
                Text(MonitorWords.heldExplanation).fixedSize(horizontal: false, vertical: true)
            }
        }
        .glassType(GlassTokens.TypeScale.caption)
        .foregroundStyle(GlassColor.textSecondary)
    }

    private var control: HistoryList.RowWithdraw {
        let detail = record.flatMap { model.sessionDetails[$0.submissionID] }
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        return HistoryList.rowWithdraw(record: record, detail: detail, result: result, account: model.accountSession)
    }

    /// Open, and Withdraw (or what stands in its place) while no outcome is
    /// drawn under the row.
    @ViewBuilder
    private func actions(_ copy: PublicRunCopy) -> some View {
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        let control = control
        Button(MonitorWords.table?.shell.open ?? copy.viewSession, action: open)
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .frame(minHeight: 44)
        if result == nil {
            withdrawControl(control, copy: copy)
        }
    }

    /// What runs the row's full width under it: the withdrawal outcome and
    /// Retry, and sign-in's progress and failure lines. The confirmation is
    /// a glass modal over the window (`WithdrawalConfirmationModal`).
    @ViewBuilder
    private func below(_ copy: PublicRunCopy) -> some View {
        let result = record.flatMap { model.withdrawals[$0.submissionID] }
        let control = control
        if record != nil {
            if let result {
                // A failure is said under the retry it is about; any other
                // outcome above the control (Ron, 2026-10-09).
                let failed = WithdrawalOutcomeView.isFailure(result)
                if HistoryList.showsOutcome(result), !failed {
                    WithdrawalOutcomeView(result: result)
                }
                withdrawControl(control, copy: copy)
                if HistoryList.showsOutcome(result), failed {
                    WithdrawalOutcomeView(result: result)
                }
            }
        }
        if control == .signIn, let words = MonitorWords.table?.historyActions {
            HistorySignInStatus(words: words)
        }
    }

    /// What stands in Withdraw's place (`HistoryList.rowWithdraw`). Sign-in
    /// and the account check need the core's words, and draw nothing
    /// without them.
    @ViewBuilder
    private func withdrawControl(_ control: HistoryList.RowWithdraw, copy: PublicRunCopy) -> some View {
        let words = MonitorWords.table?.historyActions
        switch control {
        // #1146: Withdraw reads in the outside ink.
        case .withdraw: Button(copy.withdraw) { confirming = true }
            .buttonStyle(GlassButtonStyle(.destructive, small: true))
            .frame(minHeight: 44)
            .disabled(record.map { model.withdrawing.contains($0.submissionID) } ?? true)
        // Retry withdraws without asking again, as the legacy
        // `SessionWithdrawalAction` does: the first attempt already asked.
        // (Ron's "Try again" asks again; native's behaviour is kept, under
        // his word.)
        case .retry:
            if let record {
                Button(WithdrawalCopy.tryAgain) { model.withdraw(record) }
                    .buttonStyle(GlassButtonStyle(.glass, small: true))
                    .frame(minHeight: 44)
                    .disabled(model.withdrawing.contains(record.submissionID))
            }
        case .signIn:
            if let words {
                HistorySignInControl(words: words)
            }
        case .checkingAccount:
            if let words {
                Text(words.checkingAccount)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .frame(minHeight: 44)
            }
        case .none:
            EmptyView()
        }
    }
}

/// Ron's `AccountSignInControl`: sign in through the native identity path
/// (the daemon opens the browser), and say so while it waits. The button
/// sits on the row; what it says while waiting, and why a sign-in did not
/// leave an active session, run under the row (`HistorySignInStatus`). All
/// in the core's words.
private struct HistorySignInControl: View {
    let words: MonitorHistoryActionsCopy
    @EnvironmentObject private var model: AppModel

    var body: some View {
        let label = model.canRestartNearAiBrowserSignIn
            ? HomeFormat.creditSignInLine ?? words.signInToWithdraw
            : model.accountSigningIn ? words.waitingForSignIn : words.signInToWithdraw
        Button(label) { Task { await model.signInToWithdraw() } }
            .buttonStyle(GlassButtonStyle(.glass, small: true))
            .frame(minHeight: 44)
            .disabled(model.accountSigningIn && !model.canRestartNearAiBrowserSignIn)
    }
}

/// Sign-in's progress line and its failure, under the row.
private struct HistorySignInStatus: View {
    let words: MonitorHistoryActionsCopy
    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            if model.accountSigningIn { Text(words.completeSignIn)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            }
            if let failure = model.accountSignInFailure {
                GlassAlert(line(failure))
            }
        }
    }

    private func line(_ failure: AppModel.AccountSignInFailure) -> String {
        switch failure {
        case .inactive: words.signInInactive
        case .unverified: words.signInUnverified
        case .failed: words.signInFailed
        }
    }
}

/// History's stat cards (Ron's `HistoryPage`): pending credit under D6,
/// contributed, and held for review, as Home's three are drawn.
private struct HistoryStats: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash in every cell, never the earlier counts.
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        let credit = SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
            HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                // The same figures as Home's Credit tile (#1708), so the two
                // never disagree once credit is decided.
                let creditTile = HomeFormat.creditTile(final: rollup?.creditFinal, pending: rollup?.creditPending,
                                                       accepted: rollup?.allTime?.accepted,
                                                       credit: credit)
                HomeStatTile(label: HomeFormat.creditPendingWord, value: creditTile.value,
                             detail: creditTile.detail)
                HomeStatTile(label: MonitorWords.contributed, value: HomeFormat.count(rollup?.allTime?.accepted))
                HomeStatTile(label: MonitorWords.held, value: HomeFormat.count(rollup?.quarantined))
            }
            .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The list's rules, in Ron's terms (`history-view-model.ts`): the filter is
/// an exact status match counted over every row read, and its labels are
/// the core's (`historyUi`; "all" from the public-run copy).
enum HistoryList {
    enum Filter: String, CaseIterable, Hashable {
        case all
        case accepted
        case submitted
        case quarantined
        case withdrawn
    }

    /// Where the opened row's details are drawn, for scrolling to them.
    static let detailAnchor = "history.detail"

    static func counts(_ rows: [DaemonData.HistoryRow]) -> [Filter: Int] {
        var counts: [Filter: Int] = [.all: rows.count]
        for row in rows {
            if let status = row.status, let filter = Filter(rawValue: status), filter != .all {
                counts[filter, default: 0] += 1
            }
        }
        return counts
    }

    static func rows(_ rows: [DaemonData.HistoryRow], filter: Filter) -> [DaemonData.HistoryRow] {
        filter == .all ? rows : rows.filter { $0.status == filter.rawValue }
    }

    /// A filter's label from the core (Ron's #1146 `history-filter.tsx`
    /// words, `MonitorShellCopy`), or nil when the core has not said.
    static func label(_ filter: Filter, shell: MonitorShellCopy?) -> String? {
        let label: String? = switch filter {
        case .all: shell?.filterAll
        case .accepted: shell?.filterAccepted
        case .submitted: shell?.filterSubmitted
        case .quarantined: shell?.filterQuarantined
        case .withdrawn: shell?.filterWithdrawn
        }
        return label.flatMap { $0.isEmpty ? nil : $0 }
    }

    /// Every filter's label, or nil if any is missing.
    static func labels(shell: MonitorShellCopy?) -> [Filter: String]? {
        var labels: [Filter: String] = [:]
        for filter in Filter.allCases {
            guard let label = label(filter, shell: shell) else { return nil }
            labels[filter] = label
        }
        return labels
    }

    /// What a row offers in Withdraw's place.
    enum RowWithdraw: Equatable {
        /// Nothing: no record, a status the owner's rule does not withdraw,
        /// or a withdrawal already done.
        case none
        /// The account session is being read for the first time.
        case checkingAccount
        /// No active account session: sign in first (Ron's `HistoryRow`).
        case signIn
        /// Withdraw, which asks first.
        case withdraw
        /// Withdraw again after a withdrawal that did not finish.
        case retry
    }

    /// Whether a row draws its last withdrawal's outcome. A refusal for
    /// want of an account session is never drawn: the row offers sign-in,
    /// or Withdraw once signed in, in its place, and the legacy sentence
    /// says this build has no sign-in. Every other outcome is drawn.
    static func showsOutcome(_ result: AppModel.WithdrawalResult) -> Bool {
        result != .noAccountSession
    }

    /// Withdraw on a row stands on the app's own record and the owner's
    /// rule, asked of the status the session detail decides from (its
    /// read status, else the record's: `SessionDetailView.withdrawalStatus`).
    /// No record, no Withdraw. It offers Retry after a failed attempt, as
    /// the detail's action did, and needs an active account session: until
    /// one is known, the row offers sign-in instead.
    static func rowWithdraw(
        record: HistoryRecord?, detail: SessionDetail?, result: AppModel.WithdrawalResult?,
        account: AppModel.AccountSessionRead
    ) -> RowWithdraw {
        guard let record,
              ContributionStatusPresentation.offersWithdraw(SessionDetailView.withdrawalStatus(record, detail: detail))
        else { return .none }
        if case .withdrawn = result { return .none }
        switch account {
        case .signedIn: return result == nil ? .withdraw : .retry
        case .checking: return .checkingAccount
        case .unread, .signedOut: return .signIn
        }
    }
}

/// Community standing (Ron's `CommunityPanel`), when the rollup carries
/// one; nothing otherwise: its eyebrow and heading, then rank, novelty
/// credit, accepted in the commons' window and the accept rate, and the
/// commons' own word when aggregate analytics are withheld.
struct HistoryCommunityCard: View {
    let store: HomeStore

    var body: some View {
        // Nothing after a failed read: not the earlier standing.
        if let community = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])?.community {
            if let words = MonitorWords.table?.homeHistory {
                GlassCard(quiet: true) {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                            Text(MonitorWords.community)
                                .glassType(GlassTokens.TypeScale.eyebrow)
                                .foregroundStyle(GlassColor.textTertiary)
                            Text(words.publicStanding)
                                .glassType(GlassTokens.TypeScale.title)
                                .foregroundStyle(GlassColor.textPrimary)
                                .accessibilityAddTraits(.isHeader)
                        }
                        HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                            cell(MonitorWords.rank, community.rank.map { "#\($0)" } ?? "—")
                            cell(words.noveltyCredit, HomeFormat.points(community.noveltyCredit))
                            cell(words.accepted(window: community.windowLabel ?? "—"),
                                 HomeFormat.count(community.acceptedInWindow))
                            cell(words.acceptRate, HomeFormat.rate(community.acceptRate))
                        }
                        .padding(.vertical, GlassTokens.Space.s4)
                        .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                        .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
                        if community.analyticsWithheld == true {
                            Text(words.analyticsWithheld)
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                                .foregroundStyle(GlassColor.textPrimary)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                    }
                }
            } else {
                GlassEyebrowCard(MonitorWords.community) {
                    GlassKeyValueList([
                        .init(MonitorWords.rank, community.rank.map { "#\($0)" } ?? "—"),
                        .init(MonitorWords.window, community.windowLabel ?? "—"),
                    ])
                }
            }
        }
    }

    private func cell(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(label)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
                .fixedSize(horizontal: false, vertical: true)
            Text(value)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .monospacedDigit()
                .lineLimit(1)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .accessibilityElement(children: .combine)
    }
}

/// The credit record (Ron's `CreditRecordPanel`): what credit is, then the
/// final figure and the one still being scored, or his chip before the
/// first sync. Pending credit is a figure only when the commons has stated
/// its condition (D6); without that statement it is a dash: a bare number
/// would read as owed. The statement itself is not drawn (owner, 2026-10-13).
struct HistoryCreditCard: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash, never the earlier figures or sentence.
        let credit = SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        let words = MonitorWords.table?.homeHistory
        GlassEyebrowCard(words?.creditRecord ?? MonitorWords.credit) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s5) {
                if let words {
                    Text(words.aboutCredit)
                        .glassType(GlassTokens.TypeScale.title)
                        .foregroundStyle(GlassColor.textPrimary)
                        .accessibilityAddTraits(.isHeader)
                    Text(words.aboutCreditBody)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let words, let rollup, rollup.lastRefreshedAt == nil {
                    GlassChip(words.notSynced)
                } else {
                    HStack(alignment: .top, spacing: GlassTokens.Space.s10) {
                        figure(MonitorWords.final, HomeFormat.points(rollup?.creditFinal))
                        figure(words?.stillBeingScored ?? MonitorWords.pending,
                               HomeFormat.pendingFigure(rollup?.creditPending, credit: credit))
                    }
                    .padding(.top, GlassTokens.Space.s7)
                    .overlay(alignment: .top) { GlassHairline(GlassColor.hairline) }
                }
            }
        }
    }

    private func figure(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            Text(label)
                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                .foregroundStyle(GlassColor.textSecondary)
            Text(value)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .monospacedDigit()
        }
        .accessibilityElement(children: .combine)
    }
}

/// The record by period: the week's and month's accepted counts and the
/// withdrawn count, from the rollup. #1146's History has no place for it,
/// so it closes the page, quietly, rather than sitting among Ron's stat
/// cards.
private struct HistoryPeriodCard: View {
    let store: HomeStore

    var body: some View {
        // A failed read is a dash in every row, never the earlier counts.
        let rollup = SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
        GlassEyebrowCard(MonitorWords.summary) {
            GlassKeyValueList([
                .init(MonitorWords.week, HomeFormat.count(rollup?.week?.accepted)),
                .init(MonitorWords.month, HomeFormat.count(rollup?.month?.accepted)),
                .init(MonitorWords.withdrawn, HomeFormat.count(rollup?.takenBack)),
            ])
        }
    }
}

/// Home's Account tiles: Credit, the total with its change since the last
/// day this Mac saw, and the NEAR AI balance beside it (owner, 2026-10-19),
/// each opening Private AI. Home draws them under Account, and the edge
/// rail's Private AI peek under the tool buttons (owner, 2026-10-22). A
/// failed rollup or credit read is a dash, never the store's earlier value.
struct HomeAccountTiles: View {
    let store: HomeStore
    let openPrivateAI: () -> Void
    @EnvironmentObject private var model: AppModel

    private var freshRollup: DaemonData.HistoryRollup? {
        SummaryFacts.fresh(store.rollup, unless: store.failures["history_rollup"])
    }

    private var freshCredit: DaemonData.CommonsCreditSummary? {
        SummaryFacts.fresh(store.credit, unless: store.failures["commons_credit_summary"])
    }

    var body: some View {
        let signIn = HomeFormat.creditSignIn(freshCredit, line: HomeFormat.creditSignInLine)
        Grid(horizontalSpacing: GlassTokens.Space.s3, verticalSpacing: GlassTokens.Space.s3) {
            GridRow {
                // Received and pending side by side (owner, 2026-10-29), both
                // History's figures (owner, 2026-10-28: Home said 0 while
                // History said 50.3): received is its Final, pending its
                // pending figure under D6.
                let tiles = HomeFormat.creditTiles(final: freshRollup?.creditFinal,
                                                   pending: freshRollup?.creditPending,
                                                   credit: freshCredit)
                HomeStatTile(label: HomeFormat.creditReceivedWord, value: tiles.received,
                             detail: signIn, systemImage: HomeStatIcons.credit, action: openPrivateAI)
                HomeStatTile(label: HomeFormat.creditPendingTitleWord, value: tiles.pending,
                             action: openPrivateAI)
                if let copy = model.privateInferenceCopy {
                    let balance = HomeFormat.balance(model.balanceStatus, copy: copy, calls: model.balanceCalls)
                    BalanceArrivalFlourish(status: model.balanceStatus, calls: model.balanceCalls) { counting in
                        HomeStatTile(label: copy.balanceTitle, value: counting ?? balance.value,
                                     detail: balance.detail, detailInk: balance.detailInk,
                                     systemImage: HomeStatIcons.balance, valueAccessory: balance.beside,
                                     action: openPrivateAI)
                    }
                } else {
                    Color.clear
                }
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        // A sign-in landed: read the Credit tile again.
        .onChange(of: model.accountSessionGeneration) {
            Task { await store.load() }
        }
    }
}

/// One of Home's and History's three counts (Ron's `StatCard`): a full
/// card, 10 by 12 inside, its eyebrow word over an 18pt-class bold tabular
/// figure. The word wraps at a space, never inside a word.
/// The Traces hero's three steps (owner, 2026-10-29): side by side, an
/// arrow between each, each a translucent panel on the hero's ground with
/// a numbered disc and an icon on one row, then a title and one line
/// across the panel's width. All in heroInk, as
/// every word on a hero is; the discs and icons are heroAccent, the
/// owner's mock-up's blue. The panels are as tall as the tallest.
/// Contribute opens Traces and Use Private AI opens Private AI, each panel
/// a button of its own; Earn credit stays on Home, a plain panel (owner,
/// 2026-10-29). The hero itself is a panel, so these clicks are not
/// swallowed by it; its Start earning pill is its own button. Part of the hero's one button, so VoiceOver reads
/// the steps with the card.
struct HomeHeroSteps: View {
    struct Step: Identifiable {
        let number: Int
        let systemImage: String
        let title: String
        let detail: String
        /// Opens the step's tab; nil draws a plain panel.
        var action: (() -> Void)? = nil
        var id: Int { number }
    }

    let steps: [Step]

    static var disc: CGFloat { 30 }
    static var icon: CGFloat { 28 }

    var body: some View {
        let ink = GlassTokens.Color.heroInk.color
        HStack(alignment: .center, spacing: GlassTokens.Space.s2) {
            ForEach(steps) { step in
                if step.number > 1 {
                    Image(systemName: "arrow.right")
                        .glassGlyph(14, weight: .semibold)
                        .foregroundStyle(ink.opacity(0.6))
                        .accessibilityHidden(true)
                }
                HomeHeroStepPanel(step: step)
            }
        }
        .fixedSize(horizontal: false, vertical: true)
        .padding(.top, GlassTokens.Space.s2)
    }
}

/// One of the Traces hero's steps: a translucent panel, brighter under the
/// pointer when it opens a tab, and then a button that reads its title and
/// line to VoiceOver as one.
struct HomeHeroStepPanel: View {
    let step: HomeHeroSteps.Step
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        if let action = step.action {
            Button(action: action) { panel }
                .buttonStyle(GlassPressStyle())
                .onHover { inside in
                    withAnimation(GlassMotion.fast(reduceMotion)) { hovering = inside }
                }
                .accessibilityElement(children: .combine)
                .accessibilityAddTraits(.isButton)
        } else {
            panel.accessibilityElement(children: .combine)
        }
    }

    private var panel: some View {
        let ink = GlassTokens.Color.heroInk.color
        let shape = RoundedRectangle(cornerRadius: GlassTier.cardQuiet.radius, style: .continuous)
        // The disc and icon share the top row, so the title and line take
        // the panel's whole width: beside the disc they wrapped a word a
        // line and hyphenated "Contribute" in a narrow window.
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            HStack(alignment: .center, spacing: GlassTokens.Space.s2) {
                Text("\(step.number)")
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(ink)
                    .frame(width: HomeHeroSteps.disc, height: HomeHeroSteps.disc)
                    .background(Circle().fill(GlassTokens.Color.heroAccent.color))
                    .accessibilityHidden(true)
                Image(systemName: step.systemImage)
                    .glassGlyph(HomeHeroSteps.icon, weight: .regular)
                    .foregroundStyle(GlassTokens.Color.heroAccent.color)
                    // One height for every icon, so the titles line up.
                    .frame(height: HomeHeroSteps.icon + 6, alignment: .leading)
                    .accessibilityHidden(true)
            }
            .padding(.bottom, GlassTokens.Space.s1)
            Text(step.title)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(ink)
                .fixedSize(horizontal: false, vertical: true)
            Text(step.detail)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(ink.opacity(0.8))
                .fixedSize(horizontal: false, vertical: true)
        }
        .padding(GlassTokens.Space.s3)
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .topLeading)
        .background(shape.fill(ink.opacity(hovering ? 0.16 : 0.08)))
        .contentShape(shape)
        .overlay(shape.strokeBorder(ink.opacity(0.18), lineWidth: 1))
    }
}

/// The Traces hero's graphic (owner, 2026-10-29, from the owner's
/// mock-up): one faint ring past the card's trailing edge, behind the
/// steps, where a symbol would sit on the third step's words.
struct HomeHeroRing: View {
    static var side: CGFloat { 300 }
    static var stroke: CGFloat { 36 }

    var body: some View {
        Circle()
            .strokeBorder(GlassTokens.Color.heroInk.color.opacity(0.14), lineWidth: Self.stroke)
            .frame(width: Self.side, height: Self.side)
            .offset(x: 90, y: -30)
    }
}

private struct HomeStatTile: View {
    let label: String
    let value: String
    /// A line under the value (the credit's change, the uncontributed
    /// size and estimate), in `detailInk` when it carries a tone.
    var detail: String? = nil
    var detailInk: GlassRGBA? = nil
    /// The tile's icon (`HomeStatIcons`); nil draws none.
    var systemImage: String? = nil
    /// A quieter figure beside the value; nil draws none.
    var valueAccessory: String? = nil
    /// Opens where the number lives, and the whole tile is the button;
    /// nil for a tile on the page its number lives on (History's).
    var action: (() -> Void)?

    /// #1146's `StatCard`: a full card, the value bold and tabular. Its
    /// icon is the card's watermark on the right (owner, 2026-10-20), as
    /// every small card on Home draws it.
    var body: some View {
        if let action {
            Button(action: action) {
                GlassStatCard(label, value: value, detail: detail, detailInk: detailInk, systemImage: systemImage,
                              valueAccessory: valueAccessory, interactive: true, iconPlacement: .watermark)
            }
            .buttonStyle(GlassPressStyle())
        } else {
            GlassStatCard(label, value: value, detail: detail, detailInk: detailInk, systemImage: systemImage,
                          valueAccessory: valueAccessory, iconPlacement: .watermark)
        }
    }
}

/// What waits on this Mac, not yet contributed (owner, 2026-10-12): the
/// sessions in the queue that have not gone, their size, and the credit the
/// core estimates for them. Only estimates the core says to draw are added
/// up; with none, no estimate is shown, never zero.
///
/// Dollars replace points (owner, 2026-10-16): when the daemon reports the
/// machine-wide settlement estimate, its finished line is the detail, the
/// same line the menu bar draws, and the points sum is not shown.
struct HomeUncontributed: Equatable {
    let count: Int
    let bytes: Int64
    let low: Double?
    let high: Double?
    /// The core's finished dollar line, or nil when there is none to draw.
    let dollars: String?

    /// States that have left, or are leaving, this Mac.
    static let gone: Set<String> = ["uploading", "uploaded"]

    init(_ sessions: [DaemonData.QueueEntry], settlement: DaemonData.SettlementEstimate? = nil) {
        // The core's call to share (owner, 2026-10-20), else its plain line
        // from an older core.
        let line = settlement.map { $0.shareText ?? $0.text }
        let text = line?.trimmingCharacters(in: .whitespacesAndNewlines)
        dollars = (text?.isEmpty ?? true) ? nil : line
        let waiting = sessions.filter { !Self.gone.contains($0.state) }
        count = waiting.count
        bytes = waiting.reduce(Int64(0)) { $0 + Int64($1.sizeBytes ?? 0) }
        let drawn = waiting.compactMap(\.creditEstimate).filter { $0.drawn == true }
        low = drawn.isEmpty ? nil : drawn.reduce(0) { $0 + $1.low }
        high = drawn.isEmpty ? nil : drawn.reduce(0) { $0 + $1.high }
    }

    /// The size of what waits, drawn beside the count (owner, 2026-10-12).
    func size(_ words: MonitorHomeHistoryCopy?) -> String? {
        guard let words, count > 0 else { return nil }
        return FirstRunCopy.fill(words.uncontributedSize,
                                 ["size": ByteCountFormatter.string(fromByteCount: bytes, countStyle: .file)])
    }

    /// The estimate, under the count, when there is one: the dollar line,
    /// which covers every trace on this Mac and so needs no queued count,
    /// else the points sum.
    func detail(_ words: MonitorHomeHistoryCopy?) -> String? {
        if let dollars { return dollars }
        guard let words, count > 0, let low, let high else { return nil }
        return FirstRunCopy.fill(words.uncontributedEstimate,
                                 ["min": HomeFormat.points(low), "max": HomeFormat.points(high)])
    }
}

/// Formatting for Home and History. Pure, so the rules are tested.
enum HomeFormat {
    /// The Balance tile's figure (owner, 2026-10-19): the amount left when
    /// the core reads one, else a dash with the core's sentence for why.
    /// Signed out, the core's short "Sign in required" sits beside the dash
    /// (`beside`); every other sentence is longer, some carry a recovery,
    /// and they go under it (`detail`). Never a figure the core did not give.
    ///
    /// With a figure, the core's line for the change over the last 24 hours
    /// goes under it (owner, 2026-10-23), up in the status green and down in
    /// the status red as the Credit tile's change is drawn. Missing history
    /// hides the change line; it is never represented as "$0.00".
    static func balance(
        _ status: BalanceStatus, copy: PrivateInferenceCopy, calls: BalanceCalls
    ) -> (value: String, beside: String?, detail: String?, detailInk: GlassRGBA?) {
        if let sentence = BalanceSurface.stateLine(status, copy: copy, calls: calls) {
            return status.state == signedOutBalance ? ("—", sentence, nil, nil) : ("—", nil, sentence, nil)
        }
        guard BalanceSurface.showsFigures(status, calls: calls) else { return ("—", nil, nil, nil) }
        switch BalanceSurface.remaining(status, copy: copy, calls: calls) {
        case .unavailable: return ("—", nil, nil, nil)
        case .figure(let amount):
            let change = balanceChange(status)
            return (amount, nil, change?.text, change?.ink)
        case .sentence(let sentence): return ("—", nil, sentence, nil)
        }
    }

    /// The Balance tile's 24-hour change: the core's words, inked by the
    /// sign of the core's figure. Zero stays neutral; unknown history draws no line.
    static func balanceChange(_ status: BalanceStatus) -> (text: String, ink: GlassRGBA?)? {
        guard let nanos = status.change24hNanos else { return nil }
        guard let text = status.change24hText, !text.isEmpty else { return nil }
        let direction = status.change24hDirection ?? (nanos == 0 ? 0 : (nanos > 0 ? 1 : -1))
        let ink: GlassRGBA? = direction == 0 ? nil
            : (direction > 0 ? GlassTokens.Color.statusOnText : GlassTokens.Color.statusOutsideText)
        return (text, ink)
    }

    /// The daemon's balance label with no NEAR AI sign-in kept here.
    static let signedOutBalance = "no_session"

    /// A card heading from the core's copy: the word, or nil when the copy
    /// has not arrived (or the word is empty), so no card is drawn on a
    /// word of this shell's own.
    static func cardHeading(_ word: String?) -> String? {
        guard let word, !word.isEmpty else { return nil }
        return word
    }

    /// A count, or a dash when the core did not say. Zero is a number.
    static func count(_ value: Int?) -> String {
        value.map(String.init) ?? "—"
    }

    /// The payout link's "session ended" line from the core (the same
    /// constant, not a string of this shell's); nil if the export did not
    /// arrive.
    static let creditSignInLine: String? = TCConsentCopy.payoutLinkCopyJSON()
        .flatMap(PayoutLinkCopy.decode(fromJSON:))?.sessionEnded

    /// The Credit tile's reason line: `line` when the commons' points are
    /// unknown because no account session could be used or renewed, else
    /// nil (the tile keeps its dash, as for every other unknown).
    static func creditSignIn(_ credit: DaemonData.CommonsCreditSummary?, line: String?) -> String? {
        guard credit?.accountSessionRequired == true, let line, !line.isEmpty else { return nil }
        return line
    }

    /// Home's Credit tile value: total credit, or a dash when unknown.
    static func creditTotal(_ value: Double?) -> String {
        guard let value else { return "—" }
        return value.formatted(.number.precision(.fractionLength(0...1)))
    }

    /// The Credit tile's change since the last day this Mac saw: signed, in
    /// the status green for a gain and red for a loss (owner, 2026-10-12);
    /// nil while there is no earlier day to compare with, and for no change,
    /// which the tile does not say (owner, 2026-10-16).
    static func creditDelta(_ delta: Double?, words: MonitorHomeHistoryCopy?) -> (text: String, ink: GlassRGBA?)? {
        guard let delta, let words, abs(delta) >= 0.05 else { return nil }
        let figure = delta.formatted(.number.precision(.fractionLength(0...1)).sign(strategy: .always()))
        let ink = delta > 0 ? GlassTokens.Color.statusOnText : GlassTokens.Color.statusOutsideText
        return (FirstRunCopy.fill(words.creditDelta, ["amount": figure]), ink)
    }

    static func points(_ value: Double?) -> String {
        value.map { $0.formatted(.number.precision(.fractionLength(1))) } ?? "—"
    }

    /// Home's pending-credit tile: the figure only with the commons'
    /// statement of what it waits on (D6), a dash otherwise.
    static func pendingFigure(_ value: Double?, condition: String?) -> String {
        condition == nil ? "—" : points(value)
    }

    /// The same rule from the commons' credit summary: a figure only when
    /// the commons has said what it waits on (D6); a dash otherwise, never
    /// a bare number that reads as owed.
    static func pendingFigure(_ pending: Double?, credit: DaemonData.CommonsCreditSummary?) -> String {
        pendingFigure(pending, condition: pendingCondition(credit))
    }

    /// The core's waiting-decision line. An unknown count is never
    /// presented as nothing waiting.
    static func waitingLine(_ owed: Int?) -> String? {
        guard let owed, let shell = MonitorWords.table?.shell else { return nil }
        return shell.waiting(owed)
    }

    /// History card's "X credit pending", under the same rule as the tile:
    /// only with the commons' statement of what it waits on (D6), and never
    /// for nothing pending.
    static func creditPendingAccessory(_ pending: Double?, condition: String?) -> String? {
        guard let pending, pending > 0, condition != nil,
              let template = MonitorWords.table?.shell.creditPendingAmount else { return nil }
        return template.replacingOccurrences(of: "{amount}", with: points(pending))
    }

    /// Home's Credit tile: History's Final figure once any credit is
    /// decided, with the pending accessory (or Final) under it; before
    /// that, the pending figure under D6, marked Pending.
    /// `accepted` is the rollup's accepted count: with traces accepted and
    /// nothing pending, every one is decided, so the tile reads Final even
    /// at 0 (traces scored with no award, #1703), never Pending.
    static func creditTile(final: Double?, pending: Double?, accepted: Int?,
                           credit: DaemonData.CommonsCreditSummary?) -> (value: String, detail: String) {
        let allDecided = (accepted ?? 0) > 0 && (pending ?? 0) == 0 && final != nil
        guard let final, final > 0 || allDecided else {
            return (pendingFigure(pending, credit: credit), MonitorWords.pending)
        }
        return (points(final),
                creditPendingAccessory(pending, condition: pendingCondition(credit)) ?? MonitorWords.final)
    }

    /// Home's pending-credit tile and status-card link, in the core's words.
    static var creditPendingWord: String { MonitorWords.table?.creditPending ?? "" }

    /// Home's received and pending tiles' labels; an older core's words
    /// ("Credit", "Pending") when it predates them.
    static var creditReceivedWord: String { MonitorWords.table?.creditReceived ?? creditPendingWord }
    static var creditPendingTitleWord: String { MonitorWords.table?.creditPendingTitle ?? MonitorWords.pending }

    /// Home's two credit tiles (owner, 2026-10-29: show both received and
    /// pending credit). Received is History's Final, 0 once History has
    /// answered with nothing decided; pending is its pending figure, only
    /// with the commons' statement of what it waits on (D6), a dash
    /// otherwise.
    static func creditTiles(final: Double?, pending: Double?, credit: DaemonData.CommonsCreditSummary?)
        -> (received: String, pending: String) {
        let received = final ?? (pending == nil ? nil : 0)
        return (points(received), pendingFigure(pending, credit: credit))
    }

    static var openTracesWord: String { MonitorWords.table?.openTraces ?? "" }

    /// What pending credit waits on, in the commons' own words: the only
    /// condition the shell may show it with. Nil when the commons has not
    /// said, and then pending credit is not shown as a figure.
    static func pendingCondition(_ credit: DaemonData.CommonsCreditSummary?) -> String? {
        guard let credit, credit.postureKnown,
              let explanation = credit.commonsSettlementExplanation, !explanation.isEmpty else { return nil }
        return explanation
    }

    /// Day and tool, then (in full) the size uploaded and when a
    /// withdrawal was seen. Month and day even when compact: a weekday
    /// alone reads a months-old row as this week. How it was approved is a
    /// line of its own.
    static func meta(_ row: DaemonData.HistoryRow, compact: Bool) -> String {
        var parts: [String] = []
        if let date = row.submittedAt { parts.append(day(date)) }
        if let source = row.source { parts.append(InferenceTabView.toolName(source)) }
        if !compact {
            if let bytes = row.uploadedBytes {
                parts.append(ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .memory))
            }
            if let withdrawn = row.revokedAt ?? row.withdrawnAt {
                parts.append("\(MonitorWords.withdrawn) \(day(withdrawn))")
            }
        }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
    }

    /// A History row's tool and day (Ron's `history-row.tsx`, source
    /// first). How it was approved, its size and its withdrawal are the
    /// opened row's (`HistoryDetailInspector`, `meta(_:compact: false)`).
    static func rowMeta(_ row: DaemonData.HistoryRow) -> String {
        var parts: [String] = []
        if let source = row.source { parts.append(InferenceTabView.toolName(source)) }
        if let date = row.submittedAt { parts.append(day(date)) }
        return parts.isEmpty ? "—" : parts.joined(separator: " · ")
    }

    /// The community's accept rate as a whole percentage, or a dash when
    /// the commons did not say.
    static func rate(_ value: Double?) -> String {
        value.map { $0.formatted(.percent.precision(.fractionLength(0))) } ?? "—"
    }

    static func day(_ date: Date) -> String {
        date.formatted(.dateTime.month(.abbreviated).day())
    }

    /// What Home's watching row says when the screen is ready.
    enum Watching: Equatable {
        /// The core says the contributor is not signed in.
        case signedOut
        /// The core reports a health label.
        case unhealthy(String)
        /// Watching this many tools, as the core counts them.
        case watching(Int)
    }

    /// The watching row's state (the stack-wide ScreenState): the core's
    /// line when it does not answer, a dash before the first read, when a
    /// read failed, or when the core did not say whether watching is
    /// paused, whether the contributor is signed in, or which tools it
    /// reads. A missing signal is never drawn as watching.
    @MainActor
    static func watchingState(_ store: HomeStore) -> ScreenState {
        let failure = store.failures["status"] ?? store.failures["tool_destinations"]
        return ScreenState.resolve(
            failure: failure, loaded: store.status != nil || failure != nil,
            paused: store.status?.paused,
            known: store.status?.loggedIn != nil && store.destinations != nil)
    }

    /// Signed out, then unhealthy, then watching N: the order
    /// `MenuBarStatus.state` uses. N counts the tools the core reads, unset
    /// ones included (`watchedCount`).
    static func watching(_ status: DaemonData.Status?, destinations: DaemonData.ToolDestinations?) -> Watching {
        if status?.loggedIn == false { return .signedOut }
        if let label = status?.health?.lastErrorLabel, !label.isEmpty { return .unhealthy(label) }
        return .watching(destinations?.watchedCount ?? 0)
    }

    /// History reads one page. When the page is full it says how many of
    /// the total it shows, in the core's words; the total is the rollup's
    /// all-time count, unknown when any part of it is.
    static func cap(_ shown: Int, rollup: DaemonData.HistoryRollup?) -> String? {
        guard shown >= HomeStore.historyLimit else { return nil }
        let all = rollup?.allTime
        let parts = [all?.submitted, all?.accepted, all?.quarantined, all?.withdrawn, all?.other]
        let total = parts.contains(nil) ? nil : parts.compactMap { $0 }.reduce(0, +)
        return MonitorWords.table?.historyCap(shown: shown, total: total)
    }

    /// How a contribution was approved. Not recorded is said as such, never
    /// as approved by the person.
    static func provenance(_ provenance: DaemonData.Provenance) -> String {
        switch provenance {
        case .armed: ProjectCopy.modeChoiceLabel(.autoUpload)
        case .personApproved: MonitorWords.approved
        case .notRecorded: MonitorWords.unrecorded
        }
    }

    /// The row's own credit: final when scored, zero included, otherwise
    /// nothing. A pending figure needs its condition, which lives in the
    /// summary, not per row.
    static func credit(_ row: DaemonData.HistoryRow) -> String? {
        row.creditPointsFinal.map(points)
    }

    /// Why a decided row earned no credit, in the core's words: only for a
    /// final figure of 0 the server gave a reason for. A pending row, a
    /// credited one, or no core copy reads nothing, and the row falls back
    /// to its figure.
    static func noCreditReason(_ row: DaemonData.HistoryRow, copy: PublicRunCopy?) -> String? {
        guard let copy, row.creditPointsFinal == 0,
              let reason = row.creditOutcomeReason, !reason.isEmpty else { return nil }
        return copy.creditOutcomeLabel(for: reason)
    }

    /// A row's status tag, from the core's one status table. A missing or
    /// empty status reads the core's unavailable word too. With no core copy
    /// decoded there is no tag at all -- never the raw wire token, which is
    /// not a word a contributor was meant to read.
    static func statusWord(_ status: String?, label: (String?) -> String?) -> String? {
        label(status.flatMap { $0.isEmpty ? nil : $0 })
    }

    /// The word for a status, from the core's one table (`PublicRunCopy`).
    /// A missing or empty status reads the core's unavailable word; with no
    /// core copy decoded there is no word at all, never the raw token.
    static func historyStatusLabel(copy: PublicRunCopy?, _ status: String?) -> String? {
        guard let copy else { return nil }
        guard let status, !status.isEmpty else { return copy.contributionStatusUnavailable }
        return copy.historyStatusLabel(for: status)
    }

    /// Accepted reads as done; held for review and submitted as waiting;
    /// withdrawn as neutral. Held is never drawn as rejected.
    static func tone(_ status: String?) -> GlassTag.Tone {
        switch status {
        case "accepted": .on
        case "submitted", "processing", "quarantined": .ask
        default: .neutral
        }
    }
}

/// Home's and History's words, from the core's table (`MonitorWords.table`).
extension MonitorWords {
    static var history: String { table?.history ?? "" }
    static var contributed: String { table?.contributed ?? "" }
    static var watching: String { table?.watching ?? "" }
    static var paused: String { table?.paused ?? "" }
    static var summary: String { table?.summary ?? "" }
    static var week: String { table?.week ?? "" }
    static var month: String { table?.month ?? "" }
    static var allTime: String { table?.total ?? "" }
    static var held: String { table?.held ?? "" }
    static var heldForReview: String { table?.heldForReview ?? "" }
    static var heldExplanation: String { table?.heldExplanation ?? "" }
    static var signedOut: String { table?.signedOut ?? "" }
    static var withdrawn: String { table?.withdrawn ?? "" }
    static var credit: String { table?.credit ?? "" }
    static var final: String { table?.creditFinal ?? "" }
    static var pending: String { table?.pending ?? "" }
    static var community: String { table?.community ?? "" }
    static var rank: String { table?.rank ?? "" }
    static var window: String { table?.window ?? "" }
    static var approved: String { table?.approved ?? "" }
    static var unrecorded: String { table?.unrecorded ?? "" }
}

extension HorizontalAlignment {
    private enum HomeStatusText: AlignmentID {
        static func defaultValue(in context: ViewDimensions) -> CGFloat { context[.leading] }
    }

    /// The left edge of the Home status card's words, after its dot.
    static let homeStatusText = HorizontalAlignment(HomeStatusText.self)
}
