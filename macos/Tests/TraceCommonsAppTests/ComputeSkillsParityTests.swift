import XCTest
@testable import TraceCommonsApp

final class ComputeSkillsParityTests: XCTestCase {
    static func text(_ rel: String) throws -> String {
        try String(contentsOf: GlassSurfaceRulesTests.root.appendingPathComponent(rel), encoding: .utf8)
    }

    /// Zero and non-numbers are refused; the daemon refuses an allowance of
    /// nothing and a typed word is not an allowance.
    func test_theAllowanceParsesOnlyAPositiveInteger() {
        XCTAssertNil(ComputeAllowance.parse(""))
        XCTAssertNil(ComputeAllowance.parse("0"))
        XCTAssertNil(ComputeAllowance.parse("8 GiB"))
        XCTAssertEqual(ComputeAllowance.parse("8"), 8)
    }

    func test_computeKeepsEveryControlAndCopySource() throws {
        let source = try Self.text("Views/ComputeView.swift")
        for needle in ["ComputeContent(model:", "snapshot.copy.introduction", "snapshot.copy.allowanceLabel",
                       "snapshot.copy.allowanceDetail", "snapshot.copy.resume", "snapshot.copy.pause",
                       "snapshot.copy.disable", "snapshot.copy.enable", "snapshot.canEnable", "snapshot.canResume",
                       "snapshot.canPause", "snapshot.available", "snapshot.consentGranted", "model.controlsBusy",
                       "model.quitWasRefused", "copy.quitRefused", "copy?.unavailable", "copy?.retry", "model.failureLabel",
                       ".enable(ramAllowanceGiB:", ".perform(.resume)", ".perform(.pause)", ".perform(.disable)",
                       "model.retryOpen()", "ComputeAllowance.parse(", "GlassTextField("] {
            XCTAssertTrue(source.contains(needle), "ComputeView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
    }

    /// Each control is disabled exactly as legacy disabled it, guard for
    /// guard, each needle running from the button to the line's end.
    func test_computeDisablesEachControlExactlyAsLegacy() throws {
        let source = try Self.text("Views/ComputeView.swift")
        for needle in [
            "Button(snapshot.copy.resume, action: resume)\n"
                + "                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canResume)\n",
            "Button(snapshot.copy.pause, action: pause)\n"
                + "                            .disabled(model.controlsBusy || !snapshot.canPause)\n",
            "Button(snapshot.copy.disable, action: disable)\n"
                + "                            .disabled(model.controlsBusy)\n",
            "Button(snapshot.copy.enable, action: enable)\n"
                + "                            .disabled(model.controlsBusy || !snapshot.available || !snapshot.canEnable\n"
                + "                                || ComputeAllowance.parse(allowance) == nil)\n",
        ] {
            XCTAssertTrue(source.contains(needle), "ComputeView.swift lacks \(needle)")
        }
    }

    /// A failure is never a spinner: the core's sentence, else its unknown
    /// word, with Retry wherever the core gives the word. The spinner is only
    /// for before an answer.
    func test_computeFailureIsNeverASpinner() throws {
        let source = try Self.text("Views/ComputeView.swift")
        let start = try XCTUnwrap(source.range(of: "} else if model.failureLabel != nil {\n"))
        let end = try XCTUnwrap(source.range(of: "            } else {\n", range: start.upperBound..<source.endIndex))
        let branch = String(source[start.upperBound..<end.lowerBound])
        XCTAssertFalse(branch.contains("SettingsAwaiting"))
        XCTAssertTrue(branch.contains("if let line = copy?.unavailable ?? Self.unknown {\n"))
        XCTAssertTrue(branch.contains("if let retry = copy?.retry {\n"))
        XCTAssertTrue(source.contains(
            "static let unknown: String? = MonitorScreensCopy.decode(fromJSON: TCCoreCopy.monitorScreensCopyJSON())?.unknown"))
        // The allowance card always says what it is; a value it lacks is
        // absent (the core's unknown word), never zero.
        XCTAssertTrue(source.contains(
            "} else {\n                            Text(snapshot.copy.allowanceLabel)\n"))
        XCTAssertTrue(source.contains("} else if let unknown = Self.unknown {\n"))
    }

    /// Lifecycle modifiers sit on always-present containers, adjacent.
    func test_lifecycleModifiersSitOnAlwaysPresentContainers() throws {
        let compute = try Self.text("Views/ComputeView.swift")
        XCTAssertTrue(compute.contains(
            "ComputeContent(model: model, allowance: $allowance)\n"
                + "            .onChange(of: model.snapshot?.ramAllowanceGib, initial: true)"))
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        // The always-present stack, scrolling with History's page.
        XCTAssertTrue(inspector.contains(
            "var body: some View {\n        VStack(alignment: .leading, spacing: GlassTokens.Space.cardGap) {\n"))
        XCTAssertTrue(inspector.contains(
            "        .frame(maxWidth: .infinity, alignment: .leading)\n"
                + "        .task(id: [row.submissionId, record?.status ?? \"\"]) { load() }\n    }\n"))
        // The reload on activation is the session detail's own, once.
        XCTAssertFalse(inspector.contains("didBecomeActiveNotification"))
        let skills = try Self.text("Views/SkillLearningView.swift")
        XCTAssertTrue(skills.contains("        GlassEyebrowCard(copy.heading) {\n"))
        XCTAssertTrue(skills.contains(
            "        .frame(maxWidth: .infinity, alignment: .leading)\n        .onAppear {\n"))
        XCTAssertTrue(skills.contains(
            "        .onChange(of: state.phase.candidate?.draft) { _, _ in configureDraft() }\n"
                + "        .onChange(of: state.failure) { _, _ in dismissedFailure = nil }\n"
                + "        .onChange(of: state.isWorking) { _, working in if working { dismissedFailure = nil } }\n"))
    }

    /// The inspector says it is reading the detail, and when the read fails
    /// it says why and offers to read again: the session detail it draws
    /// carries both, in its own words (B13 composes it rather than drawing a
    /// second copy).
    func test_theInspectorShowsReadingAndAReadFailure() throws {
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SessionDetailView(record: record, offersWithdrawal: false)\n"))
        let detail = try Self.text("Views/SessionDetailView.swift")
        for needle in [
            // Beside the detail it already holds, never instead of it: a
            // reload keeps the public-run editor and its draft mounted.
            "        if model.loadingSessionDetails.contains(record.submissionID) {\n            Text(copy.readingRecord)\n",
            "} else if let message = model.sessionDetailErrors[record.submissionID] {\n"
                + "            GlassNotice(tone: .outside, title: message) {\n"
                + "                Button(copy.retryRead) { model.loadSessionDetail(record) }\n",
            "        }\n        if let detail = model.sessionDetails[record.submissionID] {\n"
                + "            detailContent(detail, copy: copy)\n        }\n",
        ] {
            XCTAssertTrue(detail.contains(needle), "SessionDetailView.swift lacks \(needle)")
        }
    }

    /// The inspector finds the legacy record by the row's submission id, and
    /// finds nothing (so draws no Skills panel) for a row it does not hold.
    func test_theInspectorResolvesTheRecordBySubmissionId() {
        let record = HistoryRecord(
            submissionID: "s-1", submittedAt: Date(timeIntervalSince1970: 1_789_000_000), projectID: "p-1",
            projectLabel: "payments-api", source: "codex", status: "accepted", consentScopes: [],
            creditPointsPending: 0, creditPointsFinal: nil, explanations: [], lastRefreshedAt: nil)
        XCTAssertEqual(HistorySelection.record(for: "s-1", in: [record])?.submissionID, "s-1")
        XCTAssertNil(HistorySelection.record(for: "s-2", in: [record]))
        XCTAssertNil(HistorySelection.record(for: "", in: [record]))
    }

    /// The inspector's tag says the status until the session detail's
    /// overview, which says it from the same table, is drawn: once the
    /// record resolves and its detail has been read. Never both, and never
    /// neither while a row is selected.
    func test_theInspectorSaysTheStatusOnce() throws {
        let record = HistoryRecord(
            submissionID: "s-1", submittedAt: Date(timeIntervalSince1970: 1_789_000_000), projectID: "p-1",
            projectLabel: "payments-api", source: "codex", status: "accepted", consentScopes: [],
            creditPointsPending: 0, creditPointsFinal: nil, explanations: [], lastRefreshedAt: nil)
        XCTAssertTrue(HistorySelection.tagsStatus(record: nil, detail: nil), "a row with no record has only its tag")
        XCTAssertTrue(HistorySelection.tagsStatus(record: record, detail: nil), "the detail is still being read")
        XCTAssertFalse(HistorySelection.tagsStatus(record: record, detail: try detail()), "the overview says it")
    }

    private func detail(
        status: String? = "accepted", taskSuccess: String? = "success", correction: String? = "Use the fixture."
    ) throws -> SessionDetail {
        var object: [String: Any] = [
            "evidence": [], "contributed_version": "1", "consent_policy_version": "1",
            "redaction_pipeline_version": "1", "publication_version": 0,
        ]
        if let status { object["contribution_status"] = status }
        if let taskSuccess { object["task_success"] = taskSuccess }
        if let correction { object["human_correction"] = correction }
        return try JSONDecoder().decode(SessionDetail.self, from: JSONSerialization.data(withJSONObject: object))
    }

    /// The legacy session detail's rule, carried into the inspector (R-29):
    /// learning only on an accepted, outcome-known, open, corrected
    /// contribution, and nothing before the detail answers.
    func test_learningIsOfferedOnlyOnAnActiveCorrectedContribution() throws {
        func offers(_ detail: SessionDetail?, record: String = "accepted",
                    withdrawal: AppModel.WithdrawalResult? = nil) -> Bool {
            let done = SkillLearningGate.withdrawalCompleted(recordStatus: record, withdrawal: withdrawal)
            return SkillLearningGate.offersLearning(detail, recordStatus: record, withdrawalCompleted: done)
        }
        XCTAssertTrue(offers(try detail()))
        XCTAssertFalse(offers(nil), "no detail yet offers nothing")
        XCTAssertFalse(offers(try detail(correction: nil)))
        XCTAssertFalse(offers(try detail(taskSuccess: nil)))
        XCTAssertFalse(offers(try detail(status: "rejected"), record: "rejected"))
        XCTAssertFalse(offers(try detail(status: nil)))
        XCTAssertFalse(offers(try detail(status: "withdrawn")))
        XCTAssertFalse(offers(try detail(), record: "withdrawn"))
        XCTAssertFalse(offers(try detail(), withdrawal: .withdrawn(nil)))
        // An unknown status reads "Status unavailable" and is terminal.
        XCTAssertFalse(offers(try detail(), record: "status-from-a-newer-daemon"))
        XCTAssertFalse(offers(try detail(status: "status-from-a-newer-daemon")))
    }

    /// An installed skill (its rollback) shows whenever the regular flow is
    /// not on screen, including before the detail answers.
    func test_theInstalledSurfaceShowsOutsideTheLearningRule() throws {
        XCTAssertTrue(SkillLearningGate.showsInstalledSurface(nil, withdrawalCompleted: false))
        XCTAssertTrue(SkillLearningGate.showsInstalledSurface(try detail(), withdrawalCompleted: true))
        XCTAssertTrue(SkillLearningGate.showsInstalledSurface(try detail(status: "revoked"), withdrawalCompleted: false))
        XCTAssertFalse(SkillLearningGate.showsInstalledSurface(try detail(), withdrawalCompleted: false))

        var failed = SkillLearningSessionState()
        failed.workflowFailure = "read failed"
        XCTAssertTrue(SkillLearningGate.offersInstallStatusRetry(failed, detail: nil, recordStatus: "withdrawn"))
        XCTAssertTrue(SkillLearningGate.offersInstallStatusRetry(failed, detail: try detail(status: "rejected"),
                                                                 recordStatus: "rejected"))
        XCTAssertFalse(SkillLearningGate.offersInstallStatusRetry(failed, detail: try detail(), recordStatus: "accepted"))
        XCTAssertFalse(SkillLearningGate.offersInstallStatusRetry(SkillLearningSessionState(), detail: nil,
                                                                  recordStatus: "withdrawn"))
    }

    /// One rule, moved: the session detail screen and the inspector both
    /// ask the gate, and the inspector reads the detail before drawing.
    func test_bothSurfacesAskTheSameGate() throws {
        let legacy = try Self.text("Views/SessionDetailView.swift")
        XCTAssertTrue(legacy.contains("SkillLearningGate.offersLearning(detail, recordStatus: record.status,"))
        XCTAssertTrue(legacy.contains("SkillLearningGate.showsInstalledSurface(detail,"))
        XCTAssertFalse(legacy.contains("detail.taskSuccess != nil"))
        // The inspector draws the session detail, so it asks the gate
        // through it and never re-derives the rule beside it.
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SessionDetailView(record: record, offersWithdrawal: false)\n"))
        XCTAssertFalse(inspector.contains("SkillLearningGate."))
        XCTAssertTrue(inspector.contains("model.loadSessionDetail(record)"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("set: { selectedHistory = $0 }"))
    }

    func test_skillsLivesInTheHistoryInspector() throws {
        let inspector = try Self.text("Views/Monitor/HistoryInspector.swift")
        XCTAssertTrue(inspector.contains("SessionDetailView(record: record"))
        let detail = try Self.text("Views/SessionDetailView.swift")
        XCTAssertTrue(detail.contains("SkillLearningView(record: record, copy: skillCopy)"))
        XCTAssertTrue(detail.contains("model.skillLearningCopy"))
        // A new row is a new panel: the draft and the install status are per
        // record, and the inspector pane outlives a change of selection.
        XCTAssertTrue(inspector.contains(".id(record.submissionID)"))
        // The opened row's details are drawn in History's left pane, below
        // the list (Task 8 of the #1146 port).
        let homeViews = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(homeViews.contains("HistoryDetailInspector(row: opened)"))
        let window = try Self.text("Views/MonitorWindowView.swift")
        XCTAssertTrue(window.contains("@SceneStorage(\"monitor.selectedHistory\") private var selectedHistory = \"\""))
        let home = try Self.text("Views/Monitor/HomeViews.swift")
        XCTAssertTrue(home.contains("HistoryPage(store: store, statusLabel: statusLabel, selection: $selection,"))
        XCTAssertTrue(home.contains("selection = row.submissionId"))
    }

    func test_skillLearningKeepsEveryCallAndAuthorsNothing() throws {
        let source = try Self.text("Views/SkillLearningView.swift")
        for needle in ["model.skillLearningState(for:", "model.learnSkill(from:", "model.reviewSkill(from:",
                       "model.testSkill(from:", "model.editSkill(from:", "model.reviewSkillInstall(from:",
                       "model.installSkill(from:", "model.rollbackSkill(from:", "model.ensureLocalInstalledSkillStatus(for:",
                       "TCSkillLearning.validateDraftJSON(", "TCSkillLearning.errorLine(label:", "copy.rollbackDisclosure",
                       "copy.evaluationDisclosure", "GlassTextField(", "GlassEyebrowCard(copy.heading)",
                       "GlassExpander(copy.sourceEvidence,", "GlassExpander(copy.inspectRuns,",
                       "GlassExpander(copy.modelOutput,", "GlassNotice(tone: .outside)", ".task(id: draft)",
                       ".milliseconds(150)", "private struct SkillReviewPreview: View",
                       "private struct SkillEvaluationResults: View", "private struct SkillTrialRow: View",
                       "private struct SkillInstallPreview: View", "private struct InstalledSkillPanel: View"] {
            XCTAssertTrue(source.contains(needle), "SkillLearningView.swift lacks \(needle)")
        }
        XCTAssertNil(source.range(of: #"\bTC\."#, options: .regularExpression))
        XCTAssertFalse(source.contains("DisclosureGroup("))
        // A failure is never undismissable; the word is the core's, with the
        // shell's own (`ActionNoticeWords`) as the fallback by reference.
        XCTAssertTrue(source.contains(
            "Button(ActionNoticeWords.coreDismissWord ?? ActionNoticeWords.dismissWord) { dismissedFailure = message }"))
        XCTAssertTrue(try Self.text("Views/SettingsView.swift").contains(
            "static let coreDismissWord = MonitorTracesCopy.decode(fromJSON: TCCoreCopy.monitorTracesCopyJSON())?.dismiss"))
    }
}
