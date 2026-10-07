// INTEGRATION: rendered by SessionDetailView from the account-authenticated,
// permanently redacted session-detail record; withdrawal uses the same
// account action and canonical tier copy as History.

import SwiftUI
import TCDesign

struct SessionContributionOverview: View {
    let record: HistoryRecord
    let detail: SessionDetail
    let copy: PublicRunCopy

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if detail.contentUnavailable == true {
                GlassNotice(tone: .off) {
                    Text(copy.contentUnavailable)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
            taskAndOutcome
            correction
            evidence
            contributionDetails
        }
    }

    private var taskAndOutcome: some View {
        GlassEyebrowCard(copy.task) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                value(detail.task, missing: copy.noTask)
                    .textSelection(.enabled)
                eyebrow(copy.outcome)
                let taskOutcome = copy.taskOutcomeLabel(for: detail.taskSuccess)
                Text(taskOutcome ?? missingContentValue(copy.outcomeUnavailable))
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(taskOutcome == nil ? GlassColor.textSecondary : GlassColor.textPrimary)
                if let feedbackLine = copy.feedbackLabel(for: detail.userFeedback) {
                    Text(feedbackLine)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
            }
        }
    }

    private var correction: some View {
        GlassEyebrowCard(copy.decisiveCorrection) {
            value(detail.humanCorrection, missing: copy.noCorrection)
                .textSelection(.enabled)
        }
    }

    private var evidence: some View {
        GlassEyebrowCard(copy.supportingEvidence) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(copy.observedInVersion)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                if detail.evidence.isEmpty {
                    value(nil, missing: copy.noEvidence)
                } else {
                    ForEach(detail.evidence) { item in
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            eyebrow(copy.evidenceKindLabel(for: item.kind))
                            Text(item.excerpt)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textPrimary)
                                .textSelection(.enabled)
                        }
                        .padding(.vertical, GlassTokens.Space.s2)
                    }
                }
            }
        }
    }

    private var contributionDetails: some View {
        GlassEyebrowCard(copy.contributionDetails) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                GlassKeyValueList([
                    .init(copy.processingStatus,
                          copy.historyStatusLabel(for: detail.contributionStatus ?? record.status)),
                ])
                VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                    Text(copy.permittedUses)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                    if let uses = detail.permittedUses, !uses.isEmpty {
                        ForEach(uses, id: \.self) { use in
                            GlassStatusLabel(copy.permittedUseLabel(for: use), status: .on)
                        }
                    } else if detail.permittedUses != nil {
                        Text(copy.noPermittedUses)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    } else {
                        Text(copy.permittedUsesUnavailable)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textSecondary)
                    }
                }
                eyebrow(copy.contributedVersion)
                GlassKeyValueList([
                    .init(copy.envelopeVersion, detail.contributedVersion, mono: true),
                    .init(copy.consentPolicyVersion, detail.consentPolicyVersion, mono: true),
                    .init(copy.redactionVersion, detail.redactionPipelineVersion, mono: true),
                ])
            }
        }
    }

    /// A value in body type, or the missing-value word in the secondary
    /// colour.
    private func value(_ text: String?, missing: String) -> some View {
        Text(text ?? missingContentValue(missing))
            .glassType(GlassTokens.TypeScale.body)
            .foregroundStyle(text == nil ? GlassColor.textSecondary : GlassColor.textPrimary)
    }

    private func eyebrow(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.eyebrow)
            .foregroundStyle(GlassColor.textTertiary)
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
            GlassEyebrowCard(copy.nextAction) {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    if let result = model.withdrawals[record.submissionID] {
                        WithdrawalOutcomeView(result: result)
                        if shouldOfferRetry(result) {
                            Button(WithdrawalCopy.tryAgain) { model.withdraw(record) }
                                .buttonStyle(GlassButtonStyle(.glass))
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
                            .buttonStyle(GlassButtonStyle(.glass))
                            .frame(minHeight: 44)
                    }
                }
            }
        }
    }

    private var isWithdrawable: Bool {
        ContributionStatusPresentation.offersWithdraw(currentStatus)
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
        let (text, status): (String, GlassStatus) = {
            switch result {
            case .withdrawn(let reach, let note):
                return ([WithdrawalCopy.resultSentence(reach), note].compactMap { $0 }.joined(separator: "\n"), .off)
            case .noAccountSession:
                return (WithdrawalCopy.accountSessionRequired, .ask)
            case .failed(let label):
                return (WithdrawalCopy.failureSentence(label: label), .ask)
            }
        }()
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            // #1146 heads a completed withdrawal "Withdrawn by you".
            if case .withdrawn = result {
                Text(WithdrawalCopy.resultHeading)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
            }
            GlassStatusLabel(text, status: status)
                .fixedSize(horizontal: false, vertical: true)
        }
    }
}

