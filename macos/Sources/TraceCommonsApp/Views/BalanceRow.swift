import SwiftUI
import TCShellCore

/// What is left in the account this destination spends from.
///
/// **This file authors no wording at all**, and must never start: every
/// sentence is a field of `PrivateInferenceCopy` or comes back from a shared
/// table, and every figure is formatted by the Rust. It holds no entry in
/// `ShellWordingTests`'s baseline and must not be given one.
///
/// It authors no THRESHOLD either. Nothing here compares an amount to
/// anything, and the tone is the shared table's alone -- which answers
/// "settled" for a read that succeeded and says nothing about whether the
/// balance is healthy. A low figure painted red would be this app inventing a
/// limit nobody set, on an account whose ceiling may not exist at all.
///
/// Drawn inside `CredentialSection` rather than beside it: the balance is
/// what the key above is for, and a contributor who has just signed in should
/// find it on the same card.
struct BalanceRow: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy
    /// What the sign-in row is already offering, so this row does not draw
    /// the same button a second time. The decision is
    /// `BalanceSurface.actionToDraw`'s.
    let credentialAction: CredentialAction
    /// The sign-in row's own handler. Reused rather than reimplemented: this
    /// row's `obtain` IS that row's `obtain` -- the same ceremony, the same
    /// browser, the same key -- and a second implementation would be a second
    /// thing to keep in agreement.
    let run: (CredentialAction) -> Void

    var body: some View {
        let status = model.balanceStatus
        let tone = PrivateInferenceIndicator.palette(
            BalanceSurface.tone(status, calls: model.balanceCalls))
        let action = BalanceSurface.actionToDraw(
            balance: BalanceSurface.action(status, calls: model.balanceCalls),
            credential: credentialAction)
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: copy.balanceTitle)
            // A sentence OR the figures, never both: every state but the one
            // that was read has null figures behind it, and the sentence for
            // a null remaining amount is about an uncapped account, which is
            // nonsense beside "no sign-in is kept here".
            if let sentence = BalanceSurface.stateLine(
                status, copy: copy, calls: model.balanceCalls)
            {
                Label(sentence, systemImage: tone.symbol)
                    .font(TC.Font_.body)
                    .foregroundStyle(tone.textColor)
                    .fixedSize(horizontal: false, vertical: true)
            } else if BalanceSurface.showsFigures(status, calls: model.balanceCalls) {
                figures(status)
            }
            // What the figures cover, and what they therefore are not. Drawn
            // in every state: it qualifies the heading, which names an
            // account, as much as it qualifies any number under it.
            Text(copy.balanceWhat)
                .font(TC.Font_.meta)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            actionButton(action)
        }
    }

    /// The read itself.
    ///
    /// The lead is the amount, at metric weight, under a heading that already
    /// names it -- so the row reads as a balance rather than as a number
    /// sitting under a blank line where a sentence used to be. When there is
    /// no amount to lead with, the Rust's own sentence takes the same
    /// position at the same emphasis, because "no ceiling is set" is the
    /// answer to the heading's question and not a footnote to it.
    @ViewBuilder
    private func figures(_ status: BalanceStatus) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            switch BalanceSurface.remaining(status, copy: copy, calls: model.balanceCalls) {
            case .figure(let amount):
                // Monospaced digits so a figure that changes under a poll
                // does not reflow the line it sits on.
                Text(amount)
                    .font(TC.Font_.metricValueMono)
                    .monospacedDigit()
                    .textSelection(.enabled)
            case .sentence(let sentence):
                Text(sentence)
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The ceiling and the running total, each drawn only when the
            // account has one. Absent is no line; a zero is a real $0.00 and
            // keeps its line, and the difference is the Rust's.
            if let limit = BalanceSurface.limitLine(status, calls: model.balanceCalls) {
                Text(limit).font(TC.Font_.meta).foregroundStyle(.secondary)
            }
            if let spent = BalanceSurface.spentLine(status, calls: model.balanceCalls) {
                Text(spent).font(TC.Font_.meta).foregroundStyle(.secondary)
            }
            // When the question was put -- not when the service updated
            // anything, which is a claim nothing here supports.
            if let observed = BalanceSurface.observedLine(
                status, now: Date(), calls: model.balanceCalls)
            {
                Text(observed).font(TC.Font_.meta).foregroundStyle(.secondary)
            }
        }
    }

    /// The one button this row may offer, or none.
    ///
    /// `.none` draws nothing rather than a disabled control, for
    /// `CredentialSection.actionButton`'s reason. It is also what a duplicate
    /// of the sign-in row's own button becomes.
    @ViewBuilder
    private func actionButton(_ action: CredentialAction) -> some View {
        if let label = CredentialSurface.actionLabel(action, copy: copy) {
            Button(label) { run(action) }
                .buttonStyle(.bordered)
                .disabled(model.credentialBusy)
        }
    }
}
