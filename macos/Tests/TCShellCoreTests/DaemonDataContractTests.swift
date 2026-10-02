import XCTest
@testable import TCShellCore

/// C1 of #1173: the data contract decodes every sample set, keeps unknown
/// as unknown, and fails closed when the core is down.
final class DaemonDataContractTests: XCTestCase {
    private let reachableSets = SampleDaemonClient.SampleSet.allCases.filter { $0 != .coreDown }

    // MARK: - Every model type decodes from every sample set

    func testEveryMethodDecodesInEverySampleSet() async throws {
        for set in reachableSets {
            let client = SampleDaemonClient(set)
            do {
                _ = try await client.status()
                let pending = try await client.listPending(projectId: nil)
                let kept = try await client.listKept()
                for entry in pending + kept {
                    let summary = try await client.preview(entryId: entry.entryId)
                    XCTAssertEqual(summary.entry?.entryId, entry.entryId)
                    XCTAssertNotNil(summary.title)
                }
                for entry in pending {
                    let outcome = try await client.requestPreview(entryId: entry.entryId)
                    XCTAssertEqual(outcome.state, .ready)
                    // A scheduled card (`PreviewOutcome::to_value`) carries
                    // no entry: it outlives the entry state it was built beside.
                    XCTAssertNotNil(outcome.summary?.title)
                    XCTAssertNil(outcome.summary?.entry)
                    _ = try await client.approve(entryId: entry.entryId)
                }
                _ = try await client.setVisiblePreviews(entryIds: pending.map(\.entryId))
                _ = try await client.cancelPreview(entryId: "e")
                _ = try await client.previewUnsureSpans(entryId: "e", bodyDigest: "sha256:b")
                _ = try await client.keep(entryId: "e")
                _ = try await client.undoKeep(entryId: "e")
                try await client.dismiss(entryId: "e")
                _ = try await client.listProjects()
                _ = try await client.setProjectMode(projectId: "p", mode: .autoUpload, includeBacklog: nil)
                _ = try await client.harnessList()
                _ = try await client.settings()
                _ = try await client.setScrubCheck(.manual)
                _ = try await client.setLocalNotifications(true)
                _ = try await client.setDigestSchedule(.evening(hour: 18))
                _ = try await client.listHistory(limit: 50)
                _ = try await client.historyRollup()
                _ = try await client.commonsCreditSummary()
                _ = try await client.toolDestinations()
                _ = try await client.inferenceCalls(limit: 50, cursor: nil)
                _ = try await client.inferenceSummary()
                _ = try await client.inferenceCallProof(callId: 414)
                _ = try await client.modelSpend()
                _ = try await client.privateAI()
                _ = try await client.setPrivateAI(on: true)
                _ = try await client.missionCatalogue()
                _ = try await client.lookupInvite(code: "c")
                _ = try await client.passkeyState()
                _ = try await client.accountState()
            } catch {
                XCTFail("\(set): \(error)")
            }
        }
    }

    // MARK: - Unknown stays unknown

    func testAbsentDecisionsOwedDecodesAsNil() async throws {
        let status = try await SampleDaemonClient(.unknownCounts).status()
        XCTAssertNil(status.decisionsOwed)
        XCTAssertNotNil(status.queueDepth, "queue_depth is present and must not stand in for it")
    }

