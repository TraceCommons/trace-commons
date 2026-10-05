import XCTest
@testable import TCShellCore

/// SAMPLE transport frames from the finalized C3 contract, never release data.
final class DaemonNetworkContractTests: XCTestCase {
    func testUnavailableSummaryIsNotMeasuredEmptyData() async throws {
        let transport = NetworkTransport(#"{"readable":false,"window_hours":24,"observed_at":null,"summary":null}"#)
        let result = try await LiveDaemonClient(transport: transport).inferenceSummary()
        let object = try wire(result)
        XCTAssertEqual(object["readable"] as? Bool, false)
        XCTAssertNil(object["models"], "the upstream summary must not become model-only rows")
        XCTAssertEqual(transport.method, "inference_summary")
    }

    func testProofDetailIsStoredLabelWithoutFabricatedChecks() async throws {
        let transport = NetworkTransport(#"{"call_id":412,"proof":"gateway_only","checked_at":null,"checks":null,"readable":true,"found":true}"#)
        let result = try await LiveDaemonClient(transport: transport).inferenceCallProof(callId: 412)
        let object = try wire(result)
        XCTAssertEqual(object["readable"] as? Bool, true)
        XCTAssertEqual(object["found"] as? Bool, true)
        XCTAssertNil(result.checks)
        XCTAssertNil(result.checkedAt)
        XCTAssertEqual(transport.method, "inference_call_proof")
        XCTAssertEqual(try transport.parameters()["call_id"] as? Int, 412)
    }

    func testUnknownSpendPreservesSourceReason() async throws {
        let transport = NetworkTransport(#"{"known":false,"since":null,"models":[],"reason_label":"billed-model-spend-unavailable"}"#)
        let result = try await LiveDaemonClient(transport: transport).modelSpend()
        XCTAssertFalse(result.known)
        XCTAssertEqual(try wire(result)["reason_label"] as? String, "billed-model-spend-unavailable")
        XCTAssertEqual(transport.method, "model_spend")
    }

    func testPrivateAIReadPreservesUnreadableState() async throws {
        let transport = NetworkTransport(#"{"on":null,"state":null,"port":null,"disclosure":"SAMPLE core disclosure"}"#)
        let result = try await LiveDaemonClient(transport: transport).privateAI()
        XCTAssertNil(result.on)
        XCTAssertNil(result.state)
        XCTAssertEqual(result.disclosure, "SAMPLE core disclosure")
        XCTAssertEqual(transport.method, "private_ai")
    }

    func testMissionCatalogueKeepsSkillEvaluationWrapper() async throws {
        let transport = NetworkTransport(#"{"kind":"skill_evaluation","disclosure":"SAMPLE core consent","catalogue":{"schema_version":1,"entries":[],"next_cursor":null}}"#)
        let result = try await LiveDaemonClient(transport: transport).missionCatalogue()
        let object = try wire(result)
        XCTAssertEqual(object["kind"] as? String, "skill_evaluation")
        XCTAssertNotNil(object["catalogue"])
        XCTAssertEqual(object["disclosure"] as? String, "SAMPLE core consent")
        XCTAssertNil(object["missions"], "catalogue entries have no daily rewards")
        XCTAssertEqual(transport.method, "mission_catalogue")
        XCTAssertTrue(try transport.parameters().isEmpty, "matching stays local")
    }

    func testInviteLookupSendsFullURLOnlyInCodeParameter() async throws {
        let transport = NetworkTransport(#"{"valid":true,"issuer_display_name":"SAMPLE Pilot","credit_range":{"min":1,"max":5,"unit":"points_per_accepted_trace"}}"#)
        let result = try await LiveDaemonClient(transport: transport).lookupInvite(code: "https://issuer.example/onboard#SAMPLE-CODE")
        XCTAssertEqual(result.creditRange?.unit, "points_per_accepted_trace")
        XCTAssertEqual(transport.method, "invite_lookup")
        XCTAssertEqual(try transport.parameters()["code"] as? String, "https://issuer.example/onboard#SAMPLE-CODE")
        XCTAssertEqual(try transport.parameters().count, 1)
    }

    func testPasskeyUnknownDoesNotBecomeNoPasskeys() async throws {
        let transport = NetworkTransport(#"{"state":"unknown","passkey_count":null,"near_ai_connected":null}"#)
        let result = try await LiveDaemonClient(transport: transport).passkeyState()
        XCTAssertEqual(result.state, "unknown")
        XCTAssertNil(result.passkeyCount)
        XCTAssertNil(result.nearAiConnected)
        XCTAssertEqual(transport.method, "passkey_state")
    }

    func testAccountUnknownDoesNotBecomeSignedOut() async throws {
        let transport = NetworkTransport(#"{"state":"unknown","signed_in":null,"account_id":null,"expires_at":null}"#)
        let result = try await LiveDaemonClient(transport: transport).accountState()
        XCTAssertNil(result.signedIn)
        XCTAssertEqual(try wire(result)["state"] as? String, "unknown")
        XCTAssertEqual(transport.method, "account_session_status")
    }

    func testPrivateAIWriteSendsConfirmedOnlyWithAConsent() async throws {
        let consent = try XCTUnwrap(DaemonData.PrivateAIConsent(acknowledging: privateAISwitch("SAMPLE core disclosure")))
        for given in [nil, consent] {
            let transport = NetworkTransport(#"{"on":false,"state":"off","port":null,"disclosure":"SAMPLE core disclosure"}"#)
            _ = try await LiveDaemonClient(transport: transport).setPrivateAI(on: false, consent: given)
            XCTAssertEqual(transport.method, "set_private_ai")
            XCTAssertEqual(try transport.parameters()["on"] as? Bool, false)
            XCTAssertEqual(try transport.parameters()["confirmed"] as? Bool, given != nil)
        }
        let refusal = NetworkTransport(error: #"{"code":"bad_params","message":"confirmation-required"}"#)
        do {
            _ = try await LiveDaemonClient(transport: refusal).setPrivateAI(on: true, consent: consent)
            XCTFail("a daemon refusal succeeded")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "confirmation-required"))
        }
        let accepted = NetworkTransport(#"{"on":true,"state":"running","port":3128,"disclosure":"SAMPLE core disclosure"}"#)
        let result = try await LiveDaemonClient(transport: accepted).setPrivateAI(on: true, consent: consent)
        XCTAssertEqual(result.port, 3128)
        XCTAssertEqual(try accepted.parameters()["confirmed"] as? Bool, true)
    }

    /// Enabling Private AI needs a consent, and a consent can only be built
    /// from a `PrivateAISwitch` the core answered: nothing reaches `tc_call`
    /// without one, so a future toggle cannot enable without first holding
    /// the switch whose disclosure it shows.
    func testEnablingWithoutConsentIsRefusedBeforeTheDaemonIsAsked() async {
        let transport = NetworkTransport(#"{"on":true,"state":"running","port":3128,"disclosure":"SAMPLE core disclosure"}"#)
        do {
            _ = try await LiveDaemonClient(transport: transport).setPrivateAI(on: true, consent: nil)
            XCTFail("enabled with no consent")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "confirmation-required"))
        }
        XCTAssertNil(transport.method, "an unconsented enable reached the daemon")
    }

    func testConsentCarriesTheDigestOfTheCoreDisclosureAndNeedsOne() throws {
        let consent = try XCTUnwrap(DaemonData.PrivateAIConsent(acknowledging: privateAISwitch("SAMPLE core disclosure")))
        XCTAssertEqual(consent.disclosureSHA256, "41865ed6f1624a37db9345f6bf74d75df6abc9fea511b18be07c2f97416fc96d")
        for blank in ["", "  \n"] {
            XCTAssertNil(DaemonData.PrivateAIConsent(acknowledging: try privateAISwitch(blank)),
                         "a consent was built from a switch with no disclosure")
        }
    }

    func testEverySampleIsMarkedAndSpendIsUnknown() async throws {
        XCTAssertEqual(SampleDaemonClient.SampleSet.allCases.count, 7)
        for set in SampleDaemonClient.SampleSet.allCases where set != .coreDown {
            let client = SampleDaemonClient(set)
            for method in ["status", "list_pending", "list_kept", "list_projects", "harness_list", "get_settings",
                           "list_history", "history_rollup", "commons_credit_summary", "tool_destinations", "inference_calls",
                           "inference_summary", "inference_call_proof", "model_spend", "private_ai", "mission_catalogue",
                           "invite_lookup", "passkey_state", "account_session_status"] {
                let json = try XCTUnwrap(client.json(for: method))
                let object = try XCTUnwrap(JSONSerialization.jsonObject(with: Data(json.utf8)) as? [String: Any])
                XCTAssertEqual(object["_sample"] as? String, "SAMPLE", "\(set): \(method)")
            }
            let spend = try await client.modelSpend()
            XCTAssertFalse(spend.known, "default previews make no provider billing claim")
            XCTAssertTrue(spend.models.isEmpty)
            XCTAssertEqual(spend.reasonLabel, "billed-model-spend-unavailable")
        }
    }

    func testSampleUnknownNetworkFactsStayUnknown() async throws {
        let client = SampleDaemonClient(.unknownCounts)
        let summary = try await client.inferenceSummary()
        XCTAssertFalse(summary.readable)
        XCTAssertNil(summary.summary)
        let privateAI = try await client.privateAI()
        XCTAssertNil(privateAI.on)
        XCTAssertNil(privateAI.state)
        let passkey = try await client.passkeyState()
        XCTAssertEqual(passkey.state, "unknown")
        XCTAssertNil(passkey.passkeyCount)
        XCTAssertNil(passkey.nearAiConnected)
        let account = try await client.accountState()
        XCTAssertEqual(account.state, "unknown")
        XCTAssertNil(account.signedIn)
    }

    func testSampleEnablingNeedsAcknowledgement() async {
        do {
            _ = try await SampleDaemonClient(.normalDay).setPrivateAI(on: true, consent: nil)
            XCTFail("sample implied consent")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "bad_params", message: "confirmation-required"))
        }
    }

    func testLiveSummaryPreservesGroupsRouteTotalsAndIncompletePricing() async throws {
        let json = try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "inference_summary"))
        let result = try await LiveDaemonClient(transport: NetworkTransport(json)).inferenceSummary()
        let summary = try XCTUnwrap(result.summary)
        XCTAssertTrue(summary.enabled)
        XCTAssertTrue(summary.receipts)
        XCTAssertEqual(summary.groups.count, 2)
        XCTAssertEqual(summary.groups.map(\.model), ["example-model", "example-model"])
        XCTAssertEqual(summary.groups.map(\.backend), ["nearai", "sha256:" + String(repeating: "c", count: 64)])
        XCTAssertEqual(summary.groups.map(\.route), ["routed", "outside"])
        XCTAssertEqual(Set(summary.groups.map(\.id)).count, 2)
        XCTAssertNil(summary.groups[0].workKind)
        XCTAssertEqual(summary.routed.calls, 3)
        XCTAssertEqual(summary.routed.pricedCalls, 2)
        XCTAssertEqual(summary.routed.costUSD, 0.02)
        XCTAssertEqual(summary.routed.proof.verified, 1)
        XCTAssertEqual(summary.routed.proof.failed, 1)
        XCTAssertEqual(summary.outside.calls, 1)
        XCTAssertEqual(summary.unknown.calls, 0)
    }

    func testGroupIdentityRetainsEachGroupingKeyWithoutStringCollisions() throws {
        let base = #"{"model":"m","backend":"nearai","route":"routed","work_kind":null,"calls":1,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":1,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}}"#
        let variants = [
            base,
            base.replacingOccurrences(of: #""model":"m""#, with: #""model":null"#),
            base.replacingOccurrences(of: "nearai", with: "api_key"),
            base.replacingOccurrences(of: #""route":"routed""#, with: #""route":"outside""#),
            // A future authoritative classification must also remain part of identity.
            base.replacingOccurrences(of: #""work_kind":null"#, with: #""work_kind":"source-backed""#),
        ]
        let groups = try variants.map {
            try DaemonDataDecoding.decoder().decode(DaemonData.InferenceSummaryGroup.self, from: Data($0.utf8))
        }
        XCTAssertEqual(Set(groups.map(\.id)).count, 5)
    }

    func testLiveCataloguePreservesEntryAndCoreDisclosureWithoutRewards() async throws {
        let json = try XCTUnwrap(SampleDaemonClient(.normalDay).json(for: "mission_catalogue"))
        let result = try await LiveDaemonClient(transport: NetworkTransport(json)).missionCatalogue()
        XCTAssertEqual(result.kind, "skill_evaluation")
        XCTAssertEqual(result.disclosure, "SAMPLE: catalogue consent disclosure from the Rust core")
        XCTAssertEqual(result.catalogue.schemaVersion, 1)
        let entry = try XCTUnwrap(result.catalogue.entries.first)
        XCTAssertEqual(entry.missionId, "00000000-0000-4000-8000-000000000001")
        XCTAssertEqual(entry.programId, "00000000-0000-4000-8000-000000000002")
        XCTAssertEqual(entry.packageSHA256, String(repeating: "a", count: 64))
        XCTAssertEqual(entry.offerVersionHash, "sha256:" + String(repeating: "b", count: 64))
        XCTAssertEqual(entry.taskPreview, "SAMPLE: evaluate a reviewed skill package against its controls.")
        XCTAssertEqual(entry.publishedAt, DaemonDataDecoding.parseDate("2026-10-01T00:00:00Z"))
        XCTAssertNil(try wire(entry)["credit_range"])
        XCTAssertNil(result.catalogue.nextCursor)
    }

    func testSourceGroupIDPreventsSanitizedTupleCollisions() throws {
        let base = #"{"model":"sanitized-model","backend":"nearai","route":"routed","work_kind":null,"calls":1,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":1,"unavailable":0,"failed":0,"outside":0,"unrecorded":0},"group_id":"sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
        let second = base.replacingOccurrences(of: String(repeating: "a", count: 64), with: String(repeating: "b", count: 64))
        let renamed = base.replacingOccurrences(of: "sanitized-model", with: "new-display-label")
        let groups = try [base, second, renamed].map {
            try DaemonDataDecoding.decoder().decode(DaemonData.InferenceSummaryGroup.self, from: Data($0.utf8))
        }
        XCTAssertNotEqual(groups[0].id, groups[1].id)
        XCTAssertEqual(groups[0].id, groups[2].id)
        XCTAssertEqual(try wire(groups[0])["group_id"] as? String, "sha256:" + String(repeating: "a", count: 64))
        let malformed = base.replacingOccurrences(of: "sha256:" + String(repeating: "a", count: 64), with: "invalid")
        XCTAssertThrowsError(try DaemonDataDecoding.decoder().decode(DaemonData.InferenceSummaryGroup.self, from: Data(malformed.utf8)))
    }

    func testFullLiveSummaryPathKeepsUnsignedCountersAboveInt64() async throws {
        let proof = #"{"verified":18446744073709551615,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}"#
        let total = #"{"calls":18446744073709551615,"priced_calls":0,"cost_usd":0.0,"proof":\#(proof)}"#
        let zero = #"{"calls":0,"priced_calls":0,"cost_usd":0.0,"proof":{"verified":0,"gateway_only":0,"unattested":0,"pending":0,"unavailable":0,"failed":0,"outside":0,"unrecorded":0}}"#
        let group = #"{"model":null,"backend":"nearai","route":"routed","work_kind":null,"calls":18446744073709551615,"priced_calls":0,"cost_usd":0.0,"proof":\#(proof)}"#
        let reply = #"{"readable":true,"window_hours":24,"observed_at":"2026-10-02T00:00:00Z","summary":{"enabled":true,"receipts":true,"since":"2026-10-01T00:00:00Z","groups":[\#(group)],"routed":\#(total),"outside":\#(zero),"unknown":\#(zero)}}"#
        let result = try await LiveDaemonClient(transport: NetworkTransport(reply)).inferenceSummary()
        let summary = try XCTUnwrap(result.summary)
        let object = try wire(summary)
        let routed = try XCTUnwrap(object["routed"] as? [String: Any])
        XCTAssertEqual(routed["calls"] as? UInt64, UInt64.max)
        let counts = try XCTUnwrap(routed["proof"] as? [String: Any])
        XCTAssertEqual(counts["verified"] as? UInt64, UInt64.max)
    }

    func testFreedTransportHandleIsCoreDownForLiveNetworkReads() async {
        let transport = NetworkTransport(error: #"{"code":"unavailable","message":"handle-freed"}"#)
        do {
            _ = try await LiveDaemonClient(transport: transport).modelSpend()
            XCTFail("freed transport answered")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .unreachable)
        }
    }

    /// The sample answers what was asked: turning Private AI off answers
    /// off, even in a set whose switch reads on.
    func testSampleTurningOffAnswersOff() async throws {
        for set in [SampleDaemonClient.SampleSet.normalDay, .busyQueue, .empty] {
            let result = try await SampleDaemonClient(set).setPrivateAI(on: false, consent: nil)
            XCTAssertEqual(result.on, false, "\(set)")
            XCTAssertEqual(result.state, "off", "\(set)")
            XCTAssertNil(result.port, "\(set)")
            XCTAssertFalse(result.disclosure.isEmpty, "\(set)")
        }
        let shown = try await SampleDaemonClient(.empty).privateAI()
        let consent = try XCTUnwrap(DaemonData.PrivateAIConsent(acknowledging: shown))
        let on = try await SampleDaemonClient(.empty).setPrivateAI(on: true, consent: consent)
        XCTAssertEqual(on.on, true)
        XCTAssertEqual(on.state, "running")
        XCTAssertNotNil(on.port)
    }

    private func privateAISwitch(_ disclosure: String) throws -> DaemonData.PrivateAISwitch {
        let json = try JSONSerialization.data(withJSONObject: ["on": false, "state": "off", "port": NSNull(), "disclosure": disclosure])
        return try DaemonDataDecoding.decoder().decode(DaemonData.PrivateAISwitch.self, from: json)
    }

    private func wire<T: Encodable>(_ value: T) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: JSONEncoder().encode(value)) as? [String: Any])
    }
}

private final class NetworkTransport: DaemonTransport, @unchecked Sendable {
    let response: String
    private(set) var method: String?
    private var params = "{}"

    init(_ result: String) { response = #"{"id":1,"result":\#(result)}"# }

    init(error: String) { response = #"{"id":1,"error":\#(error)}"# }

    func call(_ method: String, params paramsJSON: String) -> String {
        self.method = method
        params = paramsJSON
        return response
    }

    func parameters() throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(params.utf8)) as? [String: Any])
    }
}
