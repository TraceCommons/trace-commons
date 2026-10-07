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
    @EnvironmentObject private var model: AppModel

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
    }

    private var ledger: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                // Saved model accounts and managed sessions lead the tab
                // (#1146's private-AI page): accounts, then sessions, then
                // their error notice.
                ManagedSessionsSection()
                // The standard tool settings below are global; managed
                // launches never edit them.
                ManagedGlobalSettingsHeader()
                // Sign-in with balance and funding, the tools with their
                // connect action, and the Private AI switch, in the main pane
                // above the ledger as #1146's Private AI page has them (owner
                // ruling on #1241). Drawn only while the daemon runs, which
                // every one of them needs: the ledger is.
                InferenceAccountSection(store: store)
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
        .scrollIndicators(.never)
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

/// The inspector on the Inference tab: the Private AI summary, as #1146's
/// `inference-inspector.tsx` draws it. How many tools point at it, the
/// listener's state in the core's sentence (never the switch: what was asked
/// for is not what happened), the credential's state, and which tools are
/// connected. Its controls are in the main pane (`InferenceAccountSection`).
struct PrivateAIInspectorView: View {
    let store: InferenceStore
    let destinationLabel: String?
    @EnvironmentObject private var model: AppModel

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                Text(destinationLabel ?? MonitorWindowView.Tab.inference.title)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                if case .running = model.startup, let copy = model.privateInferenceCopy {
                    summary(copy)
                }
            }
        }
        .scrollIndicators(.never)
    }

    private func summary(_ copy: PrivateInferenceCopy) -> some View {
        let rows = store.harnesses?.harnesses
        let connected = rows?.filter(\.connected) ?? []
        let state = PrivateAISwitchCard.stateLabel(
            state: InferenceAccountSection.surfaceState(store.privateAI?.state), copy: copy,
            calls: model.privateInferenceCalls)
        return VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
            // #1146's line under the title; none when the list was not read.
            if let line = Self.connectedLine(rows) {
                Text(line)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
            }
            // A list nobody could read is a dash, never zero connected.
            GlassLegendCell(MonitorWords.connected, value: Self.connectedCount(rows), status: .on)
            GlassStatusLabel(state.line, status: state.status)
                .fixedSize(horizontal: false, vertical: true)
            GlassKeyValueList([
                .init(copy.credentialTitle,
                      CredentialSurface.stateLine(model.credentialStatus, copy: copy, calls: model.credentialCalls)),
                .init(copy.harnessesTitle, rows == nil ? "—" : Self.names(connected)),
            ])
        }
    }

    /// The core's "{count} of {total} tools connected", or nil when the
    /// list was not read: an unread list is not "0 of 0".
    static func connectedLine(_ rows: [HarnessRow]?) -> String? {
        guard let rows, let template = ShellWords.table?.inference.toolsConnectedOf else { return nil }
        return template
            .replacingOccurrences(of: "{count}", with: String(rows.filter(\.connected).count))
            .replacingOccurrences(of: "{total}", with: String(rows.count))
    }

    /// Connected over listed, or a dash when the list was not read.
    static func connectedCount(_ rows: [HarnessRow]?) -> String {
        guard let rows else { return "—" }
        return "\(rows.filter(\.connected).count)/\(rows.count)"
    }

    /// The connected tools' names as the core reports them; a dash for none.
    static func names(_ rows: [HarnessRow]) -> String {
        rows.isEmpty ? "—" : rows.map(\.name).joined(separator: ", ")
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
