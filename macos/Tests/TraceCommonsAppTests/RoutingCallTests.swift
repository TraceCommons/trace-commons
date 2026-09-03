import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// What the routing card actually puts on the socket, and what it reads back.
///
/// `RoutingSurfaceTests` covers the mapping and `RoutingSurfaceExportTests`
/// covers the words. This is the third thing that can be wrong: the method
/// name, the parameter bytes, and the shapes `get_settings` and `status`
/// answer with. `set_settings` fails the whole object on one unrecognised
/// key, so a drift here is a silent no-write rather than an error anybody
/// would see.
private final class RecordingDaemon: DaemonCalling {
    private(set) var calls: [(method: String, params: String)] = []
    /// Answers by method, so one client can be driven through a write and
    /// both probes in the order the card issues them.
    var responses: [String: String] = [:]

    func call(_ method: String, params paramsJSON: String) -> String {
        calls.append((method: method, params: paramsJSON))
        return responses[method] ?? #"{"id":1,"result":{}}"#
    }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    func params(of method: String) -> [String: Any]? {
        guard let call = calls.last(where: { $0.method == method }),
              let data = call.params.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }
}


/// The Rust-side calls as the app wires them. Spelled here rather than taken
/// from `AppModel` so these assertions do not need a live model.
private let routingCalls = RoutingCalls(
    tokenLine: { TCRoutingCopy.tokenLine(path: $0) },
    unreachableLine: { TCRoutingCopy.unreachableLine(port: $0) },
    discoveryLine: { TCRoutingCopy.discoveryLine(port: $0) },
    toolWord: { TCRoutingCopy.toolWord(sourceMode: $0, wiring: $1) },
    toolTone: { TCRoutingCopy.toolTone(sourceMode: $0, wiring: $1) },
    stateLine: { TCRoutingCopy.stateLine(state: $0) },
    stateTone: { TCRoutingCopy.stateTone(state: $0) }
)

final class RoutingCallTests: XCTestCase {
    private var daemon = RecordingDaemon()
    private var client: DaemonClient!

    private static let settingsFrame = """
    {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,
    "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
    "max_uploads_per_day":100,"near_ai_configured":false,
    "claude_root_configured":true,"codex_root_configured":true}}
    """

    override func setUp() {
        super.setUp()
        daemon = RecordingDaemon()
        daemon.responses["set_settings"] = Self.settingsFrame
        client = DaemonClient(daemon: daemon)
    }

    // MARK: - The declaration

    func testTurningItOnWritesTheDeclaredModeAndPort() throws {
        _ = try client.setIronWire(RoutingForm(on: true, port: 8463, tokenDir: ""))

        XCTAssertEqual(daemon.calls.map(\.method), ["set_settings"])
        let params = try XCTUnwrap(daemon.params(of: "set_settings"))
        XCTAssertEqual(Array(params.keys), ["ironwire"])
        let declaration = try XCTUnwrap(params["ironwire"] as? [String: Any])
        XCTAssertEqual(declaration["mode"] as? String, "watch")
        XCTAssertEqual(declaration["port"] as? Int, 8463)
        XCTAssertNil(declaration["token_dir"])
    }

    /// Off is spelled `null` on the wire, not omitted. A key that is not
    /// there is not a change, and this is the one call that turns the
    /// reading of a local service off.
    func testTurningItOffWritesNullAndNotAnAbsentKey() throws {
        _ = try client.setIronWire(RoutingForm(on: false, port: 8463, tokenDir: "/Users/x/ironwire"))

        let raw = try XCTUnwrap(daemon.calls.last?.params)
        XCTAssertTrue(raw.contains("null"), raw)
        let params = try XCTUnwrap(daemon.params(of: "set_settings"))
        XCTAssertTrue(params["ironwire"] is NSNull)
    }

    // MARK: - The probes

    func testTheProbesAskAboutTheDeclaredPortAndFolder() throws {
        daemon.responses["probe_routing"] = #"{"id":1,"result":{"outcome":"reachable"}}"#
        daemon.responses["probe_routed_tools"] =
            #"{"id":1,"result":{"outcome":"reachable","tools":[{"id":"claude","installed":true,"wired":true}]}}"#

        let form = RoutingForm(on: true, port: 9001, tokenDir: "/Users/x/ironwire")
        XCTAssertEqual(try client.probeRouting(form), .reachable)
        let evidence = try client.probeRoutedTools(form)
        XCTAssertEqual(evidence.wiring(forToolID: "claude"), .wired)

        XCTAssertEqual(daemon.calls.map(\.method), ["probe_routing", "probe_routed_tools"])
        for method in ["probe_routing", "probe_routed_tools"] {
            let params = try XCTUnwrap(daemon.params(of: method))
            XCTAssertEqual(params["port"] as? Int, 9001, method)
            XCTAssertEqual(params["token_dir"] as? String, "/Users/x/ironwire", method)
        }
    }

    /// An empty folder box is not sent at all: the daemon refuses an empty
    /// string with `token-dir-invalid`, and absence is what falls back to the
    /// conventional location.
    func testAnEmptyFolderIsNotSentToTheProbe() throws {
        daemon.responses["probe_routing"] = #"{"id":1,"result":{"outcome":"reachable"}}"#
        _ = try client.probeRouting(RoutingForm(on: true, port: 8463, tokenDir: "  "))
        XCTAssertEqual(try XCTUnwrap(daemon.params(of: "probe_routing")).keys.sorted(), ["port"])
    }

