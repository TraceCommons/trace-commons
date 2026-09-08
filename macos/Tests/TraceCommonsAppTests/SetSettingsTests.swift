import TCBridge
import TCShellCore
import XCTest
@testable import TraceCommonsApp

/// The daemon a `DaemonClient` talks to in these tests: it records what was
/// sent and answers with whatever the test decided, so the method name and
/// the parameter bytes are assertable without a live socket, a state
/// directory. Shared decisions still cross the real FFI.
///
/// `openPreview` throws rather than answering: no test here opens one, and
/// a `TCPreview` cannot be built outside `TCBridge` anyway. It is on the
/// protocol only because `DaemonClient` offers it.
private final class RecordingDaemon: DaemonCalling {
    /// One write fans out into several `Task.detached` calls, so this
    /// double is called from more than one thread at once while the test
    /// reads `calls` from the main actor. An unsynchronised `Array.append`
    /// under that corrupts the buffer's refcount and segfaults the test
    /// process. See CredentialCancelTests for the crash this prevents.
    private let lock = NSLock()
    private var recorded: [(method: String, params: String)] = []

    var calls: [(method: String, params: String)] {
        lock.lock()
        defer { lock.unlock() }
        return recorded
    }
    /// What the next call answers with. Defaults to a well-formed frame
    /// carrying an empty result, which every call here either ignores or
    /// fails to decode -- never a silent success.
    private var responseValue = #"{"id":1,"result":{}}"#

