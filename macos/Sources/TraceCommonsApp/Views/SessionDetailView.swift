// INTEGRATION: reached from HistoryView and backed only by the account-authenticated
// history_detail / publish_public_run / unpublish_public_run daemon methods.

import AppKit
import SwiftUI
import TCBridge

struct SessionDetailView: View {
    let record: HistoryRecord
    let onBack: () -> Void

    @EnvironmentObject private var model: AppModel

    var body: some View {
        Group {
            if let copy = model.publicRunCopy {
                content(copy)
            }
        }
        .onAppear {
            model.loadSessionDetail(record)
            if ContributionStatusPresentation.isTerminal(record.status) {
                model.ensureLocalInstalledSkillStatus(for: record)
            }
        }
        .onChange(of: record.status) { _, status in
            if ContributionStatusPresentation.isTerminal(status) {
                model.ensureLocalInstalledSkillStatus(for: record)
            }
        }
        .onChange(of: model.sessionDetails[record.submissionID]?.accepted) { _, accepted in
            if accepted == false {
                model.ensureLocalInstalledSkillStatus(for: record)
            }
        }
        .onReceive(NotificationCenter.default.publisher(for: NSApplication.didBecomeActiveNotification)) { _ in
            model.loadSessionDetail(record)
        }
    }

    @ViewBuilder
    private func content(_ copy: PublicRunCopy) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.l) {
            Button(action: onBack) {
                HStack(spacing: TC.Space.xs) {
                    Image(systemName: "chevron.left")
                    Text(copy.allContributions)
                }
                .font(TC.Font_.meta)
            }
            .buttonStyle(.plain)
            .frame(minHeight: 44)

            VStack(alignment: .leading, spacing: TC.Space.xxs) {
                Text(copy.sessionDetail)
                    .font(TC.Font_.sectionTitle)
                    .foregroundStyle(TC.inkPrimary)
                Text("\(record.projectLabel) · \(Format.when(record.submittedAt))")
                    .font(TC.Font_.meta)
                    .foregroundStyle(TC.inkSecondary)
            }

            if let detail = model.sessionDetails[record.submissionID] {
                detailContent(detail, copy: copy)
            } else if model.loadingSessionDetails.contains(record.submissionID) {
                Text(copy.readingRecord)
                    .font(TC.Font_.body)
                    .foregroundStyle(TC.inkSecondary)
            } else if let message = model.sessionDetailErrors[record.submissionID] {
                VStack(alignment: .leading, spacing: TC.Space.s) {
                    Text(message)
                        .font(TC.Font_.body)
                        .foregroundStyle(TC.coralText)
                    Button(copy.retryRead) { model.loadSessionDetail(record) }
                        .buttonStyle(.bordered)
                        .frame(minHeight: 44)
                }
            }

            localInstalledSkillSurface(copy)
        }
    }

    @ViewBuilder
    private func localInstalledSkillSurface(_ copy: PublicRunCopy) -> some View {
        let state = model.skillLearningState(for: record.submissionID)
        let detail = model.sessionDetails[record.submissionID]
        let regularSurfaceIsVisible = detail?.accepted == true
            && detail?.humanCorrection != nil
            && !withdrawalCompleted
            && !ContributionStatusPresentation.isTerminal(detail?.contributionStatus)
        if !regularSurfaceIsVisible {
            if state.installedSkill != nil {
                if let skillCopy = model.skillLearningCopy {
                    SkillLearningView(record: record, copy: skillCopy)
                }
            } else if case .idle = state.phase,
                      state.failure != nil,
                      ContributionStatusPresentation.isTerminal(record.status)
                        || detail?.accepted == false
            {
                VStack(alignment: .leading, spacing: TC.Space.s) {
                    if let message = state.failure {
                        Text(message)
                            .font(TC.Font_.footnote)
                            .foregroundStyle(TC.coralText)
                    }
                    Button(copy.retryRead) {
                        model.ensureLocalInstalledSkillStatus(for: record)
                    }
                    .buttonStyle(.bordered)
                    .frame(minHeight: 44)
                    .disabled(state.isWorking)
                }
                .padding(TC.Space.l)
                .frame(maxWidth: .infinity, alignment: .leading)
                .tcCard()
            }
        }
    }

    @ViewBuilder
    private func detailContent(_ detail: SessionDetail, copy: PublicRunCopy) -> some View {
        let currentStatus = detail.contributionStatus ?? record.status
        let contributionIsWithdrawn = withdrawalCompleted
            || ContributionStatusPresentation.isTerminal(currentStatus)
        let contributionIsActive = detail.accepted == true
            && detail.taskSuccess != nil
            && !contributionIsWithdrawn

        SessionContributionOverview(record: record, detail: detail, copy: copy)

        if contributionIsActive,
           detail.humanCorrection != nil,
           let skillCopy = model.skillLearningCopy {
            SkillLearningView(record: record, copy: skillCopy)
        }

        if contributionIsActive {
            PublicRunEditor(record: record, detail: detail, copy: copy)
        } else if !contributionIsWithdrawn {
            Text(copy.publicationAfterAcceptance)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)
        }

        SessionWithdrawalAction(record: record, currentStatus: currentStatus, copy: copy)
    }

    private var withdrawalCompleted: Bool {
        if ContributionStatusPresentation.isTerminal(record.status) { return true }
        if case .some(.withdrawn) = model.withdrawals[record.submissionID] {
            return true
        }
        return false
    }

}

