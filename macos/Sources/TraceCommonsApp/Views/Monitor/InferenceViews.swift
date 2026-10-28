import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Inference tab (R8 of #1173): the model calls IronWire recorded,
/// newest first, with what each was priced at and its proof label.
///
/// An unreadable ledger is not an empty one: it never draws an empty
/// table or a zero. Priced is not billed: no figure here is money spent.
/// Only `verified` is drawn as proof.
///
/// The ledger needs the daemon, so the tab reads its startup first, as the
/// retired legacy destination did: the first run's Folders step when
/// folders are owed (it starts the daemon, takes no invite and offers no
/// Join), a spinner while starting, the core's down
/// title over the refusal's sentence.
struct InferenceTabView: View {
    let store: InferenceStore
    /// Opens the Private AI section of Settings.
    var onOpenSettings: () -> Void = {}
    @EnvironmentObject private var model: AppModel

    /// The core's analytics words (`tc_insights_copy_json`), for each
    /// call's ledger counters.
    private static let insightsCopy = TCInsights.copy() ?? [:]

    var body: some View {
        // The Tools step lays itself out, so the switch sits outside the
        // ledger's scroll; the refresh sits on the stack, which is always
        // drawn.
        VStack(alignment: .leading, spacing: 0) {
            switch model.startup {
            case .needsRoots:
                // The first run's Tools step on its own stage, scaled down to
                // fit the tab (`FirstRunStageFit`).
                OnboardingCoordinatorView(startAt: .tools, takesInvites: false, onComplete: {})
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
            case .starting:
                SettingsAwaiting().frame(maxWidth: .infinity)
            case .refused(let sentence):
                GlassHealthBanner(banner: .init(
                    title: TracesHealth.coreDownLine?.title ?? TracesHealth.unknownWord ?? "",
                    detail: sentence, tone: .outside))
            case .running:
                ledger
            }
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .top)
        .onAppear { model.refreshAll() }
        // The settings refresh re-reads the ledger feed's switch; this
        // re-reads the calls, so tokens the daemon stopped sending go too.
        // Keyed on the switch: Settings is its own window, so this tab can
        // stay in view while the feed is turned off there.
        .task(id: model.daemonSettings?.insightsLedgerFeed) { await store.appeared() }
        // Lazily, with the tab and never on the sign-in: whether to ask the
        // contributor to choose a spending organization.
        .task { model.refreshSpendOrganizations() }
    }

    private var ledger: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // The owner's order (2026-10-09, 2026-10-10, 2026-10-19),
                // with the inspector closed on this tab: the Private AI
                // step-by-step card leads until it is closed (product
                // owner, 2026-10-28), then the status (a static dot and the
                // core's sentence; the owner set aside a switch here on
                // 2026-10-19), then the managed cards, whose tool buttons
                // are how Private AI runs, then the calls, then the NEAR AI
                // balance with billing under it (product owner, 2026-10-28:
                // further down), and last the card that opens Settings,
                // where the switch, the tools and the connection are. The offer and the NEAR AI
                // sign-in are popover notices over the tab
                // (`MonitorNotices`).
                PrivateAIGuideCard()
                PrivateAIInspectorView(store: store)
                // Only with several NEAR AI organizations and no choice made
                // (owner request, 2026-10-23); otherwise draws nothing.
                SpendOrganizationPrompt(onOpenSettings: onOpenSettings)
                ManagedSessionsSection()
                ledgerSections
                InferenceBalanceSection()
                InferenceAccountSection(store: store, onOpenSettings: onOpenSettings)
            }
        }
        .scrollIndicators(.never)
    }

    /// The window's totals, the models and the calls, as the core read them.
    @ViewBuilder
    private var ledgerSections: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // The stack-wide rule (ScreenState): a core that is down or a
                // failed read is said in the core's words over the last page,
                // never as the error's fixed label and never as current.
                if let failure = store.failures["inference_calls"] {
                    GlassNotice(tone: .outside, title: MonitorWords.table?.line(for: failure) ?? "") { EmptyView() }
                }
                if let network = store.networkSummary {
                    networkTotals(network)
                }
                if let page = store.calls {
                    if page.readable {
                        if store.networkSummary == nil {
                            totals(page, summary: store.summary, destinations: store.destinations)
                        }
                        if let summary = store.summary, summary.readable, !summary.models.isEmpty {
                            models(summary)
                        }
                        // An empty ledger is said by the totals' zero; the
                        // card is drawn only for calls to list (owner,
                        // 2026-10-10: no card for one figure).
                        if !page.calls.isEmpty { calls(page) }
                    }
                    // An unreadable ledger draws nothing here: its lone
                    // Calls dash is gone from under the summary (owner,
                    // 2026-10-12).
                } else if store.failures["inference_calls"] == nil {
                    GlassSpinner(standalone: true).frame(maxWidth: .infinity)
                }
        }
    }

    // MARK: Totals

    private func totals(
        _ page: DaemonData.InferenceCallPage, summary: DaemonData.InferenceSummary?,
        destinations: DaemonData.ToolDestinations?
    ) -> some View {
        let pageTotals = Self.totals(page, summary: summary, destinations: destinations)
        let totals = (calls: Optional<Int>.none, verified: Optional<Int>.none, priced: pageTotals.priced)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            // What window the counts cover, as the core reported it.
            Text(Self.windowLine(page, destinations: destinations))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MonitorWords.calls, value: totals.calls.map(String.init) ?? "—", status: .shared)
                GlassLegendCell(InferenceWords.proof(.verified), value: totals.verified.map(String.init) ?? "—", status: .on)
                GlassLegendCell(MonitorWords.priced, value: totals.priced, status: .kept)
            }
        }
    }

    private func networkTotals(_ network: DaemonData.NetworkInferenceSummary) -> some View {
        let totals = Self.networkHeaderTotals(network)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(MonitorWords.table?.windowLine(hours: network.windowHours) ?? "—")
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textTertiary)
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(MonitorWords.calls, value: totals.calls.map(String.init) ?? "—", status: .shared)
                GlassLegendCell(InferenceWords.proof(.verified), value: totals.verified.map(String.init) ?? "—", status: .on)
                GlassLegendCell(MonitorWords.priced, value: totals.priced, status: .kept)
            }
        }
    }

    /// Every figure in the header covers the same shared and managed window.
    /// A partial registry price cannot stand for the cost of all displayed calls.
    static func networkHeaderTotals(
        _ network: DaemonData.NetworkInferenceSummary?
    ) -> (calls: Int?, verified: Int?, priced: String) {
        let totals = HomePrivateAI.networkTotals(network)
        guard totals.calls != nil, let network, network.readable,
              let summary = network.summary, summary.enabled else {
            return (totals.calls, totals.verified, "—")
        }
        var usd = 0.0
        for route in [summary.routed, summary.outside, summary.unknown] {
            guard let pricedCalls = route.pricedCalls, pricedCalls == route.calls,
                  let cost = route.costUSD, cost.isFinite, cost >= 0 else {
                return (totals.calls, totals.verified, "—")
            }
            usd += cost
            guard usd.isFinite else { return (totals.calls, totals.verified, "—") }
        }
        return (totals.calls, totals.verified, formattedUSD(usd))
    }

    /// The hours the tab's counts cover: the calls page's `window_hours`,
    /// else `tool_destinations`' (K14 keeps them the same window). Nil when
    /// neither reported one.
    static func windowHours(_ page: DaemonData.InferenceCallPage, destinations: DaemonData.ToolDestinations?) -> Int? {
        page.windowHours ?? destinations?.windowHours
    }

    /// The window line, in the core's words, or a dash when no window was
    /// reported (or the core's words have not loaded).
    static func windowLine(_ page: DaemonData.InferenceCallPage, destinations: DaemonData.ToolDestinations?) -> String {
        MonitorWords.table?.windowLine(hours: windowHours(page, destinations: destinations)) ?? "—"
    }

    /// The tab's totals. The call count is the core's: `tool_destinations`'
    /// per-tool counts plus its unattributed calls (K14), over the same
    /// window as the calls page. Otherwise, and for verified and priced, the
    /// core's per-model summary when it answered. Otherwise the page's own
    /// calls, but only when the page is the whole window (no next cursor):
    /// one page of a longer ledger is never shown as the total. Unknown is a
    /// dash.
    static func totals(
        _ page: DaemonData.InferenceCallPage, summary: DaemonData.InferenceSummary?,
        destinations: DaemonData.ToolDestinations? = nil
    ) -> (calls: Int?, verified: Int?, priced: String) {
        let fallback = pageTotals(page, summary: summary)
        guard let destinations else { return fallback }
        return (callCount(destinations), fallback.verified, fallback.priced)
    }

    /// The core's call count for the window: every tool's
    /// `counts.inference_calls` plus `unattributed_calls`. Unknown when any
    /// part is.
    static func callCount(_ destinations: DaemonData.ToolDestinations) -> Int? {
        let parts = destinations.tools.map { $0.counts?.inferenceCalls } + [destinations.unattributedCalls]
        return parts.contains(nil) ? nil : parts.compactMap { $0 }.reduce(0, +)
    }

    private static func pageTotals(
        _ page: DaemonData.InferenceCallPage, summary: DaemonData.InferenceSummary?
    ) -> (calls: Int?, verified: Int?, priced: String) {
        if let summary, summary.readable {
            let calls = summary.models.map(\.calls)
            let callTotal = calls.contains(nil) ? nil : calls.compactMap { $0 }.reduce(0, +)
            let counts = summary.models.map(\.proofCounts)
            let verified = counts.contains(nil) ? nil
                : counts.compactMap { $0 }.reduce(0) { $0 + ($1[DaemonData.ProofLabel.verified.rawValue] ?? 0) }
            let micros = summary.models.map(\.pricedMicros)
            let priced = micros.contains(nil) ? "—" : money(micros.compactMap { $0 }.reduce(0, +))
            return (callTotal, verified, priced)
        }
        guard page.nextCursor == nil else { return (nil, nil, "—") }
        return (page.calls.count, page.calls.filter { $0.proofLabel.isProof }.count, priced(page.calls.map(\.cost)))
    }

    // MARK: Per model (PROVISIONAL summary)

    private func models(_ summary: DaemonData.InferenceSummary) -> some View {
        GlassEyebrowCard(MonitorWords.models) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                ForEach(summary.models) { model in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        HStack {
                            Text(model.model)
                                .glassType(GlassTokens.TypeScale.mono)
                                .foregroundStyle(GlassColor.textPrimary)
                                .lineLimit(1)
                                .truncationMode(.middle)
                            Spacer(minLength: GlassTokens.Space.s4)
                            Text(model.calls.map(String.init) ?? "—")
                                .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                                .foregroundStyle(GlassColor.textSecondary)
                        }
                        HStack(spacing: GlassTokens.Space.s2) {
                            ForEach(Self.sortedProofs(model.proofCounts), id: \.label) { item in
                                GlassTag("\(InferenceWords.proof(item.label)) \(item.count)", tone: Self.tone(item.label))
                            }
                            Spacer(minLength: 0)
                            Text(model.pricedMicros.map(Self.money) ?? "—")
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textTertiary)
                        }
                    }
                }
            }
        }
    }

    // MARK: Calls

    private func calls(_ page: DaemonData.InferenceCallPage) -> some View {
        GlassEyebrowCard(MonitorWords.calls) {
            LazyVStack(spacing: 0) {
                ForEach(Array(page.calls.enumerated()), id: \.element.id) { index, call in
                    #if DEBUG
                    let _ = OtherCollectionRowBodies.record("supplementalCalls", String(call.id))
                    #endif
                    GlassTableRow(first: index == 0) { callRow(call) }
                }
            }
        }
    }

    private func callRow(_ call: DaemonData.InferenceCall) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
            Text(call.at.formatted(.dateTime.hour().minute()))
                .glassType(GlassTokens.TypeScale.caption.monospaced)
                .foregroundStyle(GlassColor.textTertiary)
                .lineLimit(1)
                .fixedSize()
                .frame(minWidth: 56, alignment: .leading)
            VStack(alignment: .leading, spacing: 1) {
                Text(Self.toolName(call.tool))
                    .glassType(GlassTokens.TypeScale.label.weight(.semibold))
                    .foregroundStyle(GlassColor.textPrimary)
                Text(call.model)
                    .glassType(GlassTokens.TypeScale.caption.monospaced)
                    .foregroundStyle(GlassColor.textTertiary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                // The ledger's own counters, only while the feed is on: each
                // as the proxy reported it, never added together.
                if let tokens = call.tokens {
                    Text(InferenceTokenWords.line(tokens, copy: Self.insightsCopy))
                        .glassType(GlassTokens.TypeScale.caption.monospaced)
                        .foregroundStyle(GlassColor.textTertiary)
                        .lineLimit(1)
                        .truncationMode(.tail)
                        // Read out in place of the line, so the combined row
                        // says unknown where the line draws the dash.
                        .accessibilityLabel(InferenceTokenWords.accessibilityLine(tokens, copy: Self.insightsCopy))
                }
            }
            Spacer(minLength: GlassTokens.Space.s4)
            Text(Self.priced([call.cost]))
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
            GlassTag(InferenceWords.proof(call.proof), tone: Self.tone(call.proof))
        }
        .accessibilityElement(children: .combine)
    }

    // MARK: Formatting

    /// The tool's name when the shell knows the adapter, the raw id when
    /// it does not, and a dash for a call IronWire could not attribute.
    nonisolated static func toolName(_ id: String) -> String {
        if let kind = SourceKind(rawValue: id) { return kind.displayName }
        return id == "unknown" ? "—" : id
    }

    /// The sum of what the calls were priced at. Unknown when any one is
    /// unknown: a partial sum would read as the whole.
    static func priced(_ costs: [DaemonData.PricedCost?]) -> String {
        var total: Int64 = 0
        for cost in costs {
            guard let cost, cost.known, let micros = cost.pricedMicros else { return "—" }
            total += micros
        }
        return money(total)
    }

    static func money(_ micros: Int64) -> String {
        formattedUSD(Double(micros) / 1_000_000)
    }

    private static func formattedUSD(_ usd: Double) -> String {
        usd.formatted(.currency(code: "USD").precision(.fractionLength(2 ... 4)))
    }

    static func tone(_ label: DaemonData.ProofLabel) -> GlassTag.Tone {
        switch label {
        case .verified: .on
        case .pending, .gatewayOnly: .ask
        // A failed check is not the same as a call that left for an
        // outside model: its own tone.
        case .failed: .failed
        case .outside: .outside
        case .unattested, .unavailable, .unrecorded: .neutral
        }
    }

    /// A label this shell does not know is neutral, never a verdict.
    static func tone(_ raw: String) -> GlassTag.Tone {
        DaemonData.ProofLabel(rawValue: raw).map(tone) ?? .neutral
    }

    /// Proof counts in a fixed order, so the tags do not shuffle on reload.
    static func sortedProofs(_ counts: [String: Int]?) -> [(label: String, count: Int)] {
        let order = DaemonData.ProofLabel.allCases.map(\.rawValue)
        return (counts ?? [:])
            .sorted { (order.firstIndex(of: $0.key) ?? .max) < (order.firstIndex(of: $1.key) ?? .max) }
            .map { (label: $0.key, count: $0.value) }
    }
}

