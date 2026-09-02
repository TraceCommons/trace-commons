import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The daemon a `DaemonClient` talks to in these tests: it records what was
/// sent and answers with whatever the test decided, so the method name and
/// the parameter bytes are assertable without a live socket, a state
/// directory, or the FFI.
///
/// `openPreview` throws rather than answering: no test here opens one, and
/// a `TCPreview` cannot be built outside `TCBridge` anyway. It is on the
/// protocol only because `DaemonClient` offers it.
private final class RecordingDaemon: DaemonCalling {
    private(set) var calls: [(method: String, params: String)] = []
    /// What the next call answers with. Defaults to a well-formed frame
    /// carrying an empty result, which every call here either ignores or
    /// fails to decode -- never a silent success.
    var response = #"{"id":1,"result":{}}"#

    func call(_ method: String, params paramsJSON: String) -> String {
        calls.append((method: method, params: paramsJSON))
        return response
    }

    func openPreview(entryID: String) throws -> TCPreview {
        throw TCDaemon.TCError.daemonGone
    }

    /// The JSON object of the last call's parameters, or nil if nothing was
    /// sent. Decoded rather than string-matched: what the daemon reads is
    /// the parsed object, not the spelling of it.
    var lastParams: [String: Any]? {
        guard let last = calls.last,
              let data = last.params.data(using: .utf8),
              let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return nil }
        return object
    }
}

/// `get_settings` answers this shape and so does `set_settings` -- the
/// daemon returns `redacted_settings` from both (see `handle_request`'s
/// `"set_settings"` arm). A write that answered with anything else would
/// mean the client cannot report what actually landed.
private let settingsFrame = """
{"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,
"local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
"max_uploads_per_day":100,"near_ai_configured":false,
"claude_root_configured":true,"codex_root_configured":true}}
"""

/// Covers the macOS shell's only path for *changing* a daemon setting.
///
/// Everything else this client does to settings reads them. Without a write
/// path a declaration would only take effect at daemon start, and a shell
/// that answers "restart to apply" is a shell whose contributors conclude
/// the setting does not work. The daemon applies a changed declaration to
/// the running daemon itself (`shared.rebuild_routing`), so there is nothing
/// to restart and nothing here should say there is.
final class SetSettingsTests: XCTestCase {
    private var daemon = RecordingDaemon()
    private var client: DaemonClient!

    override func setUp() {
        super.setUp()
        daemon = RecordingDaemon()
        client = DaemonClient(daemon: daemon)
    }

    // MARK: - What goes out

    /// The method name is the contract's, and the object carries exactly
    /// the declared key -- no more. `set_settings` refuses an object holding
    /// a key it does not recognise, so an extra key added here in passing
    /// would not be ignored; it would fail the whole write.
    func testASettingsWriteSendsTheDeclaredKeyAndValue() throws {
        daemon.response = settingsFrame
        _ = try client.setSettings(["ironwire": ["mode": "watch", "port": 8463]])

        XCTAssertEqual(daemon.calls.count, 1)
        XCTAssertEqual(daemon.calls.first?.method, "set_settings")
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(Array(params.keys), ["ironwire"])
        let declaration = try XCTUnwrap(params["ironwire"] as? [String: Any])
        XCTAssertEqual(declaration["mode"] as? String, "watch")
        XCTAssertEqual(declaration["port"] as? Int, 8463)
    }

