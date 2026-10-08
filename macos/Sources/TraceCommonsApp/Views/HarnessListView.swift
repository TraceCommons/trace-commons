import SwiftUI
import TCDesign
import TCShellCore

/// The tools on this computer, at the top of the destination.
///
/// **This file authors no wording at all**, and must never start: every
/// sentence is a field of `PrivateInferenceCopy`, every branch is the shared
/// table's, and the only strings written here are IronWire's own values --
/// a tool's name, its config path, the command it suggests, its id --
/// rendered verbatim or matched. It holds no entry in `ShellWordingTests`'s
/// baseline and must not be given one.
struct HarnessListSection: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy
    /// Whether the list draws its own heading. The Inference tab's panel
    /// draws #1146's eyebrow header above it instead, with the same title.
    var titled = true

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if titled {
                GlassSectionRule(copy.harnessesTitle)
            }
            // Says the choice is per tool AND that the list is what this app
            // knows how to look for. Without the second half a contributor
            // whose tool is missing concludes it cannot be connected.
            Text(copy.harnessesWhat)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            // What today's calls cost, and -- inseparably -- what that
            // figure leaves out. Both are drawn, or neither is: an amount
            // nobody could read draws no line, and the scope sentence alone
            // would qualify a number that is not on screen.
            if let spend = HarnessSurface.spendSentence(
                model.harnesses, calls: model.harnessCalls)
            {
                Text(spend)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                Text(copy.harnessesSpendScope)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // Why a connect control is not on offer. Drawn ONCE, here, and
            // not per row: the fact is about the destination and not about
            // any one tool. An empty sentence draws no line, which is what a
            // daemon that predates the credential gate and a destination the
            // contributor runs themselves both answer -- neither of them
            // refuses a connect, and saying so would be false.
            if let notice = CredentialSurface.harnessNotice(
                credentialed: model.harnesses.destinationCredentialed,
                calls: model.credentialCalls)
            {
                Text(notice)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // `.none` is "nothing is known": a list not read yet and a
            // payload this build could not read both land there, and neither
            // is a machine with no tools on it. The core's unknown word, not
            // "none found".
            if model.harnesses == HarnessList.none {
                RouteDisclosureUnreadableGlassLine(line: nil)
            } else if model.harnesses.harnesses.isEmpty {
                Text(copy.harnessesNoneFound)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(model.harnesses.harnesses) { row in
                        HarnessRowView(row: row, copy: copy)
                    }
                }
            }
        }
        // Glass modals over the whole window, not stock sheets.
        .glassModal(isPresented: exposureBinding) {
            HarnessExposureSheet(copy: copy)
        }
        .glassModal(isPresented: previewBinding) {
            if let plan = model.harnessPreview {
                HarnessPreviewSheet(plan: plan, copy: copy)
            }
        }
    }

    /// Dismissing either modal (Escape, or its cancel) is the same as
    /// saying no. The exposure question left unanswered connects nothing and
    /// records nothing; the preview left unconfirmed writes nothing and the
    /// plan expires where it was minted.
    private var exposureBinding: Binding<Bool> {
        Binding(
            get: { model.harnessExposureRequest != nil },
            set: { if !$0 { model.answerHarnessExposure(accepted: false) } })
    }

    private var previewBinding: Binding<Bool> {
        Binding(
            get: { model.harnessPreview != nil },
            set: { if !$0 { model.cancelHarnessPreview() } })
    }
}

/// One tool, as #1146's `HarnessListPanel` row: flat, 15 above and below,
/// a hairline under it; on the left the name, the state in words and the
/// settings file inline; on the right the connection caption and the one
/// neutral action.
private struct HarnessRowView: View {
    @EnvironmentObject private var model: AppModel
    let row: HarnessRow
    let copy: PrivateInferenceCopy

    var body: some View {
        let state = HarnessSurface.state(row, calls: model.harnessCalls)
        ViewThatFits(in: .horizontal) {
            HStack(alignment: .top, spacing: 18) {
                details(state)
                Spacer(minLength: 0)
                VStack(alignment: .trailing, spacing: GlassTokens.Space.s3) {
                    connectionCaption
                    actionButton
                }
                .fixedSize()
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                details(state)
                connectionCaption
                actionButton
            }
        }
        .padding(.vertical, 15)
        .frame(maxWidth: .infinity, alignment: .leading)
        .overlay(alignment: .bottom) { GlassHairline(GlassColor.hairline) }
    }

    /// The name, then the state in the core's words, then the settings
    /// file and the command, both selectable.
    private func details(_ state: HarnessState) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            Text(row.name)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            // The one state that means a call arrived is the only one the
            // core words as working, and the two that cannot be attributed
            // say nothing rather than borrow a claim. A tool that is not on
            // this machine is listed and says so. #1146 draws it as plain
            // text: the sentence is the signal, with no dot to misread.
            if let sentence = HarnessSurface.rowSentence(
                row, copy: copy, calls: model.harnessCalls)
            {
                Text(sentence)
                    .glassType(GlassTokens.TypeScale.label.weight(.regular))
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let restart = HarnessSurface.restartSentence(row, state: state, copy: copy) {
                Text(restart)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // When a call last arrived, assembled on the far side and
            // empty when there is nothing to report -- which draws no line
            // at all.
            if let lastCall = HarnessSurface.lastCallSentence(row, calls: model.harnessCalls) {
                Text(lastCall)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // #1146 shows the settings file inline as code.
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                if let path = row.configPath {
                    Text(verbatim: path)
                        .textSelection(.enabled)
                }
                Text(verbatim: row.connectCommand)
                    .textSelection(.enabled)
            }
            .glassType(GlassTokens.TypeScale.mono)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
        }
    }