private struct PublicRunEditor: View {
    let record: HistoryRecord
    let detail: SessionDetail
    let copy: PublicRunCopy

    @EnvironmentObject private var model: AppModel
    @State private var title = ""
    @State private var outcomeSummary = ""
    @State private var workflow = ""
    @State private var source = ""
    @State private var includeCorrection = false
    @State private var reusePermission: PublicRunReusePermission?
    @State private var selectedEvidence: Set<String> = []
    @State private var reviewDraft: PublicRunDraftInput?
    @State private var editingPublished = false
    @State private var configured = false

    private var working: Bool {
        model.publicRunWorking.contains(record.submissionID)
            || model.loadingSessionDetails.contains(record.submissionID)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            TCSectionHeader(title: copy.publicWorkflow)
            if let publication = detail.publication, !editingPublished {
                published(publication)
            } else if let reviewDraft {
                review(reviewDraft)
            } else {
                editor
            }
            if let message = model.publicRunErrors[record.submissionID] {
                Text(message)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
            }
        }
        .padding(TC.Space.l)
        .frame(maxWidth: .infinity, alignment: .leading)
        .tcCard(emphasised: reviewDraft != nil)
        .onAppear { configureIfNeeded() }
        .onChange(of: detail.publication?.version) { _, newVersion in
            guard newVersion != nil else { return }
            reviewDraft = nil
            editingPublished = false
        }
    }

    private func published(_ publication: PublicRunPage) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.s) {
            TCTag(text: copy.published, tone: .clear, symbol: "globe")
            Text(publication.title).font(TC.Font_.cardTitle)
            Text(publication.outcomeSummary)
                .font(TC.Font_.body)
                .foregroundStyle(TC.inkSecondary)
            HStack(spacing: TC.Space.s) {
                if let url = publication.publicURL {
                    Link(copy.openPage, destination: url)
                        .tcPrimaryAction()
                        .frame(minHeight: 44)
                }
                Button(copy.editPage) {
                    populate(from: publication)
                    editingPublished = true
                }
                .buttonStyle(.bordered)
                .frame(minHeight: 44)
                Button(working ? copy.unpublishing : copy.unpublish) {
                    model.unpublishPublicRun(record)
                }
                .buttonStyle(.bordered)
                .frame(minHeight: 44)
                .disabled(working)
            }
        }
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.publicationDisclosure)
                .font(TC.Font_.footnote)
                .foregroundStyle(TC.inkSecondary)

            fieldLabel(copy.pageTitle, count: title.count, maximum: 100)
            TextField("", text: $title)
                .textFieldStyle(.plain)
                .font(TC.Font_.body)
                .padding(.horizontal, TC.Space.m)
                .frame(minHeight: 44)
                .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.control))
                .overlay { RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line) }

            fieldLabel(copy.publicOutcome, count: outcomeSummary.count, maximum: 600)
            TextEditor(text: $outcomeSummary)
                .font(TC.Font_.body)
                .frame(minHeight: 88)
                .overlay { RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line) }

            fieldLabel(copy.reusableInstructions, count: workflow.count, maximum: 4_000)
            TextEditor(text: $workflow)
                .font(TC.Font_.body)
                .frame(minHeight: 128)
                .overlay { RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line) }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.supportingEvidence)
                Text(copy.selectEvidence)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
                ForEach(detail.evidence) { evidence in
                    Toggle(isOn: evidenceBinding(evidence.eventID)) {
                        VStack(alignment: .leading, spacing: TC.Space.xxs) {
                            Text(copy.evidenceKindLabel(for: evidence.kind))
                                .font(TC.Font_.meta)
                            Text(evidence.excerpt)
                                .font(TC.Font_.footnote)
                                .lineLimit(4)
                        }
                    }
                    .toggleStyle(.checkbox)
                    .disabled(
                        selectedEvidence.count >= 4
                        && !selectedEvidence.contains(evidence.eventID)
                    )
                    .frame(minHeight: 44, alignment: .leading)
                }
            }

            if let correction = detail.humanCorrection {
                Toggle(copy.publishCorrection, isOn: $includeCorrection)
                    .toggleStyle(.checkbox)
                    .frame(minHeight: 44)
                if includeCorrection {
                    Text(correction)
                        .font(TC.Font_.footnote)
                        .padding(TC.Space.s)
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.inset))
                }
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.reusePermission)
                if reusePermission == nil {
                    Text(copy.choosePermission)
                        .font(TC.Font_.footnote)
                        .foregroundStyle(TC.inkSecondary)
                }
                ForEach(copy.reusePermissions) { choice in
                    let selected = reusePermission == choice.permission
                    Button {
                        reusePermission = choice.permission
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: TC.Space.s) {
                            Image(systemName: selected ? "circle.inset.filled" : "circle")
                                .foregroundStyle(selected ? TC.greenText : TC.inkSecondary)
                                .accessibilityHidden(true)
                            Text([choice.label, choice.explanation].joined(separator: " · "))
                                .font(TC.Font_.footnote)
                                .multilineTextAlignment(.leading)
                            Spacer(minLength: 0)
                        }
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
                }
            }

            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.sourcePublicRun)
                TextField(copy.sourcePlaceholder, text: $source)
                    .textFieldStyle(.plain)
                    .font(TC.Font_.body)
                    .padding(.horizontal, TC.Space.m)
                    .frame(minHeight: 44)
                    .background(TC.surfaceInset, in: RoundedRectangle(cornerRadius: TC.Radius.control))
                    .overlay { RoundedRectangle(cornerRadius: TC.Radius.control).stroke(TC.line) }
                Text(copy.sourceHelp)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.inkSecondary)
            }

            if let problem = validationProblem {
                Text(problem)
                    .font(TC.Font_.footnote)
                    .foregroundStyle(TC.coralText)
            }

            HStack(spacing: TC.Space.s) {
                if detail.publication != nil {
                    Button(copy.cancelEdit) { editingPublished = false }
                        .buttonStyle(.bordered)
                        .frame(minHeight: 44)
                }
                Button(copy.reviewPage) {
                    reviewDraft = makeDraft()
                }
                .tcPrimaryAction()
                .frame(minHeight: 44)
                .disabled(makeDraft() == nil)
            }
        }
    }

    private func review(_ draft: PublicRunDraftInput) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.m) {
            Text(copy.exactPublicPreview)
                .font(TC.Font_.sectionTitle)
            Text(draft.title).font(TC.Font_.cardTitle)
            previewField(
                copy.creatorReport,
                copy.taskOutcomeLabel(for: detail.taskSuccess) ?? copy.outcomeUnavailable
            )
            previewField(copy.contributedVersion, detail.contributedVersion)
            previewField(copy.publicOutcome, draft.outcomeSummary)
            if let correction = draft.correctionExcerpt {
                previewField(copy.decisiveCorrection, correction)
            }
            VStack(alignment: .leading, spacing: TC.Space.xs) {
                TCFieldLabel(copy.observedEvidence)
                ForEach(Array(draft.evidence.enumerated()), id: \.offset) { _, evidence in
                    Text(evidence.excerpt).font(TC.Font_.footnote)
                }
            }
            previewField(copy.useWorkflow, draft.workflow)
            if let choice = copy.reuseChoice(for: draft.reusePermission) {
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    TCFieldLabel(copy.reusePermission)
                    Text(choice.label).font(TC.Font_.meta)
                }
            }
            if let sourceSlug = draft.sourceSlug {
                VStack(alignment: .leading, spacing: TC.Space.xxs) {
                    TCFieldLabel(copy.sourcePublicRun)
                    Text("/runs/\(sourceSlug)").font(TC.Font_.meta)
                }
            }
            HStack(spacing: TC.Space.s) {
                Button(copy.editDraft) { reviewDraft = nil }
                    .buttonStyle(.bordered)
                    .frame(minHeight: 44)
                Button(working ? copy.publishing : detail.publication == nil ? copy.publishPage : copy.updatePage) {
                    model.publishPublicRun(record, draft: draft)
                }
                .tcPrimaryAction()
                .frame(minHeight: 44)
                .disabled(working)
            }
        }
    }

    private func previewField(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: TC.Space.xs) {
            TCFieldLabel(label)
            Text(value)
                .font(TC.Font_.body)
                .textSelection(.enabled)
        }
    }

    private func fieldLabel(_ label: String, count: Int, maximum: Int) -> some View {
        HStack {
            TCFieldLabel(label)
            Spacer()
            Text("\(count)/\(maximum)")
                .font(TC.Font_.ledger)
                .foregroundStyle(TC.inkTertiary)
        }
    }

    private func evidenceBinding(_ id: String) -> Binding<Bool> {
        Binding(
            get: { selectedEvidence.contains(id) },
            set: { selected in
                if selected {
                    guard selectedEvidence.count < 4 else { return }
                    selectedEvidence.insert(id)
                } else {
                    selectedEvidence.remove(id)
                }
            }
        )
    }

    private var validationProblem: String? {
        guard let editorValidation else { return copy.publicationUnavailable }
        return editorValidation.error
    }

    private var editorValidation: PublicRunEditorValidation? {
        let evidence = detail.evidence
            .filter { selectedEvidence.contains($0.eventID) }
            .map { PublicRunEvidenceDraft(eventID: $0.eventID, excerpt: $0.excerpt) }
        let input = PublicRunEditorInput(
            title: title,
            outcomeSummary: outcomeSummary,
            correctionExcerpt: includeCorrection ? detail.humanCorrection : nil,
            workflow: workflow,
            reusePermission: reusePermission,
            evidence: evidence,
            source: source
        )
        guard let data = try? JSONEncoder().encode(input),
              let json = String(data: data, encoding: .utf8),
              let resultJSON = TCPublicRun.validateEditorJSON(json),
              let resultData = resultJSON.data(using: .utf8)
        else { return nil }
        return try? JSONDecoder().decode(PublicRunEditorValidation.self, from: resultData)
    }

    private func makeDraft() -> PublicRunDraftInput? {
        editorValidation?.draft
    }

    private func configureIfNeeded() {
        guard !configured else { return }
        configured = true
        if let publication = detail.publication {
            populate(from: publication)
        } else {
            source = detail.retainedSourceSlug ?? ""
        }
    }

    private func populate(from publication: PublicRunPage) {
        title = publication.title
        outcomeSummary = publication.outcomeSummary
        workflow = publication.workflow
        includeCorrection = publication.correctionExcerpt != nil
        reusePermission = publication.reusePermission
        source = publication.source?.slug ?? detail.retainedSourceSlug ?? ""
        var remaining = detail.evidence
        var restored = Set<String>()
        for published in publication.evidence {
            if let index = remaining.firstIndex(where: { $0.excerpt == published.excerpt }) {
                restored.insert(remaining.remove(at: index).eventID)
            }
        }
        selectedEvidence = restored
        reviewDraft = nil
    }

}
