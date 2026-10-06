// INTEGRATION: reached from HistoryDetailInspector and backed only by the account-authenticated
// history_detail / publish_public_run / unpublish_public_run daemon methods.

import AppKit
import SwiftUI
import TCBridge
import TCDesign
import TCShellCore

struct SessionDetailView: View {
    let record: HistoryRecord
    /// Back to the list. Nil where the detail is drawn in an inspector,
    /// which has no back control.
    let onBack: (() -> Void)?
    /// Whether the detail draws its own Withdraw. False in the Monitor's
    /// History pane, where Withdraw lives on the row above (Ron's #1146
    /// `HistoryRow`), so the page has one Withdraw.
    let offersWithdrawal: Bool

    @EnvironmentObject private var model: AppModel

    init(record: HistoryRecord, offersWithdrawal: Bool = true, onBack: (() -> Void)? = nil) {
        self.record = record
        self.offersWithdrawal = offersWithdrawal
        self.onBack = onBack
    }

    var body: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            if let copy = model.publicRunCopy {
                content(copy)
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
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
        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
            // The trail's last crumb is the heading; without a trail the
            // title is.
            if let onBack {
                GlassBreadcrumb([GlassCrumb(copy.allContributions, action: onBack), GlassCrumb(copy.sessionDetail)],
                                backLabel: copy.allContributions, onBack: onBack)
                    .frame(minHeight: 44, alignment: .leading)
            } else {
                Text(copy.sessionDetail)
                    .glassType(GlassTokens.TypeScale.title)
                    .foregroundStyle(GlassColor.textPrimary)
                    .accessibilityAddTraits(.isHeader)
            }
            Text("\(record.projectLabel) · \(Format.when(record.submittedAt))")
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
        }

        // Reading, or why a read failed, beside the detail already held
        // (`AppModel.loadSessionDetail` keeps it until the daemon answers),
        // never instead of it: a reload keeps the editor and its draft.
        if model.loadingSessionDetails.contains(record.submissionID) {
            Text(copy.readingRecord)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
        } else if let message = model.sessionDetailErrors[record.submissionID] {
            GlassNotice(tone: .outside, title: message) {
                Button(copy.retryRead) { model.loadSessionDetail(record) }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .frame(minHeight: 44)
            }
        }
        if let detail = model.sessionDetails[record.submissionID] {
            detailContent(detail, copy: copy)
        }
        if offersWithdrawal {
            SessionWithdrawalAction(record: record, currentStatus: Self.withdrawalStatus(record, detail: model.sessionDetails[record.submissionID]), copy: copy)
        }

