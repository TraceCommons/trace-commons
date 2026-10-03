// INTEGRATION: extends account-owned SessionDetailView with the first complete
// correction-to-tested-Agent-Skill flow and a guarded Codex install rollback.
// Drawn on TCDesign glass; the Monitor's History inspector shows it for the
// selected row (D-7). Every sentence is the core's (`SkillLearningCopy`).

import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

struct SkillLearningView: View {
    let record: HistoryRecord
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel
    @State private var draft = SkillDraft(name: "", description: "", procedure: "")
    @State private var configuredCandidateID: String?
    @State private var configuredDraft: SkillDraft?
    @State private var draftValidation: SkillDraftValidation?
    @State private var showsEvidence = false
    /// The failure the person dismissed. A failure that arrives clears it,
    /// and so does a new attempt, whose failure is shown even when it reads
    /// the same as the one dismissed.
    @State private var dismissedFailure: String?

    private var id: String { record.submissionID }

    var body: some View {
        let state = model.skillLearningState(for: id)
        GlassEyebrowCard(copy.heading) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                Text(copy.promise)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)

                stage(for: state)

                if let message = state.failure, message != dismissedFailure {
                    GlassNotice(tone: .outside) {
                        HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                            Text(message)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) { dismissedFailure = message }
                                .buttonStyle(GlassButtonStyle(.glass))
                                .frame(minHeight: 44)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onAppear {
            configureDraft()
            model.ensureLocalInstalledSkillStatus(for: record)
        }
        .onChange(of: state.phase.candidate?.candidateID) { _, _ in configureDraft() }
        .onChange(of: state.phase.candidate?.draft) { _, _ in configureDraft() }
        .onChange(of: state.failure) { _, _ in dismissedFailure = nil }
        .onChange(of: state.isWorking) { _, working in if working { dismissedFailure = nil } }
    }

    @ViewBuilder
    private func stage(for state: SkillLearningSessionState) -> some View {
        if let installed = state.installedSkill {
            InstalledSkillPanel(
                record: record,
                installed: installed,
                working: state.isWorking,
                copy: copy
            )
        } else {
            switch state.phase {
            case .idle:
                Text(copy.supportedFamily)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                Button(state.isWorking ? copy.learning : copy.learnAction) {
                    model.learnSkill(from: record)
                }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(minHeight: 44)
                .disabled(state.isWorking)
            case .candidate(let candidate):
                candidateEditor(candidate, working: state.isWorking)
            case .reviewed(let candidate, let review):
                SkillReviewPreview(
                    record: record,
                    candidate: candidate,
                    review: review,
                    working: state.isWorking,
                    copy: copy
                )
            case .evaluated(_, _, let report):
                SkillEvaluationResults(
                    record: record,
                    report: report,
                    working: state.isWorking,
                    copy: copy
                )
            case .planned(_, _, _, let plan):
                SkillInstallPreview(
                    record: record,
                    plan: plan,
                    working: state.isWorking,
                    copy: copy
                )
            }
        }
    }

