import XCTest

@testable import TCShellCore

/// The harness list's decoding and its branches, without the dylib.
///
/// The injected calls deliberately do NOT reimplement the Rust branch
/// tables: a fake that reproduced the real mapping would let this suite pass
/// while the shell had stopped asking the shared table at all. Every one of
/// them is a spy or a stub returning a value the test chose.
final class HarnessSurfaceTests: XCTestCase {
    private let payload = """
        {"destination":"DESTINATION","subtitle":"SUBTITLE",
         "offer_title":"T","offer_what":"WHAT","offer_exposure":"EXPOSURE",
         "offer_no_repoint":"NO-REPOINT","offer_accept":"ACCEPT",
         "offer_decline":"DECLINE","offer_asked_once":"ONCE",
         "settings_title":"S-TITLE","settings_toggle":"S-TOGGLE",
         "settings_applies_at_once":"S-AT-ONCE","state_off":"S-OFF","state_unknown":"S-UNKNOWN","state_unreported":"S-UNREPORTED","state_stopping":"S-STOPPING",
         "state_running":"S-RUNNING","state_running_no_backends":"S-NO-BACKENDS",
         "state_running_elsewhere":"S-ELSEWHERE","state_port_in_use":"S-PORT",
         "state_start_failed":"S-FAILED","state_crashed":"S-CRASHED",
         "quit_also_stops":"QUIT","write_unconfirmed":"UNCONFIRMED","settings_moved":"MOVED","tray_turn_off":"TRAYOFF","tray_open_to_turn_on":"TRAYON",
         "harnesses_title":"H-TITLE","harnesses_what":"H-WHAT",
         "harness_not_connected":"H-NOT-CONNECTED",
         "harness_connected_nothing_seen":"H-NOTHING-SEEN",
         "harness_answering":"H-ANSWERING","harness_connect":"H-CONNECT",
         "harness_disconnect":"H-DISCONNECT",
         "harness_preview_title":"H-PREVIEW","harness_preview_confirm":"H-CONFIRM",
         "harness_preview_cancel":"H-CANCEL","harness_slot_taken":"H-TAKEN",
         "harness_needs_restart":"H-RESTART","harnesses_none_found":"H-NONE",
         "harness_unreadable_config":"H-UNREADABLE",
         "harness_not_installed":"H-NOT-INSTALLED",
         "harness_plan_nothing_to_change":"H-NOTHING-TO-CHANGE",
         "harness_plan_entry_unusable":"H-ENTRY-UNUSABLE",
         "harness_plan_no_config_path":"H-NO-CONFIG-PATH"}
        """

    private func copy() -> PrivateInferenceCopy {
        guard let copy = PrivateInferenceCopy.decode(fromJSON: payload) else {
            XCTFail("the fixture payload must decode")
            fatalError("unreachable")
        }
        return copy
    }

    private static let oneRow = """
        {"catalog_present":false,"destination_port":8891,
         "harnesses":[{"id":"claude","name":"Claude Code","installed":true,
           "connected":false,"config_path":"/Users/x/.claude/settings.json",
           "connect_command":"tc connect claude","family":"anthropic",
           "state":"not_connected","last_call_at":null,
           "can_connect":true,"can_disconnect":false}],
         "activity":{"readable":true,"window_hours":24,"last_call_at":null,"families":[]}}
        """

    // MARK: - The list