/// The Private AI summary at the top of the Private AI tab's main pane
/// (owner, 2026-10-09; it was the tab's inspector, as #1146's
/// `inference-inspector.tsx` draws it, and the inspector now stays closed
/// there). No title of its own: the tab names it (owner, 2026-10-10).
/// The Status row, then the core's subtitle (owner, 2026-10-27; the main
/// pane still has none). The row is the listener's state in the core's
/// sentence (never the switch: what was asked for is not what happened). The credential and
/// Connected tools rows and the connected / not connected pair are gone
/// (owner, 2026-10-12): sign-in and the tools are in Settings. The balance
/// card is the main pane's, further down.
/// How Private AI works, step by step (product owner, 2026-10-28): first on
/// the tab, in the managed copy's words, until it is closed with its X.
/// Closed, it stays closed on this Mac.
struct PrivateAIGuideCard: View {
    static let closedKey = "privateAI.guideClosed"
    @AppStorage(Self.closedKey) private var closed = false
    @EnvironmentObject private var model: AppModel

    static let stepKeys = ["guide_step_earn", "guide_step_start", "guide_step_use"]

    var body: some View {
        if !closed {
            GlassHeroCard(
                gradient: GlassTokens.Gradient.heroCampaign,
                title: model.managedText("guide_title"), subtitle: model.managedText("guide_body"),
                action: {},
                closeLabel: MonitorWords.close, onClose: { closed = true }
            ) {
                GlassHeroSymbol("lock.shield")
            } content: {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    ForEach(Array(Self.stepKeys.enumerated()), id: \.offset) { index, key in
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s2) {
                            Text(verbatim: "\(index + 1).")
                                .glassType(GlassTokens.TypeScale.bodyStrong)
                            Text(model.managedText(key))
                                .glassType(GlassTokens.TypeScale.body)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        .foregroundStyle(GlassTokens.Color.heroInk.color)
                    }
                }
            }
        }
    }
}