/// The withdrawal confirmation as #1146's `WithdrawalControl` raises it:
/// a glass modal over the window titled "Confirm withdrawal" with its
/// description, the consequences in the body, and Keep it / Confirm
/// withdrawal in the footer, the confirm in the outside ink. Closing it
/// while the withdrawal is in flight is ignored, so its outcome is seen.
struct WithdrawalConfirmationModal: View {
    let status: String
    let keepLabel: String
    let inFlight: Bool
    let onKeep: () -> Void
    let onConfirm: () -> Void

    var body: some View {
        let confirmation = WithdrawalCopy.confirmation(for: .init(status: status))
        let keep = { if !inFlight { onKeep() } }
        GlassModal(
            title: confirmation?.question ?? WithdrawalCopy.disclosureUnavailable,
            subtitle: confirmation?.description,
            actions: Self.actions(confirmation, keepLabel: keepLabel, inFlight: inFlight, keep: keep, confirm: onConfirm),
            onCancel: keep
        ) {
            GlassModalBody(spacing: GlassTokens.Space.s4) {
                if let confirmation {
                    if let ambiguity = confirmation.ambiguity {
                        Text(ambiguity)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    ForEach(Array(confirmation.bodies.enumerated()), id: \.offset) { index, body in
                        GlassStatusLabel(body, status: index == confirmation.gravest ? .outside : .off)
                            .fontWeight(index == confirmation.gravest ? .semibold : nil)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                    if let credit = confirmation.credit {
                        Text(credit)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textSecondary)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                } else if !WithdrawalCopy.disclosureUnavailable.isEmpty {
                    // Not confirmable without the core's words: #1146's line
                    // says why, and only the way back is offered.
                    GlassStatusLabel(WithdrawalCopy.disclosureUnavailable, status: .outside)
                        .fixedSize(horizontal: false, vertical: true)
                }
            }
        }
    }

    /// Keep it, then Confirm withdrawal (destructive, never on Return, and
    /// disarmed while in flight); without the core's words, Keep it only.
    static func actions(
        _ confirmation: WithdrawalCopy.Confirmation?, keepLabel: String, inFlight: Bool,
        keep: @escaping () -> Void, confirm: @escaping () -> Void
    ) -> [GlassModalAction] {
        var actions = [GlassModalAction(keepLabel, role: .cancel, isEnabled: !inFlight, action: keep)]
        if let confirmation {
            actions.append(.destructive(
                inFlight ? confirmation.busyLabel : confirmation.confirmLabel, isEnabled: !inFlight, action: confirm))
        }
        return actions
    }
}

struct WithdrawalConfirmationView: View {
    let status: String
    let keepLabel: String
    let inFlight: Bool
    let onKeep: () -> Void
    let onConfirm: () -> Void

    var body: some View {
        if let confirmation = WithdrawalCopy.confirmation(for: .init(status: status)) {
            confirmationBody(confirmation)
        } else {
            // Not confirmable without the core's words; the way back stays,
            // and #1146's line says why withdrawal is not offered.
            if !WithdrawalCopy.disclosureUnavailable.isEmpty {
                GlassStatusLabel(WithdrawalCopy.disclosureUnavailable, status: .outside)
                    .fixedSize(horizontal: false, vertical: true)
            }
            Button(keepLabel, action: onKeep)
                .buttonStyle(GlassButtonStyle(.glass))
                .keyboardShortcut(.cancelAction)
                .frame(minHeight: 44)
        }
    }

    private func confirmationBody(_ confirmation: WithdrawalCopy.Confirmation) -> some View {
        GlassWell {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                Text(confirmation.question)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Text(confirmation.description)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textTertiary)
                    .fixedSize(horizontal: false, vertical: true)
                if let ambiguity = confirmation.ambiguity {
                    Text(ambiguity)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                        .fixedSize(horizontal: false, vertical: true)
                }
                ForEach(Array(confirmation.bodies.enumerated()), id: \.offset) { index, body in
                    GlassStatusLabel(body, status: index == confirmation.gravest ? .outside : .off)
                        .fontWeight(index == confirmation.gravest ? .semibold : nil)
                        .fixedSize(horizontal: false, vertical: true)
                }
                if let credit = confirmation.credit {
                    Text(credit)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    Button(keepLabel, action: onKeep)
                        .buttonStyle(GlassButtonStyle(.glass))
                        .keyboardShortcut(.cancelAction)
                        .frame(minHeight: 44)
                    // #1146: a glass button in the outside ink, never the
                    // accent fill: withdrawing cannot be taken back.
                    Button(inFlight ? confirmation.busyLabel : confirmation.confirmLabel, action: onConfirm)
                        .buttonStyle(GlassButtonStyle(.destructive))
                        .frame(minHeight: 44)
                        .disabled(inFlight)
                }
            }
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}
