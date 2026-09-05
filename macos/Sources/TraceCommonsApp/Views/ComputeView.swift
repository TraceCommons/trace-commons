import SwiftUI
import TCBridge
import TCShellCore

struct ComputeView: View {
    let model: ComputeModel
    @State private var allowance = ""

    var body: some View {
        ScrollView {
            ComputeContent(model: model, allowance: $allowance)
        }
        .onChange(of: model.snapshot?.ramAllowanceGib, initial: true) { _, value in
            if let value { allowance = String(value) }
        }
    }
}

/// The same content renders in a native scroll view and in CPU screenshot QA.
struct ComputeContent: View {
    let model: ComputeModel
    @Binding var allowance: String

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.l) {
            if model.quitWasRefused, let line = model.copy?.quitRefused {
                refusal(line)
            }
            if let snapshot = model.snapshot {
                Text(snapshot.copy.introduction)
                    .foregroundStyle(TC.inkSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: TC.Space.s) {
                    Text(snapshot.title).font(TC.Font_.cardTitle)
                    Text(snapshot.detail).foregroundStyle(TC.inkSecondary)
                }
                .accessibilityElement(children: .combine)
                .padding(TC.Space.l)
                .frame(maxWidth: .infinity, alignment: .leading)
                .tcCard()

                VStack(alignment: .leading, spacing: TC.Space.s) {
                    Text(snapshot.copy.allowanceLabel).font(TC.Font_.cardTitle)
                    if snapshot.canEnable {
                        TextField(snapshot.copy.allowanceLabel, text: $allowance)
                            .textFieldStyle(.roundedBorder)
                            .disabled(model.controlsBusy)
                    } else if let value = snapshot.ramAllowanceGib {
                        Text(String(value)).monospacedDigit()
                    }
                    Text(snapshot.copy.allowanceDetail)
                        .font(TC.Font_.caption)
                        .foregroundStyle(TC.inkSecondary)
                }
                HStack {
                    if snapshot.consentGranted {
                        Button(snapshot.copy.resume, action: resume)
                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canResume)
                        Button(snapshot.copy.pause, action: pause)
                            .disabled(model.controlsBusy || !snapshot.canPause)
                        Button(snapshot.copy.disable, action: disable)
                            .disabled(model.controlsBusy)
                    } else {
                        Button(snapshot.copy.enable, action: enable)
                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canEnable || parsedAllowance == nil)
                    }
                }
                if model.controlsBusy { ProgressView() }
            } else if model.failureLabel != nil {
                if let copy = model.copy, let line = copy.unavailable {
                    refusal(line)
                    if let retry = copy.retry {
                        Button(retry) { Task { await model.retryOpen() } }
                            .disabled(model.controlsBusy)
                    }
                }
            } else {
                ProgressView()
            }
        }
        .padding(TC.Space.lg)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    private func refusal(_ line: String) -> some View {
        Label(line, systemImage: TC.Tone.refused.symbol)
            .foregroundStyle(TC.coralText)
            .fixedSize(horizontal: false, vertical: true)
    }

    private var parsedAllowance: UInt64? {
        guard let value = UInt64(allowance), value > 0 else { return nil }
        return value
    }

    private func enable() {
        guard let value = parsedAllowance else { return }
        Task { await model.perform(.enable(ramAllowanceGiB: value)) }
    }
    private func resume() { Task { await model.perform(.resume) } }
    private func pause() { Task { await model.perform(.pause) } }
    private func disable() { Task { await model.perform(.disable) } }
}
