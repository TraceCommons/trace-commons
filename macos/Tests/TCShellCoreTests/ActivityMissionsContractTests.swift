import XCTest
@testable import TCShellCore

/// SAMPLE policy and frames for wire validation only; no official policy exists.
final class ActivityMissionsContractTests: XCTestCase {
    private let unconfigured = #"{"schema_version":1,"kind":"trace_activity","state":"unconfigured","policy_sha256":null,"policy":null,"rewards_enabled":false,"credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"}"#
    private let policy = #"{"schema_version":1,"policy_id":"SAMPLE-policy","starts_on":"2026-10-01","ends_before":"2026-11-01","qualification":"accepted","missions":[{"id":"SAMPLE-one","title":"SAMPLE contribution threshold","required_contributions":2}],"daily":{"kind":"fixed","mission_id":"SAMPLE-one"},"levels":null,"badges":null}"#
    private let progress = #"{"schema_version":1,"kind":"trace_activity","policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","observed_at":"2026-10-02T12:00:00Z","source":"account_contributed_submissions","qualification":"accepted","coverage_starts_on":"2026-10-01","month_starts_on":"2026-10-01","monthly_contributions":3,"daily":{"day":"2026-10-02","mission_id":"SAMPLE-one","contributions":1,"required_contributions":2,"complete":false},"completed_days":1,"current_streak":1,"level":null,"levels_configured":false,"badges":null,"rewards_enabled":false,"credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"}"#