struct PrivateAIInspectorView: View {
    let store: InferenceStore
    @EnvironmentObject private var model: AppModel

    /// The one tool list (`AppModel.harnesses`), while it is current: this window's client has read
    /// the tools (`read`, which `InferenceStore.attach` forgets on a new
    /// client) and its last read did not fail. Otherwise `.none`, so a new
    /// client or a core that stopped answering is unknown, never the last
    /// list. Only whether `read` is there is asked, never its rows.
    static func liveHarnesses(_ list: HarnessList, read: HarnessList?, failure: DaemonDataError?) -> HarnessList {
        read == nil || failure != nil ? .none : list
    }

    var body: some View {
        if let copy = runningCopy {
            summary(copy)
        }
    }

    /// The core's words, only while the daemon runs: every row reads it.
    private var runningCopy: PrivateInferenceCopy? {
        guard case .running = model.startup else { return nil }
        return model.privateInferenceCopy
    }

    private func summary(_ copy: PrivateInferenceCopy) -> some View {
        let state = PrivateAISwitchCard.stateLabel(
            state: InferenceAccountSection.surfaceState(store.privateAI?.state), copy: copy,
            calls: model.privateInferenceCalls)
        // The tab's heading row, in the one style every tab's heading
        // takes (owner, 2026-10-19, `GlassTabHeading`), with the state on
        // the right. #1146: the Status dot is on while working, outside
        // otherwise, always beside the core's sentence.
        // The state right after the title (owner, 2026-10-20), not at the
        // row's far end.
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            GlassTabHeading(copy.inspectorStatus) {
                GlassStatusLabel(state.line, status: Self.statusDot(store.privateAI?.state, calls: model.privateInferenceCalls))
                    .multilineTextAlignment(.leading)
                    .fixedSize(horizontal: false, vertical: true)
            }
            .accessibilityElement(children: .combine)
            // What the destination is for, in the core's words, at the
            // title size (product owner, 2026-10-28: make it bigger).
            Text(copy.subtitle)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// The Status row's dot (#1146 `inference-inspector.tsx`): on while the
    /// listener is working, outside otherwise.
    static func statusDot(_ state: DaemonData.PrivateInferenceState?, calls: PrivateInferenceCalls) -> GlassStatus {
        PrivateInferenceIndicator.dotStatus(
            PrivateInferenceSurface.tone(InferenceAccountSection.surfaceState(state), calls: calls))
    }
}

/// A call's four ledger counters, each with the core's label, in a fixed
/// order: input, cache read, cache write, output. Never added together
/// (on the OpenAI family, input already holds the cache reads). A counter
/// the proxy did not report is the dash; a measured zero stays zero.
enum InferenceTokenWords {
    private static let labels = [
        "metric_input_tokens", "analytics_series_cache_read", "analytics_series_cache_write",
        "metric_output_tokens",
    ]