    private func candidateEditor(_ candidate: SkillCandidate, working: Bool) -> some View {
        let validation = draftValidation
        return VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            HStack {
                Text(copy.candidateHeading)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer()
                GlassTag(copy.generatedSource)
            }

            GlassTextField(copy.name, text: $draft.name)
                .frame(minHeight: 44)

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                fieldLabel(
                    copy.applicability,
                    count: validation?.descriptionChars,
                    maximum: validation?.descriptionMaxChars
                )
                GlassWell {
                    TextEditor(text: $draft.description)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 96)
                        .accessibilityLabel(copy.applicability)
                        .padding(GlassTokens.Space.s4)
                }
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                fieldLabel(
                    copy.procedure,
                    count: validation?.procedureChars,
                    maximum: validation?.procedureMaxChars
                )
                GlassWell {
                    TextEditor(text: $draft.procedure)
                        .glassType(GlassTokens.TypeScale.mono)
                        .foregroundStyle(GlassColor.textPrimary)
                        .scrollContentBackground(.hidden)
                        .frame(minHeight: 240)
                        .accessibilityLabel(copy.procedure)
                        .padding(GlassTokens.Space.s4)
                }
            }

            GlassExpander(copy.sourceEvidence, isOpen: $showsEvidence)
                .frame(minHeight: 44, alignment: .leading)
                .accessibilityLabel(copy.sourceEvidence)
            if showsEvidence {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                    Text(candidate.sourceCorrection)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .textSelection(.enabled)
                    ForEach(candidate.sourceEvidence) { evidence in
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            skillFieldLabel(
                                model.publicRunCopy?.evidenceKindLabel(for: evidence.kind)
                                    ?? model.publicRunCopy?.unrecognizedValue
                                    ?? ""
                            )
                            Text(evidence.excerpt)
                                .glassType(GlassTokens.TypeScale.caption)
                                .foregroundStyle(GlassColor.textSecondary)
                                .textSelection(.enabled)
                        }
                    }
                }
            }

            evaluationContract(candidate)

            if let error = validation?.error,
               let message = TCSkillLearning.errorLine(label: error)
            {
                GlassNotice(tone: .outside) { Text(message).fixedSize(horizontal: false, vertical: true) }
            }

            Button(working ? copy.reviewing : copy.reviewAction) {
                model.reviewSkill(from: record, draft: draft)
            }
            .buttonStyle(GlassButtonStyle(.primary))
            .frame(minHeight: 44)
            .disabled(working || validation?.valid != true)
        }
        .task(id: draft) {
            do {
                try await Task.sleep(for: .milliseconds(150))
            } catch {
                return
            }
            guard !Task.isCancelled,
                  let input = try? JSONEncoder().encode(draft),
                  let json = String(data: input, encoding: .utf8)
            else { return }
            let validated: SkillDraftValidation? = await Task.detached(priority: .utility) {
                () -> SkillDraftValidation? in
                guard let result = TCSkillLearning.validateDraftJSON(json),
                      let data = result.data(using: .utf8)
                else { return nil }
                return try? JSONDecoder().decode(SkillDraftValidation.self, from: data)
            }.value
            guard !Task.isCancelled else { return }
            draftValidation = validated
        }
    }

    private func evaluationContract(_ candidate: SkillCandidate) -> some View {
        GlassWell {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                skillFieldLabel(copy.testContract)
                Text(
                    String(
                        format: copy.contractSummaryFormat,
                        locale: Locale.current,
                        candidate.evaluationContract.taskCount,
                        candidate.evaluationContract.clusterCount,
                        candidate.evaluationContract.totalRequests,
                        candidate.evaluationContract.outputTokenLimit,
                        candidate.evaluationContract.requestTimeoutSeconds
                    )
                )
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
                Text(candidate.evaluationContract.fixtureScope)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                    skillFieldLabel(copy.manualInstruction)
                    Text(candidate.manualControlInstruction)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textPrimary)
                        .textSelection(.enabled)
                }
            }
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }

    private func fieldLabel(_ label: String, count: Int?, maximum: Int?) -> some View {
        HStack {
            skillFieldLabel(label)
            Spacer()
            Text(count.flatMap { count in maximum.map { "\(count)/\($0)" } } ?? "—")
                .glassType(GlassTokens.TypeScale.mono)
                .foregroundStyle(GlassColor.textTertiary)
        }
    }

    private func configureDraft() {
        guard let candidate = model.skillLearningState(for: id).phase.candidate,
              configuredCandidateID != candidate.candidateID
                || configuredDraft != candidate.draft
        else { return }
        configuredCandidateID = candidate.candidateID
        configuredDraft = candidate.draft
        draft = candidate.draft
        draftValidation = nil
    }

}