    func testNullAndAbsentDecisionsOwedBothDecodeAsNilAndZeroStaysZero() throws {
        let decoder = DaemonDataDecoding.decoder()
        let absent = try decoder.decode(DaemonData.Status.self, from: Data(#"{"queue_depth":5}"#.utf8))
        XCTAssertNil(absent.decisionsOwed)
        let null = try decoder.decode(DaemonData.Status.self, from: Data(#"{"decisions_owed":null}"#.utf8))
        XCTAssertNil(null.decisionsOwed)
        let zero = try decoder.decode(DaemonData.Status.self, from: Data(#"{"decisions_owed":0}"#.utf8))
        XCTAssertEqual(zero.decisionsOwed, 0)
    }

    func testArmedFolderOwesFewerDecisionsThanQueueDepth() async throws {
        let status = try await SampleDaemonClient(.armedFolder).status()
        XCTAssertEqual(status.queueDepth, 3)
        XCTAssertEqual(status.decisionsOwed, 1)
        let projects = try await SampleDaemonClient(.armedFolder).listProjects()
        XCTAssertEqual(projects.projects.first?.mode, .autoUpload)
        XCTAssertEqual(projects.projects.first?.fromNow, true)
    }

    func testUnscrubbedEntryHasNoMarksRatherThanZero() async throws {
        let pending = try await SampleDaemonClient(.normalDay).listPending(projectId: nil)
        let unscrubbed = try XCTUnwrap(pending.first { $0.scrubState == .notYetScrubbed })
        XCTAssertNil(unscrubbed.marks)
        XCTAssertNil(unscrubbed.contentMarks)
        XCTAssertNil(unscrubbed.unsureSpans)
        let scrubbed = try XCTUnwrap(pending.first { $0.scrubState == .scrubbed })
        XCTAssertEqual(scrubbed.marks, 7)
    }

    func testEntryWithoutShapeHasNilSessionFields() async throws {
        let pending = try await SampleDaemonClient(.heldSessions).listPending(projectId: nil)
        let old = try XCTUnwrap(pending.first { $0.startedAt == nil })
        XCTAssertNil(old.endedAt)
        XCTAssertNil(old.durationSecs)
        XCTAssertNil(old.userTurns)
        let shaped = try XCTUnwrap(pending.first { $0.startedAt != nil })
        XCTAssertNotNil(shaped.userTurns)
        XCTAssertEqual(
            shaped.durationSecs, Int(try XCTUnwrap(shaped.endedAt).timeIntervalSince(try XCTUnwrap(shaped.startedAt))))
    }

    func testUnknownCreditAndSpendStayUnknown() async throws {
        let client = SampleDaemonClient(.unknownCounts)
        let credit = try await client.commonsCreditSummary()
        XCTAssertFalse(credit.pointsKnown)
        XCTAssertNil(credit.commonsPointsEarnedThisPeriod)
        XCTAssertNil(credit.commonsSettlement)
        let harness = try await client.harnessList()
        XCTAssertFalse(harness.spend.known)
        XCTAssertNil(harness.spend.micros)
        let calls = try await client.inferenceCalls(limit: 50, cursor: nil)
        XCTAssertFalse(calls.readable)
    }

    // MARK: - Held sessions, Keep, provenance, proof

    func testHeldSessionsCarryBothHolds() async throws {
        let pending = try await SampleDaemonClient(.heldSessions).listPending(projectId: nil)
        XCTAssertTrue(pending.contains { $0.heldForSecondLook })
        XCTAssertTrue(pending.contains { $0.heldByManualScrubCheck })
        let held = try XCTUnwrap(pending.first { $0.heldForSecondLook && $0.secondLook?.contains("trimmed-to-fit") == true })
        XCTAssertEqual(held.scrubState, .scrubbed)
        let settings = try await SampleDaemonClient(.heldSessions).settings()
        XCTAssertEqual(settings.scrubCheckMode, .manual)
    }

    func testKeptAndReturnedFromKeep() async throws {
        let client = SampleDaemonClient(.normalDay)
        let kept = try await client.listKept()
        XCTAssertEqual(kept.first?.isKept, true)
        XCTAssertEqual(kept.first?.queueState, .refused)
        let pending = try await client.listPending(projectId: nil)
        XCTAssertTrue(pending.contains { $0.returnedFromKeep })
    }

    func testHistoryProvenanceNotRecordedIsDistinct() async throws {
        let rows = try await SampleDaemonClient(.normalDay).listHistory(limit: 50)
        XCTAssertEqual(Set(rows.map(\.provenance)).count, 3)
        XCTAssertTrue(rows.contains { $0.provenance == .notRecorded })
        let rollup = try await SampleDaemonClient(.normalDay).historyRollup()
        XCTAssertEqual(rollup.takenBack, 1)
    }

    func testOnlyVerifiedIsProof() async throws {
        let calls = try await SampleDaemonClient(.normalDay).inferenceCalls(limit: 50, cursor: nil).calls
        XCTAssertEqual(calls.filter { $0.proofLabel.isProof }.map(\.proof), ["verified"])
        XCTAssertTrue(calls.contains { $0.proofLabel == .failed })
        XCTAssertEqual(DaemonData.ProofLabel.allCases.filter(\.isProof), [.verified])
        let unknown = try DaemonDataDecoding.decoder().decode(
            DaemonData.InferenceCall.self,
            from: Data(#"{"id":1,"at":"2026-09-30T09:00:00Z","tool":"unknown","family":"unknown","model":"m","route":"unknown","proof":"some-new-label"}"#.utf8))
        XCTAssertEqual(unknown.proofLabel, .unrecorded)
    }

    func testProjectsAndToolsCarryK1AndK2Fields() async throws {
        let client = SampleDaemonClient(.normalDay)
        let projects = try await client.listProjects()
        XCTAssertNotNil(projects.projects.first?.sessionCount)
        XCTAssertNotNil(projects.projects.first?.lastSessionAt)
        let harness = try await client.harnessList()
        XCTAssertEqual(harness.harnesses.map(\.answersAt), ["Anthropic", "OpenAI"])
        let settings = try await client.settings()
        XCTAssertEqual(settings.digestSchedule, .evening(hour: 18))
        XCTAssertEqual(settings.localNotifications, true)
    }

    // MARK: - Core down

    func testCoreDownThrowsUnreachableEverywhere() async {
        let client = SampleDaemonClient(.coreDown)
        let calls: [(String, () async throws -> Void)] = [
            ("status", { _ = try await client.status() }),
            ("listPending", { _ = try await client.listPending(projectId: nil) }),
            ("listKept", { _ = try await client.listKept() }),
            ("preview", { _ = try await client.preview(entryId: "e") }),
            ("requestPreview", { _ = try await client.requestPreview(entryId: "e") }),
            ("setVisiblePreviews", { _ = try await client.setVisiblePreviews(entryIds: ["e"]) }),
            ("cancelPreview", { _ = try await client.cancelPreview(entryId: "e") }),
            ("previewUnsureSpans", { _ = try await client.previewUnsureSpans(entryId: "e", bodyDigest: "d") }),
            ("approve", { _ = try await client.approve(entryId: "e") }),
            ("keep", { _ = try await client.keep(entryId: "e") }),
            ("undoKeep", { _ = try await client.undoKeep(entryId: "e") }),
            ("dismiss", { try await client.dismiss(entryId: "e") }),
            ("listProjects", { _ = try await client.listProjects() }),
            ("setProjectMode", { _ = try await client.setProjectMode(projectId: "p", mode: .ask, includeBacklog: nil) }),
            ("harnessList", { _ = try await client.harnessList() }),
            ("settings", { _ = try await client.settings() }),
            ("setScrubCheck", { _ = try await client.setScrubCheck(.automatic) }),
            ("setLocalNotifications", { _ = try await client.setLocalNotifications(false) }),
            ("setDigestSchedule", { _ = try await client.setDigestSchedule(.interval) }),
            ("listHistory", { _ = try await client.listHistory(limit: 5) }),
            ("historyRollup", { _ = try await client.historyRollup() }),
            ("commonsCreditSummary", { _ = try await client.commonsCreditSummary() }),
            ("toolDestinations", { _ = try await client.toolDestinations() }),
            ("inferenceCalls", { _ = try await client.inferenceCalls(limit: 5, cursor: nil) }),
            ("inferenceSummary", { _ = try await client.inferenceSummary() }),
            ("inferenceCallProof", { _ = try await client.inferenceCallProof(callId: 1) }),
            ("modelSpend", { _ = try await client.modelSpend() }),
            ("privateAI", { _ = try await client.privateAI() }),
            ("setPrivateAI", { _ = try await client.setPrivateAI(on: false) }),
            ("missionCatalogue", { _ = try await client.missionCatalogue() }),
            ("lookupInvite", { _ = try await client.lookupInvite(code: "c") }),
            ("passkeyState", { _ = try await client.passkeyState() }),
            ("accountState", { _ = try await client.accountState() }),
        ]
        for (name, call) in calls {
            do {
                try await call()
                XCTFail("\(name) answered while the core is down")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, .unreachable, name)
            }
        }
    }

    func testCoreDownEventStreamFinishesAtOnce() async {
        var count = 0
        for await _ in SampleDaemonClient(.coreDown).events() { count += 1 }
        XCTAssertEqual(count, 0)
    }

    func testSampleEventsOpenWithASnapshot() async throws {
        var iterator = SampleDaemonClient(.normalDay).events().makeAsyncIterator()
        guard case .snapshot(let pending, let status)? = await iterator.next() else {
            return XCTFail("first event is not a snapshot")
        }
        XCTAssertEqual(pending.count, 3)
        XCTAssertEqual(status?.decisionsOwed, 3)
    }

    // MARK: - Live client over a fake transport

    func testLiveClientDecodesARealStatusReply() async throws {
        // Captured from `status` on main against a temp store.
        let transport = FakeTransport(response: #"{"id":1,"result":{"schema_version":"trace_commons.daemon.v1_1","logged_in":false,"tenant_id":null,"consent_scopes":["debugging_evaluation"],"paused":false,"queue_depth":1,"decisions_owed":1,"next_digest_at":null,"health":{"last_error_label":null,"since":null},"daily_budget":{"bytes_today":0,"max_bytes_per_day":209715200,"bytes_remaining":209715200,"uploads_today":0,"max_uploads_per_day":50,"uploads_remaining":50,"resets_at":"2026-10-02T00:00:00Z","blocked":false,"blocked_entries":0,"blocked_bytes":0},"routing":{"state":"not_declared","derived":false,"last_refresh_at":null,"unreadable_rows":0},"private_inference_state":{"state":"off","port":null},"grant_voids":[],"witness_capacity":{"waiting_sessions":0,"next_retry_at":null},"legacy_invite_migration":{"offered":false,"notice":null},"arming_rewordings":[],"automatic_contribution_held":{"held_sessions":0,"reasons":[],"projects":[]}}}"#)
        let status = try await LiveDaemonClient(transport: transport).status()
        XCTAssertEqual(status.decisionsOwed, 1)
        XCTAssertEqual(status.routing?.state, "not_declared")
        XCTAssertEqual(transport.calls.first?.method, "status")
    }

    func testLiveClientSendsTheMethodAndParams() async throws {
        let transport = FakeTransport(response: #"{"id":1,"result":{"kept":true}}"#)
        let kept = try await LiveDaemonClient(transport: transport).keep(entryId: "abc")
        XCTAssertTrue(kept.kept)
        XCTAssertEqual(transport.calls.first?.method, "keep")
        XCTAssertEqual(transport.calls.first?.params, #"{"entry_id":"abc"}"#)

        let settings = FakeTransport(response: #"{"id":1,"result":{"scrub_check":"manual"}}"#)
        _ = try await LiveDaemonClient(transport: settings).setScrubCheck(.manual)
        XCTAssertEqual(settings.calls.first?.method, "set_settings")
        XCTAssertEqual(settings.calls.first?.params, #"{"scrub_check":"manual"}"#)
    }

    func testLiveClientMapsAStoppedDaemonToUnreachable() async {
        let transport = FakeTransport(response: #"{"id":0,"error":{"code":"unavailable","message":"daemon-stopped"}}"#)
        do {
            _ = try await LiveDaemonClient(transport: transport).status()
            XCTFail("a stopped daemon answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
    }

    func testLiveClientKeepsTheDaemonsRefusal() async {
        let transport = FakeTransport(response: #"{"id":1,"error":{"code":"bad_params","message":"not-kept"}}"#)
        do {
            _ = try await LiveDaemonClient(transport: transport).undoKeep(entryId: "x")
            XCTFail("a refusal answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "not-kept"))
        }
    }

    func testLiveClientThrowsNotAvailableYetForProvisionalMethods() async {
        let transport = FakeTransport(response: #"{"id":1,"result":{}}"#)
        let client = LiveDaemonClient(transport: transport)
        do {
            _ = try await client.missionCatalogue()
            XCTFail("a provisional method answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .notAvailableYet(method: "mission_catalogue"))
        }
        XCTAssertTrue(transport.calls.isEmpty, "nothing is sent for a method the daemon does not have")
    }

    func testLiveEventsDeliverParsedFrames() async {
        let client = LiveDaemonClient(transport: FakeTransport(response: "{}"))
        let stream = client.events()
        client.deliver(eventJSON: #"{"event":"status_changed","data":{}}"#)
        // The daemon's frame (`inference_map::call_added`): a pulse with four
        // fields, not an `inference_calls` row. No `at`, `family`, `route`
        // or `cost`.
        client.deliver(eventJSON: #"{"event":"inference_call_added","data":{"id":7,"tool":"codex","model":"m","proof":"pending"}}"#)
        var iterator = stream.makeAsyncIterator()
        let first = await iterator.next()
        XCTAssertEqual(first, .statusChanged)
        guard case .inferenceCallAdded(let call)? = await iterator.next() else {
            return XCTFail("no inference call event")
        }
        XCTAssertEqual(call, DaemonData.InferenceCallAdded(id: 7, tool: "codex", model: "m", proof: "pending"))
        XCTAssertEqual(call.proofLabel, .pending)
    }
}

/// A transport that answers from a script and records what it was sent,
/// and whether it ran on the live client's own queue.
final class FakeTransport: DaemonTransport, @unchecked Sendable {
    struct Call {
        let method: String
        let params: String
        let onQueue: Bool
    }

    private let answer: @Sendable (String, String) -> String
    private let lock = NSLock()
    private var recorded: [Call] = []

    var calls: [Call] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }

    convenience init(response: String) {
        self.init { _, _ in response }
    }

    init(_ answer: @escaping @Sendable (String, String) -> String) {
        self.answer = answer
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        let onQueue = String(cString: __dispatch_queue_get_label(nil)) == "trace-commons.live-daemon-client"
        lock.lock()
        recorded.append(Call(method: method, params: paramsJSON, onQueue: onQueue))
        lock.unlock()
        return answer(method, paramsJSON)
    }
}
