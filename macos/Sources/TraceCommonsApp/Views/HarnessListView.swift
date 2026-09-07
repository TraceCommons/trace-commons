import SwiftUI
import TCShellCore

/// The tools on this computer, at the top of the destination.
///
/// **This file authors no wording at all**, and must never start: every
/// sentence is a field of `PrivateInferenceCopy`, every branch is the shared
/// table's, and the only strings written here are IronWire's own values --
/// a tool's name, its config path, the command it suggests -- rendered
/// verbatim. It holds no entry in `ShellWordingTests`'s baseline and must
/// not be given one.
struct HarnessListSection: View {
    @EnvironmentObject private var model: AppModel
    let copy: PrivateInferenceCopy

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.sm) {
            TCSectionHeader(title: copy.harnessesTitle)
            // Says the choice is per tool AND that the list is what this app
            // knows how to look for. Without the second half a contributor
            // whose tool is missing concludes it cannot be connected.
            Text(copy.harnessesWhat)
                .font(TC.Font_.body)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            // What today's calls cost, and -- inseparably -- what that
            // figure leaves out. Both are drawn, or neither is: an amount
            // nobody could read draws no line, and the scope sentence alone
            // would qualify a number that is not on screen.
            if let spend = HarnessSurface.spendSentence(
                model.harnesses, calls: model.harnessCalls)
            {
                Text(spend)
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
                Text(copy.harnessesSpendScope)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if model.harnesses.harnesses.isEmpty {
                Text(copy.harnessesNoneFound)
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
            } else {
                ForEach(model.harnesses.harnesses) { row in
                    HarnessRowView(row: row, copy: copy)
                }
            }
        }
        .sheet(isPresented: exposureBinding) {
            HarnessExposureSheet(copy: copy)
        }
        .sheet(isPresented: previewBinding) {
            if let plan = model.harnessPreview {
                HarnessPreviewSheet(plan: plan, copy: copy)
            }
        }
    }

    /// Dismissing either sheet is the same as saying no. The exposure
    /// question left unanswered connects nothing and records nothing; the
    /// preview left unconfirmed writes nothing and the plan expires where it
    /// was minted.
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

/// One tool.
private struct HarnessRowView: View {
    @EnvironmentObject private var model: AppModel
    let row: HarnessRow
    let copy: PrivateInferenceCopy