private struct SkillReviewPreview: View {
    let record: HistoryRecord
    let candidate: SkillCandidate
    let review: SkillReview
    let working: Bool
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(copy.exactPackage)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            skillCodeBlock(review.skillMD)
            HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                skillFieldLabel(copy.digest)
                Text(review.skillSHA256)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
            }
            Text(copy.evaluationDisclosure)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Text(
                String(
                    format: copy.reviewBudgetFormat,
                    locale: Locale.current,
                    candidate.evaluationContract.totalRequests,
                    candidate.evaluationContract.outputTokenLimit,
                    candidate.evaluationContract.requiredModelOwner
                )
            )
            .glassType(GlassTokens.TypeScale.mono)
            .foregroundStyle(GlassColor.textSecondary)
            .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: GlassTokens.Space.s4) {
                Button(copy.editSkill) { model.editSkill(from: record) }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .frame(minHeight: 44)
                    .disabled(working)
                Button(working ? copy.testing : copy.approveAndTest) {
                    model.testSkill(from: record)
                }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(minHeight: 44)
                .disabled(working)
            }
        }
    }
}

private struct SkillEvaluationResults: View {
    let record: HistoryRecord
    let report: SkillEvaluationReport
    let working: Bool
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel
    @State private var showsRuns = false

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            HStack {
                Text(copy.results)
                    .glassType(GlassTokens.TypeScale.bodyStrong)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer()
                GlassTag(
                    report.installAllowed ? copy.passedGate : copy.failedGate,
                    tone: report.installAllowed ? .on : .failed
                )
            }

            summaryGroup(copy.repositoryPlans, summaries: report.planSummaries)
            summaryGroup(copy.skillApplicability, summaries: report.applicabilitySummaries)

            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                skillFieldLabel(copy.modelAndBudget)
                Text(report.servedModel)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
                Text(
                    String(
                        format: copy.modelBudgetFormat,
                        locale: Locale.current,
                        report.modelOwnedBy,
                        report.taskCount,
                        report.outputTokenLimit
                    )
                )
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                skillFieldLabel(copy.regressions)
                if report.regressions.isEmpty {
                    Text(copy.noRegressions)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                } else {
                    ForEach(report.regressions, id: \.self) { task in
                        Text(task)
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassTokens.Color.statusOutsideText.color)
                    }
                }
            }

            GlassExpander(copy.inspectRuns, isOpen: $showsRuns)
                .frame(minHeight: 44, alignment: .leading)
                .accessibilityLabel(copy.inspectRuns)
            if showsRuns {
                VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                    ForEach(report.trials) { trial in
                        SkillTrialRow(trial: trial, copy: copy)
                    }
                }
            }

            HStack(spacing: GlassTokens.Space.s4) {
                Button(copy.editSkill) { model.editSkill(from: record) }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .frame(minHeight: 44)
                    .disabled(working)
                if report.installAllowed {
                    Button(working ? copy.preparing : copy.reviewInstall) {
                        model.reviewSkillInstall(from: record)
                    }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .frame(minHeight: 44)
                    .disabled(working)
                }
            }
        }
    }

    private func summaryGroup(_ label: String, summaries: [SkillArmSummary]) -> some View {
        GlassWell {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                skillFieldLabel(label)
                ForEach(summaries) { summary in
                    HStack {
                        Text(summary.label)
                            .glassType(GlassTokens.TypeScale.body)
                            .foregroundStyle(GlassColor.textPrimary)
                        Spacer()
                        Text("\(summary.passed)/\(summary.total)")
                            .glassType(GlassTokens.TypeScale.mono)
                            .foregroundStyle(GlassColor.textPrimary)
                    }
                }
            }
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

private struct SkillTrialRow: View {
    let trial: SkillTrialResult
    let copy: SkillLearningCopy

    @State private var showsOutput = false

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            HStack(alignment: .firstTextBaseline) {
                Text(trial.taskID)
                    .glassType(GlassTokens.TypeScale.label)
                    .foregroundStyle(GlassColor.textPrimary)
                Spacer()
                GlassTag(
                    armLabel + " · " + (trial.passed ? copy.passed : copy.failed),
                    tone: trial.passed ? .on : .failed
                )
            }
            Text(trial.task)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Link(copy.openFixtureSource, destination: trial.sourceURL)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.accentText)
                .frame(minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
                .accessibilityLabel("\(copy.openFixtureSource): \(trial.taskID)")
            ForEach(trial.failureReasons, id: \.self) { failure in
                Text(failure)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassTokens.Color.statusOutsideText.color)
                    .fixedSize(horizontal: false, vertical: true)
            }
            if let answer = trial.answer {
                Text(answer.diagnosis)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textPrimary)
                    .fixedSize(horizontal: false, vertical: true)
                if !answer.editPaths.isEmpty {
                    Text(copy.edits + ": " + answer.editPaths.joined(separator: ", "))
                        .glassType(GlassTokens.TypeScale.mono)
                        .foregroundStyle(GlassColor.textSecondary)
                        .textSelection(.enabled)
                }
                if !answer.commands.isEmpty {
                    Text(copy.commands + ": " + answer.commands.joined(separator: "\n"))
                        .glassType(GlassTokens.TypeScale.mono)
                        .foregroundStyle(GlassColor.textSecondary)
                        .textSelection(.enabled)
                }
                if !answer.verification.isEmpty {
                    Text(copy.checks + ": " + answer.verification.joined(separator: "\n"))
                        .glassType(GlassTokens.TypeScale.mono)
                        .foregroundStyle(GlassColor.textSecondary)
                        .textSelection(.enabled)
                }
            }
            GlassExpander(copy.modelOutput, isOpen: $showsOutput)
                .frame(minHeight: 44, alignment: .leading)
            if showsOutput {
                skillCodeBlock(trial.rawOutput)
            }
        }
        .padding(.vertical, GlassTokens.Space.s2)
    }

    private var armLabel: String {
        switch trial.arm {
        case .baseline: return copy.baseline
        case .manualInstruction: return copy.simpleInstruction
        case .candidateSkill: return copy.candidateSkill
        }
    }
}

