// INTEGRATION: extends account-owned SessionDetailView with the first complete
// correction-to-tested-Agent-Skill flow and a guarded Codex install rollback.

import SwiftUI
import TCBridge

struct SkillLearningView: View {
    let record: HistoryRecord
    let copy: SkillLearningCopy

    @EnvironmentObject private var model: AppModel
    @State private var draft = SkillDraft(name: "", description: "", procedure: "")
    @State private var configuredCandidateID: String?
    @State private var configuredDraft: SkillDraft?
    @State private var draftValidation: SkillDraftValidation?

    private var id: String { record.submissionID }

    var body: some View {
        let state = model.skillLearningState(for: id)
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: copy.heading)
            Text(copy.promise)
                .font(TC.Font_.body)
                .foregroundStyle(TC.inkSecondary)

            stage(for: state)

            if let message = state.failure {
                Text(message)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard(emphasised: state.phase.isReviewedAwaitingEvaluation)
        .onAppear {
            configureDraft()
            model.ensureLocalInstalledSkillStatus(for: record)
        }
        .onChange(of: state.phase.candidate?.candidateID) { _, _ in configureDraft() }
        .onChange(of: state.phase.candidate?.draft) { _, _ in configureDraft() }
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
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
                Button(state.isWorking ? copy.learning : copy.learnAction) {
                    model.learnSkill(from: record)
                }
                .tcPrimaryAction()
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
        return VStack(alignment: .leading, spacing: TC.Space.m) {
            HStack {
                Text(copy.candidateHeading)
                    .font(TC.Font_.sectionTitle)
                Spacer()
                TCTag(text: copy.generatedSource, tone: .clear, symbol: "hammer")
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.name)
                TextField("", text: $draft.name)
                    .textFieldStyle(.roundedBorder)
                    .frame(minHeight: 44)
                    .accessibilityLabel(copy.name)
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                fieldLabel(
                    copy.applicability,
                    count: validation?.descriptionChars,
                    maximum: validation?.descriptionMaxChars
                )
                TextEditor(text: $draft.description)
                    .font(TC.Font_.body)
                    .frame(minHeight: 96)
                    .accessibilityLabel(copy.applicability)
                    .overlay {
                        RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line)
                    }
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                fieldLabel(
                    copy.procedure,
                    count: validation?.procedureChars,
                    maximum: validation?.procedureMaxChars
                )
                TextEditor(text: $draft.procedure)
                    .font(.system(.body, design: .monospaced))
                    .frame(minHeight: 240)
                    .accessibilityLabel(copy.procedure)
                    .overlay {
                        RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line)
                    }
            }

            DisclosureGroup(copy.sourceEvidence) {
                VStack(alignment: .leading, spacing: TC.Space.s) {
                    Text(candidate.sourceCorrection)
                        .font(TC.Font_.body)
                        .textSelection(.enabled)
                    ForEach(candidate.sourceEvidence) { evidence in
                        VStack(alignment: .leading, spacing: TC.Space.xxs) {
                            TCFieldLabel(
                                model.publicRunCopy?.evidenceKindLabel(for: evidence.kind)
                                    ?? model.publicRunCopy?.unrecognizedValue
                                    ?? ""
                            )
                            Text(evidence.excerpt)
                                .font(TC.Font_.footnote)
                                .textSelection(.enabled)
                        }
                    }
                }
                .padding(.top, TC.Space.s)
            }
            .frame(minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
            .accessibilityLabel(copy.sourceEvidence)

            evaluationContract(candidate)

            if let error = validation?.error,
               let message = TCSkillLearning.errorLine(label: error)
            {
                Text(message)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
            }

            Button(working ? copy.reviewing : copy.reviewAction) {
                model.reviewSkill(from: record, draft: draft)
            }
            .tcPrimaryAction()
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
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCFieldLabel(copy.testContract)
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
            .font(TC.Font_.footnote)
            .foregroundStyle(TC.inkSecondary)
            Text(candidate.evaluationContract.fixtureScope)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                TCFieldLabel(copy.manualInstruction)
                Text(candidate.manualControlInstruction)
                    .font(TC.Font_.footnote)
                    .textSelection(.enabled)
            }
        }
        .padding(TC.Space.m)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
    }

    private func fieldLabel(_ label: String, count: Int?, maximum: Int?) -> some View {
        HStack {
            TCFieldLabel(label)
            Spacer()
            Text(count.flatMap { count in maximum.map { "\(count)/\($0)" } } ?? "—")
                .font(TC.Font_.ledger)
                .foregroundStyle(TC.inkTertiary)
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
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.exactPackage)
                .font(TC.Font_.sectionTitle)
            Text(review.skillMD)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(TC.Space.m)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
            HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                TCFieldLabel(copy.digest)
                Text(review.skillSHA256)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }
            Text(copy.evaluationDisclosure)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            Text(
                String(
                    format: copy.reviewBudgetFormat,
                    locale: Locale.current,
                    candidate.evaluationContract.totalRequests,
                    candidate.evaluationContract.outputTokenLimit,
                    candidate.evaluationContract.requiredModelOwner
                )
            )
            .font(TC.Font_.ledger)
            .foregroundStyle(TC.inkSecondary)
            HStack(spacing: TC.Space.s) {
                Button(copy.editSkill) { model.editSkill(from: record) }
                    .buttonStyle(.bordered)
                    .frame(minHeight: 44)
                    .disabled(working)
                Button(working ? copy.testing : copy.approveAndTest) {
                    model.testSkill(from: record)
                }
                .tcPrimaryAction()
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

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            HStack {
                Text(copy.results)
                    .font(TC.Font_.sectionTitle)
                Spacer()
                TCTag(
                    text: report.installAllowed
                        ? copy.passedGate
                        : copy.failedGate,
                    tone: report.installAllowed ? .clear : .attention,
                    symbol: report.installAllowed ? "checkmark.seal" : "exclamationmark.triangle"
                )
            }

            summaryGroup(copy.repositoryPlans, summaries: report.planSummaries)
            summaryGroup(copy.skillApplicability, summaries: report.applicabilitySummaries)

            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                TCFieldLabel(copy.modelAndBudget)
                Text(report.servedModel)
                    .font(TC.Font_.ledger)
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
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.regressions)
                if report.regressions.isEmpty {
                    Text(copy.noRegressions)
                        .font(TC.Font_.footnote)
                        .foregroundStyle(TC.inkSecondary)
                } else {
                    ForEach(report.regressions, id: \.self) { task in
                        Text(task)
                            .font(TC.Font_.ledger)
                            .foregroundStyle(TC.coralText)
                    }
                }
            }

            DisclosureGroup(copy.inspectRuns) {
                VStack(alignment: .leading, spacing: TC.Space.m) {
                    ForEach(report.trials) { trial in
                        SkillTrialRow(trial: trial, copy: copy)
                    }
                }
                .padding(.top, TC.Space.s)
            }
            .frame(minHeight: 44, alignment: .leading)
            .contentShape(Rectangle())
            .accessibilityLabel(copy.inspectRuns)

            HStack(spacing: TC.Space.s) {
                Button(copy.editSkill) { model.editSkill(from: record) }
                    .buttonStyle(.bordered)
                    .frame(minHeight: 44)
                    .disabled(working)
                if report.installAllowed {
                    Button(working ? copy.preparing : copy.reviewInstall) {
                        model.reviewSkillInstall(from: record)
                    }
                    .tcPrimaryAction()
                    .frame(minHeight: 44)
                    .disabled(working)
                }
            }
        }
    }

    private func summaryGroup(_ label: String, summaries: [SkillArmSummary]) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            TCFieldLabel(label)
            ForEach(summaries) { summary in
                HStack {
                    Text(summary.label)
                        .font(TC.Font_.body)
                    Spacer()
                    Text("\(summary.passed)/\(summary.total)")
                        .font(TC.Font_.ledger)
                }
            }
        }
        .padding(TC.Space.m)
        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
    }
}

