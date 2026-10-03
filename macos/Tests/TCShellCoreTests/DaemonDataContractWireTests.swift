import XCTest
@testable import TCShellCore

/// C1 of #1173: the contract's error paths, events and the fields a screen
/// must not lose, against frames in the daemon's own shapes.
final class DaemonDataContractWireTests: XCTestCase {
    private func frame(_ result: String) -> String { #"{"id":0,"result":\#(result)}"# }

    // MARK: - Unreachable

    /// An attached daemon that stops listening, disconnects or times out
    /// answers `daemon-disconnected` (contributor-ffi `tc_call`), and
    /// `TCDaemon` answers `handle-freed` once teardown starts. Both are the
    /// core-down state, never a generic daemon error.
    func testEveryUnreachableLabelIsUnreachable() async {
        for label in [
            "daemon-stopped", "daemon-disconnected", "attached-transport-failed", "handle-freed",
            "null-handle", "invalid-handle-pointer",
        ] {
            let transport = FakeTransport(response: #"{"error":{"code":"unavailable","message":"\#(label)"}}"#)
            do {
                _ = try await LiveDaemonClient(transport: transport).listProjects()
                XCTFail("\(label) answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .unreachable, label)
            }
        }
    }

    // MARK: - The work queue

    func testCallsRunOnTheClientsQueueNotTheCooperativePool() async throws {
        let transport = FakeTransport { _, _ in #"{"id":0,"result":{"kept":true}}"# }
        _ = try await LiveDaemonClient(transport: transport).keep(entryId: "e1")
        XCTAssertEqual(transport.calls.map(\.onQueue), [true])
    }

    // MARK: - Undecodable keeps its coding path

    func testUndecodableKeepsTheCodingPathAndNoValue() async {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"pending":[{"entry_id":"e1","source":"codex","project_id":"p","project_label":"api","state":"pending","started_at":"secret-looking-value"}]}}"#
        }
        do {
            _ = try await LiveDaemonClient(transport: transport).listPending(projectId: nil)
            XCTFail("an unreadable reply answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .undecodable(method: "list_pending", codingPath: "pending[0].started_at"))
            XCTAssertFalse("\(error)".contains("secret-looking-value"), "the path holds keys, never values")
        }
    }

    func testAMissingRequiredKeyNamesTheKey() async {
        let transport = FakeTransport { _, _ in #"{"id":0,"result":{"entry_id":"e1"}}"# }
        do {
            _ = try await LiveDaemonClient(transport: transport).cancelPreview(entryId: "e1")
            XCTFail("an unreadable reply answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .undecodable(method: "preview_cancel", codingPath: "dropped"))
        }
    }

    // MARK: - Scheduled previews

    func testThePreviewSchedulerMethodsSendTheirNamesAndParams() async throws {
        let transport = FakeTransport { method, _ in
            switch method {
            case "preview_request": return #"{"id":0,"result":{"entry_id":"e1","state":"queued"}}"#
            case "preview_visible": return #"{"id":0,"result":{"visible":2}}"#
            case "preview_cancel": return #"{"id":0,"result":{"entry_id":"e1","dropped":true}}"#
            default: return #"{"id":0,"error":{"code":"bad_params","message":"unknown-method"}}"#
            }
        }
        let client = LiveDaemonClient(transport: transport)
        let requested = try await client.requestPreview(entryId: "e1")
        XCTAssertEqual(requested.state, .queued)
        XCTAssertNil(requested.summary)
        let visible = try await client.setVisiblePreviews(entryIds: ["e1", "e2"])
        XCTAssertEqual(visible, 2)
        let cancelled = try await client.cancelPreview(entryId: "e1")
        XCTAssertEqual(cancelled, DaemonData.PreviewCancelResult(entryId: "e1", dropped: true))
        XCTAssertEqual(transport.calls.map(\.method), ["preview_request", "preview_visible", "preview_cancel"])
        XCTAssertEqual(
            transport.calls.map(\.params),
            [#"{"entry_id":"e1"}"#, #"{"entry_ids":["e1","e2"]}"#, #"{"entry_id":"e1"}"#])
    }

    func testACachedTooLargePreviewIsNotReady() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"entry_id":"e1","state":"too_large","raw_session_bytes":90000000,"limit_bytes":67108864}}"#
        }
        let outcome = try await LiveDaemonClient(transport: transport).requestPreview(entryId: "e1")
        XCTAssertEqual(outcome.state, .tooLarge)
        XCTAssertEqual(outcome.limitBytes, 67_108_864)
        XCTAssertNil(outcome.summary)
    }

    func testPreviewReadyCarriesItsState() {
        let tooLarge = DaemonDataEventParser.parse(
            #"{"event":"preview_ready","data":{"entry_id":"e1","state":"too_large","raw_session_bytes":90000000,"limit_bytes":67108864}}"#)
        XCTAssertEqual(
            tooLarge,
            .previewReady(PreviewRequestResult(entryID: "e1", state: .tooLarge, rawSessionBytes: 90_000_000, limitBytes: 67_108_864)))

        let failed = DaemonDataEventParser.parse(
            #"{"event":"preview_ready","data":{"entry_id":"e2","state":"failed","code":"unavailable","label":"session-unreadable"}}"#)
        XCTAssertEqual(
            failed,
            .previewReady(PreviewRequestResult(entryID: "e2", state: .failed, code: "unavailable", label: "session-unreadable")))

        let ready = DaemonDataEventParser.parse(
            #"{"event":"preview_ready","data":{"entry_id":"e3","state":"ready","summary":{"title":"Fix the flaky test","redactions":{"email":2},"redactions_distinct":{"email":1}}}}"#)
        guard case .previewReady(let outcome) = ready else { return XCTFail("\(ready)") }
        XCTAssertEqual(outcome.state, .ready)
        XCTAssertEqual(outcome.summary?.title, "Fix the flaky test")
        XCTAssertEqual(outcome.summary?.redactionsDistinct, ["email": 1])
    }

    func testSampleCardsAreReadyFromTheScheduler() async throws {
        let client = SampleDaemonClient(.normalDay)
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        let outcome = try await client.requestPreview(entryId: entry.entryId)
        XCTAssertEqual(outcome.state, .ready)
        let card = try XCTUnwrap(outcome.summary)
        // The daemon's card (`preview_card_value`) is a scrub, never a
        // build: no envelope, so no `envelope_digest`, and no distinct
        // counts. A scheduled card carries no `entry` either; `preview`
        // adds a freshly read one.
        XCTAssertNil(card.envelopeDigest)
        XCTAssertNil(card.redactionsDistinct)
        XCTAssertNil(card.subagentCount)
        XCTAssertNil(card.entry)
        let blocking = try await client.preview(entryId: entry.entryId)
        XCTAssertNil(blocking.envelopeDigest)
        XCTAssertNil(blocking.redactionsDistinct)
        XCTAssertEqual(blocking.entry?.entryId, entry.entryId)
        XCTAssertEqual(blocking.title, card.title)
        let cancelled = try await client.cancelPreview(entryId: entry.entryId)
        XCTAssertFalse(cancelled.dropped)
    }

    // MARK: - Unsure spans go through tc_call

    func testUnsureSpansAreRoutedThroughTcCall() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"entry_id":"e1","body_digest":"sha256:b","envelope_digest":"sha256:e","span_count":1,"spans":[{"label":"looks-like-email","byte_offset":4,"byte_len":9}],"spans_truncated":false}}"#
        }
        let spans = try await LiveDaemonClient(transport: transport).previewUnsureSpans(entryId: "e1", bodyDigest: "sha256:b")
        XCTAssertEqual(spans.spanCount, 1)
        XCTAssertEqual(transport.calls.map(\.method), ["preview_unsure_spans"])
        XCTAssertEqual(transport.calls.first?.params, #"{"body_digest":"sha256:b","entry_id":"e1"}"#)
    }

    // MARK: - Snapshots never loop

    func testAnUnreadableQueueInASnapshotIsQueueChangedNotResync() {
        let event = DaemonDataEventParser.parse(
            #"{"event":"snapshot","data":{"pending":[{"entry_id":"e1"}],"status":{"queue_depth":1}}}"#)
        XCTAssertEqual(event, .queueChanged)
    }

    func testAnUnreadableStatusInASnapshotKeepsTheQueue() throws {
        let entry = #"{"entry_id":"e1","source":"codex","project_id":"p","project_label":"api","state":"pending"}"#
        let event = DaemonDataEventParser.parse(
            #"{"event":"snapshot","data":{"pending":[\#(entry)],"status":{"queue_depth":"many"}}}"#)
        guard case .snapshot(let pending, let status) = event else { return XCTFail("\(event)") }
        XCTAssertEqual(pending.map(\.entryId), ["e1"])
        XCTAssertNil(status)
    }

    // MARK: - The sample refuses an unknown project like the daemon

    func testSampleRefusesAnUnknownProjectId() async throws {
        let client = SampleDaemonClient(.normalDay)
        do {
            _ = try await client.listPending(projectId: "proj_does_not_exist")
            XCTFail("an unknown project answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "project-id-unrecognized"))
        }
        let all = try await client.listPending(projectId: nil)
        let known = try XCTUnwrap(all.first?.projectId)
        let narrowed = try await client.listPending(projectId: known)
        XCTAssertEqual(narrowed, all.filter { $0.projectId == known })
    }

    /// Both clients answer a stale id the same way, so a screen built on
    /// samples exercises the refusal it will meet live.
    func testSampleAndLiveAgreeOnAnUnknownProjectId() async {
        let live = LiveDaemonClient(transport: FakeTransport(response:
            #"{"id":0,"error":{"code":"bad_params","message":"project-id-unrecognized"}}"#))
        var answers: [DaemonDataError?] = []
        for client in [live, SampleDaemonClient(.normalDay)] as [any DaemonDataClient] {
            do {
                _ = try await client.listPending(projectId: "proj_does_not_exist")
                answers.append(nil)
            } catch {
                answers.append(error as? DaemonDataError)
            }
        }
        XCTAssertEqual(answers, Array(repeating: .daemon(code: "bad_params", message: "project-id-unrecognized"), count: 2))
    }

    // MARK: - Approve never draws a skip as success

    func testASkippedSingleEntryApproveThrowsWithItsReason() async {
        for reason in ["not-pending", "not-enrolled", "witness-review-stale"] {
            let transport = FakeTransport { _, _ in
                #"{"id":0,"result":{"approved":0,"hold_secs":30,"hold_until":null,"flagged":0,"redactions":{},"skipped":[{"entry_id":"e1","reason_label":"\#(reason)"}]}}"#
            }
            do {
                _ = try await LiveDaemonClient(transport: transport).approve(entryId: "e1")
                XCTFail("\(reason) was drawn as success")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .notApproved(reasonLabel: reason))
            }
        }
    }

    func testAnApprovedEntryCarriesFlaggedAndRedactions() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"approved":1,"hold_secs":30,"hold_until":"2026-10-02T09:00:30Z","flagged":2,"redactions":{"email":3,"local_path":1},"skipped":[]}}"#
        }
        let response = try await LiveDaemonClient(transport: transport).approve(entryId: "e1")
        XCTAssertEqual(response.approved, 1)
        XCTAssertEqual(response.flagged, 2)
        XCTAssertEqual(response.totalRedactions, 4)
        XCTAssertEqual(response.skipped, [])
        XCTAssertEqual(response.holdUntil, "2026-10-02T09:00:30Z")
    }

    func testSampleApproveRefusesAnIdItNeverHeldAndSkipsAKeptOne() async throws {
        let client = SampleDaemonClient(.normalDay)
        // The daemon refuses an id it never held up front (`ipc.rs`,
        // `ERR_UNKNOWN_ENTRY_ID`), rather than reporting a skip.
        do {
            _ = try await client.approve(entryId: "not-in-this-set")
            XCTFail("an unknown id was drawn as success")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "unknown-entry-id"))
        }
        // An entry it holds but cannot approve is the labelled skip.
        let keptList = try await client.listKept()
        let kept = try XCTUnwrap(keptList.first, "normalDay has a kept entry")
        do {
            _ = try await client.approve(entryId: kept.entryId)
            XCTFail("a skip was drawn as success")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notApproved(reasonLabel: "not-pending"))
        }
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        let approved = try await client.approve(entryId: entry.entryId)
        XCTAssertEqual(approved.approved, 1)
    }

    // MARK: - Status keeps the void notice and the migration notice

    func testGrantVoidsAbsentIsNotEmpty() throws {
        let decoder = DaemonDataDecoding.decoder()
        let absent = try decoder.decode(DaemonData.Status.self, from: Data(#"{"queue_depth":0}"#.utf8))
        XCTAssertNil(absent.grantVoids, "a daemon too old to say is not 'nothing to show'")
        XCTAssertNil(absent.legacyInviteMigration)
        let empty = try decoder.decode(DaemonData.Status.self, from: Data(#"{"grant_voids":[]}"#.utf8))
        XCTAssertEqual(empty.grantVoids, [])
    }

    func testGrantVoidsAndTheMigrationNoticeDecodeAndKeepTheirWire() throws {
        let json = #"{"grant_voids":[{"id":4,"kind":"project","voided_at":"2026-10-01T09:00:00Z","project_id":"proj_1","project_label":"api","reasons":["scopes-widened"]}],"legacy_invite_migration":{"offered":false,"notice":{"folders_kept":true,"automatic_grant_kept":false}}}"#
        let status = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: Data(json.utf8))
        let void = try XCTUnwrap(status.grantVoids?.first)
        XCTAssertEqual(void.id, 4)
        XCTAssertEqual(void.projectId, "proj_1")
        XCTAssertTrue(void.json.contains(#""reasons":["scopes-widened"]"#))
        XCTAssertEqual(status.legacyInviteMigration?.offered, false)
        XCTAssertEqual(status.legacyInviteMigration?.notice?.foldersKept, true)
        XCTAssertEqual(status.legacyInviteMigration?.noticeJSON, #"{"automatic_grant_kept":false,"folders_kept":true}"#)

        // `Status` stays Codable: a void re-encodes as it came.
        let reencoded = try JSONEncoder().encode(status)
        let again = try DaemonDataDecoding.decoder().decode(DaemonData.Status.self, from: reencoded)
        let element = { (wire: GrantVoidWire?) in
            try JSONSerialization.jsonObject(with: Data((wire?.json ?? "{}").utf8)) as? NSDictionary
        }
        XCTAssertEqual(try element(again.grantVoids?.first), try element(status.grantVoids?.first))
    }

    func testNoMigrationNoticeIsNil() throws {
        let status = try DaemonDataDecoding.decoder().decode(
            DaemonData.Status.self, from: Data(#"{"legacy_invite_migration":{"offered":true,"notice":null}}"#.utf8))
        XCTAssertEqual(status.legacyInviteMigration?.offered, true)
        XCTAssertNil(status.legacyInviteMigration?.noticeJSON)
    }

    // MARK: - Queue entries keep attested_inference

    func testAttestedInferenceDecodesAndAbsentStaysUnknown() throws {
        let base = #""entry_id":"e1","source":"codex","project_id":"p","project_label":"api","state":"pending","holds_certificate":true"#
        let decoder = DaemonDataDecoding.decoder()
        let certified = try decoder.decode(
            DaemonData.QueueEntry.self, from: Data(#"{\#(base),"attested_inference":{"state":"certified"}}"#.utf8))
        XCTAssertEqual(certified.attestedInference, DaemonData.AttestedInference(state: "certified", reason: nil))
        let uncertified = try decoder.decode(
            DaemonData.QueueEntry.self,
            from: Data(#"{\#(base),"attested_inference":{"state":"uncertified","reason":"no-receipt"}}"#.utf8))
        XCTAssertEqual(uncertified.attestedInference?.reason, "no-receipt")
        let absent = try decoder.decode(DaemonData.QueueEntry.self, from: Data(#"{\#(base)}"#.utf8))
        XCTAssertNil(absent.attestedInference)
    }

    // MARK: - digest_due carries the contribution half

    func testDigestDueCarriesWhatWentWithoutYou() {
        let event = DaemonDataEventParser.parse(
            #"{"event":"digest_due","data":{"pending":2,"contributed":3,"contributed_projects":["api","web"],"credit_pending":1.5,"text":"2 waiting"}}"#)
        XCTAssertEqual(
            event,
            .digestDue(DaemonData.DigestDue(
                pending: 2, contributed: 3, contributedProjects: ["api", "web"], creditPending: 1.5, text: "2 waiting")))
    }

    func testAnOlderDigestLeavesTheContributionHalfUnknown() {
        let event = DaemonDataEventParser.parse(#"{"event":"digest_due","data":{"pending":2,"text":"2 waiting"}}"#)
        XCTAssertEqual(event, .digestDue(DaemonData.DigestDue(pending: 2, text: "2 waiting")))
    }

    // MARK: - R6: the tool switch and folder approve

    func testSetSourceSendsTheDeclarationAndSendsNothingForANonAnswer() async throws {
        let transport = FakeTransport { _, _ in #"{"id":0,"result":{"codex_source_mode":"off"}}"# }
        let client = LiveDaemonClient(transport: transport)
        let settings = try await client.setSource(.codex, .off)
        XCTAssertEqual(settings.codexSourceMode, "off")
        _ = try await client.setSource(.claudeCode, .watch(path: "/Users/me/.claude/projects"))
        XCTAssertEqual(
            transport.calls.map(\.params),
            [#"{"codex_source":{"mode":"off"}}"#, #"{"claude_source":{"mode":"watch","path":"\/Users\/me\/.claude\/projects"}}"#])
        for choice in [SourceChoice.undecided, .watch(path: "  ")] {
            do {
                _ = try await client.setSource(.geminiCli, choice)
                XCTFail("\(choice) was sent")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "settings-invalid-value"))
            }
        }
        XCTAssertEqual(transport.calls.count, 2, "a non-answer sends nothing")
    }

    func testAFolderApproveSendsTheProjectAndNeverThrowsForWhatItLeftOut() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"approved":0,"hold_secs":30,"hold_until":null,"flagged":0,"redactions":{},"skipped":[],"excluded_held":2,"excluded_ineligible":1}}"#
        }
        let response = try await LiveDaemonClient(transport: transport).approveFolder(projectId: "proj_1")
        XCTAssertEqual(transport.calls.map(\.method), ["approve"])
        XCTAssertEqual(transport.calls.first?.params, #"{"project_id":"proj_1"}"#)
        XCTAssertEqual(response.approved, 0)
        XCTAssertEqual(response.excludedHeld, 2)
        XCTAssertEqual(response.excludedIneligible, 1)
    }

    /// A folder approve carries the verdict only when one was chosen.
    func testAFolderApproveSendsTheVerdictOnlyWhenChosen() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"approved":2,"hold_secs":30,"hold_until":null,"flagged":0,"redactions":{},"skipped":[],"excluded_held":0,"excluded_ineligible":1}}"#
        }
        let client = LiveDaemonClient(transport: transport)
        _ = try await client.approveFolder(projectId: "proj_1", verdict: nil)
        _ = try await client.approveFolder(projectId: "proj_1", verdict: .partly)
        XCTAssertEqual(transport.calls.map(\.params), [
            #"{"project_id":"proj_1"}"#,
            #"{"outcome":"partly","project_id":"proj_1"}"#,
        ])
    }

    func testSampleFolderApproveLeavesHeldSessionsOut() async throws {
        let client = SampleDaemonClient(.heldSessions)
        let pending = try await client.listPending(projectId: nil)
        let project = try XCTUnwrap(pending.first { $0.heldForSecondLook }?.projectId)
        let response = try await client.approveFolder(projectId: project)
        XCTAssertGreaterThan(try XCTUnwrap(response.excludedHeld), 0)
        // Every folder, held exactly as the daemon's `held_for_review` holds.
        for folder in Set(pending.map(\.projectId)) {
            let inFolder = pending.filter { $0.projectId == folder }
            let held = inFolder.filter(\.heldForReview).count
            let answer = try await client.approveFolder(projectId: folder)
            XCTAssertEqual(answer.excludedHeld, UInt64(held), folder)
            XCTAssertEqual(answer.approved, UInt64(inFolder.count - held), folder)
        }
        do {
            _ = try await client.approveFolder(projectId: "proj_does_not_exist")
            XCTFail("an unknown project answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "project-id-unrecognized"))
        }
    }

    /// `queue.rs` `REASONS_NEEDING_A_PERSON`, which the daemon's folder
    /// approve leaves out. Manual Scrub check is not on it: the daemon
    /// approves those sessions when a person approves their folder.
    func testHeldForReviewMirrorsTheDaemonsList() throws {
        func entry(_ reason: String) throws -> DaemonData.QueueEntry {
            let json = #"{"entry_id":"e","source":"codex","project_id":"p","project_label":"p","state":"pending","reason_label":"\#(reason)"}"#
            return try DaemonDataDecoding.decoder().decode(DaemonData.QueueEntry.self, from: Data(json.utf8))
        }
        for reason in [
            "token-distribution-review-required",
            "witness-risk-review-required",
            "privacy-filter-transient-exhausted",
            "second-look-review-required",
        ] {
            XCTAssertTrue(try entry(reason).heldForReview, reason)
        }
        for reason in ["scrub-check-manual", "returned-from-keep", "kept-on-this-mac", "scopes-changed"] {
            XCTAssertFalse(try entry(reason).heldForReview, reason)
        }
    }

    // MARK: - R7: verdict, correction, Undo

    func testApproveSendsTheVerdictAndCorrectionOnlyWhenGiven() async throws {
        let transport = FakeTransport { _, _ in
            #"{"id":0,"result":{"approved":1,"hold_secs":30,"hold_until":null,"flagged":0,"redactions":{},"skipped":[]}}"#
        }
        let client = LiveDaemonClient(transport: transport)
        _ = try await client.approve(entryId: "e1")
        _ = try await client.approve(entryId: "e1", verdict: .worked, correction: nil)
        _ = try await client.approve(entryId: "e1", verdict: .partly, correction: "It missed the retry path")
        XCTAssertEqual(
            transport.calls.map(\.params),
            [
                #"{"entry_id":"e1"}"#,
                #"{"entry_id":"e1","outcome":"worked"}"#,
                #"{"correction":"It missed the retry path","entry_id":"e1","outcome":"partly"}"#,
            ])
    }

    func testCancelSendsTheEntryOrTheFolderAndKeepsTheRefusal() async throws {
        let transport = FakeTransport { _, params in
            params.contains("project_id")
                ? #"{"id":0,"result":{"canceled":3}}"#
                : #"{"id":0,"error":{"code":"bad_params","message":"not-cancelable"}}"#
        }
        let client = LiveDaemonClient(transport: transport)
        let canceled = try await client.cancelFolder(projectId: "proj_1")
        XCTAssertEqual(canceled, 3)
        do {
            try await client.cancel(entryId: "e1")
            XCTFail("a refused Undo answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "not-cancelable"))
        }
        XCTAssertEqual(transport.calls.map(\.method), ["cancel", "cancel"])
        XCTAssertEqual(transport.calls.map(\.params), [#"{"project_id":"proj_1"}"#, #"{"entry_id":"e1"}"#])
    }

    func testSampleRefusesACorrectionWithoutAPartlyOrFailedVerdict() async throws {
        let client = SampleDaemonClient(.normalDay)
        let pending = try await client.listPending(projectId: nil)
        let entry = try XCTUnwrap(pending.first)
        for verdict in [nil, ContributorVerdict.worked] {
            do {
                _ = try await client.approve(entryId: entry.entryId, verdict: verdict, correction: "note")
                XCTFail("a correction without partly or failed answered")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "correction-needs-outcome"))
            }
        }
        let approved = try await client.approve(entryId: entry.entryId, verdict: .failed, correction: "note")
        XCTAssertEqual(approved.approved, 1)
    }
}