    var response: String {
        get { lock.lock(); defer { lock.unlock() }; return responseValue }
        set { lock.lock(); defer { lock.unlock() }; responseValue = newValue }
    }

    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock()
        defer { lock.unlock() }
        recorded.append((method: method, params: paramsJSON))
        return responseValue
    }

    /// Never called by these tests. Nil is the honest answer for a
    /// double with no session behind it: not a count of zero.
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }

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

    func testInferenceEvidenceNeedsExplicitDisclosureBeforeAnyWrite() {
        XCTAssertThrowsError(try client.setInferenceEvidence(true, disclosureConfirmed: false))
        XCTAssertTrue(daemon.calls.isEmpty)
    }

    func testInferenceEvidenceDoesNotInferConsentFromPrivacyScanOrRouting() throws {
        daemon.response = settingsFrame
        let settings = try client.settings()
        XCTAssertFalse(settings.inferenceEvidenceEnabled)
        XCTAssertNil(settings.ironwireAttestedBodies)
    }

    func testInferenceEvidenceWritesOnlyConsentAndRequiresDaemonConfirmation() throws {
        daemon.response = settingsFrame.replacingOccurrences(of: "\"near_ai_configured\":false", with: "\"ironwire_attested_bodies\":true,\"near_ai_configured\":false")
        let settings = try client.setInferenceEvidence(true, disclosureConfirmed: true)
        XCTAssertTrue(settings.inferenceEvidenceEnabled)
        XCTAssertEqual(daemon.lastParams?.count, 1)
        XCTAssertEqual(daemon.lastParams?["ironwire_attested_bodies"] as? Bool, true)

        daemon.response = settingsFrame
        XCTAssertThrowsError(try client.setInferenceEvidence(true, disclosureConfirmed: true))
        XCTAssertThrowsError(try client.setInferenceEvidence(false, disclosureConfirmed: false))
    }

    func testInferenceEvidenceCanBeDisabledWithoutAnotherConsentDecision() throws {
        daemon.response = settingsFrame.replacingOccurrences(of: "\"near_ai_configured\":false", with: "\"ironwire_attested_bodies\":false,\"near_ai_configured\":false")
        XCTAssertFalse(try client.setInferenceEvidence(false, disclosureConfirmed: false).inferenceEvidenceEnabled)
        XCTAssertEqual(daemon.lastParams?["ironwire_attested_bodies"] as? Bool, false)
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

final class NativeWitnessReviewTests: XCTestCase {
    func testCapabilityIsExplicitAndDoesNotRequestAReview() throws {
        let daemon = RecordingDaemon()
        let client = DaemonClient(daemon: daemon)
        XCTAssertFalse(try client.supportsWitnessReview())
        daemon.response = #"{"result":{"methods":["preview_request","witness_preview_request"]}}"#
        XCTAssertTrue(try client.supportsWitnessReview())
        XCTAssertEqual(daemon.calls.map(\.method), ["hello", "hello"])
    }

    func testConfirmedRequestHasNoApprovalOrOutcomeAndRequiresReady() throws {
        let daemon = RecordingDaemon()
        let client = DaemonClient(daemon: daemon)
        XCTAssertThrowsError(try client.requestWitnessReview(entryID: "entry"))
        daemon.response = #"{"result":{"status":"ready","summary":{}}}"#
        try client.requestWitnessReview(entryID: "entry")
        XCTAssertEqual(daemon.calls.last?.method, "witness_preview_request")
        XCTAssertEqual(daemon.lastParams?["entry_id"] as? String, "entry")
        XCTAssertEqual(daemon.lastParams?["raw_session_confirmed"] as? Bool, true)
        XCTAssertEqual(daemon.lastParams?.count, 2)
    }
}

final class NativeFlowAdapterTests: XCTestCase {
    func testWalletAdapterTransportsCoreViewWithoutLocalOriginOrCadenceLogic() throws {
        let daemon = RecordingDaemon()
        daemon.response = #"{"id":1,"result":{"flow_id":"fixture","state":"WaitingForWallet","busy":true,"can_check":false,"can_start":false,"can_edit":false,"can_cancel":true,"wait":true,"message":"fixture","tone":"neutral","glyph":"","browser_url":"https://commons.example/exact"}}"#
        let result = try DaemonClient(daemon: daemon).nativeWalletFlow(action: "wait", flowID: "fixture", commons: "", account: "")
        XCTAssertEqual(daemon.calls.last?.method, "native_wallet_flow")
        XCTAssertEqual(daemon.lastParams?["action"] as? String, "wait")
        XCTAssertTrue(result.wait)
        XCTAssertTrue(result.canCancel)
        XCTAssertFalse(result.canStart)
        XCTAssertEqual(result.browserURL, "https://commons.example/exact")
    }
    func testAdmissionAdapterUsesCoreExpiryDecision() throws {
        let daemon = RecordingDaemon()
        daemon.response = #"{"id":1,"result":{"status":"ready_for_next_inference","expires_at":1,"view":{"ready":false,"message":"fixture refusal","tone":"refused","glyph":"⊘"}}}"#
        let result = try DaemonClient(daemon: daemon).prepareAdmissionSession(entryID: "fixture", backend: "near")
        XCTAssertFalse(try XCTUnwrap(result.view).ready)
        XCTAssertEqual(result.view?.tone, "refused")
    }

    // MARK: - Answering model calls on this computer

    /// A frame carrying the offer's two keys, and the state beside them.
    private func privateInferenceFrame(
        on: Bool, answered: Bool, state: String = "off", port: String = "null"
    ) -> String {
        """
        {"id":1,"result":{"quiescence_secs":45,"digest_interval_secs":3600,
        "local_notifications":true,"queue_ttl_days":14,"max_queue_entries":500,
        "max_uploads_per_day":100,"near_ai_configured":false,
        "claude_root_configured":true,"codex_root_configured":true,
        "private_inference":\(on),"private_inference_offer_seen":\(answered),
        "private_inference_state":{"state":"\(state)","port":\(port)}}}
        """
    }

    func testPrivateInferenceAcceptRequiresBothEchoedKeys() throws {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(on: false, answered: true)
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).answerPrivateInferenceOffer(accepted: true))
        // A marker-only decline must not turn off an existing listener.
        daemon.response = privateInferenceFrame(on: true, answered: true)
        XCTAssertTrue(try DaemonClient(daemon: daemon).answerPrivateInferenceOffer(accepted: false).privateInferenceOn)
    }

    func testPrivateInferenceSwitchCannotConfirmMissingMarkerOrMissingFalse() {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(on: false, answered: true)
            .replacingOccurrences(of: "\"private_inference_offer_seen\":true,", with: "")
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).setPrivateInference(false))
        daemon.response = privateInferenceFrame(on: false, answered: true)
            .replacingOccurrences(of: "\"private_inference\":false,", with: "")
        XCTAssertThrowsError(try DaemonClient(daemon: daemon).setPrivateInference(false))
    }

    /// Declining records the answer and writes no switch.
    ///
    /// The switch is already false; writing it would make a refusal
    /// indistinguishable from a change on every surface watching settings.
    func testDecliningTheOfferWritesTheMarkerAlone() throws {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(on: false, answered: true)
        let client = DaemonClient(daemon: daemon)

        _ = try client.answerPrivateInferenceOffer(accepted: false)

        XCTAssertEqual(daemon.calls.last?.method, "set_settings")
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params.count, 1)
        XCTAssertEqual(params["private_inference_offer_seen"] as? Bool, true)
        XCTAssertNil(params["private_inference"])
    }

    /// Accepting writes both keys in one call: an accept that started the
    /// listener and failed to record the answer would ask again next launch.
    func testAcceptingTheOfferWritesBothKeysInOneCall() throws {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(
            on: true, answered: true, state: "running", port: "8463")
        let client = DaemonClient(daemon: daemon)

        let view = try client.answerPrivateInferenceOffer(accepted: true)

        XCTAssertEqual(daemon.calls.count, 1, "one call, not two")
        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params["private_inference"] as? Bool, true)
        XCTAssertEqual(params["private_inference_offer_seen"] as? Bool, true)
        XCTAssertTrue(view.privateInferenceOn)
        XCTAssertTrue(view.privateInferenceAnswered)
        XCTAssertEqual(view.privateInferenceState?.state, "running")
        XCTAssertEqual(view.privateInferenceState?.port, 8463)
    }

    /// A daemon that did not record the answer is a refusal, not a success.
    func testAnAnswerTheDaemonDidNotRecordIsRefused() {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(on: false, answered: false)
        let client = DaemonClient(daemon: daemon)
        XCTAssertThrowsError(try client.answerPrivateInferenceOffer(accepted: false))
    }

    /// The settings switch records the answer too, so a contributor who
    /// found it themselves is not asked about it later.
    func testTheSettingsSwitchAlsoAnswersTheQuestion() throws {
        let daemon = RecordingDaemon()
        daemon.response = privateInferenceFrame(on: true, answered: true, state: "start_failed")
        let client = DaemonClient(daemon: daemon)

        let view = try client.setPrivateInference(true)

        let params = try XCTUnwrap(daemon.lastParams)
        XCTAssertEqual(params["private_inference"] as? Bool, true)
        XCTAssertEqual(params["private_inference_offer_seen"] as? Bool, true)
        // A listener that refused to start is a sentence to render, NOT a
        // failed write: the switch is on and the state says what happened.
        XCTAssertTrue(view.privateInferenceOn)
        XCTAssertEqual(view.privateInferenceState?.state, "start_failed")
    }

    /// A daemon that never heard of the keys reads as off and unanswered,
    /// which is what makes the offer appear once after an upgrade.
    func testADaemonWithoutTheKeysReadsAsOffAndUnanswered() throws {
        let daemon = RecordingDaemon()
        daemon.response = settingsFrame
        let client = DaemonClient(daemon: daemon)

        let view = try client.setSettings(["local_notifications": true])

        XCTAssertFalse(view.privateInferenceOn)
        XCTAssertFalse(view.privateInferenceAnswered)
        XCTAssertNil(view.privateInferenceState)
    }
}

