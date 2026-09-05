import SwiftUI

/// Native transport and browser presentation only; Rust owns lifecycle and cadence.
struct NearAccountConnectView: View {
    @EnvironmentObject private var model: AppModel
    @Environment(\.openURL) private var openURL
    @State private var commons = ""
    @State private var account = ""
    @State private var flow: NativeWalletView?
    @State private var pending = false
    @State private var closed = false
    @State private var transportFailed = false
    var onBusyChanged: (Bool) -> Void = { _ in }
    var onEnrolled: () -> Void
    private var busy: Bool { pending || flow?.busy == true }
    var body: some View {
        Group {
            if let copy = model.witnessCopy?.wallet, let flow, flow.state != "Unsupported" {
                VStack(alignment: .leading, spacing: TC.Space.m) {
                    Text(copy.heading).font(TC.Font_.cardTitle)
                    Text(copy.disclosure).font(.callout).foregroundStyle(.secondary)
                    TextField(copy.commons, text: $commons).textFieldStyle(.roundedBorder).disabled(busy || !flow.canEdit)
                    Button(copy.check) { run("check") }.disabled(pending || !flow.canCheck)
                    if flow.canStart {
                        TextField(copy.account, text: $account).textFieldStyle(.roundedBorder).disabled(busy)
                        Button(copy.start) { run("start") }.disabled(pending)
                    }
                    if busy { ProgressView().controlSize(.small) }
                    if transportFailed { NativeFlowNotice(message: copy.failed, glyph: copy.refusedGlyph, tone: copy.refusedTone) }
                    else if flow.tone == "refused" { NativeFlowNotice(message: flow.message, glyph: flow.glyph, tone: flow.tone) }
                    else if !flow.message.isEmpty { Text(flow.message).font(.callout).foregroundStyle(.secondary) }
                    if flow.canCancel { Button(copy.cancel, role: .cancel) { run("cancel") } }
                }
            }
        }
        .task {
            flow = await model.nativeWalletFlow(action: "open", flowID: "", commons: "", account: "")
            if closed { await cancel() }
        }
        .onChange(of: busy) { _, value in onBusyChanged(value) }
        .onDisappear { closed = true; Task { await cancel() } }
    }
    private func cancel() async {
        guard let flow else { return }
        self.flow = await model.nativeWalletFlow(action: "cancel", flowID: flow.flowID, commons: "", account: "") ?? flow
    }
    private func run(_ action: String) {
        guard let flow else { return }
        pending = true
        transportFailed = false
        Task {
            defer { pending = false }
            guard let result = await model.nativeWalletFlow(action: action, flowID: flow.flowID, commons: commons, account: account) else { transportFailed = true; return }
            self.flow = result
            if closed { await cancel(); return }
            if action == "start", let browser = result.browserURL {
                guard let url = URL(string: browser) else { await cancel(); return }
                let accepted = await withCheckedContinuation { continuation in openURL(url) { continuation.resume(returning: $0) } }
                if !accepted { await cancel(); return }
            }
            while !closed, self.flow?.wait == true {
                guard let next = await model.nativeWalletFlow(action: "wait", flowID: result.flowID, commons: "", account: "") else { transportFailed = true; return }
                self.flow = next
            }
            if !closed, self.flow?.state == "Complete" { account = ""; onEnrolled() }
        }
    }
}
struct NativeWalletView: Decodable, Sendable {
    let flowID: String, state: String, message: String, tone: String, glyph: String
    let busy: Bool, canCheck: Bool, canStart: Bool, canEdit: Bool, canCancel: Bool, wait: Bool
    let browserURL: String?
    enum CodingKeys: String, CodingKey {
        case state, message, tone, glyph, busy, wait
        case flowID = "flow_id", canCheck = "can_check", canStart = "can_start"
        case canEdit = "can_edit", canCancel = "can_cancel", browserURL = "browser_url"
    }
}