private struct SkillInstallPreview: View {
    let record: HistoryRecord
    let plan: SkillInstallPlan
    let working: Bool
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(copy.installPreview)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            Text(copy.installDisclosure)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            skillFieldLabel(copy.persistentWrites)
            labeled(copy.targetPath, plan.targetLocation)
            labeled(copy.codingTool, plan.tool)
            labeled(copy.skillFile, plan.skillLocation)
            labeled(copy.digest, plan.skillSHA256)
            skillFieldLabel(copy.exactPackage)
            skillCodeBlock(plan.skillMD)
            labeled(copy.ownershipMarker, plan.markerLocation)
            labeled(copy.markerDigest, plan.markerFileSHA256)
            skillFieldLabel(copy.exactMarker)
            skillCodeBlock(plan.markerJSON)
            if !plan.canInstall {
                GlassNotice(tone: .outside) {
                    Text(
                        plan.occupied
                            ? TCSkillLearning.errorLine(label: "skill-install-occupied")
                                ?? copy.unavailable
                            : copy.unavailable
                    )
                    .fixedSize(horizontal: false, vertical: true)
                }
                HStack(spacing: GlassTokens.Space.s4) {
                    Button(copy.editSkill) { model.editSkill(from: record) }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .frame(minHeight: 44)
                        .disabled(working)
                    Button(working ? copy.preparing : copy.retryInstall) {
                        model.reviewSkillInstall(from: record)
                    }
                    .buttonStyle(GlassButtonStyle(.primary))
                    .frame(minHeight: 44)
                    .disabled(working)
                }
            } else {
                Button(working ? copy.installing : copy.installAction) {
                    model.installSkill(from: record)
                }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(minHeight: 44)
                .disabled(working || !plan.canInstall)
            }
        }
    }

    private func labeled(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            skillFieldLabel(label)
            Text(value)
                .glassType(GlassTokens.TypeScale.mono)
                .foregroundStyle(GlassColor.textPrimary)
                .textSelection(.enabled)
        }
    }
}

