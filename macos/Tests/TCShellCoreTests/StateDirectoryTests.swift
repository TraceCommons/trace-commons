import XCTest

@testable import TCShellCore

/// The defect these pin: the shipped app resolved its state directory from
/// `TRACE_COMMONS_CONTRIBUTOR_DIR` and nothing else, so a Finder launch --
/// which carries no shell environment -- always refused, and the notarized
/// DMG could not be made to work by anyone. Rust and C# both had the full
/// precedence already; Swift implemented one third of it.
final class StateDirectoryTests: XCTestCase {
    private let home = "/Users/someone"

    func testFallsBackToThePerUserDirectoryWhenNothingIsSet() throws {
        let resolved = try StateDirectory.resolve(
            explicit: nil,
            environment: [:],
            homeDirectory: home,
            probe: .init { _ in .absent }
        )
        XCTAssertEqual(
            resolved.path,
            "/Users/someone/Library/Application Support/trace-commons",
            "a Finder launch has no environment and must still resolve"
        )
    }

    func testTheEnvironmentVariableWinsOverTheDefault() throws {
        let resolved = try StateDirectory.resolve(
            explicit: nil,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/from-env"],
            homeDirectory: home,
            probe: .init { _ in .directory }
        )
        XCTAssertEqual(resolved.path, "/tmp/from-env")
    }

    func testAnExplicitDirectoryWinsOverTheEnvironment() throws {
        let resolved = try StateDirectory.resolve(
            explicit: "/tmp/explicit",
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": "/tmp/from-env"],
            homeDirectory: home,
            probe: .init { _ in .directory }
        )
        XCTAssertEqual(
            resolved.path, "/tmp/explicit",
            "precedence is explicit, then environment, then the per-user default -- "
                + "the same order config.rs and DaemonHost.cs already use"
        )
    }

    func testAnEmptyEnvironmentVariableIsTreatedAsUnset() throws {
        let resolved = try StateDirectory.resolve(
            explicit: nil,
            environment: ["TRACE_COMMONS_CONTRIBUTOR_DIR": ""],
            homeDirectory: home,
            probe: .init { _ in .absent }
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
            explicit: nil,
            environment: [:],
            homeDirectory: home,
            probe: .init { _ in .absent }
        )
        XCTAssertEqual(
            resolved.path, "/Users/someone/Library/Application Support/trace-commons")
    }

    func testAPathThatExistsButIsAFileIsRefused() {
        XCTAssertThrowsError(
            try StateDirectory.resolve(
                explicit: "/tmp/a-file",
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .file }
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
                explicit: long,
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .directory }
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
                explicit: long,
                environment: [:],
                homeDirectory: home,
                probe: .init { _ in .directory }
            )
        ) { error in
            guard case StateDirectory.Refusal.pathTooLong = error else {
                return XCTFail("expected pathTooLong, got \(error)")
            }
        }
    }
}