        localInstalledSkillSurface(copy)
    }

    /// The status Withdraw asks about: the detail's when it has been read,
    /// else the app's own record's. Withdraw stands on the record, as the
    /// legacy row offered it, so reading the detail, or failing to, never
    /// hides it or its outcome. An unknown status still offers none.
    static func withdrawalStatus(_ record: HistoryRecord, detail: SessionDetail?) -> String {
        detail?.contributionStatus ?? record.status
    }

    @ViewBuilder
    private func localInstalledSkillSurface(_ copy: PublicRunCopy) -> some View {
        let state = model.skillLearningState(for: record.submissionID)
        let detail = model.sessionDetails[record.submissionID]
        if SkillLearningGate.showsInstalledSurface(detail, withdrawalCompleted: withdrawalCompleted) {
            if state.installedSkill != nil {
                if let skillCopy = model.skillLearningCopy {
                    SkillLearningView(record: record, copy: skillCopy)
                }
            } else if SkillLearningGate.offersInstallStatusRetry(state, detail: detail, recordStatus: record.status) {
                GlassCard {
                    VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
                        if let message = state.failure {
                            GlassStatusLabel(message, status: .outside)
                                .fixedSize(horizontal: false, vertical: true)
                        }
                        Button(copy.retryRead) {
                            model.ensureLocalInstalledSkillStatus(for: record)
                        }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .frame(minHeight: 44)
                        .disabled(state.isWorking)
                    }
                    .frame(maxWidth: .infinity, alignment: .leading)
                }
            }
        }
    }

    @ViewBuilder
    private func detailContent(_ detail: SessionDetail, copy: PublicRunCopy) -> some View {
        let contributionIsWithdrawn = SkillLearningGate.contributionIsWithdrawn(
            detail, recordStatus: record.status, withdrawalCompleted: withdrawalCompleted)
        let contributionIsActive = SkillLearningGate.contributionIsActive(
            detail, recordStatus: record.status, withdrawalCompleted: withdrawalCompleted)

        SessionContributionOverview(record: record, detail: detail, copy: copy)

        if SkillLearningGate.offersLearning(detail, recordStatus: record.status, withdrawalCompleted: withdrawalCompleted),
           let skillCopy = model.skillLearningCopy {
            SkillLearningView(record: record, copy: skillCopy)
        }

        if contributionIsActive {
            PublicRunEditor(record: record, detail: detail, copy: copy)
        } else if !contributionIsWithdrawn {
            Text(copy.publicationAfterAcceptance)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
        }
    }

    private var withdrawalCompleted: Bool {
        SkillLearningGate.withdrawalCompleted(
            recordStatus: record.status, withdrawal: model.withdrawals[record.submissionID])
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
    /// The publication error the person put away. The next publish or
    /// unpublish clears it, so its error is shown even when it reads the same.
    @State private var dismissedError: String?

    private var working: Bool {
        model.publicRunWorking.contains(record.submissionID)
            || model.loadingSessionDetails.contains(record.submissionID)
    }

    var body: some View {
        GlassEyebrowCard(copy.publicWorkflow) {
            VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
                if let publication = detail.publication, !editingPublished {
                    published(publication)
                } else if let reviewDraft {
                    review(reviewDraft)
                } else {
                    editor
                }
                if let message = model.publicRunErrors[record.submissionID], message != dismissedError {
                    GlassNotice(tone: .outside) {
                        HStack(alignment: .top, spacing: GlassTokens.Space.s3) {
                            Text(message)
                                .fixedSize(horizontal: false, vertical: true)
                                .frame(maxWidth: .infinity, alignment: .leading)
                            Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) { dismissedError = message }
                                .buttonStyle(GlassButtonStyle(.glass))
                                .frame(minHeight: 44)
                        }
                    }
                }
            }
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .onAppear { configureIfNeeded() }
        .onChange(of: detail.publication?.version) { _, newVersion in
            guard newVersion != nil else { return }
            reviewDraft = nil
            editingPublished = false
        }
        .onChange(of: model.publicRunWorking.contains(record.submissionID)) { _, working in
            if working { dismissedError = nil }
        }
    }

    private func published(_ publication: PublicRunPage) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s4) {
            GlassTag(copy.published, tone: .on)
            Text(publication.title)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            Text(publication.outcomeSummary)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack(spacing: GlassTokens.Space.s4) {
                if let url = publication.publicURL {
                    Link(copy.openPage, destination: url)
                        .buttonStyle(GlassButtonStyle(.primary))
                        .frame(minHeight: 44)
                }
                Button(copy.editPage) {
                    populate(from: publication)
                    editingPublished = true
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .frame(minHeight: 44)
                Button(working ? copy.unpublishing : copy.unpublish) {
                    model.unpublishPublicRun(record)
                }
                .buttonStyle(GlassButtonStyle(.glass))
                .frame(minHeight: 44)
                .disabled(working)
            }
        }
    }

    private var editor: some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(copy.publicationDisclosure)
                .glassType(GlassTokens.TypeScale.caption)
                .foregroundStyle(GlassColor.textSecondary)
                .fixedSize(horizontal: false, vertical: true)

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                fieldLabel(copy.pageTitle, count: title.count, maximum: 100)
                GlassTextField(copy.pageTitle, text: $title, prompt: copy.pageTitle, showsLabel: false)
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                fieldLabel(copy.publicOutcome, count: outcomeSummary.count, maximum: 600)
                GlassTextArea(copy.publicOutcome, text: $outcomeSummary, showsLabel: false)
                    .frame(minHeight: 88)
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                fieldLabel(copy.reusableInstructions, count: workflow.count, maximum: 4_000)
                GlassTextArea(copy.reusableInstructions, text: $workflow, showsLabel: false)
                    .frame(minHeight: 128)
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                eyebrow(copy.supportingEvidence)
                Text(copy.selectEvidence)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
                ForEach(detail.evidence) { evidence in
                    Toggle(isOn: evidenceBinding(evidence.eventID)) {
                        VStack(alignment: .leading, spacing: GlassTokens.Space.s1) {
                            Text(copy.evidenceKindLabel(for: evidence.kind))
                                .glassType(GlassTokens.TypeScale.label)
                            Text(evidence.excerpt)
                                .glassType(GlassTokens.TypeScale.caption)
                                .lineLimit(4)
                        }
                    }
                    .toggleStyle(GlassCheckboxStyle())
                    .disabled(
                        selectedEvidence.count >= 4
                        && !selectedEvidence.contains(evidence.eventID)
                    )
                    .frame(minHeight: 44, alignment: .leading)
                }
            }

            if let correction = detail.humanCorrection {
                Toggle(copy.publishCorrection, isOn: $includeCorrection)
                    .toggleStyle(GlassCheckboxStyle())
                    .frame(minHeight: 44, alignment: .leading)
                if includeCorrection {
                    GlassWell {
                        Text(correction)
                            .glassType(GlassTokens.TypeScale.caption)
                            .foregroundStyle(GlassColor.textPrimary)
                            .padding(GlassTokens.Space.s4)
                            .frame(maxWidth: .infinity, alignment: .leading)
                    }
                }
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                eyebrow(copy.reusePermission)
                if reusePermission == nil {
                    Text(copy.choosePermission)
                        .glassType(GlassTokens.TypeScale.caption)
                        .foregroundStyle(GlassColor.textSecondary)
                }
                ForEach(copy.reusePermissions) { choice in
                    let selected = reusePermission == choice.permission
                    Button {
                        reusePermission = choice.permission
                    } label: {
                        HStack(alignment: .firstTextBaseline, spacing: GlassTokens.Space.s4) {
                            GlassCheckMark(checked: selected)
                            Text([choice.label, choice.explanation].joined(separator: " · "))
                                .glassType(GlassTokens.TypeScale.label.weight(.regular))
                                .foregroundStyle(GlassColor.textPrimary)
                                .multilineTextAlignment(.leading)
                                .fixedSize(horizontal: false, vertical: true)
                            Spacer(minLength: 0)
                        }
                        .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
                        .contentShape(Rectangle())
                    }
                    .buttonStyle(.plain)
                    .accessibilityAddTraits(selected ? [.isSelected, .isButton] : .isButton)
                }
            }

            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                eyebrow(copy.sourcePublicRun)
                GlassTextField(copy.sourcePlaceholder, text: $source, prompt: copy.sourcePlaceholder, showsLabel: false)
                Text(copy.sourceHelp)
                    .glassType(GlassTokens.TypeScale.caption)
                    .foregroundStyle(GlassColor.textSecondary)
                    .fixedSize(horizontal: false, vertical: true)
            }

            if let problem = validationProblem {
                GlassStatusLabel(problem, status: .outside)
                    .fixedSize(horizontal: false, vertical: true)
            }

            HStack(spacing: GlassTokens.Space.s4) {
                if detail.publication != nil {
                    Button(copy.cancelEdit) { editingPublished = false }
                        .buttonStyle(GlassButtonStyle(.glass))
                        .frame(minHeight: 44)
                }
                Button(copy.reviewPage) {
                    reviewDraft = makeDraft()
                }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(minHeight: 44)
                .disabled(makeDraft() == nil)
            }
        }
    }

    private func review(_ draft: PublicRunDraftInput) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s6) {
            Text(copy.exactPublicPreview)
                .glassType(GlassTokens.TypeScale.title)
                .foregroundStyle(GlassColor.textPrimary)
            Text(draft.title)
                .glassType(GlassTokens.TypeScale.bodyStrong)
                .foregroundStyle(GlassColor.textPrimary)
            previewField(
                copy.creatorReport,
                copy.taskOutcomeLabel(for: detail.taskSuccess) ?? copy.outcomeUnavailable
            )
            previewField(copy.contributedVersion, detail.contributedVersion)
            previewField(copy.publicOutcome, draft.outcomeSummary)
            if let correction = draft.correctionExcerpt {
                previewField(copy.decisiveCorrection, correction)
            }
            VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
                eyebrow(copy.observedEvidence)
                ForEach(Array(draft.evidence.enumerated()), id: \.offset) { _, evidence in
                    Text(evidence.excerpt)
                        .glassType(GlassTokens.TypeScale.body)
                        .foregroundStyle(GlassColor.textPrimary)
                        .textSelection(.enabled)
                }
            }
            previewField(copy.useWorkflow, draft.workflow)
            if let choice = copy.reuseChoice(for: draft.reusePermission) {
                previewField(copy.reusePermission, choice.label)
            }
            if let sourceSlug = draft.sourceSlug {
                previewField(copy.sourcePublicRun, "/runs/\(sourceSlug)")
            }
            HStack(spacing: GlassTokens.Space.s4) {
                Button(copy.editDraft) { reviewDraft = nil }
                    .buttonStyle(GlassButtonStyle(.glass))
                    .frame(minHeight: 44)
                Button(working ? copy.publishing : detail.publication == nil ? copy.publishPage : copy.updatePage) {
                    model.publishPublicRun(record, draft: draft)
                }
                .buttonStyle(GlassButtonStyle(.primary))
                .frame(minHeight: 44)
                .disabled(working)
            }
        }
    }

    private func previewField(_ label: String, _ value: String) -> some View {
        VStack(alignment: .leading, spacing: GlassTokens.Space.s2) {
            eyebrow(label)
            Text(value)
                .glassType(GlassTokens.TypeScale.body)
                .foregroundStyle(GlassColor.textPrimary)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
    }

    private func eyebrow(_ text: String) -> some View {
        Text(text)
            .glassType(GlassTokens.TypeScale.eyebrow)
            .foregroundStyle(GlassColor.textTertiary)
    }

    private func fieldLabel(_ label: String, count: Int, maximum: Int) -> some View {
        HStack {
            eyebrow(label)
            Spacer()
            Text("\(count)/\(maximum)")
                .glassType(GlassTokens.TypeScale.mono)
                .foregroundStyle(GlassColor.textTertiary)
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

