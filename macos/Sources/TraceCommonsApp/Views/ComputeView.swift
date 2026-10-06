import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

/// The Compute section's content; the window supplies the scroll view and
/// the pane padding, as it does for every other Settings section.
struct ComputeView: View {
    let model: ComputeModel
    @State private var allowance = ""

    var body: some View {
        ComputeContent(model: model, allowance: $allowance)
            .onChange(of: model.snapshot?.ramAllowanceGib, initial: true) { _, value in
                if let value { allowance = String(value) }
            }
    }
}

/// What the allowance field accepts: a whole number above zero. The daemon
/// refuses an allowance of nothing, and a typed word is not an allowance.
enum ComputeAllowance {
    static func parse(_ text: String) -> UInt64? {
        guard let value = UInt64(text), value > 0 else { return nil }
        return value
    }
}

/// The same content renders in the Settings window's scroll view and in CPU
/// screenshot QA (`ComputeNavigationTests`, through `ImageRenderer`).
/// Every sentence is the core's (`ComputeCopy`); this file authors none.
struct ComputeContent: View {
    let model: ComputeModel
    @Binding var allowance: String

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if model.quitWasRefused, let copy = model.copy, let line = copy.quitRefused {
                refusal(line)
            }
            if let snapshot = model.snapshot {
                Text(snapshot.copy.introduction)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                        Text(snapshot.title)
                            .glassType(GlassTokens.TypeScale.bodyStrong)
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(snapshot.detail)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityElement(children: .combine)
                }
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s3) {
                        if snapshot.canEnable {
                            // The field draws the label as its eyebrow.
                            GlassTextField(snapshot.copy.allowanceLabel, text: $allowance)
                                .disabled(model.controlsBusy)
                        } else {
                            Text(snapshot.copy.allowanceLabel)
                                .glassType(GlassTokens.TypeScale.eyebrow)
                                .foregroundStyle(GlassColor.textTertiary)
                            if let value = snapshot.ramAllowanceGib {
                                Text(String(value))
                                    .glassType(GlassTokens.TypeScale.number)
                                    .foregroundStyle(GlassColor.textPrimary)
                            } else {
                                // No allowance answered: absent, never zero.
                                Text(Self.unknownWord)
                                    .glassType(GlassTokens.TypeScale.body)
                                    .foregroundStyle(GlassColor.textTertiary)
                            }
                        }
                        Text(snapshot.copy.allowanceDetail)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                HStack(spacing: GlassTokens.Space.s3) {
                    if snapshot.consentGranted {
                        Button(snapshot.copy.resume, action: resume)
                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canResume)
                        Button(snapshot.copy.pause, action: pause)
                            .disabled(model.controlsBusy || !snapshot.canPause)
                        Button(snapshot.copy.disable, action: disable)
                            .disabled(model.controlsBusy)
                    } else {
                        Button(snapshot.copy.enable, action: enable)
                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canEnable
                                || ComputeAllowance.parse(allowance) == nil)
                    }
                    if model.controlsBusy { SettingsAwaiting() }
                }
                .buttonStyle(GlassButtonStyle(.glass))
            } else if model.failureLabel != nil {
                // Failed: the core's sentence for it, else its unknown word;
                // never a spinner, which would read as working.
                let copy = model.copy
                refusal(Self.failureLine(copy?.unavailable))
                if let retry = copy?.retry {
                    Button(retry) { Task { await model.retryOpen() } }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .disabled(model.controlsBusy)
                }
            } else {
                // Not answered yet: a spinner, never a control that reads as
                // working.
                SettingsAwaiting()
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    /// The core's word for an answer it does not have.
    static let unknown: String? = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())?.unknown

    /// The unknown word, or a dash, not a sentence, if the core's word
    /// could not be read: a label is never left with nothing under it.
    static var unknownWord: String { unknownWord(unknown) }
    static func unknownWord(_ unknown: String?) -> String {
        RouteDisclosureUnreadableGlassLine.text(line: nil, fallback: nil, unknown: unknown)
    }

    /// A failure always has words: the core's line, else its unknown word,
    /// else a dash (`RouteDisclosureUnreadableGlassLine.text`).
    static func failureLine(_ unavailable: String?) -> String { failureLine(unavailable, unknown: unknown) }
    static func failureLine(_ unavailable: String?, unknown: String?) -> String {
        RouteDisclosureUnreadableGlassLine.text(line: unavailable, fallback: nil, unknown: unknown)
    }

    private func refusal(_ line: String) -> some View {
        GlassNotice(tone: .outside) { Text(line).fixedSize(horizontal: false, vertical: true) }
    }

    private func enable() {
        guard let value = ComputeAllowance.parse(allowance) else { return }
        Task { await model.perform(.enable(ramAllowanceGiB: value)) }
    }
    private func resume() { Task { await model.perform(.resume) } }
    private func pause() { Task { await model.perform(.pause) } }
    private func disable() { Task { await model.perform(.disable) } }
}