    /// The macOS failure. The daemon reports the absolute path it actually
    /// read, and it survives into the outcome the card renders.
    func testAnUnreadableTokenCarriesTheDaemonsPath() throws {
        daemon.responses["probe_routing"] = """
        {"id":1,"result":{"outcome":"token_unreadable",
        "token_path":"/Users/someone/.ironwire/control.token"}}
        """
        XCTAssertEqual(
            try client.probeRouting(RoutingForm(on: true, port: 8463, tokenDir: "")),
            .tokenUnusable(path: "/Users/someone/.ironwire/control.token")
        )
    }

    /// An unreachable proxy is a well-formed answer, not a failed call: the
    /// call must not throw, because a throw would render as an error label
    /// rather than as the sentence naming the port.
    func testAnUnreachableProxyIsAnAnswerAndNotAThrow() throws {
        daemon.responses["probe_routing"] = #"{"id":1,"result":{"outcome":"unreachable","port":8463}}"#
        XCTAssertEqual(
            try client.probeRouting(RoutingForm(on: true, port: 8463, tokenDir: "")),
            .unreachable(port: 8463)
        )
    }

    /// A refusal from the daemon is a call that did not run, and throws --
    /// which the card degrades to the claims-nothing line rather than to a
    /// verdict.
    func testARefusedProbeThrowsWithItsLabel() {
        daemon.responses["probe_routing"] =
            #"{"id":1,"error":{"code":"bad_params","message":"port-invalid"}}"#
        XCTAssertThrowsError(
            try client.probeRouting(RoutingForm(on: true, port: 8463, tokenDir: ""))
        ) { error in
            XCTAssertEqual((error as? DaemonClient.Failure)?.message, "port-invalid")
        }
    }

    // MARK: - What comes back on the settings and status shapes

    /// The declaration the daemon holds fills the card's fields. Without
    /// this the card would show the conventional port over a declaration
    /// that named a different one.
    func testTheHeldDeclarationIsDecodedFromGetSettings() throws {
        daemon.responses["get_settings"] = """
        {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true,
        "claude_source_mode":"watch","codex_source_mode":"off","gemini_source_mode":"unset",
        "cline_source_mode":"off",
        "ironwire":{"mode":"watch","port":9001,"token_dir":"/Users/x/ironwire"}}}
        """
        let view = try client.settings()
        XCTAssertEqual(view.ironwire?.mode, "watch")
        XCTAssertEqual(view.ironwire?.port, 9001)
        XCTAssertEqual(view.ironwire?.tokenDir, "/Users/x/ironwire")
        XCTAssertEqual(view.routingSourceModes.claude, "watch")
        XCTAssertEqual(view.routingSourceModes.codex, "off")
        XCTAssertEqual(view.routingSourceModes.gemini, "unset")
        XCTAssertEqual(view.routingSourceModes.cline, "off")
    }

    /// A daemon that declared nothing answers no `ironwire` at all, and a
    /// daemon that predates the source modes answers none of those. Silence
    /// about a source is `unset` -- a tool in use -- never "not used".
    func testSilenceAboutTheDeclarationAndTheSourcesReadsAsOffAndUnset() throws {
        daemon.responses["get_settings"] = Self.settingsFrame
        let view = try client.settings()
        XCTAssertNil(view.ironwire)
        XCTAssertEqual(view.routingSourceModes, .unset)
        XCTAssertFalse(RoutingForm.fromDeclaration(
            mode: view.ironwire?.mode, port: view.ironwire?.port, tokenDir: view.ironwire?.tokenDir
        ).on)
    }

    /// `status` carries the daemon's own three-state view beside `health`,
    /// because none of the three is a fault.
    func testTheRoutingStateIsDecodedFromStatus() throws {
        daemon.responses["status"] = """
        {"id":1,"result":{"schema_version":"1.1","logged_in":true,"paused":false,
        "queue_depth":0,"consent_scopes":[],
        "routing":{"state":"awaiting_rows","last_refresh_at":"2026-09-02T10:00:00Z"}}}
        """
        let status = try client.status()
        XCTAssertEqual(status.routing.state, "awaiting_rows")
        XCTAssertNotNil(status.routing.lastRefreshAt)
    }

    /// A daemon that predates the field has declared no proxy, which is
    /// exactly what the fallback says -- and it must not read as an error
    /// state.
    func testAStatusWithoutRoutingReadsAsNothingDeclared() throws {
        daemon.responses["status"] = """
        {"id":1,"result":{"schema_version":"1.1","logged_in":true,"paused":false,
        "queue_depth":0,"consent_scopes":[]}}
        """
        let status = try client.status()
        XCTAssertEqual(status.routing, .notDeclared)
        XCTAssertNil(status.routing.lastRefreshAt)
        XCTAssertEqual(RoutingSurface.tone(forState: status.routing.state, calls: routingCalls), .neutral)
        XCTAssertFalse(RoutingSurface.showsLastChecked(forState: status.routing.state, calls: routingCalls))
    }
}
