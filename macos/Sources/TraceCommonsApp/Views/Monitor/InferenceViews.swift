import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Inference tab (R8 of #1173): the model calls IronWire recorded,
/// newest first, with what each was priced at and its proof label.
///
/// An unreadable ledger is not an empty one: it draws a dash and the
/// reason's fixed label, never an empty table. Priced is not billed: no
/// figure here is money spent. Only `verified` is drawn as proof.
///
/// The ledger needs the daemon, so the tab reads its startup first, as the
/// retired legacy destination did: the first run's Folders step when
/// folders are owed (it starts the daemon, takes no invite and offers no
/// Join), a spinner while starting, the core's down
/// title over the refusal's sentence.
struct InferenceTabView: View {
    let store: InferenceStore
    /// For the prompts (offers, undos, the first-contribution note), which
    /// head this page now that the inspector stays closed on it.
    let traces: TracesStore
    @EnvironmentObject private var model: AppModel

    /// The core's analytics words (`tc_insights_copy_json`), for each
    /// call's ledger counters.
    private static let insightsCopy = TCInsights.copy() ?? [:]

    var body: some View {
        // The Folders step scrolls itself, so the switch sits outside the
        // ledger's scroll; the refresh sits on the stack, which is always
        // drawn.
        VStack(alignment: .leading, spacing: 0) {
            switch model.startup {
            case .needsRoots:
                // Ron's 450pt first-run pane, centred, not the window's width.
                OnboardingCoordinatorView(startAt: .folders, takesInvites: false, onComplete: {})
                    .frame(width: FirstRunProgress.paneWidth)
                    .frame(maxWidth: .infinity)
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
    }

    private var ledger: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // The owner's order (2026-10-09), with the inspector closed on
                // this tab: the prompts, then the Private AI summary the
                // inspector used to hold, then the calls, then the managed
                // cards, the tools, the switch, sign-in, balance and funding.
                InspectorPrompts(store: traces)
                PrivateAIInspectorView(
                    store: store, destinationLabel: model.privateInferenceCopy?.destination)
                ledgerSections
                InferenceAccountSection(store: store)
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
                if let page = store.calls {
                    if page.readable {
                        totals(page, summary: store.summary, destinations: store.destinations)
                        if let summary = store.summary, summary.readable, !summary.models.isEmpty {
                            models(summary)
                        }
                        calls(page)
                    } else {
                        unreadable
                    }
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
        let totals = Self.totals(page, summary: summary, destinations: destinations)
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
            if page.calls.isEmpty {
                // Readable and empty: the ledger answered, with nothing in
                // its window. A zero, which is not the same as unknown.
                Text("0")
                    .glassType(GlassTokens.TypeScale.number)
                    .foregroundStyle(GlassColor.textTertiary)
            } else {
                VStack(spacing: 0) {
                    ForEach(Array(page.calls.enumerated()), id: \.element.id) { index, call in
                        GlassTableRow(first: index == 0) { callRow(call) }
                    }
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

    private var unreadable: some View {
        GlassEyebrowCard(MonitorWords.calls) {
            Text("—")
                .glassType(GlassTokens.TypeScale.number)
                .foregroundStyle(GlassColor.textTertiary)
                .accessibilityLabel(MonitorWords.unknown)
        }
    }

    // MARK: Formatting

    /// The tool's name when the shell knows the adapter, the raw id when
    /// it does not, and a dash for a call IronWire could not attribute.
    static func toolName(_ id: String) -> String {
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
        (Double(micros) / 1_000_000).formatted(.currency(code: "USD").precision(.fractionLength(2 ... 4)))
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

/// The Private AI summary at the top of the Inference tab's main pane
/// (owner, 2026-10-09; it was the tab's inspector, as #1146's
/// `inference-inspector.tsx` draws it, and the inspector now stays closed
/// there). A 17pt bold title with "N of M tools connected" under it, the
/// connected / not connected legend pair, and three rows -- the listener's
/// state in the core's sentence (never the switch: what was asked for is
/// not what happened), the credential's state, and which tools are
/// connected. The balance card is the main pane's, further down.
struct PrivateAIInspectorView: View {
    let store: InferenceStore
    let destinationLabel: String?
    @EnvironmentObject private var model: AppModel

    /// The tools, from the one list the Local tools card and the Private AI
    /// map read too (`AppModel.harnesses`, as #1146 reads one
    /// `useHarnesses()`), so the pane, the inspector and the map never
    /// disagree. `.none` is a list not read: nil, never "no tools".
    static func rows(_ list: HarnessList) -> [HarnessRow]? {
        list == .none ? nil : list.harnesses
    }

    /// The one list, while it is current: this window's client has read
    /// the tools (`read`, which `InferenceStore.attach` forgets on a new
    /// client) and its last read did not fail. Otherwise `.none`, so a new
    /// client or a core that stopped answering is unknown, never the last
    /// list. Only whether `read` is there is asked, never its rows.
    static func liveHarnesses(_ list: HarnessList, read: HarnessList?, failure: DaemonDataError?) -> HarnessList {
        read == nil || failure != nil ? .none : list
    }

    /// `model.harnesses`, while this inspector's store says it is current.
    private var harnesses: HarnessList {
        Self.liveHarnesses(model.harnesses, read: store.harnesses, failure: store.failures["harness_list"])
    }

    var body: some View {
        let copy = runningCopy
        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            VStack(alignment: .leading, spacing: 2) {
                Text(destinationLabel ?? MonitorWindowView.Tab.inference.title)
                    .glassType(GlassTokens.TypeScale.heading.weight(.bold))
                    .foregroundStyle(GlassColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
                if let copy, let sub = Self.subLine(Self.rows(harnesses), copy: copy) {
                    Text(sub)
                        .glassType(GlassTokens.TypeScale.label.weight(.regular))
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
            if let copy {
                summary(copy)
            }
        }
    }

    /// The core's words, only while the daemon runs: every row reads it.
    private var runningCopy: PrivateInferenceCopy? {
        guard case .running = model.startup else { return nil }
        return model.privateInferenceCopy
    }

    private func summary(_ copy: PrivateInferenceCopy) -> some View {
        let rows = Self.rows(harnesses)
        let counts = Self.counts(rows)
        let state = PrivateAISwitchCard.stateLabel(
            state: InferenceAccountSection.surfaceState(store.privateAI?.state), copy: copy,
            calls: model.privateInferenceCalls)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // A list nobody could read is a dash in both cells, never zero.
            // Always side by side, two equal columns, as #1146's
            // `.tc-legend` grid is.
            HStack(spacing: GlassTokens.Space.s3) {
                GlassLegendCell(copy.inspectorConnected, value: counts.connected, status: .on)
                GlassLegendCell(copy.inspectorNotConnected, value: counts.notConnected, status: .off)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                // #1146: the Status dot is on while working, outside otherwise.
                InspectorFactRow(label: copy.inspectorStatus, value: state.line,
                                 status: Self.statusDot(store.privateAI?.state, calls: model.privateInferenceCalls))
                InspectorFactRow(
                    label: copy.inspectorCredential,
                    value: CredentialSurface.stateLine(model.credentialStatus, copy: copy, calls: model.credentialCalls))
                InspectorFactRow(label: copy.inspectorConnectedTools, value: Self.names(rows, copy: copy))
            }
            .padding(.horizontal, GlassTokens.Space.s2)
        }
    }

    /// "N of M tools connected", or nil when the list was not read.
    static func subLine(_ rows: [HarnessRow]?, copy: PrivateInferenceCopy) -> String? {
        guard let rows else { return nil }
        return copy.inspectorToolsConnected
            .replacingOccurrences(of: "{connected}", with: String(rows.filter(\.connected).count))
            .replacingOccurrences(of: "{total}", with: String(rows.count))
    }

    /// Connected and not connected, or a dash for each when the list was
    /// not read.
    static func counts(_ rows: [HarnessRow]?) -> (connected: String, notConnected: String) {
        guard let rows else { return ("—", "—") }
        let connected = rows.filter(\.connected).count
        return (String(connected), String(rows.count - connected))
    }

    /// The connected tools' names as the core reports them; the core's
    /// "None" for a list read with none connected, and a dash for a list
    /// nobody could read.
    static func names(_ rows: [HarnessRow]?, copy: PrivateInferenceCopy) -> String {
        guard let rows else { return "—" }
        let connected = rows.filter(\.connected)
        return connected.isEmpty ? copy.inspectorNone : connected.map(\.name).joined(separator: ", ")
    }

    /// The Status row's dot (#1146 `inference-inspector.tsx`): on while the
    /// listener is working, outside otherwise.
    static func statusDot(_ state: DaemonData.PrivateInferenceState?, calls: PrivateInferenceCalls) -> GlassStatus {
        PrivateInferenceIndicator.dotStatus(
            PrivateInferenceSurface.tone(InferenceAccountSection.surfaceState(state), calls: calls))
    }
}

/// One inspector fact as #1146 draws it: the label on the left, the value
/// right-aligned and semibold, with the state's dot before it when it has one.
private struct InspectorFactRow: View {
    let label: String
    let value: String
    var status: GlassStatus?

    var body: some View {
        HStack(alignment: .top, spacing: GlassTokens.Space.s6) {
            Text(label)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize()
            Spacer(minLength: GlassTokens.Space.s4)
            HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                // The dot sits beside the sentence it stands for.
                if let status { GlassStatusDot(status).padding(.top, 5) }
                Text(value)
                    .fontWeight(.semibold)
                    .foregroundStyle(GlassColor.textPrimary)
                    .multilineTextAlignment(.trailing)
                    .fixedSize(horizontal: false, vertical: true)
            }
        }
        .glassType(GlassTokens.TypeScale.body)
        .accessibilityElement(children: .combine)
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
