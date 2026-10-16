import SwiftUI
import TCDesign
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
/// The body of #1146's balance panel (`PrivateAIBalanceCard`), which draws
/// the heading and the re-read link above it. It draws no button: the
/// sign-in a balance needs is the credential card's own `obtain` -- the same
/// ceremony, the same browser, the same key -- drawn there beside the
/// provider chooser it uses (`BalanceSurface.actionToDraw`).
struct BalanceRow: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy
    /// Whether the row says what the figures cover. The Private AI tab's
    /// icon card says it in its head instead.
    var showsScope = true

    var body: some View {
        let status = model.balanceStatus
        let tone = BalanceSurface.tone(status, calls: model.balanceCalls)
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            // A sentence OR the figures, never both: every state but the one
            // that was read has null figures behind it, and the sentence for
            // a null remaining amount is about an uncapped account, which is
            // nonsense beside "no sign-in is kept here".
            if let sentence = BalanceSurface.stateLine(
                status, copy: copy, calls: model.balanceCalls)
            {
                GlassStatusLabel(sentence, status: PrivateInferenceIndicator.status(tone))
                    .fixedSize(horizontal: false, vertical: true)
            } else if BalanceSurface.showsFigures(status, calls: model.balanceCalls) {
                figures(status)
            }
            // What the figures cover, and what they therefore are not. Drawn
            // in every state: it qualifies the heading, which names an
            // account, as much as it qualifies any number under it.
            if showsScope {
                Text(copy.balanceWhat)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
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
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            switch BalanceSurface.remaining(status, copy: copy, calls: model.balanceCalls) {
            case .figure(let amount):
                // Tabular digits so a figure that changes under a poll
                // does not reflow the line it sits on.
                Text(amount)
                    .glassType(GlassTokens.TypeScale.number)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
            case .sentence(let sentence):
                Text(sentence)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The ceiling and the running total, each drawn only when the
            // account has one. Absent is no line; a zero is a real $0.00 and
            // keeps its line, and the difference is the Rust's.
            if let limit = BalanceSurface.limitLine(status, calls: model.balanceCalls) {
                Text(limit).glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
            }
            if let spent = BalanceSurface.spentLine(status, calls: model.balanceCalls) {
                Text(spent).glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
            }
            // When the question was put -- not when the service updated
            // anything, which is a claim nothing here supports.
            if let observed = BalanceSurface.observedLine(
                status, now: Date(), calls: model.balanceCalls)
            {
                Text(observed).glassType(GlassTokens.TypeScale.caption).foregroundStyle(GlassColor.textSecondary)
            }
        }
    }
}
