import XCTest

@testable import TCShellCore

/// The defect these pin: the shipped app resolved its state directory from
/// `TRACE_COMMONS_CONTRIBUTOR_DIR` and nothing else, so a Finder launch --
/// which carries no shell environment -- always refused, and the notarized
/// DMG could not be made to work by anyone. Rust and C# both had the full
/// precedence already; Swift implemented one third of it.
final class StateDirectoryTests: XCTestCase {
    private let home = "/Users/someone"

    /// An override check that allows everything, for the tests about
    /// precedence and the filesystem.
    private let allowed = StateDirectory.OverrideCheck { _ in nil }

    /// Records every directory it is asked about and refuses with `label`.
    private final class RecordingCheck: @unchecked Sendable {
        var asked: [String] = []
        let label: String?
        static let sentence = "The core's sentence."
        init(refusing label: String?) { self.label = label }
        var check: StateDirectory.OverrideCheck {
            StateDirectory.OverrideCheck { dir in
                self.asked.append(dir)
                return self.label.map { .init(label: $0, sentence: Self.sentence) }
            }
        }
    }

    func testFallsBackToThePerUserDirectoryWhenNothingIsSet() throws {
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.production,
            explicit: nil,
            environment: [:],
            homeDirectory: home,
            probe: .init { _ in .absent },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            resolved.path,
            "/Users/someone/Library/Application Support/trace-commons",
            "a Finder launch has no environment and must still resolve"
        )
    }

    func testTheEnvironmentVariableWinsOverTheDefault() throws {
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.production,
            explicit: nil,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/from-env"],
            homeDirectory: home,
            probe: .init { _ in .directory },
            overrideCheck: allowed
        )
        XCTAssertEqual(resolved.path, "/tmp/from-env")
    }

    func testAnExplicitDirectoryWinsOverTheEnvironment() throws {
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.production,
            explicit: "/tmp/explicit",
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/from-env"],
            homeDirectory: home,
            probe: .init { _ in .directory },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            resolved.path, "/tmp/explicit",
            "precedence is explicit, then environment, then the per-user default -- "
                + "the same order config.rs and DaemonHost.cs already use"
        )
    }

    func testAnEmptyEnvironmentVariableIsTreatedAsUnset() throws {
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.production,
            explicit: nil,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": ""],
            homeDirectory: home,
            probe: .init { _ in .absent },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            resolved.path,
            "/Users/someone/Library/Application Support/trace-commons",
            "an empty string is not a directory anyone chose"
        )
    }

    func testAnAbsentDirectoryIsNotARefusal() throws {
        // ConfigStore::open creates the directory (0700 on unix). Refusing
        // here because it does not exist yet would put the fresh-install
        // case straight back into the dead end this slice removes.
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.production,
            explicit: nil,
            environment: [:],
            homeDirectory: home,
            probe: .init { _ in .absent },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            resolved.path, "/Users/someone/Library/Application Support/trace-commons")
    }

    func testAPathThatExistsButIsAFileIsRefused() {
        XCTAssertThrowsError(
            try StateDirectory.resolve(
                channel: BuildChannelFixture.production,
                explicit: "/tmp/a-file",
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .file },
                overrideCheck: allowed
            )
        ) { error in
            guard case StateDirectory.Refusal.notADirectory = error else {
                return XCTFail("expected notADirectory, got \(error)")
            }
        }
    }

    func testAPathTooLongForTheControlSocketIsRefused() {
        // The daemon's socket is <dir>/daemon.sock and cannot exceed 104
        // bytes; catching it here beats an opaque start failure.
        let long = "/tmp/" + String(repeating: "x", count: 120)
        XCTAssertThrowsError(
            try StateDirectory.resolve(
                channel: BuildChannelFixture.production,
                explicit: long,
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .directory },
                overrideCheck: allowed
            )
        ) { error in
            guard case StateDirectory.Refusal.pathTooLong(let bytes) = error else {
                return XCTFail("expected pathTooLong, got \(error)")
            }
            XCTAssertEqual(bytes, (long + "/daemon.sock").utf8.count)
        }
    }

    func testTheSocketBudgetIsMeasuredInBytesNotCharacters() {
        // A path of multi-byte characters can pass a character count and
        // still blow the sockaddr_un budget.
        let long = "/tmp/" + String(repeating: "é", count: 60)
        XCTAssertThrowsError(
            try StateDirectory.resolve(
                channel: BuildChannelFixture.production,
                explicit: long,
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .directory },
                overrideCheck: allowed
            )
        ) { error in
            guard case StateDirectory.Refusal.pathTooLong = error else {
                return XCTFail("expected pathTooLong, got \(error)")
            }
        }
    }

    func testDefaultDirectoryComesFromTheDylib() throws {
        // A staging build names its own directory and reads only its own
        // override: the production variable is ignored, so it can never
        // point staging at production's enrollment (spec 4.2).
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging,
            explicit: nil,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/production-dir"],
            homeDirectory: home,
            probe: .init { _ in .absent },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            resolved.path, "/Users/someone/Library/Application Support/trace-commons-staging")

        let overridden = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging,
            explicit: nil,
            environment: [
                "TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/production-dir",
                "TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR": "/tmp/staging-dir",
            ],
            homeDirectory: home,
            probe: .init { _ in .directory },
            overrideCheck: allowed
        )
        XCTAssertEqual(overridden.path, "/tmp/staging-dir")

        let compute = try StateDirectory.resolveCompute(
            channel: BuildChannelFixture.staging,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/production-dir"],
            homeDirectory: home,
            probe: .init { _ in .absent },
            overrideCheck: allowed
        )
        XCTAssertEqual(
            compute.path, "/Users/someone/Library/Application Support/trace-commons-staging")
    }

    // The other-channel refusal (staging design 4.4) is the dylib's
    // (`tc_resolve_state_dir`); TCShellCore never links it, so these pin
    // when it is asked and what its answer does. The dylib's own answer is
    // checked in TCBridgeTests (`StateDirectoryBridgeTests`).

    func testAnOverrideNamingTheOtherChannelDirectoryIsRefused() {
        let check = RecordingCheck(refusing: "state-dir-other-channel")
        let other = "/Users/someone/Library/Application Support/trace-commons"
        for resolve in [StateDirectory.resolve, StateDirectory.resolveCompute] {
            XCTAssertThrowsError(
                try resolve(
                    BuildChannelFixture.staging, nil,
                    ["TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR": other],
                    home, .init { _ in .absent }, check.check)
            ) { error in
                guard case StateDirectory.Refusal.otherChannel = error else {
                    return XCTFail("expected otherChannel, got \(error)")
                }
                XCTAssertEqual(
                    "\(error)", RecordingCheck.sentence, "the startup refusal shows the core's sentence")
            }
        }
        XCTAssertEqual(check.asked, [other, other])
    }

    func testOnlyAnOverrideIsSentToTheCheck() throws {
        let check = RecordingCheck(refusing: "state-dir-other-channel")
        // The platform default and an explicit directory are not checked:
        // the same rule as ConfigStore::resolve.
        let fallback = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging, explicit: nil, environment: [:],
            homeDirectory: home, probe: .init { _ in .absent }, overrideCheck: check.check)
        XCTAssertEqual(
            fallback.path, "/Users/someone/Library/Application Support/trace-commons-staging")
        let explicit = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging, explicit: "/tmp/explicit",
            environment: ["TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR": "/tmp/from-env"],
            homeDirectory: home, probe: .init { _ in .directory }, overrideCheck: check.check)
        XCTAssertEqual(explicit.path, "/tmp/explicit")
        // An empty variable is unset, so there is nothing to check.
        _ = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging, explicit: nil,
            environment: ["TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR": ""],
            homeDirectory: home, probe: .init { _ in .absent }, overrideCheck: check.check)
        XCTAssertEqual(check.asked, [])
    }

    func testAnAllowedOverrideIsUsedAsGiven() throws {
        let check = RecordingCheck(refusing: nil)
        let resolved = try StateDirectory.resolve(
            channel: BuildChannelFixture.staging, explicit: nil,
            environment: ["TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR": "/tmp/staging-dir"],
            homeDirectory: home, probe: .init { _ in .directory }, overrideCheck: check.check)
        XCTAssertEqual(resolved.path, "/tmp/staging-dir")
        XCTAssertEqual(check.asked, ["/tmp/staging-dir"])
    }

    func testAnyOtherCheckFailureRefusesRatherThanFallingBack() {
        // `state-dir-unresolved`, `panic`, or a label this shell does not
        // know: the override could not be checked, so it is not used.
        for label in ["state-dir-unresolved", "panic", "something-new"] {
            let check = RecordingCheck(refusing: label)
            XCTAssertThrowsError(
                try StateDirectory.resolve(
                    channel: BuildChannelFixture.production, explicit: nil,
                    environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/from-env"],
                    homeDirectory: home, probe: .init { _ in .directory },
                    overrideCheck: check.check)
            ) { error in
                guard case StateDirectory.Refusal.overrideUnchecked = error else {
                    return XCTFail("expected overrideUnchecked for \(label), got \(error)")
                }
                XCTAssertEqual("\(error)", RecordingCheck.sentence)
            }
        }
    }
}