private final class SourceSettingsDaemon: DaemonCalling {
    let refuseWrite: Bool
    let refuseRead: Bool
    init(refuseWrite: Bool, refuseRead: Bool = false) { self.refuseWrite = refuseWrite; self.refuseRead = refuseRead }
    func call(_ method: String, params paramsJSON: String) -> String {
        if (method == "set_settings" && refuseWrite) || (method == "get_settings" && refuseRead) {
            return #"{"id":1,"error":{"code":"unavailable","message":"settings-save-failed"}}"#
        }
        return settingsFrame.replacingOccurrences(
            of: "\"near_ai_configured\":false",
            with: "\"claude_source_mode\":\"off\",\"near_ai_configured\":false")
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

final class SourceSettingsModelTests: XCTestCase {
    @MainActor
    func testFolderWriteDisplaysDaemonAnswerInsteadOfRequestedWatchMode() async {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: SourceSettingsDaemon(refuseWrite: false)))
        let succeeded = await model.setSourceRoot(.claudeCode, .watch(path: "/requested"))
        XCTAssertTrue(succeeded)
        XCTAssertEqual(model.daemonSettings?.claudeSourceMode, "off")
    }

    @MainActor
    func testFailedWriteReadsBackModeWithoutClaimingRequestedPathWasSaved() async {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: SourceSettingsDaemon(refuseWrite: true)))
        let succeeded = await model.setSourceRoot(.claudeCode, .watch(path: "/requested"))
        XCTAssertFalse(succeeded)
        XCTAssertEqual(model.daemonSettings?.claudeSourceMode, "off")
    }
}