    /// A row this build cannot read must not silently become a connected one.
    func testAMalformedPayloadYieldsNoRowsRatherThanAGuess() {
        let list = HarnessSurface.list(fromJSON: #"{"harnesses":[{"name":"x"}]}"#)
        XCTAssertEqual(list.harnesses.count, 0)
        XCTAssertNil(list.destinationPort)
    }

    /// The file path is part of the row, always. A tool nobody expected to be
    /// set up is a question about which file, every time.
    func testTheRowCarriesTheFileItWouldChange() {
        let list = HarnessSurface.list(fromJSON: Self.oneRow)
        XCTAssertEqual(list.harnesses.count, 1)
        XCTAssertEqual(list.harnesses[0].configPath, "/Users/x/.claude/settings.json")
        XCTAssertEqual(list.harnesses[0].connectCommand, "tc connect claude")
        XCTAssertEqual(list.destinationPort, 8891)
        XCTAssertFalse(list.catalogPresent)
    }

    /// A tool with nowhere to write is listed with a nil path rather than
    /// dropped: "we have never heard of it" and "we cannot find its file"
    /// are different answers.
    func testARowWithNoConfigPathStillDecodes() {
        let json = Self.oneRow.replacingOccurrences(
            of: "\"config_path\":\"/Users/x/.claude/settings.json\"", with: "\"config_path\":null")
        let list = HarnessSurface.list(fromJSON: json)
        XCTAssertEqual(list.harnesses.count, 1)
        XCTAssertNil(list.harnesses[0].configPath)
    }

    // MARK: - The state, and what may be painted as working

    /// The state code is asked of the shared table, never matched on here.
    func testTheStateCodeIsAskedOfTheSharedTable() {
        let seen = SpyBox()
        let calls = HarnessCalls(
            stateCode: { seen.record($0); return 33 },
            planOutcomeCode: { _ in 40 },
            actionAvailable: { _, _, _ in false },
            stateLine: { _ in "" },
            lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        XCTAssertEqual(HarnessSurface.state("answering", calls: calls), .answering)
        XCTAssertEqual(seen.values, ["answering"])
    }

    /// A state this build has no words for is not painted as working, and a
    /// tone table that answered `.clear` for it would be the fail-open the
    /// whole surface exists to prevent.
    func testOnlyAnsweringReadsAsWorking() {
        XCTAssertTrue(HarnessSurface.tone(.answering).readsAsWorking)
        for state in [
            HarnessState.notConnected, .connectedNoCalls, .activityShared, .unknown,
        ] {
            XCTAssertFalse(
                HarnessSurface.tone(state).readsAsWorking,
                "\(state) must not be painted as working")
        }
    }

    /// The sentence is asked of the shared table, never chosen here.
    ///
    /// The label goes across untouched -- including one this build has never
    /// heard of -- and whatever comes back is what is drawn. A stub stands in
    /// for the Rust so this runs without the dylib; `harness_copy_is_central`
    /// is what stops a `switch` over the payload's fields growing back.
    func testTheStateSentenceIsAskedOfTheSharedTable() {
        let seen = SpyBox()
        let calls = HarnessCalls(
            stateCode: { _ in 33 },
            planOutcomeCode: { _ in 40 },
            actionAvailable: { _, _, _ in false },
            stateLine: { label in
                seen.record(label)
                return label == "answering" ? "SHARED-ANSWERING" : ""
            },
            lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        XCTAssertEqual(
            HarnessSurface.stateSentence("answering", calls: calls), "SHARED-ANSWERING")
        XCTAssertEqual(seen.values, ["answering"])
        XCTAssertNil(HarnessSurface.stateSentence("a_state_from_a_later_daemon", calls: calls))
    }

    /// "One of these two answered" is not "this one is answering", so the
    /// shared-activity state borrows neither the answering sentence nor the
    /// nothing-seen one -- it says nothing, which is all it can honestly say.
    ///
    /// Checked against the sentences the table itself would return, so this
    /// cannot pass by having the stub agree with a literal typed here.
    func testTheUnattributableStatesClaimNothing() {
        let answering = "H-ANSWERING"
        let nothingSeen = "H-NOTHING-SEEN"
        let calls = HarnessCalls(
            stateCode: { _ in 33 },
            planOutcomeCode: { _ in 40 },
            actionAvailable: { _, _, _ in false },
            stateLine: { label in
                switch label {
                case "not_connected": return "H-NOT-CONNECTED"
                case "connected_no_calls": return nothingSeen
                case "answering": return answering
                default: return ""
                }
            },
            lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })

        XCTAssertEqual(HarnessSurface.stateSentence("not_connected", calls: calls), "H-NOT-CONNECTED")
        XCTAssertEqual(HarnessSurface.stateSentence("connected_no_calls", calls: calls), nothingSeen)
        XCTAssertEqual(HarnessSurface.stateSentence("answering", calls: calls), answering)

        for silent in ["activity_shared", "unknown"] {
            let sentence = HarnessSurface.stateSentence(silent, calls: calls)
            XCTAssertNil(sentence, "\(silent) grew a sentence")
            XCTAssertNotEqual(sentence, answering)
            XCTAssertNotEqual(sentence, nothingSeen)
            XCTAssertFalse(HarnessSurface.tone(HarnessState.fromABI(34)).readsAsWorking)
        }
    }

    /// The when-line crosses the ABI, so this shell draws it at all.
    ///
    /// It used to exist in Rust and stop there, and this app rendered
    /// nothing where the GNOME one rendered a line. An absent timestamp
    /// crosses as a negative number, the shared "nothing to report".
    func testTheWhenLineIsAssembledOnTheFarSide() {
        let seen = SecondsBox()
        let calls = HarnessCalls(
            stateCode: { _ in 33 },
            planOutcomeCode: { _ in 40 },
            actionAvailable: { _, _, _ in false },
            stateLine: { _ in "" },
            lastCallLine: { seconds in
                seen.record(seconds)
                return seconds < 0 ? "" : "SHARED-WHEN"
            },
            outcomeLine: { _ in "" })

        let withCall = Self.oneRow.replacingOccurrences(
            of: "\"last_call_at\":null,", with: "\"last_call_at\":\"2026-01-01T00:00:00Z\",")
        let row = HarnessSurface.list(fromJSON: withCall).harnesses[0]
        let now = Date(timeIntervalSince1970: 1_767_225_600 + 120)
        XCTAssertEqual(
            HarnessSurface.lastCallSentence(row, now: now, calls: calls), "SHARED-WHEN")
        XCTAssertEqual(seen.values, [120])

        // No timestamp is nothing to report, and never a call at time zero.
        let none = HarnessSurface.list(fromJSON: Self.oneRow).harnesses[0]
        XCTAssertNil(HarnessSurface.lastCallSentence(none, now: now, calls: calls))
        XCTAssertEqual(seen.values, [120])
    }

    /// The running copy of a tool read its settings when it started. The
    /// sentence about that stays up while the file says one thing and no
    /// call has been attributed, and goes the moment one is.
    func testTheRestartSentenceStaysUpUntilACallIsAttributed() {
        let connected = Self.oneRow.replacingOccurrences(
            of: "\"connected\":false", with: "\"connected\":true")
        let row = HarnessSurface.list(fromJSON: connected).harnesses[0]
        XCTAssertEqual(
            HarnessSurface.restartSentence(row, state: .connectedNoCalls, copy: copy()), "H-RESTART")
        XCTAssertEqual(
            HarnessSurface.restartSentence(row, state: .activityShared, copy: copy()), "H-RESTART")
        XCTAssertNil(HarnessSurface.restartSentence(row, state: .answering, copy: copy()))

        let notConnected = HarnessSurface.list(fromJSON: Self.oneRow).harnesses[0]
        XCTAssertNil(
            HarnessSurface.restartSentence(notConnected, state: .notConnected, copy: copy()))
    }

    /// An unfamiliar code is `unknown`, never the value next to it.
    func testAnUnfamiliarStateCodeIsUnknown() {
        for code: Int32 in [0, 22, 29, 35, 99, -1] {
            XCTAssertEqual(HarnessState.fromABI(code), .unknown)
        }
    }

    // MARK: - The actions offered on a row

    /// The row's actions are the daemon's answer, never re-derived here.
    ///
    /// They cannot be re-derived, and that is the point. The shared table
    /// takes `connected`, which is narrowed to "names OUR destination port".
    /// The daemon computes `can_disconnect` from the broader `wired` -- "names
    /// any local proxy" -- which is not on the wire. Asking the table here
    /// answered false for a config naming a stale or foreign port, so the row
    /// offered Connect, the daemon refused it as a no-op, and the contributor
    /// was told their file "already says what this would have written" with no
    /// route to the disconnect that would fix it.
    ///
    /// The spy therefore asserts a NEGATIVE: the shared table is not consulted
    /// for these two questions at all.
    func testTheActionsOfferedAreTheDaemonsAnswerAndNotReDerived() {
        let seen = SpyBox()
        let calls = HarnessCalls(
            stateCode: { _ in 31 },
            planOutcomeCode: { _ in 40 },
            actionAvailable: { action, _, _ in
                seen.record(action)
                // Deliberately the opposite of what the row reports, so a
                // shell that consults this instead of the row fails loudly.
                return action == "disconnect"
            },
            stateLine: { _ in "" }, lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        let row = HarnessSurface.list(fromJSON: Self.oneRow).harnesses[0]

        // The fixture reports can_connect true, can_disconnect false. The spy
        // would answer the reverse.
        XCTAssertTrue(
            HarnessSurface.canConnect(row, calls: calls),
            "the row's own answer must win")
        XCTAssertFalse(
            HarnessSurface.canDisconnect(row, calls: calls),
            "the row's own answer must win")
        XCTAssertEqual(
            seen.values, [],
            "the shared table must not be consulted: it cannot see `wired`")
    }

    // MARK: - The plan

    /// Only `changes` is committable, and a plan id is what makes the commit
    /// possible at all -- the shell cannot construct a write of its own.
    func testOnlyAChangesPlanWithAnIdIsCommittable() {
        let calls = HarnessCalls(
            stateCode: { _ in 31 },
            planOutcomeCode: { outcome in outcome == "changes" ? 41 : 42 },
            actionAvailable: { _, _, _ in true },
            stateLine: { _ in "" },
            lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        let changes = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"changes","plan_id":"c0ffee",
                 "path":"/p","changes":["set a thing"],"occupied":[]}
                """#)
        XCTAssertEqual(changes?.changes, ["set a thing"])
        XCTAssertEqual(changes.map { HarnessSurface.canCommit($0, calls: calls) }, true)

        let noop = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"noop","plan_id":null,
                 "path":"/p","changes":[],"occupied":[]}
                """#)
        XCTAssertEqual(noop.map { HarnessSurface.outcome($0, calls: calls) }, .noop)
        XCTAssertEqual(noop.map { HarnessSurface.canCommit($0, calls: calls) }, false)
    }

    /// A committable outcome with no plan id is still not committable. The
    /// daemon mints the id; a shell that fell back to sending the tool id
    /// would have constructed a write.
    func testAChangesPlanWithoutAnIdIsNotCommittable() {
        let calls = HarnessCalls(
            stateCode: { _ in 31 }, planOutcomeCode: { _ in 41 },
            actionAvailable: { _, _, _ in true },
            stateLine: { _ in "" }, lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        let plan = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"changes","plan_id":null,
                 "path":"/p","changes":["set a thing"],"occupied":[]}
                """#)
        XCTAssertEqual(plan.map { HarnessSurface.canCommit($0, calls: calls) }, false)
    }

    /// Every outcome that writes nothing explains itself, and it is the
    /// shared table that says how.
    ///
    /// The stub deliberately does NOT reproduce the Rust's mapping; it
    /// echoes the label, so a shell that stopped asking and went back to
    /// choosing between the payload's fields would fail here rather than
    /// pass by agreeing with a literal typed in this file. Four of these
    /// five outcomes had no sentence at all before, and their preview opened
    /// with a title, a path, no changes and a way out.
    func testEveryOutcomeThatChangesNothingSaysWhy() {
        let seen = SpyBox()
        let calls = HarnessCalls(
            stateCode: { _ in 31 },
            planOutcomeCode: { _ in 42 },
            actionAvailable: { _, _, _ in true },
            stateLine: { _ in "" },
            lastCallLine: { _ in "" },
            outcomeLine: { label in
                seen.record(label)
                return label == "changes" ? "" : "SHARED-\(label)"
            })
        for outcome in [
            "noop", "unparseable", "not_installed", "entry_unusable", "no_config_path",
        ] {
            let plan = HarnessSurface.plan(
                fromJSON: #"""
                    {"id":"claude","action":"connect","outcome":"OUTCOME","plan_id":null,
                     "path":"/p","changes":[],"occupied":[]}
                    """#.replacingOccurrences(of: "OUTCOME", with: outcome))
            XCTAssertEqual(
                plan.map { HarnessSurface.outcomeSentence($0, calls: calls) },
                "SHARED-\(outcome)",
                "\(outcome) opens an empty preview")
        }
        XCTAssertEqual(
            seen.values,
            ["noop", "unparseable", "not_installed", "entry_unusable", "no_config_path"])

        // A plan with changes shows them; a sentence above them would be this
        // app narrating its own list.
        let changes = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"changes","plan_id":"c0ffee",
                 "path":"/p","changes":["set a thing"],"occupied":[]}
                """#)
        XCTAssertEqual(
            changes.map { HarnessSurface.outcomeSentence($0, calls: calls) }, .some(nil))
    }

    /// A tool that is not on this machine says so, and does not keep a
    /// sentence about settings it does not have.
    ///
    /// Before this the two rendered identically -- same not-connected
    /// sentence, connect button simply absent, nothing saying why.
    func testAMissingToolSaysSoRatherThanBorrowingTheNotConnectedSentence() {
        let calls = HarnessCalls(
            stateCode: { _ in 31 },
            planOutcomeCode: { _ in 42 },
            actionAvailable: { _, _, _ in false },
            stateLine: { _ in "H-NOT-CONNECTED" },
            lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        let present = HarnessSurface.list(fromJSON: Self.oneRow).harnesses[0]
        XCTAssertEqual(
            HarnessSurface.rowSentence(present, copy: copy(), calls: calls), "H-NOT-CONNECTED")

        let missingJSON = Self.oneRow.replacingOccurrences(
            of: "\"installed\":true", with: "\"installed\":false")
        let missing = HarnessSurface.list(fromJSON: missingJSON).harnesses[0]
        XCTAssertFalse(missing.installed, "the fixture must actually be uninstalled")
        XCTAssertEqual(
            HarnessSurface.rowSentence(missing, copy: copy(), calls: calls), "H-NOT-INSTALLED")
        XCTAssertNotEqual(
            HarnessSurface.rowSentence(missing, copy: copy(), calls: calls), "H-NOT-CONNECTED")
    }

    /// An occupied slot survives to the screen, never swallowed. This is the
    /// rule most likely to be lost to a well-meaning simplification.
    func testAnOccupiedSlotSurvivesToTheScreen() {
        let plan = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"noop","plan_id":null,"path":"/p",
                 "changes":[],
                 "occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}]}
                """#)
        XCTAssertEqual(plan?.occupied.first?.slot, "env.ANTHROPIC_BASE_URL")
        XCTAssertEqual(plan?.occupied.first?.current, "https://theirs.example")
    }

    /// Occupied is not an outcome. It rides alongside a committable plan,
    /// because IronWire fills the empty slots and reports the full one in the
    /// same pass.
    func testAnOccupiedSlotRidesAlongsideAPlanThatStillHasChanges() {
        let calls = HarnessCalls(
            stateCode: { _ in 31 }, planOutcomeCode: { _ in 41 },
            actionAvailable: { _, _, _ in true },
            stateLine: { _ in "" }, lastCallLine: { _ in "" },
            outcomeLine: { _ in "" })
        let plan = HarnessSurface.plan(
            fromJSON: #"""
                {"id":"claude","action":"connect","outcome":"changes","plan_id":"c0ffee","path":"/p",
                 "changes":["set a thing"],
                 "occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}]}
                """#)
        XCTAssertEqual(plan.map { HarnessSurface.canCommit($0, calls: calls) }, true)
        XCTAssertEqual(plan?.occupied.count, 1)
        XCTAssertEqual(
            plan.map { HarnessSurface.outcomeSentence($0, calls: calls) }, .some(nil))
    }

    /// The words for an occupied slot say it was left alone. They are the
    /// payload's, and the surface has none of its own.
    func testTheOccupiedSentenceIsThePayloadsAndSaysItWasLeftAlone() {
        XCTAssertEqual(HarnessSurface.occupiedSentence(copy: copy()), "H-TAKEN")
    }

    // MARK: - The commit, and a plan that is no longer held

    /// `harness_commit` takes a plan id and nothing else.
    func testTheCommitCarriesTheMintedPlanIdAndNothingElse() {
        let params = HarnessSurface.commitParams(planID: "c0ffee")
        XCTAssertEqual(params.keys.sorted(), ["plan_id"])
        XCTAssertEqual(params["plan_id"] as? String, "c0ffee")
    }

    /// The plan params name a tool and an action, and never a file or a
    /// value to write.
    func testThePlanParamsNameOnlyAToolAndAnAction() {
        let params = HarnessSurface.planParams(id: "claude", action: .connect)
        XCTAssertEqual(params.keys.sorted(), ["action", "id"])
        XCTAssertEqual(params["action"] as? String, "connect")
        XCTAssertEqual(
            HarnessSurface.planParams(id: "claude", action: .disconnect)["action"] as? String,
            "disconnect")
    }

    /// Expired, already committed, never minted, the file moved, the write
    /// failed: the daemon takes the plan out of its store before it checks
    /// any of those, so every one of them leaves nothing to commit again.
    /// None is a retry, and the contributor is told the change did not
    /// happen in the payload's own words.
    func testEveryFailedCommitSpendsThePlanAndIsNotRetried() {
        for code in [
            "harness-plan-unknown", "harness-config-changed", "harness-commit-failed",
            "unavailable",
        ] {
            XCTAssertTrue(
                HarnessSurface.planIsSpent(afterCommitFailure: code),
                "\(code) must not leave a plan id a shell could send again")
        }
        XCTAssertEqual(HarnessSurface.commitFailureSentence(copy: copy()), "UNCONFIRMED")
    }

    /// A connect with nothing answering here is a refusal the daemon makes,
    /// and the shell's answer to it is the exposure question -- not a retry.
    func testAConnectWithNoDestinationIsTheExposureQuestion() {
        XCTAssertTrue(HarnessSurface.isNoDestination("harness-no-destination"))
        XCTAssertFalse(HarnessSurface.isNoDestination("harness-unknown"))
    }

    // MARK: - The exposure gate

    /// The listener is open to everything on this machine, which does not
    /// follow from connecting one tool. Every connect made while it is off
    /// asks first, and that set is a superset of the first-run offer's.
    func testEveryConnectWhileNothingAnswersHereAsksTheExposureQuestion() {
        XCTAssertTrue(HarnessSurface.connectNeedsExposure(listenerOn: false))
        XCTAssertFalse(HarnessSurface.connectNeedsExposure(listenerOn: true))
    }

    /// The gate never lets a first connect past the first-run offer: wherever
    /// the shared table says to ask, the gate asks too.
    func testTheGateIsAtLeastAsCautiousAsTheSharedOfferTable() {
        let offer: @Sendable (Bool, Bool) -> Bool = { !$0 && !$1 }
        for answered in [false, true] {
            for on in [false, true] where offer(answered, on) {
                XCTAssertTrue(HarnessSurface.connectNeedsExposure(listenerOn: on))
            }
        }
    }

    /// Accepting turns the destination on and records the answer in one
    /// write; declining records the answer alone and connects nothing.
    func testAcceptingTurnsItOnAndDecliningWritesTheMarkerAlone() {
        let accept = HarnessSurface.exposureParams(accepted: true)
        XCTAssertEqual(accept["private_inference"] as? Bool, true)
        XCTAssertEqual(accept["private_inference_offer_seen"] as? Bool, true)
        let decline = HarnessSurface.exposureParams(accepted: false)
        XCTAssertNil(decline["private_inference"])
        XCTAssertEqual(decline["private_inference_offer_seen"] as? Bool, true)
    }
}

/// A tiny recorder, so a stub can also be a spy.
/// The same, for the seconds handed to the when-line. A separate box rather
/// than stringifying: the number is the thing under test.
private final class SecondsBox: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [Int64] = []
    func record(_ value: Int64) {
        lock.lock()
        defer { lock.unlock() }
        seen.append(value)
    }
    var values: [Int64] {
        lock.lock()
        defer { lock.unlock() }
        return seen
    }
}

private final class SpyBox: @unchecked Sendable {
    private let lock = NSLock()
    private var seen: [String] = []
    func record(_ value: String) {
        lock.lock()
        defer { lock.unlock() }
        seen.append(value)
    }
    var values: [String] {
        lock.lock()
        defer { lock.unlock() }
        return seen
    }
}
