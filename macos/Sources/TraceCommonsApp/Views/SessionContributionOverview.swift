// INTEGRATION: rendered by SessionDetailView from the account-authenticated,
// permanently redacted session-detail record; withdrawal uses the same
// account action and canonical tier copy as History.

import SwiftUI

struct SessionContributionOverview: View {
    let record: HistoryRecord
    let detail: SessionDetail
    let copy: PublicRunCopy

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.l) {
            if detail.contentUnavailable == true {
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                    Image(systemName: "info.circle")
                        .imageScale(.small)
                        .accessibilityHidden(true)
                    Text(copy.contentUnavailable)
                        .fixedSize(horizontal: false, vertical: true)
                }
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
                .padding(TC.Space.m)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(
                    TC.surfaceInset,
                    in: RoundedRectangle(cornerRadius: TC.Radius.inset)
                )
            }
            taskAndOutcome
            correction
            evidence
            contributionDetails
        }
    }

    private var taskAndOutcome: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCFieldLabel(copy.task)
            Text(detail.task ?? missingContentValue(copy.noTask))
                .font(TC.Font_.body)
                .foregroundStyle(detail.task == nil ? TC.inkSecondary : TC.inkPrimary)
                .textSelection(.enabled)
            Divider()
            TCFieldLabel(copy.outcome)
            let taskOutcome = copy.taskOutcomeLabel(for: detail.taskSuccess)
            Text(taskOutcome ?? missingContentValue(copy.outcomeUnavailable))
                .font(TC.Font_.cardTitle)
                .foregroundStyle(taskOutcome == nil ? TC.inkSecondary : TC.inkPrimary)
            if let feedbackLine = copy.feedbackLabel(for: detail.userFeedback) {
                Text(feedbackLine)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private var correction: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCFieldLabel(copy.decisiveCorrection)
            Text(detail.humanCorrection ?? missingContentValue(copy.noCorrection))
                .font(TC.Font_.body)
                .foregroundStyle(detail.humanCorrection == nil ? TC.inkSecondary : TC.inkPrimary)
                .textSelection(.enabled)
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private var evidence: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCFieldLabel(copy.supportingEvidence)
            Text(copy.observedInVersion)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            if detail.evidence.isEmpty {
                Text(missingContentValue(copy.noEvidence))
                    .font(TC.Font_.body)
                    .foregroundStyle(TC.inkSecondary)
            } else {
                ForEach(detail.evidence) { item in
                    VStack(alignment: .leading, spacing: TC.Space.xxs) {
                        Text(copy.evidenceKindLabel(for: item.kind).uppercased())
                            .font(TC.Font_.fieldLabel)
                            .tracking(TC.Font_.Tracking.eyebrow)
                            .foregroundStyle(TC.inkTertiary)
                        Text(item.excerpt)
                            .font(TC.Font_.footnote)
                            .textSelection(.enabled)
                    }
                    .padding(.vertical, TC.Space.xs)
                }
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private var contributionDetails: some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCFieldLabel(copy.contributionDetails)
            labelledValue(
                copy.processingStatus,
                copy.contributionStatusLabel(for: detail.contributionStatus ?? record.status)
            )
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                Text(copy.permittedUses)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
                if let uses = detail.permittedUses, !uses.isEmpty {
                    ForEach(uses, id: \.self) { use in
                        HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                            Image(systemName: "checkmark.circle")
                                .imageScale(.small)
                                .accessibilityHidden(true)
                            Text(copy.permittedUseLabel(for: use))
                        }
                        .font(TC.Font_.footnote)
                    }
                } else if detail.permittedUses != nil {
                    Text(copy.noPermittedUses)
                        .font(TC.Font_.footnote)
                        .foregroundStyle(TC.inkSecondary)
                } else {
                    Text(copy.permittedUsesUnavailable)
                        .font(TC.Font_.footnote)
                        .foregroundStyle(TC.inkSecondary)
                }
            }
            Divider()
            TCFieldLabel(copy.contributedVersion)
            versionLine(copy.envelopeVersion, detail.contributedVersion)
            versionLine(copy.consentPolicyVersion, detail.consentPolicyVersion)
            versionLine(copy.redactionVersion, detail.redactionPipelineVersion)
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard()
    }

    private func labelledValue(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Text(label)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
                .frame(width: 112, alignment: .leading)
            Text(value)
                .font(TC.Font_.body.weight(.semibold))
        }
    }

    private func versionLine(_ label: String, _ value: String) -> some View {
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
            Text(label)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
                .frame(width: 112, alignment: .leading)
            Text(value)
                .font(TC.Font_.ledger)
                .textSelection(.enabled)
        }
    }

    private func missingContentValue(_ ordinaryFallback: String) -> String {
        detail.contentUnavailable == true ? copy.unavailableValue : ordinaryFallback
    }
}