    /// Several knobs in one call is one write, not several: the daemon
    /// validates the whole object and saves once.
    func testSeveralDeclarationsRideInOneCall() throws {
        daemon.response = settingsFrame
        _ = try client.setSettings([
            "quiescence_secs": 45,
            "local_notifications": false,
        ])

        XCTAssertEqual(daemon.calls.count, 1)
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params.keys.sorted(), ["local_notifications", "quiescence_secs"])
        XCTAssertEqual(params["quiescence_secs"] as? Int, 45)
        XCTAssertEqual(params["local_notifications"] as? Bool, false)
    }

    /// `NSNull` is a declaration, not an absence. For `ironwire` it is the
    /// spelling of *off*, and for `claude_root` it clears an override --
    /// dropping it, the way a blank correction is dropped elsewhere in this
    /// client, would silently turn "off" into "unchanged".
    func testANullValueIsSentAsJSONNull() throws {
        daemon.response = settingsFrame
        _ = try client.setSettings(["ironwire": NSNull()])

        let raw = try XCTUnwrap(daemon.calls.last?.params)
        XCTAssertTrue(raw.contains("null"), "a null declaration must survive encoding: \(raw)")
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertTrue(params.keys.contains("ironwire"))
        XCTAssertTrue(params["ironwire"] is NSNull)
    }

    // MARK: - What comes back

    /// The answer is the daemon's updated view, not what the caller asked
    /// for. A client that echoed its own request would report a change the
    /// daemon may have refused.
    func testASettingsWriteAnswersWithTheDaemonsUpdatedView() throws {
        daemon.response = settingsFrame
        let view = try client.setSettings(["quiescence_secs": 45])
        XCTAssertEqual(view.quiescenceSecs, 45)
        XCTAssertEqual(view.maxUploadsPerDay, 100)
    }

    /// A refusal from the daemon surfaces as the same `Failure` every other
    /// call throws, carrying the contract's fixed label.
    func testADaemonRefusalIsThrownWithItsLabel() {
        daemon.response = #"{"id":1,"error":{"code":"bad_params","message":"settings-unknown-field"}}"#
        XCTAssertThrowsError(try client.setSettings(["nonsense": 1])) { error in
            let failure = error as? DaemonClient.Failure
            XCTAssertEqual(failure?.code, "bad_params")
            XCTAssertEqual(failure?.message, "settings-unknown-field")
        }
        XCTAssertEqual(daemon.calls.count, 1, "a refusal is the daemon's answer, so it was sent")
    }

    // MARK: - What never goes out

    /// An empty object is refused here rather than sent. The daemon refuses
    /// it too (`bad_params` / `no-known-setting-supplied`), so this is not a
    /// second opinion about validity -- it is a caller bug that must not
    /// reach the socket at all, and `rawResult` would have encoded it as the
    /// same `{}` an unrelated no-parameter call sends.
    func testAnEmptyDeclarationIsRefusedWithoutBeingSent() {
        XCTAssertThrowsError(try client.setSettings([:])) { error in
            XCTAssertEqual(error as? DaemonClient.SettingsRefusal, .nothingDeclared)
        }
        XCTAssertTrue(daemon.calls.isEmpty, "nothing may reach the daemon")
    }

    /// A key that is blank, or only whitespace, is not a settings key. The
    /// daemon would answer `settings-unknown-field`; this never gets that
    /// far.
    func testABlankKeyIsRefusedWithoutBeingSent() {
        for blank in ["", " ", "\n\t "] {
            XCTAssertThrowsError(try client.setSettings([blank: 1])) { error in
                XCTAssertEqual(error as? DaemonClient.SettingsRefusal, .blankKey)
            }
        }
        XCTAssertTrue(daemon.calls.isEmpty, "nothing may reach the daemon")
    }

    /// A value Foundation cannot encode is refused by key, before any
    /// encoding is attempted on the whole object. `JSONSerialization` throws
    /// an ObjC exception for this rather than a Swift error, so a client
    /// that handed it one would take the process down, not fail a call.
    func testAnUnencodableValueIsRefusedByKeyWithoutBeingSent() {
        XCTAssertThrowsError(try client.setSettings(["quiescence_secs": Date()])) { error in
            XCTAssertEqual(
                error as? DaemonClient.SettingsRefusal,
                .valueNotEncodable(key: "quiescence_secs")
            )
        }
        XCTAssertTrue(daemon.calls.isEmpty, "nothing may reach the daemon")
    }

    /// One bad key spoils the write: the good declarations beside it are not
    /// sent on their own. A partial write is a state neither the contributor
    /// nor the daemon asked for.
    func testOneBadValueRefusesTheWholeObject() {
        XCTAssertThrowsError(
            try client.setSettings(["quiescence_secs": 45, "ironwire": Date()])
        ) { error in
            XCTAssertEqual(
                error as? DaemonClient.SettingsRefusal,
                .valueNotEncodable(key: "ironwire")
            )
        }
        XCTAssertTrue(daemon.calls.isEmpty, "nothing may reach the daemon")
    }
}