    func testUnconfiguredCatalogueIsDistinctFromSkillCatalogueAndCarriesDisclosure() async throws {
        let transport = ActivityTransport(result: #"{"catalogue":\#(unconfigured),"disclosure":"SAMPLE core disclosure"}"#)
        let client: any DaemonDataClient = LiveDaemonClient(transport: transport)
        let result = try await client.activityMissionsCatalogue()
        XCTAssertEqual(result.disclosure, "SAMPLE core disclosure")
        XCTAssertEqual(result.catalogue.kind, "trace_activity")
        XCTAssertEqual(result.catalogue.state, "unconfigured")
        XCTAssertNil(result.catalogue.policy)
        XCTAssertNil(result.catalogue.policySHA256)
        XCTAssertFalse(result.catalogue.rewardsEnabled)
        XCTAssertNil(result.catalogue.creditPointsPending)
        XCTAssertEqual(transport.method, "activity_missions_catalogue")
        XCTAssertEqual(transport.params, "{}")
    }

    func testConfiguredPolicyRetainsUTCSelectionAndAbsentGamification() throws {
        let catalogue = #"{"schema_version":1,"kind":"trace_activity","state":"configured","policy_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","policy":\#(policy),"rewards_enabled":false,"credit_points_pending":null,"credit_condition":"mission_credit_ledger_unavailable"}"#
        let value = try decodeCatalogue(catalogue)
        let selected = try XCTUnwrap(value.catalogue.policy)
        XCTAssertEqual(selected.startsOn, "2026-10-01")
        XCTAssertEqual(selected.endsBefore, "2026-11-01")
        XCTAssertEqual(selected.qualification, "accepted")
        XCTAssertEqual(selected.missions.first?.requiredContributions, 2)
        XCTAssertEqual(selected.daily?.kind, "fixed")
        XCTAssertEqual(selected.daily?.missionId, "SAMPLE-one")
        XCTAssertNil(selected.levels)
        XCTAssertNil(selected.badges)
    }

    func testProgressRetainsAuthoritativeCountsAndNoEconomicValue() async throws {
        let transport = ActivityTransport(result: #"{"status":\#(progress),"disclosure":"SAMPLE core disclosure"}"#)
        let result = try await LiveDaemonClient(transport: transport).activityMissionsStatus()
        XCTAssertEqual(transport.method, "activity_missions_status")
        XCTAssertEqual(transport.params, "{}")
        XCTAssertEqual(result.disclosure, "SAMPLE core disclosure")
        XCTAssertEqual(result.status.source, "account_contributed_submissions")
        XCTAssertEqual(result.status.monthlyContributions, 3)
        XCTAssertEqual(result.status.daily?.contributions, 1)
        XCTAssertEqual(result.status.daily?.complete, false)
        XCTAssertEqual(result.status.currentStreak, 1)
        XCTAssertNil(result.status.level)
        XCTAssertNil(result.status.badges)
        XCTAssertFalse(result.status.rewardsEnabled)
        XCTAssertNil(result.status.creditPointsPending)
    }

    func testDisabledRulesStayNullRatherThanZeroProgress() throws {
        var object = try json(progress)
        object["daily"] = NSNull()
        object["completed_days"] = NSNull()
        object["current_streak"] = NSNull()
        let value = try decodeStatus(serialized(object))
        XCTAssertNil(value.status.daily)
        XCTAssertNil(value.status.completedDays)
        XCTAssertNil(value.status.currentStreak)
        XCTAssertFalse(value.status.levelsConfigured)
        XCTAssertNil(value.status.level)
        XCTAssertNil(value.status.badges)
    }

    func testRewardActivationAndNumericCreditAreRefusedInBothReplies() throws {
        for original in [unconfigured, progress] {
            for (key, unsafe) in [("rewards_enabled", true as Any), ("credit_points_pending", 0 as Any), ("credit_points_pending", 25 as Any)] {
                var object = try json(original)
                object[key] = unsafe
                let raw = try serialized(object)
                if original == unconfigured {
                    XCTAssertThrowsError(try decodeCatalogue(raw))
                } else {
                    XCTAssertThrowsError(try decodeStatus(raw))
                }
            }
        }
    }

    func testProfileRewardsAndUnsupportedSourcesAreRefused() throws {
        for (key, unsafe) in [("kind", "skill_evaluation" as Any), ("schema_version", 2 as Any), ("activity_profile", [:] as Any), ("credit_condition", "paid" as Any)] {
            var object = try json(unconfigured)
            object[key] = unsafe
            XCTAssertThrowsError(try decodeCatalogue(serialized(object)))
        }
        var status = try json(progress)
        status["source"] = "local_session_claims"
        XCTAssertThrowsError(try decodeStatus(serialized(status)))
        var invalidPolicy = try json(policy)
        invalidPolicy["rewards_enabled"] = true
        var catalogue = try json(unconfigured)
        catalogue["state"] = "configured"
        catalogue["policy_sha256"] = String(repeating: "a", count: 64)
        catalogue["policy"] = invalidPolicy
        XCTAssertThrowsError(try decodeCatalogue(serialized(catalogue)))
    }

    func testSampleStatesNeverInventConfiguredPolicyOrProgress() async throws {
        for set in SampleDaemonClient.SampleSet.allCases {
            let client = SampleDaemonClient(set)
            if set == .coreDown || set == .unknownCounts {
                do {
                    _ = try await client.activityMissionsCatalogue()
                    XCTFail("unreadable catalogue became unconfigured")
                } catch {
                    XCTAssertEqual(error as? DaemonDataError, set == .coreDown ? .unreachable : .daemon(code: "unavailable", message: "activity-missions-unavailable"))
                }
            } else {
                let result = try await client.activityMissionsCatalogue()
                XCTAssertEqual(result.catalogue.state, "unconfigured")
                XCTAssertNil(result.catalogue.policy)
                XCTAssertNil(result.catalogue.creditPointsPending)
                XCTAssertFalse(result.catalogue.rewardsEnabled)
                let raw = try XCTUnwrap(client.json(for: "activity_missions_catalogue"))
                XCTAssertEqual(try json(raw)["_sample"] as? String, "hand-written")
            }
            do {
                _ = try await client.activityMissionsStatus()
                XCTFail("no official policy yet; no sample progress is asserted")
            } catch {
                XCTAssertEqual(error as? DaemonDataError, set == .coreDown ? .unreachable : .daemon(code: "unavailable", message: "activity-missions-unavailable"))
            }
        }
    }

    func testLiveRefusalAndMalformedRewardsNeverBecomeEmptySuccess() async {
        let unavailable = ActivityTransport(error: #"{"code":"unavailable","message":"account-session-required"}"#)
        do {
            _ = try await LiveDaemonClient(transport: unavailable).activityMissionsStatus()
            XCTFail("signed-out status became zero progress")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .daemon(code: "unavailable", message: "account-session-required"))
        }
        let unsafe = unconfigured.replacingOccurrences(of: #""rewards_enabled":false"#, with: #""rewards_enabled":true"#)
        do {
            _ = try await LiveDaemonClient(transport: ActivityTransport(result: #"{"catalogue":\#(unsafe),"disclosure":"SAMPLE"}"#)).activityMissionsCatalogue()
            XCTFail("unsafe rewards answered live")
        } catch {
            XCTAssertEqual(error as? DaemonDataError, .undecodable(method: "activity_missions_catalogue"))
        }
    }

    func testRotationPolicyAndNullableBadgeStateRetainTheirWireMeaning() throws {
        var configuredPolicy = try json(policy)
        configuredPolicy["daily"] = ["kind": "rotation", "mission_ids": ["SAMPLE-one"]]
        configuredPolicy["levels"] = [["id": "SAMPLE-level", "required": 2]]
        configuredPolicy["badges"] = [["id": "SAMPLE-badge", "metric": "current_streak", "required": 2]]
        var catalogue = try json(unconfigured)
        catalogue["state"] = "configured"
        catalogue["policy_sha256"] = String(repeating: "a", count: 64)
        catalogue["policy"] = configuredPolicy
        let result = try decodeCatalogue(serialized(catalogue))
        XCTAssertEqual(result.catalogue.policy?.daily?.missionIds, ["SAMPLE-one"])
        XCTAssertNil(result.catalogue.policy?.daily?.missionId)
        XCTAssertEqual(result.catalogue.policy?.levels?.first?.required, 2)
        XCTAssertEqual(result.catalogue.policy?.badges?.first?.metric, "current_streak")
        var status = try json(progress)
        status["badges"] = [["id": "SAMPLE-badge", "achieved": NSNull()]]
        XCTAssertNil(try decodeStatus(serialized(status)).status.badges?.first?.achieved)
        configuredPolicy["daily"] = ["kind": "fixed", "mission_id": "SAMPLE-one", "mission_ids": ["SAMPLE-one"]]
        catalogue["policy"] = configuredPolicy
        XCTAssertThrowsError(try decodeCatalogue(serialized(catalogue)))
    }

    func testProgressKeepsExactUInt64CountsAndRefusesNegativeValues() async throws {
        let maximum = progress.replacingOccurrences(of: #""monthly_contributions":3"#, with: #""monthly_contributions":18446744073709551615"#)
        XCTAssertEqual(try decodeStatus(maximum).status.monthlyContributions, UInt64.max)
        let live = try await LiveDaemonClient(transport: ActivityTransport(result: #"{"status":\#(maximum),"disclosure":"SAMPLE"}"#)).activityMissionsStatus()
        XCTAssertEqual(live.status.monthlyContributions, UInt64.max)
        let negative = progress.replacingOccurrences(of: #""monthly_contributions":3"#, with: #""monthly_contributions":-1"#)
        XCTAssertThrowsError(try decodeStatus(negative))
    }

    /// The daemon returns the catalogue with each mission's `predicate` as
    /// published, so the shell's key set mirrors the protocol: a version-1
    /// block is checked against its own key set, a later version is carried
    /// unread, and anything without a positive integer version is refused.
    func testMissionPredicatesAreReadAsTheProtocolReadsThem() throws {
        func catalogue(predicate: Any) throws -> String {
            var configuredPolicy = try json(policy)
            var missions = try XCTUnwrap(configuredPolicy["missions"] as? [[String: Any]])
            missions[0]["predicate"] = predicate
            configuredPolicy["missions"] = missions
            var catalogue = try json(unconfigured)
            catalogue["state"] = "configured"
            catalogue["policy_sha256"] = String(repeating: "a", count: 64)
            catalogue["policy"] = configuredPolicy
            return try serialized(catalogue)
        }
        let v1: [String: Any] = ["version": 1, "tools": ["claude-code"], "tool_families": [String](), "languages": ["rust"], "min_sessions": 2]
        let read = try decodeCatalogue(catalogue(predicate: v1))
        XCTAssertEqual(read.catalogue.policy?.missions.first?.requiredContributions, 2)
        XCTAssertNoThrow(try decodeCatalogue(catalogue(predicate: ["version": 1, "languages": ["rust"], "min_sessions": 1])))
        XCTAssertNoThrow(try decodeCatalogue(catalogue(predicate: NSNull())))
        XCTAssertNoThrow(try decodeCatalogue(catalogue(predicate: ["version": 2, "repo_size": "large", "weights": ["a": [1, 2]]])))

        var unknownField = v1
        unknownField["repo_size"] = "large"
        var listOfNumbers = v1
        listOfNumbers["tools"] = [1]
        var noMinimum = v1
        noMinimum.removeValue(forKey: "min_sessions")
        for refused: Any in [
            unknownField, listOfNumbers, noMinimum,
            ["tools": ["claude-code"], "min_sessions": 1],
            ["version": 0, "min_sessions": 1],
            ["version": 1.5, "min_sessions": 1],
            ["version": "1", "min_sessions": 1],
            ["claude-code"],
            "claude-code",
        ] {
            XCTAssertThrowsError(try decodeCatalogue(catalogue(predicate: refused)), "\(refused)")
        }
    }

    private func decodeCatalogue(_ raw: String) throws -> DaemonData.ActivityMissionsCatalogue {
        try DaemonDataDecoding.decoder().decode(DaemonData.ActivityMissionsCatalogue.self, from: Data(#"{"catalogue":\#(raw),"disclosure":"SAMPLE"}"#.utf8))
    }
    private func decodeStatus(_ raw: String) throws -> DaemonData.ActivityMissionsStatus {
        try DaemonDataDecoding.decoder().decode(DaemonData.ActivityMissionsStatus.self, from: Data(#"{"status":\#(raw),"disclosure":"SAMPLE"}"#.utf8))
    }
    private func json(_ raw: String) throws -> [String: Any] {
        try XCTUnwrap(JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any])
    }
    private func serialized(_ object: [String: Any]) throws -> String {
        String(decoding: try JSONSerialization.data(withJSONObject: object), as: UTF8.self)
    }
}

private final class ActivityTransport: DaemonTransport, @unchecked Sendable {
    let response: String
    private(set) var method: String?
    private(set) var params: String?
    init(result: String) { response = #"{"id":1,"result":\#(result)}"# }
    init(error: String) { response = #"{"id":1,"error":\#(error)}"# }
    func call(_ method: String, params paramsJSON: String) -> String {
        self.method = method
        params = paramsJSON
        return response
    }
}