private struct SkillTrialRow: View {
    let trial: SkillTrialResult
    let copy: SkillLearningCopy

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            HStack(alignment: .firstTextBaseline) {
                Text(trial.taskID)
                    .font(TC.Font_.meta)
                Spacer()
                TCTag(
                    text: armLabel + " · " + (trial.passed ? copy.passed : copy.failed),
                    tone: trial.passed ? .clear : .attention,
                    symbol: trial.passed ? "checkmark" : "xmark"
                )
            }
            Text(trial.task)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            Link(copy.openFixtureSource, destination: trial.sourceURL)
                .font(TC.Font_.footnote)
                .frame(minHeight: 44, alignment: .leading)
                .contentShape(Rectangle())
                .accessibilityLabel("\(copy.openFixtureSource): \(trial.taskID)")
            ForEach(trial.failureReasons, id: \.self) { failure in
                Text(failure)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
            }
            if let answer = trial.answer {
                Text(answer.diagnosis)
                    .font(TC.Font_.footnote)
                if !answer.editPaths.isEmpty {
                    Text(copy.edits + ": " + answer.editPaths.joined(separator: ", "))
                        .font(TC.Font_.ledger)
                        .textSelection(.enabled)
                }
                if !answer.commands.isEmpty {
                    Text(copy.commands + ": " + answer.commands.joined(separator: "\n"))
                        .font(TC.Font_.ledger)
                        .textSelection(.enabled)
                }
                if !answer.verification.isEmpty {
                    Text(copy.checks + ": " + answer.verification.joined(separator: "\n"))
                        .font(TC.Font_.ledger)
                        .textSelection(.enabled)
                }
            }
            DisclosureGroup(copy.modelOutput) {
                Text(trial.rawOutput)
                    .font(.system(.caption, design: .monospaced))
                    .textSelection(.enabled)
                    .padding(.top, TC.Space.xs)
            }
        }
        .padding(.vertical, TC.Space.xs)
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
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.installPreview)
                .font(TC.Font_.sectionTitle)
            Text(copy.installDisclosure)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            TCFieldLabel(copy.persistentWrites)
            labeled(copy.targetPath, plan.targetLocation)
            labeled(copy.codingTool, plan.tool)
            labeled(copy.skillFile, plan.skillLocation)
            labeled(copy.digest, plan.skillSHA256)
            TCFieldLabel(copy.exactPackage)
            Text(plan.skillMD)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(TC.Space.m)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
            labeled(copy.ownershipMarker, plan.markerLocation)
            labeled(copy.markerDigest, plan.markerFileSHA256)
            TCFieldLabel(copy.exactMarker)
            Text(plan.markerJSON)
                .font(.system(.footnote, design: .monospaced))
                .textSelection(.enabled)
                .padding(TC.Space.m)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
            if !plan.canInstall {
                Text(
                    plan.occupied
                        ? TCSkillLearning.errorLine(label: "skill-install-occupied")
                            ?? copy.unavailable
                        : copy.unavailable
                )
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
                HStack(spacing: TC.Space.s) {
                    Button(copy.editSkill) { model.editSkill(from: record) }
                        .buttonStyle(.bordered)
                        .frame(minHeight: 44)
                        .disabled(working)
                    Button(working ? copy.preparing : copy.retryInstall) {
                        model.reviewSkillInstall(from: record)
                    }
                    .tcPrimaryAction()
                    .frame(minHeight: 44)
                    .disabled(working)
                }
            } else {
                Button(working ? copy.installing : copy.installAction) {
                    model.installSkill(from: record)
                }
                .tcPrimaryAction()
                .frame(minHeight: 44)
                .disabled(working || !plan.canInstall)
            }
        }
    }

    private func labeled(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xxs) {
            TCFieldLabel(label)
            Text(value)
                .font(TC.Font_.ledger)
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
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCTag(text: copy.installed, tone: .clear, symbol: "checkmark.seal")
            Text(installed.name)
                .font(TC.Font_.cardTitle)
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                TCFieldLabel(copy.targetPath)
                Text(installed.targetLocation)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                TCFieldLabel(copy.digest)
                Text(installed.skillSHA256)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }
            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                TCFieldLabel(copy.markerDigest)
                Text(installed.markerSHA256)
                    .font(TC.Font_.ledger)
                    .textSelection(.enabled)
            }
            Text(copy.rollbackDisclosure)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
            Button(working ? copy.rollingBack : copy.rollback) {
                model.rollbackSkill(from: record)
            }
            .buttonStyle(.bordered)
            .frame(minHeight: 44)
            .disabled(working)
        }
    }
}
