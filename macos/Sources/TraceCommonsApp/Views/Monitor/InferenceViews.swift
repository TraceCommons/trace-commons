#if DEBUG
import SwiftUI
import TCDesign
import TCShellCore

/// The Inference tab (R8 of #1173): the model calls IronWire recorded,
/// newest first, with what each was priced at and its proof label.
///
/// An unreadable ledger is not an empty one: it draws a dash and the
/// reason's fixed label, never an empty table. Priced is not billed: no
/// figure here is money spent. Only `verified` is drawn as proof.
struct InferenceTabView: View {
    let store: InferenceStore

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                if let failure = store.failures["inference_calls"] {
                    GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                }
                if let page = store.calls {
                    if page.readable {
                        totals(page)
                        if let summary = store.summary, summary.readable, !summary.models.isEmpty {
                            models(summary)
                        }
                        calls(page)
                    } else {
                        unreadable
                    }
                } else if store.failures["inference_calls"] == nil {
                    ProgressView().controlSize(.small).frame(maxWidth: .infinity)
                }
            }
        }
        .scrollIndicators(.never)
    }

    // MARK: Totals

    private func totals(_ page: DaemonData.InferenceCallPage) -> some View {
        let verified = page.calls.filter { $0.proofLabel.isProof }.count
        return HStack(spacing: GlassTokens.Space.s3) {
            GlassLegendCell(MonitorWords.calls, value: String(page.calls.count), status: .shared)
            GlassLegendCell(InferenceWords.proof(.verified), value: String(verified), status: .on)
            GlassLegendCell(MonitorWords.priced, value: Self.priced(page.calls.map(\.cost)), status: .kept)
        }
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
            GlassTag(InferenceWords.proof(call.proofLabel), tone: Self.tone(call.proofLabel))
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
        case .failed, .outside: .outside
        case .unattested, .unavailable, .unrecorded: .neutral
        }
    }

    static func tone(_ raw: String) -> GlassTag.Tone {
        tone(DaemonData.ProofLabel(rawValue: raw) ?? .unrecorded)
    }

    /// Proof counts in a fixed order, so the tags do not shuffle on reload.
    static func sortedProofs(_ counts: [String: Int]?) -> [(label: String, count: Int)] {
        let order = DaemonData.ProofLabel.allCases.map(\.rawValue)
        return (counts ?? [:])
            .sorted { (order.firstIndex(of: $0.key) ?? .max) < (order.firstIndex(of: $1.key) ?? .max) }
            .map { (label: $0.key, count: $0.value) }
    }
}

/// The inspector on the Inference tab: the tools that can send model calls
/// here, each with the core's sentence for its state.
struct PrivateAIInspectorView: View {
    let store: InferenceStore
    let destinationLabel: String?
    let sentence: (HarnessRow) -> String?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {
                Text(destinationLabel ?? MonitorWindowView.Tab.inference.rawValue)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                if let failure = store.failures["harness_list"] {
                    GlassNotice(tone: .outside, title: failure.description) { EmptyView() }
                }
                if let harnesses = store.harnesses {
                    ForEach(harnesses.harnesses) { row in
                        GlassCard {
                            HStack(alignment: .top, spacing: GlassTokens.Space.s4) {
                                if let tool = FlowMapScene.glassTool(harness: row.id) {
                                    GlassToolTile(.tool(tool))
                                }
                                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                                    Text(row.name)
                                        .glassType(GlassTokens.TypeScale.bodyStrong)
                                        .foregroundStyle(GlassColor.textPrimary)
                                    if let line = sentence(row) {
                                        Text(line)
                                            .glassType(GlassTokens.TypeScale.caption)
                                            .foregroundStyle(GlassColor.textSecondary)
                                            .fixedSize(horizontal: false, vertical: true)
                                    }
                                }
                                Spacer(minLength: 0)
                            }
                        }
                        .accessibilityElement(children: .combine)
                    }
                } else if store.failures["harness_list"] == nil {
                    ProgressView().controlSize(.small).frame(maxWidth: .infinity)
                }
            }
        }
        .scrollIndicators(.never)
    }
}

/// The single words the map and the Inference tab show, beside the names
/// the core reports and the core's sentences (`ShellWordingTests`).
enum InferenceWords {
    /// IronWire's proof label, as one word. Only `verified` is proof.
    static func proof(_ label: DaemonData.ProofLabel) -> String {
        switch label {
        case .verified: "Verified"
        case .gatewayOnly: "Gateway"
        case .unattested: "Unattested"
        case .pending: "Pending"
        case .unavailable: "Unavailable"
        case .failed: "Failed"
        case .outside: "Outside"
        case .unrecorded: "Unrecorded"
        }
    }

    static func proof(_ raw: String) -> String {
        proof(DaemonData.ProofLabel(rawValue: raw) ?? .unrecorded)
    }
}

extension MonitorWords {
    static let computer = "Computer"
    static let commons = "Commons"
    static let waiting = "Waiting"
    static let folders = "Folders"
    static let watched = "Watched"
    static let off = "Off"
    static let connected = "Connected"
    static let reduce = "Reduce"
    static let enlarge = "Enlarge"
    static let calls = "Calls"
    static let models = "Models"
    static let priced = "Priced"
    static let unknown = "Unknown"
}
#endif