/// The other half of adding a call: proving the calls that were already
/// there still send what they always sent.
///
/// Every method name below is driven through the real client against a
/// recording daemon, so this fails on a changed literal, a renamed method,
/// or a call that quietly started routing somewhere else -- not on a
/// second copy of the list agreeing with itself. Answers are ignored: the
/// subject here is what leaves, not what comes back.
final class DaemonClientMethodInventoryTests: XCTestCase {
    /// Every `set_settings`-era method this shell issues, in the spelling
    /// the daemon's `METHODS` list uses.
    private static let expected = [
        "acknowledge_near_ai_notice",
        "approve",
        "arming_suggestion",
        "cancel",
        "clear_public_profile",
        "consent_options",
        "decline_arming",
        "dismiss",
        "enroll",
        "get_public_profile",
        "get_settings",
        "history_rollup",
        "list_audit",
        "list_history",
        "list_pending",
        "list_projects",
        "pause",
        "preview",
        "preview_cancel",
        "preview_request",
        "preview_visible",
        "probe_routed_tools",
        "probe_routing",
        "queue_outcome_counts",
        "refresh_history",
        "resume",
        "set_consent_scopes",
        "set_project_mode",
        "set_public_profile",
        "set_settings",
        "status",
        "withdraw",
    ]

    func testEveryCallStillSendsTheMethodItAlwaysSent() {
        let daemon = RecordingDaemon()
        let client = DaemonClient(daemon: daemon)

        // Each of these fails to decode against the empty result the
        // recording daemon answers with. That is deliberate and irrelevant:
        // the call was made, which is the whole subject of this test.
        try? client.acknowledgeNearAINotice()
        _ = try? client.approve(entryID: "e")
        _ = try? client.armingSuggestion()
        try? client.cancel(entryID: "e")
        _ = try? client.clearPublicProfile()
        _ = try? client.consentOptions()
        try? client.declineArming(projectID: "p")
        try? client.dismiss(entryID: "e")
        _ = try? client.enroll(invite: "i")
        _ = try? client.publicProfile()
        _ = try? client.settings()
        _ = try? client.historyRollup()
        _ = try? client.listAudit()
        _ = try? client.listHistory()
        _ = try? client.listPending()
        _ = try? client.listProjects()
        _ = try? client.pause()
        _ = try? client.previewSummary(entryID: "e")
        try? client.cancelPreview(entryID: "e")
        _ = try? client.requestPreview(entryID: "e")
        try? client.setVisiblePreviews(entryIDs: ["e"])
        // Both proxy-facing calls, driven through the same client. They
        // reach a recording daemon here, never a socket and never a port.
        _ = try? client.probeRouting(RoutingForm(on: true, port: 8463, tokenDir: ""))
        _ = try? client.probeRoutedTools(RoutingForm(on: true, port: 8463, tokenDir: ""))
        _ = try? client.queueOutcomeCounts()
        try? client.refreshHistory()
        try? client.resume()
        _ = try? client.setConsentScopes(["model_training"])
        _ = try? client.setProjectMode(projectID: "p", mode: .ask)
        _ = try? client.setPublicProfile(handle: "h", bio: nil)
        _ = try? client.setSettings(["quiescence_secs": 45])
        _ = try? client.status()
        _ = try? client.withdraw(submissionID: "s")

        XCTAssertEqual(Set(daemon.calls.map(\.method)).sorted(), Self.expected)
    }

    /// The set above is the whole of it: every method name is one the
    /// daemon advertises. A shell calling something the daemon does not
    /// have gets `method-not-found`, which is a shipped-broken button.
    func testEveryMethodSentIsOneTheDaemonAdvertises() {
        // The daemon's own list, transcribed from
        // `crates/trace-commons-contributor/src/daemon/ipc.rs`'s `METHODS`.
        let advertised: Set<String> = [
            "acknowledge_near_ai_notice", "approve", "cancel", "clear_public_profile",
            "consent_options", "discover_routing", "dismiss", "arming_suggestion",
            "decline_arming", "enroll", "get_public_profile", "get_settings", "hello",
            "history_rollup", "list_audit", "list_history", "list_pending", "list_projects",
            "pause", "preview", "preview_body", "preview_cancel", "preview_request",
            "preview_turns", "preview_visible", "probe_routed_tools", "probe_routing",
            "queue_outcome_counts", "quiesce", "refresh_history", "resume",
            "set_consent_scopes", "set_project_mode", "set_public_profile", "set_settings",
            "shutdown", "status", "subscribe", "withdraw", "withdraw_bulk",
        ]
        XCTAssertTrue(Set(Self.expected).isSubset(of: advertised))
    }
}
