//! Finding the daemon.
//
// Getting any of this wrong costs a "not running" message, which is a state the
// app handles anyway — so the tests are about not being *confidently* wrong: a
// `port` under some other table must not be mistaken for the server's.

import XCTest

@testable import IronWireKit

final class DiscoveryTests: XCTestCase {
    func test_the_default_port_is_the_one_the_daemon_binds() {
        // `ironwire_core::DEFAULT_PORT`.
        XCTAssertEqual(Discovery.defaultPort, 8463)
    }

    func test_a_configured_port_is_read_out_of_the_server_table() {
        let toml = """
        [server]
        port = 9999
        upstream_timeout_secs = 900
        """
        XCTAssertEqual(Discovery.port(inTOML: toml), 9999)
    }

    /// The only mistake a naive search would actually make. Several other tables
    /// can carry a `port`, and following one of them would send every request to
    /// a daemon that is not there.
    func test_a_port_belonging_to_another_table_is_not_mistaken_for_the_servers() {
        let toml = """
        [[backends]]
        id = "local"
        port = 11434

        [capture]
        port = 1234
        """
        XCTAssertEqual(Discovery.port(inTOML: toml), Discovery.defaultPort)
    }

    func test_a_config_without_a_server_table_falls_back_to_the_default() {
        XCTAssertEqual(Discovery.port(inTOML: "[capture]\nenabled = true\n"), Discovery.defaultPort)
        XCTAssertEqual(Discovery.port(inTOML: ""), Discovery.defaultPort)
    }

    func test_a_port_that_is_not_a_number_is_ignored_rather_than_half_read() {
        XCTAssertEqual(Discovery.port(inTOML: "[server]\nport = \"nine\"\n"), Discovery.defaultPort)
        XCTAssertEqual(Discovery.port(inTOML: "[server]\nport =\n"), Discovery.defaultPort)
    }

    func test_a_port_outside_the_range_a_port_can_be_is_ignored() {
        XCTAssertEqual(Discovery.port(inTOML: "[server]\nport = 70000\n"), Discovery.defaultPort)
        XCTAssertEqual(Discovery.port(inTOML: "[server]\nport = 0\n"), Discovery.defaultPort)
    }

    func test_comments_and_spacing_do_not_hide_the_port() {
        let toml = """
        # IronWire configuration
        [server]   # where it listens
           port   =   8500    # not the default
        """
        XCTAssertEqual(Discovery.port(inTOML: toml), 8500)
    }

    func test_a_commented_out_port_is_not_read() {
        XCTAssertEqual(Discovery.port(inTOML: "[server]\n# port = 9999\n"), Discovery.defaultPort)
    }

    func test_ironwire_home_overrides_the_default_location() {
        let home = Discovery.home(environment: ["IRONWIRE_HOME": "/tmp/iw-test"])
        XCTAssertEqual(home.path, "/tmp/iw-test")
    }

    func test_an_empty_ironwire_home_is_treated_as_unset() {
        let home = Discovery.home(environment: ["IRONWIRE_HOME": ""])
        XCTAssertEqual(home.lastPathComponent, ".ironwire")
    }

    func test_the_default_home_is_the_one_the_daemon_uses() {
        let home = Discovery.home(environment: [:])
        XCTAssertEqual(home.lastPathComponent, ".ironwire")
        XCTAssertEqual(home.deletingLastPathComponent().path,
                       FileManager.default.homeDirectoryForCurrentUser.path)
    }

    // MARK: - The control URL

    func test_the_control_url_is_loopback_on_the_daemons_port() {
        XCTAssertEqual(
            Discovery.controlURL(port: 8500).absoluteString, "http://127.0.0.1:8500/_ironwire")
        XCTAssertEqual(
            Discovery.controlURL(port: 8463, path: "/status").absoluteString,
            "http://127.0.0.1:8463/_ironwire/status")
    }

    /// A path is appended under `/_ironwire` rather than replacing it, so the
    /// URL a user is handed and the one the client calls cannot drift apart.
    func test_a_path_is_appended_under_the_control_prefix() {
        for path in ["/status", "/events", "/pin"] {
            XCTAssertTrue(
                Discovery.controlURL(port: 8463, path: path).path.hasPrefix("/_ironwire/"),
                "\(path) escaped the control prefix")
        }
    }

    // MARK: - Token

    func test_a_missing_token_file_is_an_ordinary_absence_and_not_an_error() throws {
        let dir = try temporaryDirectory()
        XCTAssertNil(Discovery.token(home: dir))
    }

    func test_a_token_is_read_without_its_trailing_newline() throws {
        let dir = try temporaryDirectory()
        try "deadbeef\n".write(to: dir.appendingPathComponent("control.token"), atomically: true, encoding: .utf8)
        XCTAssertEqual(Discovery.token(home: dir), "deadbeef")
    }

    /// An empty token would authorise nothing, and sending it would produce a
    /// 401 the app then blames on the user. Absent is the honest reading.
    func test_an_empty_token_file_reads_as_no_token() throws {
        let dir = try temporaryDirectory()
        try "   \n".write(to: dir.appendingPathComponent("control.token"), atomically: true, encoding: .utf8)
        XCTAssertNil(Discovery.token(home: dir))
    }

    /// This app never writes into `$IRONWIRE_HOME`. Minting a control token
    /// would be creating the credential for a daemon that may not exist, in a
    /// directory it does not own — the CLI does that because it is the thing
    /// setting the daemon up.
    func test_reading_a_missing_token_does_not_create_one() throws {
        let dir = try temporaryDirectory()
        _ = Discovery.token(home: dir)
        XCTAssertFalse(FileManager.default.fileExists(atPath: dir.appendingPathComponent("control.token").path))
    }

    private func temporaryDirectory() throws -> URL {
        let dir = FileManager.default.temporaryDirectory
            .appendingPathComponent("ironwire-tests-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        addTeardownBlock { try? FileManager.default.removeItem(at: dir) }
        return dir
    }
}