    var body: some View {
        let state = HarnessSurface.state(row, calls: model.harnessCalls)
        let tone = PrivateInferenceIndicator.palette(HarnessSurface.tone(state))
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.m) {
                // IronWire's name for the tool, never spelled by this shell.
                Text(row.name).font(TC.Font_.cardTitle)
                Spacer(minLength: TC.Space.m)
                actionButton
            }
            // The one state that means a call arrived is the only one drawn
            // as working, and the two that cannot be attributed say nothing
            // rather than borrow a claim. A tool that is not on this machine
            // is listed and says so, rather than borrowing a sentence about
            // settings it does not have.
            if let sentence = HarnessSurface.rowSentence(
                row, copy: copy, calls: model.harnessCalls)
            {
                Label(sentence, systemImage: tone.symbol)
                    .font(TC.Font_.body)
                    .foregroundStyle(tone.textColor)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let restart = HarnessSurface.restartSentence(row, state: state, copy: copy) {
                Text(restart)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // When a call last arrived, assembled on the far side and empty
            // when there is nothing to report -- which draws no line at all.
            if let lastCall = HarnessSurface.lastCallSentence(row, calls: model.harnessCalls) {
                Text(lastCall)
                    .font(TC.Font_.meta)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // The file this app would change, always. A tool nobody expected
            // to be set up is a question about which file, every time.
            if let path = row.configPath {
                Text(path)
                    .font(TC.Font_.monoCode)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            // What to run instead. A command, not prose, so it is monospaced
            // and selectable and never restated in words.
            Text(row.connectCommand)
                .font(TC.Font_.monoCode)
                .foregroundStyle(.secondary)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    /// One button, or none. Which action it offers is the shared table's
    /// answer, so an uninstalled tool that still holds our line keeps the
    /// control that removes it.
    @ViewBuilder
    private var actionButton: some View {
        if let action = HarnessSurface.action(row, calls: model.harnessCalls) {
            Button(HarnessSurface.actionLabel(action, copy: copy)) {
                model.beginHarnessAction(id: row.id, action: action)
            }
            .buttonStyle(.bordered)
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
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.harnessPreviewTitle).font(TC.Font_.sectionTitle)
            if let path = plan.path {
                Text(path)
                    .font(TC.Font_.monoCode)
                    .foregroundStyle(.secondary)
                    .textSelection(.enabled)
            }
            // Every outcome that writes nothing says why. A file this app
            // refused to rewrite is not a file with nothing to change, and
            // neither is a tool that is not here or a path this build could
            // not work out; the shared table keeps all of them apart.
            if let sentence = HarnessSurface.outcomeSentence(
                plan, calls: model.harnessCalls)
            {
                Text(sentence).font(TC.Font_.body).fixedSize(horizontal: false, vertical: true)
            }
            // IronWire's own words for what would change, verbatim.
            ForEach(Array(plan.changes.enumerated()), id: \.offset) { _, change in
                Text(change)
                    .font(TC.Font_.monoCode)
                    .textSelection(.enabled)
                    .fixedSize(horizontal: false, vertical: true)
            }
            // Reported, never offered. A plan can fill two empty slots and
            // leave a third alone in the same pass, so this rides alongside
            // whatever the outcome was -- and there is deliberately no
            // control here that would take the slot over.
            if !plan.occupied.isEmpty {
                Text(HarnessSurface.occupiedSentence(copy: copy))
                    .font(TC.Font_.body)
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(Array(plan.occupied.enumerated()), id: \.offset) { _, slot in
                    VStack(alignment: .leading, spacing: TC.Space.micro) {
                        Text(slot.slot).font(TC.Font_.monoCode)
                        Text(slot.current)
                            .font(TC.Font_.monoCode)
                            .foregroundStyle(.secondary)
                            .textSelection(.enabled)
                    }
                }
            }
            HStack(spacing: TC.Space.s) {
                Spacer(minLength: TC.Space.m)
                // Saying no leaves the file with every value it has, which
                // is what this button's own words say.
                Button(copy.harnessPreviewCancel) { model.cancelHarnessPreview() }
                    .keyboardShortcut(.cancelAction)
                // Absent for every outcome that is not committable, so an
                // empty plan can never be confirmed into nothing.
                if HarnessSurface.canCommit(plan, calls: model.harnessCalls) {
                    Button(copy.harnessPreviewConfirm) { model.confirmHarnessPreview() }
                        .keyboardShortcut(.defaultAction)
                        .disabled(model.harnessBusy)
                }
            }
        }
        .padding(TC.Space.xl)
        .frame(minWidth: 460)
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

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.offerTitle).font(TC.Font_.sectionTitle)
            Text(copy.offerWhat).font(TC.Font_.body).fixedSize(horizontal: false, vertical: true)
            Text(copy.offerExposure).font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            Text(copy.offerNoRepoint).font(TC.Font_.body)
                .fixedSize(horizontal: false, vertical: true)
            // `offerAskedOnce` is deliberately NOT shown here. It says "this
            // is the only time you will be asked. The switch stays in
            // Settings", and both halves are false on this surface: the
            // harness gate is `connectNeedsExposure(listenerOn:)`, which is
            // wider than `shouldOffer` on purpose so a connect after the kill
            // switch asks again; and the switch now lives on this destination,
            // with Settings holding only a pointer. It remains true on the
            // first-run offer, which is the sentence's home.
            HStack(spacing: TC.Space.s) {
                Spacer(minLength: TC.Space.m)
                Button(copy.offerDecline) { model.answerHarnessExposure(accepted: false) }
                    .keyboardShortcut(.cancelAction)
                Button(copy.offerAccept) { model.answerHarnessExposure(accepted: true) }
                    .keyboardShortcut(.defaultAction)
                    .disabled(model.harnessBusy)
            }
        }
        .padding(TC.Space.xl)
        .frame(minWidth: 460)
    }
}