private struct InstalledSkillPanel: View {
    let record: HistoryRecord
    let installed: InstalledSkill
    let working: Bool
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            GlassTag(copy.installed, tone: .on)
            Text(installed.name)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                skillFieldLabel(copy.targetPath)
                Text(installed.targetLocation)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                skillFieldLabel(copy.digest)
                Text(installed.skillSHA256)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                skillFieldLabel(copy.markerDigest)
                Text(installed.markerSHA256)
                    .glassType(GlassTokens.TypeScale.mono)
                    .foregroundStyle(GlassColor.textPrimary)
                    .textSelection(.enabled)
            }
            Text(copy.rollbackDisclosure)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            Button(working ? copy.rollingBack : copy.rollback) {
                model.rollbackSkill(from: record)
            }
            .buttonStyle(GlassButtonStyle(.glass))
            .frame(minHeight: 44)
            .disabled(working)
        }
    }
}

/// A field's eyebrow label, as `GlassTextField` draws its own.
private func skillFieldLabel(_ label: String) -> some View {
    Text(label)
        .glassType(GlassTokens.TypeScale.eyebrow)
        .foregroundStyle(GlassColor.textTertiary)
}

/// When a contribution offers the Skills flow, one rule for every surface
/// that shows it (the legacy session detail and the Monitor's History
/// inspector). Learning is offered only on an accepted contribution whose
/// task outcome is known, which is not withdrawn or closed, and which
/// carries a human correction; an absent detail offers nothing. An
/// installed skill (its rollback) shows outside that rule.
enum SkillLearningGate {
    /// Closed by status (an unknown status reads closed), or withdrawn from
    /// this device.
    static func withdrawalCompleted(recordStatus: String, withdrawal: AppModel.WithdrawalResult?) -> Bool {
        if ContributionStatusPresentation.isTerminal(recordStatus) { return true }
        if case .some(.withdrawn) = withdrawal { return true }
        return false
    }

    static func contributionIsWithdrawn(_ detail: SessionDetail, recordStatus: String, withdrawalCompleted: Bool) -> Bool {
        withdrawalCompleted || ContributionStatusPresentation.isTerminal(detail.contributionStatus ?? recordStatus)
    }

    static func contributionIsActive(_ detail: SessionDetail, recordStatus: String, withdrawalCompleted: Bool) -> Bool {
        detail.accepted
            && detail.taskSuccess != nil
            && !contributionIsWithdrawn(detail, recordStatus: recordStatus, withdrawalCompleted: withdrawalCompleted)
    }

    /// Learn, review, test and install.
    static func offersLearning(_ detail: SessionDetail?, recordStatus: String, withdrawalCompleted: Bool) -> Bool {
        guard let detail else { return false }
        return contributionIsActive(detail, recordStatus: recordStatus, withdrawalCompleted: withdrawalCompleted)
            && detail.humanCorrection != nil
    }

    /// Whether the installed-skill surface (and its status retry) may show:
    /// whenever the regular flow is not the one on screen, including before
    /// the detail answers.
    static func showsInstalledSurface(_ detail: SessionDetail?, withdrawalCompleted: Bool) -> Bool {
        let regularSurfaceIsVisible = detail?.accepted == true
            && detail?.humanCorrection != nil
            && !withdrawalCompleted
            && !ContributionStatusPresentation.isTerminal(detail?.contributionStatus)
        return !regularSurfaceIsVisible
    }

    /// The installed-skill status read failed on a closed or unaccepted
    /// contribution with nothing installed: offer to read it again.
    static func offersInstallStatusRetry(
        _ state: SkillLearningSessionState, detail: SessionDetail?, recordStatus: String
    ) -> Bool {
        guard state.installedSkill == nil, case .idle = state.phase, state.failure != nil else { return false }
        return ContributionStatusPresentation.isTerminal(recordStatus) || detail?.accepted == false
    }
}

/// A package, marker or model output shown verbatim in an inset well.
private func skillCodeBlock(_ text: String) -> some View {
    GlassWell {
        Text(text)
            .glassType(GlassTokens.TypeScale.mono)
            .foregroundStyle(GlassColor.textPrimary)
            .textSelection(.enabled)
            .padding(GlassTokens.Space.s6)
            .frame(maxWidth: .infinity, alignment: .leading)
    }
}