    private static func counters(_ tokens: DaemonData.InferenceCallTokens) -> [UInt32?] {
        [tokens.input, tokens.cacheRead, tokens.cacheWrite, tokens.output]
    }

    static func line(_ tokens: DaemonData.InferenceCallTokens, copy: [String: String]) -> String {
        pairs(tokens, copy: copy) { InsightsOverviewWords.figure(nil, copy: copy) }
    }

    /// The same pairs read out, with unknown where the line draws the dash.
    static func accessibilityLine(_ tokens: DaemonData.InferenceCallTokens, copy: [String: String]) -> String {
        pairs(tokens, copy: copy) { MonitorWords.unknown }
    }

    private static func pairs(
        _ tokens: DaemonData.InferenceCallTokens, copy: [String: String], unknown: () -> String
    ) -> String {
        zip(labels, counters(tokens)).map { key, value in
            let figure = value.map { InsightsOverviewWords.figure(UInt64($0), copy: copy) } ?? unknown()
            return "\(InsightsOverviewWords.text(key, copy)) \(figure)"
        }
        .joined(separator: " · ")
    }
}

/// The single words the map and the Inference tab show, beside the names
/// the core reports and the core's sentences (`ShellWordingTests`).
enum InferenceWords {
    /// IronWire's proof label, as one word from the core. Only `verified` is
    /// proof.
    static func proof(_ label: DaemonData.ProofLabel) -> String {
        guard let words = MonitorWords.table else { return "" }
        switch label {
        case .verified: return words.proofVerified
        case .gatewayOnly: return words.proofGatewayOnly
        case .unattested: return words.proofUnattested
        case .pending: return words.proofPending
        case .unavailable: return words.proofUnavailable
        case .failed: return words.proofFailed
        case .outside: return words.proofOutside
        case .unrecorded: return words.proofUnrecorded
        }
    }

    /// A label from a newer daemon that this shell does not know is a
    /// dash, never "Unrecorded" or any other verdict.
    static func proof(_ raw: String) -> String {
        DaemonData.ProofLabel(rawValue: raw).map(proof) ?? "—"
    }
}