private final class ConsentReconciliationDaemon: DaemonCalling {
    let failRead: Bool
    private(set) var calls: [String] = []
    init(failRead: Bool) { self.failRead = failRead }
    func call(_ method: String, params paramsJSON: String) -> String {
        calls.append(method)
        if method == "status", !failRead {
            return #"{"result":{"logged_in":true,"consent_scopes":["debugging_evaluation","training"],"health":{}}}"#
        }
        return #"{"error":{"code":"unavailable","message":"reply-lost"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

extension SourceSettingsModelTests {
    @MainActor
    func testFailedWriteAndReadKeepLastConfirmedSourceSettings() async {
        let model = AppModel()
        model.setClientForTesting(DaemonClient(daemon: SourceSettingsDaemon(refuseWrite: false)))
        _ = await model.setSourceRoot(.claudeCode, .off)
        model.setClientForTesting(DaemonClient(daemon: SourceSettingsDaemon(refuseWrite: true, refuseRead: true)))
        let saved = await model.setSourceRoot(.claudeCode, .watch(path: "/requested"))
        XCTAssertFalse(saved)
        XCTAssertEqual(model.daemonSettings?.claudeSourceMode, "off")
    }

    @MainActor
    func testFailedConsentReplyReconcilesBeforeReturningAndPreservesPriorStateIfReadFails() async {
        for failRead in [false, true] {
            let model = AppModel()
            model.setStatusForTesting(DaemonStatus(schemaVersion: "1.1", loggedIn: true,
                tenantID: "fixture", consentScopes: ["debugging_evaluation"], paused: false,
                queueDepth: 0, nextDigestAt: nil, health: DaemonHealth(lastErrorLabel: nil, since: nil)))
            let daemon = ConsentReconciliationDaemon(failRead: failRead)
            model.setClientForTesting(DaemonClient(daemon: daemon))
            let outcome = await model.setConsentScopes(["debugging_evaluation", "training"])
            XCTAssertEqual(outcome, .failed)
            XCTAssertEqual(daemon.calls, ["set_consent_scopes", "status"])
            XCTAssertEqual(model.status.consentScopes,
                failRead ? ["debugging_evaluation"] : ["debugging_evaluation", "training"])
        }
    }
}

private final class PendingPrivateInferenceDaemon: DaemonCalling, @unchecked Sendable {
    let started: XCTestExpectation
    let release = DispatchSemaphore(value: 0)
    private let lock = NSLock()
    private var count = 0
    var callCount: Int { lock.lock(); defer { lock.unlock() }; return count }
    init(started: XCTestExpectation) { self.started = started }
    func call(_ method: String, params paramsJSON: String) -> String {
        lock.lock(); count += 1; lock.unlock()
        started.fulfill()
        _ = release.wait(timeout: .now() + 5)
        return #"{"error":{"code":"unavailable","message":"settings-save-failed"}}"#
    }
    func searchOriginal(entryID: String, needle: String) -> Int? { nil }
    func openPreview(entryID: String) throws -> TCPreview { throw TCDaemon.TCError.daemonGone }
}

final class PrivateInferencePendingWriteTests: XCTestCase {
    @MainActor
    func testPendingWriteRejectsSecondRequestAndFailureRetainsConfirmedState() async throws {
        let started = expectation(description: "first write reached daemon")
        let daemon = PendingPrivateInferenceDaemon(started: started)
        defer { daemon.release.signal() }
        let model = AppModel()
        let initial = RecordingDaemon()
        initial.response = settingsFrame.replacingOccurrences(of: "\"near_ai_configured\":false", with: "\"private_inference\":false,\"private_inference_offer_seen\":false,\"near_ai_configured\":false")
        let confirmed = try DaemonClient(daemon: initial).settings()
        model.setDaemonSettingsForTesting(confirmed)
        model.setClientForTesting(DaemonClient(daemon: daemon))
        model.applyPrivateInference(true)
        model.answerPrivateInferenceOffer(accepted: false)
        XCTAssertTrue(model.privateInferenceBusy)
        XCTAssertEqual(model.daemonSettings, confirmed)
        await fulfillment(of: [started], timeout: 2)
        XCTAssertEqual(daemon.callCount, 1)
        daemon.release.signal()
        for _ in 0..<200 where model.privateInferenceBusy {
            try await Task.sleep(nanoseconds: 10_000_000)
        }
        XCTAssertFalse(model.privateInferenceBusy)
        XCTAssertEqual(model.daemonSettings, confirmed)
        XCTAssertEqual(model.lastActionError, try XCTUnwrap(model.privateInferenceCopy).writeUnconfirmed)
        XCTAssertTrue(model.showsPrivateInferenceOffer)
        initial.response = initial.response.replacingOccurrences(of: "\"private_inference\":false", with: "\"private_inference\":true").replacingOccurrences(of: "\"private_inference_offer_seen\":false", with: "\"private_inference_offer_seen\":true")
        model.setClientForTesting(DaemonClient(daemon: initial))
        model.applyPrivateInference(true)
        for _ in 0..<200 where model.privateInferenceBusy { try await Task.sleep(nanoseconds: 10_000_000) }
        XCTAssertFalse(model.privateInferenceBusy)
        XCTAssertTrue(model.daemonSettings?.privateInferenceOn == true)
        XCTAssertNil(model.lastActionError)
    }
}