struct SessionWithdrawalAction: View {
    let record: HistoryRecord
    let currentStatus: String
    let copy: PublicRunCopy

    @EnvironmentObject private var model: AppModel
    @State private var confirming = false

    var body: some View {
        if isWithdrawable || model.withdrawals[record.submissionID] != nil {
            VStack(alignment: .leading, spacing: TC.Space.s) {
                TCFieldLabel(copy.nextAction)
                if let result = model.withdrawals[record.submissionID] {
                    WithdrawalOutcomeView(result: result)
                    if shouldOfferRetry(result) {
                        Button(copy.withdraw) { model.withdraw(record) }
                            .buttonStyle(.bordered)
                            .frame(minHeight: 44)
                    }
                } else if confirming {
                    WithdrawalConfirmationView(
                        status: currentStatus,
                        keepLabel: copy.keepContribution,
                        inFlight: model.withdrawing.contains(record.submissionID),
                        onKeep: { confirming = false },
                        onConfirm: { model.withdraw(record) }
                    )
                } else {
                    Button(copy.withdraw) { confirming = true }
                        .buttonStyle(.bordered)
                        .frame(minHeight: 44)
                }
            }
            .padding(TC.Space.l)
            .frame(maxWidth: .infinity, alignment: .leading)
            .tcCard()
        }
    }

    private var isWithdrawable: Bool {
        !ContributionStatusPresentation.isTerminal(currentStatus)
    }

    private func shouldOfferRetry(_ result: AppModel.WithdrawalResult) -> Bool {
        guard isWithdrawable else { return false }
        if case .withdrawn = result { return false }
        return true
    }

}

struct WithdrawalOutcomeView: View {
    let result: AppModel.WithdrawalResult

    var body: some View {
        let (text, tone): (String, TC.Tone) = {
            switch result {
            case .withdrawn(let reach, let note):
                return ([WithdrawalCopy.resultSentence(reach), note].compactMap { $0 }.joined(separator: "\n"), .refused)
            case .noAccountSession:
                return (WithdrawalCopy.accountSessionRequired, .attention)
            case .failed(let label):
                return (WithdrawalCopy.failureSentence(label: label), .attention)
            }
        }()
        HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
            Image(systemName: tone.symbol)
                .imageScale(.small)
                .accessibilityHidden(true)
            Text(text).fixedSize(horizontal: false, vertical: true)
        }
        .font(TC.Font_.footnote)
        .foregroundStyle(tone.textColor)
        .frame(maxWidth: TC.Measure.prose, alignment: .leading)
    }
}

struct WithdrawalConfirmationView: View {
    let status: String
    let keepLabel: String
    let inFlight: Bool
    let onKeep: () -> Void
    let onConfirm: () -> Void

    var body: some View {
        let confirmation = WithdrawalCopy.confirmation(for: .init(status: status))
        VStack(alignment: .leading, spacing: TC.Space.s) {
            Text(confirmation.question).font(TC.Font_.cardTitle)
            if let ambiguity = confirmation.ambiguity {
                Text(ambiguity)
                    .font(TC.Font_.footnote)
                    .fixedSize(horizontal: false, vertical: true)
            }
            ForEach(Array(confirmation.bodies.enumerated()), id: \.offset) { index, body in
                let gravest = index == confirmation.gravest
                HStack(alignment: .firstTextBaseline, spacing: TC.Space.xs) {
                    Image(systemName: gravest ? "exclamationmark.triangle" : "info.circle")
                        .imageScale(.small)
                        .accessibilityHidden(true)
                    Text(body).fixedSize(horizontal: false, vertical: true)
                }
                .font(TC.Font_.footnote)
                .foregroundStyle(gravest ? AnyShapeStyle(TC.coralText) : AnyShapeStyle(.primary))
            }
            Text(confirmation.credit)
                .font(TC.Font_.footnote)
                .foregroundStyle(.secondary)
            HStack(spacing: TC.Space.s) {
                Button(keepLabel, action: onKeep)
                    .keyboardShortcut(.cancelAction)
                    .frame(minHeight: 44)
                Button(inFlight ? "Withdrawing..." : confirmation.confirmLabel, action: onConfirm)
                    .tcPrimaryAction()
                    .frame(minHeight: 44)
                    .disabled(inFlight)
            }
            .font(TC.Font_.footnote)
        }
        .frame(maxWidth: TC.Measure.prose, alignment: .leading)
        .padding(TC.Space.m)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
    }
}