    /// Whether the tool's settings name this computer, as #1146's row
    /// caption says it. A settings fact, never a claim that a call arrived:
    /// that is the state line's.
    private var connectionCaption: some View {
        Text(row.connected ? copy.harnessCaptionConnected : copy.harnessCaptionNotConnected)
            .glassType(GlassTokens.TypeScale.caption)
            .foregroundStyle(GlassColor.textTertiary)
            .lineLimit(1)
    }

    /// One button, or none. Which action it offers is the shared table's
    /// answer, so an uninstalled tool that still holds our line keeps the
    /// control that removes it. #1146: Connect is a neutral glass button,
    /// Disconnect a glass button in the outside ink.
    @ViewBuilder
    private var actionButton: some View {
        if let action = HarnessSurface.action(row, calls: model.harnessCalls) {
            Button(HarnessSurface.actionLabel(action, copy: copy)) {
                model.beginHarnessAction(id: row.id, action: action)
            }
            .buttonStyle(GlassButtonStyle(action == .connect ? .glass : .destructive))
            .accessibilityLabel(Text(row.name) + Text(verbatim: ": ") + Text(HarnessSurface.actionLabel(action, copy: copy)))
            .disabled(model.harnessBusy)
        }
    }
}

/// The change, before it is made.
///
/// The confirm button appears only for a plan the daemon minted an id for.
/// There is no other route to a write: this sheet cannot describe a change,
/// only point at the one it was handed.
private struct HarnessPreviewSheet: View {
    @EnvironmentObject private var model: AppModel
    let plan: HarnessPlan
    let copy: PrivateInferenceCopy

    var body: some View {
        GlassModal(
            title: copy.harnessPreviewTitle, width: .narrow, actions: actions,
            onCancel: { model.cancelHarnessPreview() }
        ) {
            GlassModalBody { details }
        }
    }

    /// Saying no leaves the file with every value it has, which is what
    /// its own words say. Confirm is absent for every outcome that is not
    /// committable, so an empty plan can never be confirmed into nothing;
    /// it is the default (Return) and waits on busy.
    private var actions: [GlassModalAction] {
        var actions: [GlassModalAction] = [
            .cancel(copy.harnessPreviewCancel) { model.cancelHarnessPreview() },
        ]
        if HarnessSurface.canCommit(plan, calls: model.harnessCalls) {
            actions.append(GlassModalAction(
                copy.harnessPreviewConfirm, isDefault: true, isEnabled: !model.harnessBusy
            ) { model.confirmHarnessPreview() })
        }
        return actions
    }

    @ViewBuilder
    private var details: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if let path = plan.path {
                Text(path)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textSecondary)
                    .textSelection(.enabled)
            }
            // Every outcome that writes nothing says why. A file this app
            // refused to rewrite is not a file with nothing to change, and
            // neither is a tool that is not here or a path this build could
            // not work out; the shared table keeps all of them apart.
            if let sentence = HarnessSurface.outcomeSentence(
                plan, calls: model.harnessCalls)
            {
                Text(sentence)
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // IronWire's own words for what would change, verbatim.
            ForEach(Array(plan.changes.enumerated()), id: \.offset) { _, change in
                Text(change)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // Reported, never offered. A plan can fill two empty slots and
            // leave a third alone in the same pass, so this rides alongside
            // whatever the outcome was -- and there is deliberately no
            // control here that would take the slot over.
            if !plan.occupied.isEmpty {
                Text(HarnessSurface.occupiedSentence(copy: copy))
                    .glassType(GlassTokens.TypeScale.body)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(Array(plan.occupied.enumerated()), id: \.offset) { _, slot in
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                        Text(slot.slot)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textPrimary)
                        Text(slot.current)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textSecondary)
                            .textSelection(.enabled)
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// The question a first connect has to put.
///
/// Connecting one tool starts a listener open to everything on this machine,
/// which does not follow from connecting one tool -- so the exposure
/// paragraph is shown in full, with the same two answers the first-run offer
/// has. Declining records the answer and connects nothing.
private struct HarnessExposureSheet: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy

    /// Decline is the cancel (Escape); Accept is the default (Return) and
    /// waits on busy.
    var body: some View {
        GlassModal(
            title: copy.offerTitle, width: .narrow,
            actions: [
                .cancel(copy.offerDecline) { model.answerHarnessExposure(accepted: false) },
                GlassModalAction(copy.offerAccept, isDefault: true, isEnabled: !model.harnessBusy) {
                    model.answerHarnessExposure(accepted: true)
                },
            ],
            onCancel: { model.answerHarnessExposure(accepted: false) }
        ) {
            GlassModalBody { question }
        }
    }

    private var question: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(copy.offerWhat)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.offerExposure)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.offerNoRepoint)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
            // The first-run offer's asked-once sentence is deliberately NOT
            // shown here. It says "this is the only time you will be asked.
            // The switch stays in Settings", and both halves are false on
            // this surface: the harness gate is
            // `connectNeedsExposure(listenerOn:)`, which is wider than
            // `shouldOffer` on purpose so a connect after the kill switch
            // asks again; and the switch now lives on this destination, with
            // Settings holding only a pointer. It remains true on the
            // first-run offer, which is the sentence's home.
        }
        .frame(maxWidth: .infinity, alignment: .leading)
    }
}

/// IronWire's harness ids to the tools that have artwork, in one place:
/// the flow map forwards here.
enum HarnessToolArt {
    static func tool(harness id: String) -> GlassTool? {
        switch id {
        case "claude", "claude-code": .claudeCode
        case "codex": .codex
        case "gemini", "gemini-cli": .geminiCLI
        case "cline": .cline
        case "opencode": .openCode
        default: nil
        }
    }
}
